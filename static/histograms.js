/* Drum Spike v2 — histograms (refactor plan step 5.2, R9 A2)
   The Sensitivity and loudness histograms of the Inspector (HG): the live re-pick, the shift
   and refine-window steps, the velocity map handles and the test hook window.DSHist. Moved
   verbatim from app.js. Plain script (no build step), loaded after shell.js and before app.js. */
"use strict";

/* ---------------- P4-3: the Sensitivity and loudness histograms of the Inspector (owner request 2026-10-12) ----------------
   As in Massey DRT (research note): each histogram is drawn in its control's own track.
   Sensitivity: every candidate hit of the track's activation (B.histograms, from the activations the analysis keeps in
   Analysis/<track>/activation.bin) at the slider value where it turns on: strong hits at the left, weak ones and bleed
   at the right. Coloured = detected at the slider value, grey = filtered. While the slider moves, the service picks the
   hits again from the kept activations (B.repick, no model run; the same hits as a Re-analyze at that value) and the
   lane shows them live, with the hits that go as grey lines; letting go stores the re-pick (one undo step; edited hits
   are kept as Re-analyze > Keep keeps them). The largest gap between the candidates is marked; "Set in gap" puts the
   slider there (a start value, never set on its own). Loudness: the peak level of every hit (dBFS) with the floor and
   ceiling handles of the velocity map and the 20 to 127 ramp over it; the velocities follow live (velOfDb, the core's
   velocity_of_db) and are stored when the handle is let go (one undo step, as Recompute Velocities). A follower shows
   its loudness histogram only. Both canvases draw only on a change: new data, a control, a resize; nothing per frame. */
const HG = { name: null, p: null, el: null, d: null, lv: new Map(), gen: 0, ghosts: {}, refresh: 0, ro: null, kept: null, draws: 0,
  sens: { base: null, want: null, busy: false, n: null, weak: null },   // base: the events and params at the drag start
  vel: { base: null, drag: null } };
const HG_THUMB = 12;   // px: the slider thumb (style.css .hgRange); the bars sit on the thumb's travel
const HG_GREY = "#4a505c";
function pyRound(x) { return Math.abs(x % 1) === 0.5 ? 2 * Math.round(x / 2) : Math.round(x); }   // Python round: ties to even
function velOfDb(d, lo, hi) {   // crates/core/src/velocity.rs velocity_of_db, the same doubles
  const denom = Math.max(hi - lo, 1e-6), vv = 20 + pyRound((d - lo) / denom * 107);
  return Math.max(20, Math.min(127, vv));
}
function hgSensX(v, W, max) { return HG_THUMB / 2 + (v / max) * (W - HG_THUMB); }
function hgCanvas(c) {   // size the backing store to the element; returns [ctx, W, H] in CSS px
  const W = c.clientWidth, H = c.clientHeight, dpr = window.devicePixelRatio || 1;
  if (c.width !== Math.round(W * dpr) || c.height !== Math.round(H * dpr)) { c.width = Math.round(W * dpr); c.height = Math.round(H * dpr); }
  const ctx = c.getContext("2d"); ctx.setTransform(dpr, 0, 0, dpr, 0, 0); ctx.clearRect(0, 0, W, H);
  HG.draws++;   // the tests check that nothing draws without a change
  return [ctx, W, H];
}
function hgAttach(div, name, p) {   // buildParamsPanel: the controls of this track
  HG.name = name; HG.p = p; HG.d = null; HG.lv = new Map(); HG.kept = null;
  HG.sens = { base: null, want: null, busy: false, n: null, weak: null }; HG.vel = { base: null, drag: null };
  HG.el = { sens: div.querySelector(".pSens"), sensCanvas: div.querySelector(".hgSensCanvas"), sensWrap: div.querySelector(".hgSensWrap"),
    count: div.querySelector(".hgSensCount"), gapBtn: div.querySelector(".hgGapBtn"), loud: div.querySelector(".hgLoudCanvas"),
    loudInfo: div.querySelector(".hgLoudInfo"), vlo: div.querySelector(".pVLo"), vhi: div.querySelector(".pVHi"), vcut: div.querySelector(".pVCut"),
    hint: div.querySelector(".pactions .phint") };
  hgRememberParams(p);
  HG.el.gapBtn.onclick = () => hgSetInGap();
  HG.el.sensWrap.onpointermove = hgSensHover; HG.tip = "";
  div.querySelector(".pGap").title = div.querySelector(".pRefine").title = "Applies on the next re-analyze";
  hgLoudPointer(HG.el.loud);
  if (HG.ro) HG.ro.disconnect();
  HG.ro = new ResizeObserver(() => hgDraw());   // a new Inspector width: draw again (only then)
  HG.ro.observe(HG.el.sensWrap); HG.ro.observe(HG.el.loud);
  hgLoad();
}
function hgApiEvents(name) { return (S.events[name] || []).map(e => ({ t: e.t })); }
async function hgLoad() {   // the data of the shown track: on a track change, after an analysis, an edit or a re-pick
  const name = HG.name, gen = ++HG.gen;
  if (!name || !S.sid || !S.inspector || !S.tracks[name] || S.tracks[name].offline) { HG.d = null; hgDraw(); return; }
  const evs = S.events[name] || [];
  try {
    const d = await B.histograms(S.sid, name, hgApiEvents(name));
    if (gen !== HG.gen || HG.name !== name) return;
    HG.d = d; HG.lv = new Map();
    const st = d.detection.state; HG.kept = st === "ok" ? true : st === "missing" || st === "no_model" ? false : HG.kept;
    evs.forEach((e, i) => HG.lv.set(e.id, d.loudness.levels[i]));
  } catch (e) { if (gen === HG.gen) { HG.d = null; if (!S.peakErr[name]) setMsgBg(`histograms: ${errText(e)}`); } }   // P4-39c: the first track is selected at open; a file the lane already calls unsupported has no second message
  hgDraw();
}
function hgScheduleRefresh(track) {   // refreshCounts: the hits of the shown track changed (not by a drag of this card)
  if (track && track !== HG.name) return;
  if (HG.sens.base || HG.vel.drag) return;
  clearTimeout(HG.refresh); HG.refresh = setTimeout(hgLoad, 250);
}
function hgDetection() { const d = HG.d && HG.d.detection; return d && d.state === "ok" ? d : null; }
function hgDraw() { if (HG.el && HG.el.sens && HG.el.sens.isConnected) { hgDrawSens(); hgDrawLoud(); insSumRefresh(); } }
/* P4-25 (owner): the Sensitivity histogram shows every candidate on the axis of its strength (log scale, strong at the left,
   HG_NB bins), not on the slider value, so a strong group spreads into its own cluster and the gap is a natural empty
   stretch. On = the track colour, off = grey (a bin with both is stacked). Heights: log(1 + count) against the tallest bin.
   The threshold is a line between the weakest candidate that is on and the strongest that is off; the gap marker sits
   between the weakest strong candidate and the strongest weak one (the largest gap of the slider values). */
