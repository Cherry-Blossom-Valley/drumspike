"""#417 explainer 3, "How Drum Spike hears a hit" (chapter 3). Script: docs/product/scripts/explainer-03-detection.md.

Every curve comes from the core on the demo session (tools/media/demo_session.py, the snare
close mic `s top`): the log spectrogram from `parity features` (crates/devtools, the core's
`detect::features`), the spectral flux from `parity onsets --odf-out` (the core's
`detect::onset::superflux`), and the kept hits from the `parity onsets` JSON (the core's
`detect_onsets_with` for the snare role). The outputs are cached under
~/.cache/drumspike/media/explainers/03/ (audio never goes into git). Render through build.py.
"""
import json
import subprocess
import sys
import wave as wavefile
from pathlib import Path

import numpy as np
from manim import rgb_to_color

from explainer_common import (
    COBALT, DIM, DOWN, LEFT, ORANGE, RIGHT, TEXT, UP, Create, Dot, DashedLine, Explainer, FadeIn,
    FadeOut, FONT, Line, Rectangle, Text, VGroup, wave,
)

PLOT_W = 11.0
SR = 48000          # the demo session's rate
FPS = 100.0         # the detector's frame rate (frames at 100 fps)
BANDS = 84          # N_BANDS of the core's log spectrogram
T0, T1 = 41.0, 46.5  # the drawn window: the kit check of bars 17 and 18 (one hit per drum)

ROOT = Path(__file__).resolve().parents[3]
PARITY = ROOT / "target" / "release" / "parity"
DATA = Path.home() / ".cache" / "drumspike" / "media" / "explainers" / "03"


def demo_dir():
    """The demo session's folder, generated on demand (audio never goes into git)."""
    d = DATA.parent.parent / "demo"
    if not (d / "truth.json").exists():
        parent = str(Path(__file__).resolve().parents[2])
        if parent not in sys.path:
            sys.path.insert(0, parent)
        import demo_session
        demo_session.generate(d)
    return d


def _run(cmd):
    done = subprocess.run(cmd, capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit(f"FAIL: {' '.join(cmd)} exited {done.returncode}: {done.stderr[-400:]}")


def core_data(mic="s top", role="snare"):
    """The core's output on one demo track: (hits JSON dict, flux f32 array, spectrogram 2-D array)."""
    tag = mic.replace(" ", "-") + "-" + role
    DATA.mkdir(parents=True, exist_ok=True)
    out = DATA / f"{tag}.json"
    flux_path = DATA / f"{tag}-flux.bin"
    spec_path = DATA / f"{tag}-spec.npy"
    if not (out.exists() and flux_path.exists() and spec_path.exists()):
        wav = str(demo_dir() / f"{mic}.wav")
        _run([str(PARITY), "onsets", wav, role, str(out), "--odf-out", str(flux_path)])
        _run([str(PARITY), "features", wav, "cli", str(spec_path)])
    hits = json.loads(out.read_text(encoding="utf-8"))
    flux = np.fromfile(flux_path, dtype="<f4")
    spec = np.load(spec_path)
    return hits, flux, spec


def truth():
    return json.loads((demo_dir() / "truth.json").read_text(encoding="utf-8"))


def read_seg(t0, n, mic="s top"):
    """n samples of the demo session's `mic` WAV from sample t0, as float64 in -1..1."""
    with wavefile.open(str(demo_dir() / f"{mic}.wav")) as w:
        assert (w.getnchannels(), w.getsampwidth(), w.getframerate()) == (1, 3, SR), mic
        w.setpos(max(0, t0))
        raw = w.readframes(n)
    q = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3).astype(np.int32)
    q = q[:, 0] | (q[:, 1] << 8) | (q[:, 2] << 16)
    return np.where(q & 0x800000, q - (1 << 24), q).astype(np.float64) / 8388607.0


def lane_line(yc):
    return Line([-PLOT_W / 2, yc, 0], [PLOT_W / 2, yc, 0], color=DIM)


def x_of(ts, lo=T0, hi=T1):
    """The drawn x of a time in seconds inside the [lo, hi] window."""
    return -PLOT_W / 2 + PLOT_W * (ts - lo) / (hi - lo)


def flux_curve(flux, lo=T0, hi=T1, yc=-1.4, half=1.1, color=COBALT):
    """The core's flux over the window, as a drawn curve."""
    a, b = int(lo * FPS), int(hi * FPS)
    seg = flux[a:b]
    pk = float(seg.max()) or 1.0
    return wave([float(2 * v / pk - 1) for v in seg], -PLOT_W / 2, PLOT_W / 2, yc, half, color), seg, pk


