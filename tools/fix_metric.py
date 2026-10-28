"""P4-16: manual fixes per song, the hand work Drum Spike leaves an engineer.

    uv run --with numpy --with soundfile --with mido python tools/fix_metric.py refs    # the references (needs the P4-14 stage 1b refs)
    uv run --with numpy --with soundfile --with mido python tools/fix_metric.py run     # the app's analysis (DSP detector)
    uv run --with numpy --with soundfile --with mido python tools/fix_metric.py score    # the tables
    uv run --with numpy --with soundfile --with mido python tools/fix_metric.py all
    uv run --with numpy --with soundfile --with mido python tools/fix_metric.py score --midi ~/.cache/drumspike/drt-real   # P4-43: DRT
    uv run --with numpy --with soundfile --with mido python tools/fix_metric.py refs-midi --midi DIR   # the references as MIDI

DRT side (P4-43): `score --midi <dir>` reads one MIDI file per song and mic, `<dir>/<song>/<mic>.mid` (the mic names of
`refs.json`; the app's safe names also count), takes every note-on as a hit and scores it with the same matching, `TOL`
and `MOVE_S`. The offset per mic is the DRT result's own (the same rule as the app's). Only songs with a folder are scored.
The DRT tables come first, then DRT against the run and the exit rule; without app runs the DRT tables alone.
`refs-midi` writes the references as such a folder: they score 0 fixes (the check of the reader).

One number per song and role (kick, snare, toms; the toms are the sum of their close mics):

    fixes = false hits (to delete) + missed hits (to add) + moved hits (onset off by more than 1 ms: to move)

A hit is matched to a reference hit one to one within 20 ms after one offset per mic (the median of the signed
differences within 50 ms: the reference track's own latency). A matched hit is "moved" when it lies more than 1 ms
from its reference hit (after the offset). A hit the app flags out of the output (`"bleed": true`, `"cut": true`)
is not in the output: it counts as deleted already (no fix), and a real hit that the app flagged is a missed hit.
Also reported: fixes per minute of song and fixes per 100 reference hits.

Configuration (the app's release `service_tool`, `cli run`, default settings, as `tools/tom_bleed_cd.py app`):
  The app runs its DSP detector (`rust-dsp`).

The offset of a mic is fixed from the run and used for both configurations, so a systematic shift of
the onsets (version 2 sat late on the foot) shows as moved hits and is not hidden by the offset.

References (P4-16c, P4-23): `score --ref fair|union` (default `fair`). `fair`: the kick and the snare reference of a song is the trigger file
when the session has one, else the sample file with the lowest jitter in the three-cornered hat (`tools/kick_onset.py`,
`tools/snare_onset.py`). `union`: the P4-16 reference of `refs.json`. `--kick-ref` and `--snare-ref` set one role alone.

Sessions: the seven songs in ~/.cache/drumspike/sessions (read only) that have usable references; Session E and
Session I are left out (see `SKIPPED`). Outputs: `~/.cache/drumspike/fix-metric/`.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
OUT = Path.home() / ".cache" / "drumspike" / "fix-metric"
LOCK = Path.home() / ".cache" / "drumspike" / "measure.lock"
MAX_LOAD = float(os.environ.get("DRUMSPIKE_MEASURE_MAX_LOAD", "8.0"))
TOL = 0.020            # match window, s
MOVE_S = 0.001         # a matched hit further than this from its reference is a hit to move
OFFSET_WINDOW = 0.050  # the offset's window, s
ROLES = ["kick", "snare", "toms"]
CONFIGS = {"default": {}, "before": {"DRUMSPIKE_REFINE_RULE": "2", "DRUMSPIKE_BLEED_RULE": "0", "DRUMSPIKE_ROLL_RULE": "0",
                                    "DRUMSPIKE_FLAM_RULE": "0", "DRUMSPIKE_KICK_RESCUE": "0", "DRUMSPIKE_SNARE_RESCUE": "0"},
           # both configurations run the DSP detector (`rust-dsp`); the names keep the old run folders resumable
           "dsp": {}}
SKIPPED = {
    "Session E": "its combined tom reference does not fit its mics (164 of 647 sample hits lie within "
                                "20 ms of a tom mic hit, P4-14 stage 1b) and its kick and snare tracks are not printed "
                                "(only `KickSub.09_01`, `SNBot.09_01`, a sub and a bottom mic)",
    "Session I": "each drum mic is in several full-length files (`_01`, `_03`, `_04`); which one the "
                                  "mix plays is in the Pro Tools session only, so no mic and no reference is certain",
}

# ---- P4-16b: confirmed real strokes (a "false" hit that the audio and the session's own files show to be a stroke)
# Rule (stated in the report): a false hit is a confirmed stroke when
#   (a) layer: an onset of a sample layer of the session lies within LAYER_TOL_S of it (the layer is a file that is not the
#       reference; its latency is fixed against the REFERENCE hits, never against the app's output), or
#   (b) witness: the rise test (peak at the onset 10 dB or more over the peak 10 to 30 ms before it) passes on the own mic AND
#       on a second mic of the same drum (`s bot`, `k out`) that neither the app nor the reference uses.
# The app's output only decides where the rules look; the own mic alone never confirms. The control rates (random times,
# matched hits moved by 40 ms) are measured by `control_rates`.
LAYER_TOL_S = 0.003
RISE_MIN_DB = 10.0
RISE_NEAR, RISE_FAR = (-0.002, 0.010), (-0.030, -0.010)
PEAK_BEFORE_S, PEAK_AFTER_S = 0.005, 0.030
SNARE_LAYERS = {"Session A": ["s sampl savior_01", "s sampl sd3_01"], "Session B": ["s samp savior_01", "s samp sd3_01"],
                "Session C": ["s samp 2_02"], "Session G": ["s samp savior_03", "s samp sd3_03"],
                "Session D": ["s samp saviour", "s samp sd3"]}
KICK_LAYERS = {"Session A": ["k sampl savior_01", "k sampl sd3_01"], "Session B": ["k samp savior_01", "k samp sd3_01"],
               "Session C": ["k samp 2_02"], "Session G": ["k samp savior_03", "k samp sd3_03"],
               "Session D": ["k samp saviour", "k samp sd3"]}
WITNESS = {("Session A", "snare"): "s bot_02", ("Session B", "snare"): "s bot_01",
           ("Session C", "snare"): "s bot_02", ("Session H", "snare"): "s bot_01",
           ("Session F", "snare"): "s bot_01", ("Session G", "snare"): "s bot_01",
           ("Session D", "snare"): "s bot",
           ("Session A", "kick"): "k out_02", ("Session B", "kick"): "k out_01",
           ("Session C", "kick"): "k out_02", ("Session H", "kick"): "k out_01",
           ("Session F", "kick"): "k out_01", ("Session G", "kick"): "k out_01",
           ("Session D", "kick"): "k out"}


def _db(v):
    return 20.0 * np.log10(max(float(v), 1e-9))


def peak_db(x, sr, t, before=PEAK_BEFORE_S, after=PEAK_AFTER_S):
    """Peak level (dBFS) of `x` in [t - before, t + after]."""
    i0, i1 = max(0, int((t - before) * sr)), min(len(x), int((t + after) * sr))
    return _db(np.abs(x[i0:i1]).max()) if i1 > i0 else -180.0


def rise_db(x, sr, t):
    """The peak around the onset over the peak 10 to 30 ms before it (dB): a stroke 10 or more, noise about 0."""
    return peak_db(x, sr, t, -RISE_NEAR[0], RISE_NEAR[1]) - peak_db(x, sr, t, -RISE_FAR[0], RISE_FAR[1])


def layer_offset(ref, layer_on, window=OFFSET_WINDOW):
    """The layer's latency against the reference hits (median signed difference to the nearest layer onset within
    `window`; 0 when none). Fixed on the reference, so the app's output has no say."""
    lo = np.sort(np.asarray(layer_on, float))
    d = []
    for r in ref:
        if not len(lo):
            break
        i = int(np.searchsorted(lo, r))
        c = [lo[j] - r for j in (i - 1, i) if 0 <= j < len(lo) and abs(lo[j] - r) <= window]
        if c:
            d.append(min(c, key=abs))
    return float(np.median(d)) if d else 0.0


