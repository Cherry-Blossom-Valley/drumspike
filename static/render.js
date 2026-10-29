/* Drum Spike v2 — render (R17 step 5, #55 per docs/review/R17-plan.md)
   The frame scheduler (F, FR, invalidate/invalidateHits, onFrame, flushFrame — P3-14d),
   the measured geometry (GEO, measureGeo, geoObserve — read after layout, never in a
   frame) and the compositor playhead (CP, P4-1, #463, #14). Plain script (no build
   step), loaded after transport.js and before app.js. The blocks are copied verbatim
   from app.js; the draw functions they call (drawAll, drawHead, drawMeters, drawHits,
   glRepaint, drawOverview, sizeCanvases, syncLaneBuffers, spLeave, spLeftCheck, tick)
   and the strip state SP stay in app.js for now and resolve at call time. The FR.n
   counters keep their shape: the self-test and test_frames.py read them unchanged.
   The one load-time statement is the resize listener registration (no DOM read).
   spWrite moves with CP: it writes the strip from the compositor clock. */
"use strict";

/* ---------------- frame scheduler (P3-14d, concurrency review Q2) ----------------
   Input does not draw. A wheel, drag, scrub, key or edit changes the state and calls invalidate(flags) or
   invalidateHits(track); one requestAnimationFrame callback (flushFrame) draws what the flags ask for, once per frame,
   however many events came in that frame (W2). The frame reads no layout: the sizes and places of the editor, the
   lanes area, the overview, the ruler and every row come from GEO, which a ResizeObserver fills after the browser's
   own layout (measureGeo). So the frame only writes, and the browser lays out once, after it (W3).
   Flags: VIEW the whole view (ruler, overview, the near lanes, the GL lanes, the playhead); HEAD the playhead, the
   time and the overview; PH the playhead and the time (the playback tick); GL the GL lanes only (a scroll, a row
   moved or changed its class); SIZE the canvas sizes (from GEO); NEAR the canvas stores of the lanes near the view.
   invalidateHits(track) redraws the hit lines of one track (its spike lane and the 2D layer of its wave lane) and
   posts no GL draw: the wave did not change. A change of the selected hit redraws the tracks of the old and the new
   selection by itself. redrawAll() still draws everything at once (tests and the self-test call it).
   P3-16: METER the level meters of the tracks and the master (transforms only, drawn before a VIEW frame returns). */
const F = { VIEW: 1, HEAD: 2, PH: 4, GL: 8, SIZE: 16, NEAR: 32, METER: 64 };
const FR = { flags: 0, tracks: new Set(), raf: 0, inFrame: false, sel: "", sized: "", head: ["", "", "", ""],
  last: 0, n: { frames: 0, view: 0, tracks: 0, head: 0, gl: 0, size: 0, meter: 0, skipped: 0 } };   // counters: the tests and the self-test read them
function requestFrame() { if (!FR.raf) FR.raf = requestAnimationFrame(onFrame); }
function invalidate(flags) { FR.flags |= flags; requestFrame(); }
function invalidateHits(track) {   // no track: every track (new events from an analysis, velocities)
  for (const n of track ? [track] : S.order) FR.tracks.add(n);
  requestFrame();
}
/* P4-40c: the 60 Hz cap. On a 120 Hz display a scrub or a hit drag asked for a redraw in every animation frame.
   A frame that comes less than FR_MIN_MS (12, between the 8.3 ms of 120 Hz and the 16.7 ms of 60 Hz) after the last drawn one waits: the request stays pending (FR.raf is set
   again) and the next animation frame serves it. Only the frame path off playback is capped; the playback step
   (`tick`), the compositor playhead (`CP`) and the strip (`SP`) keep their own pace. No setting. */
