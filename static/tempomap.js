/* Drum Spike v2 — tempo map (refactor plan step 5.2, R9 A2)
   The tempo and meter map (TM): the rulers (Tempo, Meter, Bars|Beats, Markers), the grid lines,
   the marker edits, bar 1, the write queue and undo, File > Import Tempo Map and the Detect Tempo
   dialog. Moved verbatim from app.js. Plain script (no build step), loaded after followstrip.js
   and before app.js. The names stay top-level bindings (page globals). The lane cursor (CUR) stays
   in app.js with its load-time listeners. */
"use strict";

/* ---------------- tempo and meter map (P4-8 stage C) ----------------
   The map lives in the service (tempo_map_get, tempo_map_set, tempo_map_detect, tempo_map_import_midi): the page holds a copy
   (TM.map) only to draw it. The page does no tempo logic beyond drawing: it walks the map to place the bar and beat lines
   (tmGrid) and sends every change as the whole new map. One change is one undo step: {type: "tempo", before, after}, as the
   re-pick does (tempo_map_set answers `previous`). Three rulers above the time ruler (Tempo, Meter, Bars|Beats; View menu)
   and the grid lines in the lanes (drawEventLines draws them first, so the hit lines lie over them). Each of the four
   states is an app setting (ruler_tempo, ruler_meter, ruler_beats, grid_lines); no value means "on" (P4-29; before: only once a map is set, not
   the default map). The rulers are 2D canvases drawn by drawRuler (the same x scale as the time ruler) and move with the
   follow strip like it. Double-click a ruler: add a step or a change (at the nearest beat, or bar), or edit the value in
   place; click a value and press Delete: remove it. */
const TM = { map: null, isDefault: true, grid: null, gridOn: false, vis: { tempo: false, meter: false, beats: false, markers: false }, cans: null,
  ui: { tempo: null, meter: null, beats: null, markers: null, grid: null }, sel: null, edit: null, chain: Promise.resolve(), pending: 0, mk: {}, widths: new Map(), mctx: null,
  col: { bar: "rgba(138,147,163,0.34)", beat: "rgba(138,147,163,0.14)", sub: "rgba(138,147,163,0.07)", text: "#cdd3de", dim: "#7d8593", accent: "#4da3ff", line: "#2c313c", orange: "#ff9f43" } };
const TM_KINDS = ["tempo", "meter", "beats", "markers"], TM_H = 18;
const TM_SETTING = { tempo: "ruler_tempo", meter: "ruler_meter", beats: "ruler_beats", markers: "ruler_markers", grid: "grid_lines" };
const TM_NAME = { tempo: "Tempo ruler", meter: "Meter ruler", beats: "Bars|Beats ruler", markers: "Markers ruler", grid: "Bar and beat grid" };
function tmShown(k) {   // P4-29: no saved choice = shown (the map rulers also with the default map; P4-40g: the markers ruler when the project has markers)
  const v = TM.ui[k];
  return v === null ? (k === "markers" ? MK.list.length > 0 : !!TM.map) : v;
}
function tmBpmText(b) { return String(Math.round(b * 100) / 100); }   // 94.99999208 shows as 95; the map keeps the exact value
const tmMeterText = m => `${m[0]}/${m[1]}`;
function tmColors() {   // outside the frame: getComputedStyle is a layout read
  const cs = getComputedStyle(document.documentElement), get = (n, d) => cs.getPropertyValue(n).trim() || d;
  TM.col = { bar: get("--grid-bar", TM.col.bar), beat: get("--grid-beat", TM.col.beat), sub: get("--grid-sub", TM.col.sub), text: get("--text", TM.col.text), dim: get("--dim", TM.col.dim),
    accent: get("--accent", TM.col.accent), line: get("--line", TM.col.line), orange: get("--orange", TM.col.orange) };
}
function tmInit() {   // at boot: the canvases and the events of the rulers
  if (TM.cans) return;
  TM.cans = {};
  const box = $("#mapRulers");
  for (const row of box.querySelectorAll(".mrow")) {
    const kind = row.dataset.ruler, c = row.querySelector("canvas.mcan");
    TM.cans[kind] = c;
    if (kind === "beats") {   // a click puts the playhead, as on the time ruler; P4-39d: the bar-1 marker is dragged or double-clicked
      c.addEventListener("mousedown", tmBeatsDown); c.addEventListener("dblclick", tmBeatsDouble);
      c.addEventListener("mousemove", ev => c.classList.toggle("onMarker", !!(TM.map && S.sid && tmBar1Hit(rulerXOf(ev)))));   // P4-40b: the resize cursor on the bar-1 marker
      c.addEventListener("mouseleave", () => c.classList.remove("onMarker"));
    } else {
      c.addEventListener("mousedown", ev => tmDown(kind, ev)); c.addEventListener("dblclick", ev => tmDouble(kind, ev));
      if (kind === "tempo") {   // #173: the drag cursor on a tempo step marker (the start marker is not draggable)
        c.addEventListener("mousemove", ev => c.classList.toggle("onMarker", !!(TM.map && S.sid && tmTempoHit(ev))));
        c.addEventListener("mouseleave", () => c.classList.remove("onMarker"));
      }
    }
  }
  const inp = $("#mapEdit");
  inp.addEventListener("keydown", ev => {
    if (ev.key === "Enter") { ev.preventDefault(); tmEditEnd(true); }
    else if (ev.key === "Escape") { ev.preventDefault(); ev.stopPropagation(); tmEditEnd(false); }
  });
  inp.addEventListener("blur", () => tmEditEnd(true));
  window.addEventListener("mousedown", ev => {   // a click outside the field applies it (a lane or the ruler calls preventDefault: no blur); outside the rulers it drops the selection
    if (TM.edit && ev.target !== inp) tmEditEnd(true);
    if (TM.sel && !(ev.target.closest && ev.target.closest("#mapRulers"))) { TM.sel = null; invalidate(F.VIEW); }
  }, true);
  tmColors();
}
async function tmLoad() {   // with the project: the copy of its map, then the rulers and the grid as the settings say
  tmEditEnd(false); TM.sel = null; TM.grid = null; TM.mk = {};
  TM.map = null; TM.isDefault = true;
  if (S.sid && B.tempoMapGet) {
    try { const r = await B.tempoMapGet(S.sid); TM.map = r.map; TM.isDefault = !!r.default; } catch (e) { /* no map: nothing is drawn */ }
  }
  await mkLoad();   // P4-40g: the marker list (also without a map)
  tmApplyUi(true);
}
function tmApplyUi(resize) {   // the rulers and the grid as tmShown says; draws again
  if (!TM.cans) return;
  for (const k of TM_KINDS) TM.vis[k] = tmShown(k);
  TM.gridOn = tmShown("grid");
  $("#mapRulers").classList.toggle("hidden", !TM_KINDS.some(k => TM.vis[k]));
  for (const row of document.querySelectorAll("#mapRulers .mrow")) row.classList.toggle("hidden", !TM.vis[row.dataset.ruler]);
  spLeave("map");
  invalidate((resize ? F.SIZE : 0) | F.VIEW);
  DSShell.sync();
}
function tmToggleUi(k) {   // View > Tempo Ruler, Meter Ruler, Bars|Beats Ruler, Show Bar and Beat Grid
  TM.ui[k] = !tmShown(k);
  if (B.appSettingsSet) B.appSettingsSet(TM_SETTING[k], TM.ui[k]).catch(e => setMsg(`settings: ${errText(e)}`));
  tmApplyUi(true);
  setMsg(`${TM_NAME[k]} ${TM.ui[k] ? "shown" : "hidden"}`);
}

