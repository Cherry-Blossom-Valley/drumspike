"""P4-13 stage B: the app's flam rule (`crates/core/src/detect/flam.rs`, through `parity flam`) against the Python
reference (`tools/flam_pairs.py` `attack_env`, `transfer_gains`, `second_onsets`, as `tools/flam_split.py` runs them) on
the same signals: every second stroke equal (sample and bleed flag). Synthetic sessions cover the windows (12 to 50 ms,
the guard before the next hit), the level and floor tests, the walk's bound at the dip, the bleed gate with a source,
44.1 kHz (a block of 6 samples is not 0.125 ms there) and the file's ends. The held-out session Session E runs
when it is on this machine (`~/.cache/drumspike/sessions`, the stage A events in `~/.cache/drumspike/flam-split/`)."""
import sys
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

from core_tools import PARITY, require

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import flam_split as fs  # noqa: E402

P = fs.params()


def stroke(x, sr, t, db, tau=0.003, seed=0):
    """A stick stroke: white noise with an exponential decay `tau`, peak about `db` dBFS, added at `t`."""
    rng = np.random.default_rng(seed)
    a = int(round(t * sr))
    k = min(int(0.06 * sr), len(x) - a)
    if k > 0:
        x[a:a + k] += 10 ** (db / 20) * 0.35 * rng.standard_normal(k) * np.exp(-np.arange(k) / (tau * sr))


def noise(x, db, seed=99):
    x += 10 ** (db / 20) * np.random.default_rng(seed).standard_normal(len(x)) * 0.35


def compare(tmp_path, x, sr, hits, sources=()):
    """Python and Rust on one mic: the strokes (python, rust) and the comparison."""
    tool = require(PARITY)
    own = tmp_path / "own.wav"
    sf.write(own, x.astype(np.float32), sr, subtype="FLOAT")
    src, py_src = [], {}
    for i, (s, ssr) in enumerate(sources):
        q = tmp_path / f"src{i}.wav"
        sf.write(q, s.astype(np.float32), ssr, subtype="FLOAT")
        src.append((f"src{i}", q))
        py_src[f"src{i}"] = (s.astype(np.float32).astype(np.float64), ssr)
    xs = x.astype(np.float32).astype(np.float64)
    py = fs.py_found(xs, sr, hits, py_src, P)
    rs = fs.rust_found(tool, own, hits, src, tmp_path / "work", P)
    cmp = fs.same_strokes(py, rs["found"], sr)
    assert cmp["equal"], cmp
    late = fs.py_late(xs, sr, hits, P)
    assert fs.same_late(late, rs["late_peak_db"]), (late, rs["late_peak_db"])   # the app's gate values A and W
    assert fs.same_late(fs.py_share(xs, sr, hits, P), rs["late_share"]), rs["late_share"]
    return py, rs, cmp


def session(sr=48000, n=30, seed=1):
    """Single strokes every 0.5 s from 0.5 s at -6 dB on -80 dB noise; returns (x, hits)."""
    x = np.zeros(int((0.5 * n + 1.0) * sr))
    hits = [0.5 + 0.5 * i for i in range(n)]
    for i, t in enumerate(hits):
        stroke(x, sr, t, -6.0, seed=seed * 1000 + i)
    noise(x, -80.0, seed=seed)
    return x, hits


def test_flams_inside_and_outside_the_window_and_the_guard(tmp_path):
    x, hits = session()
    sr = 48000
    # second strokes 3 dB louder than the first at 10 (under the 12 ms minimum), 15, 25, 45 and 55 ms (past 50 ms)
    for k, (i, gap) in enumerate([(2, 0.010), (5, 0.015), (8, 0.025), (11, 0.045), (14, 0.055)]):
        stroke(x, sr, hits[i] + gap, -3.0, seed=500 + k)
    # a flam whose main stroke lies 5 ms before the next output hit: the guard ends the search before it
    stroke(x, sr, hits[17] + 0.025, -3.0, seed=600)
    guarded = sorted(hits + [hits[17] + 0.030])
    py, rs, cmp = compare(tmp_path, x, sr, guarded)
    found = sorted(round(f["t"] - f["first"], 3) for f in py)
    assert found == [0.015, 0.025, 0.045], found
    assert cmp["added_rust"] == 3 and len(rs["added"]) == 3


