"""P3-14d (concurrency review Q2): the frame scheduler. Input changes the state and calls invalidate(flags); one
animation frame draws (flushFrame). The frame reads no layout: GEO comes from a ResizeObserver. A hit edit redraws
the hit lines of its track and posts no GL draw. Smooth follow moves the view in every frame (W4). A hit drag
listens on the window only while it runs (W7). The hits of a range come from a binary search (W6).

Two ways to see a forced layout: the Chromium trace (a "Layout" event inside a JavaScript event such as
FireAnimationFrame is a layout that a script forced), and the probe below, which counts the layout reads that run
inside the frame (the self-test uses the same probe in the WKWebView). The frame times are scenario `render` and
`slice` of the self-test (G5)."""
import json

from app_support import open_project
from test_editor import build, exact_events

FRAMES = "n => new Promise(r => { let k = 0; const f = () => ++k >= n ? r() : requestAnimationFrame(f); requestAnimationFrame(f); })"
IDLE = "() => !FR.raf && FR.flags === 0 && (!GLQ.eng || GLQ.eng.stats.drawn >= GLQ.eng.stats.posted)"

# The layout reads inside the frame (FR.inFrame): every getter and method that makes the browser lay out a dirty page.
PROBE = """() => {
  if (window.__reads) { window.__reads.n = 0; window.__reads.where = []; return; }
  const R = window.__reads = { n: 0, where: [] };
  const hit = p => { if (FR.inFrame) { R.n++; if (R.where.length < 8) R.where.push(p + (new Error().stack.split("\\n")[3] || "")); } };
  const props = [[Element.prototype, ["clientWidth", "clientHeight", "clientLeft", "clientTop", "scrollTop", "scrollLeft", "scrollWidth", "scrollHeight"]],
                 [HTMLElement.prototype, ["offsetWidth", "offsetHeight", "offsetLeft", "offsetTop", "offsetParent", "innerText"]],
                 [MouseEvent.prototype, ["offsetX", "offsetY"]]];
  for (const [proto, names] of props) for (const p of names) {
    const d = Object.getOwnPropertyDescriptor(proto, p);
    Object.defineProperty(proto, p, Object.assign({}, d, { get() { hit(p); return d.get.call(this); } }));
  }
  for (const m of ["getBoundingClientRect", "getClientRects"]) {
    const f = Element.prototype[m];
    Element.prototype[m] = function () { hit(m); return f.apply(this, arguments); };
  }
  const gcs = window.getComputedStyle;
  window.getComputedStyle = function () { hit("getComputedStyle"); return gcs.apply(this, arguments); };
}"""
READS = "() => window.__reads"

# W7: the live listeners of `window` (type, capture, function), counted by wrapping add and remove
LISTENERS = """(() => {
  const live = new Map(), add = window.addEventListener, rem = window.removeEventListener;
  const key = (t, o) => t + "|" + (typeof o === "boolean" ? o : !!(o && o.capture));
  window.addEventListener = function (t, f, o) { if (f) { const s = live.get(f) || new Set(); s.add(key(t, o)); live.set(f, s); } return add.apply(this, arguments); };
  window.removeEventListener = function (t, f, o) { const s = live.get(f); if (s) { s.delete(key(t, o)); if (!s.size) live.delete(f); } return rem.apply(this, arguments); };
  window.__windowListeners = () => { let n = 0; for (const s of live.values()) n += s.size; return n; };
})()"""

# a zoom gesture as the self-test runs it: one setView per animation frame
SWEEP = """async n => { const full = maxDur();
  for (let i = 0; i < n; i++) { await new Promise(r => requestAnimationFrame(r));
    const k = 0.5 + 0.5 * Math.sin(i / 5), s = Math.max(0.05, full * Math.pow(k, 6)); setView(full / 2 - s / 2, full / 2 + s / 2); }
  await new Promise(r => requestAnimationFrame(r)); await new Promise(r => requestAnimationFrame(r)); }"""

JS_EVENTS = ("FireAnimationFrame", "FunctionCall", "EventDispatch", "TimerFire", "EvaluateScript", "v8.callFunction")


