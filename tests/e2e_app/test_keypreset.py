"""P3-13c: the key preset "Cubase" (docs/migration/ux-daw-reference.md 3.3, owner decision 2026-10-11).

Drum Spike > Settings has a key preset: "Pro Tools (default)" or "Cubase". The choice is an app setting (key_preset in
<home>/settings.json, like the Inspector state). With Cubase the page keys and the menu accelerators come from the
`cubase` column of static/keymap.js: a list REPLACES the keys of the row (the nudge keys are Cmd+Left and Cmd+Right
only), the first key of a list with Cmd, Control or Option is the menu accelerator, the other keys are page keys. Gaps
of P3-13b: Go to Project Start has Return as an alias, Zoom In and Out keep Cmd+] and Cmd+[ beside H and G.
The native menu is not in the bridge: the page tells it the preset with every shell_sync (window.__DS_MENU_STATE__
.keyPreset here); the live accelerators are checked by the self-test `shell` (selftest_menu_report) and by the Rust tests."""
import json

import pytest
from playwright.sync_api import expect

from app_support import edits_done, invoke, menu, open_project, wait_until
from test_inspector import settings_json
from test_menus import select_track, with_hits

SR = 48000


def js(page, expr, arg=None):
    return page.evaluate(expr, arg) if arg is not None else page.evaluate(expr)


@pytest.fixture
def preset_home(app):
    """The bridge's home is shared by the tests: the preset goes back to the default after each test."""
    yield app
    invoke(app, "app_settings_set", key="key_preset", value="default")
    app.evaluate("() => { DSKeys.setPreset('default'); delete window.__DS_NATIVE_MENU__; }")


def choose(page, value):
    """The user's way: the select in the Settings sheet."""
    menu(page, "settings")
    page.select_option("#setKeyPreset", value)
    page.click("#settingsDone")
    expect(page.locator("#settingsModal")).to_be_hidden()
    page.evaluate("() => document.activeElement && document.activeElement.blur()")
    assert js(page, "() => DSKeys.preset") == value


def reload(page):
    page.reload()
    page.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0")


def sel(page):
    return js(page, "() => S.sel && [S.sel.track, S.sel.index]")


def span(page):
    return js(page, "() => S.view.t1 - S.view.t0")


def hit_sample(page, track, index):
    edits_done(page)   # P4-9 stage 2: a nudge is shown when the service answered
    return round(js(page, "([n, i]) => S.events[n][i].t", [track, index]) * SR)


def test_the_settings_sheet_offers_two_presets_and_the_choice_is_saved(preset_home):
    app = preset_home
    menu(app, "settings")
    expect(app.locator("#setKeyPreset")).to_be_visible()
    assert js(app, "() => [...document.querySelectorAll('#setKeyPreset option')].map(o => [o.value, o.textContent])") == \
        [["default", "Pro Tools (default)"], ["cubase", "Cubase"]]
    assert app.input_value("#setKeyPreset") == "default"
    assert js(app, "() => DSKeys.preset") == "default" and settings_json(app).get("key_preset") in (None, "default")
    app.select_option("#setKeyPreset", "cubase")
    assert js(app, "() => DSKeys.preset") == "cubase"
    wait_until(lambda: settings_json(app).get("key_preset") == "cubase", msg="the setting is written")
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.keyPreset") == "cubase", msg="the menu is told")
    expect(app.locator("#stMsg")).to_have_text("key preset: Cubase")
    app.click("#settingsDone")
    # a reload reads the choice back, and the select shows it
    reload(app)
    assert js(app, "() => DSKeys.preset") == "cubase"
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.keyPreset") == "cubase", msg="the menu is told after the reload")
    menu(app, "settings")
    assert app.input_value("#setKeyPreset") == "cubase"
    app.select_option("#setKeyPreset", "default")
    wait_until(lambda: settings_json(app).get("key_preset") == "default", msg="back to the default")
    app.click("#settingsDone")
    # a damaged setting is the default preset; the page refuses a name that is not a preset
    invoke(app, "app_settings_set", key="key_preset", value="Cubase 12")
    reload(app)
    assert js(app, "() => DSKeys.preset") == "default"
    assert js(app, "() => [DSKeys.setPreset('nope'), DSKeys.setPreset(null), DSKeys.setPreset('cubase'), DSKeys.preset]") == \
        [False, False, True, "cubase"]
    assert js(app, "() => setKeyPreset('nope')") is False
    # the settings of the Inspector are not touched by the preset (one file, one key each)
    app.evaluate("() => setInspector(true)")
    wait_until(lambda: settings_json(app).get("inspector") is True, msg="the Inspector setting")
    app.evaluate("() => setKeyPreset('cubase')")
    wait_until(lambda: settings_json(app).get("key_preset") == "cubase", msg="the preset setting")
    assert settings_json(app)["inspector"] is True
    assert json.loads((app.home / "settings.json").read_text())["key_preset"] == "cubase"


