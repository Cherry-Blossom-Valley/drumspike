"""P4-19: the held-out tests of `tools/roll_fill.py` (a stroke on the bottom mic, a layer onset). Synthetic signals
only; no session file needed."""
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import roll_fill as rf  # noqa: E402

SR = 48000


def burst(n_s, hits, level=0.3, floor=1e-4, seed=2):
    rng = np.random.default_rng(seed)
    x = rng.normal(0, floor, int(n_s * SR))
    for t in hits:
        i = int(t * SR)
        n = int(0.02 * SR)
        x[i:i + n] += level * np.exp(-np.arange(n) / (0.004 * SR)) * rng.choice([-1, 1], n)
    return x


def test_a_stroke_on_the_bottom_mic_or_a_layer_onset_confirms_an_added_hit():
    wit = burst(4.0, [1.0, 2.0])                       # the bottom mic has strokes at 1.0 and 2.0 s
    layers = rf.layer_hits([0.5, 1.5], {"s samp": [0.505, 1.505, 3.005]})   # 5 ms latency: 3.005 is 3.0 on the track
    rows = rf.confirm([1.0, 2.0, 3.0, 3.5], wit, SR, layers)
    assert [r["witness"] for r in rows] == [True, True, False, False]
    assert [r["layer"] for r in rows] == [False, False, True, False]
    assert [r["confirmed"] for r in rows] == [True, True, True, False]
    assert rf.summary(rows) == {"added": 4, "witness": 2, "layer": 1, "confirmed": 3, "share": 0.75}
    # no witness mic: only the layers decide
    rows = rf.confirm([3.0], None, SR, layers)
    assert rows[0]["rise_wit"] is None and rows[0]["confirmed"]
    assert rf.summary([])["share"] is None


def test_the_layer_tolerance_is_4_ms():
    layers = rf.layer_hits([], {"l": [1.0045]})        # no normal hit: no latency
    assert not rf.confirm([1.0], None, SR, layers)[0]["layer"]
    layers = rf.layer_hits([], {"l": [1.0035]})
    assert rf.confirm([1.0], None, SR, layers)[0]["layer"]


def test_the_samples_option_moves_every_session_read():
    import fix_metric as fm
    old = fm.audio_path
    try:
        rf.use_samples("/tmp/x")
        assert fm.audio_path({"dir": "S"}, "s top") == Path("/tmp/x/S/Audio Files/s top.wav")
        assert rf.audio_of("S", "s top") is None
    finally:
        fm.audio_path = old
        rf.use_samples(rf.Path.home() / ".cache" / "drumspike" / "samples")
