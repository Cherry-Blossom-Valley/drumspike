# R17 plan: modular frontend state and command ownership

Ticket Cherry-Blossom-Valley/drumspike#41, step R17. This is the plan only; no code until the
coordinator answers. Line references are against `origin/main` (`6f9268c`).

Gate (from the ticket): project-session state, edit/undo, persistence, transport and rendering
extracted as modules; module tests exercise save/edit/undo transitions without a full DOM, and
late answers plus project switches are tested explicitly; the existing browser and per-file
coverage gates still run.

## today's state

Script order (`static/index.html:416-420`): `backend.js`, `lanes-gl.js`, `keymap.js`,
`shell.js`, `app.js` — plain scripts, no build step, one shared global scope. The e2e tests rely
on that: they drive the page with `page.evaluate` against globals (`S`, `setView`,
`deleteSelectedHit`, …; tests/e2e_app/test_undo_history.py:29,43-99).

`static/app.js` is 8223 lines. The main state object `S` sits at `static/app.js:7` with 828
`S.` writes in the file. Around it live module-ish globals that each own state and often their
own queue or generation counter:

| global | line | owns |
|---|---|---|
| `PH` | 48 | the playhead accessor over `S.playhead` (compositor vs user writes) |
| `FR` | 853 | frame scheduler: flags, dirty tracks, raf, counters the tests read |
| `GEO` | 906 | measured geometry, ResizeObserver |
| `CP` | 989 | compositor playhead (animations, tick history) |
| `SP` | 1124 | follow strip |
| `TM` | 1651 | tempo/meter map, its rulers and its undo waits |
| `MK` | 2099 | markers (own chain + pending) |
| `GLQ` | 2385 | GL tile queue; `loadedSeq` pairs with `S.loaded` in loadProject |
| `MX` | 2862 | inspector sample read (own `gen`, drops on `MX.gen !== gen || S.sid !== sid`, 2899) |
| `AC` | 3236 | transport/audio clock (token, epoch, `_appQ` at 3240) |
| `AU` | 3370 | audition |
| `MT` | 3577 | meters |
| `VL` | 3779 | velocity lane (drag `gen` guards against `EDGEN`, 3890-3915) |
| `LY`/`LYV` | 4394/4546 | layers (own `seq` counters) |
| `LN` | 4867 | Learn (own chain/pending, `lnUndoWaits`) |
| `HG` | 5488 | histograms (`HG.gen`, repick busy) |
| `SH` | 5763 | the edit/shift serial queue: `shSerial`, `SH.pending`, `edIdle` (5774) |
| `EDGEN` | 5806 | per-track counter that voids in-flight drags when the list was replaced |

Patterns that already exist and must survive the extraction:

- generation tokens: `_gen` (`app.js:464`) with `if (gen !== _gen) return` early outs all over
  `loadProject` (502-515); per-canvas `canvas._gen` (2572); `MX.gen`; `AC.tok`/`AC.epoch`;
  `HG.gen`; `LY.seq`.
- late answers: the gen checks above; `saveTimers` capture `sid` + body when scheduled so a
  switch cannot send one project's hits to another (`app.js:3089-3110`); `_editSeq` (3112)
  keeps the page dirty when an edit slips in during a save; `B.onDirty` re-lists with a 100 ms
  debounce (boot, 8215-8220).
- serialization: `shSerial`/`edIdle` (5763-5774) for edits, shifts, re-picks; `_appQ` (3240)
  for transport commands; `cleanCheckSoon` (3006) flushes writes before the clean check.
- persistence: `B` (backend.js:536) is one method per app command; debounced event writes
  `markDirty` → `saveTimers` → `pushEvents` (3090-3140), `flushEvents`, `cancelEvents`; the
  refused-write queue with Retry/Discard (`evFail*`, #249 R02).
- undo/redo: stacks on `S.undo`/`S.redo` (app.js:18); `pushUndo` (2987), `undoStep` (3032)
  applies via `applyOp` (3020), which dispatches to the owning module's applier (`edApplyOp`,
  `tmApplyOp`, `lnApplyOp`, `hgApplyLoudOp`, `trApplyOp`, `roleApplyOp`, `shiftApplyOp`,
  `DSShell.applyVel`); a refused step stays on its stack; `undoReveal` touches selection, view
  and `invalidateHits`.
- project open/switch: `loadProject` (465) resets `S` fields and module state (`lnReset`,
  `auReset`, `tmLoad`, `GLQ.loadedSeq`, `selMem`), re-reads tracks, events, peaks in batches of
  8, and finishes with `S.loaded = gen` (519) — the number the e2e tests wait on.
- boot: the IIFE at `app.js:8207` calls `tmInit`, `measureGeo`, `setup*`, `loadViewSettings`,
  `initSessions`, `loadProject`, `DSShell.boot`, `B.appReady`.