def test_cubase_keys_work_and_the_pro_tools_ones_do_not_and_the_reverse(preset_home, make_project):
    app = preset_home
    sid = open_project(app, make_project(seconds=12.0))
    with_hits(app, sid, {"Kick In": [0.5, 1.5, 2.5], "Snare Top": [1.0, 2.0]})
    select_track(app, "Kick In")
    app.evaluate("() => { S.snap = false; S.sel = null; S.playhead = 0; setView(0, 10); }")
    press = app.keyboard.press
    # --- Pro Tools (default): Tab is Next Hit; the Cubase keys do nothing
    press("Tab")
    assert sel(app) == ["Kick In", 0]
    press("Alt+N")
    press("N")                                                                  # the page alias: next hit on all tracks
    assert sel(app) == ["Snare Top", 0]
    press("Shift+Tab")                                                          # previous on the selected track
    app.evaluate("() => { S.sel = {track: 'Kick In', index: 0}; S.playhead = 0.5; setView(2, 10); }")
    press("H")                                                                  # #207: add_hit here: a hit at the playhead on the selected track (the Cubase preset zooms in with H)
    edits_done(app)
    assert js(app, "() => S.events['Kick In'].length") == 4
    press("G")                                                                  # #207: the grid shows; neither key zooms in this preset
    assert span(app) == pytest.approx(8.0)                                      # H and G are no zoom keys here
    press("Meta+z")                                                             # the added hit goes again: the nudge below reads index 0
    edits_done(app)
    assert js(app, "() => S.events['Kick In'].length") == 3
    app.evaluate("() => { S.sel = {track: 'Kick In', index: 0}; S.playhead = 0.5; setView(2, 10); }")
    press("M")
    press("S")
    assert js(app, "() => [!!S.muted['Kick In'], !!S.solo['Kick In']]") == [False, False]
    press("Meta+ArrowLeft")                                                     # no nudge key here
    assert hit_sample(app, "Kick In", 0) == SR // 2
    press("ArrowRight")                                                         # Right nudges the selected hit in Pro Tools
    assert hit_sample(app, "Kick In", 0) == SR // 2 + 1
    press("ArrowLeft")
    assert hit_sample(app, "Kick In", 0) == SR // 2

    # --- Cubase
    choose(app, "cubase")
    app.evaluate("() => { S.sel = {track: 'Kick In', index: 0}; S.playhead = 0.5; updateStatus(); setView(2, 10); }")
    press("Tab")                                                                # replaced by Option+N
    press("Shift+Tab")
    app.evaluate("() => document.activeElement && document.activeElement.blur()")   # an unused Tab moves the browser focus
    assert sel(app) == ["Kick In", 0] and js(app, "() => S.playhead") == 0.5
    press("Alt+N")
    assert sel(app) == ["Kick In", 1] and js(app, "() => S.playhead") == 1.5
    press("Alt+B")
    assert sel(app) == ["Kick In", 0] and js(app, "() => S.playhead") == 0.5
    press("N")                                                                  # next hit on all tracks
    assert sel(app) == ["Snare Top", 0] and js(app, "() => S.playhead") == 1.0
    press("B")
    assert sel(app) == ["Kick In", 0] and js(app, "() => S.playhead") == 0.5
    press("Shift+N")                                                            # no longer a key
    assert sel(app) == ["Kick In", 0]
    press("H")                                                                  # zoom in, out (around the playhead)
    assert span(app) == pytest.approx(4.0)
    press("G")
    assert span(app) == pytest.approx(8.0)
    press("Meta+BracketRight")                                                  # the aliases of the gap
    assert span(app) == pytest.approx(4.0)
    press("Meta+BracketLeft")
    assert span(app) == pytest.approx(8.0)
    press("T")                                                                  # the Pro Tools aliases are gone
    press("R")
    press("=")
    press("Minus")
    assert span(app) == pytest.approx(8.0)
    press("Shift+F")                                                            # zoom to project
    assert js(app, "() => [S.view.t0, S.view.t1]") == [0, 12]
    press("Control+Meta+BracketLeft")                                           # replaced by Shift+F
    assert js(app, "() => [S.view.t0, S.view.t1]") == [0, 12]
    app.evaluate("() => setView(2, 10)")
    press("Meta+ArrowRight")                                                    # the nudge keys REPLACE the old ones
    assert hit_sample(app, "Kick In", 0) == SR // 2 + 1
    press("Meta+ArrowLeft")
    press("Meta+ArrowLeft")
    assert hit_sample(app, "Kick In", 0) == SR // 2 - 1
    for k in ("ArrowRight", "ArrowLeft", "NumpadAdd", "NumpadSubtract"):        # Pro Tools nudge keys: nothing happens
        press(k)
        assert hit_sample(app, "Kick In", 0) == SR // 2 - 1, k
    assert js(app, "() => S.view.t0") == 2                                      # and the view does not pan
    press("Shift+ArrowRight")                                                   # 10 samples: not part of the preset, kept
    assert hit_sample(app, "Kick In", 0) == SR // 2 + 9
    for _ in range(4):                                                          # Shift+Right, two Cmd+Left, Cmd+Right
        press("Meta+z")
    assert hit_sample(app, "Kick In", 0) == SR // 2
    press("Escape")                                                             # nothing selected: Cmd+Arrow pans the view
    app.evaluate("() => setView(2, 7)")
    press("Meta+ArrowRight")
    assert js(app, "() => S.view.t0") == pytest.approx(3.0)
    press("Meta+ArrowLeft")
    assert js(app, "() => S.view.t0") == pytest.approx(2.0)
    select_track(app, "Snare Top")                                              # mute, solo: M and S
    press("M")
    assert js(app, "() => [!!S.muted['Snare Top'], !!S.solo['Snare Top']]") == [True, False]
    press("M")
    press("S")
    assert js(app, "() => [!!S.muted['Snare Top'], !!S.solo['Snare Top']]") == [False, True]
    press("S")
    press("Shift+M")                                                            # the Pro Tools keys are gone
    press("Shift+S")
    assert js(app, "() => [!!S.muted['Snare Top'], !!S.solo['Snare Top']]") == [False, False]
    app.evaluate("() => { S.playhead = 5; S.playStartPos = 2; }")               # transport: Go to Project Start, Stop
    press("Enter")                                                              # the laptop key stays (alias)
    assert js(app, "() => S.playhead") == 0
    app.evaluate("() => { S.playhead = 5; }")
    press("NumpadDecimal")                                                      # Cubase: numpad .
    assert js(app, "() => S.playhead") == 0
    assert js(app, "() => SETTINGS().ui.counter") == "seconds"                  # and it no longer exchanges the counters
    press(".")                                                                  # the main keyboard's . still does
    assert js(app, "() => SETTINGS().ui.counter") == "samples"
    press(".")
    app.evaluate("() => { S.playhead = 5; }")
    press("Numpad0")                                                            # Stop and Return
    assert js(app, "() => S.playhead") == 2
    app.evaluate("() => { S.playhead = 5; }")
    press("Shift+Enter")                                                       # rows without a Cubase list keep their keys
    assert js(app, "() => S.playhead") == 2

    # --- back to Pro Tools: the reverse
    choose(app, "default")
    select_track(app, "Kick In")
    app.evaluate("() => { S.sel = {track: 'Kick In', index: 0}; S.playhead = 0.5; updateStatus(); setView(2, 10); }")
    press("Alt+N")
    press("Meta+ArrowRight")
    press("H")                                                                  # #207: add_hit here: a hit at the playhead (the Cubase preset zooms in with H)
    edits_done(app)
    assert js(app, "() => S.events['Kick In'].length") == 4
    press("G")                                                                  # #207: the grid shows
    press("Meta+z")                                                             # the added hit goes again
    edits_done(app)
    assert js(app, "() => S.events['Kick In'].length") == 3
    app.evaluate("() => { S.sel = {track: 'Kick In', index: 0}; }")
    press("M")
    press("S")
    press("B")
    assert sel(app) == ["Kick In", 0] and span(app) == pytest.approx(8.0) and hit_sample(app, "Kick In", 0) == SR // 2
    press("Tab")
    assert sel(app) == ["Kick In", 1]
    press("ArrowRight")
    assert hit_sample(app, "Kick In", 1) == round(1.5 * SR) + 1
    press("T")
    assert span(app) == pytest.approx(4.0)
    press("Shift+M")
    assert js(app, "() => [!!S.muted['Kick In'], !!S.muted['Snare Top']]") == [True, False]
    press("Shift+M")
    assert js(app, "() => !!S.muted['Kick In']") is False


