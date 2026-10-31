# 120 Step 1: settings inventory (every setting with its scale, its agent API argument and its gap)

Ticket #120, step 1. The goal of the ticket: every setting as a number in the app and the agent
API, so a headless job can set any setting the app can. This report lists every user-settable
setting with (a) its scale, (b) the agent API argument that sets it, and (c) its gap: `none`
when the app and the agent API can both set it, otherwise what is missing. No code changes
(docs only). The walk of the owner's example job on the demo project is at the end, with one
gap per missing step and the command it would need.

Sources: the command registry `crates/app/src/api.rs` (118 commands, 78 on the agent surface),
the param parser `crates/app/src/jobs.rs` (`parse_params`), `crates/app/src/sensitivity.rs`,
`crates/core/src/analyze.rs` (`AnalyzeParams`), `crates/core/src/project/document.rs`
(`Track`, `Settings`, `Document`), the Inspector panel (`static/app.js` `buildParamsPanel`),
the mixer section of `static/app.js` and the agent API reference `docs/agent-api.md`. A row is
a setting, not a command; counted rows per table at the end.

## A. Track detection and velocity parameters

Documented in the track `params` map (`crates/app/src/jobs.rs` `parse_params`). Agent surface:
`track_params_set {sid, name, params}` and `track_repick {sid, name, params, ...}`; the CLI
`--project` form takes the same arguments.

| # | setting | scale | agent API argument | gap |
|---|---|---|---|---|
| 1 | Sensitivity | 0 to 150 (slider, default 50; the five class thresholds scale geometrically, floor 0.01, ceiling 0.85) | `params.sensitivity` | none |
| 2 | Class thresholds | 5 numbers, 0.01 to 0.85 (kick, snare, tom, hat, cymbal; default 0.22, 0.24, 0.32, 0.22, 0.30) | `params.thresholds` (wins over `sensitivity`) | none |
| 3 | Minimum gap | 1 to 500 ms, default 20 (UI step 1 ms) | `params.min_gap_ms` | none |
| 4 | Refine to attack | bool, default on | `params.refine` | none |
| 5 | Refine window low | ms, default -2 (the search window around the coarse time) | `params.refine_lo_ms` | UI gap: no Inspector control (agent sets it, the app shows it only via the detector record) |
| 6 | Refine window high | ms, default 12 | `params.refine_hi_ms` | UI gap: same as refine_lo_ms |
| 7 | Hitpoint shift | -100000 to 100000 samples, step 1, default 0 (positive = later) | `params.shift_samples`, also `track_shift_set {shift_samples}` (shifts the stored hits at once) | none |
| 8 | Velocity floor | dBFS, default -40 (velocity 20 at or below) | `params.vel_lo_db` | none |
| 9 | Velocity ceiling | dBFS, default -6 (velocity 127 at or above) | `params.vel_hi_db` | none |
| 10 | Loudness cut | dBFS, 0 or below, default off (hits below are cut from MIDI, spikes, export, click and followers) | `track_velocity_set {cut_db}` (default off; also recomputes velocities) | none |
| 11 | Detector | id string, `builtin` (default) or an installed plug-in id | `params.detector`; `analyze_start --detector <id>` as a per-run default | none |
| 12 | Split flams | bool, default on (snare tracks only) | `params.flam_split` (kept in `params`, read by the flam rule on the next analyze) | none |

The retry ladder of a track without a hit (95, 113, 134, 150 sensitivity, kept when the hit count
grows by more than 10 %) is a behaviour, not a setting: the kept value is stored as `sensitivity`
(#1). The rule environment pins (`DRUMSPIKE_ROLL_RULE`, `DRUMSPIKE_KICK_WALK`,
`DRUMSPIKE_FLAM_RULE`, `DRUMSPIKE_BLEED_RULE`, `DRUMSPIKE_KICK_RESCUE`) are host environment
variables, not document settings, so they are out of scope here.

## B. Track output, role, followers, layer, sampler, Learn

