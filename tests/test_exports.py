"""P4-4: the MIDI and spike WAV exports, checked by tools/check_exports.py (mido, soundfile, the
bext chunk by hand; no project code) on sessions that this file writes, edited and exported
through the app's own commands (`service_tool run`: the commands of the page, in one service).

Fast cases (G2): the session matrix (mixed rates 44.1 / 48 / 88.2 / 96 kHz in one project, PCM
16/24/32 and Float32 sources, mono and stereo, a non-zero session start in `bext`, edited hits,
a cut hit, overlapping blips, hits on the first and the last sample, hits outside the file, a
follower after an edit of its source, a muted track, a track with no hits), a stale follower
whose audio was offline during the edit, two track names that differ only in case, the Export
All overwrite list, a 10-minute take, the golden fixture Song 1, and the checker itself against
one planted fault per property.

Slow case: `DRUMSPIKE_EXPORT_SESSION=<a real session folder with "Audio Files/">` analyses its
drum tracks with the model, edits one track, makes the kick sub follow the kick in, exports and
checks (the three sessions in ~/.cache/drumspike/sessions on arm64-mac; minutes per session).
"""
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
from pathlib import Path

import numpy as np
import pytest

from core_tools import SERVICE_TOOL, require

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "apps" / "desktop" / "selftest"))
import check_exports  # noqa: E402
from make_project import build_project  # noqa: E402

SONG1 = ROOT / "tests" / "golden" / "project_fixtures" / "Songs" / "Song 1"
FULL = {"pcm16": 32767, "pcm24": 8388607, "pcm32": 2147483647}


# ---------------------------------------------------------------- writing sessions

def write_wav(path, x, sr, fmt, time_reference=None):
    """A WAV file written by hand (fmt, optional bext, data): `x` is (frames, channels) float."""
    x = np.atleast_2d(np.asarray(x, dtype=np.float64).T).T
    frames, ch = x.shape
    if fmt == "float32":
        data, tag, bps = x.astype("<f4").tobytes(), 3, 4
    else:
        full = FULL[fmt]
        q = np.round(np.clip(x, -1, 1) * full).astype("<i4")
        bps = {"pcm16": 2, "pcm24": 3, "pcm32": 4}[fmt]
        data = q.astype("<i2").tobytes() if bps == 2 else (
            q.tobytes() if bps == 4 else q.reshape(-1, 1).view(np.uint8)[:, :3].tobytes())
        tag = 1
    fmt_body = struct.pack("<HHIIHH", tag, ch, sr, sr * ch * bps, ch * bps, 8 * bps)
    chunks = [(b"fmt ", fmt_body)]
    if time_reference is not None:
        b = bytearray(602)
        b[0:12] = b"session take"
        b[256:264] = b"Test DAW"
        b[320:330] = b"2026-09-14"
        b[330:338] = b"10:31:02"
        b[338:346] = struct.pack("<Q", time_reference)
        b[346:348] = struct.pack("<H", 1)
        chunks.append((b"bext", bytes(b)))
    chunks.append((b"data", data))
    body = b"WAVE" + b"".join(cid + struct.pack("<I", len(c)) + c + (b"\0" if len(c) & 1 else b"")
                              for cid, c in chunks)
    Path(path).write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)
    return path


def drum(sr, seconds, idx, level=0.8, channels=1):
    """Silence with a short decaying burst at each sample index."""
    n = int(round(sr * seconds))
    x = np.zeros(n)
    rng = np.random.default_rng(sr + len(idx))
    burst = level * np.exp(-np.arange(int(0.03 * sr)) / (0.006 * sr)) * rng.uniform(0.6, 1.0, int(0.03 * sr))
    for i in idx:
        i = int(i)
        if 0 <= i < n:
            m = min(len(burst), n - i)
            x[i:i + m] += burst[:m]
    x = np.clip(x, -1, 1)
    return np.repeat(x[:, None], channels, axis=1) if channels > 1 else x


def ev(i, sr, **kw):
    return {"t": i / sr, "v": kw.pop("v", 100), **kw}


def run_steps(steps, tmp):
    """`service_tool run`: the answers of the steps, in order."""
    tool = require(SERVICE_TOOL)
    f = Path(tmp) / f"script{len(list(Path(tmp).glob('script*.json')))}.json"
    f.write_text(json.dumps(steps))
    r = subprocess.run([str(tool), "run", str(f)], capture_output=True, text=True, env=dict(os.environ))
    assert r.returncode == 0, r.stderr[-800:]
    out = [json.loads(line) for line in r.stdout.splitlines() if line.strip()]
    assert len(out) == len(steps), r.stdout[-800:]
    return out


def answer(rows, cmd):
    """The answer of the last step `cmd`; fails on an error."""
    row = [r for r in rows if r["cmd"] == cmd][-1]
    assert "answer" in row, row
    return row["answer"]


def problems(rep):
    return rep["problems"] + [f"{n}: {p}" for n, r in rep["tracks"].items() for p in r["problems"]]


# ---------------------------------------------------------------- the session matrix

KICK_SR, SUB_SR, SNARE_SR, TOM_SR = 48_000, 44_100, 96_000, 88_200
KICK_IDX = [0, 4_800, 9_607, 9_617, 24_000, 30_000, 36_000, 40_001, 44_000, 191_999]   # 4 s
KICK_EDITED = [ev(0, KICK_SR), ev(4_800, KICK_SR, v=127), ev(9_607, KICK_SR), ev(9_617, KICK_SR, v=40),
               ev(24_000, KICK_SR), ev(30_000, KICK_SR), ev(36_000, KICK_SR, added=True),
               ev(40_001, KICK_SR, orig=40_000 / KICK_SR), ev(44_000, KICK_SR, v=30, cut=True),
               ev(191_999, KICK_SR)]


