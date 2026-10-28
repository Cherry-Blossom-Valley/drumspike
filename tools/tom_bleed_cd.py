"""P4-14 stage 1c: two repairs of rule B, fitted on the sessions with independent per-tom references, held out on the rest.

    uv run python tools/tom_bleed_cd.py features   # the candidates' features per tom hit of all eight sessions
    uv run python tools/tom_bleed_cd.py score      # the fit (Session A, Session F), the held-out table, the verdict
    uv run python tools/tom_bleed_cd.py all
    uv run python tools/tom_bleed_cd.py app        # stage 2: the app's analysis with the rule on the eight sessions,
                                                   # against `rule_cd` on the same events and stage 1c at 9.2 dB

Needs the outputs of `tools/tom_bleed.py all` (the three stage 1 sessions) and `tools/tom_bleed_new.py all` (the
new sessions): the app's hits, the P2-6 flag, the labels. Rule B (stage 1) removes a hit when `level_db < a` and
`tom_rel_db < b`; it failed where a tom mic has (almost) no hits of its own drum (stage 1b): that mic's strong-hit level
is bleed, and a soft hit of another tom looks loud on it. The candidates keep the form `level_db < a and X < b`:

  C   X = tom_rel_c: tom_rel_db over the played tom mics only. A tom mic is played when its strong-hit level is at most
      PLAYED_GAP_DB (6 dB, P2-6's strong-hit margin, not fitted) under the median strong-hit level of the session's
      other tom mics.
  D   X = exc_d: the lowest excess over the other tom mics B of `L_A - L_B - G(B->A)`, L the tom-band peak in
      [-0.5, +8] ms at the hit, G(B->A) P2-6's calibrated transfer gain: the 10th percentile of `L_A - L_B` at B's strong
      hits where A lists no hit within 30 ms (all strong hits when fewer than 5)
  CD  X = exc_cd: D over the played tom mics only

Each also with the P2-6 flag (a hit goes when either removes it). `a` (or none) and `b` come from a quantile grid: the
best pooled tom F on the fit sessions with at most 1 % of the real tom hits lost on each of them. Accepted only with at
most 1 % lost on each independent session and on the held-out total, and a higher tom F overall. Session E is
reported apart (its reference does not fit its mics, stage 1b). Outputs: `~/.cache/drumspike/tom-bleed/cd/`.
"""
import argparse
import copy
import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import calib_kit as ck  # noqa: E402
import miss_causes as mc  # noqa: E402
import rescue_real as rr  # noqa: E402
import tom_bleed as tb  # noqa: E402
import tom_bleed_new as tn  # noqa: E402

OUT = tb.OUT / "cd"
FIT = ["Session A", "Session F"]
HELD = ["Session B", "Session C", "Session G", "Session D", "Session H"]
APART = ["Session E"]
PLAYED_GAP_DB = 6.0
QUIET_S = 0.030
PCT = 10.0
MIN_HITS = 5
NO_OTHER = 200.0
CANDIDATES = {"B": "tom_rel_db", "C": "tom_rel_c", "D": "exc_d", "CD": "exc_cd"}
OLD_SESSIONS = copy.deepcopy(tb.SESSIONS)


def sets():
    """[(session table, output folder, references)] of stage 1 and stage 1b."""
    return [(OLD_SESSIONS, tb.OUT, json.loads(tb.REFS.read_text())),
            (tn.SESSIONS, tn.OUT, json.loads((tn.OUT / "refs.json").read_text()))]


# ---------------------------------------------------------------- the candidates' measures

def played(ref, gap=PLAYED_GAP_DB):
    """{mic: True when its strong-hit level is at most `gap` dB under the median of the other mics' levels}."""
    out = {}
    for m, v in ref.items():
        others = [w for o, w in ref.items() if o != m]
        out[m] = not others or v >= float(np.median(others)) - gap
    return out