def test_the_menu_accelerators_follow_the_preset(preset_home, make_project):
    app = preset_home
    sid = open_project(app, make_project(seconds=12.0))
    with_hits(app, sid, {"Kick In": [0.5, 1.5]})
    default = js(app, "() => DSKeys.menuKeys()")
    assert len(default) == 39 and "Control+Tab" in default and "Cmd+]" in default and "Option+N" not in default
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.keyPreset") == "default", msg="the menu state")
    choose(app, "cubase")
    cubase = js(app, "() => DSKeys.menuKeys()")
    assert len(cubase) == 34 and len(set(cubase)) == 34   # #169: Option+O is in both presets; P4-31: Export Again has a key in the default preset only (menu/tests.rs counts 34); #207: Export Selected Tracks and Undo History have keys in both presets
    # what changes: Option+N, Option+B, Option+A (Audition Hit, #158), Cmd+Left and Cmd+Right come; Option+Tab, Control+Tab,
    # Control+Option+Tab and Control+Cmd+[ go (the keys of the hits on all tracks and of Zoom to Project have no modifier in the preset)
    assert set(cubase) - set(default) == {"Option+N", "Option+B", "Cmd+Left", "Cmd+Right", "Cmd+W", "Option+A"}   # P3-19: Cmd+W is a page key in the default preset
    assert set(default) - set(cubase) == {"Option+Tab", "Control+Tab", "Control+Option+Tab", "Control+Cmd+[",
                                          "Shift+Cmd+O", "Control+Shift+K", "Shift+Cmd+W",   # P3-19: the three new menu keys
                                          "Option+Cmd+]", "Option+Cmd+[", "Control+Option+Cmd+[",   # P3-20a: Waveform Zoom
                                          "Option+Cmd+E"}   # P4-31: Export Again, a key in the default preset only
    for k in ("Cmd+]", "Cmd+[", "Cmd+S", "Option+Return", "Control+1", "Option+I"):
        assert k in cubase, k
    for k in ("N", "B", "H", "G", "M", "S", "Shift+F", "Numpad0", "NumpadPeriod", "Return"):
        assert k not in cubase, k                                               # keys without a modifier stay page keys
    owned = js(app, "() => ['Option+N', 'Cmd+Left', 'Cmd+]', 'N', 'H', 'Shift+F', 'Return', 'Control+Tab'].map(k => { const h = DSKeys.table().get(k); return h ? [h.cmd.id, h.menu] : null; })")
    assert owned == [["next_hit", True], ["nudge_earlier", True], ["zoom_in", True], ["next_hit_all", False], ["zoom_in", False],
                     ["zoom_project", False], ["go_start", False], None]
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.keyPreset") == "cubase", msg="the menu state")
    # in the app the native menu runs its accelerators and the page stays out; the page keys still run in the page
    app.evaluate("() => { window.__DS_NATIVE_MENU__ = true; S.sel = null; S.playhead = 0; setView(2, 10); }")
    select_track(app, "Kick In")
    app.keyboard.press("Alt+N")                                                 # the menu owns it
    assert js(app, "() => S.playhead") == 0
    app.keyboard.press("Meta+BracketRight")
    assert span(app) == pytest.approx(8.0)
    app.keyboard.press("H")                                                     # a page key
    assert span(app) == pytest.approx(4.0)
    app.keyboard.press("Control+Tab")                                           # not an accelerator any more: nothing runs
    assert js(app, "() => S.playhead") == 0
    choose(app, "default")
    assert js(app, "() => DSKeys.menuKeys()") == default
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.keyPreset") == "default", msg="the menu state again")
    app.keyboard.press("Control+Tab")                                           # the menu owns it again (the page stays out)
    assert js(app, "() => S.playhead") == 0
    app.evaluate("() => { delete window.__DS_NATIVE_MENU__; }")
    # the check items and the other state keys are still in the same state object
    assert set(js(app, "() => Object.keys(window.__DS_MENU_STATE__)")) >= {"current", "snap", "follow", "click", "inspector", "keyPreset"}


