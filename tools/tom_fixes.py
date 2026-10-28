"""P4-20 (diagnosis only): why the tom fixes of the P4-16 yardstick are there.

    uv run --with numpy --with soundfile --with mido python tools/tom_fixes.py run      # the model's activations of the tom mics
    uv run --with numpy --with soundfile --with mido python tools/tom_fixes.py missed   # question 1: the missed tom hits
    uv run --with numpy --with soundfile --with mido python tools/tom_fixes.py false    # question 2: the false tom hits
    uv run --with numpy --with soundfile --with mido python tools/tom_fixes.py moved    # question 3: the moved tom hits
    uv run --with numpy --with soundfile --with mido python tools/tom_fixes.py all

Input: `~/.cache/drumspike/fix-metric-p419/` (`refs.json`, `run/<song>_default.json`; written by `tools/fix_metric.py`).
Nothing under `crates/` or `static/` changes. The sessions in `~/.cache/drumspike/sessions` are read only.
Outputs: `~/.cache/drumspike/tom-fixes/` (`work/` the model's kept activations, `missed.json`, `false.json`, `moved.json`,
`report.md`). The model run is the only app run; set `DRUMSPIKE_CALIB_TOOL` to a release `service_tool` when the
worktree has none.

Model of the work (as `tools/firm_fixes.py`, P4-18): every missed, false and moved tom hit of the yardstick gets one
class, the first that fits, from the own mic's audio, the other mics of the session and the app's kept activation.
A reference is a sample or trigger track, not the truth: a class "reference error" says the audio contradicts it.
"""
import argparse
import json
import math
import os
import shutil
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fix_metric as fm  # noqa: E402
import firm_fixes as ff  # noqa: E402
import miss_causes as mc  # noqa: E402
import onset_guard as og  # noqa: E402
import tom_bleed_sk as sk  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SRC = Path.home() / ".cache" / "drumspike" / "fix-metric-p419"
OUT = Path.home() / ".cache" / "drumspike" / "tom-fixes"
SONGS = ["Session A", "Session B", "Session C", "Session H", "Session F", "Session G",
         "Session D"]
ROLE = "tom"


def tid(song, mic):
    return fm.track_name(song) + "_" + fm.track_name(mic)


def tom_mics(refs, song):
    """The tom close mics of a song in the references (a tom mic's role there is `toms`)."""
    return [m for m, v in refs[song]["mics"].items() if v["role"] == "toms"]


def load_inputs(src=SRC):
    src = Path(src)
    refs = json.loads((src / "refs.json").read_text())
    runs = {s: json.loads((src / "run" / f"{fm.track_name(s)}_default.json").read_text()) for s in refs}
    return refs, runs


# ---------------------------------------------------------------- run: the activations (the model only)

def wait_for_lock(log=print):
    """Wait while another measuring agent holds the measure lock. The load is not checked: one single-thread model run of
    a track takes about a second and nothing here measures time."""
    while fm.LOCK.exists():
        log("waiting: measure lock")
        time.sleep(30)


def do_run(out, src=SRC, log=print):
    """One model run of each tom mic of the seven songs through `service_tool cli run`; the kept activation and the events
    go to `<out>/work/<id>/model/`. Resumes: a track with an `act.bin` is kept."""
    out = Path(out)
    refs, _ = load_inputs(src)
    todo = [(s, m) for s in SONGS for m in tom_mics(refs, s) if not (out / "work" / tid(s, m) / "model" / "act.bin").is_file()]
    if not todo:
        log("all activations are there")
        return
    tool, model = mc.find_tool(), "none".resolve()
    if not tool.is_file():
        raise SystemExit(f"need the release service_tool ({tool}) and the model ({model})")
    table = fm.songs()
    (out / "audio").mkdir(parents=True, exist_ok=True)
    try:
        for song in SONGS:
            for mic in tom_mics(refs, song):
                i = tid(song, mic)
                if (out / "work" / i / "model" / "act.bin").is_file():
                    continue
                stem, _ = table[song]["mics"][mic]
                files = [fm.audio_path(table[song], stem)]
                link = out / "audio" / f"{i}.wav"
                if link.is_symlink() or link.exists():
                    link.unlink()
                link.symlink_to(files[0])
                wait_for_lock(log)
                fm.LOCK.write_text(f"tom_fixes run {i}\n")
                try:
                    mc.run_item(tool, out, {"id": i, "audio": str(link), "role": ROLE, "note": mc.ROLE_NOTE[ROLE]},
                                "model", model)
                finally:
                    fm.LOCK.unlink(missing_ok=True)
                log(f"ran {i}")
    finally:
        fm.LOCK.unlink(missing_ok=True)
        shutil.rmtree(out / "run-home", ignore_errors=True)
        shutil.rmtree(out / "run-projects", ignore_errors=True)


# ---------------------------------------------------------------- the session's other files (read only)

# the tom sample or trigger tracks of a session (the references come from them): the union of their onsets says where
# the session's own tom sample was played, whatever mic the reference split gave the hit to
SAMPLE_STEMS = {
    "Session A": ["t1 sampl_01", "t2 sampl_01", "t3 sampl_01", "t4 sampl_01"],
    "Session B": ["t trigger_01"], "Session C": ["t samp_03"], "Session H": ["t samp_01"],
    "Session F": ["t1 samp_01", "t2 samp_01", "t3 samp_01", "t4 samp_01"],
    "Session G": ["t trigger_01"], "Session D": ["t trigger"]}
# hat, ride and overhead mics: a cymbal stroke that a tom mic hears
CYMBAL_STEMS = {
    "Session A": ["oh_01", "hh_03"], "Session B": ["OH_01", "hh_01", "ride_01"],
    "Session C": ["oh_03", "hh_03", "ri_03"], "Session H": ["oh_03", "hh_04", "ri.01_03"],
    "Session F": ["oh_01", "hh_01", "ri_01"], "Session G": ["oh_01", "hh_10", "ri_01"],
    "Session D": ["oh HH", "oh Ri", "hh", "ri"]}

# ---------------------------------------------------------------- features (pure; the tests cover these)

RISE_MIN_DB = ff.RISE_MIN_DB            # a stroke: the peak at the onset 10 dB over the peak 10 to 30 ms before it
CYMBAL_RISE_DB = 15.0                   # a hat, ride or overhead mic with this rise: a cymbal stroke is there
NEAR_S = 0.012                          # a reference hit of another drum this close to a hit: that drum's bleed
DOUBLE_S = (0.012, 0.060)               # a second stroke after a matched hit of the same mic
RING_S = 1.0                            # a hit up to this long after a reference hit of its own mic lies in the decay
SAMPLE_TOL_S = 0.006                    # a tom sample onset this close (after the mic's offset) is the same stroke
DOMINANCE_DB = 6.0                      # the source mic's level over its own typical hit exceeds the tom mic's by this: bleed
MERGE_S = 0.050                         # an output hit this close to a missed reference hit took another one's place


HP_HZ = 2000.0          # the attack's own band: a tom's ring is low, its stick click is not
SILENT_DB = -90.0       # a mic file this quiet at a hit is digital silence (a gated or edited track)


def stroke_rise(x, y, sr, t):
    """(full-band rise, high-band rise, the larger) in dB at `t`: the rise test of P4-18 on the mic `x` and on its high-pass
    `y` (`HP_HZ`). In a roll the ring of the earlier hit hides the full-band rise (27 to 33 % of the matched hits within
    300 ms of the previous one fail it); the stick click over the faster decaying highs does not."""
    a, b = ff.rise_db(x, sr, t), ff.rise_db(y, sr, t)
    return a, b, max(a, b)


