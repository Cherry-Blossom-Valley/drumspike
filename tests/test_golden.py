"""The golden regression: the Rust core against the frozen outputs of the retired Python server.

One corpus for every group: the WAVs are written again from the same deterministic generators
(golden_corpus.write_corpus; no audio in the repo), and their SHA-256 must match the golden and
frozen inputs first, so a changed generator shows up as such and not as a regression. The golden
and frozen files cannot be written again (engine.py is gone). Groups:

  golden   tests/golden/*.json (P1-8): the release `parity` binary per WAV and mix mode: decode
           bit-exact, the min and max of every peak level exact as integers of 1e-5, the RMS within
           one step (P1-3), refined times identical, rise within 1e-9 dB, velocities identical, the
           MIDI file byte-identical.
  io       frozen/parity_io.json: header, decode, peaks, tile and sample windows (Track.peaks and
           Track.samples with the ensureSamples clamp) and the velocities at the t0 list.
  refine   frozen/refine.json: per hit of the corpus, edge, window and bench cases the refined time
           (same float) and rise (within 1e-9 dB), and the velocities (P1-4, P2-9).
  midi     frozen/midi.json and corpus_events.json: the MIDI file of every corpus event list and of
           11 synthetic lists byte-identical, its note messages equal (P1-5); spike WAVs with a
           full-scale blip per hit and the source's bext time reference, read back.
  analyze  frozen/rust_analyze.json: the events of `service_tool core-analyze` (detect_events and
           analyze_detected of the app's job, P3-2a) identical, its branch counts, the rule option.

Refactor 1.5 (T8) moved the io, refine and midi cases here from tools/parity/check_*.py and the
analyze cases from tests/test_rust_analyze.py. Every group has a planted-fault test: its comparison
must fail on one changed value, so a pass is not a filter that cannot fire.
"""
import hashlib
import json
import os
import wave

import numpy as np
import pytest

import frozen
import golden_corpus
from core_tools import core_analyze
from golden_corpus import GOLDEN, PARITY_BIN, manifest, modes, run_parity, rust_outputs, sha256
from helpers import SR, synth_track, write_wav

GOLDEN_FILES = sorted(GOLDEN.glob("*.json"))
IO = frozen.store("parity_io", "check_parity.py: per corpus file and mix mode the Python server's decode "
                  "(SHA-256), peaks (integers of 1e-5), tile and sample windows and velocities at the t0 list")
REFINE = frozen.store("refine")
MIDI = frozen.store("midi")
EVENTS = frozen.store("corpus_events")
ANALYZE = frozen.store("rust_analyze")


def need_parity():
    if not PARITY_BIN.exists():
        if os.environ.get("DRUMSPIKE_REQUIRE_CORE") == "1":
            pytest.fail(f"DRUMSPIKE_REQUIRE_CORE=1 and {PARITY_BIN} is missing (cargo build --release -p drumspike-devtools)")
        pytest.skip(f"{PARITY_BIN} missing: cargo build --release -p drumspike-devtools")


@pytest.fixture(scope="module")
def written(tmp_path_factory):
    """(folder, {name: (t0s, channels)}) of the written corpus; needs no Rust tool."""
    out = tmp_path_factory.mktemp("golden_corpus")
    return out, {name: (t0s, channels) for name, t0s, channels in golden_corpus.write_corpus(out)}


@pytest.fixture(scope="module")
def corpus(written):
    need_parity()
    return written


def flip(digest):
    return ("1" if digest[0] == "0" else "0") + digest[1:]


# ---------------------------------------------------------------------------------------------
# golden: tests/golden/*.json

def test_every_wav_has_a_golden_file(corpus):
    _, files = corpus
    assert len(GOLDEN_FILES) == len(files) >= 20
    assert sorted(json.loads(p.read_text())["file"] for p in GOLDEN_FILES) == sorted(files)


