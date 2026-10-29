/* js_unit: the edit.js transitions (R17 step 3, #55 per docs/review/R17-plan.md).
   Each test gets a fresh context: state.js, persist.js and edit.js in script order, a
   stubbed B (the service) and the page-level helpers edit.js calls at run time. The three
   proofs the plan names for this step: undo/redo of add/del/move/setv against the stubbed
   service applier, a refused step that stays on its stack, and EDGEN voiding an in-flight
   drag (the counter bumps and the void hook asks the input layer).

   The service stub answers like the real one: events_edit applies, events_get returns the
   stored hits after the op; adoptHits then hands the page ids back. A refusal rejects
   events_edit, so edNow reloads from the service and answers false. */
"use strict";

const assert = require("node:assert/strict");

/* structures that live in the vm context go through plain() before deepEqual: a cross-realm
   prototype never compares equal to a host literal (see test_persist.js) */
const plain = (x) => JSON.parse(JSON.stringify(x));

function freshEditEnv(env) {
  /* the page-level helpers and module flags the moved code calls (all resolved at call
     time; the harness carries the DOM-free ones, the test adds the edit-specific ones) */
  Object.assign(env.sandbox, {
    normEvents: (raw) => (raw || []).map(e => ({ ...e })),
    newId: (() => { let n = 0; return () => `gen${++n}`; })(),
    analysisRunning: false,
    LN: { busy: 0 },
    jobNote() {},
    hgRepickBusy: () => false,
    tmUndoWaits: () => false,
    lnUndoWaits: () => false,
    hgUndoWaits: () => false,
    setSelTrack() {},
    centerOn() {},
    invalidateHits() {},
    refreshCounts() {},
    updateStatus() {},
    suspendFollowForEdit() {},
    hgSerial: (t, f) => f(),
  });
  env.load("state.js");
  env.load("persist.js");
  env.load("edit.js");
  env.get("persistInit")({ edIdle: () => Promise.resolve() });
  const S = env.get("S");
  S.sid = "proj-1";
  S.tracks = { kick: { sr: 48000, duration: 2, n_samples: 96000 } };
  S.events = { kick: [] };
  S.dirty = false;
  return S;
}

/* the service stub: events_edit records and answers, events_get hands out the canned
   stored hits (one array per call, the state after the op) */
function stubService(env, refuse) {
  const rec = { edits: [], stored: [] };
  const B = env.get("B");
  B.eventsEdit = (sid, name, ops) => {
    rec.edits.push({ sid, name, ops });
    return refuse ? Promise.reject(new Error("the service refused")) : Promise.resolve(true);
  };
  B.events = () => Promise.resolve(rec.stored.length ? rec.stored.shift() : []);
  return rec;
}

