"""P3-14b: counters and info line (docs/migration/ux-daw-reference.md 3.4, 3.5 and 5; Cubase and Pro Tools models).

Counters: the main counter shows seconds with 6 decimals, the sub counter shows samples at the rate of the selected track
(else of the first track). View > Main Counter (Seconds, Samples), Exchange Counters (key . until P3-19) and a click on the main
counter swap them. Info line: the selected hit (track, index of total, time, sample, velocity, origin, offset); the sample
and the velocity are fields (Return applies, Escape cancels, one undo step each); View > Show Info Line (Cmd+I) shows and
hides it, and the state is kept in the app settings. The line has a fixed height and the frames read no layout."""
import re
import struct
from pathlib import Path

import pytest
from playwright.sync_api import expect

from app_support import edits_done, export_dialog, menu, open_project, wait_until
from helpers import SR, synth_track, write_wav
from test_editor import IDX, ONSETS, build, doc_of, exact_events, service_events
from test_frames import PROBE, READS, SWEEP, settle

SR44 = 44100


def txt(page, sel):
    return page.locator(sel).text_content()


def state(page):
    return page.evaluate("() => DSShell.menuState()")


def select_header(page, name):
    page.locator(f'.track[data-name="{name}"] .thead .tname').click()


def two_rates(make_project, tmp_path):
    """Kick In at 48 kHz, Snare Top at 44.1 kHz, no hits."""
    src = tmp_path / "src_rates"
    src.mkdir()
    specs = []
    for name, role, note, sr in (("Kick In", "kick", 36, SR), ("Snare Top", "snare", 38, SR44)):
        x, _ = synth_track(ONSETS, 4.0, sr=sr)
        specs.append((name, write_wav(src / f"{name}.wav", x, sr=sr), role, note))
    return Path(make_project.builder.build_project(tmp_path / "projects_rates", "Rates", specs, "copied"))


def midi_note_ons(path):
    """[(tick, velocity)] of the note-on events of a standard MIDI file (every track chunk)."""
    data = Path(path).read_bytes()
    assert data[:4] == b"MThd"
    pos = 8 + struct.unpack(">I", data[4:8])[0]
    out = []
    while pos < len(data):
        assert data[pos:pos + 4] == b"MTrk"
        end = pos + 8 + struct.unpack(">I", data[pos + 4:pos + 8])[0]
        p, tick, status = pos + 8, 0, 0

        def varlen():
            nonlocal p
            v = 0
            while True:
                b = data[p]; p += 1
                v = (v << 7) | (b & 0x7F)
                if not b & 0x80:
                    return v
        while p < end:
            tick += varlen()
            if data[p] == 0xFF:                                    # meta event: 0xFF, type, length, data
                p += 2; ln = varlen(); p += ln
                continue
            if data[p] in (0xF0, 0xF7):
                p += 1; ln = varlen(); p += ln; continue
            if data[p] & 0x80:
                status = data[p]; p += 1
            kind = status & 0xF0
            n = 1 if kind in (0xC0, 0xD0) else 2
            args = data[p:p + n]; p += n
            if kind == 0x90 and args[1] > 0:
                out.append((tick, args[1]))
        pos = end
    return out


# ---------- counters ----------

def test_counters_show_seconds_with_six_decimals_and_samples_at_the_rate_of_the_selected_track(app, make_project, tmp_path):
    open_project(app, two_rates(make_project, tmp_path))
    app.evaluate("() => { S.playhead = 3.141592; invalidate(F.HEAD); }")
    expect(app.locator("#timeDisplay")).to_have_text("3.141592 s")
    expect(app.locator("#sampleDisplay")).to_have_text(f"{round(3.141592 * SR)} smp")                 # P4-39c: the first track is selected at open
    select_header(app, "Snare Top")
    expect(app.locator("#sampleDisplay")).to_have_text(f"{round(3.141592 * SR44)} smp")               # the rate of the selected track
    select_header(app, "Snare Top")                                                                   # P4-39c: a second click keeps the track selected
    expect(app.locator("#sampleDisplay")).to_have_text(f"{round(3.141592 * SR44)} smp")
    app.evaluate("() => { S.playhead = 0; invalidate(F.HEAD); }")
    expect(app.locator("#timeDisplay")).to_have_text("0.000000 s")
    assert state(app)["counter"] == "seconds"


