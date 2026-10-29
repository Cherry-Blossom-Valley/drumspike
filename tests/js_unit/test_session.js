/* js_unit: the session.js late-answer matrix (R17 step 6, #55 per docs/review/R17-plan.md).
   Each test gets a fresh context: state.js, persist.js and session.js in script order, the
   plan's five hooks wired to test doubles (GLQ, lnReset, auReset, tmLoad, selMem), a stubbed
   B with deferred answers and a manual clock. The four transitions the plan names for this
   step: an older loadProject finishing after a newer one writes nothing, a switch mid-load
   that leaves no events behind, a close that cancels the pending writes, and the
   switch-during-save case from step 2 replayed at session level (switchTo flushes the old
   project's write, to its own sid, before S.sid moves).

   loadProject runs for real here: the DOM/refresh layer (buildTrackDOM, invalidate,
   updateStatus, ...) is stubbed, the B answers are deferred, everything between them is the
   module under test. */
"use strict";

const assert = require("node:assert/strict");

const plain = (x) => JSON.parse(JSON.stringify(x));

const A_TRACKS = { kick: { sr: 48000, duration: 2, n_samples: 96000 } };
const A_EVENTS = [{ t: 1, v: 70 }];
const B_EVENTS = [{ t: 2, v: 80 }];

function freshSessionEnv(env) {
  env.load("state.js");
  env.load("persist.js");
  env.load("session.js");
  const H = { lnReset: 0, auReset: 0, tmLoad: 0, selMem: { sid: null, order: [] } };
  env.get("sessionInit")({
    GLQ: { loadedSeq: 0, eng: null },
    lnReset: () => { H.lnReset++; },
    auReset: () => { H.auReset++; },
    tmLoad: () => { H.tmLoad++; return Promise.resolve(); },
    selMem: () => H.selMem,
    setSelMem: (v) => { H.selMem = v; },
  });
  Object.assign(env.sandbox, {
    pause() {}, staleSync() {}, setProjName() {}, buildTrackDOM() {},
    invalidate() {}, F: { VIEW: 1, HEAD: 2, SIZE: 4 }, updateStatus() {},
    refreshCounts() {}, refreshRelinkBar() {}, normEvents: (d) => d,
    peakAmpOf: () => 0, maxDur: () => 10, flushFrame() {}, pickSelTrack: (o, s) => s,
    glEngine: () => null, showProgress() {}, hideProgress() {},
    askConfirm: async () => "discard",   // close: Discard; switch: falls through to "continue without saving"
    CP: { anim: false }, SP: { on: false }, cpPos: () => 0,   // the PH accessor's compositor side (render.js)
  });
  env.sandbox.document.createElement = () =>
    ({ style: {}, classList: { add() {}, remove() {}, contains: () => false }, appendChild() {}, append() {} });
  for (const sel of ["#esRecentList", "#esRecentClear"]) {
    env.els[sel] = env.els[sel] || { hidden: true, innerHTML: "", appendChild() {}, style: {},
      classList: { contains: () => true, toggle() {}, add() {}, remove() {} } };
  }
  Object.assign(env.get("B"), {
    sessions: () => Promise.resolve([]),
    recentProjects: () => Promise.resolve([]),
    peaks: () => Promise.resolve({}),
  });
  return H;
}

function projectS(env, sid) {
  const S = env.get("S");
  S.sid = sid;
  S.playing = false;
  S.tracks = JSON.parse(JSON.stringify(A_TRACKS));
  S.events = { kick: JSON.parse(JSON.stringify(A_EVENTS)) };
  S.dirty = false;
  return S;
}