def highpass(x, sr, hz=HP_HZ):
    """A zero-phase high-pass of `x` (`onset_guard.zero_phase`, RBJ twice forward and backward)."""
    return og.zero_phase(x, sr, [("hp", hz)])


def nearest_abs(times, t):
    """Distance (s) from `t` to the nearest of `times`; 9.0 for none."""
    a = np.asarray(times, float)
    return float(np.abs(a - t).min()) if len(a) else 9.0


def rel_level_db(x, sr, t, typical_db):
    """The level of the hit at `t` (peak in [t - 5 ms, t + 30 ms]) against the typical hit level `typical_db`."""
    return ff.peak_db(x, sr, t) - typical_db


def dominance_db(rel_src, rel_own):
    """How far the source mic's hit level (against its own typical hit) stands over the tom mic's (against its own typical
    hit). A tom stroke leaks into the snare and kick mics far below their own hits: it gives a negative number; the bleed
    of a snare or kick stroke into a tom mic gives a positive one."""
    return rel_src - rel_own


def is_source_bleed(rise_src, dom, rise_min=RISE_MIN_DB, dom_db=DOMINANCE_DB):
    """True when the source mic shows a stroke (rise) and is the dominant one (`dominance_db` `dom` at or over `dom_db`)."""
    return rise_src is not None and rise_src >= rise_min and dom >= dom_db


def in_decay(t, own, lo=DOUBLE_S[1], hi=RING_S):
    """True when a hit of `own` lies `lo` to `hi` seconds before `t`."""
    own = np.asarray(own, float)
    return bool(len(own)) and bool(np.any((t - own > lo) & (t - own <= hi)))


def after_matched_s(t, matched):
    """Seconds since the last matched hit before `t`; 9.0 for none."""
    b = [t - m for m in matched if m < t]
    return min(b) if b else 9.0


def read_all(cfg, stems):
    """{stem: (signal, rate)} of the files that exist."""
    out = {}
    for s in stems:
        x, sr = ff.read_stem(cfg, s)
        if x is not None:
            out[s] = (x, sr)
    return out


# ---------------------------------------------------------------- the track state (pure)

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


# ---------------------------------------------------------------- question 1: the missed hits

MISS_CLASSES = ["silent", "no_stroke", "bleed_rule", "loudness_cut", "outside_20ms", "other_tom", "merged", "threshold", "blind",
                "average", "lobe", "role", "other_stage"]
MISS_TEXT = {
    "silent": "the own mic file is digital silence there (a gated or edited track): no audio to find",
    "no_stroke": "the own mic shows no stroke (full-band and 2 kHz rise both under 10 dB): reference error",
    "bleed_rule": "the app found it and the tom bleed rule removed it (flag `bleed`)",
    "loudness_cut": "the app found it and the loudness cut removed it (flag `cut`)",
    "outside_20ms": "the app has an output hit 20 to 50 ms from it: found, too far to match",
    "other_tom": "the app has the hit on another tom mic (the reference gave it to this one)",
    "merged": "an output hit within 20 ms matches another reference hit (a close pair, one hit found)",
    "threshold": "the activation peaks between 0.05 and the pick threshold",
    "blind": "the activation stays under 0.05",
    "average": "the moving average lifts the margin over the threshold",
    "lobe": "one activation lobe for two hits",
    "role": "the model fired another class there",
    "other_stage": "a later stage (minimum gap, refinement, offset)"}


def miss_class(rise_own, flag_kind, nearest_false_s, other_mic_false, merged, cause, silent=False, rise_min=RISE_MIN_DB):
    """The class of one missed hit, the first that fits: `silent` (the own mic is digital silence there), `no_stroke`
    (the own mic shows no stroke: a reference error),
    `bleed_rule` / `loudness_cut` (the app's flag on an event within 20 ms), `outside_20ms` (an unmatched output hit
    20 to 50 ms away), `other_tom` (an unmatched output hit of another tom mic within 20 ms), `merged` (a matched output
    hit within 20 ms owned by another reference hit), else the stage `cause` of `miss_causes` (threshold, blind,
    average, lobe, role; the rest is `other_stage`)."""
    if silent:
        return "silent"
    if rise_own < rise_min:
        return "no_stroke"
    if flag_kind == "bleed":
        return "bleed_rule"
    if flag_kind == "cut":
        return "loudness_cut"
    if fm.TOL < nearest_false_s <= MERGE_S:
        return "outside_20ms"
    if other_mic_false:
        return "other_tom"
    if merged:
        return "merged"
    return cause if cause in ("threshold", "blind", "average", "lobe", "role") else "other_stage"


def flag_near(flagged, t, tol=fm.TOL):
    """The kind of the nearest flagged event within `tol` of `t` ("bleed" before "cut" at a tie), else None."""
    best = None
    for ft, kind in flagged:
        d = abs(ft - t)
        if d <= tol and (best is None or d < best[0]):
            best = (d, kind)
    return best[1] if best else None


def stage_causes(state, act):
    """{missed index: (cause, activation peak)} from the kept activation (`miss_causes.classify_miss`, role tom)."""
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


def analyze_missed(song, mic, state, states, x, y, sr, act, matched_peak_db):
    """The missed reference hits of one tom mic with features and class: [dict]. `states`: {tom mic: track_state} of the
    song (for the other mics' unmatched output hits)."""
    causes = stage_causes(state, act)
    est = state["est"]
    matched = {j for _, j in state["pairs"]}
    rows = []
    for i in state["missed"]:
        t = state["shifted"][i]
        rf, rh, ro = stroke_rise(x, y, sr, t)
        silent = ff.peak_db(x, sr, t) < SILENT_DB
        fk = flag_near(state["flagged"], t)
        false_t = [est[j] for j in state["false"]]
        nf = nearest_abs(false_t, t)
        other_false = any(nearest_abs([s["est"][j] for j in s["false"]], t) <= fm.TOL
                          for m, s in states.items() if m != mic)
        near_matched = [j for j in matched if abs(est[j] - t) <= fm.TOL]
        cause, peak = causes[i]
        rows.append({"t": t, "rise_own": ro, "rise_full": rf, "rise_hp": rh, "silent": bool(silent),
                     "rel_db": ff.peak_db(x, sr, t) - matched_peak_db, "flag": fk, "nearest_false_ms": nf * 1000, "other_mic_false": other_false,
                     "merged": bool(near_matched), "cause": cause, "act_peak": peak,
                     "neighbour_ms": 1000 * nearest_abs([r for k, r in enumerate(state["shifted"]) if k != i], t),
                     "class": miss_class(ro, fk, nf, other_false, bool(near_matched), cause, bool(silent))})
    return rows


# ---------------------------------------------------------------- question 2: the false hits

FALSE_CLASSES = ["no_reference", "other_tom", "snare_ref", "kick_ref", "double", "sample_track", "snare_audio",
                 "kick_audio", "real_stroke", "cymbal", "ring", "weak"]
FALSE_GROUP = {"no_reference": "real stroke (reference gap)", "sample_track": "real stroke (reference gap)",
               "real_stroke": "real stroke (reference gap)", "other_tom": "bleed", "snare_ref": "bleed",
               "kick_ref": "bleed", "snare_audio": "bleed", "kick_audio": "bleed", "cymbal": "bleed",
               "double": "double or ring", "ring": "double or ring", "weak": "no stroke"}
