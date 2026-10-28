"""Refinement parity: drumspike-core `refine_sr` (parity binary) vs the frozen engine._refine_sr.

    cargo build --release -p drumspike-devtools
    uv run --project . python tools/parity/check_refine.py [corpus_dir] [--json out.json] [--bin path]

Cases:

  corpus      every file of make_corpus.py (ffmpeg mode; average mode as well for files
              with more than one channel), two hit lists each: "dedup" = the manifest's
              detector-style t0s after the 20 ms min-gap dedup of analyze_track, each hit
              with its previous and next neighbour (what analyze_track passes), and
              "isolated" = every t0 of the manifest on its own (silence, file edges, hits
              30 ms off, t0 past the end)
  edge        flams and rolls 4 to 13 ms apart (the neighbour midpoints clip the search
              window), one-sided neighbours, 5-, 8- and 12-sample files, hits outside the file
  window      the search window as a parameter (P2-9): the flams of "edge" and the dedup
              hits of the first corpus files with the windows -7..+7 (phase 1), -3..+13 and
              -12..+2 ms (job key "window"); every other case runs the default -2..+12 ms
  bench       the 16 scenarios of the old refinement benchmark (kick and snare x clean, doubles,
              ghosts, bleed x +-5 ms and +-2 ms detector error), synthesised by bench_signals.py
              in the benchmark's order so the seeded rng gives the same signals;
              150 hits each with the bench's prev/next neighbours

Per hit the reference is engine._refine_sr(x, t0, sr, neighbours[, lo_ms, hi_ms]) -> (t, rise_db) of the
retired Python server, frozen in tests/golden/frozen/refine.json (P3-12; the inputs of a case are the
SHA-256 of its WAV file, the mix mode, the hits and the window); the Rust side runs on the same file.
A hit is "identical" when t is the same float and rise is the same within 1e-9 dB (or None on both
sides); "within 1 sample" when |dt| * sr <= 1. Velocities are the frozen engine.velocity_map at the
Python times versus the Rust velocities at the Rust times.
Every hit that is not identical is listed with t0, both times and the sample distance.
A case fails on any hit that is not identical (parity-contract.md: time equal, rise_db within
1e-9 dB); the within-1-sample count is information only. Exit status 0 when every hit is
identical and every velocity matches.
"""
import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "tests"))
sys.path.insert(0, str(HERE))
import frozen  # noqa: E402
import helpers  # noqa: E402
from make_corpus import write_wav  # noqa: E402

DEFAULT_BIN = ROOT / "target" / "release" / "parity"
STORE = frozen.store("refine")
MIN_GAP = 0.020


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


def corpus_cases(corpus):
    manifest = corpus / "manifest.json"
    if not manifest.exists():
        subprocess.run([sys.executable, str(HERE / "make_corpus.py"), str(corpus)], check=True)
    entries = json.loads(manifest.read_text())["files"]
    assert isinstance(entries, list) and entries, type(entries)
    for entry in entries:
        modes = ("ffmpeg", "average") if entry["channels"] > 1 else ("ffmpeg",)
        for mode in modes:
            t0s = [float(t) for t in entry["t0s"]]
            coarse = dedup(t0s)
            yield (f"corpus {entry['name']} [{mode}] dedup", corpus / entry["name"], mode,
                   list(zip(coarse, neighbours_like_analyze_track(coarse))))
            yield (f"corpus {entry['name']} [{mode}] isolated", corpus / entry["name"], mode,
                   [(t, []) for t in t0s])