const HG_NB = 80;
function hgBarHeights(counts, H) {   // log(1 + count) against the tallest bin, 2 px at least, never above the canvas
  const full = Math.max(2, H - 3), top = Math.max(1, ...counts);
  return { heights: counts.map(n => n ? Math.min(full, Math.max(2, Math.round(Math.log1p(n) / Math.log1p(top) * full))) : 0) };
}
function hgSensState() {   // the counts the bars show, per slider value and per strength bin; the line and marker as fractions of the axis
  const d = hgDetection(); if (!d) return null;
  const cur = +HG.el.sens.value, bins = new Array(d.max + 1).fill(0), cands = d.candidates;
  for (const c of cands) bins[c[3]]++;
  let active = 0; for (let v = 0; v <= Math.min(cur, d.max); v++) active += bins[v];
  const ls = cands.map(c => Math.log(Math.max(c[2], 1e-6)));
  const hi0 = ls.length ? Math.max(...ls) : 0, lo0 = ls.length ? Math.min(...ls) : 0, pad = (hi0 - lo0) * 0.03;   // the axis runs from the actual extremes, 3 % margin each side
  const hi = hi0 + pad, lo = lo0 - pad, span = hi - lo;
  const f = l => span > 0 ? (hi - l) / span : 0.5;   // 0 = the strongest, 1 = the weakest
  const sb = new Array(HG_NB).fill(0), son = new Array(HG_NB).fill(0);
  cands.forEach((c, i) => { const b = Math.round(f(ls[i]) * (HG_NB - 1)); sb[b]++; if (c[3] <= cur) son[b]++; });
  const edge = (arr, pick) => arr.length ? pick(...arr) : null;
  const onL = cands.flatMap((c, i) => c[3] <= cur ? [ls[i]] : []), offL = cands.flatMap((c, i) => c[3] > cur ? [ls[i]] : []);
  const a = edge(onL, Math.min), b = edge(offL, Math.max);   // the weakest on, the strongest off
  const lineF = a === null && b === null ? null : a === null ? 0 : b === null ? 1 : (f(a) + f(b)) / 2;
  let gapF = null;
  if (d.gap) {
    const strong = cands.flatMap((c, i) => c[3] < d.gap.from ? [ls[i]] : []), weak = cands.flatMap((c, i) => c[3] > d.gap.to ? [ls[i]] : []);
    if (strong.length && weak.length) gapF = (f(Math.min(...strong)) + f(Math.max(...weak))) / 2;
  }
  return { cur, bins, active, total: cands.length, gap: d.gap, max: d.max, nb: HG_NB, sb, son, lineF, gapF, hi, lo,
    ...hgBarHeights(sb, HG.el.sensCanvas.clientHeight) };
}
function hgStrX(f, W, nb) { return (f * (nb - 1) + 0.5) * (W / nb); }   // the x of a fraction of the strength axis (the bin centres)
const HG_SENS_TIP = "Every candidate hit of this track by its strength (log scale): the strongest at the left, weak ones and bleed at the right. Coloured = detected at this setting, grey = filtered out. The line is the Sensitivity threshold: drag the slider below. The triangle marks the largest gap between two clusters (Massey DRT: mind the gap); click it to snap the slider into the gap.";
function hgSensTitle(bar) {   // the tooltip of the canvas: the explanation, the counts and the thresholds; over a bar that bar's count
  const w = HG.el && HG.el.sensWrap; if (!w) return;
  w.title = (bar || "") + (bar ? "\n" : "") + HG_SENS_TIP + (HG.tip ? `\n${HG.tip}` : "");
}
function hgSensHover(ev) {
  const s = hgSensState(); if (!s) return;
  const r = HG.el.sensCanvas.getBoundingClientRect();
  const b = Math.max(0, Math.min(s.nb - 1, Math.floor((ev.clientX - r.left) / Math.max(1, r.width) * s.nb)));
  const n = s.sb[b], on = s.son[b];
  hgSensTitle(n ? `${n} ${n === 1 ? "candidate" : "candidates"} here: ${on} on, ${n - on} off` : "no candidate here");
}
function hgDrawSens() {
  const el = HG.el, d = hgDetection(), t = S.tracks[HG.name];
  const st = HG.d && HG.d.detection ? HG.d.detection.state : null;
  el.sensWrap.classList.toggle("hgEmpty", !d);
  el.gapBtn.classList.toggle("hidden", !d || !d.gap);
  const [ctx, W, H] = hgCanvas(el.sensCanvas);
  if (!d) {
    if (el.hint && st !== "follows" && !(t && t.follows)) el.hint.textContent = "detection changes apply on the next re-analyze";
    el.count.textContent = st === "follows" ? "" : st === "no_model" ? "the slider applies on the next re-analyze"
      : st === "missing" ? "re-analyze once to see the candidates; until then the slider applies on the next re-analyze" : "";
    return;
  }
  const s = hgSensState(), bw = W / s.nb, color = (t && t.color) || "#4da3ff";
  for (let b = 0; b < s.nb; b++) {
    const h = s.heights[b]; if (!h) continue;
    const n = s.sb[b], onH = s.son[b] ? Math.max(1, Math.round(h * s.son[b] / n)) : 0, x = b * bw + 0.5, w = Math.max(1, bw - 1);
    ctx.fillStyle = HG_GREY; ctx.fillRect(x, H - h, w, h - onH);
    ctx.fillStyle = color; ctx.fillRect(x, H - onH, w, onH);
  }
  if (s.lineF !== null) {   // the threshold: between the weakest candidate that is on and the strongest that is off
    const x = s.lineF === 0 ? 0.5 : s.lineF === 1 ? W - 0.5 : Math.round(hgStrX(s.lineF, W, s.nb)) + 0.5;   // all on: at the left edge; none on: at the right
    ctx.strokeStyle = LN_CSS.getPropertyValue("--accent").trim() || "#4da3ff"; ctx.lineWidth = 1;
    ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, H); ctx.stroke();
  }
  // the marker under the canvas: in the middle of the largest gap; a click snaps the slider into it
  const g = s.gap;
  el.gapBtn.classList.toggle("hidden", s.gapF === null);
  if (g && s.gapF !== null) {
    el.gapBtn.style.left = `${hgStrX(s.gapF, W, s.nb) - 5}px`;
    el.gapBtn.title = `Snap into the gap: ${s.bins.slice(0, g.from).reduce((a, b) => a + b, 0)} hits`;
  }
  const hits = HG.sens.n !== null ? HG.sens.n : (S.events[HG.name] || []).filter(e => !isManual(e)).length;
  let weak = 0; if (g) for (let v = g.from + 1; v <= Math.min(s.cur, s.max); v++) weak += s.bins[v];   // active candidates beyond the gap
  el.count.textContent = `${hits} hits` + (weak ? ` · ${weak} weak` : "");
  HG.tip = `${s.active} of ${s.total} candidates on` + (HG.sens.weak && HG.sens.weak.to != null ? `, retry at ${HG.sens.weak.to}` : "") + `\n${sensReadout(s.cur)}`;
  hgSensTitle();
}
function hgCanRepick() {   // HG.kept: true (the service keeps the activations), false (none: the slider applies on the next
  // re-analyze, as before P4-3), null (not known yet: the first re-pick tells)
  return HG.kept !== false && !analysisRunning && !SH.pending && !(S.tracks[HG.name] || {}).follows;
}
function hgSensInput(name, p) {
  if (name !== HG.name) return;
  if (!hgCanRepick() || HG.gapSet) { hgDrawSens(); return; }   // HG.gapSet: Set in gap stores once, with no preview round
  const s = HG.sens;
  if (!s.base) s.base = hgBase(name);
  if (!s.want || !s.want.store) s.want = { params: { ...p }, store: false };
  hgDrawSens(); hgPump(name);
}
function hgSensCommit(name, p) {
  if (name !== HG.name || !hgCanRepick()) { hgRememberParams(p); return; }
  if (!HG.sens.base) HG.sens.base = hgBase(name);
  HG.sens.want = { params: { ...p }, store: true };
  hgPump(name);
}
function hgBase(name) {   // the hits and the slider of the drag start (the undo step's before)
  return { events: (S.events[name] || []).map(e => ({ ...e })), thresholds: HG.baseParams.thresholds, sensitivity: HG.baseParams.sensitivity };
}
function hgRememberParams(p) { HG.baseParams = { thresholds: (p.thresholds || SENS.BASE).slice(), sensitivity: p.sensitivity }; }
async function hgPump(name) {   // one re-pick at a time; the newest value wins, a store is never dropped
  const s = HG.sens;
  if (s.busy || !s.want) return;
  const w = s.want; s.want = null; s.busy = true;
  try {
    await edIdle(); await settleEvents(name);   // P4-9: the service merges the edited hits it holds
    const r = await B.repick(S.sid, name, w.params, w.store);
    if (HG.name === name && s.base) {
      if (w.store) hgStored(name, r, w.params); else hgPreview(name, r);
    }
  } catch (e) {
    if (e && e.status === 409) {   // no kept activations (or a missing detection, an analysis running): the slider applies on the next re-analyze
      if (/re-analyze it once/.test(e.message)) HG.kept = false; else setMsg(`re-pick: ${errText(e)}`);
      hgRestore(name, true); s.want = null;
    } else { setMsg(`re-pick: ${errText(e)}`); hgRestore(name, false); }
  } finally {
    s.busy = false;
    if (s.want) hgPump(name);
  }
}
function hgTakeLevels(events, lv) {   // the levels of a re-pick for the loudness histogram (aligned with the events)
  if (!HG.d) return;
  HG.lv = lv;
  HG.d.loudness.levels = events.map(e => lv.has(e.id) ? lv.get(e.id) : -Infinity);
}
function hgGone(base, picked) {   // the detected hits of the base that the pick does not have: grey lines in the lane
  const ts = picked.map(e => e.t), out = [];
  let j = 0;
  for (const e of base) {
    if (isManual(e)) continue;
    while (j < ts.length && ts[j] < e.t - 5e-4) j++;
    if (!(j < ts.length && Math.abs(ts[j] - e.t) <= 5e-4)) out.push(e.t);
  }
  return out;
}
function hgPreview(name, r) {
  const s = HG.sens;
  const shown = adoptEvents(r.events, s.base.events), lv = new Map();   // the service merged the edited hits (keep_edited)
  shown.forEach((e, i) => { if (r.levels && Number.isFinite(r.levels[i])) lv.set(e.id, r.levels[i]); });
  S.events[name] = shown;   // shown, not written: the store follows on release
  hgTakeLevels(shown, lv);   // the loudness histogram follows the re-picked hits
  HG.ghosts[name] = hgGone(s.base.events, shown.filter(e => !isManual(e)));
  s.n = r.n_events; s.weak = r.weak_signal;
  if (S.sel && S.sel.track === name) S.sel = null;
  invalidateHits(name); insCounts(); hgDrawSens(); hgDrawLoud();
}
function hgStored(name, r, params) {
  const s = HG.sens, base = s.base, p = HG.p, t = S.tracks[name];
  const after = adoptEvents(r.events, base.events);   // stored by the service, edited hits kept
  S.events[name] = after; delete HG.ghosts[name];
  if (r.weak_signal && r.weak_signal.to != null) {   // the retry of a 0-hit pick: the params the service stored
    p.thresholds = r.weak_signal.thresholds; p.sensitivity = r.weak_signal.to; p.weak_signal = r.weak_signal;
  } else if (r.weak_signal) p.weak_signal = r.weak_signal;
  pushUndo({ type: "repick", track: name,
    before: { events: base.events, thresholds: base.thresholds, sensitivity: base.sensitivity },
    after: { events: after.map(e => ({ ...e })), thresholds: (p.thresholds || []).slice(), sensitivity: p.sensitivity } });
  s.base = null; s.n = null; s.weak = null;
  if (S.sel && S.sel.track === name) S.sel = null;
  setDirtyState(true);   // the service stored the merged hits
  invalidateHits(name); refreshCounts(name); updateStatus();
  const fol = r.entry && Array.isArray(r.entry.followers) ? r.entry.followers : [];
  if (fol.length) reloadFollowers(fol);
  setMsg(`${name}: ${r.n_events} ${r.n_events === 1 ? "hit" : "hits"} at sensitivity ${p.sensitivity}` + (r.weak_signal ? ` (${weakSignalText(r.weak_signal)})` : ""));   // #411: singular for one hit
  if (t === S.tracks[name] && INS.name === name) { hgRememberParams(p); insSensRefresh(p); insCounts(); insSumRefresh(); hgDrawSens(); }   // P4-25: in place, no rebuild of the panel (it flickered)
  hgLoad();
}
function insSensRefresh(p) {   // the slider, its readout and the note after a stored re-pick (the same as buildParamsPanel sets at build)
  const div = $("#insParams"), sens = div.querySelector(".pSens"); if (!sens) return;
  const cur = Array.isArray(p.thresholds) && p.thresholds.length === 5 ? p.thresholds : SENS.BASE;
  sens.value = Number.isFinite(p.sensitivity) ? Math.round(p.sensitivity) : Math.round(sensOf(cur));
  div.querySelector(".pSensVal").textContent = sensReadout(+sens.value);
  const txt = weakSignalText(p.weak_signal), note = div.querySelector(".pSensNote");
  note.textContent = txt; div.querySelector(".pSensNoteRow").classList.toggle("hidden", !txt);
}
function hgRestore(name, keepParams) {   // a failed re-pick: back to the hits (and unless kept, the slider) of the drag start
  const s = HG.sens, base = s.base; if (!base) return;
  S.events[name] = base.events; delete HG.ghosts[name];
  s.base = null; s.n = null; s.weak = null;
  invalidateHits(name);
  if (keepParams) { hgRememberParams(HG.p); hgDrawSens(); return; }
  HG.p.thresholds = base.thresholds; HG.p.sensitivity = base.sensitivity;
  if (INS.name === name) syncInspector(true);
}
function hgSetInGap() {
  const d = hgDetection(); if (!d || !d.gap) return;
  HG.el.sens.value = d.gap.mid;
  HG.gapSet = true;
  try { HG.el.sens.dispatchEvent(new Event("input")); } finally { HG.gapSet = false; }   // the readout and p; no preview round
  HG.el.sens.dispatchEvent(new Event("change"));   // one re-pick, stored
}
/* round 2: the params and maps that undo and redo send go out one after the other per track, so a fast undo and redo
   reach the service in the order they were made */
