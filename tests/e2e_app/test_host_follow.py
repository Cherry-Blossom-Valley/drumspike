"""#518: the page follows the host transport at any time (DAW plugin P3).

In a DAW the user starts and stops playback in the DAW: the page must follow the host
transport with no Play in the page. The same stub as in test_host_mode.py records the
calls and pushes playhead messages with send(m); here nothing ever calls play(), so any
following is the page's own.

On main these tests fail: the page registers for host positions only inside its own Play
(appPlay), so with no Play nothing follows (the red proof is in the PR)."""
import pytest

from app_support import open_project

STUB = """() => {
  B.hostMode = true;
  B.hostTransport = {
    calls: [],
    play: function (position, speed) { this.calls.push(["play", position, speed === undefined ? null : speed]); return Promise.resolve(); },
    stop: function (position) { this.calls.push(["stop", position]); return Promise.resolve(); },
    locate: function (position) { this.calls.push(["locate", position]); return Promise.resolve(); },
    send: function (m) { if (this.onPosition) this.onPosition(m); },
  };
  return B.hostMode;
}"""


def set_host_mode(app):
    """Install the stub, then call appHostFollow() — the same step a plugin shell does."""
    assert app.evaluate(STUB) is True
    assert app.evaluate("() => appHostFollow()") is True


def send(app, pos, playing):
    app.evaluate("([pos, playing]) => B.hostTransport.send({ sid: S.sid, epoch: (B.hostTransport._ep = (B.hostTransport._ep || 0) + 1), t_ns: performance.now() * 1e6, pos, playing })", [pos, playing])


@pytest.fixture(autouse=True)
def canvas_2d_lanes(app):
    """As in test_playback.py: these tests measure the page and the calls, not the WebGL2 lanes."""
    assert app.evaluate("() => setLanesGL(false)") is False
    yield
    app.evaluate("() => setLanesGL(true)")
    app.evaluate("() => { window.__DS_TEST_RAF_DELAY_MS = 0; }")


def wait_for_time_display(app, prefix):
    """#540: the frame draw writes #timeDisplay one frame after the state (drawHead, render.js).
    A test that asserts the text waits for the text — a wait that is false before the action —
    instead of reading it right after a wait on the state alone."""
    app.wait_for_function(
        "(p) => document.querySelector('#timeDisplay').textContent.startsWith(p)", arg=prefix
    )


def test_the_page_follows_the_host_playing_with_no_play_in_the_page(app, make_project):
    open_project(app, make_project(seconds=30.0))
    set_host_mode(app)
    # the listener is the page's own: bound by the appHostFollow() call above, as a shell does it
    assert app.evaluate("() => !!(B.hostTransport && B.hostTransport.onPosition)") is True
    send(app, 3.0, True)
    app.wait_for_function("() => S.playing === true")
    assert app.evaluate("() => document.querySelector('#playBtn').textContent") == "⏸"
    # the playhead and the position line follow the positions the host sends
    app.wait_for_function("() => PH.v > 3.0 && PH.v < 4.0")
    wait_for_time_display(app, "3.")
    assert app.locator("#timeDisplay").inner_text().startswith("3."), app.locator("#timeDisplay").inner_text()
    send(app, 4.0, True)
    app.wait_for_function("() => PH.v > 4.0 && PH.v < 5.0")
    wait_for_time_display(app, "4.")
    assert app.locator("#timeDisplay").inner_text().startswith("4."), app.locator("#timeDisplay").inner_text()
    # the host plays: the page asked the host for nothing
    assert app.evaluate("() => B.hostTransport.calls") == [], app.evaluate("() => B.hostTransport.calls")


def test_the_host_stopping_shows_the_stopped_state_at_its_position(app, make_project):
    open_project(app, make_project(seconds=30.0))
    set_host_mode(app)
    assert app.evaluate("() => !!(B.hostTransport && B.hostTransport.onPosition)") is True
    send(app, 3.0, True)
    app.wait_for_function("() => S.playing === true")
    n = app.evaluate("() => B.hostTransport.calls.length")
    send(app, 5.25, False)
    app.wait_for_function("() => S.playing === false")
    assert app.evaluate("() => document.querySelector('#playBtn').textContent") == "▶"
    assert app.evaluate("() => PH.v") == pytest.approx(5.25, abs=1e-9)
    wait_for_time_display(app, "5.2")
    assert app.locator("#timeDisplay").inner_text().startswith("5.2"), app.locator("#timeDisplay").inner_text()
    # the page asked the host for nothing: the DAW stopped, not the page
    assert app.evaluate("() => B.hostTransport.calls") == [], app.evaluate("() => B.hostTransport.calls")
    assert n == 0