/* The bar and beat times from the map: beat i is at g.t[i], in bar g.bar[i], beat number g.beat[i] (1 is the bar line);
   g.bars[b - 1] is the time of bar b. A beat is 4/den quarter notes; a tempo step inside a beat is handled exactly. The walk
   starts at bar 1 (bar1_s) and ends a second after the longest track. Cached until the map or the project changes. */
function tmGrid() {
  const m = TM.map;
  if (!m) return null;
  if (TM.grid) return TM.grid;
  const end = maxDur() + 1, steps = m.steps, changes = m.changes, g = { t: [], bar: [], beat: [], bars: [] };
  let t = m.bar1_s, bar = 1, si = 0, ci = 0, meter = m.meter;
  while (si < steps.length && steps[si].t <= t) si++;
  let bpm = si > 0 ? steps[si - 1].bpm : m.bpm;
  while (t <= end && g.t.length < 400000) {
    while (ci < changes.length && changes[ci].bar <= bar) meter = changes[ci++].meter;
    g.bars.push(t);
    for (let b = 1; b <= meter[0] && t <= end; b++) {
      g.t.push(t); g.bar.push(bar); g.beat.push(b);
      let rem = 4 / meter[1];   // quarter notes to the next beat
      while (rem > 1e-12) {
        const stepT = si < steps.length ? steps[si].t : Infinity, dt = rem * 60 / bpm;
        if (t + dt <= stepT) { t += dt; rem = 0; }
        else { rem -= (stepT - t) * bpm / 60; t = stepT; bpm = steps[si++].bpm; }
      }
    }
    bar++;
  }
  return (TM.grid = g);
}
function tmLower(a, x) { let lo = 0, hi = a.length; while (lo < hi) { const mid = (lo + hi) >> 1; if (a[mid] < x) lo = mid + 1; else hi = mid; } return lo; }
function tmNearest(a, x) { const i = tmLower(a, x); return i > 0 && (i >= a.length || x - a[i - 1] <= a[i] - x) ? i - 1 : Math.min(i, a.length - 1); }
function tmBpmAt(t) { const m = TM.map; let b = m.bpm; for (const s of m.steps) { if (s.t <= t) b = s.bpm; else break; } return b; }
function tmMeterAt(bar) { const m = TM.map; let v = m.meter; for (const c of m.changes) { if (c.bar <= bar) v = c.meter; else break; } return v; }

/* The grid lines of one lane, drawn before the hit lines (drawEventLines). Beat lines are dropped under 6 px apart, bar lines under 4.
   A beat wider than 24 px gets its 8th line, wider than 48 px its 8th and 16th lines (TM.col.sub, fainter than the beat lines). */