def transfer_gain(la, lb, quiet, pct=PCT, min_hits=MIN_HITS):
    """G(B->A): the `pct` percentile of `la - lb` at B's strong hits, over those where A is quiet (`quiet`) when there
    are at least `min_hits` of them, else over all; None with fewer than `min_hits` values (P2-6 `Transfer`)."""
    d = np.asarray(la, float) - np.asarray(lb, float)
    q = np.asarray(quiet, bool)
    use = d[q] if q.sum() >= min_hits else d
    return float(np.percentile(use, pct)) if len(use) >= min_hits else None


def tom_rel_over(lv, ref, m, others):
    """tom_rel_db of mic `m`'s hits over the mics `others`: own relative level minus the highest other one."""
    own = lv[m][m] - ref[m]
    if not others:
        return own + NO_OTHER
    return own - np.max([lv[m][o] - ref[o] for o in others], axis=0)


def excess_over(lv, gain, m, others):
    """The lowest `L_A - L_B - G(B->A)` over `others` with a gain, at mic `m`'s hits (NO_OTHER when none)."""
    vals = [lv[m][m] - lv[m][o] - gain[(o, m)] for o in others if gain.get((o, m)) is not None]
    return np.min(vals, axis=0) if vals else np.full(len(lv[m][m]), NO_OTHER)


def session_measures(signals, times, sr):
    """{mic: {"tom_rel_c", "exc_d", "exc_cd"} arrays at its hits}, plus the strong-hit levels, the played flags and
    the gains, for the tom close mics of one session."""
    sec = tb.tom_sections(sr)
    toms = list(signals)
    strong = {m: tb.strong_mask(signals[m], sr, times[m]) if len(times[m]) else np.zeros(0, bool) for m in toms}
    lv = {m: {o: tb.band_peaks_db(signals[o], sr, times[m], sec) for o in toms} for m in toms}
    ref = {m: float(np.median(lv[m][m][strong[m]])) if strong[m].any() else 0.0 for m in toms}
    play = played(ref)
    gain = {}
    for b in toms:
        st = np.asarray(times[b])[strong[b]]
        for a in toms:
            if a == b:
                continue
            ta = np.sort(np.asarray(times[a]))
            quiet = [not (len(ta) and float(np.min(np.abs(ta - t))) <= QUIET_S) for t in st]
            gain[(b, a)] = transfer_gain(lv[b][a][strong[b]], lv[b][b][strong[b]], quiet)
    out = {}
    for m in toms:
        others = [o for o in toms if o != m]
        pl = [o for o in others if play[o]]
        out[m] = {"tom_rel_c": tom_rel_over(lv, ref, m, pl), "exc_d": excess_over(lv, gain, m, others),
                  "exc_cd": excess_over(lv, gain, m, pl)}
    return out, ref, play, gain


# ---------------------------------------------------------------- stage 2: the product's rule (`tom_bleed.rs`)

THRESHOLD_DB = 9.2       # the controller's choice: 1.5 dB under the fitted 10.716
LOUDER_SHARE = 0.5       # played: fewer than half of the strong hits at a louder hit of another drum
SILENT_SHARE = 0.25      # edited to digital silence: silent at a quarter or more of the other drums' strong hits
SILENCE_DB = -100.0      # P2-6 `silence_db`
SHARED_SUM_DB = -26.0    # fix round 3: played pairs -29.0 to -61.2 dB, pairs with a bleed-only mic -7.1 to -22.5 dB
KEEP_GAP_DB = 8.0        # fix round 5: a shared pair keeps its direction only with its weaker mic this far under


def near(times, t, reach=QUIET_S):
    """True when `times` has a value within `reach` of `t`."""
    times = np.asarray(times, float)
    return bool(len(times)) and float(np.min(np.abs(times - t))) <= reach


