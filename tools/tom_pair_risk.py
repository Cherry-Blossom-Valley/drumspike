"""#177: the tom pair rule's risk to real cross-tom flams and ghost notes (report only).

    uv run --with numpy --with soundfile --with mido python tools/tom_pair_risk.py

One command, resumable (cached sets, runs and merges in `~/.cache/drumspike/tom-pair-risk/`);
it writes `docs/migration/reports/tom-pair-risk.md`. No product change. Heavy: the release
`service_tool` (the current app chain, the rule version read from the code) runs the synthetic sets and the close
mics of the three real sessions; on x86-mac under the host lock:

    crewboard lock run ~/.cache/drumspike/heavy.lock.d --holder agent@host \
      --reason tom-pair-risk --wait 3600 -- \
      uv run --with numpy --with soundfile --with mido python tools/tom_pair_risk.py

Part (a), the synthetic cross-tom sets. The DRT kit is not on x86-mac and has no tom file (P4-10:
"the DRT kit has no tom"), so the two toms are cut from a real session tom close mic (Session A
`t2_02`, one isolated shot, peak -6 dBFS); tom B is the same shot tuned 1 and 3 semitones up (the
two tunings). Each set: a backbone of one normal hit per tom per 1.2 s slot and 12 planted pairs,
the two strokes 5/10/15/20/30 ms apart, alternating which tom plays the quiet stroke; flams: the
grace 6/9/12 dB under the main, grace first; ghosts: 12/18/24 dB under the accent, ghost after.
Both pair strokes are real strokes. A stroke counts as planted only when the rendered audio
carries it (peak within 6 dB of the intended level), so a silent render is a FAIL, not a miss;
each row prints planted, found and lost (to the pair rule / to the DSP detector) and FAILs on 0 planted.
Solo sets (quiet strokes with no partner within 600 ms) give the DSP-only baseline.

Part (b), the toms of the 3 real sessions (Session A, Session B, Session C). The
session's close mics run twice, rule version 2 and version 3; a merge is a tom hit flagged in the
version 3 run and unflagged in the version 2 run (the pair rule's marks; the DSP detector is
deterministic, so the event lists are identical). Every merge is listed with time and track and
gets one cause, the first that fits (P4-21b style): a tom sample layer within 6 ms (Session A only:
x86-mac holds no layer files for the other two sessions), the own mic's attack rise 10 dB or more
(`tom_fixes.stroke_rise`), bleed (a kept hit of another tom within 12 ms, 6 dB or more louder),
or other. The relative level and the medians come from the version 2 run's survivors, the pair
rule's actual input.

Verdict: "no real stroke lost" with the numbers, or the smallest guard that keeps the lost
strokes without new false hits on the real sessions: a sweep of the guard "keep a marked hit when
its own-mic rise is X dB or more" (and the layer guard), measured on the merges; the smallest X
that keeps every real-stroke merge and unmarks no bleed merge wins. Measured, not shipped.
"""
import argparse
import json
import math
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import soundfile as sf

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fix_metric as fm  # noqa: E402
import firm_fixes as ff  # noqa: E402
import tom_fixes as tf  # noqa: E402
import tom_pairs as tp  # noqa: E402
import learn_eval as le  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
OUT = Path.home() / ".cache" / "drumspike" / "tom-pair-risk"
REPORT = ROOT / "docs" / "migration" / "reports" / "tom-pair-risk.md"
TOOL = Path(os.environ.get("DRUMSPIKE_CALIB_TOOL") or ROOT / "target" / "release" / "service_tool")
SAMPLES = Path.home() / ".cache" / "drumspike" / "samples"      # the owner hosts' session cache
CBV_SESSIONS = Path("/Users/x86-mac/sessions")                    # x86-mac's real sessions
PIN_KEYS = {"DRUMSPIKE_REFINE_RULE", "DRUMSPIKE_BLEED_RULE", "DRUMSPIKE_ROLL_RULE",
            "DRUMSPIKE_FLAM_RULE", "DRUMSPIKE_KICK_RESCUE", "DRUMSPIKE_SNARE_RESCUE"}

