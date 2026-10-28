"""MIDI and spike WAV export parity: drumspike-core (parity binary) vs the frozen engine.export_midi.

    cargo build --release -p drumspike-devtools
    uv run --project . python tools/parity/check_midi.py [corpus_dir] [--json out.json] [--bin path]

MIDI cases: the refined hits of every corpus file (engine._refine_sr + velocity_map on the
manifest's t0s, note 36), plus synthetic event lists: 500 random hits in 20 s, hits 10, 25
and 50 ms apart (overlapping notes, a note off and a note on on the same tick), edge
values (t = 0, t < 0, missing v, v = 0 and 127, notes 0 and 127), half-tick ties, a
200 s span (three-byte deltas; pretty_midi refuses files beyond 1e7 ticks), an empty list, and other ppq/tempo pairs.

P3-12: the references are frozen in tests/golden/frozen/: the refined corpus hits of the retired
server in corpus_events.json, and per event list the file engine.export_midi (pretty_midi) wrote,
as hex, with its note messages as mido read them, in midi.json. `parity midi` writes the Rust
file; the check compares the bytes and compares the Rust note tuples (tick, note, velocity,
duration), expanded to note on and note off messages, with the note messages of the Python file.

Spike cases: `parity spike` writes the spike file for the refined hits of a corpus file with
and without a bext chunk and at 96 kHz; Python's wave module checks rate, channels, width,
length, the full-scale sample at every hit index, silence before it and after the 1 ms
blip; the bext time reference is read from the chunk and compared with the source's, and
the Rust reader (`parity <spike.wav>`) decodes the written file to the same samples.

Exit status 0 when every case passes (identical events; the byte comparison is reported,
a difference there is a failure too unless --allow-layout is given).
"""
import argparse
import hashlib
import json
import struct
import subprocess
import sys
import wave
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "tests"))
sys.path.insert(0, str(HERE))
import frozen  # noqa: E402

DEFAULT_BIN = ROOT / "target" / "release" / "parity"
MIDI = frozen.store("midi")
EVENTS = frozen.store("corpus_events")


def corpus_events(corpus):
    """{file name: (manifest entry, sr, n_samples, refined events)}; the events are the Python server's
    (frozen in tests/golden/frozen/corpus_events.json)."""
    manifest = corpus / "manifest.json"
    if not manifest.exists():
        subprocess.run([sys.executable, str(HERE / "make_corpus.py"), str(corpus)], check=True)
    entries = json.loads(manifest.read_text())["files"]
    assert isinstance(entries, list) and entries, type(entries)
    out = {}
    for entry in entries:
        wav = corpus / entry["name"]
        t0s = [float(t) for t in entry["t0s"]]
        inputs = {"wav_sha256": hashlib.sha256(wav.read_bytes()).hexdigest(), "t0s": t0s}
        r = EVENTS.ref(entry["name"], inputs, canonical_host=True)
        out[entry["name"]] = (entry, r["sr"], r["n_samples"], r["events"])
    return out


def midi_cases(corpus_ev):
    rng = np.random.default_rng(42)
    for name, (entry, sr, n_samples, ev) in corpus_ev.items():
        yield f"corpus {name}", ev, 36, 24000, 120.0
    yield "dense 500 random hits", [{"t": float(t), "v": int(v)} for t, v in
                                   zip(sorted(rng.uniform(0.1, 19.9, 500)), rng.integers(1, 128, 500))], 36, 24000, 120.0
    yield "overlapping notes 10/25/50 ms", [{"t": 1.0, "v": 100}, {"t": 1.01, "v": 90}, {"t": 1.035, "v": 80},
                                           {"t": 1.085, "v": 70}, {"t": 1.135, "v": 60}, {"t": 1.185, "v": 127},
                                           {"t": 1.185, "v": 50}], 38, 24000, 120.0
    yield "edge values", [{"t": 0.0, "v": 1}, {"t": -0.01, "v": 5}, {"t": 0.5}, {"t": 0.6, "v": 0},
                          {"t": 0.7, "v": 127}, {"t": 0.3, "v": 64}], 0, 24000, 120.0
    yield "note 127", [{"t": 0.25, "v": 100}], 127, 24000, 120.0
    yield "half-tick ties", [{"t": (k + 0.5) / 48000, "v": 100} for k in (1000, 1001, 2400, 2401, 33333, 33334)], 36, 24000, 120.0
    yield "200-second span", [{"t": 20.0 * k + 0.123456789, "v": 100} for k in range(10)] + [{"t": 199.9, "v": 100}], 36, 24000, 120.0
    yield "empty", [], 36, 24000, 120.0
    yield "ppq 480 tempo 90", [{"t": t, "v": 100} for t in (0.1, 0.1234, 0.5, 1.0, 2.5)], 42, 480, 90.0
    yield "ppq 960 tempo 174.3", [{"t": t, "v": 77} for t in (0.1, 0.1234, 0.5, 1.0, 2.5)], 46, 960, 174.3
    yield "ppq 24000 tempo 100", [{"t": t, "v": 77} for t in (0.1, 0.1234, 0.5, 1.0, 2.5)], 49, 24000, 100.0
    yield "ppq 32767 tempo 300", [{"t": t, "v": 77} for t in (0.1, 0.1234, 0.5)], 51, 32767, 300.0


