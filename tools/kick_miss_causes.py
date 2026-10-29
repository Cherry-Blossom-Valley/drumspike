# /// script
# requires-python = ">=3.12"
# dependencies = ["numpy", "soundfile"]
# ///
"""Detector #8 package 3: the kick miss-cause pass over the real-session check.

    uv run --with numpy --with soundfile python tools/kick_miss_causes.py kick \
        [--samples /Users/Shared/drumspike-data/sessions] \
        [--cache ~/.cache/drumspike/learn-eval-76] [--tool <service_tool>]

Reads `refs.json` and `baseline.json` from the cache folder (tools/learn_eval.py refs, then
baseline --samples <the sessions>). One cause per kick false negative at the Sensitivity 50
working point (the app's default), disjoint, the first stage that stops it:

  double-stroke (1 to 1)  an output hit sits within 20 ms but the one-to-one matching gives it
                          to the neighbouring reference hit (two refs, one hit)
  offset                  the nearest output hit is within 50 ms, not 20 ms
  min-gap (20 ms)         the pre-dedup pick exists at Sensitivity 50 and the min-gap dedup
                          drops it against a kept neighbour
  refine                  the pre-dedup pick exists and no output hit stays near it (the
                          refinement moved it out of the window)
  gated (<gate>)          a candidate exists within 20 ms and the gate stage drops it at the
                          working point; the failed gate of the detection's own verdicts
  no candidate            nothing within 20 ms at any gate; `peak merged` when the nearest
                          candidate is the first stroke's own candidate (20 to 30 ms, the
                          neighbouring ref within 40 ms): the picker gave the double one peak

Prints the counted lines of the ticket's attribution and writes `kick_causes.json`.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import learn_eval as le  # noqa: E402

TOL = le.TOL              # 0.020, the metric's window
OFFSET_FAR = 0.050        # the offset cause's window (the offset_of window)
WORK_SENS = 50            # the app's default Sensitivity (sensitivity::DEFAULT)
STAGE_VALUES = [0, WORK_SENS, le.LEARN_SENSITIVITY, 150]  # context values around the working point


def nearest(xs: np.ndarray, t: float) -> tuple[float, float] | None:
    """(x, distance) of the nearest value of `xs` to `t`, or None when `xs` is empty."""
    if len(xs) == 0:
        return None
    i = int(np.argmin(np.abs(xs - t)))
    return float(xs[i]), abs(float(xs[i]) - t)


def cause_of(t: float, prev_used: float | None, ev: np.ndarray, picked: np.ndarray,
             cands: list[dict]) -> tuple[str, dict]:
    """The first stage that stops the reference hit at `t` (the times are the shifted refs)."""
    d = {}
    hit = nearest(ev, t)
    if hit and hit[1] <= TOL:
        return "double-stroke (1 to 1)", d
    pick = nearest(picked, t)
    if pick and pick[1] <= TOL:
        p, _ = pick
        kept = nearest(ev, p)
        if kept and kept[1] <= TOL:
            d["pick_ms"] = round((p - t) * 1000, 1)
            d["kept_ms"] = round((kept[0] - t) * 1000, 1)
            return "min-gap (20 ms)", d
        d["pick_ms"] = round((p - t) * 1000, 1)
        if kept:
            d["moved_to_ms"] = round((kept[0] - p) * 1000, 1)
        return "refine", d
    if hit and hit[1] <= OFFSET_FAR:
        d["event_off_ms"] = round((hit[0] - t) * 1000, 1)
        return "offset", d
    near = [c for c in cands if abs(c["t"] - t) <= TOL]
    if near:
        gates = sorted({str(c["gate"]) for c in near})
        if any(c["kept"] for c in near):
            return "kept-not-picked", {"gates": gates}
        return "gated: " + "+".join(gates), {"gates": gates}
    nc = nearest(np.asarray([c["t"] for c in cands], float), t)
    if nc and prev_used is not None and t - prev_used <= 0.040 and nc[1] <= 0.030 \
            and abs(nc[0] - prev_used) <= TOL:
        d["cand_ms"] = round((nc[0] - t) * 1000, 1)
        return "peak merged", d
    if nc:
        d["nearest_cand_ms"] = round(nc[1] * 1000, 1)
    return "no candidate", d


def cmd_kick(ctx: le.Ctx) -> dict:
    base = le.load_json(ctx, "baseline.json")
    refs = le.load_json(ctx, "refs.json")
    kicks = [(s, m, stem, role) for s, m, stem, role in le.mic_jobs(list(le.SESSIONS)) if m == "k in"]
    jobs = []
    for s, m, stem, role in kicks:
        jid = le.job_id(s, m)
        jobs.append({"id": jid + "_cands_full", "wav": str(le.audio(ctx, s, stem)), "role": role,
                     "mode": "cands_full"})
        jobs.append({"id": jid + "_stages", "wav": str(le.audio(ctx, s, stem)), "role": role,
                     "mode": "stages", "values": STAGE_VALUES})
    rows = {r["id"]: r for r in le.rust(ctx, jobs)}

    res: dict = {}
    total_fn = 0
    total_named = 0
    cause_total: Counter = Counter()
    lines = []
    for s, m, _stem, _role in kicks:
        jid = le.job_id(s, m)
        b = base[jid]
        use = np.asarray(b["ref_used"], float)
        stages = {int(r["v"]): r for r in rows[jid + "_stages"]["stages"]}
        ev = np.asarray(sorted(stages[WORK_SENS]["events"]), float)
        picked = np.asarray(sorted(stages[WORK_SENS]["picked"]), float)
        cands = rows[jid + "_cands_full"]["candidates"]
        matched = {i for i, _ in le.match_pairs(use, ev)}
        fns = [i for i in range(len(use)) if i not in matched]
        causes: Counter = Counter()
        detail = []
        for i in fns:
            t = float(use[i])
            prev_used = float(use[i - 1]) if i > 0 else None
            cause, d = cause_of(t, prev_used, ev, picked, cands)
            causes[cause] += 1
            detail.append({"t": round(t, 3), "cause": cause, **d})
        named = sum(causes.values())
        total_fn += len(fns)
        total_named += named
        cause_total.update(causes)
        res[jid] = {
            "session": s, "mic": m, "n_ref": b["n_ref"], "n_used": len(use),
            "offset_ms": b["offset_ms"], "n_fn": len(fns), "causes": dict(causes),
            "fns": detail,
            "f1_at50": b["default"], "f1_at100": b["at100"], "oracle": b["oracle"],
        }
        lines.append(f"{s} {m}: {b['n_ref']} ref hits, {len(use)} used (offset "
                     f"{b['offset_ms']:+.1f} ms), FN at Sensitivity {WORK_SENS}: {len(fns)} of "
                     f"{len(use)} -> " + (", ".join(f"{k} {v}" for k, v in sorted(causes.items()))
                                            or "none"))
    lines.append("")
    lines.append("Cause table over the three sessions (kick, Sensitivity "
                 f"{WORK_SENS}):")
    for k, v in sorted(cause_total.items()):
        lines.append(f"  {k}: {v}")
    lines.append(f"  total FN {total_fn}, attributed to named causes {total_named} "
                 f"({100.0 * total_named / total_fn if total_fn else 100.0:.1f} %)")
    for s, m, _stem, _role in kicks:
        jid = le.job_id(s, m)
        b = base[jid]
        lines.append(f"  {s} kick F1: Sensitivity 50 {b['default']['f']:.3f}, Sensitivity 100 "
                     f"{b['at100']['f']:.3f}, oracle {b['oracle']['f']:.3f}")
    print("\n".join(lines))
    (ctx.cache / "kick_causes.json").write_text(json.dumps(
        {"sens": WORK_SENS, "tracks": res, "cause_total": dict(cause_total),
         "n_fn": total_fn, "n_named": total_named}, indent=1))
    return res


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("step", choices=["kick"])
    ap.add_argument("--samples", default=str(Path.home() / ".cache" / "drumspike" / "samples"))
    ap.add_argument("--cache", default=str(Path.home() / ".cache" / "drumspike" / "learn-eval"))
    ap.add_argument("--tool", default=None)
    a = ap.parse_args(argv)
    tool = a.tool or le.REPO / "target" / "release" / "service_tool"
    ctx = le.Ctx(Path(a.samples), Path(a.cache), Path(tool))
    ctx.cache.mkdir(parents=True, exist_ok=True)
    if a.step == "kick":
        cmd_kick(ctx)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
