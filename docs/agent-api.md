# Drum Spike agent API

An agent or a script can use Drum Spike without the window: open or import a project, analyse, set detection and velocity
parameters, edit hits, make a track follow another, and export. Two front ends share one list of commands: the command-line
tool `drumspike` and the MCP server `drumspike mcp` (stdio). There is no network listener. The list of commands is
`crates/app/src/api.rs` (the registry); `drumspike help` and the MCP `tools/list` print it.

## Set up (once)

The tool is the app's own binary. Make the name `drumspike` with one link (the owner or the user does this; the app bundle
does not change):

```bash
ln -s "/Applications/Drum Spike.app/Contents/MacOS/drumspike-desktop" /usr/local/bin/drumspike
```

Without the link use the binary itself: `"/Applications/Drum Spike.app/Contents/MacOS/drumspike-desktop" cli <command> ...` and
`... drumspike-desktop mcp`.

## Claude Code

```bash
claude mcp add drumspike -- drumspike mcp
```

Then ask: "Open ~/Music/Sessions/Song, analyse it, fix the kick, save, and export all." The server lists one tool per command.
Typical order of the calls: `project_open` (or `project_import`, `project_new` and `tracks_add`), `analyze_start` and
`analyze_status` until `state` is `done`, `track_params_set` and `track_velocity_set` (it recomputes every velocity and stores
them with the cut, so call it before single-hit edits), `events_get`, `events_edit`, `project_save`, `export_all` and `job_get`
until `done`.
`project_clean_check {sid}` clears the dirty flag when the open document equals the one saved last (the window calls it after an undo back to the saved state) and answers `{dirty}`; it changes nothing else.
`tracks_remove {sid, names}` removes tracks from the project (their hits, Learn examples, layer and the `Analysis/<name>/` folder; the audio
files stay; a track that followed a removed track detects on its own); `project_save` keeps the result. With `snapshot: true` the answer
also holds the removed records, which the window's undo gives back to `tracks_restore`.

## Command line

```bash
drumspike help                                  # the commands
drumspike help events_edit                      # one command: arguments, result schema, what it changes
drumspike analyze_start --project "~/Song"      # open, analyse (progress on stderr), save, close
drumspike track_velocity_set --project ~/Song --name "k in" --lo_db -45 --hi_db -8 --cut_db -60   # stores the new velocities
drumspike events_get --project ~/Song --name "k in" --pretty
drumspike events_edit --project ~/Song --name "k in" \
  --ops '[{"op":"add","t":1.25},{"op":"move","index":3,"t":2.5},{"op":"delete","index":7},{"op":"velocity","index":2,"v":120}]'
drumspike export_all --project ~/Song
```

- One command is one process. `--project <path>` stands for `--sid`: the process opens the project (for writing when the
  command changes it, else read-only), runs the command, saves when the command changed the project (`--no-save` skips the
  save) and closes. A command that starts a job (`analyze_start`, `peaks_build_start`) waits for it.
- Flags take the argument names of `drumspike help <command>` (`--lo_db` and `--lo-db` are the same). A list or object is JSON.
  `--args '<json>'` (or `--args -` from stdin) gives all arguments as one object.