def matrix_session(tmp):
    """One project with every case of the matrix; returns its folder."""
    a = Path(tmp) / "audio"
    a.mkdir()
    kick = write_wav(a / "Kick In.wav", drum(KICK_SR, 4, KICK_IDX), KICK_SR, "pcm24", 3600 * KICK_SR)
    sub_idx = [round(i / KICK_SR * SUB_SR) - 400 for i in KICK_IDX]       # a sub mic, 9 ms early, softer
    sub = write_wav(a / "Kick Sub.wav", drum(SUB_SR, 4, sub_idx, 0.3), SUB_SR, "pcm16", 3600 * SUB_SR)
    snare_idx = [1, 9_601, 48_001, 96_001, 287_999]                       # odd samples at 96 kHz
    snare = write_wav(a / "Snare.wav", drum(SNARE_SR, 3, snare_idx, channels=2), SNARE_SR, "pcm32")
    tom_idx = [100, 44_101, 88_200, 264_599]
    tom = write_wav(a / "Tom.wav", drum(TOM_SR, 3, tom_idx), TOM_SR, "float32", 12_345_678)
    hat = write_wav(a / "Hat.wav", drum(48_000, 3, [1_000, 2_000, 50_000]), 48_000, "pcm16")
    empty = write_wav(a / "Empty.wav", np.zeros(96_000), 48_000, "pcm16")
    ride = write_wav(a / "Ride.wav", drum(48_000, 2, [24_000]), 48_000, "pcm16")
    tracks = [
        ("Kick In", kick, "kick", 36, [ev(i, KICK_SR) for i in KICK_IDX]),
        ("Kick Sub", sub, "kick", 35, []),
        ("Snare", snare, "snare", 38, [ev(i, SNARE_SR, v=60 + k) for k, i in enumerate(snare_idx)]),
        ("Tom", tom, "tom", 48, [ev(i, TOM_SR) for i in tom_idx]),
        ("Hat", hat, "hat", 42, [ev(i, 48_000) for i in (1_000, 2_000, 50_000)], {"volume_db": -120, "pan": -1}),
        ("Empty", empty, "other", 60, []),
        ("Ride", ride, "cymbal", 51, [{"t": -0.001, "v": 90}, ev(24_000, 48_000), {"t": 2.5, "v": 90}]),
    ]
    return Path(build_project(tmp, "Matrix", tracks))


@pytest.fixture(scope="module")
def matrix(tmp_path_factory):
    """The matrix session after the page's steps: follow, edit, mute, Export All, Save."""
    tmp = tmp_path_factory.mktemp("matrix")
    pdir = matrix_session(tmp)
    rows = run_steps([
        {"cmd": "project_open", "args": {"path": str(pdir)}},
        {"cmd": "track_set_follow", "args": {"sid": "$sid", "name": "Kick Sub", "source": "Kick In"}},
        {"cmd": "events_set", "args": {"sid": "$sid", "name": "Kick In", "events": KICK_EDITED}},
        {"cmd": "transport_mix", "args": {"mute": {"Hat": True}, "solo": {}}},
        {"cmd": "export_all", "args": {"sid": "$sid"}},
        {"cmd": "project_save", "args": {"sid": "$sid"}},
    ], tmp)
    return pdir, rows, tmp


def test_matrix_exports_have_no_difference(matrix):
    pdir, rows, _ = matrix
    res = answer(rows, "export_all")
    assert res["errors"] == [] and res["midi"] == res["spikes"] == 7, res
    rep = check_exports.check_project(pdir, res)
    assert problems(rep) == []
    t = rep["tracks"]
    assert t["Kick In"]["hits"] == 9 and t["Kick In"]["cut"] == 1           # the cut hit is in neither file
    assert t["Kick In"]["wav"]["blips"] == 9 and t["Kick In"]["source_time_reference"] == 3600 * KICK_SR
    assert t["Kick Sub"]["follows"] == "Kick In" and t["Kick Sub"]["hits"] == 9
    assert t["Tom"]["wav"]["subtype"] == "FLOAT" and t["Tom"]["wav"]["time_reference"] == 12_345_678
    assert t["Snare"]["wav"]["channels"] == 1 and t["Snare"]["wav"]["subtype"] == "PCM_32"
    assert t["Empty"]["hits"] == 0 and t["Empty"]["wav"]["blips"] == 0 and t["Empty"]["midi"]["notes"] == 0
    for name in ("Kick In", "Kick Sub", "Hat", "Ride", "Empty"):                # 44.1 and 48 kHz: on the sample
        assert t[name]["midi"]["rounded_sample_off"] == 0, name
    # 96 kHz: one tick is two samples, so an odd sample is one sample off, within half a tick (P4-8)
    assert t["Snare"]["midi"]["err_max_samples"] == pytest.approx(1.0) == t["Snare"]["midi"]["half_tick_samples"]
    assert t["Snare"]["midi"]["rounded_sample_off"] == 5


def test_hits_outside_the_file_are_counted_in_the_answer(matrix):
    """B1: a hit before the start or after the end has no sample in the spike file; the answers
    say how many (they were skipped with no message)."""
    pdir, rows, _ = matrix
    res = answer(rows, "export_all")
    assert res["outside"] == {"Ride": 2}
    rep = check_exports.check_project(pdir, res)
    assert rep["tracks"]["Ride"]["outside"] == 2 and rep["tracks"]["Ride"]["wav"]["blips"] == 1


