/* Drum Spike v2 — persist (R17 step 2, #55 per docs/review/R17-plan.md)
   The debounced event writes and the dirty tracking: saveTimers, _editSeq, eventsBody,
   pushEvents, markDirty, flushEvents, cancelEvents, the refused-write banner (evFailSet,
   evFailShow, #249 R02) and the clean check (P4-40a). Plain script (no build step), loaded
   after state.js. The functions stay top-level bindings: the e2e tests call markDirty and
   flushEvents and read saveTimers as page globals.
   persist <-> edit is circular (flushEvents waits for edIdle, edit.js calls pushEvents),
   so edIdle arrives as an injected hook: app.js wires it in boot with persistInit({ edIdle });
   edit.js owns edIdle since step 3. */
"use strict";

/* ---------------- the injected hooks ----------------
   Boot wires them (app.js); a js_unit test wires its own (tests/js_unit/). Nothing here
   reads the DOM or the backend at load time. */
const PERSIST_HOOKS = {};
function persistInit(deps) { if (deps) Object.assign(PERSIST_HOOKS, deps); }
function edIdleHook() { return typeof PERSIST_HOOKS.edIdle === "function" ? PERSIST_HOOKS.edIdle() : Promise.resolve(); }

/* ---------------- the clean check ---------------- */
/* P4-40a point 2: a dirty project whose document is the saved one again is clean again. The service compares the documents
   (project_clean_check), so a step the stacks do not know about cannot fool it. It runs after the writes on their way. */
let cleanTimer = null;
function cleanCheckSoon() {
  const sid = S.sid;
  clearTimeout(cleanTimer);
  if (!sid || !B.projectCleanCheck) return;
  cleanTimer = setTimeout(async () => {
    try {
      await edIdleHook(); await Promise.all(Object.keys(saveTimers).map(pushEvents));
      if (sid !== S.sid || !S.dirty) return;
      const r = await B.projectCleanCheck(sid);
      if (sid === S.sid && r && r.dirty === false) setDirtyState(false);
    } catch (e) { /* the flag stays as it is */ }
  }, 450);
}

/* ---------------- the debounced event writes ---------------- */
/* debounced event writes (400 ms). Each pending write captures the project id and the events array
   when it is scheduled (S3-1), so a switch, open or close in the meantime cannot send one project's
   hits to another: the write still goes to its own sid and track. */
let saveTimers = {};   // track -> {sid, body, timer}
let _editSeq = 0;   // #249 R03: counts the page's edits (markDirty); a save answer applies only when no edit slipped in during the write
function eventsBody(track) {
  const t = S.tracks[track];
  for (const e of S.events[track] || []) { if (isManual(e)) delete e.cut; if (isManual(e) || e.edited) { delete e.bleed; delete e.filtered; } }   // P4-3, P4-14, P4-6c: an edited hit is never cut, an edited or restored one never bleed nor filtered
  return { sr: t.sr, duration: t.duration, n_samples: t.n_samples, events: S.events[track] };
}
function pushEvents(track) {   // the server holds the events in the open project (dirty until Save)
  const p = saveTimers[track];
  if (p) clearTimeout(p.timer);
  const sid = p ? p.sid : S.sid, body = p ? p.body : eventsBody(track);
  return B.setEvents(sid, track, body)
    .then(r => {   // #249 R02: the payload leaves the queue only when the service acknowledged the write
      if (saveTimers[track] === p) delete saveTimers[track];
      if (sid !== S.sid) return;
      return reloadFollowers(r && Array.isArray(r.followers) ? r.followers : followersOf(track))   // P3-18
        .catch(e => setMsgBg(`followers: ${errText(e)}`, "error"));   // the write is in: a failed follower refresh must not refuse the flush
    })
    .catch(e => {   // #249 R02: a refused write stays queued for Retry; the flush rejects, so the step that needed it does not run
      if (saveTimers[track] !== p) saveTimers[track] = { sid, body, timer: null };
      return evFailShow(e);
    });
}
/* #249 R02: the status line holds the refused writes with Retry and Discard until the user acts */
function evFailSet(on) {
  const el = $("#stEvFail");
  if (el.classList.contains("hidden") === on) el.classList.toggle("hidden", !on);
}
function evFailShow(e) {
  evFailSet(true);
  setMsgBg(`the edited hits did not reach the project: ${errText(e)}`, "error");
  throw e;   // the flush and the step that awaited it refuse to go on
}
function markDirty(track) {
  if (!track) return;
  _editSeq++;   // #249 R03: an edit while a save is on its way keeps the page dirty, whatever the answer carries
  setDirtyState(true);
  if (saveTimers[track]) clearTimeout(saveTimers[track].timer);
  saveTimers[track] = { sid: S.sid, body: eventsBody(track), timer: setTimeout(() => pushEvents(track).catch(() => {}), 400) };   // #249 R02: a refusal shows on the status line, it is not an unhandled rejection
}
async function flushEvents() {   /* before S.sid changes, a save, an export, an analysis: the hit edits on their way first, then the page's own writes. #249 R02: rejects while a write is refused — the caller must not run its step */
  await edIdleHook();
  await Promise.all(Object.keys(saveTimers).map(pushEvents));
}
function cancelEvents(sid) {   // a discarded project: its pending writes are dropped, nothing else is touched
  for (const [t, p] of Object.entries(saveTimers)) if (!sid || p.sid === sid) { clearTimeout(p.timer); delete saveTimers[t]; }
}

/* ---------------- the queue's interfaces (R17 step 7) ----------------
   The state of this module is saveTimers and _editSeq; everything else reads them through
   these (the plan's interface rule: no module reaches into another's state). */
function hasPendingWrite(track) { return !!saveTimers[track]; }   // is a write of this track queued?
function pendingWriteTracks() { return Object.keys(saveTimers); }   // the tracks with a queued or in-flight write
function dropQueuedWrite(track) { delete saveTimers[track]; }   // a removed track's entry goes with it
function editSeq() { return _editSeq; }   // the counter the save answers compare (#249 R03)
