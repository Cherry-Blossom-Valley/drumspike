"""#246 explainer 1, "Why one millisecond matters" (chapter 1). Script: docs/product/scripts/explainer-01-why-one-ms.md.

Synthetic signals from physics.py (not recordings). Render through build.py.
"""
from explainer_common import (
    COBALT, DIM, DOWN, LEFT, ORANGE, RIGHT, TEXT, UP, Create, Dot, DashedLine, Explainer, FadeIn,
    FadeOut, FONT, Line, Text, VGroup, ValueTracker, VMobject, always_redraw, wave,
)
from physics import comb_gain, cm_per_ms, first_motion, notches, samples_per_ms, snare_hit, spectrum

import numpy as np

PLOT_W = 11.0
CURVE_SAMPLES = 2400  # the 50 ms snare hit, drawn at 48 kHz
VIEW_STEP = 4  # every 4th sample for the picture: 12 000 points is too many for Manim
DRAWN_PER_MS = samples_per_ms() / VIEW_STEP  # 12 drawn samples are 1 ms


def snare_values():
    return snare_hit(CURVE_SAMPLES)[::VIEW_STEP]


def late_by_ms(values, ms):
    """The drawn samples late by `ms` (whole drawn samples)."""
    k = round(ms * DRAWN_PER_MS)
    return [0.0] * k + values[:len(values) - k]


