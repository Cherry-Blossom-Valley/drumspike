# Refactor baseline

Step 0.1 of `docs/engineering/refactor-plan.md` (section 7.2). Measured on branch `refactor/structure` at
commit `ac545a8` (the branch base, v0.9.19 plus the plan). The refactoring changes no product code in
phase 0, so these are the "before" numbers. Every number has the command that produced it. Commands run
from the repository root.

The plan table gives "before" numbers read at 858c117. Where a number below equals the plan number,
the code did not move between the two commits. Where it differs, the reason is in the note.

## Size and shape

Production files: Rust and `static/*.js`, without test files. Set used by all rows in this block:

```
PROD='(^|/)tests?(/|\.rs$)|_tests?\.rs$'
git ls-files '*.rs' 'static/*.js' | grep -Ev "$PROD"
```

| Metric | Value | Command |
|---|---|---|
| Source files over 800 lines | 24 | `git ls-files '*.rs' 'static/*.js' \| grep -Ev "$PROD" \| xargs wc -l \| awk '$1>800 && $2!="total"' \| wc -l` |
| Largest file | `static/app.js` 7,523 lines | same list, `sort -rn \| head` |
| Largest Rust files | `menu.rs` 2,001; `api.rs` 1,943; `tempo_detect.rs` 1,922; `export.rs` 1,720; `parity.rs` 1,539 | same list, `sort -rn \| head` |
| `refine.rs` | 1,393 lines, 3 `too_many_arguments` allows | `wc -l crates/core/src/refine.rs`; `git grep -c too_many_arguments -- crates/core/src/refine.rs` |
| `menu.rs` | 2,001 lines | `wc -l apps/desktop/src-tauri/src/menu.rs` |
| `too_many_arguments` allows | 23 (app, core, shell) | `git grep -n too_many_arguments -- '*.rs' \| wc -l` |
| Role enums | 4 (`role`, `detect::attribution`, `detect::onset`, `tempo_detect`) | `git grep -n 'enum Role' -- '*.rs'` |
| Median copies | 6 in core, 1 in app (plus 1 in `bench.rs`, not product code) | `git grep -n 'fn median\b' -- 'crates/*.rs'` |
| `DRUMSPIKE_*` detection names read in core | 15 (A1 x6, A5 x2, A6 x4, DELTA_RULE, DELTA_REF_, TEMPO_RULE) | `git grep -ohE 'DRUMSPIKE_[A-Z0-9_]+' -- crates/core/src \| sort -u \| grep -E 'A1_\|A5_\|A6_\|DELTA\|TEMPO' \| wc -l` |
| Core `pub` items with no outside use | not measured: the word-match scan is step R8 and does not exist yet | n/a |
| Dispatch edit points per command | not measured: a review checklist, not a count | n/a |

Function lengths use `tools/fn_lengths.py`, a brace counter (signature, body and nested closures count;
comments and strings are blanked; test modules and test files are left out):

| Metric | Value | Command |
|---|---|---|
| Longest function | `cross_bleed_prior_session` 373 lines (`crates/core/src/detect/cross_bleed.rs`) | `uv run --no-project python tools/fn_lengths.py 100 \| head -1` |
| Next longest | `run_one` 263 (app `cross_bleed`), `transport_play_window` 224 (app `transport`), `render` 210 (core `transport::mixer`), `onsets_with_candidates_inner` 205 | `uv run --no-project python tools/fn_lengths.py 100 \| head -5` |
| Functions of 100 lines or more, all crates | 45 (app 20, core 19, devtools 4, shell 2) | `uv run --no-project python tools/fn_lengths.py 100 <prefix> \| wc -l` for `crates/app`, `crates/core`, `crates/devtools`, `apps` |
| Functions of 100 lines or more, app production | 20 | `uv run --no-project python tools/fn_lengths.py 100 crates/app \| wc -l` |

Note on the last row: the plan quotes 41 from the app review, which used another counting method. This script
gives 20 at 100 lines and 27 at 80 lines (`tools/fn_lengths.py 80 crates/app | wc -l`). Compare the release
against the numbers of this script, not against the 41.

## Tests and tools

