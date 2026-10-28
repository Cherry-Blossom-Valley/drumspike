"""#177: the tom pair risk measurement (`tools/tom_pair_risk.py`). Synthetic signals and events only;
no session file, no model and no HOME content needed."""
import json
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import tom_pair_risk as tr  # noqa: E402

SR = 48000


def shot_synth(sr=SR, f=110.0, n=int(0.45 * SR)):
    """One tom-like shot: a decaying sine with a sharp attack."""
    t = np.arange(n) / sr
    x = np.sin(2 * np.pi * f * t) * np.exp(-t * 12)
    x[: int(0.002 * sr)] *= np.linspace(0, 1, int(0.002 * sr))
    return x * tr.lin(tr.NORMAL_DBFS) / np.abs(x).max()


def test_pitch_up_changes_rate_and_length():
    x = shot_synth()
    up = tr.pitch_up(x, 12.0)
    assert abs(len(up) - len(x) / 2.0) <= 1        # one octave up: half the samples
    assert np.abs(up).max() > 0
    assert np.argmax(np.abs(up)) < len(up) // 4    # the attack stays at the front
    same = tr.pitch_up(x, 0.0)
    assert len(same) == len(x)


def test_extract_shot_takes_the_loudest_isolated_hit():
    sr = SR
    x = np.zeros(sr * 6)
    quiet = shot_synth(sr) * 0.2
    loud = shot_synth(sr)
    tr.put(x, quiet, 1.0, sr)
    tr.put(x, loud, 3.5, sr)          # the loudest, 1 s clear on both sides
    tr.put(x, quiet, 5.5, sr)
    shot, t0 = tr.extract_shot(x, sr)
    assert len(shot) == int(tr.SHOT_LEN_S * sr)
    assert 3.4 < t0 < 3.6
    assert abs(20 * np.log10(np.abs(shot).max()) - tr.NORMAL_DBFS) < 0.1


def test_extract_shot_needs_clear_space():
    sr = SR
    x = np.zeros(sr * 4)
    loud = shot_synth(sr)
    tr.put(x, loud, 1.0, sr)
    tr.put(x, loud, 1.4, sr)          # a second hit inside the clear space
    shot, t0 = tr.extract_shot(x, sr, clear_s=0.3)
    assert len(shot) == int(tr.SHOT_LEN_S * sr)


def test_build_set_plants_alternating_pairs(tmp_path):
    plan = {"name": "flam_g10_t1", "kind": "flam", "gap": 10, "tun": 1}
    strokes = tr.build_set(plan, shot_synth(), SR, tmp_path, n_slots=3)
    scored = [s for s in strokes if s["scored"]]
    assert len(scored) == 6                      # 2 pair strokes per slot
    assert [s["under_db"] for s in scored if s["kind"] == "grace"] == [6, 9, 12]
    quiet_tracks = [s["track"] for s in scored if s["kind"] == "grace"]
    assert quiet_tracks == ["t1", "t2", "t1"]    # alternating
    for s in scored:
        a = [q for q in scored if q["kind"] in ("main", "accent") and abs(q["t"] - s["t"] - 0.01) < 1e-6]
        if s["kind"] == "grace":
            assert a, "the main follows the grace by the gap"
    assert (tmp_path / "t1.wav").is_file() and (tmp_path / "t2.wav").is_file()
    planted = json.loads((tmp_path / "planted.json").read_text())
    assert planted["set"] == "flam_g10_t1" and len(planted["strokes"]) == len(strokes)


def test_build_set_ghost_puts_the_quiet_after(tmp_path):
    plan = {"name": "ghost_g20_t3", "kind": "ghost", "gap": 20, "tun": 3}
    strokes = tr.build_set(plan, shot_synth(), SR, tmp_path, n_slots=2)
    ghost = [s for s in strokes if s["kind"] == "ghost"][0]
    accent = [s for s in strokes if s["kind"] == "accent"][0]
    assert abs((ghost["t"] - accent["t"]) - 0.020) < 1e-6
    assert ghost["track"] != accent["track"]
    assert ghost["under_db"] in tr.GHOST_UNDER and accent["under_db"] == 0


def test_render_check_passes_on_a_good_render(tmp_path):
    plan = {"name": "flam_g05_t1", "kind": "flam", "gap": 5, "tun": 1}
    tr.build_set(plan, shot_synth(), SR, tmp_path, n_slots=2)
    assert tr.render_check(tmp_path, SR, log=lambda *a: None) == []


