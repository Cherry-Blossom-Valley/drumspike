/* Drum Spike v2 — wave lanes (refactor plan step 5.2, R9 A2)
   The page side of the WebGL2 lanes (GLQ, the glue to static/lanes-gl.js), drawWave (the Canvas 2D
   path), drawSpike, the hit search (hitsIn, hitDrag), drawEventLines and the samples cache (SF).
   Moved verbatim from app.js. Plain script (no build step), loaded after markers.js and before
   app.js. The names stay top-level bindings (page globals). APP_V stays in app.js: it reads the
   ?v= of app.js's own script tag at load, and that tag is not in the DOM while this file runs. */
"use strict";

function pickLevel(p, spp) {
  /* finest pyramid level whose bucket is not coarser than one pixel */
  const levels = Object.keys(p.levels).map(Number).sort((a,b)=>a-b);
  let best = null;
  for (const L of levels) if (L <= spp) best = L;
  return best;
}

/* ---------------- WebGL2 lanes (P3-6a, static/lanes-gl.js) ----------------
   One WebGL2 canvas (#lanesGL) lies under #lanesScroll and draws every visible wave lane at its place, one
   draw call per lane. The wave canvases above it are transparent: drawWave queues a lane (glPush),
   redrawAll draws the queue in one pass (glFlush), and the 2D side keeps the event lines. A scroll, a
   layout change (rows moved, resized, muted) repaints the GL canvas only (glRepaint). No WebGL2, a lost
   context or window.__DS_NO_GL__ / ?gl=0 leave the Canvas 2D path in drawWave. */
/* P4-26: the filled waveform (Pro Tools look): from a visible span of WAVE_FILL_S the peaks are drawn as one solid min/max
   shape in the track colour (the same peaks, no extra pass: the RMS body is not drawn); below it the look stays as it was
   (the sample window as bars or a line, the translucent envelope with the RMS body where no window is here yet). */
let WAVE_FILL_S = 0.5;
function waveFill(dur) { return dur >= WAVE_FILL_S; }
/* P4-26: GLQ.pend counts the sample window fetches on their way per lane, GLQ.exact says a lane showed its sample window in the
   last draw. A lane that showed the exact wave and waits for the window of a new view keeps the frame on screen
   (the draw is skipped) instead of showing the coarse envelope (bins of 256 samples, visible as blocks) for the frames
   between the jump and the window: with Tab held, every jump out of the cached window flashed those blocks.
   GLQ.noHold = true restores the old behaviour (the planted-fault test). */
const GLQ = { pend: {}, exact: {}, held: 0, noHold: false, noGen: false, eng: null, tried: false, off: false, all: false, q: [], warmed: new WeakSet(), box: "", fetched: {}, loadedSeq: 0, dims: false };
/* P4-1b fix round: does the placeholder (#lanesGL) take the size of the worker's frames (Chromium does)? Then spLeftCheck can
   see when it shows a full frame again. Seen once is enough; an engine that keeps the placeholder's own size never sets it. */
function glDims() {
  const e = GLQ.eng;
  if (!GLQ.dims && e && e.size_ && e.size_[0] > 1 && e.size_[0] !== 300 && e.canvas.width === e.size_[0] && e.canvas.height === e.size_[1]) GLQ.dims = true;
}
/* P3-6b: in the app the engine runs in a worker (tiles-worker.js) and the page holds no peak arrays (only the
   coarsest level for the overview) and no sample windows: tiles and windows move to the engine as bytes.
   Without a worker with WebGL2 the same engine runs in the page (LanesGL.create), still with raw tiles. The server
   keeps the arrays and the PCM of the page. */
