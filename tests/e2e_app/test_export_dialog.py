"""P4-30: the one Export dialog (#exportModal), through the bridge. File > Export... (Cmd+E) and the toolbar arrow open it for
all tracks, Track > Export Selected Tracks... and the track header menu with the selected track. The dialog picks the tracks,
the formats (MIDI per track or one multi-track file, spike WAV) and the destination folder; it says how many files the export
replaces and has the drag tiles that start the native file drag (mocked in the bridge: window.__DS_DRAG__).

Each test has a planted-fault half: a control that shows the check can fail (the same action without the condition under
test gives the other result), as the other e2e files do with their `the_check_can_fail` tests."""
import hashlib
import json
import re
from pathlib import Path

import mido
from playwright.sync_api import expect

from app_support import ex_confirm_tempo, ex_run, invoke, menu, open_project, wait_until
from test_shell import with_hits


def project_with_hits(app, make_project, **kw):
    folder = make_project(**kw)
    sid = open_project(app, folder)
    with_hits(app, sid)
    with_hits(app, sid, "Snare Top")
    return sid, Path(folder)


def exports(folder):
    d = folder / "Exports"
    return sorted(p.name for p in d.iterdir() if p.is_file()) if d.exists() else []


def modal_visible(app):
    return app.evaluate("() => !document.querySelector('#exportModal').classList.contains('hidden')")


def run_and_close(app):
    ex_run(app)   # #172: the export-finished event (EX.done), not the busy flag (a click's handler runs a task later)
    app.click("#exCancel")


# ---------- the menu opens the dialog ----------

