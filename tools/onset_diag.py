"""The shared parts of the kick and snare onset diagnoses (P4-22, P4-23; diagnosis only), used by `tools/snare_onset.py`:
reading the P4-21b yardstick runs, the three-cornered hat of the reference files, alignment, size and sign of a moved hit,
the two-sided count, the walk replay and the miss and false classes; and from the tom card (P4-20 question 3) the audio
class of a moved hit. Moved verbatim from `kick_onset.py` and `tom_fixes.py` when those two scripts were removed
(refactor step 1.4).
"""
import json
import math
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fix_metric as fm  # noqa: E402
import firm_fixes as ff  # noqa: E402
import onset_guard as og  # noqa: E402


def wait_for_lock(log=print):
    """Wait while another measuring agent holds the measure lock. The load is not checked: one single-thread model run of
    a track takes about a second and nothing here measures time."""
    while fm.LOCK.exists():
        log("waiting: measure lock")
        time.sleep(30)


DOUBLE_S = (0.012, 0.060)               # a second stroke after a matched hit of the same mic
RING_S = 1.0                            # a hit up to this long after a reference hit of its own mic lies in the decay

MERGE_S = 0.050                         # an output hit this close to a missed reference hit took another one's place

SILENT_DB = -90.0       # a mic file this quiet at a hit is digital silence (a gated or edited track)


def nearest_abs(times, t):
    """Distance (s) from `t` to the nearest of `times`; 9.0 for none."""
    a = np.asarray(times, float)
    return float(np.abs(a - t).min()) if len(a) else 9.0


def after_matched_s(t, matched):
    """Seconds since the last matched hit before `t`; 9.0 for none."""
    b = [t - m for m in matched if m < t]
    return min(b) if b else 9.0


def track_state(ref, events):
    """The P4-16 match of one tom mic: offset, output times, shifted references, matched pairs, missed reference indices,
    false output indices, and the flagged events [(t, "bleed" | "cut")] (they are not in the output)."""
    off = fm.track_offset(ref, events)
    est = fm.outputs(events)
    shifted = [float(r) + off for r in ref]
    pairs, missed, false = ff.split_hits(shifted, est)
    flagged = sorted((float(e["t"]), "bleed" if e.get("bleed") else "cut") for e in events
                     if e.get("bleed") or e.get("cut"))
    return {"off": off, "est": est, "shifted": shifted, "pairs": pairs, "missed": missed, "false": false,
            "flagged": flagged}


MOTION_WIN_S = 0.0005       # the window for "the audio moves at this time" (max |x|)
ENV_BLOCK_S = 0.0005        # the envelope between the event and the reference: 0.5 ms block maxima
GAP_DB = 9.0                # a dip of this many dB after a higher level between the two times: two separate lobes
OVER_FLOOR_DB = 6.0         # the audio moves when it stands this far over the floor before the hit (`FM_FLOOR_DB`)
MOVED_CLASSES = ["app_late", "ref_early", "ref_precursor", "ref_late", "app_early"]


def motion_db(x, sr, tau, win=MOTION_WIN_S):
    """The level (dBFS) of max |x| in [tau - win / 2, tau + win / 2]."""
    i0, i1 = max(0, int((tau - win / 2) * sr)), min(len(x), int((tau + win / 2) * sr) + 1)
    return ff._db(np.abs(x[i0:i1]).max()) if i1 > i0 else -180.0


def floor_db(x, sr, t, before=0.050):
    """The floor before `t` (dBFS): the median of the 1 ms block maxima in the `before` seconds before `t` without the last
    2 ms (`onset_guard.floor_before`, `refine.rs` `floor_before`)."""
    s = int(round(t * sr))
    return ff._db(og.floor_before(x, s, s - int(before * sr), sr))


