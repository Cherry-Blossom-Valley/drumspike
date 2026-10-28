"""Resampler quality: drumspike-core (parity binary, `resample`) vs soxr_hq on the corpus.

    cargo build --release -p drumspike-devtools
    uv run --with librosa==0.10.2.post1 --with soxr==1.1.0 --with numpy --with soundfile \\
        python tools/parity/check_resample.py <corpus_dir> [--real <samples_dir>] \\
        [--work <dir>] [--json out.json] [--bin path] [--mix ffmpeg|average] [--min-snr 80]

The corpus comes from make_corpus.py (18 files at 48, 96 and 44.1 kHz). `--real` adds the
close-mic tracks of the Pro Tools sessions under <samples_dir>/<Session>/Audio Files/*.wav
whose names contain `k in`, `s top`, `t1`..`t4` (also `t 1`), `oh` or `hh` as a word, and not
`samp`/`trig` (sample and trigger stems are not close mics). The source files are only read.

Per file the parity binary reads the source as mono f32 with the crate's reader (the mix mode
of `--mix`, default ffmpeg like engine.detector_input), writes that mono signal at the native
rate (`--mono-out`) and the Rust 44.1 kHz result. The reference is exactly the reference
resampler's call on the mono signal, `librosa.load(path, sr=44100, mono=True)` with soxr 1.1.0 (res_type soxr_hq, fix=True),
applied to the mono WAV the binary wrote, so both paths resample the same samples. The script
also loads the original with librosa (sr=None, mono=True) and reports whether that equals the
binary's mono signal (`input_same`); for multichannel files in the ffmpeg mode it does not
(average vs the ffmpeg fold), which is why soxr gets the binary's mono WAV.

Metrics: the lag with the best SNR among -2..2 output samples (expected 0: both filters are
zero-delay linear phase), SNR = 10 log10(sum(ref^2) / sum((out - ref)^2)) over the aligned
overlap, max |out - ref|, and the output lengths (both must be ceil(n * 147 / M)). A 44.1 kHz
file must come back bit-identical (pass-through; SNR printed as inf). PASS needs SNR >= the
threshold (default 80 dB) and equal lengths. Exit status 0 when every file passes.

Resumable: each file's result is cached as <work>/results/<name>.json and reused on a rerun;
the large WAVs are deleted after each file unless --keep is given.
"""
import argparse
import glob
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import librosa
import numpy as np
import soundfile as sf
import soxr

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEFAULT_BIN = ROOT / "target" / "release" / "parity"
ROLE = re.compile(r"(\bk in\b|\bs top\b|\bt ?[1-4]\b|\boh\b|\bhh\b)")


def real_tracks(samples_dir):
    """Close-mic WAVs of every session under samples_dir, by the owner's naming scheme."""
    out = []
    for p in sorted(glob.glob(os.path.join(samples_dir, "*", "Audio Files", "*.wav"))):
        name = re.sub(r"[_.]", " ", os.path.basename(p).lower())
        if ROLE.search(name) and "samp" not in name and "trig" not in name:
            out.append(Path(p))
    return out


def snr_db(out, ref):
    d = out.astype(np.float64) - ref.astype(np.float64)
    den = float(np.sum(d * d))
    num = float(np.sum(ref.astype(np.float64) ** 2))
    if den == 0.0:
        return float("inf"), 0.0
    return 10.0 * np.log10(num / den), float(np.max(np.abs(d)))


def best_lag(out, ref, lags=(0, -1, 1, -2, 2)):
    """Lag (ref index minus out index) with the highest SNR over the overlap; ties keep 0."""
    best = None
    for lag in lags:
        if lag >= 0:
            o, r = out[: len(out) - lag], ref[lag:]
        else:
            o, r = out[-lag:], ref[: len(ref) + lag]
        n = min(len(o), len(r))
        s, m = snr_db(o[:n], r[:n])
        if best is None or s > best[1]:
            best = (lag, s, m)
    return best


