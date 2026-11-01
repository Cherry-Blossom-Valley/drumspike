# Developing Drum Spike

This guide shows how to build, run, test and release Drum Spike. Run every command from the
repository root unless the text says otherwise. For contributions, read `CONTRIBUTING.md`. For the
rules that coding agents follow in this repository, read `AGENTS.md`.

## 1. The parts

| Part | Path | What it is |
|---|---|---|
| Core | `crates/core/` | Rust crate `drumspike-core`. Audio reader, peaks, refinement, velocities, MIDI, spike WAVs, project format, detection. No UI. See `crates/core/README.md`. |
| App service | `crates/app/` | Rust crate `drumspike-app`. The commands of the desktop app as plain functions, with no Tauri dependency. It also has the test bridge `ds-bridge` (debug builds only) and `service_tool` (the service commands and `core-analyze` for the parity checks). |
| Desktop app | `apps/desktop/` | Tauri 2 shell (crate `drumspike-desktop` in `apps/desktop/src-tauri/`). It wraps the UI in `static/`. The scripts for build, test and release are in `apps/desktop/scripts/`. |
| UI | `static/` | The editor: plain JavaScript (`app.js`, `backend.js`, `lanes-gl.js`, `tiles-worker.js`). It runs in the app (IPC backend) and, for the tests, in Chromium against the test bridge. |
| Tests | `tests/`, `tools/parity/` | pytest and the parity scripts. An earlier Python reference implementation was retired; its results are frozen in `tests/golden/`. |
| Python tools | `tools/` | Research tools (calibration kit, tom bleed, onset checks, `fix_metric.py`, `firm_fixes.py`) and `tools/check_exports.py`, the independent export checker. They are not part of the app. Each has a user: a test, an import or a documented command. |

## 2. Prerequisites

Drum Spike builds on macOS, on an Apple Silicon Mac. The desktop app builds for Apple Silicon
(arm64) and, as a cross build on the same Mac, for Intel (x86_64).

- Xcode command line tools: `xcode-select --install`. They give `codesign`, `hdiutil` and the linker.
- Rust: install `rustup` only. The file `rust-toolchain.toml` pins the channel `stable`, and
  `cargo` reads it. Never run `rustup default` for this project.
- `uv` and Python 3.12. `.python-version` holds `3.12`. `uv` downloads that Python when it is missing.
- Node 22 (the version the builds on arm64-mac use) and `npm`, for the Tauri command line tool. The only npm package is `@tauri-apps/cli`
  (`apps/desktop/package.json`).
- No ffmpeg for the app and the tests. Only the media tools in `tools/media/` use it.
- `just` is optional. The `justfile` wraps the test commands (`just --list`).

Never use bare `python3` for this project. Always use `uv run`.

## 3. First-time setup

Install the Python test environment (pytest, Playwright, numpy, httpx):

```bash
uv sync
```

Build the Rust tools the tests call (the parity binaries, `service_tool`, the test bridge):

```bash
cargo build --release -p drumspike-devtools
cargo build -p drumspike-devtools --bin ds-bridge
```

Install the browser for the end-to-end tests (once):

```bash
uv run playwright install chromium
```

Install the Tauri command line tool (once, for the desktop app):

```bash
(cd apps/desktop && npm ci)
```

## 4. Run the desktop app in development

Build the debug app (this is gate G3, see section 8):

```bash
(cd apps/desktop && npx tauri build --debug --target aarch64-apple-darwin --bundles app)
```

The result is in `target/aarch64-apple-darwin/debug/bundle/macos/Drum Spike.app`. Start it:

```bash
open "target/aarch64-apple-darwin/debug/bundle/macos/Drum Spike.app"
```

The self-test scripts (section 8) use this debug app. The debug app also has the self-test mode
that release builds do not have.

The Tauri command line tool has a live mode too:

```bash
(cd apps/desktop && npx tauri dev)
```

The project reports do not record a `tauri dev` run. The documented way to run the app is the
debug `.app` above. `tauri.conf.json` has no dev server address (`frontendDist` is `../../../static`),
so the app loads the files in `static/` directly.

## 5. Python in this repository, and the retired Python server

The app has no Python. Python is the test tool (pytest, the browser tests, the parity checks),
the helper scripts of the self-test and the release, and the research tools in `tools/`.
Run a research tool with named dependencies, for example
`uv run --with numpy --with soundfile --with mido python tools/fix_metric.py all`; its
docstring has the usage. A Python file with no test, script, import or documented command is
removed.

