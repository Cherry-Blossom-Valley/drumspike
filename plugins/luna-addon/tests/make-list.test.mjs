// The script that makes the list file from prepared files. The fixtures are written here: a
// standard MIDI file and a WAV header, as the plugin writes them (one tick = one sample where the
// rate and the tempo allow it, else 32767 ticks per quarter note).
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';
import { D } from './helpers.mjs';
import { makeList, parseSmf, preparedDir, readWavHeader, tickToSample } from '../tools/make-list.mjs';

const varLen = (n) => {
  const out = [n & 0x7f];
  while ((n >>= 7)) out.unshift((n & 0x7f) | 0x80);
  return out;
};

/** A format 1 file: a tempo track and a note track. `tempos` are { tick, us }, `notes` { tick, pitch, velocity }. */
function smf({ ppq, tempos, notes }) {
  const track = (events) => {
    const bytes = [];
    let last = 0;
    for (const e of events.sort((a, b) => a.tick - b.tick)) {
      bytes.push(...varLen(e.tick - last), ...e.bytes);
      last = e.tick;
    }
    bytes.push(0, 0xff, 0x2f, 0);
    const head = Buffer.alloc(8);
    head.write('MTrk', 0, 'latin1');
    head.writeUInt32BE(bytes.length, 4);
    return Buffer.concat([head, Buffer.from(bytes)]);
  };
  const header = Buffer.alloc(14);
  header.write('MThd', 0, 'latin1');
  header.writeUInt32BE(6, 4);
  header.writeUInt16BE(1, 8);
  header.writeUInt16BE(2, 10);
  header.writeUInt16BE(ppq, 12);
  const t0 = tempos.map((t) => ({ tick: t.tick, bytes: [0xff, 0x51, 3, (t.us >> 16) & 255, (t.us >> 8) & 255, t.us & 255] }));
  const t1 = notes.flatMap((n) => [
    { tick: n.tick, bytes: [0x90, n.pitch, n.velocity] },
    { tick: n.tick + 10, bytes: [0x80, n.pitch, 0] },
  ]);
  return Buffer.concat([header, track(t0), track(t1)]);
}

/** A mono 32 bit float WAV with `frames` frames, as the plugin caches them. */
function wav(rate, frames, timeReference = 0) {
  const data = Buffer.alloc(frames * 4);
  const fmt = Buffer.alloc(16);
  fmt.writeUInt16LE(3, 0);
  fmt.writeUInt16LE(1, 2);
  fmt.writeUInt32LE(rate, 4);
  fmt.writeUInt32LE(rate * 4, 8);
  fmt.writeUInt16LE(4, 12);
  fmt.writeUInt16LE(32, 14);
  const bext = Buffer.alloc(602);
  bext.writeUInt32LE(timeReference, 338);
  const chunk = (id, body) => {
    const h = Buffer.alloc(8);
    h.write(id, 0, 'latin1');
    h.writeUInt32LE(body.length, 4);
    return Buffer.concat([h, body, body.length % 2 ? Buffer.alloc(1) : Buffer.alloc(0)]);
  };
  const body = Buffer.concat([Buffer.from('WAVE'), chunk('fmt ', fmt), chunk('bext', bext), chunk('data', data)]);
  const riff = Buffer.alloc(8);
  riff.write('RIFF', 0, 'latin1');
  riff.writeUInt32LE(body.length, 4);
  return Buffer.concat([riff, body]);
}

/** The tick the plugin would write for `sample`: nearest tick of the file's grid (one segment of tempo). */
const tickOf = (sample, rate, ppq, us) => Math.round(sample / ((us * rate) / (ppq * 1e6)));

function folder(files) {
  const dir = mkdtempSync(join(tmpdir(), 'luna-addon-'));
  const prepared = join(dir, 'Exports', '.prepared');
  mkdirSync(prepared, { recursive: true });
  for (const [name, content] of Object.entries(files)) writeFileSync(join(prepared, name), content);
  return { dir, prepared };
}