def test_a_single_track_export_writes_the_bytes_of_export_all(matrix):
    """The page's Export MIDI and Export Spikes (the page's events, cut marks included) write the
    files Export All wrote."""
    pdir, _, tmp = matrix
    doc = json.loads((pdir / "Matrix.drumspike").read_text())
    before = {p.name: p.read_bytes() for p in (pdir / "Exports").iterdir()}
    steps = [{"cmd": "project_open", "args": {"path": str(pdir)}}]
    for name in doc["order"]:
        tr = doc["tracks"][name]
        steps += [{"cmd": "export_midi", "args": {"sid": "$sid", "name": name, "note": tr["note"],
                                                  "events": tr["events"], "ppq": 24000, "tempo": 120}},
                  {"cmd": "export_spikes", "args": {"sid": "$sid", "name": name, "events": tr["events"]}}]
    rows = run_steps(steps, tmp)
    assert all("answer" in r for r in rows), rows
    assert [r["answer"].get("outside") for r in rows if r["cmd"] == "export_spikes"] == [0, 0, 0, 0, 0, 0, 2]
    after = {p.name: p.read_bytes() for p in (pdir / "Exports").iterdir()}
    assert after == before


def test_mute_and_the_monitor_settings_do_not_change_the_exports(matrix):
    pdir, _, tmp = matrix
    before = {p.name: p.read_bytes() for p in (pdir / "Exports").iterdir()}
    run_steps([{"cmd": "project_open", "args": {"path": str(pdir)}},
               {"cmd": "transport_mix", "args": {"mute": {}, "solo": {"Snare": True}}},
               {"cmd": "export_all", "args": {"sid": "$sid"}}], tmp)
    assert {p.name: p.read_bytes() for p in (pdir / "Exports").iterdir()} == before


def test_export_existing_lists_exactly_the_files_export_all_replaces(tmp_path):
    """The Export All overwrite prompt asks about every file that Export All would replace, and
    only those (P3-4b): after one single spike export the list holds that file only."""
    a = tmp_path / "audio"
    a.mkdir()
    wavs = [write_wav(a / f"{n}.wav", drum(48_000, 1, [100]), 48_000, "pcm16") for n in ("A", "B")]
    pdir = build_project(tmp_path, "Ow", [(n, w, "kick", 36, [ev(100, 48_000)]) for n, w in zip("AB", wavs)])
    (Path(pdir) / "Exports" / "Old_spike.wav").write_bytes(b"not ours")
    rows = run_steps([
        {"cmd": "project_open", "args": {"path": str(pdir)}},
        {"cmd": "export_existing", "args": {"sid": "$sid"}},
        {"cmd": "export_spikes", "args": {"sid": "$sid", "name": "B"}},
        {"cmd": "export_existing", "args": {"sid": "$sid"}},
        {"cmd": "export_all", "args": {"sid": "$sid"}},
        {"cmd": "export_existing", "args": {"sid": "$sid"}},
    ], tmp_path)
    lists = [r["answer"]["files"] for r in rows if r["cmd"] == "export_existing"]
    assert lists[0] == [] and lists[1] == ["Exports/B_spike.wav"]
    written = sorted("Exports/" + Path(f).name for f in answer(rows, "export_all")["files"])
    assert sorted(lists[2]) == written and len(written) == 4


# ---------------------------------------------------------------- found bugs

def test_a_follower_whose_audio_was_offline_during_the_edit_exports_the_edited_hits(tmp_path):
    """B2: the follower is derived again when its source's hits change; when its audio is offline
    at that moment the derivation fails, and Export All wrote the old hits into its MIDI file."""
    a = tmp_path / "audio"
    a.mkdir()
    kick = write_wav(a / "Kick In.wav", drum(48_000, 2, [4_800, 24_000, 48_000]), 48_000, "pcm16")
    sub = write_wav(a / "Kick Sub.wav", drum(48_000, 2, [4_800, 24_000, 48_000, 72_000], 0.3), 48_000, "pcm16")
    pdir = build_project(tmp_path, "Stale", [
        ("Kick In", kick, "kick", 36, [ev(i, 48_000) for i in (4_800, 24_000, 48_000)]),
        ("Kick Sub", sub, "kick", 35, [])])
    edited = [ev(i, 48_000) for i in (4_800, 24_000, 48_000, 72_000)]
    rows = run_steps([
        {"cmd": "project_open", "args": {"path": str(pdir)}},
        {"cmd": "track_set_follow", "args": {"sid": "$sid", "name": "Kick Sub", "source": "Kick In"}},
        {"cmd": "move_file", "args": {"from": str(sub), "to": str(sub) + ".away"}},
        {"cmd": "project_get", "args": {"sid": "$sid"}},
        {"cmd": "events_set", "args": {"sid": "$sid", "name": "Kick In", "events": edited}},
        {"cmd": "move_file", "args": {"from": str(sub) + ".away", "to": str(sub)}},
        {"cmd": "export_all", "args": {"sid": "$sid"}},
        {"cmd": "project_save", "args": {"sid": "$sid"}},
    ], tmp_path)
    res = answer(rows, "export_all")
    assert res["errors"] == [], res
    rep = check_exports.check_project(pdir, res)
    assert problems(rep) == []
    assert rep["tracks"]["Kick Sub"]["midi"]["notes"] == 4


@pytest.mark.parametrize("first,second", [("Kick", "kick"), ("Straße", "STRASSE"), ("ΟΔΟΣ", "οδοσ"),
                                          ("\u00dcber", "U\u0308ber")], ids=["case", "sharp-s", "sigma", "nfc-nfd"])
