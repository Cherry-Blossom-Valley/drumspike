# Tools

Python and shell tools around the app. None of them is part of the app; the app has no Python.
Run a tool with `uv run` (named dependencies are in its docstring, for example
`uv run --with numpy --with soundfile python tools/fix_metric.py all`).

| Path | What it is |
|---|---|
| `check_exports.py` | the independent export checker: reads exported MIDI and spike files without project code and compares them with the session |
| `check_midi_timing.py` | checks MIDI note times against a tempo map in samples |
| `check_wording.py` | the wording check of the user-facing texts (guide, release notes, site) |
| `parity/` | the parity checks of the Rust core against the frozen references in `tests/golden/` |
| `release/` | `cut.py` (plan, notes, version bump and status of a release) and `licence_check.py` (every asset has its terms in `docs/licensing.md`) |
| `media/` | the guide pictures (`shots.py`), screen recordings (`record.py`), explainer videos (`explainers/`), narration (`tts.py`), the path guard for rendered media (`check_paths.py`) and the website build (`build_site.py`) |
| `eval/` | detection evaluation on the public ENST-Drums data set |

## Research references

The other scripts in this folder (`calib_*`, `onset_*`, `kick_onset`, `snare_onset`, `tom_*`,
`flam_*`, `roll_fill`, `rescue_real`, `miss_causes`, `firm_fixes`, `fix_metric`, `learn_*`,
`layer_eval`, `audio_spikes`) are the Python reference implementations and measurement tools
behind the detection rules of the Rust core. Each rule (tom bleed, tom pairs, flams, rolls, kick
walk, kick rescue) was designed and measured here first; the Rust parity tests compare the core
with these references. `fix_metric.py` is the yardstick: it scores a detection against the hits a
mixing engineer fixed by hand. Each script has a test in `tests/test_<name>.py`.
