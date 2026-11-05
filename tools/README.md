# Tools

Python and shell tools around the app. None of them is part of the app; the app has no Python.
Run a tool with `uv run` (named dependencies are in its docstring, for example
`uv run --with numpy --with soundfile python tools/fix_metric.py all`).

| Path | What it is |
|---|---|
| `check_exports.py` | the independent export checker: reads exported MIDI and spike files without project code and compares them with the session |
| `check_midi_timing.py` | checks MIDI note times against a tempo map in samples |
| `check_wording.py` | the wording check of the user-facing texts (guide, release notes, site) |
| `check_private.py` | the check for private names in a diff or a text file (`AGENTS.md`) |
| `training_export.py` | the aggregates and the truth list of the Training Mode logs (`View > Training Mode`, #343) |
| `identity.py` | the identity snapshot: hashes the engine outputs before and after a change (`ARCHITECTURE.md`) |
| `fn_lengths.py` | lists the long functions of the Rust production code |
| `build_help.py` | builds the in-app manual `static/help/` from `docs/guide/` |
| `parity/` | the synthetic corpus and session builders for the golden regression in `tests/golden/`, the bench signals and the exit-flow check |
| `release/` | `cut.py` (plan, notes, version bump and status of a release) and `licence_check.py` (every asset has its terms in `docs/licensing.md`) |
| `media/` | the guide pictures (`shots.py`), screen recordings (`record.py`), explainer videos (`explainers/`), narration (`tts.py`), the path guard for rendered media (`check_paths.py`) and the website build (`build_site.py`) |
| `eval/` | detection evaluation on the public ENST-Drums data set: `enst_gate.py` and its baseline |
| `ci/` | `scope.py` (which gates a change needs) and `loaded_repeat.sh` (a gate repeated under load) |
| `hooks/` | the pre-commit hook for rustfmt |
| `icons/` | the generator of the document icon |

## Research references

The other scripts in this folder (`calib_*`, `onset_*`, `snare_onset`, `sessions`,
`flam_pairs`, `roll_fill`, `rescue_real`, `miss_causes`, `firm_fixes`, `fix_metric`,
`learn_eval`) are the Python reference implementations and measurement tools
behind the detection rules of the Rust core. Each rule (tom bleed, tom pairs, flams, rolls, kick
walk, kick rescue) was designed and measured here first; the Rust unit tests now pin the rules, and
`tests/test_rust_kick_walk.py` still compares the kick walk with its reference. `fix_metric.py` is the yardstick: it scores a detection against the hits a
mixing engineer fixed by hand. Each script has a test or a documented command (test rules: `AGENTS.md`).
