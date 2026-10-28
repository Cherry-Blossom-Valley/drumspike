"""P4-21: the tom pair rule ("winner takes all", P4-20 candidate 1) as version 3 of the tom bleed rule: the replay.

    uv run --with numpy --with soundfile --with mido python tools/tom_pairs.py replay    # seven songs: marks, lost.csv
    uv run --with numpy --with soundfile --with mido python tools/tom_pairs.py heldout   # Summer Nights, Session E
    uv run --with numpy --with soundfile --with mido python tools/tom_pairs.py kit       # the synthetic two-tom kit
    uv run --with numpy --with soundfile --with mido python tools/tom_pairs.py equal --run DIR   # app run against the replay
    uv run --with numpy --with soundfile --with mido python tools/tom_pairs.py all

Rule (P4-20): of two output hits on different tom mics (different drums) within 12 ms, the one whose level is 6 dB or more
under the other's is bleed. The level of a hit is the peak of its mic in [t - 5 ms, t + 30 ms] against the median of the
same peak over the output hits of its own mic. Output hits: the hits the rule version 2 left in. Version 4 (#182): a mic
that carries `rise` per hit (the own-mic attack rise, `tom_fixes.stroke_rise`) keeps its marked hits with a rise of
`STROKE_DB` (10 dB) or more. `pair_marks` is the
reference of `drumspike_core::detect::tom_bleed::tom_pairs`; `equal` checks the app's events against it event by event.

Input: `~/.cache/drumspike/fix-metric-p419/` (the seven songs, rule version 2). Held-out sessions: `~/.cache/drumspike/sessions`
(read only). Outputs: `~/.cache/drumspike/tom-pairs/` (`replay.json`, `heldout.json`, `kit.json`, `lost.csv`, `report.md`).
"""
import argparse
import csv
import json
import os
import shutil
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import firm_fixes as ff  # noqa: E402
import fix_metric as fm  # noqa: E402
import tom_fixes as tf  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
OUT = Path.home() / ".cache" / "drumspike" / "tom-pairs"
SRC = Path.home() / ".cache" / "drumspike" / "fix-metric-p419"
WINDOW_S = 0.012     # two hits this close (or closer) are a pair
FLOOR_DB = -3.0      # a hit is marked only when its relative level is at most this (bleed is quiet on its own mic)
MARGIN_DB = 6.0      # the lower hit is bleed when its relative level is this far under the other's
EPS_S = 1e-9         # float slack of the window edge (the Rust rule uses the same)
STROKE_DB = ff.RISE_MIN_DB
FLOOR_STRICT_DB = -3.0   # the strict witness: an own stroke and a level within 3 dB of the mic's median
SAMPLE_TOL_S = 0.006
V2_TOOL = OUT / "service_tool_v2"    # the release tool of the base commit (rule version 2), for the held-out runs


# ---------------------------------------------------------------- the rule (pure; the tests cover these)

def median_levels(mics):
    """{mic: median level (dB) of its output hits}; `mics`: {mic: {"lvl": [dB], "output": [bool]}}. A mic without output
    hits has no median."""
    return {m: float(np.median([lv for lv, o in zip(v["lvl"], v["output"]) if o]))
            for m, v in mics.items() if any(v["output"])}


