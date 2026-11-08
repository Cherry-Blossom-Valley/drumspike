/* Drum Spike v2 — DAW-style navigation
   Concepts: playhead-anchored zoom, Tab event navigation, event lines through
   both lanes (spike + waveform), follow-playhead, persistent position readout. */
"use strict";

const OWN_ZOOM_NAME_MAX = 40;   // #169: the longest name of an own zoom preset
const ROLE_DEFAULT_NOTE = { kick:36, snare:38, tom:48, hat:42, cymbal:49, other:36 };
const ROLE_DEFAULT_CLICK = { kick:1000, snare:2500, tom:700, hat:4000, cymbal:3000, other:1500 };

/* ---------------- backend ---------------- */
/* Every call to the server goes through B (static/backend.js): named operations, no paths here. */
function normEvents(d){
  const evs = Array.isArray(d) ? d : (d && d.events) || [];
  return evs.map((e,i) => { const o = {t: e.t, v: e.v, id: "l"+(i+1)};   // stable identity (B1)
    if (e.added) o.added = true; if (e.orig !== undefined) o.orig = e.orig;   // manual-hit markers
    if (e.rise !== undefined) o.rise = e.rise;   // transient rise dB (R1 confidence) — carried, not used
    if (e.cut) o.cut = true;   // P4-3: the loudness cut took it out of the output
    if (e.bleed) o.bleed = true;   // P4-14: the tom bleed rule took it out of the output (shown as a cut hit for now)
    if (e.roll) o.roll = true;   // P4-19: the roll fill added it (a normal hit; kept so a page edit does not drop the tag)
    if (e.flam) o.flam = true;   // P4-13 stage B: the flam rule added it (the second stroke of a flam; a normal hit)
    if (e.rescue) o.rescue = true;   // P4-10 stage 5: the kick rescue added it (a quiet kick the model missed; a normal hit)
    if (e.learned) o.learned = true;   // P4-38b: a K added it (kept so undo and redo snapshots keep the tag)
    if (e.filtered) o.filtered = true;   // P4-6c: the Learn filter took it out of the output (a grey hit)
    if (e.edited) o.edited = true;   // P4-14 fix round: restored from the bleed mark, kept by a re-pick or Re-analyze
    if (e.keep) o.keep = true;   // #242: the user keeps the hit in the output (Toggle Hit Active)
    if (e.deactivated) o.deactivated = true;   // #242: the user took the hit out of the output (Toggle Hit Active)
    return o; }).sort((a, b) => a.t - b.t);   // P3-14d: hitsIn searches them by time (the service sends them sorted)
}
let _eid = 0;
function newId(){ return "n" + (++_eid); }

/* manual/edited hits: added by hand, or moved away from their detected position */
const MANUAL_COLOR = "#ff4fd8";
function isManual(e) { return !!e.added || (e.orig !== undefined && Math.abs(e.t - e.orig) > 1e-9); }
/* a kept hit: manual, a hand velocity or a restored hit (`edited: true`), or one of the user's
   per-hit overrides of #242 (`keep`/`deactivated`) — the page's copy of edit::is_kept
   (crates/app/src/edit.rs). One predicate decides the Keep/Discard dialog of a re-analysis and
   the service's merge (keep_edited), so both see the same edits. */
function isKept(e) { return isManual(e) || e.edited === true || e.keep === true || e.deactivated === true; }

/* ---------------- playhead overlay (global, Pro Tools style) ---------------- */
function positionPlayhead() {   // the overlay lives in #editor; it shows only inside the lane area (never over the header column or the Inspector)
  const W = laneWidth();
  const x = Math.round((S.theadW + tToX(S.playhead, W)) * 64) / 64;   // P4-1: float noise writes no new style
  const el = $("#playheadOverlay");
  const display = x >= S.theadW - 0.5 && x <= S.theadW + W + 0.5 ? "block" : "none";
  if (FR.head[0] !== display) el.style.display = FR.head[0] = display;
  if (CP.anim) return;   // P4-1: the compositor moves it
  const tf = `translateX(${x}px)`;
  if (FR.head[1] !== tf) el.style.transform = FR.head[1] = tf;
}
function drawHead() {   // the playhead and the time display; a text is written only when it changed
  positionPlayhead();
  const [main, sub] = counterTexts();
  if (FR.head[2] !== main) $("#timeDisplay").textContent = FR.head[2] = main;
  if (FR.head[3] !== sub) $("#sampleDisplay").textContent = FR.head[3] = sub;
}

/* P3-14b: counters. Main counter: seconds with 6 decimals, samples, Bars|Beats (bar.beat of the
   tempo map, #149) or Timecode (HH:MM:SS:FF at 30 frames per second from the sample position,
   #149); the sub counter shows the other format of the pair (seconds<->samples,
   Bars|Beats<->Timecode). The sample rate is the one of the selected track, else of the first
   track. The Timecode frame rate is 30, non drop: frames = floor(t * 30) over the whole project. */
