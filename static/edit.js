/* Drum Spike v2 — edit (R17 step 3, #55 per docs/review/R17-plan.md)
   The edit/undo ownership: the undo stacks (S.undo/S.redo stay readable as accessors on S,
   exactly like PH), pushUndo/applyOp/undoStep, the SH serial queue (shSerial/edIdle), EDGEN
   and the events_edit machinery (adoptHits, edReload, edNow, edCall, edApplyOp, edDelete)
   with hitWas/delKeep. Plain script (no build step), loaded after persist.js. The functions
   stay top-level bindings: the e2e tests call undo/redo and read the stacks as page globals.
   edit -> persist is circular (an edit flushes the writes of other steps first), so
   pushEvents arrives as an injected hook: boot wires editInit({ pushEvents, voidHitDrag });
   voidHitDrag asks the input layer to void an in-flight hit drag when the stored hits
   replaced the page's list. markDirty has no caller here today (an edit writes through the
   service and refreshes from it; the 400 ms write stays with the steps that edit S.events
   in place). */
"use strict";

/* ---------------- the injected hooks ----------------
   Boot wires them (app.js); a js_unit test wires its own (tests/js_unit/). Nothing here
   reads the DOM or the backend at load time. */
const EDIT_HOOKS = {};
function editInit(deps) { if (deps) Object.assign(EDIT_HOOKS, deps); }
function editPushEvents(track) { return typeof EDIT_HOOKS.pushEvents === "function" ? EDIT_HOOKS.pushEvents(track) : Promise.resolve(); }
function editVoidHitDrag(name) { if (typeof EDIT_HOOKS.voidHitDrag === "function") EDIT_HOOKS.voidHitDrag(name); }

/* ---------------- the undo stacks ---------------- */
/* The stacks live here since step 3; S.undo and S.redo stay the way everyone reads and
   writes them (accessors on S, exactly like PH). The setter replaces the storage, so a
   captured array keeps its steps (trReload's keepStacks). */
let _undo = [], _redo = [];
Object.defineProperty(S, "undo", { enumerable: true,
  get() { return _undo; }, set(a) { _undo = Array.isArray(a) ? a : []; } });
Object.defineProperty(S, "redo", { enumerable: true,
  get() { return _redo; }, set(a) { _redo = Array.isArray(a) ? a : []; } });

/* ---------------- editing ---------------- */
/* P4-40a: every step has a `label` ("Delete Hit") and a `where` {track, t}: a hit edit, a Learn change and so on say what
   they did and where. The step gets both when it is pushed (a site may set them first: nudge, Learn). A move shows its
   `from` when it is undone (opWhere). Edit > Undo and Redo name the step (undoTitle), an undo reveals its place. A
   selection or a view change is no step: no pushUndo call exists for them (a test guards it). */
