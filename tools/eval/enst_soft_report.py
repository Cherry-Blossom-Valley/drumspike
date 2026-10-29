"""The soft-gate report of Detector #8 package 1 (#81): the per-role effect of the soft
rescue on ENST-Drums, hard gate vs soft gate, from the run files of the two
configurations.

    uv run python tools/eval/enst_soft_report.py            # score the two run dirs, print, verdict
    uv run python tools/eval/enst_soft_report.py --skip-missing   # ... with missing items listed, not fatal

The runs themselves are `tools/eval/enst.py --all --configs dsp-hard` with
`DRUMSPIKE_A1_SOFT_SIM=0` (the hard gate) and `--configs dsp` with the shipped defaults
(the soft gate, the same run files `enst_gate.py` scores). One result file per item and
configuration under `~/.cache/drumspike/fix-metric-model-vs-dsp/enst/run/`; aggregate
numbers only leave this host.

Printed: P/R/F1 at the +-50 ms headline window per drummer and role and pooled over the
drummers, for both gates side by side, then the counted verdict lines: the gate of the
ticket is `snare recall (soft) strictly above snare recall (hard) AND snare precision
(soft) >= 0.95` on the pooled rows. Exit 0 on PASS, 1 on FAIL.
"""
import argparse
import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "eval"))
import enst  # noqa: E402
import enst_gate  # noqa: E402

OUT = Path.home() / ".cache" / "drumspike" / "fix-metric-model-vs-dsp" / "enst"
HARD = "dsp-hard"          # the soft gate pinned off (DRUMSPIKE_A1_SOFT_SIM=0)
SOFT = "dsp"               # the shipped defaults, the gate's own configuration
SOFT_PRECISION_FLOOR = 0.95
ROLES = enst_gate.ROLES


def per_rows(per):
    """{f"{drummer}|{role}": (p, r, f1)} plus the pooled rows `pooled|role`, from a `per`
    dict of enst_gate.collect_per's shape ({(config, drummer, mic): {"ref", "est"}})."""
    rows = {}
    drummers = sorted({d for (_, d, _) in per})
    cells = {}
    for (c, d, m) in per:
        role = enst.MIC_ROLE[m] if m in enst.MIC_ROLE else ("tom" if m in enst.TOM_MICS else None)
        if role not in ROLES:
            continue
        cells.setdefault((d, role), {"ref": [], "est": []})
        cells[(d, role)]["ref"] += per[(c, d, m)]["ref"]
        cells[(d, role)]["est"] += per[(c, d, m)]["est"]
    pooled = {role: {"ref": [], "est": []} for role in ROLES}
    for d in drummers:
        for role in ROLES:
            if (d, role) not in cells or not cells[(d, role)]["ref"]:
                continue
            cell = cells[(d, role)]
            rows[f"{d}|{role}"] = enst.prf(sorted(cell["ref"]), sorted(cell["est"]), enst_gate.GATE_WINDOW)
            pooled[role]["ref"] += cell["ref"]
            pooled[role]["est"] += cell["est"]
    for role in ROLES:
        if pooled[role]["ref"]:
            rows[f"pooled|{role}"] = enst.prf(sorted(pooled[role]["ref"]), sorted(pooled[role]["est"]),
                                              enst_gate.GATE_WINDOW)
    return rows


def compare(hard, soft):
    """(lines, ok) of the two row dicts: one counted line per row, then the verdict lines
    (the pooled snare rows decide)."""
    lines, ok = [], True
    for key in sorted(set(hard) | set(soft)):
        h, s = hard.get(key), soft.get(key)
        if not h or not s:
            lines.append(f"MISSING {key}: {'no hard run' if not h else 'no soft run'}")
            ok = False
            continue
        lines.append(f"{key}: hard P {h[0]:.3f} R {h[1]:.3f} F {h[2]:.3f}  ->  soft P {s[0]:.3f} "
                     f"R {s[1]:.3f} F {s[2]:.3f}  (dR {s[1] - h[1]:+.3f}, dP {s[0] - h[0]:+.3f})")
    hs, ss = hard.get("pooled|snare"), soft.get("pooled|snare")
    if not hs or not ss:
        lines.append("FAIL: no pooled snare row to decide")
        return lines, False
    recall_up = ss[1] > hs[1]
    prec_ok = ss[0] >= SOFT_PRECISION_FLOOR
    lines.append(f"pooled snare: hard recall {hs[1]:.4f}, soft recall {ss[1]:.4f} "
                 f"(strictly above: {recall_up}); soft precision {ss[0]:.4f} "
                 f"(>= {SOFT_PRECISION_FLOOR}: {prec_ok})")
    ok = ok and recall_up and prec_ok
    lines.append("PASS" if ok else "FAIL")
    return lines, ok


def main(argv=None):
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--hard", default=HARD)
    ap.add_argument("--soft", default=SOFT)
    ap.add_argument("--skip-missing", action="store_true",
                    help="score the items both configurations have instead of FAILing on a gap")
    a = ap.parse_args(argv)
    root = os.environ.get("ENST_ROOT")
    if root is None:
        for cand in (Path.home() / ".cache" / "drumspike" / "datasets" / "ENST-drums-public",):
            if cand.is_dir():
                root = str(cand)
                break
    if root is None or not Path(root).is_dir():
        raise SystemExit(f"FAIL: ENST_ROOT {root} is missing")
    out = Path(a.out)
    tool_path = Path(os.environ.get("DRUMSPIKE_CALIB_TOOL") or ROOT / "target" / "release" / "service_tool")
    tool_fp = enst.tool_fingerprint(tool_path)
    per = {}
    for config in (a.hard, a.soft):
        idx = enst.index_items(root)
        stale = enst_gate.stale_run_files(out, root, config, tool_fp)
        if stale and not a.skip_missing:
            raise SystemExit(f"FAIL {config}: {len(stale)} run file(s) stale or missing "
                             f"(first {stale[0]}); run the passes first")
        if stale:
            print(f"note {config}: {len(stale)} run file(s) stale or missing, scored without them")
        per[config] = enst_gate.collect_per(out, root, config, tool_fp=tool_fp,
                                            allow_missing=a.skip_missing)
    hard, soft = per_rows(per[a.hard]), per_rows(per[a.soft])
    print(f"soft-gate report at +-{int(enst_gate.GATE_WINDOW * 1000)} ms "
          f"(hard = {a.hard}, soft = {a.soft}, build {tool_fp}):")
    lines, ok = compare(hard, soft)
    print("\n".join(lines))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