# the 3 real sessions (stems of `rescue_real.SESSIONS`; Session A t4 is excluded there)
SESSIONS = {
    "Session A": {"mixdir": "Session A", "safe": "Session_A",
                "mics": {"k in": ("k in_02", "kick"), "s top": ("s top_02", "snare"),
                         "t1": ("t1 hh_02", "tom"), "t2": ("t2_02", "tom"), "t3": ("t3_02", "tom")}},
    "Session B": {"mixdir": "Session B", "safe": "Session_B",
                           "mics": {"k in": ("k in_01", "kick"), "s top": ("s top_01", "snare"),
                                    "t1": ("t1 HH_01", "tom"), "t2": ("t2_01", "tom"),
                                    "t3": ("t3_01", "tom"), "t4": ("t4 Ri_01", "tom")}},
    "Session C": {"mixdir": "Session C", "safe": "Session_C",
                       "mics": {"k in": ("k in_02", "kick"), "s top": ("s top_02", "snare"),
                                "t1": ("t 1_03", "tom"), "t2": ("t 2_03", "tom"),
                                "t3": ("t 3_03", "tom"), "t4": ("t 4_03", "tom")}},
}
# the tom sample layers on x86-mac (only Session A has them here; P4-21b `TOM_LAYERS`)
LAYERS = {"Session A": {"t1": ["t1 sampl_01"], "t2": ["t2 sampl_01"], "t3": ["t3 sampl_01"]}}

GAPS_MS = [5, 10, 15, 20, 30]
TUNINGS = [1, 3]
FLAM_UNDER = [6, 9, 12]      # the grace under the main, dB
GHOST_UNDER = [12, 18, 24]   # the ghost under the accent, dB
SOLO_UNDER = [12, 18, 24]
N_SLOTS = 12
SLOT_S = 1.2
LEAD_S = 1.0
TAIL_S = 1.0
NORMAL_DBFS = -6.0
SHOT_LEN_S = 0.45
MATCH_TOL_S = 0.010
RENDER_TOL_DB = 6.0
RISE_DB = tp.STROKE_DB            # 10.0, the P4-18 rise test
LAYER_TOL_MS = tp.SAMPLE_TOL_S * 1000   # 6 ms
GUARD_STEPS = [6, 8, 10, 12, 15, 20, 25]


def lin(db):
    return 10.0 ** (db / 20.0)


def rule_version():
    """The tom bleed rule's version, read from the Rust source (the code is the truth, not the
    report template)."""
    src = ROOT / "crates" / "core" / "src" / "detect" / "tom_bleed.rs"
    m = re.search(r"pub const RULE_VERSION: u32 = (\d+);", src.read_text())
    if not m:
        raise SystemExit(f"no RULE_VERSION in {src}")
    return int(m.group(1))


def session_file(song, stem):
    """The path of one session track: the owner hosts' samples cache first, then x86-mac."""
    for p in (SAMPLES / SESSIONS[song]["mixdir"] / "Audio Files" / f"{stem}.wav",
              CBV_SESSIONS / SESSIONS[song]["safe"] / f"{stem}.wav"):
        if p.is_file():
            return p
    raise SystemExit(f"missing audio: {song} `{stem}` (neither cache has it)")


# ---------------------------------------------------------------- part (a): the sets

def extract_shot(x, sr, clear_s=1.0):
    """The loudest hit of `x` with `clear_s` of no other peak within 6 dB on both sides, cut to
    `SHOT_LEN_S` from 10 ms before its attack, faded, normalized to `NORMAL_DBFS`."""
    hop = max(1, int(round(0.005 * sr)))
    nb = len(x) // hop
    blk = np.abs(np.asarray(x, float)[:nb * hop]).reshape(nb, hop).max(axis=1)
    db = 20 * np.log10(np.maximum(blk, 1e-12))
    guard = max(1, int(0.020 / 0.005))   # the attack's own blocks never count against it
    body = max(guard, int(0.250 / 0.005))  # the same hit's body within 6 dB neither
    order = np.argsort(-db)
    for i in order:
        d = db[i]
        lo, hi = max(0, i - int(clear_s / 0.005)), min(nb, i + int(clear_s / 0.005))
        left, right = db[lo:max(lo, i - guard)], db[min(nb, i + body + 1):hi]
        if not (left >= d - 6.0).any() and not (right >= d - 6.0).any():
            a = max(0, i * hop - int(0.010 * sr))
            shot = np.asarray(x[a:a + int(SHOT_LEN_S * sr)], float).copy()
            if len(shot) < int(SHOT_LEN_S * sr):
                continue
            fi, fo = int(0.002 * sr), int(0.030 * sr)
            shot[:fi] *= np.linspace(0, 1, fi)
            shot[-fo:] *= np.linspace(1, 0, fo)
            peak = np.abs(shot).max()
            return shot * lin(NORMAL_DBFS) / peak, (i * hop) / sr
    raise SystemExit("no isolated hit found for the shot")