class Explainer01(Explainer):
    script_ids = (
        "s01-hook", "s02-one-ms", "s03-sum", "s04-comb",
        "s05-shift", "s06-threshold", "s07-first-motion", "s08-result",
    )

    def s01_hook(self):
        snare = wave(snare_values(), -PLOT_W / 2, PLOT_W / 2, 0.6, 1.6, COBALT)
        # The trigger's sample: a short orange stroke, one millisecond late (48 samples of 48 kHz).
        sample = wave(late_by_ms(snare_values(), 1.0), -PLOT_W / 2, PLOT_W / 2, 0.6, 1.6, ORANGE, width=1.5)
        self.t_play(Create(snare), t=3)
        self.t_play(FadeIn(sample), t=2)
        self.say("A sample plays one millisecond late. You can hear it.", t=1.5)
        self.t_wait(4)

    def s02_one_ms(self):
        dots = VGroup(*[
            Dot(point=LEFT * 5.5 + RIGHT * 11 * i / 47 + DOWN * 0.3, radius=0.06, color=COBALT)
            for i in range(48)
        ])
        label = Text(f"1 ms = {samples_per_ms():.0f} samples at 48 kHz", font=FONT, font_size=34, color=TEXT)
        label.to_edge(UP, buff=0.7)
        self.t_play(FadeIn(label), Create(dots), t=3)
        dist = Text(f"Sound moves about {cm_per_ms():.0f} cm in 1 ms", font=FONT, font_size=30, color=TEXT)
        self.t_play(FadeOut(dots), FadeIn(dist.next_to(label, DOWN, buff=1.0)), t=2.5)
        self.say("So a sample 1 ms late sounds like a second microphone, 34 cm further away.", t=2)
        self.t_wait(5)

    def s03_sum(self):
        a = snare_values()
        b = late_by_ms(a, 1.0)  # the copy, 1 ms late
        total = [(x + y) / 2.0 for x, y in zip(a, b)]
        top = wave(a, -PLOT_W / 2, PLOT_W / 2, 2.2, 0.9, COBALT)
        mid = wave(b, -PLOT_W / 2, PLOT_W / 2, 0.0, 0.9, DIM)
        bottom = wave(total, -PLOT_W / 2, PLOT_W / 2, -2.4, 0.9, ORANGE)
        self.t_play(Create(top), t=2)
        self.t_play(Create(mid), t=2)
        self.t_play(Create(bottom), t=2)
        self.say("Two copies add. Where one is late, some parts add and some cancel.", t=2)
        self.t_wait(6)

    def s04_comb(self):
        freqs = [i * 10.0 for i in range(1, 401)]  # 10 Hz to 4 kHz
        snare = spectrum(snare_values(), freqs)

        def to_xy(f, y):
            return np.array([-PLOT_W / 2 + PLOT_W * f / 4000.0, -1.2 + 3.0 * y, 0.0])

        body = VMobject(color=DIM, stroke_width=1.5)
        body.set_points_as_corners([to_xy(f, 0.5 * s) for f, s in zip(freqs, snare)])
        gain = VMobject(color=ORANGE, stroke_width=2.5)
        gain.set_points_as_corners([to_xy(f, comb_gain(f, 1.0) / 2.0) for f in freqs])
        markers = VGroup(*[Dot(to_xy(f, 0.0), radius=0.07, color=ORANGE) for f in notches(1.0)])
        labels = VGroup(*[
            Text(f"{f:.0f} Hz", font=FONT, font_size=22, color=TEXT).next_to(to_xy(f, 0.0), DOWN, buff=0.25)
            for f in notches(1.0)
        ])
        self.t_play(Create(body), Create(gain), t=3)
        self.t_play(FadeIn(markers), FadeIn(labels), t=2)
        self.say("Notches at 500, 1500, 2500 Hz: a comb. They sit on the snare's body and crack.", t=2)
        self.t_wait(6)

    def s05_shift(self):
        delay = ValueTracker(1.0)
        freqs = [i * 10.0 for i in range(1, 401)]

        def to_xy(f, y):
            return np.array([-PLOT_W / 2 + PLOT_W * f / 4000.0, -1.2 + 3.0 * y, 0.0])

        curve = always_redraw(lambda: VMobject(color=ORANGE, stroke_width=2.5).set_points_as_corners(
            [to_xy(f, comb_gain(f, delay.get_value()) / 2.0) for f in freqs]))
        readout = always_redraw(lambda: Text(
            f"delay {delay.get_value():.1f} ms", font=FONT, font_size=30, color=TEXT).to_edge(UP, buff=0.7))
        self.t_play(Create(curve), FadeIn(readout), t=2)
        self.t_play(delay.animate.set_value(0.1), t=5)
        self.say("A smaller delay moves the first notch up: 5 kHz at 0.1 ms.", t=2)
        self.t_play(delay.animate.set_value(0.0), t=4)
        self.say("At zero delay the copies add cleanly. The trigger must fire on the first sample.", t=2)
        self.t_wait(4)

    def s06_threshold(self):
        n = 400
        xs = [i / (n - 1) for i in range(n)]

        def bump(x, x0, amp, width):
            return amp * np.exp(-((x - x0) / width) ** 2)

        loud = [float(bump(x, 0.3, 1.0, 0.04)) for x in xs]
        soft = [float(bump(x, 0.62, 0.4, 0.09)) for x in xs]
        kick = [float(bump(x, 0.9, 0.7, 0.05)) for x in xs]

        def curve(vals, color):
            return wave([2 * v - 1 for v in vals], -PLOT_W / 2, PLOT_W / 2, 0.0, 2.2, color, width=2.5)

        threshold = DashedLine(LEFT * PLOT_W / 2, RIGHT * PLOT_W / 2, color=TEXT)  # y = 0: half of the curves' height
        self.t_play(FadeIn(threshold), t=1.5)
        self.t_play(Create(curve(loud, COBALT)), t=2.5)
        self.t_play(FadeIn(curve(soft, DIM)), t=2)
        self.t_play(FadeIn(curve(kick, ORANGE)), t=2)
        self.say("A loud hit crosses early, a soft hit late, and kick bleed can cross too.", t=2)
        self.t_wait(5)

    def s07_first_motion(self):
        # 5 ms of silence, then the hit; the detector's rough time sits 2.5 ms before the attack.
        full = [0.0] * 240 + snare_hit(CURVE_SAMPLES)
        values = full[::VIEW_STEP]
        coarse = 120  # full-rate samples
        start = first_motion(full, coarse)
        pos = ValueTracker(float(coarse // VIEW_STEP))
        pic = wave(values, -PLOT_W / 2, PLOT_W / 2, 0.0, 1.6, COBALT)

        def x_of(i):
            return -PLOT_W / 2 + PLOT_W * i / (len(values) - 1)

        marker = always_redraw(lambda: Line(
            [x_of(pos.get_value()), -2.0, 0], [x_of(pos.get_value()), 2.0, 0], color=ORANGE, stroke_width=3))
        self.t_play(Create(pic), FadeIn(marker), t=3)
        self.say("The detector's time is rough. Drum Spike walks back to the first motion.", t=2)
        self.t_play(pos.animate.set_value(float(start // VIEW_STEP)), t=6)
        self.t_wait(3)

    def s08_result(self):
        lanes = VGroup(*[
            Line(LEFT * 5.5 + UP * (1.0 - k * 1.1), RIGHT * 5.5 + UP * (1.0 - k * 1.1), color=DIM)
            for k in range(2)
        ])
        hits = VGroup(*[
            Line(UP * (1.0 - 1.1 * k + 0.35), UP * (1.0 - 1.1 * k - 0.35), color=ORANGE, stroke_width=3)
            .shift(RIGHT * x)
            for k in range(2) for x in (-4.0, -1.5, 1.0, 3.5)
        ])
        self.t_play(Create(lanes), FadeIn(hits), t=3)
        self.say("Each track gets a MIDI lane and a spike track, one hit per stroke, on the first sample.", t=2)
        self.t_wait(6)
