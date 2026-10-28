"""Media 2 (ticket #242): the licence-clean demo session generator.

    uv run python tools/media/demo_session.py --out DIR [--seed N]

Writes a deterministic multitrack drum session (13 mono 48 kHz 24-bit WAV files with a BWF
`bext` time stamp, about 60 s, tempo 96 with bar 1 two beats in) plus ``truth.json`` beside the
audio (time in samples, drum, velocity per hit, and which hits are flam, roll, ghost or part of
the tom fill). Every sound is synthesized in this script: no sample files, so no licence
question. Audio never goes into git; the default output of the media plan is
``~/.cache/drumspike/media/demo/``.

The kit: kick, snare, three toms, hi-hat, ride, crash. The mics, in the owner's name scheme:
``k in``, ``k out``, ``s top``, ``s bot``, ``t1``, ``t2``, ``t3``, ``hh``, ``ride``, ``oh l``,
``oh r``, ``room``, plus one more kick mic named ``new take`` (a name outside the scheme, so
Add Tracks shows the role suggestion from the audio).

Bleed model: each mic hears every other drum lower (1/r from the drum to that mic against the
drum's own close mic), later by the mic distance (343 m/s) and darker (a distance lowpass). The
arrangement has a two-bar kit check (bars 17 and 18: one hit per drum, 0.625 s apart) so the
bleed of every drum can be measured in isolation.
"""
import argparse
import json
import struct
import sys
from pathlib import Path

import numpy as np

SR = 48000
DEFAULT_SEED = 2026
TEMPO_BPM = 96.0
SPB = 30000                    # samples per beat at 96 BPM, 48 kHz (exact)
BAR1_BEATS = 2.0               # bar 1 does not start at 0: two beats of count-off
BARS = 24
TAIL_S = 2.5
SPEED_OF_SOUND = 343.0
PEAK = 0.891                   # the global headroom (-1 dBFS) after the mix

TRACKS = ["k in", "k out", "s top", "s bot", "t1", "t2", "t3", "hh", "ride", "oh l", "oh r", "room", "new take"]

ROLE = {"k in": "kick", "k out": "kick", "s top": "snare", "s bot": "snare", "t1": "tom", "t2": "tom",
        "t3": "tom", "hh": "hat", "ride": "cymbal", "oh l": "cymbal", "oh r": "cymbal", "room": "other",
        "new take": "kick"}

DRUMS = ["kick", "snare", "t1", "t2", "t3", "hat", "ride", "crash"]

# The close mic of each drum (the crash has none: the overheads carry it).
OWN_MIC = {"kick": "k in", "snare": "s top", "t1": "t1", "t2": "t2", "t3": "t3", "hat": "hh", "ride": "ride"}
BLEED_REF_MIC = {**OWN_MIC, "crash": "oh l"}   # the crash's bleed reference is its overhead
CLOSE_MIC_DRUM = {"k in": "kick", "k out": "kick", "s top": "snare", "s bot": "snare", "t1": "t1", "t2": "t2",
                  "t3": "t3", "hh": "hat", "ride": "ride", "new take": "kick"}

# Positions (x, y, z) in metres: the drums, then the mics. z is the height.
DRUM_POS = {
    "kick": (0.00, 0.00, 0.35),
    "snare": (0.40, 0.15, 0.60),
    "t1": (0.15, -0.35, 0.85),
    "t2": (0.65, -0.35, 0.85),
    "t3": (1.15, -0.20, 0.70),
    "hat": (-0.70, 0.10, 0.80),
    "ride": (1.30, 0.10, 1.25),
    "crash": (-0.60, -0.50, 1.25),
}
MIC_POS = {
    "k in": (0.05, 0.00, 0.35),      # inside the shell, at the beater
    "k out": (0.00, -0.35, 0.35),    # in front of the resonant head
    "s top": (0.40, 0.15, 0.65),
    "s bot": (0.40, 0.15, 0.55),
    "t1": (0.15, -0.35, 0.91),
    "t2": (0.65, -0.35, 0.91),
    "t3": (1.15, -0.20, 0.76),
    "hh": (-0.60, 0.10, 0.80),
    "ride": (1.20, 0.10, 1.25),
    "oh l": (-0.45, -0.15, 1.40),
    "oh r": (0.75, -0.05, 1.40),
    "room": (0.25, -3.00, 1.20),
    "new take": (0.22, 0.05, 0.35),  # a spare kick mic, name outside the scheme
}

