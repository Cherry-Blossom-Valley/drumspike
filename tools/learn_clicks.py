# /// script
# requires-python = ">=3.12"
# dependencies = ["numpy", "soundfile", "mido"]
# ///
"""P4-34: how many hand fixes does one Learn click save? (measurement only, no product change)

    uv run --with numpy --with soundfile --with mido python tools/learn_clicks.py run|score|all \
        [--out ~/.cache/drumspike/p434] [--tool <service_tool>] [--clicks 10] [--songs "Session A,..."] [--not-only] \
        [--explain <out.jsonl>]

Third-party imports: numpy, soundfile and mido (through `fix_metric.py`, which this tool reuses: the references
`refs.json` with the fair kick and snare reference (`apply_ref`, `pick_fair`), `match`, `track_offset`, `score_track`;
no scoring rule is copied). The Rust side is the release `service_tool cli run` (the product commands `project_new`,
`tracks_add`, `analyze_start`, `track_learn_mark`, `events_get`, `project_save`). The samples are read only; every
result stays in `--out`, never in git.

The simulated engineer, per track (kick `k in`, snare `s top`, the tom close mics of each song), at most `--clicks` clicks:

  * Shift+K (`kind: not`) on the strongest output hit (velocity, then rise) with no reference hit within 50 ms
    (the offset of the track applied to the reference, as `fix_metric`);
  * if no such hit is left, K (`kind: this`) on the weakest output hit that matches a reference hit within 20 ms;
  * a hit the Learn filter greyed (`filtered`) is not in the output (counted as `cut` for the metric).

After each click: output hits, false, missed, moved, fixes (as `fix_metric.score_track` counts them). The offset of a
track is fixed from the 0-click analysis, so a click cannot change the yardstick. Hand cost model: one click per
false hit deleted, per missed hit added, per moved hit; so `saved after k clicks = fixes_0 - fixes_k` and a click
pays while it saves more than one fix. Real hits lost by Learn: `missed_k - missed_0` (the maximum over k).

`run` is resumable: one result file `<out>/tracks/<song>__<mic>.json` per track, a finished song is skipped; one project
per song in `<out>/work/` (removed when the song is done). `score` writes `<out>/report.md` and `<out>/summary.json`.

`--explain <out.jsonl>` (#293 diagnosis, no rule change): for every Forget (Shift+K) click of the run, one line with
every candidate of that track as the filter saw it before the click (`explain_click`: the score, the shape cos
1 - 2 * score, the attack against the gate window, greyed, ref-matched) and the per-click summary. The table of the
summary goes to the same name with `.md`. Appends per track, so a resumed run adds only the tracks it runs.

`run` takes `heavy.lock.d` itself (it waits for it and writes the info file), so it must never run inside
`crewboard lock run`: that waits for its own lock and self-deadlocks (26 min observed, #69).
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fix_metric as fm  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
OUT = Path.home() / ".cache" / "drumspike" / "p434"
HEAVY = Path.home() / ".cache" / "drumspike" / "heavy.lock.d"
FP_WINDOW = 0.050      # a hit with no reference hit this near is false (the offset window of fix_metric)
TOL_MISSED = 0.020     # P4-38c: a reference hit with no output hit this near is missed (the metric's TOL)
CLICKS = 10
SHOW = (0, 1, 2, 3, 5, 10)
REAL_SESSIONS = ("Session A", "Session B", "Session C")   # the three sessions of refs.json (learn-eval)
FILTERED = "filtered"


# ---------------------------------------------------------------- the engineer (pure: tested without the app)

def as_output(events):
    """The events as the metric reads them: a hit the Learn filter greyed counts as cut (not in the output)."""
    return [dict(e, cut=True) if e.get(FILTERED) else e for e in events]


def choose_click(events, ref_shifted, marked=(), not_only=False, missed=False):
    """The next click on `events` (dicts with `t`, `v`, optional `rise`, `bleed`, `cut`, `filtered`): ("not", t) on the
    strongest output hit with no reference hit within FP_WINDOW, else ("this", t) on the weakest output hit that matches
    a reference hit (fix_metric.match, one to one), else None. A hit that carries a mark already (`marked`, times)
    is not clicked again (the app answers "already marked"). `not_only` (P4-37, `--not-only`): Shift+K clicks only, so
    None when no false hit is left. `missed` (P4-38c, `--missed`): K on the strongest MISSED reference hit (no output
    hit within TOL): the click is a RANGE around the reference time (the service resolves the strongest candidate in
    it and adds the hit, P4-38b); None when every reference hit has an output hit."""
    ev = as_output(events)
    out = [e for e in ev if not e.get("bleed") and not e.get("cut")]
    if not out:
        return None
    ref = np.sort(np.asarray(ref_shifted, float))

    def near(t):
        if not len(ref):
            return float("inf")
        i = int(np.searchsorted(ref, t))
        return min(abs(ref[j] - t) for j in (i - 1, i) if 0 <= j < len(ref))

    free = lambda e: all(abs(float(e["t"]) - m) > 0.002 for m in marked)  # noqa: E731
    if missed:
        # P4-38c: a reference hit with no OUTPUT hit within TOL_MISSED (the metric's missed set);
        # the distance is to the output hits, not to the reference itself
        ot = np.sort(np.asarray([float(e["t"]) for e in out], float))
        got = [float(r) for r in ref if not len(ot) or np.min(np.abs(ot - r)) > TOL_MISSED]
        got = [t for t in got if all(abs(t - m) > 0.002 for m in marked)]
        if not got:
            return None
        t = got[0]
        return ("this", (round(t - 0.02, 6), round(t + 0.02, 6)))
    fp = [e for e in out if near(float(e["t"])) > FP_WINDOW and free(e)]
    if fp:
        e = max(fp, key=lambda e: (float(e.get("v", 0)), float(e.get("rise", 0)), -float(e["t"])))
        return ("not", float(e["t"]))
    if not_only:
        return None
    times = [float(e["t"]) for e in out]
    pairs = fm.match(list(ref), times)
    hit = [out[j] for _, j in pairs if free(out[j])]
    if not hit:
        return None
    e = min(hit, key=lambda e: (float(e.get("v", 0)), float(e.get("rise", 0)), float(e["t"])))
    return ("this", float(e["t"]))


def row_of(ref, events, offset, k, click):
    s = fm.score_track(ref, as_output(events), offset)
    return {"k": k, "click": click, "n_out": s["n_out"], "false": s["false"], "missed": s["missed"],
            "moved": s["moved"], "fixes": s["fixes"]}


def simulate_track(engine, ref, events0, n_clicks=CLICKS, not_only=False, missed=False):
    """The click curve of one track. `engine(kind, t)` makes the click and returns the track's events after it (t is a
    time for a hit click, a (t0, t1) range for P4-38b's missed-click). Returns the rows (k = 0 .. the last click made;
    a track that has no click left stops early)."""
    off = fm.track_offset(ref, events0)
    rows = [row_of(ref, events0, off, 0, None)]
    ev, marked = events0, []
    for k in range(1, n_clicks + 1):
        c = choose_click(ev, [float(r) + off for r in ref], marked, not_only, missed)
        if c is None:
            break
        marked.append(c[1] if not isinstance(c[1], tuple) else sum(c[1]) / 2)
        ev = engine(*c)
        rows.append(row_of(ref, ev, off, k, list(c)))
    return {"offset": off, "rows": rows}


def at(rows, k):
    """The row after k clicks; a track with fewer clicks holds its last row."""
    return rows[min(k, len(rows) - 1)]


EXPLAIN_TOL = 0.020   # --explain: a candidate is greyed when a greyed hit of the track lies this near


def explain_click(scores, events, ref_shifted, k, click_t):
    """--explain (#293 diagnosis): one Forget click's candidates as the Learn filter saw them before the click.

    `scores`: `track_learn_scores` of the track before the click (`candidates` rows [t, class, score, v, gate, attack],
    `shape` [template cos, mark cos] per candidate, `threshold`, `attack` {marks, window}); `events`: the track's saved
    events before the click (`filtered` = greyed); `ref_shifted`: the reference times with the track's offset applied.
    Per candidate: the score, the shape cos to the marks (the mean over the nearest "not" examples, as the score sees it),
    the template cos, `via_template` (the template decided the 0.5 of the score: its cos is at least the mark cos), the
    attack in ms and whether it lies in the gate window, `would_grey` (under the threshold and in the window: the rule
    alone, no mark's own override), `greyed` (the app greyed a hit there) and `ref` (`fix_metric.match`, one to one).
    `in_output` (#405): a detected hit of the track lies within EXPLAIN_TOL of the candidate (greyed or not). A candidate
    under the detection threshold has none, and a Forget cannot remove it. The summary counts the output candidates
    only; `candidates` in the record counts all of them. Pure."""
    thr = float(scores["threshold"]) if scores.get("state") == "ok" else None
    att = scores.get("attack") or {}
    win = att.get("window")
    hits = np.asarray(sorted(float(e["t"]) for e in events), float)
    grey = np.asarray(sorted(float(e["t"]) for e in events if e.get(FILTERED)), float)
    cands = scores.get("candidates", [])
    shape = scores.get("shape") or [[None, None]] * len(cands)
    times = [float(c[0]) for c in cands]
    matched = {j for _, j in fm.match([float(r) for r in ref_shifted], times)} if times and len(ref_shifted) else set()
    rows = []
    for j, (t, cls, sc, v, _gate, a) in enumerate(cands):
        sc = float(sc)
        tc, mc = shape[j]
        in_win = bool(a is not None and win is not None and win[0] <= a <= win[1])
        past = bool(thr is not None and sc >= thr)
        out = bool(len(hits) and float(np.min(np.abs(hits - float(t)))) <= EXPLAIN_TOL)
        rows.append({"t": float(t), "class": cls, "v": v, "score": sc, "cos": mc, "tmpl_cos": tc,
                     "via_template": bool(tc is not None and mc is not None and tc >= mc),
                     "past_shape": past, "attack_ms": a, "in_window": in_win, "would_grey": bool(thr is not None and sc < thr and in_win),
                     "greyed": bool(len(grey) and float(np.min(np.abs(grey - float(t)))) <= EXPLAIN_TOL),
                     "ref": j in matched, "in_output": out})
    rows_out = [r for r in rows if r["in_output"]]

    def count(pred):
        return sum(1 for r in rows_out if pred(r))

    summary = {"n": len(rows_out), "candidates": len(rows), "past_shape": count(lambda r: r["past_shape"]),
               "via_template": count(lambda r: r["via_template"]),
               "in_window": count(lambda r: r["in_window"]), "would_grey": count(lambda r: r["would_grey"]),
               "greyed": count(lambda r: r["greyed"]), "ref": count(lambda r: r["ref"]),
               "would_grey_ref": count(lambda r: r["would_grey"] and r["ref"]),
               "greyed_ref": count(lambda r: r["greyed"] and r["ref"])}
    return {"k": k, "t": float(click_t), "state": scores.get("state"), "threshold": thr,
            "window": list(win) if win else None, "mark_ms": att.get("marks"), "summary": summary, "candidates": rows}


def explain_table(records):
    """--explain: the per-click summary of the records as one markdown table (pure)."""
    head = ["song", "mic", "role", "click", "t (s)", "threshold", "output candidates", "past shape (score >= thr)",
            "via template", "attack in window", "would grey", "greyed", "ref-matched", "ref in would grey", "ref in greyed"]
    body = []
    for r in records:
        s = r["summary"]
        thr = "" if r["threshold"] is None else f"{r['threshold']:.4f}"
        body.append([r["song"], r["mic"], r["role"], r["k"], f"{r['t']:.4f}", thr, s["n"], s["past_shape"],
                     s["via_template"], s["in_window"], s["would_grey"], s["greyed"], s["ref"], s["would_grey_ref"],
                     s["greyed_ref"]])
    return fm.md_table(head, body)


def load_explain(path):
    return [json.loads(x) for x in Path(path).read_text().splitlines() if x.strip()]


# ---------------------------------------------------------------- the app

def tool_path(a_tool=None):
    return Path(a_tool or os.environ.get("DRUMSPIKE_CALIB_TOOL") or ROOT / "target" / "release" / "service_tool")


def add_run_steps(names, mics, add, folder):
    """The steps of the P4-38c Add run: the project of `setup` opened first (#69), then `track_learn_set` per mic.

    Each `Song.run` is a fresh `service_tool cli run` process, so `track_learn_set` without a preceding
    `project_open` has no project and the service answers `404 no such project` (exit 3). The fresh project's
    learn holds the service defaults (learn.rs: on false, threshold 0.5, auto true, no marks)."""
    cur = {"on": False, "threshold": 0.5, "auto": True, "marks": []}
    return [{"command": "project_open", "args": {"path": folder}}] + \
        [{"command": "track_learn_set",
          "args": {"sid": "$sid", "name": names[m], "learn": dict(cur, add=add)}} for m in mics]


class Song:
    """One song in the app: a project with its tracks analysed once; every click round is a `cli run` of its own
    (open, the marks, events_get of every track, save), so the state between rounds is the saved project."""

    def __init__(self, tool, name, cfg, mics, work):
        # every mic of the song is a track (the pair rule and the bleed sources read the other tracks, as in
        # `fix_metric run`); only `mics` (those with a reference) are clicked and read back
        self.tool, self.name, self.cfg, self.work = tool, name, cfg, work
        self.all = [(m, v[0]) for m, v in cfg["mics"].items()]
        self.mics = [(m, cfg["mics"][m][0]) for m in mics]
        self.names = {m: fm.track_name(stem) for m, stem in self.mics}
        self.folder = None

    def env(self):
        env = {k: v for k, v in os.environ.items() if k not in fm.CONFIGS["before"]}
        env.update({"DRUMSPIKE_HOME": str(self.work / "h"), "DRUMSPIKE_PROJECTS": str(self.work / "p")})
        return env

    def run(self, steps, tag):
        (self.work).mkdir(parents=True, exist_ok=True)
        path = self.work / f"{tag}.json"
        path.write_text(json.dumps(steps))
        r = subprocess.run([str(self.tool), "cli", "run", str(path)], capture_output=True, text=True, env=self.env(),
                           stdin=subprocess.DEVNULL)
        rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
        bad = [x for x in rows if not x.get("ok", False)]
        if r.returncode != 0 or bad or len(rows) != len(steps):
            raise RuntimeError(f"{self.name} {tag}: exit {r.returncode} {bad[:1]} {r.stderr[-400:]}")
        return rows

    def setup(self):
        roles = {"kick": "kick", "snare": "snare", "toms": "tom"}
        note = {"kick": 36, "snare": 38, "tom": 47}
        rl = [roles[self.cfg["mics"][m][1]] for m, _ in self.all]
        (self.work / "p").mkdir(parents=True, exist_ok=True)
        steps = [{"command": "project_new", "args": {"name": "clicks", "dir": str(self.work / "p")}},
                 {"command": "tracks_add", "args": {"sid": "$sid", "paths": [str(fm.audio_path(self.cfg, s)) for _, s in self.all],
                                                    "roles": rl, "notes": [note[r] for r in rl]}, "wait": True},
                 {"command": "analyze_start", "args": {"sid": "$sid"}, "wait": True},
                 {"command": "project_save", "args": {"sid": "$sid"}}]
        steps += [{"command": "events_get", "args": {"sid": "$sid", "name": self.names[m]}} for m, _ in self.mics]
        rows = self.run(steps, "setup")
        self.folder = rows[0]["result"].get("folder") or rows[0]["result"].get("path")
        (self.work / "folder.txt").write_text(str(self.folder))
        return {m: rows[4 + i]["result"]["events"] for i, (m, _) in enumerate(self.mics)}

    def round(self, marks, explain=()):
        """`marks` {mic: (kind, t or (t0, t1))}: the click of each track in one round. `explain`: the mics whose
        `track_learn_scores` is read after the open and before the marks (--explain: the state the click saw; left in
        `self.scores`). Returns {mic: events}."""
        if self.folder is None:
            self.folder = (self.work / "folder.txt").read_text()
        steps = [{"command": "project_open", "args": {"path": self.folder}}]
        order = [m for m, _ in self.mics]
        read = [m for m in order if m in explain]
        steps += [{"command": "track_learn_scores", "args": {"sid": "$sid", "name": self.names[m]}} for m in read]
        for m in order:
            if m in marks:
                kind, at = marks[m]
                args = {"sid": "$sid", "name": self.names[m], "kind": kind}
                if isinstance(at, tuple):
                    args["t0"], args["t1"] = at
                else:
                    args["hit"] = at
                steps.append({"command": "track_learn_mark", "args": args})
        steps.append({"command": "project_save", "args": {"sid": "$sid"}})
        steps += [{"command": "events_get", "args": {"sid": "$sid", "name": self.names[m]}} for m in order]
        rows = self.run(steps, "round")
        self.scores = {m: rows[1 + i]["result"] for i, m in enumerate(read)}
        n = len(order)
        # #81: how each K found its time (`from`, `added_hit`, the rise log `k_rise` of a K-add on an empty range)
        self.k_answers = {}
        for r, s in zip(rows, steps):
            if s["command"] == "track_learn_mark" and not r["result"].get("marked", False):
                raise RuntimeError(f"{self.name}: the mark {s['args']} was not stored: {r['result']}")
            if s["command"] == "track_learn_mark" and r["result"].get("filter_error"):
                raise RuntimeError(f"{self.name}: filter error {r['result']['filter_error']}")
            if s["command"] == "track_learn_mark":
                m = next(k for k in order if self.names[k] == s["args"]["name"])
                self.k_answers[m] = {"args": {k: v for k, v in s["args"].items() if k != "sid"},
                                     **{k: r["result"].get(k) for k in ("from", "added_hit", "k_rise")}}
        return {m: rows[len(rows) - n + i]["result"]["events"] for i, m in enumerate(order)}

    def all_events(self):
        """#81: the saved events of every mic of the song (also those without a reference: the bleed sources)."""
        if self.folder is None:
            self.folder = (self.work / "folder.txt").read_text()
        steps = [{"command": "project_open", "args": {"path": self.folder}}]
        steps += [{"command": "events_get", "args": {"sid": "$sid", "name": fm.track_name(s)}} for _, s in self.all]
        rows = self.run(steps, "final")
        return {m: rows[1 + i]["result"]["events"] for i, (m, _) in enumerate(self.all)}


def references(out):
    """refs.json of fix_metric (union) with the fair kick and snare reference applied (P4-16c, P4-23)."""
    src = fm.OUT / "refs.json"
    refs = json.loads(src.read_text())
    hats = {"kick": fm.read_hat(fm.HAT_SRC), "snare": fm.read_hat(fm.HAT_SRC_SNARE)}
    return fm.apply_ref(refs, {"kick": "fair", "snare": "fair"}, hats, log=lambda *_: None)


def take_lock(song):
    """Take heavy.lock.d and write the owner file in the the crewboard host lock format (issue 69).

    the crewboard host lock reads <dir>/owner (key=value lines). Without it, the lock counts as
    stale after --empty-grace (empty-after-grace) and a wrapped gate run takes it over
    while learn_clicks still runs. The host name matches the crewboard host lock:
    ${HOST_LOCK_HOST:-$(hostname -s)}. The owner file is written to a temp name and
    renamed; shutil.rmtree(HEAVY) in the caller removes it with the directory.
    """
    HEAVY.mkdir()
    (HEAVY / "info").write_text(f"p434 learn_clicks {song} pid {os.getpid()}\n")
    host = os.environ.get("HOST_LOCK_HOST") or socket.gethostname().split(".")[0]
    agent = os.environ.get("CREWBOARD_AGENT") or "agent@host"
    tmp = HEAVY / "owner.tmp"
    tmp.write_text(
        f"agent={agent}\n"
        f"host={host}\n"
        f"pid={os.getpid()}\n"
        f"started={time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}\n"
        f"started_epoch={int(time.time())}\n"
        f"reason=p434 learn_clicks {song}\n"
    )
    tmp.rename(HEAVY / "owner")


def do_run(out, tool, clicks=CLICKS, only=None, log=print, not_only=False, missed=False, add=None, explain=None):
    out = Path(out)
    (out / "tracks").mkdir(parents=True, exist_ok=True)
    refs = references(out)
    table = fm.songs()
    t_start = time.time()
    HEAVY.parent.mkdir(parents=True, exist_ok=True)
    for song, cfg in table.items():
        if only and song not in only:
            continue
        mics = [m for m in refs[song]["mics"] if m in cfg["mics"]]
        files = {m: out / "tracks" / f"{fm.track_name(song)}__{fm.track_name(m)}.json" for m in mics}
        if all(f.is_file() for f in files.values()):
            continue
        held_by_caller = os.environ.get("CREWBOARD_HOST_LOCK") == str(HEAVY)
        if not held_by_caller:
            # CREWBOARD_HOST_LOCK names a lock the caller already holds (the crewboard host lock run);
            # see docs/guides/ci-and-host-lock.md, "A tool that takes the lock itself".
            while HEAVY.exists():
                log("waiting for heavy.lock.d")
                time.sleep(30)
            take_lock(song)
        try:
            t0 = time.time()
            work = out / "work" / fm.track_name(song)
            if work.exists():
                shutil.rmtree(work)
            sg = Song(tool, song, cfg, mics, work)
            ev0 = sg.setup()
            off = {m: fm.track_offset(refs[song]["mics"][m]["onsets"], ev0[m]) for m in mics}
            if add is not None:
                # P4-38c: the Add threshold through track_learn_set; the fresh project's learn has the
                # service defaults (learn.rs: on false, threshold 0.5, auto true, no marks)
                sg.run(add_run_steps(sg.names, mics, add, sg.folder), f"add{add}")
            state = {m: {"events": ev0[m], "rows": [row_of(refs[song]["mics"][m]["onsets"], ev0[m], off[m], 0, None)],
                         "done": False, "marked": [], "k": []} for m in mics}
            explained = {}   # --explain: {mic: [click records]}, written with the track's file
            for k in range(1, clicks + 1):
                marks = {}
                for m in mics:
                    s = state[m]
                    if s["done"]:
                        continue
                    c = choose_click(s["events"], [float(r) + off[m] for r in refs[song]["mics"][m]["onsets"]],
                                     s["marked"], not_only, missed)
                    if c is None:
                        s["done"] = True
                    else:
                        marks[m] = c
                if not marks:
                    break
                forget = {m for m, c in marks.items() if c[0] == "not"} if explain else ()
                got = sg.round(marks, forget)
                for m in forget:
                    refm = [float(r) + off[m] for r in refs[song]["mics"][m]["onsets"]]
                    rec = explain_click(sg.scores[m], state[m]["events"], refm, k, marks[m][1])
                    explained.setdefault(m, []).append(dict(rec, song=song, mic=m, role=refs[song]["mics"][m]["role"]))
                for m, c in marks.items():
                    state[m]["events"] = got[m]
                    state[m]["marked"].append(c[1] if not isinstance(c[1], tuple) else sum(c[1]) / 2)
                    state[m]["rows"].append(row_of(refs[song]["mics"][m]["onsets"], got[m], off[m], k, list(c)))
                    state[m]["k"].append({"round": k, **getattr(sg, "k_answers", {}).get(m, {})})
            sec = time.time() - t0
            # #81: the last events of every mic, for `diagnose` (which mic an added false hit bleeds from)
            final = sg.all_events()
            others = {o: [float(e["t"]) for e in ev if not e.get("bleed") and not e.get("cut") and not e.get(FILTERED)]
                      for o, ev in final.items()}
            for m in mics:
                files[m].write_text(json.dumps({"song": song, "mic": m, "role": refs[song]["mics"][m]["role"],
                                                "n_ref": len(refs[song]["mics"][m]["onsets"]), "offset": off[m],
                                                "minutes": refs[song]["duration_s"] / 60.0,
                                                "rows": state[m]["rows"], "k_log": state[m]["k"], "seconds_song": sec,
                                                "events": final[m], "ref": refs[song]["mics"][m]["onsets"],
                                                "others": {o: t for o, t in others.items() if o != m}}))
            if explain:
                with open(explain, "a") as f:
                    for rec in (r for m in mics for r in explained.get(m, [])):
                        f.write(json.dumps(rec) + "\n")
            shutil.rmtree(work, ignore_errors=True)
            log(f"{song}: {len(mics)} tracks, {sec:.0f} s")
        finally:
            if not held_by_caller:  # the caller's lock is not ours to remove
                shutil.rmtree(HEAVY, ignore_errors=True)
    log(f"run time {time.time() - t_start:.0f} s")


# ---------------------------------------------------------------- the tables

def load_tracks(out):
    return [json.loads(p.read_text()) for p in sorted((Path(out) / "tracks").glob("*.json"))]


def fp_only(t):
    """The track as an engineer who stops at the first K: the rows before the first "this" click (no false hit left)."""
    rows = []
    for r in t["rows"]:
        if r["click"] and r["click"][0] == "this":
            break
        rows.append(r)
    return dict(t, rows=rows)


def click_kinds(tracks):
    """How the clicks changed the fixes: {(role, kind): {saved more than 1, exactly 1, none, worse}} (pure)."""
    out = {}
    for t in tracks:
        for a, b in zip(t["rows"], t["rows"][1:]):
            d = a["fixes"] - b["fixes"]
            c = out.setdefault((t["role"], b["click"][0]), {"gt1": 0, "eq1": 0, "zero": 0, "worse": 0})
            c["gt1" if d > 1 else "eq1" if d == 1 else "zero" if d == 0 else "worse"] += 1
    return out


def summarize(tracks):
    """Per role and per track the numbers of the report (pure)."""
    res = {"roles": {}, "tracks": []}
    for t in tracks:
        r = t["rows"]
        f0, m0 = r[0]["fixes"], r[0]["missed"]
        t["lost"] = max(max(x["missed"] - m0 for x in r), 0)
        res["tracks"].append({"song": t["song"], "mic": t["mic"], "role": t["role"], "n_ref": t["n_ref"],
                              "clicks": len(r) - 1, "fixes": {k: at(r, k)["fixes"] for k in SHOW},
                              "saved3": f0 - at(r, 3)["fixes"], "saved10": f0 - at(r, 10)["fixes"], "lost": t["lost"],
                              "real": t["song"] in REAL_SESSIONS})
    for role in ("kick", "snare", "toms"):
        ts = [t for t in tracks if t["role"] == role]
        if not ts:
            continue
        fx = {k: sum(at(t["rows"], k)["fixes"] for t in ts) for k in SHOW}
        # the click count where a click saves less than one fix (the role's mean curve, then the first k)
        stop = next((k for k in range(1, CLICKS + 1)
                     if sum(at(t["rows"], k - 1)["fixes"] - at(t["rows"], k)["fixes"] for t in ts) < len(ts)), None)
        res["roles"][role] = {"tracks": len(ts), "fixes": fx, "lost_total": sum(t["lost"] for t in ts),
                              "lost_max": max(t["lost"] for t in ts),
                              "saved_per_click": {k: (fx[0] - fx[k]) / k for k in SHOW if k},
                              "avg_saved3": (fx[0] - fx[3]) / len(ts), "stop": stop}
    return res


def do_score(out, log=print):
    tracks = load_tracks(out)
    res = summarize(tracks)
    L = ["## P4-34 tables (generated by tools/learn_clicks.py)", ""]
    L += ["### Per role: fixes after k clicks (sum over the tracks)", ""]
    L += [fm.md_table(["role", "tracks", *[f"k={k}" for k in SHOW], "saved/click, k=3 (role sum)",
                       "saved/click, k=10 (role sum)", "real hits lost (sum / max)"],
                      [[r, v["tracks"], *[v["fixes"][k] for k in SHOW], f"{v['saved_per_click'][3]:.2f}",
                        f"{v['saved_per_click'][10]:.2f}",
                        f"{v['lost_total']} / {v['lost_max']}"] for r, v in res["roles"].items()]), ""]
    L += ["### Per track", ""]
    L += [fm.md_table(["song", "mic", "role", "ref", "clicks", *[f"k={k}" for k in SHOW], "saved at 3", "lost", "real session"],
                      [[t["song"], t["mic"], t["role"], t["n_ref"], t["clicks"], *[t["fixes"][k] for k in SHOW],
                        t["saved3"], t["lost"], "yes" if t["real"] else ""] for t in res["tracks"]]), ""]
    ck = click_kinds(tracks)
    L += ["### What a click did (number of clicks by the fixes it saved)", ""]
    L += [fm.md_table(["role", "click", "saved more than 1", "saved 1", "saved 0", "made worse"],
                      [[r, k, c["gt1"], c["eq1"], c["zero"], c["worse"]] for (r, k), c in sorted(ck.items())]), ""]
    fo = summarize([fp_only(t) for t in tracks])
    L += ["### Same, stopping at the first K (only Shift+K clicks on false hits)", ""]
    L += [fm.md_table(["role", "tracks", *[f"k={k}" for k in SHOW], "real hits lost (sum / max)"],
                      [[r, v["tracks"], *[v["fixes"][k] for k in SHOW], f"{v['lost_total']} / {v['lost_max']}"]
                       for r, v in fo["roles"].items()]), ""]
    toms = [t for t in res["tracks"] if t["role"] == "toms"]
    avg3 = sum(t["saved3"] for t in toms) / len(toms) if toms else float("nan")
    real_lost = max([t["lost"] for t in res["tracks"] if t["real"]] or [0])
    bar = avg3 > 3 and real_lost <= 2
    ft = [t for t in fo["tracks"] if t["role"] == "toms"]
    L += [f"Stopping at the first K: toms mean saved after 3 clicks {sum(t['saved3'] for t in ft) / len(ft):.2f} per track, "
          f"worst real-session loss {max([t['lost'] for t in fo['tracks'] if t['real']] or [0])}.", ""]
    L += [f"Bar: toms, mean saved after 3 clicks {avg3:.2f} per track (needs more than 3); worst real-session loss "
          f"{real_lost} (needs 2 or fewer): **{'met' if bar else 'not met'}**", ""]
    out = Path(out)
    (out / "report.md").write_text("\n".join(L))
    (out / "summary.json").write_text(json.dumps({"all": res, "fp_only": fo["roles"],
                                                  "kinds": {f"{r}/{k}": c for (r, k), c in ck.items()}}))
    log("\n".join(L))
    return res


SOURCE_WINDOW = 0.012  # #81: the bleed source of a hit is a hit of another mic this near (the pair rule's 12 ms)


def added_false(t):
    """#81: the hits Learn added (`learned`: the K-added hit, `added`, and the Add-rule hits) of a track result that
    are false: no reference hit within TOL_MISSED (the offset applied; the metric's window, so a K-add 20 ms early
    counts). Each with its `kind` (`K` or `Add`), the nearest hit of another mic within SOURCE_WINDOW (`source`: the
    mic, `dt`: ms, None when no mic has a hit that near) and whether the hit is in the output (pure)."""
    ref = np.sort(np.asarray([float(r) + t["offset"] for r in t.get("ref", [])], float))
    rows = []
    for e in t.get("events", []):
        if not e.get("learned"):
            continue
        x = float(e["t"])
        if len(ref) and np.min(np.abs(ref - x)) <= TOL_MISSED:
            continue
        best = None
        for o, ts in t.get("others", {}).items():
            if not ts:
                continue
            a = np.asarray(ts, float)
            d = float(a[np.argmin(np.abs(a - x))] - x)
            if abs(d) <= SOURCE_WINDOW and (best is None or abs(d) < abs(best[1])):
                best = (o, d)
        out = not e.get("bleed") and not e.get("cut") and not e.get(FILTERED)
        rows.append({"t": x, "kind": "K" if e.get("added") else "Add", "out": out, "source": best[0] if best else None,
                     "dt": round(best[1] * 1000, 1) if best else None})
    return rows


def do_diagnose(out, log=print):
    """#81: per track, the false hits Learn added (K-add or Add rule; in the output, or taken out by a rule) by source
    mic: `diagnose.md`."""
    L = ["## #81 False hits Learn added, by source mic (generated by tools/learn_clicks.py diagnose)", ""]
    body = []
    for t in load_tracks(out):
        if "events" not in t:
            continue
        rows = added_false(t)
        for kind in ("K", "Add"):
            for kept in (True, False):
                src = {}
                for r in rows:
                    if r["kind"] == kind and r["out"] == kept:
                        src[r["source"] or "none"] = src.get(r["source"] or "none", 0) + 1
                if src:
                    body.append([t["song"], t["mic"], t["role"], kind, "in the output" if kept else "taken out",
                                 sum(src.values()), ", ".join(f"{k} {v}" for k, v in sorted(src.items()))])
    L += [fm.md_table(["song", "mic", "role", "added by", "state", "false hits", "source mic (count)"], body), ""]
    (Path(out) / "diagnose.md").write_text("\n".join(L))
    log("\n".join(L))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("step", choices=["run", "score", "all", "diagnose"])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--tool", default=None)
    ap.add_argument("--clicks", type=int, default=CLICKS)
    ap.add_argument("--songs", default="", help="only these songs, comma-separated")
    ap.add_argument("--not-only", action="store_true", help="P4-37: Shift+K (Forget This) clicks only, no K")
    ap.add_argument("--missed", action="store_true", help="P4-38c: K on the strongest missed reference hit (the Add rule)")
    ap.add_argument("--add", type=float, default=None, help="P4-38c: the Add threshold set through track_learn_set (default: the service's 0.85)")
    ap.add_argument("--explain", default=None, metavar="OUT.JSONL", help="#293 diagnosis: every Forget click's candidates as the filter saw them")
    a = ap.parse_args(argv)
    if a.step in ("run", "all"):
        tool = tool_path(a.tool)
        if not tool.is_file():
            raise SystemExit(f"need the release service_tool ({tool})")
        do_run(a.out, tool, a.clicks, set(filter(None, a.songs.split(","))) or None, not_only=a.not_only,
               missed=a.missed, add=a.add, explain=a.explain)
        if a.explain and Path(a.explain).is_file():
            recs = load_explain(a.explain)
            table = explain_table(recs)
            Path(a.explain).with_suffix(".md").write_text(table + "\n")
            print(table)
    if a.step in ("score", "all"):
        do_score(a.out)
    if a.step in ("diagnose", "all"):
        do_diagnose(a.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
