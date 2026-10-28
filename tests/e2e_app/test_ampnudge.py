"""P3-20a: the amplitude zoom of the wave lanes and the nudge value (Avid Pro Tools Shortcuts Guide 2025.10 = A1, p. 33, 34, 28).

View > Waveform Zoom (In Option+Cmd+], Out Option+Cmd+[, Reset Control+Option+Cmd+[; Option+A also resets) is a display gain of
the wave lanes, 1x to 64x: the lane pixels are read as the user sees them (WebGL2 and Canvas 2D), the peaks, hits and the
service's data do not change. Edit > Nudge Value (1, 10, 100 samples, 1 ms, 10 ms; Option+Cmd+numpad plus and minus step) is the
step of the nudge keys. Both are view settings (app settings amp_zoom and nudge_value, read at boot). The web page has no native
menu, so the menu keys run in the page here (the native accelerators are checked by the Rust tests and the self-test `shell`)."""
import json

import pytest
from playwright.sync_api import expect

from app_support import edits_done, invoke, menu, open_project, wait_until
from test_inspector import settings_json
from test_keypreset import choose, preset_home  # noqa: F401  (the fixture and the Settings sheet helper)
from test_menus import empty_page, select_track, with_hits
from test_render import SETTLE

SR = 48000
LANE = ("Kick In", "kick", 36)


def js(page, expr, arg=None):
    return page.evaluate(expr, arg) if arg is not None else page.evaluate(expr)


# the drawn pixels of a lane (not the lane background) and the first and last row that has one, as the user sees the lane
LANE_ROWS = """async name => { const im = await laneImage(name), d = im.data, W = im.width, H = im.height; let n = 0, top = H, bot = -1;
  for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) { const i = (y * W + x) * 4;
    if (Math.abs(d[i] - 16) + Math.abs(d[i+1] - 19) + Math.abs(d[i+2] - 24) > 24) { n++; top = Math.min(top, y); bot = Math.max(bot, y); } }
  return { n, top, bot, H }; }"""


def lane(page, name="Kick In"):
    """The lane pixels once they stop changing (the sample window of a short view arrives after the view)."""
    last = None
    for _ in range(40):
        page.evaluate(SETTLE)
        now = page.evaluate(LANE_ROWS, name)
        if now == last:
            return now
        last = now
    raise AssertionError(f"the lane does not settle: {last}")


def amp(page):
    return js(page, "() => S.ampZoom")


def label(page):
    return js(page, "() => { const e = document.querySelector('#ampLbl'); return [e.textContent, e.classList.contains('hidden')]; }")


@pytest.fixture
def project(app, make_project):
    open_project(app, make_project(tracks=(LANE, ("Snare Top", "snare", 38))))
    # P4-29: the bar and beat grid lines are lane pixels and would count into the wave extent
    # (a line at the top row made `top` 0); these tests measure the waveform, so the grid is off
    app.evaluate("() => { TM.ui.grid = false; tmApplyUi(true); }")
    app.evaluate("() => setView(0, maxDur())")
    yield app
    app.evaluate("() => { setAmpZoom(1); setNudgeValue('1smp'); }")        # the next test starts from the defaults


