"""P4-22 (diagnosis only): why the kick fixes of the P4-21b yardstick are there (426 of 1361; 363 are "moved").

    uv run --with numpy --with soundfile --with mido python tools/kick_onset.py run      # the app on k in, k out, k sub (and the held-out sessions)
    uv run --with numpy --with soundfile --with mido python tools/kick_onset.py files    # question 1: the reference files, the three-cornered hat
    uv run --with numpy --with soundfile --with mido python tools/kick_onset.py moved    # question 1: moved kicks, size, sign, audio class
    uv run --with numpy --with soundfile --with mido python tools/kick_onset.py hits     # question 2: level, spacing, double strokes
    uv run --with numpy --with soundfile --with mido python tools/kick_onset.py missed   # question 3: missed and false kicks
    uv run --with numpy --with soundfile --with mido python tools/kick_onset.py rules    # question 4: rule candidates
    uv run --with numpy --with soundfile --with mido python tools/kick_onset.py all

Input: `~/.cache/drumspike/fix-metric-p421b/` (`refs.json`, `run/<song>_default.json` and `_before.json`; written by
`tools/fix_metric.py`). Nothing under `crates/` or `static/` changes. The sessions in `~/.cache/drumspike/sessions` are read
only. Outputs: `~/.cache/drumspike/kick-onset/` (`work/` the kept events and activations of the kick mics, `tracks.json`,
`report.md`). The only app run is `run`: `service_tool cli run` on each kick mic (`DRUMSPIKE_CALIB_TOOL` points to a release
tool).

Method (as `tools/tom_fixes.py`, P4-20 question 3): a moved kick is a hit more than 1 ms from its reference after the mic's
offset. The audio between the two times says which side is wrong. A kick reference is a trigger track or a sample file, not
the truth. A session has several such files of the same strokes (a trigger track and sample layers). Each file has its own
onset error against the mic audio. The three-cornered hat gives the jitter of each estimator from the pairwise differences
of aligned onset series, with no estimator taken as the truth.
"""
import argparse
import json
import os
import shutil
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fix_metric as fm  # noqa: E402
import firm_fixes as ff  # noqa: E402
import miss_causes as mc  # noqa: E402
import onset_guard as og  # noqa: E402
import tom_fixes as tf  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SRC = Path.home() / ".cache" / "drumspike" / "fix-metric-p421b"
OUT = Path.home() / ".cache" / "drumspike" / "kick-onset"
SONGS = ["Session A", "Session B", "Session C", "Session H", "Session F", "Session G",
         "Session D"]
ROLE = "kick"
MICS = ["k in", "k out", "k sub"]
MOVE_S = fm.MOVE_S

# the reference files of each song: `yard` is what the yardstick's `refs.json` uses (the union of its onsets within 20 ms,
# the earliest of a cluster), `files` all the files of the same strokes (a trigger track and sample layers)
FILES = {
    "Session A": {"yard": ["k sampl trig_01"], "files": ["k sampl trig_01", "k sampl savior_01", "k sampl sd3_01"]},
    "Session B": {"yard": ["k trigger_01"], "files": ["k trigger_01", "k samp savior_01", "k samp sd3_01"]},
    "Session C": {"yard": ["k samp 1_02"], "files": ["k samp 1_02", "k samp 2_02"]},
    "Session H": {"yard": ["k samp 1_01", "k samp 2_01", "k samp 3_01"], "files": ["k samp 1_01", "k samp 2_01", "k samp 3_01"]},
    "Session F": {"yard": ["k samp 1_01", "k samp 2_01", "k samp 3_01"],
                         "files": ["k samp 1_01", "k samp 2_01", "k samp 3_01"]},
    "Session G": {"yard": ["k triggerr_03"], "files": ["k triggerr_03", "k samp savior_03", "k samp sd3_03"]},
    "Session D": {"yard": ["k trigger"], "files": ["k trigger", "k samp saviour", "k samp sd3"]},
}
# sessions the yardstick leaves out (`fix_metric.SKIPPED`): their kick mics and reference files, for a held-out check
HELD = {
    "Session E": {"dir": "Session E", "kick": "k in-AtA2", "out": "k out-AtA2", "sub": "k sub-AtA2",
                         "files": ["k trigger-AtA2", "k sample savior-AtA2", "k sample sd3-AtA2"]},
    "Session I": {"dir": "Session I", "kick": "k in_01", "out": "k out_01", "sub": "k sub_01",
                                   "files": ["k samp 1_01", "k samp 2_01", "k samp 3_01"]},
}
FIT = ["Session A", "Session C", "Session F"]      # the fit songs of the rules; the other four are held out
HELD_SONGS = [s for s in SONGS if s not in FIT]

CLEAN_GAP_S = 0.300       # a clean onset: no other hit this close before or after
CLEAN_DB = -6.0           # and a level this close to the typical hit or louder
CLEAN_JUMP_DB = 30.0      # and sharp: within 2.5 ms of the onset the audio stands this far over the floor before the hit
JUMP_S = 0.0025
TRIM_MS = 3.0             # the hat's trim: a difference this far from the median is an outlier, not jitter
MIN_COVER = 0.5           # a series that matches fewer of the anchor hits than this is left out of the hat
GAP_BINS = [("under 120 ms", 0.0, 0.12), ("120 to 250 ms", 0.12, 0.25), ("250 to 600 ms", 0.25, 0.6),
            ("600 ms or more", 0.6, 1e9)]
HAT_MIN_HITS = 20         # fewer hits in common: no jitter estimate
SHIFT_GRID_MS = [round(v * 0.25, 2) for v in range(-12, 13)]
# the back-track variants replayed from the version 2 time: name -> (peak dB under the hit, floor margin dB); the shipped walk is
# (33, 6). A lower peak margin puts the walk's threshold higher: the foot comes later. The replay of the shipped walk equals the
# app's times (`replay_exact`).
WALKS = {"shipped": (None, None), "peak 20": (20.0, None), "peak 25": (25.0, None), "peak 27": (27.0, None),
         "peak 29": (29.0, None), "peak 31": (31.0, None), "peak 29 floor 9": (29.0, 9.0), "peak 37": (37.0, None), "peak 41": (41.0, None), "peak 45": (45.0, None), "floor 3": (None, 3.0),
         "floor 9": (None, 9.0), "floor 12": (None, 12.0), "peak 25 floor 9": (25.0, 9.0), "peak 41 floor 3": (41.0, 3.0)}


# a back-track variant applied only to a hit whose onset is soft (`jump_db` under the limit at the app's time)
COND_WALKS = [("peak 25", 26.0), ("peak 25", 30.0), ("peak 29", 30.0)]


def tid(name, mic):
    return fm.track_name(name) + "_" + fm.track_name(mic)


# ---------------------------------------------------------------- sources (read only)

def load_inputs(src=SRC):
    """(refs, default runs, before runs): `refs.json` and the P4-16 runs (`before` is refinement version 2)."""
    src = Path(src)
    refs = json.loads((src / "refs.json").read_text())
    runs = {s: json.loads((src / "run" / f"{fm.track_name(s)}_default.json").read_text()) for s in refs}
    before = {s: json.loads((src / "run" / f"{fm.track_name(s)}_before.json").read_text()) for s in refs}
    return refs, runs, before


