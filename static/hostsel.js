/* Drum Spike - the page follows the selection of the DAW (DAW plugin M1b13, #562)
   ARA gives one direction: the host tells the plugin which tracks and which time range it has selected and which tracks it
   hides. The plugin maps that to the tracks of the page (the payload of the event plugin-selection, plugins/ara/src/selection.rs)
   and this script shows it, as the editors of the role models do. The design and the rules are in
   docs/design/daw-plugin/m1b13-selection.md. In short:
   - Tracks: the first track of the payload is the selected track of the page (setSelTrack) and scrolls into view; the other
     tracks of the payload get the mark .hostSel (a dashed outline). A selection of a DAW track that has no source in the plugin
     arrives as `none`: the page changes nothing.
   - Range: the time range of the host is a band over the ruler and the lanes (#hostRange), in the colour of the host, not the
     hit selection of the page (LN.sel). It is in source time of the first selected track already. A range outside the view
     scrolls the view to it; no range, an empty range and a range in no region remove the band.
   - Hidden tracks: the rows of the hidden tracks are hidden (.hostHidden) and one short text in the status bar names how many.
   - The user can switch the follow off (View > Follow the DAW Selection in the page menu). Off: the marks, the band and the
     hidden rows go and nothing follows; on: the page takes the last selection of the host.
   - A selection that the user makes in the page is never overwritten while a pointer or key action runs. A selection of the
     host that arrives meanwhile waits and is taken when the action ends, unless the user changed the selection of the page in
     the meantime; then the host's selection is dropped (the hidden tracks are still applied).
   - The other direction does not exist: ARA has no call for a plugin to set the host's selection. A click on a track in the page
     changes the page only.
   With the mode off nothing here runs: hostSelInit() returns at once (the standalone page is unchanged). */
"use strict";

const HS = { ready: false, follow: true, state: null, n: 0, applied: "", pending: null, actionSig: "", timer: 0,
  pointer: false, pointerAt: 0, keyDown: false, keyAt: -1e9, ranges: [], band: null, note: null, hiddenKey: "" };
const HS_KEY_TAIL_MS = 300;    // a key action ends this long after its key up (the command it started may still be running)
const HS_STUCK_MS = 20000;     // a pointer or key that was never released (the window lost the up event) stops counting

/* A pointer or key action runs: the button is down in the page, or a key is down or was released a moment ago. */
function hostSelEditing() {   // a text field has the focus: the user types; the host's selection waits until the field is left
  const e = document.activeElement;
  if (!document.hasFocus()) return false;   // the user works in the DAW: the field keeps its focus in the page, but nobody types in it
  return !!e && (e.isContentEditable || e.tagName === "TEXTAREA" || e.tagName === "SELECT" || (e.tagName === "INPUT" && !["checkbox", "radio", "button", "range", "color", "file"].includes(e.type)));
}
function hostSelBusy() {
  const now = performance.now();
  return hostSelEditing() || (HS.pointer && now - HS.pointerAt < HS_STUCK_MS) || (HS.keyDown && now - HS.keyAt < HS_STUCK_MS) || now - HS.keyAt < HS_KEY_TAIL_MS;
}
function hostSelFollowOn() { return HS.follow; }

/* The selection of the page as a text: the selected track, the selected hit and the range selection. A change of it while an action
   runs is the user's own change. */
function hostSelSig() {
  return [S.selTrack, S.sel ? `${S.sel.track}:${S.sel.index}` : "", typeof LN !== "undefined" && LN.sel ? `${LN.sel.track}:${LN.sel.t0}:${LN.sel.t1}` : ""].join("|");
}

/* A track that the DAW hides (its row is hidden). The keys that move the selection of the track or step over the hits skip it. */
function hostHidden(name) {
  if (!hostMode()) return false;
  const r = document.querySelector(`.track[data-name="${CSS.escape(name)}"]`);
  return !!r && (r.classList.contains("hostHidden") || r.classList.contains("hostMerged"));   // #555: a merged row is hidden too: no key stops on it
}
const hsRow = name => document.querySelector(`.track[data-name="${CSS.escape(name)}"]`);

/* Scroll the lanes area so that the row is inside it (a row taller than the area: its top). The view in time is not touched. */
function hostSelReveal(name) {
  const row = hsRow(name), sc = document.getElementById("lanesScroll");
  if (!row || !sc || row.classList.contains("hostHidden")) return;
  const r = row.getBoundingClientRect(), c = sc.getBoundingClientRect();
  if (r.top < c.top || r.height > c.height) sc.scrollTop += r.top - c.top;
  else if (r.bottom > c.bottom) sc.scrollTop += r.bottom - c.bottom;
}

/* The marks of the other selected tracks. */
function hostSelMarks(names) {
  const set = new Set(names);
  for (const row of document.querySelectorAll(".track")) row.classList.toggle("hostSel", set.has(row.dataset.name));
}