const FR_MIN_MS = 12;   // not 16: frame times of a 60 Hz display jitter below 16.7 ms and lost a frame in five (measured)
const frPending = () => !!(FR.flags || FR.tracks.size || SP.leaving || selKey() !== FR.sel);
function onFrame(t) {
  FR.raf = 0;
  if (S.playing) tick(t);   // the playback step moves the playhead, follows it and draws the frame
  else if (FR.last && t - FR.last < FR_MIN_MS && t >= FR.last && frPending()) { FR.n.skipped++; requestFrame(); }
  else flushFrame(t);
}
function selKey() { return S.sel ? `${S.sel.track}\u0000${S.sel.index}` : ""; }
function flushFrame(t) {
  if (SP.leaving) spLeftCheck();   // P4-1
  const sk = selKey();
  if (sk !== FR.sel) {   // the selected hit changed: redraw the hit lines of the old and the new track
    if (FR.sel) FR.tracks.add(FR.sel.split("\u0000")[0]);
    if (S.sel) FR.tracks.add(S.sel.track);
    FR.sel = sk;
  }
  if (SP.on && ((FR.flags & (F.VIEW | F.SIZE | F.NEAR | F.GL)) || FR.tracks.size)) spLeave("draw");   // P4-1: back to the frame path
  let f = FR.flags;
  const tracks = FR.tracks;
  if (!f && !tracks.size) return;
  FR.flags = 0; FR.tracks = new Set();
  if (t) FR.last = t;   // P4-40c: the time of the last drawn frame (the playback step passes none)
  FR.inFrame = true; FR.n.frames++;
  try {
    if (f & F.METER) { drawMeters(); FR.n.meter++; }   // P3-16
    if (f & F.SIZE) { sizeCanvases(); FR.n.size++; f |= F.VIEW; }
    else if ((f & F.NEAR) && syncLaneBuffers()) f |= F.VIEW;   // a lane came near: it has a new canvas store to draw
    if (f & F.VIEW) { drawAll(); FR.n.view++; return; }
    for (const n of tracks) if (drawHits(n)) FR.n.tracks++;
    if (f & F.GL) { glRepaint(); FR.n.gl++; }
    if (f & F.HEAD) { drawOverview(); FR.n.head++; }
    if (f & (F.HEAD | F.PH)) drawHead();
  } finally { FR.inFrame = false; }
}

/* ---------------- geometry (P3-14d): read after layout, never in a frame ----------------
   measureGeo runs in the ResizeObserver callback (the browser has just laid out, so the reads do not force a
   layout) and once at boot. The observer watches the editor, the lanes area, the overview, the ruler and every row;
   a change of the rows (built, moved) observes them again, and a new observation reports at once. The scroll
   position comes from the scroll event. Row places are in the coordinates of the scrolled content. */
const GEO = { ok: false, editorW: 0, laneW: 300, rulerW: 0, laneX: 0, rulerX: 0, ovX: 0, ovW: 0, ovH: 0,
  scL: 0, scT: 0, scW: 0, scH: 0, scX: 0, scY: 0, top: 0, waveX: 0, waveH: 120, spikeH: 33, dpr: 1, rows: {}, n: 0 };
