/* js_unit: the pattern of a track in static/agent.js (#447). An engineer knows how the clear hits of a track look and
   places an unclear hit by that. The pattern is built from the track's own clear hits; the judge moves a hit only when the
   pattern confirms the move within one sample, and a hit where the two differ by 2 or more goes on the list. The agent file
   loads alone in a vm, the way tools/judge_measure.js does. */
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const ROOT = path.resolve(__dirname, "..", "..");
const HOOK = "DSAgent.judge = DSAgent.judge || judge; DSAgent._pat = typeof patternBuild === 'function' ? { patternBuild, patternOffset, second } : null; window.DSAgent = DSAgent;";

function load(globals) {
  const win = {};
  const src = fs.readFileSync(path.join(ROOT, "static", "agent.js"), "utf8").replace("window.DSAgent = DSAgent;", HOOK);
  vm.runInNewContext(src, { window: win, $: () => null, document: {}, console, ...globals });
  return win.DSAgent;
}

const SR = 48000, BEFORE = 96, AFTER = 248;                 // 2 ms before the hit and 5 ms after it at 48 kHz, and room for a hit a few samples late
/* one stroke of a generated track: a quiet line with a little hum, then a rise whose first motion is the sample after
   `foot`. Every stroke has its own level and its own tail, as on a real track; the foot is the same shape. */
function stroke(k, foot = BEFORE, noise = 0.002) {
  const level = 0.4 + 0.5 * ((k * 37) % 17) / 17, x = [];
  for (let i = 0; i < BEFORE + AFTER; i++) {
    const n = noise * Math.sin(i * 1.7 + k) * level;
    const d = i - foot;
    let v = n;
    if (d > 0) v += level * Math.min(1, [0, 0.04, 0.12, 0.3, 0.55, 0.8, 0.95][Math.min(d, 6)] + (d > 6 ? 0.05 * Math.sin(d * (0.3 + 0.02 * (k % 5))) : 0));
    x.push(v);
  }
  return x;
}
const track = n => Array.from({ length: n }, (_, k) => ({ x: stroke(k), at: BEFORE }));

