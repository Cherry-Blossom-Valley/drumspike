"""P4-17 stage 1 (measure only): tom bleed from the snare and kick close mics.

    uv run --with numpy --with soundfile --with mido python tools/tom_bleed_sk.py causes   # the false tom hits by source
    uv run --with numpy --with soundfile --with mido python tools/tom_bleed_sk.py score    # candidate SK: fit, held out, fixes
    uv run --with numpy --with soundfile --with mido python tools/tom_bleed_sk.py all
    uv run --with numpy --with soundfile --with mido python tools/tom_bleed_sk.py app --run DIR   # stage 2: a new app run

Input: the P4-16 run (`tools/fix_metric.py run`, `~/.cache/drumspike/fix-metric/run/<song>_default.json`: the app's own
events, refinement version 3, rule CD on) and its `refs.json`; the mic files of the seven songs in ~/.cache/drumspike/sessions
(read only). No app run, no build.

`causes`: every false tom hit of the P4-16 run (an output hit that no reference hit owns, P4-16's match and offsets) gets
one source: the nearest reference hit of the snare, the kick or another tom within 30 ms (each reference shifted by its
own mic's offset); else `own` (its own tom's reference within 30 ms: a second detection of one stroke); else `ring` (its
own tom's reference 30 ms to 1 s before: inside the decay); else `unknown`.

Candidate SK: rule CD as built (`tom_bleed_cd.rule_cd`, every P4-14 safeguard: the played test, the shared pairs, the
off cases; the sample tracks are not in the input) plus the snare and kick close mics as sources. At a tom hit of mic A
the snare/kick excess is `L_A - L_S - G(S->A)`, L the tom-band peak in [-0.5, +8) ms, G(S->A) P2-6's transfer gain
measured at S's strong hits where A lists no hit within 30 ms (`tom_bleed_cd.transfer_gain`). A source is used only
with a gain, not when it is silent at a quarter or more of A's strong hits, and not when it forms a shared pair with A
(`G(S->A) + G(A->S)` above -26 dB, fix round 3's cut). A hit is bleed when rule CD marks it or its snare/kick excess
is under the threshold (one threshold for both, or one each). The rule stays off where rule CD is off.

Optional ring guard: a tom hit 30 ms to 1 s after the previous output hit of its own mic whose tom-band level rises
less than `rise` dB over the level in [-6, -1] ms before it (the decay explains it).

Fit on Session A and Session F only (the two songs with independent per-tom references); held out: the other five.
Accepted only when it loses at most 1 % of the real tom hits on each fit song and on the held-out total (a real hit:
an output hit of today's run that a reference owns; lost: such a hit the candidate marks) and lowers the P4-16 fixes.
Outputs: `~/.cache/drumspike/tom-bleed/sk/`.

Stage 2 (the app's rule version 2, `crates/core/src/detect/tom_bleed.rs` `tom_bleed_with`): the snare sources at
SNARE_DB (9.2 dB, as the tom sources), the kick sources at KICK_DB (4.0 dB, the controller's choice); `rule_v2` is the
reference. `app --run DIR`: a `fix_metric.py run` of the new app (DIR holds `run/<song>_default.json`): rule version 2
on its events against the app's marks, and the real tom hits it loses per song (a real hit: a tom output hit with rule
CD's marks, recomputed on the same events, that a reference owns; lost: such a hit the app flags).
"""
import argparse
import json
import os
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fix_metric as fm  # noqa: E402
import tom_bleed as tb  # noqa: E402
import tom_bleed_cd as cd  # noqa: E402

OUT = tb.OUT / "sk"
FM = fm.OUT
FIT = ["Session A", "Session F"]
HELD = ["Session B", "Session C", "Session H", "Session G", "Session D"]
REACH = 0.030          # a source hit within this of a false tom hit owns it (P4-14 stage 1)
RING_S = 1.0           # a false hit up to this long after its own tom's reference hit lies in the decay
SOURCES = ("snare", "kick", "other tom", "own", "ring", "unknown")
MAX_LOST = 0.01
PRE_RING = (-0.006, -0.001)   # the ring guard's level before the hit
LOCK = fm.LOCK
MAX_LOAD = 8.0
SNARE_DB = cd.THRESHOLD_DB   # stage 2: the snare sources at the tom sources' threshold
KICK_DB = 4.0                # stage 2 (controller): a tom played with the kick is a common accent


# ---------------------------------------------------------------- causes (pure)

