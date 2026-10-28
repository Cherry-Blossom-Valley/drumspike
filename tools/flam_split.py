"""P4-13 stage 2, stage A: the evidence for the flam split (second strokes of flams become hits).

    uv run --with numpy --with soundfile --with mido python tools/flam_split.py songs     # the seven songs
    uv run --with numpy --with soundfile --with mido python tools/flam_split.py heldout   # Session E, Summer Nights
    uv run --with numpy --with soundfile --with mido python tools/flam_split.py kit       # the DRT kit
    uv run --with numpy --with soundfile --with mido python tools/flam_split.py all
    uv run --with numpy --with soundfile --with mido python tools/flam_split.py parity    # stage B: the Rust rule against this one
    uv run --with numpy --with soundfile --with mido python tools/flam_split.py app --app-run DIR   # stage B: the app's flam hits
    uv run --with numpy --with soundfile --with mido python tools/flam_split.py real --app-run DIR --off-run DIR2   # real sessions
    options: --run DIR (a fix_metric run folder with `run/`, default ~/.cache/drumspike/fix-metric-p421b), --refs DIR (a
    folder with the fair `refs.json` from `fix_metric.py score`, default ~/.cache/drumspike/fix-metric-p413b-base),
    --out DIR, --rise DB, --margin DB, --min-gap MS (rule variants)

The rule is the stage 1 best rule of `tools/flam_pairs.py` (R9_m10: a second stroke inside a hit's lobe, 1 kHz attack
envelope re-rise of 9 dB, 12 to 50 ms after the hit, version 3 walk, cross-mic bleed gate of 10 dB). Every added hit
(a second stroke that no hit of the run owns) is classified:

  owner      within 15 ms of a second stroke the owner confirmed by listening (2026-10-14, `OWNER`)
  reference  a reference hit lies within 20 ms (the app missed that stroke: a found hit)
  layer      an onset of a sample layer of the session that is not the reference lies within 3 ms (P4-16b)
  witness    the rise test passes on the own mic and on the witness mic `s bot` (P4-16b: no kick hit within 12 ms), and no
             hat, overhead or ride explains the re-rise (`hat_cause`: the cross-mic gate with those mics alone and a margin of 20 dB, looser than the rule's 10 dB)
  hat        the witness passes but a hat, overhead or ride explains the re-rise
  none       no evidence: the hit looks false

"Looks false" is `hat` or `none`. Held out: the two sessions that the yardstick leaves out and the rule was not fitted on
(the app analyses the snare top mic; the kick mic of the session gives the kick hits for the witness test).
Outputs in `~/.cache/drumspike/flam-split/`. The sessions are read only. Results: `phase4-report.md` "P4-13 stage 2".

Stage B (the rule in the app, `crates/core/src/detect/flam.rs`):
  parity   the Rust rule (`parity flam`, release build) against `second_onsets` on the same input: the snare mic of the
           seven songs with the output hits of `--run` (the `flam` hits left out) and the song's other mics (`fp.EXTRA`
           included), and the two held-out snare mics with the stage A events and the kick, hat, overhead (L and R
           files) and ride mics. Every second stroke must be equal (sample, bleed). Writes `parity.json`.
  app      the app's own flam hits: in a `fix_metric.py run` folder (`--app-run`, the rule on) the hits tagged `flam` of
           each snare mic against this rule on the run's other output hits with the project's other mics as sources;
           the held-out sessions as an app project (snare, kick, hat, overheads, ride; the rule on and pinned off, `0`);
           the DRT kit, one project per file, the rule on and off (found second strokes, false hits per file).
           Writes `app.json`.
  real     the three real sessions (P4-6b references): found, false and F per mic with the rule (`--app-run`) and
           pinned off (`--off-run`). Writes `real.json`.
"""
import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import firm_fixes as ff  # noqa: E402
import fix_metric as fm  # noqa: E402
import flam_pairs as fp  # noqa: E402

OUT = Path.home() / ".cache" / "drumspike" / "flam-split"
RUN = Path.home() / ".cache" / "drumspike" / "fix-metric-p421b"
REFS = Path.home() / ".cache" / "drumspike" / "fix-metric-p413b-base"
SAMPLES = Path.home() / ".cache" / "drumspike" / "samples"
DRT = Path.home() / ".cache" / "drumspike" / "drt-testkit"
OWNER_TOL_S = 0.015
REF_TOL_S = 0.020
HAT_MARGIN_DB = 20.0        # the hat or cymbal explains a re-rise when its attack plus the gain plus 20 dB reaches the stroke (looser than the rule's 10 dB)
MAX_FALSE_SHARE = 0.10      # the bar: at most 10 % of the held-out added hits look false

# the 11 second strokes the owner confirmed (listening page, 2026-10-14): song -> times (s) on the snare top mic
OWNER = {"Session B": [224.80, 226.71, 230.52, 232.43, 234.33, 236.23, 238.15],
         "Session G": [187.26, 189.33, 192.46], "Session A": [152.28]}

