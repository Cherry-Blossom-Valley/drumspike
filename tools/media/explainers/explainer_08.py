"""#325 explainer 8, "From waveform to velocity" (chapter 8). Script: docs/product/scripts/explainer-08-velocity.md.

The picture is the demo session's snare track (tools/media/demo_session.py, seed 2026, cached in
~/.cache/drumspike/media/demo/): the levels are the peaks of its real hits, the velocities are the
app's default map (crates/core/src/velocity.rs), both computed in physics.py. Render through build.py.
"""
from explainer_common import (
    COBALT, DIM, DOWN, LEFT, ORANGE, RIGHT, TEXT, UP, Create, Dot, DashedLine, Explainer, FadeIn,
    FONT, Line, Rectangle, Text, VGroup, VMobject, wave,
)
from physics import (
    LO_V, RATE, demo_hit_times, demo_snare_hits, demo_snare_track, demo_track, hit_peak_db, hit_window, map_points,
    nearest_hit_time, velocity_of_db,
)

import numpy as np

PLOT_W = 11.0
GHOST, NORMAL, ACCENT = 8.594, 8.125, 40.607  # targets in s; each is snapped to the demo session's nearest hit
BINS = 40  # the histogram: 1 dB per bin from -40 to 0 dBFS
BASE = -2.0  # the baseline of the bar and stalk pictures
CUT_DB = -30.0  # the loudness cut of s06: hits under it are greyed (the tom mic's snare bleed is under it)
DOT_GAP = 0.2  # the vertical gap of a stacked dot column (#325 review: the pile stays under the title)
DOT_ROWS = 13  # dots per column; a taller pile starts a new column to the right


def picked_hits(x):
    """[(name, time, peak dBFS)] of the three hits the scenes show: a ghost note, a backbeat, the loudest hit."""
    return [(name, nearest_hit_time(t), hit_peak_db(x, nearest_hit_time(t)))
            for name, t in (("ghost", GHOST), ("normal", NORMAL), ("accent", ACCENT))]


def x_db(db):
    """The x of a level on the histogram and the map pictures: -40 dBFS at the left edge, 0 dBFS at the right."""
    return -5.5 + PLOT_W * (db + 40.0) / 40.0


def y_v(v):
    """The y of a velocity on the s04 map: LO_V at the baseline, 127 at 2.0."""
    return BASE + 4.0 * (v - LO_V) / (127 - LO_V)


def hit_labels(picks, dur):
    """The s01 labels: name and level under each wave. `dur` is the track's length in s; the three
    windows are shown side by side (not on a timeline), so the labels follow the windows' index order."""
    rows = picks.items() if isinstance(picks, dict) else [(n, {"level_db": db}) for n, _, db in picks]
    return [Text(f"{name}  {row['level_db']:.1f} dBFS", font=FONT, font_size=26, color=TEXT)
            .move_to([-3.6 + 3.6 * k, -2.2, 0]) for k, (name, row) in enumerate(rows)]


def map_xy(db, v):
    """Where a dot's label sits on the s04 map: up and to the right of the dot."""
    return x_db(db) + 0.4, y_v(v) + 0.3


def _boxes_meet(a, b):
    return (a.get_left()[0] < b.get_right()[0] and b.get_left()[0] < a.get_right()[0]
            and a.get_bottom()[1] < b.get_top()[1] and b.get_bottom()[1] < a.get_top()[1])


def map_labels(picks, velocities, xy):
    """The s04 labels: the velocity of each picked hit, placed by `xy`; a label that would meet the
    previous one moves below its dot instead (#325 review: the top-right labels stacked up)."""
    labels = []
    for name, row in picks.items():
        d, v = row["level_db"], velocities[name]
        x, y = xy(d, v)
        label = Text(f"{v}", font=FONT, font_size=24, color=TEXT).move_to([x, y, 0])
        if labels and _boxes_meet(label, labels[-1]):
            label.move_to([x, y_v(v) - 0.35, 0])
        labels.append(label)
    return labels