def rule_cd(signals, times, sr, drums=None, threshold=THRESHOLD_DB):
    """Stage 2's rule CD (`rule_pass`) with fix round 3: a pair of mics whose summed transfer gains lie above
    SHARED_SUM_DB hears one drum about as loud on both ("shared"); when its weaker mic (the lower level) passes the
    level test (a hot preamp), the pair counts as one drum and the rule runs again. The result has "merged": [(a, b)]."""
    mics = list(signals)
    drums = dict(drums or {m: m for m in mics})
    r = rule_pass(signals, times, sr, drums, threshold)
    merged = merged_pairs(r, mics, drums)
    if merged:
        for a, b in merged:
            old = drums[b]
            for m in mics:
                if drums[m] == old:
                    drums[m] = drums[a]
        r = rule_pass(signals, times, sr, drums, threshold)
    r["merged"] = merged
    return r


def group_levels(st, drums, skip):
    """One level per drum not in `skip`: the lowest level of its mics with strong hits."""
    by = {}
    for m, v in st.items():
        if v["level_db"] is None or drums[m] in skip:
            continue
        by[drums[m]] = min(by.get(drums[m], v["level_db"]), v["level_db"])
    return list(by.values())


def merged_pairs(r, mics, drums, cut=None):
    """The shared pairs (in mic order, both gains present, G(a->b) + G(b->a) above SHARED_SUM_DB) whose weaker mic
    is less than KEEP_GAP_DB under the other drums without the pair (fix round 5)."""
    cut = SHARED_SUM_DB if cut is None else cut
    out = []
    for i, a in enumerate(mics):
        for b in mics[i + 1:]:
            if (a, b) not in r["gain"] or (b, a) not in r["gain"]:
                continue
            if r["gain"][(a, b)][0] + r["gain"][(b, a)][0] <= cut:
                continue
            la, lb = r["mics"][a]["level_db"], r["mics"][b]["level_db"]
            if la is None or lb is None:
                continue
            others = group_levels(r["mics"], drums, {drums[a], drums[b]})
            if not others or min(la, lb) > float(np.median(others)) - KEEP_GAP_DB:
                out.append((a, b))
    return out


def rule_pass(signals, times, sr, drums=None, threshold=THRESHOLD_DB):
    """Stage 2's rule CD, the reference of `crates/core/src/detect/tom_bleed.rs`: {"mics": {mic: stats}, "gain":
    {(b, a): (gain, values used, quiet only)}, "excess": {mic: array}, "bleed": {mic: bool array}, "off": None or
    ("silence" | "played", [mics])}. `drums`: {mic: drum}; mics of one drum never explain each other (default: every
    mic its own drum). Stage 1c's CD plus the verifier conditions: played also needs fewer than half of the strong hits
    at a louder hit of another drum (a listed hit within 30 ms whose tom-band level is higher), a mic without strong
    hits is not played, the rule is off when a mic is edited to digital silence or fewer than two drums are played."""
    mics = list(signals)
    drums = drums or {m: m for m in mics}
    sec = tb.tom_sections(sr)

    def other(a, b):
        return a != b and drums[a] != drums[b]

    times = {m: np.asarray(times[m], float) for m in mics}
    strong = {m: tb.strong_mask(signals[m], sr, times[m]) for m in mics}
    lv = {m: {o: tb.band_peaks_db(signals[o], sr, times[m], sec) for o in mics if o == m or other(m, o)} for m in mics}
    st = {}
    for m in mics:
        idx = np.nonzero(strong[m])[0]
        louder = sum(1 for j in idx if any(other(m, o) and near(times[o], times[m][j]) and lv[m][o][j] > lv[m][m][j]
                                           for o in mics))
        st[m] = {"strong": len(idx), "level_db": float(np.median(lv[m][m][idx])) if len(idx) else None,
                 "louder_share": louder / len(idx) if len(idx) else None}
    for a in mics:
        vals = np.concatenate([lv[b][a][strong[b]] for b in mics if other(a, b)] or [np.zeros(0)])
        share = float(np.mean(vals <= SILENCE_DB)) if len(vals) >= MIN_HITS else None
        st[a]["silent_share"] = share
        st[a]["silent"] = share is not None and share >= SILENT_SHARE
    for m in mics:
        lvl = st[m]["level_db"]
        others = group_levels(st, drums, {drums[m]})
        ok = lvl is not None and (not others or lvl >= float(np.median(others)) - PLAYED_GAP_DB)
        st[m]["level_ok"] = ok
        st[m]["played"] = ok and st[m]["louder_share"] < LOUDER_SHARE
    gain = {}
    for b in mics:
        for a in mics:
            if not other(a, b):
                continue
            d = lv[b][a][strong[b]] - lv[b][b][strong[b]]
            q = np.asarray([not near(times[a], t) for t in times[b][strong[b]]], bool)
            g = transfer_gain(lv[b][a][strong[b]], lv[b][b][strong[b]], q)
            if g is not None:
                gain[(b, a)] = (g, int(q.sum()) if q.sum() >= MIN_HITS else len(d), bool(q.sum() >= MIN_HITS))
    excess = {}
    for m in mics:
        vals = [lv[m][m] - lv[m][o] - gain[(o, m)][0] for o in mics
                if other(m, o) and st[o]["played"] and (o, m) in gain]
        excess[m] = np.min(vals, axis=0) if vals else np.full(len(times[m]), NO_OTHER)
    silent = [m for m in mics if st[m]["silent"]]
    played_m = [m for m in mics if st[m]["played"]]
    off = (("silence", silent) if silent else ("played", played_m) if len({drums[m] for m in played_m}) < 2 else None)
    bleed = {m: (excess[m] < threshold) & (off is None) for m in mics}
    return {"mics": st, "gain": gain, "excess": excess, "bleed": bleed, "off": off}


