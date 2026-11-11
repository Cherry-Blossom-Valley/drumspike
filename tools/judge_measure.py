"""#385: measure the judge of static/agent.js on this host's internal data (Session K and the gold rows of Sessions A to J).

    uv run python tools/judge_measure.py MEASURE_DIR [--ref origin/main]

MEASURE_DIR holds `v10/fit_v10.csv` + `fit_v10.f32` (the labelled hits) and `session-k/` (the project and the stems). Without
--ref the working tree's static/agent.js is measured; with it, the file at that git ref (the "before" run). The output is
aggregates only: per gold set the share exact, within one sample and the summed distance of the judge's move to the label
(no move counts as 0), and for Session K the moves and how many shape keys got more than one answer. The gold rows are
`confirmed = 1 OR label_off != 0 OR owner = 1`. With --rows FILE the per-row distances are written next to it, to compare
two runs row by row (`--compare FILE` prints how many gold rows are worse, equal, better than that earlier run).

--inject is the fault-injection run (#421): every row with a placed truth (`confirmed = 1 OR owner = 1`) is judged with its
hit shifted 2, 3, 5 and 8 samples late of the label. Per shift it prints how many the judge flagged as late (and how many
suggestions land on the label, or one off), listed without an answer, left with a reason, and left without one
("unexplained": the misses). It needs only `v10/`. A run that injected 0 hits FAILs.

#447: the judge has a second opinion, the pattern of the track (built from the track's own clear hits). The tool builds one
pattern per track from the labelled rows of that track, at the positions the tool gave the hits, and hands every window the
pattern of its track, as the page does. `second` in the output counts what the second opinion is for: per gold set the
wrong moves, the hits 2 or more samples off that were left without a word, and the listed hits; for the injected hits the
moves the pattern took back. `pattern` counts the tracks and the patterns built; a run whose agent file has the pattern and
builds none FAILs. --at-current judges every labelled hit at the position of the current placement rule (the `cur_off`
column) instead of the position at labelling time: the case of a fine-tune run on a fresh analysis. A file at --ref without
the pattern measures the judge alone.
"""
import argparse
import csv
import json
import subprocess
import sys
import wave
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
BEFORE_MS, AFTER_MS = 0.9, 0.6   # the page's sample-zoom window (static/agent.js)
STROKE_AFTER_MS = 5.0            # #394: the detector's own peak span after the hit (edge.rs EDGE_SPAN_S); the stroke-peak reference


def cut(data, hit, sr, after_ms=AFTER_MS):
    """The window the page cuts around `hit`: (samples, index of the hit in it)."""
    a, b = int(np.floor(hit - BEFORE_MS * sr / 1000)), int(np.ceil(hit + after_ms * sr / 1000))
    return data[max(a, 0):b].tolist(), hit - max(a, 0)


def stroke_span(data, hit, sr):
    """The detector's span around `hit` (samples only): its tail from the hit holds the hit's peak (#394)."""
    return cut(data, hit, sr, STROKE_AFTER_MS)[0]


PATTERN_LINE_MS, PATTERN_PEAK_MS = 2.0, 5.0   # the samples the pattern is built from: static/agent.js PAT_LINE_MS, PAT_PEAK_MS


def pattern_row(data, hit, sr):
    """The samples of one hit for the pattern of its track: 2 ms before to 5 ms after, and the hit's index in them."""
    a, b = hit - round(PATTERN_LINE_MS * sr / 1000), hit + round(PATTERN_PEAK_MS * sr / 1000)
    return {"x": data[max(a, 0):b].tolist(), "at": hit - max(a, 0)}


def is_gold(r):
    return r["confirmed"] == "1" or int(r["label_off"]) != 0 or r["owner"] == "1"


def base_of(r, at_current):
    """The hit's index in its 960-sample row: 480 is the app's placement at labelling time; with `at_current` the
    position of the current rule (None for a row the current rule has no hit for)."""
    if not at_current:
        return 480
    return 480 + int(r["cur_off"]) if r["cur_off"] != "" else None