def test_two_track_names_that_differ_only_in_case_do_not_overwrite_each_other(tmp_path, first, second):
    """B3: `Kick` and `kick` are two tracks but one file on a case-insensitive disk (APFS): Export
    All wrote both into the same files, the second silently over the first, in a race. APFS also
    folds `ß`/`SS`, the final sigma and NFC/NFD (fix round)."""
    a = tmp_path / "audio"
    (a / "x").mkdir(parents=True)
    k1 = write_wav(a / "one.wav", drum(48_000, 1, [100]), 48_000, "pcm16")
    k2 = write_wav(a / "x" / "two.wav", drum(48_000, 1, [200, 300]), 48_000, "pcm24")
    pdir = build_project(tmp_path, "Dup", [(first, k1, "kick", 36, [ev(100, 48_000)]),
                                           (second, k2, "kick", 35, [ev(200, 48_000), ev(300, 48_000)])])
    rows = run_steps([{"cmd": "project_open", "args": {"path": str(pdir)}},
                      {"cmd": "export_all", "args": {"sid": "$sid"}}], tmp_path)
    res = answer(rows, "export_all")
    assert [(e["name"], e["code"]) for e in res["errors"]] == [(second, 409)], res
    assert first in res["errors"][0]["message"]
    rep = check_exports.check_project(pdir, res, expected_errors={second})
    assert rep["tracks"][first]["problems"] == []                            # the first track's files are its own
    assert len(rep["problems"]) == 1 and "case-insensitive" in rep["problems"][0]   # the checker names the clash


# ---------------------------------------------------------------- long take, golden fixture

def test_a_ten_minute_take_keeps_every_hit_on_its_sample(tmp_path):
    """Rounding over a long take: 10 minutes at 44.1 kHz, a hit every 0.37 s on a random sample."""
    sr, n = 44_100, 44_100 * 600
    rng = np.random.default_rng(4)
    idx = np.unique(np.arange(0, n - 2_000, int(0.37 * sr)) + rng.integers(0, 1_000, size=len(range(0, n - 2_000, int(0.37 * sr)))))
    x = np.zeros(n)
    x[idx] = 0.5
    w = write_wav(tmp_path / "Long.wav", x, sr, "pcm16", 3600 * sr)
    pdir = build_project(tmp_path, "Long", [("Long", w, "kick", 36, [ev(int(i), sr) for i in idx])])
    res = answer(run_steps([{"cmd": "project_open", "args": {"path": str(pdir)}},
                            {"cmd": "export_all", "args": {"sid": "$sid"}}], tmp_path), "export_all")
    rep = check_exports.check_project(pdir, res)
    assert problems(rep) == []
    r = rep["tracks"]["Long"]
    assert r["hits"] == len(idx) > 1_500 and r["wav"]["blips"] == len(idx)
    assert r["midi"]["rounded_sample_off"] == 0 and r["midi"]["err_max_samples"] < 0.5


def test_the_golden_project_song_1(tmp_path):
    # the fixture keeps no audio (the freeze left its WAV files out) and its absolute paths point into an
    # old scratch folder of one Mac; write the two sources here (1.5 s, 48 kHz, PCM16 mono as frozen) at the
    # document's relative paths and give the records their identity, so the test runs on any host (CI)
    pdir = tmp_path / "Songs" / "Song 1"
    shutil.copytree(SONG1, pdir)
    for f in (pdir / "Exports").glob("*"):
        f.unlink()
    doc_path = pdir / "Song 1.drumspike"
    doc = json.loads(doc_path.read_text())
    for tr in doc["tracks"].values():
        src = tr["source"]
        wav = (pdir / src["relative"]).resolve()
        wav.parent.mkdir(parents=True, exist_ok=True)
        write_wav(wav, np.zeros(72_000), 48_000, "pcm16")
        st = wav.stat()
        src.update(path=str(wav), size=st.st_size, mtime=int(st.st_mtime),
                   head_sha1=hashlib.sha1(wav.read_bytes()[:1 << 20]).hexdigest())
    doc_path.write_text(json.dumps(doc))
    res = answer(run_steps([{"cmd": "project_open", "args": {"path": str(pdir)}},
                            {"cmd": "export_all", "args": {"sid": "$sid"}}], tmp_path), "export_all")
    rep = check_exports.check_project(pdir, res)
    assert problems(rep) == []
    assert res["outside"] == {"Kick In": 2}


# ---------------------------------------------------------------- the checker can fire

def test_the_checker_holds_the_answer_to_every_track(matrix):
    """A1 (fix round): `outside` is compared for every track (a track the answer leaves out has 0),
    and an answer error is a problem unless the test names it; only a named track may lack files."""
    pdir, rows, _ = matrix
    res = answer(rows, "export_all")
    assert problems(check_exports.check_project(pdir, res)) == []
    no_ride = {**res, "outside": {}}
    assert any("answer: outside 0 != 2" in p for p in problems(check_exports.check_project(pdir, no_ride)))
    extra = {**res, "outside": {**res["outside"], "Hat": 1}}
    assert any("answer: outside 1 != 0" in p for p in problems(check_exports.check_project(pdir, extra)))
    err = {**res, "errors": [{"name": "Hat", "code": 409, "message": "planted"}]}
    assert any("answer: error for Hat" in p for p in problems(check_exports.check_project(pdir, err)))
    assert problems(check_exports.check_project(pdir, err, expected_errors={"Hat"})) == []
    assert any("no error for Hat" in p for p in problems(check_exports.check_project(pdir, res, expected_errors={"Hat"})))


def _midi_shift_first_note(path, ticks):
    import mido
    mid = mido.MidiFile(path)
    tr = mid.tracks[1]
    i = next(k for k, m in enumerate(tr) if m.type == "note_on" and m.velocity > 0)
    tr[i].time += ticks
    nxt = next(k for k in range(i + 1, len(tr)) if tr[k].time >= ticks)
    tr[nxt].time -= ticks
    mid.save(path)


def _midi_edit_first_note(path, **kw):
    import mido
    mid = mido.MidiFile(path)
    m = next(m for m in mid.tracks[1] if m.type == "note_on" and m.velocity > 0)
    for k, v in kw.items():
        setattr(m, k, v)
    mid.save(path)