# held-out sessions: (session, folder, snare stem, witness stem, kick stem, layer stems, bleed sources)
HELDOUT = [("Session E", "Session E", "s top-AtA2", "s btm-AtA2", "k in-AtA2",
            ["s sample savior-AtA2", "s sample sd3-AtA2"], ["hh-AtA2", "oh-AtA2", "ri-AtA2"]),
           ("Session I", "Session I", "s top_01", "s bot_01", "k in_01",
            ["s samp 1_01", "s samp 2_01", "s samp 3_01"], ["hh_01", "oh_01", "ri_01"])]


# ---------------------------------------------------------------- pure helpers (the tests cover these)

def params(rise=9.0, margin=10.0, min_gap_ms=12.0):
    """The rule's parameters: the stage 1 defaults with the three that the variants change."""
    return {**fp.DEFAULTS, "rise_db": float(rise), "margin_db": float(margin), "min_gap_s": min_gap_ms / 1000.0}


def near(times, t, tol):
    """True when a time of `times` lies within `tol` seconds of `t`."""
    a = np.sort(np.asarray(times, float))
    return bool(len(a)) and fm.near_any(a, float(t), tol)


def classify(row):
    """The class of one added hit from its flags: owner, reference, layer, witness, hat or none (see the module text)."""
    if row.get("owner"):
        return "owner"
    if row.get("in_ref"):
        return "reference"
    if row.get("layer"):
        return "layer"
    if row.get("witness"):
        return "hat" if row.get("hat_cause") else "witness"
    return "none"


def looks_false(cls):
    return cls in ("hat", "none")


def tally(rows):
    """{added, owner, reference, layer, witness, hat, none, false, share}: the classes of `rows` and the share that
    looks false (None without rows)."""
    out = {k: 0 for k in ("owner", "reference", "layer", "witness", "hat", "none")}
    for r in rows:
        out[r["cls"]] += 1
    n = len(rows)
    fl = out["hat"] + out["none"]
    return {"added": n, **out, "false": fl, "share": (fl / n) if n else None}


def local_rise_db(env, t, sr, hop, near_s=(-0.001, 0.004), far_s=(-0.005, -0.0015)):
    """The re-rise test of the rule on an attack envelope `env` around the second stroke's onset `t` (seconds): the peak in
    [t - 1 ms, t + 4 ms] over the loudest block of [t - 5 ms, t - 1.5 ms] (dB; the stroke's own lobe before it counts as the
    base, so a second stroke 30 to 50 ms after a first one is tested against that one's ring). 0 outside the file."""
    b = lambda s: int(round((t + s) * sr / hop))  # noqa: E731
    n0, n1, f0, f1 = b(near_s[0]), b(near_s[1]) + 1, b(far_s[0]), b(far_s[1]) + 1
    if f0 < 0 or n1 > len(env) or n0 >= n1 or f0 >= f1:
        return 0.0
    return float(20 * np.log10(max(float(env[n0:n1].max()), 1e-12) / max(float(env[f0:f1].max()), 1e-12)))


def owner_found(added_times, owner, tol=OWNER_TOL_S):
    """How many of the owner's times have an added hit within `tol`."""
    return sum(1 for o in owner if near(added_times, o, tol))


def verdict(held, kit, owner_hit, owner_n):
    """The bar for the app. `held`: tally of the held-out added hits; `kit`: {found, targets, new_false}."""
    checks = {"owner_all": owner_hit == owner_n,
              "held_false": held["share"] is None or held["share"] <= MAX_FALSE_SHARE,
              "drt_found": kit["found"] >= kit["targets"], "drt_false": kit["new_false"] == 0}
    return {**checks, "pass": all(checks.values())}


# ---------------------------------------------------------------- the rule on one mic

def rows_for_track(x, sr, hits, sources, p, wx=None, lay=(), other=(), refs_t=(), owner=(), want_flags=True):
    """The added second strokes of one snare mic with their evidence. `sources`: {name: signal} of the other mics; the
    names that start with `hat` are the hat and cymbals (`hat_cause`). Rows: t, first, gap_ms, rise_db, level_db, bleed,
    in_ref, owner, layer, witness, hat_cause, cls (the gated ones are left out: the rule does not add them)."""
    env, hop = fp.attack_env(x, sr, p)
    envs = {m: fp.attack_env(s, sr, p)[0] for m, (s, srb) in sources.items() if srb == sr}
    gains = fp.transfer_gains(env, envs, hits, sr, hop)
    found = fp.second_onsets(x, sr, hits, p, env=env, gains=gains)
    hat_envs = {m: e for m, e in envs.items() if m.startswith("hat")}
    hat_gains = fp.transfer_gains(env, hat_envs, hits, sr, hop)
    hat_found = fp.second_onsets(x, sr, hits, {**p, "margin_db": HAT_MARGIN_DB}, env=env, gains=hat_gains)
    hat_at = {round(f["t"], 6): f["bleed"] for f in hat_found}
    kept = [f for f in found if not f["bleed"]]
    flags = fm.stroke_flags([f["t"] for f in kept], list(lay), x, sr, None, other=list(other)) if want_flags and kept else []
    wenv = fp.attack_env(wx, sr, p)[0] if (wx is not None and kept) else None
    oth = np.sort(np.asarray(list(other), float))
    rows = []
    for i, f in enumerate(kept):
        r = {k: float(f[k]) for k in ("t", "first", "gap_ms", "rise_db", "level_db")}
        r["bleed"] = False
        r["in_ref"] = near(refs_t, f["t"], REF_TOL_S)
        r["owner"] = near(owner, f["t"], OWNER_TOL_S)
        r["layer"] = bool(flags[i][0]) if flags else False
        r["own_rise"] = local_rise_db(env, f["t"], sr, hop)
        r["wit_rise"] = local_rise_db(wenv, f["t"], sr, hop) if wenv is not None else None
        r["witness"] = bool(wenv is not None and r["own_rise"] >= p["rise_db"] and r["wit_rise"] >= p["rise_db"]
                            and not (len(oth) and fm.near_any(oth, f["t"], fm.BLEED_S)))
        r["hat_cause"] = bool(hat_at.get(round(f["t"], 6), False))
        r["cls"] = classify(r)
        rows.append(r)
    return rows, len(found) - len(kept)