def check(got, exp, where):
    """One WAV and mix mode against its golden values; raises AssertionError on the first difference."""
    assert (got["sr"], got["n_samples"]) == (exp["sr"], exp["n_samples"]), where
    assert got["signal_sha256"] == exp["signal_sha256"], f"{where}: decode"
    assert list(got["peaks"]) == list(exp["peaks"]), where
    for bs, lvl in exp["peaks"].items():
        g = got["peaks"][bs]
        assert g["n"] == lvl["n"], f"{where} level {bs}"
        assert (g["min_sha256"], g["max_sha256"]) == (lvl["min_sha256"], lvl["max_sha256"]), f"{where} level {bs}"
        assert len(g["rms"]) == len(lvl["rms"]), f"{where} level {bs}"
        assert max((abs(a - b) for a, b in zip(g["rms"], lvl["rms"])), default=0) <= 1, f"{where} level {bs} rms"
    assert [h["t"] for h in got["hits"]] == [h["t"] for h in exp["hits"]], f"{where}: refined times"
    for g, e in zip(got["hits"], exp["hits"]):
        assert (g["rise_db"] is None) == (e["rise_db"] is None), where
        if e["rise_db"] is not None:
            assert abs(g["rise_db"] - e["rise_db"]) <= 1e-9, f"{where}: rise"
    assert got["velocities"] == exp["velocities"], f"{where}: velocities"
    assert got["midi"]["notes"] == exp["midi"]["notes"], f"{where}: MIDI notes"
    assert got["midi"]["sha256"] == exp["midi"]["sha256"], f"{where}: MIDI bytes"


@pytest.mark.parametrize("golden", GOLDEN_FILES, ids=lambda p: p.stem)
def test_outputs_match_the_golden_file(golden, corpus):
    out, files = corpus
    want = json.loads(golden.read_text())
    name = want["file"]
    assert sha256((out / name).read_bytes()) == want["wav_sha256"], "corpus generator changed"
    t0s, channels = files[name]
    assert t0s == want["t0s"] and sorted(want["modes"]) == sorted(modes(channels))
    for mode, exp in want["modes"].items():
        check(rust_outputs(out / name, t0s, mode), exp, f"{name} {mode}")


def test_a_planted_change_fails_the_golden_comparison(corpus):
    """P3-12: the comparison can fail. One changed refined time, one changed velocity, one changed MIDI
    byte digest and one changed decode digest each raise."""
    out, files = corpus
    want = json.loads((GOLDEN / "pcm16_mono.json").read_text())
    t0s, _ = files["pcm16_mono.wav"]
    got = rust_outputs(out / "pcm16_mono.wav", t0s, "ffmpeg")
    exp = want["modes"]["ffmpeg"]
    check(got, exp, "unchanged")
    for plant in ("time", "velocity", "midi", "decode"):
        bad = json.loads(json.dumps(exp))
        if plant == "time":
            bad["hits"][3]["t"] += 1 / 48000
        elif plant == "velocity":
            bad["velocities"][3] = (bad["velocities"][3] % 127) + 1
        elif plant == "midi":
            bad["midi"]["sha256"] = flip(bad["midi"]["sha256"])
        else:
            bad["signal_sha256"] = flip(bad["signal_sha256"])
        with pytest.raises(AssertionError):
            check(got, bad, plant)


# ---------------------------------------------------------------------------------------------
# io: decode, header, peaks, tile and sample windows, velocities at the t0 list (frozen/parity_io.json)

def q5(values):
    """Peak values (rounded to 5 decimals by both sides) as integers of 1e-5."""
    return [int(round(v * 1e5)) for v in values]


def f32_sha(a):
    return hashlib.sha256(np.asarray(a, dtype=np.float32).tobytes()).hexdigest()


def within_one(a, b):
    return len(a) == len(b) and all(abs(u - v) <= 1 for u, v in zip(a, b))


def io_case(entry, wav, mode, scratch):
    """(reference, Rust answer, Rust decode) of one corpus file and mix mode. The reference is
    engine.load_f32, engine.compute_peaks and engine.velocity_map of the retired server."""
    inputs = {"wav_sha256": sha256(wav.read_bytes()), "mode": mode, "t0s": entry["t0s"]}
    ref = IO.ref(f"{entry['name']} [{mode}]", inputs, canonical_host=True)
    pcm_out = scratch / f"{wav.stem}.{mode}.f32"
    t0file = pcm_out.with_suffix(".t0.json")
    t0file.write_text(json.dumps([float(t) for t in entry["t0s"]]))
    args = [wav, "--mix", mode, "--pcm-out", pcm_out, "--t0-file", t0file]
    for t in ref["tiles"]:
        args += ["--tile", f"{t['level']},{t['from']},{t['to']}"]
    for w in ref["samples"]:
        args += ["--samples", f"{w['from']},{w['to']}"]
    rs = run_parity(*args)
    assert {"info", "peaks", "velocities", "tiles", "samples"} <= set(rs), list(rs)[:5]
    return ref, rs, np.fromfile(pcm_out, dtype=np.float32)


