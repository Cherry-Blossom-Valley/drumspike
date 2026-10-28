"""P4-18 (diagnosis only): the largest firm fixes of kick and snare in the P4-16 yardstick.

    uv run --with numpy --with soundfile --with mido python tools/firm_fixes.py run      # the model's activations of the snare tracks (and Session A's kick)
    uv run --with numpy --with soundfile --with mido python tools/firm_fixes.py report   # the tables

Input: `~/.cache/drumspike/fix-metric-v2/` (`refs.json`, `run/<song>_default.json`; written by `tools/fix_metric.py`).
Nothing under `crates/` changes. The sessions in `~/.cache/drumspike/sessions` are read only.

What it does per track (a role of one song):
  missed hits   one cause each (`tools/miss_causes.py` `classify_miss`: blind, threshold, lobe, average, role, refine,
                offset ...) plus a class: ghost note (level far under the median matched hit), flam or close pair
                (a reference neighbour within 50 ms), quiet section, other layer (a second snare layer says the same
                hit is there or not), reference error (the track's own audio has no onset there)
  false hits    one source each: another role's reference (kick bleed), a hat or cymbal onset, a snare layer onset
                (a real hit the reference misses), a double (a matched hit of the same track within 50 ms), a ghost
                note, a flam second stroke, or unexplained
  fix costs     the replay of a lower threshold or a cross-class rescue on all songs: hits found, false hits added
"""
import argparse
import json
import math
import os
import shutil
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fix_metric as fm  # noqa: E402
import miss_causes as mc  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SRC = Path.home() / ".cache" / "drumspike" / "fix-metric-v2"
OUT = Path.home() / ".cache" / "drumspike" / "firm-fixes"

# which tracks get an activation run: every snare mic of the seven songs and Session A's kick
TRACKS = [("Session A", "s top"), ("Session B", "s top"), ("Session C", "s top"), ("Session H", "s top"),
          ("Session F", "s top"), ("Session G", "s top"), ("Session D", "s top"),
          ("Session A", "k in")]


def tid(song, mic):
    return fm.track_name(song) + "_" + fm.track_name(mic)


# ---------------------------------------------------------------- run: the activations (the model only)

def do_run(out, log=print):
    """One model run of each track in TRACKS through `service_tool cli run`; the kept activation and the events go to
    `<out>/work/<id>/model/`. Resumes: a track with an `act.bin` is kept. Waits while the measure lock or the load say so."""
    out = Path(out)
    tool, model = mc.find_tool(), "none".resolve()
    if not tool.is_file():
        raise SystemExit(f"need the release service_tool ({tool}) and the model ({model})")
    table = fm.songs()
    (out / "audio").mkdir(parents=True, exist_ok=True)
    try:
        for song, mic in TRACKS:
            i = tid(song, mic)
            if (out / "work" / i / "model" / "act.bin").is_file():
                continue
            stem, role = table[song]["mics"][mic]
            link = out / "audio" / f"{i}.wav"
            if link.is_symlink() or link.exists():
                link.unlink()
            link.symlink_to(fm.audio_path(table[song], stem))
            fm.wait_for_room(log)
            fm.LOCK.write_text(f"firm_fixes run {i}\n")
            try:
                mc.run_item(tool, out, {"id": i, "audio": str(link), "role": role, "note": mc.ROLE_NOTE[role]},
                            "model", model)
            finally:
                fm.LOCK.unlink(missing_ok=True)
            log(f"ran {i}")
    finally:
        fm.LOCK.unlink(missing_ok=True)
        shutil.rmtree(out / "run-home", ignore_errors=True)
        shutil.rmtree(out / "run-projects", ignore_errors=True)


# ---------------------------------------------------------------- audio features (pure; the tests cover these)

PEAK_BEFORE_S, PEAK_AFTER_S = 0.005, 0.030      # the hit's level: peak in [t - 5 ms, t + 30 ms]
RISE_NEAR = (-0.002, 0.010)                     # the rise: peak near the onset over the peak 10 to 30 ms before it
RISE_FAR = (-0.030, -0.010)
RISE_MIN_DB = 10.0      # a real stroke rises at least this over what came before (a control of random times: -1.4 dB)
GHOST_DB = -15.0        # a hit this far under the track's median matched hit is soft (ghost note, soft roll stroke)
KICK_BLEED_S = 0.012    # an event this close to a reference hit of the other role is bleed of that drum
DOUBLE_MIN_S, DOUBLE_MAX_S = 0.012, 0.060   # a second stroke after a matched hit of the same track
LAYER_TOL_S = 0.004     # an onset of a sample layer this close confirms a hit
NEIGHBOUR_S = 0.050     # a reference hit this close to another one is a flam or a close pair
ROLL_TOL, ROLL_MIN = 0.12, 5    # a run of 5 or more hits with equal spacing (12 %) is a roll
SAMPLE_GAP = (0.08, 0.40)       # the spacing of a roll: 80 to 400 ms


