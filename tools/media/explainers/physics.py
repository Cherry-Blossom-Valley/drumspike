"""#246: the numbers and the synthetic signals the explainers show, as plain functions (no Manim).

The signals are synthetic, not recordings: the demo session (#242) is not on main yet. The
comb-filter numbers are the ones the narration states; tests/test_media_explainers.py checks them.
"""
import functools
import json
import math
import wave
from pathlib import Path

import numpy as np

RATE = 48000
SPEED_M_S = 343.0  # sound in air at about 20 C: 34 cm in 1 ms

# The app's default velocity map and peak window (crates/core/src/velocity.rs)
LO_DB, HI_DB, LO_V, HI_V = -40.0, -6.0, 20, 127
WINDOW_BEFORE_S = 0.005
WINDOW_AFTER_S = 0.060

# The demo session (#242, tools/media/demo_session.py, seed 2026), as the media scripts generate it
DEMO_DIR = Path.home() / ".cache" / "drumspike" / "media" / "demo"


def samples_per_ms(rate=RATE):
    return rate / 1000.0


def cm_per_ms():
    return SPEED_M_S * 0.001 * 100.0


def comb_gain(freq_hz, delay_ms):
    """|1 + e^(-i 2 pi f tau)|: the level of a signal summed with a copy delayed by tau. 0 at a notch, 2 where the copies add."""
    return abs(2.0 * math.cos(math.pi * freq_hz * delay_ms / 1000.0))


def notches(delay_ms, max_hz=4000.0):
    """Frequencies (Hz) where the sum cancels: odd multiples of 1 / (2 tau). None for no delay."""
    if delay_ms <= 0:
        return []
    step = 1000.0 / (2.0 * delay_ms)
    return [(2 * k + 1) * step for k in range(int(max_hz / step) + 1) if (2 * k + 1) * step <= max_hz]


def snare_hit(n=2400, rate=RATE):
    """A synthetic snare hit: two decaying tones and a short crack from a fixed pseudo-random sequence.
    Deterministic, so every run draws the same curve. 2400 samples is 50 ms at 48 kHz."""
    out = []
    seed = 12345
    for i in range(n):
        t = i / rate
        seed = (seed * 1103515245 + 12345) % 2**31
        noise = seed / 2**31 * 2.0 - 1.0
        tones = 0.6 * math.sin(2 * math.pi * 180 * t) * math.exp(-t / 0.04)
        tones += 0.4 * math.sin(2 * math.pi * 330 * t) * math.exp(-t / 0.02)
        crack = 0.5 * noise * math.exp(-t / 0.006)
        out.append(tones + crack)
    peak = max(abs(x) for x in out)
    return [x / peak for x in out]


def peak_db(values, t, rate=RATE):
    """engine.peak_in as dBFS (#325): max |x| in [t - 5 ms, t + 60 ms), int() cut as pyops.window, 0 for an empty window."""
    lo = max(0, int((t - 0.005) * rate))
    hi = min(len(values), int((t + 0.060) * rate))
    peak = max((abs(x) for x in values[lo:hi]), default=0.0)
    return 20.0 * math.log10(max(peak, 1e-9))


def velocity_of_db(d, lo_db=-40.0, hi_db=-6.0, lo_v=20, hi_v=127):
    """engine.velocity_of_db (#325): the default map, linear in dB, ties to even, clamped to lo_v..hi_v."""
    denom = max(hi_db - lo_db, 1e-6)
    span = hi_v - lo_v
    vv = lo_v + round((d - lo_db) / denom * span)
    return min(max(vv, lo_v), hi_v)


def delayed(values, delay_ms, rate=RATE):
    """The signal late by delay_ms, in whole samples (0.1 ms rounds to 5 samples at 48 kHz)."""
    k = round(delay_ms * rate / 1000.0)
    return [0.0] * k + values[:len(values) - k] if k else list(values)


def spectrum(values, freqs, rate=RATE):
    """Magnitude of a direct DFT at each frequency, normalised to the largest value. Small inputs only."""
    mags = []
    for f in freqs:
        re = sum(x * math.cos(2 * math.pi * f * i / rate) for i, x in enumerate(values))
        im = sum(x * math.sin(2 * math.pi * f * i / rate) for i, x in enumerate(values))
        mags.append(math.hypot(re, im))
    top = max(mags) or 1.0
    return [m / top for m in mags]


