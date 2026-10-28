"""Write the phase 1 test corpus: synthetic kick tracks in every format the reader supports.

    uv run --project . python tools/parity/make_corpus.py <out_dir>

Audio comes from tests/helpers.py (synth_track, write_wav) so the corpus is the same signal
the pytest suite analyses; nothing is committed (the default folder is git-ignored). Writes
<out_dir>/manifest.json with one entry per file: name, sr, channels, format, header kind,
the hit onsets, the detector-style t0 list for the velocity check and the bext time
reference where one was written.

Variants: 16-bit mono and stereo (identical and different channels), 24-bit mono and
stereo, 32-bit PCM mono, float32 mono and stereo, 96 kHz 24-bit and float32, a Broadcast
Wave with bext + iXML, an extensible 24-bit stereo header, 4-channel 16-bit (default 4.0
layout), 3-channel float (2.1), a 100-sample file (shorter than one peak bucket), a
5,000-sample file (one partial 4096 bucket), and at 44.1 kHz a 24-bit stereo file with
different channels and a 16-bit mono file (added after the others, so the earlier files
keep their bytes).
"""
import json
import struct
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests"))
import helpers  # noqa: E402

SUBFORMAT_TAIL = bytes([0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71])


def encode(data, fmt):
    """Interleaved float array (frames x channels) -> sample bytes. PCM clamps and rounds."""
    c = np.clip(data, -1.0, 1.0)
    if fmt == "f32":
        return data.astype("<f4").tobytes()
    if fmt == "s16":
        return np.round(c * 32767).astype("<i2").tobytes()
    if fmt == "s24":
        q = np.round(c * 8388607).astype("<i4").tobytes()
        return b"".join(q[i:i + 3] for i in range(0, len(q), 4))
    if fmt == "s32":
        return np.round(c * 2147483647).astype("<i4").tobytes()
    raise ValueError(fmt)


BPS = {"s16": 2, "s24": 3, "s32": 4, "f32": 4}


def write_wav(path, data, sr, fmt, extensible=False, channel_mask=0, bext=None, ixml=None):
    """General writer: plain or WAVE_FORMAT_EXTENSIBLE header, optional bext and iXML chunks."""
    data = np.asarray(data, dtype=np.float64)
    if data.ndim == 1:
        data = data[:, None]
    ch = data.shape[1]
    bps = BPS[fmt]
    tag = 3 if fmt == "f32" else 1
    fmt_chunk = struct.pack("<HHIIHH", 0xFFFE if extensible else tag, ch, sr, sr * bps * ch, bps * ch, 8 * bps)
    if extensible:
        fmt_chunk += struct.pack("<HHI", 22, 8 * bps, channel_mask) + struct.pack("<H", tag) + SUBFORMAT_TAIL
    chunks = [(b"fmt ", fmt_chunk)]
    if bext is not None:
        chunks.append((b"bext", bext))
    if ixml is not None:
        chunks.append((b"iXML", ixml))
    chunks.append((b"data", encode(data, fmt)))
    body = b"WAVE"
    for cid, c in chunks:
        body += cid + struct.pack("<I", len(c)) + c + (b"\0" if len(c) % 2 else b"")
    Path(path).write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)


def bext_chunk(description, date, time, time_reference, coding_history=""):
    """EBU bext body: 602 fixed bytes plus the coding history, even length."""
    b = bytearray(602)
    b[0:len(description)] = description.encode()[:256]
    b[320:320 + len(date)] = date.encode()[:10]
    b[330:330 + len(time)] = time.encode()[:8]
    b[338:346] = struct.pack("<Q", time_reference)
    b[346:348] = struct.pack("<H", 1)
    if coding_history:
        b += coding_history.encode() + b"\0"
        if len(b) % 2:
            b += b"\0"
    return bytes(b)


def make_onsets():
    """Onsets that exercise the refinement branches later: isolated hits, hits 25 ms apart,
    quiet hits, hits near the start and the end (the spike's list)."""
    on = [0.012, 0.2, 0.5, 0.9, 1.4, 1.425, 1.45, 2.0, 2.02, 2.6, 3.1, 3.7, 4.4, 5.0, 5.03, 5.8, 6.5, 7.2, 7.95]
    gains = [-3, -6, -1, -12, -3, -3, -3, -2, -8, -30, -3, -5, -1, -3, -3, -40, -4, -6, -3]
    attacks = [0.5, 0.2, 1.0, 0.5, 0.5, 0.5, 0.5, 2.0, 0.5, 0.5, 0.5, 0.3, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5]
    return on, gains, attacks


