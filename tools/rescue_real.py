"""P4-10 stage 2a: do the two rescues of stage 1 hold on real material? A dry run, no change under crates/.

    uv run python tools/rescue_real.py run      # the app chain on the kick, snare and tom mics of the three real sessions
    uv run python tools/rescue_real.py score    # found, false, P, R, F per session and track with and without each rescue
    uv run python tools/rescue_real.py review   # the rescued hits that the references lack: level, bleed test, features
    uv run python tools/rescue_real.py all

The two rescues (`tools/miss_causes.py`, `VARIANTS`):
  cross  a peak of another class counts for the track's role when the role's own column is at least 0.05 within +-3
         frames of it (the role filter; snare and tom tracks)
  dsp    the DSP fallback's events that lie 30 ms or more from every model event (kick tracks)

Ground truth: the P4-6b stage A references (`~/.cache/drumspike/learn-eval/refs.json`): the onsets of the printed trigger
and sample tracks of each session (`tools/learn_eval.py` of branch `feat/desktop-app`, function `ref_onsets`), one global
offset per mic (median of the matched differences within 50 ms), matched one to one within +-20 ms. The owner spot-checked
kick and snare 120 of 120; Session A t4 is excluded (3 ok, 9 unsure, 8 wrong). What the references cannot say: a rescued
hit that no reference lists may be a real ghost note or tom hit that the trigger track lacks, or bleed. `review` therefore
lists every such hit with its level against the track's median hit level and runs a bleed test on a second mic.
Inputs are read only; outputs are under `~/.cache/drumspike/detect-gaps/real/`.
"""
import argparse
import json
import math
import os
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
import soundfile as sf

sys.path.insert(0, str(Path(__file__).resolve().parent))
import calib_kit as ck  # noqa: E402
import miss_causes as mc  # noqa: E402

SAMPLES = Path.home() / ".cache" / "drumspike" / "samples"
REFS = Path.home() / ".cache" / "drumspike" / "learn-eval" / "refs.json"
OUT = mc.OUT / "real"
TOL = 0.020
OFFSET_WINDOW = 0.050
CROSS_MIN = 0.05
DSP_FAR = 0.030

SESSIONS = {
    "Session A": {"dir": "Session A", "other": {"snare": "hh_03", "tom": "s top_02", "kick": "s top_02"},
                "mics": {"k in": ("k in_02", "kick"), "s top": ("s top_02", "snare"), "t1": ("t1 hh_02", "tom"),
                         "t2": ("t2_02", "tom"), "t3": ("t3_02", "tom"), "t4": ("t4 ri_02", "tom")},
                "excluded": {"t4"}},
    "Session B": {"dir": "Session B",
                           "other": {"snare": "hh_01", "tom": "s top_01", "kick": "s top_01"},
                           "mics": {"k in": ("k in_01", "kick"), "s top": ("s top_01", "snare"),
                                    "t1": ("t1 HH_01", "tom"), "t2": ("t2_01", "tom"), "t3": ("t3_01", "tom"),
                                    "t4": ("t4 Ri_01", "tom")}, "excluded": set()},
    "Session C": {"dir": "Session C",
                       "other": {"snare": "oh_03.L", "tom": "s top_02", "kick": "s top_02"},
                       "mics": {"k in": ("k in_02", "kick"), "s top": ("s top_02", "snare"), "t1": ("t 1_03", "tom"),
                                "t2": ("t 2_03", "tom"), "t3": ("t 3_03", "tom"), "t4": ("t 4_03", "tom")},
                       "excluded": set()},
}
CROSS_ROLES = ("snare", "tom")
DSP_ROLES = ("kick",)
VARIANTS = ["base", "cross", "dsp", "both"]


def audio_path(session, stem):
    return SAMPLES / SESSIONS[session]["dir"] / "Audio Files" / f"{stem}.wav"


def safe(session, mic):
    return f"{session}_{mic}".replace(" ", "_")


def tracks():
    """[(session, mic, role, id)] of every kick, snare and tom mic."""
    return [(s, m, role, safe(s, m)) for s, cfg in SESSIONS.items() for m, (_, role) in cfg["mics"].items()]


# ---------------------------------------------------------------- run

