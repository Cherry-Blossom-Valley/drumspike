"""#276 part B: the DSP detector (`rust-dsp`) on ENST-Drums.

    uv run --with numpy --with soundfile --with mido --with mir_eval \
        python tools/eval/enst.py --all          # index, tom mapping, runs, scores, tables
    uv run --with numpy --with soundfile --with mido --with mir_eval \
        python tools/eval/enst.py --all --configs default,dsp
    python tools/eval/enst.py --score-only       # tables from the existing run files

Data: `ENST_ROOT` (default: the copy under ~/.cache/drumspike/datasets/ENST-drums-public), the public copy of
Gillet & Richard, "ENST-Drums", ISMIR 2006 (CC BY-NC-ND 4.0). Never commit its audio, annotations or derived
per-hit copies; only aggregate numbers leave this host. The path is read from the env; a missing root FAILs
(SKIP only with an explicit `--allow-missing`).

Items: the `phrase`, `solo`, `minus-one` and `MIDI-minus-one` items of the three drummers (the file name
`<NNN>_<type>_<details>_<tool>`, with a trailing `_xN` repeat count on `hits` items; #262). Each item is a
session of every close mic (kick, snare, hi-hat, the tom mics) plus the two overheads (role `cymbal`), run
through the release `service_tool` in the configurations of #276; every configuration runs
the app's DSP detector.

Labels (#262): kick = `bd`; snare = `sd`, `sd-`, `rs`, `cs`; hat = `chh`, `ohh`; tom = the tom labels
(`mt`, `lt`, `lmt`, `lft`, `mtr`, `ltr`); cymbal = rides, crashes and the other cymbal labels; `cb`, `sweep`
and `sticks` are unmapped and counted. The tom label to tom mic mapping per drummer is found from the data
(#262 step 2): for each tom label, the tom mic with the highest mean energy around the label's annotated
onsets in the drummer's isolated `hits` items; a label without hits evidence FAILs (undecided).

Scoring: each close mic against its drum's annotations, one-to-one nearest matching within the window
(the `mir_eval.onset` semantics; the local matcher when mir_eval is not importable), at +-50 ms (headline)
and +-10 ms: P/R/F1 per role and drummer (pooled over the drummer's items), the median timing offset of the
matched hits per role and drummer, and the bleed false positives (an output hit of a mic that matches another
drum's annotation), per mic and other role. One result file per item and configuration under
`~/.cache/drumspike/fix-metric-model-vs-dsp/enst/run/` makes the heavy part resumable.

The tables print the item and event counts per drummer and FAIL on a missing item (no wav or no annotation)
or a missing role row. Aggregate numbers only.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
from fix_metric import track_name, wait_for_room  # noqa: E402  (the app's safe name, the load guard)

OUT = Path.home() / ".cache" / "drumspike" / "fix-metric-model-vs-dsp" / "enst"
ITEM_TYPES = ("phrase", "solo", "minus-one", "MIDI-minus-one")
DRUMMERS = ("drummer_1", "drummer_2", "drummer_3")
CONFIGS = ("default", "dsp")            # #276: the run folder names; both run the DSP detector today
WINDOWS = (0.050, 0.010)
ROLES = ("kick", "snare", "hat", "tom")
LABEL_ROLES = {"bd": "kick", "sd": "snare", "sd-": "snare", "rs": "snare", "cs": "snare",
               "chh": "hat", "ohh": "hat",
               "mt": "tom", "lt": "tom", "lmt": "tom", "lft": "tom", "mtr": "tom", "ltr": "tom"}
UNMAPPED = ("cb", "sweep", "sticks")
ROLE_NOTES = {"kick": 36, "snare": 38, "tom": 47, "hat": 42, "cymbal": 49}   # CLASS_NOTES
TOM_MICS = ("tom_1", "tom_2", "tom_3")
TOM_ALIAS = {"mtr": "mt", "ltr": "lt"}   # rimshot variants of the mid/low tom: the same drum, the same mic
CLOSE_MICS = ("kick", "snare", "hi-hat", *TOM_MICS)
SESSION_MICS = (*CLOSE_MICS, "overhead_L", "overhead_R")


# ---------------------------------------------------------------- the index

def parse_name(stem):
    """`<NNN>_<type>_<details>_<tool>` with an optional trailing `_xN` (the repeat count of `hits` items)
    -> (num, type, details, tool, xn)."""
    m = re.match(r"^(\d{3})_(hits|phrase|solo|minus-one|MIDI-minus-one)_(.+)$", stem)
    if not m:
        raise ValueError(f"unparsable item name: {stem}")
    rest, num, typ = m.group(3), m.group(1), m.group(2)
    xn = None
    m2 = re.match(r"^(.+)_x(\d+)$", rest)
    if m2 and "_" in m2.group(1):
        rest, xn = m2.group(1), m2.group(2)
    if "_" not in rest:
        raise ValueError(f"unparsable item name: {stem}")
    details, tool = rest.rsplit("_", 1)
    return num, typ, details, tool, xn


def label_role(label):
    """The Drum Spike role of an annotation label, or None when it is unmapped (`cb`, `sweep`, `sticks`)."""
    if label in LABEL_ROLES:
        return LABEL_ROLES[label]
    if label in UNMAPPED:
        return None
    if re.match(r"^(rc|cr)\d*$|^c\d+$|^ch\d+$|^spl\d*$", label):     # rides, crashes, other cymbals
        return "cymbal"
    return None


def read_annotation(path):
    """[(time s, label)] of one annotation file (`<seconds> <label>`)."""
    out = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line:
            continue
        t, label = line.split()[:2]
        out.append((float(t), label))
    return out


def session_mics(root, drummer):
    """The session mics that the drummer's audio dir actually has: drummer_1 and drummer_2 have no tom_3."""
    audio = Path(root) / drummer / "audio"
    return [m for m in SESSION_MICS if (audio / m).is_dir()]


