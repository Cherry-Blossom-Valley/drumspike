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
        """Show the scene's caption; the one on screen fades out as the new one fades in."""
        new = caption(text)
        outs = [FadeOut(self.cap)] if self.cap is not None else []
        self.cap = new
        self.t_play(*outs, FadeIn(new), t=t)

    def tear_down(self):
        path = os.environ.get("DRUMSPIKE_EXPLAINER_PLAN")
        if path:
            with open(path, "w", encoding="utf-8") as f:
                json.dump({"scenes": self.plan, "config_frame_rate": config.frame_rate}, f, indent=2)
