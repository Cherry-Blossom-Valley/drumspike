# AGENTS.md: drumspike

Read this file first. Read other documents only for the item you work on. Humans start with
`README.md` and `CONTRIBUTING.md`; the full developer guide is `docs/DEVELOPING.md`.

## What it is

Drum Spike: close-mic drum hit detection with a spectral onset detector (DSP), refined to the first
sample of the attack, with velocities and sample-exact MIDI and spike files per track. The product
is the desktop app (Tauri 2, Rust crates `drumspike-core` and `drumspike-app`). The app has no
Python. Python is the test tool (pytest, the browser tests through the bridge, the self-test
tools, the parity checks of the Rust binaries) and the research tools in `tools/`.

## Map

| Path | Content |
|---|---|
| `static/` | the editor: `app.js` (search it, do not read it whole), `backend.js` (object `B`: one method per app operation), `lanes-gl.js` (WebGL2 wave lanes), `tiles-worker.js` (tile textures; the page holds no PCM), `keymap.js`, `shell.js`, `index.html`, `style.css` |
| `crates/core/` | `drumspike-core`: audio I/O, peaks, `refine.rs` (attack refinement), `analyze.rs`, `velocity.rs`, `midi/`, `spike.rs`, `project/`, `tempo_detect.rs`, `transport/`, `detect/` (features, onset, attribution, tom bleed, roll, flam, kick rescue) |
| `crates/app/` | `drumspike-app`: the app commands as plain functions (no Tauri dependency): tracks, analysis jobs, export, transport, Learn, review queue, agent API (`api.rs`, `call.rs`, `cli.rs`, `mcp.rs`; `docs/agent-api.md`), relink, import, autosave |
| `crates/devtools/` | developer tools, not product code: `parity`, `bench`, `cache_tool`, `service_tool`, `ds-bridge` (test bridge, debug only) |
| `apps/desktop/` | the Tauri shell (`src-tauri/`: menu, window, open, about, self-test), scripts (`build-dmg.sh`, `release.sh`, `self-test.sh`, `gen_third_party.py`), `THIRD_PARTY.md`, `selftest/` scenarios |
| `tests/` | pytest: `e2e_app/` (the page against `ds-bridge`), `jscov.py` (JS coverage gate), `frozen.py` and `golden/` (frozen references), parity and export tests |
| `tools/` | `parity/` checks, `check_exports.py` (independent export checker), `release/cut.py` (release cut), `media/` (guide pictures, recordings, explainers), `eval/` and research tools (each has a test or a documented command) |
| `site/` | the landing page (`index.html`, `assets/`; `tools/media/build_site.py` builds it for a sub-path) |
| `docs/` | `guide/` (user guide), `DEVELOPING.md`, `RELEASING.md`, `release-notes/`, `adr/`, `design/`, `agent-api.md`, `product/` |

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
- Once before hand-back: G1 (fmt, clippy, cargo test), the tool builds and G2. G2 contains the
  `e2e_app` tests. G3 and G5 only when the change touches `apps/desktop/` or the ticket names a
  self-test.
- A push to a `crew/**` branch starts no CI run: the local gate is the gate. CI runs on `main`.
- No `[skip ci]` in a PR title or in a commit that changes code. Docs-only changes need no
  marker: CI ignores `docs/` by path.

## Invariants

- Python only through uv (`uv run ...`), Python 3.12 from `.python-version`. A new Python file
  needs a user (a test, a script, a documented command or an import).
- Never run `rustup default`. `rust-toolchain.toml` pins `stable`.
- Coverage gate: `static/app.js`, `backend.js`, `lanes-gl.js`, `tiles-worker.js`, `keymap.js` at
  85 % of lines each, measured by `tests/e2e_app`. Do not delete or weaken a test.
- Parity: hit times, velocities and MIDI bytes are exact. The references in `tests/golden/frozen/`
  are frozen.
- Source audio is never modified. Project format 1 stays loadable.
- No audio files, real session names or `target/` in git. Real sessions for local checks
  live in `~/.cache/drumspike/sessions/Session A` to `Session I` (outside the repository).
- Licence: the source code is Apache-2.0 (`LICENSE`); attributions, exceptions and the media and
  brand terms are in `NOTICE` and `docs/licensing.md`. Keep `NOTICE` current when a bundled
  third-party part changes.
- Every public Rust function has a unit test. Documents use plain hyphens, no em-dashes.

## Branches and PRs

- Work happens through crewboard tickets: one ticket, one branch `crew/<agent>/<issue>-<slug>`,
  one PR with `Closes #<n>`. The coordinator verifies and merges; agents never merge.
- Keep the first line of a commit message short. Put measurements in a report file, not in the
  commit body.

## Do not touch

- `rust-toolchain.toml`
- `tests/golden/*`: frozen references; they cannot be written again
- `.github/workflows/*`, `.crewboard.toml`, `AUTHORITY.md`: maintainers only