let _geoRO = null;
function measureGeo() {
  const ed = $("#editor"), sc = $("#lanesScroll"), ow = $("#overviewWrap");
  GEO.editorW = ed.clientWidth;
  GEO.laneW = Math.max(300, GEO.editorW - S.theadW - 2);
  GEO.rulerW = GEO.editorW - S.theadW - 2;
  GEO.scL = sc.offsetLeft; GEO.scT = sc.offsetTop; GEO.scW = sc.clientWidth; GEO.scH = sc.clientHeight;
  const sr = sc.getBoundingClientRect();
  GEO.scX = sr.left + sc.clientLeft; GEO.scY = sr.top + sc.clientTop; GEO.top = sc.scrollTop;
  GEO.ovW = ow.clientWidth; GEO.ovH = ow.clientHeight;
  GEO.ovX = $("#overview").getBoundingClientRect().left; GEO.rulerX = $("#rulerClip").getBoundingClientRect().left;
  const rows = {}, y0 = GEO.top - GEO.scY;
  sc.querySelectorAll(".track").forEach(row => {   // P4-1: the lane box, not the canvas, which a strip moves by a transform
    const r = row.getBoundingClientRect(), w = row.querySelector("canvas.wave"), s = row.querySelector("canvas.spike");
    const tr = row.querySelector(".tlanes").getBoundingClientRect(), wt = tr.top + w.offsetTop;
    rows[row.dataset.name] = { top: r.top + y0, bottom: r.bottom + y0, wave: wt + y0, waveH: w.clientHeight };   // P4-27: a row can be taller than the others (waveH of its own)
    GEO.waveX = tr.left - GEO.scX; GEO.laneX = tr.left; GEO.spikeH = s.clientHeight;
    if (!S.rowH || !S.rowH[row.dataset.name] || GEO.waveH === undefined) GEO.waveH = w.clientHeight;   // GEO.waveH = the height of a row without an override
  });
  GEO.rows = rows; GEO.dpr = window.devicePixelRatio || 1; GEO.ok = true; GEO.n++;
}
function sizeKey() { return [GEO.laneW, GEO.rulerW, GEO.ovW, GEO.ovH, GEO.waveH, GEO.spikeH, GEO.dpr, rowHKey(), VL.on ? VL.h : 0].join(","); }   // P3-14c: the lane heights too; P4-27: the rows with their own height; #208: the velocity lane's
let _geoLanes = "";
function onGeo() {   // the GL lanes are drawn again only when the lanes area or a row moved (not when, say, the status line wrapped)
  measureGeo();
  const lanes = [GEO.scL, GEO.scT, GEO.scW, GEO.scH, GEO.waveX, GEO.top, ...Object.entries(GEO.rows).map(([n, g]) => n + g.wave)].join(",");
  const moved = lanes !== _geoLanes, sized = sizeKey() !== FR.sized;
  _geoLanes = lanes;
  if (!moved && !sized) return;
  invalidate(F.GL | F.NEAR | F.METER | (sized ? F.SIZE : 0));   // P4-40h: rows that moved into the view paint their meters
  if (!S.playing) flushFrame();   // the layout is fresh now: draw in this frame, before it is painted
}
function geoObserve() {   // (again) every element the geometry depends on
  const sc = $("#lanesScroll");
  if (!_geoRO) {
    _geoRO = new ResizeObserver(onGeo);
    sc.addEventListener("scroll", () => { GEO.top = sc.scrollTop; invalidate(F.GL | F.NEAR | F.METER); }, { passive: true });   // P4-40h: rows that enter the view paint their meters once
    new MutationObserver(geoObserve).observe(sc, { childList: true });   // rows built, moved or removed
    new MutationObserver(recs => { if (recs.some(r => r.target !== sc && r.target.tagName !== "CANVAS")) invalidate(F.GL); }).observe(sc, { subtree: true, attributes: true, attributeFilter: ["class"] });   // muted: the GL lane fades; P4-40b: the cursor classes of #lanesScroll and of a spike canvas draw nothing
  }
  _geoRO.disconnect();
  for (const el of [$("#editor"), sc, $("#overviewWrap"), $("#rulerClip"), ...sc.querySelectorAll(".track")]) _geoRO.observe(el);
}
window.addEventListener("resize", () => { GEO.dpr = window.devicePixelRatio || 1; invalidate(FR.sized !== sizeKey() ? F.SIZE : F.HEAD); });   // a new screen scale; sizes come from the observer
function rowOf(name) { return document.querySelector(`.track[data-name="${CSS.escape(name)}"]`); }
function laneSize(canvas) {
  if (canvas.classList.contains("spike")) return [GEO.laneW, GEO.spikeH];
  if (canvas.classList.contains("vel")) return [GEO.laneW, VL.h];   // P4-40f: the velocity lane (#208: its own height)
  const row = canvas.closest(".track"), g = row && GEO.rows[row.dataset.name];   // P4-27: a row with its own height
  return [GEO.laneW, g && g.waveH ? g.waveH : GEO.waveH];
}