def _wav_poke(path, frame, value):
    """Overwrite one 24-bit sample of the Kick In spike file (data chunk after fmt and bext)."""
    chunks, _, _ = check_exports.riff_chunks(path)
    at = next(a for cid, a, _ in chunks if cid == b"data")
    with open(path, "r+b") as f:
        f.seek(at + 3 * frame)
        f.write(struct.pack("<i", value)[:3])


def _bext_time(path, value):
    chunks, _, _ = check_exports.riff_chunks(path)
    at = next(a for cid, a, _ in chunks if cid == b"bext")
    with open(path, "r+b") as f:
        f.seek(at + 338)
        f.write(struct.pack("<Q", value))


def _doc_edit(pdir, fn):
    p = pdir / "Matrix.drumspike"
    d = json.loads(p.read_text())
    fn(d)
    p.write_text(json.dumps(d))


PLANTS = {
    "a MIDI note one tick late (48 kHz: one sample)": ("M2", lambda d: _midi_shift_first_note(d / "Exports/Kick In_spike.mid", 1)),
    "a wrong velocity": ("M3", lambda d: _midi_edit_first_note(d / "Exports/Kick In_spike.mid", velocity=99)),
    "a wrong note": ("M3", lambda d: _midi_edit_first_note(d / "Exports/Kick In_spike.mid", note=37)),
    "a wrong channel": ("M3", lambda d: _midi_edit_first_note(d / "Exports/Kick In_spike.mid", channel=0)),
    "a blip one sample late": ("W2", lambda d: (_wav_poke(d / "Exports/Kick In_spike.wav", 4_800, 8_210_000),
                                                _wav_poke(d / "Exports/Kick In_spike.wav", 4_801, 8_388_607))),
    "a blip missing": ("W2", lambda d: [_wav_poke(d / "Exports/Kick In_spike.wav", 24_000 + k, 0) for k in range(48)]),
    "a click between the hits": ("W2", lambda d: _wav_poke(d / "Exports/Kick In_spike.wav", 100_000, 3)),
    "a wrong decay sample": ("W2", lambda d: _wav_poke(d / "Exports/Kick In_spike.wav", 30_010, 100)),
    "a wrong bext time reference": ("W3", lambda d: _bext_time(d / "Exports/Kick In_spike.wav", 3600 * 48_000 + 1)),
    "a spike file one sample short": ("W1", lambda d: _truncate(d / "Exports/Kick In_spike.wav", 3)),
    "a stale follower": ("F1", lambda d: _doc_edit(d, lambda doc: doc["tracks"]["Kick Sub"]["events"].pop())),
    "a hit the MIDI file lacks": ("M1", lambda d: _doc_edit(d, lambda doc: doc["tracks"]["Hat"]["events"].append(
        {"t": 0.5, "v": 100}))),
}


def _truncate(path, nbytes):
    """Cut the last sample off and fix the sizes, so the file stays readable."""
    b = bytearray(path.read_bytes()[:-nbytes])
    struct.pack_into("<I", b, 4, len(b) - 8)
    chunks, _, _ = check_exports.riff_chunks(path)
    data_at = next(a for cid, a, _ in chunks if cid == b"data")
    struct.pack_into("<I", b, data_at - 4, struct.unpack_from("<I", b, data_at - 4)[0] - nbytes)
    path.write_bytes(bytes(b))


@pytest.mark.parametrize("plant", sorted(PLANTS))
def test_the_checker_finds_a_planted_fault(matrix, tmp_path, plant):
    src, _, _ = matrix
    pdir = tmp_path / "Matrix"
    shutil.copytree(src, pdir)
    doc = json.loads((pdir / "Matrix.drumspike").read_text())
    for tr in doc["tracks"].values():   # the copy points at the same audio
        tr["source"]["relative"] = os.path.relpath(tr["source"]["path"], pdir)
    (pdir / "Matrix.drumspike").write_text(json.dumps(doc))
    assert problems(check_exports.check_project(pdir)) == []
    rule, plant_fn = PLANTS[plant]
    plant_fn(pdir)
    found = problems(check_exports.check_project(pdir))
    assert found, f"{rule}: the checker missed {plant}"


# ---------------------------------------------------------------- the audio spikes (#209)

@pytest.fixture(scope="module")
def audio_matrix(tmp_path_factory):
    """The matrix session exported again with the audio spikes (#209): every track gets its
    `<track>_audio_spike.wav`, cut from the track's own audio, next to the blip file."""
    tmp = tmp_path_factory.mktemp("audio")
    pdir = matrix_session(tmp)
    rows = run_steps([{"cmd": "project_open", "args": {"path": str(pdir)}},
                      {"cmd": "export_all", "args": {"sid": "$sid", "formats": ["midi", "spikes", "audio"]}}],
                     tmp)
    return pdir, rows, tmp


def test_the_audio_spikes_start_at_the_hit_samples(audio_matrix):
    pdir, rows, _ = audio_matrix
    res = answer(rows, "export_all")
    assert res["errors"] == [] and res["audio"] == 7, res
    rep = check_exports.check_project(pdir, res)
    assert problems(rep) == []
    for name in ("Kick In", "Snare", "Tom", "Hat", "Ride"):
        a = rep["tracks"][name]["audio"]
        assert a["differences"] == 0 and a["checked"] > 0, (name, a)
        assert a["lengths"] and min(a["lengths"]) >= 2, (name, a)
    # tracks without hits: the file exists, is exact silence, nothing is checkable
    for name in ("Kick Sub", "Empty"):
        a = rep["tracks"][name]["audio"]
        assert a["runs"] == 0 and a["differences"] == 0 and a["lengths"] == [], (name, a)


