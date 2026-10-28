"""#336 explainer 10, "Is it a hit or is it bleed?" (chapter 10). Script: docs/product/scripts/explainer-10-bleed.md.

The picture uses the real demo session (tools/media/demo_session.py): the WAV files and
truth.json are read from ~/.cache/drumspike/media/demo/ (generated on demand with the
default seed). The kit check of bar 17 carries the isolated bleed, the flam of bar 16 and
the tom fill of bar 23 carry the pair and split scenes. The rule numbers are the script's
"Facts used"; the delays and levels in the labels are measured from the session at render
time. Render through build.py.
"""
import json
import os
import sys
import wave as wavefile
from pathlib import Path

import numpy as np

from explainer_common import (
    CAPTION_SIZE, COBALT, DIM, DOWN, LEFT, ORANGE, RIGHT, TEXT, UP, Arrow, Create, Dot,
    DashedLine, Explainer, FadeIn, FadeOut, FONT, Line, Rectangle, Text, VGroup, wave,
)

PLOT_W = 11.0
SR = 48000  # the demo session's rate

# The session's close mics and the drum each one owns (the crash has no close mic; the ride,
# the overheads and the room are not close mics; "new take" is a spare kick mic).
CLOSE_MIC_OWN = {"k in": "kick", "k out": "kick", "s top": "snare", "s bot": "snare",
                 "t1": "t1", "t2": "t2", "t3": "t3", "hh": "hat"}

_TRUTH = None


def demo_dir():
    """The demo session's folder, generated on demand (audio never goes into git)."""
    d = Path(os.environ.get("DRUMSPIKE_DEMO_DIR", str(Path.home() / ".cache" / "drumspike" / "media" / "demo")))
    if not (d / "truth.json").exists():
        parent = str(Path(__file__).resolve().parent.parent)
        if parent not in sys.path:
            sys.path.insert(0, parent)
        import demo_session
        demo_session.generate(d)
    return d


def truth():
    global _TRUTH
    if _TRUTH is None:
        _TRUTH = json.loads((demo_dir() / "truth.json").read_text(encoding="utf-8"))
    return _TRUTH


def read_seg(mic, t0, n):
    """n samples of the demo session's `mic` WAV from sample t0, as float64 in -1..1."""
    with wavefile.open(str(demo_dir() / f"{mic}.wav")) as w:
        assert (w.getnchannels(), w.getsampwidth(), w.getframerate()) == (1, 3, SR), mic
        w.setpos(max(0, t0))
        raw = w.readframes(n)
    q = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3).astype(np.int32)
    q = q[:, 0] | (q[:, 1] << 8) | (q[:, 2] << 16)
    return np.where(q & 0x800000, q - (1 << 24), q).astype(np.float64) / 8388607.0


def kit_hits():
    """The kit check of bar 17: one hit per drum, 0.625 s apart, so the bleed is measurable."""
    t = truth()
    spb, bar1 = t["samples_per_beat"], t["bar1_offset_samples"]
    sel = [h for h in t["hits"] if bar1 + 16 * 4 * spb <= h["t"] < bar1 + 17 * 4 * spb]
    return {h["drum"]: h["t"] for h in sel}


def flagged_hits(flag):
    return sorted([h for h in truth()["hits"] if h[flag]], key=lambda h: h["t"])


def first_cross(x, thr):
    idx = np.nonzero(np.abs(x) >= thr)[0]
    return int(idx[0]) if len(idx) else -1


def frac(x, i):
    """The drawn x fraction of raw sample index i (the drawn points spread over the same span)."""
    return i / max(1, len(x) - 1)


def band_level(mic, t0, lo=50.0, hi=400.0):
    """The mic's mean level over 50 to 400 Hz in the [-0.5, +8) ms window at the hit (Goertzel)."""
    x = read_seg(mic, t0 + int(-0.5 / 1000 * SR), int(8.5 / 1000 * SR))
    mags = []
    for f in range(int(lo), int(hi) + 1, 10):
        w = 2 * np.pi * f / SR
        c = 2 * np.cos(w)
        s1 = s2 = 0.0
        for v in x:
            s0 = v + c * s1 - s2
            s2 = s1
            s1 = s0
        mags.append(np.hypot(s1, s2 - s1 * np.cos(w)))
    return float(np.mean(mags)) / np.sqrt(len(x))


