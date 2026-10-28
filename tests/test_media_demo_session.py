"""Media 2 (#242): the licence-clean demo session generator (tools/media/demo_session.py).

The gate: two runs with the same seed give byte-equal files (sha256); every feature is
present in truth.json with the counts printed (ghost notes, 1 flam, 1 roll, 1 tom fill);
the bleed of each drum into the other close mics is 15 to 35 dB under its own mic, measured
on the kit check of bars 17 and 18 and printed per pair; and a run of Drum Spike's analysis
on the session (service_tool, crates/devtools) finds the truth onsets within 1 ms, printed
per role as found/true with the missed times, failing when a role finds no hit at all or
under the per-role floor of the DSP detector). The session is
generated into temporary directories: audio never goes into git.

The 15-35 dB pairs are the drums that have a close mic; the crash has none (the overheads
carry it) and the room mic has no truth drum, so both stay out of their checks.
"""
import hashlib
import json
import sys
import wave
from pathlib import Path

import numpy as np
import pytest

from core_tools import core_analyze
from helpers import ROOT

sys.path.insert(0, str(ROOT / "tools" / "media"))
import demo_session as ds  # noqa: E402

SEED = ds.DEFAULT_SEED
MATCH_S = 0.001                       # the analysis must land within 1 ms of the truth onset

# The close mics each role is analysed on, and the truth drums of that role. The k out mic
# sits 0.35 m from the beater, so its arrivals are 1.0 ms after the truth onset on their
# own; the role's count is the union over its tracks, where k in carries the kick.
ROLE_TRACKS = {
    "kick": ["k in", "k out", "new take"],
    "snare": ["s top", "s bot"],
    "tom": ["t1", "t2", "t3"],
    "hat": ["hh"],
    "cymbal": ["ride", "oh l", "oh r"],
}
ROLE_DRUMS = {
    "kick": ["kick"], "snare": ["snare"], "tom": ["t1", "t2", "t3"],
    "hat": ["hat"], "cymbal": ["ride", "crash"],
}


@pytest.fixture(scope="module")
def session(tmp_path_factory):
    """The demo session, generated once into a temporary directory."""
    out = tmp_path_factory.mktemp("demo242")
    ds.generate(out, seed=SEED)
    return out


@pytest.fixture(scope="module")
def truth(session):
    return json.loads((session / "truth.json").read_text())


def _read_wav(path):
    """The mono 24-bit WAV as float64 in -1..1, with its sample rate."""
    with wave.open(str(path)) as w:
        assert (w.getnchannels(), w.getsampwidth()) == (1, 3), path
        sr = w.getframerate()
        raw = w.readframes(w.getnframes())
    q = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3).astype(np.int32)
    q = q[:, 0] | (q[:, 1] << 8) | (q[:, 2] << 16)
    q = np.where(q & 0x800000, q - (1 << 24), q)     # sign-extend the 24 bits
    return q.astype(np.float64) / 8388607.0, sr


def test_two_runs_with_the_same_seed_are_byte_equal(tmp_path):
    a, b = tmp_path / "a", tmp_path / "b"
    ds.generate(a, seed=SEED)
    ds.generate(b, seed=SEED)
    names = sorted(p.name for p in a.iterdir())
    assert names == sorted(p.name for p in b.iterdir())
    assert len(names) == len(ds.TRACKS) + 1          # the 13 WAVs and truth.json
    for name in names:
        ha = hashlib.sha256((a / name).read_bytes()).hexdigest()
        hb = hashlib.sha256((b / name).read_bytes()).hexdigest()
        assert ha == hb, name


def test_every_feature_is_in_truth(truth):
    hits = truth["hits"]
    counted = {
        "ghost_notes": sum(1 for h in hits if h["ghost"]),
        "flams": sum(1 for h in hits if h["flam"]) // 2,   # a flam is a grace note and its accent
        "rolls": 1 if any(h["roll"] for h in hits) else 0,
        "tom_fills": 1 if any(h["tom_fill"] for h in hits) else 0,
    }
    assert truth["features"] == counted
    print(f"features: {counted['ghost_notes']} ghost notes, {counted['flams']} flam, "
          f"{counted['rolls']} roll, {counted['tom_fills']} tom fill")
    assert counted["ghost_notes"] > 0
    assert counted["flams"] == 1 and counted["rolls"] == 1 and counted["tom_fills"] == 1


def test_close_mic_bleed_is_15_to_35_db_under_own_mic(session, truth):
    spb, bar1, sr = truth["samples_per_beat"], truth["bar1_offset_samples"], truth["sr"]
    check = [h for h in truth["hits"]
             if bar1 + 16 * 4 * spb <= h["t"] < bar1 + (16 * 4 + len(ds.DRUMS)) * spb]
    assert sorted(h["drum"] for h in check) == sorted(ds.DRUMS)   # one kit-check hit per drum
    onset = {h["drum"]: h["t"] for h in check}
    x = {m: _read_wav(session / f"{m}.wav")[0] for m in ds.CLOSE_MIC_DRUM}
    a = int(0.006 * sr)      # the window starts after the farthest close-mic arrival (4.4 ms)
    n = int(0.34 * sr)       # and ends before the next kit-check hit (0.625 s later)
    rows = []
    for drum, own in ds.OWN_MIC.items():
        t0 = onset[drum]
        rms_own = float(np.sqrt(np.mean(x[own][t0 + a: t0 + a + n] ** 2)))
        for mic, owner in ds.CLOSE_MIC_DRUM.items():
            if owner == drum:
                continue
            rms = float(np.sqrt(np.mean(x[mic][t0 + a: t0 + a + n] ** 2)))
            rows.append((20.0 * np.log10(rms_own / rms), drum, mic, own))
    for db, drum, mic, own in sorted(rows):
        print(f"bleed {drum:5s} -> {mic:7s}: {db:5.1f} dB under its own mic ({own})")
    bad = [(db, drum, mic) for db, drum, mic, _ in rows if not 15.0 <= db <= 35.0]
    assert not bad, ("bleed outside 15-35 dB under its own mic: "
                     + ", ".join(f"{drum}->{mic} {db:.1f} dB" for db, drum, mic in bad))


def test_the_analysis_finds_the_truth_hits(session, truth):
    jobs, spans = [], []
    for role, tracks in ROLE_TRACKS.items():
        first = len(jobs)
        jobs += [{"wav": str(session / f"{m}.wav"), "role": role} for m in tracks]
        spans.append((role, first, len(jobs)))
    results = core_analyze(jobs)
    # what the DSP detector finds on the demo session. The floors sit just under the counts of
    # this run; a DSP quality card (#317) may raise them, a regression must not fall under them.
    FLOOR = {"kick": 55, "snare": 50, "tom": 7, "hat": 90, "cymbal": 25}
    empty = []
    for role, i, j in spans:
        assert all("error" not in res for res in results[i:j]), results[i:j]
        ev = [e["t"] for res in results[i:j] for e in res["events"]]
        true = sorted(h["t"] / truth["sr"] for h in truth["hits"] if h["drum"] in ROLE_DRUMS[role])
        missed = [t for t in true if not any(abs(t - e) <= MATCH_S for e in ev)]
        found = len(true) - len(missed)
        print(f"{role}: found {found} / true {len(true)} hits within 1 ms of the truth onset; "
              f"missed {len(missed)}{': ' + ', '.join(f'{t:.3f}' for t in missed) if missed else ''}")
        if found == 0:
            empty.append(role)
        assert found >= FLOOR[role], (role, found, FLOOR[role])
    assert not empty, f"the analysis finds no hit at all for: {', '.join(empty)}"
