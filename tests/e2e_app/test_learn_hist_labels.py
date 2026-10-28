"""#210: the Learn histogram labels (Match, Add) are fully visible at dpr 1 and dpr 2.

The labels are drawn with textBaseline top and a 2*dpr top margin, and the bars' height is
reduced by the label band. A pixel check on the drawn canvas: a clipped label has ink on the
very first rows (the old code drew a 10*dpr font on the fixed baseline 11, so at dpr 2 its top
was cut off). When the two labels would overlap, the Add label moves to the other side of its
line or to a second line, so their ink never intersects. Needs the model (the DSP fallback keeps
no candidates)."""
import pytest

from app_support import inspect_track, open_project
from conftest import READY
from test_learn_filter import NAME, build, idle, js, mark_hit, scores
from test_subtracks import analyse, has_model


def boxes(page, dpr):
    """Ink bounding boxes of the red (Match) and green (Add) labels and of the blue bars.

    The 1px vertical lines at the threshold and at the Add value are excluded (they span the
    full height, so their ink would mask a clipped label)."""
    return js(page, """([dpr]) => {
      const c = document.querySelector('#insParams .lnHist'), x = c.getContext('2d');
      const W = c.width, H = c.height, d = x.getImageData(0, 0, W, H).data;
      const near = (p, r, g, b) => Math.abs(p[0] - r) < 40 && Math.abs(p[1] - g) < 40 && Math.abs(p[2] - b) < 40;
      const lines = [...Array(W).keys()].filter(cx => {
        const y = Math.floor(H / 2), p = [d[(y * W + cx) * 4], d[(y * W + cx) * 4 + 1], d[(y * W + cx) * 4 + 2]];
        return near(p, 0xe5, 0x53, 0x4b) || near(p, 0x4c, 0xc3, 0x8a);
      });
      const isLine = new Set(lines.flatMap(cx => [cx - 1, cx, cx + 1]));
      const box = (r, g, b, band) => { let t = null, bt = null;
        for (let y = 0; y < (band || H); y++) for (let cx = 0; cx < W; cx++) {
          if (isLine.has(cx)) continue;
          const i = (y * W + cx) * 4;
          if (d[i + 3] && near([d[i], d[i + 1], d[i + 2]], r, g, b)) {
            if (t === null || y < t) t = y; if (bt === null || y > bt) bt = y;
          } }
        return t === null ? null : { t, b: bt }; };
      return { red: box(0xe5, 0x53, 0x4b, Math.round(24 * dpr)), green: box(0x4c, 0xc3, 0x8a, Math.round(24 * dpr)),
               blue: box(0x4d, 0xa3, 0xff, null) };
    }""", [dpr])


@pytest.fixture(params=[1, 2])
def appdpr(browser, warm_page, request):
    """The page on the bridge in a context with deviceScaleFactor 1 or 2."""
    ctx = browser.new_context(device_scale_factor=request.param, viewport={"width": 1280, "height": 900})
    pg = ctx.new_page()
    pg.errors = []
    pg.on("pageerror", lambda e: pg.errors.append(f"pageerror: {e}"))
    pg.on("dialog", lambda d: d.accept())
    pg.add_init_script("localStorage.removeItem('dsg_sid')")
    pg.goto(warm_page.url + "/")
    pg.wait_for_function(READY)
    pg.request_param = request.param
    yield pg
    assert pg.errors == [], pg.errors
    ctx.close()


@pytest.fixture
def lrd(appdpr, make_project, tmp_path):
    """One kick track with real hits, analysed, with a This example (so the Add label exists)."""
    page = appdpr
    sid = open_project(page, build(make_project, tmp_path))
    analyse(page)
    inspect_track(page, NAME)
    page.locator("#insParams .pSens").fill("150")
    page.locator("#insParams .pSens").dispatch_event("change")
    page.wait_for_function("() => !hgRepickBusy()")
    js(page, "() => { if (document.activeElement && document.activeElement.blur) document.activeElement.blur(); }")
    head = page.locator('#inspector .insSec[data-sec="learn"] .insSecHead')
    if not js(page, "() => !!INSSEC.open.learn"):
        head.click()
    mark_hit(page, 0.5, "K")            # a This example: the Add line and its label are drawn
    mark_hit(page, 1.0, "Shift+K")      # a Not example: the filter (and the histogram) is on
    idle(page)
    scores(page)
    js(page, "n => lnHistDraw(n)", NAME)
    return page, sid


def test_labels_are_drawn_and_never_clipped(lrd):
    page, sid = lrd
    dpr = page.request_param
    bx = boxes(page, dpr)
    assert bx["red"], "the Match label is not drawn"
    assert bx["green"], "the Add label is not drawn"
    assert bx["red"]["t"] >= dpr, bx       # planted fault: the old code clips the label at row 0 on dpr 2
    assert bx["green"]["t"] >= dpr, bx
    assert bx["red"]["b"] <= 14 * dpr + 4 and bx["green"]["b"] <= 14 * dpr + 4, bx   # inside the label band
    assert bx["blue"] is None or bx["blue"]["t"] >= 12 * dpr, bx   # planted fault: the bars run into the label band


def test_overlapping_labels_move_apart(lrd):
    """A threshold next to the Add value puts the two labels on one row: the Add label must move
    (other side of its line, or a second line) so the ink of the two labels never intersects."""
    page, sid = lrd
    js(page, "n => lnHistSetThreshold(n, 0.8)", NAME)   # 0.8 sits between the label start and the Add 0.85
    idle(page)
    # #280: the threshold's Learn change drops the cached scores and refetches them async
    # (lnApply -> lnScoreDrop -> lnScores); idle() waits for the Learn queue and the hit edits,
    # not for that fetch. A draw before it lands shows "no scores yet" and no labels at all
    # (all boxes None, e2e-arm64 run 39913704509). Wait for the scores of the new learn object.
    page.wait_for_function("n => DSLearn.scoresReady(n)", arg=NAME, timeout=30000)
    js(page, "n => lnHistDraw(n)", NAME)
    bx = boxes(page, page.request_param)
    assert bx["red"] and bx["green"], bx
    disjoint = bx["red"]["b"] < bx["green"]["t"] or bx["green"]["b"] < bx["red"]["t"]
    assert disjoint, bx   # planted fault: both labels share the fixed baseline 11 and overlap
