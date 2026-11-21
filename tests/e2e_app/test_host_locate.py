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
    assert all(g >= 38 for g in gaps), gaps   # 40 ms apart; the clock of performance.now() and the timer round to a ms or two


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


# ---- round 2 (U1 to U7) ----

def test_a_reload_clamp_is_no_move_and_a_move_before_a_project_change_is_not_sent(app, make_project):
    """U1: the clamp of loadProject sends no locate; a waiting locate of the old project is dropped."""
    host_page(app, make_project)
    app.evaluate("() => { PH.v = 99; }")   # beyond the song, no user write: the load clamps it
    app.evaluate("async () => { await loadProject(); }")
    app.wait_for_timeout(200)
    assert locates(app) == [], locates(app)
    app.evaluate("() => { S.playhead = 5; S.playhead = 6; }")   # the first goes out, the second waits in the timer
    app.evaluate("() => { window.__sid = S.sid; S.sid = 'another-project'; }")
    app.wait_for_timeout(200)
    app.evaluate("() => { S.sid = window.__sid; }")
    assert [round(c[0], 3) for c in locates(app)] == [5.0], locates(app)


def test_a_refusal_of_the_old_project_does_not_move_the_new_one(app, make_project):
    host_page(app, make_project)
    app.evaluate("""() => { B.hostTransport.locate = function (p) { return new Promise((_, rej) => setTimeout(() => rej(new Error('late refusal')), 150)); };
      S.playhead = 9.0; window.__sid = S.sid; S.sid = 'another-project'; }""")
    app.wait_for_timeout(400)
    app.evaluate("() => { S.sid = window.__sid; }")
    assert abs(app.evaluate("() => PH.v") - 9.0) < 1e-9   # not put back by a refusal of another project


def test_a_move_while_the_host_starts_waits_and_the_playing_page_seeks(app, make_project):
    """U2: during AC.starting no locate goes out; after the start the tick seeks (the path while playing), once."""
    host_page(app, make_project)
    app.evaluate("() => { B.hostTransport.play = function (p) { this.calls.push(['play', p]); return new Promise(r => { window.__playDone = r; }); }; }")
    app.click("#playBtn")
    app.wait_for_function("() => AC.starting === true")
    app.evaluate("() => { S.playhead = 9.0; }")
    app.wait_for_timeout(250)
    assert locates(app) == [], locates(app)
    app.evaluate("() => window.__playDone()")
    app.wait_for_function("() => B.hostTransport.calls.some(c => c[0] === 'locate' && Math.abs(c[1] - 9.0) < 1e-9)")
    app.wait_for_timeout(200)
    assert len(locates(app)) == 1, locates(app)


def test_a_ctrl_click_and_a_context_menu_do_not_hold_the_playhead(app, make_project):
    """U3: no mouseup after ctrl+click: the refusal still rolls back."""
    host_page(app, make_project)
    app.evaluate("""() => { B.hostTransport.refuse = 'refused'; window.dispatchEvent(new MouseEvent('mousedown', { button: 0, ctrlKey: true, bubbles: true })); S.playhead = 9.0; }""")
    app.wait_for_function("() => Math.abs(PH.v - 2.0) < 1e-9")
    app.evaluate("""() => { window.dispatchEvent(new MouseEvent('mousedown', { button: 0, bubbles: true })); S.playhead = 8.0; }""")
    app.wait_for_timeout(200)
    assert abs(app.evaluate("() => PH.v") - 8.0) < 1e-9   # the pointer is down
    app.evaluate("() => window.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }))")
    app.wait_for_function("() => Math.abs(PH.v - 2.0) < 1e-9")


def test_a_refusal_before_any_host_position_goes_back_to_the_position_before_the_move(app, make_project):
    """U4: the host sent no position yet."""
    open_project(app, make_project(seconds=30.0))
    assert app.evaluate(STUB) is True
    assert app.evaluate("() => appHostFollow()") is True
    before = app.evaluate("() => PH.v")
    app.evaluate("() => { B.hostTransport.refuse = 'refused'; S.playhead = 9.0; }")
    app.wait_for_function("(b) => Math.abs(PH.v - b) < 1e-9", arg=before)


def test_after_a_refused_jump_the_playhead_is_in_view_again(app, make_project):
    """U5: centerOn moved the view to the jump; the roll-back brings the playhead into sight."""
    host_page(app, make_project)
    app.evaluate("() => { setView(0, 5); B.hostTransport.refuse = 'refused'; S.playhead = 20; centerOn(20); }")
    app.wait_for_function("() => Math.abs(PH.v - 2.0) < 1e-9")
    app.wait_for_function("() => S.view.t0 <= 2.0 && S.view.t1 >= 2.0")


def test_the_end_of_the_song_in_host_mode_stops_then_locates_zero_once(app, make_project):
    """U7: tick() ends the song with pause(); S.playhead = 0. Order at the host: stop, then locate 0, once."""
    host_page(app, make_project)
    app.evaluate("() => { B.hostTransport.calls.length = 0; B.hostTransport.send({ sid: S.sid, epoch: 2, t_ns: performance.now() * 1e6, pos: maxDur() - 0.1, playing: true }); }")
    app.wait_for_function("() => B.hostTransport.calls.some(c => c[0] === 'locate')")
    app.wait_for_timeout(300)
    kinds = app.evaluate("() => B.hostTransport.calls.map(c => c[0] === 'locate' ? ['locate', c[1]] : [c[0]])")
    assert kinds == [["stop"], ["locate", 0]], kinds


def test_a_late_timer_does_not_send_a_second_locate_right_after_a_direct_one(app, make_project):
    """Round 3: the event loop is blocked for more than 40 ms between two moves (the timer is late); the next
    move takes the direct path. A third move at once must wait the full 40 ms: no stale timer sends it early."""
    host_page(app, make_project)
    app.evaluate("""async () => {
      const spin = ms => { const t = performance.now(); while (performance.now() - t < ms) {} };
      S.playhead = 5.0;                                  // direct send, sentAt = now
      await new Promise(r => setTimeout(r, 1));
      S.playhead = 5.1;                                  // waits: the timer is armed for the rest of the 40 ms
      spin(60);                                          // the timer is due but cannot run
      S.playhead = 5.2;                                  // past sentAt + 40: the direct path sends
      S.playhead = 5.3;                                  // must wait 40 ms again
      await new Promise(r => setTimeout(r, 200));
    }""")
    calls = locates(app)
    assert [round(c[0], 3) for c in calls] == [5.0, 5.2, 5.3], calls
    assert calls[2][1] - calls[1][1] >= 38, calls
