# Drum Spike engineering plan: modular code, fewer but better tests, a "refactored" test release

Status: DONE. The refactoring is finished and merged. The numbers before and after, and the steps
not done, are in `docs/review/refactor-baseline.md`; `ARCHITECTURE.md` describes the result. This file
is the record of the plan and is not updated any more.

Was: adopted 2026-11-03 on branch `refactor/structure` (base: main 040e863, v0.9.19). Analysis read main at 858c117.
Inputs: `research.md` (role models and authors), the four reviews (`core.md`, `app.md`, `frontend.md`, `tests.md`) and 21 verified claims (T1-T11, R1-R10).
Rule for this draft: only claims with verdict "confirmed" or "adjust" are in the plan. For "adjust" the plan uses the adjusted claim and its keep list. Refuted claims are in Appendix A.

## 1. Summary

1. The code is better than its size suggests. It has one application service, one command registry, one policy module and pure detection rules in core. The problems are size, duplication and scattered rule plumbing.
2. We keep the 3 crates and the Tauri shell. We do not add crates. We regroup modules inside them and fix the dependency direction.
3. Every refactoring step is structure-only or a Parallel Change. Each step has a gate that proves identical output.
4. The new main gate is an identity snapshot: events, times, velocities, marks and MIDI bytes on the golden corpus, ENST and the 3 internal sessions, hashed and compared before and after.
5. Tests: about 450 of 1,451 Python tests go (about 31 %), mostly research prototypes, parked media and pixel tests. Rust test count stays about the same. The reviews' "65 % fewer" target did not survive verification.
6. tools/ shrinks from about 31,000 to about 18,000 Python lines. Golden data can shrink from 13 MB to about 3.7 MB (owner decision).
7. The biggest readability wins: one Role type, refine.rs split by mechanism, the 373-line cross-bleed function split into phases, one dispatch table, app.js split into feature scripts, menu.rs split.
8. Four AGENTS.md rules make tests grow. The owner lifted them for this branch; section 4.3 holds the replacement text (T11).
9. Work happens on branch `refactor/structure`. The release is an Apple-silicon-only prerelease, for example `v0.10.0-refactor.1`, never marked "latest".
10. We measure file sizes, function sizes, test counts and `just ci` time before and after.

## 2. Principles we adopt, and where we do not follow an author

### 2.1 Adopted

| Principle | Source (research.md) | How we apply it |
|---|---|---|
| Separate structure changes from behaviour changes | Beck, Tidy First? (2, Authors/Beck 3) | One commit is either a move/rename or a behaviour change, never both. The refactored release has no behaviour change. |
| Four rules of simple design, "fewest elements" | Beck rules (Authors/Beck 2) | No trait, crate or generator with one user. Example: no `RefineStrategy` trait (R5), no menu codegen (R10). |
| Test Desiderata: structure-insensitive, predictive | Beck (3, Authors/Beck 1) | A test must name a domain invariant or a user-visible result. Tests that only pin a deleted script go. |
| Characterization tests before a change | Feathers (2) | Step 0 builds the identity snapshot. Each refactor step adds its own small pinning test first. |
| Parallel Change (expand, migrate, contract) | Fowler (Authors/Fowler 1) | All rule code: add the new type beside the old, move callers one per commit, delete the old last. |
| Branch by Abstraction | Fowler (Authors/Fowler 2) | `AnalysisStep` for the analysis job only (R2). |
| Strangler Fig | Fowler (2, Authors/Fowler 3) | app.js: move one section per PR into its own script (R9). |
| Classical tests, real collaborators, no mocks | Fowler test shapes, matklad How to Test (1, Authors/Fowler 5) | Keep temp-folder integration tests. Add no mock framework. |
| Dependency rule: dependencies point inward | Martin (Authors/Martin 1) | shell -> app -> core. Cargo enforces the crate direction. We also remove the one module cycle in core (R1). |
| SRP as "one reason to change" | Martin (Authors/Martin 2), Beck cohesion (Authors/Beck 4) | Split files by what changes together (a rule mechanism with its constants), not by line count. |
| Pipes and Filters, in process | Hohpe (Authors/Hohpe 2) | The analysis is named as stages (section 3.4). Function composition, no bus. |
| Architecture sells options | Hohpe (Authors/Hohpe 1) | Keep only options that will be used: the rule-version record, the serde DTO seam. Do not buy a plugin system or a second UI. |
| Ubiquitous language | Evans (4) | One glossary (section 3.3) used in code, UI and docs. |
| Codemap with invariants | matklad ARCHITECTURE.md (1) | Add `ARCHITECTURE.md`: coarse modules, invariants stated as absences, layer borders. |
| Functional core, imperative shell | Bernhardt (4) | Split long step functions into gather, decide, apply where the lock boundaries allow it (R2). |

### 2.2 Where we deliberately do not follow an author

| Author / idea | Why not here |
|---|---|
| Martin: functions of 4 to 6 lines, tiny classes | A cohesive DSP pass of 60 to 100 lines is easier to judge in one piece (research.md, "Where authors conflict"). We split only by phase or mechanism. |
| Martin: four-ring Clean Architecture, many interfaces, mocks | One team, one local app. Follow matklad and Fowler: sociable tests, no ports for `fs` (Authors/Martin "Do NOT follow"). |
| Fowler: Replace Conditional with Polymorphism for rule versions | The refine versions are layered (v3 to v7 share the FM_* constants). The 7-arm `match` is the clearest dispatch (R5 verdict). |
| Fowler PoEAA: rich domain objects, repositories | No database. Plain structs and functions (Authors/Fowler "Do NOT follow"). |
| Evans/Vernon: aggregates, domain events, context maps | Overkill for a single-user app (4). We use the glossary and context names only. |
| Zed, ripgrep: many small crates | "Fewest elements". A new crate would add build and visibility cost now. Contexts become modules first (research 1, 5). |
| Beck: TDD for detector tuning | Detector outcomes are known only on real sessions. Use the golden and gate approach (Authors/Beck "Do NOT follow"). |
| Hohpe: brokers, queues, idempotent receivers | Single process. Not applicable (Authors/Hohpe "Do NOT follow"). |
| research "apply first" 4: newtypes Samples/Seconds/Db | Touches numeric code in every rule. Not verified. Backlog (Appendix B). |
| tests.md: loosen rise dB to 1e-6 | The hard constraint says times, velocities and MIDI bytes stay exact. We change no tolerance in this plan. |
| ES modules in the page now | About 1,740 test `evaluate` strings read globals. Last, behind a `window.DSDebug` shim, after this release (R9). |

## 3. Target architecture

### 3.1 Crates and dependency rule