function undoMeta(op) {
  const hitT = (trk, id) => { const e = (S.events[trk] || []).find(x => x.id === id); return e ? e.t : null; };
  const at = (track, t) => ({ track: track || null, t: Number.isFinite(t) ? t : null });
  const set = (label, where) => { if (!op.label) op.label = label; if (!op.where) op.where = where; };
  switch (op.type) {
    case "del": set("Delete Hit", at(op.track, op.t)); break;
    case "toggle": set("Toggle Hit Active", at(op.track, op.t)); break;   // #242
    case "add": set("Add Hit", at(op.track, op.t)); break;
    case "move": set("Move Hit", at(op.track, op.to)); break;
    case "setv": set("Set Velocity", at(op.track, hitT(op.track, op.id))); break;
    case "setv-many": {   // P4-40f: one velocity gesture over several hits, one step
      const n = (op.list || []).length;
      set(`Set Velocity (${op.track}, ${n} hits)`, at(op.track, n ? hitT(op.track, (op.list)[0].id) : null));
      break;
    }
    case "del-many": {   // #151: one Delete over a range selection
      const n = (op.list || []).length;
      set(`Delete Hits (${op.track}, ${n} hits)`, at(op.track, n ? (op.list)[0].t : null));
      break;
    }
    case "move-many": {   // #151: one nudge over a range selection
      const n = (op.list || []).length;
      set(`Nudge Hits (${op.track}, ${n} hits)`, at(op.track, n ? (op.list)[0].from : null));
      break;
    }
    case "shift": set("Shift Hits", at(op.track, null)); break;
    case "repick": set("Re-pick Hits", at(op.track, null)); break;
    case "reanalyze": set(`Re-analyze Selection (${op.track}, ${fmt((op.t0), 3)} to ${fmt((op.t1), 3)} s)`, at(op.track, op.t0)); break;   // #265
    case "loud": set("Set Loudness Range", at(op.track, null)); break;
    case "learn": set("Learn Change", at(op.track, null)); break;
    case "trkdel": set(`Remove Track ${op.name}`, at(op.name, null)); break;
    case "role": set(`Set Role (${op.track})`, at(op.track, null)); break;
    case "tempo": set(/^bar 1/.test(op.what || "") ? "Set Bar 1" : op.what ? op.what.charAt(0).toUpperCase() + op.what.slice(1) : "Tempo Map Change", at(null, null)); break;
    case "vel": {
      const name = Object.keys(op.tracks || {})[0], id = name ? Object.keys(op.tracks[name].after)[0] : null;
      set("Recompute Velocities", at(name, id === undefined ? null : hitT(name, id)));
      break;
    }
    default: set("Edit", at(null, null));
  }
  return op;
}
function opWhere(op, isUndo) { return op.type === "move" && isUndo ? { track: op.track, t: op.from } : (op.where || { track: null, t: null }); }
function stepText(op, isUndo) {   // "Delete Hit (s top, 12.345 s)"
  const w = opWhere(op, isUndo), parts = [];
  if (w.track) parts.push(w.track);
  if (w.t !== null && w.t !== undefined) parts.push(`${w.t.toFixed(3)} s`);
  return (op.label || "Edit") + (parts.length ? ` (${parts.join(", ")})` : "");
}
function undoTitles() {   // the texts of Edit > Undo and Edit > Redo: the step each one takes, or the plain word
  const u = S.undo[S.undo.length - 1], r = S.redo[S.redo.length - 1];
  return { undoTitle: u ? `Undo ${stepText(u, true)}` : "Undo", redoTitle: r ? `Redo ${stepText(r, false)}` : "Redo" };
}
function undoShell() { if (window.DSShell && DSShell.sync) DSShell.sync(); }
function pushUndo(op) { undoMeta(op); S.undo.push(op); if (S.undo.length>300) S.undo.shift(); S.redo.length = 0; undoShell(); }
function undoReveal(op, isUndo) {   // after an undo or redo: the track is selected, the hit is selected, the view shows the place
  const w = opWhere(op, isUndo);
  if (!w.track || !S.tracks[w.track]) return;
  if (S.selTrack !== w.track) setSelTrack(w.track);
  if (w.t === null || w.t === undefined) return;
  const evs = S.events[w.track] || [];
  let j = op.id ? evs.findIndex(e => e.id === op.id) : -1;
  if (j < 0) { let best = 0.005; evs.forEach((e, i) => { if (Math.abs(e.t - w.t) < best) { best = Math.abs(e.t - w.t); j = i; } }); }
  if (j >= 0) S.sel = { track: w.track, index: j };
  if (w.t < S.view.t0 || w.t > S.view.t1) centerOn(w.t);
  invalidateHits(w.track); updateStatus();
}
function applyOp(op, reverse) {   // a step of the service (shift, add, del, move, setv) answers a promise: true when it was applied
  if (op.type === "vel") { DSShell.applyVel(op, reverse); updateStatus(); return; }   // P3-4b: Recompute Velocities, all tracks in one step
  if (op.type === "learn") { lnApplyOp(op, reverse); return; }   // P4-6a: a Learn mark, removal, flip or Forget
  if (op.type === "tempo") { tmApplyOp(op, reverse); return; }   // P4-8: a change of the tempo and meter map
  if (op.type === "loud") { hgApplyLoudOp(op, reverse); return; }   // P4-3: the loudness histogram (floor, ceiling, cut)
  if (op.type === "trkdel") return trApplyOp(op, reverse);   // P4-39a: Track > Remove Track
  if (op.type === "role") return roleApplyOp(op, reverse);   // 0.9.1: the role of a track (Inspector)
  if (op.type === "shift") return shiftApplyOp(op, reverse);   // P4-9: the Hitpoint shift
  if (op.type === "params") { hgApplyParamsOp(op, reverse); return; }   // #232: a stored detection setting (the refine window)
  if (op.type === "repick") { hgApplyRepickOp(op, reverse); return; }   // P4-3: a stored re-pick of the Sensitivity histogram
  if (op.type === "reanalyze") { raApplyOp(op, reverse); return; }   // #265: a Re-analyze Selection (the hits of the range)
  return edApplyOp(op, reverse);   // P4-9 stage 2: add, del, move, setv: events_edit
}
/* P4-9 stage 2 (fix round items b and c): a step moves to the other stack after the service applied it, so a refused
   undo or redo leaves the stacks as the service is. While a shift or an edit is on its way, an undo or a redo joins the
   same queue and runs after it (it takes the step that is on top then); the other waits (tm, ln, hg) still drop it with a
   message, because their own busy flags have no hook that could run it later. */
