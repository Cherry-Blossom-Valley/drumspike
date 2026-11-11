# AGENTS.md: drumspike

Read this file first. It holds only what the code does not show: the rules that past mistakes
taught. Everything else is in the repository: the map, the invariants and the glossary in
`ARCHITECTURE.md`, the commands in `justfile` (`just --list`) and `docs/DEVELOPING.md`, the
agent API in `docs/agent-api.md`. Read other documents only for the item you work on.

## Rules from past mistakes

- Python only through `uv run` (`.python-version` pins 3.12). A bare `python3` is a different
  interpreter on every host. A new Python file needs a user: a test, a script or a documented command.
- Never run `rustup default`. `rust-toolchain.toml` pins `stable`.
- The repository and the board are public; all test data is internal. No audio, derived file,
  session, song, take or file name of the real sessions, ENST or STAR Drums in a commit, PR body,
  ticket comment or report. Public text carries aggregates and "Session A", "Session B". Before
  `done`: `uv run python tools/check_private.py --diff origin/main` and
  `--text-file <draft>`. Exit 2 means `DRUMSPIKE_PRIVATE_WORDS` is unset on this host.
- Kill only processes you started, by PID. Never `pkill` by name: the owner's installed app and
  sessions run on the same Macs.
- Documents use plain hyphens, no em-dashes (`tests/test_docs_wording.py` fails on one).
- No `[skip ci]` in a PR title or a commit. A push that changes only `docs/**` or the root files `AGENTS.md`, `CLAUDE.md`, `README.md`, `.crewboard.toml`, `AUTHORITY.md` starts no run by path; any other root Markdown file starts one.
- Keep `NOTICE` current when a bundled third-party part changes.

## Which gates when

- While you work: only the affected tests (`cargo test -p <crate> <name>`, `uv run pytest <file>`).
- Once before hand-back, scoped like CI (`tools/ci/scope.py`): G1 (fmt, clippy with
  `--all-targets`, `cargo test --workspace`) when the change touches Rust or the app; the pytest
  suite without `tests/e2e_app` when it touches only `tools/`, `site/` or non-e2e tests; the
  `e2e_app` tests when it touches the app, `static/` or the e2e tests. The app build and the
  self-test only when it touches `apps/desktop/`. The ENST gate and the real-session check only
  for detector code.
- A push to `crew/**` starts no CI run, and you do not dispatch one. The local gate is the gate;
  main's CI runs the full scope after the merge. Run long gates in the background with a log file.
- A ticket that adds a Tauri command needs `cargo test -p drumspike-app` in its gate.

## Design rules

The invariants of `ARCHITECTURE.md` hold for every change. A change that cannot keep one stops
and asks; it adds no exception. "Where a new thing goes" in `ARCHITECTURE.md` names the places
of a command, a session rule, a hit mark, a page feature and a menu item.

- Dependencies point inward only: page -> `backend.js` -> shell -> `drumspike-app` ->
  `drumspike-core`. Core reads no environment and takes no `serde_json::Value` as a domain type.
  Configuration is read once at the edge (`ServiceConfig`, `RuleSet`) and passed down typed.
- One concern per change. A refactoring (behaviour stays) and a behaviour change go in
  separate commits. A refactoring of detection or export code proves "no output change" with the
  identity snapshot (`tools/identity.py snapshot`, then `compare` against the base commit).
- To change a signature with callers: add the new form, move the callers, remove the old form.
- A detection change is a new rule version. It needs proof on ENST and the real sessions that
  accuracy is kept or better. No tuning of a threshold to named hits: principle first, then
  owner-placed ground truth.
- Use the glossary words of `ARCHITECTURE.md` in names, comments and tests. Comments say why.

## Test rules

1. A test names a domain invariant or a user-visible result. A defect fix starts with a test
   that fails for the defect. Test through the public entry point, not the private steps.
2. A test may be deleted when it only pins a removed script, or a fixed bug that another test
   covers. The PR names the deleted test and the covering test.
3. Never delete or weaken: `tests/test_golden.py`, `tests/frozen.py`, `tests/golden/*`,
   `tests/test_exports.py`, `tests/test_layer_exports.py`, the identity snapshot, the ENST gate,
   the real-session check, the tests of source-audio safety and of project format 1 loading, and
   `tools/check_private.py`.
4. JS coverage is gated per file: 70 % of the lines of each file in `JS_FILES`
   (`tests/jscov.py`), measured by `tests/e2e_app`. A new script in `static/` enters `JS_FILES`.
5. Every public Tauri command and every core entry point has a test. A script in `tools/` has a
   test or a documented command.

## Branches and PRs

- One crewboard ticket, one branch `crew/<agent>/<issue>-<slug>`, one PR with `Closes #<n>`.
  The coordinator verifies and merges; agents never merge.
- Keep the first line of a commit message short. Measurements go in a report file, not in the
  commit body or the PR.

## Do not touch

- `rust-toolchain.toml`
- `tests/golden/*`: frozen references; they cannot be written again
- `.github/workflows/*`, `.crewboard.toml`, `AUTHORITY.md`: maintainers only