const COUNTER_PAIRS = { seconds: "samples", samples: "seconds", barsbeats: "timecode", timecode: "barsbeats" };
function counterRate() {
  const t = (S.selTrack && S.tracks[S.selTrack]) || (S.order.length && S.tracks[S.order[0]]);
  return t && t.sr ? t.sr : 48000;
}
function counterBBText(bar, beat, ticks) {   // #149: the Pro Tools Bars|Beats shape, 960 ticks per beat
  return `${bar}| ${String(beat).padStart(2, " ")}| ${String(ticks).padStart(4, " ")}`;
}
function counterBarsBeats() {
  const m = TM.map;
  if (!m) {   // no map: the default one the export uses, 120 BPM 4/4, bar 1 at 0 s
    // quarter-note beats at 120 BPM; the same microsecond snap to the next beat as below, and the
    // ticks are floored (#208: rounding showed 960 -> 0 of the old beat, a beat too early)
    const total = S.playhead * 2 + 2e-6;
    return counterBBText(Math.floor(total / 4) + 1, Math.floor(total % 4) + 1, Math.min(959, Math.floor((total % 1) * 960 + 1e-6)));
  }
  const g = tmGrid();
  if (!g || !g.t.length) return counterBBText(1, 1, 0);
  let i = tmLower(g.t, S.playhead) - 1;   // the beat at or before the playhead (#149)
  if (i < 0) return counterBBText(1, 1, 0);   // before bar 1: clamped to 1| 1| 000
  // the grid times accumulate (tmGrid), so a playhead on a beat can sit a hair below its
  // grid time: treat anything within a microsecond of the next beat as that beat (#149)
  if (i + 1 < g.t.length && g.t[i + 1] - S.playhead * 1 <= 1e-6) i++;
  const dt = (i + 1 < g.t.length ? g.t[i + 1] - g.t[i] : 0.5) || 0.5;
  const ticks = Math.floor((S.playhead - g.t[i]) / dt * 960 + 1e-6);   // floored, never a beat too early (#208)
  return counterBBText(g.bar[i], g.beat[i], Math.max(0, Math.min(959, ticks)));
}
function counterTimecode() {
  const fps = 30, sr = counterRate();
  const t = Math.round(S.playhead * sr) / sr;   // the sample position of the playhead (#149)
  let frames = Math.floor(t * fps);
  const ff = frames % fps; frames = Math.floor(frames / fps);
  const ss = frames % 60; frames = Math.floor(frames / 60);
  const mm = frames % 60, hh = Math.floor(frames / 60);
  const two = n => String(n).padStart(2, "0");
  return `${two(hh)}:${two(mm)}:${two(ss)}:${two(ff)} tc`;
}
function counterTexts() {
  const secs = `${fmt(S.playhead, 6)} s`, smp = `${Math.round(S.playhead * counterRate())} smp`;
  const bb = counterBarsBeats(), tc = counterTimecode();
  const texts = { seconds: secs, samples: smp, barsbeats: bb, timecode: tc };
  const main = texts[SETTINGS().ui.counter] || secs;
  return [main, texts[COUNTER_PAIRS[SETTINGS().ui.counter] || "samples"]];
}
function setCounter(mode) {   // View > Main Counter: "seconds", "samples", "barsbeats" or "timecode" (#149)
  if (!(mode in COUNTER_PAIRS)) return;
  SETTINGS().ui.counter = mode; saveSettings();
  FR.head[2] = FR.head[3] = null; invalidate(F.HEAD);
  $("#timeDisplay").title = `Main counter (${mode}${mode === "timecode" ? ", 30 fps" : ""}). Click: exchange the counters (View > Main Counter > Exchange Counters)`;
  DSShell.sync();
}
function exchangeCounters() { setCounter(COUNTER_PAIRS[SETTINGS().ui.counter] || "seconds"); }

/* ---------------- canvas sizing ---------------- */
function laneWidth() { return GEO.laneW; }
/* P3-6b: the canvases of a lane keep a backing store only while the lane is in view or half a screen from it: each
   2D canvas of the 30-lane project cost 0.8 to 2.6 MB of graphics memory in the WebContent process (about 100 MB
   together). Other lanes hold a 1 x 1 canvas with the same CSS size and are drawn again when they come near.
   P3-14d: "near" comes from GEO (no layout read); a row that the observer has not measured yet is not near. */
function lanesNear() {
  const m = GEO.scH / 2, near = new Set();
  document.querySelectorAll("#lanesScroll .track").forEach(row => {
    const g = GEO.rows[row.dataset.name];
    if (g && g.bottom - GEO.top >= -m && g.top - GEO.top <= GEO.scH + m) near.add(row);
  });
  return near;
}
function sizeLaneCanvas(c, live) {
  const dpr = GEO.dpr, W = laneWidth(), hh = laneHeights(), row = c.closest(".track"), own = row && S.rowH && S.rowH[row.dataset.name];
  const vel = c.classList.contains("vel");   // #208: the velocity lane has its own height (VL.h), not the wave height
  const h = c.classList.contains("spike") ? hh.spike : vel ? VL.h : (own ? Math.round(hh.wave * own) : hh.wave);   // P3-14c; P4-27/#167: a row with its own factor
  c.style.width = W + "px";   // the CSS height comes from the variables of setTrackHeight (an inline height would win over them)
  c.style.transform = ""; c._sptf = "";   // #463: a lane sized for a draw is not on a strip (the rAF path's cache resets too)
  if (vel) c.style.height = h + "px";
  if (live) { c.width = W * dpr; c.height = h * dpr; c.getContext("2d").setTransform(dpr, 0, 0, dpr, 0, 0); }
  else { c.width = 1; c.height = 1; }
}
function syncLaneBuffers() {   // in the frame: a lane came near the view, or left it; true when one came near
  const near = lanesNear();
  let woke = false;
  document.querySelectorAll("#lanesScroll .track").forEach(row => {
    const live = near.has(row);
    if (live === (row._live !== false)) return;
    row._live = live; woke = woke || live;
    row.querySelectorAll("canvas.lane").forEach(c => sizeLaneCanvas(c, live));
  });
  return woke;
}
function sizeCanvases() {   // in the frame: every canvas to the sizes of GEO (writes only)
  const dpr = GEO.dpr, near = lanesNear();
  document.querySelectorAll("#lanesScroll .track").forEach(row => {
    row._live = near.has(row);
    row.querySelectorAll("canvas.lane").forEach(c => sizeLaneCanvas(c, row._live));
  });
  const ov = $("#overview"), rul = $("#ruler"), rw = GEO.rulerW;
  ov.width = GEO.ovW * dpr; ov.height = GEO.ovH * dpr;
  ov.getContext("2d").setTransform(dpr,0,0,dpr,0,0);
  rul.style.width = rw + "px"; rul.style.height = "22px";
  rul.style.transform = ""; rul._sptf = "";   // #150: the time ruler drops its strip transform with its size, like the lane canvases (sizeLaneCanvas)
  rul.width = rw * dpr; rul.height = 22 * dpr; rul.getContext("2d").setTransform(dpr,0,0,dpr,0,0);
  sizeMapCanvases();   // P4-8
  FR.sized = sizeKey();
}
function resizeCanvases() { invalidate(F.SIZE); }   // the next frame sizes and draws every canvas

/* ---------------- coordinates ---------------- */
/* P4-1b fix: the span to the nanosecond. A view is stored as {t0, t1}, and t1 - t0 of a view set as {t0, t0 + d} is d plus
   or minus an ulp for many t0; the follow step carried that noise on (8 became 7.999999999999999). Two draws of the same
   span then put a peak bin at a column edge on different sides (the strip against a direct draw). */
const viewDur = () => Math.round((S.view.t1 - S.view.t0) * 1e9) / 1e9;
const spanOf = v => Math.round((v.t1 - v.t0) * 1e9) / 1e9;   // the same for any view
function tToX(t, w) { return (t - S.view.t0) / viewDur() * w; }
function xToT(x, w) { return S.view.t0 + x / w * viewDur(); }
function maxDur() { return Math.max(10, ...S.order.map(n => S.tracks[n].duration)); }
function snapSample(t, name) { const sr = S.tracks[name].sr; return Math.round(t*sr)/sr; }
/* #353: the view of a hit edit in Training Mode: the visible span of the waveform in milliseconds and the pixels
   one sample occupies (the waveform canvas width over the samples the span holds on the track). */
