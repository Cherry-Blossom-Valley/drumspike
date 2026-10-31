# E2E flake sweep, Intel (x86_64)

Ticket #220. Date: 2026-10-31. Agent: hermes@x86-mac.

## Method

- Host: the Intel x86_64 runner host, 12 cores. Load 4-8 during the sweep (the CI runner and other crew jobs ran in parallel; that is the point of the sweep: the gate form on a busy host).
- 3 full runs of `tests/e2e_app` in gate form: `DRUMSPIKE_AUDIO=null DRUMSPIKE_REQUIRE_APP=1 DRUMSPIKE_REQUIRE_CORE=1`, no deselect, no shard, no `-x`, chromium only (`pyproject.toml` addopts `--browser chromium`).
- Tree: the #220 sweep worktree on the agent host, branch `crew/hermes@x86-mac/220-hardening-e2e-flake-sweep-on-intel-3`, measured at `7fa115ed` (main at sweep start).
- Fresh debug ds-bridge built from the sweep tree; release devtools + parity copied into the worktree target (`REQUIRE_CORE` golden tests read the worktree path).
- Per-run guard: the sweep aborts rc=9 if the tree HEAD moves or the tree is dirty, so no run can record a mixed measurement.
- Logs: `220-sweepb1.log`, `220-sweepb2.log`, `220-sweepb3.log` (agent state `results/`); the RC lines and the run tails are in `220-sweep-b.log`.

## Take 1 (sweep-a): contaminated, discarded