| Crate | Responsibility | May depend on |
|---|---|---|
| `crates/core` (`drumspike-core`) | Domain and DSP: audio I/O, detection, refinement, attribution, velocity, project format, tempo, MIDI and spike export, playback engine (feature `audio`). | std and its libraries only |
| `crates/app` (`drumspike-app`) | Application service: the `Service` commands as plain functions, analysis jobs and session steps, policy, agent surfaces (call, bridge, CLI, MCP). No Tauri. | core |
| `crates/devtools` | Developer binaries: `service_tool`, `parity`, `bench`, `cache_tool`, `ds-bridge`. Not shipped. | app, core |
| `apps/desktop/src-tauri` | Tauri shell: command wrappers, menu, window, self-test. | app, core |
| `static/` | The editor page, classic scripts. | the IPC surface through `backend.js` only |

Rules (written into `ARCHITECTURE.md`):
- core never names Tauri, `crates/app` or `serde_json::Value` as a domain type. Detection functions take samples and parameters and return data.
- core reads no `DRUMSPIKE_*` detection variable after R6. Env is read at the edges: `ServiceConfig::from_env` in the app and the devtools binaries.
- app never names Tauri (true today; keep it).
- No module cycle inside core (one exists today: `analyze` <-> `backend`; R1 removes it).
- In `static/`, load order is the dependency order: `state` -> `util` -> core page modules -> platform (`backend`, `keymap`, `shell`) -> view -> features -> `app.js` (boot). Features reach each other only through `*Init(deps)` hooks wired in boot.

### 3.2 Modules and one-line responsibilities

core (target after this release; contexts from core.md 1):

| Module | Responsibility |
|---|---|
| `io`, `peaks`, `cache`, `pyops` | Read and write audio, waveform pyramid and cache, Python-compatible rounding (named policies: half-even `py_round`, half-up `ms_to_samples`). |
| `dsp` (new, R7) | Filter and statistics primitives: `Biquad`, `FilterKind`, `band_sections`, `band_peak_db`, `percentile`, `np_percentile`, `ms_to_samples`, named dB helpers. |
| `role` | The one `Role` type (R1): `FromStr`, `as_str`, `ALL`, `parse_or_other`, `detector_note()`, `default_note()`; `role_note`, `class_note`, `role_classes`; role from audio. |
| `detect::{features, peaks, onset, nmfd}` | Coarse onset candidates for one track. |
| `detect::{attribution, tom_bleed, cross_bleed, flam}` | Per-hit ownership and special-case rules. `cross_bleed` gets phase functions and a `CrossBleedContext` (R2 core part). |
| `refine::{reference, reach, walk, edge, track}` | Coarse time to the attack sample, by mechanism (R5). `refine.rs` keeps all public paths as re-exports. |
| `rules` (new, R6) | `RuleSet` (refine, bleed, flam, cross bleed, ghost gate, kick walk, delta, tempo) and `ResearchOverrides` (typed A1, A5, A6 numbers). |
| `analyze`, `backend`, `velocity` | Orchestrate detection for one track, detector backends, dB to MIDI velocity. |
| `similarity`, `agreement` | Learn similarity score, detector-refine agreement. |
| `project` | Document, sources, save, relink. Format 1 stays loadable. |
| `tempo`, `tempo_detect` | Tempo map and its inference. |
| `midi`, `spike`, `layer`, `sampler` | Outputs and one-shot alignment. |
| `transport` | Real-time playback engine (feature `audio`). |

app (target):

| Module | Responsibility |
|---|---|
| `lib` (`Service`, `ServiceConfig`) | Composition root. Env pins are named fields (R2, R6). |
| `api` | `REGISTRY`: the one description of every command, now also the dispatch source (R3). |
| `call`, `command_policy` | One name-to-handler table; agent gate and surface defaults. |
| `policy` | Event marks and precedence: `is_marked`, `mark`, `clear`, `is_manual`, `event_out` (R4 extends the R16 module). |
| `jobs` + `steps` (new) | Analysis job and the fixed `AnalysisStep` list for the job path (R2). |
| `cross_bleed`, `ghost_gate`, `tom_bleed`, `flam` | Session-step adapters: gather mics, call core, apply marks. |
| `histogram`, `reanalyze`, `learn*` | Re-pick (own explicit tom bleed + flam call), reanalyze selection (no steps), Learn. |
| `edit`, `export`, `tracks`, `roles`, `lifecycle`, `registry`, `autosave`, `transport` | Commands over the open project. |
| `bridge`, `cli`, `mcp`, `headless`, `agent` | Driving adapters over `call`. |

desktop shell: `lib.rs` (wrappers, one handler list macro), `menu/{keymap, items, build, sync, macos}` (R10).

static/ (after R9 part A): `state`, `util` (new), `persist`, `edit`, `transport`, `render`, `session`, `backend`, `lanes-gl`, `tiles-worker`, `keymap`, `help`, `shell`, then feature scripts `export`, `addtracks`, `mixer`, `histograms`, `learn`, `layer`, `velocity`, `followstrip`, `tempomap`, `markers`, `wave`, `commands`, `files`, `review`, `analysis`, `plugins`, `status`, `inspector`, `tracks`, and `app.js` (boot, global handlers, wiring; about 400 to 600 lines).

### 3.3 Ubiquitous language (glossary)

