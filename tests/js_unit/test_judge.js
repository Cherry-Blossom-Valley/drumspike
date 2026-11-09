/* js_unit: the judge of static/agent.js (#394, #408). The judge measures the line directly before the stroke: the steepest
   step near the hit, walked back to its foot, and the line from the samples before the foot. The detector's band
   (DEPARTURE_VIS_PCT of the hit's peak) decides what counts as motion; the hit sample is part of the guard. The answer
   carries a shape verdict (`late`, `at_foot`, `no_quiet_line`) and `DSAgent.audit` lists the late hits from it alone. The
   agent file loads alone in a vm, the way tools/judge_measure.js does. */
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const ROOT = path.resolve(__dirname, "..", "..");

function load(globals) {
  const win = {};
  const src = fs.readFileSync(path.join(ROOT, "static", "agent.js"), "utf8")
    .replace("window.DSAgent = DSAgent;", "DSAgent.judge = DSAgent.judge || judge; window.DSAgent = DSAgent;");
  vm.runInNewContext(src, { window: win, $: () => null, document: {}, console, ...globals });
  return win.DSAgent;
}
const judge = () => load().judge;

const HIT = 43, LEN = 72;
/* A window in absolute units, the hit's peak 1.0: the band (2.5 % of it) is 0.025. `edit` writes the samples; the window is
   handed over the way `look` hands it, as shares of its own peak with the ratio of the two peaks. */
function win(edit, base = 0.01) {
  const a = new Array(LEN).fill(base);
  edit(a);
  const local = Math.max(...a.map(Math.abs));
  return { s: a.map(v => v / local), hitAt: HIT, ref: local / 1.0 };
}
const stroke = (a, at, rise = [0.15, 0.4, 0.55]) => { rise.forEach((v, k) => { a[at + 1 + k] = v; }); for (let i = at + 1 + rise.length; i < LEN; i++) a[i] = rise[rise.length - 1]; };