function drawGridLines(ctx, W, H) {
  if (!TM.gridOn || !TM.map) return;
  const g = tmGrid();
  if (!g || !g.t.length) return;
  const d = viewDur(), pad = 2 * d / W, a = S.view.t0 - pad, b = S.view.t1 + pad;
  const i0 = tmLower(g.t, a), i1 = tmLower(g.t, b), nb = i1 - i0;
  const bars = tmLower(g.bars, b) - tmLower(g.bars, a);
  if (nb <= 0) return;
  const beats = W / nb >= 6, barLines = !bars || W / bars >= 4;
  if (!beats && !barLines) return;
  ctx.save(); ctx.lineWidth = 1; ctx.globalAlpha = 1;
  if (beats) {
    ctx.strokeStyle = TM.col.beat; ctx.beginPath();
    for (let i = i0; i < i1; i++) if (g.beat[i] !== 1) { const x = Math.round(tToX(g.t[i], W)) + 0.5; ctx.moveTo(x, 0); ctx.lineTo(x, H); }
    ctx.stroke();
    ctx.strokeStyle = TM.col.sub; ctx.beginPath();   // the subdivisions of each beat by its own width on the lane (a step inside a beat changes it)
    for (let i = i0; i < i1 && i + 1 < g.t.length; i++) {
      const x0 = tToX(g.t[i], W), x1 = tToX(g.t[i + 1], W), n = x1 - x0 > 48 ? 4 : x1 - x0 > 24 ? 2 : 1;
      for (let k = 1; k < n; k++) { const x = Math.round(x0 + (x1 - x0) * k / n) + 0.5; ctx.moveTo(x, 0); ctx.lineTo(x, H); }
    }
    ctx.stroke();
  }
  if (barLines) {
    ctx.strokeStyle = TM.col.bar; ctx.beginPath();
    for (let i = i0; i < i1; i++) if (g.beat[i] === 1) { const x = Math.round(tToX(g.t[i], W)) + 0.5; ctx.moveTo(x, 0); ctx.lineTo(x, H); }
    ctx.stroke();
  }
  ctx.restore();
}

