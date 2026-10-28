"""P4-11: the calibration kit. Onset bias of Drum Spike per drum shell against a sample-exact ground truth.

    uv run python tools/calib_kit.py all                 # generate, run, compare (one command)
    uv run python tools/calib_kit.py generate [--quick]  # audio, reference MIDI and CSV
    uv run python tools/calib_kit.py run [--jobs 2]      # detection through the agent CLI (service_tool cli run)
    uv run python tools/calib_kit.py compare             # match, statistics, tables.md and compare.json

Everything is written under ``~/.cache/drumspike/calib-kit/`` (``--out`` changes it); nothing goes into git. The
Logic samples are read only. The project folders and the app home of the runs are under the output folder too.

Shells: kick, snare, t1 to t4 (toms, high to low). Two sample sets:

  syn   synthetic one-shots. A hit starts at a known sample (the first non-zero sample); variants: attack rise
        0, 0.5, 2 and 5 ms, levels -6, -20 and -35 dBFS, both polarities, fractional starts 0, 0.25, 0.5 and
        0.75 sample (windowed-sinc delay; the reference time is then fractional). 48 kHz 24-bit; the clean file
        also at 44.1 and 96 kHz.
  real  one-shots of the Logic content (Ultrabeat Samples, 44.1 kHz, read natively: a resampled file would blur
        the start). Each has two references: `abs` (first sample above -90 dBFS) and `aud` (first sample within
        40 dB of the peak), and its features (10-90 % rise, centroid, decay, peak level, pre-silence, the gap).

Per shell and set: a mix file (the hits at off-grid positions, gaps of 150 ms or more, one block of close pairs
20 to 60 ms apart), the reference MIDI (format 1, 24,000 PPQ, 120 BPM: 48,000 ticks per second) and the CSV with
the exact start of every event in samples (float). Conditions: clean, noise (white, -60 dBFS RMS), bleed (the next
shell's clean mix at -20 dB). Detector: the DSP detector, at the
defaults of the track's role (kick, snare, tom).

Compare: D is matched to R within +-10 ms (one to one, nearest first). The onset error is D minus R in samples,
read from the events (sample exact) and from the exported MIDI D.
"""
import argparse
import csv
import glob
import json
import math
import os
import re
import shutil
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import mido
import numpy as np
import soundfile as sf

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUT = Path.home() / ".cache" / "drumspike" / "calib-kit"
LOGIC = Path("/Library/Application Support/Logic/Ultrabeat Samples")

SHELLS = ["kick", "snare", "t1", "t2", "t3", "t4"]
ROLE = {"kick": "kick", "snare": "snare", "t1": "tom", "t2": "tom", "t3": "tom", "t4": "tom"}
NOTE = {"kick": 36, "snare": 38, "t1": 50, "t2": 47, "t3": 45, "t4": 41}
BLEED_FROM = {"kick": "snare", "snare": "kick", "t1": "snare", "t2": "t1", "t3": "t2", "t4": "t3"}
RISES_MS = [0.0, 0.5, 2.0, 5.0]
LEVELS_DB = [-6.0, -20.0, -35.0]
POLARITIES = [1, -1]
FRACS = [0.0, 0.25, 0.5, 0.75]
CONDITIONS = ["clean", "noise", "bleed"]
DETECTORS = ["dsp"]  # the DSP detector; the detector names of the frozen results
NOISE_DB = -60.0
BLEED_DB = -20.0
PPQ = 24000
TICKS_PER_S = 48000.0          # 120 BPM, 24,000 PPQ
MATCH_S = 0.010
FAR_S = 0.050
SINC_HALF = 32
SR_MAIN = 48000
SR_EXTRA = [44100, 96000]
SR_REAL = 44100

TOM_F0 = {"t1": 230.0, "t2": 170.0, "t3": 125.0, "t4": 88.0}
TOM_DECAY_S = {"t1": 0.20, "t2": 0.24, "t3": 0.28, "t4": 0.34}


# ---------------------------------------------------------------- synthetic one-shots

def _bandpass_noise(rng, n, sr, lo, hi):
    spec = np.fft.rfft(rng.standard_normal(n))
    f = np.fft.rfftfreq(n, 1.0 / sr)
    spec[(f < lo) | (f > hi)] = 0.0
    x = np.fft.irfft(spec, n)
    return x / np.max(np.abs(x))


def body(shell, sr, rng, tune=None, click=None, band=None, phase0=0.7):
    """The one-shot of a shell before attack shaping: a float array with a non-zero first sample.
    Sweep parameters (None keeps the default shape of the plain kit): `tune` the pitch in Hz (kick: the end of the
    sweep; snare: the tone; toms: the fundamental), `click` the level of a 3 ms broadband burst against the body
    (0 = none), `band` 0 to 1 the position of the snare noise (0 mids 200 Hz to 2 kHz, 1 highs 4 to 12 kHz)."""
    n = int(0.5 * sr)
    t = np.arange(n) / sr
    burst = _bandpass_noise(rng, n, sr, 1000, 12000) * np.exp(-t / 0.003)
    if shell == "kick":
        f_end = 45.0 if tune is None else tune
        f = f_end + (105.0 + (f_end - 45.0)) * np.exp(-t / 0.025)   # swept sine, ends at f_end
        ph = phase0 + 2 * np.pi * np.cumsum(f) / sr
        x = np.sin(ph) * np.exp(-t / 0.11)
        if click is None:
            x = x + _bandpass_noise(rng, n, sr, 2000, 8000) * np.exp(-t / 0.002) * 0.5
        else:
            x = x + burst * click
    elif shell == "snare":
        f0 = 190.0 if tune is None else tune
        tone = np.sin(phase0 + 2 * np.pi * f0 * t) * np.exp(-t / 0.07)
        if band is None:
            noise = _bandpass_noise(rng, n, sr, 1500, 9000)
        else:
            noise = ((1 - band) * _bandpass_noise(rng, n, sr, 200, 2000)
                     + band * _bandpass_noise(rng, n, sr, 4000, 12000))
            noise = noise / np.max(np.abs(noise))
        x = 0.6 * tone + 0.8 * noise * np.exp(-t / 0.11)
        if click is not None:
            x = x + burst * click
    else:
        f0 = TOM_F0[shell] if tune is None else tune
        f = f0 * (1.0 + 0.25 * np.exp(-t / 0.03))
        ph = phase0 + 2 * np.pi * np.cumsum(f) / sr
        x = np.sin(ph) * np.exp(-t / TOM_DECAY_S[shell]) + 0.1 * np.sin(2.3 * ph) * np.exp(-t / 0.05)
        if click is not None:
            x = x + burst * click
    x = x * np.minimum(1.0, (n - np.arange(n)) / (0.05 * sr))        # fade out the last 50 ms
    x[0] = x[0] if x[0] != 0.0 else 1e-3
    return x / np.max(np.abs(x))


def frac_delay(x, d, half=SINC_HALF):
    """`x` delayed by `d` samples (0 <= d < 1) with a Kaiser-windowed sinc. The result is `half` samples longer at
    both ends; x[0] sits at index `half`. For d = 0 the input is returned unchanged with the same offset."""
    pad = np.concatenate([np.zeros(half), x, np.zeros(half)])
    if d == 0.0:
        return pad
    k = np.arange(-half, half + 1)
    h = np.sinc(k - d) * np.kaiser(2 * half + 1, 8.0)
    h /= h.sum()
    return np.convolve(pad, h, mode="same")


def ramp(n, r, shape="lin"):
    """The attack ramp of `r` samples (1.0 after it): linear, cosine (S curve) or exponential (slow start)."""
    u = np.minimum(1.0, (np.arange(n) + 1.0) / r)
    if shape == "cos":
        return 0.5 * (1.0 - np.cos(np.pi * u))
    if shape == "exp":
        return (np.exp(4.0 * u) - 1.0) / (np.exp(4.0) - 1.0)
    return u


def make_shot(shell, sr, rise_ms, level_db, pol, frac, seed, tune=None, click=None, band=None, phase0=0.7,
              shape="lin"):
    """One synthetic one-shot. Returns dict(y, lead, abs_off, aud_off, feat): `y[lead]` is the nominal first
    non-zero sample, the reference (`abs_off`) is lead + frac, `aud_off` the first sample within 40 dB of the peak."""
    rng = np.random.default_rng(seed)
    x = body(shell, sr, rng, tune, click, band, phase0)
    r = int(round(rise_ms * 1e-3 * sr))
    if r > 0:
        x = x * ramp(len(x), r, shape)
        x = x / np.max(np.abs(x))
    x = x * pol * 10 ** (level_db / 20.0)
    y = frac_delay(x, frac)
    lead = SINC_HALF
    y = y.copy()
    peak = float(np.max(np.abs(y)))
    aud = int(np.argmax(np.abs(y) >= peak * 0.01))
    return {"y": y, "lead": lead, "abs_off": lead + frac, "aud_off": aud, "peak_db": 20 * math.log10(peak),
            "fp": fingerprint(y, sr, aud)}


# ---------------------------------------------------------------- features of a sample (rise, centroid, decay)

