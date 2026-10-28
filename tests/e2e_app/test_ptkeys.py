"""P3-19: the Pro Tools key commands (docs/migration/ux-daw-reference.md 3.3, Avid Pro Tools Shortcuts Guide 2025.10 = A1).

The default preset "Pro Tools" gets the keys of card P3-19; the preset "Cubase" keeps its keys. One test for each key
group: track selection (P, ;), hit keys (L, ', Control+Cmd+L and '), undo and redo (Z, Shift+Z), the zoom toggle (E,
Option+Shift+E), the nudge (, and .), the aliases (Option+A, Control+Return, Option+Up and Down, Shift+Cmd+W), the
numpad (0 and 7), Open Most Recent (Shift+Cmd+O), the text field that keeps every one of these keys, and the Cubase
preset that does not react to them. The web page has no native menu, so the keys with a menu id run in the page here
(the native accelerators are checked by the Rust tests and the self-test `shell`)."""
import pytest

from app_support import edits_done, invoke, menu, open_project, wait_until
from test_keypreset import choose, preset_home  # noqa: F401  (the fixture and the Settings sheet helper)
from test_menus import select_track, with_hits

SR = 48000
THREE = (("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Hat", "hat", 42))


def js(page, expr, arg=None):
    return page.evaluate(expr, arg) if arg is not None else page.evaluate(expr)


def span(page):
    return js(page, "() => S.view.t1 - S.view.t0")


def sel(page):
    return js(page, "() => S.selTrack")


def test_p_and_semicolon_select_the_previous_and_the_next_track(app, make_project):
    open_project(app, make_project(tracks=THREE))
    assert sel(app) == "Kick In"                                          # P4-39c: the first track is selected at open
    app.keyboard.press("p")                                               # at the first track nothing happens
    assert sel(app) == "Kick In"
    app.keyboard.press(";")
    app.keyboard.press(";")
    assert sel(app) == "Hat"
    app.keyboard.press(";")                                               # at the last track nothing happens
    assert sel(app) == "Hat"
    app.keyboard.press("p")
    assert sel(app) == "Snare Top"
    app.keyboard.press("p")
    assert sel(app) == "Kick In"
    app.keyboard.press("p")                                               # at the first track nothing happens
    assert sel(app) == "Kick In"
    app.keyboard.press(";")
    assert sel(app) == "Snare Top"
    # the header and the Inspector follow
    assert js(app, "() => document.querySelector('.track.selTrack').dataset.name") == "Snare Top"
    assert js(app, "() => document.querySelector('#insName').value") == "Snare Top"
    select_track(app, None)
    app.keyboard.press(";")                                               # P4-39c: none is not possible: the first track is selected, ; takes the next
    assert sel(app) == "Snare Top"


def test_l_and_quote_step_the_hits_of_the_track_and_control_cmd_the_hits_of_all_tracks(app, make_project):
    sid = open_project(app, make_project(seconds=12.0))
    with_hits(app, sid, {"Kick In": [0.5, 1.5], "Snare Top": [1.0, 2.0]})
    select_track(app, "Snare Top")
    app.evaluate("() => { S.playhead = 0; S.sel = null; setView(0, 10); }")
    at = lambda: js(app, "() => [S.sel.track, S.playhead]")
    app.keyboard.press("'")                                               # next hit on the track (Tab)
    assert at() == ["Snare Top", 1.0]
    app.keyboard.press("'")
    assert at() == ["Snare Top", 2.0]
    app.keyboard.press("l")                                               # previous hit on the track (Option+Tab)
    assert at() == ["Snare Top", 1.0]
    select_track(app, "Kick In")
    app.keyboard.press("l")
    assert at() == ["Kick In", 0.5]
    # all tracks: the same as N and Shift+N
    select_track(app, None)
    app.keyboard.press("Control+Meta+'")
    assert at() == ["Snare Top", 1.0]
    app.keyboard.press("Control+Meta+'")
    assert at() == ["Kick In", 1.5]
    app.keyboard.press("Control+Meta+l")
    assert at() == ["Snare Top", 1.0]