def t0_list(onsets, dur):
    """Detector-style t0s: true onsets plus offsets, plus silence and edge cases."""
    t0s = []
    for t in onsets:
        for off in (-0.012, -0.005, 0.0, 0.0021, 0.0087, 0.03):
            t0s.append(t + off)
    t0s += [0.0, 0.001, 0.1, dur - 0.001, dur - 0.05, dur + 0.01, 3.4, 4.1]
    return sorted(set(t0s))


def stereo_lr(x, sr):
    """Different channels: R = 0.7 * L delayed by 7 samples (plus a tiny offset so the
    16-bit average rounding rule shows)."""
    right = np.concatenate([np.zeros(7), 0.7 * x[:-7]]) + 1.0 / 65536
    return np.stack([x, right], axis=1)


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).parent / "corpus"
    out.mkdir(parents=True, exist_ok=True)
    dur = 8.5
    onsets, gains, attacks = make_onsets()
    files = []

    def add(name, data, sr, fmt, channels, header="plain", t0s=None, tref=None, **kw):
        write_wav(out / name, data, sr, fmt, **kw)
        files.append({"name": name, "sr": sr, "channels": channels, "format": fmt, "header": header,
                      "onsets": list(map(float, onsets)) if t0s is not None else [],
                      "t0s": t0s if t0s is not None else [0.0, 0.0005, 0.001],
                      "time_reference": tref})

    for sr in (48000, 96000):
        x, _ = helpers.synth_track(onsets, dur, sr, gains_db=gains, attacks_ms=attacks)
        t0s = t0_list(onsets, dur)
        tag = "" if sr == 48000 else "_96k"
        if sr == 48000:
            helpers.write_wav(out / "pcm16_mono.wav", x, sr, channels=1)
            files.append({"name": "pcm16_mono.wav", "sr": sr, "channels": 1, "format": "s16", "header": "plain",
                          "onsets": list(map(float, onsets)), "t0s": t0s, "time_reference": None})
            helpers.write_wav(out / "pcm16_stereo.wav", x, sr, channels=2)
            files.append({"name": "pcm16_stereo.wav", "sr": sr, "channels": 2, "format": "s16", "header": "plain",
                          "onsets": list(map(float, onsets)), "t0s": t0s, "time_reference": None})
            add("pcm16_stereo_lr.wav", stereo_lr(x, sr), sr, "s16", 2, t0s=t0s)
            add("pcm24_stereo_lr.wav", stereo_lr(x, sr), sr, "s24", 2, t0s=t0s)
            add("pcm32_mono.wav", x, sr, "s32", 1, t0s=t0s)
            add("float32_stereo_lr.wav", stereo_lr(x, sr), sr, "f32", 2, t0s=t0s)
            tref = 48000 * 3600 * 10 + 12345
            add("bext_ixml.wav", x, sr, "s16", 1, header="bext+iXML", t0s=t0s, tref=tref,
                bext=bext_chunk("parity test", "2026-10-10", "12:34:56", tref, "A=PCM,F=48000,W=16,M=mono"),
                ixml=b"<BWFXML><IXML_VERSION>1.5</IXML_VERSION><PROJECT>drumspike</PROJECT></BWFXML>\0")
            add("extensible_pcm24_stereo.wav", stereo_lr(x, sr), sr, "s24", 2, header="extensible", t0s=t0s,
                extensible=True, channel_mask=0x3)
            quad = np.stack([x, 0.7 * x, -0.5 * x, 0.25 * x + 1.0 / 32768], axis=1)
            add("pcm16_4ch.wav", quad, sr, "s16", 4, header="extensible 4.0", t0s=t0s, extensible=True,
                channel_mask=0x107)
            three = np.stack([x, 0.6 * x, 0.9 * x], axis=1)
            add("float32_3ch.wav", three, sr, "f32", 3, header="plain 2.1", t0s=t0s)
            add("pcm16_short100.wav", x[:100], sr, "s16", 1, t0s=[0.0, 0.001])
            add("pcm16_partial5000.wav", x[:5000], sr, "s16", 1, t0s=[0.0, 0.012, 0.05])
        add(f"pcm24_mono{tag}.wav", x, sr, "s24", 1, t0s=t0s)
        add(f"float32_mono{tag}.wav", x, sr, "f32", 1, t0s=t0s)

    # 44.1 kHz (CD rate, A2-m4): the hit times are not whole multiples of 1/48000 s here
    x, _ = helpers.synth_track(onsets, dur, 44100, gains_db=gains, attacks_ms=attacks)
    t0s = t0_list(onsets, dur)
    add("pcm24_stereo_lr_44k.wav", stereo_lr(x, 44100), 44100, "s24", 2, t0s=t0s)
    add("pcm16_mono_44k.wav", x, 44100, "s16", 1, t0s=t0s)

    (out / "manifest.json").write_text(json.dumps({"files": files}, indent=1))
    print(f"{len(files)} files in {out}")


if __name__ == "__main__":
    main()