def features(y, sr):
    """Rise 10-90 % (ms), spectral centroid (Hz), decay to -30 dB (ms), peak (dBFS), the two starts (samples)."""
    a = np.abs(y)
    peak = float(a.max())
    thr = 10 ** (-90 / 20)
    abs_start = int(np.argmax(a > thr)) if (a > thr).any() else 0
    aud_start = int(np.argmax(a >= peak * 0.01))
    w = max(1, int(0.001 * sr))
    n = len(a)
    pad = np.concatenate([a, np.zeros(w)])
    env = np.max(np.lib.stride_tricks.sliding_window_view(pad, w)[:n], axis=1)
    ipk = int(np.argmax(env))
    t10 = int(np.argmax(env >= 0.1 * env[ipk]))
    t90 = int(np.argmax(env >= 0.9 * env[ipk]))
    rise_ms = (t90 - t10) / sr * 1000.0
    after = env[ipk:]
    below = np.nonzero(after < env[ipk] * 10 ** (-30 / 20))[0]
    decay_ms = (below[0] if len(below) else len(after)) / sr * 1000.0
    seg = y[aud_start:aud_start + int(0.1 * sr)]
    sp = np.abs(np.fft.rfft(seg * np.hanning(len(seg)))) if len(seg) > 8 else np.zeros(2)
    fr = np.fft.rfftfreq(len(seg), 1.0 / sr) if len(seg) > 8 else np.zeros(2)
    centroid = float((sp * fr).sum() / sp.sum()) if sp.sum() > 0 else 0.0
    return {"rise_ms": round(rise_ms, 3), "centroid_hz": round(centroid, 1), "decay_ms": round(decay_ms, 1),
            "peak_db": round(20 * math.log10(peak), 2), "abs_start": abs_start, "aud_start": aud_start,
            "gap_samples": aud_start - abs_start, "presilence_ms": round(abs_start / sr * 1000.0, 2)}