FALSE_TEXT = {
    "no_reference": "the mic has no reference hit in the song (t1 or t4 got no sample group): the reference cannot say",
    "other_tom": "a reference hit of another tom lies within 12 ms (bleed of that tom)",
    "snare_ref": "a reference hit of the snare lies within 12 ms",
    "kick_ref": "a reference hit of the kick lies within 12 ms",
    "double": "12 to 60 ms after a matched hit of the same mic (second stroke, flam, ring)",
    "sample_track": "a tom sample onset lies within 6 ms: the session played a tom there, the split gave it to no mic",
    "snare_audio": "the snare mic shows a stroke that dominates (a ghost note the reference lacks)",
    "kick_audio": "the kick mic shows a stroke that dominates",
    "real_stroke": "a stroke on the own mic (rise 10 dB or more) and no source: the reference lacks it",
    "cymbal": "a hat, ride or overhead mic rises 15 dB or more and the tom mic shows the stroke only in the highs (full band under 10 dB)",
    "ring": "inside the decay of an earlier reference hit (60 ms to 1 s), no stroke",
    "weak": "no stroke on the own mic and no source"}


def false_class(no_ref_mic, other_tom_s, snare_s, kick_s, after_s, sample_s, snare_bleed, kick_bleed, rise_own, rise_cym,
                decay, rise_full=None):
    """The source of one false tom hit, the first that fits: `no_reference` (the mic has no reference hit), `other_tom`
    / `snare_ref` / `kick_ref` (a reference hit of that drum within 12 ms; the nearest names it), `double` (12 to 60 ms
    after a matched hit of the same mic), `sample_track` (a tom sample onset within 6 ms that no reference of this mic
    has), `snare_audio` / `kick_audio` (that mic shows a stroke and dominates the tom mic: bleed of a stroke the reference
    lacks), `cymbal` (a hat, ride or overhead mic rises 15 dB or more and the tom mic's own stroke is only in the highs:
    no stroke on the full band, `rise_full`, or none at all), `real_stroke` (a stroke on the own mic and no source), `ring`
    (in the decay of an earlier hit), `weak`."""
    if no_ref_mic:
        return "no_reference"
    near = [(other_tom_s, "other_tom"), (snare_s, "snare_ref"), (kick_s, "kick_ref")]
    d, k = min(near, key=lambda a: abs(a[0]))
    if abs(d) <= NEAR_S:
        return k
    if DOUBLE_S[0] <= after_s <= DOUBLE_S[1]:
        return "double"
    if abs(sample_s) <= SAMPLE_TOL_S:
        return "sample_track"
    if snare_bleed:
        return "snare_audio"
    if kick_bleed:
        return "kick_audio"
    if rise_cym is not None and rise_cym >= CYMBAL_RISE_DB and (
            rise_own < RISE_MIN_DB or (rise_full is not None and rise_full < RISE_MIN_DB)):
        return "cymbal"
    if rise_own >= RISE_MIN_DB:
        return "real_stroke"
    return "ring" if decay else "weak"


def source_flags(t, own_x, own_sr, own_typical_db, src_x, src_sr, src_typical_db):
    """(rise on the source mic, dominance over the tom mic in dB) at `t`; (None, None) without the source mic."""
    if src_x is None or src_sr != own_sr:
        return None, None
    return (ff.rise_db(src_x, src_sr, t),
            dominance_db(rel_level_db(src_x, src_sr, t, src_typical_db), rel_level_db(own_x, own_sr, t, own_typical_db)))


def analyze_false(song, mic, state, states, x, y, sr, ctx, typical_db):
    """The false output hits of one tom mic with features and class: [dict]. `ctx`: {"snare": shifted reference times of the
    snare mics, "kick": same, "toms": {other tom mic: shifted reference times}, "sample": sample onsets (the mic's time
    base), "src": {"snare": (signal, rate, typical dB), "kick": (...)}, "cym": [(signal, rate)]}."""
    est = state["est"]
    matched = sorted(est[j] for _, j in state["pairs"])
    rows = []
    own_ref = state["shifted"]
    for j in state["false"]:
        t = est[j]
        near_tom = min((nearest_abs(v, t) for v in ctx["toms"].values()), default=9.0)
        other_found, src_mic, pair_diff = None, None, None
        for m, v in ctx["toms"].items():
            if len(v) and abs(nearest_abs(v, t) - near_tom) < 1e-12 and near_tom <= NEAR_S:
                s = states[m]
                other_found = any(abs(s["est"][b] - t) <= fm.TOL for _, b in s["pairs"])
                src_mic = m
                if m in ctx.get("tom_sig", {}):
                    sx, ssr = ctx["tom_sig"][m]
                    pair_diff = ff.peak_db(x, sr, t) - ff.peak_db(sx, ssr, t)
        sn, ki = nearest_abs(ctx["snare"], t), nearest_abs(ctx["kick"], t)
        after = after_matched_s(t, matched)
        sample = nearest_abs(ctx["sample"], t)
        rf, rh, ro = stroke_rise(x, y, sr, t)
        flags = {}
        for kind in ("snare", "kick"):
            src = ctx["src"].get(kind)
            if src is None:
                flags[kind] = (None, None)
            else:
                flags[kind] = source_flags(t, x, sr, typical_db, src[0], src[1], src[2])
        sb = is_source_bleed(*flags["snare"])
        kb = is_source_bleed(*flags["kick"])
        rc = max((ff.rise_db(cx, csr, t) for cx, csr in ctx["cym"] if csr == sr), default=None)
        cls = false_class(len(own_ref) == 0, near_tom, sn, ki, after, sample, sb, kb, ro, rc,
                          in_decay(t, own_ref), rf)
        rows.append({"t": t, "rel_db": ff.peak_db(x, sr, t) - typical_db, "abs_db": ff.peak_db(x, sr, t),
                     "rise_own": ro, "rise_full": rf, "rise_hp": rh,
                     "rise_snare": flags["snare"][0], "dom_snare": flags["snare"][1],
                     "rise_kick": flags["kick"][0], "dom_kick": flags["kick"][1], "rise_cym": rc,
                     "other_tom_ms": near_tom * 1000, "other_found": other_found, "src_mic": src_mic, "pair_diff_db": pair_diff, "snare_ms": sn * 1000,
                     "kick_ms": ki * 1000, "after_ms": after * 1000, "sample_ms": sample * 1000, "class": cls})
    return rows


# ---------------------------------------------------------------- question 3: the matched hits (features per hit)

def attack_ms(x, sr, t, span=0.030):
    """Milliseconds from `t` to the peak of |x| in [t, t + span]: how slowly the hit rises after the event time."""
    i0, i1 = max(0, int(t * sr)), min(len(x), int((t + span) * sr))
    return 1000.0 * float(np.abs(x[i0:i1]).argmax()) / sr if i1 > i0 else 0.0


def analyze_matched(state, events_out, x, y, sr, typical_db):
    """One row per matched hit of a tom mic: error (event - shifted reference, s), level against the typical hit, attack
    time, the app's `v` and `rise`, the gaps to the neighbouring output hits (ms)."""
    est = state["est"]
    rows = []
    for i, j in state["pairs"]:
        t = est[j]
        prev = est[j - 1] if j > 0 else None
        nxt = est[j + 1] if j + 1 < len(est) else None
        e = events_out[j]
        rows.append({"t": t, "err": t - state["shifted"][i], "rel_db": ff.peak_db(x, sr, t) - typical_db,
                     "abs_db": ff.peak_db(x, sr, t), "rise_own": stroke_rise(x, y, sr, t)[2], "attack_ms": attack_ms(x, sr, t), "v": e.get("v"), "rise_ev": e.get("rise"),
                     "gap_prev_ms": 1000 * (t - prev) if prev is not None else 9e3,
                     "gap_next_ms": 1000 * (nxt - t) if nxt is not None else 9e3})
    return rows