# A drum's second and third mics are on the drum, so the 1/r bleed model does not apply to them.
OWN_GAIN_DB = {("kick", "k out"): -8.0, ("kick", "new take"): -6.0, ("snare", "s bot"): -6.0,
               ("crash", "oh l"): 0.0, ("crash", "oh r"): -2.0}

TOM_F0 = {"t1": 230.0, "t2": 170.0, "t3": 125.0}
TOM_DECAY = {"t1": 0.20, "t2": 0.24, "t3": 0.28}
SHOT_S = {"kick": 0.45, "snare": 0.40, "t1": 0.50, "t2": 0.50, "t3": 0.50, "hat": 0.15, "ride": 1.20, "crash": 1.80}


def _bandpass_noise(rng, n, lo, hi):
    spec = np.fft.rfft(rng.standard_normal(n))
    f = np.fft.rfftfreq(n, 1.0 / SR)
    spec[(f < lo) | (f > hi)] = 0.0
    x = np.fft.irfft(spec, n)
    return x / np.max(np.abs(x))


def _shot(drum, seed):
    """One synthesized one-shot of a drum, peak-normalized to 1.0."""
    rng = np.random.default_rng(seed)
    n = int(SHOT_S[drum] * SR)
    t = np.arange(n) / SR
    if drum == "kick":
        f = 45.0 + 150.0 * np.exp(-t / 0.025)
        ph = 2 * np.pi * np.cumsum(f) / SR
        x = np.sin(ph) * np.exp(-t / 0.11)
        x += _bandpass_noise(rng, n, 2000, 8000) * np.exp(-t / 0.002) * 0.5
    elif drum == "snare":
        tone = np.sin(2 * np.pi * 190.0 * t) * np.exp(-t / 0.07)
        noise = _bandpass_noise(rng, n, 1500, 9000) * np.exp(-t / 0.11)
        x = 0.6 * tone + 0.8 * noise
    elif drum in TOM_F0:
        f0 = TOM_F0[drum]
        f = f0 * (1.0 + 0.25 * np.exp(-t / 0.03))
        ph = 2 * np.pi * np.cumsum(f) / SR
        x = np.sin(ph) * np.exp(-t / TOM_DECAY[drum]) + 0.1 * np.sin(2.3 * ph) * np.exp(-t / 0.05)
        x += _bandpass_noise(rng, n, 800, 6500) * np.exp(-t / 0.015) * 0.6   # the stick on the head
        x += _bandpass_noise(rng, n, 100, 1200) * np.exp(-t / 0.08) * 0.12   # shell and head noise
    elif drum == "hat":
        x = _bandpass_noise(rng, n, 4000, 12000) * np.exp(-t / 0.025)
        for f0 in (3100.0, 4700.0, 6300.0):
            x += 0.3 * np.sin(2 * np.pi * f0 * t) * np.exp(-t / 0.02)
    elif drum == "ride":
        x = _bandpass_noise(rng, n, 2000, 9000) * np.exp(-t / 0.35)
        for f0 in (3300.0, 4900.0, 5700.0):
            x += 0.25 * np.sin(2 * np.pi * f0 * t) * np.exp(-t / 0.5)
    else:  # crash
        x = _bandpass_noise(rng, n, 1500, 13000) * np.exp(-t / 0.5)
        for f0 in (2800.0, 4100.0, 6800.0):
            x += 0.3 * np.sin(2 * np.pi * f0 * t) * np.exp(-t / 0.4)
    x *= np.minimum(1.0, (n - np.arange(n)) / (0.01 * SR))    # fade the last 10 ms
    return x / np.max(np.abs(x))


_LP_CACHE = {}


def _lowpassed(drum, fc):
    """The drum's shot through the distance lowpass (a 4th-order Butterworth shape), cached."""
    key = (drum, round(fc))
    if key not in _LP_CACHE:
        shot = _SHOTS[drum]
        n = len(shot) + 64
        pad = np.zeros(n)
        pad[: len(shot)] = shot
        spec = np.fft.rfft(pad)
        f = np.fft.rfftfreq(n, 1.0 / SR)
        spec *= 1.0 / (1.0 + (f / fc) ** 4)
        _LP_CACHE[key] = np.fft.irfft(spec, n)[: len(shot)]
    return _LP_CACHE[key]


def _dist(a, b):
    return float(np.linalg.norm(np.array(a) - np.array(b)))


def _bleed_db(drum, mic):
    """The bleed of `drum` into close mic `mic`, in dB under the drum's own close mic."""
    ref = _dist(DRUM_POS[drum], MIC_POS[BLEED_REF_MIC[drum]])
    return -20.0 * np.log10(_dist(DRUM_POS[drum], MIC_POS[mic]) / ref)


