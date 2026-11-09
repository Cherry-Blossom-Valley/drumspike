/* js_unit: DSAgent.handPlaced (#418). The training log is far too large to read: the helper names the
   hand-placed hits through the service's trimmed answer (static/agent.js loads alone in a vm, the way
   test_judge.js does). A log with a person record and an agent record must yield only the person's hit
   as hand-placed; a record no stored hit matches is dropped; an older record without `by` is a person's. */
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

function env(records) {
  const evs = [
    { id: "h1", t: 0.1 }, { id: "h2", t: 0.2 }, { id: "h3", t: 0.3 }, { id: "h4", t: 0.4 },
  ];
  const S = { sid: "proj-1", tracks: { kick: { sr: SR } }, events: { kick: evs } };
  const asked = [];
  const B = {
    trainingHandPlaced: (sid, track) => { asked.push([sid, track]); return Promise.resolve({ records, path: "/log.jsonl" }); },
  };
  return { S, B, asked, DSAgent: load({ S, B }) };
}

const rec = (over) => ({ track: "kick", op: "move", id: null, sample: null, ...over });

const tests = {
  async a_person_record_and_an_agent_record_yield_only_the_persons_hit() {
    const e = env([
      rec({ op: "move", by: "person", id: "h2", sample: 9600 }),
      rec({ op: "move", by: "agent", id: "h3", sample: 14400 }),
    ]);
    const rows = await e.DSAgent.handPlaced("kick");
    assert.deepEqual(plain(rows.map(r => r.id)), ["h2"]);
    assert.deepEqual(plain(rows[0]), { index: 1, id: "h2", t: 0.2, sample: 9600, ops: ["move"] });
    assert.deepEqual(e.asked, [["proj-1", "kick"]]);
  },

  async a_confirm_an_older_record_without_by_and_an_add_count_a_delete_and_a_learn_do_not() {
    const e = env([
      rec({ op: "confirm", by: "person", id: "h1", sample: 4800 }),
      rec({ op: "move", id: "h4", sample: 19200 }),                     // an older log carries no `by`
      rec({ op: "add", by: "person", id: null, sample: 555 }),          // no stored hit at 555: dropped
      rec({ op: "delete", by: "person", id: "h2", sample: 9600 }),
      rec({ op: "learn_this", by: "person", id: "h2", sample: 1234 }),
    ]);
    const rows = await e.DSAgent.handPlaced("kick");
    assert.deepEqual(plain(rows.map(r => r.id)), ["h1", "h4"]);
    assert.deepEqual(plain(rows[1].ops), ["move"]);
  },

  async a_record_whose_id_is_gone_falls_back_to_the_sample_and_one_hit_folds_its_records() {
    const e = env([
      rec({ op: "move", by: "person", id: "gone", sample: 14400 }),
      rec({ op: "confirm", by: "person", id: "h3", sample: 14400 }),
    ]);
    const rows = await e.DSAgent.handPlaced("kick");
    assert.deepEqual(plain(rows), [{ index: 2, id: "h3", t: 0.3, sample: 14400, ops: ["move", "confirm"] }]);
  },

  async an_empty_log_answers_no_rows() {
    const e = env([]);
    assert.deepEqual(plain(await e.DSAgent.handPlaced("kick")), []);
  },
};

module.exports.tests = tests;
