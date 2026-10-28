"""P4-4: an independent check of the MIDI and spike WAV exports of a Drum Spike project.

    uv run --with mido --with soundfile --with numpy python tools/check_exports.py <project_dir>... \
        [--answer export_all.json [--expect-error TRACK]...] [--json report.json]

The checker never imports project code. It reads the project document (format 1, JSON) for the
hit list, the source WAV with soundfile and its `bext` chunk by hand, the exported
`Exports/<track>_spike.mid` with mido and `Exports/<track>_spike.wav` with soundfile plus a hand
parse of its chunks, and derives every expected value from the document and the source alone.
The rules are the export contract of `docs/migration/phase4-report.md` section "P4-4" (the
property list). Per track:

  names    E1  one MIDI and one spike file per track, `<track>_spike.mid` / `.wav`; no two tracks
               share a file name when the disk ignores case (APFS default)
  hits     E2  the exported hits are the events without `"cut": true`; a hit is "outside" when
               round(t * sr) is not a sample of the source (< 0 or >= frames)
  MIDI     M1  format 1, PPQ and tempo read from the file (all set_tempo events); one note per
               hit (outside hits too: the MIDI file has no length; t <= 0 is tick 0), in time order
           M2  note-on time within half a tick of the hit (the error is reported in samples;
               at 44.1 and 48 kHz that is 0 samples after rounding). P4-8: through the file's own
               tempo events, so any tempo map; half a tick is that of the tempo at the note.
               #466/#479: the file holds whole-microsecond set_tempo events while the notes sit
               on the map's EXACT tempos, so the note time is checked against the round-trip
               bound of integer_tempo_round_trip_ticks in crates/core/src/midi/tests.rs: half a
               tick of note rounding plus half a tick per segment boundary before the note (each
               boundary tick is the nearest tick to the step time) plus the elapsed time times
               the worst relative mismatch of the written tempos against the map's exact ones.
               A whole-microsecond tempo with no steps keeps the strict half a tick.
           M4  P4-8: the tempo track holds the document's `tempo_map` (default: one set_tempo of
               500000 and 4/4, no marker): the start tempo, one set_tempo per step within half a
               tick of its time, the start meter at tick 0, a time signature at the tick of each
               meter change (bars counted from bar 1), and a `Bar 1` marker at the tick of
               `bar1_s` (none when bar 1 is at 0 s)
           M5  P4-40g: with the document's `export_markers` on, the one multi-track file holds
               one marker per `markers` entry at its time (note as text or `Marker N`); with it
               off, no marker beyond `Bar 1`
           M3  note number = the track's `note` (36 when absent), velocity = `v` (100 when absent),
               channel 10, every note-on has its note-off, the note-off within half a tick of
               50 ms after the hit
  WAV      W1  rate, frame count and sample format (PCM_16/24/32, FLOAT) of the source, mono
           W2  the full-scale sample (32767, 8388607, 2147483647 or 1.0) exactly at every in-range
               hit and nowhere else; then the 1 ms linear decay (n = max(2, round(0.001 * sr))
               samples, level 1 - k/n within one step of the format, at 32-bit PCM within the
               24-bit precision of the float blip); zero everywhere else
           W3  RIFF size and data size consistent with the frames; the `bext` chunk present
               exactly when the source has one, with the source's time reference, origination
               date and time
  follower F1  a track with `follows` holds the source's uncut hits at the nearest sample of its
               own rate (a hit exactly between two samples may take either); stale hits after
               an edit of the source show here
  audio    AS1 (#209) `<track>_audio_spike.wav`, when the export wrote one: mono, the rate,
               frame count and sample format of the source; the `bext` description
               "Drum Spike audio spikes" with the source's time reference
           AS2 every maximal nonzero run starts at an in-range uncut hit sample (the snippet
               starts where the blip of the same hit does, 0 differences); a track with hits
               whose file lets 0 of them be checked (nonzero at the hit, silent just before)
               is a problem
           AS3 at every hit sample the file holds the source's mono mix (the copy; the exact
               code for a mono source, within 1.5 steps of the format for stereo; skipped for
               >2 channels); the measured snippet lengths are reported per track for the
               per-drum distribution
  answer   A1  with --answer (the JSON answer of export_all): no errors except those named with
               --expect-error (only those tracks may lack files), and `outside` of every track
               equal to the checker's count (a track the answer leaves out has 0). A single-mode
               export (the answer's `midi_file`) replaced the per-track MIDI files, so no
               `<track>_spike.mid` is expected then

P4-7a, with --layers: the one-shot layer of every track that has one (the document's `layer`
object), `Exports/<track>_layer.wav` against the source and the one-shot file (read with soundfile):

  layer    L1  the file exists; rate and frame count of the source; the one-shot's channels (1 or
               2); IEEE float; the `bext` chunk exactly when the source has one, with its time
               reference, date and time
           L2  the onset on the hit: for every uncut (no `cut`, no `bleed`) in-range hit whose voice
               overlaps no other, the file from `round(t * sr) - onset` holds the one-shot times the
               hit's gain (level, polarity, velocity curve), so the one-shot's onset sample sits on
               the hit sample (0 samples off). A one-shot at another rate: the cross-correlation with
               the checker's own (linear) resample peaks at lag 0. With `align` (P4-7b) the voice
               sits at the lag of the alignment answer (`--align track=answer.json`), checked by the
               sub-sample peak of the cross-correlation within 0.15 samples; `auto` polarity takes
               the answer's sign
           L3  silence before the first voice and after the last one

Exit status 1 when any track has a problem. The JSON report has the numbers per track.
"""
import argparse
import glob
import json
import os
import struct
import sys
import unicodedata
from fractions import Fraction

import mido
import numpy as np
import soundfile as sf

BLIP_S = 0.001          # blip length: max(2, round(0.001 * sr)) samples
NOTE_LENGTH_S = 0.05    # MIDI note length
DRUM_CHANNEL = 9        # MIDI channel 10
DEFAULT_NOTE = 36
DEFAULT_VELOCITY = 100
SUBTYPES = {"PCM_16": 16, "PCM_24": 24, "PCM_32": 32, "FLOAT": None}


# ---------------------------------------------------------------- reading by hand

