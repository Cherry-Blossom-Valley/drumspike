"""P4-7a and P4-7b: the layer render (`Exports/<track>_layer.wav`), checked by tools/check_exports.py
--layers (soundfile, the bext chunk by hand; no project code) on sessions that this file writes and that
the app's own commands edit and render (`service_tool run`).

Cases: a kick at 48 kHz (24-bit with a session start in bext) with a mono WAV one-shot, -3 dB, the dB
curve over 12 dB; a stereo snare at 96 kHz with a 16-bit AIFF one-shot, inverted, the linear curve; a tom
at 88.2 kHz with a 44.1 kHz one-shot (resampled), fixed; overlapping hits, a cut hit, a bleed hit and a
hit outside the file. Then a planted fault per property (L1 to L3), and the aligned render of P4-7b
(a mic made of the one-shot at planted sub-sample lags, inverted) against the alignment answer.

Slow: `DRUMSPIKE_LAYER_SESSION=<make_session.py folder>` (the 30-track session) with a layer on a kick,
a snare and a tom track.
"""
import json
import os
import shutil
import struct
import sys
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "apps" / "desktop" / "selftest"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_exports  # noqa: E402
from make_project import build_project  # noqa: E402
from test_exports import answer, drum, ev, run_steps, write_wav  # noqa: E402


def one_shot(sr, pre_s=0.004, secs=0.15, seed=1):
    """A drum-like one-shot: silence, then a decaying 160 Hz tone and a short noise burst."""
    n = int(round(secs * sr))
    t = (np.arange(n) - int(round(pre_s * sr))) / sr
    rng = np.random.default_rng(seed)
    x = 0.6 * np.sin(2 * np.pi * 160 * t) * np.exp(-t / 0.04) + 0.4 * rng.uniform(-1, 1, n) * np.exp(-t / 0.003)
    x[t < 0] = 0.0
    return x


def layer_problems(rep):
    return rep["problems"] + [f"{n}: {p}" for n, r in rep["tracks"].items() for p in r["problems"]]


KICK_SR, SNARE_SR, TOM_SR = 48_000, 96_000, 88_200


def layer_session(tmp):
    a = Path(tmp) / "audio"
    a.mkdir()
    kick_idx = [4_800, 24_000, 26_000, 60_000, 100_000, 150_000]
    kick = write_wav(a / "Kick.wav", drum(KICK_SR, 4, kick_idx), KICK_SR, "pcm24", 3600 * KICK_SR)
    snare_idx = [9_601, 96_001, 200_001, 287_999]
    snare = write_wav(a / "Snare.wav", drum(SNARE_SR, 3, snare_idx, channels=2), SNARE_SR, "pcm32")
    tom_idx = [44_101, 132_300, 220_500]
    tom = write_wav(a / "Tom.wav", drum(TOM_SR, 3, tom_idx), TOM_SR, "float32", 12_345_678)
    shots = Path(tmp) / "shots"
    shots.mkdir()
    sf.write(shots / "kick.wav", one_shot(KICK_SR), KICK_SR, subtype="FLOAT")
    st = one_shot(SNARE_SR, seed=2)
    sf.write(shots / "snare.aif", np.stack([st, -0.5 * st], axis=1), SNARE_SR, format="AIFF", subtype="PCM_16")
    sf.write(shots / "tom.wav", one_shot(44_100, seed=3), 44_100, subtype="PCM_24")
    kick_ev = [ev(i, KICK_SR, v=v) for i, v in zip(kick_idx, [127, 100, 90, 64, 30, 1])]
    kick_ev[3]["cut"] = True
    kick_ev.append({"t": 5.0, "v": 100})                                   # outside the file
    snare_ev = [ev(i, SNARE_SR, v=60 + 10 * k) for k, i in enumerate(snare_idx)]
    snare_ev[1]["bleed"] = True
    tom_ev = [ev(i, TOM_SR) for i in tom_idx]
    tracks = [("Kick", kick, "kick", 36, kick_ev), ("Snare", snare, "snare", 38, snare_ev),
              ("Tom", tom, "tom", 45, tom_ev)]
    return Path(build_project(tmp, "Layers", tracks)), shots


