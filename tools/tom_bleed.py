"""P4-14 stage 1: tom bleed on the three real sessions. A measurement, no change under crates/.

    uv run python tools/tom_bleed.py run        # the app chain (refinement version 2) on the drum mics of each session
    uv run python tools/tom_bleed.py attribute  # `parity attribute` (P2-6) on the app's hits: the flag and the features
    uv run python tools/tom_bleed.py learn      # the Learn score of each tom candidate (`service_tool learn-eval`,
                                                # --learn-tool <service_tool> when the release build lacks it)
    uv run python tools/tom_bleed.py features   # the features per tom hit, labelled by the references
    uv run python tools/tom_bleed.py score      # the tables: P2-6 flag removed, AUC per feature, the rule held out
    uv run python tools/tom_bleed.py all

Hits: the app's analysis (`service_tool cli run`: project_new, tracks_add, analyze_start, events_get, through
`tools/miss_causes.py` `run_item`) with `DRUMSPIKE_REFINE_RULE=2`, on the kick, snare, tom, hi-hat and overhead mics
(the P2-6 session tracks). Ground truth: the P4-6b references (`~/.cache/drumspike/learn-eval/refs.json`), one offset
per mic, one to one within +-20 ms (`tools/rescue_real.py` `offset_of`, `prf`); Session A t4 is excluded (its reference
is wrong). A tom hit that a reference owns is real, every other tom hit is false.

Features per tom hit (each calibrated on the session itself):
  ex_ab, dom   the P2-6 excess `ex_ab = L_A - L_B - G(B->A)` and `ex_ba - ex_ab` of the best source B under the P2-6
               timing (`parity attribute`, margin and dominance tests off); no source: ex_ab +40, dom -40
  lag_ms       arrival order: source attack minus hit (ms) of the best source in +-7 ms, any rise (negative: the
               source mic heard it first); no source: +7
  level_db     peak in the tom band (50 to 400 Hz) over [-0.5, +8] ms against the median of the track's strong hits
               (broadband peak within 6 dB of the 90th percentile, as the P2-6 calibration)
  ring_db      energy at the tom's own fundamental (+-1 semitone; the fundamental is the peak of the strong hits'
               mean spectrum, 50 to 400 Hz) 20 to 120 ms after the hit, own mic minus the louder of the snare and
               kick mics, against the median of the strong hits
  ring_own_db  the same energy on the own mic alone, against the strong hits
  tom_rel_db   level_db minus the highest level_db of the session's other tom mics at the same time (each against
               its own strong hits): the tom mic of the drum that was hit is the loudest of the toms. Added after
               the first held-out run (the false tom hits are mostly other toms, see the report); the split of the
               combined tom reference of Session B and Session C used a similar comparison, so its AUC
               there is not independent
  learn        the Learn score (`similarity.rs`, the app's recipe) of the nearest candidate within 30 ms; examples
               as P4-6b (5 "this" from the reference, 2 "not"); the 7 example hits are left out of its AUC

Rule: remove a tom hit when one feature (or two, both) are on the bleed side of a threshold; chosen on Session A (the
best F with at most 1 % of its real tom hits lost), held out on Session B and Session C. Kick and
snare tracks are never touched. Inputs are read only; outputs are under `~/.cache/drumspike/tom-bleed/`.
"""
import argparse
import itertools
import json
import math
import re
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
import soundfile as sf

sys.path.insert(0, str(Path(__file__).resolve().parent))
import calib_kit as ck  # noqa: E402
import miss_causes as mc  # noqa: E402
import rescue_real as rr  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
TARGET = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
SAMPLES = rr.SAMPLES
REFS = rr.REFS
OUT = Path.home() / ".cache" / "drumspike" / "tom-bleed"
TOL = rr.TOL
REACH = 0.030                      # learn.rs CANDIDATE_REACH
MAX_LOST = 0.01                    # the card: at most 1 % of the real tom hits lost
NO_SOURCE = {"ex_ab": 40.0, "dom": -40.0, "lag_ms": 7.0}

# (stem, role, instrument) per mic; the P2-6 session tracks: kick in, snare top, toms, hi-hat, overheads
SESSIONS = {
    "Session A": {"dir": "Session A", "excluded": {"t4"},
                "mics": {"k in": ("k in_02", "kick", "k in"), "s top": ("s top_02", "snare", "s top"),
                         "t1": ("t1 hh_02", "tom", "t1"), "t2": ("t2_02", "tom", "t2"), "t3": ("t3_02", "tom", "t3"),
                         "t4": ("t4 ri_02", "tom", "t4"), "hh": ("hh_03", "hat", "hh"), "oh": ("oh_01", "cymbal", "oh")}},
    "Session B": {"dir": "Session B", "excluded": set(),
                           "mics": {"k in": ("k in_01", "kick", "k in"), "s top": ("s top_01", "snare", "s top"),
                                    "t1": ("t1 HH_01", "tom", "t1"), "t2": ("t2_01", "tom", "t2"),
                                    "t3": ("t3_01", "tom", "t3"), "t4": ("t4 Ri_01", "tom", "t4"),
                                    "hh": ("hh_01", "hat", "hh"), "oh": ("OH_01", "cymbal", "oh")}},
    "Session C": {"dir": "Session C", "excluded": set(),
                       "mics": {"k in": ("k in_02", "kick", "k in"), "s top": ("s top_02", "snare", "s top"),
                                "t1": ("t 1_03", "tom", "t1"), "t2": ("t 2_03", "tom", "t2"),
                                "t3": ("t 3_03", "tom", "t3"), "t4": ("t 4_03", "tom", "t4"),
                                "hh": ("hh_03", "hat", "hh"), "oh L": ("oh_03.L", "cymbal", "oh"),
                                "oh R": ("oh_03.R", "cymbal", "oh")}},
}
DEV = "Session A"
HELD_OUT = ["Session B", "Session C"]
FEATURES = ["ex_ab", "dom", "lag_ms", "level_db", "ring_db", "ring_own_db", "tom_rel_db", "learn"]
# +1: a higher value means a real hit; -1: a higher value means bleed (fixed before any measurement)
DIRECTION = {"ex_ab": 1, "dom": -1, "lag_ms": 1, "level_db": 1, "ring_db": 1, "ring_own_db": 1, "tom_rel_db": 1, "learn": 1,
             "tom_rel_c": 1, "exc_d": 1, "exc_cd": 1}   # stage 1c (`tools/tom_bleed_cd.py`)