def pitch_up(x, semitones):
    """`x` tuned `semitones` up (resampled playback; a one-shot keeps its envelope shape)."""
    ratio = 2.0 ** (semitones / 12.0)
    idx = np.arange(0, len(x), ratio)
    return np.interp(idx, np.arange(len(x)), x)


def put(buf, x, t, sr):
    i = int(round(t * sr))
    n = min(len(x), len(buf) - i)
    if n > 0:
        buf[i:i + n] += x[:n]


def set_plans():
    plans = []
    for tun in TUNINGS:
        for gap in GAPS_MS:
            plans.append({"name": f"flam_g{gap:02d}_t{tun}", "kind": "flam", "gap": gap, "tun": tun})
        for gap in GAPS_MS:
            plans.append({"name": f"ghost_g{gap:02d}_t{tun}", "kind": "ghost", "gap": gap, "tun": tun})
        plans.append({"name": f"solo_t{tun}", "kind": "solo", "gap": None, "tun": tun})
    return plans


def build_set(plan, shot, sr, out_dir, n_slots=N_SLOTS, lead_s=LEAD_S, slot_s=SLOT_S):
    """Render one set: {name: tracks {track: samples}, planted: strokes}. Returns the strokes and
    the intended peak (dBFS) of each. The quiet stroke alternates between the toms; the under
    levels cycle. The pair sits 0.45 s into a slot, 0.45 s after the own tom's backbone hit."""
    shot_b = pitch_up(shot, plan["tun"])
    peak = np.abs(shot_b).max()
    shot_b = shot_b * lin(NORMAL_DBFS) / peak
    dur = lead_s + n_slots * slot_s + TAIL_S
    tracks = {"t1": np.zeros(int(dur * sr)), "t2": np.zeros(int(dur * sr))}
    strokes = []
    g = (plan["gap"] or 0) / 1000.0
    for i in range(n_slots):
        s = lead_s + i * slot_s
        quiet_t1 = i % 2 == 0
        if plan["kind"] == "solo":
            under = SOLO_UNDER[i % 3]
            for tr in ("t1", "t2"):
                t = s + (0.0 if tr == "t1" else 0.6)
                put(tracks[tr], shot if tr == "t1" else shot_b, t, sr)
                strokes.append({"track": tr, "t": round(t, 4), "kind": "quiet", "under_db": under,
                                "scored": True, "intended_dbfs": NORMAL_DBFS - under})
        else:
            put(tracks["t1"], shot, s, sr)
            put(tracks["t2"], shot_b, s + 0.9, sr)
            strokes.append({"track": "t1", "t": round(s, 4), "kind": "backbone", "scored": False,
                            "intended_dbfs": NORMAL_DBFS})
            strokes.append({"track": "t2", "t": round(s + 0.9, 4), "kind": "backbone", "scored": False,
                            "intended_dbfs": NORMAL_DBFS})
            under = (FLAM_UNDER if plan["kind"] == "flam" else GHOST_UNDER)[i % 3]
            qt = "t1" if quiet_t1 else "t2"
            lt = "t2" if quiet_t1 else "t1"
            qshot = shot if qt == "t1" else shot_b
            lshot = shot if lt == "t1" else shot_b
            if plan["kind"] == "flam":          # grace first, main after
                qt_off, lt_off = 0.45, 0.45 + g
            else:                               # accent first, ghost after
                qt_off, lt_off = 0.45 + g, 0.45
            put(tracks[qt], qshot * lin(-under), s + qt_off, sr)
            put(tracks[lt], lshot, s + lt_off, sr)
            strokes.append({"track": qt, "t": round(s + qt_off, 4),
                            "kind": "grace" if plan["kind"] == "flam" else "ghost",
                            "under_db": under, "scored": True, "intended_dbfs": NORMAL_DBFS - under})
            strokes.append({"track": lt, "t": round(s + lt_off, 4),
                            "kind": "main" if plan["kind"] == "flam" else "accent",
                            "under_db": 0, "scored": True, "intended_dbfs": NORMAL_DBFS})
    d = Path(out_dir)
    d.mkdir(parents=True, exist_ok=True)
    for tr, buf in tracks.items():
        sf.write(str(d / f"{tr}.wav"), np.clip(buf, -1, 1), sr, subtype="PCM_24")
    (d / "planted.json").write_text(json.dumps({"set": plan["name"], "sr": sr, "strokes": strokes}))
    return strokes