def test_the_level_floor_and_rise_tests(tmp_path):
    x, hits = session(seed=2)
    sr = 48000
    stroke(x, sr, hits[3] + 0.025, -40.0, seed=701)   # 34 dB under the first stroke: over the 30 dB level limit
    stroke(x, sr, hits[6] + 0.025, -30.0, seed=702)   # 24 dB under: found
    # a loud noise in the 50 ms before hit 9 sets a high floor: a quiet second stroke is under floor + 12 dB
    a, b = int((hits[9] - 0.048) * sr), int((hits[9] - 0.003) * sr)
    x[a:b] += 10 ** (-20 / 20) * 0.35 * np.random.default_rng(5).standard_normal(b - a)
    stroke(x, sr, hits[9] + 0.025, -24.0, seed=703)
    # a slow first stroke (20 ms decay): its ring is the base, a second stroke 8 dB over it is no re-rise
    x[int(hits[12] * sr):int(hits[12] * sr) + int(0.06 * sr)] *= 0
    stroke(x, sr, hits[12], -6.0, tau=0.02, seed=704)
    stroke(x, sr, hits[12] + 0.03, -6.0 - 8.7 + 8.0, tau=0.003, seed=705)
    py, _, _ = compare(tmp_path, x, sr, hits)
    firsts = sorted(f["first"] for f in py)
    assert hits[6] in firsts and hits[3] not in firsts and hits[9] not in firsts, firsts


def test_the_walk_stops_at_the_dip_after_the_first_stroke(tmp_path):
    # a 10 ms decay of the first stroke: its tail lies above the walk's threshold up to the second stroke
    x, hits = session(seed=3)
    sr = 48000
    for k, i in enumerate((4, 9, 14)):
        x[int(hits[i] * sr):int(hits[i] * sr) + int(0.06 * sr)] *= 0
        stroke(x, sr, hits[i], -6.0, tau=0.004, seed=800 + k)
        stroke(x, sr, hits[i] + 0.015, -2.0, seed=810 + k)
    py, rs, _ = compare(tmp_path, x, sr, hits)
    assert len(py) == 3
    for f in rs["found"]:
        assert 13.0 <= f["gap_ms"] <= 15.5, f


def test_the_bleed_gate_with_a_source(tmp_path):
    # the own mic hears the source 20 dB lower; the source strokes 25 ms after four own hits (bleed) and between the hits
    sr = 48000
    x, hits = session(seed=4, n=24)
    src = np.zeros_like(x)
    for i, t in enumerate(hits):
        stroke(src, sr, t + (0.025 if i % 6 == 0 else 0.25), -1.0, seed=900 + i)
    noise(src, -80.0, seed=7)
    x = x + 0.1 * src
    py, rs, cmp = compare(tmp_path, x, sr, hits, [(src, sr)])
    assert cmp["py"] == 4 and cmp["added_py"] == 0, cmp
    assert all(f["bleed"] for f in rs["found"])
    assert rs["gains"][0]["gain_db"] == pytest.approx(-20.0, abs=1.0)
    # a source at another rate is left out on both sides: the strokes are added
    _, rs2, cmp2 = compare(tmp_path, x, sr, hits, [(src[::2], 24000)])
    assert cmp2["added_rust"] == 4 and rs2["gains"] == []


def test_the_late_peak_of_a_wire_mic_and_a_top_mic_is_the_same_in_both(tmp_path):
    sr = 48000
    top, hits = session(seed=6, n=14)
    wire = np.zeros_like(top)
    for i, t in enumerate(hits):
        stroke(wire, sr, t, -14.0, seed=2000 + i)
        stroke(wire, sr, t + 0.014, -4.0, tau=0.006, seed=3000 + i)
    noise(wire, -80.0, seed=8)
    for x, gated in ((top, False), (wire, True)):
        _, rs, _ = compare(tmp_path, x, sr, hits)
        assert (rs["late_peak_db"] >= -3.0 and rs["late_share"] >= 0.4) == gated, (rs["late_peak_db"], rs["late_share"])


def test_44k1_and_the_ends_of_the_file(tmp_path):
    sr = 44100
    x, hits = session(sr=sr, seed=5, n=12)
    stroke(x, sr, hits[2] + 0.02, -3.0, seed=901)
    stroke(x, sr, hits[7] + 0.033, -3.0, seed=902)
    # a hit 2 ms before the end of the file and one at 1.5 ms after its start
    x[int(0.0015 * sr):int(0.0015 * sr) + 10] += 0.3
    hits = [0.0015] + hits + [len(x) / sr - 0.002]
    py, _, cmp = compare(tmp_path, x, sr, hits)
    assert cmp["py"] == 2


HELD = fs.held_cases()


@pytest.mark.skipif(not HELD, reason="the held-out session and the stage A events are not on this machine")
def test_the_held_out_session_session_e(tmp_path):
    name, path, hits, src = HELD[0]
    tool = require(PARITY)
    x, sr = fs.fp.read_mono(path)
    sources = {i: fs.fp.read_mono(q) for i, q in src}
    py = fs.py_found(x, sr, hits, sources, P)
    rs = fs.rust_found(tool, path, hits, src, tmp_path / "work", P)
    cmp = fs.same_strokes(py, rs["found"], sr)
    assert cmp["equal"] and cmp["py"] >= 5, (name, cmp)
    late = fs.py_late(x, sr, hits, P)
    assert fs.same_late(late, rs["late_peak_db"]) and late < -3.0, (late, rs["late_peak_db"])
    assert fs.same_late(fs.py_share(x, sr, hits, P), rs["late_share"]) and rs["late_share"] < 0.4
