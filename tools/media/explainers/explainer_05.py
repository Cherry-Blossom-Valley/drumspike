"""#402 explainer 5, "The body of the hit" (chapter 5). Script:
docs/product/scripts/explainer-05-audio-spike.md.

Real data: the demo session's kick and snare close mics (tools/media/explainers/physics.py,
DEMO_DIR), the snippet rule of crates/core/src/spike.rs mirrored in audio_spike.py. Render
through build.py. Output stays out of git.

Every hit opens as the whole hit (from just before the onset to the decay, about 85 ms) with
the snippet shaded on it; the counting zooms into the snippet (#402 review).
"""
from explainer_common import (
    COBALT, DIM, DOWN, LEFT, ORANGE, RIGHT, TEXT, UP, Create, Explainer, FadeIn,
    FONT, Line, Rectangle, Text, VGroup, wave,
)
from manim import Brace

import audio_spike as A
import numpy as np
import physics as P

PLOT_W = 11.0
PRE_S = 0.005   # silence before the onset in the whole-hit view
SPAN_S = 0.080  # the hit's body after the onset in the whole-hit view
ZOOM_DECIMATE = 2  # the zoomed scenes draw at 24 kHz
WHOLE_DECIMATE = 6  # the whole-hit scenes at 8 kHz


def pick_hits():
    """(kick samples, kick start, snare samples, snare start) of the demo session: the first
    kick and the first loud (not ghost) snare hit of truth.json. The too-small scene is a
    constructed illustration (the demo hits have no mid-snippet dip inside the band)."""
    import json

    truth = json.loads((P.DEMO_DIR / "truth.json").read_text(encoding="utf-8"))
    kicks = [h["t"] / truth["sr"] for h in truth["hits"] if h["drum"] == "kick"]
    snares = [h["t"] / truth["sr"] for h in truth["hits"]
              if h["drum"] == "snare" and not h["ghost"] and h["v"] >= 0.9]
    kx = P.demo_track("k in")
    sx = P.demo_track("s top")
    k0 = round(kicks[3] * A.RATE)
    s0 = round(snares[0] * A.RATE)
    return kx, k0, sx, s0


KX, K0, SX, S0 = pick_hits()
KSMOOTH = A.low_component(KX)
SSMOOTH = A.low_component(SX)


def _window(x, lo, hi, decimate):
    seg = x[lo:hi]
    peak = max(float(np.max(np.abs(seg))), 1e-9)
    return np.array(seg[::decimate]) / peak


def whole_hit(x, start, decimate=WHOLE_DECIMATE):
    """The whole hit (5 ms before the onset to 80 ms after it), normalised to the hit's peak."""
    lo = max(0, start - round(PRE_S * A.RATE))
    hi = min(len(x), start + round(SPAN_S * A.RATE))
    return _window(x, lo, hi, decimate)


def zoomed(smooth, start, span_s, decimate=ZOOM_DECIMATE):
    """The smoothed tone over `span_s` after the onset, normalised to its own peak."""
    hi = start + round(span_s * A.RATE)
    return _window(smooth, start, hi, decimate)


def x_of_index(idx, n):
    """The x of sample array index `idx` of an n-point plot from -PLOT_W/2 to PLOT_W/2."""
    return -PLOT_W / 2 + PLOT_W * idx / max(1, n - 1)


def shaded_band(n, lo_idx, hi_idx, yc, half):
    """The snippet range as a shaded band over an n-point plot."""
    x0 = x_of_index(lo_idx, n)
    x1 = x_of_index(hi_idx, n)
    return Rectangle(width=max(x1 - x0, 0.05), height=2 * half, fill_color=ORANGE,
                     fill_opacity=0.18, stroke_width=0).move_to([(x0 + x1) / 2, yc, 0])