def envelope_blocks(x, sr, t0, t1, block=ENV_BLOCK_S):
    """The levels (dBFS) of the block maxima of |x| from `t0` to `t1`, `block` seconds each (at least two blocks)."""
    n = max(2, int(math.ceil((t1 - t0) / block)))
    out = []
    for k in range(n):
        i0, i1 = int((t0 + k * block) * sr), int((t0 + (k + 1) * block) * sr)
        out.append(ff._db(np.abs(x[max(0, i0):max(i0 + 1, i1)]).max()) if i0 < len(x) else -180.0)
    return out


def max_dip_db(levels):
    """The deepest drop (dB) below the running maximum of `levels`: 0 for a rise without a gap."""
    run, dip = -1e9, 0.0
    for v in levels:
        run = max(run, v)
        dip = max(dip, run - v)
    return dip


def moved_class(err, level_early_db, floor_db, dip_db, over_floor=OVER_FLOOR_DB, gap=GAP_DB):
    """The class of one hit more than 1 ms from its reference, by what the audio does between the two times
    (`level_early_db`: the level at the earlier time; `floor_db`: the floor before it; `dip_db`: `max_dip_db` of the
    envelope between them). `err` > 0 (the app is later): `ref_early` (quiet at the reference time), `ref_precursor` (a gap
    of 9 dB or more lies between: the reference sits on an earlier separate lobe), else `app_late` (the audio rises
    without a gap to the app's time: a slow attack the walk stops in). `err` < 0: `ref_late` (a rise without a gap from the
    app's time to the later reference time), else `app_early` (quiet, or a gap)."""
    quiet = level_early_db < floor_db + over_floor
    if err > 0:
        if quiet:
            return "ref_early"
        return "ref_precursor" if dip_db >= gap else "app_late"
    return "app_early" if quiet or dip_db >= gap else "ref_late"


def size_bin(err_ms):
    """The size class of an error in ms: "1 to 2", "2 to 5", "over 5" (absolute value)."""
    a = abs(err_ms)
    return "1 to 2" if a <= 2.0 else ("2 to 5" if a <= 5.0 else "over 5")


def moved_audio(state, x, sr):
    """For every matched hit, in `state["pairs"]` order: {level at the earlier of event and reference time, the floor before
    it, the deepest dip of the envelope between the two times (dB)}."""
    est, sh = state["est"], state["shifted"]
    out = []
    for i, j in state["pairs"]:
        te, tl = min(est[j], sh[i]), max(est[j], sh[i])
        lv = envelope_blocks(x, sr, te - ENV_BLOCK_S, max(tl, te + ENV_BLOCK_S))
        out.append({"m_db": motion_db(x, sr, te), "floor_db": floor_db(x, sr, te), "dip_db": max_dip_db(lv[1:])})
    return out


SRC = Path.home() / ".cache" / "drumspike" / "fix-metric-p421b"

SONGS = ["Session A", "Session B", "Session C", "Session H", "Session F", "Session G",
         "Session D"]

MOVE_S = fm.MOVE_S

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