### The retired Python reference

An earlier Python implementation (a FastAPI server with a PyO3 binding) was the reference for the
parity tests. It was retired when the Rust core took over; its results are frozen as golden files
in `tests/golden/` (`tests/frozen.py` reads them). The frozen files name the commits of the
development history they were recorded at; that history is not part of this repository.

## 6. Build the release app

The release build is done by `build-dmg.sh` (section 7). It builds the release `.app` as its first
step. The `.app` is in `target/aarch64-apple-darwin/release/bundle/macos/Drum Spike.app`.

## 7. Build the ad hoc signed .dmg

```bash
apps/desktop/scripts/build-dmg.sh
```

```bash
apps/desktop/scripts/build-dmg.sh --arch x86_64
```

The default architecture is `arm64`. The script does these steps:

1. Checks that `apps/desktop/THIRD_PARTY.md` is current.
2. Builds the release `.app` (`npx tauri build`, bundle `app`) for the architecture.
3. Checks that the app bundles no dynamic library.
4. Signs the `.app` ad hoc with the hardened runtime, and verifies it.
5. Compares every bundled resource with the repository file by SHA-256 (notices).
6. Makes the `.dmg` with `hdiutil` and signs it ad hoc.

`--app-only` stops after step 5 (no `.dmg`). `--dmg-only` does only step 6, with the `.app` that is
there, and builds nothing. `release.sh` uses the two phases to staple the `.app` before the `.dmg` is
made (section 7a). Without a phase flag the script does all steps.

The results:

| Architecture | File |
|---|---|
| `arm64` | `target/aarch64-apple-darwin/release/bundle/dmg/Drum Spike_<version>_aarch64.dmg` |
| `x86_64` | `target/x86_64-apple-darwin/release/bundle/dmg/Drum Spike_<version>_x86_64.dmg` |

Nothing here is committed to git. Set `DRUMSPIKE_BUNDLE_DIR` to change the output folder.

The Intel build needs one thing once:

```bash
rustup target add x86_64-apple-darwin
```

An ad hoc signed `.dmg` runs on the Mac that built it. Another Mac shows a Gatekeeper warning.
The ad hoc Intel app turns library validation off (`entitlements-adhoc-x86_64.plist`), because an
ad hoc signature has no Team ID. Use it only for tests; never give it to anyone. For other Macs,
use the signed and notarized release (section 7a).

If the third-party list is stale, the script stops. Regenerate it:

```bash
uv run --no-project --python 3.12 apps/desktop/scripts/gen_third_party.py
```

Check it without a change (this is what the script and CI run):

```bash
uv run --no-project --python 3.12 apps/desktop/scripts/gen_third_party.py --check
```

### 7a. Signed and notarized release, and publishing

Only the owner does this, on the Mac with the Developer ID certificate in its keychain.
Agents and CI never sign, never notarize and never publish. The script reads no password and no
key. It handles only two names:

```bash
export DRUMSPIKE_SIGN_IDENTITY="<name of the Developer ID Application certificate>"
```

```bash
export DRUMSPIKE_NOTARY_PROFILE=<name of the notarytool keychain profile>
```

Print the plan without a build, a signature, a keychain call or a `gh` command:

```bash
apps/desktop/scripts/release.sh --dry-run
```

Build, sign, notarize and staple both architectures (`--arch arm64` or `--arch x86_64` for one).
For each architecture the script notarizes and staples the `.app` first, then makes the `.dmg` of
the stapled `.app`, then notarizes and staples the `.dmg`:

```bash
apps/desktop/scripts/release.sh
```

Then check the Intel app on an Intel Mac (`docs/RELEASING.md`, "Check the Intel app"). Publish only
when that check passes. Publishing makes the tag `v<version>` (from `tauri.conf.json`) and a GitHub Release with
both `.dmg` files, `SHA256SUMS` and the notes, through your `gh` login:

```bash
apps/desktop/scripts/release.sh --publish-only
```

The script stops when the tag or the release exists (`--replace` deletes the old ones first), when
the work tree is not clean, or when `HEAD` is not pushed. The tag points to `HEAD`, so publish from
the commit that you built. `--publish` in the first run does both steps at once, but it skips the
Intel check.

The full procedure, the checks on a second Mac, the entitlements and the error table are in
`docs/RELEASING.md`. In this repository the release workflow (`.github/workflows/release.yml`) runs
these steps on a tag push and makes a draft release; a maintainer publishes it.