function edOwns(op) { return !!op && (op.type === "trkdel" || op.type === "shift" || op.type === "add" || op.type === "del" || op.type === "move" || op.type === "setv" || op.type === "setv-many" || op.type === "del-many" || op.type === "move-many" || op.type === "toggle"); }
async function undoStep(isUndo) {
  const from = isUndo ? S.undo : S.redo, to = isUndo ? S.redo : S.undo, op = from[from.length - 1];
  if (tmUndoWaits() || lnUndoWaits() || hgUndoWaits(op) || !op) return;
  const r = applyOp(op, isUndo);
  if (r && typeof r.then === "function" && !(await r)) return;   // refused: the step stays where it was
  const i = from.lastIndexOf(op);
  if (i < 0) return;   // a new edit cleared the redo stack meanwhile
  from.splice(i, 1); to.push(op);
  undoReveal(op, isUndo);
  setMsg(`${isUndo ? "undid" : "redid"} ${stepText(op, isUndo)}`);
  undoShell(); cleanCheckSoon();
  return true;
}
function undoRedo(isUndo) {   // answers a promise: true when a step moved
  const stack = isUndo ? S.undo : S.redo;
  if (SH.pending || edOwns(stack[stack.length - 1])) return shSerial(() => undoStep(isUndo));
  return undoStep(isUndo).catch(e => { setMsg(errText(e)); return false; });
}
/* P4-40a point 4: Edit > Undo History. The steps newest first: the undone ones (grey) above, then the done ones. A click takes
   the steps back (a done one) or forward (an undone one) up to and with that row, one after the other. */
function undoHistoryRows() {
  const rows = [];
  S.redo.forEach((op, j) => rows.push({ op, redo: true, count: S.redo.length - j }));
  for (let k = S.undo.length - 1; k >= 0; k--) rows.push({ op: S.undo[k], redo: false, count: S.undo.length - k });
  return rows.slice(0, 40);
}
async function undoSteps(isUndo, count) {
  let n = 0;
  for (let tries = 0; n < count && tries < 40; tries++) {
    const stack = isUndo ? S.undo : S.redo, before = stack.length;
    if (!before) break;
    await undoRedo(isUndo);
    if (stack.length < before) { n++; tries = 0; } else await new Promise(r => setTimeout(r, 100));   // refused while a write is on its way: wait
  }
  return n;
}
function undo() { undoRedo(true); }
function redo() { undoRedo(false); }

function settleEvents(name) { return hasPendingWrite(name) ? editPushEvents(name) : Promise.resolve(); }   // the service reads the stored hits (pushEvents arrives as the injected hook)
/* P4-9 stage 2: the service calls that change the hits (a shift, a hit edit, the undo and redo of both) go through one queue
   (shSerial): one after the other, in the order the user made them; SH.pending counts the calls queued or running (a re-pick,
   a save and an export wait for it). Each call reads what it needs when it runs, not when it was asked. */
