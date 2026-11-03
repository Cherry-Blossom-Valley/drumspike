"""The WAV files of tests/golden/*.json (P1-8), the Rust outputs the golden files hold (P3-12) and the
cases of the frozen refine and MIDI references (refactor 1.5: from tools/parity/check_refine.py and
check_midi.py; tests/test_golden.py runs them).

The golden files keep the outputs of the retired Python server (engine.py) for every corpus
and test WAV; tests/make_golden.py wrote them. The WAVs are synthetic and deterministic
(tools/parity/make_corpus.py and the test helpers), so no audio is committed:
write_corpus() writes the same files again, and test_golden.py compares the outputs of the
Rust core (the release `parity` binary) with the files.
"""
import hashlib
import json
import os
import struct
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


def manifest(out):
    """The entries of the corpus manifest.json that write_corpus wrote (name, channels, t0s, time_reference ...)."""
    entries = json.loads((out / "manifest.json").read_text())["files"]
    assert isinstance(entries, list) and entries, type(entries)
    return entries


def run_parity(*args):
    """The parity binary as the moved check scripts ran it (no environment pin); its JSON answer."""
    r = subprocess.run([str(PARITY_BIN), *map(str, args)], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"parity {' '.join(map(str, args[:1]))} failed: {r.stderr[-500:]}")
    d = json.loads(r.stdout)
    assert isinstance(d, dict), type(d)
    return d


def bext_time_reference(path):
    """TimeReference of the bext chunk, or None when the file has none."""
    b = Path(path).read_bytes()
    pos = 12
    while pos + 8 <= len(b):
        cid, size = b[pos:pos + 4], struct.unpack("<I", b[pos + 4:pos + 8])[0]
        if cid == b"bext":
            return struct.unpack("<Q", b[pos + 8 + 338:pos + 8 + 346])[0]
        pos += 8 + size + (size % 2)
    return None


# The refinement cases (tools/parity/check_refine.py until refactor 1.5). Each case is
# (name, wav, mode, hits[, window]) with hits = [(t0, neighbours)]; the name is the key in
# tests/golden/frozen/refine.json.

MIN_GAP = 0.020
WINDOWS = ((-7.0, 7.0), (-3.0, 13.0), (-12.0, 2.0))


def neighbours_like_analyze_track(coarse):
    return [([coarse[i - 1]] if i > 0 else []) + ([coarse[i + 1]] if i + 1 < len(coarse) else [])
            for i in range(len(coarse))]


def dedup(times, mingap=MIN_GAP):
    out, last = [], -9e9
    for t in sorted(times):
        if t - last >= mingap:
            out.append(t)
            last = t
    return out


def refine_corpus_cases(corpus):
    """Every corpus file (ffmpeg mode; average mode as well with more than one channel), two hit lists each:
    "dedup" = the t0s after the 20 ms min-gap dedup of analyze_track, each hit with its previous and next
    neighbour, and "isolated" = every t0 on its own (silence, file edges, hits 30 ms off, t0 past the end)."""
    for entry in manifest(corpus):
        for mode in modes(entry["channels"]):
            t0s = [float(t) for t in entry["t0s"]]
            coarse = dedup(t0s)
            yield (f"corpus {entry['name']} [{mode}] dedup", corpus / entry["name"], mode,
                   list(zip(coarse, neighbours_like_analyze_track(coarse))))
            yield (f"corpus {entry['name']} [{mode}] isolated", corpus / entry["name"], mode,
                   [(t, []) for t in t0s])


