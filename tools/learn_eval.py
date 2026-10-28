# /// script
# requires-python = ">=3.12"
# dependencies = ["numpy", "soundfile"]
# ///
"""P4-6b stage A: the Learn evaluation on the real sessions (`docs/migration/concept-learn-mode.md` 4.3).

    uv run --with numpy --with soundfile python tools/learn_eval.py refs|baseline|learn|tune|notthis|filter|all \
        [--samples ~/.cache/drumspike/sessions] [--cache ~/.cache/drumspike/learn-eval] [--tool <service_tool>]

Third-party imports: numpy and soundfile (both in the dev group of `pyproject.toml`). The Rust side is
`service_tool learn-eval` (`crates/devtools/src/bin/service_tool/learn_eval.rs`), DSP-only since #293
(no model: the candidates are the DSP detector's). The sessions in `~/.cache/drumspike/sessions` are read only. The reference times and every result stay in
the cache folder, never in git.

Steps:

  refs      the onsets of each reference track (the printed trigger and sample tracks: a clean signal,
            an energy rise plus an onset sample, `ref_onsets`); a combined tom reference (Can Not Get
            Enough, Session C) is split per tom mic: each hit goes to the tom mic with the strongest
            attack around it, relative to that mic's own loud level (`split_toms`)
  baseline  the candidates and the hits at every Sensitivity value 0 to 150 (re-picked from the kept
            activations), one global offset per pair (the median of matched differences within 50 ms,
            `offset_of`), P, R, F at +-20 ms one to one (`match_count`), FP + FN, and the oracle
            Sensitivity (the best F) per track; reference hits with no candidate within the window
            are listed separately and do not count (`reachable`)
  learn     Sensitivity 100, 5 "this" examples (the first 5 reference hits after 10 s that match a
            candidate), 2 "not" examples (the 2 strongest candidates with no reference hit within
            50 ms), the automatic threshold; per recipe (option A `patch-2`, the ablation `mfcc-2`)
  tune      a grid of block weights on Session A only (the dev set); the test sessions are not used
  timing    `track_learn_scores` on one 6-minute track (Session A t2) through `service_tool run`: cold
            (no `learn.bin`, a fresh process) and warm (the cache)
  notthis   P4-6c: the threshold rule of Not This. Per tom track at Sensitivity 50 the hits are split
            into real (a reference hit within 20 ms) and bleed (none within 50 ms); the three loudest
            bleed hits are three separate trials of one Shift+K, alone and with one K on a real hit
            after 10 s. Per threshold rule: how many real and bleed hits go grey (`grey_counts`;
            the marked hit always does). Counts are summed over the trials.
  filter    P4-6c: the app's filter path (`service_tool learn-eval` mode `filter`) with the scripted
            examples of `learn` and the suggested threshold, against the filter off: P, R, F per tom
            track at Sensitivity 50 and 100
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import soundfile as sf

REPO = Path(__file__).resolve().parent.parent
TOL = 0.020  # the metric's window (s)
FPS = 100.0  # the Learn scorer's frame grid (learn_score.rs FPS, 10 ms frames)
OFFSET_WINDOW = 0.050  # the offset's and the "not" example's window (s)
CANDIDATE_REACH = 0.030  # an example takes the candidate within this reach (learn.rs CANDIDATE_REACH)
VALUES = list(range(0, 151))
LEARN_SENSITIVITY = 100
N_THIS, N_NOT, THIS_AFTER = 5, 2, 10.0
GRID = [round(i * 0.01, 2) for i in range(30, 100)]

# The three sessions: the close mics (file stem, role) and the references (files, the mic or "toms").
SESSIONS = {
    "Session A": {
        "dir": "Session A",
        "mics": {"k in": ("k in_02", "kick"), "s top": ("s top_02", "snare"), "t1": ("t1 hh_02", "tom"),
                 "t2": ("t2_02", "tom"), "t3": ("t3_02", "tom"), "t4": ("t4 ri_02", "tom")},
        "refs": {"k in": ["k sampl trig_01"], "s top": ["s sampl trig_01"], "t1": ["t1 sampl_01"],
                 "t2": ["t2 sampl_01"], "t3": ["t3 sampl_01"], "t4": ["t4 sampl_01"]},
        "excluded": {"t4": "owner spot-check 2026-10-13: 3 ok, 9 unsure, 8 wrong"},
    },
    "Session B": {
        "dir": "Session B",
        "mics": {"k in": ("k in_01", "kick"), "s top": ("s top_01", "snare"), "t1": ("t1 HH_01", "tom"),
                 "t2": ("t2_01", "tom"), "t3": ("t3_01", "tom"), "t4": ("t4 Ri_01", "tom")},
        "refs": {"k in": ["k trigger_01"], "s top": ["s trigger_01"], "toms": ["t trigger_01"]},
        "excluded": {},
    },
    "Session C": {
        "dir": "Session C",
        "mics": {"k in": ("k in_02", "kick"), "s top": ("s top_02", "snare"), "t1": ("t 1_03", "tom"),
                 "t2": ("t 2_03", "tom"), "t3": ("t 3_03", "tom"), "t4": ("t 4_03", "tom")},
        "refs": {"k in": ["k samp 1_02.L", "k samp 1_02.R"], "s top": ["s samp 1_02.L", "s samp 1_02.R"],
                 "toms": ["t samp_03.L", "t samp_03.R"]},
        "excluded": {},
    },
}
DEV = "Session A"
TOMS = ["t1", "t2", "t3", "t4"]


# ------------------------------------------------------------------ signal helpers


def read_mono(paths: list[Path]) -> tuple[np.ndarray, int]:
    """The mean of all channels of all files (the L and R files of a split stereo track)."""
    xs, sr = [], None
    for p in paths:
        x, r = sf.read(str(p), dtype="float32", always_2d=True)
        xs.append(x.mean(axis=1))
        sr = r
    n = min(len(x) for x in xs)
    return np.mean([x[:n] for x in xs], axis=0).astype(np.float32), int(sr)


def frame_db(x: np.ndarray, sr: int, hop_s: float = 0.0005, win_s: float = 0.001) -> tuple[np.ndarray, float]:
    """Energy in dB of `win_s` windows every `hop_s` seconds (the window starts at the frame time)."""
    hop, win = max(1, round(hop_s * sr)), max(1, round(win_s * sr))
    p = np.concatenate([[0.0], np.cumsum(x.astype(np.float64) ** 2)])
    starts = np.arange(0, max(1, len(x) - win), hop)
    e = (p[starts + win] - p[starts]) / win
    return 10 * np.log10(e + 1e-20), hop / sr


def ref_onsets(x: np.ndarray, sr: int, rise_db: float = 12.0, floor_db: float = -45.0, gap: float = 0.020,
               look_s: float = 0.010) -> np.ndarray:
    """The onsets of a clean reference track. The signal is differentiated twice (+12 dB per octave,
    so the attack click stands out and the low cycles of a kick sample do not), then its energy in
    2 ms windows every 0.5 ms in dB. An onset is the first window that is `rise_db` or more over the
    loudest window of the `look_s` before it (2 ms back, so the rise itself is not compared with), and
    within `floor_db` of the loudest window of the track; the loudest earlier window, not the lowest,
    so the dips of a ringing low tail never look like a rise. Hits closer than `gap` keep the first."""
    y = np.diff(np.diff(x.astype(np.float64), prepend=0.0), prepend=0.0)
    e, step = frame_db(y.astype(np.float32), sr, hop_s=0.0005, win_s=0.002)
    if len(e) == 0:
        return np.zeros(0)
    lag, guard = max(1, round(look_s / step)), max(1, round(0.002 / step))
    pad = np.concatenate([np.full(lag + guard, e.min()), e])
    before = np.lib.stride_tricks.sliding_window_view(pad, lag).max(axis=1)[: len(e)]
    hot = (e - before >= rise_db) & (e >= e.max() + floor_db)
    out: list[float] = []
    last = -2
    for i in np.flatnonzero(hot):
        if i == last + 1:
            last = i
            continue
        last = i
        t = i * step
        if not out or t - out[-1] >= gap:
            out.append(t)
    return np.asarray(out)


SILENT_FLOOR_DB = -60.0   # P4-38a: a window this quiet is digital near-silence (a gated mic)
LOCAL_GATE_DB = 40.0      # a window this far under the mic's own loud level: the mic is gated at the hit


def split_toms(refs: np.ndarray, mics: dict[str, np.ndarray], sr: int, pre: float = 0.005,
               post: float = 0.025) -> tuple[dict[str, np.ndarray], list[float]]:
    """Each hit of a combined tom reference goes to the mic with the strongest attack around it: the
    loudest 1 ms frame in `[t - pre, t + post]`, relative to that mic's own loud level (its 99.9th
    percentile frame), so a mic with more gain does not win every hit. P4-38a: a mic whose window is
    silent (under an absolute -60 dBFS floor, or more than 40 dB under its own loud level - a gated
    mic) is skipped for the hit; a hit that no mic claims is dropped and returned as the second
    value (the caller stores it or counts it)."""
    levels = {}
    for name, x in mics.items():
        e, step = frame_db(x, sr)
        levels[name] = (e, step, float(np.percentile(e, 99.9)))
    out: dict[str, list[float]] = {name: [] for name in mics}
    dropped: list[float] = []
    for t in refs:
        best, arg = -np.inf, None
        for name, (e, step, loud) in levels.items():
            a, b = max(0, int((t - pre) / step)), min(len(e), int((t + post) / step) + 1)
            if b <= a:
                continue
            peak = float(e[a:b].max())
            if peak < SILENT_FLOOR_DB or loud - peak > LOCAL_GATE_DB:
                continue   # P4-38a: the mic is silent around the hit (a gated mic) - it cannot own the hit
            v = peak - loud
            if v > best:
                best, arg = v, name
        if arg is not None:
            out[arg].append(float(t))
        else:
            dropped.append(float(t))
    return {k: np.asarray(v) for k, v in out.items()}, dropped


# ------------------------------------------------------------------ metric helpers


def match_count(ref: np.ndarray, est: np.ndarray, tol: float = TOL) -> int:
    """The size of the largest one-to-one matching of sorted `ref` and `est` within `tol` (the greedy
    left-to-right pass is optimal for windows of one width, as mir_eval's matching)."""
    return len(match_pairs(ref, est, tol))


def match_pairs(ref: np.ndarray, est: np.ndarray, tol: float = TOL) -> list[tuple[int, int]]:
    """The greedy one-to-one matching itself: sorted `ref` and `est`, the pairs (i, j) within `tol`."""
    ref, est = np.sort(np.asarray(ref, float)), np.sort(np.asarray(est, float))
    pairs, i, j = [], 0, 0
    while i < len(ref) and j < len(est):
        d = est[j] - ref[i]
        if d < -tol:
            j += 1
        elif d > tol:
            i += 1
        else:
            pairs.append((i, j))
            i += 1
            j += 1
    return pairs


def missed_of(ref: np.ndarray, est: np.ndarray, tol: float = TOL) -> np.ndarray:
    """P4-38m: the reference hits with no output hit within `tol` (the greedy pass's leftovers)."""
    ref = np.sort(np.asarray(ref, float))
    paired = {i for i, _ in match_pairs(ref, est, tol)}
    return np.asarray([ref[i] for i in range(len(ref)) if i not in paired])


def reachable_split(missed: np.ndarray, ct: np.ndarray, tol: float = TOL) -> tuple[np.ndarray, np.ndarray]:
    """The missed hits split by whether a detection candidate exists within `tol` (the others no rule
    that works on candidates can find)."""
    missed = np.asarray(missed, float)
    if len(missed) == 0 or len(ct) == 0:
        return missed[:0], missed
    ct = np.asarray(ct, float)
    near = np.array([bool(np.min(np.abs(ct - t)) <= tol) for t in missed])
    return missed[near], missed[~near]


def prf(ref: np.ndarray, est: np.ndarray, tol: float = TOL) -> dict:
    """Precision, recall, F-measure and the work count FP + FN."""
    m = match_count(ref, est, tol)
    nr, ne = len(ref), len(est)
    p = m / ne if ne else (1.0 if nr == 0 else 0.0)
    r = m / nr if nr else 1.0
    f = 2 * p * r / (p + r) if p + r else 0.0
    return {"p": p, "r": r, "f": f, "tp": m, "fp": ne - m, "fn": nr - m, "work": (ne - m) + (nr - m)}


def nearest_diffs(ref: np.ndarray, est: np.ndarray, window: float = OFFSET_WINDOW) -> np.ndarray:
    """For each reference hit the difference to the nearest estimate within `window`."""
    est = np.sort(np.asarray(est, float))
    out = []
    for t in ref:
        i = np.searchsorted(est, t)
        best = None
        for j in (i - 1, i):
            if 0 <= j < len(est) and abs(est[j] - t) <= window and (best is None or abs(est[j] - t) < abs(best)):
                best = est[j] - t
        if best is not None:
            out.append(best)
    return np.asarray(out)


def offset_of(ref: np.ndarray, est: np.ndarray, window: float = OFFSET_WINDOW) -> tuple[float, int]:
    """The global offset of a reference against a mic's hits: the median of the matched differences
    within `window` (est minus ref), and the number of matches."""
    d = nearest_diffs(ref, est, window)
    return (float(np.median(d)) if len(d) else 0.0), len(d)


def reachable(ref: np.ndarray, cands: np.ndarray, tol: float = TOL) -> np.ndarray:
    """A mask of the reference hits that have a candidate within `tol` (the others are listed
    separately: no filter can find them)."""
    cands = np.sort(np.asarray(cands, float))
    ok = []
    for t in ref:
        i = np.searchsorted(cands, t)
        ok.append(any(0 <= j < len(cands) and abs(cands[j] - t) <= tol for j in (i - 1, i)))
    return np.asarray(ok, bool)


def pick_examples(ref: np.ndarray, cands: list[list[float]], after: float = THIS_AFTER, n_this: int = N_THIS,
                  n_not: int = N_NOT) -> tuple[list[float], list[float]]:
    """Section 4.3 step 4: the first `n_this` reference hits after `after` s that match a candidate
    within CANDIDATE_REACH (the candidate's time), and the `n_not` strongest candidates with no
    reference hit within OFFSET_WINDOW. `cands` rows are `[t, class, strength, v]`."""
    ct = np.asarray([c[0] for c in cands], float)
    this: list[float] = []
    for t in np.sort(ref):
        if t < after or len(ct) == 0:
            continue
        i = int(np.argmin(np.abs(ct - t)))
        if abs(ct[i] - t) <= CANDIDATE_REACH and ct[i] not in this:
            this.append(float(ct[i]))
        if len(this) == n_this:
            break
    rs = np.sort(np.asarray(ref, float))
    free = []
    for c in cands:
        i = np.searchsorted(rs, c[0])
        near = any(0 <= j < len(rs) and abs(rs[j] - c[0]) <= OFFSET_WINDOW for j in (i - 1, i))
        if not near:
            free.append(c)
    free.sort(key=lambda c: -c[2])
    not_: list[float] = []
    for c in free:
        if float(c[0]) not in not_:
            not_.append(float(c[0]))
        if len(not_) == n_not:
            break
    return this, not_


# ------------------------------------------------------------------ runs


@dataclass
class Ctx:
    samples: Path
    cache: Path
    tool: Path


def audio(ctx: Ctx, session: str, stem: str) -> Path:
    return ctx.samples / SESSIONS[session]["dir"] / "Audio Files" / f"{stem}.wav"


def job_id(session: str, mic: str) -> str:
    return f"{session}/{mic}"


def rust(ctx: Ctx, jobs: list[dict]) -> list[dict]:
    spec = {"cache": str(ctx.cache), "jobs": jobs}
    path = ctx.cache / "spec.json"
    path.write_text(json.dumps(spec))
    out = subprocess.run([str(ctx.tool), "learn-eval", str(path)], check=True, capture_output=True, text=True)
    rows = [json.loads(line) for line in out.stdout.splitlines() if line.strip()]
    for r in rows:
        if not isinstance(r, dict) or "error" in r:
            raise SystemExit(f"service_tool learn-eval: {r}")
    return rows


def cmd_refs(ctx: Ctx) -> dict:
    refs: dict = {}
    for s, cfg in SESSIONS.items():
        refs[s] = {}
        for key, stems in cfg["refs"].items():
            x, sr = read_mono([audio(ctx, s, st) for st in stems])
            on = ref_onsets(x, sr)
            if key == "toms":
                mics = {m: read_mono([audio(ctx, s, cfg["mics"][m][0])])[0] for m in TOMS}
                split, dropped = split_toms(on, mics, sr)
                if dropped:
                    print(f"{s}: {len(dropped)} tom hits dropped (no mic claims them: silent mics)")
                refs[s]["toms_combined"] = on.tolist()
                for m, v in split.items():
                    refs[s][m] = v.tolist()
            else:
                refs[s][key] = on.tolist()
        print(s, {k: len(v) for k, v in refs[s].items()})
    (ctx.cache / "refs.json").write_text(json.dumps(refs))
    return refs


def load_json(ctx: Ctx, name: str) -> dict:
    return json.loads((ctx.cache / name).read_text())


def mic_jobs(sessions: list[str]) -> list[tuple[str, str, str, str]]:
    return [(s, m, stem, role) for s in sessions for m, (stem, role) in SESSIONS[s]["mics"].items()]


def cmd_baseline(ctx: Ctx) -> dict:
    refs = load_json(ctx, "refs.json")
    jobs = []
    for s, m, stem, role in mic_jobs(list(SESSIONS)):
        w = str(audio(ctx, s, stem))
        jobs.append({"id": job_id(s, m), "wav": w, "role": role, "mode": "cands"})
        jobs.append({"id": job_id(s, m), "wav": w, "role": role, "mode": "baseline", "values": VALUES})
    rows = rust(ctx, jobs)
    res: dict = {}
    for s, m, _stem, _role in mic_jobs(list(SESSIONS)):
        cands = next(r["candidates"] for r in rows if r["id"] == job_id(s, m) and "candidates" in r)
        hits = next(r["hits"] for r in rows if r["id"] == job_id(s, m) and "hits" in r)
        ref = np.asarray(refs[s].get(m, []))
        off, n_off = offset_of(ref, np.asarray(hits[VALUES.index(LEARN_SENSITIVITY)]))
        shifted = ref + off
        ok = reachable(shifted, np.asarray(hits[VALUES.index(150)]))
        use = shifted[ok]
        per_v = [prf(use, np.asarray(h)) for h in hits]
        best = max(range(len(VALUES)), key=lambda i: (per_v[i]["f"], -per_v[i]["work"]))
        res[job_id(s, m)] = {
            "session": s, "mic": m, "n_ref": len(ref), "n_unreachable": int((~ok).sum()),
            "unreachable": (shifted[~ok]).round(3).tolist(), "offset_ms": off * 1000, "n_offset": n_off,
            "n_cands": len(cands), "oracle_v": VALUES[best], "oracle": per_v[best],
            "default": per_v[VALUES.index(50)], "at100": per_v[VALUES.index(LEARN_SENSITIVITY)],
            "ref_used": use.tolist(), "cands": cands,
            "curve": [[VALUES[i], round(per_v[i]["f"], 4), per_v[i]["work"]] for i in range(len(VALUES))],
        }
    (ctx.cache / "baseline.json").write_text(json.dumps(res))
    print(baseline_table(res))
    return res


def baseline_table(res: dict) -> str:
    lines = ["| Session | Mic | Ref hits | Unreachable | Offset ms | Candidates | Oracle v | P | R | F | FP+FN | F at 50 | F at 100 |",
             "|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for r in res.values():
        o = r["oracle"]
        lines.append(f"| {r['session']} | {r['mic']} | {r['n_ref']} | {r['n_unreachable']} | {r['offset_ms']:+.1f} | "
                     f"{r['n_cands']} | {r['oracle_v']} | {o['p']:.3f} | {o['r']:.3f} | {o['f']:.3f} | {o['work']} | "
                     f"{r['default']['f']:.3f} | {r['at100']['f']:.3f} |")
    return "\n".join(lines)


def learn_jobs(ctx: Ctx, base: dict, sessions: list[str], variants: list[dict]) -> list[dict]:
    jobs = []
    for s, m, stem, role in mic_jobs(sessions):
        b = base[job_id(s, m)]
        if not b["ref_used"]:
            continue
        this, not_ = pick_examples(np.asarray(b["ref_used"]), b["cands"])
        jobs.append({"id": job_id(s, m), "wav": str(audio(ctx, s, stem)), "role": role, "mode": "learn",
                     "sensitivity": LEARN_SENSITIVITY, "this": this, "not": not_, "variants": variants, "grid": GRID})
    return jobs


def score_learn(base: dict, rows: list[dict]) -> dict:
    res: dict = {}
    for r in rows:
        b = base[r["id"]]
        use = np.asarray(b["ref_used"])
        for v in r["variants"]:
            auto = prf(use, np.asarray(v["hits_auto"]))
            grid = [(th, prf(use, np.asarray(h))) for th, h in v["grid"]] or [(v["auto"], auto)]
            th_best, best = max(grid, key=lambda g: (g[1]["f"], -g[1]["work"]))
            res.setdefault(v["name"], {})[r["id"]] = {
                "auto_th": v["auto"], "auto": auto, "oracle_th": th_best, "oracle": best,
                "this_scores": v["this_scores"], "not_scores": v["not_scores"], "ms_score": v["ms_score"],
                "ms_features": v["ms_features"], "n_candidates": r["n_candidates"],
                "duration": r["duration"],
            }
    return res


VARIANTS = [
    {"name": "A, track centring (the app)", "recipe": "patch-2", "weights": [1.0, 1.0, 0.5], "centre": "track"},
    {"name": "A, bleed centring", "recipe": "patch-2", "weights": [1.0, 1.0, 0.5], "centre": "bleed"},
    {"name": "A, auto centring", "recipe": "patch-2", "weights": [1.0, 1.0, 0.5], "centre": "auto"},
    {"name": "A without centring", "recipe": "patch-2", "weights": [1.0, 1.0, 0.5], "centre": "none"},
    {"name": "C (mfcc-2), track centring", "recipe": "mfcc-2", "weights": [1.0, 1.0, 0.5], "centre": "track"},
]


UNMATCH_K = (1, 2, 3, 5)   # P4-38m: the clicks measured per example way


def added_by_frame(cands: list, scores_by_frame: dict, sens: float, th: float) -> list[float]:
    """P4-38m rule under test: a candidate BELOW the Sensitivity (`v > sens`) becomes a hit when its
    frame's score >= `th`. `scores_by_frame` maps the FPS frame to the score. One added time per
    frame (two candidates in one 10 ms frame are one hit)."""
    out = set()
    for c in cands:
        f = round(float(c[0]) * FPS)
        s = scores_by_frame.get(f)
        if s is not None and float(c[3]) > sens and s >= th:
            out.add(round(float(c[0]), 6))
    return sorted(out)


def unmatched_counts(ref: np.ndarray, hits0: np.ndarray, added: list[float], tol: float = TOL) -> dict:
    """The measurement's counts: recovered (a missed ref hit an added hit now matches, one to one),
    added_false (an added hit with no reference hit within tol), lost (a ref hit the output had
    matched before and does not any more; the rule only adds, so this stays 0)."""
    ref = np.sort(np.asarray(ref, float))
    matched0 = {i for i, _ in match_pairs(ref, hits0, tol)}
    out = np.sort(np.asarray(list(hits0) + list(added), float))
    matched1 = {i for i, _ in match_pairs(ref, out, tol)}
    missed = [float(ref[i]) for i in range(len(ref)) if i not in matched0]
    recovered = len(match_pairs(np.asarray(missed), np.asarray(added), tol))
    added_false = sum(1 for a in np.asarray(added, float) if not np.any(np.abs(ref - a) <= tol))
    return {"recovered": recovered, "added_false": added_false, "lost": len(matched0 - matched1)}


def unmatched_table(res: dict) -> str:
    """Per track and set: missed/reachable, then net = recovered - added_false per rule (the grid
    collapsed to its best and the three automatic rules)."""
    lines = []
    for jid, per in res.items():
        lines += [f"{per['session']} / {per['mic']} @ sens {per['sens']}: ref {per['n_ref']}, "
                  f"missed {per['n_missed']} (reachable {per['n_reachable']}, no candidate {per['n_unreachable']})",
                  "| set | rule | th | recovered | added_false | net | lost |",
                  "|---|---|---|---|---|---|", ]
        for name, sr in per["sets"].items():
            rules = {k: v for k, v in sr["rules"].items() if not k.startswith("grid-")}
            best_grid = max((v for k, v in sr["rules"].items() if k.startswith("grid-")),
                            key=lambda v: v["recovered"] - v["added_false"])
            rules["grid-best"] = best_grid
            for rname, c in sorted(rules.items()):
                lines.append(f"| {name} | {rname} | {c['th']:.2f} | {c['recovered']} | {c['added_false']} | "
                             f"{c['recovered'] - c['added_false']} | {c['lost']} |")
        lines.append("")
    return "\n".join(lines)


def cmd_unmatched(ctx: Ctx) -> dict:
    """P4-38m: can a Learn This example bring back MISSED tom hits (candidates below the Sensitivity
    that look like the example) without adding false hits? Per track, at the oracle Sensitivity and
    at 100: the missed and reachable reference hits, then two example ways (A: today's K on detected
    hits, B: Learn Unmatched - the missed reference time as the example) for k in UNMATCH_K, the rule
    under test at the threshold grid and three automatic rules."""
    base = load_json(ctx, "baseline.json")
    sessions = list(SESSIONS)

    # pass 1: the output hits at the two sensitivities per track
    jobs = []
    for s, m, stem, role in mic_jobs(sessions):
        b = base[job_id(s, m)]
        if not b["ref_used"]:
            continue
        jobs.append({"id": job_id(s, m), "wav": str(audio(ctx, s, stem)), "role": role,
                     "mode": "baseline", "values": sorted({b["oracle_v"], LEARN_SENSITIVITY})})
    hits_at: dict = {}
    values_of = {job_id(s, m): sorted({base[job_id(s, m)]["oracle_v"], LEARN_SENSITIVITY})
                 for s, m, _stem, _role in mic_jobs(sessions)}
    for r in rust(ctx, jobs):
        for n, v in enumerate(values_of[r["id"]]):
            hits_at[(r["id"], int(v))] = r["hits"][n]

    # pass 2: the scores per example set (one decode per track and sensitivity)
    jobs, meta = [], []
    for s, m, stem, role in mic_jobs(sessions):
        jid = job_id(s, m)
        b = base[jid]
        if not b["ref_used"]:
            continue
        ref = np.asarray(b["ref_used"])
        for sens in sorted({b["oracle_v"], LEARN_SENSITIVITY}):
            hits0 = np.asarray(hits_at[(jid, int(sens))])
            missed = missed_of(ref, hits0)
            reach, noreach = reachable_split(missed, np.asarray([c[0] for c in b["cands"]], float))
            sets = []
            for way in ("detected", "unmatched"):
                for k in UNMATCH_K:
                    if way == "detected":
                        this, _ = pick_examples(ref, b["cands"], after=0.0, n_this=k, n_not=0)
                    else:
                        this = [float(t) for t in missed[:k]]
                    sets.append({"name": f"{'A-detected' if way == 'detected' else 'B-unmatched'}-k{k}",
                                 "this": this, "not": []})
            meta.append((jid, s, m, int(sens), hits0, missed, reach, noreach))
            jobs.append({"id": f"{jid}@{sens}", "wav": str(audio(ctx, s, stem)), "role": role,
                         "mode": "learn", "sets": sets, "recipe": "patch-2", "sensitivity": float(sens)})
    rows = {r["id"]: r for r in rust(ctx, jobs)}

    res: dict = {}
    for jid, s, m, sens, hits0, missed, reach, noreach in meta:
        row = rows[f"{jid}@{sens}"]
        b = base[jid]
        ref = np.asarray(b["ref_used"])
        per: dict = {"session": s, "mic": m, "sens": sens, "n_ref": len(ref),
                     "n_missed": len(missed), "n_reachable": len(reach), "n_unreachable": len(noreach),
                     "sets": {}}
        for setrow in row["sets"]:
            if not setrow["this"]:
                continue   # no missed hits to example: the set is empty and the rule adds blind (skipped)
            name = setrow["name"]
            scores = {round(t * FPS): v for t, v in setrow["scores"]}
            ex = [v for v in setrow["this_scores"] if v is not None]
            rules: dict = {f"grid-{th:.2f}": th for th in GRID}
            rules["auto"] = float(setrow["auto"])
            if ex:
                rules["min-example"] = float(np.min(ex))
                rules["median-example"] = float(np.median(ex))
            out_rules = {}
            for rname, th in rules.items():
                added = added_by_frame(b["cands"], scores, float(sens), float(th))
                out_rules[rname] = {"th": th, **unmatched_counts(ref, hits0, added)}
            per["sets"][name] = {"auto": float(setrow["auto"]), "rules": out_rules}
        res[f"{jid}@{sens}"] = per
    (ctx.cache / "unmatched.json").write_text(json.dumps(res))

    # the bar check: the mean net after 3 clicks over the tom tracks, the max added per click
    bar = {}
    for key, per in res.items():
        if per["mic"] not in TOMS or per["n_missed"] == 0:
            continue
        for name, sr in per["sets"].items():
            k = int(name.rsplit("k", 1)[1])
            for rname, c in sr["rules"].items():
                if c["lost"]:
                    continue
                d = bar.setdefault(rname, {"nets": [], "max_added": 0.0})
                if k == 3:
                    d["nets"].append(c["recovered"] - c["added_false"])
                d["max_added"] = max(d["max_added"], c["added_false"] / k)
    lines = [unmatched_table(res), "Bar (toms, mean net after 3 clicks; max added per click):", ""]
    for rname, d in sorted(bar.items()):
        if d["nets"]:
            lines.append(f"- {rname}: mean net {float(np.mean(d['nets'])):+.2f}, max added/click {d['max_added']:.2f}")
    print("\n".join(lines))
    return res


def cmd_learn(ctx: Ctx, variants: list[dict] | None = None, sessions: list[str] | None = None) -> dict:
    base = load_json(ctx, "baseline.json")
    variants = variants or VARIANTS
    rows = rust(ctx, learn_jobs(ctx, base, sessions or list(SESSIONS), variants))
    res = score_learn(base, rows)
    (ctx.cache / "learn.json").write_text(json.dumps(res))
    print(learn_table(base, res))
    if set(sessions or SESSIONS) == set(SESSIONS):
        for name in res:
            print(summary(base, res, name), "\n")
    return res


def learn_table(base: dict, res: dict) -> str:
    lines = []
    for name, per in res.items():
        lines += [f"Variant {name}", "",
                  "| Session | Mic | Oracle F | Learn th | P | R | F | FP+FN | Oracle FP+FN | Best th | Best F |",
                  "|---|---|---|---|---|---|---|---|---|---|---|"]
        for jid, r in per.items():
            b, a = base[jid], r["auto"]
            lines.append(f"| {b['session']} | {b['mic']} | {b['oracle']['f']:.3f} | {r['auto_th']:.2f} | {a['p']:.3f} | "
                         f"{a['r']:.3f} | {a['f']:.3f} | {a['work']} | {b['oracle']['work']} | {r['oracle_th']:.2f} | "
                         f"{r['oracle']['f']:.3f} |")
        lines.append("")
    return "\n".join(lines)


def summary(base: dict, res: dict, variant: str) -> str:
    """The targets of section 4.3 for one variant."""
    per = res[variant]
    lines = [f"Targets, variant {variant}", ""]

    def toms(session: str, mics: list[str]) -> tuple[float, float, int, int]:
        ids = [job_id(session, m) for m in mics if job_id(session, m) in per]
        return (float(np.mean([per[i]["auto"]["f"] for i in ids])), float(np.mean([base[i]["oracle"]["f"] for i in ids])),
                sum(per[i]["auto"]["work"] for i in ids), sum(base[i]["oracle"]["work"] for i in ids))

    for s in SESSIONS:
        mics = [m for m in TOMS if m not in SESSIONS[s]["excluded"]]
        f, fo, w, wo = toms(s, mics)
        lines.append(f"- toms {s} ({', '.join(mics)}): mean F Learn {f:.3f}, oracle {fo:.3f} (diff {f - fo:+.3f}); "
                     f"FP+FN Learn {w}, oracle {wo}")
    test = [s for s in SESSIONS if s != DEV]
    ids = [job_id(s, m) for s in test for m in TOMS if job_id(s, m) in per]
    f = float(np.mean([per[i]["auto"]["f"] for i in ids]))
    fo = float(np.mean([base[i]["oracle"]["f"] for i in ids]))
    w, wo = sum(per[i]["auto"]["work"] for i in ids), sum(base[i]["oracle"]["work"] for i in ids)
    lines.append(f"- toms, test sessions together: mean F Learn {f:.3f}, oracle {fo:.3f} (target >= {fo + 0.10:.3f}); "
                 f"FP+FN Learn {w}, oracle {wo} (target <= {wo / 2:.0f})")
    for s in SESSIONS:
        for m in ("k in", "s top"):
            i = job_id(s, m)
            d = per[i]["auto"]["f"] - base[i]["oracle"]["f"]
            d100 = per[i]["auto"]["f"] - base[i]["at100"]["f"]
            lines.append(f"- {m} {s}: F Learn {per[i]['auto']['f']:.3f}, oracle {base[i]['oracle']['f']:.3f} ({d:+.3f}), "
                         f"Sensitivity 100 without Learn {base[i]['at100']['f']:.3f} ({d100:+.3f})")
    return "\n".join(lines)


def tune_variants() -> list[dict]:
    out = []
    for a in (0.0, 0.5, 1.0):
        for b in (0.0, 0.5, 1.0, 2.0):
            for e in (0.0, 0.5, 1.0):
                if a + b + e == 0:
                    continue
                out.append({"name": f"w{a}-{b}-{e}", "recipe": "patch-2", "weights": [a, b, e]})
    return out


def cmd_tune(ctx: Ctx) -> dict:
    """The weight grid on Session A only: the mean F at the automatic threshold over its tom tracks
    (t1 to t3), with the kick and snare F as the no-harm check."""
    base = load_json(ctx, "baseline.json")
    variants = tune_variants()
    jobs = learn_jobs(ctx, base, [DEV], variants)
    for j in jobs:
        j["grid"] = []  # the automatic threshold only
    rows = rust(ctx, jobs)
    res = score_learn(base, rows)
    ranked = []
    for name, per in res.items():
        toms = [per[job_id(DEV, m)]["auto"]["f"] for m in ("t1", "t2", "t3") if job_id(DEV, m) in per]
        ks = [per[job_id(DEV, m)]["auto"]["f"] - base[job_id(DEV, m)]["oracle"]["f"] for m in ("k in", "s top")]
        ranked.append((float(np.mean(toms)), min(ks), name))
    ranked.sort(reverse=True)
    (ctx.cache / "tune.json").write_text(json.dumps({"ranked": ranked, "res": res}))
    for row in ranked[:15]:
        print(f"{row[2]:24s} toms F {row[0]:.3f}  kick/snare worst dF {row[1]:+.3f}")
    return res


def run_script(ctx: Ctx, steps: list[dict], name: str) -> list[dict]:
    path = ctx.cache / f"{name}.json"
    path.write_text(json.dumps(steps))
    out = subprocess.run([str(ctx.tool), "run", str(path)], check=True, capture_output=True, text=True)
    return [json.loads(line) for line in out.stdout.splitlines() if line.strip()]


def cmd_timing(ctx: Ctx, session: str = DEV, mic: str = "t2") -> dict:
    """The time of `track_learn_scores` on one 6-minute track through the app's commands: a project
    in the cache folder, one analysis (the model keeps the activations), the scripted examples as
    marks, then the scores: built (the audio decoded by the analysis), from the cache; then a fresh
    process without `learn.bin` (cold: decode and features) and the same again (warm)."""
    import shutil

    base = load_json(ctx, "baseline.json")
    b = base[job_id(session, mic)]
    this, not_ = pick_examples(np.asarray(b["ref_used"]), b["cands"])
    wav = audio(ctx, session, SESSIONS[session]["mics"][mic][0])
    home = ctx.cache / "timing"
    shutil.rmtree(home, ignore_errors=True)
    home.mkdir(parents=True)
    rows = run_script(ctx, [{"cmd": "project_new", "args": {"name": "timing", "dir": str(home)}},
                            {"cmd": "tracks_add", "args": {"sid": "$sid", "paths": [str(wav)]}},
                            {"cmd": "project_save", "args": {"sid": "$sid"}}], "timing0")
    added = rows[1]["answer"]["added"][0]
    track = added if isinstance(added, str) else added["name"]
    pdir = rows[0]["answer"]["folder"]
    marks = [{"cmd": "track_learn_mark", "args": {"sid": "$sid", "name": track, "kind": "this", "hit": t}} for t in this]
    marks += [{"cmd": "track_learn_mark", "args": {"sid": "$sid", "name": track, "kind": "not", "hit": t}} for t in not_]
    score = {"cmd": "track_learn_scores", "args": {"sid": "$sid", "name": track}}
    r1 = run_script(ctx, [{"cmd": "project_open", "args": {"path": pdir}},
                          {"cmd": "track_params_set", "args": {"sid": "$sid", "name": track,
                                                               "params": {"sensitivity": LEARN_SENSITIVITY}}},
                          {"cmd": "analyze_start", "args": {"sid": "$sid"}}, {"cmd": "wait_analysis", "args": {"sid": "$sid"}},
                          *marks, score, score, {"cmd": "project_save", "args": {"sid": "$sid"}}], "timing1")
    learn_bin = Path(pdir) / "Analysis" / track / "learn.bin"
    learn_bin.unlink()
    r2 = run_script(ctx, [{"cmd": "project_open", "args": {"path": pdir}}, score, score], "timing2")
    out = {"track": track, "built_after_analysis": r1[-3]["answer"], "hit": r1[-2]["answer"],
           "cold": r2[-2]["answer"], "warm": r2[-1]["answer"]}
    for k in ("built_after_analysis", "hit", "cold", "warm"):
        a = out[k]
        if "error" in a or a.get("state") != "ok":
            raise SystemExit(f"timing {k}: {a}")
        print(f"{k:22s} cache {a['cache']:5s} ms {a['ms']}  candidates {len(a['candidates'])}  auto {a['auto']}")
    (ctx.cache / "timing.json").write_text(json.dumps({k: (v if k == "track" else {"ms": v["ms"], "cache": v["cache"]})
                                                       for k, v in out.items()}))
    return out


# ------------------------------------------------------------------ P4-6c


NOT_SENS = 50
N_TRIALS = 3


def split_hits(hits: np.ndarray, ref: np.ndarray) -> tuple[list[float], list[float]]:
    """The hits with a reference hit within TOL (real) and those with none within OFFSET_WINDOW
    (bleed); a hit in between is neither."""
    real, bleed = [], []
    for t in hits:
        d = float(np.min(np.abs(ref - t))) if len(ref) else np.inf
        if d <= TOL:
            real.append(float(t))
        elif d > OFFSET_WINDOW:
            bleed.append(float(t))
    return real, bleed


def nearest_score(scores: list[list[float]], t: float, reach: float = CANDIDATE_REACH) -> float | None:
    """The score of the candidate nearest to `t` within `reach` (rows `[t, score]`)."""
    best, bd = None, reach + 1e-9
    for ct, sc in scores:
        d = abs(ct - t)
        if d <= bd:
            best, bd = sc, d
    return best


def grey_counts(real: list[float], bleed: list[float], hits: list[float], scores: list[list[float]], th: float,
                not_t: list[float], this_t: list[float]) -> tuple[int, int]:
    """How many real and bleed hits go grey at threshold `th`: a hit whose candidate scores under it,
    and the hit nearest to each "not" example; a hit within reach of a "this" example never does
    (`learn_filter::filter_candidates`)."""
    hs = np.asarray(hits, float)
    marked = set()
    for n in not_t:
        if len(hs):
            j = int(np.argmin(np.abs(hs - n)))
            if abs(hs[j] - n) <= CANDIDATE_REACH:
                marked.add(float(hs[j]))

    def grey(t: float) -> bool:
        if any(abs(t - x) <= CANDIDATE_REACH for x in this_t):
            return False
        if t in marked:
            return True
        sc = nearest_score(scores, t)
        return sc is not None and sc < th

    return sum(grey(t) for t in real), sum(grey(t) for t in bleed)


def two_means(v: list[float]) -> float:
    """The boundary of a 2-means split of `v` seeded at its ends."""
    s = np.asarray(v, float)
    a, b = float(s.min()), float(s.max())
    for _ in range(30):
        m = (a + b) / 2
        lo, hi = s[s < m], s[s >= m]
        if not len(lo) or not len(hi):
            break
        a, b = float(lo.mean()), float(hi.mean())
    return (a + b) / 2


def rule_thresholds(hit_scores: list[float], auto: float, suggest: float, not_scores: list[float]) -> dict[str, float]:
    """The candidate threshold rules of Not This, from the scores of the candidates of the hits."""
    med = float(np.median(hit_scores))
    mad = float(np.median(np.abs(np.asarray(hit_scores) - med)))
    return {
        "0.49 cap (before P4-6c)": min(0.49, max(auto, max(not_scores + [0.0]) + 0.02)),
        "median of the hits": med,
        "median - 3 MAD": med - 3 * 1.4826 * mad,
        "2-means of the hits": two_means(hit_scores),
        "stage A automatic": auto,
        "0.5 (more like not than this)": 0.5,
        "0.4 (by 0.2 or more)": 0.4,
        "0.25 (similarity to not >= 0.5)": 0.25,
        "0.05 (near copies only)": 0.05,
        "P4-6c suggestion (the app)": suggest,
    }


def cmd_notthis(ctx: Ctx) -> dict:
    base = load_json(ctx, "baseline.json")
    tracks = [(s, m) for s in SESSIONS for m in TOMS if m not in SESSIONS[s]["excluded"]]
    rows = rust(ctx, [{"id": job_id(s, m), "wav": str(audio(ctx, s, SESSIONS[s]["mics"][m][0])), "role": "tom",
                       "mode": "baseline", "values": [NOT_SENS]} for s, m in tracks])
    hits = {r["id"]: np.asarray(r["hits"][0]) for r in rows}
    trials = []
    for s, m in tracks:
        jid = job_id(s, m)
        b = base[jid]
        ref, cands = np.asarray(b["ref_used"]), b["cands"]
        ct = np.asarray([c[0] for c in cands])
        real, bleed = split_hits(hits[jid], ref)
        if not real or not bleed:
            continue

        def cand(t: float) -> int | None:
            i = int(np.argmin(np.abs(ct - t)))
            return i if abs(ct[i] - t) <= CANDIDATE_REACH else None

        loud = sorted(((cands[cand(t)][2], float(ct[cand(t)])) for t in bleed if cand(t) is not None), reverse=True)
        good = [float(ct[cand(t)]) for t in real if t > THIS_AFTER and cand(t) is not None]
        wav = str(audio(ctx, s, SESSIONS[s]["mics"][m][0]))
        for k, (_, nt) in enumerate(loud[:N_TRIALS]):
            for kind, this in (("not", []), ("both", [good[min(k, len(good) - 1)]] if good else None)):
                if this is None:
                    continue
                trials.append({"jid": jid, "kind": kind, "this": this, "not": [nt], "real": real, "bleed": bleed,
                               "hits": hits[jid].tolist(),
                               "job": {"id": jid, "wav": wav, "role": "tom", "mode": "learn", "sensitivity": NOT_SENS,
                                       "this": this, "not": [nt], "variants": [VARIANTS[0]], "grid": []}})
    out = rust(ctx, [t["job"] for t in trials])
    res: dict = {}
    for t, r in zip(trials, out):
        v = r["variants"][0]
        hs = [sc for h in t["hits"] if (sc := nearest_score(v["scores"], h)) is not None]
        for name, th in rule_thresholds(hs, v["auto"], v["suggest"], list(v["not_scores"])).items():
            rg, bg = grey_counts(t["real"], t["bleed"], t["hits"], v["scores"], th, t["not"], t["this"])
            for scope in ("all", t["jid"].split("/")[0], t["jid"]):   # P4-37: per track too
                a = res.setdefault(t["kind"], {}).setdefault(scope, {}).setdefault(name, [0, 0, 0, 0])
                a[0] += rg
                a[1] += len(t["real"])
                a[2] += bg
                a[3] += len(t["bleed"])
    (ctx.cache / "notthis.json").write_text(json.dumps(res))
    for kind, title in (("not", "One Shift+K on a bleed hit"), ("both", "One K on a real hit and one Shift+K on a bleed hit")):
        print(f"{title} (tom tracks, Sensitivity {NOT_SENS}, {N_TRIALS} trials per track)\n")
        print("| Rule | Real hits grey | Bleed hits grey | Session A real | Session A bleed |")
        print("|---|---|---|---|---|")
        for name, a in res[kind]["all"].items():
            sl = res[kind]["Session A"][name]
            print(f"| {name} | {a[0]} of {a[1]} ({100 * a[0] / a[1]:.1f} %) | {a[2]} of {a[3]} ({100 * a[2] / a[3]:.1f} %) | "
                  f"{100 * sl[0] / sl[1]:.1f} % | {100 * sl[2] / sl[3]:.1f} % |")
        print()
    return res


def cmd_filter(ctx: Ctx) -> dict:
    base = load_json(ctx, "baseline.json")
    jobs, keys = [], []
    for sens in (50, LEARN_SENSITIVITY):
        for s, m, stem, role in mic_jobs(list(SESSIONS)):
            b = base[job_id(s, m)]
            if m not in TOMS or not b["ref_used"] or m in SESSIONS[s]["excluded"]:
                continue
            this, not_ = pick_examples(np.asarray(b["ref_used"]), b["cands"])
            jobs.append({"id": job_id(s, m), "wav": str(audio(ctx, s, stem)), "role": role, "mode": "filter",
                         "sensitivity": sens, "this": this, "not": not_})
            keys.append((sens, s, m))
    rows = rust(ctx, jobs)
    res: dict = {}
    print("| Sensitivity | Session | Mic | Off P | Off R | Off F | On P | On R | On F | Grey hits |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for (sens, s, m), r in zip(keys, rows):
        use = np.asarray(base[job_id(s, m)]["ref_used"])
        off, on = prf(use, np.asarray(r["off"])), prf(use, np.asarray(r["on"]))
        res[f"{sens}/{s}/{m}"] = {"off": off, "on": on, "out": len(r["out"])}
        print(f"| {sens} | {s} | {m} | {off['p']:.3f} | {off['r']:.3f} | {off['f']:.3f} | {on['p']:.3f} | {on['r']:.3f} | "
              f"{on['f']:.3f} | {len(r['out'])} |")
    for sens in (50, LEARN_SENSITIVITY):
        v = [x for k, x in res.items() if k.startswith(f"{sens}/")]
        print(f"- Sensitivity {sens}: mean F off {np.mean([x['off']['f'] for x in v]):.3f}, on {np.mean([x['on']['f'] for x in v]):.3f}; "
              f"FP+FN off {sum(x['off']['work'] for x in v)}, on {sum(x['on']['work'] for x in v)}")
    (ctx.cache / "filter.json").write_text(json.dumps(res))
    return res


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("step", choices=["refs", "baseline", "learn", "tune", "timing", "notthis", "filter", "unmatched", "all"])
    ap.add_argument("--sessions", default=",".join(SESSIONS), help="learn: the sessions, comma-separated")
    ap.add_argument("--samples", default=str(Path.home() / ".cache" / "drumspike" / "samples"))
    ap.add_argument("--cache", default=str(Path.home() / ".cache" / "drumspike" / "learn-eval"))
    ap.add_argument("--tool", default=os.environ.get("DRUMSPIKE_SERVICE_TOOL",
                                                     str(REPO / "target" / "release" / "service_tool")))
    a = ap.parse_args(argv)
    ctx = Ctx(Path(a.samples), Path(a.cache), Path(a.tool))
    ctx.cache.mkdir(parents=True, exist_ok=True)
    if a.step in ("refs", "all"):
        cmd_refs(ctx)
    if a.step in ("baseline", "all"):
        cmd_baseline(ctx)
    if a.step == "tune":
        cmd_tune(ctx)
    if a.step == "timing":
        cmd_timing(ctx)
    if a.step == "notthis":
        cmd_notthis(ctx)
    if a.step == "filter":
        cmd_filter(ctx)
    if a.step == "unmatched":
        cmd_unmatched(ctx)
    if a.step in ("learn", "all"):
        cmd_learn(ctx, sessions=a.sessions.split(","))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
