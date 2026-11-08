/* js_unit: the judge of static/agent.js (#394). The rule's bands stay shares of the local peak, as on main; the hit's
   peak (the detector's reference, edge.rs EDGE_SPAN_S) feeds a guard: when the judge would move the hit earlier over a
   stretch that stands within the detector's own band (DEPARTURE_VIS_PCT of the hit's peak) of the line level, the bump
   is not the stroke's first motion and the answer is suggestion 0. The agent file loads alone in a vm, the way
   tools/judge_measure.js does. */
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const ROOT = path.resolve(__dirname, "..", "..");

function judge() {
  const win = {};
  const src = fs.readFileSync(path.join(ROOT, "static", "agent.js"), "utf8")
    .replace("window.DSAgent = DSAgent;", "DSAgent.judge = DSAgent.judge || judge; window.DSAgent = DSAgent;");
  vm.runInNewContext(src, { window: win, $: () => null, document: {}, console });
  return win.DSAgent.judge;
}

/* Absolute sizes in stroke-peak shares (stroke peak 1.0): line 0.01, a lead bump of `bump` (8 samples, ending one
   before the hit at index 43), then the stroke: 0.05 one sample after the hit and the rise to 0.25 at the look
   window's end. The look window's local peak is 0.25, so a bump of 0.02 reads as 8 % of the local peak (over the 0.03
   band) but 2 % of the hit's peak (under the detector's 2.5 % band). */
function gateWindow(bump) {
  const BEFORE = 43, AFTER = 29;
  const sig = [...new Array(35).fill(0.01), ...new Array(8).fill(bump), 0.01, 0.05, ...new Array(AFTER - 2).fill(0.25)];
  const localPeak = 0.25, strokePeak = 1.0;
  return { s: sig.map(v => v / localPeak), hitAt: BEFORE, ref: localPeak / strokePeak };
}

const tests = {
  async a_bump_under_the_detectors_band_gets_suggestion_0() {
    const { s, hitAt, ref } = gateWindow(0.02);
    const j = judge()(s, hitAt, ref);
    assert.equal(j.suggest, 0);
    assert.equal(j.why, "under the detector's band");
  },

  async a_real_departure_above_the_band_still_moves_earlier() {
    const { s, hitAt, ref } = gateWindow(0.06);   // 24 % of the local peak, over the 0.025 x 4 = 0.1 band
    const j = judge()(s, hitAt, ref);
    assert.equal(j.suggest, -9);
    assert.equal(j.why, undefined);
  },

  async an_unreadable_hit_peak_leaves_mains_answer_standing() {
    const { s, hitAt } = gateWindow(0.02);
    const j = judge()(s, hitAt, null);            // no readable reference: the guard does not apply
    assert.equal(j.suggest, -9);
    assert.equal(j.why, undefined);
  },

  async a_later_move_is_unchanged() {
    // The departure after the hit: the line runs one sample past the hit, the stroke starts two after it (suggest = 1).
    const BEFORE = 43, AFTER = 29;
    const sig = [...new Array(BEFORE + 2).fill(0.01), 0.05, ...new Array(AFTER - 3).fill(0.25)];
    const j = judge()(sig.map(v => v / 0.25), BEFORE, 0.25);
    assert.equal(j.suggest, 1);
  },
};

module.exports = { tests };
