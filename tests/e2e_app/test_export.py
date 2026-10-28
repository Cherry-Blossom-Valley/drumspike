"""P3-3c: spike WAV export, Export all and velocities in the app, through the bridge. The page runs the
IPC backend (B.app); the commands are the Rust ones of crates/app. The references are frozen (P3-12,
tests/golden/frozen/): the spike files of drumspike_core.write_spike_wav, the MIDI files of engine._py_export_midi
(pretty_midi) and the velocities of engine.recompute_velocities, each for the same inputs."""
import struct
from pathlib import Path

import pytest
from playwright.sync_api import expect

import frozen
from app_support import export_dialog, invoke, midi_reference, open_project, spike_reference, track_menu, velocities_reference
from helpers import synth_track, write_wav

TIME_REF = 5_000_000_123      # above 2^32: both halves of the bext field are used
UNSUPPORTED_TEXT = "This file is not supported. Drum Spike reads WAV files with 44.1, 48 or 96 kHz."


def add_bext(path, time_reference):
    """Insert a bext chunk (602 bytes) after the fmt chunk of a wave-module file."""
    b = bytearray(Path(path).read_bytes())
    assert b[12:16] == b"fmt " and b[36:40] == b"data"
    body = bytearray(602)
    body[0:9] = b"take 3 kk"
    body[256:265] = b"Pro Tools"
    body[320:330] = b"2026-09-14"
    body[330:338] = b"10:31:02"
    body[338:346] = struct.pack("<Q", time_reference)
    body[346:348] = struct.pack("<H", 2)
    chunk = b"bext" + struct.pack("<I", len(body)) + bytes(body)
    b[36:36] = chunk
    b[4:8] = struct.pack("<I", len(b) - 8)
    Path(path).write_bytes(bytes(b))


def bext_time_reference(path):
    b = Path(path).read_bytes()
    pos = 12
    while pos + 8 <= len(b):
        cid, size = b[pos:pos + 4], struct.unpack("<I", b[pos + 4:pos + 8])[0]
        if cid == b"bext":
            return struct.unpack("<Q", b[pos + 8 + 338:pos + 8 + 346])[0]
        pos += 8 + size + (size % 2)
    return None


@pytest.fixture
def bext_project(tmp_path, make_project):
    """A copied project: Kick In has a bext chunk with a time reference, Snare Top has none."""
    src = tmp_path / "bext_src"
    src.mkdir()
    specs = []
    for name, role, note, tref in (("Kick In", "kick", 36, TIME_REF), ("Snare Top", "snare", 38, None)):
        x, _ = synth_track([0.5, 1.5, 2.5], 4.0)
        wav = write_wav(src / f"{name}.wav", x)
        if tref:
            add_bext(wav, tref)
        specs.append((name, wav, role, note))
    return Path(make_project.builder.build_project(tmp_path / "bext_projects", "Song", specs, "copied"))


def source_of(folder, name):
    return next(Path(folder, "Audio Files").glob(f"{name}*.wav"))


def analyse(app):
    app.click("#analyzeAllBtn")
    expect(app.locator("#progressText")).to_have_text("analysis done", timeout=60000)


def pin_frozen_snare(app):
    """P3-18: the synthetic kick under the snare role gave 0 hits at the default thresholds when the references were
    frozen; the 0-hit retry now lowers the thresholds until the snare class fires on it. The export tests check the
    export bytes for the frozen case, so Snare Top gets the frozen events (none) back."""
    app.evaluate("async () => { await B.setEvents(S.sid, 'Snare Top', {events: []}); await loadEventsOnly(); }")
    assert app.evaluate("() => S.events['Snare Top'].length") == 0


