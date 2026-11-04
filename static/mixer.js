/* Drum Spike v2 — mixer (refactor plan step 5.2, R9 A2)
   Volume, pan and meters per track, the master fader and meter (MT), and the click settings
   send appClicksSoon. Moved verbatim from app.js. Plain script (no build step), loaded after
   shell.js and before app.js. The names stay top-level bindings (page globals): the e2e tests read MT. */
"use strict";

/* ---------------- mixer: volume, pan and meters per track; master fader and meter (P3-16) ----------------
   Pro Tools style. Volume -inf to +12 dB (the fader position follows VOL_TAPER: 0 dB at 3/4), pan -1 (left) to
   +1 (right); both are kept in the project (track_set_mix, marks it dirty). The master volume is kept in the app
   settings (transport_master). A drag moves a control; with Option the drag is ten times finer; a double-click
   or an Option-click resets it (0 dB, centre). The value shows in a tooltip while the mouse is on the control.
   Monitor settings only: no export reads them.
   Meters: the transport sends one message for all tracks about 25 times a second while it plays ({meters:
   [l0, r0, ...], master: [l, r]}, linear peaks, post-fader; a muted track still shows its level). A level falls
   at METER_FALL dB/s; the peak-hold line stays HOLD_MS and then falls at HOLD_FALL dB/s; a peak at or above
   0 dBFS lights the red clip light until a click on it. A message or a fall step changes the model and calls
   invalidate(F.METER); the frame (flushFrame) draws the meters with transforms only (no layout read); after the
   last message the meters fall on a 40 ms timer, which stops when every meter is at the floor. */
