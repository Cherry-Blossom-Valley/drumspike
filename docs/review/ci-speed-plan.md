# CI speed: a measured plan (17 min to under 8 min per main push)

Ticket #254. Date: 2026-10-31. Agent: hermes@x86-mac.

This is a plan, not an implementation: the owner picks the steps, and every picked step becomes a
follow-up ticket. All numbers are measured, none estimated by eye; the measurement method is named
next to each table. Raw logs live in the agent state `results/` directory of hermes@x86-mac
(`254-m-*.log`, `254-measure*.sh`, `254-ci-timings*.py`).

## Where a main push spends its time

Method: GitHub Actions API over the last 12 successful `ci.yml` runs on `main` (push event);
medians per job and per step (`254-ci-timings.py`). The run wall is the span from the first job
start to the last job end, so queue wait is not in it.

| job | median | n | notes |
|---|---|---|---|
| whole run | 993 s | 12 | range 625-1400 s; the ticket's "17 min" |
| `test` (Intel runner) | 983 s | 10 | the critical path |
| `e2e-arm64` (Apple silicon) | 416 s | 10 | parallel to `test` |
| `js-coverage` (ubuntu) | 12 s | 10 | needs both jobs: a serial tail |
| `scope` (ubuntu) | 6 s | 12 | |

Steps of the `test` job (medians, n=10): `pytest` 666 s, Rust tests (`cargo test --workspace`)
247 s, release build of the devtools binaries 109 s, Rust lint 14 s, debug ds-bridge 11 s, job
setup 9 s, Rust format ~2 s. Steps of `e2e-arm64`: pytest e2e_app shard rest in 3 parallel parts
383 s, debug ds-bridge 21 s.

The critical path is the `test` job, and inside it three steps: pytest 666 s + Rust tests 247 s +
the release devtools build 109 s. The target is a run wall under 480 s.

## The A/B measurements

