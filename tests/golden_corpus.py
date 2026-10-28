"""The WAV files of tests/golden/*.json (P1-8) and the Rust outputs the golden files hold (P3-12).

The golden files keep the outputs of the retired Python server (engine.py) for every corpus
and test WAV; tests/make_golden.py wrote them. The WAVs are synthetic and deterministic
(tools/parity/make_corpus.py and the test helpers), so no audio is committed:
write_corpus() writes the same files again, and test_golden.py compares the outputs of the
Rust core (the release `parity` binary) with the files.
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

TESTS = Path(__file__).resolve().parent
ROOT = TESTS.parent
GOLDEN = TESTS / "golden"
PARITY_BIN = ROOT / "target" / "release" / "parity"
sys.path.insert(0, str(TESTS))
sys.path.insert(0, str(ROOT / "tools" / "parity"))

import helpers  # noqa: E402
import make_corpus  # noqa: E402

NOTE = 36


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def q5(values):
    """Peak values (float32 rounded to 5 decimals) as integers of 1e-5."""
    return [int(round(v * 1e5)) for v in values]


def write_corpus(out):
    """The parity corpus plus the WAVs the pytest fixtures write; returns [(name, t0s, channels)]."""
    argv = sys.argv
    sys.argv = ["make_corpus.py", str(out)]
    try:
        make_corpus.main()
    finally:
        sys.argv = argv
    files = [(f["name"], f["t0s"], f["channels"]) for f in json.loads((out / "manifest.json").read_text())["files"]]
    onsets = [0.25, 0.75, 1.25, 1.75, 2.25, 2.75]                     # conftest.kick_wav
    x, _ = helpers.synth_track(onsets, 3.5)
    helpers.write_wav(out / "test_kick_in.wav", x)
    files.append(("test_kick_in.wav", sorted(t + d for t in onsets for d in (-0.006, 0.0, 0.004)), 1))
    x, _ = helpers.synth_track([0.25, 0.75], 1.5, gains_db=[-3.0, -3.0])  # test_project.kick_file, stereo
    helpers.write_wav(out / "test_kick_stereo.wav", x, channels=2)
    files.append(("test_kick_stereo.wav", [0.244, 0.25, 0.7552, 1.2], 2))
    return files


def modes(channels):
    return ["ffmpeg", "average"] if channels > 1 else ["ffmpeg"]


def parity(*args):
    # The frozen references predate the A5 delta rule (like the flam/bleed/roll pins):
    # the parity binary must run the shipped detector, so the rule is pinned 0 here.
    env = dict(os.environ, DRUMSPIKE_DELTA_RULE="0")
    r = subprocess.run([str(PARITY_BIN), *map(str, args)], capture_output=True, text=True, env=env)
    if r.returncode != 0:
        raise RuntimeError(f"parity {' '.join(map(str, args))} failed: {r.stderr[-500:]}")
    d = json.loads(r.stdout)
    assert isinstance(d, dict), type(d)
    return d


def rust_outputs(path, t0s, mix_mode):
    """Every golden value of one WAV and mix mode from the Rust core, in the layout of make_golden.compute:
    the decode digest and peak pyramid, the refinement of the t0 list as analyze_track runs it (the
    neighbouring t0s clip the window), the velocities at the refined times and the MIDI file of the hits."""
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        io = parity(path, "--mix", mix_mode, "--pcm-out", td / "x.f32")
        x = np.fromfile(td / "x.f32", dtype=np.float32)
        levels = {bs: {"n": lvl["n"], "min_sha256": sha256(json.dumps(q5(lvl["min"])).encode()),
                       "max_sha256": sha256(json.dumps(q5(lvl["max"])).encode()), "rms": q5(lvl["rms"])}
                  for bs, lvl in io["peaks"].items()}
        hits = [{"t0": t, "neighbors": ([t0s[i - 1]] if i > 0 else []) + ([t0s[i + 1]] if i + 1 < len(t0s) else [])}
                for i, t in enumerate(t0s)]
        (td / "refine.json").write_text(json.dumps({"wav": str(path), "mix": mix_mode, "hits": hits}))
        ref = parity("refine", td / "refine.json")
        out_hits = [{"t0": h["t0"], "t": r["t"], "rise_db": r["rise"]} for h, r in zip(hits, ref["refined"])]
        vels = [int(v) for v in ref["velocities"]]
        (td / "midi.json").write_text(json.dumps({"events": [{"t": h["t"], "v": v} for h, v in zip(out_hits, vels)],
                                                  "note": NOTE, "ppq": 24000, "tempo": 120.0}))
        mid = parity("midi", td / "midi.json", td / "golden.mid")
        midi = {"sha256": sha256((td / "golden.mid").read_bytes()),
                "notes": [[n["tick"], n["note"], n["velocity"], n["duration"]] for n in mid["notes"]]}
    return {"sr": int(io["info"]["sample_rate"]), "n_samples": int(io["n_samples"]),
            "signal_sha256": sha256(x.tobytes()), "peaks": levels, "hits": out_hits, "velocities": vels, "midi": midi}
