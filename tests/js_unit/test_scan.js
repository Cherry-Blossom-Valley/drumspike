/* js_unit: DSAgent.scan (#412). The helper reads the detector's own candidate list (B.histograms) and the
   hits of the page and answers {candidates, limit}: a candidate the sensitivity keeps but that has no hit
   is listed (removed or merged away), a candidate the sensitivity cuts is listed only when its peak level
   (5 ms before to 60 ms after the time, read with B.samples) is over the quietest kept hit, and a
   candidate within the minimum gap of an existing hit is never listed. Without a candidate list the
   candidates are empty and the limit says to analyze the track once. static/agent.js loads alone in a vm,
   the way test_hand_placed.js does. */
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const ROOT = path.resolve(__dirname, "..", "..");

function load(globals) {
  const win = {};
  vm.runInNewContext(fs.readFileSync(path.join(ROOT, "static", "agent.js"), "utf8"), {
    window: win, $: () => null, document: {}, console, ...globals,
  });
  return win.DSAgent;
}

const SR = 48000;
/* structures that live in the vm context go through plain() before deepEqual (a cross-realm
   prototype never compares equal to a host literal, see test_edit.js) */
const plain = (x) => JSON.parse(JSON.stringify(x));

const EVS = [{ id: "h1", t: 0.1 }, { id: "h2", t: 0.2 }];
const QUIETEST = -40;   // the quietest kept hit, in dBFS, as the loudness histogram answers it

/* one samples read per merged span: 0.5 at index 100 of the whole track, silence elsewhere, so the
   candidate at 0.3 s measures -6 dB (over the quietest hit) and the one at 0.35 s measures silence */
function env(candidates, state) {
  const S = { sid: "proj-1", tracks: { kick: { sr: SR, params: { sensitivity: 50, min_gap_ms: 20 } } }, events: { kick: EVS.map(e => ({ ...e })) } };
  const asked = [];
  const B = {
    histograms: (sid, track, evs) => {
      asked.push(["histograms", sid, track, evs.map(e => e.t)]);
      return Promise.resolve({ loudness: { levels: [QUIETEST, -20] }, detection: { state: state || "ok", candidates } });
    },
    samples: (sid, track, a, b) => {
      asked.push(["samples", sid, track, a, b]);
      const data = new Array(b - a).fill(0);
      data[14400 - a] = 0.5;   // the peak of the candidate at 0.3 s (sample 14400), silence elsewhere
      return Promise.resolve({ start: a, data });
    },
  };
  return { S, B, asked, DSAgent: load({ S, B }) };
}

const tests = {
  async a_kept_candidate_without_a_hit_is_listed_and_a_candidate_at_a_hit_is_never_listed() {
    const e = env([[0.1, 0, 0.9, 0], [0.15, 0, 0.8, 0], [0.2, 0, 0.9, 0]]);
    const r = await e.DSAgent.scan("kick");
    assert.deepEqual(plain(r.candidates.map(c => c.time)), [0.15]);
    assert.equal(r.candidates[0].sample, 7200);
    assert.match(r.candidates[0].reason, /no hit is stored/);
    assert.match(r.limit, /candidate list/);
    assert.deepEqual(e.asked[0].slice(0, 3), ["histograms", "proj-1", "kick"]);
  },

  async a_cut_candidate_louder_than_the_quietest_kept_hit_is_listed_and_a_quieter_one_is_not() {
    const e = env([[0.3, 0, 0.4, 120], [0.35, 0, 0.3, 130]]);
    const r = await e.DSAgent.scan("kick");
    assert.deepEqual(plain(r.candidates.map(c => c.time)), [0.3]);
    assert.match(r.candidates[0].reason, /cut by the sensitivity.*louder than the quietest hit/);
    // the two spans overlap, so the page reads them as one merged window
    assert.equal(e.asked.filter(a => a[0] === "samples").length, 1);
  },

  async a_candidate_within_the_minimum_gap_of_a_hit_is_never_listed() {
    const e = env([[0.1 + 0.005, 0, 0.9, 0], [0.2 - 0.005, 0, 0.9, 0]]);   // 5 ms off: inside the 20 ms gap
    const r = await e.DSAgent.scan("kick");
    assert.deepEqual(plain(r.candidates), []);
  },

  async without_a_candidate_list_the_answer_is_empty_and_the_limit_says_so() {
    const e = env([], "missing");
    const r = await e.DSAgent.scan("kick");
    assert.deepEqual(plain(r.candidates), []);
    assert.match(r.limit, /analyze the track once/);
    assert.deepEqual(e.asked, [["histograms", "proj-1", "kick", [0.1, 0.2]]]);
  },
};

module.exports.tests = tests;