function glMount(eng) {   // P3-14d: a scroll or a layout change repaints through the frame (GEO, invalidate(F.GL))
  const cv = eng.canvas, sc = $("#lanesScroll");
  if (GLQ.eng && GLQ.eng.canvas.parentNode) GLQ.eng.canvas.remove();
  cv.id = "lanesGL"; $("#editor").insertBefore(cv, sc);
  GLQ.eng = eng; GLQ.box = "";
}
function glEvict(key) { delete S.samplesCache[key]; invalidate(F.GL); }   // the engine dropped an old window: fetch it again when the lane needs it
function glEngine() {
  if (!GLQ.tried) {
    GLQ.tried = true;
    const no = window.__DS_NO_GL__ || /[?&]gl=0(&|$)/.test(location.search);
    const worker = !window.__DS_NO_WORKER__ && !/[?&]worker=0(&|$)/.test(location.search);
    const eng = no || !window.LanesGL ? null
      : (worker && LanesGL.remote({ url: "/tiles-worker.js?v=" + APP_V, onLost: why => glRemoteLost(why), onEvict: glEvict, onDrawn: () => { glDims(); if (SP.leaving) requestFrame(); } }))
        || LanesGL.create({ onLost: () => glDrop(), onEvict: glEvict });
    if (eng) glMount(eng);
  }
  return GLQ.off ? null : GLQ.eng;
}
function glRemoteLost(why) {   // the worker engine failed: before its first answer, try the engine in the page; later, Canvas 2D
  const eng = GLQ.eng;
  if (eng && eng.ready) { glDrop(); return; }
  const local = window.LanesGL && LanesGL.create({ onLost: () => glDrop(), onEvict: glEvict });
  if (!local) { glDrop(); return; }
  glMount(local);
  glFeed(local);
  invalidate(F.GL);
}
function glDrop() {   // context lost or a draw failed: back to Canvas 2D for good
  if (GLQ.eng) { GLQ.eng.canvas.remove(); GLQ.eng.close(); }
  GLQ.eng = null; GLQ.q = [];
  invalidate(F.VIEW);
}
/* the arrays of a track as the bytes of peaks_tile answers (u32 level, start, n, then f32 min, max, rms) */
function packTiles(p) {
  const raw = {};
  for (const [lv, L] of Object.entries(p.levels)) {
    const n = L.n, buf = new ArrayBuffer(12 + 12 * n), f = new Float32Array(buf, 12);
    new Uint32Array(buf, 0, 3).set([+lv, 0, n]);
    f.set(L.min, 0); f.set(L.max, n); f.set(L.rms || new Float32Array(n), 2 * n);
    raw[lv] = buf;
  }
  return raw;
}
/* the tiles of every track to the engine (again): after the engine in the page took over from a failed worker */
function glFeed(eng) {
  const sid = S.sid;
  for (const n of S.order) {
    if (!S.peaks[n] || !S.peaks[n].tiles || !B.peaksRaw) continue;
    B.peaksRaw(sid, n).then(raw => { if (S.sid === sid && GLQ.eng === eng) eng.addTiles(n, raw.raw); }).catch(() => null);
  }
}
/* P3-6b: the arrays of a track for the Canvas 2D path (a track in app mode keeps only the coarsest level) */
function ensureArrays(name) {
  const p = S.peaks[name];
  if (!p || !p.tiles) return Promise.resolve();
  if (!p.loading) {
    const sid = S.sid;
    p.loading = B.peaks(sid, name).then(full => { if (S.sid === sid && S.peaks[name] === p) { S.peaks[name] = full; invalidate(F.VIEW); } }, () => { p.loading = null; });
  }
  return p.loading;
}
async function setLanesGL(on) {   // test hook: switch the renderer
  GLQ.off = !on; GLQ.fetched = {};
  if (GLQ.eng) GLQ.eng.canvas.style.display = on ? "" : "none";
  if (!on) await Promise.all(S.order.map(ensureArrays));
  redrawAll();
  return !!glEngine();
}
function glPush(canvas, name) {
  if (!glEngine() || !S.peaks[name] || !S.tracks[name]) return false;
  GLQ.q.push({ canvas, name });
  return true;
}
function glLane(canvas, name, view) {
  // P4-2 (P4-1b verifier minor 3): the span from spanOf (to the nanosecond, as viewDur), not t1 * sr - span0, which kept the
  // float noise of the view
  const p = S.peaks[name], d = spanOf(view), span0 = view.t0 * p.sr, spanS = d * p.sr, c = S.samplesCache[name];
  const have = spanS <= 262144 && c && c.start <= span0 && c.end >= span0 + spanS && !!c.data === !B.peaksRaw;   // the app keeps the window in the engine, not in the page
  const row = canvas.closest(".track");
  const [W, H] = laneSize(canvas), dpr = GEO.dpr;   // not canvas.width: a far lane holds a 1 x 1 canvas; P3-14d: no layout read
  return { key: name, W, H, dw: Math.floor(W * dpr), dh: Math.floor(H * dpr), x: 0, y: 0,
           fade: row && row.classList.contains("muted") ? 0.35 : 1,   // the .tlanes opacity of a muted track
           fill: waveFill(d),   // P4-26: the solid min/max shape from a visible span of WAVE_FILL_S
           color: S.tracks[name].color, peaks: B.peaksRaw ? undefined : p, amp: S.peakAmp[name] || 1, gain: S.ampZoom, span0, spanS,
           samples: have ? (c.data ? { start: c.start, data: c.data } : { start: c.start, end: c.end }) : null, want: spanS <= 262144 && !have };
}
/* draw the lanes of items [{canvas, name}] at their places; false when WebGL2 is not available */
function glDraw(items) {   // P3-14d: places from GEO; the GL canvas style is written only when its box changed
  const eng = glEngine();
  if (!eng) return null;
  const cv = eng.canvas, dpr = GEO.dpr, view = S.view, cw = GEO.scW, chh = GEO.scH, box = `${GEO.scL},${GEO.scT},${cw},${chh}`;
  if (GLQ.box !== box) {
    GLQ.box = box;
    cv.style.left = GEO.scL + "px"; cv.style.top = GEO.scT + "px"; cv.style.width = cw + "px"; cv.style.height = chh + "px";
  }
  eng.size(Math.max(1, Math.round(cw * dpr)), Math.max(1, Math.round(chh * dpr)));
  const lanes = [], x = GEO.waveX, W = GEO.laneW;
  for (const it of items) {
    const g = GEO.rows[it.name];
    if (!g) continue;   // not measured yet: the observer repaints when it is
    const y = g.wave - GEO.top, H = g.waveH || GEO.waveH;
    if (!GLQ.all && (y + H < 0 || y > chh || x + W < 0 || x > cw)) continue;   // scrolled out of view
    const l = glLane(it.canvas, it.name, view);
    l.x = Math.round(x * dpr); l.y = Math.round(y * dpr);
    l.item = it; lanes.push(l);
  }
  eng.prune(S.order);
  if (B.peaksRaw) for (const it of items) { const p = S.peaks[it.name]; if (p && !p.tiles && !eng.hasTiles(it.name)) eng.addTiles(it.name, packTiles(p)); }   // arrays of the Canvas 2D period go back to the engine
  for (const l of lanes) {
    if (!l.want || GLQ.fetched[l.key] === view) continue;   // the sample window is not here yet
    GLQ.fetched[l.key] = view;
    GLQ.pend[l.key] = (GLQ.pend[l.key] || 0) + 1;
    ensureSamples(l.key, Math.max(0, Math.floor(l.span0)), Math.ceil(l.span0 + l.spanS)).catch(() => null).then(() => {
      GLQ.pend[l.key]--;
      if (S.view === view && glEngine()) invalidate(F.GL);   // the window is here: draw the exact wave
    });
  }
  if (!GLQ.noHold && lanes.some(l => l.want && GLQ.pend[l.key] > 0 && GLQ.exact[l.key])) { GLQ.held++; return lanes; }   // P4-26: keep the exact frame until the window is here
  if (!eng.draw(lanes)) return null;
  for (const l of lanes) GLQ.exact[l.key] = !!l.samples;
  if (GLQ.loadedSeq < 0 && lanes.length) GLQ.loadedSeq = eng.stats.posted;   // the first draw with lanes after a load: the self-test waits for its answer
  return lanes;
}
function glItems() {   // every lane that WebGL2 draws, from the DOM
  const out = [];
  for (const name of S.order) {
    const row = rowOf(name);
    if (row && S.peaks[name] && S.tracks[name]) out.push({ canvas: row.querySelector(".wave"), name });
  }
  return out;
}
function glRepaint() {   // the lanes moved or changed on screen, not the data
  if (!glEngine()) return;
  try { glDraw(glItems()); } catch (e) { glDrop(); }
}
function glFlush() {   // the end of drawAll: draw the queued lanes, then the 2D parts
  const items = GLQ.q; GLQ.q = [];
  if (!glEngine()) return;
  let lanes;
  try { lanes = glDraw(items); } catch (e) { lanes = null; }
  if (!lanes) { glDrop(); return; }   // the lanes fall back to Canvas 2D
  for (const it of items) {
    if (it.canvas.closest(".track")._live === false) continue;   // a lane without a canvas store has no event lines to draw
    const c2 = it.canvas.getContext("2d"), [W, H] = laneSize(it.canvas);
    c2.clearRect(0, 0, W, H);
    drawEventLines(it.canvas, it.name);
    const p = S.peaks[it.name];
    if (!B.peaksRaw && !GLQ.warmed.has(p)) { GLQ.warmed.add(p); GLQ.eng.warm([{ key: it.name, peaks: p }]); }
  }
}
/* The pixels of a lane as the user sees it (the GL lane under the 2D canvas); tests and the self-test read this,
   because the browser clears a WebGL canvas after it showed it. Returns a promise of ImageData. */
