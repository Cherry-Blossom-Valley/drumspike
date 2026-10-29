/* js_unit harness (R17 step 2, #55 per docs/review/R17-plan.md): loads the static/ plain
   scripts into a fresh vm context per test — a stubbed B, a minimal document/window, a
   localStorage stand-in and a manual clock. No browser, no full DOM. The modules keep their
   real script order (state.js first, then the module under test); their top-level bindings
   stay reachable through get(expr), the way page.evaluate sees them in the e2e suite. */
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const STATIC_DIR = path.resolve(__dirname, "..", "..", "static");

function defer() {
  let res, rej;
  const promise = new Promise((a, b) => { res = a; rej = b; });
  return { promise, res, rej };
}

/* a manual clock: setTimeout records, tick(ms) runs every callback that is due, newest last;
   timers scheduled while a callback runs are picked up by the next pass. */
function makeClock() {
  const clock = { now: 0, _seq: 0, _timers: [] };
  clock.setTimeout = (fn, ms) => {
    const id = ++clock._seq;
    clock._timers.push({ id, fn, at: clock.now + (ms || 0) });
    return id;
  };
  clock.clearTimeout = (id) => { clock._timers = clock._timers.filter(t => t.id !== id); };
  clock.tick = (ms) => {
    clock.now += ms;
    for (;;) {
      const due = clock._timers.filter(t => t.at <= clock.now).sort((a, b) => a.at - b.at);
      if (!due.length) return;
      clock._timers = clock._timers.filter(t => !due.includes(t));
      for (const t of due) t.fn();
    }
  };
  return clock;
}

function stubEl() {
  const el = { hidden: true, toggles: [], style: {}, textContent: "",
    classList: {
      contains(c) { return c === "hidden" ? el.hidden : false; },
      toggle(c, force) { el.toggles.push([c, force]); if (c === "hidden") el.hidden = !!force; },
      add() {}, remove() {},
    } };
  return el;
}

function makeEnv() {
  const clock = makeClock();
  const calls = { setEvents: [], clean: [], followers: [] };
  const els = {};
  const sandbox = {
    console,
    performance: { now: () => clock.now },
    setTimeout: clock.setTimeout,
    clearTimeout: clock.clearTimeout,
    localStorage: {
      _d: {},
      getItem(k) { return k in this._d ? this._d[k] : null; },
      setItem(k, v) { this._d[k] = String(v); },
      removeItem(k) { delete this._d[k]; },
    },
    document: { title: "" },
    window: {},
    $: (sel) => (els[sel] = els[sel] || stubEl()),
    DSShell: { sync() {} },
    setMsg() {},
    setMsgBg() {},
    errText: (e) => (e && e.message) || String(e),
    isManual: () => false,
    requestFrame() {},
    reloadFollowers(...a) { calls.followers.push(a); return Promise.resolve(); },
    followersOf: (t) => [t],
    B: {
      setEvents(...a) { calls.setEvents.push({ args: a, d: defer() }); return calls.setEvents[calls.setEvents.length - 1].d.promise; },
      projectCleanCheck(...a) { calls.clean.push(a); return Promise.resolve({ dirty: false }); },
    },
  };
  const ctx = vm.createContext(sandbox);
  return {
    clock, calls, els, sandbox, defer,
    load(rel) { vm.runInContext(fs.readFileSync(path.join(STATIC_DIR, rel), "utf8"), ctx, { filename: rel }); },
    get(expr) { return vm.runInContext(`(${expr})`, ctx); },
    /* drain pending microtasks (an ack that arrived, a clean check that resumed) */
    async settle() { await new Promise(r => setImmediate(r)); },
  };
}

module.exports = { makeEnv, stubEl, defer };
