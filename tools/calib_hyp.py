"""P4-11b: the hypothesis sweep. Eighteen hypotheses about what moves the onset of Drum Spike, each with a
prediction, tested with the P4-11 kit (`tools/calib_kit.py`).

    uv run python tools/calib_hyp.py all                 # generate, run, verdicts, report tables (one command)
    uv run python tools/calib_hyp.py generate [--quick]
    uv run python tools/calib_hyp.py run [--jobs 3]
    uv run python tools/calib_hyp.py verdicts            # writes hyp/verdicts.md and hyp/verdicts.json

Output under ``~/.cache/drumspike/calib-kit/hyp/`` (audio, references, results, verdicts). Nothing goes into git.

M1 to M3 are about the refinement (`crates/core/src/refine.rs`) and run `parity refine` directly on controlled coarse
times (no detector), with the app's refinement rule (P4-12 stage 2: version 3, the reach rule and the first-motion
back-track; `DRUMSPIKE_REFINE_RULE=1` runs the engine's rule of P4-11b, `2` the reach rule of P4-12 stage 1, and pins the
app's analyses to it as well). S1 to S4 (hit shape), C1 to C5 (context before the hit) and R1 to R6 (recording chain) are
audio cases run through the agent CLI with the DSP detector at the defaults of the role, like the
kit. Error = detection minus the first motion (the exact start for synthetic hits), in samples; positive = late.
A verdict is `held`, `did not hold` or `unclear` against the prediction written in `HYPOTHESES`.
"""
import argparse
import csv
import json
import math
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
import soundfile as sf

sys.path.insert(0, str(Path(__file__).resolve().parent))
import calib_kit as ck  # noqa: E402

ck.STR_COLS |= {"tag", "variant"}
ROOT = ck.ROOT
HYP = ck.DEFAULT_OUT / "hyp"
SR = 48000
# P4-12: the refinement rule of `parity refine` (the app reads the same variable for its analyses; unset: the current rule 3)
RULE = int(os.environ.get("DRUMSPIKE_REFINE_RULE") or 3)

HYPOTHESES = {
    "M1": "the 25 % rule puts the onset about 12 dB under the local peak on every rise shape",
    "M2": "the walk-back stops early when the attack rises for more than about 1.5 ms: the error grows with the rise time",
    "M3": "the error and the miss rate depend on where the attack falls in the 10 ms model frame; attacks more than 2 ms before the frame time are out of reach",
    "S1": "the error grows with the period of the fundamental (40 to 120 Hz)",
    "S2": "an inverted hit (first half cycle negative) is as exact as a positive one",
    "S3": "a clipped hit (flat top, fast attack) is detected earlier than its clean form",
    "S4": "a dark hit (low-pass 2 to 4 kHz) is later than its bright form",
    "C1": "the tail of the previous hit (rolls at 120 to 220 BPM sixteenths) makes the next onset later",
    "C2": "bleed before the hit (hat into the snare mic, -40 to -10 dB) moves the onset; there is a level where it starts, and one where false hits start",
    "C3": "a flam (grace 10 to 40 ms before the main hit): the main hit stays exact and the grace is found from a spacing on",
    "C4": "ghost notes 20 to 120 ms after a loud hit are masked: a masking curve exists",
    "C5": "sympathetic tom ringing excited by the kick gives false tom hits above a ring level",
    "R1": "a DC offset or a subsonic rumble (5 to 20 Hz) moves the onset",
    "R2": "mains hum (50 or 60 Hz with harmonics) at -50 to -30 dBFS gives false hits or moves the onset",
    "R3": "a gated track gives the gate opening, not the first motion",
    "R4": "the error is constant in milliseconds, not in samples, across 44.1, 48, 88.2 and 96 kHz",
    "R5": "two summed mics with 0.5 to 3 ms delay: the onset follows the earlier mic",
    "R6": "a hit in the first 50 ms or the last 50 ms of the file is found and exact",
}


# ---------------------------------------------------------------- building blocks

def mk(shell, sr, seed, rise=0.0, level=-6.0, pol=1, **kw):
    """A synthetic hit of the kit (`calib_kit.make_shot`) at frac 0. The default rise is 0 (a sharp onset: the error of
    the plain hit is 0 to 1 sample, so a shift is the effect under test); the sweeps that need a rise name it."""
    return ck.make_shot(shell, sr, rise, level, pol, 0.0, seed, **kw)


def shot_from(y, sr, name=""):
    """A shot dict from raw samples: the first motion is the first non-zero sample, `aud` the first sample within
    40 dB of the peak (the kit's references)."""
    a = np.abs(y)
    abs_off = int(np.argmax(a > 0.0))
    aud = int(np.argmax(a >= a.max() * 0.01))
    return {"y": y, "lead": 0, "abs_off": float(abs_off), "aud_off": aud, "peak_db": 20 * math.log10(a.max() + 1e-12)}


def lowpass(y, sr, fc):
    """Causal 2nd-order Butterworth low-pass (bilinear transform), so the first motion stays where it was."""
    k = math.tan(math.pi * fc / sr)
    q = 1.0 / math.sqrt(2.0)
    n = 1.0 / (1.0 + k / q + k * k)
    b0, b1, b2 = k * k * n, 2 * k * k * n, k * k * n
    a1, a2 = 2 * (k * k - 1) * n, (1 - k / q + k * k) * n
    out = np.zeros(len(y))
    x1 = x2 = y1 = y2 = 0.0
    for i, v in enumerate(y.tolist()):
        o = b0 * v + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2
        out[i] = o
        x2, x1, y2, y1 = x1, v, y1, o
    return out


class Mix:
    """A track of one shell: hits placed at integer samples; every placement is an event with its references."""

    def __init__(self, sr, shell, seconds):
        self.sr, self.shell = sr, shell
        self.buf = np.zeros(int(seconds * sr) + 4096)
        self.ev = []

    def add(self, shot, q, group="main", **meta):
        base = int(q) - shot["lead"]
        need = base + len(shot["y"]) + 4096
        if need > len(self.buf):
            self.buf = np.concatenate([self.buf, np.zeros(need - len(self.buf))])
        ck.place(self.buf, shot, base)
        a, u = base + shot["abs_off"], base + shot["aud_off"]
        e = {"id": len(self.ev), "shell": self.shell, "sr": self.sr, "group": group, "abs_sample": float(a),
             "aud_sample": float(u), "abs_s": a / self.sr, "velocity": 100, **meta}
        self.ev.append(e)
        return e

    def audio(self, length=None):
        return self.buf[:length] if length else self.buf


def positions(sr, n, rng, gap=(0.5, 0.8), start=0.6):
    """n off-grid integer positions and the length (samples) that holds them."""
    t = int(start * sr) + int(rng.integers(0, 997))
    pos = []
    for _ in range(n):
        pos.append(t)
        t += int(rng.uniform(*gap) * sr)
    return pos, t + int(0.5 * sr)


def case(tag, mix, length=None, variant=""):
    n = length or int(len(mix.ev) and (max(e["abs_sample"] for e in mix.ev) + 0.6 * mix.sr)) or len(mix.buf)
    n = min(max(n, 1), len(mix.buf))
    return {"tag": tag, "shell": mix.shell, "sr": mix.sr, "buf": mix.buf[:n].copy(), "events": mix.ev, "variant": variant}


