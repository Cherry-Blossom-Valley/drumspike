# Architecture

This file is the map of Drum Spike for a person who wants to change the code. It says where
things are and which rules the structure keeps. It does not explain every function. For commands
and gates read `AGENTS.md`; for the plan that shapes the current refactoring read
`docs/engineering/refactor-plan.md`.

## Bird's-eye view

Drum Spike reads the close-mic recordings of a drum kit, one WAV per microphone. It finds each
hit with a spectral onset detector, moves the hit to the first sample of its attack, gives it a
velocity and writes sample-exact MIDI and spike files. The product is a desktop app (Tauri 2).
The app has no Python. Python is only a test and research tool.

Three layers carry the work, from the inside out:

1. **core** (`crates/core`, `drumspike-core`): domain and DSP. It reads audio, detects, refines,
   decides which microphone owns a hit, maps loudness to velocity, stores the project document
   and writes MIDI and spike files. It knows nothing about the application.
2. **app** (`crates/app`, `drumspike-app`): the application service. Every command of the app is
   a plain function of `Service`. The app layer runs analysis jobs, applies the session rules
   (cross bleed, ghost gate, tom bleed, flam), keeps the event marks and serves the agent
   surfaces (call, bridge, command line, MCP). It has no Tauri dependency.
3. **shells**: `apps/desktop/src-tauri` (the Tauri window, menu and command wrappers) and
   `crates/devtools` (developer binaries). The page in `static/` is the user interface; it
   talks to the shell only through `static/backend.js`.

A user action travels like this: a click in the page calls a method of the object `B` in
`backend.js`; `B` sends one command over the IPC; the Tauri wrapper (or the test bridge, or the
command-line tool) calls `drumspike_app::call` with the command name; the registry in `api.rs`
knows the command; the handler is a method of `Service`; the method uses core functions on
decoded samples and returns JSON.

The detection result is a list of events. An event is a time, a velocity and marks. A mark is a
key in the event (`cut`, `bleed`, `flam`, ...). Rules do not delete hits; they mark them. The
export step reads the marks.

## Codemap

### `crates/core`

The module paths are the API (`drumspike_core::refine::refine_sr`). An item no code outside core uses is
`pub(crate)`; the crate root re-exports only the names callers import from the root. See `crates/core/README.md`.

| Module | What it holds |
|---|---|
| `io` | WAV and BWF reader at the native rate, explicit downmix, WAV writer. |
| `peaks`, `cache` | Min/max/RMS waveform pyramid and its cache; tile reads for the page. |
| `pyops` | Python-compatible rounding (half-even `py_round`, `int()` truncation); the frozen engine needs it. |
| `dsp` | Shared filter and statistics primitives: `Biquad`, `band_sections`, `band_peak_db`, `band_level_db`, `broadband_peak_db`, `percentile`, `np_percentile`, `median`, half-up `ms_to_samples`, the dB floor policies `amp_db_add` and `amp_db_max`. |
| `detect` | Detection per track: `features` (spectral flux), `onset` (candidates), `peaks` (thresholds), `nmfd`, `resample`. |
| `detect::attribution`, `tom_bleed`, `cross_bleed`, `flam` | Per-hit ownership and the special-case rules. |
| `refine::{reference, reach, walk, edge, track}` | Moves a coarse onset to the first sample of the attack, one module per mechanism. `RefineRule` v1 to v7 (v7 is current). `refine` re-exports the items callers outside core use; the tuning constants are crate-internal. |
| `analyze`, `backend` | Orchestrate detection for one track; detector backends. |
| `velocity` | Peak dB to MIDI velocity 1 to 127. |
| `role` | The instrument a track records and its MIDI notes. |
| `similarity`, `agreement` | Learn similarity score; detector and refine agreement. |
| `project` | The `.drumspike` document: tracks, sources, events, save, relink. Format 1 stays loadable. |
| `tempo`, `tempo_detect` | Tempo map and the inference of a map from kick and snare hits. |
| `midi`, `spike`, `layer`, `sampler` | Outputs: MIDI bytes, spike WAVs, layers, one-shot alignment. |
| `transport` | Real-time playback engine (feature `audio`): reader, mixer, clock, device, clicks. |

### `crates/app`