describe('make-list from prepared files', () => {
  it('one tick is one sample: every sample comes back exactly', () => {
    const rate = 48000;
    const ppq = 24000; // 120 BPM: us = 500000, one tick = 1 sample
    const samples = [0, 1, 24000, 96001, 48000 * 600 + 17];
    const { dir, prepared } = folder({
      'Kick_spike.mid': smf({ ppq, tempos: [{ tick: 0, us: 500000 }], notes: samples.map((s, i) => ({ tick: s, pitch: 36, velocity: 20 + i })) }),
      'Kick_spike.wav': wav(rate, 1000),
      'Kick_audio_spike.wav': wav(rate, 1000),
    });
    try {
      assert.equal(preparedDir(dir), prepared);
      const { list, warnings } = makeList(prepared);
      assert.deepEqual(warnings, []);
      const t = list.tracks[0];
      assert.equal(t.name, 'Kick');
      assert.equal(t.sampleRate, 48000);
      assert.deepEqual(t.hits.map((h) => h.sample), samples);
      assert.deepEqual(t.hits.map((h) => h.velocity), [20, 21, 22, 23, 24]);
      assert.deepEqual(t.files, [{ role: 'spike', name: 'Kick_spike.wav', frames: 1000 }, { role: 'audio_spike', name: 'Kick_audio_spike.wav', frames: 1000 }]);
      assert.equal(t.accuracy.worstErrorSamples, 0);
      D.parseList(JSON.stringify(list)); // the Add On reads it
    } finally {
      rmSync(dir, { recursive: true });
    }
  });

  for (const [rate, bpm] of [[44100, 117], [96000, 60], [48000, 133.3], [96000, 250]]) {
    it(`a long tick (${rate} Hz, ${bpm} BPM, 32767 ticks per quarter): within the stated worst case`, () => {
      const ppq = 32767;
      const us = Math.floor(6e7 / bpm);
      const samples = [0, 5, 12345, 3 * rate + 1, 77 * rate + 31, 300 * rate];
      for (let k = 0; k < 400; k++) samples.push(1000 + k * 7919 + (k % 3));
      samples.sort((a, b) => a - b);
      const { dir, prepared } = folder({
        'Snare_spike.mid': smf({ ppq, tempos: [{ tick: 0, us }], notes: samples.map((s) => ({ tick: tickOf(s, rate, ppq, us), pitch: 38, velocity: 99 })) }),
        'Snare_spike.wav': wav(rate, 10),
      });
      try {
        const { list } = makeList(prepared);
        const t = list.tracks[0];
        const bound = t.accuracy.worstErrorSamples;
        let worst = 0;
        t.hits.forEach((h, i) => (worst = Math.max(worst, Math.abs(h.sample - samples[i]))));
        // a tick of one sample or less is exact; a longer tick stays within the stated whole samples
        assert.ok(worst <= bound, `worst ${worst}, stated ${bound}`);
        if (t.accuracy.samplesPerTick <= 1) assert.equal(worst, 0);
      } finally {
        rmSync(dir, { recursive: true });
      }
    });
  }

  it('two tempos in the file: the ticks after the change are placed with the second tempo', () => {
    const rate = 48000;
    const ppq = 32767;
    const tempos = [{ tick: 0, us: 500000 }, { tick: ppq * 8, us: 666666 }]; // 120 BPM, then 90 BPM at beat 8 = 4 s
    const m = parseSmf(smf({ ppq, tempos, notes: [{ tick: ppq * 8, pitch: 36, velocity: 1 }, { tick: ppq * 9, pitch: 36, velocity: 1 }] }));
    assert.equal(tickToSample(m, ppq * 8, rate), 4 * 48000);
    // one more beat at 666666 us = 0.666666 s
    assert.equal(tickToSample(m, ppq * 9, rate), 4 * 48000 + Math.round(0.666666 * 48000));
  });

  it('warns about a time reference that is not 0 and names what is missing', () => {
    const { dir, prepared } = folder({ 'OH_spike.wav': wav(44100, 10, 7) });
    try {
      const { list, warnings } = makeList(prepared);
      assert.equal(warnings.length, 1);
      assert.match(warnings[0], /time reference is 7/);
      assert.deepEqual(list.tracks[0].hits, []);
      assert.equal(readWavHeader(readFileSync(join(prepared, 'OH_spike.wav'))).frames, 10);
    } finally {
      rmSync(dir, { recursive: true });
    }
    const empty = folder({ 'readme.txt': 'x' });
    try {
      assert.throws(() => preparedDir(empty.dir), /No prepared files/);
    } finally {
      rmSync(empty.dir, { recursive: true });
    }
  });

  it('the command line writes the file next to the prepared files', () => {
    const { dir, prepared } = folder({
      'Tom_spike.mid': smf({ ppq: 24000, tempos: [{ tick: 0, us: 500000 }], notes: [{ tick: 48000, pitch: 45, velocity: 90 }] }),
      'Tom_spike.wav': wav(48000, 100),
    });
    try {
      const script = fileURLToPath(new URL('../tools/make-list.mjs', import.meta.url));
      const out = execFileSync(process.execPath, [script, dir], { encoding: 'utf8' });
      assert.match(out, /1 tracks, 1 notes/);
      const list = D.parseList(readFileSync(join(prepared, 'drumspike-deliver.json'), 'utf8'));
      assert.equal(list.tracks[0].hits[0].sample, 48000);
    } finally {
      rmSync(dir, { recursive: true });
    }
  });
});