def riff_chunks(path):
    """[(id, offset of the body, declared size)] of the top-level chunks of a RIFF/WAVE file and
    the file size. Reads the headers only."""
    size = os.path.getsize(path)
    out = []
    with open(path, "rb") as f:
        head = f.read(12)
        if head[:4] != b"RIFF" or head[8:12] != b"WAVE":
            raise ValueError(f"not a RIFF/WAVE file: {path}")
        riff_size = struct.unpack("<I", head[4:8])[0]
        pos = 12
        while pos + 8 <= size:
            f.seek(pos)
            cid, n = struct.unpack("<4sI", f.read(8))
            out.append((cid, pos + 8, n))
            pos += 8 + n + (n & 1)
    return out, size, riff_size


def bext_of(path):
    """{time_reference, date, time, originator, description} of the `bext` chunk, or None."""
    chunks, _, _ = riff_chunks(path)
    for cid, at, n in chunks:
        if cid == b"bext":
            with open(path, "rb") as f:
                f.seek(at)
                body = f.read(min(n, 602))
            if len(body) < 346:
                return None
            text = lambda a, b: body[a:b].split(b"\0", 1)[0].decode("latin-1")
            return {"description": text(0, 256), "originator": text(256, 288),
                    "date": text(320, 330), "time": text(330, 338),
                    "time_reference": struct.unpack("<Q", body[338:346])[0]}
    return None


def doc_of(project_dir):
    """(document, its path): the one `.drumspike` file of the folder (the folder's name first)."""
    named = os.path.join(project_dir, os.path.basename(os.path.normpath(project_dir)) + ".drumspike")
    docs = [named] if os.path.isfile(named) else sorted(glob.glob(os.path.join(project_dir, "*.drumspike")))
    if len(docs) != 1:
        raise SystemExit(f"{project_dir}: expected one .drumspike document, found {len(docs)}")
    with open(docs[0]) as f:
        return json.load(f), docs[0]


def source_path(project_dir, rec):
    """The source file of a source record: `relative` to the project folder first, then `path`."""
    if not rec:
        return None
    rel = rec.get("relative")
    if rel:
        p = os.path.normpath(os.path.join(project_dir, rel))
        if os.path.isfile(p):
            return p
    p = rec.get("path")
    return p if p and os.path.isfile(p) else None


def name_key(name):
    """The name as a case-insensitive, normalisation-insensitive disk (APFS) sees it: Unicode's
    canonical caseless form NFD(casefold(NFD(name))) (`Straße` = `STRASSE`, final sigma, NFC = NFD)."""
    nfd = lambda x: unicodedata.normalize("NFD", x)
    return nfd(nfd(name).casefold())


# ---------------------------------------------------------------- MIDI

def midi_notes(path):
    """The file's notes: (ppq, tempo map, notes [{on_tick, off_tick, on_s, off_s, note, velocity,
    channel}], problems). Times in seconds go through the file's own tempo events, exactly
    (fractions); pairs are first in, first out per channel and key."""
    mid = mido.MidiFile(path)
    problems = []
    if mid.type != 1:
        problems.append(f"MIDI type {mid.type}, not 1")
    ppq = mid.ticks_per_beat
    tempos = []          # (abs tick, microseconds per quarter)
    sigs, markers = [], []   # (abs tick, numerator, denominator), (abs tick, text)
    raw = []             # (abs tick, order, msg)
    for ti, track in enumerate(mid.tracks):
        tick = 0
        for k, msg in enumerate(track):
            tick += msg.time
            if msg.type == "set_tempo":
                tempos.append((tick, msg.tempo))
            elif msg.type == "time_signature":
                sigs.append((tick, msg.numerator, msg.denominator))
            elif msg.type == "marker":
                markers.append((tick, msg.text))
            elif msg.type in ("note_on", "note_off"):
                raw.append((tick, ti, k, msg))
        if not track or track[-1].type != "end_of_track":
            problems.append(f"track {ti} does not end with end_of_track")
    tempos.sort(key=lambda x: x[0])
    if not tempos or tempos[0][0] != 0:
        problems.append("no set_tempo at tick 0")
        tempos.insert(0, (0, 500000))   # the SMF default, 120 BPM

    def seconds(tick):
        s, last_t, last_tempo = Fraction(0), 0, tempos[0][1]
        for t, tempo in tempos:
            if t > tick:
                break
            s += Fraction(t - last_t, ppq) * Fraction(last_tempo, 1_000_000)
            last_t, last_tempo = t, tempo
        return s + Fraction(tick - last_t, ppq) * Fraction(last_tempo, 1_000_000)

    raw.sort(key=lambda r: (r[0], r[1], r[2]))
    open_notes, notes = {}, []
    for tick, _, _, msg in raw:
        key = (msg.channel, msg.note)
        if msg.type == "note_on" and msg.velocity > 0:
            open_notes.setdefault(key, []).append((tick, msg.velocity))
        else:
            if not open_notes.get(key):
                problems.append(f"note-off without a note-on at tick {tick} (key {msg.note})")
                continue
            on, vel = open_notes[key].pop(0)
            notes.append({"on_tick": on, "off_tick": tick, "note": msg.note, "velocity": vel,
                          "channel": msg.channel})
    hanging = sum(len(v) for v in open_notes.values())
    if hanging:
        problems.append(f"{hanging} note-ons without a note-off")
    for n in notes:
        n["on_s"], n["off_s"] = seconds(n["on_tick"]), seconds(n["off_tick"])
    notes.sort(key=lambda n: (n["on_tick"], n["off_tick"]))
    def tick_len(tick):
        """Seconds of one tick at `tick`: the tempo in force there."""
        us = tempos[0][1]
        for t, tempo in tempos:
            if t > tick:
                break
            us = tempo
        return Fraction(us, 1_000_000) / ppq

    for n in notes:
        n["half_tick_on"] = tick_len(n["on_tick"]) / 2
        n["half_tick_off"] = tick_len(n["off_tick"]) / 2
    meta = {"sigs": sigs, "markers": markers, "seconds": seconds, "tick_len": tick_len}
    return ppq, tempos, meta, notes, problems