def test_the_help_panel_shows_the_active_preset(preset_home, make_project):
    app = preset_home
    open_project(app, make_project())

    def rows():
        return js(app, """() => Object.fromEntries([...document.querySelectorAll('#keymapTable tr[data-cmd]')]
            .map(r => [r.dataset.cmd, r.children[0].textContent]))""")

    menu(app, "shortcuts")
    expect(app.locator("#keymapPreset")).to_contain_text("Pro Tools (default)")
    assert rows()["next_hit"] == "Tab, '" and rows()["nudge_earlier"] == "Left, numpad -, ,"
    app.click("#helpDone")
    choose(app, "cubase")
    menu(app, "shortcuts")
    expect(app.locator("#keymapPreset")).to_contain_text("Cubase")
    expect(app.locator("#keymapPreset")).not_to_contain_text("Pro Tools")
    r = rows()
    assert r["next_hit"] == "Option+N" and r["prev_hit"] == "Option+B" and r["next_hit_all"] == "N" and r["prev_hit_all"] == "B"
    assert r["zoom_in"] == "H, Cmd+]" and r["zoom_out"] == "G, Cmd+[" and r["zoom_project"] == "Shift+F"
    assert r["nudge_earlier"] == "Cmd+Left" and r["nudge_later"] == "Cmd+Right"          # replaced, not added
    assert r["mute_track"] == "M" and r["solo_track"] == "S"
    assert r["go_start"] == "numpad ., Return" and r["stop_return"] == "numpad 0"
    assert r["save"] == "Cmd+S" and r["go_end"] == "Option+Return" and r["snap"] == "J"   # rows without a Cubase list
    assert len(r) == 113                                                                  # every command is still listed (#169: Show Overview, Add Current, Organize; #157: Other Mics Under Hit; P4-wf10: Suspend Follow when Editing; #436: Stop Returns to Start; #150: Next/Previous Doubtful Hit; P4-31: Export Again; P4-40f: Velocity Lane; P4-40g: Set Marker at Selection, Next Marker, Previous Marker; P4-40d: Audition Hit; P4-6a: four Learn items; P4-36: Forget This; P4-39d: Set Bar 1; P4-39a: Remove Track; P4-40a: Undo History; #468: Set Bar 1 at Audio Start; #149: Bars|Beats and Timecode (30 fps) have rows)
    app.click("#helpDone")
    # the panel follows a change while it is closed, and a reload
    reload(app)
    menu(app, "shortcuts")
    expect(app.locator("#keymapPreset")).to_contain_text("Cubase")
    assert rows()["zoom_in"] == "H, Cmd+]"
    app.click("#helpDone")


def test_the_cubase_table_has_no_key_in_two_commands_and_keeps_the_gap_fixes(app):
    assert js(app, "() => DSKeys.problems(DSKeys.COMMANDS)") == []
    keys = js(app, "() => Object.fromEntries(DSKeys.COMMANDS.map(c => [c.id, DSKeys.keysOf(c, 'cubase')]))")
    assert keys["go_start"] == ["NumpadPeriod", "Return"]
    assert keys["zoom_in"] == ["H", "Cmd+]"] and keys["zoom_out"] == ["G", "Cmd+["]
    assert keys["nudge_earlier"] == ["Cmd+Left"] and keys["nudge_later"] == ["Cmd+Right"]
    # a planted second command on Return is found in the Cubase preset only
    dup = js(app, """() => { const t = DSKeys.COMMANDS.map(c => ({...c, cubase: c.cubase && c.cubase.slice()}));
        t.find(c => c.id === 'go_play_start').cubase = ['Return']; return DSKeys.problems(t); }""")
    assert dup == ["cubase: key Return is in two commands: go_start and go_play_start"]
