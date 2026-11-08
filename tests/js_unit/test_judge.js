/* js_unit: the judge of static/agent.js (#394). The judge measures a lead bump against the hit's peak, the detector's
   reference (edge.rs EDGE_SPAN_S), not the look window's peak: a bump of 2 % of the stroke's peak that reads as 8 % of
   the local peak gets suggestion 0 or none. The agent file loads alone in a vm, the way tools/judge_measure.js does. */
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

/* Absolute sizes in stroke-peak shares (stroke peak 1.0): line 0.01, a lead bump of 0.02 (8 samples, ending one before
   the hit at index 43), then the stroke: 0.05 one sample after the hit and the rise to 0.25 at the look window's end.
   The look window's local peak is therefore 0.25 and the bump reads 8 % of it; over the detector's span the stroke
   peaks at 1.0, so the bump is 2 % of the hit's peak. */
function gateWindow() {
  const BEFORE = 43, AFTER = 29;
  const sig = [...new Array(35).fill(0.01), ...new Array(8).fill(0.02), 0.01, 0.05, ...new Array(AFTER - 2).fill(0.25)];
  const localPeak = 0.25, strokePeak = 1.0;
  return { s: sig.map(v => v / localPeak), hitAt: BEFORE, ref: localPeak / strokePeak };
}

const tests = {
  async bump_against_the_stroke_peak_is_no_counter_suggestion() {
    const { s, hitAt, ref } = gateWindow();
    const j = judge()(s, hitAt, ref);
    assert.ok(j.suggest === 0 || j.suggest === null, `suggest ${j.suggest}`);
  },

  async the_wrong_reference_would_counter_suggest_the_bump() {
    const { s, hitAt } = gateWindow();
    const j = judge()(s, hitAt, 1);            // shares of the local peak: the bump reads 8 %, over the 0.03 band
    assert.equal(j.suggest, -9);
  },

  async an_unreadable_stroke_peak_suggests_nothing_with_why() {
    const { s, hitAt } = gateWindow();
    const j = judge()(s, hitAt, null);
    assert.equal(j.suggest, null);
    assert.match(j.why, /stroke peak/);
  },
};

module.exports = { tests };