const VOL_TAPER = [[0, -120], [0.03, -60], [0.25, -24], [0.5, -9], [0.75, 0], [1, 12]];
const METER_FLOOR = -60, METER_FALL = 24, HOLD_MS = 1500, HOLD_FALL = 20, METER_STEP_MS = 40;
function taper(x, a, b) {   // piecewise linear over VOL_TAPER: column a in, column b out
  const T = VOL_TAPER;
  if (x <= T[0][a]) return T[0][b];
  for (let i = 1; i < T.length; i++) {
    if (x <= T[i][a]) { const p = T[i - 1], q = T[i]; return p[b] + (q[b] - p[b]) * (x - p[a]) / (q[a] - p[a]); }
  }
  return T[T.length - 1][b];
}
const volPos = db => taper(db, 1, 0), posVol = p => taper(p, 0, 1);
const clamp01 = v => Math.max(0, Math.min(1, v));
function fmtDb(db) { return db <= -119.95 ? "-inf" : (db > 0.04 ? "+" : "") + db.toFixed(1); }
function fmtPan(p) { const v = Math.round(p * 100); return v === 0 ? "C" : v < 0 ? `L${-v}` : `R${v}`; }   // P3-14c: C, L50, R50 (Pro Tools)
function mixTip(el, text) {
  let t = $("#mixTip");
  if (!t) { t = document.createElement("div"); t.id = "mixTip"; document.body.appendChild(t); }
  t.textContent = text; t.classList.remove("hidden");
  const r = el.getBoundingClientRect();
  t.style.left = Math.max(0, Math.min(r.left, innerWidth - t.offsetWidth - 4)) + "px";
  t.style.top = Math.max(0, r.top - t.offsetHeight - 4) + "px";
}
function hideMixTip() { const t = $("#mixTip"); if (t) t.classList.add("hidden"); }
/* One control: a horizontal slider (div). o = {get, set(v), reset, toPos, fromPos, fmt, label, commit}. */
function bindMixSlider(el, o) {
  let dragging = false;
  const show = () => {
    const v = o.get(), p = clamp01(o.toPos(v));
    el.querySelector(".mknob").style.left = `${p * 100}%`;
    const f = el.querySelector(".mfill"); if (f) f.style.width = `${p * 100}%`;
    el.dataset.value = o.fmt(v);
    el.title = `${o.label} ${o.fmt(v)}. Drag (Option: fine); double-click or Option-click: reset`;
    const out = el.parentElement && el.parentElement.querySelector(`[data-for="${el.dataset.ctl}"]`);
    if (out) out.textContent = o.fmt(v);
  };
  el._show = show;
  el.addEventListener("mousedown", ev => {
    if (ev.button !== 0) return;
    ev.preventDefault(); ev.stopPropagation();
    const w = el.clientWidth || 1;
    let x0 = ev.clientX, pos = clamp01(o.toPos(o.get())), moved = false;
    dragging = true;
    const mv = e2 => {
      const dx = e2.clientX - x0; x0 = e2.clientX;
      if (!dx) return;
      moved = true;
      pos = clamp01(pos + dx / w * (e2.altKey ? 0.1 : 1));
      o.set(o.fromPos(pos)); show(); mixTip(el, `${o.label} ${o.fmt(o.get())}`);
    };
    const up = e2 => {
      document.removeEventListener("mousemove", mv, true); document.removeEventListener("mouseup", up, true);
      dragging = false; hideMixTip();
      if (!moved && e2.altKey) { o.reset(); show(); }
    };
    document.addEventListener("mousemove", mv, true); document.addEventListener("mouseup", up, true);
    mixTip(el, `${o.label} ${o.fmt(o.get())}`);
  });
  el.addEventListener("click", ev => ev.stopPropagation());   // not a click on the header (track selection)
  el.addEventListener("dblclick", ev => { ev.preventDefault(); ev.stopPropagation(); o.reset(); show(); });
  el.addEventListener("mouseenter", () => mixTip(el, `${o.label} ${o.fmt(o.get())}`));
  el.addEventListener("mouseleave", () => { if (!dragging) hideMixTip(); });
  show();
}
/* Sends: one call in flight per target; a change during the call sends the latest value after it. */
const _mixQ = {};
function mixSend(key, call, onDone) {
  const q = _mixQ[key] || (_mixQ[key] = { busy: false, again: false });
  if (q.busy) { q.again = true; return; }
  q.busy = true; q.again = false;
  call().then(r => { if (onDone) onDone(r); })
    .catch(e => setMsg(`volume/pan failed: ${e.message.replace(/^\d+: /, "")}`, "error"))
    .finally(() => { q.busy = false; if (q.again) mixSend(key, call, onDone); });
}
function trackVol(name) { const t = S.tracks[name] || {}; return typeof t.volume_db === "number" ? t.volume_db : 0; }
function trackPan(name) { const t = S.tracks[name] || {}; return typeof t.pan === "number" ? t.pan : 0; }
function sendTrackMix(name) {
  const sid = S.sid;
  mixSend(`t:${sid}:${name}`, () => B.trackSetMix(sid, name, trackVol(name), trackPan(name)),
    () => { if (S.sid === sid) setDirtyState(true); });
}
function setTrackMix(name, volumeDb, pan) {   // the model, the controls, the service
  const t = S.tracks[name]; if (!t) return;
  if (volumeDb !== null) t.volume_db = Math.round(Math.max(-120, Math.min(12, volumeDb)) * 10) / 10;
  if (pan !== null) t.pan = Math.round(Math.max(-1, Math.min(1, pan)) * 100) / 100;
  const row = rowOf(name);
  if (row) for (const el of row.querySelectorAll(".mslider")) el._show();
  if (name === INS.name) for (const el of document.querySelectorAll("#inspector .mslider")) el._show();   // P3-14a
  sendTrackMix(name);
}
function bindTrackMix(row, name) {   // P3-14a: set goes through setTrackMix too, so the Inspector's controls follow a header drag
  bindMixSlider(row.querySelector(".mslider.vol"), { label: "Volume", get: () => trackVol(name),
    set: v => setTrackMix(name, v, null), reset: () => setTrackMix(name, 0, null),
    toPos: volPos, fromPos: posVol, fmt: fmtDb });
  bindMixSlider(row.querySelector(".mslider.pan"), { get label() { return (S.tracks[name] || {}).play_channels === 2 ? "Balance" : "Pan"; }, get: () => trackPan(name),
    set: v => setTrackMix(name, null, v), reset: () => setTrackMix(name, null, 0),
    toPos: p => (p + 1) / 2, fromPos: x => x * 2 - 1, fmt: fmtPan });
  const meter = row.querySelector(".meter");
  MT.tracks[name] = newMeter(meter);
  meter.querySelector(".mclip").addEventListener("click", ev => { ev.stopPropagation(); clearClip(MT.tracks[name]); });
}
/* A stereo file plays as stereo (P3-16): its meter has two bars (L, R) and its pan is a balance. A mono track's meter
   is one bar (its level after the volume, before the pan). The channels come with the play answer. */