def classify(t, own, others, reach=REACH, ring_s=RING_S):
    """The source of a false tom hit at `t` (s, the mic's time). `own`: its tom's reference times; `others`:
    [(kind, times)] with kind "snare", "kick" or "other tom" (times already on the mics' time). The nearest other hit
    within `reach` names the source; else `own` within `reach`, `ring` within `ring_s` before, else `unknown`."""
    best = None
    for kind, times in others:
        times = np.asarray(times, float)
        if not len(times):
            continue
        d = float(np.min(np.abs(times - t)))
        if d <= reach and (best is None or d < best[0]):
            best = (d, kind)
    if best:
        return best[1]
    own = np.asarray(own, float)
    if len(own) and float(np.min(np.abs(own - t))) <= reach:
        return "own"
    if len(own) and np.any((t - own > reach) & (t - own <= ring_s)):
        return "ring"
    return "unknown"


def in_decay(t, own, reach=REACH, ring_s=RING_S):
    """True when an own reference hit lies `reach` to `ring_s` before `t`."""
    own = np.asarray(own, float)
    return bool(len(own)) and bool(np.any((t - own > reach) & (t - own <= ring_s)))


def match(ref, est, tol=fm.TOL):
    """`fix_metric.match` (one to one, nearest pair first) with the candidate pairs found by a sorted search."""
    ref, est = np.asarray(ref, float), np.asarray(est, float)
    order = np.argsort(est, kind="stable")
    se = est[order]
    cand = []
    for i, r in enumerate(ref):
        lo, hi = np.searchsorted(se, r - tol - 1e-9), np.searchsorted(se, r + tol + 1e-9, side="right")
        for k in range(lo, hi):
            j = int(order[k])
            if abs(est[j] - r) <= tol:
                cand.append((abs(est[j] - r), i, j))
    cand.sort()
    ur, ud, pairs = set(), set(), []
    for _, i, j in cand:
        if i in ur or j in ud:
            continue
        ur.add(i)
        ud.add(j)
        pairs.append((i, j))
    return sorted(pairs)


def score_track(ref, events, offset, tol=fm.TOL, move_s=fm.MOVE_S):
    """`fix_metric.score_track` with the sorted-search `match`."""
    est = fm.outputs(events)
    shifted = [float(r) + offset for r in ref]
    pairs = match(shifted, est, tol)
    errs = [est[j] - shifted[i] for i, j in pairs]
    moved = sum(1 for e in errs if abs(e) > move_s + 1e-9)
    false, missed = len(est) - len(pairs), len(ref) - len(pairs)
    return {"n_ref": len(ref), "n_out": len(est), "flagged": len(events) - len(est), "matched": len(pairs),
            "false": false, "missed": missed, "moved": moved, "fixes": false + missed + moved, "errs": errs}


def false_hits(ref, events, offset):
    """The output times (`fix_metric.outputs`) that no reference hit owns (P4-16's one-to-one match after `offset`)."""
    est = fm.outputs(events)
    owned = {j for _, j in match([float(r) + offset for r in ref], est)}
    return [t for j, t in enumerate(est) if j not in owned]


def real_hits(ref, events, offset):
    """The output times that a reference hit owns."""
    est = fm.outputs(events)
    return sorted(est[j] for _, j in match([float(r) + offset for r in ref], est))


# ---------------------------------------------------------------- candidate SK (pure on signals)

def source_gains(tom_sig, tom_times, src_sig, src_times, sr, sec=None):
    """For one tom mic A and one source mic S: (G(S->A), G(A->S), silent share of A at S's strong hits); a gain is None
    with fewer than `cd.MIN_HITS` strong hits."""
    sec = sec or tb.tom_sections(sr)
    tom_times, src_times = np.asarray(tom_times, float), np.asarray(src_times, float)
    ss = tb.strong_mask(src_sig, sr, src_times) if len(src_times) else np.zeros(0, bool)
    st = src_times[ss]
    la, ls = tb.band_peaks_db(tom_sig, sr, st, sec), tb.band_peaks_db(src_sig, sr, st, sec)
    quiet = [not cd.near(tom_times, t) for t in st]
    g_sa = cd.transfer_gain(la, ls, quiet)
    silent = float(np.mean(la <= cd.SILENCE_DB)) if len(la) >= cd.MIN_HITS else None
    ts = tb.strong_mask(tom_sig, sr, tom_times) if len(tom_times) else np.zeros(0, bool)
    tt = tom_times[ts]
    la2, ls2 = tb.band_peaks_db(tom_sig, sr, tt, sec), tb.band_peaks_db(src_sig, sr, tt, sec)
    g_as = cd.transfer_gain(ls2, la2, [not cd.near(src_times, t) for t in tt])
    return g_sa, g_as, silent


