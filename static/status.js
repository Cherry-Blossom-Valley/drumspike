/* Drum Spike v2 — status bar and info line (refactor plan step 5.2, R9 A2)
   The status message with its levels (#112: setMsg, setMsgBg), updateStatus (the status bar and everything
   that follows the selection) and the info line of the selected hit (P3-14b, IL) with its sample and
   velocity fields. Moved verbatim from app.js. Plain script (no build step), loaded after plugins.js and
   before app.js. The names stay top-level bindings (page globals). */
"use strict";

/* ---------------- status ---------------- */
var _msgSeq = 0;   // var, not let: setMsg may run before this line; counts status messages: the background PCM prefetch never overwrites a newer one
/* #112: messages have levels (error, warning, info). A background info does not replace a held error
   until the error's hold time ends (MSG_HOLD) or the user acts (a direct setMsg from a user action). */
var MSG_HOLD = 5000;   // ms an error keeps the status line
var _msgLevel = "info";   // the level of the message now shown
var _msgHoldUntil = 0;   // performance.now() until which a background info must wait
function setMsgLvl(m, lvl, bg) {
  const now = performance.now();
  if (bg && _msgLevel === "error" && now < _msgHoldUntil) return;   // a held error stays readable
  _msgSeq++;
  $("#stMsg").textContent = m;
  _msgLevel = lvl;
  _msgHoldUntil = lvl === "error" ? now + MSG_HOLD : 0;
}
function setMsg(m, lvl) { setMsgLvl(m, lvl || "info", false); }   // direct (user action): replaces everything
function setMsgBg(m, lvl) { setMsgLvl(m, lvl || "info", true); }   // background writer: yields to a held error
/* A rejected promise or a thrown error that no handler caught would end in the console only: the user clicks and sees
   nothing. One message to the status line, the full error to the console. A failure inside the report is not reported
   again (the flag), so the handler cannot loop. */
var _reporting = false;
function reportPageError(err) {
  if (_reporting) return;
  _reporting = true;
  try {
    console.error("drumspike: unhandled", err);
    setMsg(`unexpected error: ${errText(err)}`, "error");
  } catch (e) { /* the console line above is all that is left */ }
  finally { _reporting = false; }
}
window.addEventListener("error", ev => {
  if (!ev.error && /^ResizeObserver loop/.test(ev.message || "")) return;   // the browser's harmless layout notice, no error of the page
  reportPageError(ev.error || ev.message);
});
window.addEventListener("unhandledrejection", ev => reportPageError(ev.reason));
let selSynced = null;   // the last selHit that the menu saw (sync only when it flips, not on every drag mousemove)
function selShellSync() {   // #158 (rank 9): Delete, the nudge items and Audition Hit are greyed without a hit or range selection
  const sel = !!(S.sel || LN.sel);
  if (sel !== selSynced) { selSynced = sel; if (window.DSShell) DSShell.sync(); }
}
function updateStatus() {   // P3-14b: the status bar is short; the info line has the details
  const e = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  const cut = e ? (hgIsCut(e) || (lnLearn(S.sel.track) && lnGreyHit(S.sel.track, e))) : false;   // #175: a deactivated hit says so
  $("#stSel").textContent = e ? `${S.sel.track}: hit ${S.sel.index + 1}/${S.events[S.sel.track].length}${cut ? " · deactivated" : ""}` : "no selection";
  ensureSelTrack();   // P4-39c
  drawInfo();
  syncInspector();   // P3-14a: the Inspector follows the selected track
  lnInspector();   // P4-6a: the Learn row (buttons follow the selection)
  mxRefresh();   // #157: the other mics of the selected hit
  selShellSync();   // #158 (rank 9): the selection-dependent menu items follow the selection
  trainingSync();   // #343
}

/* #343: View > Training Mode. A page setting (SETTINGS().ui.training, off by default, saved with saveSettings): with it
   on, every hit edit and Learn mark request carries training: true and the service logs one record per hit affected.
   The status bar shows "training: N records" while the mode is on (N from training_status). */