RULE_FEATURES = ["ex_ab", "dom", "lag_ms", "level_db", "ring_db", "ring_own_db"]   # learn needs the owner's marks
# A: the card's features, chosen before any held-out result was seen; B: with tom_rel_db, added after A's held-out run
RULES = {"A": RULE_FEATURES, "B": RULE_FEATURES + ["tom_rel_db"]}

# P2-6 parameter sets for `parity attribute` (bleed only, targets: toms only)
P26 = {"bleed_enabled": True, "resonance_enabled": False, "bleed_roles": ["tom"]}
P26_EXCESS = {**P26, "margin_db": 1e9, "dominance_db": -1e9}
P26_ARRIVAL = {**P26, "margin_db": 1e9, "dominance_db": -1e9, "window_ms": 7.0, "lag_tol_ms": 7.0,
               "source_rise_db": -1e9, "source_match_ms": 1e9}
PARAM_SETS = {"flag": P26, "excess": P26_EXCESS, "arrival": P26_ARRIVAL}


def use_sessions(sessions):
    """Point the module at another session table (P4-14 stage 1b, `tools/tom_bleed_new.py`: the new sessions)."""
    global SESSIONS
    SESSIONS = sessions


def audio_path(session, mic):
    return SAMPLES / SESSIONS[session]["dir"] / "Audio Files" / f"{SESSIONS[session]['mics'][mic][0]}.wav"


def tid(session, mic):
    return rr.safe(session, mic)


def all_tracks():
    """[(session, mic, role, instrument)] of every mic of every session."""
    return [(s, m, v[1], v[2]) for s, cfg in SESSIONS.items() for m, v in cfg["mics"].items()]


def tom_tracks(with_excluded=False):
    return [(s, m) for s, m, role, _ in all_tracks()
            if role == "tom" and (with_excluded or m not in SESSIONS[s]["excluded"])]


# ---------------------------------------------------------------- run

def run_all(out, jobs=3, log=print):
    """The app's analysis of every mic, refinement version 2 pinned; `work/<id>/model/{events.json,act.bin}`."""
    out = Path(out)
    raise SystemExit("#281 D16: this comparison moved to the feature branch; it does not run on main")
    tool, model = mc.find_tool(), ck.find_model()  # noqa: unreachable on main
    if not tool.is_file():
        raise SystemExit(f"need service_tool ({tool}) and the model ({model})")
    os.environ["DRUMSPIKE_REFINE_RULE"] = "2"
    (out / "audio").mkdir(parents=True, exist_ok=True)
    items = []
    for s, m, role, _ in all_tracks():
        link = out / "audio" / f"{tid(s, m)}.wav"
        if link.is_symlink() or link.exists():
            link.unlink()
        link.symlink_to(audio_path(s, m))
        items.append({"id": tid(s, m), "audio": str(link), "role": role, "note": mc.ROLE_NOTE[role]})
    with ThreadPoolExecutor(max_workers=jobs) as ex:
        futs = [(i, ex.submit(mc.run_item, tool, out, i, "model", model)) for i in items]
        for i, f in futs:
            f.result()
            log(f"ran {i['id']}")
    return len(items)


def load_events(out, session, mic):
    """(sr, [(t, rise)]) of the app's events of a mic."""
    d = json.loads((Path(out) / "work" / tid(session, mic) / "model" / "events.json").read_text())
    return int(d["sr"]), sorted((float(e["t"]), e.get("rise")) for e in d["events"])


# ---------------------------------------------------------------- P2-6 attribution

def manifest(out, session, params):
    tracks = []
    for m, (_, role, inst) in SESSIONS[session]["mics"].items():
        sr, ev = load_events(out, session, m)
        tracks.append({"id": m, "role": role, "instrument": inst, "path": str(audio_path(session, m)), "mix": "ffmpeg",
                       "hits": [{"sample": int(round(t * sr)), "rise": r} for t, r in ev]})
    return {"session": session, "params": params, "tracks": tracks}


