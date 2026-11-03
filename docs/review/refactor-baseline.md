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