def test_the_amplitude_zoom_steps_from_1x_to_64x_and_the_lane_follows(project):
    app = project
    assert amp(app) == 1 and label(app)[1] is True                       # hidden at 1x
    base = lane(app)
    assert 0.03 * base["H"] < base["top"] and base["bot"] < 0.97 * base["H"]   # the auto-gain: the peak is at 90 % of the half height
    seen = [base["n"]]
    for want in (2, 4, 8, 16, 32, 64):
        app.keyboard.press("Alt+Meta+]")                                  # View > Waveform Zoom > In (the menu key, run by the page here)
        assert amp(app) == want
        assert label(app) == [f"Amp {want}x", False]
        seen.append(lane(app)["n"])
    assert seen[1] > seen[0] and seen[2] > seen[1]                         # the wave grows with the zoom, then it clips
    expect(app.locator("#stMsg")).to_have_text("amplitude zoom: 64x")
    app.keyboard.press("Alt+Meta+]")                                      # the largest: nothing changes, the message says so
    assert amp(app) == 64
    expect(app.locator("#stMsg")).to_have_text("amplitude zoom: 64x is the largest")
    for want in (32, 16, 8, 4, 2, 1):
        assert menu(app, "amp_out") is True                               # the menu item does the same as the key
        assert amp(app) == want
    assert label(app)[1] is True
    assert lane(app) == base                                              # 1x again: the same pixels as before
    app.keyboard.press("Alt+Meta+[")                                      # no zoom below 1x
    assert amp(app) == 1
    expect(app.locator("#stMsg")).to_have_text("amplitude zoom: 1x is the smallest")
    assert menu(app, "amp_in") is True and amp(app) == 2
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.ampZoom") == 2, msg="the menu state")
    # a text field keeps the keys
    app.evaluate("() => { const i = document.createElement('input'); i.type = 'text'; i.id = 'probe'; document.body.appendChild(i); i.focus(); }")
    app.keyboard.press("Alt+Meta+]")
    assert amp(app) == 2
    app.evaluate("() => document.querySelector('#probe').remove()")


def test_both_reset_keys_bring_the_amplitude_back_to_1x(project):
    app = project
    base = lane(app)
    for _ in range(3):
        app.keyboard.press("Alt+Meta+]")
    assert amp(app) == 8 and lane(app) != base
    app.keyboard.press("Control+Alt+Meta+[")                              # "default waveform height"
    assert amp(app) == 1 and label(app)[1] is True
    assert lane(app) == base
    for _ in range(2):
        app.keyboard.press("Alt+Meta+]")
    assert amp(app) == 4
    app.evaluate("() => setView(1, 2)")
    app.keyboard.press("Alt+a")                                           # Zoom to Project resets the amplitude zoom too
    assert amp(app) == 1 and js(app, "() => [S.view.t0, S.view.t1 - maxDur()]") == [0, 0]
    assert lane(app) == base
    app.keyboard.press("Alt+Meta+]")
    assert menu(app, "amp_reset") is True and amp(app) == 1               # the menu item
    app.keyboard.press("Alt+Meta+]")
    assert menu(app, "zoom_project") is True and amp(app) == 1            # View > Zoom to Project
    app.keyboard.press("Alt+Meta+]")
    app.keyboard.press("Control+Alt+Meta+[")
    assert amp(app) == 1                                                  # the reset key is not the Out key


@pytest.mark.parametrize("gl", [True, False])
def test_a_clipped_peak_draws_at_the_lane_edge_and_nothing_else_changes(project, gl):
    app = project
    app.evaluate("on => setLanesGL(on)", gl)
    before = js(app, "() => JSON.stringify([S.events, S.peakAmp, S.tracks, S.order, S.view])")
    peaks = js(app, "() => JSON.stringify(S.peakAmp)")
    wire = json.dumps(invoke(app, "tracks_get", sid=js(app, "() => S.sid")), sort_keys=True)
    base = lane(app)
    assert base["top"] > 0.03 * base["H"] and base["bot"] < 0.97 * base["H"]   # at 1x the wave stays inside the lane
    for _ in range(2):
        app.keyboard.press("Alt+Meta+]")                                  # 4x: the peaks are 3.6 half heights high
    zoomed = lane(app)
    assert zoomed["top"] == 0 and zoomed["bot"] == zoomed["H"] - 1         # the clipped peak draws at the lane edge, no further
    assert zoomed["H"] == base["H"]                                       # the lane keeps its size
    # a display gain only: the data of the page and of the service are the same
    assert js(app, "() => JSON.stringify([S.events, S.peakAmp, S.tracks, S.order, S.view])") == before
    assert js(app, "() => JSON.stringify(S.peakAmp)") == peaks
    assert json.dumps(invoke(app, "tracks_get", sid=js(app, "() => S.sid")), sort_keys=True) == wire
    assert js(app, "() => S.dirty") is False                              # no edit, nothing to save
    app.evaluate("() => setLanesGL(true)")