module.exports.tests = {
  /* C5: the superseded load wakes after the newer one finished and must write nothing:
     it returns at its first checkpoint, so S keeps the newer project's state and no
     events read of the old project ever happens. */
  async "an older loadProject finishing after a newer one writes nothing"(env) {
    freshSessionEnv(env);
    const S = projectS(env, "proj-a");
    const dOld = env.defer();
    const trackCalls = [], evCalls = [];
    env.get("B").tracks = (sid) => {
      trackCalls.push(sid);
      return sid === "proj-a" ? dOld.promise : Promise.resolve({ tracks: A_TRACKS, order: ["kick"], name: "proj-b" });
    };
    env.get("B").events = (sid, n) => {
      evCalls.push([sid, n]);
      if (sid === "proj-a") throw new Error("the superseded load must not read events");
      return Promise.resolve(B_EVENTS);
    };

    const pOld = env.get("loadProject")();   // gen 1, stalls on B.tracks("proj-a")
    await env.settle();
    S.sid = "proj-b";
    const pNew = env.get("loadProject")();   // gen 2, runs to completion
    assert.equal(await pNew, true);
    assert.equal(S.loaded, 2);
    assert.deepEqual(plain(S.events.kick), B_EVENTS);

    dOld.res({ tracks: A_TRACKS, order: ["kick"], name: "proj-a" });   // the old answer arrives last
    assert.equal(await pOld, undefined);   // it returned at its first checkpoint
    assert.deepEqual(trackCalls, ["proj-a", "proj-b"]);   // each load read its own sid once
    assert.deepEqual(evCalls, [["proj-b", "kick"]]);   // the old load never got as far as the events
    assert.deepEqual(plain(S.events.kick), B_EVENTS);   // the newer project's state stands
    assert.deepEqual(plain(S.tracks.kick), A_TRACKS.kick);
  },

  /* a switch mid-load: the old load's in-flight events write still carries the old sid
     (S3-1 capture), the old load then bails at its gen check, and the newer load's reset
     and own reads leave none of the old project's events behind. */
  async "a switch mid-load leaves no events behind"(env) {
    freshSessionEnv(env);
    projectS(env, "proj-a");
    const dTracks = env.defer(), dEvA = env.defer(), dTracksB = env.defer();
    const evCalls = [];
    env.get("B").tracks = (sid) => (sid === "proj-a" ? dTracks.promise : dTracksB.promise);
    env.get("B").events = (sid, n) => { evCalls.push([sid, n]); return sid === "proj-a" ? dEvA.promise : Promise.resolve(B_EVENTS); };

    const pOld = env.get("loadProject")();   // gen 1
    await env.settle();
    dTracks.res({ tracks: A_TRACKS, order: ["kick"], name: "proj-a" });
    await env.settle();
    assert.deepEqual(evCalls, [["proj-a", "kick"]]);   // the old load is in its events batch

    env.get("S").sid = "proj-b";
    const pNew = env.get("loadProject")();   // gen 2, waits on its own tracks answer
    await env.settle();

    dEvA.res(A_EVENTS);   // the old write lands now: it writes the old project's events...
    await env.settle();
    assert.equal(await pOld, undefined);   // ...then the old load bails at its gen check

    dTracksB.res({ tracks: A_TRACKS, order: ["kick"], name: "proj-b" });
    assert.equal(await pNew, true);
    assert.deepEqual(evCalls, [["proj-a", "kick"], ["proj-b", "kick"]]);   // the old write went to the old sid
    assert.deepEqual(plain(env.get("S").events.kick), B_EVENTS);   // and none of it is left behind
    assert.equal(env.get("S").loaded, 2);
  },

  /* a close drops the project's pending writes (cancelEvents) before and after the
     close goes out: the queued write is never sent, the service sees a forced close. */
  async "a close cancels pending writes"(env) {
    const H = freshSessionEnv(env);
    env.get("persistInit")({ edIdle: () => Promise.resolve() });
    projectS(env, "proj-a");
    env.get("markDirty")("kick");   // a queued write of proj-a (400 ms timer, not fired)
    assert.equal(env.get("Object.keys(saveTimers).length"), 1);

    const closeCalls = [];
    env.get("B").closeProject = (...a) => { closeCalls.push(a); return Promise.resolve("ok"); };
    const p = env.get("closeProject")("proj-a", "proj-a");
    assert.equal(await p, true);

    assert.deepEqual(closeCalls, [["proj-a", true]]);   // the Discard path closes with force
    assert.deepEqual([...env.get("Object.keys(saveTimers)")], []);   // the pending write is gone
    assert.deepEqual(env.calls.setEvents, []);   // and it never went out
    assert.equal(env.get("S").sid, null);   // the closed project's lanes are gone
    assert.equal(H.lnReset >= 1 && H.auReset >= 1, true);   // the empty load reset the owners' state
  },

  /* the switch-during-save case from step 2, replayed at session level: switchTo
     flushes the old project's queued write BEFORE S.sid moves, the write carries the
     sid it was scheduled under, and the switch then loads the new project. */
  async "a switch flushes the old project's write, to its own sid, before it moves"(env) {
    freshSessionEnv(env);
    env.get("persistInit")({ edIdle: () => Promise.resolve() });
    projectS(env, "proj-a");
    env.get("markDirty")("kick");

    const writes = [];
    env.get("B").setEvents = (sid, track, body) => {
      const entry = { args: [sid, track, body], d: env.defer(), sidAtCall: env.get("S").sid };
      writes.push(entry);
      return entry.d.promise;
    };
    env.get("B").tracks = (sid) => {
      if (sid !== "proj-b") throw new Error(`the switch must load proj-b, not ${sid}`);
      return Promise.resolve({ tracks: A_TRACKS, order: ["kick"], name: "proj-b" });
    };
    env.get("B").sessions = () => Promise.resolve([{ sid: "proj-a", name: "proj-a", dirty: false }, { sid: "proj-b", name: "proj-b", dirty: false }]);
    env.get("B").events = () => Promise.resolve(B_EVENTS);

    const p = env.get("switchTo")("proj-b");
    await env.settle();
    assert.equal(writes.length, 1);
    assert.deepEqual(plain(writes[0].args), ["proj-a", "kick",
      { sr: 48000, duration: 2, n_samples: 96000, events: A_EVENTS }]);
    assert.equal(writes[0].sidAtCall, "proj-a");   // the write went out before the switch moved the project
    assert.equal(env.get("S").sid, "proj-a");   // the switch has not moved the project yet

    writes[0].d.res({ followers: ["kick"] });   // the flushed save acks, then the switch moves
    assert.equal(await p, true);
    assert.equal(env.get("S").sid, "proj-b");
    assert.deepEqual(writes.map((w) => w.args[0]), ["proj-a"]);   // the write went to its own project
    assert.deepEqual(env.calls.followers, [[["kick"]]]);   // the ack refreshed the old project's followers
  },
};
