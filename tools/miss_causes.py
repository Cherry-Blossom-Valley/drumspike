"""P4-10 stage 1: why does Drum Spike miss a ground-truth hit? One cause per miss.

    uv run python tools/miss_causes.py run        # detect every kit file with DSP, keep the events
    uv run python tools/miss_causes.py gain       # what-if: the DSP detector on copies with +12, +24, +36 dB
    uv run python tools/miss_causes.py dryrun     # the cheapest fix per cause: peak rules on the kept activations, DSP
                                                  # rescue, the waveform second-onset search, the gain runs scored
    uv run python tools/miss_causes.py classify   # found or missed-with-cause per ground-truth hit; tables.md
    uv run python tools/miss_causes.py all

Inputs (read only): the owner's DRT kit (``~/.cache/drumspike/drt-testkit``: audio, ``ground_truth.csv``) and the P4-11
calibration kit (``~/.cache/drumspike/calib-kit``: sets ``syn`` and ``real``, conditions clean, noise and bleed,
levels and close pairs). Output: ``~/.cache/drumspike/detect-gaps/`` (``work/<id>/`` holds ``events.json``;
``causes.json``, ``causes.csv``, ``tables.md``, ``dryrun.json``). Nothing goes into git.

The detection chain (``crates/core/src/detect/peaks.rs``, ``analyze.rs``) is replayed in Python with the madmom rules of ``Chain::Mix``:

    1. activation `a` of the class column                  (blind: the column never rises in the hit's window)
    2. `a >= thr`                                           (threshold: the column is under the class threshold)
    3. `a >= avg + thr`, avg = mean over [t-10, t+1]        (average: the loud neighbour lifted the average)
    4. `det` is the maximum of [t-2, t+1]                   (not a local maximum: a neighbour frame wins)
    5. combine, 20 ms, keeps the first                      (merged by combine)
    6. role filter, min gap 20 ms, refinement, tolerance    (later: role, dedup, refine, offset)

A miss gets the FIRST stage that stops it, so the causes are disjoint and add up to the misses.
"""
import argparse
import csv
import json
import math
import os
import re
import shutil
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import calib_kit as ck  # noqa: E402

OUT = Path.home() / ".cache" / "drumspike" / "detect-gaps"
DRT = Path.home() / ".cache" / "drumspike" / "drt-testkit"
CALIB = ck.DEFAULT_OUT

N_CLASSES = 5
THRESHOLDS = [0.22, 0.24, 0.32, 0.22, 0.30]
FPS = 100.0
PRE_AVG, POST_AVG, PRE_MAX, POST_MAX = 10, 1, 2, 1
COMBINE_S = 0.02
MIN_GAP_S = 0.020
TOL_S = 0.010
CLASS_NOTES = [35, 38, 47, 42, 49]
ROLE_CLASSES = {"kick": [0], "snare": [1], "tom": [2], "hat": [3], "cymbal": [4]}
ROLE_NOTE = {"kick": 36, "snare": 38, "tom": 47, "hat": 42, "cymbal": 49}

# DRT kit: the role of each file (the owner's run: snare files as snare, the kick ramp as kick).
DRT_ROLE = {"07_kick_velocity_ramp": "kick"}
DRT_DEFAULT_ROLE = "snare"

# The causes in pipeline order (the order of the table).
CAUSES = ["blind", "threshold", "lobe", "average", "not_max", "combine", "role", "mingap", "refine", "offset"]
CAUSE_TEXT = {
    "blind": "no activation: the class column stays below BLIND in the hit's window",
    "threshold": "activation under the class threshold",
    "lobe": "inside the lobe of a found neighbour hit: the column has no second maximum of its own",
    "average": "removed by the moving average (act - avg < thr, pre_avg 0.1 s)",
    "not_max": "not a local maximum (a neighbour frame in [t-2, t+1] is larger)",
    "combine": "merged by combine (20 ms keeps the first peak)",
    "role": "another class fired, the own class did not (role filter drops it)",
    "mingap": "dropped by the 20 ms minimum gap after the role filter",
    "refine": "picked, but no event left near it (refinement merged it or moved it away)",
    "offset": "an event exists but lies outside the 10 ms tolerance",
}
BLIND = 0.05            # a column maximum under this in the window: the model is blind
WIN_BEFORE_S = 0.030    # the hit's window for the activation: [t - 30 ms, t + 10 ms]; the pick leads the attack
WIN_AFTER_S = 0.010     # by 10 ms on median (measured on the found hits), the refinement moves it forward


# ---------------------------------------------------------------- the items (kit files with their ground truth)

def kind_group(file, kind):
    """The hit group of a DRT ground-truth row from its `role` column."""
    if kind.startswith("bleed"):
        return "bleed"
    if kind == "target_grace":
        return "flam_grace"
    if kind == "target_main":
        return "flam_main"
    if kind == "target_ghost":
        return "ghost"
    if kind == "target_first":
        return "pair_first"
    if kind.startswith("target_second"):
        return "pair_second"
    if file.startswith("06") or file.startswith("07"):
        return "ramp"
    if file.startswith("01"):
        return "click"
    return "plain"