def first_motion(values, coarse, rate=RATE):
    """Where the walk back stops, as explainer 4's script describes the shipped rule (refine.rs, version 3).

    The attack's peak is taken in the next 5 ms. The threshold is 33 dB under that peak, and at
    least 6 dB over the noise floor (the median of 1 ms block maxima in the 50 ms before the coarse
    time). The walk goes back from the peak while the signal stays over the threshold, and stops
    after 0.5 ms under it. It goes back at most 10 ms. Illustration only: the Rust rule decides.
    """
    per_ms = samples_per_ms(rate)
    peak_end = min(len(values), coarse + int(5 * per_ms))
    peak_idx = max(range(coarse, peak_end), key=lambda i: abs(values[i]))
    peak = abs(values[peak_idx])

    floor_start = max(0, coarse - int(50 * per_ms))
    blocks = [max(abs(x) for x in values[s:s + int(per_ms)])
              for s in range(floor_start, coarse, int(per_ms))]
    blocks.sort()
    floor = blocks[len(blocks) // 2] if blocks else 0.0

    threshold = max(peak * 10 ** (-33 / 20), floor * 10 ** (6 / 20))
    limit = max(0, coarse - int(10 * per_ms))
    gap = int(0.5 * per_ms)
    start, quiet = peak_idx, 0
    for i in range(peak_idx, limit, -1):
        if abs(values[i]) >= threshold:
            start, quiet = i, 0
        else:
            quiet += 1
            if quiet >= gap:
                break
    return start


def velocity_of_db(d, lo_db=LO_DB, hi_db=HI_DB, lo_v=LO_V, hi_v=HI_V):
    """A peak level d (dBFS) as the app's velocity: linear in dB, rounded (ties to even), clamped. velocity.rs."""
    denom = max(hi_db - lo_db, 1e-6)
    vv = lo_v + round((d - lo_db) / denom * (hi_v - lo_v))
    return max(lo_v, min(hi_v, vv))


def map_points(levels):
    """(level, velocity) for each level: the points that explainer 8 draws on the velocity map."""
    return [(d, velocity_of_db(d)) for d in levels]


def read_wav_mono(path):
    """The first channel of a PCM WAV as floats in -1..1 (16, 24 bit, or 32 bit float)."""
    with wave.open(str(path), "rb") as w:
        channels, width, n = w.getnchannels(), w.getsampwidth(), w.getnframes()
        raw = w.readframes(n)
    if width == 2:
        x = np.frombuffer(raw, dtype="<i2").astype(np.float64) / 32768.0
    elif width == 3:
        b = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3).astype(np.int32)
        v = b[:, 0] | (b[:, 1] << 8) | (b[:, 2] << 16)
        x = np.where(v & 0x800000, v - 0x1000000, v).astype(np.float64) / 8388608.0
    elif width == 4:
        x = np.frombuffer(raw, dtype="<f4").astype(np.float64)
    else:
        raise ValueError(f"{path}: {width}-byte samples are not read")
    return x.reshape(-1, channels)[:, 0]


@functools.lru_cache(maxsize=None)
def demo_snare_track():
    """The demo session's snare top mic (s top.wav), as floats; the file must be generated first."""
    return read_wav_mono(DEMO_DIR / "s top.wav")


@functools.lru_cache(maxsize=None)
def demo_track(name):
    """One mic of the demo session as floats, e.g. 't1' (tom 1's close mic); the file must be generated first."""
    return read_wav_mono(DEMO_DIR / f"{name}.wav")


def demo_snare_hits():
    """The snare hits of truth.json: [(time in s, ghost note)] in time order."""
    truth = json.loads((DEMO_DIR / "truth.json").read_text(encoding="utf-8"))
    return [(h["t"] / truth["sr"], h["ghost"]) for h in truth["hits"] if h["drum"] == "snare"]


def demo_hit_times(drums):
    """The times (s) of the truth.json hits of the drums named, in time order."""
    truth = json.loads((DEMO_DIR / "truth.json").read_text(encoding="utf-8"))
    return [h["t"] / truth["sr"] for h in truth["hits"] if h["drum"] in drums]


def nearest_hit_time(t):
    """The time of the snare hit of the demo session nearest to t (s)."""
    return min((h[0] for h in demo_snare_hits()), key=lambda s: abs(s - t))


def hit_window(x, t, rate=RATE):
    """The samples of the app's peak window of the hit at t: 5 ms before to 60 ms after it."""
    a = max(0, round((t - WINDOW_BEFORE_S) * rate))
    b = min(len(x), round((t + WINDOW_AFTER_S) * rate))
    return x[a:b]


def hit_peak_db(x, t, rate=RATE):
    """The peak level of the hit at t in dBFS (the largest |sample| of its window)."""
    peak = float(np.max(np.abs(hit_window(x, t, rate))))
    return 20.0 * math.log10(max(peak, 1e-9))