def forced_layouts(trace):
    """The Layout events of a Chromium trace that ran inside a JavaScript event of the same thread."""
    data = json.loads(trace)
    ev = data["traceEvents"] if isinstance(data, dict) else data
    js = [e for e in ev if e.get("ph") == "X" and e.get("name") in JS_EVENTS]
    lay = [e for e in ev if e.get("ph") == "X" and e.get("name") == "Layout"]
    inside = lambda l, j: j["tid"] == l["tid"] and j["ts"] <= l["ts"] and l["ts"] + l.get("dur", 0) <= j["ts"] + j.get("dur", 0)
    return len(lay), [sorted({j["name"] for j in js if inside(l, j)}) for l in lay if any(inside(l, j) for j in js)]


def settle(app):
    app.evaluate(FRAMES, 3)
    app.wait_for_function(IDLE)


def test_a_zoom_sweep_forces_no_layout(app, make_project, browser):
    """60 frames of zoom on 6 lanes: the trace has no layout inside JavaScript (the code before P3-14d had one per
    frame: glDraw wrote the GL canvas style and then read the lanes' rectangles), and the frame read no layout."""
    open_project(app, make_project(tracks=tuple((f"T{i}", "kick", 36) for i in range(6)), seconds=12.0))
    settle(app)
    app.evaluate(PROBE)
    v0 = app.evaluate("() => FR.n.view")
    browser.start_tracing(page=app, categories=["devtools.timeline"])
    try:
        app.evaluate(SWEEP, 60)
    finally:
        trace = browser.stop_tracing()
    layouts, forced = forced_layouts(trace)
    assert layouts > 0, "the trace has no Layout event at all: the check cannot see one"
    assert forced == [], f"{len(forced)} of {layouts} layouts ran inside JavaScript: {forced[:5]}"
    reads = app.evaluate(READS)
    assert reads["n"] == 0, reads
    assert 55 <= app.evaluate("() => FR.n.view") - v0 <= 62   # one draw per frame


def test_ten_hit_edits_redraw_their_track_and_post_no_gl_draw(app, make_project, browser):
    """Add (double click, Option-click, at the playhead), move (drag, nudge), delete (right click, Delete) and undo:
    10 edits on one track. The GL engine gets no draw, only that track's lanes are drawn, the frames read no layout and
    the trace has no layout inside JavaScript. The new hits are on the spike lane (pixels)."""
    open_project(app, make_project(tracks=(("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Tom 1", "tom", 48)), seconds=8.0))
    app.evaluate("() => { S.selTrack = 'Kick In'; setView(0, 8); }")
    settle(app)
    app.evaluate(PROBE)
    app.evaluate("() => { window.__drawn = []; const ds = window.drawSpike; window.drawSpike = (c, n) => { window.__drawn.push(n); return ds(c, n); }; }")
    p0, n0 = app.evaluate("() => [GLQ.eng.stats.posted, Object.assign({}, FR.n)]")
    box = app.evaluate("() => { const r = document.querySelector(\".track[data-name='Kick In'] canvas.spike\").getBoundingClientRect(); return [r.left, r.top, r.width]; }")
    x_of = lambda t: box[0] + t / 8.0 * box[2]
    y = box[1] + 12
    n_ev = lambda: app.evaluate("() => S.events['Kick In'].length")
    browser.start_tracing(page=app, categories=["devtools.timeline"])
    try:
        n = n_ev()
        app.mouse.dblclick(x_of(4.0), y); settle(app)                            # 1 add
        app.keyboard.down("Alt"); app.mouse.click(x_of(5.0), y); app.keyboard.up("Alt"); settle(app)   # 2 Option-click adds
        assert n_ev() == n + 2
        app.mouse.move(x_of(4.0), y); app.mouse.down(); app.mouse.move(x_of(4.4), y, steps=4); app.mouse.up(); settle(app)   # 3 drag
        app.keyboard.press("NumpadAdd"); settle(app)                             # 4 nudge later
        app.keyboard.press("NumpadSubtract"); settle(app)                        # 5 nudge earlier
        app.mouse.click(x_of(5.0), y, button="right"); settle(app)               # 6 delete by right click
        app.evaluate("() => { S.playhead = 6.0; addHitAtPlayhead(); }")
        settle(app)                                                              # 7 add at the playhead
        app.evaluate("() => undo()"); settle(app)                               # 8 undo
        app.evaluate("() => redo()"); settle(app)                               # 9 redo
        app.evaluate("() => deleteSelectedHit()"); settle(app)                  # 10 delete the selected hit
    finally:
        trace = browser.stop_tracing()
    assert n_ev() == n + 1, (n, n_ev())   # +2, -1, +1, -1 +1 (undo, redo), -1
    p1, n1, drawn = app.evaluate("() => [GLQ.eng.stats.posted, Object.assign({}, FR.n), window.__drawn]")
    assert p1 == p0, f"{p1 - p0} GL draws for 10 hit edits"
    assert n1["view"] == n0["view"] and n1["gl"] == n0["gl"], (n0, n1)
    assert n1["tracks"] - n0["tracks"] >= 10, (n0, n1)
    assert set(drawn) == {"Kick In"}, drawn
    reads = app.evaluate(READS)
    assert reads["n"] == 0, reads
    layouts, forced = forced_layouts(trace)
    assert forced == [], f"{len(forced)} of {layouts} layouts ran inside JavaScript: {forced[:5]}"
    # the hand-added hit at 4.4 s (moved there by the drag) is drawn on the spike lane in the manual colour
    px = app.evaluate("""x => { const c = document.querySelector(".track[data-name='Kick In'] canvas.spike"), d = devicePixelRatio;
        const im = c.getContext('2d').getImageData(Math.round(x * d) - 2 * d, 10 * d, 4 * d, 1).data; let k = 0;
        for (let i = 0; i < im.length; i += 4) if (im[i] > 200 && im[i + 1] < 120 && im[i + 2] > 180) k++; return k; }""",
                      4.4 / 8.0 * box[2])
    assert px > 0


