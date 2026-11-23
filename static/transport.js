/* Drum Spike v2 — transport (R17 step 4, #55)
   The native transport clock (AC: sync, slew, seek, play/pause/mix and the playhead and
   meter messages), the hit audition (AU, P4-40d) and the serial command queue (_appQ,
   appCmd) with the playhead arithmetic (appPos). Plain script (no build step), loaded
   after edit.js and before app.js. The clock sync and epoch logic is copied verbatim from
   app.js: it is the most timing-sensitive code in the file (P3-5b, #463, #471, P3-20b).
   The functions stay top-level bindings: the e2e tests call appPlay, appPause, appPos,
   auReset and DSAudition.state and read AC as page globals; the frame loop in app.js
   calls appTickPos each frame. Nothing below reads the DOM or the backend at load time. */
"use strict";

/* ---------------- native transport (P3-5b) ----------------
   In the app, play/pause/seek/mute/solo/clicks run on the native transport (crates/app/src/transport.rs).
   The page holds no audio. The transport sends the heard position on every change of its state and
   else 4 times a second (P3-15a; each message is a script eval in the WebView)
   ({t_ns, pos, playing, epoch}); the page extrapolates it for every frame it draws:
   pos + (now - sent time), where the sent time is t_ns on the page clock. The page clock offset is the
   smallest (arrival - t_ns) seen since play started (the fastest message has the least delay).
   The UI follows the playhead as before: S.playhead is set from the model each frame. When the
   user moves S.playhead during playback (ruler, lane click, Tab, Enter), the next frame sees that it
   differs from the value drawn last and asks the transport to seek (at most every 60 ms). */
const AC = { tickAt: 0, sync: null, syncs: [], syncTimer: null, mix: null, mixN: 0, tok: 0, active: false, starting: false, pos: 0, at: 0, running: false, off: Infinity, drawn: 0, errI: 0, epoch: null,
  pending: null, seeking: false, seekAt: 0, minEpoch: 0, stopping: false, underruns: 0, reopens: 0, last: null, sent: null, speed: 1 };
