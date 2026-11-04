/* Drum Spike v2 — learn (refactor plan step 5.2, R9 A2)
   Range selection and Learn marks (LN), the similarity scores in the page (LN_SC: the filter, the
   histogram, the grey hits) and the test hook window.DSLearn. Moved verbatim from app.js. Plain
   script (no build step), loaded after histograms.js and before app.js; its load-time statements
   read the theme tokens (LN_CSS) and set window.DSLearn. */
"use strict";

/* ---------------- P4-6a: range selection and Learn marks (owner request 2026-10-12, docs/migration/concept-learn-mode.md) ----------------
   A drag in the wave lane selects a time range on that track (LN.sel; one at a time, a click, K or Escape clears it). K
   (Hits > Learn This) and Shift+K (Forget This) turn the selection, or the selected hit, into a mark: the service resolves the
   examples (track_learn_mark: the active hits, else the strongest candidate, else the strongest rise of 6 dB) and keeps them in
   the track's `learn` object. The page draws the selection band in both lanes, a 3 px bar per mark at the top of the spike
   lane (green: this, red: not) with a dot or a cross on each example hit, and the Inspector has a Learn row. Every step
   (mark, remove, flip, forget) is one undo step {type: "learn", track, before, after}: the whole learn object goes back with
   track_learn_set. A follower marks its source (the service routes). There is no filter yet (P4-6b): a "not" hit is drawn grey. */