def check_tempo_track(tmap, ppq, tempos, meta, r):
    """M4: the tempo track holds the document's tempo map (`tmap`: the `tempo_map` object or None)."""
    m = tmap or {}
    bad = r["problems"]
    seconds, tick_len = meta["seconds"], meta["tick_len"]
    bpm = m.get("bpm", 120.0)
    steps = m.get("steps", [])
    want_us = [round(60e6 / bpm)] + [round(60e6 / s["bpm"]) for s in steps]
    if len(tempos) != len(want_us):
        bad.append(f"tempo track: {len(tempos)} tempo events, the map has {len(want_us)}")
    else:
        for i, ((t, us), w) in enumerate(zip(tempos, want_us)):
            if abs(us - w) > 1:
                bad.append(f"tempo track: event {i} is {us} us per quarter, the map says {w}")
            if i == 0 and t != 0:
                bad.append(f"tempo track: the first tempo is at tick {t}, not 0")
            if i > 0:
                err = seconds(t) - Fraction(steps[i - 1]["t"])
                if abs(err) > tick_len(t) / 2 + Fraction(1, 10**12):
                    bad.append(f"tempo track: step {i - 1} is {float(err):+.6f} s from its time")
    # bar 1
    bar1_s = m.get("bar1_s", 0.0)
    marks = [(t, x) for t, x in meta["markers"] if x.strip().lower() == "bar 1"]
    t1 = 0
    if bar1_s == 0:
        if marks:
            bad.append("tempo track: a Bar 1 marker although bar 1 is at 0 s")
    elif not marks:
        bad.append("tempo track: no Bar 1 marker")
    else:
        t1 = marks[0][0]
        err = seconds(t1) - Fraction(bar1_s)
        if abs(err) > tick_len(t1) / 2 + Fraction(1, 10**12):
            bad.append(f"tempo track: the Bar 1 marker is {float(err):+.6f} s from bar1_s")
    # meter
    meter = m.get("meter", [4, 4])
    want = [(0, meter[0], meter[1])]
    tick, bar, cur = t1, 1, meter
    for c in m.get("changes", []):
        tick += (c["bar"] - bar) * ppq * 4 * cur[0] // cur[1]
        bar, cur = c["bar"], c["meter"]
        want.append((tick, cur[0], cur[1]))
    if sorted(meta["sigs"]) != want:
        bad.append(f"tempo track: time signatures {sorted(meta['sigs'])}, the map says {want}")


# ---------------------------------------------------------------- the integer round trip (#479)

RT_NOISE_TICKS = 1e-6                # float noise of the round-trip bound, in ticks


def exact_map_tempos(tmap):
    """[(exact us per quarter, start time)] of the document's tempo map, exact (fractions). No
    map: the default 120 BPM (500000 us, exact)."""
    m = tmap or {}
    out = [(Fraction(60_000_000) / Fraction(m.get("bpm", 120.0)), Fraction(0))]
    for s in m.get("steps", []):
        out.append((Fraction(60_000_000) / Fraction(s["bpm"]), Fraction(s["t"])))
    return out


def file_seg(tempos, tick):
    """The index of the file's tempo segment that holds `tick`."""
    j = 0
    for k, (tk, _) in enumerate(tempos):
        if tk <= tick:
            j = k
    return j


def rt_rel_mismatch(tempos, exact, j):
    """The relative mismatch |us - exact_us| / exact_us of the worst of the file's first j+1
    tempo events against the map's exact tempos."""
    worst = Fraction(0)
    for k in range(j + 1):
        us = Fraction(tempos[k][1])
        ex = exact[min(k, len(exact) - 1)][0]
        worst = max(worst, abs(us - ex) / ex)
    return worst


def rt_bound_ticks(ppq, tempos, exact, j, t_s):
    """The round-trip bound of #466 in ticks (`integer_tempo_round_trip_ticks` in
    crates/core/src/midi/tests.rs): half a tick of note rounding plus half a tick per segment
    boundary before the note (each boundary tick is the nearest tick to the step time), plus
    the elapsed time times the worst relative tempo mismatch, in ticks of the written tempo.
    A whole-microsecond tempo with no steps keeps the strict half a tick."""
    us_j = tempos[j][1]
    rel = rt_rel_mismatch(tempos, exact, j)
    return (j + 1) * 0.5 + t_s * float(rel) * ppq * 1e6 / us_j + RT_NOISE_TICKS


def check_midi(path, hits, sr, note, r, tmap=None):
    """M1 to M4 for one track; `hits` are (t, v) in time order; `tmap` the document's `tempo_map`."""
    ppq, tempos, meta, notes, problems = midi_notes(path)
    tick_len = meta["tick_len"]
    exact = exact_map_tempos(tmap)
    r["midi"] = {"ppq": ppq, "tempo_us": [t for _, t in tempos], "notes": len(notes),
                 "meter_events": len(meta["sigs"]),
                 "half_tick_samples": float(max(tick_len(t) for t, _ in tempos) * sr / 2)}
    r["problems"] += [f"MIDI: {p}" for p in problems]
    check_tempo_track(tmap, ppq, tempos, meta, r)
    if len(notes) != len(hits):
        r["problems"].append(f"MIDI: {len(notes)} notes for {len(hits)} hits")
    errs, rounded_off, worst, err_ticks = [], 0, None, []
    bound_ticks, bound_samples = [], []
    for n, (t, v) in zip(notes, hits):
        want = Fraction(t) if t > 0 else Fraction(0)     # a hit at t <= 0 is tick 0
        # M2: the note time through the file's own integer set_tempo events (n["on_s"]) against
        # the hit, within the round-trip bound of the note's segment (#466/#479)
        j = file_seg(tempos, n["on_tick"])
        us_j = tempos[j][1]
        tick_j = Fraction(us_j, 1_000_000) / ppq
        err = abs(n["on_s"] - want)
        errs.append(float(err * sr))
        err_ticks.append(float(err / tick_j))
        bound = rt_bound_ticks(ppq, tempos, exact, j, float(t))
        bound_ticks.append(bound)
        bound_samples.append(float(bound * tick_j * sr))
        if err > bound * tick_j:
            if worst is None or err > worst[0]:
                worst = (err, t)
        if round(n["on_s"] * sr) != round(want * sr):
            rounded_off += 1
        if n["note"] != note:
            r["problems"].append(f"MIDI: note {n['note']} != {note} at t={t}")
        if n["velocity"] != v:
            r["problems"].append(f"MIDI: velocity {n['velocity']} != {v} at t={t}")
        if n["channel"] != DRUM_CHANNEL:
            r["problems"].append(f"MIDI: channel {n['channel'] + 1} != 10 at t={t}")
        off_want = max(Fraction(0), Fraction(t) + Fraction(NOTE_LENGTH_S))   # 50 ms after the hit
        jf = file_seg(tempos, n["off_tick"])
        us_f = tempos[jf][1]
        tick_f = Fraction(us_f, 1_000_000) / ppq
        err_off = abs(n["off_s"] - off_want)
        bound_f = rt_bound_ticks(ppq, tempos, exact, jf, float(t + NOTE_LENGTH_S))
        bound_ticks.append(bound_f)
        bound_samples.append(float(bound_f * tick_f * sr))
        if err_off > bound_f * tick_f:
            r["problems"].append(f"MIDI: note-off {float(err_off * sr):+.3f} samples from 50 ms "
                                 f"after the hit at t={t} (more than the integer limit "
                                 f"{bound_f:.3f} ticks)")
    if worst is not None:
        r["problems"].append(f"MIDI: note-on {float(worst[0] * sr):+.3f} samples from the hit at "
                             f"t={worst[1]} (more than the integer round-trip limit)")
    overlaps = sum(1 for a, b in zip(notes, notes[1:]) if b["on_tick"] < a["off_tick"])
    r["midi"].update({"err_max_samples": max((abs(e) for e in errs), default=0.0),
                      "err_max_ticks": max(err_ticks, default=0.0),
                      "err_bound_ticks": max(bound_ticks, default=0.0),
                      "err_bound_samples": max(bound_samples, default=0.0),
                      "rounded_sample_off": rounded_off, "overlapping_notes": overlaps})