/* The rulers. Markers (a tempo step, a meter change) are lines with the value to their right. */
function tmMarkers(kind) {   // cached with the grid (tmLoad clears both); P4-40g: the review markers come from the service
  if (kind === "markers") return MK.items;
  if (!TM.map) return [];
  return TM.mk[kind] || (TM.mk[kind] = tmMarkersBuild(kind));
}
function tmMarkersBuild(kind) {
  const m = TM.map, out = [];
  if (kind === "tempo") {
    out.push({ id: "start", i: -1, t: 0, text: tmBpmText(m.bpm), del: false });
    m.steps.forEach((s, i) => out.push({ id: "s" + i, i, t: s.t, text: tmBpmText(s.bpm), del: true }));
  } else {
    const g = tmGrid();
    out.push({ id: "start", i: -1, t: g && g.bars.length ? g.bars[0] : m.bar1_s, text: tmMeterText(m.meter), del: false });
    m.changes.forEach((c, i) => { const t = g && g.bars[c.bar - 1]; if (t !== undefined) out.push({ id: "c" + i, i, bar: c.bar, t, text: tmMeterText(c.meter), del: true }); });
  }
  return out;
}
function tmTextW(text) {
  let w = TM.widths.get(text);
  if (w === undefined) {
    if (!TM.mctx) { TM.mctx = document.createElement("canvas").getContext("2d"); TM.mctx.font = "10px -apple-system, sans-serif"; }
    TM.widths.set(text, w = Math.ceil(TM.mctx.measureText(text).width));
    if (TM.widths.size > 500) TM.widths.clear();
  }
  return w;
}
function tmHit(kind, px) {   // the marker under x (ruler coordinates), or null
  for (const it of tmMarkers(kind)) {
    const x = tToX(it.t, GEO.rulerW);
    if (px >= x - 4 && px <= x + tmTextW(it.text) + 8) return it;
  }
  return null;
}
function tmTempoHit(ev) {   // #173: the draggable tempo step under the pointer, or null (the start marker is not a step)
  if (!TM.map || !S.sid) return null;
  const it = tmHit("tempo", rulerXOf(ev));
  return it && it.del ? it : null;
}
function sizeMapCanvases() {   // in the frame (and for a strip): the visible canvases to GEO.rulerW; writes only
  if (!TM.cans) return;
  const dpr = GEO.dpr, rw = GEO.rulerW;
  for (const k of TM_KINDS) {
    if (!TM.vis[k]) continue;
    const c = TM.cans[k];
    c.style.width = rw + "px"; c.style.height = TM_H + "px";
    c.style.transform = ""; c._sptf = "";   // #150: a map ruler sized for a draw is not on a strip (the stale strip transform would shift the draw off the clip)
    c.width = rw * dpr; c.height = TM_H * dpr; c.getContext("2d").setTransform(dpr, 0, 0, dpr, 0, 0);
  }
}
function drawMapRulers() {
  if (!TM.cans) return;
  for (const k of TM_KINDS) if (TM.vis[k]) { if (k === "beats") drawBeatsRuler(TM.cans[k]); else drawMarkerRuler(k, TM.cans[k]); }
}
function drawMarkerRuler(kind, c) {
  const ctx = c.getContext("2d"), W = GEO.rulerW, H = TM_H, col = TM.col;
  ctx.clearRect(0, 0, W, H);
  if (kind === "markers") { if (!MK.items.length) return; }   // P4-40g: the review markers, independent of the map
  else if (!TM.map) return;
  ctx.font = "10px -apple-system, sans-serif"; ctx.textBaseline = "middle"; ctx.lineWidth = 1;
  const mk = tmMarkers(kind), selId = TM.sel && TM.sel.kind === kind ? TM.sel.id : null;
  for (const it of mk) {
    const x = Math.round(tToX(it.t, W)) + 0.5;
    if (x > W) break;
    const on = it.id === selId;
    ctx.strokeStyle = on ? col.accent : (kind === "markers" ? col.orange : col.dim);
    ctx.fillStyle = on ? col.accent : col.text;
    ctx.beginPath(); ctx.moveTo(x, 2); ctx.lineTo(x, H); ctx.stroke();
    if (kind === "markers") {   // a flag: the marker's note, or its number
      ctx.beginPath(); ctx.moveTo(x, 2); ctx.lineTo(x + 5, 2); ctx.stroke();
    }
    ctx.fillText(it.text, x + 4, H / 2);
  }
  /* #150: the value in force at the left edge of each view the canvas draws. One view: the label at x 3. Under the
     follow strip the canvas holds two views (spApply, SP.drawing): a label only at x 3 would glide off-screen with
     the first view, so the second view's left edge gets its own carry label too (the last marker left of that edge). */
  if (kind !== "markers") {
    const half = SP.drawing ? W / 2 : W;
    for (let vx = 0; vx < W; vx += half) {
      let c2 = null, f2 = null;
      for (const it of mk) {
        const x = Math.round(tToX(it.t, W)) + 0.5;
        if (x >= vx + half) break;
        if (x < vx) { c2 = it; continue; }
        if (f2 === null) f2 = x;
      }
      if (c2 && (f2 === null || f2 - vx > tmTextW(c2.text) + 12)) { ctx.fillStyle = col.dim; ctx.fillText(c2.text, vx + 3, H / 2); }
    }
  }
}
function drawBeatsRuler(c) {
  const ctx = c.getContext("2d"), W = GEO.rulerW, H = TM_H, col = TM.col;
  ctx.clearRect(0, 0, W, H);
  const g = TM.map && tmGrid();
  if (!g || !g.t.length) return;
  const d = viewDur(), pad = 40 * d / W, a = S.view.t0 - pad, b = S.view.t1 + pad;
  const i0 = tmLower(g.t, a), i1 = tmLower(g.t, b), nb = i1 - i0;
  if (nb <= 0) return;
  const bars = Math.max(1, tmLower(g.bars, b) - tmLower(g.bars, a)), pxBar = W / bars, pxBeat = W / nb;
  let k = 1;
  for (const s of [1, 2, 4, 5, 8, 10, 16, 20, 25, 50, 100, 200, 500, 1000]) { k = s; if (pxBar * s >= 44) break; }
  ctx.font = "10px -apple-system, sans-serif"; ctx.textBaseline = "middle"; ctx.lineWidth = 1;
  const small = [], bigs = [], labels = [];
  for (let i = i0; i < i1; i++) {
    const x = Math.round(tToX(g.t[i], W)) + 0.5;
    if (g.beat[i] === 1) {
      if (pxBar >= 4 || (g.bar[i] - 1) % k === 0) bigs.push(x);
      if ((g.bar[i] - 1) % k === 0) labels.push([x, String(g.bar[i])]);
    } else if (pxBeat >= 5) {
      small.push(x);
      if (pxBeat >= 44) labels.push([x, `${g.bar[i]}|${g.beat[i]}`, true]);
    }
  }
  ctx.strokeStyle = col.line; ctx.beginPath();
  for (const x of small) { ctx.moveTo(x, H - 5); ctx.lineTo(x, H); }
  ctx.stroke();
  ctx.strokeStyle = col.dim; ctx.beginPath();
  for (const x of bigs) { ctx.moveTo(x, 2); ctx.lineTo(x, H); }
  ctx.stroke();
  for (const [x, text, dim] of labels) { ctx.fillStyle = dim ? col.dim : col.text; ctx.fillText(text, x + 4, H / 2); }
  const x1 = Math.round(tToX(g.bars[0], W)) + 0.5;   // P4-39d: the bar-1 marker (drag it, or double-click it for its time)
  if (x1 >= 0 && x1 <= W) { ctx.strokeStyle = col.accent; ctx.beginPath(); ctx.moveTo(x1, 0); ctx.lineTo(x1, H); ctx.stroke(); }
}

/* Selecting, adding, editing and deleting. */
function tmDown(kind, ev) {
  if (ev.button !== 0) return;
  const it = tmHit(kind, ev.clientX - GEO.rulerX);
  ev.preventDefault();
  if (kind === "markers" && !it) return rulerDown(ev);   // P4-40g: a click on the ruler puts the playhead
  if (TM.edit) tmEditEnd(true);
  TM.sel = it ? { kind, id: it.id, i: it.i, del: it.del } : null;
  if (it && S.sel) { S.sel = null; updateStatus(); }
  if (it && kind === "tempo" && it.del && TM.map && S.sid) tmTempoDrag(it, ev);   // #173: a step marker drags
  invalidate(F.VIEW);
}
/* #173: a drag of a tempo step marker moves that step by the pointer delta (no snap: a drag of 200 ms
   moves the step 200 ms), clamped between the neighbours with the map's 1 ms gap (MIN_STEP_GAP_S of the
   service; the first step stays after 0). The preview map redraws the ruler, the bars and the grid at
   once (TM.grid and TM.mk cleared, like the bar-1 drag); the drop is one tmCommit, so one undo step.
   The start marker is the tempo in force, not a change: not draggable (as Delete refuses it). */