def kick_mic_stems(stem):
    """The stems of the three kick mics of a session from the stem of `k in`: `k in_02` gives `k out_02` and `k sub_02`
    (a `k in` without a suffix gives `k out`, `k sub`)."""
    if not stem.startswith("k in"):
        raise ValueError(f"not a kick inside mic: {stem!r}")
    rest = stem[len("k in"):]
    return {"k in": stem, "k out": "k out" + rest, "k sub": "k sub" + rest}


def session_cfg(song):
    """(config with `dir`, stems of the kick mics, reference file stems, yardstick file stems) of a yardstick or held-out song."""
    if song in HELD:
        h = HELD[song]
        return ({"dir": h["dir"]}, {"k in": h["kick"], "k out": h["out"], "k sub": h["sub"]}, h["files"], h["files"][:1])
    cfg = fm.songs()[song]
    return cfg, kick_mic_stems(cfg["mics"]["k in"][0]), FILES[song]["files"], FILES[song]["yard"]


# ---------------------------------------------------------------- run: the app on every kick mic

def do_run(out, log=print):
    """One analysis of every kick mic of the seven songs and the two held-out sessions through `service_tool cli run` (role
    kick, default settings); the events go to `<out>/work/<id>/model/`. Resumes: a track with an
    `events.json` is kept. (#281 D16: the activation replay moved to the model branch; the folder name stays.)"""
    out = Path(out)
    names = SONGS + list(HELD)
    todo = [(s, m) for s in names for m in MICS if not (out / "work" / tid(s, m) / "model" / "act.bin").is_file()]
    todo_v2 = [h for h in HELD if not (out / "work" / (tid(h, "k in") + "_v2") / "model" / "events.json").is_file()]
    if not todo and not todo_v2:
        log("all kick tracks are there")
        return
    tool = mc.find_tool()
    model = "none"           # the agreed no-op: the DSP detector runs
    if not tool.is_file():
        raise SystemExit(f"need the release service_tool ({tool})")
    (out / "audio").mkdir(parents=True, exist_ok=True)
    try:
        for song, mic in todo:
            cfg, stems, _, _ = session_cfg(song)
            i = tid(song, mic)
            link = out / "audio" / f"{i}.wav"
            if link.is_symlink() or link.exists():
                link.unlink()
            src = fm.audio_path(cfg, stems[mic])
            if not src.is_file():
                log(f"missing {src}")
                continue
            link.symlink_to(src)
            tf.wait_for_lock(log)
            fm.LOCK.write_text(f"kick_onset run {i}\n")
            try:
                mc.run_item(tool, out, {"id": i, "audio": str(link), "role": ROLE, "note": mc.ROLE_NOTE[ROLE]}, "model", model)
            finally:
                fm.LOCK.unlink(missing_ok=True)
            log(f"ran {i}")
        for song in HELD:       # the version 2 time of the held-out kick (the walk's start), refinement rule 2
            i = tid(song, "k in") + "_v2"
            if (out / "work" / i / "model" / "events.json").is_file():
                continue
            cfg, stems, _, _ = session_cfg(song)
            link = out / "audio" / f"{i}.wav"
            if link.is_symlink() or link.exists():
                link.unlink()
            link.symlink_to(fm.audio_path(cfg, stems["k in"]))
            tf.wait_for_lock(log)
            fm.LOCK.write_text(f"kick_onset run {i}\n")
            keep = os.environ.get("DRUMSPIKE_REFINE_RULE")
            os.environ["DRUMSPIKE_REFINE_RULE"] = "2"
            try:
                mc.run_item(tool, out, {"id": i, "audio": str(link), "role": ROLE, "note": mc.ROLE_NOTE[ROLE]}, "model", model)
            finally:
                if keep is None:
                    os.environ.pop("DRUMSPIKE_REFINE_RULE", None)
                else:
                    os.environ["DRUMSPIKE_REFINE_RULE"] = keep
                fm.LOCK.unlink(missing_ok=True)
            log(f"ran {i}")
    finally:
        fm.LOCK.unlink(missing_ok=True)
        shutil.rmtree(out / "run-home", ignore_errors=True)
        shutil.rmtree(out / "run-projects", ignore_errors=True)


def read_events(out, name, mic):
    """The kept events of one kick track of `do_run`: [{t, v, rise}]; [] when the track is not there."""
    p = Path(out) / "work" / tid(name, mic) / "model" / "events.json"
    return json.loads(p.read_text())["events"] if p.is_file() else []


# ---------------------------------------------------------------- features (pure; the tests cover these)

def union_onsets(lists, tol=fm.TOL):
    """The onsets of several files as one reference (`fm.read_reference`): sorted, and an onset within `tol` of the last kept
    one is the same hit (the earliest of a cluster stays)."""
    merged = []
    for t in sorted(t for lst in lists for t in lst):
        if not merged or t - merged[-1] >= tol:
            merged.append(float(t))
    return merged


def pair_errors(ref, est, offset, tol=fm.TOL):
    """[(ref index, event index, error s)] of the one to one match after `offset` (error = event - shifted reference)."""
    sh = [float(r) + offset for r in ref]
    return [(i, j, float(est[j]) - sh[i]) for i, j in fm.match(sh, est, tol)]


def moved_count(errs, move_s=MOVE_S):
    """The hits more than `move_s` from the reference."""
    return int(np.sum(np.abs(np.asarray(errs, float)) > move_s + 1e-9))


def size_sign(errs_s):
    """{(size, sign): count} of the moved hits: size "1 to 2" / "2 to 5" / "over 5" (`tom_fixes.size_bin`), sign "late"
    (the app after the reference) or "early". Hits within 1 ms are not counted."""
    out = {}
    for e in errs_s:
        if abs(e) <= MOVE_S + 1e-9:
            continue
        k = (tf.size_bin(e * 1000.0), "late" if e > 0 else "early")
        out[k] = out.get(k, 0) + 1
    return out


def trimmed_var_ms2(d_s, trim_ms=TRIM_MS):
    """The variance (ms squared) of a difference series in s without the outliers (a difference more than `trim_ms` from the
    median is dropped: a missed hit matched to a neighbour, a double). The onsets of a reference file lie on a 0.5 ms grid, so a
    MAD collapses to 0 there; the trimmed variance does not."""
    d = np.asarray(d_s, float) * 1000.0
    d = d[~np.isnan(d)]
    if len(d) < 3:
        return float("nan")
    d = d[np.abs(d - np.median(d)) <= trim_ms]
    return float(np.var(d)) if len(d) >= 3 else float("nan")


def hat_jitter(series):
    """The three-cornered hat: the jitter (ms, one sigma) of each estimator of `series` ({name: onsets (s) of the same hits,
    same order}; at least three). The variance of the difference of two estimators is the sum of their variances (a
    constant offset between them does not matter); the variances come from a least squares fit to all pairs, never below 0.
    A name with fewer than `HAT_MIN_HITS` hits in common gets nan."""
    names = list(series)
    n = len(next(iter(series.values()))) if series else 0
    if len(names) < 3 or n < HAT_MIN_HITS:
        return {k: float("nan") for k in names}
    rows, y = [], []
    for a in range(len(names)):
        for b in range(a + 1, len(names)):
            v = trimmed_var_ms2(np.asarray(series[names[a]], float) - np.asarray(series[names[b]], float))
            if np.isnan(v):
                continue
            r = np.zeros(len(names))
            r[a] = r[b] = 1.0
            rows.append(r)
            y.append(v)
    sol = np.linalg.lstsq(np.asarray(rows), np.asarray(y), rcond=None)[0]
    return {k: float(np.sqrt(max(v, 0.0))) for k, v in zip(names, sol)}