# ---------------------------------------------------------------- spike WAV

def full_scale_codes(path, info):
    """The samples as exact integer codes (PCM) or floats (FLOAT), and the full-scale value and
    one step of the format."""
    if info.subtype == "FLOAT":
        x = sf.read(path, dtype="float32", always_2d=True)[0][:, 0]
        return x, np.float32(1.0), 2.0 ** -23
    bits = SUBTYPES[info.subtype]
    x = sf.read(path, dtype="int32", always_2d=True)[0][:, 0]   # left-justified by libsndfile
    x = x >> (32 - bits)
    full = (1 << (bits - 1)) - 1
    # one step: the format's LSB, or a 32-bit float's precision of the blip level (24 bits) at
    # 32-bit PCM: the decay is computed in float, the full-scale sample is exact in every format
    return x, full, max(1, full * 2.0 ** -23)


def check_wav(path, src, src_info, idx, r):
    """W1 to W3 for one track; `idx` are the sample positions of the in-range hits."""
    info = sf.info(path)
    r["wav"] = {"sr": info.samplerate, "subtype": info.subtype, "channels": info.channels,
                "frames": info.frames}
    want_subtype = src_info.subtype if src_info.subtype in SUBTYPES else "FLOAT"
    for what, got, want in (("rate", info.samplerate, src_info.samplerate),
                            ("frames", info.frames, src_info.frames),
                            ("subtype", info.subtype, want_subtype), ("channels", info.channels, 1)):
        if got != want:
            r["problems"].append(f"WAV: {what} {got} != {want}")
    if info.subtype not in SUBTYPES:
        return
    # W3: the chunks by hand
    chunks, size, riff_size = riff_chunks(path)
    if riff_size != size - 8:
        r["problems"].append(f"WAV: RIFF size {riff_size} != file size - 8 ({size - 8})")
    bps = 4 if info.subtype == "FLOAT" else SUBTYPES[info.subtype] // 8
    data = [n for cid, _, n in chunks if cid == b"data"]
    if data != [info.frames * bps]:
        r["problems"].append(f"WAV: data chunk sizes {data} != {info.frames * bps}")
    sb, wb = bext_of(src), bext_of(path)
    r["wav"]["time_reference"] = wb and wb["time_reference"]
    r["source_time_reference"] = sb and sb["time_reference"]
    if (sb is None) != (wb is None):
        r["problems"].append(f"WAV: bext {'missing' if wb is None else 'present'}; the source has "
                             f"{'one' if sb else 'none'}")
    elif sb:
        for k in ("time_reference", "date", "time", "originator"):
            if sb[k] != wb[k]:
                r["problems"].append(f"WAV: bext {k} {wb[k]!r} != source {sb[k]!r}")
    # W2: the samples
    x, full, step = full_scale_codes(path, info)
    sr, frames = info.samplerate, info.frames
    n = max(2, round(BLIP_S * sr))
    want_full = np.unique(np.asarray(idx, dtype=np.int64))
    got_full = np.flatnonzero(x == full)
    if len(got_full) != len(want_full) or not np.array_equal(got_full, want_full):
        missing = np.setdiff1d(want_full, got_full)
        extra = np.setdiff1d(got_full, want_full)
        r["problems"].append(f"WAV: full-scale samples: {len(missing)} hits without one "
                             f"(first {missing[:3].tolist()}), {len(extra)} extra (first {extra[:3].tolist()})")
    if np.any(x > full) or np.any(x < 0):
        r["problems"].append("WAV: samples above full scale or below zero")
    # the expected blip level per sample: the louder blip wins where two overlap
    want = {}
    for i in want_full.tolist():
        for k in range(n):
            j = i + k
            if j >= frames:
                break
            level = 1.0 - k / n
            if level > want.get(j, -1.0):
                want[j] = level
    pos = np.fromiter(want.keys(), dtype=np.int64, count=len(want))
    lev = np.fromiter(want.values(), dtype=np.float64, count=len(want))
    scale = 1.0 if info.subtype == "FLOAT" else float(full)
    got = x[pos].astype(np.float64) if len(pos) else np.zeros(0)
    bad = np.flatnonzero(np.abs(got - lev * scale) > step * 1.0001)
    if len(bad):
        j = int(pos[bad[0]])
        r["problems"].append(f"WAV: {len(bad)} blip samples off the 1 ms decay (first at {j}: "
                             f"{got[bad[0]]} != {lev[bad[0]] * scale})")
    nonzero = int(np.count_nonzero(x))
    want_nonzero = int(np.count_nonzero(lev if info.subtype == "FLOAT" else np.round(lev * scale)))
    if nonzero != want_nonzero:
        r["problems"].append(f"WAV: {nonzero} nonzero samples, the blips have {want_nonzero}")
    r["wav"].update({"blips": int(len(got_full)), "blip_len": n})


