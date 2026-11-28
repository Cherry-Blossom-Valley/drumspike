#!/usr/bin/env node
// Makes `drumspike-deliver.json` (schema drumspike-deliver, version 1) from the prepared files of
// a Drum Spike plugin project, for the proof of the Add On.
//
//   node tools/make-list.mjs <project folder | Exports/.prepared folder> [--out file] [--rate 48000]
//
// Route: the prepared files only, exactly what the plugin hands to the DAW (m1c3-one-drag.md):
//   <track>_spike.mid         the hits: note, velocity, and a tick in song time from the session start
//   <track>_spike.wav         the synthetic spike file, from song time 0 (rate and frames come from it)
//   <track>_audio_spike.wav   the audio spike file, the same
// The MIDI ticks are turned into samples with the tempo events of the file itself, exactly (whole
// number arithmetic), then rounded to the nearest sample. Accuracy of this route, in the list as
// `accuracy`: the file holds a tick, not a sample. One tick is `samplesPerTick` samples long. If a
// tick is one sample or shorter (the plugin picks that where it can), the rounding gives back the
// plugin's sample exactly. If a tick is longer, the position is within half a tick, rounded to a
// whole sample. The list states the worst case per track. The Add On does not need the plugin to change.
//
// Not done: the route over the project events (sample exact, no tick grid). That needs the row and
// clip mapping of the plugin (rows.rs, prepare.rs) and so belongs in the plugin. See NOTE.md.
import { readFileSync, readdirSync, statSync, writeFileSync, existsSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// ---------------------------------------------------------------- standard MIDI file

function readVarLen(buf, at) {
  let value = 0;
  let i = at;
  for (;;) {
    if (i >= buf.length) throw new Error('MIDI file ends inside a number');
    const b = buf[i++];
    value = value * 128 + (b & 0x7f);
    if (!(b & 0x80)) return [value, i];
  }
}

/** Reads a standard MIDI file: ticks per quarter note, tempo events, note-ons. */
export function parseSmf(buf) {
  if (buf.length < 14 || buf.toString('latin1', 0, 4) !== 'MThd') throw new Error('not a MIDI file');
  const ntracks = buf.readUInt16BE(10);
  const division = buf.readUInt16BE(12);
  if (division & 0x8000) throw new Error('MIDI file uses SMPTE time; only ticks per quarter note are read');
  const tempos = [];
  const notes = [];
  let at = 14 + (buf.readUInt32BE(4) - 6);
  for (let t = 0; t < ntracks; t++) {
    if (buf.toString('latin1', at, at + 4) !== 'MTrk') throw new Error('MIDI track header missing');
    const end = at + 8 + buf.readUInt32BE(at + 4);
    let i = at + 8;
    let tick = 0;
    let running = 0;
    while (i < end) {
      let delta;
      [delta, i] = readVarLen(buf, i);
      tick += delta;
      let status = buf[i];
      if (status === 0xff) {
        const type = buf[i + 1];
        let len;
        [len, i] = readVarLen(buf, i + 2);
        if (type === 0x51 && len === 3) tempos.push({ tick, us: (buf[i] << 16) | (buf[i + 1] << 8) | buf[i + 2] });
        i += len;
        continue;
      }
      if (status === 0xf0 || status === 0xf7) {
        let len;
        [len, i] = readVarLen(buf, i + 1);
        i += len;
        continue;
      }
      if (status & 0x80) {
        running = status;
        i += 1;
      } else status = running;
      const kind = status & 0xf0;
      if (kind === 0xc0 || kind === 0xd0) {
        i += 1;
        continue;
      }
      const d1 = buf[i];
      const d2 = buf[i + 1];
      i += 2;
      if (kind === 0x90 && d2 > 0) notes.push({ tick, pitch: d1, velocity: d2 });
    }
    at = end;
  }
  tempos.sort((a, b) => a.tick - b.tick);
  if (!tempos.length || tempos[0].tick !== 0) tempos.unshift({ tick: 0, us: 500000 }); // the MIDI default, 120 BPM
  notes.sort((a, b) => a.tick - b.tick || a.pitch - b.pitch);
  return { ppq: division, tempos, notes };
}

/**
 * Tick to sample, exact: the sum over the tempo segments of ticks * us * rate / (ppq * 1e6),
 * rounded half up to a whole sample. BigInt, so no float error enters.
 */
export function tickToSample(smf, tick, rate) {
  const den = BigInt(smf.ppq) * 1000000n;
  let num = 0n; // numerator over `den`
  for (let i = 0; i < smf.tempos.length; i++) {
    const from = smf.tempos[i].tick;
    if (from >= tick) break;
    const to = i + 1 < smf.tempos.length ? Math.min(smf.tempos[i + 1].tick, tick) : tick;
    num += BigInt(to - from) * BigInt(smf.tempos[i].us) * BigInt(rate);
  }
  return Number((2n * num + den) / (2n * den));
}

/** The longest a tick is, in samples, over the tempos of the file: the bound of the rounding. */
export function samplesPerTick(smf, rate) {
  return Math.max(...smf.tempos.map((t) => (t.us * rate) / (smf.ppq * 1e6)));
}

// ---------------------------------------------------------------- WAV header

/** Reads rate and frames from a RIFF/WAVE (PCM or float) file, and the Broadcast Wave time reference if there is one. */
export function readWavHeader(buf) {
  if (buf.length < 12 || buf.toString('latin1', 0, 4) !== 'RIFF' || buf.toString('latin1', 8, 12) !== 'WAVE') throw new Error('not a WAV file');
  let rate;
  let blockAlign;
  let dataBytes;
  let timeReference = 0;
  let i = 12;
  while (i + 8 <= buf.length) {
    const id = buf.toString('latin1', i, i + 4);
    const size = buf.readUInt32LE(i + 4);
    const body = i + 8;
    if (id === 'fmt ') {
      rate = buf.readUInt32LE(body + 4);
      blockAlign = buf.readUInt16LE(body + 12);
    } else if (id === 'data') {
      dataBytes = Math.min(size, buf.length - body);
    } else if (id === 'bext' && size >= 346) {
      timeReference = buf.readUInt32LE(body + 338) + buf.readUInt32LE(body + 342) * 2 ** 32;
    }
    i = body + size + (size % 2);
  }
  if (!rate || !blockAlign || dataBytes === undefined) throw new Error('WAV file without fmt or data');
  return { rate, frames: Math.floor(dataBytes / blockAlign), timeReference };
}

// ---------------------------------------------------------------- the list

/** The folder with the prepared files: the folder itself, or `<project>/Exports/.prepared`. */
export function preparedDir(path) {
  const root = resolve(path);
  for (const candidate of [root, join(root, 'Exports', '.prepared'), join(root, '.prepared')]) {
    if (existsSync(candidate) && statSync(candidate).isDirectory() && readdirSync(candidate).some((n) => /_spike\.(mid|wav)$/.test(n))) return candidate;
  }
  throw new Error(`No prepared files (*_spike.mid, *_spike.wav) in ${root} or its Exports/.prepared folder.`);
}

/** Makes the list object and the warnings from a folder of prepared files. */
export function makeList(dir, { rate: fallbackRate } = {}) {
  const warnings = [];
  const stems = new Map(); // stem -> { mid, spike, audio }
  for (const name of readdirSync(dir).sort()) {
    let m;
    if ((m = /^(.*)_audio_spike\.wav$/.exec(name))) stems.set(m[1], { ...stems.get(m[1]), audio: name });
    else if ((m = /^(.*)_spike\.wav$/.exec(name))) stems.set(m[1], { ...stems.get(m[1]), spike: name });
    else if ((m = /^(.*)_spike\.mid$/.exec(name))) stems.set(m[1], { ...stems.get(m[1]), mid: name });
  }
  const tracks = [];
  for (const [stem, f] of stems) {
    const files = [];
    let rate;
    for (const [role, name] of [['spike', f.spike], ['audio_spike', f.audio]]) {
      if (!name) continue;
      const head = readWavHeader(readFileSync(join(dir, name)));
      if (head.timeReference !== 0) warnings.push(`${name}: the time reference is ${head.timeReference}, not 0; the Add On places the file at tick 0.`);
      if (rate !== undefined && rate !== head.rate) throw new Error(`${stem}: the files have different rates (${rate} and ${head.rate}).`);
      rate = head.rate;
      files.push({ role, name, frames: head.frames });
    }
    rate ??= fallbackRate;
    const track = { name: stem, sampleRate: rate, hits: [], files };
    if (f.mid) {
      if (!rate) throw new Error(`${stem}: no WAV file to read the rate from; give --rate.`);
      const smf = parseSmf(readFileSync(join(dir, f.mid)));
      track.hits = smf.notes.map((n) => ({ sample: tickToSample(smf, n.tick, rate), velocity: n.velocity, note: n.pitch }));
      const spt = samplesPerTick(smf, rate);
      track.accuracy = {
        route: 'prepared-midi',
        ticksPerQuarter: smf.ppq,
        samplesPerTick: Number(spt.toFixed(6)),
        // A tick of one sample or less gives the sample back exactly. A longer tick leaves at most half a
        // tick; rounding that to a whole sample (half up) gives the worst case in whole samples.
        worstErrorSamples: spt <= 1 ? 0 : Math.floor(spt / 2 + 0.5),
      };
    } else if (!files.length) continue;
    if (!rate) throw new Error(`${stem}: no rate; give --rate.`);
    tracks.push(track);
  }
  if (!tracks.length) throw new Error(`No tracks in ${dir}.`);
  return { list: { schema: 'drumspike-deliver', version: 1, tracks }, warnings };
}

// ---------------------------------------------------------------- command line

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const args = process.argv.slice(2);
  const take = (flag) => {
    const i = args.indexOf(flag);
    return i < 0 ? undefined : args.splice(i, 2)[1];
  };
  const out = take('--out');
  const rate = take('--rate');
  if (args.length !== 1) {
    console.error('usage: node tools/make-list.mjs <project folder | prepared folder> [--out file] [--rate 48000]');
    process.exit(2);
  }
  try {
    const dir = preparedDir(args[0]);
    const { list, warnings } = makeList(dir, { rate: rate ? Number(rate) : undefined });
    const target = out ? resolve(out) : join(dir, 'drumspike-deliver.json');
    writeFileSync(target, `${JSON.stringify(list, null, 1)}\n`);
    for (const w of warnings) console.error(`warning: ${w}`);
    const notes = list.tracks.reduce((n, t) => n + t.hits.length, 0);
    console.log(`${basename(target)}: ${list.tracks.length} tracks, ${notes} notes -> ${target}`);
  } catch (e) {
    console.error(`error: ${e.message}`);
    process.exit(1);
  }
}
