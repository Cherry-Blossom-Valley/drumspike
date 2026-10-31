"""#245: the velocity lane's gesture hint is the tooltip of the Velocity label, not text drawn
into the lane canvas. The canvas pixels of the former hint area show no text, hovering the
label shows the hint, hovering a stalk shows no tooltip; screenshots of the lane with the hint
drawn (the old look) and without it go to test-shots for the review.
"""
import pytest

from app_support import invoke, open_project, shot_dir
from test_velocity_lane import js  # noqa: F401
from test_follow_strip import IDLE  # noqa: F401


@pytest.fixture
def lane(app, make_project):
    """A project with two hits late in the lane (so the former hint area at the lane's top left
    is free of stalks) and the velocity lane on."""
    sid = open_project(app, make_project(seconds=8.0))
    invoke(app, "events_set", sid=sid, name="Kick In",
           events=[{"t": t, "v": 90, "id": f"K{i}"} for i, t in enumerate((6.0, 7.0))])
    app.evaluate("async () => { await loadProject(); }")
    app.evaluate("() => { if (!VL.on) vlToggle(); }")
    app.wait_for_function(IDLE)
    return sid


def test_the_lane_canvas_shows_no_hint_text(app, lane):
    assert js(app, "() => typeof VL.hint") == "undefined", "the hint flag is gone"
    assert "Drag a stalk" not in js(app, "() => drawVelLane.toString()"), "the hint is not drawn"
    # the pixels of the former hint area (x 64 to 340, y 6 to 16, where the one-line hint sat)
    # are the plain lane background: no text, and no stalk (the hits sit right of x 340)
    rgb = app.evaluate(
        """() => { const c = document.querySelector('.track[data-name="Kick In"] canvas.vel');
          const ctx = c.getContext('2d'), [W, H] = laneSize(c);
          if (W < 360) return null;
          for (let y = 6; y <= 16; y++) for (let x = 64; x <= 340; x++) {
            const p = ctx.getImageData(x, y, 1, 1).data;
            if (p[3] !== 0 && (p[0] !== 0 || p[1] !== 0 || p[2] !== 0)) return [x, y, p[0], p[1], p[2]];
          }
          return 'clear'; }"""
    )
    assert rgb == "clear", f"text (or a stalk) in the former hint area at {rgb}"


def test_hovering_the_label_shows_the_hint_and_a_stalk_shows_no_tooltip(app, lane):
    label = app.locator('.track[data-name="Kick In"] .velLabel')
    assert label.get_attribute("title") == ("Drag a stalk to set velocity. Option-drag: draw a "
                                            "line. Cmd-drag: draw freehand.")
    label.hover()   # the tooltip is the title on the label
    over = app.evaluate(
        "() => { const l = document.querySelector('.track[data-name=\"Kick In\"] .velLabel');"
        " const r = l.getBoundingClientRect();"
        " return document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2).tagName; }")
    assert over == "SPAN", over   # the label is hoverable: the tooltip shows
    # a stalk: the pointer is over the canvas, which carries no title
    x, y = app.evaluate(
        """() => { const c = document.querySelector('.track[data-name="Kick In"] canvas.vel');
          const r = c.getBoundingClientRect(); return [r.left + tToX(6.0, GEO.laneW), r.top + 40]; }""")
    app.mouse.move(x, y)
    under = app.evaluate(
        """([x, y]) => { const el = document.elementFromPoint(x, y); return el ? el.tagName + (el.title || '') : null; }""",
        [x, y])
    assert under == "CANVAS", under   # no tooltip on a stalk: the canvas has no title


def test_the_screenshots_of_the_lane_with_and_without_the_hint(app, lane):
    # the old look: the hint drawn into the canvas exactly as the pre-#245 code drew it
    app.evaluate(
        """() => { const c = document.querySelector('.track[data-name="Kick In"] canvas.vel');
          const ctx = c.getContext('2d'), [W, H] = laneSize(c);
          const headY = vlYOf(127, H) + 4;
          ctx.fillStyle = 'rgba(230,233,239,0.75)'; ctx.textAlign = 'left';
          ctx.textBaseline = 'top';
          ctx.fillText('Drag a stalk to set velocity. Option-drag: draw a line. Cmd-drag: draw freehand.', 64, headY);
          ctx.textBaseline = 'middle'; }""")
    app.screenshot(path=str(shot_dir("245-vel-hint-before.png")))
    app.evaluate(
        "() => { const c = document.querySelector('.track[data-name=\"Kick In\"] canvas.vel'); drawVelLane(c, 'Kick In'); }")
    app.screenshot(path=str(shot_dir("245-vel-hint-after.png")))