def load_inputs(src=SRC):
    """(refs, default runs, before runs): `refs.json` and the P4-16 runs (`before` is refinement version 2)."""
    src = Path(src)
    refs = json.loads((src / "refs.json").read_text())
    runs = {s: json.loads((src / "run" / f"{fm.track_name(s)}_default.json").read_text()) for s in refs}
    before = {s: json.loads((src / "run" / f"{fm.track_name(s)}_before.json").read_text()) for s in refs}
    return refs, runs, before


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
    """{(size, sign): count} of the moved hits: size "1 to 2" / "2 to 5" / "over 5" (`size_bin`), sign "late"
    (the app after the reference) or "early". Hits within 1 ms are not counted."""
    out = {}
    for e in errs_s:
        if abs(e) <= MOVE_S + 1e-9:
            continue
        k = (size_bin(e * 1000.0), "late" if e > 0 else "early")
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
    `floor_db`). A kick's peak comes 6 to 11 ms after the foot, so the tom card's "peak within 1.5 ms" never holds
    for it; a sharp kick onset is a high level a few samples after the foot."""
    i0, i1 = max(0, int(t * sr)), min(len(x), int((t + JUMP_S) * sr))
    if i1 <= i0:
        return -180.0
    return ff._db(np.abs(x[i0:i1]).max()) - (floor_db(x, sr, t) if floor is None else floor)


def clean_row(gap_prev_s, gap_next_s, rel_db, jump):
    """A clean onset: isolated (no hit within 300 ms before or after), strong (level within 6 dB of the typical hit or
    louder), sharp (`jump_db` 30 dB or more)."""
    return gap_prev_s >= CLEAN_GAP_S and gap_next_s >= CLEAN_GAP_S and rel_db >= CLEAN_DB and jump >= CLEAN_JUMP_DB


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


MISS_CLASSES = ["silent", "no_stroke", "outside_20ms", "merged", "threshold", "blind", "average", "lobe", "role", "other_stage"]


def miss_class(rise_own, nearest_false_s, merged, cause, silent=False, rise_min=ff.RISE_MIN_DB):
    """The class of one missed kick, the first that fits: `silent` (the mic file is digital silence there), `no_stroke`
    (the own mic shows no stroke: a reference error), `outside_20ms` (an unmatched output hit 20 to 50 ms away: found, too
    far to match), `merged` (a matched output hit within 20 ms belongs to another reference hit), else the stage `cause` of
    `miss_causes` (threshold, blind, average, lobe, role); the rest is `other_stage`."""
    if silent:
        return "silent"
    if rise_own < rise_min:
        return "no_stroke"
    if fm.TOL < nearest_false_s <= MERGE_S:
        return "outside_20ms"
    if merged:
        return "merged"
    return cause if cause in ("threshold", "blind", "average", "lobe", "role") else "other_stage"


def false_class(layer, witness, snare_near_s, after_matched_s, rise_own, since_ref_s, rise_min=ff.RISE_MIN_DB,
                bleed_s=fm.BLEED_S, double_s=DOUBLE_S, ring_s=RING_S):
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


def audio_between(x, sr, t_est, t_ref):
    """The audio facts of a hit that lies more than 1 ms from its reference, as `moved_audio`: the level at the
    earlier of the two times, the floor before it, the deepest dip of the envelope between the two times (dB)."""
    te, tl = min(t_est, t_ref), max(t_est, t_ref)
    lv = envelope_blocks(x, sr, te - ENV_BLOCK_S, max(tl, te + ENV_BLOCK_S))
    return {"m_db": motion_db(x, sr, te), "floor_db": floor_db(x, sr, te), "dip_db": max_dip_db(lv[1:])}


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
    neighbouring hits (s), the app's `v`, the audio at the two times (`moved_audio`), the class of a moved hit and
    the change of the time (s) under each back-track variant of `WALKS` (`replay`, from the version 2 time `t_v2` when it is known)."""
    est, sh = state["est"], state["shifted"]
    mv = moved_audio(state, x, sr)
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
            row["moved_class"] = moved_class(err, a["m_db"], a["floor_db"], a["dip_db"])
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
    state = track_state(refs_on, events)
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
            r["fair_class"] = moved_class(e, a["m_db"], a["floor_db"], a["dip_db"])
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


def md(head, body):
    return "\n".join(["| " + " | ".join(head) + " |", "|" + "---|" * len(head)] + ["| " + " | ".join(str(c) for c in r) + " |" for r in body])


def f2(v):
    return "nan" if v is None or (isinstance(v, float) and np.isnan(v)) else f"{v:.2f}"


def stats_row(errs):
    s = fm.err_stats(errs)
    return [s["n"], f2(s["p10"]), f2(s["median"]), f2(s["p90"]), f"{100 * s['beyond']:.0f} %" if s["n"] else "-"]
