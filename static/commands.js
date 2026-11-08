/* Drum Spike v2 — editor commands (refactor plan step 5.2, R9 A2)
   One function per command of the native menu: add, delete, toggle and nudge hits (one hit or the range
   selection), the nudge value, the amplitude zoom, the zoom presets and the Zoom Toggle, the row heights,
   track selection, Go to, Play/Stop, follow, snap, click and Audition on Tab, Settings and Keyboard
   Shortcuts. Moved verbatim from app.js. Plain script (no build step), loaded after wave.js and before
   app.js. The names stay top-level bindings (page globals). */
"use strict";

/* ---------------- editor commands (P3-13a) ----------------
   One function per command of the native menu (shell.js `handlers`); the keys and the toolbar call the same
   function, so a menu item, its key and its button do the same thing. */
function addHitAt(name, t, snap) {   // hand-added → manual (magenta): events_edit add, one undo step; #144: snap puts it on the attack near t
  if (followerLocked(name)) return;   // P3-18
  const id = newId();
  return edCall(name, () => {
    const pl = { ops: [{ op: "add", t, ...(snap ? { snap: true } : {}) }], make: [[t, id]], sel: id, step: { type: "add", track: name, id, t, v: 100 } };
    if (snap) pl.done = r => {   // the step and the page id follow the hit to where it landed; the redo adds it there, without a new snap
      const s = r && r.snaps && r.snaps[0];
      if (!s) return;
      pl.make = [[s.t, id]]; pl.step.t = s.t; pl.step.v = s.v;
      const sr = S.tracks[name].sr || 48000, d = Math.round((s.t - s.click) * sr);
      pl.msg = s.snapped ? `added hit on the attack (${d >= 0 ? "+" : ""}${d} samples from the click), velocity ${s.v}`
        : "no attack within 10 ms: the hit stays at the click, velocity 100";
    };
    return pl;
  });
}
function needTrack() {   // the selected track (header click), or a message
  if (!needProject()) return null;
  if (!S.selTrack || !S.tracks[S.selTrack]) { setMsg("no track selected: click a track header first"); return null; }
  return S.selTrack;
}
function addHitAtPlayhead() { const n = needTrack(); if (n) addHitAt(n, snapSample(S.playhead, n)); }
function deleteSelectedHit() {
  if (LN.sel) return rangeDelete();   // #151: a range selection takes the key: every hit of the range, one undo step
  if (!S.sel) { setMsg("no hit selected"); return; }
  const trk = S.sel.track, e = (S.events[trk] || [])[S.sel.index];
  if (followerLocked(trk)) return;   // P3-18
  if (e) return edDelete(trk, e.id);
}
/* #242: Hits > Toggle Hit Active (X). The selected hit flips between active and deactivated: a
   deactivated one (the loudness cut, the tom bleed rule, the Learn filter or the user's own
   mark) gets `keep: true` — the user's override that wins over every automatic mark — and an
   active one gets `deactivated: true`, the explicit per-hit user mark. The flags ride in the
   hit (edit.rs `toggle` op), so export, MIDI, spikes and the click follow (policy::is_out), the
   lane drawing, the info line and the Tab navigation follow (hgIsCut), and the choice survives
   a re-analyze (a kept hit of merge_edited), a cut or bleed threshold change and a Learn
   change. One toggle is one undo step: the step records the flags the hit had, the undo sends
   them back as `was` (byte-identical, R05). */
function toggleHitActive() {
  if (!S.sel) { setMsg("no hit selected"); return; }
  const trk = S.sel.track, id = ((S.events[trk] || [])[(S.sel || {}).index] || {}).id;
  if (followerLocked(trk)) return;   // P3-18
  if (id === undefined) { setMsg("no hit selected"); return; }
  return shSerial(async () => {
    let stored = [];   // the service's hits: the step keeps the flags of the hit as they are stored
    try {
      if (S.sid && (S.events[trk] || []).some(x => x.id === id)) { if (hasPendingWrite(trk)) await settleEvents(trk); const r = await B.events(S.sid, trk); stored = Array.isArray(r) ? r : (r && r.events) || []; }
    } catch (e) { stored = []; }
    return edNow(trk, evs => {
      const j = edIndex(evs, id);
      if (j < 0) { setMsg("the hit is not there any more"); return null; }
      const e = evs[j];
      const raw = stored.find(x => Math.abs(x.t - e.t) < 1e-9) || e;
      const to = hgIsCut(e) ? { keep: true, deactivated: false } : { keep: false, deactivated: true };
      return { ops: [{ op: "toggle", index: j, keep: to.keep, deactivated: to.deactivated }], sel: id,
        step: { type: "toggle", track: trk, id, t: e.t, to, was: hitWas(raw) } };
    });
  });
}
/* #151 (workflow polish rank 3): Delete and the nudge keys act on every hit of the range selection, so a bleed
   burst or a whole fill is one action, not one per hit. One events_edit call with one op per hit, one undo step
   (del-many, move-many — the shape of setv-many): the step holds every hit (id, t, v, added, orig, keep), so one
   undo puts the document back byte-identical (the keep fields of the stored hits, as in edDelete). Every hit of
   the range counts, a cut or bleed one too: the range is aimed at what the lane shows. No hit in the range: a
   message, no step. A follower is refused (its hits are derived, P3-18). The range stays after the edit. */