MOVE_S = fm.MOVE_S
MOTION_WIN_S = 0.0005       # the window for "the audio moves at this time" (max |x|)
ENV_BLOCK_S = 0.0005        # the envelope between the event and the reference: 0.5 ms block maxima
GAP_DB = 9.0                # a dip of this many dB after a higher level between the two times: two separate lobes
OVER_FLOOR_DB = 6.0         # the audio moves when it stands this far over the floor before the hit (`FM_FLOOR_DB`)
MOVED_CLASSES = ["app_late", "ref_early", "ref_precursor", "ref_late", "app_early"]
MOVED_TEXT = {
    "app_late": "the app is late; from the reference time the audio rises without a gap to the app's time: a slow attack the walk stops in",
    "ref_early": "the app is late; the audio at the reference time is under the floor plus 6 dB: the reference is early",
    "ref_precursor": "the app is late; a dip of 9 dB or more lies between the reference and the app: the reference sits on an earlier separate lobe",
    "ref_late": "the app is early; the audio rises without a gap from the app's time to the reference time: the reference is later on the slope",
    "app_early": "the app is early; the audio at its time is quiet, or a gap separates it from the reference"}


def motion_db(x, sr, tau, win=MOTION_WIN_S):
    """The level (dBFS) of max |x| in [tau - win / 2, tau + win / 2]."""
    i0, i1 = max(0, int((tau - win / 2) * sr)), min(len(x), int((tau + win / 2) * sr) + 1)
    return ff._db(np.abs(x[i0:i1]).max()) if i1 > i0 else -180.0


def hit_peak_db(x, sr, t, span=0.005):
    """The hit's peak: max |x| in [t, t + span] (the peak window of the back-track, `FM_PEAK_S`)."""
    i0, i1 = max(0, int(t * sr)), min(len(x), int((t + span) * sr))
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


def rewalk(x, sr, t, prev_t, next_t, peak_db=None, span_s=None, floor_margin_db=None):
    """The version 3 back-track again from the event time `t` (`onset_guard.walk`) with another peak threshold `peak_db`
    (dB under the peak; shipped 33), reach `span_s` (shipped 10 ms) and floor margin `floor_margin_db` (shipped 6 dB over
    the floor before the hit; on a noisy tom mic this term, not the peak term, sets the threshold). The new time, never
    later than `t`."""
    keep = (og.FM_PEAK_DB, og.FM_SPAN_S, og.FM_FLOOR_DB)
    try:
        if peak_db is not None:
            og.FM_PEAK_DB = peak_db
        if span_s is not None:
            og.FM_SPAN_S = span_s
        if floor_margin_db is not None:
            og.FM_FLOOR_DB = floor_margin_db
        s = int(round(t * sr))
        pm = int((prev_t + t) / 2 * sr) if prev_t is not None else None
        nm = int((t + next_t) / 2 * sr) if next_t is not None else None
        foot, how = og.walk(x, s, sr, pm, nm)
    finally:
        og.FM_PEAK_DB, og.FM_SPAN_S, og.FM_FLOOR_DB = keep
    return foot / sr if how == "moved" else t


def shift_score(errs_s, delta_s, move_s=MOVE_S):
    """(moved after a constant shift of the app's onsets by `delta_s`, hits that were right and are moved now, hits that were
    moved and are right now). The two-sided rule: a shift that makes a correct hit wrong is paid for."""
    e = np.asarray(errs_s, float)
    before = np.abs(e) > move_s + 1e-9
    after = np.abs(e + delta_s) > move_s + 1e-9
    return int(after.sum()), int((after & ~before).sum()), int((before & ~after).sum())


# the back-track variants replayed: name -> (peak dB under the hit, reach s, floor margin dB); None = shipped (33, 10 ms, 6)
WALKS = {"shipped": (None, None, None), "peak 40": (40.0, None, None), "floor 3": (None, None, 3.0),
         "floor 0": (None, None, 0.0), "floor 3 reach 15": (None, 0.015, 3.0), "floor 0 peak 40": (40.0, None, 0.0),
         "floor 0 peak 40 reach 15": (40.0, 0.015, 0.0)}


def control_pass_rate(x, y, sr, avoid, n=300, seed=1, min_gap=0.100):
    """The share of random times (at least `min_gap` from every time in `avoid`) at which the stroke test passes: the false
    alarm rate of the test on that mic (silence counts as a fail)."""
    rng = np.random.default_rng(seed)
    av = np.sort(np.asarray(avoid, float))
    dur = len(x) / sr
    hit = tot = 0
    for t in rng.uniform(1.0, dur - 1.0, size=n * 3):
        if nearest_abs(av, t) < min_gap:
            continue
        tot += 1
        hit += stroke_rise(x, y, sr, t)[2] >= RISE_MIN_DB
        if tot >= n:
            break
    return hit, tot


def analyze_walks(state, all_times, x, sr, walks=None):
    """For every matched hit the error (s) after the back-track is run again with each variant of `walks` ({name: (peak
    dB, reach s)}); `all_times`: the track's event times with the flagged ones (the walk's neighbours). Aligned with
    `state["pairs"]`."""
    walks = walks or WALKS
    at = sorted(all_times)
    est, sh = state["est"], state["shifted"]
    out = []
    for i, j in state["pairs"]:
        t = est[j]
        k = int(np.searchsorted(at, t - 1e-9))
        prev_t = at[k - 1] if k > 0 else None
        nxt = [a for a in at[k:] if a > t + 1e-9]
        row = {}
        for name, (db, reach, fl) in walks.items():
            row[name] = rewalk(x, sr, t, prev_t, nxt[0] if nxt else None, db, reach, fl) - sh[i]
        out.append(row)
    return out


# ---------------------------------------------------------------- question 4: candidate rules (pure parts)

FIT = ["Session A", "Session F"]     # the two songs with an independent reference per tom: the fit; the rest is held out
HELD = [s for s in SONGS if s not in FIT]


def cut_eval(rows, cut_db, key="rel_db"):
    """A loudness cut for toms at `cut_db` under the mic's typical hit: (false hits it removes, matched hits it loses).
    `rows`: dicts with `status` ("matched" or "false") and the level `key`."""
    gone = [r for r in rows if r[key] <= cut_db]
    return sum(1 for r in gone if r["status"] == "false"), sum(1 for r in gone if r["status"] == "matched")


def wta_drop(events, window_s=NEAR_S, margin_db=0.0, key="rel_db"):
    """The indices a winner-takes-all rule drops: of two output hits of different tom mics within `window_s`, the one whose
    level `key` is lower by `margin_db` or more. An event is dropped when a kept event of another mic lies within the
    window with a level at least `margin_db` higher; events are visited loudest first, so a chain keeps its loudest.
    `events`: dicts with `mic`, `t`, and the level `key`."""
    order = sorted(range(len(events)), key=lambda i: -events[i][key])
    kept, dropped = [], set()
    for i in order:
        e = events[i]
        win = [k for k in kept if events[k]["mic"] != e["mic"] and abs(events[k]["t"] - e["t"]) <= window_s
               and events[k][key] - e[key] >= margin_db]
        if win:
            dropped.add(i)
        else:
            kept.append(i)
    return dropped


def wta_eval(events, window_s=NEAR_S, margin_db=0.0, key="rel_db"):
    """(false hits dropped, matched hits dropped) of `wta_drop`; `events` also carry `status`."""
    d = wta_drop(events, window_s, margin_db, key)
    return (sum(1 for i in d if events[i]["status"] == "false"), sum(1 for i in d if events[i]["status"] == "matched"))