def envelope(x, win=24):
    """Smoothed |x|: a moving average over `win` samples (explainer 4's envelope)."""
    return np.convolve(np.abs(x), np.ones(win) / win, mode="same")


def lane_wave(mic, t0, ms, yc, half=0.55, color=COBALT, step=4):
    """The mic's real waveform over `ms` at t0, normalized to its own peak, drawn as a wave."""
    x = read_seg(mic, t0, int(ms / 1000 * SR))[::step]
    pk = float(np.max(np.abs(x))) or 1.0
    return wave([float(v) / pk for v in x], -PLOT_W / 2, PLOT_W / 2, yc, half, color)


def lane_line(yc):
    return Line([-PLOT_W / 2, yc, 0], [PLOT_W / 2, yc, 0], color=DIM)


def lane_label(name, yc):
    # review 6396207826 #5: the names sit left of the lane start, off the hit lines
    return Text(name, font=FONT, font_size=22, color=TEXT).move_to([-6.3, yc, 0])


def lane_group(mic, t0, ms, yc, half=0.55, color=COBALT):
    return VGroup(lane_line(yc), lane_wave(mic, t0, ms, yc, half, color), lane_label(mic, yc))


def stalks(times_s, t_lo, t_hi, x0, x1, yc, color, half=0.3, width=3):
    """Vertical hit stalks on one lane: times in seconds, mapped from t_lo..t_hi onto x0..x1."""
    span = max(t_hi - t_lo, 1e-9)

    def x_of(ts):
        return x0 + (x1 - x0) * (ts - t_lo) / span

    return VGroup(*[Line([x_of(ts), yc + half, 0], [x_of(ts), yc - half, 0], color=color,
                          stroke_width=width) for ts in times_s])


def overview():
    """The whole session on the 8 close-mic lanes: the true strokes as orange stalks, and the
    kit-check bleed copies on the tom mics grey (the rules take them out of the output)."""
    t = truth()
    dur = t["duration_samples"] / SR
    names = list(CLOSE_MIC_OWN)
    ys = {m: 2.2 - 0.62 * i for i, m in enumerate(names)}
    lines = VGroup(*[lane_line(ys[m]) for m in names])
    labels = VGroup(*[Text(m, font=FONT, font_size=18, color=TEXT).move_to([-6.3, ys[m], 0])
                      for m in names])
    orange = VGroup()
    for m in names:
        times = [h["t"] / SR for h in t["hits"] if h["drum"] == CLOSE_MIC_OWN[m]]
        orange.add(*stalks(times, 0.0, dur, -PLOT_W / 2, PLOT_W / 2, ys[m], ORANGE, half=0.2, width=2))
    grey = VGroup()
    kit = kit_hits()
    for m in ("t1", "t2", "t3"):
        for drum, tk in kit.items():
            if CLOSE_MIC_OWN[m] == drum:
                continue
            grey.add(*stalks([(tk + int(0.0025 * SR)) / SR], 0.0, dur, -PLOT_W / 2, PLOT_W / 2,
                             ys[m], DIM, half=0.2, width=2))
    return lines, labels, orange, grey, len(grey)