def do_features(out, log=print):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    rows, info, refs_all = [], {}, {}
    for table, src, refs in sets():
        tb.use_sessions(table)
        feats = json.loads((Path(src) / "features.json").read_text())
        for s, cfg in table.items():
            refs_all[s] = refs[s]
            toms = [m for m, v in cfg["mics"].items() if v[1] == "tom"]
            sig, times = {}, {}
            for m in toms:
                sig[m], sr = tb.read_mono(tb.audio_path(s, m))
                times[m] = np.asarray([t for t, _ in tb.load_events(src, s, m)[1]])
            meas, ref, play, gain = session_measures(sig, times, sr)
            info[s] = {"ref": ref, "played": play, "gain": {f"{b}->{a}": g for (b, a), g in gain.items()}}
            for m in toms:
                fr = [r for r in feats if r["session"] == s and r["mic"] == m]
                if len(fr) != len(times[m]) or any(abs(r["t"] - t) > 1e-5 for r, t in zip(fr, times[m])):
                    raise SystemExit(f"{s} {m}: feature rows do not match the events")
                for j, r in enumerate(fr):
                    rows.append(dict(r, **{k: float(v[j]) for k, v in meas[m].items()}))
            log(f"{s}: played {play}")
    (out / "rows.json").write_text(json.dumps(rows))
    (out / "info.json").write_text(json.dumps(info))
    (out / "refs.json").write_text(json.dumps(refs_all))
    return rows, refs_all


# ---------------------------------------------------------------- fit and score

def remover(rule, with_p26):
    def gone(r):
        return (with_p26 and r["p26"]) or (bool(rule) and tb.removes(rule, r))
    return gone


def lost_per_session(rows, gone, sessions):
    return {s: (sum(1 for r in rows if r["session"] == s and r["real"] and not r["excluded"] and gone(r)),
                sum(1 for r in rows if r["session"] == s and r["real"] and not r["excluded"])) for s in sessions}


