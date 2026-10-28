/* Self-test step "playhead_steps" (#463): how much does the drawn playhead jump?
   For every follow mode the step plays the open project on the real transport (silent, master gain 0)
   and samples the on-screen x of #playheadOverlay at every animation frame (getComputedStyle, the same
   value the compositor shows). Between two frames the playhead may only advance by the view's px/s:
   anything beyond that is a step. The step is reported in px and in ms of song time (px / (px/s)),
   with the cause read from the page state at that frame:
   - re-arm: the compositor playhead was armed or re-armed between the frames (CP.arms moved)
   - view: the view moved between the frames (page turn, smooth glide start, seek)
   - clock sync: AC.sync changed between the frames
   - frame path: the page drew the playhead itself (no compositor animation, no strip)
   - compositor: none of the above (the animation ran on and still stepped)
   The result goes to report.metrics.playhead_steps; `need` fails the step when a mode shows a step
   over the budget in sc.playhead_steps.max_jump_px (default 1 px) outside the page-turn cause. */
window.__selftestPlayheadSteps = async ({ report, need, until, sleep, now, r2, T, sc }) => {
  const cfg = Object.assign({ seconds: 10, settle_ms: 800, max_jump_px: 1.0, views: [30], modes: ["off", "page", "smooth"] }, sc.playhead_steps || {});
  const el = document.querySelector("#playheadOverlay");
  need(!!el, "no playhead overlay");
  need(!S.playing, "the page is playing already");

  const xOf = () => {
    if (getComputedStyle(el).display === "none") return null;   // #463: the playhead left the view; no transform is shown
    const tf = getComputedStyle(el).transform;
    if (!tf || tf === "none") return null;
    try { return new DOMMatrix(tf).m41; } catch (e) {}
    const m = tf.match(/matrix\(([^)]+)\)/);
    if (!m) return null;
    const v = m[1].split(",").map(Number);
    return v.length >= 6 ? v[4] : v[12];
  };
  const pxPerS = () => laneWidth() / viewDur() * AC.speed;   // screen px per song second

  const out = { budget_px: cfg.max_jump_px, modes: [] };
  for (const mode of cfg.modes) for (const viewS of cfg.views) {
    if (S.playing) { document.querySelector("#playBtn").click(); await until(() => !S.playing, "pause", 5000); }
    S.followHold = false;
    setFollow(mode);
    const ph = Math.min(60, Math.max(0, maxDur() - 45));
    S.playhead = ph;
    setView(ph, Math.min(ph + viewS, maxDur()));
    /* #463: in off mode the view stands still, so the playhead leaves a short view (1 s) after that long; the
       measured span is what is left after the settle wait. Shrink the settle wait so a short view still samples
       (at 0.8 s settle a 1 s view had ~0.2 s left and came out with 0 samples). */
    const shownMs = Math.max(0, Math.min(ph + viewS, maxDur()) - ph) / (AC.speed || 1) * 1000;
    const settle = Math.min(cfg.settle_ms, shownMs / 3);
    const res = { mode, view_s: viewS, px_per_song_s: r2(pxPerS() / (AC.speed || 1)), samples: 0, max_jump_px: 0, max_jump_ms: 0, jumps_over_1px: 0, jumps_over_2px: 0, top: [], causes: {}, arms: 0, view_moves: 0 };
    document.querySelector("#playBtn").click();
    await until(() => S.playing && AC.running, `playback to start (${mode})`, 30000);
    await new Promise(done => {
      let lastT = null, lastX = null, lastArms = CP.arms, lastT0 = S.view.t0, lastSync = AC.sync;
      const endAt = performance.now() + settle + cfg.seconds * 1000;
      // #463: a stalled rAF (occluded window, throttled WebKit) must settle, not hang to the watchdog
      let settled = false;
      const finish = () => { if (!settled) { settled = true; clearInterval(guard); done(); } };
      const guard = setInterval(() => { if (performance.now() > endAt + 20000) { res.no_frames = res.samples === 0; finish(); } }, 1000);
      const loop = t => {
        if (t >= endAt || !S.playing) { finish(); return; }
        if (t - (endAt - cfg.seconds * 1000) >= settle) {   // past the settle window
          const x = xOf();
          const armDelta = CP.arms - lastArms;
          const viewMoved = S.view.t0 !== lastT0;
          const syncMoved = AC.sync !== lastSync;
          const framePath = !CP.anim;
          const cause = viewMoved ? "view" : armDelta !== 0 ? "re-arm" : syncMoved ? "clock sync" : framePath ? "frame path" : "compositor";
          if (x !== null && lastX !== null && lastT !== null) {
            const dt = t - lastT, dx = x - lastX, rate = pxPerS();
            const jump = rate > 0 ? dx - rate * dt / 1000 : 0;   // px beyond the glide
            const ms = rate > 0 ? Math.abs(jump) / rate * 1000 : 0;
            res.samples++;
            if (Math.abs(jump) > Math.abs(res.max_jump_px)) {
              res.max_jump_px = r2(jump);
              res.max_jump_ms = r2(ms);
              res.max_cause = cause;
              res.max_at_ms = r2(t - (endAt - cfg.seconds * 1000));
            }
            if (Math.abs(jump) > 1) {
              res.jumps_over_1px++;
              if (res.top.length < 12) res.top.push({ at_ms: r2(t - (endAt - cfg.seconds * 1000)), jump_px: r2(jump), jump_ms: r2(ms), cause,
                dt: r2(dt), anim: CP.anim ? CP.anim.playState : null, until_left_ms: r2((CP.until - AC.drawn) * 1000), x: r2(x) });
            }
            res.causes[cause] = (res.causes[cause] || 0) + 1;
          }
          if (armDelta !== 0) { res.arms += armDelta; lastArms = CP.arms; }
          if (viewMoved) { res.view_moves++; lastT0 = S.view.t0; }
          if (syncMoved) lastSync = AC.sync;
          lastT = t; if (x !== null) lastX = x;
        }
        requestAnimationFrame(loop);
      };
      requestAnimationFrame(loop);
    });
    if (S.playing) { document.querySelector("#playBtn").click(); await until(() => !S.playing, `pause (${mode})`, 5000); }
    need(res.samples > 0, `${mode} (view ${viewS} s): no rAF frames sampled (occluded window or throttled WebKit?)`);
    out.modes.push(res);
    report.metrics.playhead_steps = out;   // kept also when the budget fails: the jumps and their causes
    need(Math.abs(res.max_jump_px) <= cfg.max_jump_px || res.max_cause === "view",
         `${mode} (view ${viewS} s): the playhead stepped ${res.max_jump_px} px (${res.max_jump_ms} ms, cause ${res.max_cause})`);
  }
  setFollow("smooth");
  return out;
};