/* ---------------- compositor playhead (P4-1, phase 3 finding X-2, review Q7) ----------------
   While the transport plays and the view stands still (follow off; page follow between two page turns; smooth
   follow left of its anchor at the start of the song, or at its end), the playhead overlay is moved by a Web
   Animation: a linear translateX that the compositor (Core Animation in the WKWebView) runs between the 4 Hz playhead
   messages. The page draws no frame for it; P3-15a experiment B put the 60 Hz frames at about 2 points of the app's
   CPU. The animation comes from the transport model (appPos: the messages and the 1 s clock sync) and is armed again
   only when the model and the animation differ by more than CP_TOL_S, or the view, the lane width or the speed
   changed. A timer runs the tick at the moment the view must move (the page turn, the smooth anchor, the end of the
   view or of the song). The ticks that still run (the meter frames, about 25 Hz; else every CP_SLOW_MS) update the
   counters and the overview. A seek, a pause, a zoom or a pan stop it (cpStop) or arm it again for the new view.
   CP.segs and CP.hist keep what was shown (the animations and the drawn ticks) for drawnPlayheadAt. */
const CP = { anim: null, at0: 0, pos0: 0, rate: 1, pr: 1, dur: 0, until: 0, key: "", timer: 0, slow: 0, arms: 0, segs: [], hist: [] };
const CP_TOL_S = 0.00025, CP_SLOW_MS = 100, CP_KEEP = 16384, CP_GUARD_S = 0.15;
/* #463 (re-scope, Ardour's editor.cc super_rapid_screen_update is the model): the 4 Hz anchors carry ms of jitter and
   the clock sync offset switches samples every second, so a re-arm per sample is a visible step when zoomed in. The
   page dead-reckons instead and slews: CP_SLEW of the sample error plus a small integrator per sample (Ardour:
   guess += err*0.12 + I; I += 0.0144*(err - I)), a hard re-anchor only on seek, play, pause, speed change, epoch
   change or |err| > CP_HARD_S. The running compositor animations are never restarted for a small drift: their
   playbackRate is corrected (CP_TAU_S time constant), which WebKit keeps on the compositor (measured in the gate). */
const CP_SLEW = 0.12, CP_SLEW_I = 0.0144, CP_HARD_S = 0.02, CP_TAU_S = 0.5;
/* #463 (coordinator bound): the pr clamp slews at most CP_PR_MAX - 1 song s per real second, so within 1 s of
   song time it can remove at most 0.1 song s of divergence. A divergence the clamp cannot remove within 1 s of
   song time is a real discontinuity, not drift: re-anchor then. Threshold = (CP_PR_MAX - 1) x 1 s = 0.1 song s
   (100 ms, speed-independent: the clamp removes (CP_PR_MAX - 1) x speed song s per real s, and 1 s of song time
   takes 1 / speed real s). Seek/play/pause/speed/epoch keep re-anchoring as before. */
const CP_PR_MIN = 0.9, CP_PR_MAX = 1.1, CP_REANCHOR_S = CP_PR_MAX - 1;
const cpSnap = x => Math.round(x * GEO.dpr) / GEO.dpr;   // #463: whole device pixels (the playhead overlay; the strip's 2D transforms are written per frame)
function cpPos(t) { return CP.pos0 + Math.min(Math.max(0, t - CP.at0), CP.dur) / 1000 * CP.rate * CP.pr; }
function cpRate(e) {   // #463: correct the drift e (song s) through the playback rate, time constant CP_TAU_S
  const now = performance.now(), shown = cpPos(now);   // the clock's position now, before the rate change
  const want = CP.rate * CP.pr + e / CP_TAU_S;   // song s per real second
  CP.pr = Math.min(CP_PR_MAX, Math.max(CP_PR_MIN, want / CP.rate));
  for (const a of SP.anims) { try { a.updatePlaybackRate(CP.pr); } catch (x) { CP.pr = 1; } }
  if (CP.anim) { try { CP.anim.updatePlaybackRate(CP.pr); } catch (x) { CP.pr = 1; } }
  /* #463: updatePlaybackRate keeps the animation's current position; the model must too, else the seg's
     pos0 + (t - at0) * rate * pr jumps by (t - at0) * rate * dpr at the change (a ms of slew is a step of
     tens of ms on a long seg). Rebase the clock at `now`: the position stays continuous, the end time too. */
  CP.at0 = now; CP.pos0 = shown; CP.dur = Math.max(0, CP.at0 + CP.dur - now);
  /* #14: end the drawn seg here and arm the next one from `now`, instead of moving the last seg in place: the
     rAF frames sample drawnPlayheadAt at timestamps between the ticks, and a seg moved to `now` leaves them
     nothing (under a strip the hist ticks are flagged, so the fallback finds only nulls - the smooth-30s case
     measured 2 paired frames of 43). The seg history is what the page showed; the slew must not erase it. */
  cpEndSeg(now);
  CP.segs.push({ at0: now, pos0: shown, rate: CP.rate, pr: CP.pr, dur: CP.dur, end: Infinity });
  if (CP.segs.length > CP_KEEP) CP.segs.splice(0, CP.segs.length - CP_KEEP);
}
/* #14: the test hook that forces a real step of the drawn playhead: the model the page shows (the clock and
   the live seg with it) jumps by `s` song s at once, so every later drawn position carries the jump until a
   re-anchor. Only the tests call it (test_playhead_steps.py). */