def midi_reference(name, events, note, ppq, tempo):
    """The frozen file of the retired server for one event list: {hex, notes}."""
    inputs = {"events": events, "note": note, "ppq": ppq, "tempo": tempo}
    return MIDI.ref(name, inputs)


def check_midi_case(name, events, note, ppq, tempo, bin_path, scratch, allow_layout):
    """The Rust file against the frozen Python file: identical bytes, and the Rust note tuples
    (tick, note, velocity, duration), expanded to note on and note off messages, equal the note
    messages of the Python file (identical bytes imply identical pretty_midi and mido readings)."""
    rs = scratch / "rs.mid"
    job = scratch / "midi_job.json"
    ref = midi_reference(name, events, note, ppq, tempo)
    job.write_text(json.dumps({"events": events, "note": note, "ppq": ppq, "tempo": tempo}))
    r = subprocess.run([str(bin_path), "midi", str(job), str(rs)], capture_output=True, text=True)
    if r.returncode != 0:
        return False, [f"parity midi failed: {r.stderr[-400:]}"], {}
    d = json.loads(r.stdout)
    assert isinstance(d, dict) and "notes" in d, list(d)[:5]
    rs_msgs = sorted([[n["tick"], n["note"], n["velocity"]] for n in d["notes"]]
                     + [[n["tick"] + n["duration"], n["note"], 0] for n in d["notes"]])
    problems = []
    pb, rb = bytes.fromhex(ref["hex"]), rs.read_bytes()
    identical = pb == rb
    if not identical:
        first = next((i for i, (a, b) in enumerate(zip(pb, rb)) if a != b), min(len(pb), len(rb)))
        msg = f"bytes differ: python {len(pb)} B, rust {len(rb)} B, first difference at offset {first}"
        if allow_layout:
            print(f"      note: {msg}")
        else:
            problems.append(msg)
    if ref["notes"] != rs_msgs:
        problems.append(f"note tuples differ: python file {ref['notes'][:3]} rust tuples {rs_msgs[:3]}")
    rec = {"events": len(events), "bytes_identical": identical, "py_bytes": len(pb), "rs_bytes": len(rb),
           "notes_identical": ref["notes"] == rs_msgs, "problems": problems}
    return not problems, problems, rec


def bext_time_reference(path):
    """TimeReference of the bext chunk, or None when the file has none."""
    b = Path(path).read_bytes()
    pos = 12
    while pos + 8 <= len(b):
        cid, size = b[pos:pos + 4], struct.unpack("<I", b[pos + 4:pos + 8])[0]
        if cid == b"bext":
            return struct.unpack("<Q", b[pos + 8 + 338:pos + 8 + 346])[0]
        pos += 8 + size + (size % 2)
    return None


def spike_cases(corpus_ev):
    for name in ("bext_ixml.wav", "pcm16_mono.wav", "float32_mono_96k.wav", "pcm16_short100.wav"):
        if name in corpus_ev:
            entry, sr, n_samples, ev = corpus_ev[name]
            yield name, entry, sr, n_samples, [e["t"] for e in ev]


