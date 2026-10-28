"""P4-12 stage 1 and 2: the early-hit and late-hit rates on real sessions (the P4-11b verifier's estimator).

    uv run python tools/onset_early.py all [--samples ~/.cache/drumspike/sessions] [--out ~/.cache/drumspike/onset-reach/real]
    uv run python tools/onset_early.py detect       # the detectors once per track: the coarse times (detected.json)
    uv run python tools/onset_early.py measure      # refinement rules 1, 2 and 3 on the coarse times: early.json, early.md

A hit is early when its time lies 10 ms or more before the attack estimate: the first sample at 25 % of the maximum
`|x|` in [t - 2 ms, t + 40 ms], the window clipped at the midpoint to the next hit of the track. Hits are the events of
the app's analysis (`service_tool core-analyze`: the detector, the role filter, the dedup, the refinement with the
rule of the job, the second dedup), per detector (the DSP detector) and role. The detection runs
once; `measure` runs the steps after it with refinement rule 1 (the engine's, before P4-12), 2 (the reach rule) and 3
(P4-12 stage 2: the first-motion back-track) on the same coarse times. Only the three songs in `SESSIONS` are read
(other folders under `samples` are new sessions without references); they are read only; nothing goes into git.

The other direction (P4-12 fix round): a hit is late when it lies 3 ms or more after the clear attack of its hit: the
start of the run of the 0.5 ms envelope at or above 25 % of the hit's maximum (t .. t + 40 ms) that holds the hit, the
window clipped at the midpoints to the neighbouring hits. `moved_onto_body` counts the hits rule 2 moved 3 ms or more
later from a time that was not early, onto a late place.

Side effects between two rules (P4-12 stage 2): hits that moved by more than 1 ms, earlier and later, by more than the
back-track's span (10 ms), hits that crossed the previous hit's old time (a swap of order), new pairs closer than the
20 ms min gap, and a changed hit count (a doubled or merged hit). They are reported for rule 2 to 3 and rule 1 to 3.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
import soundfile as sf

ROOT = Path(__file__).resolve().parents[1]
TARGET = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
OUT = Path.home() / ".cache" / "drumspike" / "onset-reach" / "real"
SAMPLES = Path.home() / ".cache" / "drumspike" / "samples"
# the three songs with references (P4-6b); other folders under `samples` are not measured
SESSIONS = ("Session A", "Session B", "Session C")
RULES = (1, 2, 3)
SPAN_MS = 10.0         # the back-track's largest move (refine.rs FM_SPAN_S)
MIN_GAP_MS = 20.0      # the analysis' min gap: a pair closer than this is a doubled hit
EARLY_MS = 10.0
LATE_MS = 3.0
MOVE_MS = 1.0          # a later move by this much or more that ends late counts as moved onto the body
ENV_MS = 0.5
PRE_MS = 2.0
POST_MS = 40.0
# close mics by the owner's naming scheme: "k in", "s top", "t1" to "t4" (sample and trigger tracks are not mics)
ROLE_OF = [(re.compile(r"^k in_"), "kick"), (re.compile(r"^s top_"), "snare"), (re.compile(r"^t[1-4](?: (?!samp)[a-z]+)?_\d+\.wav$", re.I), "tom")]


def role_of(name):
    for pat, role in ROLE_OF:
        if pat.search(name):
            return role
    return None


def tracks(samples, sessions=SESSIONS):
    """[(session, file name, path, role)] of the close mics of the sessions under `samples` whose folder name is in
    `sessions` (`None`: every session folder)."""
    out = []
    for sess in sorted(p for p in Path(samples).iterdir() if (p / "Audio Files").is_dir()
                       and (sessions is None or p.name in sessions)):
        for f in sorted((sess / "Audio Files").glob("*.wav")):
            r = role_of(f.name)
            if r:
                out.append((sess.name, f.name, f, r))
    return out


def core_analyze(jobs):
    """`service_tool core-analyze` on a list of jobs; one answer per job."""
    tool = TARGET / "release" / "service_tool"
    if not tool.is_file():
        raise SystemExit(f"no {tool}: cargo build --release -p drumspike-devtools --bin service_tool")
    with tempfile.TemporaryDirectory() as td:
        jf = Path(td) / "jobs.json"
        jf.write_text(json.dumps(jobs))
        r = subprocess.run([str(tool), "core-analyze", str(jf)], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(r.stderr[-800:])
    out = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
    if not (isinstance(out, list) and len(out) == len(jobs)):
        raise SystemExit(f"core-analyze gave {len(out)} answers for {len(jobs)} jobs")
    return out


def detect(samples, out, log=print):
    """The coarse times per track (`detected.json`)."""
    rows = []
    for sess, name, path, role in tracks(samples):
        for det in ("dsp",):
            res = core_analyze([{"wav": str(path), "role": role, "params": {}}])[0]
            if "error" in res:
                raise SystemExit(f"{name} {det}: {res['error']}")
            rows.append({"session": sess, "name": name, "path": str(path), "role": role, "detector": det,
                         "detected": res["detected"]})
            log(f"{sess} / {name} [{det}]: {len(res['detected'])} detected")
    Path(out).mkdir(parents=True, exist_ok=True)
    (Path(out) / "detected.json").write_text(json.dumps(rows))
    return rows


def early_flags(x, sr, times):
    """Per hit (times ascending, seconds): the attack estimate minus the hit in ms, and whether it is early."""
    n = len(x)
    ax = np.abs(x)
    res = []
    for i, t in enumerate(times):
        s = int(t * sr)
        a = max(0, s - int(PRE_MS * sr / 1000))
        b = min(n, s + int(POST_MS * sr / 1000))
        if i + 1 < len(times):
            b = min(b, int((t + times[i + 1]) / 2 * sr))
        if b <= a:
            res.append((0.0, False))
            continue
        w = ax[a:b]
        m = w.max()
        if m <= 0:
            res.append((0.0, False))
            continue
        c = a + int(np.argmax(w >= 0.25 * m))
        d_ms = (c - s) * 1000.0 / sr
        res.append((d_ms, d_ms >= EARLY_MS))
    return res


def late_flags(x, sr, times):
    """Per hit (times ascending, seconds): the hit minus the start of its clear attack in ms, and whether it is late."""
    n = len(x)
    ax = np.abs(x)
    half = max(1, int(ENV_MS * sr / 2000))
    res = []
    for i, t in enumerate(times):
        s = int(t * sr)
        a = max(0, s - int(POST_MS * sr / 1000))
        b = min(n, s + int(POST_MS * sr / 1000))
        if i > 0:
            a = max(a, int((times[i - 1] + t) / 2 * sr))
        if i + 1 < len(times):
            b = min(b, int((t + times[i + 1]) / 2 * sr))
        if not a <= s < b:
            res.append((0.0, False))
            continue
        seg = np.pad(ax[a:b], half, mode="edge")
        env = np.lib.stride_tricks.sliding_window_view(seg, 2 * half + 1).max(axis=1)
        k = s - a
        m = env[k:].max()
        if m <= 0 or env[k] < 0.25 * m:
            res.append((0.0, False))
            continue
        j = k
        while j > 0 and env[j - 1] >= 0.25 * m:
            j -= 1
        d_ms = (k - j) * 1000.0 / sr
        res.append((d_ms, d_ms >= LATE_MS))
    return res


def read_mono(path):
    x, sr = sf.read(str(path), dtype="float32", always_2d=True)
    return x.mean(axis=1) if x.shape[1] > 1 else x[:, 0], sr


def measure(out, rules=RULES, log=print):
    """The rules on the cached coarse times: per detector, role and rule the hits and the early and late ones."""
    rows = [r for r in json.loads((Path(out) / "detected.json").read_text()) if r["session"] in SESSIONS]
    per = []
    times_of, flags_of = {}, {}
    for r in rows:
        jobs = [{"wav": r["path"], "role": r["role"], "params": {}, "detected": r["detected"], "rule": rule}
                for rule in rules]
        res = core_analyze(jobs)
        x, sr = read_mono(r["path"])
        for rule, a in zip(rules, res):
            if "error" in a:
                raise SystemExit(f"{r['name']} rule {rule}: {a['error']}")
            ts = [e["t"] for e in a["events"]]
            times_of[(r["session"], r["name"], r["detector"], rule)] = ts
            fl = early_flags(x, sr, ts)
            lf = late_flags(x, sr, ts)
            flags_of[(r["session"], r["name"], r["detector"], rule)] = ([f for _, f in fl], [f for _, f in lf])
            per.append({"session": r["session"], "name": r["name"], "role": r["role"], "detector": r["detector"],
                        "rule": rule, "hits": len(ts), "early": sum(f for _, f in fl), "late": sum(f for _, f in lf),
                        "branches": a["branches"], "early_ms": [round(d, 2) for d, f in fl if f],
                        "late_at": [round(t, 4) for t, (_, f) in zip(ts, lf) if f]})
        log(f"{r['session']} / {r['name']} [{r['detector']}]: " + ", ".join(
            f"rule {p['rule']} early {p['early']} late {p['late']} / {p['hits']}" for p in per[-len(rules):]))
    pairs = [(a, rules[-1]) for a in rules[-2::-1]] if len(rules) > 1 else []
    moves = {f"{a}->{b}": side_effects(rows, times_of, (a, b), flags_of) for a, b in pairs}
    summary = table(per, rules)
    (Path(out) / "early.json").write_text(json.dumps({"tracks": per, "side_effects": moves, "summary": summary}, indent=1))
    (Path(out) / "early.md").write_text(report(summary, moves, rules))
    return summary, moves


def side_effects(rows, times_of, rules, flags_of=None):
    """Rule 1 against rule 2 per detector: hits that moved by more than 1 ms, the largest move, the change of the hit
    count (doubled or merged hits), and the hits moved MOVE_MS or more later from a time that was not early onto a late
    place (`moved_onto_body`, with the times in `onto_body_at`)."""
    if len(rules) < 2:
        return {}
    out = {}
    for r in rows:
        a = times_of[(r["session"], r["name"], r["detector"], rules[0])]
        b = times_of[(r["session"], r["name"], r["detector"], rules[1])]
        o = out.setdefault(r["detector"], {"hits_before": 0, "hits_after": 0, "moved_1ms": 0, "max_move_ms": 0.0,
                                           "moved_later": 0, "moved_earlier": 0, "moved_onto_body": 0,
                                           "onto_body_at": [], "earlier_1ms": 0, "later_1ms": 0, "beyond_span": 0,
                                           "crossed": 0, "new_close": 0})
        o["hits_before"] += len(a)
        o["hits_after"] += len(b)
        if len(a) == len(b):
            d = (np.array(b) - np.array(a)) * 1000.0
            o["moved_1ms"] += int((np.abs(d) > 1.0).sum())
            o["moved_later"] += int((d > 0).sum())
            o["moved_earlier"] += int((d < 0).sum())
            o["earlier_1ms"] += int((d < -1.0).sum())
            o["later_1ms"] += int((d > 1.0).sum())
            o["beyond_span"] += int((np.abs(d) > SPAN_MS).sum())
            o["crossed"] += sum(1 for i in range(1, len(b)) if b[i] <= a[i - 1] or b[i] <= b[i - 1])
            o["new_close"] += sum(1 for i in range(1, len(b))
                                  if (b[i] - b[i - 1]) * 1000.0 < MIN_GAP_MS <= (a[i] - a[i - 1]) * 1000.0)
            if len(d):
                o["max_move_ms"] = max(o["max_move_ms"], float(np.abs(d).max()))
            if flags_of:
                (early_a, _), (_, late_b) = (flags_of[(r["session"], r["name"], r["detector"], q)] for q in rules[:2])
                for i in np.flatnonzero(d >= MOVE_MS):
                    if not early_a[i] and late_b[i]:
                        o["moved_onto_body"] += 1
                        o["onto_body_at"].append(f"{r['session'][:12]} / {r['name']} {a[i]:.3f}")
        else:
            o.setdefault("count_changed", []).append(f"{r['session']} / {r['name']}: {len(a)} -> {len(b)}")
    return out


def table(per, rules):
    out = []
    for det in ("dsp",):
        for role in ("kick", "snare", "tom", "all"):
            row = {"detector": det, "role": role}
            for rule in rules:
                sel = [p for p in per if p["detector"] == det and p["rule"] == rule and role in (p["role"], "all")]
                h, e = sum(p["hits"] for p in sel), sum(p["early"] for p in sel)
                lt = sum(p.get("late", 0) for p in sel)
                row[f"hits_{rule}"], row[f"early_{rule}"], row[f"late_{rule}"] = h, e, lt
                row[f"rate_{rule}"] = 100.0 * e / h if h else 0.0
                row[f"late_rate_{rule}"] = 100.0 * lt / h if h else 0.0
            out.append(row)
    return out


def markdown(summary, moves, rules):
    """The rules table, then (with `moves`) the side-effects table of one pair of rules."""
    return rules_table(summary, rules) + ("\n" + moves_table(moves) if moves else "")


def report(summary, moves, rules):
    """The rules table, then the side effects of each pair of rules (`moves` keyed by "a->b")."""
    return rules_table(summary, rules) + "".join(f"\nRule {label}:\n\n" + moves_table(m) for label, m in moves.items())


def rules_table(summary, rules):
    head = "| detector | role | " + " | ".join(f"rule {r}: early / late / hits (early %, late %)" for r in rules) + " |"
    lines = [head, "|" + "---|" * (2 + len(rules))]
    for row in summary:
        cells = [f"{row[f'early_{r}']} / {row.get(f'late_{r}', 0)} / {row[f'hits_{r}']} ({row[f'rate_{r}']:.2f} %, "
                 f"{row.get(f'late_rate_{r}', 0.0):.2f} %)" for r in rules]
        lines.append(f"| {row['detector']} | {row['role']} | " + " | ".join(cells) + " |")
    return "\n".join(lines) + "\n"


def moves_table(moves):
    lines = []
    if moves:
        lines += ["| detector | hits before / after | moved > 1 ms | later / earlier | largest move ms | moved onto the body |"
                  " earlier / later > 1 ms | beyond 10 ms | crossed | new pairs < 20 ms |",
                  "|---|---|---|---|---|---|---|---|---|---|"]
        for det, o in moves.items():
            lines.append(f"| {det} | {o['hits_before']} / {o['hits_after']} | {o['moved_1ms']} | {o['moved_later']} / "
                         f"{o['moved_earlier']} | {o['max_move_ms']:.2f} | {o.get('moved_onto_body', 0)} | "
                         f"{o.get('earlier_1ms', 0)} / {o.get('later_1ms', 0)} | {o.get('beyond_span', 0)} | "
                         f"{o.get('crossed', 0)} | {o.get('new_close', 0)} |")
            for c in o.get("count_changed", []):
                lines.append(f"| {det} | count changed: {c} | | | | |")
            for c in o.get("onto_body_at", []):
                lines.append(f"| {det} | onto the body: {c} | | | | |")
    return "\n".join(lines) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["detect", "measure", "all"])
    ap.add_argument("--samples", default=str(SAMPLES))
    ap.add_argument("--out", default=str(OUT))
    a = ap.parse_args(argv)
    if a.cmd in ("detect", "all"):
        detect(a.samples, a.out)
    if a.cmd in ("measure", "all"):
        summary, moves = measure(a.out)
        print(report(summary, moves, RULES))
    return 0


if __name__ == "__main__":
    sys.exit(main())