function trainingView(name) {
  const sr = S.tracks[name] && S.tracks[name].sr, d = viewDur();
  if (!sr || !(d > 0)) return null;
  /* 1e-6 px: viewDur() rounds the span to whole ns, which leaves the exact one-pixel boundary a
     hair under 1 (0.999999978 ...); precision_of cuts at >= 1, so the rounding keeps that boundary
     on the sample side (#353). */
  return { ms: d * 1000, px_per_sample: Math.round(laneWidth() / (d * sr) * 1e6) / 1e6 };
}
/* playhead snapping: within 10px on screen AND <= 30 ms of the nearest hit -> land exactly on it */
function snapPlayhead(t) {
  if (!S.snap) return t;
  const pxTime = viewDur() / laneWidth();          // seconds per screen pixel
  const thresh = Math.min(10 * pxTime, 0.030);
  let best = null, bestDt = thresh;
  for (const n of S.order) {   // W6: only the hits within the threshold (binary search)
    const evs = S.events[n] || [];
    for (const i of hitsIn(evs, t - thresh, t + thresh, dragSkip(n))) {
      const dt = Math.abs(evs[i].t - t);
      if (dt < bestDt) { bestDt = dt; best = evs[i].t; }
    }
  }
  return best !== null ? best : t;
}
/* per-track display gain: normalize so the track's own peak fills ~90% of half-height */
function peakAmpOf(p) {
  if (!p || !p.levels) return 1;
  const L = p.levels[65536] || p.levels[4096] || p.levels[256];
  if (!L || !L.max || !L.max.length) return 1;
  const mx = Math.max(...L.max), mn = Math.min(...L.min);   // E: negative peaks count too
  return Math.max(1e-5, mx, -mn);
}

/* ---------------- view navigation ---------------- */
function viewClamp(t0, t1) {
  const maxD = maxDur();
  const d = Math.min(maxD, Math.max(t1 - t0, 0.002));   // keep the requested span (zoom presets, pans
  t0 = Math.max(0, Math.min(t0, maxD - d));             // and follow near 0 used to shrink it) and
  return { t0, t1: t0 + d };                            // slide it inside [0, maxD]
}
function setView(t0, t1) {
  S.view = viewClamp(t0, t1); invalidate(F.VIEW); updateStatus();
}
function centerOn(t) {
  const d = viewDur();
  setView(t - d/2, t + d/2);
}
function zoomAround(anchorT, factor) {
  S.followHold = true;   // user interaction pauses follow until next play
  const d = viewDur();
  const nd = Math.min(maxDur(), Math.max(0.002, d*factor));
  setView(anchorT - nd/2, anchorT + nd/2);
}
function zoomAtX(x, w, factor) {   // P4 #145: Cmd+wheel and pinch; the time under lane x stays under x
  S.followHold = true;
  const a = xToT(x, w), f = x / w;
  const nd = Math.min(maxDur(), Math.max(0.002, viewDur()*factor));
  setView(a - f*nd, a - f*nd + nd);
}
function zoomPlayhead(factor) {
  if (S.playhead < S.view.t0 || S.playhead > S.view.t1) centerOn(S.playhead);
  zoomAround(S.playhead, factor);
}
function panView(dt) {
  S.followHold = true;   // user interaction pauses follow until next play
  const maxD = maxDur();
  const t0 = Math.max(0, Math.min(S.view.t0 + dt, maxD - viewDur()));
  setView(t0, t0 + viewDur());
}

/* ---------------- drawing ---------------- */
function redrawAll() {   // everything, now (tests and the self-test; the app invalidates and draws in the frame)
  FR.flags &= ~(F.VIEW | F.HEAD | F.PH | F.GL); FR.tracks.clear(); FR.sel = selKey();
  drawAll();
}
function drawAll() {
  drawRuler(); drawOverview();
  for (const name of S.order) {
    const row = rowOf(name);
    if (!row) continue;
    if (row._live === false && !GLQ.all) continue;   // far from the view: no canvas store (the GL lane is culled by its position)
    drawSpike(row.querySelector(".spike"), name);
    drawWave(row.querySelector(".wave"), name);
    if (VL.on) drawVelLane(row.querySelector(".vel"), name);   // P4-40f: the vel lane draws at open too, not only after edits
  }
  glFlush();   // P3-6a: the wave lanes queued by drawWave
  drawHead();
}
/* P3-14d: the hit lines of one track after an edit. With the GL engine the wave lane's 2D canvas holds only the hit
   lines, so the GL lane is not drawn again; the Canvas 2D path draws its wave lane again. False: no canvas store. */
function drawHits(name) {
  const row = rowOf(name);
  if (!row || row._live === false || !S.tracks[name]) return false;   // far from the view: drawn when it comes near
  const wave = row.querySelector(".wave");
  drawSpike(row.querySelector(".spike"), name);
  if (VL.on) drawVelLane(row.querySelector(".vel"), name);   // P4-40f: the velocity lane draws with the hits
  if (glEngine() && S.peaks[name]) {
    const [W, H] = laneSize(wave);
    wave.getContext("2d").clearRect(0, 0, W, H);
    drawEventLines(wave, name);
  } else drawWave(wave, name);
  return true;
}

function drawRuler() {
  const c = $("#ruler"), ctx = c.getContext("2d"), W = GEO.rulerW, H = 22;
  ctx.clearRect(0,0,W,H);
  ctx.fillStyle = "#1c1f26"; ctx.fillRect(0,0,W,H);
  const dur = viewDur();
  const steps = [0.001,0.002,0.005,0.01,0.02,0.05,0.1,0.2,0.5,1,2,5,10,30,60];
  const step = steps.find(s => s*W/dur > 60) || 60;
  // Pro Tools style: the major tick runs the full height, its label sits right of it with room above and beside
  ctx.strokeStyle = "#3a404c"; ctx.fillStyle = "#8a93a3"; ctx.font = "10px -apple-system, sans-serif"; ctx.textBaseline = "top";
  for (let t = Math.ceil(S.view.t0/step)*step; t <= S.view.t1; t += step) {
    const x = Math.round(tToX(t, W)) + 0.5;
    ctx.beginPath(); ctx.moveTo(x, 3); ctx.lineTo(x, H); ctx.stroke();
    ctx.fillText(step < 1 ? fmt(t,3) : fmt(t,1), x+5, 5);
    for (let k=1;k<5;k++){ const xx = Math.round(tToX(t + step*k/5, W)) + 0.5; ctx.beginPath(); ctx.moveTo(xx,H-5); ctx.lineTo(xx,H); ctx.stroke(); }
  }
  if (AU.win) {   // P4-40d: the loop of the audition
    const x0 = Math.max(0, tToX(AU.win.t0, W)), x1 = Math.min(W, tToX(AU.win.t1, W));
    if (x1 > x0) { ctx.fillStyle = "rgba(255, 184, 77, 0.45)"; ctx.fillRect(x0, 0, x1 - x0, H); ctx.fillStyle = "#ffb84d"; ctx.fillRect(x0, H - 3, x1 - x0, 3); }
  }
  if (LN.sel) {   // P4-6a: two small edge marks of the range selection
    ctx.fillStyle = "#4da3ff";
    for (const t of [LN.sel.t0, LN.sel.t1]) { const x = Math.round(tToX(t, W)); if (x >= -4 && x <= W + 4) { ctx.fillRect(x - 1, 0, 2, H); ctx.beginPath(); ctx.moveTo(x - 4, 0); ctx.lineTo(x + 4, 0); ctx.lineTo(x, 5); ctx.closePath(); ctx.fill(); } }
  }
  drawMapRulers();   // P4-8: the tempo, meter and Bars|Beats rulers share the x scale (and the strip)
}