async function rangeDelete() {
  const s = LN.sel, trk = s.track;
  if (followerLocked(trk)) return;
  let stored = [];   // the service's hits: the step keeps the fields of each hit as they are stored
  try {
    if (S.sid && (S.events[trk] || []).length) { if (hasPendingWrite(trk)) await settleEvents(trk); const r = await B.events(S.sid, trk); stored = Array.isArray(r) ? r : (r && r.events) || []; }
  } catch (e) { stored = []; }
  return edCall(trk, evs => {
    const pool = stored.slice(), list = [];
    for (const e of evs) {
      if (e.t < s.t0 || e.t > s.t1) continue;
      const j = edIndex(evs, e.id);
      if (j < 0) continue;
      const raw = pool.find(x => Math.abs(x.t - e.t) < 1e-9);
      if (raw) pool.splice(pool.indexOf(raw), 1);
      list.push({ id: e.id, t: e.t, v: e.v, added: !!e.added, ...(e.orig !== undefined ? { orig: e.orig } : {}), keep: delKeep(raw) });
    }
    if (!list.length) { setMsg(`no hits in the range (${fmt(s.t0, 3)} to ${fmt(s.t1, 3)} s)`); return null; }
    return { ops: list.map(x => ({ op: "delete", index: edIndex(evs, x.id) })), drop: list.map(x => x.id), sel: null,
      msg: `deleted ${list.length} ${list.length === 1 ? "hit" : "hits"} of the range`,
      step: { type: "del-many", track: trk, list } };
  });
}
/* P3-20a (A1 p. 28 "Change Nudge value"): the nudge value of the arrow, comma and period keys. 1, 10, 100 samples, 1 ms, 10 ms
   (ms at the rate of the hit's track, rounded to a sample, at least one). A view setting (app setting "nudge_value"). */
const NUDGE_VALUES = { "1smp": { label: "1 smp", smp: () => 1 }, "10smp": { label: "10 smp", smp: () => 10 }, "100smp": { label: "100 smp", smp: () => 100 },
  "1ms": { label: "1 ms", smp: sr => Math.max(1, Math.round(sr * 0.001)) }, "10ms": { label: "10 ms", smp: sr => Math.max(1, Math.round(sr * 0.01)) } };
const NUDGE_ORDER = Object.keys(NUDGE_VALUES);
function nudgeSamples(sr) { return NUDGE_VALUES[S.nudgeValue].smp(sr || 48000); }
function setNudgeValue(key, save) {
  if (!NUDGE_VALUES[key]) return false;
  S.nudgeValue = key;
  const sel = $("#nudgeSel"); if (sel) sel.value = key;
  if (save !== false && B.appSettingsSet) B.appSettingsSet("nudge_value", key).catch(e => setMsg(`settings: ${errText(e)}`));
  DSShell.sync();
  return true;
}
function chooseNudgeValue(key) { if (setNudgeValue(key)) setMsg(`nudge value: ${NUDGE_VALUES[key].label}`); }   // Edit > Nudge Value
function stepNudgeValue(dir) {   // Option+Cmd+numpad plus (+1) and minus (-1)
  const i = NUDGE_ORDER.indexOf(S.nudgeValue) + dir;
  if (i < 0 || i >= NUDGE_ORDER.length) { setMsg(`nudge value: ${NUDGE_VALUES[S.nudgeValue].label} is the ${dir > 0 ? "largest" : "smallest"}`); return; }
  chooseNudgeValue(NUDGE_ORDER[i]);
}
function nudgeUnits(units) {   // the selected hit by units x the nudge value (negative = earlier); a range selection moves all its hits (#151)
  if (LN.sel) return rangeNudge(units);
  if (!S.sel) { setMsg("no hit selected"); return; }
  nudgeSelected(units * nudgeSamples(S.tracks[S.sel.track].sr));
}
function rangeNudge(units) {   // #151: every hit of the range by units x the nudge value, one call, one undo step (move-many)
  const s = LN.sel, trk = s.track, sr = S.tracks[trk] && S.tracks[trk].sr;
  if (followerLocked(trk)) return;
  if (!sr) { setMsg(`${trk}: the track has no sample rate`); return; }
  return edCall(trk, evs => {
    const list = [];
    let inRange = 0;
    for (const e of evs) {
      if (e.t < s.t0 || e.t > s.t1) continue;
      inRange++;
      const to = Math.max(0, Math.round((e.t + units * nudgeSamples(sr) / sr) * sr) / sr);
      if (to === e.t) continue;
      const j = edIndex(evs, e.id);
      if (j < 0) continue;
      list.push({ id: e.id, from: e.t, to, was: hitWas(e) });
    }
    if (!inRange) { setMsg(`no hits in the range (${fmt(s.t0, 3)} to ${fmt(s.t1, 3)} s)`); return null; }
    if (!list.length) { setMsg("the hits of the range did not move"); return null; }
    return { ops: list.map(x => ({ op: "move", index: edIndex(evs, x.id), t: x.to })), drop: list.map(x => x.id), make: list.map(x => [x.to, x.id]),
      msg: `nudged ${list.length} ${list.length === 1 ? "hit" : "hits"} of the range by ${NUDGE_VALUES[S.nudgeValue].label}`,
      step: { type: "move-many", track: trk, list } };
  });
}
/* P3-20a (A1 p. 33, 34 "Waveform zoom"): the amplitude zoom of every wave lane, 1x to 64x in steps of two, a display gain only
   (peaks, hits and exports stay). A clipped peak draws at the lane edge (the lane clips). A view setting ("amp_zoom"). */