let _appQ = Promise.resolve();   // the transport commands run one after the other, in the order the user made them
function appCmd(fn) { const p = _appQ.then(fn); _appQ = p.catch(() => {}); return p; }
function appPos(now) { return AC.running ? AC.pos + (now - AC.at) / 1000 * AC.speed : AC.pos; }   // P3-20b: speed 0.5 at half speed
function appSlew(m) {   // #471: the shared slew/anchor of one sample (4 Hz playhead and 25 Hz meter messages)
  const recv = performance.now(), sent = m.t_ns / 1e6;
  AC.off = Math.min(AC.off, recv - sent);   // fallback until the first clock sync: the fastest message has the least delay
  /* #463: the slewed local clock. A hard re-anchor only on seek, play, pause, speed change, epoch change or a sample
     more than CP_HARD_S from the shown position; else the shown position gains CP_SLEW of the error plus the
     integrator (the line keeps its slope, the anchor time shifts). A new clock sync offset is slewed in the same
     way: appSync no longer steps AC.at, the next samples carry it in through this correction. */
  const hard = AC.sent === null || m.playing !== AC.running || m.epoch !== AC.epoch
    || (typeof m.speed === "number" && m.speed > 0 && m.speed !== AC.speed)
    || (hostMode() && m.playing && Math.abs(m.pos - appPos(recv)) > 0.25);   // #549: a locate of the host while it plays (its epoch may stay) re-anchors hard
  if (!hard) {
    const err = m.pos - appPos(recv), c = Math.abs(err) > CP_HARD_S ? 0 : CP_SLEW * err + AC.errI;   // song s
    AC.errI = Math.abs(err) > CP_HARD_S ? 0 : AC.errI + CP_SLEW_I * (err - AC.errI);
    if (c) AC.at -= c * 1000 / AC.speed;   // shown(recv) gains c: the anchor moves back by c / (speed / 1000)
  } else { AC.errI = 0; AC.pos = m.pos; }
  AC.sent = sent; if (hard) AC.at = sent + (AC.sync !== null ? AC.sync : AC.off); AC.running = !!m.playing;
  AC.epoch = m.epoch;
  if (typeof m.speed === "number" && m.speed > 0) AC.speed = m.speed;
}
function appOnPlayhead(m) {
  if (!m) return;
  if (HT.fault === "selected-track" && typeof m.pos === "number") {   // the fault run: the position of the host is mapped through the first clip of the selected row (the way of #554)
    const e = HT.view.find(x => x.srcs.includes(S.selTrack)), c = e && e.clips[0];
    if (c) m = Object.assign({}, m, { pos: m.pos - c.s + c.a });
  }
  if (!AC.active) {   // #518: the DAW starts playback in the DAW, not in the plugin window: the page follows
    if (!hostMode() || m.sid !== S.sid || AC.starting) return;   // the host's guest, or a Play of the page is starting (its own branch takes over)
    Object.assign(AC, { active: true, off: Infinity, minEpoch: 0, pending: null, seeking: false, running: false, errI: 0,
      epoch: null, pos: m.pos, at: performance.now(), drawn: m.pos, stopping: false, last: null, sent: null, speed: 1 });
    if (m.playing && !S.playing) {   // the host plays: the page shows the playing state and follows, with no play call
      PH.user = false;   // a host message is not a user seek (loadProject and the like leave PH.user set)
      S.playing = true; S.followHold = false; S.followResumeAt = null; $("#playBtn").textContent = "⏸";
      showSpeed(AC.speed);
      tick();
    }
  }
  if (m.sid !== S.sid) return;
  if (hostMode() && !m.playing && typeof m.pos === "number" && HL.on) { hostLocateSync(); HL.hostPos = m.pos; }   // #556: where a refused locate puts the playhead back
  if (m.error && !m.end) { setMsg(`audio: ${m.error} (trying again)`); AC.running = false; return; }
  if (m.epoch < AC.minEpoch || AC.seeking || AC.pending !== null) return;   // older than our last seek
  const metered = !!m.meters;   // #471: the meter message (25 Hz) carries the playhead fields too
  if (typeof m.pos !== "number" || typeof m.t_ns !== "number") return;   // an old bridge sends meters without them
  if (metered) onMeters(m);   // P3-16: the meters
  appSlew(m);
  if (metered) return;   // the meter message has no underruns, reopens or end fields
  AC.last = m;
  AC.underruns = m.underruns; AC.reopens = m.reopens;
  if (hostMode() && !m.playing) {   // #518: the DAW stopped (or located while stopped): the host's position is the truth
    if (S.playing) { S.playing = false; $("#playBtn").textContent = "▶"; showSpeed(1); cpStop(); spLeave("pause"); }
    AC.pos = m.pos; AC.errI = 0;   // a stopped message never slews: the next play re-anchors hard anyway
    AC.active = false;   // #549: the next host play takes the start branch above again (hard anchor, playing state); a stopped message that finds it false re-arms and clears it again
    PH.v = Math.max(0, Math.min(m.pos, maxDur()));
    HL.ref = PH.v;   // #556: the host knows this position: a write of it is no locate
    invalidate(F.HEAD); updateStatus();
  }
  if (m.end && S.playing && !AC.starting && !AC.stopping) pause();   // the transport ended on its own (project closed)
}
/* #518: the one host listener. The plugin shell sets window.__DS_HOST_MODE__ and B.hostTransport
   before the boot of the page, so app.js boot calls this one time and only when the mode is on.
   A shell that injects the transport later calls appHostFollow() itself (appPlay keeps its call
   for its own play). With the mode off this returns false and binds nothing: the standalone app
   runs no host listener of any kind (no timer re-runs it). The stub of the tests records the
   calls and pushes with send(m); the page's handler is appOnPlayhead. */
function appHostFollow() {
  if (!hostMode()) return false;
  const host = typeof B !== "undefined" && B.hostTransport;
  if (!host) return false;
  if (!host._appFollow) host._appFollow = m => appOnPlayhead(m);
  if (host.onPosition !== host._appFollow) host.onPosition = host._appFollow;
  hostLocateInstall();
  return true;
}
/* #556: a move of the playhead while the page is stopped, in host mode only. appTickPos (and so appSeekNow)
   runs only while the page plays; a click in the ruler of a stopped page only sets PH.v and PH.user (the
   setter of S.playhead in state.js). Here PH.user becomes an accessor once, when the host listener is bound,
   and a user move (PH.user set) calls B.hostTransport.locate(PH.v) itself: at most HL_MIN_MS apart (about
   25 a second) and the last position always. HL.ref is the position the host knows (its last message, the
   own stop of the page, the last locate): a write of that value is no move (loadProject writes the same).
   A refused locate (the promise rejects with the text) shows the text and puts PH.v back on the last position
   the host sent (HL.hostPos); under the pointer of a drag it waits for the pointer to go up, so a drag keeps
   the playhead under the pointer (#554). The standalone page never gets here: hostMode() is false. */
