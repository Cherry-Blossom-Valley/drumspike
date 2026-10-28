"""The synthetic kick and snare tracks of the refinement benchmark (bench/refine_bench.py until P3-12).

P3-12: the benchmark compared methods with engine._refine_sr of the retired server and is gone; its
signal generator stays here, unchanged, because the "bench" cases of check_refine.py are frozen
on these signals (tests/golden/frozen/refine.json keeps the SHA-256 of every WAV file). The order of
the calls matters: one seeded generator (rng) draws every random number, so check_refine.py calls
make_track and rng.uniform in the benchmark's loop order.
"""
import numpy as np

SR = 48000
KICK_TAU = 0.12
rng = np.random.default_rng(7)


def kick_hit(n, attack_ms, phase):
    t = np.arange(n) / SR
    f = 55 + 70 * np.exp(-t / 0.03)                       # pitch sweep 125 -> 55 Hz
    ph = 2 * np.pi * np.cumsum(f) / SR + phase
    env = np.minimum(1, t / (attack_ms / 1000)) * np.exp(-t / KICK_TAU)
    click = rng.standard_normal(n) * np.exp(-t / 0.0015) * 0.35
    click[: int(0.0002 * SR)] *= np.linspace(0, 1, int(0.0002 * SR))
    return np.sin(ph) * env + click


def snare_hit(n, attack_ms, phase):
    t = np.arange(n) / SR
    env = np.minimum(1, t / (attack_ms / 1000))
    body = np.sin(2 * np.pi * 190 * t + phase) * np.exp(-t / 0.08)
    wires = rng.standard_normal(n) * np.exp(-t / 0.12) * 0.6
    return (body + wires) * env


def make_track(kind, scenario, n_hits=150, bleed_db=-22.0):
    """scenario: clean | doubles | ghosts | bleed. Returns x, onsets (samples), gains (dB)."""
    x = np.zeros(int((n_hits * 0.4 + 2) * SR))
    onsets, gains = [], []
    hit = kick_hit if kind == "kick" else snare_hit
    other = snare_hit if kind == "kick" else kick_hit
    t = 0.5
    for k in range(n_hits):
        if scenario == "doubles":      # 16ths at 200 bpm = 75 ms
            gap, g_db = 0.075, rng.uniform(-6, 0)
        elif scenario == "ghosts":     # ghost note 90-150 ms after a loud hit
            gap = 0.45 if k % 2 == 0 else rng.uniform(0.09, 0.15)
            g_db = rng.uniform(-6, 0) if k % 2 == 0 else rng.uniform(-26, -16)
        else:
            gap, g_db = 0.4, rng.uniform(-12, 0)
        t += gap
        i = int(round(t * SR)); g = 10 ** (g_db / 20)
        h = hit(int(0.6 * SR), rng.uniform(0.3, 3.0), rng.uniform(0, 2 * np.pi)) * g
        x[i:i + len(h)] += h[: len(x) - i]
        onsets.append(i); gains.append(g_db)
        if scenario == "bleed":        # other drum 120-200 ms later, 1.5 ms acoustic delay
            j = i + int(rng.uniform(0.12, 0.2) * SR) + int(0.0015 * SR)
            b = other(int(0.4 * SR), 1.0, rng.uniform(0, 2 * np.pi)) * 10 ** ((bleed_db + rng.uniform(-6, 6)) / 20)
            x[j:j + len(b)] += b[: len(x) - j]
    x += rng.standard_normal(len(x)) * 10 ** (-72 / 20)
    return x.astype(np.float32), np.array(onsets), np.array(gains)