def io_problems(entry, ref, rs, y):
    """Decode, header and sample windows exact, min and max exact as integers of 1e-5, RMS within one
    step of 1e-5 (P1-3), velocities exact."""
    problems = []
    if len(y) != ref["n_samples"]:
        problems.append(f"decode: {len(y)} samples, reference {ref['n_samples']}")
    elif f32_sha(y) != ref["decode_sha256"]:
        problems.append("decode: the samples differ from the reference (SHA-256)")
    info = rs["info"]
    if info["sample_rate"] != ref["sr"]:
        problems.append(f"sr {info['sample_rate']} != {ref['sr']}")
    if info["channels"] != entry["channels"]:
        problems.append(f"channels {info['channels']} != {entry['channels']}")
    if rs["n_samples"] != ref["n_samples"]:
        problems.append(f"n_samples {rs['n_samples']} != {ref['n_samples']}")
    tref = (info.get("bext") or {}).get("time_reference")
    if tref != entry.get("time_reference"):
        problems.append(f"time_reference {tref} != {entry.get('time_reference')}")
    for bs, lvl in ref["peaks"].items():
        r = rs["peaks"].get(bs)
        if r is None or r["n"] != lvl["n"]:
            problems.append(f"level {bs}: n {r and r['n']} != {lvl['n']}")
            continue
        for key in ("min", "max", "rms"):
            got = q5(r[key])
            if not (got == lvl[key] or (key == "rms" and within_one(got, lvl[key]))):
                problems.append(f"level {bs} {key}: values differ from the reference")
    if len(rs["tiles"]) != len(ref["tiles"]) or len(rs["samples"]) != len(ref["samples"]):
        problems.append(f"{len(rs['tiles'])} tiles, {len(rs['samples'])} sample windows")
    for want, t in zip(ref["tiles"], rs["tiles"]):
        where = f"tile {want['level']},{want['from']},{want['to']}"
        if t is None:
            problems.append(f"{where}: missing level")
            continue
        if t["start"] != want["start"]:
            problems.append(f"{where}: rust start {t['start']}, reference {want['start']}")
            continue
        for key in ("min", "max", "rms"):
            got = q5(t[key])
            if not (got == want[key] or (key == "rms" and within_one(got, want[key]))):
                problems.append(f"{where} {key}: rust len {len(got)}, reference {want['start']}:{want['end']}")
                break
    for want, w in zip(ref["samples"], rs["samples"]):
        got = np.asarray(w["data"], dtype=np.float32)
        if w["start"] != want["start"] or len(got) != want["end"] - want["start"] or f32_sha(got) != want["sha256"]:
            problems.append(f"samples {want['from']},{want['to']}: rust start {w['start']} len {len(got)}, "
                            f"reference {want['start']}:{want['end']}")
    if rs["velocities"] != ref["velocities"]:
        problems.append(f"velocities: rust {rs['velocities'][:5]}, reference {ref['velocities'][:5]}")
    return problems


def test_io_matches_the_frozen_server(corpus, tmp_path):
    """Every corpus file in both mix modes (36 cases, every key of frozen/parity_io.json)."""
    out, _ = corpus
    seen, failed = [], {}
    for entry in manifest(out):
        for mode in ("ffmpeg", "average"):
            ref, rs, y = io_case(entry, out / entry["name"], mode, tmp_path)
            seen.append(f"{entry['name']} [{mode}]")
            if problems := io_problems(entry, ref, rs, y):
                failed[seen[-1]] = problems[:4]
    assert not failed, failed
    assert sorted(seen) == sorted(IO.doc["entries"]), "a frozen io case is not read"


