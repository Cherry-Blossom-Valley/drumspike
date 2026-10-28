"""I/O, peak and velocity parity: drumspike-core (parity binary) vs the frozen engine.py on the corpus.

    cargo build --release -p drumspike-devtools
    uv run --project . python tools/parity/check_parity.py [corpus_dir] [--json out.json] [--bin path]

The corpus comes from make_corpus.py (run it when the folder has no manifest.json). For
every file and both mix modes the reference is what the retired server gave: engine.load_f32
(the ffmpeg decode of the fusing arm64 build), engine.compute_peaks and engine.velocity_map at
the manifest's t0 list, frozen in tests/golden/frozen/parity_io.json (P3-12). The Rust side is
the parity binary. Checks per case:

  decode      every sample bit-identical to the reference decode (SHA-256 of the f32 samples); the
              Rust reader equals the fusing build on every host, so this holds on x86_64 too
  header      sr, channels, n_samples and the bext time reference
  peaks       every min/max value identical as an integer of 1e-5, the RMS within one step (P1-3);
              counts the values that are not bit-identical
  tiles       Track.peaks(level, from, to) equals the slice of the reference level with the
              ensureSamples clamp, for windows inside, across the end and past the end
  samples     Track.samples(from, to) equals x[s:e] with the same clamp
  velocities  the reference velocities at the same times, exact

Exit status 0 when every case passes.
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
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "tests"))
import frozen  # noqa: E402

STORE = frozen.store("parity_io", "check_parity.py: per corpus file and mix mode the Python server's decode "
                     "(SHA-256), peaks (integers of 1e-5), tile and sample windows and velocities at the t0 list")
DEFAULT_BIN = ROOT / "target" / "release" / "parity"


def clamp(n, a, b):
    s = min(max(0, a), n)
    e = max(s, min(n, b))
    return s, e


def tile_windows(levels, n_samples):
    """Tile requests: inside, across the end, past the end, reversed; one per level."""
    req = []
    for bs, lvl in levels.items():
        n = lvl["n"]
        req += [(bs, 0, min(n, 3)), (bs, max(0, n - 2), n + 5), (bs, n + 10, n + 20), (bs, 2, 1)]
    return req


def sample_windows(n_samples):
    return [(0, 10), (n_samples - 5, n_samples + 100), (n_samples + 1, n_samples + 2), (-5, 3), (10, 5), (1000, 3000)]


def run_rust(bin_path, wav, mode, t0s, tiles, windows, pcm_out):
    cmd = [str(bin_path), str(wav), "--mix", mode, "--pcm-out", str(pcm_out)]
    t0file = pcm_out.with_suffix(".t0.json")
    t0file.write_text(json.dumps([float(t) for t in t0s]))
    cmd += ["--t0-file", str(t0file)]
    for bs, a, b in tiles:
        cmd += ["--tile", f"{bs},{a},{b}"]
    for a, b in windows:
        cmd += ["--samples", f"{a},{b}"]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"parity failed on {wav.name} ({mode}): {r.stderr[-500:]}")
    d = json.loads(r.stdout)
    assert isinstance(d, dict) and {"info", "peaks", "velocities", "tiles", "samples"} <= set(d), list(d)[:5]
    return d


def q5(values):
    """Peak values (rounded to 5 decimals by both sides) as integers of 1e-5."""
    return [int(round(v * 1e5)) for v in values]


def f32_sha(a):
    return hashlib.sha256(np.asarray(a, dtype=np.float32).tobytes()).hexdigest()


def reference(entry, wav, mode):
    """The frozen reference of one case (P3-12, tests/golden/frozen/parity_io.json): the header, the SHA-256
    of the ffmpeg decode of the retired server (engine.load_f32), its peak levels (engine.compute_peaks) as
    integers of 1e-5, the tile and sample windows of that decode and engine.velocity_map at the t0 list."""
    inputs = {"wav_sha256": hashlib.sha256(wav.read_bytes()).hexdigest(), "mode": mode, "t0s": entry["t0s"]}
    return STORE.ref(f"{entry['name']} [{mode}]", inputs, canonical_host=True)


def within_one(a, b):
    return len(a) == len(b) and all(abs(u - v) <= 1 for u, v in zip(a, b))


def check_case(entry, wav, mode, bin_path, scratch):
    """The Rust side against the frozen reference: decode, header and sample windows exact, min and max
    exact as integers of 1e-5, RMS within one step of 1e-5 (P1-3), velocities exact."""
    problems = []
    ref = reference(entry, wav, mode)
    t0s = entry["t0s"]
    tiles = [(t["level"], t["from"], t["to"]) for t in ref["tiles"]]
    windows = [(w["from"], w["to"]) for w in ref["samples"]]
    pcm_out = scratch / f"{wav.stem}.{mode}.f32"
    rs = run_rust(bin_path, wav, mode, t0s, tiles, windows, pcm_out)

    # decode: bit-identical samples (the Rust reader equals the fusing arm64 ffmpeg build on every host)
    y = np.fromfile(pcm_out, dtype=np.float32)
    if len(y) != ref["n_samples"]:
        problems.append(f"decode: {len(y)} samples, reference {ref['n_samples']}")
    elif f32_sha(y) != ref["decode_sha256"]:
        problems.append("decode: the samples differ from the reference (SHA-256)")
    # header
    info = rs["info"]
    if info["sample_rate"] != ref["sr"]:
        problems.append(f"sr {info['sample_rate']} != {ref['sr']}")
    if info["channels"] != entry["channels"]:
        problems.append(f"channels {info['channels']} != {entry['channels']}")
    if rs["n_samples"] != ref["n_samples"]:
        problems.append(f"n_samples {rs['n_samples']} != {ref['n_samples']}")
    tref = (info.get("bext") or {}).get("time_reference")
    if tref != entry.get("time_reference"):
        problems.append(f"time_reference {tref} != {entry.get('time_reference')}")
    # peaks
    n_vals = n_exact = 0
    for bs, lvl in ref["peaks"].items():
        r = rs["peaks"].get(bs)
        if r is None or r["n"] != lvl["n"]:
            problems.append(f"level {bs}: n {r and r['n']} != {lvl['n']}")
            continue
        for key in ("min", "max", "rms"):
            got = q5(r[key])
            n_vals += len(got)
            n_exact += sum(a == b for a, b in zip(got, lvl[key]))
            if not (got == lvl[key] or (key == "rms" and within_one(got, lvl[key]))):
                problems.append(f"level {bs} {key}: values differ from the reference")
    # tiles
    for want, t in zip(ref["tiles"], rs["tiles"]):
        where = f"tile {want['level']},{want['from']},{want['to']}"
        if t is None:
            problems.append(f"{where}: missing level")
            continue
        if t["start"] != want["start"]:
            problems.append(f"{where}: rust start {t['start']}, reference {want['start']}")
            continue
        for key in ("min", "max", "rms"):
            got = q5(t[key])
            if not (got == want[key] or (key == "rms" and within_one(got, want[key]))):
                problems.append(f"{where} {key}: rust len {len(got)}, reference {want['start']}:{want['end']}")
                break
    # samples
    for want, w in zip(ref["samples"], rs["samples"]):
        got = np.asarray(w["data"], dtype=np.float32)
        if w["start"] != want["start"] or len(got) != want["end"] - want["start"] or f32_sha(got) != want["sha256"]:
            problems.append(f"samples {want['from']},{want['to']}: rust start {w['start']} len {len(got)}, "
                            f"reference {want['start']}:{want['end']}")
    # velocities
    vels = ref["velocities"]
    n_vel = 0
    if len(rs["velocities"]) != len(vels):
        problems.append(f"velocities: {len(rs['velocities'])} values, reference {len(vels)}")
    for k, (p, r) in enumerate(zip(vels, rs["velocities"])):
        if p == r:
            n_vel += 1
        else:
            problems.append(f"velocity[{k}] t0={t0s[k]!r}: reference {p} rust {r}")
    ok = not problems
    detail = (f"decode {ref['n_samples']} samples {'identical' if f32_sha(y) == ref['decode_sha256'] else 'DIFFER'}; "
              f"peaks {n_vals} values, {n_vals - n_exact} not identical; velocities {n_vel}/{len(vels)}")
    return ok, detail, problems, {"n_samples": ref["n_samples"], "peak_values": n_vals,
                                  "peak_not_identical": n_vals - n_exact,
                                  "velocities": len(vels), "velocities_exact": n_vel, "problems": problems}


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("corpus", nargs="?", type=Path, default=HERE / "corpus")
    ap.add_argument("--bin", type=Path, default=DEFAULT_BIN)
    ap.add_argument("--json", type=Path, default=None)
    args = ap.parse_args(argv)
    if not args.bin.exists():
        print(f"parity binary missing: {args.bin} (cargo build --release -p drumspike-devtools)")
        return 2
    manifest = args.corpus / "manifest.json"
    if not manifest.exists():
        subprocess.run([sys.executable, str(HERE / "make_corpus.py"), str(args.corpus)], check=True)
    entries = json.loads(manifest.read_text())["files"]
    assert isinstance(entries, list) and entries, type(entries)
    scratch = args.corpus / "_rust"
    scratch.mkdir(exist_ok=True)
    results = {}
    n_ok = 0
    for entry in entries:
        wav = args.corpus / entry["name"]
        for mode in ("ffmpeg", "average"):
            name = f"{entry['name']} [{mode}]"
            ok, detail, problems, rec = check_case(entry, wav, mode, args.bin, scratch)
            results[name] = rec
            n_ok += ok
            print(f"{'PASS' if ok else 'FAIL'}  {name:42s} {detail}")
            for p in problems[:8]:
                print(f"      {p}")
            if len(problems) > 8:
                print(f"      ... {len(problems) - 8} more")
    total = len(results)
    print(f"\n{n_ok}/{total} cases PASS")
    if args.json:
        args.json.write_text(json.dumps({"cases": results, "pass": n_ok, "total": total}, indent=1))
    return 0 if n_ok == total else 1


if __name__ == "__main__":
    sys.exit(main())