const LN_CSS = getComputedStyle(document.documentElement);   // the theme tokens, read once (never in a frame)
const LN_NOT_TINT = "#b8706c";   // P4-6c: a "not" example that is still in the output (not the filtered grey)
const LN_THIS = LN_CSS.getPropertyValue("--learn-this").trim() || "#4cc38a", LN_NOT = LN_CSS.getPropertyValue("--learn-not").trim() || "#e5534b", LN_SAME = 0.002, LN_BAR = 3, LN_PULSE_MS = 450;
const LN = { sel: null, mark: null, pending: 0, busy: 0, chain: Promise.resolve(), cache: {}, sig: "" };
function lnReset() { LN.sel = null; LN.mark = null; LN.sig = ""; }
function lnLearn(name) { const t = S.tracks[name]; return t && t.learn && Array.isArray(t.learn.marks) && t.learn.marks.length ? t.learn : null; }
function lnTarget(name) { const t = S.tracks[name]; return t && t.follows && S.tracks[t.follows] ? t.follows : name; }   // a follower marks its source
function lnCounts(learn) {
  const c = { this: 0, not: 0 };
  for (const m of (learn && learn.marks) || []) c[m.kind === "not" ? "not" : "this"] += (m.at || []).length;
  return c;
}
function lnTimes(name) {   // the example times of a track as the lanes draw them (the Hitpoint shift is back on), sorted, cached per learn object
  const learn = lnLearn(name), t = S.tracks[name];
  const shift = t && t.sr && t.params ? (t.params.shift_samples || 0) / t.sr : 0;
  const c = LN.cache[name];
  if (c && c.learn === learn && c.shift === shift) return c;
  const out = { learn, shift, this: [], not: [] };
  for (const m of (learn && learn.marks) || []) for (const a of m.at || []) out[m.kind === "not" ? "not" : "this"].push(a + shift);
  out.this.sort((a, b) => a - b); out.not.sort((a, b) => a - b);
  return (LN.cache[name] = out);
}
function lnNear(sorted, t) {   // an example within LN_SAME of t (binary search)
  let lo = 0, hi = sorted.length;
  while (lo < hi) { const mid = (lo + hi) >> 1; if (sorted[mid] < t - LN_SAME) lo = mid + 1; else hi = mid; }
  return lo < sorted.length && sorted[lo] <= t + LN_SAME;
}
function lnKindOf(name, t) { const c = lnTimes(name); return c.not.length && lnNear(c.not, t) ? "not" : c.this.length && lnNear(c.this, t) ? "this" : null; }
function lnHitsIn(name, t0, t1) { return (S.events[name] || []).filter(e => e.t >= t0 && e.t <= t1 && !hgIsCut(e)).length; }
function lnRedraw(name) { if (name) invalidateHits(name); }
function lnSetSel(track, t0, t1) {
  const was = LN.sel ? LN.sel.track : null;
  LN.sel = track === null ? null : { track, t0: Math.min(t0, t1), t1: Math.max(t0, t1) };
  if (LN.sel) LN.mark = null;
  lnRedraw(was); lnRedraw(track); drawRuler(); drawInfo(); lnInspector();
  selShellSync();   // #158 (rank 9): a range selection enables Delete and the nudge items
}
function lnClearSel() { if (LN.sel) lnSetSel(null); }
function lnSetMark(track, id) {
  const was = LN.mark ? LN.mark.track : null;
  LN.mark = track ? { track, id } : null;
  LN.pulse = track ? { until: performance.now() + LN_PULSE_MS } : null;   // the ticks of the selected mark pulse once
  if (track) setTimeout(() => { if (LN.pulse && performance.now() >= LN.pulse.until) { LN.pulse = null; lnRedraw(track); } }, LN_PULSE_MS + 20);
  lnRedraw(was); lnRedraw(track); lnInspector();
}
function lnSelText() {
  const s = LN.sel; if (!s) return "";
  const n = lnHitsIn(s.track, s.t0, s.t1);
  return `selection ${fmt(s.t0, 3)} to ${fmt(s.t1, 3)} s (${fmt(s.t1 - s.t0, 3)} s), ${n} ${n === 1 ? "hit" : "hits"}`;
}
/* the drawing: the band (both lanes, under the hit lines) and the marks (spike lane, over them) */
function lnDrawBand(ctx, W, H, name) {
  const s = LN.sel; if (!s || s.track !== name) return;
  const x0 = Math.max(0, tToX(s.t0, W)), x1 = Math.min(W, tToX(s.t1, W));
  if (x1 < 0 || x0 > W) return;
  ctx.globalAlpha = 0.2; ctx.fillStyle = "#4da3ff"; ctx.fillRect(x0, 0, Math.max(1, x1 - x0), H);
  ctx.globalAlpha = 0.75; ctx.fillRect(Math.round(x0), 0, 1, H); ctx.fillRect(Math.round(x1) - (x1 - x0 < 1 ? 0 : 1), 0, 1, H);
  ctx.globalAlpha = 1;
}
function lnBarSpan(m, W) {   // the x range of a mark bar: at least 6 px (one hit)
  const a = tToX(m.t0, W), b = tToX(m.t1, W), mid = (a + b) / 2, w = Math.max(6, b - a);
  return [mid - w / 2, mid + w / 2];
}
function lnDrawMarks(ctx, W, H, name) {
  const learn = lnLearn(name); if (!learn) return;
  const dim = learn.on === false, shift = lnTimes(name).shift;
  for (const m of learn.marks) {
    const [x0, x1] = lnBarSpan(m, W);
    if (x1 < -6 || x0 > W + 6) continue;
    const sel = LN.mark && LN.mark.track === name && LN.mark.id === m.id, col = m.kind === "not" ? LN_NOT : LN_THIS;
    const pulse = sel && LN.pulse && performance.now() < LN.pulse.until, r = pulse ? 3.5 : 2;   // the ticks of a selected mark pulse once
    ctx.globalAlpha = dim ? 0.4 : 1; ctx.fillStyle = col;
    ctx.fillRect(x0, 0, x1 - x0, sel ? LN_BAR + 1 : LN_BAR);   // the bar and the ticks stay in the top 4 px (concept 2.4)
    if (sel) { ctx.strokeStyle = "#fff"; ctx.lineWidth = 1; ctx.strokeRect(x0 + 0.5, 0.5, x1 - x0 - 1, LN_BAR); }
    for (const a of m.at || []) {
      const x = Math.round(tToX(a + shift, W)) + 0.5;
      if (x < -4 || x > W + 4) continue;
      if (m.kind === "not") { ctx.strokeStyle = col; ctx.lineWidth = 1.5; ctx.beginPath(); ctx.moveTo(x - r, 2 - r); ctx.lineTo(x + r, 2 + r); ctx.moveTo(x + r, 2 - r); ctx.lineTo(x - r, 2 + r); ctx.stroke(); }
      else { ctx.beginPath(); ctx.arc(x, 2, r, 0, 6.2832); ctx.fill(); }
    }
  }
  ctx.globalAlpha = 1; ctx.lineWidth = 1;
}
function lnMarkAt(name, px, py) {   // the mark bar under a point of the spike lane (its top 6 px), the last one drawn
  const learn = lnLearn(name); if (!learn || py > 6) return null;
  const W = laneWidth(); let hit = null;
  for (const m of learn.marks) { const [x0, x1] = lnBarSpan(m, W); if (px >= x0 - 1 && px <= x1 + 1) hit = m; }
  return hit;
}
/* the gesture: a drag in the wave lane (more than 3 px) selects; a click is the playhead, as before, and clears the selection */
function lnClampT(name, t) { const d = (S.tracks[name] && S.tracks[name].duration) || maxDur(); return Math.max(0, Math.min(t, d)); }
function lnRangeGesture(ev, name) {
  S.followHold = true;
  const W = laneWidth(), startCX = ev.clientX, startX = laneXOf(ev), t0 = lnClampT(name, xToT(startX, W));
  let moved = false;
  const mv = ev2 => {
    if (!moved && Math.abs(ev2.clientX - startCX) <= 3) return;
    moved = true;
    lnSetSel(name, t0, lnClampT(name, xToT(laneXOf(ev2), W)));
  };
  const up = () => {
    window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up);
    if (moved) {
      if (S.selTrack !== name) setSelTrack(name);
      setMsg(`${name}: ${lnSelText()} (K: Learn This, Shift+K: Forget This)`);
      return;
    }
    S.playhead = snapPlayhead(xToT(startX, W));
    lnClearSel(); invalidate(F.HEAD); updateStatus();
  };
  window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
}
/* the commands: one function per menu item; the key and the buttons call them too */
function lnQueue(fn) {   // the calls run one after the other; an undo waits while one is on its way
  LN.pending++;
  // P4-6c: the hit edits on their way first: a Learn change with the filter on re-picks in the service, keeping them
  const p = LN.chain.then(async () => { await flushEvents(); LN.busy++; try { return await fn(); } finally { LN.busy--; } }).catch(e => setMsg(`learn failed: ${errText(e)}`, "error")).finally(() => { LN.pending--; lnInspector(); });
  LN.chain = p;
  return p;
}
function lnUndoWaits() { return LN.pending > 0; }
function lnSnap(track) { return (S.events[track] || []).map(({ id, ...e }) => ({ ...e })); }   // P4-6c: the hits as the service stores them
function lnApply(track, learn, r) {   // the new learn object of a track from the service; returns the one before.
  // P4-6c: `r` is the command's answer; with the filter on (or just turned off) the service marked the track's hits
  // again (r.filter.events: the same hits, the grey ones "filtered": true; hand deletions and velocities stay). The
  // hits before and after go into LN.snap: the undo step restores them exactly (track_learn_set with events)
  const t = S.tracks[track], before = t.learn || null;
  if (learn) t.learn = learn; else delete t.learn;   // an object without marks stays (Forget keeps the settings)
  LN.snap = null;
  if (r && r.filter) { const evBefore = lnSnap(track); lnAdopt(track, r.filter); LN.snap = { evBefore, evAfter: lnSnap(track) }; }
  lnScoreDrop(track);
  lnRedraw(track); lnInspector(); drawInfo(); setDirtyState(true);
  if (window.DSShell && DSShell.sync) DSShell.sync();   // Hits > Learn Filter shows the filter of the selected track
  return before;
}
function lnPush(r, before, label, t) {   // P4-6c: one undo step of a Learn change: the learn object and, when the hits changed, their snapshot; P4-40a: its label and place
  pushUndo({ type: "learn", track: r.track, before, after: r.learn || null, ...(LN.snap || {}),
    ...(label ? { label, where: { track: r.track, t: Number.isFinite(t) ? t : null } } : {}) });
}
function lnAdopt(track, f) {   // P4-6c: the service's hits after a Learn change; the page ids and the selected hit stay
  const old = S.events[track] || [], selT = S.sel && S.sel.track === track && old[S.sel.index] ? old[S.sel.index].t : null;
  S.events[track] = adoptHits(f.events, old, [], []);
  EDGEN[track] = (EDGEN[track] || 0) + 1;   // the list was replaced (an edit plan made on the old one is void)
  if (selT !== null) {
    const i = S.events[track].findIndex(e => Math.abs(e.t - selT) < 1e-6);
    S.sel = i >= 0 ? { track, index: i } : null;
  }
  const e = LN_SC[lnTarget(track)]; if (e) e.drag = false;   // a threshold drag's preview ends with the stored hits
  invalidateHits(track); refreshCounts(track); updateStatus();
  const fol = Array.isArray(f.followers) ? f.followers : [];
  if (fol.length) reloadFollowers(fol);
  if (INS.name === track) hgLoad();
}
const LN_CANDIDATE_REACH = 0.030;   // the same reach as the service's sweep (crates/app/src/learn.rs CANDIDATE_REACH)
function lnSweepLocal(track, learn) {   // P4-38b: the learned+added hits the service's sweep dropped leave the page at once (not stale until the poll)
  const at = (((learn || {}).marks || []).filter(m => m.kind === "this")).flatMap(m => m.at || []);
  const old = S.events[track] || [];
  const kept = old.filter(e => !(e.learned && e.added && !at.some(x => Math.abs(x - e.t) <= LN_CANDIDATE_REACH)));
  if (kept.length === old.length) return false;
  S.events[track] = kept;
  EDGEN[track] = (EDGEN[track] || 0) + 1;   // the list was replaced (an edit plan made on the old one is void)
  invalidateHits(track); refreshCounts(track); updateStatus();
  return true;
}
const LN_HINT_NOT = "Forget This alone takes only this hit and near copies: add a Learn This example (K) so similar bleed goes too.";
const LN_HINT_OFF = "Learn filter off: Shift+K on bleed or tick Filter to use it";
function lnHint(learn) {   // P4-6c: the hint of the Learn section: Forget This alone, with the threshold following the examples
  const c = lnCounts(learn);
  return learn && learn.on === true && learn.auto === true && c.this === 0 && c.not > 0 ? LN_HINT_NOT : "";
}
function lnResult(r) {   // P4-6c: the hits and the grey hits after a Learn change, for the status line
  if (r && r.filter_error) return ` (the filter applies on the next Re-analyze: ${r.filter_error})`;
  if (!r || !r.filter) return "";
  const c = lnOutCounts(r.track), hint = lnHint(r.learn);
  return `. ${c.live} hits, Learn takes ${c.learnOut} out${hint ? `. ${hint}` : ""}`;   // the same counts as the Learn row
}
/* P4-35: Learn guard. One Learn step (mark, remove, flip, forget, filter on or off) that takes out many more hits than
   before gets a dialog: Keep, or Undo (the normal undo step of that change). The counts come from the service's answer
   (learn_out_before, learn_out_after, hits, hits_before); a threshold drag is an explicit choice and never asks. */