def check_file(path, args, work):
    stem = path.stem if path.parent.name != "Audio Files" else f"{path.parents[1].name} - {path.stem}"
    cache = work / "results" / (stem + ".json")
    if cache.exists():
        r = json.loads(cache.read_text())
        r["cached"] = True
        return r
    out_wav = work / (stem + ".out.wav")
    mono_wav = work / (stem + ".mono.wav")
    cmd = [str(args.bin), "resample", str(path), str(out_wav), "--mix", args.mix, "--mono-out", str(mono_wav)]
    t = time.perf_counter()
    p = subprocess.run(cmd, capture_output=True, text=True)
    wall = time.perf_counter() - t
    if p.returncode != 0:
        return {"name": stem, "error": p.stderr.strip() or f"exit {p.returncode}"}
    meta = json.loads(p.stdout)
    sr_in = meta["sr_in"]
    out, sr_out = sf.read(out_wav, dtype="float32")
    mono, sr_mono = sf.read(mono_wav, dtype="float32")
    assert sr_out == 44100 and sr_mono == sr_in
    # Does librosa's own read of the original equal the binary's mono signal?
    orig, _ = librosa.load(str(path), sr=None, mono=True)
    if len(orig) == len(mono):
        in_diff = float(np.max(np.abs(orig.astype(np.float64) - mono.astype(np.float64)))) if len(mono) else 0.0
    else:
        in_diff = float("inf")
    # The reference: exactly the reference resampler's call on the mono signal.
    t = time.perf_counter()
    ref, sr_ref = librosa.load(str(mono_wav), sr=44100, mono=True)
    ref_seconds = time.perf_counter() - t
    assert sr_ref == 44100
    want_len = -(-len(mono) * 147 // (sr_in * 147 // 44100)) if sr_in != 44100 else len(mono)
    r = {
        "name": stem, "sr_in": sr_in, "n_in": len(mono), "n_out": len(out), "n_ref": len(ref),
        "n_want": want_len, "taps_per_phase": meta["taps_per_phase"], "rust_seconds": meta["seconds"],
        "rust_wall_seconds": wall, "soxr_seconds": ref_seconds, "input_same": in_diff == 0.0,
        "input_max_diff": in_diff,
    }
    if sr_in == 44100:
        same = len(out) == len(mono) and np.array_equal(out, mono)
        r.update({"lag": 0, "snr_db": float("inf") if same else snr_db(out, ref)[0],
                  "max_diff": 0.0 if same else float(np.max(np.abs(out - mono))), "passthrough": same})
        r["pass"] = same and len(ref) == len(out)
    else:
        lag, s, m = best_lag(out, ref)
        r.update({"lag": lag, "snr_db": s, "max_diff": m})
        r["pass"] = bool(s >= args.min_snr and len(out) == len(ref) == want_len)
    cache.write_text(json.dumps(r, indent=1))
    if not args.keep:
        out_wav.unlink(missing_ok=True)
        mono_wav.unlink(missing_ok=True)
    return r


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("corpus", type=Path)
    ap.add_argument("--real", type=Path, help="samples root with <Session>/Audio Files/*.wav")
    ap.add_argument("--work", type=Path, help="work and cache dir (default: a temp dir)")
    ap.add_argument("--json", type=Path)
    ap.add_argument("--bin", type=Path, default=DEFAULT_BIN)
    ap.add_argument("--mix", default="ffmpeg", choices=["ffmpeg", "average"])
    ap.add_argument("--min-snr", type=float, default=80.0)
    ap.add_argument("--keep", action="store_true", help="keep the WAVs the binary wrote")
    args = ap.parse_args()
    if not args.bin.exists():
        sys.exit(f"parity binary missing: {args.bin} (cargo build --release -p drumspike-devtools)")
    work = args.work or Path(tempfile.mkdtemp(prefix="drumspike-resample-"))
    (work / "results").mkdir(parents=True, exist_ok=True)
    files = sorted(args.corpus.glob("*.wav"))
    if args.real:
        files += real_tracks(args.real)
    print(f"soxr {soxr.__version__}, librosa {librosa.__version__}, numpy {np.__version__}; work {work}")
    print(f"{'file':52s} {'sr_in':>6s} {'n_in':>9s} {'n_out':>9s} {'n_ref':>9s} lag {'SNR dB':>8s} {'max|diff|':>10s} {'rust s':>7s} {'in=':>3s} result")
    results = []
    ok = True
    for f in files:
        r = check_file(f, args, work)
        results.append(r)
        if "error" in r:
            ok = False
            print(f"{r['name']:52s} ERROR {r['error']}")
            continue
        snr = "inf" if r["snr_db"] == float("inf") else f"{r['snr_db']:.2f}"
        verdict = "PASS" if r["pass"] else "FAIL"
        ok &= r["pass"]
        print(f"{r['name'][:52]:52s} {r['sr_in']:6d} {r['n_in']:9d} {r['n_out']:9d} {r['n_ref']:9d} {r['lag']:3d} {snr:>8s} {r['max_diff']:10.3e} {r['rust_seconds']:7.3f} {'y' if r['input_same'] else 'n':>3s} {verdict}{' (cached)' if r.get('cached') else ''}", flush=True)
    done = [r for r in results if "error" not in r]
    finite = [r["snr_db"] for r in done if r["snr_db"] != float("inf")]
    print(f"{sum(r['pass'] for r in done)}/{len(results)} pass; min SNR {min(finite):.2f} dB; max |diff| {max(r['max_diff'] for r in done):.3e}")
    if args.json:
        args.json.write_text(json.dumps(results, indent=1))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