def sk_excess(tom_sig, tom_times, sources, sr, shared_cut=cd.SHARED_SUM_DB):
    """{kind: excess array at the tom hits} over the usable sources {kind: (signal, times)}, plus {kind: note}: the gain
    pair and why a source is not used (no gain, silent, shared)."""
    sec = tb.tom_sections(sr)
    tom_times = np.asarray(tom_times, float)
    la = tb.band_peaks_db(tom_sig, sr, tom_times, sec)
    out, notes = {}, {}
    for kind, (sig, times) in sources.items():
        g_sa, g_as, silent = source_gains(tom_sig, tom_times, sig, times, sr, sec)
        note = {"g_sa": g_sa, "g_as": g_as, "silent": silent, "used": False}
        if g_sa is None:
            note["why"] = "no gain"
        elif silent is not None and silent >= cd.SILENT_SHARE:
            note["why"] = "silent"
        elif g_as is not None and g_sa + g_as > shared_cut:
            note["why"] = "shared"
        else:
            note["used"] = True
            out[kind] = la - tb.band_peaks_db(sig, sr, tom_times, sec) - g_sa
        notes[kind] = note
    return out, notes


def ring_rise(sig, times, sr, pre=PRE_RING):
    """Per hit: the tom-band peak in [-0.5, +8) ms minus the tom-band peak in `pre` (the decay just before the hit)."""
    sec = tb.tom_sections(sr)
    times = np.asarray(times, float)
    return tb.band_peaks_db(sig, sr, times, sec) - tb.band_peaks_db(sig, sr, times, sec, window=pre)


def ring_mask(times, keep, rise, rise_db, reach=REACH, ring_s=RING_S):
    """The ring guard: a hit whose previous kept hit (`keep`) of its own mic lies `reach` to `ring_s` before it and whose
    `rise` is under `rise_db`."""
    times = np.asarray(times, float)
    out = np.zeros(len(times), bool)
    last = None
    for j in np.argsort(times, kind="stable"):
        t = times[j]
        if last is not None and reach < t - last <= ring_s and rise[j] < rise_db:
            out[j] = True
        if keep[j] and not out[j]:
            last = t
    return out


def rule_sk(cd_result, exc, th_snare, th_kick=None, ring=None, rise_db=None):
    """The candidate's marks per tom mic: rule CD's marks (`cd_result["bleed"]`), or a snare/kick excess under its
    threshold (`th_kick` None: the snare threshold), or the ring guard (`ring`: {mic: (times, rise)}). Nothing when
    rule CD is off."""
    th_kick = th_snare if th_kick is None else th_kick
    out = {}
    for m, b in cd_result["bleed"].items():
        b = np.asarray(b, bool).copy()
        if cd_result["off"] is None:
            e = exc.get(m, {})
            if th_snare is not None and "snare" in e:
                b |= e["snare"] < th_snare
            if th_kick is not None and "kick" in e:
                b |= e["kick"] < th_kick
            if ring is not None and rise_db is not None:
                times, rise = ring[m]
                b |= ring_mask(times, ~b, rise, rise_db)
        out[m] = b
    return out


def rule_v2(signals, times, sources, sr, drums=None, snare_db=SNARE_DB, kick_db=KICK_DB):
    """The app's rule version 2 on one session: rule CD on the tom mics (`signals`, `times` in s, {mic: ...}) plus the
    sources {"snare" | "kick": (signal, times)}. Returns (rule CD's result, {mic: {kind: excess}}, {mic: notes},
    {mic: marks})."""
    r = cd.rule_cd(signals, {m: np.asarray(t, float) for m, t in times.items()}, sr, drums=drums)
    exc, notes = {}, {}
    for m in signals:
        exc[m], notes[m] = sk_excess(signals[m], times[m], sources, sr)
    marks = rule_sk(r, exc, snare_db, kick_db)
    return r, exc, notes, marks


# ---------------------------------------------------------------- the seven songs

def load_song(song, refs_all, table, signals=True):
    """Signals (when `signals`), events, references and offsets of one song of the P4-16 run."""
    run = json.loads((FM / "run" / f"{fm.track_name(song)}_default.json").read_text())
    cfg = table[song]
    sig, sr = {}, None
    for m, (stem, _role) in cfg["mics"].items():
        if signals:
            sig[m], sr = tb.read_mono(fm.audio_path(cfg, stem))
    refs = refs_all[song]["mics"]
    off = {m: fm.track_offset(refs[m]["onsets"], run["events"][m]) for m in refs}
    return {"sig": sig, "sr": sr, "events": run["events"], "job": run["job"], "refs": refs, "off": off,
            "roles": {m: v[1] for m, v in cfg["mics"].items()}, "minutes": refs_all[song]["duration_s"] / 60.0}