def test_the_nudge_value_sets_the_step_of_the_nudge_keys(make_project, app):
    open_project(app, make_project(seconds=12.0, tracks=(LANE,)))
    with_hits(app, js(app, "() => S.sid"), {"Kick In": [1.0, 2.0]})
    select_track(app, "Kick In")
    app.evaluate("() => { S.playhead = 0; S.sel = null; setView(0, 10); }")
    app.keyboard.press("Tab")
    t = lambda: (edits_done(app), round(js(app, "() => S.events['Kick In'][S.sel.index].t") * SR))[1]   # P4-9 stage 2: the stored hits are shown after the edit
    assert js(app, "() => S.nudgeValue") == "1smp"
    assert js(app, "() => document.querySelector('#nudgeSel').selectedOptions[0].textContent") == "Nudge 1 smp"
    at = SR
    for key, smp, text in (("1smp", 1, "Nudge 1 smp"), ("10smp", 10, "Nudge 10 smp"), ("100smp", 100, "Nudge 100 smp"),
                           ("1ms", 48, "Nudge 1 ms"), ("10ms", 480, "Nudge 10 ms")):
        assert menu(app, f"nudge_val_{key}") is True                      # Edit > Nudge Value > ... (a check item)
        assert js(app, "() => S.nudgeValue") == key
        assert js(app, "() => document.querySelector('#nudgeSel').selectedOptions[0].textContent") == text
        wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.nudgeValue") == key, msg="the radio item state")
        app.keyboard.press(".")                                           # later by the value
        at += smp
        assert t() == at, key
        app.keyboard.press("ArrowLeft")                                   # earlier by the value
        at -= smp
        assert t() == at, key
        app.keyboard.press("Shift+ArrowRight")                            # 10 x the value
        at += 10 * smp
        assert t() == at, key
        app.keyboard.press("Shift+ArrowLeft")
        at -= 10 * smp
        assert t() == at, key
        app.keyboard.press("NumpadAdd")
        app.keyboard.press(",")
        app.keyboard.press("NumpadSubtract")
        app.keyboard.press("ArrowRight")
        assert t() == at, key                                             # + and - cancel, comma and arrow cancel
    # the keys that step the value: Option+Cmd+numpad plus and minus; the ends stay
    assert js(app, "() => S.nudgeValue") == "10ms"
    app.keyboard.press("Alt+Meta+NumpadAdd")
    assert js(app, "() => S.nudgeValue") == "10ms"
    expect(app.locator("#stMsg")).to_have_text("nudge value: 10 ms is the largest")
    for want in ("1ms", "100smp", "10smp", "1smp"):
        app.keyboard.press("Alt+Meta+NumpadSubtract")
        assert js(app, "() => S.nudgeValue") == want
    app.keyboard.press("Alt+Meta+NumpadSubtract")
    assert js(app, "() => S.nudgeValue") == "1smp"
    expect(app.locator("#stMsg")).to_have_text("nudge value: 1 smp is the smallest")
    app.keyboard.press("Alt+Meta+NumpadAdd")
    assert js(app, "() => S.nudgeValue") == "10smp"
    app.keyboard.press("NumpadAdd")                                       # numpad plus is the nudge key, the modifiers are the step keys
    assert t() == at + 10
    app.keyboard.press("NumpadSubtract")
    # a click on the value opens the list (a select): the choice sets the value and the page keeps its keys
    app.select_option("#nudgeSel", "100smp")
    assert js(app, "() => S.nudgeValue") == "100smp"
    assert js(app, "() => document.activeElement.id") != "nudgeSel"
    app.keyboard.press(".")
    assert t() == at + 100
    # the help panel names the value
    menu(app, "shortcuts")
    expect(app.locator("#keymapTable tr[data-cmd='nudge_later']")).to_contain_text("now 100 smp")
    app.click("#helpDone")
    # no hit selected: the keys pan the view, whatever the value is
    app.keyboard.press("Escape")
    app.evaluate("() => setView(2, 7)")
    app.keyboard.press(".")
    assert js(app, "() => S.view.t0") == pytest.approx(3.0)
    # menu items work without a project (the value is a view setting)
    empty_page(app)
    assert menu(app, "nudge_val_1ms") is True and js(app, "() => S.nudgeValue") == "1ms"
    app.evaluate("() => setNudgeValue('1smp')")


