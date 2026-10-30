"""P4-31: the Export dialog keeps its formats per project, and File > Export Again (Option+Cmd+E) writes the
files of the last export again: the same formats, the same tracks and the same folder, with no dialog. The
item and the key are greyed (the page says so) before the first export of the project.

The choices are recorded at once, like the destination folder and the Markers-in-MIDI checkbox: a change that
is cancelled is still what Export again repeats, and the dialog of a reloaded project shows it.

Each test has a planted-fault half: a control that shows the check can fail (the same action without the
condition under test gives the other result), as the other e2e files do with their `the_check_can_fail` tests."""
import json

from playwright.sync_api import expect

from app_support import ex_run, invoke, menu, open_project, wait_until
from test_export_dialog import clear_exports, exports, modal_visible, project_with_hits, run_and_close


def msg(app):
    return app.evaluate("() => document.querySelector('#stMsg').textContent")


def menu_state(app):
    return app.evaluate("() => window.__DS_MENU_STATE__")


def wait_export(app, text="exported"):
    wait_until(lambda: msg(app).startswith(text), msg="the export message")


def press_again(app):
    """#226: the Export Again key through the native menu arrives late under load, and the stale
    "exported ..." message of an earlier export already satisfies wait_export (the check then saw an
    empty folder). Wait for the export-finished event first (#172: EX.done), the same signal ex_run
    uses; only then is the message and the folder the result of THIS key press.
    #151: on the default map Export Again asks the default-tempo notice before it writes; the helper
    answers Export anyway (a set map never asks, and the gate test drives the notice itself)."""
    done0 = app.evaluate("() => EX.done")
    app.keyboard.press("Alt+Meta+E")
    app.wait_for_function(
        """d0 => (!document.querySelector('#confirmModal').classList.contains('hidden')
             && document.querySelector('#confirmActions [data-act="anyway"]')) || EX.done > d0""",
        arg=done0, timeout=60000)
    if not app.evaluate("() => document.querySelector('#confirmModal').classList.contains('hidden')"):
        app.locator('#confirmActions [data-act="anyway"]').click()
    app.wait_for_function("d0 => EX.done > d0", arg=done0, timeout=60000)


def answer_tempo_notice(app):
    """#151: Export Again on the default map asks the default-tempo notice before it writes; the
    tests that reload the project between the export and Export Again answer Export anyway here
    (a set map never asks, and the gate test in test_export_dialog.py drives the notice itself)."""
    app.wait_for_function(
        """() => (!document.querySelector('#confirmModal').classList.contains('hidden')
              && document.querySelector('#confirmActions [data-act="anyway"]'))
             || document.querySelector('#stMsg').textContent.startsWith('exported')""", timeout=60000)
    if not app.evaluate("() => document.querySelector('#confirmModal').classList.contains('hidden')"):
        app.locator('#confirmActions [data-act="anyway"]').click()