const TRN = { sid: null, records: 0 };
function trainingOn() { return !!SETTINGS().ui.training; }
function drawTraining() {
  const el = $("#stTraining"), on = trainingOn();
  el.classList.toggle("hidden", !on);
  if (on) el.textContent = `training: ${TRN.records} records`;
}
function trainingRefresh() {   // the service's count for the open project (it also learns that the mode is on)
  const sid = S.sid;
  if (!sid || !trainingOn()) { drawTraining(); return Promise.resolve(); }
  return B.trainingStatus(sid, true).then(r => { if (sid === S.sid) { TRN.sid = sid; TRN.records = r.records; drawTraining(); } },
    e => setMsg(`training: ${errText(e)}`, "error"));
}
function trainingSync() {   // after a project switch: ask for its count once
  drawTraining();
  if (trainingOn() && S.sid && TRN.sid !== S.sid) { TRN.sid = S.sid; trainingRefresh(); }
}
function trainingNote(r) {   // the answer of a call that asked for a log
  if (r && r.training_error) setMsg(`training log failed: ${r.training_error}`, "error");
  trainingRefresh();
}
function setTraining(on) {
  SETTINGS().ui.training = !!on; saveSettings();
  if (S.sid) B.trainingStatus(S.sid, !!on).catch(() => {});   // the service reports the state (agents read it)
  TRN.sid = null; trainingSync();
  DSShell.sync();
}
function toggleTraining() {
  setTraining(!trainingOn());
  setMsg(`training mode: ${trainingOn() ? "on - every hit edit and Learn mark is logged with its audio" : "off"}`);
}

/* ---------------- info line (P3-14b, Cubase model) ----------------
   Under the toolbar, a fixed height (it does not move the lanes when a hit is selected). The selected hit: track, index of
   total, time, sample (field), velocity (field), origin (detected, moved, added) and the offset from the detected position
   in samples. Return applies a field, Escape cancels; each edit is one undo step (move or setv). Texts are written only
   when they changed (no layout read). */
