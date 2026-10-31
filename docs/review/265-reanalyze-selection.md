# Verification report: #265 Re-analyze Selection (Hits menu item + Shift+R)

Branch of the ticket on top of main 3dc32042. Date: 2026-10-31.

## What was built (scope 1-6 of the ticket)

- **Engine** (`crates/app/src/reanalyze.rs`, `Service::reanalyze_selection`): the shared rules 1 to 7
  of `docs/concepts/reanalyze-selection.md`. The detector runs on a 100 ms-padded slice of the
  track's audio; only hits whose refined time lies in `[t0, t1]` are taken; the hits outside are
  carried over untouched (same values, so also ids on the page); a new hit closer than the minimum
  gap to an untouched outside hit is dropped; edited hits inside the range follow `keep_edited`
  (merge within 20 ms, `crate::edit::merge_edited`); the Learn marks of the track apply to the new
  hits (geometrically: `learned` at a This example, `filtered` at a Forget example when the filter
  is on — the score-based filter is a full-file computation and re-applies at the next whole-track
  run); the loudness cut marks the new hits; the backup of the previous events is written
  (`Backups/<track>.events.prev.json`); the followers are refreshed. One writer guard, 409 while an
  analysis or another write runs, 409 for a follower, 400 for a reversed range.
- **Agent API**: the command row `reanalyze_selection {sid, name, t0, t1, keep_edited?}` in
  `crates/app/src/api.rs` (Agent surface, Quick), the dispatch in `call.rs`, the Tauri wrapper in
  `apps/desktop/src-tauri/src/lib.rs` (both handler lists) and the section in
  `docs/agent-api.md`.