# ---------------------------------------------------------------- the seven songs

def do_songs(out, run, refs_dir, p, log=print):
    refs_all = json.loads((Path(refs_dir) / "refs.json").read_text())
    table = fm.songs()
    res = []
    for song, entry in refs_all.items():
        cfg = table[song]
        sig = fp.song_signals(song, cfg)
        run_d = json.loads((Path(run) / "run" / f"{fm.track_name(song)}_default.json").read_text())
        for mic, v in entry["mics"].items():
            events = run_d["events"][mic]
            hits = fm.outputs(events)
            off = fm.track_offset(v["onsets"], events)
            x, sr = sig[mic]
            srcs = {("hat " + m[6:] if m.startswith("extra ") else m): s for m, s in sig.items() if m != mic}
            refs_t = [t + off for t in v["onsets"]]
            if v["role"] == "snare":
                _, _, wx, lay, used, wit = fm.stroke_inputs(song, mic, "snare", cfg, v["onsets"], off, exclude=v.get("ref_file"))
                other = [r + off for m2, v2 in entry["mics"].items() if v2["role"] == "kick" for r in v2["onsets"]]
                rows, gated = rows_for_track(x, sr, hits, srcs, p, wx=wx, lay=lay, other=other, refs_t=refs_t,
                                             owner=OWNER.get(song, []))
                res.append({"song": song, "mic": mic, "role": "snare", "hits": len(hits), "gated": gated,
                            "layers": used, "witness": wit, "rows": rows})
            else:
                rows, gated = rows_for_track(x, sr, hits, srcs, p, refs_t=refs_t, want_flags=False)
                res.append({"song": song, "mic": mic, "role": v["role"], "hits": len(hits), "gated": gated, "rows": rows})
            log(f"{song} {mic} ({v['role']}): added {len(rows)}, gated {gated}")
    Path(out).mkdir(parents=True, exist_ok=True)
    (Path(out) / "songs.json").write_text(json.dumps(res))
    return res


# ---------------------------------------------------------------- the held-out sessions

def do_heldout(out, p, log=print):
    import roll_fill as rf
    out = Path(out)
    res = []
    for song, folder, stem, wit, kick, layers, src in HELDOUT:
        cfg = {"dir": folder}
        audio = rf.audio_of(folder, stem)
        if audio is None:
            log(f"{song}: no {stem}")
            continue
        tag = fm.track_name(song)
        ev = rf.run_track(out / "heldout", tag + "_snare", audio, "snare")
        kev = rf.run_track(out / "heldout", tag + "_kick", rf.audio_of(folder, kick), "kick")
        x, sr = ff.read_stem(cfg, stem)
        wx, wsr = ff.read_stem(cfg, wit)
        if wx is not None and wsr != sr:
            wx = None
        sources = {}
        for s in src:
            sx, ssr = ff.read_stem(cfg, s)
            if sx is not None:
                sources["hat " + s] = (sx, ssr)
        kx, ksr = ff.read_stem(cfg, kick)
        if kx is not None:
            sources["kick"] = (kx, ksr)
        hits = fm.outputs(ev)
        lay = []
        normal = hits
        for st in layers:
            lx, lsr = ff.read_stem(cfg, st)
            if lx is None:
                continue
            import learn_eval as le
            on = le.ref_onsets(lx, lsr, gap=0.005)
            lay += [float(t) - fm.layer_offset(normal, on) for t in on]
        rows, gated = rows_for_track(x, sr, hits, sources, p, wx=wx, lay=sorted(lay), other=fm.outputs(kev), refs_t=[])
        res.append({"song": song, "mic": stem, "role": "snare", "hits": len(hits), "gated": gated,
                    "witness": wit if wx is not None else None, "layers": len(layers), "rows": rows})
        log(f"{song} {stem}: {len(hits)} hits, added {len(rows)} (gated {gated}): {tally(rows)}")
    (out / "heldout.json").write_text(json.dumps(res))
    return res


# ---------------------------------------------------------------- the DRT kit

