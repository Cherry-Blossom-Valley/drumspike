"""#485 explainer 9, "Doubtful hits first" (chapter 9). Script: docs/product/scripts/explainer-09-doubtful-hits.md.

The picture is the real answer of the app on the generated session (tools/media/demo_session.py, WAV
files in ~/.cache/drumspike/media/demo/), for its two snare tracks: the service tool of the repo's
devtools opens a project of all the tracks, analyses it and asks the service for the hits of the
two tracks (`events_get`) and their ratings (`track_confidence`), as the page does. The
queue order is the page's (`rqQueue` of static/review.js: lowest confidence first, then time, then
track order) and the info line is the page's `rqWhy`. Build the tool first (`cargo build -p
drumspike-devtools --bin service_tool`) or point DRUMSPIKE_SERVICE_TOOL at a built one. The numbers
of the rule are the script's "Facts used". Render through build.py.
"""
import importlib.util
import json
import os
import subprocess
import tempfile
from pathlib import Path

from explainer_common import (
    COBALT, DIM, DOWN, ORANGE, TEXT, UP, Create, DashedLine, Explainer, FadeIn, FadeOut, FONT, Line,
    Rectangle, Text, VGroup,
)
from explainer_10 import SR, demo_dir, lane_wave, truth

ROOT = Path(__file__).resolve().parent.parent.parent.parent
PLOT_W = 11.0
TRACKS = ("s top", "s bot")   # the two snare tracks; both are rated by the level of the hit
BASE = {"s top": 0.3, "s bot": -1.9}   # the baseline of each track's lane
HEIGHT = 1.2   # a stalk of confidence 1
DECIMALS = 2   # the page writes the confidence with two decimals (`toFixed(2)`)
ZOOM_MS = 80.0
LEAD_MS = 20.0


def service_tool():
    """The built service tool: DRUMSPIKE_SERVICE_TOOL, else the checkout's debug or release build."""
    env = os.environ.get("DRUMSPIKE_SERVICE_TOOL")
    paths = [Path(env)] if env else [ROOT / "target" / "debug" / "service_tool", ROOT / "target" / "release" / "service_tool"]
    for path in paths:
        if path.exists():
            return path
    raise SystemExit(f"FAIL: no service_tool at {paths[0]}: cargo build -p drumspike-devtools --bin service_tool, "
                     "or set DRUMSPIKE_SERVICE_TOOL")


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def ratings_of(events, confidence):
    """One track's hits with their ratings: [{t, c, doubt, why}] in the order of the hits."""
    assert len(events) == len(confidence), (len(events), len(confidence))
    return [{"t": float(e["t"]), "c": float(r["c"]), "doubt": bool(r["doubt"]), "why": list(r["why"])}
            for e, r in zip(events, confidence)]


_SESSION = None


