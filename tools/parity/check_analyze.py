"""P3-2a: the analysis pipeline in the core against the frozen engine.analyze_track in the Rust mode.

    cargo build --release -p drumspike-devtools
    uv run --project . python tools/parity/check_analyze.py <corpus_dir> \\
        [--samples <dir> [--layout named|flat] [--song1 <dir>]] [--detectors dsp] [--json out.json] [--bin service_tool]

<corpus_dir> is the output of make_corpus.py (every file, role kick). With --samples the
real tracks of tests/golden/detect run too (29 on arm64-mac from the Pro Tools folders, 38 on
x86-mac with --layout flat --song1; the role comes from the file token: k in = kick,
s top = snare, t1 to t4 = tom, hh = hat, oh = cymbal, song1 = other). Read only.

New: the core's analysis (`service_tool core-analyze`: the decode, detect_events and
analyze_detected of the app's job, detection and every step after it in Rust). Reference:
engine.analyze_track of the retired server in its Rust mode, that is engine._rust_detect plus
the Python steps engine._events_after_detection with the Rust refinement and velocities,
frozen in tests/golden/frozen/analyze.json (P3-12; the inputs are the source SHA-256, role,
params, thresholds, detector and the model SHA-256 slot of the record, `None` on main). Per file
and detector (the DSP detector) four cases:

  A  {} with the file's role, both sides from the signal (detection included);
  B  {"refine": False}, role other (no filter);
  C  shift +37 samples, min gap 45 ms, refine window -7..+7 ms, velocities -50..-3 dB;
  D  thresholds 0.3 each, shift -480 samples, the unknown role "bass" (filter with the set
     of other, DSP with the parameters of other), both sides from the signal.

B and C start from the reference detection of A (the frozen (t, note) pairs); case A also
checks that the core detects those pairs. Events must be identical: t, v, rise and order
(json.dumps of both lists). The branch counts of the core (filter, both dedups, refine off,
refinement, shift) are summed over both detectors (a DSP hit carries the role's note, so the
filter never removes one there); the exit status is 1 when an event differs or a branch
count stays 0.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(ROOT / "tests"))
import frozen  # noqa: E402
from check_detect import local_path  # noqa: E402

STORE = frozen.store("analyze")
DEFAULT_BIN = ROOT / "target" / "release" / "service_tool"
THRESHOLDS = (0.22, 0.24, 0.32, 0.22, 0.30)
_SHA = {}


def file_sha(path):
    key = str(path)
    if key not in _SHA:
        import hashlib
        h = hashlib.sha256()
        with open(path, "rb") as f:
            for block in iter(lambda: f.read(1 << 20), b""):
                h.update(block)
        _SHA[key] = h.hexdigest()
    return _SHA[key]


def frozen_events(name, path, d, case, role, params, thresholds, model):
    """{events[, detected]} of the retired server for one file, detector and case."""
    inputs = {"source_sha256": file_sha(path), "role": role, "params": params, "thresholds": list(thresholds),
              "detector": d, "model_sha256": file_sha(model) if d == "model" else None}
    return STORE.full_value(STORE.ref(f"{name}|{d}|{case}", inputs))


def core_analyze(bin_path, jobs, scratch):
    """`service_tool core-analyze`: one answer {events, branches, detected, detector, sr} or {error} per job."""
    jf = scratch / "analyze_jobs.json"
    jf.write_text(json.dumps(jobs))
    r = subprocess.run([str(bin_path), "core-analyze", str(jf)], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"service_tool core-analyze failed: {r.stderr[-600:]}")
    out = [json.loads(line) for line in r.stdout.splitlines() if line.strip()]
    assert len(out) == len(jobs) and all(isinstance(o, dict) for o in out), (len(out), len(jobs))
    return out

ROLE_OF_TOKEN = {"k in": "kick", "s top": "snare", "t1": "tom", "t2": "tom", "t3": "tom", "t4": "tom",
                 "hh": "hat", "oh": "cymbal"}
CASES = {
    "A": (None, {}),
    "B": ("other", {"refine": False}),
    "C": (None, {"shift_samples": 37, "min_gap_ms": 45, "refine_lo_ms": -7, "refine_hi_ms": 7,
                 "vel_lo_db": -50, "vel_hi_db": -3}),
    "D": ("bass", {"thresholds": [0.3] * 5, "shift_samples": -480}),
}
MUST_HIT = ("filter_runs", "filtered_out", "dedup_dropped", "dedup2_dropped", "refine_off", "refine_moved",
            "shifted")


def tracks(a):
    out = [(f"corpus/{f['name']}", Path(a.corpus) / f["name"], "kick")
           for f in json.loads((Path(a.corpus) / "manifest.json").read_text())["files"]]
    if a.samples:
        for p in sorted((ROOT / "tests" / "golden" / "detect").glob("*.json")):
            g = json.loads(p.read_text())
            if not isinstance(g, dict) or g.get("set") != "real":
                continue
            if g["session"] == "song1" and not a.song1:
                continue
            out.append((g["stem"], local_path(a.samples, g, a.layout, a.song1), ROLE_OF_TOKEN.get(g["token"], "other")))
    return out


def first_diff(a, b):
    for i, (x, y) in enumerate(zip(a, b)):
        if json.dumps(x) != json.dumps(y):
            return {"index": i, "core": x, "engine": y}
    return {"index": min(len(a), len(b)), "core_len": len(a), "engine_len": len(b)}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("corpus")
    ap.add_argument("--samples")
    ap.add_argument("--layout", choices=["named", "flat"], default="named")
    ap.add_argument("--song1")
    ap.add_argument("--detectors", default="dsp")
    ap.add_argument("--bin", type=Path, default=DEFAULT_BIN)
    ap.add_argument("--json")
    a = ap.parse_args()
    if not a.bin.exists():
        print(f"service_tool missing: {a.bin} (cargo build --release -p drumspike-devtools)")
        return 2
    dets = a.detectors.split(",")
    model = "none"   # the model SHA slot of the frozen reference inputs; there is no model file on main
    total = {d: {} for d in dets}
    rows, bad = [], 0
    files = tracks(a)
    print(f"{len(files)} files ({sum(1 for f in files if not f[0].startswith('corpus/'))} real), "
          f"detectors {', '.join(dets)}, model {model}", flush=True)
    with tempfile.TemporaryDirectory(prefix="ds_analyze_") as td:
        scratch = Path(td)
        for name, path, role in files:
            for d in dets:
                t0 = time.time()
                res = {"file": name, "detector": d, "role": role, "cases": {}}
                jobs, refs = [], []
                ref_a = None
                for case, (crole, params) in CASES.items():
                    r = crole or role
                    thresholds = tuple(params.get("thresholds", THRESHOLDS))
                    ref = frozen_events(name, path, d, case, r, params, thresholds, model)
                    job = {"wav": str(path), "mix": "ffmpeg", "role": r, "params": params,
                           "model": model if d == "model" else None}
                    if case == "A":
                        ref_a = ref["detected"]
                    elif case in ("B", "C"):
                        job["detected"] = ref_a
                    jobs.append(job)
                    refs.append(ref)
                for case, ref, out in zip(CASES, refs, core_analyze(a.bin, jobs, scratch)):
                    if "error" in out:
                        c = {"events": len(ref["events"]), "ok": False, "branches": {}, "diff": {"error": out["error"]}}
                    else:
                        core, br = out["events"], out["branches"]
                        ok = json.dumps(core) == json.dumps(ref["events"])
                        if case == "A" and out["detected"] != ref["detected"]:
                            ok = False
                        if case in ("A", "D") and out["detector"] != "rust-dsp":
                            ok = False
                        c = {"events": len(ref["events"]), "ok": ok, "branches": br}
                        if not ok:
                            c["diff"] = first_diff(core, ref["events"])
                        for k, v in br.items():
                            total[d][k] = total[d].get(k, 0) + v
                    bad += not c["ok"]
                    res["cases"][case] = c
                res["seconds"] = round(time.time() - t0, 2)
                rows.append(res)
                n = sum(c["events"] for c in res["cases"].values())
                status = "ok" if all(c["ok"] for c in res["cases"].values()) else "FAIL"
                print(f"{status} {name} [{d}, {role}]: {n} events in 4 cases, {res['seconds']} s"
                      + ("" if status == "ok" else f" {json.dumps({k: c.get('diff') for k, c in res['cases'].items() if not c['ok']})}"),
                      flush=True)
    summary = {}
    for d in dets:
        dr = [r for r in rows if r["detector"] == d]
        summary[d] = {"files": len(dr), "files_ok": sum(all(c["ok"] for c in r["cases"].values()) for r in dr),
                      "events": sum(c["events"] for r in dr for c in r["cases"].values()),
                      "events_case_A": sum(r["cases"]["A"]["events"] for r in dr),
                      "branches": total[d]}
        print(f"{d}: {summary[d]['files_ok']}/{summary[d]['files']} files identical, {summary[d]['events']} events "
              f"({summary[d]['events_case_A']} in case A); branches {json.dumps(total[d])}")
    # a DSP hit carries the role's note, so the filter never removes one there: the branches
    # count over both detectors together
    both = {k: sum(total[d].get(k, 0) for d in dets) for k in MUST_HIT}
    missing = [k for k in MUST_HIT if both[k] == 0]
    summary["branches_not_hit"] = missing
    bad += len(missing)
    print(f"branches over all detectors: {json.dumps(both)}" + (f"; NOT HIT: {missing}" if missing else ""))
    if a.json:
        Path(a.json).write_text(json.dumps({"summary": summary, "rows": rows}, indent=1))
    print("PASS" if bad == 0 else f"FAIL ({bad})")
    return 0 if bad == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