function lnGuardLimit(liveBefore) { return Math.max(10, 0.25 * liveBefore); }   // P4-35: the hits one filter step may take out (10, or 25 % of the output hits before)
function lnGuardCuts(r) {   // the number of hits the step takes out beyond the limit lnGuardLimit, else 0
  if (!r || !r.filter || !Number.isFinite(r.learn_out_after) || !Number.isFinite(r.learn_out_before)) return 0;
  const more = r.learn_out_after - r.learn_out_before, liveBefore = (r.hits_before || 0) - r.learn_out_before;
  return more > lnGuardLimit(liveBefore) ? more : 0;
}
function lnGuardAdds(r) {   // P4-38b: the hits the step ADDS beyond max(10, 25 % of the live hits before), else 0
  if (!r || !Number.isFinite(r.hits) || !Number.isFinite(r.hits_before)) return 0;
  const more = r.hits - r.hits_before;
  return more > Math.max(10, 0.25 * r.hits_before) ? more : 0;
}
function lnGuard(r) {   // not awaited by the Learn queue: the Undo button joins that queue
  const adds = lnGuardAdds(r);
  if (!lnGuardCuts(r)) {
    if (!adds) return;
    askConfirm({ title: `Learn adds many hits to ${r.track}`,
      msg: `Learn now adds ${adds} hits (before: ${r.hits_before}). The examples may be mixed up.`,
      buttons: [{ label: "Keep", act: "keep" }, { label: "Undo", act: "undo", cls: "primary" }] }).then(async a => {
      if (a !== "undo") return;
      await LN.chain;   // the step is on top of the undo stack once the queue is empty
      undo();
    });
    return;
  }
  askConfirm({ title: `Learn takes many hits out of ${r.track}`,
    msg: `Learn now takes ${r.learn_out_after} of ${r.hits} hits out (before: ${r.learn_out_before}). The examples may be mixed up.`,
    buttons: [{ label: "Keep", act: "keep" }, { label: "Undo", act: "undo", cls: "primary" }] }).then(async a => {
    if (a !== "undo") return;
    await LN.chain;   // the step is on top of the undo stack once the queue is empty
    undo();
  });
}
function lnOutCounts(target) {   // owner check 2026-10-15: count only what Learn changes; hits the bleed rules or the loudness cut already took out are not "filtered"
  const evs = S.events[target] || [], other = x => (!!x.cut || !!x.bleed) && !isManual(x);
  return { live: evs.filter(x => !hgIsCut(x)).length, learnOut: evs.filter(x => lnGreyHit(target, x) && !other(x)).length };
}
function lnWhat(r) {
  const c = r.counts || lnCounts(r.learn), n = r.mark ? r.mark.at.length : 0;
  const via = r.routed ? ` on ${r.track}` : "";
  return { n, c, via };
}
function learnMark(kind) {   // Hits > Learn This (K), Forget This (Shift+K)
  if (!needProject()) return false;
  let name, a, hit = null, hitId = null;
  if (LN.sel) { name = LN.sel.track; a = { t0: LN.sel.t0, t1: LN.sel.t1 }; }
  else if (S.sel && (S.events[S.sel.track] || [])[S.sel.index]) { name = S.sel.track; hitId = S.events[name][S.sel.index].id; hit = S.events[name][S.sel.index].t; a = { hit }; }
  else { setMsg("select a range or a hit first (drag in a wave lane, or click a hit)"); return false; }
  const track = S.tracks[name], routed = track && track.follows && S.tracks[track.follows] ? track.follows : null;
  return lnQueue(async () => {
    await flushEvents();   // the hit edits on their way first (P4-9 stage 2)
    if (hitId !== null) {   // the time of the hit when the call runs: a nudge asked before it has moved the hit
      const e = (S.events[name] || []).find(x => x.id === hitId);
      if (!e) { setMsg(`${name}: the hit is not there any more`); return; }
      a = { hit: e.t };
    }
    const r = await B.learnMark(S.sid, name, kind, a.t0, a.t1, a.hit);
    if (!r.marked) { setMsg(r.reason === "already marked" ? `${name}: ${hit === null ? "those hits are" : "that hit is"} already ${kind === "this" ? "learned" : "marked not"}` : `${name}: no hit in the selection`); return; }
    // P4-6c (controller 2026-10-13): the first Shift+K of a track whose filter is off turns the filter on in the
    // service (r.filter_on), with the threshold following the examples; K alone never does. One undo step
    // (track_learn_set with the learn before re-picks again)
    const before = lnApply(r.track, r.learn, r);
    lnPush(r, before, kind === "this" ? "Learn This" : "Forget This", a.hit !== undefined ? a.hit : a.t0); lnGuard(r);
    lnSetSel(null);
    const w = lnWhat(r), off = !r.learn || r.learn.on !== true;
    const matchHint = kind === "not" && r.learn && r.learn.on && !lnHint(r.learn) ?   // P4-37b: how strict the current Match is (not next to the "Forget This alone" hint)
      `. The current Match takes ${lnOutCounts(r.track).learnOut} similar ${lnOutCounts(r.track).learnOut === 1 ? "hit" : "hits"} out - drag the Match line in the histogram for more or fewer` : "";
    const pre = routed ? `learned on ${r.track} (${name} follows it): ` : `${name}: `;
    if (r.added_hit != null && r.learn) {   // P4-38b: K on a missed hit added it: the status says what the Add rule does from here (P4-38d: with the filter off too, which a K-add never turns on)
      const tail = `. Learn adds ${lnAddCount(r.track, lnAddOf(r.track))} similar hits (Add ${lnAddOf(r.track).toFixed(2)}) - drag the Add line for more or fewer`;
      setMsg(`${pre}added 1 hit at ${r.added_hit.toFixed(3)} s and learned it (${w.c.this} learn, ${w.c.not} forget)${r.filter_error ? ` (the filter applies on the next Re-analyze: ${r.filter_error})` : ""}${tail}`);
      return;
    }
    setMsg(`${pre}${kind === "this" ? "learned" : "Forget This on"} ${w.n} ${w.n === 1 ? "hit" : "hits"}${r.flipped ? `, ${r.flipped} changed kind` : ""} (${w.c.this} learn, ${w.c.not} forget)${r.filter_on ? ", Learn filter on" : ""}${lnResult(r)}${off && kind === "this" ? `. ${LN_HINT_OFF}` : ""}${matchHint}`);
  });
}
function lnTrackFor() { return S.selTrack || (LN.sel && LN.sel.track) || (LN.mark && LN.mark.track) || (S.sel && S.sel.track) || null; }
function learnForget() {   // Hits > Forget Examples of Track
  if (!needProject()) return false;
  const name = lnTrackFor();
  if (!name) { setMsg("no track selected: click a track header first"); return false; }
  const target = lnTarget(name);
  if (!lnLearn(target)) { setMsg(`${target}: no Learn examples to forget`); return false; }
  return lnQueue(async () => {
    const r = await B.learnForget(S.sid, name);
    const before = lnApply(r.track, r.learn, r);
    lnPush(r, before, "Forget Examples"); lnGuard(r);
    if (LN.mark && LN.mark.track === r.track) lnSetMark(null);
    setMsg(`${r.track}: forgot ${r.examples} ${r.examples === 1 ? "example (Undo brings it back)" : "examples (Undo brings them back)"}${lnResult(r)}`);
  });
}
function learnForgetThis() {   // P4-36: Hits > Remove Example (Option+K): the example at the selected range or hit, else the selected mark bar
  if (!needProject()) return false;
  let name, a, hitId = null;
  if (LN.sel) { name = LN.sel.track; a = { t0: LN.sel.t0, t1: LN.sel.t1 }; }
  else if (S.sel && (S.events[S.sel.track] || [])[S.sel.index]) { name = S.sel.track; hitId = S.events[name][S.sel.index].id; a = {}; }
  else if (LN.mark) return learnRemove(LN.mark.track, LN.mark.id);
  else { setMsg("Remove Example: select a hit, a range or a mark first"); return false; }
  if (!lnLearn(lnTarget(name))) { setMsg(`${lnTarget(name)}: no Learn example to forget`); return false; }
  return lnQueue(async () => {
    await flushEvents();   // the time of the hit when the call runs, as in learnMark
    if (hitId !== null) {
      const e = (S.events[name] || []).find(x => x.id === hitId);
      if (!e) { setMsg(`${name}: the hit is not there any more`); return; }
      a = { hit: e.t };
    }
    const r = await B.learnForgetAt(S.sid, name, a.t0, a.t1, a.hit);
    if (!r.removed) { setMsg(`${r.track}: no Learn example ${a.hit === undefined ? "in the selection" : "at that hit"}`); return; }   // nothing changed: no undo step
    const evBefore = r.swept ? lnSnap(r.track) : null;   // the sweep drops hits: snapshot them for the undo step
    const before = lnApply(r.track, r.learn, r);
    if (evBefore) { lnSweepLocal(r.track, r.learn); LN.snap = { evBefore, evAfter: lnSnap(r.track) }; }
    lnPush(r, before, "Remove Example", a.hit !== undefined ? a.hit : a.t0); lnGuard(r);
    if (LN.mark && LN.mark.track === r.track && !(r.learn && r.learn.marks.some(m => m.id === LN.mark.id))) lnSetMark(null);
    if (a.hit === undefined) lnSetSel(null);
    setMsg(`${r.track}: forgot ${r.removed} ${r.removed === 1 ? "example (Undo brings it back)" : "examples (Undo brings them back)"}${lnResult(r)}`);
  });
}
function learnRemove(track, id) {   // a mark bar: Remove Example, or Delete with the bar selected
  const m = lnLearn(track) && lnLearn(track).marks.find(x => x.id === id);
  if (!m) return false;
  return lnQueue(async () => {
    const r = await B.learnRemove(S.sid, track, id);
    const before = lnApply(r.track, r.learn, r);
    lnPush(r, before, "Remove Example"); lnGuard(r);
    if (LN.mark && LN.mark.id === id) lnSetMark(null);
    setMsg(`${r.track}: removed ${r.removed} ${r.removed === 1 ? "example" : "examples"}${lnResult(r)}`);
  });
}
function learnFlip(track, id) {
  if (!lnLearn(track)) return false;
  return lnQueue(async () => {
    const r = await B.learnFlip(S.sid, track, id);
    const before = lnApply(r.track, r.learn, r);
    lnPush(r, before, "Flip Example"); lnGuard(r);
    setMsg(`${r.track}: mark ${id} is now ${r.kind === "this" ? "Learn This" : "Forget This"}${lnResult(r)}`);
  });
}
function lnDeleteMark() {   // Delete with a mark bar selected (and no hit): true when it took the key
  if (!LN.mark || S.sel) return false;
  learnRemove(LN.mark.track, LN.mark.id);
  return true;
}
/* ---------------- P4-6c: the similarity scores in the page (the filter, the histogram, the grey hits) ----------------
   The filter runs in the service: a Learn change with the filter on re-picks the track there (lnApply adopts the
   stored hits), and a hit the filter takes out stays a hit marked "filtered": true, drawn grey (hgIsCut), skipped
   by Tab, out of MIDI, spikes, clicks and followers. The scores of a track (track_learn_scores,
   crates/app/src/learn_score.rs) feed the histogram, the info line and the live grey of a threshold drag; they
   are cached per (sid, target track, learn object), and a Learn change drops them (lnScoreDrop). */