def jump_db(x, sr, t, floor=None):
    """How sharp the onset at `t` is: the peak of |x| in the 2.5 ms from `t` over the floor before the hit (dB; the floor is
    `tom_fixes.floor_db`). A kick's peak comes 6 to 11 ms after the foot, so the tom card's "peak within 1.5 ms" never holds
    for it; a sharp kick onset is a high level a few samples after the foot."""
    i0, i1 = max(0, int(t * sr)), min(len(x), int((t + JUMP_S) * sr))
    if i1 <= i0:
        return -180.0
    return ff._db(np.abs(x[i0:i1]).max()) - (tf.floor_db(x, sr, t) if floor is None else floor)


def clean_row(gap_prev_s, gap_next_s, rel_db, jump):
    """A clean onset: isolated (no hit within 300 ms before or after), strong (level within 6 dB of the typical hit or
    louder), sharp (`jump_db` 30 dB or more)."""
    return gap_prev_s >= CLEAN_GAP_S and gap_next_s >= CLEAN_GAP_S and rel_db >= CLEAN_DB and jump >= CLEAN_JUMP_DB


def gap_bin(gap_s):
    """The name of the spacing bin of a gap to the previous hit."""
    for name, lo, hi in GAP_BINS:
        if lo <= gap_s < hi:
            return name
    return GAP_BINS[-1][0]


def level_bins(levels, n=3):
    """The inner edges (dB) that cut the levels into `n` groups of equal size."""
    return [float(np.percentile(levels, 100.0 * k / n)) for k in range(1, n)]


def bin_index(v, edges):
    """The group of a level against the edges: 0 is the quietest."""
    return int(sum(v >= e for e in edges))


def level_offsets(errs, levels, n=3):
    """(edges, median error per level group, moved after each group's median is taken off): does the offset depend on the
    hit's level? One median per group; a group without hits gets 0."""
    e = np.asarray(errs, float)
    edges = level_bins(levels, n)
    idx = np.asarray([bin_index(v, edges) for v in levels])
    med = [float(np.median(e[idx == k])) if np.any(idx == k) else 0.0 for k in range(n)]
    fixed = e - np.asarray([med[k] for k in idx])
    return edges, med, moved_count(fixed)


def two_clusters(errs_s, levels, cut_ms=0.4):
    """Does the error come in two groups that follow the order of the hits, not the level? The errors (s) are cut at the
    median plus `cut_ms`; returns {late share, gap ms between the group medians, lag-1 correlation of the group sequence
    (negative: the groups alternate), correlation of the error with the level}; nan where a group is empty or too short."""
    e = (np.asarray(errs_s, float) - np.median(errs_s)) * 1000.0
    late = e > cut_ms
    out = {"late_share": float(late.mean()) if len(e) else float("nan"), "gap_ms": float("nan"), "lag1": float("nan"),
           "corr_level": float("nan")}
    if late.any() and (~late).any():
        out["gap_ms"] = float(np.median(e[late]) - np.median(e[~late]))
        if len(e) > 2:
            out["lag1"] = float(np.corrcoef(late[:-1].astype(float), late[1:].astype(float))[0, 1])
    if len(e) > 2 and np.std(levels) > 0 and np.std(e) > 0:
        out["corr_level"] = float(np.corrcoef(e, levels)[0, 1])
    return out


def best_shift(errs, grid_ms=None):
    """(shift ms, moved hits at that shift) that gives the fewest moved hits; ties go to the smaller shift."""
    grid = SHIFT_GRID_MS if grid_ms is None else grid_ms
    e = np.asarray(errs, float)
    best = min(grid, key=lambda g: (moved_count(e + g / 1000.0), abs(g)))
    return float(best), moved_count(e + best / 1000.0)


def two_sided(before, after, move_s=MOVE_S):
    """(moved after, right hits made wrong, wrong hits made right) for the errors of the same hits before and after a rule:
    the two-sided rule counts a correct hit that the rule moves out of the 1 ms window."""
    b = np.abs(np.asarray(before, float)) > move_s + 1e-9
    a = np.abs(np.asarray(after, float)) > move_s + 1e-9
    return int(a.sum()), int((a & ~b).sum()), int((b & ~a).sum())


def fit_line(levels, errs):
    """(slope s per dB, intercept s) of the least squares line error against level."""
    if len(levels) < 2 or np.ptp(levels) == 0:
        return 0.0, float(np.median(errs)) if len(errs) else 0.0
    a, b = np.polyfit(np.asarray(levels, float), np.asarray(errs, float), 1)
    return float(a), float(b)


pick_fair = fm.pick_fair      # one choice for the yardstick (P4-16c) and the diagnosis


def side_class(file_errs, move_s=MOVE_S):
    """Which side a hit moved against the yardstick belongs to, from the app's error against every reference file of the
    session (each with its own offset; None where a file has no hit there): `ref_file` when some file agrees with the app
    within 1 ms (the yardstick's file is the odd one out), `app` when no file agrees (every reference says the app is off),
    `unknown` when no file has the hit."""
    e = [abs(v) for v in file_errs if v is not None]
    if not e:
        return "unknown"
    return "ref_file" if min(e) <= move_s + 1e-9 else "app"


# ---------------------------------------------------------------- the misses and the false hits (pure)

MISS_CLASSES = ["silent", "no_stroke", "outside_20ms", "merged", "threshold", "blind", "average", "lobe", "role", "other_stage"]
FALSE_CLASSES = ["layer", "witness", "snare_ref", "double", "ring", "no_stroke", "real_stroke"]
FALSE_TEXT = {
    "layer": "a sample layer of the session has an onset within 3 ms: a real stroke the reference lacks",
    "witness": "the own mic and `k out` both show a stroke and no snare reference hit lies near: a real stroke",
    "snare_ref": "a snare reference hit within 12 ms: snare bleed in the kick mic",
    "double": "12 to 60 ms after a matched hit: a second stroke or the beater rebound",
    "ring": "60 ms to 1 s after a reference hit: the decay of the earlier hit",
    "no_stroke": "the own mic shows no stroke there (rise under 10 dB): a false pick",
    "real_stroke": "the own mic shows a stroke, no second file confirms it"}


def miss_class(rise_own, nearest_false_s, merged, cause, silent=False, rise_min=ff.RISE_MIN_DB):
    """The class of one missed kick, the first that fits: `silent` (the mic file is digital silence there), `no_stroke`
    (the own mic shows no stroke: a reference error), `outside_20ms` (an unmatched output hit 20 to 50 ms away: found, too
    far to match), `merged` (a matched output hit within 20 ms belongs to another reference hit), else the stage `cause` of
    `miss_causes` (threshold, blind, average, lobe, role); the rest is `other_stage`."""
    if silent:
        return "silent"
    if rise_own < rise_min:
        return "no_stroke"
    if fm.TOL < nearest_false_s <= tf.MERGE_S:
        return "outside_20ms"
    if merged:
        return "merged"
    return cause if cause in ("threshold", "blind", "average", "lobe", "role") else "other_stage"


