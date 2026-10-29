"""Detector #8 package 3: the cause classifier of `tools/kick_miss_causes.py` on synthetic data.

One test per named cause, on tiny arrays; no audio files, no service tool. `t` is always an
UNMATCHED reference hit (the caller classifies the leftovers of the one-to-one matching only),
so an event within the window was claimed by the neighbouring reference.
"""

import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))
import kick_miss_causes as kmc  # noqa: E402

TOL = kmc.TOL


def C(t, gate="Level", kept=False):
    return {"t": t, "gate": gate, "kept": kept}


def test_a_hit_within_the_window_is_a_double_stroke():
    ev = np.asarray([1.000])
    cause, _ = kmc.cause_of(1.010, 1.000, ev, np.zeros(0), [])
    assert cause == "double-stroke (1 to 1)"


def test_an_event_just_outside_the_window_is_the_offset():
    ev = np.asarray([1.030])
    cause, d = kmc.cause_of(1.0, None, ev, np.zeros(0), [])
    assert cause == "offset" and d["event_off_ms"] == 30.0


def test_a_dropped_pick_with_a_kept_neighbour_is_the_min_gap():
    # no event within 20 ms of the ref, but the pick's kept neighbour is within 20 ms of the pick
    ev = np.asarray([1.044])
    picked = np.asarray([1.025])
    cause, d = kmc.cause_of(1.010, None, ev, picked, [])
    assert cause == "min-gap (20 ms)" and d["pick_ms"] == 15.0 and d["kept_ms"] == 34.0


def test_a_pick_that_vanished_is_refine():
    ev = np.asarray([1.060])
    picked = np.asarray([1.010])
    cause, d = kmc.cause_of(1.010, None, ev, picked, [])
    assert cause == "refine" and d["moved_to_ms"] == 50.0


def test_a_gated_candidate_names_the_failed_gate():
    cause, d = kmc.cause_of(1.000, None, np.zeros(0), np.zeros(0), [C(1.005, "Level")])
    assert cause == "gated: Level" and d["gates"] == ["Level"]


def test_a_kept_candidate_without_a_pick_is_reported():
    cause, _ = kmc.cause_of(1.000, None, np.zeros(0), np.zeros(0), [C(1.005, "Level", kept=True)])
    assert cause == "kept-not-picked"


def test_the_previous_refs_candidate_is_a_merged_peak():
    # the previous ref 30 ms earlier owns the candidate: 10 ms from it, 20.5 ms from here
    cause, d = kmc.cause_of(1.020, 0.990, np.zeros(0), np.zeros(0), [C(0.9995, "WeakFlux")])
    assert cause == "peak merged" and d["cand_ms"] == -20.5


def test_nothing_near_is_no_candidate():
    cause, d = kmc.cause_of(1.000, None, np.zeros(0), np.zeros(0), [C(1.100, "Level")])
    assert cause == "no candidate" and d["nearest_cand_ms"] == 100.0


def test_a_zero_fn_run_is_not_a_passed_attribution():
    # #98: a 0 FN run is a valid result, but the line must not read as a passed attribution.
    line = kmc.summary_line(0, 0)
    assert line == "no kick FN at Sensitivity 50: nothing to attribute"
    assert "attributed" not in line


def test_a_nonzero_fn_run_keeps_the_counted_attribution_line():
    assert kmc.summary_line(4, 4) == "total FN 4, attributed to named causes 4 (100.0 %)"
    assert kmc.summary_line(7, 5) == "total FN 7, attributed to named causes 5 (71.4 %)"