def drt_items(drt=DRT):
    rows = {}
    with open(drt / "ground_truth.csv") as fh:
        for r in csv.DictReader(fh):
            rows.setdefault(r["file"][:-4], []).append(r)
    items = []
    for stem, rs in sorted(rows.items()):
        role = DRT_ROLE.get(stem, DRT_DEFAULT_ROLE)
        gt = []
        for r in rs:
            # the kit's columns: `kind` is the instrument, `role` says what the row is for the scorer
            g = kind_group(stem, r["role"])
            m = re.match(r"target_second_(\d+)ms", r["role"])
            gt.append({"t": float(r["onset_s"]), "group": g, "level": float(r["level_dbfs"]),
                       "gap_ms": int(m.group(1)) if m else None, "kind": r["role"]})
        items.append({"id": f"drt/{stem}", "set": "drt", "file": stem, "audio": str(drt / f"{stem}.wav"),
                      "role": role, "note": ROLE_NOTE[role], "gt": gt})
    return items


def calib_items(calib=CALIB):
    items = []
    for kind in ("syn", "real"):
        for audio in sorted((calib / "audio" / kind).glob("*/*.wav")):
            cond, shell = audio.parent.name, audio.stem.split("_")[0]
            sr = int(audio.stem.split("_")[1])
            if kind == "syn" and sr != 48000:
                continue
            refs = ck.read_csv(calib / "ref" / kind / cond / f"{audio.stem}.csv")
            starts = {}
            for r in refs:
                if r["group"] == "pair":
                    starts.setdefault(r["pair"], []).append(r["abs_s"])
            gt = []
            for r in refs:
                if r["group"] == "main":
                    g, gap = "main", None
                else:
                    ts = sorted(starts[r["pair"]])
                    second = r["abs_s"] != ts[0]
                    g = "pair_second" if second else "pair_first"
                    gap = round((ts[1] - ts[0]) * 1000) if second else None
                gt.append({"t": r["abs_s"], "group": g, "level": r["level_db"], "gap_ms": gap, "kind": g})
            role = ck.ROLE[shell]
            items.append({"id": f"{kind}/{cond}/{audio.stem}", "set": f"{kind}-{cond}", "file": audio.stem,
                          "audio": str(audio), "role": role, "note": ck.NOTE[shell], "gt": gt})
    return items


def all_items(drt=DRT, calib=CALIB):
    items = drt_items(drt)
    if calib.is_dir():
        items += calib_items(calib)
    return items


# ---------------------------------------------------------------- detection through the agent CLI (project kept)

def find_tool():
    env = os.environ.get("DRUMSPIKE_CALIB_TOOL")
    if env:
        return Path(env)
    return Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target") / "release" / "service_tool"


def run_item(tool, out, item, detector, model_path):
    """project_new, tracks_add, analyze_start, events_get through `service_tool cli run`; copies the kept activations
    and the events to `work/<id>/<detector>/`; the project and the app home are removed."""
    wd = out / "work" / item["id"] / detector
    if wd.exists():
        shutil.rmtree(wd)
    wd.mkdir(parents=True)
    home, projects = out / "run-home" / item["id"] / detector, out / "run-projects" / item["id"] / detector
    for d in (home, projects):
        if d.exists():
            shutil.rmtree(d)
        d.mkdir(parents=True)
    audio = item["audio"]
    name = Path(audio).stem
    steps = [
        {"command": "project_new", "args": {"name": "mc", "dir": str(projects)}},
        {"command": "tracks_add", "args": {"sid": "$sid", "paths": [audio], "roles": [item["role"]],
                                           "notes": [item["note"]]}, "wait": True},
        {"command": "analyze_start", "args": {"sid": "$sid"}, "wait": True},
        {"command": "events_get", "args": {"sid": "$sid", "name": name}},
    ]
    script = home / "steps.json"
    script.write_text(json.dumps(steps))
    env = {**os.environ, "DRUMSPIKE_HOME": str(home / "h"), "DRUMSPIKE_PROJECTS": str(projects)}
    r = subprocess.run([str(tool), "cli", "run", str(script)], capture_output=True, text=True, env=env)
    rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
    bad = [x for x in rows if not x.get("ok", False)]
    if r.returncode != 0 or bad or len(rows) != len(steps):
        raise RuntimeError(f"{item['id']} {detector}: exit {r.returncode} {bad[:1]} {r.stderr[-400:]}")
    (wd / "events.json").write_text(json.dumps(rows[3]["result"]))
    shutil.rmtree(home, ignore_errors=True)
    shutil.rmtree(projects, ignore_errors=True)
    return wd


def run_all(out, jobs=2, detectors=("dsp",), only=None, log=print):
    tool = find_tool()
    if not tool.is_file():
        raise SystemExit(f"no service_tool at {tool}: cargo build --release -p drumspike-devtools --bin service_tool")
    items = all_items()
    if only:
        items = [i for i in items if re.search(only, i["id"])]
    out.mkdir(parents=True, exist_ok=True)
    (out / "items.json").write_text(json.dumps(items))
    work = [(i, d) for i in items for d in detectors]
    with ThreadPoolExecutor(max_workers=jobs) as ex:
        futs = [(i, d, ex.submit(run_item, tool, out, i, d, "")) for i, d in work]
        for i, d, f in futs:
            f.result()
            log(f"ran {i['id']} {d}")
    return len(work)