const AMP_STEPS = [1, 2, 4, 8, 16, 32, 64];
function setAmpZoom(v, save) {
  if (!AMP_STEPS.includes(v)) return false;
  const changed = S.ampZoom !== v;
  S.ampZoom = v;
  const el = $("#ampLbl"); if (el) { el.textContent = `Amp ${v}x`; el.classList.toggle("hidden", v === 1); }
  if (changed) { GLQ.box = ""; invalidate(F.VIEW | F.GL); }
  if (changed && save !== false && B.appSettingsSet) B.appSettingsSet("amp_zoom", v).catch(e => setMsg(`settings: ${errText(e)}`));
  DSShell.sync();
  return true;
}
function stepAmpZoom(dir) {   // In (+1) doubles, Out (-1) halves
  const i = AMP_STEPS.indexOf(S.ampZoom) + dir;
  if (i < 0 || i >= AMP_STEPS.length) { setMsg(`amplitude zoom: ${S.ampZoom}x is the ${dir > 0 ? "largest" : "smallest"}`); return; }
  setAmpZoom(AMP_STEPS[i]); setMsg(`amplitude zoom: ${S.ampZoom}x`);
}
function nudgeSelected(step) {   // step in samples, negative = earlier; the time is read when the call runs (two quick nudges add up)
  if (!S.sel) { setMsg("no hit selected"); return; }
  const trk = S.sel.track, i = S.sel.index, sr = S.tracks[trk].sr;
  return setHitTime(trk, i, from => from + step/sr, "Nudge Hit");
}
function setHitTime(trk, i, t, label) {   // move hit i of the track to the sample nearest to t (a number, or a function of the hit's time): events_edit move, one undo step (nudge, info line)
  if (followerLocked(trk)) return;   // P3-18
  const e = (S.events[trk] || [])[i];
  if (!e) return;
  const id = e.id, sr = S.tracks[trk].sr;
  return edCall(trk, evs => {
    const j = edIndex(evs, id);
    if (j < 0) return null;
    const from = evs[j].t, to = Math.max(0, Math.round((typeof t === "function" ? t(from) : t) * sr) / sr);
    if (to === from) return null;
    return { ops: [{ op: "move", index: j, t: to }], drop: [id], make: [[to, id]], sel: S.sel && S.sel.track === trk ? id : undefined,
      step: { type: "move", track: trk, id, from, to, ...(label ? { label } : {}), was: hitWas(evs[j]) } };
  });
}
function zoomStep(inward) { flashBtn(document.querySelector(`.zBtn[data-z="${inward ? "in" : "out"}"]`)); zoomPlayhead(inward ? 0.5 : 2); }
function zoomPreset(i) {   // 0 = Full, 1 = 30 s, 2 = 5 s, 3 = 500 ms, 4 = 50 ms, 5 = 5 ms (the toolbar buttons, largest span first)
  const opts = [...document.querySelectorAll("#zoomSel option[value]:not([value=''])")]   // P3-14e: the dropdown, largest span first
    .map(o => o.value);
  const span = opts[i];
  if (span === undefined) return false;
  flashBtn($("#zoomSel")); applyZoomSpan(span);
  return true;
}
function applyZoomSpan(v) {   // a preset span ("full" or seconds), centred on the playhead
  const span = v === "full" ? maxDur() : parseFloat(v);
  S.followHold = true;   // F1: user view change
  setView(S.playhead - span/2, S.playhead + span/2);
}
/* #169: View > Show Overview: the overview strip is shown or hidden; the lanes take its height (the layout, then the
   ResizeObserver of P3-14d). The choice is the app setting overview. */