def test_the_counters_swap_by_menu_key_and_click_and_the_radio_items_follow(app, make_project, tmp_path):
    open_project(app, make_project())
    app.evaluate("() => { S.playhead = 2.5; invalidate(F.HEAD); }")
    main, sub = app.locator("#timeDisplay"), app.locator("#sampleDisplay")
    expect(main).to_have_text("2.500000 s")
    expect(sub).to_have_text("120000 smp")
    menu(app, "counter_exchange")                                  # the menu item
    expect(main).to_have_text("120000 smp")
    expect(sub).to_have_text("2.500000 s")
    assert state(app)["counter"] == "samples"
    app.keyboard.press(".")                                        # P3-19: the key . nudges a hit now, it no longer exchanges
    assert state(app)["counter"] == "samples"
    main.click()                                                   # the click on the main counter
    expect(main).to_have_text("2.500000 s")
    main.click()
    expect(main).to_have_text("120000 smp")
    menu(app, "counter_seconds")                                   # the radio items
    expect(main).to_have_text("2.500000 s")
    menu(app, "counter_samples")
    expect(main).to_have_text("120000 smp")
    menu(app, "counter_samples")                                   # a radio item that is on stays on
    assert state(app)["counter"] == "samples"
    # the choice is an app setting: it stays after a reload
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0")
    assert state(app)["counter"] == "samples"
    expect(app.locator("#timeDisplay")).to_have_text(re.compile(r" smp$"))