# ---------------------------------------------------------------- the audio spikes (#209)

AUDIO_BEXT_DESCRIPTION = "Drum Spike audio spikes"


def check_audio_wav(path, src, src_info, idx, r):
    """The audio-spike file of a track (#209), when the export wrote one; `idx` are the sample
    positions of the in-range uncut hits, as in check_wav. The file is the track's own audio
    with a snippet of about 1.5 waveform periods per hit, exact digital silence everywhere
    else. Checked: the shape of the source (mono, rate, frames, format) and the audio `bext`;
    every maximal nonzero run starts at a hit sample — the snippet starts where the blip of
    the same hit does, 0 differences — with a track whose file lets 0 hits be checked a
    problem; and at every hit sample the file holds the source's mono mix (the copy). The
    measured snippet lengths go into the report for the per-drum distribution."""
    info = sf.info(path)
    r["audio"] = {"sr": info.samplerate, "subtype": info.subtype, "frames": info.frames}
    want_subtype = src_info.subtype if src_info.subtype in SUBTYPES else "FLOAT"
    for what, got, want in (("rate", info.samplerate, src_info.samplerate),
                            ("frames", info.frames, src_info.frames),
                            ("subtype", info.subtype, want_subtype), ("channels", info.channels, 1)):
        if got != want:
            r["problems"].append(f"audio: {what} {got} != {want}")
    if info.subtype not in SUBTYPES:
        return
    ab, sb = bext_of(path), bext_of(src)
    if ab:
        r["audio"]["description"] = ab["description"]
        if ab["description"] != AUDIO_BEXT_DESCRIPTION:
            r["problems"].append(f"audio: bext description {ab['description']!r} != "
                                 f"{AUDIO_BEXT_DESCRIPTION!r}")
        if sb and ab["time_reference"] != sb["time_reference"]:
            r["problems"].append(f"audio: bext time reference {ab['time_reference']} != source "
                                 f"{sb['time_reference']}")
    x, _, step = full_scale_codes(path, info)
    frames = info.frames
    want = np.unique(np.asarray(idx, dtype=np.int64))
    if frames != src_info.frames:
        return   # reported above; the index arithmetic below is not safe
    # the snippet of a quiet source holds quantized-zero crossings: zero runs of a sample or
    # two inside the copy. They do not start a snippet, so interior zero gaps of at most 8
    # samples are closed before the runs are cut (a snippet is at least 2 ms, a hundred
    # samples and more). The file's own leading and trailing silence stays: closing it would
    # fabricate a snippet before the first hit or stretch the last one past its fade.
    nz = x != 0
    if nz.any():
        d = np.diff(nz.astype(np.int8))
        for a, b in zip((np.flatnonzero(d == -1) + 1).tolist(),
                        (np.flatnonzero(d == 1) + 1).tolist()):
            if b - a <= 8:
                nz[a:b] = True
    d = np.diff(nz.astype(np.int8))
    starts = [0] if len(nz) and nz[0] else []
    starts += (np.flatnonzero(d == 1) + 1).tolist()
    starts = np.asarray(starts, dtype=np.int64)
    r["audio"]["runs"] = int(len(starts))
    # a run starts at its snippet's hit sample, unless the source is silent there: the copy
    # then starts with the hit's own zeros and the first nonzero lands a few samples later
    # (attributable, up to 8). Anything else starts a snippet off its hit: a difference.
    off_by = np.searchsorted(want, starts, side="right") - 1   # the nearest hit at or before
    ok = off_by >= 0
    ok[ok] = starts[ok] - want[off_by[ok]] <= 8
    off = starts[~ok]
    r["audio"]["differences"] = int(len(off))
    if len(off):
        r["problems"].append(f"audio: {len(off)} snippets do not start at a hit sample "
                             f"(first at {int(off[0])})")
    # a hit is checked when its snippet start is observable: the file is nonzero at the hit
    # and silent just before it (a snippet cut at the next hit merges into that one's run)
    at_hit = x[want] != 0 if len(want) else np.zeros(0, bool)
    silent_before = np.ones(len(want), bool)
    if len(want):
        pos = want > 0
        silent_before[pos] = x[want[pos] - 1] == 0
    checked = int(np.count_nonzero(at_hit & silent_before))
    r["audio"]["checked"] = checked
    if len(want) and checked == 0:
        r["problems"].append("audio: 0 hits checked (no snippet starts at a hit sample)")
    # the copy: at every hit sample the file holds the source's mono mix. A hit on the last
    # sample or directly before the next one got no snippet (the core skips it). The core
    # reads by 2^(b-1) and writes by full-1 with a clamp, so the round trip of a mono source
    # is within one step; a stereo fold adds its own half step and a hot mix clamps.
    skip = {frames - 1} | {int(s) + 1 for s in want.tolist()}
    copy_idx = np.asarray([s for s in want.tolist() if s not in skip], dtype=np.int64)
    if len(copy_idx):
        if src_info.channels == 1:
            s_codes, _, _ = full_scale_codes(src, src_info)
            bad = copy_idx[np.abs(s_codes[copy_idx].astype(np.float64) - x[copy_idx]) > step * 1.5]
        elif src_info.channels == 2:
            st = sf.read(src, dtype="float32", always_2d=True)[0]
            mix = (st[:, 0].astype(np.float64) + st[:, 1]) * np.float64(np.float32(np.sqrt(0.5)))
            _, full, _ = full_scale_codes(path, info)
            want_mix = np.clip(mix[copy_idx], -1.0, 1.0) * float(full)
            bad = copy_idx[np.abs(x[copy_idx].astype(np.float64) - want_mix) > step * 2.5]
        else:
            bad = []
            r["audio"]["copy"] = "skipped: the ffmpeg fold is not replicated for >2 channels"
        if len(bad):
            r["problems"].append(f"audio: {len(bad)} hit samples do not hold the source's mix "
                                 f"(first at {int(bad[0])})")
    # the measured snippet length per observable hit, for the per-drum distribution
    lens = []
    for k, s in enumerate(want.tolist()):
        if not (at_hit[k] and silent_before[k]):
            continue
        stop = want[k + 1] if k + 1 < len(want) else frames
        seg = x[s:stop] != 0
        lens.append(int(np.flatnonzero(seg)[-1]) + 1)
    r["audio"]["lengths"] = lens