def _db(v):
    return 20.0 * math.log10(max(float(v), 1e-9))


def peak_db(x, sr, t, before=PEAK_BEFORE_S, after=PEAK_AFTER_S):
    """Peak level (dBFS) of `x` in [t - before, t + after]."""
    i0, i1 = max(0, int((t - before) * sr)), min(len(x), int((t + after) * sr))
    return _db(np.abs(x[i0:i1]).max()) if i1 > i0 else -180.0


def rise_db(x, sr, t):
    """How far the peak around the onset (RISE_NEAR) stands over the peak before it (RISE_FAR): a stroke is 10 dB or
    more, the decay of an earlier hit or noise is about 0."""
    return peak_db(x, sr, t, -RISE_NEAR[0], RISE_NEAR[1]) - peak_db(x, sr, t, -RISE_FAR[0], RISE_FAR[1])


def nearest(times, t):
    """Signed distance (s) from `t` to the nearest of `times` (positive: it lies after `t`); 9.0 for none."""
    a = np.asarray(times, float)
    return float(a[np.abs(a - t).argmin()] - t) if len(a) else 9.0


def chains(times, tol=ROLL_TOL, nmin=ROLL_MIN, gap=SAMPLE_GAP):
    """Boolean mask over sorted `times`: the member of a run of at least `nmin` hits whose successive gaps lie within
    `tol` of the run's median gap, and inside `gap` (min, max) seconds. A roll or a steady pattern."""
    t = np.asarray(times, float)
    n = len(t)
    keep = np.zeros(n, bool)
    i = 0
    while i < n - 1:
        g0 = t[i + 1] - t[i]
        if not gap[0] <= g0 <= gap[1]:
            i += 1
            continue
        gaps, j = [g0], i + 1
        while j + 1 < n:
            g, m = t[j + 1] - t[j], float(np.median(gaps))
            if abs(g - m) > tol * m:
                break
            gaps.append(g)
            j += 1
        if j - i + 1 >= nmin:
            keep[i:j + 1] = True
        i = max(j, i + 1)
    return keep


def split_hits(shifted_ref, est):
    """(matched pairs, missed reference indices, false event indices) of the P4-16 match."""
    pairs = fm.match(shifted_ref, est)
    mr, me = {p[0] for p in pairs}, {p[1] for p in pairs}
    return pairs, [i for i in range(len(shifted_ref)) if i not in mr], [j for j in range(len(est)) if j not in me]


MISS_CLASSES = ["no_transient", "flam", "roll", "ghost", "normal"]


def miss_class(rel_db, rise_own, rise_wit, neighbour_s, in_roll):
    """The class of one missed hit, the first that fits: `no_transient` (the own mic and the witness mic show no
    stroke: a reference error), `flam` (a reference neighbour within 50 ms), `roll` (in a regular run of reference
    hits), `ghost` (15 dB or more under the median hit), `normal`. `rise_wit` None: no witness mic."""
    if rise_own < RISE_MIN_DB and (rise_wit is None or rise_wit < RISE_MIN_DB):
        return "no_transient"
    if abs(neighbour_s) <= NEIGHBOUR_S:
        return "flam"
    if in_roll:
        return "roll"
    return "ghost" if rel_db <= GHOST_DB else "normal"


FALSE_CLASSES = ["other_role_bleed", "layer_hit", "double", "ghost_note", "real_stroke", "hat_cymbal", "one_mic", "weak"]
EXTRA_RISE_DB = 15.0    # a hat, ride or overhead mic with this rise at the hit: a cymbal or hat stroke is there