| Metric | Value | Command |
|---|---|---|
| Rust `#[test]` | 1,387 | `git grep -c '#\[test\]' -- '*.rs' \| awk -F: '{s+=$2} END{print s}'` |
| Python `def test_` | 1,451 | `git grep -c '^\s*\(async \)\?def test_' -- '*.py' \| awk -F: '{s+=$2} END{print s}'` |
| of them in `tests/e2e_app` | 733 in 98 files | `git grep -c '^\s*def test_' -- 'tests/e2e_app/*.py' \| awk -F: '{s+=$2; n++} END{print s, n}'` |
| `tools/` Python lines | 31,004 | `git ls-files 'tools/*.py' \| xargs wc -l \| tail -1` |
| `tests/golden` size | 13 MB in 107 files | `du -sh tests/golden`; `git ls-files tests/golden \| wc -l` |
| `just ci` wall time | not measured | `just` is not installed on this host, and the recipe runs `uv sync`, a Playwright install and the whole e2e suite. The plan allows "not measured" here. Measure it in the PR that needs the number, on a host with `just`, under the heavy lock. |

The two counts that the plan quotes for Python tests use `def test_` at any indent. The numbers above equal the
plan numbers.

## Identity snapshot baseline (steps 0.2 and 0.3)

The tool is `tools/identity.py`. The corpus locations are in a local config outside the repository. The
manifests are in `~/.cache/drumspike/identity/` and are not in git.

Baseline build: `cargo build --release --bin service_tool` at `ac545a8` on an Apple-silicon Mac, binary kept as
`~/.cache/drumspike/identity/tool_ac545a8`. Snapshot command:

```
uv run --project . python tools/identity.py snapshot --tool <service_tool> --out <manifest.json> --parts golden,enst,sessions
```

| Part | Items | What one item is |
|---|---|---|
| golden | 60 | 20 synthetic WAVs of `tests/golden_corpus.py`, each: track events and MIDI, multi-track MIDI, tempo suggestion |
| enst | 210 | every ENST item of the three drummers (`phrase`, `solo`, `minus-one`, `MIDI-minus-one`): whole project of 7 or 8 mics, events and MIDI per mic, tempo suggestion |
| sessions | 75 | 3 real sessions, 6 to 9 tracks each: analysis with all session steps (events, MIDI, spike WAV per track, multi-track MIDI, tempo), one re-pick (sensitivity 80, stored) and one reanalyze-selection (30 to 45 percent of the track) per track |

Wall time of one full snapshot (3 projects in parallel, release build, Apple-silicon Mac): about 123 s
(golden 2 s, ENST 87 s, sessions 33 s).

Determinism: two snapshots of the same binary and corpus compare identical (`compare`: golden 60, enst 210,
sessions 75 items, 0 differ, exit 0).

The gate can fail. A copy of the manifest with one session item altered compares to exit 1 and names the
item. A copy without one ENST item compares to exit 1 and names the missing item. A snapshot with
`DRUMSPIKE_DELTA_RULE=0` set against the unpinned baseline differs in 83 items (82 ENST items, 1 session
item; golden unchanged): a real behaviour change is seen. `tests/test_identity.py` plants the same faults
(one velocity, one MIDI byte, an absent mark, a key order, an event order, a missing item, too few items)
on synthetic manifests.

Variants with an env pin, taken at the same base (`--env KEY=VALUE`; the file name carries the pin):

| File | Pin |
|---|---|
| `ac545a8.json` | none (the baseline) |
| `ac545a8-delta0.json` | `DRUMSPIKE_DELTA_RULE=0` |
| `ac545a8-tempo1.json`, `ac545a8-tempo2.json`, `ac545a8-tempo3.json` | `DRUMSPIKE_TEMPO_RULE=1`, `2`, `3` |

Checks of the variants against the baseline: `DRUMSPIKE_TEMPO_RULE=3` is identical (rule 3 is the default when the
variable is unset); rule 1 differs in 13 items (11 ENST, 2 session, golden unchanged); rule 2 in 5 items (4 ENST,
1 session). So the variants are not copies of the baseline and the snapshot sees the pins.

## After the refactoring (step 6.2)

Measured at commit `8d2c11e` on branch `refactor/structure` (the last code commit is `8e23410`, step 5.2; `8d2c11e`
changes only documents). Same commands as above, same `PROD` set, run from the repository root. "Target" is the
column of the plan, section 7.2.

### Size and shape