| Module | What it holds |
|---|---|
| `lib` | `Service` and `ServiceConfig`: the composition root and its environment pins. |
| `api` | `REGISTRY`: one row per command (name, arguments, result, effect, surface). |
| `call`, `command_policy` | Command name to handler; which surface may call what. |
| `policy` | Event marks and their precedence (`is_marked`, `mark`, `clear`, `is_manual`). |
| `jobs`, `copyjob` | The analysis job over all tracks, and copy jobs. |
| `steps` | `AnalysisStep` and the fixed list of session steps the analysis job runs (not the re-pick or Reanalyze selection). |
| `cross_bleed`, `ghost_gate`, `tom_bleed`, `flam` | Session steps: gather the microphones, call core, apply marks. |
| `histogram`, `reanalyze`, `learn*` | Re-pick after a sensitivity change, reanalyze a selection, Learn This and Forget This. |
| `edit`, `export`, `tracks`, `roles`, `lifecycle`, `registry`, `autosave`, `transport`, `layer`, `markers`, `tempo` | Commands on the open project. |
| `bridge`, `cli`, `mcp`, `headless`, `agent` | Adapters that drive `call`: test bridge, command line, MCP server, batch use. |

### Other crates and folders

| Path | What it holds |
|---|---|
| `crates/devtools` | Developer binaries, not shipped: `service_tool` (the app engine from a shell), `parity`, `bench`, `cache_tool`, `ds-bridge` (the test bridge for the page). |
| `apps/desktop/src-tauri` | The Tauri shell: command wrappers, `menu`, window, open, about, self-test. |
| `static/` | The editor page, classic scripts: `state`, `persist`, `edit`, `transport`, `render`, `session`, `backend`, `lanes-gl`, `tiles-worker`, `keymap`, `help`, `shell`, and `app.js` (feature code and boot). |
| `tests/` | pytest: the page against `ds-bridge` (`e2e_app/`), the frozen references (`golden/`), parity and export tests, the JS coverage gate. |
| `tools/` | Research and check scripts; `identity.py` is the before and after gate of the refactoring. |
| `docs/` | User guide, developer guide, release notes, ADRs, design notes, engineering plan. |

## Invariants

The structure keeps these rules. Most of them are absences: a thing that must not appear. A
rule marked "target" is not true yet; the refactoring plan names the step that makes it true.

- `drumspike-core` has no dependency on Tauri or on `drumspike-app`. Holds today.
- `drumspike-app` has no dependency on Tauri. Holds today. Only `apps/desktop/src-tauri` names Tauri.
- Core code does not take `serde_json::Value` as a domain type. Detection functions take samples
  and parameters and return data. Target (R1, R2): a few uses remain in `backend`, `layer` and `project::source`.
- Core reads no `DRUMSPIKE_*` detection variable. The environment is read at the edges:
  `ServiceConfig::from_env` and the devtools binaries. Target (R6): 15 detection names are read in core today.
- No module cycle inside core. Holds since R1: `backend` no longer calls `analyze`; the note tables live in `role`.
- One `Role` type in core (`role::Role`, also `detect::onset::Role`). Holds since R1. The tempo detection keeps its own
  `BeatVoice` (a hi-hat counts as Other there).
- Detection functions are pure: the same samples and parameters give the same bits. A move or a
  split never reorders a float operation.
- A rule marks a hit; it does not delete it. An absent mark is not the same as a false mark.
- The source audio is never modified. Project format 1 stays loadable.
- The page reaches the app only through `backend.js`. Feature scripts reach each other only through
  `*Init(deps)` hooks wired at boot. Load order is the dependency order: `state`, then util, then the
  page modules, then the platform scripts, then the view, then the features, then `app.js`.
- Every rule that changes detection output has a version. The project records the versions it used,
  so an old project reproduces.
- No audio, real session name or private path in git. All real test data stays on the owner's machines.

## The glossary

One word means one thing. The code name is the place to look.