async function laneImage(name) {
  const c = document.querySelector(`.track[data-name="${name}"] canvas.wave`), tmp = document.createElement("canvas");
  const dpr0 = window.devicePixelRatio || 1;
  tmp.width = Math.floor(c.clientWidth * dpr0); tmp.height = Math.floor(c.clientHeight * dpr0);
  const t = tmp.getContext("2d"), eng = glEngine();
  if (eng && S.peaks[name]) {
    const l = glLane(c, name, S.view), sc = $("#lanesScroll"), dpr = window.devicePixelRatio || 1;
    eng.size(Math.max(1, Math.round(sc.clientWidth * dpr)), Math.max(1, Math.round(sc.clientHeight * dpr)));
    const im = await eng.readLane(l);
    if (im) t.putImageData(new ImageData(im.data, im.width, im.height), 0, 0);
    GLQ.box = ""; glRepaint();   // the read drew the lane at the corner: put the canvas back
  }
  if (c.width > 1) t.drawImage(c, 0, 0);
  const out = t.getImageData(0, 0, tmp.width, tmp.height);
  tmp.width = 0; tmp.height = 0;
  return out;
}

function drawWave(canvas, name) {
  if (glPush(canvas, name)) return;   // P3-6a: WebGL2 draws this lane
  const ctx = canvas.getContext("2d"), [W, H] = laneSize(canvas);
  const t = S.tracks[name], p = S.peaks[name];
  const gen = canvas._gen = (canvas._gen || 0) + 1;   // P4-26: a draw that waits for its samples is dropped when a newer draw of this lane came meanwhile
  ctx.clearRect(0,0,W,H);
  ctx.fillStyle = "#101318"; ctx.fillRect(0,0,W,H);
  if (!p) { ctx.fillStyle = "#7d8593"; ctx.fillText(S.peakErr[name] ? `cannot draw this track: ${S.peakErr[name]}` : "no peaks — re-analyze", 10, 20); return; }
  if (p.tiles) { ensureArrays(name); return; }   // P3-6b: the page holds the tiles in the engine only; the 2D path loads the arrays
  const sr = p.sr, span0 = S.view.t0*sr, spanS = viewDur()*sr, span1 = span0+spanS;   // P4-1b fix: the span of viewDur
  const mid = H/2, color = t.color;
  const amp = S.peakAmp[name] || 1;
  const scale = (H*0.45) / amp * S.ampZoom;   // auto-gain: track peak → 90% of half-height; P3-20a: times the amplitude zoom (the canvas clips)
  const fill = waveFill(viewDur());   // the tile path below; the sample window path (up to 262144 samples) draws the exact wave, which the GL lane shows as solid columns too
  if (spanS <= 262144) {
    ensureSamples(name, Math.max(0,Math.floor(span0)), Math.ceil(span1)).catch(() => null).then(() => {
      if (!GLQ.noGen && canvas._gen !== gen) return;   // P4-26: stale: a newer draw has cleared and drawn this lane
      const cache = S.samplesCache[name];
      if (!cache || !cache.data || cache.start > span0 || cache.end < span1) return;   // P3-14d: the GL engine took the window over meanwhile
      const i0 = Math.max(cache.start, Math.floor(span0)), i1 = Math.min(cache.end, Math.ceil(span1));
      const pxPer = W/spanS;
      if (pxPer > 3) {
        ctx.fillStyle = color;
        for (let i=i0; i<i1; i++) {
          const x = tToX(i/sr, W), y = mid - cache.data[i-cache.start]*scale;
          ctx.beginPath(); ctx.arc(x,y,1.4,0,7); ctx.fill();
        }
      }
      ctx.strokeStyle = color; ctx.lineWidth = 1; ctx.beginPath();
      for (let i=i0; i<i1; i++) {
        const x = tToX(i/sr, W), y = mid - cache.data[i-cache.start]*scale;
        i===i0 ? ctx.moveTo(x,y) : ctx.lineTo(x,y);
      }
      ctx.stroke();
      ctx.strokeStyle = "rgba(255,255,255,0.15)";
      ctx.beginPath(); ctx.moveTo(0,mid); ctx.lineTo(W,mid); ctx.stroke();
      drawEventLines(canvas, name);
    });
    return;
  }
  const level = pickLevel(p, spanS/W) || 256;
  const L = p.levels[level];
  const hasRms = Array.isArray(L.rms);
  // pass 1: peak envelope (min/max), translucent
  ctx.fillStyle = fill ? color : color + "55";   // P4-26: from WAVE_FILL_S solid, no RMS body
  for (let px=0; px<W; px++) {
    const s0 = span0 + px/W*spanS, s1 = span0 + (px+1)/W*spanS;
    let b0 = Math.max(0, Math.floor(s0/level)), b1 = Math.min(L.n, Math.ceil(s1/level));
    if (b1 <= b0) { b0 = Math.min(L.n-1, b0); b1 = b0+1; }
    let mn = 1, mx = -1;
    for (let b=b0; b<b1; b++) { mn = Math.min(mn, L.min[b]); mx = Math.max(mx, L.max[b]); }
    const y0 = mid - mx*scale, y1 = mid - mn*scale;
    ctx.fillRect(px, y0, 1, Math.max(1, y1-y0));
  }
  // pass 2: RMS body — the solid "flesh" of the waveform (Pro Tools / Logic look)
  if (hasRms && !fill) {
    ctx.fillStyle = color;
    for (let px=0; px<W; px++) {
      const s0 = span0 + px/W*spanS, s1 = span0 + (px+1)/W*spanS;
      let b0 = Math.max(0, Math.floor(s0/level)), b1 = Math.min(L.n, Math.ceil(s1/level));
      if (b1 <= b0) { b0 = Math.min(L.n-1, b0); b1 = b0+1; }
      let rms = 0;
      for (let b=b0; b<b1; b++) rms = Math.max(rms, L.rms[b]);
      const hh = rms*scale;
      if (hh > 0.3) ctx.fillRect(px, mid-hh, 1, hh*2);
    }
  }
  ctx.strokeStyle = "rgba(255,255,255,0.12)"; ctx.beginPath(); ctx.moveTo(0,mid); ctx.lineTo(W,mid); ctx.stroke();
  drawEventLines(canvas, name);
}