def gold_patterns(measure, at_current=False):
    """#447: per track (session and role) the rows its pattern is built from: every gold row at the tool's position."""
    rows = np.fromfile(measure / "v10" / "fit_v10.f32", dtype="<f4").reshape(-1, 960)
    out = {}
    for r in csv.DictReader(open(measure / "v10" / "fit_v10.csv")):
        base = base_of(r, at_current)
        if not is_gold(r) or base is None:
            continue
        t = out.setdefault(f"{r['session']}:{r['role']}", {"sr": 48000, "rows": []})
        t["rows"].append(pattern_row(rows[int(r["i"])].astype(float), base, 48000))
    return out


def gold_windows(measure, at_current=False):
    rows = np.fromfile(measure / "v10" / "fit_v10.f32", dtype="<f4").reshape(-1, 960)
    out = []
    for r in csv.DictReader(open(measure / "v10" / "fit_v10.csv")):
        base = base_of(r, at_current)
        if not is_gold(r) or base is None:
            continue
        i = int(r["i"])
        s, at = cut(rows[i].astype(float), base, 48000)
        out.append({"set": "gold" + r["session"], "id": i, "role": r["role"], "label_off": 480 + int(r["label_off"]) - base,
                    "owner": r["owner"] == "1", "samples": s, "stroke": stroke_span(rows[i].astype(float), base, 48000),
                    "hit_at": at, "track": f"{r['session']}:{r['role']}"})
    return out


INJECT_SHIFTS = (2, 3, 5, 8)


def inject_windows(measure):
    """#421: the rows with a placed truth, the hit moved `shift` samples late of the label."""
    rows = np.fromfile(measure / "v10" / "fit_v10.f32", dtype="<f4").reshape(-1, 960)
    out = []
    for r in csv.DictReader(open(measure / "v10" / "fit_v10.csv")):
        if not (r["confirmed"] == "1" or r["owner"] == "1"):
            continue
        d, true = rows[int(r["i"])].astype(float), 480 + int(r["label_off"])
        for shift in INJECT_SHIFTS:
            s, at = cut(d, true + shift, 48000)
            out.append({"set": "inject", "id": int(r["i"]), "shift": shift, "role": r["role"], "samples": s,
                        "stroke": stroke_span(d, true + shift, 48000), "hit_at": at, "track": f"{r['session']}:{r['role']}"})
    return out


def session_k_windows(measure, patterns=None):
    """The hits of Session K. `patterns` (a dict) gets the pattern rows of each track: every hit of the project."""
    k = measure / "session-k"
    proj = json.load(open(next(k.glob("*/*.drumspike"))))
    out = []
    for track, stem in (("01_Kick", "01 Kick.wav"), ("02_Snr", "02 Snr.wav"), ("04_Toms", "04 Toms.wav")):
        w = wave.open(str(k / stem))
        sr = w.getframerate()
        data = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(float) / 32768
        for n, e in enumerate(proj["tracks"][track]["events"]):
            s, at = cut(data, round(e["t"] * sr), sr)
            out.append({"set": "K", "id": f"{track}:{n}", "role": proj["tracks"][track]["role"], "samples": s,
                        "stroke": stroke_span(data, round(e["t"] * sr), sr), "hit_at": at, "track": f"K:{track}"})
            if patterns is not None:
                patterns.setdefault(f"K:{track}", {"sr": sr, "rows": []})["rows"].append(pattern_row(data, round(e["t"] * sr), sr))
    return out


def session_k_records(measure):
    """#408: the Session K kick hits the rule-A check names, from the Training Mode records: the 5 hand-placed hits (a move
    record without precision, judged at its `to`) and the 19 confirmed hits (a confirm record). Sets "hand" and "confirmed",
    label 0: exact = the judge leaves the hit alone."""
    k = measure / "session-k"
    w = wave.open(str(k / "01 Kick.wav"))
    sr = w.getframerate()
    data = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(float) / 32768
    out = []
    for n, line in enumerate(open(next((k / "training").glob("*.jsonl")))):
        r = json.loads(line)
        if r["track"] != "01_Kick":
            continue
        name = "confirmed" if r["op"] == "confirm" else "hand" if r["op"] == "move" and r.get("precision") is None else None
        if name is None:
            continue
        s, at = cut(data, r["to"], sr)
        out.append({"set": name, "id": n, "role": "kick", "label_off": 0, "samples": s,
                    "stroke": stroke_span(data, r["to"], sr), "hit_at": at, "track": "K:01_Kick"})
    return out