def run_all(out, jobs=3, log=print):
    out = Path(out)
    raise SystemExit("#281 D16: this comparison moved to the feature branch; it does not run on main")
    tool, model = mc.find_tool(), ck.find_model()
    if not tool.is_file() or not model.is_file():
        raise SystemExit(f"need service_tool ({tool}) and the model ({model})")
    (out / "audio").mkdir(parents=True, exist_ok=True)
    work = []
    for s, m, role, tid in tracks():
        src = audio_path(s, SESSIONS[s]["mics"][m][0])
        link = out / "audio" / f"{tid}.wav"
        if link.is_symlink() or link.exists():
            link.unlink()
        link.symlink_to(src)
        item = {"id": tid, "audio": str(link), "role": role, "note": mc.ROLE_NOTE[role]}
        work += [(item, "model"), (item, "dsp")]
    with ThreadPoolExecutor(max_workers=jobs) as ex:
        futs = [(i, d, ex.submit(mc.run_item, tool, out, i, d, model)) for i, d in work]
        for i, d, f in futs:
            f.result()
            log(f"ran {i['id']} {d}")
    return len(work)


def load_track(out, tid):
    wd = Path(out) / "work" / tid
    act = mc.read_activation(wd / "model" / "act.bin")
    ev = sorted(e["t"] for e in json.loads((wd / "model" / "events.json").read_text())["events"])
    dsp = sorted(e["t"] for e in json.loads((wd / "dsp" / "events.json").read_text())["events"])
    return act, ev, dsp


# ---------------------------------------------------------------- the rescues on a track

def own_lead(role, act, events):
    """Median of event minus its claimed pick over the base events (the proxy time of a rescued pick)."""
    _, st, _, _ = mc.effective_picks(act, role)
    kept = sorted(f for c in mc.ROLE_CLASSES[role] for f in st[c]["kept"])
    leads = [e - f / mc.FPS for e in events for f in [mc.claim(kept, e)] if f is not None]
    return float(np.median(leads)) if leads else mc.LEAD_S


def cross_rescued(role, act, lead, src=None):
    """The proxy event times of the picks that the cross-class rule adds to the track's own picks."""
    picks, _, _, _ = mc.effective_picks(act, role)
    base, _ = mc.role_dedup(picks, role)
    own = mc.ROLE_CLASSES[role][0]
    conv = sorted({(f, c if c in mc.ROLE_CLASSES[role] else own) for f, c in picks
                   if c in mc.ROLE_CLASSES[role]
                   or ((src is None or c in src) and float(act[max(0, f - 3):f + 4, own].max()) >= CROSS_MIN)})
    new, _ = mc.role_dedup(conv, role)
    return [t + lead for t in new if all(abs(t - b) > 1e-9 for b in base)]


def variant_sets(role, act, events, dsp, src=None):
    """{variant: (events, rescued times)}: the app's events and what each rescue adds. `src`: the classes whose peaks
    the cross-class rule may take over (None: every other class)."""
    lead = own_lead(role, act, events)
    cross = cross_rescued(role, act, lead, src) if role in CROSS_ROLES else []
    # a cross hit that lies within 20 ms of an event is the same hit seen again
    cross = [t for t in cross if all(abs(t - e) > TOL for e in events)]

    def dsp_add(existing):
        return [d for d in dsp if all(abs(d - e) >= DSP_FAR for e in existing)] if role in DSP_ROLES else []

    d_only = dsp_add(events)
    both_base = sorted(events + cross)
    d_both = dsp_add(both_base)
    return {"base": (list(events), []), "cross": (sorted(events + cross), cross),
            "dsp": (sorted(events + d_only), d_only), "both": (sorted(both_base + d_both), cross + d_both)}


# ---------------------------------------------------------------- scoring

def prf(ref, est, tol=TOL):
    pairs, _, _ = ck.match(list(ref), list(est), tol)
    m = len(pairs)
    p = m / len(est) if len(est) else (1.0 if not len(ref) else 0.0)
    r = m / len(ref) if len(ref) else 1.0
    f = 2 * p * r / (p + r) if p + r else 0.0
    return {"tp": m, "fp": len(est) - m, "fn": len(ref) - m, "p": p, "r": r, "f": f, "n_est": len(est), "n_ref": len(ref)}


def offset_of(ref, est, window=OFFSET_WINDOW):
    est = np.sort(np.asarray(est, float))
    d = []
    for t in ref:
        i = np.searchsorted(est, t)
        c = [est[j] - t for j in (i - 1, i) if 0 <= j < len(est) and abs(est[j] - t) <= window]
        if c:
            d.append(min(c, key=abs))
    return float(np.median(d)) if d else 0.0


def score_track(ref, events, sets):
    off = offset_of(ref, events)
    shifted = np.asarray(ref) + off
    return off, {v: prf(shifted, ev) for v, (ev, _) in sets.items()}


