/* Drum Spike v2 — follow strip (refactor plan step 5.2, R9 A2)
   The follow strip (SP): smooth follow draws a strip two views wide once and a compositor animation
   moves it; the fallback to the frame path; the test hook spFreeze. Moved verbatim from app.js.
   Plain script (no build step), loaded after velocity.js and before app.js. The names stay
   top-level bindings (page globals). Its one load-time statement registers spSyncView on the
   window input events (capture), before the CUR listeners of app.js, as in the old order. */
"use strict";

/* ---------------- follow strip (P4-1, review Q7) ----------------
   Smooth follow moves the view with the playhead. Before, every frame drew the ruler, the spike lanes, the hit lines
   and the GL wave lanes again (60 draws per second since P3-14d). Now the page draws a strip two views wide once
   (S.view [s, s + 2d] on twice the lane width, with the same draw functions, so a hit line sits on the same pixel
   as in a direct draw) and a compositor animation moves it (translateX, linear, from the transport model). The GL
   wave lanes of the strip come from the tile worker as two ImageBitmaps (one per view, op "strip"); the page shows
   them in two bitmaprenderer canvases (#lanesGLStrip) in the same task as the 2D canvases and the animations, so the
   wave and the hit lines move together (before, the GL wave was a frame behind the 2D lines: W4). The strips lie on
   a grid of half a view: strip k starts at s0 + k d/2, so the next one is asked for while the current one shows and
   is shown when the view's start reaches it (it would be good until s + d). Only the visible rows are in a strip.
   Fallback to the frame path (spLeave): a pause, a seek, a zoom or a pan (a VIEW frame), an edit or a new selection
   (a track redraw), a scroll, a resize, a mute (a GL repaint), a view under SP_MIN_S or at the sample level (the
   sample windows are not in a strip), the end of the song, a late or failed strip. After a fallback the GL canvas is
   drawn again; the strip stays (frozen) until that draw is shown. Not in a strip: the engine in the page (no
   worker), a page without the ImageBitmap transfer. Canvas 2D (no GL): the page draws the wave lanes into the strip. */
const SP = { on: false, s: 0, d: 0, key: "", req: null, els: [], anims: [], box: null, leaving: 0, leaveFrames: 0, timer: 0, retryAt: 0,
  n: { enter: 0, swap: 0, leave: 0, late: 0, failed: 0, ahead: 0 }, why: "" };
const SP_MIN_S = 1, SP_SAMPLES = 262144, SP_LEAD_S = 0.5, SP_AHEAD_S = 1, SP_SHOW_FRAMES = 60;   // P4-1b: lead and ahead in seconds of wall time
function spRows() {   // the rows that show in the lanes area now (a strip draws only these)
  const out = [];
  for (const n of S.order) {
    const row = rowOf(n), g = GEO.rows[n];
    if (row && g && row._live !== false && g.bottom - GEO.top >= 0 && g.top - GEO.top <= GEO.scH) out.push({ row, name: n });
  }
  return out;
}
function spKey() {   // what a strip depends on besides its time: a change makes it stale
  // P4-1b: spTick sets S.view = {t0, t0 + d}, and (t0 + d) - t0 can be d plus or minus an ulp; with that in the key the
  // strip left for "layout" at random and came back (a fallback draw and new stores). viewDur() gives the span without it.
  return [viewDur(), GEO.laneW, GEO.rulerW, GEO.dpr, GEO.top, GEO.scW, GEO.scH, GEO.waveX, GEO.waveH, rowHKey(), GEO.n, S.ampZoom, S.order.join("|"),
          [...document.querySelectorAll("#lanesScroll .track.muted")].map(r => r.dataset.name).join("|"), !!glEngine()].join(",");
}
/* P4-39b: can a glide strip show in this view at all? The static conditions only (the transient ones stay in spCan):
   the view is at least SP_MIN_S, no track is at the sample level, the engine has a strip (GL) or the page has the peak
   arrays (Canvas 2D). */
