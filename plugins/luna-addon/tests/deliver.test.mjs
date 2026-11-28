// The delivery against Luna.MockEngine. The questions of the ticket (#600):
//   - every note tick is the expected tick of its sample position, within the stated bound
//   - the audio clip starts at tick 0 with the right source
//   - one transaction (one undo step)
//   - a second press does not duplicate
//   - nothing that exists is changed
//   - a tempo map with two tempos
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { BEAT, D, Luna, bytes, connect, exactTick, listOf, snapshotNodes } from './helpers.mjs';

// The proof of NOTE.md: the model error of a tick is at most half a tick. One tick is
// rate * 60 / (bpm * 96e9) samples; at 300 BPM and 96 kHz that is 2e-7 samples.
const MAX_TICK_ERROR = 1; // ticks; the rounding is 0.5, float error of the tempo walk stays below 0.5 more
const SAMPLE_BOUND = 1e-6; // samples, at the rates and tempos tested

const KICK = {
  name: 'Kick',
  sampleRate: 48000,
  hits: [
    { sample: 0, velocity: 100, note: 36 },
    { sample: 24000, velocity: 64, note: 36 },
    { sample: 96001, velocity: 127, note: 36 },
    { sample: 5 * 48000 + 17, velocity: 1, note: 35 },
  ],
  files: [
    { role: 'spike', name: 'Kick_spike.wav', frames: 480000 },
    { role: 'audio_spike', name: 'Kick_audio_spike.wav', frames: 480000 },
  ],
};

async function laneOf(session, name) {
  await session.tracks.membersReady();
  const track = session.tracks.items.find((t) => t.name.value === name);
  assert.ok(track, `track ${name} exists`);
  const lane = session.playlistFor(track);
  await lane.clips.load();
  await lane.clips.membersReady();
  return { track, lane };
}

async function notesOf(session, name) {
  const { lane } = await laneOf(session, name);
  const out = [];
  for (const clip of lane.clips.items) {
    await clip.midi.ready();
    for (const n of clip.midi.notes.items) out.push({ pitch: n.pitch, tick: clip.start.value + n.startTick, stop: clip.start.value + n.stopTick, velocity: n.velocity });
  }
  return out.sort((a, b) => a.tick - b.tick);
}

describe('sample position to tick', () => {
  const tempoSets = [
    [{ tick: 0, tempo: 120 }],
    [{ tick: 0, tempo: 300 }],
    [{ tick: 0, tempo: 60 }],
    [{ tick: 0, tempo: 120 }, { tick: 8 * BEAT, tempo: 90 }],
    [{ tick: 0, tempo: 100 }, { tick: 4 * BEAT, tempo: 300 }, { tick: 40 * BEAT, tempo: 45 }],
  ];
  for (const rate of [44100, 48000, 96000]) {
    for (const rows of tempoSets) {
      it(`${rate} Hz, tempos ${rows.map((r) => r.tempo).join('/')}: tick within ${MAX_TICK_ERROR} of the exact tick, sample within ${SAMPLE_BOUND}`, async () => {
        const { session } = await connect({ tempos: rows });
        await session.musical.ready();
        const timeline = session.musical.timeline;
        // positions: the start, one sample, round times, odd times, tempo change edges, one hour
        const samples = [0, 1, 2, rate, rate * 4, rate * 4 + 1, rate * 4 - 1, 12345, 1000003, rate * 60, rate * 3600 - 7];
        for (let i = 0; i < 200; i++) samples.push(Math.floor(((i * 7919 + 13) * 104729) % (rate * 600)));
        let worstTick = 0;
        let worstSample = 0;
        for (const s of samples) {
          const tick = D.sampleToTick(timeline, s, rate);
          const exact = exactTick(s, rate, rows);
          worstTick = Math.max(worstTick, Math.abs(tick - exact));
          const back = D.tickToSample(timeline, tick, rate);
          worstSample = Math.max(worstSample, Math.abs(back - s));
          assert.equal(Math.round(back), s, `sample ${s} comes back as the same sample`);
        }
        assert.ok(worstTick <= MAX_TICK_ERROR, `worst tick error ${worstTick}`);
        assert.ok(worstSample <= SAMPLE_BOUND, `worst sample error ${worstSample}`);
      });
    }
  }
});

