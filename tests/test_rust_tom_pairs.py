"""P4-21: the tom pair rule of the app (`drumspike_core::detect::tom_bleed::tom_pairs`, through `parity tom-pairs`) against
the Python reference `tools/tom_pairs.py` `pair_marks` on random synthetic sessions, and the app's run on the seven songs
against the replay, event by event (skips without the cache `~/.cache/drumspike/fix-metric-p421b/run/`)."""
import json
import subprocess
import sys
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

from core_tools import PARITY, require

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import firm_fixes as ff  # noqa: E402
import tom_fixes as tf  # noqa: E402
import tom_pairs as tp  # noqa: E402

SR = 48000
RUN = Path.home() / ".cache" / "drumspike" / "fix-metric-p421b"


def session(seed, n_mics=3, n_hits=90):
    """Random mics: bursts of random amplitude at random times, some hits of different mics within 15 ms of each other."""
    rng = np.random.default_rng(seed)
    secs = 30
    x = [np.zeros(SR * secs, np.float32) for _ in range(n_mics)]
    times = [[] for _ in range(n_mics)]
    # a steady noise floor per mic, well under the quiet bursts: a burst a few dB over it has no
    # attack of its own (the rise stays under 10 dB), so the version 4 keep guard lets the pair
    # rule mark it - the bleed case
    for m in range(n_mics):
        x[m] += (10 ** (rng.uniform(-45, -35) / 20) * rng.standard_normal(SR * secs)).astype(np.float32)
    base = np.sort(rng.uniform(0.5, secs - 1, n_hits))
    for t0 in base:
        for m in range(n_mics):
            if rng.random() < 0.55:
                t = float(t0 + rng.uniform(-0.015, 0.015))
                a = int(t * SR)
                k = np.arange(SR // 10) / SR
                x[m][a:a + len(k)] += (10 ** (rng.uniform(-30, -6) / 20) * np.sin(2 * np.pi * 110 * k) * np.exp(-k / 0.05)).astype(np.float32)
                times[m].append(t)
    return x, [sorted(t) for t in times]


@pytest.mark.parametrize("seed", [1, 2, 3, 4])
def test_the_rust_rule_equals_the_reference(tmp_path, seed):
    x, times = session(seed)
    rng = np.random.default_rng(seed + 100)
    mics, ref = [], {}
    for i, (sig, t) in enumerate(zip(x, times)):
        p = tmp_path / f"m{i}.wav"
        sf.write(str(p), sig, SR, subtype="FLOAT")
        out = [bool(v) for v in (rng.random(len(t)) < 0.9)]
        tgt = [bool(v) for v in (rng.random(len(t)) < 0.9)]
        hp = tf.highpass(sig, SR)
        rise = [tf.stroke_rise(sig, hp, SR, v)[2] for v in t]
        mics.append({"id": f"m{i}", "drum": f"d{i}", "path": str(p), "times": t, "output": out, "target": tgt})
        ref[f"m{i}"] = {"t": t, "lvl": [ff.peak_db(sig, SR, v) for v in t], "rise": rise, "output": out,
                        "target": tgt}
    jf, of = tmp_path / "job.json", tmp_path / "out.json"
    jf.write_text(json.dumps({"mics": mics}))
    r = subprocess.run([str(require(PARITY)), "tom-pairs", str(jf), str(of)], capture_output=True, text=True)
    assert r.returncode == 0, r.stderr[-800:]
    rust = json.loads(of.read_text())
    py = tp.pair_marks(ref, drums={f"m{i}": f"d{i}" for i in range(len(x))})
    n_marked = 0
    for i, m in enumerate(ref):
        assert np.allclose(rust["levels"][i], ref[m]["lvl"], atol=1e-9, rtol=0), m
        assert np.allclose(rust["rises"][i], ref[m]["rise"], atol=1e-6, rtol=0), m
        assert rust["marks"][i] == py[m], m
        n_marked += sum(py[m])
    assert n_marked >= 5, "the session must have pairs to decide"
    # version 4: a marked hit with a 12 dB own-mic rise stays; the same hit without an attack stays marked
    guarded = {m: dict(ref[m], rise=[12.0] * len(ref[m]["t"])) for m in ref}
    kept = tp.pair_marks(guarded, drums={f"m{i}": f"d{i}" for i in range(len(x))})
    n_guarded = sum(kept[m][j] for m in ref for j in range(len(ref[m]["t"])) if py[m][j])
    assert n_guarded == 0, "a 12 dB rise keeps every marked hit"
    unguarded = {m: dict(ref[m]) for m in ref}
    for m in unguarded:
        del unguarded[m]["rise"]
    # without the rise the reference runs the version 3 rule: it marks a superset of the guarded run
    v3 = tp.pair_marks(unguarded, drums={f"m{i}": f"d{i}" for i in range(len(x))})
    assert all(v3[m][j] for m in ref for j in range(len(ref[m]["t"])) if py[m][j])


def test_the_window_and_margin_are_parameters(tmp_path):
    x, times = session(9)
    mics = []
    for i, (sig, t) in enumerate(zip(x, times)):
        p = tmp_path / f"m{i}.wav"
        sf.write(str(p), sig, SR, subtype="FLOAT")
        mics.append({"id": f"m{i}", "path": str(p), "times": t})
    counts = []
    for kw in ({}, {"window_ms": 4.0}, {"margin_db": 12.0}):
        jf, of = tmp_path / "job.json", tmp_path / "out.json"
        jf.write_text(json.dumps({"mics": mics, **kw}))
        r = subprocess.run([str(require(PARITY)), "tom-pairs", str(jf), str(of)], capture_output=True, text=True)
        assert r.returncode == 0, r.stderr[-800:]
        counts.append(sum(sum(m) for m in json.loads(of.read_text())["marks"]))
    assert counts[1] < counts[0] and counts[2] < counts[0], counts


ALL_RUNS = all((RUN / "run" / f"{tp.fm.track_name(s)}_default.json").is_file() for s in tp.tf.SONGS)


@pytest.mark.skipif(not ALL_RUNS, reason="no app run of the seven songs (tools/fix_metric.py run --out ~/.cache/drumspike/fix-metric-p421b)")
def test_the_app_run_equals_the_replay_on_the_seven_songs(tmp_path):
    rows = tp.do_equal(tmp_path, RUN, log=lambda *_: None)
    assert set(rows) == set(tp.tf.SONGS)
    for song, v in rows.items():
        assert v["total"] > 0 and v["same"] == v["total"], (song, v)
