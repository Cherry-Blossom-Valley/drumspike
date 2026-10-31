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


# #241: the hide rule must account for the full height of .tctl (the M/S buttons AND the
# readouts under them). The rule of #172 used tctl.offsetTop + 8 + size, which ignores
# tctl.offsetHeight: in a band of row heights the drawing is still shown and overlaps the
# controls. This test sweeps the row height from tall to short in small steps and asserts at
# EVERY step that a visible drawing does not intersect the controls; it also saves the three
# screenshots (tall, the threshold, just below it) for the review.
def test_the_drawing_stays_clear_of_the_controls_at_every_row_height(app, song):
    from app_support import shot_dir
    name = app.evaluate("() => S.order[0]")
    first_fail = None
    last_visible = None
    for h in range(200, 79, -2):
        app.evaluate("([name, h]) => setRowHeights({ [name]: h / laneHeights().wave })", [name, h])
        tart = app.evaluate(TART_BOX, name)
        ctl = app.evaluate(CTL_BOX, name)
        if tart is not None:
            if not disjoint(tart, ctl) and first_fail is None:
                overlap = min(tart[3], ctl[3]) - max(tart[1], ctl[1])
                first_fail = (h, overlap)
            last_visible = h
    assert first_fail is None, (
        f"the drawing overlaps the controls by {first_fail[1]:.1f}px at the first failing "
        f"row height {first_fail[0]}px")
    assert last_visible is not None, "the drawing was never visible in the sweep"
    # the three heights for the review: tall, the threshold (the last visible), just below it
    app.evaluate("([name, h]) => setRowHeights({ [name]: h / laneHeights().wave })", [name, 200])
    app.screenshot(path=str(shot_dir("241-tart-tall-200px.png")))
    app.evaluate("([name, h]) => setRowHeights({ [name]: h / laneHeights().wave })", [name, last_visible])
    app.screenshot(path=str(shot_dir(f"241-tart-threshold-{last_visible}px.png")))
    below = last_visible - 2
    app.evaluate("([name, h]) => setRowHeights({ [name]: h / laneHeights().wave })", [name, below])
    app.screenshot(path=str(shot_dir(f"241-tart-just-below-{below}px.png")))
    # and the drawing is visible again at the default height
    app.evaluate("() => setRowHeights({})")
    assert app.evaluate(TART_BOX, name) is not None