| Metric | Before (`ac545a8`) | After (`8d2c11e`) | Target | Met |
|---|---|---|---|---|
| Source files over 800 lines | 24 | 22 | 12 or fewer | no |
| Largest file | `static/app.js` 7,523 | `crates/app/src/api.rs` 1,937 (a data file); `static/app.js` is 753 | app.js 400 to 600 | no (753) |
| Largest Rust files | `menu.rs` 2,001; `api.rs` 1,943; `tempo_detect.rs` 1,922; `export.rs` 1,720; `parity.rs` 1,539 | `api.rs` 1,937; `tempo_detect.rs` 1,920; `export.rs` 1,714; app `transport/mod.rs` 1,511; core `transport/mod.rs` 1,398 | under 1,000 without `api.rs` and `tempo_detect.rs` | no (`export.rs` 1,714) |
| `refine.rs` | 1,393 lines, 3 `too_many_arguments` allows | `refine.rs` 72 lines plus `reference` 305, `reach` 202, `walk` 438, `edge` 176, `track` 230; 0 allows | 5 modules under 500 each, 0 allows | yes |
| `menu.rs` | 2,001 lines | `menu/` `mod` 38, `items` 387, `keymap` 342, `build` 421, `sync` 599, `macos` 316 | 5 modules of about 500 or less | yes (`sync` 599) |
| `too_many_arguments` allows | 23 | 20 | 20 | yes |
| Role enums | 4 | 1 (`role::Role`; the tempo detection has its own `BeatVoice`) | 1 | yes |
| `fn median` | 8: 6 in core, 1 in app, 1 in `bench.rs` | 7: 5 in core (`dsp`, `detect::flam`, `detect::onset`, `similarity`, `tempo_detect`), 1 in app, 1 in `bench.rs` | 4 named variants | no |
| `DRUMSPIKE_*` detection names in core | 15 | 21 with the same command | 0 | no, see the note below |
| Core `pub` item lines (production source, no `src/bin`) | 640 `pub`, 19 `pub(crate)` | 395 `pub`, 280 `pub(crate)` | under 50 `pub` items with no outside use | not measured |
| Dispatch: `matches!` and `or_else` in `crates/app/src/call.rs` | 21 | 3 | no `matches!` list, no `or_else` chain | yes |

Commands for the rows that are new in this section:

```
git ls-files '*.rs' 'static/*.js' | grep -Ev "$PROD" | xargs wc -l | awk '$1>800 && $2!="total"' | wc -l
wc -l crates/core/src/refine.rs crates/core/src/refine/*.rs apps/desktop/src-tauri/src/menu/*.rs static/app.js
git grep -n 'fn median\b' <commit> -- 'crates/*.rs'
git grep -hE '^\s*pub (fn|struct|enum|const|static|type|trait|use) ' <commit> -- 'crates/core/src/*.rs' \
  ':!crates/core/src/**/tests.rs' ':!crates/core/src/**/*_tests.rs' ':!crates/core/src/bin' | wc -l
git grep -hE '^\s*pub\(crate\) (fn|struct|enum|const|static|type|trait|use) ' <commit> -- (same paths) | wc -l
git grep -nE 'matches!\(|or_else' <commit> -- crates/app/src/call.rs | wc -l
```

The `pub` line count is not the R8 metric "pub items with no outside use"; that word-match scan is not in the
repository. The line count shows the direction only: 245 `pub` lines fewer, 261 `pub(crate)` lines more.

Note on the `DRUMSPIKE_*` row: the count went up because `rules.rs` now spells out each per-role name
(`DRUMSPIKE_DELTA_REF_KICK` and so on) where the old code built it from a prefix. What changed is where the names are
read: the names and their parse are in `rules.rs` only (`RuleSet::from_lookup`, `ResearchOverrides::from_lookup`),
and the app passes `RuleSet` and `ResearchOverrides` down. The other hits are doc comments in `detect/onset.rs`,
`detect/cross_bleed.rs` and `tempo_detect.rs`. Core still reads the environment through three old entry points
(`OnsetParams::for_role`, `detect_onsets`, the `Default` of `CrossBleedParams`), which call `RuleSet::from_env` or
`ResearchOverrides::from_env`; `DspBackend::detect` and `analyze::detect_events` use `detect_onsets`. The contract step
of this Parallel Change is open (see "Steps not done" below).

### Function lengths