const tests = {
  async a_drift_at_the_far_left_does_not_hide_a_clean_edge_and_the_hit_moves_to_the_foot() {
    const { s, hitAt, ref } = win(a => {
      for (let i = 0; i < 12; i++) a[i] = 0.05 + 0.15 * (1 - i / 12);      // the drift the old line (the first 12 samples) read
      stroke(a, 40, [0.15, 0.35, 0.5]);                                     // flat from 12 to 40, the foot at 40, the hit 3 up
    }, 0.05);
    const j = judge()(s, hitAt, ref);
    assert.equal(j.foot_off, -3);
    assert.equal(j.suggest, -3);
    assert.equal(j.verdict, "late");
  },

  async a_dip_under_the_band_is_line_and_the_hit_moves_to_the_bottom_of_the_dip() {
    const { s, hitAt, ref } = win(a => {
      a[38] = 0.0; a[39] = -0.005; a[40] = -0.01; a[41] = -0.01;           // 0.02 under the line: under the 0.025 band
      stroke(a, 41, [0.2, 0.45, 0.5]);                                      // the bottom of the dip is the foot, the hit 2 up
    });
    const j = judge()(s, hitAt, ref);
    assert.equal(j.foot_off, -2);
    assert.equal(j.suggest, -2);
    assert.equal(j.verdict, "late");
  },

  async a_hit_sample_over_the_band_with_the_departure_at_the_hit_does_not_stand() {
    const { s, hitAt, ref } = win(a => { stroke(a, 42, [0.2, 0.5, 0.55]); });   // first_off 0: the hit sample is the first one up
    const j = judge()(s, hitAt, ref);
    assert.equal(j.first_off, 0);
    assert.equal(j.suggest, -1);
    assert.notEqual(j.why, "under the detector's band");
    assert.equal(j.verdict, "late");
  },

  async a_hit_sample_inside_the_band_stands() {
    const { s, hitAt, ref } = win(a => {
      for (let i = 38; i <= 43; i++) a[i] = 0.03;                          // a bump of 0.02, under the band, up to the hit
      stroke(a, 43, [0.3, 0.5, 0.55]);
    });
    const j = judge()(s, hitAt, ref);
    assert.equal(j.suggest, 0);
    assert.equal(j.verdict, "at_foot");
  },

  async a_slow_wave_before_the_edge_is_line_and_the_foot_is_the_edge() {
    const { s, hitAt, ref } = win(a => {
      for (let i = 0; i <= 40; i++) a[i] = 0.02 * Math.sin(2 * Math.PI * (i - 40) / 30);   // rising into the edge from sample 32
      stroke(a, 40, [0.1, 0.3, 0.5]);
    }, 0);
    const j = judge()(s, hitAt, ref);
    assert.equal(j.foot_off, -3, "the foot is at the edge, not where the wave began to rise");
    assert.equal(j.suggest, -3);
  },

  async a_ring_of_a_previous_stroke_leaves_no_quiet_line_and_the_hit_is_not_moved() {
    const { s, hitAt, ref } = win(a => {
      for (let i = 0; i <= 40; i++) a[i] = (i % 2 ? 1 : -1) * 0.22 * Math.exp(-i / 40);   // still ringing at the foot
      stroke(a, 40, [0.1, 0.35, 0.55]);
    }, 0);
    const j = judge()(s, hitAt, ref);
    assert.equal(j.suggest, null);
    assert.equal(j.verdict, "no_quiet_line");
  },

  async a_lead_bump_over_the_band_that_runs_into_the_stroke_still_moves_the_hit_before_it() {
    const { s, hitAt, ref } = win(a => {
      for (let i = 36; i <= 42; i++) a[i] = 0.06 + 0.01 * (i - 36);       // a bump over the band, no line sample between it and the stroke
      a[43] = 0.14;
      stroke(a, 43, [0.4, 0.55, 0.6]);
    });
    const j = judge()(s, hitAt, ref);
    assert.equal(j.suggest, -8, "the hit goes to the last line sample before the bump");
    assert.equal(j.verdict, "late");
  },

  async a_bump_under_the_detectors_band_gets_suggestion_0() {
    const { s, hitAt, ref } = win(a => {
      for (let i = 35; i <= 42; i++) a[i] = 0.03;                          // 2 % of the hit's peak over the line: under the 2.5 % band
      a[43] = 0.01;
      stroke(a, 43, [0.1, 0.3, 0.5]);
    });
    assert.equal(judge()(s, hitAt, ref).suggest, 0);
  },

  async a_hit_before_the_foot_that_is_inside_the_band_stays() {
    const { s, hitAt, ref } = win(a => { stroke(a, 45, [0.3, 0.5, 0.55]); });   // the line runs two past the hit
    const j = judge()(s, hitAt, ref);
    assert.equal(j.foot_off, 2);
    assert.equal(j.suggest, 0);
  },

  async a_window_without_a_step_over_the_band_leaves_the_hit_where_it_is() {
    const { s, hitAt, ref } = win(a => { stroke(a, 42, [0.02, 0.03, 0.035]); });   // the whole edge is under the band
    const j = judge()(s, hitAt, ref);
    assert.equal(j.suggest, 0);
    assert.equal(j.verdict, "at_foot");
  },

  async an_unreadable_hit_peak_reads_the_band_as_a_share_of_the_local_peak() {
    const { s, hitAt } = win(a => { stroke(a, 40, [0.15, 0.35, 0.5]); });
    const j = judge()(s, hitAt, null);
    assert.equal(j.suggest, -3);
  },

  async a_slow_stroke_whose_every_step_is_under_the_band_is_still_measured_and_the_late_hit_moves() {
    const { s, hitAt, ref } = win(a => { for (let i = 37; i < LEN; i++) a[i] = 0.01 + 0.02 * (i - 36); });   // 0.02 a sample: under the 0.025 band
    const j = judge()(s, hitAt, ref);
    assert.equal(j.verdict, "late");
    assert.equal(j.foot_off, -7);
    assert.equal(j.suggest, -7);
  },

  async a_hit_in_a_dip_after_the_stroke_passed_the_band_is_late() {
    const { s, hitAt, ref } = win(a => {
      a[37] = 0.06; a[38] = 0.12; a[39] = 0.06;                             // the stroke's first motion, over the band
      for (let i = 44; i < LEN; i++) a[i] = 0.01 + 0.03 * (i - 43);         // the hit sits on the line again, in the dip before the main rise
    });
    const j = judge()(s, hitAt, ref);
    assert.equal(j.rise, 0, "the hit's own sample is on the line");
    assert.equal(j.verdict, "late");
    assert.equal(j.suggest, -7);
  },

  async a_view_whose_level_stays_within_the_band_near_the_hit_stands() {
    const { s, hitAt, ref } = win(a => { a[42] = 0.012; for (let i = 60; i < LEN; i++) a[i] = 0.6; });   // a ripple near the hit; the stroke rises past the 6 samples after it
    const j = judge()(s, hitAt, ref);
    assert.equal(j.verdict, "at_foot");
    assert.equal(j.suggest, 0);
    assert.equal(j.why, "no motion above the band near the hit");
  },

  async audit_lists_every_late_hit_from_the_shape_measure_whatever_the_suggestion_says(env) {
    const late = win(a => { stroke(a, 40, [0.15, 0.35, 0.5]); });
    const standing = win(a => { stroke(a, 43, [0.3, 0.5, 0.55]); });
    // a late hit whose foot lies 14 samples before it: the suggestion is null ("far from the hit"), the verdict is still late
    const far = win(a => { stroke(a, 29, [0.15, 0.35, 0.5]); });
    const wins = [late, standing, far];
    const DSAgent = load({ S: { sid: "p", tracks: { k: { sr: 48000 } }, events: { k: wins.map((w, i) => ({ id: `h${i}`, t: i })) } } });
    DSAgent.look = async (track, i) => ({ judge: DSAgent.judge(wins[i].s, wins[i].hitAt, wins[i].ref) });
    const rows = await DSAgent.audit("k");
    assert.equal(rows.map(r => r.index).join(","), "0,2");        // joined: an array of the vm realm is no deepEqual match
    assert.equal(DSAgent.judge(far.s, far.hitAt, far.ref).suggest, null, "the suggestion alone would have hidden it");
    assert.equal(rows[0].foot_off, -3);
    assert.equal(rows[0].id, "h0");
  },
};

module.exports = { tests };