function glideStatic() {
  const d = viewDur();
  if (d < SP_MIN_S) return false;
  for (const n of S.order) { const p = S.peaks[n]; if (p && p.sr && d * p.sr <= SP_SAMPLES) return false; }   // the sample level
  const eng = glEngine();
  if (eng) return !!eng.strip;
  return S.order.every(n => !S.peaks[n] || !S.peaks[n].tiles);   // Canvas 2D: the page needs the peak arrays
}
/* P4-39b: what the follow does in this view: "smooth", "page" or "off". The chosen mode "smooth" behaves like "page"
   (the view stands still, the compositor moves the playhead, one flip to FOLLOW_FRAC) where no glide can show; before,
   every frame was a full VIEW redraw there (a 2 s view: the owner's stutter and flicker). */
function followKind() {
  if (S.followMode === "off" || S.followHold) return "off";
  if (S.followMode === "page") return "page";
  return glideStatic() ? "smooth" : "page";
}
function spCan(pos, ahead) {   // the strip may show at playhead pos (ahead: pos is the anchor where the glide starts)
  if (window.__DS_NO_STRIP__ || followKind() !== "smooth" || !GEO.ok || hitDrag) return false;
  if (!AC.running || AC.pending !== null || AC.seeking) return false;
  const v = S.view, d = viewDur();
  if (pos < v.t0 || pos > v.t1 || (!ahead && cpStill(pos))) return false;
  if (pos - FOLLOW_FRAC * d + d >= maxDur()) return false;   // the view reaches the end of the song and stops there
  if (!glideStatic()) return false;   // P4-39b: under SP_MIN_S, the sample level, no strip in the engine
  const eng = glEngine();
  return eng ? !!(eng.ready && !eng.lost) : true;
}
function spLanes(view) {   // the GL lanes of the visible rows for one view (glDraw's places and culling)
  const dpr = GEO.dpr, out = [];
  for (const n of S.order) {
    const row = rowOf(n), g = GEO.rows[n];
    if (!row || !g || !S.peaks[n] || !S.tracks[n]) continue;
    const y = g.wave - GEO.top, H = g.waveH || GEO.waveH;
    if (y + H < 0 || y > GEO.scH) continue;
    const l = glLane(row.querySelector(".wave"), n, view);
    l.y = Math.round(y * dpr);   // #435: the strip draws each view on its own laneW-wide tile, so the lane sits at x 0 (glDraw's shared canvas places it at the scroll's waveX)
    out.push(l);
  }
  return out;
}
function spDrop() {   // P4-1b fix: drop the strip request; its bitmaps (if they came) are closed, an answer still on its way too
  const r = SP.req;
  SP.req = null;
  if (r && r.bitmaps) { r.bitmaps.forEach(b => b.close()); r.bitmaps = null; }
}
function spRequest(s) {   // ask the worker for the GL part of the strip from s (Canvas 2D: nothing to ask)
  spDrop();
  const d = viewDur(), eng = glEngine(), req = SP.req = { s, d, key: spKey(), bitmaps: null, done: !eng, failed: false };
  if (!eng) return;
  eng.size(Math.max(1, Math.round(GEO.laneW * GEO.dpr)), Math.max(1, Math.round(GEO.scH * GEO.dpr)));   // #435: the strip's tiles are the lane's own canvas, laneW wide, not the scroll's — tiles sized to the scroll's clientWidth left a laneW - scW dark gap column between them (the clientWidth is smaller by the scrollbar)
  // keep: no strip shows, so #lanesGL shows now or will show before this strip does (after a fallback spLeftCheck shows
  // it once the fallback draw is there, maybe before this answer); the worker draws its lanes again after taking the parts
  // off its canvas. Under a strip it lets the canvas go (1 x 1) until the strip ends. P4-1b fix: was "#lanesGL visible
  // now", which was false in a re-entry before spLeftCheck: the worker released the canvas and spLeftCheck showed it blank.
  const keep = !SP.on;
  eng.strip([spLanes({ t0: s, t1: s + d }), spLanes({ t0: s + d, t1: s + 2 * d })], keep).then(r => {
    if (SP.req !== req) { if (r.bitmaps) r.bitmaps.forEach(b => b.close()); return; }
    if (r.bitmaps) { req.bitmaps = r.bitmaps; req.done = true; } else { req.failed = true; SP.n.failed++; SP.why = r.error; }
    if (S.playing) requestFrame();
  });
}
function spBox() {
  if (SP.box) return SP.box;
  const box = document.createElement("div"), inner = document.createElement("div");
  box.id = "lanesGLStrip"; box.appendChild(inner);
  for (let i = 0; i < 2; i++) inner.appendChild(document.createElement("canvas"));
  $("#editor").insertBefore(box, $("#lanesScroll"));
  return (SP.box = box);
}
/* Show strip req: draw its 2D part (two views on twice the width), show its bitmaps, arm the animations. In a tick. */
function spApply(req, now) {
  const W = GEO.laneW, RW = GEO.rulerW, d = req.d, s = req.s, dpr = GEO.dpr, rows = spRows();
  const keep = [S.view, GEO.laneW, GEO.rulerW];
  cpStop();   // P4-1b fix round: an entry from spAhead comes straight from the compositor playhead (no frame path stopped it)
  for (const a of SP.anims) a.cancel();   // spApply redraws in this task: no paint sees the canceled state
  SP.anims = [];
  try {
    SP.drawing = true;
    S.view = { t0: s, t1: s + 2 * d }; GEO.laneW = 2 * W; GEO.rulerW = 2 * RW;   // the draw functions draw the strip
    for (const { row, name } of rows) {
      const sp = row.querySelector(".spike"), wv = row.querySelector(".wave");
      sizeLaneCanvas(sp, true); sizeLaneCanvas(wv, true);
      drawSpike(sp, name);
      const vl = VL.on ? row.querySelector(".vel") : null;   // #435: the vel lane is a canvas.lane too: it rides the strip's animation, so it must cover both views (before, it kept its one-view draw: the stalks ended at the seam with the scale text there and nothing right of it)
      if (vl && S.tracks[name]) { sizeLaneCanvas(vl, true); drawVelLane(vl, name); }
      if (req.bitmaps) { wv.getContext("2d").clearRect(0, 0, 2 * W, laneSize(wv)[1]); drawEventLines(wv, name); }
      else drawWave(wv, name);
    }
    const rul = $("#ruler");
    rul.style.width = 2 * RW + "px"; rul.width = 2 * RW * dpr; rul.height = 22 * dpr; rul.getContext("2d").setTransform(dpr, 0, 0, dpr, 0, 0);
    sizeMapCanvases();   // P4-8: the map rulers are strips too (drawRuler draws them)
    drawRuler();
  } finally { [S.view, GEO.laneW, GEO.rulerW] = keep; SP.drawing = false; }
  FR.sized = "";   // the canvases are not at the sizes of GEO now: the fallback sizes them again
  SP.els = []; SP.clips = [];
  const clip = (el, w) => { const v = `polygon(0 0, ${w}px 0, ${w}px 100%, 0 100%)`; if (el.style.clipPath !== v) el.style.clipPath = v; SP.clips.push(el); };
  for (const { row } of rows) {   // the lane box is a little wider than the lane: show the strip on the lane's width only
    for (const c of row.querySelectorAll("canvas.lane")) SP.els.push({ el: c, scale: W });
    clip(row.querySelector(".tlanes"), W);
  }
  SP.els.push({ el: $("#ruler"), scale: RW });
  clip($("#rulerClip"), RW);
  for (const k of TM_KINDS) if (TM.cans && TM.vis[k]) { SP.els.push({ el: TM.cans[k], scale: RW }); clip(TM.cans[k].parentNode, RW); }   // P4-8
  if (req.bitmaps) {
    const box = spBox(), inner = box.firstChild;
    Object.assign(box.style, { left: GEO.scL + GEO.waveX + "px", top: GEO.scT + "px", width: W + "px", height: GEO.scH + "px", display: "block" });
    [...inner.children].forEach((c, i) => {
      Object.assign(c.style, { left: i * W + "px", width: W + "px", height: GEO.scH + "px" });   // #435: each tile is the lane's own W-wide canvas, so the tiles sit W apart with no gap (they were scW wide at W apart: a laneW - scW dark column slid through the lane)
      c.getContext("bitmaprenderer").transferFromImageBitmap(req.bitmaps[i]);
    });
    req.bitmaps = null;
    if (GLQ.eng && GLQ.eng.canvas.style.visibility !== "hidden") {   // under the strip; drawn again when the strip ends
      GLQ.eng.canvas.style.visibility = "hidden";
      if (GLQ.eng.release) GLQ.eng.release();   // P4-1b: its buffers go while it is hidden (the footprint, item 2)
    }
    SP.leaving = 0;
    SP.els.push({ el: inner, scale: W, gl: true });
  }
  SP.s = s; SP.d = d; SP.key = req.key; SP.req = null; SP.geo = spGeo();
  spArm(now);   // P4-1b: the next strip of the grid is asked for SP_LEAD_S before it is due (spTick), not now: two strips
}               // of bitmaps were held for half a view each time
/* The strip from the transport model (the same model as the compositor playhead). The 2D els are written per frame
   (spWrite, from the rAF loop); only the GL strip keeps a compositor animation. */
