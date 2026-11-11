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
`... drumspike-desktop mcp`. The binary answers without a window: `drumspike-desktop --help` (also
`-h` or `help`) prints the modes `bridge`, `cli`, `mcp`, `--app-info` and `--analyze`, and
`--version` prints the version. Any other flag prints the usage to stderr and exits 2; an
unknown flag never opens the window.

## Claude Code

```bash
claude mcp add --scope user drumspike -- drumspike mcp
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
- Learn marks: `track_learn_mark` (`kind` this or not, a range `t0` and `t1` or one `hit` time), `track_learn_remove`, `track_learn_forget_at` (Forget This: the examples at one `hit` or inside `t0` to `t1`; none there answers `removed: 0` and changes nothing), `track_learn_flip`,
  `track_learn_forget` and `track_learn_set` keep good and bad examples in the track's `learn` object (`tracks_get` shows it). A range takes its
  hits, else the strongest candidate, else the strongest rise of 6 dB or more; no hit gives `marked: false` with a `reason`. A follower marks its
  source (`routed: true`). No filter reads the marks yet. `track_learn_scores` gives the similarity score of each detection candidate
  and of each example, and the automatic threshold; it writes only the cache `Analysis/<track>/learn.bin`.
- `track_velocity_set` also writes the new velocities and cut marks to the stored hits (`apply`, default true here; the page sends
  `apply: false` and applies the answer itself). It recomputes every velocity, so it comes before single-hit velocity edits.
- Export: `export_all` takes `dest` (an absolute, existing, writable folder; default `Exports/` in the project folder), `midi_mode`
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
- Sampler calibration: `sampler_calibration_midi {name, dest?, path?}` writes `<track>_calibration.mid` (or the file `path`: the Layer section's save dialog; a source or a one-shot in use is refused) (one note, the track's note, velocity 100,
  on the start of bar 2 of the tempo map: 2.000 s with the default map). Import it in the DAW at the session start, bounce the sampler's kit piece
  for that note from the session start, then `track_sampler_set {name, path, one_shot?}` measures the bounce and keeps `sampler`
  `{offset_samples, sr, polarity, match, measured_at, file}` in the track (an empty `path` removes it; `one_shot: true` also sets the bounce as the
  layer, with its polarity); `track_sampler_get` reads it. `export_midi`, `export_all` (also `midi_mode: single`) and the drag files then put each
  note at the aligned attack (the hit, plus the layer's per-hit lag when its `align` is on) minus the offset, rounded to the sample; a note before 0 s
  goes to 0 s. `compensate: false` turns this off (default true). The answers have `sampler` (per track: `offset_ms`, `aligned`, `notes`, `clamped`)
  and `warnings`. A track without a calibration is written as before.
- One-shot layer: `track_layer_set {name, path?, ...}` sets the layer; `onset`
  overrides the measured onset for `export_layer`, the playback and `track_layer_align` (the answer's `layer.onset_set`
  carries it, `layer.onset` stays the measured one); `onset: null` resets to the measured onset, a new `path` clears it.
  `track_layer_wave {name, start?, end?}` and `track_sampler_wave {name, start?, end?}` answer the mono samples of the
  one-shot and of the calibration bounce as the bytes of `samples_window` (`u32 start`, `u32 rate`, then f32).
- The project zip import and export (`project_export_zip`, `project_import` with `zip_path`) were removed; `project_import` takes `session_dir` only.
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

- `reanalyze_selection {sid, name, t0, t1, keep_edited?}` (#265): re-analyzes only the hits of the
  time range `[t0, t1]` (seconds) on one track. The detector runs with a 100 ms pad on both sides;
  only hits whose refined time lies in the range are taken, the hits outside keep their times, ids
  and velocities, and a new hit closer than the minimum gap to an untouched outside hit is dropped.
  `keep_edited` (default true) keeps the hits added or moved by hand inside the range (they win
  within 20 ms over a picked one); the backup of the previous events is written as for a
  whole-track run and the followers of the track are refreshed. The answer is
  `{track, t0, t1, replaced, n_new, events, info, followers?}`, with `events` the whole stored
  list and `info` the line the page shows (`Kick In: re-analyzed 0.900 to 2.100 s, 3 hits replaced
  by 4`). A follower is refused (409), as in `events_set`.
- `events_get` gives `[{t, v, ...}]` (`t` in seconds, `v` velocity 1 to 127). `events_edit` ops address a hit by its position
  in that list before the call; an added or moved hit must lie within the track (0 s to its `duration`, else 400). A hit you add has `added: true`; a hit you move keeps its detected time in `orig`; an edited hit
  is never cut by the loudness cut. `{op: add, t, v, added: false, orig?}` puts a hit back as a detected one (the page's undo of a delete).
  `{op: add, t, snap: true}` (off by default; the page's Option-click sets it) puts the hit on the attack within 10 ms of `t`,
  found by the refinement rule of the track's analysis, with the velocity that `velocities_recompute` gives there (a `v` in the op wins).
  With no attack within 10 ms the hit stays at `t` with velocity 100. The answer lists each snap op as `snaps: [{click, t, v, snapped}]`.
- Roll fill, kick walk and kick rescue are not part of the DSP detector. An old project can carry `roll` and `rescue` marks. They stay on the hit and cap its `track_confidence` (0.4 and 0.3); they do not change the export.
- Flams: after an analysis and its bleed rules the second stroke of a flam inside a snare hit's lobe (a 1 kHz
  attack re-rise of 9 dB, 12 to 50 ms after the hit) becomes a normal hit with `flam: true`, unless another track of the
  project (no snare and no sample track) explains it as bleed. The `detector` record names the version (`flam_rule`) and the
  outcome (`flam: {on, reason, added, gated, sources}`); `job_get` has the session's outcome in `flam`. The track param
  `flam_split: false` turns it off for one track; a project analysed before keeps its hits until Re-analyze;
  `DRUMSPIKE_FLAM_RULE=0` turns it off.
- Tom bleed: after an analysis the tom close mics' hits that the other toms' mics explain carry `bleed: true`. They stay in
  `events_get` but leave MIDI, spikes, `export_all`, the click and a follower, as cut hits do. `{op: restore, index}` takes the mark
  off one hit and sets `edited: true`: a re-pick or Re-analyze with `keep_edited` keeps the hit, and the rule never marks it
  again (an edited hit never has the mark). The track's `detector` record names the rule version (`bleed_rule`) and the outcome
  (`bleed: {on, reason, removed, played, shared?, sources?}`; `shared` names tom mic pairs counted as one drum).
  Version 2: the kick and snare close mics (role `kick` or `snare`, no sample or trigger name, no follower) are
  sources too (a tom hit is also bleed under 9.2 dB snare excess or 4.0 dB kick excess); `sources` names them.
  Version 3: the tom pair rule (winner takes all) runs after version 2 on the hits still in the output: of two hits on tom
  close mics of different drums within 12 ms, the one 6 dB or more lower (its level against the median level of its mic's output hits)
  carries `bleed: true` too when it is also 3 dB or more under the median level of its own mic (the floor: bleed is quiet on its own mic, a unison stroke is not). It needs no bleed between the hits, so it also runs when rule CD is off (tom tracks edited to digital
  silence); its outcome is `bleed.pairs` (`{on, reason, removed}`; the job answer has it with a `tracks` count). An analysis with version 1
  or 2 keeps its marks until Re-analyze, and a re-pick runs the recorded version; `DRUMSPIKE_BLEED_RULE` pins 0, 1, 2 or 3.
  A version 1 analysis keeps its hits until Re-analyze; `job_get` after `analyze_start` has the session's outcome in `bleed` (the reason when the
  rule is off: fewer than two played tom mics, or tom tracks edited to digital silence).
- `sensitivity` (0 to 150, the Inspector slider; 50 is the default) can replace `thresholds` in `params`.
- `track_params_set {sid, name, params, replace?}` and the `params` of `analyze_start` (with
  `replace`) merge into the track's stored params (#442): a key in the call replaces that key, a
  key with `null` removes it, a key that is absent keeps its stored value. With `replace: true`
  the stored object becomes the sent object (the page sends whole objects). A `sensitivity`
  without `thresholds` stores the thresholds of that sensitivity.
- `track_set_note` takes the MIDI number. General MIDI: kick 36, snare 38, closed hat 42, toms 41 to 50 (floor tom 41, low 45,
  mid 47, high 50), crash 49, ride 51. The names follow C1 = 36.

## Tempo and meter map

`tempo_map_get` gives the project's map (`{map, default}`), `tempo_map_set {map}` replaces the whole map in one step (the answer has the
`previous` map, null when the stored value was not a valid map (it is replaced in any case): setting it again is the undo; `{}` or `null` is 120 BPM, 4/4, bar 1 at 0 s), and `tempo_map_import_midi {path}` replaces the map
with the tempo and time-signature events of a MIDI file from a DAW. The MIDI exports of the project (`export_midi`, `export_all`) then carry the map
and put each note at its exact time through it; with the default map the files are the same as before. The map format is in
the module comment of `crates/app/src/tempo.rs`. Like every change, it needs `project_save`.

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

## Review markers

The project document holds the marker list under the `markers` key (a pass-through key), so a document
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
never stored; the detection results do not change. `source` is `level` (the level only) or `follows`. A hit that
the user added or moved, or that is not in the output, is never doubtful. A hit that a rule added under
the level threshold (Learn Add, flam) gets the cap of its tag times its level. The
rules are in `crates/app/src/confidence.rs`. The page steps through the doubtful hits of all tracks
with D (Hits > Next Doubtful Hit), lowest confidence first; Shift+D steps back.

## Training Mode

A tool for the owner who refines the product, off by default (View > Training Mode, a page setting). With
`training: true` on `events_edit` and `track_learn_mark`, the command appends one JSON line per hit affected
(op `move`, `add`, `delete`, `confirm`, `learn_this` or `learn_not`) to `<DRUMSPIKE_HOME>/training/<project id>.jsonl`: the
time, the app version, the detect and refine rule versions, the track, role and rate, `from` and `to` in samples,
the event as stored before the op, its `confidence` and the track's samples from 40 ms before to 10 ms after
`from` and `to` (`audio.at_from`, `audio.at_to`). #389: every record also says who made the edit, `by: "person"`
(the keyboard, the menu, the mouse, a Learn mark) or `by: "agent"` (a `DSAgent` helper); a record of an older log
carries no `by`. The answer gets `training: {records}` (or `training_error`; the
edit stands). Without the parameter nothing is written and the answer is unchanged.
#354: `{op: confirm, index}` (Training Mode only, else refused) logs the hit as reviewed, right where it is:
the document, its `dirty` flag and the undo history stay as they were, the record says `from` = `to` = the stored
position and carries one audio window.
#363: a `move`, `add`, `delete` or `confirm` record carries `placement: {edge, line_noise, line_slope,
onset_spread}`, the measures of the review queue (`placement_diag`) at its `from` (its `to` for an add), computed on
demand and never stored in the project: `edge` says whether a steep edge counts, `line_noise` and `line_slope` are
shares of the hit's peak (the slope per ms), `onset_spread` is the hit's `timing_confidence` in samples or null.
A record is exact when `edge` is true, `line_noise` is at most 0.01 and `onset_spread` is under 4 samples or null;
else it is approximate. A record without `placement` (an older log, a Learn mark) is unknown.
`training_status {sid, on?}` answers `{on, records, path}`. `training_hand_placed {sid, track?}` (#418) reads
the same log and answers `{records, path}` with one trimmed row `{track, op, by, id, sample}` per `move`,
`add` or `confirm` record whose `by` is `person` or absent - the audio windows stay unread, so the agent
names the hand-placed hits without reading the log itself. `tools/training_export.py <jsonl...>` prints the
aggregates per role and op, with the count of exact, approximate and unknown records and the split of who made
the edits (`by person: x, agent: y, unknown: z`), and writes the truth list
`[{track, role, from, to, op, precision, by, exact, edge, line_noise, line_slope, onset_spread}]`. `--by` selects
whose edits the truth list keeps: `person` (the default), `agent`, `unknown` (the records without a `by`) or
`all` (every record).

### Page helpers (`window.DSAgent`)

#371: for a visual hit-by-hit pass the page (`drumspike bridge`, in a browser) loads `static/agent.js`, which exposes
`window.DSAgent`. The functions are plain and async, draw nothing of their own and do nothing until called. A track is
named by its track name, a hit by its index in the track's hit list. Every move and confirm runs through the code the
keyboard uses (`nudgeSelected`, Hits > Confirm Hit), so the record of an agent is byte for byte the record of a person.
The helpers that change the document (`move`, `confirm`, `undo`, `apply`) refuse with an error that names the menu command
`training_mode` while Training Mode is off; turn it on first with that command. The skill `skills/drumspike-finetune` uses them.

- `hits(track)`: the hits as `{index, t, sample, v, added, edited, flam, bleed}`; with Training Mode on each also has
  `confidence`, `doubt` and `why` (the reasons of the review queue, `track_confidence` with `training: true`).
- `look(track, index, opts?)`: selects the hit and sets the sample-zoom view (0.9 ms before, 0.6 ms after, the amplitude
  zoom to the local peak; `opts.before_ms`, `opts.after_ms`, `opts.amp` override; the page never shows less than 2 ms),
  waits for the draw and answers `{sample, start, hit_at, samples, trainingView, judge, ...}`. `samples` is the window as
  shares of the local peak and `hit_at` is the place of the hit's own sample in it. `stroke_peak` (#394) is the hit's peak
  in raw samples, the largest sample from the hit over the detector's own span (`EDGE_SPAN_S`, 5 ms); `null` when it
  cannot be read. `trainingView` is the view the record
  will carry. `band` (#408) states the unit of the judge: `{pct: 0.025, of: "the hit's peak", span_ms: 5,
  share_of_local_peak}` (`DEPARTURE_VIS_PCT` over `EDGE_SPAN_MS`, as a share of the local peak that `samples` uses).
  `judge` is `{level, spread, first_off, suggest, foot_off, rise, quiet_ratio, verdict}`. It measures the line directly
  before the stroke: the steepest step near the hit, walked back to its foot (the walk stops where a step falls under 25 %
  of the steepest, so a slow wave is line, and goes back over the samples above the band that lead into the foot), and the
  line from the 12 quietest samples ending nearest the foot. `foot_off` is the foot against the hit (negative: the foot is
  earlier), `rise` the hit sample's distance from the line (a share of the local peak), `quiet_ratio` the steepest step over
  the largest step of the line, and `verdict`: `late` (the hit stands up the rise: its own sample is over the band and the foot
  is earlier), `at_foot`, or `no_quiet_line` (no quiet stretch exists, as after a previous stroke's ring; `foot_off` may
  be set, `suggest` is `null`). `suggest` is the offset in samples that the placement rule suggests (the hit is the last
  line sample before the stroke's first motion), 0 for a hit whose own sample is within the band (`judge.why` is then
  `"under the detector's band"` when the foot is earlier) or when no step near the hit is over the band (the stroke rises
  past the view; `judge.why` is `"no motion above the band near the hit"`), and `null` when the verdict is
  `no_quiet_line` or the foot lies more than 12 samples before or 6 after the hit (a pre-bump, bleed or wobble,
  not the stroke; `judge.why` is then `"departure far from the hit"`, the verdict is still `late` or `at_foot`).
  A sample within the band is the foot only when the line leads to it: a foot that the level reached in one run from
  beyond the band on the other side is a zero crossing of the stroke's first ripple, and the foot is then before that
  ripple (the verdict is `late`). When the ripple stays under the band the hit stands and `judge.why` is
  `"a motion under the detector's band leads in"`. A foot in a dip of a rise over the band on the stroke's own side is
  not clear: `suggest` is `null` and `judge.why` is `"a dip after a rise over the band: the foot is not clear"`. When
  the verdict is `late`, move the hit by `judge.foot_off` samples (a whole number; negative is earlier); a late hit can
  carry a `null` `suggest` when its foot is far from the hit - look at those yourself, and list one you do not move
  with the reason.
  The judge has a second opinion (#447), the pattern of the track: the median foot of the track's own clear hits (a hit
  is clear when the 2 ms before it stay within 1 % of the peak of the 5 ms after it; the foot is 1/3 ms before the hit to
  1/6 ms after it; three rounds of re-alignment; no label and no session value). `look` answers `pattern`: `{hits, clear,
  off, score}` - the hits and the clear hits the pattern was built from, where it matches this hit best (`off`, in
  samples from the hit, negative is earlier; `null` when the view is too short for the search or the hit does not look
  like the pattern: the best match at the end of the search range, or a correlation under 0.5) and the match - or `null`
  for a track with fewer than 40 hits. `judge` then carries `pattern_off` and `pattern_agrees`. A move stands only when
  the pattern confirms it within one sample. When the judge would move the hit and the pattern puts the start more than
  one sample from that move, or the judge would leave the hit and the pattern puts the start 2 or more samples away,
  `suggest` is `null`, `judge.line_suggest` keeps the judge's own number and `judge.why` is `"two opinions differ: the
  line before the hit puts the start ..., the track's clear hits put it ..."`. The `verdict` stays the judge's. Such a
  hit is never a move: list it for the user. The pattern is built once per track (one read per hit) and kept until the
  count of hits changes. Without a
  readable `stroke_peak` the band is 3 % of the local peak. A page in the
  background gets no animation frames: `look` waits at most 300 ms for the draw and goes on, so a hidden page is slower
  but never stuck.