- `analyze_start` takes `--detector <id>`: `builtin` or a missing flag selects the built-in DSP detector, otherwise the
  detector plug-in with that id (guide chapter 3b; a track's own `params.detector` wins over the flag). The first use of a
  plug-in on the agent surface records its licence in the project, the same record the app's licence dialog writes. The MCP
  `analyze_start` schema names the same `detector` argument.
- Output is one JSON value on stdout (`--pretty` indents it). Progress is JSON lines on stderr. An error is
  `{"error": {"code", "message"}}` on stderr. Exit codes: 0 ok, 1 another error or a failed analysis track, 2 bad usage or
  argument, 3 not found (an unknown command, project or track), 4 conflict or locked.
- Several steps in one process: `drumspike batch` reads one `{"command", "args", "wait"?}` per line from stdin and answers a
  line each; `drumspike run script.json` runs a list of steps and stops at the first error. `"$sid"` in an argument is the
  sid of the last answer that had one. Nothing is saved unless a step is `project_save`.

## Track patterns, session folders and the example job

- A Pro Tools session folder is an input of its own: `tracks_scan` and `tracks_add` accept a folder that holds no audio of its
  own but has an `Audio Files/` subfolder and use the WAV files of that subfolder. The `.ptx` stays ignored (no PT
  integration; the track names and roles come from the file names, or from the `roles` argument).
- Name patterns stand for track names on the agent surface. `tracks_add {tracks}` is a filter on the scanned files (matched
  against the file stems, `roles` and `notes` then align with the filtered files), `analyze_start {tracks}` and
  `export_all {tracks}` accept patterns in the name list. A pattern is an exact name (`k in`), a glob (`t*`, `s?top`:
  `*` any run of characters, `?` one, not case-sensitive) or a numbered range (`t1-t4`: every track `t1`, `t2`, ... up to
  and including `t4`). A pattern that matches nothing is an error (400) that names the pattern.
- `export_all {roles: ["kick", "snare"]}` exports the tracks whose role is one of these (case-insensitive), on its own or
  on top of `tracks` (then only the tracks of both). A role that no track has is an error (400) that names the role.

The owner's example job as a worked `drumspike run` script on the demo project (the generator
`uv run python tools/media/demo_session.py --out DIR` makes 13 mics in the owner's name scheme; the session folder form
puts them in `DIR/Audio Files/` beside a `song.ptx`):

```json
[
  {"command": "project_new", "args": {"name": "Demo", "dir": "/tmp/drumspike-demo"}},
  {"command": "tracks_add", "args": {"sid": "$sid", "paths": ["/tmp/demo-session"], "tracks": ["k in", "s top", "t1-t3"]}},
  {"command": "analyze_start", "wait": true, "args": {"sid": "$sid",
      "tracks": ["k in", "s top", "t1-t3"],
      "params": {"k_in": {"sensitivity": 55}, "s_top": {"sensitivity": 60, "vel_hi_db": -8}, "t1": {"sensitivity": 50}}}},
  {"command": "tempo_map_detect", "args": {"sid": "$sid"}},
  {"command": "export_all", "args": {"sid": "$sid", "roles": ["kick"], "formats": ["midi", "spikes"]}},
  {"command": "export_all", "args": {"sid": "$sid", "roles": ["snare"], "formats": ["midi", "spikes"]}},
  {"command": "export_all", "args": {"sid": "$sid", "tracks": ["t1-t3"], "formats": ["midi", "spikes"]}},
  {"command": "project_save", "args": {"sid": "$sid"}}
]
```