def index_items(root):
    """{drummer: {stem: {"wav": {mic: path}, "ann": path, "num", "type", "details", "tool", "xn"}}} of the chosen
    item types. A chosen item without its wav set or annotation is kept and FAILs later (the gate wants the count)."""
    idx = {}
    for d in DRUMMERS:
        items = {}
        ann_dir, audio = Path(root) / d / "annotation", Path(root) / d / "audio"
        mics = session_mics(root, d)
        for p in sorted(ann_dir.glob("*.txt")):
            num, typ, details, tool, xn = parse_name(p.stem)
            if typ not in ITEM_TYPES:
                continue
            wavs = {m: audio / m / f"{p.stem}.wav" for m in mics}
            items[p.stem] = {"num": num, "type": typ, "details": details, "tool": tool, "xn": xn,
                             "ann": p, "wav": wavs}
        idx[d] = items
    return idx


def tom_mapping(root, drummer, window=(0.050, 0.100)):
    """{label: tom mic} for one drummer, from the isolated `hits` items: the tom mic with the highest mean energy
    in [t - window[0], t + window[1]] around the label's annotated onsets (mean RMS over the events, 44.1 kHz mono
    16-bit wav). A label with no hits item gets no entry (the caller FAILs on an undecided mapping)."""
    import soundfile as sf
    votes = {}
    for p in sorted((Path(root) / drummer / "annotation").glob("*_hits_*.txt")):
        try:
            parse_name(p.stem)
        except ValueError:
            continue
        ev = [(t, lab) for t, lab in read_annotation(p) if label_role(lab) == "tom"]
        if not ev:
            continue
        energy = {}
        for mic in TOM_MICS:
            wav = Path(root) / drummer / "audio" / mic / f"{p.stem}.wav"
            if not wav.is_file():
                continue
            x, sr = sf.read(str(wav))
            e = []
            for t, _ in ev:
                i0, i1 = max(0, int((t - window[0]) * sr)), min(len(x), int((t + window[1]) * sr))
                if i1 > i0:
                    e.append(float(np.sqrt(np.mean(np.square(x[i0:i1])))))
            energy[mic] = float(np.mean(e)) if e else 0.0
        if not energy or max(energy.values()) <= 0.0:
            continue
        best = max(energy, key=energy.get)
        for _, lab in ev:
            votes.setdefault(TOM_ALIAS.get(lab, lab), {}).setdefault(best, []).append(energy[best] / max(min(energy.values()), 1e-12))
    return {lab: max(mics, key=lambda m: len(votes[lab][m])) for lab, mics in votes.items()}