def render_check(set_dir, sr, log=print):
    """The rendered audio must carry every scored stroke (peak within `RENDER_TOL_DB` of the
    intended level, measured like the rule does: peak in [t - 5 ms, t + 30 ms])."""
    d = Path(set_dir)
    bad = []
    sig = {}
    for tr in ("t1", "t2"):
        x, r = le.read_mono([d / f"{tr}.wav"])
        sig[tr] = (x, r)
    sr = sig["t1"][1]
    for st in json.loads((d / "planted.json").read_text())["strokes"]:
        if not st["scored"]:
            continue
        x = sig[st["track"]][0]
        got = ff.peak_db(x, sr, st["t"])
        if got < st["intended_dbfs"] - RENDER_TOL_DB:
            bad.append((st["track"], st["t"], st["kind"], round(got, 1), st["intended_dbfs"]))
    if bad:
        log(f"{d.name}: {len(bad)} strokes not in the render: {bad[:3]}")
    return bad


def app_run(name, tracks, env_extra, log=print):
    """The app's analysis of one project (the current app chain) via the release `service_tool`;
    cached to `OUT/run/<name>.json`. `tracks`: [(track name, path, role, note)]."""
    cache = OUT / "run" / f"{fm.track_name(name)}.json"
    if cache.is_file():
        return json.loads(cache.read_text())
    if not TOOL.is_file():
        raise SystemExit(f"need the release service_tool ({TOOL})")
    work = OUT / "scratch" / fm.track_name(name)
    if work.exists():
        shutil.rmtree(work)
    (work / "p").mkdir(parents=True)
    steps = [{"command": "project_new", "args": {"name": "tp", "dir": str(work / "p")}},
             {"command": "tracks_add", "args": {"sid": "$sid", "paths": [t[1] for t in tracks],
                                                "roles": [t[2] for t in tracks],
                                                "notes": [t[3] for t in tracks]}, "wait": True},
             {"command": "analyze_start", "args": {"sid": "$sid"}, "wait": True},
             {"command": "analyze_status", "args": {"sid": "$sid"}},
             {"command": "tracks_get", "args": {"sid": "$sid"}}]
    steps += [{"command": "events_get", "args": {"sid": "$sid", "name": fm.track_name(Path(t[1]).stem)}}
              for t in tracks]
    (work / "steps.json").write_text(json.dumps(steps))
    env = {k: v for k, v in os.environ.items() if k not in PIN_KEYS}
    env.update({"DRUMSPIKE_HOME": str(work / "h"), "DRUMSPIKE_PROJECTS": str(work / "p")})
    env.update(env_extra)
    r = subprocess.run([str(TOOL), "cli", "run", str(work / "steps.json")], capture_output=True,
                       text=True, env=env, stdin=subprocess.DEVNULL, timeout=3600)
    rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
    bad = [x for x in rows if not x.get("ok", False)]
    if r.returncode != 0 or bad or len(rows) != len(steps):
        raise RuntimeError(f"{name}: exit {r.returncode} bad {bad[:1]} err {r.stderr[-300:]}")
    res = {"job": rows[3]["result"], "env": env_extra,
           "events": {t[0]: rows[5 + i]["result"]["events"] for i, t in enumerate(tracks)}}
    cache.parent.mkdir(parents=True, exist_ok=True)
    cache.write_text(json.dumps(res))
    shutil.rmtree(work, ignore_errors=True)
    return res


