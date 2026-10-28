"""Features parity (P2-2b): the Rust log-filtered spectrogram against R1, for both chains.

    cargo build --release -p drumspike-devtools --bin parity
    uv run --with numpy python tools/parity/check_features.py <chain> <refs_run_dir> \\
        [--sig-dir <dir>] [--work <dir>] [--bin path] [--tol 1e-5] [--json out.json] [--md out.md]

<chain> is `cli` or `madmom` (`mix` computes the cli features, so the cli run covers it).
<refs_run_dir> is a reference run of make_detect_refs.py with
`<stem>/features.npy` (R1, float32 (frames, 84)) for every stem: the CLI chain is `run3`
(`--chain cli`, the port's AudioProcessor), the madmom chain is `run1` (madmom 27f032e). The
input of each file is `<sig-dir>/<stem>/sig44.npy` (default: <refs_run_dir>), the 44.1 kHz
signal the reference features were computed from (soxr_hq; run1 and run3 have the same
sig44.npy for all 56 files). Every file runs through `parity features <sig44.npy> <chain>
<work>/<stem>.npy`.

Per file: the frame count of the reference, of Rust and of the chain's rule (`1 + n // 441`
for cli, `ceil(n / 441)` for madmom), the max abs diff over all values, where it is, the
number of values that differ at all, and PASS when the frame counts are equal and the max
abs diff is <= --tol (1e-5).
"""
import argparse
import json
import math
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BIN = ROOT / "target" / "release" / "parity"
HOP = 441


def rule_frames(chain: str, n: int) -> int:
    return 1 + n // HOP if chain in ("cli", "mix") else math.ceil(n / HOP)


def one(binary: Path, chain: str, stem: str, sig: Path, ref: Path, work: Path, tol: float) -> dict:
    out = work / f"{stem}.{chain}.npy"
    r = subprocess.run([str(binary), "features", str(sig), chain, str(out)], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"{stem}: parity features failed: {r.stderr.strip()}")
    info = json.loads(r.stdout)
    got = np.load(out)
    want = np.load(ref)
    n = int(np.load(sig, mmap_mode="r").shape[0])
    row = {
        "stem": stem,
        "samples": n,
        "frames_ref": int(want.shape[0]),
        "frames_rust": int(got.shape[0]),
        "frames_rule": rule_frames(chain, n),
        "seconds": info["min_s"],
    }
    if got.shape == want.shape and got.size:
        d = np.abs(got.astype(np.float64) - want.astype(np.float64))
        k = int(np.argmax(d))
        row["max_abs_diff"] = float(d.flat[k])
        row["at"] = [k // want.shape[1], k % want.shape[1]]
        row["values_differ"] = int(np.count_nonzero(d))
        row["values"] = int(d.size)
    else:
        row["max_abs_diff"] = 0.0 if got.shape == want.shape else None
        row["values_differ"] = 0
        row["values"] = int(want.size)
    row["pass"] = (row["frames_ref"] == row["frames_rust"] == row["frames_rule"]
                   and row["max_abs_diff"] is not None and row["max_abs_diff"] <= tol)
    out.unlink()
    return row


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("chain", choices=["cli", "madmom", "mix"])
    ap.add_argument("refs")
    ap.add_argument("--sig-dir")
    ap.add_argument("--work")
    ap.add_argument("--bin", default=str(DEFAULT_BIN))
    ap.add_argument("--tol", type=float, default=1e-5)
    ap.add_argument("--json")
    ap.add_argument("--md")
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 4) // 4))
    a = ap.parse_args()
    refs = Path(a.refs)
    sig_dir = Path(a.sig_dir) if a.sig_dir else refs
    work = Path(a.work) if a.work else Path(tempfile.mkdtemp(prefix="features-"))
    work.mkdir(parents=True, exist_ok=True)
    stems = sorted(p.parent.name for p in refs.glob("*/features.npy"))
    if not stems:
        raise SystemExit(f"no <stem>/features.npy under {refs}")
    jobs = []
    for s in stems:
        sig = sig_dir / s / "sig44.npy"
        if not sig.exists():
            raise SystemExit(f"{s}: missing {sig}")
        jobs.append((s, sig, refs / s / "features.npy"))
    with ThreadPoolExecutor(a.jobs) as ex:
        rows = list(ex.map(lambda j: one(Path(a.bin), a.chain, j[0], j[1], j[2], work, a.tol), jobs))

    lines = [f"| Stem | Samples | Frames R1 / Rust / rule | Max abs diff | At (frame, band) | Values that differ | Result |",
             "|---|---|---|---|---|---|---|"]
    for r in rows:
        mad = "n/a" if r["max_abs_diff"] is None else f"{r['max_abs_diff']:.2e}"
        at = f"({r['at'][0]}, {r['at'][1]})" if "at" in r else "-"
        lines.append(f"| `{r['stem']}` | {r['samples']} | {r['frames_ref']} / {r['frames_rust']} / {r['frames_rule']} "
                     f"| {mad} | {at} | {r['values_differ']} of {r['values']} | {'PASS' if r['pass'] else 'FAIL'} |")
    worst = max(rows, key=lambda r: -1 if r["max_abs_diff"] is None else r["max_abs_diff"])
    total_diff = sum(r["values_differ"] for r in rows)
    total = sum(r["values"] for r in rows)
    passed = sum(r["pass"] for r in rows)
    summary = (f"{a.chain}: {passed}/{len(rows)} files PASS; frames equal on "
               f"{sum(r['frames_ref'] == r['frames_rust'] == r['frames_rule'] for r in rows)}/{len(rows)}; "
               f"largest max abs diff {worst['max_abs_diff']:.2e} (`{worst['stem']}`); "
               f"{total_diff} of {total} values differ at all")
    print("\n".join(lines))
    print(summary)
    if a.md:
        Path(a.md).write_text("\n".join(lines) + "\n\n" + summary + "\n")
    if a.json:
        Path(a.json).write_text(json.dumps({"chain": a.chain, "rows": rows, "summary": summary}, indent=1))
    return 0 if passed == len(rows) else 1


if __name__ == "__main__":
    sys.exit(main())