def do_score(out, log=print):
    out = Path(out)
    refs = json.loads(REFS.read_text())
    rows = []
    for s, m, role, tid in tracks():
        if m in SESSIONS[s]["excluded"]:
            continue
        ref = refs[s].get(m)
        if not ref:
            continue
        act, ev, dsp = load_track(out, tid)
        sets = variant_sets(role, act, ev, dsp)
        off, sc = score_track(ref, ev, sets)
        rows.append({"session": s, "mic": m, "role": role, "id": tid, "offset_ms": off * 1000, "scores": sc,
                     "rescued": {v: len(r) for v, (_, r) in sets.items()}})
    (out / "scores.json").write_text(json.dumps(rows))
    text = score_table(rows)
    (out / "scores.md").write_text(text)
    log(text)
    return rows


def score_table(rows):
    head = ["session", "track", "role", "refs"] + [f"{v} found/false/F" for v in VARIANTS]
    body = []
    for r in rows:
        sc = r["scores"]
        body.append([r["session"], r["mic"], r["role"], sc["base"]["n_ref"]]
                    + [f"{sc[v]['tp']}/{sc[v]['fp']}/{sc[v]['f']:.3f}" for v in VARIANTS])
    for role in ("kick", "snare", "tom"):
        sel = [r for r in rows if r["role"] == role]
        if not sel:
            continue
        cells = []
        for v in VARIANTS:
            tp = sum(r["scores"][v]["tp"] for r in sel)
            fp = sum(r["scores"][v]["fp"] for r in sel)
            fn = sum(r["scores"][v]["fn"] for r in sel)
            p = tp / (tp + fp) if tp + fp else 0.0
            rc = tp / (tp + fn) if tp + fn else 0.0
            cells.append(f"{tp}/{fp}/{(2 * p * rc / (p + rc) if p + rc else 0.0):.3f}")
        body.append(["**all**", f"**{role}**", role, sum(r["scores"]["base"]["n_ref"] for r in sel)] + cells)
    return mc.md_table(head, body)


# ---------------------------------------------------------------- review of the rescued hits the references lack

def peak_db(x, sr, t, before=0.005, after=0.030):
    a, b = max(0, int((t - before) * sr)), min(len(x), int((t + after) * sr))
    return 20 * math.log10(float(np.abs(x[a:b]).max()) + 1e-9) if b > a else -180.0


def envelope_features(x, sr, t):
    """Rise time 10 to 90 % of the peak amplitude within 30 ms of `t` (ms) and the time from the peak down to 20 dB
    below it (ms, capped at 200)."""
    a = max(0, int((t - 0.005) * sr))
    seg = np.abs(x[a:a + int(0.25 * sr)])
    if len(seg) < 100:
        return None
    k = max(1, int(0.0005 * sr))
    env = np.convolve(seg, np.ones(k) / k, mode="same")
    ipk = int(np.argmax(env[:int(0.03 * sr)])) if len(env) > int(0.03 * sr) else int(np.argmax(env))
    pk = env[ipk]
    if pk <= 0:
        return None
    lo = int(np.argmax(env[:ipk + 1] >= 0.1 * pk))
    hi = int(np.argmax(env[:ipk + 1] >= 0.9 * pk))
    tail = np.flatnonzero(env[ipk:] <= 0.1 * pk)
    decay = (tail[0] if len(tail) else len(env) - ipk) / sr * 1000
    return {"rise_ms": (hi - lo) / sr * 1000, "decay20_ms": min(200.0, decay)}


def band_spectrum(x, sr, t, bands=24):
    """Log energy in `bands` log-spaced bands from 60 Hz to 16 kHz of 50 ms after `t`, mean removed."""
    a = int(t * sr)
    seg = x[a:a + int(0.05 * sr)]
    if len(seg) < 256:
        return None
    w = np.hanning(len(seg))
    spec = np.abs(np.fft.rfft(seg * w)) ** 2
    f = np.fft.rfftfreq(len(seg), 1 / sr)
    edges = np.geomspace(60, min(16000, sr / 2 - 1), bands + 1)
    e = np.array([spec[(f >= edges[i]) & (f < edges[i + 1])].sum() + 1e-12 for i in range(bands)])
    v = 10 * np.log10(e)
    return v - v.mean()


def cosine(a, b):
    return float(np.dot(a, b) / (np.linalg.norm(a) * np.linalg.norm(b) + 1e-12))