def test_export_spikes_button_writes_the_file_of_the_core_with_the_time_reference(app, bext_project):
    open_project(app, bext_project)
    analyse(app)
    pin_frozen_snare(app)
    sid = app.evaluate("() => S.sid")
    for name, tref in (("Kick In", TIME_REF), ("Snare Top", None)):
        times = app.evaluate("n => S.events[n].map(e => e.t)", name)
        if times:   # the dialog exports tracks with hits; the frozen snare has none (P4-30): the command writes its empty file
            track_menu(app, name, "spikes")
            expect(app.locator("#stMsg")).to_contain_text("exported 1 spike file to Exports/ in the project folder")
        else:
            invoke(app, "export_spikes", sid=sid, name=name)
        out = Path(bext_project, "Exports", f"{name}_spike.wav")
        assert len(times) >= 3 or name == "Snare Top"      # the synthetic kick is not a snare hit
        ref = spike_reference(f"export_spikes/{name}", source_of(bext_project, name), times)
        assert frozen.same(frozen.wav_summary(out.read_bytes()), ref)
        assert ref["samples"] == 4 * 48000
        assert bext_time_reference(out) == tref == bext_time_reference(source_of(bext_project, name))
    assert "export_spikes" in app.invokes


def test_the_dialog_writes_midi_and_spikes_of_the_tracks_with_hits(app, bext_project):
    open_project(app, bext_project)
    assert app.evaluate("() => document.querySelector('#exportAllBtn').textContent") == "⬇"
    assert "Export" in app.evaluate("() => document.querySelector('#exportAllBtn').title")
    analyse(app)
    pin_frozen_snare(app)
    export_dialog(app, close=False)
    expect(app.locator("#stMsg")).to_contain_text("exported 1 MIDI and 1 spike file to Exports/", timeout=30000)
    expect(app.locator("#exSkipped")).to_have_text("Skipped: Snare Top (no hits)")
    assert app.evaluate("() => document.querySelector('#stMsg').title") == ""
    exports = Path(bext_project, "Exports")
    assert sorted(p.name for p in exports.iterdir() if p.name != ".drag") == ["Kick In_spike.mid", "Kick In_spike.wav"]   # the hidden .drag folder stays (#433)
    events = app.evaluate("n => S.events[n].map(e => ({t: e.t, v: e.v}))", "Kick In")
    assert (exports / "Kick In_spike.mid").read_bytes() == midi_reference("export_all/Kick In", events, 36)
    ref = spike_reference("export_all/Kick In", source_of(bext_project, "Kick In"), [e["t"] for e in events])
    assert frozen.same(frozen.wav_summary((exports / "Kick In_spike.wav").read_bytes()), ref)
    expect(app.locator("#progressText")).to_contain_text("export done")
    assert "export_all" in app.invokes
    # the command itself still writes the files of a track without hits (the frozen snare): the agents and the CLI use it
    sid = app.evaluate("() => S.sid")
    invoke(app, "export_all", sid=sid, tracks=["Snare Top"])
    assert (exports / "Snare Top_spike.mid").read_bytes() == midi_reference("export_all/Snare Top", [], 38)
    ref = spike_reference("export_all/Snare Top", source_of(bext_project, "Snare Top"), [])
    assert frozen.same(frozen.wav_summary((exports / "Snare Top_spike.wav").read_bytes()), ref)


def test_export_all_sends_edits_that_are_still_waiting(app, bext_project):
    """A hit changed just before Export all is in the files: the page flushes its pending write first."""
    open_project(app, bext_project)
    app.evaluate("() => { const n = 'Kick In'; S.events[n] = [{t: 1.0, v: 90}, {t: 2.0, v: 80}]; markDirty(n); }")
    export_dialog(app, formats=("midi",))
    expect(app.locator("#stMsg")).to_contain_text("exported 1 MIDI file", timeout=30000)
    ref = midi_reference("export_all_pending/Kick In", [{"t": 1.0, "v": 90}, {"t": 2.0, "v": 80}], 36)
    assert Path(bext_project, "Exports", "Kick In_spike.mid").read_bytes() == ref