def test_the_menu_items_and_the_toolbar_arrow_open_the_dialog(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    assert not modal_visible(app)                                            # control: closed until an item opens it
    menu(app, "export")
    expect(app.locator("#exportModal")).to_be_visible()
    expect(app.locator("#exScopeAll")).to_be_checked()
    expect(app.locator("#exMidi")).to_be_checked()
    expect(app.locator("#exSpikes")).to_be_checked()
    expect(app.locator("#exModeTrack")).to_be_checked()
    expect(app.locator("#exAllText")).to_have_text("All tracks (2)")
    assert app.locator("#exDest").input_value() == str(folder / "Exports")
    assert app.locator("#exDest").get_attribute("readonly") is not None      # read-only text, selectable for copy
    app.click("#exCancel")
    expect(app.locator("#exportModal")).to_be_hidden()
    app.click("#exportAllBtn")
    expect(app.locator("#exportModal")).to_be_visible()
    app.click("#exCancel")
    # Track > Export Selected Tracks... needs a selected track and opens with it
    app.evaluate("() => setSelTrack(null)")                                  # P4-39c: a project with tracks always has a selected track (the first)
    menu(app, "export_selected")
    expect(app.locator("#exSelText")).to_have_text("Selected track (1): Kick In")
    app.click("#exCancel")
    app.evaluate("() => setSelTrack('Snare Top')")
    menu(app, "export_selected")
    expect(app.locator("#exScopeSel")).to_be_checked()
    expect(app.locator("#exSelText")).to_have_text("Selected track (1): Snare Top")
    app.click("#exCancel")
    # the removed items are gone (the menu runs nothing for them)
    for gone in ("export_all", "export_midi", "export_spikes"):
        assert menu(app, gone) is False
    assert exports(folder) == []                                             # nothing was written by opening


def test_the_removed_menu_ids_and_the_keymap_rows(app):
    rows = {r["id"]: r for r in app.evaluate("() => DSKeys.COMMANDS")}
    assert rows["export"]["menu_key"] == "Cmd+E" and rows["export_selected"]["path"] == "Track"
    assert not {"export_all", "export_midi", "export_spikes", "import_zip", "export_zip"} & set(rows)   # control: the planted old ids


# ---------- formats ----------

def test_a_format_is_required_and_enter_exports_only_with_one(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    menu(app, "export")
    app.locator("#exMidi").uncheck()
    expect(app.locator("#exGo")).to_be_enabled()                             # spikes alone is a choice
    app.locator("#exSpikes").uncheck()
    expect(app.locator("#exGo")).to_be_disabled()
    expect(app.locator("#exProblem")).to_have_text("Choose at least one format.")
    expect(app.locator("#exReplace")).to_have_text("")
    app.keyboard.press("Enter")                                              # Enter = Export, but there is nothing to export
    app.wait_for_timeout(300)
    assert exports(folder) == [] and modal_visible(app)
    app.locator("#exSpikes").check()                                         # control: with a format the same key exports
    app.evaluate("() => document.activeElement.blur()")
    app.keyboard.press("Enter")
    wait_until(lambda: exports(folder) == ["Kick In_spike.wav", "Snare Top_spike.wav"], msg="the spike files")
    app.wait_for_function("() => !EX.busy")
    assert not any(n.endswith(".mid") for n in exports(folder))              # MIDI was off
    app.click("#exCancel")


# ---------- scope ----------

def test_selected_scope_exports_the_selected_track_only_and_skipped_tracks_are_listed(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    app.evaluate("() => setSelTrack('Snare Top')")
    menu(app, "export_selected")
    expect(app.locator("#exScopeSel")).to_be_checked()
    run_and_close(app)
    assert exports(folder) == ["Snare Top_spike.mid", "Snare Top_spike.wav"]
    expect(app.locator("#stMsg")).to_contain_text("exported 1 MIDI and 1 spike file to Exports/ in the project folder")
    # control: the All scope writes the other track too
    menu(app, "export")
    run_and_close(app)
    assert exports(folder) == ["Kick In_spike.mid", "Kick In_spike.wav", "Snare Top_spike.mid", "Snare Top_spike.wav"]
    # a track without hits is listed as skipped; Selected on it has nothing to export
    invoke(app, "events_set", sid=sid, name="Snare Top", events=[])
    app.evaluate("async () => { await loadProject(); }")
    menu(app, "export")
    expect(app.locator("#exSkipped")).to_have_text("Skipped: Snare Top (no hits)")
    expect(app.locator("#exGo")).to_be_enabled()
    app.click("#exCancel")
    app.evaluate("() => setSelTrack('Snare Top')")
    menu(app, "export_selected")
    expect(app.locator("#exProblem")).to_contain_text("No track to export")
    expect(app.locator("#exGo")).to_be_disabled()
    app.click("#exCancel")
    # a track whose audio is offline is skipped too
    src = next(Path(folder, "Audio Files").glob("Kick In*.wav"))
    src.rename(src.with_suffix(".away"))
    app.evaluate("async () => { await loadProject(); }")
    menu(app, "export")
    expect(app.locator("#exSkipped")).to_contain_text("Kick In (offline)")
    app.click("#exCancel")


# ---------- destination ----------

def test_choose_writes_into_the_chosen_folder_and_reset_goes_back_to_exports(app, make_project, tmp_path):
    sid, folder = project_with_hits(app, make_project)
    out = tmp_path / "my exports"
    out.mkdir()
    menu(app, "export")
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(out))
    app.click("#exChoose")
    expect(app.locator("#exDest")).to_have_value(str(out))
    assert app.evaluate("() => window.__dialogOpts.directory") is True       # the native folder picker
    run_and_close(app)
    assert exports(folder) == []                                              # the default folder is untouched
    assert sorted(p.name for p in out.iterdir()) == ["Kick In_spike.mid", "Kick In_spike.wav", "Snare Top_spike.mid", "Snare Top_spike.wav"]
    expect(app.locator("#stMsg")).to_contain_text(f"to {out}")
    # the choice is kept in the project: a new dialog starts with it, and the saved document has it
    menu(app, "export")
    expect(app.locator("#exDest")).to_have_value(str(out))
    expect(app.locator("#exReplace")).to_have_text("4 files will be replaced")
    app.keyboard.press("Escape")
    assert invoke(app, "project_save", sid=sid)["saved"] is not None
    doc = json.loads(next(folder.glob("*.drumspike")).read_text())
    assert doc["export_dest"] == str(out)
    # Reset: back to Exports/ and the field is gone from the document
    menu(app, "export")
    app.click("#exReset")
    expect(app.locator("#exDest")).to_have_value(str(folder / "Exports"))
    expect(app.locator("#exReset")).to_be_disabled()
    expect(app.locator("#exReplace")).to_have_text("No file will be replaced.")
    run_and_close(app)
    assert len(exports(folder)) == 4
    invoke(app, "project_save", sid=sid)
    assert "export_dest" not in json.loads(next(folder.glob("*.drumspike")).read_text())
    # a pick that cannot be used changes nothing and says why
    menu(app, "export")
    for bad, why in ((str(tmp_path / "nope"), "does not exist"), (str(next(out.iterdir())), "not a folder")):
        app.evaluate("p => { window.__DS_PICK__ = p; }", bad)
        app.click("#exChoose")
        expect(app.locator("#exProblem")).to_contain_text(why)
        expect(app.locator("#exDest")).to_have_value(str(folder / "Exports"))
    app.evaluate("() => { window.__DS_PICK__ = null; }")                       # a cancelled picker changes nothing
    app.click("#exChoose")
    expect(app.locator("#exDest")).to_have_value(str(folder / "Exports"))
    app.click("#exCancel")


def test_a_kept_folder_that_is_gone_falls_back_to_exports_with_a_note(app, make_project, tmp_path):
    sid, folder = project_with_hits(app, make_project)
    out = tmp_path / "gone soon"
    out.mkdir()
    invoke(app, "export_dest_set", sid=sid, dest=str(out))
    out.rmdir()
    menu(app, "export")
    expect(app.locator("#exDest")).to_have_value(str(folder / "Exports"))
    expect(app.locator("#exProblem")).to_contain_text("The folder kept for this project cannot be used")
    app.click("#exCancel")


# ---------- the replace count, Enter and Escape ----------

def test_the_dialog_says_how_many_files_it_replaces_and_enter_and_escape_work(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    menu(app, "export")
    expect(app.locator("#exReplace")).to_have_text("No file will be replaced.")
    app.keyboard.press("Escape")                                             # Escape = Cancel: nothing is written
    expect(app.locator("#exportModal")).to_be_hidden()
    expect(app.locator("#stMsg")).to_have_text("export cancelled")
    assert exports(folder) == []
    menu(app, "export")
    app.evaluate("() => document.activeElement.blur()")
    app.keyboard.press("Enter")                                              # Enter = Export
    ex_confirm_tempo(app)                                                    # the default-tempo notice answers (once)
    wait_until(lambda: len(exports(folder)) == 4, msg="Enter exported")
    app.wait_for_function("() => !EX.busy")
    expect(app.locator("#exResult")).to_be_visible()
    app.keyboard.press("Escape")
    # the count follows the options
    menu(app, "export")
    expect(app.locator("#exReplace")).to_have_text("4 files will be replaced")
    app.locator("#exSpikes").uncheck()
    expect(app.locator("#exReplace")).to_have_text("2 files will be replaced")
    app.locator("#exModeSingle").check()
    expect(app.locator("#exReplace")).to_have_text("No file will be replaced.")   # the multi-track file is not there yet
    run_and_close(app)
    menu(app, "export")
    app.locator("#exSpikes").uncheck()
    app.locator("#exModeSingle").check()
    expect(app.locator("#exReplace")).to_have_text("1 file will be replaced")
    app.click("#exCancel")
    # the service gives the same numbers (and no confirm dialog came up in between)
    assert invoke(app, "export_existing", sid=sid, midi_mode="single", formats=["midi"])["count"] == 1
    expect(app.locator("#confirmModal")).to_be_hidden()


# ---------- the multi-track file ----------

def test_one_multi_track_midi_file_has_a_track_per_drum_on_its_own_note_and_the_tempo_map(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    invoke(app, "tempo_map_set", sid=sid, map={"bpm": 100, "steps": [{"t": 2.0, "bpm": 150}]})
    menu(app, "export")
    app.locator("#exSpikes").uncheck()
    app.locator("#exModeSingle").check()
    run_and_close(app)
    name = app.evaluate("() => S.name") + "_drums.mid"
    assert exports(folder) == [name]                                          # one file, no per-track files, no spike files
    mid = mido.MidiFile(str(folder / "Exports" / name))
    assert mid.type == 1 and len(mid.tracks) == 3
    tempos = [m.tempo for m in mid.tracks[0] if m.type == "set_tempo"]
    assert tempos == [mido.bpm2tempo(100), mido.bpm2tempo(150)]
    notes = {}
    for t in mid.tracks[1:]:
        title = next(m.name for m in t if m.type == "track_name")
        notes[title] = sorted({m.note for m in t if m.type == "note_on" and m.velocity > 0})
        assert sum(1 for m in t if m.type == "note_on" and m.velocity > 0) == 3
    assert notes == {"Kick In": [36], "Snare Top": [38]}
    expect(app.locator("#stMsg")).to_contain_text("exported 1 multi-track MIDI file to Exports/ in the project folder")
    # control: the per-track mode writes two files and no multi-track file
    menu(app, "export")
    app.locator("#exSpikes").uncheck()
    app.locator("#exModeTrack").check()
    run_and_close(app)
    assert exports(folder) == sorted([name, "Kick In_spike.mid", "Snare Top_spike.mid"])


# ---------- P4-40g: markers in MIDI ----------

def test_markers_in_midi_exports_marker_events_and_the_choice_is_remembered(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    invoke(app, "markers_set", sid=sid, markers=[{"t": 1.0, "note": "verse"}, {"t": 3.5}])
    menu(app, "export")
    assert app.locator("#exMarkers").is_checked() is False          # off by default
    app.locator("#exMarkers").check()
    assert invoke(app, "project_get", sid=sid)["dirty"] is True     # the choice is a document field
    app.locator("#exSpikes").uncheck()
    app.locator("#exModeSingle").check()
    run_and_close(app)
    name = app.evaluate("() => S.name") + "_drums.mid"
    mid = mido.MidiFile(str(folder / "Exports" / name))
    ppq = mid.ticks_per_beat
    marks = [(sum(m.time for m in mid.tracks[0][:i + 1]), m.text)
             for i, m in enumerate(mid.tracks[0]) if m.type == "marker"]
    # the default export tempo is 120 (EXPORT_TEMPO): 2 quarters per second, so 1 s is 2*ppq ticks
    assert marks == [(round(1.0 * 2 * ppq), "verse"), (round(3.5 * 2 * ppq), "Marker 2")]
    menu(app, "export")
    assert app.locator("#exMarkers").is_checked() is True           # remembered per project
    app.locator("#exMarkers").uncheck()
    run_and_close(app)
    mid = mido.MidiFile(str(folder / "Exports" / name))
    assert not [m for m in mid.tracks[0] if m.type == "marker"]     # the control: off, no markers


# ---------- the keys ----------

def test_page_keys_are_off_while_the_dialog_is_open(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    app.evaluate("() => { S.playhead = 0; }")
    menu(app, "export")
    app.locator("#exGo").focus()
    app.keyboard.press("Space")                                               # Space plays and stops in the page
    ex_confirm_tempo(app)   # #151: Space on the focused button activates it: the export asks on the default map
    app.wait_for_function("() => !EX.busy")   # the export ran; the dialog answers Escape again
    app.keyboard.press("j")
    app.keyboard.press("m")
    app.wait_for_timeout(300)
    assert app.evaluate("() => !!S.playing") is False
    app.evaluate("() => document.activeElement.blur()")
    app.keyboard.press("Space")
    app.wait_for_timeout(300)
    assert app.evaluate("() => !!S.playing") is False
    app.keyboard.press("Escape")
    # control: with the dialog closed the same key starts the transport
    app.evaluate("() => { document.activeElement.blur(); }")
    app.keyboard.press("Space")
    app.wait_for_function("() => S.playing === true")
    app.keyboard.press("Space")
    app.wait_for_function("() => !S.playing")
    invoke(app, "transport_stop", close=True)                                 # the playback tests start without an open transport


# ---------- the drag tiles ----------

def test_the_drag_tiles_prepare_the_files_and_a_drag_calls_the_native_drag_with_their_paths(app, make_project, tmp_path):
    sid, folder = project_with_hits(app, make_project)
    # a slow render shows "preparing..." on the tile, and a drag then does nothing (the files are not there yet)
    app.evaluate("""() => { const o = B.exportDragPrepare.bind(B); window.__slow = true;
        B.exportDragPrepare = async (...a) => { if (window.__slow && !(a[1] && a[1].clear)) await new Promise(r => setTimeout(r, 1200)); return o(...a); }; }""")
    menu(app, "export")
    midi, spikes = app.locator("#exDragMidi"), app.locator("#exDragSpikes")
    expect(midi).to_have_attribute("data-state", "preparing")
    expect(midi.locator(".exDragN")).to_have_text("preparing…")
    box = midi.bounding_box()
    app.mouse.move(box["x"] + 5, box["y"] + 5)
    app.mouse.down()
    app.mouse.move(box["x"] + 60, box["y"] + 20, steps=4)
    app.mouse.up()
    assert app.evaluate("() => window.__DS_DRAG__ || []") == []              # not ready: no drag
    expect(midi).to_have_attribute("data-state", "ready", timeout=10000)
    expect(midi.locator(".exDragN")).to_have_text("2 files")
    expect(spikes).to_have_attribute("data-state", "ready")
    # #433: the tiles render into the hidden Exports/.drag folder; Exports/ gets a file only by a drag
    drag_dir = Path(invoke(app, "export_drag_prepare", sid=sid, formats=["midi"])["dir"])
    assert drag_dir == folder / "Exports" / ".drag"
    assert sorted(p.name for p in (folder / "Exports").iterdir()) == [".drag"]
    app.evaluate("() => { window.__slow = false; }")
    app.wait_for_function("() => document.querySelector('#exDragMidi').dataset.state === 'ready'")
    box = midi.bounding_box()
    app.mouse.move(box["x"] + 5, box["y"] + 5)
    app.mouse.down()
    app.mouse.move(box["x"] + 60, box["y"] + 20, steps=4)
    app.mouse.up()
    drags = app.evaluate("() => window.__DS_DRAG__")
    assert len(drags) == 1 and sorted(Path(p).name for p in drags[0]) == ["Kick In_spike.mid", "Snare Top_spike.mid"]
    assert all(Path(p).is_file() and Path(p).parent == folder / "Exports" for p in drags[0])   # the dragged files are the exports (#433)
    assert sorted(p.name for p in (folder / "Exports").iterdir() if p.name != ".drag") == ["Kick In_spike.mid", "Snare Top_spike.mid"]
    # a press without a move is no drag (control)
    app.mouse.move(box["x"] + 5, box["y"] + 5)
    app.mouse.down()
    app.mouse.up()
    assert len(app.evaluate("() => window.__DS_DRAG__")) == 1
    # an option change renders again: the tile shows the new count; the single file is one file
    app.locator("#exModeSingle").check()
    expect(midi.locator(".exDragN")).to_have_text("1 file")
    app.wait_for_function("() => document.querySelector('#exDragMidi').dataset.state === 'ready'")
    box = midi.bounding_box()
    app.mouse.move(box["x"] + 5, box["y"] + 5)
    app.mouse.down()
    app.mouse.move(box["x"] + 60, box["y"] + 20, steps=4)
    app.mouse.up()
    drags = app.evaluate("() => window.__DS_DRAG__")
    assert len(drags) == 2 and [Path(p).name for p in drags[1]] == [app.evaluate("() => S.name") + "_drums.mid"]
    # the tile of an unchecked format is hidden, and a format without tracks is disabled
    app.locator("#exSpikes").uncheck()
    expect(spikes).to_be_hidden()
    app.locator("#exSpikes").check()
    app.click("#exCancel")
    app.evaluate("async () => { await EX.dragChain; }")                      # the clean-up runs on the drag chain (x86 CI was faster to look)
    assert all(Path(p).is_file() for p in drags[0])                           # the dialog leaves the dragged exports in place
    assert not drag_dir.exists()                                              # and the staged files of the tiles are gone


def test_the_tiles_are_disabled_without_tracks_and_a_cancel_or_close_removes_the_staged_files(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    invoke(app, "events_set", sid=sid, name="Snare Top", events=[])
    app.evaluate("async () => { await loadProject(); }")
    app.evaluate("() => setSelTrack('Snare Top')")
    menu(app, "export_selected")
    expect(app.locator("#exDragMidi")).to_have_attribute("data-state", "disabled")
    expect(app.locator("#exDragSpikes")).to_have_attribute("data-state", "disabled")
    app.click("#exScopeAll")
    expect(app.locator("#exDragMidi")).to_have_attribute("data-state", "ready", timeout=10000)
    drag_dir = Path(invoke(app, "export_drag_prepare", sid=sid)["dir"])
    assert drag_dir.is_dir() and any(drag_dir.iterdir())
    app.click("#exCancel")
    app.evaluate("async () => { await EX.dragChain; }")
    assert not drag_dir.exists()                                              # a cancel removes the staged files
    assert not (folder / "Exports").exists() or not any((folder / "Exports").iterdir())   # and writes nothing into Exports/
    invoke(app, "project_save", sid=sid)
    invoke(app, "export_drag_prepare", sid=sid)
    assert drag_dir.is_dir()
    invoke(app, "project_close", sid=sid, force=True)
    assert not drag_dir.exists()                                              # closing the project removes them too (#433)


# ---------- Show in Finder ----------

def test_show_in_finder_names_the_first_file_after_an_export(app, make_project, tmp_path):
    sid, folder = project_with_hits(app, make_project)
    menu(app, "export")
    expect(app.locator("#exResult")).to_be_hidden()
    app.click("#exGo")
    ex_confirm_tempo(app)
    expect(app.locator("#exResult")).to_be_visible()
    expect(app.locator("#exResultText")).to_contain_text("Exported 2 MIDI and 2 spike files to Exports/ in the project folder")
    expect(app.locator("#exCancel")).to_have_text("Close")
    app.click("#exReveal")
    wait_until(lambda: app.evaluate("() => window.__DS_REVEAL__ || []"), msg="reveal")
    shown = app.evaluate("() => window.__DS_REVEAL__")
    assert len(shown) == 1 and Path(shown[0]).is_file() and Path(shown[0]).parent == folder / "Exports"
    app.click("#exCancel")
    menu(app, "export")
    expect(app.locator("#exResult")).to_be_hidden()                           # a new dialog starts without the old result
    expect(app.locator("#exCancel")).to_have_text("Cancel")
    app.click("#exCancel")


# ---------- #218: the spike type (Synthetic / Audio / Both) ----------

def sha(path):
    return hashlib.sha1(Path(path).read_bytes()).hexdigest()


def source_of(folder, name):
    return next(Path(folder, "Audio Files").glob(f"{name}*.wav"))


def clear_exports(folder):
    for f in (folder / "Exports").iterdir():
        if f.is_file():   # the hidden .drag folder of the staged drag files stays (#433)
            f.unlink()


def test_the_dialog_offers_the_three_spike_types_and_audio_writes_only_audio_files(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    menu(app, "export")
    for rid in ("#exSpikeSynth", "#exSpikeAudio", "#exSpikeBoth"):
        expect(app.locator(rid)).to_be_visible()                             # the three choices are in the dialog
    expect(app.locator("#exSpikeSynth")).to_be_checked()                     # Synthetic is the default (today's behaviour)
    app.locator("#exSpikeAudio").check()
    run_and_close(app)
    assert exports(folder) == sorted([
        "Kick In_audio_spike.wav", "Kick In_spike.mid",
        "Snare Top_audio_spike.wav", "Snare Top_spike.mid"])                 # the audio spikes sit next to the other exports
    assert not any(n.endswith("_spike.wav") and not n.endswith("_audio_spike.wav") for n in exports(folder))  # and no synthetic spike file
    expect(app.locator("#stMsg")).to_contain_text("exported 2 MIDI and 2 audio spike files")
    # the choice is kept: a new dialog shows Audio again, also after a reload of the project
    menu(app, "export")
    expect(app.locator("#exSpikeAudio")).to_be_checked()
    app.keyboard.press("Escape")
    app.evaluate("async () => { await loadProject(); }")
    menu(app, "export")
    expect(app.locator("#exSpikeAudio")).to_be_checked()
    # Both writes both WAV kinds
    app.locator("#exSpikeBoth").check()
    run_and_close(app)
    assert exports(folder) == sorted([
        "Kick In_audio_spike.wav", "Kick In_spike.mid", "Kick In_spike.wav",
        "Snare Top_audio_spike.wav", "Snare Top_spike.mid", "Snare Top_spike.wav"])
    # control (the check can fail): Synthetic writes no audio-spike file
    clear_exports(folder)
    menu(app, "export")
    app.locator("#exSpikeSynth").check()
    run_and_close(app)
    assert exports(folder) == ["Kick In_spike.mid", "Kick In_spike.wav", "Snare Top_spike.mid", "Snare Top_spike.wav"]


def test_an_audio_export_from_the_dialog_leaves_the_source_audio_unchanged(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    names = ("Kick In", "Snare Top")
    before = {n: sha(source_of(folder, n)) for n in names}
    menu(app, "export")
    app.locator("#exSpikeAudio").check()
    run_and_close(app)
    assert sorted(n for n in exports(folder) if n.endswith("_audio_spike.wav")) == [
        "Kick In_audio_spike.wav", "Snare Top_audio_spike.wav"]              # the dialog really wrote the audio spikes
    assert {n: sha(source_of(folder, n)) for n in names} == before           # the sources are byte-for-byte the same
    # control: the audio-spike file holds snippets, so it is not an empty stand-in for the source
    assert all((folder / "Exports" / f"{n}_audio_spike.wav").stat().st_size > 44 for n in names)


# ---------- the service: dest on the single commands ----------

def test_the_commands_take_a_dest_and_refuse_a_bad_one(app, make_project, tmp_path):
    sid, folder = project_with_hits(app, make_project)
    out = tmp_path / "svc"
    out.mkdir()
    r = invoke(app, "export_midi", sid=sid, name="Kick In", dest=str(out))
    assert r["path"] == str(out / "Kick In_spike.mid") and (out / "Kick In_spike.mid").is_file()
    r = invoke(app, "export_spikes", sid=sid, name="Kick In", dest=str(out))
    assert r["path"] == str(out / "Kick In_spike.wav")
    for bad, code in (("relative/dir", 400), (str(tmp_path / "missing"), 404), (str(out / "Kick In_spike.mid"), 400)):
        assert invoke(app, "export_midi", sid=sid, name="Kick In", dest=bad)["code"] == code
        assert invoke(app, "export_all", sid=sid, dest=bad)["code"] == code
        assert invoke(app, "export_existing", sid=sid, dest=bad)["code"] == code
    assert not (folder / "Exports").exists() or exports(folder) == []
    r = invoke(app, "export_all", sid=sid, dest=str(out), midi_mode="single")
    assert r["midi_file"] == str(out / (app.evaluate("() => S.name") + "_drums.mid")) and r["dest"] == str(out)
    assert invoke(app, "export_all", sid=sid, midi_mode="both")["code"] == 400
    assert re.search(r"midi_mode", invoke(app, "export_all", sid=sid, midi_mode="both")["message"])


# ---------- #151: the default-tempo notice ----------

def test_the_default_tempo_notice_asks_and_cancel_writes_no_file(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    menu(app, "export")
    app.click("#exGo")
    expect(app.locator("#confirmModal")).to_be_visible()
    expect(app.locator("#confirmTitle")).to_have_text("No tempo set")
    expect(app.locator("#confirmMsg")).to_contain_text("120 BPM")
    app.locator('#confirmActions [data-act="cancel"]').click()
    expect(app.locator("#confirmModal")).to_be_hidden()
    expect(app.locator("#stMsg")).to_contain_text("export cancelled")
    assert exports(folder) == []                                             # Cancel writes no file
    # the dialog stays open and exports after the answer; the session answers only once
    app.click("#exGo")
    ex_confirm_tempo(app)
    wait_until(lambda: exports(folder) == ["Kick In_spike.mid", "Kick In_spike.wav",
                                           "Snare Top_spike.mid", "Snare Top_spike.wav"], msg="the export")
    app.click("#exCancel")
    clear_exports(folder)
    menu(app, "export")
    run_and_close(app)                                                       # no second notice in this session
    expect(app.locator("#confirmModal")).to_be_hidden()
    assert len(exports(folder)) == 4


def test_export_anyway_writes_the_bytes_of_today_and_the_reply_carries_the_warning(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    (folder / "Ref").mkdir()                                                 # the dest folder must exist
    invoke(app, "export_midi", sid=sid, name="Kick In", dest=str(folder / "Ref"))   # the service path of today
    ref = (folder / "Ref" / "Kick In_spike.mid").read_bytes()
    menu(app, "export")
    app.locator("#exSpikes").uncheck()
    app.locator("#exModeTrack").check()
    app.click("#exGo")
    ex_confirm_tempo(app)
    wait_until(lambda: exports(folder) == ["Kick In_spike.mid", "Snare Top_spike.mid"], msg="the export")
    assert (folder / "Exports" / "Kick In_spike.mid").read_bytes() == ref    # Export anyway: the same bytes as today
    r = invoke(app, "export_all", sid=sid)
    assert "No tempo set: the MIDI export uses the default 120 BPM." in r["warnings"]   # the reply warns on the default map
    app.click("#exCancel")


def test_a_project_with_a_set_map_exports_without_the_notice(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    invoke(app, "tempo_map_set", sid=sid, map={"bpm": 100})
    menu(app, "export")
    app.click("#exGo")
    app.wait_for_timeout(300)
    expect(app.locator("#confirmModal")).to_be_hidden()                      # no notice on a set map
    wait_until(lambda: exports(folder) == ["Kick In_spike.mid", "Kick In_spike.wav",
                                           "Snare Top_spike.mid", "Snare Top_spike.wav"], msg="the export")
    (folder / "Ref").mkdir()
    r = invoke(app, "export_all", sid=sid, dest=str(folder / "Ref"))
    assert r["warnings"] == []                                               # and the reply carries no warning
    app.click("#exCancel")