# ---------------------------------------------------------------- the layer (P4-7a)

LAYER_LEVEL = (-60.0, 12.0)


def layer_gain(layer, v, sign):
    """The gain of a hit with velocity `v`: the level (dB, clamped), `sign`, the velocity curve."""
    level = 10 ** (min(max(float(layer.get("level_db", 0.0)), LAYER_LEVEL[0]), LAYER_LEVEL[1]) / 20)
    curve = layer.get("curve") or {"kind": "db", "range_db": 24.0}
    v = min(max(int(v), 0), 127)
    kind = curve.get("kind", "db")
    if kind == "db":
        g = 10 ** (-min(max(float(curve.get("range_db", 24.0)), 0.0), 96.0) * (1 - v / 127) / 20)
    elif kind == "linear":
        g = v / 127
    else:
        g = 1.0
    return sign * level * g


def layer_hits(track):
    """(t, v) of the hits that have a layer voice: no `cut`, no `bleed`, in time order."""
    ev = [e for e in (track.get("events") or []) if e.get("cut") is not True and e.get("bleed") is not True]
    return sorted(((float(e["t"]), int(e.get("v", DEFAULT_VELOCITY))) for e in ev), key=lambda h: h[0])


def parabolic_peak(y, i):
    """The sub-sample offset of the peak at index i of y."""
    if i <= 0 or i >= len(y) - 1:
        return 0.0
    a, b, c = y[i - 1], y[i], y[i + 1]
    den = a - 2 * b + c
    return 0.0 if den >= 0 else float(np.clip(0.5 * (a - c) / den, -0.5, 0.5))


def check_layer(project_dir, name, track, src, src_info, r, align=None):
    """L1 to L3 for one track with a layer. `align`: the answer of track_layer_align (its `hits` and
    `sign`), needed for an aligned layer or an `auto` polarity."""
    layer = track["layer"]
    path = os.path.join(project_dir, "Exports", f"{name}_layer.wav")
    shot_path = source_path(project_dir, layer.get("source"))
    if shot_path is None:
        base = os.path.basename((layer.get("source") or {}).get("path") or "")
        cand = os.path.join(project_dir, "Audio Files", base)
        shot_path = cand if base and os.path.isfile(cand) else None
    lr = r.setdefault("layer", {"problems": []})
    probs = lr["problems"]
    if not os.path.isfile(path):
        probs.append(f"missing {os.path.relpath(path, project_dir)}")
        return
    if shot_path is None:
        probs.append("the one-shot file is not found")
        return
    info = sf.info(path)
    ch = int(layer.get("channels", 1))
    for what, got, want in (("rate", info.samplerate, src_info.samplerate), ("frames", info.frames, src_info.frames),
                            ("channels", info.channels, ch), ("subtype", info.subtype, "FLOAT")):
        if got != want:
            probs.append(f"layer: {what} {got} != {want}")
    sb, wb = bext_of(src), bext_of(path)
    if (sb is None) != (wb is None):
        probs.append(f"layer: bext {'missing' if wb is None else 'present'}; the source has {'one' if sb else 'none'}")
    elif sb:
        for k in ("time_reference", "date", "time"):
            if sb[k] != wb[k]:
                probs.append(f"layer: bext {k} {wb[k]!r} != source {sb[k]!r}")
    x = sf.read(path, dtype="float32", always_2d=True)[0]
    shot, shot_sr = sf.read(shot_path, dtype="float32", always_2d=True)
    if shot.shape[1] > 2:
        shot = shot.mean(axis=1, keepdims=True)
    sr, frames = info.samplerate, info.frames
    pol = layer.get("polarity", "normal")
    sign = -1.0 if pol == "invert" else 1.0
    if pol == "auto" or layer.get("align"):
        if align is None:
            probs.append("layer: aligned or auto polarity: give the alignment answer (--align)")
            return
        sign = float(align["sign"])
    lags = {round(h["t"], 9): h["lag"] for h in align["hits"]} if (align and layer.get("align")) else {}
    if shot_sr != sr:
        n_out = int(round(len(shot) * sr / shot_sr))
        tt = np.arange(n_out) * shot_sr / sr
        shot = np.stack([np.interp(tt, np.arange(len(shot)), shot[:, c]) for c in range(shot.shape[1])], axis=1)
        # P4-40e: an onset override (samples of the one-shot) replaces the measured onset
        base = layer.get("onset_set") if layer.get("onset_set") is not None else layer["onset"]
        onset = int(round(int(base) * sr / shot_sr))
    else:
        base = layer.get("onset_set") if layer.get("onset_set") is not None else layer["onset"]
        onset = int(base)
    n = len(shot)
    hits = [(t, v, round(t * sr)) for t, v in layer_hits(track)]
    hits = [h for h in hits if 0 <= h[2] < frames]
    starts = [i - onset + lags.get(round(t, 9), 0.0) for t, _, i in hits]
    checked, off = 0, []
    for k, (t, v, i) in enumerate(hits):
        s0 = starts[k]
        lo = s0 - (starts[k - 1] + n if k else -np.inf)
        hi = (starts[k + 1] if k + 1 < len(hits) else np.inf) - (s0 + n)
        a, b = int(np.floor(s0)), int(np.floor(s0)) + n
        if lo < 1 or hi < 1 or a < 0 or b > frames:
            continue                                   # overlaps a neighbour or the file edge
        g = layer_gain(layer, v, sign)
        seg = x[a:b, :ch].astype(np.float64)
        want = g * shot[:, :ch].astype(np.float64)
        checked += 1
        if lags or shot_sr != sr:
            # the voice's position by cross-correlation (sub-sample), against the expected start
            m = seg[:, 0] if ch else seg
            lag_range = 4
            pad = np.zeros(n + 2 * lag_range)
            pad[lag_range:lag_range + n] = want[:, 0]
            ys = np.array([np.dot(m, pad[lag_range - d:lag_range - d + n]) for d in range(-lag_range, lag_range + 1)])
            j = int(np.argmax(ys))
            got = a + (j - lag_range) + parabolic_peak(ys, j)
            exp = s0
            tol = 0.15 if lags else 0.0
            if (lags and abs(got - exp) > tol) or (not lags and j - lag_range != 0):
                off.append((t, round(got - exp, 3)))
            elif not lags:
                # another rate: the level by least squares against the checker's resample (within 3 %)
                ref = shot[:, 0].astype(np.float64)
                fit = float(np.dot(m, ref) / max(np.dot(ref, ref), 1e-30))
                if abs(fit - g) > 0.03 * abs(g):
                    off.append((t, f"gain {fit:.4f} != {g:.4f}"))
            continue
        err = np.max(np.abs(seg - want))
        if err > 1e-6 * max(1.0, float(np.max(np.abs(want)))):
            # where does the one-shot sit instead? the integer lag of the best match
            m = seg[:, 0]
            ys = [np.dot(m, np.roll(want[:, 0], d)) for d in range(-8, 9)]
            off.append((t, int(np.argmax(ys)) - 8))
    lr.update({"checked": checked, "voices": len(hits), "onset": onset, "sign": sign})
    if off:
        probs.append(f"layer: {len(off)} of {checked} voices not on their hit (first t={off[0][0]}: off {off[0][1]})")
    if checked == 0 and hits:
        probs.append("layer: no voice could be checked (all overlap)")
    if starts:
        first = int(np.floor(min(starts)))
        last = int(np.floor(max(starts))) + n + 2 * 16 + 1
        if first > 0 and np.any(x[:max(0, first - 16)] != 0):
            probs.append("layer: sound before the first voice")
        if last < frames and np.any(x[last:] != 0):
            probs.append("layer: sound after the last voice")