- `wide(track, index)`: the same answer for the 20 ms view on the hit, for deletes and bleed checks.
- `render(track, index, opts?)`: a picture of the view as the user sees it: the sample-zoom window of the hit
  (the `look` defaults; `opts.before_ms`, `opts.after_ms`, `opts.amp` override), drawn from the same samples and
  amplitude zoom the page draws, with the hit marked. Answers `{track, index, sample, width, height, format,
  image}`; `image` is a PNG data URL.
- `move(track, index, samples)`: the nudge by whole samples (negative is earlier) with the view at sample zoom, so the
  record says precision `sample`. Answers `{moved, index, from, to}`.
- `confirm(track, index)`: Hits > Confirm Hit for a reviewed, unchanged hit. `undo()`: Edit > Undo; an undo writes no record.
- `plan(track, from, to)`: `look` over the hit indices `from` to `to` without moving; one row per hit with `index`, `id`,
  `sample`, `suggest`, `spread`, `verdict`, `confidence`, `line_ok`, `pattern_off`, `pattern_agrees`, `why`, `queue_doubt` and `queue_why`. `line_ok` is the
  judge's verdict (`suggest` is not `null`); `why` says why it is not. `queue_doubt` and `queue_why` are the review queue's doubt:
  a reason to look, not a block (on some kits the queue doubts most kicks). Use chunks of about 100 hits.