def pair_marks(mics, window_s=WINDOW_S, margin_db=MARGIN_DB, drums=None, floor_db=None):
    """The hits the pair rule marks: {mic: [bool per hit]}. `mics`: {mic: {"t": [s], "lvl": [dB], "output": [bool],
    "target": [bool], "rise": [dB]}}; `output` hits count (the rest is out already: marked by version 2 or cut); only a
    `target` hit can be marked (an edited hit stays). `drums`: {mic: drum}; two mics of one drum never pair (default:
    every mic is a drum). The relative level is `lvl` minus the median of the mic's output hits. Hits are visited
    loudest first; a hit is marked when a kept hit of another drum lies within `window_s` with a relative level at
    least `margin_db` higher; a marked hit does not protect another. A hit is marked only when its relative level is
    at most `floor_db`. Version 4 (#182): a mic with `rise` (the own-mic attack rise, `tom_fixes.stroke_rise`) keeps
    its marked hits with a rise of `STROKE_DB` (10 dB) or more - a real stroke has an attack, bleed has none."""
    drums = drums or {m: m for m in mics}
    floor_db = FLOOR_DB if floor_db is None else floor_db
    med = median_levels(mics)
    ev = [(v["lvl"][j] - med[m], v["t"][j], m, j, v["target"][j])
          for m, v in mics.items() if m in med for j in range(len(v["t"])) if v["output"][j]]
    ev.sort(key=lambda e: -e[0])
    kept = []
    marks = {m: [False] * len(v["t"]) for m, v in mics.items()}
    for rel, t, m, j, target in ev:
        keep = "rise" in mics[m] and mics[m]["rise"][j] >= STROKE_DB
        if target and rel <= floor_db and not keep and any(drums[k[2]] != drums[m] and abs(k[1] - t) <= window_s + EPS_S and k[0] - rel >= margin_db
                          for k in kept):
            marks[m][j] = True
        else:
            kept.append((rel, t, m, j))
    return marks


def partner_of(mics, marks, mic, j, window_s=WINDOW_S, margin_db=MARGIN_DB, drums=None):
    """The kept hit that took the place of the marked hit `(mic, j)`: (mic, index, relative level gap dB, dt ms), the
    loudest one; `mics` and `marks` as for `pair_marks`."""
    drums = drums or {m: m for m in mics}
    med = median_levels(mics)
    me = mics[mic]
    rel = me["lvl"][j] - med[mic]
    best = None
    for m, v in mics.items():
        if m == mic or drums[m] == drums[mic] or m not in med:
            continue
        for k in range(len(v["t"])):
            if not v["output"][k] or marks[m][k] or abs(v["t"][k] - me["t"][j]) > window_s + EPS_S:
                continue
            gap = v["lvl"][k] - med[m] - rel
            if gap >= margin_db and (best is None or gap > best[2]):
                best = (m, k, gap, (v["t"][k] - me["t"][j]) * 1000)
    return best


# ---------------------------------------------------------------- the seven songs

def song_mics(song, refs, run, table):
    """The pair input of one song from the app's events: ({mic: pair input}, {mic: signal}, rate, {mic: state})."""
    cfg = table[song]
    # every tom close mic the app analysed, also one without a reference (Session A t4): it is a winner for the others
    toms = [m for m, (_stem, role) in cfg["mics"].items() if role == "toms" and m in run["events"]]
    mics, sig, states = {}, {}, {}
    sr = None
    for m in toms:
        stem = cfg["mics"][m][0]
        x, sr_ = ff.read_stem(cfg, stem)
        sig[m] = x
        sr = sr_
        ev = sorted(run["events"][m], key=lambda e: e["t"])
        out = [not e.get("bleed") and not e.get("cut") for e in ev]
        mics[m] = {"t": [float(e["t"]) for e in ev], "lvl": [ff.peak_db(x, sr_, float(e["t"])) for e in ev],
                   "output": out, "target": [True] * len(ev), "ev": ev}
        if m in refs[song]["mics"]:
            states[m] = tf.track_state(refs[song]["mics"][m]["onsets"], run["events"][m])
    return mics, sig, sr, states