def near_any(times_sorted, t, tol):
    """True when a time of the sorted array lies within `tol` of `t`."""
    a = times_sorted
    i = int(np.searchsorted(a, t))
    return any(0 <= j < len(a) and abs(a[j] - t) <= tol + 1e-12 for j in (i - 1, i))


BLEED_S = 0.012     # a hit this close to a reference hit of the other role (kick or snare) is that drum's bleed, never confirmed

# ---- P4-16f: strokes the owner heard as real (listening list `docs/migration/listening-flam-2026-10-17.md`, 2026-10-18)
# (c) listened: an output hit within LISTEN_TOL_S of one of these times is a confirmed stroke. The times are the flam grace
# strokes of the #58 run on `s top` (events' time base). Only the 8 "real" verdicts; "not" and "unsure" rows are not here.
LISTEN_TOL_S = 0.002
LISTENED = {
    ("Session A", "snare"): [153.1789, 171.7214, 181.8304],
    ("Session B", "snare"): [80.0414, 141.0036, 171.9561],
    ("Session H", "snare"): [202.4370],
    ("Session G", "snare"): [185.1909],
}


def listened_flags(est, times, tol=LISTEN_TOL_S):
    """For each event time: True when one of the owner's `times` lies within `tol` of it (P4-16f)."""
    lt = np.sort(np.asarray(times, float))
    return [bool(len(lt)) and near_any(lt, t, tol) for t in est]


def stroke_flags(est, layer_times, x, sr, wx, tol=LAYER_TOL_S, min_db=RISE_MIN_DB, other=(), bleed_s=BLEED_S):
    """For each event time: (layer, witness). `layer_times`: the layer onsets in the events' time base (may be empty);
    `x`: the own mic, `wx`: the witness mic (None: no witness, witness is False); `other`: the reference hits of the other
    role (kick for a snare mic and the reverse), sorted, in the events' time base: a hit within `bleed_s` of one is bleed
    of that drum, which the witness mic hears too, so the witness never confirms it."""
    lt = np.sort(np.asarray(layer_times, float))
    ot = np.sort(np.asarray(other, float))
    out = []
    for t in est:
        lay = bool(len(lt)) and near_any(lt, t, tol)
        bleed = bool(len(ot)) and near_any(ot, t, bleed_s)
        wit = (wx is not None and not bleed and rise_db(x, sr, t) >= min_db and rise_db(wx, sr, t) >= min_db)
        out.append((lay, bool(wit)))
    return out


# the reference of each role, in words (the report prints it)
OLD_SOURCES = {
    "Session A": {"kick": "`k sampl trig_01` (printed sample trigger track)", "snare": "`s sampl trig_01`",
                "toms": "one sample track per tom, `t1 sampl_01` to `t4 sampl_01`; t4 excluded as in P4-6b (owner "
                        "spot-check: 3 ok, 9 unsure, 8 wrong)"},
    "Session B": {"kick": "`k trigger_01`", "snare": "`s trigger_01`",
                           "toms": "one combined track `t trigger_01`, split to the tom mics by the mic levels (P4-6b)"},
    "Session C": {"kick": "`k samp 1_02` (L and R)", "snare": "`s samp 1_02` (L and R)",
                       "toms": "one combined track `t samp_03`, split to the tom mics by the mic levels (P4-6b)"},
}
NEW_KICK_SNARE = {  # song -> (kick files, snare files); several files: the union of their onsets
    "Session H": (["k samp 1_01", "k samp 2_01", "k samp 3_01"], ["s samp 1_01", "s samp 2_01", "s samp 3_01"]),
    "Session F": (["k samp 1_01", "k samp 2_01", "k samp 3_01"], ["s samp 1_01", "s samp 2_01", "s samp 3_01"]),
    "Session G": (["k triggerr_03"], ["s trigger_03"]),
    "Session D": (["k trigger"], ["s trigger"]),
}
NEW_TOM_SOURCES = {
    "Session H": "combined `t samp_01`, split by the sample's waveform (stage 1b; t1 and t4 get no group)",
    "Session F": "one sample track per tom, `t1 samp_01` to `t4 samp_01` (independent of the mics)",
    "Session G": "combined `t trigger_01`, split by the sample's waveform (stage 1b)",
    "Session D": "combined `t trigger`, split by the sample's waveform (stage 1b)",
}


# ---------------------------------------------------------------- the metric (pure; the tests cover these)

def match(ref, est, tol=TOL):
    """One to one, nearest pair first, within `tol` seconds: [(ref index, est index)] sorted by ref index."""
    cand = sorted((abs(d - r), i, j) for i, r in enumerate(ref) for j, d in enumerate(est) if abs(d - r) <= tol)
    ur, ud, pairs = set(), set(), []
    for _, i, j in cand:
        if i in ur or j in ud:
            continue
        ur.add(i)
        ud.add(j)
        pairs.append((i, j))
    return sorted(pairs)


def outputs(events):
    """The hit times that stay in the output: the events without `bleed` and without `cut`."""
    return [float(e["t"]) for e in events if not e.get("bleed") and not e.get("cut")]


def track_offset(ref, events, window=OFFSET_WINDOW):
    """The median signed difference (event - reference) of each reference hit to its nearest event within `window`,
    over all events (the flagged ones too); 0 when no reference hit has one."""
    est = np.sort(np.asarray([float(e["t"]) for e in events], float))
    d = []
    for t in ref:
        i = int(np.searchsorted(est, t))
        c = [est[j] - t for j in (i - 1, i) if 0 <= j < len(est) and abs(est[j] - t) <= window]
        if c:
            d.append(min(c, key=abs))
    return float(np.median(d)) if d else 0.0


def score_track(ref, events, offset, tol=TOL, move_s=MOVE_S, flags=None):
    """The fixes of one track: false, missed, moved, with the counts and the signed onset errors (s) of the matched
    hits. `events`: dicts with `t` and optional `bleed`, `cut`; `offset`: the track's reference offset (s).
    `flags` (P4-16b): (layer, witness) per output hit, in the order of `outputs(events)`; a false hit with either flag is a
    confirmed stroke: counted in `confirmed` (`confirmed_layer`, `confirmed_witness`: how many of them each rule has) and
    left out of `fixes_ex` (the fixes without them). A flag may have a third item, listened (P4-16f, `confirmed_listened`)."""
    est = outputs(events)
    shifted = [float(r) + offset for r in ref]
    pairs = match(shifted, est, tol)
    errs = [est[j] - shifted[i] for i, j in pairs]
    moved = sum(1 for e in errs if abs(e) > move_s + 1e-9)
    false, missed = len(est) - len(pairs), len(ref) - len(pairs)
    used = {j for _, j in pairs}
    fl = [tuple(flags[j]) + (False,) * (3 - len(flags[j])) for j in range(len(est)) if j not in used] \
        if flags is not None else []
    conf = sum(1 for f in fl if any(f))
    return {"n_ref": len(ref), "n_out": len(est), "flagged": len(events) - len(est), "matched": len(pairs),
            "false": false, "missed": missed, "moved": moved, "fixes": false + missed + moved, "errs": errs,
            "confirmed": conf, "confirmed_layer": sum(1 for f in fl if f[0]),
            "confirmed_witness": sum(1 for f in fl if f[1]), "confirmed_listened": sum(1 for f in fl if f[2]),
            "fixes_ex": false + missed + moved - conf}