def test_a_planted_change_fails_the_io_comparison(corpus, tmp_path):
    out, _ = corpus
    entry = next(e for e in manifest(out) if e["name"] == "bext_ixml.wav")
    ref, rs, y = io_case(entry, out / entry["name"], "ffmpeg", tmp_path)
    assert io_problems(entry, ref, rs, y) == []
    for plant in ("decode", "velocity", "peak", "tile", "samples", "time_reference"):
        bad, bad_entry = json.loads(json.dumps(ref)), dict(entry)
        if plant == "decode":
            bad["decode_sha256"] = flip(bad["decode_sha256"])
        elif plant == "velocity":
            bad["velocities"][0] = (bad["velocities"][0] % 127) + 1
        elif plant == "peak":
            lvl = next(iter(bad["peaks"].values()))
            lvl["max"][0] += 1
        elif plant == "tile":
            bad["tiles"][0]["min"][0] += 1
        elif plant == "samples":
            bad["samples"][0]["sha256"] = flip(bad["samples"][0]["sha256"])
        else:
            bad_entry["time_reference"] = (entry["time_reference"] or 0) + 1
        assert io_problems(bad_entry, bad, rs, y), plant


# ---------------------------------------------------------------------------------------------
# refine: refined time and rise per hit, velocities (frozen/refine.json)

@pytest.fixture(scope="module")
def refine_cases(corpus):
    out, _ = corpus
    scratch = out / "_refine"
    scratch.mkdir(exist_ok=True)
    return {"corpus": list(golden_corpus.refine_corpus_cases(out)),
            "edge": list(golden_corpus.refine_edge_cases(scratch)),
            "window": list(golden_corpus.refine_window_cases(out, scratch)),
            "bench": list(golden_corpus.refine_bench_cases(scratch))}


def refine_case(name, wav, mode, hits, scratch, window=None):
    """(reference {sr, t, rise, v}, Rust answer) of one case. The reference is engine._refine_sr per hit
    and engine.velocity_map of the retired server; its inputs are the SHA-256 of the WAV file, the mix
    mode, the hits and the window."""
    inputs = {"wav_sha256": sha256(wav.read_bytes()), "mode": mode,
              "hits": [[t0, list(nb)] for t0, nb in hits], "window": list(window) if window else None}
    ref = REFINE.ref(name, inputs, canonical_host=True)
    job = {"wav": str(wav), "mix": mode, "hits": [{"t0": t0, "neighbors": nb} for t0, nb in hits]}
    if window is not None:
        job["window"] = list(window)
    (scratch / "job.json").write_text(json.dumps(job))
    rs = run_parity("refine", scratch / "job.json")
    assert {"sr", "refined", "velocities"} <= set(rs), list(rs)[:5]
    return ref, rs


def refine_problems(hits, ref, rs):
    """Every hit identical: the same time float and rise within 1e-9 dB (or None on both sides);
    every velocity exact (parity-contract.md)."""
    problems = []
    if rs["sr"] != ref["sr"]:
        problems.append(f"sr {rs['sr']} != {ref['sr']}")
    if len(rs["refined"]) != len(hits):
        return problems + [f"{len(rs['refined'])} refined hits, reference {len(hits)}"]
    for k, ((t0, _), rr) in enumerate(zip(hits, rs["refined"])):
        rt, rrise, pt, prise = rr["t"], rr["rise"], ref["t"][k], ref["rise"][k]
        rise_same = (rrise is None and prise is None) or (
            rrise is not None and prise is not None and abs(rrise - prise) <= 1e-9)
        if rt != pt or not rise_same:
            problems.append(f"hit {k} t0={t0!r}: reference {pt!r} rust {rt!r} "
                            f"({(rt - pt) * ref['sr']:+.3f} samples), rise reference {prise} rust {rrise}")
    n_vel = sum(int(a == b) for a, b in zip(ref["v"], rs["velocities"]))
    if n_vel != len(hits):
        problems.append(f"velocities: {len(hits) - n_vel} of {len(hits)} differ")
    return problems


@pytest.mark.parametrize("group", ["corpus", "edge", "window", "bench"])
def test_refinement_matches_the_frozen_server(group, refine_cases, tmp_path):
    """corpus: every file, dedup and isolated hit lists; edge: flams and rolls 4 to 13 ms apart,
    one-sided neighbours, 5-, 8- and 12-sample files, hits outside the file; window: the search window
    as a job key (P2-9); bench: the 16 scenarios of the old refinement benchmark."""
    failed, n_hits = {}, 0
    for name, wav, mode, hits, *win in refine_cases[group]:
        ref, rs = refine_case(name, wav, mode, hits, tmp_path, *win)
        n_hits += len(hits)
        if problems := refine_problems(hits, ref, rs):
            failed[name] = problems[:4]
    assert not failed, failed
    assert n_hits >= {"corpus": 1000, "edge": 40, "window": 100, "bench": 2400}[group], n_hits


