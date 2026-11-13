/* js_unit: DSAgent.scan (#412). The helper reads the detector's own candidate list (B.histograms) and
   the hits of the page and answers {candidates, left_out, limit}: each stored hit claims at most one
   candidate (the nearest in its refine window, a flam partner cannot cover a neighbour's), a kept
   candidate that no hit claims is listed (removed or merged away), and a cut candidate is listed
   sorted by the needed sensitivity unless it is bleed (at the time of a hit on another track, within
   half the minimum gap) or tail (inside the 60 ms decay of an own hit) - the rules read no samples,
   so B.samples is never called. Without a candidate list the candidates are empty and the limit says
   to analyze the track once. static/agent.js loads alone in a vm, the way test_hand_placed.js does. */
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

/* `others`: the hits of the other tracks of the project (rule a reads them); the scan reads no
   samples, so a samples call would fail the test through `asked` */
function env(candidates, state, others) {
  const S = { sid: "proj-1", tracks: { kick: { sr: SR, params: { sensitivity: 50, min_gap_ms: 20 } },
                                      snare: { sr: SR, params: {} } },
              events: { kick: EVS.map(e => ({ ...e })), snare: (others || []).map(t => ({ t })) } };
  const asked = [];
  const B = {
    histograms: (sid, track, evs) => {
      asked.push(["histograms", sid, track, evs.map(e => e.t)]);
      return Promise.resolve({ loudness: { levels: [-20] }, detection: { state: state || "ok", candidates } });
    },
    samples: (sid, track, a, b) => {
      asked.push(["samples", sid, track, a, b]);
      throw new Error("scan must not read samples");
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

  async each_hit_claims_only_its_nearest_candidate_and_a_neighbour_14_ms_away_is_listed() {
    /* Finding 2: two candidates 14 ms apart, one stored hit (at the first): the hit claims the
       candidate at its own time; the other one, 14 ms later, no hit claims and it is listed */
    const e = env([[0.3, 0, 0.9, 0], [0.314, 0, 0.9, 0]]);
    const S = e.S; S.events.kick = [{ id: "h9", t: 0.3 }];
    const r = await e.DSAgent.scan("kick");
    assert.deepEqual(plain(r.candidates.map(c => c.time)), [0.314]);
    assert.match(r.candidates[0].reason, /no hit is stored/);
  },

  async a_cut_candidate_is_listed_with_the_slider_it_would_need_and_the_reads_are_sorted_by_it() {
    const e = env([[0.3, 0, 0.4, 120], [0.4, 0, 0.3, 80]]);
    const r = await e.DSAgent.scan("kick");
    assert.deepEqual(plain(r.candidates.map(c => c.time)), [0.4, 0.3]);   // nearest to the slider first
    assert.match(r.candidates[0].reason, /^cut by the sensitivity \(the slider would need 80\)$/);
    assert.deepEqual(e.asked.filter(a => a[0] === "samples").length, 0);  // no level reads any more
  },

  async a_cut_candidate_at_the_time_of_a_hit_on_another_track_is_bleed_and_not_listed() {
    const e = env([[0.3, 0, 0.4, 120]], "ok", [0.295]);   // 5 ms from the snare hit: inside half the gap
    const r = await e.DSAgent.scan("kick");
    assert.deepEqual(plain(r.candidates), []);
    assert.deepEqual(plain(r.left_out), { bleed: 1, tail: 0 });
    assert.match(r.limit, /left_out/);
  },

  async a_cut_candidate_inside_the_decay_of_an_own_hit_is_its_tail_and_not_listed() {
    const e = env([[0.25, 0, 0.4, 120]]);                 // 50 ms after the hit at 0.2: inside the 60 ms
    const r = await e.DSAgent.scan("kick");
    assert.deepEqual(plain(r.candidates), []);
    assert.deepEqual(plain(r.left_out), { bleed: 0, tail: 1 });
  },

  async a_cut_candidate_after_the_decay_and_far_from_the_other_tracks_hits_is_listed() {
    const e = env([[0.35, 0, 0.4, 120]], "ok", [0.3]);    // 150 ms after the hit, 50 ms from the snare hit
    const r = await e.DSAgent.scan("kick");
    assert.deepEqual(plain(r.candidates.map(c => c.time)), [0.35]);
    assert.deepEqual(plain(r.left_out), { bleed: 0, tail: 0 });
  },

  async a_candidate_within_the_refine_window_of_a_hit_is_claimed_and_never_listed() {
    const e = env([[0.1 + 0.005, 0, 0.9, 0], [0.2 - 0.005, 0, 0.9, 0]]);   // 5 ms off: inside the window
    const r = await e.DSAgent.scan("kick");
    assert.deepEqual(plain(r.candidates), []);
  },

  async without_a_candidate_list_the_answer_is_empty_and_the_limit_says_so() {
    const e = env([], "missing");
    const r = await e.DSAgent.scan("kick");
    assert.deepEqual(plain(r.candidates), []);
    assert.match(r.limit, /analyze the track once/);
    assert.deepEqual(plain(r.left_out), { bleed: 0, tail: 0 });
    assert.deepEqual(e.asked, [["histograms", "proj-1", "kick", [0.1, 0.2]]]);
  },
};

module.exports.tests = tests;