def false_class(layer, witness, snare_near_s, after_matched_s, rise_own, since_ref_s, rise_min=ff.RISE_MIN_DB,
                bleed_s=fm.BLEED_S, double_s=tf.DOUBLE_S, ring_s=tf.RING_S):
    """The class of one false kick, the first that fits: `layer` (a sample layer confirms it), `snare_ref` (a snare reference
    hit within 12 ms), `witness` (own mic and `k out` show a stroke), `double` (12 to 60 ms after a matched hit), `ring` (up to
    1 s after a reference hit), `no_stroke` (own mic rise under 10 dB), else `real_stroke`."""
    if layer:
        return "layer"
    if snare_near_s <= bleed_s:
        return "snare_ref"
    if witness:
        return "witness"
    if double_s[0] < after_matched_s <= double_s[1]:
        return "double"
    if rise_own < rise_min:
        return "no_stroke"
    if since_ref_s <= ring_s:
        return "ring"
    return "real_stroke"


def kick_stage_causes(state, act):
    """{missed index: (cause, activation peak)} from the kept activation (`miss_causes.classify_miss`, role kick)."""
    picks, st, _, _ = mc.effective_picks(act, ROLE)
    classes = mc.ROLE_CLASSES[ROLE]
    own_kept = sorted(f for c in classes for f in st[c]["kept"])
    est = state["est"]
    used = {j for _, j in state["pairs"]}
    claimed = [f for f in (mc.claim(own_kept, est[j]) for j in used) if f is not None]
    free = [e for j, e in enumerate(est) if j not in used]
    out = {}
    for i in state["missed"]:
        cause, det = mc.classify_miss(state["shifted"][i], ROLE, act, st, picks, free, claimed)
        out[i] = (cause, float(det.get("peak", 0.0)))
    return out


# ---------------------------------------------------------------- the analysis of one track (reads audio)

def audio_between(x, sr, t_est, t_ref):
    """The audio facts of a hit that lies more than 1 ms from its reference, as `tom_fixes.moved_audio`: the level at the
    earlier of the two times, the floor before it, the deepest dip of the envelope between the two times (dB)."""
    te, tl = min(t_est, t_ref), max(t_est, t_ref)
    lv = tf.envelope_blocks(x, sr, te - tf.ENV_BLOCK_S, max(tl, te + tf.ENV_BLOCK_S))
    return {"m_db": tf.motion_db(x, sr, te), "floor_db": tf.floor_db(x, sr, te), "dip_db": tf.max_dip_db(lv[1:])}


def replay_walk(x, sr, t2, prev_t, next_t, peak_db=None, floor_margin_db=None):
    """The version 3 back-track again from the version 2 time `t2` (the walk's own start; `onset_guard.walk`, a port of
    `refine.rs` `first_motion`) with another peak margin `peak_db` (shipped 33) and floor margin `floor_margin_db` (shipped
    6). `prev_t`, `next_t`: the neighbouring output times (None at the ends). The foot in s; `t2` when the walk keeps it."""
    keep = (og.FM_PEAK_DB, og.FM_FLOOR_DB)
    try:
        if peak_db is not None:
            og.FM_PEAK_DB = peak_db
        if floor_margin_db is not None:
            og.FM_FLOOR_DB = floor_margin_db
        pm = int((prev_t + t2) / 2 * sr) if prev_t is not None else None
        nm = int((t2 + next_t) / 2 * sr) if next_t is not None else None
        foot, how = og.walk(x, int(round(t2 * sr)), sr, pm, nm)
    finally:
        og.FM_PEAK_DB, og.FM_FLOOR_DB = keep
    return foot / sr if how == "moved" else t2


def hit_rows(state, events, x, sr, typical_db, all_times, t_v2=None):
    """One row per matched hit of the kick mic: error (s), level against the typical hit (dB), attack (ms), gaps to the
    neighbouring hits (s), the app's `v`, the audio at the two times (`tom_fixes.moved_audio`), the class of a moved hit and
    the change of the time (s) under each back-track variant of `WALKS` (`replay`, from the version 2 time `t_v2` when it is known)."""
    est, sh = state["est"], state["shifted"]
    mv = tf.moved_audio(state, x, sr)
    rows = []
    for (i, j), a in zip(state["pairs"], mv):
        t = est[j]
        prev = est[j - 1] if j > 0 else None
        nxt = est[j + 1] if j + 1 < len(est) else None
        err = t - sh[i]
        row = {"i": i, "j": j, "t": t, "err": err, "rel_db": ff.peak_db(x, sr, t) - typical_db, "jump_db": jump_db(x, sr, t, a["floor_db"]),
               "gap_prev_s": t - prev if prev is not None else 9.0, "gap_next_s": nxt - t if nxt is not None else 9.0,
               "v": events[j].get("v"), **a}
        if t_v2 is not None:
            row["t_v2"] = t_v2[j]
            row["replay"] = {n: replay_walk(x, sr, t_v2[j], prev, nxt, *p) - t for n, p in WALKS.items()}
        row["clean"] = clean_row(row["gap_prev_s"], row["gap_next_s"], row["rel_db"], row["jump_db"])
        if abs(err) > MOVE_S + 1e-9:
            row["moved_class"] = tf.moved_class(err, a["m_db"], a["floor_db"], a["dip_db"])
        rows.append(row)
    return rows


def mic_counts(ref, events, offset=None):
    """The P4-16 fixes of one mic against one reference: {offset, matched, false, missed, moved, errs}."""
    off = fm.track_offset(ref, events) if offset is None else offset
    sc = fm.score_track(ref, events, off)
    return {"off_ms": off * 1000.0, "n_ref": sc["n_ref"], "n_out": sc["n_out"], "matched": sc["matched"], "false": sc["false"],
            "missed": sc["missed"], "moved": sc["moved"], "errs": sc["errs"]}


def file_onsets(cfg, stems):
    """{stem: onsets (s)} of the reference files that exist (`fm.read_reference`, one file each; the onset gap of the
    yardstick, 20 ms, merges the fast doubles: the same rule as `refs.json`)."""
    out = {}
    for s in stems:
        x, sr = fm.read_stem(cfg, s)
        if x is None:
            continue
        on, _ = fm.read_reference(cfg, [s])
        out[s] = on
    return out


def align(files_on, events_t, tol=fm.TOL, min_cover=MIN_COVER):
    """The same hits in every estimator: `files_on` {name: onsets (s)}, `events_t` {name: output times (s)}; the first entry of
    `events_t` anchors the hits. Each series is the nearest entry within `tol` of the anchor hit after the series' own median
    offset to the anchor, and that constant latency is taken off: the series are on the anchor's clock, so a difference of
    two series is jitter only. A series that matches fewer than
    `min_cover` of the anchor hits is left out (a quiet mic the app hears twice); a hit that a kept series lacks is dropped.
    Returns {name: array} in the anchor's order."""
    anchor = np.asarray(sorted(next(iter(events_t.values()))), float)
    series = {**{k: np.asarray(sorted(v), float) for k, v in files_on.items()},
              **{k: np.asarray(sorted(v), float) for k, v in events_t.items()}}
    cols = {}
    for k, arr in series.items():
        if not len(arr) or not len(anchor):
            continue
        d = []
        for a in anchor:
            c = arr[np.argmin(np.abs(arr - a))]
            if abs(c - a) <= 0.05:
                d.append(c - a)
        off = float(np.median(d)) if d else 0.0
        col = []
        for a in anchor:
            c = arr[np.argmin(np.abs(arr - (a + off)))]
            col.append(c - off if abs(c - (a + off)) <= tol else np.nan)
        col = np.asarray(col)
        if np.mean(~np.isnan(col)) >= min_cover:
            cols[k] = col
    if not cols:
        return {}
    ok = np.all([~np.isnan(v) for v in cols.values()], axis=0)
    return {k: v[ok] for k, v in cols.items()}