def read_activation(path):
    """The retired activation dump format (was activation.rs on the model path); DSP mode has none."""
    b = Path(path).read_bytes()
    if b[:8] != b"DSACT\0\0\0":
        raise ValueError(f"{path}: not an activation file")
    version, classes = np.frombuffer(b[8:16], "<u4").tolist()
    frames = int(np.frombuffer(b[16:24], "<u8")[0])
    if version != 1 or classes != N_CLASSES or len(b) != 24 + frames * classes * 4:
        raise ValueError(f"{path}: bad header or size")
    return np.frombuffer(b[24:], "<f4").reshape(frames, classes)


# ---------------------------------------------------------------- the peak rules, replayed (Chain::Mix = madmom)

def moving_average(x, pre=PRE_AVG, post=POST_AVG):
    """Mean over [t - pre, t + post] with zero padding (peaks.rs `moving_average`, madmom; the shipped 10 and 1)."""
    n = len(x)
    pad = np.concatenate([np.zeros(pre), np.asarray(x, dtype=np.float64), np.zeros(post)])
    cs = np.concatenate([[0.0], np.cumsum(pad)])
    w = pre + 1 + post
    return (cs[w:w + n] - cs[:n]) / w


# The shipped peak rules; a dry run passes a changed copy (`avg_off`: no moving average, `a >= thr` alone).
SHIPPED = {"pre_avg": PRE_AVG, "post_avg": POST_AVG, "pre_max": PRE_MAX, "post_max": POST_MAX,
           "combine_s": COMBINE_S, "avg_off": False}


def stages(x, thr, rules=None):
    """The madmom picker of one column with its intermediate sets.

    Returns a dict: `avg`, `pass_thr` (a >= thr), `det` (a if a >= avg + thr), `peaks` (local maxima of det),
    `kept` (after combine), `merged` ({frame: the kept frame it joined})."""
    r = {**SHIPPED, **(rules or {})}
    x = np.asarray(x, dtype=np.float32)
    n = len(x)
    avg = np.zeros(n, np.float32) if r["avg_off"] else moving_average(x, r["pre_avg"], r["post_avg"]).astype(np.float32)
    thr32 = np.float32(thr)
    det = np.where(x >= avg + thr32, x, np.float32(0.0)).astype(np.float32)
    pm, qm = r["pre_max"], r["post_max"]
    pad = np.concatenate([np.zeros(pm, np.float32), det, np.zeros(qm, np.float32)])
    wmax = np.max(np.stack([pad[k:k + n] for k in range(pm + qm + 1)]), axis=0)
    peaks = [int(t) for t in np.flatnonzero((det != 0.0) & (det == wmax))]
    merged, kept = {}, []
    if peaks and not all(p == 0 for p in peaks):
        kept = [peaks[0]]
        left = peaks[0] / FPS
        for p in peaks[1:]:
            if p / FPS - left <= r["combine_s"] + 1e-12:
                merged[p] = kept[-1]
            else:
                kept.append(p)
                left = p / FPS
    return {"avg": avg, "pass_thr": x >= thr32, "det": det, "peaks": peaks, "kept": kept, "merged": merged}


def all_picks(act, thresholds=THRESHOLDS, rules=None):
    """(frame, class) of every kept peak of the five columns, sorted; and the five columns' stages."""
    out, st = [], []
    for c in range(N_CLASSES):
        s = stages(act[:, c], thresholds[c], rules)
        st.append(s)
        out += [(f, c) for f in s["kept"]]
    return sorted(out), st


def role_dedup(picks, role, min_gap=MIN_GAP_S, classes=None):
    """The role filter and the minimum gap of `analyze_detected`: the kept times and the ones the gap dropped."""
    classes = ROLE_CLASSES[role] if classes is None else classes
    ev = sorted((f / FPS, c) for f, c in picks if c in classes)
    kept, dropped, last = [], [], -9e9
    for t, _ in ev:
        if t - last >= min_gap:
            kept.append(t)
            last = t
        else:
            dropped.append(t)
    return kept, dropped


# ---------------------------------------------------------------- the retry of a track without hits (P3-18)

RETRY_STEPS = [95.0, 113.0, 134.0, 150.0]
RETRY_GROWTH = 0.10
SENS_OLD_MAX, SENS_MAX, SENS_FLOOR, SENS_CEIL = 100.0, 150.0, 0.01, 0.85


def scale_of(v):
    """`sensitivity.rs` `scale_of`: the factor of the base thresholds at slider value `v`."""
    v = min(max(v, 0.0), SENS_MAX)
    if v <= SENS_OLD_MAX:
        return 1.6 - 1.2 * (v / SENS_OLD_MAX)
    frac = (v - SENS_OLD_MAX) / (SENS_MAX - SENS_OLD_MAX)
    at_max = SENS_FLOOR / min(THRESHOLDS)
    return 0.4 * (at_max / 0.4) ** frac


def thresholds_of(v):
    s = scale_of(v)
    return [min(max(b * s, SENS_FLOOR), SENS_CEIL) for b in THRESHOLDS]