| Term | Meaning | Code name |
|---|---|---|
| Project / Session | A `.drumspike` document: tracks, sources, events, tempo map, recorded rule versions. | `project::Document` |
| Track | One mic recording in a project. | `project::Track` (document), `peaks::Track` (pyramid) |
| Source | The audio file of a track plus its identity. Never modified. | `project::Source` |
| Role | The instrument a track records: kick, snare, tom, hat, cymbal, other. Unknown names become Other on purpose. | `role::Role` (one type after R1) |
| Detector note | MIDI note the detector uses per role: 35/38/47/42/49, Other 35. | `Role::detector_note` |
| Default note | MIDI note the user sees per role: 36/38/48/42/49, Other 36. Equal to `ROLE_DEFAULT_NOTE` in app.js. | `Role::default_note` |
| Onset (candidate) | A spectral-flux peak at 44.1 kHz with a coarse time. | `detect::onset::Candidate` |
| Peak pick | Choose candidates above the role thresholds. Sensitivity scales the thresholds. | `detect::peaks` |
| Refine | Move a coarse onset to the first sample of the attack at the native rate. | `refine`, `RefineRule` v1-v7 (v7 "Visible" is current) |
| Hit | A refined time plus velocity plus class. Five stage types exist today; renaming is backlog. | `refine::Hit`, `backend::DetectedHit`, ... |
| Event | A hit as stored in the project, with marks in `extra`. | `project::document::Event` |
| Velocity | Peak dB mapped to MIDI 1..127. | `velocity` |
| Mark | A per-event key: cut, bleed, filtered, keep, deactivated, flam, learned, roll, rescue, stage8. Absent is not the same as false. | `policy::Mark` (R4) |
| Bleed | Sound of one drum on another drum's mic. | `detect::attribution` |
| Cross bleed | Session rule: arrival time and level decide which mic owns a kick, snare or tom hit. | `detect::cross_bleed`, app step |
| Tom bleed | Session rule: transfer gain between tom mics, versions 0-5. | `detect::tom_bleed` |
| Ghost gate | Session rule for overhead ghost hits (A1). | app `ghost_gate` |
| Flam | Two strokes a few ms apart; the rule adds the second hit. | `detect::flam` |
| Roll, Kick rescue | Special-case detectors inside onset/analyze. | `detect::onset`, `analyze` |
| Cut (loudness cut) | Hits under the loudness threshold are marked cut, not deleted. | app `histogram` |
| Re-pick | Recompute cut, tom bleed and flam after a sensitivity change, with the recorded rule versions. | app `histogram` |
| Reanalyze selection | Detect again inside a time range. Runs no session step. | app `reanalyze` |
| Learn This / Forget This | Mark a pattern; similar hits are added or removed. | app `learn*`, core `similarity` |
| Rule version | The version of a rule used for an analysis. Recorded so old projects reproduce. | `RefineRule`, `RULE_VERSION`, `RuleSet` |
| Pin | A forced rule version from the environment (tests, research). | `ServiceConfig` fields |
| Golden | Frozen reference outputs from the retired Python engine. | `tests/golden/` |
| Identity snapshot | Our new before/after hash of all outputs (section 5, step 0). | `tools/identity.py` |
| ENST | Public drum dataset used for the detection decision number. | `tools/eval/enst*.py`, `enst_fast_gate.rs` |
| Real sessions | The 3 internal Pro Tools sessions with reviewed references (`refs.json`). Internal only. | `tools/learn_eval.py`, `tools/rescue_real.py` |

### 3.4 The analysis pipeline as stages

```
 1 Decode        io            read WAV/BWF, downmix, resample to 44.1 kHz for detection
 2 Features      detect        spectral flux per role band
 3 Onsets        detect::onset candidates with per-role params (delta rule, A5 gain)
 4 Pick          detect::peaks thresholds from Sensitivity
 5 Refine        refine        attack sample at native rate (RuleSet.refine)
 6 Velocity      velocity      peak dB -> 1..127
 7 Track flags   attribution   per-hit band flags, roll, kick rescue
 8 Store         app jobs      events + recorded rule versions into the project
 9 Session steps app steps     fixed order: cross bleed -> ghost gate -> tom bleed -> flam
10 User passes   app           re-pick (cut + tom bleed + flam, recorded versions),
                               reanalyze selection (stages 1-7 only), Learn
11 Export        midi, spike   MIDI bytes, spike WAVs, layers
side branch      tempo_detect  kick/snare hits -> integer BPM suggestion
```

Each stage keeps its own tests. The identity snapshot checks the end of stages 8, 9, 10 and 11.

## 4. Test strategy

### 4.1 The three layers

**Layer 1: domain unit tests in Rust (fast, the bulk).**
- Detection: `detect/onset/tests.rs`, `detect/peaks`, `detect/features`, `detect/attribution`, `detect/tom_bleed/tests.rs` (29), `detect/cross_bleed/tests.rs` (26, 1,492 lines, unchanged during R2), `detect/flam/tests.rs` (14, plus the ported 44.1 kHz file-ends case), `refine/tests.rs`, `velocity`, `analyze/tests.rs`, `role/tests.rs`, `similarity/tests.rs`, `agreement/tests.rs`.
- Output: `midi/tests.rs`, `spike.rs`, `io/*`, `tempo`, `tempo_detect/tests.rs`, `core/tests/tempo_detect.rs`.
- Data safety: `project/*/tests.rs`, `core/tests/project_cross_load.rs`, `core/tests/robustness.rs`, `app/tests/{export_safety, lifecycle, edit, relink, read_only, learn, reanalyze, flam, layer, api, cross_load}.rs`.
- Process safety: `devtools/tests/lock.rs`, `exit_jobs.rs`.
- New in this plan: R1 role/note tables and serde round trip, R5 refine golden (7 rules x 2 peak margins x lead-in), R7 `to_bits` helper tests, R3 registry "one handler per row" test, R10 menu report snapshot, `level_calib.rs` un-ignored if it passes.