def test_every_frozen_refine_case_is_read(refine_cases):
    names = [c[0] for cases in refine_cases.values() for c in cases]
    assert len(names) == len(set(names)) == 90
    assert sorted(names) == sorted(REFINE.doc["entries"])


def test_a_planted_change_fails_the_refine_comparison(refine_cases, tmp_path):
    name, wav, mode, hits = refine_cases["edge"][0]
    ref, rs = refine_case(name, wav, mode, hits, tmp_path)
    assert refine_problems(hits, ref, rs) == []
    k = next(i for i, r in enumerate(ref["rise"]) if r is not None)
    for plant in ("time", "rise", "velocity", "count"):
        bad, bad_hits = json.loads(json.dumps(ref)), hits
        if plant == "time":
            bad["t"][k] += 1 / bad["sr"]
        elif plant == "rise":
            bad["rise"][k] += 1e-6
        elif plant == "velocity":
            bad["v"][k] = (bad["v"][k] % 127) + 1
        else:
            bad_hits = hits[:-1]
        assert refine_problems(bad_hits, bad, rs), plant


# ---------------------------------------------------------------------------------------------
# midi: MIDI bytes and note messages (frozen/midi.json, corpus_events.json); spike WAVs

@pytest.fixture(scope="module")
def corpus_events(corpus):
    """{file name: (manifest entry, sr, n_samples, refined events)}; the events are engine._refine_sr and
    velocity_map of the retired server on the manifest's t0s (frozen/corpus_events.json)."""
    out, _ = corpus
    got = {}
    for entry in manifest(out):
        inputs = {"wav_sha256": sha256((out / entry["name"]).read_bytes()), "t0s": [float(t) for t in entry["t0s"]]}
        r = EVENTS.ref(entry["name"], inputs, canonical_host=True)
        got[entry["name"]] = (entry, r["sr"], r["n_samples"], r["events"])
    return got


def midi_cases(corpus_events):
    for name, (_, _, _, ev) in corpus_events.items():
        yield f"corpus {name}", ev, 36, 24000, 120.0
    yield from golden_corpus.midi_synthetic_cases()


def midi_case(name, events, note, ppq, tempo, scratch):
    """(reference {hex, notes}, Rust file bytes, Rust note messages). The reference is the file
    engine.export_midi (pretty_midi) wrote and its note messages as mido read them."""
    ref = MIDI.ref(name, {"events": events, "note": note, "ppq": ppq, "tempo": tempo})
    job, mid = scratch / "midi_job.json", scratch / "rs.mid"
    job.write_text(json.dumps({"events": events, "note": note, "ppq": ppq, "tempo": tempo}))
    d = run_parity("midi", job, mid)
    assert "notes" in d, list(d)[:5]
    msgs = sorted([[n["tick"], n["note"], n["velocity"]] for n in d["notes"]]
                  + [[n["tick"] + n["duration"], n["note"], 0] for n in d["notes"]])
    return ref, mid.read_bytes(), msgs


def midi_problems(ref, rs_bytes, rs_msgs):
    """Identical bytes, and the Rust note tuples (tick, note, velocity, duration), expanded to note on and
    note off messages, equal the note messages of the reference file."""
    problems = []
    pb = bytes.fromhex(ref["hex"])
    if pb != rs_bytes:
        first = next((i for i, (a, b) in enumerate(zip(pb, rs_bytes)) if a != b), min(len(pb), len(rs_bytes)))
        problems.append(f"bytes differ: reference {len(pb)} B, rust {len(rs_bytes)} B, first difference at {first}")
    if ref["notes"] != rs_msgs:
        problems.append(f"note messages differ: reference {ref['notes'][:3]} rust {rs_msgs[:3]}")
    return problems


def test_midi_files_match_the_frozen_server(corpus_events, tmp_path):
    """The corpus event lists (18) and 11 synthetic lists: random, overlapping, edge values, note 127,
    half-tick ties, a 200 s span, empty, other ppq/tempo pairs; every file byte-identical."""
    failed, n = {}, 0
    for name, events, note, ppq, tempo in midi_cases(corpus_events):
        n += 1
        if problems := midi_problems(*midi_case(name, events, note, ppq, tempo, tmp_path)):
            failed[name] = problems
    assert not failed, failed
    assert n == len(corpus_events) + 11 == 29, n