def session():
    """{track: [{t, c, doubt, why}]} of the two snare tracks, from the real service (computed once). The whole
    session is analysed: the rules between tracks (bleed) decide which hits of a snare track are in its output."""
    global _SESSION
    if _SESSION is not None:
        return _SESSION
    demo = demo_dir()
    shots = load("ds_shots", ROOT / "tools" / "media" / "shots.py")   # the roles and notes of the session's tracks
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        tracks = [(n, str(demo / f"{n}.wav"), role, shots.NOTE[role]) for n, role in shots.ROLE.items()]
        project = load("ds_make_project", ROOT / "apps" / "desktop" / "selftest" / "make_project.py").build_project(
            tmp / "projects", "Session", tracks, "referenced")
        steps = [{"cmd": "project_open", "args": {"path": str(project)}},
                 {"cmd": "analyze_start", "args": {"sid": "$sid"}},
                 {"cmd": "wait_analysis", "args": {"sid": "$sid"}}]
        for n in TRACKS:
            steps.append({"cmd": "events_get", "args": {"sid": "$sid", "name": n}})
            steps.append({"cmd": "track_confidence", "args": {"sid": "$sid", "name": n}})
        (tmp / "steps.json").write_text(json.dumps(steps))
        done = subprocess.run([str(service_tool()), "run", str(tmp / "steps.json")], capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit(f"FAIL: service tool exited {done.returncode}: {done.stderr[-400:]}")
    rows = [json.loads(line) for line in done.stdout.splitlines()]
    answers = {}
    for row in rows:
        if "error" in row:
            raise SystemExit(f"FAIL: service tool step {row['cmd']}: {row['error']}")
    # events_get and track_confidence answer per track, in the order of the steps
    gets = [r["answer"] for r in rows if r["cmd"] == "events_get"]
    rates = [r["answer"] for r in rows if r["cmd"] == "track_confidence"]
    for n, g, r in zip(TRACKS, gets, rates):
        answers[n] = ratings_of(g["events"], r["confidence"])
    _SESSION = answers
    return answers


def queue_of(rated, order=TRACKS):
    """The review queue, as `rqQueue` builds it: the doubtful hits of the tracks, lowest confidence first, ties
    by time, then by track order. [{track, i, t, c, why}]."""
    q = [{"track": n, "i": i, "t": h["t"], "c": h["c"], "why": h["why"]}
         for n in order for i, h in enumerate(rated[n]) if h["doubt"]]
    return sorted(q, key=lambda p: (p["c"], p["t"], order.index(p["track"])))


def info_line(item):
    """The text of the Doubt field, as `rqWhy` writes it: the confidence with two decimals, a colon, the reasons."""
    return f"{item['c']:.{DECIMALS}f}: " + "; ".join(item["why"])


def x_of(t, dur):
    return -PLOT_W / 2 + PLOT_W * t / dur


def label_rows(xs, gap=0.4):
    """The row (0, 1, 2, ...) of each label so that two labels of one row are at least `gap` apart in x."""
    last, rows = {}, []
    for x in xs:
        row = 0
        while row in last and x - last[row] < gap:
            row += 1
        last[row] = x
        rows.append(row)
    return rows


class Explainer09(Explainer):
    script_ids = ("s01-problem", "s02-confidence", "s03-queue", "s04-why", "s05-decide", "s06-result")

    def setup(self):
        super().setup()
        self.rated = session()
        self.dur = truth()["duration_samples"] / SR
        self.queue = queue_of(self.rated)
        assert self.queue, "the session has no doubtful hit: the picture has nothing to show"

    # ---- the lanes ----

    def lane_frame(self):
        """The baselines and the names of the two tracks."""
        out = VGroup()
        for n in TRACKS:
            y = BASE[n]
            out.add(Line([-PLOT_W / 2, y, 0], [PLOT_W / 2, y, 0], color=DIM))
            out.add(Text(n, font=FONT, font_size=22, color=TEXT).move_to([-6.3, y + HEIGHT / 2, 0]))
        return out

    def stalks(self, by_confidence, mark_doubtful=True, track=None):
        """The hits of the tracks as stalks from the baseline: all of one height, or the height of the
        confidence; the doubtful ones orange."""
        out = VGroup()
        for n in TRACKS if track is None else (track,):
            for h in self.rated[n]:
                height = HEIGHT * h["c"] if by_confidence else HEIGHT * 0.8
                color = ORANGE if mark_doubtful and h["doubt"] else COBALT
                x = x_of(h["t"], self.dur)
                out.add(Line([x, BASE[n], 0], [x, BASE[n] + max(height, 0.04), 0], color=color, stroke_width=2))
        return out

    def threshold_lines(self):
        out = VGroup()
        for n in TRACKS:
            y = BASE[n] + HEIGHT * 0.5
            out.add(DashedLine([-PLOT_W / 2, y, 0], [PLOT_W / 2, y, 0], color=ORANGE, stroke_width=2))
        return out

    def scale_ticks(self):
        out = VGroup()
        for n in TRACKS:
            for value, text in ((0.0, "0"), (0.5, "0.5"), (1.0, "1")):
                out.add(Text(text, font=FONT, font_size=18, color=DIM).move_to([6.2, BASE[n] + HEIGHT * value, 0]))
        return out

    def rank_numbers(self):
        """The place of each doubtful hit in the queue, under its lane's baseline (rows when two are close)."""
        out = VGroup()
        for n in TRACKS:
            items = [(rank, p) for rank, p in enumerate(self.queue, start=1) if p["track"] == n]
            items.sort(key=lambda rp: rp[1]["t"])
            xs = [x_of(p["t"], self.dur) for _, p in items]
            for (rank, p), x, row in zip(items, xs, label_rows(xs)):
                out.add(Text(str(rank), font=FONT, font_size=20, color=ORANGE).move_to([x, BASE[n] - 0.3 - 0.35 * row, 0]))
        return out

    def playhead(self, item):
        x = x_of(item["t"], self.dur)
        y = BASE[item["track"]]
        return Line([x, y - 0.05, 0], [x, y + HEIGHT + 0.1, 0], color=TEXT, stroke_width=4)

    def hit_count(self):
        return sum(len(v) for v in self.rated.values())

    # ---- the scenes ----

    def s01_problem(self):
        """Two snare tracks of one song, every hit a stalk."""
        title = Text(f"two tracks of one song: {self.hit_count()} hits", font=FONT, font_size=26, color=TEXT)
        self.t_play(Create(self.lane_frame()), t=2.0)
        self.t_play(FadeIn(self.stalks(False, mark_doubtful=False)), FadeIn(title.to_edge(UP, buff=0.9)), t=3.5)
        self.say("Most hits are clear. A few are less sure.", t=2.0)

    def s02_confidence(self):
        """The height of a stalk is its confidence; the dashed line at 0.5; the doubtful ones orange."""
        top = Text("height of a stalk = its confidence", font=FONT, font_size=26, color=TEXT).to_edge(UP, buff=0.8)
        scale = Text("1: as loud as the typical hit of its track.  0: 24 dB under it.", font=FONT, font_size=22,
                     color=DIM).next_to(top, DOWN, buff=0.25)
        self.t_play(Create(self.lane_frame()), FadeIn(self.stalks(True, mark_doubtful=False)), FadeIn(self.scale_ticks()),
                    t=3.0)
        self.t_play(FadeIn(top), FadeIn(scale), t=2.5)
        self.t_play(Create(self.threshold_lines()), t=2.0)
        doubtful = Text("under 0.5: doubtful", font=FONT, font_size=24, color=ORANGE).next_to(scale, DOWN, buff=0.25)
        self.t_play(FadeIn(self.stalks(True, mark_doubtful=True).set_z_index(2)), FadeIn(doubtful), t=2.5)
        self.t_wait(1.0)

    def s03_queue(self):
        """The queue numbers the doubtful hits across both tracks; a playhead jumps from one to the next."""
        key = Text("D: Hits > Next Doubtful Hit", font=FONT, font_size=24, color=TEXT).to_edge(UP, buff=0.8)
        self.t_play(Create(self.lane_frame()), FadeIn(self.stalks(True)), t=2.5)
        self.t_play(FadeIn(self.rank_numbers()), FadeIn(key), t=2.0)
        n = len(self.queue)
        head = self.playhead(self.queue[0])
        status = Text(f"doubtful hit 1 of {n}", font=FONT, font_size=26, color=ORANGE).next_to(key, DOWN, buff=0.3)
        self.t_play(FadeIn(head), FadeIn(status), t=1.5)
        nxt = self.queue[1] if n > 1 else self.queue[0]
        status2 = Text(f"doubtful hit {2 if n > 1 else 1} of {n}", font=FONT, font_size=26, color=ORANGE).move_to(status)
        target = self.playhead(nxt)
        self.t_play(FadeOut(status), head.animate.move_to(target.get_center()), t=1.0)
        self.t_play(FadeIn(status2), t=1.0)
        self.say("Lowest confidence first, across all tracks.", t=2.0)

    def s04_why(self):
        """The first doubtful hit zoomed, its waveform, and the info line as the app writes it."""
        first = self.queue[0]
        t0 = round(first["t"] * SR) - int(LEAD_MS / 1000 * SR)
        mic = first["track"]
        wave = lane_wave(mic, t0, ZOOM_MS, 0.9, half=0.9, color=COBALT)
        x = -PLOT_W / 2 + PLOT_W * LEAD_MS / ZOOM_MS
        hit = Line([x, -0.3, 0], [x, 2.1, 0], color=ORANGE, stroke_width=4)
        title = Text(f"the first doubtful hit of the queue, on the {mic} track", font=FONT, font_size=26,
                     color=TEXT).to_edge(UP, buff=0.8)
        self.t_play(Create(wave), FadeIn(title), t=3.0)
        self.t_play(FadeIn(hit), t=1.5)
        label = Text("info line, Doubt", font=FONT, font_size=20, color=DIM).move_to([0, -1.2, 0])
        line = Text(info_line(first), font=FONT, font_size=24, color=ORANGE)
        if line.width > PLOT_W:
            line.scale_to_fit_width(PLOT_W)
        line.move_to([0, -1.8, 0])
        box = Rectangle(width=PLOT_W + 0.4, height=1.3, color=DIM, stroke_width=1).move_to([0, -1.5, 0])
        self.t_play(Create(box), FadeIn(label), FadeIn(line), t=3.0)
        self.say("The app says what to check.", t=2.0)

    def s05_decide(self):
        """Audition, keep, delete or nudge; your edit decides."""
        rows = ["A: audition the hit", "a real hit: keep it", "not a hit: Delete", "a little off: Left or Right"]
        shown = VGroup(*[Text(t, font=FONT, font_size=30, color=TEXT).move_to([0, 2.2 - 0.9 * i, 0])
                         for i, t in enumerate(rows)])
        for row in shown:
            self.t_play(FadeIn(row), t=1.8)
        decides = Text("your edit decides", font=FONT, font_size=34, color=ORANGE).move_to([0, -1.7, 0])
        self.t_play(FadeIn(decides), t=2.0)
        self.say("The queue only points. It changes no hit.", t=2.0)

    def s06_result(self):
        """The same hits as before; the few doubtful ones marked; Export."""
        n = len(self.queue)
        total = self.hit_count()
        counts = Text(f"{n} doubtful hits to check, the other {total - n} stay as they are", font=FONT, font_size=28,
                      color=TEXT).to_edge(UP, buff=0.9)
        self.t_play(Create(self.lane_frame()), FadeIn(self.stalks(False)), t=3.0)
        self.t_play(FadeIn(counts), t=2.0)
        export = Text("then Export", font=FONT, font_size=32, color=ORANGE).next_to(counts, DOWN, buff=0.3)
        self.t_play(FadeIn(export), t=2.0)
        self.t_wait(1.0)
