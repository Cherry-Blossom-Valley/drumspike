# AGENTS.md: drumspike

Read this file first. Read other documents only for the item you work on. Humans start with
`README.md` and `CONTRIBUTING.md`; the full developer guide is `docs/DEVELOPING.md`.

The map of the code, its invariants and the glossary are in `ARCHITECTURE.md`.

## What it is

Drum Spike: close-mic drum hit detection with a spectral onset detector (DSP), refined to the first
sample of the attack, with velocities and sample-exact MIDI and spike files per track. The product
is the desktop app (Tauri 2, Rust crates `drumspike-core` and `drumspike-app`). The app has no
Python. Python is the test tool (pytest, the browser tests through the bridge, the self-test
tools, the parity checks of the Rust binaries) and the research tools in `tools/`.

## Map

| Path | Content |
|---|---|
| `static/` | the editor, classic scripts in the load order of `index.html`: page modules (`state.js`, `util.js`, `persist.js`, `edit.js`, `transport.js`, `render.js`, `session.js`, `backend.js` (object `B`: one method per app operation), `lanes-gl.js` (WebGL2 wave lanes), `tiles-worker.js` (tile textures; the page holds no PCM), `keymap.js`, `help.js`, `shell.js`), one script per feature (`export.js`, `addtracks.js`, `mixer.js`, `histograms.js`, `learn.js`, `layer.js`, `velocity.js`, `followstrip.js`, `tempomap.js`, `markers.js`, `wave.js`, `commands.js`, `files.js`, `review.js`, `analysis.js`, `plugins.js`, `status.js`, `inspector.js`, `tracks.js`), and `app.js` (boot, global handlers, wiring), `index.html`, `style.css` |
| `crates/core/` | `drumspike-core`: audio I/O (`io/`), peaks, `dsp.rs` (filter and statistics primitives), `role.rs` (the one `Role`), `rules.rs` (`RuleSet`, `ResearchOverrides`), `refine/` (attack refinement: `reference`, `reach`, `walk`, `edge`, `track`), `analyze.rs`, `backend.rs`, `velocity.rs`, `midi.rs`, `spike.rs`, `project/`, `tempo_detect.rs`, `transport/`, `detect/` (`features`, `onset`, `peaks`, `nmfd`, `resample`, `attribution`, `cross_bleed`, `tom_bleed`, `flam`) |
| `crates/app/` | `drumspike-app`: the app commands as plain functions (no Tauri dependency): tracks, analysis jobs (`jobs.rs`, session steps in `steps.rs`), event marks (`policy.rs`), export, transport, Learn, review queue, agent API (`api.rs`, `call.rs` with the per-module `HANDLERS`, `cli.rs`, `mcp.rs`; `docs/agent-api.md`), relink, import, autosave |
| `crates/devtools/` | developer tools, not product code: `parity`, `bench`, `cache_tool`, `service_tool`, `ds-bridge` (test bridge, debug only) |
| `apps/desktop/` | the Tauri shell (`src-tauri/`: one handler list macro in `lib.rs`, `menu/` (`items`, `keymap`, `build`, `sync`, `macos`), window, open, about, self-test), scripts (`build-dmg.sh`, `release.sh`, `self-test.sh`, `gen_third_party.py`), `THIRD_PARTY.md`, `selftest/` scenarios |
| `tests/` | pytest: `e2e_app/` (the page against `ds-bridge`), `jscov.py` (JS coverage gate), `frozen.py`, `golden/` (frozen references) and `test_golden.py` (the golden regression), export tests |
| `tools/` | `identity.py` (identity snapshot), `fn_lengths.py` (function lengths), `parity/` (corpus and session builders), `check_exports.py` (independent export checker), `release/cut.py` (release cut), `media/` (guide pictures, recordings, explainers), `eval/` and research tools (each has a test or a documented command) |
| `site/` | the landing page (`index.html`, `assets/`; `tools/media/build_site.py` builds it for a sub-path) |
| `docs/` | `guide/` (user guide, also the source of the in-app help), `DEVELOPING.md`, `RELEASING.md`, `release-notes/`, `adr/`, `design/`, `concepts/`, `engineering/` (the finished refactoring plan), `review/` (its baseline), `agent-api.md`, `event-policy.md`, `licensing.md`, `product/` (feature path, landing copy, media scripts) |

## Commands (from the repository root)

Requirements: `uv`, Rust stable through rustup, Node 22, and `just` (optional; `just --list`).

```bash
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo build --release -p drumspike-devtools                # parity, bench, cache_tool, service_tool
cargo build -p drumspike-devtools --bin ds-bridge          # test bridge (debug only)
uv run pytest -m "not e2e_app"                             # no browser
DRUMSPIKE_AUDIO=null DRUMSPIKE_REQUIRE_APP=1 DRUMSPIKE_REQUIRE_CORE=1 uv run pytest -q   # G2: the full suite
```

