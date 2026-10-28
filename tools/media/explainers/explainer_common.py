"""#246: the shared look and timing of the explainers (Manim CE). Run through build.py, which sets:

- DRUMSPIKE_EXPLAINER_NARRATION: the scenes.json of the narration (tts.py), one length per scene;
- DRUMSPIKE_EXPLAINER_PLAN: where the planned picture length per scene is written (for the gate).

Each explainer is one Scene. Its `s_<id>` methods run in script order; each one is padded with a
wait to its narration's length, so the picture and the narration start together. A scene ends
with its objects faded out (except the disclosure note), and its caption is set with `say`, one
line at a time.
"""
import json
import os

import numpy as np
from manim import (  # noqa: F401  (the explainers import the names they use from here)
    DOWN, DR, LEFT, ORIGIN, RIGHT, UP, Arrow, Create, Dot, DashedLine, FadeIn, FadeOut, Line,
    Rectangle, Scene, Text, VGroup, VMobject, ValueTracker, always_redraw, config,
)


# The app's colours (static/style.css) and the media plan's style: dark, cobalt waveform, orange hits.
BG = "#14161a"
PANEL = "#1c1f26"
TEXT = "#cdd3de"
DIM = "#7d8593"
COBALT = "#4da3ff"
ORANGE = "#ff9f43"
FONT = "Helvetica"  # one sans-serif font for all text
CAPTION_SIZE = 30
CLEAR_SECONDS = 0.5  # the fade that empties the picture at the end of each scene
NOTE = "AI-generated"  # the disclosure corner note in every video (owner, 2026-10-23)


def wave(values, x0, x1, yc, half, color=COBALT, width=2.0):
    """A waveform: values in -1..1 spread from x0 to x1, centred on yc, `half` units up and down."""
    n = len(values)
    pts = [np.array([x0 + (x1 - x0) * i / max(1, n - 1), yc + half * v, 0.0]) for i, v in enumerate(values)]
    line = VMobject(color=color, stroke_width=width)
    line.set_points_as_corners(pts)
    return line


def caption(text, width=11.0):
    """The narration's gist in one line at the bottom, small and dim, so the picture carries the scene."""
    t = Text(text, font=FONT, font_size=CAPTION_SIZE, color=TEXT)
    if t.width > width:
        t.scale_to_fit_width(width)
    return t.to_edge(DOWN, buff=0.6)


# ---------------- #459: the overlap guard ----------------

class OverlapError(Exception):
    """Two visible Text/MarkupText mobjects' bounding boxes overlap."""


def visible_texts(scene):
    """Every visible Text/MarkupText of the scene but the corner note (the note is a Text too,
    and the disclosure never counts as an overlap)."""
    from manim import MarkupText, Text
    note = getattr(scene, "note", None)
    out = []
    for m in scene.mobjects:
        for sub in m.get_family():
            if isinstance(sub, (Text, MarkupText)) and sub is not note:
                out.append(sub)
    return out


def _label(t):
    """The readable name of a text mobject: its own string when it carries one."""
    return repr(getattr(t, "text", None) or t)


def assert_no_text_overlap(scene, sid=""):
    """FAIL (OverlapError, named with the scene and the two texts) when the bounding boxes of two
    visible Text/MarkupText mobjects overlap. Touching edges are no overlap; a positive
    intersection area is."""
    texts = visible_texts(scene)
    boxes = []
    for t in texts:
        # the axis-aligned box of the text: the extreme points of the glyphs
        boxes.append((t, t.get_critical_point(LEFT)[0], t.get_critical_point(RIGHT)[0],
                      t.get_critical_point(DOWN)[1], t.get_critical_point(UP)[1]))
    eps = 1e-6
    for i in range(len(boxes)):
        for j in range(i + 1, len(boxes)):
            _, ax0, ax1, ay0, ay1 = boxes[i]
            _, bx0, bx1, by0, by1 = boxes[j]
            if min(ax1, bx1) - max(ax0, bx0) > eps and min(ay1, by1) - max(ay0, by0) > eps:
                raise OverlapError(
                    f"{sid}: two visible texts overlap: {_label(boxes[i][0])} and {_label(boxes[j][0])}")


class Explainer(Scene):
    """Base of an explainer. Subclasses define a method per scene, named after its id (`s01-hook` is `s01_hook`)."""

    script_ids = ()  # the `##` ids of the script, in order; set by each subclass

    def setup(self):
        self.camera.background_color = BG
        with open(os.environ["DRUMSPIKE_EXPLAINER_NARRATION"], encoding="utf-8") as f:
            rows = json.load(f)["scenes"]
        self.narration = {row["id"]: float(row["seconds"]) for row in rows}
        self.plan = {}
        self.sid = None
        self.elapsed = 0.0
        self.cap = None  # the caption on screen now, if any
        self.note = Text(NOTE, font=FONT, font_size=18, color=DIM).to_corner(DR, buff=0.3)
        self.add(self.note)
        # #459: the overlap guard runs on every frame of every play and wait (updaters run
        # per frame), so a transition frame with two texts on top of each other FAILs the
        # render too, not only the scene ends.
        self.note.add_updater(lambda m: self.guard_overlap())

    def guard_overlap(self):
        """The overlap guard of this frame: OverlapError (named with the scene) on an overlap."""
        assert_no_text_overlap(self, sid=self.sid or "scene")

    def construct(self):
        for sid in self.script_ids:
            self.begin(sid)
            getattr(self, sid.replace("-", "_"))()
            self.end()

    def begin(self, sid):
        self.sid = sid
        self.elapsed = 0.0
        self.cap = None
        self.next_section(sid)

    def end(self):
        """Empty the picture (all but the note), then pad the scene to its narration's length.
        A scene longer than its narration is a FAIL of the gate."""
        target = self.narration[self.sid]
        leftover = [m for m in self.mobjects if m is not self.note]
        hold = target - CLEAR_SECONDS if leftover else target  # the fade counts inside the narration's length
        if self.elapsed < hold:
            self.wait(hold - self.elapsed)
            self.elapsed = hold
        if leftover:
            self.t_play(FadeOut(*leftover), t=CLEAR_SECONDS)
        self.cap = None
        self.plan[self.sid] = self.elapsed

    def t_play(self, *anims, t=2.0):
        self.play(*anims, run_time=t)
        self.elapsed += t

    def t_wait(self, t):
        self.wait(t)
        self.elapsed += t

    def say(self, text, t=2.0):
        """Show the scene's caption in t seconds total. The one on screen fades out BEFORE the
        new one fades in (split evenly): two captions on top of each other during a swap are
        exactly the overlap the #459 guard FAILs, so a swap must never hold both at once."""
        new = caption(text)
        half = t / 2
        if self.cap is not None:
            self.t_play(FadeOut(self.cap), t=half)
        self.cap = new
        self.t_play(FadeIn(new), t=half)

    def tear_down(self):
        path = os.environ.get("DRUMSPIKE_EXPLAINER_PLAN")
        if path:
            with open(path, "w", encoding="utf-8") as f:
                json.dump({"scenes": self.plan, "config_frame_rate": config.frame_rate}, f, indent=2)