The Intel runner host (the `test` job's runner, this host): 12 cores, rustc 1.99.1,
cargo-nextest 0.9.150, warm shared `target/`, tree `e540503e` (= `main` at measurement time). The
pytest rows run in exactly the CI step environment (`DRUMSPIKE_E2E_SHARD=x86`,
`DRUMSPIKE_JS_COV_OUT`, `DRUMSPIKE_AUDIO=null`, `DRUMSPIKE_REQUIRE_APP=1`,
`DRUMSPIKE_REQUIRE_CORE=1`). Scripts: `254-measure.sh`, `254-measure2.sh`.

| what | serial | variant | delta |
|---|---|---|---|
| pytest, full suite + e2e shard x86 | 513.9 s (849 passed, 38 skipped) | 158.3 s with `-n auto` (849 passed, 38 skipped) | -355.6 s (-69 %) |
| pytest, no e2e_app (the scope=py step form) | 202.7 s (795 passed, 33 skipped) | 63.9 s with `-n auto` (795 passed, 33 skipped) | -138.8 s (-68 %) |
| cargo test --workspace, warm | 205.6 s (1308 passed, 20 ignored) | nextest: 403.6 s (1308 passed, 20 skipped) | nextest +198 s (slower) |
| cargo test --workspace, first run | 331.2 s, rc=101 | - | stopped at a timing flake, see caveats |
| cargo fmt --all -- --check | 1.8 s | - | cheap |
| cargo clippy --all-targets -D warnings | 18.3 s | - | cheap |
| release devtools build, warm | 168.8 s | - | CI measures 109 s on its hotter target |
| debug ds-bridge build, warm | 90.0 s | - | CI measures 11 s |
| uv sync / playwright install chromium | 0.1 s / 3.0 s | - | warm; not a factor |

Apple silicon: CI itself measures the e2e half (the `e2e-arm64` job medians above: shard rest 383 s in
3 parallel parts). The remaining Apple Silicon rows (cargo test vs nextest, pytest with and without
xdist) are requested from the coordinator on the ticket thread (2026-10-31T16:49Z) and land here
when they arrive.

### The measured blocker of pytest -n auto: the JS coverage dump

The xdist full run passes every test and its raw coverage file has the same 10 files and the same
source hashes - but fewer covered lines. Decoded bitmaps, xdist run vs serial run: edit.js
-8298 of 16318 lines (50.9 % lost), persist.js -1478 of 4728 (31.3 %), app.js -28734 of 264347
(10.9 %), backend.js -602 of 27941 (2.2 %), render.js -302 of 19748 (1.5 %), lanes-gl.js -71 of
11416 (0.6 %), state.js -29 of 6685 (0.4 %); transport.js +6 lines, keymap.js and session.js
identical.

Cause: the coverage accumulator (`tests/jscov.py`) lives per pytest process and is dumped once per
process at session finish. With `-n auto` every worker writes the same `DRUMSPIKE_JS_COV_OUT`
path, and the last writer wins, so the other workers' coverage is lost. Consequence: the
`js-cov-x86` artifact of an xdist `test` job would under-report, and the merged 85 % gate of the
`js-coverage` job could fail or hide a real drop. The merge side (`tests/jscov.py merge`) already
ORs any number of raw files, so the fix is to dump per worker and merge - a small follow-up ticket,
and a hard precondition for idea 1 below.

## The ideas, with expected saving

1. **pytest `-n auto` in the `test` job.** Measured 513.9 -> 158.3 s on the Intel runner (-69 %);
   applied
   to the CI step (666 s) that projects to ~205 s (CI is colder; the ratio, not the
   absolute seconds, transfers). Expected wall: 993 - 666 + 205 + 12 (js-cov tail) = ~544 s.
   Precondition: the coverage dump above. Risk: worker isolation of the browser tests is measured
   green on the Intel runner (849 passed incl. the e2e shard x86); Apple Silicon numbers pending.
2. **Skip or overlap the release devtools build (109 s).** The parity tests need
   `target/release/parity` and `service_tool`; `clean: false` keeps `target/` between runs, so the
   binaries persist. Gate the step on "binary missing or Rust changed", or build it in the
   background during the Rust tests step and wait before pytest. Expected: -109 s of wall when
   skipped.
3. **Rust tests: keep `cargo test`, do not adopt nextest.** Measured slower on the Intel runner,
   warm:
   403.6 s (nextest) vs 205.6 s (cargo test), same 1308 tests. Both run the identical set; the
   gap is not profiled, but the decision needs no profile. Re-check only if the Apple Silicon
   numbers reverse it.
4. **e2e-arm64 reserve: 6 parts instead of 3.** After ideas 1-2 the wall is ~435 s and the
   `e2e-arm64` job (416 s) becomes co-critical. The parts split the shard exactly
   (`tests/test_e2e_shards.py` proves it), so doubling the parts is a one-line change with a
   measured basis (383 s serial-per-part time in 3 parts). Hold it as the reserve.
5. **Not worth a ticket:** Rust format (1.8 s), lint (14 s), ds-bridge (11 s on CI), uv sync and
   playwright (warm, seconds). The `js-coverage` serial tail (12 s) is fine.

## Ranked list

| rank | idea | projected run wall | size |
|---|---|---|---|
| 1 | pytest `-n auto` in the `test` job, after the per-worker coverage dump | ~544 s | one step in ci.yml + a small jscov.py ticket |
| 2 | skip/overlap the release devtools build | ~435 s | a step condition or a background build |
| 3 | e2e-arm64 in 6 parts (reserve) | keeps < 480 s when e2e turns co-critical | one line + the shard test |
| 4 | nextest | measured slower: reject | none |

Ideas 1 + 2 land the median main push at ~7:15 (< 8 min) with the e2e job as the new floor. Idea 3
is the reserve that buys headroom when the e2e half slows. The owner picks; each row becomes its
own follow-up ticket.

## The hook prototype

`tools/hooks/pre-commit` (new, executable): refuses the commit when a staged `*.rs` file is not
rustfmt-clean (`rustfmt --edition 2024 --check`, the workspace edition; `rust-toolchain.toml`
pins stable). Without a rustfmt on PATH it exits 0, so a clone without Rust can still commit. It
does not read `rust-toolchain.toml` and does not fall back to `cargo fmt` - prototype scope, a
follow-up can widen it. Install per clone: `git config core.hooksPath tools/hooks`.

`tests/test_pre_commit_hook.py` (new): proves the hook blocks an unformatted Rust file, allows a
formatted one, and ignores a commit without Rust files, each in a throwaway git repo wired to the
hook by `core.hooksPath`.

Cost: checking the staged files is about a second (`cargo fmt --all -- --check` is 1.8 s warm for
the whole tree). A red G1 found after the push costs a full CI round: 993 s median here, more
when the Intel runner is queued. The hook moves that finding to before the commit.

## Caveats

- The A/B ran warm on a shared `target/`; CI runs after `git clean -ffdx -e target/`, so the first
  compile of changed crates is not in the A/B. The ratios transfer, the absolute seconds do not.
- cargo test run 1 hit a timing flake (`transport_play_sends_the_meters_at_about_25_hz_while_playing`,
  `crates/app/tests/transport.rs:1026`, "a mono track: one level") and stopped rc=101 before the
  core tests; the warm rerun passed all 1308. The flake belongs to the #220 territory, not here.
- The run-wall medians are job spans; the two 1360-1400 s runs of the last twelve show the cold
  and queue variance that the projections do not model.
- The merged 85 % gate could not be simulated with CI's arm64 raw files: they carry the source
  hash of their run, and `main` moved since. The per-runner loss above is the measured fact.
- The Apple Silicon A/B rows are pending the coordinator's answer on the ticket thread; this document
  is updated when they arrive.
