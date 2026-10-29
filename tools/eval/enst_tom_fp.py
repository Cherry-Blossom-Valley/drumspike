"""Where the tom false positives of the DSP detector come from (Detector #8 package 8, tickets #90 and #96).

    uv run python tools/eval/enst_tom_fp.py            # score the dsp run files, print the table, verdict
    uv run python tools/eval/enst_tom_fp.py --max-none 10

The runs are the `dsp` run files that `enst_gate.py` scores (`~/.cache/drumspike/fix-metric-model-vs-dsp/enst/run/dsp`),
built by the release `service_tool`. The scored output of a tom mic is what the gate scores: the hits without the
`bleed` and `cut` flags. A tom false positive (FP) is a scored hit that matches no own tom annotation of its mic
within +-50 ms (one to one, nearest first, per item: a hit is matched inside its own item only).

The cause of an FP is the nearest annotation of another drum within +-50 ms: kick, snare, hat, cymbal, tom-other
(a tom annotation mapped to another tom mic), or none. Annotations with an unmapped label (`cb`, `sweep`, `sticks`)
are no cause. The same cause is also counted for the flagged hits (`bleed` or `cut`): those are the hits the
attribution rules already removed from the output, so the table shows which share the rules catch and which they miss.

Printed: per drummer and cause the FP count and share (the shares sum to the printed total), per drummer and tom mic
the pooled P/R/F1 of the gate's semantics next to the per-item matching, and the flagged hits per cause.

#96, per rule (reporting only): a flagged hit carries `by`, the names of the rules that marked it (CD, v2-kick,
v2-snare, pair, v5-kick; see `TomBleed::by` in the core). Printed: the flagged hits per rule (any rule that fired),
and per drummer the first rule of each flagged hit in the fixed order RULES (the sum check: the first-rule columns
add up to the flagged total). A cut hit with no rule is `cut`; a flagged hit with neither is `unattributed` and fails.
A6 (the cross-track rule) takes its hits out of the events, so its counts come from the track's `detector` record:
the dropped candidates (`dropped`) and the step-1 marks that reach the CD step (`survive`). The scored hat FPs
carry `a6_lead_ms` and `a6_level_db`: their quartiles are printed. The hat left-share is information only here.

Exit 0 on PASS, 1 on FAIL (the cause shares do not sum to the total, the share of `none` is above `--max-none`, or the
first-rule columns do not sum to the flagged total, or a flagged hit has no rule).
Aggregate numbers only leave this host.
"""
import argparse
import json
import os
import statistics
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "eval"))
import enst  # noqa: E402
import enst_gate  # noqa: E402

OUT = Path.home() / ".cache" / "drumspike" / "fix-metric-model-vs-dsp" / "enst"
CONFIG = "dsp"
WINDOW = enst.WINDOWS[0]           # the +-50 ms headline window
CAUSES = ("kick", "snare", "hat", "cymbal", "tom-other", "none")
TOM_MICS = enst.TOM_MICS
RULES = ("CD", "v2-kick", "v2-snare", "pair", "v5-kick")   # the fixed order of the first rule
FIRST_COLS = RULES + ("cut", "unattributed")


def item_annotations(evs, tom_map):
    """[(t, role)] of one item for every annotation with a role: role is kick, snare, hat, cymbal, or
    `tom:<mic>` for a tom annotation (the mic from the drummer's tom mapping, `tom:?` when undecided).
    Unmapped labels are left out."""
    out = []
    for t, lab in evs:
        role = enst.label_role(lab)
        if role is None:
            continue
        if role == "tom":
            mic = tom_map.get(enst.TOM_ALIAS.get(lab, lab), "?")
            out.append((t, f"tom:{mic}"))
        else:
            out.append((t, role))
    return out