/* P4-40b: the lane cursor says what a click does. #lanesScroll carries one class: curAdd (Option held: a click adds a hit)
   or curDelete (Option+Shift held over a hit that can be deleted: a click deletes it). The classes follow the keys
   (keydown and keyup of Option and Shift, also the modifiers of every mouse move) and the pointer; the CSS draws the cursors. */
const CUR = { alt: false, shift: false, del: false };
function curApply() {
  const sc = document.getElementById("lanesScroll"); if (!sc) return;
  sc.classList.toggle("curAdd", CUR.alt && !CUR.shift);
  sc.classList.toggle("curDelete", CUR.alt && CUR.shift && CUR.del);
}
function curMods(ev) { CUR.alt = !!ev.altKey; CUR.shift = !!ev.shiftKey; curApply(); }
function curOver(del, ev) { CUR.del = del; if (ev) { CUR.alt = !!ev.altKey; CUR.shift = !!ev.shiftKey; } curApply(); }   // del: the pointer is over a hit that Delete can remove
for (const t of ["keydown", "keyup", "mousemove"]) window.addEventListener(t, ev => { if (ev.target !== window) curMods(ev); }, { capture: true, passive: true });
window.addEventListener("blur", () => { CUR.alt = CUR.shift = false; curApply(); });

function drawOverviewTracks(ctx, W, H) {
  ctx.fillStyle = "#161920"; ctx.fillRect(0,0,W,H);
  const rowH = H / Math.max(1, S.order.length);
  S.order.forEach((name, idx) => {
    const p = S.peaks[name]; if (!p) return;
    const L = p.levels[65536] || p.levels[4096] || p.levels[256]; if (!L) return;
    const y0 = idx*rowH;
    ctx.fillStyle = "rgba(255,255,255,0.04)"; ctx.fillRect(0, y0, W, rowH-1);
    const amp = S.peakAmp[name] || 1;
    const scale = ((rowH-4)/2) / amp;
    if (Array.isArray(L.rms)) {
      ctx.fillStyle = S.tracks[name].color;
      for (let px=0; px<W; px++) {
        const b0 = Math.floor(px/W*L.n), b1 = Math.max(b0+1, Math.floor((px+1)/W*L.n));
        let r = 0; for (let b=b0;b<Math.min(b1,L.n);b++) r = Math.max(r, L.rms[b]);
        const hh = Math.min((rowH-4)/2, r*scale);
        if (hh>0.3) ctx.fillRect(px, y0+rowH/2-hh, 1, hh*2);
      }
    }
    ctx.fillStyle = S.tracks[name].color + "66";
    for (let px=0; px<W; px++) {
      const b0 = Math.floor(px/W*L.n), b1 = Math.max(b0+1, Math.floor((px+1)/W*L.n));
      let mn=1, mx=-1;
      for (let b=b0;b<Math.min(b1,L.n);b++) { mn=Math.min(mn,L.min[b]); mx=Math.max(mx,L.max[b]); }
      const yy0 = y0+rowH/2 - mx*scale, yy1 = y0+rowH/2 - mn*scale;
      ctx.fillRect(px, yy0, 1, Math.max(1, yy1-yy0));
    }
  });
}

/* P3-6a: the tracks of the overview do not depend on the view or the playhead: they are drawn once into
   an offscreen canvas (OV) and blitted; only the view box and the playhead are drawn per redraw. The
   uncached drawing cost 7.9 ms per redraw with 30 tracks in WKWebView. */
const OV = { key: "", peaks: [], amps: [], canvas: null };
function overviewTracks(c, W, H) {
  const peaks = S.order.map(n => S.peaks[n]), amps = S.order.map(n => S.peakAmp[n]);
  const key = `${c.width}x${c.height}|${W}x${H}|${S.order.map(n => n + S.tracks[n].color).join("|")}`;
  if (OV.canvas && key === OV.key && peaks.every((p, i) => p === OV.peaks[i]) && amps.every((a, i) => a === OV.amps[i])) return OV.canvas;
  OV.key = key; OV.peaks = peaks; OV.amps = amps;
  if (OV.canvas) { OV.canvas.width = 0; OV.canvas.height = 0; }
  OV.canvas = document.createElement("canvas"); OV.canvas.width = c.width; OV.canvas.height = c.height;
  const ctx = OV.canvas.getContext("2d");
  ctx.setTransform(c.width / W, 0, 0, c.height / H, 0, 0);
  drawOverviewTracks(ctx, W, H);
  return OV.canvas;
}
function drawOverview() {
  const c = $("#overview"), ctx = c.getContext("2d"), W = GEO.ovW, H = GEO.ovH;
  ctx.clearRect(0,0,W,H); ctx.fillStyle = "#161920"; ctx.fillRect(0,0,W,H);
  const maxD = maxDur();
  if (c.width && c.height) { ctx.save(); ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.drawImage(overviewTracks(c, W, H), 0, 0); ctx.restore(); }
  const x0 = S.view.t0/maxD*W, x1 = S.view.t1/maxD*W;
  ctx.fillStyle = "rgba(77,163,255,0.10)"; ctx.fillRect(x0, 0, x1-x0, H);
  ctx.strokeStyle = "#4da3ff"; ctx.strokeRect(x0+0.5, 0.5, x1-x0-1, H-1);
  ctx.fillStyle = "#4da3ff"; ctx.fillRect(x0-1, 0, 3, H); ctx.fillRect(x1-2, 0, 3, H);
  const px = S.playhead/maxD*W;
  ctx.strokeStyle = "#ff9f43"; ctx.lineWidth = 1.5;
  ctx.beginPath(); ctx.moveTo(px,0); ctx.lineTo(px,H); ctx.stroke();
  ctx.lineWidth = 1;
  ctx.fillStyle = "#ff9f43";
  ctx.beginPath(); ctx.moveTo(px-5,0); ctx.lineTo(px+5,0); ctx.lineTo(px,7); ctx.closePath(); ctx.fill();
}