def stroke_fate(events_by_track, strokes, tol=MATCH_TOL_S):
    """Per scored stroke: found (an output event within `tol`), rule (an event is there but the
    pair rule flagged it: bleed or cut), or dsp (no event at all)."""
    out = []
    for st in strokes:
        evs = events_by_track.get(st["track"], [])
        near = [e for e in evs if abs(float(e["t"]) - st["t"]) <= tol]
        if not near:
            out.append(dict(st, fate="dsp"))
            continue
        e = min(near, key=lambda e: abs(float(e["t"]) - st["t"]))
        out.append(dict(st, fate="rule" if (e.get("bleed") or e.get("cut")) else "found",
                        ev_t=float(e["t"])))
    return out


# ---------------------------------------------------------------- part (b): the sessions

def diff_marks(v2_events, v3_events, tol=0.002):
    """The pair rule's marks: per tom mic, the version 3 bleed events that the version 2 run does
    not flag (the two runs share the model's event times)."""
    out = {}
    for m, ev3 in v3_events.items():
        f2 = [float(e["t"]) for e in v2_events.get(m, []) if e.get("bleed")]
        out[m] = [e for e in ev3 if e.get("bleed")
                  and not any(abs(float(e["t"]) - t) <= tol for t in f2)]
    return out


def layer_onsets(song, mic):
    """The shifted onsets (s) of mic's tom sample layers, the latency fixed against the version 2
    run's own events of that mic (`fm.layer_offset`; no reference file on x86-mac)."""
    out = []
    v2 = json.loads((OUT / "run" / f"{fm.track_name(song)}_v2.json").read_text())
    ref = sorted(float(e["t"]) for e in v2["events"][mic])
    for stem in LAYERS.get(song, {}).get(mic, []):
        x, sr = le.read_mono([session_file(song, stem)])
        on = le.ref_onsets(x, sr, gap=0.005)
        off = fm.layer_offset(ref, list(on))
        out += [t - off for t in on]
    return sorted(out)


def session_merges(song, v2, v3):
    """The pair rule's merges of one session with their evidence: {mic: [rows]}."""
    toms = [m for m, (s, role) in SESSIONS[song]["mics"].items() if role == "tom"]
    marks = diff_marks(v2["events"], {m: v3["events"][m] for m in toms})
    sig, sr = {}, None
    for m in toms:
        x, r = le.read_mono([session_file(song, SESSIONS[song]["mics"][m][0])])
        sig[m], sr = x, r
    hp = {m: tf.highpass(x, sr) for m, x in sig.items()}
    # the pair rule's input, per mic: the version 2 survivors with their levels (computed once)
    surv, med, marked_t, lay_cache = {}, {}, {}, {}
    for m in toms:
        st = [float(e["t"]) for e in v2["events"][m] if not e.get("bleed") and not e.get("cut")]
        lv = [ff.peak_db(sig[m], sr, t) for t in st]
        surv[m] = list(zip(st, lv))
        med[m] = float(np.median(lv)) if lv else None
        marked_t[m] = {float(e["t"]) for e in marks[m]}
    rows = {}
    for m in toms:
        for st in marks[m]:
            t = float(st["t"])
            rel = ff.peak_db(sig[m], sr, t) - med[m] if med[m] is not None else None
            rise = tf.stroke_rise(sig[m], hp[m], sr, t)[2]
            if m not in lay_cache:
                lay_cache[m] = layer_onsets(song, m)
            lay = tf.nearest_abs(lay_cache[m], t) * 1000 if lay_cache[m] else None
            partner, gain = "", None
            for o in toms:
                if o == m or med[o] is None or rel is None:
                    continue
                for et, olvl in surv[o]:
                    if abs(et - t) > tp.WINDOW_S + 1e-9 or any(abs(et - mt) <= 0.002 for mt in marked_t[o]):
                        continue
                    g = (olvl - med[o]) - rel
                    if g >= tp.MARGIN_DB and (gain is None or g > gain):
                        partner, gain = o, g
            if lay is not None and lay <= LAYER_TOL_MS:
                cause = "real stroke (tom sample layer)"
            elif rise >= RISE_DB:
                cause = "real stroke (own-mic attack rise)"
            elif partner:
                cause = "bleed"
            else:
                cause = "other"
            rows.setdefault(m, []).append({"t": t, "rel_db": rel, "rise_own": rise, "layer_ms": lay,
                                           "partner": partner, "gain_db": gain, "cause": cause})
    return rows