def run_attribute(out, log=print):
    out = Path(out)
    parity = TARGET / "release" / "parity"
    if not parity.is_file():
        raise SystemExit(f"need {parity}: cargo build --release -p drumspike-devtools --bin parity")
    (out / "attr").mkdir(parents=True, exist_ok=True)
    for s in SESSIONS:
        for name, params in PARAM_SETS.items():
            man = out / "attr" / f"{rr.safe(s, name)}.manifest.json"
            res = out / "attr" / f"{rr.safe(s, name)}.json"
            man.write_text(json.dumps(manifest(out, s, params)))
            r = subprocess.run([str(parity), "attribute", str(man), str(res)], capture_output=True, text=True)
            if r.returncode != 0:
                raise RuntimeError(f"parity attribute {s} {name}: {r.stderr[-400:]}")
            log(f"attribute {s} {name}")


def load_attr(out, session, name):
    """{mic: [hit dict]} of one `parity attribute` result (input order = sorted event order)."""
    d = json.loads((Path(out) / "attr" / f"{rr.safe(session, name)}.json").read_text())
    if not isinstance(d, dict) or not isinstance(d.get("tracks"), list):
        raise SystemExit(f"attr {session} {name}: unexpected shape {type(d).__name__}")
    return {t["id"]: t["hits"] for t in d["tracks"]}, int(d["sr"])


# ---------------------------------------------------------------- signal features

def rbj(kind, f, sr):
    """RBJ biquad (Q = 1/sqrt(2)) as (b0, b1, b2, a1, a2), the stage of `attribution.rs` `Biquad::new`."""
    w = 2 * math.pi * f / sr
    cw, sw = math.cos(w), math.sin(w)
    alpha = sw / (2 * math.sqrt(0.5))
    b = [(1 - cw) / 2, 1 - cw, (1 - cw) / 2] if kind == "lp" else [(1 + cw) / 2, -(1 + cw), (1 + cw) / 2]
    a0 = 1 + alpha
    return (b[0] / a0, b[1] / a0, b[2] / a0, -2 * cw / a0, (1 - alpha) / a0)


def tom_sections(sr):
    """The tom band of P2-6 (`Role::band`): highpass 50 Hz and lowpass 400 Hz, each stage twice (24 dB per octave)."""
    hp, lp = rbj("hp", 50.0, sr), rbj("lp", 400.0, sr)
    return [hp, hp, lp, lp]


def band_peaks_db(x, sr, times, sections, window=(-0.0005, 0.008), preroll=0.050):
    """Peak level (dB) of `x` through `sections` in `[t + window[0], t + window[1])` for every t, filter state zero at
    `preroll` before the window (direct form II transposed, vectorised over the hits)."""
    times = np.asarray(times, float)
    if not len(times):
        return np.zeros(0)
    pre, w0, w1 = int(round(preroll * sr)), int(round(window[0] * sr)), int(round(window[1] * sr))
    n = pre + w1 - w0
    starts = np.round(times * sr).astype(np.int64) + w0 - pre
    idx = starts[:, None] + np.arange(n)[None, :]
    seg = np.where((idx >= 0) & (idx < len(x)), x[np.clip(idx, 0, len(x) - 1)], 0.0).astype(np.float64)
    y = seg
    for b0, b1, b2, a1, a2 in sections:
        z0 = np.zeros(len(times))
        z1 = np.zeros(len(times))
        out = np.empty_like(y)
        for k in range(n):
            v = y[:, k]
            o = b0 * v + z0
            z0 = b1 * v - a1 * o + z1
            z1 = b2 * v - a2 * o
            out[:, k] = o
        y = out
    return 20 * np.log10(np.abs(y[:, pre:]).max(axis=1) + 1e-9)


def broad_peaks_db(x, sr, times, window=(-0.0005, 0.005)):
    out = []
    for t in times:
        a, b = max(0, int(round((t + window[0]) * sr))), min(len(x), int(round((t + window[1]) * sr)))
        out.append(20 * math.log10(float(np.abs(x[a:b]).max()) + 1e-9) if b > a else -180.0)
    return np.asarray(out)


def strong_mask(x, sr, times, strong_db=6.0):
    """The P2-6 strong hits: broadband peak in [-0.5, +5] ms within `strong_db` of the 90th percentile."""
    pk = broad_peaks_db(x, sr, times)
    return pk >= np.percentile(pk, 90) - strong_db if len(pk) else np.zeros(0, bool)


NFFT = 1 << 15
RING = (0.020, 0.120)


def ring_spectrum(x, sr, t, ring=RING):
    a, b = int(round((t + ring[0]) * sr)), int(round((t + ring[1]) * sr))
    seg = x[max(0, a):min(len(x), b)].astype(np.float64)
    if len(seg) < 64:
        return None
    return np.abs(np.fft.rfft(seg * np.hanning(len(seg)), NFFT)) ** 2


def fundamental(x, sr, strong_times, lo=50.0, hi=400.0):
    """The peak (Hz) of the mean normalised ring spectrum of the strong hits in `lo`..`hi`."""
    f = np.fft.rfftfreq(NFFT, 1 / sr)
    sel = (f >= lo) & (f <= hi)
    acc = np.zeros(int(sel.sum()))
    for t in strong_times:
        p = ring_spectrum(x, sr, t)
        if p is not None and p[sel].sum() > 0:
            acc += p[sel] / p[sel].sum()
    return float(f[sel][int(np.argmax(acc))]) if acc.any() else None