def effective_picks(act, role, thresholds=None, rules=None, classes=None, min_gap=MIN_GAP_S):
    """The picks the app keeps for a track at the default sensitivity: one pick at the thresholds; with no event left
    after the role filter and the minimum gap, the retry steps of `jobs.rs` `retry_without_hits` (first step with an
    event, a later step replaces it while its count grows by more than 10 %, the first step that does not ends it).
    Returns (picks, stages, thresholds used, retry slider value or None)."""
    th = list(thresholds or THRESHOLDS)
    picks, st = all_picks(act, th, rules)
    if role_dedup(picks, role, min_gap, classes)[0]:
        return picks, st, th, None
    best = None
    for v in RETRY_STEPS:
        t2 = thresholds_of(v)
        if min(t2) >= min(th) - 1e-9:
            continue
        p2, s2 = all_picks(act, t2, rules)
        n = len(role_dedup(p2, role, min_gap, classes)[0])
        if best is None:
            if n:
                best = (v, t2, p2, s2, n)
        elif n > best[4] * (1.0 + RETRY_GROWTH):
            best = (v, t2, p2, s2, n)
        else:
            break
    if best is None:
        return picks, st, th, None
    return best[2], best[3], best[1], best[0]


# ---------------------------------------------------------------- classification

def window_frames(t, n, window=(WIN_BEFORE_S, WIN_AFTER_S)):
    return max(0, int(math.floor((t - window[0]) * FPS))), min(n - 1, int(math.ceil((t + window[1]) * FPS)))


def claim(kept_frames, event_t):
    """The kept frame of the class that an event came from: the nearest to `event_t` - 10 ms (the refinement moves a
    pick forward to the attack, 10 ms on median), within [event - 25 ms, event + 5 ms]; or None."""
    c = [f for f in kept_frames if event_t - 0.025 <= f / FPS <= event_t + 0.005]
    return min(c, key=lambda f: abs(f / FPS - (event_t - 0.010))) if c else None


def is_lobe(t, act, classes, fr, claimed):
    """The hit follows a found hit's pick by -5 to 70 ms and the column has no local maximum of its own in the window
    (outside the found hit's frames): the model gave one lobe for both."""
    if not any(-0.005 <= t - f / FPS <= 0.07 for f in claimed):
        return False
    n = len(act)
    for c in classes:
        for f in fr:
            left = act[f - 1, c] if f > 0 else 0.0
            right = act[f + 1, c] if f + 1 < n else 0.0
            if act[f, c] > left and act[f, c] >= right and act[f, c] >= 0.02:
                return False
    return True


def classify_miss(t, role, act, st, picks, events, claimed=(), blind=BLIND):
    """The first stage that stops the ground-truth hit at `t` (seconds): (cause, detail). `st` are the five columns'
    stages, `picks` all kept peaks, `events` the final event times that no other hit owns, `claimed` the kept frames
    of the hits that were found (their frames [f - 2, f + 1] are not evidence for this hit)."""
    classes = ROLE_CLASSES[role]
    lo, hi = window_frames(t, len(act))
    if lo > hi:
        return "blind", {"peak": 0.0}
    skip = {g for f in claimed for g in range(f - PRE_MAX, f + POST_MAX + 1)}
    fr = [f for f in range(lo, hi + 1) if f not in skip]
    detail = {"peak": max((float(act[f, c]) for f in fr for c in classes), default=0.0)}
    kept_own = [f for c in classes for f in st[c]["kept"] if f in fr]
    if kept_own:
        if events:
            near = min(events, key=lambda e: abs(e - t))
            if abs(near - t) <= 0.05:
                return "offset", {**detail, "event_off_ms": (near - t) * 1000}
        _, dropped = role_dedup(picks, role)
        if any(round(d * FPS) in fr for d in dropped):
            return "mingap", detail
        return "refine", detail
    if any(f in fr for c in classes for f in st[c]["peaks"]):
        own = "combine"
    elif any(st[c]["det"][f] != 0.0 for c in classes for f in fr):
        own = "not_max"
    elif any(bool(st[c]["pass_thr"][f]) for c in classes for f in fr):
        own = "average"
    elif is_lobe(t, act, classes, fr, claimed):
        own = "lobe"
    elif detail["peak"] < blind:
        own = "blind"
    else:
        own = "threshold"
    detail["own"] = own
    if any(f in fr and c not in classes for f, c in picks):
        return "role", detail
    return own, detail


def classify_item(item, act, events, dsp_events=None, thresholds=THRESHOLDS):
    """One row per ground-truth target: found, or the cause of the miss. Also the counts of false events and bleed
    triggers (a `bleed` row of the ground truth is a negative: an event that no target owns and that lies within the
    tolerance of it is a trigger)."""
    role, gt = item["role"], item["gt"]
    classes = ROLE_CLASSES[role]
    targets = [g for g in gt if g["group"] != "bleed"]
    pairs, _, false = ck.match([g["t"] for g in targets], events, TOL_S)
    pairs = dict(pairs)
    picks, st, used, retry = effective_picks(act, role, thresholds)
    own_kept = sorted(f for c in classes for f in st[c]["kept"])
    claimed = [f for f in (claim(own_kept, events[j]) for j in pairs.values()) if f is not None]
    free_events = [e for j, e in enumerate(events) if j not in set(pairs.values())]
    dsp_pairs = dict(ck.match([g["t"] for g in targets], dsp_events, TOL_S)[0]) if dsp_events is not None else {}
    rows = []
    for k, g in enumerate(targets):
        row = {"id": item["id"], "set": item["set"], "file": item["file"], "role": role, **g}
        if k in pairs:
            row.update(found=True, err_ms=(events[pairs[k]] - g["t"]) * 1000, cause="")
        else:
            cause, det = classify_miss(g["t"], role, act, st, picks, free_events, claimed)
            row.update(found=False, cause=cause, own=det.get("own", cause), peak=det.get("peak"),
                       event_off_ms=det.get("event_off_ms"))
        if dsp_events is not None:
            row["dsp_found"] = k in dsp_pairs
        rows.append(row)
    bleed_t = [g["t"] for g in gt if g["group"] == "bleed"]
    info = {"retry": retry, "false": len(false), "n_events": len(events), "bleed_total": len(bleed_t),
            "bleed_hit": sum(1 for b in bleed_t if any(abs(e - b) <= TOL_S for e in free_events))}
    return rows, info