def wta_detail(events, window_s=NEAR_S, margin_db=0.0, key="rel_db"):
    """`wta_eval` with the lost matched hits split: `unison` (every louder hit that took its place is matched too: two real
    strokes, one lost) and `swap` (a louder partner is a false hit: the reference may have given the stroke to the wrong tom,
    the rule keeps the louder mic's hit). Returns {"false", "unison", "swap"}."""
    d = wta_drop(events, window_s, margin_db, key)
    out = {"false": 0, "unison": 0, "swap": 0}
    for i in d:
        e = events[i]
        if e["status"] == "false":
            out["false"] += 1
            continue
        partners = [events[k] for k in range(len(events)) if k not in d and events[k]["mic"] != e["mic"]
                    and abs(events[k]["t"] - e["t"]) <= window_s and events[k][key] - e[key] >= margin_db]
        out["swap" if any(p["status"] == "false" for p in partners) else "unison"] += 1
    return out


def output_median_levels(rows):
    """`rel_db` of every output row against the median level of its mic's output hits (matched and false): a reference free
    typical level, the one an app could use. `rows`: dicts with `song`, `mic`, `abs_db`; returns a new list with `rel_out`."""
    med = {}
    for r in rows:
        med.setdefault((r["song"], r["mic"]), []).append(r["abs_db"])
    med = {k: float(np.median(v)) for k, v in med.items()}
    return [dict(r, rel_out=r["abs_db"] - med[(r["song"], r["mic"])]) for r in rows]


def shift_fit(errs_by_song, fit, grid_ms):
    """The constant shift (ms) of `grid_ms` that gives the fewest moved hits on the `fit` songs (ties: the smaller
    shift), and its moved counts per song. `errs_by_song`: {song: [error s]}."""
    best = None
    for g in grid_ms:
        n = sum(shift_score(errs_by_song[s], g / 1000.0)[0] for s in fit)
        if best is None or n < best[0] or (n == best[0] and abs(g) < abs(best[1])):
            best = (n, g)
    return best[1]


def pick_variant(act, thr=None, rules=None):
    """The role's kept pick times (s) of the tom class with its threshold `thr` (shipped 0.32) and peak rules `rules`
    (`miss_causes.SHIPPED` keys: `avg_off`, `pre_avg` ...)."""
    th = list(mc.THRESHOLDS)
    if thr is not None:
        th[mc.ROLE_CLASSES[ROLE][0]] = thr
    picks, _ = mc.all_picks(act, th, rules)
    return mc.role_dedup(picks, ROLE)[0]


def avg_chain_picks(act, base, tol=ff.CHAIN_TOL, nmin=ff.CHAIN_MIN):
    """The shipped picks plus the picks of the peak rules with the moving average off that lie in a run of `nmin` or more
    equally spaced picks (the roll fill's run test; the average is what hides the soft strokes of a roll)."""
    cand = pick_variant(act, None, {"avg_off": True})
    allp = sorted(set(cand) | set(base))
    keep = ff.chains(allp, tol, nmin)
    have = set(base)
    return [t for t, k in zip(allp, keep) if k or t in have]


def variant_events(base_picks, base_events, var_picks, tol=1e-6):
    """The event times of a variant's picks: a pick the shipped run also made keeps its event time, a new pick gets
    `pick + lead` (the track's median of event minus pick). `base_events` sorted, one per base pick."""
    lead = float(np.median([e - p for p, e in zip(base_picks, base_events)])) if len(base_picks) else mc.LEAD_S
    bp = np.asarray(base_picks, float)
    out = []
    for t in var_picks:
        k = int(np.abs(bp - t).argmin()) if len(bp) else -1
        out.append(float(base_events[k]) if k >= 0 and abs(bp[k] - t) <= tol else t + lead)
    return sorted(out)


# ---------------------------------------------------------------- the songs

def analyze_song(song, refs, run, table, out, log=print):
    """Missed, false and matched rows of every tom mic of one song: {mic: {"missed", "false", "matched", "state"}}."""
    cfg = table[song]
    mics = refs[song]["mics"]
    toms = tom_mics(refs, song)
    states = {m: track_state(mics[m]["onsets"], run["events"][m]) for m in toms}
    sig = {}
    for m, (stem, _role) in cfg["mics"].items():
        if m in mics:
            x, sr = ff.read_stem(cfg, stem)
            sig[m] = (x, sr)
    cym = [(x, sr) for x, sr in (v for v in read_all(cfg, CYMBAL_STEMS.get(song, [])).values())]
    sample_on = []
    if song in SAMPLE_STEMS:
        sample_on, _ = fm.read_reference(cfg, SAMPLE_STEMS[song])
    shifted_role = {}
    src = {}
    for kind in ("snare", "kick"):
        ms = [m for m, v in mics.items() if v["role"] == kind]
        t_all = []
        for m in ms:
            off = fm.track_offset(mics[m]["onsets"], run["events"][m])
            t_all += [r + off for r in mics[m]["onsets"]]
        shifted_role[kind] = sorted(t_all)
        if ms and ms[0] in sig and sig[ms[0]][0] is not None:
            x, sr = sig[ms[0]]
            src[kind] = (x, sr, float(np.median([ff.peak_db(x, sr, t) for t in shifted_role[kind]])))
    res = {}
    for m in toms:
        x, sr = sig[m]
        y = highpass(x, sr)
        st = states[m]
        matched_t = sorted(st["est"][j] for _, j in st["pairs"])
        typical = float(np.median([ff.peak_db(x, sr, t) for t in matched_t])) if matched_t else -20.0
        act = mc.read_activation(Path(out) / "work" / tid(song, m) / "model" / "act.bin")
        ev_out = [e for e in run["events"][m] if not e.get("bleed") and not e.get("cut")]
        ctx = {"snare": shifted_role["snare"], "kick": shifted_role["kick"],
               "toms": {o: states[o]["shifted"] for o in toms if o != m}, "sample": [s + st["off"] for s in sample_on],
               "src": src, "cym": cym, "tom_sig": {o: sig[o] for o in toms if o != m}}
        mv = moved_audio(st, x, sr)
        wk = analyze_walks(st, [e["t"] for e in run["events"][m]], x, sr)
        matched_rows = analyze_matched(st, ev_out, x, y, sr, typical)
        for r_, a_, w_ in zip(matched_rows, mv, wk):
            r_.update(a_)
            r_["walk_err"] = w_
            if abs(r_["err"]) > MOVE_S + 1e-9:
                r_["moved_class"] = moved_class(r_["err"], a_["m_db"], a_["floor_db"], a_["dip_db"])
        res[m] = {"missed": analyze_missed(song, m, st, states, x, y, sr, act, typical),
                  "false": analyze_false(song, m, st, states, x, y, sr, ctx, typical),
                  "matched": matched_rows,
                  "control": control_pass_rate(x, y, sr, st["est"] + st["shifted"]),
                  "off_ms": st["off"] * 1000, "n_ref": len(st["shifted"]), "n_out": len(st["est"]),
                  "flagged": len(st["flagged"]), "typical_db": typical}
        log(f"{song} {m}: missed {len(res[m]['missed'])}, false {len(res[m]['false'])}, matched {len(res[m]['matched'])}")
    return res


def do_songs(out, src=SRC, log=print):
    out = Path(out)
    refs, runs = load_inputs(src)
    table = fm.songs()
    allres = {}
    for song in SONGS:
        allres[song] = analyze_song(song, refs, runs[song], table, out, log)
    (out / "tracks.json").write_text(json.dumps(allres))
    return allres


# ---------------------------------------------------------------- question 4: the replay of candidates (rule version 2 on the picks)

PICK_VARIANTS = {
    "thr 0.28": {"thr": 0.28}, "thr 0.24": {"thr": 0.24}, "thr 0.20": {"thr": 0.20}, "thr 0.16": {"thr": 0.16},
    "thr 0.12": {"thr": 0.12}, "average off": {"rules": {"avg_off": True}},
    "average off, thr 0.40": {"thr": 0.40, "rules": {"avg_off": True}}, "average 3 frames": {"rules": {"pre_avg": 3}},
    "average off, in a run": {"avg_chain": True}, "roll fill": {"roll": True}}