| Term | Meaning | Code name |
|---|---|---|
| Project / Session | A `.drumspike` document: tracks, sources, events, tempo map, recorded rule versions. | `project::Document` |
| Track | One mic recording in a project. | `project::Track` (document), `peaks::Track` (pyramid) |
| Source | The audio file of a track plus its identity. Never modified. | `project::Source` |
| Role | The instrument a track records: kick, snare, tom, hat, cymbal, other. Unknown names become Other on purpose. | `role::Role` |
| Detector note | MIDI note the detector uses per role: 35/38/47/42/49, Other 35. | `Role::detector_note`, `role::role_note` |
| Default note | MIDI note the user sees per role: 36/38/48/42/49, Other 36. Equal to `ROLE_DEFAULT_NOTE` in `app.js`. | `Role::default_note` |
| Onset (candidate) | A spectral-flux peak at 44.1 kHz with a coarse time. | `detect::onset::Candidate` |
| Peak pick | Choose candidates above the role thresholds. Sensitivity scales the thresholds. | `detect::peaks` |
| Refine | Move a coarse onset to the first sample of the attack at the native rate. | `refine`, `RefineRule` v1-v7 |
| Hit | A refined time plus velocity plus class. Five stage types exist today. | `refine::Hit`, `backend::DetectedHit`, ... |
| Event | A hit as stored in the project, with marks in `extra`. | `project::document::Event` |
| Velocity | Peak dB mapped to MIDI 1..127. | `velocity` |
| Mark | A per-event key: cut, bleed, filtered, keep, deactivated, flam, learned, roll, rescue, stage8. | `policy::Mark` |
| Bleed | Sound of one drum on another drum's mic. | `detect::attribution` |
| Cross bleed | Session rule: arrival time and level decide which mic owns a kick, snare or tom hit. | `detect::cross_bleed`, app step |
| Tom bleed | Session rule: transfer gain between tom mics, versions 0-5. | `detect::tom_bleed` |
| Ghost gate | Session rule for overhead ghost hits (A1). | app `ghost_gate` |
| Flam | Two strokes a few ms apart; the rule adds the second hit. | `detect::flam` |
| Roll, Kick rescue | Special-case detectors inside onset and analyze. | `detect::onset`, `analyze` |
| Cut (loudness cut) | Hits under the loudness threshold are marked cut, not deleted. | app `histogram` |
| Re-pick | Recompute cut, tom bleed and flam after a sensitivity change, with the recorded rule versions. | app `histogram` |
| Reanalyze selection | Detect again inside a time range. Runs no session step. | app `reanalyze` |
| Learn This / Forget This | Mark a pattern; similar hits are added or removed. | app `learn*`, core `similarity` |
| Rule version | The version of a rule used for an analysis. Recorded so old projects reproduce. | `RefineRule`, `RULE_VERSION` |
| Pin | A forced rule version from the environment (tests, research). | `ServiceConfig` fields |
| Golden | Frozen reference outputs from the retired Python engine. | `tests/golden/` |
| Identity snapshot | Before and after hash of all outputs of the engine. | `tools/identity.py` |
| ENST | Public drum dataset used for the detection decision number. | `tools/eval/enst*.py` |
| Real sessions | Internal Pro Tools sessions with reviewed references. Internal only. | `tools/learn_eval.py`, `tools/rescue_real.py` |

## The analysis pipeline

```
 1 Decode        io            read WAV/BWF, downmix, resample to 44.1 kHz for detection
 2 Features      detect        spectral flux per role band
 3 Onsets        detect::onset candidates with per-role params (delta rule, A5 gain)
 4 Pick          detect::peaks thresholds from Sensitivity
 5 Refine        refine        attack sample at native rate
 6 Velocity      velocity      peak dB -> 1..127
 7 Track flags   attribution   per-hit band flags, roll, kick rescue
 8 Store         app jobs      events + recorded rule versions into the project
 9 Session steps app steps     fixed order: cross bleed -> ghost gate -> tom bleed -> flam
10 User passes   app           re-pick (cut + tom bleed + flam, recorded versions),
                               reanalyze selection (stages 1-7 only), Learn
11 Export        midi, spike   MIDI bytes, spike WAVs, layers
side branch      tempo_detect  kick/snare hits -> integer BPM suggestion
```

Stages 1 to 7 run per track and need only that track. Stage 9 needs all tracks, because one
drum leaks into the other microphones. Each stage has its own tests. The identity snapshot
checks the end of stages 8, 9, 10 and 11.

## Gates that guard the structure

- Unit tests per module (`cargo test --workspace`), the page tests against `ds-bridge`
  (`tests/e2e_app`), per-file JS coverage of at least 85 percent.
- The frozen references in `tests/golden/` and the parity binaries prove that detection, velocity
  and MIDI bytes are exact.
- The ENST gate (`tools/eval/enst_gate.py`) and the real-session check guard the detection rates.
- The identity snapshot (`tools/identity.py snapshot` and `compare`) hashes events with all
  marks, times, velocities and exported bytes for the golden corpus, ENST items and the real
  sessions, including a re-pick and a reanalyze case. A refactoring step must show zero difference.
  The manifests live in `~/.cache/drumspike/identity/`, never in git.
