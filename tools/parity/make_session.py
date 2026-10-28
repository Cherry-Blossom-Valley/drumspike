"""Write a large synthetic session without audio in git (P3-2b).

    uv run --project . python tools/parity/make_session.py <out_dir> [--tracks 30] [--seconds 300] [--sr 96000]

Writes N mono 24-bit WAV files (default 30 tracks, 96 kHz, 5 minutes: 96,000 x 300 x 3 =
86.4 MB of data each) and <out_dir>/session.json with the known hit times of every track.
The folder must be outside the repository (the script refuses a folder inside it).
A track is a low noise floor with a kick-like hit (pitch sweep 125 to 55 Hz, 0.5 ms attack,
exponential decay) at each hit time; the hit spacing and offset differ per track, with a
fixed seed, so two runs give the same bytes. Hit times are in samples and in seconds.
"""
import argparse
import json
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]


def kick(n, sr, gain):
    t = np.arange(n) / sr
    f = 55 + 70 * np.exp(-t / 0.03)
    ph = 2 * np.pi * np.cumsum(f) / sr
    env = np.minimum(1.0, t / 0.0005) * np.exp(-t / 0.12)
    return (np.sin(ph) * env * gain).astype(np.float32)


def write_wav24(path, x, sr):
    """Mono 24-bit PCM WAV, streamed in blocks so the peak memory stays small."""
    n = len(x)
    data_bytes = n * 3
    hdr = b"RIFF" + (36 + data_bytes).to_bytes(4, "little") + b"WAVEfmt " + (16).to_bytes(4, "little")
    hdr += (1).to_bytes(2, "little") + (1).to_bytes(2, "little") + sr.to_bytes(4, "little")
    hdr += (sr * 3).to_bytes(4, "little") + (3).to_bytes(2, "little") + (24).to_bytes(2, "little")
    hdr += b"data" + data_bytes.to_bytes(4, "little")
    with open(path, "wb") as f:
        f.write(hdr)
        block = 1 << 20
        for i in range(0, n, block):
            q = np.round(np.clip(x[i:i + block], -1.0, 1.0) * 8388607).astype("<i4")
            raw = q.view(np.uint8).reshape(-1, 4)[:, :3]
            f.write(raw.tobytes())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--tracks", type=int, default=30)
    ap.add_argument("--seconds", type=float, default=300.0)
    ap.add_argument("--sr", type=int, default=96000)
    a = ap.parse_args()
    out = Path(a.out).resolve()
    if ROOT in out.parents or out == ROOT:
        sys.exit(f"refusing to write into the repository: {out}")
    out.mkdir(parents=True, exist_ok=True)
    n = int(a.seconds * a.sr)
    manifest = {"sr": a.sr, "seconds": a.seconds, "tracks": []}
    for k in range(a.tracks):
        rng = np.random.default_rng(1000 + k)
        x = (rng.standard_normal(n) * 10 ** (-60 / 20)).astype(np.float32)
        step = 0.4 + 0.02 * (k % 10)
        first = 0.5 + 0.013 * k
        times = [round(first + i * step, 6) for i in range(int((a.seconds - first - 0.6) / step))]
        samples = [int(round(t * a.sr)) for t in times]
        h = kick(int(0.4 * a.sr), a.sr, 10 ** (-3 / 20))
        for s in samples:
            x[s:s + len(h)] += h[: n - s]
        name = f"t{k + 1:02d}.wav"
        write_wav24(out / name, x, a.sr)
        manifest["tracks"].append({"file": name, "hit_samples": samples, "hit_times_s": times})
        print(name, len(samples), "hits", flush=True)
    (out / "session.json").write_text(json.dumps(manifest))


if __name__ == "__main__":
    main()