@pytest.fixture(scope="module")
def rendered(tmp_path_factory):
    tmp = tmp_path_factory.mktemp("layers")
    pdir, shots = layer_session(tmp)
    rows = run_steps([
        {"cmd": "project_open", "args": {"path": str(pdir)}},
        {"cmd": "track_layer_set", "args": {"sid": "$sid", "name": "Kick", "path": str(shots / "kick.wav"),
                                            "level_db": -3, "curve": {"kind": "db", "range_db": 12}}},
        {"cmd": "track_layer_set", "args": {"sid": "$sid", "name": "Snare", "path": str(shots / "snare.aif"),
                                            "polarity": "invert", "curve": {"kind": "linear"}}},
        {"cmd": "track_layer_set", "args": {"sid": "$sid", "name": "Tom", "path": str(shots / "tom.wav"),
                                            "curve": {"kind": "fixed"}}},
        {"cmd": "export_layer", "args": {"sid": "$sid", "name": "Kick"}},
        {"cmd": "export_layer", "args": {"sid": "$sid", "name": "Snare"}},
        {"cmd": "export_layer", "args": {"sid": "$sid", "name": "Tom"}},
        {"cmd": "project_save", "args": {"sid": "$sid"}},
    ], tmp)
    return pdir, rows


def test_every_layer_voice_has_its_onset_on_the_hit_sample(rendered):
    pdir, rows = rendered
    exports = [r["answer"] for r in rows if r["cmd"] == "export_layer"]
    assert [e["channels"] for e in exports] == [1, 2, 1]
    assert exports[0]["outside"] == 1 and exports[0]["time_reference"] == 3600 * KICK_SR
    assert exports[2]["time_reference"] == 12_345_678
    rep = check_exports.check_layers(pdir)
    assert layer_problems(rep) == []
    t = rep["tracks"]
    # kick: 5 voices (the cut one and the one outside have none), 2 overlap (24,000 and 26,000)
    assert t["Kick"]["layer"]["voices"] == 5 and t["Kick"]["layer"]["checked"] == 3
    assert t["Snare"]["layer"]["voices"] == 3 and t["Snare"]["layer"]["sign"] == -1.0   # the bleed hit has none
    assert t["Tom"]["layer"]["checked"] == 3                                           # 44.1 kHz one-shot


def copy_project(src, dst):
    shutil.copytree(src, dst)
    name = Path(src).name
    doc_path = dst / f"{name}.drumspike"
    doc = json.loads(doc_path.read_text())
    for tr in doc["tracks"].values():
        tr["source"]["relative"] = os.path.relpath(tr["source"]["path"], dst)
        tr["layer"]["source"]["relative"] = os.path.relpath(tr["layer"]["source"]["path"], dst)
    doc_path.write_text(json.dumps(doc))
    return dst, doc_path


def _shift(path, k):
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    sub = sf.info(path).subtype
    tref = check_exports.bext_of(path)
    y = np.roll(x, k, axis=0)
    sf.write(path, y, sr, subtype=sub)
    if tref:   # soundfile drops the bext chunk; the planted fault is the shift only
        _add_bext(path, tref["time_reference"])


def _add_bext(path, tref, date="2026-09-14", time_="10:31:02"):
    b = bytearray(open(path, "rb").read())
    body = bytearray(602)
    body[320:330] = date.encode()
    body[330:338] = time_.encode()
    body[338:346] = struct.pack("<Q", tref)
    chunk = b"bext" + struct.pack("<I", 602) + bytes(body)
    b = b[:12] + chunk + b[12:]
    struct.pack_into("<I", b, 4, len(b) - 8)
    open(path, "wb").write(bytes(b))


def _doc(doc_path, fn):
    doc = json.loads(doc_path.read_text())
    fn(doc)
    doc_path.write_text(json.dumps(doc))