def add_up(rows, minutes):
    """The sum of track scores (`score_track` results) as one role of one song, with the two rates."""
    s = {k: sum(r[k] for r in rows) for k in ("n_ref", "n_out", "flagged", "matched", "false", "missed", "moved", "fixes")}
    for k in ("confirmed", "confirmed_layer", "confirmed_witness", "confirmed_listened"):
        s[k] = sum(r.get(k, 0) for r in rows)
    s["fixes_ex"] = sum(r.get("fixes_ex", r["fixes"]) for r in rows)
    s["errs"] = [e for r in rows for e in r["errs"]]
    s["per_min"] = s["fixes"] / minutes if minutes else float("nan")
    s["per_min_ex"] = s["fixes_ex"] / minutes if minutes else float("nan")
    s["per_100"] = 100.0 * s["fixes"] / s["n_ref"] if s["n_ref"] else float("nan")
    return s


def err_stats(errs, move_s=MOVE_S):
    """Median, p10 and p90 of the onset errors in ms, and the share beyond `move_s`."""
    if not len(errs):
        return {"n": 0, "median": float("nan"), "p10": float("nan"), "p90": float("nan"), "beyond": float("nan")}
    e = np.asarray(errs, float) * 1000.0
    return {"n": len(e), "median": float(np.median(e)), "p10": float(np.percentile(e, 10)),
            "p90": float(np.percentile(e, 90)), "beyond": float(np.mean(np.abs(e) > move_s * 1000.0 + 1e-6))}


def score_run(refs, events, offsets, minutes, songs=None, flags=None):
    """{song: {role: add_up result, "mics": {mic: score_track result}}} for one run.
    `refs`: {song: {mic: {"role": r, "onsets": [...]}}}; `events`: {song: {mic: [event]}}; `offsets`: {song: {mic: s}};
    `minutes`: {song: float}; `flags`: {song: {mic: [(layer, witness)]}} (P4-16b; none: no stroke is confirmed).
    A tom mic's role is "toms"."""
    res = {}
    for song in songs or refs:
        mics = {m: score_track(v["onsets"], events[song][m], offsets[song][m],
                               flags=(flags or {}).get(song, {}).get(m)) for m, v in refs[song].items()}
        roles = {}
        for role in ROLES:
            rows = [mics[m] for m, v in refs[song].items() if v["role"] == role]
            roles[role] = add_up(rows, minutes[song])
        roles["mics"] = mics
        res[song] = roles
    return res


# ---------------------------------------------------------------- P4-16c, P4-23: the fair kick and snare reference

HAT_SRC = Path.home() / ".cache" / "drumspike" / "kick-onset" / "tracks.json"          # P4-22: the hat jitter of each kick file per song
HAT_SRC_SNARE = Path.home() / ".cache" / "drumspike" / "snare-onset" / "tracks.json"   # P4-23: the same for the snare files
FAIR_ROLES = ("kick", "snare")
ROLE_MIC = {"kick": "k in", "snare": "s top"}      # the mic whose reference is replaced (the name starts with it)


def pick_fair(files, hat):
    """The fair reference file of a song, with no help from the app's errors: a trigger file (the raw pulse of the drum
    trigger: its name has `trig`) when the session has one, else the file with the lowest jitter in the three-cornered hat
    of the mic estimators (`hat` {name: jitter ms}; nan = unknown). Ties go to the first file. Returns (name, how)."""
    trig = [f for f in files if "trig" in f.lower()]
    if trig:
        return trig[0], "trigger"
    ok = [(hat.get(f, float("nan")), i, f) for i, f in enumerate(files) if not np.isnan(hat.get(f, float("nan")))]
    if not ok:
        return files[0], "first"
    return min(ok)[2], "hat"


def kick_files(song):
    """The reference files of the same kick strokes of a song (kick_onset.FILES)."""
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import kick_onset as ko
    return list(ko.FILES[song]["files"])


def snare_files(song):
    """The reference files of the same snare strokes of a song (snare_onset.FILES)."""
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import snare_onset as so
    return list(so.FILES[song]["files"])


def ref_files(song, role):
    """The reference files of the same strokes of `song` for a fair role (kick or snare)."""
    return kick_files(song) if role == "kick" else snare_files(song)


def fair_ref(song, role, hat_all):
    """(file, how) of the fair reference of `role` in `song`; `hat_all` is {song: {file: jitter}}."""
    return pick_fair(ref_files(song, role), hat_all.get(song, {}))


def fair_kick(song, hat_all):
    """(file, how) of the fair kick reference of `song`."""
    return fair_ref(song, "kick", hat_all)


def read_hat(path):
    """{song: {file: jitter}} from a `tracks.json` of `kick_onset.py` or `snare_onset.py`."""
    return {k: v["hat"] for k, v in json.loads(Path(path).read_text())["songs"].items()}


def apply_ref(refs_all, modes, hats, log=print):
    """`refs_all` (refs.json) with the reference of each role set by `modes` ({role: "union" | "fair"}): "union" keeps it as
    it is (the P4-16 yardstick), "fair" takes the onsets of the fair file (`pick_fair`; `hats` {role: {song: {file: jitter}}}).
    Other roles stay. Marks the mic with `ref_file`."""
    table = None
    out = json.loads(json.dumps(refs_all))
    for role in FAIR_ROLES:
        if modes.get(role, "union") != "fair":
            continue
        table = table or songs()
        for song, v in out.items():
            for m, mv in v["mics"].items():
                if mv["role"] != role or not m.startswith(ROLE_MIC[role]):
                    continue
                f, how = fair_ref(song, role, hats.get(role, {}))
                on, _ = read_reference(table[song], [f])
                mv["onsets"] = [float(t) for t in on]
                mv["ref_file"] = f
                mv["source"] = f"fair {role} reference: `{f}` ({how}), {len(on)} hits"
                log(f"{song} {m}: fair {role} reference {f} ({how}), {len(on)} hits")
    return out


def ref_modes(ref, kick_ref=None, snare_ref=None):
    """{role: "union" | "fair"} of the two fair roles: `ref` for both, a role's own option over it."""
    return {"kick": kick_ref or ref, "snare": snare_ref or ref}


def apply_kick_ref(refs_all, mode, hat_all, log=print):
    """P4-16c: `apply_ref` for the kick only."""
    if mode == "union":
        return refs_all
    return apply_ref(refs_all, {"kick": mode}, {"kick": hat_all}, log=log)


# ---------------------------------------------------------------- sessions and references

def songs():
    """{song: {"dir", "mics": {mic: (stem, role)}}} of the seven songs: kick, snare and tom close mics. A tom mic's
    role is "toms"; the mics `excluded` are left out (Session A t4)."""
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import rescue_real as rr
    import tom_bleed_new as tn
    out = {}
    for name, cfg in rr.SESSIONS.items():
        out[name] = {"dir": cfg["dir"], "excluded": set(cfg["excluded"]),
                     "mics": {m: (v[0], "toms" if v[1] == "tom" else v[1]) for m, v in cfg["mics"].items()}}
    for name, cfg in tn.SESSIONS.items():
        if name in SKIPPED:
            continue
        out[name] = {"dir": cfg["dir"], "excluded": set(),
                     "mics": {m: (v[0], "toms" if v[1] == "tom" else v[1]) for m, v in cfg["mics"].items()
                              if v[1] in ("kick", "snare", "tom")}}
    return out


def audio_path(cfg, stem):
    return Path.home() / ".cache" / "drumspike" / "samples" / cfg["dir"] / "Audio Files" / f"{stem}.wav"