def test_render_check_flags_a_missing_stroke(tmp_path):
    plan = {"name": "flam_g05_t1", "kind": "flam", "gap": 5, "tun": 1}
    tr.build_set(plan, shot_synth(), SR, tmp_path, n_slots=2)
    planted = json.loads((tmp_path / "planted.json").read_text())
    scored = [s for s in planted["strokes"] if s["scored"]]
    scored[0]["t"] = 4.2                           # a silent region: the audio does not carry it
    planted["strokes"] = scored + [s for s in planted["strokes"] if not s["scored"]]
    (tmp_path / "planted.json").write_text(json.dumps(planted))
    bad = tr.render_check(tmp_path, SR, log=lambda *a: None)
    assert len(bad) == 1 and bad[0][0] == scored[0]["track"]


def test_stroke_fate_splits_found_rule_and_dsp():
    events = {"t1": [{"t": 1.0}, {"t": 2.0, "bleed": True}], "t2": [{"t": 1.005}]}
    strokes = [{"track": "t1", "t": 1.0, "scored": True},
               {"track": "t1", "t": 2.0, "scored": True},
               {"track": "t2", "t": 3.5, "scored": True}]
    fates = {s["t"]: s["fate"] for s in tr.stroke_fate(events, strokes)}
    assert fates[1.0] == "found"
    assert fates[2.0] == "rule"
    assert fates[3.5] == "dsp"


def test_diff_marks_takes_the_new_flags_only():
    v2 = {"a": [{"t": 1.0, "bleed": True}, {"t": 2.0}], "b": [{"t": 1.005}]}
    v3 = {"a": [{"t": 1.0, "bleed": True}, {"t": 2.0, "bleed": True}],
          "b": [{"t": 1.005, "bleed": True}]}
    marks = tr.diff_marks(v2, v3)
    assert marks["a"] == [{"t": 2.0, "bleed": True}]
    assert len(marks["b"]) == 1


def test_guard_sweep_picks_the_smallest_clean_threshold():
    rows = {"all": [{"cause": "real stroke (own-mic attack rise)", "rise_own": 11.0, "layer_ms": None},
                    {"cause": "real stroke (own-mic attack rise)", "rise_own": 7.5, "layer_ms": None},
                    {"cause": "bleed", "rise_own": 3.0, "layer_ms": None}]}
    steps, chosen, lay = tr.guard_sweep(rows)
    assert chosen == ("rise", 6)               # keeps both real strokes, no bleed merge unmarked
    assert steps[0] == ("rise", 6, 2, 0)
    assert lay == (0, 2)


def test_guard_sweep_refuses_a_threshold_that_unmarks_bleed():
    rows = {"all": [{"cause": "real stroke (own-mic attack rise)", "rise_own": 13.0, "layer_ms": None},
                    {"cause": "bleed", "rise_own": 14.0, "layer_ms": None}]}
    steps, chosen, lay = tr.guard_sweep(rows)
    assert chosen is None                       # every X that keeps the real stroke frees the bleed
    assert ("rise", 12, 1, 1) in steps


def test_rule_version_reads_the_rust_const():
    v = tr.rule_version()
    assert v == 5  # #368: kick bleed on the floor tom, core rule v5 (the Rust test pins the same)
    src = (ROOT / "crates" / "core" / "src" / "detect" / "tom_bleed.rs").read_text()
    assert f"pub const RULE_VERSION: u32 = {v};" in src


def test_write_report_renders_tables_and_verdict(tmp_path, monkeypatch):
    res = {"when": "test", "host": "test-host", "sha": "deadbee",
           "sets": {"rows": [["flam_g05_t1", "flam", 5, 1, 24, 24, 0, 0, "OK"]],
                    "solo_rows": [[12, 1, 8, 8, 0]]},
           "sessions": {"Session A": {"merges": [{"track": "t1", "t": 10.5, "rel_db": -9.0,
                                                "rise_own": 12.0, "layer_ms": 1.0,
                                                "partner": "t2", "gain_db": 7.0,
                                                "cause": "real stroke (tom sample layer)"}],
                                    "counts": {"all": 1, "real": 1, "bleed": 0, "other": 0},
                                    "v2_marks": 2, "tom_hits": 100, "job_pairs": {"on": True, "removed": 1}}},
           "guard": {"steps": [("rise", 6, 1, 0)], "chosen": ("rise", 6), "layer": (1, 1)}}
    report = tmp_path / "tom-pair-risk.md"
    monkeypatch.setattr(tr, "REPORT", report)
    tr.write_report(res, log=lambda *a: None)
    text = report.read_text()
    assert "planted" in text and "found" in text and "lost" in text
    assert "Verdict" in text and "real strokes" in text
    assert "6 dB or more" in text and "not shipped" in text
    assert "10.5" in text and "tom sample layer" in text
    assert f"rule version {tr.rule_version()}" in text