const HL_MIN_MS = 40;
const HL = { on: false, down: false, seq: 0, sentAt: 0, timer: null, pending: null, hostPos: null, base: null, refusedSeq: 0, ref: 0, sid: null, gen: 0 };
/* A change of project (S.sid) drops everything that belongs to the old one: the waiting locate and its timer,
   the refusal mark and the positions to go back to. */
function hostLocateSync() {
  if (HL.sid === S.sid) return;
  HL.sid = S.sid; HL.pending = null; HL.refusedSeq = 0; HL.hostPos = null; HL.base = null; HL.ref = PH.v;
  clearTimeout(HL.timer); HL.timer = null;
}
function hostLocateInstall() {
  if (HL.on) return;
  HL.on = true; HL.ref = PH.v; HL.sid = S.sid;
  let user = PH.user;
  Object.defineProperty(PH, "user", { enumerable: true, configurable: true, get() { return user; }, set(x) {
    user = x;
    if (!x || S.playing || typeof PH.v !== "number" || !isFinite(PH.v)) return;   // playing: the tick seeks
    hostLocateSync();
    if (AC.stopping || !S.loaded) { HL.ref = PH.v; return; }   // the own stop of the page (appPause) and a write of a load (the clamp) are no locate
    if (Math.abs(PH.v - HL.ref) < 1e-9) return;
    if (HL.hostPos === null && HL.base === null) HL.base = HL.ref;   // U4: where a refusal goes back to when the host sent no position yet
    HL.ref = PH.v;
    hostLocateSend(PH.v);
  } });
  // the primary button (not ctrl+click: that opens the context menu and may give no mouseup)
  window.addEventListener("mousedown", ev => { if (ev.button === 0 && !ev.ctrlKey) HL.down = true; }, true);
  const up = () => { HL.down = false; if (HL.refusedSeq && HL.refusedSeq === HL.seq && HL.pending === null) hostLocateBack(); };
  window.addEventListener("mouseup", up, true); window.addEventListener("blur", up); window.addEventListener("contextmenu", up, true);
}
function hostLocateArm(ms) {   // one timer at most
  if (!HL.timer) HL.timer = setTimeout(() => { HL.timer = null; hostLocateFlush(); }, ms);
}
function hostLocateSend(t) { HL.pending = t; HL.gen = S.loaded; hostLocateFlush(); }
/* U2, the rule for a move while the host starts (AC.starting, up to 2 s): the move waits. When the start is
   settled and the page plays, the waiting locate is dropped: PH.user is still set, and the tick seeks through
   the path of a move while playing. When the start failed, the locate goes out then.
   Round 3: the rate check is made here, for the direct call and for the timer alike. A timer that fires late
   must not send a position a few ms after a direct send; a send clears the armed timer. */
function hostLocateFlush() {
  hostLocateSync();
  if (HL.pending === null) return;
  if (AC.starting) { hostLocateArm(HL_MIN_MS); return; }
  const wait = HL.sentAt + HL_MIN_MS - performance.now();
  if (wait > 0) { hostLocateArm(wait); return; }
  clearTimeout(HL.timer); HL.timer = null;
  const t = HL.pending; HL.pending = null;
  if (S.playing || !S.loaded || HL.gen !== S.loaded) return;   // playing now: the tick seeks; a load came between: the move is old
  const host = typeof B !== "undefined" && B.hostTransport;
  if (!host || typeof host.locate !== "function") return;
  const seq = ++HL.seq, sid = S.sid; HL.sentAt = performance.now();
  const refused = e => { setMsg(String(e && e.message || e)); if (sid === S.sid) hostLocateRefused(seq); };
  try { Promise.resolve(host.locate(t)).catch(refused); } catch (e) { refused(e); }
}
function hostLocateRefused(seq) {
  if (seq !== HL.seq || HL.pending !== null || S.playing) return;   // a newer locate decides
  HL.refusedSeq = seq;
  if (!HL.down) hostLocateBack();
}
function hostLocateBack() {
  HL.refusedSeq = 0;
  hostLocateSync();
  const to = HL.hostPos !== null ? HL.hostPos : HL.base;
  if (to === null || S.playing) return;
  PH.v = HL.ref = Math.max(0, Math.min(to, maxDur()));
  if (PH.v < S.view.t0 || PH.v > S.view.t1) {   // U5: a jump moved the view; the playhead must be in sight again
    const d = S.view.t1 - S.view.t0, v = viewClamp(PH.v - d / 2, PH.v + d / 2);
    setView(v.t0, v.t1);
  }
  invalidate(F.HEAD); updateStatus();
}
/* Clock sync: the channel may deliver every message late (in the app, by about 25 ms), so the min-delay offset
   is wrong by that delay. A command answers within a millisecond: transport_status carries t_ns, and the
   middle of the call is the page time of that t_ns (error: half the round trip). The best of the last 8
   samples (smallest round trip) sets the offset; it is renewed every second. */
