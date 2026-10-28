"""P4-27: Zoom Toggle to the selection (owner request 2026-10-14; Pro Tools key E, static/keymap.js row zoom_toggle).
With a range selected (P4-6a) E zooms so the range fills the lane width; E again restores the zoom, the scroll position and
the track heights. Without a range the old behaviour stays (5 ms around the selected hit or the playhead). The app setting
"Zoom Toggle expands track height" (zoom_toggle_tall, off by default) makes E also double the wave lane of the track that
holds the selection, as a local override of that row (the app setting track_height is not touched). Each guard has a
planted-fault check: the measuring function must fail on a wrong state."""
import pytest

from app_support import invoke, open_project, wait_until
from test_inspector import settings_json
from test_keypreset import reload
from test_menus import select_track, with_hits

SIX = tuple((f"Tom {i}", "tom", 45 + i) for i in range(1, 7))
EDGE_PX = 2


def js(page, expr, arg=None):
    return page.evaluate(expr, arg) if arg is not None else page.evaluate(expr)


def edges(page, t0, t1):
    """The x offsets (px) of t0 and t1 from the left and the right edge of the lane."""
    return js(page, "([a, b]) => [tToX(a, laneWidth()), tToX(b, laneWidth()) - laneWidth()]", [t0, t1])


def fills(page, t0, t1):
    l, r = edges(page, t0, t1)
    return abs(l) <= EDGE_PX and abs(r) <= EDGE_PX


SNAP = """() => { const sc = document.querySelector('#lanesScroll');
  return { view: [S.view.t0, S.view.t1], top: sc.scrollTop,
           heights: S.order.map(n => document.querySelector(`.track[data-name="${n}"] canvas.wave`).clientHeight),
           stores: S.order.map(n => { const c = document.querySelector(`.track[data-name="${n}"] canvas.wave`); return c.width > 1 ? c.height : 0; }),
           geo: S.order.map(n => GEO.rows[n].waveH), rowH: {...S.rowH}, glob: S.trackHeight }; }"""


def snap(page):
    page.evaluate("() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))")
    return js(page, SNAP)


def same_view(a, b):
    """The state a toggle must restore: zoom, scroll position, every lane height and the overrides."""
    return all(a[k] == b[k] for k in ("view", "top", "heights", "geo", "rowH", "glob"))


@pytest.fixture
def zt(app, make_project):
    sid = open_project(app, make_project(tracks=SIX, seconds=12.0))
    with_hits(app, sid, {"Tom 3": [2.0, 3.0], "Tom 5": [4.0]})
    js(app, "() => setZoomTall(false, false)")
    app.evaluate("() => { S.playhead = 0; S.sel = null; lnClearSel(); setView(0, 12); }")
    yield app
    js(app, "() => setZoomTall(false)")


def select_range(page, track, a, b):
    js(page, "([n, a, b]) => { S.selTrack = n; lnSetSel(n, a, b); }", [track, a, b])


def test_the_range_fills_the_lane_width_and_e_again_restores_the_view(zt):
    app = zt
    before = snap(app)
    select_range(app, "Tom 3", 2.2, 4.7)
    app.keyboard.press("e")
    v = js(app, "() => [S.view.t0, S.view.t1]")
    assert fills(app, 2.2, 4.7), edges(app, 2.2, 4.7)
    assert v == [2.2, 4.7]
    app.keyboard.press("e")
    assert same_view(snap(app), before)
    # planted faults: the measure fails for a view a little too wide, shifted, or the old 5 ms zoom
    for bad in ([2.0, 4.9], [2.4, 4.9], [2.2 - 0.0125, 4.7 + 0.0125 + 0.5]):
        js(app, "([a, b]) => setView(a, b)", bad)
        assert not fills(app, 2.2, 4.7), bad
    js(app, "() => setView(0, 12)")


def test_a_second_toggle_and_no_range_keep_the_old_behaviour(zt):
    app = zt
    select_track(app, "Tom 3")
    app.keyboard.press("Tab")                                             # the hit at 2.0 s
    app.keyboard.press("e")
    v = js(app, "() => [S.view.t0, S.view.t1]")
    assert v[1] - v[0] == pytest.approx(0.005) and (v[0] + v[1]) / 2 == pytest.approx(2.0)
    app.keyboard.press("e")
    assert js(app, "() => [S.view.t0, S.view.t1]") == [0, 12]
    # a zero-width range is no range: the hit decides
    js(app, "() => { lnSetSel('Tom 3', 5, 5); }")
    app.keyboard.press("e")
    assert js(app, "() => S.view.t1 - S.view.t0") == pytest.approx(0.005)
    app.keyboard.press("e")


