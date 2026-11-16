"""#509: host mode of the page (DAW plugin P2).

In a DAW the host owns the files, the transport and the tempo. The mode is one flag: the
plugin shell sets window.__DS_HOST_MODE__ before the page loads, backend.js turns it into
B.hostMode, and state.js hostMode() is the one place that reads it. The tests here set the
flag at run time (every read is at call time; the key table cache is keyed by the mode),
which is the same switch the plugin shell flips before load.

The stub is what the plugin window (S2) will be: B.hostTransport with play, stop, locate and
tempoMap that record their calls, send(m) pushing a playhead message like the app's channel
does, and onPosition, which the page registers on play. With the mode off (the desktop app
never sets the flag) nothing changes: the last test screenshots the editor with the flag off
and with the flag absent and compares the pixels.

On main these tests fail: there is no hostMode/hostOwned, no host branch in the transport and
the tempo map, and the menus and the keys run (the red proof is in the PR)."""
import re
from pathlib import Path

import pytest

from app_support import menu, open_project

ROOT = Path(__file__).resolve().parents[2]
SR = 48000

# The commands the host owns: open, create, add and relink files, and the tempo (gate 1 and 3).
ABSENT = ["new", "open", "open_recent", "recent_clear", "close", "save", "save_as", "add_tracks",
          "import_old", "consolidate", "import_tempo_map", "detect_tempo"]

STUB = """() => {
  B.hostMode = true;
  B.hostTransport = {
    calls: [],
    play: function (position, speed) { this.calls.push(["play", position, speed === undefined ? null : speed]); return Promise.resolve(); },
    stop: function (position) { this.calls.push(["stop", position]); return Promise.resolve(); },
    locate: function (position) { this.calls.push(["locate", position]); return Promise.resolve(); },
    tempoMap: function () { this.calls.push(["tempoMap"]);
      return Promise.resolve({ bpm: 101, steps: [], meter: [4, 4], changes: [], bar1_s: 0 }); },
    send: function (m) { if (this.onPosition) this.onPosition(m); },
  };
  return B.hostMode;
}"""


def set_host_mode(app):
    assert app.evaluate(STUB) is True


@pytest.fixture(autouse=True)
def canvas_2d_lanes(app):
    """As in test_playback.py: these tests measure the page and the calls, not the WebGL2 lanes."""
    assert app.evaluate("() => setLanesGL(false)") is False
    yield
    app.evaluate("() => setLanesGL(true)")


def test_the_host_owned_commands_are_absent_from_the_menus_and_the_keys(app, make_project):
    open_project(app, make_project())
    # control, mode off: the key is in the table (Cmd+W closes the project)
    assert app.evaluate("() => { const hit = DSKeys.find({ key: 'w', code: 'KeyW', metaKey: true }); return hit ? hit.cmd.id : null; }") == "close"
    set_host_mode(app)
    for id in ABSENT:
        assert menu(app, id) is False, id   # absent from the menus
    assert app.evaluate("() => { const hit = DSKeys.find({ key: 'w', code: 'KeyW', metaKey: true }); return hit ? hit.cmd.id : null; }") is None
    # the project was never closed and nothing was created: the page is still the host's guest
    assert app.evaluate("() => !!S.sid") is True
    # every element id of the page still exists with the mode on
    ids = re.findall(r'id="([\w-]+)"', (ROOT / "static" / "index.html").read_text())
    missing = app.evaluate("ids => ids.filter(i => !document.getElementById(i))", ids)
    assert missing == [], missing


