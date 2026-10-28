"""P4-6b stage A: the helpers of `tools/learn_eval.py` on synthetic data (no audio files, no model)."""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))

import learn_eval as L  # noqa: E402

SR = 48_000


def clicks(times: list[float], amps: list[float] | None = None, dur: float = 3.0, f: float = 60.0) -> np.ndarray:
    """Decaying low sine bursts (a kick-like sample) at `times`."""
    x = np.zeros(int(dur * SR), np.float32)
    amps = amps or [0.5] * len(times)
    n = int(1.0 * SR)  # long enough that the cut is under the floor
    t = np.arange(n) / SR
    burst = (np.sin(2 * np.pi * f * t) * np.exp(-t / 0.06)).astype(np.float32)
    for at, a in zip(times, amps):
        i = int(at * SR)
        m = min(n, len(x) - i)
        x[i : i + m] += a * burst[:m]
    return x


def test_match_count_is_one_to_one_and_maximal():
    assert L.match_count([1.0, 2.0], [1.01, 2.03]) == 1
    assert L.match_count([1.0, 1.015], [1.012]) == 1, "one estimate matches one reference"
    # the greedy pass must not take the first reference's late partner from the second
    assert L.match_count([1.000, 1.030], [1.015, 1.045]) == 2
    assert L.match_count([], [1.0]) == 0


def test_prf_counts_the_work():
    r = L.prf(np.array([1.0, 2.0, 3.0]), np.array([1.0, 2.5]))
    assert (r["tp"], r["fp"], r["fn"], r["work"]) == (1, 1, 2, 3)
    assert abs(r["p"] - 0.5) < 1e-9 and abs(r["r"] - 1 / 3) < 1e-9
    assert abs(r["f"] - 0.4) < 1e-9
    assert L.prf(np.array([]), np.array([]))["f"] == 1.0, "nothing to find and nothing found"
    assert L.prf(np.array([1.0]), np.array([]))["f"] == 0.0


def test_offset_of_is_the_median_of_matched_differences():
    ref = np.array([1.0, 2.0, 3.0, 4.0])
    est = np.array([1.004, 2.004, 3.006, 4.2])  # the last one is out of the window
    off, n = L.offset_of(ref, est)
    assert n == 3 and abs(off - 0.004) < 1e-9


def test_reachable_marks_hits_without_a_candidate():
    ok = L.reachable(np.array([1.0, 2.0]), np.array([1.01]))
    assert ok.tolist() == [True, False]


def test_ref_onsets_find_each_sample_once_on_its_attack():
    times = [0.25, 0.9, 1.0, 1.6, 2.2]
    on = L.ref_onsets(clicks(times, [0.5, 0.5, 0.2, 0.05, 0.5]), SR)
    assert len(on) == len(times), on
    assert np.all(np.abs(on - np.array(times)) < 0.003), on


def test_split_toms_takes_the_mic_with_the_strongest_attack():
    t = [0.5, 1.5]
    # mic a: a loud hit at 0.5 and bleed at 1.5; mic b the other way round, with 12 dB more gain overall
    a = clicks([0.5, 1.5], [0.5, 0.05], f=100.0)
    b = clicks([0.5, 1.5], [0.4, 1.6], f=150.0) * 4.0
    out, dropped = L.split_toms(np.array(t), {"a": a, "b": b}, SR)
    assert out["a"].tolist() == [0.5]
    assert out["b"].tolist() == [1.5]
    assert dropped == []


def test_split_toms_skips_a_silent_mic_and_drops_an_unclaimed_hit():
    # P4-38a: at 0.5 mic a has the attack (b is gated), at 1.5 the other way; at 2.5 BOTH are gated
    t = [0.5, 1.5, 2.5]
    a = clicks([0.5, 1.5], [0.9, 0.0001], f=100.0)
    b = clicks([0.5, 1.5], [0.0001, 0.9], f=150.0)
    out, dropped = L.split_toms(np.array(t), {"a": a, "b": b}, SR)
    assert out["a"].tolist() == [0.5], (out, dropped)
    assert out["b"].tolist() == [1.5], (out, dropped)
    assert dropped == [2.5], (out, dropped)          # planted fault: the old code claims 2.5 (the lowest-loud mic wins)


