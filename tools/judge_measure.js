/* #385: the judge of static/agent.js over windows, for tools/judge_measure.py. Reads {agent, windows} as JSON on stdin and
   writes the aggregates as JSON: per gold set the exact / within-one / summed distance of the judge's move to the label,
   per Session K role the moves, and per shape key whether one key got more than one answer. Aggregates only. */
"use strict";
const fs = require("node:fs");
const vm = require("node:vm");

const { agent, windows } = JSON.parse(fs.readFileSync(0, "utf8"));
const win = {};
// DSAgent.judge is the pure decision; an older copy of the file (the "before" run) exposes it through the same hook
const src = agent.replace("window.DSAgent = DSAgent;", "DSAgent.judge = DSAgent.judge || judge; window.DSAgent = DSAgent;");
vm.runInNewContext(src, { window: win, $: () => null, document: {}, console });
const judge = win.DSAgent.judge;
// #394: the judge takes the reference (declared length 3): the local peak over the hit's peak, for the guard. The
// window then carries `stroke`, the raw samples of the detector's span (0.9 ms before to 5 ms after the hit, same
// start as `samples`), whose tail from hit_at holds the hit's peak. An older judge (the "before" run) is 2-arg.
const withRef = judge.length >= 3;

const gold = {}, k = {}, rows = [], inject = {};
for (const w of windows) {
  const peak = Math.max(0, ...w.samples.map(Math.abs));
  let j;
  if (withRef) {
    if (!w.stroke) throw new Error(`window ${w.id} has no stroke span`);
    const sp = Math.max(0, ...w.stroke.slice(w.hit_at).map(Math.abs));
    j = judge(w.samples.map(v => (peak > 0 ? v / peak : 0)), w.hit_at, peak > 0 && sp > 0 ? peak / sp : null);
  } else {
    j = judge(w.samples.map(v => (peak > 0 ? v / peak : 0)), w.hit_at);
  }
  if (w.set === "inject") {
    // #421: a labelled hit shifted `shift` samples late. flagged: the verdict is `late` (fixed: the suggestion lands on the
    // label, within1: one off); listed: no answer with a reason, or no line; explained: left with a reason; else unexplained.
    const r = inject[w.shift] || (inject[w.shift] = { n: 0, flagged: 0, fixed: 0, within1: 0, listed: 0, explained: 0, unexplained: 0 });
    const land = j.suggest === null ? null : j.suggest + w.shift;
    r.n++;
    if (j.verdict === "late") { r.flagged++; if (land === 0) r.fixed++; if (land !== null && Math.abs(land) <= 1) r.within1++; }
    else if (j.verdict === "no_quiet_line" || j.suggest === null) r.listed++;
    else if (j.why) r.explained++;
    else r.unexplained++;
    continue;
  }
  if (w.set === "K") {
    const r = k[w.role] || (k[w.role] = { n: 0, moves: 0, by_move: {}, shapes: {} });
    r.n++;
    if (j.suggest) r.moves++;
    if (j.suggest !== null) {
      r.by_move[j.suggest] = (r.by_move[j.suggest] || 0) + 1;
      if (j.shape !== undefined) (r.shapes[j.shape] = r.shapes[j.shape] || new Set()).add(j.suggest - j.edge);
    }
    rows.push({ set: "K", id: w.id, suggest: j.suggest });
  } else {
    const d = Math.abs((j.suggest || 0) - w.label_off);
    const g = gold[w.set] || (gold[w.set] = { n: 0, exact: 0, within1: 0, moves: 0, dist: 0, no_answer: 0 });
    if (j.suggest === null) g.no_answer++;                          // #408: exact counts "no answer" as 0; this says how many of them
    g.n++; g.dist += d; if (d === 0) g.exact++; if (d <= 1) g.within1++; if (j.suggest) g.moves++;
    if (w.owner) {                                                  // #408: the owner's own rows are the Gate's set
      const o = gold.owner || (gold.owner = { n: 0, exact: 0, within1: 0, moves: 0, dist: 0 });
      o.n++; o.dist += d; if (d === 0) o.exact++; if (d <= 1) o.within1++; if (j.suggest) o.moves++;
    }
    rows.push({ set: w.set, id: w.id, d, suggest: j.suggest || 0, label: w.label_off, edge: j.edge === undefined ? null : j.edge, shape: j.shape || null });
  }
}
for (const r of Object.values(k)) {
  const keys = Object.values(r.shapes);
  r.shape_keys = keys.length;
  r.keys_with_two_answers = keys.filter(s => s.size > 1).length;
  delete r.shapes;
}
console.log(JSON.stringify({ gold, k, rows, inject }));