def replay_song(song, refs, run, table):
    """The pair rule on one song's events: per mic the marks, and the dropped hits split by the P4-16 match."""
    mics, sig, sr, states = song_mics(song, refs, run, table)
    marks = pair_marks({m: {k: v[k] for k in ("t", "lvl", "output", "target")} for m, v in mics.items()})
    res = {"marks": marks, "mics": {}, "lost": []}
    for m, v in mics.items():
        if m not in states:     # no reference: its marks cannot be scored
            res["mics"][m] = {"output": sum(v["output"]), "marked": int(sum(marks[m])), "false": 0, "matched": 0}
            continue
        st = states[m]
        # output index -> event index
        oi = [j for j, o in enumerate(v["output"]) if o]
        matched_out = {e for _, e in st["pairs"]}
        n_false = n_match = 0
        for k, j in enumerate(oi):
            if not marks[m][j]:
                continue
            if k in matched_out:
                n_match += 1
                p = partner_of({a: {x: b[x] for x in ("t", "lvl", "output", "target")} for a, b in mics.items()}, marks, m, j)
                med = median_levels(mics)
                res["lost"].append({"song": song, "mic": m, "t": v["t"][j], "rel_db": v["lvl"][j] - med[m],
                                    "partner": p[0] if p else "", "partner_gap_db": p[2] if p else None,
                                    "dt_ms": p[3] if p else None,
                                    "kind": ""})
            else:
                n_false += 1
        res["mics"][m] = {"output": len(oi), "marked": int(sum(marks[m])), "false": n_false, "matched": n_match}
    return res, mics, states


def classify_lost(lost, mics, states, marks):
    """Each lost real hit: `unison` when the hit that took its place is a matched hit too (two real strokes, one lost),
    `swap` when it is a false hit (the reference may have given the stroke to the wrong tom)."""
    for r in lost:
        pm = r["partner"]
        if not pm:
            r["kind"] = ""
            continue
        v = mics[pm]
        if pm not in states:
            r["kind"] = "unscored"
            continue
        k = min(range(len(v["t"])), key=lambda i: abs(v["t"][i] - (r["t"] + r["dt_ms"] / 1000)))
        oi = [j for j, o in enumerate(v["output"]) if o]
        matched_out = {e for _, e in states[pm]["pairs"]}
        r["kind"] = "unison" if (oi.index(k) in matched_out) else "swap"