function tmTempoDrag(it, ev) {
  const orig = TM.map, i = it.i, cx0 = ev.clientX, px0 = rulerXOf(ev), gap = 0.001;
  let t = it.t, moved = false, copy = null;
  const lo = i > 0 ? orig.steps[i - 1].t + gap : gap, hi = i + 1 < orig.steps.length ? orig.steps[i + 1].t - gap : Infinity;
  const show = tt => {
    if (tt === orig.steps[i].t) { if (TM.map !== orig) { TM.map = orig; TM.grid = null; TM.mk = {}; invalidate(F.VIEW); } return; }
    if (!copy) copy = JSON.parse(JSON.stringify(orig));
    copy.steps[i].t = tt;
    TM.map = copy; TM.grid = null; TM.mk = {}; invalidate(F.VIEW);
  };
  const move = e2 => {
    if (!moved && Math.abs(e2.clientX - cx0) < 3) return;
    moved = true;
    t = Math.min(hi, Math.max(lo, it.t + xToT(rulerXOf(e2), GEO.rulerW) - xToT(px0, GEO.rulerW)));
    show(t);
  };
  const up = () => {
    window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up);
    show(orig.steps[i].t);   // the stored map until the service has the new one
    if (moved && t !== orig.steps[i].t) {
      const next = JSON.parse(JSON.stringify(orig));
      next.steps[i].t = t;
      tmCommit(next, `tempo step ${tmBpmText(orig.steps[i].bpm)} BPM at ${fmt(t, 3)} s`);
    }
  };
  window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
}
function tmDouble(kind, ev) {
  if (!S.sid) return;
  if (kind !== "markers" && !TM.map) return;
  ev.preventDefault();
  const px = ev.clientX - GEO.rulerX, t = xToT(px, GEO.rulerW), g = tmGrid();
  let it = tmHit(kind, px);
  if (!it && kind === "markers") {   // P4-40g: a double-click adds a marker (the field takes its optional note)
    it = { id: "new", i: -1, t: Math.max(0, t), text: "", add: true };
  } else if (!it && kind === "tempo") {
    const bt = g && g.t.length ? g.t[tmNearest(g.t, t)] : Math.max(0, t), at = Math.round(bt * 1e6) / 1e6;
    it = tmMarkers("tempo").find(x => Math.abs(x.t - at) < 0.0005);   // a step is there already: edit it
    if (!it) it = { id: "new", i: -1, t: at, text: tmBpmText(tmBpmAt(at)), add: true };
  } else if (!it && kind === "meter") {
    if (!g || !g.bars.length) return;
    const n = tmNearest(g.bars, t) + 1;
    it = tmMarkers("meter").find(x => x.bar === n || (n === 1 && x.id === "start"));
    if (!it) it = { id: "new", i: -1, bar: n, t: g.bars[n - 1], text: tmMeterText(tmMeterAt(n)), add: true };
  }
  if (it) tmEditStart(kind, it);
}
function tmEditStart(kind, it) {
  tmEditEnd(false);
  spLeave("map");   // a strip would move the markers away from the field
  const inp = $("#mapEdit"), row = document.querySelector(`#mapRulers .mrow[data-ruler="${kind}"]`);
  const x = tToX(it.t, GEO.rulerW), box = $("#mapRulers").getBoundingClientRect().left;
  inp.style.left = Math.round(GEO.rulerX - box + Math.min(Math.max(2, x), GEO.rulerW - 70)) + "px";
  inp.style.top = row.offsetTop + 1 + "px";
  inp.value = it.raw !== undefined ? it.raw : it.text; inp.classList.remove("hidden");
  TM.edit = { kind, it, done: false };
  inp.focus(); inp.select();
}
function tmEditEnd(commit) {
  const e = TM.edit;
  if (!e || e.done) return;
  e.done = true; TM.edit = null;
  const inp = $("#mapEdit"), v = inp.value.trim();
  inp.classList.add("hidden");
  if (!commit || (e.kind !== "markers" && !TM.map)) return;
  if (v === e.it.text && !e.it.add && e.it.raw === undefined) return;   // unchanged: the exact imported value stays
  if (e.kind === "beats") {   // P4-39d: the time of bar 1 in seconds
    const s = Number(v.replace(/\s*s$/, ""));
    if (!v || !Number.isFinite(s) || s < 0) { setMsg("bar 1: type a time in seconds, for example 2.025"); return; }
    return void tmSetBar1(s);
  }
  if (e.kind === "markers") {   // P4-40g: the note of a marker; an empty text is no note
    const list = MK.list.slice(), it = e.it;
    if (it.add) {
      const entry = { t: it.t };
      if (v) entry.note = v;
      list.push(entry);
      return void mkCommit(list, `marker at ${fmt(it.t, 3)} s${v ? `: ${v}` : ""}`, it.t);
    }
    const cur = list[it.i];
    if (!cur) return;
    if (v) cur.note = v; else delete cur.note;
    return void mkCommit(list, `marker at ${fmt(cur.t, 3)} s${v ? `: ${v}` : ": no note"}`, cur.t);
  }
  const next = JSON.parse(JSON.stringify(TM.map)), it = e.it;
  if (e.kind === "tempo") {
    const b = Number(v);
    if (!v || !Number.isFinite(b) || b <= 0) { setMsg("tempo: type a number of BPM, for example 95"); return; }
    if (it.add && tmBpmText(b) === it.text) { setMsg("tempo: the same as the tempo there, no step added"); return; }   // a step equal to the value in force changes nothing
    if (it.id === "start") { next.bpm = b; return void tmCommitStartTempo(next); }
    if (it.add) { next.steps.push({ t: it.t, bpm: b }); next.steps.sort((p, q) => p.t - q.t); }
    else next.steps[it.i].bpm = b;
    return void tmCommit(next, it.add ? `tempo step ${tmBpmText(b)} BPM at ${fmt(it.t, 2)} s` : `tempo ${tmBpmText(b)} BPM`);
  }
  const mm = /^(\d{1,3})\s*\/\s*(\d{1,3})$/.exec(v);
  if (!mm) { setMsg("meter: type beats and note value, for example 7/8"); return; }
  const meter = [Number(mm[1]), Number(mm[2])];
  if (it.add && tmMeterText(meter) === it.text) { setMsg("meter: the same as the meter there, no change added"); return; }
  if (it.id === "start") next.meter = meter;
  else if (it.add) { next.changes.push({ bar: it.bar, meter }); next.changes.sort((p, q) => p.bar - q.bar); }
  else next.changes[it.i].meter = meter;
  tmCommit(next, it.add ? `meter ${v} from bar ${it.bar}` : `meter ${v}`);
}
function tmDeleteSel() {   // Delete (or Edit > Delete Hit) with a marker selected: true when it took the key
  const s = TM.sel;
  if (s && s.kind === "markers") {   // P4-40g: a selected review marker
    const m = MK.list[s.i];
    if (!m) return false;
    const list = MK.list.slice();
    list.splice(s.i, 1);
    TM.sel = null;
    mkCommit(list, `marker at ${fmt(m.t, 3)} s deleted`);
    return true;
  }
  if (!s || !TM.map) return false;
  if (!s.del) { setMsg(`the start ${s.kind === "tempo" ? "tempo" : "meter"} stays: edit its value`); return true; }
  const next = JSON.parse(JSON.stringify(TM.map));
  if (s.kind === "tempo") next.steps.splice(s.i, 1); else next.changes.splice(s.i, 1);
  TM.sel = null;
  tmCommit(next, s.kind === "tempo" ? "tempo step deleted" : "meter change deleted");
  return true;
}
/* P4-39d: bar 1 by hand. Track > Set Bar 1 at Playhead (or at the selected hit), a double-click on the bar-1 marker of the
   Bars|Beats ruler (a field with the time in seconds) and a drag of that marker move bar 1 only: the tempo, the steps, the meter
   and the meter changes stay. One undo step through tmCommit, as every tempo map edit. */