def do_kit(out, p, drt=DRT, log=print):
    """The DRT kit through the app (release `service_tool`, default settings) with the rule on top: the second strokes
    found (the `target_main` of 03_flams, the `target_second_*` of 09) and the false hits before and after."""
    import os
    tool = Path(os.environ.get("DRUMSPIKE_CALIB_TOOL") or fp.ROOT / "target" / "release" / "service_tool")
    model = "none"
    files = [(f, "kick" if f.name.startswith("07_") else "snare") for f in sorted(Path(drt).glob("0*.wav"))]
    d = {"files": {f.stem: str(f) for f, _ in files},
         "events": fp.run_project(tool, model, files, Path(out) / "scratch" / "drt", log)}
    rows = []
    for stem, events in d["events"].items():
        ref, lab = fp.kit_refs("drt", stem, fp.OUT)
        x, sr = fp.read_mono(d["files"][stem])
        hits = fm.outputs(events)
        found = fp.second_onsets(x, sr, hits, p)
        add = [e["t"] for e in fp.added_events(found)]
        sc = fp.kit_score(ref, lab, hits, sorted(hits + add), tol=0.005)
        main = [i for i, lb in enumerate(lab) if lb == "target_main" or lb.startswith("target_second")]
        before = {i for i, _ in fm.match(ref, hits, 0.005)}
        after = {i for i, _ in fm.match(ref, sorted(hits + add), 0.005)}
        rows.append({"file": stem, "targets": len(ref), "seconds": len(main),
                     "seconds_before": sum(1 for i in main if i in before), "seconds_after": sum(1 for i in main if i in after),
                     "added": len(add), "false_before": sc["before"]["false"], "false_after": sc["after"]["false"]})
        log(f"{stem}: second strokes {rows[-1]['seconds_before']} to {rows[-1]['seconds_after']} of {len(main)}, "
            f"false {sc['before']['false']} to {sc['after']['false']}")
    (Path(out) / "kit.json").write_text(json.dumps(rows))
    return rows


def kit_summary(rows):
    """{found, targets, new_false}: the 03_flams second strokes found and the new false hits of the other files."""
    flams = [r for r in rows if r["file"].startswith("03_")]
    other = [r for r in rows if not r["file"].startswith("03_")]
    return {"found": sum(r["seconds_after"] for r in flams), "targets": sum(r["seconds"] for r in flams),
            "new_false": sum(max(0, r["false_after"] - r["false_before"]) for r in other)}


# ---------------------------------------------------------------- stage B: the app's rule against this one

def py_found(x, sr, hits, sources, p, gate=False):
    """The rule as `rows_for_track` runs it, every stroke (the bleed ones too): [{t, bleed, ...}]. `sources`: {name:
    (signal, rate)}; a source at another rate is left out. `gate`: the app's late-peak gate (no stroke on a mic whose late
    peak is -3 dB or more: a snare bottom)."""
    env, hop = fp.attack_env(x, sr, p)
    if gate and fp.late_peaking(fp.late_peak_db(env, hits, sr, hop, p), fp.late_share(env, hits, sr, hop, p)):
        return []
    envs = {m: fp.attack_env(sx, sr, p)[0] for m, (sx, srb) in sources.items() if srb == sr}
    gains = fp.transfer_gains(env, envs, hits, sr, hop)
    return fp.second_onsets(x, sr, hits, p, env=env, gains=gains)


def py_late(x, sr, hits, p):
    """The late peak A of the mic (`fp.late_peak_db`), as the app gate reads it."""
    env, hop = fp.attack_env(x, sr, p)
    return fp.late_peak_db(env, hits, sr, hop, p)


def py_share(x, sr, hits, p):
    """The late share W of the mic (`fp.late_share`)."""
    env, hop = fp.attack_env(x, sr, p)
    return fp.late_share(env, hits, sr, hop, p)


def same_late(py, rs, tol=1e-6):
    """True when the late peaks agree (both None, or within `tol` dB)."""
    return (py is None and rs is None) or (py is not None and rs is not None and abs(py - rs) <= tol)


def parity_tool():
    root = Path(os.environ.get("CARGO_TARGET_DIR") or fp.ROOT / "target")
    return root / "release" / "parity"


def rust_found(tool, path, hits, sources, work, p):
    """`parity flam` on the same input: `sources` [(id, path)]. Returns the tool's answer."""
    work = Path(work)
    work.mkdir(parents=True, exist_ok=True)
    man = {"path": str(path), "hits": [float(t) for t in hits], "sources": [{"id": i, "path": str(q)} for i, q in sources],
           "rise_db": p["rise_db"], "margin_db": p["margin_db"], "min_gap_s": p["min_gap_s"]}
    (work / "flam.json").write_text(json.dumps(man))
    r = subprocess.run([str(tool), "flam", str(work / "flam.json"), str(work / "out.json")], capture_output=True, text=True,
                       stdin=subprocess.DEVNULL)
    if r.returncode != 0:
        raise RuntimeError(f"parity flam: {r.stderr[-400:]}")
    return json.loads((work / "out.json").read_text())


def same_strokes(py, rs, sr):
    """{py, rust, added_py, added_rust, equal, diff}: the strokes compared by sample and bleed flag."""
    key = lambda f: (int(round(f["t"] * sr)), bool(f["bleed"]))  # noqa: E731
    a, b = sorted(map(key, py)), sorted(map(key, rs))
    diff = sorted(set(a) ^ set(b))
    return {"py": len(a), "rust": len(b), "added_py": sum(1 for _, bl in a if not bl),
            "added_rust": sum(1 for _, bl in b if not bl), "equal": a == b, "diff": [[s / sr, bl] for s, bl in diff]}