def test_a_planted_change_fails_the_midi_comparison(corpus_events, tmp_path):
    name, events, note, ppq, tempo = next(c for c in midi_cases(corpus_events) if c[0] == "overlapping notes 10/25/50 ms")
    ref, rs_bytes, rs_msgs = midi_case(name, events, note, ppq, tempo, tmp_path)
    assert midi_problems(ref, rs_bytes, rs_msgs) == []
    changed = bytes([rs_bytes[-5] ^ 1])
    assert midi_problems(ref, rs_bytes[:-5] + changed + rs_bytes[-4:], rs_msgs), "byte"
    bad = [list(m) for m in rs_msgs]
    bad[0][2] = (bad[0][2] % 127) + 1
    assert midi_problems(ref, rs_bytes, bad), "note message"


def spike_case(name, corpus_events, out, scratch):
    """(entry, sr, n_samples, times, spike file, Rust reader answer, Rust reader samples) of one corpus file."""
    entry, sr, n_samples, ev = corpus_events[name]
    times = [e["t"] for e in ev]
    spike = scratch / f"{os.path.splitext(name)[0]}_spikes.wav"
    (scratch / "spike_job.json").write_text(json.dumps({"source": str(out / name), "times": times, "format": "pcm16"}))
    d = run_parity("spike", scratch / "spike_job.json", spike)
    assert {"sr", "n_samples"} <= set(d), list(d)[:5]
    back = run_parity(spike, "--pcm-out", scratch / "spikes_back.f32")
    return entry, sr, n_samples, times, spike, back, np.fromfile(scratch / "spikes_back.f32", dtype=np.float32)


def spike_problems(entry, sr, n_samples, times, spike, back, back_pcm):
    """Python's wave module: rate, channels, width, length, the full-scale sample at every hit index,
    silence before it and after the 1 ms blip; the bext time reference of the source; the Rust reader
    decodes the file to the same samples."""
    problems = []
    with wave.open(str(spike), "rb") as w:
        head = (w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes())
        pcm = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2")
    if head != (1, 2, sr, n_samples):
        problems.append(f"wave header {head} != {(1, 2, sr, n_samples)}")
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
    tref = golden_corpus.bext_time_reference(spike)
    if tref != entry.get("time_reference"):
        problems.append(f"bext time reference {tref} != source {entry.get('time_reference')}")
    if len(back_pcm) != n_samples or not np.array_equal(back_pcm, pcm.astype(np.float32) / 32768.0):
        problems.append("rust reader: samples differ from the wave module's")
    if (back["info"].get("bext") or {}).get("time_reference") != entry.get("time_reference"):
        problems.append("rust reader: bext time reference differs")
    return problems


def test_spike_files_have_a_blip_per_hit(corpus, corpus_events, tmp_path):
    """With and without a bext chunk, at 96 kHz and in a 100-sample file."""
    out, _ = corpus
    failed = {}
    for name in golden_corpus.SPIKE_FILES:
        if problems := spike_problems(*spike_case(name, corpus_events, out, tmp_path)):
            failed[name] = problems[:4]
    assert not failed, failed


def test_a_planted_change_fails_the_spike_check(corpus, corpus_events, tmp_path):
    out, _ = corpus
    entry, sr, n_samples, times, spike, back, back_pcm = spike_case("bext_ixml.wav", corpus_events, out, tmp_path)
    assert spike_problems(entry, sr, n_samples, times, spike, back, back_pcm) == []
    assert spike_problems(entry, sr, n_samples, times[:-1], spike, back, back_pcm), "one hit less"
    moved = dict(entry, time_reference=(entry["time_reference"] or 0) + 1)
    assert spike_problems(moved, sr, n_samples, times, spike, back, back_pcm), "time reference"
    assert spike_problems(entry, sr, n_samples, times, spike, back, back_pcm * 0.5), "reader samples"


# ---------------------------------------------------------------------------------------------
# analyze: `service_tool core-analyze` (frozen/rust_analyze.json)
#
# Reference: engine.analyze_track of the retired server in the Rust mode as it ran before P3-2a
# (engine._rust_detect plus engine._events_after_detection with the Rust refinement and velocities);
# key: file, detector, role, params. Events must be identical: t, v, rise and order (json.dumps keeps
# the sign of zero and every float bit).