def false_class(rel_db, rise_own, rise_wit, other_role_s, layer_s, matched_before_s, rise_extra=None):
    """The source of one false hit, the first that fits: `other_role_bleed` (a reference hit of the other role within
    12 ms), `layer_hit` (a sample layer has the hit: the trigger reference lacks it), `double` (12 to 60 ms after a
    matched hit of the same track: a second stroke, flam or ring), `ghost_note` (soft, 15 dB or more under the median,
    and a stroke on both mics), `real_stroke` (not soft, a stroke on both mics: the reference lacks it),
    `hat_cymbal` (no stroke on both mics, but a hat, ride or overhead mic rises 15 dB or more), `one_mic` (a stroke on
    the own mic only), `weak` (no stroke by the rise test on either mic). `matched_before_s`: seconds since the last
    matched hit, 9 for none; `rise_wit` None: no witness mic (then the own mic decides); `rise_extra` None: no
    hat or overhead mic."""
    if abs(other_role_s) <= KICK_BLEED_S:
        return "other_role_bleed"
    if abs(layer_s) <= LAYER_TOL_S:
        return "layer_hit"
    if DOUBLE_MIN_S <= matched_before_s <= DOUBLE_MAX_S:
        return "double"
    if rise_own >= RISE_MIN_DB and (rise_wit is None or rise_wit >= RISE_MIN_DB):
        return "ghost_note" if rel_db <= GHOST_DB else "real_stroke"
    if rise_extra is not None and rise_extra >= EXTRA_RISE_DB:
        return "hat_cymbal"
    return "one_mic" if rise_own >= RISE_MIN_DB else "weak"


def layer_offset(ref, layer_on, window=0.020):
    """The layer's latency against the reference: the median signed difference of the nearest layer onset to each
    reference hit within `window` (0 when none)."""
    d = [nearest(layer_on, r) for r in ref]
    d = [x for x in d if abs(x) <= window]
    return float(np.median(d)) if d else 0.0


def count_by(labels, order):
    """{label: n} in the order of `order`, zeros kept."""
    return {k: sum(1 for x in labels if x == k) for k in order}


# ---------------------------------------------------------------- fix candidates, replayed on the kept activations

CHAIN_LO, CHAIN_TOL, CHAIN_MIN = 0.05, 0.08, 8   # the roll fill candidate (chosen on the seven songs, see the report)
THRESHOLD_SWEEP = [0.20, 0.16, 0.12, 0.10, 0.08, 0.06]
CUT_SWEEP = [-12.0, -15.0, -18.0, -20.0]


def picks_at(act, thr, role="snare"):
    """The role's kept pick times (s) with the role's class threshold set to `thr` (the others stay)."""
    th = list(mc.THRESHOLDS)
    for c in mc.ROLE_CLASSES[role]:
        th[c] = thr
    p, _ = mc.all_picks(act, th)
    return mc.role_dedup(p, role)[0]


def chain_picks(act, role="snare", lo=CHAIN_LO, tol=CHAIN_TOL, nmin=CHAIN_MIN):
    """The picks of the shipped threshold plus the candidates at `lo` that lie in a run of `nmin` or more picks with
    equal spacing (`chains`): the roll fill. A candidate alone, or in a short or irregular run, stays out."""
    base = picks_at(act, mc.THRESHOLDS[mc.ROLE_CLASSES[role][0]], role)
    cand = picks_at(act, lo, role)
    allp = sorted(set(cand) | set(base))
    keep = chains(allp, tol, nmin)
    have = set(base)
    return [t for t, k in zip(allp, keep) if k or t in have]


def lead_of(events, picks):
    """The median of (event - its pick): a pick leads its refined event (10 ms on median); 0.010 for none."""
    d = []
    for e in events:
        c = [b for b in picks if e - 0.025 <= b <= e + 0.005]
        if c:
            d.append(e - min(c, key=lambda b: abs(b - (e - 0.010))))
    return float(np.median(d)) if d else mc.LEAD_S


def gain_false(shifted_ref, base_times, var_times, lead):
    """(more matched, more false) of a variant's picks against the base picks, both as pick + `lead`, on the
    P4-16 match. The replay of the base picks equals the app's output counts on all seven songs."""
    def score(ts):
        ev = [t + lead for t in ts]
        n = len(fm.match(shifted_ref, ev))
        return n, len(ev) - n
    b, v = score(base_times), score(var_times)
    return v[0] - b[0], v[1] - b[1]