def cause_of(t, mic, anns):
    """The cause of a hit at t on tom mic `mic`: the role of the nearest annotation of another drum within the
    window (a tom annotation of this mic is no cause), else `none`."""
    best = None
    for ta, role in anns:
        if role == f"tom:{mic}":
            continue
        d = abs(ta - t)
        if d <= WINDOW and (best is None or d < best[0]):
            best = (d, role)
    if best is None:
        return "none"
    role = best[1]
    if role.startswith("tom:"):
        return "tom-other"
    return role


def first_rule(e):
    """The first rule of a flagged hit in the fixed order RULES; `cut` for a loudness cut without a rule, else
    `unattributed` (a bleed flag with no rule name)."""
    by = e.get("by") or []
    for r in RULES:
        if r in by:
            return r
    return "cut" if e.get("cut") else "unattributed"


def score(root, out, tool_fp, log=print):
    """(counts, flagged, pooled, rules, hat_a6) of the dsp run files: counts[drummer][mic][cause] of the scored FPs,
    flagged[drummer][cause] of the flagged hits (bleed or cut) that sit at another drum, pooled[drummer][mic] of the
    gate semantics, rules[drummer] the #96 per-rule counts (see the module documentation), hat_a6 the (lead, level)
    keys of the scored hat FPs (None where the key is missing)."""
    idx = enst.index_items(root)
    counts, flagged, pooled, rules = {}, {}, {}, {}
    hat_a6 = []
    for d in enst.DRUMMERS:
        tom_map = enst.tom_mapping(root, d)
        counts[d] = {m: {c: 0 for c in CAUSES} for m in TOM_MICS}
        flagged[d] = {c: 0 for c in CAUSES}
        flagged[d]["own-ref"] = 0
        pooled[d] = {m: {"ref": [], "est": []} for m in TOM_MICS}
        rules[d] = {"any": {r: 0 for r in RULES}, "first": {c: 0 for c in FIRST_COLS},
                    "a6": {"dropped": 0, "survive": 0}}
        for stem, item in idx[d].items():
            res = json.loads((out / "run" / CONFIG / d / f"{stem}.json").read_text())
            if res.get("tool_fp") != tool_fp:
                raise SystemExit(f"FAIL {d}/{stem}: run file is stale (tool_fp {res.get('tool_fp')!r})")
            anns = item_annotations(enst.read_annotation(item["ann"]), tom_map)
            for mic in TOM_MICS:
                if mic not in res["events"]:
                    continue
                own = sorted(t for t, role in anns if role == f"tom:{mic}")
                evs = res["events"][mic]
                scored_ev = sorted(((float(e["t"]), e) for e in evs if not e.get("bleed") and not e.get("cut")),
                                   key=lambda p: p[0])
                scored = [t for t, _ in scored_ev]
                pooled[d][mic]["ref"] += own
                pooled[d][mic]["est"] += scored
                matched = {j for _, j in enst._match_events(own, scored, WINDOW)}
                for j, (t, e) in enumerate(scored_ev):
                    if j not in matched:
                        cause = cause_of(t, mic, anns)
                        counts[d][mic][cause] += 1
                        if cause == "hat":
                            hat_a6.append((e.get("a6_lead_ms"), e.get("a6_level_db")))
                for e in evs:
                    if not (e.get("bleed") or e.get("cut")):
                        continue
                    t = float(e["t"])
                    if any(abs(t - o) <= WINDOW for o in own):
                        flagged[d]["own-ref"] += 1
                    else:
                        flagged[d][cause_of(t, mic, anns)] += 1
                    by = e.get("by") or []
                    for r in RULES:
                        if r in by:
                            rules[d]["any"][r] += 1
                    rules[d]["first"][first_rule(e)] += 1
                cb = ((res.get("detector") or {}).get(mic) or {}).get("cross_bleed") or {}
                rules[d]["a6"]["dropped"] += len(cb.get("dropped") or [])
                rules[d]["a6"]["survive"] += int(cb.get("survive") or 0)
    return counts, flagged, pooled, rules, hat_a6


def quartiles(vals):
    """`p25 / p50 / p75` of the values, `n/a` with fewer than two."""
    if len(vals) < 2:
        return "n/a"
    q = statistics.quantiles(sorted(vals), n=4, method="inclusive")
    return " / ".join(f"{v:.2f}" for v in q)


