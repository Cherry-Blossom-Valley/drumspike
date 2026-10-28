"""#167: the height of one track by a drag of its lower border.

The drag handle sits on the lower border of every track row; a drag changes only that row's wave
lane (the spike lane stays), a double-click resets the row to the global Track Height, the factors
live in the project document (`row_heights_get`, `row_heights_set`) and come back with the
reload, and the global Track Height zoom still scales every lane, the dragged one included.
"""
import pytest

from app_support import edits_done, invoke, open_project

WAVE_H = """() => Object.fromEntries(S.order.map(n => {
  const c = document.querySelector(`.track[data-name="${CSS.escape(n)}"] canvas.wave`);
  return [n, c.clientHeight];
}))"""


def handle(app, name, sel=".tlanes"):
    return app.locator(f'.track[data-name="{name}"] {sel} .rowResize').bounding_box()


def drag_border(app, name, dy, sel=".tlanes"):
    box = handle(app, name, sel)
    app.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    app.mouse.down()
    app.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2 + dy, steps=8)
    app.mouse.up()
    edits_done(app)
    app.wait_for_function("() => !document.querySelector('.rowResize:active')")


@pytest.fixture
def song(app, make_project):
    sid = open_project(app, make_project(seconds=8.0))
    app.evaluate("() => { setTrackHeight('medium', false); }")   # a known global height (120)
    yield sid


def test_a_drag_of_the_lower_border_changes_only_that_lane(app, song):
    h0 = app.evaluate(WAVE_H)
    order = app.evaluate("() => S.order")
    second = order[1]
    drag_border(app, second, 60)
    h1 = app.evaluate(WAVE_H)
    assert h1[second] == pytest.approx(h0[second] + 60, abs=2)
    for n, h in h1.items():
        if n != second:
            assert h == h0[n], n


def test_a_double_click_on_the_border_resets_the_row_to_the_global_height(app, song):
    second = app.evaluate("() => S.order[1]")
    drag_border(app, second, 60)
    box = handle(app, second)
    app.mouse.dblclick(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    edits_done(app)
    h = app.evaluate(WAVE_H)
    assert h[second] == 120   # the global medium wave height


def test_the_heights_are_saved_in_the_project_and_restored_after_reload(app, song):
    second = app.evaluate("() => S.order[1]")
    drag_border(app, second, 60)
    assert invoke(app, "row_heights_get", sid=song)["heights"][second] == pytest.approx(1.5)
    invoke(app, "project_save", sid=song)
    app.evaluate("async () => { await loadProject(); }")
    app.wait_for_function("() => document.querySelectorAll('#lanesScroll .track').length > 0")
    h = app.evaluate(WAVE_H)
    assert h[second] == pytest.approx(180, abs=2)


def test_the_global_track_height_zoom_still_scales_all_lanes(app, song):
    second = app.evaluate("() => S.order[1]")
    drag_border(app, second, 60)                       # factor 1.5 of the medium 120
    app.evaluate("() => { setTrackHeight('large', false); }")   # global wave 240
    h = app.evaluate(WAVE_H)
    assert h[second] == pytest.approx(360, abs=2)      # 1.5 * 240: the dragged lane scales too
    for n, v in h.items():
        if n != second:
            assert v == 240, n


def test_row_heights_set_validates(app, song):
    r = invoke(app, "row_heights_set", sid=song, heights={"Kick In": 2.0})
    assert r["changed"] and r["n"] == 1
    bad = invoke(app, "row_heights_set", sid=song, heights={"Kick In": 0})
    assert bad["code"] == 400 and "Kick In" in bad["message"], bad   # a factor of 0 is refused (the message names the track)


# #439: the track header's bottom edge is the same drag handle
def test_a_drag_on_the_header_edge_changes_the_lane_like_the_lane_drag(app, song):
    h0 = app.evaluate(WAVE_H)
    order = app.evaluate("() => S.order")
    second = order[1]
    drag_border(app, second, 60, sel=".thead")   # the header's bottom edge: the same stored height, the whole row follows
    h1 = app.evaluate(WAVE_H)
    assert h1[second] == pytest.approx(h0[second] + 60, abs=2)
    for n, h in h1.items():
        if n != second:
            assert h == h0[n], n


def test_a_double_click_on_the_header_edge_resets_the_row(app, song):
    second = app.evaluate("() => S.order[1]")
    drag_border(app, second, 60, sel=".thead")
    box = handle(app, second, ".thead")
    app.mouse.dblclick(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    edits_done(app)
    h = app.evaluate(WAVE_H)
    assert h[second] == 120   # the global medium wave height


def test_a_click_on_the_header_handle_does_not_select_the_track(app, song):
    first = app.evaluate("() => S.order[0]")
    second = app.evaluate("() => S.order[1]")
    box = handle(app, second, ".thead")
    app.mouse.click(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    edits_done(app)
    assert app.evaluate("() => S.selTrack") == first   # the selection keeps the first track (P4-39c)