def ring_db(x, sr, times, f0, semis=1.0):
    """Energy (dB) in f0 * 2^(+-semis/12) of the ring window after every t."""
    f = np.fft.rfftfreq(NFFT, 1 / sr)
    sel = (f >= f0 * 2 ** (-semis / 12)) & (f <= f0 * 2 ** (semis / 12))
    out = []
    for t in times:
        p = ring_spectrum(x, sr, t)
        out.append(10 * math.log10(float(p[sel].sum()) + 1e-20) if p is not None else -200.0)
    return np.asarray(out)


def relative_to_other_toms(lvl, ref, others):
    """`lvl - ref` (each hit against the track's strong hits) minus the highest `o_lvl - o_ref` of the other tom mics
    at the same times; no other tom: +200 dB (the hit is never on the bleed side)."""
    own = np.asarray(lvl, float) - ref
    if not others:
        return own + 200.0
    return own - np.max([np.asarray(o, float) - r for o, r in others], axis=0)


CLOSE_TOM_NOT = re.compile(r"samp|trig", re.I)


def is_close_tom(stem):
    """False for the mixer's sample and trigger tracks (`t1 sampl`, `t samp`, `t trigger`, `t sample sd3`): they are
    not other drums, and `guess_role` calls them tom (P4-14 verifier: with Session A's `t<n> sampl` as other toms, rule B
    lost 21 of 131 real hits)."""
    return not CLOSE_TOM_NOT.search(stem)


def tom_rel_all(signals, times, sr):
    """{tom mic: (its strong-hit level, tom_rel_db at its hits)} for the tom close mics of one session. Each mic's
    level is measured against ITS OWN strong hits (median tom-band peak of the hits within 6 dB of its 90th
    percentile); a hit's tom_rel_db is its own relative level minus the highest relative level of the other mics at the
    same time (`relative_to_other_toms`)."""
    ref = {}
    for m, x in signals.items():
        t = np.asarray(times[m], float)
        st = strong_mask(x, sr, t) if len(t) else np.zeros(0, bool)
        ref[m] = float(np.median(band_peaks_db(x, sr, t[st], tom_sections(sr)))) if st.any() else 0.0
    out = {}
    for m, x in signals.items():
        t = np.asarray(times[m], float)
        lvl = band_peaks_db(x, sr, t, tom_sections(sr))
        out[m] = (ref[m], relative_to_other_toms(lvl, ref[m], [(band_peaks_db(signals[o], sr, t, tom_sections(sr)),
                                                                ref[o]) for o in signals if o != m]))
    return out


def stroke_kind(r, refs, unison=0.010, flam=0.050, double=0.250):
    """How a tom hit row `r` sits among the session's tom reference hits (the row's offset applied): `unison` when
    another tom's reference has a hit within `unison`, `flam` when a reference hit of any tom lies `unison` to `flam`
    before it, `double` when its own tom's reference has a hit `flam` to `double` before it, else None. P4-13 needs
    the second strokes kept."""
    t = r["t"] - r.get("offset_ms", 0.0) / 1000
    toms = [k for k in refs[r["session"]] if k != "toms_combined" and k not in ("k in", "s top")]
    other = [np.asarray(refs[r["session"]][k]) for k in toms if k != r["mic"] and refs[r["session"]][k]]
    own = np.asarray(refs[r["session"]].get(r["mic"]) or [])
    if any(float(np.min(np.abs(o - t))) <= unison for o in other):
        return "unison"
    before = [t - v for v in itertools.chain(own, *other) if unison < t - v <= flam]
    if before:
        return "flam"
    if len(own) and any(flam < t - v <= double for v in own):
        return "double"
    return None


def read_mono(path):
    x, sr = sf.read(str(path), dtype="float32", always_2d=True)
    return x.mean(axis=1), sr


# ---------------------------------------------------------------- Learn score

def run_learn(out, tool=None, log=print):
    """`service_tool learn-eval`: the candidates, then the Learn scores with P4-6b's examples, per tom track. `tool`:
    a service_tool with `learn-eval` (default: the one of `run`)."""
    out = Path(out)
    raise SystemExit("#281 D16: the Learn scores moved to the feature branch; they do not run on main")
    tool, model = Path(tool) if tool else mc.find_tool(), ck.find_model()
    refs = json.loads(REFS.read_text())
    cache = out / "learn"
    cache.mkdir(parents=True, exist_ok=True)
    env = {**os.environ}

    def call(jobs, name):
        spec = cache / f"{name}.spec.json"
        spec.write_text(json.dumps({"cache": str(cache), "jobs": jobs}))
        r = subprocess.run([str(tool), "learn-eval", str(spec)], capture_output=True, text=True, env=env)
        rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
        bad = [x for x in rows if not isinstance(x, dict) or "error" in x]
        if r.returncode != 0 or bad or len(rows) != len(jobs):
            raise RuntimeError(f"learn-eval {name}: exit {r.returncode} {bad[:1]} {r.stderr[-400:]}")
        return rows

    toms = tom_tracks()
    cands = call([{"id": tid(s, m), "wav": str(audio_path(s, m)), "role": "tom", "mode": "cands"} for s, m in toms],
                 "cands")
    jobs = []
    for (s, m), c in zip(toms, cands):
        _, ev = load_events(out, s, m)
        ref = np.asarray(refs[s].get(m) or [])
        off = rr.offset_of(ref, [t for t, _ in ev])
        this, not_ = pick_examples(ref + off, c["candidates"])
        jobs.append({"id": tid(s, m), "wav": str(audio_path(s, m)), "role": "tom", "mode": "learn", "sensitivity": 100,
                     "this": this, "not": not_, "grid": [],
                     "variants": [{"name": "app", "recipe": "patch-1", "weights": [1.0, 1.0, 0.5, 0.5]}]})
    rows = call(jobs, "learn")
    res = {r["id"]: {"this": j["this"], "not": j["not"], "scores": r["variants"][0]["scores"]}
           for j, r in zip(jobs, rows)}
    (out / "learn.json").write_text(json.dumps(res))
    log(f"learn scores of {len(res)} tom tracks")
    return res