function spArm(now) {
  const d = SP.d, s = SP.s, pos = AC.drawn, rate = AC.speed, t0 = pos - FOLLOW_FRAC * d;
  const dur = Math.max(0, (s + d - t0) / rate * 1000);   // the strip is good until the view's start reaches s + d
  for (const a of SP.anims) a.cancel();
  SP.anims = SP.els.filter(({ gl }) => gl).map(({ el, scale, gl }) => {
    const a = el.animate([{ transform: `translateX(${-(t0 - s) / d * scale}px)` }, { transform: `translateX(${-scale}px)` }], { duration: dur, easing: "linear", fill: "forwards" });
    a.startTime = now; a._gl = !!gl;
    /* #463: the model time the keyframe 0 shows (the playhead's position at arm) and the model span the keyframes
       cover. spFreeze maps a model time to currentTime through them: CP.at0 moves on when cpRate rebases the clock,
       the animation's axis does not. */
    a._pos0 = pos; a._span = s + d - t0;
    return a;
  });
  /* #463: the swap must not step the drawn playhead. spArm re-anchored the shared clock from AC.drawn; after a
     device position correction (the stall of an underrun catch-up) the model and the clock are apart (measured
     276 ms in the probe), and the re-anchor stepped the line by exactly that (gate10: nv jumps of 81-133 ms
     tagged 'compositor' at the swap). While the playhead animation runs at an unchanged speed, rebase the live
     seg in place instead (cpRate): the position stays continuous and the pr slews the clock to the model. A
     fresh anchor only without an animation (the frame path drew until now) or at a speed change. The seg covers
     the strip's span, so cpPos cannot clamp before the next swap. */
  if (CP.anim && CP.rate === rate) {
    const e = pos - cpPos(now);
    if (Math.abs(e) > CP_TOL_S) cpRate(e);
    const g = CP.segs[CP.segs.length - 1];
    if (g && g.end === Infinity) { g.dur = Math.max(g.dur, Math.max(0, (SP.s + d - cpPos(now)) / rate * 1000)); CP.dur = g.dur; }
  } else {
    cpEndSeg(now);
    Object.assign(CP, { at0: now, pos0: pos, rate, pr: 1, dur });
    CP.segs.push({ at0: now, pos0: pos, rate, pr: 1, dur, end: Infinity });
  }
  if (CP.segs.length > CP_KEEP) CP.segs.splice(0, CP.segs.length - CP_KEEP);
  spWrite(now);   // the 2D els show the strip in this task, before the redraw is painted
  spTimer(now);
}
function spLead(d) { return Math.min(d / 4, SP_LEAD_S * AC.speed); }   // song time before a swap at which the next strip is asked for
function spTimer(now) {   // the tick that asks for the next strip, or the one that swaps it in (the view's start reaches it)
  const d = SP.d, t0 = cpPos(now) - FOLLOW_FRAC * d, due = SP.s + d / 2 - (SP.req ? 0 : spLead(d));
  clearTimeout(SP.timer);
  SP.timer = setTimeout(requestFrame, Math.max(0, (due - t0) / AC.speed * 1000) + 1);
}
/* In the tick while a strip shows: the view of the strip now, the next strip, or the fallback. False: it fell back. */
function spTick(now) {
  const pos = AC.drawn;
  if (SP.frozen) return true;   // spFreeze (tests)
  if (!spCan(pos)) { spLeave("state"); return false; }
  if (spKey() !== SP.key) { spLeave("layout"); return false; }
  const d = SP.d, t0 = pos - FOLLOW_FRAC * d, next = SP.req;
  S.view = { t0, t1: t0 + d };   // the view the strip shows now; nothing is drawn (input, the overview and the counters use it)
  if (next && next.failed) { spLeave("failed"); return false; }
  if (t0 >= SP.s + d / 2 && next && next.done && next.s === SP.s + d / 2 && next.key === SP.key) { spApply(next, now); SP.n.swap++; }
  else if (t0 >= SP.s + d * 0.9) { SP.n.late++; spLeave("late"); return false; }
  else {
    if (!next && t0 >= SP.s + d / 2 - spLead(d)) { spRequest(SP.s + d / 2); spTimer(now); }
    const e = pos - cpPos(now);   // #463: slew the strip animations through their playback rate; re-arm only beyond CP_HARD_S
    if (CP.rate !== AC.speed || Math.abs(e) > CP_REANCHOR_S) spArm(now);   // the bound: a real discontinuity re-anchors
    else if (Math.abs(e) > CP_TOL_S) cpRate(e);
  }
  return true;
}
/* After a frame of the frame path (the view glides): ask for the first strip, or show it when it is here.
   only: show a strip that is here and fits (no request, no fallback); the tick calls it before the follow step, so a
   strip asked for ahead (spAhead) shows at the first tick of the glide, without a draw of the frame path. */