def spectrogram_image(spec, lo=T0, hi=T1):
    """The core's log spectrogram over the window, drawn as a grid of cells (dark background,
    cobalt energy). ImageMobject does not render in this manim version (the camera's
    Image.fromarray of a float zeros array fails), so the picture is mobjects."""
    a, b = int(lo * FPS), int(hi * FPS)
    block = spec[a:b, :].T  # (bands, frames)
    n_f, n_b = 55, 21       # the drawn grid: every 10th frame, every 4th band
    f_step = block.shape[1] / n_f
    b_step = block.shape[0] / n_b
    lo_v, hi_v = np.percentile(block, 5.0), np.percentile(block, 99.0)
    bg = np.array([0x10, 0x12, 0x18]) / 255.0
    fg = np.array([0x66, 0xB5, 0xFF]) / 255.0
    cw, ch = PLOT_W / n_f, 3.4 / n_b
    cells = VGroup()
    for ib in range(n_b):
        band_rows = block[int(ib * b_step): int((ib + 1) * b_step)]
        for i_f in range(n_f):
            v = float(np.mean(band_rows[:, int(i_f * f_step): int((i_f + 1) * f_step)]))
            t = np.clip((v - lo_v) / max(hi_v - lo_v, 1e-9), 0.0, 1.0) ** 0.5
            rgb = bg + (fg - bg) * t
            color = rgb_to_color(rgb)
            # low bands at the bottom, high bands at the top (the ticks in spec_axes)
            cells.add(Rectangle(width=cw, height=ch, stroke_width=0, fill_color=color,
                                fill_opacity=1.0).move_to([-PLOT_W / 2 + (i_f + 0.5) * cw,
                                                           -0.3 + (ib + 0.5) * ch, 0]))
    return cells


def spec_axes():
    """Axes with units: a few frequency ticks (Hz) and time ticks (s). The grid rows go
    bottom-up over the core's 84 bands (log-spaced, 12 per octave from 20 Hz: the core's
    `features::cli_bins`), so band b sits at 20 * 2**(b/12) Hz."""
    def fy(f):
        return -0.3 + 3.4 * (12 * np.log2(f / 20.0)) / 84.0
    out = VGroup(
        Text("time (s)", font=FONT, font_size=20, color=TEXT).move_to([5.4, -0.95, 0]),
        Text("frequency (Hz)", font=FONT, font_size=20, color=TEXT).rotate(-90 * np.pi / 180)
            .move_to([-6.55, 1.4, 0]),
    )
    for f, lab in ((50, "50"), (100, "100"), (300, "300"), (1000, "1000")):
        out.add(Line([-5.62, fy(f), 0], [-5.48, fy(f), 0], color=DIM, stroke_width=2),
                Text(lab, font=FONT, font_size=16, color=DIM).next_to([-5.62, fy(f), 0], LEFT, buff=0.08))
    for ts in (42.0, 44.0, 46.0):
        out.add(Text(f"{ts:.0f}", font=FONT, font_size=16, color=DIM).move_to([x_of(ts), -0.75, 0]))
    return out


def caption_36(text, buff=0.6):
    """The scene caption: one short gist, 36 px, at most two lines (review of 03)."""
    words, lines, cur = text.split(), [], ""
    for w in words:
        trial = f"{cur} {w}".strip()
        if len(trial) > 52 and cur:
            lines.append(cur)
            cur = w
        else:
            cur = trial
    lines.append(cur)
    t = Text("\n".join(lines[:2]), font=FONT, font_size=36, color=TEXT, line_spacing=0.15)
    if t.width > 11.0:
        t.scale_to_fit_width(11.0)
    return t.to_edge(DOWN, buff=buff)