def pick_examples(ref, cands, after=10.0, n_this=5, n_not=2):
    """P4-6b (`tools/learn_eval.py` `pick_examples`): the first `n_this` reference hits after `after` s that match a
    candidate within REACH (the candidate's time), and the `n_not` strongest candidates with no reference hit within
    50 ms. `cands` rows are `[t, class, strength, v]`."""
    ct = np.asarray([c[0] for c in cands], float)
    this = []
    for t in np.sort(np.asarray(ref, float)):
        if t < after or not len(ct):
            continue
        i = int(np.argmin(np.abs(ct - t)))
        if abs(ct[i] - t) <= REACH and float(ct[i]) not in this:
            this.append(float(ct[i]))
        if len(this) == n_this:
            break
    rs = np.sort(np.asarray(ref, float))
    free = [c for c in cands if not (len(rs) and np.abs(rs - c[0]).min() <= 0.050)]
    free.sort(key=lambda c: -c[2])
    not_ = []
    for c in free:
        if float(c[0]) not in not_:
            not_.append(float(c[0]))
        if len(not_) == n_not:
            break
    return this, not_


def nearest(times, values, t, reach=REACH):
    """The value at the time nearest to `t` within `reach`, else None."""
    if not len(times):
        return None
    i = int(np.argmin(np.abs(times - t)))
    return values[i] if abs(times[i] - t) <= reach else None


# ---------------------------------------------------------------- features per tom hit

def label_hits(ref, times, tol=TOL):
    """(offset, [True/False per hit]): a hit is real when the shifted reference owns it one to one within `tol`."""
    off = rr.offset_of(ref, times)
    pairs, _, _ = ck.match(list(np.asarray(ref) + off), list(times), tol)
    owned = {j for _, j in pairs}
    return off, [j in owned for j in range(len(times))]


def session_features(out, session, refs, learn, log=print):
    cfg = SESSIONS[session]
    sig = {m: read_mono(audio_path(session, m)) for m in cfg["mics"] if cfg["mics"][m][1] in ("tom", "snare", "kick")}
    flag, sr = load_attr(out, session, "flag")
    excess, _ = load_attr(out, session, "excess")
    arrival, _ = load_attr(out, session, "arrival")
    toms = [m for m, v in cfg["mics"].items() if v[1] == "tom"]
    for m in toms:
        if not is_close_tom(cfg["mics"][m][0]):
            raise SystemExit(f"{session} {m}: {cfg['mics'][m][0]} is a sample or trigger track, not a tom close mic")
    rel_all = tom_rel_all({m: sig[m][0] for m in toms},
                          {m: np.asarray([t for t, _ in load_events(out, session, m)[1]]) for m in toms}, sr)
    rows = []
    for m in toms:
        x, xsr = sig[m]
        if xsr != sr:
            raise SystemExit(f"{session} {m}: rate {xsr}, session {sr}")
        _, ev = load_events(out, session, m)
        times = np.asarray([t for t, _ in ev])
        if len(flag[m]) != len(times):
            raise SystemExit(f"{session} {m}: {len(flag[m])} attributed hits for {len(times)} events")
        excluded = m in cfg["excluded"]
        ref = refs[session].get(m) or []
        known = m in refs[session] and not excluded       # an empty reference: every hit of the mic is false
        off, real = label_hits(ref, times) if known else (0.0, [None] * len(times))
        strong = strong_mask(x, sr, times)
        lvl = band_peaks_db(x, sr, times, tom_sections(sr))
        lvl_ref, tom_rel = rel_all[m]
        f0 = fundamental(x, sr, times[strong])
        own = ring_db(x, sr, times, f0)
        oth = np.maximum(ring_db(sig["s top"][0], sr, times, f0), ring_db(sig["k in"][0], sr, times, f0))
        vs = own - oth
        own_ref, vs_ref = float(np.median(own[strong])), float(np.median(vs[strong]))
        ld = learn.get(tid(session, m))
        lt = np.asarray([c[0] for c in ld["scores"]]) if ld else np.zeros(0)
        lv = [c[1] for c in ld["scores"]] if ld else []
        examples = np.asarray((ld["this"] + ld["not"]) if ld else [])
        for j, t in enumerate(times):
            fl, exc, arr = flag[m][j], excess[m][j]["bleed"], arrival[m][j]["bleed"]
            li = nearest(lt, lv, t)
            is_example = bool(len(examples)) and float(np.abs(examples - t).min()) <= REACH
            rows.append({
                "session": session, "mic": m, "t": round(float(t), 6), "real": real[j], "excluded": excluded,
                "strong": bool(strong[j]), "f0": f0, "offset_ms": off * 1000,
                "p26": "bleed" in fl["flags"],
                "ex_ab": exc["excess_db"] if exc else NO_SOURCE["ex_ab"],
                "dom": exc["dominance_db"] if exc else NO_SOURCE["dom"],
                "source": exc["source"] if exc else None,
                "lag_ms": arr["lag_samples"] / sr * 1000 if arr else NO_SOURCE["lag_ms"],
                "level_db": float(lvl[j] - lvl_ref), "ring_db": float(vs[j] - vs_ref),
                "ring_own_db": float(own[j] - own_ref),
                "tom_rel_db": float(tom_rel[j]),
                "learn": None if li is None or is_example else float(li)})
        log(f"{session} {m}: {len(times)} hits, f0 {f0:.1f} Hz, {int(strong.sum())} strong")
    return rows


