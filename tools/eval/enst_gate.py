"""C13 (#389): ENST part B as a standing detection gate on linux-arm64.

    uv run --project . python tools/eval/enst_gate.py
    uv run --project . python tools/eval/enst_gate.py --plant drop-kick-tenth   # the test regression
    uv run --project . python tools/eval/enst_gate.py --refresh-baseline        # after an intended change

The gate scores the ENST-Drums corpus of #276 part B (see `enst.py`) from the run files under
`~/.cache/drumspike/fix-metric-model-vs-dsp/enst/run/`, prints the twelve aggregate F1 numbers
(three drummers x kick, snare, hat, tom, pooled over the drummer's items, at the +-50 ms headline
window) and compares them with the committed baseline `enst_gate_baseline.json` (aggregate numbers
only, no per-hit data, no audio). Any number more than 2 % below its baseline is a regression: the
changed rows are printed and the gate FAILs. It also FAILs when no row could be compared (the count
of compared rows is always printed).

Run files: missing items are run through the release `service_tool` exactly like `enst.py --all`
(resumable, one file per item). Every run file records the fingerprint (the sha256) of the
`service_tool` binary that measured it; the gate reruns every item whose run file is missing or
was written by another build, and refuses to score a stale file. `--rerun` stays as the force
switch that measures every item again. `ENST_ROOT` (default: the linux-arm64 copy under
`~/.cache/drumspike/datasets/`, then the arm64-mac path) points at the dataset; a missing root FAILs
(SKIP only with an explicit `--allow-missing`).

`--refresh-baseline` writes the current numbers as the baseline together with the git sha of the
working tree and the date, so the baseline always says which detector it belongs to.

`--plant drop-kick-tenth` (or the env `ENST_GATE_PLANT`) is the test-only regression of the ticket:
it drops every tenth kick hit at scoring time, so the gate must print the changed kick rows and
FAIL. A planted run refuses to refresh the baseline.
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "eval"))
sys.path.insert(0, str(ROOT / "tools"))
import enst  # noqa: E402


def git_sha():
    """The git sha of the working tree (the detector the numbers belong to), 'unknown' outside a repo."""
    try:
        return subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                              capture_output=True, text=True, check=True).stdout.strip()
    except Exception:
        return "unknown"

OUT = Path.home() / ".cache" / "drumspike" / "fix-metric-model-vs-dsp" / "enst"
BASELINE = Path(__file__).resolve().parent / "enst_gate_baseline.json"
GATE_WINDOW = enst.WINDOWS[0]          # the +-50 ms headline window
GATE_CONFIG = "dsp"                    # every configuration runs the DSP detector (#276)
REGRESSION = 0.02                      # FAIL on a 2 % regression
ROLES = ("kick", "snare", "hat", "tom")
MIC_OF_ROLE = {"kick": "kick", "snare": "snare", "hat": "hi-hat"}
PLANTS = {"drop-kick-tenth"}


# ---------------------------------------------------------------- the scoring

def plant_est(role, est, plant):
    """The test-only regression: the planted kick series with every tenth hit dropped."""
    if plant == "drop-kick-tenth" and role == "kick":
        return [e for i, e in enumerate(est) if i % 10 != 9]
    return est


def numbers_from_per(per):
    """{f"{drummer}|{role}": f1} of a `per` dict of enst.do_score's shape
    ({(config, drummer, mic): {"ref": [...], "est": [...]}}): the F1 of each role per drummer at the
    headline window, the tom mics pooled per drummer. A drummer/role with no reference hits is
    omitted (the caller FAILs on a missing row)."""
    numbers = {}
    drummers = sorted({d for (_, d, _) in per})
    for d in drummers:
        for role in ROLES:
            mics = [MIC_OF_ROLE[role]] if role != "tom" else [m for m in enst.TOM_MICS]
            ref, est = [], []
            for (c, dd, m) in per:
                if dd != d or m not in mics:
                    continue
                ref += per[(c, dd, m)]["ref"]
                est += per[(c, dd, m)]["est"]
            if not ref:
                continue
            p, r, f = enst.prf(sorted(ref), sorted(est), GATE_WINDOW)
            numbers[f"{d}|{role}"] = f
    return numbers


def collect_per(out, root, config, plant=None, tool_fp=None, log=print):
    """The `per` dict of the run files of one configuration (enst.do_score's cell shape), with the
    planted regression applied. FAILs on a missing run file, a stale run file (a fingerprint that
    does not match the current tool build), a wrong detector or a missing annotation."""
    idx = enst.index_items(root)
    per = {}
    fingerprint = hashlib.sha256()
    n_items = 0
    for d in enst.DRUMMERS:
        for stem in sorted(idx[d]):
            item = idx[d][stem]
            if not item["ann"].is_file():
                raise SystemExit(f"FAIL {d}/{stem}: the annotation {item['ann']} is missing")
            p = out / "run" / config / d / f"{stem}.json"
            if not p.is_file():
                raise SystemExit(f"FAIL {p} is missing: run the gate again so the item is measured")
            res = json.loads(p.read_text())
            if res.get("tool_fp") != tool_fp:
                raise SystemExit(f"FAIL {config}/{d}/{stem}: run file is stale "
                                 f"(tool_fp {res.get('tool_fp')!r}, current build {tool_fp!r}); "
                                 "the gate reruns stale items, it never scores them")
            n_items += 1
            for m, det in sorted(res["detector"].items()):
                name = det.get("detector") if isinstance(det, dict) else det
                if name != "rust-dsp":
                    raise SystemExit(f"FAIL {config}/{d}/{stem} {m}: detector {det!r} is not rust-dsp")
                fingerprint.update(f"{d}/{stem}/{m}={json.dumps(det, sort_keys=True)};".encode())
            refs = enst.item_refs(enst.read_annotation(item["ann"]), _tom_map_cached(out, root, d))
            for mic in enst.CLOSE_MICS:
                if mic not in res["events"]:
                    continue
                role = enst.MIC_ROLE[mic]
                est = [float(e["t"]) for e in res["events"][mic] if not e.get("bleed") and not e.get("cut")]
                est = plant_est(role, sorted(est), plant)
                ref = sorted(t for t, _ in refs.get(mic, []))
                cell = per.setdefault((config, d, mic), {"ref": [], "est": []})
                cell["ref"] += ref
                cell["est"] += est
    log(f"scored {n_items} items of config {config}; detector fingerprint {fingerprint.hexdigest()[:16]}"
        + (f" (PLANTED regression: {plant})" if plant else ""))
    return per


_TOM_MAPS = {}


def _tom_map_cached(out, root, drummer):
    """The drummer's tom mapping, computed once per gate run (needs the isolated hits wavs)."""
    if drummer not in _TOM_MAPS:
        _TOM_MAPS[drummer] = enst.tom_mapping(root, drummer)
    return _TOM_MAPS[drummer]


# ---------------------------------------------------------------- the compare

def compare_numbers(baseline, current, tol=REGRESSION):
    """(rows, changed, improved, missing) of the baseline against the current numbers: a row is
    changed when the current F1 is more than `tol` (relative) below the baseline. Rows are compared
    on the baseline's keys; a key without a current number is missing."""
    changed, improved, missing = [], [], []
    rows = 0
    for k, base in baseline.items():
        if k not in current:
            missing.append(k)
            continue
        rows += 1
        cur = current[k]
        if cur < base * (1.0 - tol):
            changed.append((k, base, cur))
        elif cur > base:
            improved.append((k, base, cur))
    return rows, changed, improved, missing


def verdict(rows, changed, missing):
    """(ok, lines) of one compare: the changed and missing rows, FAIL on no compared row."""
    lines = []
    for k, base, cur in changed:
        lines.append(f"REGRESSED {k}: baseline {base:.3f} current {cur:.3f} ({1000 * (cur - base):+.1f} milli-F1)")
    for k in missing:
        lines.append(f"MISSING {k}: no current number")
    if rows == 0:
        lines.append("FAIL: no row could be compared")
    return (not changed and not missing and rows > 0), lines


# ---------------------------------------------------------------- the gate

def stale_run_files(out, root, config, tool_fp):
    """[drummer/stem] of one configuration whose run file is missing or stale: it was not written by
    the current tool build (no `tool_fp`, or one that differs). A stale file is never scored."""
    idx = enst.index_items(root)
    stale = []
    for d in idx:
        for stem in idx[d]:
            p = out / "run" / config / d / f"{stem}.json"
            if not p.is_file() or json.loads(p.read_text()).get("tool_fp") != tool_fp:
                stale.append(f"{d}/{stem}")
    return stale


def do_gate(root, out, baseline_path, tool=None, config=GATE_CONFIG, plant=None,
            rerun=False, refresh=False, allow_missing=False, log=print):
    """The gate: measure, compare with the baseline, print the verdict. Returns the exit code."""
    if plant and refresh:
        raise SystemExit("FAIL: a planted regression must not refresh the baseline")
    if not Path(root).is_dir():
        if allow_missing:
            log(f"SKIP: ENST_ROOT {root} is missing")
            return 0
        raise SystemExit(f"FAIL: ENST_ROOT {root} is missing")

    # the run step: stale items (another detector build) and missing items are measured; --rerun
    # is the force switch that measures every item again
    tool_path = Path(tool or os.environ.get("DRUMSPIKE_CALIB_TOOL") or ROOT / "target" / "release" / "service_tool")
    tool_fp = enst.tool_fingerprint(tool_path)
    if rerun:
        n_all = sum(len(i) for i in enst.index_items(root).values())
        log(f"--rerun (force): all {n_all} items are measured again (build fingerprint {tool_fp})")
        for p in (Path(out) / "run" / config).rglob("*.json"):
            p.unlink()
    stale = stale_run_files(out, root, config, tool_fp)
    if stale:
        if not tool_fp:
            raise SystemExit(f"FAIL: {len(stale)} run file(s) stale or missing, but the tool "
                             f"{tool_path} is not built (DRUMSPIKE_CALIB_TOOL)")
        log(f"{len(stale)} run file(s) from another detector build (or missing), they are measured "
            f"again for the current build (fingerprint {tool_fp}); the first: {stale[0]}")
        for d, stem in (s.split("/", 1) for s in stale):
            (Path(out) / "run" / config / d / f"{stem}.json").unlink(missing_ok=True)
        missing = stale
    else:
        missing = []
    if missing and not tool_path.is_file():
        raise SystemExit(f"FAIL: {len(missing)} run file(s) missing, but the tool "
                         f"{tool_path} is not built (DRUMSPIKE_CALIB_TOOL)")
    if missing:
        enst.do_run(root, out, tool_path, configs=(config,), log=log)

    per = collect_per(out, root, config, plant=plant, tool_fp=tool_fp, log=log)
    numbers = numbers_from_per(per)

    log("")
    log(f"the {len(numbers)} numbers of the gate (F1 at +-{int(GATE_WINDOW * 1000)} ms, config {config}):")
    for k in sorted(numbers):
        log(f"  {k}: {numbers[k]:.3f}")

    if refresh:
        if len(numbers) != len(enst.DRUMMERS) * len(ROLES):
            raise SystemExit(f"FAIL: {len(enst.DRUMMERS) * len(ROLES) - len(numbers)} role row(s) "
                             f"have no reference hits, the baseline must hold all twelve")
        baseline_path.write_text(json.dumps({"window": GATE_WINDOW, "config": config,
                                             "compared": len(numbers), "tool_fp": tool_fp,
                                             "git_sha": git_sha(), "date": time.strftime("%Y-%m-%d"),
                                             "numbers": numbers}, indent=1) + "\n")
        log(f"baseline refreshed: {baseline_path} (aggregate numbers only, git {git_sha()}, "
            f"build {tool_fp}); not compared this run")
        return 0
    if not baseline_path.is_file():
        raise SystemExit(f"FAIL: the baseline {baseline_path} is missing (run --refresh-baseline once)")
    base = json.loads(baseline_path.read_text())["numbers"]
    rows, changed, improved, missing_rows = compare_numbers(base, numbers)
    ok, lines = verdict(rows, changed, missing_rows)
    for k, b, c in improved:
        log(f"improved  {k}: baseline {b:.3f} current {c:.3f}")
    for line in lines:
        log(line)
    log(f"rows compared: {rows}")
    log("PASS" if ok else "FAIL")
    return 0 if ok else 1


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--root", default=None, help="the ENST-Drums root (default: ENST_ROOT, the linux-arm64 copy, the arm64-mac path)")
    ap.add_argument("--tool", default=None, help="the service_tool for missing items (default: DRUMSPIKE_CALIB_TOOL, the release build)")
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--config", default=GATE_CONFIG, help=f"the run configuration (default: {GATE_CONFIG})")
    ap.add_argument("--plant", default=None, choices=sorted(PLANTS),
                    help="the test-only regression of the ticket (or env ENST_GATE_PLANT)")
    ap.add_argument("--rerun", action="store_true", help="measure every item again, not only the missing ones")
    ap.add_argument("--refresh-baseline", action="store_true", help="write the current numbers as the baseline, do not compare")
    ap.add_argument("--allow-missing", action="store_true", help="SKIP (exit 0) when the ENST root is missing, not FAIL")
    ap.add_argument("--baseline", default=str(BASELINE))
    a = ap.parse_args(argv)
    root = a.root or os.environ.get("ENST_ROOT")
    plant = a.plant or (os.environ.get("ENST_GATE_PLANT") if os.environ.get("ENST_GATE_PLANT") in PLANTS else None)
    if root is None:
        for cand in (Path.home() / ".cache" / "drumspike" / "datasets" / "ENST-drums-public",
                     Path("/Volumes/Corsair-4TB/Datasets/ENST_Drums/ENST-drums-public")):
            if cand.is_dir():
                root = str(cand)
                break
    if root is None:
        root = str(Path.home() / ".cache" / "drumspike" / "datasets" / "ENST-drums-public")
    return do_gate(root, Path(a.out), Path(a.baseline), a.tool, a.config, plant,
                   a.rerun, a.refresh_baseline, a.allow_missing)


if __name__ == "__main__":
    sys.exit(main())
