"""P4-13 stage 1: flams and close pairs, both strokes are hits (measure only; no product change).

    uv run --with numpy --with soundfile --with mido python tools/flam_pairs.py pairs    # the labelled real close pairs
    uv run --with numpy --with soundfile --with mido python tools/flam_pairs.py kits     # the kits through the app (events)
    uv run --with numpy --with soundfile --with mido python tools/flam_pairs.py measure  # the candidate on kits and songs
    uv run --with numpy --with soundfile --with mido python tools/flam_pairs.py all

The model sees a flam or two hits closer than about 30 ms as one lobe and keeps the first stroke (P4-10 stage 1).
The candidate looks for a second stroke inside each output hit's lobe on the hit's own mic:

  1. The attack envelope: the own mic through a zero-phase high-pass at `hp_hz` (the stick attack, not the shell's
     low ring), the peak of |y| per block of HOP_S, held over `hold_s`.
  2. The first stroke's peak P1: the largest block in [hit, hit + `peak_s`] (12 ms: the own attack complex; a kick,
     snare or tom has a second, louder burst in the 1 kHz band about 4 ms after its first motion).
  3. A re-rise: the first block c after P1, at least `min_gap_s` (12 ms) after the hit and before `max_gap_s` (and
     `guard_s` before the next output hit), whose level is `rise_db` or more over the loudest block of the `look_s`
     before it (`base` "max", the hold excluded; "min": over the lowest block of the `rise_s` before it, not before
     P1), within `level_db` of P1, and `floor_db` over the floor before the hit (median block 50 to 2 ms before it).
     One second stroke per hit.
  4. Its onset: the version 3 first-motion walk (`refine.rs` `first_motion`, the Python port of `onset_guard.walk`)
     from the crossing, bounded by the dip before it (the walk never crosses into the first stroke).
  5. The bleed gate (P4-14/P4-15 cross-mic code): for every other mic B of the session (the other close mics, hat,
     overheads, ride) the transfer gain G(B->A) in the attack band (`onset_guard.calibrate_gain`, P2-6). The re-rise is
     bleed when B's attack envelope in [c - 2 ms, c + 1 ms] plus G plus `margin_db` reaches the own peak P2.

Measured (yardsticks):
  kits   the owner's DRT kit (`~/.cache/drumspike/drt-testkit`, read only: 03 flams, 09 close pairs, the other files for
         added false hits), the P4-11 kit (`tools/calib_kit.py` audio: close pairs 20 to 60 ms, acoustic and Ultrabeat
         shots, clean, noise, bleed) and a flam kit made here from the same acoustic shots (spacings 5 to 40 ms,
         grace 6 to 12 dB under the main, clean and bleed). Each kit runs through the app (release `service_tool`).
  songs  the seven P4-16 songs (`tools/fix_metric.py`): the `default` run's events (the app as it is), the added
         second strokes, and the fixes before and after per song and role (P4-16 yardstick, the same offsets).

The snare's added hits are also checked against the session's other snare sample layers (`LAYERS`, not the
reference): a second stroke the trigger track lacks can be in them. Results: `phase4-report.md` "P4-13 stage 1".

Outputs in `~/.cache/drumspike/flams/`; nothing goes into git. Sessions are read only.
"""
import argparse
import csv
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fix_metric as fm  # noqa: E402
import onset_guard as og  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
OUT = Path.home() / ".cache" / "drumspike" / "flams"
DRT = Path.home() / ".cache" / "drumspike" / "drt-testkit"
KIT = Path.home() / ".cache" / "drumspike" / "calib-kit"
LOCK = fm.LOCK
HOP_S = 0.000125
PAIR_S = 0.050          # a real close pair: a same-mic reference neighbour within 50 ms
KIT_TOL = 0.005         # kit match window (flams of 5 ms)
DEFAULTS = {"hp_hz": 1000.0, "hold_s": 0.001, "rise_s": 0.002, "rise_db": 9.0, "level_db": 30.0, "floor_db": 12.0,
            "min_gap_s": 0.012, "max_gap_s": 0.050, "guard_s": 0.010, "margin_db": 10.0, "peak_s": 0.012,
            "base": "max", "look_s": 0.003}
# the hat or overhead of each song: one more bleed source (the close mics come from fix_metric's table)
EXTRA = {"Session A": ["hh_03", "oh_01"], "Session B": ["hh_01", "OH_01", "ride_01"],
         "Session C": ["hh_03", "oh_03.L", "ri_03"], "Session H": ["hh_04", "oh_03.L"],
         "Session F": ["hh_01", "oh_01.L", "ri_01"], "Session G": ["hh_10", "oh_01.L", "ri_01"],
         "Session D": ["hh", "oh HH", "oh Ri", "ri"]}
