"""#486 explainer 13, "Tempo, bars and beats" (chapter 13). Script: docs/product/scripts/explainer-13-tempo.md.

The picture is the real answer of the app on the generated session (tools/media/demo_session.py, WAV
files in ~/.cache/drumspike/media/demo/): the service tool of the repo's devtools opens a project of
all the tracks, analyses it, asks the service for the hits of a kick and a snare track (`events_get`)
and for the tempo that Detect Tempo proposes (`tempo_map_detect`, read only), as the dialog does. The
dialog's lines are the page's (`tmDetectDialog` of static/tempomap.js: whole BPM, at most three other
tempos, Half time when the half is above 40 BPM). The grid is drawn from the proposed map. Build the
tool first (`cargo build -p drumspike-devtools --bin service_tool`) or point DRUMSPIKE_SERVICE_TOOL at
a built one. The numbers of the rules are the script's "Facts used". Render through build.py.
"""
import json
import subprocess
import tempfile
from pathlib import Path

from explainer_09 import ROOT, load, service_tool
from explainer_10 import SR, demo_dir, truth
from explainer_common import (
    CLEAR_SECONDS, COBALT, DIM, DOWN, ORANGE, TEXT, UP, Arrow, Create, Explainer, FadeIn, FadeOut, FONT, Line,
    Rectangle, Text, VGroup,
)

PLOT_W = 11.0
TRACKS = ("k in", "s top")   # a kick and a snare track; the dialog reads the kick and snare tracks
BASE = {"k in": 0.3, "s top": -1.9}   # the baseline of each track's lane
HEIGHT = 1.2
WINDOW_S = 10.0   # the stretch of the song that the bar scenes show
BEATS_PER_BAR = 4   # the map's meter is 4/4 (the suggestion's meter)
HALF_MIN_BPM = 40.0   # `half > 40` of tmDetectDialog: the Half time button shows above this tempo
MAX_ALTERNATIVES = 3   # `.slice(0, 3)` of tmDetectDialog
RULER_Y = 2.15

_SESSION = None


def whole(bpm):
    """The page's `whole`: a tempo as a whole number of BPM."""
    return str(round(bpm))


def percent(x):
    """The page's `pct`: a fraction as a whole percent, with the sign."""
    return f"{round(x * 100)} %"