function drawSpike(canvas, name) {
  const ctx = canvas.getContext("2d"), [W, H] = laneSize(canvas);
  ctx.clearRect(0,0,W,H);
  ctx.fillStyle = S.tracks[name].color + "0d"; ctx.fillRect(0,0,W,H);
  ctx.strokeStyle = "#2c313c"; ctx.beginPath(); ctx.moveTo(0,H-0.5); ctx.lineTo(W,H-0.5); ctx.stroke();
  drawEventLines(canvas, name);
}

/* P3-14d (W6): the hits of a track are sorted by time, except the one a drag moves (hitDrag) until the drag ends. The
   hits in [t0, t1] are found by a binary search: the dragged hit counts as its left neighbour's time in the search,
   so the rest stays sorted, and it is tested on its own. Returns the indices in order. */
let hitDrag = null;   // {track, index} while a hit of that track is dragged
function hitsIn(evs, t0, t1, skip) {
  const key = i => i === skip ? (i > 0 ? evs[i-1].t : -Infinity) : evs[i].t;
  let lo = 0, hi = evs.length;
  while (lo < hi) { const mid = (lo + hi) >> 1; if (key(mid) < t0) lo = mid + 1; else hi = mid; }
  const out = [];
  for (let i = lo; i < evs.length && key(i) <= t1; i++) if (i !== skip) out.push(i);
  if (skip >= 0 && skip < evs.length && evs[skip].t >= t0 && evs[skip].t <= t1) {
    let k = 0; while (k < out.length && out[k] < skip) k++;
    out.splice(k, 0, skip);
  }
  return out;
}
function dragSkip(name) { return hitDrag && hitDrag.track === name ? hitDrag.index : -1; }
/* Event lines drawn through BOTH lanes: handle in spike lane, continuation in wave lane. */
function drawEventLines(canvas, name) {
  const ctx = canvas.getContext("2d"), [W, H] = laneSize(canvas);
  const isSpike = canvas.classList.contains("spike");
  drawGridLines(ctx, W, H);   // P4-8: the bar and beat lines lie under the hit lines
  lnDrawBand(ctx, W, H, name);   // P4-6a: the range selection
  const color = S.tracks[name].color, events = S.events[name] || [];
  const lnNot = lnLearn(name) ? lnTimes(name).not : null;   // P4-6a: a hit marked "not"; P4-6c: grey when the filter took it out, else a red tint (it is exported, Tab stops on it)
  const pad = 5 * viewDur() / W;   // the hits up to 5 px outside the lane: their markers (8 px wide) reach into it (P4-1: a strip shows them too)
  // Owner request 2026-10-11: a hit line in the track colour vanished over the wave of that colour (the old 2D wave
  // filled its body at 33 %, the GL wave is solid). In the wave lane the lines are the UI text grey at 60 %, with a
  // soft dark halo, so the position reads on any colour without glare (white at 85 % was too bright for the owner).
  // Manual hits stay magenta, the selected hit yellow. Three passes: the halos in one path, then the lines per colour.
  const groups = new Map();   // colour -> [x, ...]
  let sel = null;
  for (const i of hitsIn(events, S.view.t0 - pad, S.view.t1 + pad, dragSkip(name))) {
    const x = Math.round(tToX(events[i].t, W)) + 0.5;
    if (x < -5 || x > W+5) continue;
    const t0 = events[i].t;
    const pv = lnPreview(name, events[i]);   // P4-6c: a threshold drag shows the grey of the live threshold
    const evGrey = pv === null ? hgIsCut(events[i]) : pv || hgCutOnly(events[i]);
    if (S.sel && S.sel.track===name && S.sel.index===i) { sel = { x, i, grey: evGrey }; continue; }   // #175: a selected deactivated hit keeps its state visible
    const notTint = !evGrey && lnNot && lnNot.length && lnNear(lnNot, t0);   // marked "not", still in the output (filter off, or an edited hit)
    const c = evGrey ? HG_GREY : notTint ? LN_NOT_TINT : (isManual(events[i]) || events[i].edited) ? MANUAL_COLOR : (isSpike ? color : "#cdd3de");   // P4-3: a cut hit is grey; P4-6c: so is a hit the Learn filter took out; a "not" example still in the output is tinted; P4-40f: a hand velocity keeps the manual colour
    (groups.get(c) || groups.set(c, []).get(c)).push(x);
  }
  const gone = HG.ghosts[name];   // P4-3: while the Sensitivity moves, the hits that go stay as grey lines
  if (gone && gone.length) {
    ctx.strokeStyle = "#7a818d"; ctx.lineWidth = 1.25; ctx.globalAlpha = 1; ctx.setLineDash([4, 3]); ctx.beginPath();
    for (const t of gone) { const x = Math.round(tToX(t, W)) + 0.5; if (x >= -5 && x <= W + 5) { ctx.moveTo(x, 0); ctx.lineTo(x, H); } }
    ctx.stroke(); ctx.setLineDash([]);
  }
  ctx.lineWidth = 3; ctx.strokeStyle = "#000"; ctx.globalAlpha = isSpike ? 0.5 : 0.4;
  ctx.beginPath();
  for (const xs of groups.values()) for (const x of xs) { ctx.moveTo(x, 0); ctx.lineTo(x, H); }
  if (sel) { ctx.moveTo(sel.x, 0); ctx.lineTo(sel.x, H); }
  ctx.stroke();
  ctx.lineWidth = 1; ctx.globalAlpha = isSpike ? 1 : 0.6;
  for (const [c, xs] of groups) {
    ctx.strokeStyle = c; ctx.beginPath();
    for (const x of xs) { ctx.moveTo(x, 0); ctx.lineTo(x, H); }
    ctx.stroke();
    if (isSpike) { ctx.fillStyle = c; for (const x of xs) { ctx.beginPath(); ctx.moveTo(x-4, 0); ctx.lineTo(x+4, 0); ctx.lineTo(x, 7); ctx.closePath(); ctx.fill(); } }
  }
  if (sel) {
    ctx.globalAlpha = 1;
    if (sel.grey) {   // #175: a selected deactivated hit stays visibly deactivated: the line dashed and grey, the marker in the deactivated colour, the selection ring yellow
      ctx.strokeStyle = HG_GREY; ctx.lineWidth = 2; ctx.setLineDash([5, 4]);
      ctx.beginPath(); ctx.moveTo(sel.x, 0); ctx.lineTo(sel.x, H); ctx.stroke();
      ctx.setLineDash([]);
      if (isSpike) {
        ctx.fillStyle = HG_GREY;
        ctx.beginPath(); ctx.moveTo(sel.x-4, 0); ctx.lineTo(sel.x+4, 0); ctx.lineTo(sel.x, 7); ctx.closePath(); ctx.fill();
        ctx.strokeStyle = "#ffe066"; ctx.lineWidth = 1.5;   // the selection ring around the grey marker
        ctx.beginPath(); ctx.arc(sel.x, 3.5, 7, 0, 2 * Math.PI); ctx.stroke();
        ctx.font = "9px sans-serif";
        ctx.fillStyle = "#cdd3de";
        ctx.fillText(`${sel.i+1}/${events.length} · deactivated`, sel.x+5, 9);
      }
    } else {
      ctx.strokeStyle = "#ffe066"; ctx.lineWidth = 2;
      ctx.beginPath(); ctx.moveTo(sel.x, 0); ctx.lineTo(sel.x, H); ctx.stroke();
      if (isSpike) {
        ctx.fillStyle = "#ffe066";
        ctx.beginPath(); ctx.moveTo(sel.x-4, 0); ctx.lineTo(sel.x+4, 0); ctx.lineTo(sel.x, 7); ctx.closePath(); ctx.fill();
        ctx.font = "9px sans-serif"; ctx.fillStyle = "#ffe066";
        ctx.fillText(`${sel.i+1}/${events.length}`, sel.x+5, 9);
      }
    }
  }
  ctx.globalAlpha = 1; ctx.lineWidth = 1;
  if (isSpike) lnDrawMarks(ctx, W, H, name);   // P4-6a: the mark bars and the example ticks over the hit lines
}