# the snare sample layers that are not the P4-16 reference (an independent look at a second stroke); files with
# `.L`/`.R` are read as one track
LAYERS = {"Session A": ["s sampl savior_01", "s sampl sd3_01"], "Session B": ["s samp savior_01", "s samp sd3_01"],
          "Session C": ["s samp 2_02"], "Session G": ["s samp savior_03", "s samp sd3_03"],
          "Session D": ["s samp saviour", "s samp sd3"]}
LAYER_GAP_S = 0.005     # the layers' onsets with a 5 ms merge (the reference keeps 20 ms)
LAYER_TOL_S = 0.003
ROLE_OF = {"kick": "kick", "snare": "snare", "toms": "tom", "tom": "tom"}


# ---------------------------------------------------------------- the candidate (pure; the tests cover these)

def attack_env(x, sr, p=DEFAULTS):
    """The attack envelope on a block grid: |high-pass(x)| maximum per block of HOP_S, held over `hold_s` (the block
    and the ones before it). Returns (env, hop in samples)."""
    y = og.zero_phase(np.asarray(x, np.float64), sr, [("hp", p["hp_hz"])]) if p["hp_hz"] else np.asarray(x, float)
    hop = max(1, int(round(HOP_S * sr)))
    nb = len(y) // hop
    blk = np.abs(y[:nb * hop]).reshape(nb, hop).max(axis=1) if nb else np.zeros(0)
    k = max(1, int(round(p["hold_s"] / HOP_S)))
    pad = np.concatenate([np.zeros(k - 1), blk])
    env = np.lib.stride_tricks.sliding_window_view(pad, k).max(axis=1) if nb else blk
    return env, hop


def transfer_gains(env_a, sources, own_times, sr, hop):
    """[(env_b, G(B->A) dB)] for the other mics' attack envelopes `sources` ({name: env_b}); a source without a
    calibrated gain (fewer than 5 strong hits) is left out."""
    sb = sr / hop
    out = []
    for env_b in sources.values():
        n = min(len(env_a), len(env_b))
        g = og.calibrate_gain(env_a[:n], env_b[:n], og.strong_peaks(env_b[:n], sb), own_times, sb)
        if g is not None:
            out.append((env_b[:n], g))
    return out


LATE_END_S = 0.030     # the late-peak window ends here (the first stroke's peak window `peak_s` ends where it starts)
LATE_MAX_DB = -3.0     # the rule runs on a mic whose late peak is under this (dB)


LATE_SHARE_MIN = 0.4   # ... and skipped only when also this share of the hits peak late (W)


def late_ratios(env, hits, sr, hop, p=DEFAULTS):
    """Per hit with a full 30 ms window and a first peak: the largest attack-envelope block in [hit + peak_s, hit + 30 ms)
    over the largest in [hit, hit + peak_s). The Rust twin is `flam::late_ratios`."""
    r = sr / hop
    f, e = max(1, int(p["peak_s"] * r)), int(LATE_END_S * r)
    out = []
    for t in hits:
        b = int(t * sr) // hop
        if b < 0 or b + e >= len(env):
            continue
        p1 = float(env[b:b + f].max())
        if p1 > 0:
            out.append(float(env[b + f:b + e].max()) / p1)
    return out


def late_peak_db(env, hits, sr, hop, p=DEFAULTS):
    """A: the median over `hits` (s) of the late ratio in dB. A snare top mic decays early (-6 to -9 dB on 9 sessions),
    a bottom mic's wires peak late (+0 to +6 dB); on a monotone decay A is the decay at 12 ms. None without a value."""
    v = [20 * np.log10(x) if x > 0 else -np.inf for x in late_ratios(env, hits, sr, hop, p)]
    return float(np.median(v)) if v else None


def late_share(env, hits, sr, hop, p=DEFAULTS):
    """W: the share of the same hits whose late peak beats the first (0.00 to 0.23 top mics, 0.52 to 0.94 bottom mics,
    0.06 to 0.12 the DRT single-snare files). None without a value."""
    v = late_ratios(env, hits, sr, hop, p)
    return sum(1 for x in v if x > 1) / len(v) if v else None


