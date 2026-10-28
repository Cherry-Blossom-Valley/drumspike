"""Main-thread spans of a traced self-test run (P3-15c, card Q4).

    uv run python apps/desktop/selftest/trace_spans.py TRACE.json REPORT.json [--json OUT.json]

TRACE.json is the Chrome trace of an app built with `--features trace` and started with
DRUMSPIKE_TRACE=TRACE.json; REPORT.json is the report of the scenario `lifecycle` of that run
(its phases carry their windows on the process clock, `selftest_clock`). The trace clock starts
with the event `trace_start`, whose `process_ms` is the offset.

Prints, per phase (open, save, close; round 0 is the first open of the run) the spans on the
main thread (top-level spans only: count, max, p95 and the longest ones), the command bodies on
other threads (`cmd`), the lock waits, and the threads of the async runtime. Only the Python
standard library is used.
"""
import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path


def load_trace(path):
    """The events of a Chrome trace file; a file without its closing bracket (the app was killed) is read too."""
    text = Path(path).read_text().strip()
    if not text.endswith("]"):
        text = text.rstrip(",\n ") + "]"
    return json.loads(text)


def spans_of(events):
    """(spans, offset_ms, thread names): spans as dicts {kind, name, thread, tid, start_ms, dur_ms, depth},
    times on the process clock."""
    names, offset = {}, None
    for e in events:
        if e.get("ph") == "M" and e.get("name") == "thread_name":
            names[e["tid"]] = e.get("args", {}).get("name", str(e["tid"]))
        if e.get("ph") == "i" and e.get("name") == "trace_start":
            offset = float(e.get("args", {}).get("process_ms", 0.0)) - e["ts"] / 1000.0
    if offset is None:
        raise SystemExit("no trace_start event: not a trace of a `--features trace` build")
    stacks, out = defaultdict(list), []
    for e in events:
        ph = e.get("ph")
        if ph == "B":
            stacks[e["tid"]].append(e)
        elif ph == "E" and stacks[e["tid"]]:
            b = stacks[e["tid"]].pop()
            args = b.get("args", {})
            out.append({
                "kind": b["name"],
                "name": args.get("name") or args.get("at") or "",
                "thread": names.get(b["tid"], str(b["tid"])),
                "tid": b["tid"],
                "start_ms": offset + b["ts"] / 1000.0,
                "dur_ms": (e["ts"] - b["ts"]) / 1000.0,
                "depth": len(stacks[e["tid"]]),
            })
    return out, offset, names


