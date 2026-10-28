"""#246 explainer 4, "Finding the first sample" (chapter 4). Script: docs/product/scripts/explainer-04-first-sample.md.

The hits are synthetic (physics.py): the demo session (#242) is not on main, so the real-hit
version waits for it. The walk back is the script's description of refine.rs version 3, drawn by
physics.first_motion; the Rust rule decides the real result. Render through build.py.
"""
from explainer_common import (
    COBALT, DIM, DOWN, LEFT, ORANGE, RIGHT, TEXT, UP, Create, Dot, DashedLine, Explainer, FadeIn,
    FONT, Line, Rectangle, Text, VGroup, ValueTracker, always_redraw, wave,
)
from physics import first_motion, snare_hit

import numpy as np

PLOT_W = 11.0
SEG = 240  # 5 ms at 48 kHz: the zoomed view draws every sample
LEAD = 240  # 5 ms of low noise before the attack
COARSE = 120  # the detector's rough time, 2.5 ms before the attack
NOISE = 0.01


def hit_signal(amp=1.0):
    """Low deterministic noise, then a synthetic snare hit of amplitude `amp`, at 48 kHz."""
    lead = [NOISE * np.sin(i * 0.7) for i in range(LEAD)]
    return lead + [amp * x for x in snare_hit(2400)]


def envelope(values, win=24):
    """Smoothed |x|: a moving average over `win` samples."""
    out = []
    acc = 0.0
    for i, x in enumerate(values):
        acc += abs(x)
        if i >= win:
            acc -= abs(values[i - win])
        out.append(acc / win)
    return out