def test_the_key_dot_in_a_text_field_does_not_swap_the_counters(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    app.keyboard.press("Tab")
    app.locator("#ilVel").click()
    app.keyboard.press(".")
    assert state(app)["counter"] == "seconds"
    app.keyboard.press("Escape")


# ---------- info line ----------

def fields(page):
    return page.evaluate("""() => ({
      empty: !document.querySelector('#ilEmpty').classList.contains('hidden'),
      track: document.querySelector('#ilTrack').textContent, index: document.querySelector('#ilIndex').textContent,
      time: document.querySelector('#ilTime').textContent, sample: document.querySelector('#ilSample').value,
      vel: document.querySelector('#ilVel').value, origin: document.querySelector('#ilOrigin').textContent,
      offset: document.querySelector('#ilOffset').textContent })""")


def test_the_info_line_shows_a_detected_a_moved_and_an_added_hit_and_stays_fixed(app, make_project, tmp_path):
    ev = [{"t": i / SR, "v": 90 + n, "id": f"k{n}"} for n, i in enumerate(IDX)]
    open_project(app, build(make_project, tmp_path, events={"Kick In": ev}))
    expect(app.locator("#infoLine")).to_be_visible()
    expect(app.locator("#ilEmpty")).to_contain_text("No hit selected")
    expect(app.locator("#ilFields")).to_be_hidden()
    expect(app.locator("#stSel")).to_have_text("no selection")
    top = app.evaluate("() => [document.querySelector('#lanesScroll').getBoundingClientRect().top, document.querySelector('#infoLine').getBoundingClientRect().height]")
    app.keyboard.press("Tab")                                       # a detected hit
    expect(app.locator("#ilFields")).to_be_visible()
    expect(app.locator("#stSel")).to_have_text("Kick In: hit 1/3")
    f = fields(app)
    assert f == {"empty": False, "track": "Kick In", "index": "1/3", "time": "0.500000 s", "sample": str(IDX[0]),
                 "vel": "90", "origin": "detected", "offset": "0 smp"}
    assert app.evaluate("() => [document.querySelector('#lanesScroll').getBoundingClientRect().top, document.querySelector('#infoLine').getBoundingClientRect().height]") == top
    app.keyboard.press("Tab")
    app.keyboard.press("Shift+ArrowRight")                          # moved 10 samples later
    expect(app.locator("#ilOrigin")).to_have_text("moved")
    f = fields(app)
    assert (f["index"], f["sample"], f["vel"], f["offset"]) == ("2/3", str(IDX[1] + 10), "91", "+10 smp")
    assert f["time"] == f"{(IDX[1] + 10) / SR:.6f} s"
    app.keyboard.press("ArrowLeft"); app.keyboard.press("ArrowLeft")
    expect(app.locator("#ilOffset")).to_have_text("+8 smp")
    for _ in range(8):
        app.keyboard.press("ArrowLeft")
    expect(app.locator("#ilOrigin")).to_have_text("detected")       # back on the detected sample
    expect(app.locator("#ilOffset")).to_have_text("0 smp")
    select_header(app, "Kick In")                                   # an added hit at the playhead
    app.evaluate("() => { S.playhead = 3.0; invalidate(F.HEAD); }")
    menu(app, "add_hit")
    expect(app.locator("#ilOrigin")).to_have_text("added")
    f = fields(app)
    assert (f["index"], f["sample"], f["vel"], f["offset"], f["time"]) == ("4/4", str(3 * SR), "100", "-", "3.000000 s")
    expect(app.locator("#stSel")).to_have_text("Kick In: hit 4/4")
    app.keyboard.press("Escape")                                    # deselect: the empty text again, same height
    expect(app.locator("#ilEmpty")).to_be_visible()
    assert app.evaluate("() => [document.querySelector('#lanesScroll').getBoundingClientRect().top, document.querySelector('#infoLine').getBoundingClientRect().height]") == top


def test_a_sample_and_a_velocity_typed_into_the_info_line_reach_the_project_and_the_midi_file_and_undo_restores(app, make_project, tmp_path):
    folder = build(make_project, tmp_path, events={"Kick In": exact_events()})
    sid = open_project(app, folder)
    app.keyboard.press("Tab"); app.keyboard.press("Tab")           # the second hit
    undo0 = app.evaluate("() => S.undo.length")
    smp = app.locator("#ilSample")
    smp.click(); smp.fill("40000"); smp.press("Enter")
    edits_done(app)
    assert app.evaluate("() => S.undo.length") == undo0 + 1        # one undo step
    expect(smp).to_have_value("40000")
    expect(app.locator("#ilOrigin")).to_have_text("moved")
    expect(app.locator("#ilOffset")).to_have_text(f"{40000 - IDX[1]:+d} smp")
    assert app.evaluate("() => document.activeElement === document.body")   # the focus is back on the page
    vel = app.locator("#ilVel")
    vel.click(); vel.fill("64"); vel.press("Enter")
    edits_done(app)
    assert app.evaluate("() => S.undo.length") == undo0 + 2
    expect(vel).to_have_value("64")
    app.keyboard.press("Tab")                                       # the field kept no focus: Tab selects the next hit
    expect(app.locator("#ilIndex")).to_have_text("3/3")
    app.keyboard.press("Shift+Tab")
    expect(app.locator("#ilIndex")).to_have_text("2/3")
    # the project document (Save) and the MIDI export see the edit
    wait_until(lambda: [round(e["t"] * SR) for e in service_events(app, sid)][1] == 40000, msg="the edit reaches the service")
    assert service_events(app, sid)[1]["v"] == 64
    app.keyboard.press("Control+s")
    wait_until(lambda: round(doc_of(folder)["tracks"]["Kick In"]["events"][1]["t"] * SR) == 40000, msg="the save")
    assert doc_of(folder)["tracks"]["Kick In"]["events"][1]["v"] == 64
    select_header(app, "Kick In")
    export_dialog(app, formats=("midi",), scope="selected", track="Kick In", via="menu")
    mid = folder / "Exports" / "Kick In_spike.mid"
    wait_until(mid.is_file, msg="the MIDI file")
    ons = midi_note_ons(mid)
    assert [v for _, v in ons] == [100, 64, 100]
    assert ons[1][0] - ons[0][0] == 40000 - IDX[0]                  # the tick grid is the sample grid (24000 ppq, 120 bpm, 48 kHz)
    # undo: velocity first, then the sample; each is one step
    app.keyboard.press("Control+z")
    expect(vel).to_have_value("100")
    expect(smp).to_have_value("40000")
    app.keyboard.press("Control+z")
    expect(smp).to_have_value(str(IDX[1]))
    expect(app.locator("#ilOrigin")).to_have_text("detected")
    wait_until(lambda: [round(e["t"] * SR) for e in service_events(app, sid)][1] == IDX[1], msg="the undo reaches the service")
    assert service_events(app, sid)[1]["v"] == 100
    app.keyboard.press("Control+y")                                 # redo brings the sample back
    expect(smp).to_have_value("40000")


def test_return_applies_escape_cancels_and_a_bad_value_is_refused(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    app.keyboard.press("Tab")
    undo0 = app.evaluate("() => S.undo.length")
    smp, vel = app.locator("#ilSample"), app.locator("#ilVel")
    smp.click(); smp.fill("123"); smp.press("Escape")               # cancel
    expect(smp).to_have_value(str(IDX[0]))
    assert app.evaluate("() => S.undo.length") == undo0
    assert app.evaluate("() => S.sel !== null")                      # Escape in the field did not deselect the hit
    smp.click(); smp.fill("12x"); smp.press("Enter")                 # not a number
    expect(app.locator("#stMsg")).to_contain_text("type a whole number")
    expect(smp).to_have_value(str(IDX[0]))
    vel.click(); vel.fill("500"); vel.press("Enter")                 # clamped to 127 with a message
    expect(vel).to_have_value("127")
    expect(app.locator("#stMsg")).to_contain_text("outside 1 to 127")
    vel.click(); vel.fill("127"); vel.press("Enter")                 # no change: no undo step
    assert app.evaluate("() => S.undo.length") == undo0 + 1
    smp.click(); smp.fill("99999999"); smp.press("Enter")            # after the end: the last sample of the track
    n = app.evaluate("() => S.tracks['Kick In'].n_samples")
    expect(smp).to_have_value(str(n - 1))
    smp.click(); smp.fill("5000"); app.locator("#overview").click()  # a field that loses the focus is cancelled
    expect(smp).to_have_value(str(n - 1))


def test_the_info_line_is_shown_and_hidden_by_menu_and_key_and_the_state_is_kept(app, make_project):
    open_project(app, make_project())
    box = app.locator("#infoLine")
    expect(box).to_be_visible()
    assert state(app)["infoLine"] is True
    h0 = app.evaluate("() => document.querySelector('#lanesScroll').getBoundingClientRect().height")
    menu(app, "info_line")                                          # the menu item hides it
    expect(box).to_be_hidden()
    assert state(app)["infoLine"] is False
    app.wait_for_function("h => document.querySelector('#lanesScroll').getBoundingClientRect().height > h", arg=h0)   # the lanes use the room
    assert app.evaluate("() => GEO.scH") > h0 - 1                    # and the geometry (ResizeObserver) knows
    app.reload()                                                    # kept in the app settings
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0")
    expect(box).to_be_hidden()
    app.keyboard.press("Control+i")                                 # the key (Cmd+I in the app; the page maps Control to Cmd)
    expect(box).to_be_visible()
    assert state(app)["infoLine"] is True
    app.wait_for_function("h => Math.abs(document.querySelector('#lanesScroll').getBoundingClientRect().height - h) < 1", arg=h0)
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0")
    expect(box).to_be_visible()
    assert app.evaluate("() => JSON.parse(localStorage.getItem('dsg_settings')).ui.infoLine") is True


def test_the_key_table_has_the_new_rows_and_the_dot_key(app):
    rows = app.evaluate("""() => Object.fromEntries(DSKeys.COMMANDS.filter(c => /^(info_line|counter_)/.test(c.id))
        .map(c => [c.id, [c.path, c.title, c.menu_key, c.page_keys]]))""")
    assert rows == {"info_line": ["View", "Show Info Line", "Cmd+I", []],
                    "counter_seconds": ["View > Main Counter", "Seconds", None, []],
                    "counter_samples": ["View > Main Counter", "Samples", None, []],
                    "counter_exchange": ["View > Main Counter", "Exchange Counters", None, []]}   # P3-19: the key . is a nudge key in the default preset
    assert app.evaluate("() => DSKeys.problems(DSKeys.COMMANDS)") == []


def test_an_info_line_with_a_selected_hit_forces_no_layout_in_the_frames(app, make_project, tmp_path):
    ev = [{"t": i / SR, "v": 90, "id": f"k{n}"} for n, i in enumerate(IDX)]
    open_project(app, build(make_project, tmp_path, events={"Kick In": ev}, seconds=12.0))
    settle(app)
    app.evaluate(PROBE)
    app.keyboard.press("Tab")
    smp = app.locator("#ilSample")
    for k in range(5):                                              # edits from the field, nudges, undo: all in frames
        smp.click(); smp.fill(str(IDX[0] + 100 * (k + 1))); smp.press("Enter")
        app.keyboard.press("ArrowRight")
    app.keyboard.press("Control+z")
    app.evaluate(SWEEP, 40)
    menu(app, "counter_exchange")
    menu(app, "info_line"); menu(app, "info_line")
    settle(app)
    reads = app.evaluate(READS)
    assert reads["n"] == 0, reads


def test_undo_and_redo_keep_the_info_line_on_its_hit(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    app.keyboard.press("Tab"); app.keyboard.press("Tab")
    app.keyboard.press("Delete")                                    # the hit goes: the empty text
    expect(app.locator("#ilEmpty")).to_be_visible()
    app.keyboard.press("Control+z")                                 # it comes back and is selected again
    expect(app.locator("#ilFields")).to_be_visible()
    expect(app.locator("#ilIndex")).to_have_text("2/3")
    expect(app.locator("#ilSample")).to_have_value(str(IDX[1]))
    app.keyboard.press("Shift+ArrowLeft")                           # moved 10 samples earlier, still hit 2
    app.keyboard.press("Control+z")
    expect(app.locator("#ilIndex")).to_have_text("2/3")
    expect(app.locator("#ilOrigin")).to_have_text("detected")