def check_layers(project_dir, align=None):
    """L1 to L3 for every track with a layer; `align` maps a track to its track_layer_align answer."""
    project_dir = os.path.abspath(project_dir)
    doc, doc_path = doc_of(project_dir)
    report = {"project": project_dir, "doc": doc_path, "tracks": {}, "problems": []}
    for name, tr in doc["tracks"].items():
        if not isinstance(tr.get("layer"), dict):
            continue
        r = {"hits": len(layer_hits(tr)), "problems": []}
        src = source_path(project_dir, tr.get("source"))
        if not src:
            r["problems"].append("source not found")
        else:
            check_layer(project_dir, name, tr, src, sf.info(src), r, (align or {}).get(name))
            r["problems"] += r["layer"]["problems"]
        report["tracks"][name] = r
    if not report["tracks"]:
        report["problems"].append("no track has a layer")
    return report


# ---------------------------------------------------------------- the project

def expected_hits(track):
    """(t, v) of the events that go into the output, in time order; (number of cut events)."""
    events = track.get("events") or []
    keep = [e for e in events if e.get("cut") is not True]
    hits = sorted(((float(e["t"]), int(e.get("v", DEFAULT_VELOCITY))) for e in keep), key=lambda h: h[0])
    return hits, len(events) - len(keep)


def check_follower(doc, name, track, sr, r):
    """F1: the follower's hits are its source's uncut hits at the same sample in its rate."""
    src_name = track.get("follows")
    if not src_name:
        return
    src = doc["tracks"].get(src_name)
    if src is None:
        r["problems"].append(f"follows {src_name!r}, which is not a track")
        return
    src_hits, _ = expected_hits(src)
    want = sorted(Fraction(t) * sr for t, _ in src_hits)                 # exact source positions
    got = sorted(round(float(e["t"]) * sr) for e in (track.get("events") or []))
    r["follows"] = src_name
    # the nearest sample of the follower's rate; a source hit exactly between two samples (a tie
    # between rates, 0.625 s at 44.1 kHz) may take either
    off = [abs(g - w) > Fraction(1, 2) for g, w in zip(got, want)]
    if len(want) != len(got) or any(off):
        diff = sum(off) + abs(len(want) - len(got))
        r["problems"].append(f"follower: {len(got)} hits, the source {src_name} has {len(want)} "
                             f"({diff} positions differ): stale after an edit of the source?")


def check_project(project_dir, answer=None, expected_errors=()):
    """The report of one project folder (see the module text). With `answer` (the export_all
    answer) every track's `outside` is compared (a track the answer leaves out has 0), and an error
    is a problem unless its track is in `expected_errors`; only such a track may lack its files
    (its files, if any, are not this export's and are not checked)."""
    project_dir = os.path.abspath(project_dir)
    doc, doc_path = doc_of(project_dir)
    exports = os.path.join(project_dir, "Exports")
    order = [n for n in doc.get("order") or [] if n in doc["tracks"]]
    order += [n for n in doc["tracks"] if n not in order]
    report = {"project": project_dir, "doc": doc_path, "tracks": {}, "problems": []}
    # E1: two names that the disk sees as one file
    seen = {}
    for name in order:
        k = name_key(name)
        if k in seen:
            report["problems"].append(f"tracks {seen[k]!r} and {name!r} write the same files on a "
                                      "case-insensitive disk")
        seen.setdefault(k, name)
    answer_errors = {e["name"]: e for e in (answer or {}).get("errors", [])}
    answer_outside = (answer or {}).get("outside", {})
    expected = set(expected_errors)
    for name in sorted(set(answer_errors) - expected):
        report["problems"].append(f"answer: error for {name}: {answer_errors[name].get('message')}")
    for name in sorted(expected - set(answer_errors)):
        report["problems"].append(f"answer: no error for {name}, one was expected")
    for name in order:
        tr = doc["tracks"][name]
        hits, n_cut = expected_hits(tr)
        note = tr.get("note", DEFAULT_NOTE)
        r = {"hits": len(hits), "cut": n_cut, "problems": []}
        src = source_path(project_dir, tr.get("source"))
        src_info = sf.info(src) if src else None
        sr = src_info.samplerate if src_info else tr.get("sr")
        r["sr"] = sr
        if src_info and tr.get("sr") not in (None, src_info.samplerate):
            r["problems"].append(f"document sr {tr.get('sr')} != source {src_info.samplerate}")
        check_follower(doc, name, tr, sr, r)
        inside = []
        if src_info:
            idx = [round(t * sr) for t, _ in hits]
            inside = [i for i in idx if 0 <= i < src_info.frames]
            r["outside"] = len(idx) - len(inside)
        refused = name in expected and name in answer_errors
        if refused:
            r["export_error"] = answer_errors[name]
            report["tracks"][name] = r
            continue
        if answer is not None and "outside" in r and answer_outside.get(name, 0) != r["outside"]:
            r["problems"].append(f"answer: outside {answer_outside.get(name, 0)} != {r['outside']}")
        mid = os.path.join(exports, f"{name}_spike.mid")
        wav = os.path.join(exports, f"{name}_spike.wav")
        if os.path.isfile(mid):
            check_midi(mid, hits, sr or 48000, note, r, doc.get("tempo_map"))
        elif not (answer or {}).get("midi_file"):
            r["problems"].append(f"missing {os.path.relpath(mid, project_dir)}")
        if os.path.isfile(wav) and src_info:
            check_wav(wav, src, src_info, inside, r)
        else:
            r["problems"].append(f"missing {os.path.relpath(wav, project_dir)}" if src_info
                                 else "source not found")
        audio = os.path.join(exports, f"{name}_audio_spike.wav")
        if os.path.isfile(audio) and src_info:
            check_audio_wav(audio, src, src_info, inside, r)
        report["tracks"][name] = r
    check_multi_markers(doc, project_dir, report)
    return report