# ---------------------------------------------------------------- loading the run

def load_run(out, items=None, prefix=""):
    """The items with their kept events: [(item, act, dsp events, dsp events)]. `prefix` is the
    work folder of a gained run (`gain24`): the DSP events come from there."""
    out = Path(out)
    res = []
    for it in items or all_items():
        wd = out / "work" / prefix / it["id"]
        # Only DSP events are available now (main is model-free, #281 D16)
        ev = sorted(e["t"] for e in json.loads(
            (wd / "dsp" / "events.json").read_text())["events"])
        # No activation dump without the model path; return empty array with correct shape
        act = np.zeros((100, 5), dtype=np.float32)
        dsp = ev
        res.append((it, act, ev, dsp))
    return res


def classify_all(data):
    """All rows and the per-item info of `classify_item` over a loaded run."""
    rows, infos = [], {}
    for it, act, ev, dsp in data:
        r, info = classify_item(it, act, ev, dsp)
        rows += r
        infos[it["id"]] = info
    return rows, infos


def count_table(rows, keyf):
    """{key: {cause or 'found': n}} over `rows`, and the same for the misses the DSP fallback finds."""
    c, d = {}, {}
    for r in rows:
        k = keyf(r)
        cause = r["cause"] or "found"
        c.setdefault(k, {}).setdefault(cause, 0)
        c[k][cause] += 1
        if not r["found"] and r.get("dsp_found"):
            d.setdefault(k, {}).setdefault(cause, 0)
            d[k][cause] += 1
    return c, d


# ---------------------------------------------------------------- the dry run of a changed rule (no model rerun)

LEAD_S = 0.010   # a pick leads the refined event by 10 ms on median (found hits): the proxy event time of a pick


def family(item):
    """Which false-hit check a file belongs to."""
    if item["set"] == "drt":
        if item["file"].startswith("08"):
            return "noise"
        if item["file"].startswith("05"):
            return "bleed"
        return "clean"
    return item["set"].split("-")[1]


def file_lead(item, act, events):
    """The lead of a pick against its refined event in this file: the median of event minus the claimed pick over the
    found hits (the refinement moves the pick forward by the rise of the sample); `LEAD_S` when none was found."""
    role = item["role"]
    picks, st, _, _ = effective_picks(act, role)
    kept = sorted(f for c in ROLE_CLASSES[role] for f in st[c]["kept"])
    targets = [g["t"] for g in item["gt"] if g["group"] != "bleed"]
    pairs, _, _ = ck.match(targets, events, TOL_S)
    leads = []
    for _, j in pairs:
        f = claim(kept, events[j])
        if f is not None:
            leads.append(events[j] - f / FPS)
    return float(np.median(leads)) if leads else LEAD_S


def variant_events(item, act, dsp_events, v, lead=LEAD_S):
    """The event-time estimates of one file under the variant `v` (a dict): the picks of the changed rules, the role
    filter with `classes`, the minimum gap, each pick moved by `LEAD_S`; the DSP fallback's events are added when
    `dsp_far` (s) is set and none of the picks is that close."""
    role = item["role"]
    th = list(THRESHOLDS)
    for c in ROLE_CLASSES[role]:
        th[c] = th[c] * v.get("thr_scale", 1.0)
    classes = v.get("classes")
    picks, _, _, _ = effective_picks(act, role, th, v.get("rules"), classes, v.get("min_gap", MIN_GAP_S))
    cross = v.get("cross_min")
    if cross is not None:
        # a peak of another class counts for the role's class when the role's own column rises there too
        own = ROLE_CLASSES[role][0]
        picks = sorted({(f, c if c in ROLE_CLASSES[role] else own) for f, c in picks
                        if c in ROLE_CLASSES[role] or float(act[max(0, f - 3):f + 4, own].max()) >= cross})
    kept, _ = role_dedup(picks, role, v.get("min_gap", MIN_GAP_S), classes)
    est = [t + lead for t in kept]
    far = v.get("dsp_far")
    if far is not None and role in v.get("dsp_roles", ROLE_CLASSES):
        est += [d for d in dsp_events if all(abs(d - e) > far for e in est)]
    return sorted(est)


def score_events(item, est):
    """(found by group, false, bleed triggers) of the event times `est` against the file's ground truth."""
    targets = [g for g in item["gt"] if g["group"] != "bleed"]
    pairs, _, false = ck.match([g["t"] for g in targets], est, TOL_S)
    found = {}
    for k, _ in pairs:
        found[targets[k]["group"]] = found.get(targets[k]["group"], 0) + 1
    bleed = [g["t"] for g in item["gt"] if g["group"] == "bleed"]
    trig = sum(1 for j in false if any(abs(est[j] - b) <= TOL_S for b in bleed))
    return found, len(false), trig