`DRUMSPIKE_REQUIRE_APP=1` and `DRUMSPIKE_REQUIRE_CORE=1` turn the skip of tests whose binary is
missing into a failure. Playwright needs Chromium once: `uv run playwright install chromium`.
Run long gates in the background with a log file. The app build (G3), the self-test (G5) and the
release builds are in `docs/DEVELOPING.md`, sections 4, 7 and 8.

### Which gates when

- While you work: only the affected tests (`cargo test -p <crate> <name>`, `uv run pytest <file>`).
- Once before hand-back, scoped like CI (#125, `tools/ci/scope.py`): G1 (fmt, clippy, cargo test)
  only when the change touches Rust or the app; G2 without `tests/e2e_app` when it touches only
  `tools/`, `site/` or non-e2e tests; the `e2e_app` tests only when it touches the app, `static/`
  or the e2e tests and their helpers. G3 and G5 only when the change touches `apps/desktop/` or
  the ticket names a self-test. The ENST gate and the session check only for detector code.
- A push to a `crew/**` branch starts no CI run, and you do not dispatch one (ci.yml refuses a by-hand run on `crew/**` that the owner did not start): the local gate is
  the gate, and main's CI runs the full scope after the merge.
- No `[skip ci]` in a PR title or in a commit that changes code. Docs-only changes need no
  marker: CI ignores `docs/` by path.

## Invariants

- Python only through uv (`uv run ...`), Python 3.12 from `.python-version`. A new Python file
  needs a user (a test, a script, a documented command or an import).
- Never run `rustup default`. `rust-toolchain.toml` pins `stable`.
- Parity: hit times, velocities and MIDI bytes are exact. The references in `tests/golden/frozen/`
  are frozen.
- Source audio is never modified. Project format 1 stays loadable.
- No audio files, real session names or `target/` in git. Real sessions for local checks
  live in `~/.cache/drumspike/sessions/Session A` to `Session I` (outside the repository).
- Licence: the source code is Apache-2.0 (`LICENSE`); attributions, exceptions and the media and
  brand terms are in `NOTICE` and `docs/licensing.md`. Keep `NOTICE` current when a bundled
  third-party part changes.
- Documents use plain hyphens, no em-dashes.

## Test rules

1. A test names a domain invariant or a user-visible result in its name or first line.
2. A test may be deleted when it only pins a removed script, or a fixed bug that another test
   covers. The PR names the deleted test and the covering test.
3. Never delete or weaken: `tests/test_golden.py`, `tests/frozen.py`, `tests/golden/*`,
   `tests/test_exports.py`, `tests/test_layer_exports.py`, the identity snapshot, the ENST gate,
   the real-session check, the tests of source-audio safety and of project format 1 loading, and
   `tools/check_private.py`.
4. JS coverage is gated per file, not as a total. The floor is 70 % of the lines of each file
   (`JS_FAIL_UNDER` in `tests/jscov.py`), measured by `tests/e2e_app`. The files are the 31 of
   `JS_FILES`: the boot script `app.js`, the page modules `state.js`, `util.js`, `persist.js`, `edit.js`,
   `transport.js`, `render.js`, `session.js`, `backend.js`, `lanes-gl.js`, `tiles-worker.js` and
   `keymap.js`, and the feature scripts `export.js`, `addtracks.js`, `mixer.js`, `histograms.js`,
   `learn.js`, `layer.js`, `velocity.js`, `followstrip.js`, `tempomap.js`, `markers.js`, `wave.js`,
   `commands.js`, `files.js`, `review.js`, `analysis.js`, `plugins.js`, `status.js`,
   `inspector.js` and `tracks.js`. A new script in `static/` enters `JS_FILES` at the same floor.
5. Every public Tauri command and every core entry point has a test.
6. A script in `tools/` has a test or a documented command.

## Branches and PRs

- Work happens through crewboard tickets: one ticket, one branch `crew/<agent>/<issue>-<slug>`,
  one PR with `Closes #<n>`. The coordinator verifies and merges; agents never merge.
- Keep the first line of a commit message short. Put measurements in a report file, not in the
  commit body.
- The repository and the board are public, and all test data is internal only (the real
  sessions, ENST, STAR Drums): no audio, derived files, frames, song, take or file names in a
  commit, PR body or ticket comment. Public text carries aggregates and Session A/B/C only.
- Before `done`, run `uv run python tools/check_private.py --diff origin/main` and
  `uv run python tools/check_private.py --text-file <pr-body-or-comment-draft>`. Exit 1 names the
  file, line and word; exit 2 means `DRUMSPIKE_PRIVATE_WORDS` is unset on the host (it is set per
  host, never in the repo).

## Do not touch

- `rust-toolchain.toml`
- `tests/golden/*`: frozen references; they cannot be written again
- `.github/workflows/*`, `.crewboard.toml`, `AUTHORITY.md`: maintainers only