Constraints from the gates:

- the JS coverage gate covers a fixed file tuple, `JS_FILES = (app.js, backend.js, lanes-gl.js,
  tiles-worker.js, keymap.js)` (tests/jscov.py:21), 85 % lines per file. A new file is not
  measured until it is added to the tuple, and once added it must clear 85 % itself.
- the e2e tests are playwright + `ds-bridge` (docs/DEVELOPING.md, "the e2e_app tests and the
  bridge"). They must keep running unchanged wherever possible.
- there is no JS unit-test runner today; the only JS test path is the browser (tiles-worker is
  exercised "with a stand-in scope"). The gate asks for module tests without a full DOM, so one
  has to be introduced.

## target shape

- keep the no-build plain-script setup. ES modules would be the cleaner end state, but they
  change the loading model, the coverage harness and every global the tests touch at once; the
  repo has no bundler and does not need one. New modules are plain scripts loaded before
  `app.js`, each ending by exporting its public names onto `window`.
- five new modules under `static/`, in dependency order:

| file | contents |
|---|---|
| `state.js` | `S`, `PH`, settings (`SETTINGS` 97, `saveSettings` 116, `loadViewSettings`), `setDirtyState` |
| `persist.js` | `saveTimers`, `pushEvents`, `markDirty`, `flushEvents`, `cancelEvents`, `eventsBody`, `cleanCheckSoon`, `_editSeq` |
| `edit.js` | the editing/undo section (2935-3140 and the hit/velocity drag code), `EDGEN`, `shSerial`/`edIdle` |
| `transport.js` | `AC`, `AU`, `_appQ`/`appCmd` |
| `session.js` | `loadProject`, `_gen`, `initSessions`, `openProjectPath`, close/switch/recent handling (134-242) |

- rendering stays split between `app.js` (the DOM frame, `FR`/`GEO`/`CP`/`SP`) and the existing
  `lanes-gl.js`/`tiles-worker.js`; the ticket's "rendering extracted" is satisfied by moving
  the frame scheduler and geometry into a `render.js` and leaving lane drawing where it is.
- cross-module calls that are circular today (`persist.js` needs `edIdle` from `edit.js`,
  `edit.js` needs `markDirty` from `persist.js`) go through explicit hook objects set at boot,
  not through globals: each module exposes `init(deps)` and `app.js` wires them in `boot`
  (8207).
- the global surface the tests and `shell.js` use (`S`, `undo`, `redo`, `setView`,
  `deleteSelectedHit`, …) stays on `window` until every test is moved off it; `state.js`
  defines `S` on `window` from day one so nothing changes for `page.evaluate`.
- module tests: a new `tests/js_unit/` folder of plain node scripts (Node 22 is already a
  prerequisite) that import the modules with a stubbed `B`, a minimal `document`/`window`
  stand-in and no browser, exercising save/edit/undo transitions, late answers and project
  switches. They run via a small pytest wrapper with its own marker (`-m js_unit`) so the
  existing `-m e2e_app` shards and the coverage gate are untouched; the CI `test` job runs it.

## migration in small PRs

Each PR keeps main green: full `uv run pytest -q` with `DRUMSPIKE_AUDIO=null
DRUMSPIKE_REQUIRE_APP=1 DRUMSPIKE_REQUIRE_CORE=1`, plus `cargo test --workspace`. Every PR that
adds a file adds it to `JS_FILES` in the same commit. No test is deleted or weakened; e2e tests
are only touched where a moved name must be re-pointed.

1. **state.js — the state object moves.** `S`, `PH`, the settings functions and
   `setDirtyState` go to `static/state.js`, loaded first; `window.S` is explicit. `app.js`
   keeps everything else. The `S.playhead` accessor (48-51) moves verbatim.
   Tests: the existing e2e suite is the proof (it reads `S` and `S.playhead` everywhere, e.g.
   test_playhead_steps.py); no new test needed. Coverage: `state.js` added to `JS_FILES`.
2. **persist.js — writes and dirty tracking.** `saveTimers`, `pushEvents`, `markDirty`,
   `flushEvents`, `cancelEvents`, `eventsBody`, `cleanCheckSoon`, `_editSeq` move; `edIdle`
   arrives as an injected hook. First `tests/js_unit/` runner lands here with the save/edit
   transition tests: a mark → flush → ack → dirty-clear cycle, a refused write that stays
   queued, a late ack after a switch that clears nothing (`sid` capture), an edit during a
   write that keeps `_editSeq` ahead.
   Tests: tests/e2e_app/test_save_integrity.py (R02/R03) must stay green; the new js_unit
   files cover the same transitions without a DOM.
3. **edit.js — edit/undo ownership.** The editing section, the undo stacks (kept readable as
   `S.undo`/`S.redo` via accessors on `S`, exactly like `PH`), `applyOp`, `undoStep`,
   `EDGEN`, `shSerial`/`edIdle` and the drag gen guards move. `pushEvents`/`markDirty` come
   back as hooks. js_unit tests: undo/redo of add/del/move/setv with a stub applier, a refused
   step that stays on its stack, `EDGEN` invalidating an in-flight drag.
   Tests: test_undo_history.py, test_undo_r05.py, test_flam_undo.py, test_range_edits.py.
4. **transport.js — `AC`, `AU`, `_appQ`.** Pure move; the clock sync and epoch logic is copied
   verbatim (it is the most timing-sensitive code in the file). js_unit tests are limited to
   queue ordering (`appCmd`) and `appPos` arithmetic; the audio paths stay under the browser
   tests.
   Tests: test_playback.py, test_playreturn.py, test_transport_mode.py, test_halfspeed.py,
   test_audition.py.
5. **render.js — frame scheduler and geometry.** `FR`, `GEO`, `CP` (and `SP`'s glue if it
   comes free) move; the `FR.n` counters stay in place, the self-test and test_frames.py read
   them unchanged.
   Tests: test_frames.py, test_playhead_steps.py, test_follow_strip.py, test_follow_zoomed.py.
6. **session.js — project open/switch/close.** `loadProject`, `_gen`, `initSessions`,
   `openProjectPath`, close/recent handling move; `GLQ.loadedSeq`, `lnReset`, `auReset`,
   `tmLoad`, `selMem` become documented hooks the module calls. js_unit tests: an explicit
   late-answer matrix — an older `loadProject` finishing after a newer one writes nothing, a
   switch mid-load leaves no events behind, a close cancels pending writes
   (`cancelEvents`) — plus the switch-during-save case from step 2 replayed at session level.
   Tests: test_lifecycle.py, test_ready.py, test_relink.py, test_clear_recent.py; new
   js_unit files for the matrix.
7. **cleanup.** `app.js` keeps DOM wiring, menus glue and the panels (TM/MK/LN/HG/VL stay
   until a later ticket — the gate does not name them). Remove any window re-exports the tests
   no longer need; re-check the coverage report per file.

Steps 2 and 6 can swap places if the coordinator prefers session state first; 2 before 6 is
chosen because `persist.js` has the clearest DOM-free contract and its tests define the runner
that 6 reuses.

## risks

- **the global surface is a public API.** The e2e tests, `shell.js` (`DSShell.sync`,
  `DSShell.applyVel`) and the self-test scenarios read globals by name. A moved function that
  is not re-exported breaks a test far from the move. Mitigation: re-export in the same PR;
  grep `tests/ apps/desktop/selftest/` for each moved name.
- **the coverage gate can fail in both directions.** A new file missing from `JS_FILES`
  (tests/jscov.py:21) escapes the gate silently; a thin new file added to it fails at 85 %.
  Add the file and check its percentage in the same PR; move code, do not duplicate it.
- **no build step means load order is the dependency graph.** A wrong `<script>` order shows up
  only at boot. test_ready.py and a local G2 run catch it; keep each new script free of
  boot-time DOM reads (LN already reads computed styles at load, `app.js:4864` — that pattern
  must not spread into the new modules).
- **copy-verbatim, do not refactor.** The debounced writes, `_editSeq`, the gen guards and the
  clock code encode reviewed race fixes (#249 R02/R03, C5, P4-26, #471). Rewriting them "more
  cleanly" while moving is the main way this work regresses correctness.
- **circular dependencies** between persist/edit/session are real; the `init(deps)` wiring must
  stay in one place (boot) or the graph becomes invisible again.
- **js_unit runner drift.** A stubbed DOM can pass while the browser breaks. The runner is an
  addition, never a replacement: every behavior it covers keeps its e2e test, and the coverage
  gate still measures only real Chromium runs.
- **WKWebView vs Chromium.** The page runs in both; new modules must stay inside what
  WKWebView supports (no top-level await, no import maps).

## what each step proves

| step | proof |
|---|---|
| 1 | full e2e suite green with `window.S` from `state.js`; coverage report lists `state.js` ≥ 85 % |
| 2 | test_save_integrity.py green; js_unit: mark/flush/ack, refused write stays queued, late ack after switch, edit-during-write keeps dirty |
| 3 | the four undo test files green; js_unit: undo/redo against a stub applier, refused step stays, EDGEN voids a stale drag |
| 4 | the five transport/audition files green; js_unit: `appCmd` ordering, `appPos` math |
| 5 | test_frames, playhead and follow tests green; `FR.n` counters unchanged |
| 6 | lifecycle/ready/relink green; js_unit: superseded load writes nothing, switch mid-load is clean, close cancels pending writes |
| 7 | coverage-js.json: every listed file ≥ 85 %; no duplicate definitions between app.js and the new modules |
