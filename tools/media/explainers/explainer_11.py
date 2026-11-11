"""#484 explainer 11, "Learn, teach by example" (chapter 11). Script: docs/product/scripts/explainer-11-learn.md.

The picture uses the real demo session (tools/media/demo_session.py) through explainer_10's
helpers: the snare top mic's WAV and truth.json are read from ~/.cache/drumspike/media/demo/
(generated on demand with the default seed). The story's anchors are the session's real ones:
the unwanted ghost at 412500 and its near copy at 457500 (both truth ghosts on the snare), the
missed soft ghost at 2257500 (the softest of the session, v 0.151), the three ghosts the Add
rule brings in at 2332500, 2362500 and 2377500, and the kick's copy on the snare mic at the
loud bar (the kit kick at 1980000; the copy's delay is measured from the audio at render time).
The Learn marks, the grey hits and the histogram are the feature's documented behavior (the
script's "Facts used") drawn in the app's colours. Render through build.py.
"""
import numpy as np
from manim import Circle

from explainer_common import (
    COBALT, DIM, DOWN, LEFT, ORANGE, RIGHT, TEXT, UP, Create, Dot, DashedLine, Explainer, FadeIn,
    FadeOut, FONT, Line, Rectangle, Text, VGroup, wave,
)
from explainer_10 import SR, envelope, first_cross, lane_label, lane_line, lane_wave, read_seg, truth

PLOT_W = 11.0
MIC = "s top"
# The story's anchors (truth.json samples at 48 kHz):
GHOST_WANT = 412500    # the ghost the rules keep and the engineer does not want (v 0.198)
GHOST_NEAR = 457500    # its near copy (v 0.252)
GHOST_MISS = 2257500   # the softest ghost of the session (v 0.151): under the Sensitivity
GHOST_ADDS = (2332500, 2362500, 2377500)  # the missed ones the Add rule brings in
KIT_KICK = 1980000    # the loud bar's kick: its copy on the snare mic is the bleed
THIS_COLOR = "#4cc38a"  # the app's --learn-this (static/learn.js)
NOT_COLOR = "#e5534b"   # the app's --learn-not


def snare_hits():
    """The snare hits of the demo session, in time order (the ghosts among them)."""
    return sorted([h["t"] for h in truth()["hits"] if h["drum"] == "snare"])


class Lane:
    """One drawn lane: the mic's real waveform over `ms` at `t0`, centred on `yc`.

    x(t) maps a sample time onto the drawn span, so stalks, mark bars and rings sit on the wave
    they belong to (the same mapping lane_wave draws with).
    """

    def __init__(self, mic, t0, ms, yc, half=0.55, color=COBALT, step=4):
        self.mic, self.t0, self.ms, self.yc, self.half = mic, t0, ms, yc, half
        self.group = VGroup(lane_line(yc), lane_wave(mic, t0, ms, yc, half, color, step=step),
                            lane_label(mic, yc))

    def x(self, t):
        return -PLOT_W / 2 + PLOT_W * (t - self.t0) / (self.ms / 1000 * SR)


def overview_x(t):
    """The x of a sample time on a whole-song lane (no wave drawn at this zoom, as in 10)."""
    return -PLOT_W / 2 + PLOT_W * t / truth()["duration_samples"]


def stalk(lane, t, color=ORANGE, half=0.35, width=3):
    """One hit of the lane: a vertical line at its time, the lane's own height."""
    return Line([lane.x(t), lane.yc + half, 0], [lane.x(t), lane.yc - half, 0],
                color=color, stroke_width=width)


