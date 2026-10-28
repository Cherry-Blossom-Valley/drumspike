"""P4-18: the firm-fix diagnosis (`tools/firm_fixes.py`). Synthetic signals and activations only; no session file and
no HOME content needed (run with HOME pointed at an empty folder)."""
import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import firm_fixes as ff  # noqa: E402

SR = 48000


def burst(n_s, hits, level=0.5, floor=1e-4, seed=1):
    """Noise floor with a 20 ms decaying burst at each time in `hits`."""
    rng = np.random.default_rng(seed)
    x = rng.normal(0, floor, int(n_s * SR))
    for t in hits:
        i = int(t * SR)
        n = int(0.02 * SR)
        x[i:i + n] += level * np.exp(-np.arange(n) / (0.004 * SR)) * rng.choice([-1, 1], n)
    return x


def test_peak_and_rise_of_a_burst_against_noise():
    x = burst(1.0, [0.5])
    assert ff.peak_db(x, SR, 0.5) > -10 and ff.peak_db(x, SR, 0.2) < -60
    assert ff.rise_db(x, SR, 0.5) > 40
    assert abs(ff.rise_db(x, SR, 0.2)) < 12          # noise only: no stroke


def test_rise_is_small_on_a_decaying_tail():
    x = burst(1.0, [0.5])
    assert ff.rise_db(x, SR, 0.515) < 10             # 15 ms into the decay of the burst


def test_nearest_is_signed_and_empty_is_nine():
    assert ff.nearest([1.0, 2.0], 1.3) == pytest.approx(-0.3)
    assert ff.nearest([1.0, 2.0], 1.8) == pytest.approx(0.2)
    assert ff.nearest([], 1.0) == 9.0


def test_chains_find_a_regular_run_only():
    reg = [10 + 0.25 * i for i in range(10)]
    assert ff.chains(reg).all()
    assert not ff.chains(reg[:4]).any()                              # too short (5 needed)
    assert ff.chains(reg[:5]).all()
    jit = [10, 10.25, 10.60, 10.80, 11.30, 11.55]                    # gaps vary by far more than 12 %
    assert not ff.chains(jit).any()
    assert not ff.chains([10 + 0.04 * i for i in range(10)]).any()   # spacing under 80 ms
    two = reg + [20 + 0.25 * i for i in range(6)]
    m = ff.chains(two)
    assert m.all() and len(m) == 16


def test_chains_tolerance_is_relative_to_the_median_gap():
    t = [0, 0.25, 0.50, 0.75, 1.00, 1.27]                            # last gap 0.27 = 8 % over 0.25
    assert ff.chains(t, tol=0.12).all()
    assert not ff.chains(t, tol=0.05)[-1] and ff.chains(t, tol=0.05)[:5].all()


def test_split_hits_gives_missed_and_false():
    pairs, missed, false = ff.split_hits([1.0, 2.0, 3.0], [1.001, 2.5, 3.0])
    assert pairs == [(0, 0), (2, 2)] and missed == [1] and false == [1]


def test_miss_class_priority_and_boundaries():
    # no stroke on either mic: a reference error, even next to a neighbour
    assert ff.miss_class(-30, 3.0, 2.0, 0.02, True) == "no_transient"
    assert ff.miss_class(-30, 3.0, None, 0.5, False) == "no_transient"
    assert ff.miss_class(-30, 12.0, 2.0, 0.5, False) != "no_transient"       # one mic with a stroke is enough
    assert ff.miss_class(-30, 20.0, 20.0, 0.04, True) == "flam"              # flam before roll
    assert ff.miss_class(-30, 20.0, 20.0, -0.05, False) == "flam"            # 50 ms counts
    assert ff.miss_class(-30, 20.0, 20.0, 0.26, True) == "roll"
    assert ff.miss_class(-15, 20.0, 20.0, 0.5, False) == "ghost"             # 15 dB counts
    assert ff.miss_class(-14.9, 20.0, 20.0, 0.5, False) == "normal"