def level_cut(rel_matched, rel_false, cut_db):
    """A loudness cut at `cut_db` under the median hit: (false hits it removes, matched hits it loses)."""
    return (sum(1 for r in rel_false if r <= cut_db), sum(1 for r in rel_matched if r <= cut_db))


# ---------------------------------------------------------------- the sessions: one track at a time

# the witness: a second mic of the same drum that the model, the references and the P4-16 yardstick do not use
WITNESS = {("Session A", "snare"): "s bot_02", ("Session B", "snare"): "s bot_01",
           ("Session G", "snare"): "s bot_01", ("Session D", "snare"): "s bot",
           ("Session A", "kick"): "k out_02"}
KICK_LAYERS = {"Session A": ["k sampl savior_01", "k sampl sd3_01"]}


def read_stem(cfg, stem):
    import learn_eval as le
    files = [fm.audio_path(cfg, stem)]
    if not files[0].is_file():
        files = [fm.audio_path(cfg, stem + ".L"), fm.audio_path(cfg, stem + ".R")]
    return le.read_mono(files) if files[0].is_file() else (None, None)


def layer_times(cfg, stems):
    """The onsets of the layer files (union, 5 ms merge), by file: {stem: onsets}."""
    import learn_eval as le
    out = {}
    for stem in stems:
        x, sr = read_stem(cfg, stem)
        if x is not None:
            out[stem] = le.ref_onsets(x, sr, gap=0.005).tolist()
    return out


def load_song(song, mic):
    refs = json.loads((SRC / "refs.json").read_text())
    run = json.loads((SRC / "run" / f"{fm.track_name(song)}_default.json").read_text())
    return refs, run


def analyze_track(song, mic, role, log=print):
    """Every false and missed hit of one track with its features and class. Returns a dict (JSON-able)."""
    import flam_pairs as fp
    refs, run = load_song(song, mic)
    cfg = fm.songs()[song]
    ref, ev = refs[song]["mics"][mic]["onsets"], run["events"][mic]
    off = fm.track_offset(ref, ev)
    est, shifted = fm.outputs(ev), [r + off for r in ref]
    pairs, missed, false = split_hits(shifted, est)
    x, sr = read_stem(cfg, cfg["mics"][mic][0])
    wit_stem = WITNESS.get((song, role))
    wx, wsr = read_stem(cfg, wit_stem) if wit_stem else (None, None)
    if wx is not None and wsr != sr:
        wx = None
    matched_t = sorted(est[j] for _, j in pairs)
    med = float(np.median([peak_db(x, sr, t) for t in matched_t])) if matched_t else -20.0
    other_role = "kick" if role == "snare" else "snare"
    other = []
    for m, v in refs[song]["mics"].items():
        if v["role"] == other_role:
            o = fm.track_offset(v["onsets"], run["events"][m])
            other += [r + o for r in v["onsets"]]
    stems = (fp.LAYERS.get(song, []) if role == "snare" else KICK_LAYERS.get(song, []))
    lay = layer_times(cfg, stems)
    lay_off = {k: layer_offset(ref, v) for k, v in lay.items()}
    lay_each = {k: sorted(t - lay_off[k] + off for t in v) for k, v in lay.items()}   # in the events' time base
    lay_all = sorted(t for v in lay_each.values() for t in v)
    roll = chains(sorted(shifted), ROLL_TOL, ROLL_MIN)
    roll_of = dict(zip(sorted(range(len(shifted)), key=lambda i: shifted[i]), roll))
    out = {"song": song, "mic": mic, "role": role, "n_ref": len(ref), "n_out": len(est), "offset_ms": off * 1000,
           "median_peak_db": med, "witness": wit_stem if wx is not None else None, "layers": sorted(lay),
           "missed": [], "false": []}
    extras = []
    for stem in fp.EXTRA.get(song, []):
        ex, esr = read_stem(cfg, stem)
        if ex is not None and esr == sr:
            extras.append(ex)
    ref_sorted = sorted(shifted)
    for i in missed:
        t = shifted[i]
        rel = peak_db(x, sr, t) - med
        ro, rw = rise_db(x, sr, t), (rise_db(wx, sr, t) if wx is not None else None)
        nb = nearest([r for r in ref_sorted if r != t], t)
        out["missed"].append({"t": t, "rel_db": rel, "rise_own": ro, "rise_wit": rw, "neighbour_s": nb,
                              "in_roll": bool(roll_of[i]), "layer_s": nearest(lay_all, t),
                              "layers": {k: abs(nearest(v, t)) <= LAYER_TOL_S for k, v in lay_each.items()},
                              "class": miss_class(rel, ro, rw, nb, bool(roll_of[i]))})
    for j in false:
        t = est[j]
        rel = peak_db(x, sr, t) - med
        ro, rw = rise_db(x, sr, t), (rise_db(wx, sr, t) if wx is not None else None)
        before = [t - m for m in matched_t if m < t]
        mb = min(before) if before else 9.0
        os_, ls = nearest(other, t), nearest(lay_all, t)
        rx = max((rise_db(ex, sr, t) for ex in extras), default=None)
        out["false"].append({"t": t, "rel_db": rel, "rise_own": ro, "rise_wit": rw, "rise_extra": rx, "other_role_s": os_,
                             "layer_s": ls, "after_matched_s": mb, "class": false_class(rel, ro, rw, os_, ls, mb, rx)})
    out["rel_matched"] = [peak_db(x, sr, t) - med for t in matched_t]
    return out