def fit(rows, refs, feat, with_p26, sessions=FIT, cap=tb.MAX_LOST, n=30):
    """The rule `level_db < a and feat < b` (a may be absent) with the best pooled F on `sessions` and at most `cap` of
    the real hits lost on each of them; None when no rule keeps the cap."""
    dev = [r for r in rows if r["session"] in sessions and not r["excluded"]]
    ths = {f: tb.candidate_thresholds(dev, f, n) for f in ("level_db", feat)}
    cands = [[(feat, b)] for b in ths[feat]] + [[("level_db", a), (feat, b)] for a in ths["level_db"] for b in ths[feat]]
    best = None
    for rule in cands:
        gone = remover(rule, with_p26)
        lost = lost_per_session(dev, gone, sessions)
        if any(n_lost > cap * n_real for n_lost, n_real in lost.values()):
            continue
        _, _, pool = tb.evaluate(dev, refs, lambda r, g=gone: not g(r), sessions)
        key = (round(pool["f"], 6), -sum(v[0] for v in lost.values()), -len(rule))
        if best is None or key > best[0]:
            best = (key, rule)
    return best[1] if best else None


def stroke_cells(rows, refs, gone):
    out = {"real": {}, "false": {}}
    for r in rows:
        if r["real"] is None or r["excluded"] or not gone(r):
            continue
        side = out["real" if r["real"] else "false"]
        k = tb.stroke_kind(r, refs) or "single"
        side[k] = side.get(k, 0) + 1
    return {k: " / ".join(str(v.get(x, 0)) for x in ("unison", "flam", "double", "single")) for k, v in out.items()}


def do_score(out, log=print):
    out = Path(out)
    rows = json.loads((out / "rows.json").read_text())
    refs = json.loads((out / "refs.json").read_text())
    sessions = FIT + HELD + APART
    variants = {"base": ([], False), "P2-6": ([], True)}
    fits = {}
    for name, feat in CANDIDATES.items():
        for p in (False, True):
            rule = fit(rows, refs, feat, p)
            fits[name + (" + P2-6" if p else "")] = rule
            if rule:
                variants[name + (" + P2-6" if p else "")] = (rule, p)
    res, lines = {}, []
    head = ["variant", "rule"] + [s for s in sessions] + ["held out", "all but Session E", "strokes removed: real",
                                                          "false (unison / flam / double / single)"]
    body = []
    base_all = None
    for name, (rule, p) in variants.items():
        gone = remover(rule, p)
        per, ses, _ = tb.evaluate(rows, refs, lambda r, g=gone: not g(r), sessions)
        _, _, held = tb.evaluate(rows, refs, lambda r, g=gone: not g(r), HELD)
        _, _, allx = tb.evaluate(rows, refs, lambda r, g=gone: not g(r), FIT + HELD)
        lost = lost_per_session(rows, gone, sessions)
        held_lost = sum(lost[s][0] for s in HELD), sum(lost[s][1] for s in HELD)
        cells = [f"{tb.fmt(ses[s])}, lost {lost[s][0]}" for s in sessions]
        sel = [r for r in rows if r["session"] in FIT + HELD]
        st = stroke_cells(sel, refs, gone)
        body.append([name, tb.rule_text(rule) if rule else ("P2-6 flag" if p else "-")] + cells
                    + [f"{tb.fmt(held)}, lost {held_lost[0]} of {held_lost[1]}", tb.fmt(allx), st["real"], st["false"]])
        if name == "base":
            base_all = allx
        ok = (all(lost[s][0] <= tb.MAX_LOST * lost[s][1] for s in FIT)
              and held_lost[0] <= tb.MAX_LOST * held_lost[1] and allx["f"] > base_all["f"])
        res[name] = {"rule": rule, "p26": p, "sessions": ses, "lost": lost, "held": held, "held_lost": held_lost,
                     "all": allx, "accepted": ok if name != "base" else None, "strokes": st}
    no_fit = [k for k, v in fits.items() if v is None]
    lines += ["### Candidates fitted on Session A and Session F (found/false/F and real hits lost per session)", "",
              mc.md_table(head, body), ""]
    if no_fit:
        lines += [f"No rule keeps the cap on both fit sessions for: {', '.join(no_fit)}.", ""]
    acc = [k for k, v in res.items() if v["accepted"]]
    if acc:
        top = max(acc, key=lambda k: res[k]["all"]["f"])
        rule, p = variants[top]
        feat, b = rule[-1]
        sw = []
        for d in (-3.0, -1.5, 0.0, 1.5, 3.0):
            rr_ = rule[:-1] + [(feat, round(b + d, 3))]
            gone = remover(rr_, p)
            lost = lost_per_session(rows, gone, sessions)
            _, _, allx = tb.evaluate(rows, refs, lambda r, g=gone: not g(r), FIT + HELD)
            sw.append([tb.rule_text(rr_)] + [f"{lost[s][0]} of {lost[s][1]}" for s in FIT]
                      + [f"{sum(lost[s][0] for s in HELD)} of {sum(lost[s][1] for s in HELD)}", f"{allx['f']:.3f}"])
        uni = tn.union_check([r for r in rows if r["session"] in tn.SESSIONS], refs, rule)
        lines += [f"Threshold sweep of the best accepted candidate ({top}; real hits lost):", "",
                  mc.md_table(["rule"] + FIT + ["held out", "F, all but Session E"], sw), "",
                  f"Split-free check of {top} on the new sessions (a combined reference hit found by some tom mic before "
                  "and kept by none after):", "",
                  mc.md_table(["session", "tom reference hits", "found before", "kept after", "lost"],
                              [[k, u["refs"], u["found_before"], u["kept_after"], u["lost"]] for k, u in uni.items()]),
                  ""]
        res["best"] = {"name": top, "union": uni, "sweep": sw}
    lines += [f"Accepted (at most 1 % lost on Session A, Session F and the held-out total, F up): "
              f"{', '.join(acc) if acc else 'none'}.", ""]
    (out / "scores.json").write_text(json.dumps(res))
    (out / "report.md").write_text("\n".join(lines))
    log("\n".join(lines))
    return res