def causes_of(song, d):
    """[(mic, t, source, in decay)] of the false tom hits of one song."""
    refs, off = d["refs"], d["off"]
    shifted = {m: [t + off[m] for t in refs[m]["onsets"]] for m in refs}
    rows = []
    for m, v in refs.items():
        if v["role"] != "toms":
            continue
        others = []
        for o, w in refs.items():
            if o == m:
                continue
            kind = {"snare": "snare", "kick": "kick"}.get(w["role"], "other tom")
            others.append((kind, shifted[o]))
        for t in false_hits(v["onsets"], d["events"][m], off[m]):
            rows.append((m, t, classify(t, shifted[m], others), in_decay(t, shifted[m])))
    return rows


def wait_for_room(log=print):
    while LOCK.exists() or os.getloadavg()[0] >= MAX_LOAD:
        log(f"waiting: lock {LOCK.exists()}, load {os.getloadavg()[0]:.1f}")
        time.sleep(30)


def measure(out, log=print):
    """Per song: rule CD on the run's tom events (it must equal the app's marks), the snare/kick excess and the ring
    rise at every tom hit, the false-hit causes. Written to `<out>/measures.json`."""
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    refs_all = json.loads((FM / "refs.json").read_text())
    table = fm.songs()
    res = {}
    wait_for_room(log)
    LOCK.parent.mkdir(parents=True, exist_ok=True)
    LOCK.write_text("tom_bleed_sk measure\n")
    try:
        for song in FIT + HELD:
            d = load_song(song, refs_all, table)
            toms = [m for m, r in d["roles"].items() if r == "toms"]
            src = {r: m for m, r in d["roles"].items() if r in ("snare", "kick")}
            times = {m: np.asarray([e["t"] for e in d["events"][m]], float) for m in toms}
            r = cd.rule_cd({m: d["sig"][m] for m in toms}, times, d["sr"])
            app = {m: [bool(e.get("bleed")) for e in d["events"][m]] for m in toms}
            same = sum(int(a == b) for m in toms for a, b in zip(app[m], r["bleed"][m].tolist()))
            sources = {k: (d["sig"][m], np.asarray([e["t"] for e in d["events"][m]], float)) for k, m in src.items()}
            exc, notes, rise = {}, {}, {}
            for m in toms:
                exc[m], notes[m] = sk_excess(d["sig"][m], times[m], sources, d["sr"])
                rise[m] = ring_rise(d["sig"][m], times[m], d["sr"])
            res[song] = {"toms": toms, "same": [same, sum(len(times[m]) for m in toms)],
                         "off": r["off"], "cd": {m: r["bleed"][m].tolist() for m in toms},
                         "exc": {m: {k: v.tolist() for k, v in exc[m].items()} for m in toms}, "notes": notes,
                         "rise": {m: rise[m].tolist() for m in toms},
                         "causes": causes_of(song, d)}
            log(f"{song}: rule CD = app marks {same} of {res[song]['same'][1]}, off {r['off']}")
    finally:
        LOCK.unlink(missing_ok=True)
    (out / "measures.json").write_text(json.dumps(res))
    return res


# ---------------------------------------------------------------- scoring

def marked_events(events, marks):
    """The events with `bleed` set from `marks` (a bool list per event)."""
    return [dict({k: v for k, v in e.items() if k != "bleed"}, **({"bleed": True} if b else {}))
            for e, b in zip(events, marks)]


def song_scores(d, marks):
    """P4-16's score of one song (`fix_metric.score_run`) with the tom marks `marks` ({mic: bools}); the other mics as in
    the run. Returns (scores by role with "mics", the events by mic)."""
    ev, mics = {}, {}
    for m, v in d["refs"].items():
        ev[m] = marked_events(d["events"][m], marks[m]) if m in marks else d["events"][m]
        if m not in marks and m in d.get("base", {}):
            mics[m] = d["base"][m]
        else:
            mics[m] = score_track(v["onsets"], ev[m], d["off"][m])
    roles = {role: fm.add_up([mics[m] for m, v in d["refs"].items() if v["role"] == role], d["minutes"])
             for role in fm.ROLES}
    roles["mics"] = mics
    return roles, ev


def lost_real(d, ev_after):
    """(lost, real): the tom output hits a reference owns in the run that the candidate flags."""
    lost = real = 0
    for m, v in d["refs"].items():
        if v["role"] != "toms":
            continue
        before = d["real"][m] if "real" in d else real_hits(v["onsets"], d["events"][m], d["off"][m])
        after = set(fm.outputs(ev_after[m]))
        real += len(before)
        lost += sum(1 for t in before if t not in after)
    return lost, real


