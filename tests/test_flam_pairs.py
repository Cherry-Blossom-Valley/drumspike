"""P4-13 stage 1: the second-stroke candidate (`tools/flam_pairs.py`). Synthetic signals only; no session file, no
HOME needed."""
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import flam_pairs as fp  # noqa: E402

SR = 48000


def hit(n, at, amp=1.0, tau=0.008, length=0.15, seed=0):
    """A noise burst with a sharp start at `at` seconds and an exponential decay (time constant `tau`)."""
    rng = np.random.default_rng(seed)
    y = np.zeros(n)
    a = int(at * SR)
    k = min(int(length * SR), n - a)
    y[a:a + k] = amp * rng.standard_normal(k) * np.exp(-np.arange(k) / (tau * SR))
    return y


def test_a_second_stroke_inside_the_lobe_is_found_at_its_start():
    n = SR
    x = hit(n, 0.5, seed=1) + hit(n, 0.520, seed=2)
    found = fp.second_onsets(x, SR, [0.5])
    assert len(found) == 1
    assert abs(found[0]["gap_ms"] - 20.0) <= 0.5
    assert not found[0]["bleed"]
    assert fp.added_events(found) == [{"t": found[0]["t"], "added": True}]


def test_a_single_decaying_hit_gives_no_second_stroke():
    n = SR
    for tau in (0.004, 0.008, 0.030, 0.100):
        x = hit(n, 0.5, tau=tau, seed=3)
        assert fp.second_onsets(x, SR, [0.5]) == []


def test_the_own_attack_complex_is_not_a_second_stroke():
    # a soft click, then the loud body 10 ms later: one stroke; the first-peak window (12 ms) covers the body
    n = SR
    x = hit(n, 0.5, amp=0.1, tau=0.001, seed=4) + hit(n, 0.510, amp=1.0, seed=5)
    p = {**fp.DEFAULTS, "min_gap_s": 0.003}
    assert fp.second_onsets(x, SR, [0.5], p) == []


def test_no_second_stroke_is_searched_into_the_next_hit():
    n = SR
    x = hit(n, 0.5, seed=6) + hit(n, 0.520, seed=7)
    assert len(fp.second_onsets(x, SR, [0.5, 0.80])) == 1
    # the next output hit 5 ms after the re-rise: it owns that stroke (the guard is 10 ms)
    assert fp.second_onsets(x, SR, [0.5, 0.525]) == []


def test_a_re_rise_that_another_mic_explains_is_bleed():
    n = 12 * SR
    b = hit(n, 1.025, seed=8)
    a = hit(n, 1.0, seed=9) + 0.5 * b           # the other drum reaches mic A at -6 dB
    for k in range(2, 12):                       # B's strong hits elsewhere calibrate G(B->A)
        hb = hit(n, float(k), seed=10 + k)
        b = b + hb
        a = a + 0.5 * hb
    env_a, hop = fp.attack_env(a, SR)
    env_b, _ = fp.attack_env(b, SR)
    gains = fp.transfer_gains(env_a, {"B": env_b}, [1.0], SR, hop)
    assert len(gains) == 1 and abs(gains[0][1] + 6.0) < 1.0
    found = fp.second_onsets(a, SR, [1.0], env=env_a, gains=gains)
    assert len(found) == 1 and found[0]["bleed"]
    assert fp.added_events(found) == []
    alone = fp.second_onsets(a, SR, [1.0], env=env_a)
    assert len(alone) == 1 and not alone[0]["bleed"]


def test_close_pairs_are_seconds_within_50_ms_of_the_previous_reference_hit():
    assert fp.close_pairs([0.0, 0.030, 0.200, 0.260, 0.300, 0.350]) == [1, 4, 5]
    assert fp.close_pairs([0.3, 0.0, 0.05]) == [1]        # sorted first; 50 ms is in


def test_kit_score_counts_each_label_one_to_one():
    ref, lab = [1.0, 1.020, 2.0], ["grace_20", "main_20", "single"]
    sc = fp.kit_score(ref, lab, [1.0, 2.001], [1.0, 1.021, 2.001, 3.0])
    assert sc["before"]["per"] == {"grace_20": (1, 1), "main_20": (0, 1), "single": (1, 1)}
    assert sc["after"]["per"]["main_20"] == (1, 1)
    assert (sc["before"]["false"], sc["after"]["false"]) == (0, 1)
    assert fp.kit_score([1.0], ["single"], [1.006], [1.006])["before"]["per"]["single"] == (0, 1)   # beyond 5 ms


