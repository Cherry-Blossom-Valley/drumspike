"""#343: aggregates and the truth list of the Training Mode logs.

    uv run python tools/training_export.py <training.jsonl>... [--out truth.json]

Reads the correction logs that View > Training Mode writes (`<DRUMSPIKE_HOME>/training/<project id>.jsonl`, one
record per hit edit, confirm or Learn mark). Prints the aggregates per role and op: the count and the histogram of
`to - from` in samples (a #354 `confirm` record says `from` = `to`, so its bin is `0`), the count and the histogram per
precision (#353: `sample` at a pixel per sample or more, `ms` at a coarser zoom, `-` without a view), and the count per
refine rule version. Writes every record as the
truth list `[{track, role, from, to, op, precision}]` (samples) to `--out` (default `training_truth.json`) for the
measurement kit. The printout carries no project names and no paths: it is safe to paste into a public ticket. The
truth file carries the track names; keep it with the logs.
"""
import argparse
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

# the bins of `to - from` in samples: (label, first, last), the edges inclusive
BINS = [("<=-100", None, -100), ("-99..-11", -99, -11), ("-10..-2", -10, -2), ("-1", -1, -1), ("0", 0, 0),
        ("+1", 1, 1), ("+2..+10", 2, 10), ("+11..+99", 11, 99), (">=+100", 100, None)]

# the precisions of the printout, in this order; a record without one (an older log, a Learn mark) is `-`
PRECISIONS = ("sample", "ms", "-")


def precision_of(r):
    """The label of a record's precision (`-` when the record carries none)."""
    p = r.get("precision")
    return p if p in PRECISIONS else "-"


def read_records(paths):
    """Every record of the files, in order; a line that is not a JSON object is skipped and counted."""
    records, bad = [], 0
    for p in paths:
        for line in Path(p).read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except ValueError:
                bad += 1
                continue
            if isinstance(rec, dict) and rec.get("op"):
                records.append(rec)
            else:
                bad += 1
    return records, bad


def delta_bin(d):
    """The label of the bin of a distance `d` in samples."""
    for label, lo, hi in BINS:
        if (lo is None or d >= lo) and (hi is None or d <= hi):
            return label
    raise ValueError(d)


def truth_list(records):
    """`[{track, role, from, to, op, precision}]` of the records."""
    return [{"track": r.get("track"), "role": r.get("role"), "from": r.get("from"), "to": r.get("to"), "op": r["op"],
             "precision": r.get("precision")} for r in records]


def aggregate(records):
    """`{role: {op: {"n", "delta": {bin: n}, "refine": {version: n}, "prec": {label: {"n", "delta"}}}}}`: the deltas
    only where both ends exist, the per-precision ones under the record's own label."""
    out = defaultdict(lambda: defaultdict(lambda: {
        "n": 0, "delta": Counter(), "refine": Counter(), "prec": defaultdict(lambda: {"n": 0, "delta": Counter()})}))
    for r in records:
        a = out[r.get("role") or "-"][r["op"]]
        a["n"] += 1
        a["refine"][str(r.get("refine", "-"))] += 1
        p = a["prec"][precision_of(r)]
        p["n"] += 1
        if isinstance(r.get("from"), int) and isinstance(r.get("to"), int):
            a["delta"][delta_bin(r["to"] - r["from"])] += 1
            p["delta"][delta_bin(r["to"] - r["from"])] += 1
    return out


def report(records, bad):
    """The text of the aggregates."""
    lines = [f"records: {len(records)}" + (f" ({bad} unreadable lines skipped)" if bad else "")]
    for role, ops in sorted(aggregate(records).items()):
        for op, a in sorted(ops.items()):
            lines.append(f"{role} {op}: {a['n']}")
            if a["delta"]:
                lines.append("  to-from (samples): " + ", ".join(f"{label} {a['delta'][label]}" for label, _, _ in BINS if a["delta"][label]))
            for label in PRECISIONS:
                p = a["prec"].get(label)
                if not p:
                    continue
                hist = ", ".join(f"{label2} {p['delta'][label2]}" for label2, _, _ in BINS if p["delta"][label2])
                lines.append(f"  precision {label}: {p['n']}" + (f", to-from: {hist}" if hist else ""))
            lines.append("  refine rule: " + ", ".join(f"{v} x{n}" for v, n in sorted(a["refine"].items())))
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("logs", nargs="+", help="training .jsonl files")
    ap.add_argument("--out", default="training_truth.json", help="the truth list to write")
    args = ap.parse_args(argv)
    records, bad = read_records(args.logs)
    print(report(records, bad))
    Path(args.out).write_text(json.dumps(truth_list(records), indent=1) + "\n", encoding="utf-8")
    print(f"truth list: {len(records)} entries")
    return 0


if __name__ == "__main__":
    sys.exit(main())