function setOverview(on, save) {
  S.overviewOn = !!on;
  $("#overviewWrap").classList.toggle("hidden", !S.overviewOn);
  invalidate(F.SIZE | F.VIEW);
  if (save !== false) {
    if (B.appSettingsSet) B.appSettingsSet("overview", S.overviewOn).catch(e => setMsg(`settings: ${errText(e)}`));
    setMsg(`overview ${S.overviewOn ? "shown" : "hidden"}`);
  }
  if (window.DSShell && DSShell.sync) DSShell.sync();
}
/* #169: View > Zoom Presets > Add Current and Organize: the engineer's own spans, the app setting zoom_presets (a list
   of {name, span}, span in seconds). Each one is an entry of the zoom dropdown after the six presets, and of the menu:
   the first five with Option+1 to Option+5 (sync_zoom_own in menu/sync.rs reads the names of the page state). */
function renderZoomOwn() {   // the dropdown entries of the own presets
  const sel = $("#zoomSel");
  for (const o of sel.querySelectorAll("option[data-own]")) o.remove();
  for (const p of S.zoomOwn) {
    const o = document.createElement("option");
    o.value = String(p.span); o.dataset.own = "1"; o.textContent = p.name;
    sel.appendChild(o);
  }
}
function saveZoomOwn() {   // the list, the dropdown and the menu follow
  renderZoomOwn();
  if (B.appSettingsSet) B.appSettingsSet("zoom_presets", S.zoomOwn).catch(e => setMsg(`settings: ${errText(e)}`));
  if (window.DSShell && DSShell.sync) DSShell.sync();
}
function applyOwnZoom(n) {   // an own preset: the menu item (Option+1 to Option+5 for the first five) or the dropdown
  const p = S.zoomOwn[n];
  if (!p) return false;
  flashBtn($("#zoomSel")); applyZoomSpan(String(p.span));
  return true;
}
/* the zoom preset dialogs: one modal for Add Current and Organize. fill(body) builds the body, accept() returns true
   when the dialog may close; Cancel and Escape close it too (cancel: false hides the Cancel button). */
function presetDialog(title, hint, fill, accept, cancel) {
  const m = $("#presetModal"), body = $("#presetBody"), ok = $("#presetAccept"), no = $("#presetCancel");
  $("#presetTitle").textContent = title; $("#presetHint").textContent = hint || "";
  body.textContent = ""; fill(body);
  no.classList.toggle("hidden", !cancel);
  const close = () => { m.classList.add("hidden"); document.removeEventListener("keydown", esc, true); };
  const esc = ev => { if (ev.key === "Escape") { ev.preventDefault(); close(); } };
  ok.onclick = () => { if (accept()) close(); };
  no.onclick = close;
  document.addEventListener("keydown", esc, true);
  m.classList.remove("hidden");
  const first = body.querySelector("input") || ok;
  first.focus();
}
function zoomPresetAdd() {   // File: View > Zoom Presets > Add Current: the span of the view under a name
  const span = S.view.t1 - S.view.t0;
  presetDialog("Add Current Span", `The span is ${fmt(span, 3)} s. The name shows in the zoom dropdown and in View > Zoom Presets.`, body => {
    const inp = document.createElement("input");
    inp.type = "text"; inp.id = "presetName"; inp.maxLength = OWN_ZOOM_NAME_MAX; inp.value = `Span ${S.zoomOwn.length + 1}`;
    inp.onkeydown = ev => { if (ev.key === "Enter") { ev.preventDefault(); $("#presetAccept").click(); } };
    body.appendChild(inp);
  }, () => {
    const name = $("#presetName").value.trim().slice(0, OWN_ZOOM_NAME_MAX);
    if (!name) { $("#presetName").focus(); return false; }
    S.zoomOwn = [...S.zoomOwn, { name, span }];
    saveZoomOwn(); setMsg(`zoom preset "${name}" saved`);
    return true;
  }, true);
}
function zoomPresetOrganize() {   // View > Zoom Presets > Organize: rename and remove (each change is saved at once)
  presetDialog("Organize Zoom Presets", S.zoomOwn.length ? "Rename a preset here or remove it. The first five keep Option+1 to Option+5." : "No own presets yet: Add Current saves the span of the view.",
    body => renderPresetRows(body), () => true, false);
}
function renderPresetRows(body) {   // one row per own preset: the name (renamed on change) and Remove
  body.textContent = "";
  S.zoomOwn.forEach((p, n) => {
    const row = document.createElement("div"); row.className = "presetRow";
    const inp = document.createElement("input"); inp.type = "text"; inp.value = p.name; inp.maxLength = OWN_ZOOM_NAME_MAX;
    inp.onchange = () => {
      const name = inp.value.trim().slice(0, OWN_ZOOM_NAME_MAX);
      if (!name) { inp.value = p.name; return; }
      S.zoomOwn = S.zoomOwn.map((q, i) => i === n ? { ...q, name } : q);
      saveZoomOwn();
    };
    const rm = document.createElement("button"); rm.className = "btn"; rm.textContent = "Remove";
    rm.onclick = () => { S.zoomOwn = S.zoomOwn.filter((_, i) => i !== n); saveZoomOwn(); renderPresetRows(body); };
    row.append(inp, rm);
    body.appendChild(row);
  });
}
/* P3-19 (A1 p. 5 "Zoom Toggle"): E zooms to 5 ms around the selected hit (the playhead when no hit is selected) and
   stores the view; E again puts the stored view back. Option+Shift+E keeps the view and forgets the stored one.
   P4-27: with a range selected (P4-6a) E zooms so the range fills the lane width exactly. With the app setting "Zoom Toggle
   expands track height" (`zoom_toggle_tall`) E also doubles the wave lane of the track that holds the range (else the selected
   hit's track) and scrolls it into view; E again puts the view, the scroll position and the heights back. The height is a
   local override of the row (S.rowH, a factor of the global wave height; a CSS variable of the row); the app
   setting track_height is not touched. */