function hgSerial(track, call) {
  const q = HG.queue || (HG.queue = {});
  const next = (q[track] || Promise.resolve()).then(call).catch(e => setMsg(`${track}: ${errText(e)}`));
  q[track] = next;
  return next;
}
function hgRepickBusy() { return HG.sens.busy || !!HG.sens.want || !!HG.sens.base; }   // a re-pick (or its store) is on its way
function hgUndoWaits(op) {   // round 2: undo and redo of a re-pick step wait while a re-pick is on its way
  if (!op || (op.type !== "repick" && op.type !== "shift") || !hgRepickBusy()) return false;
  setMsg("a re-pick is being stored: undo and redo wait for it");
  return true;
}
/* P4-9: the Hitpoint shift and the hand-edit merge run in the service (track_shift_set; keep_edited of track_repick and
   analyze_start, Rust merge_edited). The page sends its pending edits first, asks, and shows the stored hits. */
function adoptEvents(raw, old) {   // the service's hits with page ids; an edited hit keeps the id of the page's copy (its undo steps)
  const man = (old || []).filter(isManual);
  return normEvents(raw).map(e => { const m = isManual(e) ? man.find(x => Math.abs(x.t - e.t) < 1e-9) : null; e.id = m ? m.id : newId(); return e; });
}
function shiftNow(name) { const t = S.tracks[name]; return Math.round((t && t.params && t.params.shift_samples) || 0); }
async function shiftApply(name, nv) {   // true when the service shifted the track to nv samples; the hits shown are the stored ones
  const sid = S.sid;
  if (hgRepickBusy()) { setMsg("a re-pick is being stored: try the shift again"); return false; }
  try {
    await settleEvents(name);
    const r = await B.shiftSet(sid, name, nv);
    if (sid !== S.sid || !S.tracks[name]) return false;
    const t = S.tracks[name], old = S.events[name] || [], fresh = normEvents(await B.events(sid, name));
    if (fresh.length === old.length) fresh.forEach((e, i) => { e.id = old[i].id; });   // the same hits: the ids (undo, selection) stay
    S.events[name] = fresh;
    t.params = t.params || defaultParams(t.role); t.params.shift_samples = r.shift_samples;
    if (S.sel && S.sel.track === name && S.sel.index >= fresh.length) S.sel = null;
    setDirtyState(true); invalidateHits(name); refreshCounts(name); updateStatus();
    reloadFollowers(followersOf(name));
    if (INS.name === name) {   // P4-25: in place, no panel rebuild (a rebuild took the focus of the stepper keys and flickered)
      const f = document.querySelector("#insParams .pShift");
      if (f && SH.pending <= 1) f.value = shiftNow(name);   // undo and redo show the value; a newer step in the queue keeps its own
      insSumRefresh();
    }
    return true;
  } catch (e) { setMsg(`shift: ${errText(e)}`); return false; }
}
async function shiftApplyOp(op, reverse) {   // applyOp: undo and redo of a shift (runs in the queue)
  const to = reverse ? op.from : op.to;
  const ok = await hgSerial(op.track, () => shiftApply(op.track, to));
  if (ok) setMsg(`${reverse ? "undo" : "redo"}: shift of ${op.track} to ${to} samples`);
  return !!ok;
}
function hgApplyRepickOp(op, reverse) {   // applyOp: undo and redo of a stored re-pick
  const st = reverse ? op.before : op.after, t = S.tracks[op.track];
  S.events[op.track] = st.events.map(e => ({ ...e }));
  if (t) {
    t.params = t.params || defaultParams(t.role); t.params.thresholds = st.thresholds;
    if (st.sensitivity === undefined) delete t.params.sensitivity; else t.params.sensitivity = st.sensitivity;
    // fix round: the service's params follow, so a Save writes the slider of the hits it shows; round 2: in order
    const sid = S.sid, params = JSON.parse(JSON.stringify(t.params));
    hgSerial(op.track, () => B.paramsSet(sid, op.track, params));
  }
  if (S.sel && S.sel.track === op.track) S.sel = null;
  markDirty(op.track); invalidateHits(op.track); refreshCounts(op.track); updateStatus();
  if (INS.name === op.track) syncInspector(true);
}