def _wav_poke_f32(path, frame, value):
    """Overwrite one float32 sample (a FLOAT data chunk)."""
    chunks, _, _ = check_exports.riff_chunks(path)
    at = next(a for cid, a, _ in chunks if cid == b"data")
    with open(path, "r+b") as f:
        f.seek(at + 4 * frame)
        f.write(struct.pack("<f", value))


def _wav_zero_data(path):
    """Zero the whole data chunk of a WAV (every snippet gone)."""
    chunks, _, _ = check_exports.riff_chunks(path)
    at, n = next((a, n) for cid, a, n in chunks if cid == b"data")
    with open(path, "r+b") as f:
        f.seek(at)
        f.write(bytes(n))


def _bext_desc(path, text):
    chunks, _, _ = check_exports.riff_chunks(path)
    at = next(a for cid, a, _ in chunks if cid == b"bext")
    with open(path, "r+b") as f:
        f.seek(at)
        f.write(text.encode().ljust(256, b"\0"))


AUDIO_PLANTS = {
    "an audio snippet one sample late": ("AS2", lambda d: _wav_poke_f32(d / "Exports/Tom_audio_spike.wav", 99, 0.5)),
    "an audio file with every snippet gone": ("AS2", lambda d: _wav_zero_data(d / "Exports/Hat_audio_spike.wav")),
    "a wrong audio bext description": ("AS1", lambda d: _bext_desc(d / "Exports/Tom_audio_spike.wav", "not the audio spikes")),
}


@pytest.mark.parametrize("plant", sorted(AUDIO_PLANTS))
def test_the_checker_finds_a_planted_audio_fault(audio_matrix, tmp_path, plant):
    src, _, _ = audio_matrix
    pdir = tmp_path / "Matrix"
    shutil.copytree(src, pdir)
    doc = json.loads((pdir / "Matrix.drumspike").read_text())
    for tr in doc["tracks"].values():   # the copy points at the same audio
        tr["source"]["relative"] = os.path.relpath(tr["source"]["path"], pdir)
    (pdir / "Matrix.drumspike").write_text(json.dumps(doc))
    assert problems(check_exports.check_project(pdir)) == []
    rule, plant_fn = AUDIO_PLANTS[plant]
    plant_fn(pdir)
    found = problems(check_exports.check_project(pdir))
    assert found, f"{rule}: the checker missed {plant}"


# ---------------------------------------------------------------- slow: a real session

STANDARD = {"k in": ("kick", 36), "k out": ("kick", 36), "k sub": ("kick", 35), "s top": ("snare", 38),
            "s bot": ("snare", 38), "t1": ("tom", 50), "t2": ("tom", 47), "t3": ("tom", 45), "t4": ("tom", 41)}


@pytest.mark.skipif(not os.environ.get("DRUMSPIKE_EXPORT_SESSION"),
                    reason="slow: set DRUMSPIKE_EXPORT_SESSION to a real session folder (Audio Files/ inside)")
def test_a_real_session_after_analysis_edit_and_follow(tmp_path):
    session = Path(os.environ["DRUMSPIKE_EXPORT_SESSION"])
    tracks = []
    for f in sorted((session / "Audio Files").glob("*.wav")):
        stem = f.stem.rsplit("_", 1)[0].lower()
        key = next((k for k in STANDARD if stem == k or stem.startswith(k + " ")), None)
        if key and key not in [t[0] for t in tracks]:
            tracks.append((key, str(f), *STANDARD[key]))
    assert {"k in", "k sub"} <= {t[0] for t in tracks}, tracks
    pdir = Path(build_project(tmp_path, "Real", tracks))
    rows = run_steps([{"cmd": "project_open", "args": {"path": str(pdir)}},
                      {"cmd": "analyze_start", "args": {"sid": "$sid"}},
                      {"cmd": "wait_analysis", "args": {"sid": "$sid"}},
                      {"cmd": "project_save", "args": {"sid": "$sid"}}], tmp_path)
    assert answer(rows, "wait_analysis")["state"] == "done", rows[-2]
    doc = json.loads((pdir / "Real.drumspike").read_text())
    kin, sr = doc["tracks"]["k in"]["events"], doc["tracks"]["k in"]["sr"]
    assert len(kin) > 50
    edited = [dict(e) for e in kin[1:]]                                         # delete the first hit,
    edited[0]["t"] = (round(edited[0]["t"] * sr) + 7) / sr                      # move one by 7 samples,
    edited[0]["orig"] = kin[1]["t"]
    edited[1]["v"] = 127                                                        # set a velocity,
    edited.append({"t": (round(kin[-1]["t"] * sr) + sr) / sr, "v": 100, "added": True})   # add one
    rows = run_steps([
        {"cmd": "project_open", "args": {"path": str(pdir)}},
        {"cmd": "track_set_follow", "args": {"sid": "$sid", "name": "k sub", "source": "k in"}},
        {"cmd": "events_set", "args": {"sid": "$sid", "name": "k in", "events": edited}},
        {"cmd": "export_all", "args": {"sid": "$sid"}},
        {"cmd": "project_save", "args": {"sid": "$sid"}},
    ], tmp_path)
    res = answer(rows, "export_all")
    rep = check_exports.check_project(pdir, res)
    out = Path(os.environ.get("DRUMSPIKE_EXPORT_REPORT", tmp_path / "report.json"))
    out.write_text(json.dumps(rep, indent=1, default=float))
    assert res["errors"] == [] and problems(rep) == []
    assert rep["tracks"]["k sub"]["hits"] == len(edited)


# ---------------------------------------------------------------- P4-8 stage A: the tempo and meter map