def replay_exact(rows):
    """The share of the rows whose replay of the shipped walk from the version 2 time equals the app's time (within a
    nanosecond); None without a version 2 time."""
    r = [abs(x["replay"]["shipped"]) < 1e-9 for x in rows if "replay" in x]
    return float(np.mean(r)) if r else None


def file_pairs(files_on):
    """The reference files against each other, with no app in it: {"a | b": {n, sd ms, share of hits more than 1 ms apart}} for
    each pair of files, after the pair's median offset (`align`, trimmed variance)."""
    out = {}
    names = list(files_on)
    for i, a in enumerate(names):
        for b in names[i + 1:]:
            al = align({}, {a: files_on[a], b: files_on[b]})
            if a not in al or b not in al or not len(al[a]):
                continue
            d = al[b] - al[a]
            d = d - np.median(d)
            out[f"{a} | {b}"] = {"n": int(len(d)), "sd_ms": float(np.sqrt(trimmed_var_ms2(d))),
                                 "beyond": float(np.mean(np.abs(d) > MOVE_S + 1e-9)),
                                 "hist": np.histogram(d * 1000.0, bins=np.arange(-3.0, 3.01, 0.5))[0].tolist()}
    return out


def analyze_track(name, refs_on, events, x, sr, files_on, mic_events, before_events, cfg_stems, out, log=print):
    """Everything of one song: the yardstick state and counts, the matched rows, the per-file counts and the hat, the other
    mics' counts. Reads no files but its arguments (the audio is in `x`)."""
    state = tf.track_state(refs_on, events)
    matched_t = sorted(state["est"][j] for _, j in state["pairs"])
    typical = float(np.median([ff.peak_db(x, sr, t) for t in matched_t])) if matched_t else -20.0
    est = state["est"]
    # the version 2 time of the same hit (the walk's start), by index when the counts agree
    b_est = fm.outputs(before_events) if before_events else []
    rows = hit_rows(state, events, x, sr, typical, [e["t"] for e in events], b_est if len(b_est) == len(est) else None)
    res = {"state": {k: state[k] for k in ("off", "est", "shifted", "false", "missed")}, "pairs": state["pairs"], "rows": rows,
           "typical_db": typical, "counts": mic_counts(refs_on, events, state["off"]), "mics": {}, "files": {},
           "replay_exact": replay_exact(rows)}
    for m, ev in mic_events.items():
        if ev:
            res["mics"][m] = mic_counts(refs_on, ev)
    for f, on in files_on.items():
        c = mic_counts(on, events)
        res["files"][f] = {"n": len(on), "off_ms": c["off_ms"], "moved": c["moved"], "matched": c["matched"], "false": c["false"],
                           "missed": c["missed"], "errs": c["errs"]}
    ev_t = {"k in": est}
    for m, ev in mic_events.items():
        if ev:
            ev_t[m] = fm.outputs(ev)
    al = align(files_on, ev_t) if files_on else {}
    hat0 = hat_jitter(al) if al else {}
    off_f = {f: fm.track_offset(on, events) for f, on in files_on.items()}
    by_file = {f: {j: e for _, j, e in pair_errors(on, est, off_f[f])} for f, on in files_on.items()}
    for r in rows:
        r["file_err"] = {f: by_file[f].get(r["j"]) for f in files_on}
        if "moved_class" in r:
            r["side"] = side_class(list(r["file_err"].values()))
    fair, how = pick_fair(list(files_on), hat0) if files_on else (None, None)
    for r in rows:
        e = r["file_err"].get(fair) if fair else None
        if e is not None and abs(e) > MOVE_S + 1e-9:
            a = audio_between(x, sr, r["t"], r["t"] - e)
            r["fair_class"] = tf.moved_class(e, a["m_db"], a["floor_db"], a["dip_db"])
    res["hat"] = hat0
    res["hat_n"] = len(next(iter(al.values()))) if al else 0
    res["file_pairs"] = file_pairs(files_on)
    res["fair"] = {"file": fair, "how": how}
    if fair:
        fc = res["files"][fair]
        res["fair_counts"] = {k: fc[k] for k in ("n", "off_ms", "matched", "false", "missed", "moved")}
        res["mics_fair"] = {m: {k: v for k, v in mic_counts(files_on[fair], ev).items() if k != "errs"}
                            for m, ev in mic_events.items() if ev}
    log(f"{name}: matched {len(state['pairs'])}, moved {res['counts']['moved']}, fair {fair} ({how})")
    return res


def analyze_song(song, refs, run, before, out, log=print):
    cfg, stems, files, _yard = session_cfg(song)
    x, sr = fm.read_stem(cfg, stems["k in"])
    ev = run["events"]["k in"]
    mic_events = {m: read_events(out, song, m) for m in ("k out", "k sub")}
    files_on = file_onsets(cfg, files)
    res = analyze_track(song, refs[song]["mics"]["k in"]["onsets"], ev, x, sr, files_on, mic_events,
                        before["events"]["k in"], stems, out, log)
    res["miss_false"] = missed_false(song, res, refs, run, x, sr, cfg, stems, out)
    return res


def analyze_held(name, out, log=print):
    cfg, stems, files, yard = session_cfg(name)
    x, sr = fm.read_stem(cfg, stems["k in"])
    ev = read_events(out, name, "k in")
    if x is None or not ev:
        return None
    files_on = file_onsets(cfg, files)
    ref = union_onsets([files_on[f] for f in yard if f in files_on])
    mic_events = {m: read_events(out, name, m) for m in ("k out", "k sub")}
    before = {"events": read_events(out, name, "k in v2")}
    return analyze_track(name, ref, ev, x, sr, files_on, mic_events, before["events"], stems, out, log)