def test_a_locate_in_the_daw_moves_the_playhead_while_stopped(app, make_project):
    open_project(app, make_project(seconds=30.0))
    set_host_mode(app)
    assert app.evaluate("() => !!(B.hostTransport && B.hostTransport.onPosition)") is True
    send(app, 2.0, False)   # the host is stopped at 2 s
    app.wait_for_function("() => !S.playing && Math.abs(PH.v - 2.0) < 1e-9")
    # a locate in the DAW: another stopped position moves the playhead and the position line
    send(app, 8.5, False)
    app.wait_for_function("() => Math.abs(PH.v - 8.5) < 1e-9")
    wait_for_time_display(app, "8.5")
    assert app.locator("#timeDisplay").inner_text().startswith("8.5"), app.locator("#timeDisplay").inner_text()
    # and the page asked the host for nothing (no stop, no locate, no play)
    assert app.evaluate("() => B.hostTransport.calls") == [], app.evaluate("() => B.hostTransport.calls")


def test_the_time_display_wait_survives_a_delayed_frame_draw(app, make_project):
    """#540 fault run, kept: with the frame draw held back (the render.js test switch), the state
    is true one frame before the text. The old test (wait on PH.v, then read the text at once)
    fails here; the wait on the text passes."""
    open_project(app, make_project(seconds=30.0))
    set_host_mode(app)
    assert app.evaluate("() => !!(B.hostTransport && B.hostTransport.onPosition)") is True
    send(app, 2.0, False)
    app.wait_for_function("() => !S.playing && Math.abs(PH.v - 2.0) < 1e-9")
    assert app.evaluate("() => { window.__DS_TEST_RAF_DELAY_MS = 300; return window.__DS_TEST_RAF_DELAY_MS; }") == 300
    send(app, 8.5, False)
    app.wait_for_function("() => Math.abs(PH.v - 8.5) < 1e-9")
    wait_for_time_display(app, "8.5")
    assert app.locator("#timeDisplay").inner_text().startswith("8.5"), app.locator("#timeDisplay").inner_text()


def test_no_following_when_host_mode_is_off(app, make_project):
    """The standalone rule: with the mode off the page binds no host listener at all.

    A shell only sets window.__DS_HOST_MODE__ and B.hostTransport when the host is
    there; the boot calls appHostFollow() once and only when hostMode() is true. A
    transport object that appears later with the mode off gets no onPosition, and a
    position message through it changes nothing. This test fails on 0966eade, where
    a setInterval re-ran appHostFollow every 250 ms regardless of the mode."""
    open_project(app, make_project(seconds=30.0))
    # the plain page: the mode flag is absent, exactly as the standalone app boots
    assert app.evaluate("() => B.hostMode") is False
    # a transport injected after the boot while the mode is off: no listener is bound
    app.evaluate(
        "() => (B.hostTransport = { send: function (m) { if (this.onPosition) this.onPosition(m); } }, true)"
    )
    # outwait the old head's 250 ms re-registration timer: even after it, nothing is bound
    app.wait_for_timeout(400)
    assert app.evaluate("() => !!(B.hostTransport && B.hostTransport.onPosition)") is False
    # a position message through it: nothing changes (playhead and Play button stay)
    app.evaluate(
        "() => B.hostTransport.send({ sid: S.sid, epoch: 1, t_ns: performance.now() * 1e6, pos: 9.0, playing: false })"
    )
    assert app.evaluate("() => S.playing") is False
    assert app.evaluate("() => PH.v") == pytest.approx(0.0, abs=1e-9)
    assert app.evaluate("() => document.querySelector('#playBtn').textContent") == "▶"


def send_ep(app, pos, playing, epoch):
    """Like send(), with a fixed epoch: a host that locates keeps its epoch (no epoch change helps the page)."""
    app.evaluate("([pos, playing, epoch]) => B.hostTransport.send({ sid: S.sid, epoch, t_ns: performance.now() * 1e6, pos, playing })", [pos, playing, epoch])


def assert_playing_at(app, lo):
    app.wait_for_function("() => S.playing === true")
    assert app.evaluate("() => document.querySelector('#playBtn').textContent") == "⏸"
    app.wait_for_function("(lo) => PH.v > lo && PH.v < lo + 1.0", arg=lo)   # the playhead runs from the host position