- **Menu**: `Hits > Re-analyze Selection` between Analyze Selected Track and Analyze All
  (`REANALYZE_SELECTION` in menu.rs, SPEC 111); greyed without a range (`sync_checks` reads the
  page's new `selRange` state key). An entry in the range context menu next to Learn This /
  Forget This (app.js `lnContext`). The Inspector Re-analyze button, Analyze Selected Track and
  Analyze All are untouched (whole-track).
- **Page**: `reanalyzeSelected`/`runReanalyzeSelection` (app.js) run the command, adopt the events
  and push one undo step `reanalyze` ("Re-analyze Selection (track, t0 to t1 s)") whose undo/redo
  swaps the before/after lists and writes them back (`edit.js` applyOp + `raApplyOp`); the Keep
  dialog (the existing `#reanalyzeModal`) opens only when edited hits are inside the range and its
  text names the range; the info line shows the answer's `info` ("Kick In: re-analyzed 0.900 to
  2.100 s, 3 hits replaced by 4"). Shift+R runs it (page key, both presets).
- **Count files, all +1**: `apps/desktop/src-tauri/src/menu/tests.rs` (SPEC 110 -> 111, keymap rows
  116 -> 117), `tests/e2e_app/test_menus.py` (111), `tests/e2e_app/test_keys.py` (117),
  `tests/e2e_app/test_keypreset.py` (117), plus `apps/desktop/selftest/shell.json`
  (menu_items 111).

## The Shift+R conflict (please read)

The ticket asks for the page key Shift+R "check it is free in both presets" — it was **not free**:
the Markers Ruler toggle has had Shift+R as a page key since #439 (default and Cubase preset), and
the key table refuses a key in two commands (`keymap_problems`, `DSKeys.problems`). The gate
demands "Shift+R runs it", so I took the gate literally and moved the Markers Ruler toggle to
**Option+Shift+R** (free in both presets, keeps the R mnemonic). Touched:
`static/keymap.js` (both rows), `apps/desktop/src-tauri/src/menu/tests.rs`
(`page("ruler_markers")`), `tests/test_keymap.py` (the #439 parametrize),
`tests/e2e_app/test_view_toggles.py` (the toggle test presses Shift+Alt+R now),
`docs/guide/06-find-your-way.md`. **If the owner prefers the Markers Ruler to keep Shift+R, the
alternative is to give Re-analyze Selection no key or another key — say the word and I re-key it.**

## Gate results (counted lines)

Rust, `cargo test -p drumspike-app --test reanalyze` (new `crates/app/tests/reanalyze.rs`):

```
test a_refused_range_and_a_refused_follower_and_the_registry_row ... ok
test a_new_hit_closer_than_the_minimum_gap_to_an_outside_hit_is_dropped ... ok
test a_range_run_replaces_only_the_hits_inside_and_leaves_the_outside_bit_identical ... ok
test the_previous_events_written_back_restore_the_range_and_the_backup_holds_them ... ok
test result: ok. 4 passed; 0 failed
```

On main these fail to compile (no `reanalyze_selection`), i.e. they fail on main.

e2e, `uv run pytest tests/e2e_app/test_reanalyze_selection.py -m e2e_app --no-cov -q`
(new; greyed without a range and enabled with one via `__DS_MENU_STATE__.selRange`, Shift+R runs
it on the real DSP detector, the info line names the range, the Keep dialog appears only with an
edited hit inside the range and names the range, one undo step restores the hits):

```
3 passed in 4.57s
```

Count files (all run, e2e):

```
tests/e2e_app/test_menus.py tests/e2e_app/test_keys.py tests/e2e_app/test_keypreset.py tests/e2e_app/test_view_toggles.py
28 passed in 28.79s   (after the selRange state pin and the Option+Shift+R fix)
```

`cargo test -p drumspike-app` (317 + the new suites): all green, 0 failed.
`cargo test -p drumspike-app --test api`: 7 passed (the MCP/bridge table runs
`reanalyze_selection` against the real project).
Fast suite: `uv run pytest -m "not e2e_app" -q` -> `675 passed, 153 skipped, 0 failed`.
`cargo clippy -p drumspike-app --all-targets -- -D warnings` rc=0;
`cargo fmt --all -- --check` rc=0.
`uv run python tools/check_private.py --diff crew-main` rc=0 (no `origin/main` ref exists in this
worktree; `crew` is the real remote — the local main branch is `crew-main`).

## Not verifiable on this host (coordinator follow-ups)

- `cargo clippy`/`test` for `drumspike-desktop` and `menu.rs`: macOS code — this host cannot build
  the desktop crate (libdbus-sys needs dbus-devel, and the menu is macOS code anyway). The menu
  unit tests (`apps/desktop/src-tauri/src/menu/tests.rs`, incl. SPEC 111 and the key rows) were
  updated but must run on macOS.
- The native macOS Hits menu shot needs the app window (menu.rs is macOS code); the range
  context menu with the item, the band and the info line after a run are captured by
  `test_screenshots_the_range_the_context_menu_and_the_info_line`
  (`265-range-and-context-menu.png`, `265-info-line.png`).
- The full e2e suite (`just test`) was not run end to end; the four count files, the new e2e file,
  test_view_toggles.py and test_range_edits.py (the #151 path next to the new undo step) were.

## Self-review fixes (after the first implementation)

- `apps/desktop/src-tauri/src/lib.rs`: the `reanalyze_selection` Tauri wrapper was not `async`
  (the crate did not compile; `cargo test --workspace` rc=0 now, with the host's dbus sysroot).
- `tests/e2e_app/test_learn.py`: the range context menu gained the entry, the exact-match
  assertion of the selection menu test was updated (suite green again).

## Files

- new: `crates/app/src/reanalyze.rs`, `crates/app/tests/reanalyze.rs`,
  `tests/e2e_app/test_reanalyze_selection.py`, this report
- changed: `crates/app/src/{lib,api,call}.rs`, `crates/app/tests/api.rs`,
  `apps/desktop/src-tauri/src/{lib.rs,menu.rs,menu/tests.rs}`, `static/{keymap.js,app.js,shell.js,
  backend.js,edit.js}`, `apps/desktop/selftest/shell.json`,
  `tests/{test_keymap.py,e2e_app/test_menus.py,e2e_app/test_keys.py,
  e2e_app/test_keypreset.py,e2e_app/test_view_toggles.py}`,
  `docs/agent-api.md`, `docs/guide/{03-detection,06-find-your-way}.md`