def bench_cases(scratch):
    """Same loop order as the old bench/refine_bench.py so the seeded rng draws the bench's signals."""
    import bench_signals as rb
    for de in (0.005, 0.002):
        for kind in ("kick", "snare"):
            for sc in ("clean", "doubles", "ghosts", "bleed"):
                x, on, _ = rb.make_track(kind, sc)
                coarse = on / rb.SR + rb.rng.uniform(-de, de, len(on))
                hits = []
                for k in range(len(on)):
                    nb = ([float(coarse[k - 1])] if k > 0 else []) + ([float(coarse[k + 1])] if k + 1 < len(on) else [])
                    hits.append((float(coarse[k]), nb))
                name = f"bench {kind}/{sc}/+-{int(de * 1000)}ms"
                wav = scratch / f"bench_{kind}_{sc}_{int(de * 1000)}ms.wav"
                write_wav(wav, x, rb.SR, "f32")
                yield name, wav, "ffmpeg", hits


def edge_cases(scratch):
    """Branches the corpus and the bench do not reach: neighbours closer than 14 ms (the
    midpoint clips the search window on either side), a file shorter than 8 samples
    (the short-cut return) and coarse hits before the start and past the end."""
    sr = helpers.SR
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


WINDOWS = ((-7.0, 7.0), (-3.0, 13.0), (-12.0, 2.0))


def window_cases(corpus, scratch, n_corpus=3):
    """The non-default windows on the flams of edge_cases and the dedup hits of the first
    corpus files: (name, wav, mode, hits, (lo_ms, hi_ms))."""
    flams = [c for c in edge_cases(scratch) if c[0].startswith("edge flams")]
    dedups = [c for c in corpus_cases(corpus) if c[0].endswith("dedup")][:n_corpus]
    for lo, hi in WINDOWS:
        for name, wav, mode, hits in flams + dedups:
            yield (f"window {lo:+g}..{hi:+g} {name}", wav, mode, hits, (lo, hi))


def run_rust(bin_path, wav, mode, hits, job_path, window=None):
    job = {"wav": str(wav), "mix": mode, "hits": [{"t0": t0, "neighbors": nb} for t0, nb in hits]}
    if window is not None:
        job["window"] = list(window)
    job_path.write_text(json.dumps(job))
    r = subprocess.run([str(bin_path), "refine", str(job_path)], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"parity refine failed on {wav.name}: {r.stderr[-500:]}")
    d = json.loads(r.stdout)
    assert isinstance(d, dict) and {"sr", "refined", "velocities"} <= set(d), list(d)[:5]
    return d


def reference(name, wav, mode, hits, window=None):
    """The frozen reference of one case: {sr, t, rise, v} (engine._refine_sr per hit, engine.velocity_map)."""
    inputs = {"wav_sha256": hashlib.sha256(Path(wav).read_bytes()).hexdigest(), "mode": mode,
              "hits": [[t0, list(nb)] for t0, nb in hits], "window": list(window) if window else None}
    return STORE.ref(name, inputs, canonical_host=True)