def test_export_all_reports_a_track_that_fails_and_keeps_the_rest(app, make_project):
    folder = make_project(mode="copied")
    open_project(app, folder)
    source_of(folder, "Snare Top").unlink()
    sid = app.evaluate("() => S.sid")
    invoke(app, "events_set", sid=sid, name="Kick In", events=[{"t": 0.5, "v": 90}, {"t": 1.5, "v": 90}])
    app.evaluate("async () => { await loadProject(); }")
    # the dialog lists an offline track as skipped (P4-30); the command reports it as a failed track and keeps the rest
    export_dialog(app, close=False)
    expect(app.locator("#exSkipped")).to_have_text("Skipped: Snare Top (offline)")
    app.click("#exCancel")
    r = invoke(app, "export_all", sid=sid)
    assert [e["name"] for e in r["errors"]] == ["Snare Top"]
    names = sorted(p.name for p in Path(folder, "Exports").iterdir() if p.is_file())
    assert names == ["Kick In_spike.mid", "Kick In_spike.wav", "Snare Top_spike.mid"]


def test_export_spikes_of_an_offline_track_says_so(app, make_project):
    folder = make_project(mode="copied")
    open_project(app, folder)
    source_of(folder, "Snare Top").unlink()
    app.evaluate("async () => { await loadProject(); }")
    r = invoke(app, "export_spikes", sid=app.evaluate("() => S.sid"), name="Snare Top")
    assert r["code"] == 409
    assert not Path(folder, "Exports", "Snare Top_spike.wav").exists()