def refine_edge_cases(scratch):
    """Branches the corpus and the bench do not reach: neighbours closer than 14 ms (the
    midpoint clips the search window on either side), a file shorter than 8 samples
    (the short-cut return) and coarse hits before the start and past the end."""
    sr = helpers.SR
    write_wav = make_corpus.write_wav
    # flams and rolls: pairs and triples 4 to 13 ms apart, every hit with both neighbours
    onsets = [0.5, 0.504, 0.7, 0.708, 0.9, 0.910, 1.1, 1.113, 1.3, 1.306, 1.312, 1.6, 1.609, 1.618, 1.627]
    gains = [-3, -6, -3, -3, -6, -3, -3, -9, -3, -3, -3, -3, -12, -3, -20]
    attacks = [0.5, 0.5, 0.3, 1.0, 0.5, 0.5, 2.0, 0.5, 0.5, 0.5, 0.5, 0.3, 0.3, 0.3, 0.3]
    x, _ = helpers.synth_track(onsets, 2.0, sr, gains_db=gains, attacks_ms=attacks)
    x = x.astype(np.float32)
    wav = scratch / "edge_flams.wav"
    write_wav(wav, x, sr, "f32")
    for off_name, offs in (("exact", [0.0] * len(onsets)), ("offsets", [0.002, -0.003, 0.001, 0.004, -0.001, 0.002,
                                                                        0.0, 0.003, -0.002, 0.001, 0.002, 0.0,
                                                                        0.0025, -0.0015, 0.001])):
        coarse = [t + o for t, o in zip(onsets, offs)]
        yield (f"edge flams {off_name}", wav, "ffmpeg", list(zip(coarse, neighbours_like_analyze_track(coarse))))
    # one-sided clipping: only a previous or only a next neighbour, 6 ms away
    one_sided = [(0.5, [0.494]), (0.5, [0.506]), (0.7, [0.694]), (0.708, [0.714]), (0.9, [0.9 - 0.0049]),
                 (0.9, [0.9 + 0.0051]), (1.1, [1.0999]), (1.1, [1.1001]), (1.3, [1.3, 1.306])]
    yield "edge one-sided neighbours", wav, "ffmpeg", one_sided
    # 5-sample file: every cut is shorter than 8 samples
    tiny = np.array([0.0, 0.5, -0.5, 0.25, 0.0], dtype=np.float32)
    wav5 = scratch / "edge_5_samples.wav"
    write_wav(wav5, tiny, sr, "f32")
    yield "edge 5-sample file", wav5, "ffmpeg", [(0.0, []), (0.00002, []), (-0.001, []), (0.01, [])]
    # 12-sample file at 48 kHz: the cut is 12 samples, the 9-point smoothing nearly as long
    twelve = np.array([0.0] * 3 + [0.9, -0.8, 0.7, -0.6, 0.5, -0.4, 0.3, -0.2, 0.1], dtype=np.float32)
    wav12 = scratch / "edge_12_samples.wav"
    write_wav(wav12, twelve, sr, "f32")
    yield "edge 12-sample file", wav12, "ffmpeg", [(0.0, []), (0.00006, []), (0.0001, [0.00005]), (0.0002, [])]
    # 8-sample file at 48 kHz: the shortest cut that is refined; the smoothing kernel (9) is longer
    eight = np.array([0.0, 0.0, 0.6, -0.5, 0.4, -0.3, 0.2, -0.1], dtype=np.float32)
    wav8 = scratch / "edge_8_samples.wav"
    write_wav(wav8, eight, sr, "f32")
    yield "edge 8-sample file", wav8, "ffmpeg", [(0.0, []), (0.00004, []), (0.0001, [])]
    # coarse hits before the start and past the end of a normal file
    yield "edge outside the file", wav, "ffmpeg", [(-0.5, []), (-0.02, []), (1.99, []), (2.0, []), (2.5, [])]


def refine_window_cases(corpus, scratch, n_corpus=3):
    """The search window as a parameter (P2-9): the flams of the edge cases and the dedup hits of the
    first corpus files with the windows -7..+7, -3..+13 and -12..+2 ms; every other case runs the
    default -2..+12 ms."""
    flams = [c for c in refine_edge_cases(scratch) if c[0].startswith("edge flams")]
    dedups = [c for c in refine_corpus_cases(corpus) if c[0].endswith("dedup")][:n_corpus]
    for lo, hi in WINDOWS:
        for name, wav, mode, hits in flams + dedups:
            yield (f"window {lo:+g}..{hi:+g} {name}", wav, mode, hits, (lo, hi))


