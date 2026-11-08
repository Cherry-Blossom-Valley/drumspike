"""#385: tools/judge_measure.py counts the judge's answers on windows; a bump under the departure band gets no move."""
import shutil
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import judge_measure  # noqa: E402

pytestmark = pytest.mark.skipif(shutil.which("node") is None, reason="node is not available")


def window(bump):
    line = [0.022] * 16
    ramp = [0.022 + bump * k for k in (0.2, 0.3, 0.6, 1, 1, 0.8, 0.5)]
    return line + ramp + [0.039, 0.107, 0.233, 0.305, 0.6, 1.0], 23


def test_gold_distance_and_session_k_moves_are_counted():
    src = (ROOT / "static" / "agent.js").read_text()
    wins = []
    for n, bump in enumerate((0.016, 0.02, 0.0245)):
        s, at = window(bump)
        wins.append({"set": "goldE", "id": n, "role": "kick", "label_off": 0, "samples": s, "hit_at": at})
        wins.append({"set": "K", "id": f"t:{n}", "role": "kick", "samples": s, "hit_at": at})
    res = judge_measure.measure(src, wins)
    assert res["gold"]["goldE"] == {"n": 3, "exact": 3, "within1": 3, "moves": 0, "dist": 0}
    assert res["k"]["kick"]["moves"] == 0 and res["k"]["kick"]["keys_with_two_answers"] == 0