/* APP_V stays in app.js (refactor step 5.2): it reads the ?v= of this script's own tag at load; wave.js
   loads before app.js, where the tag is not in the DOM yet. glEngine (wave.js) reads it at run time. */
const APP_V = (Array.from(document.scripts).map(x => /app\.js\?v=(\w+)/.exec(x.src)).find(Boolean) || [0, "0"])[1];

function openUndoHistory() {
  if (!needProject()) return;
  const box = $("#undoHistList"); box.textContent = "";
  const rows = undoHistoryRows();
  $("#undoHistEmpty").classList.toggle("hidden", rows.length > 0);
  for (const r of rows) {
    const b = document.createElement("button");
    b.className = "uhRow" + (r.redo ? " uhRedo" : "");
    b.textContent = stepText(r.op, !r.redo);
    b.title = r.redo ? `Redo ${r.count} step${r.count === 1 ? "" : "s"}` : `Undo ${r.count} step${r.count === 1 ? "" : "s"}`;
    b.dataset.count = r.count; b.dataset.dir = r.redo ? "redo" : "undo";
    b.onclick = async () => { $("#undoHistModal").classList.add("hidden"); await undoSteps(!r.redo, r.count); };
    box.appendChild(b);
  }
  showModal("#undoHistModal");
}
function needProject() {   // S3-3: the page can show no project at all
  if (S.sid) return true;
  setMsg("no project is open: use New, Open or Open Recent"); return false;
}

/* ---------------- playback: the native transport (P3-5b) ---------------- */
function applyMixer() { appMix(); }   // live mute/solo during playback
function play(returnKey) { return appPlay(undefined, returnKey); }   // #207: returnKey is the play-and-return key that starts the play
function showSpeed(speed) {   // the status bar says "half speed" while the transport plays at half speed
  const el = $("#stSpeed"), half = speed === 0.5;
  if (el.classList.contains("hidden") === half) el.classList.toggle("hidden", !half);
}
function pause() { return appPause(); }

/* ---------------- follow modes (Pro Tools/Logic style) ----------------
   smooth: timeline glides continuously, playhead pinned at FOLLOW_FRAC of the view
   page:   flip one page when the playhead crosses the right edge
   off:    playhead may leave the view                                             */
const FOLLOW_FRAC = 0.25;
function followStep() {
  if (S.followMode === "off" || S.followHold) return;
  const d = viewDur();
  if (followKind() === "page") {   // P4-39b: also the smooth follow where no glide can show; it flips to FOLLOW_FRAC, the page mode centres
    if (S.playhead > S.view.t1 - d*0.1 || S.playhead < S.view.t0) {
      if (S.followMode !== "smooth") { centerOn(S.playhead); return; }
      const v = viewClamp(S.playhead - FOLLOW_FRAC * d, S.playhead - FOLLOW_FRAC * d + d);
      if (Math.abs(v.t0 - S.view.t0) > 1e-12 || Math.abs(v.t1 - S.view.t1) > 1e-9) setView(v.t0, v.t1);
    }
    return;
  }
  // smooth
  if (S.playhead < S.view.t0 || S.playhead > S.view.t1) { centerOn(S.playhead); return; }  // re-enter after Tab/jump
  const anchorX = FOLLOW_FRAC * d;
  const curX = S.playhead - S.view.t0;
  // P3-14d (W4): the view moves in every frame in which the playhead moved half a pixel or more (before: about 33 Hz
  // and 2 px steps, so the pinned playhead juddered against the 60 Hz frames)
  // P4-1: only when the clamped view changes. Left of the anchor at the start of the song (and at its end) the view
  // cannot move; before, every frame drew all lanes again for nothing (the whole `play` scenario has a 300 s view).
  if (Math.abs(curX - anchorX) / d * laneWidth() < 0.5) return;
  const v = viewClamp(S.playhead - anchorX, S.playhead - anchorX + d);
  if (Math.abs(v.t0 - S.view.t0) > 1e-12 || Math.abs(v.t1 - S.view.t1) > 1e-9) setView(v.t0, v.t1);   // the span is (t0 + d) - t0: an ulp apart
}
function tick() {   // one playback frame (the frame scheduler calls it while playing; play() calls it once)
  if (!S.playing) { cpStop(); spLeave("pause"); flushFrame(); return; }
  PH.v = appTickPos();
  followResume();   // P4-wf10: the 2 s of playback after the edit are over: follow again
  if (PH.v >= maxDur()) { pause(); S.playhead = 0; invalidate(F.HEAD); flushFrame(); return; }
  const now = AC.tickAt;
  let strip = SP.on && spTick(now);   // P4-1: a strip of the smooth follow moves by itself
  if (!SP.on && SP.req && spEnter(now, true)) strip = spTick(now);   // P4-1b: a strip asked for ahead shows now
  if (SP.on) { spWrite(now); requestFrame(); }   // #463: the strip's 2D transforms are written per frame (rAF), so the loop keeps running under a strip
  if (!strip) followStep();
  const cp = !strip && cpArm(now);   // P4-1: the compositor moves the playhead while the view stands still
  if (cp) spAhead(now);
  FR.flags |= F.PH;   // cheap: the overlay and the time; a follow step asked for the whole view
  const ox = Math.round(PH.v / maxDur() * GEO.ovW * 2);   // the overview playhead, in half pixels: a VIEW frame draws it too
  if (ox !== FR.ovx) { FR.ovx = ox; FR.flags |= F.HEAD; }
  flushFrame();
  if (!strip && !cp) spEnter(now);   // the view glides in the frame path: the first strip
  /* #463: the flag says this tick did NOT draw the playhead (an animation did). cp is also true when cpStill said
     show:false and no animation exists (the playhead left the view in off mode): the frame path drew it, so the
     tick must stay readable for drawnPlayheadAt. */
  CP.hist.push([now, AC.drawn, performance.now(), (cp && !!CP.anim) || SP.on]);
  if (CP.hist.length > CP_KEEP) CP.hist.splice(0, CP.hist.length - CP_KEEP);
  if (!cp && !SP.on) { requestFrame(); return; }
  clearTimeout(CP.slow); CP.slow = setTimeout(() => { if (S.playing) requestFrame(); }, CP_SLOW_MS);   // the counters without meter frames
}

/* P3-13c: the key preset ("default" = Pro Tools, or "cubase"); the app setting "key_preset". The page keys follow at once
   (DSKeys.preset); the menu accelerators follow through DSShell.sync (shell_sync carries the preset). */
