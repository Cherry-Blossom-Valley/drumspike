"""#464 explainer 4, "Finding the first sample" (chapter 4). Script: docs/product/scripts/explainer-04-first-sample.md.

The picture uses the real demo session (tools/media/demo_session.py): the snare top mic's WAV and
truth.json are read from ~/.cache/drumspike/media/demo/. The placed sample of every hit on screen is
the answer of the real rule: `parity refine` of the repo's devtools (rule version 10), not a copy of
it. Build it first (`cargo build --release -p drumspike-devtools --bin parity`) or point
DRUMSPIKE_PARITY at a built one. The ripple and the bump of scenes 5 and 6 are added to the real
hit, and the rule places that audio; the numbers of the rule are the script's "Facts used".
Render through build.py.
"""
import json
import os
import struct
import subprocess
import tempfile
from pathlib import Path

import numpy as np

from explainer_common import (
    COBALT, DIM, LEFT, ORANGE, RIGHT, TEXT, UP, Create, Dot, DashedLine, Explainer, FadeIn,
    FONT, Line, Rectangle, Text, VGroup, wave,
)
from explainer_10 import SR, read_seg, truth

ROOT = Path(__file__).resolve().parent.parent.parent.parent
PLOT_W = 11.0
MIC = "s top"
LOUD_T, GHOST_T = 150000, 412500  # a loud snare hit and a ghost note of the demo session (truth.json)
COARSE_EARLY = 96  # the detector's coarse time, 2 ms before the hit
PRE = 4800  # samples before the hit in the excerpt the rule places
RIPPLE_PCT, RIPPLE_WIDTH = 0.08, 6  # the added ripple: 8 percent of the peak, 6 samples wide
BUMP_PCT, BUMP_WIDTH = 0.01, 4  # the added bump: 1 percent of the peak, inside the 2.5 percent band
BAND_PCT = 0.025  # DEPARTURE_VIS_PCT
ZOOM = 10.0  # the lead-in scenes show the lowest 10 percent of the peak


def parity_bin():
    path = Path(os.environ.get("DRUMSPIKE_PARITY", str(ROOT / "target" / "release" / "parity")))
    if not path.exists():
        raise SystemExit(f"FAIL: no parity binary at {path}: cargo build --release -p drumspike-devtools "
                         "--bin parity, or set DRUMSPIKE_PARITY")
    return path