## 8. Tests

The gates have short names (G1 to G6) that tickets and pull requests use. Run long gates in the
background with a log file.

| Gate | Host | Commands |
|---|---|---|
| G1 Rust | arm64-mac and x86-mac | `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` |
| G2 pytest | arm64-mac and x86-mac | build the tools (section 3), then `DRUMSPIKE_AUDIO=null DRUMSPIKE_REQUIRE_APP=1 DRUMSPIKE_REQUIRE_CORE=1 uv run pytest -q` |
| G3 app build | arm64-mac only | `(cd apps/desktop && npm ci && npx tauri build --debug --target aarch64-apple-darwin --bundles app)` |
| G4 app e2e through the bridge | arm64-mac and x86-mac | `cargo build -p drumspike-devtools --bin ds-bridge`, then `DRUMSPIKE_AUDIO=null DRUMSPIKE_REQUIRE_APP=1 uv run pytest -m e2e_app --no-cov` |
| G5 app self-test in the built app | arm64-mac only | `apps/desktop/scripts/self-test.sh <scenario>` |
| G6 x86 check | x86-mac | `X86_REPO=~/src/<clone> X86_RUN=~/x86run-<job> ~/x86run.sh` |

A faster loop without the browser tests:

```bash
uv run pytest -m "not e2e_app"
```