def fingerprint(y, sr, aud):
    """The spectral fingerprint of a hit, from the audible start `aud` (first sample within 40 dB of the peak): the
    energy shares of the first 10 ms in three bands (low < 200 Hz, mid 200 Hz to 2 kHz, high > 2 kHz), the high to mid
    and low to mid ratios in dB, the centroid and the 85 % roll-off of those 10 ms, the attack strength (peak of the
    first 5 ms against the body peak after them, dB), the rise time 10-90 %, the fundamental where clear (Hz, else
    NaN) and the decay time to -30 dB (ms)."""
    ft = features(y, sr)
    seg = y[aud:aud + int(0.010 * sr)]
    nfft = 8192
    sp = np.abs(np.fft.rfft(seg * np.hanning(len(seg)), nfft)) ** 2 if len(seg) > 8 else np.ones(nfft // 2 + 1)
    f = np.fft.rfftfreq(nfft, 1.0 / sr)
    tot = sp.sum() + 1e-30
    lo, mid, hi = sp[f < 200].sum(), sp[(f >= 200) & (f < 2000)].sum(), sp[f >= 2000].sum()
    cum = np.cumsum(sp) / tot
    a5 = np.abs(y[aud:aud + int(0.005 * sr)])
    body_pk = np.abs(y[aud + int(0.005 * sr):aud + int(0.2 * sr)])
    attack_db = 20 * math.log10((a5.max() + 1e-12) / (body_pk.max() + 1e-12)) if len(a5) and len(body_pk) else 0.0
    # fundamental: the strongest peak of 30 to 500 Hz in 20 to 220 ms after the start, when it stands 10 dB above
    # the median of that band
    seg2 = y[aud + int(0.02 * sr):aud + int(0.22 * sr)]
    f0 = float("nan")
    if len(seg2) > 100:
        s2 = np.abs(np.fft.rfft(seg2 * np.hanning(len(seg2)), 65536))
        f2 = np.fft.rfftfreq(65536, 1.0 / sr)
        m = (f2 >= 30) & (f2 <= 500)
        if m.any() and s2[m].max() > 3.16 * (np.median(s2[m]) + 1e-18):
            f0 = float(f2[m][np.argmax(s2[m])])
    return {"e_low": float(lo / tot), "e_mid": float(mid / tot), "e_high": float(hi / tot),
            "hi_mid_db": 10 * math.log10((hi + 1e-30) / (mid + 1e-30)),
            "low_mid_db": 10 * math.log10((lo + 1e-30) / (mid + 1e-30)),
            "centroid10_hz": float((sp * f).sum() / tot),
            "rolloff85_hz": float(f[int(np.searchsorted(cum, 0.85))]),
            "attack_db": attack_db, "rise1090_ms": ft["rise_ms"], "f0_hz": f0, "decay_ms": ft["decay_ms"]}


# ---------------------------------------------------------------- real one-shots

ELECTRONIC_KITS = {"Boutique 78", "Boutique 808", "Electro Bump", "Epic Electro", "Modern Club", "Drum Machine Designer"}
POOL_RE = {
    "kick": re.compile(r"(kick|(^|[_ ])bd(\d|[_ .]|$))", re.I),
    "snare": re.compile(r"(snare|(^|[_ ])sd(\d|[_ .]|$)|snr)", re.I),
    "tom": re.compile(r"tom", re.I),
}
EXCLUDE_RE = re.compile(r"(rim|side|stick|clap|hat|loop|crash|ride|cym|perc|cowbell|conga|bongo|timbal|tomtom_loop)", re.I)


def read_mono(path):
    x, sr = sf.read(str(path), dtype="float64", always_2d=True)
    return x.mean(axis=1), sr


def real_pool(logic=LOGIC):
    """Candidate files per class (kick, snare, tom) with features; the files are read only."""
    pool = {"kick": [], "snare": [], "tom": []}
    seen = set()
    for f in sorted(glob.glob(str(logic / "**" / "*.aif"), recursive=True)):
        stem = Path(f).stem
        if EXCLUDE_RE.search(stem):
            continue
        cls = next((c for c, rx in POOL_RE.items() if rx.search(stem)), None)
        if cls is None:
            continue
        x, sr = read_mono(f)
        if sr != SR_REAL or len(x) < 2000 or np.abs(x).max() < 10 ** (-40 / 20):
            continue
        key = (len(x), round(float(np.abs(x).sum()), 3))
        if key in seen:
            continue
        seen.add(key)
        ft = features(x, sr)
        if ft["presilence_ms"] > 300.0:
            continue
        kit = Path(f).parent.name
        pool[cls].append({"path": f, "name": stem, "kit": kit, "family": "electronic" if kit in ELECTRONIC_KITS else "acoustic-or-mixed",
                          **ft})
    return pool


def spread(items, k):
    """Pick k items far apart in (log rise, log centroid, log decay, log pre-silence) space (farthest point)."""
    if len(items) <= k:
        return list(items)
    m = np.array([[math.log10(max(i["rise_ms"], 0.05)), math.log10(max(i["centroid_hz"], 20)),
                   math.log10(max(i["decay_ms"], 5)), math.log10(i["presilence_ms"] + 0.5)] for i in items])
    m = (m - m.mean(0)) / (m.std(0) + 1e-9)
    chosen = [int(np.argmax(np.linalg.norm(m - m.mean(0), axis=1)))]
    d = np.linalg.norm(m - m[chosen[0]], axis=1)
    while len(chosen) < k:
        # a small bonus for a kit not chosen yet
        kits = {items[c]["kit"] for c in chosen}
        score = d + np.array([0.4 if i["kit"] not in kits else 0.0 for i in items])
        score[chosen] = -1
        c = int(np.argmax(score))
        chosen.append(c)
        d = np.minimum(d, np.linalg.norm(m - m[c], axis=1))
    return [items[c] for c in chosen]


def choose_real(pool, per_shell=12):
    """Samples per shell. Toms are split into four groups by centroid (t1 the highest)."""
    out = {"kick": spread(pool["kick"], per_shell), "snare": spread(pool["snare"], per_shell)}
    toms = sorted(pool["tom"], key=lambda i: -i["centroid_hz"])
    q = len(toms) // 4
    for k, s in enumerate(["t1", "t2", "t3", "t4"]):
        grp = toms[k * q:(k + 1) * q] if k < 3 else toms[3 * q:]
        out[s] = spread(grp, per_shell)
    return out


def real_shot(entry, gain_db, max_s=0.8):
    x, sr = read_mono(entry["path"])
    n = int(max_s * sr) + entry["abs_start"]
    y = x[:n].copy()
    fade = int(0.05 * sr)
    if len(y) > fade:
        y[-fade:] *= np.linspace(1, 0, fade)
    y = y * 10 ** (gain_db / 20.0)
    ft = features(y, sr)
    return {"y": y, "lead": 0, "abs_off": float(ft["abs_start"]), "aud_off": ft["aud_start"], "peak_db": ft["peak_db"],
            "name": entry["name"], "fp": fingerprint(y, 44100, ft["aud_start"])}


# ---------------------------------------------------------------- the mix of a shell

def place(buf, shot, q):
    y = shot["y"]
    a, b = max(q, 0), min(q + len(y), len(buf))
    buf[a:b] += y[a - q:b - q]


def velocity(level_db, rng):
    return int(np.clip(round(127 + 2.2 * level_db + rng.integers(-4, 5)), 1, 127))


def build_mix(shell, sr, specs, pair_specs, seed, kind):
    """The events of one shell. `specs` are the main shots in a shuffled order, `pair_specs` the shots of the block of
    close pairs (two per pair). A spec is a dict(shot, meta). Returns (mix float array, events list).
    Positions are off-grid integer samples; the gap after a shot is 300 to 550 ms (synthetic) or its length plus up
    to 200 ms (real), never below 150 ms; pairs are 20 to 60 ms apart."""
    rng = np.random.default_rng(seed)
    events = []
    pos = int(0.5 * sr) + int(rng.integers(0, 997))
    seq = []   # main events, then the block of pairs

    def gap(shot):
        if kind == "syn":
            return int(rng.uniform(0.30, 0.55) * sr)
        return max(int(0.15 * sr), len(shot["y"]) + int(rng.uniform(0.05, 0.20) * sr))

    for s in specs:
        seq.append((s, pos, "main", None))
        pos += gap(s["shot"])
    pairs = list(zip(pair_specs[0::2], pair_specs[1::2]))
    for pid, (a, b) in enumerate(pairs):
        d = int(rng.uniform(0.020, 0.060) * sr)
        seq.append((a, pos, "pair", pid))
        seq.append((b, pos + d, "pair", pid))
        pos += d + gap(b["shot"])
    total = pos + int(0.5 * sr)
    mix = np.zeros(total + 4096)
    for i, (s, q, grp, pid) in enumerate(seq):
        shot = s["shot"]
        base = q - shot["lead"]
        place(mix, shot, base)
        abs_ref = base + shot["abs_off"]
        aud_ref = base + shot["aud_off"]
        events.append({"id": i, "shell": shell, "sr": sr, "group": grp, "pair": pid, "abs_sample": float(abs_ref),
                       "aud_sample": float(aud_ref), "abs_s": abs_ref / sr, "velocity": s["vel"], **shot["fp"], **s["meta"]})
    return mix[:total], events


def syn_specs(shell, sr, quick, seed):
    rises, levels, pols, fracs = (([0.0, 2.0], [-6.0, -35.0], [1], [0.0, 0.5]) if quick
                                  else (RISES_MS, LEVELS_DB, POLARITIES, FRACS))
    rng = np.random.default_rng(seed)
    specs = []
    n = 0
    for r in rises:
        for lv in levels:
            for p in pols:
                for fr in fracs:
                    shot = make_shot(shell, sr, r, lv, p, fr, seed=seed + (1000 * n) % 7919)
                    specs.append({"shot": shot, "vel": velocity(lv, rng),
                                  "meta": {"rise_ms": r, "level_db": lv, "pol": p, "frac": fr}})
                    n += 1
    order = rng.permutation(len(specs))
    specs = [specs[i] for i in order]
    k = 4 if quick else 12
    pairs = []
    for j in range(2 * k):
        shot = make_shot(shell, sr, 0.5, -6.0 if j % 2 == 0 else -14.0, 1, 0.0, seed=seed + 77 + j)
        pairs.append({"shot": shot, "vel": velocity(-6.0, rng),
                      "meta": {"rise_ms": 0.5, "level_db": -6.0 if j % 2 == 0 else -14.0, "pol": 1, "frac": 0.0}})
    return specs, pairs


SWEEP_LEVEL_DB = -6.0
SWEEP_CLICKS = [2.0, 1.0, 0.5, 0.2, 0.0]
SWEEP_BANDS = [0.0, 0.25, 0.5, 0.75, 1.0]
SWEEP_TUNE = {"kick": [35.0, 45.0, 55.0, 65.0, 75.0], "snare": [150.0, 190.0, 230.0, 280.0, 330.0]}
SWEEP_TUNE_RATIO = [0.7, 0.85, 1.0, 1.2, 1.4]


def sweep_axes(shell, quick):
    """(axis, value, kwargs of make_shot) for a shell. The base shape: kick 45 Hz, snare 190 Hz and band 0.5, toms
    their own pitch; click 0.5 (a click level against the body: 2 strong, 0 none)."""
    base = {"click": 0.5}
    if shell == "snare":
        base["band"] = 0.5
    axes = []
    if shell in SWEEP_TUNE:
        tunes = SWEEP_TUNE[shell]
    else:
        tunes = [TOM_F0[shell] * r for r in SWEEP_TUNE_RATIO]
    pick = (lambda v: [v[0], v[-1]]) if quick else (lambda v: v)
    for v in pick(tunes):
        axes.append(("tune", v, {**base, "tune": v}))
    for v in pick(SWEEP_CLICKS):
        axes.append(("click", v, {**base, "click": v}))
    if shell == "snare":
        for v in pick(SWEEP_BANDS):
            axes.append(("band", v, {**base, "band": v}))
    return axes


def sweep_specs(shell, sr, quick, seed):
    """The synthetic sweeps of the owner's fingerprint question: tuning, click level (attack strength) and, for the
    snare, the noise band (mids to highs), each six times with another noise seed and position."""
    rng = np.random.default_rng(seed)
    specs = []
    n = 0
    for axis, v, kw in sweep_axes(shell, quick):
        for rep in range(1 if quick else 6):
            n += 1
            shot = make_shot(shell, sr, 0.0, SWEEP_LEVEL_DB, 1, 0.0, seed=seed + 31 * n, **kw)
            meta = {"axis": axis, "axis_value": v, "rise_ms": 0.0, "level_db": SWEEP_LEVEL_DB, "pol": 1, "frac": 0.0,
                    "tune_hz": kw.get("tune", float("nan")), "click": kw["click"], "band": kw.get("band", float("nan"))}
            specs.append({"shot": shot, "vel": velocity(SWEEP_LEVEL_DB, rng), "meta": meta})
    order = rng.permutation(len(specs))
    specs = [specs[i] for i in order]
    pairs = []
    for j in range(2 * (3 if quick else 8)):
        shot = make_shot(shell, sr, 0.5, -6.0 if j % 2 == 0 else -14.0, 1, 0.0, seed=seed + 977 + j)
        pairs.append({"shot": shot, "vel": velocity(-6.0, rng),
                      "meta": {"axis": "pair", "axis_value": 0.0, "rise_ms": 0.5, "level_db": -6.0, "pol": 1, "frac": 0.0,
                               "tune_hz": float("nan"), "click": float("nan"), "band": float("nan")}})
    return specs, pairs


def real_specs(shell, entries, quick, seed):
    rng = np.random.default_rng(seed)
    specs = []
    for rep in range(1 if quick else 2):
        for e in entries:
            for g in ([0.0] if quick else [0.0, -20.0]):
                shot = real_shot(e, g)
                specs.append({"shot": shot, "vel": velocity(shot["peak_db"], rng),
                              "meta": {"sample": e["name"], "kit": e["kit"], "family": e["family"], "gain_db": g,
                                       "rise_ms": e["rise_ms"], "centroid_hz": e["centroid_hz"],
                                       "decay_ms": e["decay_ms"], "level_db": shot["peak_db"],
                                       "presilence_ms": e["presilence_ms"], "gap_samples": e["gap_samples"]}})
    order = rng.permutation(len(specs))
    specs = [specs[i] for i in order]
    k = 3 if quick else 8
    pairs = []
    for j in range(2 * k):
        e = entries[int(rng.integers(0, len(entries)))]
        shot = real_shot(e, 0.0 if j % 2 == 0 else -10.0)
        pairs.append({"shot": shot, "vel": velocity(shot["peak_db"], rng),
                      "meta": {"sample": e["name"], "kit": e["kit"], "family": e["family"], "gain_db": 0.0,
                               "rise_ms": e["rise_ms"], "centroid_hz": e["centroid_hz"], "decay_ms": e["decay_ms"],
                               "level_db": shot["peak_db"], "presilence_ms": e["presilence_ms"],
                               "gap_samples": e["gap_samples"]}})
    return specs, pairs


# ---------------------------------------------------------------- acoustic one-shots (Drum Kit Designer, consolidated)

DKD = Path("/Library/Application Support/Logic/EXS Factory Samples/Drum Kit Designer Consolidated")
ACOUSTIC_GLOBS = {"kick": "**/*Kick*_consolidated.caf", "snare": "**/*Snare*_consolidated.caf",
                  "tom": "**/*Toms*_consolidated.caf"}


def slice_by_gaps(level, sr, gap_s=0.02, thr_db=-90.0, min_s=0.05):
    """Slices of a consolidated file: `level` is the per-sample peak over the channels. A gap is a run of at least
    `gap_s` seconds with every sample at or below `thr_db`; a slice starts at the first sample above `thr_db` after a
    gap (or at the start of the file) and ends after its last sample above the threshold before the next gap. Returns
    [(start, end)] (end exclusive), slices shorter than `min_s` dropped."""
    thr = 10 ** (thr_db / 20.0)
    idx = np.flatnonzero(np.asarray(level) > thr)
    if len(idx) == 0:
        return []
    n = max(1, int(round(gap_s * sr)))
    cuts = np.flatnonzero(np.diff(idx) - 1 >= n)          # a gap of >= n silent samples between idx[k] and idx[k+1]
    starts = np.concatenate([[idx[0]], idx[cuts + 1]])
    ends = np.concatenate([idx[cuts] + 1, [idx[-1] + 1]])
    return [(int(a), int(b)) for a, b in zip(starts, ends) if b - a >= min_s * sr]


ACOUSTIC_FLOOR_DB = 20.0
ACOUSTIC_MAX_GAP_S = 0.005   # `aud` more than 5 ms after `abs`: dropped (SoCal Kit Snare#38)


def cut_shot(y, sr, max_s=0.8):
    """The part of a slice that goes into the mix: 0.8 s with a 50 ms fade at the cut (a slice with several hits is
    cut inside the first). The features of the shot are taken from this cut."""
    y = y[:int(max_s * sr)].copy()
    fade = int(0.05 * sr)
    if len(y) > fade:
        y[-fade:] *= np.linspace(1, 0, fade)
    return y


def acoustic_pool(per_shell=30, root=DKD):
    """Slices per shell (kick, snare, t1 to t4) from the consolidated files, spread over velocity (the peak level).
    The toms file holds several toms: its slices are split in four groups by spectral centroid (t1 the highest)."""
    pool = {"kick": [], "snare": [], "tom": []}
    for cls, pat in ACOUSTIC_GLOBS.items():
        for f in sorted(glob.glob(str(root / pat), recursive=True)):
            x, sr = sf.read(f, dtype="float64", always_2d=True)
            if sr != SR_REAL:
                continue
            kit = Path(f).stem.replace("_consolidated", "")
            file_peak_db = 20 * math.log10(np.abs(x).max() + 1e-12)
            for i, (a, b) in enumerate(slice_by_gaps(np.abs(x).max(axis=1), sr)):
                y = cut_shot(x[a:b].mean(axis=1), sr)
                ft = features(y, sr)
                if ft["peak_db"] < file_peak_db - ACOUSTIC_FLOOR_DB:   # a fragment (tail, noise), not a hit
                    continue
                if ft["gap_samples"] > ACOUSTIC_MAX_GAP_S * sr:        # a long run-in under -40 dB: no clear start
                    continue
                pool[cls].append({"y": y, "name": f"{kit}#{i}", "kit": kit, "family": "acoustic", **ft,
                                  "abs_start": 0, "aud_start": ft["aud_start"]})
    return pool


def spread_velocity(items, k):
    """Up to k items evenly over the rank of the peak level (the velocity layers)."""
    items = sorted(items, key=lambda i: i["peak_db"])
    if len(items) <= k:
        return items
    return [items[int(round(j * (len(items) - 1) / (k - 1)))] for j in range(k)]


def choose_acoustic(pool, per_shell=30):
    out = {"kick": spread_velocity(pool["kick"], per_shell), "snare": spread_velocity(pool["snare"], per_shell)}
    toms = sorted(pool["tom"], key=lambda i: -i["centroid_hz"])
    q = len(toms) // 4
    for k, sh in enumerate(["t1", "t2", "t3", "t4"]):
        grp = toms[k * q:(k + 1) * q] if k < 3 else toms[3 * q:]
        out[sh] = spread_velocity(grp, per_shell)
    return out


def acoustic_shot(e):
    sr = SR_REAL
    y = e["y"]                     # already cut by `cut_shot` in the pool
    ft = features(y, sr)
    return {"y": y, "lead": 0, "abs_off": float(ft["abs_start"]), "aud_off": ft["aud_start"], "peak_db": ft["peak_db"],
            "name": e["name"], "fp": fingerprint(y, sr, ft["aud_start"])}


def acoustic_specs(shell, entries, quick, seed):
    rng = np.random.default_rng(seed)

    def spec(e):
        shot = acoustic_shot(e)
        return {"shot": shot, "vel": velocity(shot["peak_db"], rng),
                "meta": {"sample": e["name"], "kit": e["kit"], "family": "acoustic", "gain_db": 0.0,
                         "rise_ms": e["rise_ms"], "centroid_hz": e["centroid_hz"], "decay_ms": e["decay_ms"],
                         "level_db": shot["peak_db"], "presilence_ms": e["presilence_ms"],
                         "gap_samples": e["gap_samples"]}}
    specs = [spec(e) for _ in range(1 if quick else 2) for e in entries]
    specs = [specs[i] for i in rng.permutation(len(specs))]
    pairs = [spec(entries[int(rng.integers(0, len(entries)))]) for _ in range(2 * (3 if quick else 8))]
    return specs, pairs


def apply_condition(mix, others, cond, sr, seed):
    """clean: the mix; noise: plus white noise at -60 dBFS RMS; bleed: plus another shell's clean mix at -20 dB."""
    if cond == "clean":
        return mix.copy()
    if cond == "noise":
        rng = np.random.default_rng(seed + 5)
        return mix + rng.standard_normal(len(mix)) * 10 ** (NOISE_DB / 20.0)
    out = mix.copy()
    b = others
    n = min(len(out), len(b))
    out[:n] += b[:n] * 10 ** (BLEED_DB / 20.0)
    return out


def write_wav(path, x, sr):
    peak = float(np.max(np.abs(x)))
    if peak > 0.999:
        x = x * (0.999 / peak)
    sf.write(str(path), x, sr, subtype="PCM_24")


def write_reference(out_dir, shell, sr, events, kind):
    """The reference MIDI (24,000 PPQ, 120 BPM, format 1) and the CSV of the exact starts (samples, float)."""
    mid = mido.MidiFile(type=1, ticks_per_beat=PPQ)
    tempo = mido.MidiTrack()
    tempo.append(mido.MetaMessage("set_tempo", tempo=500000, time=0))
    tempo.append(mido.MetaMessage("end_of_track", time=1))
    notes = mido.MidiTrack()
    msgs = []
    for e in events:
        tick = int(round(e["abs_s"] * TICKS_PER_S))
        msgs.append((tick, 1, mido.Message("note_on", channel=9, note=NOTE[shell], velocity=e["velocity"])))
        msgs.append((tick + int(0.05 * TICKS_PER_S), 0, mido.Message("note_off", channel=9, note=NOTE[shell], velocity=0)))
        e["tick"] = tick
    msgs.sort(key=lambda m: (m[0], m[1]))
    last = 0
    for tick, _, m in msgs:
        notes.append(m.copy(time=tick - last))
        last = tick
    notes.append(mido.MetaMessage("end_of_track", time=1))
    mid.tracks.extend([tempo, notes])
    mid.save(out_dir / f"{shell}_{sr}.mid")
    keys = sorted({k for e in events for k in e})
    with open(out_dir / f"{shell}_{sr}.csv", "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=keys)
        w.writeheader()
        for e in events:
            w.writerow(e)


def generate(out, quick=False, real_per_shell=24, log=print):
    """Audio, reference MIDI and CSV under `out/{audio,ref}/<set>/...`. Returns the list of audio files made."""
    out = Path(out)
    files = []
    syn_clean = {}
    for sr in [SR_MAIN] + ([] if quick else SR_EXTRA):
        for si, shell in enumerate(SHELLS):
            specs, pairs = syn_specs(shell, sr, quick, seed=101 + si)
            mix, ev = build_mix(shell, sr, specs, pairs, seed=900 + si, kind="syn")
            if sr == SR_MAIN:
                syn_clean[shell] = mix
            rd = out / "ref" / "syn" / "clean"
            rd.mkdir(parents=True, exist_ok=True)
            write_reference(rd, shell, sr, ev, "syn")
            (out / "audio" / "syn" / "clean").mkdir(parents=True, exist_ok=True)
            p = out / "audio" / "syn" / "clean" / f"{shell}_{sr}.wav"
            write_wav(p, mix, sr)
            files.append(p)
            log(f"syn clean {shell} {sr}: {len(ev)} events")
    for cond in CONDITIONS[1:]:
        for si, shell in enumerate(SHELLS):
            mix = syn_clean[shell]
            other = syn_clean[BLEED_FROM[shell]]
            x = apply_condition(mix, other, cond, SR_MAIN, seed=33 + si)
            d = out / "audio" / "syn" / cond
            d.mkdir(parents=True, exist_ok=True)
            p = d / f"{shell}_{SR_MAIN}.wav"
            write_wav(p, x, SR_MAIN)
            files.append(p)
            rd = out / "ref" / "syn" / cond
            rd.mkdir(parents=True, exist_ok=True)
            for ext in ("mid", "csv"):
                shutil.copy(out / "ref" / "syn" / "clean" / f"{shell}_{SR_MAIN}.{ext}", rd / f"{shell}_{SR_MAIN}.{ext}")
    # sweep: the fingerprint axes
    sweep_clean = {}
    for si, shell in enumerate(SHELLS):
        specs, pairs = sweep_specs(shell, SR_MAIN, quick, seed=501 + si)
        mix, ev = build_mix(shell, SR_MAIN, specs, pairs, seed=800 + si, kind="syn")
        sweep_clean[shell] = mix
        rd = out / "ref" / "sweep" / "clean"
        rd.mkdir(parents=True, exist_ok=True)
        write_reference(rd, shell, SR_MAIN, ev, "sweep")
        (out / "audio" / "sweep" / "clean").mkdir(parents=True, exist_ok=True)
        p = out / "audio" / "sweep" / "clean" / f"{shell}_{SR_MAIN}.wav"
        write_wav(p, mix, SR_MAIN)
        files.append(p)
        log(f"sweep clean {shell}: {len(ev)} events")
    for cond in CONDITIONS[1:]:
        for si, shell in enumerate(SHELLS):
            x = apply_condition(sweep_clean[shell], sweep_clean[BLEED_FROM[shell]], cond, SR_MAIN, seed=66 + si)
            d = out / "audio" / "sweep" / cond
            d.mkdir(parents=True, exist_ok=True)
            p = d / f"{shell}_{SR_MAIN}.wav"
            write_wav(p, x, SR_MAIN)
            files.append(p)
            rd = out / "ref" / "sweep" / cond
            rd.mkdir(parents=True, exist_ok=True)
            for ext in ("mid", "csv"):
                shutil.copy(out / "ref" / "sweep" / "clean" / f"{shell}_{SR_MAIN}.{ext}", rd / f"{shell}_{SR_MAIN}.{ext}")
    # real
    if LOGIC.is_dir():
        pool = real_pool()
        chosen = choose_real(pool, real_per_shell if not quick else 4)
        (out / "real_samples.json").write_text(json.dumps({s: [{k: v for k, v in e.items()} for e in es]
                                                           for s, es in chosen.items()}, indent=1))
        real_clean = {}
        for si, shell in enumerate(SHELLS):
            specs, pairs = real_specs(shell, chosen[shell], quick, seed=301 + si)
            mix, ev = build_mix(shell, SR_REAL, specs, pairs, seed=700 + si, kind="real")
            real_clean[shell] = mix
            rd = out / "ref" / "real" / "clean"
            rd.mkdir(parents=True, exist_ok=True)
            write_reference(rd, shell, SR_REAL, ev, "real")
            (out / "audio" / "real" / "clean").mkdir(parents=True, exist_ok=True)
            p = out / "audio" / "real" / "clean" / f"{shell}_{SR_REAL}.wav"
            write_wav(p, mix, SR_REAL)
            files.append(p)
            log(f"real clean {shell}: {len(ev)} events")
        for cond in CONDITIONS[1:]:
            for si, shell in enumerate(SHELLS):
                x = apply_condition(real_clean[shell], real_clean[BLEED_FROM[shell]], cond, SR_REAL, seed=55 + si)
                d = out / "audio" / "real" / cond
                d.mkdir(parents=True, exist_ok=True)
                p = d / f"{shell}_{SR_REAL}.wav"
                write_wav(p, x, SR_REAL)
                files.append(p)
                rd = out / "ref" / "real" / cond
                rd.mkdir(parents=True, exist_ok=True)
                for ext in ("mid", "csv"):
                    shutil.copy(out / "ref" / "real" / "clean" / f"{shell}_{SR_REAL}.{ext}", rd / f"{shell}_{SR_REAL}.{ext}")
    # acoustic: Drum Kit Designer slices
    if DKD.is_dir():
        chosen = choose_acoustic(acoustic_pool(), 4 if quick else 30)
        (out / "acoustic_samples.json").write_text(json.dumps(
            {sh: [{k: v for k, v in e.items() if k != "y"} for e in es] for sh, es in chosen.items()}, indent=1))
        clean = {}
        for si, shell in enumerate(SHELLS):
            specs, pairs = acoustic_specs(shell, chosen[shell], quick, seed=401 + si)
            mix, ev = build_mix(shell, SR_REAL, specs, pairs, seed=600 + si, kind="real")
            clean[shell] = mix
            rd = out / "ref" / "acoustic" / "clean"
            rd.mkdir(parents=True, exist_ok=True)
            write_reference(rd, shell, SR_REAL, ev, "acoustic")
            (out / "audio" / "acoustic" / "clean").mkdir(parents=True, exist_ok=True)
            p = out / "audio" / "acoustic" / "clean" / f"{shell}_{SR_REAL}.wav"
            write_wav(p, mix, SR_REAL)
            files.append(p)
            log(f"acoustic clean {shell}: {len(ev)} events from {len(chosen[shell])} slices")
        for cond in CONDITIONS[1:]:
            for si, shell in enumerate(SHELLS):
                x = apply_condition(clean[shell], clean[BLEED_FROM[shell]], cond, SR_REAL, seed=77 + si)
                d = out / "audio" / "acoustic" / cond
                d.mkdir(parents=True, exist_ok=True)
                p = d / f"{shell}_{SR_REAL}.wav"
                write_wav(p, x, SR_REAL)
                files.append(p)
                rd = out / "ref" / "acoustic" / cond
                rd.mkdir(parents=True, exist_ok=True)
                for ext in ("mid", "csv"):
                    shutil.copy(out / "ref" / "acoustic" / "clean" / f"{shell}_{SR_REAL}.{ext}", rd / f"{shell}_{SR_REAL}.{ext}")
    return files


# ---------------------------------------------------------------- detection through the agent CLI

def find_tool():
    env = os.environ.get("DRUMSPIKE_CALIB_TOOL")
    if env:
        return Path(env)
    target = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    return target / "release" / "service_tool"


def run_one(tool, out, audio, detector, model_path):
    """Detect one audio file with one detector: project_new, tracks_add, analyze_start (wait), events_get,
    export_midi, through `service_tool cli run`. Writes `res/<rel>/<name>__<detector>.json` and `.mid`."""
    rel = audio.relative_to(out / "audio")
    shell = audio.stem.split("_")[0]
    resdir = out / "res" / rel.parent
    resdir.mkdir(parents=True, exist_ok=True)
    stem = f"{audio.stem}__{detector}"
    home = out / "run-home" / rel.parent / stem
    projects = out / "run-projects" / rel.parent / stem
    for d in (home, projects):
        if d.exists():
            shutil.rmtree(d)
        d.mkdir(parents=True)
    name = audio.stem
    steps = [
        {"command": "project_new", "args": {"name": stem, "dir": str(projects)}},
        {"command": "tracks_add", "args": {"sid": "$sid", "paths": [str(audio)], "roles": {str(audio): ROLE[shell]},
                                           "notes": {str(audio): NOTE[shell]}}, "wait": True},
        {"command": "analyze_start", "args": {"sid": "$sid"}, "wait": True},
        {"command": "events_get", "args": {"sid": "$sid", "name": name}},
        {"command": "export_midi", "args": {"sid": "$sid", "name": name, "note": NOTE[shell]}},
    ]
    script = home / "steps.json"
    script.write_text(json.dumps(steps))
    env = {**os.environ, "DRUMSPIKE_HOME": str(home / "h"), "DRUMSPIKE_PROJECTS": str(projects)}
    r = subprocess.run([str(tool), "cli", "run", str(script)], capture_output=True, text=True, env=env)
    rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
    bad = [x for x in rows if not x.get("ok", False)]
    if r.returncode != 0 or bad or len(rows) != len(steps):
        raise RuntimeError(f"{audio.name} {detector}: exit {r.returncode} {bad[:1]} {r.stderr[-400:]}")
    events = rows[3]["result"]
    midi = Path(rows[4]["result"]["path"])
    shutil.copy(midi, resdir / f"{stem}.mid")
    analysis = rows[2]["result"]
    (resdir / f"{stem}.json").write_text(json.dumps({"events": events, "analyze": analysis, "audio": str(audio),
                                                      "detector": detector}))
    shutil.rmtree(home, ignore_errors=True)
    shutil.rmtree(projects, ignore_errors=True)
    return resdir / f"{stem}.json"


def run_all(out, jobs=2, detectors=DETECTORS, only=None, log=print):
    out = Path(out)
    tool = find_tool()
    if not tool.is_file():
        raise SystemExit(f"no service_tool at {tool}: cargo build --release -p drumspike-devtools --bin service_tool")
    audio = sorted((out / "audio").glob("*/*/*.wav"))
    if only:
        audio = [a for a in audio if re.search(only, str(a.relative_to(out / "audio")))]
    work = [(a, d) for a in audio for d in detectors]
    done = []
    with ThreadPoolExecutor(max_workers=jobs) as ex:
        futs = [(a, d, ex.submit(run_one, tool, out, a, d, "none")) for a, d in work]
        for a, d, f in futs:
            f.result()
            done.append(a)
            log(f"ran {a.relative_to(out / 'audio')} {d}")
    return len(work)


# ---------------------------------------------------------------- compare

STR_COLS = {"shell", "group", "sample", "kit", "family", "axis"}


def read_csv(path):
    with open(path) as fh:
        rows = list(csv.DictReader(fh))
    for r in rows:
        for k, v in list(r.items()):
            if k in STR_COLS:
                continue
            try:
                r[k] = float(v)
            except ValueError:
                r[k] = float("nan")
    return rows


def match(ref_s, det_s, tol=MATCH_S):
    """One to one, nearest pair first, within `tol` seconds. Returns (pairs [(ri, di)], missed [ri], false [di])."""
    cand = []
    for i, r in enumerate(ref_s):
        for j, d in enumerate(det_s):
            dt = abs(d - r)
            if dt <= tol:
                cand.append((dt, i, j))
    cand.sort()
    ur, ud, pairs = set(), set(), []
    for _, i, j in cand:
        if i in ur or j in ud:
            continue
        ur.add(i)
        ud.add(j)
        pairs.append((i, j))
    pairs.sort()
    return pairs, [i for i in range(len(ref_s)) if i not in ur], [j for j in range(len(det_s)) if j not in ud]


def midi_hits_s(path, note=None):
    """Note-on times in seconds of a MIDI file, through its own tempo events (the last tempo wins here: a calibration
    file has one)."""
    mid = mido.MidiFile(str(path))
    tempo = 500000
    t = 0.0
    out = []
    for tr in mid.tracks:
        for m in tr:
            if m.type == "set_tempo":
                tempo = m.tempo
    for tr in mid.tracks:
        tick = 0
        for m in tr:
            tick += m.time
            if m.type == "note_on" and m.velocity > 0 and (note is None or m.note == note):
                out.append(tick * tempo / 1e6 / mid.ticks_per_beat)
    return sorted(out)


def stats(errs):
    a = np.asarray(errs, dtype=float)
    if len(a) == 0:
        return {"n": 0}
    return {"n": int(len(a)), "mean": float(a.mean()), "median": float(np.median(a)),
            "sd": float(a.std(ddof=0)), "p5": float(np.percentile(a, 5)), "p95": float(np.percentile(a, 95)),
            "min": float(a.min()), "max": float(a.max())}


def compare_pair(ref_rows, det_events_s, sr, midi_s=None):
    """Per reference row: the matched detection and its error in samples (events and MIDI), plus the counts."""
    ref_s = [r["abs_s"] for r in ref_rows]
    pairs, missed, false = match(ref_s, det_events_s)
    rows = []
    mp = {}
    if midi_s is not None:
        mp_pairs, _, _ = match(ref_s, midi_s)
        mp = {i: j for i, j in mp_pairs}
    dmap = dict(pairs)
    for i, r in enumerate(ref_rows):
        row = dict(r)
        if i in dmap:
            d = det_events_s[dmap[i]]
            row["found"] = True
            row["err_abs"] = (d - r["abs_s"]) * sr
            if "aud_sample" in r:
                row["err_aud"] = d * sr - r["aud_sample"]
            if i in mp:
                row["err_midi_abs"] = (midi_s[mp[i]] - r["abs_s"]) * sr
                row["midi_minus_events"] = (midi_s[mp[i]] - d) * sr
        else:
            row["found"] = False
        rows.append(row)
    # a missed hit with an unmatched detection 10 to 50 ms away: reported apart (a far-off detection)
    far = 0
    free = set(false)
    for i in missed:
        near = [j for j in free if abs(det_events_s[j] - ref_s[i]) <= FAR_S]
        if near:
            j = min(near, key=lambda j: abs(det_events_s[j] - ref_s[i]))
            rows[i]["err_far"] = (det_events_s[j] - ref_s[i]) * sr
            free.discard(j)
            far += 1
    return rows, {"found": len(pairs), "missed": len(missed), "false": len(false), "far": far,
                  "n_ref": len(ref_rows), "n_det": len(det_events_s)}


def compare_all(out, log=print):
    """Walk `res/`, compare each result with its reference. Returns {"runs": [...], "rows": [...]}."""
    out = Path(out)
    runs, allrows = [], []
    for jf in sorted((out / "res").glob("*/*/*__*.json")):
        kind, cond = jf.parent.parent.name, jf.parent.name
        stem, detector = jf.stem.rsplit("__", 1)
        shell, sr = stem.split("_")[0], int(stem.split("_")[1])
        refs = read_csv(out / "ref" / kind / cond / f"{shell}_{sr}.csv")
        res = json.loads(jf.read_text())
        det_s = sorted(e["t"] for e in res["events"]["events"])
        midi_path = jf.with_suffix(".mid")
        midi_s = midi_hits_s(midi_path) if midi_path.is_file() else None
        rows, counts = compare_pair(refs, det_s, sr, midi_s)
        for r in rows:
            r.update(kind=kind, cond=cond, shell=shell, sr=sr, detector=detector)
        runs.append({"kind": kind, "cond": cond, "shell": shell, "sr": sr, "detector": detector, **counts,
                     "n_det_midi": len(midi_s) if midi_s is not None else None})
        allrows.extend(rows)
    return {"runs": runs, "rows": allrows}


# ---------------------------------------------------------------- tables

def fmt(s):
    if s.get("n", 0) == 0:
        return "-"
    return f"{s['mean']:+.2f} / {s['median']:+.2f} / {s['sd']:.2f} / {s['p5']:+.1f} / {s['p95']:+.1f} / {s['min']:+.1f} / {s['max']:+.1f}"


def _err(rows, key):
    return [r[key] for r in rows if r.get("found") and key in r]


def tables(res):
    """Markdown tables of the comparison. Errors are D minus R in samples: positive = late."""
    rows = [r for r in res["rows"] if r["group"] == "main"]
    runs = res["runs"]
    out = []
    head = "| shell | cond | det | found | missed (far) | false | mean / median / sd / p5 / p95 / min / max |\n|---|---|---|---|---|---|---|\n"

    def block(title, kind, key, sr=None):
        sub_runs = [r for r in runs if r["kind"] == kind and (sr is None or r["sr"] == sr)]
        s = f"### {title}\n\n{head}"
        for sh in SHELLS:
            for cond in CONDITIONS:
                for det in DETECTORS:
                    rr = [r for r in sub_runs if r["shell"] == sh and r["cond"] == cond and r["detector"] == det]
                    if not rr:
                        continue
                    sel = [r for r in rows if r["kind"] == kind and r["shell"] == sh and r["cond"] == cond
                           and r["detector"] == det and (sr is None or r["sr"] == sr)]
                    n_main = len(sel)
                    found = sum(1 for r in sel if r["found"])
                    false = rr[0]["false"]
                    s += f"| {sh} | {cond} | {det} | {found}/{n_main} | {n_main - found} ({sum(1 for r in sel if 'err_far' in r)}) | {false} | {fmt(stats(_err(sel, key)))} |\n"
        return s + "\n"

    out.append(block("Synthetic, 48 kHz, error against the first non-zero sample (`abs`)", "syn", "err_abs", SR_MAIN))
    out.append(block("Synthetic, 48 kHz, error against the audible start (first sample within 40 dB of the peak)", "syn",
                     "err_aud", SR_MAIN))
    out.append(block("Real one-shots, 44.1 kHz, error against `abs` (first sample above -90 dBFS)", "real", "err_abs"))
    out.append(block("Real one-shots, 44.1 kHz, error against `aud` (first sample within 40 dB of the peak)", "real",
                     "err_aud"))
    out.append(block("Acoustic slices (Drum Kit Designer), 44.1 kHz, error against `abs` (first sample above -90 dBFS)",
                     "acoustic", "err_abs"))
    out.append(block("Acoustic slices, 44.1 kHz, error against `aud` (first sample within 40 dB of the peak)", "acoustic",
                     "err_aud"))

    vhead = "| shell | det | {k} | found | mean / median / sd / p5 / p95 / min / max |\n|---|---|---|---|---|\n"

    def by(title, kind, var, key, cond="clean", sr=SR_MAIN, values=None):
        s = f"### {title}\n\n" + vhead.format(k=var)
        for sh in SHELLS:
            for det in DETECTORS:
                sel0 = [r for r in rows if r["kind"] == kind and r["shell"] == sh and r["cond"] == cond
                        and r["detector"] == det and r["sr"] == sr]
                vals = values if values is not None else sorted({r[var] for r in sel0})
                for v in vals:
                    sel = [r for r in sel0 if r[var] == v]
                    if not sel:
                        continue
                    f = sum(1 for r in sel if r["found"])
                    s += f"| {sh} | {det} | {v:g} | {f}/{len(sel)} | {fmt(stats(_err(sel, key)))} |\n"
        return s + "\n"

    for var, label in (("rise_ms", "attack rise (ms)"), ("level_db", "level (dBFS)"), ("pol", "polarity"),
                       ("frac", "fractional start (sample)")):
        out.append(by(f"Synthetic, clean, 48 kHz, by {label}, against `abs`", "syn", var, "err_abs"))
    for var, label in (("rise_ms", "attack rise (ms)"), ("level_db", "level (dBFS)")):
        out.append(by(f"Synthetic, noise floor -60 dBFS, by {label}, against `abs`", "syn", var, "err_abs", cond="noise"))
        out.append(by(f"Synthetic, bleed -20 dB, by {label}, against `abs`", "syn", var, "err_abs", cond="bleed"))

    # sample rates
    s = "### Synthetic, clean, by sample rate, against `abs`\n\n| shell | det | sr | found | mean / median / sd / p5 / p95 / min / max |\n|---|---|---|---|---|\n"
    for sh in SHELLS:
        for det in DETECTORS:
            for sr in [SR_MAIN] + SR_EXTRA:
                sel = [r for r in rows if r["kind"] == "syn" and r["shell"] == sh and r["cond"] == "clean"
                       and r["detector"] == det and r["sr"] == sr]
                if sel:
                    s += f"| {sh} | {det} | {sr} | {sum(1 for r in sel if r['found'])}/{len(sel)} | {fmt(stats(_err(sel, 'err_abs')))} |\n"
    out.append(s + "\n")

    # close pairs
    prow = [r for r in res["rows"] if r["group"] == "pair" and r["cond"] == "clean" and r["sr"] in (SR_MAIN, SR_REAL)]
    s = "### Close pairs (20 to 60 ms), clean\n\n| set | shell | det | found | mean / median / sd / p5 / p95 / min / max (abs) |\n|---|---|---|---|---|\n"
    for kind in ("syn", "real", "acoustic"):
        for sh in SHELLS:
            for det in DETECTORS:
                sel = [r for r in prow if r["kind"] == kind and r["shell"] == sh and r["detector"] == det]
                if sel:
                    s += f"| {kind} | {sh} | {det} | {sum(1 for r in sel if r['found'])}/{len(sel)} | {fmt(stats(_err(sel, 'err_abs')))} |\n"
    out.append(s + "\n")

    # MIDI D against the events
    s = ("### MIDI D against the events (export exactness), matched hits, samples\n\n"
         "| set | det | n | mean | max abs |\n|---|---|---|---|---|\n")
    for kind in ("syn", "real", "acoustic"):
        for det in DETECTORS:
            d = [r["midi_minus_events"] for r in res["rows"] if r["kind"] == kind and r["detector"] == det
                 and "midi_minus_events" in r]
            if d:
                s += f"| {kind} | {det} | {len(d)} | {np.mean(d):+.3f} | {np.max(np.abs(d)):.3f} |\n"
    out.append(s + "\n")

    # drivers: real, bias against features, clean
    s = ("### What drives the bias (real and acoustic one-shots, clean, against `abs`): mean error by tercile of a feature\n\n"
         "| set | det | feature | low third | mid third | high third | Pearson r |\n|---|---|---|---|---|---|\n")
    for kind, det in [(k, d) for k in ("real", "acoustic") for d in DETECTORS]:
        sel = [r for r in rows if r["kind"] == kind and r["cond"] == "clean" and r["detector"] == det
               and r["found"] and "err_abs" in r]
        for feat in ("rise_ms", "centroid_hz", "level_db", "decay_ms", "presilence_ms"):
            if len(sel) < 6:
                continue
            xs = np.array([r[feat] for r in sel])
            ys = np.array([r["err_abs"] for r in sel])
            lo, hi = np.percentile(xs, [33.3, 66.7])
            parts = []
            for m in (xs <= lo, (xs > lo) & (xs <= hi), xs > hi):
                parts.append(f"{ys[m].mean():+.2f} (n={int(m.sum())}, {xs[m].min():g}..{xs[m].max():g})" if m.any() else "-")
            rr = float(np.corrcoef(xs, ys)[0, 1]) if np.std(xs) > 0 and np.std(ys) > 0 else float("nan")
            s += f"| {kind} | {det} | {feat} | {parts[0]} | {parts[1]} | {parts[2]} | {rr:+.2f} |\n"
    out.append(s + "\n")
    s = ("### What drives the bias (synthetic, clean, 48 kHz, against `abs`): Pearson r with each variant\n\n"
         "| shell | det | rise r | level r | n |\n|---|---|---|---|---|\n")
    for sh in SHELLS:
        for det in DETECTORS:
            sel = [r for r in rows if r["kind"] == "syn" and r["cond"] == "clean" and r["sr"] == SR_MAIN
                   and r["shell"] == sh and r["detector"] == det and r["found"]]
            if len(sel) > 5:
                ys = np.array([r["err_abs"] for r in sel])
                rr = [float(np.corrcoef(np.array([r[f] for r in sel]), ys)[0, 1]) for f in ("rise_ms", "level_db")]
                s += f"| {sh} | {det} | {rr[0]:+.2f} | {rr[1]:+.2f} | {len(sel)} |\n"
    out.append(s + "\n")
    return "\n".join(out)



FEATURES = ["e_low", "e_mid", "e_high", "hi_mid_db", "low_mid_db", "centroid10_hz", "rolloff85_hz", "attack_db",
            "rise1090_ms", "f0_hz", "decay_ms", "level_db"]
SHELL_CLASS = {"kick": "kick", "snare": "snare", "t1": "toms", "t2": "toms", "t3": "toms", "t4": "toms"}


def _rate(sel):
    return f"{100.0 * sum(1 for r in sel if r['found']) / len(sel):.0f} % ({sum(1 for r in sel if r['found'])}/{len(sel)})" if sel else "-"


def _ms(sel, key):
    e = _err(sel, key)
    return f"{np.mean(e):+.2f} / {np.std(e):.2f}" if e else "-"


def _terciles(sel, feat):
    xs = np.array([r[feat] for r in sel])
    lo, hi = np.percentile(xs, [33.3, 66.7])
    return [[r for r, x in zip(sel, xs) if x <= lo], [r for r, x in zip(sel, xs) if lo < x <= hi],
            [r for r, x in zip(sel, xs) if x > hi]], (lo, hi)


def _corr(xs, ys):
    xs, ys = np.asarray(xs, float), np.asarray(ys, float)
    if len(xs) < 6 or np.std(xs) == 0 or np.std(ys) == 0:
        return float("nan")
    return float(np.corrcoef(xs, ys)[0, 1])


def fingerprint_tables(res):
    """The owner's question (2026-10-13): does the spectral fingerprint change the detection rate and the onset bias?"""
    out = []
    rows = [r for r in res["rows"] if r["group"] == "main" and r["cond"] == "clean"
            and ((r["kind"] in ("real", "acoustic")) or r["sr"] == SR_MAIN)]
    # 1. sweep axes
    s = ("### Sweeps (synthetic, clean, -6 dBFS, 48 kHz): detection rate and error by axis value\n\n"
         "Error = D minus R in samples against the first non-zero sample (`abs`) and the audible start (`aud`); mean / sd.\n\n"
         "| shell | det | axis | value | found | abs mean / sd | aud mean / sd |\n|---|---|---|---|---|---|---|\n")
    for sh in SHELLS:
        for det in DETECTORS:
            for axis in ("tune", "click", "band"):
                vals = sorted({r["axis_value"] for r in rows if r["kind"] == "sweep" and r["shell"] == sh and r["axis"] == axis})
                for v in vals:
                    sel = [r for r in rows if r["kind"] == "sweep" and r["shell"] == sh and r["detector"] == det
                           and r["axis"] == axis and r["axis_value"] == v]
                    s += f"| {sh} | {det} | {axis} | {v:g} | {_rate(sel)} | {_ms(sel, 'err_abs')} | {_ms(sel, 'err_aud')} |\n"
    out.append(s + "\n")
    # 2. feature terciles per set, shell class and detector
    for kind, title in (("sweep", "synthetic sweeps"), ("real", "Ultrabeat one-shots"), ("acoustic", "acoustic slices"), ("syn", "synthetic kit variants")):
        s = (f"### Detection rate and bias by feature tercile ({title}, clean)\n\n"
             "Bias = mean / sd of the error against `abs` in samples; `r` is the Pearson correlation of the error (found "
             "hits) and of the found flag with the feature.\n\n"
             "| class | det | feature | low third | mid third | high third | r (error) | r (found) |\n|---|---|---|---|---|---|---|---|\n")
        for cls in ("kick", "snare", "toms"):
            for det in DETECTORS:
                sel = [r for r in rows if r["kind"] == kind and SHELL_CLASS[r["shell"]] == cls and r["detector"] == det]
                for feat in FEATURES:
                    ok = [r for r in sel if feat in r and not math.isnan(r[feat])]
                    if len(ok) < 12 or len({r[feat] for r in ok}) < 3:
                        continue
                    parts, (lo, hi) = _terciles(ok, feat)
                    cells = [f"{_rate(p)}; {_ms(p, 'err_abs')}" for p in parts]
                    fo = [r for r in ok if r["found"] and "err_abs" in r]
                    rr = _corr([r[feat] for r in fo], [r["err_abs"] for r in fo])
                    rf = _corr([r[feat] for r in ok], [1.0 if r["found"] else 0.0 for r in ok])
                    s += f"| {cls} | {det} | {feat} ({lo:.3g}, {hi:.3g}) | {cells[0]} | {cells[1]} | {cells[2]} | {rr:+.2f} | {rf:+.2f} |\n"
        out.append(s + "\n")
    # 3. ranking of drivers
    s = ("### Features that drive the bias most (|Pearson r| of the error with the feature, found hits, clean, against `abs`)\n\n"
         "| set | class | det | n | top three features (r) |\n|---|---|---|---|---|\n")
    for kind in ("sweep", "real", "acoustic", "syn"):
        for cls in ("kick", "snare", "toms"):
            for det in DETECTORS:
                fo = [r for r in rows if r["kind"] == kind and SHELL_CLASS[r["shell"]] == cls and r["detector"] == det
                      and r["found"] and "err_abs" in r]
                rs = []
                for feat in FEATURES + ["rise_ms"]:
                    ok = [r for r in fo if feat in r and not math.isnan(r[feat])]
                    if len(ok) >= 12:
                        rs.append((abs(_corr([r[feat] for r in ok], [r["err_abs"] for r in ok])), feat,
                                   _corr([r[feat] for r in ok], [r["err_abs"] for r in ok])))
                rs = [x for x in rs if not math.isnan(x[0])]
                rs.sort(reverse=True)
                s += f"| {kind} | {cls} | {det} | {len(fo)} | " + ", ".join(f"{f} ({r:+.2f})" for _, f, r in rs[:3]) + " |\n"
    out.append(s + "\n")
    # 4. the three hypotheses, snare
    s = ("### The owner's three hypotheses (snare, clean)\n\n"
         "Real snares: only the hits at the native level (gain 0). H1 highs are detected more reliably than mids; H2 highs are detected more exactly (smaller error spread); "
         "H3 a weak attack is late. Rate = found share; error mean / sd in samples against `abs`.\n\n"
         "| test | det | group A | group B |\n|---|---|---|---|\n")

    def two(label, kind, det, fa, fb, na, nb):
        base = [r for r in rows if r["kind"] == kind and r["shell"] == "snare" and r["detector"] == det
                and (kind not in ("real", "acoustic") or r["gain_db"] == 0.0)]
        sa = [r for r in base if fa(r)]
        sb = [r for r in base if fb(r)]
        return (f"| {label} | {det} | {na}: {_rate(sa)}; {_ms(sa, 'err_abs')} | {nb}: {_rate(sb)}; {_ms(sb, 'err_abs')} |\n")

    for det in DETECTORS:
        s += two("sweep band: mids against highs (H1, H2)", "sweep", det,
                 lambda r: r["axis"] == "band" and r["axis_value"] == 1.0, lambda r: r["axis"] == "band" and r["axis_value"] == 0.0,
                 "highs (band 1)", "mids (band 0)")
        s += two("sweep click: none against strong (H3)", "sweep", det,
                 lambda r: r["axis"] == "click" and r["axis_value"] == 0.0, lambda r: r["axis"] == "click" and r["axis_value"] == 2.0,
                 "no click", "click 2.0")
        for kind, label in (("real", "Ultrabeat snares"), ("acoustic", "acoustic snares")):
            real = [r for r in rows if r["kind"] == kind and r["shell"] == "snare" and r["detector"] == det
                    and r["gain_db"] == 0.0 and not math.isnan(r["hi_mid_db"])]
            if len(real) < 12:
                continue
            lo, hi = np.percentile([r["hi_mid_db"] for r in real], [33.3, 66.7])
            s += two(f"{label} by hi_mid_db tercile (H1, H2)", kind, det, lambda r, h=hi: r["hi_mid_db"] > h,
                     lambda r, l=lo: r["hi_mid_db"] <= l, f"most highs (> {hi:.1f} dB)", f"most mids (<= {lo:.1f} dB)")
            lo, hi = np.percentile([r["attack_db"] for r in real], [33.3, 66.7])
            s += two(f"{label} by attack_db tercile (H3)", kind, det, lambda r, l=lo: r["attack_db"] <= l,
                     lambda r, h=hi: r["attack_db"] > h, f"weak attack (<= {lo:.1f} dB)", f"strong attack (> {hi:.1f} dB)")
        s += two("synthetic rise 5 ms against 0 (H3)", "syn", det, lambda r: r["rise_ms"] == 5.0, lambda r: r["rise_ms"] == 0.0,
                 "rise 5 ms", "rise 0")
    out.append(s + "\n")
    return "\n".join(out)



def rise_level_matrix(res, cond):
    """Mean error (samples, against `abs`) for each attack rise and level, per shell and detector: the table that
    shows whether the bias is steady (found/n in brackets)."""
    rows = [r for r in res["rows"] if r["kind"] == "syn" and r["group"] == "main" and r["cond"] == cond and r["sr"] == SR_MAIN]
    cols = [(rs, lv) for rs in RISES_MS for lv in LEVELS_DB]
    s = (f"### Synthetic, {cond}, 48 kHz: mean error (samples, `abs`) by attack rise (ms) and level (dBFS) [found/n]\n\n"
         "| shell | det | " + " | ".join(f"r{rs:g} / {lv:g}" for rs, lv in cols) + " |\n|---|---|" + "---|" * len(cols) + "\n")
    for sh in SHELLS:
        for det in DETECTORS:
            cells = []
            for rs, lv in cols:
                sel = [r for r in rows if r["shell"] == sh and r["detector"] == det and r["rise_ms"] == rs and r["level_db"] == lv]
                e = _err(sel, "err_abs")
                cells.append(f"{np.mean(e):+.1f} [{len(e)}/{len(sel)}]" if e else f"- [0/{len(sel)}]")
            s += f"| {sh} | {det} | " + " | ".join(cells) + " |\n"
    return s + "\n"



def backtrack(x, d, sr, k_db, win_ms=0.5, back_ms=8.0, ahead_ms=3.0):
    """What-if (no detector change): from a detected sample `d`, the foot of the attack: the first sample, going
    back, whose forward `win_ms` envelope is below `k_db` under the peak of the next `ahead_ms`. Returns the new
    position in samples (float) or `d` when the signal does not fall that low within `back_ms`."""
    w = max(1, int(win_ms * 1e-3 * sr))
    i0 = int(d)
    pk = np.max(np.abs(x[i0:i0 + int(ahead_ms * 1e-3 * sr)])) if i0 < len(x) else 0.0
    if pk <= 0:
        return float(d)
    thr = pk * 10 ** (-k_db / 20.0)
    lo = max(0, i0 - int(back_ms * 1e-3 * sr))
    seg = np.abs(x[lo:i0 + w])
    env = np.array([seg[j:j + w].max() for j in range(i0 - lo + 1)])
    below = np.flatnonzero(env < thr)
    return float(lo + below[-1] + w) if len(below) else float(d)


def whatif_table(out, res, cond="clean"):
    """Per set and class: the error now, after a constant offset of the class's median (the best a global or per-role
    offset can do), and after a back-track to the foot of the attack at -20, -30 and -40 dB. Model detector, clean,
    main hits, against `aud`: median / sd / p5 / p95 in samples."""
    out = Path(out)
    rows = [r for r in res["rows"] if r["group"] == "main" and r["cond"] == cond and r["detector"] == "model"
            and r.get("found") and "err_aud" in r and (r["kind"] in ("real", "acoustic") or (r["kind"] == "syn" and r["sr"] == SR_MAIN))]
    cache = {}
    s = (f"### What-if ({cond} condition, model detector, against `aud`, samples): a constant offset against a back-track to the foot\n\n"
         "Back-track: from the detected sample, go back to the last sample whose 0.5 ms envelope is below k dB under the "
         "next 3 ms peak. No detector change; the numbers say what a refinement of that kind would do.\n\n"
         "| set | class | n | now: median / sd / p5 / p95 | median removed: sd / p5 / p95 | back-track 20 dB | 30 dB | 40 dB |\n"
         "|---|---|---|---|---|---|---|---|\n")

    def cell(v):
        v = np.asarray(v, float)
        return f"{np.median(v):+.1f} / {v.std():.1f} / {np.percentile(v, 5):+.1f} / {np.percentile(v, 95):+.1f}"
    for kind in ("syn", "real", "acoustic"):
        for cls in ("kick", "snare", "toms"):
            sel = [r for r in rows if r["kind"] == kind and SHELL_CLASS[r["shell"]] == cls
                   and (kind != "syn" or (r["level_db"] == -6.0 and r["rise_ms"] in (0.5, 2.0)))]
            if len(sel) < 6:
                continue
            now = [r["err_aud"] for r in sel]
            med = np.median(now)
            cols = []
            for k in (20, 30, 40):
                v = []
                for r in sel:
                    key = (r["kind"], r["shell"], r["sr"])
                    if key not in cache:
                        cache[key] = sf.read(str(out / "audio" / r["kind"] / cond / f"{r['shell']}_{r['sr']}.wav"))[0]
                    x = cache[key]
                    dpos = r["aud_sample"] + r["err_aud"]
                    v.append(backtrack(x, dpos, r["sr"], k) - r["aud_sample"])
                cols.append(cell(v))
            dev = np.asarray(now) - med
            s += (f"| {kind}{' (rise 0.5, 2 ms, -6 dBFS)' if kind == 'syn' else ''} | {cls} | {len(sel)} | {cell(now)} | "
                  f"{dev.std():.1f} / {np.percentile(dev, 5):+.1f} / {np.percentile(dev, 95):+.1f} | " + " | ".join(cols) + " |\n")
    return s + "\n"


def compare(out, log=print):
    out = Path(out)
    res = compare_all(out, log)
    (out / "compare.json").write_text(json.dumps(res["runs"], indent=1))
    with open(out / "compare_rows.csv", "w", newline="") as fh:
        keys = sorted({k for r in res["rows"] for k in r})
        w = csv.DictWriter(fh, fieldnames=keys)
        w.writeheader()
        w.writerows(res["rows"])
    (out / "tables.md").write_text(tables(res) + "\n" + "".join(rise_level_matrix(res, c) for c in CONDITIONS)
                                      + "\n" + fingerprint_tables(res) + "\n"
                                      + "".join(whatif_table(out, res, c) for c in CONDITIONS))
    log(f"{len(res['runs'])} runs compared; tables in {out / 'tables.md'}")
    return res


# ---------------------------------------------------------------- command line

def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["generate", "run", "compare", "all"])
    ap.add_argument("--out", default=str(DEFAULT_OUT))
    ap.add_argument("--quick", action="store_true", help="a small kit (tests and trial runs)")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--only", help="regex on the audio path (set/cond/file) for run")
    ap.add_argument("--detectors", default="dsp")
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    dets = [d for d in a.detectors.split(",") if d]
    if a.cmd in ("generate", "all"):
        generate(out, quick=a.quick)
    if a.cmd in ("run", "all"):
        run_all(out, jobs=a.jobs, detectors=dets, only=a.only)
    if a.cmd in ("compare", "all"):
        compare(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