def test_export_again_is_greyed_before_the_first_export_and_says_so(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    assert menu_state(app)["exportAgain"] is False                     # greyed: the project keeps no export shape
    assert menu(app, "export_again") is True
    expect(app.locator("#stMsg")).to_contain_text("nothing exported yet")
    expect(app.locator("#exportModal")).to_be_hidden()                 # no dialog
    assert exports(folder) == []                                       # and nothing was written
    # the key refuses the same way
    app.keyboard.press("Alt+Meta+E")
    app.wait_for_timeout(300)
    assert exports(folder) == [] and not modal_visible(app)
    # control: once an export ran, the same key writes
    menu(app, "export")
    run_and_close(app)
    assert len(exports(folder)) == 4
    press_again(app)
    wait_export(app)


def test_the_dialog_keeps_its_choices_per_project_and_export_again_writes_the_same_files(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    menu(app, "export")
    expect(app.locator("#exMidi")).to_be_checked()                     # control: the defaults before any export
    expect(app.locator("#exSpikes")).to_be_checked()
    expect(app.locator("#exModeTrack")).to_be_checked()
    app.keyboard.press("Escape")
    menu(app, "export")
    app.locator("#exSpikes").uncheck()                                 # MIDI on, spikes off
    app.locator("#exModeSingle").check()
    run_and_close(app)
    name = app.evaluate("() => S.name") + "_drums.mid"
    assert exports(folder) == [name]
    wait_until(lambda: menu_state(app)["exportAgain"] is True, msg="the Export Again item is on")
    invoke(app, "project_save", sid=sid)
    doc = json.loads(next(folder.glob("*.drumspike")).read_text())
    assert doc["export_last"]["formats"] == ["midi"] and doc["export_last"]["midi_mode"] == "single"
    # reload the project: the dialog shows the same choices
    app.evaluate("async () => { await loadProject(); }")
    menu(app, "export")
    expect(app.locator("#exMidi")).to_be_checked()
    expect(app.locator("#exSpikes")).not_to_be_checked()
    expect(app.locator("#exModeSingle")).to_be_checked()
    app.keyboard.press("Escape")
    # the key writes the same file to the same place with no dialog
    press_again(app)
    wait_export(app)
    assert exports(folder) == [name]
    expect(app.locator("#exportModal")).to_be_hidden()
    expect(app.locator("#stMsg")).to_contain_text("exported 1 multi-track MIDI file")
    # a change that is cancelled is still what the project keeps (recorded at once, like the folder)
    menu(app, "export")
    app.locator("#exSpikes").check()
    app.keyboard.press("Escape")
    press_again(app)
    wait_export(app)
    assert exports(folder) == sorted([name, "Kick In_spike.wav", "Snare Top_spike.wav"])


def test_export_again_writes_to_the_same_place(app, make_project, tmp_path):
    sid, folder = project_with_hits(app, make_project)
    out = tmp_path / "again"
    out.mkdir()
    menu(app, "export")
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(out))
    app.click("#exChoose")
    # the folder is kept after two round trips (the pick, then the record): wait for it before Export, or the
    # export runs into Exports/ under load (the same race as #226)
    wait_until(lambda: app.evaluate("() => EX.dest") == str(out), msg="the chosen export folder")
    run_and_close(app)
    assert sorted(p.name for p in out.iterdir()) == [
        "Kick In_spike.mid", "Kick In_spike.wav", "Snare Top_spike.mid", "Snare Top_spike.wav"]
    assert exports(folder) == []
    app.evaluate("async () => { await loadProject(); }")
    menu(app, "export")                                                # the dest is kept anyway: control
    expect(app.locator("#exDest")).to_have_value(str(out))
    app.keyboard.press("Escape")
    menu(app, "export_again")                                          # the item: the same place, no dialog
    answer_tempo_notice(app)                                           # #151: the reloaded project asks on the default map
    wait_export(app)
    assert exports(folder) == [] and len(list(out.iterdir())) == 4
    expect(app.locator("#exportModal")).to_be_hidden()
    expect(app.locator("#stMsg")).to_contain_text(f"to {out}")


def test_export_again_repeats_the_spike_type(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    menu(app, "export")
    app.locator("#exSpikeAudio").check()                                     # #218: the type is part of the export shape
    run_and_close(app)
    assert sorted(n for n in exports(folder) if n.endswith("_audio_spike.wav")) == [
        "Kick In_audio_spike.wav", "Snare Top_audio_spike.wav"]
    assert not any(n.endswith("_spike.wav") and not n.endswith("_audio_spike.wav") for n in exports(folder))
    invoke(app, "project_save", sid=sid)
    doc = json.loads(next(folder.glob("*.drumspike")).read_text())
    assert doc["export_last"]["formats"] == ["midi", "audio"]                # the record keeps the type
    # the key writes the same files again: the audio spikes, no dialog
    clear_exports(folder)
    press_again(app)
    wait_export(app)
    expect(app.locator("#exportModal")).to_be_hidden()
    assert sorted(n for n in exports(folder) if n.endswith("_audio_spike.wav")) == [
        "Kick In_audio_spike.wav", "Snare Top_audio_spike.wav"]
    assert not any(n.endswith("_spike.wav") and not n.endswith("_audio_spike.wav") for n in exports(folder))
    # control: a recorded Synthetic repeats as synthetic (the record moves at once, like the other choices)
    menu(app, "export")
    app.locator("#exSpikeSynth").check()
    app.keyboard.press("Escape")
    press_again(app)
    wait_export(app)
    assert sorted(n for n in exports(folder) if n.endswith("_spike.wav") and not n.endswith("_audio_spike.wav")) == [
        "Kick In_spike.wav", "Snare Top_spike.wav"]
    invoke(app, "project_save", sid=sid)
    doc = json.loads(next(folder.glob("*.drumspike")).read_text())
    assert doc["export_last"]["formats"] == ["midi", "spikes"]


def test_export_again_repeats_the_tracks_of_the_last_export(app, make_project):
    sid, folder = project_with_hits(app, make_project)
    app.evaluate("() => setSelTrack('Snare Top')")
    menu(app, "export_selected")
    run_and_close(app)
    assert exports(folder) == ["Snare Top_spike.mid", "Snare Top_spike.wav"]
    app.evaluate("() => setSelTrack('Kick In')")                       # the selection moved on
    menu(app, "export_again")
    wait_export(app)
    assert exports(folder) == ["Snare Top_spike.mid", "Snare Top_spike.wav"]   # still Snare Top only
    # control: an export with the other track selected writes that track
    menu(app, "export_selected")
    run_and_close(app)
    assert "Kick In_spike.mid" in exports(folder)
