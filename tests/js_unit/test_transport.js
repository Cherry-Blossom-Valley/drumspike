/* js_unit: the transport.js surfaces (R17 step 4, #55 per docs/review/R17-plan.md).
   The plan limits this step's module tests to two things: the serial command queue
   (appCmd — the commands run one after the other, in the order the user made them, and
   a refused command does not stall the queue) and the playhead arithmetic (appPos — the
   transport model extrapolated on the page clock, P3-20b half speed). The clock sync,
   the seek and the audio paths stay under the browser tests (test_playback.py and
   friends); each test gets a fresh context with state.js and transport.js in script
   order and the harness's manual clock. */
"use strict";

const assert = require("node:assert/strict");

function freshTransportEnv(env) {
  env.load("state.js");
  env.load("transport.js");
  return env;
}

module.exports.tests = {
  /* the queue is serial: the second command does not start before the first resolved,
     and each answer goes to its own caller. */
  async "appCmd runs the queued commands in order, one after the other"(env) {
    freshTransportEnv(env);
    const appCmd = env.get("appCmd");
    const order = [];
    const d1 = env.defer(), d2 = env.defer();
    const p1 = appCmd(async () => { order.push("first"); await d1.promise; return "r1"; });
    const p2 = appCmd(async () => { order.push("second"); await d2.promise; return "r2"; });
    await env.settle();
    assert.deepEqual(order, ["first"]);   // the second command waits for the first
    d1.res("r1");
    assert.equal(await p1, "r1");
    await env.settle();
    assert.deepEqual(order, ["first", "second"]);
    d2.res("r2");
    assert.equal(await p2, "r2");
  },

  /* a refused command does not stall the queue: the chain catches the rejection (the
     next command still runs) and the rejection still reaches its own caller. */
  async "a refused command does not stall the queue"(env) {
    freshTransportEnv(env);
    const appCmd = env.get("appCmd");
    const ran = [];
    const refused = appCmd(async () => { ran.push("first"); throw new Error("the transport refused"); });
    const next = appCmd(async () => { ran.push("second"); return "went on"; });
    await env.settle();
    await assert.rejects(refused, /the transport refused/);
    assert.equal(await next, "went on");
    assert.deepEqual(ran, ["first", "second"]);
  },

  /* appPos: idle is the model position; playing extrapolates pos + (now - at)/1000*speed
     on the page clock; half speed halves the extrapolation (P3-20b); stopped it is the
     model position again. */
  async "appPos extrapolates the transport clock at the play speed"(env) {
    freshTransportEnv(env);
    env.clock.now = 5000;
    env.get("(() => { Object.assign(AC, { running: false, pos: 10, at: 3000, speed: 1 }); })()");
    assert.equal(env.get("appPos(performance.now())"), 10);

    env.get("(() => { AC.running = true; })()");
    assert.equal(env.get("appPos(performance.now())"), 12);   // 10 + (5000 - 3000)/1000 * 1

    env.get("(() => { AC.speed = 0.5; })()");
    assert.equal(env.get("appPos(performance.now())"), 11);   // P3-20b: half speed

    env.get("(() => { AC.running = false; })()");
    assert.equal(env.get("appPos(performance.now())"), 10);   // stopped: the model position again
  },
};