def stack(levels, base):
    """Dot positions for many hits that share a level: they pile up from `base` in a column capped
    at DOT_ROWS, then a new column starts to the right. Returns [(x, db, dy)] with dy above `base`."""
    rows, out = {}, []
    for db in sorted(levels):
        key = round(db * 2.0) / 2.0  # half-dB bins share a column
        row = rows.get(key, 0)
        rows[key] = row + 1
        out.append((x_db(key) + (row // DOT_ROWS) * 0.14, db, round((row % DOT_ROWS) * DOT_GAP, 6)))
    return out


def bar_labels(picks, spacing=1.5):
    """The s03 labels: one name under each bar of the two groups, `spacing` units apart (#371: the
    old 0.9 spacing made 'normal' and 'accent' touch)."""
    names = VGroup()
    for k, (name, _, _) in enumerate(picks):
        cx = -spacing + spacing * k
        names.add(Text(name, font=FONT, font_size=22, color=TEXT).move_to([-3.0 + cx, -1.9, 0]))
        names.add(Text(name, font=FONT, font_size=22, color=TEXT).move_to([3.0 + cx, -1.9, 0]))
    return names


def histogram(levels, cut=None):
    """The loudness histogram of the hit levels: one bar per dB, with a baseline. The bars under `cut` are grey."""
    counts, edges = np.histogram(levels, bins=BINS, range=(-40.0, 0.0))
    bars = VGroup()
    for i, count in enumerate(counts):
        if count == 0:
            continue
        h = 0.06 * count
        xc = x_db(edges[i] + 0.5)
        color = DIM if cut is not None and edges[i] + 1.0 <= cut else COBALT
        bars.add(Rectangle(width=PLOT_W / BINS, height=h, fill_color=color, fill_opacity=0.8,
                           stroke_width=0).move_to([xc, BASE + h / 2, 0]))
    axis = Line(LEFT * 5.5 + DOWN * 2.5, RIGHT * 5.5 + DOWN * 2.5, color=DIM)
    return VGroup(bars, axis)


def handle(db, name, top=2.4):
    """A dashed handle at a level, with its name above it."""
    line = DashedLine([x_db(db), BASE, 0], [x_db(db), top, 0], color=ORANGE)
    label = Text(f"{name} {db:.0f}", font=FONT, font_size=24, color=ORANGE).move_to([x_db(db), top + 0.35, 0])
    return VGroup(line, label)


class Explainer08(Explainer):
    script_ids = (
        "s01-question", "s02-peak", "s03-why-db", "s04-map",
        "s05-histogram", "s06-cut", "s07-lane", "s08-result",
    )

    def s01_question(self):
        x = demo_snare_track()
        picks = picked_hits(x)
        group = VGroup()
        for k, (name, t, db) in enumerate(picks):
            cx = -3.6 + 3.6 * k
            group.add(wave(hit_window(x, t)[::10], cx - 1.4, cx + 1.4, 0.0, 1.5, COBALT))
        labels = hit_labels(picks, len(x) / RATE)
        self.t_play(Create(group), FadeIn(VGroup(*labels)), t=3)
        self.say("Each hit gets a velocity, like a MIDI keyboard. A ghost note is soft. An accent is loud.", t=2)
        self.t_wait(4)

    def s02_peak(self):
        x = demo_snare_track()
        t = nearest_hit_time(NORMAL)
        w = hit_window(x, t)
        pic = wave(w[::4], -PLOT_W / 2, PLOT_W / 2, 0.0, 2.0, COBALT)
        p = int(np.argmax(np.abs(w)))
        db = 20.0 * np.log10(abs(w[p]))
        mark = Dot([-PLOT_W / 2 + PLOT_W * p / (len(w) - 1), 2.0 * w[p], 0], radius=0.1, color=ORANGE)
        label = Text(f"peak {db:.1f} dBFS", font=FONT, font_size=28, color=TEXT).next_to(mark, UP, buff=0.2)
        self.t_play(Create(pic), t=3)
        self.t_play(FadeIn(mark, label), t=2)
        self.say("The window runs from 5 ms before the hit to 60 ms after it. Its largest sample is the peak.", t=2)
        self.t_wait(6)

    def s03_why_db(self):
        x = demo_snare_track()
        picks = picked_hits(x)
        bars = VGroup()
        for k, (name, t, db) in enumerate(picks):
            cx = -1.5 + 1.5 * k  # #371: 0.9 packed the bars so 'normal' and 'accent' touched
            lin_h = 10.0 ** (db / 20.0) * 3.0  # linear amplitude, 0 to 1 at 3 units
            log_h = (db + 60.0) / 60.0 * 3.0  # decibels, -60 to 0 at 3 units
            bars.add(Rectangle(width=0.6, height=lin_h, fill_color=COBALT, fill_opacity=0.8, stroke_width=0)
                     .move_to([-3.0 + cx, -1.5 + lin_h / 2, 0]))
            bars.add(Rectangle(width=0.6, height=log_h, fill_color=ORANGE, fill_opacity=0.8, stroke_width=0)
                     .move_to([3.0 + cx, -1.5 + log_h / 2, 0]))
        names = bar_labels(picks)
        heads = VGroup(
            Text("linear amplitude", font=FONT, font_size=28, color=TEXT).move_to([-3.0, 2.3, 0]),
            Text("decibels", font=FONT, font_size=28, color=TEXT).move_to([3.0, 2.3, 0]),
        )
        self.t_play(FadeIn(bars, names, heads), t=3)
        self.say("On a linear scale the ghost note sits almost at zero. In decibels it has its own place.", t=2)
        self.t_wait(6)

    def s04_map(self):
        x = demo_snare_track()
        picks = picked_hits(x)
        levels = [db for _, _, db in picks]

        axes = VGroup(
            Line(LEFT * 5.5 + DOWN * 2.5, RIGHT * 5.5 + DOWN * 2.5, color=DIM),
            Line(LEFT * 5.5 + DOWN * 2.5, LEFT * 5.5 + UP * 2.5, color=DIM),
        )
        app_map = VMobject(color=COBALT, stroke_width=3).set_points_as_corners([
            np.array([x_db(-60.0), y_v(20), 0]), np.array([x_db(-40.0), y_v(20), 0]),
            np.array([x_db(-6.0), y_v(127), 0]), np.array([x_db(0.0), y_v(127), 0]),
        ])
        dots = VGroup()
        for d, v in map_points([-40.0] + levels):
            dots.add(Dot([x_db(d), y_v(v), 0], radius=0.09, color=ORANGE))
        # the picked hits' labels go through map_labels (the -40 floor point keeps its label here)
        rows = {name: {"level_db": db} for name, _, db in picks}
        mapped = dict(map_points([row["level_db"] for row in rows.values()]))
        velocities = {name: mapped[row["level_db"]] for name, row in rows.items()}
        labels = VGroup(Text("20", font=FONT, font_size=24, color=TEXT)
                        .move_to([x_db(-40.0) + 0.4, y_v(20) + 0.3, 0]),
                        *map_labels(rows, velocities, map_xy))
        self.t_play(Create(app_map), FadeIn(axes), t=3)
        self.t_play(FadeIn(dots, labels), t=2)
        self.say("A straight line turns the level into a velocity: 20 at the floor, 127 at the ceiling.", t=2)
        self.t_wait(6)

    def s05_histogram(self):
        x = demo_snare_track()
        levels = [hit_peak_db(x, t) for t, _ in demo_snare_hits()]
        hist = histogram(levels)
        floor = handle(-40.0, "floor")
        ceil = handle(-6.0, "ceiling")
        self.t_play(FadeIn(hist), t=3)
        self.t_play(FadeIn(floor, ceil), t=2)
        self.say("The Inspector shows a histogram of all hit levels. Drag the handles, and the velocities follow.", t=2)
        self.t_play(ceil.animate.shift(RIGHT * (x_db(-4.0) - x_db(-6.0))), t=3)
        self.t_wait(3)

    def s06_cut(self):
        # tom 1's close mic: its own tom hits, and the snare's bleed bumps under the cut (the snare track has none)
        x = demo_track("t1")
        levels = [hit_peak_db(x, t) for t in demo_hit_times(("tom", "snare"))]
        hist = histogram(levels, cut=CUT_DB)
        cut = handle(CUT_DB, "cut", top=2.4)
        self.t_play(FadeIn(hist), t=3)
        self.t_play(FadeIn(cut), t=2)
        self.say("The loudness cut leaves out the hits under it. They are not exported.", t=2)
        self.t_wait(4)

    def s07_lane(self):
        x = demo_snare_track()
        hits = [(t, hit_peak_db(x, t)) for t, _ in demo_snare_hits() if 46.5 <= t <= 49.5]
        axis = Line(LEFT * 5.5 + DOWN * 2.0, RIGHT * 5.5 + DOWN * 2.0, color=DIM)
        stalks = VGroup()
        for t, db in hits:
            xs = -5.0 + 10.0 * (t - 46.5) / 3.0
            v = velocity_of_db(db)
            stalks.add(Line([xs, BASE, 0], [xs, BASE + 3.5 * v / 127.0, 0], color=COBALT, stroke_width=4))
        drag = stalks[2]
        xs = drag.get_start()[0]
        crescendo = DashedLine(stalks[0].get_end(), stalks[-1].get_end(), color=ORANGE)
        self.t_play(FadeIn(stalks, axis), t=3)
        self.t_play(drag.animate.put_start_and_end_on([xs, BASE, 0], [xs, BASE + 3.5 * 60 / 127.0, 0]), t=2)
        self.t_play(Create(crescendo), t=2)
        self.say("Drag a stalk to change one hit. Option-drag draws a line over a crescendo.", t=2)
        self.t_wait(4)

    def s08_result(self):
        x = demo_snare_track()
        hits = [(t, hit_peak_db(x, t)) for t, _ in demo_snare_hits() if 8.0 <= t <= 14.0]
        track = Line(LEFT * 5.5 + UP * 0.5, RIGHT * 5.5 + UP * 0.5, color=COBALT)
        audio = VGroup()
        midi = VGroup()
        for t, db in hits:
            xs = -5.0 + 10.0 * (t - 8.0) / 6.0
            audio.add(Line([xs, 0.5, 0], [xs, 0.9, 0], color=ORANGE, stroke_width=3))
            v = velocity_of_db(db)
            h = 2.5 * v / 127.0
            midi.add(Rectangle(width=0.2, height=h, fill_color=DIM, fill_opacity=0.9, stroke_width=0)
                     .move_to([xs, BASE + h / 2, 0]))
        self.t_play(FadeIn(track, audio), t=3)
        self.t_play(FadeIn(midi), t=2)
        self.say("Every hit becomes a MIDI note with its velocity.", t=2)
        self.t_wait(2.5)