def test_the_milliseconds_of_the_nudge_value_follow_the_rate_of_the_track(make_project, app):
    open_project(app, make_project(seconds=4.0, tracks=(LANE,), sr=44100))
    with_hits(app, js(app, "() => S.sid"), {"Kick In": [1.0]})
    select_track(app, "Kick In")
    app.evaluate("() => { S.playhead = 0; S.sel = null; setView(0, 4); }")
    app.keyboard.press("Tab")
    t = lambda: (edits_done(app), round(js(app, "() => S.events['Kick In'][S.sel.index].t") * 44100))[1]   # P4-9 stage 2: the stored hits are shown after the edit
    menu(app, "nudge_val_1ms")
    app.keyboard.press(".")
    assert t() == 44100 + 44                                              # 44.1 samples, rounded to a sample
    menu(app, "nudge_val_10ms")
    app.keyboard.press(".")
    assert t() == 44100 + 44 + 441
    app.evaluate("() => setNudgeValue('1smp')")


def test_both_view_settings_are_app_settings_read_at_boot(project):
    app = project
    app.evaluate("() => setInspector(true)")
    wait_until(lambda: settings_json(app).get("inspector") is True, msg="the Inspector setting")
    app.keyboard.press("Alt+Meta+]")
    app.keyboard.press("Alt+Meta+]")
    assert menu(app, "nudge_val_10ms") is True
    wait_until(lambda: settings_json(app).get("amp_zoom") == 4, msg="the amp_zoom setting")
    wait_until(lambda: settings_json(app).get("nudge_value") == "10ms", msg="the nudge_value setting")
    assert settings_json(app)["inspector"] is True                        # the other keys stay
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0 && GEO.ok")
    assert amp(app) == 4 and label(app) == ["Amp 4x", False]
    assert js(app, "() => S.nudgeValue") == "10ms"
    assert js(app, "() => document.querySelector('#nudgeSel').selectedOptions[0].textContent") == "Nudge 10 ms"
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.nudgeValue") == "10ms", msg="the radio item state after the reload")
    assert lane(app)["top"] == 0                                          # the lane is drawn with the saved zoom
    app.keyboard.press("Control+Alt+Meta+[")
    wait_until(lambda: settings_json(app).get("amp_zoom") == 1, msg="the reset is saved")
    # values the page does not know are ignored: the defaults
    invoke(app, "app_settings_set", key="amp_zoom", value=3)
    invoke(app, "app_settings_set", key="nudge_value", value="1 beat")
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0 && GEO.ok")
    assert amp(app) == 1 and label(app)[1] is True
    assert js(app, "() => S.nudgeValue") == "1smp"
    invoke(app, "app_settings_set", key="amp_zoom", value=1)
    invoke(app, "app_settings_set", key="nudge_value", value="1smp")


def test_the_cubase_preset_has_no_keys_for_the_new_commands(preset_home, make_project):
    app = preset_home
    open_project(app, make_project(tracks=(LANE,)))
    choose(app, "cubase")
    for k in ("Alt+Meta+]", "Alt+Meta+[", "Control+Alt+Meta+[", "Alt+Meta+NumpadAdd", "Alt+Meta+NumpadSubtract"):
        app.keyboard.press(k)
    assert amp(app) == 1 and js(app, "() => S.nudgeValue") == "1smp"
    for id in ("amp_in", "amp_out", "amp_reset", "nudge_value_up", "nudge_value_down"):
        assert js(app, "id => DSKeys.keysOf(DSKeys.COMMANDS.find(c => c.id === id), 'cubase')", id) == []
        assert js(app, "id => DSKeys.keysOf(DSKeys.COMMANDS.find(c => c.id === id), 'default').length", id) == 1
    assert menu(app, "amp_in") is True and amp(app) == 2                  # the menu items work in both presets
    app.evaluate("() => setAmpZoom(1)")