def write_float_wav(path, x, sr=SR):
    """A mono IEEE-float wav (format tag 3), what the repo's wav reader accepts."""
    data = np.asarray(x, dtype="<f4").tobytes()
    with open(path, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVE")
        f.write(b"fmt " + struct.pack("<IHHIIHH", 16, 3, 1, sr, sr * 4, 4, 32))
        f.write(b"data" + struct.pack("<I", len(data)) + data)


def place_many(x, ats, rule=10):
    """The refined sample of the real rule for each coarse sample in `ats`, in the samples of `x`."""
    with tempfile.TemporaryDirectory() as tmp:
        wav, job = Path(tmp) / "a.wav", Path(tmp) / "job.json"
        write_float_wav(wav, x)
        job.write_text(json.dumps({"wav": str(wav), "rule": rule, "hits": [{"t0": a / SR} for a in ats]}))
        done = subprocess.run([str(parity_bin()), "refine", str(job)], capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit(f"FAIL: parity refine: {done.stderr[-400:]}")
    return [round(r["t"] * SR) for r in json.loads(done.stdout)["refined"]]


def place(x, at, rule=10):
    return place_many(x, [at], rule)[0]


def hit_excerpt(t):
    """One real hit: 0.1 s before and after the truth time `t`; its coarse sample and the real rule's answer."""
    x = read_seg(MIC, t - PRE, 2 * PRE)
    at = PRE - COARSE_EARLY
    return x, at, place(x, at)


def add_swing(x, end, sign, share, width, peak):
    """`x` with a half-sine swing of `width` samples ending at sample `end` (exclusive), `share` of the peak."""
    y = x.copy()
    for i in range(width):
        y[end - width + i] += sign * share * peak * np.sin(np.pi * (i + 0.5) / width)
    return y


def peak_of(x, p):
    return float(np.max(np.abs(x[p:p + 240])))


class Lead:
    """One trace of the lead-in: samples a..b of `x` as a curve with a dot per sample, `half` units high
    at height `yc`; the picture shows `zoom` times the signal, clipped at the picture's edge."""

    def __init__(self, x, a, b, peak, yc, half, zoom=ZOOM):
        self.a, self.b, self.yc, self.half, self.zoom, self.peak = a, b, yc, half, zoom, peak
        self.x = x

    def px(self, i):
        return -PLOT_W / 2 + PLOT_W * (i - self.a) / (self.b - self.a)

    def py(self, v):
        return self.yc + self.half * max(-1.0, min(1.0, self.zoom * v / self.peak))

    def curve(self, color=COBALT):
        vals = [max(-1.0, min(1.0, self.zoom * self.x[i] / self.peak)) for i in range(self.a, self.b + 1)]
        return wave(vals, -PLOT_W / 2, PLOT_W / 2, self.yc, self.half, color, width=2.0)

    def dots(self, lo, hi, color=COBALT, radius=0.045):
        return VGroup(*[Dot([self.px(i), self.py(self.x[i]), 0], radius=radius, color=color)
                        for i in range(max(lo, self.a), min(hi, self.b) + 1)])

    def marker(self, i, color=ORANGE, span=None):
        span = self.half if span is None else span
        return Line([self.px(i), self.yc - span, 0], [self.px(i), self.yc + span, 0], color=color, stroke_width=3)

    def band(self, level, share=BAND_PCT, color=DIM):
        """The band of `share` of the peak around `level` over the whole trace."""
        lo, hi = self.py(level - share * self.peak), self.py(level + share * self.peak)
        return Rectangle(width=PLOT_W, height=max(hi - lo, 0.02), fill_color=color, fill_opacity=0.25,
                         stroke_width=0).move_to([0, (lo + hi) / 2, 0])


def line_level(x, foot, n=24):
    """The median of the n samples before `foot` (DEPARTURE_LEVEL_S = 0.5 ms), for the picture's line."""
    return float(np.median(x[foot - n:foot]))


def first_over(x, a, level):
    """The first sample from `a` on whose |x| reaches `level`: the picture's first foot."""
    for i in range(a, len(x)):
        if abs(x[i]) >= level:
            return i
    return a


def snare_track_hits():
    """Snare hits of the demo session's first 8 s of hits that are plain strokes (truth.json), in samples."""
    hits = [h["t"] for h in truth()["hits"]
            if h["drum"] == "snare" and not (h["flam"] or h["roll"] or h["tom_fill"]) and 90000 <= h["t"] < 90000 + 8 * SR]
    return sorted(set(hits))


class Explainer04(Explainer):
    script_ids = (
        "s01-recap", "s02-coarse", "s03-edge", "s04-line", "s05-first-motion", "s06-small-bump",
        "s07-polarity", "s08-soft-hit", "s09-limits", "s10-result",
    )

    def setup(self):
        super().setup()
        self.x, self.at, self.p = hit_excerpt(LOUD_T)
        self.peak = peak_of(self.x, self.p)

    def lead(self, x, yc=0.0, half=1.6, before=44, after=12, zoom=ZOOM, p=None):
        p = self.p if p is None else p
        return Lead(x, p - before, p + after, self.peak, yc, half, zoom)

    def s01_recap(self):
        comb = VGroup(*[
            Line(LEFT * 1.5 + RIGHT * 3.0 * i / 9, LEFT * 1.5 + RIGHT * 3.0 * i / 9 + UP * 0.6, color=ORANGE)
            for i in range(10)
        ]).shift(UP * 1.7 + RIGHT * 4.2)
        v = Lead(self.x, self.p - 30, self.p + 90, self.peak, -0.3, 1.5, zoom=1.0)
        title = Text("Explainer 1: a trigger must fire on the first sample", font=FONT, font_size=30, color=TEXT)
        self.t_play(Create(v.curve()), FadeIn(v.dots(v.a, v.b)), t=3)
        self.t_play(FadeIn(comb), FadeIn(title.to_edge(UP, buff=0.7)), t=2)
        self.say("Now we take one snare hit and find its first sample.", t=2)
        self.t_wait(1)

    def s02_coarse(self):
        a, b = self.at - 24, self.p + 140
        v = Lead(self.x, a, b, self.peak, 0.0, 1.5, zoom=1.0)
        thr = 10 ** (-33 / 20) * self.peak
        foot = first_over(self.x, self.at, thr)
        band = Rectangle(width=0.9, height=4.0, fill_color=ORANGE, fill_opacity=0.3, stroke_width=0).move_to(
            [v.px(self.at), 0.0, 0.0])
        thr_up = DashedLine([-PLOT_W / 2, v.py(thr), 0], [PLOT_W / 2, v.py(thr), 0], color=ORANGE)
        thr_dn = DashedLine([-PLOT_W / 2, v.py(-thr), 0], [PLOT_W / 2, v.py(-thr), 0], color=ORANGE)
        self.t_play(Create(v.curve()), t=2.5)
        self.t_play(FadeIn(band), t=2)
        self.t_play(Create(thr_up), Create(thr_dn), FadeIn(v.marker(foot, span=1.6)), t=2.5)
        self.say("The detector's time is coarse. The first foot is only where the search begins.", t=2)
        self.t_wait(1)

    def s03_edge(self):
        v = self.lead(self.x, before=24, after=36, zoom=1.0)
        w = 5  # EDGE_RISE_S = 0.1 ms: 5 samples at 48 kHz
        need = self.peak * 10 ** (-18 / 20)
        j = next(i for i in range(v.a, v.b - w) if abs(self.x[i + w]) - abs(self.x[i]) >= need)
        edge = VGroup(*[Line([v.px(i), v.py(self.x[i]), 0], [v.px(i + 1), v.py(self.x[i + 1]), 0],
                             color=ORANGE, stroke_width=6) for i in range(j, j + w)])
        label = Text("a rise of 18 dB under the peak within 0.1 ms", font=FONT, font_size=28, color=ORANGE).move_to([0, 2.9, 0])
        label2 = Text("searched in the 5 ms after the foot", font=FONT, font_size=28, color=DIM).move_to([0, 2.4, 0])
        self.t_play(Create(v.curve()), FadeIn(v.dots(v.a, v.b)), t=2.5)
        self.t_play(FadeIn(edge), FadeIn(label), FadeIn(label2), t=2.5)
        self.say("A steep edge: the signal jumps by a clear part of the hit's height within a few samples.", t=2)
        self.t_wait(1)

    def s04_line(self):
        v = self.lead(self.x)
        lvl = line_level(self.x, self.p)
        shade = Rectangle(width=PLOT_W * 24 / (v.b - v.a), height=3.2, fill_color=DIM, fill_opacity=0.15,
                          stroke_width=0).move_to([v.px(self.p - 12), 0, 0])
        mid = DashedLine([-PLOT_W / 2, v.py(lvl), 0], [PLOT_W / 2, v.py(lvl), 0], color=ORANGE)
        zoom = Text("vertical zoom 10x", font=FONT, font_size=26, color=DIM).to_corner(UP + RIGHT, buff=0.6)
        span = Text("the line: median of the 0.5 ms before the foot", font=FONT, font_size=28, color=ORANGE).move_to(
            [-1.5, 2.6, 0])
        self.t_play(Create(v.curve()), FadeIn(v.dots(v.a, v.b)), FadeIn(zoom), t=2.5)
        self.t_play(FadeIn(shade), Create(mid), FadeIn(span), t=2.5)
        self.t_play(FadeIn(v.band(lvl)), t=1.5)
        self.say("The line: its middle value, and a band of 2.5 percent of the peak.", t=2)
        self.t_wait(1)

    def swing_pair(self, share, width):
        """The hit with the swing added once with each sign: [(audio, the real rule's sample)]."""
        out = []
        for sign in (1, -1):
            y = add_swing(self.x, self.p - 2, sign, share, width, self.peak)
            out.append((y, place(y, self.at)))
        return out

    def s05_first_motion(self):
        (pos, p_pos), (neg, p_neg) = self.swing_pair(RIPPLE_PCT, RIPPLE_WIDTH)
        top = self.lead(pos, yc=1.3, half=0.95, before=30, after=12)
        bottom = self.lead(neg, yc=-1.0, half=0.95, before=30, after=12)
        run_top = top.dots(p_pos - 9, p_pos, color=ORANGE, radius=0.07)
        run_bottom = bottom.dots(p_neg - 9, p_neg, color=ORANGE, radius=0.07)
        ripple = Text("ripple of 8 percent of the peak, either sign", font=FONT, font_size=28, color=ORANGE).move_to(
            [-1.5, 2.8, 0])
        run = Text("a run of 10 line samples, 0.2 ms", font=FONT, font_size=28, color=ORANGE).move_to([-3.0, -2.35, 0])
        self.t_play(Create(top.curve()), Create(bottom.curve()), FadeIn(ripple), t=2.5)
        self.t_play(FadeIn(top.dots(top.a, top.b)), FadeIn(bottom.dots(bottom.a, bottom.b)), t=1.5)
        self.t_play(FadeIn(run_top), FadeIn(run_bottom), FadeIn(run), t=2)
        self.t_play(FadeIn(top.marker(p_pos)), FadeIn(bottom.marker(p_neg)), t=2.5)
        self.say("A swing before the edge is the drum already moving.", t=2)
        self.t_wait(1)

    def s06_small_bump(self):
        bump, p_bump = self.swing_pair(BUMP_PCT, BUMP_WIDTH)[0]
        v = self.lead(bump, before=30, after=12)
        lvl = line_level(bump, self.p)
        note = Text("bump of 1 percent of the peak: inside the band, no first motion", font=FONT, font_size=28,
                    color=ORANGE).move_to([0, 2.9, 0])
        rule = Text("a motion must stand out: 2 times the line's wobble over the 2 ms before it", font=FONT,
                    font_size=28, color=DIM).move_to([0, 2.4, 0])
        self.t_play(Create(v.curve()), FadeIn(v.dots(v.a, v.b)), FadeIn(v.band(lvl)), t=3)
        self.t_play(FadeIn(v.marker(p_bump)), FadeIn(note), FadeIn(rule), t=2.5)
        self.say("A bump that stays inside the band does not move the hit.", t=2)
        self.t_wait(1)

    def s07_polarity(self):
        flipped = -self.x
        p_flip = place(flipped, self.at)
        top = self.lead(self.x, yc=1.3, half=0.95, before=30, after=12)
        bottom = self.lead(flipped, yc=-1.0, half=0.95, before=30, after=12)
        guide = DashedLine([top.px(self.p), 2.6, 0], [top.px(p_flip), -2.2, 0], color=DIM)
        self.t_play(Create(top.curve()), Create(bottom.curve()), t=2.5)
        self.t_play(FadeIn(top.dots(top.a, top.b)), FadeIn(bottom.dots(bottom.a, bottom.b)), t=1.5)
        self.t_play(FadeIn(top.marker(self.p)), FadeIn(bottom.marker(p_flip)), Create(guide), t=2.5)
        self.say("The same stroke with flipped polarity lands on the same sample.", t=2)
        self.t_wait(1)

    def s08_soft_hit(self):
        xg, _, pg = hit_excerpt(GHOST_T)
        peak = peak_of(xg, pg)
        v = Lead(xg, pg - 44, pg + 12, peak, 0.0, 1.6, zoom=ZOOM)
        lvl = line_level(xg, pg)
        self.t_play(Create(v.curve()), FadeIn(v.dots(v.a, v.b)), FadeIn(v.band(lvl)), t=3)
        self.t_play(FadeIn(v.marker(pg)), t=2.5)
        self.say("A ghost note: its own peak, its own band, the same first motion.", t=2)
        self.t_wait(1)

    def s09_limits(self):
        notes = ["Back at most 0.8 ms from the edge.", "A swing at least 0.06 ms wide: a flicker is no motion.",
                 "No steep edge: the strict foot, at most 0.06 ms from the first foot.",
                 "96 kHz: the same times, twice the samples."]
        rows = [Text(t, font=FONT, font_size=26, color=TEXT).shift(UP * (2.2 - 1.0 * i)) for i, t in enumerate(notes)]
        for row in rows:
            self.t_play(FadeIn(row), t=2.2)
        self.say("The limits of the rule, in time, not in samples.", t=2)
        self.t_wait(1)

    def s10_result(self):
        hits = snare_track_hits()
        t0 = hits[0] - 4800
        n = 8 * SR + 9600
        x = read_seg(MIC, t0, n)
        placed = place_many(x, [h - t0 - COARSE_EARLY for h in hits])
        env = np.array([np.max(np.abs(x[i:i + 480])) for i in range(0, len(x) - 480, 480)])
        env = env / (env.max() or 1.0)
        track = wave(np.concatenate([env, -env[::-1]]), -5.5, 5.5, 1.2, 0.8, COBALT, width=1.5)
        px = lambda s: -5.5 + 11.0 * s / len(x)  # noqa: E731
        ticks = VGroup(*[Line([px(s), 0.4, 0], [px(s), 2.0, 0], color=ORANGE, stroke_width=3) for s in placed])
        midi = VGroup(*[Line([px(s), -0.6, 0], [px(s), -0.9, 0], color=DIM, stroke_width=6) for s in placed])
        spike = VGroup(*[Line([px(s), -1.6, 0], [px(s), -2.0, 0], color=ORANGE, stroke_width=2) for s in placed])
        self.t_play(Create(track), t=3)
        self.t_play(FadeIn(ticks), FadeIn(midi), FadeIn(spike), t=3)
        self.say("Every hit sits on the last sample of the line before its first motion.", t=2)
        self.t_wait(1)
