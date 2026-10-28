"""P4-7b step 5: the helpers of tools/layer_eval.py on synthetic signals (the real sessions run with
`uv run python tools/layer_eval.py run`; results in phase4-report.md "P4-7b")."""
import json
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import layer_eval  # noqa: E402

SR = 48_000


def shot(seed=1, n=4_800, pre=240):
    rng = np.random.default_rng(seed)
    t = (np.arange(n) - pre) / SR
    x = 0.6 * np.sin(2 * np.pi * 160 * t) * np.exp(-t / 0.04) + 0.4 * rng.uniform(-1, 1, n) * np.exp(-t / 0.003)
    x[t < 0] = 0
    return x


def place(n, x, starts, gain=1.0):
    y = np.zeros(n)
    for s in starts:
        y[s:s + len(x)] += gain * x[: n - s]
    return y


def test_stem_and_the_session_files(tmp_path):
    assert layer_eval.stem("k in_02.wav") == "k in"
    assert layer_eval.stem("k samp 1_02.L.wav") == "k samp 1"
    a = tmp_path / "Audio Files"
    a.mkdir()
    for n in ["k in_02.wav", "k sampl sd3_01.wav", "k sampl trig_01.wav", "s top_02.wav", "s samp 1_02.L.wav",
              "s samp 1_02.R.wav", "s samp rev_02.L.wav", "k out_02.wav"]:
        (a / n).write_bytes(b"")
    f = layer_eval.session_files(tmp_path)
    assert [p.name for p in f["k in"][1]] == ["k sampl sd3_01.wav"]          # the trigger print is left out
    assert [p.name for p in f["s top"][1]] == ["s samp 1_02.L.wav"]          # .L of a split print, no reverb


def test_cut_one_shot_takes_the_loud_isolated_hit():
    x = shot()
    hits = [10_000, 12_000, 40_000, 80_000]                                  # 10,000 and 12,000 are not isolated
    y = place(120_000, x, [h - 240 for h in hits]) * np.array([1.0] * 120_000)
    y[40_000 - 240:40_000 - 240 + len(x)] *= 0.5                               # 40,000 is quieter than 80,000
    cut, used = layer_eval.cut_one_shot(y, SR, hits)
    assert used == 80_000
    assert len(cut) == int(0.31 * SR)
    # the cut starts 10 ms before the transient
    first = int(np.argmax(np.abs(cut) > 0.05 * np.max(np.abs(cut))))
    assert abs(first - 480) <= 5
    try:
        layer_eval.cut_one_shot(y, SR, [10_000, 12_000])
        raise AssertionError("no isolated hit must raise")
    except RuntimeError:
        pass


def test_find_template_finds_a_planted_lag_and_the_sign():
    x = shot(seed=2)
    tpl = x[240 - 48:240 - 48 + 480]
    y = place(48_000, x, [20_000 - 240 + 7]) * -1
    pos, h = layer_eval.find_template(y, 20_000, tpl, 48, 480)
    assert abs(pos - 20_007) < 0.2 and h < -0.97


def test_score_print_on_a_synthetic_session(tmp_path):
    """A print equal to the mic and a render placed by the alignment answer: offset 0, the same polarity,
    render match >= print match."""
    x = shot(seed=3)
    hits = [24_000, 60_000, 96_000]
    mic = place(150_000, x, [h - 240 for h in hits], 0.8)
    for name, sig in (("mic.wav", mic), ("print.wav", mic), ("layer.wav", mic)):
        sf.write(tmp_path / name, sig, SR, subtype="FLOAT")
    sf.write(tmp_path / "oneshot.wav", x, SR, subtype="FLOAT")
    info = {"layer": {"onset": 240},
            "align": {"sign": 1, "flips": [], "uncertain": 0,
                      "hits": [{"t": h / SR, "lag": 0.0, "match": 0.99, "sign": 1} for h in hits]}}
    r = layer_eval.score_print(tmp_path / "mic.wav", tmp_path / "print.wav", tmp_path / "layer.wav", info)
    assert r["hits_printed"] == 3 and abs(r["offset_median"]) < 0.2 and r["within_1"] == 1.0
    assert r["polarity_agrees"] and r["render_ge_print"] == 1.0
    json.dumps({k: v for k, v in r.items() if k != "rows"})                 # serialisable