# ---------------------------------------------------------------- stage 2: the app's own events

APP_OUT = OUT / "app"
LOCK = Path.home() / ".cache" / "drumspike" / "measure.lock"
MAX_LOAD = 8.0


def wait_for_room(log=print):
    """Wait while another measuring agent holds the lock or the 1-minute load is 8 or more."""
    import os
    import time
    while LOCK.exists() or os.getloadavg()[0] >= MAX_LOAD:
        log(f"waiting: lock {LOCK.exists()}, load {os.getloadavg()[0]:.1f}")
        time.sleep(30)


def track_name(stem):
    """The app's track name of a file (`tracks.rs` `safe_name`): letters, digits and `-` stay, other runs become `_`."""
    import re
    return re.sub(r"[^A-Za-z0-9-]+", "_", stem).strip("_")


def app_mics(table, s):
    """[(mic, stem, role)] of the session's kick, snare and tom close mics (the rule's inputs and the unchanged check)."""
    return [(m, v[0], v[1]) for m, v in table[s]["mics"].items() if v[1] in ("kick", "snare", "tom")]


def run_app(tool, model, out, table, s, rule, refine=None):
    """The app's analysis of one session as one project (`service_tool cli run`, the tracks referenced, nothing copied)
    with `DRUMSPIKE_BLEED_RULE=rule` (and `DRUMSPIKE_REFINE_RULE=refine` when given): {"job": the final job, "detector": {mic: record}, "events": {mic: [...]}}. Written
    to `<out>/<session>_rule<rule>.json`; an existing file is kept (the run resumes)."""
    import os
    import shutil
    import subprocess
    dst = out / f"{rr.safe(s, 'rule' + str(rule))}.json"
    if dst.is_file():
        return json.loads(dst.read_text())
    tb.use_sessions(table)
    mics = app_mics(table, s)
    work = out / "run" / rr.safe(s, str(rule))
    if work.exists():
        shutil.rmtree(work)
    (work / "p").mkdir(parents=True)
    steps = [{"command": "project_new", "args": {"name": "bleed", "dir": str(work / "p")}},
             {"command": "tracks_add", "args": {"sid": "$sid", "paths": [str(tb.audio_path(s, m)) for m, _, _ in mics],
                                                "roles": [r for _, _, r in mics],
                                                "notes": [mc.ROLE_NOTE[r] for _, _, r in mics]}, "wait": True},
             {"command": "analyze_start", "args": {"sid": "$sid"}, "wait": True},
             {"command": "analyze_status", "args": {"sid": "$sid"}},
             {"command": "tracks_get", "args": {"sid": "$sid"}}]
    steps += [{"command": "events_get", "args": {"sid": "$sid", "name": track_name(stem)}} for _, stem, _ in mics]
    (work / "steps.json").write_text(json.dumps(steps))
    env = {**os.environ, "DRUMSPIKE_HOME": str(work / "h"), "DRUMSPIKE_PROJECTS": str(work / "p"),
           "DRUMSPIKE_BLEED_RULE": str(rule)}
    env.pop("DRUMSPIKE_REFINE_RULE", None)
    if refine is not None:
        env["DRUMSPIKE_REFINE_RULE"] = str(refine)
    r = subprocess.run([str(tool), "cli", "run", str(work / "steps.json")], capture_output=True, text=True, env=env)
    rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
    bad = [x for x in rows if not x.get("ok", False)]
    if r.returncode != 0 or bad or len(rows) != len(steps):
        raise RuntimeError(f"{s} rule {rule}: exit {r.returncode} {bad[:1]} {r.stderr[-400:]}")
    tracks = rows[4]["result"]["tracks"]
    res = {"job": rows[3]["result"], "detector": {m: tracks[track_name(stem)].get("detector") for m, stem, _ in mics},
           "events": {m: rows[5 + i]["result"] for i, (m, _, _) in enumerate(mics)}}
    dst.write_text(json.dumps(res))
    shutil.rmtree(work, ignore_errors=True)
    return res