def review_track(out, s, m, role, tid, refs, log):
    """The rescued hits of one track that no reference owns, with level, bleed test and features."""
    cfg = SESSIONS[s]
    ref = refs[s].get(m)
    act, ev, dsp = load_track(out, tid)
    sets = variant_sets(role, act, ev, dsp)
    x, sr = sf.read(str(audio_path(s, cfg["mics"][m][0])), dtype="float32", always_2d=True)
    x = x.mean(axis=1)
    o, osr = sf.read(str(audio_path(s, cfg["other"][role])), dtype="float32", always_2d=True)
    o = o.mean(axis=1)
    off = offset_of(ref, ev) if ref else 0.0
    shifted = np.asarray(ref if ref else []) + off
    pairs, _, _ = ck.match(list(shifted), ev, TOL)
    matched = [ev[j] for _, j in pairs]
    lv = np.array([peak_db(x, sr, t) for t in matched]) if matched else np.array([])
    med = float(np.median(lv)) if len(lv) else float("nan")
    ratio = np.array([peak_db(o, osr, t) - peak_db(x, sr, t) for t in matched]) if matched else np.array([])
    rmed = float(np.median(ratio)) if len(ratio) else float("nan")
    rsd = float(ratio.std()) if len(ratio) > 1 else float("nan")
    spec_ref = [band_spectrum(x, sr, t) for t in matched]
    spec_ref = [v for v in spec_ref if v is not None]
    spec_med = np.median(np.stack(spec_ref), axis=0) if spec_ref else None
    env_ref = [envelope_features(x, sr, t) for t in matched]
    env_ref = [e for e in env_ref if e]
    ref_rise = float(np.median([e["rise_ms"] for e in env_ref])) if env_ref else float("nan")
    ref_dec = float(np.median([e["decay20_ms"] for e in env_ref])) if env_ref else float("nan")
    cos_ref = [cosine(v, spec_med) for v in spec_ref] if spec_med is not None else []
    out_rows = []
    others = {k: np.asarray(v) for k, v in refs[s].items() if k != m and k != "toms_combined" and v}

    def owner(t):
        """The other instrument whose reference lies within 30 ms of the rescued hit (the hit is that drum's bleed)."""
        best = None
        for k, v in others.items():
            d = np.abs(v + 0.004 - t)
            if len(d) and d.min() <= 0.030 and (best is None or d.min() < best[1]):
                best = (k, float(d.min()))
        return best[0] if best else None

    for variant in ("cross", "dsp"):
        _, resc = sets[variant]
        pr, _, _ = ck.match(list(shifted), resc, TOL)
        owned = {j for _, j in pr}
        for j, t in enumerate(resc):
            sp = band_spectrum(x, sr, t)
            env = envelope_features(x, sr, t)
            lvl = peak_db(x, sr, t)
            rat = peak_db(o, osr, t) - lvl
            out_rows.append({
                "session": s, "mic": m, "role": role, "rescue": variant, "t": round(t, 3), "in_refs": j in owned,
                "other_ref": owner(t),
                "level_db": round(lvl, 1), "rel_median_db": round(lvl - med, 1) if med == med else None,
                "other_minus_own_db": round(rat, 1), "bleed_z": round((rat - rmed) / (rsd + 1e-9), 1) if rmed == rmed else None,
                "spec_cos": round(cosine(sp, spec_med), 2) if sp is not None and spec_med is not None else None,
                "rise_ms": round(env["rise_ms"], 1) if env else None,
                "decay20_ms": round(env["decay20_ms"], 0) if env else None})
    ctx = {"session": s, "mic": m, "role": role, "median_level_db": round(med, 1), "ratio_median_db": round(rmed, 1),
           "ratio_sd_db": round(rsd, 1), "ref_rise_ms": round(ref_rise, 1), "ref_decay20_ms": round(ref_dec, 0),
           "spec_cos_p10": round(float(np.percentile(cos_ref, 10)), 2) if cos_ref else None,
           "spec_cos_median": round(float(np.median(cos_ref)), 2) if cos_ref else None}
    return out_rows, ctx


def do_review(out, log=print):
    out = Path(out)
    refs = json.loads(REFS.read_text())
    rows, ctxs = [], []
    for s, m, role, tid in tracks():
        if m in SESSIONS[s]["excluded"] or not refs[s].get(m):
            continue
        r, c = review_track(out, s, m, role, tid, refs, log)
        rows += r
        ctxs.append(c)
    (out / "review.json").write_text(json.dumps({"rows": rows, "context": ctxs}))
    log(f"{len(rows)} rescued hits, {sum(1 for r in rows if not r['in_refs'])} not in the references")
    return rows, ctxs


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["run", "score", "review", "all"])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--jobs", type=int, default=3)
    a = ap.parse_args(argv)
    out = Path(a.out)
    if a.cmd in ("run", "all"):
        run_all(out, a.jobs)
    if a.cmd in ("score", "all"):
        do_score(out)
    if a.cmd in ("review", "all"):
        do_review(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