def check_case(name, wav, mode, hits, bin_path, scratch, window=None):
    rs = run_rust(bin_path, wav, mode, hits, scratch / "job.json", window)
    rise_tol = 1e-9
    problems, details = [], []
    ref = reference(name, wav, mode, hits, window)
    if rs["sr"] != ref["sr"]:
        problems.append(f"sr {rs['sr']} != {ref['sr']}")
    if len(rs["refined"]) != len(hits):
        problems.append(f"{len(rs['refined'])} refined hits, python {len(hits)}")
        return False, problems, details, {}
    py_t, py_rise, py_v = ref["t"], ref["rise"], ref["v"]
    sr = ref["sr"]
    n_ident = n_within = 0
    max_ds = 0.0
    max_drise = 0.0
    for k, ((t0, nb), rr) in enumerate(zip(hits, rs["refined"])):
        rt, rrise = rr["t"], rr["rise"]
        ds = (rt - py_t[k]) * sr
        rise_same = (rrise is None and py_rise[k] is None) or (
            rrise is not None and py_rise[k] is not None and abs(rrise - py_rise[k]) <= rise_tol)
        if rrise is not None and py_rise[k] is not None:
            max_drise = max(max_drise, abs(rrise - py_rise[k]))
        if rt == py_t[k] and rise_same:
            n_ident += 1
            continue
        max_ds = max(max_ds, abs(ds))
        within = abs(ds) <= 1.0 + 1e-6
        n_within += within
        details.append({"hit": k, "t0": t0, "py_t": py_t[k], "rs_t": rt, "d_samples": ds,
                        "py_rise": py_rise[k], "rs_rise": rrise, "within_1": bool(within)})
        # parity-contract.md: times are exact and rise_db is within 1e-9 dB, so any hit that is not
        # identical fails the case; "within 1 sample" stays as information in the report.
        problems.append(f"hit {k} t0={t0!r}: py {py_t[k]!r} rs {rt!r} ({ds:+.3f} samples), "
                        f"rise py {py_rise[k]} rs {rrise}")
    n_vel = sum(int(a == b) for a, b in zip(py_v, rs["velocities"]))
    if n_vel != len(hits):
        problems.append(f"velocities: {len(hits) - n_vel} of {len(hits)} differ")
    rec = {"hits": len(hits), "identical": n_ident, "within_1": n_within, "max_d_samples": max_ds,
           "max_d_rise_db": max_drise, "velocities_exact": n_vel, "problems": problems, "differences": details}
    return not problems, problems, details, rec


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("corpus", nargs="?", type=Path, default=HERE / "corpus")
    ap.add_argument("--bin", type=Path, default=DEFAULT_BIN)
    ap.add_argument("--json", type=Path, default=None)
    ap.add_argument("--no-bench", action="store_true", help="skip the refine_bench scenarios")
    args = ap.parse_args(argv)
    if not args.bin.exists():
        print(f"parity binary missing: {args.bin} (cargo build --release -p drumspike-devtools)")
        return 2
    args.corpus.mkdir(parents=True, exist_ok=True)
    scratch = args.corpus / "_refine"
    scratch.mkdir(exist_ok=True)
    cases = list(corpus_cases(args.corpus)) + list(edge_cases(scratch)) + list(window_cases(args.corpus, scratch))
    if not args.no_bench:
        cases += list(bench_cases(scratch))
    results = {}
    n_ok = 0
    tot = {"hits": 0, "identical": 0, "within_1": 0, "velocities_exact": 0}
    print(f"{'case':52s} {'hits':>5s} {'ident':>5s} {'<=1smp':>6s} {'max|d|':>7s} {'rise|d|':>8s} {'vel':>7s}")
    for name, wav, mode, hits, *win in cases:
        ok, problems, details, rec = check_case(name, wav, mode, hits, args.bin, scratch, *win)
        results[name] = rec
        n_ok += ok
        for k in tot:
            tot[k] += rec.get(k, 0)
        print(f"{'PASS' if ok else 'FAIL'} {name:47s} {rec.get('hits', 0):5d} {rec.get('identical', 0):5d} "
              f"{rec.get('within_1', 0):6d} {rec.get('max_d_samples', 0.0):7.3f} {rec.get('max_d_rise_db', 0.0):8.1e} "
              f"{rec.get('velocities_exact', 0):3d}/{rec.get('hits', 0):<3d}")
        for d in details[:6]:
            print(f"      hit {d['hit']} t0={d['t0']!r}: py {d['py_t']!r} rs {d['rs_t']!r} ({d['d_samples']:+.3f} samples) "
                  f"rise py {d['py_rise']} rs {d['rs_rise']}")
        if len(details) > 6:
            print(f"      ... {len(details) - 6} more")
        for p in problems[:4]:
            print(f"      {p}")
    total = len(results)
    print(f"\n{n_ok}/{total} cases PASS; hits {tot['hits']}, identical {tot['identical']}, "
          f"within 1 sample {tot['within_1']}, velocities exact {tot['velocities_exact']}/{tot['hits']}")
    if args.json:
        args.json.write_text(json.dumps({"cases": results, "pass": n_ok, "total": total, "totals": tot}, indent=1))
    return 0 if n_ok == total else 1


if __name__ == "__main__":
    sys.exit(main())