const LN_SC = {};
function lnFilterOn(name) {   // the filter draws grey only when the track's learn says on
  const t = S.tracks[name], learn = t && t.learn;
  return !!(learn && learn.on === true && Array.isArray(learn.marks) && learn.marks.length);
}
function lnScoreSig(name) {
  return JSON.stringify([S.sid, lnTarget(name), S.tracks[lnTarget(name)] && S.tracks[lnTarget(name)].learn || null]);
}
function lnScoreDrop(name) {   // the learn changed: drop the cache and refetch (the grey hits and the histogram follow)
  const target = lnTarget(name);
  delete LN_SC[target];
  lnScores(target).catch(() => {});
}
function lnScores(name) {   // the data of the target track (async; the sync map follows when it lands)
  const target = lnTarget(name), sig = lnScoreSig(name);
  let e = LN_SC[target];
  if (!e || e.sig !== sig) {
    e = LN_SC[target] = { sig, data: null, byT: new Map(), byGate: new Map(), threshold: 0.5 };
    e.p = B.learnScores(S.sid, target).then(r => {
      if (r && r.state === "ok") {
        e.data = r;
        e.threshold = r.threshold;
        e.byT = new Map(r.candidates.map(row => [Math.round(row[0] * 1000) / 1000, row[2]]));
        e.byGate = new Map(r.candidates.map(row => [Math.round(row[0] * 1000) / 1000, row[4] === true]));   // #293 B8: the attack gate keeps it out
      }
      lnRedraw(target);
      const row = document.querySelector("#insParams .lnRow"); if (row) { lnHistDraw(target); invalidateHits(target); }
      return e;
    }).catch(() => e);
  }
  return e.p;
}
function lnScoreSync(name) {   // the sync view of the data: {on, threshold, byT, byGate} or null (the first draw after a mark)
  const target = lnTarget(name), e = LN_SC[target];
  return e && e.data && e.data.state === "ok" ? { on: lnFilterOn(name), threshold: e.threshold, byT: e.byT, byGate: e.byGate } : null;
}
function lnNearestCand(map, t) {   // the value of the candidate nearest to t (unshifted times) in a map keyed by time, or null
  let best = null, bd = 0.051;
  for (const [ct, v] of map) { const d = Math.abs(ct - t); if (d <= bd) { bd = d; best = v; } }
  return best;
}
function lnScoreAt(name, t) {   // the score of the candidate nearest to t (unshifted times), or null
  const sc = lnScoreSync(name); return sc ? lnNearestCand(sc.byT, t) : null;
}
function lnShift(name) { const t = S.tracks[name]; return t && t.sr && t.params ? (t.params.shift_samples || 0) / t.sr : 0; }
function lnPreview(name, e) {   // P4-6c: during a threshold drag the grey of a hit follows the live threshold (null: no drag)
  const x = LN_SC[lnTarget(name)]; if (!x || !x.drag || !lnFilterOn(name)) return null;
  if (isManual(e) || e.keep === true || e.deactivated === true) return false;   // an edited hit is never filtered; the user's overrides win (#242)
  const lnT = lnTimes(name);
  if (lnT.this.length && lnNear(lnT.this, e.t)) return false;   // a "this" example stays
  if (lnT.not.length && lnNear(lnT.not, e.t)) return true;
  const s = lnScoreAt(name, e.t - lnShift(name));
  const sc = lnScoreSync(name), gated = sc !== null && lnNearestCand(sc.byGate, e.t - lnShift(name)) === true;   // #293 B8: the attack gate keeps it out, as the service stores it
  return s !== null && s < x.threshold && !gated;
}
function lnGreyHit(name, e) {   // P4-6c: a grey hit of the Learn filter (the stored mark, or the drag's preview)
  const pv = lnPreview(name, e);
  return pv === null ? !!e.filtered && !isManual(e) && e.keep !== true && e.deactivated !== true : pv;
}
function lnHistDraw(name) {   // the Similarity histogram of the Learn row (concept 3.2)
  const cv = document.querySelector("#inspector .lnHist"); if (!cv) return;
  const target = lnTarget(name), e = LN_SC[target], ctx = cv.getContext("2d");
  const dpr = devicePixelRatio || 1, W = cv.width = cv.clientWidth * dpr, H = cv.height = 60 * dpr;
  ctx.clearRect(0, 0, W, H);
  const info = document.querySelector("#inspector .lnHistInfo");
  const sc = lnScoreSync(name); if (!sc || !e || !e.data || e.data.state !== "ok") { ctx.fillStyle = "#88909c"; ctx.textBaseline = "alphabetic"; ctx.font = `${11 * dpr}px sans-serif`; ctx.fillText("no scores yet", 8, H / 2); if (info) info.textContent = ""; return; }
  const rows = e.data.candidates, sens = Number.isFinite(e.data.sens) ? e.data.sens : 150;
  const NB = 50, bins = new Array(NB).fill(0), binsOut = new Array(NB).fill(0);
  for (const row of rows) { const b = Math.min(NB - 1, Math.max(0, Math.floor(row[2] * NB))); if (row[3] <= sens) bins[b]++; else binsOut[b]++; }
  if (info) {   // the counts under the histogram (concept 2.7): the hits, the grey ones, the Match strictness
    const { live, learnOut } = lnOutCounts(target);
    const addPart = lnHasThis(name) ? ` · Add ${lnAddOf(name).toFixed(2)} (adds ${lnAddCount(name, lnAddOf(name))})` : "";
    info.textContent = `${live} hits · Learn takes ${learnOut} out · Match ${sc.threshold.toFixed(2)}${e.drag ? "" : lnAutoOn(target) ? " auto" : ""}${addPart}`;
  }
  // #210: the labels live above the bars. Lay them out first (font and margins scale with dpr), so the
  // bar area can shrink by the label band and no glyph is clipped at the top of the canvas.
  const fontPx = 10 * dpr, topPad = 2 * dpr, lineH = Math.round(fontPx * 1.1);
  ctx.font = `${fontPx}px sans-serif`; ctx.textBaseline = "top";
  const thX = sc.threshold * W;
  const lbl = `Match ${sc.threshold.toFixed(2)}`, lw = ctx.measureText(lbl).width;
  const mx = Math.max(2, Math.min(W - lw - 2, thX + 4));
  let add = null, albl = "", aw = 0, axx = 0, ay = topPad;
  if (lnHasThis(name)) {   // P4-38b: the Add line (only with a Learn This example) and its live count
    add = e.dragAdd ?? lnAddOf(name);
    albl = `Add ${add.toFixed(2)} · adds ${lnAddCount(name, add)}`; aw = ctx.measureText(albl).width;
    axx = Math.max(2, Math.min(W - aw - 2, add * W - aw - 4));   // left of the Add line
    const clash = x2 => x2 < mx + lw + 2 && x2 + aw > mx - 2;
    if (clash(axx)) {   // would overlap the Match label: try the other side of the Add line, else a second line
      const rx = Math.max(2, Math.min(W - aw - 2, add * W + 4));
      if (clash(rx)) { ay = topPad + lineH; axx = Math.max(2, Math.min(W - aw - 2, add * W - aw - 4)); }
      else axx = rx;
    }
  }
  const labelBand = ay > topPad ? topPad + 2 * lineH : topPad + lineH;   // the rows the labels occupy
  const max = Math.max(1, ...bins, ...binsOut), bw = W / NB, barBot = H - 12 * dpr;
  for (let b = 0; b < NB; b++) {
    const h = bins[b] / max * (barBot - labelBand), ho = binsOut[b] / max * (barBot - labelBand);
    if (bins[b]) { ctx.fillStyle = "#4da3ff"; ctx.fillRect(b * bw + 1, barBot - h, bw - 2, h); }
    if (binsOut[b]) { ctx.strokeStyle = "#4da3ff"; ctx.strokeRect(b * bw + 1.5, barBot - ho, bw - 3, ho); }
  }
  ctx.strokeStyle = "#e5534b"; ctx.beginPath(); ctx.moveTo(thX, 0); ctx.lineTo(thX, barBot); ctx.stroke();
  ctx.fillStyle = "#e5534b";   // P4-37b: the strictness label
  ctx.fillText(lbl, mx, topPad);
  if (add !== null) {
    ctx.strokeStyle = "#4cc38a"; ctx.beginPath(); ctx.moveTo(add * W, 0); ctx.lineTo(add * W, barBot); ctx.stroke();
    ctx.fillStyle = "#4cc38a";
    ctx.fillText(albl, axx, ay);
  }
  ctx.fillStyle = "#4cc38a";   // the this ticks
  for (const x of (e.data.examples.this || []).map(x2 => x2[1] * W)) { ctx.fillRect(x - 1.5, H - 10 * dpr, 3, 4 * dpr); }
  ctx.fillStyle = "#e5534b";   // the not ticks
  for (const x of (e.data.examples.not || []).map(x2 => x2[1] * W)) { ctx.fillRect(x - 1.5, H - 6 * dpr, 3, 4 * dpr); }
}
function lnAutoOn(name) { const l = lnLearn(name); return !!(l && l.auto === true); }
function lnAddOf(name) { const l = lnLearn(name); return l && typeof l.add === "number" ? l.add : 0.85; }   // P4-38b
function lnHasThis(name) { const t = lnTimes(name); return t.this.length > 0; }
function lnAddCount(name, add) {   // P4-38b: how many below-Sens candidates the Add rule would add
  const target = lnTarget(name), e = LN_SC[target];
  if (!e || !e.data || e.data.state !== "ok") return 0;
  const sens = Number.isFinite(e.data.sens) ? e.data.sens : 150;
  let n = 0;
  for (const row of e.data.candidates) {
    if (row[3] <= sens) continue;
    const s = lnScoreAt(name, row[0]);
    if (s !== null && s >= add) n++;
  }
  return n;
}
function lnSetLearn(name, change, msg) {   // P4-6c: one change of the learn object (filter on/off, threshold, Auto): one undo step
  const target = lnTarget(name), cur = S.tracks[target] && S.tracks[target].learn; if (!cur) return false;
  const next = Object.assign({}, cur, change);
  return lnQueue(async () => {
    const r = await B.learnSet(S.sid, target, next);
    const before = lnApply(r.track, r.learn, r);
    lnPush(r, before, "on" in change ? `Learn Filter ${change.on ? "On" : "Off"}` : "Learn Settings"); if ("on" in change) lnGuard(r);   // filter on or off asks; a threshold or Auto change is an explicit choice
    setMsg(`${r.track}: ${msg}${lnResult(r)}`);
  });
}
function lnHistSetThreshold(name, th) {   // the release of a threshold drag: the threshold is the engineer's now (auto off)
  const v = Math.round(th * 1000) / 1000;
  return lnSetLearn(name, { threshold: v, auto: false }, `similarity threshold ${v.toFixed(2)}`);
}