def test_play_stop_and_locate_call_the_host_transport_and_never_the_app_transport(app, make_project):
    open_project(app, make_project(seconds=30.0))
    set_host_mode(app)
    app.evaluate("() => { S.playhead = 1.25; updateStatus(); }")
    app.click("#playBtn")
    app.wait_for_function("() => S.playing === true")
    calls = app.evaluate("() => B.hostTransport.calls")
    assert ["play", 1.25, None] in calls, calls
    n0 = len(app.invokes)   # the page load and the open may have called the app transport; the play must not
    for cmd in ("transport_play", "transport_stop", "transport_seek", "transport_mix", "transport_clicks", "transport_status"):
        assert cmd not in app.invokes[n0:], cmd
    # the position line follows a position that the stub sends
    app.evaluate("() => B.hostTransport.send({ sid: S.sid, epoch: 1, t_ns: performance.now() * 1e6, pos: 3.25, playing: true })")
    app.wait_for_function("() => S.playhead > 3.2 && S.playhead < 4.2")
    # the text is written in the frame draw, one frame after the state (#540): wait for the text
    app.wait_for_function("() => document.querySelector('#timeDisplay').textContent.startsWith('3.')")
    assert app.locator("#timeDisplay").inner_text().startswith("3."), app.locator("#timeDisplay").inner_text()
    # locate: a moved playhead seeks the host, never the app transport
    app.evaluate("() => { S.playhead = 5.5; }")
    app.wait_for_function("() => B.hostTransport.calls.some(c => c[0] === 'locate' && Math.abs(c[1] - 5.5) < 1e-9)")
    assert "transport_seek" not in app.invokes[n0:]
    # stop goes to the host too
    app.click("#playBtn")
    app.wait_for_function("() => S.playing === false")
    assert app.evaluate("() => B.hostTransport.calls.some(c => c[0] === 'stop')")
    assert "transport_stop" not in app.invokes[n0:]


def test_the_tempo_map_comes_from_the_host_and_detect_and_import_are_absent(app, make_project):
    open_project(app, make_project())
    set_host_mode(app)
    app.evaluate("async () => { await tmLoad(); }")
    app.wait_for_function("() => TM.map && TM.map.bpm === 101")
    assert app.evaluate("() => TM.isDefault") is False
    n0 = len(app.invokes)   # the load before the mode was on may have asked the service
    app.evaluate("async () => { await tmLoad(); }")   # again, now in host mode
    app.wait_for_function("() => TM.map && TM.map.bpm === 101")
    assert "tempo_map_get" not in app.invokes[n0:]
    assert menu(app, "detect_tempo") is False
    assert menu(app, "import_tempo_map") is False
    assert app.locator("#tempoDetectBtn").is_hidden()   # the button is gone; the id stays


def test_the_file_flows_say_the_host_owns_the_files(app, make_project):
    open_project(app, make_project())
    set_host_mode(app)
    app.evaluate("() => { setMsg(''); fileNew(); }")
    assert app.locator("#stMsg").inner_text() == "the host owns the files"
    app.evaluate("() => { setMsg(''); fileOpen(); }")
    assert app.locator("#stMsg").inner_text() == "the host owns the files"
    app.evaluate("() => { setMsg(''); addTracksFlow(['/x/a.wav']); }")
    assert app.locator("#stMsg").inner_text() == "the host owns the files"
    app.evaluate("() => { setMsg(''); detectTempo(); }")
    assert app.locator("#stMsg").inner_text() == "the host owns the tempo"


def test_with_the_mode_off_the_editor_is_pixel_identical(app, make_project):
    """The desktop app never sets the flag: the mode switch itself must not touch the page.
    The screenshot with the flag off and the one with the flag absent are the same pixels."""
    open_project(app, make_project(seconds=12.0))
    app.wait_for_function("() => S.loaded > 0 && !FR.raf && FR.flags === 0")
    off = app.locator("#editor").screenshot()
    assert app.evaluate("() => B.hostMode") is False   # the flag exists and is off
    app.evaluate("() => { delete B.hostMode; delete B.hostTransport; }")   # the flag absent, as on the plain page
    app.wait_for_function("() => !FR.raf && FR.flags === 0")
    absent = app.locator("#editor").screenshot()
    assert off == absent, "the mode switch changed the page while the mode is off"