| Metric | Before | After | Target | Met |
|---|---|---|---|---|
| Longest function | `cross_bleed_prior_session` 373 | `transport_play_window` 224 (app `transport`, not in the plan) | under 120 | no |
| Next longest | `run_one` 263, `transport_play_window` 224, `render` 210, `onsets_with_candidates_inner` 205 | `render` 210 (core `transport::mixer`), `onsets_with_candidates_inner` 207, `reanalyze_selection` 183, `tracks_add` 169 | n/a | n/a |
| The two split functions | `cross_bleed_prior_session` 373, app `cross_bleed::run_one` 263 | 17 and 43; the longest phase function in core `detect/cross_bleed.rs` is `learn_pair` 107, in app `cross_bleed.rs` it is `redetect` 108 | under 120 | yes |
| Functions of 100 lines or more, all crates | 45 (app 20, core 19, devtools 4, shell 2) | 40 (app 18, core 19, devtools 3, shell 0) | n/a | n/a |
| Functions of 100 lines or more, app production | 20 (27 at 80 lines) | 18 (25 at 80 lines) | fewer than 30 (plan method; this script gave 20 before) | yes |

Commands: as in the first table (`tools/fn_lengths.py 100 <prefix> | wc -l`), plus
`uv run --no-project python tools/fn_lengths.py 1 crates/app/src/cross_bleed.rs crates/core/src/detect/cross_bleed.rs`
for the two split functions.

### Tests and tools

| Metric | Before | After | Target | Met |
|---|---|---|---|---|
| Rust `#[test]` | 1,387 | 1,413 (+26) | about 1,410 | yes |
| Python `def test_` | 1,451 | 1,065 (-386, 27 %) | about 1,000 | close |
| of them in `tests/e2e_app` | 733 in 98 files | 629 in 68 files | about 616 in about 62 files | close |
| `tools/` Python lines | 31,004 | 21,068 (-9,936) | about 18,000 | no |
| `tests/golden` size | 13 MB in 107 files | 13 MB in 107 files | unchanged (owner decision, plan Appendix C) | yes |
| `just ci` wall time | not measured | not measured | report the difference | no: `just` is still not installed on this host |
| JS files under the per-file coverage gate (`JS_FILES` in `tests/jscov.py`) | 11 | 31 | every new `static/` script | yes |

The identity diff of the release head (target 0) and the other final gates are step 6.3. An independent verifier runs
them on the release head; their result is not in this file.

## Steps not done

The plan steps that have a commit on this branch (`git log --oneline ac545a8..HEAD`, subject prefix
"Refactor <step>:") are 0.1 to 0.4, 1.1 to 1.5, 1.7, 1.10, 2.1 to 2.7, 3.1, 3.2, 4.1 to 4.4, 5.1, 5.2, 6.1 and 6.2.
The other steps:

| Step | State | Why |
|---|---|---|
| 0.5 | done, no commit | Owner decisions, taken on 2026-11-03 and recorded in the plan, Appendix C. |
| 1.6 | not done as its own step | It ran together with 1.7 (T3, T4). No per-file coverage table with and without the T3, T4 and T6 candidates was recorded. The per-file floor of 70 % is checked by the whole e2e suite in the final gate (6.3). |
| 1.8 | skipped | T5 was not in the implementation run. It is layout only and removes no test; it moves e2e tests between files, so it also touches the shard balance in `tests/e2e_shards.json`. Backlog. |
| 1.9 | skipped | T6 was not in the implementation run. Its size is an estimate (15 to 25 tests), and its precondition, the coverage of `lanes-gl.js`, `tiles-worker.js` and `render.js` per candidate (step 1.6), was not measured. Backlog. |
| 1.3, optional part | skipped | `calib_hyp` and `roll_fill` are optional in T2 and stay. |
| 3.1, contract part | not done | The env readers `RuleSet::from_env` and `ResearchOverrides::from_env` still serve three old core entry points (note above). Moving their callers to explicit parameters touches `DspBackend::detect` and `analyze::detect_events`; it is a structure change that needs its own G-ID run with each pin set. |
| 5.3 | skipped | Not in the implementation run. `shell.js`, `help.js` and `splash.js` are still outside `JS_FILES` without a documented exception. |
| 6.3 | open | The final gates run on the release head by an independent verifier, after this step. |

Not in this release by plan decision (section 5 and Appendix B): R6 steps 3 and 4, R3 stage C, the R2 re-pick list,
R9 part B and the Appendix B backlog.