module.exports.tests = {
  /* undo of a move: the applier moves the hit back through the service, the page shows the
     stored hits under the old id, and the step lands on the redo stack; redo reverses it. */
  async "undo and redo of a move run the applier and move the step between the stacks"(env) {
    const S = freshEditEnv(env);
    const rec = stubService(env);
    env.get("editInit")({ pushEvents: () => Promise.resolve(), voidHitDrag() {} });
    S.events.kick = [{ t: 1, v: 90, id: "h1" }];
    env.get("pushUndo")({ type: "move", track: "kick", id: "h1", from: 1, to: 2 });
    assert.deepEqual([...env.get("S.undo")].length, 1);

    rec.stored.push([{ t: 1, v: 90 }]);   // the service holds the hit back at t=1
    await env.get("undoRedo")(true);
    assert.equal(S.events.kick[0].t, 1);
    assert.equal(S.events.kick[0].id, "h1");   // the page id survived the round trip
    assert.deepEqual(plain(rec.edits[0].ops), [{ op: "move", index: 0, t: 1 }]);
    assert.equal(env.get("S.undo.length"), 0);
    assert.equal(env.get("S.redo.length"), 1);

    rec.stored.push([{ t: 2, v: 90 }]);   // the redo moves it to t=2 again
    await env.get("undoRedo")(false);
    assert.equal(S.events.kick[0].t, 2);
    assert.equal(S.events.kick[0].id, "h1");
    assert.deepEqual(plain(rec.edits[1].ops), [{ op: "move", index: 0, t: 2 }]);
    assert.equal(env.get("S.undo.length"), 1);
    assert.equal(env.get("S.redo.length"), 0);
  },

  /* the other hit edits: the undo of an add deletes through the service, the redo re-adds
     under the same id; the undo of a setv restores the velocity. */
  async "undo and redo of an add and of a setv run the applier"(env) {
    const S = freshEditEnv(env);
    const rec = stubService(env);
    env.get("editInit")({ pushEvents: () => Promise.resolve(), voidHitDrag() {} });

    S.events.kick = [{ t: 3, v: 80, id: "h2" }];
    env.get("pushUndo")({ type: "add", track: "kick", id: "h2", t: 3, v: 80 });
    rec.stored.push([]);   // the service deleted the hit
    await env.get("undoRedo")(true);
    assert.deepEqual(plain(S.events.kick), []);
    assert.deepEqual(plain(rec.edits[0].ops), [{ op: "delete", index: 0 }]);
    assert.equal(env.get("S.redo.length"), 1);

    rec.stored.push([{ t: 3, v: 80 }]);   // the service re-added it
    await env.get("undoRedo")(false);
    assert.equal(S.events.kick.length, 1);
    assert.equal(S.events.kick[0].id, "h2");   // make hands the id back
    assert.deepEqual(plain(rec.edits[1].ops), [{ op: "add", t: 3, v: 80 }]);   // a re-added hit is added by default (added:false rides only with a restored delete)
    assert.equal(env.get("S.undo.length"), 1);

    env.get("pushUndo")({ type: "setv", track: "kick", id: "h2", from: 80, to: 100 });
    rec.stored.push([{ t: 3, v: 80 }]);   // the undo of the setv restores 80
    await env.get("undoRedo")(true);
    assert.equal(S.events.kick[0].v, 80);
    assert.deepEqual(plain(rec.edits[2].ops), [{ op: "velocity", index: 0, v: 80, edited: true }]);
  },

  /* P4-9 stage 2: a refused step changes nothing and stays where it was; the page shows
     what the service holds (edReload) and EDGEN bumps (the drags are void). */
  async "a refused step stays on its stack"(env) {
    const S = freshEditEnv(env);
    const rec = stubService(env, true);
    env.get("editInit")({ pushEvents: () => Promise.resolve(), voidHitDrag() {} });
    S.events.kick = [{ t: 1, v: 90, id: "h1" }];
    env.get("pushUndo")({ type: "move", track: "kick", id: "h1", from: 1, to: 2 });

    const moved = env.get("undoRedo")(true);
    await env.settle();
    assert.equal(await moved, undefined);   // no step moved
    assert.equal(env.get("S.undo.length"), 1);   // it stayed on the undo stack
    assert.equal(env.get("S.redo.length"), 0);
    assert.equal(rec.edits.length, 1);   // the applier ran and the service refused
    assert.equal(env.get("EDGEN.kick"), 1);   // the reload replaced the list
  },

  /* EDGEN invalidates an in-flight drag: every replaced list bumps the counter and asks the
     input layer to void the drag of that track — unless the plan is the drag's own commit. */
  async "EDGEN voids an in-flight drag"(env) {
    const S = freshEditEnv(env);
    const rec = stubService(env);
    const voided = [];
    env.get("editInit")({ pushEvents: () => Promise.resolve(), voidHitDrag: (name) => voided.push(name) });
    S.events.kick = [{ t: 1, v: 90, id: "h1" }];
    assert.equal(env.get("EDGEN.kick") || 0, 0);   // a drag started now would capture gen 0

    rec.stored.push([{ t: 5, v: 90 }]);   // a plain edit replaced the list
    await env.get("edCall")("kick", () => ({ ops: [{ op: "move", index: 0, t: 5 }] }));
    assert.equal(env.get("EDGEN.kick"), 1);
    assert.deepEqual(voided, ["kick"]);   // the drag of kick is void

    rec.stored.push([{ t: 6, v: 90 }]);
    await env.get("edCall")("kick", () => ({ ops: [{ op: "move", index: 0, t: 6 }], drag: true, step: { type: "move", track: "kick", id: "h1", from: 5, to: 6 } }));
    assert.equal(env.get("EDGEN.kick"), 2);
    assert.deepEqual(voided, ["kick"]);   // the drag's own commit does not void itself
  },
};