def test_window_listeners_stay_the_same_after_three_project_opens(app, make_project, tmp_path):
    """W7: before P3-14d every track added a window mouseup listener at each open and never removed it."""
    app.add_init_script(LISTENERS)
    app.reload()
    app.wait_for_function("typeof S !== 'undefined' && S.loaded > 0")
    ev = {"Kick In": exact_events()}
    a = build(make_project, tmp_path, events=ev, name="A")
    b = build(make_project, tmp_path, tracks=(("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Tom 1", "tom", 48)), events=ev, name="B")
    counts = []
    for folder in (a, b, a):
        open_project(app, folder)
        settle(app)
        # a hit drag and a pan add their listeners and remove them again
        box = app.evaluate("() => { const r = document.querySelector('#lanesScroll canvas.spike').getBoundingClientRect(); return [r.left, r.top, r.width]; }")
        t0 = app.evaluate("() => (S.events[S.order[0]][0].t - S.view.t0) / (S.view.t1 - S.view.t0)")
        x, y = box[0] + t0 * box[2], box[1] + 12
        app.mouse.move(x, y); app.mouse.down(); app.mouse.move(x + 20, y, steps=3); app.mouse.up()
        app.mouse.move(box[0] + box[2] * 0.9, y + 60); app.mouse.down(); app.mouse.move(box[0] + box[2] * 0.8, y + 60, steps=3); app.mouse.up()
        settle(app)
        assert app.evaluate("() => S.undo.length") == 1   # the drag moved the hit
        app.evaluate("async () => { await saveProjectNow(); }")   # clean, so the next open does not ask
        counts.append(app.evaluate("() => window.__windowListeners()"))
    assert counts[0] > 0 and counts[0] == counts[1] == counts[2], counts


def test_twenty_input_events_in_one_task_draw_once(app, make_project):
    """W2: 20 wheel pans in one task change the view 20 times and draw one frame (one GL draw)."""
    open_project(app, make_project(seconds=12.0))
    app.evaluate("() => setView(0, 8)")
    settle(app)
    v0, p0, t0 = app.evaluate("() => [FR.n.view, GLQ.eng.stats.posted, S.view.t0]")
    app.evaluate("""() => { const c = document.querySelector('#lanesScroll canvas.wave');
        for (let i = 0; i < 20; i++) c.dispatchEvent(new WheelEvent('wheel', { deltaX: 10, deltaY: 0, bubbles: true, cancelable: true })); }""")
    settle(app)
    v1, p1, t1 = app.evaluate("() => [FR.n.view, GLQ.eng.stats.posted, S.view.t0]")
    assert t1 > t0
    assert v1 - v0 == 1 and p1 - p0 == 1, (v1 - v0, p1 - p0)