def test_the_page_follows_every_start_and_stop_of_the_host(app, make_project):
    """#549: play, stop, play, stop, play (a locate between two of them). On main the second play only
    slews: AC.active stays true after the first stop, so the playing state never shows again."""
    open_project(app, make_project(seconds=30.0))
    set_host_mode(app)
    send(app, 3.0, True)
    assert_playing_at(app, 3.0)
    send(app, 5.25, False)
    app.wait_for_function("() => S.playing === false")
    assert app.evaluate("() => document.querySelector('#playBtn').textContent") == "▶"
    assert app.evaluate("() => PH.v") == pytest.approx(5.25, abs=1e-9)
    send(app, 7.0, True)   # the second play: this is the step that fails on main
    assert_playing_at(app, 7.0)
    send(app, 9.5, False)
    app.wait_for_function("() => S.playing === false")
    assert app.evaluate("() => PH.v") == pytest.approx(9.5, abs=1e-9)
    send(app, 12.0, False)   # a locate while stopped
    app.wait_for_function("() => Math.abs(PH.v - 12.0) < 1e-9")
    assert app.evaluate("() => S.playing") is False
    send(app, 14.0, True)   # the third play, after a stopped locate
    assert_playing_at(app, 14.0)
    send(app, 15.0, False)
    app.wait_for_function("() => S.playing === false && Math.abs(PH.v - 15.0) < 1e-9")
    assert app.evaluate("() => document.querySelector('#playBtn').textContent") == "▶"
    assert app.evaluate("() => B.hostTransport.calls") == []


def test_a_locate_while_the_host_plays_after_a_stop_re_anchors_the_playhead(app, make_project):
    open_project(app, make_project(seconds=30.0))
    set_host_mode(app)
    send_ep(app, 2.0, True, 1)
    assert_playing_at(app, 2.0)
    send_ep(app, 4.0, False, 1)
    app.wait_for_function("() => S.playing === false")
    send_ep(app, 6.0, True, 1)
    assert_playing_at(app, 6.0)
    send_ep(app, 20.0, True, 1)   # a locate while playing, same epoch: a hard re-anchor, no slew from 6 s
    app.wait_for_function("() => PH.v > 20.0 && PH.v < 21.0", timeout=1500)


def test_the_shell_workaround_before_a_play_message_does_not_double_start(app, make_project):
    """The plugin shell sets AC.active = false when the host starts to play: the fixed page must still
    show playing one time, with no second start and no stopped flicker."""
    open_project(app, make_project(seconds=30.0))
    set_host_mode(app)
    send(app, 3.0, True)
    assert_playing_at(app, 3.0)
    send(app, 5.0, False)
    app.wait_for_function("() => S.playing === false")
    app.evaluate("() => { AC.active = false; }")   # the workaround, right before the play message
    send(app, 8.0, True)
    assert_playing_at(app, 8.0)
    assert app.evaluate("() => AC.active") is True
    assert app.evaluate("() => document.querySelector('#playBtn').textContent") == "⏸"
    send(app, 9.0, True)   # the next playing message only follows: still playing, no re-start
    assert app.evaluate("() => S.playing") is True
    assert app.evaluate("() => B.hostTransport.calls") == []
    # the workaround while the host already plays (a second message after it) changes nothing visible
    app.evaluate("() => { AC.active = false; }")
    send(app, 9.5, True)
    assert app.evaluate("() => S.playing") is True
    assert app.evaluate("() => document.querySelector('#playBtn').textContent") == "⏸"


def test_a_play_of_the_page_itself_after_a_stop_still_works_with_the_mode_off(app, make_project):
    """Standalone rule: with the mode off the page's own play, pause, play works as before."""
    open_project(app, make_project(seconds=30.0))
    assert app.evaluate("() => B.hostMode") is False
    app.evaluate("() => appPlay()")
    app.wait_for_function("() => S.playing === true && AC.active === true")
    app.evaluate("() => appPause()")
    app.wait_for_function("() => S.playing === false")
    assert app.evaluate("() => AC.active") is False
    app.evaluate("() => appPlay()")
    app.wait_for_function("() => S.playing === true && AC.active === true")
    assert app.evaluate("() => document.querySelector('#playBtn').textContent") == "⏸"
    app.evaluate("() => appPause()")
    app.wait_for_function("() => S.playing === false")
