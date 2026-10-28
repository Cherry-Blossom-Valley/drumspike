"""P3-6a: WebGL2 lanes. The page draws the wave lanes with static/lanes-gl.js (one draw call per lane);
the Canvas 2D path stays as the fallback. P3-6b: in the app the engine runs in a worker (tiles-worker.js,
OffscreenCanvas); the page holds raw tiles and sample windows only in the engine, so `GLQ.eng` answers
its counters one message later (the tests wait for them). Chromium runs WebGL2 through SwiftShader here, so these tests
check the drawing (agreement with Canvas 2D, the fallback paths); the frame times are scenario `render`
of the self-test in the real WKWebView (G5)."""
import base64
import time

import pytest

from app_support import open_project

# Per CSS column, the first and last row that differs from the lane background, for the lane canvas of a track.
EXTENTS = """async ([name, thr]) => {
  const im = await laneImage(name), d = im.data, W = im.width, H = im.height, out = [];
  const near = (i, rgb, tol) => Math.abs(d[i] - rgb[0]) + Math.abs(d[i+1] - rgb[1]) + Math.abs(d[i+2] - rgb[2]) < tol;
  const body = S.tracks[name].color.match(/[0-9a-f]{2}/gi).map(h => parseInt(h, 16));
  for (let x = 0; x < W; x++) {
    let top = -1, bot = -1, rt = -1, rb = -1;
    for (let y = 0; y < H; y++) {
      const i = (y * W + x) * 4;
      if (!near(i, [16, 19, 24], thr)) { if (top < 0) top = y; bot = y; }
      if (near(i, body, 40)) { if (rt < 0) rt = y; rb = y; }
    }
    out.push([top, bot, rt, rb]);
  }
  return out;
}"""

# the number of drawn pixels of every wave lane, as the user sees it
DRAWN = """async () => { const out = []; for (const n of S.order) { const d = (await laneImage(n)).data; let k = 0; for (let i = 3; i < d.length; i += 4) if (d[i]) k++; out.push(k); } return out; }"""
ARRAYS = "() => S.order.every(n => !S.peaks[n] || !S.peaks[n].tiles)"      # the Canvas 2D path has loaded its arrays
SETTLE = "() => new Promise(r => setTimeout(r, 120))"


def draw(page, gl, name, thr=24):
    page.evaluate("on => setLanesGL(on)", gl)
    page.evaluate(SETTLE)
    return page.evaluate(EXTENTS, [name, thr])


def _union(cols, x):
    near = [c for c in cols[max(0, x - 1):x + 2] if c[0] >= 0]
    return (min(c[0] for c in near), max(c[1] for c in near)) if near else None


def agreement(a, b, tiles, tol=1.5):
    """Share of columns that agree. Peak tiles: the envelope and the solid RMS body extents agree within tol
    device pixels. Sample line: Canvas 2D smears the steep parts of an anti-aliased line over the next column,
    so each column's extent must lie inside the other renderer's extent over x-1..x+1 (within tol)."""
    ok = 0
    for x, (ca, cb) in enumerate(zip(a, b)):
        if tiles:
            ok += all((u < 0 and v < 0) or (u >= 0 and v >= 0 and abs(u - v) <= tol) for u, v in zip(ca, cb))
            continue
        ua, ub = _union(a, x), _union(b, x)
        inside = lambda c, u: c[0] < 0 or (u is not None and c[0] >= u[0] - tol and c[1] <= u[1] + tol)
        ok += inside(ca, ub) and inside(cb, ua)
    return ok / len(a)


def test_wave_lanes_are_drawn_by_webgl2(app, make_project):
    open_project(app, make_project())
    app.wait_for_function("() => GLQ.eng && GLQ.eng.stats.draws >= 1 && GLQ.eng.stats.lanes >= 2")
    assert app.evaluate("() => GLQ.eng.remote === true") is True      # P3-6b: the worker draws
    assert app.evaluate("() => GLQ.eng.info().renderer") != ""
    # one draw call per lane: the engine counts lanes, the page has two
    assert app.evaluate("() => S.order.length") == 2