def missed_false(song, res, refs, run, x, sr, cfg, stems, out):
    """The classes of the missed and the false kicks of one yardstick song: {"missed": [row], "false": [row]}."""
    st = res["state"]
    state = {"est": st["est"], "shifted": st["shifted"], "pairs": res["pairs"], "missed": st["missed"], "false": st["false"]}
    act = mc.read_activation(Path(out) / "work" / tid(song, "k in") / "model" / "act.bin")
    causes = kick_stage_causes(state, act)
    est, sh = st["est"], st["shifted"]
    matched = sorted(est[j] for _, j in res["pairs"])
    used = {j for _, j in res["pairs"]}
    false_t = [est[j] for j in st["false"]]
    wx, wsr = fm.read_stem(cfg, stems["k out"])
    if wx is not None and wsr != sr:
        wx = None
    layers = []
    for stem in FILES[song]["files"]:
        if stem in FILES[song]["yard"]:
            continue
        lx, lsr = fm.read_stem(cfg, stem)
        if lx is None:
            continue
        import learn_eval as le
        on = le.ref_onsets(lx, lsr, gap=0.005)
        layers += [float(t) - fm.layer_offset(refs[song]["mics"]["k in"]["onsets"], on) + st["off"] for t in on]
    layers = sorted(layers)
    snare = sorted(r + fm.track_offset(v["onsets"], run["events"][m]) for m, v in refs[song]["mics"].items()
                   if v["role"] == "snare" for r in v["onsets"])
    ref_t = np.asarray(sh, float)
    missed = []
    for i in st["missed"]:
        t = sh[i]
        silent = ff.peak_db(x, sr, t) < tf.SILENT_DB
        nf = tf.nearest_abs(false_t, t)
        merged = any(abs(est[j] - t) <= fm.TOL for j in used)
        cause, peak = causes[i]
        missed.append({"t": t, "rise_own": ff.rise_db(x, sr, t), "silent": bool(silent), "nearest_false_ms": nf * 1000,
                       "merged": bool(merged), "cause": cause, "act_peak": peak,
                       "class": miss_class(ff.rise_db(x, sr, t), nf, merged, cause, bool(silent))})
    false = []
    for j in st["false"]:
        t = est[j]
        fl = fm.stroke_flags([t], layers, x, sr, wx, other=snare)[0]
        after = tf.after_matched_s(t, matched)
        since = float(t - ref_t[ref_t < t].max()) if np.any(ref_t < t) else 9.0
        false.append({"t": t, "layer": bool(fl[0]), "witness": bool(fl[1]), "rise_own": ff.rise_db(x, sr, t),
                      "snare_near_ms": tf.nearest_abs(snare, t) * 1000, "after_matched_ms": after * 1000,
                      "class": false_class(bool(fl[0]), bool(fl[1]), tf.nearest_abs(snare, t), after, ff.rise_db(x, sr, t), since)})
    return {"missed": missed, "false": false}


# ---------------------------------------------------------------- the song loop

def do_songs(out, src=SRC, log=print):
    out = Path(out)
    refs, runs, before = load_inputs(src)
    allres = {s: analyze_song(s, refs, runs[s], before[s], out, log) for s in SONGS}
    held = {}
    for h in HELD:
        r = analyze_held(h, out, log)
        if r:
            held[h] = r
    (out / "tracks.json").write_text(json.dumps({"songs": allres, "held": held}, default=float))
    return allres, held


# ---------------------------------------------------------------- question 4: candidates (pure parts)

def cat(arrays):
    """The arrays as one; an empty list gives an empty array."""
    return np.concatenate(arrays) if arrays else np.zeros(0)


def track_errors(res, ref="yard"):
    """(errors s, levels dB, row indices) of the matched hits of one track result against the yardstick reference (`ref`
    "yard") or the fair file (`ref` "fair"; a hit that file lacks is dropped)."""
    rows = res["rows"]
    if ref == "yard":
        idx = list(range(len(rows)))
        return np.asarray([rows[k]["err"] for k in idx]), np.asarray([rows[k]["rel_db"] for k in idx]), idx
    f = res["fair"]["file"]
    idx = [k for k, r in enumerate(rows) if r["file_err"].get(f) is not None]
    return (np.asarray([rows[k]["file_err"][f] for k in idx]), np.asarray([rows[k]["rel_db"] for k in idx]), idx)


def rule_errors(res, ref="yard"):
    """{rule: errors after} for the back-track variants (`WALKS`, replayed from the version 2 time) and the errors before
    ("base"), against the yardstick reference or the fair file. The error after a rule is the error before plus the change of
    the time. No variants without a version 2 time."""
    base, _, idx = track_errors(res, ref)
    rows = res["rows"]
    out = {"base": base}
    if rows and all("replay" in r for r in rows):
        for name in WALKS:
            if name != "shipped":
                out["walk " + name] = base + np.asarray([rows[k]["replay"][name] for k in idx])
        for name, jmax in COND_WALKS:
            out[f"walk {name} if jump under {jmax:.0f} dB"] = base + np.asarray(
                [rows[k]["replay"][name] if rows[k]["jump_db"] < jmax else 0.0 for k in idx])
    return out


def fitted_rules(results, fit, ref="yard"):
    """The two rules that are fitted: a constant shift (the best on the `fit` songs, `best_shift`) and a level line (error
    against level, least squares on the fit songs). Returns ({rule: (parameters)}, {song: {rule: errors after}})."""
    e = np.concatenate([track_errors(results[s], ref)[0] for s in fit if s in results])
    lv = np.concatenate([track_errors(results[s], ref)[1] for s in fit if s in results])
    shift, _ = best_shift(e)
    slope, icpt = fit_line(lv, e)
    par = {"shift": shift, "line": (slope, icpt)}
    out = {}
    for s, res in results.items():
        eb, lb, _ = track_errors(res, ref)
        out[s] = {"shift " + f"{shift:+.2f} ms": eb + shift / 1000.0, "level line": eb - (slope * lb + icpt)}
    return par, out


def candidate_rows(results, fit, held, sessions, ref="yard"):
    """{rule: {group: (moved before, moved after, wrong, right)}} for the groups fit songs, held-out songs and held-out
    sessions; `results` {name: track result}."""
    per = {}
    for s, res in results.items():
        r = rule_errors(res, ref)
        per[s] = {"base": r.pop("base"), **r}
    par, fitted = fitted_rules(results, fit, ref)
    for s, d in fitted.items():
        per[s].update(d)
    groups = {"fit": [s for s in fit if s in per], "held": [s for s in held if s in per],
              "sessions": [s for s in sessions if s in per], "seven": [s for s in fit + held if s in per]}
    rules = [k for k in per[next(iter(per))] if k != "base"] if per else []
    out = {}
    for rule in rules:
        row = {}
        for g, names in groups.items():
            b = cat([per[s]["base"] for s in names if rule in per[s]])
            a = cat([per[s][rule] for s in names if rule in per[s]])
            row[g] = (moved_count(b), *two_sided(b, a))
        out[rule] = row
    return out, par


# ---------------------------------------------------------------- reports

def md(head, body):
    return "\n".join(["| " + " | ".join(head) + " |", "|" + "---|" * len(head)] + ["| " + " | ".join(str(c) for c in r) + " |" for r in body])


def f2(v):
    return "nan" if v is None or (isinstance(v, float) and np.isnan(v)) else f"{v:.2f}"


def stats_row(errs):
    s = fm.err_stats(errs)
    return [s["n"], f2(s["p10"]), f2(s["median"]), f2(s["p90"]), f"{100 * s['beyond']:.0f} %" if s["n"] else "-"]


def all_tracks(data):
    return {**data["songs"], **data["held"]}