`tracks_add` picks the kick in, the snare top and the three toms out of the session folder by pattern (the rest of the
mics stays out); the track names are the safe names of the file stems (`k in` becomes `k_in`), so `params` and later
exact names use them, while the patterns accept the stem's spelling. `analyze_start` analyses exactly those with
per-track params, and the three `export_all` steps write `Exports/<track>_spike.mid` and `<track>_spike.wav` per role
group. `tempo_map_detect` answers a suggestion; pass it to `tempo_map_set` (or use `tempo_map_import_midi {path}` with
the DAW's own map) before the exports when the map should differ from the default 120 BPM.

## MCP

`drumspike mcp` speaks JSON-RPC 2.0, one message per line on stdin and stdout (protocol versions 2025-06-18, 2025-03-26 and
2024-11-05; `initialize`, `ping`, `tools/list`, `tools/call`). Every tool has the argument schema of its command; a tool with
an object result has an `outputSchema` and `structuredContent`. An error is a result with `isError: true`.

- Nothing is saved unless you call `project_save`. The description of each tool that changes the project in memory says so.
- `analyze_start` and `peaks_build_start` answer at once; poll `analyze_status` and `peaks_build_status`.
- `export_all`, `tracks_add`, `project_save_as` and `project_consolidate` answer `{job}` at once; poll `job_get {job}` until
  `state` is `done` (`result`) or `error`.
- `analyze_start` and a stored `track_repick` keep the hits you added or moved by hand (`keep_edited`, default true here).
- Learn marks (P4-6a): `track_learn_mark` (`kind` this or not, a range `t0` and `t1` or one `hit` time), `track_learn_remove`, `track_learn_forget_at` (Forget This: the examples at one `hit` or inside `t0` to `t1`; none there answers `removed: 0` and changes nothing), `track_learn_flip`,
  `track_learn_forget` and `track_learn_set` keep good and bad examples in the track's `learn` object (`tracks_get` shows it). A range takes its
  hits, else the strongest candidate, else the strongest rise of 6 dB or more; no hit gives `marked: false` with a `reason`. A follower marks its
  source (`routed: true`). No filter reads the marks yet. `track_learn_scores` (P4-6b) gives the similarity score of each detection candidate
  and of each example, and the automatic threshold; it writes only the cache `Analysis/<track>/learn.bin`.
- `track_velocity_set` also writes the new velocities and cut marks to the stored hits (`apply`, default true here; the page sends
  `apply: false` and applies the answer itself). It recomputes every velocity, so it comes before single-hit velocity edits.
- Export (P4-30): `export_all` takes `dest` (an absolute, existing, writable folder; default `Exports/` in the project folder), `midi_mode`
  (`per_track`, the default, or `single`), `tracks` (names or patterns, default all), `roles` (export by role, see the next section) and
  `formats` (`midi` and/or `spikes`, default both). `single` writes one
  type 1 file `<project>_drums.mid` instead of the `<track>_spike.mid` files: the project's tempo map, one MIDI track per exported track named after
  it, each on its own note (the track's note), the same ticks as the per-track files. The answer adds `dest` and `midi_file`. A spike export of one
  type (`spikes` or `audio`) also rewrites the other type's file of the same track (`<track>_audio_spike.wav` or `<track>_spike.wav`) when the folder
  holds one, from the same hits; the answer lists these files in `also_updated` (#274). `export_midi` and
  `export_spikes` take `dest` too. `export_existing` takes the same options and lists the files that `export_all` would replace (a bad `dest` is
  the same 400 or 404). `export_dest_set {dest}` keeps a folder as the project's export folder (document field `export_dest`; none clears it): the
  Export dialog starts with it. The commands never use the kept folder themselves. A bad `dest` is 400 (relative path, not a folder, not writable)
  or 404 (missing); `check_dest` writes and removes a probe file `.drumspike-write-test-<pid>` in the folder.
- Sampler calibration (P4-7c): `sampler_calibration_midi {name, dest?, path?}` writes `<track>_calibration.mid` (or the file `path`, P4-7d: the Layer section's save dialog; a source or a one-shot in use is refused) (one note, the track's note, velocity 100,
  on the start of bar 2 of the tempo map: 2.000 s with the default map). Import it in the DAW at the session start, bounce the sampler's kit piece
  for that note from the session start, then `track_sampler_set {name, path, one_shot?}` measures the bounce and keeps `sampler`
  `{offset_samples, sr, polarity, match, measured_at, file}` in the track (an empty `path` removes it; `one_shot: true` also sets the bounce as the
  layer, with its polarity); `track_sampler_get` reads it. `export_midi`, `export_all` (also `midi_mode: single`) and the drag files then put each
  note at the aligned attack (the hit, plus the layer's per-hit lag when its `align` is on) minus the offset, rounded to the sample; a note before 0 s
  goes to 0 s. `compensate: false` turns this off (default true). The answers have `sampler` (per track: `offset_ms`, `aligned`, `notes`, `clamped`)
  and `warnings`. A track without a calibration is written as before.
- One-shot layer (P4-7a): `track_layer_set {name, path?, ...}` sets the layer; `onset` (P4-40e, samples of the one-shot)
  overrides the measured onset for `export_layer`, the playback and `track_layer_align` (the answer's `layer.onset_set`
  carries it, `layer.onset` stays the measured one); `onset: null` resets to the measured onset, a new `path` clears it.
  `track_layer_wave {name, start?, end?}` and `track_sampler_wave {name, start?, end?}` answer the mono samples of the
  one-shot and of the calibration bounce as the bytes of `samples_window` (`u32 start`, `u32 rate`, then f32).
- The project zip import and export (`project_export_zip`, `project_import` with `zip_path`) were removed in P4-30; `project_import` takes `session_dir` only.
- `tools/list` gives every `Progress` tool the output schema `{job}`; the result of the work is in `job_get`.

## Locks and read-only

A process that opens a project for writing holds an exclusive lock on `<project>/.drumspike.lock` until it closes the project
or ends (a crash frees it). The app takes the same lock. A second process gets error 409 that names the holder and its pid.
`project_open` with `read_only: true` takes no lock and works while the app has the project open; it reads the saved document
(not the app's unsaved changes), and a command that changes the project is refused with 409. Allowed on a read-only project: every
read, `track_repick` without `store`, and `project_save_as` (the way to a writable copy: the project then is
the copy). A folder or volume that cannot hold a lock (read-only folder, no `flock`) does not refuse the project: it opens
unlocked and the answer says so (`lock: "unavailable: ..."`; `lock: "held"` otherwise). Only another holder gives 409. To switch from read-only to
writing, close the project and open it again. The agent works on projects the app does not hold for writing; it does not see
or change the live project of the running app (a local socket for that is a possible later option).

## Hits and notes

- `events_get` gives `[{t, v, ...}]` (`t` in seconds, `v` velocity 1 to 127). `events_edit` ops address a hit by its position
  in that list before the call; an added or moved hit must lie within the track (0 s to its `duration`, else 400). A hit you add has `added: true`; a hit you move keeps its detected time in `orig`; an edited hit
  is never cut by the loudness cut. `{op: add, t, v, added: false, orig?}` puts a hit back as a detected one (the page's undo of a delete).
  `{op: add, t, snap: true}` (off by default; the page's Option-click sets it) puts the hit on the attack within 10 ms of `t`,
  found by the refinement rule of the track's analysis, with the velocity that `velocities_recompute` gives there (a `v` in the op wins).
  With no attack within 10 ms the hit stays at `t` with velocity 100. The answer lists each snap op as `snaps: [{click, t, v, snapped}]`.
- P4-19, roll fill: an analysis of a snare track adds the soft strokes of a roll (the model's candidates down to 0.05 in a
  run of 8 or more equally spaced picks, 80 to 400 ms) as normal hits with `roll: true`. The `detector` record names the
  version (`roll_rule`); a project analysed before keeps its hits until Re-analyze; `DRUMSPIKE_ROLL_RULE=0` turns it off.
- P4-31, kick walk: an analysis of a kick track runs the refinement back-track with a peak margin of 29 dB (the shipped margin is 33 dB):
  the same hits, onsets a little later where a quiet lead-in lies before the foot. The `detector` record names the version
  (`kick_walk`); a project analysed before keeps its hits until Re-analyze; `DRUMSPIKE_KICK_WALK=0` turns it off.
- P4-13 stage B, flams: after an analysis and its bleed rules the second stroke of a flam inside a snare hit's lobe (a 1 kHz
  attack re-rise of 9 dB, 12 to 50 ms after the hit) becomes a normal hit with `flam: true`, unless another track of the
  project (no snare and no sample track) explains it as bleed. The `detector` record names the version (`flam_rule`) and the
  outcome (`flam: {on, reason, added, gated, sources}`); `job_get` has the session's outcome in `flam`. The track param
  `flam_split: false` turns it off for one track; a project analysed before keeps its hits until Re-analyze;
  `DRUMSPIKE_FLAM_RULE=0` turns it off.
- P4-10 stage 5, kick rescue: after an analysis, its bleed rules and the flam rule, a kick close mic (no sample track) with a
  model analysis gets the hits of the DSP onset detector that lie 30 ms or more from every model hit, as normal hits with
  `rescue: true`. An added hit goes again when a tom or snare close mic of the project has a hit within 20 ms and the hit
  is 8 dB or more under the track's median hit level (bleed). The `detector` record names the version (`kick_rescue`) and
  the outcome (`rescue: {added, removed, median_db, sources}`); `job_get` has the session's outcome in `rescue`. A project
  analysed before keeps its hits until Re-analyze; `DRUMSPIKE_KICK_RESCUE=0` turns it off.
- P4-14, tom bleed: after an analysis the tom close mics' hits that the other toms' mics explain carry `bleed: true`. They stay in
  `events_get` but leave MIDI, spikes, `export_all`, the click and a follower, as cut hits do. `{op: restore, index}` takes the mark
  off one hit and sets `edited: true`: a re-pick or Re-analyze with `keep_edited` keeps the hit, and the rule never marks it
  again (an edited hit never has the mark). The track's `detector` record names the rule version (`bleed_rule`) and the outcome
  (`bleed: {on, reason, removed, played, shared?, sources?}`; `shared` names tom mic pairs counted as one drum).
  P4-17, version 2: the kick and snare close mics (role `kick` or `snare`, no sample or trigger name, no follower) are
  sources too (a tom hit is also bleed under 9.2 dB snare excess or 4.0 dB kick excess); `sources` names them. A
  P4-21, version 3: the tom pair rule (winner takes all) runs after version 2 on the hits still in the output: of two hits on tom
  close mics of different drums within 12 ms, the one 6 dB or more lower (its level against the median level of its mic's output hits)
  carries `bleed: true` too when it is also 3 dB or more under the median level of its own mic (the floor: bleed is quiet on its own mic, a unison stroke is not). It needs no bleed between the hits, so it also runs when rule CD is off (tom tracks edited to digital
  silence); its outcome is `bleed.pairs` (`{on, reason, removed}`; the job answer has it with a `tracks` count). An analysis with version 1
  or 2 keeps its marks until Re-analyze, and a re-pick runs the recorded version; `DRUMSPIKE_BLEED_RULE` pins 0, 1, 2 or 3.
  A version 1 analysis keeps its hits until Re-analyze; `job_get` after `analyze_start` has the session's outcome in `bleed` (the reason when the
  rule is off: fewer than two played tom mics, or tom tracks edited to digital silence).
- `sensitivity` (0 to 150, the Inspector slider; 50 is the default) can replace `thresholds` in `params`.
- `track_set_note` takes the MIDI number. General MIDI: kick 36, snare 38, closed hat 42, toms 41 to 50 (floor tom 41, low 45,
  mid 47, high 50), crash 49, ride 51. The names follow C1 = 36.

## Tempo and meter map (P4-8)

`tempo_map_get` gives the project's map (`{map, default}`), `tempo_map_set {map}` replaces the whole map in one step (the answer has the
`previous` map, null when the stored value was not a valid map (it is replaced in any case): setting it again is the undo; `{}` or `null` is 120 BPM, 4/4, bar 1 at 0 s), and `tempo_map_import_midi {path}` replaces the map
with the tempo and time-signature events of a MIDI file from a DAW. The MIDI exports of the project (`export_midi`, `export_all`) then carry the map
and put each note at its exact time through it; with the default map the files are the same as before. The map format is in
`docs/migration/project-format.md` ("Tempo and meter map"). Like every change, it needs `project_save`.

`tempo_map_detect {tracks?, min_bars?, min_step_pct?}` suggests a map from the hits of the kick and snare tracks (analyse them first): the
start tempo with a confidence and the other tempos it could be (half, double, 3/2), bar 1, the meter (4/4 unless the evidence for 3/4 is
clear) and tempo steps where a new tempo holds at least `min_bars` bars (default 4) and differs by at least `min_step_pct` (default 2 %). It
changes nothing: to accept, pass its `suggestion` to `tempo_map_set`. A low confidence or a `warnings` entry means: check the tempo (an
octave error, half or double, is the usual fault). Example: `drumspike tempo_map_detect --project ~/Song --pretty`.
Each hit counts with its velocity (ghost notes weigh little); hits after the end of the tracks are left out with a warning. The
suggested tempo is a whole BPM (or 0.5, 0.1) when that lies within the fit's `start.error_bar`. `bar1` has a `confidence` (the snare
`backbeat` measure times the `downbeat` measure: kicks on 1 against 3, crash hits of a named cymbal track, fill endings, the first
kick) and an `alternative` two beats later: below about 0.4 bar 1 is a coin flip, so ask the user. Name a cymbal or overhead track
in `tracks` (with the kick and snare tracks) to let its crash hits vote for beat 1. `meter.feel: "two-beat"` means 2/4, or 6/8 in
dotted quarters, fit the hits as well as 4/4.

## Review markers (P4-40g)

The project document holds the marker list under the `markers` key — a pass-through key, so a document
written by the page or an agent round-trips unchanged (format 1 stays). One marker is an object with a
numeric `t` (seconds of the audio) and an optional string `note`; an entry may carry more keys, they are
kept. `markers_get {sid}` answers `{markers, n}` (oldest first). `markers_set {sid, markers}` replaces
the whole list in one step and answers `{markers, previous, changed, n}`: one more call with `previous`
is the undo, an empty list clears the markers. Like every change it needs `project_save`. The page sets
markers with M (at the selected hit or the playhead), shows them in the Markers ruler and jumps with
Option+M and Option+Shift+M.

## Review queue

`track_confidence {sid, name, events?}` rates each hit of `events` (else the stored hits) and answers
`{confidence, source, doubt_below}`: one `{c, doubt, why}` per hit, in their order. `c` is 0 to 1, and a
hit under `doubt_below` (0.5) is doubtful; `why` lists the reasons. The value is computed on each call and
never stored; the detection results do not change. `source` is `model` (kept activations: the margin of
the activation over the threshold, and the level), `level` (the level only) or `follows`. A hit that
the user added or moved, or that is not in the output, is never doubtful. A hit that a rule added under
the model threshold (rescue, Learn Add, roll fill, flam) gets the cap of its tag times its level. The
rules are in `crates/app/src/confidence.rs`. The page steps through the doubtful hits of all tracks
with D (Hits > Next Doubtful Hit), lowest confidence first; Shift+D steps back.

Export (P4-40g): `export_markers_set {sid, on}` keeps the Export dialog's "Markers in MIDI" checkbox in
the document field `export_markers` (absent is off); `export_existing` answers it as `export_markers`.
With it on, the one multi-track file (`<project>_drums.mid`, `midi_mode: "single"`) holds one marker
event (FF 06) per review marker at its time through the tempo map, the note as the text or `Marker N`
(counted from 1 in time order). The per-track files keep only the map's own `Bar 1` marker.

#232: `export_compensate_set {sid, compensate}` keeps the Export dialog's "Compensate" checkbox in the
document field `export_compensate` (absent is the default, true); `export_last_get` answers the
effective value as `export_compensate`, `export_existing` only a kept choice (`null` before one, so the
dialog keeps its own default: on when a track has a sampler calibration). An `export_all` without a per-call `compensate`
uses the kept value; an explicit `compensate` argument still wins.

#232: the four settings that the #120 inventory named as gaps are numbers the agent and the app both
set: `track_params_set`/`track_repick` carry the refine window (`refine_lo_ms`, `refine_hi_ms`, also
in the Inspector's Detection section), the role's name pattern (`track_set_role {role_match}`) is in
the Inspector's Track section, `track_layer_set {range}` takes the velocity curve's dB range as a
plain number (the curve must be `db`), and the Compensate checkbox is `export_compensate_set`.

## What the agent cannot do

Play back (the transport needs the window and a sound device), draw or read waveform tiles, undo (close the project without
saving and open it again, or use `project_save_as` as a snapshot), and use the app's view settings.