class Explainer03(Explainer):
    script_ids = (
        "s01-loudness", "s02-spectrogram", "s03-flux", "s04-peaks", "s05-gates", "s06-handoff",
    )

    def say(self, text, t=2.0, **kw):
        """The scene's caption as one short 36 px gist (at most two lines)."""
        new = caption_36(text, **kw)
        outs = [FadeOut(self.cap)] if self.cap is not None else []
        self.cap = new
        self.t_play(*outs, FadeIn(new), t=t)

    def s01_loudness(self):
        """The snare close mic around the kit check; a level meter rides the bleed as high as the hits."""
        x = read_seg(int(T0 * SR), int((T1 - T0) * SR))
        step = 12
        pic = wave([float(v) for v in x[::step]], -PLOT_W / 2, PLOT_W / 2, 1.5, 1.2, COBALT, width=1.5)
        self.t_play(Create(pic), t=3.0)
        env = np.convolve(np.abs(x), np.ones(240) / 240, mode="same")[::step]  # the 5 ms level meter
        meter = wave([float(v) for v in env], -PLOT_W / 2, PLOT_W / 2, -0.9, 1.2, ORANGE)
        label = Text("the level meter: loud is all it knows", font=FONT, font_size=22, color=TEXT)
        label.next_to(meter, DOWN, buff=0.35)
        self.t_play(Create(meter), FadeIn(label), t=3.0)
        # each event gets its own label over the waveform at its own x (review point 2):
        # the kick's copy, the snare's hit, and the snare's decay as its own label
        t = truth()
        spb, bar1 = t["samples_per_beat"], t["bar1_offset_samples"]
        kit = {h["drum"]: h["t"] / SR for h in t["hits"] if bar1 + 16 * 4 * spb <= h["t"] < bar1 + 17 * 4 * spb}
        marks = VGroup()
        for name, ts in (("kick bleed", kit["kick"]), ("snare hit", kit["snare"]),
                         ("the decay", kit["snare"] + 0.75)):
            marks.add(Line([x_of(ts), 2.72, 0], [x_of(ts), 2.95, 0], color=DIM, stroke_width=2),
                      Text(name, font=FONT, font_size=20, color=TEXT)
                          .move_to([x_of(ts), 3.2, 0]))
        self.t_play(FadeIn(marks), t=2.0)
        self.say("Loudness alone cannot find a hit.", t=2.0)

    def s02_spectrogram(self):
        """The core's own log spectrogram of the demo snare track: the hits stand up as stripes."""
        _, _, spec = core_data()
        img = spectrogram_image(spec)
        self.t_play(FadeIn(img), t=3.0)
        self.t_play(FadeIn(spec_axes()), t=2.0)
        self.say("Each stripe is one hit: energy rising in many bands at once.", t=2.0)

    def s03_flux(self):
        """The core's spectral flux of the snare band under the spectrogram; decays add nothing."""
        _, flux, spec = core_data()
        img = spectrogram_image(spec).scale(0.62).move_to([0.0, 2.6, 0])
        curve, _, _ = flux_curve(flux, yc=-1.3, half=1.0)
        self.t_play(FadeIn(img), t=2.0)
        self.t_play(Create(curve), t=2.5)
        band = Text("the snare's band (the core's role constants): 100 to 300 Hz",
                    font=FONT, font_size=20, color=TEXT)
        band.next_to(curve, DOWN, buff=0.35)
        self.t_play(FadeIn(band), t=2.0)
        # a real decay: the frames right after a kept hit's peak — the core's flux is
        # already back at the floor there (a 1-frame-wide peak, then nothing)
        hits_s = core_data()[0]["hits"]
        after = min(h["time"] for h in hits_s if h["time"] >= T0 + 0.5) + 0.05
        _, seg, pk = flux_curve(flux)
        val = -1.3 + 1.0 * float(seg[int(after * FPS) - int(T0 * FPS)]) / pk
        dot = Dot([x_of(after), val, 0], radius=0.045, color=DIM)
        mark = Text("a decay: falling energy adds nothing", font=FONT, font_size=20, color=DIM)
        mark.move_to([x_of(after), -0.05, 0])
        arrow_line = Line([x_of(after), -0.28, 0], [x_of(after), val + 0.1, 0],
                          color=DIM, stroke_width=2)
        self.t_play(FadeIn(mark), Create(arrow_line), FadeIn(dot), t=2.0)
        # the time axis in s, between the band label and the caption (review of 34c3258)
        axis = VGroup(Text("time (s)", font=FONT, font_size=16, color=DIM).move_to([5.4, -3.05, 0]))
        for ts in (42.0, 44.0, 46.0):
            axis.add(Text(f"{ts:.0f}", font=FONT, font_size=16, color=DIM).move_to([x_of(ts), -3.05, 0]))
        self.t_play(FadeIn(axis), t=1.0)
        self.say("A rise counts. A decay adds nothing.", t=2.0, buff=0.22)

    def s04_peaks(self):
        """The flux curve with its peaks; the core's kept hits in orange; the 20 ms gap."""
        hits, flux, _ = core_data()
        curve, seg, pk = flux_curve(flux, yc=0.0, half=2.0)
        self.t_play(Create(curve), t=3.0)
        # the peaks of the curve: local maxima of the core's flux (grey dots)
        peaks = [i for i in range(1, len(seg) - 1) if seg[i] > seg[i - 1] and seg[i] >= seg[i + 1]
                 and seg[i] > 0.15 * pk]
        grey = VGroup(*[Dot([-PLOT_W / 2 + PLOT_W * p / (len(seg) - 1), -2.0 + 4.0 * seg[p] / pk, 0],
                             radius=0.05, color=DIM) for p in peaks])
        self.t_play(FadeIn(grey), t=2.0)
        kept = VGroup(*[
            Dot([-PLOT_W / 2 + PLOT_W * (h["frame"] * FPS - T0) / (T1 - T0),
                 -2.0 + 4.0 * flux[h["frame"]] / pk, 0], radius=0.08, color=ORANGE)
            for h in hits["hits"] if T0 <= h["time"] < T1
        ])
        self.t_play(FadeIn(kept), t=2.0)
        tag = Text("grey: peaks of the flux. orange: the hits the detector keeps (20 ms apart or more)",
                   font=FONT, font_size=20, color=TEXT).to_edge(UP, buff=0.7)
        self.t_play(FadeIn(tag), t=1.8)
        self.say("The peaks are the candidates, 20 ms apart or more.", t=2.0)

    def s05_gates(self):
        """The kit check on the tom mic: the tom hit keeps its mark, the kick's and the snare's copies get none."""
        t = truth()
        spb, bar1 = t["samples_per_beat"], t["bar1_offset_samples"]
        kit = {h["drum"]: h["t"] / SR for h in t["hits"] if bar1 + 16 * 4 * spb <= h["t"] < bar1 + 17 * 4 * spb}
        lo, hi = kit["kick"] - 0.3, kit["t2"] + 0.3
        x = read_seg(int(lo * SR), int((hi - lo) * SR), mic="t2")
        step = 4
        pic = wave([float(v) for v in x[::step]], -PLOT_W / 2, PLOT_W / 2, 0.8, 1.6, COBALT)
        lane = Line([-PLOT_W / 2, 0.8, 0], [PLOT_W / 2, 0.8, 0], color=DIM)
        name = Text("the demo session's tom mic (t2)", font=FONT, font_size=20, color=TEXT)
        name.to_edge(UP, buff=0.25)
        self.t_play(Create(lane), Create(pic), FadeIn(name), t=3.0)
        hits, _, _ = core_data("t2", "tom")
        kept_at = [h["time"] for h in hits["hits"]]
        marks = VGroup()
        for label, ts in (("tom hit: kept", kit["t2"]),
                          ("the kick's copy:\nno hit", kit["kick"]),
                          ("the snare's copy:\nno hit", kit["snare"])):
            kept = any(abs(ts - k) < 0.02 for k in kept_at)
            color = ORANGE if kept else DIM
            marks.add(Line([x_of(ts, lo, hi), 2.5, 0], [x_of(ts, lo, hi), 1.1, 0], color=color,
                           stroke_width=3),
                      Text(label, font=FONT, font_size=20, color=color, line_spacing=0.2)
                          .move_to([x_of(ts, lo, hi), 3.05, 0]))
        self.t_play(FadeIn(marks), t=2.5)
        rule = Text("loud enough against the track's own reference level, and a shape that fits "
                    "the template of the track's loud hits", font=FONT, font_size=20, color=TEXT)
        rule.next_to(pic, DOWN, buff=0.5)
        self.t_play(FadeIn(rule), t=2.5)
        self.say("Loud enough against the track's own level, and the right shape.", t=2.0)

    def s06_handoff(self):
        """The whole demo snare track with the kept hits; hand-off to explainer 4."""
        t = truth()
        dur = t["duration_samples"] / SR
        lane = lane_line(0.6)
        stalks = VGroup(*[
            Line([-PLOT_W / 2 + PLOT_W * h["time"] / dur, 0.9, 0],
                 [-PLOT_W / 2 + PLOT_W * h["time"] / dur, 0.3, 0], color=ORANGE, stroke_width=2)
            for h in core_data()[0]["hits"]
        ])
        label = Text(f"the demo snare track: {len(core_data()[0]['hits'])} hits kept", font=FONT,
                     font_size=22, color=TEXT).to_edge(UP, buff=0.7)
        self.t_play(Create(lane), Create(stalks), FadeIn(label), t=3.0)
        nxt = Text("now: where exactly does the hit start?", font=FONT, font_size=26, color=TEXT)
        self.t_play(FadeIn(nxt), t=2.0)
        self.say("What is left are the real hits of this track.", t=2.0)