function setKeyPreset(name, save) {
  if (!DSKeys.setPreset(name)) return false;
  const sel = $("#setKeyPreset");
  if (sel) sel.value = DSKeys.preset;
  if (save !== false && B.appSettingsSet) B.appSettingsSet("key_preset", DSKeys.preset).catch(e => setMsg(`settings: ${errText(e)}`));
  DSShell.sync();
  if (save !== false) setMsg(`key preset: ${DSKeys.presetName()}`);
  return true;
}
/* P4-29: native app feel. The page must not behave like a web page in the webview: no text selection (CSS user-select in
   style.css; here the events that CSS cannot stop: Select All from the native Edit menu, the browser context menu, drags of
   images and text, page zoom by pinch or Cmd+wheel or Cmd+plus). Text fields, text areas, contenteditable and `.selectable`
   text (About, error details) keep their selection, their menu and their drags. The app's own context menus (track header,
   spike lane, wave lane) call preventDefault themselves and stay. */
function nativeFeel() {
  const SEL = "input:not([type=checkbox]):not([type=radio]):not([type=button]):not([type=range]), textarea, [contenteditable=''], [contenteditable='true'], .selectable";
  const inText = t => !!(t && t.closest && t.closest(SEL));
  const node = n => n && (n.nodeType === 1 ? n : n.parentElement);
  document.addEventListener("contextmenu", ev => { if (!inText(ev.target)) ev.preventDefault(); });
  document.addEventListener("dragstart", ev => { if (!inText(ev.target)) ev.preventDefault(); });   // the app's own drags use mouse events, not HTML drag and drop
  document.addEventListener("selectstart", ev => { if (!inText(node(ev.target))) ev.preventDefault(); });
  document.addEventListener("mousedown", ev => {   // P4-29 fix: a double-click selects a word; the stray selection made the
    // selectionchange handler call removeAllRanges, which forced a layout inside the event dispatch (the frames test)
    if (ev.detail > 1 && !inText(ev.target)) ev.preventDefault();
  });
  /* No selectionchange handler here: a removeAllRanges in it forces a layout inside the event
     dispatch (the frames test). The stray selection cannot happen: the mousedown guard above
     stops the multi-click word selection and selectstart stops the drags. */
  for (const t of ["gesturestart", "gesturechange", "gestureend"]) document.addEventListener(t, ev => ev.preventDefault());
  window.addEventListener("wheel", ev => { if (ev.ctrlKey || ev.metaKey) ev.preventDefault(); }, { passive: false });   // pinch arrives as ctrl+wheel; the lanes zoom the timeline with it (P4 #145)
  window.addEventListener("keydown", ev => {
    const mod = ev.metaKey || ev.ctrlKey;
    if (!mod || ev.altKey) return;
    const k = ev.key.toLowerCase();
    if (k === "a" && !ev.shiftKey && !inText(ev.target)) ev.preventDefault();   // Select All outside a field: nothing to select (the app has no Select All of hits)
    else if (["+", "=", "-", "_", "0"].includes(k) && !(window.DSKeys && DSKeys.find(ev))) ev.preventDefault();   // page zoom; the app's own zoom keys stay
  }, true);
}
nativeFeel();