class Explainer05(Explainer):
    script_ids = (
        "s01-not-a-click", "s02-the-body", "s03-count-the-swings", "s03b-too-small",
        "s04-the-cut", "s05-real-hits", "s06-click-versus-audio",
    )

    def s01_not_a_click(self):
        pic = wave(whole_hit(KX, K0), -PLOT_W / 2, PLOT_W / 2, 0.0, 1.6, COBALT)
        spike = Line([-1.0, -1.2, 0], [-1.0, 1.2, 0], color=ORANGE, stroke_width=4)
        label = Text("a click is one sample", font=FONT, font_size=26, color=DIM).next_to(spike, DOWN)
        self.t_play(Create(pic), t=2.5)
        self.t_play(FadeIn(spike), FadeIn(label), t=1.5)
        self.say("The Audio spike is not a click. It is a short piece of your own track.", t=2)
        self.t_wait(2)

    def s02_the_body(self):
        # the whole hit first: the silence before the onset, the attack, the decay
        raw = whole_hit(KX, K0)
        tone_raw = _window(KSMOOTH, K0 - round(PRE_S * A.RATE),
                           K0 + round(SPAN_S * A.RATE), WHOLE_DECIMATE)
        tone = tone_raw / max(float(np.max(np.abs(tone_raw))), 1e-9) * float(np.max(np.abs(raw)))
        n = len(raw)
        pic = wave(raw, -PLOT_W / 2, PLOT_W / 2, 0.0, 1.6, COBALT)
        self.t_play(Create(pic), t=2)
        tone_pic = wave(tone, -PLOT_W / 2, PLOT_W / 2, 0.0, 1.6, ORANGE, width=3)
        self.t_play(Create(tone_pic), t=2.5)
        end, _ = A.snippet_end(KX, KSMOOTH, K0)
        band = shaded_band(n, (K0 - (K0 - round(PRE_S * A.RATE))) // WHOLE_DECIMATE,
                           (end - (K0 - round(PRE_S * A.RATE))) // WHOLE_DECIMATE, 0.0, 1.6)
        self.t_play(FadeIn(band), t=1.5)
        self.say("The body of the hit is the low tone under the attack, below 800 hertz.", t=2)
        self.t_wait(2.5)

    def s03_count_the_swings(self):
        # zoomed into the counted span; one number per counted swing, on its own half-wave
        span = (A.snippet_end(KX, KSMOOTH, K0)[0] - K0) / A.RATE + 0.003
        low = zoomed(KSMOOTH, K0, span)
        n = len(low)
        pic = wave(low, -PLOT_W / 2, PLOT_W / 2, 0.0, 1.8, ORANGE, width=3)
        self.t_play(Create(pic), t=2)
        lobes, _ = A.counted_lobes(KX, KSMOOTH, K0)
        numbers = VGroup(*[
            Text(str(k + 1), font=FONT, font_size=34, color=TEXT).shift(
                UP * 2.2 + RIGHT * x_of_index(((lobes[k][0] + lobes[k][1]) / 2 - K0)
                                              // ZOOM_DECIMATE, n))
            for k in range(3)])
        self.t_play(FadeIn(numbers[0]), t=1)
        self.t_play(FadeIn(numbers[1:]), t=2)
        self.say("The swings count one, two, three. A swing counts at a quarter of the largest.", t=3)
        self.t_wait(2)

    def s03b_too_small(self):
        # spike.rs third_significant_lobe_end: a significant lobe with the SAME sign after an
        # insignificant dip EXTENDS the current counted swing. Half-waves +1.0 (count 1),
        # -0.15 (dip, not counted), +0.8 (same sign -> still swing 1), -0.6 (count 2),
        # +0.4 (count 3, above the band).
        f = 120.0                       # Hz, about the demo kick's low tone
        amps = [1.0, 0.15, 0.8, 0.6, 0.4]
        half_n = round(A.RATE / (2 * f))
        n = half_n * len(amps)
        t = np.arange(n) / A.RATE
        k = np.minimum((t * 2 * f).astype(int), len(amps) - 1)
        y = np.array(amps)[k] * np.sin(2 * np.pi * f * t)
        pic = wave(y, -PLOT_W / 2, PLOT_W / 2, 0.0, 1.8, ORANGE, width=3)
        self.t_play(Create(pic), t=2)
        band = VGroup(
            Line([-PLOT_W / 2, 0.25 * 1.8, 0], [PLOT_W / 2, 0.25 * 1.8, 0],
                 color=DIM, stroke_width=2),
            Line([-PLOT_W / 2, -0.25 * 1.8, 0], [PLOT_W / 2, -0.25 * 1.8, 0],
                 color=DIM, stroke_width=2))
        mark = Text("a quarter of the largest (P)", font=FONT, font_size=22,
                    color=DIM).next_to(band[0], UP)
        tag = Text("illustration", font=FONT, font_size=22, color=DIM).shift(
            UP * 2.4 + LEFT * 4.6)  # top-left: clear of the '3' above the last hump
        self.t_play(Create(band), FadeIn(mark), FadeIn(tag), t=1.5)

        def cx(half):
            return x_of_index(half * half_n + half_n // 2, n)

        # swing 1 spans both positive humps (the dip does not restart the count)
        brace = Brace(Line([cx(0), 1.95, 0], [cx(2), 1.95, 0], stroke_width=0),
                      UP, buff=0.05)
        one = Text("1", font=FONT, font_size=34, color=TEXT).next_to(brace, UP, buff=0.1)
        self.t_play(FadeIn(brace), FadeIn(one), t=1)
        two = Text("2", font=FONT, font_size=34, color=TEXT).shift(
            DOWN * 2.2 + RIGHT * cx(3))
        self.t_play(FadeIn(two), t=1)
        three = Text("3", font=FONT, font_size=34, color=TEXT).shift(
            UP * 2.2 + RIGHT * cx(4))
        self.t_play(FadeIn(three), t=1.5)
        dip = Text("too small: does not count", font=FONT, font_size=26, color=TEXT).shift(
            DOWN * 1.6 + RIGHT * cx(1) * 0.4)
        self.t_play(FadeIn(dip), t=1.5)
        self.say("A dip under a quarter does not count. The swing after it keeps the "
                 "same count.", t=3.5)

    def s04_the_cut(self):
        # the cut on the whole-hit view: the end crossing of counted lobe 3
        raw = whole_hit(KX, K0)
        n = len(raw)
        pic = wave(raw, -PLOT_W / 2, PLOT_W / 2, 0.0, 1.6, COBALT)
        end, _ = A.snippet_end(KX, KSMOOTH, K0)
        onset0 = K0 - round(PRE_S * A.RATE)
        x_end = x_of_index((end - onset0) // WHOLE_DECIMATE, n)
        cut = Line([x_end, -2.0, 0], [x_end, 2.0, 0], color=ORANGE, stroke_width=4)
        band = shaded_band(n, (K0 - onset0) // WHOLE_DECIMATE,
                           (end - onset0) // WHOLE_DECIMATE, 0.0, 1.6)
        self.t_play(Create(pic), t=2)
        self.t_play(FadeIn(band), t=1)
        self.t_play(Create(cut), t=1.5)
        len_ms = (end - K0) / A.RATE * 1000.0
        limits = Text("at most 60 ms", font=FONT, font_size=28, color=TEXT).shift(UP * 2.75)
        # above the cut line (top y = 2.0), so the line does not run through the text
        measured = Text(f"this demo kick: {len_ms:.1f} ms; a kick 15 to 25 ms, a snare 5 to 10 ms",
                        font=FONT, font_size=24, color=TEXT).shift(UP * 2.25)
        self.t_play(FadeIn(limits), FadeIn(measured), t=2)
        self.say("The cut sits on a zero crossing, at most 60 milliseconds.", t=2)
        self.t_wait(1.31)

    def s05_real_hits(self):
        kr = whole_hit(KX, K0)
        sr_ = whole_hit(SX, S0)
        kend, _ = A.snippet_end(KX, KSMOOTH, K0)
        send, _ = A.snippet_end(SX, SSMOOTH, S0)
        onset_k = K0 - round(PRE_S * A.RATE)
        onset_s = S0 - round(PRE_S * A.RATE)
        top = wave(kr, -PLOT_W / 2, PLOT_W / 2, 1.5, 1.1, COBALT)
        bottom = wave(sr_, -PLOT_W / 2, PLOT_W / 2, -1.5, 1.1, COBALT)
        kband = shaded_band(len(kr), (K0 - onset_k) // WHOLE_DECIMATE,
                            (kend - onset_k) // WHOLE_DECIMATE, 1.5, 1.1)
        sband = shaded_band(len(sr_), (S0 - onset_s) // WHOLE_DECIMATE,
                            (send - onset_s) // WHOLE_DECIMATE, -1.5, 1.1)
        names = VGroup(
            Text("kick", font=FONT, font_size=26, color=DIM).shift(LEFT * 5.6 + UP * 1.5),
            Text("snare", font=FONT, font_size=26, color=DIM).shift(LEFT * 5.6 + DOWN * 1.5))
        self.t_play(Create(top), Create(bottom), FadeIn(names), t=3)
        self.t_play(FadeIn(kband), FadeIn(sband), t=2)
        self.say("Two real hits: the kick breathes longer, the snare is short and crisp.", t=3)
        self.t_wait(1.79)

    def s06_click_versus_audio(self):
        click = VGroup(*[Line([-3.5, -1.4, 0], [-3.5, 1.4, 0], color=ORANGE, stroke_width=5)])
        click_label = Text("click spike", font=FONT, font_size=26, color=DIM).next_to(click, DOWN)
        body = wave(whole_hit(KX, K0)[:200], -1.0, 4.5, 0.0, 1.4, COBALT)
        body_label = Text("audio spike", font=FONT, font_size=26, color=DIM).next_to(body, DOWN)
        self.t_play(FadeIn(click), FadeIn(click_label), t=2)
        self.t_play(Create(body), FadeIn(body_label), t=2.5)
        self.say("The audio spike triggers on the hit's own attack: the phase lines up.", t=3)
        self.t_wait(2.5)
