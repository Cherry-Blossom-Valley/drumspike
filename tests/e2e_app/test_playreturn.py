"""#207: play-and-return in the page on the native transport, through the bridge with the null device
(silent: DRUMSPIKE_AUDIO=null, DRUMSPIKE_MASTER_GAIN=0). Option+Space plays from the playhead; the next
Space (or Option+Space again) stops and puts the playhead back where this play started. Option+Shift+Space
is the same at half speed. Space and Shift+Space keep their behaviour (play / pause where the playhead is).
The stop side is the Transport > Stop and Return command (stopAndReturn, the playStartPos of this play)."""
import pytest

from app_support import open_project


@pytest.fixture(autouse=True)
def canvas_2d_lanes(app):
    """As in test_playback.py: these tests measure the playhead, not the WebGL2 lanes (SwiftShader is slow)."""
    assert app.evaluate("() => setLanesGL(false)") is False
    yield
    app.evaluate("() => setLanesGL(true)")


def wait_playing(page, speed, ms=10000):
    page.wait_for_function("() => S.playing === true && AC.running === true", timeout=ms)
    page.wait_for_function(f"() => AC.speed === {speed}", timeout=ms)


def wait_stopped(page):
    page.wait_for_function("() => S.playing === false")


def test_option_space_plays_and_the_next_space_or_the_same_key_returns(app, make_project):
    open_project(app, make_project(seconds=12.0))
    app.evaluate("() => { S.playhead = 3; }")
    app.keyboard.press("Alt+Space")                                       # Option+Space: play-and-return
    wait_playing(app, 1)
    assert app.evaluate("() => [S.playStartPos, S.playReturn]") == [3, "Option+Space"]
    app.wait_for_function("() => S.playhead > 3.5")                       # the play moves, the playhead runs
    app.keyboard.press("Space")                                           # the next Space stops and returns
    wait_stopped(app)
    assert app.evaluate("() => S.playhead") == 3, "the playhead is back at the start frame"
    app.keyboard.press("Alt+Space")                                       # the same key returns too
    wait_playing(app, 1)
    app.wait_for_function("() => S.playhead > 3.5")
    app.keyboard.press("Alt+Space")
    wait_stopped(app)
    assert app.evaluate("() => S.playhead") == 3
    # a play that Space starts keeps its behaviour: the stop leaves the playhead where it is
    app.keyboard.press("Space")
    wait_playing(app, 1)
    assert app.evaluate("() => S.playReturn") is None
    app.wait_for_function("() => S.playhead > 3.5")
    app.keyboard.press("Space")
    wait_stopped(app)
    p = app.evaluate("() => S.playhead")
    assert p > 3.5, p


def test_option_shift_space_is_play_and_return_at_half_speed(app, make_project):
    open_project(app, make_project(seconds=12.0))
    app.evaluate("() => { S.playhead = 2; }")
    app.keyboard.press("Alt+Shift+Space")                                 # Option+Shift+Space: half speed
    wait_playing(app, 0.5)
    assert app.evaluate("() => [S.playStartPos, S.playReturn]") == [2, "Option+Shift+Space"]
    app.wait_for_function("() => S.playhead > 2.2")
    app.keyboard.press("Space")                                           # the next Space stops and returns
    wait_stopped(app)
    assert app.evaluate("() => S.playhead") == 2, "the playhead is back at the start frame"
    app.keyboard.press("Alt+Shift+Space")
    wait_playing(app, 0.5)
    app.wait_for_function("() => S.playhead > 2.2")
    app.keyboard.press("Alt+Shift+Space")                                 # the same key returns too
    wait_stopped(app)
    assert app.evaluate("() => S.playhead") == 2
    # a play that Shift+Space starts keeps its behaviour: the stop leaves the playhead where it is
    app.keyboard.press("Shift+Space")
    wait_playing(app, 0.5)
    assert app.evaluate("() => S.playReturn") is None
    app.wait_for_function("() => S.playhead > 2.2")
    app.keyboard.press("Shift+Space")
    wait_stopped(app)
    p = app.evaluate("() => S.playhead")
    assert p > 2.2, p


def test_play_and_return_is_in_the_key_table_and_a_text_field_keeps_the_keys(app, make_project):
    open_project(app, make_project())
    play = app.evaluate("() => DSKeys.COMMANDS.find(c => c.id === 'play_stop')")
    assert play["page_keys"] == ["Space", "Numpad0", "Option+Space"] and play["cubase"] == ["Space"]
    half = app.evaluate("() => DSKeys.COMMANDS.find(c => c.id === 'half_speed')")
    assert half["page_keys"] == ["Shift+Space", "Option+Shift+Space"] and half["cubase"] == []
    assert app.evaluate("() => DSKeys.problems(DSKeys.COMMANDS).length") == 0
    # a text field keeps the keys (the page runs the key table only outside a field)
    app.evaluate("() => { const i = document.createElement('input'); i.id = 'prField'; document.body.appendChild(i); }")
    app.focus("#prField")
    app.keyboard.press("Alt+Space")
    app.wait_for_timeout(300)
    assert app.evaluate("() => S.playing") is False
    app.keyboard.press("Alt+Shift+Space")
    app.wait_for_timeout(300)
    assert app.evaluate("() => S.playing") is False
    app.evaluate("() => document.querySelector('#prField').remove()")