def main(argv=None):
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    ap.add_argument("--root", default=None, help="the ENST-Drums root (default: ENST_ROOT, then the usual copies)")
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--tool", default=None, help="the service_tool the run files were measured with (default: the release build)")
    ap.add_argument("--max-none", type=float, default=10.0, help="FAIL when more than this share of the FPs has no cause (default 10)")
    a = ap.parse_args(argv)
    root = a.root or os.environ.get("ENST_ROOT")
    if root is None:
        for cand in (Path.home() / ".cache" / "drumspike" / "datasets" / "ENST-drums-public",
                     Path("/Volumes/Corsair-4TB/Datasets/ENST_Drums/ENST-drums-public")):
            if cand.is_dir():
                root = str(cand)
                break
    if root is None or not Path(root).is_dir():
        raise SystemExit(f"FAIL: ENST_ROOT {root} is missing")
    out = Path(a.out)
    tool_path = Path(a.tool or os.environ.get("DRUMSPIKE_CALIB_TOOL") or ROOT / "target" / "release" / "service_tool")
    tool_fp = enst.tool_fingerprint(tool_path)
    stale = enst_gate.stale_run_files(out, root, CONFIG, tool_fp)
    if stale:
        raise SystemExit(f"FAIL: {len(stale)} run file(s) stale or missing (first {stale[0]}); run enst_gate.py first")

    counts, flagged, pooled, rules, hat_a6 = score(root, out, tool_fp)
    lines = [f"tom false positives at +-{int(WINDOW * 1000)} ms, config {CONFIG} (build {tool_fp}):", "",
             "Scored FPs per drummer and cause (all tom mics; shares of the drummer's total):", "",
             "| drummer | " + " | ".join(CAUSES) + " | total | shares % (" + " / ".join(CAUSES) + ") |",
             "|---|" + "---|" * (len(CAUSES) + 2)]
    verdict_ok = True
    grand = {c: 0 for c in CAUSES}
    for d in enst.DRUMMERS:
        tot = {c: sum(counts[d][m][c] for m in TOM_MICS) for c in CAUSES}
        n = sum(tot.values())
        for c in CAUSES:
            grand[c] += tot[c]
        if sum(tot.values()) != n:
            verdict_ok = False
        shares = [100.0 * tot[c] / n if n else 0.0 for c in CAUSES]
        lines.append(f"| {d} | " + " | ".join(str(tot[c]) for c in CAUSES) + f" | {n} | "
                     + " / ".join(f"{s:.1f}" for s in shares) + " |")
    total = sum(grand.values())
    gshares = {c: 100.0 * grand[c] / total if total else 0.0 for c in CAUSES}
    lines.append("| all | " + " | ".join(str(grand[c]) for c in CAUSES) + f" | {total} | "
                 + " / ".join(f"{gshares[c]:.1f}" for c in CAUSES) + " |")
    lines.append("")
    lines += ["Scored FPs per drummer and tom mic (rows: the mic; the cause columns as above):", "",
              "| drummer | mic | " + " | ".join(CAUSES) + " | total |", "|---|---|" + "---|" * (len(CAUSES) + 1)]
    for d in enst.DRUMMERS:
        for m in TOM_MICS:
            if not pooled[d][m]["ref"]:
                continue
            cs = counts[d][m]
            lines.append(f"| {d} | {m} | " + " | ".join(str(cs[c]) for c in CAUSES) + f" | {sum(cs.values())} |")
    lines.append("")
    lines += ["Gate semantics versus per-item matching, per drummer and tom mic (ref hits, scored output, P, R, F1, FP):", "",
              "| drummer | mic | ref | output | P | R | F1 | FP pooled | FP per item |", "|---|---|---|---|---|---|---|---|---|"]
    for d in enst.DRUMMERS:
        for m in TOM_MICS:
            cell = pooled[d][m]
            if not cell["ref"]:
                continue
            p, r, f = enst.prf(sorted(cell["ref"]), sorted(cell["est"]), WINDOW)
            fp_pooled = len(cell["est"]) - len(enst._match_events(sorted(cell["ref"]), sorted(cell["est"]), WINDOW))
            lines.append(f"| {d} | {m} | {len(cell['ref'])} | {len(cell['est'])} | {p:.3f} | {r:.3f} | {f:.3f} | "
                         f"{fp_pooled} | {sum(counts[d][m].values())} |")
    lines.append("")
    lines += ["Flagged hits (bleed or cut) per drummer and cause: the hits the rules already removed. `own-ref` = within the window of an own tom annotation (a real stroke the rules removed):", "",
              "| drummer | " + " | ".join(CAUSES) + " | own-ref |", "|---|" + "---|" * (len(CAUSES) + 1)]
    for d in enst.DRUMMERS:
        lines.append(f"| {d} | " + " | ".join(str(flagged[d][c]) for c in CAUSES) + f" | {flagged[d]['own-ref']} |")
    lines.append("")
    lines += ["Flagged hits (bleed or cut) per rule, any rule that fired (a hit can count for several rules):", "",
              "| drummer | " + " | ".join(RULES) + " | A6 dropped | A6 survived to CD |", "|---|" + "---|" * (len(RULES) + 2)]
    for d in enst.DRUMMERS:
        rr = rules[d]
        lines.append(f"| {d} | " + " | ".join(str(rr["any"][r]) for r in RULES)
                     + f" | {rr['a6']['dropped']} | {rr['a6']['survive']} |")
    lines.append("")
    lines += ["Flagged hits per first rule (fixed order " + " > ".join(RULES) + "; cut = a loudness cut with no rule):", "",
              "| drummer | " + " | ".join(FIRST_COLS) + " | sum | flagged total |", "|---|" + "---|" * (len(FIRST_COLS) + 2)]
    for d in enst.DRUMMERS:
        first = rules[d]["first"]
        fsum = sum(first.values())
        ftot = sum(flagged[d].values())
        lines.append(f"| {d} | " + " | ".join(str(first[c]) for c in FIRST_COLS) + f" | {fsum} | {ftot} |")
        if fsum != ftot:
            lines.append(f"FAIL: {d} the first-rule columns sum to {fsum}, the flagged total is {ftot}")
            verdict_ok = False
    unattributed = sum(rules[d]["first"]["unattributed"] for d in enst.DRUMMERS)
    if unattributed:
        lines.append(f"FAIL: {unattributed} flagged hit(s) have no rule name (`by`)")
        verdict_ok = False
    lines.append("")
    hat_n = len(hat_a6)
    hat_missing = sum(1 for lead, level in hat_a6 if lead is None or level is None)
    lines += [f"Hat FPs left in the output: {hat_n}; A6 lead (ms) quartiles p25 / p50 / p75: "
              f"{quartiles([lead for lead, _ in hat_a6 if lead is not None])}; A6 level (dB, ratio against the "
              f"coinciding source hit) quartiles: {quartiles([lv for _, lv in hat_a6 if lv is not None])}; "
              f"without the keys: {hat_missing}",
              f"hat left-share: {gshares['hat']:.1f} % of the tom FPs (limit 50 %, information only in this step)",
              ""]
    none_share = gshares["none"]
    if not verdict_ok:
        lines.append("FAIL: the cause counts do not sum to the total FPs")
    if none_share > a.max_none:
        lines.append(f"FAIL: {none_share:.1f} % of the tom FPs have no cause (limit {a.max_none:.1f} %)")
        verdict_ok = False
    lines.append(f"total tom FPs (scored, all drummers and mics): {total}; sum of the causes: {sum(grand.values())}; "
                 f"none {none_share:.1f} % (limit {a.max_none:.1f} %)")
    lines.append("PASS" if verdict_ok else "FAIL")
    print("\n".join(lines))
    return 0 if verdict_ok else 1


if __name__ == "__main__":
    sys.exit(main())