const ZOOM_TOGGLE_SPAN = 0.005;
function rowHKey() { return S.rowH ? Object.entries(S.rowH).map(([n, h]) => n + "=" + h).join("|") : ""; }
function setRowHeights(map) {   // map: track name -> factor of the global wave height (an override), {} = every row as the Track Height says; #167
  S.rowH = {};
  for (const row of document.querySelectorAll("#lanesScroll .track")) {
    const f = map[row.dataset.name];
    if (f) { S.rowH[row.dataset.name] = f; row.style.setProperty("--wave-h", `${Math.round(laneHeights().wave * f)}px`); }
    else row.style.removeProperty("--wave-h");
    setHeadTartFit(row);   // #172: a row dragged below the header drawing's height hides the drawing
  }
  invalidate(F.SIZE | F.GL | F.NEAR);
}
/* #172: the role drawing (.tart) sits bottom-right in the header (bottom: 8px); when the row is
   dragged short it would overlap the .tctl controls (the M/S buttons and the pan slider). The
   drawing shows only while the header is tall enough for the drawing plus its 8 px margin above
   the BOTTOM of the controls (#241: the readouts under the M/S buttons are part of .tctl, so
   the rule measures tctl.offsetHeight too — the old tctl.offsetTop + 8 + size hid the drawing
   ~2 rows of heights too late and let it clip the readouts). The size is the authored
   --tart-size, not the live box: a hidden drawing measures 0 px and would unhide itself (an
   oscillation). Measured here (outside the frame loop), not per frame. */
function setHeadTartFit(row) {
  const th = row.querySelector(".thead");
  const tctl = th && th.querySelector(".tctl");
  const tart = th && th.querySelector(".tart");
  if (!th || !tctl || !tart) return;
  const size = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--tart-size")) || 54;
  const needed = tctl.offsetTop + tctl.offsetHeight + 8 + size;
  th.classList.toggle("tart-hidden", th.offsetHeight < needed);
}
function rowHeightsSave() {   // #167: the factor map into the project document (the drag and the double-click land here)
  if (!S.sid || !B.rowHeightsSet) return;
  B.rowHeightsSet(S.sid, S.rowH).catch(e => setMsg(`row heights: ${errText(e)}`, "error"));
}
function rowHeightOf(name) { return S.rowH[name] || 1; }   // the factor of a row (1 = as the Track height says)
function rowHeightPx(name) { return Math.round(laneHeights().wave * rowHeightOf(name)); }
/* #167: the lower border of a row is a drag handle for that row's wave lane. The drag stores a
   factor of the global wave height (the Track Height zoom scales every lane, the dragged one
   included); a double-click puts the row back to the global height. The map lands in the project
   (row_heights) when the drag ends or the double-click resets. */