def do_features(out, log=print):
    out = Path(out)
    refs = json.loads(REFS.read_text())
    lp = out / "learn.json"
    learn = json.loads(lp.read_text()) if lp.is_file() else {}
    rows = []
    for s in SESSIONS:
        rows += session_features(out, s, refs, learn, log)
    (out / "features.json").write_text(json.dumps(rows))
    return rows


# ---------------------------------------------------------------- scoring

def auc(pos, neg):
    """P(a positive scores above a negative), ties half (Mann-Whitney); None when a side is empty."""
    pos, neg = np.asarray(pos, float), np.asarray(neg, float)
    if not len(pos) or not len(neg):
        return None
    allv = np.concatenate([pos, neg])
    order = np.argsort(allv, kind="mergesort")
    ranks = np.empty(len(allv))
    sv = allv[order]
    i = 0
    while i < len(sv):
        j = i
        while j + 1 < len(sv) and sv[j + 1] == sv[i]:
            j += 1
        ranks[order[i:j + 1]] = (i + j) / 2 + 1
        i = j + 1
    r = ranks[:len(pos)].sum()
    return float((r - len(pos) * (len(pos) + 1) / 2) / (len(pos) * len(neg)))


def feature_auc(rows, feat):
    """AUC of real against false hits in the feature's fixed direction (above 0.5: the feature separates as expected)."""
    d = DIRECTION[feat]
    pos = [d * r[feat] for r in rows if r["real"] is True and r[feat] is not None]
    neg = [d * r[feat] for r in rows if r["real"] is False and r[feat] is not None]
    return auc(pos, neg)


def removes(rule, r):
    """True when the rule removes hit row `r`. A rule is [(feature, threshold)]: every feature on its bleed side."""
    for feat, th in rule:
        v = r[feat]
        if v is None or DIRECTION[feat] * v >= DIRECTION[feat] * th:
            return False
    return True


def track_score(ref_n, rows, keep):
    """found/false/F of the kept hits of one track (rows labelled by the references)."""
    kept = [r for r in rows if keep(r)]
    tp = sum(1 for r in kept if r["real"])
    fp = len(kept) - tp
    p = tp / len(kept) if kept else 0.0
    rc = tp / ref_n if ref_n else 0.0
    return {"tp": tp, "fp": fp, "fn": ref_n - tp, "f": 2 * p * rc / (p + rc) if p + rc else 0.0}


def pooled(scores):
    tp, fp, fn = (sum(s[k] for s in scores) for k in ("tp", "fp", "fn"))
    p = tp / (tp + fp) if tp + fp else 0.0
    rc = tp / (tp + fn) if tp + fn else 0.0
    return {"tp": tp, "fp": fp, "fn": fn, "f": 2 * p * rc / (p + rc) if p + rc else 0.0}


def by_track(rows):
    out = {}
    for r in rows:
        out.setdefault((r["session"], r["mic"]), []).append(r)
    return out


def evaluate(rows, refs, keep, sessions):
    """{(session, mic): score} and pooled per session and over `sessions` for a keep predicate."""
    tr = by_track([r for r in rows if r["session"] in sessions and not r["excluded"]])
    per = {k: track_score(len(refs[k[0]][k[1]]), v, keep) for k, v in tr.items()}
    return per, {s: pooled([v for k, v in per.items() if k[0] == s]) for s in sessions}, pooled(list(per.values()))


def candidate_thresholds(rows, feat, n=60):
    v = np.asarray([r[feat] for r in rows if r[feat] is not None and r["real"] is not None], float)
    return sorted(set(np.round(np.quantile(v, np.linspace(0.01, 0.99, n)), 3).tolist())) if len(v) else []


def choose_rule(rows, refs, session=DEV, feats=RULE_FEATURES, max_lost=MAX_LOST, n=40):
    """The rule of one or two features with the best pooled tom F on `session` that removes at most `max_lost` of its
    real tom hits; ties go to the rule that removes fewer real hits, then to the single feature."""
    dev = [r for r in rows if r["session"] == session and not r["excluded"]]
    n_real = sum(1 for r in dev if r["real"])
    th = {f: candidate_thresholds(dev, f, n) for f in feats}
    cands = [[(f, t)] for f in feats for t in th[f]]
    cands += [[(f, a), (g, b)] for i, f in enumerate(feats) for g in feats[i + 1:] for a in th[f] for b in th[g]]
    best = None
    for rule in cands:
        lost = sum(1 for r in dev if r["real"] and removes(rule, r))
        if lost > max_lost * n_real:
            continue
        _, _, pool = evaluate(rows, refs, lambda r, rule=rule: not removes(rule, r), [session])
        key = (round(pool["f"], 6), -lost, -len(rule))
        if best is None or key > best[0]:
            best = (key, rule, lost)
    return {"rule": best[1], "lost": best[2], "n_real": n_real} if best else None


