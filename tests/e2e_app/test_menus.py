"""P3-13a: the native menu bar of docs/migration/ux-daw-reference.md section 3.2, as far as the page goes, through
the bridge. Every item arrives as the event "menu" (the bridge's command selftest_menu, the same payload that a
click on the native item emits). The check items (Snap, Follow Playhead, Metronome Click) follow the toolbar and the
keys, and the toolbar follows them; the page keys of decision 2 (Return, Shift+Return, Option+Return) and the new
page keys C, J, F, Shift+M, Shift+S; the Window menu; the Settings sheet; Help > Keyboard Shortcuts.
The live NSMenu (labels, accelerators, check marks) is checked in the real .app by the self-test scenario `shell`."""
import re
from pathlib import Path

from playwright.sync_api import expect

from app_support import ex_confirm_tempo, export_dialog, invoke, menu, open_project, wait_until

ROOT = Path(__file__).resolve().parents[2]
SR = 48000


def spec_ids():
    """The custom item ids of menu.rs SPEC (item, keyed, check and check_project constructors)."""
    src = (ROOT / "apps/desktop/src-tauri/src/menu.rs").read_text()
    body = src[src.index("// Drum Spike\n"):src.index("/// Every custom item")]
    return re.findall(r'= (?:item|keyed|check|check_project)\(\s*"(\w+)"', body)


def state(page):
    return page.evaluate("() => window.__DS_MENU_STATE__")


def with_hits(page, sid, hits):
    """Set the hits of each track ({name: [t, ...]}) and reload the lanes."""
    for name, ts in hits.items():
        invoke(page, "events_set", sid=sid, name=name, events=[{"t": t, "v": 90, "id": f"{name[0]}{i}"} for i, t in enumerate(ts)])
    page.evaluate("async () => { await loadProject(); }")
    page.wait_for_function("h => Object.entries(h).every(([n, ts]) => (S.events[n] || []).length === ts.length)", arg=hits)


def select_track(page, name):
    page.evaluate("n => { S.selTrack = n; updateStatus(); }", name)


def empty_page(page):
    for s in invoke(page, "project_list"):
        invoke(page, "project_close", sid=s["sid"], force=True)
    page.evaluate("async () => { S.sid = null; setDirtyState(false); await initSessions(); await loadProject(); }")
    page.wait_for_function("() => !S.sid && S.loaded > 0")


def test_every_menu_item_has_a_handler_in_the_page(app):
    ids = spec_ids()
    assert len(ids) == 111 and len(set(ids)) == 111   # #240: Audition on Tab; #255: Drum Spike Help; #169: Show Overview, Add Current, Organize; #157: Other Mics Under Hit; #158: Set Marker at Selection, Next Marker, Previous Marker, Zoom Toggle, Audition Hit, Select Previous Track, Select Next Track; #150: Next and Previous Doubtful Hit; P4-wf10: Suspend Follow when Editing; #436: Stop Returns to Start; P4-31: Export Again; P4-40f: Velocity Lane; P4-40g: Markers Ruler; P4-40a: Undo History; P4-39a: Remove Track; P4-39d: Set Bar 1 at Playhead; P4-30: Export and Export Selected Tracks for five items, no zip items; P4-6a: Learn This, Forget This, Remove Example, Learn Filter, Forget Examples; P4-8: Import Tempo Map, Detect Tempo, three rulers, the grid; P3-18: Own Detection; P3-19: Open Most Recent; P3-20a: Waveform Zoom, Nudge Value; P3-20b: Half-Speed Play; #468: Set Bar 1 at Audio Start; #175: Active Hits Only, All Found Hits (Tab Moves To)
    handlers = set(app.evaluate("() => Object.keys(DSShell.handlers)"))
    assert set(ids) - handlers == set()
    assert handlers - set(ids) == {"recent", "recent_clear", "project", "follow", "zoom_own"}   # the Open Recent, Clear Recent (#441), Window, Follow Hits Of and own zoom preset entries
    assert menu(app, "no_such_item") is False