# ---------------------------------------------------------------- the app's analysis

def tool_fingerprint(path):
    """The detector build's fingerprint: the sha256 of the `service_tool` binary (16 hex chars), or
    None when the path is missing. Run files record it (`tool_fp`), the gate reruns every item whose
    fingerprint does not match the current build, so a stale run file is never scored."""
    p = Path(path)
    if not p.is_file():
        return None
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:16]


def run_item(tool, out, drummer, stem, item, config, scratch):
    """One item through the release `service_tool` (project_new, tracks_add, analyze_start, events_get per track)
    with the environment of `config`; written to `<out>/run/<config>/<drummer>/<stem>.json` (kept: resumes).
    every configuration runs the DSP detector."""
    dst = out / "run" / config / drummer / f"{stem}.json"
    if dst.is_file():
        return json.loads(dst.read_text())
    # A killed run (pre-emption, crash) can leave a partial scratch project behind; project_new
    # would then fail with 409 "project already exists". No result file means the run is not
    # valid, so the whole scratch dir for this item is stale: remove it and start over.
    if scratch.exists():
        shutil.rmtree(scratch)
    present = [m for m in SESSION_MICS if item["wav"].get(m)]
    missing = [m for m in item["wav"] if not item["wav"][m].is_file()]
    if missing:
        raise SystemExit(f"FAIL {drummer}/{stem}: missing mics {missing}")
    roles = {"kick": "kick", "snare": "snare", "hi-hat": "hat", "overhead_L": "cymbal", "overhead_R": "cymbal"}
    src = scratch / "src"
    src.mkdir(parents=True, exist_ok=True)
    # Every mic of an ENST item shares the same wav stem, so the app would de-duplicate the track
    # names (_2, _4, ...). One symlink per mic gives each track the mic's name instead.
    srcs = {}
    for m in present:
        link = src / f"{m}.wav"
        if not link.exists():
            link.symlink_to(item["wav"][m])
        srcs[m] = link
    steps = [{"command": "project_new", "args": {"name": "enst", "dir": str(scratch / "p")}},
             {"command": "tracks_add", "args": {"sid": "$sid", "paths": [str(srcs[m]) for m in present],
                                                "roles": [roles.get(m, "tom") for m in present],
                                                "notes": [ROLE_NOTES[roles.get(m, "tom")] for m in present]},
              "wait": True},
             {"command": "analyze_start", "args": {"sid": "$sid"}, "wait": True},
             {"command": "analyze_status", "args": {"sid": "$sid"}},
             {"command": "tracks_get", "args": {"sid": "$sid"}}]
    steps += [{"command": "events_get", "args": {"sid": "$sid", "name": track_name(m)}} for m in present]
    (scratch / "steps.json").parent.mkdir(parents=True, exist_ok=True)
    (scratch / "steps.json").write_text(json.dumps(steps))
    env = dict(os.environ)
    env.update({"DRUMSPIKE_HOME": str(scratch / "h"), "DRUMSPIKE_PROJECTS": str(scratch / "p")})
    r = subprocess.run([str(tool), "cli", "run", str(scratch / "steps.json")], capture_output=True, text=True,
                       env=env, stdin=subprocess.DEVNULL)
    rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
    bad = [x for x in rows if not x.get("ok", False)]
    if r.returncode != 0 or bad or len(rows) != len(steps):
        raise RuntimeError(f"{drummer}/{stem} {config}: exit {r.returncode} {bad[:1]} {r.stderr[-400:]}")
    tracks = rows[4]["result"]["tracks"]
    res = {"drummer": drummer, "stem": stem, "config": config,
           "tool_fp": tool_fingerprint(tool),
           "detector": {m: tracks[track_name(m)].get("detector") for m in present},
           "events": {m: rows[5 + i]["result"]["events"] for i, m in enumerate(present)}}
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_text(json.dumps(res))
    shutil.rmtree(scratch, ignore_errors=True)
    return res


