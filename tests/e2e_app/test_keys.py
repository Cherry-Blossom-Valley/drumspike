"""P3-13b: the key command table (static/keymap.js, docs/migration/ux-daw-reference.md 3.3 and 5).

One table holds every command: menu.rs takes its accelerators from it (a Rust test compares each item with its row),
app.js takes its keys from it, and Help > Keyboard Shortcuts is made from it. These tests check the page side: the
table has no key in two commands (and the check finds a planted one), every menu item has its row, the help panel
lists every command, the keys run their commands (the numpad nudge, Shift+arrow, the menu keys in the web page),
and Option-click adds a hit."""
import re
from pathlib import Path

import pytest
from playwright.sync_api import expect

from app_support import edits_done, invoke, menu, open_project
from test_menus import select_track, spec_ids, with_hits

ROOT = Path(__file__).resolve().parents[2]
SR = 48000


def js(page, expr, arg=None):
    return page.evaluate(expr, arg) if arg is not None else page.evaluate(expr)


def span(page):
    return js(page, "() => S.view.t1 - S.view.t0")


def test_the_key_table_has_no_key_in_two_commands_and_finds_a_planted_one(app):
    assert js(app, "() => DSKeys.problems(DSKeys.COMMANDS)") == []
    # plant 1: J (Snap Playhead to Hits) also on Follow Playhead On/Off
    dup = js(app, """() => {
        const t = DSKeys.COMMANDS.map(c => ({...c, page_keys: c.page_keys.slice()}));
        t.find(c => c.id === "follow_toggle").page_keys.push("J");
        return DSKeys.problems(t);
    }""")
    assert dup == ["default: key J is in two commands: follow_toggle and snap",
                   "cubase: key J is in two commands: follow_toggle and snap"]
    # plant 2: the other mistakes the check knows
    bad = js(app, """() => {
        const t = DSKeys.COMMANDS.map(c => ({...c, page_keys: c.page_keys.slice()}));
        t[0].menu_key = "Q";                                  // a menu key without Cmd, Control or Option
        t[1].id = "about";                                    // an id twice
        t[1].page_keys = ["Cmd+Shift+X", "Shift+="];          // modifiers out of order; Shift on a character
        t.push({id: "x", menu: null, path: null, title: "X", menu_key: "Cmd+K", page_keys: ["Cmd+K"], cubase: ["Shift+F"], note: null});
        return DSKeys.problems(t);
    }""")
    for want in ("about: menu key Q has no Cmd, Control or Option", "about: the id is twice in the table",
                 'about: key "Cmd+Shift+X" is not in the table\'s notation', 'about: key "Shift+=" is not in the table\'s notation',
                 "x: menu key Cmd+K without a menu item", "default: key Cmd+K is twice in x",
                 "cubase: key Shift+F is in two commands: zoom_project and x"):
        assert want in bad, (want, bad)
    # the Cubase preset (P3-13c) has its own key map: N, B, H, G, M, S, Shift+F
    cub = js(app, "() => ['N', 'B', 'H', 'G', 'M', 'S', 'Shift+F', 'Tab'].map(k => (DSKeys.table('cubase').get(k) || {cmd: {id: null}}).cmd.id)")
    assert cub == ["next_hit_all", "prev_hit_all", "zoom_in", "zoom_out", "mute_track", "solo_track", "zoom_project", None]


def test_every_menu_item_has_one_row_and_menu_rs_has_no_keys_of_its_own(app):
    rows = js(app, "() => DSKeys.COMMANDS.filter(c => c.menu).map(c => c.menu)")
    assert rows == spec_ids()                                             # the same 103 items, in the menu's order
    assert js(app, "() => DSKeys.COMMANDS.length") == 119                 # #255: Drum Spike Help; #265: Re-analyze Selection; #242: Toggle Hit Active; #240: Audition on Tab; plus 6 keys without a menu item (#169: Show Overview, Add Current, Organize have rows; #157: Other Mics Under Hit has a menu item; P4-wf10: Suspend Follow when Editing, no key; #436: Stop Returns to Start; #158 gave Set Marker at Selection, Next Marker, Previous Marker, Audition Hit, Zoom Toggle and the track steps menu items; the rest: the nudge x10 aliases, Nudge Value Up/Down, Keep the Zoom, Deselect; #150 and P4-31: 98 items; P4-40f: Velocity Lane; P4-40a: 88 items; P4-39a and P4-39d: 85 items; 74 items since P3-20a, 75 since P3-20b, 81 since P4-8, 85 since P4-6a, 82 since P4-30; #468: Set Bar 1 at Audio Start; #149: Bars|Beats and Timecode (30 fps) have menu items; #175: Tab Skips Deactivated Hits, All Found Hits have rows)
    assert js(app, "() => DSKeys.menuKeys().length") == 39                 # #169: Show Overview; P3-19: Open Most Recent, Keyboard Shortcuts; P3-20a: Waveform Zoom In, Out, Reset; P4-31: Export Again; #158: Next Marker, Previous Marker; #207: Export Selected Tracks, Undo History
    handlers = set(js(app, "() => Object.keys(DSShell.handlers)"))
    assert set(rows) == handlers - {"recent", "recent_clear", "project", "follow", "zoom_own"}   # P3-18: the Follow Hits Of entries; #169: the own zoom presets; #441: Clear Recent
    # menu.rs sets an accelerator only through accel_of (the table); its items carry no key strings
    src = (ROOT / "apps/desktop/src-tauri/src/menu.rs").read_text()
    main = src[:src.index("#[cfg(test)]")]
    assert re.findall(r"\.accelerator\(([^)]*)\)", main) == ["a", "a"]
    assert main.count("if let Some(a) = accel_of(it.id, current_preset())") == 2
    assert not re.search(r'"(CmdOrCtrl|Ctrl|Alt)\+', main)
    assert 'include_str!("../../../../static/keymap.js")' in main