/* #232: an Inspector number that is stored at once (the refine window): one track_params_set, one undo step. */
const PARAM_FIELD_DEFAULTS = { refine_lo_ms: -2, refine_hi_ms: 12 };   // the shown defaults (RefineWindow::DEFAULT)
function paramsFieldCommit(name, key, value) {
  const t = S.tracks[name]; if (!t) return value;
  const before = JSON.parse(JSON.stringify(t.params || {}));
  if (before[key] === undefined) before[key] = PARAM_FIELD_DEFAULTS[key];   // the shown default is the undo's before
  t.params = t.params || defaultParams(t.role);
  t.params[key] = value;
  pushUndo({ type: "params", label: "Detection setting", sid: S.sid, track: name,
    before: { params: before }, after: { params: JSON.parse(JSON.stringify(t.params)) } });
  const sid = S.sid;
  hgSerial(name, () => B.paramsSet(sid, name, JSON.parse(JSON.stringify(t.params))))
    .then(() => setDirtyState(true));
  return value;
}
function hgApplyParamsOp(op, reverse) {   // applyOp: undo and redo of a stored params change (the refine window)
  if (op.sid !== S.sid) { setMsg("that project is not the open one"); return; }
  const st = reverse ? op.before : op.after, t = S.tracks[op.track];
  if (t) {
    t.params = JSON.parse(JSON.stringify(st.params));
    hgSerial(op.track, () => B.paramsSet(op.sid, op.track, JSON.parse(JSON.stringify(t.params))));
  }
  setDirtyState(true);
  if (INS.name === op.track) syncInspector(true);
}

