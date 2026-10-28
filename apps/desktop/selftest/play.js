/* Self-test step "play" (P3-5b). Loaded by selftest.rs before driver.js, which calls
   window.__selftestPlay({report, need, until, sleep, now, r2, T, sc}) for the step "play".
   Plays the open project for sc.play.seconds (60) on the real transport and the real device, through the page's
   own buttons (play, M, S, the click button, the playhead), and measures:
   - underruns and stream errors of the transport over the whole run (target 0)
   - the drawn playhead against the transport: transport_status is called about every probe_ms; the page time of
     its answer is the middle of the call (error of that guess: half the round trip, reported); the drawn
     position at that time comes from the page (P4-1: drawnPlayheadAt, the compositor animation that ran then, or
     the page's own ticks, linear between two of them). The probe has no animation-frame loop of its own: a loop
     would keep the display link of the app at 60 Hz and add its cost to the CPU the scenario measures. A page
     without drawnPlayheadAt (before P4-1) is sampled at every animation frame, as before
   - the mute and solo changes: page handler -> command -> reply (round trip) and, from the transport, the time
     from the change to the moment the first block with the new gains is heard (latency_ms)
   - the seek: from S.playhead = x to the transport playing again
   The run is silent: it refuses to start unless the master gain of the transport is 0 (the app is started with
   DRUMSPIKE_MASTER_GAIN=0; apps/desktop/scripts/self-test.sh does that) or the scenario sets allow_audible. The
   gain is only the master gain after the mix: the full mix and the clicks run. Nothing here touches the system
   volume or the audio settings. */