def measure(agent_src, windows, pattern_rows=None):
    """The aggregates of the judge in `agent_src` over `windows`; `pattern_rows` are the rows each track's pattern is built from."""
    r = subprocess.run(["node", str(ROOT / "tools" / "judge_measure.js")],
                       input=json.dumps({"agent": agent_src, "windows": windows, "pattern_rows": pattern_rows or {}}),
                       capture_output=True, text=True, check=True)
    return json.loads(r.stdout)


def check_patterns(pattern):
    """#447: an agent file with the pattern that built none measured nothing of it."""
    if pattern["supported"] and pattern["built"] == 0:
        sys.exit(f"FAIL: the run built 0 patterns from {pattern['tracks']} tracks")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("measure_dir")
    ap.add_argument("--ref", help="measure static/agent.js at this git ref instead of the working tree")
    ap.add_argument("--rows", help="write the per-row gold distances here")
    ap.add_argument("--compare", help="per-row file of an earlier run: count the gold rows worse, equal and better")
    ap.add_argument("--inject", action="store_true", help="the fault-injection run: labelled hits shifted late (needs only v10/)")
    ap.add_argument("--at-current", action="store_true", help="judge the labelled hits at the current rule's position (cur_off), not at the labelling position")
    a = ap.parse_args()
    measure_dir = Path(a.measure_dir)
    src = (subprocess.run(["git", "show", f"{a.ref}:static/agent.js"], capture_output=True, text=True, check=True, cwd=ROOT).stdout
           if a.ref else (ROOT / "static" / "agent.js").read_text())
    if a.inject:
        res = measure(src, inject_windows(measure_dir), gold_patterns(measure_dir, a.at_current))
        by_shift = res["inject"]
        total = {k: sum(v[k] for v in by_shift.values()) for k in ("n", "flagged", "fixed", "within1", "listed", "explained", "unexplained")}
        json.dump({"inject_total": total, "inject_by_shift": by_shift, "second": res["second"]["inject"], "pattern": res["pattern"]}, sys.stdout, indent=1)
        print()
        if total["n"] == 0:
            sys.exit("FAIL: the run injected 0 hits")
        check_patterns(res["pattern"])
        return
    patterns = gold_patterns(measure_dir, a.at_current)
    res = measure(src, gold_windows(measure_dir, a.at_current) + session_k_windows(measure_dir, patterns) + session_k_records(measure_dir), patterns)
    dist = {f"{r['set']}:{r['id']}": r["d"] for r in res["rows"] if r["set"].startswith("gold")}
    gold = res["gold"]
    sets = [g for name, g in gold.items() if name.startswith("gold")]   # "owner" is a subset of Session E; "hand" and "confirmed" are Session K
    tot = {"n": sum(g["n"] for g in sets), "exact": sum(g["exact"] for g in sets),
           "within1": sum(g["within1"] for g in sets), "dist": sum(g["dist"] for g in sets)}
    second = res["second"]["gold"]
    sec_sets = [g for name, g in second.items() if name.startswith("gold")]
    out = {"gold_total": tot, "gold": gold, "session_k": res["k"], "pattern": res["pattern"],
           "second_total": {k: sum(g[k] for g in sec_sets) for k in ("n", "wrong", "silent", "listed")}, "second": second}
    if a.rows:
        Path(a.rows).write_text(json.dumps(dist))
    if a.compare:
        before = json.load(open(a.compare))
        out["gold_rows_vs_compare"] = {"worse": sum(dist[k] > before[k] for k in dist), "equal": sum(dist[k] == before[k] for k in dist),
                                       "better": sum(dist[k] < before[k] for k in dist)}
    json.dump(out, sys.stdout, indent=1)
    print()
    check_patterns(res["pattern"])


if __name__ == "__main__":
    main()