def read_reference(cfg, stems):
    """The onsets (s) of the union of the sample or trigger files `stems`; onsets within 20 ms of an earlier one of
    another file are the same hit. Returns (onsets, per-file counts). Files with L and R are `<stem>.L`, `<stem>.R`."""
    import learn_eval as le
    per, allon = {}, []
    for stem in stems:
        files = [audio_path(cfg, stem)]
        if not files[0].is_file():
            files = [audio_path(cfg, stem + ".L"), audio_path(cfg, stem + ".R")]
        x, sr = le.read_mono(files)
        on = le.ref_onsets(x, sr).tolist()
        per[stem] = len(on)
        allon += on
    merged = []
    for t in sorted(allon):
        if not merged or t - merged[-1] >= TOL:
            merged.append(t)
    return merged, per


def do_refs(out, log=print):
    """Write `refs.json`: per song the duration, and per mic its role, onsets and source."""
    import soundfile as sf
    import rescue_real as rr
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    table = songs()
    old = json.loads(rr.REFS.read_text())
    nw_path = Path.home() / ".cache" / "drumspike" / "tom-bleed" / "new" / "refs.json"
    if not nw_path.is_file():
        raise SystemExit(f"{nw_path} is missing: run `uv run ... python tools/tom_bleed_new.py refs` first")
    nw = json.loads(nw_path.read_text())
    refs = {}
    for song, cfg in table.items():
        mics = {}
        for m, (stem, role) in cfg["mics"].items():
            if m in cfg["excluded"]:
                log(f"{song} {m}: excluded")
                continue
            if song in old and m in old[song]:
                on, src = old[song][m], f"P4-6b `refs.json`: {OLD_SOURCES[song][role]}"
            elif role == "toms":
                on, src = nw[song][m], f"P4-14 stage 1b: {NEW_TOM_SOURCES[song]}"
            else:
                files = NEW_KICK_SNARE[song][0 if role == "kick" else 1]
                on, per = read_reference(cfg, files)
                src = "onsets of " + " + ".join(f"`{f}` ({per[f]})" for f in files) + " (union within 20 ms)"
            mics[m] = {"role": role, "stem": stem, "onsets": [float(t) for t in on], "source": src}
            log(f"{song} {m}: {len(on)} reference hits")
        dur = max(sf.info(str(audio_path(cfg, v["stem"]))).duration for v in mics.values())
        refs[song] = {"duration_s": float(dur), "mics": mics}
    (out / "refs.json").write_text(json.dumps(refs))
    return refs


# ---------------------------------------------------------------- the app's analysis

def track_name(stem):
    """The app's track name of a file (`tracks.rs` `safe_name`)."""
    import re
    return re.sub(r"[^A-Za-z0-9-]+", "_", stem).strip("_")


def wait_for_room(log=print):
    """Wait while another measuring agent holds the lock or the 1-minute load is 8 or more."""
    while LOCK.exists() or os.getloadavg()[0] >= MAX_LOAD:
        log(f"waiting: lock {LOCK.exists()}, load {os.getloadavg()[0]:.1f}")
        time.sleep(30)


def run_song(tool, out, song, cfg, config, env_extra, scratch):
    """The app's analysis of one song (project_new, tracks_add referenced, analyze_start, events_get per mic) with the
    environment of `config`. Written to `<out>/run/<song>_<config>.json`; an existing file is kept (resumes)."""
    dst = out / "run" / f"{track_name(song)}_{config}.json"
    if dst.is_file():
        return json.loads(dst.read_text())
    mics = [(m, v[0], v[1]) for m, v in cfg["mics"].items()]
    roles = {"kick": "kick", "snare": "snare", "toms": "tom"}
    work = scratch / f"{track_name(song)}_{config}"
    if work.exists():
        shutil.rmtree(work)
    (work / "p").mkdir(parents=True)
    note = {"kick": 36, "snare": 38, "tom": 47}
    steps = [{"command": "project_new", "args": {"name": "fixes", "dir": str(work / "p")}},
             {"command": "tracks_add", "args": {"sid": "$sid", "paths": [str(audio_path(cfg, s)) for _, s, _ in mics],
                                                "roles": [roles[r] for _, _, r in mics],
                                                "notes": [note[roles[r]] for _, _, r in mics]}, "wait": True},
             {"command": "analyze_start", "args": {"sid": "$sid"}, "wait": True},
             {"command": "analyze_status", "args": {"sid": "$sid"}},
             {"command": "tracks_get", "args": {"sid": "$sid"}}]
    steps += [{"command": "events_get", "args": {"sid": "$sid", "name": track_name(s)}} for _, s, _ in mics]
    (work / "steps.json").write_text(json.dumps(steps))
    env = {k: v for k, v in os.environ.items() if k not in CONFIGS["before"]}
    env.update({"DRUMSPIKE_HOME": str(work / "h"), "DRUMSPIKE_PROJECTS": str(work / "p")})
    env.update(env_extra)
    r = subprocess.run([str(tool), "cli", "run", str(work / "steps.json")], capture_output=True, text=True, env=env,
                       stdin=subprocess.DEVNULL)
    rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
    bad = [x for x in rows if not x.get("ok", False)]
    if r.returncode != 0 or bad or len(rows) != len(steps):
        raise RuntimeError(f"{song} {config}: exit {r.returncode} {bad[:1]} {r.stderr[-400:]}")
    tracks = rows[4]["result"]["tracks"]
    res = {"job": rows[3]["result"], "env": env_extra,
           "detector": {m: tracks[track_name(s)].get("detector") for m, s, _ in mics},
           "events": {m: rows[5 + i]["result"]["events"] for i, (m, _, _) in enumerate(mics)}}
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_text(json.dumps(res))
    shutil.rmtree(work, ignore_errors=True)
    return res


def do_run(out, tool=None, log=print, configs=None):
    out = Path(out)
    tool = Path(tool or os.environ.get("DRUMSPIKE_CALIB_TOOL") or ROOT / "target" / "release" / "service_tool")
    if not tool.is_file():
        raise SystemExit(f"need the release service_tool ({tool})")
    scratch = out / "scratch"
    LOCK.parent.mkdir(parents=True, exist_ok=True)
    try:
        for song, cfg in songs().items():
            for config, extra in CONFIGS.items():
                if configs and config not in configs:
                    continue
                if (out / "run" / f"{track_name(song)}_{config}.json").is_file():
                    continue
                wait_for_room(log)
                LOCK.write_text(f"fix_metric run {song} {config}\n")
                try:
                    run_song(tool, out, song, cfg, config, extra, scratch)
                finally:
                    LOCK.unlink(missing_ok=True)
                log(f"ran {song} {config}")
    finally:
        LOCK.unlink(missing_ok=True)
        shutil.rmtree(scratch, ignore_errors=True)


# ---------------------------------------------------------------- the tables

def md_table(head, body):
    lines = ["| " + " | ".join(head) + " |", "|" + "|".join("---" for _ in head) + "|"]
    lines += ["| " + " | ".join(str(c) for c in row) + " |" for row in body]
    return "\n".join(lines)


def load_runs(out, names, configs=None):
    """{config: {song: events by mic}} plus the job and detector records."""
    runs = {}
    for config in (configs or CONFIGS):
        runs[config] = {}
        for song in names:
            p = Path(out) / "run" / f"{track_name(song)}_{config}.json"
            if not p.is_file():
                raise SystemExit(f"{p} is missing: run `run` first")
            runs[config][song] = json.loads(p.read_text())
    return runs


def f1(v):
    return f"{v:.1f}"


def read_stem(cfg, stem):
    """(mono signal, rate) of a session file (`.L`/`.R` files as one track); (None, None) when it is not there."""
    import learn_eval as le
    files = [audio_path(cfg, stem)]
    if not files[0].is_file():
        files = [audio_path(cfg, stem + ".L"), audio_path(cfg, stem + ".R")]
    return le.read_mono(files) if files[0].is_file() else (None, None)