PLANTS = {
    "shifted_one_sample": ("L2", lambda p, d: _shift(p / "Exports" / "Kick_layer.wav", 1)),
    "polarity_in_the_document": ("L2", lambda p, d: _doc(d, lambda doc: doc["tracks"]["Snare"]["layer"].update(polarity="normal"))),
    "velocity_curve": ("L2", lambda p, d: _doc(d, lambda doc: doc["tracks"]["Kick"]["layer"].update(curve={"kind": "linear"}))),
    "level": ("L2", lambda p, d: _doc(d, lambda doc: doc["tracks"]["Tom"]["layer"].update(level_db=-1))),
    "onset_in_the_document": ("L2", lambda p, d: _doc(d, lambda doc: doc["tracks"]["Kick"]["layer"].update(
        onset=doc["tracks"]["Kick"]["layer"]["onset"] + 1))),
    "tom_one_shot_one_sample_late": ("L2", lambda p, d: _shift(p / "Exports" / "Tom_layer.wav", 1)),
    "bext_time_reference": ("L1", lambda p, d: _rewrite_bext(p / "Exports" / "Kick_layer.wav", 7)),
    "a_missing_file": ("L1", lambda p, d: (p / "Exports" / "Snare_layer.wav").unlink()),
    "sound_before_the_first_voice": ("L3", lambda p, d: _poke(p / "Exports" / "Kick_layer.wav", 10, 0.5)),
}


def _rewrite_bext(path, tref):
    b = bytearray(open(path, "rb").read())
    chunks, _, _ = check_exports.riff_chunks(path)
    at = next(a for cid, a, _ in chunks if cid == b"bext")
    struct.pack_into("<Q", b, at + 338, tref)
    open(path, "wb").write(bytes(b))


def _poke(path, frame, value):
    chunks, _, _ = check_exports.riff_chunks(path)
    at = next(a for cid, a, _ in chunks if cid == b"data")
    b = bytearray(open(path, "rb").read())
    struct.pack_into("<f", b, at + 4 * frame, value)
    open(path, "wb").write(bytes(b))


@pytest.mark.parametrize("plant", sorted(PLANTS))
def test_the_layer_checker_finds_a_planted_fault(rendered, tmp_path, plant):
    src, _ = rendered
    pdir, doc_path = copy_project(src, tmp_path / "Layers")
    assert layer_problems(check_exports.check_layers(pdir)) == []
    rule, fn = PLANTS[plant]
    fn(pdir, doc_path)
    assert layer_problems(check_exports.check_layers(pdir)), f"{rule}: the checker missed {plant}"


# ---------------------------------------------------------------- P4-7b: the aligned render