/* ---------------- global handlers ---------------- */
function setupGlobal() {
  $("#playBtn").onclick = () => playStop();
  $("#stopBtn").onclick = () => stopAndReturn();
  $("#analyzeAllBtn").onclick = () => startAnalysis(null);
  $("#exportAllBtn").onclick = () => openExport("all");
  setupExport();
  $("#progressCancel").onclick = async () => { $("#progressCancel").disabled = true; try { await B.jobCancel(S.sid); setMsg("cancelling…"); } catch(e) { setMsg(`cancel failed: ${e.message}`, "error"); } };   // P3-4d
  $("#relinkFind").onclick = findMovedAudio;
  $("#relinkDismiss").onclick = () => { relinkHidden = S.sid; refreshRelinkBar(); };
  $("#relinkClose").onclick = () => $("#relinkModal").classList.add("hidden");
  $("#relinkAgain").onclick = findMovedAudio;
  // P3-13a: the sidebar, the File menu and the ? button are gone; the native menu (shell.js) runs their actions
  $("#settingsDone").onclick = () => settingsClose();   // #387: the focus goes back to the opener
  document.querySelectorAll("#setCats .setCatBtn").forEach(b => b.onclick = () => {   // #387: the category list
    settingsShowCat(b.dataset.cat);
    localStorage.setItem("dsg_setcat", b.dataset.cat);   // the last open category, remembered per app home
  });
  plHooks();   // #387: the licence and missing-plugin dialogs of the detector plugins
  pmHooks();
  $("#setOutput").onchange = ev => chooseAudioOutput(ev.target.value);
  const kp = $("#setKeyPreset");   // P3-13c
  for (const [value, text] of Object.entries(DSKeys.PRESETS)) { const o = document.createElement("option"); o.value = value; o.textContent = text; kp.appendChild(o); }
  kp.value = DSKeys.preset;
  kp.onchange = ev => setKeyPreset(ev.target.value);
  $("#setZoomTall").onchange = ev => setZoomTall(ev.target.checked);   // P4-27
  $("#setTabMode").value = SETTINGS().ui.tabMode === "all" ? "all" : "active";   // #175
  $("#setTabMode").onchange = ev => setTabMode(ev.target.value);
  $("#setAuditionTab").checked = !!SETTINGS().ui.auditionTab;   // P4-40d
  $("#setAuditionTab").onchange = ev => setAuditionTab(ev.target.checked);   // #240: one flip for button, checkbox, menu and settings
  setupMaster();   // P3-16
  $("#tempoDetectBtn").onclick = () => DSShell.menu({id: "detect_tempo"});   // owner 2026-10-14: the same as Track > Detect Tempo
  setupInspector();   // P3-14a
  $("#helpDone").onclick = () => $("#helpModal").classList.add("hidden");   // kept for the tests; the manual wires its own buttons in DSHelp.hooks
  DSHelp.hooks();   // #255: the manual (chapters, back/forward, links out)
  $("#undoHistDone").onclick = () => $("#undoHistModal").classList.add("hidden");   // P4-40a
  $("#importCancel").onclick = () => $("#importModal").classList.add("hidden");
  // global settings bindings
  const st = SETTINGS();
  $("#setClickOn").checked = st.click.enabled;
  $("#setClickVol").value = Math.round(st.click.volume*100);
  $("#setClickOn").onchange = ev => setClick(ev.target.checked);
  $("#setClickVol").oninput = ev => { st.click.volume = ev.target.value/100; saveSettings(); };
  const pitchBox = $("#setPitches");
  for (const role of ["kick","snare","tom","hat","cymbal","other"]) {
    const row = document.createElement("div"); row.className = "prow";
    const lab = document.createElement("label"); lab.textContent = role[0].toUpperCase()+role.slice(1);
    const ctl = document.createElement("div"); ctl.className = "pctl";
    const inp = document.createElement("input"); inp.type = "number"; inp.id = `setPitch-${role}`; inp.value = st.click.pitches[role] || 1500; inp.step = 100; inp.style.width = "90px";
    lab.htmlFor = inp.id;
    inp.onchange = () => { st.click.pitches[role] = Math.max(50, Math.min(8000, +inp.value||1500)); saveSettings(); };
    const unit = document.createElement("span"); unit.className = "punit"; unit.textContent = "Hz";
    ctl.append(inp, unit); row.append(lab, ctl); pitchBox.appendChild(row);
  }
  $("#clickBtn").classList.toggle("on", !!st.click.enabled);
  $("#clickBtn").onclick = () => toggleClick();
  $("#auditionTabBtn").classList.toggle("on", !!st.ui.auditionTab);   // #240
  $("#auditionTabBtn").setAttribute("aria-pressed", st.ui.auditionTab ? "true" : "false");
  $("#auditionTabBtn").onclick = () => toggleAuditionTab();
  $("#followBtn").onclick = () => { const m = ["smooth", "page", "off"]; setFollow(m[(m.indexOf(S.followMode) + 1) % 3]); setMsg(`follow playhead: ${S.followMode}`); };
  $("#snapChk").onchange = ev => setSnap(ev.target.checked);
  $("#raKeep").onclick = () => { if (pendingRA) { const l = pendingRA; pendingRA = null; runAnalysis(l, true); } if (pendingRASel) { const s = pendingRASel; pendingRASel = null; runReanalyzeSelection(s, true); } };
  $("#raDiscard").onclick = () => { if (pendingRA) { const l = pendingRA; pendingRA = null; runAnalysis(l, false); } if (pendingRASel) { const s = pendingRASel; pendingRASel = null; runReanalyzeSelection(s, false); } };
  $("#raCancel").onclick = () => { $("#reanalyzeModal").classList.add("hidden"); pendingRA = null; pendingRASel = null; setMsg("re-analyze cancelled"); };
  document.querySelectorAll(".zBtn").forEach(b => b.onclick = () => zoomPlayhead(b.dataset.z === "in" ? 0.5 : 2));
  $("#zoomSel").onchange = ev => { const v = ev.target.value; ev.target.value = ""; if (v && needProject()) applyZoomSpan(v); };
  $("#ruler").addEventListener("mousedown", rulerDown);
  const ov = $("#overview");
  function ovMode(px) {
    const maxD = maxDur(), W = GEO.ovW;
    const x0 = S.view.t0/maxD*W, x1 = S.view.t1/maxD*W;
    const phX = S.playhead/maxD*W;
    if (Math.abs(px-phX) <= 6) return "scrub";
    if (Math.abs(px-x0) <= 5) return "resizeL";
    if (Math.abs(px-x1) <= 5) return "resizeR";
    return (px > x0 && px < x1) ? "move" : "jump";
  }
  function ovCursor(px) {
    const m = ovMode(px);
    return m==="scrub" ? "col-resize" : (m==="resizeL"||m==="resizeR") ? "ew-resize" : "crosshair";
  }
  ov.addEventListener("mousemove", ev => { ov.style.cursor = ovCursor(ovXOf(ev)); });
  ov.addEventListener("mousedown", ev => {
    if (ev.button!==0) return;
    S.followHold = true;   // F1: any overview interaction is a user view change
    const maxD = maxDur(), W = GEO.ovW;
    const mode = ovMode(ovXOf(ev));
    const startX = ovXOf(ev);
    const t0 = S.view.t0, t1 = S.view.t1;
    if (mode === "scrub") {
      const mv = e2 => { S.playhead = Math.max(0, Math.min(ovXOf(e2)/W*maxD, maxD)); invalidate(F.HEAD); updateStatus(); };
      mv(ev);
      const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
      window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
      return;
    }
    if (mode === "resizeL" || mode === "resizeR") {
      const mv = e2 => {
        const t = Math.max(0, Math.min(ovXOf(e2)/W*maxD, maxD));
        if (mode === "resizeL") setView(Math.min(t, t1-0.002), t1);
        else setView(t0, Math.max(t, t0+0.002));
      };
      const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
      window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
      return;
    }
    if (mode === "jump") centerOn(startX/W*maxD);   // jump, then continue dragging the window
    const anchorT0 = S.view.t0;
    const mv = e2 => {
      const dt = (ovXOf(e2) - startX)/W*maxD;
      // direct manipulation: the rectangle follows the pointer (drag right → view moves right)
      const n0 = Math.max(0, Math.min(anchorT0 + dt, maxD - viewDur()));
      setView(n0, n0 + viewDur());
    };
    const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
  });
  ov.addEventListener("wheel", ev => {
    ev.preventDefault();
    const dY = ev.deltaY * dScale(ev.deltaMode), dX = ev.deltaX * dScale(ev.deltaMode);
    if (Math.abs(dX) > Math.max(2, Math.abs(dY)) || ev.shiftKey) {
      const secPerPx = maxDur() / GEO.ovW;
      panView((dX || dY) * secPerPx);
    } else $("#lanesScroll").scrollBy({ top: dY });
  }, {passive:false});

  window.addEventListener("keydown", ev => {
    if (DSShell.menuOwnsKey(ev)) return;   // P3-4b: the native menu runs this key (app only), so it runs once
    // a field keeps its keys (P3-13a: a check box does not, so J still works after a click on the snap box).
    // #436: a slider gives its keys to the app too (Space plays after a click on a slider); only a text
    // field keeps them, and the status line says when a transport key went into a text field.
    const tg = ev.target;
    const keeps = tg.tagName === "SELECT" || tg.tagName === "TEXTAREA" || tg.isContentEditable
        || (tg.tagName === "INPUT" && !["checkbox", "radio", "button", "range"].includes(tg.type));
    if (keeps) {
      const swallowed = DSKeys.find(ev);   // the key has a command: the field takes it, say so
      if (swallowed && (swallowed.cmd.id === "play_stop" || swallowed.cmd.id === "half_speed")) {
        const what = tg.title || tg.id ? `the ${tg.title || tg.id} field` : "the text field";
        setMsg(`"${DSKeys.display(swallowed.key)}" went to ${what}: press Escape and click the lanes to play`);
      }
      return;
    }
    // P3-13b: the key table (static/keymap.js) says which command the key runs
    const hit = DSKeys.find(ev);
    if (!hit) return;
    if (hit.cmd.id !== "deselect") ev.preventDefault();
    if (hit.cmd.menu && DSKeys.parse(hit.key).key === "Tab") DSKeys.notePageRun(hit.cmd.menu);   // #317: see DSKeys.menuOwns
    runKey(hit.cmd.id, ev);
  });
}

