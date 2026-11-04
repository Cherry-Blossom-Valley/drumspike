/* Drum Spike v2 — review markers (refactor plan step 5.2, R9 A2)
   The review markers (MK): the copy of the marker list, the write queue, M and Option+M. The
   Markers ruler draws and edits through tempomap.js. Moved verbatim from app.js. Plain script (no
   build step), loaded after tempomap.js and before app.js. The names stay top-level bindings
   (page globals). */
"use strict";

/* ---------------- review markers (P4-40g) ----------------
   The markers live in the project document (the `markers` pass-through key) and come from the service
   (markers_get, markers_set): the page holds a copy (MK.list) to draw them, like the map (TM.map).
   markers_set replaces the whole list and answers `previous`, so one more call undoes it
   (docs/agent-api.md). M sets a marker at the selected hit or the playhead; the Markers ruler shows
   them (a click puts the playhead, a click on a flag selects it, Delete removes it, a double-click
   edits its note); Option+M and Option+Shift+M jump to the next and the previous marker. */
const MK = { list: [], items: [], chain: Promise.resolve(), pending: 0 };
function mkItems() {   // the ruler items: {id, i, t, text (the note, or the number), raw (the note), del}
  return MK.list.map((m, i) => { const note = typeof m.note === "string" ? m.note : ""; return { id: "m" + i, i, t: m.t, text: note || `M${i + 1}`, raw: note, del: true }; });
}
function mkQueue(fn) {   // every write runs in order (the tempo chain pattern)
  MK.pending++;
  return (MK.chain = MK.chain.then(fn).finally(() => { MK.pending--; }));
}
async function mkLoad() {   // with the project, and after every write: the copy of the marker list
  MK.list = [];
  if (S.sid && B.markersGet) {
    try { const r = await B.markersGet(S.sid); MK.list = Array.isArray(r.markers) ? r.markers : []; } catch (e) { /* no markers */ }
  }
  MK.items = mkItems();
  if (TM.sel && TM.sel.kind === "markers" && TM.sel.i >= MK.list.length) TM.sel = null;
  tmApplyUi(false);   // the ruler shows when the project has markers, until a toggle stores the choice
  invalidate(F.VIEW);
}
function mkCommit(list, what, selT) {   // the whole new list goes to the service; selT: the marker to select after
  const sid = S.sid;
  return mkQueue(async () => {
    if (sid !== S.sid) return false;
    try {
      const r = await B.markersSet(sid, list);
      if (sid !== S.sid) return false;
      await mkLoad();
      if (r.changed) { setDirtyState(true); setMsg(what); } else setMsg(`${what}: no change`);
      if (selT !== undefined) {   // the marker at selT stays selected (its index moved with the sort)
        const i = MK.list.findIndex(m => Math.abs(m.t - selT) < 1e-9);
        if (i >= 0) { TM.sel = { kind: "markers", id: "m" + i, i, del: true }; invalidate(F.VIEW); }
      }
      return true;
    } catch (e) { setMsg(`${what} refused: ${errText(e)}`); return false; }
  });
}
function markerAdd(t, note) {   // one more marker at t (the service stores the list sorted)
  if (!needProject()) return Promise.resolve(false);
  const entry = { t: Math.max(0, t) };
  if (note) entry.note = note;
  const what = `marker at ${fmt(t, 3)} s${note ? `: ${note}` : ""}`;
  return mkCommit(MK.list.concat([entry]), what, t);
}
function markerAtSelection() {   // M: the selected hit's time, else the playhead (as Set Bar 1 at Playhead)
  if (!needProject()) return Promise.resolve(false);
  const e = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  return markerAdd(e ? e.t : S.playhead);
}
function markerJump(dir) {   // Option+M and Option+Shift+M: the next or the previous marker from the playhead
  if (!MK.list.length) { setMsg(dir > 0 ? "no next marker" : "no previous marker"); return; }
  const at = S.playhead;
  const m = dir > 0 ? MK.list.find(x => x.t > at + 1e-9) : MK.list.slice().reverse().find(x => x.t < at - 1e-9);
  if (!m) { setMsg(dir > 0 ? "no next marker" : "no previous marker"); return; }
  S.playhead = Math.min(m.t, maxDur());
  centerOn(S.playhead);
  updateStatus();
  setMsg(`marker at ${fmt(m.t, 3)} s${typeof m.note === "string" && m.note ? `: ${m.note}` : ""}`);
}
