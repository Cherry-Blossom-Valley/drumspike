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

- A feature is a Rust command first (`crates/app`), and the page only shows it. Every public Rust
  function has a unit test.
- Do not delete or weaken a test. The JS coverage gate (85 % of lines per file) and the frozen
  parity references (`tests/golden/`) stay as they are.
- Source audio is never modified, and project format 1 stays loadable.
- No audio files or build output in git.
- Python only through `uv run`; a new Python file needs a user (a test, a script or a documented
  command).
- Documents: short sentences, plain hyphens.
- Keep the first line of a commit message short.

Pull requests from forks run no job on our self-hosted runners. A maintainer runs the full gate
before the merge.

## Licence of contributions

Drum Spike is free and open source (Apache-2.0). By opening a pull request you agree that your
contribution is licensed under the Apache License 2.0 (section 5 of the licence). The name, the
icon, the logo and the media stay © Cherry Blossom Valley ([docs/licensing.md](docs/licensing.md)).

## Coding agents

The repository is also maintained with coding agents. Their rules and the repository map are in
[AGENTS.md](AGENTS.md); they are a good short reference for human contributors too.