function spEnter(now, only) {
  const pos = AC.drawn;
  if (!spCan(pos) || now < SP.retryAt) { if (!only && !SP.on) spDrop(); return false; }
  const r = SP.req, d = viewDur(), t0 = S.view.t0;
  const fits = r && !r.failed && Math.abs(r.d - d) <= 1e-9 * d && r.key === spKey() && t0 >= r.s && t0 < r.s + d * 0.9;
  if (only) { if (!fits || !r.done) return false; }
  else if (r && r.failed) { spDrop(); SP.retryAt = now + 2000; return false; }
  else if (!fits) { spRequest(t0); return false; }
  else if (!r.done) return false;
  SP.on = true; SP.n.enter++; if (r.ahead) SP.n.ahead++;
  spApply(r, now);
  return true;
}
/* P4-1b: while the view stands still at the start of the song (smooth follow, the playhead left of its anchor), ask
   for the first strip SP_AHEAD_S before the glide starts. The glide then starts in the strip: no frames of the frame
   path (each drew every lane and kept the GL canvas's surfaces busy just before the strip took two more). */
function spAhead(now) {
  if (SP.on || S.followMode !== "smooth" || S.followHold || now < SP.retryAt || window.__DS_NO_AHEAD__) return;   // the flag: tests (the P4-1 entry)
  const pos = AC.drawn, v = S.view, d = viewDur(), still = cpStill(pos), anchor = v.t0 + FOLLOW_FRAC * d;
  if (!still || !still.show || v.t0 > 0 || still.until !== anchor || (anchor - pos) / AC.speed > SP_AHEAD_S) return;
  if (!spCan(anchor, true)) return;
  const r = SP.req;
  if (r && (r.failed || (Math.abs(r.d - d) <= 1e-9 * d && r.key === spKey() && r.s === v.t0))) return;
  spRequest(v.t0);
  SP.req.ahead = true;
}
function spGlFreeze() {   // P4-2: the GL strip that moved on after a leave stops where it is (the view stopped or jumped)
  if (!SP.glAnim) return;
  try { SP.glAnim.commitStyles(); } catch (e) { /* not in every engine */ }
  SP.glAnim.cancel(); SP.glAnim = null;
}
function spLeave(why) {
  spDrop();
  if (why === "seek" || why === "pause") spGlFreeze();   // P4-2 fix round: also after the strip left (a moving fallback)
  if (!SP.on) return;
  // P4-2 (P4-1b verifier minor 1): when the glide goes on over the same lanes (a mute or a solo, a late strip; not a seek, a
  // pause, a zoom, a scroll or a resize), the GL strip keeps moving with the transport model until #lanesGL shows the frame
  // path's draw. Frozen, it stood 2 to 4 frames while the hit lines moved on, and the wave then jumped 12 to 17 px.
  const glides = S.playing && AC.running && !AC.seeking && S.followMode === "smooth" && !S.followHold && !SP.frozen && why !== "seek" && why !== "pause" && SP.geo === spGeo()
    && AC.drawn - FOLLOW_FRAC * viewDur() + viewDur() < maxDur();   // P4-2 fix round: not where the view stops at the song end (followStep's clamp)
  SP.on = false; SP.frozen = false; SP.n.leave++; SP.why = why; PH.r = NaN;   // P4-1b fix round: PH.r as in cpStop
  clearTimeout(SP.timer);
  if (SP.glAnim) { SP.glAnim.cancel(); SP.glAnim = null; }
  for (const a of SP.anims) {   // #463: every animation commits its shown transform inline before the cancel, else the
    // canvas shows the stale inline transform of a page ago for a frame until the fallback draw lands (the lanes flash);
    // sizeLaneCanvas clears it again when a canvas is sized for a draw
    try { a.commitStyles(); } catch (e) { /* not in every engine, and not for a finished animation without fill */ }
    if (a._gl && glides) { SP.glAnim = a; continue; }
    a.cancel();
  }
  SP.anims = []; SP.els = [];
  for (const el of SP.clips || []) el.style.clipPath = "";
  SP.clips = [];
  cpEndSeg(performance.now());
  const eng = GLQ.eng;
  if (SP.box && SP.box.style.display === "block") { SP.leaving = eng && !eng.lost ? eng.stats.posted : -1; SP.leaveFrames = 0; SP.posted = null; }
  invalidate(F.SIZE | F.VIEW);   // the canvases back to one view, and everything drawn again (the GL canvas too)
}
function spGeo() { return [GEO.n, GEO.top, GEO.laneW, GEO.scW, GEO.scH, GEO.waveH, rowHKey(), GEO.dpr, viewDur()].join(","); }   // a glide over the same lanes
function glSized() {   // #lanesGL has the size of the lanes area again (where the placeholder takes the worker's frame size)
  const eng = GLQ.eng;
  return !eng || eng.lost || !GLQ.dims || !eng.size_ || (eng.canvas.width === eng.size_[0] && eng.canvas.height === eng.size_[1]);
}
function spLeftCheck() {   // in the frame after a fallback: hide the frozen GL strip once the GL canvas showed a new draw
  const eng = GLQ.eng, posted = SP.posted || (SP.posted = []);
  posted.push(eng ? eng.stats.posted : 0);   // P4-2: the draws posted until each frame of the fallback (below)
  if (SP.glAnim && S.view.t1 >= maxDur() - 1e-9) spGlFreeze();   // P4-2 fix round: the view reached the song end and stops
  if (SP.leaving > 0 && eng && !eng.lost && eng.stats.drawn <= SP.leaving) return;
  // P4-1b fix round: under the strip the worker's canvas was 1 x 1 (released). The placeholder (#lanesGL) shows a frame of
  // the worker some frames after its drawn answer when the worker is behind (Chromium: up to 9 frames, measured), so it
  // could still show the 1 x 1 frame: blank lanes. Where the placeholder takes the size of the frames it shows (GLQ.dims),
  // wait until it has the size of the lanes area again (glSized).
  // P4-2 (P4-1b verifier minor 2): no limit any more. After SP_SHOW_FRAMES frames the page showed #lanesGL even at 1 x 1
  // (a worker more than 1 s behind: blank lanes); now it asks for the draw again every SP_SHOW_FRAMES frames and waits on
  // (the frozen strip shows meanwhile).
  if (!glSized()) { if (++SP.leaveFrames % SP_SHOW_FRAMES === 0) glRepaint(); requestFrame(); return; }
  // P4-2: while the GL strip moves on (a glide), #lanesGL takes over only with a draw at most two frames old: the first
  // draw after the leave can be many frames old when the worker was behind (measured in Chromium: 19 px behind the view)
  const anim = SP.glAnim && SP.glAnim.playState === "running";
  if (anim && eng && !eng.lost && posted.length > 2 && eng.stats.drawn < posted[posted.length - 3]) { requestFrame(); return; }
  if (++SP.leaveFrames < 2) { requestFrame(); return; }   // the drawn answer comes before the canvas shows it
  SP.leaving = 0; SP.posted = null;
  const box = SP.box;
  if (SP.glAnim) { SP.glAnim.cancel(); SP.glAnim = null; }
  box.style.display = "none"; box.firstChild.style.transform = "";
  for (const c of box.firstChild.children) c.getContext("bitmaprenderer").transferFromImageBitmap(null);
  if (GLQ.eng) GLQ.eng.canvas.style.visibility = "";
}
/* Test hook: hold the strip as it is at page time t (the tests compare its pixels with a direct draw of that view);
   null goes on from the model. */