def test_smooth_follow_moves_the_view_in_every_frame(app, make_project):
    """W4: with a 8 s view (the glide path of the frame loop: strip and compositor playhead off; P4-39b: a 2 s view no
    longer glides, it flips pages) the playhead moves about 2 px per frame; the view moves with it in every drawn frame
    and the playhead stays at a quarter of the view (before: every second frame, in 2 px steps at about 33 Hz).

    #172 race: the test waited for 40 rAF samples by time and required the view to move in 90 % of the frames where the
    playhead moved. On a loaded host two rAF ticks can land in one 16 ms window (the 60 Hz cap skips the draw), so
    frames without a view draw counted against the 90 % (35 of 39). Now the test counts the drawn frames: the view must
    move in EVERY frame that drew one (FR.n.view), and the draws count against the measured frame rate (the cap allows
    one draw per 16 ms of wall time); the playhead lag bound is per frame too (2 px per 16 ms since the last draw)."""
    open_project(app, make_project(seconds=12.0))
    app.evaluate("() => { window.__DS_NO_STRIP__ = true; window.__DS_NO_CP__ = true; setFollow('smooth'); S.playhead = 0; setView(0, 8); }")
    settle(app)
    app.click("#playBtn")
    app.wait_for_function("() => S.playing && AC.running && S.playhead > 2.7", timeout=30000)
    rec = app.evaluate("""n => new Promise(r => { const out = []; const f = () => {
        out.push([S.view.t0, S.playhead, FR.n.view, performance.now()]); out.length >= n ? r(out) : requestAnimationFrame(f); };
        requestAnimationFrame(f); })""", 40)
    app.click("#playBtn")
    app.wait_for_function("() => !S.playing")
    pairs = [(b[0] != a[0], b[2] > a[2]) for a, b in zip(rec, rec[1:]) if b[1] != a[1]]
    assert len(pairs) >= 20, rec
    drawn = [m for m, d in pairs if d]
    assert drawn and all(drawn), f"the view did not move in {drawn.count(False)} of {len(drawn)} drawn frames"
    ms = rec[-1][3] - rec[0][3]
    assert len(drawn) >= 0.8 * min(len(pairs), ms / 16.0), (len(drawn), len(pairs), ms)   # the 60 Hz cap vs the measured rate
    lw = app.evaluate("() => laneWidth()")
    last_draw = rec[0][3]
    worst = 0.0
    for a, b in zip(rec, rec[1:]):
        if b[2] > a[2]:
            last_draw = b[3]
        worst = max(worst, abs((b[1] - b[0]) - 2.0) / 8.0 * lw - 2.0 * (b[3] - last_draw) / 16.0)
    assert worst <= 1.0, f"the playhead left its place by {worst:.2f} px plus one frame of lag"


def test_hits_in_a_range_equal_a_full_scan_also_during_a_drag(app):
    """W6: hitsIn (binary search) gives the same indices as a scan of every hit, for sorted hits and for hits where one
    (the dragged one) is out of order."""
    bad = app.evaluate("""() => {
      let seed = 7; const rnd = () => (seed = (seed * 16807) % 2147483647) / 2147483647, bad = [];
      for (let trial = 0; trial < 400; trial++) {
        const n = Math.floor(rnd() * 40), evs = [];
        for (let i = 0; i < n; i++) evs.push({ t: Math.round(rnd() * 100) / 10 });
        evs.sort((a, b) => a.t - b.t);
        let skip = -1;
        if (n && trial % 2) { skip = Math.floor(rnd() * n); evs[skip].t = Math.round(rnd() * 100) / 10; }
        const t0 = rnd() * 10, t1 = t0 + rnd() * 4;
        const want = evs.map((e, i) => i).filter(i => evs[i].t >= t0 && evs[i].t <= t1);
        const got = hitsIn(evs, t0, t1, skip);
        if (JSON.stringify(got) !== JSON.stringify(want)) bad.push({ t: evs.map(e => e.t), skip, t0, t1, want, got });
      }
      return bad.slice(0, 3); }""")
    assert bad == [], bad


