"""#250: the toolbar in a narrow window. The toolbar's natural width is about 1160 px (the #246
review): below that, the right side (Master, meter, Feedback) was cut off and the icon buttons
were squeezed. The fix: #topbar wraps to a second row and no control shrinks under its natural
width (#topbar > * { flex: none }); the window gets a minWidth of 640 px in tauri.conf.json.

The gate: at 1600, 1200, 1000 and the smallest allowed width (640) every toolbar control is
inside the viewport and not narrower than its natural width (getBoundingClientRect + scrollWidth
vs clientWidth: squeezed content is wider than the box). Screenshots at the four widths go to
test-shots/ for the coordinator."""
import pytest

from app_support import open_project, shot_dir

WIDTHS = [1600, 1200, 1000, 640]   # 640: the smallest allowed window width (tauri.conf.json minWidth)

PROBE = """() => {
  const vw = window.innerWidth;
  const name = el => el.id || (el.className && String(el.className).split(' ')[0]) || el.tagName;
  const els = document.querySelectorAll('#topbar button, #topbar select, #topbar .tgroup, #topbar .timebox, #topbar .master, #topbar .logo, #topbar .sep, #topbar .ampLbl, #topbar #feedbackBtn');
  const bad = [];
  for (const el of els) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) continue;   // hidden (Amp 1x at 1x)
    if (r.left < -0.5 || r.right > vw + 0.5) {
      bad.push(name(el) + ' cut off: right ' + r.right.toFixed(1) + ' of ' + vw);
    } else if (el.scrollWidth > el.clientWidth + 0.5) {
      bad.push(name(el) + ' squeezed: scrollWidth ' + el.scrollWidth + ' > clientWidth ' + el.clientWidth);
    }
  }
  return { vw, bad, rows: Math.round(document.querySelector('#topbar').getBoundingClientRect().height) };
}"""


@pytest.mark.parametrize("width", WIDTHS)
def test_every_toolbar_control_is_whole_at(width, app, make_project):
    """#250: no control is cut off or squeezed at any allowed width; Feedback stays visible."""
    open_project(app, make_project())
    app.set_viewport_size({"width": width, "height": 700})
    app.wait_for_timeout(200)
    res = app.evaluate(PROBE)
    print(f"#250 at {res['vw']} px: {len(res['bad'])} problem(s), topbar height {res['rows']} px"
          + (": " + "; ".join(res["bad"]) if res["bad"] else " — all controls whole"))
    assert res["bad"] == [], res["bad"]
    fb = app.locator("#feedbackBtn").bounding_box()
    assert fb is not None and fb["x"] >= 0 and fb["x"] + fb["width"] <= width
    app.screenshot(path=str(shot_dir(f"250-toolbar-{width}px.png")))
