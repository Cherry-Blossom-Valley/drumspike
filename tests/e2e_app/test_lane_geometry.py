"""#559 item 7: the lines of the hits and of the grid span the whole lane after a row got its own height.

The finding of the owner in the DAW: after the zoom and the track height changed, the lines of a track spanned about two
thirds of its lane. The Zoom Toggle and the drag of a row give a row its own wave height (S.rowH, setRowHeights). The page
writes the new store size at once, but the row is laid out later: the first draw used the old height (120 px of a 180 px
lane), the observer then measured the new one, and the key of the sizes did not include it, so nothing drew the lines
again. The same defect is in the standalone app (the tests run in both modes). The test reads the pixels of the line layer."""
import pytest

from app_support import open_project
from test_host_page import FOUR, analyzed

LINES = """() => { const out = {};
  for (const row of document.querySelectorAll('.track')) {
    const c = row.querySelector('canvas.wave'), g = c.getContext('2d'), d = g.getImageData(0, 0, c.width, c.height).data;
    let minY = 1e9, maxY = -1;
    for (let y = 0; y < c.height; y++) for (let x = 0; x < c.width; x++) if (d[(y * c.width + x) * 4 + 3] > 100) { if (y < minY) minY = y; if (y > maxY) maxY = y; }
    out[row.dataset.name] = { box: c.getBoundingClientRect().height, minY: minY / GEO.dpr, maxY: maxY / GEO.dpr };
  } return out; }"""


def settle(page):
    page.wait_for_timeout(500)
    page.wait_for_function("() => !FR.raf && FR.flags === 0")


def open_analyzed(request, fixture, make_project):
    page = request.getfixturevalue(fixture)
    folder = make_project(tracks=FOUR, seconds=6.0)
    if fixture == "host_app":
        analyzed(page, folder)
    else:
        open_project(page, folder)
        page.click("#analyzeAllBtn")
        page.wait_for_function("() => S.order.every(n => (S.events[n] || []).length > 0) && !analysisRunning", timeout=60000)
    page.set_viewport_size({"width": 1400, "height": 900})
    page.evaluate("() => zoomPreset(0)")
    settle(page)
    return page


@pytest.mark.parametrize("fixture", ["app", "host_app"])
def test_the_lines_span_a_row_with_its_own_height_and_follow_every_change(request, make_project, fixture):
    page = open_analyzed(request, fixture, make_project)
    first = page.evaluate("() => S.order[0]")
    for step, js in [("base", "() => 0"), ("row 1.5x", "n => { const o = {}; o[n] = 1.5; setRowHeights(o); }"),
                     ("row 1.5x, large", "() => setTrackHeight('large')"), ("rows reset", "() => setRowHeights({})"),
                     ("small", "() => setTrackHeight('small')"), ("row 2x", "n => { const o = {}; o[n] = 2; setRowHeights(o); }"),
                     ("medium", "() => setTrackHeight('medium')")]:
        page.evaluate(js, first)
        settle(page)
        for name, ln in page.evaluate(LINES).items():
            assert ln["minY"] <= 1.5 and ln["maxY"] >= ln["box"] - 2.5, (step, name, ln)


def test_the_check_sees_the_old_key_of_the_sizes(host_app, make_project, request):
    """The negative case: with the key of before #559 the lines of the taller row stay at the old height."""
    page = open_analyzed(request, "host_app", make_project)
    page.evaluate("() => { window.sizeKey = () => [GEO.laneW, GEO.rulerW, GEO.ovW, GEO.ovH, GEO.waveH, GEO.spikeH, GEO.dpr, rowHKey(), VL.on ? VL.h : 0].join(','); }")
    first = page.evaluate("() => S.order[0]")
    page.evaluate("n => { const o = {}; o[n] = 1.5; setRowHeights(o); }", first)
    settle(page)
    ln = page.evaluate(LINES)[first]
    assert ln["maxY"] < ln["box"] - 20, ln   # two thirds of the lane