def app_rows(table, refs, s, res):
    """Per tom hit of the app's events: the label, the app's mark and the Python reference's mark on the same events."""
    tb.use_sessions(table)
    cfg = table[s]
    toms = [m for m, v in cfg["mics"].items() if v[1] == "tom"]
    sig, times = {}, {}
    for m in toms:
        sig[m], sr = tb.read_mono(tb.audio_path(s, m))
        times[m] = np.asarray([e["t"] for e in res["events"][m]["events"]])
    py = rule_cd(sig, times, sr, drums={m: m for m in toms})
    rows = []
    for m in toms:
        excluded = m in cfg["excluded"]
        known = m in refs[s] and not excluded
        _, real = tb.label_hits(refs[s].get(m) or [], times[m]) if known else (0.0, [None] * len(times[m]))
        for j, e in enumerate(res["events"][m]["events"]):
            rows.append({"session": s, "mic": m, "t": e["t"], "real": real[j], "excluded": excluded,
                         "app": e.get("bleed") is True, "py": bool(py["bleed"][m][j]), "excess": float(py["excess"][m][j])})
    return rows, py


def do_app(out, refine=None, log=print):
    """Stage 2's measurement: the app's analysis of the eight sessions with the rule (version 1) and without it
    (version 0); the per-session table of base, the app's rule, the Python reference on the app's events and stage 1c
    at the same threshold; kick and snare compared between the two runs. `refine` pins the refinement version (stage
    1c's events are version 2; the app's default is 3)."""
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    raise SystemExit("#281 D16: this comparison moved to the feature branch; it does not run on main")
    tool, model = mc.find_tool(), ck.find_model()
    stage1c = json.loads((OUT / "rows.json").read_text())
    LOCK.parent.mkdir(parents=True, exist_ok=True)
    rows, ks, offs, agree = [], [], {}, {}
    refs_all = {}
    try:
        for table, _src, refs in sets():
            for s in table:
                if s not in FIT + HELD + APART:
                    continue
                refs_all[s] = refs[s]
                res = {}
                for rule in (1, 0):
                    wait_for_room(log)
                    LOCK.write_text(f"tom_bleed_cd app {s} rule {rule}\n")
                    try:
                        res[rule] = run_app(tool, model, out, table, s, rule, refine)
                    finally:
                        LOCK.unlink(missing_ok=True)
                    log(f"ran {s} rule {rule}")
                r, py = app_rows(table, refs, s, res[1])
                rows += r
                offs[s] = res[1]["job"].get("bleed")
                agree[s] = (sum(1 for x in r if x["app"] == x["py"]), len(r))
                for m, _, role in app_mics(table, s):
                    a, b = res[1]["events"][m]["events"], res[0]["events"][m]["events"]
                    base = [{k: v for k, v in e.items() if k != "bleed"} for e in a]
                    ks.append([s, m, role, len(a), sum(1 for e in a if e.get("bleed")), "yes" if base == b else "NO"])
    finally:
        LOCK.unlink(missing_ok=True)
    (out / "rows.json").write_text(json.dumps(rows))
    sessions = [s for s in FIT + HELD + APART if s in refs_all]
    keep = {"base": lambda r: True, "app": lambda r: not r["app"], "py": lambda r: not r["py"]}
    per = {k: tb.evaluate(rows, refs_all, f, sessions)[1] for k, f in keep.items()}
    old = [r for r in stage1c if r["session"] in sessions]
    per["1c"] = tb.evaluate(old, refs_all, lambda r: not (r["exc_cd"] < THRESHOLD_DB), sessions)[1]
    per["1c base"] = tb.evaluate(old, refs_all, lambda r: True, sessions)[1]

    def lost(rs, gone):
        return sum(1 for r in rs if r["real"] and not r["excluded"] and gone(r))

    body = []
    for s in sessions:
        rs = [r for r in rows if r["session"] == s]
        os_ = [r for r in old if r["session"] == s]
        d = (per["app"][s]["tp"] - per["1c"][s]["tp"], per["app"][s]["fp"] - per["1c"][s]["fp"])
        b = offs.get(s) or {}
        body.append([s, tb.fmt(per["1c base"][s]), tb.fmt(per["1c"][s]), f"{lost(os_, lambda r: r['exc_cd'] < THRESHOLD_DB)}",
                     tb.fmt(per["base"][s]), tb.fmt(per["app"][s]), str(lost(rs, lambda r: r["app"])),
                     f"{d[0]:+d} / {d[1]:+d}", f"{agree[s][0]} of {agree[s][1]}",
                     "on" if b.get("on") else f"off: {b.get('reason')}"])
    head = ["session", "1c base", "1c CD 9.2 dB", "1c real lost", "app base", "app rule", "app real lost",
            "app - 1c (found / false)", "app marks = Python on the app's events", "rule"]
    lines = [f"### Stage 2: the app's events (refinement version {refine or 3}) with the rule, against stage 1c at 9.2 dB",
             "",
             mc.md_table(head, body), "",
             "### Kick, snare and tom events with the rule (version 1) and without it (version 0)", "",
             mc.md_table(["session", "mic", "role", "hits", "marked", "same hits without the mark"], ks), ""]
    (out / "report.md").write_text("\n".join(lines))
    log("\n".join(lines))
    return per


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["features", "score", "all", "app"])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--refine", type=int, default=None, help="app: pin the refinement version (stage 1c: 2)")
    a = ap.parse_args(argv)
    if a.cmd == "app":
        do_app(a.out if a.out != str(OUT) else APP_OUT / (f"refine{a.refine}" if a.refine else ""), a.refine)
        return 0
    if a.cmd in ("features", "all"):
        do_features(a.out)
    if a.cmd in ("score", "all"):
        do_score(a.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