def test_the_aligned_render_sits_at_the_lags_of_the_alignment(tmp_path):
    sr = 48_000
    x = one_shot(sr, seed=5)
    shots = tmp_path / "shots"
    shots.mkdir()
    sf.write(shots / "k.wav", x, sr, subtype="FLOAT")
    # the onset the app measures is not known here: build the mic from the one-shot itself, with the lags
    hits = [24_000, 48_000, 72_000, 96_000, 120_000]
    lags = [3.0, -6.5, 10.25, 0.0, -2.75]
    mic = np.zeros(3 * sr)
    # the one-shot's onset as the app measures it (a layer on a silent track)
    silent = write_wav(tmp_path / "silent.wav", np.zeros(sr), sr, "float32")
    p0 = Path(build_project(tmp_path / "p0", "P0", [("K", silent, "kick", 36, [])]))
    onset = answer(run_steps([
        {"cmd": "project_open", "args": {"path": str(p0)}},
        {"cmd": "track_layer_set", "args": {"sid": "$sid", "name": "K", "path": str(shots / "k.wav")}},
    ], tmp_path), "track_layer_set")["layer"]["onset"]
    n = len(x)
    X = np.fft.rfft(np.concatenate([x, np.zeros(n)]))
    f = np.fft.rfftfreq(2 * n)
    for i, lag in zip(hits, lags):
        # the one-shot delayed by `lag` samples (a band-limited shift), its onset at i + lag, inverted
        y = np.fft.irfft(X * np.exp(-2j * np.pi * f * lag), 2 * n)[:n + 40]
        a = i - onset
        mic[a:a + len(y)] += -0.8 * y                                   # inverted
    mic += 0.002 * np.sin(np.arange(len(mic)) * 0.21)
    src = write_wav(tmp_path / "mic.wav", mic, sr, "float32")
    pdir = Path(build_project(tmp_path / "p1", "Aligned", [("K", src, "kick", 36, [ev(i, sr, v=127) for i in hits])]))
    rows = run_steps([
        {"cmd": "project_open", "args": {"path": str(pdir)}},
        {"cmd": "track_layer_set", "args": {"sid": "$sid", "name": "K", "path": str(shots / "k.wav"),
                                            "polarity": "auto", "align": True, "curve": {"kind": "fixed"}}},
        {"cmd": "track_layer_align", "args": {"sid": "$sid", "name": "K"}},
        {"cmd": "export_layer", "args": {"sid": "$sid", "name": "K"}},
        {"cmd": "project_save", "args": {"sid": "$sid"}},
    ], tmp_path)
    al = answer(rows, "track_layer_align")
    assert al["polarity"] == "inverted" and al["sign"] == -1 and al["flips"] == []
    for h, lag in zip(al["hits"], lags):
        assert abs(h["lag"] - lag) < 0.2, (h, lag)
        assert h["match"] > 0.97
    assert answer(rows, "export_layer")["aligned"] is True
    rep = check_exports.check_layers(pdir, {"K": al})
    assert layer_problems(rep) == [], rep
    assert rep["tracks"]["K"]["layer"]["checked"] == 5
    # planted: one lag of the answer off by one sample, then no answer at all
    bad = json.loads(json.dumps(al))
    bad["hits"][2]["lag"] += 1.0
    assert layer_problems(check_exports.check_layers(pdir, {"K": bad}))
    assert layer_problems(check_exports.check_layers(pdir))


# ---------------------------------------------------------------- slow: the 30-track session

@pytest.mark.skipif(not os.environ.get("DRUMSPIKE_LAYER_SESSION"),
                    reason="slow: set DRUMSPIKE_LAYER_SESSION to a make_session.py folder (30 tracks)")
def test_the_30_track_session_with_a_layer_on_kick_snare_and_a_tom(tmp_path):
    session = Path(os.environ["DRUMSPIKE_LAYER_SESSION"])
    meta = json.loads((session / "session.json").read_text())
    roles = ["kick", "snare", "tom", "hat", "cymbal", "other"]
    tracks = []
    for k, t in enumerate(meta["tracks"]):
        sr = t.get("sr", meta.get("sr", 96_000))
        events = [{"t": h / sr, "v": 100} for h in t["hit_samples"]]
        tracks.append((f"T{k:02d}", str(session / t["file"]), roles[k % 6], 36, events))
    pdir = Path(build_project(tmp_path, "Thirty", tracks))
    shots = tmp_path / "shots"
    shots.mkdir()
    sf.write(shots / "k.wav", one_shot(48_000), 48_000, subtype="FLOAT")
    sf.write(shots / "s.wav", one_shot(96_000, seed=2), 96_000, subtype="PCM_24")
    sf.write(shots / "t.aif", one_shot(44_100, seed=3), 44_100, format="AIFF", subtype="PCM_16")
    steps = [{"cmd": "project_open", "args": {"path": str(pdir)}}]
    for name, shot in (("T00", "k.wav"), ("T01", "s.wav"), ("T02", "t.aif")):
        steps += [{"cmd": "track_layer_set", "args": {"sid": "$sid", "name": name, "path": str(shots / shot)}},
                  {"cmd": "export_layer", "args": {"sid": "$sid", "name": name}}]
    steps.append({"cmd": "project_save", "args": {"sid": "$sid"}})
    run_steps(steps, tmp_path)
    rep = check_exports.check_layers(pdir)
    assert layer_problems(rep) == [], rep
    assert set(rep["tracks"]) == {"T00", "T01", "T02"}
    for name, r in rep["tracks"].items():
        print(f"{name}: {r['layer']['voices']} voices, {r['layer']['checked']} checked")
        assert r["layer"]["checked"] > 0.5 * r["layer"]["voices"]