function appSync() {
  const a = performance.now();
  return B.transportStatus().then(st => {
    const b = performance.now();
    if (!st || typeof st.t_ns !== "number") return;
    AC.syncs.push({ rtt: b - a, off: (a + b) / 2 - st.t_ns / 1e6 });
    if (AC.syncs.length > 8) AC.syncs.shift();
    AC.sync = AC.syncs.reduce((best, x) => x.rtt < best.rtt ? x : best).off;
    // #463: a better offset is no longer stepped in here (up to 250 ms later it would jump); the next playhead
    // samples carry it in through the slew of appOnPlayhead, so the shown playhead never steps.
  }).catch(() => {});
}
function appSeekNow() {
  const target = AC.pending; AC.pending = null; AC.seeking = true; AC.seekAt = performance.now(); AC.errI = 0;   // #463: the seek re-anchors
  if (hostMode()) {   // #509: locate goes to the host; its next position message re-anchors the page
    const host = B.hostTransport;
    try { if (host && typeof host.locate === "function") Promise.resolve(host.locate(target)).catch(e => setMsg(String(e && e.message || e))); } catch (e) { setMsg(String(e && e.message || e)); }   // #556: a refusal shows its text
    AC.seeking = false;
    return;
  }
  appCmd(() => B.transportSeek(target)).then(r => { AC.minEpoch = r.epoch; })
    .catch(e => setMsg(`seek failed: ${e.message}`, "error")).finally(() => { AC.seeking = false; });
}
function appTickPos() {   // the position to draw in this frame
  const now = AC.tickAt = performance.now();
  if (PH.user) {   // the user moved the playhead (P4-1: the setter of S.playhead says so)
    PH.user = false; cpStop(); spLeave("seek");
    AC.pending = PH.v; AC.pos = PH.v; AC.running = false; AC.sent = null;
  }
  if (AC.pending !== null && !AC.seeking && now - AC.seekAt >= 60) appSeekNow();
  AC.drawn = (AC.pending !== null || AC.seeking) ? AC.pos : appPos(now);
  return AC.drawn;
}
async function appPlay(speed, returnKey) {   // speed: 0.5 is half speed (P3-20b); undefined plays at normal speed; returnKey: the play-and-return key (#207): the next Space or the same key stops and returns to the start of this play
  if (S.playing || AC.starting) return;
  if (!needProject()) return;
  auReset();   // P4-40d: a play replaces an audition (the host does the same)
  AC.starting = true;
  try {
    const click = SETTINGS().click;
    const events = {};
    for (const n of S.order) events[n] = (S.events[n] || []).map(e => e.t);
    Object.assign(AC, { active: true, off: Infinity, minEpoch: 0, pending: null, seeking: false, running: false, errI: 0, epoch: null,
      pos: S.playhead, at: performance.now(), drawn: S.playhead, stopping: false, last: null, sent: null, sync: null, syncs: [],
      speed: speed || 1 });
    S.playStartPos = S.playhead; PH.user = false;   // P4-1: a move from here on seeks
    S.playReturn = returnKey || null;   // #207: play-and-return, cleared by every play that the plain keys start
    const pos0 = S.playhead, tok = ++AC.tok;   // messages of an earlier play (its channel) are dropped by the token
    if (hostMode()) {   // #509: the host plays: the page asks the host transport and follows the positions it sends
      const host = B.hostTransport;
      if (!host || typeof host.play !== "function") throw new Error("the host transport is missing");
      appHostFollow();   // #518: the listener is the page's own, registered once (not per play)
      await host.play(pos0, speed);
      AC.running = true;   // the extrapolation runs until the first position of the host re-anchors it
      S.playing = true; S.followHold = false; S.followResumeAt = null; $("#playBtn").textContent = "⏸";
      showSpeed(AC.speed);
      tick();
      return;   // the app transport (mix, clicks, play, the clock sync) never runs in host mode
    }
    const r = await appCmd(async () => {
      await B.transportMix(S.muted, S.solo);
      await B.transportClicks(click.enabled, click.volume, click.pitches);
      return B.transportPlay(S.sid, pos0, events, m => { if (tok === AC.tok) appOnPlayhead(m); }, speed);
    });
    AC.minEpoch = r.epoch; AC.names = r.names || null;   // P3-16: the order of the meter values
    AC.speed = r.speed || 1; showSpeed(AC.speed);
    if (r.names && r.channels) markTrackChannels(r.names, r.channels);
    await appSync(); await appSync(); await appSync();
    clearInterval(AC.syncTimer); AC.syncTimer = setInterval(() => { if (AC.active) appSync(); }, 1000);
    S.playing = true; S.followHold = false; S.followResumeAt = null; $("#playBtn").textContent = "⏸";
    if (!r.ready) setMsg("audio is slow to start");
    else if (r.note) setMsg(r.note);   // P3-16: the saved output device is missing; the system default plays
    tick();
  } catch (e) {
    AC.active = false;
    setMsg(`playback failed: ${e.message.replace(/^\d+: /, "")}`, "error");
  } finally { AC.starting = false; }
}
function appPause() {
  if (!S.playing) return;
  const p = Math.max(0, Math.min(appPos(performance.now()), maxDur()));
  S.playing = false; AC.stopping = true; AC.active = false; clearInterval(AC.syncTimer); cpStop(); spLeave("pause");
  $("#playBtn").textContent = "▶"; showSpeed(1);
  S.playhead = p;
  if (hostMode()) {   // #509: the host stops; its last position message has already re-anchored the page
    const host = B.hostTransport;
    if (host && typeof host.stop === "function") host.stop(p);
    AC.stopping = false;
    invalidate(F.HEAD);
    return;
  }
  appCmd(() => B.transportStop(false)).then(r => {
    // the transport stopped within a block of our estimate: take its position if nothing moved the playhead since
    if (S.playhead === p && !S.playing && r && typeof r.pos === "number" && r.rate > 0) { S.playhead = Math.min(r.pos, maxDur()); invalidate(F.HEAD); updateStatus(); }
  }).catch(e => setMsg(`stop failed: ${e.message}`, "error"));
  invalidate(F.HEAD);
}
function appMix() {   // AC.mix: the round trip of the last change and the transport's own latency (the self-test reads it)
  if (hostMode()) return;   // #539: no mute and solo in host mode
  const a = performance.now();
  appCmd(() => B.transportMix(S.muted, S.solo))
    .then(r => {   // inbound: from the call to the change in the service, on the page clock estimated from the playhead messages
      const off = AC.sync !== null ? AC.sync : AC.off;
      const inbound = r && r.t_ns && isFinite(off) ? Math.max(0, r.t_ns / 1e6 + off - a) : null;
      AC.mix = { rtt: performance.now() - a, latency_ms: r && r.latency_ms !== undefined ? r.latency_ms : null, inbound };
      AC.mixN++;
    })
    .catch(e => setMsg(`mute/solo failed: ${e.message}`, "error"));
}
/* ---------------- P4-40d: hit audition and loop ----------------
   The key "Audition Hit" (A in the Pro Tools preset, Option+A in Cubase) plays the selected track alone, from AU_PRE
   before the selected hit to AU_POST after it. Shift with the key loops the window until Escape or Space. A range
   selection (LN.sel) plays and loops the range, and the ruler shows the loop band. The call is a transport_play with
   start, end, loop and tracks (crates/app/src/transport.rs): the host solos the track for this play only, mute and solo
   stay as the page set them, and no playhead messages come, so the playhead of the page does not move. Half-Speed
   applies when the transport plays at half speed, or an audition at half speed runs, as the key is pressed. The click
   is off during an audition: it is the sound of the track that counts. Setting "Audition on Tab" (SETTINGS().ui.auditionTab):
   a step to the next or previous hit (Tab, gotoEvent) plays that hit, unless the page plays. Status: "audition s top 12.345 s". */