def mark_bar(lane, times, kind):
    """A Learn mark at the top of the lane, as the app draws it (static/learn.js): a thin bar
    over the examples' span, a dot per example of a Learn This mark, a cross per Forget one."""
    xs = [lane.x(t) for t in times]
    x0, x1 = min(xs) - 0.12, max(xs) + 0.12
    color = THIS_COLOR if kind == "this" else NOT_COLOR
    bar = Rectangle(width=max(x1 - x0, 0.3), height=0.07, color=color, stroke_width=1,
                    fill_color=color, fill_opacity=1).move_to([(x0 + x1) / 2, lane.yc + 0.78, 0])
    ticks = VGroup()
    for x in xs:
        if kind == "this":
            ticks.add(Dot([x, lane.yc + 0.78, 0], radius=0.045, color=color))
        else:
            ticks.add(Line([x - 0.05, lane.yc + 0.83, 0], [x + 0.05, lane.yc + 0.73, 0],
                           color=color, stroke_width=2),
                      Line([x - 0.05, lane.yc + 0.73, 0], [x + 0.05, lane.yc + 0.83, 0],
                           color=color, stroke_width=2))
    return VGroup(bar, ticks)


def bleed_delay():
    """The kick's copy on the snare mic at the loud bar: its delay after the kick's own close
    mic, measured from the session's audio the way explainer 10 measures its copies."""
    t0 = KIT_KICK - int(0.005 * SR)
    ref = read_seg("k in", t0, int(0.04 * SR))
    x = read_seg(MIC, t0, int(0.04 * SR))
    thr = 0.05 * float(np.max(np.abs(ref)))
    return (first_cross(x, thr) - first_cross(ref, thr)) / SR * 1000.0


def copy_stalk(lane, color=DIM, half=0.35, width=3):
    """The kick's copy on this lane: the real delay after the kick of the loud bar."""
    return stalk(lane, KIT_KICK + int(bleed_delay() / 1000 * SR), color, half, width)


def strip_x(t, w0, w1):
    """The x of a sample time on a stretch drawn from w0 to w1 across the plot width."""
    return -PLOT_W / 2 + PLOT_W * (t - w0) / (w1 - w0)


def cross(x, yc, half=0.09):
    """A cross over a hit, in the app's Forget colour (static/learn.js draws one per Forget mark)."""
    return VGroup(Line([x - half, yc + half, 0], [x + half, yc - half, 0],
                       color=NOT_COLOR, stroke_width=2.5),
                  Line([x - half, yc - half, 0], [x + half, yc + half, 0],
                       color=NOT_COLOR, stroke_width=2.5))


def legend_entry(stalk_color, mark, label):
    """One legend row: a mini stalk with its mark over it, then the word."""
    row = VGroup(Line([0, 0.16, 0], [0, -0.16, 0], color=stalk_color, stroke_width=3), mark)
    return VGroup(row, Text(label, font=FONT, font_size=18, color=TEXT).next_to(row, RIGHT, buff=0.18))