function bindRowResize(row, name, hd) {   // hd: the handle; the row's lower border (default) or the header's bottom edge (#439: the same drag)
  hd = hd || row.querySelector(".tlanes .rowResize");
  let drag = null;
  hd.addEventListener("pointerdown", ev => {
    if (ev.button !== 0) return;
    if (VL.on) { vlResize(ev); return; }   // #437: with the velocity lane on, the handle is at its bottom edge and drags the lane's height
    drag = { y: ev.clientY, f0: rowHeightOf(name), px0: rowHeightPx(name), f: rowHeightOf(name) };
    hd.setPointerCapture(ev.pointerId);
    ev.preventDefault(); ev.stopPropagation();
  });
  hd.addEventListener("pointermove", ev => {
    if (!drag) return;
    const px = Math.min(1200, Math.max(24, drag.px0 + ev.clientY - drag.y));
    drag.f = px / laneHeights().wave;
    setRowHeights({ ...S.rowH, [name]: drag.f });
  });
  const end = () => {
    if (!drag) return;
    drag = null;
    rowHeightsSave();
  };
  hd.addEventListener("pointerup", end);
  hd.addEventListener("pointercancel", end);
  hd.addEventListener("dblclick", ev => {   // the row back to the global height
    ev.preventDefault(); ev.stopPropagation();
    const map = { ...S.rowH };
    delete map[name];
    setRowHeights(map);
    rowHeightsSave();
    setMsg(`track height of ${name}: ${S.trackHeight}`);
  });
}
function setZoomTall(on, save) {
  S.zoomTall = !!on;
  const c = $("#setZoomTall"); if (c) c.checked = S.zoomTall;
  if (save !== false && B.appSettingsSet) B.appSettingsSet("zoom_toggle_tall", S.zoomTall).catch(e => setMsg(`settings: ${errText(e)}`));
}
function setTabMode(mode) {   // #175: View > Tab Moves To (radio: active hits only | all found hits)
  SETTINGS().ui.tabMode = mode === "all" ? "all" : "active";
  saveSettings();
  const c = $("#setTabMode"); if (c) c.value = SETTINGS().ui.tabMode;
  setMsg(`Tab moves to: ${SETTINGS().ui.tabMode === "all" ? "all found hits" : "active hits only"}`);
  DSShell.sync();
}
function zoomToggle(keep) {
  if (!needProject()) return;
  if (keep) { S.zoomBack = null; setMsg("zoom kept: E zooms in again"); return; }
  S.followHold = true;
  const sc = $("#lanesScroll");
  if (S.zoomBack) {
    const z = S.zoomBack; S.zoomBack = null;
    setView(z.t0, z.t1);
    if (z.rowH) { setRowHeights(Object.fromEntries(Object.entries(z.rowH).filter(([n]) => S.tracks[n]))); sc.scrollTop = z.top; }
    return;
  }
  const rg = LN.sel && S.tracks[LN.sel.track] && LN.sel.t1 > LN.sel.t0 ? LN.sel : null;
  const e = S.sel ? (S.events[S.sel.track] || [])[S.sel.index] : null;
  S.zoomBack = { t0: S.view.t0, t1: S.view.t1, rowH: null, top: sc.scrollTop };
  if (rg) setView(rg.t0, rg.t1);   // the range fills the lane width
  else { const t = e ? e.t : S.playhead; setView(t - ZOOM_TOGGLE_SPAN / 2, t + ZOOM_TOGGLE_SPAN / 2); }
  const trk = rg ? rg.track : e ? S.sel.track : null, row = trk && rowOf(trk);
  if (S.zoomTall && row) {
    S.zoomBack.rowH = { ...S.rowH };
    setRowHeights({ ...S.rowH, [trk]: rowHeightOf(trk) * 2 });
    row.scrollIntoView({ block: "nearest" });   // forces the layout once; this is a key command, not a frame
    GEO.top = sc.scrollTop;
  }
}
/* P3-19 (A1 p. 6): P and ; select the previous and the next track; nothing happens at the first and the last track.
   No track selected: ; takes the first, P the last. */