const AU_PRE = 0.030, AU_POST = 0.250;
const AU = { on: false, loop: false, speed: 1, tok: 0, timer: null, win: null, track: null, t: null, until: Infinity };   // until: when a single hit has ended (page clock)   // win: {t0, t1} while the ruler shows a loop
function auTarget(hit) {   // the window to play: a given hit {track, t}, else the range selection, else the selected hit
  if (hit) return { track: hit.track, t0: Math.max(0, hit.t - AU_PRE), t1: hit.t + AU_POST, hit: hit.t, range: false };
  const rg = LN.sel && S.tracks[LN.sel.track] && LN.sel.t1 > LN.sel.t0 ? LN.sel : null;
  if (rg) return { track: rg.track, t0: rg.t0, t1: rg.t1, hit: null, range: true };
  const e = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  return e ? { track: S.sel.track, t0: Math.max(0, e.t - AU_PRE), t1: e.t + AU_POST, hit: e.t, range: false } : null;
}
function auReset() {
  clearTimeout(AU.timer); AU.timer = null; AU.tok++; AU.on = false; AU.loop = false;
  if (AU.win) { AU.win = null; invalidate(F.VIEW); }
}
function auStop(quiet) {   // Escape, Space, a new play: the host pauses and gives mute and solo back
  if (!AU.on || performance.now() > AU.until) return false;   // none, or a single hit that has ended: the key keeps its own job
  auReset();
  appCmd(() => B.transportStop(false)).catch(e => setMsg(`stop failed: ${e.message}`, "error"));
  if (!quiet) setMsg("audition stopped");
  return true;
}
async function audition(loop, hit, shot) {   // loop: Shift; hit: {track, t} of a step with Audition on Tab; shot: a track name whose layer one-shot plays once (P4-40e)
  if (hostAbsent("audio")) return;   // #539: the host plays the audio; the page opens no second output
  if (!needProject()) return;
  let g;
  if (shot) {
    const l = lyLayer(shot);
    if (!l) { setMsg(`audition: ${shot} has no one-shot`); return; }
    g = { track: shot, t0: 0, t1: l.frames / l.sr, hit: null, range: false, shot: true };
  } else {
    g = auTarget(hit);
    if (!g) { setMsg("audition: select a hit or a range first"); return; }
  }
  const looped = g.range || !!loop;
  const speed = S.playing ? (AC.speed === 0.5 ? 0.5 : 1) : (AU.on ? AU.speed : 1);
  if (S.playing) pause();
  auReset();
  const tok = AU.tok;
  Object.assign(AU, { on: true, loop: looped, speed, track: g.track, t: g.hit, until: Infinity });
  if (looped) { AU.win = { t0: g.t0, t1: g.t1 }; invalidate(F.VIEW); }
  try {
    const click = SETTINGS().click;
    const r = await appCmd(async () => {
      await B.transportMix(S.muted, S.solo);
      await B.transportClicks(false, click.volume, click.pitches);
      const o = { track: g.track, start: g.t0, end: g.t1, loop: looped, speed };
      if (g.shot) o.shot = g.track;   // P4-40e: the one-shot preview voice at the window start
      return B.transportAudition(S.sid, o);
    });
    if (tok !== AU.tok) return;   // stopped or replaced while it started
    if (!looped) {
      const ms = (g.t1 - g.t0) / speed * 1000 + 20;
      AU.until = performance.now() + ms;
      AU.timer = setTimeout(() => { if (tok === AU.tok) { AU.on = false; AU.timer = null; } }, ms);
    }
    setMsg(g.shot ? `one-shot of ${g.track}: ${g.t1.toFixed(3)} s`
      : g.range ? `audition ${g.track} ${g.t0.toFixed(3)} to ${g.t1.toFixed(3)} s (loop)` : `audition ${g.track} ${g.hit.toFixed(3)} s${looped ? " (loop)" : ""}`);
    if (r && r.note) setMsg(r.note);
  } catch (e) {
    if (tok === AU.tok) auReset();
    setMsg(`audition failed: ${e.message.replace(/^\d+: /, "")}`, "error");
  }
}
window.DSAudition = { state: () => ({ on: AU.on, loop: AU.loop, speed: AU.speed, track: AU.track, t: AU.t, win: AU.win && { ...AU.win } }) };   // the e2e tests read it