def stage_causes(song, mic, role="snare"):
    """The `miss_causes` cause of each missed hit of a track (blind, threshold, lobe, average, role, ...) from its kept
    activation: [(time, cause, activation peak)]. The activation file comes from `run`."""
    refs, run = load_song(song, mic)
    ref, ev = refs[song]["mics"][mic]["onsets"], run["events"][mic]
    off = fm.track_offset(ref, ev)
    est, shifted = fm.outputs(ev), [r + off for r in ref]
    pairs, missed, _ = split_hits(shifted, est)
    act = mc.read_activation(OUT / "work" / tid(song, mic) / "model" / "act.bin")
    picks, st, _, _ = mc.effective_picks(act, role)
    classes = mc.ROLE_CLASSES[role]
    own_kept = sorted(f for c in classes for f in st[c]["kept"])
    used = {j for _, j in pairs}
    claimed = [f for f in (mc.claim(own_kept, est[j]) for j in used) if f is not None]
    free = [e for j, e in enumerate(est) if j not in used]
    res = []
    for i in missed:
        cause, det = mc.classify_miss(shifted[i], role, act, st, picks, free, claimed)
        res.append((shifted[i], cause, det.get("peak", 0.0)))
    return res


def snare_costs(log=print):
    """For each of the seven songs' snare tracks: the base (app) counts and the replay of the fix candidates:
    the threshold sweep and the roll fill. Returns {song: {...}}."""
    refs = json.loads((SRC / "refs.json").read_text())
    res = {}
    for song in refs:
        _, run = load_song(song, "s top")
        ref, ev = refs[song]["mics"]["s top"]["onsets"], run["events"]["s top"]
        off = fm.track_offset(ref, ev)
        est, shifted = fm.outputs(ev), [r + off for r in ref]
        act = mc.read_activation(OUT / "work" / tid(song, "s top") / "model" / "act.bin")
        base = picks_at(act, mc.THRESHOLDS[1])
        lead = lead_of(est, base)
        n_match = len(fm.match(shifted, est))
        r = {"n_ref": len(ref), "n_out": len(est), "replay_n": len(base), "matched": n_match,
             "false": len(est) - n_match, "missed": len(ref) - n_match, "lead_ms": lead * 1000, "sweep": {}}
        for thr in THRESHOLD_SWEEP:
            r["sweep"][thr] = gain_false(shifted, base, picks_at(act, thr), lead)
        r["chain"] = gain_false(shifted, base, chain_picks(act), lead)
        res[song] = r
    return res


def do_kit(out, log=print):
    """The roll fill on the DRT kit (`~/.cache/drumspike/drt-testkit`, 9 files: clicks, single hits, flams, ghosts, hat bleed,
    ramps, noise, close pairs): the model's kept activation of each file, the role's picks with and without the roll
    fill, found and false against the kit's ground truth. Needs the kit; returns the rows."""
    tool, model = mc.find_tool(), "none".resolve()
    rows = []
    for it in mc.drt_items():
        wd = out / "kit" / "work" / it["id"] / "model"
        if not (wd / "act.bin").is_file():
            fm.wait_for_room(log)
            mc.run_item(tool, out / "kit", it, "model", model)
        act = mc.read_activation(wd / "act.bin")
        role = it["role"]
        base = picks_at(act, mc.THRESHOLDS[mc.ROLE_CLASSES[role][0]], role)
        var = chain_picks(act, role)
        a = mc.score_events(it, [t + mc.LEAD_S for t in base])
        b = mc.score_events(it, [t + mc.LEAD_S for t in var])
        rows.append({"file": it["file"], "role": role, "base_found": sum(a[0].values()), "base_false": a[1],
                     "chain_found": sum(b[0].values()), "chain_false": b[1], "added": len(var) - len(base)})
    shutil.rmtree(out / "kit" / "run-home", ignore_errors=True)
    shutil.rmtree(out / "kit" / "run-projects", ignore_errors=True)
    return rows


