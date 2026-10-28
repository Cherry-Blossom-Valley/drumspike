"""Peak-picking parity (P2-4): the Rust picker against R3, for both pickers.

    cargo build --release -p drumspike-devtools --bin parity
    uv run --with numpy python tools/parity/check_pick.py <chain> <refs_run_dir> \\
        [--avg-dump <dir>] [--work <dir>] [--bin path] [--json out.json] [--md out.md]

<chain> is `cli` or `madmom` (`mix` uses the madmom picker, so the madmom run covers it).
<refs_run_dir> is a reference run of make_detect_refs.py with `<stem>/activations.npy` (R2)
and `<stem>/r3.json` (R3) for every stem: the CLI picker is checked on `run3` (`--chain cli`,
the port's PeakPicker), the madmom picker on `run1` (madmom 27f032e
NotePeakPickingProcessor). Every file runs through `parity pick <activations.npy> <chain>
<work>/<stem>.json`. PASS when the onset list (frame, class) is identical to R3 and every
time equals R3's time.

`--avg-dump <dir>`: `<dir>/<stem>.npy` holds the reference moving averages (T, 5) float32
(the CLI's `_moving_average`, or scipy's `uniform_filter` as madmom calls it); the Rust
averages (`--avg-out`) are then compared bit for bit, and the count of differing values is
reported.

The branch counts of the Rust picker (`branches` in the output) are summed over all files.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BIN = ROOT / "target" / "release" / "parity"


def one(binary, chain, stem, refs, work, avg_dump):
    out = work / f"{stem}.{chain}.json"
    cmd = [str(binary), "pick", str(refs / stem / "activations.npy"), chain, str(out)]
    avg_out = work / f"{stem}.{chain}.avg.npy"
    if avg_dump:
        cmd += ["--avg-out", str(avg_out)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"{stem}: parity pick failed: {r.stderr.strip()}")
    got = json.loads(out.read_text())
    ref = json.loads((refs / stem / "r3.json").read_text())
    g = [(o[0], o[1]) for o in got["onsets"]]
    w = [(o[0], o[1]) for o in ref["onsets"]]
    times_equal = all(a[2] == b[2] for a, b in zip(got["onsets"], ref["onsets"]))
    row = {
        "stem": stem,
        "frames": got["frames"],
        "r3": len(w),
        "rust": len(g),
        "only_r3": len(set(w) - set(g)),
        "only_rust": len(set(g) - set(w)),
        "identical": g == w and times_equal,
        "branches": got["branches"],
    }
    if avg_dump:
        a = np.load(avg_out)
        b = np.load(Path(avg_dump) / f"{stem}.npy")
        row["avg_values"] = int(b.size)
        row["avg_differ"] = int(np.count_nonzero(a != b)) if a.shape == b.shape else int(b.size)
        avg_out.unlink()
    out.unlink()
    return row


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("chain", choices=["cli", "madmom", "mix"])
    ap.add_argument("refs")
    ap.add_argument("--avg-dump")
    ap.add_argument("--work")
    ap.add_argument("--bin", default=str(DEFAULT_BIN))
    ap.add_argument("--json")
    ap.add_argument("--md")
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 4) // 2))
    a = ap.parse_args()
    refs = Path(a.refs)
    work = Path(a.work) if a.work else Path(tempfile.mkdtemp(prefix="pick-"))
    work.mkdir(parents=True, exist_ok=True)
    stems = sorted(p.parent.name for p in refs.glob("*/r3.json") if (p.parent / "activations.npy").exists())
    if not stems:
        raise SystemExit(f"no <stem>/r3.json with activations.npy under {refs}")
    with ThreadPoolExecutor(a.jobs) as ex:
        rows = list(ex.map(lambda s: one(Path(a.bin), a.chain, s, refs, work, a.avg_dump), stems))

    avg = a.avg_dump is not None
    head = "| Stem | Frames | R3 onsets | Rust onsets | Only R3 / only Rust | " + ("Avg values that differ | " if avg else "") + "Result |"
    lines = [head, "|---" * (head.count("|") - 1) + "|"]
    for r in rows:
        lines.append(f"| `{r['stem']}` | {r['frames']} | {r['r3']} | {r['rust']} | {r['only_r3']} / {r['only_rust']} | "
                     + (f"{r['avg_differ']} of {r['avg_values']} | " if avg else "")
                     + ("identical" if r["identical"] else "DIFF") + " |")
    branches = {}
    for r in rows:
        for k, v in r["branches"].items():
            branches[k] = branches.get(k, 0) + v
    ident = sum(r["identical"] for r in rows)
    summary = (f"{a.chain}: {ident}/{len(rows)} files identical to R3; "
               f"{sum(r['rust'] for r in rows)} Rust onsets, {sum(r['r3'] for r in rows)} R3 onsets")
    if avg:
        summary += (f"; moving average: {sum(r['avg_differ'] for r in rows)} of "
                    f"{sum(r['avg_values'] for r in rows)} values differ")
    summary += f"; branches {json.dumps(branches)}"
    print("\n".join(lines))
    print(summary)
    if a.md:
        Path(a.md).write_text("\n".join(lines) + "\n\n" + summary + "\n")
    if a.json:
        Path(a.json).write_text(json.dumps({"chain": a.chain, "rows": rows, "branches": branches, "summary": summary}, indent=1))
    return 0 if ident == len(rows) else 1


if __name__ == "__main__":
    sys.exit(main())