- `audit(track)` (#408, #447): every hit of the track whose shape `verdict` is `late`, and every hit whose two opinions
  differ, as `[{index, id, t, reason, suggest, foot_off, rise, quiet_ratio, pattern_off, why}]`. `reason` is `late` (move
  it by `suggest` when that is a number) or `opinions differ` (never a move: a hit for the user's list, `why` names both
  positions). It reads the shape measures of `look` and never a `plan` row, so a wrong suggestion cannot hide a late hit. One `look` per hit: run it on a track you have not finished, not in a loop.
- `handPlaced(track)` (#418): the hits the user placed by hand, as `[{index, id, t, sample, ops}]` - one row per
  hit that carries a person's move, add or confirm record in the project's training log (`by` `person` or
  absent; an agent's record never counts). The service reads the log - it is far too large to read - and answers
  trimmed rows; the helper folds them into the hits by the records' event id, else by their sample, and drops a
  record no stored hit matches. Call it on every track before you work on it, and leave those hits as they are.
- `apply(track, list)`: the moves `[{index, samples}]` (a row of `plan` also works, its `suggest` is used); a zero or `null`
  move is skipped. `save()` writes the project. `backups()` answers `{rule, before}`: a backup is named with the time of
  the save that replaced it, but holds the state as it was just BEFORE that save - `before` names the backup that holds
  the state before this session (the one the session's first save wrote; `null` until the session saves). `status()`
  answers `{training, records, path}`.
- `statusLine()`: what the window shows, `{message, selection, training}` (the status message, the selection field and the
  Training Mode counter, `null` while the mode is off). `setTraining(on)`: the window's Training Mode switch, answers `{training}`.

### The running window (#388)

The page helpers run in the page of the window, so the app opens a local endpoint that passes a call to that page: no
browser, no second process on the project, no quit. The window you see is the project the agent edits. At start the app
writes `<home>/window-endpoint.json` (`{port, token, pid}`, mode 0600, removed at exit); only a process that can read that file
can call.

- `POST http://127.0.0.1:<port>/call` with `Authorization: Bearer <token>` and the body `{"fn": "move", "args": ["Kick In", 1, 3],
  "timeout_ms": 60000?}`. `fn` is one of `project`, `hits`, `look`, `wide`, `render`, `move`, `confirm`, `undo`, `plan`,
  `audit`, `handPlaced`, `apply`, `save`, `backups`, `status`, `statusLine`, `setTraining`; any other name is refused with 400 before it
  reaches the page. The answer is `{"ok": true, "value": <what the helper returned>}`.
- Errors: 401 without the token (or a wrong one); 409 when no page listens or the helper refused (its message is the page's,
  for example "Training Mode is off"); 504 when the page did not answer within `timeout_ms` (default 60 s, at most 600 s).
- `GET /status` (same token) answers `{listening, helpers}`.
- The one call: an agent does not read the token file or speak HTTP itself. The MCP tool `agent_call {fn, args?, timeout_ms?}`
  and the command line `drumspike agent_call <fn> [arg]...` (on the app binary: `<binary> cli agent_call <fn> [arg]...`; each `arg` is JSON when it parses, else a string) read
  `<home>/window-endpoint.json`, post the call and answer the helper's value. With no window open (no file, or a dead port)
  they answer a 409 "no Drum Spike window is open" (exit code 4 on the command line): then start the bridge with a browser tool.
- `project()` answers the project the window has open (its summary and the track names).
- The page polls the commands `window_call_next` and `window_call_result` (window surface only). A newer page ends the poll of
  an older one, so a closed window never swallows a call. `DRUMSPIKE_WINDOW_ENDPOINT=1` opens the same endpoint in the test
  bridge (`ds-bridge`), with the file in `DRUMSPIKE_HOME`.

## Agent fine-tune skill

#371: the skill `skills/drumspike-finetune/` (a `SKILL.md` and a `README.md`) ships inside the app bundle as a resource. Two
commands, both on the agent surface, put it where the agent looks for skills:

- `agent_skill_install {target?}` copies the bundled folder to `target` (default `~/.claude/skills/drumspike-finetune`,
  Claude Code's user skills; another runtime or path with `target`), replaces an older copy, and writes the absolute path of
  the running binary into `SKILL.md` for its `<binary>` placeholders, so no symlink is needed. It answers
  `{installed, files, version}`; `installed` is the path. It refuses (409) a `target` that holds other files and no `SKILL.md`.
- `agent_skill_status {target?}` answers `{installed, path, version, bundled_version, current, binary, mcp_command}`.
  `current` is false for an older copy, or one written by another binary; `mcp_command` is the one line that adds the
  MCP server for this binary.

The window has the same in Settings, the row "Agent fine-tune skill": the status text (not installed, installed with the
version, older copy), one button (Install, Update for an older copy) and the `claude mcp add` line with a Copy button. The
owner does two things once: Install, then the `claude mcp add` line. A session starts with: "Fine-tune the drums of
~/Music/Sessions/Song in Drum Spike." `DRUMSPIKE_SKILL_SOURCE` and `DRUMSPIKE_SKILL_TARGET` name the bundled folder and
the default target for a test or a packaging step.

Export: `export_markers_set {sid, on}` keeps the Export dialog's "Markers in MIDI" checkbox in
the document field `export_markers` (absent is off); `export_existing` answers it as `export_markers`.
With it on, the one multi-track file (`<project>_drums.mid`, `midi_mode: "single"`) holds one marker
event (FF 06) per review marker at its time through the tempo map, the note as the text or `Marker N`
(counted from 1 in time order). The per-track files keep only the map's own `Bar 1` marker.

#232: `export_compensate_set {sid, compensate}` keeps the Export dialog's "Compensate" checkbox in the
document field `export_compensate` (absent is the default, true); `export_last_get` answers the
effective value as `export_compensate`, `export_existing` only a kept choice (`null` before one, so the
dialog keeps its own default: on when a track has a sampler calibration). An `export_all` without a per-call `compensate`
uses the kept value; an explicit `compensate` argument still wins.

#232: four settings that were gaps are numbers the agent and the app both
set: `track_params_set`/`track_repick` carry the refine window (`refine_lo_ms`, `refine_hi_ms`, also
in the Inspector's Detection section), the role's name pattern (`track_set_role {role_match}`) stays
an agent field (#285 removed its Inspector field: the role suggestion reads a term table per drum, and a
role set by hand wins), `track_layer_set {range}` takes the velocity curve's dB range as a
plain number (the curve must be `db`), and the Compensate checkbox is `export_compensate_set`.

## Page mode (`drumspike bridge`)

For a visual pass (hit by hit at sample zoom in the app's own page), the shipped binary serves the page and
the invoke API on localhost:

```bash
"/Applications/Drum Spike.app/Contents/MacOS/drumspike bridge"          # or: drumspike bridge
drumspike bridge --port 7777 --home ~/Library/Application\ Support/com.cbv.drumspike
```

It prints one line when it listens — open that URL in a browser:

```
drumspike bridge: http://127.0.0.1:<port>/
```

- The page is the one bundled in the binary (the Tauri asset context, `frontendDist`); the app bundle gains
  no files. The API is the same command registry the window and the command line use (`POST
  /invoke/<command>`, `GET /channel/<id>`, `GET /events?after=N`), on 127.0.0.1 only, no token — nothing
  here is a security boundary, like the development bridge.
- `--home DIR` and `--projects DIR` default to the app's own folders (the same as the window), so a project
  the owner opened in the window is the one the agent sees. `--port 0` (the default) picks a free port.
- The window and the bridge do not share one home at the same time. Both hold an advisory flock on the
  home folder (`<home>/.drumspike.lock`, the mechanism the project locks use): the bridge refuses to start
  while the window runs on that home — close the window, or pass `--home DIR` — and names the holder
  (`app (pid N)`). A window started while a bridge holds the home prints a warning and keeps working; end
  the bridge session before opening the window on that home.
- In the page, the agent helpers of the sibling ticket drive the editor; Training Mode (View > Training
  Mode) writes a record per hit edit as everywhere else. The shipped page mode has no native menu: the
  agent turns Training Mode on from the page itself with `setTraining(true)` (or `toggleTraining()`);
  the `training_mode` menu event exists only in the debug bridge.
- Development: `ds-bridge` keeps serving `static/` for the tests, and `drumspike bridge --static DIR`
  serves a folder instead of the embedded assets.

## What the agent cannot do

Play back (the transport needs the window and a sound device), draw or read waveform tiles, undo (close the project without
saving and open it again, or use `project_save_as` as a snapshot), and use the app's view settings.
