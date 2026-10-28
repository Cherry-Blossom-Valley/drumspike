"""Shared test helpers: synthetic drum audio with known onsets."""
import sys
import wave
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

SR = 48000


def kick_hit(n, sr=SR, attack_ms=0.5, phase=0.0):
    """Deterministic kick: pitch sweep 125 -> 55 Hz, short attack ramp, exponential decay."""
    t = np.arange(n) / sr
    f = 55 + 70 * np.exp(-t / 0.03)
    ph = 2 * np.pi * np.cumsum(f) / sr + phase
    env = np.minimum(1.0, t / (attack_ms / 1000)) * np.exp(-t / 0.12)
    return np.sin(ph) * env


def synth_track(onsets_s, dur_s, sr=SR, gains_db=None, attacks_ms=None, phases=None):
    """Mono float track with one kick hit at each onset (seconds). Returns (x, onset_samples)."""
    x = np.zeros(int(dur_s * sr))
    idx = [int(round(t * sr)) for t in onsets_s]
    gains_db = gains_db or [-3.0] * len(idx)
    attacks_ms = attacks_ms or [0.5] * len(idx)
    phases = phases or [0.0] * len(idx)
    for i, g, a, ph in zip(idx, gains_db, attacks_ms, phases):
        h = kick_hit(int(0.4 * sr), sr, a, ph) * 10 ** (g / 20)
        x[i:i + len(h)] += h[: len(x) - i]
    return x, idx


def write_wav(path, x, sr=SR, channels=1):
    """16-bit PCM WAV (what most DAWs export); ffmpeg reads it like any session file."""
    data = np.clip(x, -1, 1)
    if channels == 2:
        data = np.stack([data, data], axis=1).reshape(-1)
    pcm = (data * 32767).astype("<i2").tobytes()
    with wave.open(str(path), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(pcm)
    return path
