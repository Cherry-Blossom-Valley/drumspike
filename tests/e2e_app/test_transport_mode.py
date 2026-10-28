"""#436: the transport keys reach the app, and Stop Returns to Start.

The P3-13a key guard kept the keys of a focused slider from the app (after a click on the
Analyze All button and a move of the histogram's Sensitivity slider, Space did nothing). Now
only a text field keeps its keys — and the status line says when a transport key went into
one. Transport > Stop Returns to Start is a check item like Suspend Follow when Editing (a
persisted user setting): while it is checked, a Stop (Space while playing, or the ■ button)
puts the playhead back where the play started, exactly like Option+Space; unchecked, the stop
keeps the playhead where it is. The setting survives a restart.
"""
import pytest

from app_support import menu, open_project


@pytest.fixture(autouse=True)
def canvas_2d_lanes(app):
    """As in test_playback.py: these tests measure the playhead, not the WebGL2 lanes (SwiftShader is slow)."""
    assert app.evaluate("() => setLanesGL(false)") is False
    yield
    app.evaluate("() => setLanesGL(true)")


def wait_playing(page, ms=10000):
    page.wait_for_function("() => S.playing === true && AC.running === true", timeout=ms)


def wait_stopped(page):
    page.wait_for_function("() => S.playing === false")


def test_space_after_the_analyze_flow_still_plays(app, make_project):
    open_project(app, make_project(seconds=12.0))
    app.click("#analyzeAllBtn")   # the focus sits on the button now
    app.click(".pSens")           # the histogram's Sensitivity slider: the analyze flow's own slider
    app.wait_for_timeout(100)
    assert app.evaluate("() => document.activeElement.classList.contains('pSens')")
    app.keyboard.press("Space")
    wait_playing(app)             # on main the slider keeps the key and nothing plays
    app.keyboard.press("Space")
    wait_stopped(app)


def test_a_text_field_keeps_the_keys_and_the_status_line_says_so(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => { const i = document.createElement('input'); i.id = 'tmField'; document.body.appendChild(i); }")
    app.focus("#tmField")
    app.keyboard.press("Space")
    app.wait_for_timeout(300)
    assert app.evaluate("() => S.playing") is False   # the field keeps the key: no play
    assert "went to" in app.evaluate("() => document.querySelector('#stMsg').textContent"), "the status line says where the key went"
    app.evaluate("() => document.querySelector('#tmField').remove()")


def test_stop_returns_to_start_mode_returns_the_playhead_and_off_keeps_it(app, make_project):
    open_project(app, make_project(seconds=12.0))
    menu(app, "stop_return_mode")
    assert app.evaluate("() => SETTINGS().ui.stopReturnToStart") is True
    app.evaluate("() => { S.playhead = 3; }")
    app.keyboard.press("Space")
    wait_playing(app)
    app.wait_for_function("() => S.playhead > 3.5")
    app.keyboard.press("Space")   # the mode is on: the stop returns to the start of the play
    wait_stopped(app)
    assert app.evaluate("() => S.playhead") == 3, "the playhead is back where the play started"
    # with the mode off the stop keeps the playhead where it is (the old behaviour)
    menu(app, "stop_return_mode")
    assert app.evaluate("() => SETTINGS().ui.stopReturnToStart") is False
    app.keyboard.press("Space")
    wait_playing(app)
    app.wait_for_function("() => S.playhead > 3.5")
    app.keyboard.press("Space")
    wait_stopped(app)
    assert app.evaluate("() => S.playhead") > 3.5


def test_the_mode_returns_every_stop_button_numpad0_and_space(app, make_project):
    """#436 review: with the mode on, every Stop of Play/Stop returns — the ■ button, Numpad0 and Space."""
    open_project(app, make_project(seconds=12.0))
    menu(app, "stop_return_mode")
    assert app.evaluate("() => SETTINGS().ui.stopReturnToStart") is True
    for stop in ("button", "numpad0", "space"):
        app.evaluate("() => { S.playhead = 3; }")
        app.keyboard.press("Space")
        wait_playing(app)
        app.wait_for_function("() => S.playhead > 3.5")
        if stop == "button":
            app.click("#playBtn")   # the ■ button: playStop() with no key
        elif stop == "numpad0":
            app.keyboard.press("Numpad0")
        else:
            app.keyboard.press("Space")
        wait_stopped(app)
        assert app.evaluate("() => S.playhead") == 3, f"the {stop} stop returns to where the play started"


def test_the_mode_is_in_the_key_table_and_survives_a_restart(app, make_project):
    open_project(app, make_project())
    row = app.evaluate("() => DSKeys.COMMANDS.find(c => c.id === 'stop_return_mode')")
    assert row["path"] == "Transport" and row["menu"] == "stop_return_mode"
    assert row["page_keys"] == ["I"] and row["cubase"] is None
    assert app.evaluate("() => DSKeys.problems(DSKeys.COMMANDS).length") == 0   # no duplicate keys, no OS shortcut
    menu(app, "stop_return_mode")
    app.reload()
    app.wait_for_function("() => window.DSShell && DSShell.listening()")
    assert app.evaluate("() => SETTINGS().ui.stopReturnToStart") is True
    menu(app, "stop_return_mode")
    assert app.evaluate("() => SETTINGS().ui.stopReturnToStart") is False
