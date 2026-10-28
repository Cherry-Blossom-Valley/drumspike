"""P4-21: the tom pair rule replay (`tools/tom_pairs.py`). Synthetic signals and events only; no session file and no HOME
content needed (run with HOME pointed at an empty folder)."""
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import tom_pairs as tp  # noqa: E402

SR = 48000


def mic(hits, output=None, target=None):
    """A pair input from `(time, level dB)` hits."""
    n = len(hits)
    return {"t": [h[0] for h in hits], "lvl": [h[1] for h in hits], "output": output or [True] * n,
            "target": target or [True] * n}


def two(a_hit, b_hit):
    """Two mics with two quiet-median hits each and one hit under test (the last of each)."""
    return {"a": mic([(1.0, 0.0), (2.0, 0.0), a_hit]), "b": mic([(1.5, 0.0), (2.5, 0.0), b_hit])}


def marks_of(mics, **kw):
    return tp.pair_marks(mics, **kw)


def test_window_edge_is_12_ms():
    assert marks_of(two((3.0, 0.0), (3.012, -10.0)))["b"][2]
    assert marks_of(two((3.0, 0.0), (2.988, -10.0)))["b"][2]
    assert not marks_of(two((3.0, 0.0), (3.0122, -10.0)))["b"][2]
    assert not marks_of(two((3.0, 0.0), (3.0125, -10.0)))["b"][2]


def test_margin_edge_is_6_db():
    assert marks_of(two((3.0, 0.0), (3.001, -6.0)))["b"][2]
    assert not marks_of(two((3.0, 0.0), (3.001, -5.99)))["b"][2]
    m = marks_of(two((3.0, -6.0), (3.001, 0.0)))
    assert m["a"][2] and not m["b"][2]
    m = marks_of(two((3.0, 0.0), (3.001, 0.0)))
    assert not m["a"][2] and not m["b"][2]


def test_level_is_against_the_median_of_the_mics_output_hits():
    # b is 20 dB louder in all its hits: two unison strokes at their typical level stay
    m = marks_of({"a": mic([(1.0, -30.0), (2.0, -30.0), (3.0, -30.0)]), "b": mic([(1.0, -10.0), (2.0, -10.0), (3.0, -10.0)])})
    assert not any(any(v) for v in m.values())
    # even count: the mean of the middle two (0 and -8 give -4): a's hits are +4 and -4
    med = tp.median_levels({"a": mic([(1.0, 0.0), (2.0, -8.0)]), "b": mic([(1.0, -10.0)], output=[False])})
    assert med == {"a": -4.0}
    m = marks_of({"a": mic([(1.0, 0.0), (2.0, -8.0)]), "b": mic([(1.0, -10.0), (5.0, 0.0)])})
    assert m["b"][0] and not m["a"][0]


def test_three_toms_and_a_marked_hit_protects_no_other():
    a = [(1.0, 0.0), (2.0, 0.0), (3.0, 0.0)]
    b = [(1.002, -9.0), (2.5, 0.0), (3.5, 0.0)]
    c = [(1.010, -14.0), (2.7, 0.0), (3.7, 0.0)]
    m = marks_of({"a": mic(a), "b": mic(b), "c": mic(c)})
    assert (m["a"][0], m["b"][0], m["c"][0]) == (False, True, True)
    # c at 1.020: 20 ms from a, 10 ms from the marked b: it stays
    m = marks_of({"a": mic(a), "b": mic([(1.010, -9.0), (2.5, 0.0), (3.5, 0.0)]), "c": mic([(1.020, -16.0), (2.7, 0.0), (3.7, 0.0)])})
    assert (m["b"][0], m["c"][0]) == (True, False)
    # b at -5 dB is kept against a and wins against c at -11 dB
    m = marks_of({"a": mic(a), "b": mic([(1.002, -5.0), (2.5, 0.0), (3.5, 0.0)]), "c": mic([(1.007, -11.0), (2.7, 0.0), (3.7, 0.0)])})
    assert (m["a"][0], m["b"][0], m["c"][0]) == (False, False, True)


def test_same_mic_and_same_drum_pairs_are_ignored():
    m = marks_of({"a": mic([(1.0, 0.0), (1.005, -20.0), (2.0, 0.0)])})
    assert not any(m["a"])
    two_mics = {"top": mic([(1.0, 0.0), (2.0, 0.0), (3.0, 0.0)]), "bot": mic([(1.001, -20.0), (2.0, 0.0), (3.0, 0.0)])}
    assert not any(any(v) for v in marks_of(two_mics, drums={"top": "tom1", "bot": "tom1"}).values())
    assert marks_of(two_mics, drums={"top": "tom1", "bot": "tom2"})["bot"][0]