def evaluate(data, meas, th_snare, th_kick=None, rise_db=None, songs=None):
    """{song: {"scores", "lost", "real", "marked"}} of one candidate setting."""
    res = {}
    for song in songs or data:
        d, ms = data[song], meas[song]
        cdr = {"bleed": {m: np.asarray(ms["cd"][m], bool) for m in ms["toms"]}, "off": ms["off"]}
        exc = {m: {k: np.asarray(v) for k, v in ms["exc"][m].items()} for m in ms["toms"]}
        ring = {m: (np.asarray([e["t"] for e in d["events"][m]]), np.asarray(ms["rise"][m])) for m in ms["toms"]}
        marks = rule_sk(cdr, exc, th_snare, th_kick, ring, rise_db)
        sc, ev = song_scores(d, {m: marks[m].tolist() for m in ms["toms"]})
        lost, real = lost_real(d, ev)
        res[song] = {"scores": sc, "lost": lost, "real": real,
                     "marked": int(sum(marks[m].sum() for m in ms["toms"])), "marks": marks}
    return res


def second_strokes(d, newly, lo=0.010, hi=0.050):
    """The newly marked hits (`newly`: {mic: times}) that no reference owns and that lie `lo` to `hi` after a tom
    reference hit of the song (each reference on its own mic's time): possible second strokes a reference merged."""
    toms = [np.asarray(v["onsets"], float) + d["off"][m] for m, v in d["refs"].items() if v["role"] == "toms"]
    allt = np.concatenate(toms) if toms else np.zeros(0)
    n = 0
    for m, times in newly.items():
        if m not in d["refs"]:
            continue
        real = set(d["real"][m]) if "real" in d else set()
        for t in times:
            if t in real:
                continue
            if np.any((t - allt >= lo) & (t - allt <= hi)):
                n += 1
    return n


def accepted(res, cap=MAX_LOST):
    fit_ok = all(res[s]["lost"] <= cap * res[s]["real"] for s in FIT if s in res)
    held = [s for s in HELD if s in res]
    held_ok = sum(res[s]["lost"] for s in held) <= cap * sum(res[s]["real"] for s in held)
    return fit_ok and held_ok


def tom_fixes(res, songs):
    return sum(res[s]["scores"]["toms"]["fixes"] for s in songs)


def fit(data, meas, grid, cap=MAX_LOST, two=False, rise_grid=(None,)):
    """The setting with the fewest tom fixes on FIT and at most `cap` lost on each FIT song; ties: the lower threshold.
    `two`: separate snare and kick thresholds. Returns (th_snare, th_kick, rise_db) or None."""
    best = None
    pairs = [(a, b) for a in grid for b in grid] if two else [(a, None) for a in grid]
    for a, b in pairs:
        for rz in rise_grid:
            r = evaluate(data, meas, a, b, rz, FIT)
            if any(r[s]["lost"] > cap * r[s]["real"] for s in FIT):
                continue
            key = (tom_fixes(r, FIT), -(a if a is not None else -99), -(b if b is not None else -99))
            if best is None or key < best[0]:
                best = (key, (a, b, rz))
    return best[1] if best else None


# ---------------------------------------------------------------- report

def md(head, body):
    return fm.md_table(head, body)


def do_causes(meas, data, log=print):
    lines = ["### The false tom hits of the P4-16 run by source", "",
             "Source: the nearest reference hit of the snare, the kick or another tom within 30 ms; else `own` (its own "
             "tom's reference within 30 ms, a second detection), `ring` (own reference 30 ms to 1 s before), `unknown`. "
             "In brackets: of them inside the own tom's decay (own reference 30 ms to 1 s before).", ""]
    body, tot = [], {k: [0, 0] for k in SOURCES}
    for song in FIT + HELD:
        c = meas[song]["causes"]
        cnt = {k: [sum(1 for x in c if x[2] == k), sum(1 for x in c if x[2] == k and x[3])] for k in SOURCES}
        for k in SOURCES:
            tot[k][0] += cnt[k][0]
            tot[k][1] += cnt[k][1]
        body.append([song, len(c)] + [f"{cnt[k][0]} ({cnt[k][1]})" for k in SOURCES]
                    + ["on" if meas[song]["off"] is None else f"off: {meas[song]['off'][0]}"])
    n = sum(v[0] for v in tot.values())
    body.append(["**all**", n] + [f"{tot[k][0]} ({tot[k][1]})" for k in SOURCES] + [""])
    lines += [md(["song", "false tom hits"] + list(SOURCES) + ["rule CD"], body), ""]
    log("\n".join(lines))
    return lines, tot


def fixes_cells(res, songs, role):
    r = [res[s]["scores"][role] for s in songs]
    return (sum(x["false"] for x in r), sum(x["missed"] for x in r), sum(x["moved"] for x in r),
            sum(x["fixes"] for x in r))