def test_z_undoes_and_shift_z_redoes(app, make_project):
    sid = open_project(app, make_project(seconds=12.0))
    with_hits(app, sid, {"Kick In": [1.0, 2.0]})
    select_track(app, "Kick In")
    app.evaluate("() => { S.playhead = 0; S.sel = null; setView(0, 10); }")
    app.keyboard.press("Tab")
    app.keyboard.press("NumpadAdd")                                       # one edit: 1 sample later
    t = lambda: (edits_done(app), round(js(app, "() => S.events['Kick In'][0].t") * SR))[1]   # P4-9 stage 2: the stored hits are shown after the edit
    assert t() == SR + 1
    app.keyboard.press("z")
    assert t() == SR
    app.keyboard.press("Shift+Z")
    assert t() == SR + 1
    app.keyboard.press("z")
    assert t() == SR
    assert js(app, "() => [S.undo.length, S.redo.length]") == [0, 1]


def test_e_zooms_to_the_hit_and_back(app, make_project):
    sid = open_project(app, make_project(seconds=12.0))
    with_hits(app, sid, {"Kick In": [1.0, 2.0]})
    select_track(app, "Kick In")
    app.evaluate("() => { S.playhead = 0; S.sel = null; setView(1, 9); }")
    app.keyboard.press("Tab")                                             # the hit at 1.0 s is selected
    app.evaluate("() => setView(1, 9)")
    app.keyboard.press("e")
    v = js(app, "() => [S.view.t0, S.view.t1]")
    assert span(app) == pytest.approx(0.005) and (v[0] + v[1]) / 2 == pytest.approx(1.0)   # 5 ms around the hit
    app.keyboard.press("e")                                               # again: the zoom and the position before
    assert js(app, "() => [S.view.t0, S.view.t1]") == [1, 9]
    # Option+Shift+E: stay at the current zoom and forget the stored one
    app.keyboard.press("e")
    assert span(app) == pytest.approx(0.005)
    app.keyboard.press("Alt+Shift+E")
    assert span(app) == pytest.approx(0.005) and js(app, "() => S.zoomBack") is None
    app.evaluate("() => setView(0, 8)")                                   # nothing stored: E zooms in and stores this view
    app.keyboard.press("e")
    assert span(app) == pytest.approx(0.005)
    app.keyboard.press("e")
    assert js(app, "() => [S.view.t0, S.view.t1]") == [0, 8]
    # no hit selected: E zooms on the playhead
    app.evaluate("() => { S.sel = null; S.playhead = 3.0; setView(0, 12); }")
    app.keyboard.press("e")
    v = js(app, "() => [S.view.t0, S.view.t1]")
    assert span(app) == pytest.approx(0.005) and (v[0] + v[1]) / 2 == pytest.approx(3.0)
    app.keyboard.press("e")
    assert js(app, "() => [S.view.t0, S.view.t1]") == [0, 12]


def test_comma_and_period_nudge_the_selected_hit(app, make_project):
    sid = open_project(app, make_project(seconds=12.0))
    with_hits(app, sid, {"Kick In": [1.0, 2.0]})
    select_track(app, "Kick In")
    app.evaluate("() => { S.playhead = 0; S.sel = null; setView(0, 10); }")
    app.keyboard.press("Tab")
    t = lambda: (edits_done(app), round(js(app, "() => S.events['Kick In'][S.sel.index].t") * SR))[1]   # P4-9 stage 2: the stored hits are shown after the edit
    app.keyboard.press(",")
    assert t() == SR - 1
    app.keyboard.press(".")
    app.keyboard.press(".")
    assert t() == SR + 1
    assert js(app, "() => SETTINGS().ui.counter") == "seconds"          # . no longer exchanges the counters
    app.keyboard.press("Escape")                                          # no hit selected: the keys pan the view, as the arrows do
    app.evaluate("() => setView(2, 7)")
    app.keyboard.press(".")
    assert js(app, "() => S.view.t0") == pytest.approx(3.0)
    app.keyboard.press(",")
    assert js(app, "() => S.view.t0") == pytest.approx(2.0)
    # the counter exchange stays on the click of the main counter
    app.click("#timeDisplay")
    assert js(app, "() => SETTINGS().ui.counter") == "samples"
    app.click("#timeDisplay")