function lnApplyOp(op, reverse) {   // applyOp: undo and redo of a Learn step
  const learn = (reverse ? op.before : op.after) || null, events = reverse ? op.evBefore : op.evAfter;   // P4-6c: the snapshot, exactly
  lnQueue(async () => {
    const r = await B.learnSet(S.sid, op.track, learn, events);
    lnApply(r.track, r.learn, r);
    if (LN.mark && !lnLearn(op.track)) lnSetMark(null);
    setMsg(`${reverse ? "undo" : "redo"}: Learn marks of ${op.track}`);
  });
}
function learnFilter() {   // Hits > Learn Filter (a check item; the shell syncs it): the check box of the Learn row
  const name = lnTrackFor(), learn = name && lnLearn(lnTarget(name));
  if (!learn || !(learn.marks || []).length) { setMsg("no Learn examples: mark hits with K or Shift+K first"); return false; }
  return lnSetLearn(name, { on: learn.on !== true }, `Learn filter ${learn.on !== true ? "on" : "off"}`);
}
/* the context menus: right-click inside the selection (Learn This, Forget This, Clear Selection) or on a mark bar */
function lnContext(ev, name, lane) {
  const px = laneXOf(ev), W = laneWidth();
  let items = null;
  const m = lane === "spike" ? lnMarkAt(name, px, ev.offsetY) : null;
  if (m) {
    lnSetMark(name, m.id);
    items = [["remove", "Remove Example", () => learnRemove(name, m.id)],
      ["flip", m.kind === "this" ? "Change to Forget This" : "Change to Learn This", () => learnFlip(name, m.id)]];
  } else if (LN.sel && LN.sel.track === name) {
    const t = xToT(px, W);
    if (t >= LN.sel.t0 && t <= LN.sel.t1) items = [["learn", "Learn This", () => learnMark("this")], ["not", "Forget This", () => learnMark("not")], ["resel", "Re-analyze Selection", () => reanalyzeSelected()], ["clear", "Clear Selection", () => lnClearSel()]];   // #265: the range only, like the Hits menu item
  }
  if (!items) return false;
  ev.preventDefault(); openCtxItems(items, ev.clientX, ev.clientY);
  return true;
}
/* the Inspector Learn row: counts or the hint, the buttons, the collapsed example list */
function lnInspector() {
  const row = document.querySelector("#insParams .lnRow"); if (!row || !INS.name) return;
  const name = INS.name, tgt = lnTarget(name), learn = lnLearn(tgt), c = lnCounts(learn);
  const can = !!LN.sel || !!(S.sel && (S.events[S.sel.track] || [])[S.sel.index]);
  const sig = JSON.stringify([name, tgt, learn, can, LN.mark, LN.pending]);
  if (sig === LN.sig) return;
  LN.sig = sig; insSumRefresh();
  const follows = tgt !== name;
  row.querySelector(".lnCount").textContent = learn
    ? `${follows ? `on ${tgt}: ` : ""}✓ ${c.this}  ✗ ${c.not}${learn.on === false ? "  (filter off)" : ""}${c.this + c.not === 1 ? "  · 1 example: add 2 more for a stable result" : ""}`
    : follows ? `follows ${tgt}: no examples there` : "Select hits you want and press K";
  const hintEl = row.querySelector(".lnHintMsg"), hint = lnHint(learn);   // P4-6c: Forget This alone
  if (hintEl) { hintEl.textContent = hint; hintEl.classList.toggle("hidden", !hint); }
  row.querySelector(".lnThis").disabled = !can; row.querySelector(".lnNot").disabled = !can;
  row.querySelector(".lnForgetThis").disabled = !learn || !(can || LN.mark);
  row.querySelector(".lnForget").disabled = !learn;
  const box = row.querySelector(".lnFilterBox");   // P4-6c: the checkbox follows the stored state
  if (box) { box.disabled = !learn || !Array.isArray(learn.marks) || !learn.marks.length; box.checked = !!(learn && learn.on === true); }
  const hist = row.querySelector(".lnHist"), on = lnFilterOn(tgt);   // concept 2.4: filter off, the histogram is hidden
  if (hist) { hist.classList.toggle("hidden", !on); row.querySelector(".lnHistRow").classList.toggle("hidden", !on); }
  if (hist && on && !hist.dataset.dragging) lnHistDraw(name);
  const rows = row.querySelector(".lnRows"); rows.textContent = "";
  for (const m of (learn && learn.marks) || []) {
    const d = document.createElement("div"); d.className = "lnMk" + (LN.mark && LN.mark.id === m.id && LN.mark.track === tgt ? " sel" : ""); d.dataset.id = m.id; d.dataset.kind = m.kind;
    const dot = document.createElement("span"); dot.className = `lnDot ${m.kind}`; dot.textContent = m.kind === "not" ? "✗" : "✓";
    const txt = document.createElement("span"); txt.className = "lnTxt";
    txt.textContent = `${m.id}  ${fmt(m.t0, 3)}${m.t1 !== m.t0 ? ` to ${fmt(m.t1, 3)}` : ""} s  ·  ${(m.at || []).length} ${(m.at || []).length === 1 ? "hit" : "hits"}`;
    const x = document.createElement("button"); x.className = "btn mini lnX"; x.textContent = "×"; x.title = "Remove this mark";
    x.onclick = ev => { ev.stopPropagation(); learnRemove(tgt, m.id); };
    d.onclick = () => {   // a click on a row selects the mark and moves the playhead to its first example (concept 2.7)
      lnSetMark(tgt, m.id);
      const t = ((m.at || [])[0] ?? m.t0) + lnTimes(tgt).shift;
      S.followHold = true; S.playhead = Math.max(0, t);
      if (t < S.view.t0 || t > S.view.t1) centerOn(t);
      invalidate(F.HEAD); updateStatus();
    };
    d.append(dot, txt, x); rows.appendChild(d);
  }
}
function lnBuildRow(div, name) {   // buildParamsPanel: the button handlers of the Learn row
  const row = div.querySelector(".lnRow"); if (!row) return;
  row.querySelector(".lnThis").onclick = () => learnMark("this");
  row.querySelector(".lnNot").onclick = () => learnMark("not");
  row.querySelector(".lnForgetThis").onclick = () => learnForgetThis();
  row.querySelector(".lnForget").onclick = () => learnForget();
  const box = row.querySelector(".lnFilterBox"), learn = S.tracks[name] && S.tracks[name].learn;
  box.disabled = !learn || !Array.isArray(learn.marks) || !learn.marks.length;
  box.checked = !!(learn && learn.on === true);
  box.onchange = () => lnSetLearn(name, { on: box.checked }, `Learn filter ${box.checked ? "on" : "off"}`);   // P4-6c: one undo step (the examples stay)
  const auto = row.querySelector(".lnAuto");
  auto.onclick = ev => {   // the threshold follows the examples again (one undo step)
    ev.preventDefault();
    if (!lnAutoOn(lnTarget(name))) lnSetLearn(name, { auto: true }, "similarity threshold: automatic");
    return false;
  };
  const hist = row.querySelector(".lnHist");
  hist.onmousemove = ev => {   // P4-40b: the resize cursor within 6 px of the Match line or the Add line
    const e = LN_SC[lnTarget(name)], r = hist.getBoundingClientRect(), x = ev.clientX - r.left;
    const ready = !!(e && e.data && e.data.state === "ok");
    hist.classList.toggle("onLine", ready && (Math.abs(x - e.threshold * r.width) <= 6 || (lnHasThis(name) && Math.abs(x - lnAddOf(name) * r.width) <= 6)));
  };
  hist.onmouseleave = () => hist.classList.remove("onLine");
  hist.onmousedown = ev => {   // the threshold drag: the grey hits follow live (lnPreview), one undo step on release.
    // P4-38b: a mousedown near the Add line (green) drags the Add threshold instead
    const target = lnTarget(name), e = LN_SC[target];
    if (!e || !e.data || e.data.state !== "ok" || ev.button !== 0) return;
    ev.preventDefault();
    const r0 = hist.getBoundingClientRect();
    const addShown = lnHasThis(name);
    const addX = lnAddOf(name) * r0.width;
    const dragAdd = addShown && Math.abs(ev.clientX - r0.left - addX) <= 6;
    hist.dataset.dragging = "1";
    if (dragAdd) { e.dragAdd = lnAddOf(name); } else { e.drag = true; }
    const start = e.threshold, startAdd = lnAddOf(name);
    const move = ev2 => {
      const r = hist.getBoundingClientRect();
      const v = Math.round(Math.min(1, Math.max(0, (ev2.clientX - r.left) / r.width)) * 1000) / 1000;
      if (dragAdd) { e.dragAdd = v; } else { e.threshold = v; }
      lnHistDraw(name); invalidateHits(target);
    }, up = () => {
      delete hist.dataset.dragging;
      document.removeEventListener("mousemove", move); document.removeEventListener("mouseup", up);
      if (dragAdd) {
        const v = e.dragAdd; delete e.dragAdd;
        if (Math.abs(v - startAdd) > 1e-9) lnSetLearn(name, { add: v }, `Add threshold ${v.toFixed(2)}`);
        else lnHistDraw(name);
      } else if (Math.abs(e.threshold - start) > 1e-9) lnHistSetThreshold(target, e.threshold);
      else { e.drag = false; lnHistDraw(name); invalidateHits(target); }
    };
    document.addEventListener("mousemove", move); document.addEventListener("mouseup", up);
    move(ev);
  };
  LN.sig = ""; lnInspector();
  lnScores(name).then(() => { lnHistDraw(name); invalidateHits(name); });   // the scores arrive: the grey hits and the histogram
}
window.DSLearn = {   // the e2e tests read the state through this
  state: () => ({ sel: LN.sel && { ...LN.sel }, mark: LN.mark && { ...LN.mark }, pending: LN.pending, text: lnSelText(),
    learn: Object.fromEntries(S.order.filter(n => S.tracks[n].learn).map(n => [n, S.tracks[n].learn])) }),
  times: name => { const c = lnTimes(name); return { this: c.this.slice(), not: c.not.slice(), shift: c.shift }; },
  barX: (name, id) => { const m = lnLearn(name).marks.find(x => x.id === id); return lnBarSpan(m, laneWidth()).map(x => x + GEO.laneX); },
  hitKind: (name, t) => lnKindOf(name, t),
  pulsing: () => !!(LN.pulse && performance.now() < LN.pulse.until),
  colors: () => ({ this: LN_THIS, not: LN_NOT }),
  /* P4-6c: the scores of a track (the histogram and the grey-hit checks read this) */
  scores: name => { const e = LN_SC[lnTarget(name)]; return e && e.data; },
  scoresReady: name => lnScores(name).then(e => e && e.data),   // the scores of the current learn object (a promise)
  idle: () => LN.chain.then(() => edIdle()),   // every queued Learn change and its re-pick is applied
  filterOn: name => lnFilterOn(name),
  greyHit: (name, t) => { const e = (S.events[name] || []).find(x => Math.abs(x.t - t) < 0.03); return !!e && lnGreyHit(name, e); },
};