var IL = {};   // the last texts written, key -> text
function ilSet(id, text, prop) {
  const k = id + (prop || "");
  if (IL[k] === text) return;
  IL[k] = text;
  const el = $("#" + id);
  if (prop === "value") el.value = text; else el.textContent = text;
}
function hitOrigin(e, sr) {   // {origin, offset}: "detected" 0, "moved" and the offset in samples, "added" and null
  if (e.added) return { origin: "added", offset: null };
  if (isManual(e)) return { origin: "moved", offset: Math.round((e.t - e.orig) * sr) };
  return { origin: "detected", offset: 0 };
}
function drawInfo() {
  const box = $("#infoLine");
  const shown = !!SETTINGS().ui.infoLine;
  if (box.classList.contains("hidden") === shown) box.classList.toggle("hidden", !shown);
  if (!shown) return;
  const selText = lnSelText(), selEl = $("#ilSel");   // P4-6a: the range selection and its hit count
  ilSet("ilSel", selText); if (selEl.classList.contains("hidden") === !!selText) selEl.classList.toggle("hidden", !selText);
  const e = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  const f = $("#ilFields"), empty = $("#ilEmpty");
  if (f.classList.contains("hidden") !== !e) f.classList.toggle("hidden", !e);
  if (empty.classList.contains("hidden") !== !!e) empty.classList.toggle("hidden", !!e);
  if (!e) return;
  const tr = S.tracks[S.sel.track], sr = tr.sr, o = hitOrigin(e, sr);
  ilSet("ilTrack", S.sel.track);
  const cut = hgIsCut(e) || (lnLearn(S.sel.track) && lnGreyHit(S.sel.track, e));   // #175: a deactivated hit says so
  ilSet("ilIndex", `${S.sel.index + 1}/${S.events[S.sel.track].length}${cut ? " · deactivated" : ""}`);
  ilSet("ilTime", `${fmt(e.t, 6)} s`);
  if (document.activeElement !== $("#ilSample")) ilSet("ilSample", String(Math.round(e.t * sr)), "value");
  if (document.activeElement !== $("#ilVel")) ilSet("ilVel", String(e.v), "value");
  ilSet("ilOrigin", o.origin);
  ilSet("ilOffset", o.offset === null ? "-" : `${o.offset > 0 ? "+" : ""}${o.offset} smp`);
  const lk = lnLearn(S.sel.track) ? lnKindOf(S.sel.track, e.t) : null, lf = $("#ilLearnF");   // P4-6a: a hit that is a Learn example
  // P4-6c: the hit's similarity score; a grey hit of the filter says why
  const sim = lnLearn(S.sel.track) ? lnScoreAt(S.sel.track, e.t - lnShift(S.sel.track)) : null, th = (LN_SC[lnTarget(S.sel.track)] || {}).threshold;
  const simTxt = sim === null ? "" : lnGreyHit(S.sel.track, e) && th !== undefined ? `filtered: sim ${sim.toFixed(2)} < ${th.toFixed(2)}` : `sim ${sim.toFixed(2)}`;
  ilSet("ilLearn", lk === "not" ? "not this" : lk === "this" ? "this" : simTxt);
  const show = !!lk || !!simTxt;
  if (lf.classList.contains("hidden") === show) lf.classList.toggle("hidden", !show);
  const box2 = $("#ilOrigin"); if (box2.dataset.o !== o.origin) box2.dataset.o = o.origin;
  const why = rqWhy(e), df = $("#ilDoubtF");   // #150: why the review queue counts the hit as doubtful
  ilSet("ilDoubt", why);
  if (df.classList.contains("hidden") === !!why) df.classList.toggle("hidden", !why);
  const lm = lyLayer(S.sel.track) ? lyMatchOf(S.sel.track, e.t) : null, mf = $("#ilMatchF");   // P4-7b: the layer match of the hit
  ilSet("ilMatch", lm ? `${lm.match === null ? "-" : lm.match.toFixed(2)}${lm.uncertain ? " uncertain" : ""}${lm.sign !== null && LY.align[S.sel.track] && lm.sign !== (LY.align[S.sel.track].polarity === "inverted" ? -1 : 1) ? " flip" : ""}` : "");
  if (mf.classList.contains("hidden") === !!lm) mf.classList.toggle("hidden", !lm);
}
function setInfoLine(on) {   // View > Show Info Line
  SETTINGS().ui.infoLine = !!on; saveSettings();
  drawInfo(); DSShell.sync();
}
function infoFieldApply(which) {   // Return in a field: the value is applied as one undo step; a bad value is refused with a message
  const inp = $(which === "sample" ? "#ilSample" : "#ilVel");
  const e = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  const text = inp.value.trim();
  const back = () => { IL[inp.id + "value"] = null; drawInfo(); };
  if (!e) { back(); return false; }
  if (!/^[0-9]+$/.test(text)) { setMsg(`${which === "sample" ? "sample" : "velocity"}: type a whole number`); back(); return false; }
  const n = parseInt(text, 10), trk = S.sel.track, t = S.tracks[trk];
  if (which === "sample") {
    const last = (t.n_samples || Infinity) - 1, smp = Math.min(n, last);
    if (smp !== n) setMsg(`sample ${n} is after the end of ${trk}: set to ${smp}`);
    setHitTime(trk, S.sel.index, smp / t.sr);
  } else {
    const v = Math.max(1, Math.min(127, n));
    if (v !== n) setMsg(`velocity ${n} is outside 1 to 127: set to ${v}`);
    if (followerLocked(trk)) { back(); updateStatus(); return true; }   // P3-18
    const id = e.id;
    edCall(trk, evs => {   // events_edit velocity, one undo step
      const j = edIndex(evs, id);
      if (j < 0 || evs[j].v === v) return null;
      return { ops: [{ op: "velocity", index: j, v, edited: true }], step: { type: "setv", track: trk, id, from: evs[j].v, to: v, was: hitWas(evs[j]) } };   // setv; P4-40f: a hand velocity is an edit; R05: was for the undo
    });
  }
  back(); updateStatus();
  return true;
}
function setupInfoLine() {
  for (const [id, which] of [["ilSample", "sample"], ["ilVel", "vel"]]) {
    const inp = $("#" + id);
    inp.addEventListener("keydown", ev => {
      if (ev.key === "Enter") { ev.preventDefault(); ev.stopPropagation(); infoFieldApply(which); inp.blur(); }
      else if (ev.key === "Escape") { ev.preventDefault(); ev.stopPropagation(); IL[id + "value"] = null; drawInfo(); inp.blur(); }
    });
    inp.addEventListener("blur", () => { IL[id + "value"] = null; drawInfo(); });   // a field that loses the focus is cancelled
  }
  $("#timeDisplay").addEventListener("click", () => exchangeCounters());
  $("#timeDisplay").style.cursor = "pointer";
  const ns = $("#nudgeSel");   // P3-20a: the list of nudge values; focus goes back to the page so the keys work
  for (const k of NUDGE_ORDER) ns.add(new Option(`Nudge ${NUDGE_VALUES[k].label}`, k));
  ns.value = S.nudgeValue;
  ns.addEventListener("change", () => { chooseNudgeValue(ns.value); ns.blur(); });
  DSKeys.live.nudge_earlier = DSKeys.live.nudge_later = () => `now ${NUDGE_VALUES[S.nudgeValue].label}`;
  $("#timeDisplay").title = `Main counter (${SETTINGS().ui.counter}). Click: exchange the counters (View > Main Counter > Exchange Counters)`;
  drawInfo();
}