def fmt(s):
    return f"{s['tp']}/{s['fp']}/{s['f']:.3f}"


def do_score(out, log=print):
    out = Path(out)
    refs = json.loads(REFS.read_text())
    rows = json.loads((out / "features.json").read_text())
    sessions = list(SESSIONS)
    lines = []

    def keep_all(r):
        return True

    def keep_p26(r):
        return not r["p26"]

    base_per, base_s, base_all = evaluate(rows, refs, keep_all, sessions)
    p26_per, p26_s, p26_all = evaluate(rows, refs, keep_p26, sessions)
    real_n = {k: sum(1 for r in v if r["real"]) for k, v in by_track(rows).items()}
    head = ["session", "track", "refs", "base found/false/F", "P2-6 removed found/false/F",
            "flagged real", "flagged false"]
    body = []
    for k in base_per:
        tr = by_track(rows)[k]
        body.append([k[0], k[1], len(refs[k[0]][k[1]]), fmt(base_per[k]), fmt(p26_per[k]),
                     sum(1 for r in tr if r["p26"] and r["real"]), sum(1 for r in tr if r["p26"] and not r["real"])])
    for s in sessions:
        sel = [r for r in rows if r["session"] == s and not r["excluded"]]
        body.append([f"**{s}**", "**toms**", sum(len(refs[s][m]) for ss, m in base_per if ss == s), fmt(base_s[s]),
                     fmt(p26_s[s]), sum(1 for r in sel if r["p26"] and r["real"]),
                     sum(1 for r in sel if r["p26"] and not r["real"])])
    sel = [r for r in rows if not r["excluded"]]
    body.append(["**all**", "**toms**", sum(len(refs[k[0]][k[1]]) for k in base_per), fmt(base_all), fmt(p26_all),
                 sum(1 for r in sel if r["p26"] and r["real"]), sum(1 for r in sel if r["p26"] and not r["real"])])
    excl = [r for r in rows if r["excluded"]]
    lines += ["### P2-6 bleed flag on refinement version 2", "", mc.md_table(head, body), "",
              f"Excluded tracks (no reference): {len(excl)} hits, {sum(1 for r in excl if r['p26'])} flagged.", ""]

    head = ["session", "false tom hits", "another tom's reference", "snare's", "kick's", "none",
            "P2-6 flagged (tom/snare/kick/none)"]
    body = []
    for s in sessions + ["all"]:
        sel = [r for r in rows if r["real"] is False and s in (r["session"], "all")]
        own = [false_owner(r, refs) for r in sel]
        cnt = {k: sum(1 for o in own if o == k) for k in ("tom", "s top", "k in", None)}
        fl = {k: sum(1 for o, r in zip(own, sel) if o == k and r["p26"]) for k in ("tom", "s top", "k in", None)}
        body.append([s, len(sel), cnt["tom"], cnt["s top"], cnt["k in"], cnt[None],
                     f"{fl['tom']}/{fl['s top']}/{fl['k in']}/{fl[None]}"])
    lines += ["### What the false tom hits are (the nearest other reference hit within 30 ms)", "",
              mc.md_table(head, body), ""]

    head = ["feature"] + sessions + ["all", "hits with a value (real/false)"]
    body = []
    for f in FEATURES:
        cells = []
        for s in sessions:
            a = feature_auc([r for r in rows if r["session"] == s], f)
            cells.append("-" if a is None else f"{a:.3f}")
        a = feature_auc(rows, f)
        nr = sum(1 for r in rows if r["real"] is True and r[f] is not None)
        nf = sum(1 for r in rows if r["real"] is False and r[f] is not None)
        body.append([f] + cells + ["-" if a is None else f"{a:.3f}", f"{nr}/{nf}"])
    lines += ["### AUC per feature (real against false tom hits; above 0.5: separates in the expected direction)", "",
              mc.md_table(head, body), ""]

    res = {"base": base_all, "p26": p26_all, "rules": {}}
    for name, feats in RULES.items():
        sec, r = rule_section(rows, refs, name, feats, base_per, base_s)
        lines += sec
        res["rules"][name] = r
    (out / "scores.json").write_text(json.dumps(res))
    (out / "report.md").write_text("\n".join(lines))
    log("\n".join(lines))
    return res


def false_owner(r, refs, window=0.030, lead=0.004):
    """The other reference (`tom` for any other tom, `s top`, `k in`) with a hit within `window` of the hit row `r`
    (reference times plus `lead`, the typical offset, as `tools/rescue_real.py` `review_track`); None when none."""
    best = None
    for k, v in refs[r["session"]].items():
        if k == r["mic"] or not v:
            continue
        d = float(np.abs(np.asarray(v) + lead - r["t"]).min())
        if d <= window and (best is None or d < best[1]):
            best = (k, d)
    if best is None:
        return None
    return "tom" if best[0] not in ("s top", "k in") else best[0]


def stroke_counts(rows, refs, rule):
    """{"real"|"false": {kind: hits the rule removes}} with the kinds of `stroke_kind` (None: a single stroke)."""
    out = {"real": {}, "false": {}}
    for r in rows:
        if r["real"] is None or r["excluded"] or not removes(rule, r):
            continue
        k = stroke_kind(r, refs) or "single"
        side = out["real" if r["real"] else "false"]
        side[k] = side.get(k, 0) + 1
    return out