def late_peaking(late, share):
    """True for a late-peaking mic (a snare bottom): A at -3 dB or more AND W at 0.4 or more; the rule does not run on it."""
    return late is not None and share is not None and late >= LATE_MAX_DB and share >= LATE_SHARE_MIN


def second_onsets(x, sr, hits, p=DEFAULTS, env=None, gains=()):
    """The second strokes inside the lobes of `hits` (seconds, the output hits of the own mic `x`). Returns one dict
    per re-rise found: t (onset, s), first (the hit, s), gap_ms, rise_db, level_db (P2 against P1), bleed (bool: the
    gate says another mic explains it). `env`: a precomputed `attack_env`; `gains`: [(env_b, G)] of `transfer_gains`."""
    if env is None:
        env, hop = attack_env(x, sr, p)
    else:
        hop = max(1, int(round(HOP_S * sr)))
    times = sorted(float(t) for t in hits)
    nb = len(env)
    b = lambda s: int(round(s / HOP_S))  # noqa: E731
    rise_b, peak_b, min_b, max_b, guard_b = b(p["rise_s"]), b(p["peak_s"]), b(p["min_gap_s"]), b(p["max_gap_s"]), \
        b(p["guard_s"])
    out = []
    for k, t in enumerate(times):
        b0 = int(t * sr) // hop
        end = min(b0 + max_b, nb)
        if k + 1 < len(times):
            end = min(end, int(times[k + 1] * sr) // hop - guard_b)
        if end - b0 <= max(min_b, 2):
            continue
        ipk = b0 + int(np.argmax(env[b0:min(b0 + peak_b, end) + 1]))
        p1 = float(env[ipk])
        if p1 <= 0:
            continue
        f0, f1 = max(0, b0 - b(0.050)), max(0, b0 - b(0.002))
        floor = float(np.median(env[f0:f1])) if f1 - f0 >= 8 else 0.0
        start = max(ipk + 1, b0 + min_b)
        if start >= end:
            continue
        seg = env[ipk:end]
        if p["base"] == "max":   # over the loudest block of the `look_s` before the rise (the hold excluded)
            g_b, l_b = b(p["hold_s"] + 0.00025), b(p["look_s"])
            lo = np.array([env[max(0, ipk + i - g_b - l_b):ipk + i - g_b + 1].max() for i in range(len(seg))])
        else:                    # over the lowest block of the `rise_s` before it, from P1 on
            lo = np.array([seg[max(0, i - rise_b):i].min() if i > 0 else seg[0] for i in range(len(seg))])
        i0 = start - ipk
        ok = ((seg[i0:] >= lo[i0:] * 10 ** (p["rise_db"] / 20)) & (seg[i0:] >= p1 * 10 ** (-p["level_db"] / 20))
              & (seg[i0:] >= floor * 10 ** (p["floor_db"] / 20)))
        if not ok.any():
            continue
        c = ipk + i0 + int(np.argmax(ok))
        d = max(ipk, c - rise_b) + int(np.argmin(env[max(ipk, c - rise_b):c]))
        p2 = float(env[c:min(c + peak_b, nb)].max())
        sc = c * hop
        foot, _ = og.walk(np.asarray(x, np.float64), sc, sr, prev_mid=(d + 1) * hop)   # never before the bound
        bleed = False
        for env_b, g in gains:
            lo_b, hi_b = max(0, c - b(0.002)), min(len(env_b), c + b(0.001) + 1)
            if hi_b > lo_b and float(env_b[lo_b:hi_b].max()) * 10 ** ((g + p["margin_db"]) / 20) >= p2:
                bleed = True
                break
        out.append({"t": foot / sr, "first": t, "gap_ms": (foot / sr - t) * 1000.0,
                    "rise_db": 20 * np.log10(p2 / max(float(env[d]), 1e-12)), "level_db": 20 * np.log10(p2 / p1),
                    "bleed": bleed})
    return out


def added_events(found):
    """The second strokes that pass the bleed gate, as output events."""
    return [{"t": f["t"], "added": True} for f in found if not f["bleed"]]


def confirmed(times, layer_onsets, offset, tol=LAYER_TOL_S):
    """For each time in `times`: True when a layer onset (shifted by `offset`, the layer's latency against the
    own mic) lies within `tol` seconds."""
    on = np.sort(np.asarray(layer_onsets, float)) + offset
    out = []
    for t in times:
        i = int(np.searchsorted(on, t))
        out.append(any(0 <= j < len(on) and abs(on[j] - t) <= tol + 1e-12 for j in (i - 1, i)))
    return out


def layer_onsets(song, cfg):
    """The onsets of the song's snare sample layers (`LAYERS`, union, merged within LAYER_GAP_S); None without."""
    import learn_eval as le
    stems = LAYERS.get(song)
    if not stems:
        return None
    allon = []
    for stem in stems:
        files = [fm.audio_path(cfg, stem)]
        if not files[0].is_file():
            files = [fm.audio_path(cfg, stem + ".L"), fm.audio_path(cfg, stem + ".R")]
        x, sr = le.read_mono(files)
        allon += le.ref_onsets(x, sr, gap=LAYER_GAP_S).tolist()
    merged = []
    for t in sorted(allon):
        if not merged or t - merged[-1] >= LAYER_GAP_S:
            merged.append(t)
    return merged


def with_layers(ref, layers, ref_off, layer_off, gap=LAYER_GAP_S):
    """The reference plus the layer onsets that lie more than `gap` from every reference hit, in the reference's
    time base (a layer onset t is t - layer_off + ref_off)."""
    r = np.sort(np.asarray(ref, float))
    extra = []
    for t in np.asarray(layers, float) - layer_off + ref_off:
        i = int(np.searchsorted(r, t))
        if not any(0 <= j < len(r) and abs(r[j] - t) <= gap for j in (i - 1, i)):
            extra.append(float(t))
    return sorted(r.tolist() + extra)


def close_pairs(onsets, within=PAIR_S):
    """Indices of the reference hits that have an earlier reference hit of the same mic within `within` seconds
    (the second strokes of real close pairs)."""
    on = np.sort(np.asarray(onsets, float))
    return [i for i in range(1, len(on)) if on[i] - on[i - 1] <= within + 1e-9]


def kit_score(ref, labels, before, after, tol=KIT_TOL):
    """Found per label before and after (one to one, nearest first, within `tol`), and the false hits.
    `ref`: reference times; `labels`: one label per reference time; `before`, `after`: event times."""
    res = {}
    for name, est in (("before", before), ("after", after)):
        pairs = fm.match(list(ref), list(est), tol)
        hit = {i for i, _ in pairs}
        per = {}
        for i, lab in enumerate(labels):
            f, n = per.get(lab, (0, 0))
            per[lab] = (f + (i in hit), n + 1)
        res[name] = {"per": per, "false": len(est) - len(pairs), "found": len(pairs)}
    return res


# ---------------------------------------------------------------- the flam kit (acoustic shots, spacings 5 to 40 ms)

FLAM_GAPS_MS = [5, 10, 15, 20, 25, 30, 35, 40]


def flam_mix(shots, sr, gaps_ms, reps, n_single, seed):
    """A mix of flams and single hits from `shots` (arrays, the shot starting at its first sample). Each flam is a
    grace (a shot 6 to 12 dB under) and a main `gap` later; singles are controls. Returns (mix, rows) with rows
    {t, label} (label `grace_<gap>`, `main_<gap>`, `single`); the times are the shots' first samples."""
    rng = np.random.default_rng(seed)
    plan = [("flam", g) for g in gaps_ms for _ in range(reps)] + [("single", 0)] * n_single
    plan = [plan[i] for i in rng.permutation(len(plan))]
    pos, rows, parts = int(0.5 * sr), [], []
    for kind, g in plan:
        main = shots[int(rng.integers(0, len(shots)))]
        if kind == "flam":
            grace = shots[int(rng.integers(0, len(shots)))] * 10 ** (-rng.uniform(6.0, 12.0) / 20)
            q = pos + int(round(g / 1000.0 * sr))
            parts += [(pos, grace), (q, main)]
            rows += [{"t": pos / sr, "label": f"grace_{g}"}, {"t": q / sr, "label": f"main_{g}"}]
            pos = q
        else:
            parts.append((pos, main))
            rows.append({"t": pos / sr, "label": "single"})
        pos += max(int(0.25 * sr), len(main)) + int(rng.uniform(0.05, 0.2) * sr)
    mix = np.zeros(pos + int(0.5 * sr))
    for q, y in parts:
        n = min(len(y), len(mix) - q)
        mix[q:q + n] += y[:n]
    return mix, rows


def make_flam_kit(out, log=print):
    """The flam kit: per shell (kick, snare, t1 to t4) a clean and a bleed mix (the next shell's clean mix at
    -20 dB, as the P4-11 kit) with 3 flams per spacing and 8 singles. Written to `<out>/flamkit/`."""
    import calib_kit as ck
    pool = ck.choose_acoustic(ck.acoustic_pool())
    mixes, refs = {}, {}
    for si, sh in enumerate(ck.SHELLS):
        shots = []
        for e in pool[sh]:
            y = np.asarray(e["y"], float)[int(e["abs_start"]):]
            shots.append(y)
        mixes[sh], refs[sh] = flam_mix(shots, ck.SR_REAL, FLAM_GAPS_MS, 3, 8, seed=1300 + si)
    d = Path(out) / "flamkit"
    for cond in ("clean", "bleed"):
        (d / cond).mkdir(parents=True, exist_ok=True)
        for sh in ck.SHELLS:
            x = mixes[sh]
            if cond == "bleed":
                x = ck.apply_condition(x, mixes[ck.BLEED_FROM[sh]], "bleed", ck.SR_REAL, 0)
            ck.write_wav(d / cond / f"{sh}.wav", x, ck.SR_REAL)
    (d / "refs.json").write_text(json.dumps(refs))
    log(f"flam kit: {sum(len(r) for r in refs.values())} reference hits per condition")


# ---------------------------------------------------------------- the app's analysis of the kits

def run_project(tool, files, work, log=print):
    """One app project with `files` [(path, role)] analysed (`service_tool cli run`): {stem: events}."""
    if work.exists():
        shutil.rmtree(work)
    (work / "p").mkdir(parents=True)
    note = {"kick": 36, "snare": 38, "tom": 47}
    steps = [{"command": "project_new", "args": {"name": "flams", "dir": str(work / "p")}},
             {"command": "tracks_add", "args": {"sid": "$sid", "paths": [str(f) for f, _ in files],
                                                "roles": [r for _, r in files], "notes": [note[r] for _, r in files]},
              "wait": True},
             {"command": "analyze_start", "args": {"sid": "$sid"}, "wait": True}]
    steps += [{"command": "events_get", "args": {"sid": "$sid", "name": fm.track_name(Path(f).stem)}} for f, _ in files]
    (work / "steps.json").write_text(json.dumps(steps))
    env = {k: v for k, v in os.environ.items() if not k.startswith("DRUMSPIKE_")}
    env.update({"DRUMSPIKE_HOME": str(work / "h"), "DRUMSPIKE_PROJECTS": str(work / "p")})
    r = subprocess.run([str(tool), "cli", "run", str(work / "steps.json")], capture_output=True, text=True, env=env,
                       stdin=subprocess.DEVNULL)
    rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
    bad = [x for x in rows if not x.get("ok", False)]
    if r.returncode != 0 or bad or len(rows) != len(steps):
        raise RuntimeError(f"{work.name}: exit {r.returncode} {bad[:1]} {r.stderr[-400:]}")
    res = {Path(f).stem: rows[3 + i]["result"]["events"] for i, (f, _) in enumerate(files)}
    shutil.rmtree(work, ignore_errors=True)
    return res


def kit_projects(out):
    """{name: [(path, role)]}: the DRT kit, the P4-11 kit (acoustic and Ultrabeat shots, three conditions) and the
    flam kit (clean, bleed)."""
    import calib_kit as ck
    pro = {"drt": [(f, "kick" if f.name.startswith("07_") else "snare") for f in sorted(DRT.glob("0*.wav"))]}
    for kind in ("acoustic", "real"):
        for cond in ("clean", "noise", "bleed"):
            pro[f"p411_{kind}_{cond}"] = [(f, ck.ROLE[f.stem.split("_")[0]])
                                          for f in sorted((KIT / "audio" / kind / cond).glob("*.wav"))]
    for cond in ("clean", "bleed"):
        pro[f"flam_{cond}"] = [(f, ck.ROLE[f.stem]) for f in sorted((Path(out) / "flamkit" / cond).glob("*.wav"))]
    return pro


def do_kits(out, log=print):
    out = Path(out)
    tool = Path(os.environ.get("DRUMSPIKE_CALIB_TOOL") or ROOT / "target" / "release" / "service_tool")
    if not tool.is_file():
        raise SystemExit(f"need the release service_tool ({tool})")
    if not (out / "flamkit" / "refs.json").is_file():
        make_flam_kit(out, log)
    dst = out / "kits"
    dst.mkdir(parents=True, exist_ok=True)
    try:
        for name, files in kit_projects(out).items():
            if (dst / f"{name}.json").is_file():
                continue
            if not files:
                raise SystemExit(f"{name}: no audio files")
            fm.wait_for_room(log)
            LOCK.write_text(f"flam_pairs kits {name}\n")
            try:
                ev = run_project(tool, files, out / "scratch" / name, log)
            finally:
                LOCK.unlink(missing_ok=True)
            (dst / f"{name}.json").write_text(json.dumps({"files": {Path(f).stem: str(f) for f, _ in files},
                                                          "events": ev}))
            log(f"ran {name}")
    finally:
        LOCK.unlink(missing_ok=True)
        shutil.rmtree(out / "scratch", ignore_errors=True)


# ---------------------------------------------------------------- measures

def read_mono(path):
    import soundfile as sf
    x, sr = sf.read(str(path), dtype="float32", always_2d=True)
    return x.mean(axis=1).astype(np.float64), int(sr)


def kit_refs(name, stem, out):
    """(times, labels) of one kit track; `None` for a file without targets."""
    import calib_kit as ck
    if name == "drt":
        with open(DRT / "ground_truth.csv") as fh:
            rows = [r for r in csv.DictReader(fh) if r["file"] == stem + ".wav"]
        tg = [r for r in rows if r["role"].startswith("target")]
        return [float(r["onset_s"]) for r in tg], [r["role"] for r in tg]
    if name.startswith("p411_"):
        _, kind, cond = name.split("_")
        rows = ck.read_csv(KIT / "ref" / kind / cond / f"{stem}.csv")
        lab = []
        for r in rows:
            if r["group"] == "pair":
                same = [q for q in rows if q["group"] == "pair" and q["pair"] == r["pair"]]
                lab.append("pair_first" if r["abs_s"] == min(q["abs_s"] for q in same) else "pair_second")
            else:
                lab.append("main")
        return [float(r["abs_s"]) for r in rows], lab
    refs = json.loads((Path(out) / "flamkit" / "refs.json").read_text())[stem]
    return [r["t"] for r in refs], [r["label"] for r in refs]


def measure_kits(out, p=DEFAULTS, log=print):
    """Per kit project and label group: found before and after, false before and after, added hits."""
    out = Path(out)
    rows = []
    for f in sorted((out / "kits").glob("*.json")):
        name = f.stem
        d = json.loads(f.read_text())
        for stem, events in d["events"].items():
            ref, lab = kit_refs(name, stem, out)
            x, sr = read_mono(d["files"][stem])
            hits = fm.outputs(events)
            found = second_onsets(x, sr, hits, p)
            add = [e["t"] for e in added_events(found)]
            sc = kit_score(ref, lab, hits, sorted(hits + add))
            rows.append({"kit": name, "track": stem, "added": len(add), "score": sc})
    return rows


def kit_group(label):
    """The reported group of a kit label."""
    if label.startswith("main_") or label == "target_main":
        return "flam main"
    if label.startswith("grace_") or label == "target_grace":
        return "flam grace"
    if label.startswith("target_second") or label == "pair_second":
        return "pair second"
    if label in ("target_first", "pair_first"):
        return "pair first"
    return "single"


def kit_table(rows):
    """{(kit family, group): [found before, found after, n]} and {(kit family): [false before, false after]}."""
    cells, false = {}, {}
    for r in rows:
        fam = "DRT" if r["kit"] == "drt" else ("flam kit " + r["kit"].split("_")[1] if r["kit"].startswith("flam")
                                               else "P4-11 " + r["kit"].split("_")[1] + " " + r["kit"].split("_")[2])
        for lab, (fb, n) in r["score"]["before"]["per"].items():
            fa = r["score"]["after"]["per"][lab][0]
            c = cells.setdefault((fam, kit_group(lab)), [0, 0, 0])
            c[0] += fb
            c[1] += fa
            c[2] += n
        fl = false.setdefault(fam, [0, 0])
        fl[0] += r["score"]["before"]["false"]
        fl[1] += r["score"]["after"]["false"]
    return cells, false


def song_signals(song, cfg):
    """{mic: (x, sr)} of the song's close mics in the P4-16 table and its hat or overhead (`EXTRA`)."""
    sig = {}
    for m, (stem, _role) in cfg["mics"].items():
        sig[m] = read_mono(fm.audio_path(cfg, stem))
    for stem in EXTRA.get(song, []):
        p = fm.audio_path(cfg, stem)
        if p.is_file():
            sig["extra " + stem] = read_mono(p)
    return sig


def measure_songs(out, grid, log=print):
    """For every parameter set of `grid` ({name: params}) and every song and mic: the added second strokes (with
    and without the bleed gate) and the scores before and after. Envelopes and gains are computed once per
    high-pass frequency."""
    out = Path(out)
    refs = json.loads((fm.OUT / "refs.json").read_text())
    table = fm.songs()
    res = {name: {} for name in grid}
    for song in refs:
        run = json.loads((fm.OUT / "run" / f"{fm.track_name(song)}_default.json").read_text())
        sig = song_signals(song, table[song])
        layers = layer_onsets(song, table[song])
        envs = {}
        for name, p in grid.items():
            key = (p["hp_hz"], p["hold_s"])
            if key not in envs:
                envs[key] = {m: attack_env(x, sr, p)[0] for m, (x, sr) in sig.items()}
            res[name][song] = {}
            for mic, v in refs[song]["mics"].items():
                x, sr = sig[mic]
                events = run["events"][mic]
                hits = fm.outputs(events)
                hop = max(1, int(round(HOP_S * sr)))
                env_a = envs[key][mic]
                others = {m: e for m, e in envs[key].items() if m != mic}
                gains = transfer_gains(env_a, others, hits, sr, hop)
                found = second_onsets(x, sr, hits, p, env=env_a, gains=gains)
                off = fm.track_offset(v["onsets"], events)
                before = fm.score_track(v["onsets"], events, off)
                after_ev = list(events) + added_events(found)
                after = fm.score_track(v["onsets"], after_ev, off)
                nogate = fm.score_track(v["onsets"], list(events) + [{"t": f["t"]} for f in found], off)
                pairs = close_pairs(v["onsets"])
                on = np.sort(np.asarray(v["onsets"], float)) + off
                pf = [sum(1 for i in pairs if any(abs(t - on[i]) <= fm.TOL for t in fm.outputs(ev)))
                      for ev in (events, after_ev)]
                on_s = np.sort(np.asarray(v["onsets"], float)) + off
                for f in found:
                    f["in_ref"] = bool(len(on_s) and np.min(np.abs(on_s - f["t"])) <= fm.TOL)
                lay_scores = None
                if layers is not None and v["role"] == "snare":
                    l_off = fm.track_offset(layers, events)
                    lay = confirmed([f["t"] for f in found], layers, l_off)
                    for f, c in zip(found, lay):
                        f["layer"] = bool(c)
                    ref2 = with_layers(v["onsets"], layers, off, l_off)
                    lay_scores = {"before": strip(fm.score_track(ref2, events, off)),
                                  "after": strip(fm.score_track(ref2, after_ev, off))}
                res[name][song][mic] = {"role": v["role"], "before": strip(before), "after": strip(after),
                                        "nogate": strip(nogate), "with_layers": lay_scores, "pairs": len(pairs), "pairs_found": pf,
                                        "found": [{k: (bool(w) if k in ("bleed", "layer", "in_ref") else float(w)) for k, w in f.items()}
                                                  for f in found]}
        log(f"measured {song}")
    return res


def strip(s):
    return {k: s[k] for k in ("n_ref", "n_out", "false", "missed", "moved", "fixes", "matched")}


def role_sums(res_one):
    """{role: {before, after, nogate: {false, missed, moved, fixes}, n_ref, added, gated}} over the songs."""
    out = {}
    for mics in res_one.values():
        for r in mics.values():
            o = out.setdefault(r["role"], {"n_ref": 0, "added": 0, "gated": 0, "pairs": 0, "pairs_before": 0,
                                           "pairs_after": 0,
                                           **{c: {k: 0 for k in ("false", "missed", "moved", "fixes")}
                                              for c in ("before", "after", "nogate")}})
            o["n_ref"] += r["before"]["n_ref"]
            kept = [f for f in r["found"] if not f["bleed"]]
            o["no_ref"] = o.get("no_ref", 0) + sum(1 for f in kept if not f["in_ref"])
            o["no_ref_layer"] = o.get("no_ref_layer", 0) + sum(1 for f in kept if not f["in_ref"] and f.get("layer"))
            o["layer_checked"] = o.get("layer_checked", 0) + sum(1 for f in kept if "layer" in f)
            o["added"] += sum(1 for f in r["found"] if not f["bleed"])
            o["gated"] += sum(1 for f in r["found"] if f["bleed"])
            o["pairs"] += r["pairs"]
            o["pairs_before"] += r["pairs_found"][0]
            o["pairs_after"] += r["pairs_found"][1]
            for c in ("before", "after", "nogate"):
                for k in ("false", "missed", "moved", "fixes"):
                    o[c][k] += r[c][k]
    return out


def layer_sums(res_one):
    """The snare mics of the songs with sample layers, scored against the reference plus the layers' extra onsets:
    {before, after: {false, missed, moved, fixes, n_ref}, songs}."""
    out = {"songs": [], **{c: {k: 0 for k in ("n_ref", "false", "missed", "moved", "fixes")} for c in ("before", "after")}}
    for song, mics in res_one.items():
        for r in mics.values():
            if r.get("with_layers"):
                out["songs"].append(song)
                for c in ("before", "after"):
                    for k in out[c]:
                        out[c][k] += r["with_layers"][c][k]
    return out


def verdict(sums):
    """Accepted when the added false hits are fewer than the found second strokes over all roles (net fixes go
    down) and no role's fixes grow by more than 1 % of its reference hits."""
    added_false = sum(s["after"]["false"] - s["before"]["false"] for s in sums.values())
    found = sum(s["before"]["missed"] - s["after"]["missed"] for s in sums.values())
    worse = [r for r, s in sums.items() if s["after"]["fixes"] - s["before"]["fixes"] > 0.01 * s["n_ref"]]
    return {"added_false": added_false, "found": found, "worse": worse,
            "accepted": added_false < found and not worse}


GRID = {f"R{r}_m{m}": {**DEFAULTS, "rise_db": float(r), "margin_db": float(m)} for r in (6, 9, 12) for m in (0, 6, 10)}


def do_pairs(out, log=print):
    """The labelled real close pairs: per song and mic the reference hits with a same-mic neighbour within 50 ms,
    and how many of them the app finds today."""
    refs = json.loads((fm.OUT / "refs.json").read_text())
    rows = []
    for song in refs:
        run = json.loads((fm.OUT / "run" / f"{fm.track_name(song)}_default.json").read_text())
        for mic, v in refs[song]["mics"].items():
            events = run["events"][mic]
            off = fm.track_offset(v["onsets"], events)
            on = np.sort(np.asarray(v["onsets"], float))
            pairs = close_pairs(on)
            est = fm.outputs(events)
            pairs_m = fm.match([float(t) + off for t in on], est)
            hit = {i for i, _ in pairs_m}
            gaps = [round(1000 * (on[i] - on[i - 1]), 1) for i in pairs]
            rows.append({"song": song, "mic": mic, "role": v["role"], "n_ref": len(on), "pairs": len(pairs),
                         "second_found": sum(1 for i in pairs if i in hit),
                         "first_found": sum(1 for i in pairs if i - 1 in hit), "gaps_ms": gaps})
    Path(out).mkdir(parents=True, exist_ok=True)
    (Path(out) / "pairs.json").write_text(json.dumps(rows, indent=1))
    for r in rows:
        if r["pairs"]:
            log(f"{r['song']} {r['mic']}: {r['pairs']} pairs, second found {r['second_found']}, first found "
                f"{r['first_found']}, gaps {r['gaps_ms']}")
    return rows


def do_measure(out, grid=None, log=print):
    out = Path(out)
    grid = grid or GRID
    songs = measure_songs(out, grid, log)
    (out / "songs.json").write_text(json.dumps(songs))
    kits = {name: measure_kits(out, p, log) for name, p in grid.items()}
    (out / "kits.json").write_text(json.dumps(kits))
    summary = {}
    for name in grid:
        sums = role_sums(songs[name])
        cells, false = kit_table(kits[name])
        summary[name] = {"roles": sums, "verdict": verdict(sums), "snare_with_layers": layer_sums(songs[name]),
                         "kit": {f"{a} | {b}": v for (a, b), v in cells.items()}, "kit_false": false}
        v = summary[name]["verdict"]
        log(f"{name}: songs found {v['found']}, added false {v['added_false']}, worse {v['worse']}; kit "
            + ", ".join(f"{k} {c[0]}->{c[1]}/{c[2]}" for k, c in summary[name]["kit"].items()
                        if k.endswith(("flam main", "pair second"))))
    (out / "summary.json").write_text(json.dumps(summary, indent=1))
    return summary


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["pairs", "kits", "measure", "all"])
    ap.add_argument("--out", default=str(OUT))
    a = ap.parse_args(argv)
    if a.cmd in ("pairs", "all"):
        do_pairs(a.out)
    if a.cmd in ("kits", "all"):
        do_kits(a.out)
    if a.cmd in ("measure", "all"):
        do_measure(a.out)


if __name__ == "__main__":
    main()
