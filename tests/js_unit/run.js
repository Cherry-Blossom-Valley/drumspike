/* js_unit runner (R17 step 2, #55): runs every test_*.js in this folder. Each file exports
   `tests`: named async functions that receive a fresh env (a fresh vm context per test).
   Output is one "ok"/"FAIL" line per test and a counted summary; exit 1 on any failure.
   A test that does not settle within 10 s counts as failed: a bare pending promise would
   otherwise let node exit silently with rc 0 and hide the hang.
   Run directly with node, or through tests/js_unit/test_js_unit.py (marker js_unit). */
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { makeEnv } = require("./harness.js");

const TEST_TIMEOUT_MS = 10000;

async function runOne(fn, env) {
  let timer;
  const guard = new Promise((_, rej) => {
    timer = setTimeout(() => rej(new Error(`the test did not settle within ${TEST_TIMEOUT_MS} ms`)), TEST_TIMEOUT_MS);
  });
  const work = fn(env);
  work.catch(() => {});   // a late rejection after a guard win must not crash the runner
  try {
    await Promise.race([work, guard]);
  } finally {
    clearTimeout(timer);
  }
}

async function main() {
  const files = fs.readdirSync(__dirname).filter(f => /^test_.*\.js$/.test(f)).sort();
  let passed = 0, failed = 0;
  for (const f of files) {
    const { tests } = require(path.join(__dirname, f));
    for (const [name, fn] of Object.entries(tests || {})) {
      const env = makeEnv();
      try {
        await runOne(fn, env);
        passed++;
        console.log(`ok ${f} :: ${name}`);
      } catch (e) {
        failed++;
        console.log(`FAIL ${f} :: ${name}\n  ${e && e.stack ? e.stack : e}`);
      }
    }
  }
  console.log(`js_unit: ${passed} passed, ${failed} failed`);
  process.exitCode = failed ? 1 : 0;
}

main().catch(e => { console.error(e); process.exitCode = 1; });