- Run 1 (04:54-05:33Z): 706 passed, 10 skipped, 0 failed in 2336.35s (38:56). Valid.
- At 05:40:42Z a foreign `git merge origin/main: Fast-forward` moved the sweep tree from `12a16fba` to `7fa115ed` while run 2 was mid-flight (my own intermediate wake; reflog-confirmed, report #220/c-6540262683). Run 3 died on the stale-ds-bridge guard (715 errors).
- Run 2 (05:33-06:19Z): 2 failed, 704 passed, 10 skipped (45:08). Both failures are explained by the switch, not by flakes:
  - `tests/e2e_app/test_bridge_stdin_exit.py::test_sigkilled_child_pytest_leaves_no_bridge`: the child pytest hit the stale-ds-bridge guard that the switch created. Cascade. Needs no fix.
  - `tests/e2e_app/test_playback.py::test_pause_holds_the_position_and_stop_returns_to_the_start[chromium]`: playhead 2.66 s after pause+stop, allowed < 1.8. Real test bug; fixed, see below.

## Take 2 (sweep-b): the 3 counted runs

Tree locked at `7fa115ed` for every run; `RC_SWEEPi=0` each.

| run | start (UTC) | result line |
|-----|-------------|-------------|
| 1 | 06:47:39Z | 706 passed, 10 skipped, 0 failed in 2795.50s (46:35) |
| 2 | 07:34:16Z | 706 passed, 10 skipped, 0 failed in 2844.25s (47:24) |
| 3 | 08:21:42Z | 706 passed, 10 skipped, 0 failed in 2373.28s (39:33) |

Failed test ids over the 3 counted runs: none (`RC_SWEEP1=0`, `RC_SWEEP2=0`, `RC_SWEEP3=0`; no `FAILED`/`ERROR` line in any per-run log).

## The first fix (test only): the playback pause windows

`tests/e2e_app/test_playback.py::test_pause_holds_the_position_and_stop_returns_to_the_start[chromium]`.

- Evidence (sweep-a run 2, `220-sweep2.log`): `assert 2.6613333333333333 < 1.8`. 2.6613 s is 499 blocks x 5.333 ms (block-quantized transport position): start latency plus a ~1.5 s late pause click on the loaded host.
- The old windows were absolute wall-clock bounds (`0.9 < p < 1.8`, replay `0.9 < x < 2.4`); they budgeted only ~0.7 s beyond the fixed 700 ms dwell.
- Fix, in the repo's P4-42 idiom: anchor the windows on measured positions. `p0` is read after `wait_playing` (absorbs the start latency); the pause window is `p0 + 0.3 < p < p0 + 3.0`; the replay window is `p <= x < p + 3.0`. The frozen-position checks (lines 123, 125-126) and the stop-returns-to-start check (1e-9) are unchanged; no sweep evidence flags them.
- Semantics kept: pause holds a frozen position; stop returns to the exact start; replay continues from the playhead (the lower bound `p <= x` still fails a reset-to-0). A transport that keeps running still fails (the `wait_for_function` timeout).

## The second flake (found by the post-fix full suite)

`tests/e2e_app/test_export_dialog.py::test_choose_writes_into_the_chosen_folder_and_reset_goes_back_to_exports[chromium]` — 1 failed, 706 passed, 10 skipped in 2343.08s (gate run 1 on the merged tree `ce5ce7d0`, started 2026-10-31T09:11:06Z; `220-gatec-full.log`).

- Evidence: the assert saw `..drumspike-write-test.56066-977442000-2627.tmp` among the 4 exported files in the chosen folder.
- Root cause: the probe is `check_dest`'s short-lived write test, created and removed inside the check by its `TempFile` guard (`crates/app/src/export.rs`); the dialog refreshes the replace count with an un-awaited `exInfo()` after the export (`static/app.js`), so a destination check can be in flight when the test lists the folder. Under load the listing landed in that window.
- Fix (test only, the #170-gate idiom): `visible()` in `test_export_dialog.py` lists the chosen folder without dot files, like `test_export_again.py` since #170. The asserted export file names are unchanged. Not an app bug: the probe must exist while a check runs, by design.
- Commit: `48f76ab8` (test only, on the branch).

## Post-fix evidence

Gate run 1 (started 2026-10-31T09:11:06Z, `220-gate-c.log`): phases A-E green — the playback fix 5/5 (`1 passed` in 11.18s, 16.91s, 10.45s, 10.70s, 10.62s, `RC55_1..5=0`, `220-fix55-1.log`..`-5.log`), `check_private --diff origin/main` rc=0 — and the full suite caught the second flake above (rc=8, phase F).

Gate run 2 (started 2026-10-31T10:11:22Z, `220-gate-c2.log`), on the tree with both fixes (`48f76ab8` on top of `ce5ce7d0`):

- The fixed tests 5/5 green each: playback (`1 passed` in 9.87s, 9.72s, 10.00s, 12.06s, 11.31s, `RC55_PLAYBACK_1..5=0`, `220-fix55-p1..5.log`); export (`1 passed` in 14.50s, 11.23s, 13.68s, 11.85s, 13.82s, `RC55_EXPORT_1..5=0`, `220-fix55-e1..5.log`).
- The full e2e suite once green after the fixes: 707 passed, 10 skipped, 0 failed in 2898.70s (48:18) — counted line `707 passed, 10 skipped, 3 warnings in 2898.70s (0:48:18)` (`220-gatec-full2.log`). The count sits 1 above the sweep counts: the merged main adds the #228 lifecycle test.
- `tools/check_private.py --diff origin/main`, host word list sourced: rc=0 (`RC_CP=0` in `220-gate-c2.log`; `220-cp-c.log` silent). The guard is silent on a clean pass — a hit prints `FAIL: ...` and exits 1, an unset word list exits 2 loudly — so a silent rc=0 is a real pass: no private name in the added lines of the branch diff against origin/main. Re-checked on the final diff including the doc after the doc commit (`220-cp-diff.sh`); the doc text itself is guard-clean (`--text-file`, rc=0).

## Notes

- The bridge cascade test from take 1 needs no fix: sweep-b rebuilt the bridge fresh (`RC_BRIDGE=0`) and the test passed in run 1.
- Main moved +7 commits during the sweep (7fa115ed -> 27da48ea: R8 clippy batches 3-5, the #215 error/panic docs, the #229 crewboard repo list, #230 inventory step 2, the #231 failed-autosave status line, the 0.9.9 release; crates/app + crates/core and apps/desktop heavily touched). In tests/e2e_app only `test_lifecycle.py` changed (+1 test, #228); `test_playback.py` is untouched at 27da48ea, so the patch targets verified at 7fa115ed stay valid. The fix branch merges main (27da48ea) before the post-fix runs; the merged suite collects the new lifecycle test, so the post-fix full-suite count can sit 1 above the sweep counts.
- Gate run names: run 1 keeps `220-gate-c.log`, `220-gatec-full.log`, `220-fix55-1..5.log`; run 2 writes `220-gate-c2.log`, `220-gatec-full2.log`, `220-fix55-p1..5.log`, `220-fix55-e1..5.log`. No run-1 evidence is overwritten.
