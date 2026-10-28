/* Step "memory" (P3-6b): the 30-track session in the real WKWebView. It defines window.__selftestMemory, which the
   driver calls (like play.js for the step "play"). What it does, in the order of the card:
     1. memory after the first open (the cache is built then);
     2. open to drawn with a warm cache: close the project, open it again, 3 times (the time from the call of
        openProjectPath until the engine has answered the first draw of the loaded project);
     3. a zoom sweep with 30 lanes (frame times) before any playback;
     4. real playback for `play_seconds` (default 60), silent (master gain 0), memory every 5 s;
     5. the same zoom sweep after the playback (the P3-5b integration found p99 49 to 96 ms here), and once more after
        the transport has closed;
     6. zoom from the whole song to the sample level and back, memory at the deepest point, and once more with
        all 30 lanes at the sample level (more windows than the cache holds);
   Memory is the physical footprint of the WebContent and GPU processes of this app (command selftest_memory:
   `footprint`, the helpers that started after the app). The pass rule: the WebContent peak stays under
   `max_web_content_mb` (the target of the card, `target_web_content_mb`, is reported), the median warm open to drawn under
   `max_open_to_drawn_ms`, no underruns. */
window.__selftestMemory = async function (x) {
  const { report, need, until, sleep, now, r2, T, sc } = x;
  const mc = sc.memory || {};
  const frame = () => new Promise(r => requestAnimationFrame(r));
  const $q = s => document.querySelector(s);
  const med = a => { const s = a.slice().sort((p, q) => p - q); return s.length ? s[s.length >> 1] : 0; };
  const m = report.metrics.memory = { samples: [], open_to_drawn_ms: [] };
  const t00 = now();
  async function mem(tag) {
    const r = await T.invoke("selftest_memory", {});
    let held = 0;
    for (const n of S.order) { const p = S.peaks[n]; if (p) for (const L of Object.values(p.levels)) for (const k of ["min", "max", "rms"]) if (L[k]) held += L[k].length; }
    const wc = (r.processes || []).find(q => q.kind === "WebContent");
    const row = { tag, t_s: r2((now() - t00) / 1000), top: wc ? wc.top : null, web_content_mb: r.web_content_mb, web_content_rss_mb: r.web_content_rss_mb, gpu_mb: r.gpu_mb,
                  web_content_count: r.web_content_count, page_peak_numbers: held, canvases_with_store: [...document.querySelectorAll("#lanesScroll canvas.lane")].filter(c => c.width > 1).length, textures: GLQ.eng && GLQ.eng.textures ? GLQ.eng.textures() : null,
                  strip: typeof SP !== "undefined" ? Object.assign({ on: SP.on, gl_dims: GLQ.dims, leaving: SP.leaving, leave_frames: SP.leaveFrames }, SP.n) : null };   // P4-1b: the follow strip's counters; gl_dims: the placeholder takes the size of the worker's frames (fix round)
    m.samples.push(row);
    return row;
  }
  const full = Math.max(10, ...S.order.map(n => S.tracks[n].duration));
  need(GLQ.eng, "WebGL2 lanes are not active (GLQ.eng is null)");
  m.window = { inner: [innerWidth, innerHeight], dpr: window.devicePixelRatio, lane_css_px: laneWidth(), lanes_area_css: [$q("#lanesScroll").clientWidth, $q("#lanesScroll").clientHeight] };
  m.engine = { remote: !!GLQ.eng.remote, renderer: GLQ.eng.info().renderer, max_texture: GLQ.eng.maxTexture };
  await sleep(1500);
  await mem("after_open");
  need(m.samples[0].web_content_mb > 0, "selftest_memory found no WebContent process of this app (footprint 0)");
  m.process_count = m.samples[0].web_content_count;

  /* 1b. where the footprint is (diagnosis, scenario field "diag"): the lanes' 2D canvases and the project itself */
  if (mc.diag) {
    const settle = async () => { await frame(); await sleep(2500); };
    m.diag = [];
    const take = async tag => { const r = await mem(tag); m.diag.push({ tag, web_content_mb: r.web_content_mb, top: r.top }); };
    await take("diag_baseline");
    for (const [tag, sel] of [["wave_canvases_1px", "canvas.wave"], ["spike_canvases_1px", "canvas.spike"], ["all_lane_canvases_1px", "canvas.lane"]]) {
      document.querySelectorAll(sel).forEach(c => { c.width = 1; c.height = 1; });
      await settle(); await take(tag);
      resizeCanvases(); await settle(); await take("restored_after_" + tag);
    }
    const ov = $q("#overview"); const ow = ov.width, oh = ov.height;
    ov.width = 1; ov.height = 1; OV.canvas = null; await settle(); await take("overview_1px");
    resizeCanvases(); await settle();
    GLQ.eng.canvas.style.display = "none"; await settle(); await take("gl_canvas_hidden");
    GLQ.eng.canvas.style.display = ""; await settle();
    need(await closeProject(S.sid, S.name), "closeProject failed");
    await settle(); await take("no_project");
    need(await openProjectPath(sc.project), "openProjectPath returned false");
    await until(() => S.loaded > 0 && S.order.length === S.order.length && S.order.every(n => n in S.peaks), "the lanes", 60000);
    await settle(); await take("reopened");
  }

  /* 2. open to drawn, warm cache */
  const tracks = S.order.length;
  for (let i = 0; i < (mc.reopens || 3); i++) {
    need(await closeProject(S.sid, S.name), "closeProject failed");
    await sleep(300);
    const t0 = now();
    need(await openProjectPath(sc.project), "openProjectPath returned false");
    let ms = null;
    for (;;) {
      await frame();
      if (S.loaded > 0 && S.order.length === tracks && S.order.every(n => n in S.peaks) && GLQ.eng && GLQ.loadedSeq > 0 && GLQ.eng.stats.drawn >= GLQ.loadedSeq) { ms = now() - t0; break; }
      if (now() - t0 > 30000) throw new Error("the project was not drawn within 30 s");
    }
    m.open_to_drawn_ms.push(r2(ms));
  }
  m.open_to_drawn_median_ms = med(m.open_to_drawn_ms);
  m.open_to_drawn_max_ms = Math.max(...m.open_to_drawn_ms);
  await sleep(1500);
  await mem("after_reopen");

  /* The zoom sweep of step "frames" with 30 lanes. Besides the frame times it times the JS of each frame's callback
     (setView: ruler, overview, 30 spike lanes, the lane post to the worker) and counts the long frames (over 30 ms)
     whose own JS took under 5 ms: those frames were delayed outside our code (compositor, host load, GC of another thread). */
  /* B.samples is timed in every sweep: [start, end, samples] of each window fetch, to see whether a long frame has a
     fetch (IPC bytes from the app) in flight */
  const fetches = [];
  let issued = 0, pending = 0;
  const origSamples = B.samples;
  B.samples = async (...a) => { const t0 = now(); issued++; pending++; try { const r = await origSamples(...a); fetches.push([t0, now(), r.data.length]); return r; } finally { pending--; } };
  const sweep = ms => new Promise(res => {
    const d = [], js = [], long = [];
    fetches.length = 0; const issued0 = issued;
    let last = null, i = 0, startAt = null;
    const sw0 = GLQ.eng.stats.posted, f0 = typeof GLQ.eng.stats.drawn === "number" ? GLQ.eng.stats.drawn : 0;
    requestAnimationFrame(function tick(t) {
      if (startAt === null) startAt = t;
      if (last !== null) d.push(t - last);
      last = t;
      const j0 = now();
      const k = 0.5 + 0.5 * Math.sin(i++ / 25), sp = Math.max(0.05, full * Math.pow(k, 6));
      setView(full / 2 - sp / 2, full / 2 + sp / 2);
      js.push(now() - j0);
      if (d.length && d[d.length - 1] > 30) long.push([r2(d[d.length - 1]), r2(js[js.length - 2] || 0), r2(js[js.length - 1]), now(), fetches.filter(f => f[0] < now() && f[1] > now() - d[d.length - 1]).length]);
      if (t - startAt < ms) requestAnimationFrame(tick);
      else {
        const sj = js.slice().sort((p, q) => p - q);
        const st = x.stats(d);
        st.js_p99_ms = r2(sj[Math.min(sj.length - 1, Math.floor(0.99 * sj.length))]); st.js_max_ms = r2(sj[sj.length - 1]);
        st.over_30ms = long.length; st.over_30ms_js_under_5ms = long.filter(q => q[1] < 5 && q[2] < 5).length;
        st.long = long.slice(0, 8); st.lane_posts = GLQ.eng.stats.posted - sw0;
        const fd = fetches.map(f => f[1] - f[0]).sort((p, q) => p - q);
        st.issued = issued - issued0; st.fetches = fetches.length; st.fetch_ms_median = r2(fd.length ? fd[fd.length >> 1] : 0); st.fetch_ms_max = r2(fd.length ? fd[fd.length - 1] : 0);
        st.fetched_mb = r2(fetches.reduce((a, f) => a + f[2] * 4, 0) / 1048576);
        st.long_with_fetch = long.filter(q => q[4] > 0).length;
        res(st);
      }
    });
  });
  const sweeps = async (key, n) => {   // n sweeps one after the other: the first may pay one-time costs (windows, JIT)
    const all = [];
    for (let i = 0; i < n; i++) { setView(0, full); await frame(); all.push(await sweep(2000)); }
    setView(0, full); await frame();
    m[key] = all;
  };
  /* 2b. diagnosis (scenario field "diag"): which redraw work raises the footprint. Each part runs at 30 Hz for 6 s
     without any playback; the footprint is taken at the end, and again 5 s after the work stopped. */
  if (mc.diag) {
    m.diag_redraw = [];
    const liveRows = () => [...document.querySelectorAll("#lanesScroll .track")].filter(r => r._live !== false);
    const parts = {
      redrawAll: () => redrawAll(),
      gl_repaint_only: () => glRepaint(),
      ruler_and_overview: () => { drawRuler(); drawOverview(); },
      spike_lanes: () => liveRows().forEach(r => drawSpike(r.querySelector(".spike"), r.dataset.name)),
      event_lines_on_wave_canvases: () => liveRows().forEach(r => { const c = r.querySelector(".wave"); c.getContext("2d").clearRect(0, 0, c.clientWidth, c.clientHeight); drawEventLines(c, r.dataset.name); }),
    };
    parts.gl_repaint_half_size = () => { const sz = GLQ.eng.size; GLQ.eng.size = (w, h) => sz(w >> 1, h >> 1); glRepaint(); GLQ.eng.size = sz; };
    parts.gl_repaint_15hz = (() => { let k = 0; return () => { if (k++ % 2 === 0) glRepaint(); }; })();
    parts.gl_repaint_quarter_size = () => { const sz = GLQ.eng.size; GLQ.eng.size = (w, h) => sz(w >> 2, h >> 2); glRepaint(); GLQ.eng.size = sz; };
    await sleep(3000);
    const r0 = await mem("redraw_idle"); m.diag_redraw.push({ tag: "idle before", web_content_mb: r0.web_content_mb });
    for (const [tag, fn] of Object.entries(parts)) {
      const timer = setInterval(fn, 33);
      await sleep(6000);
      const r1 = await mem("redraw_" + tag);
      clearInterval(timer);
      await sleep(5000);
      const r2_ = await mem("redraw_" + tag + "_after");
      m.diag_redraw.push({ tag, while_running_mb: r1.web_content_mb, after_5s_mb: r2_.web_content_mb });
    }
  }

  /* 3a. the window fetches of a zoom gesture, with and without one fetch per track at a time (cold windows each time) */
  m.coalesce_ab = [];
  for (const noCoalesce of [true, false, true, false]) {
    GLQ.noCoalesce = noCoalesce;
    S.samplesCache = {}; GLQ.eng.dropWindows();   // cold windows in the page and in the engine
    for (const k of Object.keys(GLQ.fetched)) delete GLQ.fetched[k];
    setView(0, full); await frame(); await sleep(300);
    const w = await sweep(2000);
    const tdr = now();
    while (pending > 0 && now() - tdr < 30000) await sleep(100);   // the next run starts when this run's fetches are all done
    w.drain_ms = r2(now() - tdr);
    m.coalesce_ab.push({ coalesce: !noCoalesce, issued: w.issued, drain_ms: w.drain_ms, p99_ms: w.p99_ms, max_ms: w.max_ms, over_30ms: w.over_30ms, fetches: w.fetches, fetched_mb: w.fetched_mb, fetch_ms_median: w.fetch_ms_median, fetch_ms_max: w.fetch_ms_max });
  }
  GLQ.noCoalesce = false;
  /* 3. zoom sweep before playback */
  await sweeps("zoom_sweeps_before_play", mc.sweeps || 3);

  /* 4. playback */
  const span = Math.min(30, full), secs = mc.play_seconds || 60;
  if (mc.strip === false) window.__DS_NO_STRIP__ = true;   // P4-1b: the control without the follow strip (the frame path glides)
  S.followMode = "smooth"; S.followHold = false; S.playhead = 0; setView(0, span); await frame();
  $q("#playBtn").click();
  await until(() => S.playing && AC.running, "playback to start", 60000);
  const st0 = await T.invoke("transport_status", {});
  need(mc.allow_audible || st0.master === 0, `the transport runs with master gain ${st0.master}, not 0`);
  const tp = now();
  await mem("play_0s");
  while ((now() - tp) / 1000 < secs) {
    await sleep(Math.max(0, 5000 - ((now() - tp) % 5000)));
    if ((now() - tp) / 1000 < secs + 1) await mem(`play_${Math.round((now() - tp) / 1000)}s`);
  }
  const st1 = await T.invoke("transport_status", {});
  m.play = { seconds: r2((now() - tp) / 1000), underruns: st1.underruns - st0.underruns, tracks: st1.tracks, master: st1.master, device: st1.device };
  $q("#playBtn").click();
  await until(() => !S.playing, "the pause", 10000);
  await sleep(500);

  /* 5. the same sweep after the playback */
  await sweeps("zoom_sweeps_after_play", mc.sweeps || 3);
  /* diagnosis of the P3-5b finding: the same sweeps once more after the transport has closed its files and rings */
  await T.invoke("transport_stop", { close: true });
  await sleep(500);
  await sweeps("zoom_sweeps_after_transport_close", mc.sweeps || 3);
  await mem("after_play");

  /* 5b. P3-14c: the footprint at each track height (the 2D canvases of the near lanes grow with the wave lane); the
     setting is not written (save false), the height is put back */
  { const was = S.trackHeight;
    for (const h of ["small", "large", "medium"]) {
      setTrackHeight(h, false);
      await until(() => GEO.waveH === TRACK_HEIGHTS[h].wave && !FR.raf, `the ${h} height`, 10000);
      await frame(); await sleep(500);
      await mem(`height_${h}`);
    }
    setTrackHeight(was, false); await frame(); }

  /* 6. whole song to the sample level and back */
  const steps = 80, deep = 0.004, centre = full / 2;
  const at = i => { const k = Math.min(1, i / steps), s = full * Math.pow(deep / full, k); setView(centre - s / 2, centre + s / 2); };
  for (let i = 0; i <= steps; i++) { at(i); await frame(); }
  await until(() => GLQ.eng.textures().samples >= 1, "the sample windows at the deepest zoom", 10000);
  await sleep(500);
  await mem("zoom_deep");
  GLQ.all = true; redrawAll();   // all 30 lanes need a window: more than the cache holds
  await sleep(2500);
  await mem("zoom_deep_all_lanes");
  GLQ.all = false;
  for (let i = steps; i >= 0; i--) { at(i); await frame(); }
  setView(0, full); await frame();
  await sleep(1000);
  await mem("zoom_back");

  B.samples = origSamples;
  const rows = m.samples;
  m.web_content_peak_mb = Math.max(...rows.map(r => r.web_content_mb));
  m.web_content_rss_peak_mb = Math.max(...rows.map(r => r.web_content_rss_mb));
  m.gpu_peak_mb = Math.max(...rows.map(r => r.gpu_mb));
  m.web_content_first_mb = rows[0].web_content_mb;
  m.web_content_last_mb = rows[rows.length - 1].web_content_mb;
  m.page_peak_numbers_max = Math.max(...rows.map(r => r.page_peak_numbers));
  need(m.process_count === 1, `${m.process_count} new WebContent processes: the footprint may belong to another app`);
  /* The card's target (400 MB) is reported, not enforced: the footprint of the WebContent process counts the graphics
     surfaces it owns, and the surfaces of the animated lanes make it rise while the lanes move (see the report, P3-6b).
     max_web_content_mb is the regression guard. */
  if (mc.target_web_content_mb) m.target = { web_content_mb: mc.target_web_content_mb, met: m.web_content_peak_mb < mc.target_web_content_mb,
                                              met_at_rest: Math.min(...rows.map(r => r.web_content_mb)) < mc.target_web_content_mb };
  if (mc.max_web_content_mb) need(m.web_content_peak_mb < mc.max_web_content_mb, `WebContent peak ${m.web_content_peak_mb} MB >= ${mc.max_web_content_mb} MB`);
  if (mc.max_open_to_drawn_ms) need(m.open_to_drawn_median_ms < mc.max_open_to_drawn_ms, `open to drawn median ${m.open_to_drawn_median_ms} ms >= ${mc.max_open_to_drawn_ms} ms`);
  if (mc.max_underruns !== undefined) need(m.play.underruns <= mc.max_underruns, `${m.play.underruns} underruns`);
  need(S.pcm === undefined, "the page has a PCM store");   // P3-12: the page holds no PCM
  return { web_content_peak_mb: m.web_content_peak_mb, gpu_peak_mb: m.gpu_peak_mb, open_to_drawn_ms: m.open_to_drawn_ms };
};
