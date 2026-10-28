"""P4-13 stage 2, stage A: the evidence tool (`tools/flam_split.py`). Synthetic signals only; no session file."""
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import flam_pairs as fp  # noqa: E402
import flam_split as fs  # noqa: E402

SR = 48000


def hit(n, at, amp=1.0, tau=0.008, length=0.15, seed=0):
    rng = np.random.default_rng(seed)
    y = np.zeros(n)
    a = int(at * SR)
    k = min(int(length * SR), n - a)
    y[a:a + k] = amp * rng.standard_normal(k) * np.exp(-np.arange(k) / (tau * SR))
    return y


def test_classify_priority_and_looks_false():
    assert fs.classify({"owner": True, "in_ref": True, "layer": True}) == "owner"
    assert fs.classify({"in_ref": True, "layer": True}) == "reference"
    assert fs.classify({"layer": True, "witness": True}) == "layer"
    assert fs.classify({"witness": True}) == "witness"
    assert fs.classify({"witness": True, "hat_cause": True}) == "hat"
    assert fs.classify({"hat_cause": True}) == "none"
    assert fs.classify({}) == "none"
    assert [fs.looks_false(c) for c in ("owner", "reference", "layer", "witness", "hat", "none")] == \
        [False, False, False, False, True, True]


def test_tally_counts_and_share():
    rows = [{"cls": c} for c in ("owner", "layer", "none", "hat", "none")]
    t = fs.tally(rows)
    assert (t["added"], t["owner"], t["layer"], t["hat"], t["none"], t["false"]) == (5, 1, 1, 1, 2, 3)
    assert abs(t["share"] - 0.6) < 1e-12
    assert fs.tally([])["share"] is None


def test_owner_found_window_edges():
    assert fs.owner_found([10.014], [10.0]) == 1
    assert fs.owner_found([10.016], [10.0]) == 0
    assert fs.owner_found([9.986], [10.0]) == 1
    assert fs.owner_found([], [10.0]) == 0
    assert fs.near([], 1.0, 0.1) is False


def test_the_owner_list_has_eleven_strokes():
    assert sum(len(v) for v in fs.OWNER.values()) == 11


def test_local_rise_sees_a_stroke_and_not_a_ring():
    n = SR
    x = hit(n, 0.5, seed=1) + hit(n, 0.540, seed=2)
    env, hop = fp.attack_env(x, SR)
    assert fs.local_rise_db(env, 0.540, SR, hop) >= 9.0
    ring = hit(n, 0.5, tau=0.03, seed=3)
    env, hop = fp.attack_env(ring, SR)
    assert fs.local_rise_db(env, 0.540, SR, hop) < 3.0
    assert fs.local_rise_db(env, 0.0005, SR, hop) == 0.0     # before the file's start: 0, no crash


def test_local_rise_window_reaches_four_ms_after_the_onset():
    hop = 6                                    # 0.125 ms at 48 kHz
    env = np.ones(8000)
    env[int(0.5 / 0.000125) + 24:] = 10.0      # the loud body starts 3 ms after the walk's foot at 0.5 s
    r = fs.local_rise_db(env, 0.5, SR, hop)
    assert abs(r - 20.0) < 0.01


def test_verdict_bar():
    held = {"share": 0.10}
    kit = {"found": 12, "targets": 12, "new_false": 0}
    assert fs.verdict(held, kit, 11, 11)["pass"]
    assert not fs.verdict({"share": 0.11}, kit, 11, 11)["pass"]
    assert not fs.verdict(held, {**kit, "found": 11}, 11, 11)["pass"]
    assert not fs.verdict(held, {**kit, "new_false": 1}, 11, 11)["pass"]
    assert not fs.verdict(held, kit, 10, 11)["pass"]
    assert fs.verdict({"share": None}, kit, 11, 11)["pass"]


def test_kit_summary_counts_flams_and_new_false_only():
    rows = [{"file": "03_flams", "seconds": 12, "seconds_after": 3, "false_before": 0, "false_after": 0},
            {"file": "07_kick", "seconds": 0, "seconds_after": 0, "false_before": 2, "false_after": 2},
            {"file": "09_pairs", "seconds": 11, "seconds_after": 0, "false_before": 0, "false_after": 1},
            {"file": "08_noise", "seconds": 0, "seconds_after": 0, "false_before": 3, "false_after": 1}]
    assert fs.kit_summary(rows) == {"found": 3, "targets": 12, "new_false": 1}


