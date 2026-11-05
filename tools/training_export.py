"""#343: aggregates and the truth list of the Training Mode logs.

    uv run python tools/training_export.py <training.jsonl>... [--out truth.json]

Reads the correction logs that View > Training Mode writes (`<DRUMSPIKE_HOME>/training/<project id>.jsonl`, one
record per hit edit or Learn mark). Prints the aggregates per role and op: the count and the histogram of
`to - from` in samples, and the count per refine rule version. Writes every record as the truth list
`[{track, role, from, to, op}]` (samples) to `--out` (default `training_truth.json`) for the measurement kit.
The printout carries no project names and no paths: it is safe to paste into a public ticket. The truth file
carries the track names; keep it with the logs.
"""
import argparse
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

# the bins of `to - from` in samples: (label, first, last), the edges inclusive
BINS = [("<=-100", None, -100), ("-99..-11", -99, -11), ("-10..-2", -10, -2), ("-1", -1, -1), ("0", 0, 0),
        ("+1", 1, 1), ("+2..+10", 2, 10), ("+11..+99", 11, 99), (">=+100", 100, None)]


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
    """`[{track, role, from, to, op}]` of the records."""
    return [{"track": r.get("track"), "role": r.get("role"), "from": r.get("from"), "to": r.get("to"), "op": r["op"]}
            for r in records]


def aggregate(records):
    """`{role: {op: {"n", "delta": {bin: n}, "refine": {version: n}}}}`: the delta only where both ends exist."""
    out = defaultdict(lambda: defaultdict(lambda: {"n": 0, "delta": Counter(), "refine": Counter()}))
    for r in records:
        a = out[r.get("role") or "-"][r["op"]]
        a["n"] += 1
        a["refine"][str(r.get("refine", "-"))] += 1
        if isinstance(r.get("from"), int) and isinstance(r.get("to"), int):
            a["delta"][delta_bin(r["to"] - r["from"])] += 1
    return out


def report(records, bad):
    """The text of the aggregates."""
    lines = [f"records: {len(records)}" + (f" ({bad} unreadable lines skipped)" if bad else "")]
    for role, ops in sorted(aggregate(records).items()):
        for op, a in sorted(ops.items()):
            lines.append(f"{role} {op}: {a['n']}")
            if a["delta"]:
                lines.append("  to-from (samples): " + ", ".join(f"{label} {a['delta'][label]}" for label, _, _ in BINS if a["delta"][label]))
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