def test_the_help_panel_lists_every_command_of_the_table(app, make_project):
    open_project(app, make_project())
    menu(app, "shortcuts")
    expect(app.locator("#helpModal")).to_be_visible()
    rows = js(app, """() => [...document.querySelectorAll('#keymapTable tr[data-cmd]')]
        .map(r => [r.dataset.cmd, r.children[0].textContent, r.children[1].textContent])""")
    table = js(app, "() => DSKeys.COMMANDS.map(c => [c.id, DSKeys.keysOf(c, 'default').map(DSKeys.display), c.title])")
    assert [r[0] for r in rows] == [t[0] for t in table]
    for (cid, keys, where), (_, want, title) in zip(rows, table):
        assert title in where, cid
        assert keys == (", ".join(want) or "none"), cid
    by = {r[0]: r for r in rows}
    assert by["redo"][1] == "Shift+Cmd+Z, Shift+Z, Cmd+Y" and by["redo"][2] == "Edit > Redo"
    assert by["nudge_earlier"][1] == "Left, numpad -, ,"
    assert by["zoom_in"][1] == "Cmd+], T, +, =" and by["zoom_out"][1] == "Cmd+[, R, -, _"
    assert by["zoom_5s"][2] == "View > Zoom Presets > 5 s"
    assert by["about"][1] == "none"
    assert by["deselect"][2] == "Deselect the Hit"
    groups = js(app, "() => [...document.querySelectorAll('#keymapTable tr.kmGroup')].map(r => r.textContent)")
    assert groups == ["Drum Spike", "File", "Edit", "View", "Transport", "Hits", "Track", "Help", "Keys without a menu item", "Mouse"]
    assert js(app, "() => document.querySelectorAll('#keymapTable tr:not(.kmGroup):not([data-cmd])').length") == js(app, "() => DSKeys.GESTURES.length")
    expect(app.locator("#helpModal")).to_contain_text("Option-click")
    app.click("#helpDone")


def test_no_key_is_a_macos_standard_shortcut_and_none_is_twice(app):
    """#207, owner rule of 2026-10-21: no key of the table is a macOS standard shortcut, in either preset,
    not even as an extra key; the whole keymap holds no key twice (DSKeys.problems FAILs on one). Cmd+Q on
    Quit and Cmd+, on Settings stay (the macOS standard keys for exactly those commands)."""
    reserved = ["Cmd+Space", "Control+Space", "Control+Up", "Control+Down", "Control+Left", "Control+Right",
                "Cmd+Tab", "Cmd+`", "Shift+Cmd+3", "Shift+Cmd+4", "Shift+Cmd+5", "Cmd+H", "Cmd+M", "Cmd+Option+Escape"]
    allowed = {"Cmd+Q": "quit", "Cmd+,": "settings"}
    assert js(app, "() => DSKeys.problems(DSKeys.COMMANDS)") == []        # the whole keymap: a duplicate FAILs here
    bad = js(app, """([reserved, allowed]) => DSKeys.COMMANDS.flatMap(c => ['default', 'cubase']
        .flatMap(p => DSKeys.keysOf(c, p).map(k => [p, c.id, k])))
        .filter(([p, id, k]) => reserved.includes(k) && allowed[k] !== id)""", [reserved, allowed])
    assert bad == [], bad


