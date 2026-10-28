"""P4-10 stage 2a: the dry run of the two rescues on real sessions (`tools/rescue_real.py`). Synthetic activations and
signals only; the real sessions are not needed."""
import math
import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import rescue_real as rr  # noqa: E402


def act_of(n, **cols):
    a = np.zeros((n, 5), dtype=np.float32)
    for i, name in enumerate(["bd", "sd", "tt", "hh", "cy"]):
        if name in cols:
            a[:, i] = cols[name]
    return a


def col(n, bumps):
    x = np.zeros(n, dtype=np.float32)
    for f, v in bumps:
        x[f] = v
    return x


def test_prf_counts_one_to_one_within_20_ms():
    r = rr.prf([1.0, 2.0, 3.0], [1.01, 1.015, 3.5])
    assert (r["tp"], r["fp"], r["fn"]) == (1, 2, 2) and abs(r["f"] - 1 / 3) < 0.001
    assert rr.prf([], [])["f"] == 0.0 or rr.prf([], [])["p"] == 1.0
    assert rr.prf([1.0], [1.0])["f"] == 1.0


def test_offset_is_the_median_of_the_matched_differences():
    assert abs(rr.offset_of([1.0, 2.0, 3.0], [1.005, 2.003, 3.004, 9.0]) - 0.004) < 1e-9
    assert rr.offset_of([1.0], [5.0]) == 0.0


def test_cross_rescue_adds_the_other_class_peak_when_the_own_column_rises():
    n = 120
    act = act_of(n, sd=col(n, [(48, 0.1), (80, 0.9)]), hh=col(n, [(48, 0.8)]))
    events = [0.81]
    lead = rr.own_lead("snare", act, events)
    assert abs(lead - 0.01) < 1e-9
    assert [round(t, 3) for t in rr.cross_rescued("snare", act, lead)] == [0.49]
    assert rr.cross_rescued("snare", act, lead, src={0}) == []                    # only kick-class peaks may be taken
    assert rr.cross_rescued("snare", act_of(n, sd=col(n, [(48, 0.01), (80, 0.9)]), hh=col(n, [(48, 0.8)])), lead) == []


def test_variant_sets_apply_each_rescue_to_its_roles():
    n = 120
    act = act_of(n, sd=col(n, [(48, 0.1), (80, 0.9)]), hh=col(n, [(48, 0.8)]))
    s = rr.variant_sets("snare", act, [0.81], [0.81, 2.0])
    assert s["base"][0] == [0.81] and len(s["cross"][0]) == 2 and s["dsp"][0] == [0.81]       # the DSP is for kicks
    assert len(s["both"][1]) == 1
    k = rr.variant_sets("kick", act_of(n, bd=col(n, [(80, 0.9)])), [0.81], [0.81, 2.0, 0.83])
    assert k["dsp"][0] == [0.81, 2.0] and k["dsp"][1] == [2.0] and k["cross"][1] == []         # 0.83 is within 30 ms
    assert k["both"][0] == k["dsp"][0]


def test_a_cross_hit_next_to_an_event_is_the_same_hit():
    n = 120
    act = act_of(n, sd=col(n, [(48, 0.1), (80, 0.9)]), hh=col(n, [(48, 0.8)]))
    assert len(rr.variant_sets("snare", act, [0.81, 0.50], [])["cross"][0]) == 2


def test_score_track_uses_the_offset_of_the_base_events():
    ev = [1.03, 2.03, 3.03]
    sets = {"base": (ev, []), "cross": (ev + [4.0], [4.0])}
    off, sc = rr.score_track([1.0, 2.0, 3.0], ev, sets)
    assert abs(off - 0.03) < 1e-9 and sc["base"]["f"] == 1.0 and sc["cross"]["fp"] == 1


def test_score_table_sums_the_roles():
    row = lambda role, tp, fp, fn: {"session": "S", "mic": "m", "role": role, "scores": {  # noqa: E731
        v: {"tp": tp, "fp": fp, "fn": fn, "n_ref": tp + fn, "f": 0.5} for v in rr.VARIANTS}}
    t = rr.score_table([row("kick", 8, 2, 2), row("kick", 2, 0, 0)]).splitlines()
    assert "10/2/0.833" in t[-1] and "| **kick** |" in t[-1]


def test_peak_envelope_and_spectrum_of_a_planted_hit():
    sr = 48000
    n = sr * 2
    t = np.arange(n) / sr
    x = np.zeros(n)
    k = int(0.3 * sr)
    tt = np.arange(k) / sr
    x[sr:sr + k] = 0.1 * np.sin(2 * np.pi * 1000 * tt) * np.exp(-tt / 0.01) * np.minimum(1, tt / 0.003)
    assert -24.0 < rr.peak_db(x, sr, 1.0) < -21.0                      # 0.1 after the 3 ms rise
    assert rr.peak_db(x, sr, 0.2) < -100
    env = rr.envelope_features(x, sr, 1.0)
    assert 1.0 < env["rise_ms"] < 4.0 and 10 < env["decay20_ms"] < 60
    a = rr.band_spectrum(x, sr, 1.0)
    b = rr.band_spectrum(0.5 * x, sr, 1.0)
    low = rr.band_spectrum(np.sin(2 * np.pi * 150 * t) * (t > 1.0), sr, 1.0)
    assert abs(rr.cosine(a, b) - 1.0) < 1e-4 and rr.cosine(a, low) < 0.5
    assert math.isclose(float(np.argmax(a)), float(np.argmax(b)))