# Bytes of engine.export_midi (pretty_midi 0.2.11, mido 1.3.3) for the hits 0.5 s v90, 0.52 s v100, 0 s v1 on
# note 36 at 24,000 PPQ and 120 BPM: the same bytes as before the tempo map (`crates/core/src/midi.rs`).
PYTHON_MIDI = bytes.fromhex(
    "4d54686400000006000100025dc04d54726b0000001300ff510307a12000ff58040402180801ff2f00"
    "4d54726b0000002a00ff03067370696b657300c900009924019260240081a860245a874024648b2024008740240001ff2f00")

MAP = {"bpm": 120.0, "steps": [{"t": 10.0, "bpm": 95.0}, {"t": 20.0, "bpm": 140.0}, {"t": 33.3, "bpm": 72.5}],
       "meter": [4, 4], "changes": [{"bar": 6, "meter": [3, 4]}, {"bar": 12, "meter": [7, 8]}], "bar1_s": 1.5}


def tempo_session(tmp, tempo_map, sr=48_000):
    """One kick track of 46 s with 70 hits (odd sample positions, one before bar 1, one before 0 s), the map
    set and the project exported with Export All through the app's commands; returns (folder, answer)."""
    a = Path(tmp) / "audio"
    a.mkdir()
    idx = [int(i) for i in np.linspace(777, 45 * sr + 13, 70)]
    wav = write_wav(a / "Kick In.wav", drum(sr, 46, idx), sr, "pcm24")
    hits = [{"t": -0.002, "v": 90}] + [ev(i, sr) for i in idx]
    pdir = Path(build_project(tmp, "Tempo", [("Kick In", wav, "kick", 36, hits)]))
    steps = [{"cmd": "project_open", "args": {"path": str(pdir)}}]
    if tempo_map is not None:
        steps.append({"cmd": "tempo_map_set", "args": {"sid": "$sid", "map": tempo_map}})
    steps += [{"cmd": "export_all", "args": {"sid": "$sid"}}, {"cmd": "project_save", "args": {"sid": "$sid"}}]
    rows = run_steps(steps, tmp)
    res = answer(rows, "export_all")
    assert res["errors"] == [], res
    return pdir, res


@pytest.fixture(scope="module")
def tempo_export(tmp_path_factory):
    return tempo_session(tmp_path_factory.mktemp("tempo"), MAP)


def test_a_map_with_three_tempo_steps_and_a_meter_change_exports_every_note_within_half_a_tick(tempo_export):
    pdir, res = tempo_export
    rep = check_exports.check_project(pdir, res)
    assert problems(rep) == []
    m = rep["tracks"]["Kick In"]["midi"]
    assert m["notes"] == 71 and len(m["tempo_us"]) == 4 and m["meter_events"] == 3
    assert m["err_max_ticks"] <= m["err_bound_ticks"] + 1e-6
    assert m["err_max_samples"] <= m["err_bound_samples"] + 1e-6
    assert json.loads((pdir / "Tempo.drumspike").read_text())["format"] == 1


def rewrite(src, dst, drop):
    """A copy of a MIDI file without the messages of track 0 for which `drop(msg)` is true (their delta
    times move to the next message, so every other event keeps its tick)."""
    import mido
    mid = mido.MidiFile(src)
    carry, kept = 0, []
    for msg in mid.tracks[0]:
        if drop(msg):
            carry += msg.time
        else:
            kept.append(msg.copy(time=msg.time + carry))
            carry = 0
    mid.tracks[0] = mido.MidiTrack(kept)
    mid.save(dst)


def planted(tmp_path, tempo_export, drop=None, doc_edit=None):
    """The checker's problems for a copy of the tempo session with one planted fault."""
    pdir, res = tempo_export
    copy = tmp_path / "copy"
    shutil.copytree(pdir, copy)
    if drop:
        mid = copy / "Exports" / "Kick In_spike.mid"
        rewrite(mid, mid, drop)
    if doc_edit:
        f = copy / "Tempo.drumspike"
        doc = json.loads(f.read_text())
        doc_edit(doc["tempo_map"])
        f.write_text(json.dumps(doc))
    return problems(check_exports.check_project(copy, res))


def test_planted_a_missing_tempo_step_is_found(tmp_path, tempo_export):
    p = planted(tmp_path, tempo_export, drop=lambda m: m.type == "set_tempo" and m.tempo == 631_579)
    assert any("tempo track" in x for x in p) and any("samples from the hit" in x for x in p), p


def test_planted_a_missing_bar_1_marker_and_a_wrong_meter_are_found(tmp_path, tempo_export):
    p = planted(tmp_path, tempo_export, drop=lambda m: m.type == "marker")
    assert any("no Bar 1 marker" in x for x in p), p
    p = planted(tmp_path / "b", tempo_export, drop=lambda m: m.type == "time_signature" and m.numerator == 7)
    assert any("time signatures" in x for x in p), p


def test_planted_a_document_map_that_the_file_does_not_hold_is_found(tmp_path, tempo_export):
    p = planted(tmp_path, tempo_export, doc_edit=lambda m: m["steps"][1].update(bpm=150.0))
    assert any("tempo track: event 2" in x for x in p), p
    p = planted(tmp_path / "b", tempo_export, doc_edit=lambda m: m.update(bar1_s=2.0))
    assert any("Bar 1 marker" in x for x in p), p
    p = planted(tmp_path / "c", tempo_export, doc_edit=lambda m: m["changes"][0].update(bar=7))
    assert any("time signatures" in x for x in p), p


def test_planted_a_one_tick_drift_in_the_tempo_session_is_found(tmp_path, tempo_export):
    """#479: the two-view check stays strict: a note moved one tick in the file is more than half
    a tick off in the DAW view (the map's exact tempos), also through an integer set_tempo."""
    pdir, res = tempo_export
    copy = tmp_path / "copy"
    shutil.copytree(pdir, copy)
    mid = copy / "Exports" / "Kick In_spike.mid"
    _midi_shift_first_note(mid, 1)
    p = problems(check_exports.check_project(copy, res))
    assert any("samples from the hit" in x for x in p), p