def stroke_line(rows, refs, rule):
    c = stroke_counts(rows, refs, rule)

    def cell(side):
        return ", ".join(f"{k} {c[side].get(k, 0)}" for k in ("unison", "flam", "double", "single"))

    return (f"Removed hits by stroke (unison: another tom's reference within 10 ms; flam: a tom reference hit 10 to 50 ms "
            f"before; double: its own tom's reference 50 to 250 ms before): real {cell('real')}; false {cell('false')}.")


def rule_text(rule):
    return " and ".join(f"{f} {'<' if DIRECTION[f] > 0 else '>'} {t:g}" for f, t in rule)


def leave_one_track_out(rows, refs, feats, session=DEV):
    """Per tom track of `session`: the rule chosen on its other tom tracks, applied to it (the dev set's own
    references only). [(mic, rule, base score, rule score, real lost)]."""
    out = []
    mics = sorted({r["mic"] for r in rows if r["session"] == session and not r["excluded"]})
    for m in mics:
        train = [r for r in rows if r["session"] == session and r["mic"] != m]
        test = [r for r in rows if r["session"] == session and r["mic"] == m]
        ch = choose_rule(train, refs, session, feats)
        if not ch:
            continue
        rule = ch["rule"]
        base = track_score(len(refs[session][m]), test, lambda r: True)
        sc = track_score(len(refs[session][m]), test, lambda r, rule=rule: not removes(rule, r))
        out.append((m, rule, base, sc, sum(1 for r in test if r["real"] and removes(rule, r))))
    return out


def rule_section(rows, refs, name, feats, base_per, base_s):
    """The report lines and the result of one rule family: chosen on DEV, every track, the held-out pool."""
    sessions = list(SESSIONS)
    ch = choose_rule(rows, refs, DEV, feats)
    if not ch:
        return [f"### Rule {name}: no rule keeps the cap on {DEV}", ""], None
    rule = ch["rule"]

    def keep(r):
        return not removes(rule, r)

    rule_per, rule_s, rule_all = evaluate(rows, refs, keep, sessions)
    _, _, held_base = evaluate(rows, refs, lambda r: True, HELD_OUT)
    _, _, held_rule = evaluate(rows, refs, keep, HELD_OUT)
    head = ["session", "track", "base found/false/F", "rule found/false/F", "real lost", "false removed"]
    body = []
    for k in rule_per:
        tr = by_track(rows)[k]
        body.append([k[0], k[1], fmt(base_per[k]), fmt(rule_per[k]), sum(1 for r in tr if r["real"] and removes(rule, r)),
                     sum(1 for r in tr if r["real"] is False and removes(rule, r))])
    for s in sessions:
        sel = [r for r in rows if r["session"] == s and not r["excluded"]]
        body.append([f"**{s}**", "**toms**", fmt(base_s[s]), fmt(rule_s[s]),
                     sum(1 for r in sel if r["real"] and removes(rule, r)),
                     sum(1 for r in sel if r["real"] is False and removes(rule, r))])
    held = [r for r in rows if r["session"] in HELD_OUT and not r["excluded"]]
    lost = sum(1 for r in held if r["real"] and removes(rule, r))
    n_real = sum(1 for r in held if r["real"])
    body.append(["**held out**", "**toms**", fmt(held_base), fmt(held_rule), lost,
                 sum(1 for r in held if r["real"] is False and removes(rule, r))])
    excl = [r for r in rows if r["excluded"]]
    loo = leave_one_track_out(rows, refs, feats)
    lines = [f"### Rule {name} (features {', '.join(feats)}): remove a tom hit when {rule_text(rule)}", "",
             f"Chosen on {DEV}: {ch['lost']} of {ch['n_real']} real tom hits lost. Held out: {lost} of {n_real} real "
             f"tom hits lost ({100 * lost / max(1, n_real):.1f} %). Excluded tracks: {sum(1 for r in excl if removes(rule, r))} "
             f"of {len(excl)} hits removed.", "", mc.md_table(head, body), "", stroke_line(rows, refs, rule), "",
             f"Leave one track out on {DEV} (the rule chosen on the other tom tracks of {DEV}):", "",
             mc.md_table(["track", "rule", "base found/false/F", "rule found/false/F", "real lost"],
                         [[m, rule_text(rl), fmt(b), fmt(sc), n] for m, rl, b, sc, n in loo]), ""]
    return lines, {"rule": rule, "text": rule_text(rule), "dev_lost": ch["lost"], "dev_real": ch["n_real"],
                   "all": rule_all, "held_base": held_base, "held_rule": held_rule, "held_lost": lost,
                   "held_real": n_real, "loo": [[m, rl, b, sc, n] for m, rl, b, sc, n in loo]}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["run", "attribute", "learn", "features", "score", "all"])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--jobs", type=int, default=3)
    ap.add_argument("--learn-tool", default=None, help="service_tool with learn-eval (default: the one of run)")
    a = ap.parse_args(argv)
    out = Path(a.out)
    if a.cmd in ("run", "all"):
        run_all(out, a.jobs)
    if a.cmd in ("attribute", "all"):
        run_attribute(out)
    if a.cmd in ("learn", "all"):
        run_learn(out, a.learn_tool)
    if a.cmd in ("features", "all"):
        do_features(out)
    if a.cmd in ("score", "all"):
        do_score(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