@pytest.mark.parametrize("view", [(0.0, 4.0), (0.3, 1.1), (0.45, 0.6), (0.4995, 0.5035), (0.5, 0.505)])
def test_webgl2_lane_agrees_with_canvas_2d(app, make_project, view):
    """Same data, same view, both renderers: at least 99 % of the columns agree (tile levels, dense sample
    window, sparse sample line with dots)."""
    open_project(app, make_project())
    app.evaluate("v => setView(v[0], v[1])", list(view))
    # the sample window of a short view comes from samples_window: wait for it
    app.wait_for_function("() => S.order.every(n => S.view.t1 - S.view.t0 > 5.5 || !S.peaks[n] || "
                          "(S.samplesCache[n] && S.samplesCache[n].start <= S.view.t0 * S.peaks[n].sr))")
    for name in app.evaluate("() => S.order"):
        tiles = (view[1] - view[0]) * 48000 > 262144
        gl = draw(app, True, name)
        cv = draw(app, False, name)
        assert any(c[0] >= 0 for c in gl), "the lane is empty"
        share = agreement(gl, cv, tiles)
        print(f"agreement {name} {view}: {share:.4f}")
        assert share >= 0.99, f"{name} {view}: {share:.4f}"
    app.evaluate("() => setLanesGL(true)")


