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


def gold_windows(measure):
    rows = np.fromfile(measure / "v10" / "fit_v10.f32", dtype="<f4").reshape(-1, 960)
    out = []
    for r in csv.DictReader(open(measure / "v10" / "fit_v10.csv")):
        if not (r["confirmed"] == "1" or int(r["label_off"]) != 0 or r["owner"] == "1"):
            continue
        i = int(r["i"])
        s, at = cut(rows[i].astype(float), 480, 48000)   # index 480 is the app's placement at labelling time
        out.append({"set": "gold" + r["session"], "id": i, "role": r["role"], "label_off": int(r["label_off"]),
                    "owner": r["owner"] == "1", "samples": s, "stroke": stroke_span(rows[i].astype(float), 480, 48000),
                    "hit_at": at})
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
                        "stroke": stroke_span(d, true + shift, 48000), "hit_at": at})
    return out


def session_k_windows(measure):
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
                        "stroke": stroke_span(data, round(e["t"] * sr), sr), "hit_at": at})
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
                    "stroke": stroke_span(data, r["to"], sr), "hit_at": at})
    return out


def measure(agent_src, windows):
    """The aggregates of the judge in `agent_src` over `windows`."""
    r = subprocess.run(["node", str(ROOT / "tools" / "judge_measure.js")], input=json.dumps({"agent": agent_src, "windows": windows}),
                       capture_output=True, text=True, check=True)
    return json.loads(r.stdout)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("measure_dir")
    ap.add_argument("--ref", help="measure static/agent.js at this git ref instead of the working tree")
    ap.add_argument("--rows", help="write the per-row gold distances here")
    ap.add_argument("--compare", help="per-row file of an earlier run: count the gold rows worse, equal and better")
    ap.add_argument("--inject", action="store_true", help="the fault-injection run: labelled hits shifted late (needs only v10/)")
    a = ap.parse_args()
    measure_dir = Path(a.measure_dir)
    src = (subprocess.run(["git", "show", f"{a.ref}:static/agent.js"], capture_output=True, text=True, check=True, cwd=ROOT).stdout
           if a.ref else (ROOT / "static" / "agent.js").read_text())
    if a.inject:
        by_shift = measure(src, inject_windows(measure_dir))["inject"]
        total = {k: sum(v[k] for v in by_shift.values()) for k in ("n", "flagged", "fixed", "within1", "listed", "explained", "unexplained")}
        json.dump({"inject_total": total, "inject_by_shift": by_shift}, sys.stdout, indent=1)
        print()
        if total["n"] == 0:
            sys.exit("FAIL: the run injected 0 hits")
        return
    res = measure(src, gold_windows(measure_dir) + session_k_windows(measure_dir) + session_k_records(measure_dir))
    dist = {f"{r['set']}:{r['id']}": r["d"] for r in res["rows"] if r["set"].startswith("gold")}
    gold = res["gold"]
    sets = [g for name, g in gold.items() if name.startswith("gold")]   # "owner" is a subset of Session E; "hand" and "confirmed" are Session K
    tot = {"n": sum(g["n"] for g in sets), "exact": sum(g["exact"] for g in sets),
           "within1": sum(g["within1"] for g in sets), "dist": sum(g["dist"] for g in sets)}
    out = {"gold_total": tot, "gold": gold, "session_k": res["k"]}
    if a.rows:
        Path(a.rows).write_text(json.dumps(dist))
    if a.compare:
        before = json.load(open(a.compare))
        out["gold_rows_vs_compare"] = {"worse": sum(dist[k] > before[k] for k in dist), "equal": sum(dist[k] == before[k] for k in dist),
                                       "better": sum(dist[k] < before[k] for k in dist)}
    json.dump(out, sys.stdout, indent=1)
    print()


if __name__ == "__main__":
    main()