/* the loudness histogram: dBFS on x, the floor and ceiling handles, the velocity ramp, and the loudness cut (controller
   decision 4, DRT's Loudness slider filters): a third handle, off at the left edge. A detected hit below it is grey in
   the histogram and in the lane and leaves the output (MIDI, spikes, Export All, the click, a follower); it stays in the
   document (e.cut). An edited hit is never cut. */
function hgCutOf(p) { return Number.isFinite(p.vel_cut_db) ? p.vel_cut_db : null; }
function hgIsCut(e) { return !!e.deactivated || ((!!e.cut || !!e.bleed || !!e.filtered) && !isManual(e) && e.keep !== true); }   // P4-14: a hit of the tom bleed rule shows as a cut hit (owner: tom-bleed-show); P4-6c: so does one the Learn filter took out; #242: the user's deactivated wins, the user's keep holds the hit in
function hgCutOnly(e) { return !!e.deactivated || ((!!e.cut || !!e.bleed) && !isManual(e) && e.keep !== true); }   // the cut and bleed marks without the Learn filter's (its drag preview decides that one)
function hgLoudRange() {   // P4-25: the actual dBFS range of the hits plus 2 dB, widened to hold the floor, the ceiling and the cut handles (at least 12 dB)
  if (HG.vel.drag && HG.vel.drag.range) return HG.vel.drag.range;   // a drag keeps its axis: the handle under the pointer must not move away
  const lv = (HG.d ? HG.d.loudness.levels : []).filter(Number.isFinite), c = hgCutOf(HG.p);
  const pts = [...lv, HG.p.vel_lo_db, HG.p.vel_hi_db, ...(c === null ? [] : [c])].filter(Number.isFinite);
  let lo = Math.max(-96, Math.floor(Math.min(...pts)) - 2), hi = Math.min(0, Math.ceil(Math.max(...pts)) + 2);
  if (hi - lo < 12) { lo = Math.max(-96, hi - 12); hi = Math.min(0, lo + 12); }
  const r = { lo, hi };
  if (HG.vel.drag) HG.vel.drag.range = r;
  return r;
}
function hgLoudX(db, W, r) { return 6 + (db - r.lo) / (r.hi - r.lo) * (W - 12); }
function hgLoudDb(x, W, r) { return r.lo + (x - 6) / (W - 12) * (r.hi - r.lo); }
function hgLoudState() {
  if (!HG.d) return null;
  const r = hgLoudRange(), bins = new Array(r.hi - r.lo + 1).fill(0), cutBins = new Array(r.hi - r.lo + 1).fill(0);
  const cut = hgCutOf(HG.p), evs = S.events[HG.name] || [];
  let nCut = 0;
  HG.d.loudness.levels.forEach((l, i) => {
    const b = Math.max(0, Math.min(bins.length - 1, Math.floor(l) - r.lo));
    bins[b]++;
    const e = evs[i];
    if (cut !== null && l < cut && !(e && isManual(e)) && !(e && e.keep === true)) { cutBins[b]++; nCut++; }   // #242: a hit the user keeps is never counted as cut
  });
  return { range: r, bins, cutBins, n: HG.d.loudness.levels.length, nCut, floor: HG.p.vel_lo_db, ceiling: HG.p.vel_hi_db, cut };
}
function hgDrawLoud() {
  const c = HG.el.loud, st = hgLoudState(), t = S.tracks[HG.name];
  const [ctx, W, H] = hgCanvas(c);
  if (!st) { HG.el.loudInfo.textContent = ""; return; }
  const { range: r, bins, cutBins } = st, top = Math.max(1, ...bins), bw = Math.max(1, (W - 12) / bins.length);
  const color = (t && t.color) || "#4da3ff";
  for (let i = 0; i < bins.length; i++) {
    if (!bins[i]) continue;
    const h = Math.max(2, Math.round(Math.sqrt(bins[i] / top) * (H - 12))), x = hgLoudX(r.lo + i, W, r);
    const hc = cutBins[i] ? Math.round(h * cutBins[i] / bins[i]) : 0;   // the cut part of the bar is grey
    ctx.fillStyle = color; ctx.fillRect(x, H - h, bw, h - hc);
    if (hc) { ctx.fillStyle = HG_GREY; ctx.fillRect(x, H - hc, bw, hc); }
  }
  const yv = v => H - 2 - (v / 127) * (H - 12);
  const xf = hgLoudX(st.floor, W, r), xc = hgLoudX(st.ceiling, W, r);
  ctx.strokeStyle = "#cdd3de"; ctx.lineWidth = 1.5; ctx.beginPath();   // the velocity ramp 20 .. 127
  ctx.moveTo(6, yv(20)); ctx.lineTo(xf, yv(20)); ctx.lineTo(xc, yv(127)); ctx.lineTo(W - 6, yv(127)); ctx.stroke();
  const handle = (x, lab, col) => {
    ctx.strokeStyle = col; ctx.lineWidth = 2; ctx.beginPath(); ctx.moveTo(x, 8); ctx.lineTo(x, H); ctx.stroke();
    ctx.fillStyle = col; ctx.beginPath(); ctx.moveTo(x - 5, 0); ctx.lineTo(x + 5, 0); ctx.lineTo(x, 8); ctx.closePath(); ctx.fill();
    ctx.fillStyle = "#7d8593"; ctx.font = "9px sans-serif"; ctx.fillText(lab, Math.min(W - 18, x + 4), 9);
  };
  handle(xf, "20", "#4da3ff"); handle(xc, "127", "#4da3ff");
  const xcut = hgLoudX(st.cut === null ? r.lo : st.cut, W, r);
  if (st.cut !== null) { ctx.fillStyle = "rgba(0,0,0,0.35)"; ctx.fillRect(6, 0, xcut - 6, H); }   // the cut region
  handle(xcut, st.cut === null ? "cut" : "", "#ff9f43");
  HG.el.loudInfo.textContent = `${st.n} hits · vel 20 at ${st.floor} dB · 127 at ${st.ceiling} dB · ` +
    (st.cut === null ? "no cut" : `${st.nCut} cut below ${st.cut} dB`) + ` · axis ${r.lo} to 0 dBFS`;
}
function hgLoudPointer(c) {
  c.addEventListener("pointerdown", ev => {
    const st = hgLoudState(); if (!st || analysisRunning) return;
    const rect = c.getBoundingClientRect(), W = rect.width, x = ev.clientX - rect.left;
    const near = [["lo", hgLoudX(st.floor, W, st.range)], ["hi", hgLoudX(st.ceiling, W, st.range)],
      ["cut", hgLoudX(st.cut === null ? st.range.lo : st.cut, W, st.range)]].sort((a, b) => Math.abs(x - a[1]) - Math.abs(x - b[1]));
    HG.vel.drag = { which: near[0][0], range: st.range, W, left: rect.left };
    HG.vel.base = hgLoudBase(HG.name);
    c.setPointerCapture(ev.pointerId);
    hgVelMove(ev.clientX);
    ev.preventDefault();
  });
  c.addEventListener("pointermove", ev => { if (HG.vel.drag) hgVelMove(ev.clientX); });
  const up = () => { if (!HG.vel.drag) return; HG.vel.drag = null; hgVelCommit(HG.name, HG.p, HG.p.vel_lo_db, HG.p.vel_hi_db, hgCutOf(HG.p), true); };
  c.addEventListener("pointerup", up); c.addEventListener("pointercancel", up);
}
function hgLoudBase(name) {   // the floor, ceiling, cut, velocities and cut marks before a change (the undo step's before)
  const p = HG.name === name ? HG.p : S.tracks[name].params;
  return { lo: p.vel_lo_db, hi: p.vel_hi_db, cut: hgCutOf(p), v: new Map((S.events[name] || []).map(e => [e.id, [e.v, !!e.cut]])) };
}
function hgVelMove(clientX) {   // a handle follows the pointer in whole dB; the velocities and cut marks follow live, not written
  const dr = HG.vel.drag, p = HG.p;
  const db = Math.round(hgLoudDb(clientX - dr.left, dr.W, dr.range));
  if (dr.which === "lo") p.vel_lo_db = Math.max(dr.range.lo, Math.min(p.vel_hi_db - 1, db));
  else if (dr.which === "hi") p.vel_hi_db = Math.min(0, Math.max(p.vel_lo_db + 1, db));
  else p.vel_cut_db = db <= dr.range.lo ? null : Math.min(0, db);   // at the left edge: off
  hgShowLoudFields();
  hgLiveVelocities();
  hgDrawLoud();
}
function hgShowLoudFields() {
  const p = HG.p;
  HG.el.vlo.value = p.vel_lo_db; HG.el.vhi.value = p.vel_hi_db;
  HG.el.vcut.value = hgCutOf(p) === null ? "" : hgCutOf(p);
}
function hgLiveVelocities() {
  const name = HG.name, p = HG.p, cut = hgCutOf(p);
  for (const e of S.events[name] || []) {
    const d = HG.lv.get(e.id);
    if (d === undefined) continue;
    e.v = velOfDb(d, p.vel_lo_db, p.vel_hi_db);
    if (cut !== null && d < cut && !isManual(e) && e.keep !== true) e.cut = true; else delete e.cut;   // #242: a hit the user keeps is never marked
  }
  invalidateHits(name); updateStatus();
}
async function hgVelCommit(name, p, lo, hi, cut, fromDrag) {   // a handle let go, or a number field: store the map, the cut and the velocities
  if (!Number.isFinite(lo) || !Number.isFinite(hi) || (cut !== null && !Number.isFinite(cut))) { if (HG.name === name) hgShowLoudFields(); return; }
  if (analysisRunning) {   // fix round: as the handles, the number fields wait for the analysis (the service answers 409 too)
    setMsg("velocity mapping: wait for the analysis to finish");
    if (HG.name === name) hgShowLoudFields();
    return;
  }
  const base = fromDrag && HG.vel.base ? HG.vel.base : hgLoudBase(name);
  HG.vel.base = null;
  p.vel_lo_db = lo; p.vel_hi_db = hi;
  if (cut === null) delete p.vel_cut_db; else p.vel_cut_db = cut;
  if (HG.el && HG.name === name) hgShowLoudFields();
  try {
    await edIdle();   // P4-9 stage 2: a hit edit on its way is stored first
    const sent = S.events[name] || [];
    const r = await B.velocitySet(S.sid, name, lo, hi, cut, sent.map(e => ({ t: e.t, id: e.id, ...(e.added ? { added: true } : {}), ...(e.orig !== undefined ? { orig: e.orig } : {}) })));
    if (r.follows) { S.events[name] = normEvents(r.events); invalidateHits(name); refreshCounts(name); }
    else {
      const op = { type: "loud", track: name, before: { lo: base.lo, hi: base.hi, cut: base.cut, hits: {} }, after: { lo, hi, cut, hits: {} } };
      r.events.forEach((e, i) => {
        const o = sent[i];
        if (!o || o.id !== e.id) return;
        const b = base.v.get(o.id) || [o.v, !!o.cut];
        op.before.hits[o.id] = b; op.after.hits[o.id] = [e.v, !!e.cut];
      });
      hgApplyLoudOp(op, false, true); pushUndo(op);
    }
    setMsg(`${name}: velocity 20 at ${lo} dB, 127 at ${hi} dB` + (cut === null ? "" : `, hits below ${cut} dB cut`));
  } catch (e) {
    setMsg(`velocities: ${errText(e)}`);
    // round 2: nothing was stored: the floor, ceiling and cut, the fields and the live velocities go back
    p.vel_lo_db = base.lo; p.vel_hi_db = base.hi;
    if (base.cut === null) delete p.vel_cut_db; else p.vel_cut_db = base.cut;
    for (const ev of S.events[name] || []) {
      const b = base.v.get(ev.id);
      if (!b) continue;
      ev.v = b[0]; if (b[1]) ev.cut = true; else delete ev.cut;
    }
    if (HG.el && HG.name === name) hgShowLoudFields();
    invalidateHits(name); updateStatus();
  }
  if (HG.name === name) hgDrawLoud();
}
function hgApplyLoudOp(op, reverse, initial) {   // applyOp of a "loud" step: the floor, ceiling and cut, the velocities and cut marks
  const st = reverse ? op.before : op.after, t = S.tracks[op.track];
  if (!t) return;
  t.params = t.params || defaultParams(t.role);
  t.params.vel_lo_db = st.lo; t.params.vel_hi_db = st.hi;
  if (st.cut === null) delete t.params.vel_cut_db; else t.params.vel_cut_db = st.cut;
  for (const e of S.events[op.track] || []) {
    const h = st.hits[e.id];
    if (!h) continue;
    e.v = h[0];
    if (h[1]) e.cut = true; else delete e.cut;
  }
  if (!initial && B.velocitySet) { const sid = S.sid; hgSerial(op.track, () => B.velocitySet(sid, op.track, st.lo, st.hi, st.cut, [])); }   // undo, redo: the stored map follows, in order
  markDirty(op.track); invalidateHits(op.track); refreshCounts(op.track); updateStatus();
  if (HG.name === op.track && HG.el) { hgShowLoudFields(); hgDrawLoud(); }
}
/* the test hook: what the histograms show (the e2e tests read the counts, the threshold and the handle positions) */
window.DSHist = {
  state() {
    const s = HG.el && HG.el.sens ? hgSensState() : null, l = HG.el ? hgLoudState() : null;
    const sw = HG.el ? HG.el.sensCanvas.clientWidth : 0, lw = HG.el ? HG.el.loud.clientWidth : 0;
    return { name: HG.name, detection: HG.d ? HG.d.detection.state : null, sens: s && { ...s, x: hgSensX(s.cur, sw, s.max), lineX: s.lineF === null ? null : hgStrX(s.lineF, sw, s.nb), gapX: s.gapF === null ? null : hgStrX(s.gapF, sw, s.nb), width: sw },
      hits: HG.sens.n, ghosts: (HG.ghosts[HG.name] || []).length, busy: HG.sens.busy || !!HG.sens.want,
      loud: l && { ...l, xFloor: hgLoudX(l.floor, lw, l.range), xCeiling: hgLoudX(l.ceiling, lw, l.range),
        xCut: hgLoudX(l.cut === null ? l.range.lo : l.cut, lw, l.range), width: lw },
      text: HG.el ? HG.el.count.textContent : "", draws: HG.draws, kept: HG.kept };
  },
  velOfDb, pyRound, sensX: hgSensX, loudX: hgLoudX, barHeights: hgBarHeights, strX: hgStrX,
};