| # | setting | scale | agent API argument | gap |
|---|---|---|---|---|
| 13 | Role | enum: kick, snare, tom, hat, cymbal, sample, other (plus the audio-based suggestion) | `track_set_role {role}` | none |
| 14 | MIDI note | 0 to 127 (General MIDI: kick 36, snare 38, hat 42, toms 41 to 50, crash 49, ride 51) | `track_set_note {note}`, also `track_set_role {note}` | none |
| 15 | Role match | string, the name pattern the role was guessed from | `track_set_role {role_match}` | UI gap: the Inspector shows the role but no role_match field (an agent can set it) |
| 16 | Follower | the name of another track, or own detection (empty) | `track_set_follow {source}` (`source: null` is own detection) | none |
| 17 | Track volume | -120 to +12 dB, step 0.1 (fader -inf to +12) | `track_set_mix {volume_db}` | none |
| 18 | Track pan | -1 to 1, step 0.01 | `track_set_mix {pan}` | none |
| 19 | Layer on | bool | `track_layer_set {on}` | none |
| 20 | Layer file | path (an empty path clears it) | `track_layer_set {path}` | none |
| 21 | Layer level | -60 to +12 dB, step 0.5, default 0 | `track_layer_set {level_db}` | none |
| 22 | Layer polarity | normal or inverted | `track_layer_set {polarity}` | none |
| 23 | Layer curve | db, linear or fixed | `track_layer_set {curve}` | none |
| 24 | Layer range | 0 to 96 dB, step 1, default 24 | `track_layer_set {curve}` object | UI gap and agent gap: the range lives inside the `curve` value; `track_layer_set` takes no plain `range` number (it is reachable as part of `curve`) |
| 25 | Layer align | bool (align the one-shot to the hits) | `track_layer_set {align}` | none |
| 26 | Layer onset | samples of the one-shot (overrides the measured onset; `null` resets) | `track_layer_set {onset}` | none |
| 27 | Sampler calibration | file path plus the measured record (`offset_samples`, `sr`, `polarity`, `match`, `measured_at`) | `track_sampler_set {path, one_shot}`; an empty `path` removes it | none |
| 28 | Sampler compensate | bool, default true (notes at the aligned attack minus the offset) | per-export argument `compensate` on `export_midi`, `export_all`, `export_layer` | UI gap: the Export dialog checkbox has no agent command that keeps it in the document (unlike `export_markers_set`; it is a per-call argument only) |
| 29 | Learn filter on | bool (hits unlike the examples go grey and leave MIDI, spikes and the click) | `track_learn_set {learn}` (`on` inside the `learn` object) | none |
| 30 | Learn threshold | similarity score 0 to 1 (the red line of the Learn histogram) | `track_learn_set {learn}` (`threshold` inside the object) | none |
| 31 | Learn examples | the mark list (`kind`, `t0`, `t1` or `hit`) | `track_learn_mark`, `track_learn_remove`, `track_learn_forget_at`, `track_learn_flip`, `track_learn_forget` | none |

## C. Project and document settings

| # | setting | scale | agent API argument | gap |
|---|---|---|---|---|
| 32 | Tempo map | tempo events (BPM; the suggested tempo is a whole BPM, 0.5 or 0.1 when it lies in the fit's error bar), meter (4/4, 3/4), bar 1 position | `tempo_map_set {map}`; `tempo_map_import_midi {path}` from a MIDI file; `tempo_map_detect` suggests | none |
| 33 | Review markers | a list of `{t (seconds), note?}` | `markers_set {markers}` | none |
| 34 | Export folder | an absolute, existing, writable folder | `export_dest_set {dest}` (document field `export_dest`; `null` clears) | none |
| 35 | Markers in MIDI | bool, default off | `export_markers_set {on}` (document field `export_markers`) | none |
| 36 | Export formats | midi and/or spikes | `export_all {formats}` (per call), kept by `export_last_set {formats}` | none |
| 37 | MIDI export mode | per_track (default) or single | `export_all {midi_mode}`, kept by `export_last_set {midi_mode}` | none |
| 38 | Exported tracks | names list, default all | `export_all {tracks}` (per call), `scope` and `track` in `export_last_set` | gap: names only, no name pattern (see walk gap 2) |
| 39 | Row heights | pixels per track | `row_heights_set {heights}` | none (view state; the agent can set it but the app owns its layout) |
| 40 | Project name | string | `project_rename` | none |

## D. App settings (settings.json in the home folder)

`app_settings_set {key, value}` writes any short `[a-z_]` key (1 to 32 characters) and refuses
the audio keys. The keys the page writes, with their scales:

| # | key | scale | agent API argument | gap |
|---|---|---|---|---|
| 41 | ruler_tempo | bool | `app_settings_set {key: "ruler_tempo"}` | view state: the agent can write the key, but no command uses it headless (the agent "cannot use the app's view settings", `docs/agent-api.md`) |
| 42 | ruler_meter | bool | as above | same view-state gap |
| 43 | ruler_beats | bool | as above | same |
| 44 | ruler_markers | bool | as above | same |
| 45 | grid_lines | bool | as above | same |
| 46 | other_mics | bool | as above | same |
| 47 | velocity_lane | bool | as above | same |
| 48 | velocity_lane_height | pixels | as above | same |
| 49 | inspector | object (the Inspector view state) | as above | same |
| 50 | key_preset | enum (keymap preset) | as above | same |
| 51 | track_height | string (the layout preset name) | as above | same |
| 52 | nudge_value | enum (the nudge steps) | as above | same |
| 53 | amp_zoom | number | as above | same |
| 54 | overview | bool | as above | same |
| 55 | zoom_presets | list | as above | same |
| 56 | zoom_toggle_tall | bool | as above | same |

Audio settings (the keys `app_settings_set` refuses):

| # | setting | scale | agent API argument | gap |
|---|---|---|---|---|
| 57 | Master volume | -120 to +12 dB, step 0.1, default 0, kept in the app settings | `transport_master {volume_db}` | agent gap: `transport_master` is Window-only (it drives the audio device); no agent-surface command keeps `master_db`. Out of scope for a headless job (no device), named here for completeness |
| 58 | Audio output device | the device id of `outputs_list` | `output_select` | agent gap: Window-only, same as master volume |
| 59 | Click on | bool | `transport_clicks {enabled}` | agent gap: Window-only, same |
| 60 | Click volume | number (dB) | `transport_clicks {volume}` | agent gap: Window-only, same |
| 61 | Click pitches | the note per drum | `transport_clicks {pitches}` | agent gap: Window-only, same |

## Walk of the owner's example job on the demo project

The owner's example job, as a headless walk (`drumspike run script.json` on the demo project,
the generator `uv run python tools/media/demo_session.py --out DIR`, 13 mics in the owner's name
scheme: `k in`, `k out`, `s top`, `s bot`, `t1`, `t2`, `t3`, `hh`, `ride`, `oh l`, `oh r`,
`room`, `new take`):