def song_cases(run):
    """[(name, own path, hits, [(id, path)])]: the snare mic of each song with the run's output hits (the `flam` hits
    left out) and the song's other mics (the table and `fp.EXTRA`)."""
    out = []
    for song, cfg in fm.songs().items():
        f = Path(run) / "run" / f"{fm.track_name(song)}_default.json"
        if not f.is_file():
            continue
        events = json.loads(f.read_text())["events"]
        for mic, (stem, role) in cfg["mics"].items():
            if role != "snare":
                continue
            hits = fm.outputs([e for e in events[mic] if not e.get("flam")])
            src = [(m, fm.audio_path(cfg, st)) for m, (st, _) in cfg["mics"].items() if m != mic]
            src += [("extra " + st, fm.audio_path(cfg, st)) for st in fp.EXTRA.get(song, []) if fm.audio_path(cfg, st).is_file()]
            out.append((f"{song} {mic}", fm.audio_path(cfg, stem), hits, src))
    return out


def held_files(folder, stems):
    """[(stem, path)] of the stems (a split pair as its L and R files)."""
    out = []
    for st in stems:
        q = SAMPLES / folder / "Audio Files" / f"{st}.wav"
        if q.is_file():
            out.append((st, q))
        else:
            out += [(st + e, SAMPLES / folder / "Audio Files" / f"{st}{e}.wav") for e in (".L", ".R")
                    if (SAMPLES / folder / "Audio Files" / f"{st}{e}.wav").is_file()]
    return out


def held_cases(held=OUT / "heldout"):
    """The held-out snare mics with the stage A events (`<held>/work/<song>_snare/model/events.json`) and the kick,
    hat, overhead and ride files as sources."""
    out = []
    for song, folder, stem, _wit, kick, _layers, src in HELDOUT:
        f = Path(held) / "work" / f"{fm.track_name(song)}_snare" / "model" / "events.json"
        if not f.is_file():
            continue
        hits = fm.outputs(json.loads(f.read_text())["events"])
        out.append((f"{song} {stem}", SAMPLES / folder / "Audio Files" / f"{stem}.wav", hits, held_files(folder, [kick] + src)))
    return out


def do_parity(out, run, p, log=print):
    tool = parity_tool()
    if not tool.is_file():
        raise SystemExit(f"need the release parity tool ({tool})")
    res = []
    for name, path, hits, src in song_cases(run) + held_cases():
        x, sr = fp.read_mono(path)
        sources = {i: fp.read_mono(q) for i, q in src}
        py = py_found(x, sr, hits, sources, p)
        rs = rust_found(tool, path, hits, src, Path(out) / "scratch" / "parity", p)
        late = py_late(x, sr, hits, p)
        row = {"case": name, "hits": len(hits), "sources": len(src), "seconds": rs["seconds"],
               **same_strokes(py, rs["found"], sr), "late_py": late, "late_rust": rs["late_peak_db"], "share_py": py_share(x, sr, hits, p),
               "share_rust": rs["late_share"],
               "late_equal": same_late(late, rs["late_peak_db"]) and same_late(py_share(x, sr, hits, p), rs["late_share"])}
        res.append(row)
        log(f"{name}: {row['py']} strokes (added {row['added_py']}) python, {row['rust']} (added {row['added_rust']}) rust, "
            f"equal {row['equal']} {row['diff'][:4]}; late peak {row['late_py']} / {row['late_rust']}, share {row['share_py']} / {row['share_rust']} equal {row['late_equal']}")
    (Path(out) / "parity.json").write_text(json.dumps(res, indent=1))
    return res


def app_classes(song, mic, cfg, times, refs_dir, events, p):
    """The stage A classes of the app's flam hits `times` of a snare mic (owner, reference, layer, witness, none; the hat
    class needs the hat mics, which the yardstick project lacks: a witness hit is counted as witness). The reference is
    the fair one of `refs_dir` (`fix_metric.py score --ref fair`); the kick hits for the witness test are the reference's."""
    entry = json.loads((Path(refs_dir) / "refs.json").read_text())[song]
    v = entry["mics"][mic]
    off = fm.track_offset(v["onsets"], events)
    refs_t = [t + off for t in v["onsets"]]
    x, sr, wx, lay, _used, _wit = fm.stroke_inputs(song, mic, "snare", cfg, v["onsets"], off, exclude=v.get("ref_file"))
    other = sorted(r + off for v2 in entry["mics"].values() if v2["role"] == "kick" for r in v2["onsets"])
    flags = fm.stroke_flags(list(times), lay, x, sr, None, other=other) if times else []
    env, hop = fp.attack_env(x, sr, p)
    wenv = fp.attack_env(wx, sr, p)[0] if wx is not None else None
    oth = np.sort(np.asarray(other, float))
    out = []
    for i, t in enumerate(times):
        r = {"t": t, "in_ref": near(refs_t, t, REF_TOL_S), "owner": near(OWNER.get(song, []), t, OWNER_TOL_S),
             "layer": bool(flags[i][0]) if flags else False}
        own, wit = local_rise_db(env, t, sr, hop), (local_rise_db(wenv, t, sr, hop) if wenv is not None else None)
        r["witness"] = bool(wit is not None and own >= p["rise_db"] and wit >= p["rise_db"]
                            and not (len(oth) and fm.near_any(oth, t, fm.BLEED_S)))
        r["cls"] = classify(r)
        out.append(r)
    return out