def guard_sweep(rows):
    """((kind, X, kept_real, new_false) per step, the chosen guard) for the rise guard; the layer
    guard keeps exactly the layer-confirmed rows (its `new false` is 0 by the cause order)."""
    real = [r for rs in rows.values() for r in rs if r["cause"].startswith("real")]
    bleed = [r for rs in rows.values() for r in rs if r["cause"] == "bleed"]
    steps = []
    chosen = None
    for x in GUARD_STEPS:
        kept = sum(1 for r in real if r["rise_own"] >= x)
        nf = sum(1 for r in bleed if r["rise_own"] >= x)
        steps.append(("rise", x, kept, nf))
        if chosen is None and kept == len(real) and nf == 0:
            chosen = ("rise", x)
    lay_kept = sum(1 for r in real if r["layer_ms"] is not None and r["layer_ms"] <= LAYER_TOL_MS)
    return steps, chosen, (lay_kept, len(real))


# ---------------------------------------------------------------- the report

def md_table(head, body):
    lines = ["| " + " | ".join(head) + " |", "|" + "|".join("---" for _ in head) + "|"]
    lines += ["| " + " | ".join(str(c) for c in row) + " |" for row in body]
    return "\n".join(lines)


def write_report(res, log=print):
    L = ["# Tom pair rule: lost cross-tom flams and ghost notes (#177, report only)", "",
         f"One command: `uv run --with numpy --with soundfile --with mido python tools/tom_pair_risk.py`",
         f"(resumable; outputs in `~/.cache/drumspike/tom-pair-risk/`). Run {res['when']} on {res['host']},",
         f"release `service_tool` at repo {res['sha']}, the current app chain (DSP detector)",
         f"(rule version {rule_version()}). No product change in this ticket.", "",
         "Sound: the DRT kit is not on x86-mac and has no tom file (P4-10), so the planted sets use a",
         "real tom shot cut from the Session A tom close mic `t2_02` (peak -6 dBFS); tom B is the same",
         "shot tuned 1 and 3 semitones up (the two tunings), as documented in the tool.", ""]
    a = res["sets"]
    L += ["## Part (a): the synthetic cross-tom sets", "",
          "Each set: a backbone of one normal hit per tom per 1.2 s and 12 planted pairs, the strokes",
          "5/10/15/20/30 ms apart, alternating which tom plays the quiet stroke. Flams: the grace 6/9/12 dB",
          "under the main, grace first. Ghosts: 12/18/24 dB under the accent, ghost after. Both pair",
          "strokes are real strokes; `planted` counts the pair strokes that the rendered audio carries",
          "(peak within 6 dB of the intended level). Lost is split into `to the pair rule` (the DSP detector",
          "found it, the rule flagged it out) and `to the DSP` (no event at all).", "",
          md_table(["set", "kind", "gap ms", "tuning st", "planted", "found", "lost rule", "lost dsp", "check"],
                   a["rows"]), ""]
    L += ["Solo baseline (quiet strokes with no partner within 600 ms; the DSP-only ghost rate):", "",
          md_table(["under dB", "tuning st", "planted", "found", "lost dsp"],
                   a["solo_rows"]), ""]
    b = res["sessions"]
    L += ["## Part (b): the toms of the 3 real sessions", "",
          "Each session's close mics run twice (rule version 2 and 3); a merge is a tom hit flagged in",
          "the version 3 run and unflagged in the version 2 run - the pair rule's marks. Causes, the",
          "first that fits: a tom sample layer within 6 ms (Session A only), the own-mic attack rise 10 dB",
          "or more, bleed (a kept hit of another tom within 12 ms, 6 dB or more louder), or other.", ""]
    for song, s in b.items():
        L += [f"### {song}", "",
              md_table(["track", "time s", "mm:ss.mmm", "rel dB", "rise own dB", "layer ms", "partner", "cause"],
                       [[r["track"], f"{r['t']:.3f}", f"{int(r['t'] // 60)}:{r['t'] % 60:06.3f}",
                         "" if r["rel_db"] is None else f"{r['rel_db']:.1f}", f"{r['rise_own']:.1f}",
                         "" if r["layer_ms"] is None else f"{r['layer_ms']:.1f}", r["partner"], r["cause"]]
                        for r in s["merges"]]), ""]
        j = s.get("job_pairs") or {}
        L += [f"Version 2 marks (kick and snare bleed, not pair merges): {s['v2_marks']}. "
              f"Tom hits before the rules: {s['tom_hits']}. The app's own pair record: "
              f"on {j.get('on')}, removed {j.get('removed')}." + (f" ({j.get('reason')})" if j.get("reason") else ""), ""]
    total_real = sum(s["counts"]["real"] for s in b.values())
    total = sum(s["counts"]["all"] for s in b.values())
    total_bleed = sum(s["counts"]["bleed"] for s in b.values())
    total_other = sum(s["counts"]["other"] for s in b.values())
    L += ["## Verdict", ""]
    if total == 0:
        L += ["No pair-rule merge on the toms of the 3 real sessions; nothing lost.", ""]
    elif total_real == 0:
        L += [f"**No real stroke lost.** All {total} pair-rule merges on the toms of the 3 real sessions",
              f"are bleed ({total_bleed}) or other ({total_other}); 0 are real strokes.", ""]
    else:
        steps, chosen, lay = res["guard"]["steps"], res["guard"]["chosen"], res["guard"]["layer"]
        L += [f"**The pair rule loses real strokes**: {total_real} of {total} merges on the toms of the",
              f"3 real sessions are real strokes ({total_bleed} bleed, {total_other} other).", "",
              "The smallest guard that keeps the lost strokes without new false hits on the real sessions",
              "(keep a marked hit when its own-mic rise is X dB or more; measured on the merges, not shipped):", "",
              md_table(["guard", "X dB", "real strokes kept", "bleed merges that would become false hits"],
                       [[k, x, f"{kept} of {total_real}", nf] for k, x, kept, nf in steps]), ""]
        if chosen:
            L += [f"Chosen guard: keep a marked hit when its own-mic rise is **{chosen[1]} dB or more** -",
                  f"it keeps every lost real stroke and unmarks no bleed merge. The layer guard keeps",
                  f"{lay[0]} of {lay[1]} real strokes by the sample layer alone (0 new false hits by the",
                  "cause order). Measured, not shipped.", ""]
        else:
            L += ["No rise threshold keeps every lost real stroke without unmarking a bleed merge;",
                  "the sweep above is the trade-off. The layer guard keeps "
                  f"{lay[0]} of {lay[1]} real strokes (0 new false hits by the cause order).", ""]
    L += ["Notes: the layer evidence covers Session A only (x86-mac holds no layer files for the other",
          "two sessions); the layer latency is fixed against the version 2 run's own events of the same",
          "mic (no reference file on x86-mac), not against the P4-16 references. The false-hit side of a",
          "guard is measured on the merges' causes (no reference scoring on x86-mac).", ""]
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text("\n".join(L))
    log(f"report written: {REPORT}")