def test_setting_off_leaves_every_height(zt):
    app = zt
    assert js(app, "() => S.zoomTall") is False
    assert "zoom_toggle_tall" not in settings_json(app) or settings_json(app)["zoom_toggle_tall"] is False
    before = snap(app)
    select_range(app, "Tom 5", 3.5, 4.5)
    app.keyboard.press("e")
    now = snap(app)
    assert now["heights"] == before["heights"] and now["rowH"] == {} and fills(app, 3.5, 4.5)
    # planted fault: the same measure sees a height change when the setting is on
    app.keyboard.press("e")
    js(app, "() => setZoomTall(true, false)")
    app.keyboard.press("e")
    assert snap(app)["heights"] != before["heights"]
    app.keyboard.press("e")


def test_setting_on_doubles_the_selected_track_keeps_it_in_view_and_e_again_restores_everything(zt):
    app = zt
    js(app, "() => setZoomTall(true, false)")
    js(app, "() => setTrackHeight('large', false)")                       # six tall rows: the lanes scroll
    app.wait_for_function("() => GEO.waveH === 240")
    js(app, "() => { const sc = document.querySelector('#lanesScroll'); sc.scrollTop = 0; }")
    saved = settings_json(app).get("track_height")
    before = snap(app)
    assert before["heights"] == [240] * 6
    select_range(app, "Tom 5", 3.5, 4.5)                                  # a track below the fold
    app.keyboard.press("e")
    now = snap(app)
    i = 4
    assert now["heights"][i] == 480 and now["geo"][i] == 480 and all(h == 240 for j, h in enumerate(now["heights"]) if j != i), now["heights"]
    assert now["stores"][i] in (0, 480 * js(app, "() => GEO.dpr"))        # the canvas store has the new size (or none when far away)
    assert now["glob"] == "large" and settings_json(app).get("track_height") == saved   # the app setting is not touched
    assert fills(app, 3.5, 4.5)
    # the expanded track fills the lanes viewport: with the rulers on (P4-29) a 480 px track is taller
    # than the visible lanes area, so "in view" means the viewport shows only this track, top aligned
    inview = js(app, """() => { const sc = document.querySelector('#lanesScroll'), r = document.querySelector('.track[data-name="Tom 5"]').getBoundingClientRect(), s = sc.getBoundingClientRect();
        return r.height <= s.height + 1 ? (r.top >= s.top - 1 && r.bottom <= s.bottom + 1) : Math.abs(r.top - s.top) <= 1; }""")
    assert inview and now["top"] > 0
    app.keyboard.press("e")
    after = snap(app)
    assert same_view(after, before), (before, after)
    # planted fault: without the restore of the heights the comparison fails
    app.keyboard.press("e")
    js(app, "() => { S.zoomBack.rowH = null; }")
    app.keyboard.press("e")
    assert not same_view(snap(app), before)
    js(app, "() => setRowHeights({})")
    js(app, "() => setTrackHeight('medium', false)")


def test_a_selected_hit_without_a_range_expands_its_own_track(zt):
    app = zt
    js(app, "() => setZoomTall(true, false)")
    before = snap(app)
    select_track(app, "Tom 5")
    app.keyboard.press("Tab")                                             # the hit at 4.0 s
    assert js(app, "() => S.sel.track") == "Tom 5"
    app.keyboard.press("e")
    now = snap(app)
    assert now["heights"][4] == 2 * before["heights"][4] and sum(now["heights"]) == sum(before["heights"]) + before["heights"][4]
    assert js(app, "() => S.view.t1 - S.view.t0") == pytest.approx(0.005)
    app.keyboard.press("e")
    assert same_view(snap(app), before)
    # no hit and no range: nothing is expanded
    js(app, "() => { S.sel = null; lnClearSel(); }")
    app.keyboard.press("e")
    assert snap(app)["heights"] == before["heights"]
    app.keyboard.press("e")


def test_the_setting_is_a_checkbox_in_settings_saved_and_read_back_after_a_reload(zt):
    app = zt
    js(app, "() => openSettings()")
    box = app.locator("#setZoomTall")
    assert not box.is_checked()                                           # off by default
    box.check()
    wait_until(lambda: settings_json(app).get("zoom_toggle_tall") is True, msg="the setting saved on")
    assert js(app, "() => S.zoomTall") is True
    reload(app)
    assert js(app, "() => S.zoomTall") is True
    js(app, "() => openSettings()")
    assert app.locator("#setZoomTall").is_checked()
    app.locator("#setZoomTall").uncheck()
    wait_until(lambda: settings_json(app).get("zoom_toggle_tall") is False, msg="the setting saved off")
    assert js(app, "() => S.zoomTall") is False
    reload(app)
    assert js(app, "() => S.zoomTall") is False