def test_rows_for_track_flags_owner_reference_and_hat_cause():
    n = 2 * SR
    x = hit(n, 0.5, seed=1) + hit(n, 0.540, seed=2) + hit(n, 1.2, seed=3) + hit(n, 1.240, seed=4)
    hat = 0.2 * (hit(n, 0.540, seed=5, tau=0.004) + hit(n, 1.240, seed=6, tau=0.004))
    hat_quiet = np.zeros(n)
    p = fs.params()
    rows, gated = fs.rows_for_track(x, SR, [0.5, 1.2], {"hat 1": (hat_quiet, SR)}, p, refs_t=[0.540], owner=[1.24])
    assert gated == 0 and len(rows) == 2
    a, b = rows
    assert a["in_ref"] and not a["owner"] and a["cls"] == "reference"
    assert b["owner"] and not b["in_ref"] and b["cls"] == "owner"
    assert not a["hat_cause"] and not b["hat_cause"]


def test_a_hat_that_explains_the_re_rise_sets_hat_cause():
    n = 10 * SR
    x = np.zeros(n)
    hat = np.zeros(n)
    firsts = []
    for k in range(1, 9):
        x += hit(n, k + 0.0, seed=k) + hit(n, k + 0.040, seed=20 + k)
        hat += 5.0 * hit(n, k + 0.040, seed=40 + k, tau=0.004) + hit(n, k + 0.5, seed=60 + k, tau=0.004)
        x += 0.3 * hit(n, k + 0.5, seed=60 + k, tau=0.004)      # the hat's own strokes bleed into the snare mic
        firsts.append(float(k))
    srcs = {"hat 1": (hat, SR)}
    rows, gated = fs.rows_for_track(x, SR, firsts, srcs, fs.params(), want_flags=False)
    assert gated + len(rows) == 8
    quiet, _ = fs.rows_for_track(x, SR, firsts, {"hat 1": (np.zeros(n), SR)}, fs.params(), want_flags=False)
    assert len(quiet) == 8 and not any(r["hat_cause"] for r in quiet)
    # the rule's own gate (10 dB, all mics) passes a weaker hat that the 20 dB cause test still names
    weak = {"hat 1": (hat * 0.05 + 0.3 * x, SR)}
    rows2, _ = fs.rows_for_track(x, SR, firsts, weak, fs.params(margin=0.0), want_flags=False)
    assert rows2 and any(r["hat_cause"] for r in rows2)


def test_params_overrides_only_the_three_knobs():
    p = fs.params(rise=6, margin=4, min_gap_ms=8)
    assert (p["rise_db"], p["margin_db"], p["min_gap_s"]) == (6.0, 4.0, 0.008)
    assert p["hp_hz"] == fp.DEFAULTS["hp_hz"] and p["max_gap_s"] == fp.DEFAULTS["max_gap_s"]


def test_same_strokes_compares_samples_and_bleed_flags():
    sr = 48000
    py = [{"t": 1.0, "bleed": False}, {"t": 2.0, "bleed": True}]
    assert fs.same_strokes(py, [{"t": 2.0, "bleed": True}, {"t": 1.0 + 0.4 / sr, "bleed": False}], sr)["equal"]
    one_off = fs.same_strokes(py, [{"t": 1.0 + 1.0 / sr, "bleed": False}, {"t": 2.0, "bleed": True}], sr)
    assert not one_off["equal"] and len(one_off["diff"]) == 2
    flag = fs.same_strokes(py, [{"t": 1.0, "bleed": False}, {"t": 2.0, "bleed": False}], sr)
    assert not flag["equal"] and (flag["added_py"], flag["added_rust"]) == (1, 2)


def test_the_late_peak_gate_leaves_a_wire_mic_without_strokes_and_the_peaks_compare():
    n = 6 * SR
    hits = [0.5 + 0.5 * k for k in range(10)]
    wire = sum(0.2 * hit(n, t, tau=0.003, seed=k) + 0.8 * hit(n, t + 0.014, tau=0.006, seed=100 + k) for k, t in enumerate(hits))
    wire = wire + sum(hit(n, t + 0.030, tau=0.003, seed=200 + k) for k, t in enumerate(hits[:3]))
    p = fs.params()
    assert fs.py_found(wire, SR, hits, {}, p), "the rule alone finds strokes on the wire mic"
    assert fs.py_found(wire, SR, hits, {}, p, gate=True) == [], "the gate leaves none"
    top = sum(hit(n, t, tau=0.003, seed=k) for k, t in enumerate(hits)) + hit(n, 1.040, tau=0.003, seed=300)
    assert fs.py_found(top, SR, hits, {}, p, gate=True) == fs.py_found(top, SR, hits, {}, p)
    assert fs.py_late(top, SR, hits, p) < -3.0 <= fs.py_late(wire, SR, hits, p)
    assert fs.py_share(top, SR, hits, p) < 0.4 <= fs.py_share(wire, SR, hits, p)
    assert fs.same_late(None, None) and fs.same_late(-1.0, -1.0 + 1e-9)
    assert not fs.same_late(None, -1.0) and not fs.same_late(-1.0, -1.1)