def section_files(data):
    songs, held = data["songs"], data["held"]
    lines = ["### Reference files against the app (`k in`)", ""]
    body = []
    for name, res in all_tracks(data).items():
        yard = FILES[name]["yard"] if name in FILES else HELD[name]["files"][:1]
        for f, v in res["files"].items():
            tag = (" (yardstick)" if f in yard else "") + (" (fair)" if f == res["fair"]["file"] else "")
            body.append([name, f + tag, v["n"], f2(v["off_ms"]), v["moved"], *stats_row(v["errs"])[1:],
                         f2(res["hat"].get(f, float("nan")))])
    lines += [md(["song", "file", "onsets", "offset ms", "moved", "p10", "median", "p90", "beyond 1 ms", "hat jitter ms"], body), ""]
    body = []
    for name, res in all_tracks(data).items():
        h = {k: v for k, v in res["hat"].items() if k in MICS}
        body.append([name, res["hat_n"], *[f2(h.get(m, float("nan"))) for m in MICS]])
    lines += ["Jitter of the app's onset on each kick mic (three-cornered hat, ms; a mic the app hears in under half of the hits is left out):", "",
              md(["song", "hits in common", "k in", "k out", "k sub"], body), ""]
    return "\n".join(lines)


def section_pairs(data):
    lines = ["### The reference files against each other (no app; one offset per pair)", ""]
    body = []
    for name, res in all_tracks(data).items():
        for k, v in res["file_pairs"].items():
            body.append([name, k, v["n"], f2(v["sd_ms"]), f"{100 * v['beyond']:.0f} %", " ".join(str(c) for c in v["hist"])])
    lines += [md(["song", "pair", "hits", "sd ms", "more than 1 ms apart", "hits per 0.5 ms bin, -3 to +3 ms"], body), ""]
    return "\n".join(lines)


def section_fair(data):
    lines = ["### The yardstick against a fair reference file (kick, `k in`)", ""]
    body, tot = [], np.zeros(8, int)
    for s, res in data["songs"].items():
        y, f = res["counts"], res["fair_counts"]
        yf = y["false"] + y["missed"] + y["moved"]
        ff_ = f["false"] + f["missed"] + f["moved"]
        tot += np.asarray([y["false"], y["missed"], y["moved"], yf, f["false"], f["missed"], f["moved"], ff_])
        body.append([s, y["false"], y["missed"], y["moved"], f"**{yf}**", f"{res['fair']['file']} ({res['fair']['how']})",
                     f["false"], f["missed"], f["moved"], f"**{ff_}**"])
    body.append(["**all**", *tot[:3], f"**{tot[3]}**", "", *tot[4:7], f"**{tot[7]}**"])
    lines += [md(["song", "false", "missed", "moved", "fixes", "fair file", "false", "missed", "moved", "fixes"], body), ""]
    return "\n".join(lines)


def section_moved(data):
    songs = data["songs"]
    lines = ["### Moved kicks by size and sign (yardstick reference; `k in`)", "", "Cells: late / early.", ""]
    body, tot = [], {}
    for s, res in songs.items():
        ss = size_sign([r["err"] for r in res["rows"]])
        cells = [f"{ss.get((b, 'late'), 0)} / {ss.get((b, 'early'), 0)}" for b in ("1 to 2", "2 to 5", "over 5")]
        for k, v in ss.items():
            tot[k] = tot.get(k, 0) + v
        body.append([s, res["counts"]["matched"], *cells, res["counts"]["moved"]])
    cells = [f"{tot.get((b, 'late'), 0)} / {tot.get((b, 'early'), 0)}" for b in ("1 to 2", "2 to 5", "over 5")]
    body.append(["**all**", sum(r["counts"]["matched"] for r in songs.values()), *cells, sum(r["counts"]["moved"] for r in songs.values())])
    lines += [md(["song", "matched", "1 to 2 ms", "2 to 5 ms", "over 5 ms", "moved"], body), ""]
    body = []
    for s, res in songs.items():
        row = [s]
        for m in MICS:
            c = res["counts"] if m == "k in" else res["mics"].get(m)
            cf = res["fair_counts"] if m == "k in" else res.get("mics_fair", {}).get(m)
            row.append(f"{c['moved']} / {cf['moved']} (matched {c['matched']}, offset {f2(c['off_ms'])} ms)" if c and cf else "no run")
        body.append(row)
    lines += ["Moved kicks per mic, against the yardstick reference / against the fair file (each mic has its own offset):", "",
              md(["song", "k in", "k out", "k sub"], body), ""]
    sides = ("ref_file", "app", "unknown")
    body = []
    for s, res in songs.items():
        c = {k: 0 for k in sides}
        for r in res["rows"]:
            if "side" in r:
                c[r["side"]] += 1
        body.append([s, *[c[k] for k in sides]])
    body.append(["**all**", *[sum(r[1 + i] for r in body) for i in range(len(sides))]])
    lines += ["Whose error (a moved hit whose app time agrees within 1 ms with another reference file of the session: the yardstick's file is the odd one out):",
              "", md(["song", *sides], body), ""]
    classes = tf.MOVED_CLASSES
    for key, title in (("moved_class", "yardstick reference"), ("fair_class", "fair file")):
        body = []
        for s, res in songs.items():
            c = {k: 0 for k in classes}
            for r in res["rows"]:
                if key in r:
                    c[r[key]] += 1
            body.append([s, *[c[k] for k in classes]])
        body.append(["**all**", *[sum(row[1 + i] for row in body) for i in range(len(classes))]])
        lines += [f"Audio class of the moved kicks against the {title} (`tom_fixes.moved_class`):", "", md(["song", *classes], body), ""]
    body = []
    for s, res in songs.items():
        ss = size_sign([r["file_err"][res["fair"]["file"]] for r in res["rows"] if r["file_err"].get(res["fair"]["file"]) is not None])
        body.append([s, *[f"{ss.get((b, 'late'), 0)} / {ss.get((b, 'early'), 0)}" for b in ("1 to 2", "2 to 5", "over 5")]])
    lines += ["Size and sign against the fair file (late / early):", "", md(["song", "1 to 2 ms", "2 to 5 ms", "over 5 ms"], body), ""]
    for ref, title in (("yard", "yardstick reference"), ("fair", "fair file")):
        body = []
        allcl = []
        for s, res in songs.items():
            e, _, idx = track_errors(res, ref)
            cl = [res["rows"][k] for k in idx if res["rows"][k]["clean"]]
            ce = [e[i] for i, k in enumerate(idx) if res["rows"][k]["clean"]]
            allcl += ce
            body.append([s, *stats_row(ce)])
        body.append(["**all**", *stats_row(allcl)])
        lines += [f"Clean onsets (isolated 300 ms, level -6 dB or more, 30 dB over the floor within 2.5 ms), error against the {title}, ms:", "",
                  md(["song", "hits", "p10", "median", "p90", "beyond 1 ms"], body), ""]
    return "\n".join(lines)