def app_song_rows(app_run, p, refs_dir=REFS):
    """Per song: the app's `flam` hits of the snare mic against this rule on the other output hits, with the project's
    other mics (the table) as sources; and their stage A classes (`app_classes`)."""
    rows = []
    for song, cfg in fm.songs().items():
        f = Path(app_run) / "run" / f"{fm.track_name(song)}_default.json"
        if not f.is_file():
            continue
        d = json.loads(f.read_text())
        for mic, (stem, role) in cfg["mics"].items():
            if role != "snare":
                continue
            events = d["events"][mic]
            x, sr = fp.read_mono(fm.audio_path(cfg, stem))
            base = fm.outputs([e for e in events if not e.get("flam")])
            sources = {m: fp.read_mono(fm.audio_path(cfg, st)) for m, (st, _) in cfg["mics"].items() if m != mic}
            want = sorted(int(round(f["t"] * sr)) for f in py_found(x, sr, base, sources, p, gate=True) if not f["bleed"])
            got = sorted(int(round(e["t"] * sr)) for e in events if e.get("flam"))
            rec = (d.get("detector") or {}).get(mic) or {}
            times = sorted(e["t"] for e in events if e.get("flam"))
            classes = app_classes(song, mic, cfg, times, refs_dir, events, p) if Path(refs_dir, "refs.json").is_file() else []
            rows.append({"case": f"{song} {mic}", "song": song, "hits": len(base), "app": len(got), "python": len(want),
                         "equal": got == want, "diff": sorted(set(got) ^ set(want)), "flam_rule": rec.get("flam_rule"),
                         "record": rec.get("flam"), "classes": classes, "tally": tally(classes)})
    return rows


def run_steps(tool, model, files, work, env_extra, names):
    """One app project (`service_tool cli run`): `files` [(path, role)] analysed, `events_get` of `names`."""
    import shutil
    if work.exists():
        shutil.rmtree(work)
    (work / "p").mkdir(parents=True)
    note = {"kick": 36, "snare": 38, "tom": 47, "hat": 42, "cymbal": 49}
    steps = [{"command": "project_new", "args": {"name": "flam", "dir": str(work / "p")}},
             {"command": "tracks_add", "args": {"sid": "$sid", "paths": [str(f) for f, _ in files],
                                                "roles": [r for _, r in files], "notes": [note[r] for _, r in files]},
              "wait": True},
             {"command": "analyze_start", "args": {"sid": "$sid"}, "wait": True},
             {"command": "tracks_get", "args": {"sid": "$sid"}}]
    steps += [{"command": "events_get", "args": {"sid": "$sid", "name": n}} for n in names]
    (work / "steps.json").write_text(json.dumps(steps))
    env = {k: v for k, v in os.environ.items() if not k.startswith("DRUMSPIKE_")}
    env.update({"DRUMSPIKE_HOME": str(work / "h"), "DRUMSPIKE_PROJECTS": str(work / "p")})
    env.update(env_extra)
    r = subprocess.run([str(tool), "cli", "run", str(work / "steps.json")], capture_output=True, text=True, env=env,
                       stdin=subprocess.DEVNULL)
    rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
    bad = [x for x in rows if not x.get("ok", False)]
    if r.returncode != 0 or bad or len(rows) != len(steps):
        raise RuntimeError(f"{work.name}: exit {r.returncode} {bad[:1]} {r.stderr[-400:]}")
    tracks = rows[3]["result"]["tracks"]
    res = {n: {"events": rows[4 + i]["result"]["events"], "detector": tracks[n].get("detector")} for i, n in enumerate(names)}
    shutil.rmtree(work, ignore_errors=True)
    return res


def app_tool():
    tool = Path(os.environ.get("DRUMSPIKE_CALIB_TOOL") or fp.ROOT / "target" / "release" / "service_tool")
    model = "none"
    if not tool.is_file():
        raise SystemExit(f"need the release service_tool ({tool}) and the model ({model})")
    return tool, model