def stroke_inputs(song, mic, role, cfg, ref, off, exclude=None):
    """The audio side of the confirmation of one kick or snare mic: (own signal, rate, witness signal or None, layer onsets
    in the events' time base, layer file names). The layer latency is fixed on the reference hits `ref`."""
    import learn_eval as le
    x, sr = read_stem(cfg, cfg["mics"][mic][0])
    wstem = WITNESS.get((song, role))
    wx, wsr = read_stem(cfg, wstem) if wstem else (None, None)
    if wx is not None and wsr != sr:
        wx = None
    stems = (SNARE_LAYERS if role == "snare" else KICK_LAYERS).get(song, [])
    lay, used = [], []
    for st in stems:
        if st == exclude:       # the reference file is never its own confirmation
            continue
        lx, lsr = read_stem(cfg, st)
        if lx is None:
            continue
        on = le.ref_onsets(lx, lsr, gap=0.005)
        lay += [float(t) - layer_offset(ref, on) + off for t in on]
        used.append(st)
    return x, sr, wx, sorted(lay), used, (wstem if wx is not None else None)


def control_rates(est, ref_shifted, x, sr, wx, layer_times, seed=1, n=400, other=()):
    """How often the two rules confirm where no stroke is meant: (random times at least 200 ms from every reference hit
    and event; the matched events' times moved by 40 ms (kept when no reference or event lies within 20 ms of them)),
    and for contrast the matched events themselves. Each is (layer rate, witness rate) over the times tested."""
    rng = np.random.default_rng(seed)
    allt = np.sort(np.concatenate([np.asarray(ref_shifted, float), np.asarray(est, float)]))
    dur = len(x) / sr
    m = min(5.0, dur / 4)
    rnd = [t for t in rng.uniform(m, dur - m, n) if abs(allt - t).min() > 0.2]
    pairs = match(ref_shifted, est)
    matched = [est[j] for _, j in pairs]
    moved = [t + 0.040 for t in matched if abs(allt - (t + 0.040)).min() > 0.020]

    def rate(ts):
        f = stroke_flags(ts, layer_times, x, sr, wx, other=other)
        return (float(np.mean([a for a, _ in f])) if f else float("nan"),
                float(np.mean([b for _, b in f])) if f else float("nan"), len(f))
    return {"random": rate(rnd), "moved40": rate(moved), "matched": rate(matched)}


def compute_flags(names, refs, ev, off, log=print):
    """{song: {mic: [(layer, witness)]}} for the kick and snare mics of one run, and the controls of that run:
    {song: {mic: control_rates}}. Toms get no flags (no confirming file is known for them)."""
    table = songs()
    flags, ctl, info = {}, {}, {}
    for song in names:
        for mic, v in refs[song].items():
            if v["role"] not in ("kick", "snare"):
                continue
            x, sr, wx, lay, used, wit = stroke_inputs(song, mic, v["role"], table[song], v["onsets"], off[song][mic],
                                                       exclude=v.get("ref_file"))
            est = outputs(ev[song][mic])
            other_role = "kick" if v["role"] == "snare" else "snare"
            other = [r + off[song][m2] for m2, v2 in refs[song].items() if v2["role"] == other_role for r in v2["onsets"]]
            lis = listened_flags(est, LISTENED.get((song, v["role"]), []) if mic.startswith(ROLE_MIC["snare"]) else [])
            flags.setdefault(song, {})[mic] = [f + (c,) for f, c in zip(stroke_flags(est, lay, x, sr, wx, other=other), lis)]
            sh = [r + off[song][mic] for r in v["onsets"]]
            ctl.setdefault(song, {})[mic] = control_rates(est, sh, x, sr, wx, lay, other=other)
            info.setdefault(song, {})[mic] = {"layers": used, "witness": wit}
            log(f"flags {song} {mic}: layers {used} witness {wit}")
    return flags, ctl, info