VARIANTS = {
    "base": {},
    "thr80": {"thr_scale": 0.80},
    "thr65": {"thr_scale": 0.65},
    "thr50": {"thr_scale": 0.50},
    "avg5": {"rules": {"pre_avg": 5}},
    "avg3": {"rules": {"pre_avg": 3}},
    "avg_off": {"rules": {"avg_off": True}},
    "max11": {"rules": {"pre_max": 1, "post_max": 1}},
    "max00": {"rules": {"pre_max": 0, "post_max": 0}},
    "comb10": {"rules": {"combine_s": 0.01}},
    "comb10_gap10": {"rules": {"combine_s": 0.01}, "min_gap": 0.010},
    "role_any": {"classes": [0, 1, 2, 3, 4]},
    "cross05": {"cross_min": 0.05},
    "cross10": {"cross_min": 0.10},
    "dsp30": {"dsp_far": 0.030},
    "dsp08": {"dsp_far": 0.008},
    "dsp30_kick": {"dsp_far": 0.030, "dsp_roles": ["kick"]},
    "dsp30_snare": {"dsp_far": 0.030, "dsp_roles": ["snare"]},
    "dsp30_tom": {"dsp_far": 0.030, "dsp_roles": ["tom"]},
    "dsp08_snare": {"dsp_far": 0.008, "dsp_roles": ["snare"]},
    "cross05_dsp30_kick": {"cross_min": 0.05, "dsp_far": 0.030, "dsp_roles": ["kick"]},
    "thr65_avg5": {"thr_scale": 0.65, "rules": {"pre_avg": 5}},
    "thr65_dsp30": {"thr_scale": 0.65, "dsp_far": 0.030},
}


def dry_run(data, variants=None):
    """Every variant over every file: {variant: {"found": {group-key: n}, "false": {family: n}, "bleed": n,
    "targets": n}}. Keys are `<drt|syn|real>/<group or family>`."""
    variants = variants or VARIANTS
    leads = {it["id"]: file_lead(it, act, ev) for it, act, ev, _ in data}
    out = {}
    for name, v in variants.items():
        found, false, bleed, targets = {}, {}, 0, 0
        for it, act, ev, dsp in data:
            est = variant_events(it, act, dsp, v, leads[it["id"]])
            f, nfalse, trig = score_events(it, est)
            top = it["set"].split("-")[0]
            for g, n in f.items():
                found[f"{top}/{g}"] = found.get(f"{top}/{g}", 0) + n
            fam = family(it)
            key = f"{top}/{fam}"
            false[key] = false.get(key, 0) + nfalse
            bleed += trig
            targets += sum(1 for g in it["gt"] if g["group"] != "bleed")
        out[name] = {"found": found, "false": false, "bleed": bleed, "targets": targets, "variant": v}
    return out


# ---------------------------------------------------------------- what-if: a second onset inside a found hit's lobe

def read_mono(path):
    import soundfile as sf
    x, sr = sf.read(path, dtype="float64", always_2d=True)
    return x.mean(axis=1), sr


def step_onsets(x, sr, events, rise_db=5.0, start_s=0.006, stop_s=0.045, before_s=0.003, after_s=0.003, step_s=0.0005):
    """Extra onsets in the waveform: after each event (up to the next one), the earliest local maximum of the power
    ratio `mean x^2 [t, t + after] / mean x^2 [t - before, t]` that is at least `rise_db` dB, searched from `start_s`
    to `stop_s` after the event. A second drum hit inside the lobe of the first (the model gives one lobe for 5 to 35 ms)
    shows as such a step; the ratio is on the signal itself, so the 10 ms frame of the model does not limit it."""
    p = np.concatenate([[0.0], np.cumsum(x * x)])
    nb, na, st = int(before_s * sr), int(after_s * sr), max(1, int(step_s * sr))

    def power(a, b):
        a, b = max(0, a), min(len(x), b)
        return (p[b] - p[a]) / max(1, b - a) if b > a else 0.0

    out = []
    ev = sorted(events)
    for k, e in enumerate(ev):
        hi = min(e + stop_s, (ev[k + 1] - start_s) if k + 1 < len(ev) else e + stop_s)
        taus = np.arange(int((e + start_s) * sr), int(hi * sr), st)
        if len(taus) < 3:
            continue
        r = np.array([10 * math.log10((power(t, t + na) + 1e-12) / (power(t - nb, t) + 1e-12)) for t in taus])
        for i in range(1, len(r) - 1):
            if r[i] >= rise_db and r[i] >= r[i - 1] and r[i] > r[i + 1]:
                out.append(float(taus[i] / sr))
                break
    return out


SPLITS = {"split5": {"rise_db": 5.0}, "split6_a4": {"rise_db": 6.0, "after_s": 0.004, "before_s": 0.004},
          "split9_a4": {"rise_db": 9.0, "after_s": 0.004, "before_s": 0.004}}