/* The hidden tracks: their rows and the one text that names how many. */
function hostSelHidden(names) {
  const set = new Set((names || []).filter(n => S.tracks[n]));
  const key = [...set].sort().join("\u0000") + "\u0001" + document.querySelectorAll(".track").length;   // the rows are new after a load
  for (const row of document.querySelectorAll(".track")) row.classList.toggle("hostHidden", set.has(row.dataset.name));
  if (HS.note) {
    HS.note.textContent = set.size ? `${set.size} ${set.size === 1 ? "track" : "tracks"} hidden in the DAW` : "";
    HS.note.classList.toggle("hidden", !set.size);
  }
  if (S.sel && set.has(S.sel.track)) { S.sel = null; updateStatus(); }
  if (LN.sel && set.has(LN.sel.track)) lnClearSel();   // nor on the range selection of a hidden track   // no key may act on a hit that nobody can see
  if (key !== HS.hiddenKey) { HS.hiddenKey = key; invalidate(F.SIZE | F.VIEW); }   // the rows moved: the page measures and draws again
}

/* The bands of the host range over the ruler and the lanes, one for each part of the range (a part for each region of the track that
   holds a piece of it). Called by drawHead (every frame that moves the view). */
function hostSelPaint() {
  const b = HS.band;
  if (!b) return;
  if (!HS.ranges.length || !hostMode()) { b.style.display = "none"; return; }
  const W = laneWidth();
  while (b.children.length < HS.ranges.length) { const d = document.createElement("div"); d.className = "hostBand"; b.appendChild(d); }
  while (b.children.length > HS.ranges.length) b.lastChild.remove();
  let any = false;
  HS.ranges.forEach((r, i) => {
    const el = b.children[i], x0 = Math.max(0, tToX(r.t0, W)), x1 = Math.min(W, tToX(r.t1, W));
    if (x1 <= 0 || x0 >= W || x1 <= x0) { el.style.display = "none"; return; }
    any = true;
    el.style.display = "block"; el.style.left = `${S.theadW + x0}px`; el.style.width = `${Math.max(2, x1 - x0)}px`;
  });
  b.style.display = any ? "block" : "none";
}

/* Scroll the view in time to the first band, when no band is in view. A band that is partly in view stays. */
function hostSelScrollTo(ranges) {
  if (ranges.some(r => r.t1 > S.view.t0 && r.t0 < S.view.t1)) return;
  const r = ranges[0], span = viewDur(), mid = (r.t0 + r.t1) / 2;
  if (r.t1 - r.t0 <= span * 0.9) setView(mid - span / 2, mid + span / 2);
  else setView(r.t0 - span * 0.05, r.t0 - span * 0.05 + span);
}

function hostSelRange(list, reveal) {
  const rs = (Array.isArray(list) ? list : []).filter(r => r && Number.isFinite(r.t0) && Number.isFinite(r.t1) && r.t1 > r.t0).map(r => ({ t0: r.t0, t1: r.t1 }));
  HS.ranges = rs;
  hostSelPaint();
  if (rs.length && reveal) hostSelScrollTo(rs);
}

/* Take the selection of the payload. `fresh`: a new message (it selects, scrolls, may move the view); not fresh: the page
   was built again (a project load): marks, hidden rows and the band only. Returns what it did (the tests read it). */
function hostSelKey(p) { return JSON.stringify([p.tracks || [], p.ranges || []]); }
function hostSelApply(p, fresh, hiddenOnly) {
  hostSelHidden(p.hidden);
  if (hiddenOnly || p.none) return { hidden: true };
  const hid = new Set(p.hidden || []);
  const keys = [...new Set((p.tracks || []).filter(n => S.tracks[n] && !hid.has(n)).map(HT.homeFor))];   // a hidden track is never selected: the first one that is shown is; #555: a source selects the row that shows it
  const key = hostSelKey(p);
  const changed = key !== HS.applied;
  if (keys.length) {
    if (fresh && changed) { if (S.selTrack !== keys[0]) setSelTrack(keys[0]); hostSelReveal(keys[0]); }
    hostSelMarks(keys.slice(1));
  } else hostSelMarks([]);   // all selected tracks are hidden (or none is named): the selection of the page does not change
  hostSelRange(p.ranges, fresh && changed);
  if (fresh && (changed || keys.length)) HS.applied = key;
  return { selected: keys[0] || null, marked: keys.slice(1) };
}

function hostSelClear() {
  HS.applied = ""; HS.pending = null;
  hostSelMarks([]); hostSelHidden([]); hostSelRange([], false);
}

