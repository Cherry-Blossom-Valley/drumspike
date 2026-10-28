/* Self-test driver (P3-7a). The debug build of the app injects this file into the page after the
   `load` event when it starts with `--self-test <scenario> --out <file>`; release builds do not
   contain it (apps/desktop/src-tauri/src/selftest.rs). It runs the scenario in window.__SELFTEST__ in the
   real WKWebView, with the real commands and the real page code, and sends one JSON report to the
   command `selftest_report`, which writes the file and ends the process (exit 0 = pass).
   The report holds numbers only: no screenshot is taken (no macOS screen permission needed). */
(async () => {
  const sc = window.__SELFTEST__;
  const T = window.__TAURI__.core;
  const report = { scenario: sc.name, pass: false, steps: [], metrics: {}, tracks: {}, errors: [] };
  const now = () => performance.now();
  const r2 = x => Math.round(x * 100) / 100;
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const frame = () => new Promise(r => requestAnimationFrame(r));
  const $q = s => document.querySelector(s);

  async function until(fn, what, ms) {
    const end = now() + (ms || 60000);
    for (;;) {
      const v = await fn();
      if (v) return v;
      if (now() > end) throw new Error(`timed out waiting for ${what}`);
      await sleep(50);
    }
  }
  async function step(name, fn) {
    const t = now();
    try {
      const detail = await fn();
      report.steps.push({ step: name, ok: true, ms: r2(now() - t), detail: detail === undefined ? null : detail });
    } catch (e) {
      report.steps.push({ step: name, ok: false, ms: r2(now() - t), error: String((e && e.message) || e) });
      throw e;
    }
  }
  function stats(d) {
    if (!d.length) return { n: 0 };
    const s = d.slice().sort((a, b) => a - b), q = p => s[Math.min(s.length - 1, Math.floor(p * s.length))];
    return { n: d.length, mean_ms: r2(d.reduce((a, b) => a + b, 0) / d.length), p50_ms: r2(q(0.5)), p95_ms: r2(q(0.95)),
             p99_ms: r2(q(0.99)), max_ms: r2(s[s.length - 1]), over_20ms: d.filter(x => x > 20).length };
  }
  /* frame times for `ms` milliseconds; onFrame(i) may change the view; a hidden window gives no frames */
  function sampleFrames(ms, onFrame) {
    return new Promise(res => {
      const d = []; let last = null, i = 0, done = false, startAt = null;
      const fin = note => { if (!done) { done = true; const s = stats(d); if (note) s.note = note; res(s); } };
      const timer = setTimeout(() => fin("no animation frames (window hidden?)"), ms + 4000);
      requestAnimationFrame(function tick(t) {
        if (startAt === null) startAt = t;
        if (last !== null) d.push(t - last);
        last = t;
        if (onFrame) onFrame(i++);
        if (t - startAt < ms) requestAnimationFrame(tick); else { clearTimeout(timer); fin(); }
      });
    });
  }
  /* a wave lane is drawn by WebGL2 under its transparent 2D canvas (P3-6a): laneImage gives both together */
  /* laneImage reads the pixels back from the tile worker (P3-6b), so it is async. A far lane holds a 1 x 1 canvas
     (P3-6b): its wave lane is read through the engine, its spike canvas is not counted. */
  const drawnCanvases = async () => {
    const out = [];
    for (const c of document.querySelectorAll("#lanesScroll canvas")) {
      if (c.classList.contains("wave")) {
        const px = (await laneImage(c.closest(".track").dataset.name)).data;
        let n = 0; for (let i = 3; i < px.length; i += 4) if (px[i]) n++;
        out.push(n);
      } else if (c.width > 1) {
        const px = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
        let n = 0; for (let i = 3; i < px.length; i += 4) if (px[i]) n++;
        out.push(n);
      }
    }
    return out;
  };
  /* P3-14d: the layout reads inside the page's frame (FR.inFrame, the frame scheduler of app.js). A read there lays the
     page out when the frame wrote before it (a forced layout); the frame should read none (it takes GEO). */
  const layoutReads = (() => {
    const R = { n: 0, where: [] };
    const hit = p => { if (typeof FR !== "undefined" && FR.inFrame) { R.n++; if (R.where.length < 8) R.where.push(p); } };
    const props = [[Element.prototype, ["clientWidth", "clientHeight", "clientLeft", "clientTop", "scrollTop", "scrollLeft", "scrollWidth", "scrollHeight"]],
                   [HTMLElement.prototype, ["offsetWidth", "offsetHeight", "offsetLeft", "offsetTop", "offsetParent", "innerText"]],
                   [MouseEvent.prototype, ["offsetX", "offsetY"]]];
    for (const [proto, names] of props) for (const p of names) {
      const d = Object.getOwnPropertyDescriptor(proto, p);
      if (d && d.get) Object.defineProperty(proto, p, Object.assign({}, d, { get() { hit(p); return d.get.call(this); } }));
    }
    for (const m of ["getBoundingClientRect", "getClientRects"]) {
      const f = Element.prototype[m];
      Element.prototype[m] = function () { hit(m); return f.apply(this, arguments); };
    }
    const gcs = window.getComputedStyle;
    window.getComputedStyle = function () { hit("getComputedStyle"); return gcs.apply(this, arguments); };
    return { reset: () => { R.n = 0; R.where = []; }, get: () => ({ n: R.n, where: R.where.slice() }) };
  })();
  const frameCounts = () => (typeof FR !== "undefined" ? Object.assign({}, FR.n) : null);
  const need = (cond, msg) => { if (!cond) throw new Error(msg); };
  const want = name => !sc.steps || sc.steps.includes(name);   // optional list of the steps to run (cold_start, open, lanes always run)

  try {
    await step("cold_start", async () => {
      await frame();
      report.metrics.cold_start_ms = r2(await T.invoke("selftest_clock"));
      need(B.app === true, "B.app is not true: the page did not get the IPC backend");
      const info = await T.invoke("app_info");
      report.metrics.app = { core_version: info.core_version, detector: info.detector, chain: info.chain, model_ok: info.model ? info.model.ok : null };
      need(!info.model || info.model.ok, `model not ok: ${info.model && info.model.error}`);   // #281 D16: main is model-free (no model key)
      return report.metrics.app;
    });

    await step("open", async () => {
      const clock0 = await T.invoke("selftest_clock");   // P3-15c: the process clock, for the trace windows
      const t = now();
      const texts = [], show = window.showProgress;   // P3-4a: what the bar said while peaks.bin was built
      window.showProgress = (pct, text) => { texts.push(text); show(pct, text); };
      need(await openProjectPath(sc.project), "openProjectPath returned false");
      await until(() => S.loaded > 0 && S.order.length > 0 && S.order.every(n => n in S.peaks), "the lanes", 120000);
      window.showProgress = show;
      report.metrics.open_ms = r2(now() - t);
      report.metrics.open_clock_ms = [r2(clock0), r2(await T.invoke("selftest_clock"))];
      report.metrics.lane_progress = { n: texts.length, first: texts[0] || null, last: texts[texts.length - 1] || null };
      need(S.order.length === sc.expect.tracks, `expected ${sc.expect.tracks} tracks, got ${S.order.length}`);
      return { tracks: S.order.length, sid: S.sid };
    });

    await step("lanes", async () => {
      await frame(); await frame();
      const bad = S.order.filter(n => !S.peaks[n] || S.peakErr[n]);
      need(bad.length === 0, `lanes without peaks: ${bad.join(", ")}`);
      const px = await drawnCanvases();
      report.metrics.lanes_drawn = px.filter(n => n > 0).length;
      report.metrics.lanes_canvases = px.length;
      need(px.length >= S.order.length + 2 && px.every(n => n > 0), `canvases with no pixels: ${px.filter(n => !n).length} of ${px.length}`);
      need(S.pcm === undefined, "the page has a PCM store");   // P3-12: the page holds no PCM
      need(S.order.every(n => S.peaks[n].tiles === true), "the page holds peak arrays in app mode (P3-6b: the tiles belong to the tile worker)");
      return { lanes: S.order.length, canvases: px.length, engine: GLQ.eng && GLQ.eng.remote ? "worker" : GLQ.eng ? "page" : "none" };
    });

    if (want("frames")) {
      await step("frames", async () => {
        const ms = (sc.frames && sc.frames.seconds ? sc.frames.seconds : 2) * 1000;
        const full = Math.max(10, ...S.order.map(n => S.tracks[n].duration));
        const span = Math.min(30, full), travel = full - span;
        layoutReads.reset();
        const fc0 = frameCounts();
        report.metrics.frames_scroll = await sampleFrames(ms, i => setView((i * 0.15) % travel, (i * 0.15) % travel + span));
        report.metrics.frames_zoom = await sampleFrames(ms, i => {
          const k = 0.5 + 0.5 * Math.sin(i / 25), s = Math.max(0.05, full * Math.pow(k, 6));
          setView(full / 2 - s / 2, full / 2 + s / 2);
        });
        setView(0, full);
        await frame();
        const fc1 = frameCounts();
        report.metrics.frames_layout_reads = layoutReads.get();   // P3-14d: reads inside the frames of the two sweeps
        report.metrics.frames_drawn = fc0 && fc1 ? { frames: fc1.frames - fc0.frames, view: fc1.view - fc0.view } : null;
        if (sc.frames && sc.frames.max_layout_reads !== undefined) need(report.metrics.frames_layout_reads.n <= sc.frames.max_layout_reads, `layout reads in the frame: ${JSON.stringify(report.metrics.frames_layout_reads)}`);
        if (sc.frames && sc.frames.max_zoom_p99_ms) need(report.metrics.frames_zoom.p99_ms <= sc.frames.max_zoom_p99_ms, `zoom p99 ${report.metrics.frames_zoom.p99_ms} ms > ${sc.frames.max_zoom_p99_ms} ms`);
        return { scroll: report.metrics.frames_scroll, zoom: report.metrics.frames_zoom, layout_reads: report.metrics.frames_layout_reads.n };
      });
    }

    if (want("analyze")) {
      await step("analyze", async () => {
        const t = now();
        const sid = S.sid;
        const during = sampleFrames(Math.min(20000, (sc.timeout_s || 300) * 200), null);
        $q("#analyzeAllBtn").click();
        await until(() => /analysis done|failed/.test($q("#progressText") ? $q("#progressText").textContent : "") , "the analysis", (sc.timeout_s || 300) * 1000);
        report.metrics.analyze_ms = r2(now() - t);
        report.metrics.frames_during_analyze = await during;
        const st = await T.invoke("analyze_status", { sid });
        const names = Object.keys(st.tracks);
        const detectors = [...new Set(names.map(n => st.tracks[n].detector))];
        const failed = names.filter(n => st.tracks[n].state !== "done");
        report.metrics.detectors = detectors;
        report.metrics.tracks_done = names.length - failed.length;
        need(failed.length === 0, `tracks not done: ${failed.map(n => n + ": " + (st.tracks[n].error || st.tracks[n].state)).join("; ")}`);
        if (sc.expect.detector) need(detectors.length === 1 && detectors[0] === sc.expect.detector, `detectors ${JSON.stringify(detectors)}, expected ${sc.expect.detector}`);
        // "analysis done" shows before the page has loaded the new events of every track
        await until(() => S.order.every(n => S.events[n].length === st.tracks[n].n_events), "the page to load the new events", 30000);
        let total = 0;
        for (const n of S.order) {
          report.tracks[n] = { hits: S.events[n].length };
          total += S.events[n].length;
          if (sc.record_events) report.tracks[n].events = S.events[n].map(e => [e.t, e.v]);
          report.tracks[n].note = S.tracks[n].note;
        }
        report.metrics.hits_total = total;
        need(total >= (sc.expect.min_hits_total || 1), `only ${total} hits`);
        return { done: names.length, detectors, hits: total };
      });
    }

    if (want("edit")) {
      await step("edit", async () => {
        const name = S.order.find(n => S.events[n].length > 0);
        const n0 = S.events[name].length, sid = S.sid;
        const held = async () => (await T.invoke("events_get", { sid, name })).events.length;
        const spike = $q(`.track[data-name="${name}"] .spike`);
        const r = spike.getBoundingClientRect();
        const ev = (type, extra) => spike.dispatchEvent(new MouseEvent(type, Object.assign({ bubbles: true, clientX: r.left + 40, clientY: r.top + 8 }, extra)));
        ev("dblclick");
        need(S.events[name].length === n0 + 1, "double click did not add a hit");
        await until(async () => (await held()) === n0 + 1, "events_set after the add", 5000);
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true, bubbles: true }));
        need(S.events[name].length === n0, "undo did not remove the hit");
        await until(async () => (await held()) === n0, "events_set after the undo", 5000);
        /* P3-14d: 10 hit edits (5 adds, 5 undos, one frame each) redraw the track's hit lines and post no GL draw */
        await until(() => !GLQ.eng || GLQ.eng.stats.drawn >= GLQ.eng.stats.posted, "the GL engine to answer its draws", 10000);
        const g0 = GLQ.eng ? GLQ.eng.stats.posted : 0, f0 = frameCounts();
        layoutReads.reset();
        const at = S.view.t0 + 0.37 * (S.view.t1 - S.view.t0);
        for (let k = 0; k < 5; k++) { addHitAt(name, snapSample(at + k * 0.01, name)); await frame(); }
        for (let k = 0; k < 5; k++) { undo(); await frame(); }
        await frame();
        const f1 = frameCounts();
        report.metrics.edit_gl_draws = GLQ.eng ? GLQ.eng.stats.posted - g0 : null;
        report.metrics.edit_frames = f0 && f1 ? Object.fromEntries(Object.keys(f1).map(k => [k, f1[k] - f0[k]])) : null;
        report.metrics.edit_layout_reads = layoutReads.get();
        need(S.events[name].length === n0, "the 5 undos did not remove the 5 hits");
        need(report.metrics.edit_gl_draws === 0, `${report.metrics.edit_gl_draws} GL draws for 10 hit edits`);
        need(report.metrics.edit_layout_reads.n === 0, `layout reads in the frames of the edits: ${JSON.stringify(report.metrics.edit_layout_reads)}`);
        await until(async () => (await held()) === n0, "events_set after the 10 edits", 5000);
        return { track: name, before: n0, gl_draws_10_edits: report.metrics.edit_gl_draws, frames: report.metrics.edit_frames };
      });
    }

    if (want("export")) {
      await step("export", async () => {
        const name = S.order.find(n => S.events[n].length > 0);
        const head = $q(`.track[data-name="${name}"] .thead`);                  // P3-14e: the context menu of the header
        head.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 40, clientY: 40 }));
        $q('#ctxMenu [data-act="export"]').click();   // P4-30: Export... opens the dialog with this track; MIDI only
        await until(() => !$q("#exportModal").classList.contains("hidden"), "the Export dialog", 15000);
        if ($q("#exSpikes").checked) $q("#exSpikes").click();
        $q("#exGo").click();
        await until(() => /exported 1 MIDI file to Exports\//.test($q("#stMsg").textContent), "the export message", 15000);
        $q("#exCancel").click();
        report.metrics.export_message = $q("#stMsg").textContent;
        const files = {};
        for (const n of S.order) {
          const t = S.tracks[n];
          const r = await T.invoke("export_midi", { sid: S.sid, name: n, note: t.note, events: S.events[n], ppq: 24000, tempo: 120 });
          files[n] = r;
          report.tracks[n].midi = r;
        }
        return { files: Object.keys(files).length };
      });
    }

    if (sc.steps && sc.steps.includes("render")) {   // P3-6a: 30 lanes drawn by WebGL2 during a (clock-driven) playback; agreement with Canvas 2D
      await step("render", async () => {
        const rc = sc.render || {};
        need(typeof GLQ !== "undefined" && GLQ.eng, "WebGL2 lanes are not active in this WebView (GLQ.eng is null)");
        const eng = GLQ.eng, levels = 3 * S.order.length;
        const tw = now();
        await until(() => eng.textures().peaks >= levels, "the tile textures (background warm-up)", 30000);
        report.metrics.render = { gl: eng.info(), lanes: S.order.length, warm_ms: r2(now() - tw) };
        if (rc.marks) {   // P4-6a: Learn marks on every lane (drawn only, no service call): the bars and ticks cost nothing visible
          for (const n of S.order) {
            const d = S.tracks[n].duration || 10, k = rc.marks;
            S.tracks[n].learn = { on: true, threshold: 0.5, auto: false, method: "patch-1",
              marks: Array.from({ length: k }, (_, i) => ({ id: `m${i + 1}`, kind: i % 3 === 2 ? "not" : "this", t0: d * (i + 0.2) / k, t1: d * (i + 0.6) / k, at: [d * (i + 0.3) / k, d * (i + 0.5) / k] })) };
          }
          invalidateHits(); report.metrics.render.learn_marks = { per_lane: rc.marks, lanes: S.order.length };
        }
        const full = Math.max(10, ...S.order.map(n => S.tracks[n].duration));
        const span = Math.min(30, full);
        const med = a => { const x = a.slice().sort((p, q) => p - q); return x.length ? x[x.length >> 1] : 0; };
        /* Real playback (P3-5b merged): the page's play button starts the native transport and the page's own
           tick() moves the playhead from the transport clock (appTickPos), follows it and draws. The driver wraps
           tick() to time the JS of each frame; the frame interval comes from the animation frame timestamps.
           mode "app": the page's own smooth follow (the lanes redraw about every 30 ms); mode "every": the
           follow is off and the wrapper redraws all lanes in every frame (a stricter load than the app).
           The run is silent: the transport's master gain must be 0 (self-test.sh starts the app with
           DRUMSPIKE_MASTER_GAIN=0) unless the scenario sets allow_audible.
           P4-1: the page's tick no longer runs in every frame while the compositor moves the playhead (it runs in the
           meter frames, about 25 Hz). The frame rate comes from the driver's own animation-frame loop (the frames the
           WebView delivers while the page plays; a long tick or a long draw shows as a long interval), the JS per frame
           from the page's ticks; `ticks` counts them. */
        async function playback(ms, mode) {
          S.followMode = mode === "every" ? "off" : "smooth"; S.followHold = false;
          S.playhead = 0; setView(0, span); await frame();
          const realTick = window.tick;
          const d0 = eng.stats.draws, js = [], dt = [], marks = [], arms0 = typeof CP !== "undefined" ? CP.arms : 0;
          let t0 = null, last = 0, i = 0, ticks = 0, run = true;
          window.tick = function (t) {
            const j0 = now();
            realTick();
            if (mode === "every" && S.playing) setView(S.playhead - span * 0.25, S.playhead - span * 0.25 + span);
            if (typeof t !== "number" || !AC.running || t0 === null) return;   // play() calls tick() once directly; the rings fill first
            js.push(now() - j0); ticks++;
          };
          const loop = t => {
            if (!run) return;
            if (S.playing && AC.running) {
              if (t0 === null) { t0 = t; last = t; }
              else { dt.push(t - last); last = t; i++; if (Math.floor((t - t0) / 1000) >= marks.length) marks.push(i); }
            }
            requestAnimationFrame(loop);
          };
          requestAnimationFrame(loop);
          const ts = now();
          $q("#playBtn").click();
          let st0;
          try {
            await until(() => S.playing && AC.running, "playback to start", 60000);
            st0 = await T.invoke("transport_status", {});
            need(rc.allow_audible || st0.master === 0, `the transport runs with master gain ${st0.master}, not 0`);
            const start_ms = r2(now() - ts);
            await until(() => t0 !== null && last - t0 >= ms, `${ms} ms of playback`, ms + 30000);
            const st1 = await T.invoke("transport_status", {});
            $q("#playBtn").click();
            await until(() => !S.playing, "the pause", 10000);
            const per = []; for (let k = 1; k < marks.length; k++) per.push(marks[k] - marks[k - 1]);   // frames in each full second
            const js95 = js.slice().sort((p, q) => p - q);
            return { mode, transport: true, start_ms, frames: dt.length, ticks, compositor_arms: typeof CP !== "undefined" ? CP.arms - arms0 : null, seconds: ms / 1000, fps_median: per.length ? med(per) : null, fps_per_second: per,
                     fps_mean: r2(1000 * dt.length / dt.reduce((p, q) => p + q, 0)), frame: stats(dt.slice(1)),
                     js_p50_ms: r2(med(js)), js_p95_ms: r2(js95[Math.min(js95.length - 1, Math.floor(0.95 * js95.length))]),
                     js_max_ms: r2(js95[js95.length - 1]), lane_redraws: eng.stats.draws - d0, gl_max_ms: r2(eng.stats.max_ms),
                     underruns: st1.underruns - st0.underruns, tracks: st1.tracks, device: st1.device, master: st1.master,
                     pos_end_s: r2(st1.pos),
                     strip: typeof SP !== "undefined" ? Object.assign({ leaving: SP.leaving, leave_frames: SP.leaveFrames }, SP.n) : null };   // P4-2: a fallback that still waits shows here
          } finally {
            run = false;
            if (S.playing) $q("#playBtn").click();
            window.tick = realTick;
          }
        }
        /* P4-2: mode "pan", the view moved by the user with the transport stopped. Real wheel events on a wave lane go
           through the page's own handler (panView, or the track list scroll), one event per frame, sent from the driver's
           animation-frame loop: a steady pan of one view per second (right, then left), a horizontal wheel burst that
           decays like a trackpad's momentum, and a vertical burst (the track list down and up). Before them: 2 s of
           frames with no change (the floor of the frame clock) and zoom steps (the frames until the lanes show the new
           view). Per segment: the frame intervals (over_16_7: longer than one 60 Hz period; missed: 25 ms or longer,
           a frame the WebView did not deliver), the JS per frame (the wheel handler plus the page's frame callback), the
           view draws and the GL draws posted, and the wall-clock window (the CPU sampler of a measurement cuts it). */
        async function panMode(pc) {
          pc = Object.assign({ preset: 2, at_s: 40, seconds: 4, idle_ms: 1500, burst_px: 80, burst_decay: 0.92, scroll_px: 40, scroll_frames: 60 }, pc || {});
          S.followMode = "off"; S.followHold = false;
          if (S.playing) { $q("#playBtn").click(); await until(() => !S.playing, "the pause", 10000); }
          S.playhead = pc.at_s;
          zoomPreset(pc.preset); await frame(); await frame();
          await until(() => eng.stats.drawn >= eng.stats.posted, "the GL lanes after the preset", 10000);
          await sleep(pc.idle_ms);
          const lane = () => { const sr = $q("#lanesScroll").getBoundingClientRect();
            return [...document.querySelectorAll("#lanesScroll .track canvas.wave")].find(c => { const r = c.getBoundingClientRect(); return r.top >= sr.top && r.bottom <= sr.bottom; }); };
          const wheel = (dx, dy) => { const c = lane(), r = c.getBoundingClientRect();
            c.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaX: dx, deltaY: dy, deltaMode: 0, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 })); };
          const realFrame = window.onFrame;
          let appJs = 0;
          window.onFrame = function (t) { const a = now(); try { return realFrame(t); } finally { appJs += now() - a; } };
          /* one segment: onFrameFn(i, dt) sends this frame's input; false ends the segment */
          const segment = (maxMs, onFrameFn) => new Promise(res => {
            const dt = [], js = [], fc0 = frameCounts(), g0 = eng.stats.posted, w0 = Date.now(), c0 = now();
            let last = null, i = 0, pend = 0;
            const fin = () => {
              const s = stats(dt), sorted = js.slice().sort((p, q) => p - q), fc1 = frameCounts();
              Object.assign(s, { over_16_7: dt.filter(x => x > 16.7).length, missed: dt.filter(x => x >= 25).length,
                fps: r2(1000 * dt.length / Math.max(1, dt.reduce((p, q) => p + q, 0))),
                js_p95_ms: r2(sorted[Math.min(sorted.length - 1, Math.floor(0.95 * sorted.length))] || 0), js_max_ms: r2(sorted[sorted.length - 1] || 0),
                view_draws: fc1.view - fc0.view, gl_posts: eng.stats.posted - g0, epoch_ms: [w0, Date.now()], ms: r2(now() - c0) });
              res(s);
            };
            requestAnimationFrame(function tick(t) {
              if (last !== null) { dt.push(t - last); js.push(pend + appJs); }
              appJs = 0;
              const a = now(), go = onFrameFn(i++, last === null ? 1000 / 60 : t - last);
              pend = now() - a; last = t;
              if (go !== false && now() - c0 < maxMs) requestAnimationFrame(tick);
              else requestAnimationFrame(t2 => { dt.push(t2 - last); js.push(pend + appJs); fin(); });
            });
          });
          const out = { preset: pc.preset, view_s: r2(viewDur()), lane_w: laneWidth() };
          try {
            out.idle = await segment(2000, () => true);
            /* zoom steps: frames (and ms) from the call until the lanes show the new view: the page drew it (or showed it
               from its tiles) and the GL engine answered every draw posted until then */
            out.zoom_steps = [];
            for (const [what, fn] of [["out", () => zoomStep(false)], ["in", () => zoomStep(true)], ["preset_30s", () => zoomPreset(1)], ["preset_back", () => zoomPreset(pc.preset)]]) {
              await sleep(pc.idle_ms);
              const v0 = FR.n.view, s0 = typeof TL !== "undefined" ? TL.n.shown : 0, t0 = now();
              let frames = 0;
              fn();
              const seen = await new Promise(res => requestAnimationFrame(function chk() {
                frames++;
                const drawn = FR.n.view > v0 || (typeof TL !== "undefined" && TL.n.shown > s0);
                if ((drawn && eng.stats.drawn >= eng.stats.posted && !FR.raf) || frames > 120) res(drawn); else requestAnimationFrame(chk);
              }));
              out.zoom_steps.push({ step: what, view_s: r2(viewDur()), frames, ms: r2(now() - t0), shown: seen });
            }
            await sleep(pc.idle_ms);
            const perS = laneWidth();   // one view per second: the lane width in wheel pixels per second
            out.steady_right = await segment(pc.seconds * 1000, (i, dt) => { wheel(perS * dt / 1000, 0); });
            out.steady_left = await segment(pc.seconds * 1000, (i, dt) => { wheel(-perS * dt / 1000, 0); });
            await sleep(pc.idle_ms);
            let v = pc.burst_px;
            out.burst = await segment(10000, () => {
              if (Math.abs(v) < 0.5) { if (v > 0) { v = -pc.burst_px; return true; } return false; }
              wheel(v, 0); v *= pc.burst_decay; return true;
            });
            await sleep(pc.idle_ms);
            out.scroll = await segment(10000, i => { if (i >= 2 * pc.scroll_frames) return false; wheel(0, i < pc.scroll_frames ? pc.scroll_px : -pc.scroll_px); return true; });
            await sleep(1000);   // the CPU sampler of a measurement (0.25 s) covers the last segment
          } finally { window.onFrame = realFrame; }
          return out;
        }
        if (rc.pan_only) {   // P4-2 measurement runs: only the pan mode
          report.metrics.render.pan = await panMode(rc.pan);
          setView(0, full); await frame();
          const p = report.metrics.render.pan;
          return { pan: ["steady_right", "steady_left", "burst", "scroll"].map(k => [k, p[k].over_16_7, p[k].missed]) };
        }
        /* where the JS of one redraw goes: each part repeated 200 times (the WebView rounds performance.now to 1 ms) */
        { setView(0, span); await frame();
          const rows = S.order.map(n => ({ n, row: $q(`.track[data-name="${n}"]`) }));
          const bench = fn => { const N = 200, t = now(); for (let i = 0; i < N; i++) fn(); return r2((now() - t) / N); };
          GLQ.all = true;
          report.metrics.render.profile_ms_per_redraw = {
            ruler: bench(() => drawRuler()), overview: bench(() => drawOverview()),
            spike_lanes: bench(() => rows.forEach(r => drawSpike(r.row.querySelector(".spike"), r.n))),
            wave_lanes_queue_flush: bench(() => { rows.forEach(r => drawWave(r.row.querySelector(".wave"), r.n)); glFlush(); }),
            gl_draw_only: bench(() => glRepaint()),
            redrawAll: bench(() => redrawAll()) };
          GLQ.all = false; }
        const ms = (rc.seconds || 10) * 1000;
        report.metrics.render.visible_lanes = glItems().filter(it => { const r = it.canvas.getBoundingClientRect(), sr = $q("#lanesScroll").getBoundingClientRect(); return r.bottom >= sr.top && r.top <= sr.bottom; }).length;
        GLQ.all = false;
        layoutReads.reset();   // P3-14d: from here on (4 playbacks, the zoom sweep) the frames should read no layout
        const fcr0 = frameCounts();
        report.metrics.render.playback_app = await playback(ms, "app");
        report.metrics.render.playback_every = await playback(ms, "every");
        GLQ.all = true;   // all 30 lanes in every pass, also those scrolled out of view: the worst case, as in the spike
        report.metrics.render.playback_all_app = await playback(ms, "app");
        report.metrics.render.playback_all_every = await playback(ms, "every");
        GLQ.all = false;
        report.metrics.render.pan = await panMode(rc.pan);   // P4-2
        /* the zoom sweep of step "frames", with 30 lanes. "no_coalesce" in the scenario restores the window fetches of
           before P3-6b (one request per lane and frame) for the comparison in the report */
        /* P3-14d: the floor of the frame times: 2 s with no change at all (the WebView's own jitter of the frame clock) */
        report.metrics.render.frames_idle = await sampleFrames(2000, null);
        GLQ.noCoalesce = !!rc.no_coalesce;
        let requests = 0; const origSamples = B.samples;
        B.samples = (...a) => { requests++; return origSamples(...a); };
        report.metrics.render.frames_zoom = await sampleFrames(2000, i => {
          const k = 0.5 + 0.5 * Math.sin(i / 25), s = Math.max(0.05, full * Math.pow(k, 6));
          setView(full / 2 - s / 2, full / 2 + s / 2);
        });
        B.samples = origSamples;
        report.metrics.render.zoom_window_requests = requests;
        GLQ.noCoalesce = false;
        setView(0, full); await frame();
        const fcr1 = frameCounts();
        report.metrics.render.layout_reads_in_frames = layoutReads.get();
        report.metrics.render.frames_drawn = fcr0 && fcr1 ? Object.fromEntries(Object.keys(fcr1).map(k => [k, fcr1[k] - fcr0[k]])) : null;
        if (rc.max_layout_reads !== undefined) need(report.metrics.render.layout_reads_in_frames.n <= rc.max_layout_reads, `layout reads in the frame: ${JSON.stringify(report.metrics.render.layout_reads_in_frames)}`);
        if (rc.max_zoom_p99_ms) need(report.metrics.render.frames_zoom.p99_ms <= rc.max_zoom_p99_ms, `zoom p99 ${report.metrics.render.frames_zoom.p99_ms} ms > ${rc.max_zoom_p99_ms} ms`);
        /* agreement with Canvas 2D on the real lanes: columns with the same vertical extent (device px, +-1.5) */
        const extents = async name => {
          const im = await laneImage(name), d = im.data, out = [];
          for (let x = 0; x < im.width; x++) {
            let top = -1, bot = -1;
            for (let y = 0; y < im.height; y++) { const i = (y * im.width + x) * 4; if (Math.abs(d[i] - 16) + Math.abs(d[i + 1] - 19) + Math.abs(d[i + 2] - 24) >= 24) { if (top < 0) top = y; bot = y; } }
            out.push([top, bot]);
          }
          return out;
        };
        const agree = [];
        for (const v of [[0, full], [full * 0.2, full * 0.3], [20, 30]]) {
          const sh = [];
          /* every third lane of those that hold a canvas store (P3-6b: a far lane has none, so Canvas 2D has nothing to compare) */
          const nearLanes = S.order.filter(n => document.querySelector(`.track[data-name="${n}"]`)._live !== false);
          for (const n of nearLanes.filter((_, k) => k % 2 === 0)) {
            setView(v[0], v[1]); await setLanesGL(true); const a = await extents(n);
            await setLanesGL(false); const b = await extents(n); await setLanesGL(true);
            let ok = 0; for (let x = 0; x < a.length; x++) ok += (a[x][0] < 0 && b[x][0] < 0) || (a[x][0] >= 0 && b[x][0] >= 0 && Math.abs(a[x][0] - b[x][0]) <= 1.5 && Math.abs(a[x][1] - b[x][1]) <= 1.5);
            sh.push(ok / a.length);
          }
          agree.push({ view_s: [r2(v[0]), r2(v[1])], lanes: sh.length, min: r2(Math.min(...sh)), mean: r2(sh.reduce((p, q) => p + q, 0) / sh.length) });
        }
        report.metrics.render.agreement_2d = agree;
        need(agree.every(a => a.lanes >= 2 && a.min >= 0.99), `GL lanes agree with Canvas 2D on less than 99 % of the columns: ${JSON.stringify(agree)}`);
        /* P3-14c: the three track heights render (reference 5, done criterion): every lane and the header take the height,
           the GL lanes are drawn again, and the frames of the change read no layout. The setting is not written (save false). */
        { const was = S.trackHeight, hs = {};
          setView(0, full); await frame();
          for (const h of ["small", "large", "medium"]) {
            layoutReads.reset(); const d0 = eng.stats.draws, n0 = FR.n.size, t0 = now();
            setTrackHeight(h, false);
            await until(() => GEO.waveH === TRACK_HEIGHTS[h].wave && FR.n.size > n0 && !FR.raf, `the ${h} height`, 10000);
            await frame(); await frame();
            await until(() => eng.stats.drawn >= eng.stats.posted, `the GL lanes at the ${h} height`, 10000);
            const rows = [...document.querySelectorAll("#lanesScroll .track")];
            const bad = rows.filter(r => { const w = r.querySelector("canvas.wave"), s = r.querySelector("canvas.spike"), th = r.querySelector(".thead"), tr = r.querySelector(".track-row");
              return w.offsetHeight !== TRACK_HEIGHTS[h].wave || s.offsetHeight !== TRACK_HEIGHTS[h].spike || (r._live !== false && w.height !== Math.round(w.clientHeight * GEO.dpr))
                || Math.abs(th.getBoundingClientRect().height - tr.getBoundingClientRect().height) > 0.5 || th.scrollHeight > th.clientHeight + 0.5; }).length;
            hs[h] = { wave_h: TRACK_HEIGHTS[h].wave, rows: rows.length, rows_wrong: bad, gl_draws: eng.stats.draws - d0, layout_reads: layoutReads.get().n, ms: r2(now() - t0),
                      visible_lanes: glItems().filter(it => { const r = it.canvas.getBoundingClientRect(), sr = $q("#lanesScroll").getBoundingClientRect(); return r.bottom >= sr.top && r.top <= sr.bottom; }).length };
            need(bad === 0 && hs[h].gl_draws >= 1 && hs[h].layout_reads === 0, `track height ${h}: ${JSON.stringify(hs[h])}`);
          }
          setTrackHeight(was, false); await frame();
          report.metrics.render.track_heights = hs; }
        setView(0, full); await frame();
        const gate = report.metrics.render.playback_all_app;   // the Done-when: 30 lanes drawn during playback
        if (rc.min_fps) need(gate.fps_median >= rc.min_fps, `fps median ${gate.fps_median} < ${rc.min_fps}`);
        if (rc.max_js_p95_ms) need(gate.js_p95_ms < rc.max_js_p95_ms, `JS p95 ${gate.js_p95_ms} ms >= ${rc.max_js_p95_ms}`);
        const under = ["playback_app", "playback_every", "playback_all_app", "playback_all_every"].map(k => report.metrics.render[k].underruns);
        if (rc.max_underruns !== undefined) need(under.every(u => u <= rc.max_underruns), `transport underruns ${under.join(", ")} > ${rc.max_underruns}`);
        return { fps: gate.fps_median, js_p95: gate.js_p95_ms, underruns: under };
      });
    }

    /* P3-15c (card Q4): save, close and open again through the page's own functions, `rounds` times, then
       seeks during playback. Each phase gets its window on the process clock (selftest_clock), so the trace of a
       `--features trace` build (DRUMSPIKE_TRACE) can be cut into open, save and close (selftest/trace_spans.py). */
    if (sc.steps && sc.steps.includes("lifecycle")) {
      await step("lifecycle", async () => {
        const cfg = Object.assign({ rounds: 3, seek_count: 0 }, sc.lifecycle || {});
        const clock = async () => r2(await T.invoke("selftest_clock"));
        const phases = [{ phase: "open", round: 0, start_ms: report.metrics.open_clock_ms[0], end_ms: report.metrics.open_clock_ms[1], page_ms: report.metrics.open_ms }];
        const phase = async (name, round, fn) => {
          const a = await clock(), t = now();
          await fn();
          phases.push({ phase: name, round, start_ms: a, end_ms: await clock(), page_ms: r2(now() - t) });
        };
        const lanesReady = () => S.loaded > 0 && S.order.length === sc.expect.tracks && S.order.every(n => n in S.peaks);
        for (let i = 0; i < cfg.rounds; i++) {
          const first = S.order[0];
          await phase("save", i, async () => {
            markDirty(first);   // a real edit: the debounced event write is flushed by the save
            await saveProjectNow();
            need(!S.dirty, "the project is still dirty after the save");
          });
          const sid = S.sid;
          await phase("close", i, async () => {
            need(await closeProject(sid, "Self Test"), "closeProject returned false");
            need(S.sid !== sid, "the project is still the current one");
          });
          await phase("open", i + 1, async () => {
            need(await openProjectPath(sc.project), "openProjectPath returned false");
            await until(lanesReady, "the lanes after the reopen", 120000);
          });
        }
        report.metrics.lifecycle = phases;
        /* seeks while the transport plays (silent): the answer time of transport_seek, as the page sees it */
        if (cfg.seek_count > 0) {
          const st0 = await T.invoke("transport_status", {});
          need(cfg.allow_audible || st0.master === 0, `the master gain is ${st0.master}, not 0`);
          const full = Math.max(10, ...S.order.map(n => S.tracks[n].duration));
          const a = await clock();
          await T.invoke("transport_play", { sid: S.sid, position: 1.0, events: null, progress: new T.Channel() });
          const seeks = [];
          for (let k = 0; k < cfg.seek_count; k++) {
            await sleep(100);
            const t = now();
            await T.invoke("transport_seek", { position: (k * 7.3) % (full - 2) });
            seeks.push(now() - t);
          }
          await T.invoke("transport_stop", { close: false });
          const st = await T.invoke("transport_status", {});
          report.metrics.seek = Object.assign(stats(seeks), { underruns: st.underruns, window: [a, await clock()] });
        }
        return { phases: phases.length, seek: report.metrics.seek || null };
      });
    }

    if (sc.steps && sc.steps.includes("memory")) await step("memory", () => window.__selftestMemory({ report, need, until, sleep, now, r2, T, sc, stats }));   // P3-6b, memory.js

    if (want("play") && sc.steps) await step("play", () => window.__selftestPlay({ report, need, until, sleep, now, r2, T, sc }));   // P3-5b, play.js
    if (want("playhead_steps") && sc.steps) await step("playhead_steps", () => window.__selftestPlayheadSteps({ report, need, until, sleep, now, r2, T, sc }));   // #463, playhead_steps.js

    report.pass = report.steps.every(s => s.ok);
  } catch (e) {
    report.errors.push(String((e && e.message) || e));
    report.pass = false;
  }
  await T.invoke("selftest_report", { report });
})();
