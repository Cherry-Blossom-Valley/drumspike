"""P1-8: the Rust core against tests/golden/ (the outputs of the retired Python server).

The WAVs are written again from the same deterministic generators (golden_corpus.write_corpus);
their SHA-256 must match the golden files first, so a changed generator shows up as such and
not as a parity failure. The Rust side is the release `parity` binary (P3-12: the Python
binding and the Python path are gone): decode bit-exact (on every host: the Rust reader equals
the fusing arm64 ffmpeg build the golden files come from), the min and max of every peak level
exact as integers of 1e-5, the RMS within one step (P1-3), refined times identical, rise within
1e-9 dB, velocities identical, the MIDI file byte-identical. The golden files cannot be written
again: tests/make_golden.py needed engine.py.
"""
import json
import os

import pytest

import golden_corpus
from golden_corpus import GOLDEN, PARITY_BIN, modes, rust_outputs, sha256

GOLDEN_FILES = sorted(GOLDEN.glob("*.json"))


@pytest.fixture(scope="module")
def corpus(tmp_path_factory):
    if not PARITY_BIN.exists():
        if os.environ.get("DRUMSPIKE_REQUIRE_CORE") == "1":
            pytest.fail(f"DRUMSPIKE_REQUIRE_CORE=1 and {PARITY_BIN} is missing (cargo build --release -p drumspike-devtools)")
        pytest.skip(f"{PARITY_BIN} missing: cargo build --release -p drumspike-devtools")
    out = tmp_path_factory.mktemp("golden_corpus")
    return out, {name: (t0s, channels) for name, t0s, channels in golden_corpus.write_corpus(out)}


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


def flip(digest):
    return ("1" if digest[0] == "0" else "0") + digest[1:]


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