def do_score(out, log=print, run=None, ref="union", hat=None, hat_snare=None, kick_ref=None, snare_ref=None, midi=None,
             configs=None):
    """Score the runs in `run` (default `out`; its `refs.json` and `run/`) into `out`. `ref` "union" is the P4-16 yardstick,
    "fair" the P4-16c / P4-23 reference of kick and snare (trigger file, else the file with the lowest hat jitter; `hat` and
    `hat_snare`: the `tracks.json` of `kick_onset.py` and `snare_onset.py`); `kick_ref` / `snare_ref` set one role alone.
    `midi` (P4-43): a folder `<song>/<mic>.mid` of DRT results; its tables come first (`drt-report.md`), with DRT against
    the `default` run when the runs exist, else alone."""
    out = Path(out)
    src = Path(run) if run else out
    out.mkdir(parents=True, exist_ok=True)
    refs_all = json.loads((src / "refs.json").read_text())
    modes = ref_modes(ref, kick_ref, snare_ref)
    hats = {}
    if modes["kick"] == "fair":
        hats["kick"] = read_hat(hat or HAT_SRC)
    if modes["snare"] == "fair":
        hats["snare"] = read_hat(hat_snare or HAT_SRC_SNARE)
    refs_all = apply_ref(refs_all, modes, hats, log=log)
    (out / "refs.json").write_text(json.dumps(refs_all))
    names = list(refs_all)
    refs = {s: refs_all[s]["mics"] for s in names}
    minutes = {s: refs_all[s]["duration_s"] / 60.0 for s in names}
    drt = None
    if midi:
        drt, drt_off = score_midi(refs, read_midi_run(midi, refs), minutes)
        (out / "drt-scores.json").write_text(json.dumps(
            {s: {x: {k: v for k, v in drt[s][x].items() if k != "errs"} for x in ROLES} for s in drt}))
        if not all((src / "run" / f"{track_name(s)}_{c}.json").is_file() for s in names for c in CONFIGS):
            lines = drt_tables(drt, drt_off, refs, minutes)
            (out / "drt-report.md").write_text("\n".join(lines))
            log("no app run in " + str(src / "run") + ": the DRT tables alone\n")
            log("\n".join(lines))
            return {"drt": drt}
    runs = load_runs(src, names, configs)
    ev = {c: {s: {m: runs[c][s]["events"][m] for m in refs[s]} for s in names} for c in runs}
    off = {s: {m: track_offset(refs[s][m]["onsets"], ev["default"][s][m]) for m in refs[s]} for s in names}
    own = {c: {s: {m: track_offset(refs[s][m]["onsets"], ev[c][s][m]) for m in refs[s]} for s in names} for c in runs}
    flags, ctl, info = {}, None, None
    for c in runs:
        flags[c], c_ctl, c_info = compute_flags(names, refs, ev[c], off, log=lambda *_: None)
        if c == "default":
            ctl, info = c_ctl, c_info
    res = {c: score_run(refs, ev[c], off, minutes, flags=flags[c]) for c in runs}
    own_res = {c: score_run(refs, ev[c], own[c], minutes) for c in runs}

    def total(r, role=None):
        sel = [r[s][x] for s in names for x in (ROLES if role is None else [role])]
        return add_up(sel, sum(minutes.values()) * (1 if role else 3))

    lines = ["### Fixes per song and role: the app as it is (refinement version 3, tom bleed rule on)", ""]
    head = ["song", "min", "role", "ref hits", "output", "flagged", "false", "missed", "moved", "fixes", "per min",
            "per 100 ref"]
    body = []
    for s in names:
        for role in ROLES:
            r = res["default"][s][role]
            body.append([s, f1(minutes[s]), role, r["n_ref"], r["n_out"], r["flagged"], r["false"], r["missed"],
                         r["moved"], f"**{r['fixes']}**", f1(r["per_min"]), f1(r["per_100"])])
    for role in ROLES:
        r = total(res["default"], role)
        body.append(["**all**", f1(sum(minutes.values())), role, r["n_ref"], r["n_out"], r["flagged"], r["false"],
                     r["missed"], r["moved"], f"**{r['fixes']}**", f1(r["fixes"] / sum(minutes.values())),
                     f1(r["per_100"])])
    lines += [md_table(head, body), ""]
    mins = sum(minutes.values())

    if "before" in runs:
        lines += ["### Today against before (fixes; `before` = refinement version 2, no tom bleed rule)", ""]
        head = ["song", "role", "ref hits", "before: false / missed / moved = fixes", "today: false / missed / moved = fixes",
                "saved", "per min before", "per min today"]
        body = []

        def cell(r):
            return f"{r['false']} / {r['missed']} / {r['moved']} = **{r['fixes']}**"

        for s in names:
            for role in ROLES:
                a, b = res["before"][s][role], res["default"][s][role]
                body.append([s, role, b["n_ref"], cell(a), cell(b), a["fixes"] - b["fixes"], f1(a["per_min"]),
                             f1(b["per_min"])])
        mins = sum(minutes.values())
        for role in ROLES:
            a, b = total(res["before"], role), total(res["default"], role)
            body.append(["**all**", role, b["n_ref"], cell(a), cell(b), a["fixes"] - b["fixes"], f1(a["fixes"] / mins),
                         f1(b["fixes"] / mins)])
        a, b = total(res["before"]), total(res["default"])
        body.append(["**all**", "**all roles**", b["n_ref"], cell(a), cell(b), a["fixes"] - b["fixes"], f1(a["fixes"] / mins),
                     f1(b["fixes"] / mins)])
        lines += [md_table(head, body), ""]
        lines += confirmed_tables(res, names, minutes, ctl, info)
    if "before" in runs:
        song_rows = []
        for s in names:
            a = sum(res["before"][s][x]["fixes"] for x in ROLES)
            ao = sum(own_res["before"][s][x]["fixes"] for x in ROLES)
            b = sum(res["default"][s][x]["fixes"] for x in ROLES)
            n = sum(res["default"][s][x]["n_ref"] for x in ROLES)
            bl = runs["default"][s]["job"].get("bleed") or {}
            song_rows.append([s, f1(minutes[s]), n, a, ao, b, f"{a - b} to {ao - b}", f1(a / minutes[s]),
                              f1(b / minutes[s]), f1(100.0 * b / n), "on" if bl.get("on") else "off"])
        tot = [sum(r[i] for r in song_rows) for i in (2, 3, 4, 5)]
        song_rows.append(["**all**", f1(mins), tot[0], tot[1], tot[2], tot[3], f"{tot[1] - tot[3]} to {tot[2] - tot[3]}",
                          f1(tot[1] / mins), f1(tot[3] / mins), f1(100.0 * tot[3] / tot[0]), ""])
        lines += ["### Fixes per song (kick + snare + toms)", "",
                  md_table(["song", "min", "ref hits", "before", "before (own offset)", "today", "saved",
                            "per min before", "per min today", "per 100 ref today", "tom rule"], song_rows), "",
                  "`before` takes the offsets of the `default` run; `before (own offset)` takes each run's own offsets "
                  "(the saved fixes lie between the two numbers, see the caveats).", ""]

    lines += ["### Onset error of the matched hits (ms, after the offset; the `moved` part)", ""]
    body = []
    for c in ("before", "default"):
        if c not in runs:
            continue
        for role in ROLES:
            es = err_stats([e for s in names for e in res[c][s][role]["errs"]])
            own_moved = sum(own_res[c][s][role]["moved"] for s in names)
            fixed_moved = sum(res[c][s][role]["moved"] for s in names)
            body.append([c, role, es["n"], f"{es['median']:+.2f}", f"{es['p10']:+.2f}", f"{es['p90']:+.2f}",
                         f"{100 * es['beyond']:.0f} %", fixed_moved, own_moved])
    lines += [md_table(["config", "role", "matched", "median", "p10", "p90", "beyond 1 ms", "moved (default offset)",
                        "moved (own offset)"], body), "",
              "Fixes without the onset check (false + missed only):", ""]
    body = []
    if "before" in runs:
        for role in ROLES:
            a, b = total(res["before"], role), total(res["default"], role)
            body.append([role, a["false"] + a["missed"], b["false"] + b["missed"]])
        a, b = total(res["before"]), total(res["default"])
        body.append(["**all**", a["false"] + a["missed"], b["false"] + b["missed"]])
        lines += [md_table(["role", "before", "today"], body), ""]

    lines += ["### Per mic (false / missed / moved)", ""] if "before" not in runs else \
             ["### Per mic (today: false / missed / moved; before: false / missed / moved)", ""]
    body = []
    for s in names:
        for m, v in refs[s].items():
            a = res["before"][s]["mics"][m] if "before" in runs else {"false": "", "missed": "", "moved": ""}
            b = res["default"][s]["mics"][m]
            body.append([s, m, v["role"], b["n_ref"], b["n_out"], b["flagged"],
                         f"{b['false']} / {b['missed']} / {b['moved']}", f"{a['false']} / {a['missed']} / {a['moved']}",
                         f"{off[s][m] * 1000:+.1f}"])
    lines += [md_table(["song", "mic", "role", "ref hits", "output", "flagged", "today", "before", "offset ms"], body), "",
              "### References", ""]
    body = []
    for s in names:
        for m, v in refs[s].items():
            body.append([s, m, v["role"], len(v["onsets"]), v["source"]])
    lines += [md_table(["song", "mic", "role", "ref hits", "source"], body), ""]
    if drt is not None:
        dl = drt_tables(drt, drt_off, refs, minutes, ds=res["default"])
        (out / "drt-report.md").write_text("\n".join(dl))
        lines = dl + lines
    (out / "report.md").write_text("\n".join(lines))
    (out / "scores.json").write_text(json.dumps(
        {c: {s: {x: {k: v for k, v in res[c][s][x].items() if k != "errs"} for x in ROLES} for s in names}
         for c in res}))
    log("\n".join(lines))
    return res


# ---------------------------------------------------------------- P4-43: the DRT side (MIDI per song and mic)

MIDI_TICKS = 960        # `write_midi`: ticks per beat; at 120 BPM one tick is 0.52 ms
MIDI_TEMPO = 500000     # us per beat (120 BPM)


def midi_hits(path):
    """The hit times (s) of a MIDI file: every note-on with a velocity above 0, all tracks, the file's tempo map."""
    import mido
    t, out = 0.0, []
    for msg in mido.MidiFile(str(path)):
        t += msg.time
        if msg.type == "note_on" and msg.velocity > 0:
            out.append(t)
    return sorted(out)


def write_midi(path, times, note=36):
    """A one-track MIDI file with one note-on per time (s), 120 BPM, `MIDI_TICKS` per beat."""
    import mido
    per_s = MIDI_TICKS * 1e6 / MIDI_TEMPO
    ev = sorted([(round(t * per_s), 1, "note_on") for t in times] + [(round(t * per_s) + 4, 0, "note_off") for t in times])
    mf = mido.MidiFile(ticks_per_beat=MIDI_TICKS)
    tr = mido.MidiTrack()
    mf.tracks.append(tr)
    tr.append(mido.MetaMessage("set_tempo", tempo=MIDI_TEMPO, time=0))
    last = 0
    for tick, _, kind in ev:
        tr.append(mido.Message(kind, note=note, velocity=100 if kind == "note_on" else 0, time=tick - last))
        last = tick
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    mf.save(str(path))


def midi_path(midi_dir, song, mic):
    """`<dir>/<song>/<mic>.mid`; the app's safe names (`track_name`) also count."""
    for s in (song, track_name(song)):
        for m in (mic, track_name(mic)):
            p = Path(midi_dir) / s / f"{m}.mid"
            if p.is_file():
                return p
    return None