def test_recompute_velocities_equals_the_engine(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    sid = open_project(app, folder)
    events = [{"t": 0.5, "v": 1, "added": True}, {"t": 1.5, "v": 127}, {"t": 2.5, "id": "l9"}, {"t": 99.0}]
    got = app.evaluate("async ([sid, ev]) => (await B.recomputeVelocities(sid, 'Kick In', ev)).events", [sid, events])
    want = velocities_reference("recompute/Kick In", source_of(folder, "Kick In"), events)
    assert got == want and [list(e) for e in got] == [list(e) for e in want]
    assert got[0]["v"] > 20 and got[3]["v"] == 20
    # without a list the events of the document are used
    app.evaluate("async ([sid, ev]) => B.setEvents(sid, 'Kick In', {events: ev})", [sid, events])
    again = invoke(app, "velocities_recompute", sid=sid, name="Kick In")
    assert again["events"] == want
    assert invoke(app, "velocities_recompute", sid=sid, name="nope")["code"] == 400


@pytest.fixture
def mpeg_tag_project(tmp_path, make_project):
    """A project whose only file has the WAV format tag of MPEG audio (85): the core cannot read it."""
    src = tmp_path / "odd"
    src.mkdir()
    x, _ = synth_track([0.5, 1.5], 3.0)
    wav = write_wav(src / "Kick In.wav", x)
    b = bytearray(wav.read_bytes())
    b[20:22] = struct.pack("<H", 85)
    wav.write_bytes(bytes(b))
    return Path(make_project.builder.build_project(tmp_path / "odd_projects", "Odd", [("Kick In", wav, "kick", 36)], "copied"))


def test_an_unsupported_file_shows_the_clear_text_only_and_keeps_the_detail_for_a_tooltip(app, mpeg_tag_project):
    warnings = []
    app.on("console", lambda m: warnings.append(m.text) if m.type == "warning" else None)
    open_project(app, mpeg_tag_project)
    # the lane: the clear text, no core text in it
    lane = app.evaluate("() => S.peakErr['Kick In']")
    assert lane.startswith(UNSUPPORTED_TEXT) and "unsupported format" not in lane and "wav format" not in lane, lane
    assert "unsupported format tag 85" in app.evaluate("() => document.querySelector('.track canvas.wave').title")
    # the status line and the track header after Analyze
    app.click("#analyzeAllBtn")
    expect(app.locator("#stMsg")).to_contain_text("1 of 1 tracks failed", timeout=60000)
    msg = app.evaluate("() => document.querySelector('#stMsg').textContent")
    assert UNSUPPORTED_TEXT in msg and "wav format" not in msg and "tag 85" not in msg, msg
    assert "tag 85" in app.evaluate("() => document.querySelector('#stMsg').title")
    mark = app.evaluate("() => document.querySelector('.track .terrmark').textContent")
    assert "wav format" not in mark and "tag 85" not in mark and UNSUPPORTED_TEXT in mark
    assert "tag 85" in app.evaluate("() => document.querySelector('.track .terrmark').title")
    assert "tag 85" in app.evaluate("() => document.querySelector('#stDet').title")
    # the detail goes to the log too
    assert any("unsupported format tag 85" in w for w in warnings), warnings
    # the Export dialog says the same (it exports tracks with hits: give the kick two)
    sid = app.evaluate("() => S.sid")
    invoke(app, "events_set", sid=sid, name="Kick In", events=[{"t": 0.5, "v": 90}, {"t": 1.5, "v": 90}])
    app.evaluate("async () => { await loadEventsOnly(); }")
    export_dialog(app, formats=("spikes",))
    expect(app.locator("#stMsg")).to_contain_text("1 failed (Kick In: " + UNSUPPORTED_TEXT, timeout=30000)
    assert "tag 85" in app.evaluate("() => document.querySelector('#stMsg').title")


# ---------- P4-4: hits outside the audio file, a follower that was stale ----------

def test_the_messages_count_the_hits_outside_the_audio_file(app, make_project):
    """B1 (P4-4): a hit before the start or after the end of the file has no sample in the spike file;
    the messages say how many (they were skipped without a word). The MIDI message names the hits before
    the start apart: the MIDI file puts them at 0 s."""
    folder = make_project()
    sid = open_project(app, folder)
    invoke(app, "events_set", sid=sid, name="Kick In",
           events=[{"t": -0.5, "v": 60}, {"t": 0.5, "v": 90}, {"t": 9.0, "v": 80}, {"t": 12.0, "v": 70}])
    invoke(app, "events_set", sid=sid, name="Snare Top", events=[{"t": 0.5, "v": 90}])
    app.evaluate("async () => { await loadEventsOnly(); }")
    track_menu(app, "Kick In", "spikes")
    expect(app.locator("#stMsg")).to_contain_text("exported 1 spike file to Exports/ in the project folder")
    expect(app.locator("#stMsg")).to_contain_text("hits outside the audio file, not in the spike files: Kick In 3 (the MIDI files keep them; a hit before the start sits at 0 s there)")
    track_menu(app, "Snare Top", "spikes")
    expect(app.locator("#stMsg")).not_to_contain_text("outside")
    # the command still says how the MIDI file keeps them
    r = invoke(app, "export_midi", sid=sid, name="Kick In")
    assert (r["outside"], r["before"]) == (3, 1)
    export_dialog(app)
    expect(app.locator("#stMsg")).to_contain_text("hits outside the audio file, not in the spike files: Kick In 3 (the MIDI files keep them", timeout=30000)


def test_export_all_derives_a_stale_follower_and_the_page_shows_its_hits(app, make_project):
    """B2 (P4-4): the follower's audio was offline when its source was edited; Export All derives it again
    (it wrote the old hits before) and the page reloads the follower's hits."""
    folder = make_project()
    sid = open_project(app, folder)
    invoke(app, "events_set", sid=sid, name="Kick In", events=[{"t": t, "v": 100} for t in (0.5, 1.0, 1.5)])
    assert invoke(app, "track_set_follow", sid=sid, name="Snare Top", source="Kick In")["follows"] == "Kick In"
    app.evaluate("async () => { await loadProject(); }")
    n0 = app.evaluate("() => S.events['Snare Top'].length")
    snare = source_of(folder, "Snare Top")
    away = snare.with_suffix(".away")
    snare.rename(away)
    invoke(app, "project_get", sid=sid)                                   # the service sees it offline
    edited = [{"t": t, "v": 100} for t in (0.5, 1.0, 1.5, 2.0, 2.5)]
    invoke(app, "events_set", sid=sid, name="Kick In", events=edited)
    away.rename(snare)
    assert app.evaluate("() => S.events['Snare Top'].length") == n0       # the page still shows the old hits
    export_dialog(app)
    expect(app.locator("#stMsg")).to_contain_text("exported 2 MIDI and 2 spike files", timeout=30000)
    app.wait_for_function("() => S.events['Snare Top'].length === 5")