ONSETS = [0.25, 0.75, 1.25, 1.75, 2.25, 2.75]
PARAMS = [
    ({}, "kick"),
    ({"refine": False}, "other"),
    ({"shift_samples": 37, "min_gap_ms": 45, "refine_lo_ms": -7, "refine_hi_ms": 7,
      "vel_lo_db": -50, "vel_hi_db": -3}, "kick"),
    ({"shift_samples": -480, "thresholds": [0.3, 0.3, 0.3, 0.3, 0.3]}, "bass"),
    ({"min_gap_ms": 5, "shift_samples": 12.7}, "snare"),
]
DETECTOR = "dsp"   # the DSP detector is the only one on main (#281 D16)
ANALYZE_FILES = ["pcm16_mono.wav", "float32_mono_96k.wav", "pcm16_mono_44k.wav",
                 "pcm24_stereo_lr.wav", "pcm16_partial5000.wav", "pcm16_short100.wav"]


@pytest.fixture
def soft_gate_off(monkeypatch):
    """The frozen references predate the A1 snare soft gate (P3-12 recorded the retired server, which has
    no soft rescue): DRUMSPIKE_A1_SOFT_SIM=0 for the analyze cases only. A module-level os.environ set
    would leak into the other core_analyze callers of the same pytest process (the demo-session truth
    test read 46 snares instead of 50 with the gate pinned off). The A1 behaviour itself is covered by
    the soft-gate unit tests and the a1-gate (enst, fix_metric)."""
    monkeypatch.setenv("DRUMSPIKE_A1_SOFT_SIM", "0")


def analyze_reference(path, role, params):
    """(events, detector) of the retired server for one file, role and params."""
    inputs = {"source_sha256": sha256(open(path, "rb").read()), "role": role, "params": params,
              "detector": DETECTOR, "model_sha256": None}
    r = ANALYZE.ref(f"{os.path.basename(path)}|{DETECTOR}|{role}|{json.dumps(params, sort_keys=True)}", inputs)
    return r["events"], r["detector"]


def analyze_jobs(path):
    return [{"wav": str(path), "role": role, "params": params, "model": None} for params, role in PARAMS]


def same(a, b):
    return json.dumps(a) == json.dumps(b)


@pytest.mark.usefixtures("soft_gate_off")
def test_the_kick_track_gives_identical_events(kick_wav):
    src, idx = kick_wav
    for (params, role), res in zip(PARAMS, core_analyze(analyze_jobs(src))):
        ref, det = analyze_reference(src, role, params)
        assert "error" not in res, res
        assert same(res["events"], ref), (params, role, res["events"], ref)
        assert res["detector"] == det == "rust-dsp"
    res = core_analyze([{"wav": str(src), "role": "kick", "params": {}, "model": None}])[0]
    got = [round(e["t"] * SR) for e in res["events"]]
    assert len(got) == len(idx) and all(abs(g - i) <= 24 for g, i in zip(got, idx)), (got, idx)
    assert all(e["rise"] == round(e["rise"], 1) for e in res["events"])


@pytest.mark.usefixtures("soft_gate_off")
@pytest.mark.parametrize("name", ANALYZE_FILES)
def test_the_corpus_gives_identical_events(written, name):
    out, _ = written
    for (params, role), res in zip(PARAMS, core_analyze(analyze_jobs(out / name))):
        ref, _ = analyze_reference(out / name, role, params)
        assert "error" not in res, res
        assert same(res["events"], ref), (name, params, role)


def planted_detection():
    """A detection list that reaches every branch of analyze_detected: a pair 6 ms before and 400 ms after
    each onset, near duplicates, one onset without its early hit."""
    det = []
    for t in ONSETS:
        det += [(t - 0.006, 35), (t + 0.4, 38)]
    det += [(0.749, 35), (1.2385, 35), (1.2585, 35)]
    return [list(d) for d in det if d[0] != 1.25 - 0.006]


