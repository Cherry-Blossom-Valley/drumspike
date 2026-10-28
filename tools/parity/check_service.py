"""P3-3a: the service commands (crates/app) against the frozen Python engine and pretty_midi.

    cargo build --release -p drumspike-devtools
    uv run --project . python tools/parity/check_service.py <corpus_dir> \\
        [--samples <dir> [--layout named|flat] [--song1 <dir>]] [--detectors dsp] [--json out.json] \\
        [--record-retry]

<corpus_dir> is the output of tools/parity/make_corpus.py. Two checks, both read only
on the sources:

analyze_start. For every file (the corpus; with --samples the real tracks of tests/golden/detect,
as check_analyze.py) and the DSP detector a
project folder holds four tracks that point to the file: the cases A to D of check_analyze.py
(role and params stored in the document). `service_tool analyze` opens the folder, runs
Service::analyze_start on all four tracks and prints the events of events_get. The reference is
engine._rust_analyze_track of the retired server on the same decode, frozen in
tests/golden/frozen/service_analyze.json (P3-12). Rule: identical json.dumps of both event lists
(t, v, rise, order).

The P3-18 retry (2026-10-12, after the freeze) picks again with lower thresholds when the model
gives no hit; the retired server had no retry. A job entry with a `weak_signal` step (`to` set) is
checked in two parts: the frozen server reference must be empty (the first pick had no hit, which is
what starts the retry), and the job's step and events must equal the retry store
tests/golden/frozen/service_retry.json. That store is recorded from the Rust service
(`--record-retry`), not from the server; its `note` says why (check_service case D, 2026-10-13).

export_midi. For every event list of check_midi.py (the refined hits of each corpus file and the
synthetic lists) the reference is the file engine._py_export_midi (pretty_midi) wrote, frozen in
tests/golden/frozen/midi.json, and Service::export_midi writes Exports/<track>_spike.mid. Rule:
identical bytes.

Exit status 1 when a list differs.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "parity"))
sys.path.insert(0, str(ROOT / "tests"))
sys.path.insert(0, str(ROOT / "apps" / "desktop" / "selftest"))
import frozen  # noqa: E402
from check_analyze import CASES, THRESHOLDS, file_sha, tracks  # noqa: E402  (the files and the four cases)
from check_midi import corpus_events, midi_cases, midi_reference  # noqa: E402
from make_project import build_project  # noqa: E402

STORE = frozen.store("service_analyze")
RETRY = frozen.store("service_retry", "check_service.py: per file, detector and case the P3-18 retry of "
                     "a track without hits (the job's weak_signal step and the events); recorded from the Rust "
                     "service with --record-retry, because the frozen server reference has no retry")

DEFAULT_BIN = ROOT / "target" / "release" / "service_tool"


def make_project(base, name, src, role, case_params):
    """A project with the four case tracks A to D on one source file."""
    return build_project(base, name, [(case, src, crole or role, 36, [], {"params": params})
                                      for case, (crole, params) in case_params.items()], mix_mode="ffmpeg")


def run_tool(bin_path, args):
    env = dict(os.environ)
    env["DRUMSPIKE_REFINE_RULE"] = "1"    # P4-12: the frozen references are the engine's refinement (version 1)
    env["DRUMSPIKE_ROLL_RULE"] = "0"     # P4-19: and the engine had no roll fill
    env["DRUMSPIKE_FLAM_RULE"] = "0"     # P4-13 stage B: nor the flam rule
    env["DRUMSPIKE_KICK_RESCUE"] = "0"   # P4-10 stage 5: nor the kick rescue
    env["DRUMSPIKE_SNARE_RESCUE"] = "0"  # P4-10 stage 7: nor the snare rescue
    r = subprocess.run([str(bin_path), *args], capture_output=True, text=True, env=env)
    if r.returncode != 0:
        raise SystemExit(f"service_tool failed: {r.stderr[-600:]}")
    return r.stdout


def retry_ref(key, inputs, got, record):
    """The retry store's value for key; with record the live value `got` is stored for this host.
    Recorded on arm64-mac only; x86-mac takes the arm64 value, as for STORE."""
    if not record:
        return RETRY.ref(key, inputs, canonical_host=True)
    old = os.environ.get("DRUMSPIKE_FROZEN")
    os.environ["DRUMSPIKE_FROZEN"] = "record"
    try:
        return RETRY.ref(key, inputs, lambda: got, canonical_host=True)
    finally:
        if old is None:
            os.environ.pop("DRUMSPIKE_FROZEN")
        else:
            os.environ["DRUMSPIKE_FROZEN"] = old


def check_analyze(a, files, dets, bin_path, scratch):
    bad, rows = 0, []
    for d in dets:
        dirs, refs, keys = [], {}, {}
        for fname, path, role in files:
            safe = fname.replace("/", "_").replace(" ", "_")
            dirs.append(make_project(scratch / d, safe, path, role, CASES))
            for case, (crole, params) in CASES.items():
                thresholds = tuple(params.get("thresholds", THRESHOLDS))
                inputs = {"source_sha256": file_sha(path), "role": crole or role, "params": params,
                          "thresholds": list(thresholds), "detector": d,
                          "model_sha256": None}
                # recorded on arm64-mac only; the analysis of the core gives the same events on both hosts
                # (analyze.json: 376 of 376 shared keys equal), so x86-mac takes the arm64 value (canonical_host)
                keys[(str(dirs[-1]), case)] = (f"{fname}|{d}|{case}", inputs)
                refs[(str(dirs[-1]), case)] = STORE.full_value(STORE.ref(f"{fname}|{d}|{case}", inputs, canonical_host=True))
        out = run_tool(bin_path, ["analyze", *map(str, dirs)])
        lines = [json.loads(line) for line in out.splitlines()]
        assert isinstance(lines, list) and len(lines) == len(dirs), (len(lines), len(dirs))
        n_files = n_ok = n_events = 0
        for line in lines:
            ok_file = True
            for case in CASES:
                got = line["events"][case]
                want = refs[(line["project"], case)]
                if frozen.is_digest(want):
                    want = got if frozen.same(got, want) else []
                entry = line["job"]["tracks"][case]
                note, retry_ok = "", True
                if (entry.get("weak_signal") or {}).get("to") is not None:
                    # P3-18: the retry ran, so the first pick had no hit; the server (no retry) must
                    # agree, and the retried step and events compare with the retry store
                    key, inputs = keys[(line["project"], case)]
                    ref = retry_ref(key, inputs, {"weak_signal": entry["weak_signal"], "events": got},
                                    a.record_retry)
                    if want != []:
                        retry_ok, note = False, ", the retry ran but the server reference has hits at the first pick"
                    else:
                        want = ref["events"]
                        retry_ok = ref["weak_signal"] == entry["weak_signal"]
                        note = f", retry step {entry['weak_signal']} against {ref['weak_signal']}"
                same = retry_ok and json.dumps(got) == json.dumps(want) and entry["state"] == "done" \
                    and entry["detector"] == "rust-dsp"
                n_events += len(want)
                if not same:
                    ok_file = False
                    bad += 1
                    k = next((i for i, (g, w) in enumerate(zip(got, want)) if json.dumps(g) != json.dumps(w)),
                             min(len(got), len(want)))
                    print(f"FAIL {Path(line['project']).name} [{d}] case {case}: {len(got)} vs {len(want)} events, "
                          f"first difference at {k}, job entry {entry}{note}")
            n_files += 1
            n_ok += ok_file
        print(f"{d}: {n_ok}/{n_files} files identical in 4 cases, {n_events} events", flush=True)
        rows.append({"detector": d, "files": n_files, "files_ok": n_ok, "events": n_events})
    return bad, rows


def check_export(corpus, bin_path, scratch):
    corpus_ev = corpus_events(corpus)
    src = next(iter(corpus_ev))
    pdir = make_project(scratch / "export", "ExportProject", corpus / src, "kick", {"T": (None, {})})
    cases = list(midi_cases(corpus_ev))
    jobs = [{"project": str(pdir), "track": "T", "events": ev, "note": note, "ppq": ppq, "tempo": tempo}
            for _, ev, note, ppq, tempo in cases]
    # one process per case: every case writes the same Exports/T_spike.mid
    bad, same_bytes, sizes = 0, 0, 0
    for (name, ev, note, ppq, tempo), job in zip(cases, jobs):
        want = bytes.fromhex(midi_reference(name, ev, note, ppq, tempo)["hex"])
        jf = scratch / "job.json"
        jf.write_text(json.dumps([job]))
        res = json.loads(run_tool(bin_path, ["export", str(jf)]))
        assert isinstance(res, list) and len(res) == 1, res
        if "error" in res[0]:
            print(f"FAIL export {name}: {res[0]['error']}")
            bad += 1
            continue
        got = Path(res[0]["path"]).read_bytes()
        if got == want:
            same_bytes += 1
            sizes += len(got)
        else:
            bad += 1
            k = next((i for i, (g, w) in enumerate(zip(got, want)) if g != w), min(len(got), len(want)))
            print(f"FAIL export {name}: {len(got)} vs {len(want)} bytes, first difference at {k}")
    print(f"export_midi: {same_bytes}/{len(cases)} event lists give identical bytes "
          f"({len([c for c in cases if c[0].startswith('corpus')])} corpus lists, "
          f"{sum(len(c[1]) for c in cases)} events, {sizes} bytes)", flush=True)
    return bad, {"lists": len(cases), "identical": same_bytes}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("corpus")
    ap.add_argument("--samples")
    ap.add_argument("--layout", choices=["named", "flat"], default="named")
    ap.add_argument("--song1")
    ap.add_argument("--detectors", default="dsp")
    ap.add_argument("--bin", default=str(DEFAULT_BIN))
    ap.add_argument("--json")
    ap.add_argument("--record-retry", action="store_true",
                    help="store the live retry results in tests/golden/frozen/service_retry.json (arm64-mac)")
    a = ap.parse_args()
    dets = a.detectors.split(",")
    corpus = Path(a.corpus)
    files = tracks(a)
    print(f"{len(files)} files ({sum(1 for f in files if not f[0].startswith('corpus/'))} real), "
          f"detectors {', '.join(dets)} (the DSP detector)", flush=True)
    with tempfile.TemporaryDirectory(prefix="ds_service_") as tmp:
        scratch = Path(tmp)
        bad_a, rows = check_analyze(a, files, dets, Path(a.bin), scratch)
        bad_e, exp = check_export(corpus, Path(a.bin), scratch)
    if a.json:
        Path(a.json).write_text(json.dumps({"analyze": rows, "export": exp}, indent=1))
    print("PASS" if bad_a + bad_e == 0 else f"FAIL ({bad_a + bad_e})")
    return 0 if bad_a + bad_e == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