def test_the_geometry_follows_a_resize_the_inspector_and_a_reorder(app, make_project):
    """GEO (from the ResizeObserver) equals the layout after a window resize, a shown and hidden Inspector (P3-14a: the
    editor loses its width) and a new track order; the GL lanes are drawn at those places."""
    open_project(app, make_project(tracks=(("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Tom 1", "tom", 48))))
    check = """() => { const sc = document.querySelector('#lanesScroll'), sr = sc.getBoundingClientRect(), y0 = sc.scrollTop - sr.top - sc.clientTop, bad = [];
      for (const n of S.order) { const w = document.querySelector(`.track[data-name='${n}'] canvas.wave`), g = GEO.rows[n];
        if (!g || Math.abs(g.wave - (w.getBoundingClientRect().top + y0)) > 0.5) bad.push([n, g && g.wave, w.getBoundingClientRect().top + y0]);
        if (w.clientWidth !== GEO.laneW) bad.push([n, 'width', w.clientWidth, GEO.laneW]); }
      if (GEO.scW !== sc.clientWidth || GEO.scH !== sc.clientHeight) bad.push(['scroll', GEO.scW, sc.clientWidth, GEO.scH, sc.clientHeight]);
      return bad; }"""
    settle(app)
    assert app.evaluate(check) == []
    app.set_viewport_size({"width": 1000, "height": 640})
    settle(app)
    assert app.evaluate(check) == []
    for on in (False, True):                                            # P3-14a: the Inspector takes 260 px from the editor
        app.evaluate("on => setInspector(on)", on)
        settle(app)
        assert app.evaluate(check) == [], on
    app.evaluate("() => applyOrder(S.order.slice().reverse())")
    settle(app)
    assert app.evaluate(check) == []


CAP = """() => {   // P4-40c: one synchronous run with synthetic frame times; the browser's own frames cannot interleave
  const view = () => FR.n.view, out = {};
  const ask = () => { invalidate(F.VIEW); cancelAnimationFrame(FR.raf); };   // the request is pending, the browser frame is not
  cancelAnimationFrame(FR.raf); FR.raf = 0; FR.flags = 0; FR.last = 0;
  let v = view(), s = FR.n.skipped;
  ask(); onFrame(1000); out.first = view() - v;                                  // draws
  v = view(); ask(); onFrame(1008); out.at8 = view() - v; out.pending = !!FR.raf; out.flagKept = FR.flags === F.VIEW;
  cancelAnimationFrame(FR.raf); FR.raf = 0; out.skipped = FR.n.skipped - s;     // the second request is still pending
  v = view(); onFrame(1020); out.at20 = view() - v; out.pendingAfter = !!FR.raf;   // the third frame serves it
  v = view(); ask(); onFrame(1030); out.at10 = view() - v; cancelAnimationFrame(FR.raf); FR.raf = 0; FR.flags = 0;
  return out;
}"""


def test_two_frame_requests_8_ms_apart_draw_once_and_a_third_after_20_ms_draws_again(app, make_project):
    """P4-40c: the 60 Hz cap. A frame less than 16 ms after the last drawn one waits (the request stays pending);
    one 20 ms after draws. Planted fault: without the cap the 8 ms frame draws and `at8` is 1."""
    open_project(app, make_project(tracks=(("T0", "kick", 36),), seconds=6.0))
    settle(app)
    r = app.evaluate(CAP)
    assert r == {"first": 1, "at8": 0, "pending": True, "flagKept": True, "skipped": 1, "at20": 1, "pendingAfter": False, "at10": 0}, r


def test_a_scripted_scrub_draws_at_most_every_16_ms(app, make_project):
    """P4-40c: a view moved in every animation frame redraws at most once per 16 ms, and still ends on the last view."""
    open_project(app, make_project(tracks=tuple((f"T{i}", "kick", 36) for i in range(3)), seconds=12.0))
    settle(app)
    r = app.evaluate("""async () => { const full = maxDur(), times = [];
      const v0 = FR.n.view, t0 = performance.now(); let end = 0;
      for (let i = 0; i < 90; i++) { await new Promise(r => requestAnimationFrame(r));
        const a = full * (0.1 + 0.002 * i); setView(a, a + 2); end = a; }
      await new Promise(r => requestAnimationFrame(r)); await new Promise(r => requestAnimationFrame(r));
      return { draws: FR.n.view - v0, ms: performance.now() - t0, t0: S.view.t0, end }; }""")
    assert r["draws"] <= r["ms"] / 16 + 2, r
    assert abs(r["t0"] - r["end"]) < 1e-6, r