# the rule marks a hit when its excess over the source's leak is UNDER the threshold: a higher threshold marks more
RULE_GRID = [(sk.SNARE_DB, sk.KICK_DB), (sk.SNARE_DB, 6.0), (sk.SNARE_DB, 8.0), (sk.SNARE_DB, 10.0), (11.0, sk.KICK_DB),
             (13.0, sk.KICK_DB), (11.0, 6.0), (13.0, 8.0)]


def score_outputs(shifted_ref, out_times):
    """(matched, false, missed) of output times against the shifted references (the P4-16 match)."""
    n = len(fm.match(shifted_ref, out_times))
    return {"matched": n, "false": len(out_times) - n, "missed": len(shifted_ref) - n}


def replay_song(song, refs, run, table, act_dir, log=print):
    """The candidate replays of one song: {name: {mic: counts}} for the shipped rule (`app`, equal to the app's flags),
    the rule version 2 grid (`rule snare/kick dB`) and the pick variants (rule version 2 on the changed picks)."""
    cfg = table[song]
    mics = refs[song]["mics"]
    toms = tom_mics(refs, song)
    sig = {}
    for m, (stem, _r) in cfg["mics"].items():
        if m in run["events"]:
            sig[m] = ff.read_stem(cfg, stem)
    sr = sig[toms[0]][1]
    off = {m: fm.track_offset(mics[m]["onsets"], run["events"][m]) for m in toms}
    shifted = {m: [r + off[m] for r in mics[m]["onsets"]] for m in toms}
    src_roles = {r: m for m, (_s, r) in cfg["mics"].items() if r in ("snare", "kick") and m in run["events"]}
    sources = {r: (sig[m][0], np.asarray([e["t"] for e in run["events"][m]], float)) for r, m in src_roles.items()}
    tsig = {m: sig[m][0] for m in toms}
    base_ev = {m: sorted(float(e["t"]) for e in run["events"][m]) for m in toms}
    res = {}

    def run_rule(times, grid):
        r, exc, _notes, marks = sk.rule_v2(tsig, {m: np.asarray(t, float) for m, t in times.items()}, sources, sr)
        out = {}
        for g in grid:
            mk = marks if g is None else sk.rule_sk(r, exc, g[0], g[1])
            out[g] = {m: [t for t, b in zip(times[m], mk[m]) if not b] for m in times}
        return out, marks

    out, marks = run_rule(base_ev, [None] + RULE_GRID)
    app_flags = {m: [bool(e.get("bleed")) for e in sorted(run["events"][m], key=lambda e: e["t"])] for m in toms}
    same = sum(int(a == bool(b)) for m in toms for a, b in zip(app_flags[m], marks[m].tolist()))
    total = sum(len(v) for v in app_flags.values())
    res["app"] = {m: score_outputs(shifted[m], out[None][m]) for m in toms}
    res["_same"] = [same, total]
    for g in RULE_GRID:
        res[f"rule {g[0]:.1f}/{g[1]:.1f}"] = {m: score_outputs(shifted[m], out[g][m]) for m in toms}
    acts = {m: mc.read_activation(Path(act_dir) / "work" / tid(song, m) / "model" / "act.bin") for m in toms}
    base_picks = {}
    for m in toms:
        pk, _, _, _ = mc.effective_picks(acts[m], ROLE)
        base_picks[m] = mc.role_dedup(pk, ROLE)[0]
    for name, v in PICK_VARIANTS.items():
        times = {}
        for m in toms:
            if v.get("roll"):
                vp = ff.chain_picks(acts[m], ROLE)
            elif v.get("avg_chain"):
                vp = avg_chain_picks(acts[m], base_picks[m])
            else:
                vp = pick_variant(acts[m], v.get("thr"), v.get("rules"))
            times[m] = variant_events(base_picks[m], base_ev[m], vp)
        o, _ = run_rule(times, [None])
        res[name] = {m: score_outputs(shifted[m], o[None][m]) for m in toms}
    log(f"{song}: replay done, app flags = rule_v2 {same} of {total}")
    return res


def do_candidates(out, src=SRC, log=print):
    out = Path(out)
    refs, runs = load_inputs(src)
    table = fm.songs()
    res = {s: replay_song(s, refs, runs[s], table, out, log) for s in SONGS}
    (out / "candidates.json").write_text(json.dumps(res))
    return res


# ---------------------------------------------------------------- the report

def md(head, body):
    return fm.md_table(head, body)


def flat(data, kind):
    """Every row of `kind` ("missed", "false", "matched") with its song and mic."""
    return [dict(r, song=s, mic=m) for s, v in data.items() for m, t in v.items() for r in t[kind]]


def pct(v, q):
    return float(np.percentile(v, q)) if len(v) else float("nan")


def class_table(rows, classes, label="class"):
    """A table of counts: one row per song plus the total, one column per class (columns with no hit are left out)."""
    songs = [s for s in SONGS if any(r["song"] == s for r in rows)]
    used = [c for c in classes if any(r["class"] == c for r in rows)]
    body = [[s] + [sum(1 for r in rows if r["song"] == s and r["class"] == c) for c in used]
            + [sum(1 for r in rows if r["song"] == s)] for s in songs]
    body.append(["**all**"] + [f"**{sum(1 for r in rows if r['class'] == c)}**" for c in used] + [f"**{len(rows)}**"])
    return md(["song"] + used + ["all"], body)


def section_missed(data):
    miss, mat = flat(data, "missed"), flat(data, "matched")
    L = [f"### Question 1: the missed tom hits ({len(miss)})", "", class_table(miss, MISS_CLASSES), "",
         md(["class", "meaning"], [[c, MISS_TEXT[c]] for c in MISS_CLASSES if any(r["class"] == c for r in miss)]), ""]
    body = []
    for c in ("threshold", "average", "role", "blind"):
        v = [r["act_peak"] for r in miss if r["class"] == c]
        if v:
            body.append([c, len(v), f"{pct(v, 10):.2f}", f"{pct(v, 50):.2f}", f"{pct(v, 90):.2f}",
                         sum(1 for a in v if a >= 0.20)])
    L.append("Activation of the tom column at the reference time (peak in [t - 30 ms, t + 10 ms]; the pick threshold is 0.32):")
    L += ["", md(["class", "hits", "p10", "median", "p90", "0.20 or more"], body)]
    st = [r for r in miss if r["class"] in ("threshold", "average", "role", "blind")]
    L += ["", f"- The {len(st)} model-stage misses by song: " + ", ".join(
        f"{s} {sum(1 for r in st if r['song'] == s)}" for s in SONGS if any(r["song"] == s for r in st)) + ". Their level against "
        f"the mic's typical hit: median {pct([r['rel_db'] for r in st], 50):+.1f} dB; in a close pair (a reference "
        f"neighbour within 50 ms) {sum(1 for r in st if r['neighbour_ms'] <= 50)}, within 300 ms "
        f"{sum(1 for r in st if r['neighbour_ms'] <= 300)}.", ""]
    dense = [r for r in mat if r["gap_prev_ms"] < 300]
    sparse = [r for r in mat if r["gap_prev_ms"] >= 300]
    ctl = [tuple(t["control"]) for v in data.values() for t in v.values()]
    ch, ct = sum(c[0] for c in ctl), sum(c[1] for c in ctl)
    L += [f"**Reference-error test and its controls.** The stroke test is the rise of 10 dB or more (P4-18) on the full band or on "
          f"the 2 kHz high-pass (the full band alone fails inside rolls: the ring of the earlier hit hides the rise). Matched "
          f"hits fail it: {sum(1 for r in dense if r['rise_own'] < RISE_MIN_DB)} of {len(dense)} when the previous hit is "
          f"under 300 ms away, {sum(1 for r in sparse if r['rise_own'] < RISE_MIN_DB)} of {len(sparse)} otherwise. Random times "
          f"(at least 100 ms from any hit) pass it: {ch} of {ct}. So `no_stroke` is a reference error; in a run the test has a false-fail rate of about 5 %, so a miss that fails it is still very "
          f"probably without a stroke, and a roll of very soft strokes is the one case it cannot exclude.", ""]
    ns = [r for r in miss if r["class"] == "no_stroke"]
    L += [f"`no_stroke` ({len(ns)}): {sum(1 for r in ns if r['neighbour_ms'] <= 300)} have a reference neighbour within 300 ms; "
          f"rise on the full band median {pct([r['rise_full'] for r in ns], 50):.1f} dB, on the 2 kHz band "
          f"{pct([r['rise_hp'] for r in ns], 50):.1f} dB (matched hits: {pct([r['rise_own'] for r in mat], 50):.1f} dB).", ""]
    return L