function tmSetBar1(t) {
  if (!TM.map || !S.sid) { setMsg("bar 1: open a project first"); return Promise.resolve(false); }
  const next = JSON.parse(JSON.stringify(TM.map));
  next.bar1_s = Math.max(0, t);
  return tmCommit(next, `bar 1 at ${fmt(next.bar1_s, 3)} s`);
}
function setBar1AtPlayhead() {   // Track > Set Bar 1 at Playhead: the selected hit's time when a hit is selected, else the playhead
  if (!needProject()) return null;
  const e = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  return tmSetBar1(e ? e.t : S.playhead);
}
function setBar1AtAudioStart() {   // #468: Track > Set Bar 1 at Audio Start: bar 1 at 0 s, the DAW convention
  if (!needProject()) return null;
  return tmSetBar1(0);
}
function tmBar1Hit(px) {   // true when x (ruler coordinates) is on the bar-1 marker: its line and its label "1"
  if (!TM.map) return false;
  const x = tToX(TM.map.bar1_s, GEO.rulerW);
  return px >= x - 4 && px <= x + tmTextW("1") + 8;
}

function tmBeatsDown(ev) {
  if (ev.button !== 0 || !S.sid || !tmBar1Hit(rulerXOf(ev))) return rulerDown(ev);
  ev.preventDefault();
  if (TM.edit) tmEditEnd(true);
  const orig = TM.map, x0 = ev.clientX;
  let t = orig.bar1_s, moved = false;
  const show = bar1 => { TM.map = bar1 === orig.bar1_s ? orig : { ...orig, bar1_s: bar1 }; TM.grid = null; TM.mk = {}; invalidate(F.VIEW); };
  const move = e2 => {
    if (!moved && Math.abs(e2.clientX - x0) < 3) return;
    moved = true;
    t = Math.max(0, snapPlayhead(Math.min(xToT(rulerXOf(e2), GEO.rulerW), maxDur())));
    show(t);
  };
  const up = () => {
    window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up);
    show(orig.bar1_s);   // the stored map until the service has the new one
    if (moved && t !== orig.bar1_s) tmSetBar1(t);
  };
  window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
}
function tmBeatsDouble(ev) {
  if (!TM.map || !S.sid || !tmBar1Hit(rulerXOf(ev))) return;
  ev.preventDefault();
  tmEditStart("beats", { id: "bar1", i: -1, t: TM.map.bar1_s, text: fmt(TM.map.bar1_s, 3) });
}
/* One change: the whole new map goes to the service, the answer's `previous` is the undo state. Changes are done in order. */
function tmQueue(fn) {   // every write of the map runs in order; undo and redo wait while one is on its way (tmUndoWaits)
  TM.pending++;
  return (TM.chain = TM.chain.then(fn).finally(() => { TM.pending--; }));
}
function tmUndoWaits() {   // like hgUndoWaits: an undo while a change is stored would take the wrong step
  if (!TM.pending) return false;
  setMsg("a tempo map change is being stored: undo and redo wait for it");
  return true;
}
/* A new start tempo (next.bpm), from the Tempo ruler or the Detect Tempo dialog; answers the commit's promise.
   #468: on a map without steps bar 1 is re-fitted to the hits (the phase of the kick and snare hits at
   the new tempo, tempo_map_refit_bar1), so the grid sits on them again; a map with steps keeps bar 1.
   The commit is one step, so one undo restores the tempo and bar 1 together. Hotfix 0.9.10: bar 1 is
   never before 0 s (a map with a negative bar 1 is refused, and the old tempo stayed). */
