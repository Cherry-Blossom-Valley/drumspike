/* js_unit: the persist.js transitions (R17 step 2, #55 per docs/review/R17-plan.md).
   Each test gets a fresh context: state.js and persist.js in script order, a stubbed B and
   a manual clock. The four transitions the plan names for this step:
   a mark -> flush -> ack -> dirty-clear cycle, a refused write that stays queued, a late
   ack after a switch that clears nothing (the sid capture), an edit during a write that
   keeps _editSeq ahead.

   Sequencing rule: start the flush, settle the microtasks (the write reaches the stubbed
   B), resolve or reject the deferred, THEN await the flush. Awaiting the flush before its
   deferred settles would deadlock, and a bare pending promise would let node exit
   silently. Structures that live in the vm context go through plain() before deepEqual:
   a cross-realm prototype never compares equal to a host literal. */
"use strict";

const assert = require("node:assert/strict");

const plain = (x) => JSON.parse(JSON.stringify(x));

function freshProject(env, sid) {
  const S = env.get("S");
  S.sid = sid;
  S.tracks = { kick: { sr: 48000, duration: 2, n_samples: 96000 } };
  S.events = { kick: [{ t: 1, v: 90 }] };
  S.dirty = false;
  return S;
}

module.exports.tests = {
  /* the save cycle: markDirty schedules the write, the flush sends it, the ack frees the
     queue, and the clean check clears the dirty mark when the service says clean. */
  async "mark, flush, ack, then the clean check clears the dirty mark"(env) {
    env.load("state.js");
    env.load("persist.js");
    env.get("persistInit({ edIdle: () => Promise.resolve() })");
    const S = freshProject(env, "proj-1");
    env.get("markDirty")("kick");
    assert.equal(env.get("_editSeq"), 1);
    assert.equal(S.dirty, true);
    assert.deepEqual([...env.get("Object.keys(saveTimers)")], ["kick"]);

    const flushing = env.get("flushEvents")();
    await env.settle();
    assert.equal(env.calls.setEvents.length, 1);
    assert.deepEqual(plain(env.calls.setEvents[0].args), ["proj-1", "kick",
      { sr: 48000, duration: 2, n_samples: 96000, events: [{ t: 1, v: 90 }] }]);
    env.calls.setEvents[0].d.res({ followers: ["kick"] });   // the service names the followers it refreshed
    await flushing;
    assert.deepEqual(env.calls.followers, [[["kick"]]]);   // the ack refreshed the followers
    assert.deepEqual([...env.get("Object.keys(saveTimers)")], []);   // the ack freed the queue

    env.get("cleanCheckSoon")();
    env.clock.tick(450);
    await env.settle();
    assert.deepEqual(env.calls.clean, [["proj-1"]]);
    assert.equal(S.dirty, false);   // the service says clean: the mark clears
  },

  /* #249 R02: a refused write stays queued (sid and body kept), the flush rejects so the
     step that needed it does not run, the banner shows; the retry sends it again. */
  async "a refused write stays queued and the flush rejects"(env) {
    env.load("state.js");
    env.load("persist.js");
    env.get("persistInit({ edIdle: () => Promise.resolve() })");
    const S = freshProject(env, "proj-1");
    env.get("markDirty")("kick");

    const flushing = env.get("flushEvents")();
    await env.settle();
    env.calls.setEvents[0].d.rej(new Error("disk full"));
    await assert.rejects(flushing, /disk full/);
    await env.settle();
    assert.equal(env.get("Object.keys(saveTimers).length"), 1);   // it stayed queued
    const queued = env.get("saveTimers.kick");
    assert.equal(queued.sid, "proj-1");   // the captured sid survived
    assert.deepEqual(queued.body.events, S.events.kick);   // the captured body survived
    assert.equal(env.els["#stEvFail"].hidden, false);   // the banner shows
    assert.equal(env.get("_editSeq"), 1);   // a refusal is no edit

    const retry = env.get("flushEvents")();   // Retry: the write goes out again
    await env.settle();
    assert.equal(env.calls.setEvents.length, 2);
    env.calls.setEvents[1].d.res({ followers: [] });
    await retry;
    await env.settle();
    assert.deepEqual([...env.get("Object.keys(saveTimers)")], []);
  },

  /* the sid capture (#249 R03/S3-1): a write of the old project acks after the switch; it
     must refresh no followers of the new project and must not drop the new write. */
  async "a late ack after a switch clears nothing"(env) {
    env.load("state.js");
    env.load("persist.js");
    env.get("persistInit({ edIdle: () => Promise.resolve() })");
    const S = freshProject(env, "proj-1");
    env.get("markDirty")("kick");
    const flushing = env.get("flushEvents")();   // the write of proj-1 is on its way
    await env.settle();

    S.sid = "proj-2";   // the switch
    env.get("markDirty")("kick");   // an edit of the new project queues its own write
    assert.equal(env.get("_editSeq"), 2);

    env.calls.setEvents[0].d.res({ followers: [] });   // the old write acks late
    await flushing;
    await env.settle();
    assert.deepEqual(env.calls.followers, []);   // the late ack refreshed nothing
    const queued = env.get("saveTimers.kick");   // ... and dropped no queued write
    assert.equal(queued.sid, "proj-2");

    env.clock.tick(400);   // the new project's write goes out under its own sid
    await env.settle();
    assert.equal(env.calls.setEvents[1].args[0], "proj-2");
    env.calls.setEvents[1].d.res({ followers: [] });
    await env.settle();
    assert.deepEqual([...env.get("Object.keys(saveTimers)")], []);
  },

  /* #249 R03: an edit during a write keeps _editSeq ahead of the answer, so the save
     answer cannot clear the dirty mark; the first ack must not drop the second write. */
  async "an edit during a write keeps _editSeq ahead"(env) {
    env.load("state.js");
    env.load("persist.js");
    env.get("persistInit({ edIdle: () => Promise.resolve() })");
    freshProject(env, "proj-1");
    env.get("markDirty")("kick");   // _editSeq 1, write queued
    const flushing = env.get("flushEvents")();   // the write is on its way
    await env.settle();

    env.get("markDirty")("kick");   // the edit during the write: _editSeq 2, entry replaced
    assert.equal(env.get("_editSeq"), 2);

    env.calls.setEvents[0].d.res({ followers: [] });   // the first write acks
    await flushing;
    await env.settle();
    assert.equal(env.get("_editSeq"), 2);   // the ack moved no counter
    assert.equal(env.get("Object.keys(saveTimers).length"), 1);   // the second write is still queued

    env.clock.tick(400);   // the second write goes out and acks
    await env.settle();
    assert.equal(env.calls.setEvents.length, 2);
    env.calls.setEvents[1].d.res({ followers: [] });
    await env.settle();
    assert.deepEqual([...env.get("Object.keys(saveTimers)")], []);
    assert.equal(env.get("_editSeq"), 2);   // a save that captured seq 1 sees the edit: dirty stays
  },
};