class Explainer10(Explainer):
    script_ids = (
        "s01-bleed", "s02-session", "s03-tom-bleed", "s04-cross",
        "s05-pairs", "s06-ghost", "s07-flam", "s08-control", "s09-result",
    )

    def s01_bleed(self):
        """Four close mics stacked; one kit-check snare hit: copies on the tom mics, later and quieter."""
        kit = kit_hits()
        t0 = kit["snare"] - int(0.01 * SR)   # 10 ms of lead-in before the attack
        ys = {"k in": 2.4, "s top": 1.0, "t1": -0.6, "t2": -2.2}
        pic = VGroup()
        for m, y in ys.items():
            pic.add(lane_group(m, t0, 70, y))
        self.t_play(Create(pic), t=3.0)
        self.say("Every close mic hears more than its own drum: the tom mics hear the snare, "
                 "later and quieter.", t=2.0)
        # the snare mic peaks; the copies on the tom mics start a measured 2.5 ms later
        ref = read_seg("s top", t0, int(0.07 * SR))
        thr = 0.05 * float(np.max(np.abs(ref)))
        for m in ("t1", "t2"):
            x = read_seg(m, t0, int(0.07 * SR))
            delay = (first_cross(x, thr) - first_cross(ref, thr)) / SR * 1000.0
            dot = Dot([-PLOT_W / 2 + PLOT_W * frac(x, first_cross(x, thr)), ys[m], 0],
                      radius=0.09, color=ORANGE)
            # review 6396207826 #4: the tag rides above the lane, off the waveforms
            tag = Text(f"+{delay:.1f} ms, quieter", font=FONT, font_size=22, color=ORANGE)
            tag.move_to([dot.get_center()[0], ys[m] + 0.8, 0])
            self.t_play(FadeIn(dot), FadeIn(tag), t=2.5)
        self.say("A trigger on the tom mic must not fire on the snare.", t=2.0)

    def s02_session(self):
        """The whole session analyzed; the status line says what the rules did."""
        lines, labels, orange, grey, n_grey = overview()
        self.t_play(Create(lines), FadeIn(labels), Create(orange), t=3.5)
        self.t_play(FadeIn(grey), t=2.0)
        status = Text(f"tom bleed: {n_grey} hits out of the output", font=FONT, font_size=26,
                      color=TEXT).to_edge(UP, buff=1.0)  # review: below would touch the lowest lane
        self.t_play(FadeIn(status), t=2.5)
        self.say("The rules run over the whole session, after the per-track analysis.", t=2.0)
        self.say("Every close mic is a source for every other mic, so load every close mic you have.", t=2.0)
        self.say("A hit that a rule marks keeps its place, shows grey, and stays out of the exports.", t=2.0)

    def s03_tom_bleed(self):
        """One hit on the t2 mic (the kick's copy): its tom-band level is explained by the sources."""
        kit = kit_hits()
        tk = kit["kick"]
        title = Text("The low band where toms live: 50 to 400 Hz, the window [-0.5, +8) ms at the hit",
                     font=FONT, font_size=24, color=TEXT).to_edge(UP, buff=0.7)
        stalk = Line([-4.8, 2.9, 0], [-4.8, 2.3, 0], color=ORANGE, stroke_width=4)  # review #3: clear of the arrows' boxes
        tag = Text("a hit on the t2 mic", font=FONT, font_size=24, color=TEXT).next_to(stalk, RIGHT, buff=0.3)
        self.t_play(FadeIn(title), Create(stalk), FadeIn(tag), t=3.0)
        mics = list(CLOSE_MIC_OWN)
        levels = [band_level(m, tk) for m in mics]
        top = max(levels)
        bars = VGroup()
        names = VGroup()
        for i, (m, lv) in enumerate(zip(mics, levels)):
            h = max(3.2 * lv / top, 0.02)
            x = -PLOT_W / 2 + PLOT_W * (i + 0.5) / len(mics)
            color = COBALT if m == "t2" else DIM
            bars.add(Rectangle(width=0.55, height=h, color=color, stroke_width=2,
                               fill_color=color, fill_opacity=0.5).move_to([x, -1.6 + h / 2, 0]))
            names.add(Text(m, font=FONT, font_size=18, color=TEXT).next_to(bars[-1], DOWN, buff=0.15))
        base = Line([-PLOT_W / 2, -1.6, 0], [PLOT_W / 2, -1.6, 0], color=TEXT)
        self.t_play(Create(bars), Create(base), FadeIn(names), t=3.0)
        self.say("For every tom hit, Drum Spike looks at the low band where toms live, 50 to 400 "
                 "hertz, in a small window around the hit.", t=2.0)
        # the transfer gains come from the strong hits; the kick and snare close mics are sources
        src = ["k in", "k out", "s top", "s bot"]
        xt = -PLOT_W / 2 + PLOT_W * (mics.index("t2") + 0.5) / len(mics)
        arrows = VGroup(*[
            Arrow([-PLOT_W / 2 + PLOT_W * (mics.index(m) + 0.5) / len(mics),
                   -1.6 + max(3.2 * levels[mics.index(m)] / top, 0.02), 0],
                  [xt - 0.4, -1.0, 0], color=ORANGE, stroke_width=2,
                  max_tip_length_to_length_ratio=0.2)
            for m in src])
        self.t_play(Create(arrows), t=3.0)
        self.say("It knows how much of each drum normally arrives at each mic, calibrated from the "
                 "strong hits of the session.", t=2.0)
        explained = Text("the sum explains the level: bleed", font=FONT, font_size=24, color=DIM)
        explained.next_to(bars, UP, buff=0.4)  # review #2: above the bars, off the arrows
        note = Text("Two mics of one drum never explain each other; the kick and snare close mics "
                    "are sources too.", font=FONT, font_size=22, color=TEXT).to_edge(DOWN, buff=1.2)
        self.t_play(FadeIn(explained), FadeIn(note), t=2.0)
        grey_stalk = stalk.copy().set_color(DIM)
        self.t_play(FadeOut(stalk), FadeIn(grey_stalk), t=2.0)
        self.say("If another drum explains the level on this mic, the hit is bleed.", t=2.0)

    def s04_cross(self):
        """The kit snare's copy on the t1 mic: it starts later and is quieter in the snare mic's band."""
        kit = kit_hits()
        t0 = kit["snare"] - int(0.005 * SR)
        ys = {"s top": 1.4, "t1": -1.4}
        pic = VGroup()
        for m, y in ys.items():
            pic.add(lane_group(m, t0, 40, y))
        self.t_play(Create(pic), t=3.0)
        ref = read_seg("s top", t0, int(0.04 * SR))
        thr = 0.05 * float(np.max(np.abs(ref)))
        i_ref = first_cross(ref, thr)
        marks = VGroup()
        fracs = {}
        for m, y in ys.items():
            x = read_seg(m, t0, int(0.04 * SR))
            f = frac(x, first_cross(x, thr))
            fracs[m] = f
            marks.add(Line([-PLOT_W / 2 + PLOT_W * f, y - 0.8, 0],
                           [-PLOT_W / 2 + PLOT_W * f, y + 0.8, 0], color=ORANGE, stroke_width=3))
        self.t_play(Create(marks), t=2.0)
        delay = (fracs["t1"] - fracs["s top"]) * 40.0
        tag = Text(f"the t1 mic starts {delay:.1f} ms later", font=FONT, font_size=24, color=TEXT)
        tag.to_edge(UP, buff=0.7)
        self.t_play(FadeIn(tag), t=2.0)
        # the snare mic is louder in its own band
        ls = band_level("s top", kit["snare"], 150.0, 9000.0)
        lt = band_level("t1", kit["snare"], 150.0, 9000.0)
        h_t = max(2.2 * lt / ls, 0.03)
        bar_s = Rectangle(width=0.5, height=2.2, color=COBALT, stroke_width=2, fill_color=COBALT,
                          fill_opacity=0.5).move_to([3.4, 0.0, 0])
        bar_t = Rectangle(width=0.5, height=h_t, color=DIM, stroke_width=2, fill_color=DIM,
                          fill_opacity=0.5).move_to([4.4, -1.1 + h_t / 2, 0])
        # review #3: the labels sit beside the bars, in the free band between the two lanes
        labels = VGroup(Text("snare band", font=FONT, font_size=20, color=TEXT).next_to(bar_s, RIGHT, buff=0.25),
                        Text("quieter", font=FONT, font_size=20, color=DIM).next_to(bar_t, RIGHT, buff=0.25).set_y(-0.45))
        self.t_play(Create(bar_s), Create(bar_t), FadeIn(labels), t=2.5)
        self.say("A real hit is loudest and earliest on its own mic, so the cross-track rule checks "
                 "the times of arrival.", t=2.0)
        self.say("A tom hit that starts later than a kick, snare or hi-hat hit within 10 ms of it, "
                 "and is quieter in that mic's own band, is bleed.", t=2.0)
        self.t_wait(2.0)

    def s05_pairs(self):
        """The tom fill's first stroke and its copy on the next tom's mic: the quieter one turns grey."""
        fill = flagged_hits("tom_fill")
        t0 = fill[0]["t"] - int(0.004 * SR)
        ys = {"t1": 1.2, "t2": -1.2}
        pic = VGroup()
        for m, y in ys.items():
            pic.add(lane_group(m, t0, 30, y))
        self.t_play(Create(pic), t=3.0)
        # the pair: the t1 stroke and its copy on the t2 mic, a measured gap and dB apart
        ref = read_seg("t1", t0, int(0.03 * SR))
        thr = 0.05 * float(np.max(np.abs(ref)))
        x2 = read_seg("t2", t0, int(0.03 * SR))
        gap = (first_cross(x2, thr) - first_cross(ref, thr)) / SR * 1000.0
        l1 = band_level("t1", fill[0]["t"])
        l2 = band_level("t2", fill[0]["t"] + int(gap / 1000 * SR))
        under = 20.0 * np.log10(l1 / max(l2, 1e-9))
        tag = Text(f"two hits on different tom drums, {gap:.1f} ms apart: the one "
                   f"{under:.0f} dB under turns grey", font=FONT, font_size=22, color=TEXT)
        tag.to_edge(UP, buff=0.7)
        self.t_play(FadeIn(tag), t=2.5)
        dot = Dot([-PLOT_W / 2 + PLOT_W * frac(x2, first_cross(x2, thr)), ys["t2"], 0],
                  radius=0.1, color=DIM)
        self.t_play(FadeIn(dot), t=2.0)
        self.say("Of two hits on different tom drums within 12 ms, the one that plays 6 dB or more "
                 "quieter is bleed.", t=2.0)
        # the next fill stroke has a clear attack of its own and stays
        keep_at = -PLOT_W / 2 + PLOT_W * (fill[1]["t"] - t0) / (0.03 * SR)
        keep = Text("a clear attack of its own stays", font=FONT, font_size=22, color=ORANGE)
        keep.next_to([keep_at, ys["t1"], 0], UP, buff=0.7)  # review: above the lane's wave
        self.t_play(FadeIn(keep), t=2.0)
        self.say("A hit whose own mic shows a clear attack rise stays.", t=2.0)

    def s06_ghost(self):
        """A soft hit whose shape matches the snare template survives; a bump next to a tom hit is dropped."""
        ghosts = flagged_hits("ghost")
        t0 = ghosts[0]["t"] - int(0.01 * SR)
        g = lane_group("s top", t0, 60, 1.6)
        keep = Text("shape matches the snare template (cosine 0.92 or more): stays",
                    font=FONT, font_size=22, color=TEXT).to_edge(UP, buff=0.7)
        self.t_play(Create(g), FadeIn(keep), t=3.0)
        self.say("A quiet snare hit stays when its spectral shape matches the snare template, and "
                 "no kick or tom hit lies within 30 ms of it.", t=2.0)
        # the tom fill heard on the snare mic: a soft bump next to a tom hit, within 30 ms
        fill = flagged_hits("tom_fill")
        w0 = fill[0]["t"] - int(0.01 * SR)
        f = lane_group("s top", w0, 40, -1.6)
        self.t_play(Create(f), t=2.5)
        x = read_seg("s top", w0, int(0.04 * SR))
        thr = 0.05 * float(np.max(np.abs(read_seg("t1", w0, int(0.04 * SR)))))
        dot = Dot([-PLOT_W / 2 + PLOT_W * frac(x, first_cross(x, thr)), -1.6, 0],
                  radius=0.1, color=DIM)
        drop = Text("within 30 ms of the tom hit: dropped", font=FONT, font_size=22, color=DIM)
        drop.next_to([-PLOT_W / 2 + PLOT_W * frac(x, first_cross(x, thr)), -1.6, 0], DOWN, buff=0.65)
        self.t_play(FadeIn(dot), FadeIn(drop), t=2.5)
        self.say("This keeps the ghost notes, and drops the kick and tom bleed on the snare mic.", t=2.0)

    def s07_flam(self):
        """The real flam of bar 16: one kept hit, a second rise 18 ms later, then its own hit."""
        flam = flagged_hits("flam")
        grace, accent = flam[0]["t"], flam[1]["t"]
        gap = (accent - grace) / SR * 1000.0
        t0 = grace - int(0.008 * SR)
        pic = VGroup(lane_line(1.2), lane_wave("s top", t0, 40, 1.2), lane_label("s top", 1.2))
        grid = VGroup(*[DashedLine([-PLOT_W / 2 + PLOT_W * k / 8, 0.5, 0],
                                   [-PLOT_W / 2 + PLOT_W * k / 8, 1.9, 0], color=DIM, stroke_width=1)
                        for k in range(9)])
        one = Text("the detector keeps one hit", font=FONT, font_size=22, color=TEXT).to_edge(UP, buff=0.7)
        self.t_play(Create(pic), Create(grid), FadeIn(one), t=3.0)
        # the attack envelope shows the second rise
        x = read_seg("s top", t0, int(0.04 * SR))
        env = envelope(x)[::4]
        pk = float(np.max(env)) or 1.0
        curve = wave([float(2 * v / pk - 1) for v in env], -PLOT_W / 2, PLOT_W / 2, -0.9, 0.9, ORANGE)
        rise = Text(f"the attack envelope shows a second rise {gap:.0f} ms later",
                    font=FONT, font_size=CAPTION_SIZE, color=TEXT).next_to(curve, DOWN, buff=0.4)
        self.t_play(Create(curve), FadeIn(rise), t=3.0)
        self.say("Drum Spike looks inside that one hit for a second attack, on the snare mic itself: "
                 "12 to 50 ms after the hit, and rising 9 dB or more.", t=2.0)
        # the second stroke becomes its own hit, on its own first sample
        peaks = [int(np.argmax(env[: len(env) // 2])), len(env) // 2 + int(np.argmax(env[len(env) // 2:]))]
        split_dots = VGroup(*[Dot([-PLOT_W / 2 + PLOT_W * p / max(1, len(env) - 1), 1.2, 0],
                                  radius=0.09, color=ORANGE) for p in peaks])
        split = VGroup(Text("it becomes its own hit, on its own first sample", font=FONT, font_size=22,
                            color=TEXT).to_edge(UP, buff=0.7),
                       split_dots,
                       Text("flam", font=FONT, font_size=20, color=ORANGE).next_to(
                           split_dots[-1], UP, buff=0.75))
        # #459: one title at a time (review #1): the old title fades out BEFORE the new one
        # fades in (split evenly) — mid-fade both sit at to_edge(UP) and the guard FAILs it.
        half = 2.5 / 2
        self.t_play(FadeOut(one), t=half)
        self.t_play(FadeIn(split), t=half)
        self.say("If the other mics do not explain it as bleed, the second stroke becomes a hit of "
                 "its own, and a late-peaking mic like a snare bottom is skipped.", t=2.0)
        status = Text("Flams: 1 hit added", font=FONT, font_size=24, color=TEXT).to_edge(DOWN, buff=1.1)
        self.t_play(FadeIn(status), t=2.0)

    def s08_control(self):
        """The grey hit selected: Other Mics Under Hit, then a hand velocity edit brings it back."""
        kit = kit_hits()
        t0 = kit["snare"] - int(0.005 * SR)
        lane = VGroup(lane_line(1.2), lane_wave("t1", t0, 40, 1.2), lane_label("t1", 1.2))
        ref = read_seg("s top", t0, int(0.04 * SR))
        x = read_seg("t1", t0, int(0.04 * SR))
        f = frac(x, first_cross(x, 0.05 * float(np.max(np.abs(ref)))))
        hit = Line([-PLOT_W / 2 + PLOT_W * f, 1.5, 0], [-PLOT_W / 2 + PLOT_W * f, 0.9, 0],
                   color=DIM, stroke_width=4)
        box = Rectangle(width=0.5, height=0.9, color=TEXT, stroke_width=2).move_to(hit)
        self.t_play(Create(lane), FadeIn(hit), Create(box), t=3.0)
        self.say("You stay in control: grey hits are only marked, never deleted.", t=2.0)
        # View > Other Mics Under Hit (O): the other tracks in the window of the selected hit
        panel = Rectangle(width=PLOT_W, height=1.6, color=DIM, stroke_width=1).move_to([0.0, -1.4, 0])
        ptitle = Text("View > Other Mics Under Hit (O)", font=FONT, font_size=20, color=TEXT)
        ptitle.next_to(panel, UP, buff=0.15)
        inner = VGroup()
        for k, m in enumerate(("s top", "t2", "k in")):
            yy = -0.9 - 0.4 * k
            inner.add(lane_line(yy), lane_wave(m, t0, 40, yy, 0.22, color=DIM, step=8))
        hline = DashedLine([-PLOT_W / 2 + PLOT_W * f, -2.2, 0],
                           [-PLOT_W / 2 + PLOT_W * f, -0.7, 0], color=ORANGE)
        self.t_play(Create(panel), FadeIn(ptitle), FadeIn(inner), Create(hline), t=2.5)
        self.say("View Other Mics Under Hit shows you what the other mics heard at that moment.",
                 t=2.0)
        # a hand velocity edit marks the hit edited, and it comes back
        info = Text("info line: velocity 96", font=FONT, font_size=22, color=TEXT).to_edge(DOWN, buff=1.1)
        back = hit.copy().set_color(ORANGE)
        self.t_play(FadeIn(info), t=1.5)
        # #459: one line at a time at the bottom edge — split the fade so both are never visible.
        self.t_play(FadeOut(hit), t=0.75)
        self.t_play(FadeIn(back), t=0.75)
        self.say("One edit brings a hit back, and an edited hit is never marked again.", t=2.0)
        nxt = Text("D: Hits > Next Doubtful Hit", font=FONT, font_size=22, color=TEXT).to_edge(DOWN, buff=1.1)
        self.t_play(FadeOut(info), t=0.75)
        self.t_play(FadeIn(nxt), t=0.75)
        self.say("The review queue takes you to the doubtful hits first.", t=2.0)

    def s09_result(self):
        """All tracks: clean spikes on every mic, bleed in grey, the flam split."""
        lines, labels, orange, grey, _ = overview()
        flam = flagged_hits("flam")
        inset_box = Rectangle(width=1.8, height=0.8, color=TEXT, stroke_width=1).move_to([4.4, 3.0, 0])
        inset = VGroup(
            inset_box,
            Text("the flam, split", font=FONT, font_size=18, color=TEXT).next_to(inset_box, UP, buff=0.15),
            Line([3.9, 3.15, 0], [3.9, 2.85, 0], color=ORANGE, stroke_width=3),
            Line([4.9, 3.15, 0], [4.9, 2.85, 0], color=ORANGE, stroke_width=3),
        )
        self.t_play(Create(lines), FadeIn(labels), Create(orange), FadeIn(grey), FadeIn(inset), t=2.5)
        self.say("The result: one hit per real stroke on every mic, and the bleed left out.", t=2.0)