def check_multi_markers(doc, project_dir, report):
    """P4-40g: the marker events of the one multi-track MIDI file. With the document's
    `export_markers` on, one marker per entry of `markers` at its time (the file's own tempo
    events applied, half a tick), the note as the text or `Marker N`; with it off, none
    beyond the map's `Bar 1`. No multi file: nothing to check."""
    exports = os.path.join(project_dir, "Exports")
    mids = sorted(glob.glob(os.path.join(exports, "*_drums.mid")))
    if not mids:
        return
    ppq, tempos, meta, _, problems = midi_notes(mids[-1])
    report["problems"] += [f"multi MIDI: {p}" for p in problems]
    on = doc.get("export_markers") is True
    ms = sorted((m for m in doc.get("markers") or []
                 if isinstance(m, dict) and isinstance(m.get("t"), (int, float))),
                key=lambda m: m["t"])
    want = [(m["t"], m["note"] if isinstance(m.get("note"), str) else f"Marker {i + 1}")
            for i, m in enumerate(ms)]
    got = list(meta["markers"])
    if on:
        for t, text in want:
            at = [(tick, x) for tick, x in got if x == text]
            if not at:
                report["problems"].append(f"multi MIDI: no marker {text!r} for {t} s")
                continue
            err = meta["seconds"](at[0][0]) - Fraction(t)
            if abs(err) > meta["tick_len"](at[0][0]) / 2 + Fraction(1, 10**12):
                report["problems"].append(f"multi MIDI: marker {text!r} is {float(err):+.6f} s from {t} s")
        named = {text for _, text in want}
        for _, x in got:
            if x.strip().lower() != "bar 1" and x not in named:
                report["problems"].append(f"multi MIDI: marker {x!r} is no review marker")
    elif not on:
        for _, x in got:
            if x.strip().lower() != "bar 1":
                report["problems"].append(f"multi MIDI: marker {x!r} although Markers in MIDI is off")


def table(reports):
    print(f"{'track':<26}{'sr':>7}{'hits':>6}{'out':>4} {'MIDI err':>9} {'off':>4} {'blips':>6}  problems")
    for rep in reports:
        print(f"# {rep['project']}")
        for p in rep["problems"]:
            print(f"  ! {p}")
        for name, r in rep["tracks"].items():
            m, w = r.get("midi", {}), r.get("wav", {})
            p = "; ".join(r["problems"][:2]) + (" ..." if len(r["problems"]) > 2 else "")
            print(f"{name[:25]:<26}{r.get('sr') or '-':>7}{r['hits']:>6}{r.get('outside', '-'):>4} "
                  f"{m.get('err_max_samples', float('nan')):>9.3f} {m.get('rounded_sample_off', '-'):>4} "
                  f"{w.get('blips', '-'):>6}  {p}")


def count_problems(reports):
    return sum(len(rep["problems"]) + sum(1 for r in rep["tracks"].values() if r["problems"]) for rep in reports)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("projects", nargs="+")
    ap.add_argument("--answer", help="the JSON answer of export_all (one project only)")
    ap.add_argument("--expect-error", action="append", default=[], metavar="TRACK",
                    help="a track whose export error is expected (with --answer; repeat for more)")
    ap.add_argument("--json", help="write the reports here")
    ap.add_argument("--layers", action="store_true", help="P4-7a: check the layer files instead of the MIDI and spikes")
    ap.add_argument("--align", action="append", default=[], metavar="TRACK=ANSWER.json",
                    help="the track_layer_align answer of an aligned or auto-polarity layer (repeat for more)")
    a = ap.parse_args(argv)
    if a.layers:
        align = {}
        for item in a.align:
            k, f = item.split("=", 1)
            with open(f) as fh:
                ans = json.load(fh)
            align[k] = ans.get("result", ans)
        reports = [check_layers(p, align) for p in a.projects]
        for rep_ in reports:
            print(f"# {rep_['project']}")
            for p in rep_["problems"]:
                print(f"  ! {p}")
            for name, r in rep_["tracks"].items():
                lr = r.get("layer", {})
                print(f"{name[:25]:<26} voices {lr.get('voices', '-'):>5} checked {lr.get('checked', '-'):>5}  "
                      + "; ".join(r["problems"][:2]))
        if a.json:
            with open(a.json, "w") as f:
                json.dump(reports, f, indent=1, default=float)
        bad = count_problems(reports)
        print(f"\n{sum(len(r['tracks']) for r in reports)} layers, {bad} with problems")
        return 1 if bad else 0
    answer = None
    if a.answer:
        with open(a.answer) as f:
            answer = json.load(f)
        answer = answer.get("result", answer)
    one = len(a.projects) == 1
    reports = [check_project(p, answer if one else None, a.expect_error if one else ()) for p in a.projects]
    if a.json:
        with open(a.json, "w") as f:
            json.dump(reports, f, indent=1, default=float)
    table(reports)
    bad = count_problems(reports)
    n = sum(len(rep["tracks"]) for rep in reports)
    print(f"\n{n} tracks, {bad} with problems")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