window.__DS_STEP_PLAYHEAD = s => {
  const now = performance.now(), shown = cpPos(now);
  CP.at0 = now; CP.pos0 = shown + s;
  const g = CP.segs[CP.segs.length - 1];
  if (g && g.end === Infinity) { g.at0 = now; g.pos0 = shown + s; }
};
function cpEndSeg(t) { const g = CP.segs[CP.segs.length - 1]; if (g && g.end === Infinity) g.end = Math.min(t, g.at0 + g.dur); }
/* #463: the 2D strip els move per frame from the slewn clock (openDAW, peaks.js, wavesurfer write translateX per
   frame): a compositor animation interpolates between its keyframes and sits on fractional pixels mid-flight, so
   spFreeze and the pixel tests saw the strip off the whole device pixel. The GL strip keeps its compositor
   animation. A frame writes a style only when the snapped value changed. */
function spWrite(now, force) {
  if (!SP.on || !SP.els.length || (SP.frozen && !force)) return;
  const x = -((cpPos(now) - FOLLOW_FRAC * SP.d - SP.s) / SP.d);
  for (const { el, scale, gl } of SP.els) {
    if (gl) continue;
    const tf = `translateX(${cpSnap(x * scale)}px)`;
    if (el._sptf !== tf) el.style.transform = el._sptf = tf;
  }
}
function cpStop() {
  clearTimeout(CP.timer); CP.timer = 0; PH.r = NaN;   // P4-1b fix round: the value the animation gave is no longer the playhead's
  if (!CP.anim) return;
  const now = performance.now();
  cpEndSeg(now);
  /* #463: the cancel must not show the stale inline transform of the last frame path draw: write the animation's
     current position inline in the same task, else WebKit paints one frame at the position of a page ago. */
  if (FR.head[0] !== "none") $("#playheadOverlay").style.transform = `translateX(${cpSnap(S.theadW + tToX(cpPos(now), laneWidth()))}px)`;
  CP.anim.cancel(); CP.anim = null; CP.key = "";
  FR.head[0] = FR.head[1] = null;   // the inline style shows again: positionPlayhead writes it in this frame
}
/* The song time until which the view stands still at playhead `pos` in the current follow mode; null: it moves now.
   `show`: the playhead is inside the view. Mirrors followStep. */
