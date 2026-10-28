"""#325: the snare hits of the demo session with their peak levels and velocities, for explainer 8.

The session is the seeded synthetic one of tools/media/demo_session.py (not a recording). It is
generated into a temporary folder once per process, the 's top' WAV is read back (24-bit PCM), and
each snare hit's level is physics.peak_db at the truth time: the same window Drum Spike's peak_in
reads. Nothing is written outside that folder.
"""
import functools
import json
import struct
import sys
import tempfile
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))  # tools/media: demo_session
import demo_session as ds  # noqa: E402
sys.path.insert(0, str(HERE))
import physics  # noqa: E402

TRACK = "s top"  # the snare's close mic: the track the explainer reads


def read_wav24(path):
    """The samples of a mono 24-bit PCM WAV as floats in -1..1 (the chunks are walked to `data`)."""
    raw = Path(path).read_bytes()
    pos = 12
    while True:
        ident = raw[pos:pos + 4]
        size = struct.unpack("<I", raw[pos + 4:pos + 8])[0]
        if ident == b"data":
            break
        pos += 8 + size + (size & 1)
    b = np.frombuffer(raw, dtype=np.uint8, count=size, offset=pos + 8).reshape(-1, 3).astype(np.int32)
    v = b[:, 0] | (b[:, 1] << 8) | (b[:, 2] << 16)
    v = np.where(v >= 1 << 23, v - (1 << 24), v)
    return v / 8388607.0  # the writer scales by 8388607 (demo_session._wav_bytes)


@functools.lru_cache(maxsize=None)
def session(seed=ds.DEFAULT_SEED):
    """(samples of TRACK, the snare rows of snare_hits): one generate per seed and process."""
    with tempfile.TemporaryDirectory(prefix="explainer-08-") as tmp:
        folder = Path(tmp)
        ds.generate(folder, seed=seed, log=lambda *_: None)
        truth = json.loads((folder / "truth.json").read_text(encoding="utf-8"))
        samples = read_wav24(folder / f"{TRACK}.wav")
    rows = []
    for h in truth["hits"]:
        if h["drum"] != "snare":
            continue
        t = h["t"] / truth["sr"]
        level = physics.peak_db(samples, t)
        rows.append({"t": t, "v": h["v"], "ghost": h["ghost"], "flam": h["flam"], "roll": h["roll"],
                     "level_db": level, "velocity": physics.velocity_of_db(level)})
    return samples, rows


def snare_hits(seed=ds.DEFAULT_SEED):
    """The snare hits in time order: {t (s), v (the truth velocity, 0..1), ghost, flam, roll,
    level_db (dBFS, physics.peak_db on TRACK), velocity (physics.velocity_of_db, default map)}."""
    return session(seed)[1]


def picked(rows):
    """The three hits explainer 8 follows: a ghost note, the first normal backbeat, and the loudest accent
    (the loudest hit that is not a flam or a roll stroke). Returns {name: row}."""
    ghost = next(r for r in rows if r["ghost"])
    normal = next(r for r in rows if not (r["ghost"] or r["flam"] or r["roll"]) and r["v"] == 0.9)
    accent = max((r for r in rows if not (r["ghost"] or r["flam"] or r["roll"])), key=lambda r: r["level_db"])
    return {"ghost": ghost, "normal": normal, "accent": accent}