def dry_run_split(data, splits=None):
    """The second-onset search on the real events of every file, scored like `dry_run`."""
    splits = splits or SPLITS
    out = {}
    cache = {}
    for name, kw in splits.items():
        found, false, bleed, targets, extra = {}, {}, 0, 0, 0
        for it, act, ev, dsp in data:
            if it["audio"] not in cache:
                cache[it["audio"]] = read_mono(it["audio"])
            x, sr = cache[it["audio"]]
            more = step_onsets(x, sr, ev, **kw)
            extra += len(more)
            f, nfalse, trig = score_events(it, sorted(ev + more))
            top = it["set"].split("-")[0]
            for g, n in f.items():
                found[f"{top}/{g}"] = found.get(f"{top}/{g}", 0) + n
            key = f"{top}/{family(it)}"
            false[key] = false.get(key, 0) + nfalse
            bleed += trig
            targets += sum(1 for g in it["gt"] if g["group"] != "bleed")
        out[name] = {"found": found, "false": false, "bleed": bleed, "targets": targets, "extra": extra, "variant": kw}
    return out


# ---------------------------------------------------------------- what-if: a gain ahead of the model (model rerun)

def gained_item(out, item, db):
    """A float copy of the item's audio with `db` dB of gain (not clipped), under `out/gain/g<db>/`; the item with the
    new path and the id `gain<db>/<id>`. The ground truth is the same."""
    import soundfile as sf
    x, sr = sf.read(item["audio"], dtype="float32", always_2d=True)
    path = Path(out) / "gain" / f"g{db:g}" / item["id"].replace("/", "__") / Path(item["audio"]).name
    path.parent.mkdir(parents=True, exist_ok=True)
    sf.write(path, (x * np.float32(10 ** (db / 20))).astype("float32"), sr, subtype="FLOAT")
    return {**item, "id": f"gain{db:g}/{item['id']}", "audio": str(path)}


def run_gain(out, db_list, jobs=2, only=None, log=print):
    """Rerun the DSP detector on gained copies of every kit file (the events and activations are kept)."""
    out = Path(out)
    tool = find_tool()
    items = all_items()
    if only:
        items = [i for i in items if re.search(only, i["id"])]
    work = [gained_item(out, i, db) for db in db_list for i in items]
    with ThreadPoolExecutor(max_workers=jobs) as ex:
        futs = [(i, ex.submit(run_item, tool, out, i, "dsp", "")) for i in work]
        for i, f in futs:
            f.result()
            log(f"ran {i['id']}")
    return len(work)


def gain_scores(out, db_list):
    """{db: dry-run-shaped totals} from the real events of the gained runs (not a replay)."""
    out = Path(out)
    res = {}
    for db in db_list:
        found, false, bleed, targets = {}, {}, 0, 0
        for it in all_items():
            ev = sorted(e["t"] for e in json.loads(
                (out / "work" / f"gain{db:g}" / it["id"] / "dsp" / "events.json").read_text())["events"])
            f, nfalse, trig = score_events(it, ev)
            top = it["set"].split("-")[0]
            for g, n in f.items():
                found[f"{top}/{g}"] = found.get(f"{top}/{g}", 0) + n
            key = f"{top}/{family(it)}"
            false[key] = false.get(key, 0) + nfalse
            bleed += trig
            targets += sum(1 for g in it["gt"] if g["group"] != "bleed")
        res[f"gain{db:g}"] = {"found": found, "false": false, "bleed": bleed, "targets": targets}
    return res


# ---------------------------------------------------------------- tables

def md_table(head, body):
    lines = ["| " + " | ".join(head) + " |", "|" + "|".join("---" for _ in head) + "|"]
    lines += ["| " + " | ".join(str(c) for c in row) + " |" for row in body]
    return "\n".join(lines)


def level_bin(level):
    """A level in dBFS as its 10 dB bin, `-30 to -20`; the sweep's -6, -20 and -35 land in three bins."""
    hi = math.ceil(level / 10.0) * 10
    return f"{int(hi) - 10:d} to {int(hi):d}"


def cause_cell(n, d):
    return "-" if not n else (f"{n} ({d})" if d else str(n))


def cause_table(rows, keyf, key_head):
    """Found and the nine causes per key; a cell `n (d)`: n misses of which d the DSP fallback finds."""
    c, d = count_table(rows, keyf)
    body = []
    for k in sorted(c):
        n = sum(c[k].values())
        body.append([k if isinstance(k, str) else " / ".join(map(str, k)), n, c[k].get("found", 0)]
                    + [cause_cell(c[k].get(x, 0), d.get(k, {}).get(x, 0)) for x in CAUSES])
    tot = {x: sum(c[k].get(x, 0) for k in c) for x in ["found"] + CAUSES}
    totd = {x: sum(d.get(k, {}).get(x, 0) for k in d) for x in CAUSES}
    body.append(["**all**", sum(tot.values()), tot["found"]] + [cause_cell(tot[x], totd[x]) for x in CAUSES])
    return md_table([key_head, "hits", "found"] + CAUSES, body)


def replay_check(data):
    """The replay against the app: files where the replayed event count (role filter, minimum gap, the retry) equals the
    app's, and the largest difference (the refinement's second dedup can drop a few)."""
    same, diffs = 0, []
    for it, act, ev, _ in data:
        picks, _, _, _ = effective_picks(act, it["role"])
        n = len(role_dedup(picks, it["role"])[0])
        if n == len(ev):
            same += 1
        else:
            diffs.append((it["id"], n, len(ev)))
    return {"files": len(data), "same": same, "diffs": diffs}