/* ---------------- samples cache ---------------- */
/* P3-6b: one window fetch per track at a time. Without this a zoom gesture started a new 1 MB fetch for each of the
   visible lanes in every frame while the earlier ones were still on their way (23 to 30 fetches of 0.8 MB with a median
   of 300 to 460 ms in one 2 s sweep, frames up to 120 ms); now a call only records the range it wants, and the running
   fetch is followed by one more for the newest range. GLQ.noCoalesce restores the old behaviour (measurements). */
const SF = {};
function windowCovers(name, start, end) {
  const cache = S.samplesCache[name], eng = glEngine();
  return !!(cache && cache.start <= start && cache.end >= end && !!cache.data === !eng);   // data in the page: Canvas 2D; none: in the engine
}
function ensureSamples(name, start, end) {
  if (windowCovers(name, start, end)) return Promise.resolve();
  if (GLQ.noCoalesce) return fetchWindow(name, start, end);
  const f = SF[name] || (SF[name] = { run: null, want: null });
  f.want = { start, end };
  if (!f.run) f.run = (async () => {
    try {
      while (f.want) {
        const w = f.want; f.want = null;
        if (!windowCovers(name, w.start, w.end)) await fetchWindow(name, w.start, w.end);
      }
    } finally { f.run = null; }
  })();
  return f.run;
}
async function fetchWindow(name, start, end) {
  const eng = glEngine(), sid = S.sid;
  const s = Math.max(0, start - 48000), e = end + 48000;
  const r = await B.samples(sid, name, s, e);
  if (S.sid !== sid) return;   // another project is open now
  if (eng && !eng.lost) {   // P3-6b: the window moves to the engine; the page keeps its range only
    S.samplesCache[name] = { start: r.start, end: r.start + r.data.length };
    eng.setWindow(name, r.start, r.data);
    return;
  }
  S.samplesCache[name] = { start: r.start, end: r.start + r.data.length, data: r.data };
  if (S.samplesCache[name].data.length > 900000) {
    const c2 = S.samplesCache[name];
    const keep0 = Math.max(c2.start, Math.floor(start) - 96000), keep1 = Math.min(c2.end, end + 96000);
    c2.data = c2.data.slice(keep0 - c2.start, keep1 - c2.start); c2.start = keep0; c2.end = keep1;
  }
}
