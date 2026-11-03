# Contributing to Drum Spike

Thank you for helping. Bug reports, ideas, documentation fixes and code are all welcome.

## Report a bug or ask for a feature

Open an [issue](https://github.com/Cherry-Blossom-Valley/drumspike/issues/new/choose) with the
form for bugs or features. For a bug, give the Drum Spike version (Help > About), your Mac
(Apple silicon or Intel) and macOS version, the steps, and what you expected. A short audio
example helps, if you may share it. Security problems do not go into an issue: see
[SECURITY.md](SECURITY.md).

## Change the code

1. Fork the repository and make a branch from `main`.
2. Build and test as in [docs/DEVELOPING.md](docs/DEVELOPING.md). The short form:

   ```bash
   uv sync && uv run playwright install chromium
   cargo build --release -p drumspike-devtools && cargo build -p drumspike-devtools --bin ds-bridge
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test --workspace
   DRUMSPIKE_AUDIO=null DRUMSPIKE_REQUIRE_APP=1 DRUMSPIKE_REQUIRE_CORE=1 uv run pytest -q
   ```

3. Open a pull request against `main`. Say what changes for the user and how you tested it.
   One topic per pull request.

## Rules that keep the project healthy

- A feature is a Rust command first (`crates/app`), and the page only shows it.
- Tests follow the test rules below.
- Source audio is never modified, and project format 1 stays loadable.
- No audio files or build output in git.
- Private names (session or take names, dataset file names, host paths) never go into a commit,
  a PR body or a ticket comment. The list is set per host in the environment
  (`DRUMSPIKE_PRIVATE_WORDS`, `DRUMSPIKE_PRIVATE_PATTERNS`), never in the repo. Check text
  with `uv run python tools/check_private.py --diff origin/main` or `--text-file <draft>`.
- Python only through `uv run`; a new Python file needs a user (a test, a script or a documented
  command).
- Documents: short sentences, plain hyphens.
- Keep the first line of a commit message short.

### Test rules

1. A test names a domain invariant or a user-visible result in its name or first line.
2. A test may be deleted when it only pins a removed script, or a fixed bug that another test
   covers. The PR names the deleted test and the covering test.
3. Never delete or weaken: `tests/test_golden.py`, `tests/frozen.py`, `tests/golden/*`,
   `tests/test_exports.py`, `tests/test_layer_exports.py`, the identity snapshot, the ENST gate,
   the real-session check, the tests of source-audio safety and of project format 1 loading, and
   `tools/check_private.py`.
4. JS coverage is gated per file, not as a total. The floor is 70 % of the lines of each file
   (`JS_FAIL_UNDER` in `tests/jscov.py`), measured by `tests/e2e_app`. The files are the 11 of
   `JS_FILES`: `app.js`, `state.js`, `persist.js`, `edit.js`, `transport.js`, `render.js`,
   `session.js`, `backend.js`, `lanes-gl.js`, `tiles-worker.js` and `keymap.js`. A new script in
   `static/` enters `JS_FILES` at the same floor.
5. Every public Tauri command and every core entry point has a test.
6. A script in `tools/` has a test or a documented command.

Pull requests from forks run no job on our self-hosted runners. A maintainer runs the full gate
before the merge.

## Licence of contributions

Drum Spike is free and open source (Apache-2.0). By opening a pull request you agree that your
contribution is licensed under the Apache License 2.0 (section 5 of the licence). The name, the
icon, the logo and the media stay © Cherry Blossom Valley ([docs/licensing.md](docs/licensing.md)).

## Coding agents

The repository is also maintained with coding agents. Their rules and the repository map are in
[AGENTS.md](AGENTS.md); they are a good short reference for human contributors too.