def read_midi_run(midi_dir, refs):
    """{song: {mic: [{"t": s}]}} of the songs that have a folder in `midi_dir`. A song folder needs a file for each mic."""
    ev, missing = {}, []
    for song, mics in refs.items():
        if not any((Path(midi_dir) / s).is_dir() for s in (song, track_name(song))):
            continue
        ev[song] = {}
        for m in mics:
            p = midi_path(midi_dir, song, m)
            if p is None:
                missing.append(f"{song}/{m}.mid")
                continue
            ev[song][m] = [{"t": t} for t in midi_hits(p)]
    if missing:
        raise SystemExit(f"{midi_dir}: no MIDI file for " + ", ".join(missing))
    if not ev:
        raise SystemExit(f"{midi_dir}: no song folder of {', '.join(refs)}")
    return ev


def score_midi(refs, ev, minutes):
    """The fixes of a MIDI run (`read_midi_run`): `score_run` with each mic's own offset (the DRT result's latency)."""
    off = {s: {m: track_offset(refs[s][m]["onsets"], ev[s][m]) for m in refs[s]} for s in ev}
    return score_run(refs, ev, off, minutes, songs=list(ev)), off


def do_refs_midi(src, midi_dir, log=print):
    """Write the reference hits of `<src>/refs.json` as `<midi_dir>/<song>/<mic>.mid` (a check: they score 0 fixes)."""
    refs_all = json.loads((Path(src) / "refs.json").read_text())
    for song, v in refs_all.items():
        for m, mv in v["mics"].items():
            write_midi(Path(midi_dir) / song / f"{m}.mid", mv["onsets"])
        log(f"{song}: {len(v['mics'])} MIDI files")


def drt_tables(drt, off, refs, minutes, ds=None):
    """The DRT tables: per song and role, per mic, and with `ds` (the `default` run's `score_run`) DRT against Drum Spike
    and the exit rule."""
    names = list(drt)
    mins = sum(minutes[s] for s in names)
    lines = ["### DRT: fixes per song and role (the MIDI of `--midi`; own offset per mic)", ""]
    body = []
    for s in names:
        for role in ROLES:
            r = drt[s][role]
            body.append([s, f1(minutes[s]), role, r["n_ref"], r["n_out"], r["false"], r["missed"], r["moved"],
                         f"**{r['fixes']}**", f1(r["per_min"]), f1(r["per_100"])])
    for role in ROLES:
        r = add_up([drt[s][role] for s in names], mins)
        body.append(["**all**", f1(mins), role, r["n_ref"], r["n_out"], r["false"], r["missed"], r["moved"],
                     f"**{r['fixes']}**", f1(r["per_min"]), f1(r["per_100"])])
    lines += [md_table(["song", "min", "role", "ref hits", "output", "false", "missed", "moved", "fixes", "per min",
                        "per 100 ref"], body), "", "DRT per mic (false / missed / moved):", ""]
    body = [[s, m, v["role"], drt[s]["mics"][m]["n_ref"], drt[s]["mics"][m]["n_out"],
             f"{drt[s]['mics'][m]['false']} / {drt[s]['mics'][m]['missed']} / {drt[s]['mics'][m]['moved']}",
             f"{off[s][m] * 1000:+.1f}"] for s in names for m, v in refs[s].items()]
    lines += [md_table(["song", "mic", "role", "ref hits", "output", "DRT", "offset ms"], body), ""]
    if ds is None:
        return lines

    def cell(r):
        return f"{r['false']} / {r['missed']} / {r['moved']} = **{r['fixes']}**"

    lines += ["### DRT against Drum Spike (false / missed / moved = fixes; Drum Spike: the `default` run, strict)", ""]
    body, verdict = [], []
    for s in names:
        for role in ROLES:
            a, b = drt[s][role], ds[s][role]
            body.append([s, role, b["n_ref"], cell(a), cell(b), a["fixes"] - b["fixes"]])
        a, b = (add_up([x[s][r] for r in ROLES], minutes[s]) for x in (drt, ds))
        body.append([s, "**all roles**", b["n_ref"], cell(a), cell(b), f"**{a['fixes'] - b['fixes']}**"])
        verdict.append((s, b["fixes"] < a["fixes"]))
    lines += [md_table(["song", "role", "ref hits", "DRT", "Drum Spike", "DRT - Drum Spike"], body), "",
              "Exit rule (Drum Spike has fewer fixes than DRT on each song, kick + snare + toms): **"
              + ("PASS" if all(ok for _, ok in verdict) else "FAIL") + "** ("
              + ", ".join(f"{s} {'yes' if ok else 'no'}" for s, ok in verdict) + ").", ""]
    return lines


def confirmed_tables(res, names, minutes, ctl, info):
    """The P4-16b tables: fixes strict and fixes without the confirmed strokes, today and before; the confirmation
    sources; the controls of the two rules."""
    mins = sum(minutes.values())

    def tot(c, role=None, s=None):
        sel = [res[c][x][r] for x in (names if s is None else [s]) for r in (ROLES if role is None else [role])]
        return add_up(sel, 1.0)

    lines = ["### Fixes strict and without confirmed strokes (P4-16b)", "",
             "Strict is the number of the tables above. A false hit is a **confirmed stroke** (not a fix in the second "
             "number) when (a) a sample layer of the session that is not the reference has an onset within 3 ms of it "
             "(`s samp savior`, `s samp sd3`, `s samp 2`, the kick layers; latency fixed on the reference hits), or (b) the "
             "rise test (10 dB over the peak 10 to 30 ms before) passes on the own mic and on the second mic of the same "
             "drum (`s bot`, `k out`; used by neither the app nor the reference) and no reference hit of the other role (kick "
             "or snare) lies within 12 ms (that is bleed, which both mics hear). The own mic alone never confirms, and "
             "no rule reads the app's onset times except as the place to look. Kick and snare only (no confirming file for "
             "the toms). (c) listened (P4-16f): the owner heard the hit as a real stroke (`LISTENED`, 8 snare flam "
             "strokes of the 2026-10-17 listening list, within 2 ms). Missed and moved hits are unchanged.", ""]
    head = ["song", "role", "ref hits", "false today", "of them confirmed (layer / witness / listened)", "strict today",
            "without confirmed today", "strict before", "without confirmed before"]
    body = []
    for s in names:
        for role in ROLES:
            b, a = res["default"][s][role], res["before"][s][role]
            body.append([s, role, b["n_ref"], b["false"], f"{b['confirmed']} ({b['confirmed_layer']} / "
                         f"{b['confirmed_witness']} / {b.get('confirmed_listened', 0)})", b["fixes"], f"**{b['fixes_ex']}**", a["fixes"], f"**{a['fixes_ex']}**"])
    for role in ROLES:
        b, a = tot("default", role), tot("before", role)
        body.append(["**all**", role, b["n_ref"], b["false"], f"{b['confirmed']} ({b['confirmed_layer']} / "
                     f"{b['confirmed_witness']} / {b.get('confirmed_listened', 0)})", b["fixes"], f"**{b['fixes_ex']}**", a["fixes"], f"**{a['fixes_ex']}**"])
    b, a = tot("default"), tot("before")
    body.append(["**all**", "**all roles**", b["n_ref"], b["false"], f"{b['confirmed']} ({b['confirmed_layer']} / "
                 f"{b['confirmed_witness']} / {b.get('confirmed_listened', 0)})", b["fixes"], f"**{b['fixes_ex']}**", a["fixes"], f"**{a['fixes_ex']}**"])
    lines += [md_table(head, body), "",
              f"Per minute (all roles): strict today {f1(b['fixes'] / mins)}, without confirmed today "
              f"{f1(b['fixes_ex'] / mins)}; strict before {f1(a['fixes'] / mins)}, without confirmed before "
              f"{f1(a['fixes_ex'] / mins)}.", ""]
    lines += ["Per song (all roles; strict / without confirmed):", ""]
    body = []
    for s in names:
        b, a = tot("default", None, s), tot("before", None, s)
        body.append([s, f1(minutes[s]), f"{b['fixes']} / {b['fixes_ex']}", f"{a['fixes']} / {a['fixes_ex']}",
                     f"{f1(b['fixes_ex'] / minutes[s])}"])
    lines += [md_table(["song", "min", "today", "before", "per min today (without confirmed)"], body), ""]
    lines += ["Controls (today's run): how often each rule confirms a time where no stroke is meant, against the matched hits "
              "(the rules can fire). `random`: times 200 ms or more from every reference hit and event; `moved 40 ms`: the "
              "matched hits moved by 40 ms (kept 20 ms or more from any hit). Cells: layer % / witness % (times tested).", ""]
    body = []
    for s in names:
        for m, c in ctl[s].items():
            i = info[s][m]
            cell = lambda k: f"{100 * c[k][0]:.0f} / {100 * c[k][1]:.0f} ({c[k][2]})"
            body.append([s, m, ", ".join(f"`{x}`" for x in i["layers"]) or "none", f"`{i['witness']}`" if i["witness"] else "none",
                         cell("matched"), cell("moved40"), cell("random")])
    lines += [md_table(["song", "mic", "layers", "witness", "matched hits", "moved 40 ms", "random"], body), ""]
    return lines