function markTrackChannels(names, channels) {
  names.forEach((n, i) => {
    const row = rowOf(n), t = S.tracks[n]; if (!row || !t) return;
    t.play_channels = channels[i];
    row.querySelector(".meter").classList.toggle("mono", channels[i] !== 2);
    invalidate(F.METER);   // P4-40h: the canvas paints one bar for mono, two for stereo
    const pan = row.querySelector(".mslider.pan"); if (pan && pan._show) pan._show();
  });
}
function resetSelectedMix() {   // Track > Reset Volume and Pan
  const n = needTrack(); if (!n) return;
  setTrackMix(n, 0, 0); setMsg(`${n}: volume 0 dB, pan centre`);
}
function setMasterDb(db, send) {
  S.masterDb = Math.round(Math.max(-120, Math.min(12, db)) * 10) / 10;
  const el = $("#masterVol"); if (el && el._show) el._show();
  if (send) mixSend("master", () => B.transportMaster(S.masterDb));
}
function setupMaster() {
  S.masterDb = 0;
  bindMixSlider($("#masterVol"), { label: "Master", get: () => S.masterDb, set: v => setMasterDb(v, true),
    reset: () => setMasterDb(0, true), toPos: volPos, fromPos: posVol, fmt: fmtDb });
  MT.master = newMeter($("#masterMeter"), true);
  $("#masterMeter .mclip").addEventListener("click", () => clearClip(MT.master));
  B.transportStatus().then(st => { if (st && typeof st.master_db === "number") setMasterDb(st.master_db, false); }).catch(() => {});
}
/* meters. P4-40h: the bars and the hold lines are drawn on ONE canvas per meter (a style write of any meter
   element triggers the Blink style/prepaint lifecycle of the whole 30-track page in every frame, measured at
   19 % of one core at 25 msg/s; a canvas write skips that lifecycle, 4 % with all rows drawn). The model (lvl,
   hold, clip) and `drawn` (the last painted 1/200 fractions, the tests read them) are unchanged. The clip light
   stays a DOM element: the click on it must keep working, and it changes rarely. Only rows inside the scroll
   viewport are painted (their model still updates); a scroll or a geometry change repaints once. */