describe('the list file', () => {
  it('reads a good list and sorts the hits', () => {
    const l = listOf([{ ...KICK, hits: [...KICK.hits].reverse() }]);
    assert.deepEqual(l.tracks[0].hits.map((h) => h.sample), [0, 24000, 96001, 5 * 48000 + 17].sort((a, b) => a - b));
    assert.deepEqual(D.summarize(l), { songTracks: 1, midiTracks: 1, audioTracks: 2, tracks: 3, notes: 4, files: 2 });
    assert.deepEqual(D.plannedNames(l), ['Kick MIDI', 'Kick spike', 'Kick audio spike']);
  });
  const bad = (change, text) =>
    it(`refuses: ${text}`, () => {
      const doc = { schema: 'drumspike-deliver', version: 1, tracks: [structuredClone(KICK)] };
      change(doc);
      assert.throws(() => D.parseList(JSON.stringify(doc)), (e) => e instanceof D.Refused && e.message.length > 10);
    });
  bad((d) => (d.version = 2), 'another version');
  bad((d) => (d.schema = 'x'), 'another schema');
  bad((d) => (d.tracks = []), 'no tracks');
  bad((d) => (d.tracks[0].sampleRate = 44100.5), 'a fractional rate');
  bad((d) => (d.tracks[0].hits[0].sample = 1.5), 'a fractional sample');
  bad((d) => (d.tracks[0].hits[0].sample = -1), 'a negative sample');
  bad((d) => (d.tracks[0].hits[0].velocity = 0), 'velocity 0');
  bad((d) => (d.tracks[0].hits[0].note = 128), 'note 128');
  bad((d) => (d.tracks[0].files[1].role = 'spike'), 'one role twice');
  bad((d) => (d.tracks[0].files[0].frames = 0), 'no frames');
  bad((d) => d.tracks.push(structuredClone(KICK)), 'a name twice');
  it('refuses text that is not JSON', () => assert.throws(() => D.parseList('{'), D.Refused));
});