def _hits(seed):
    """The groove: (list of hit dicts, features dict). Times are exact samples; velocity 0..1."""
    rng = np.random.default_rng(seed + 7)
    hits = []

    def add(beats, drum, v, **flags):
        hits.append({"t": int(BAR1_BEATS * SPB) + int(round(beats * SPB)), "drum": drum, "v": round(float(v), 4),
                     "ghost": False, "flam": False, "roll": False, "tom_fill": False, **flags})

    def backbeat(b):
        add(b * 4 + 1, "snare", 0.9)
        add(b * 4 + 3, "snare", 0.9)

    def hats(b, bars):
        for i in np.arange(b * 4, (b + bars) * 4, 0.5):
            add(float(i), "hat", 0.95 if i % 1 == 0 else 0.6)

    def kick_a(b):
        add(b * 4 + 0, "kick", rng.uniform(0.85, 0.95))
        add(b * 4 + 1.5, "kick", rng.uniform(0.7, 0.85))
        add(b * 4 + 2, "kick", rng.uniform(0.8, 0.9))
        if b % 2 == 1:
            add(b * 4 + 3.5, "kick", rng.uniform(0.6, 0.75))

    def ghost(b, off, lo=0.15, hi=0.28):
        add(b * 4 + off, "snare", rng.uniform(lo, hi), ghost=True)

    # A (bars 1-8): hats, kick, backbeat snares, ghost notes.
    add(0, "crash", 0.9)
    hats(0, 8)
    for b in range(0, 8):
        kick_a(b)
        backbeat(b)
        if b in (2, 3, 5, 6, 7):
            ghost(b, 1.25)
            ghost(b, 3.75)
    # B (bars 9-15): the ride takes over; one crash on the section start. Bar 16 rests (only
    # the flam below): the ride's 1.2 s tail must be long gone before the kit check opens.
    add(8 * 4, "crash", 0.95)
    for i in np.arange(8 * 4, 15 * 4, 0.5):
        add(float(i), "ride", 0.65 if i % 1 == 0 else 0.45)
    for b in range(8, 16):
        add(b * 4 + 0, "kick", rng.uniform(0.85, 0.95))
        add(b * 4 + 2, "kick", rng.uniform(0.7, 0.85))
        backbeat(b)
        if b in (11, 13):
            ghost(b, 1.25)
            ghost(b, 3.75)
    # The flam: a grace note 18 ms before the accent, in bar 16's last beat.
    add(15 * 4 + 3, "snare", 0.9, flam=True)
    add(15 * 4 + 3 - 18.0 * SR / 1e3 / SPB, "snare", 0.35, flam=True)
    # Kit check (bars 17-18): one hit per drum, 0.625 s apart, so the bleed can be measured.
    for i, drum in enumerate(DRUMS):
        add(16 * 4 + i, drum, 0.8)
    # C (bars 19-22): hats again, busier kick, more ghosts.
    hats(18, 4)
    for b in range(18, 22):
        kick_a(b)
        backbeat(b)
        if b in (18, 19, 21):
            ghost(b, 0.75)
            ghost(b, 1.25)
            ghost(b, 3.75)
    # The fast snare roll: twelve strokes in bar 21's last beat (52 ms apart).
    for k in range(12):
        add(20 * 4 + 3 + k / 12.0, "snare", 0.25 + 0.55 * k / 11.0, roll=True)
    # The tom fill: bar 23's beats 3 and 4 on sixteenth notes, high to low.
    for k, (drum, v) in enumerate(zip(["t1", "t1", "t2", "t2", "t3", "t3", "t3", "t3"],
                                      [0.6, 0.7, 0.7, 0.8, 0.85, 0.9, 0.95, 1.0])):
        add(22 * 4 + 2 + k * 0.25, drum, v, tom_fill=True)
    # The end: crash, kick and snare together, then the last kick.
    add(23 * 4, "crash", 1.0)
    add(23 * 4, "kick", 0.95)
    add(23 * 4 + 1, "snare", 0.9)
    add(23 * 4 + 2, "kick", 1.0)
    hits.sort(key=lambda h: (h["t"], h["drum"]))
    ghosts = sum(1 for h in hits if h["ghost"])
    features = {"ghost_notes": ghosts, "flams": sum(1 for h in hits if h["flam"]) // 2, "rolls": 1, "tom_fills": 1}
    return hits, features


_SHOTS = {}


def generate(out, seed=DEFAULT_SEED, log=print):
    """Write the session (13 WAV files and truth.json) into `out`. Returns the summary dict."""
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    _SHOTS.clear()          # a new seed must not reuse the last seed's shots ...
    _LP_CACHE.clear()       # ... nor its distance-lowpassed copies
    for i, drum in enumerate(DRUMS):
        _SHOTS[drum] = _shot(drum, seed * 100 + i)
    hits, features = _hits(seed)
    n = max(h["t"] for h in hits) + int(TAIL_S * SR)
    mixes = {m: np.zeros(n + int(max(SHOT_S.values()) * SR) + 8) for m in TRACKS}
    for h in hits:
        drum, t0, v = h["drum"], h["t"], h["v"]
        for m in TRACKS:
            own_drum = CLOSE_MIC_DRUM.get(m)
            if own_drum == drum:
                x = _SHOTS[drum] * 10 ** (OWN_GAIN_DB.get((drum, m), 0.0) / 20.0)
                dist = _dist(DRUM_POS[drum], MIC_POS[m])
            else:
                dist = _dist(DRUM_POS[drum], MIC_POS[m])
                fc = 18000.0 * float(np.exp(-dist / 2.0))
                x = _lowpassed(drum, fc) * 10 ** (_bleed_db(drum, m) / 20.0)
            start = t0 + int(round(dist / SPEED_OF_SOUND * SR))
            mixes[m][start: start + len(x)] += x * v
    peak = max(float(np.max(np.abs(mixes[m]))) for m in TRACKS)
    time_ref = 3600 * SR                      # 01:00:00 since midnight, the same stamp on every track
    truth_hits = []
    for m in TRACKS:
        x = np.clip(mixes[m] * (PEAK / peak), -1.0, 1.0)
        (out / f"{m}.wav").write_bytes(_wav_bytes(x, time_ref))
        del mixes[m]
    for h in hits:
        truth_hits.append({"t": h["t"], "drum": h["drum"], "v": h["v"], "ghost": h["ghost"],
                           "flam": h["flam"], "roll": h["roll"], "tom_fill": h["tom_fill"]})
    truth = {
        "generator": "tools/media/demo_session.py",
        "licence": "synthesized in the script; no sample files, no licence question",
        "seed": seed,
        "sr": SR,
        "tempo_bpm": TEMPO_BPM,
        "samples_per_beat": SPB,
        "bar1_offset_samples": int(BAR1_BEATS * SPB),
        "duration_samples": n,
        "tracks": list(TRACKS),
        "features": features,
        "hits": truth_hits,
    }
    (out / "truth.json").write_text(json.dumps(truth, indent=1) + "\n")
    log(f"demo session: {len(TRACKS)} tracks, {n / SR:.1f} s, {len(hits)} hits, "
        f"{features['ghost_notes']} ghost notes, 1 flam, 1 roll, 1 tom fill -> {out}")
    return {"tracks": len(TRACKS), "duration_samples": n, "hits": len(hits), "features": features}


def _wav_bytes(x, time_reference):
    """A mono PCM 24-bit WAV with a `bext` chunk (EBU Tech 3285) carrying the time stamp."""
    a = np.clip(x, -1.0, 1.0)
    q = (a * 8388607.0).round().astype("<i4")
    pcm = np.ascontiguousarray(q).view(np.uint8).reshape(-1, 4)[:, :3].tobytes()
    bext = bytearray(602)

    def put(off, s, w):
        bext[off: off + min(len(s), w)] = s.encode()[:w]

    put(0, "Drum Spike demo session (synthesized, licence-clean)", 256)
    put(256, "tools/media/demo_session.py", 32)
    put(288, "DrumSpike demo", 32)
    put(320, "2026-01-01", 10)
    put(330, "00:00:00", 8)
    bext[338:346] = struct.pack("<Q", int(time_reference))
    bext[346:348] = struct.pack("<H", 1)
    fmt = struct.pack("<HHIIHH", 1, 1, SR, SR * 3, 3, 24)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt
    body += b"bext" + struct.pack("<I", len(bext)) + bytes(bext)
    body += b"data" + struct.pack("<I", len(pcm)) + pcm
    if len(pcm) % 2:
        body += b"\x00"
    return b"RIFF" + struct.pack("<I", len(body)) + body


def main(argv=None):
    ap = argparse.ArgumentParser(description="the licence-clean demo session generator (#242)")
    ap.add_argument("--out", required=True, help="output folder (audio never goes into git)")
    ap.add_argument("--seed", type=int, default=DEFAULT_SEED)
    return generate(ap.parse_args(argv).out, seed=ap.parse_args(argv).seed)


if __name__ == "__main__":
    main(sys.argv[1:])
