"""#172: the header role drawing hides when a row is dragged below its needed height.

The drawing (.tart) sits bottom-right in the header. A row dragged short (the minimum wave
height) would let the drawing overlap the M/S buttons and the pan slider of the header; the
drawing hides instead, while a row at the global height keeps it.
"""
import pytest

from app_support import edits_done, open_project


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


TART_BOX = """name => {
  const row = document.querySelector(`.track[data-name="${CSS.escape(name)}"]`);
  const t = row && row.querySelector('.tart');
  if (!t) return null;
  const cs = getComputedStyle(t);
  if (cs.display === 'none') return null;
  const r = t.getBoundingClientRect();
  return [r.x, r.y, r.x + r.width, r.y + r.height];
}"""

CTL_BOX = """name => {
  const row = document.querySelector(`.track[data-name="${CSS.escape(name)}"]`);
  const c = row && row.querySelector('.tctl');
  if (!c) return null;
  const r = c.getBoundingClientRect();
  return [r.x, r.y, r.x + r.width, r.y + r.height];
}"""


def disjoint(a, b):
    return a[0] >= b[2] or b[0] >= a[2] or a[1] >= b[3] or b[1] >= a[3]


@pytest.fixture
def song(app, make_project):
    sid = open_project(app, make_project(seconds=8.0))
    app.evaluate("() => { setTrackHeight('medium', false); }")   # a known global height (120)
    yield sid


def test_a_row_dragged_to_the_minimum_hides_the_drawing(app, song):
    short = app.evaluate("() => S.order[1]")
    drag_border(app, short, -800)   # down to the minimum wave height (24 px)
    assert app.evaluate(TART_BOX, short) is None   # the drawing of the short row is hidden


def test_a_medium_row_keeps_its_drawing_clear_of_the_controls(app, song):
    medium = app.evaluate("() => S.order[0]")
    tart = app.evaluate(TART_BOX, medium)
    assert tart is not None   # the drawing of an untouched medium row stays
    assert disjoint(tart, app.evaluate(CTL_BOX, medium))   # and does not overlap the controls


def test_dragging_the_row_back_restores_the_drawing(app, song):
    name = app.evaluate("() => S.order[1]")
    drag_border(app, name, -800)
    drag_border(app, name, 400)
    assert app.evaluate(TART_BOX, name) is not None