async function tmCommitStartTempo(next) {
  const b = next.bpm, was = next.bar1_s, sid0 = S.sid;
  if (next.steps.length) return tmCommit(next, `tempo ${tmBpmText(b)} BPM, bar 1 stays: the map has steps`);
  try {
    const r = await B.tempoMapRefitBar1(sid0, b);
    if (sid0 !== S.sid) return false;
    const at = Math.max(0, r.bar1_s);
    if (Math.abs(at - was) < 1e-9) return tmCommit(next, `tempo ${tmBpmText(b)} BPM, bar 1 stays at ${fmt(was, 3)} s`);
    next.bar1_s = at;
    return tmCommit(next, `tempo ${tmBpmText(b)} BPM, bar 1 moved from ${fmt(was, 3)} s to ${fmt(at, 3)} s`);
  } catch (e) { return tmCommit(next, `tempo ${tmBpmText(b)} BPM, bar 1 stays: ${errText(e)}`); }
}
function tmCommit(next, what) {
  const sid = S.sid;
  return tmQueue(async () => {
    if (sid !== S.sid) return false;
    try {
      const r = await B.tempoMapSet(sid, next);
      if (sid !== S.sid) return false;
      if (!r.changed) { setMsg(`${what}: no change`); return false; }
      pushUndo({ type: "tempo", sid, before: r.previous || {}, after: r.map, what });
      await tmLoad(); setDirtyState(true); setMsg(what);
      return true;
    } catch (e) { setMsg(`${what} refused: ${errText(e)}`); return false; }
  });
}
function tmApplyOp(op, reverse) {   // undo and redo: the stored map goes back to the service
  const sid = op.sid, map = reverse ? op.before : op.after;
  tmQueue(async () => {
    if (sid !== S.sid) return;
    try { await B.tempoMapSet(sid, map); await tmLoad(); setDirtyState(true); setMsg(`${reverse ? "undo" : "redo"}: ${op.what}`); }
    catch (e) { setMsg(`${reverse ? "undo" : "redo"} failed: ${errText(e)}`, "error"); }
  });
}

/* File > Import Tempo Map: the tempo and time-signature events of a MIDI file from the DAW replace the map (one undo step). */
async function importTempoMap() {
  if (!needProject()) return null;
  const path = await B.pickTempoFile();
  if (!path) { setMsg("import cancelled"); return null; }
  const name = path.split("/").pop(), sid = S.sid;
  return tmQueue(async () => {
    try {
      const r = await B.tempoMapImportMidi(sid, path);
      if (sid !== S.sid) return r;
      if (!r.changed) { setMsg(`${name}: the tempo map is the same as before`); return r; }
      pushUndo({ type: "tempo", sid, before: r.previous || {}, after: r.map, what: "import tempo map" });
      await tmLoad(); setDirtyState(true);
      setMsg(`imported ${r.tempo_events} tempo and ${r.meter_events} meter events from ${name}`);
      return r;
    } catch (e) { setMsg(`tempo map import refused: ${errText(e)}`); return null; }
  });
}