const SH = { pending: 0, chain: Promise.resolve() };
function shSerial(call) {
  SH.pending++;
  const next = SH.chain.then(call).catch(e => { setMsg(errText(e)); return false; }).finally(() => { SH.pending--; });
  SH.chain = next;
  return next;
}
async function edIdle() { while (SH.pending) await SH.chain; }   // outside the queue only (a call in the queue would wait for itself)

/* P4-9 stage 2: the hit edits (add, move, delete, velocity, and the undo and redo of them) are events_edit calls. The page asks
   (edCall with a plan made when the call runs: the ops, the ids it needs for the hits, the undo step), the service changes
   the stored hits, the page shows them (events_get) and pushes the undo step after that. A refused call changes nothing and
   makes no step. The 400 ms write (markDirty) stays for the other steps (velocity map, re-pick undo, Learn); an edit sends
   those writes first (settleEvents), a save, an export and a re-pick wait for the edits (flushEvents, edIdle).
   A hit drag shows the hit at the pointer on the page's copy and commits one move on release. */
const EDGEN = {};   // track -> counter: the page's list of a track was replaced by the stored hits (a drag in progress is void then)
function adoptHits(raw, old, drop, make) {   // the stored hits with page ids: the hits the call did not touch keep theirs (undo steps)
  const gone = new Set(drop || []), pool = new Map();
  for (const e of old) if (!gone.has(e.id)) { const a = pool.get(e.t); if (a) a.push(e.id); else pool.set(e.t, [e.id]); }
  const fresh = normEvents(raw), left = [];
  fresh.forEach(e => { const a = pool.get(e.t); if (a && a.length) e.id = a.shift(); else left.push(e); });
  const want = (make || []).slice();
  for (const e of left) {
    const k = want.findIndex(([t]) => Math.abs(t - e.t) < 1e-6);
    e.id = k >= 0 ? want.splice(k, 1)[0][1] : newId();
  }
  return fresh;
}
async function edReload(sid, name) {   // after a refused call: the page shows what the service holds
  try {
    if (sid !== S.sid || !S.tracks[name]) return;
    S.events[name] = adoptHits(await B.events(sid, name), S.events[name] || [], [], []);
    EDGEN[name] = (EDGEN[name] || 0) + 1;
    invalidateHits(name); refreshCounts(name); updateStatus();
  } catch (e) { /* the next load shows them */ }
}
async function edNow(name, mk, before) {   // true when the service applied the plan mk() made; mk(evs) -> null (nothing to do) or {ops, drop, make, sel, step, msg}
  const sid = S.sid;
  if (!sid || !S.tracks[name]) return false;
  if (analysisRunning) { jobNote("hit edits are refused while an analysis runs"); return false; }
  if (hgRepickBusy()) { setMsg("a re-pick is being stored: try the edit again"); return false; }
  if (LN.busy > 0) { setMsg("a Learn change is being applied: try the edit again"); return false; }   // P4-6c: its answer replaces the hit list; a Learn call that still waits for the edits on their way does not refuse them (they run first)
  if (hasPendingWrite(name)) { if (before) before(); try { await settleEvents(name); } catch (e) { setMsg(`edit: ${errText(e)}`, "error"); return false; } }   // the 400 ms writes of other steps reach the service first; #249 R02: a refused one refuses the edit
  if (sid !== S.sid || !S.tracks[name]) return false;
  const pl = mk(S.events[name] || []);
  if (!pl) return false;
  let r;
  try { r = await B.eventsEdit(sid, name, pl.ops); }
  catch (e) { setMsg(`edit: ${errText(e)}`); await edReload(sid, name); return false; }
  if (sid !== S.sid || !S.tracks[name]) return false;
  if (pl.done) pl.done(r);   // #144: the plan reads the answer (where a snapped hit landed)
  const old = S.events[name] || [], cur = S.sel && S.sel.track === name ? (old[S.sel.index] || {}).id : undefined;
  let fresh;
  try { fresh = adoptHits(await B.events(sid, name), old, pl.drop, pl.make); }
  catch (e) { setMsg(`edit: ${errText(e)}`); return false; }
  if (sid !== S.sid || !S.tracks[name]) return false;
  S.events[name] = fresh; EDGEN[name] = (EDGEN[name] || 0) + 1;
  const want = pl.sel !== undefined ? pl.sel : cur;
  if (want !== undefined) { const j = want ? fresh.findIndex(x => x.id === want) : -1; S.sel = j >= 0 ? { track: name, index: j } : null; }
  if (pl.step) pushUndo(pl.step);
  if (!pl.drag) editVoidHitDrag(name);   // the list was replaced: a drag in progress is void (the hook asks the input layer)
  setDirtyState(true); invalidateHits(name); refreshCounts(name); updateStatus();
  if (pl.ops && pl.ops.some(op => op.op === "add" || op.op === "delete" || op.op === "move")) suspendFollowForEdit();   // P4-wf10
  if (pl.msg) setMsg(pl.msg);
  reloadFollowers(Array.isArray(r && r.followers) ? r.followers : followersOf(name));
  return true;
}
function edCall(name, mk, before) { return shSerial(() => edNow(name, mk, before)); }
const edIndex = (evs, id) => evs.findIndex(e => e.id === id);
function edApplyOp(op, reverse) {   // applyOp: the ops of a step, forward or back (runs in the queue)
  if (op.type === "setv-many") {   // P4-40f: one velocity gesture over several hits
    return edNow(op.track, evs => {
      const ops = [];
      for (const s of op.list || []) {
        const j = edIndex(evs, s.id);
        if (j < 0) { setMsg(`${reverse ? "undo" : "redo"}: the hit is not there any more`); return null; }
        ops.push({ op: "velocity", index: j, v: reverse ? s.from : s.to, edited: true, ...(reverse && s.was ? { was: s.was } : {}) });   // R05: the undo restores the hit's field presence
      }
      return { ops };
    });
  }
  if (op.type === "del-many") {   // #151: one Delete over a range selection: the undo puts every hit back as it was stored (byte-identical), the redo deletes them again
    return edNow(op.track, evs => {
      if (!reverse) {
        const ops = [];
        for (const s of op.list || []) {
          const j = edIndex(evs, s.id);
          if (j < 0) { setMsg("redo: the hit is not there any more"); return null; }
          ops.push({ op: "delete", index: j });
        }
        return { ops, drop: (op.list || []).map(s => s.id) };
      }
      return { ops: (op.list || []).map(s => ({ op: "add", t: s.t, v: s.v, ...(s.added ? {} : { added: false }), ...(s.orig !== undefined ? { orig: s.orig } : {}), ...(s.keep ? { keep: s.keep } : {}) })),
        make: (op.list || []).map(s => [s.t, s.id]) };
    });
  }
  if (op.type === "move-many") {   // #151: one nudge over a range selection
    return edNow(op.track, evs => {
      const ops = [];
      for (const s of op.list || []) {
        const j = edIndex(evs, s.id);
        if (j < 0) { setMsg(`${reverse ? "undo" : "redo"}: the hit is not there any more`); return null; }
        ops.push({ op: "move", index: j, t: reverse ? s.from : s.to, ...(reverse && s.was ? { was: s.was } : {}) });   // R05
      }
      return { ops, drop: (op.list || []).map(s => s.id), make: (op.list || []).map(s => [reverse ? s.from : s.to, s.id]) };
    });
  }
  if (op.type === "toggle") {   // #242: Toggle Hit Active: forward to the step's `to`, back to the flags `was` holds
    return edNow(op.track, evs => {
      const j = edIndex(evs, op.id);
      if (j < 0) { setMsg(`${reverse ? "undo" : "redo"}: the hit is not there any more`); return null; }
      if (reverse) return { ops: [{ op: "toggle", index: j, keep: op.was.keep, deactivated: op.was.deactivated, was: op.was }], sel: op.id };
      return { ops: [{ op: "toggle", index: j, keep: op.to.keep, deactivated: op.to.deactivated }], sel: op.id };
    });
  }
  const id = op.id, label = reverse ? "undo" : "redo";
  const mk = evs => {
    const j = edIndex(evs, id);
    if (op.type === "add" && !reverse || op.type === "del" && reverse) {   // the hit comes back and is selected again
      const e = op.type === "add" ? { t: op.t, v: op.v } : { t: op.t, v: op.v, ...(op.added ? {} : { added: false }), ...(op.orig !== undefined ? { orig: op.orig } : {}), ...(op.keep ? { keep: op.keep } : {}) };
      return { ops: [{ op: "add", ...e }], make: [[op.t, id]], sel: id };
    }
    if (j < 0) { setMsg(`${label}: the hit is not there any more`); return null; }
    if (op.type === "add" || op.type === "del") return { ops: [{ op: "delete", index: j }], drop: [id] };
    if (op.type === "move") { const to = reverse ? op.from : op.to; return { ops: [{ op: "move", index: j, t: to, ...(reverse && op.was ? { was: op.was } : {}) }], drop: [id], make: [[to, id]] }; }
    return { ops: [{ op: "velocity", index: j, v: reverse ? op.from : op.to, edited: true, ...(reverse && op.was ? { was: op.was } : {}) }] };   // setv; R05: the undo restores the field presence, the redo marks the hit edited again (like the setv-many list branch)
  };
  return edNow(op.track, mk);
}
/* R05: the field presence of a hit before an edit (the page's copy of the reserved flags; `orig` and
   `rise` ride along as values). The undo sends it as `was` on the inverse op, so the service puts the
   hit back byte-identical (`undo(edit(doc)) == doc`): an edited flag a hand velocity or a restore
   added goes away again, a moved hit's `orig` comes back (or goes), a stripped cut, bleed or
   filtered mark returns. Every field the forward op can add or take off is in the map, also when
   the hit did not have it. */