window.__selftestPlay = async ({ report, need, until, sleep, now, r2, T, sc }) => {
  const cfg = Object.assign({ seconds: 60, probe_ms: 250 }, sc.play || {});
  const q = (v, p) => { const s = v.slice().sort((a, b) => a - b); return s.length ? s[Math.min(s.length - 1, Math.floor(p * s.length))] : null; };
  const num = (v) => v === null ? null : r2(v);
  const status = () => T.invoke("transport_status", {});

  let st = await status();
  need(cfg.allow_audible || st.master === 0, `the master gain is ${st.master}, not 0: start the app with DRUMSPIKE_MASTER_GAIN=0 (a device run must be silent)`);
  need(S.order.length >= 2, "the play step needs at least 2 tracks");
  need(!S.playing, "the page is playing already");

  // P4-1: the measurement variants (not in play.json): view_s zooms to a view of that many seconds from the playhead,
  // so smooth follow glides (the 300 s view of the scenario stands still), follow sets the follow mode
  if (cfg.follow) setFollow(cfg.follow);
  if (cfg.view_s) { setView(S.playhead, S.playhead + cfg.view_s); S.followHold = false; await sleep(500); }
  const t0 = now();
  document.querySelector("#playBtn").click();
  try { await until(() => S.playing && AC.running, "playback to start", 30000); }
  catch (e) {
    const now_st = await status().catch(x => ({ error: String(x) }));
    throw new Error(`${e.message}; status line: "${document.querySelector("#stMsg").textContent}"; playing=${S.playing} running=${AC.running} starting=${AC.starting}; transport: ${JSON.stringify(now_st)}`);
  }
  const m = report.metrics;
  m.play_start_ms = r2(now() - t0);
  st = await status();
  need(cfg.allow_audible || st.master === 0, `the transport runs with master gain ${st.master}`);
  need(st.open && st.tracks >= 1, "the transport is not open");
  need(st.peak >= 0, "no stats");
  const base = { underruns: st.underruns, stream_xruns: st.stream_xruns, stream_warnings: st.stream_warnings, stream_errors: st.stream_errors, callbacks: st.callbacks, peak: st.peak };
  m.play_device = st.device;
  m.play_tracks = st.tracks;
  m.play_hits = st.hits;
  m.play_open_ms = st.open_ms;
  m.play_master = st.master;
  // P3-16: the meter messages of the window (about 25 Hz; the master meter runs before the master gain)
  const meter0 = { n: AC.meterN || 0, t: now() };
  let masterPeak = 0;
  const meterWatch = setInterval(() => { const x = AC.meters; if (x && x.master) masterPeak = Math.max(masterPeak, x.master[0], x.master[1]); }, 100);

  // animation frames: [page time, S.playhead] (pages before P4-1 only)
  const frames = [];        // keyed by the time the position was computed (AC.tickAt): the value is right for that time
  const late = [];          // keyed by the time this callback ran, after the frame's drawing work: includes the work
  const paged = typeof drawnPlayheadAt === "function";
  const cp0 = paged ? { arms: CP.arms } : null;
  let run = !paged, lastTick = -1;
  const loop = () => {
    if (AC.tickAt !== lastTick) { lastTick = AC.tickAt; frames.push([AC.tickAt, S.playhead]); late.push([performance.now(), S.playhead]); }
    if (run) requestAnimationFrame(loop);
  };
  if (!paged) requestAnimationFrame(loop);

  const probes = [];          // [t_a, t_b, pos, playing]
  const mutes = [];           // {rtt, latency_ms, what}
  const seeks = [];           // ms until playing again
  const seekWindows = [];     // [start, end + 600 ms] on the page clock: the drawn playhead jumps there, no probe counts
  const byName = n => document.querySelector(`.track[data-name="${n}"]`);
  const names = S.order.slice();
  async function press(btn, what) {   // the real button; waits for the reply of the mix command
    const n0 = AC.mixN;
    btn.click();
    await until(() => AC.mixN > n0, `the reply of ${what}`, 5000);
    const r = AC.mix;
    // click to heard = inbound (call to the change in the service) + block latency (change to the block heard)
    const ok = r.latency_ms !== null && r.inbound !== null;
    mutes.push({ what, rtt_ms: r2(r.rtt), inbound_ms: r.inbound === null ? null : r2(r.inbound),
                 latency_ms: r.latency_ms === null ? null : r2(r.latency_ms), total_ms: ok ? r2(r.inbound + r.latency_ms) : null });
  }
  const plan = [   // [second, action]
    [0.15, async () => { await press(byName(names[0]).querySelector(".bMute"), "mute 1"); }],
    [0.22, async () => { await press(byName(names[1]).querySelector(".bMute"), "mute 2"); }],
    [0.30, async () => { await press(byName(names[0]).querySelector(".bMute"), "unmute 1"); await press(byName(names[1]).querySelector(".bMute"), "unmute 2"); }],
    [0.40, async () => { await press(byName(names[2 % names.length]).querySelector(".bSolo"), "solo"); }],
    [0.46, async () => { await press(byName(names[2 % names.length]).querySelector(".bSolo"), "unsolo"); }],
    [0.55, async () => {   // a click on the ruler while playing: the user path to a seek
      const target = Math.min(maxDur() * 0.5, S.playhead + 20);
      const e0 = AC.minEpoch, t = now();
      S.playhead = target;
      await until(() => AC.minEpoch > e0 && AC.running, "the seek", 15000);
      seeks.push(r2(now() - t));
      seekWindows.push([t - 300, now() + 600]);
    }],
    [0.70, async () => { document.querySelector("#clickBtn").click(); await sleep(300); document.querySelector("#clickBtn").click(); }],
    [0.80, async () => { await press(byName(names[0]).querySelector(".bMute"), "mute 1 again"); await press(byName(names[0]).querySelector(".bMute"), "unmute 1 again"); }],
  ];
  let next = 0;
  const end = now() + cfg.seconds * 1000;
  const start = now();
  while (now() < end) {
    const a = performance.now();
    const s = await status();
    const b = performance.now();
    probes.push([a, b, s.pos, s.playing, appPos((a + b) / 2)]);
    const frac = (now() - start) / (cfg.seconds * 1000);
    while (next < plan.length && frac >= plan[next][0]) { await plan[next][1](); next++; }
    if (!S.playing) throw new Error(`playback stopped by itself at ${S.playhead.toFixed(2)} s`);
    await sleep(cfg.probe_ms);
  }
  run = false;
  const fin = await status();
  document.querySelector("#playBtn").click();
  await until(() => !S.playing, "pause", 5000);
  const after = await status();
  m.play_paused_pos_diff_ms = r2(Math.abs(after.pos - S.playhead) * 1000);

  const mk = fr => t => {
    let lo = 0, hi = fr.length - 1;
    if (!fr.length || t < fr[0][0] || t > fr[hi][0]) return null;
    while (hi - lo > 1) { const mid = (lo + hi) >> 1; if (fr[mid][0] <= t) lo = mid; else hi = mid; }
    const [ta, pa] = fr[lo], [tb, pb] = fr[hi];
    return tb > ta ? pa + (pb - pa) * (t - ta) / (tb - ta) : pa;
  };
  const at = paged ? t => drawnPlayheadAt(t, false) : mk(frames), atLate = paged ? t => drawnPlayheadAt(t, true) : mk(late);
  if (paged) {   // the page's ticks in the window: the frame intervals of the ticks that drew the playhead themselves
    for (const x of CP.hist) if (x[0] >= start && x[0] <= end) frames.push([x[0], x[1], x[3]]);
  }
  const err = [], rtt = [], signed = [], model = [], errLate = [];
  for (const [a, b, pos, playing, modelPos] of probes) {
    const d = at((a + b) / 2);
    if (d === null || !playing || seekWindows.some(([w0, w1]) => a >= w0 && b <= w1)) continue;
    const dl = atLate((a + b) / 2);
    if (dl !== null) errLate.push(Math.abs(dl - pos) * 1000);
    err.push(Math.abs(d - pos) * 1000); rtt.push(b - a); signed.push((d - pos) * 1000); model.push((modelPos - pos) * 1000);
  }
  const lat = mutes.filter(x => x.total_ms !== null);
  clearInterval(meterWatch);
  const meterN = (AC.meterN || 0) - meter0.n, meterS = (now() - meter0.t) / 1000;
  m.play = {
    meters: { messages: meterN, per_s: num(meterN / meterS), master_peak_max: num(masterPeak) },
    seconds: cfg.seconds, view_s: cfg.view_s || null, follow: S.followMode, probes: probes.length, animation_frames: frames.length,
    strip: typeof SP !== "undefined" ? Object.assign({ leaving: SP.leaving, leave_frames: SP.leaveFrames }, SP.n) : null,   // P4-2: a fallback that still waits shows here
    compositor: paged ? { arms: CP.arms - cp0.arms, ticks: frames.length, ticks_compositor: frames.filter(f => f[2]).length, per_s: num(frames.length / cfg.seconds) } : null, frame_ms_median: frames.length > 2 ? num(q(frames.slice(1).map((f, i) => f[0] - frames[i][0]), 0.5)) : null, frame_ms_p95: frames.length > 2 ? num(q(frames.slice(1).map((f, i) => f[0] - frames[i][0]), 0.95)) : null,
    underruns: fin.underruns - base.underruns, underrun_frames: fin.underrun_frames, stream_errors: fin.stream_errors - base.stream_errors,
    stream_xruns: fin.stream_xruns - base.stream_xruns, stream_warnings: fin.stream_warnings - base.stream_warnings, load_note: sc.play && sc.play.load_note,
    reopens: fin.reopens, callbacks: fin.callbacks - base.callbacks, pre_gain_peak: fin.peak, master_after: fin.master,
    playhead_error_ms: err.length ? { n: err.length, median: num(q(err, 0.5)), p95: num(q(err, 0.95)), max: num(Math.max(...err)),
      status_rtt_median: num(q(rtt, 0.5)), incl_frame_work_median: num(q(errLate, 0.5)), incl_frame_work_p95: num(q(errLate, 0.95)), incl_frame_work_max: errLate.length ? num(Math.max(...errLate)) : null, signed_median: num(q(signed, 0.5)), model_signed_median: num(q(model, 0.5)), model_abs_p95: num(q(model.map(Math.abs), 0.95)), status_rtt_max: num(Math.max(...rtt)) } : { n: 0, note: "no animation frames (window hidden?)" },
    mute: { n: mutes.length, with_latency: lat.length, inbound_ms_max: lat.length ? num(Math.max(...lat.map(x => x.inbound_ms))) : null, latency_ms_max: lat.length ? num(Math.max(...lat.map(x => x.latency_ms))) : null,
      latency_ms_median: lat.length ? num(q(lat.map(x => x.latency_ms), 0.5)) : null,
      rtt_ms_max: num(Math.max(...mutes.map(x => x.rtt_ms))), total_ms_max: lat.length ? num(Math.max(...lat.map(x => x.total_ms))) : null, all: mutes },
    seek_settle_ms: seeks,
  };
  need(m.play.underruns === 0, `${m.play.underruns} underruns in ${cfg.seconds} s`);
  need(m.play.stream_errors === 0, `${m.play.stream_errors} stream errors`);
  need(cfg.allow_audible || fin.master === 0, `master gain ${fin.master} at the end`);
  need(fin.peak > 0, "the pre-gain peak is 0: the mix did not run");
  need(m.play.meters.per_s >= 15 && m.play.meters.per_s <= 32, `meter messages ${m.play.meters.per_s} per s`);
  need(masterPeak > 0, "the master meter stayed at 0 (it runs before the master gain)");
  need(lat.length >= 4, `only ${lat.length} mute changes had a latency`);
  return m.play;
};