def section_false(data):
    fal = flat(data, "false")
    L = [f"### Question 2: the false tom hits ({len(fal)})", "", class_table(fal, FALSE_CLASSES), "",
         md(["class", "group", "meaning"], [[c, FALSE_GROUP[c], FALSE_TEXT[c]] for c in FALSE_CLASSES
                                           if any(r["class"] == c for r in fal)]), ""]
    groups = {}
    for r in fal:
        groups.setdefault(FALSE_GROUP[r["class"]], []).append(r)
    body = [[g] + [sum(1 for r in v if r["song"] == s) for s in SONGS] + [len(v)] for g, v in sorted(groups.items())]
    L += [md(["group"] + SONGS + ["all"], body), ""]
    ot = [r for r in fal if r["class"] == "other_tom"]
    loud = [r for r in ot if r["pair_diff_db"] is not None and r["pair_diff_db"] >= -6.0]
    small = [r for r in ot if r["pair_diff_db"] is not None and r["pair_diff_db"] < -6.0]
    L += [f"**Tom to tom.** {len(ot)} false hits have a reference hit of another tom within 12 ms; the other tom's hit is found "
          f"(matched) in {sum(1 for r in ot if r['other_found'])} of them. The level of the false hit on its own mic against the "
          f"level of the other tom's mic at the same time: median {pct([r['pair_diff_db'] for r in ot if r['pair_diff_db'] is not None], 50):+.1f} dB "
          f"(the hits the bleed rule flagged: about -14 dB). Bleed-sized (under -6 dB): {len(small)}; as loud as the source or "
          f"louder (-6 dB or more; the tom was struck too, or the reference gave the hit to the other tom): {len(loud)}. "
          f"By song, bleed-sized / loud: " + ", ".join(
              f"{s} {sum(1 for r in small if r['song'] == s)} / {sum(1 for r in loud if r['song'] == s)}"
              for s in SONGS) + ".", ""]
    nr = [r for r in fal if r["class"] == "no_reference"]
    if nr:
        L += [f"**Mics without a reference** ({len(nr)}: " + ", ".join(f"{s} {m}" for s, m in sorted({(r['song'], r['mic']) for r in nr}))
              + f"): rise on the own mic median {pct([r['rise_own'] for r in nr], 50):.1f} dB; "
              f"a tom sample onset within 6 ms: {sum(1 for r in nr if r['sample_ms'] <= SAMPLE_TOL_S * 1000)} of {len(nr)}.", ""]
    body = []
    for c in FALSE_CLASSES:
        v = [r for r in fal if r["class"] == c]
        if v:
            body.append([c, len(v), f"{pct([r['rel_db'] for r in v], 50):+.1f}", f"{pct([r['rise_own'] for r in v], 50):.1f}",
                         f"{pct([r['rise_snare'] for r in v if r['rise_snare'] is not None], 50):.1f}"
                         if any(r['rise_snare'] is not None for r in v) else "-"])
    L += ["Level and rise per class (dB; level against the mic's typical matched hit; rise on the own mic and on the snare mic):", "",
          md(["class", "hits", "level median", "rise own median", "rise snare median"], body), ""]
    return L


def section_moved(data):
    mat = flat(data, "matched")
    mv = [r for r in mat if abs(r["err"]) > MOVE_S + 1e-9]
    L = [f"### Question 3: the moved tom hits ({len(mv)} of {len(mat)} matched)", ""]
    bins = ["1 to 2", "2 to 5", "over 5"]
    body = []
    for s in SONGS:
        rs = [r for r in mv if r["song"] == s]
        body.append([s, len([r for r in mat if r["song"] == s])] + [
            f"{sum(1 for r in rs if r['err'] > 0 and size_bin(r['err'] * 1000) == b)} / "
            f"{sum(1 for r in rs if r['err'] < 0 and size_bin(r['err'] * 1000) == b)}" for b in bins] + [len(rs)])
    body.append(["**all**", len(mat)] + [
        f"**{sum(1 for r in mv if r['err'] > 0 and size_bin(r['err'] * 1000) == b)} / "
        f"{sum(1 for r in mv if r['err'] < 0 and size_bin(r['err'] * 1000) == b)}**" for b in bins] + [f"**{len(mv)}**"])
    L += ["Size and sign (cells: app late / app early; late = the app's onset is after the reference's):", "",
          md(["song", "matched"] + [f"{b} ms" for b in bins] + ["moved"], body), ""]
    for r in mv:
        r["class"] = r["moved_class"]
    L += [class_table(mv, MOVED_CLASSES), "", md(["class", "meaning"], [[c, MOVED_TEXT[c]] for c in MOVED_CLASSES]), ""]
    body = [[c] + [sum(1 for r in mv if r["class"] == c and size_bin(r["err"] * 1000) == b) for b in bins]
            for c in MOVED_CLASSES]
    L += [md(["class"] + [f"{b} ms" for b in bins], body), ""]
    body = []
    for name, sel in [("all matched", mat), ("previous hit under 150 ms", [r for r in mat if r["gap_prev_ms"] < 150]),
                      ("150 to 300 ms", [r for r in mat if 150 <= r["gap_prev_ms"] < 300]),
                      ("300 to 600 ms", [r for r in mat if 300 <= r["gap_prev_ms"] < 600]),
                      ("600 ms or more", [r for r in mat if r["gap_prev_ms"] >= 600]),
                      ("level 15 dB or more under the typical hit", [r for r in mat if r["rel_db"] < -15]),
                      ("level 15 to 8 dB under", [r for r in mat if -15 <= r["rel_db"] < -8]),
                      ("level within 3 dB of the typical hit or louder", [r for r in mat if r["rel_db"] >= -3]),
                      ("sharp, isolated and strong (peak within 1.5 ms, gap 300 ms or more, level -6 dB or more)",
                       [r for r in mat if r["attack_ms"] <= 1.5 and r["gap_prev_ms"] >= 300 and r["rel_db"] >= -6])]:
        e = np.array([r["err"] for r in sel]) * 1000
        body.append([name, len(e), f"{pct(e, 10):+.2f}", f"{pct(e, 50):+.2f}", f"{pct(e, 90):+.2f}",
                     f"{100 * float(np.mean(np.abs(e) > 1.0 + 1e-6)):.0f} %" if len(e) else "-"])
    L += ["Error (ms, after the mic's offset) by crowding and level; the last row is the nearest thing to a control for the "
          "reference's own jitter (a sharp, isolated, strong hit: the app is exact there, P4-11):", "",
          md(["hits", "n", "p10", "median", "p90", "beyond 1 ms"], body), ""]
    body = []
    for pos in ("t1", "t2", "t3", "t4"):
        offs = [(s, data[s][pos]["off_ms"]) for s in SONGS if pos in data[s] and data[s][pos]["n_ref"] > 0]
        if offs:
            body.append([pos, ", ".join(f"{o:.1f}" for _, o in offs), f"{min(o for _, o in offs):.1f} to {max(o for _, o in offs):.1f}"])
    L += ["The reference's latency per mic (the offset the yardstick removes, ms; one value per song in the order of the song "
          "table): it is not a constant of the tom.", "", md(["mic", "offsets", "range"], body), ""]
    errs = {s: [r["err"] for r in mat if r["song"] == s] for s in SONGS}
    grid = [-1.0, -0.5, -0.25, 0.0, 0.25, 0.5, 1.0]
    body = []
    for g in grid:
        row = [f"{g:+.2f}"]
        tw = 0
        for s in SONGS:
            n, w, b = shift_score(errs[s], g / 1000.0)
            row.append(n)
            tw += w
        row += [sum(shift_score(errs[s], g / 1000.0)[0] for s in SONGS), tw]
        body.append(row)
    best = shift_fit(errs, FIT, [x / 20 for x in range(-40, 41)])
    held = [shift_score(errs[s], best / 1000.0)[0] for s in HELD]
    held0 = [shift_score(errs[s], 0.0)[0] for s in HELD]
    L += ["A constant shift of all tom onsets (the reference offset kept at its baseline value; moved hits after the shift; "
          "`new` = hits that were right and are moved now):", "",
          md(["shift ms"] + SONGS + ["all", "new"], body), "",
          f"Best shift on the fit songs ({', '.join(FIT)}): {best:+.2f} ms; held-out moved {sum(held)} against {sum(held0)} "
          f"unshifted.", ""]
    names = list(next(iter(mat))["walk_err"].keys())
    body = []
    for n in names:
        row = [n]
        for group in (FIT, HELD):
            rs = [r for r in mat if r["song"] in group]
            b0 = sum(1 for r in rs if abs(r["err"]) > MOVE_S + 1e-9)
            a = [abs(r["walk_err"][n]) > MOVE_S + 1e-9 for r in rs]
            worse = sum(1 for r, x in zip(rs, a) if x and abs(r["err"]) <= MOVE_S + 1e-9)
            better = sum(1 for r, x in zip(rs, a) if not x and abs(r["err"]) > MOVE_S + 1e-9)
            row += [f"{b0} to {sum(a)}", f"+{worse} / -{better}"]
        body.append(row)
    L += ["The back-track again with another setting (version 3 walk from the event time; `onset_guard.walk`), moved hits "
          "before to after, and the two-sided count (right hits made wrong / moved hits made right):", "",
          md(["walk", "fit: moved", "fit: wrong / right", "held out: moved", "held out: wrong / right"], body), ""]
    return L