The media tests (explainer scripts, recordings and renders under `tools/media/`) are marked
`media` and deselected by default (#277); run them on purpose with `uv run pytest -q -m media`
(or `DRUMSPIKE_MEDIA_TESTS=1`), and only when a fundamental app feature or the media sources
and tools changed.

Coverage gate: `static/app.js`, `backend.js`, `lanes-gl.js` and `tiles-worker.js` at least 85 % of
lines each (`DRUMSPIKE_JS_FAIL_UNDER`), measured by Chromium in `tests/e2e_app`. It applies to G2 and
to G4 (`-m e2e_app` alone). There is no Python coverage gate any more: no product code is Python.
Do not delete or weaken a test.

### Rules for the gates

- Build before G2 (section 3). The tests that call `parity` or `service_tool` skip when the release
  binaries are missing; `DRUMSPIKE_REQUIRE_CORE=1` makes that skip a failure (`tests/core_tools.py`).

- Build the bridge before G4. The `e2e_app` tests skip when `ds-bridge` is missing.
  `DRUMSPIKE_REQUIRE_APP=1` makes that skip a failure.
- `DRUMSPIKE_AUDIO=null` selects the null audio device, so the tests need no sound card.

### The e2e_app tests and the bridge

`ds-bridge` (`crates/devtools/src/bin/ds-bridge.rs`) is a debug-only HTTP server on `127.0.0.1`. It serves
`static/` and `POST /invoke/<command>`, and it calls the same Rust commands as the app. The tests in
`tests/e2e_app/` load the real page in Chromium against it. Set `DRUMSPIKE_BRIDGE` to use another
binary than `target/debug/ds-bridge`.

### The app self-test

`apps/desktop/scripts/self-test.sh <scenario>` runs a scenario in the debug `.app` (build it first,
gate G3) and checks the report. Scenarios are in `apps/desktop/selftest/`: `slice`, `shell`, `tracks`,
`render`, `play`, `memory`. Run two of them:

```bash
apps/desktop/scripts/self-test.sh slice
```

```bash
apps/desktop/scripts/self-test.sh shell
```

The scenarios `slice`, `shell` and `tracks` read a real session. The default is
`~/.cache/drumspike/sessions/Session A` (it needs an `Audio Files/` folder). Set
`DRUMSPIKE_SELFTEST_SESSION` for another folder. The scenarios `render`, `play` and `memory` use
a generated session. The full commands:

```bash
DRUMSPIKE_SELFTEST_WAVS=<make_session.py folder> DRUMSPIKE_SELFTEST_TRACKS=30 apps/desktop/scripts/self-test.sh render   # G5: 30 lanes, real playback
```

```bash
DRUMSPIKE_SELFTEST_SYNTH=<make_session.py folder> apps/desktop/scripts/self-test.sh play   # G5: 60 s on the device, silent (master gain 0)
```

```bash
DRUMSPIKE_SELFTEST_WAVS=<make_session.py folder> DRUMSPIKE_SELFTEST_TRACKS=30 apps/desktop/scripts/self-test.sh memory   # G5: WebContent footprint over open, 60 s play and zoom, open to drawn; `"diag": true` in memory.json adds the attribution runs
```

The app plays silent during a self-test (`DRUMSPIKE_MASTER_GAIN=0`) unless you set
`DRUMSPIKE_SELFTEST_AUDIBLE=1`. The self-test opens a real window. The screen must stay awake.

`apps/desktop/scripts/finder-open.sh` tests the Finder path (`open -a`, cold and warm start) of the
debug app.

### Parity and benchmark

The retired Python reference is frozen in `tests/golden/` (`tests/frozen.py` reads and checks the
values; a reference keyed by changed inputs fails). The parity scripts are in `tools/parity/`, and
each has its usage in its docstring. The benchmark:

```bash
./target/release/bench
```

## 9. CI

`.github/workflows/ci.yml` runs on every push to `main` and `feat/**` (a push that changes only
`docs/` or the root Markdown files starts no run) and by hand (Actions > ci > Run workflow).
Changes to the workflow files need a maintainer.

| Job | Runner | What it does |
|---|---|---|
| `test` | self-hosted Intel Mac (labels `self-hosted`, `x86-mac`) | format, clippy, `cargo test --workspace`, the release tools, `ds-bridge`, then pytest (all tests except the arm64 share of `e2e_app`) |
| `e2e-arm64` | self-hosted Apple Silicon Mac (label `arm64-mac`) | the other share of the `e2e_app` tests (`tests/e2e_shards.json`); on `main` pushes, by hand, and on a pull request of this repository with the label `ci:e2e` |
| `js-coverage` | `ubuntu-latest` | merges the JS coverage of both shares and gates 85 % per file |
| `desktop-app` | self-hosted Apple Silicon Mac | by hand only (`run_desktop`): debug app (G3), self-test scenarios (G5), ad hoc `.dmg` |

The jobs set `DRUMSPIKE_REQUIRE_CORE` and `DRUMSPIKE_REQUIRE_APP`, so a missing tool or binary fails
the job instead of skipping tests. Pull requests from forks never start a job on a self-hosted
runner. To start `desktop-app` from the command line:

```bash
gh workflow run ci.yml -f run_desktop=true --ref <branch>
```

`.github/workflows/release.yml` runs on a tag `v*`: it checks that the tag equals the version in
the four version files, that the release notes file exists and that `ci.yml` is green on the
commit; then it builds, signs and notarizes both architectures on the Apple Silicon runner, checks
the Intel app on the Intel runner and makes a draft release.

## 10. Self-hosted runners

The macOS jobs run on two self-hosted runners: an Apple Silicon Mac (label `arm64-mac`, also the
signing Mac of the release) and an Intel Mac (label `x86-mac`). Each runner needs Rust through
`rustup`, `uv`, Node 22 and the Xcode command line tools; the jobs put `~/.cargo/bin`,
`~/.local/bin` and `/usr/local/bin` on their `PATH` themselves. Heavy jobs on a runner that is also
a development machine wait for a host lock, when the machine has one, so a test run and a local
build do not compete for the CPU.

## 11. Environment variables

Only variables that the code reads. "Test" means a variable for tests and scripts.

| Variable | Effect | Source |
|---|---|---|
| `DRUMSPIKE_HOME` | state folder (`recent.json`, `state.json`; default `~/.drumspike`) | `crates/app/src/lifecycle.rs` |
| `DRUMSPIKE_PROJECTS` | folder for new projects (default `~/Documents/Drumspike Projects`) | `crates/app/src/lifecycle.rs` |
| `DRUMSPIKE_AUTOSAVE_SECONDS` | autosave interval (default 120) | `crates/app/src/autosave.rs` |
| `DRUMSPIKE_AUDIO` | `null` selects the null audio device | `crates/core/src/transport/device.rs` |
| `DRUMSPIKE_MASTER_GAIN` | master gain after the mix; `0` is silent | `crates/app/src/transport.rs` |
| `DRUMSPIKE_STARTUP_LOG` | `1` prints the cold-start time of the app and exits; `info` also prints `app_info` | `apps/desktop/src-tauri/src/lib.rs` |
| `DRUMSPIKE_OPEN_LOG` | debug builds: file that gets every path the app receives | `apps/desktop/src-tauri/src/open.rs` |
| `DRUMSPIKE_REQUIRE_CORE` | test: the tests that call `parity` or `service_tool` fail instead of skip when the binary is missing | `tests/core_tools.py`, `tests/test_golden.py` |
| `DRUMSPIKE_FROZEN`, `DRUMSPIKE_FROZEN_EXTERNAL` | test: `replay` (default) reads the frozen references; `record` and `check` need the retired code; the folder of large values | `tests/frozen.py` |
| `DRUMSPIKE_REQUIRE_APP` | test: `e2e_app` tests fail instead of skip | `tests/e2e_app/conftest.py` |
| `DRUMSPIKE_BRIDGE` | test: path of the `ds-bridge` binary | `tests/e2e_app/conftest.py` |
| `DRUMSPIKE_JS_FAIL_UNDER` | test: line coverage gate for the JS files (default 85) | `tests/jscov.py` |
| `DRUMSPIKE_SELFTEST_SESSION`, `_WAVS`, `_SYNTH`, `_TRACKS`, `_DIR`, `_AUDIBLE`, `_APP` | self-test inputs and options | `apps/desktop/scripts/self-test.sh` |
| `DRUMSPIKE_SIGN_IDENTITY`, `DRUMSPIKE_NOTARY_PROFILE` | owner only: names for the signed release | `apps/desktop/scripts/release.sh` |
| `DRUMSPIKE_BUNDLE_DIR` | output folder of the bundle and the `.dmg` | `build-dmg.sh`, `release.sh` |
| `DRUMSPIKE_RELEASE_LOG` | notary log file of `release.sh` (default `target/release-notary.log`) | `apps/desktop/scripts/release.sh` |
| `DS_CODESIGN_IDENTITY`, `DS_CODESIGN_TIMESTAMP` | set by `release.sh` for `build-dmg.sh`; do not set by hand | `apps/desktop/scripts/build-dmg.sh` |

More test-only variables exist for diagnostics and benchmarks (`DRUMSPIKE_TRACKS_BENCH`,
`DRUMSPIKE_RESIDENCY`, `DRUMSPIKE_SESSION`, `DRUMSPIKE_REAL_SESSIONS`). Each one is explained in
the doc comment of the test that reads it (`crates/app/tests/relink.rs`,
`crates/app/tests/lanes_fast_path.rs`, `crates/core/src/transport/device/tests.rs`,
`crates/app/tests/tracks.rs`).

## 12. Troubleshooting

Only problems that the project reports record.

| Symptom | Cause and action |
|---|---|
| Parity tests skip (`... not built: cargo build --release ...`) | The release binaries are missing. Build them (section 3) before G2. |
| `FrozenError: no frozen ... reference` or `the inputs ... differ from the frozen run` | A test case changed (other inputs than at the freeze) or a host has no frozen value for the key. The references cannot be written again without the retired code (section 5). |
| `e2e_app` tests skip | `ds-bridge` is not built. Run `cargo build -p drumspike-devtools --bin ds-bridge`. Use `DRUMSPIKE_REQUIRE_APP=1` to make a skip a failure. |
| A run of a few browser test files fails the JS coverage gate | The gate applies unless `-k`, a `-m` other than `e2e_app`, or a command line that names test files or node IDs is given. Select single tests with `-k` or a node ID (`test_file.py::test_name`), or read only the test results. |
| `cargo: command not found` in a CI job | The runner process did not have the tool folders on its `PATH`. The job now adds `~/.cargo/bin`, `~/.local/bin` and `/usr/local/bin` itself. |
| `build-dmg.sh` stops at the third-party check | `THIRD_PARTY.md` is stale. Run `gen_third_party.py` (section 7) and commit the result. |
| `release.sh`: `identity not found in the keychain` | Run `security find-identity -v -p codesigning` and copy the name exactly. |
| `release.sh`: `notary profile not usable` | The profile is missing, belongs to another Mac or user, or Apple refused it. Run the check command that the script prints. Make it once with `xcrun notarytool store-credentials <profile>`. |
| Other notarization errors | Read `docs/RELEASING.md`, section "Errors". The submission ID is in `target/release-notary.log`. |
| Self-test: `no animation frames` or a timeout | The screen is locked or asleep. Keep it awake and run again. |
| The ad hoc `.dmg` shows a Gatekeeper warning on another Mac | This is expected. Only the notarized release runs there without a warning. |
| A timing test fails once on a busy host | Two timing tests failed once each on a loaded Intel Mac and passed on rerun. Run it again before you look for a bug. |