def test_a_song_whose_bar_1_is_not_at_0_s_and_a_hit_before_bar_1(tmp_path):
    """bar 1 at 3.25 s; the hits at 0.016 s and 1.0 s come before it, the hit at -0.002 s before the audio."""
    pdir, res = tempo_session(tmp_path, {"bpm": 100.0, "bar1_s": 3.25, "meter": [6, 8]})
    rep = check_exports.check_project(pdir, res)
    assert problems(rep) == []
    ppq, tempos, meta, notes, _ = check_exports.midi_notes(pdir / "Exports" / "Kick In_spike.mid")
    assert meta["markers"] == [(130_000, "Bar 1")]                       # 3.25 s at 100 BPM, 24,000 PPQ
    assert meta["sigs"] == [(0, 6, 8)] and tempos == [(0, 600_000)]
    assert notes[0]["on_tick"] == 0 and notes[1]["on_tick"] in (647, 648)         # 777 samples = 647.5 ticks


def test_the_default_map_writes_the_bytes_of_before(tmp_path):
    a = tmp_path / "audio"
    a.mkdir()
    wav = write_wav(a / "Kick In.wav", drum(48_000, 1, [24_000]), 48_000, "pcm16")
    hits = [{"t": 0.5, "v": 90}, {"t": 0.52}, {"t": 0.0, "v": 1}]
    pdir = Path(build_project(tmp_path, "Plain", [("Kick In", wav, "kick", 36, hits)]))
    rows = run_steps([{"cmd": "project_open", "args": {"path": str(pdir)}},
                      {"cmd": "tempo_map_set", "args": {"sid": "$sid", "map": MAP}},
                      {"cmd": "tempo_map_set", "args": {"sid": "$sid", "map": {}}},
                      {"cmd": "export_all", "args": {"sid": "$sid"}}], tmp_path)
    assert answer(rows, "export_all")["errors"] == []
    assert (pdir / "Exports" / "Kick In_spike.mid").read_bytes() == PYTHON_MIDI


# ---------------------------------------------------------------- P4-40g: the review markers

def marker_session(tmp, on, sr=48_000):
    """One kick track with three hits and two review markers, exported as the one multi-track file
    with `export_markers` as given; returns (folder, answer)."""
    a = Path(tmp) / "audio"
    a.mkdir()
    idx = [12_000, 36_000, 72_000]
    wav = write_wav(a / "Kick In.wav", drum(sr, 2, idx), sr, "pcm24")
    pdir = Path(build_project(tmp, "Marks", [("Kick In", wav, "kick", 36, [ev(i, sr) for i in idx])]))
    rows = run_steps([{"cmd": "project_open", "args": {"path": str(pdir)}},
                      {"cmd": "markers_set",
                       "args": {"sid": "$sid", "markers": [{"t": 0.5, "note": "verse"}, {"t": 1.5}]}},
                      {"cmd": "export_markers_set", "args": {"sid": "$sid", "on": on}},
                      {"cmd": "export_all", "args": {"sid": "$sid", "midi_mode": "single"}},
                      {"cmd": "project_save", "args": {"sid": "$sid"}}], tmp)
    res = answer(rows, "export_all")
    assert res["errors"] == [], res
    return pdir, res


def test_review_markers_reach_the_multi_track_file_and_the_checker_holds_them(tmp_path):
    pdir, res = marker_session(tmp_path, True)
    assert problems(check_exports.check_project(pdir, res)) == []
    _, _, meta, _, _ = check_exports.midi_notes(pdir / "Exports" / "Marks_drums.mid")
    # the default export tempo (120): 2 quarters per second, so 0.5 s is 24,000 ticks at 24,000 PPQ;
    # no Bar 1 marker (the default map has bar 1 at 0 s)
    assert meta["markers"] == [(24_000, "verse"), (72_000, "Marker 2")]


def test_the_box_off_leaves_no_review_marker_in_the_multi_track_file(tmp_path):
    pdir, res = marker_session(tmp_path, False)
    assert problems(check_exports.check_project(pdir, res)) == []
    _, _, meta, _, _ = check_exports.midi_notes(pdir / "Exports" / "Marks_drums.mid")
    assert meta["markers"] == []                  # no Bar 1 (bar 1 at 0 s) and no review markers


def planted_markers(tmp_path, on, drop=None, doc_edit=None):
    """The checker's problems for a copy of the marker session with one planted fault."""
    pdir, res = marker_session(tmp_path, on)
    copy = tmp_path / "copy"
    shutil.copytree(pdir, copy)
    if drop:
        mid = copy / "Exports" / "Marks_drums.mid"
        rewrite(mid, mid, drop)
    if doc_edit:
        f = copy / "Marks.drumspike"
        doc = json.loads(f.read_text())
        doc_edit(doc)
        f.write_text(json.dumps(doc))
    return problems(check_exports.check_project(copy, res))


def test_planted_a_missing_review_marker_is_found(tmp_path):
    p = planted_markers(tmp_path, True, drop=lambda m: m.type == "marker" and m.text == "verse")
    assert any("no marker 'verse'" in x for x in p), p


def test_planted_a_review_marker_while_the_box_is_off_is_found(tmp_path):
    p = planted_markers(tmp_path, True, doc_edit=lambda doc: doc.update(export_markers=False))
    assert any("although Markers in MIDI is off" in x for x in p), p


def test_planted_a_review_marker_without_a_doc_entry_is_found(tmp_path):
    p = planted_markers(tmp_path, True, doc_edit=lambda doc: doc.update(markers=[]))
    assert any("'verse' is no review marker" in x for x in p), p
