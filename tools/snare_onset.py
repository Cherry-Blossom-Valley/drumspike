"""P4-23 (diagnosis only): why the snare fixes of the P4-21b yardstick are there (358 of 1361: 144 false, 62 missed, 152 moved).

    uv run --with numpy --with soundfile --with mido python tools/snare_onset.py run      # the app on s top (default, version 2) and s bot
    uv run --with numpy --with soundfile --with mido python tools/snare_onset.py songs    # the analysis of every song into tracks.json
    uv run --with numpy --with soundfile --with mido python tools/snare_onset.py report   # every table: report.md (also `files`, `moved`, `hits`, `missed`, `rules`)
    uv run --with numpy --with soundfile --with mido python tools/snare_onset.py all
    uv run --with numpy --with soundfile --with mido python tools/snare_onset.py gate --src RUN --out DST [--db 10]   # P4-31 B on a fix_metric run
    uv run --with numpy --with soundfile --with mido python tools/snare_onset.py gatekit --out DIR [--db 10]   # P4-31 B on the kits

Same method as `tools/kick_onset.py` (P4-22) with the role `snare`; the pure parts (hat, alignment, size and sign, two-sided
count, walk replay) are the ones of `kick_onset`, imported, not copied. Input: `~/.cache/drumspike/fix-metric-p421b/` (`refs.json`,
`run/<song>_default.json`; read only) and the sessions in `~/.cache/drumspike/sessions` (read only). Outputs:
`~/.cache/drumspike/snare-onset/` (`work/` the kept events and activations, `tracks.json`, `report.md`). Nothing under `crates/`,
`static/` or `apps/` changes. The only app run is `run`: `service_tool cli run` on `s top` (the shipped settings and refinement
version 2, the walk's start) and on `s bot` (a second estimator for the hat) of the seven songs and two held-out sessions.
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
import kick_onset as ko  # noqa: E402
import miss_causes as mc  # noqa: E402
import tom_fixes as tf  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SRC = ko.SRC
OUT = Path.home() / ".cache" / "drumspike" / "snare-onset"
SONGS = ko.SONGS
ROLE = "snare"
MICS = ["s top", "s bot"]
APP = "s top"               # the label of the app's own mic in the hat

# the reference files of each song: `yard` is what the yardstick's `refs.json` uses (the union of the onsets of all of them within
# 20 ms, or one file), `files` the files of the same strokes of the session (a trigger track and sample layers), `extra`
# a file that is another sample (the hat leaves it out; the pair table lists it)
FILES = {
    "Session A": {"yard": ["s sampl trig_01"], "files": ["s sampl trig_01", "s sampl savior_01", "s sampl sd3_01"]},
    "Session B": {"yard": ["s trigger_01"], "files": ["s trigger_01", "s samp savior_01", "s samp sd3_01"]},
    "Session C": {"yard": ["s samp 1_02"], "files": ["s samp 1_02", "s samp 2_02"], "extra": ["s samp rev_02"]},
    "Session H": {"yard": ["s samp 1_01", "s samp 2_01", "s samp 3_01"], "files": ["s samp 1_01", "s samp 2_01", "s samp 3_01"]},
    "Session F": {"yard": ["s samp 1_01", "s samp 2_01", "s samp 3_01"], "files": ["s samp 1_01", "s samp 2_01", "s samp 3_01"]},
    "Session G": {"yard": ["s trigger_03"], "files": ["s trigger_03", "s samp savior_03", "s samp sd3_03"]},
    "Session D": {"yard": ["s trigger"], "files": ["s trigger", "s samp saviour", "s samp sd3"]},
}
# sessions the yardstick leaves out: the snare mics and the files, a held-out check of the file jitter and the rules
HELD = {
    "Session E": {"dir": "Session E", "top": "s top-AtA2", "bot": "s btm-AtA2",
                         "files": ["s trigger-AtA2", "s sample savior-AtA2", "s sample sd3-AtA2"]},
    "Session I": {"dir": "Session I", "top": "s top_01", "bot": "s bot_01",
                                   "files": ["s samp 1_01", "s samp 2_01", "s samp 3_01"]},
}
# The fit songs of the rules. P4-22's three songs hold no roll hit at all (Session A, Session C, Session F have none),
# so the snare card fits on Session A, Session B (9 roll hits) and Session F and holds out the other four and the
# two sessions: Session G (47 roll hits) is the held-out test of every roll rule.
FIT = ["Session A", "Session B", "Session F"]
HELD_SONGS = [s for s in SONGS if s not in FIT]
FILL_VARIANTS = {"shipped (0.05, 8 %, 8)": (0.05, 0.08, 8), "tolerance 12 %": (0.05, 0.12, 8), "tolerance 15 %": (0.05, 0.15, 8),
                 "run 6": (0.05, 0.08, 6), "run 5": (0.05, 0.08, 5), "run 6, tolerance 12 %": (0.05, 0.12, 6),
                 "low 0.03": (0.03, 0.08, 8), "low 0.08": (0.08, 0.08, 8), "low 0.03, run 6, tolerance 12 %": (0.03, 0.12, 6)}
GATE_DB = [6.0, 8.0, 10.0, 12.0]   # the own-mic rise test (P4-16b, `firm_fixes.rise_db`) as a gate on the output
JUMP_BINS = [("under 25 dB", -1e9, 25.0), ("25 to 35 dB", 25.0, 35.0), ("35 to 45 dB", 35.0, 45.0), ("45 dB or more", 45.0, 1e9)]
LEVEL_BINS = [("under -12 dB", -1e9, -12.0), ("-12 to -6 dB", -12.0, -6.0), ("-6 to -2 dB", -6.0, -2.0), ("-2 to +2 dB", -2.0, 2.0),
              ("+2 dB or more", 2.0, 1e9)]
MOVE_S = fm.MOVE_S
tid = ko.tid


# ---------------------------------------------------------------- sources (read only)

def snare_mic_stems(song):
    """{"s top": stem, "s bot": stem} of a yardstick song: the snare top mic of the yardstick's table and the witness mic
    of P4-16b (`fix_metric.WITNESS`)."""
    cfg = fm.songs()[song]
    return {"s top": cfg["mics"]["s top"][0], "s bot": fm.WITNESS[(song, ROLE)]}


def session_cfg(song):
    """(config with `dir`, stems of the snare mics, reference file stems, yardstick file stems) of a yardstick or held-out song."""
    if song in HELD:
        h = HELD[song]
        return {"dir": h["dir"]}, {"s top": h["top"], "s bot": h["bot"]}, h["files"], h["files"][:1]
    return fm.songs()[song], snare_mic_stems(song), FILES[song]["files"], FILES[song]["yard"]


def files_of(song):
    """The reference files of the same strokes of a song (what `fix_metric.ref_files` reads for the fair choice)."""
    return list(FILES[song]["files"])


# ---------------------------------------------------------------- run: the app on the snare mics

def run_one(tool, model, out, i, src, version2=False, log=print):
    """One analysis through `service_tool cli run` (role snare, shipped settings; `version2`: refinement rule 2, the start of
    the version 3 walk) into `<out>/work/<i>/model/`. A track with an `events.json` is kept."""
    wd = out / "work" / i / "model"
    if (wd / "events.json").is_file() and (wd / "act.bin").is_file():
        return
    link = out / "audio" / f"{i}.wav"
    if link.is_symlink() or link.exists():
        link.unlink()
    link.symlink_to(src)
    tf.wait_for_lock(log)
    fm.LOCK.write_text(f"snare_onset run {i}\n")
    keep = os.environ.get("DRUMSPIKE_REFINE_RULE")
    if version2:
        os.environ["DRUMSPIKE_REFINE_RULE"] = "2"
    try:
        mc.run_item(tool, out, {"id": i, "audio": str(link), "role": ROLE, "note": mc.ROLE_NOTE[ROLE]}, "model", model)
    finally:
        if version2:
            if keep is None:
                os.environ.pop("DRUMSPIKE_REFINE_RULE", None)
            else:
                os.environ["DRUMSPIKE_REFINE_RULE"] = keep
        fm.LOCK.unlink(missing_ok=True)
    log(f"ran {i}")


def do_run(out, log=print):
    """`s top` (shipped, and refinement 2) and `s bot` of the seven songs and the two held-out sessions. Resumes."""
    out = Path(out)
    tool = mc.find_tool()
    model = "none".resolve()
    if not tool.is_file():
        raise SystemExit(f"need the release service_tool ({tool}) and the model ({model})")
    (out / "audio").mkdir(parents=True, exist_ok=True)
    try:
        for song in SONGS + list(HELD):
            cfg, stems, _, _ = session_cfg(song)
            for mic in MICS:
                src = fm.audio_path(cfg, stems[mic])
                if not src.is_file():
                    log(f"missing {src}")
                    continue
                run_one(tool, model, out, tid(song, mic), src, log=log)
                if mic == APP:
                    run_one(tool, model, out, tid(song, mic) + "_v2", src, version2=True, log=log)
    finally:
        fm.LOCK.unlink(missing_ok=True)
        shutil.rmtree(out / "run-home", ignore_errors=True)
        shutil.rmtree(out / "run-projects", ignore_errors=True)


def read_events(out, name, mic, suffix=""):
    """The kept events of one snare track of `do_run`: [{t, v, rise, roll?}]; [] when the track is not there."""
    p = Path(out) / "work" / (tid(name, mic) + suffix) / "model" / "events.json"
    return json.loads(p.read_text())["events"] if p.is_file() else []


# ---------------------------------------------------------------- pure parts of this card

def roll_flags(events):
    """For each output hit (`fm.outputs` order) whether the roll fill (P4-19) added it."""
    return [bool(e.get("roll")) for e in events if not e.get("bleed") and not e.get("cut")]


def same_events(a, b, tol=1e-6):
    """The share of the events of `a` that `b` has at the same time (within `tol` s, same count required)."""
    if not a or len(a) != len(b):
        return 0.0
    return float(np.mean([abs(float(x["t"]) - float(y["t"])) <= tol for x, y in zip(a, b)]))


def stage_causes(state, act, role=ROLE):
    """{missed index: (cause, activation peak)} from the kept activation (`miss_causes.classify_miss`)."""
    picks, st, _, _ = mc.effective_picks(act, role)
    classes = mc.ROLE_CLASSES[role]
    own_kept = sorted(f for c in classes for f in st[c]["kept"])
    est = state["est"]
    used = {j for _, j in state["pairs"]}
    claimed = [f for f in (mc.claim(own_kept, est[j]) for j in used) if f is not None]
    free = [e for j, e in enumerate(est) if j not in used]
    out = {}
    for i in state["missed"]:
        cause, det = mc.classify_miss(state["shifted"][i], role, act, st, picks, free, claimed)
        out[i] = (cause, float(det.get("peak", 0.0)))
    return out


MISS_CLASSES = ko.MISS_CLASSES
FALSE_CLASSES = ["layer", "witness", "kick_ref", "double", "ring", "no_stroke", "real_stroke"]
FALSE_TEXT = {
    "layer": "a sample layer of the session has an onset within 3 ms: a real stroke the reference lacks",
    "witness": "the own mic and `s bot` both show a stroke and no kick reference hit lies near: a real stroke",
    "kick_ref": "a kick reference hit within 12 ms: kick bleed in the snare mic",
    "double": "12 to 60 ms after a matched hit: a second stroke or a rebound",
    "ring": "60 ms to 1 s after a reference hit: the decay of the earlier hit",
    "no_stroke": "the own mic shows no stroke there (rise under 10 dB): a false pick",
    "real_stroke": "the own mic shows a stroke, no second file confirms it"}


def miss_class(rise_own, nearest_false_s, merged, cause, silent=False, rise_min=ff.RISE_MIN_DB):
    """The class of one missed snare (as `kick_onset.miss_class`; the stage causes of the snare column)."""
    return ko.miss_class(rise_own, nearest_false_s, merged, cause, silent, rise_min)


def false_class(layer, witness, kick_near_s, after_matched_s, rise_own, since_ref_s, **kw):
    """The class of one false snare, as `kick_onset.false_class` with the other role being the kick."""
    c = ko.false_class(layer, witness, kick_near_s, after_matched_s, rise_own, since_ref_s, **kw)
    return "kick_ref" if c == "snare_ref" else c


def size_roll(rows):
    """{"roll": [errors], "normal": [errors]} (s) of the moved rows by the roll flag."""
    out = {"roll": [], "normal": []}
    for r in rows:
        if abs(r["err"]) > MOVE_S + 1e-9:
            out["roll" if r.get("roll") else "normal"].append(r["err"])
    return out


def side_class3(file_errs, yard_files, move_s=MOVE_S):
    """Whose error a hit that moved against the yardstick is, from the app's error against each reference file (`file_errs`
    {file: error s or None}; `yard_files`: the files the yardstick uses). `app`: no file agrees with the app within 1 ms.
    `ref_file`: every file that is not the yardstick's single file agrees (the yardstick's file is the odd one out; needs a
    second file). `mixed`: some agree. `unknown`: no file has the hit. The files of a session come from one trigger, so
    agreement between them is no proof of independence (see the report)."""
    items = [(f, abs(e)) for f, e in file_errs.items() if e is not None]
    if not items:
        return "unknown"
    others = [e for f, e in items if not (len(yard_files) == 1 and f in yard_files)]
    ok = [e <= move_s + 1e-9 for _, e in items]
    if not any(ok):
        return "app"
    if others and all(e <= move_s + 1e-9 for e in others) and len(yard_files) == 1:
        return "ref_file"
    return "mixed"


def bot_class(bot_ref_err, bot_top_err, move_s=MOVE_S):
    """The second estimator of a moved hit: the app on `s bot` (after its own offset to `s top`). `ref` when its time lies
    within 1 ms of the reference (the `s top` time is the odd one), else `top` when it lies within 1 ms of the `s top` time
    (both app mics say the same, against the reference), else `neither`; `none` when `s bot` has no hit there."""
    if bot_ref_err is None or bot_top_err is None:
        return "none"
    if abs(bot_ref_err) <= move_s + 1e-9:
        return "ref"
    return "top" if abs(bot_top_err) <= move_s + 1e-9 else "neither"


def bot_errors(est, shifted, pairs, bot_t, tol=0.020):
    """{event index: (bot time minus reference, bot time minus the `s top` time)} for the matched hits, after the median offset of
    `s bot` to `s top` (a constant latency of the mic is no error). `bot_t`: the output times of the app on `s bot`."""
    b = np.sort(np.asarray(bot_t, float))
    if not len(b) or not pairs:
        return {}
    d = []
    for _, j in pairs:
        k = int(np.argmin(np.abs(b - est[j])))
        if abs(b[k] - est[j]) <= tol:
            d.append(b[k] - est[j])
    off = float(np.median(d)) if d else 0.0
    out = {}
    for i, j in pairs:
        k = int(np.argmin(np.abs(b - (est[j] + off))))
        if abs(b[k] - off - est[j]) <= tol:
            out[j] = (float(b[k] - off - shifted[i]), float(b[k] - off - est[j]))
    return out


def gate_counts(rises, matched, moved, confirmed, false, min_db):
    """The rise gate: drop every output hit whose own-mic rise is under `min_db`. `rises` {event index: dB}; `matched` /
    `moved` / `confirmed` / `false`: sets of event indices (`moved` inside `matched`; `confirmed` inside `false`). Returns
    {false removed, of them confirmed strokes, matched lost, of them moved}: the two sides of the rule."""
    low = {j for j, r in rises.items() if r < min_db}
    fr, ml = low & false, low & matched
    return {"false": len(fr), "confirmed": len(fr & confirmed), "matched": len(ml), "moved": len(ml & moved)}


def bin_of(v, bins):
    """The name of the bin (name, lo, hi) that holds `v` (lo <= v < hi); the last bin when none."""
    for name, lo, hi in bins:
        if lo <= v < hi:
            return name
    return bins[-1][0]


def moved_share(rows, pred_bins, key):
    """{bin: (hits, moved)} of the matched rows by a feature `key(row)` cut into `pred_bins` [(name, lo, hi)]."""
    out = {n: [0, 0] for n, _, _ in pred_bins}
    for r in rows:
        c = out[bin_of(key(r), pred_bins)]
        c[0] += 1
        c[1] += int(abs(r["err"]) > MOVE_S + 1e-9)
    return {k: tuple(v) for k, v in out.items()}


def subset_walks(results, pred, fit, held, sessions, ref="yard"):
    """{rule: {group: (moved before, moved after, wrong, right)}} of the back-track variants (`kick_onset.WALKS`) on the rows
    that `pred(row)` keeps; a song without such rows is left out (no fit on nothing)."""
    per = {}
    for s, res in results.items():
        sub = dict(res, rows=[r for r in res["rows"] if pred(r)])
        if not sub["rows"]:
            continue
        r = ko.rule_errors(sub, ref)
        per[s] = r
    groups = {"fit": [s for s in fit if s in per], "held": [s for s in held if s in per],
              "sessions": [s for s in sessions if s in per], "all": [s for s in per]}
    out = {}
    for rule in [k for k in (per[next(iter(per))] if per else {}) if k != "base"]:
        row = {}
        for g, names in groups.items():
            b = ko.cat([per[s]["base"] for s in names if rule in per[s]])
            a = ko.cat([per[s][rule] for s in names if rule in per[s]])
            row[g] = (ko.moved_count(b), *ko.two_sided(b, a))
        out[rule] = row
    return out


def group_shift(results, pred, fit, held, sessions):
    """A constant shift for the hits that `pred(row)` keeps, fitted on the fit songs (`kick_onset.best_shift`; the yardstick
    errors): (shift ms, {group: (moved before, moved after, right hits made wrong, wrong hits made right)}) for the groups fit
    songs, held-out songs, held-out sessions and the seven songs; the counts are of the chosen hits only."""
    def errs(names):
        return ko.cat([np.asarray([r["err"] for r in results[s]["rows"] if pred(r)], float) for s in names if s in results])
    shift, _ = ko.best_shift(errs(fit))
    out = {}
    for g, names in (("fit", fit), ("held", held), ("sessions", sessions), ("seven", list(fit) + list(held))):
        b = errs(names)
        out[g] = (ko.moved_count(b), *ko.two_sided(b, b + shift / 1000.0))
    return shift, out


def fill_variants(act, shifted, events, variants=None):
    """The roll fill (P4-19) with other settings, replayed on the kept activation: {variant: (more matched, more false)}
    against the shipped fill (`firm_fixes.chain_picks`, pick time plus the track's median lead, matched as in P4-16)."""
    base = ff.chain_picks(act, ROLE)
    lead = ff.lead_of(fm.outputs(events), base)
    out = {}
    for name, (lo, tol, nmin) in (variants or FILL_VARIANTS).items():
        out[name] = ff.gain_false(shifted, base, ff.chain_picks(act, ROLE, lo, tol, nmin), lead)
    return out


# ---------------------------------------------------------------- the analysis (reads audio)

def analyze_one(name, refs_on, events, b_events, mic_events, x, sr, files_on, out, yard=(), log=print):
    """`kick_onset.analyze_track` on the snare mic, with the roll flag on each row and the app labelled `s top` in the hat, the
    rise of every output hit on the own mic, the three-way whose-error class and the second mic (`s bot`) of each moved hit."""
    res = ko.analyze_track(name, refs_on, events, x, sr, files_on, mic_events, b_events, None, out, log)
    rf = roll_flags(events)
    est = res["state"]["est"]
    bot_t = fm.outputs(mic_events.get("s bot", []))
    be = bot_errors(est, res["state"]["shifted"], res["pairs"], bot_t)
    for r in res["rows"]:
        r["roll"] = bool(rf[r["j"]]) if r["j"] < len(rf) else False
        r["side3"] = side_class3(r["file_err"], list(yard))
        if abs(r["err"]) > MOVE_S + 1e-9:
            b = be.get(r["j"])
            r["bot"] = bot_class(b[0] if b else None, b[1] if b else None)
    res["hat"] = {(APP if k == "k in" else k): v for k, v in res["hat"].items()}
    res["n_roll"] = int(sum(rf))
    res["rise"] = [ff.rise_db(x, sr, t) for t in est]
    # P4-31: the flam rule's hits (`"flam": true`) in output order (the rise gate leaves them alone)
    res["flam"] = [bool(e.get("flam")) for e in events if not e.get("bleed") and not e.get("cut")]
    return res


def analyze_song(song, refs, run, out, log=print):
    cfg, stems, files, yard = session_cfg(song)
    x, sr = fm.read_stem(cfg, stems[APP])
    ev = run["events"][APP]
    mic_events = {"s bot": read_events(out, song, "s bot")}
    files_on = ko.file_onsets(cfg, files)
    res = analyze_one(song, refs[song]["mics"][APP]["onsets"], ev, read_events(out, song, APP, "_v2"), mic_events, x, sr, files_on, out, yard, log)
    own = read_events(out, song, APP)
    res["events_equal_yardstick"] = same_events(ev, own)
    res["extra"] = extra_pairs(cfg, song, files_on)
    act = mc.read_activation(Path(out) / "work" / tid(song, APP) / "model" / "act.bin")
    res["fill"] = fill_variants(act, res["state"]["shifted"], own)
    res["miss_false"] = missed_false(song, res, refs, run, x, sr, cfg, stems, out)
    return res


def extra_pairs(cfg, song, files_on):
    """The pair table of the `extra` files (another sample than the strokes' files) against the first file."""
    ex = FILES[song].get("extra", [])
    if not ex:
        return {}
    on = ko.file_onsets(cfg, ex + list(files_on)[:1])
    return {k: v for k, v in ko.file_pairs(on).items()}


def analyze_held(name, out, log=print):
    cfg, stems, files, yard = session_cfg(name)
    x, sr = fm.read_stem(cfg, stems[APP])
    ev = read_events(out, name, APP)
    if x is None or not ev:
        return None
    files_on = ko.file_onsets(cfg, files)
    ref = ko.union_onsets([files_on[f] for f in yard if f in files_on])
    res = analyze_one(name, ref, ev, read_events(out, name, APP, "_v2"), {"s bot": read_events(out, name, "s bot")}, x, sr, files_on, out, yard, log)
    act = mc.read_activation(Path(out) / "work" / tid(name, APP) / "model" / "act.bin")
    res["fill"] = fill_variants(act, res["state"]["shifted"], ev)
    return res


def missed_false(song, res, refs, run, x, sr, cfg, stems, out):
    """The classes of the missed and the false snares of one yardstick song: {"missed": [row], "false": [row]}."""
    import learn_eval as le
    st = res["state"]
    state = {"est": st["est"], "shifted": st["shifted"], "pairs": res["pairs"], "missed": st["missed"], "false": st["false"]}
    act = mc.read_activation(Path(out) / "work" / tid(song, APP) / "model" / "act.bin")
    causes = stage_causes(state, act)
    est, sh = st["est"], st["shifted"]
    matched = sorted(est[j] for _, j in res["pairs"])
    used = {j for _, j in res["pairs"]}
    false_t = [est[j] for j in st["false"]]
    wx, wsr = fm.read_stem(cfg, stems["s bot"])
    if wx is not None and wsr != sr:
        wx = None
    layers = []
    for stem in fm.SNARE_LAYERS.get(song, []):
        lx, lsr = fm.read_stem(cfg, stem)
        if lx is None:
            continue
        on = le.ref_onsets(lx, lsr, gap=0.005)
        layers += [float(t) - fm.layer_offset(refs[song]["mics"][APP]["onsets"], on) + st["off"] for t in on]
    layers = sorted(layers)
    kick = sorted(r + fm.track_offset(v["onsets"], run["events"][m]) for m, v in refs[song]["mics"].items()
                  if v["role"] == "kick" for r in v["onsets"])
    ref_t = np.asarray(sh, float)
    med = float(np.median([ff.peak_db(x, sr, t) for t in matched])) if matched else -20.0
    order = sorted(range(len(sh)), key=lambda i: sh[i])
    in_roll = dict(zip(order, ff.chains([sh[i] for i in order], ff.ROLL_TOL, ff.ROLL_MIN)))
    missed = []
    for i in st["missed"]:
        t = sh[i]
        silent = ff.peak_db(x, sr, t) < tf.SILENT_DB
        nf = tf.nearest_abs(false_t, t)
        merged = any(abs(est[j] - t) <= fm.TOL for j in used)
        cause, peak = causes[i]
        gaps = np.abs(ref_t - t)
        gaps = gaps[gaps > 1e-9]
        missed.append({"t": t, "rise_own": ff.rise_db(x, sr, t), "rel_db": ff.peak_db(x, sr, t) - med, "silent": bool(silent),
                       "nearest_false_ms": nf * 1000, "merged": bool(merged), "cause": cause, "act_peak": peak,
                       "in_roll": bool(in_roll[i]),
                       "ref_gap_ms": float(gaps.min() * 1000) if len(gaps) else 9e3,
                       "class": miss_class(ff.rise_db(x, sr, t), nf, merged, cause, bool(silent))})
    false = []
    for j in st["false"]:
        t = est[j]
        fl = fm.stroke_flags([t], layers, x, sr, wx, other=kick)[0]
        after = tf.after_matched_s(t, matched)
        since = float(t - ref_t[ref_t < t].max()) if np.any(ref_t < t) else 9.0
        false.append({"t": t, "layer": bool(fl[0]), "witness": bool(fl[1]), "rise_own": ff.rise_db(x, sr, t),
                      "rel_db": ff.peak_db(x, sr, t) - med, "kick_near_ms": tf.nearest_abs(kick, t) * 1000,
                      "after_matched_ms": after * 1000,
                      "class": false_class(bool(fl[0]), bool(fl[1]), tf.nearest_abs(kick, t), after, ff.rise_db(x, sr, t), since)})
    return {"missed": missed, "false": false}


def do_songs(out, src=SRC, log=print):
    out = Path(out)
    refs, runs, _before = ko.load_inputs(src)
    allres = {s: analyze_song(s, refs, runs[s], out, log) for s in SONGS}
    held = {}
    for h in HELD:
        r = analyze_held(h, out, log)
        if r:
            held[h] = r
    (out / "tracks.json").write_text(json.dumps({"songs": allres, "held": held}, default=float))
    return allres, held


# ---------------------------------------------------------------- reports

md, f2, stats_row = ko.md, ko.f2, ko.stats_row


def all_tracks(data):
    return {**data["songs"], **data["held"]}


def yard_of(name):
    return FILES[name]["yard"] if name in FILES else HELD[name]["files"][:1]


def section_files(data):
    lines = ["### Question 1: the snare reference files against the app (`s top`)", ""]
    body = []
    for name, res in all_tracks(data).items():
        for f, v in res["files"].items():
            tag = (" (yardstick)" if f in yard_of(name) else "") + (" (fair)" if f == res["fair"]["file"] else "")
            body.append([name, f + tag, v["n"], f2(v["off_ms"]), v["moved"], *stats_row(v["errs"])[1:], f2(res["hat"].get(f, float("nan")))])
    lines += [md(["song", "file", "onsets", "offset ms", "moved", "p10", "median", "p90", "beyond 1 ms", "hat jitter ms"], body), ""]
    body = []
    for name, res in all_tracks(data).items():
        body.append([name, res["hat_n"], f2(res["hat"].get(APP)), f2(res["hat"].get("s bot"))])
    lines += ["Jitter of the app's onset on each snare mic (three-cornered hat, ms; the files of the session and both mics in it):", "",
              md(["song", "hits in common", "s top", "s bot"], body), ""]
    body = []
    for name, res in all_tracks(data).items():
        for k, v in res["file_pairs"].items():
            body.append([name, k, v["n"], f2(v["sd_ms"]), f"{100 * v['beyond']:.0f} %", " ".join(str(c) for c in v["hist"])])
        for k, v in (res.get("extra") or {}).items():
            body.append([name, k + " (another sample, not in the hat)", v["n"], f2(v["sd_ms"]), f"{100 * v['beyond']:.0f} %",
                         " ".join(str(c) for c in v["hist"])])
    lines += ["The reference files against each other (no app; one offset per pair):", "",
              md(["song", "pair", "hits", "sd ms", "more than 1 ms apart", "hits per 0.5 ms bin, -3 to +3 ms"], body), ""]
    body = [[n, r["events_equal_yardstick"] if r.get("events_equal_yardstick") is not None else "-", r["replay_exact"], r["n_roll"]]
            for n, r in data["songs"].items()]
    lines += ["Checks: share of the events of this run (`s top` alone) equal to the yardstick's events; share of the version 3 walk replays equal "
              "to the app's time; hits the roll fill added:", "", md(["song", "events equal", "replay exact", "roll hits"], body), ""]
    return "\n".join(lines)


def section_fair(data):
    lines = ["### The yardstick against the fair snare file (`s top`)", ""]
    body, tot = [], np.zeros(8, int)
    for s, res in data["songs"].items():
        y, f = res["counts"], res["fair_counts"]
        yf, ff_ = y["false"] + y["missed"] + y["moved"], f["false"] + f["missed"] + f["moved"]
        tot += np.asarray([y["false"], y["missed"], y["moved"], yf, f["false"], f["missed"], f["moved"], ff_])
        body.append([s, y["n_ref"], y["false"], y["missed"], y["moved"], f"**{yf}**", f"{res['fair']['file']} ({res['fair']['how']})",
                     f["n"], f["false"], f["missed"], f["moved"], f"**{ff_}**"])
    body.append(["**all**", "", *tot[:3], f"**{tot[3]}**", "", "", *tot[4:7], f"**{tot[7]}**"])
    lines += [md(["song", "ref hits", "false", "missed", "moved", "fixes", "fair file", "ref hits", "false", "missed", "moved", "fixes"], body), ""]
    return "\n".join(lines)


def section_moved(data):
    songs = data["songs"]
    lines = ["### Question 2: the moved snares (yardstick reference, `s top`)", "", "Cells: app later / app earlier than the reference.", ""]
    body, tot = [], {}
    for s, res in songs.items():
        ss = ko.size_sign([r["err"] for r in res["rows"]])
        for k, v in ss.items():
            tot[k] = tot.get(k, 0) + v
        body.append([s, res["counts"]["matched"], *[f"{ss.get((b, 'late'), 0)} / {ss.get((b, 'early'), 0)}" for b in ("1 to 2", "2 to 5", "over 5")],
                     res["counts"]["moved"]])
    body.append(["**all**", sum(r["counts"]["matched"] for r in songs.values()),
                 *[f"{tot.get((b, 'late'), 0)} / {tot.get((b, 'early'), 0)}" for b in ("1 to 2", "2 to 5", "over 5")],
                 sum(r["counts"]["moved"] for r in songs.values())])
    lines += [md(["song", "matched", "1 to 2 ms", "2 to 5 ms", "over 5 ms", "moved"], body), ""]
    body = []
    for s, res in songs.items():
        row = [s, f"{res['counts']['moved']} / {res['fair_counts']['moved']}"]
        for f, v in res["files"].items():
            row.append(f"{f}: {v['moved']}")
        body.append([row[0], row[1], "; ".join(row[2:]), f"{res['mics']['s bot']['moved']} (matched {res['mics']['s bot']['matched']})"
                     if "s bot" in res["mics"] else "no run"])
    lines += ["Moved snares against the yardstick reference / the fair file, against each file of the session (each with its own offset), and the app "
              "on `s bot` against the yardstick reference:", "", md(["song", "yardstick / fair", "per file", "s bot"], body), ""]
    sides = ("app", "mixed", "ref_file", "unknown")
    body = []
    for s, res in songs.items():
        c = {k: 0 for k in sides}
        for r in res["rows"]:
            if abs(r["err"]) > MOVE_S + 1e-9:
                c[r["side3"]] += 1
        body.append([s, *[c[k] for k in sides]])
    body.append(["**all**", *[sum(r[1 + i] for r in body) for i in range(len(sides))]])
    lines += ["Whose error, by the files of the session (`app`: no file agrees with the app within 1 ms; `ref_file`: every other file agrees and "
              "the yardstick's single file is the odd one; `mixed`: some agree; the files share one trigger, so this is no proof):", "",
              md(["song", *sides], body), ""]
    cls = ("ref", "top", "neither", "none")
    body = []
    for s, res in songs.items():
        c = {k: 0 for k in cls}
        for r in res["rows"]:
            if "bot" in r:
                c[r["bot"]] += 1
        body.append([s, *[c[k] for k in cls]])
    body.append(["**all**", *[sum(r[1 + i] for r in body) for i in range(len(cls))]])
    lines += ["The second mic: the app on `s bot` (own offset) at each moved hit. `ref`: it agrees with the reference within 1 ms (the `s top` time "
              "is the odd one); `top`: it agrees with the `s top` time (the same answer from other audio); `neither`; `none`: no hit there:", "",
              md(["song", *cls], body), ""]
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
        lines += [f"Audio class of the moved snares against the {title} (`tom_fixes.moved_class`):", "", md(["song", *classes], body), ""]
    for ref, title in (("yard", "yardstick reference"), ("fair", "fair file")):
        body, allcl = [], []
        for s, res in songs.items():
            e, _, idx = ko.track_errors(res, ref)
            ce = [e[i] for i, k in enumerate(idx) if res["rows"][k]["clean"]]
            allcl += ce
            body.append([s, *stats_row(ce)])
        body.append(["**all**", *stats_row(allcl)])
        lines += [f"Clean onsets (isolated 300 ms, level -6 dB or more, 30 dB over the floor within 2.5 ms), error against the {title}, ms:", "",
                  md(["song", "hits", "p10", "median", "p90", "beyond 1 ms"], body), ""]
    rows = [r for res in songs.values() for r in res["rows"]]
    body = []
    for lab, sel in (("roll fill hits", [r for r in rows if r["roll"]]), ("all other hits", [r for r in rows if not r["roll"]])):
        e = [r["err"] for r in sel]
        cl = {k: sum(1 for r in sel if r.get("moved_class") == k) for k in classes}
        bc = {k: sum(1 for r in sel if r.get("bot") == k) for k in ("ref", "top", "neither")}
        body.append([lab, len(sel), sum(1 for v in e if abs(v) > MOVE_S + 1e-9), *stats_row(e)[1:4], *[cl[k] for k in classes],
                     *[bc[k] for k in ("ref", "top", "neither")]])
    lines += ["The roll fill hits (P4-19) against the other hits, seven songs, yardstick reference (ms; classes of the moved hits; the second mic "
              "`s bot` at the moved hits):", "",
              md(["group", "hits", "moved", "p10", "median", "p90", *classes, "bot ref", "bot top", "bot neither"], body), ""]
    body = []
    for s, res in songs.items():
        sel = [r for r in res["rows"] if r["roll"]]
        if sel:
            body.append([s, len(sel), sum(1 for r in sel if abs(r["err"]) > MOVE_S + 1e-9), res["counts"]["moved"]])
    lines += [md(["song", "roll hits matched", "of them moved", "moved in the song"], body), ""]
    return "\n".join(lines)


def section_hits(data):
    rows = [r for res in data["songs"].values() for r in res["rows"]]
    lines = ["### Question 2: does the error depend on the hit? (`s top`, yardstick reference)", ""]
    for title, bins, key in (("level against the typical hit", LEVEL_BINS, lambda r: r["rel_db"]),
                             ("sharpness of the onset (the audio within 2.5 ms of the app's time over the floor)", JUMP_BINS, lambda r: r["jump_db"]),
                             ("previous hit", [(n, lo, hi) for n, lo, hi in ko.GAP_BINS], lambda r: r["gap_prev_s"])):
        ms = moved_share(rows, bins, key)
        body = [[n, h, m, f"{100 * m / h:.0f} %" if h else "-"] for n, (h, m) in ms.items()]
        lines += [f"Moved share by {title}:", "", md(["group", "hits", "moved", "share"], body), ""]
    strong = [r for r in rows if r["rel_db"] >= ko.CLEAN_DB]
    ms = moved_share(strong, [(n, lo, hi) for n, lo, hi in ko.GAP_BINS], lambda r: r["gap_prev_s"])
    lines += ["Moved share by previous hit, strong hits only (level -6 dB or more; the spacing without the soft hits that sit between backbeats):", "",
              md(["previous hit", "hits", "moved", "share"], [[n, h, m, f"{100 * m / h:.0f} %" if h else "-"] for n, (h, m) in ms.items()]), ""]

    def reason(r):
        out = []
        if r["gap_prev_s"] < ko.CLEAN_GAP_S or r["gap_next_s"] < ko.CLEAN_GAP_S:
            out.append("not isolated")
        if r["rel_db"] < ko.CLEAN_DB:
            out.append("soft")
        if r["jump_db"] < ko.CLEAN_JUMP_DB:
            out.append("not sharp")
        return " + ".join(out) or "clean"
    c, m = {}, {}
    for r in rows:
        k = reason(r)
        c[k] = c.get(k, 0) + 1
        m[k] = m.get(k, 0) + int(abs(r["err"]) > MOVE_S + 1e-9)
    lines += ["Why a hit is not clean, and how many of its group moved:", "",
              md(["group", "hits", "moved"], [[k, c[k], m[k]] for k in sorted(c, key=lambda k: -c[k])]), ""]
    body = []
    for s, res in data["songs"].items():
        e, lv, _ = ko.track_errors(res, "yard")
        shift, mv = ko.best_shift(e)
        cl = ko.two_clusters(e, lv)
        body.append([s, ko.moved_count(e), f"{shift:+.2f} -> {mv}", f"{100 * cl['late_share']:.0f} %", f2(cl["gap_ms"]), f2(cl["corr_level"])])
    lines += ["Per song: moved now, best constant shift (ms) and moved after it, late group (more than 0.4 ms after the median), gap between the "
              "groups, correlation of the error with the level:", "",
              md(["song", "moved", "best shift", "late group", "gap ms", "corr level"], body), ""]
    return "\n".join(lines)


def section_missed(data):
    songs = data["songs"]
    lines = ["### Question 4: missed and false snares (yardstick reference)", ""]
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
    body = []
    for s, res in songs.items():
        ms = res["miss_false"]["missed"]
        for roll in (True, False):
            sel = [r for r in ms if r["in_roll"] == roll]
            if sel:
                body.append([s, "in a reference roll" if roll else "outside a roll", len(sel), *[sum(1 for r in sel if r["class"] == k) for k in ("threshold", "blind", "average", "role", "no_stroke")],
                             f2(float(np.median([r["act_peak"] for r in sel]))), f2(float(np.median([r["rel_db"] for r in sel])))])
    lines += ["The missed hits by position (a reference run of 5 or more hits with equal spacing, 12 %, is a roll):", "",
              md(["song", "position", "missed", "threshold", "blind", "average", "role", "no_stroke", "median activation", "median level dB"], body), ""]
    body = []
    for s, res in songs.items():
        for r in res["miss_false"]["false"]:
            if r["class"] not in ("layer", "witness"):
                body.append([s, f"{r['t']:.2f}", r["class"], f2(r["rise_own"]), f2(r["rel_db"]), f2(r["kick_near_ms"]), f2(r["after_matched_ms"])])
    lines += ["The false snares that no layer and no witness confirms (the fixes of the second number):", "",
              md(["song", "t s", "class", "rise dB", "level dB", "kick ref ms", "after matched ms"], body), ""]
    return "\n".join(lines)


def section_rules(data):
    results = all_tracks(data)
    sessions = list(data["held"])
    lines = []
    for ref, title in (("yard", "yardstick reference"), ("fair", "fair file")):
        tbl, par = ko.candidate_rows(results, FIT, HELD_SONGS, sessions, ref)
        body = [[k, *[f"{v[g][0]} -> {v[g][1]} (+{v[g][2]} -{v[g][3]})" for g in ("fit", "held", "sessions", "seven")]] for k, v in tbl.items()]
        lines += [f"### Question 4: onset rule candidates, moved snares against the {title} (before -> after, right hits made wrong +, wrong hits made right -)", "",
                  md(["rule", "fit songs", "held-out songs", "held-out sessions", "all seven songs"], body), "",
                  f"Fitted: shift {par['shift']:+.2f} ms; level line {par['line'][0] * 1000:+.3f} ms per dB, intercept {par['line'][1] * 1000:+.2f} ms.", ""]
    for name, pred in (("roll fill hits", lambda r: r["roll"]), ("soft hits (level under -6 dB)", lambda r: r["rel_db"] < -6.0),
                       ("hits that are not clean", lambda r: not r["clean"])):
        tbl = subset_walks(results, pred, FIT, HELD_SONGS, sessions)
        keep = ("walk peak 25", "walk peak 29", "walk peak 31", "walk peak 37", "walk floor 3", "walk floor 9", "walk floor 12")
        body = [[k, *[f"{v[g][0]} -> {v[g][1]} (+{v[g][2]} -{v[g][3]})" for g in ("fit", "held", "sessions", "all")]] for k, v in tbl.items() if k in keep]
        lines += [f"Back-track variants on the {name} only (a song without such a hit is left out):", "",
                  md(["rule", "fit songs", "held-out songs", "held-out sessions", "all"], body), ""]
    body = []
    for name, pred in (("roll fill hits", lambda r: r["roll"]), ("soft hits (level under -6 dB)", lambda r: r["rel_db"] < -6.0),
                       ("hits that are not clean", lambda r: not r["clean"])):
        sh, g = group_shift(results, pred, FIT, HELD_SONGS, sessions)
        body.append([name, f"{sh:+.2f} ms", *[f"{g[k][0]} -> {g[k][1]} (+{g[k][2]} -{g[k][3]})" for k in ("fit", "held", "sessions", "seven")]])
    lines += ["A constant shift for one group of hits (fitted on the fit songs; counts of the group's hits only):", "",
              md(["group", "fitted shift", "fit songs", "held-out songs", "held-out sessions", "seven songs"], body), ""]
    body, tot = [], {}
    for s, res in results.items():
        for k, v in res["fill"].items():
            tot.setdefault(k, {})[s] = v
    for k, v in tot.items():
        seven = np.sum([v[s] for s in SONGS], axis=0)
        sess = np.sum([v[s] for s in sessions if s in v], axis=0)
        lv = v["Session G"]
        body.append([k, f"{seven[0]:+d} / {seven[1]:+d}", f"{lv[0]:+d} / {lv[1]:+d}", f"{sess[0]:+d} / {sess[1]:+d}"])
    lines += ["### Question 4: the roll fill with other settings (more found / more false against the shipped fill; replay on the kept activations)", "",
              md(["fill (low activation, tolerance, run)", "seven songs", "Session G", "two held-out sessions"], body), ""]
    body = []
    for db in GATE_DB:
        tot = {"false": 0, "confirmed": 0, "matched": 0, "moved": 0}
        sess = {"false": 0, "matched": 0}
        flam_under = 0
        for s, res in results.items():
            fl = res.get("flam") or [False] * len(res["rise"])
            flam_under += sum(1 for r, f in zip(res["rise"], fl) if f and r < db)
            used = {j for _, j in res["pairs"]}
            moved = {r["j"] for r in res["rows"] if abs(r["err"]) > MOVE_S + 1e-9}
            conf = {j for j, r in zip(res["state"]["false"], res["miss_false"]["false"]) if r["class"] in ("layer", "witness")} if "miss_false" in res else set()
            g = gate_counts({j: r for j, r in enumerate(res["rise"]) if not fl[j]}, used, moved, conf, set(res["state"]["false"]), db)
            tgt = tot if s in SONGS else sess
            for k in tgt:
                tgt[k] += g[k]
        body.append([f"{db:.0f} dB", tot["false"], tot["confirmed"], tot["false"] - tot["confirmed"], tot["matched"], tot["moved"], sess["false"], sess["matched"], flam_under])
    lines += ["### Question 4: an own-mic rise gate on the output (drop a hit whose rise over the peak 10 to 30 ms before is under the limit; P4-31: the flam rule's hits are exempt)", "",
              md(["limit", "false removed (seven songs)", "of them confirmed strokes", "of them fixes", "matched lost", "of them moved", "sessions: false removed", "sessions: matched lost", "flam hits under the limit (exempt)"], body), ""]
    return "\n".join(lines)


def do_gate(src, dst, db, log=print):
    """P4-31 candidate B on a `fix_metric.py run` folder (`src`, the app's default configuration): every snare hit of a snare
    close mic whose own-mic rise (`firm_fixes.rise_db`) is under `db` is dropped from the events, except the hits the flam
    rule made (`"flam": true`); the result is a run folder `dst` that `fix_metric.py score --out dst` scores (kick, toms and
    the `before` runs are copied). Returns {song: {mic: {"hits", "dropped", "flam_kept", "flam_under"}}}: `flam_under` is
    the number of flam hits that would fall under the limit without the exemption."""
    src, dst = Path(src), Path(dst)
    (dst / "run").mkdir(parents=True, exist_ok=True)
    shutil.copy(src / "refs.json", dst / "refs.json")
    info = {}
    for f in sorted((src / "run").glob("*.json")):
        if not f.name.endswith("_default.json"):
            shutil.copy(f, dst / "run" / f.name)
    for song, cfg in fm.songs().items():
        f = src / "run" / f"{fm.track_name(song)}_default.json"
        res = json.loads(f.read_text())
        info[song] = {}
        for mic, (stem, role) in cfg["mics"].items():
            if role != "snare" or mic not in res["events"]:
                continue
            x, sr = fm.read_stem(cfg, stem)
            ev = res["events"][mic]
            keep, dropped, fk, fu = [], 0, 0, 0
            for e in ev:
                low = ff.rise_db(x, sr, e["t"]) < db
                if e.get("flam"):
                    fk += 1
                    fu += int(low)
                    keep.append(e)
                elif low:
                    dropped += 1
                else:
                    keep.append(e)
            res["events"][mic] = keep
            info[song][mic] = {"hits": len(ev), "dropped": dropped, "flam_kept": fk, "flam_under": fu}
            log(f"{song} {mic}: {len(ev)} hits, {dropped} dropped under {db:g} dB, flam {fk} (under the limit {fu})")
        (dst / "run" / f.name).write_text(json.dumps(res))
    (dst / "gate.json").write_text(json.dumps({"db": db, "songs": info}, indent=1))
    return info


def do_gate_kit(out, db, log=print):
    """P4-31 candidate B on the kits (the app, default settings, the flam rule on): the DRT kit, the flam kit (clean, bleed; made
    from acoustic shots, grace 5 to 40 ms) and the P4-11 kits. Per kit and label: reference strokes found before and after the
    gate (`flam_pairs.kit_score`, 5 ms), false hits before and after, and the flam hits (`"flam": true`) under the limit. The
    gate drops a non-flam snare hit under `db` of rise. Returns the rows; `kit_gate.json` in `out`."""
    import flam_pairs as fp
    out = Path(out)
    tool = Path(os.environ.get("DRUMSPIKE_CALIB_TOOL") or ROOT / "target" / "release" / "service_tool")
    model = "none".resolve()
    rows = []
    for name, files in fp.kit_projects(fp.OUT).items():
        snare = [(f, r) for f, r in files if r == "snare"]
        if not snare:
            continue
        cache = out / "kits" / f"{name}.json"
        if cache.is_file():
            events = json.loads(cache.read_text())
        else:
            events = fp.run_project(tool, model, files, out / "scratch" / name, log)
            cache.parent.mkdir(parents=True, exist_ok=True)
            cache.write_text(json.dumps(events))
        for f, _ in snare:
            stem = Path(f).stem
            ref, lab = fp.kit_refs(name, stem, fp.OUT)
            if not ref:
                continue
            x, sr = fp.read_mono(f)
            ev = [e for e in events[stem] if not (e.get("bleed") or e.get("cut"))]
            keep = [e for e in ev if e.get("flam") or ff.rise_db(x, sr, e["t"]) >= db]
            under = [e for e in ev if e.get("flam") and ff.rise_db(x, sr, e["t"]) < db]
            sc = fp.kit_score(ref, lab, sorted(e["t"] for e in ev), sorted(e["t"] for e in keep), tol=0.005)
            per = {k: [sc["before"]["per"][k][0], sc["after"]["per"][k][0], sc["before"]["per"][k][1]] for k in sc["before"]["per"]}
            rows.append({"kit": name, "file": stem, "hits": len(ev), "dropped": len(ev) - len(keep), "flam": sum(1 for e in ev if e.get("flam")),
                         "flam_under": len(under), "found_before": sc["before"]["found"], "found_after": sc["after"]["found"],
                         "false_before": sc["before"]["false"], "false_after": sc["after"]["false"], "per_label": per})
            r = rows[-1]
            log(f"{name} {stem}: hits {r['hits']}, dropped {r['dropped']}, found {r['found_before']} to {r['found_after']} of {len(ref)}, "
                f"false {r['false_before']} to {r['false_after']}, flam hits {r['flam']} (under the limit {r['flam_under']})")
    (out / "kit_gate.json").write_text(json.dumps({"db": db, "rows": rows}, indent=1))
    return rows


def do_report(out, log=print):
    out = Path(out)
    data = json.loads((out / "tracks.json").read_text())
    text = "\n".join([section_files(data), section_fair(data), section_moved(data), section_hits(data), section_missed(data), section_rules(data)])
    (out / "report.md").write_text(text)
    log(text)
    return text


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("cmd", choices=["run", "songs", "report", "all", "files", "moved", "hits", "missed", "rules", "gate", "gatekit"])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--src", default=str(SRC))
    ap.add_argument("--db", type=float, default=10.0, help="gate: the own-mic rise limit (dB)")
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    if a.cmd == "gate":
        do_gate(a.src, out, a.db)
        return 0
    if a.cmd == "gatekit":
        do_gate_kit(out, a.db)
        return 0
    if a.cmd == "run":
        do_run(out)
        return 0
    if a.cmd in ("songs", "all"):
        if a.cmd == "all":
            do_run(out)
        do_songs(out, a.src)
    if a.cmd in ("report", "all", "files", "moved", "hits", "missed", "rules"):
        do_report(out, print)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