def test_pick_examples_takes_the_first_matched_hits_and_the_strongest_free_candidates():
    ref = np.array([5.0, 11.0, 12.0, 13.0])
    cands = [[11.004, 2, 0.3, 50], [12.02, 2, 0.2, 60], [13.1, 2, 0.9, 10], [20.0, 2, 0.5, 30], [30.0, 2, 0.7, 20],
             [40.0, 2, 0.1, 90]]
    this, not_ = L.pick_examples(ref, cands, after=10.0, n_this=5, n_not=2)
    assert this == [11.004, 12.02], "13.0 has no candidate within 30 ms; 5.0 is before 10 s"
    assert not_ == [13.1, 30.0], "13.1 is 100 ms from the reference: free; strongest first"


def test_split_hits_and_grey_counts_follow_the_filter_rules():
    ref = np.array([1.0, 2.0])
    real, bleed = L.split_hits(np.array([1.005, 1.5, 2.03, 3.0]), ref)
    assert real == [1.005] and bleed == [1.5, 3.0], "2.03 is neither: 30 ms from a reference hit"
    hits = [1.005, 1.5, 3.0]
    scores = [[1.0, 0.6], [1.5, 0.2], [3.0, 0.45]]
    assert L.grey_counts(real, bleed, hits, scores, 0.5, [], []) == (0, 2)
    # the "not" example takes its hit at any threshold; a "this" example is never grey
    assert L.grey_counts(real, bleed, hits, scores, 0.0, [3.0], []) == (0, 1)
    assert L.grey_counts(real, bleed, hits, scores, 0.9, [], [1.0]) == (0, 2)
    # planted fault: a candidate out of reach gives no score, so its hit stays
    assert L.grey_counts(real, bleed, hits, [[1.0, 0.6]], 0.5, [], []) == (0, 0)


def test_rule_thresholds_hold_the_old_cap_and_the_suggestion():
    r = L.rule_thresholds([0.2, 0.4, 0.5, 0.6], auto=0.5, suggest=0.05, not_scores=[0.3])
    assert r["0.49 cap (before P4-6c)"] == 0.49
    assert r["P4-6c suggestion (the app)"] == 0.05 and r["median of the hits"] == 0.45
    assert 0.2 < r["2-means of the hits"] < 0.6


def test_missed_of_finds_the_reference_hits_the_output_lacks():
    ref = np.array([1.0, 2.0, 3.0, 4.0])
    est = np.array([1.005, 2.99])            # 1.0 and 3.0 matched, 2.0 and 4.0 missed
    assert L.missed_of(ref, est).tolist() == [2.0, 4.0]
    assert L.missed_of(ref, np.array([1.0, 2.0, 3.0, 4.0])).tolist() == []


def test_reachable_split_needs_a_candidate_within_tol():
    missed = np.array([10.0, 20.0, 30.0])
    reach, no = L.reachable_split(missed, np.array([10.01, 30.5]))
    assert reach.tolist() == [10.0]          # 10.01 within TOL; 30.5 is 0.5 away
    assert no.tolist() == [20.0, 30.0]


def test_added_by_frame_needs_v_above_sens_and_score_at_or_over_th():
    # candidates at 1.0 (v 40: solid at sens 100? no: 40 <= 100) and 2.0 (v 120: below the Sensitivity)
    cands = [[1.0, 2, 0.5, 40.0], [2.0, 2, 0.5, 120.0], [3.0, 2, 0.5, 130.0]]
    scores = {100: 0.9, 200: 0.8, 300: 0.2}  # frames at FPS 100
    assert L.added_by_frame(cands, scores, 100.0, 0.5) == [2.0]   # 1.0 already solid; 3.0 score too low
    assert L.added_by_frame(cands, scores, 100.0, 0.85) == []     # the stricter threshold adds nothing
    assert L.added_by_frame(cands, scores, 100.0, 0.15) == [2.0, 3.0]  # a loose one adds both missed


def test_unmatched_counts_recovered_added_and_lost():
    ref = np.array([1.0, 2.0, 3.0])
    hits0 = np.array([1.0, 2.0])             # 3.0 missed
    assert L.unmatched_counts(ref, hits0, [3.005]) == {"recovered": 1, "added_false": 0, "lost": 0}
    assert L.unmatched_counts(ref, hits0, [3.5]) == {"recovered": 0, "added_false": 1, "lost": 0}
    assert L.unmatched_counts(ref, hits0, []) == {"recovered": 0, "added_false": 0, "lost": 0}