def test_output_and_target_flags():
    top = [(1.0, 0.0), (2.0, 0.0), (3.0, 0.0)]
    low = [(1.001, -20.0), (2.5, 0.0), (3.5, 0.0)]
    assert not marks_of({"a": mic(top, output=[False, True, True]), "b": mic(low)})["b"][0]      # the winner is not in the output
    assert not marks_of({"a": mic(top), "b": mic(low, target=[False, True, True])})["b"][0]      # an edited hit stays
    assert marks_of({"a": mic(top, target=[False, True, True]), "b": mic(low)})["b"][0]          # but it still wins
    assert marks_of({"a": mic(top), "b": mic(low, output=[False] * 3)})["b"] == [False] * 3       # no output, no median


def test_partner_of_names_the_winner():
    mics = {"a": mic([(1.0, 0.0), (2.0, 0.0), (3.0, 0.0)]), "b": mic([(1.003, -20.0), (2.5, 0.0), (3.5, 0.0)])}
    m = marks_of(mics)
    p = tp.partner_of(mics, m, "b", 0)
    assert p[0] == "a" and p[1] == 0 and abs(p[2] - 20.0) < 1e-9 and abs(p[3] + 3.0) < 1e-6
    assert tp.partner_of(mics, m, "a", 0) is None


def test_kit_loses_no_real_hit_and_removes_the_bleed():
    mics, truth = tp.kit_session()
    marks = tp.pair_marks(mics)
    assert not any(mk and r for m in mics for mk, r in zip(marks[m], truth[m]))
    assert sum(mk for m in mics for mk in marks[m]) == sum(1 for m in truth for r in truth[m] if not r) == 40


def tone(n, at, amp, f=120.0):
    x = np.zeros(n)
    k = np.arange(min(n - at, SR // 5)) / SR
    x[at:at + len(k)] = amp * np.sin(2 * np.pi * f * k) * np.exp(-k / 0.08)
    return x


def test_witness_separates_bleed_from_a_real_stroke():
    # tom a struck at 1 s and at 3 s; mic b hears the first at -20 dB (bleed) and is struck at 3.002 s too (a real unison)
    n = 5 * SR
    a = tone(n, SR, 0.5) + tone(n, 3 * SR, 0.5)
    b = tone(n, SR + 48, 0.05) + tone(n, 3 * SR + 96, 0.3)
    sig = {"a": a.astype(np.float32), "b": b.astype(np.float32)}
    mics = {"a": {"t": [1.0, 3.0], "lvl": [-6.0, -6.0], "output": [True] * 2, "target": [True] * 2},
            "b": {"t": [1.001, 3.002], "lvl": [-26.0, -10.5], "output": [True] * 2, "target": [True] * 2}}
    marks = {"a": [False, False], "b": [True, False]}
    rows = tp.witness_rows("x", {}, {}, marks, mics, sig, SR, [])
    assert len(rows) == 1 and rows[0]["mic"] == "b" and rows[0]["t"] == 1.001
    assert rows[0]["verdict"] == "bleed" and rows[0]["louder_stroke"]
    # the marked hit of b at 3.002 s is a real stroke on b: the own mic shows it and a is not louder on a relative level
    marks = {"a": [False, False], "b": [False, True]}
    mics["a"]["lvl"] = [-6.0, -9.0]            # a's second stroke is soft against its own median
    rows = tp.witness_rows("x", {}, {}, marks, mics, sig, SR, [])
    assert rows[0]["rise_own"] > 10 and rows[0]["verdict"] == "real" and not rows[0]["louder_stroke"]
    # a marked hit with no stroke on its own mic is `none`
    sig["b"] = np.zeros(n, np.float32)
    assert tp.witness_rows("x", {}, {}, marks, mics, sig, SR, [])[0]["verdict"] == "none"


def floor_case(rel_b):
    """b's hit at 1.001 s with the relative level `rel_b` (its other hits at 0 dB) against a +30 dB hit of a."""
    m = marks_of({"a": mic([(1.0, 30.0), (2.0, 0.0), (3.0, 0.0)]), "b": mic([(1.001, rel_b), (5.0, 0.0), (6.0, 0.0)])})
    return m["b"][0]


def test_floor_edge_is_3_db_under_the_median():
    assert tp.FLOOR_DB == -3.0
    assert floor_case(-3.1) and floor_case(-3.0)
    assert not floor_case(-2.9) and not floor_case(0.0) and not floor_case(5.0)
    # a loud unison stays, and a hit that stays still wins against a third mic's hit
    m = marks_of({"a": mic([(1.0, 7.0), (2.0, 0.0), (3.0, 0.0)]), "b": mic([(1.001, 0.0), (5.0, 0.0), (6.0, 0.0)])})
    assert not any(any(v) for v in m.values())
    m = marks_of({"a": mic([(1.0, 12.0), (2.0, 0.0), (3.0, 0.0)]), "b": mic([(1.001, -1.0), (5.0, 0.0), (6.0, 0.0)]),
                  "c": mic([(1.002, -8.0), (7.0, 0.0), (8.0, 0.0)])})
    assert (m["a"][0], m["b"][0], m["c"][0]) == (False, False, True)