class Explainer11(Explainer):
    script_ids = (
        "s01-track", "s02-forget", "s03-learn", "s04-add",
        "s05-bleed", "s06-safe", "s07-project", "s08-result",
    )

    def s01_track(self):
        """The snare track of the song: the rules keep the ghosts, and miss the softest one."""
        title = Text("the snare track of this song", font=FONT, font_size=24, color=TEXT)
        title.to_edge(UP, buff=0.7)
        lines = VGroup(lane_line(0.0), lane_label(MIC, 0.0))
        orange = VGroup(*[Line([overview_x(t), 0.2, 0], [overview_x(t), -0.2, 0],
                               color=ORANGE, stroke_width=2) for t in snare_hits()])
        self.t_play(Create(lines), Create(orange), FadeIn(title), t=3.0)
        ring = Circle(radius=0.28, color=ORANGE, stroke_width=3).move_to([overview_x(GHOST_WANT), 0.0, 0])
        missed = Text("no hit here", font=FONT, font_size=20, color=DIM)
        missed.move_to([overview_x(GHOST_MISS), 0.95, 0])  # review 6396207826 #4: off the lane's wave
        self.t_play(Create(ring), FadeIn(missed), t=2.5)
        self.say("Here is a ghost note you do not want, and a soft hit you do want that the detector never found.", t=2.0)
        self.say("The Sensitivity slider would change the whole track; Learn works from your examples.", t=2.0)

    def s02_forget(self):
        """Forget This on the unwanted ghost: it and its near copy go grey, the backbeats stay."""
        lane = Lane(MIC, GHOST_WANT - int(0.1 * SR), 1200, 0.4)
        hits = [t for t in snare_hits() if lane.t0 <= t < lane.t0 + int(1.2 * SR)]
        pic = VGroup(lane.group, *[stalk(lane, t) for t in hits])
        title = Text("Hits > Forget This (Shift+K)", font=FONT, font_size=24, color=TEXT)
        title.to_edge(UP, buff=0.7)
        self.t_play(Create(pic), FadeIn(title), t=2.5)
        bar = mark_bar(lane, [GHOST_WANT], "not")
        self.t_play(FadeIn(bar), t=1.5)
        grey = VGroup(*[stalk(lane, t, color=DIM) for t in (GHOST_WANT, GHOST_NEAR)])
        keep = Text("the loud hits stay", font=FONT, font_size=20, color=TEXT)
        # review 6806569693: the label was cut by the frame edge; it sits inside the plot width now
        keep.move_to([PLOT_W / 2 - 0.3 - keep.width / 2, lane.yc + 1.0, 0])
        self.t_play(FadeIn(grey), FadeIn(keep), t=1.5)
        self.say("Select the unwanted hit, Shift+K: the hit and its near copies turn grey at once.", t=1.8)
        self.say("Grey is not deleted: the wave keeps the hit, the exports leave it out.", t=1.8)

    def s03_learn(self):
        """Learn This on the missed soft one: the strongest rise is the example, the hit is added."""
        lane = Lane(MIC, GHOST_MISS - int(0.1 * SR), 400, 1.3)
        title = Text("Hits > Learn This (K)", font=FONT, font_size=24, color=TEXT)
        title.to_edge(UP, buff=0.7)
        self.t_play(Create(lane.group), FadeIn(title), t=2.5)
        # the attack envelope of the same stretch: the rise the rule takes is the ghost's own
        x = read_seg(MIC, lane.t0, int(0.4 * SR))
        env = envelope(x)[::4]
        pk = float(np.max(env)) or 1.0
        curve = wave([float(1.6 * v / pk - 0.4) for v in env], -PLOT_W / 2, PLOT_W / 2, -0.9, 0.7, ORANGE)
        note = Text("the strongest rise of 6 dB or more", font=FONT, font_size=20, color=TEXT)
        note.move_to([0.0, -1.9, 0])  # under the envelope, off both waves
        self.t_play(Create(curve), FadeIn(note), t=2.0)
        ring = Circle(radius=0.2, color=ORANGE, stroke_width=3)
        idx = (GHOST_MISS - lane.t0) // 4  # the env index of the ghost's attack
        ring.move_to([lane.x(GHOST_MISS), -0.9 + 0.7 * (1.6 * float(env[idx]) / pk - 0.4), 0])
        self.t_play(Create(ring), t=1.5)
        bar = mark_bar(lane, [GHOST_MISS], "this")
        added = stalk(lane, GHOST_MISS)
        tag = Text("added, learned", font=FONT, font_size=20, color=TEXT)
        tag.next_to([lane.x(GHOST_MISS), lane.yc, 0], UP, buff=0.35)
        self.t_play(FadeIn(bar), Create(added), FadeIn(tag), t=2.5)
        self.say("Select a stretch around it, K: the step takes the strongest rise of 6 dB or more.", t=2.0)
        self.say("The hit is added and learned as your example.", t=1.6)

    def s04_add(self):
        """The Inspector's Learn section: the histogram with Match and Add, and the added hits."""
        title = Text("the Inspector's Learn section", font=FONT, font_size=24, color=TEXT)
        title.to_edge(UP, buff=0.7)
        panel = Rectangle(width=PLOT_W, height=2.0, color=DIM, stroke_width=1).move_to([0.0, 1.6, 0])
        heights = [0.25, 0.5, 0.95, 1.3, 1.05, 0.65, 0.38, 0.22, 0.14, 0.08]
        nb = len(heights)
        bars = VGroup()
        for i, h in enumerate(heights):
            x = -PLOT_W / 2 + 0.5 + PLOT_W * (i + 0.5) / (nb + 1) - 0.5
            bars.add(Rectangle(width=0.55, height=h, color=COBALT, stroke_width=1,
                               fill_color=COBALT, fill_opacity=0.5).move_to([x, 0.75 + h / 2, 0]))
        self.t_play(FadeIn(title), Create(panel), Create(bars), t=2.0)
        w = PLOT_W - 1.0
        match = Line([-PLOT_W / 2 + 0.5 + 0.80 * w, 0.75, 0], [-PLOT_W / 2 + 0.5 + 0.80 * w, 2.5, 0],
                     color=NOT_COLOR, stroke_width=2)
        add = Line([-PLOT_W / 2 + 0.5 + 0.85 * w, 0.75, 0], [-PLOT_W / 2 + 0.5 + 0.85 * w, 2.5, 0],
                   color=THIS_COLOR, stroke_width=2)
        mlabel = Text("Match 0.80", font=FONT, font_size=18, color=NOT_COLOR)
        mlabel.next_to(match, LEFT, buff=0.15).set_y(2.35)  # #210: the labels above the bars
        alabel = Text("Add 0.85", font=FONT, font_size=18, color=THIS_COLOR)
        alabel.next_to(add, RIGHT, buff=0.15).set_y(2.35)
        counts = Text("5 hits · Learn takes 2 out", font=FONT, font_size=20, color=TEXT)
        counts.move_to([0.0, 0.25, 0])  # under the panel, above the lane
        self.t_play(Create(match), Create(add), FadeIn(mlabel), FadeIn(alabel), FadeIn(counts), t=2.5)
        lane = Lane(MIC, GHOST_ADDS[0] - int(0.05 * SR), 1300, -1.5, half=0.4)
        hits = [t for t in snare_hits() if lane.t0 <= t < lane.t0 + int(1.3 * SR)]
        pic = VGroup(lane.group, *[stalk(lane, t, color=DIM) for t in hits])
        self.t_play(Create(pic), t=2.0)
        orange = VGroup(*[stalk(lane, t) for t in GHOST_ADDS])
        tag = Text("added, learned", font=FONT, font_size=20, color=TEXT)
        tag.move_to([lane.x(GHOST_ADDS[1]), lane.yc + 1.05, 0])
        self.t_play(FadeIn(orange), FadeIn(tag), t=1.5)
        self.say("From then on the track watches the hits below the Sensitivity.", t=1.6)
        self.say("The ones that look like your example come in; a Forget example or your own hit keeps one out.", t=1.6)

    def s05_bleed(self):
        """The loud bar on the snare mic: the kick's copy; the same Shift+K takes it too."""
        lane = Lane(MIC, KIT_KICK - int(0.1 * SR), 400, 0.6)
        title = Text("the same Shift+K reaches further now", font=FONT, font_size=24, color=TEXT)
        title.to_edge(UP, buff=0.7)
        self.t_play(Create(lane.group), FadeIn(title), t=2.5)
        delay = bleed_delay()
        copy = copy_stalk(lane)
        tag = Text(f"the kick's copy, {delay:.1f} ms after the kick", font=FONT, font_size=20, color=TEXT)
        tag.move_to([lane.x(KIT_KICK), lane.yc - 1.05, 0])  # under the lane, off the wave
        self.t_play(Create(copy), FadeIn(tag), t=2.0)
        grey = copy_stalk(lane)
        self.t_play(FadeOut(copy), FadeIn(grey), t=1.5)
        self.say("Your example told the filter the difference between the hits you want and the rest.", t=1.8)
        self.say("The similar bleed on this track goes grey with the same keystroke.", t=1.8)

    def s06_safe(self):
        """What stays safe: grey is not deleted, a hand edit wins, and the guard asks first."""
        lane = Lane(MIC, GHOST_WANT - int(0.1 * SR), 1200, 1.5)
        title = Text("what stays safe", font=FONT, font_size=24, color=TEXT).to_edge(UP, buff=0.7)
        grey = VGroup(*[stalk(lane, t, color=DIM) for t in (GHOST_WANT, GHOST_NEAR)])
        note = Text("not deleted: still in the lane", font=FONT, font_size=20, color=TEXT)
        note.move_to([-3.4, 0.35, 0])  # under the lane, clear of the dialog on the right
        self.t_play(Create(lane.group), FadeIn(title), FadeIn(grey), FadeIn(note), t=2.5)
        back = stalk(lane, GHOST_WANT)
        edit = Text("your hand edit wins", font=FONT, font_size=20, color=TEXT)
        edit.next_to([lane.x(GHOST_WANT), lane.yc, 0], UP, buff=0.35)
        self.t_play(FadeOut(grey[0]), Create(back), FadeIn(edit), t=2.0)
        box = Rectangle(width=4.6, height=1.7, color=DIM, stroke_width=1).move_to([2.4, -1.7, 0])
        dtitle = Text("Learn adds many hits", font=FONT, font_size=20, color=TEXT)
        dtitle.move_to([2.4, -1.15, 0])
        dmsg = Text("more than 10 hits, or a quarter of the track", font=FONT, font_size=18, color=DIM)
        if dmsg.width > 4.3:
            dmsg.scale_to_fit_width(4.3)  # the guard sees text on text; the box border is on me
        dmsg.move_to([2.4, -1.6, 0])
        keep = Rectangle(width=1.2, height=0.5, color=TEXT, stroke_width=1).move_to([1.7, -2.15, 0])
        keep_t = Text("Keep", font=FONT, font_size=18, color=TEXT).move_to(keep)
        undo = Rectangle(width=1.2, height=0.5, color=TEXT, stroke_width=1).move_to([3.1, -2.15, 0])
        undo_t = Text("Undo", font=FONT, font_size=18, color=TEXT).move_to(undo)
        dialog = VGroup(box, dtitle, dmsg, keep, keep_t, undo, undo_t)
        self.t_play(FadeIn(dialog), t=2.5)
        self.say("And you stay safe.", t=1.5)
        self.say("A hit the filter takes out goes grey; it is not deleted.", t=1.8)
        self.say("An edited hit is never filtered, so your hand edits win.", t=1.8)
        self.say("And when one step adds or takes out a lot, Drum Spike asks first: Keep, or Undo.", t=1.8)

    def s07_project(self):
        """The marks are kept in the project: both bars, the Inspector's counts."""
        forget_lane = Lane(MIC, GHOST_WANT - int(0.1 * SR), 1200, 1.2)
        hits = [t for t in snare_hits() if forget_lane.t0 <= t < forget_lane.t0 + int(1.2 * SR)]
        states = VGroup(forget_lane.group)
        for t in hits:
            color = DIM if t in (GHOST_WANT, GHOST_NEAR) else ORANGE
            states.add(stalk(forget_lane, t, color=color))
        learn_lane = Lane(MIC, GHOST_MISS - int(0.1 * SR), 400, -1.2, half=0.4)
        states.add(learn_lane.group, stalk(learn_lane, GHOST_MISS))
        title = Text("kept in the project", font=FONT, font_size=24, color=TEXT).to_edge(UP, buff=0.7)
        self.t_play(Create(states), FadeIn(title), t=2.5)
        bars = VGroup(mark_bar(forget_lane, [GHOST_WANT], "not"), mark_bar(learn_lane, [GHOST_MISS], "this"))
        counts = Text("1 learn · 2 forget examples · up to 64 of each kind", font=FONT,
                      font_size=20, color=TEXT).to_edge(DOWN, buff=1.2)
        self.t_play(FadeIn(bars), FadeIn(counts), t=2.0)
        self.say("Open the song tomorrow: the marks are still there, up to 64 of each kind on a track.", t=1.8)
        self.say("Every step is one undo step.", t=1.4)

    def s08_result(self):
        """The snare track before and after: the unwanted grey, the missed in, counts on screen,
        and the two stretches that hold the changes drawn larger with the marks and the legend."""
        before_line = VGroup(lane_line(2.4), lane_label(MIC, 2.4))
        after_line = VGroup(lane_line(1.1), lane_label(MIC, 1.1))
        hits = snare_hits()
        out = (GHOST_WANT, GHOST_NEAR)
        copy_at = KIT_KICK + int(bleed_delay() / 1000 * SR)
        before = VGroup(*[Line([overview_x(t), 2.6, 0], [overview_x(t), 2.2, 0],
                               color=ORANGE, stroke_width=2) for t in hits],
                        Line([overview_x(copy_at), 2.6, 0], [overview_x(copy_at), 2.2, 0],
                             color=ORANGE, stroke_width=2))
        after = VGroup(*[Line([overview_x(t), 1.3, 0], [overview_x(t), 0.9, 0],
                              color=DIM if t in out else ORANGE, stroke_width=2) for t in hits],
                       Line([overview_x(copy_at), 1.3, 0], [overview_x(copy_at), 0.9, 0],
                            color=DIM, stroke_width=2),
                       *[Line([overview_x(t), 1.3, 0], [overview_x(t), 0.9, 0],
                              color=THIS_COLOR, stroke_width=2) for t in GHOST_ADDS])
        label_b = Text("before", font=FONT, font_size=20, color=TEXT).move_to([5.4, 2.95, 0])
        label_a = Text("after", font=FONT, font_size=20, color=TEXT).move_to([5.4, 1.65, 0])
        # review 6806569693: at full length the changed hits are a few thin stalks (the added
        # ones 0.055 units apart), so the two stretches that hold them are drawn larger under
        # the lanes, with a cross over each hit that went out, a ring over each added one.
        w0, w1 = GHOST_WANT - int(0.75 * SR), copy_at + int(2.0 * SR)
        strip_a = VGroup(lane_line(-0.3),
                         *[Line([strip_x(t, w0, w1), -0.05, 0], [strip_x(t, w0, w1), -0.55, 0],
                                color=DIM if t in out else ORANGE, stroke_width=2)
                           for t in hits if w0 <= t <= w1],
                         Line([strip_x(copy_at, w0, w1), -0.05, 0],
                              [strip_x(copy_at, w0, w1), -0.55, 0], color=DIM, stroke_width=2))
        crosses = VGroup(*[cross(strip_x(t, w0, w1), -0.3) for t in out],
                         cross(strip_x(copy_at, w0, w1), -0.3))
        w2, w3 = GHOST_ADDS[0] - int(0.75 * SR), GHOST_ADDS[2] + int(0.5 * SR)
        strip_b = VGroup(lane_line(-1.5),
                         *[Line([strip_x(t, w2, w3), -1.25, 0], [strip_x(t, w2, w3), -1.75, 0],
                                color=THIS_COLOR if t in GHOST_ADDS else ORANGE, stroke_width=2)
                           for t in hits if w2 <= t <= w3])
        rings = VGroup(*[Circle(radius=0.13, color=THIS_COLOR, stroke_width=2.5)
                         .move_to([strip_x(t, w2, w3), -1.5, 0]) for t in GHOST_ADDS])
        legend = VGroup(legend_entry(DIM, cross(0.0, 0.0), "out"),
                        legend_entry(THIS_COLOR,
                                     Circle(radius=0.12, color=THIS_COLOR, stroke_width=2.5),
                                     "added")).arrange(RIGHT, buff=0.8).move_to([0.0, -2.2, 0])
        self.t_play(Create(before_line), Create(before), Create(after_line), Create(after),
                    Create(strip_a), Create(strip_b), FadeIn(label_b), FadeIn(label_a), t=3.0)
        n_out, n_in = 3, len(GHOST_ADDS)
        counts = Text(f"the snare track: {n_out} hits out, {n_in} added, the rest untouched",
                      font=FONT, font_size=20, color=TEXT).to_edge(DOWN, buff=1.2)
        self.t_play(Create(crosses), Create(rings), FadeIn(legend), FadeIn(counts), t=1.5)
        self.say("The ghosts you do not want are out of the exports, the missed hits play again.", t=1.8)
        self.say("You taught it in three keystrokes.", t=1.4)