def test_false_class_priority_and_boundaries():
    fc = ff.false_class
    assert fc(-5, 30, 30, 0.012, 9, 9) == "other_role_bleed"
    assert fc(-5, 30, 30, 0.0121, 9, 9) == "real_stroke"
    assert fc(-5, 30, 30, 0.003, 0.002, 9) == "other_role_bleed"             # bleed before layer
    assert fc(-5, 30, 30, 9, 0.004, 9) == "layer_hit"
    assert fc(-5, 30, 30, 9, 0.0041, 9) == "real_stroke"
    assert fc(-5, 30, 30, 9, 9, 0.030) == "double"
    assert fc(-5, 30, 30, 9, 9, 0.012) == "double" and fc(-5, 30, 30, 9, 9, 0.0119) == "real_stroke"
    assert fc(-5, 30, 30, 9, 9, 0.061) == "real_stroke"
    assert fc(-25, 20, 20, 9, 9, 9) == "ghost_note"
    assert fc(-15, 20, 20, 9, 9, 9) == "ghost_note" and fc(-14.9, 20, 20, 9, 9, 9) == "real_stroke"
    assert fc(-25, 20, None, 9, 9, 9) == "ghost_note"                         # no witness: the own mic decides
    assert fc(-25, 20, 3, 9, 9, 9) == "one_mic"
    assert fc(-25, 20, 3, 9, 9, 9, rise_extra=20) == "hat_cymbal"
    assert fc(-25, 20, 3, 9, 9, 9, rise_extra=14.9) == "one_mic"
    assert fc(-25, 20, 20, 9, 9, 9, rise_extra=30) == "ghost_note"            # a stroke on both mics wins
    assert fc(-25, 4, 3, 9, 9, 9) == "weak"


def test_layer_offset_is_the_median_latency():
    ref = [1.0, 2.0, 3.0, 4.0]
    lay = [1.003, 2.0035, 3.0025, 9.0]
    assert ff.layer_offset(ref, lay) == pytest.approx(0.003)
    assert ff.layer_offset(ref, []) == 0.0
    assert ff.layer_offset(ref, [1.5, 2.5]) == 0.0                           # nothing within 20 ms


def test_count_by_keeps_zeros_and_order():
    assert ff.count_by(["a", "c", "a"], ["a", "b", "c"]) == {"a": 2, "b": 0, "c": 1}


def act_with(strong, weak_runs, n=2000):
    """A kept activation: snare column (1) peaks of 0.6 at `strong` frames, 0.12 at each frame of `weak_runs`."""
    a = np.zeros((n, 5), np.float32)
    for f in strong:
        a[f, 1] = 0.6
    for f in weak_runs:
        a[f, 1] = 0.12
    return a


def test_threshold_and_roll_fill_on_a_synthetic_activation():
    strong = [100, 400, 700]
    roll = [1000 + 25 * i for i in range(10)]            # ten weak peaks 0.25 s apart: a soft roll
    lone = [1500]                                        # one weak peak
    irregular = [1600, 1630, 1690, 1720]                 # four weak peaks, uneven
    act = act_with(strong, roll + lone + irregular)
    assert ff.picks_at(act, 0.24) == pytest.approx([f / 100 for f in strong])
    low = ff.picks_at(act, 0.10)
    assert len(low) == 3 + 10 + 1 + 4
    fill = ff.chain_picks(act)
    assert fill == pytest.approx(sorted([f / 100 for f in strong + roll]))
    assert ff.chain_picks(act_with(strong, roll[:7])) == pytest.approx([f / 100 for f in strong])   # 7 < 8: no fill


def test_roll_fill_ignores_a_weak_column_of_another_class():
    act = act_with([100], [])
    for i in range(10):
        act[1000 + 25 * i, 3] = 0.12                     # the hat column rises on a grid: not the snare's
    assert ff.chain_picks(act) == pytest.approx([1.0])


def test_gain_false_counts_matched_and_false_against_the_base():
    ref = [1.0, 2.0, 3.0, 4.0]
    base = [0.99, 1.99]
    var = [0.99, 1.99, 2.99, 8.0]                         # one more match (3.0), one more false (8.0)
    assert ff.gain_false(ref, base, var, 0.01) == (1, 1)
    assert ff.gain_false(ref, base, base, 0.01) == (0, 0)


def test_lead_of_is_the_median_event_minus_pick():
    picks = [1.0, 2.0, 3.0]
    assert ff.lead_of([1.012, 2.010, 3.008], picks) == pytest.approx(0.010)
    assert ff.lead_of([], picks) == 0.010


def test_level_cut_counts_false_removed_and_matched_lost():
    assert ff.level_cut([-5, -16, -30], [-20, -15, -10, -2], -15.0) == (2, 2)
    assert ff.level_cut([], [], -15.0) == (0, 0)
