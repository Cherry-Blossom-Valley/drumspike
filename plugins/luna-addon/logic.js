// Drum Spike Deliver: the logic of the Add On, without the DOM, so the tests run it against
// Luna.MockEngine. It is a classic script: it reads the `Luna` global (the SDK build of vendor/) and
// defines `DrumSpikeDeliver`. The accuracy rules are in NOTE.md.
//
// The Add On only CREATES things: tracks named "<name> MIDI" and "<name> spike", one clip on each,
// and source files. It reads the rest of the session (track names, the tempo map) and changes
// nothing else. Its one other write is the edit selection that LUNA's create_note moves; the
// selection is put back (restore).

(() => {
  const LIST_SCHEMA = 'drumspike-deliver';
  const LIST_VERSION = 1;
  /** A MIDI note is 20 ms long: a drum hit, not a held note. Clipped to the next note of its pitch. */
  const NOTE_SECONDS = 0.02;
  /** The song starts at tick 0 of the session. Not measured: see NOTE.md, "not known". */
  const ORIGIN_TICK = 0;
  /** File role -> the suffix of the audio track name. */
  const ROLE_SUFFIX = { spike: ' spike', audio_spike: ' audio spike' };
  const MIDI_SUFFIX = ' MIDI';
  const UNDO_NAME = 'Drum Spike: deliver to tracks';

  /** A refusal: the session was not changed. The text is for the person at the page. */
  class Refused extends Error {
    constructor(message) {
      super(message);
      this.name = 'Refused';
    }
  }

  const isInt = (v) => Number.isInteger(v);

  // ---------------------------------------------------------------- the list file

  /**
   * Reads and checks `drumspike-deliver.json` (schema `drumspike-deliver`, version 1):
   *
   *   { "schema": "drumspike-deliver", "version": 1,
   *     "tracks": [ { "name": "Kick", "sampleRate": 48000,
   *                   "hits": [ { "sample": 48000, "velocity": 100, "note": 36 } ],
   *                   "files": [ { "role": "spike", "name": "Kick_spike.wav", "frames": 480000 } ] } ] }
   *
   * `sample` is the position in song time from the session start, in samples of `sampleRate`.
   * Throws Refused with a plain text for the first fault.
   */
  function parseList(text) {
    let doc;
    try {
      doc = JSON.parse(text);
    } catch (e) {
      throw new Refused(`The list file is not JSON (${e.message}).`);
    }
    if (!doc || typeof doc !== 'object' || doc.schema !== LIST_SCHEMA) {
      throw new Refused(`This is not a Drum Spike list: "schema" must be "${LIST_SCHEMA}".`);
    }
    if (doc.version !== LIST_VERSION) {
      throw new Refused(`The list has version ${doc.version}; this Add On reads version ${LIST_VERSION} only.`);
    }
    if (!Array.isArray(doc.tracks) || doc.tracks.length === 0) throw new Refused('The list has no tracks.');
    const seen = new Set();
    const tracks = doc.tracks.map((t, i) => {
      const where = `track ${i + 1}`;
      if (!t || typeof t.name !== 'string' || t.name.trim() === '') throw new Refused(`The ${where} has no name.`);
      const name = t.name.trim();
      if (seen.has(name.toLowerCase())) throw new Refused(`The name "${name}" is in the list twice.`);
      seen.add(name.toLowerCase());
      if (!isInt(t.sampleRate) || t.sampleRate < 8000 || t.sampleRate > 384000) {
        throw new Refused(`"${name}": sampleRate must be a whole number between 8000 and 384000.`);
      }
      if (!Array.isArray(t.hits)) throw new Refused(`"${name}": hits must be a list.`);
      const hits = t.hits.map((h, j) => {
        if (!h || !isInt(h.sample) || h.sample < 0) throw new Refused(`"${name}", hit ${j + 1}: sample must be a whole number, 0 or more.`);
        if (!isInt(h.velocity) || h.velocity < 1 || h.velocity > 127) throw new Refused(`"${name}", hit ${j + 1}: velocity must be 1 to 127.`);
        if (!isInt(h.note) || h.note < 0 || h.note > 127) throw new Refused(`"${name}", hit ${j + 1}: note must be 0 to 127.`);
        return { sample: h.sample, velocity: h.velocity, note: h.note };
      });
      hits.sort((a, b) => a.sample - b.sample);
      const files = (t.files ?? []).map((f, j) => {
        if (!f || !(f.role in ROLE_SUFFIX)) throw new Refused(`"${name}", file ${j + 1}: role must be "spike" or "audio_spike".`);
        if (typeof f.name !== 'string' || f.name === '') throw new Refused(`"${name}", file ${j + 1}: it has no name.`);
        if (!isInt(f.frames) || f.frames < 1) throw new Refused(`"${name}", file ${j + 1}: frames must be a whole number, 1 or more.`);
        return { role: f.role, name: f.name, frames: f.frames };
      });
      const roles = new Set(files.map((f) => f.role));
      if (roles.size !== files.length) throw new Refused(`"${name}": each role may have one file only.`);
      if (hits.length === 0 && files.length === 0) throw new Refused(`"${name}" has no hits and no files.`);
      return { name, sampleRate: t.sampleRate, hits, files };
    });
    return { schema: LIST_SCHEMA, version: LIST_VERSION, tracks };
  }

  /** The tracks, notes and files that a press will make. */
  function summarize(list) {
    let notes = 0;
    let files = 0;
    let midiTracks = 0;
    let audioTracks = 0;
    for (const t of list.tracks) {
      notes += t.hits.length;
      files += t.files.length;
      if (t.hits.length) midiTracks += 1;
      audioTracks += t.files.length;
    }
    return { songTracks: list.tracks.length, midiTracks, audioTracks, tracks: midiTracks + audioTracks, notes, files };
  }

  /** The names of the session tracks that a press will create. */
  function plannedNames(list) {
    const names = [];
    for (const t of list.tracks) {
      if (t.hits.length) names.push(t.name + MIDI_SUFFIX);
      for (const f of t.files) names.push(t.name + ROLE_SUFFIX[f.role]);
    }
    return names;
  }

  /** Names of tracks and buses that the session has already. Read only. */
  function existingNames(session) {
    const names = [];
    for (const t of session.tracks.items) names.push(t.name.value ?? '');
    for (const b of session.buses.items) names.push(b.name.value ?? '');
    return names;
  }

  /**
   * The names of the plan that the session already has. A second press finds its own tracks here:
   * it REFUSES and changes nothing (decision of the proof; see NOTE.md).
   */
  function conflicts(session, list) {
    const have = new Set(existingNames(session).map((n) => n.toLowerCase()));
    return plannedNames(list).filter((n) => have.has(n.toLowerCase()));
  }

  // ---------------------------------------------------------------- sample <-> tick

  /**
   * A song position in samples as a session tick. The tempo map of the session turns seconds
   * into ticks (timeline.tickAtSeconds, constant tempo between tempo rows). One tick is 1/96e9
   * of a beat, so the rounding to a whole tick is far below one sample (NOTE.md).
   */
  function sampleToTick(timeline, sample, sampleRate) {
    return ORIGIN_TICK + Math.round(timeline.tickAtSeconds(sample / sampleRate));
  }

  /** The inverse, as a real number of samples: what the tempo map says a tick is. */
  function tickToSample(timeline, tick, sampleRate) {
    return timeline.seconds(tick - ORIGIN_TICK) * sampleRate;
  }

  /** The notes of one song track as session ticks. A note ends after 20 ms or at the next note of its pitch. */
  function noteTicks(timeline, track) {
    const rate = track.sampleRate;
    const notes = track.hits.map((h) => ({
      pitch: h.note,
      velocity: h.velocity,
      sample: h.sample,
      startTick: sampleToTick(timeline, h.sample, rate),
      stopTick: sampleToTick(timeline, h.sample + NOTE_SECONDS * rate, rate),
    }));
    const next = new Map();
    for (let i = notes.length - 1; i >= 0; i--) {
      const n = notes[i];
      const later = next.get(n.pitch);
      if (later !== undefined && n.stopTick > later) n.stopTick = later;
      if (n.stopTick <= n.startTick) n.stopTick = n.startTick + 1;
      next.set(n.pitch, n.startTick);
    }
    return notes;
  }

  /** The audio clip of one spike file: tick 0 to the end of the file, source from its first sample. */
  function audioSpan(timeline, file, sampleRate) {
    return {
      startTick: ORIGIN_TICK,
      stopTick: Math.max(ORIGIN_TICK + 1, sampleToTick(timeline, file.frames, sampleRate)),
      sourceStartNs: 0,
      sourceStopNs: Math.round((file.frames / sampleRate) * 1e9),
    };
  }

  /** Waits for the tempo map and gives the timeline. Refuses with a text when it is not there. */
  async function readyTimeline(session) {
    await session.musical.ready();
    const timeline = session.musical.timeline;
    if (!timeline.tempoLoaded) throw new Refused('The tempo map of the session is not loaded, so positions cannot be made. Open a session and try again.');
    return timeline;
  }

  // ---------------------------------------------------------------- the press

  /** What a press will do, as text and numbers, without touching anything. */
  async function preview(session, list, fileNames) {
    await session.tracks.membersReady();
    const have = new Set(fileNames ?? []);
    const missing = [];
    for (const t of list.tracks) for (const f of t.files) if (fileNames && !have.has(f.name)) missing.push(f.name);
    const clash = conflicts(session, list);
    return { ...summarize(list), missing, conflicts: clash };
  }

  async function playlistOf(luna, session, trackId) {
    const r = await luna.raw.get(`${session.path}/tracks/${trackId}`, { propfilter: 'playlist_uid', children: false, commands: false, flatvalue: true });
    const uid = r?.properties?.playlist_uid;
    if (typeof uid !== 'string' || uid === '') throw new Error(`The new track ${trackId} has no playlist.`);
    return uid;
  }

  /**
   * One press. `getBytes(fileName)` gives the bytes of a prepared file (File, Blob or Uint8Array).
   * Steps: check (nothing is changed on a refusal), put the files into the session as sources
   * (not part of the undo step; unused sources stay if the press fails later), then ONE
   * transaction that makes every track and clip. Resolves to what was made.
   */
  async function deliver(luna, session, list, getBytes, hooks = {}) {
    const say = hooks.progress ?? (() => {});
    const timeline = await readyTimeline(session);
    await session.tracks.membersReady();

    const clash = conflicts(session, list);
    if (clash.length) {
      throw new Refused(
        `Nothing was changed. The session has these tracks already: ${clash.join(', ')}. ` +
          'A second press does not make a second copy. To deliver again, undo the first delivery (Edit > Undo) or delete those tracks.',
      );
    }

    // All positions are made before anything is written.
    const plan = list.tracks.map((t) => ({
      track: t,
      notes: noteTicks(timeline, t),
      spans: t.files.map((f) => ({ file: f, ...audioSpan(timeline, f, t.sampleRate) })),
    }));

    const sources = new Map();
    for (const t of list.tracks) {
      for (const f of t.files) {
        say(`Putting ${f.name} into the session...`);
        let bytes;
        try {
          bytes = await getBytes(f.name);
        } catch (e) {
          throw new Refused(`Nothing was changed. The file ${f.name} is missing (${e.message}).`);
        }
        sources.set(`${t.name}\u0000${f.role}`, await session.uploadFile({ bytes, name: f.name }));
      }
    }

    const context = session.context;
    await context.refreshSelection().catch(() => {});
    const saved = context.snapshot();
    const made = { midiTracks: [], audioTracks: [], notes: 0, clips: 0 };
    say('Making the tracks...');
    try {
      await session.transaction(UNDO_NAME, async () => {
        for (const { track, notes, spans } of plan) {
          if (notes.length) {
            const [midi] = await session.newTrack({ name: track.name + MIDI_SUFFIX, trackType: 'instrument', stemFormat: 'stereo', makeNameUnique: false });
            const playlist = await playlistOf(luna, session, midi.id);
            // One create_note call: LUNA makes the MIDI clip (bar line before the first note to the
            // bar line after the last) and writes every note at its session tick (fact 111). A batch
            // transaction does not allow Playlist.createNotes, so the verb is posted directly.
            await luna.raw.post(`${session.path}/contexts/main/commands/create_note`, {
              session_uid: session.uid,
              playlist_uid: playlist,
              notes: notes.map((n) => ({ note: n.pitch, start: n.startTick, stop: n.stopTick, velocity: n.velocity })),
            });
            made.midiTracks.push(track.name + MIDI_SUFFIX);
            made.notes += notes.length;
            made.clips += 1;
          }
          for (const span of spans) {
            const name = track.name + ROLE_SUFFIX[span.file.role];
            const [audio] = await session.newTrack({ name, trackType: 'audio', stemFormat: 'mono', makeNameUnique: false });
            const playlist = await playlistOf(luna, session, audio.id);
            const source = sources.get(`${track.name}\u0000${span.file.role}`);
            await session.insertClip({
              playlistUid: playlist,
              sourceUid: source.id,
              startTick: span.startTick,
              stopTick: span.stopTick,
              sourceStartNs: span.sourceStartNs,
              sourceStopNs: span.sourceStopNs,
              name: span.file.name,
              // The clip keeps its length in time, not in beats: a later tempo change must not stretch the audio.
              lengthFollows: 'time',
            });
            made.audioTracks.push(name);
            made.clips += 1;
          }
        }
      });
    } finally {
      await context.restore(saved).catch(() => {});
    }
    await luna.settled();
    return { ...made, tracks: made.midiTracks.length + made.audioTracks.length };
  }

  // ---------------------------------------------------------------- measure (read only)

  /**
   * Reads back what a press made and says how far it is from the list, in samples. Read only: it
   * posts nothing and writes nothing. The ticks are what LUNA STORED; they are turned into samples
   * with the tempo map of the session, the same model that placed them. So this proves that LUNA
   * kept our ticks (note offsets, clip starts, source offsets). It cannot see how the audio engine
   * rounds a tick to a sample; for that, bounce the tracks and compare (NOTE.md).
   */
  async function measure(luna, session, list) {
    const timeline = await readyTimeline(session);
    await session.tracks.membersReady();
    const byName = new Map(session.tracks.items.map((t) => [(t.name.value ?? '').toLowerCase(), t]));
    const report = { tracks: [], tempoRows: [], worstSamples: 0 };

    for (const row of session.musical.tempoMap.items) {
      report.tempoRows.push({ tick: row.tick.value, tempo: row.tempo.value });
    }
    const raw = await luna.raw.get(`${session.path}/tempo_map`, { children: true, commands: false, flatvalue: true }).catch(() => undefined);
    report.tempoRowsRaw = raw?.children ?? raw ?? null;

    const note = (value) => (Number.isFinite(value) ? value : Number.NaN);
    for (const t of list.tracks) {
      const out = { name: t.name, midi: null, audio: [] };
      const midiTrack = byName.get((t.name + MIDI_SUFFIX).toLowerCase());
      if (t.hits.length && !midiTrack) out.midi = { error: 'track not found' };
      else if (midiTrack) {
        const lane = session.playlistFor(midiTrack);
        await lane.clips.load();
        await lane.clips.membersReady();
        const found = [];
        for (const clip of lane.clips.items) {
          if (clip.clipType.value !== 'midi') continue;
          await clip.midi.ready();
          for (const n of clip.midi.notes.items) found.push({ pitch: n.pitch, tick: clip.start.value + n.startTick, velocity: n.velocity });
        }
        found.sort((a, b) => a.tick - b.tick || a.pitch - b.pitch);
        const want = [...t.hits].sort((a, b) => a.sample - b.sample || a.note - b.note);
        const diffs = [];
        for (let i = 0; i < Math.min(found.length, want.length); i++) {
          const got = tickToSample(timeline, found[i].tick, t.sampleRate);
          diffs.push({ index: i, wantSample: want[i].sample, gotSample: got, diff: note(got - want[i].sample), pitchOk: found[i].pitch === want[i].note, velocityOk: found[i].velocity === want[i].velocity });
        }
        const worst = diffs.reduce((m, d) => Math.max(m, Math.abs(d.diff)), 0);
        out.midi = { wantNotes: want.length, gotNotes: found.length, worstSamples: worst, allPitchAndVelocityOk: diffs.every((d) => d.pitchOk && d.velocityOk), diffs };
        report.worstSamples = Math.max(report.worstSamples, worst);
      }
      for (const f of t.files) {
        const track = byName.get((t.name + ROLE_SUFFIX[f.role]).toLowerCase());
        if (!track) {
          out.audio.push({ file: f.name, error: 'track not found' });
          continue;
        }
        const lane = session.playlistFor(track);
        await lane.clips.load();
        await lane.clips.membersReady();
        const clips = lane.clips.items.filter((c) => c.clipType.value === 'audio');
        const clip = clips[0];
        if (!clip) {
          out.audio.push({ file: f.name, error: 'no audio clip' });
          continue;
        }
        const rate = t.sampleRate;
        const startSample = tickToSample(timeline, clip.start.value, rate);
        const offsetSample = (clip.sourceStart.value * rate) / 1e9;
        const lengthSample = tickToSample(timeline, clip.stop.value, rate) - startSample;
        const lengthWant = ((clip.sourceStop.value - clip.sourceStart.value) * rate) / 1e9;
        const sourcePath = session.sourcePath(clip.sourceUid.value);
        const entry = {
          file: f.name,
          clips: clips.length,
          startSamples: startSample,
          sourceOffsetSamples: offsetSample,
          lengthSamples: lengthSample,
          sourceLengthSamples: lengthWant,
          wantFrames: f.frames,
          lengthDiffSamples: lengthSample - f.frames,
          sourceLengthDiffSamples: lengthWant - f.frames,
          sourcePath,
        };
        out.audio.push(entry);
        report.worstSamples = Math.max(report.worstSamples, Math.abs(startSample), Math.abs(offsetSample), Math.abs(entry.lengthDiffSamples), Math.abs(entry.sourceLengthDiffSamples));
      }
      report.tracks.push(out);
    }
    return report;
  }

  /** The report of measure() as plain text lines. */
  function formatReport(report) {
    const f = (v) => (Number.isFinite(v) ? v.toFixed(6) : String(v));
    const lines = [];
    lines.push(`Tempo rows read: ${report.tempoRows.map((r) => `tick ${r.tick} at ${r.tempo} BPM`).join('; ') || 'none'}`);
    for (const t of report.tracks) {
      if (t.midi?.error) lines.push(`${t.name} MIDI: ${t.midi.error}`);
      else if (t.midi) lines.push(`${t.name} MIDI: ${t.midi.gotNotes} of ${t.midi.wantNotes} notes, worst difference ${f(t.midi.worstSamples)} samples, pitch and velocity ${t.midi.allPitchAndVelocityOk ? 'equal' : 'DIFFER'}`);
      for (const a of t.audio) {
        if (a.error) lines.push(`${t.name} ${a.file}: ${a.error}`);
        else lines.push(`${t.name} ${a.file}: start ${f(a.startSamples)} samples, source offset ${f(a.sourceOffsetSamples)}, length difference ${f(a.lengthDiffSamples)} (clip) and ${f(a.sourceLengthDiffSamples)} (source)`);
      }
    }
    lines.push(`Worst difference of everything read: ${f(report.worstSamples)} samples`);
    return lines.join('\n');
  }

  globalThis.DrumSpikeDeliver = {
    LIST_SCHEMA,
    LIST_VERSION,
    NOTE_SECONDS,
    ORIGIN_TICK,
    UNDO_NAME,
    Refused,
    parseList,
    summarize,
    plannedNames,
    conflicts,
    sampleToTick,
    tickToSample,
    noteTicks,
    audioSpan,
    preview,
    deliver,
    measure,
    formatReport,
  };
})();