def check_spike_case(name, entry, sr, n_samples, times, corpus, bin_path, scratch):
    out = scratch / f"{Path(name).stem}_spikes.wav"
    job = scratch / "spike_job.json"
    job.write_text(json.dumps({"source": str(corpus / name), "times": times, "format": "pcm16"}))
    r = subprocess.run([str(bin_path), "spike", str(job), str(out)], capture_output=True, text=True)
    if r.returncode != 0:
        return False, [f"parity spike failed: {r.stderr[-400:]}"], {}
    d = json.loads(r.stdout)
    assert isinstance(d, dict) and {"sr", "n_samples"} <= set(d), list(d)[:5]
    problems = []
    with wave.open(str(out), "rb") as w:
        if (w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes()) != (1, 2, sr, n_samples):
            problems.append(f"wave header {(w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes())}"
                            f" != {(1, 2, sr, n_samples)}")
        pcm = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2")
    idx = sorted({int(round(t * sr)) for t in times if 0 <= round(t * sr) < n_samples})
    blip = max(2, int(round(0.001 * sr)))
    n_full = int((pcm == 32767).sum())
    if n_full != len(idx):
        problems.append(f"{n_full} full-scale samples, {len(idx)} hits")
    for i in idx:
        if pcm[i] != 32767:
            problems.append(f"hit at {i}: sample {pcm[i]} != 32767")
        if i > 0 and pcm[i - 1] != 0 and (i - 1) not in idx and not any(0 <= i - 1 - j < blip for j in idx):
            problems.append(f"hit at {i}: sample before is {pcm[i - 1]}")
        if i + blip < n_samples and pcm[i + blip] != 0 and not any(0 <= i + blip - j < blip for j in idx):
            problems.append(f"hit at {i}: sample after the blip is {pcm[i + blip]}")
        if i + 1 < n_samples and not (0 < pcm[i + 1] < 32767):
            problems.append(f"hit at {i}: decay sample {pcm[i + 1]}")
    tref = bext_time_reference(out)
    if tref != entry.get("time_reference"):
        problems.append(f"bext time reference {tref} != source {entry.get('time_reference')}")
    # the Rust reader decodes the written file to the same samples as the wave module
    r2 = subprocess.run([str(bin_path), str(out), "--pcm-out", str(scratch / "spikes_back.f32")],
                        capture_output=True, text=True)
    if r2.returncode != 0:
        problems.append(f"rust reader failed on the spike file: {r2.stderr[-300:]}")
    else:
        back = np.fromfile(scratch / "spikes_back.f32", dtype=np.float32)
        info = json.loads(r2.stdout)["info"]
        if len(back) != n_samples or not np.array_equal(back, (pcm.astype(np.float32) / 32768.0)):
            problems.append("rust reader: samples differ from the wave module's")
        if (info.get("bext") or {}).get("time_reference") != entry.get("time_reference"):
            problems.append("rust reader: bext time reference differs")
    rec = {"hits": len(idx), "n_samples": n_samples, "sr": sr, "time_reference": tref, "problems": problems}
    return not problems, problems, rec


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("corpus", nargs="?", type=Path, default=HERE / "corpus")
    ap.add_argument("--bin", type=Path, default=DEFAULT_BIN)
    ap.add_argument("--json", type=Path, default=None)
    ap.add_argument("--allow-layout", action="store_true", help="report byte differences without failing")
    args = ap.parse_args(argv)
    if not args.bin.exists():
        print(f"parity binary missing: {args.bin} (cargo build --release -p drumspike-devtools)")
        return 2
    args.corpus.mkdir(parents=True, exist_ok=True)
    scratch = args.corpus / "_midi"
    scratch.mkdir(exist_ok=True)
    corpus_ev = corpus_events(args.corpus)
    results = {"midi": {}, "spike": {}}
    n_ok = total = 0
    n_bytes = n_cases_midi = 0
    print(f"{'MIDI case':44s} {'events':>6s} {'bytes':>5s} {'py B':>6s} {'rs B':>6s}  notes")
    for name, events, note, ppq, tempo in midi_cases(corpus_ev):
        ok, problems, rec = check_midi_case(name, events, note, ppq, tempo, args.bin, scratch, args.allow_layout)
        results["midi"][name] = rec
        n_ok += ok
        total += 1
        n_cases_midi += 1
        n_bytes += bool(rec.get("bytes_identical"))
        print(f"{'PASS' if ok else 'FAIL'} {name:39s} {rec.get('events', 0):6d} {'same' if rec.get('bytes_identical') else 'DIFF':>5s} "
              f"{rec.get('py_bytes', 0):6d} {rec.get('rs_bytes', 0):6d}  {'identical' if rec.get('notes_identical') else 'DIFFER'}")
        for p in problems[:4]:
            print(f"      {p}")
    print(f"\n{'Spike case':44s} {'hits':>6s} {'samples':>8s} {'sr':>6s}  time reference")
    for name, entry, sr, n_samples, times in spike_cases(corpus_ev):
        ok, problems, rec = check_spike_case(name, entry, sr, n_samples, times, args.corpus, args.bin, scratch)
        results["spike"][name] = rec
        n_ok += ok
        total += 1
        print(f"{'PASS' if ok else 'FAIL'} {name:39s} {rec.get('hits', 0):6d} {rec.get('n_samples', 0):8d} {rec.get('sr', 0):6d}  {rec.get('time_reference')}")
        for p in problems[:4]:
            print(f"      {p}")
    print(f"\n{n_ok}/{total} cases PASS; MIDI files byte-identical {n_bytes}/{n_cases_midi}")
    if args.json:
        args.json.write_text(json.dumps({"cases": results, "pass": n_ok, "total": total,
                                         "midi_bytes_identical": n_bytes, "midi_cases": n_cases_midi}, indent=1))
    return 0 if n_ok == total else 1


if __name__ == "__main__":
    sys.exit(main())