# ---------------------------------------------------------------- the driver

def do_sets(log=print):
    shots = {}
    x, sr = le.read_mono([session_file("Session A", "t2_02")])
    shot, _ = extract_shot(x, sr)
    plans = set_plans()
    for p in plans:
        d = OUT / "sets" / p["name"]
        if (d / "planted.json").is_file():
            continue
        build_set(p, shot, sr, d)
        log(f"rendered {p['name']}")
    return plans, sr


def measure_sets(plans, sr, log=print):
    rows, solo_rows = [], []
    for p in plans:
        res = app_run(p["name"], [("t1", str(OUT / "sets" / p["name"] / "t1.wav"), "tom", 47),
                                  ("t2", str(OUT / "sets" / p["name"] / "t2.wav"), "tom", 47)], {}, log)
        strokes = json.loads((OUT / "sets" / p["name"] / "planted.json").read_text())["strokes"]
        bad_keys = {(b[0], round(b[1], 4)) for b in render_check(OUT / "sets" / p["name"], sr, log)}
        fates = stroke_fate(res["events"], [s for s in strokes if s["scored"]])
        counted = [f for f in fates if (f["track"], f["t"]) not in bad_keys]
        planted = len(counted)
        found = sum(1 for f in counted if f["fate"] == "found")
        rule = sum(1 for f in counted if f["fate"] == "rule")
        dsp = sum(1 for f in counted if f["fate"] == "dsp")
        check = "FAIL (0 planted)" if planted == 0 else "OK"
        if p["kind"] == "solo":
            for u in SOLO_UNDER:
                fs = [f for f in counted if f["under_db"] == u]
                solo_rows.append([u, p["tun"], len(fs), sum(1 for f in fs if f["fate"] == "found"),
                                  sum(1 for f in fs if f["fate"] == "dsp")])
        else:
            rows.append([p["name"], p["kind"], p["gap"], p["tun"], planted, found, rule, dsp, check])
        log(f"{p['name']}: planted {planted}, found {found}, lost rule {rule}, dsp {dsp} {check}")
    return {"rows": rows, "solo_rows": solo_rows}