def fix_table(res, groups=None):
    """One row per variant: found per group and the false hits per family."""
    groups = groups or ["drt/ghost", "drt/flam_main", "drt/pair_second", "drt/ramp", "syn/main", "real/main", "syn/pair_second",
                        "real/pair_second"]
    fams = ["drt/noise", "drt/bleed", "drt/clean", "syn/noise", "syn/bleed", "syn/clean", "real/noise", "real/bleed",
            "real/clean"]
    head = ["variant"] + [g.replace("pair_second", "pair2") for g in groups] + ["false " + f for f in fams] + ["bleed trig"]
    body = []
    for name, r in res.items():
        body.append([name] + [r["found"].get(g, 0) for g in groups] + [r["false"].get(f, 0) for f in fams] + [r["bleed"]])
    return md_table(head, body)


def tables_md(rows, data, dry=None):
    """The whole `tables.md`: causes per file and group, and the dry run."""
    drt = [r for r in rows if r["set"] == "drt"]
    cal = [r for r in rows if r["set"] != "drt"]
    out = ["# P4-10 stage 1: found and missed hits with the cause of each miss", "",
           "A cell `n (d)`: n misses, of which d the DSP fallback finds (one to one, +-10 ms). Pipeline order of the causes: "
           "blind, threshold, lobe, average, not_max, combine, role (other class fired), mingap, refine, offset.", ""]
    out += ["## Owner's DRT kit, per file", "", cause_table(drt, lambda r: r["file"], "file"), ""]
    out += ["## Owner's DRT kit, per hit group", "", cause_table(drt, lambda r: r["group"], "group"), ""]
    out += ["## P4-11 kit (syn and real), per set and role", "",
            cause_table(cal, lambda r: f"{r['set']} / {r['role']}", "set / role"), ""]
    out += ["## P4-11 kit, per hit group (main = the level sweep -6, -20, -35 dBFS; pair = close pairs 20 to 60 ms)", "",
            cause_table(cal, lambda r: r["group"], "group"), ""]
    out += ["## P4-11 kit, main hits by peak level (10 dB bins, dBFS)", "",
            cause_table([r for r in cal if r["group"] == "main"], lambda r: level_bin(r["level"]), "level"), ""]
    rc = replay_check(data)
    out += ["## Replay check", "", f"The Python replay of the shipped peak rules gives the app's event count in {rc['same']} of "
            f"{rc['files']} files; the others: " + (", ".join(f"{i} {a} against {b}" for i, a, b in rc["diffs"]) or "none"), ""]
    if dry:
        for title, key in (("Dry run on the kept activations (peak rules, role, DSP rescue)", "peak"),
                           ("Gain ahead of the model (model rerun on gained copies)", "gain"),
                           ("Second onset inside a found hit's lobe (waveform step, on the real events)", "split")):
            if dry.get(key):
                out += [f"## {title}", "", fix_table(dry[key]), ""]
    return "\n".join(out)


# ---------------------------------------------------------------- the classify and dryrun commands

def write_rows(out, rows):
    out = Path(out)
    (out / "causes.json").write_text(json.dumps(rows))
    cols = ["id", "set", "file", "role", "group", "kind", "t", "level", "gap_ms", "found", "cause", "own", "peak", "err_ms",
            "event_off_ms", "dsp_found"]
    with open(out / "causes.csv", "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=cols, extrasaction="ignore")
        w.writeheader()
        w.writerows(rows)


def do_classify(out, log=print):
    out = Path(out)
    data = load_run(out)
    rows, infos = classify_all(data)
    write_rows(out, rows)
    (out / "infos.json").write_text(json.dumps(infos))
    dry = json.loads((out / "dryrun.json").read_text()) if (out / "dryrun.json").is_file() else None
    (out / "tables.md").write_text(tables_md(rows, data, dry))
    miss = sum(1 for r in rows if not r["found"])
    log(f"{len(rows)} hits, {len(rows) - miss} found, {miss} missed; {out / 'tables.md'}")
    return rows


def do_dryrun(out, gains=(12, 24, 36), log=print):
    out = Path(out)
    data = load_run(out)
    dry = {"peak": dry_run(data), "split": dry_run_split(data)}
    have = [db for db in gains if (out / "work" / f"gain{db:g}").is_dir()]
    if have:
        dry["gain"] = gain_scores(out, have)
    (out / "dryrun.json").write_text(json.dumps(dry))
    log(f"dry run: {len(dry['peak'])} peak variants, {len(dry['split'])} split variants, gains {have}")
    return dry


# ---------------------------------------------------------------- command line

def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["run", "gain", "classify", "dryrun", "all"])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--only", help="regex on the item id for run and gain")
    ap.add_argument("--gains", default="12,24,36", help="dB list for gain")
    a = ap.parse_args(argv)
    out = Path(a.out)
    gains = [float(x) for x in a.gains.split(",") if x]
    if a.cmd in ("run", "all"):
        run_all(out, jobs=a.jobs, only=a.only)
    if a.cmd in ("gain", "all"):
        run_gain(out, gains, jobs=a.jobs, only=a.only)
    if a.cmd in ("dryrun", "all"):
        do_dryrun(out, gains)
    if a.cmd in ("classify", "all"):
        do_classify(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