**Layer 2: golden and ground truth (exact, run per detection or refactor step).**
- `tests/test_golden.py` as the single golden regression, extended by T8 with the check_refine and check_midi cases and the rule-option test.
- Identity snapshot (new, section 5 step 0).
- ENST: `crates/core/tests/enst_fast_gate.rs` (with `tools/eval/dump_tom_map.py` as setup), `tools/eval/enst_gate.py` + baseline, `tests/test_eval_enst_gate.py` (merged with `test_eval_enst.py`, every test kept).
- Real sessions (internal, not in CI): `tools/learn_eval.py baseline`, `tools/rescue_real.py`, `core/tests/backend_check.rs`, `tempo_detect_real.rs`, `tools/eval/role_enst.py` (role suggestion gate), `tools/eval/enst_tom_fp.py` (open package 8 gate).
- End-to-end onset: `tests/test_calib_kit.py:177` (caught the #239 regression on 2026-11-01).
- Exports: `tests/test_exports.py`, `test_layer_exports.py` with `tools/check_exports.py`.
- Specifications kept as Python references: `fix_metric`, `firm_fixes`, `onset_guard`, `onset_early`, `miss_causes`, `snare_onset`, `calib_kit` with their tests, and `test_rust_kick_walk.py`.

**Layer 3: e2e through ds-bridge (the page, slower).**
- Core workflows: `test_ready`, `shell`, `lifecycle`, `tracks`, `editor`, `export`, `export_dialog`, `save_integrity`, `relink`, `import`, `learn`, `learn_filter`, `tempo_map`, `undo_history`, `playback`, `review_queue`, `layer`, `sampler`, `agent_parity`.
- Only-cover files from the verdicts (kept): `infoline`, `event_policy_table`, `help`, `main_counter`, `modifier_cursors` (2 tests), `bridge_stdin_exit`, `feedback`, `menu_plain_keys`, `tab_keys_native`, `ptkeys`, `keys` (3 tests), `ampnudge`, `plugins`, `track_selected`, `menus`, `mixer`, `velocity_lane`, `onset_view`, `audition`, the undo regression files (`undo_r05`, `undo_exact`, `flam_undo`), `range_edits`, `learn_add`, `learn_guard`, `learn_forget_this`, `toggle_hit`, `follow_strip`, `histograms`, `inspector`, `inspector_sections`, `slice`, `reanalyze_selection`, `transport_mode`, `subtracks`, `tracks_remove`, `tempo_bar1`, `tempo_drag`.
- Plus `tests/js_unit/*` (grows with R9) and `test_keymap.py`.
- Repo hygiene guards: `test_ci_scope`, `test_release_cut`, `test_licence_check`, `test_third_party`, `test_check_private`, `test_check_paths` (new, from T9), `test_guide` (renamed from `test_media_guide`), `test_docs_wording` (merged), `test_site` (merged), `test_r17_interfaces`, `test_e2e_bridge_guard`, `test_pre_commit_hook`, `test_media_demo_session`.

### 4.2 What gets deleted or merged (verified groups)

| Id | Action (adjusted claim) | Files | Tests / lines removed | Preconditions |
|---|---|---|---|---|
| T1a | Delete tom/flam prototypes that nothing kept imports. | tools: `tom_bleed_sk`, `tom_bleed_cd`, `tom_pairs`, `tom_pair_risk`, `flam_split`, `flam_pairs`; tests: their 6 tests plus `test_rust_tom_bleed`, `test_rust_tom_pairs`, `test_rust_flam` | part of T1 total | Port `test_rust_flam.py::test_44k1_and_the_ends_of_the_file` to `detect/flam/tests.rs`. Remove the lazy `flam_pairs` import in `firm_fixes.py:295`. Remove parity subcommands `tom-bleed`, `tom-pairs`, `flam` and stale doc comments. |
| T1b | Move tables, then delete the rest. | Move `tom_bleed_new.SESSIONS` and `kick_onset.FILES` (and what `snare_onset` needs from `tom_fixes`) into `tools/sessions.py`. Then delete `tom_bleed_new`, `tom_bleed`, `kick_onset`, `tom_fixes` and `test_tom_bleed`, `test_tom_bleed_new`, `test_tom_fixes`. | T1 total: about 151 tests, 2,170 test lines, about 7,000 tools lines | `fix_metric.songs()` and `kick_files()` run once unpatched on the internal host. Keep `test_rust_kick_walk.py`. |
| T2 | Delete leaf scripts only. | `learn_clicks`, `kick_miss_causes`, `layer_eval`, `audio_spikes` and tests; optional `calib_hyp`, `roll_fill` | 24 tests, 363 lines (optional +26 tests, 363 lines); tools about 2,900 lines | `calib_hyp`: first confirm a Rust test covers the rule 2 to rule 3 onset gain. Update `tools/README.md`, `docs/review/detector-plan.md:147`. |
| T3 | Delete 16 e2e files, fold 2. | Delete `tile_seam`, `loaded_10fps`, `follow_zoomed`, `playhead_steps`, `native_feel`, `toolbar_narrow`, `toolbar_name`, `row_heights`, `trackheight`, `zoom_toggle`, `zoom_presets`, `lane_polish`, `wait_until`, `halfspeed`, `view_toggles`, `vel_hint`. Fold `suspend_follow` and `tab_deactivated` into one test each in a kept file. | about 58 tests, about 2,050 lines | T11 per-file coverage measured. |
| T4 | Delete 12 e2e files. | `declutter`, `other_mics`, `head_tart_overlap`, `markers`, `clear_recent`, `flag_counts`, `start_no_warning`, `overview`, `settings_numbers`, `layer_dialogs`, `playreturn`, `add_snap` | 39 tests, 1,030 lines | Move `test_keypreset` helpers to `app_support.py`. Coverage measured. |
| T5 | Layout only, no test removed. | Move shared helpers to `app_support.py`. Join `flam_page` + `kick_rescue_page` + `roll` into `test_page_tags.py`; `learn_names`, `learn_hist_labels` into `test_learn.py`; `export_again` into `test_export_dialog.py`; `track_role` into `test_tracks.py`. | 0 tests, about 6 files | Keep `tests/e2e_shards.json` balance: do not merge into the x86 shard file. Original claim refuted (Appendix A). |
| T6 | Trim renderer internals only. | Parts of `test_frames`, `test_render`, `test_tiles`: layout-read probes, 60 Hz cap, GL vs 2D agreement percent, texture and LRU counters. | est. 15 to 25 tests, 400 to 700 lines | Move helpers first; keep `test_fallback_without_webgl2`, worker-host tests, #110 and project-switch tests. Measure coverage of `lanes-gl.js`, `tiles-worker.js`, `render.js`. Original claim refuted. |
| T7 | Delete probes, examples, side tools. | Rust: `enst_contrast_probe`, `enst_scale_probe`, `quiet_gain_probe`, `level_probe`, `false_hit_probe`, `probe412_tempo`, `tempo_detect_bar1_probe`, `speed`; examples `nmfd_eval`, `placement_bias`, `lead_in_check`, `spike_len`, `clock_probe` (+ Cargo.toml `[[example]]`). Python: `eval/nmfd.py`, `eval/enst_soft_report.py` + test, `eval/spike_len.py`, `clock_residuals.py`. Merge `test_eval_enst.py` into `test_eval_enst_gate.py`. | about 10 Rust tests, 1,460 Rust lines; 2 Python tests, about 1,010 Python lines | Keep `dump_tom_map.py`, `role_enst.py`, `enst_tom_fp.py`, `level_calib.rs` + `level_calib_render.rs` (run with `--ignored`; un-ignore if it passes). |
| T8 | One golden test. | Move check_refine (edges, windows, bench) and check_midi (11 synthetic + corpus) cases and the `test_rust_analyze` corpus-equality, branch-count and rule-option tests into `test_golden.py`. Then delete `test_rust_parity.py`, `test_rust_analyze.py`, the 11 `check_*.py`, `fit_resample_kernel.py`, `x86run.sh`, the justfile `parity` recipe. Inline 2 helpers into `exit_flow.py`. Delete frozen kinds with no reader incl. `analyze.json`. | about 8 Python tests merged, about 1,900 tools lines; golden 13 MB -> about 3.7 MB | Owner decision: `tests/golden/*` is on the "Do not touch" list. The identity snapshot replaces `check_analyze.py` as the "byte-identical" gate named in `detector-plan.md:213` before it goes. |
| T9 | Media: split, keep the live guards. | Rename `test_media_guide.py` -> `test_guide.py`. Keep `test_media_demo_session.py`. Move 8 `check_paths` tests to `test_check_paths.py`. Delete `test_media_tts`, `test_media_compose`, `test_media_explainers`, `test_media_scripts` and the record scenario tests. Delete `tools/media/compose.py`. | about 146 tests, about 2,120 lines (36 of them run by default) | Keep `record.py`, `audio_spike.py`, `physics.py`, explainer renderers and `tts.py` until the owner accepts frozen landing-page videos. Original claim refuted. |
| T10 | Docs/site guards merged. | `test_docs_wording` 13 -> about 5 tests (keep planted-fault and #402 tests); fold `test_ui_spike_texts` into it. `test_site`: drop the em-dash and banned-wording tests (covered by `check_wording.py`), merge link checks, keep hosts, licence, render, build_site; fold `test_help_build` (both tests). Keep `test_r17_interfaces`. | about 13 tests, 250-300 lines | After T9. |

Totals (Python): about 450 of 1,451 tests and about 9,000 test lines go. e2e: about 117 of 733 tests (16 %). tools/: about 13,000 of 31,004 lines. Rust: about 10 tests go, about 30 characterization tests are added.

### 4.3 Replacement rules for AGENTS.md (T11, owner decision)

Replace the rules in all copies: AGENTS.md:65-66 and :75, CONTRIBUTING.md:31-35, docs/DEVELOPING.md:234-237, tools/README.md:25. Proposed text:

1. A test names a domain invariant or a user-visible result in its name or first line.
2. A test may be deleted when it only pins a removed script, or a fixed bug that another test covers. The PR names the test and the covering test.
3. Never delete or weaken: `test_golden`, `frozen.py`, `golden/*`, `test_exports`, `test_layer_exports`, the identity snapshot, the ENST gate, the real-session check, tests of source-audio safety, project format 1 loading and `check_private`.
4. JS coverage stays per file, not as a total. The floor is a measured value in `tests/jscov.py:20` (one floor such as 70, or a ratchet: no file falls more than N points below main). A new script in `static/` enters `JS_FILES` at the same floor. Docs list the same 11 files as `JS_FILES`.
5. "Every public Rust function has a unit test" becomes "Every public Tauri command and every core entry point has a test".
6. tools/: "A script has a test or a documented command" (the AGENTS.md:24 wording).

The ci.yml js-coverage job needs no logic change. Its "85 %" comments are a maintainer-only edit.

## 5. Refactoring roadmap

Gates used below:
- **G-ID** identity snapshot (new): hash of serialized events (key order, absent vs false), times, velocities and MIDI bytes for the 18 golden WAVs, ENST items and the 3 internal sessions, plus one re-pick, one reanalyze and one Learn case per session. Zero diff.
- **G1** fmt, `cargo clippy --workspace --all-targets --features drumspike-core/audio -- -D warnings`, `cargo test --workspace --no-fail-fast` on an Apple-silicon Mac (Linux clippy misses macOS code).
- **G2** `uv run pytest -q` without e2e. **G-E2E** the e2e suite plus the per-file JS coverage gate.
- **G-ENST** `enst_fast_gate.rs` and `enst_gate.py` against the baseline. **G-REAL** `learn_eval.py baseline` and `rescue_real.py`.
- **G-MOVE** `git diff --color-moved=zebra` shows only moved lines; each moved line appears once; no duplicate definitions.

Size: S = 1 PR under 300 changed lines; M = 1 to 3 PRs; L = more than 3 PRs or over 1,500 lines. Risk is detection risk plus merge risk.

### Phase 0: safety net and decisions (structure only, no product code)

| # | Step | Pattern | Files | Gate | Size | Risk |
|---|---|---|---|---|---|---|
| 0.1 | Record baseline metrics (section 7) into `docs/review/refactor-baseline.md`. | Measure first | docs | numbers printed with counts | S | none |
| 0.2 | Build the identity snapshot: `tools/identity.py snapshot|compare` over `service_tool analyze`, `export`, a re-pick and a reanalyze script; manifest in `~/.cache/drumspike/identity/<sha>.json`, never in git. Planted-fault test: change one velocity, compare must FAIL. | Characterization test (Feathers) | `tools/identity.py`, `tests/test_identity.py`, maybe a `service_tool snapshot` subcommand | planted fault fails; two runs on the same head are equal | M | low |
| 0.3 | Take the baseline at the branch base on an Apple-silicon Mac, with env pins unset and with `DRUMSPIKE_DELTA_RULE=0`, `DRUMSPIKE_TEMPO_RULE` 1/2/3. | Golden master | cache only | manifest written | S | none |
| 0.4 | Write `ARCHITECTURE.md` (codemap, invariants, glossary of 3.3, pipeline of 3.4). Point AGENTS.md to it. | Codemap (matklad) | docs | `check_wording`, no em-dash | S | none |
| 0.5 | Owner decisions: T11 rules, T8 golden edits, T9 frozen videos, prerelease naming, detection-ticket freeze on hot files during phases 2-3. | | | owner answer | | |

### Phase 1: test cleanup (after T11)

| # | Step | Pattern | Gate | Size | Risk |
|---|---|---|---|---|---|
| 1.1 | T11 rule text in all copies. | Rule change | `check_wording` | S | low |
| 1.2 | T7 Rust probes, examples, side tools; merge `test_eval_enst`. Run `level_calib` with `--ignored` once. | Remove Dead Code | G1, G2, G-ENST setup still works (`dump_tom_map.py`) | M | low |
| 1.3 | T2 leaf scripts; T1a (port the flam case first, in its own commit). | Remove Dead Code | G1 (`detect/flam` new test), G2 collects without ImportError, `grep` for deleted module names = 0 | M | low |
| 1.4 | T1b sessions table move, then delete. | Move Function, then Remove Dead Code | G2; `fix_metric score` runs unpatched on the internal host with the same numbers as before | M | medium (silent yardstick break) |
| 1.5 | T8 golden consolidation in its 7 sub-steps; identity tool replaces `check_analyze.py` in the detector work packages. | Merge tests, Remove Dead Code | G2; the moved cases fail with a planted fault; golden size printed | L | medium |
| 1.6 | Measure per-file JS coverage with and without the T3/T4/T6 candidates. | Measure first | coverage table in the PR | S | none |
| 1.7 | T3 and T4 (moves of `midi_note_ons` and `test_keypreset` helpers first). | Remove Dead Code | G-E2E, coverage at or above the floor | M | low |
| 1.8 | T5 helper move and the 4 small joins. | Move Function | G-E2E, same test count | M | low |
| 1.9 | T6 renderer trims. | Remove Dead Code | G-E2E, coverage floor for the 3 render files | S | low |
| 1.10 | T9, then T10. | Move, Remove Dead Code | G2 (default run), media marker run once | M | low |

### Phase 2: core structure (Tidy First, compiler-checked)

| # | Step | Pattern | Files | Gate | Size | Risk |
|---|---|---|---|---|---|---|
| 2.1 | R8a: 12 test-only items under `#[cfg(test)]` (10 named + `peaks_level`, `Track::all_samples`); delete `KICK_WALK_VERSION`; decide `set_lead_in` and `refine_foot_tuned` (no production caller after T7; `LEAD_IN` defaults false). Fix doc rot: `refine.rs:726-728`, `analyze.rs:62`, `pick_with` doc, PyO3 lines in `lib.rs`. | Hide Method, Remove Dead Code | `analyze.rs`, `detect/{onset,resample,peaks,attribution}.rs`, `transport/mod.rs`, `project/mod.rs`, `refine.rs`, `lib.rs` | G1, G-ID | S | none |
| 2.2 | R7: create `dsp.rs`; move `Biquad`, `FilterKind`, `band_sections`, `band_peak_db`, `percentile`, `np_percentile`, `ms_to_samples` with re-exports; switch `spike.rs`, `flam.rs`, `cross_bleed.rs`, `tom_bleed.rs`; merge only proven-identical copies (`level_at`+`level_db`, `broad_db`+`broadband_peak_db`, the two Option medians); add named dB helpers per floor policy; leave the onset select-median, `np_percentile` and power-dB sites alone. `to_bits` tests first. | Move Function, Remove Duplicate Code | listed in R7 | G1, G-ID per commit | M | medium (float bits) |
| 2.3 | R1: characterization tables (role x note, `role_classes`, tempo `from_name("hat")`, serde "Kick" and missing key); add one `Role` with PascalCase serde and `Default=Other`, `pub use` as `onset::Role`; move attribution and `role.rs`; tempo keeps a local `BeatVoice` (Hat -> Other) with `weight()`, `is_ks()`; attribution keeps `band()`; add `detector_note()` and `default_note()` (two tables, never one); `parse_or_other()` at the string edge; move `role_note`, `class_note`, `role_classes` into `role` last (breaks the cycle). | Parallel Change, Replace Type Code with Class, Move Function | `role.rs`, `detect/onset.rs`, `detect/attribution.rs`, `tempo_detect.rs`, `backend.rs`, `analyze.rs`, `detect/mod.rs`, app `roles.rs` | G1, G-ID, G-ENST | M | medium if the note tables mix; low as staged |
| 2.4 | R5: refine golden first (Refined bits for 7 rules x {FM_PEAK_DB, kick margin} x lead-in {off, on}, plus `first_motion_mid`, `first_motion_trace`); add `BackTune{peak_db, tune, edge}` for `back_track`, `first_motion_traced`, `refine_back_peak` (removes 3 allows); move code to `refine/{reference, reach, walk, edge, track}.rs` with all public paths re-exported; keep the 7-arm match; keep `refine_sr` name (or a deprecated alias); keep `peak_db == FM_PEAK_DB` exactly. | Introduce Parameter Object, Extract Module | `refine.rs`, `refine/*` | G1, G-MOVE, G-ID, golden on arm64 | M | low with the golden |
| 2.5 | R2 core: phase functions for `cross_bleed_prior_session` (373 lines); structs for `MeasuredRow` and the `measure_pair` tuple; `cross_bleed_with(mics, &CrossBleedContext)` with the 4 old functions as wrappers; no change to float operation order. | Split Phase, Replace Tuple with Struct, Introduce Parameter Object | `detect/cross_bleed.rs` | G1 (`cross_bleed/tests.rs` unchanged), G-ID, G-REAL | M | medium (hot file) |
| 2.6 | R8b: `pub(crate)` per module (refine, then similarity, transport, backend, spike, midi, ...). Count `core/src/bin`, `core/examples`, `core/tests` as external. | Hide Method | per module | G1 with `--features audio`, public-surface list | M | low (merge conflicts) |
| 2.7 | R8c: shrink the `lib.rs` re-export block to what callers import from the root; document module paths as the API (also in `crates/core/README.md`). | Hide Method | `lib.rs`, `bench.rs` | G1 | S | low |

### Phase 3: rule plumbing (Parallel Change)

| # | Step | Pattern | Files | Gate | Size | Risk |
|---|---|---|---|---|---|---|
| 3.1 | R6 steps 1-2: `DeltaRule`, `TempoRule` enums beside `RefineRule`; `RuleSet` resolved once per analysis or re-pick (pin, else record, else default); `ResearchOverrides` built in `ServiceConfig::from_env` and devtools, passed into `OnsetParams` and cross-bleed params (removes the env read per candidate at `onset.rs:948`); `delta_rule()`, `tempo_rule()` stay as wrappers until no caller is left; tests that use `set_var` (`lib.rs` tests) move to explicit params. Runtime env vars stay out of scope. | Introduce Parameter Object, Replace Global with Parameter | `onset.rs`, `tempo_detect.rs`, `cross_bleed.rs`, `analyze.rs`, app `lib.rs`, `jobs.rs` | G1, G-ID with each pin set and unset, G-ENST | L | medium |
| 3.2 | R4: `policy::Mark`, `is_marked`, `mark`, `clear` (= `shift_remove`) for Event and Value/Map; replace raw reads one file per commit; delete the exact duplicates `confidence::is_edited`, `edit::is_manual`; every call site keeps its current predicate. Golden of serialized events first. | Encapsulate Field, Parallel Change | `policy.rs`, `tom_bleed.rs`, `learn.rs`, `learn_filter.rs`, `flam.rs`, `confidence.rs`, `cross_bleed.rs`, `histogram.rs` | G1, policy truth table, G-ID (key order and absent vs false) | M | low as staged |

Not in this release: R6 step 3 (record delta and tempo in the project; changes saved bytes) and step 4 (fail-loud parsing; behaviour change). R4 predicate convergence is a detection ticket.

### Phase 4: app and shell structure

| # | Step | Pattern | Files | Gate | Size | Risk |
|---|---|---|---|---|---|---|
| 4.1 | R2 app: `AnalysisStep` (id, `analysis_version(cfg)`, `run`, `emit`) and a fixed list for the analysis job only, replacing `jobs.rs:784-843`; each step keeps its JSON shape, `v>0` gating and target type. Extract helpers inside `cross_bleed::run_one` and `tom_bleed::run_step` without moving lock boundaries or the hits-changed abort. Re-pick and reanalyze stay as they are. | Branch by Abstraction, Extract Function | `jobs.rs`, `steps.rs`, `cross_bleed.rs`, `tom_bleed.rs`, `flam.rs`, `ghost_gate.rs` | G1, G-ID incl. re-pick and reanalyze cases, G-REAL | M | medium |
| 4.2 | R3 stage A: characterization of every registry row ({} / minimal / null / wrong-type args, reply or error code; surface defaults `keep_edited` page false, agent true; `start/end` 0/`i64::MAX`). Then per-module `HANDLERS` slices and one name-to-handler map in `call::run`; `transport::dispatch` registered like the others; `call()` keeps refusing Window commands. Tests: one handler per row, no duplicates. | Replace Conditional with Table | `call.rs`, `bridge.rs`, modules with `*_COMMANDS` | G1, characterization, G-E2E, G-ID on page and agent surface | M | low |
| 4.3 | R3 stage B: one `macro_rules!` handler list with a `cfg(debug_assertions)` extra; rewrite the `tests/api.rs` text parser in the same PR. | Remove Duplicate Code | desktop `lib.rs`, `crates/app/tests/api.rs` | G1, G3 self-test | S | low |
| 4.4 | R10: split `menu.rs` into `menu/{keymap, items, build, sync, macos}.rs` with `mod.rs` re-exports; snapshot `menu::report` and NS menu rows first; then parse `path`, add `needs_project` and `kind` to keymap rows and test them against SPEC and the builder tree. | Extract Module, add drift test | `menu.rs`, `menu/*`, `static/keymap.js` | G1 (clippy on an Apple-silicon Mac), G-MOVE, menu snapshot, G3 | M | none for detection |

Not in this release: R3 stage C (Args structs; wire format and surface defaults need a spike), R2 re-pick list.

### Phase 5: frontend

| # | Step | Pattern | Files | Gate | Size | Risk |
|---|---|---|---|---|---|---|
| 5.1 | R9 A1: `util.js` right after `state.js` with `$`, `fmt`, `dScale`, `NOTE_NAMES`, `nameFromNote`, `noteFromName`, `askConfirm` (avoids the load-time TDZ error). | Extract Module | `static/util.js`, `index.html`, `jscov.py` | G-MOVE, G-E2E, `test_r17_interfaces`, `test_ready` | S | low |
| 5.2 | R9 A2: one section per PR, verbatim, in this order: export, addtracks, mixer, histograms, learn, layer, velocity; followstrip, tempomap, markers, lanes-gl glue; editor commands, files/relink, review, analysis, plugins, status/info; inspector, tracks last. Each file: `"use strict"`, own `?v=`, `JS_FILES` entry at the floor. Load-time statements keep their order or move into `*Init` in boot. | Extract Module (Strangler Fig) | `static/*.js`, `index.html`, `jscov.py` | G-MOVE, G-E2E, coverage per new file | L (about 20 PRs) | low (detection-neutral) |
| 5.3 | Measure Chromium coverage of `shell.js`, `help.js`, `splash.js`; add them to `JS_FILES` or document an exception. | Measure first | `jscov.py` | coverage table | S | none |

Not in this release: R9 part B (`token()`, `serialQueue()` with an explicit error policy, `dragSession()` for about 12 equal window drags, `modal()`). The MK/TM promise chains stay rejected after a failure today; file that as its own defect ticket.

### Phase 6: close

| # | Step | Gate |
|---|---|---|
| 6.1 | Refresh `ARCHITECTURE.md`, AGENTS.md map and `crates/core/README.md` to the new module names. | `check_wording` |
| 6.2 | Record the after metrics next to the baseline (section 7). | numbers with counts |
| 6.3 | Final G-ID, G-ENST, G-REAL, G1, G2, G-E2E, G3 on the release head. | all green, zero identity diff |

## 6. The refactored release

- **Branch:** `refactor/structure`, cut from main at the phase 0 base. Tickets branch from it (`crew/<agent>/<issue>-<slug>`) and merge back into it. main keeps taking defect fixes.
- **Sync with main:** merge main into the branch after each main merge that touches core or app. After a detection merge on main, take a new G-ID baseline at that main commit and compare the branch against it. Ask the owner to hold detection tickets on `refine.rs`, `cross_bleed.rs`, `tom_bleed.rs`, `onset.rs` during phases 2-3.
- **Build arm64 only:**
  - Unsigned build to try locally: `apps/desktop/scripts/build-dmg.sh --arch arm64` (arm64 is the default).
  - Signed and notarized (owner only; agents only `--dry-run`): `apps/desktop/scripts/release.sh --arch arm64 --publish --draft`. Unlock the Apple-silicon build Mac first (notary).
  - Then run `~/.cache/drumspike/draft-check.sh <tag>` on the draft (sums, stapler, spctl, codesign, version, lipo shows arm64 only).
- **Prerelease naming:** tag `v0.10.0-refactor.1`, title "Drum Spike 0.10.0-refactor.1 (Apple silicon test build)", asset `Drum Spike_0.10.0-refactor.1_aarch64.dmg`.
  - Open point 1: check that Tauri accepts a semver prerelease in `tauri.conf.json` for a macOS bundle. If not, keep a numeric app version (for example `0.9.90`) and put the marker only in the tag and the title.
  - Open point 2: `release.sh` has no prerelease switch. Add `--prerelease` that passes `--prerelease --latest=false` to `gh release create`, so the landing page and 0.9.x users stay on the stable release. Owner approves this script change.
  - Release notes: "No change to detection. Same hits, times, velocities and MIDI bytes as 0.9.x. Intel Macs: stay on 0.9.x."
- **What the owner should test (about 45 minutes, on copies of projects):**
  1. Open each of the 3 real sessions as copies. Run Analyze All. Export MIDI. Compare with the 0.9.18 export of the same copy: `cmp` must report no difference.
  2. Move the Sensitivity slider (re-pick). Export again and compare with 0.9.18 after the same move.
  3. Learn This and Forget This on a dense kick track. Check that the hit counts match 0.9.18.
  4. Reanalyze a selection. Undo, redo, save, close, reopen. Check that nothing is lost.
  5. Menus and keys in the Pro Tools and Cubase presets; typing in a track name does not fire commands.
  6. Mixer volume and pan survive save and reopen. Audition does not change mute, solo or the playhead.
  7. Export dialog, add tracks, tempo detect (integer BPM), relink, layer and sampler, help.
  8. Report anything that looks or feels different. Any difference in step 1 to 3 is a blocker.

## 7. Risks and what we measure

### 7.1 Risks

| Risk | Where | Mitigation |
|---|---|---|
| Float bits change during a move | R7 merges, R5 split, R2 core phases | `to_bits` tests and the refine golden first; G-ID per commit; never reorder float operations. |
| Two note tables merged into one | R1 | Two methods, characterization table, G-ENST (a kick 35 -> 36 change drops every kick). |
| Silent yardstick break | T1b (`fix_metric.songs()`, `kick_files()`) | Run the yardstick unpatched on the internal host before and after. |
| Lost only-cover tests | T3, T4, T6, T9, T10 | Delete only from the adjusted lists; measure per-file coverage; keep planted-fault tests. |
| Golden or identity gate cannot fire | identity tool, T8 moves | Planted-fault test for every moved case and for the identity compare. |
| Merge conflicts with detection work | refine, cross_bleed, tom_bleed, onset | Owner freeze in phases 2-3, one module per PR, sync after each main merge. |
| Hidden env inputs | R6 | Snapshot with each pin set and unset; keep env wrappers until no caller is left. |
| Load-order break in the page | R9 | `util.js` first, verbatim moves, `test_ready` and G-E2E per PR. |
| macOS-only code not linted | R10, desktop | Clippy on an Apple-silicon Mac. |
| Scope creep into behaviour | all | Behaviour changes found during the work become separate tickets (R4 predicates, MK/TM chains, R6 steps 3-4). |

### 7.2 Measurements before and after

| Metric | Before (858c117) | Target after the release | How |
|---|---|---|---|
| Source files over 800 lines (Rust + static JS, no tests) | 24 | 12 or fewer | `wc -l` loop over `git ls-files` |
| Largest file | `static/app.js` 7,523 | app.js 400-600; largest Rust file under 1,000 (excl. data files `api.rs`, `tempo_detect.rs`) | `wc -l` |
| `refine.rs` | 1,393 lines, 3 `too_many_arguments` allows | 5 modules, each under 500; 0 allows | `wc -l`, `grep allow` |
| `menu.rs` | 2,001 lines | 5 modules, each about 500 or less | `wc -l` |
| Longest function | `cross_bleed_prior_session` 373 lines | under 120 | brace-count script from core.md |
| Functions over 100 lines (app, production) | 41 (app.md) | fewer than 30 | same script |
| `too_many_arguments` allows | 23 (app + core + shell) | 20 (refine 3 removed; R3 C later) | `grep` |
| Role enums | 4 | 1 (+ tempo-local `BeatVoice`) | `grep "enum Role"` |
| Median copies | 6 in core + 1 app | 4 named variants | `grep "fn median"` |
| `DRUMSPIKE_*` detection env names read in core | 15 (A1 x6, A5 x2, A6 x4, DELTA_RULE, DELTA_REF_, TEMPO_RULE) | 0 | `grep` in `crates/core/src` |
| core `pub` items with no outside use | 352 of 859 | under 50 | word-match scan (R8) |
| Dispatch edit points per command | 8 | 6 (no `matches!`, no or_else chain, one handler list) | review checklist |
| Rust `#[test]` | 1,387 | about 1,410 | `grep -c "#\[test\]"` |
| Python `def test_` | 1,451 (e2e 733 in 98 files) | about 1,000 (e2e about 616 in about 62 files) | `grep` |
| tools/ Python lines | 31,004 | about 18,000 | `wc -l` |
| `tests/golden` size | 13 MB | about 3.7 MB (if owner agrees) | `du -sh` |
| `just ci` wall time on an Apple-silicon Mac | not measured: measure in step 0.1 | report the difference; no target promised | `time just ci`, per e2e shard |
| Identity diff | n/a | 0 on golden, ENST, 3 sessions, re-pick, reanalyze, Learn | `tools/identity.py compare` |

## Appendix A: refuted claims

| Id | Claim | Reason it was refuted |
|---|---|---|
| T2 | Delete 15 research and diagnosis scripts and their 14 tests. | Six kept tests and ten kept tools import them at top level. `test_calib_kit.py:177` is the only end-to-end onset test and caught the #239 regression on 2026-11-01. `learn_eval.py`, `rescue_real.py` and `fix_metric.py` are the documented real-session gate and yardstick. Two "no user" statements were false (`onset_early`, `snare_onset`). Only the leaf scripts go (section 4.2). |
| T5 | Merge 26 e2e variation files into owner files, about 40 % fewer tests. | Ten files have no owner. Many single tests guard real bugs (R05, #154, #242, P4-35, P4-38d, P4-41a) whose page side no Rust test covers. 11 helper importers were missed. `test_flam_undo` needs its own bridge. A merge into `test_tempo_map` moves tests into the time-budgeted x86 shard. Only a layout change survives. |
| T6 | Trim 8 display files to one smoke file; delete or shrink the bridge guard. | Five files test user-visible results that reach the project file or the MIDI export (mixer save and reopen, velocity drag to note-on, onset override, menus to service, audition). `test_jscov.py` hard-codes a `test_render` test. `tiles-worker.js` coverage depends on `test_tiles`. The bridge guard is 9 cheap unit tests for 3 real bugs (#25 spaces in the path applies to the owner's work path). Only renderer internals may be trimmed. |
| T9 | Delete the 7 media test files and the explainers, TTS, compose and record tools. | `test_media_guide` is the user-guide checker (links, menu titles, keys, pictures). `test_media_demo_session` tests an e2e dependency. `test_media_record` holds the only tests of the OCR private-path guard (#459). Two explainer modules feed a live guide picture and a docs test. The landing page embeds 5 explainer videos. |

## Appendix B: review proposals not verified (backlog, not in this release)

- core: split `transport/mod.rs` and rename the three `layer` modules (core.md 6.7); stage names for the 5 Hit types (6.9); table-driven per-role parameters (6.10); `tempo` free of `project::Document` (6.11); split the other 150-210 line functions incl. `Engine::render` (6.12); `#[source]` error chains (6.13); shared test temp-dir helpers (6.14).
- app: move pure domain modules (`roles` name logic, `patterns`, `sensitivity`, loudness cut, Learn DSP) to core (app.md 6.7); one job abstraction (6.9); `DocState` enum (6.10); ports for audio out and clock (6.11); lock order and a `too_many_lines` ratchet (6.12).
- research: newtypes for samples, seconds and dB; JSDoc type check; ES modules; `cargo-mutants` per module to choose which Rust unit tests to keep (the est. 150-250 Rust trims in tests.md were not verified per test).

## Appendix C: owner decisions (taken 2026-11-03)

Decided:
- T11: the owner lifted the AGENTS.md test rules for this branch. The replacement text of section 4.3 applies.
- Detection rules stay. A rule changes only with proof on ENST and the real sessions that accuracy or the UI stays or improves. This release changes none.
- T8: `tests/golden/*` stays untouched in this release. Only the Python test files are merged. Shrinking the golden data is backlog.
- T9: the media tools and the 5 landing-page videos stay. Only the media tests in the adjusted T9 list go; `compose.py` stays.
- R6 step 3 is a behaviour change and stays out of this release.
- No detection ticket is open, so the freeze of phases 2-3 costs nothing now.

The original open points, for the record:

1. T11 rule text (section 4.3) and the per-file JS floor value.
2. T8: permission to edit `tests/golden/*` (on the "Do not touch" list).
3. T9: are the 5 landing-page explainer videos frozen?
4. Prerelease naming and the `release.sh --prerelease` change.
5. Detection-ticket freeze on the hot core files during phases 2-3.
6. Whether R6 step 3 (record delta and tempo in projects) is wanted after this release.