# ---------------------------------------------------------------- the report

TRACK_LIST = [(sg, "s top", "snare") for sg in ["Session A", "Session B", "Session C", "Session H",
                                                 "Session F", "Session G", "Session D"]]
TRACK_LIST += [("Session A", "k in", "kick")]


def md_table(head, body):
    return fm.md_table(head, body)


def med(v):
    return f"{float(np.median(v)):+.1f}" if len(v) else "-"


def do_report(out, log=print):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    a = {f"{sg}|{mic}": analyze_track(sg, mic, role) for sg, mic, role in TRACK_LIST}
    lines = ["### Missed snare hits of Session G", ""]
    liv = a["Session G|s top"]
    causes = stage_causes("Session G", "s top")
    cc = count_by([c for _, c, _ in causes], mc.CAUSES)
    pk = np.array([p for _, _, p in causes])
    lines += [md_table(["cause (`miss_causes`)", "hits"], [[k, v] for k, v in cc.items() if v]), "",
              f"Activation peak of the snare column at the missed hits (the class threshold is {mc.THRESHOLDS[1]}): "
              f"p10 {np.percentile(pk, 10):.2f}, median {np.median(pk):.2f}, p90 {np.percentile(pk, 90):.2f}; "
              f"{int(np.sum(pk >= 0.15))} of {len(pk)} are 0.15 or more.", ""]
    miss = liv["missed"]
    mcls = count_by([m["class"] for m in miss], MISS_CLASSES)
    lines += [md_table(["class", "hits"], [[k, v] for k, v in mcls.items()]), ""]
    secs = {}
    for m in miss:
        secs.setdefault(int(m["t"] // 20) * 20, [0, 0])[0] += 1
    refs = json.loads((SRC / "refs.json").read_text())
    for r in refs["Session G"]["mics"]["s top"]["onsets"]:
        secs.setdefault(int(r // 20) * 20, [0, 0])[1] += 1
    lines += ["Missed hits per 20 s of the song (missed / reference hits): "
              + ", ".join(f"{k}-{k + 20} s: {v[0]} / {v[1]}" for k, v in sorted(secs.items()) if v[0]), "",
              f"Level of the missed hits on `s top`: median {med([m['rel_db'] for m in miss])} dB against the median "
              f"found hit ({liv['median_peak_db']:.1f} dBFS); {sum(1 for m in miss if m['rel_db'] <= GHOST_DB)} of "
              f"{len(miss)} are 15 dB or more under it. Rise on `s top` (control at random times -1.4 dB): median "
              f"{np.median([m['rise_own'] for m in miss]):.1f} dB; on `{liv['witness']}`: median "
              f"{np.median([m['rise_wit'] for m in miss]):.1f} dB; both 10 dB or more: "
              f"{sum(1 for m in miss if m['rise_own'] >= RISE_MIN_DB and m['rise_wit'] >= RISE_MIN_DB)} of {len(miss)}.",
              "In a roll (5 or more reference hits with equal spacing, 12 %): "
              f"{sum(1 for m in miss if m['in_roll'])} of {len(miss)}.",
              "Sample layers with an onset at the missed hit (4 ms): "
              + ", ".join(f"`{k}` {sum(1 for m in miss if m['layers'].get(k))} of {len(miss)}"
                          for k in liv["layers"]) + ".", ""]
    lines += ["### False hits by source (snare of the seven songs, kick of Session A)", ""]
    head = ["song", "role", "ref", "output", "false"] + FALSE_CLASSES
    body = []
    for key, r in a.items():
        c = count_by([f["class"] for f in r["false"]], FALSE_CLASSES)
        body.append([r["song"], r["role"], r["n_ref"], r["n_out"], len(r["false"])] + [c[k] for k in FALSE_CLASSES])
    lines += [md_table(head, body), ""]
    lines += ["Level and rise of the false hits per class (dB against the median found hit; rise = peak at the onset over the "
              "peak 10 to 30 ms before; `witness` = a second mic of the same drum, not used by the app):", ""]
    body = []
    for key, r in a.items():
        for k in FALSE_CLASSES:
            f = [x for x in r["false"] if x["class"] == k]
            if f:
                w = [x["rise_wit"] for x in f if x["rise_wit"] is not None]
                body.append([r["song"], r["role"], k, len(f), med([x["rel_db"] for x in f]),
                             f"{np.median([x['rise_own'] for x in f]):.1f}", f"{np.median(w):.1f}" if w else "no witness",
                             f"{np.median([abs(x['other_role_s']) for x in f]) * 1000:.0f} ms",
                             f"{np.median([abs(x['layer_s']) for x in f]) * 1000:.0f} ms"])
    lines += [md_table(["song", "role", "class", "hits", "level dB", "rise own dB", "rise witness dB",
                        "other role, median distance", "layer, median distance"], body), ""]
    lines += ["### Fix candidates (replay on the kept activations; all seven songs)", "",
              "Threshold sweep: the snare class threshold (shipped 0.24) lowered; cells are more matched / more false "
              "against the shipped replay (which equals the app's output counts).", ""]
    costs = snare_costs()
    head = ["song", "ref", "output", "false", "missed"] + [f"thr {t:.2f}" for t in THRESHOLD_SWEEP] + ["roll fill"]
    body, tot = [], {t: [0, 0] for t in THRESHOLD_SWEEP}
    tc = [0, 0]
    for sg, r in costs.items():
        row = [sg, r["n_ref"], r["n_out"], r["false"], r["missed"]]
        for t in THRESHOLD_SWEEP:
            g, f = r["sweep"][t]
            tot[t][0] += g
            tot[t][1] += f
            row.append(f"+{g} / +{f}")
        tc[0] += r["chain"][0]
        tc[1] += r["chain"][1]
        row.append(f"+{r['chain'][0]} / +{r['chain'][1]}")
        body.append(row)
    body.append(["**all**", "", "", "", ""] + [f"**+{tot[t][0]} / +{tot[t][1]}**" for t in THRESHOLD_SWEEP]
                + [f"**+{tc[0]} / +{tc[1]}**"])
    lines += [md_table(head, body), "",
              f"Roll fill: candidates at {CHAIN_LO} (class threshold) kept when they lie in a run of {CHAIN_MIN} or more "
              f"picks with equal spacing ({int(CHAIN_TOL * 100)} %, 80 to 400 ms); the shipped picks stay.", ""]
    lines += ["Loudness cut (the app's `vel_cut_db` idea): false hits removed / matched hits lost, cut at x dB under the "
              "median found hit (level on the own mic):", ""]
    head = ["song", "role"] + [f"{c:.0f} dB" for c in CUT_SWEEP]
    body = []
    for key, r in a.items():
        rf = [f["rel_db"] for f in r["false"]]
        body.append([r["song"], r["role"]] + [f"-{x} / +{y}" for x, y in (level_cut(r["rel_matched"], rf, c)
                                                                             for c in CUT_SWEEP)])
    lines += [md_table(head, body), ""]
    (out / "report.md").write_text("\n".join(lines))
    (out / "results.json").write_text(json.dumps({"tracks": a, "costs": {k: {**v, "sweep": {str(t): g for t, g in v["sweep"].items()}}
                                                                       for k, v in costs.items()},
                                                   "living_causes": [[t, c, p] for t, c, p in causes]}))
    log("\n".join(lines))
    return a, costs


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["run", "report", "kit", "all"])
    ap.add_argument("--out", default=str(OUT))
    a = ap.parse_args(argv)
    if a.cmd in ("run", "all"):
        do_run(a.out)
    if a.cmd in ("report", "all"):
        do_report(a.out)
    if a.cmd in ("kit", "all"):
        rows = do_kit(Path(a.out))
        (Path(a.out) / "kit.json").write_text(json.dumps(rows))
        print(md_table(["file", "role", "found", "false", "found with fill", "false with fill", "added"],
                       [[r["file"], r["role"], r["base_found"], r["base_false"], r["chain_found"], r["chain_false"],
                         r["added"]] for r in rows]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