def test_items_that_need_a_project_say_so_without_one(app):
    empty_page(app)
    for id in ("zoom_in", "zoom_project", "zoom_5ms", "zoom_toggle", "go_end", "stop_return", "next_hit", "prev_hit_all", "delete_hit",
               "add_hit", "markers_set", "marker_next", "marker_prev", "audition", "nudge_later", "analyze_track", "analyze_all", "mute_track",
               "export", "export_selected", "tracks_remove", "rename", "half_speed", "set_bar1", "prev_track", "next_track"):
        app.evaluate("() => setMsg('')")
        assert menu(app, id) is True, id
        expect(app.locator("#stMsg")).to_have_text(re.compile("^no project is open"))


def test_view_items_zoom_around_the_playhead_and_to_the_project(app, make_project):
    open_project(app, make_project(seconds=12.0))
    app.evaluate("() => { S.playhead = 6; setView(2, 10); }")
    menu(app, "zoom_in")
    assert app.evaluate("() => S.view.t1 - S.view.t0") == 4
    menu(app, "zoom_out")
    assert app.evaluate("() => S.view.t1 - S.view.t0") == 8
    menu(app, "zoom_project")
    assert app.evaluate("() => [S.view.t0, S.view.t1]") == [0, 12]
    spans = {"zoom_5s": 5, "zoom_500ms": 0.5, "zoom_50ms": 0.05, "zoom_5ms": 0.005, "zoom_30s": 12, "zoom_full": 12}
    for id, want in spans.items():
        menu(app, id)
        assert abs(app.evaluate("() => S.view.t1 - S.view.t0") - want) < 1e-9, id


def test_transport_go_items_and_the_return_keys(app, make_project):
    """Decision 2: Return goes to the project start, Shift+Return to the playback start (the old Return),
    Option+Return to the project end."""
    open_project(app, make_project(seconds=4.0))
    app.evaluate("() => { S.playStartPos = 1.25; S.playhead = 2.5; redrawAll(); }")
    app.keyboard.press("Enter")
    assert app.evaluate("() => S.playhead") == 0
    app.keyboard.press("Shift+Enter")
    assert app.evaluate("() => S.playhead") == 1.25
    app.keyboard.press("Alt+Enter")
    assert app.evaluate("() => S.playhead") == 4.0
    menu(app, "go_start")
    assert app.evaluate("() => S.playhead") == 0
    menu(app, "go_play_start")
    assert app.evaluate("() => S.playhead") == 1.25
    menu(app, "go_end")
    assert app.evaluate("() => S.playhead") == 4.0
    assert app.evaluate("() => S.view.t0 <= S.playhead && S.playhead <= S.view.t1")   # the view follows
    app.evaluate("() => { S.playhead = 3; }")
    menu(app, "stop_return")                                               # the ■ button: stop and back to the start
    assert app.evaluate("() => S.playhead") == 1.25
    # Play/Stop runs the transport (the null device of the bridge)
    app.evaluate("() => { S.playhead = 0.5; }")
    menu(app, "play_stop")
    app.wait_for_function("() => S.playing")
    assert app.evaluate("() => S.playStartPos") == 0.5
    menu(app, "play_stop")
    app.wait_for_function("() => !S.playing")
    invoke(app, "transport_stop", close=True)                              # the next tests (test_mixer) start without an open transport