function selectTrackStep(dir) {
  if (!needProject() || !S.order.length) return;
  const i = S.order.indexOf(S.selTrack);
  const j = i < 0 ? (dir > 0 ? 0 : S.order.length - 1) : i + dir;
  if (j < 0 || j >= S.order.length) return;
  setSelTrack(S.order[j]);
  const row = document.querySelector(`.track[data-name="${CSS.escape(S.order[j])}"]`);
  if (row && row.scrollIntoView) row.scrollIntoView({ block: "nearest" });
}
function zoomToProject() { S.followHold = true; setAmpZoom(1); setView(0, maxDur()); }   // P3-20a: also resets the amplitude zoom (A1 p. 34)
function projectEnd() { return S.order.length ? Math.max(...S.order.map(n => S.tracks[n].duration || 0)) : 0; }
function goTo(t) {   // Transport > Go to ...: the playhead moves (a seek while playing), the view follows it
  S.playhead = Math.max(0, t);
  if (S.playhead < S.view.t0 || S.playhead > S.view.t1) centerOn(S.playhead);
  invalidate(F.HEAD); updateStatus();
}
function playStop(key) {   // key: the pressed key in the table's notation (a menu item or the play button: undefined)
  return auStop() ? undefined : S.playing
    ? ((S.playReturn && (key === "Space" || key === S.playReturn))   // #207 play-and-return: the next Space or the same key returns
       || SETTINGS().ui.stopReturnToStart ? stopAndReturn()   // #436: the mode returns every Stop of Play/Stop (Space, Numpad0, the ■ button)
      : pause())
    : play(key === "Option+Space" ? key : null);
}
/* P3-20b: Shift+Space (Pro Tools "Half-speed playback"): play from the playhead at half speed, one octave down;
   Space or Shift+Space stops. The transport resamples to twice the device rate; the playhead, the follow, the
   meters and the clicks run at the played speed (the messages carry speed 0.5). */
function halfSpeedPlayStop(key) {   // P4-40d: Space and Shift+Space stop an audition first; key as in playStop (#207)
  return auStop() ? undefined : S.playing
    ? (S.playReturn && (key === "Space" || key === S.playReturn) ? stopAndReturn() : pause())
    : appPlay(0.5, key === "Option+Shift+Space" ? key : null);
}
function stopAndReturn() { const p = S.playStartPos; auStop(true); pause(); S.playhead = p; invalidate(F.HEAD); updateStatus(); }
const FOLLOW_ICON = { smooth: "⇢", page: "⇥", off: "⊘" };   // P3-14e: the follow button shows the mode
let lastFollow = "smooth";   // the mode that Follow Playhead On/Off (F) turns back on
function setFollow(mode) {
  if (!["smooth", "page", "off"].includes(mode)) return;
  if (mode !== "off") lastFollow = mode;
  S.followMode = mode; S.followHold = false;
  const fb = $("#followBtn"); fb.dataset.mode = mode; fb.textContent = FOLLOW_ICON[mode];
  fb.classList.toggle("on", mode !== "off"); fb.setAttribute("aria-label", `follow playhead: ${mode}`);
  DSShell.sync();
}
function toggleFollow() { setFollow(S.followMode === "off" ? lastFollow : "off"); setMsg(`follow playhead: ${S.followMode}`); }
/* P4-wf10: Transport > Suspend Follow when Editing (a user setting: it persists across a restart).
   While it is checked and the transport plays, an add, move or delete of a hit suspends the follow: the
   view start does not change for 2 s of playback, then followResume turns the view to the page of the playhead. */
function toggleSuspendFollow() {
  const st = SETTINGS();
  st.ui.suspendFollowOnEdit = !st.ui.suspendFollowOnEdit; saveSettings();
  if (!st.ui.suspendFollowOnEdit) S.followResumeAt = null;   // unchecked while suspended: follow again at once
  setMsg(`suspend follow when editing: ${st.ui.suspendFollowOnEdit ? "on" : "off"}`);
  DSShell.sync();
}
/* #436: Transport > Stop Returns to Start (a user setting: it persists across a restart). While it is
   checked, a Stop (Space while playing, or the ■ button) puts the playhead back where the play started,
   exactly like Option+Space. Unchecked, the stop keeps the playhead where it is. */
function toggleStopReturnMode() {
  const st = SETTINGS();
  st.ui.stopReturnToStart = !st.ui.stopReturnToStart; saveSettings();
  setMsg(`stop returns to start: ${st.ui.stopReturnToStart ? "on" : "off"}`);
  DSShell.sync();
}
/* #240: Audition on Tab as a toolbar button and a shortcut: a step to a hit (Tab) plays it, unless
   the page plays. One flip sets the button state, the Settings checkbox, the menu check and the
   saved app settings together (like setClick). */