def session():
    """{hits: {track: [t]}, detect: the answer of tempo_map_detect}, from the real service (computed once)."""
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
        steps += [{"cmd": "events_get", "args": {"sid": "$sid", "name": n}} for n in TRACKS]
        steps.append({"cmd": "tempo_map_detect", "args": {"sid": "$sid"}})
        (tmp / "steps.json").write_text(json.dumps(steps))
        done = subprocess.run([str(service_tool()), "run", str(tmp / "steps.json")], capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit(f"FAIL: service tool exited {done.returncode}: {done.stderr[-400:]}")
    rows = [json.loads(line) for line in done.stdout.splitlines()]
    for row in rows:
        if "error" in row:
            raise SystemExit(f"FAIL: service tool step {row['cmd']}: {row['error']}")
    gets = [r["answer"] for r in rows if r["cmd"] == "events_get"]
    detect = [r["answer"] for r in rows if r["cmd"] == "tempo_map_detect"][0]
    _SESSION = {"hits": {n: [float(e["t"]) for e in g["events"]] for n, g in zip(TRACKS, gets)}, "detect": detect}
    return _SESSION


def alternatives_of(detect):
    """The other tempos of the dialog's "The beat could also be" line: not gated, at most three, whole BPM."""
    return [whole(a["bpm"]) for a in detect["start"]["alternatives"] if not a["gated"]][:MAX_ALTERNATIVES]


def half_of(detect):
    """The half tempo that the Half time button offers, or None when it is not above the limit (it stays hidden)."""
    half = detect["suggestion"]["bpm"] / 2
    return half if half > HALF_MIN_BPM else None


def grid_of(bar1_s, bpm, window=WINDOW_S):
    """The beats of the map inside the window: [(t, bar number, beat in the bar)] from bar 1 on, 4/4."""
    beat = 60.0 / bpm
    out, k = [], 0
    while bar1_s + k * beat <= window:
        out.append((bar1_s + k * beat, k // BEATS_PER_BAR + 1, k % BEATS_PER_BAR + 1))
        k += 1
    return out


def x_of(t, span):
    return -PLOT_W / 2 + PLOT_W * t / span


class Explainer13(Explainer):
    script_ids = ("s01-problem", "s02-detect", "s03-bar1", "s04-steps", "s05-grid", "s06-result")
    # The seconds that each scene's steps take at pace 1 (the sum of its t= values). A scene whose narration is
    # longer slows its steps down (never speeds them up), so the holds sit between the steps, not at the end.
    # The last scene keeps pace 1: its result frame is the poster, taken at the middle of the scene.
    PLAN = {"s01-problem": 8.0, "s02-detect": 12.0, "s03-bar1": 9.5, "s04-steps": 10.0, "s05-grid": 9.0}

    def setup(self):
        super().setup()
        data = session()
        self.hits = data["hits"]
        self.detect = data["detect"]
        self.dur = truth()["duration_samples"] / SR
        self.bpm = self.detect["suggestion"]["bpm"]
        self.bar1 = self.detect["suggestion"]["bar1_s"]
        self.later = self.detect["bar1"]["alternative"]   # the other candidate for bar 1
        assert tuple(self.detect["suggestion"]["meter"]) == (BEATS_PER_BAR, 4), "the picture is drawn in 4/4"
        assert half_of(self.detect) is not None, "the session's tempo is too low for a Half time button"
        self.pace = 1.0

    def begin(self, sid):
        super().begin(sid)
        plan = self.PLAN.get(sid)
        self.pace = max(1.0, (self.narration[sid] - CLEAR_SECONDS - 0.3) / plan) if plan else 1.0

    def end(self):
        self.pace = 1.0   # the closing fade and the pad to the narration run at their own length
        super().end()

    def t_play(self, *anims, t=2.0):
        super().t_play(*anims, t=t * self.pace)

    def t_wait(self, t):
        super().t_wait(t * self.pace)

    # ---- the lanes ----

    def lane_frame(self):
        out = VGroup()
        for n in TRACKS:
            y = BASE[n]
            out.add(Line([-PLOT_W / 2, y, 0], [PLOT_W / 2, y, 0], color=DIM))
            out.add(Text(n, font=FONT, font_size=22, color=TEXT).move_to([-6.3, y + HEIGHT / 2, 0]))
        return out

    def stalks(self, span, color_of=None):
        """The hits of both tracks inside 0..span seconds as stalks from the baseline."""
        out = VGroup()
        for n in TRACKS:
            color = COBALT if n == TRACKS[0] else ORANGE
            for t in self.hits[n]:
                if t <= span:
                    x = x_of(t, span)
                    out.add(Line([x, BASE[n], 0], [x, BASE[n] + HEIGHT * 0.8, 0], color=color, stroke_width=2))
        return out

    def hit_count(self):
        return sum(len(v) for v in self.hits.values())

    # ---- the rulers and the grid ----

    def ruler(self, bar1_s, bpm=None):
        """The Bars|Beats ruler over the lanes: a tick per beat, a longer one and the bar number at each bar."""
        bpm = bpm or self.bpm
        out = VGroup()
        out.add(Line([-PLOT_W / 2, RULER_Y, 0], [PLOT_W / 2, RULER_Y, 0], color=DIM))
        for t, bar, beat in grid_of(bar1_s, bpm):
            x = x_of(t, WINDOW_S)
            if beat == 1:
                out.add(Line([x, RULER_Y, 0], [x, RULER_Y + 0.3, 0], color=TEXT, stroke_width=3))
                out.add(Text(str(bar), font=FONT, font_size=18, color=TEXT).move_to([x + 0.18, RULER_Y + 0.43, 0]))
            else:
                out.add(Line([x, RULER_Y, 0], [x, RULER_Y + 0.14, 0], color=DIM, stroke_width=2))
        return out

    def grid_lines(self, bar1_s):
        """The bar and beat lines through both lanes, under the hits: bar lines bright, beat lines faint."""
        out = VGroup()
        top, bottom = BASE[TRACKS[0]] + HEIGHT + 0.1, BASE[TRACKS[1]] - 0.1
        for t, bar, beat in grid_of(bar1_s, self.bpm):
            x = x_of(t, WINDOW_S)
            out.add(Line([x, bottom, 0], [x, top, 0], color=TEXT if beat == 1 else DIM,
                         stroke_width=3 if beat == 1 else 1))
        return out

    def bar1_marker(self, t):
        x = x_of(t, WINDOW_S)
        return VGroup(Line([x, RULER_Y - 0.1, 0], [x, BASE[TRACKS[1]] - 0.1, 0], color=ORANGE, stroke_width=4),
                      Text("Bar 1", font=FONT, font_size=20, color=ORANGE).move_to([x + 0.55, RULER_Y - 0.35, 0]))

    def title(self, text):
        return Text(text, font=FONT, font_size=26, color=TEXT).to_edge(UP, buff=0.8)

    # ---- the scenes ----

    def s01_problem(self):
        """Kick and snare hits of one song, in time, with no bars."""
        title = self.title(f"kick and snare of one song: {self.hit_count()} hits")
        none = Text("no bars, no beats", font=FONT, font_size=26, color=DIM).next_to(title, DOWN, buff=0.3)
        self.t_play(Create(self.lane_frame()), t=2.0)
        self.t_play(FadeIn(self.stalks(self.dur)), FadeIn(title), t=3.5)
        self.t_play(FadeIn(none), t=1.5)
        self.say("Hits alone have no bars.", t=1.0)

    def s02_detect(self):
        """The Detect Tempo dialog as the app writes it, then the three answers."""
        d = self.detect
        panel = Rectangle(width=10.0, height=5.0, color=DIM, stroke_width=2).move_to([0, 0.2, 0])
        head = Text("Track > Detect Tempo…", font=FONT, font_size=26, color=TEXT).next_to(panel, UP, buff=0.2)
        start = Text(f"Start tempo: {whole(self.bpm)} BPM (confidence {percent(d['start']['confidence'])})",
                     font=FONT, font_size=28, color=TEXT).move_to([0, 1.9, 0])
        also = Text(f"The beat could also be: {', '.join(alternatives_of(d))} BPM", font=FONT, font_size=24,
                    color=DIM).move_to([0, 1.1, 0])
        use = self.button(f"Use {whole(self.bpm)} BPM", -3.3)
        half = self.button(f"Half time: {whole(half_of(d))} BPM", 0.0)
        own = self.button("Use own tempo", 3.3)
        usual = Text("half time is the usual mistake", font=FONT, font_size=24, color=ORANGE).move_to([0, -1.8, 0])
        self.t_play(Create(panel), FadeIn(head), t=1.5)
        self.t_play(FadeIn(start), t=2.0)
        self.t_play(FadeIn(also), t=2.0)
        self.t_play(FadeIn(use), FadeIn(half), FadeIn(own), t=2.0)
        self.t_play(FadeIn(usual), half[0].animate.set_color(ORANGE), t=1.5)
        self.t_wait(1.0)
        self.say("One click for half time.", t=2.0)

    def button(self, label, x):
        text = Text(label, font=FONT, font_size=22, color=TEXT)
        box = Rectangle(width=text.width + 0.5, height=0.7, color=DIM, stroke_width=2)
        return VGroup(box, text).move_to([x, -0.6, 0])

    def s03_bar1(self):
        """Bar 1 at the playhead, then at the audio start: the ruler's bar numbers move with it."""
        x = x_of(self.later, WINDOW_S)
        playhead = Line([x, RULER_Y - 0.1, 0], [x, BASE[TRACKS[1]] - 0.1, 0], color=TEXT, stroke_width=4)
        cmd = Text("Track > Set Bar 1 at Playhead", font=FONT, font_size=26, color=TEXT).to_edge(UP, buff=0.8)
        cmd2 = Text("Track > Set Bar 1 at Audio Start", font=FONT, font_size=26, color=TEXT).to_edge(UP, buff=0.8)
        ruler, marker = self.ruler(self.later), self.bar1_marker(self.later)
        ruler2, marker2 = self.ruler(0.0), self.bar1_marker(0.0)
        self.t_play(Create(self.lane_frame()), FadeIn(self.stalks(WINDOW_S)), t=2.0)
        self.t_play(FadeIn(cmd), FadeIn(playhead), t=1.5)
        self.t_play(FadeIn(ruler), FadeIn(marker), t=2.0)
        self.t_wait(1.5)
        self.t_play(FadeOut(ruler), FadeOut(marker), FadeOut(playhead), FadeOut(cmd), t=0.5)
        self.t_play(FadeIn(cmd2), FadeIn(ruler2), FadeIn(marker2), t=2.0)
        self.t_wait(1.0)

    def s04_steps(self):
        """A tempo step where the song changes speed (drawn: the beats come closer after it), and the import."""
        ruler = VGroup(Line([-PLOT_W / 2, 1.0, 0], [PLOT_W / 2, 1.0, 0], color=DIM))
        x, gap = -PLOT_W / 2 + 0.2, 0.9
        step_x = 0.0
        while x < PLOT_W / 2 - 0.1:
            if x > step_x:
                gap = 0.6   # after the step: faster
            ruler.add(Line([x, 1.0, 0], [x, 1.25, 0], color=TEXT, stroke_width=3))
            x += gap
        step = Line([step_x, 0.7, 0], [step_x, 1.5, 0], color=ORANGE, stroke_width=4)
        label = Text("a tempo step: the song speeds up", font=FONT, font_size=24, color=ORANGE).move_to([step_x, 1.9, 0])
        hint = Text("View > Tempo Ruler: double-click adds a step", font=FONT, font_size=24, color=TEXT).move_to([0, 0.2, 0])
        daw = self.box("your DAW", -3.6, -1.6)
        app = self.box("Drum Spike", 3.6, -1.6)
        arrow = Arrow([-2.2, -1.6, 0], [2.2, -1.6, 0], color=ORANGE, buff=0)
        imp = Text("File > Import Tempo Map…", font=FONT, font_size=24, color=TEXT).move_to([0, -2.4, 0])
        self.t_play(Create(ruler), t=2.0)
        self.t_play(Create(step), FadeIn(label), t=2.0)
        self.t_play(FadeIn(hint), t=2.0)
        self.t_play(FadeIn(daw), FadeIn(app), Create(arrow), FadeIn(imp), t=2.5)
        self.t_wait(1.5)

    def box(self, label, x, y):
        text = Text(label, font=FONT, font_size=26, color=TEXT)
        return VGroup(Rectangle(width=text.width + 0.6, height=0.9, color=DIM, stroke_width=2), text).move_to([x, y, 0])

    def s05_grid(self):
        """The grid under the hits; the key G; a guide with no snap."""
        key = Text("G: View > Show Bar and Beat Grid", font=FONT, font_size=26, color=TEXT).to_edge(UP, buff=0.8)
        guide = Text("a guide: no snap, no hit moves", font=FONT, font_size=24, color=DIM).move_to([0, -2.9, 0])
        self.t_play(Create(self.lane_frame()), FadeIn(self.stalks(WINDOW_S)), FadeIn(self.ruler(self.bar1)), t=2.0)
        self.t_play(FadeIn(key), t=1.5)
        self.t_play(Create(self.grid_lines(self.bar1)), t=3.0)
        self.t_play(FadeIn(guide), t=2.0)
        self.t_wait(0.5)

    def s06_result(self):
        """The hits on their bars in Drum Spike, and the same bars in the DAW after the export. The steps end before
        the middle of the scene, so the result is on screen for the rest of it (and for the poster)."""
        title = self.title("the MIDI export carries the map")
        daw_y = -2.5
        notes = VGroup()
        for n in TRACKS:
            for t in self.hits[n]:
                if t <= WINDOW_S:
                    x = x_of(t, WINDOW_S)
                    notes.add(Rectangle(width=0.08, height=0.22, color=COBALT if n == TRACKS[0] else ORANGE,
                                        fill_opacity=1, stroke_width=0).move_to([x, daw_y + (0.2 if n == TRACKS[0] else -0.2), 0]))
        daw_bars = VGroup()
        for t, bar, beat in grid_of(self.bar1, self.bpm):
            x = x_of(t, WINDOW_S)
            daw_bars.add(Line([x, daw_y - 0.5, 0], [x, daw_y + 0.5, 0], color=TEXT if beat == 1 else DIM,
                              stroke_width=3 if beat == 1 else 1))
        daw_name = Text("your DAW", font=FONT, font_size=22, color=TEXT).move_to([-6.3, daw_y, 0])
        same = Text("the same bars", font=FONT, font_size=32, color=ORANGE).move_to([0, -3.4, 0])
        self.t_play(Create(self.lane_frame()), FadeIn(self.stalks(WINDOW_S)), FadeIn(self.ruler(self.bar1)),
                    Create(self.grid_lines(self.bar1)), t=1.5)
        self.t_play(FadeIn(title), t=1.0)
        self.t_play(FadeIn(daw_bars), FadeIn(notes), FadeIn(daw_name), t=1.5)
        self.t_play(FadeIn(same), t=1.0)