def test_numpad_and_shift_arrow_keys_nudge_the_selected_hit_or_pan(app, make_project):
    sid = open_project(app, make_project(seconds=12.0))
    with_hits(app, sid, {"Kick In": [1.0, 2.0]})
    select_track(app, "Kick In")
    app.evaluate("() => { S.playhead = 0; S.sel = null; setView(0, 10); }")
    app.keyboard.press("Tab")                                             # select the hit at 1.0 s
    t = lambda: (edits_done(app), round(js(app, "() => S.events['Kick In'][S.sel.index].t") * SR))[1]   # P4-9 stage 2: the stored hits are shown after the edit
    assert t() == SR
    app.keyboard.press("NumpadSubtract")                                  # Pro Tools: numpad minus nudges earlier
    assert t() == SR - 1
    app.keyboard.press("NumpadAdd")
    app.keyboard.press("NumpadAdd")
    assert t() == SR + 1
    app.keyboard.press("Shift+ArrowLeft")                                 # 10 samples
    assert t() == SR - 9
    app.keyboard.press("Shift+ArrowRight")
    assert t() == SR + 1
    app.keyboard.press("Escape")                                          # Deselect the Hit
    assert js(app, "() => S.sel") is None
    app.evaluate("() => setView(2, 7)")
    app.keyboard.press("NumpadAdd")                                       # no hit selected: pan 20 %
    assert js(app, "() => S.view.t0") == pytest.approx(3.0) and span(app) == pytest.approx(5.0)
    app.keyboard.press("NumpadSubtract")
    app.keyboard.press("Shift+ArrowLeft")
    assert js(app, "() => S.view.t0") == pytest.approx(1.0) and span(app) == pytest.approx(5.0)
    app.keyboard.press("Numpad1")                                         # no numpad command: the main key 1 (Full)
    assert span(app) == pytest.approx(12.0)
    app.keyboard.press("Delete")                                          # no hit selected: Delete does nothing
    assert js(app, "() => S.events['Kick In'].length") == 2


def test_option_click_on_a_lane_adds_a_hit(app, make_project):
    sid = open_project(app, make_project(seconds=12.0))   # P4-9 stage 2: the service refuses a hit after the end of the track
    with_hits(app, sid, {"Kick In": [1.0]})
    app.evaluate("() => { S.snap = false; setView(0, 10); }")
    row = app.locator(".track").first
    spike = row.locator(".spike")
    box = spike.bounding_box()
    spike.click(position={"x": box["width"] * 0.5, "y": box["height"] / 2}, modifiers=["Alt"])
    edits_done(app)
    ev = js(app, "() => S.events['Kick In'].map(e => [e.t, !!e.added])")
    assert len(ev) == 2 and ev[1][1] is True and 4.9 < ev[1][0] < 5.1
    assert float(ev[1][0] * SR) == pytest.approx(round(ev[1][0] * SR))  # on a sample
    assert js(app, "() => S.sel") == {"track": "Kick In", "index": 1}
    wave = row.locator(".wave")
    wb = wave.bounding_box()
    wave.click(position={"x": wb["width"] * 0.3, "y": wb["height"] / 2}, modifiers=["Alt"])
    edits_done(app)
    ev = js(app, "() => S.events['Kick In'].map(e => e.t)")
    assert len(ev) == 3 and 2.9 < ev[1] < 3.1
    menu(app, "undo")
    edits_done(app)
    assert js(app, "() => S.events['Kick In'].length") == 2