def do_score(out, log=print):
    out = Path(out)
    meas = json.loads((out / "measures.json").read_text())
    refs_all = json.loads((FM / "refs.json").read_text())
    table = fm.songs()
    data = {s: load_song(s, refs_all, table, signals=False) for s in FIT + HELD}
    for s, d in data.items():
        d["sig"] = None     # the scores need no signal
        d["base"] = {m: score_track(v["onsets"], d["events"][m], d["off"][m]) for m, v in d["refs"].items()
                     if v["role"] != "toms"}
        d["real"] = {m: real_hits(v["onsets"], d["events"][m], d["off"][m]) for m, v in d["refs"].items()
                     if v["role"] == "toms"}
    lines, tot = do_causes(meas, data, log)
    lines += ["### Rule CD on the run's events against the app's marks", "",
              md(["song", "same marks", "rule CD"],
                 [[s, f"{meas[s]['same'][0]} of {meas[s]['same'][1]}",
                   "on" if meas[s]["off"] is None else f"off: {meas[s]['off'][0]}"] for s in FIT + HELD]), ""]
    notes = []
    for s in FIT + HELD:
        for m in meas[s]["toms"]:
            for k, v in meas[s]["notes"][m].items():
                g = lambda x: "-" if x is None else f"{x:.1f}"  # noqa: E731
                notes.append([s, m, k, g(v["g_sa"]), g(v["g_as"]),
                              "-" if v["silent"] is None else f"{100 * v['silent']:.0f} %",
                              "yes" if v["used"] else f"no: {v['why']}"])
    lines += ["### Snare and kick as sources: transfer gains (dB) in the tom band", "",
              md(["song", "tom mic", "source", "G(S->A)", "G(A->S)", "A silent at S's strong hits", "used"], notes), ""]

    base = evaluate(data, meas, None, None, None)
    grid = [round(x, 1) for x in np.arange(-6.0, 20.01, 0.5)]
    variants = {"today (rule CD)": (None, None, None)}
    variants["SK, same 9.2 dB"] = (cd.THRESHOLD_DB, None, None)
    variants[f"SK stage 2: snare {SNARE_DB} dB, kick {KICK_DB} dB"] = (SNARE_DB, KICK_DB, None)
    one = fit(data, meas, grid)
    if one:
        variants[f"SK, one threshold fitted ({one[0]} dB)"] = one
    two = fit(data, meas, grid, two=True)
    if two:
        variants[f"SK, snare {two[0]} dB, kick {two[1]} dB (fitted)"] = two
    ring_grid = [None] + [round(x, 1) for x in np.arange(0.0, 12.01, 1.0)]
    ringv = fit(data, meas, [None], rise_grid=ring_grid)
    if ringv and ringv[2] is not None:
        variants[f"ring guard alone (rise < {ringv[2]} dB, fitted)"] = ringv
    if one:
        both = fit(data, meas, [one[0]], rise_grid=ring_grid)
        if both and both[2] is not None:
            variants[f"SK {both[0]} dB + ring guard (rise < {both[2]} dB)"] = both
    results = {k: evaluate(data, meas, *v) for k, v in variants.items()}

    head = ["variant", "lost: Session A", "lost: Session F", "lost: held out", "tom false / missed / moved = fixes",
            "fit songs tom fixes", "held-out tom fixes", "all roles fixes", "marked", "accepted"]
    body = []
    for k, r in results.items():
        held_l = sum(r[s]["lost"] for s in HELD), sum(r[s]["real"] for s in HELD)
        f, mi, mo, fx = fixes_cells(r, FIT + HELD, "toms")
        allr = sum(fixes_cells(r, FIT + HELD, x)[3] for x in fm.ROLES)
        ok = "-" if k.startswith("today") else ("yes" if accepted(r) and fx < fixes_cells(base, FIT + HELD, "toms")[3]
                                                 else "no")
        body.append([k] + [f"{r[s]['lost']} of {r[s]['real']}" for s in FIT] + [f"{held_l[0]} of {held_l[1]}",
                     f"{f} / {mi} / {mo} = **{fx}**", tom_fixes(r, FIT), tom_fixes(r, HELD), f"**{allr}**",
                     sum(r[s]["marked"] for s in FIT + HELD), ok])
    lines += ["### Candidates (fit: Session A, Session F; lost = real tom hits of today's run the candidate flags)",
              "", md(head, body), ""]

    sw = []
    for th in [x for x in grid if -2.0 <= x <= 16.0 and float(x).is_integer()]:
        r = evaluate(data, meas, th)
        held_l = sum(r[s]["lost"] for s in HELD), sum(r[s]["real"] for s in HELD)
        sw.append([f"{th:.1f}"] + [f"{r[s]['lost']} of {r[s]['real']}" for s in FIT]
                  + [f"{held_l[0]} of {held_l[1]}", fixes_cells(r, FIT + HELD, "toms")[3],
                     sum(fixes_cells(r, FIT + HELD, x)[3] for x in fm.ROLES)])
    lines += ["### Threshold sweep (one threshold for snare and kick sources)", "",
              md(["threshold dB", "lost: Session A", "lost: Session F", "lost: held out", "tom fixes",
                  "all roles fixes"], sw), ""]

    lines += detail(data, meas, results["SK, same 9.2 dB"], results["today (rule CD)"], ring_grid)
    per = []
    for k, r in results.items():
        for s in FIT + HELD:
            f, mi, mo, fx = fixes_cells(r, [s], "toms")
            per.append([k, s, f"{f} / {mi} / {mo} = {fx}", r[s]["lost"]])
    lines += ["### Per song (toms: false / missed / moved = fixes; lost)", "",
              md(["variant", "song", "toms", "lost"], per), ""]
    (out / "report.md").write_text("\n".join(lines))
    (out / "scores.json").write_text(json.dumps(
        {"variants": {k: list(v) for k, v in variants.items()}, "causes": tot,
         "results": {k: {s: {"lost": r[s]["lost"], "real": r[s]["real"], "marked": r[s]["marked"],
                             "fixes": {x: {y: r[s]["scores"][x][y] for y in ("false", "missed", "moved", "fixes")}
                                       for x in fm.ROLES}} for s in r} for k, r in results.items()}}))
    log("\n".join(lines))
    return results