function hitWas(e) {
  return { orig: e.orig === undefined ? null : e.orig, edited: !!e.edited, cut: !!e.cut, bleed: !!e.bleed, filtered: !!e.filtered,
    learned: !!e.learned, roll: !!e.roll, flam: !!e.flam, rescue: !!e.rescue, keep: !!e.keep, deactivated: !!e.deactivated, rise: e.rise !== undefined };
}
function delKeep(raw) {   // P4-40a: the stored fields of a deleted hit besides t, v, orig and added (the page's own list drops some, e.g. rise_db); the undo gives them back. #154: the stored `id` too, so the undo of a delete restores the hit byte-identically
  const out = {};
  for (const [k, v] of Object.entries(raw || {})) if (!["t", "v", "orig", "added"].includes(k)) out[k] = v;
  return Object.keys(out).length ? out : undefined;
}
function edDelete(trk, id) {   // delete the hit with this id (the key, the right-click)
  // the read of the stored hits is inside the queue: SH.pending counts the delete from the moment it is asked (edits_done waits for it)
  return shSerial(async () => {
    let stored = [];   // the service's hits: the step keeps the fields of the hit as they are stored
    try {
      if (S.sid && (S.events[trk] || []).some(x => x.id === id)) { if (hasPendingWrite(trk)) await settleEvents(trk); const r = await B.events(S.sid, trk); stored = Array.isArray(r) ? r : (r && r.events) || []; }
    } catch (e) { stored = []; }
    return edNow(trk, evs => {
      const j = edIndex(evs, id);
      if (j < 0) return null;
      const e = evs[j], raw = stored.find(x => Math.abs(x.t - e.t) < 1e-9);
      return { ops: [{ op: "delete", index: j }], drop: [id], sel: null,
        step: { type: "del", track: trk, id, t: e.t, v: e.v, added: !!e.added, ...(e.orig !== undefined ? { orig: e.orig } : {}), keep: delKeep(raw) } };
    });
  });
}
