# Drum Spike Deliver: proof of a LUNA Add On (#600)

One page, one button, "Deliver to tracks". It takes the hits and the spike files that the Drum
Spike plugin prepares, and makes for each drum track a MIDI track "<name> MIDI" and an audio track
"<name> spike" (and "<name> audio spike" when that file exists), as ONE undo step. It adds tracks
and clips only. A second press refuses (decision below). It changes no part of the plugin or the
standalone app. Status: proof, tested on `Luna.MockEngine` only; nothing was run against LUNA.

## Files

| File | What it is |
|---|---|
| `manifest.json`, `index.html`, `main.js` | the page (DOM only; no decision) |
| `logic.js` | the work: list check, tick maths, press, measure; tested |
| `tools/make-list.mjs` | prepared files of a plugin project to `drumspike-deliver.json` |
| `tools/measure.mjs` | read-only read-back from a running LUNA |
| `tests/` | `npm test` (Node 22, no install): 47 tests on the mock engine |
| `vendor/` | the place of the SDK build, fetched at setup (see "The SDK build") |

## The list file `drumspike-deliver.json` (schema `drumspike-deliver`, version 1)

Per drum track: `name`, `sampleRate`, `hits` (`sample` in song time from the session start,
`velocity` 1-127, `note` 0-127) and `files` (`role` `spike` or `audio_spike`, `name`, `frames`).
`tools/make-list.mjs <project folder>` writes it into `Exports/.prepared/` from `<track>_spike.mid`,
`<track>_spike.wav` and `<track>_audio_spike.wav`. Route: the MIDI ticks become samples with the
tempo events of the file, in whole number arithmetic. The file holds a tick, not a sample. One tick
is `samplesPerTick` samples (written to the list as `accuracy`). If that is 1 or less (the plugin
picks one tick = one sample where the rate and the tempo allow it) the sample comes back exactly.
Else the position is within half a tick, rounded to whole samples (`floor(samplesPerTick / 2 + 0.5)`,
for example 1 sample for a tick of more than 1 and less than 3 samples; tested at 44.1, 48 and 96 kHz,
60 to 250 BPM, 32767 ticks per quarter note). The better route (the events of the project, sample exact) needs the row and clip
mapping of the plugin; it is a plugin ticket, not done here. The press itself has no tick grid
of this kind: it reads samples.

## How a sample position becomes a tick

`seconds = sample / sampleRate`, then `session.musical.timeline.tickAtSeconds(seconds)` (the SDK walks
the tempo map of the session, constant tempo between tempo rows), rounded to a whole tick. Tick 0 is
the session start. LUNA ticks are 1/96e9 of a beat. Notes are 20 ms long (clipped to the next note
of the same pitch). MIDI clips are made by LUNA's `create_note` (one call per track, posted inside
the transaction because `Playlist.createNotes` is refused in a batch). The audio clip is
`insertClip` from tick 0 to the tick of the file end, source offset 0, `lengthFollows: 'time'`.

## Worst-case error, and why

- Model (proved by the tests): one tick is `rate * 60 / (bpm * 96e9)` samples, so half a tick is at
  most 1e-7 samples at 300 BPM and 96 kHz, 5e-8 at 48 kHz, 4.6e-8 at 44.1 kHz; at 45 BPM and 96 kHz
  it is 6.7e-7 (it grows as the tempo falls: 1.5e-6 at 20 BPM, 96 kHz). The double arithmetic of the
  tempo walk adds less than one more tick, also over an hour of song. Tests: the tick of about 3000
  positions at 44.1, 48 and 96 kHz and at five tempo maps (one tempo at 60, 120 and 300 BPM, two
  tempos, three tempos from 45 to 300 BPM) is within 1 tick of the exact rational value, and the
  sample read back is within 1e-6 samples and rounds to the same sample.
- Notes: this model error, so below 1e-6 samples at 44.1, 48 and 96 kHz from 45 BPM up, also across tempo changes
  (the walk is exact at the change). The audible error is then set by the engine (next section).
