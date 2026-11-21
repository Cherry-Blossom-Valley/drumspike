"""#556: in host mode the page calls B.hostTransport.locate itself when the user moves the playhead while stopped.

Before, the plugin shell watched PH.user every 25 ms and sent the locate. Now the page does it: at most
about 25 calls a second with the last position always sent, and a refused locate (the promise rejects with
the text) shows the text and puts the playhead back on the last position the host sent. A drag keeps the
playhead under the pointer. With host mode off nothing of this exists (last test).

On main these tests fail: a stopped page calls no locate at all."""
import pytest

from app_support import open_project

STUB = """() => {
  B.hostMode = true;
  B.hostTransport = {
    calls: [], refuse: null,
    play: function (position, speed) { this.calls.push(["play", position]); return Promise.resolve(); },
    stop: function (position) { this.calls.push(["stop", position]); return Promise.resolve(); },
    locate: function (position) { this.calls.push(["locate", position, performance.now()]);
      return this.refuse ? Promise.reject(new Error(this.refuse)) : Promise.resolve(); },
    send: function (m) { if (this.onPosition) this.onPosition(m); },
  };
  return B.hostMode;
}"""


@pytest.fixture(autouse=True)
def canvas_2d_lanes(app):
    assert app.evaluate("() => setLanesGL(false)") is False
    yield
    app.evaluate("() => setLanesGL(true)")


def host_page(app, make_project):
    open_project(app, make_project(seconds=30.0))
    assert app.evaluate(STUB) is True
    assert app.evaluate("() => appHostFollow()") is True
    app.evaluate("() => B.hostTransport.send({ sid: S.sid, epoch: 1, t_ns: performance.now() * 1e6, pos: 2.0, playing: false })")
    app.wait_for_function("() => Math.abs(PH.v - 2.0) < 1e-9")


def locates(app):
    return app.evaluate("() => B.hostTransport.calls.filter(c => c[0] === 'locate').map(c => [c[1], c[2]])")


def test_a_move_of_the_stopped_page_calls_locate_and_a_burst_is_rate_limited_with_the_last_position(app, make_project):
    host_page(app, make_project)
    app.evaluate("() => { S.playhead = 5.5; }")
    app.wait_for_function("() => B.hostTransport.calls.some(c => c[0] === 'locate' && Math.abs(c[1] - 5.5) < 1e-9)")
    app.evaluate("""() => { window.__burst = new Promise(res => { let i = 0;
      const id = setInterval(() => { S.playhead = 6 + (++i) * 0.01; if (i >= 40) { clearInterval(id); res(); } }, 5); }); }""")
    app.evaluate("() => window.__burst")
    app.wait_for_function("() => B.hostTransport.calls.some(c => c[0] === 'locate' && Math.abs(c[1] - 6.4) < 1e-9)")
    calls = locates(app)
    assert 2 <= len(calls) < 25, len(calls)   # 40 moves in 200 ms: far fewer calls
    assert abs(calls[-1][0] - 6.4) < 1e-9, calls[-1]   # the last position is always sent
    gaps = [b[1] - a[1] for a, b in zip(calls[1:], calls[2:])]
    assert all(g >= 35 for g in gaps), gaps   # about 25 a second at most


def test_a_write_that_is_no_user_move_sends_no_locate(app, make_project):
    host_page(app, make_project)
    app.evaluate("() => { S.playhead = PH.v; S.playhead = Math.min(S.playhead, maxDur()); }")   # the same value (loadProject)
    app.evaluate("() => B.hostTransport.send({ sid: S.sid, epoch: 2, t_ns: performance.now() * 1e6, pos: 4.0, playing: false })")   # the host's own move
    app.wait_for_timeout(200)
    assert locates(app) == [], locates(app)


def test_a_refused_locate_shows_the_text_and_goes_back_to_the_host_position(app, make_project):
    host_page(app, make_project)
    app.evaluate("() => { B.hostTransport.refuse = 'the host gives plugins no control of locate'; S.playhead = 9.0; }")
    app.wait_for_function("() => Math.abs(PH.v - 2.0) < 1e-9")   # back to the last position of the host
    assert "no control of locate" in app.locator("#stMsg").inner_text()


def test_a_refusal_in_a_drag_waits_for_the_pointer(app, make_project):
    host_page(app, make_project)
    app.evaluate("""() => { B.hostTransport.refuse = 'refused'; window.dispatchEvent(new MouseEvent('mousedown', { button: 0, bubbles: true })); S.playhead = 9.0; }""")
    app.wait_for_function("() => B.hostTransport.calls.some(c => c[0] === 'locate')")
    app.wait_for_timeout(300)
    assert abs(app.evaluate("() => PH.v") - 9.0) < 1e-9   # under the pointer
    app.evaluate("() => window.dispatchEvent(new MouseEvent('mouseup', { button: 0, bubbles: true }))")
    app.wait_for_function("() => Math.abs(PH.v - 2.0) < 1e-9")


def test_the_own_stop_of_the_page_sends_no_locate(app, make_project):
    host_page(app, make_project)
    app.evaluate("() => B.hostTransport.send({ sid: S.sid, epoch: 2, t_ns: performance.now() * 1e6, pos: 3.0, playing: true })")
    app.wait_for_function("() => S.playing === true")
    app.click("#playBtn")
    app.wait_for_function("() => S.playing === false")
    app.wait_for_timeout(200)
    assert locates(app) == [], locates(app)


def test_with_host_mode_off_the_page_is_unchanged(app, make_project):
    open_project(app, make_project(seconds=30.0))
    assert app.evaluate("() => appHostFollow()") is False
    assert app.evaluate("() => !HL.on && !Object.getOwnPropertyDescriptor(PH, 'user').get") is True
    app.evaluate("() => { B.hostTransport = { locate: function (p) { window.__loc = p; return Promise.resolve(); } }; S.playhead = 7.0; }")
    app.wait_for_timeout(200)
    assert app.evaluate("() => window.__loc === undefined") is True