const MT = { tracks: {}, master: null, timer: null, at: 0 };
function meterBox(el, horizontal) { return el.querySelector(horizontal ? ".mhbars" : ".mbars"); }
function newMeter(el, horizontal) {
  const box = meterBox(el, horizontal);
  const c = document.createElement("canvas");
  box.textContent = "";
  box.appendChild(c);
  const m = { el, horizontal: !!horizontal, lvl: [-Infinity, -Infinity], hold: [-Infinity, -Infinity], holdAt: [0, 0], clip: false,
    light: el.querySelector(".mclip"), canvas: c, ctx: null, grad: null, w: 0, h: 0, dpr: 0, mono: null,
    drawn: [-1, -1, -1, -1], clipDrawn: false, t: 0 };
  m.ctx = c.getContext("2d");
  meterSize(m);
  drawMeter(m);   // the empty slots at once: the look before the first message is the old one
  new ResizeObserver(() => { meterSize(m); invalidate(F.METER); }).observe(box);
  return m;
}
function meterSize(m) {   // the backing store to the box size and the device pixel ratio; writes only
  const box = m.canvas.parentElement, dpr = window.devicePixelRatio || 1;
  if (!box.isConnected) return;   /* #110: the box left the DOM (the project closed). The final ResizeObserver
     delivery of a removed element lands after buildTrackDOM, and a write here would size the detached
     canvas again (1 x 1) and take its backing store back. A draw sizes canvases that are in the DOM only. */
  const w = Math.max(1, Math.round(box.clientWidth * dpr)), h = Math.max(1, Math.round(box.clientHeight * dpr));
  if (m.w === w && m.h === h && m.dpr === dpr) return;
  m.w = w; m.h = h; m.dpr = dpr;
  m.canvas.width = w; m.canvas.height = h;
  m.grad = meterGrad(m);
  m.drawn = [-1, -1, -1, -1];
}
function meterGrad(m) {   // the level gradient of the CSS look, as an offscreen strip at the bar size
  const off = document.createElement("canvas"), ctx = off.getContext("2d");
  const w = m.w, h = m.horizontal ? Math.max(1, Math.floor(m.h / 2)) : m.h;
  off.width = Math.max(1, w); off.height = Math.max(1, h);
  const g = m.horizontal ? ctx.createLinearGradient(0, 0, w, 0) : ctx.createLinearGradient(0, h, 0, 0);
  g.addColorStop(0, "#2fbf5b"); g.addColorStop(0.7, "#2fbf5b"); g.addColorStop(0.88, "#e8c547"); g.addColorStop(1, "#ff4b4b");
  ctx.fillStyle = g; ctx.fillRect(0, 0, off.width, off.height);
  return off;
}
function clearClip(m) { if (!m) return; m.clip = false; invalidate(F.METER); }
function meterIn(m, l, r, now) {   // linear peaks of one message (or 0 for the fall)
  const dt = m.t ? Math.min(0.5, (now - m.t) / 1000) : 0; m.t = now;
  [l, r].forEach((v, c) => {
    v = +v || 0;
    if (v >= 1) m.clip = true;
    const db = v > 0 ? 20 * Math.log10(v) : -Infinity;
    m.lvl[c] = Math.max(db, m.lvl[c] - METER_FALL * dt);
    if (db >= m.hold[c]) { m.hold[c] = db; m.holdAt[c] = now; }
    else if (now - m.holdAt[c] > HOLD_MS) m.hold[c] = Math.max(m.lvl[c], m.hold[c] - HOLD_FALL * dt);
  });
}
const meterFrac = db => clamp01((db - METER_FLOOR) / -METER_FLOOR);
function drawMeter(m) {   // in the frame (F.METER): paints the one canvas; no layout read
  if (window.devicePixelRatio !== m.dpr) meterSize(m);   // a display move must not leave a stale backing store
  const f0 = Math.round(meterFrac(m.lvl[0]) * 200) / 200, f1 = Math.round(meterFrac(m.lvl[1]) * 200) / 200;
  const h0 = Math.round(meterFrac(m.hold[0]) * 200) / 200, h1 = Math.round(meterFrac(m.hold[1]) * 200) / 200;
  const mono = !m.horizontal && m.el.classList.contains("mono");
  if (f0 === m.drawn[0] && f1 === m.drawn[1] && h0 === m.drawn[2] && h1 === m.drawn[3] && mono === m.mono && m.clip === m.clipDrawn) return;
  if (!m.w || !m.h) { m.mono = mono; return; }   // sized by the observer; nothing to paint into yet
  m.mono = mono;
  m.drawn[0] = f0; m.drawn[1] = f1; m.drawn[2] = h0; m.drawn[3] = h1;
  if (m.clip !== m.clipDrawn) { m.clipDrawn = m.clip; m.light.classList.toggle("on", m.clip); }
  const ctx = m.ctx, W = m.w, H = m.h, grad = m.grad;
  ctx.clearRect(0, 0, W, H);
  if (m.horizontal) {   // the master: two bars of H/2 (the CSS gap of 2 px between), the gradient along the width
    const bh = Math.floor(H / 2), gap = H - 2 * bh, lw = Math.max(1, m.dpr);
    for (let c = 0; c < 2; c++) {
      const y = c * (bh + gap), f = c ? f1 : f0, h = c ? h1 : h0;
      ctx.fillStyle = "#0d0f13"; ctx.fillRect(0, y, W, bh);
      const fw = Math.round(f * W);
      if (fw > 0) ctx.drawImage(grad, 0, 0, W, bh, 0, y, fw, bh);   // the full gradient compressed: the old scaleX look
      if (h > 0) { ctx.fillStyle = "#ffe08a"; ctx.fillRect(Math.max(0, Math.round(h * W) - lw), y, lw, bh); }
    }
    return;
  }
  const gap = Math.round(m.dpr), bw = mono ? W : Math.floor((W - gap) / 2);   // the bars, 1 CSS px apart
  for (let c = 0; c < 2; c++) {
    if (mono && c === 1) break;
    const x = mono ? 0 : c * (W - bw), f = c ? f1 : f0, h = c ? h1 : h0;
    ctx.fillStyle = "#0d0f13"; ctx.fillRect(x, 0, bw, H);
    const fh = Math.round(f * H);
    if (fh > 0) ctx.drawImage(grad, 0, 0, bw, H, x, H - fh, bw, fh);   // the full gradient compressed: the old scaleY look
    if (h > 0) { ctx.fillStyle = "#ffe08a"; ctx.fillRect(x, Math.max(0, H - Math.round(h * H)), bw, Math.max(1, m.dpr)); }
  }
}
function allMeters() { const v = Object.values(MT.tracks); if (MT.master) v.push(MT.master); return v; }
function drawMeters() {   // in the frame (F.METER): only the rows inside the scroll viewport (GEO, cached)
  const top = GEO.top, bot = GEO.top + GEO.scH;
  for (const name of S.order) {
    const m = MT.tracks[name];
    if (!m) continue;
    const g = GEO.ok && GEO.rows[name];
    if (g && (g.bottom <= top || g.top >= bot)) continue;
    drawMeter(m);
  }
  if (MT.master) drawMeter(MT.master);
}
function onMeters(m) {
  AC.meters = m; AC.meterN = (AC.meterN || 0) + 1;
  const now = performance.now(), names = AC.names || S.order;
  for (let i = 0; i < names.length; i++) { const t = MT.tracks[names[i]]; if (t) meterIn(t, m.meters[2 * i], m.meters[2 * i + 1], now); }
  if (MT.master && m.master) meterIn(MT.master, m.master[0], m.master[1], now);
  MT.at = now;
  invalidate(F.METER);
  meterFallSoon();
}
/* When no message came for 3 steps (pause, end, an error), the meters fall to the floor on a 40 ms timer. */
function meterFallSoon() {
  if (MT.timer) return;
  MT.timer = setTimeout(function step() {
    MT.timer = null;
    const now = performance.now();
    if (now - MT.at < 3 * METER_STEP_MS) { meterFallSoon(); return; }
    let up = false;
    for (const x of allMeters()) {
      meterIn(x, 0, 0, now);
      if ([...x.lvl, ...x.hold].some(db => db > METER_FLOOR)) up = true;
    }
    invalidate(F.METER);
    if (up) MT.timer = setTimeout(step, METER_STEP_MS);
  }, METER_STEP_MS);
}
let _clicksTimer = null;
function appClicksSoon() {   // settings sliders fire often: one call per 100 ms
  clearTimeout(_clicksTimer);
  _clicksTimer = setTimeout(() => { const c = SETTINGS().click; appCmd(() => B.transportClicks(c.enabled, c.volume, c.pitches)).catch(e => setMsg(`clicks failed: ${e.message}`, "error")); }, 100);
}