def app_held_rows(out, p, log=print):
    """The held-out sessions as one app project each (snare, kick, hat, overheads, ride), the rule on: the app's flam
    hits, this rule on the same hits and sources, and the pin `0` (no flam hit)."""
    tool, model = app_tool()
    rows = []
    for song, folder, stem, _wit, kick, _layers, src in HELDOUT:
        own = SAMPLES / folder / "Audio Files" / f"{stem}.wav"
        others = held_files(folder, [kick] + src)
        files = [(own, "snare")] + [(q, "kick" if st == kick else ("hat" if st.startswith("hh") else "cymbal"))
                                    for st, q in others]
        name = fm.track_name(stem)
        res = {}
        for cfg, env in (("on", {}), ("off", {"DRUMSPIKE_FLAM_RULE": "0"})):
            res[cfg] = run_steps(tool, model, files, Path(out) / "scratch" / f"{fm.track_name(song)}_{cfg}", env, [name])[name]
        events = res["on"]["events"]
        x, sr = fp.read_mono(own)
        base = fm.outputs([e for e in events if not e.get("flam")])
        sources = {st: fp.read_mono(q) for st, q in others}
        want = sorted(int(round(f["t"] * sr)) for f in py_found(x, sr, base, sources, p, gate=True) if not f["bleed"])
        got = sorted(int(round(e["t"] * sr)) for e in events if e.get("flam"))
        off_hits = fm.outputs(res["off"]["events"])
        row = {"case": f"{song} {stem}", "hits": len(base), "app": len(got), "python": len(want), "equal": got == want,
               "diff": sorted(set(got) ^ set(want)), "times": [g / sr for g in got],
               "off_same": sorted(off_hits) == sorted(base), "off_flam": sum(1 for e in res["off"]["events"] if e.get("flam")),
               "record": (res["on"]["detector"] or {}).get("flam")}
        rows.append(row)
        log(f"{row['case']}: app {row['app']} flam hits, python {row['python']}, equal {row['equal']}, off: same hits "
            f"{row['off_same']}, flam {row['off_flam']}; at {[round(t, 2) for t in row['times']]}")
    return rows


def app_kit_rows(out, drt=DRT, log=print):
    """The DRT kit through the app, one project per file, the rule on and pinned off: second strokes found and false
    hits per file (`fp.kit_score`, 5 ms)."""
    tool, model = app_tool()
    rows = []
    for f in sorted(Path(drt).glob("0*.wav")):
        role = "kick" if f.name.startswith("07_") else "snare"
        ev = {cfg: run_steps(tool, model, [(f, role)], Path(out) / "scratch" / f"kit_{f.stem}_{cfg}", env,
                             [fm.track_name(f.stem)])[fm.track_name(f.stem)]["events"]
              for cfg, env in (("on", {}), ("off", {"DRUMSPIKE_FLAM_RULE": "0"}))}
        ref, lab = fp.kit_refs("drt", f.stem, fp.OUT)
        on, off = fm.outputs(ev["on"]), fm.outputs(ev["off"])
        sc = fp.kit_score(ref, lab, sorted(off), sorted(on), tol=0.005)
        main = [i for i, lb in enumerate(lab) if lb == "target_main" or lb.startswith("target_second")]
        before = {i for i, _ in fm.match(ref, sorted(off), 0.005)}
        after = {i for i, _ in fm.match(ref, sorted(on), 0.005)}
        rows.append({"file": f.stem, "targets": len(ref), "seconds": len(main), "seconds_before": sum(1 for i in main if i in before),
                     "seconds_after": sum(1 for i in main if i in after), "added": sum(1 for e in ev["on"] if e.get("flam")),
                     "false_before": sc["before"]["false"], "false_after": sc["after"]["false"]})
        log(f"{f.stem}: added {rows[-1]['added']}, second strokes {rows[-1]['seconds_before']} to {rows[-1]['seconds_after']} "
            f"of {len(main)}, false {sc['before']['false']} to {sc['after']['false']}")
    return rows


def do_app(out, app_run, p, log=print):
    out = Path(out)
    res = {"songs": app_song_rows(app_run, p) if app_run else [], "heldout": app_held_rows(out, p, log), "kit": app_kit_rows(out, log=log)}
    for r in res["songs"]:
        log(f"{r['case']}: app {r['app']} flam hits, python {r['python']}, equal {r['equal']} {r['diff'][:4]}; "
            f"classes {r['tally']}")
    (out / "app.json").write_text(json.dumps(res, indent=1))
    return res


def do_real(out, app_run, off_run, log=print):
    """The three real sessions (P4-6b references, `~/.cache/drumspike/learn-eval/refs.json`): found, false, P, R, F per
    session and mic of the app with the rule (`--app-run`) and pinned off (`--off-run`), both `fix_metric.py run` folders;
    one offset per mic from the run without the rule, matched within 20 ms (`tools/rescue_real.py`)."""
    import rescue_real as rr
    refs = json.loads(rr.REFS.read_text())
    rows = []
    for song, mics in refs.items():
        runs = {}
        for cfg, d in (("on", app_run), ("off", off_run)):
            f = Path(d) / "run" / f"{fm.track_name(song)}_default.json"
            runs[cfg] = json.loads(f.read_text())["events"] if f.is_file() else None
        if runs["on"] is None or runs["off"] is None:
            log(f"{song}: no run")
            continue
        for mic, ref in mics.items():
            if mic in rr.SESSIONS[song]["excluded"] or mic not in runs["on"]:
                continue
            off_hits = fm.outputs(runs["off"][mic])
            on_hits = fm.outputs(runs["on"][mic])
            shift = rr.offset_of(ref, off_hits)
            shifted = (np.asarray(ref) + shift).tolist()
            sc = {cfg: rr.prf(shifted, h) for cfg, h in (("off", off_hits), ("on", on_hits))}
            rows.append({"session": song, "mic": mic, "flam": sum(1 for e in runs["on"][mic] if e.get("flam")), **sc})
            log(f"{song} {mic}: flam hits {rows[-1]['flam']}; found {sc['off']['tp']} to {sc['on']['tp']} of "
                f"{sc['on']['n_ref']}, false {sc['off']['fp']} to {sc['on']['fp']}, F {sc['off']['f']:.4f} to {sc['on']['f']:.4f}")
    (Path(out) / "real.json").write_text(json.dumps(rows, indent=1))
    return rows