def calibrate(spans, phases):
    """Shift (ms) that puts the trace on the process clock exactly: each phase mark is the answer of a
    `selftest_clock` call, so it lies inside one `ipc selftest_clock` span on the main thread (a span of about
    0.1 ms). The shift is the distance from span middle to mark that most marks share (the error of `trace_start`,
    the time its own setup took); a mark is often near two clock calls, so the nearest one is not always its own."""
    mids = [s["start_ms"] + s["dur_ms"] / 2 for s in spans if s["kind"] == "ipc" and s["name"] == "selftest_clock"]
    marks = [m for ph in phases for m in (ph["start_ms"], ph["end_ms"])]
    cands = [[m - x for x in mids if -2.0 <= m - x <= 30.0] for m in marks]
    best, best_n = 0.0, 0
    for c in sorted({round(d, 2) for ds in cands for d in ds}):
        n = sum(1 for ds in cands if any(abs(d - c) <= 0.15 for d in ds))
        if n > best_n:
            best, best_n = c, n
    hits = sorted(min(ds, key=lambda d: abs(d - best)) for ds in cands if any(abs(d - best) <= 0.15 for d in ds))
    return hits[len(hits) // 2] if hits else 0.0


def pct(values, p):
    if not values:
        return None
    s = sorted(values)
    return s[min(len(s) - 1, int(p * len(s)))]


def r2(x):
    return None if x is None else round(x, 2)


def summarize(spans, phases, main="main"):
    """Per phase kind and round: the main-thread spans that start inside the window."""
    rows = []
    for ph in phases:
        a, b = ph["start_ms"], ph["end_ms"]
        inside = [s for s in spans if a <= s["start_ms"] <= b]
        top_main = [s for s in inside if s["thread"] == main and s["depth"] == 0]
        durs = [s["dur_ms"] for s in top_main]
        longest = sorted(top_main, key=lambda s: -s["dur_ms"])[:3]
        cmds = defaultdict(float)
        for s in inside:
            if s["kind"] == "cmd":
                cmds[s["name"]] = max(cmds[s["name"]], s["dur_ms"])
        waits = [s for s in inside if s["kind"] == "lock_wait"]
        rows.append({
            "phase": ph["phase"], "round": ph["round"], "window_ms": r2(b - a), "page_ms": ph.get("page_ms"),
            "main_n": len(durs), "main_max_ms": r2(max(durs) if durs else 0.0), "main_p95_ms": r2(pct(durs, 0.95) or 0.0),
            "main_longest": [f"{s['kind']} {s['name']} {s['dur_ms']:.2f}" for s in longest],
            "main_over_16ms": sum(1 for d in durs if d > 16.0),
            "cmd_max_ms": {k: r2(v) for k, v in sorted(cmds.items(), key=lambda kv: -kv[1])[:6]},
            "lock_waits": len(waits),
            "lock_wait_max_ms": r2(max((s["dur_ms"] for s in waits), default=0.0)),
            "lock_wait_main_max_ms": r2(max((s["dur_ms"] for s in waits if s["thread"] == main), default=0.0)),
        })
    return rows


def group(rows, kind, rounds):
    sel = [r for r in rows if r["phase"] == kind and r["round"] in rounds]
    if not sel:
        return None
    return {"phase": kind, "rounds": sorted(r["round"] for r in sel), "n": sum(r["main_n"] for r in sel),
            "max_ms": max(r["main_max_ms"] for r in sel), "p95_ms": max(r["main_p95_ms"] for r in sel),
            "over_16ms": sum(r["main_over_16ms"] for r in sel)}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("trace")
    ap.add_argument("report")
    ap.add_argument("--json", help="write the summary here")
    a = ap.parse_args(argv)
    spans, offset, names = spans_of(load_trace(a.trace))
    report = json.loads(Path(a.report).read_text())
    phases = report.get("metrics", {}).get("lifecycle") or []
    if not phases:
        raise SystemExit("the report has no metrics.lifecycle (scenario lifecycle)")
    shift = calibrate(spans, phases)
    for s_ in spans:
        s_["start_ms"] += shift
    rows = summarize(spans, phases)
    threads = sorted(set(names.values()))
    seek_window = report["metrics"].get("seek", {}).get("window")
    seek_cmd = [s["dur_ms"] for s in spans if s["kind"] == "cmd" and s["name"] == "transport_seek"]
    seek_ipc = [s["dur_ms"] for s in spans if s["kind"] == "ipc" and s["name"] == "transport_seek"]
    main_all = [s["dur_ms"] for s in spans if s["thread"] == "main" and s["depth"] == 0]
    out = {
        "offset_ms": r2(offset),
        "calibration_ms": r2(shift),
        "phases": rows,
        "first": [g for g in (group(rows, k, {0}) for k in ("open", "save", "close")) if g],
        "later": [g for g in (group(rows, k, set(range(1, 99))) for k in ("open", "save", "close")) if g],
        "main_all": {"n": len(main_all), "max_ms": r2(max(main_all, default=0.0)), "p95_ms": r2(pct(main_all, 0.95))},
        "seek_page": report["metrics"].get("seek"),
        "seek_cmd": {"n": len(seek_cmd), "p95_ms": r2(pct(seek_cmd, 0.95)), "max_ms": r2(max(seek_cmd, default=None) if seek_cmd else None)},
        "seek_ipc_main": {"n": len(seek_ipc), "p95_ms": r2(pct(seek_ipc, 0.95))},
        "seek_window_ms": seek_window,
        "async_threads": sum(1 for t in names.values() if t in ("tokio-runtime-worker", "tokio-rt-worker", "ds-async")),
        "threads": threads,
        "lock_waits": {
            "n": sum(1 for s in spans if s["kind"] == "lock_wait"),
            "main_n": sum(1 for s in spans if s["kind"] == "lock_wait" and s["thread"] == "main"),
            "max_ms": r2(max((s["dur_ms"] for s in spans if s["kind"] == "lock_wait"), default=0.0)),
            "top": [f"{s['thread']} {s['name']} {s['dur_ms']:.2f}" for s in sorted(
                (s for s in spans if s["kind"] == "lock_wait"), key=lambda s: -s["dur_ms"])[:5]],
        },
    }
    for r in rows:
        print(f"{r['phase']:5} #{r['round']}: window {r['window_ms']} ms, main spans {r['main_n']}, max {r['main_max_ms']} ms, "
              f"p95 {r['main_p95_ms']} ms, over 16 ms {r['main_over_16ms']}; longest {r['main_longest']}; "
              f"cmd {r['cmd_max_ms']}; lock waits {r['lock_waits']} (max {r['lock_wait_max_ms']} ms, main {r['lock_wait_main_max_ms']} ms)")
    for k in ("first", "later"):
        for g in out[k]:
            print(f"{k:5} {g['phase']:5}: n {g['n']}, max {g['max_ms']} ms, p95 {g['p95_ms']} ms, over 16 ms {g['over_16ms']}")
    print(f"seek: page {out['seek_page']}; cmd {out['seek_cmd']}; ipc on main {out['seek_ipc_main']}")
    print(f"lock waits: {out['lock_waits']}")
    print(f"calibration: {shift:.2f} ms added to the trace times (from the selftest_clock spans)")
    print(f"threads: {len(threads)} named in the trace, async runtime threads with spans: {out['async_threads']}")
    if a.json:
        Path(a.json).write_text(json.dumps(out, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