def test_the_menu_keys_run_in_the_web_page_and_stay_out_in_the_app(app, make_project):
    sid = open_project(app, make_project(seconds=12.0))
    with_hits(app, sid, {"Kick In": [0.5, 1.5], "Snare Top": [1.0, 2.0]})
    app.evaluate("() => { S.playhead = 6; setView(2, 10); }")
    app.keyboard.press("Meta+BracketRight")                               # View > Zoom In (no native menu here)
    assert span(app) == pytest.approx(4.0)
    app.keyboard.press("Meta+BracketLeft")
    assert span(app) == pytest.approx(8.0)
    app.keyboard.press("Control+Meta+BracketLeft")                        # Zoom to Project
    assert js(app, "() => [S.view.t0, S.view.t1]") == [0, 12]
    app.keyboard.press("=")                                               # the aliases of + and minus
    assert span(app) == pytest.approx(6.0)
    app.keyboard.press("Shift+Minus")                                     # _
    assert span(app) == pytest.approx(12.0)
    select_track(app, None)
    app.evaluate("() => { S.playhead = 0; S.sel = null; }")
    app.keyboard.press("Control+Tab")                                     # Next Hit on All Tracks
    app.keyboard.press("Control+Tab")
    assert js(app, "() => [S.sel.track, S.playhead]") == ["Snare Top", 1.0]
    app.keyboard.press("Control+Alt+Tab")                                 # Previous Hit on All Tracks
    assert js(app, "() => [S.sel.track, S.playhead]") == ["Kick In", 0.5]
    app.keyboard.press("n")                                               # the page aliases N and Shift+N
    app.keyboard.press("n")
    assert js(app, "() => [S.sel.track, S.playhead]") == ["Kick In", 1.5]
    app.keyboard.press("Shift+N")
    assert js(app, "() => [S.sel.track, S.playhead]") == ["Snare Top", 1.0]
    select_track(app, "Snare Top")
    app.keyboard.press("Tab")
    assert js(app, "() => [S.sel.track, S.playhead]") == ["Snare Top", 2.0]
    app.keyboard.press("Alt+Tab")                                         # Previous Hit on the selected track
    assert js(app, "() => [S.sel.track, S.playhead]") == ["Snare Top", 1.0]
    app.keyboard.press("Tab")
    app.keyboard.press("Shift+Tab")
    assert js(app, "() => S.playhead") == 1.0
    n = app.invokes.count("project_save")
    app.evaluate("() => markDirty('Kick In')")
    app.keyboard.press("Meta+s")
    app.wait_for_timeout(300)
    assert app.invokes.count("project_save") == n + 1
    # in the app the native menu runs these keys: the page stays out
    app.evaluate("() => { window.__DS_NATIVE_MENU__ = true; setView(2, 10); }")
    for k in ("Meta+BracketRight", "Meta+BracketLeft", "Control+Meta+BracketLeft", "Control+2"):
        app.keyboard.press(k)
        assert js(app, "() => [S.view.t0, S.view.t1]") == [2, 10], k
    app.keyboard.press("t")                                               # a page key still runs in the page
    assert span(app) == pytest.approx(4.0)
    app.evaluate("() => { delete window.__DS_NATIVE_MENU__; }")


def test_key_notation_of_the_page(app):
    key = lambda o: js(app, "o => DSKeys.keyOf(new KeyboardEvent('keydown', o))", o)
    assert key({"key": "S", "code": "KeyS", "metaKey": True, "shiftKey": True}) == "Shift+Cmd+S"
    assert key({"key": "Dead", "code": "KeyN", "altKey": True}) == "Option+N"     # Option+N on macOS is a dead key
    assert key({"key": "+", "shiftKey": True}) == "Plus"                          # a character carries its Shift
    assert key({"key": "_", "shiftKey": True}) == "_"
    assert key({"key": "-", "code": "NumpadSubtract"}) == "NumpadMinus"
    assert key({"key": "1", "code": "Numpad1"}) == "Numpad1"
    assert key({"key": "!", "code": "Digit1", "shiftKey": True}) == "Shift+1"
    assert key({"key": "Enter", "ctrlKey": True, "altKey": True}) == "Control+Option+Return"
    assert key({"key": "ArrowUp"}) == "Up" and key({"key": " "}) == "Space"
    assert key({"key": "Unidentified"}) is None
    show = js(app, "() => ['NumpadMinus', 'Numpad0', 'NumpadEnter', 'Control+Cmd+[', 'Minus', 'no key'].map(DSKeys.display)")
    assert show == ["numpad -", "numpad 0", "numpad Enter", "Control+Cmd+[", "-", "no key"]
    tauri = js(app, "() => ['Shift+Cmd+S', 'Control+Option+Tab', 'Option+Return', 'Cmd+Left', 'NumpadPeriod', 'Cmd+Shift+S'].map(DSKeys.tauriOf)")
    assert tauri == ["CmdOrCtrl+Shift+S", "Ctrl+Alt+Tab", "Alt+Enter", "CmdOrCtrl+ArrowLeft", "NumpadDecimal", None]
    assert js(app, "() => [DSKeys.parse(''), DSKeys.parse(3), DSKeys.parse('Shift+[')]") == [None, None, None]
    assert js(app, "() => DSKeys.compose(['Cmd', 'Control'], 'X')") == "Control+Cmd+X"
    assert js(app, "() => DSKeys.renderHelp(null)") == 0
    assert js(app, "() => DSKeys.find(new KeyboardEvent('keydown', {key: 'Unidentified'}))") is None
    assert js(app, "() => DSKeys.find(new KeyboardEvent('keydown', {key: 'z', ctrlKey: true})).cmd.id") == "undo"   # Control as Cmd
    assert js(app, "() => DSKeys.find(new KeyboardEvent('keydown', {key: '1', code: 'Numpad1', shiftKey: true}))") is None
    assert js(app, "() => DSKeys.find(new KeyboardEvent('keydown', {key: '-', code: 'NumpadSubtract', shiftKey: true})).cmd.id") == "zoom_out"
    assert js(app, "() => runKey('no_such_command')") is False
    assert js(app, "() => { runKey('shortcuts'); return !document.querySelector('#helpModal').classList.contains('hidden'); }") is True
    app.click("#helpDone")