function spFreeze(t) {
  if (!SP.on) return false;
  if (t === null) { SP.frozen = false; spArm(performance.now()); return true; }
  SP.frozen = true; clearTimeout(SP.timer);
  /* #463: the GL strip's compositor animation keeps its own time axis (startTime at arm); CP.at0 moves on with every
     cpRate rebase, so t - CP.at0 is wrong. The keyframes map the strip's model span linearly: keyframe 0 at the arm's
     playhead position (a._pos0), keyframe 1 one view d later - the fraction follows the model position, not the clock. */
  for (const a of SP.anims) {
    const dur = a.effect.getTiming().duration;
    a.pause();
    a.currentTime = Math.max(0, Math.min(dur, (cpPos(t) - a._pos0) / a._span * dur));
  }
  spWrite(t, true);   // the 2D els hold their place at t from now on (no frame writes while frozen)
  return true;
}
function spSyncView() {   // input while a strip shows: the view of this moment (the ticks set it only about every 40 ms)
  if (!SP.on) return;
  const d = SP.d, t0 = cpPos(performance.now()) - FOLLOW_FRAC * d;
  S.view = { t0, t1: t0 + d };
}
for (const t of ["pointerdown", "mousedown", "mousemove", "wheel", "click", "dblclick", "contextmenu", "keydown"])
  window.addEventListener(t, spSyncView, { capture: true, passive: true });
