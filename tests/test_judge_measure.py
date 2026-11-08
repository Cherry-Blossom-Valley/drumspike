"""#385: tools/judge_measure.py counts the judge's answers on windows; a bump under the departure band gets no move.
#394: the judge measures the lead bump against the hit's peak (the detector's reference, the stroke span after the
hit), not the look window's peak: a bump of 2 % of the stroke's peak that reads as 8 % of the local peak gets
suggestion 0 or none."""
import shutil
import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import judge_measure  # noqa: E402

pytestmark = pytest.mark.skipif(shutil.which("node") is None, reason="node is not available")

BEFORE = 43   # the cut at 48 kHz: 0.9 ms before the hit (floor)
AFTER = 29    # 0.6 ms after (ceil); the look window is BEFORE + AFTER samples around the hit
SPAN_AFTER = 240   # the detector's 5 ms stroke span after the hit


def window(bump):
    line = [0.022] * 16
    ramp = [0.022 + bump * k for k in (0.2, 0.3, 0.6, 1, 1, 0.8, 0.5)]
    return line + ramp + [0.039, 0.107, 0.233, 0.305, 0.6, 1.0], 23


def test_gold_distance_and_session_k_moves_are_counted():
    src = (ROOT / "static" / "agent.js").read_text()
    wins = []
    for n, bump in enumerate((0.016, 0.02, 0.0245)):
        s, at = window(bump)
        wins.append({"set": "goldE", "id": n, "role": "kick", "label_off": 0, "samples": s, "stroke": s, "hit_at": at})
        wins.append({"set": "K", "id": f"t:{n}", "role": "kick", "samples": s, "stroke": s, "hit_at": at})
    res = judge_measure.measure(src, wins)
    assert res["gold"]["goldE"] == {"n": 3, "exact": 3, "within1": 3, "moves": 0, "dist": 0}
    assert res["k"]["kick"]["moves"] == 0 and res["k"]["kick"]["keys_with_two_answers"] == 0


def gate_window():
    """A bump of 2 % of the stroke's peak that reads as 8 % of the local peak.

    Stroke peak 1.0 (over the detector's 5 ms span, reached past the look window); the look window's local peak is the
    stroke's early rise, 0.25. Line 0.01, a lead bump of 0.02 (8 samples ending one before the hit at index 43), then
    the stroke: 0.05 one sample after the hit, the rise to 0.25 at the window's end. In local-peak shares the bump reads
    0.08 off the line, over the 0.03 band; in stroke-peak shares it reads 0.01, under it, and the departure is the
    stroke's own first motion at +1: suggestion 0.
    """
    sig = [0.01] * 35 + [0.02] * 8 + [0.01]
    sig += [0.05] + [0.25] * (AFTER - 2)                                     # the look window: 28 samples after the hit
    sig += [min(1.0, 0.25 + 0.05 * k) for k in range(1, 100)] + [1.0] * (SPAN_AFTER - 28 - 99)
    assert len(sig) == BEFORE + 1 + SPAN_AFTER   # the hit plus 43 samples before and 240 after it
    return {"samples": sig[:BEFORE + AFTER], "stroke": sig, "hit_at": BEFORE}


def test_bump_of_2_percent_of_stroke_peak_gets_suggestion_0_or_none():
    src = (ROOT / "static" / "agent.js").read_text()
    w = gate_window()
    assert max(abs(v) for v in w["samples"]) == 0.25         # the look window's local peak
    assert max(abs(v) for v in w["stroke"][w["hit_at"]:]) == 1.0   # the hit's peak over the detector's span
    res = judge_measure.measure(src, [{"set": "K", "id": "gate", "role": "kick", **w}])
    row = [r for r in res["rows"] if r["id"] == "gate"][0]
    assert row["suggest"] in (0, None)


def test_stroke_span_cut_matches_the_detector():
    data = np.zeros(1000)
    data[520 + 200] = 1.0
    s, at = judge_measure.cut(data, 520, 48000)
    assert (at, len(s)) == (520 - 476, 549 - 476)   # 0.9 ms before (floor), 0.6 ms after (ceil)
    st = judge_measure.stroke_span(data, 520, 48000)
    assert (len(st), st.index(1.0)) == (760 - 476, at + 200)   # the 5 ms span after the hit