# ---------------------------------------------------------------- #276: the detector check and the compare report

def detector_name(d):
    """The detector string of a track's `detector` record (a `rust-dsp` name, or the record dict
    as `jobs::detector_record` writes it: `{"detector": ..., "chain": ..., "refine": ...}`)."""
    if isinstance(d, dict):
        return d.get("detector")
    return d


def do_check_detectors(out, log=print):
    """Print the detector counts per configuration from `<out>/run/*.json` and FAIL when a track did not
    run the DSP detector. The `detector` record of each track: `rust-dsp` (the DSP detector)."""
    run = Path(out) / "run"
    bad = False
    for p in sorted(run.glob("*_default.json")) + sorted(run.glob("*_dsp.json")):
        config = "default" if p.name.endswith("_default.json") else "dsp"
        want = "rust-dsp"
        det = json.loads(p.read_text()).get("detector", {})
        for mic, d in sorted(det.items()):
            if detector_name(d) != want:
                log(f"FAIL {p.name} {mic}: detector {d!r} in the {config} configuration")
                bad = True
    counts = {c: {} for c in ("default", "dsp")}
    for p in sorted(run.glob("*_default.json")) + sorted(run.glob("*_dsp.json")):
        config = "default" if p.name.endswith("_default.json") else "dsp"
        for d in json.loads(p.read_text()).get("detector", {}).values():
            d = detector_name(d)
            counts[config][d] = counts[config].get(d, 0) + 1
    for c in ("default", "dsp"):
        if not counts[c]:
            log(f"FAIL no run files for the {c} configuration in {run}")
            bad = True
        log(f"{c}: " + ", ".join(f"{d} {n}" for d, n in sorted(counts[c].items())))
    if bad:
        raise SystemExit(1)
    log("detectors OK")


def do_compare(out, log=print):
    """The part A tables of #276: the fix-metric score of the `default` and the `dsp`
    configurations per song and role, with totals (both run the DSP detector; the difference
    shows the effect of the pinned rule environment). Reads `<out>/scores.json` (from `score --ref fair`)."""
    scores = json.loads((Path(out) / "scores.json").read_text())
    res = {c: {s: {r: scores[c][s][r] for r in ROLES} for s in scores[c]} for c in ("default", "dsp") if c in scores}
    if len(res) != 2:
        raise SystemExit(f"need the `default` and the `dsp` configurations in {out}/scores.json")
    names = list(res["default"])
    lines = ["### Part A: fixes per song and role (`ref fair`)", ""]
    head = ["song", "role", "ref hits", "default (rust-dsp): false / missed / moved = fixes",
            "dsp (rust-dsp): false / missed / moved = fixes", "difference (dsp - default)",
            "default per 100 ref", "dsp per 100 ref"]
    body = []

    def cell(r):
        return f"{r['false']} / {r['missed']} / {r['moved']} = **{r['fixes']}**"

    for s in names:
        for role in ROLES:
            a, b = res["default"][s][role], res["dsp"][s][role]
            body.append([s, role, b["n_ref"], cell(a), cell(b), b["fixes"] - a["fixes"], f1(a["per_100"]),
                         f1(b["per_100"])])
    for role in ROLES:
        a = {k: sum(res["default"][s][role][k] for s in names) for k in ("false", "missed", "moved", "fixes", "n_ref")}
        b = {k: sum(res["dsp"][s][role][k] for s in names) for k in ("false", "missed", "moved", "fixes", "n_ref")}
        body.append(["**all**", role, b["n_ref"], cell(a), cell(b), b["fixes"] - a["fixes"],
                     f1(100.0 * a["fixes"] / a["n_ref"]), f1(100.0 * b["fixes"] / b["n_ref"])])
    a = {k: sum(res["default"][s][x][k] for s in names for x in ROLES) for k in ("false", "missed", "moved", "fixes", "n_ref")}
    b = {k: sum(res["dsp"][s][x][k] for s in names for x in ROLES) for k in ("false", "missed", "moved", "fixes", "n_ref")}
    body.append(["**all**", "**all roles**", b["n_ref"], cell(a), cell(b), b["fixes"] - a["fixes"],
                 f1(100.0 * a["fixes"] / a["n_ref"]), f1(100.0 * b["fixes"] / b["n_ref"])])
    lines += [md_table(head, body), ""]
    log("\n".join(lines))
    (Path(out) / "compare.md").write_text("\n".join(lines))
    return a["fixes"], b["fixes"]


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["refs", "run", "score", "all", "refs-midi", "check-detectors", "compare"])
    ap.add_argument("--configs", default=None, help="run/score: a comma-separated subset of the configurations "
                                                    "(default: all; #276 uses `default,dsp`)")
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--midi", default=None, help="score: a folder `<song>/<mic>.mid` of DRT results (P4-43); refs-midi: "
                                                 "the folder to write the references to")
    ap.add_argument("--run", default=None, help="score: the folder with `refs.json` and `run/` (default: --out)")
    ap.add_argument("--ref", choices=["fair", "union"], default="fair",
                    help="score: the kick and snare reference: fair (trigger file, else lowest jitter; default) or union "
                         "(P4-16: the reference of refs.json)")
    ap.add_argument("--kick-ref", choices=["fair", "union"], default=None, help="score: the kick reference alone (overrides --ref)")
    ap.add_argument("--snare-ref", choices=["fair", "union"], default=None, help="score: the snare reference alone (overrides --ref)")
    ap.add_argument("--hat", default=None, help="score: kick-onset `tracks.json` with the hat jitter (default: P4-22's)")
    ap.add_argument("--hat-snare", default=None, help="score: snare-onset `tracks.json` with the hat jitter (default: P4-23's)")
    ap.add_argument("--tool", default=None, help="run: the service_tool (default: the release build)")
    a = ap.parse_args(argv)
    cfgs = [c.strip() for c in a.configs.split(",")] if a.configs else None
    if a.cmd in ("refs", "all"):
        do_refs(a.out)
    if a.cmd in ("run", "all"):
        do_run(a.out, a.tool, configs=cfgs)
    if a.cmd == "refs-midi":
        if not a.midi:
            ap.error("refs-midi needs --midi DIR")
        do_refs_midi(a.run or a.out, a.midi)
    if a.cmd in ("score", "all"):
        do_score(a.out, run=a.run, ref=a.ref, hat=a.hat, hat_snare=a.hat_snare, kick_ref=a.kick_ref, snare_ref=a.snare_ref,
                 midi=a.midi, configs=cfgs)
    if a.cmd == "check-detectors":
        do_check_detectors(a.run or a.out)
    if a.cmd == "compare":
        do_compare(a.run or a.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