# ---------------------------------------------------------------- report

def md(head, body):
    return "\n".join(["| " + " | ".join(head) + " |", "|" + "---|" * len(head)] + ["| " + " | ".join(map(str, r)) + " |" for r in body])


def do_report(out, log=print):
    out = Path(out)
    lines = []
    songs = json.loads((out / "songs.json").read_text()) if (out / "songs.json").is_file() else []
    snare = [r for r in songs if r["role"] == "snare"]
    if snare:
        body = []
        for r in snare:
            t = tally(r["rows"])
            body.append([r["song"], r["hits"], t["added"], t["owner"], t["reference"], t["layer"], t["witness"], t["hat"], t["none"],
                         t["false"], r["gated"]])
        allrows = [x for r in snare for x in r["rows"]]
        t = tally(allrows)
        body.append(["**all**", sum(r["hits"] for r in snare), t["added"], t["owner"], t["reference"], t["layer"], t["witness"],
                     t["hat"], t["none"], t["false"], sum(r["gated"] for r in snare)])
        lines += ["### Seven songs, snare added hits by class", "",
                  md(["song", "hits", "added", "owner", "reference", "layer", "witness", "hat", "none", "looks false", "gated"], body), ""]
        owner_hit = {s: owner_found([x["t"] for r in snare if r["song"] == s for x in r["rows"]], o) for s, o in OWNER.items()}
        lines += [f"Owner's 11 found: {sum(owner_hit.values())} ({owner_hit}). Looks false: {t['false']} of {t['added']}.", ""]
        other = [r for r in songs if r["role"] != "snare"]
        for role in ("kick", "toms"):
            rr = [x for r in other if r["role"] == role for x in r["rows"]]
            lines.append(f"{role}: added {len(rr)}, with a reference hit within 20 ms {sum(1 for x in rr if x['in_ref'])}.")
        lines.append("")
    if (out / "heldout.json").is_file():
        held = json.loads((out / "heldout.json").read_text())
        body = [[r["song"], r["mic"], r["hits"], tally(r["rows"])["added"], *[tally(r["rows"])[k] for k in ("layer", "witness", "hat", "none")],
                 tally(r["rows"])["false"]] for r in held]
        ht = tally([x for r in held for x in r["rows"]])
        body.append(["**all**", "", sum(r["hits"] for r in held), ht["added"], ht["layer"], ht["witness"], ht["hat"], ht["none"], ht["false"]])
        lines += ["### Held out, snare added hits by class", "",
                  md(["session", "mic", "hits", "added", "layer", "witness", "hat", "none", "looks false"], body), "",
                  f"Looks false: {ht['false']} of {ht['added']} (bar: at most {100 * MAX_FALSE_SHARE:.0f} %).", ""]
    if (out / "kit.json").is_file():
        kit = json.loads((out / "kit.json").read_text())
        body = [[r["file"], r["seconds"] or "-", f"{r['seconds_before']} to {r['seconds_after']}", r["added"],
                 f"{r['false_before']} to {r['false_after']}"] for r in kit]
        lines += ["### DRT kit (the app's events, the rule on top)", "",
                  md(["file", "second strokes", "found before to after", "added", "false before to after"], body), ""]
    (out / "report.md").write_text("\n".join(lines))
    log("\n".join(lines))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["songs", "heldout", "kit", "report", "all", "parity", "app", "real"])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--run", default=str(RUN))
    ap.add_argument("--refs", default=str(REFS))
    ap.add_argument("--rise", type=float, default=9.0)
    ap.add_argument("--margin", type=float, default=10.0)
    ap.add_argument("--min-gap", type=float, default=12.0)
    ap.add_argument("--app-run", default=None, help="app, real: a `fix_metric.py run` folder analysed with the rule on")
    ap.add_argument("--off-run", default=None, help="real: the same with `DRUMSPIKE_FLAM_RULE=0`")
    a = ap.parse_args(argv)
    p = params(a.rise, a.margin, a.min_gap)
    Path(a.out).mkdir(parents=True, exist_ok=True)
    if a.cmd == "parity":
        do_parity(a.out, a.run, p)
        return 0
    if a.cmd == "app":
        do_app(a.out, a.app_run, p)
        return 0
    if a.cmd == "real":
        do_real(a.out, a.app_run, a.off_run)
        return 0
    if a.cmd in ("songs", "all"):
        do_songs(a.out, a.run, a.refs, p)
    if a.cmd in ("heldout", "all"):
        do_heldout(a.out, p)
    if a.cmd in ("kit", "all"):
        do_kit(a.out, p)
    do_report(a.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