def do_replay(out, src=SRC, log=print):
    out = Path(out)
    refs, runs = tf.load_inputs(src)
    table = fm.songs()
    res = {"songs": {}, "lost": []}
    for song in tf.SONGS:
        r, mics, states = replay_song(song, refs, runs[song], table)
        classify_lost(r["lost"], mics, states, r["marks"])
        res["songs"][song] = {"mics": r["mics"], "marks": {m: [int(b) for b in v] for m, v in r["marks"].items()}}
        res["lost"] += r["lost"]
        f = sum(v["false"] for v in r["mics"].values())
        m_ = sum(v["matched"] for v in r["mics"].values())
        log(f"{song}: marked false {f}, matched {m_}")
    out.mkdir(parents=True, exist_ok=True)
    (out / "replay.json").write_text(json.dumps(res))
    with open(out / "lost.csv", "w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["song", "mic", "time_s", "time_mm_ss", "level_vs_median_db", "winner_mic", "winner_over_it_db", "dt_ms", "kind"])
        for r in sorted(res["lost"], key=lambda r: (r["song"], r["mic"], r["t"])):
            w.writerow([r["song"], r["mic"], f"{r['t']:.3f}", f"{int(r['t'] // 60)}:{r['t'] % 60:06.3f}", f"{r['rel_db']:.1f}",
                        r["partner"], "" if r["partner_gap_db"] is None else f"{r['partner_gap_db']:.1f}",
                        "" if r["dt_ms"] is None else f"{r['dt_ms']:+.1f}", r["kind"]])
    return res


# ---------------------------------------------------------------- the held-out sessions (no tom references)

HELDOUT = {
    "Session I": {"dir": "Session I",
                                   "mics": {"k in": ("k in_03", "kick"), "s top": ("s top_02", "snare"),
                                            "t1": ("t 1_03", "toms"), "t2": ("t 2_03", "toms"), "t3": ("t 3_03", "toms")},
                                   "sample": []},
    "Session E": {"dir": "Session E",
                         "mics": {"k in": ("k in-AtA2", "kick"), "s top": ("s top-AtA2", "snare"),
                                  "t1": ("t1 hh-AtA2", "toms"), "t2": ("t2-AtA2", "toms"), "t3": ("t3-AtA2", "toms"),
                                  "t4": ("t4 ri-AtA2", "toms")},
                         "sample": ["t sample sd3_03"]},
}


def witness_rows(song, cfg, run, marks, mics, sig, sr, sample_on):
    """Per marked hit: the own mic's stroke test, whether another tom mic shows a louder stroke at that time, the sample
    onset within 6 ms, and the verdict (`bleed`: a louder stroke on another tom; `real`: a stroke on the own mic and none
    louder elsewhere; `none`: no stroke on the own mic)."""
    med = median_levels(mics)
    hp = {m: tf.highpass(x, sr) for m, x in sig.items()}
    rows = []
    for m, v in mics.items():
        for j, mk in enumerate(marks[m]):
            if not mk:
                continue
            t = v["t"][j]
            own = tf.stroke_rise(sig[m], hp[m], sr, t)[2]
            rel = v["lvl"][j] - med[m]
            louder = False
            for o, w in mics.items():
                if o == m or o not in med:
                    continue
                ro = tf.stroke_rise(sig[o], hp[o], sr, t)[2]
                if ro >= STROKE_DB and ff.peak_db(sig[o], sr, t) - med[o] > rel:
                    louder = True
            sm = tf.nearest_abs(sample_on, t) * 1000 if len(sample_on) else None
            verdict = "none" if own < STROKE_DB else ("bleed" if louder else "real")
            rows.append({"song": song, "mic": m, "t": t, "rel_db": rel, "rise_own": own, "louder_stroke": louder,
                         "sample_ms": sm, "verdict": verdict, "strict_real": bool(own >= STROKE_DB and rel >= FLOOR_STRICT_DB)})
    return rows


def do_heldout(out, log=print):
    out = Path(out)
    tool = Path(os.environ.get("DRUMSPIKE_CALIB_TOOL") or V2_TOOL)
    scratch = out / "scratch"
    res = {}
    for song, cfg in HELDOUT.items():
        if (out / "heldout" / "run" / f"{fm.track_name(song)}_default.json").is_file():
            run = fm.run_song(tool, out / "heldout", song, cfg, "default", {}, scratch)   # kept: no run
        else:
            fm.wait_for_room(log)
            fm.LOCK.write_text(f"tom_pairs heldout {song}\n")
            try:
                run = fm.run_song(tool, out / "heldout", song, cfg, "default", {}, scratch)
            finally:
                fm.LOCK.unlink(missing_ok=True)
        toms = [m for m, v in cfg["mics"].items() if v[1] == "toms"]
        mics, sig, sr = {}, {}, None
        for m in toms:
            x, sr = ff.read_stem(cfg, cfg["mics"][m][0])
            sig[m] = x
            ev = sorted(run["events"][m], key=lambda e: e["t"])
            mics[m] = {"t": [float(e["t"]) for e in ev], "lvl": [ff.peak_db(x, sr, float(e["t"])) for e in ev],
                       "output": [not e.get("bleed") and not e.get("cut") for e in ev], "target": [True] * len(ev)}
        marks = pair_marks(mics)
        sample_on = fm.read_reference(cfg, cfg["sample"])[0] if cfg["sample"] else []
        rows = witness_rows(song, cfg, run, marks, mics, sig, sr, sample_on)
        res[song] = {"rows": rows, "job": run["job"].get("bleed"),
                     "output": {m: int(sum(v["output"])) for m, v in mics.items()},
                     "marked": {m: int(sum(marks[m])) for m in mics}}
        log(f"{song}: marked {len(rows)}: " + ", ".join(f"{k} {sum(1 for r in rows if r['verdict'] == k)}"
                                                         for k in ("bleed", "real", "none")))
    shutil.rmtree(scratch, ignore_errors=True)
    (out / "heldout.json").write_text(json.dumps(res))
    return res


# ---------------------------------------------------------------- the synthetic two-tom kit (rule level)

def kit_session(seed=1, n_solo=40, n_unison=40, n_bleed=40):
    """Hits of two toms with known truth (levels in dB, the hit times are the truth): solo hits of each tom (the other mic
    hears them 14 to 22 dB lower), unison strokes (both toms struck together, the levels within 3 dB of each tom's typical
    level; each mic also hears the other drum at -14 dB), and the bleed hits as a model would add them (a hit on the quiet
    mic at each solo stroke). Returns ({mic: pair input}, truth {mic: [real bool per hit]})."""
    rng = np.random.default_rng(seed)
    typical = {"t1": -12.0, "t2": -9.0}
    mics = {m: {"t": [], "lvl": [], "output": [], "target": []} for m in typical}
    truth = {m: [] for m in typical}
    t = 1.0

    def add(m, tt, lvl, real):
        v = mics[m]
        v["t"].append(tt)
        v["lvl"].append(lvl)
        v["output"].append(True)
        v["target"].append(True)
        truth[m].append(real)

    for _ in range(n_solo):
        m, o = ("t1", "t2") if rng.random() < 0.5 else ("t2", "t1")
        lv = typical[m] + rng.uniform(-4, 4)
        add(m, t, lv, True)
        add(o, t + rng.uniform(-0.002, 0.002), lv - rng.uniform(14, 22), False)    # the bleed hit the model found there
        t += 0.4
    for _ in range(n_unison):
        a = typical["t1"] + rng.uniform(-3, 3)
        b = typical["t2"] + rng.uniform(-3, 3)
        add("t1", t, a, True)
        add("t2", t + rng.uniform(-0.003, 0.003), b, True)
        t += 0.4
    for _ in range(n_bleed):
        m, o = ("t1", "t2") if rng.random() < 0.5 else ("t2", "t1")
        lv = typical[m] + rng.uniform(-4, 4)
        add(m, t, lv, True)
        t += 0.4
    return mics, truth


def do_kit(out, log=print):
    out = Path(out)
    mics, truth = kit_session()
    marks = pair_marks(mics)
    lost = sum(1 for m in mics for mk, r in zip(marks[m], truth[m]) if mk and r)
    removed = sum(1 for m in mics for mk, r in zip(marks[m], truth[m]) if mk and not r)
    res = {"hits": {m: len(v["t"]) for m, v in mics.items()}, "bleed_in": sum(1 for m in truth for r in truth[m] if not r),
           "bleed_removed": removed, "real": sum(1 for m in truth for r in truth[m] if r), "real_lost": lost}
    log(f"kit: real {res['real']}, lost {lost}; bleed hits {res['bleed_in']}, removed {removed}")
    (out / "kit.json").write_text(json.dumps(res))
    return res


# ---------------------------------------------------------------- the app against the replay

def do_equal(out, run_dir, src=SRC, log=print):
    """The app's run (`tools/fix_metric.py run --out RUN_DIR`, default config, rule version 3) against the replay on the rule
    version 2 run: every event of every tom mic must have the flag the replay gives, and the same time."""
    refs, runs = tf.load_inputs(src)
    table = fm.songs()
    new = {s: json.loads((Path(run_dir) / "run" / f"{fm.track_name(s)}_default.json").read_text()) for s in refs}
    rows = {}
    for song in tf.SONGS:
        r, mics, _ = replay_song(song, refs, runs[song], table)
        same = total = 0
        bad = []
        for m, v in mics.items():
            ev_new = sorted(new[song]["events"][m], key=lambda e: e["t"])
            if len(ev_new) != len(v["t"]):
                bad.append((m, "count", len(ev_new), len(v["t"])))
                continue
            for j, e in enumerate(ev_new):
                total += 1
                want = bool(not v["output"][j] or r["marks"][m][j])
                got = bool(e.get("bleed") or e.get("cut"))
                if abs(e["t"] - v["t"][j]) < 1e-6 and want == got:
                    same += 1
                else:
                    bad.append((m, j, v["t"][j], want, got))
        rows[song] = {"same": same, "total": total, "bad": bad[:5]}
        log(f"{song}: {same} of {total}" + (f", first differences {bad[:3]}" if bad else ""))
    (Path(out) / "equal.json").write_text(json.dumps(rows))
    return rows


# ---------------------------------------------------------------- the yardstick before and after

NEW = Path.home() / ".cache" / "drumspike" / "fix-metric-p421b"


def compare_scores(old, new, config="default"):
    """Rows (song, role, old counts, new counts) of the default configuration of two `fix_metric.py score` outputs, and
    the totals per role: counts are (false, missed, moved, fixes)."""
    keys = ("false", "missed", "moved", "fixes")
    rows, tot = [], {}
    for song in old[config]:
        for role in fm.ROLES:
            a, b = old[config][song][role], new[config][song][role]
            rows.append((song, role, tuple(a[k] for k in keys), tuple(b[k] for k in keys)))
            for r in (role, "all"):
                t = tot.setdefault(r, [[0] * 4, [0] * 4])
                for i, k in enumerate(keys):
                    t[0][i] += a[k]
                    t[1][i] += b[k]
    return rows, tot


def do_compare(out, old_dir=SRC, new_dir=NEW, log=print):
    old = json.loads((Path(old_dir) / "scores.json").read_text())
    new = json.loads((Path(new_dir) / "scores.json").read_text())
    rows, tot = compare_scores(old, new)
    fmt = lambda c: f"{c[0]} / {c[1]} / {c[2]} = **{c[3]}**"  # noqa: E731
    body = [[s, r, fmt(a), fmt(b), a[3] - b[3]] for s, r, a, b in rows if r == "toms" or a != b]
    for r in (*fm.ROLES, "all"):
        a, b = tot[r]
        body.append([f"**all songs**", r if r != "all" else "**all roles**", fmt(a), fmt(b), a[3] - b[3]])
    text = ["### P4-21 yardstick: P4-19 (rule version 2) against P4-21 (version 3 with the floor), fixes (false / missed / moved)", "",
            fm.md_table(["song", "role", "P4-19", "P4-21", "saved"], body), ""]
    (Path(out) / "compare.md").write_text("\n".join(text))
    log("\n".join(text))
    return rows, tot


# ---------------------------------------------------------------- the report

def do_report(out, log=print):
    out = Path(out)
    L = ["# P4-21 tom pair rule: stage A", ""]
    if (out / "replay.json").is_file():
        res = json.loads((out / "replay.json").read_text())
        body = []
        tot = [0, 0, 0]
        for s, v in res["songs"].items():
            f = sum(x["false"] for x in v["mics"].values())
            m = sum(x["matched"] for x in v["mics"].values())
            body.append([s, f, m, f - m])
            tot = [tot[0] + f, tot[1] + m, tot[2] + f - m]
        body.append(["all", tot[0], tot[1], tot[2]])
        L += ["Seven songs (replay on the rule version 2 run):", "", fm.md_table(["song", "false dropped", "real dropped", "net"], body), ""]
    if (out / "heldout.json").is_file():
        res = json.loads((out / "heldout.json").read_text())
        body = []
        for s, v in res.items():
            rows = v["rows"]
            c = {k: sum(1 for r in rows if r["verdict"] == k) for k in ("bleed", "real", "none")}
            smp = sum(1 for r in rows if r["sample_ms"] is not None and r["sample_ms"] <= SAMPLE_TOL_S * 1000)
            body.append([s, sum(v["output"].values()), len(rows), c["bleed"], c["none"], c["real"],
                         f"{100 * c['real'] / max(1, len(rows)):.1f} %", smp if HELDOUT[s]["sample"] else "n/a"])
        L += ["Held-out sessions:", "", fm.md_table(["session", "output hits", "marked", "bleed (louder stroke elsewhere)",
                                                    "no stroke on the own mic", "looks real", "share", "sample within 6 ms"], body), ""]
    if (out / "kit.json").is_file():
        L += ["Kit: " + json.dumps(json.loads((out / "kit.json").read_text())), ""]
    (out / "report.md").write_text("\n".join(L) + "\n")
    log("\n".join(L))


def do_avgheld(out, base_tool, cand_tool, log=print):
    """P4-31 candidate C on the held-out sessions: the app with the shipped picks (`base_tool`) and with the tom avg-off run
    fill (`cand_tool`, env `DRUMSPIKE_P431_TOM_AVG_OFF=1`). Per tom mic: hits before and after, the added hits (`"roll"`
    flag) by the witness test of `witness_rows` (own stroke and no louder stroke on another tom: real; a louder stroke
    elsewhere: bleed; no stroke on the own mic: none). Writes `avgheld.json`."""
    out = Path(out)
    model = "none".resolve()
    res = {}
    for song, cfg in HELDOUT.items():
        runs = {}
        for name, tool, env in (("default", base_tool, {}), ("avgoff", cand_tool, {"DRUMSPIKE_P431_TOM_AVG_OFF": "1"})):
            fm.wait_for_room(log)
            runs[name] = fm.run_song(Path(tool), model, out / "avgheld", song, cfg, name, env, out / "scratch")
        toms = [m for m, v in cfg["mics"].items() if v[1] == "toms"]
        sig, sr = {}, None
        for m in toms:
            sig[m], sr = ff.read_stem(cfg, cfg["mics"][m][0])
        hp = {m: tf.highpass(x, sr) for m, x in sig.items()}
        med = {}
        for m in toms:
            lv = [ff.peak_db(sig[m], sr, float(e["t"])) for e in runs["avgoff"]["events"][m] if not e.get("bleed") and not e.get("cut")]
            med[m] = float(np.median(lv)) if lv else 0.0
        rows = []
        for m in toms:
            for e in runs["avgoff"]["events"][m]:
                if not e.get("roll"):
                    continue
                t = float(e["t"])
                own = tf.stroke_rise(sig[m], hp[m], sr, t)[2]
                rel = ff.peak_db(sig[m], sr, t) - med[m]
                louder = any(tf.stroke_rise(sig[o], hp[o], sr, t)[2] >= STROKE_DB and ff.peak_db(sig[o], sr, t) - med[o] > rel
                             for o in toms if o != m)
                rows.append({"song": song, "mic": m, "t": t, "rise_own": own, "rel_db": rel, "bleed_flag": bool(e.get("bleed")),
                             "verdict": "none" if own < STROKE_DB else ("bleed" if louder else "real")})
        res[song] = {"hits_before": {m: len(runs["default"]["events"][m]) for m in toms},
                     "hits_after": {m: len(runs["avgoff"]["events"][m]) for m in toms}, "added": rows}
        log(f"{song}: hits {sum(res[song]['hits_before'].values())} to {sum(res[song]['hits_after'].values())}, added {len(rows)}: "
            + ", ".join(f"{k} {sum(1 for r in rows if r['verdict'] == k)}" for k in ("real", "bleed", "none")))
    shutil.rmtree(out / "scratch", ignore_errors=True)
    (out / "avgheld.json").write_text(json.dumps(res))
    return res


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["replay", "heldout", "kit", "equal", "compare", "report", "all", "avgheld"])
    ap.add_argument("--base-tool", default=None, help="avgheld: the release service_tool with the shipped picks")
    ap.add_argument("--cand-tool", default=None, help="avgheld: the release service_tool with the P4-31 pin")
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--no-floor", action="store_true", help="replay, heldout: the version of 60a4391 (no level floor)")
    ap.add_argument("--run", default=None, help="equal: the fix_metric run folder of the app with rule version 3")
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    if a.no_floor:
        global FLOOR_DB
        FLOOR_DB = float("inf")
    if a.cmd in ("replay", "all"):
        do_replay(out)
    if a.cmd in ("heldout", "all"):
        do_heldout(out)
    if a.cmd in ("kit", "all"):
        do_kit(out)
    if a.cmd == "avgheld":
        do_avgheld(out, a.base_tool, a.cand_tool)
    if a.cmd == "equal":
        do_equal(out, a.run or str(Path.home() / ".cache" / "drumspike" / "fix-metric-p421b"))
    if a.cmd == "compare":
        do_compare(out)
    if a.cmd in ("report", "all"):
        do_report(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