def test_sample_modes_are_used(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => setView(0.45, 0.6)")
    app.wait_for_function("() => GLQ.eng.stats.modes[1] > 0")
    app.evaluate("() => setView(0.5, 0.505)")
    app.wait_for_function("() => GLQ.eng.stats.modes[2] > 0")
    app.evaluate("() => setView(0, 4)")
    app.wait_for_function("() => GLQ.eng.stats.modes[0] >= 2")
    app.wait_for_function("() => GLQ.eng.textures().samples === 2")


def test_event_lines_and_selection_stay_on_the_lane(app, make_project):
    """The 2D event lines (and the selected hit in yellow) are drawn over the WebGL2 lane, as over the 2D lane."""
    open_project(app, make_project())
    app.evaluate("() => { const n = S.order[0]; S.events[n] = [{t: 1.0, v: 100, id: 1}, {t: 2.0, v: 90, id: 2}]; "
                 "S.sel = {track: n, index: 0}; }")
    px = """async () => { redrawAll(); const im = await laneImage(S.order[0]), W = im.width, d = im.data;
        const at = t => { const x = Math.round((t - S.view.t0) / (S.view.t1 - S.view.t0) * W), i = (100 * W + x) * 4; return [d[i], d[i+1], d[i+2]]; };
        return [at(1.0), at(2.0), at(3.0)]; }"""
    gl = app.evaluate(px)
    app.evaluate("() => setLanesGL(false)")
    cv = app.evaluate(px)
    app.evaluate("() => setLanesGL(true)")
    assert gl[0][0] > 100 and gl[0][1] > 90, gl      # the selected hit: yellow #ffe066 over the lane
    assert gl[1] != gl[2] and gl[0] != gl[2]
    assert all(abs(a - b) <= 12 for x, y in zip(gl, cv) for a, b in zip(x, y)), (gl, cv)


def test_fallback_without_webgl2(page, bridge, make_project):
    """window.__DS_NO_GL__ (as a browser without WebGL2): the Canvas 2D lanes draw as before."""
    page.add_init_script("window.__DS_NO_GL__ = true; localStorage.removeItem('dsg_sid')")
    page.goto(bridge.url + "/")
    page.wait_for_function("typeof S !== 'undefined' && typeof B !== 'undefined' && B.app === true && S.loaded > 0")
    open_project(page, make_project())
    assert page.evaluate("() => GLQ.eng") is None
    drawn = page.evaluate(DRAWN)
    assert len(drawn) == 2 and all(n > 0 for n in drawn)
    assert page.evaluate("() => setLanesGL(true)") is False


def test_fallback_when_the_context_is_lost(app, make_project):
    open_project(app, make_project())
    assert app.evaluate("() => GLQ.eng.loseContext()") is True
    app.wait_for_function("() => GLQ.eng === null")
    app.wait_for_function(ARRAYS)          # the Canvas 2D path asks for the arrays of the tracks
    drawn = app.evaluate(DRAWN)
    assert all(n > 0 for n in drawn)


def test_fallback_when_a_draw_fails(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => { GLQ.eng.draw = () => { throw new Error('boom'); }; setView(1, 2); }")
    app.wait_for_function("() => GLQ.eng === null")                     # P3-14d: the next frame draws and falls back
    app.wait_for_function(ARRAYS)
    assert all(n > 0 for n in app.evaluate(DRAWN))


def test_engine_creation_failures_return_null(app):
    """No WebGL2 context, or a shader that does not compile: create() answers null."""
    assert app.evaluate("""() => { const g = HTMLCanvasElement.prototype.getContext;
        HTMLCanvasElement.prototype.getContext = function (t, ...a) { return t === 'webgl2' ? null : g.call(this, t, ...a); };
        const r = LanesGL.create(); HTMLCanvasElement.prototype.getContext = g; return r; }""") is None
    assert app.evaluate("""() => { const g = HTMLCanvasElement.prototype.getContext;
        HTMLCanvasElement.prototype.getContext = function (t, ...a) { if (t === 'webgl2') throw new Error('no'); return g.call(this, t, ...a); };
        const r = LanesGL.create(); HTMLCanvasElement.prototype.getContext = g; return r; }""") is None
    assert app.evaluate("""() => { const g = WebGL2RenderingContext.prototype.getShaderParameter;
        WebGL2RenderingContext.prototype.getShaderParameter = () => false;
        const r = LanesGL.create(); WebGL2RenderingContext.prototype.getShaderParameter = g; return r; }""") is None


def test_textures_follow_the_tracks(app, make_project):
    """Textures of removed tracks are deleted (prune), and the background warm-up uploads every level."""
    open_project(app, make_project())
    app.wait_for_function("() => GLQ.eng.textures().peaks >= 6")
    app.evaluate("() => GLQ.eng.prune([S.order[0]])")
    app.wait_for_function("() => GLQ.eng.textures().peaks === 3")


SHOT = """async ([b64, box, rgb, tol]) => {
  const img = new Image(); img.src = 'data:image/png;base64,' + b64; await img.decode();
  const c = document.createElement('canvas'); c.width = img.width; c.height = img.height;
  const t = c.getContext('2d'); t.drawImage(img, 0, 0);
  const d = t.getImageData(0, 0, c.width, c.height).data, k = img.width / innerWidth;
  let n = 0;
  for (let y = Math.max(0, Math.floor(box.y * k)); y < Math.min(c.height, Math.ceil((box.y + box.height) * k)); y++)
    for (let x = Math.max(0, Math.floor(box.x * k)); x < Math.min(c.width, Math.ceil((box.x + box.width) * k)); x++) {
      const i = (y * c.width + x) * 4;
      if (Math.abs(d[i] - rgb[0]) + Math.abs(d[i+1] - rgb[1]) + Math.abs(d[i+2] - rgb[2]) <= tol) n++;
    }
  return n; }"""


def on_screen(page, name, rgb, tol=30):
    """How many pixels of the lane's box in a screenshot of the page are within tol of rgb (the compositor's
    output: the WebGL2 canvas under the transparent 2D lane)."""
    box = page.evaluate("n => { const r = document.querySelector(`.track[data-name=\"${n}\"] canvas.wave`).getBoundingClientRect(); "
                        "return {x: r.x, y: r.y, width: r.width, height: r.height}; }", name)
    png = base64.b64encode(page.screenshot()).decode()
    return page.evaluate(SHOT, [png, box, rgb, tol])


def on_screen_until(page, name, rgb, tol, ok, timeout=5.0):
    """on_screen, polled until ok(count) is true or timeout seconds pass; returns the last count. A loaded host
    (for example the shared x86 CI runner) can composite a lane a few frames after SETTLE, so a single reading
    races the drawing. A lane that never reaches the state still fails the assert after the timeout."""
    end = time.monotonic() + timeout
    while True:
        n = on_screen(page, name, rgb, tol)
        if ok(n) or time.monotonic() > end:
            return n
        page.wait_for_timeout(100)


def on_screen_soon(page, name, rgb, tol, need, timeout=5.0):
    """on_screen_until the count exceeds need."""
    return on_screen_until(page, name, rgb, tol, lambda n: n > need, timeout)


def test_the_lane_is_on_screen_through_the_gl_canvas(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => setView(0.45, 0.6)")
    app.wait_for_function("() => S.samplesCache[S.order[0]]")
    app.evaluate(SETTLE)
    assert app.evaluate("() => !!document.querySelector('#lanesGL') && getComputedStyle(document.querySelector('#lanesGL')).pointerEvents") == "none"
    assert on_screen_soon(app, "Kick In", [16, 19, 24], 4, 10000) > 10000        # the lane background #101318
    assert on_screen_soon(app, "Kick In", [0xe0, 0x55, 0x55], 60, 100) > 100   # the kick track's colour (the line of the samples)


def test_a_muted_lane_fades_like_the_tlanes_opacity(app, make_project):
    open_project(app, make_project())
    app.evaluate(SETTLE)
    full = on_screen_soon(app, "Kick In", [16, 19, 24], 3, 10000)
    app.click(".track[data-name='Kick In'] .bMute")
    app.evaluate(SETTLE)
    assert on_screen_until(app, "Kick In", [16, 19, 24], 3, lambda n: n == 0) == 0 < full      # 0.35 towards the page colour #14161a: (19, 21, 25)
    assert on_screen_soon(app, "Kick In", [19, 21, 25], 3, full / 2) > full / 2


def test_lanes_follow_a_layout_change(app, make_project):
    """Showing the Inspector (P3-14a) moves the lanes to the right and makes them narrower: the GL lane moves with them."""
    app.set_viewport_size({"width": 1280, "height": 1100})
    open_project(app, make_project())
    app.evaluate("() => { setInspector(false); setView(0.45, 0.6); }")
    app.wait_for_function("() => S.samplesCache[S.order[1]]")
    app.evaluate(SETTLE)
    before = app.evaluate("() => document.querySelector('.track[data-name=\"Snare Top\"] canvas.wave').getBoundingClientRect().x")
    app.evaluate("() => setInspector(true)")
    app.evaluate(SETTLE)
    after = app.evaluate("() => document.querySelector('.track[data-name=\"Snare Top\"] canvas.wave').getBoundingClientRect().x")
    assert after > before + 200, (before, after)
    assert on_screen_soon(app, "Snare Top", [16, 19, 24], 4, 10000) > 10000
    assert on_screen_soon(app, "Snare Top", [0xe0, 0xa1, 0x3c], 60, 100) > 100


def test_scrolling_draws_the_lanes_that_come_into_view(app, make_project):
    """30 lanes: only the visible ones are drawn; a scroll repaints the canvas for the new ones."""
    tracks = tuple((f"T{i:02d}", "kick" if i % 2 else "snare", 36 if i % 2 else 38) for i in range(30))
    open_project(app, make_project(tracks=tracks, seconds=2.0))
    app.evaluate(SETTLE)
    app.wait_for_function("() => GLQ.eng.stats.drawn >= GLQ.eng.stats.posted")
    st = app.evaluate("() => GLQ.eng.stats")
    assert 0 < st["lanes"] / st["draws"] < 30                     # culled: fewer than 30 lanes per pass
    assert on_screen(app, "T29", [16, 19, 24], 4) == 0              # not on screen yet
    app.evaluate("() => { document.querySelector('#lanesScroll').scrollTop = 1e6; }")
    app.evaluate(SETTLE)
    assert on_screen_soon(app, "T29", [16, 19, 24], 4, 10000) > 10000
    assert on_screen_until(app, "T00", [16, 19, 24], 4, lambda n: n == 0) == 0             # the first lane is out of view now
    app.wait_for_function("() => GLQ.eng.stats.drawn >= GLQ.eng.stats.posted")
    n = app.evaluate("""async () => { const e = GLQ.eng, l0 = e.stats.lanes, d0 = e.stats.draws; GLQ.all = true; redrawAll(); GLQ.all = false;
        while (e.stats.drawn < e.stats.posted) await new Promise(r => setTimeout(r, 10));
        return [e.stats.lanes - l0, e.stats.draws - d0]; }""")
    assert n == [30, 1]                                           # all 30 in one pass when asked (the benchmark)
    assert len(app.evaluate(DRAWN)) == 30 and all(k > 0 for k in app.evaluate(DRAWN))


def test_the_cached_overview_equals_the_uncached_one(app, make_project):
    """P3-6a: the tracks of the overview are drawn once into OV and blitted; the pixels equal a fresh drawing,
    and a new gain or new peaks invalidate the cache."""
    open_project(app, make_project())
    px = """() => { const c = document.querySelector('#overview'); return Array.from(c.getContext('2d').getImageData(0, 0, c.width, c.height).data); }"""
    app.evaluate("() => { setView(1, 2); }")
    app.evaluate("() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))")   # P3-14d: drawn in the frame
    cached = app.evaluate(px)
    app.evaluate("() => { OV.canvas = null; drawOverview(); }")
    assert app.evaluate(px) == cached
    assert any(cached[i] != 0x16 for i in range(0, len(cached), 4))     # not just the background
    app.evaluate("() => drawOverview()")
    assert app.evaluate("() => { const a = OV.canvas; drawOverview(); return a === OV.canvas; }") is True   # reused
    app.evaluate("() => { S.peakAmp[S.order[0]] *= 2; drawOverview(); }")
    assert app.evaluate(px) != cached                                                            # the gain changed: redrawn
    app.evaluate("() => { const n = S.order[0], p = S.peaks[n]; S.peaks[n] = JSON.parse(JSON.stringify(p)); drawOverview(); }")
    assert app.evaluate("() => OV.peaks[0] === S.peaks[S.order[0]]") is True                    # new peaks object: redrawn