class Explainer04(Explainer):
    script_ids = (
        "s01-recap", "s02-coarse", "s03-envelope", "s04-peak-threshold",
        "s05-walk-back", "s06-soft-hit", "s07-safety", "s08-result",
    )

    def s01_recap(self):
        comb = VGroup(*[
            Line(LEFT * 1.5 + RIGHT * 3.0 * i / 9, LEFT * 1.5 + RIGHT * 3.0 * i / 9 + UP * 0.6, color=ORANGE)
            for i in range(10)
        ]).shift(UP * 2.4 + RIGHT * 4.2)
        pic = wave(hit_signal()[::4], -PLOT_W / 2, PLOT_W / 2, -0.4, 1.6, COBALT)
        title = Text("Explainer 1: a trigger must fire on the first sample", font=FONT, font_size=30, color=TEXT)
        self.t_play(Create(pic), FadeIn(comb), t=3)
        self.t_play(FadeIn(title.to_edge(UP, buff=0.7)), t=2)
        self.say("Now we take one snare hit and find its first sample.", t=2)
        self.t_wait(5)

    def s02_coarse(self):
        values = hit_signal()[::4]
        pic = wave(values, -PLOT_W / 2, PLOT_W / 2, 0.0, 1.6, COBALT)
        x_coarse = -PLOT_W / 2 + PLOT_W * COARSE / len(hit_signal())
        band = Rectangle(width=0.9, height=4.0, fill_color=ORANGE, fill_opacity=0.3, stroke_width=0).move_to(
            [x_coarse, 0.0, 0.0])
        self.t_play(Create(pic), t=3)
        self.t_play(FadeIn(band), t=2)
        self.say("The detector's time is a few milliseconds early or late. Far too much for a trigger.", t=2)
        self.t_wait(6)

    def s03_envelope(self):
        raw = hit_signal()
        env = envelope(raw)[::4]
        pic = wave(raw[::4], -PLOT_W / 2, PLOT_W / 2, 0.0, 1.6, DIM, width=1.5)
        top = wave(env, -PLOT_W / 2, PLOT_W / 2, 0.0, 1.8, COBALT, width=3)
        steepest = max(range(1, len(env)), key=lambda i: env[i] - env[i - 1])
        x_steep = -PLOT_W / 2 + PLOT_W * steepest / (len(env) - 1)
        glow = Dot([x_steep, 1.8 * env[steepest], 0], radius=0.12, color=ORANGE)
        self.t_play(Create(pic), t=2)
        self.t_play(Create(top), t=2.5)
        self.t_play(FadeIn(glow), t=1.5)
        self.say("The envelope rises most steeply at the attack. That is the anchor of the hit.", t=2)
        self.t_wait(6)

    def s04_peak_threshold(self):
        seg = hit_signal()[COARSE:COARSE + SEG]
        peak = max(range(SEG), key=lambda i: abs(seg[i]))
        peak_val = abs(seg[peak])
        norm = [v / peak_val for v in seg]  # the attack's peak is 1; the picture is 3.5 units high
        thr = 10 ** (-33 / 20)  # 33 dB under the peak
        floor_line = NOISE * 10 ** (6 / 20) / peak_val  # 6 dB over the noise, in the same units
        x_of = lambda i: -PLOT_W / 2 + PLOT_W * i / (SEG - 1)  # noqa: E731
        pic = wave(norm, -PLOT_W / 2, PLOT_W / 2, 0.0, 3.5, COBALT)
        peak_dot = Dot([x_of(peak), 3.5 * norm[peak], 0], radius=0.1, color=ORANGE)
        thr_line = DashedLine(LEFT * 5.5, RIGHT * 5.5, color=ORANGE).shift(UP * 3.5 * thr)
        thr_neg = DashedLine(LEFT * 5.5, RIGHT * 5.5, color=ORANGE).shift(DOWN * 3.5 * thr)
        noise_w = PLOT_W * (LEAD - COARSE) / (SEG - 1)  # the samples before the hit, in this window
        noise_band = Rectangle(width=noise_w, height=2 * 3.5 * floor_line, fill_color=DIM,
                               fill_opacity=0.25, stroke_width=0).move_to([x_of((LEAD - COARSE) / 2), 0, 0])
        self.t_play(Create(pic), FadeIn(peak_dot), t=3)
        self.t_play(Create(thr_line), Create(thr_neg), FadeIn(noise_band), t=2.5)
        self.say("Threshold: 33 dB under the attack's peak, and 6 dB over the noise before the hit.", t=2)
        self.t_wait(6)

    def s05_walk_back(self):
        full = hit_signal()
        seg = full[COARSE:COARSE + SEG]
        start = first_motion(full, COARSE) - COARSE
        peak = max(range(SEG), key=lambda i: abs(seg[i]))
        pos = ValueTracker(float(peak))
        pic = wave([v / abs(seg[peak]) for v in seg], -PLOT_W / 2, PLOT_W / 2, 0.0, 3.5, COBALT)

        def x_of(i):
            return -PLOT_W / 2 + PLOT_W * i / (SEG - 1)

        marker = always_redraw(lambda: Line(
            [x_of(pos.get_value()), -3.0, 0], [x_of(pos.get_value()), 3.0, 0], color=ORANGE, stroke_width=3))
        self.t_play(Create(pic), FadeIn(marker), t=2)
        self.say("The walk goes back sample by sample, while the signal stays over the threshold.", t=2)
        self.t_play(pos.animate.set_value(float(max(0, start))), t=7)
        self.say("It stops after half a millisecond under the threshold. The hit goes on this sample.", t=2)
        self.t_wait(4)

    def s06_soft_hit(self):
        loud = hit_signal(1.0)[COARSE:COARSE + SEG]
        soft = hit_signal(0.3)[COARSE:COARSE + SEG]
        top = wave(loud, -PLOT_W / 2, PLOT_W / 2, 1.6, 1.2, COBALT)
        bottom = wave(soft, -PLOT_W / 2, PLOT_W / 2, -1.6, 1.2, DIM)  # raw amplitudes: the soft hit is lower
        self.t_play(Create(top), t=2.5)
        self.t_play(Create(bottom), t=2.5)
        self.say("A ghost note has a lower peak and a lower threshold. Both start on the first motion.", t=2)
        self.t_wait(6)

    def s07_safety(self):
        limit = Text("Never more than 10 ms back.", font=FONT, font_size=34, color=TEXT).shift(UP * 2)
        limit2 = Text("Never past the middle to the previous hit.", font=FONT, font_size=34, color=TEXT).shift(UP * 1)
        self.t_play(FadeIn(limit), t=2)
        self.t_play(FadeIn(limit2), t=2)
        kick = np.concatenate([np.zeros(240), np.array([np.exp(-((i - 500) / 80.0) ** 2) for i in range(1200)])])
        pic = wave(kick[::4] * 1.0, -PLOT_W / 2, PLOT_W / 2, -1.0, 1.4, ORANGE)
        self.t_play(Create(pic), t=2.5)
        self.say("A kick peaks late, so its threshold sits 29 dB under the peak, not 33.", t=2)
        self.t_wait(5)

    def s08_result(self):
        track = Line(LEFT * 5.5, RIGHT * 5.5, color=COBALT)
        hits = VGroup(*[Line(UP * 0.5, DOWN * 0.5, color=ORANGE, stroke_width=3).shift(RIGHT * x)
                        for x in np.linspace(-4.8, 4.8, 9)])
        midi = VGroup(*[Line(UP * 1.4, UP * 1.2, color=DIM).shift(RIGHT * x) for x in np.linspace(-4.8, 4.8, 9)])
        self.t_play(Create(track), FadeIn(hits), FadeIn(midi), t=3)
        self.say("Every hit sits on the first sample of its attack. The MIDI and spike files get that point.", t=2)
        self.t_wait(6)