def do_run(root, out, tool=None, configs=CONFIGS, log=print):
    """Every chosen item of every drummer through both configurations (resumable, one file per item).
    `ENST_LIMIT` (a count) limits the items per drummer for a smoke run."""
    tool = Path(tool or os.environ.get("DRUMSPIKE_CALIB_TOOL") or ROOT / "target" / "release" / "service_tool")
    idx = index_items(root)
    limit = int(os.environ.get("ENST_LIMIT", "0") or 0)
    if limit:
        idx = {d: dict(list(items.items())[:limit]) for d, items in idx.items()}
        log(f"ENST_LIMIT {limit}: {sum(len(i) for i in idx.values())} items")
    scratch_root = out / "scratch"
    n = 0
    for drummer, items in idx.items():
        for stem, item in items.items():
            for config in configs:
                wait_for_room(log)
                scratch = scratch_root / f"{track_name(drummer)}_{stem}_{config}"
                (scratch / "p").mkdir(parents=True, exist_ok=True)
                try:
                    run_item(tool, out, drummer, stem, item, config, scratch)
                finally:
                    shutil.rmtree(scratch, ignore_errors=True)
                n += 1
                log(f"ran {drummer}/{stem} {config} ({n})")
    shutil.rmtree(scratch_root, ignore_errors=True)


# ---------------------------------------------------------------- the scoring

def _match_events(ref, est, window):
    """One to one, nearest pair first, within `window` seconds: [(ref index, est index)] (the `mir_eval.onset`
    semantics)."""
    cand = sorted((abs(e - r), i, j) for i, r in enumerate(ref) for j, e in enumerate(est) if abs(e - r) <= window)
    ur, ue, pairs = set(), set(), []
    for _, i, j in cand:
        if i in ur or j in ue:
            continue
        ur.add(i)
        ue.add(j)
        pairs.append((i, j))
    return pairs


def prf(ref, est, window):
    """(precision, recall, f1) of one onset series against a reference within `window` s (the `mir_eval.onset`
    semantics)."""
    if not len(ref):
        return None
    pairs = _match_events(ref, est, window)
    p = len(pairs) / len(est) if est else 0.0
    r = len(pairs) / len(ref)
    f = 2 * p * r / (p + r) if p + r else 0.0
    return p, r, f


def item_refs(evs, tom_map):
    """{mic: [(t, label)]} of one item: the annotations of the mic's drum (the tom labels via the drummer's
    tom mapping)."""
    refs = {m: [] for m in CLOSE_MICS}
    for t, lab in evs:
        role = label_role(lab)
        if role == "kick":
            refs["kick"].append((t, lab))
        elif role == "snare":
            refs["snare"].append((t, lab))
        elif role == "hat":
            refs["hi-hat"].append((t, lab))
        elif role == "tom" and tom_map.get(TOM_ALIAS.get(lab, lab)) in TOM_MICS:
            refs[tom_map[TOM_ALIAS.get(lab, lab)]].append((t, lab))
    return refs


MIC_ROLE = {"kick": "kick", "snare": "snare", "hi-hat": "hat", "tom_1": "tom", "tom_2": "tom", "tom_3": "tom"}


