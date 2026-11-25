#!/usr/bin/env node
// Measure (read only): reads back from the open LUNA session the notes and clips that a press of
// "Deliver to tracks" made, and prints how far their positions are from the list file, in samples.
// It posts nothing and writes nothing to the session. It connects to LUNA on localhost (the
// engine socket, port 4718 unless --port is given), so LUNA must be running with the session open.
//
//   node tools/measure.mjs drumspike-deliver.json [--port 4718]
//
// What it can and cannot see: NOTE.md, "What the measure proves".
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadLogic } from './luna-node.mjs';

/** Reads the list, measures the focused session of `luna`, and returns the text and the report. */
export async function runMeasure(luna, listText) {
  const D = loadLogic();
  const list = D.parseList(listText);
  await luna.sessions.ready();
  const session = luna.sessions.current();
  if (!session) throw new Error('No session is open in LUNA.');
  await session.ready();
  const report = await D.measure(luna, session, list);
  return { text: `Session: ${session.name.value ?? session.uid}\n${D.formatReport(report)}`, report };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const args = process.argv.slice(2);
  const i = args.indexOf('--port');
  const port = i < 0 ? undefined : Number(args.splice(i, 2)[1]);
  if (args.length !== 1) {
    console.error('usage: node tools/measure.mjs <drumspike-deliver.json> [--port 4718]');
    process.exit(2);
  }
  loadLogic(); // loads the SDK build into this process
  const luna = await globalThis.Luna.connect(port ? { port } : {});
  try {
    const { text } = await runMeasure(luna, readFileSync(args[0], 'utf8'));
    console.log(text);
  } catch (e) {
    console.error(`error: ${e.message}`);
    process.exitCode = 1;
  } finally {
    await luna.close();
  }
}
