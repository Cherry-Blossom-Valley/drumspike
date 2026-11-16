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