def refine_bench_cases(scratch):
    """The 16 scenarios of the old refinement benchmark (kick and snare x clean, doubles, ghosts,
    bleed x +-5 ms and +-2 ms detector error), 150 hits each with the bench's neighbours. One seeded
    generator draws every random number, so the loop order is the benchmark's and the generator
    starts fresh here (the frozen inputs keep the SHA-256 of every WAV file)."""
    import bench_signals as rb
    rb.rng = np.random.default_rng(7)
    for de in (0.005, 0.002):
        for kind in ("kick", "snare"):
            for sc in ("clean", "doubles", "ghosts", "bleed"):
                x, on, _ = rb.make_track(kind, sc)
                coarse = on / rb.SR + rb.rng.uniform(-de, de, len(on))
                hits = []
                for k in range(len(on)):
                    nb = ([float(coarse[k - 1])] if k > 0 else []) + ([float(coarse[k + 1])] if k + 1 < len(on) else [])
                    hits.append((float(coarse[k]), nb))
                wav = scratch / f"bench_{kind}_{sc}_{int(de * 1000)}ms.wav"
                make_corpus.write_wav(wav, x, rb.SR, "f32")
                yield f"bench {kind}/{sc}/+-{int(de * 1000)}ms", wav, "ffmpeg", hits


# The MIDI export cases (tools/parity/check_midi.py until refactor 1.5): (name, events, note, ppq, tempo);
# the name is the key in tests/golden/frozen/midi.json.

def midi_synthetic_cases():
    """500 random hits in 20 s, hits 10, 25 and 50 ms apart (overlapping notes, a note off and a note on
    on the same tick), edge values (t = 0, t < 0, missing v, v = 0 and 127, notes 0 and 127), half-tick
    ties, a 200 s span (three-byte deltas), an empty list, and other ppq/tempo pairs."""
    rng = np.random.default_rng(42)
    yield "dense 500 random hits", [{"t": float(t), "v": int(v)} for t, v in
                                   zip(sorted(rng.uniform(0.1, 19.9, 500)), rng.integers(1, 128, 500))], 36, 24000, 120.0
    yield "overlapping notes 10/25/50 ms", [{"t": 1.0, "v": 100}, {"t": 1.01, "v": 90}, {"t": 1.035, "v": 80},
                                           {"t": 1.085, "v": 70}, {"t": 1.135, "v": 60}, {"t": 1.185, "v": 127},
                                           {"t": 1.185, "v": 50}], 38, 24000, 120.0
    yield "edge values", [{"t": 0.0, "v": 1}, {"t": -0.01, "v": 5}, {"t": 0.5}, {"t": 0.6, "v": 0},
                          {"t": 0.7, "v": 127}, {"t": 0.3, "v": 64}], 0, 24000, 120.0
    yield "note 127", [{"t": 0.25, "v": 100}], 127, 24000, 120.0
    yield "half-tick ties", [{"t": (k + 0.5) / 48000, "v": 100} for k in (1000, 1001, 2400, 2401, 33333, 33334)], 36, 24000, 120.0
    yield "200-second span", [{"t": 20.0 * k + 0.123456789, "v": 100} for k in range(10)] + [{"t": 199.9, "v": 100}], 36, 24000, 120.0
    yield "empty", [], 36, 24000, 120.0
    yield "ppq 480 tempo 90", [{"t": t, "v": 100} for t in (0.1, 0.1234, 0.5, 1.0, 2.5)], 42, 480, 90.0
    yield "ppq 960 tempo 174.3", [{"t": t, "v": 77} for t in (0.1, 0.1234, 0.5, 1.0, 2.5)], 46, 960, 174.3
    yield "ppq 24000 tempo 100", [{"t": t, "v": 77} for t in (0.1, 0.1234, 0.5, 1.0, 2.5)], 49, 24000, 100.0
    yield "ppq 32767 tempo 300", [{"t": t, "v": 77} for t in (0.1, 0.1234, 0.5)], 51, 32767, 300.0


SPIKE_FILES = ("bext_ixml.wav", "pcm16_mono.wav", "float32_mono_96k.wav", "pcm16_short100.wav")