/* Track > Detect Tempo: the suggestion of tempo_map_detect with its confidence and warnings; Accept is one undo step. */
let tmDetecting = false;
async function detectTempo() {
  if (!needProject() || tmDetecting) return null;
  if (!$("#tempoModal").classList.contains("hidden")) { setMsg("answer the Detect Tempo dialog first"); return null; }
  tmDetecting = true; setMsg("detecting the tempo from the kick and snare hits…");
  let d;
  try { d = await B.tempoMapDetect(S.sid); }
  catch (e) { setMsg(`detect tempo failed: ${errText(e)}`, "error"); return null; }
  finally { tmDetecting = false; }
  setMsg("tempo detected: accept or cancel");
  return tmDetectDialog(d);
}
function tmDetectDialog(d) {
  return new Promise(resolve => {
    const m = $("#tempoModal"), body = $("#tempoBody"), b1 = $("#tempoBar1"), pct = x => `${Math.round(x * 100)} %`;
    const whole = b => String(Math.round(b));   // whole-number tempos only in this dialog (owner rule 2026-10-27); the map keeps the exact value
    const low = d.bar1.confidence < 0.4, m44 = (d.suggestion.meter || [4, 4]).join("/") === "4/4";
    const other = m44 ? "two beats later" : "at the other candidate";   // the candidate is two beats later in 4/4 only
    let later = false;
    const line = (text, cls) => { const p = document.createElement("p"); p.textContent = text; if (cls) p.className = cls; body.appendChild(p); return p; };
    const render = () => {
      body.textContent = "";
      line(`Start tempo: ${whole(d.suggestion.bpm)} BPM (confidence ${pct(d.start.confidence)})`);
      const alt = (d.start.alternatives || []).filter(a => !a.gated).slice(0, 3).map(a => whole(a.bpm));
      if (alt.length) line(`The beat could also be: ${alt.join(", ")} BPM`, "tdim");
      line(`Bar 1 at ${fmt(later ? d.bar1.alternative : d.bar1.t, 3)} s (confidence ${pct(d.bar1.confidence)})` + (later ? `, ${other} than the first guess` : ""));
      if (low) line("The bar 1 confidence is low: the first bar line is a coin flip. Try the other candidate.", "twarn");
      const mv = d.meter && d.meter.value || d.suggestion.meter;
      line(`Meter: ${mv[0]}/${mv[1]}` + (d.meter && d.meter.feel === "two-beat" ? " (2/4, or 6/8 in dotted quarters, fit as well)" : ""));
      if (d.steps.length) {
        line(`Tempo steps (${d.steps.length}):`);
        const tb = document.createElement("table");
        for (const s of d.steps) {
          const tr = document.createElement("tr");
          for (const x of [`bar ${s.bar}`, `${fmt(s.t, 2)} s`, `${whole(s.bpm)} BPM`, `${s.bars} bars`, `confidence ${pct(s.confidence)}`]) { const td = document.createElement("td"); td.textContent = x; tr.appendChild(td); }
          tb.appendChild(tr);
        }
        body.appendChild(tb);
      } else line("No tempo steps: the tempo is steady.");
      for (const w of d.warnings || []) line(w, "twarn");
      line(`${d.fit.hits} hits, ${d.fit.beats} beats; ${pct(d.fit.on_beat)} on the beat, jitter ${tmBpmText(d.fit.jitter_ms)} ms`, "tdim");
    };
    const close = answer => {
      m.classList.add("hidden"); document.removeEventListener("keydown", esc, true);
      resolve(answer);
    };
    const esc = ev => { if (ev.key === "Escape") { ev.stopPropagation(); setMsg("detect tempo cancelled"); close(false); } };
    document.addEventListener("keydown", esc, true);
    b1.classList.toggle("hidden", !low);
    const b1Text = () => (later ? "Bar 1 as first guessed" : m44 ? "Bar 1 two beats later" : "Bar 1 at the other candidate");
    b1.textContent = b1Text();
    b1.onclick = () => { later = !later; b1.textContent = b1Text(); render(); };
    $("#tempoCancel").onclick = () => { setMsg("detect tempo cancelled"); close(false); };
    const chosen = () => {   // the suggestion with the chosen bar 1
      const map = JSON.parse(JSON.stringify(d.suggestion));
      if (later) map.bar1_s = d.bar1.alternative;
      return map;
    };
    const done = async commit => { m.classList.add("hidden"); document.removeEventListener("keydown", esc, true); resolve(await commit); };
    // Hotfix 0.9.10 (owner): three answers. The detected tempo as suggested; half of it (the detector
    // can lock on double time) when that is above 40 BPM; or a typed tempo. The last two commit like a
    // start tempo typed in the Tempo ruler (tmCommitStartTempo); the tempo steps scale with the start.
    const atTempo = b => {
      const map = chosen(), k = b / map.bpm;
      map.bpm = b;
      for (const st of map.steps) st.bpm *= k;
      return tmCommitStartTempo(map);
    };
    const half = d.suggestion.bpm / 2, hb = $("#tempoHalf"), own = $("#tempoOwn");
    $("#tempoAccept").textContent = `Use ${whole(d.suggestion.bpm)} BPM`;
    $("#tempoAccept").onclick = () => done(tmCommit(chosen(), "tempo map detected"));
    hb.classList.toggle("hidden", !(half > 40));
    hb.textContent = `Half time: ${whole(half)} BPM`;
    hb.onclick = () => done(atTempo(half));
    own.value = "";
    const ownUse = () => {
      const b = Number(own.value.trim());
      if (!own.value.trim() || !Number.isFinite(b) || b <= 0 || b > 1000) { setMsg("tempo: type a number of BPM, for example 92"); own.focus(); return; }
      done(atTempo(b));
    };
    $("#tempoOwnUse").onclick = ownUse;
    own.onkeydown = ev => { if (ev.key === "Enter") { ev.preventDefault(); ownUse(); } };
    render();
    m.classList.remove("hidden");
  });
}