def detail(data, meas, r, base, ring_grid):
    """The recommended setting's tables: false hits removed and left by cause, the lost real hits, the possible second
    strokes; the 2D sweep around it; the ring guard sweep on top of it."""
    lines = []
    body, tot = [], {k: [0, 0] for k in SOURCES}
    second = 0
    lost_rows = []
    for s in FIT + HELD:
        d, ms = data[s], meas[s]
        marked = {m: {e["t"] for e, b in zip(d["events"][m], r[s]["marks"][m]) if b} for m in ms["toms"]}
        newly = {m: [e["t"] for e, b, c in zip(d["events"][m], r[s]["marks"][m], ms["cd"][m]) if b and not c]
                 for m in ms["toms"]}
        second += second_strokes(d, newly)
        for m, t, src, _dec in ms["causes"]:
            tot[src][0 if t in marked[m] else 1] += 1
        shifted = {m: np.asarray(v["onsets"], float) + d["off"][m] for m, v in d["refs"].items()}
        for m in ms["toms"]:
            if m not in d["refs"]:
                continue
            for t in d["real"][m]:
                if t in marked[m]:
                    j = [e["t"] for e in d["events"][m]].index(t)
                    near = sorted(((float(np.min(np.abs(v - t))) if len(v) else 9.0, o) for o, v in shifted.items()
                                   if o != m))[0]
                    e = ms["exc"][m]
                    lost_rows.append([s, m, f"{t:.3f}", f"{e['snare'][j]:.1f}" if "snare" in e else "-",
                                      f"{e['kick'][j]:.1f}" if "kick" in e else "-",
                                      f"{near[1]} {1000 * near[0]:.1f} ms"])
    lines += ["### SK at 9.2 dB: the false tom hits removed and left, by source", "",
              md(["", *SOURCES, "all"], [["removed", *(tot[k][0] for k in SOURCES), sum(v[0] for v in tot.values())],
                                         ["left", *(tot[k][1] for k in SOURCES), sum(v[1] for v in tot.values())]]),
              "",
              f"Newly marked hits that no reference owns and that lie 10 to 50 ms after a tom reference hit (possible "
              f"second strokes, P4-13): {second}.", "",
              "The real tom hits SK at 9.2 dB loses (excess in dB; the nearest other reference hit):", "",
              md(["song", "mic", "t (s)", "snare excess", "kick excess", "nearest other reference"], lost_rows), ""]
    sw = []
    for ts in (7.7, 9.2, 10.7):
        for tk in (3.2, 4.7, 6.2, 7.7, 9.2, 10.7):
            x = evaluate(data, meas, ts, tk)
            sw.append([ts, tk] + [f"{x[q]['lost']} of {x[q]['real']}" for q in FIT]
                      + [f"{sum(x[q]['lost'] for q in HELD)} of {sum(x[q]['real'] for q in HELD)}",
                         fixes_cells(x, FIT + HELD, "toms")[3], "yes" if accepted(x) else "no"])
    lines += ["### Separate thresholds for the snare and the kick (not fitted; real hits lost, tom fixes)", "",
              md(["snare dB", "kick dB", "lost: Session A", "lost: Session F", "lost: held out", "tom fixes",
                  "within 1 %"], sw), ""]
    sw = []
    for rz in ring_grid:
        x = evaluate(data, meas, cd.THRESHOLD_DB, None, rz)
        sw.append(["off" if rz is None else f"rise < {rz:.0f} dB"] + [f"{x[q]['lost']} of {x[q]['real']}" for q in FIT]
                  + [f"{sum(x[q]['lost'] for q in HELD)} of {sum(x[q]['real'] for q in HELD)}",
                     fixes_cells(x, FIT + HELD, "toms")[3], sum(x[q]["marked"] for q in FIT + HELD)])
    lines += ["### Ring guard on top of SK at 9.2 dB (a hit 30 ms to 1 s after its mic's last kept hit, rising less "
              "than the threshold over [-6, -1] ms)", "",
              md(["ring guard", "lost: Session A", "lost: Session F", "lost: held out", "tom fixes", "marked"], sw),
              ""]
    return lines


