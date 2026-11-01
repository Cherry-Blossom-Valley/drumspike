"""P4-31: the kick walk of the app (`refine::first_motion_peak`, `parity refine` with `peak_db`) against the Python port of
the version 3 back-track `tools/onset_guard.py` `walk` with the same peak margin: kick-like hits with a lead-in tone under
their peak (the margins 33 dB shipped and 29 dB, the kick walk) and with a noise floor, on one track. The walk starts at
the version 2 time (`rule` 2 of the same binary), as `tools/kick_onset.py` `replay_walk` does."""
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
import onset_guard as og  # noqa: E402

SR = 48_000


def kick_track(onsets, lead_amps, noise_db=-80.0, seed=7):
    """Kicks (125 to 55 Hz sweep, 0.5 ms attack, 0.12 s decay, peak about 0.35) at `onsets`; before each a 3 ms tone of 900 Hz at `lead_amp`."""
    rng = np.random.default_rng(seed)
    x = np.zeros(int((onsets[-1] + 1.0) * SR))
    for t0, amp in zip(onsets, lead_amps):
        i0 = int(round(t0 * SR))
        k = np.arange(int(0.4 * SR))
        t = k / SR
        cum = np.cumsum(55.0 + 70.0 * np.exp(-t / 0.03))
        env = np.minimum(t / 0.0005, 1.0) * np.exp(-t / 0.12)
        x[i0:i0 + len(k)] += 0.7 * np.sin(2 * np.pi * cum / SR) * env
        n = 144
        x[i0 - n:i0] += amp * np.sin(2 * np.pi * 900.0 * np.arange(n) / SR)
    x += 10 ** (noise_db / 20) * rng.uniform(-1, 1, len(x))
    return x.astype(np.float32)


def rust_refine(tmp_path, wav, hits, rule, peak_db=None):
    job = {"wav": str(wav), "mix": "average", "rule": rule, "hits": hits}
    if peak_db is not None:
        job["peak_db"] = peak_db
    jf = tmp_path / f"job{rule}_{peak_db}.json"
    jf.write_text(json.dumps(job))
    r = subprocess.run([str(require(PARITY)), "refine", str(jf)], capture_output=True, text=True)
    assert r.returncode == 0, r.stderr[-800:]
    return json.loads(r.stdout)["refined"]


def python_walk(x, t2, t0, prev, nxt, peak_db):
    """The Python port from the version 2 time `t2` with the midpoints to the coarse neighbours of the hit at `t0`."""
    keep = og.FM_PEAK_DB
    og.FM_PEAK_DB = peak_db
    try:
        pm = int((prev + t0) / 2 * SR) if prev is not None else None
        nm = int((t0 + nxt) / 2 * SR) if nxt is not None else None
        foot, how = og.walk(x.astype(np.float64), int(round(t2 * SR)), SR, pm, nm)
    finally:
        og.FM_PEAK_DB = keep
    return (foot / SR if how == "moved" else t2), how


@pytest.mark.parametrize("noise_db", [-80.0, -45.0])
def test_the_python_walk_and_the_rust_walk_agree_at_both_margins(tmp_path, noise_db):
    onsets = [0.5 + 0.8 * i for i in range(12)]   # far enough apart: the floor before a hit is silence or the noise
    amps = [0.0, 0.005, 0.008, 0.009, 0.01, 0.011, 0.012, 0.014, 0.02, 0.04, 0.009, 0.011]
    x = kick_track(onsets, amps, noise_db)
    wav = tmp_path / "kicks.wav"
    sf.write(str(wav), x, SR, subtype="FLOAT")
    # the coarse time: 6 ms before the onset (the DSP/model frame), the neighbours are the other coarse hits
    coarse = [t - 0.006 for t in onsets]
    hits = [{"t0": t, "neighbors": [c for c in (coarse[i - 1] if i else None, coarse[i + 1] if i + 1 < len(coarse) else None) if c is not None]}
            for i, t in enumerate(coarse)]
    v2 = rust_refine(tmp_path, wav, hits, 2)
    moved_later = 0
    for peak_db in (33.0, 29.0):
        v3 = rust_refine(tmp_path, wav, hits, 3, peak_db)
        for i, (a, b) in enumerate(zip(v2, v3)):
            prev = coarse[i - 1] if i else None
            nxt = coarse[i + 1] if i + 1 < len(coarse) else None
            want, how = python_walk(x, a["t"], coarse[i], prev, nxt, peak_db)
            assert abs(b["t"] - want) < 1e-9, (noise_db, peak_db, i, b, want, how)
    d33, d29 = (rust_refine(tmp_path, wav, hits, 3, p) for p in (33.0, 29.0))
    assert [r["t"] for r in rust_refine(tmp_path, wav, hits, 3)] == [r["t"] for r in d33], "no peak_db is the shipped margin"
    moved_later = sum(1 for a, b in zip(d33, d29) if b["t"] > a["t"] + 1e-9)
    assert moved_later >= 3, "the lower margin must move some hits later"
    assert all(b["t"] >= a["t"] - 1e-12 for a, b in zip(d33, d29)), "a lower margin never walks further back"