/* P3-13b: what a key of the table does in the page. The menu items (shell.js handlers) and these functions
   call the same editor commands; a key differs only where the old key did: Delete without a selected hit
   removes the selected track after a confirm (P4-39a), the nudge keys pan the view when no hit is selected, and Escape deselects. A command that is
   not here (a menu key in the web page, where no native menu runs it) runs its menu handler. */
function nudgeOrPan(step) {   // step in units of the nudge value (P3-20a); a range selection nudges its hits (#151)
  if (LN.sel || S.sel) nudgeUnits(step);
  else panView(Math.sign(step) * viewDur() * 0.2);
}
const KEY_RUN = {
  next_hit: () => gotoEvent(1, false),
  prev_hit: () => gotoEvent(-1, false),
  next_hit_all: () => gotoEvent(1, true),
  prev_hit_all: () => gotoEvent(-1, true),
  review_next: () => reviewStep(1),
  review_prev: () => reviewStep(-1),
  other_mics: () => mxToggle(),   // #157: View > Other Mics Under Hit (O)
  zoom_in: () => zoomStep(true),
  zoom_out: () => zoomStep(false),
  zoom_full: () => zoomPreset(0),
  zoom_30s: () => zoomPreset(1),
  zoom_5s: () => zoomPreset(2),
  zoom_500ms: () => zoomPreset(3),
  zoom_50ms: () => zoomPreset(4),
  zoom_5ms: () => zoomPreset(5),
  play_stop: ({ key }) => playStop(key),
  half_speed: ({ key }) => halfSpeedPlayStop(key),   // P3-20b
  go_start: () => goTo(0),
  go_end: () => goTo(projectEnd()),
  go_play_start: () => goTo(S.playStartPos),
  click: () => toggleClick(),
  snap: () => toggleSnap(),
  follow_toggle: () => toggleFollow(),
  mute_track: () => trackButton(".bMute"),
  solo_track: () => trackButton(".bSolo"),
  delete_hit: () => { if (!tmDeleteSel() && !lnDeleteMark()) { if (LN.sel || S.sel) deleteSelectedHit(); else if (S.selTrack && S.tracks[S.selTrack]) removeTrack(S.selTrack); } },   // P4-39a: no hit, no marker: the selected track (after the confirm)   // P4-6a: a selected Learn mark bar too   // P4-8 and P4-40g: a selected tempo, meter or review marker first   // #151: a range selection deletes its hits
  markers_set: () => markerAtSelection(),   // P4-40g: M sets a marker at the selected hit or the playhead
  marker_next: () => markerJump(1),   // P4-40g: Option+M
  marker_prev: () => markerJump(-1),   // P4-40g: Option+Shift+M
  nudge_earlier: () => nudgeOrPan(-1),
  nudge_later: () => nudgeOrPan(1),
  nudge_earlier_10: () => nudgeOrPan(-10),
  nudge_later_10: () => nudgeOrPan(10),
  deselect: () => { if (auStop()) return; lnClearSel(); if (LN.mark) lnSetMark(null); S.sel = null; if (TM.sel) { TM.sel = null; invalidate(F.VIEW); } requestFrame(); updateStatus(); },   // the frame redraws the track of the old selection
  undo: () => undo(),
  redo: () => redo(),
  prev_track: () => selectTrackStep(-1),   // P3-19
  next_track: () => selectTrackStep(1),
  zoom_toggle: () => zoomToggle(false),
  zoom_keep: () => zoomToggle(true),
  nudge_value_up: () => stepNudgeValue(1),   // P3-20a
  nudge_value_down: () => stepNudgeValue(-1),
  save: () => saveProjectNow().catch(() => {}),   // #249 R02: the refusal is already on the status line
  audition: ({ shift }) => audition(shift),   // P4-40d
};
function runKey(id, ev) {
  const f = KEY_RUN[id] || DSShell.handlers[id];
  if (!f) return false;
  try {
    const r = f({ id, shift: !!(ev && ev.shiftKey), key: ev ? DSKeys.keyOf(ev) : null });
    if (r && typeof r.catch === "function") r.catch(e => setMsg(`${id} failed: ${e.message}`, "error"));
  } catch (e) { setMsg(`${id} failed: ${e.message}`, "error"); }
  return true;
}

function rulerDown(ev) {   // the time ruler and the Bars|Beats ruler (P4-8): a click puts the playhead
  if (ev.button!==0) return;
  ev.preventDefault();
  S.followHold = true;   // F1: repositioning the playhead is a user intent
  const move = e2 => { S.playhead = Math.max(0, snapPlayhead(Math.min(xToT(rulerXOf(e2), GEO.rulerW), maxDur()))); invalidate(F.HEAD); updateStatus(); };
  move(ev);
  const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
  window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
}

/* ---------------- boot ---------------- */
(async function boot() {
  persistInit({ edIdle });   // R17 step 2: persist waits for the edit queue through the injected hook (edit.js owns edIdle since step 3)
  editInit({ pushEvents, voidHitDrag: name => { if (hitDrag && hitDrag.track === name) hitDrag = null; } });   // R17 step 3: edit.js flushes the writes of other steps through the injected hook; voidHitDrag is the input layer's (a replaced hit list voids an in-flight drag)
  sessionInit({ GLQ, lnReset, auReset, tmLoad, selMem: () => selMem, setSelMem: v => { selMem = v; }, exTempoReset: () => { EX.tempoAck = false; } });   // R17 step 6: session.js resets the module state of the owners a load replaces (the GL tile queue, LN/AU/TM, the selection memory) through the injected hooks; #151: the default-tempo answer is per project
  tmInit(); measureGeo(); geoObserve();   // P4-8: the map rulers; P3-14d: the geometry once now, then from the ResizeObserver
  setupUpload(); setupGlobal(); setupInfoLine(); staleSetup();
  if (B.onDirty) {   // P3-4a: the service tells when a project becomes dirty or clean
    let listTimer = null;
    B.onDirty(p => {
      if (p.sid === S.sid) setDirtyState(p.dirty);
      clearTimeout(listTimer);   // one refresh of the project list (its dots) for a burst of events
      listTimer = setTimeout(() => initSessions(true).catch(() => {}), 100);   // P3-4d: keepNone, else a late event reopens a project the user closed
    });
  }
  try {
    await loadViewSettings();   // P3-14a: the Inspector's state, before the first layout the user sees
    await initSessions();
    await loadProject();
    showBackend();
  } catch(e) {
    setMsg(`init failed: ${e.message}`, "error");
  }
  await DSShell.boot();   // P3-4b: menu events, Finder paths (app only)
  B.appReady().catch(() => {});   // P3-17: the first layout is done; the shell shows the window and closes the splash
})();