/* The state of the host is shown now, or waits for the end of an action. */
function hostSelRun() {
  if (!hostMode() || !HS.state) return;
  if (!hostSelFollowOn()) return;
  if (!S.sid || !S.order.length) return;   // the project is not loaded: hostSelReapply takes it after the load
  if (hostSelBusy()) {
    HS.pending = HS.state;
    hostSelWait();
    return;
  }
  hostSelApply(HS.state, true, false);
}
function hostSelWait() {
  if (HS.timer) return;
  HS.timer = setTimeout(() => {
    HS.timer = 0;
    if (!HS.pending) return;
    if (hostSelBusy()) { hostSelWait(); return; }
    const p = HS.pending; HS.pending = null;
    // the user changed the selection during the action: that one stays, the hidden tracks are still applied
    const userChanged = hostSelSig() !== HS.actionSig;
    if (userChanged) HS.applied = hostSelKey(p);   // dropped: it counts as seen, so a later load of the project does not select it over the user's choice
    hostSelApply(p, true, userChanged);
  }, 60);
}

/* A message of the plugin (the event plugin-selection, or the answer of the pull after the page opened). An older one is ignored. */
function hostSelEvent(p) {
  if (!hostMode() || !p || typeof p !== "object") return;
  if (Number.isFinite(p.n) && p.n < HS.n) return;
  HS.n = Number.isFinite(p.n) ? p.n : HS.n;
  HS.state = p;
  hostSelRun();
}

/* The page was built again (a project load): the marks and the hidden rows are new DOM. Called at the end of buildTrackDOM. */
function hostSelReapply() {
  if (!hostMode() || !HS.state || !HS.ready) return;
  if (!hostSelFollowOn()) return;
  if (hostSelBusy()) { hostSelRun(); return; }
  hostSelApply(HS.state, !HS.applied, false);
}

function hostSelSetFollow(on) {
  if (!hostMode()) return;
  HS.follow = !!on;
  try { window.localStorage.setItem("dsHostFollowSel", on ? "1" : "0"); } catch (e) { /* no storage: the choice lasts for this page */ }
  if (!on) { hostSelClear(); setMsg("the page no longer follows the selection of the DAW"); return; }
  HS.applied = "";
  setMsg("the page follows the selection of the DAW");
  hostSelRun();
}

function hostSelInit() {
  if (!hostMode() || HS.band) return;
  try { HS.follow = window.localStorage.getItem("dsHostFollowSel") !== "0"; } catch (e) { /* no storage */ }
  const band = document.createElement("div");
  band.id = "hostRange"; band.title = "The time range that the DAW has selected (not the selection of this page)";
  band.setAttribute("aria-label", "DAW time range");
  document.getElementById("editor").appendChild(band);
  HS.band = band;
  const note = document.createElement("span");
  note.id = "hostHiddenNote"; note.className = "hidden"; note.title = "The DAW hides these tracks; the page hides them too (View > Follow the DAW Selection turns this off)";
  const st = document.getElementById("stSel");
  st.parentNode.insertBefore(note, st.nextSibling);
  HS.note = note;
  const begin = () => { if (!hostSelBusy()) HS.actionSig = hostSelSig(); };   // the selection of the page when the action starts (the capture listener runs before the page's own)
  const down = () => { begin(); HS.pointer = true; HS.pointerAt = performance.now(); };
  const up = () => { HS.pointer = false; HS.pointerAt = performance.now(); };
  document.addEventListener("mousedown", down, true);
  document.addEventListener("pointerdown", down, true);
  document.addEventListener("mouseup", up, true);
  document.addEventListener("pointerup", up, true);
  document.addEventListener("pointercancel", up, true);
  window.addEventListener("blur", () => { HS.pointer = false; HS.keyDown = false; });
  document.addEventListener("keydown", () => { begin(); HS.keyDown = true; HS.keyAt = performance.now(); }, true);
  document.addEventListener("keyup", () => { HS.keyDown = false; HS.keyAt = performance.now(); if (HS.pending) hostSelWait(); }, true);
  document.addEventListener("mouseup", () => { if (HS.pending) hostSelWait(); }, true);
  document.addEventListener("focusin", () => { if (!HS.pointer && !HS.keyDown) HS.actionSig = hostSelSig(); }, true);   // a field gets the focus (also by Tab): the selection of the page now is the one the user had
  document.addEventListener("focusout", () => { if (HS.pending) hostSelWait(); }, true);
  if (B.listen) B.listen("plugin-selection", hostSelEvent).catch(() => {});
  HS.ready = true;
}

window.DSHostSel = {
  init: hostSelInit, event: hostSelEvent, setFollow: hostSelSetFollow, follows: () => HS.follow, busy: hostSelBusy, reapply: hostSelReapply,
  get ready() { return HS.ready; },
  state: () => ({ follow: HS.follow, n: HS.n, range: HS.ranges[0] || null, ranges: HS.ranges, pending: !!HS.pending, applied: HS.applied, payload: HS.state,
    marked: [...document.querySelectorAll(".track.hostSel")].map(r => r.dataset.name),
    hidden: [...document.querySelectorAll(".track.hostHidden")].map(r => r.dataset.name), note: HS.note ? HS.note.textContent : "" }),
};