function setAuditionTab(on) {
  const st = SETTINGS();
  st.ui.auditionTab = !!on; saveSettings();
  $("#setAuditionTab").checked = st.ui.auditionTab;
  $("#auditionTabBtn").classList.toggle("on", st.ui.auditionTab);
  $("#auditionTabBtn").setAttribute("aria-pressed", st.ui.auditionTab ? "true" : "false");
  DSShell.sync();
}
function toggleAuditionTab() {
  setAuditionTab(!SETTINGS().ui.auditionTab);
  setMsg(`audition on tab: ${SETTINGS().ui.auditionTab ? "on" : "off"}`);
}
function suspendFollowForEdit() {   // an edit during playback: hold the view, resume 2 s of playback later
  if (!SETTINGS().ui.suspendFollowOnEdit || !S.playing || S.followMode === "off") return;
  S.followHold = true;
  S.followResumeAt = AC.drawn + 2;   // song seconds: 2 s of playback (a second edit extends it)
}
function followResume() {   // in the tick: 2 s of playback after the edit the follow resumes at the next page
  if (S.followResumeAt === null || !S.playing || PH.v < S.followResumeAt) return;
  S.followResumeAt = null;
  if (!S.followHold) return;
  S.followHold = false;
  const d = viewDur(), v = viewClamp(PH.v - FOLLOW_FRAC * d, PH.v - FOLLOW_FRAC * d + d);
  if (Math.abs(v.t0 - S.view.t0) > 1e-12 || Math.abs(v.t1 - S.view.t1) > 1e-9) setView(v.t0, v.t1);   // the next page: only a turn is a setView
}
function setSnap(on) { S.snap = !!on; $("#snapChk").checked = S.snap; DSShell.sync(); }
function toggleSnap() { setSnap(!S.snap); setMsg(S.snap ? "snap to hits on" : "snap to hits off"); }
function setClick(on) {
  const st = SETTINGS();
  st.click.enabled = !!on; saveSettings();
  $("#setClickOn").checked = st.click.enabled; $("#clickBtn").classList.toggle("on", st.click.enabled);
  DSShell.sync();
}
function toggleClick() {
  setClick(!SETTINGS().click.enabled);
  setMsg(SETTINGS().click.enabled ? "metronome click on — beeps on every hit of audible tracks" : "metronome click off");
}
function trackButton(cls) {   // Track > Mute or Solo Selected Track: the header button of the selected track
  const n = needTrack();
  const row = n && document.querySelector(`.track[data-name="${CSS.escape(n)}"]`);
  if (row) row.querySelector(cls).click();
}
function analyzeSelected() { const n = needTrack(); if (n) startAnalysis([n]); }
function exportSelectedTracks() { const n = needTrack(); if (n) return openExport("selected", n); }   // P4-30: Track > Export Selected Tracks…
function showModal(id) {   // Settings and Keyboard Shortcuts: Escape or the button closes them
  const m = $(id);
  m._opener = document.activeElement;   // #387: the focus comes back to it on close
  m.classList.remove("hidden");
  const esc = ev => { if (ev.key === "Escape" && !m.classList.contains("hidden")) { ev.stopPropagation(); m.classList.add("hidden"); if (m._onclose) m._onclose(); } };
  document.removeEventListener("keydown", m._esc, true);
  m._esc = esc; document.addEventListener("keydown", esc, true);
  return m;
}
/* #387: the Settings dialog is a preferences window: a category list on the left, one category's
   rows on the right. The last open category is remembered per app home (localStorage). */
const SET_CATS = ["audio", "keyboard", "view", "audition", "pitches", "plugins", "agent"];
function settingsShowCat(cat) {
  if (!SET_CATS.includes(cat)) cat = "audio";
  document.querySelectorAll("#settingsModal .setCat").forEach(s => s.classList.toggle("active", s.dataset.cat === cat));
  document.querySelectorAll("#setCats .setCatBtn").forEach(b => {
    const on = b.dataset.cat === cat;
    b.classList.toggle("active", on);
    b.setAttribute("aria-selected", on ? "true" : "false");
  });
  const page = $("#setPage"); if (page) page.scrollTop = 0;
  return cat;
}
function settingsOpenCat() {
  const saved = localStorage.getItem("dsg_setcat");
  return SET_CATS.includes(saved) ? saved : "audio";
}
function settingsClose() {
  const m = $("#settingsModal");
  if (m.classList.contains("hidden")) return;
  m.classList.add("hidden");
  if (m._onclose) m._onclose();
}
function openSettings() {
  const m = showModal("#settingsModal");
  settingsShowCat(settingsOpenCat());
  const btn = document.querySelector("#setCats .setCatBtn.active"); if (btn) btn.focus();   // the focus moves into the dialog
  m._onclose = () => { if (m._opener && m._opener.focus) m._opener.focus(); };
  refreshAudioOutput(); setPluginsFill(); agentSkillFill();
}

function openShortcuts() {
  DSHelp.open("shortcuts");   // #255: the Shortcuts section of the manual (static/keymap.js, one source)
}   // P3-13b: made from static/keymap.js