@pytest.mark.usefixtures("soft_gate_off")
def test_analyze_detected_counts_every_branch(kick_wav):
    src, _ = kick_wav
    det = planted_detection()
    total = {}
    jobs = [dict(job, detected=det) for job in analyze_jobs(src)]
    for (params, role), res in zip(PARAMS, core_analyze(jobs)):
        ref = ANALYZE.ref(f"detected|{role}|{json.dumps(params, sort_keys=True)}",
                          {"source_sha256": sha256(open(src, "rb").read()), "detected": [tuple(d) for d in det],
                           "role": role, "params": params})
        assert "error" not in res, res
        assert same(res["events"], ref), (params, role)
        for k, v in res["branches"].items():
            total[k] = total.get(k, 0) + v
    assert set(total) == {"tracks", "detected", "filter_runs", "filtered_out", "dedup_dropped", "refine_off",
                          "refine_moved", "rise_none", "dedup2_dropped", "shifted", "events",
                          "reach_moved", "tail_moved", "reach_unresolved",   # P4-12: the reach rule's steps
                          "back_moved", "back_busy", "back_separate"}        # P4-12 stage 2: the back-track's
    assert total["reach_moved"] == total["tail_moved"] == total["reach_unresolved"] == 0   # the engine's rule (1)
    assert total["back_moved"] == total["back_busy"] == total["back_separate"] == 0
    for k in ("filter_runs", "filtered_out", "dedup_dropped", "refine_off", "refine_moved", "dedup2_dropped",
              "shifted"):
        assert total[k] > 0, (k, total)


@pytest.mark.usefixtures("soft_gate_off")
def test_the_refinement_rule_is_a_job_option(kick_wav):
    """P4-12: `rule` 2 runs the reach rule after the detector; no `rule` (or 1) is the engine's, frozen above. Coarse
    times 17 ms before each attack (the DSP frame before it): rule 1 gives them back, rule 2 finds the attacks."""
    src, idx = kick_wav
    det = [[t - 0.017, 35] for t in ONSETS]
    old, one, new, three, bad = core_analyze([{"wav": str(src), "role": "kick", "model": None, "detected": det, "rule": r}
                                              for r in (None, 1, 2, 3, 99)])   # 99: no such rule (#321 made 7 real)
    assert same(old["events"], one["events"]) and old["branches"]["reach_moved"] == 0
    assert [round(e["t"] * SR) for e in old["events"]] == [round((t - 0.017) * SR) for t in ONSETS]
    assert new["branches"]["reach_moved"] == len(ONSETS), new["branches"]
    got = [round(e["t"] * SR) for e in new["events"]]
    assert all(abs(g - i) <= 24 for g, i in zip(got, idx)) and len(got) == len(idx), (got, idx)
    # P4-12 stage 2: rule 3 is rule 2 and then the back-track: each hit moves back to the first motion of its attack
    assert new["branches"]["back_moved"] == 0 and three["branches"]["back_moved"] == len(ONSETS), three["branches"]
    first = [round(e["t"] * SR) for e in three["events"]]
    assert all(f < g and 0 <= f - i <= 8 for f, g, i in zip(first, got, idx)) and len(first) == len(idx), (first, got, idx)
    assert "unknown rule" in bad.get("error", ""), bad


@pytest.mark.usefixtures("soft_gate_off")
def test_an_unsupported_rate_is_an_error(tmp_path):
    x, _ = synth_track(ONSETS, 3.5, sr=22050)
    src = write_wav(tmp_path / "kick22k.wav", x, sr=22050)
    a, b = core_analyze([{"wav": str(src), "role": "kick", "model": None},
                         {"wav": str(src), "role": "kick", "model": None, "detected": [[0.25, 35]]}])
    assert "22050" in a.get("error", ""), a
    assert "22050" in b.get("error", ""), b


@pytest.mark.usefixtures("soft_gate_off")
def test_a_planted_change_fails_the_analyze_comparison(written):
    """`same` compares every float bit, the sign of zero and the order of the events."""
    out, _ = written
    res = core_analyze(analyze_jobs(out / "pcm16_mono.wav")[:1])[0]
    ref, _ = analyze_reference(out / "pcm16_mono.wav", "kick", {})
    assert same(res["events"], ref) and len(ref) > 2
    bad = json.loads(json.dumps(ref))
    bad[1]["t"] = float(np.nextafter(bad[1]["t"], 1.0))
    assert not same(res["events"], bad), "one ulp"
    bad = json.loads(json.dumps(ref))
    bad[1]["v"] = (bad[1]["v"] % 127) + 1
    assert not same(res["events"], bad), "velocity"
    assert not same(res["events"], ref[::-1]), "order"