- Audio clip: 0 samples by construction. It starts at tick 0, which is time 0 for every tempo map,
  and plays the source from nanosecond 0. Its end (clip stop) is made from the frame count and
  differs from it by less than 1e-6 samples at the same rates.

## What happens to placed clips when the tempo map changes later

- Notes sit on ticks, so they stay on their beats and MOVE in time. On the mock, a change from 120
  to 100 BPM moves later notes by thousands of samples; the first note (tick 0) stays. After a tempo
  change the delivered MIDI is no longer where the plugin hits are. Undo and deliver again.
- The audio clip starts at tick 0 and keeps its start. We ask for `lengthFollows: 'time'` so that
  LUNA keeps its length in seconds; that LUNA does so is not measured. The mock does not model it.
- So MIDI and spike audio drift apart as soon as the tempo map is edited. This is a property of
  tick based placement, not a defect of the Add On.

## Second press: refuse

It REFUSES and changes nothing, with a text that names the tracks and says "undo the first delivery
(Edit > Undo) or delete those tracks". Reason: replacing would delete tracks, and the rule of this
Add On is that it never deletes or changes anything. The test is by name, without case, so a user
track with the same name also stops a press. Uploaded files are not part of the undo step; an
unused source stays in the session if a press fails after the upload.

## Measure (read only)

`Measure` in the page, or `node tools/measure.mjs drumspike-deliver.json`. It reads the new tracks
back and prints, per track, the notes found against the list (difference in samples), pitch and
velocity equality, the audio clip start, source offset and length, and the tempo rows. It posts
nothing. It proves that LUNA kept the ticks that we sent (note offsets inside the clip, clip start,
source offset). It does not see how the audio engine rounds a tick to a sample. Run it on a scratch
session copy only. Real session steps:

1. LUNA is running with a scratch copy of a session open (never a working session).
2. `node tools/make-list.mjs <plugin project folder>` writes `Exports/.prepared/drumspike-deliver.json`.
3. Serve the folder `plugins/luna-addon` to a browser on the same computer, for example
   `uv run --no-project python -m http.server 8790 --directory plugins/luna-addon`, and open
   `http://localhost:8790/`. A browser reaches LUNA on `localhost:4718`; no Add On is installed. The
   Add On can also be installed from `manifest.json` in the usual way; that was not done.
4. Choose `drumspike-deliver.json` and the `.wav` files from the hidden folder `Exports/.prepared`
   (in the file dialog: Cmd+Shift+. shows hidden folders), press "Deliver to tracks", then "Measure".
5. For the audible truth, bounce the new tracks (or the click and a spike) and compare in a wave
   editor: this is the only check of the engine rounding.

## What is not known without a real DAW

- How LUNA turns a tick into an engine sample (rounding, an extra step), and the engine rate when
  it differs from the file rate: then LUNA plays a resampled render and sample accuracy is lost.
- Whether tempo rows are steps (as the SDK assumes) or ramps, and the `time` column of a tempo row.
- Whether tick 0 is the session start for a session with a start offset or a count-in.
- Whether `create_note` takes thousands of notes in one post, and whether raw reads inside a batch
  transaction (the playlist of a track made in the same batch) work as on the mock.
- Whether track type `instrument` is the right track for MIDI without an instrument plugin, whether
  `uploadFile` of a float WAV keeps rate and length, and that LUNA honours `lengthFollows`.
- Whether a browser page may connect to LUNA, and what LUNA shows the user (an Add On permission).
- The page was shown on the mock engine in a browser up to the plan; the button press itself is
  covered by the tests of `logic.js`, not by the page.

## The SDK build

`vendor/luna.global.js` is the script-tag build of `@uaudio/luna-builder-sdk` 3.1.2 (tested with commit 62b8d38 of `uaudio-labs/luna-builder-sdk`). It is NOT in this repository: `npm run fetch-sdk -- <path to luna-builder-sdk>` copies it from a local copy of the vendor's repository, and git ignores it. The build is under the Apache License 2.0 plus the vendor's "Supplemental Terms for LUNA Builder SDK" (rights in feedback, an indemnity, rules for outputs); read them in the vendor's repository before use. There is no `node_modules`.