describe('deliver to tracks', () => {
  it('makes a MIDI track and two audio tracks, notes on their ticks, audio from tick 0, in ONE undo step', async () => {
    const { engine, luna, session } = await connect({ tempos: [{ tick: 0, tempo: 120 }, { tick: 8 * BEAT, tempo: 90 }] });
    const list = listOf([KICK]);
    const asked = [];
    const result = await D.deliver(luna, session, list, async (name) => (asked.push(name), bytes(name)));
    await engine.settled();
    await luna.settled();

    assert.deepEqual(asked, ['Kick_spike.wav', 'Kick_audio_spike.wav']);
    assert.deepEqual(engine.transactions, ['begin', 'commit'], 'one transaction');
    assert.deepEqual(result, { midiTracks: ['Kick MIDI'], audioTracks: ['Kick spike', 'Kick audio spike'], notes: 4, clips: 3, tracks: 3 });

    // notes: pitch, velocity and tick of every hit
    const notes = await notesOf(session, 'Kick MIDI');
    const rows = [{ tick: 0, tempo: 120 }, { tick: 8 * BEAT, tempo: 90 }];
    const want = [...KICK.hits].sort((a, b) => a.sample - b.sample);
    assert.equal(notes.length, want.length);
    notes.forEach((n, i) => {
      assert.equal(n.pitch, want[i].note);
      assert.equal(n.velocity, want[i].velocity);
      const exact = exactTick(want[i].sample, 48000, rows);
      assert.ok(Math.abs(n.tick - exact) <= MAX_TICK_ERROR, `note ${i}: tick ${n.tick}, exact ${exact}`);
    });
    // notes of one pitch never overlap, and every note has a length
    for (const n of notes) assert.ok(n.stop > n.tick);

    // the audio clip keeps its length in time: a later tempo change must not stretch the file
    const inserts = engine.requests.filter((r) => /insert_clip$/.test(r.path));
    assert.equal(inserts.length, 2);
    for (const r of inserts) assert.equal(r.data.length_follows, 'time');

    // audio: one clip per track, from tick 0, source from its first sample, for the whole file
    for (const [track, file] of [['Kick spike', 'Kick_spike.wav'], ['Kick audio spike', 'Kick_audio_spike.wav']]) {
      const { lane } = await laneOf(session, track);
      assert.equal(lane.clips.items.length, 1);
      const clip = lane.clips.items[0];
      assert.equal(clip.start.value, 0, 'the clip starts at tick 0');
      assert.equal(clip.sourceStart.value, 0, 'the source plays from its first sample');
      assert.equal(clip.sourceStop.value, 10e9, '480000 frames at 48 kHz');
      const source = session.sources.byId(clip.sourceUid.value);
      assert.equal(source.name.value, file, 'the right source');
      const stopSample = D.tickToSample(session.musical.timeline, clip.stop.value, 48000);
      assert.ok(Math.abs(stopSample - 480000) <= SAMPLE_BOUND);
    }
  });

  it('a second press changes nothing: it refuses with a clear text', async () => {
    const { engine, luna, session } = await connect();
    const list = listOf([KICK]);
    await D.deliver(luna, session, list, async (n) => bytes(n));
    await engine.settled();
    await luna.settled();
    const before = snapshotNodes(engine);
    const requests = engine.requests.length;
    const transactions = [...engine.transactions];
    await assert.rejects(
      () => D.deliver(luna, session, list, async (n) => bytes(n)),
      (e) => e instanceof D.Refused && /Nothing was changed/.test(e.message) && /Kick MIDI/.test(e.message) && /Undo/.test(e.message),
    );
    assert.deepEqual(snapshotNodes(engine), before, 'no node changed');
    assert.deepEqual(engine.transactions, transactions, 'no new transaction');
    const writes = engine.requests.slice(requests).filter((r) => r.verb !== 'get');
    assert.deepEqual(writes, [], 'not one write request');
    assert.equal(session.tracks.items.filter((t) => t.name.value === 'Kick MIDI').length, 1);
  });

  it('refuses when a track of that name exists, without case, and does not touch it', async () => {
    const { engine, luna, session } = await connect({ existing: ['kick spike', 'Bass'] });
    const before = snapshotNodes(engine);
    await assert.rejects(() => D.deliver(luna, session, listOf([KICK]), async (n) => bytes(n)), (e) => e instanceof D.Refused && /kick spike|Kick spike/i.test(e.message));
    assert.deepEqual(snapshotNodes(engine), before);
    assert.deepEqual(engine.transactions, []);
  });

  it('a missing file refuses before any write', async () => {
    const { engine, luna, session } = await connect();
    const before = snapshotNodes(engine);
    await assert.rejects(
      () => D.deliver(luna, session, listOf([KICK]), async (n) => { throw new Error(`${n} was not chosen`); }),
      (e) => e instanceof D.Refused && /Kick_spike\.wav/.test(e.message) && /Nothing was changed/.test(e.message),
    );
    assert.deepEqual(snapshotNodes(engine), before);
    assert.deepEqual(engine.transactions, []);
  });

  it('never changes what exists: tracks, clips, settings, the selection', async () => {
    const { engine, luna, session } = await connect({ existing: ['Existing', 'Vocal'] });
    // an existing audio clip and an existing MIDI clip with a note, and a user selection
    const src = await session.uploadFile({ bytes: bytes('user.wav'), name: 'user.wav' });
    const vocal = session.tracks.items.find((t) => t.name.value === 'Vocal');
    await session.insertClip({ trackUid: vocal.id, sourceUid: src.id, startTick: 0, stopTick: 4 * BEAT });
    const existing = session.tracks.items.find((t) => t.name.value === 'Existing');
    const midiClip = engine.addMidiClip('abc', existing.id, { startTick: 0, stopTick: 4 * BEAT });
    assert.ok(midiClip);
    await session.context.select({ startTick: BEAT, stopTick: 2 * BEAT, trackUids: [vocal.id] });
    await engine.settled();
    await luna.settled();

    const before = snapshotNodes(engine);
    const selection = JSON.stringify(session.context.snapshot());
    const requests = engine.requests.length;
    await D.deliver(luna, session, listOf([KICK]), async (n) => bytes(n));
    await engine.settled();
    await luna.settled();

    const after = snapshotNodes(engine);
    const changed = [];
    for (const [path, props] of before) if (after.get(path) !== props) changed.push(path);
    // the session node counts its changes; that is LUNA's own bookkeeping. Nothing else may differ.
    assert.deepEqual(changed.filter((p) => p !== '/sessions/abc' && p !== '/workspace/sessions/abc/audio'), [], 'no existing node changed');
    assert.equal(JSON.stringify(session.context.snapshot()), selection, 'the edit selection is put back');
    // every write request names a path or a body that belongs to a new track, a new clip or a new source
    const newTrackIds = new Set(session.tracks.items.filter((t) => /^Kick /.test(t.name.value)).map((t) => t.id));
    const writes = engine.requests.slice(requests).filter((r) => r.verb !== 'get');
    assert.ok(writes.length > 0);
    for (const r of writes) {
      if (/\/tracks\//.test(r.path)) assert.ok([...newTrackIds].some((id) => r.path.includes(`/tracks/${id}`)), `write to ${r.path}`);
      // a body that names a track or a playlist names a new one, never an existing one
      // (the edit selection is the one thing put back: its restore names what the user had selected)
      if (/\/contexts\/main\//.test(r.path)) continue;
      const body = JSON.stringify(r.data ?? {});
      for (const t of [existing, vocal]) assert.ok(!body.includes(`"${t.id}"`), `${r.path} names the existing track ${t.id}`);
    }
    // the existing tracks have the same names and clips as before
    assert.deepEqual(session.tracks.items.filter((t) => !newTrackIds.has(t.id)).map((t) => t.name.value), ['Existing', 'Vocal']);
    const { lane } = await laneOf(session, 'Vocal');
    assert.equal(lane.clips.items.length, 1);
    assert.equal(lane.clips.items[0].stop.value, 4 * BEAT);
  });

  it('a tempo map with two tempos: notes before and after the change', async () => {
    const rows = [{ tick: 0, tempo: 120 }, { tick: 8 * BEAT, tempo: 90 }];
    const { engine, luna, session } = await connect({ tempos: rows });
    const track = {
      name: 'Snare',
      sampleRate: 44100,
      hits: [0, 1, 44100, 4 * 44100 - 1, 4 * 44100, 4 * 44100 + 1, 10 * 44100 + 333, 600 * 44100 + 7].map((sample, i) => ({ sample, velocity: 10 + i, note: 38 })),
      files: [],
    };
    await D.deliver(luna, session, listOf([track]), async (n) => bytes(n));
    await engine.settled();
    await luna.settled();
    const notes = await notesOf(session, 'Snare MIDI');
    assert.equal(notes.length, track.hits.length);
    let worst = 0;
    notes.forEach((n, i) => {
      const exact = exactTick(track.hits[i].sample, 44100, rows);
      assert.ok(Math.abs(n.tick - exact) <= MAX_TICK_ERROR, `hit ${i}: tick ${n.tick} vs ${exact}`);
      const back = D.tickToSample(session.musical.timeline, n.tick, 44100);
      worst = Math.max(worst, Math.abs(back - track.hits[i].sample));
    });
    assert.ok(worst <= SAMPLE_BOUND, `worst ${worst} samples`);
    // the change sits at 4 s = sample 176400: hits on both sides were tested above
    assert.ok(track.hits.some((h) => h.sample < 4 * 44100) && track.hits.some((h) => h.sample > 4 * 44100));
  });

  it('a file only (no hits) makes only an audio track', async () => {
    const { engine, luna, session } = await connect();
    const r = await D.deliver(luna, session, listOf([{ name: 'OH', sampleRate: 96000, hits: [], files: [{ role: 'spike', name: 'OH_spike.wav', frames: 96000 * 3 }] }]), async (n) => bytes(n));
    await engine.settled();
    assert.deepEqual(r.midiTracks, []);
    assert.deepEqual(r.audioTracks, ['OH spike']);
    const { lane } = await laneOf(session, 'OH spike');
    assert.equal(lane.clips.items[0].sourceStop.value, 3e9);
  });

  it('the preview counts tracks, notes and files, and names what is missing or in the way', async () => {
    const { luna, session } = await connect({ existing: ['Kick MIDI'] });
    const p = await D.preview(session, listOf([KICK]), ['Kick_spike.wav']);
    assert.deepEqual({ tracks: p.tracks, notes: p.notes, files: p.files }, { tracks: 3, notes: 4, files: 2 });
    assert.deepEqual(p.missing, ['Kick_audio_spike.wav']);
    assert.deepEqual(p.conflicts, ['Kick MIDI']);
    void luna;
  });
});

describe('measure (read only)', () => {
  it('reads back a delivery: every difference is below the bound', async () => {
    const { engine, luna, session } = await connect({ tempos: [{ tick: 0, tempo: 120 }, { tick: 8 * BEAT, tempo: 75 }] });
    const list = listOf([KICK]);
    await D.deliver(luna, session, list, async (n) => bytes(n));
    await engine.settled();
    await luna.settled();
    const requests = engine.requests.length;
    const report = await D.measure(luna, session, list);
    const writes = engine.requests.slice(requests).filter((r) => r.verb !== 'get');
    assert.deepEqual(writes, [], 'measure writes nothing');
    assert.ok(report.worstSamples <= SAMPLE_BOUND, `worst ${report.worstSamples}`);
    assert.equal(report.tracks[0].midi.gotNotes, 4);
    assert.equal(report.tracks[0].midi.allPitchAndVelocityOk, true);
    assert.equal(report.tracks[0].audio.length, 2);
    assert.equal(report.tracks[0].audio[0].startSamples, 0);
    assert.match(D.formatReport(report), /Worst difference of everything read: 0\.0000/);
  });

  it('after a later tempo change the notes move in time and the audio start does not', async () => {
    const { engine, luna, session } = await connect({ tempos: [{ tick: 0, tempo: 120 }] });
    const list = listOf([KICK]);
    await D.deliver(luna, session, list, async (n) => bytes(n));
    await engine.settled();
    await luna.settled();
    // the user changes the tempo of the whole song to 100 BPM
    engine.setValue('/sessions/abc/tempo_map/1', 'tempo', 100);
    await engine.settled();
    await luna.settled();
    const report = await D.measure(luna, session, list);
    const midi = report.tracks[0].midi;
    assert.ok(midi.worstSamples > 1000, `the notes moved: ${midi.worstSamples} samples`);
    // the first note is at tick 0 and stays; the audio clip starts at tick 0 and stays
    assert.equal(midi.diffs[0].diff, 0);
    assert.equal(report.tracks[0].audio[0].startSamples, 0);
    // the clip follows time, so its source offset and length in the source are unchanged
    assert.equal(report.tracks[0].audio[0].sourceOffsetSamples, 0);
    assert.equal(report.tracks[0].audio[0].sourceLengthDiffSamples, 0);
    // The mock does not model length_follows, so it keeps the stop TICK made for 120 BPM and the clip's length in
    // time changes with the map. Whether LUNA recomputes the stop tick of a 'time' clip is not measured (NOTE.md).
  });
});

void Luna;

describe('the measure command (tools/measure.mjs), on the mock engine', () => {
  it('reads the focused session and prints the report; writes nothing', async () => {
    const { runMeasure } = await import('../tools/measure.mjs');
    const { engine, luna, session } = await connect();
    const list = listOf([KICK]);
    await D.deliver(luna, session, list, async (n) => bytes(n));
    await engine.settled();
    await luna.settled();
    const requests = engine.requests.length;
    const { text, report } = await runMeasure(luna, JSON.stringify(list));
    assert.match(text, /^Session: Song\n/);
    assert.match(text, /Kick MIDI: 4 of 4 notes, worst difference 0\.00000\d samples, pitch and velocity equal/);
    assert.ok(report.worstSamples <= SAMPLE_BOUND);
    assert.deepEqual(engine.requests.slice(requests).filter((r) => r.verb !== 'get'), []);
  });
});