def write_case(out, hid, c):
    out = Path(out)
    stem = f"{c['shell']}_{c['tag']}"
    (out / "audio" / hid).mkdir(parents=True, exist_ok=True)
    (out / "ref" / hid).mkdir(parents=True, exist_ok=True)
    x = c["buf"]
    peak = float(np.max(np.abs(x)))
    if peak > 0.999:
        x = x * (0.999 / peak)
    sf.write(str(out / "audio" / hid / f"{stem}.wav"), x, c["sr"], subtype="PCM_24")
    keys = sorted({k for e in c["events"] for k in e} | {"tag"})
    with open(out / "ref" / hid / f"{stem}.csv", "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=keys)
        w.writeheader()
        for e in c["events"]:
            w.writerow({**e, "tag": c["tag"]})
    return out / "audio" / hid / f"{stem}.wav"


# ---------------------------------------------------------------- generators (audio cases)
# Each returns a list of cases. `quick` shrinks the repeats (tests).

def reps(quick, n):
    return max(2, n // 4) if quick else n


def gen_m3_phase(quick=False):
    """M3 end to end: the attack at phase 0 to 9 ms inside the 10 ms frame grid (the file starts on a frame edge)."""
    out = []
    for shell in ("kick", "snare"):
        rng = np.random.default_rng(31)
        phases = [0, 2, 4, 6, 8] if quick else list(range(10))
        n = reps(quick, 8)
        m = Mix(SR, shell, 1.0 + len(phases) * n * 0.6)
        t = 0
        for rep in range(n):
            for ph in phases:
                t += int(0.5 * SR) + int(rng.integers(0, 100))
                t = (t // 480) * 480                       # a 10 ms frame edge
                q = t + int(round(ph * 0.001 * SR))
                m.add(mk(shell, SR, 1000 + rep * 10 + ph), q, phase_ms=float(ph))
                t += int(0.3 * SR)
        out.append(case("phase", m))
    return out


def gen_s1(quick=False):
    out = []
    tunes = [40.0, 70.0, 120.0] if quick else [40.0, 55.0, 70.0, 90.0, 120.0]
    for shell in ("kick", "t4"):
        rng = np.random.default_rng(41)
        n = reps(quick, 8)
        m = Mix(SR, shell, 1.0 + len(tunes) * n * 0.7)
        pos, _ = positions(SR, len(tunes) * n, rng)
        i = 0
        for rep in range(n):
            for f in tunes:
                kw = {"tune": f, "phase0": 0.0}
                if shell == "kick":
                    kw["click"] = 0.0
                else:
                    kw["click"] = 0.0
                m.add(mk(shell, SR, 2000 + i, rise=0.0, **kw), pos[i], tune_hz=f, period_ms=1000.0 / f)
                i += 1
        out.append(case("lowfreq", m))
    return out


def gen_s2(quick=False):
    out = []
    rises = [0.0, 2.0] if quick else [0.0, 0.5, 2.0]
    for shell in ("kick", "snare"):
        rng = np.random.default_rng(51)
        n = reps(quick, 10)
        m = Mix(SR, shell, 1.0 + len(rises) * 2 * n * 0.7)
        pos, _ = positions(SR, len(rises) * 2 * n, rng)
        i = 0
        for rep in range(n):
            for r in rises:
                for pol in (1, -1):
                    m.add(mk(shell, SR, 3000 + i // 2, rise=r, pol=pol), pos[i], rise_ms=r, pol=float(pol), pair=float(i // 2))
                    i += 1
        out.append(case("polarity", m))
    return out


def clip_shot(shell, sr, seed, rise, gain_db):
    """The hit at full scale, driven `gain_db` into a hard clip and brought back to -6 dBFS: flat top, faster attack."""
    s = mk(shell, sr, seed, rise=rise, level=0.0)
    y = np.clip(s["y"] * 10 ** (gain_db / 20.0), -1.0, 1.0)
    y = y * (0.5 / np.abs(y).max())
    return shot_from(y, sr)


def gen_s3(quick=False):
    out = []
    drives = [0.0, 12.0] if quick else [0.0, 12.0, 18.0]
    for shell in ("kick", "snare"):
        rng = np.random.default_rng(61)
        n = reps(quick, 8)
        rises = [2.0] if quick else [2.0, 5.0]
        m = Mix(SR, shell, 1.0 + len(rises) * len(drives) * n * 0.7)
        pos, _ = positions(SR, len(rises) * len(drives) * n, rng)
        i = 0
        for rep in range(n):
            for r in rises:
                for g in drives:
                    s = clip_shot(shell, SR, 4000 + rep * 7 + int(r), r, g)
                    m.add(s, pos[i], rise_ms=r, drive_db=g, pair=float(rep * 10 + int(r)))
                    i += 1
        out.append(case("clip", m))
    return out


def gen_s4(quick=False):
    out = []
    cuts = [0, 2000] if quick else [0, 4000, 2000]
    for shell in ("kick", "snare"):
        rng = np.random.default_rng(71)
        n = reps(quick, 6)
        rises = [0.5] if quick else [0.5, 2.0]
        m = Mix(SR, shell, 1.0 + len(rises) * len(cuts) * n * 0.7)
        pos, _ = positions(SR, len(rises) * len(cuts) * n, rng)
        i = 0
        for rep in range(n):
            for r in rises:
                base = mk(shell, SR, 5000 + rep * 5 + int(r * 2), rise=r)["y"]
                for fc in cuts:
                    y = base if fc == 0 else lowpass(base, SR, float(fc))
                    y = y * (0.5 / np.abs(y).max())
                    m.add(shot_from(y, SR), pos[i], rise_ms=r, cutoff_hz=float(fc), pair=float(rep * 10 + int(r * 2)))
                    i += 1
        out.append(case("lowpass", m))
    return out


def gen_c1(quick=False):
    out = []
    bpms = [120, 220] if quick else [120, 160, 200, 220]
    for shell in ("kick", "snare"):
        rng = np.random.default_rng(81)
        seqs = 2 if quick else 4
        length = 8 if quick else 12
        total = 1.0 + (len(bpms) * seqs * (length * 0.13 + 1.5)) + length * 0.8
        m = Mix(SR, shell, total)
        t = int(0.6 * SR)
        i = 0
        for _ in range(length):                          # isolated baseline hits
            m.add(mk(shell, SR, 6000 + i), t, spacing_ms=0.0, k=0.0)
            t += int(0.8 * SR) + int(rng.integers(0, 500))
            i += 1
        for bpm in bpms:
            sp = 60.0 / bpm / 4.0
            for _s in range(seqs):
                t += int(1.2 * SR)
                for k in range(length):
                    m.add(mk(shell, SR, 6000 + i), t + int(rng.integers(0, 3)), spacing_ms=sp * 1000.0, k=float(k))
                    t += int(sp * SR)
                    i += 1
        out.append(case("roll", m, length=t + int(0.8 * SR)))
    return out


def hat_bleed(n, sr, rng, level_db, period=0.125):
    """Hat bleed: high-passed noise bursts (6 to 16 kHz, decay 20 ms) every `period` s, peak `level_db`."""
    x = np.zeros(n)
    k = int(0.15 * sr)
    t = np.arange(k) / sr
    for s in range(0, n - k, int(period * sr)):
        x[s:s + k] += ck._bandpass_noise(rng, k, sr, 6000, 16000) * np.exp(-t / 0.02)
    return x / np.abs(x).max() * 10 ** (level_db / 20.0)


def gen_c2(quick=False):
    levels = [None, -40.0, -20.0] if quick else [None, -50.0, -40.0, -34.0, -28.0, -22.0, -16.0, -10.0]
    out = []
    rng = np.random.default_rng(91)
    n = reps(quick, 24)
    pos, length = positions(SR, n, rng, gap=(0.55, 0.8))
    for lv in levels:
        m = Mix(SR, "snare", length / SR)
        for i, q in enumerate(pos):
            m.add(mk("snare", SR, 7000 + i), q, bleed_db=lv if lv is not None else -99.0)
        if lv is not None:
            m.buf += hat_bleed(len(m.buf), SR, np.random.default_rng(5), lv)
        out.append(case("bleed" + ("none" if lv is None else str(int(-lv))), m, length=length))
    return out


def gen_c3(quick=False):
    out = []
    delays = [10.0, 30.0] if quick else [10.0, 20.0, 30.0, 40.0]
    grace_db = [-14.0] if quick else [-14.0, -6.0]
    for gd in grace_db:
        rng = np.random.default_rng(101)
        n = reps(quick, 8)
        m = Mix(SR, "snare", 1.0 + len(delays) * n * 0.9)
        pos, _ = positions(SR, len(delays) * n, rng, gap=(0.6, 0.9))
        i = 0
        for rep in range(n):
            for d in delays:
                q = pos[i] + int(0.05 * SR)
                m.add(mk("snare", SR, 8000 + i), q, "main", delay_ms=d, grace_db=gd)
                m.add(mk("snare", SR, 8500 + i, level=gd), q - int(round(d * 0.001 * SR)), "grace", delay_ms=d,
                      grace_db=gd)
                i += 1
        out.append(case(f"flam{int(-gd)}", m))
    return out


def gen_c4(quick=False):
    out = []
    gaps = [20.0, 60.0] if quick else [20.0, 40.0, 60.0, 80.0, 120.0, 160.0]
    offs = [-3.0, -12.0] if quick else [0.0, -3.0, -6.0, -9.0, -12.0, -20.0]
    rng = np.random.default_rng(111)
    n = reps(quick, 6)
    m = Mix(SR, "snare", 1.0 + len(gaps) * len(offs) * n * 0.9)
    pos, _ = positions(SR, len(gaps) * len(offs) * n, rng, gap=(0.6, 0.9))
    i = 0
    for rep in range(n):
        for g in gaps:
            for o in offs:
                m.add(mk("snare", SR, 9000 + i), pos[i], "main", gap_ms=g, ghost_db=o)
                m.add(mk("snare", SR, 9500 + i, level=-6.0 + o), pos[i] + int(round(g * 0.001 * SR)), "ghost",
                      gap_ms=g, ghost_db=o)
                i += 1
    out.append(case("ghost", m))
    return out


def gen_c5(quick=False):
    """A tom track that hears the kick: kick bleed (-30 dBFS) and a ringing tom (170 Hz sine, decay 0.3 s) excited at each
    kick; peak level of the ring per case. A few real tom hits sit in the file."""
    rings = [None, -40.0, -22.0] if quick else [None, -50.0, -40.0, -34.0, -28.0, -22.0, -16.0, -10.0]
    out = []
    rng = np.random.default_rng(121)
    nk = reps(quick, 16)
    pos, length = positions(SR, nk + 4, rng, gap=(0.7, 1.0))
    for rv in rings:
        m = Mix(SR, "t2", length / SR)
        for i, q in enumerate(pos):
            if i % 5 == 4:
                m.add(mk("t2", SR, 10000 + i, level=-6.0), q, "tom", ring_db=rv if rv is not None else -99.0)
            else:
                kick = mk("kick", SR, 10100 + i, level=-30.0)
                ck.place(m.buf, kick, q - kick["lead"])
                if rv is not None:
                    t = np.arange(int(0.6 * SR)) / SR
                    ring = np.sin(2 * np.pi * 170.0 * t) * np.exp(-t / 0.3) * 10 ** (rv / 20.0)
                    ring[:int(0.002 * SR)] *= np.linspace(0, 1, int(0.002 * SR))
                    m.buf[q:q + len(ring)] += ring
                m.ev.append({"id": len(m.ev), "shell": "t2", "sr": SR, "group": "kick", "abs_sample": -1e9,
                             "aud_sample": -1e9, "abs_s": -1e6, "velocity": 100, "ring_db": rv if rv is not None else -99.0,
                             "kick_s": q / SR})
        out.append(case("ring" + ("none" if rv is None else str(int(-rv))), m, length=length))
    return out


def gen_r1(quick=False):
    out = []
    specs = [("clean", 0.0, 0.0, 0.0), ("dc03", 0.03, 0.0, 0.0), ("dc10", 0.10, 0.0, 0.0),
             ("rumble5", 0.0, 5.0, 10 ** (-20 / 20)), ("rumble10", 0.0, 10.0, 10 ** (-20 / 20)),
             ("rumble20", 0.0, 20.0, 10 ** (-20 / 20)), ("rumble20q", 0.0, 20.0, 10 ** (-30 / 20))]
    if quick:
        specs = [specs[0], specs[2], specs[5]]
    for shell in ("kick", "snare"):
        rng = np.random.default_rng(131)
        n = reps(quick, 20)
        pos, length = positions(SR, n, rng)
        for tag, dc, f, amp in specs:
            m = Mix(SR, shell, length / SR)
            for i, q in enumerate(pos):
                rise = 2.0 if i % 2 else 0.0                     # a sharp and a slow onset in every file
                m.add(mk(shell, SR, 11000 + i, rise=rise), q, dc=dc, rumble_hz=f, rumble_amp=amp, rise_ms=rise)
            t = np.arange(len(m.buf)) / SR
            m.buf += dc
            if f:
                m.buf += amp * np.sin(2 * np.pi * f * t + 0.3)
            out.append(case(tag, m, length=length))
    return out


def gen_r2(quick=False):
    out = []
    specs = [("clean", 0, 0.0)] + [(f"hum{hz}_{int(-lv)}", hz, lv) for hz in ((50,) if quick else (50, 60))
                                  for lv in ((-40.0,) if quick else (-50.0, -40.0, -30.0))]
    for shell in ("kick", "snare"):
        rng = np.random.default_rng(141)
        n = reps(quick, 20)
        pos, length = positions(SR, n, rng)
        for tag, hz, lv in specs:
            m = Mix(SR, shell, length / SR)
            for i, q in enumerate(pos):
                rise = 2.0 if i % 2 else 0.0
                m.add(mk(shell, SR, 12000 + i, rise=rise), q, hum_hz=float(hz), hum_db=lv if hz else -99.0, rise_ms=rise)
            if hz:
                t = np.arange(len(m.buf)) / SR
                hum = sum(np.sin(2 * np.pi * hz * h * t + 0.5 * h) / h for h in range(1, 7))
                m.buf += hum / np.abs(hum).max() * 10 ** (lv / 20.0)
            out.append(case(tag, m, length=length))
    return out


def gate_shot(y, sr, thr_db, hold=0.2, rel=0.03):
    """A noise gate on one hit: closed (zero) until the first sample at `thr_db` under the peak, open until `hold` s after
    that, then a release fade. Returns (gated y, gate open index)."""
    a = np.abs(y)
    gi = int(np.argmax(a >= a.max() * 10 ** (thr_db / 20.0)))
    g = np.zeros(len(y))
    h, r = int(hold * sr), int(rel * sr)
    g[gi:gi + h] = 1.0
    g[gi + h:gi + h + r] = np.linspace(1, 0, r)[:max(0, len(y) - gi - h)]
    return y * g, gi


def gen_r3(quick=False):
    out = []
    thrs = [-20.0] if quick else [-30.0, -20.0, -12.0]
    rises = [0.5, 2.0] if quick else [0.5, 2.0, 5.0]
    for shell in ("kick", "snare"):
        rng = np.random.default_rng(151)
        n = reps(quick, 6)
        m = Mix(SR, shell, 1.0 + len(thrs) * len(rises) * n * 0.7)
        pos, _ = positions(SR, len(thrs) * len(rises) * n, rng)
        i = 0
        for rep in range(n):
            for r in rises:
                y = mk(shell, SR, 13000 + rep * 3 + int(r), rise=r)["y"]
                for th in thrs:
                    yg, gi = gate_shot(y, SR, th)
                    s = shot_from(yg, SR)
                    e = m.add({**s, "abs_off": float(np.argmax(np.abs(y) > 0)), "aud_off": s["aud_off"]}, pos[i],
                              rise_ms=r, gate_db=th)
                    e["gate_sample"] = float(pos[i] + gi)
                    e["first_motion_sample"] = e["abs_sample"]
                    i += 1
        # the placed shot is the gated one: the reference stays the first motion of the ungated hit
        out.append(case("gate", m))
    return out


def gen_r4(quick=False):
    out = []
    rates = [44100, 96000] if quick else [44100, 48000, 88200, 96000]
    for shell in ("kick", "snare"):
        for sr in rates:
            rng = np.random.default_rng(161)
            n = reps(quick, 8)
            rises = [0.0, 2.0] if quick else [0.0, 0.5, 2.0]
            m = Mix(sr, shell, 1.0 + len(rises) * n * 0.7)
            pos, _ = positions(sr, len(rises) * n, rng)
            i = 0
            for rep in range(n):
                for r in rises:
                    m.add(mk(shell, sr, 14000 + i, rise=r), pos[i], rise_ms=r)
                    i += 1
            out.append(case(f"sr{sr}", m))
    return out


def gen_r5(quick=False):
    out = []
    delays = [0.0, 1.0, 3.0] if quick else [0.0, 0.5, 1.0, 2.0, 3.0]
    for shell in ("snare", "kick"):
        for variant, (l1, l2) in (("earlier_louder", (-6.0, -9.0)), ("earlier_quieter", (-12.0, -6.0))):
            rng = np.random.default_rng(171)
            n = reps(quick, 8)
            m = Mix(SR, shell, 1.0 + len(delays) * n * 0.7)
            pos, _ = positions(SR, len(delays) * n, rng)
            i = 0
            for rep in range(n):
                for d in delays:
                    ds = int(round(d * 0.001 * SR))
                    a = mk(shell, SR, 15000 + i, level=l1)["y"]
                    b = mk(shell, SR, 15000 + i, level=l2, pol=-1)["y"]
                    y = np.zeros(len(a) + ds)
                    y[:len(a)] += a
                    if d > 0:
                        y[ds:ds + len(b)] += b
                    s = shot_from(y, SR)
                    e = m.add(s, pos[i], delay_ms=d)
                    e["second_sample"] = e["abs_sample"] + ds
                    i += 1
            out.append(case(f"mics_{variant}", m, variant=variant))
    return out


def gen_r6(quick=False):
    out = []
    ps = [5.0, 25.0, 50.0] if quick else [2.0, 10.0, 25.0, 40.0, 50.0, 80.0]
    for shell in ("kick", "snare"):
        for p in ps:
            rng = np.random.default_rng(181)
            n = 6
            total = 4.0
            m = Mix(SR, shell, total)
            length = int(total * SR)
            m.add(mk(shell, SR, 16000), int(p * 0.001 * SR), "first", edge_ms=p)
            for i in range(3):
                m.add(mk(shell, SR, 16010 + i), int((1.0 + i * 0.8) * SR) + int(rng.integers(0, 777)), "mid", edge_ms=p)
            m.add(mk(shell, SR, 16020), length - int(p * 0.001 * SR), "last", edge_ms=p)
            out.append(case(f"edge{int(p)}", m, length=length))
    return out


GENERATORS = {"M3e": gen_m3_phase, "S1": gen_s1, "S2": gen_s2, "S3": gen_s3, "S4": gen_s4, "C1": gen_c1, "C2": gen_c2,
              "C3": gen_c3, "C4": gen_c4, "C5": gen_c5, "R1": gen_r1, "R2": gen_r2, "R3": gen_r3, "R4": gen_r4,
              "R5": gen_r5, "R6": gen_r6}


# ---------------------------------------------------------------- refinement-only sweeps (M1, M2, M3)

def find_parity():
    target = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    return target / "release" / "parity"


def refine_run(parity, wav, hits, window=None, rule=None):
    """`parity refine`: the core's refinement (rule `rule`, default `RULE`) on controlled coarse times; returns
    [{t, rise}] (seconds, dB)."""
    job = {"wav": str(wav), "mix": "average", "hits": [{"t0": t, "neighbors": []} for t in hits], "rule": rule or RULE}
    if window:
        job["window"] = list(window)
    f = Path(wav).with_suffix(".job.json")
    f.write_text(json.dumps(job))
    r = subprocess.run([str(parity), "refine", str(f)], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(r.stderr[-400:])
    return json.loads(r.stdout)["refined"]


RISES_M = [0.5, 1.0, 2.0, 3.0, 5.0, 8.0]
SHAPES = ["lin", "cos", "exp"]


def gen_m_rise(out, quick=False):
    """M1 and M2: hits of several rise shapes and times, the coarse time 4 to 7 ms before the attack. Returns the rows."""
    parity = find_parity()
    rows = []
    d = Path(out) / "audio" / "M"
    d.mkdir(parents=True, exist_ok=True)
    for shell in ("kick", "snare"):
        for shape in (["lin"] if quick else SHAPES):
            rng = np.random.default_rng(201)
            rises = [0.5, 2.0, 5.0] if quick else RISES_M
            n = 3 if quick else 6
            pos, length = positions(SR, len(rises) * n, rng, gap=(0.5, 0.6))
            m = Mix(SR, shell, length / SR)
            i = 0
            for rep in range(n):
                for r in rises:
                    m.add(mk(shell, SR, 17000 + i, rise=r, shape=shape), pos[i], rise_ms=r, shape=shape,
                          lead_ms=4.0 + (i % 4))
                    i += 1
            wav = d / f"{shell}_{shape}.wav"
            sf.write(str(wav), m.buf[:length], SR, subtype="PCM_24")
            t0s = [e["abs_s"] - e["lead_ms"] / 1000.0 for e in m.ev]
            ref = refine_run(parity, wav, t0s)
            x = m.buf
            for e, rr in zip(m.ev, ref):
                dpos = rr["t"] * SR
                pk = 10 ** (e_level(x, e) / 20.0)
                lvl = 20 * math.log10(np.abs(x[int(dpos):int(dpos) + int(0.0001 * SR) + 1]).max() / pk + 1e-12)
                rows.append({"hyp": "M", "shell": shell, "shape": shape, "rise_ms": e["rise_ms"],
                             "err_abs": dpos - e["abs_sample"], "level_under_peak_db": lvl, "rise_db": rr["rise"],
                             "ideal25": (0.25 * e["rise_ms"] * 0.001 * SR) if shape == "lin" else float("nan")})
    return rows


def e_level(x, e):
    """The peak (dBFS) of the hit of an event: the maximum of the 12 ms after its first motion."""
    a = int(e["abs_sample"])
    return 20 * math.log10(np.abs(x[a:a + int(0.012 * SR)]).max() + 1e-12)


def gen_m3_refine(out, quick=False, rule=None):
    """M3: the coarse time at `p` ms before the attack, p from -6 to +16 (the window is -2..+12 around the coarse time, so
    p = -(attack - t0) in the window's terms: attack position a = attack - t0 from -4 to +18 ms)."""
    parity = find_parity()
    rows = []
    d = Path(out) / "audio" / "M"
    d.mkdir(parents=True, exist_ok=True)
    offs = np.arange(-6.0, 18.01, 1.0 if quick else 0.5)
    for shell in ("kick", "snare"):
        rng = np.random.default_rng(211)
        pos, length = positions(SR, len(offs), rng, gap=(0.45, 0.5))
        m = Mix(SR, shell, length / SR)
        for i, a in enumerate(offs):
            m.add(mk(shell, SR, 18000 + i, rise=0.5), pos[i], attack_vs_t0_ms=float(a))
        wav = d / f"{shell}_window.wav"
        sf.write(str(wav), m.buf[:length], SR, subtype="PCM_24")
        t0s = [e["abs_s"] - e["attack_vs_t0_ms"] / 1000.0 for e in m.ev]
        ref = refine_run(parity, wav, t0s, rule=rule)
        for e, rr in zip(m.ev, ref):
            rows.append({"hyp": "M3r", "shell": shell, "attack_vs_t0_ms": e["attack_vs_t0_ms"],
                         "err_abs": rr["t"] * SR - e["abs_sample"], "t_rel_t0_ms": (rr["t"] - (e["abs_s"] - e["attack_vs_t0_ms"] / 1000.0)) * 1000.0})
    return rows


# ---------------------------------------------------------------- run and load

def generate(out=HYP, quick=False, only=None, log=print):
    out = Path(out)
    files = []
    for hid, fn in GENERATORS.items():
        if only and hid not in only:
            continue
        for c in fn(quick):
            files.append(write_case(out, hid, c))
        log(f"generated {hid}")
    return files


def run_cases(out=HYP, jobs=2, detectors=ck.DETECTORS, only=None, log=print):
    out = Path(out)
    tool = ck.find_tool()
    if not tool.is_file():
        raise SystemExit(f"no service_tool at {tool}")
    audio = sorted((out / "audio").glob("*/*.wav"))
    audio = [a for a in audio if a.parent.name in GENERATORS and (not only or a.parent.name in only)]
    unsupported = [a for a in audio if "sr88200" in a.name]            # the app reads 44.1, 48 and 96 kHz only
    audio = [a for a in audio if a not in unsupported]
    for a in unsupported:
        scan_reject(tool, out, a)
    work = [(a, d) for a in audio for d in detectors]
    with ThreadPoolExecutor(max_workers=jobs) as ex:
        futs = [(a, d, ex.submit(ck.run_one, tool, out, a, d, "none")) for a, d in work]
        for a, d, f in futs:
            f.result()
    log(f"ran {len(work)} detections")
    return len(work)


def scan_reject(tool, out, wav):
    """`tracks_scan` of a file with a rate the app does not read: the answer's `rejected` entry goes to `<out>/rejected.json`."""
    home = Path(out) / "run-home" / "scan"
    home.mkdir(parents=True, exist_ok=True)
    f = home / "steps.json"
    f.write_text(json.dumps([{"command": "tracks_scan", "args": {"paths": [str(wav)]}}]))
    env = {**os.environ, "DRUMSPIKE_HOME": str(home / "h"), "DRUMSPIKE_PROJECTS": str(home / "p")}
    r = subprocess.run([str(tool), "cli", "run", str(f)], capture_output=True, text=True, env=env)
    res = json.loads(r.stdout.splitlines()[0])["result"]
    path = Path(out) / "rejected.json"
    old = json.loads(path.read_text()) if path.is_file() else {}
    old[wav.name] = res["rejected"]
    path.write_text(json.dumps(old, indent=1))


def load_rows(out=HYP, only=None):
    """Every reference event of every case and detector, with the match (`found`, `err_abs`, `err_aud`) and the case's
    meta columns; plus the detections that matched no reference (`false`) per run."""
    out = Path(out)
    rows, runs = [], []
    for jf in sorted((out / "res").glob("*/*__*.json")):
        hid = jf.parent.name
        if only and hid not in only:
            continue
        stem, det = jf.stem.rsplit("__", 1)
        refs = ck.read_csv(out / "ref" / hid / f"{stem}.csv")
        res = json.loads(jf.read_text())
        det_s = sorted(e["t"] for e in res["events"]["events"])
        real = [r for r in refs if r["abs_s"] > -1e5]
        sr = int(real[0]["sr"]) if real else int(refs[0]["sr"])
        rws, counts = ck.compare_pair(real, det_s, sr)
        for r in rws:
            r.update(hyp=hid, case=stem, detector=det)
            if r.get("found"):
                r["det_sample"] = r["abs_sample"] + r["err_abs"]
        rows.extend(rws)
        pair_s, _, false = ck.match([r["abs_s"] for r in real], det_s)
        kicks = [r for r in refs if r["abs_s"] <= -1e5]
        runs.append({"hyp": hid, "case": stem, "detector": det, "sr": sr, **counts, "kicks": kicks,
                     "det_s": det_s, "false_s": [det_s[j] for j in false]})
    return rows, runs


# ---------------------------------------------------------------- verdicts
# `V(verdict, headline, table)`; a table is markdown. The thresholds are written next to each prediction.

def V(verdict, headline, table=""):
    return {"verdict": verdict, "headline": headline, "table": table}


def md(head, body):
    s = "| " + " | ".join(head) + " |\n|" + "---|" * len(head) + "\n"
    for r in body:
        s += "| " + " | ".join(str(c) for c in r) + " |\n"
    return s + "\n"


def mean(v):
    v = [x for x in v if x is not None and not (isinstance(x, float) and math.isnan(x))]
    return float(np.mean(v)) if v else float("nan")


def sd(v):
    v = [x for x in v if x is not None and not (isinstance(x, float) and math.isnan(x))]
    return float(np.std(v)) if v else float("nan")


def med(v):
    v = [x for x in v if x is not None and not (isinstance(x, float) and math.isnan(x))]
    return float(np.median(v)) if v else float("nan")


def f1(x):
    return "-" if x is None or (isinstance(x, float) and math.isnan(x)) else f"{x:+.1f}"


def rate(sel):
    return f"{sum(1 for r in sel if r['found'])}/{len(sel)}" if sel else "-"


def found(sel):
    return [r for r in sel if r["found"]]


def v_m1(mrows):
    sel = [r for r in mrows if r["rise_ms"] >= 1.0]
    body = []
    for shape in SHAPES:
        for shell in ("kick", "snare"):
            s = [r["level_under_peak_db"] for r in sel if r["shape"] == shape and r["shell"] == shell]
            if s:
                body.append([shape, shell, len(s), f"{med(s):.1f}", f"{np.percentile(s, 10):.1f} to {np.percentile(s, 90):.1f}"])
    m = med([r["level_under_peak_db"] for r in sel])
    ok = [abs(med([r["level_under_peak_db"] for r in sel if r["shape"] == sh])) for sh in SHAPES
          if any(r["shape"] == sh for r in sel)]
    held = all(9.0 <= x <= 15.0 for x in ok)
    return V("held" if held else "did not hold",
             f"detected point {abs(m):.1f} dB under the local peak (median, rise >= 1 ms; per shape {', '.join(f'{x:.1f}' for x in ok)})",
             "Level of the refined point under the hit peak (0.1 ms window), by shape and shell (median; p10 to p90), rise >= 1 ms\n\n"
             + md(["shape", "shell", "n", "median dB", "p10 to p90"], body))




def grp(rows, **kw):
    return [r for r in rows if all(r.get(k) == v for k, v in kw.items())]


def v_s1(rows):
    rr = [r for r in rows if r["hyp"] == "S1"]
    body, rs = [], []
    for det in ck.DETECTORS:
        for shell in ("kick", "t4"):
            s = grp(rr, detector=det, shell=shell)
            for f in sorted({r["tune_hz"] for r in s}):
                q = grp(s, tune_hz=f)
                body.append([det, shell, f"{f:g}", rate(q), f1(med([r["err_abs"] for r in found(q)])),
                             f1(med([r["err_aud"] for r in found(q)]))])
            fo = [r for r in s if r["found"]]
            if len(fo) >= 8:
                x = np.array([r["period_ms"] for r in fo]); y = np.array([r["err_aud"] for r in fo])
                rs.append((det, shell, float(np.corrcoef(x, y)[0, 1]) if x.std() and y.std() else 0.0,
                           float(np.polyfit(x, y, 1)[0])))
    kick = [(d, r, sl) for d, sh, r, sl in rs if sh == "kick"]
    held = bool(kick) and all(r > 0.5 and sl * (1000 / 40 - 1000 / 120) > 3.0 for _, r, sl in kick) if kick else False
    return V("held" if held else "did not hold",
             "; ".join(f"{d} {sh}: r(period, error against aud) {r:+.2f}, {sl:+.1f} samples per ms of period" for d, sh, r, sl in rs),
             "Pitched decaying sine from phase 0 (slow first half cycle), no click; error against `abs` and `aud`, samples (median)\n\n"
             + md(["det", "shell", "Hz", "found", "median abs", "median aud"], body))












def v_r3(rows):
    rr = [r for r in rows if r["hyp"] == "R3"]
    tab, fr = [], []
    for det in ck.DETECTORS:
        for shell in ("kick", "snare"):
            for th in sorted({r["gate_db"] for r in rr}):
                for rise in sorted({r["rise_ms"] for r in rr}):
                    s = found(grp(rr, detector=det, shell=shell, gate_db=th, rise_ms=rise))
                    if not s:
                        continue
                    gate = med([r["gate_sample"] - r["abs_sample"] for r in s])
                    e1 = med([r["err_abs"] for r in s])
                    e2 = med([r["det_sample"] - r["gate_sample"] for r in s])
                    if gate > 20:
                        fr.append((abs(e2), abs(e1)))
                    tab.append([det, shell, f"{th:g}", f"{rise:g}", len(s), f"{gate:.0f}", f1(e1), f1(e2)])
    closer = sum(1 for a, b in fr if a < b)
    held = len(fr) > 0 and closer >= 0.8 * len(fr)
    return V("held" if held else ("unclear" if closer >= 0.5 * max(1, len(fr)) else "did not hold"),
             f"in {closer} of {len(fr)} cells (gate opening 20 samples or more after the first motion) the detection is closer to the gate opening than to the first motion",
             "Gated hit: gate opens at the given level under the peak. Gate delay = opening minus first motion (samples); median error against the first motion and against the gate opening\n\n"
             + md(["det", "shell", "gate dB", "rise ms", "n", "gate delay", "err vs first motion", "err vs gate opening"], tab))





def paired(rr, det, shell, a, b, field="err_abs"):
    """Differences b minus a of `field` for the pairs (same `pair` id) found in both variants; `a` and `b` are filters."""
    base = [r for r in rr if r["detector"] == det and r["shell"] == shell]
    A = {r["pair"]: r[field] for r in base if r["found"] and a(r)}
    B = {r["pair"]: r[field] for r in base if r["found"] and b(r)}
    return [B[k] - A[k] for k in A if k in B]


def v_m2(mrows):
    lin = [r for r in mrows if r["shape"] == "lin"]
    rises = sorted({r["rise_ms"] for r in lin})
    body = []
    ex, rd, er = {}, {}, {}
    for rs in rises:
        s = [r for r in lin if r["rise_ms"] == rs]
        ex[rs] = med([r["err_abs"] - r["ideal25"] for r in s])
        rd[rs] = med([r["rise_db"] for r in s])
        er[rs] = med([r["err_abs"] for r in s])
        body.append([rs, len(s), f"{er[rs]:+.1f}", f"{med([r['ideal25'] for r in s]):+.1f}", f"{ex[rs]:+.1f}", f"{rd[rs]:.1f}"])
    short = mean([rd[r] for r in rises if r <= 2.0])
    long_ = mean([rd[r] for r in rises if 3.0 <= r <= 5.0])
    grows = er[max(rises)] - er[min(rises)]
    pre_in = short - long_
    # prediction: from about 1.5 ms of rise the pre-attack window lies in the rise (rise_db collapses) and the error grows with the rise
    held = pre_in > 6.0 and grows > 20.0
    return V("held" if held else ("unclear" if pre_in > 6.0 or grows > 20.0 else "did not hold"),
             f"`rise_db` {short:.1f} dB for rises up to 2 ms against {long_:.1f} dB for 3 to 5 ms (the pre-attack window moves into the ramp); error grows from {er[min(rises)]:+.1f} to {er[max(rises)]:+.1f} samples "
             f"but the part beyond the 25 % rule is small up to 5 ms ({ex.get(5.0, float('nan')):+.1f}) and large at {max(rises):g} ms ({ex[max(rises)]:+.1f})",
             "Linear ramps, refinement alone (coarse time 4 to 7 ms before the attack). Median error against the first motion, the error the 25 % rule alone predicts "
             "(25 % of the ramp), the excess, and `rise_db` (envelope at the anchor minus the pre-attack level; the walk-back stops 3 dB above it)\n\n"
             + md(["rise ms", "n", "median error", "25 % of ramp", "excess", "median rise_db"], body))


def v_m3(rows, mrows):
    body = []
    for a in sorted({r["attack_vs_t0_ms"] for r in mrows}):
        s = [r for r in mrows if r["attack_vs_t0_ms"] == a]
        body.append([f"{a:+.1f}", f"{med([r['err_abs'] for r in s]):+.1f}", f"{med([r['t_rel_t0_ms'] for r in s]):+.2f}"])
    pos = sorted({r["attack_vs_t0_ms"] for r in mrows})
    ok = {a: all(abs(r["err_abs"]) <= 24 for r in mrows if r["attack_vs_t0_ms"] == a) for a in pos}
    start = min((a for a in pos if a >= 0.0), key=abs, default=None)
    lo = hi = float("nan")
    if start is not None and ok[start]:
        lo = hi = start
        for a in [x for x in pos if x < start][::-1]:     # contiguous reach below and above the attack at the coarse time
            if not ok[a]:
                break
            lo = a
        for a in [x for x in pos if x > start]:
            if not ok[a]:
                break
            hi = a
    ph = [r for r in rows if r["hyp"] == "M3e"]
    pbody, ranges, lost = [], [], []
    for det in ck.DETECTORS:
        for shell in ("kick", "snare"):
            ps = sorted({r["phase_ms"] for r in ph})
            cells = []
            for p in ps:
                s = [r for r in ph if r["detector"] == det and r["shell"] == shell and r["phase_ms"] == p]
                if s:
                    cells.append((p, sum(1 for r in s if r["found"]) / len(s), med([r["err_abs"] for r in found(s)])))
                    far = [r["err_far"] / r["sr"] * 1000.0 for r in s if not r["found"] and "err_far" in r and not math.isnan(r["err_far"])]
                    far_s = f"{med(far):+.1f} ms ({len(far)})" if far else "-"
                    if far and len(far) >= len(s) / 2:
                        lost.append((det, shell, p, med(far)))
                    pbody.append([det, shell, f"{p:g}", rate(s), f1(med([r["err_abs"] for r in found(s)])), far_s])
            tot = sum(c[1] for c in cells)
            if cells and tot > 0.3 * len(cells):
                errs = [c[2] for c in cells if not math.isnan(c[2])]
                ranges.append((det, shell, max(c[1] for c in cells) - min(c[1] for c in cells), (max(errs) - min(errs)) if errs else 0.0))
    reach_ok = -3.0 <= lo <= -1.5 and 11.5 <= hi <= 13.5
    phase_dep = any(model_range[2] > 0.2 or model_range[3] > 5.0 for model_range in ranges if model_range[0] == "model")
    mod = [x for x in ranges if x[0] == "model"]
    dsp = [x for x in ranges if x[0] == "dsp"]
    return V("held" if reach_ok and phase_dep else ("did not hold" if not phase_dep else "unclear"),
             f"refinement alone reaches an attack from {lo:+.1f} to {hi:+.1f} ms around the coarse time without a break (window -2..+12 ms): {'confirmed' if reach_ok else 'not as predicted'}; "
             f"end to end by phase inside the frame, model: found-share range {', '.join(f'{x[1]} {x[2]*100:.0f} points' for x in mod)}, median-error range "
             f"{', '.join(f'{x[1]} {x[3]:.0f} samples' for x in mod)} ({'dependence' if phase_dep else 'no dependence'}); DSP found-share range {', '.join(f'{x[1]} {x[2]*100:.0f} points' for x in dsp)}. "
             + ("Lost hits (all repeats reported 10 to 50 ms away): " + "; ".join(f"{d} {sh} phase {p:g} ms: reported {m:+.0f} ms from the attack" for d, sh, p, m in lost) if lost else "No hit is reported away from its attack."),
             "Refinement alone: median error by position of the attack relative to the coarse time (positive: attack after it), ms\n\n"
             + md(["attack vs t0 ms", "median err samples", "refined point vs t0 ms"], body)
             + "End to end: found share and median error by the attack's phase inside the 10 ms frame (sharp onset)\n\n"
             + md(["det", "shell", "phase ms", "found", "median err", "missed hit: detection 10 to 50 ms away"], pbody))


def v_s2(rows):
    rr = [r for r in rows if r["hyp"] == "S2"]
    body, worst = [], 0.0
    for det in ck.DETECTORS:
        for shell in ("kick", "snare"):
            for rise in sorted({r["rise_ms"] for r in rr}):
                sub = [r for r in rr if r["rise_ms"] == rise]
                d = paired(sub, det, shell, lambda r: r["pol"] == 1.0, lambda r: r["pol"] == -1.0)
                md_ = med(d)
                if len(d) >= 4:
                    worst = max(worst, abs(md_))
                body.append([det, shell, f"{rise:g}", rate(grp(sub, detector=det, shell=shell, pol=1.0)), rate(grp(sub, detector=det, shell=shell, pol=-1.0)),
                             len(d), f1(md_)])
    return V("held" if worst <= 1.0 else "did not hold", f"largest median paired difference (negative minus positive) over cells with 4 or more pairs: {worst:.2f} samples",
             "Paired difference of the error (same hit, inverted), samples\n\n" + md(["det", "shell", "rise ms", "found +", "found -", "pairs", "median diff"], body))


def v_s3(rows):
    rr = [r for r in rows if r["hyp"] == "S3"]
    body, cells = [], []
    for det in ck.DETECTORS:
        for shell in ("kick", "snare"):
            for rise in sorted({r["rise_ms"] for r in rr}):
                sub = [r for r in rr if r["rise_ms"] == rise]
                for g in sorted({r["drive_db"] for r in sub if r["drive_db"] > 0}):
                    d = paired(sub, det, shell, lambda r: r["drive_db"] == 0.0, lambda r, g=g: r["drive_db"] == g)
                    m_ = med(d)
                    if len(d) >= 4:
                        cells.append(m_)
                    body.append([det, shell, f"{rise:g}", f"{g:g}", rate(grp(sub, detector=det, shell=shell, drive_db=0.0)),
                                 rate(grp(sub, detector=det, shell=shell, drive_db=g)), len(d), f1(m_)])
    m = med(cells)
    earlier = sum(1 for c in cells if c < -1.0)
    held = bool(cells) and m < -2.0 and earlier >= 0.6 * len(cells)
    return V("held" if held else ("unclear" if cells and abs(m) > 1.0 else "did not hold"),
             f"median paired difference clipped minus clean {m:+.1f} samples ({earlier} of {len(cells)} cells earlier, cells with 4 or more pairs)",
             "Hard clip driven 12 and 18 dB, back to -6 dBFS; paired difference (clipped minus clean, same hit), samples\n\n"
             + md(["det", "shell", "rise ms", "drive dB", "found clean", "found clipped", "pairs", "median diff"], body))


def v_s4(rows):
    rr = [r for r in rows if r["hyp"] == "S4"]
    body, cells = [], []
    for det in ck.DETECTORS:
        for shell in ("kick", "snare"):
            for rise in sorted({r["rise_ms"] for r in rr}):
                sub = [r for r in rr if r["rise_ms"] == rise]
                for fc in sorted({r["cutoff_hz"] for r in sub if r["cutoff_hz"] > 0}):
                    d = paired(sub, det, shell, lambda r: r["cutoff_hz"] == 0.0, lambda r, fc=fc: r["cutoff_hz"] == fc)
                    m_ = med(d)
                    if len(d) >= 4:
                        cells.append(m_)
                    body.append([det, shell, f"{rise:g}", f"{fc:g}", rate(grp(sub, detector=det, shell=shell, cutoff_hz=0.0)),
                                 rate(grp(sub, detector=det, shell=shell, cutoff_hz=fc)), len(d), f1(m_)])
    m = med(cells)
    later = sum(1 for c in cells if c > 1.0)
    held = bool(cells) and m > 2.0 and later >= 0.6 * len(cells)
    return V("held" if held else ("unclear" if cells and abs(m) > 1.0 else "did not hold"),
             f"median paired difference dark minus bright {m:+.1f} samples ({later} of {len(cells)} cells later, cells with 4 or more pairs)",
             "Causal 2nd-order low-pass; paired difference (dark minus bright, same hit), samples\n\n"
             + md(["det", "shell", "rise ms", "cutoff Hz", "found bright", "found dark", "pairs", "median diff"], body))


def v_c1(rows):
    rr = [r for r in rows if r["hyp"] == "C1"]
    body, shifts = [], []
    for det in ck.DETECTORS:
        for shell in ("kick", "snare"):
            base = found(grp(rr, detector=det, shell=shell, spacing_ms=0.0))
            b = med([r["err_abs"] for r in base])
            for sp in sorted({r["spacing_ms"] for r in rr if r["spacing_ms"] > 0}, reverse=True):
                s = [r for r in grp(rr, detector=det, shell=shell, spacing_ms=sp) if r["k"] >= 1]
                fo = found(s)
                m_ = med([r["err_abs"] for r in fo])
                if len(fo) >= 8 and len(base) >= 4:
                    shifts.append((sp, m_ - b))
                body.append([det, shell, f"{sp:.0f}", rate(s), f1(m_), f1(m_ - b) if len(fo) else "-"])
    fast = med([d for sp, d in shifts if sp < 80])
    slow = med([d for sp, d in shifts if sp >= 90])
    return V("held" if fast > 2.0 and fast > slow + 1.0 else "did not hold",
             f"median shift of hits 2 and later against isolated hits: {fast:+.1f} samples at sixteenths under 80 ms, {slow:+.1f} at 90 ms or more (cells with 8 or more found); "
             + ("later, as predicted" if fast > 2.0 else "the shift is early, not late" if fast < -1.0 else "no shift"),
             "Rolls of one hit repeated (the first of each roll left out); median error and its shift against the isolated hits of the file\n\n"
             + md(["det", "shell", "sixteenth ms", "found", "median err", "median - isolated"], body))


def v_c2(rows, runs):
    rr = [r for r in rows if r["hyp"] == "C2"]
    tab, start_move, start_false = [], {}, {}
    for det in ck.DETECTORS:
        base = found(grp(rr, detector=det, bleed_db=-99.0))
        b = med([r["err_abs"] for r in base]) if len(base) >= 6 else 0.0
        run0 = [x for x in runs if x["hyp"] == "C2" and x["detector"] == det and x["case"] == "snare_bleednone"]
        f0 = run0[0]["false"] if run0 else 0
        for lv in sorted({r["bleed_db"] for r in rr if r["bleed_db"] > -99}):
            s = grp(rr, detector=det, bleed_db=lv)
            fo = found(s)
            run = [x for x in runs if x["hyp"] == "C2" and x["detector"] == det and x["case"] == f"snare_bleed{int(-lv)}"]
            fa = run[0]["false"] if run else 0
            m_ = med([r["err_abs"] for r in fo])
            tab.append([det, f"{lv:g}", rate(s), f1(m_), f"{sd([r['err_abs'] for r in fo]):.1f}", fa])
            if len(fo) >= 6 and (abs(m_ - b) > 2.0 or sd([r["err_abs"] for r in fo]) > 5.0) and det not in start_move:
                start_move[det] = lv
            if fa > f0 + 1 and det not in start_false:
                start_false[det] = lv
    # the levels run quiet to loud: the first hit of the condition is the lowest level
    head = ("median moves by more than 2 samples or the spread (sd) exceeds 5 samples from bleed " + ", ".join(f"{d} {v:g} dB" for d, v in start_move.items())) if start_move else "onset does not move up to -10 dB"
    head += "; false hits exceed the clean file's by more than 1 from " + (", ".join(f"{d} {v:g} dB" for d, v in start_false.items()) if start_false else "none")
    held = bool(start_move) and bool(start_false)
    return V("held" if held else ("unclear" if start_move or start_false else "did not hold"), head,
             "Hat bleed (6 to 16 kHz bursts every 125 ms, peak at the given level); 24 snare hits at -6 dBFS per run, sharp onset. False = detections without a hit; the clean file has "
             + ", ".join(f"{d} {[x for x in runs if x['hyp'] == 'C2' and x['detector'] == d and x['case'] == 'snare_bleednone'][0]['false'] if [x for x in runs if x['hyp'] == 'C2' and x['detector'] == d and x['case'] == 'snare_bleednone'] else '-'}" for d in ck.DETECTORS)
             + "\n\n" + md(["det", "bleed dB", "found", "median err", "sd", "false"], tab))


def v_c3(rows):
    rr = [r for r in rows if r["hyp"] == "C3"]
    tab = []
    mains, errs = [], []
    for det in ck.DETECTORS:
        for gd in sorted({r["grace_db"] for r in rr}):
            for d in sorted({r["delay_ms"] for r in rr}):
                m_ = grp(rr, detector=det, grace_db=gd, delay_ms=d, group="main")
                g_ = grp(rr, detector=det, grace_db=gd, delay_ms=d, group="grace")
                if not m_ and not g_:
                    continue
                both = sum(1 for a, b in zip(sorted(m_, key=lambda r: r["id"]), sorted(g_, key=lambda r: r["id"])) if a["found"] and b["found"])
                tab.append([det, f"{gd:g}", f"{d:g}", rate(m_), f1(med([r["err_abs"] for r in found(m_)])), rate(g_), f1(med([r["err_abs"] for r in found(g_)])), both])
                if d >= 20:
                    mains.append(sum(1 for r in m_ if r["found"]) / max(1, len(m_)))
                    errs += [r["err_abs"] for r in found(m_)]
    mf = mean(mains)
    gf = mean([sum(1 for r in grp(rr, group="grace", delay_ms=d) if r["found"]) / max(1, len(grp(rr, group="grace", delay_ms=d))) for d in sorted({r["delay_ms"] for r in rr})])
    held = mf >= 0.8 and abs(med(errs)) <= 3.0
    return V("held" if held else "did not hold",
             f"main hit found in {mf*100:.0f} % of the flams with spacing 20 ms or more (median error of those {med(errs):+.1f} samples); the grace is found in {gf*100:.0f} % overall",
             "Flams: main hit -6 dBFS, grace at the given level, spacing in ms; found, median error (samples), and flams with both hits found\n\n"
             + md(["det", "grace dB", "spacing ms", "main found", "main median err", "grace found", "grace median err", "both"], tab))


def v_c4(rows):
    rr = [r for r in rows if r["hyp"] == "C4" and r["group"] == "ghost"]
    tab, curve = [], {}
    gaps = sorted({r["gap_ms"] for r in rr})
    for det in ck.DETECTORS:
        for o in sorted({r["ghost_db"] for r in rr}, reverse=True):
            cells = []
            for g in gaps:
                s = grp(rr, detector=det, ghost_db=o, gap_ms=g)
                cells.append(rate(s))
                curve[(det, o, g)] = sum(1 for r in s if r["found"]) / max(1, len(s))
            tab.append([det, f"{o:g}"] + cells)
    live = {d for d in ck.DETECTORS if any(v > 0 for (dd, o, g), v in curve.items() if dd == d)}        # a detector that finds no ghost at all says nothing
    loud = mean([v for (d, o, g), v in curve.items() if d in live and o >= -3.0])
    quiet = mean([v for (d, o, g), v in curve.items() if d in live and o <= -12.0])
    early = mean([v for (d, o, g), v in curve.items() if d in live and g <= 40])
    late = mean([v for (d, o, g), v in curve.items() if d in live and g >= 120])
    held = loud - quiet > 0.2 and late - early > 0.1
    return V("held" if held else ("unclear" if loud - quiet > 0.2 or late - early > 0.1 else "did not hold"),
             f"ghost found ({', '.join(sorted(live)) or 'no detector'}; the others find none) {loud*100:.0f} % at -3 dB or louder against {quiet*100:.0f} % at -12 dB or quieter; {early*100:.0f} % at gaps up to 40 ms against {late*100:.0f} % at 120 ms or more",
             "Ghost note after a loud hit (-6 dBFS): found share of the ghost by its level under the loud hit (rows) and the gap (columns)\n\n"
             + md(["det", "ghost dB"] + [f"{g:g} ms" for g in gaps], tab))


def v_c5(runs, rows):
    tab, first = [], {}
    for det in ck.DETECTORS:
        names = sorted({x["case"] for x in runs if x["hyp"] == "C5" and x["detector"] == det},
                       key=lambda n: -1 if n.endswith("none") else -int(n.replace("t2_ring", "")))
        # from the quietest ring to the loudest: `ring50` is -50 dBFS
        for n in names:
            run = [x for x in runs if x["hyp"] == "C5" and x["detector"] == det and x["case"] == n][0]
            ks = [k["kick_s"] for k in run["kicks"]]
            nfk = sum(1 for t in run["false_s"] if any(0.0 <= t - k <= 0.2 for k in ks))
            lv = "none" if n.endswith("none") else f"-{n.replace('t2_ring', '')}"
            tab.append([det, lv, len(ks), nfk, f"{nfk / max(1, len(ks)):.2f}", run["false"]])
            if not n.endswith("none") and nfk / max(1, len(ks)) >= 0.2 and det not in first:
                first[det] = int(lv)
    head = "false tom hits after a kick (0.2 per kick or more) from ring level " + (", ".join(f"{d} {v} dBFS" for d, v in first.items()) if first else "none up to -10 dBFS")
    return V("held" if first else "did not hold", head,
             "Tom track hearing the kick (kick bleed -30 dBFS; 170 Hz ring of the given peak, decay 0.3 s): detections within 200 ms after a kick that are no tom hit\n\n"
             + md(["det", "ring dBFS", "kicks", "false after kick", "per kick", "all false"], tab))


def v_cond(rows, runs, hid, label, strong):
    """R1 and R2: the median error and the found share against the clean file of the shell (per rise), and the false hits
    against the clean file. `strong` names the cases the verdict is judged on (the strongest disturbance)."""
    rr = [r for r in rows if r["hyp"] == hid]
    tab, hits = [], []
    for det in ck.DETECTORS:
        for shell in ("kick", "snare"):
            run0 = [x for x in runs if x["hyp"] == hid and x["detector"] == det and x["case"] == f"{shell}_clean"]
            f0 = run0[0]["false"] if run0 else 0
            for rise in sorted({r["rise_ms"] for r in rr}):
                cb = grp(rr, detector=det, case=f"{shell}_clean", rise_ms=rise)
                base = found(cb)
                b = med([r["err_abs"] for r in base]) if len(base) >= 4 else 0.0
                rb = len(base) / max(1, len(cb))
                for tag in sorted({r["case"] for r in rr if r["shell"] == shell}):
                    s = grp(rr, detector=det, case=tag, rise_ms=rise)
                    fo = found(s)
                    run = [x for x in runs if x["hyp"] == hid and x["detector"] == det and x["case"] == tag]
                    fa = run[0]["false"] if run else 0
                    d = med([r["err_abs"] for r in fo]) - b if len(fo) >= 4 and len(base) >= 4 else float("nan")
                    drop = rb - len(fo) / max(1, len(s))
                    tab.append([det, tag, f"{rise:g}", rate(s), f1(med([r["err_abs"] for r in fo])), f1(d), fa])
                    if tag != f"{shell}_clean":
                        hits.append((abs(d) if not math.isnan(d) else 0.0, fa - f0, drop if len(base) >= 4 else 0.0, tag))
    worst = max((h[0] for h in hits), default=0.0)
    more = max((h[1] for h in hits), default=0)
    drop = max((h[2] for h in hits), default=0.0)
    held = worst > 5.0 or more > 2 or drop > 0.25
    top = max(hits, key=lambda h: h[0])[3] if hits else "-"
    return V("held" if held else "did not hold",
             f"over all cases: largest shift of the median error {worst:.1f} samples ({top}; cells with 4 or more found in both files), most false hits beyond the clean file {more}, largest drop of the found share {drop*100:.0f} points",
             label + "\n\n" + md(["det", "case", "rise ms", "found", "median err", "shift vs clean", "false"], tab))


def v_r1(rows, runs):
    return v_cond(rows, runs, "R1", "DC offset (fraction of full scale) and subsonic rumble (-20 dBFS, `q` = -30 dBFS) added; hits alternate between a sharp and a 2 ms onset; shift of the median error against the clean file of the shell", {"dc10", "rumble20", "rumble10"})


def v_r2(rows, runs):
    return v_cond(rows, runs, "R2", "Hum (50 or 60 Hz, harmonics 1 to 6 at 1/h; case name hum<Hz>_<minus dBFS>); sharp and 2 ms onsets; shift against the clean file of the shell", {"hum50_30", "hum60_30"})


def v_r4(rows, out=HYP):
    rr = [r for r in rows if r["hyp"] == "R4"]
    tab, spreads = [], []
    for det in ck.DETECTORS:
        for shell in ("kick", "snare"):
            for rise in sorted({r["rise_ms"] for r in rr}):
                ms, smp = [], []
                for srate in sorted({r["sr"] for r in rr}):
                    s = found(grp(rr, detector=det, shell=shell, rise_ms=rise, sr=srate))
                    e = med([r["err_abs"] for r in s])
                    ms.append(e / srate * 1000.0)
                    smp.append(e)
                    tab.append([det, shell, f"{rise:g}", int(srate), rate(grp(rr, detector=det, shell=shell, rise_ms=rise, sr=srate)), f1(e), f"{e / srate * 1000.0:+.3f}"])
                if rise > 0 and all(not math.isnan(x) for x in ms):
                    spreads.append((max(ms) - min(ms), (max(smp) - min(smp))))
    rej = Path(out) / "rejected.json"
    note = ""
    if rej.is_file():
        for k, v in json.loads(rej.read_text()).items():
            note += f"88.2 kHz file `{k}`: " + (v[0]["reason"] if v else "accepted") + "\n\n"
    ms_spread, smp_spread = mean([a for a, _ in spreads]), mean([b for _, b in spreads])
    held = bool(spreads) and ms_spread < 0.1 and smp_spread > 5 * ms_spread * 48
    return V("held" if held else "did not hold",
             f"mean spread of the median error across the rates (rise > 0): {ms_spread:.3f} ms against {smp_spread:.1f} samples; 88.2 kHz is refused by the app (reads 44.1, 48 and 96 kHz)",
             note + "Median error against the first motion by sample rate, in samples and in ms\n\n"
             + md(["det", "shell", "rise ms", "rate", "found", "median samples", "median ms"], tab))


def v_r5(rows):
    rr = [r for r in rows if r["hyp"] == "R5"]
    tab, ok, extra = [], [], []
    for det in ck.DETECTORS:
        for shell in ("snare", "kick"):
            for var in ("earlier_louder", "earlier_quieter"):
                base = med([r["err_abs"] for r in found([r for r in rr if r["detector"] == det and r["shell"] == shell and r["case"].endswith(var) and r["delay_ms"] == 0.0])])
                for d in sorted({r["delay_ms"] for r in rr}):
                    s = found([r for r in rr if r["detector"] == det and r["shell"] == shell and r["case"].endswith(var) and r["delay_ms"] == d])
                    if len(s) < 3:
                        continue
                    e1 = med([r["err_abs"] for r in s])
                    e2 = med([r["det_sample"] - r["second_sample"] for r in s])
                    tab.append([det, shell, var, f"{d:g}", len(s), f1(e1), f1(e2), f1(e1 - base)])
                    if d >= 1.0:
                        ok.append(abs(e1) < abs(e2))
                        extra.append((var, e1 - base))
    quiet = med([e for v, e in extra if v == "earlier_quieter"])
    loud = med([e for v, e in extra if v == "earlier_louder"])
    held = bool(ok) and sum(ok) >= 0.8 * len(ok)
    return V("held" if held else ("unclear" if sum(ok) >= 0.5 * max(1, len(ok)) else "did not hold"),
             f"the median detection is closer to the earlier mic than to the later one in {sum(ok)} of {len(ok)} cells (delay 1 ms or more); it is late against the single mic by {loud:+.1f} samples when the earlier mic is the louder and {quiet:+.1f} when it is 6 dB quieter",
             "Two mics summed (the second delayed, inverted polarity); median error against the earlier mic and against the later mic, and against the single mic (delay 0), samples\n\n"
             + md(["det", "shell", "variant", "delay ms", "n", "err vs earlier", "err vs later", "vs single mic"], tab))


def v_r6(rows, runs):
    rr = [r for r in rows if r["hyp"] == "R6"]
    tab, share, exact, midshare = [], [], [], []
    for det in ck.DETECTORS:
        for shell in ("kick", "snare"):
            mid = med([r["err_abs"] for r in found(grp(rr, detector=det, shell=shell, group="mid"))])
            for p in sorted({r["edge_ms"] for r in rr}):
                cells = [det, shell, f"{p:g}"]
                for g in ("first", "last", "mid"):
                    s = grp(rr, detector=det, shell=shell, edge_ms=p, group=g)
                    fo = found(s)
                    cells += [rate(s), f1(med([r["err_abs"] for r in fo]))]
                    if g != "mid":
                        share.append(len(fo) / max(1, len(s)))
                        exact += [abs(r["err_abs"] - mid) for r in fo]
                    else:
                        midshare.append(len(fo) / max(1, len(s)))
                tab.append(cells)
    held = bool(share) and mean(share) >= mean(midshare) - 0.1 and med(exact) <= 5.0
    return V("held" if held else "did not hold",
             f"first and last hits (2 to 80 ms from the edge): found {mean(share)*100:.0f} % against {mean(midshare)*100:.0f} % for mid-file hits of the same files; median distance of their error from the mid-file hits {med(exact):.1f} samples",
             "Hits at the given distance from the start and from the end of a 4 s file (the last hit is cut by the end of the file); found and median error (samples)\n\n"
             + md(["det", "shell", "edge ms", "first found", "first err", "last found", "last err", "mid found", "mid err"], tab))



def compute_verdicts(out=HYP):
    out = Path(out)
    rows, runs = load_rows(out)
    mrows = json.loads((out / "refine_rows.json").read_text()) if (out / "refine_rows.json").is_file() else []
    mr = [r for r in mrows if r["hyp"] == "M"]
    m3 = [r for r in mrows if r["hyp"] == "M3r"]
    v = {}
    if mr:
        v["M1"], v["M2"] = v_m1(mr), v_m2(mr)
    if m3:
        v["M3"] = v_m3(rows, m3)
    for k, fn in (("S1", v_s1), ("S2", v_s2), ("S3", v_s3), ("S4", v_s4), ("C1", v_c1), ("C3", v_c3), ("C4", v_c4),
                  ("R3", v_r3), ("R5", v_r5)):
        if any(r["hyp"] == k for r in rows):
            v[k] = fn(rows)
    if any(r["hyp"] == "R4" for r in rows):
        v["R4"] = v_r4(rows, out)
    for k, fn in (("C2", v_c2), ("R1", v_r1), ("R2", v_r2), ("R6", v_r6)):
        if any(r["hyp"] == k for r in rows):
            v[k] = fn(rows, runs)
    if any(x["hyp"] == "C5" for x in runs):
        v["C5"] = v_c5(runs, rows)
    return v


def verdict_text(v):
    s = ""
    for k in sorted(v, key=lambda k: (k[0], int(k[1:]))):
        s += f"#### {k}: {HYPOTHESES[k]}\n\n**{v[k]['verdict']}.** {v[k]['headline']}.\n\n{v[k]['table']}\n"
    return s


def verdicts(out=HYP, log=print):
    out = Path(out)
    v = compute_verdicts(out)
    (out / "verdicts.json").write_text(json.dumps({k: {"verdict": x["verdict"], "headline": x["headline"]} for k, x in v.items()}, indent=1))
    (out / "verdicts.md").write_text(verdict_text(v))
    for k in sorted(v, key=lambda k: (k[0], int(k[1:]))):
        log(f"{k}: {v[k]['verdict']}: {v[k]['headline']}")
    return v


def refine_all(out=HYP, quick=False):
    rows = gen_m_rise(out, quick) + gen_m3_refine(out, quick)
    Path(out).mkdir(parents=True, exist_ok=True)
    (Path(out) / "refine_rows.json").write_text(json.dumps(rows))
    return rows


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["generate", "run", "verdicts", "all"])
    ap.add_argument("--out", default=str(HYP))
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--jobs", type=int, default=3)
    ap.add_argument("--only", help="comma list of hypothesis ids (generators: M3e,S1..R6)")
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    only = set(a.only.split(",")) if a.only else None
    if a.cmd in ("generate", "all"):
        generate(out, a.quick, only)
        refine_all(out, a.quick)
    if a.cmd in ("run", "all"):
        run_cases(out, a.jobs, only=only)
    if a.cmd in ("verdicts", "all"):
        verdicts(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