def test_option_a_control_return_control_arrows_and_shift_cmd_w(app, make_project):
    open_project(app, make_project(seconds=12.0, tracks=THREE))
    app.evaluate("() => { S.playhead = 6; setView(2, 10); }")
    app.keyboard.press("Alt+a")                                           # Zoom to Project
    assert js(app, "() => [S.view.t0, S.view.t1]") == [0, 12]
    app.evaluate("() => { S.playhead = 2.5; }")
    app.keyboard.press("Control+Enter")                                   # Go to Project Start
    assert js(app, "() => S.playhead") == 0
    h = lambda: js(app, "() => S.trackHeight")
    assert h() == "medium"
    app.keyboard.press("Alt+ArrowDown")                                   # Taller
    assert h() == "large"
    app.keyboard.press("Alt+ArrowUp")                                     # Shorter
    app.keyboard.press("Alt+ArrowUp")
    assert h() == "small"
    app.keyboard.press("Alt+ArrowDown")
    assert h() == "medium"
    # File > Close Project: Shift+Cmd+W is the menu key, Cmd+W stays a page key
    keys = js(app, "() => [DSKeys.keysOf(DSKeys.COMMANDS.find(c => c.id === 'close'), 'default'), DSKeys.menuKeyOf(DSKeys.COMMANDS.find(c => c.id === 'close'), 'default')]")
    assert keys == [["Shift+Cmd+W", "Cmd+W"], "Shift+Cmd+W"]
    sid = js(app, "() => S.sid")
    app.evaluate("() => setDirtyState(false)")
    app.keyboard.press("Meta+w")
    wait_until(lambda: not any(s["sid"] == sid for s in invoke(app, "project_list")), msg="Cmd+W closes the project")


def test_numpad_0_plays_and_stops_and_numpad_7_toggles_the_click(app, make_project):
    open_project(app, make_project(seconds=4.0))
    click0 = js(app, "() => SETTINGS().click.enabled")
    app.evaluate("() => { S.playhead = 0.5; }")                           # the bridge plays on the null device
    app.keyboard.press("Numpad0")
    app.wait_for_function("() => S.playing")
    app.keyboard.press("Numpad0")
    app.wait_for_function("() => !S.playing")
    app.keyboard.press("Numpad7")
    assert js(app, "() => SETTINGS().click.enabled") is (not click0)
    app.keyboard.press("Numpad7")
    assert js(app, "() => SETTINGS().click.enabled") is click0
    app.keyboard.press("Space")                                           # Space plays too
    app.wait_for_function("() => S.playing")
    app.keyboard.press("Space")
    app.wait_for_function("() => !S.playing")