def test_the_layer_check_shifts_the_layer_by_its_offset():
    layer = [1.000, 2.040]
    assert fp.confirmed([1.002, 2.042, 3.0], layer, 0.002) == [True, True, False]
    assert fp.confirmed([1.002], layer, -0.002) == [False]
    merged = fp.with_layers([1.0, 2.0], [1.003, 2.043, 2.0025], ref_off=0.001, layer_off=0.003)
    assert merged == [1.0, 2.0, 2.041]


def test_the_flam_mix_places_the_main_at_its_spacing():
    shots = [np.r_[1.0, np.zeros(999)], np.r_[-1.0, np.zeros(1499)]]
    mix, rows = fp.flam_mix(shots, SR, [5, 40], 2, 3, seed=1)
    assert sorted(r["label"] for r in rows).count("single") == 3
    for g in (5, 40):
        gr = sorted(r["t"] for r in rows if r["label"] == f"grace_{g}")
        mn = sorted(r["t"] for r in rows if r["label"] == f"main_{g}")
        assert len(gr) == len(mn) == 2
        for a, b in zip(gr, mn):
            assert round((b - a) * SR) == round(g / 1000 * SR)
    for r in rows:
        assert mix[round(r["t"] * SR)] != 0.0
        if r["label"].startswith("grace"):
            assert -12.01 <= 20 * np.log10(abs(mix[round(r["t"] * SR)])) <= -5.99


def test_the_verdict_needs_fewer_false_than_found_and_no_role_worse_by_1_percent():
    def role(n_ref, fb, fa, mb, ma, fix_b, fix_a):
        return {"n_ref": n_ref, "before": {"false": fb, "missed": mb, "fixes": fix_b},
                "after": {"false": fa, "missed": ma, "fixes": fix_a}}
    ok = {"snare": role(1000, 10, 14, 20, 15, 30, 29), "toms": role(1000, 5, 5, 5, 5, 10, 10)}
    assert fp.verdict(ok)["accepted"]
    tie = {"snare": role(1000, 10, 15, 20, 15, 30, 30)}
    assert not fp.verdict(tie)["accepted"]                       # added false 5 = found 5
    worse = {"snare": role(1000, 10, 11, 20, 18, 30, 41), "toms": role(100, 0, 0, 0, 0, 0, 2)}
    v = fp.verdict(worse)
    assert v["worse"] == ["snare", "toms"] and not v["accepted"]   # 10 > 1 % of 1000; 2 > 1 % of 100


def test_the_late_peak_separates_a_top_mic_from_a_wire_mic_and_gates_the_rule():
    n = 8 * SR
    hits = [0.5 + 0.5 * k for k in range(12)]
    top = sum(hit(n, t, tau=0.003, seed=k) for k, t in enumerate(hits))
    wire = sum(0.2 * hit(n, t, tau=0.003, seed=k) + 0.8 * hit(n, t + 0.014, tau=0.006, seed=100 + k)
               for k, t in enumerate(hits))
    ringy = sum(hit(n, t, tau=0.06, length=0.2, seed=k) for k, t in enumerate(hits))   # a slow decay: A over -3, W 0
    a, w = {}, {}
    for name, x in (("top", top), ("wire", wire), ("ringy", ringy)):
        env, hop = fp.attack_env(x, SR)
        a[name], w[name] = fp.late_peak_db(env, hits, SR, hop), fp.late_share(env, hits, SR, hop)
    assert a["top"] < -10 and not fp.late_peaking(a["top"], w["top"]), (a, w)
    assert a["wire"] > 3 and w["wire"] > 0.9 and fp.late_peaking(a["wire"], w["wire"]), (a, w)
    assert a["ringy"] >= fp.LATE_MAX_DB and w["ringy"] < 0.4 and not fp.late_peaking(a["ringy"], w["ringy"]), (a, w)
    # the edges: A at -3 dB and W at 0.4 gate; either under it does not; no value does not; windows without data count for nothing
    assert fp.late_peaking(-3.0, 0.4) and not fp.late_peaking(-3.01, 0.9) and not fp.late_peaking(0.0, 0.39)
    assert not fp.late_peaking(None, 1.0) and not fp.late_peaking(0.0, None)
    env, hop = fp.attack_env(top, SR)
    assert fp.late_peak_db(env, [len(env) * hop / SR - 0.01], SR, hop) is None
    assert fp.late_peak_db(np.zeros(8000), [0.1], SR, 6) is None and fp.late_share(np.zeros(8000), [0.1], SR, 6) is None
    assert fp.late_peak_db(env, [], SR, hop) is None