const tests = {
  async the_pattern_flags_the_one_hit_that_stands_three_samples_late_and_no_other() {
    const P = load()._pat;
    assert.ok(P, "the agent file carries the pattern");
    const rows = track(41);
    rows[20].at = BEFORE + 3;                                 // one hit 3 samples up its rise
    const pat = P.patternBuild(rows, SR);
    assert.ok(pat && pat.hits === 41 && pat.clear >= 15, `a pattern from the clear hits: ${JSON.stringify(pat && { hits: pat.hits, clear: pat.clear })}`);
    const offs = rows.map(r => P.patternOffset(r.x, r.at, pat).off);
    assert.equal(offs[20], -3, "the late hit: the pattern puts its start 3 samples earlier");
    const others = offs.filter((_, k) => k !== 20);
    assert.equal(others.length, 40);
    assert.ok(others.every(o => o === 0), `every other hit matches at its own sample: ${others.join(",")}`);
  },
  async the_pattern_carries_no_shift_of_its_own_when_all_hits_stand_one_sample_late() {
    // the rule's own habit on a track is not an error of one hit: the pattern stays where the clear hits stand
    const P = load()._pat, rows = track(41).map(r => ({ x: r.x, at: r.at + 1 }));
    const pat = P.patternBuild(rows, SR);
    assert.ok(rows.every(r => P.patternOffset(r.x, r.at, pat).off === 0));
  },
  async the_pattern_takes_its_position_from_the_hits_at_the_foot_when_half_the_track_stands_late() {
    // half of the hits at the foot (the clear ones), a quarter 2 late, a quarter 3 late: a hit up its rise has no quiet
    // line, so it does not shape the pattern. Every hit gets its own distance back.
    const late = [0, 2, 0, 3], P = load()._pat, rows = Array.from({ length: 60 }, (_, k) => ({ x: stroke(k), at: BEFORE + late[k % 4] }));
    const pat = P.patternBuild(rows, SR);
    assert.equal(pat.clear, 30, "only the hits at the foot have a quiet line");
    const offs = rows.map(r => P.patternOffset(r.x, r.at, pat).off);
    assert.equal(offs.length, 60);
    assert.ok(offs.every((o, k) => o === -late[k % 4]), offs.join(","));
  },
  async a_track_with_fewer_than_40_hits_has_no_pattern_and_the_judge_answers_as_before() {
    const A = load(), P = A._pat;
    assert.equal(P.patternBuild(track(39), SR), null);
    const j = { suggest: -3, verdict: "late", foot_off: -3 };
    const out = P.second(j, null);
    assert.equal(out.suggest, -3);
    assert.equal(out.pattern_off, null);
    assert.equal(out.pattern_agrees, null);
    assert.equal(out.line_suggest, undefined);
  },
  async a_move_the_pattern_confirms_within_one_sample_stays_a_move() {
    const P = load()._pat;
    for (const off of [-4, -3, -2]) {
      const out = P.second({ suggest: -3, verdict: "late" }, { off, score: 0.99 });
      assert.equal(out.suggest, -3, `pattern at ${off}`);
      assert.equal(out.pattern_agrees, true);
      assert.equal(out.pattern_off, off);
    }
  },
  async a_move_the_pattern_does_not_confirm_becomes_a_listed_hit_with_both_positions() {
    const P = load()._pat;
    const out = P.second({ suggest: -3, verdict: "late", foot_off: -3 }, { off: 0, score: 0.99 });
    assert.equal(out.suggest, null, "no move");
    assert.equal(out.line_suggest, -3, "the judge's own suggestion is kept for the reader");
    assert.equal(out.verdict, "late", "the verdict is the judge's: audit still lists the hit");
    assert.equal(out.why, "two opinions differ: the line before the hit puts the start 3 samples earlier, the track's clear hits put it at the hit");
  },
  async a_standing_hit_the_pattern_puts_two_or_more_away_is_listed_and_one_away_stands() {
    const P = load()._pat;
    const one = P.second({ suggest: 0, verdict: "at_foot" }, { off: -1, score: 0.99 });
    assert.equal(one.suggest, 0);
    assert.equal(one.pattern_agrees, true);
    const two = P.second({ suggest: 0, verdict: "at_foot" }, { off: -2, score: 0.99 });
    assert.equal(two.suggest, null);
    assert.equal(two.line_suggest, 0);
    assert.equal(two.why, "two opinions differ: the line before the hit puts the start at the hit, the track's clear hits put it 2 samples earlier");
  },
  async no_answer_stays_no_answer_and_carries_the_patterns_position() {
    const P = load()._pat;
    const out = P.second({ suggest: null, verdict: "no_quiet_line", why: "x" }, { off: -5, score: 0.9 });
    assert.equal(out.suggest, null);
    assert.equal(out.why, "x");
    assert.equal(out.pattern_off, -5);
    assert.equal(out.line_suggest, undefined);
  },
  async a_view_too_short_for_the_search_gives_no_opinion() {
    const P = load()._pat, rows = track(41), pat = P.patternBuild(rows, SR);
    assert.equal(P.patternOffset(rows[0].x.slice(BEFORE - 20), 20, pat), null, "20 samples before the hit do not hold the search range");
  },
  async a_hit_that_does_not_look_like_the_pattern_gets_no_opinion() {
    const P = load()._pat, rows = track(41), pat = P.patternBuild(rows, SR);
    // a stroke of the other polarity: no position matches well (the best accidental match is far under 0.5)
    const flipped = rows[3].x.map(v => -v);
    assert.equal(P.patternOffset(flipped, BEFORE, pat), null);
    // a hit whose foot lies at the end of the search range: the best match is at the edge, so it is no peak
    assert.equal(P.patternOffset(rows[3].x, BEFORE + pat.search, pat), null);
    // one sample inside the range is an opinion
    assert.equal(P.patternOffset(rows[3].x, BEFORE + pat.search - 1, pat).off, -(pat.search - 1));
    // and the judge then answers alone
    assert.equal(P.second({ suggest: -11, verdict: "late" }, P.patternOffset(flipped, BEFORE, pat)).suggest, -11);
  },
  async the_spans_follow_the_sample_rate() {
    const P = load()._pat;
    const up = r => ({ x: r.x.flatMap(v => [v, v]), at: r.at * 2 });   // the same strokes at twice the rate
    const pat48 = P.patternBuild(track(41), SR), pat96 = P.patternBuild(track(41).map(up), 2 * SR);
    assert.equal(pat96.pre, 2 * pat48.pre);
    assert.equal(pat96.post, 2 * pat48.post);
    assert.equal(pat96.search, 2 * pat48.search);
  },
  async audit_lists_a_late_hit_and_a_hit_whose_two_opinions_differ_with_the_reason() {
    const judged = [
      { verdict: "late", suggest: -3, foot_off: -3, rise: 0.2, quiet_ratio: 50, pattern_off: -3, pattern_agrees: true },
      { verdict: "at_foot", suggest: 0, foot_off: 0, rise: 0, quiet_ratio: 50, pattern_off: 0, pattern_agrees: true },
      { verdict: "at_foot", suggest: null, line_suggest: 0, foot_off: 0, rise: 0, quiet_ratio: 50, pattern_off: -4, pattern_agrees: false, why: "two opinions differ: a" },
      { verdict: "late", suggest: null, line_suggest: -3, foot_off: -3, rise: 0.2, quiet_ratio: 50, pattern_off: 0, pattern_agrees: false, why: "two opinions differ: b" },
    ];
    const DSAgent = load({ S: { sid: "p", tracks: { k: { sr: SR } }, events: { k: judged.map((_, i) => ({ id: `h${i}`, t: i })) } } });
    DSAgent.look = async (_, i) => ({ judge: judged[i] });
    const rows = await DSAgent.audit("k");
    assert.equal(rows.map(r => `${r.index}:${r.reason}`).join(" "), "0:late 2:opinions differ 3:opinions differ");
    assert.equal(rows[0].suggest, -3);
    assert.equal(rows[1].pattern_off, -4);
    assert.equal(rows[1].why, "two opinions differ: a");
    assert.equal(rows[1].suggest, null);
    assert.equal(rows[2].foot_off, -3);
  },
};

module.exports = { tests };
