// Shared set-up of the tests: the SDK build of vendor/, Luna.MockEngine, small fixtures. No DAW is used.
import { loadLogic, loadLuna } from '../tools/luna-node.mjs';

export const Luna = loadLuna();
export const D = loadLogic();
export const BEAT = Luna.TICKS_PER_BEAT; // 96e9 ticks per beat

/** A session with one existing track that holds an audio clip, and the given tempo rows. */
export async function connect({ tempos = [{ tick: 0, tempo: 120 }], existing = ['Existing'] } = {}) {
  const engine = new Luna.MockEngine();
  engine.addSession('abc', { name: 'Song', tracks: existing });
  // the mock starts with one 120 BPM row; replace the rows with the wanted ones
  const base = '/sessions/abc/tempo_map';
  engine.setValue(`${base}/1`, 'tempo', tempos[0].tempo);
  tempos.slice(1).forEach((row, i) => engine.addNode(`${base}/${i + 2}`, { properties: { tick: row.tick, time: 0, tempo: row.tempo } }));
  await engine.settled();
  const luna = await Luna.connect({ port_: engine.port() });
  const session = await luna.session();
  await session.tracks.membersReady();
  return { engine, luna, session };
}

/** The bytes of a prepared file: the mock does not read them, only counts them. */
export const bytes = (name) => new TextEncoder().encode(`fake audio of ${name}`);

export function listOf(tracks) {
  return D.parseList(JSON.stringify({ schema: 'drumspike-deliver', version: 1, tracks }));
}

/** Exact tick of `sample` at whole BPM rows, in BigInt rationals (the reference of the tests). */
export function exactTick(sample, rate, rows) {
  // rows: [{tick, tempo}] sorted; every tempo is a whole number of BPM and every row tick a whole number of beats
  let remaining = BigInt(sample); // samples left to place
  let tick = 0n;
  for (let i = 0; i < rows.length; i++) {
    const bpm = BigInt(rows[i].tempo);
    const next = rows[i + 1];
    // samples in this segment: (nextTick - tick) ticks = beats * 60 / bpm seconds * rate samples
    const segTicks = next ? BigInt(next.tick) - BigInt(rows[i].tick) : null;
    const segSamples = segTicks === null ? null : (segTicks * 60n * BigInt(rate)) / (BigInt(BEAT) * bpm);
    if (segSamples !== null && remaining >= segSamples) {
      remaining -= segSamples;
      tick = BigInt(next.tick);
      continue;
    }
    // ticks = samples * 96e9 * bpm / (60 * rate), rounded to nearest
    const num = remaining * BigInt(BEAT) * bpm;
    const den = 60n * BigInt(rate);
    return Number(tick + (2n * num + den) / (2n * den));
  }
  throw new Error('no tempo rows');
}

/** A snapshot of every node's properties, to prove that existing nodes are not changed. */
export function snapshotNodes(engine) {
  const out = new Map();
  for (const [path, node] of engine.nodes) out.set(path, JSON.stringify([...node.properties.entries()]));
  return out;
}