def test_open_most_recent_opens_the_first_entry_of_open_recent(app, make_project):
    folders = [make_project(tracks=(("Kick In", "kick", 36),)) for _ in range(2)]
    open_project(app, folders[0])
    invoke(app, "project_open", path=str(folders[1]))                   # the newest recent entry, not the page's project
    app.evaluate("() => setDirtyState(false)")
    listed = app.evaluate("() => B.recentProjects()")
    assert listed[0]["folder"].endswith(str(folders[1]).rsplit("/", 1)[-1])
    folder = lambda: app.evaluate("async () => S.sid ? (await B.projectGet(S.sid)).folder : ''")
    assert folder() != listed[0]["folder"]
    assert menu(app, "open_recent") is True
    wait_until(lambda: folder() == listed[0]["folder"] or folder().rstrip("/") == str(folders[1]).rstrip("/"), msg="the first recent entry is open")
    row = js(app, "() => DSKeys.COMMANDS.find(c => c.id === 'open_recent')")
    assert row["menu_key"] == "Shift+Cmd+O" and row["path"] == "File"
    # the key runs the item in the web page too
    open_project(app, folders[0])
    app.evaluate("() => setDirtyState(false)")
    invoke(app, "project_open", path=str(folders[1]))
    app.keyboard.press("Shift+Meta+O")
    wait_until(lambda: folder().rstrip("/") == str(folders[1]).rstrip("/"), msg="Shift+Cmd+O opens the most recent project")


def test_a_text_field_keeps_every_new_key(app, make_project):
    sid = open_project(app, make_project(seconds=12.0, tracks=THREE))
    with_hits(app, sid, {"Kick In": [1.0, 2.0]})
    select_track(app, "Kick In")
    app.evaluate("() => { S.playhead = 0; S.sel = null; setView(2, 7); }")
    app.keyboard.press("Tab")
    app.evaluate("() => { setView(2, 7); const i = document.createElement('input'); i.type = 'text'; i.id = 'probe'; document.body.appendChild(i); i.focus(); }")
    before = js(app, "() => JSON.stringify([S.selTrack, S.sel, S.view, S.playhead, S.events['Kick In'], S.undo.length, S.zoomBack || null, SETTINGS().ui.counter])")
    for k in ("p", ";", "l", "'", "z", "Shift+Z", "e", ",", ".", "Alt+a", "Alt+Shift+E"):
        app.keyboard.press(k)
    assert js(app, "() => JSON.stringify([S.selTrack, S.sel, S.view, S.playhead, S.events['Kick In'], S.undo.length, S.zoomBack || null, SETTINGS().ui.counter])") == before
    assert js(app, "() => document.querySelector('#probe').value").startswith("pl'z") or js(app, "() => document.querySelector('#probe').value").startswith("p;l'z")   # the field got the characters
    app.evaluate("() => document.querySelector('#probe').remove()")


def test_the_cubase_preset_does_not_have_the_pro_tools_keys(preset_home, make_project):
    app = preset_home
    sid = open_project(app, make_project(seconds=12.0, tracks=THREE))
    with_hits(app, sid, {"Kick In": [1.0, 2.0], "Snare Top": [1.0, 2.0]})
    choose(app, "cubase")
    select_track(app, "Snare Top")                                        # a middle track: P and ; would move the selection
    app.evaluate("() => { S.playhead = 0; S.sel = null; setView(2, 7); }")
    state = lambda: js(app, "() => JSON.stringify([S.selTrack, S.sel, S.view, S.playhead, S.events['Kick In'], S.undo.length, S.zoomBack || null, S.playing])")
    before = state()
    for k in ("p", ";", "l", "'", "z", "Shift+Z", "e", ",", "Alt+a", "Alt+Shift+E", "Control+Enter"):
        app.keyboard.press(k)
        assert state() == before, k
    # the Cubase keys are the old ones: . exchanges the counters, Numpad0 is Stop and Return, Return is the start
    assert js(app, "() => SETTINGS().ui.counter") == "seconds"
    app.keyboard.press(".")
    assert js(app, "() => SETTINGS().ui.counter") == "samples"
    app.keyboard.press(".")
    app.keyboard.press("Meta+z")
    assert js(app, "() => DSKeys.table('cubase').get('Cmd+Z').cmd.id") == "undo"
    assert js(app, "() => DSKeys.table('cubase').get('Cmd+W').cmd.id") == "close"
    assert js(app, "() => DSKeys.table('cubase').get('Cmd+Down').cmd.id") == "height_taller"