def do_score(out, root, log=print, configs=None):
    """The tables: the item and event counts per drummer (per type), P/R/F1 per role and drummer at both windows
    (pooled over the drummer's items), the median timing offset of the matched hits, and the bleed table. FAILs on
    a missing item (no wav or annotation), a missing role row (a close mic with no reference hits for a drummer)
    or a wrong detector. Writes `<out>/enst-scores.json` and `<out>/enst.md`. `configs` (default all): every
    configuration runs the DSP detector, so only the run configs are read and the expected detector is `rust-dsp`."""
    idx = index_items(root)
    dmics = {d: session_mics(root, d) for d in DRUMMERS}
    tom_maps = {d: tom_mapping(root, d) for d in DRUMMERS}
    missing, undecided = [], []
    for d in DRUMMERS:
        for stem, item in idx[d].items():
            miss = [m for m in item["wav"] if not item["wav"][m].is_file()]
            if miss or not item["ann"].is_file():
                missing.append(f"{d}/{stem}: missing {miss or 'annotation'}")
        tom_labels = {TOM_ALIAS.get(lab, lab) for stem, item in idx[d].items() for _, lab in read_annotation(item["ann"])
                      if label_role(lab) == "tom"}
        need = tom_labels - set(tom_maps[d])
        if need:
            undecided.append(f"{d}: no tom mic decided for {sorted(need)}")
    if missing:
        raise SystemExit("FAIL missing items:\n" + "\n".join(missing))
    if undecided:
        raise SystemExit("FAIL undecided tom mapping:\n" + "\n".join(undecided))

    anns = {d: {stem: item_refs(read_annotation(item["ann"]), tom_maps[d]) for stem, item in idx[d].items()}
            for d in DRUMMERS}
    runs, det_bad = {}, False
    score_configs = configs or CONFIGS
    for config in score_configs:
        for d in DRUMMERS:
            for stem in idx[d]:
                p = out / "run" / config / d / f"{stem}.json"
                if not p.is_file():
                    raise SystemExit(f"FAIL {p} is missing: run `--all` first")
                res = json.loads(p.read_text())
                runs[(config, d, stem)] = res
                expected = "rust-dsp"
                for m, det in res["detector"].items():
                    name = det.get("detector") if isinstance(det, dict) else det
                    if name != expected:
                        log(f"FAIL {config}/{d}/{stem} {m}: detector {det!r}")
                        det_bad = True
    if det_bad:
        raise SystemExit(1)

    counts = {c: {} for c in score_configs}
    for (config, d, stem), res in runs.items():
        for det in res["detector"].values():
            k = det.get("detector") if isinstance(det, dict) else det
            counts[config][k] = counts[config].get(k, 0) + 1
    for config in score_configs:
        log(f"{config} detectors: " + ", ".join(f"{k} {v}" for k, v in sorted(counts[config].items())))

    per = {}                      # (config, drummer, mic): {"ref": [...], "est": [...], "bleed": {role: n}, "offs": []}
    for (config, d, stem), res in runs.items():
        for mic in CLOSE_MICS:
            if mic not in res["events"]:
                continue
            est = [float(e["t"]) for e in res["events"][mic] if not e.get("bleed") and not e.get("cut")]
            ref = sorted(t for t, _ in anns[d][stem].get(mic, []))
            est = sorted(est)
            cell = per.setdefault((config, d, mic), {"ref": [], "est": [], "bleed": {}, "offs": []})
            cell["ref"] += ref
            cell["est"] += est
            if ref and est:
                pairs = dict(_match_events(ref, est, WINDOWS[0]))
                cell["offs"] += [est[j] - ref[i] for i, j in pairs.items()]
                for j in range(len(est)):
                    if j not in pairs.values():
                        role = MIC_ROLE[mic]
                        for other, ts in anns[d][stem].items():
                            other_role = MIC_ROLE.get(other)
                            if not other_role or other_role == role or other_role == "cymbal":
                                continue
                            if any(abs(est[j] - t) <= WINDOWS[0] for t, _ in ts):
                                cell["bleed"][other] = cell["bleed"].get(other, 0) + 1
                                break

    lines = ["### Part B: ENST-Drums (Gillet & Richard, ISMIR 2006; CC BY-NC-ND 4.0, aggregate numbers only)", ""]
    for config in score_configs:
        lines += [f"#### {config} (rust-dsp, the DSP detector)", ""]
        lines += [f"Items and output events per drummer (types: {', '.join(ITEM_TYPES)}):", "",
                  "| drummer | items | output events |", "|---|---|---|"]
        for d in DRUMMERS:
            stems = [s for (c, dd, s) in runs if c == config and dd == d]
            ev = sum(len(runs[(config, d, s)]["events"][m]) for s in stems for m in runs[(config, d, s)]["events"])
            lines.append(f"| {d} | {len(stems)} | {ev} |")
        lines.append("")
        for window in WINDOWS:
            lines += [f"P / R / F1 at +-{int(window * 1000)} ms (pooled over the drummer's items):", "",
                      "| drummer | role | ref hits | output | P | R | F1 | offset ms |", "|---|---|---|---|---|---|---|---|"]
            for d in DRUMMERS:
                for mic in dmics[d]:
                    if mic not in CLOSE_MICS:
                        continue          # overheads: counted in the items/events table, not scored (no reference)
                    cell = per.get((config, d, mic))
                    role = f"{MIC_ROLE[mic]} ({mic})"
                    if not cell or not cell["ref"]:
                        raise SystemExit(f"FAIL missing role row: {config} {d} {mic} has no reference hits")
                    p, r, f = prf(cell["ref"], cell["est"], window)
                    off = f"{1000 * float(np.median(cell['offs'])):+.1f}" if cell["offs"] else "-"
                    lines.append(f"| {d} | {role} | {len(cell['ref'])} | {len(cell['est'])} | "
                                 f"{p:.3f} | {r:.3f} | {f:.3f} | {off} |")
            lines.append("")
        lines += ["Bleed false positives at +-50 ms (an output hit of the mic that matches another drum's "
                  "annotation, pooled over the drummer's items):", "",
                  "| drummer | mic | " + " | ".join(r for r in ("kick", "snare", "hat", "tom")) + " |",
                  "|---|---|---|---|---|---|"]
        for d in DRUMMERS:
            for mic in dmics[d]:
                cell = per.get((config, d, mic), {})
                agg = dict(cell.get("bleed", {}))
                lines.append(f"| {d} | {mic} | " + " | ".join(str(agg.get(r, 0)) for r in ("kick", "snare", "hat", "tom")) + " |")
        lines.append("")
    unmapped = {}
    for d in DRUMMERS:
        for stem, item in idx[d].items():
            for _, lab in read_annotation(item["ann"]):
                if label_role(lab) is None:
                    unmapped[lab] = unmapped.get(lab, 0) + 1
    lines += ["Unmapped labels (counted, not scored): " +
              (", ".join(f"`{k}` {v}" for k, v in sorted(unmapped.items())) or "none"), ""]
    lines += ["Tom label to tom mic mapping per drummer (from the isolated hits items, #262 step 2): " +
              "; ".join(f"{d}: " + ", ".join(f"{k}->{v}" for k, v in sorted(tom_maps[d].items())) for d in DRUMMERS), ""]
    (out / "enst-scores.json").write_text(json.dumps({"tom_maps": tom_maps, "counts": counts, "unmapped": unmapped,
                                                      "per": {f"{c}|{d}|{m}": v for (c, d, m), v in per.items()}},
                                                     default=list))
    (out / "enst.md").write_text("\n".join(lines) + "\n")
    log("\n".join(lines))
    return per


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--all", action="store_true", help="index, tom mapping, the runs, the scores and the tables")
    ap.add_argument("--score-only", action="store_true", help="the tables from the existing run files")
    ap.add_argument("--configs", default=",".join(CONFIGS), help="the configurations (default: default,dsp)")
    ap.add_argument("--tool", default=None, help="run: the service_tool (default: the release build)")
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--allow-missing", action="store_true", help="SKIP (exit 0) when ENST_ROOT is missing, not FAIL")
    a = ap.parse_args(argv)
    root = os.environ.get("ENST_ROOT", str(Path.home() / ".cache" / "drumspike" / "datasets" / "ENST-drums-public"))
    if not Path(root).is_dir():
        if a.allow_missing:
            print(f"SKIP: ENST_ROOT {root} is missing")
            return 0
        raise SystemExit(f"FAIL: ENST_ROOT {root} is missing")
    out = Path(a.out)
    configs = tuple(c.strip() for c in a.configs.split(","))
    if a.all:
        do_run(root, out, a.tool, configs)
    if a.all or a.score_only:
        if os.environ.get("ENST_LIMIT", ""):
            print(f"ENST_LIMIT {os.environ['ENST_LIMIT']}: incomplete run, skipping the score")
        else:
            do_score(out, root, configs=configs)
    return 0


if __name__ == "__main__":
    sys.exit(main())
