"""P3-20b: half-speed playback (Pro Tools Shift+Space) in the page on the native transport, through the
bridge with the null device (silent: DRUMSPIKE_AUDIO=null, DRUMSPIKE_MASTER_GAIN=0). Shift+Space plays from
the playhead at half speed, one octave down; Space or Shift+Space stops. The page extrapolates the playhead
with the speed of the messages, so the drawn playhead runs at half the rate. The audio itself (500 Hz from
1 kHz, twice the length, the clicks at twice the interval) is proved by the Rust tests of the core
(crates/core/src/transport/mod.rs, `transport_half_speed_*`)."""
import pytest

from app_support import open_project


@pytest.fixture(autouse=True)
def canvas_2d_lanes(app):
    """As in test_playback.py: these tests measure the playhead, not the WebGL2 lanes (SwiftShader is slow)."""
    assert app.evaluate("() => setLanesGL(false)") is False
    yield
    app.evaluate("() => setLanesGL(true)")


# The rate of the drawn playhead: the median of the frame-to-frame rates of S.playhead over `seconds`. The
# page extrapolates between two messages (4 Hz), so the median is the extrapolation's own rate; the first and
# the last frame alone would hide a wrong rate (each message puts the playhead back).
DRAWN_RATE = """async seconds => {
  const frames = [];
  await new Promise(done => {
    const end = performance.now() + seconds * 1000;
    const loop = t => { frames.push([performance.now(), S.playhead]); if (performance.now() < end) requestAnimationFrame(loop); else done(); };
    requestAnimationFrame(loop);
  });
  const rates = [];
  for (let i = 1; i < frames.length; i++) {
    const dt = (frames[i][0] - frames[i - 1][0]) / 1000;
    if (dt > 0.004) rates.push((frames[i][1] - frames[i - 1][1]) / dt);
  }
  rates.sort((x, y) => x - y);
  return rates[rates.length >> 1];
}"""


def status(page):
    return page.evaluate("() => B.transportStatus()")


def transport_rate(page, ms=800):
    a = status(page)
    page.wait_for_timeout(ms)
    b = status(page)
    return (b["pos"] - a["pos"]) / ((b["t_ns"] - a["t_ns"]) / 1e9)


def wait_playing(page, speed, ms=10000):
    page.wait_for_function("() => S.playing === true && AC.running === true", timeout=ms)
    page.wait_for_function(f"() => AC.speed === {speed}", timeout=ms)


def wait_stopped(page):
    page.wait_for_function("() => S.playing === false")


def half_shown(page):
    return page.evaluate("() => !$('#stSpeed').classList.contains('hidden') && $('#stSpeed').textContent")


def test_shift_space_plays_at_half_speed_and_space_or_shift_space_stops(app, make_project):
    open_project(app, make_project(seconds=6.0))
    app.evaluate("() => { S.playhead = 0.3; }")
    app.keyboard.press("Shift+Space")
    wait_playing(app, 0.5)
    st = status(app)
    assert st["playing"] is True and st["speed"] == 0.5 and st["master"] == 0.0, st
    assert half_shown(app) == "half speed"
    assert app.evaluate("() => S.playStartPos") == 0.3, "the play starts at the playhead"
    drawn = app.evaluate(DRAWN_RATE, 1.2)
    heard = transport_rate(app)
    print(f"\nHALF SPEED drawn playhead {drawn:.3f} s per s, transport {heard:.3f} s per s")
    assert abs(drawn - 0.5) < 0.08, drawn
    assert abs(heard - 0.5) < 0.06, heard
    # the playhead messages carry the speed
    assert app.evaluate("() => AC.last && AC.last.speed") == 0.5
    # Shift+Space stops
    app.keyboard.press("Shift+Space")
    wait_stopped(app)
    assert half_shown(app) is False, "the status line still says half speed"
    p = app.evaluate("() => S.playhead")
    assert 0.5 < p < 1.8, p
    # Space plays at normal speed from there, in the same transport (no reopen), and stops
    app.keyboard.press("Space")
    wait_playing(app, 1)
    assert half_shown(app) is False
    st = status(app)
    assert st["speed"] == 1.0 and st["reopens"] == 0, st
    normal = app.evaluate(DRAWN_RATE, 1.0)
    assert abs(normal - 1.0) < 0.12, normal
    # Shift+Space also stops a normal-speed play
    app.keyboard.press("Shift+Space")
    wait_stopped(app)
    # and Space stops a half-speed play
    app.keyboard.press("Shift+Space")
    wait_playing(app, 0.5)
    assert half_shown(app) == "half speed"
    app.keyboard.press("Space")
    wait_stopped(app)
    assert half_shown(app) is False


def test_half_speed_is_in_the_key_table_menu_and_help_and_a_text_field_keeps_shift_space(app, make_project):
    open_project(app, make_project())
    row = app.evaluate("() => DSKeys.COMMANDS.find(c => c.id === 'half_speed')")
    assert row["menu"] == "half_speed" and row["path"] == "Transport" and row["title"] == "Half-Speed Play"
    assert row["page_keys"] == ["Shift+Space", "Option+Shift+Space"] and row["cubase"] == [] and row["menu_key"] is None
    assert app.evaluate("() => DSKeys.problems(DSKeys.COMMANDS).length") == 0
    # the Cubase preset has no key for it
    assert app.evaluate("() => DSKeys.keysOf(DSKeys.COMMANDS.find(c => c.id === 'half_speed'), 'cubase').length") == 0
    # the menu item (the shell handler) plays and stops like the key
    app.evaluate("() => DSShell.handlers.half_speed()")
    wait_playing(app, 0.5)
    app.evaluate("() => DSShell.handlers.half_speed()")
    wait_stopped(app)
    # a text field keeps the key
    app.evaluate("() => { const i = document.createElement('input'); i.id = 'hsField'; document.body.appendChild(i); }")
    app.focus("#hsField")
    app.keyboard.press("Shift+Space")
    app.wait_for_timeout(300)
    assert app.evaluate("() => S.playing") is False
    assert app.evaluate("() => document.querySelector('#hsField').value") == " "
    app.evaluate("() => document.querySelector('#hsField').remove()")