function cpStill(pos) {
  const v = S.view, d = viewDur(), maxD = maxDur();
  if (S.followMode === "off" || S.followHold) return pos < v.t0 ? { until: v.t0, show: false } : pos <= v.t1 ? { until: v.t1, show: true } : { until: maxD, show: false };
  if (pos < v.t0 || pos > v.t1) return null;
  const kind = followKind();
  if (kind === "page") {
    const flip = v.t1 - d * 0.1;
    if (S.followMode === "smooth" && v.t1 >= maxD - 1e-9) return { until: maxD, show: true };   // P4-39b: the view stops at the end of the song
    return pos <= flip ? { until: flip, show: true } : null;
  }
  const anchor = v.t0 + FOLLOW_FRAC * d;
  if (v.t0 <= 0 && pos < anchor) return { until: anchor, show: true };       // left of the anchor at the start
  if (v.t1 >= maxD - 1e-9 && pos >= anchor) return { until: maxD, show: true };   // the view is at the end of the song
  return null;
}
/* In the tick: arm (or keep) the animation; false: this frame draws the playhead itself. */
function cpArm(now) {
  const pos = AC.drawn, still = AC.running && AC.pending === null && !AC.seeking && !window.__DS_NO_CP__ && cpStill(pos);
  if (!still) { cpStop(); return false; }
  const W = laneWidth(), key = `${S.view.t0},${S.view.t1},${W},${S.theadW},${AC.speed},${still.show}`, until = Math.min(still.until, maxDur());
  if (CP.anim && key === CP.key && CP.until === until && CP.rate === AC.speed && now - CP.at0 < CP.dur) {
    const e = pos - cpPos(now);   // #463: the model is ahead of the animation by e song s
    /* #463: slew, never re-anchor for drift. A model hard correction (the device position stall of an underrun
       catch-up) leaves the clock up to hundreds of ms from the model; re-anchoring here stepped the line by
       exactly that (gate10). The pr clamp bounds the slew, so any e stays within the step budget per frame.
       Bound (coordinator): a divergence the clamp cannot remove within 1 s of song time is a real discontinuity,
       not drift - re-anchor then (|e| > CP_REANCHOR_S, the 276 ms case of gate10). */
    if (Math.abs(e) > CP_REANCHOR_S) { cpStop(); }
    else if (Math.abs(e) > CP_TOL_S) { cpRate(e); return true; }
    else return true;   // still right: the animation keeps the playhead
  }
  const dur = Math.max(0, (until - pos) / AC.speed * 1000), el = $("#playheadOverlay");
  const durX = dur + Math.min(CP_GUARD_S, Math.max(0, maxDur() - until) / AC.speed) * 1000;   // #463: runs past `until`
  cpStop();
  if (still.show) {
    const x0 = cpSnap(S.theadW + tToX(pos, W)), x1 = cpSnap(S.theadW + tToX(Math.min(until + CP_GUARD_S * AC.speed, maxDur()), W));
    if (FR.head[0] !== "block") el.style.display = FR.head[0] = "block";
    CP.anim = el.animate([{ transform: `translateX(${x0}px)` }, { transform: `translateX(${x1}px)` }], { duration: durX, easing: "linear", fill: "forwards" });
    CP.anim.startTime = now;   // the page clock: performance.now and the document timeline share their origin
    Object.assign(CP, { at0: now, pos0: pos, rate: AC.speed, pr: 1, dur: durX, until, key });
    CP.segs.push({ at0: now, pos0: pos, rate: AC.speed, pr: 1, dur: durX, end: Infinity });
    if (CP.segs.length > CP_KEEP) CP.segs.splice(0, CP.segs.length - CP_KEEP);
    CP.arms++;
  } else if (FR.head[0] !== "none") el.style.display = FR.head[0] = "none";
  CP.timer = setTimeout(requestFrame, dur + 1);   // the tick that turns the page, starts the smooth glide or shows it
  return true;
}
/* The playhead the page shows at page time t (the self-test `play` and the tests): the animation that ran then, else
   the drawn ticks, linear between two of them. late: the time of a tick is its end (after the frame's drawing work). */
function drawnPlayheadAt(t, late) {
  for (let i = CP.segs.length - 1; i >= 0; i--) {
    const g = CP.segs[i];
    if (t >= g.at0 && t <= Math.min(g.end, g.at0 + g.dur)) return g.pos0 + (t - g.at0) / 1000 * g.rate * (g.pr || 1);
    if (g.at0 + g.dur < t - 60000) break;
  }
  const h = CP.hist, k = late ? 2 : 0;
  let lo = 0, hi = h.length - 1;
  if (!h.length || t < h[0][k] || t > h[hi][k]) return null;
  while (hi - lo > 1) { const m = (lo + hi) >> 1; if (h[m][k] <= t) lo = m; else hi = m; }
  const a = h[lo], b = h[hi];
  if (a[3] || b[3]) return null;   // a tick of the compositor mode: the animation, not the tick, drew the playhead
  return b[k] > a[k] ? a[1] + (b[1] - a[1]) * (t - a[k]) / (b[k] - a[k]) : a[1];
}