def md_rate(a, b):
    return f"{a} of {b}"


def section_rules(data, cand):
    L = ["### Question 4: rule candidates", ""]
    fal = flat(data, "false")
    mat = flat(data, "matched")
    out_rows = output_median_levels([dict(r, status="false") for r in fal] + [dict(r, status="matched") for r in mat])
    body = []
    for cut in (-12.0, -15.0, -18.0, -21.0):
        row = [f"{cut:.0f} dB"]
        for group in (FIT, HELD):
            a, b = cut_eval([r for r in out_rows if r["song"] in group], cut, "rel_out")
            row.append(f"-{a} false / +{b} missed")
        body.append(row)
    L += ["Loudness cut for toms: a hit under the median level of its mic's output hits by this much goes (the level is "
          "measured on the output, no reference needed): false hits removed / matched hits lost, fit songs "
          f"({', '.join(FIT)}) and held-out songs.", "", md(["cut", "fit", "held out"], body), ""]
    body = []
    for key in ("rel_out", "abs_db"):
        for margin in (0.0, 3.0, 6.0):
            row = [f"{key}, margin {margin:.0f} dB"]
            for group in (FIT, HELD):
                tot = {"false": 0, "unison": 0, "swap": 0}
                for s in group:
                    d = wta_detail([r for r in out_rows if r["song"] == s], NEAR_S, margin, key)
                    for k in tot:
                        tot[k] += d[k]
                row.append(f"-{tot['false']} false / +{tot['unison']} unison, +{tot['swap']} swap")
            body.append(row)
    L += ["Winner takes all (two output hits of different tom mics within 12 ms: the lower one goes). Cells: false hits dropped / "
          "matched hits dropped whose louder partner is a matched hit too (`unison`: two real strokes, one lost) / matched hits "
          "dropped whose louder partner is a false hit (`swap`: the reference may have given the stroke to the wrong tom). "
          f"`rel_out` = level against the median of the mic's output hits, `abs_db` = the raw peak. Fit songs ({', '.join(FIT)}) and held-out "
          "songs:", "", md(["level", "fit", "held out"], body), ""]
    body = []
    for s in SONGS:
        d = wta_detail([r for r in out_rows if r["song"] == s], NEAR_S, 6.0, "rel_out")
        body.append([s, d["false"], d["unison"], d["swap"], d["false"] - d["unison"] - d["swap"]])
    L += ["Winner takes all, `rel_out` with a margin of 6 dB, per song (net = false dropped minus matched dropped):", "",
          md(["song", "false dropped", "unison lost", "swap lost", "net fixes saved"], body), ""]
    if cand:
        names = [k for k in cand[SONGS[0]] if k not in ("app", "_same")]
        body = []
        for n in names:
            row = [n]
            for group in (FIT, HELD):
                f = sum(sum(v["false"] for v in cand[s][n].values()) - sum(v["false"] for v in cand[s]["app"].values()) for s in group)
                mi = sum(sum(v["missed"] for v in cand[s][n].values()) - sum(v["missed"] for v in cand[s]["app"].values()) for s in group)
                row.append(f"{f:+d} false, {mi:+d} missed = {f + mi:+d}")
            body.append(row)
        L += ["Replays with the app's tom bleed rule (version 2) on the changed picks or settings; cells are the change in "
              "false and missed hits against the shipped replay (moved hits of new hits are not measured):", "",
              md(["candidate", "fit songs", "held-out songs"], body), "",
              "App flags equal the replay of rule version 2: " + ", ".join(f"{s} {cand[s]['_same'][0]} of {cand[s]['_same'][1]}"
                                                                           for s in SONGS) + ".", ""]
    return L


def do_report(out, log=print):
    out = Path(out)
    data = json.loads((out / "tracks.json").read_text())
    cand = json.loads((out / "candidates.json").read_text()) if (out / "candidates.json").is_file() else None
    L = section_missed(data) + section_false(data) + section_moved(data) + section_rules(data, cand)
    (out / "report.md").write_text("\n".join(L))
    log("\n".join(L))
    return L


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["run", "songs", "candidates", "missed", "false", "moved", "rules", "all"])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--src", default=str(SRC))
    a = ap.parse_args(argv)
    out = Path(a.out)
    if a.cmd in ("run", "all"):
        do_run(out, a.src)
    if a.cmd in ("songs", "all"):
        do_songs(out, a.src)
    if a.cmd in ("candidates", "all"):
        do_candidates(out, a.src)
    if a.cmd in ("missed", "false", "moved", "rules", "all"):
        data = json.loads((out / "tracks.json").read_text())
        cand = json.loads((out / "candidates.json").read_text()) if (out / "candidates.json").is_file() else None
        part = {"missed": section_missed, "false": section_false, "moved": section_moved}
        if a.cmd == "all":
            do_report(out)
        elif a.cmd == "rules":
            print("\n".join(section_rules(data, cand)))
        else:
            print("\n".join(part[a.cmd](data)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
