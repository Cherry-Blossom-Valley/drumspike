"""P3-17: the page tells the shell that its first layout is finished, and it is. The main window starts hidden
(tauri.conf.json); the shell shows it when the page calls B.appReady() (command app_ready), so what the user
sees first must already be laid out. The cause of the zoomed start was measured here: a canvas without a CSS
height (#ruler) took the 2:1 box of its 300 x 150 backing store, 685 px high at 1600 px width, until the
first resizeCanvases (which runs only after a project is loaded). The real window and the splash are
checked in the .app by the self-test scenario `shell` (step splash_and_first_show)."""
PROBE = """
window.__ready = [];
window.__DS_ON_READY__ = n => {
  const r = s => { const e = document.querySelector(s); if (!e) return null; const b = e.getBoundingClientRect();
    return { w: Math.round(b.width), h: Math.round(b.height) }; };
  window.__ready.push({ n, ruler: r('#ruler'), overview: r('#overview'), lanes: r('#lanesScroll'), gl: r('#lanesGL'),
    vw: innerWidth, vh: innerHeight, sh: document.documentElement.scrollHeight, loaded: S.loaded,
    sized: !!document.querySelector('#ruler').style.height });
};
"""


def load(browser, bridge, dpr=2, init=PROBE):
    ctx = browser.new_context(viewport={"width": 1600, "height": 1000}, device_scale_factor=dpr)
    page = ctx.new_page()
    page.add_init_script("localStorage.removeItem('dsg_sid')")
    page.add_init_script(init)
    page.goto(bridge.url + "/")
    page.wait_for_function("() => window.__ready && window.__ready.length > 0", timeout=60000)
    return ctx, page


def test_the_page_says_ready_once_and_its_layout_is_finished(browser, bridge):
    ctx, page = load(browser, bridge)
    try:
        page.wait_for_timeout(300)
        got = page.evaluate("() => window.__ready")
        assert len(got) == 1, f"B.appReady must run once: {got}"
        r = got[0]
        # no canvas takes the box of its backing store: the ruler is one 22 px strip, in the first frame too
        assert r["ruler"]["h"] <= 40, r
        assert r["overview"]["h"] <= 60, r
        assert r["sh"] <= r["vh"], f"the page scrolls: {r}"
        assert page.evaluate("() => window.__DS_READY__") == 1
    finally:
        ctx.close()


def test_no_frame_before_the_first_layout_shows_a_blown_up_ruler(browser, bridge):
    """The cause itself: sample every frame from the start (device pixel ratio 2, as on a Retina display)."""
    probe = PROBE + """
    window.__frames = [];
    let k = 0; const loop = () => { const e = document.querySelector('#ruler');
      if (e) window.__frames.push(Math.round(e.getBoundingClientRect().height));
      if (++k < 30) requestAnimationFrame(loop); };
    requestAnimationFrame(loop);
    """
    ctx, page = load(browser, bridge, init=probe)
    try:
        page.wait_for_function("() => window.__frames.length >= 30", timeout=30000)
        frames = page.evaluate("() => window.__frames")
        assert max(frames) <= 40, f"the ruler was taller than its strip in some frame: {frames}"
    finally:
        ctx.close()