def test_check_items_follow_the_toolbar_and_the_keys(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => { setSnap(true); setFollow('smooth'); setClick(true); setInspector(true); }")
    assert state(app) | {"current": None} == {"current": None, "snap": True, "follow": "smooth", "click": True, "infoLine": True, "counter": "seconds", "inspector": True, "trackHeight": "medium", "keyPreset": "default", "projectName": "Song 1", "dirty": False, "exportAgain": False, "undo": False, "redo": False, "selHit": False, "undoTitle": "Undo", "redoTitle": "Redo", "nudgeValue": "1smp", "ampZoom": 1,
                                              "tempoRuler": True, "meterRuler": True, "beatsRuler": True, "markersRuler": False, "grid": True, "velocityLane": False, "otherMics": False, "overview": True, "zoomOwn": [],   # P4-40g: the markers ruler is hidden without markers; P4-40f: velocityLane; #158: undo, redo, selHit; #157: otherMics
                                              "suspendFollow": False,   # P4-wf10
                                              "tabMode": "active",   # #175: View > Tab Moves To
                                              "stopReturnMode": False,   # #436
                                              "auditionTab": False,   # #240
                                              "tracks": ["Kick In", "Snare Top"], "selected": "Kick In", "follows": None,
                                              "learnFilter": False}   # P3-14a: Show Inspector; P3-14c: Track Height; P3-18: Follow Hits Of; P4-6c: Hits > Learn Filter; P4-40f: Velocity Lane
    # Snap Playhead to Hits: the toolbar box, the menu item and J
    app.click("#snapChk")
    assert state(app)["snap"] is False and app.evaluate("() => S.snap") is False
    menu(app, "snap")
    assert app.locator("#snapChk").is_checked() and state(app)["snap"] is True
    app.keyboard.press("j")
    assert not app.locator("#snapChk").is_checked() and state(app)["snap"] is False
    # Follow Playhead: the button, the radio items and F (on/off, back to the last mode)
    app.click("#followBtn")                          # the button cycles smooth, page, off
    assert state(app)["follow"] == "page" and app.evaluate("() => S.followMode") == "page"
    menu(app, "follow_off")
    assert app.locator("#followBtn").get_attribute("data-mode") == "off" and state(app)["follow"] == "off"
    app.keyboard.press("f")
    assert app.locator("#followBtn").get_attribute("data-mode") == "page" and state(app)["follow"] == "page"
    menu(app, "follow_toggle")
    assert state(app)["follow"] == "off"
    menu(app, "follow_smooth")
    assert app.locator("#followBtn").get_attribute("data-mode") == "smooth" and state(app)["follow"] == "smooth"
    # Metronome Click: the 🎵 button, the menu item, C and the Settings box
    app.click("#clickBtn")
    assert state(app)["click"] is False and not app.locator("#setClickOn").is_checked()
    menu(app, "click")
    assert state(app)["click"] is True
    expect(app.locator("#clickBtn")).to_have_class(re.compile(r"\bon\b"))
    app.keyboard.press("c")
    assert state(app)["click"] is False
    expect(app.locator("#stMsg")).to_have_text("metronome click off")
    # a text field keeps the letters: no toggle while it has the focus
    app.evaluate("() => { document.body.insertAdjacentHTML('beforeend', '<input id=tmpKeys>'); document.querySelector('#tmpKeys').focus(); }")
    app.keyboard.type("cjfMS")
    assert app.locator("#tmpKeys").input_value() == "cjfMS"
    assert state(app) | {"current": None} == {"current": None, "snap": False, "follow": "smooth", "click": False, "infoLine": True, "counter": "seconds", "inspector": True, "trackHeight": "medium", "keyPreset": "default", "projectName": "Song 1", "dirty": False, "exportAgain": False, "undo": False, "redo": False, "selHit": False, "undoTitle": "Undo", "redoTitle": "Redo", "nudgeValue": "1smp", "ampZoom": 1,
                                              "tempoRuler": True, "meterRuler": True, "beatsRuler": True, "markersRuler": False, "grid": True, "velocityLane": False, "otherMics": False, "overview": True, "zoomOwn": [],   # P4-40g: the markers ruler is hidden without markers; P4-40f: velocityLane; #158: undo, redo, selHit; #157: otherMics
                                              "suspendFollow": False,   # P4-wf10
                                              "tabMode": "active",   # #175: View > Tab Moves To
                                              "stopReturnMode": False,   # #436
                                              "auditionTab": False,   # #240
                                              "tracks": ["Kick In", "Snare Top"], "selected": "Kick In", "follows": None, "learnFilter": False}
    app.evaluate("() => { document.querySelector('#tmpKeys').remove(); setClick(true); setSnap(true); }")
    assert state(app)["current"] == app.evaluate("() => S.sid")


def test_undo_redo_and_the_selection_items_grey_with_the_state(app, make_project):
    """#158 (rank 9): Edit > Undo and Redo follow the stacks; Delete, the nudge items and Audition Hit follow a hit
    or range selection (the native menu greys them through `sync_checks`, the state keys drive it)."""
    sid = open_project(app, make_project(seconds=12.0))
    with_hits(app, sid, {"Kick In": [1.0, 2.0]})
    select_track(app, "Kick In")
    st = state(app)
    assert st["undo"] is False and st["redo"] is False and st["selHit"] is False
    menu(app, "add_hit")                                                   # one edit: the stack grows
    app.wait_for_function("() => window.__DS_MENU_STATE__.undo === true")
    assert state(app)["redo"] is False
    menu(app, "undo")
    app.wait_for_function("() => window.__DS_MENU_STATE__.undo === false")
    assert state(app)["redo"] is True
    menu(app, "redo")
    app.wait_for_function("() => window.__DS_MENU_STATE__.undo === true")
    assert state(app)["redo"] is False
    # a hit selection enables the selection items, Escape disables them again, a range counts too
    app.evaluate("() => { S.sel = {track: 'Kick In', index: 0}; updateStatus(); }")
    app.wait_for_function("() => window.__DS_MENU_STATE__.selHit === true")
    app.keyboard.press("Escape")
    app.wait_for_function("() => window.__DS_MENU_STATE__.selHit === false")
    app.evaluate("() => lnSetSel('Kick In', 0.9, 1.6)")
    app.wait_for_function("() => window.__DS_MENU_STATE__.selHit === true")
    app.evaluate("() => lnClearSel()")
    app.wait_for_function("() => window.__DS_MENU_STATE__.selHit === false")


def test_the_page_only_commands_have_menu_items_that_run(app, make_project):
    """#158 (rank 8): Audition Hit, the markers, the Zoom Toggle and the track steps are menu items; every keymap
    row of the four names its item and path, and the items run the same commands as the keys."""
    sid = open_project(app, make_project(seconds=12.0))
    with_hits(app, sid, {"Kick In": [1.0, 2.0]})
    select_track(app, "Kick In")
    by = {r["id"]: r for r in app.evaluate("() => DSKeys.COMMANDS")}
    assert by["audition"]["menu"] == "audition" and by["audition"]["path"] == "Hits"
    assert by["markers_set"]["menu"] == "markers_set" and by["markers_set"]["path"] == "Edit"
    assert by["marker_next"]["menu"] == "marker_next" and by["marker_next"]["path"] == "Transport" and by["marker_next"]["menu_key"] == "Option+M"
    assert by["marker_prev"]["menu"] == "marker_prev" and by["marker_prev"]["path"] == "Transport" and by["marker_prev"]["menu_key"] == "Option+Shift+M"
    assert by["zoom_toggle"]["menu"] == "zoom_toggle" and by["zoom_toggle"]["path"] == "View"
    assert by["prev_track"]["menu"] == "prev_track" and by["prev_track"]["path"] == "Track"
    assert by["next_track"]["menu"] == "next_track" and by["next_track"]["path"] == "Track"
    # the items run the page's commands
    menu(app, "next_track")
    assert app.evaluate("() => S.selTrack") == "Snare Top"
    menu(app, "prev_track")
    assert app.evaluate("() => S.selTrack") == "Kick In"
    app.evaluate("() => { S.playhead = 3; }")
    menu(app, "markers_set")                                               # the marker commits before the menu returns
    ms = invoke(app, "markers_get", sid=sid)["markers"]   # markers are service state, no page undo step
    assert any(abs(m["t"] - 3.0) < 1e-9 for m in ms)
    app.evaluate("() => { S.playhead = 0; updateStatus(); }")
    menu(app, "marker_next")
    app.wait_for_function("() => S.playhead === 3")   # 3 is the only marker, so next from 0 is 3
    menu(app, "zoom_toggle")
    app.wait_for_function("() => S.view.t1 - S.view.t0 < 0.01")   # 5 ms around the playhead
    menu(app, "zoom_toggle")                                               # again: back to the zoom before
    app.wait_for_function("() => S.view.t1 - S.view.t0 > 1.0")
    app.evaluate("() => { S.sel = {track: 'Kick In', index: 0}; updateStatus(); }")
    menu(app, "audition")
    app.wait_for_function("() => AU.on")


def test_hits_items_use_the_selected_track_or_all_tracks(app, make_project):
    """Next Hit and Previous Hit (Tab, Option+Tab) work on the selected track when a track header is selected,
    on all tracks when none is; the All Tracks items (Control+Tab) ignore the selection."""
    sid = open_project(app, make_project())
    with_hits(app, sid, {"Kick In": [0.5, 1.5], "Snare Top": [1.0, 2.0]})
    app.evaluate("() => { S.playhead = 0; S.sel = null; }")
    # P4-39c: a track is always selected (the first one at open), so Next Hit walks the hits of Kick In
    seen = []
    for _ in range(2):
        menu(app, "next_hit")
        seen.append(app.evaluate("() => [S.sel.track, S.playhead]"))
    assert seen == [["Kick In", 0.5], ["Kick In", 1.5]]
    select_track(app, "Snare Top")
    menu(app, "next_hit")
    assert app.evaluate("() => [S.sel.track, S.playhead]") == ["Snare Top", 2.0]
    menu(app, "prev_hit")
    assert app.evaluate("() => [S.sel.track, S.playhead]") == ["Snare Top", 1.0]
    menu(app, "next_hit_all")
    assert app.evaluate("() => [S.sel.track, S.playhead]") == ["Kick In", 1.5]
    menu(app, "prev_hit_all")
    assert app.evaluate("() => [S.sel.track, S.playhead]") == ["Snare Top", 1.0]
    app.keyboard.press("Tab")                                              # the page key of Next Hit
    assert app.evaluate("() => [S.sel.track, S.playhead]") == ["Snare Top", 2.0]


def test_edit_items_delete_add_and_nudge_hits(app, make_project):
    sid = open_project(app, make_project())
    with_hits(app, sid, {"Kick In": [0.5, 1.5], "Snare Top": [1.0]})
    app.evaluate("() => { S.sel = null; S.playhead = 2.25; }")
    select_track(app, None)
    menu(app, "delete_hit")
    expect(app.locator("#stMsg")).to_have_text("no hit selected")
    menu(app, "nudge_later")
    expect(app.locator("#stMsg")).to_have_text("no hit selected")
    assert app.evaluate("() => S.selTrack") == "Kick In"                   # P4-39c: a track is always selected, Add Hit has its track
    menu(app, "add_hit")                                                   # at the playhead, snapped to a sample
    assert app.evaluate("() => S.events['Kick In'].map(e => e.t)") == [0.5, 1.5, 2.25]
    assert app.evaluate("() => S.events['Kick In'][2].added") is True
    assert app.evaluate("() => S.sel") == {"track": "Kick In", "index": 2}
    menu(app, "nudge_later")
    menu(app, "nudge_later")
    menu(app, "nudge_earlier")
    assert round(app.evaluate("() => S.events['Kick In'][2].t") * SR) == round(2.25 * SR) + 1
    menu(app, "delete_hit")
    assert app.evaluate("() => S.events['Kick In'].length") == 2
    wait_until(lambda: len(invoke(app, "events_get", sid=sid, name="Kick In")["events"]) == 2, msg="the delete reaches the service")
    menu(app, "undo")
    assert app.evaluate("() => S.events['Kick In'].length") == 3


def test_track_items_mute_solo_export_and_analyze_the_selected_track(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    with_hits(app, sid, {"Kick In": [0.5, 1.5], "Snare Top": [1.0]})
    select_track(app, None)                                                # P4-39c: none is not possible, the first track stays selected
    menu(app, "mute_track")
    assert app.evaluate("() => S.muted['Kick In']") is True
    menu(app, "mute_track")
    assert app.evaluate("() => S.muted['Kick In']") is False
    select_track(app, "Snare Top")
    menu(app, "mute_track")
    assert app.evaluate("() => S.muted['Snare Top']") is True
    app.keyboard.press("Shift+M")
    assert app.evaluate("() => S.muted['Snare Top']") is False
    menu(app, "solo_track")
    assert app.evaluate("() => S.solo['Snare Top']") is True
    app.keyboard.press("Shift+S")
    assert app.evaluate("() => S.solo['Snare Top']") is False
    assert app.evaluate("() => !!S.muted['Kick In'] || !!S.solo['Kick In']") is False
    menu(app, "export_selected")                                           # P4-30: the Export dialog with the selected track
    expect(app.locator("#exportModal")).to_be_visible()
    expect(app.locator("#exScopeSel")).to_be_checked()
    app.locator("#exSpikes").uncheck()
    app.click("#exGo")
    ex_confirm_tempo(app)
    expect(app.locator("#stMsg")).to_contain_text("exported 1 MIDI file to Exports/")
    app.click("#exCancel")
    assert (folder / "Exports" / "Snare Top_spike.mid").is_file()
    export_dialog(app, formats=("spikes",), scope="selected", track="Snare Top", via="menu")
    expect(app.locator("#stMsg")).to_contain_text("exported 1 spike file to Exports/")
    assert not (folder / "Exports" / "Kick In_spike.mid").exists()
    # Analyze Selected Track and Analyze All start the analysis of the selected track or of every track
    app.evaluate("() => { window.__an = []; window.startAnalysis = t => { window.__an.push(t); }; }")
    menu(app, "analyze_track")
    menu(app, "analyze_all")
    assert app.evaluate("() => window.__an") == [["Snare Top"], None]


def test_the_window_menu_switches_between_the_open_projects(app, make_project):
    a = open_project(app, make_project())
    b = invoke(app, "project_open", path=str(make_project()))["sid"]
    app.evaluate("() => setDirtyState(false)")
    assert state(app)["current"] == a
    menu(app, "project", sid=b)
    app.wait_for_function("b => S.sid === b && S.loaded > 0", arg=b)
    assert state(app)["current"] == b
    menu(app, "project")                                                   # an entry without a project does nothing
    assert app.evaluate("() => S.sid") == b


def test_settings_sheet_and_shortcuts_panel(app, make_project):
    open_project(app, make_project())
    menu(app, "settings")
    expect(app.locator("#settingsModal")).to_be_visible()
    for sel in ("#setClickOn", "#setClickVol", "#setPitches input"):
        assert app.locator(sel).count() >= 1, sel
    assert app.locator("#setPitches input").count() == 6
    app.keyboard.press("Escape")
    expect(app.locator("#settingsModal")).to_be_hidden()
    # a sheet waits for nothing: another menu item closes it and runs (a dialog would make the item wait)
    menu(app, "settings")
    app.evaluate("() => setView(0, 8)")
    assert menu(app, "zoom_in") is True
    expect(app.locator("#settingsModal")).to_be_hidden()
    assert app.evaluate("() => S.view.t1 - S.view.t0") == 4
    menu(app, "shortcuts")
    expect(app.locator("#helpModal")).to_be_visible()
    for text in ("Go to Project Start", "Shift+Return", "Snap Playhead to Hits", "Cmd+,"):
        expect(app.locator("#helpModal")).to_contain_text(text)
    assert menu(app, "settings") is True                                   # the panel closes, the sheet opens
    expect(app.locator("#helpModal")).to_be_hidden()
    expect(app.locator("#settingsModal")).to_be_visible()
    app.click("#settingsDone")
    expect(app.locator("#settingsModal")).to_be_hidden()