def section_hits(data):
    songs = data["songs"]
    lines = []
    for ref, title in (("yard", "yardstick reference"), ("fair", "fair file")):
        lines += [f"### The error against the hit (`k in`, {title}, ms)", ""]
        errs, rows = [], []
        for res in songs.values():
            e, _, idx = track_errors(res, ref)
            errs += list(e)
            rows += [res["rows"][k] for k in idx]
        body = []
        for name, lo, hi in GAP_BINS:
            body.append([name, *stats_row([e for e, r in zip(errs, rows) if lo <= r["gap_prev_s"] < hi])])
        lines += [md(["previous hit", "hits", "p10", "median", "p90", "beyond 1 ms"], body), ""]
        edges = level_bins([r["rel_db"] for r in rows])
        body = []
        for k, name in enumerate(["quiet third", "middle third", "loud third"]):
            body.append([name, *stats_row([e for e, r in zip(errs, rows) if bin_index(r["rel_db"], edges) == k])])
        lines += [f"Level thirds (edges {edges[0]:.1f} and {edges[1]:.1f} dB against the typical hit):", "",
                  md(["level", "hits", "p10", "median", "p90", "beyond 1 ms"], body), ""]
        body = []
        for s, res in songs.items():
            e, lv, _ = track_errors(res, ref)
            _, med, moved = level_offsets(e, lv)
            shift, mv = best_shift(e)
            body.append([s, moved_count(e), f"{shift:+.2f} -> {mv}", " / ".join(f"{m * 1000:+.2f}" for m in med), moved])
        if ref == "yard":
            body = []
            for s, res in songs.items():
                e, lv, _ = track_errors(res, ref)
                c = two_clusters(e, lv)
                body.append([s, f"{100 * c['late_share']:.0f} %", f2(c["gap_ms"]), f2(c["lag1"]), f2(c["corr_level"])])
            lines += ["Two groups in the error? Share of hits more than 0.4 ms after the median, the gap between the group medians (ms), the "
                      "lag-1 correlation of the group sequence (negative: the groups alternate), the correlation of the error with the level:",
                      "", md(["song", "late group", "gap ms", "lag-1", "corr level"], body), ""]
        lines += ["Per song: moved now; best constant shift (ms) and moved after it; median error in the quiet / middle / loud third; moved after "
                  "one offset per third:", "", md(["song", "moved", "best shift", "median by level third", "moved after"], body), ""]
    return "\n".join(lines)


def section_missed(data):
    songs = data["songs"]
    lines = ["### Missed and false kicks (yardstick reference)", ""]
    body = []
    for s, res in songs.items():
        c = {k: 0 for k in MISS_CLASSES}
        for r in res["miss_false"]["missed"]:
            c[r["class"]] += 1
        f = {k: 0 for k in FALSE_CLASSES}
        for r in res["miss_false"]["false"]:
            f[r["class"]] += 1
        body.append([s, *[c[k] for k in MISS_CLASSES], *[f[k] for k in FALSE_CLASSES]])
    body.append(["**all**", *[sum(r[1 + i] for r in body) for i in range(len(MISS_CLASSES) + len(FALSE_CLASSES))]])
    lines += [md(["song", *["missed " + k for k in MISS_CLASSES], *["false " + k for k in FALSE_CLASSES]], body), ""]
    return "\n".join(lines)


def section_rules(data):
    results = all_tracks(data)
    lines = []
    for ref, title in (("yard", "yardstick reference"), ("fair", "fair file")):
        tbl, par = candidate_rows(results, FIT, HELD_SONGS, list(data["held"]), ref)
        body = [[k, *[f"{v[g][0]} -> {v[g][1]} (+{v[g][2]} -{v[g][3]})" for g in ("fit", "held", "sessions", "seven")]]
                for k, v in tbl.items()]
        lines += [f"### Rule candidates: moved kicks against the {title} (before -> after, right hits made wrong +, wrong hits made right -)", "",
                  md(["rule", "fit songs", "held-out songs", "held-out sessions", "all seven songs"], body),
                  "", f"Fitted: shift {par['shift']:+.2f} ms; level line {par['line'][0] * 1000:+.3f} ms per dB, intercept {par['line'][1] * 1000:+.2f} ms.", ""]
    return "\n".join(lines)


def section_walk_songs(data):
    songs = data["songs"]
    names = ["shipped", "peak 25", "peak 27", "peak 29", "peak 31"]
    lines = []
    for ref, title in (("yard", "yardstick reference"), ("fair", "fair file")):
        body, tot = [], np.zeros(len(names), int)
        for s, res in songs.items():
            e, _, idx = track_errors(res, ref)
            row = []
            for k, n in enumerate(names):
                after = e + np.asarray([res["rows"][i]["replay"][n] for i in idx])
                m = moved_count(after)
                tot[k] += m
                row.append(m)
            body.append([s, *row])
        body.append(["**all**", *tot])
        lines += [f"### Moved kicks per song under a kick peak margin (the walk's peak term, dB; shipped 33), against the {title}", "",
                  md(["song", *names], body), ""]
    return "\n".join(lines)


def do_report(out, log=print):
    out = Path(out)
    data = json.loads((out / "tracks.json").read_text())
    text = "\n".join([section_files(data), section_pairs(data), section_fair(data), section_moved(data), section_hits(data), section_missed(data),
                      section_rules(data), section_walk_songs(data)])
    (out / "report.md").write_text(text)
    log(text)
    return text


def do_kit(out, base_tool, cand_tool, log=print):
    """P4-31 candidate A on the kits' kick files (the DRT kit and the P4-11 kits): the app before the kick walk (`base_tool`, a
    release `service_tool` of the commit before) and with it (`cand_tool`, the current build), both default settings. Per file:
    hits before and after (the walk changes onsets, never the hit count), and the onsets more than 1 ms from the reference
    (within 5 ms) before and after. Returns the rows; `kit_kick.json` in `out`."""
    import flam_pairs as fp
    out = Path(out)
    model = "none".resolve()
    rows = []
    for name, files in fp.kit_projects(fp.OUT).items():
        kick = [(f, r) for f, r in files if r == "kick"]
        if not kick:
            continue
        ev0 = fp.run_project(Path(base_tool), model, kick, out / "scratch" / (name + "_base"), log)
        ev1 = fp.run_project(Path(cand_tool), model, kick, out / "scratch" / (name + "_cand"), log)
        for f, _ in kick:
            stem = Path(f).stem
            ref, _lab = fp.kit_refs(name, stem, fp.OUT)
            t0, t1 = fm.outputs(ev0[stem]), fm.outputs(ev1[stem])
            e0 = [(t0[j] - ref[i]) * 1000 for i, j in fm.match(ref, t0, 0.005)]
            e1 = [(t1[j] - ref[i]) * 1000 for i, j in fm.match(ref, t1, 0.005)]
            rows.append({"kit": name, "file": stem, "hits_before": len(t0), "hits_after": len(t1), "n": len(ref),
                         "matched_before": len(e0), "matched_after": len(e1),
                         "far_before": sum(1 for v in e0 if abs(v) > 1.0), "far_after": sum(1 for v in e1 if abs(v) > 1.0)})
            log(f"{name} {stem}: hits {len(t0)} to {len(t1)}, matched {len(e0)} to {len(e1)} of {len(ref)}, "
                f"errors over 1 ms {rows[-1]['far_before']} to {rows[-1]['far_after']}")
    (out / "kit_kick.json").write_text(json.dumps(rows, indent=1))
    return rows


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("cmd", choices=["run", "songs", "report", "all", "files", "moved", "hits", "missed", "rules", "kit"])
    ap.add_argument("--base-tool", default=None, help="kit: the release service_tool of the commit before the kick walk")
    ap.add_argument("--cand-tool", default=None, help="kit: the release service_tool of the current build")
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--src", default=str(SRC))
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    if a.cmd == "kit":
        do_kit(out, a.base_tool, a.cand_tool)
        return 0
    if a.cmd == "run":
        do_run(out)
        return 0
    if a.cmd in ("songs", "all"):
        if a.cmd == "all":
            do_run(out)
        do_songs(out, a.src)
    if a.cmd in ("report", "all", "files", "moved", "hits", "missed", "rules"):
        text = do_report(out, print)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