def do_sessions(log=print):
    out = {}
    for song, cfg in SESSIONS.items():
        tracks = [(m, str(session_file(song, stem)), role, {"kick": 36, "snare": 38, "tom": 47}[role])
                  for m, (stem, role) in cfg["mics"].items()]
        v2 = app_run(f"{song}_v2", tracks, {"DRUMSPIKE_BLEED_RULE": "2"}, log)
        v3 = app_run(f"{song}_v3", tracks, {}, log)
        for m in cfg["mics"]:
            if len(v2["events"][m]) != len(v3["events"][m]):
                raise RuntimeError(f"{song} {m}: the two runs disagree on the event count "
                                   f"({len(v2['events'][m])} vs {len(v3['events'][m])})")
        toms = [m for m, (s, role) in cfg["mics"].items() if role == "tom"]
        marks = diff_marks(v2["events"], {m: v3["events"][m] for m in toms})
        rows = session_merges(song, v2, v3)
        merges = [{"track": m, **r} for m, rs in rows.items() for r in rs]
        counts = {"all": len(merges),
                  "real": sum(1 for r in merges if r["cause"].startswith("real")),
                  "bleed": sum(1 for r in merges if r["cause"] == "bleed"),
                  "other": sum(1 for r in merges if r["cause"] == "other")}
        v2_marks = sum(1 for m in toms for e in v2["events"][m] if e.get("bleed"))
        tom_hits = sum(len(v2["events"][m]) for m in toms)
        jp = (v3["job"].get("bleed") or {}).get("pairs") or {}
        out[song] = {"merges": sorted(merges, key=lambda r: (r["track"], r["t"])), "counts": counts,
                     "v2_marks": v2_marks, "tom_hits": tom_hits, "job_pairs": jp}
        log(f"{song}: {counts['all']} merges, {counts['real']} real, {counts['bleed']} bleed, "
            f"{counts['other']} other (app record removed {jp.get('removed')})")
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", nargs="?", default="all", choices=["sets", "sessions", "all"])
    a = ap.parse_args(argv)
    started = time.strftime("%Y-%m-%d %H:%M %Z")
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "run").mkdir(exist_ok=True)
    res = {"when": started, "host": os.uname().nodename}
    try:
        res["sha"] = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=str(ROOT),
                                    capture_output=True, text=True).stdout.strip()
    except Exception:
        res["sha"] = "?"
    plans, sr = do_sets()
    if a.cmd in ("sets", "all"):
        res["sets"] = measure_sets(plans, sr)
    if a.cmd in ("sessions", "all"):
        res["sessions"] = do_sessions()
        real = [r for s in res["sessions"].values() for rs in [s["merges"]] for r in rs
                if r["cause"].startswith("real")]
        if real:
            steps, chosen, lay = guard_sweep(
                {m: [r for s in res["sessions"].values() for r in s["merges"]] for m in ("all",)})
            res["guard"] = {"steps": steps, "chosen": chosen, "layer": lay}
        else:
            res["guard"] = {"steps": [], "chosen": None, "layer": (0, 0)}
    if a.cmd == "all":
        write_report(res)
    (OUT / "result.json").write_text(json.dumps(res, indent=1, default=str))
    return 0


if __name__ == "__main__":
    sys.exit(main())