1. Input: a Pro Tools session folder (audio files and the `.ptx`).
   GAP: `project_import {session_dir}` imports a session folder of the old drumspike server
   (`meta.json` and `tracks/<name>/...`), not a Pro Tools session folder. A PT folder can come
   in today only as loose audio via `tracks_scan` + `tracks_add` (the `.ptx` is ignored, see
   `crates/app/src/tracks/tests.rs`). Command it would need: a Pro Tools session import, for
   example `project_import --ptx <file or folder>` (or a `session_dir` importer that reads the
   `.ptx` track list), so the track names and roles come from the session instead of a guess.
2. Track selection by name pattern (`k in`, `s top`, `t1-t4`).
   GAP: `tracks_add {paths}` adds every given file, `export_all {tracks}` and the export scope
   take explicit name lists, `analyze_start` runs the whole project. No command takes a name
   pattern or a range. Command it would need: a pattern argument (`tracks_glob` or the same
   `tracks` argument accepting patterns like `t1-t4`) on `tracks_add`, `analyze_start` and
   `export_all`.
3. Parameters per track: `track_params_set` and `track_velocity_set` with the numbers (rows 1
   to 12). No gap.
4. Analysis: `analyze_start` (poll `analyze_status`). No gap.
5. Tempo map: `tempo_map_detect` (suggest), `tempo_map_set {suggestion}` to accept, or
   `tempo_map_import_midi {path}` for the DAW's own map. No gap.
6. Export per role (the MIDI and spike files per role group).
   GAP: `export_all {tracks}` takes names, not roles. The role of each track is in the document
   (`tracks_get`), so a script can resolve a role to names itself, but the command has no
   `roles` filter. Command it would need: `export_all {roles: [...]}` (or a documented
   resolve-and-loop example in the agent API).

## Counts

- Table A: 12 rows, 2 UI gaps (refine_lo_ms, refine_hi_ms have no app control).
- Table B: 19 rows, 3 gaps (role_match UI, layer range inside `curve`, sampler compensate as a
  per-call argument without a kept document field).
- Table C: 9 rows, 1 gap (no name patterns in the exported tracks).
- Table D: 16 view-state keys (agent-writable but unused headless) + 5 audio settings
  (Window-only, out of scope for headless).
- Walk: 6 steps, 3 gaps (Pro Tools session input, name-pattern selection, export per role).

Total: 61 counted rows plus the walk. Every setting of tables A to C is reachable headless on
the agent surface today except the 4 named gaps; the headless job of step 2 needs no
Window-only command.