def do_app(run_dir, out, log=print):
    """Stage 2: rule version 2 (`rule_v2`) on the events of a new app run against the app's marks, and the real tom
    hits the app loses per song against rule CD on the same events. Written to `<out>/app.md` and `<out>/app.json`."""
    run_dir, out = Path(run_dir), Path(out)
    out.mkdir(parents=True, exist_ok=True)
    refs_all = json.loads((FM / "refs.json").read_text())
    table = fm.songs()
    rows, res = [], {}
    for song in FIT + HELD:
        run = json.loads((run_dir / "run" / f"{fm.track_name(song)}_default.json").read_text())
        cfg = table[song]
        roles = {m: v[1] for m, v in cfg["mics"].items()}
        sig, sr = {}, None
        for m, (stem, _role) in cfg["mics"].items():
            sig[m], sr = tb.read_mono(fm.audio_path(cfg, stem))
        ev = run["events"]
        toms = [m for m, r in roles.items() if r == "toms"]
        times = {m: np.asarray([e["t"] for e in ev[m]], float) for m in toms}
        sources = {r: (sig[m], np.asarray([e["t"] for e in ev[m]], float)) for m, r in roles.items()
                   if r in ("snare", "kick")}
        r, _exc, notes, marks = rule_v2({m: sig[m] for m in toms}, times, sources, sr)
        app = {m: [bool(e.get("bleed")) for e in ev[m]] for m in toms}
        same = sum(int(a == b) for m in toms for a, b in zip(app[m], marks[m].tolist()))
        n = sum(len(times[m]) for m in toms)
        refs = refs_all[song]["mics"]
        lost = real = 0
        for m in toms:
            if m not in refs:
                continue
            off = fm.track_offset(refs[m]["onsets"], ev[m])
            v1 = marked_events(ev[m], r["bleed"][m].tolist())
            before = real_hits(refs[m]["onsets"], v1, off)
            after = set(fm.outputs(ev[m]))
            real += len(before)
            lost += sum(1 for t in before if t not in after)
        used = sorted({k for m in toms for k, v in notes[m].items() if v["used"]})
        res[song] = {"same": [same, n], "lost": lost, "real": real, "off": r["off"], "used": used,
                     "v1": int(sum(r["bleed"][m].sum() for m in toms)), "v2": int(sum(sum(app[m]) for m in toms))}
        rows.append([song, f"{same} of {n}", "on" if r["off"] is None else f"off: {r['off'][0]}",
                     ", ".join(used) or "-", res[song]["v1"], res[song]["v2"], f"{lost} of {real}"])
        log(f"{song}: app = rule_v2 {same} of {n}, lost {lost} of {real}")
    held = sum(res[s]["lost"] for s in HELD), sum(res[s]["real"] for s in HELD)
    ok = all(res[s]["lost"] <= MAX_LOST * res[s]["real"] for s in FIT) and held[0] <= MAX_LOST * held[1]
    lines = [f"### Rule version 2 in the app (snare {SNARE_DB} dB, kick {KICK_DB} dB) on `{run_dir}`", "",
             md(["song", "app marks = rule_v2", "rule CD", "sources used", "marked v1", "marked v2",
                 "real tom hits lost"], rows), "",
             f"Held out: {held[0]} of {held[1]} lost. Within 1 % on each fit song and the held-out total: "
             f"{'yes' if ok else 'no'}.", ""]
    (out / "app.md").write_text("\n".join(lines))
    (out / "app.json").write_text(json.dumps(res))
    log("\n".join(lines))
    return res


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["causes", "score", "all", "app"])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--run", default=None, help="app: the folder of a `fix_metric.py run` of the new app")
    a = ap.parse_args(argv)
    if a.cmd == "app":
        do_app(a.run or FM, a.out)
        return 0
    if a.cmd in ("causes", "all") or not (Path(a.out) / "measures.json").is_file():
        measure(a.out)
    if a.cmd in ("causes", "score", "all"):
        do_score(a.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
