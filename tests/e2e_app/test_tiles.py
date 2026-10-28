"""P3-6b: tile worker, no PCM in the UI. In the app the wave lanes are drawn by a worker (static/tiles-worker.js,
an OffscreenCanvas with the WebGL2 engine of static/lanes-gl.js). The page sends it the tile bytes and the sample
windows and keeps no PCM, no peak arrays (except the coarsest level for the overview) and no sample data. The
sample windows sit in a small LRU in the worker. The memory and open-to-drawn numbers are scenario `memory` of
the self-test (G5, the real WebView); these tests check the paths and the fallbacks."""
import pytest

from app_support import invoke, open_project, wait_until

SETTLE = "() => new Promise(r => setTimeout(r, 120))"
HELD = """() => { let n = 0; for (const name of S.order) { const p = S.peaks[name]; if (!p) continue;
    for (const L of Object.values(p.levels)) for (const k of ['min', 'max', 'rms']) if (L[k]) n += L[k].length; }
  return n; }"""
# P4-67: the state of the window pipeline, for the message of a failed wait
WINDOWS = """() => { const e = GLQ.eng, c = S.samplesCache;
  return { eng: !!e, off: !!GLQ.off, lost: !!(e && e.lost), why: e && e.why || null, remote: !!(e && e.remote),
    tex: e && e.textures ? e.textures() : null, frames: Object.assign({}, FR.n), flags: FR.flags, raf: !!FR.raf,
    sp: !!SP.on, playing: !!S.playing, view: S.view, cache: Object.keys(c).length,
    no_window: S.order.filter(n => !c[n]).length, fetched_this_view: Object.values(GLQ.fetched).filter(v => v === S.view).length,
    pend: Object.values(GLQ.pend).reduce((a, b) => a + b, 0), runs: Object.values(SF).filter(f => f.run).length,
    held: GLQ.held, rows: Object.keys(GEO.rows).length, peaks: S.order.filter(n => S.peaks[n]).length,
    stats: e && e.stats ? Object.assign({}, e.stats) : null }; }"""
THIRTY = tuple((f"T{i:02d}", "kick" if i % 2 else "snare", 36 if i % 2 else 38) for i in range(30))


def test_the_page_holds_no_pcm_no_samples_and_no_fine_peaks(app, make_project):
    open_project(app, make_project(tracks=THIRTY, seconds=2.0))
    app.wait_for_function("() => GLQ.eng.textures().peaks === 90")          # 3 levels of 30 tracks, all on the GPU side
    # the page holds the 3 numbers of the coarsest level (65536 samples per bin) of each track, nothing else
    held = app.evaluate(HELD)
    assert 0 < held <= 30 * 3 * 4, held
    app.evaluate("() => setView(0.45, 0.6)")
    app.wait_for_function("() => GLQ.eng.textures().samples >= 1")
    assert app.evaluate("() => Object.values(S.samplesCache).every(c => !('data' in c))") is True
    assert app.evaluate("() => S.pcm === undefined && typeof prefetchPCM === \"undefined\"")   # P3-12: no PCM store in the page
    assert not any(i in ("pcm", "audio") for i in app.invokes)
    assert app.evaluate("() => S.order.every(n => S.peaks[n].tiles === true)") is True


def test_lanes_are_drawn_from_the_tiles_of_the_worker(app, make_project):
    open_project(app, make_project())
    app.wait_for_function("() => GLQ.eng.stats.drawn >= 1 && GLQ.eng.stats.drawn >= GLQ.eng.stats.posted")
    drawn = app.evaluate("""async () => { const out = []; for (const n of S.order) { const d = (await laneImage(n)).data; let k = 0;
        for (let i = 3; i < d.length; i += 4) if (d[i]) k++; out.push(k); } return out; }""")
    assert len(drawn) == 2 and all(n > 1000 for n in drawn)
    assert app.evaluate("() => GLQ.eng.remote") is True
    assert app.evaluate("() => GLQ.loadedSeq") >= 1


def test_open_to_drawn_waits_for_the_engine_to_answer(app, make_project):
    """The self-test measures open to drawn as: S.loaded set and the engine has answered the draw posted before it."""
    open_project(app, make_project())
    app.wait_for_function("() => GLQ.eng.stats.drawn >= GLQ.loadedSeq && GLQ.loadedSeq > 0")


def test_the_window_lru_drops_the_oldest_and_the_page_forgets_its_range(app, make_project):
    """More lanes than the cache holds (16): the worker evicts the least recently used windows and tells the
    page, which fetches a window again only when the view changes."""
    open_project(app, make_project(tracks=THIRTY, seconds=2.0))
    app.evaluate("() => { GLQ.all = true; setView(0.45, 0.6); }")
    app.wait_for_function("() => GLQ.eng.textures().samples === 16 && Object.keys(S.samplesCache).length === 16", timeout=60000)
    app.evaluate(SETTLE)
    assert app.evaluate("() => GLQ.eng.textures().samples") == 16
    assert app.evaluate("() => Object.keys(S.samplesCache).length") == 16
    # P4-44: the answers of the first round can land after the check above (a loaded host): each late answer
    # setWindows again and evicts again, and the evict notices race the second view. If the second view is set
    # while the cache still holds all 30 windows, the next draw fetches nothing (CI 38968041447: 30 calls
    # before and after). Wait for the fetch pipeline to be idle and the evictions to be applied first.
    wait_until(lambda: app.evaluate("() => Object.values(GLQ.pend).reduce((a, b) => a + b, 0) === 0 && Object.values(SF).every(f => !f.run)"),
               timeout=60.0, msg="the first round of windows to settle (no fetch on the way)")
    app.evaluate(SETTLE)
    wait_until(lambda: app.evaluate("() => Object.keys(S.samplesCache).length") == 16,
               timeout=60.0, msg="the evict notices to apply (the cache holds 16 windows again)")
    # a new view fetches the windows again (the evicted lanes show the envelope until then)
    n0 = app.invokes.count("samples_window")
    before, n_all = app.evaluate(WINDOWS), len(app.invokes)
    app.evaluate("() => setView(0.46, 0.61)")
    # P4-42: a loaded host answers late; the wait is a condition with 60 s and says how many windows came
    # P4-67b: app.invokes grows from Playwright request events; wait_until pumps them in (app_support.PUMP)
    try:
        wait_until(lambda: app.invokes.count("samples_window") > n0, timeout=60.0, msg="a second round of windows")
    except AssertionError as e:   # P4-67: the state before the new view and now, so a failed run names the cause
        now = app.evaluate(WINDOWS)   # first: the call delivers the pending events, so the counts below are current
        raise AssertionError(f"{e} (samples_window calls: {app.invokes.count('samples_window')}, before the new view: {n0}); "
                             f"before: {before}; now: {now}; page errors: {app.errors}; "
                             f"invokes since: {app.invokes[n_all:][-20:]}") from None
    app.evaluate("() => { GLQ.all = false; }")


def test_the_engine_in_the_page_takes_over_without_a_worker(page, bridge, make_project):
    """No Worker (as an old WebView): the same engine runs in the page, still with raw tiles and no arrays."""
    page.add_init_script("window.__DS_NO_WORKER__ = true; localStorage.removeItem('dsg_sid')")
    page.goto(bridge.url + "/")
    page.wait_for_function("typeof S !== 'undefined' && typeof B !== 'undefined' && B.app === true && S.loaded > 0")
    open_project(page, make_project())
    assert page.evaluate("() => GLQ.eng.remote") is None
    assert page.evaluate("() => S.order.every(n => S.peaks[n].tiles === true)") is True
    page.evaluate("() => setView(0.45, 0.6)")
    page.wait_for_function("() => GLQ.eng.textures().samples === 2")
    assert page.evaluate("() => Object.values(S.samplesCache).every(c => !('data' in c))") is True
    drawn = page.evaluate("""async () => { const out = []; for (const n of S.order) { const d = (await laneImage(n)).data; let k = 0;
        for (let i = 3; i < d.length; i += 4) if (d[i]) k++; out.push(k); } return out; }""")
    assert all(n > 1000 for n in drawn)


STUB_WORKER = """window.Worker = class { constructor() { this.t = setTimeout(() => this.onmessage && this.onmessage({ data: { op: 'ready', ok: false } }), 20); }
  postMessage() {} terminate() { clearTimeout(this.t); } }; localStorage.removeItem('dsg_sid');"""


def test_a_worker_without_webgl2_hands_over_to_the_engine_in_the_page(page, bridge, make_project):
    """The worker answers 'ready' with ok false (no WebGL2 in workers): the page builds the engine itself and sends the
    tiles again."""
    page.add_init_script(STUB_WORKER)
    page.goto(bridge.url + "/")
    page.wait_for_function("typeof S !== 'undefined' && typeof B !== 'undefined' && B.app === true && S.loaded > 0")
    open_project(page, make_project())
    page.wait_for_function("() => GLQ.eng && !GLQ.eng.remote && GLQ.eng.textures().peaks === 6")
    page.evaluate("() => redrawAll()")
    drawn = page.evaluate("""async () => { const out = []; for (const n of S.order) { const d = (await laneImage(n)).data; let k = 0;
        for (let i = 3; i < d.length; i += 4) if (d[i]) k++; out.push(k); } return out; }""")
    assert all(n > 1000 for n in drawn)


def test_a_page_without_webgl2_anywhere_keeps_the_arrays(page, bridge, make_project):
    """No GL at all: the page loads the arrays as before and draws with Canvas 2D."""
    page.add_init_script("window.__DS_NO_GL__ = true; localStorage.removeItem('dsg_sid')")
    page.goto(bridge.url + "/")
    page.wait_for_function("typeof S !== 'undefined' && typeof B !== 'undefined' && B.app === true && S.loaded > 0")
    open_project(page, make_project())
    assert page.evaluate("() => S.order.every(n => !S.peaks[n].tiles && Array.isArray(S.peaks[n].levels[256].min))") is True
    page.evaluate("() => setView(0.45, 0.6)")
    page.wait_for_function("() => S.order.every(n => S.samplesCache[n] && S.samplesCache[n].data instanceof Float32Array)")


def test_canvas_2d_fallback_loads_the_arrays_when_asked(app, make_project):
    open_project(app, make_project())
    assert app.evaluate("() => S.peaks[S.order[0]].tiles") is True
    assert app.evaluate("() => setLanesGL(false)") is False
    assert app.evaluate("() => S.order.every(n => !S.peaks[n].tiles && S.peaks[n].levels[256].min.length === S.peaks[n].levels[256].n)") is True
    n0 = app.invokes.count("peaks_tile")
    app.evaluate("() => setView(0, 3)")
    app.evaluate(SETTLE)
    assert app.invokes.count("peaks_tile") == n0                       # loaded once
    assert app.evaluate("() => setLanesGL(true)") is True
    app.evaluate("() => setView(0.45, 0.6)")                           # sample windows after a Canvas 2D period: from the engine again
    app.wait_for_function("() => GLQ.eng.textures().samples === 2")


def test_a_lost_context_in_the_worker_falls_back_and_reloads_the_arrays(app, make_project):
    open_project(app, make_project())
    assert app.evaluate("() => GLQ.eng.loseContext()") is True
    app.wait_for_function("() => GLQ.eng === null")
    app.wait_for_function("() => S.order.every(n => !S.peaks[n].tiles)")
    assert app.evaluate("() => document.querySelector('#lanesGL')") is None


def test_the_worker_engine_reports_an_error_of_the_worker(app):
    """A worker script that cannot load: onLost gets the message, draws answer false, reads answer null."""
    res = app.evaluate("""async () => {
        let why = null;
        const e = LanesGL.remote({ url: '/no-such-worker.js', onLost: w => { why = w; } });
        for (let i = 0; i < 100 && !why; i++) await new Promise(r => setTimeout(r, 20));
        return [!!why, e.lost, e.draw([]), await e.readLane({ dw: 4, dh: 4, key: 'x' }), e.info().renderer, await e.loseContext() ];
    }""")
    assert res == [True, True, False, None, "", False]


def test_the_worker_host_runs_in_a_page(page, bridge):
    """tiles-worker.js and LanesGL.host loaded in a page with a stand-in scope: every message of the page side."""
    page.goto(bridge.url + "/")
    page.wait_for_function("typeof LanesGL !== 'undefined'")
    page.add_script_tag(url=bridge.url + "/tiles-worker.js?v=1")
    res = page.evaluate("""async () => {
        const sent = [], scope = { LanesGL: window.LanesGL, importScripts() {}, postMessage: (m, t) => sent.push(m), location: { search: '' } };
        const handle = window.__tilesWorkerBoot(scope);
        scope.onmessage({ data: { op: 'tiles', key: 'ignored before init', raw: {} } });
        const cv = document.createElement('canvas');
        scope.onmessage({ data: { op: 'init', canvas: cv, maxWindows: 2 } });
        const tile = (n, v) => { const b = new ArrayBuffer(12 + 12 * n), u = new Uint32Array(b, 0, 3); u[0] = 256; u[1] = 0; u[2] = n;
          const f = new Float32Array(b, 12); for (let i = 0; i < n; i++) { f[i] = -v; f[n + i] = v; f[2 * n + i] = v / 2; } return b; };
        handle({ op: 'tiles', key: 'a', raw: { 256: tile(100, 0.5), 4096: tile(10, 0.5) } });
        handle({ op: 'tiles', key: 'b', raw: { 256: tile(100, 0.25) } });
        await new Promise(r => setTimeout(r, 100));
        const lane = (key, extra) => Object.assign({ key, x: 0, y: 0, dw: 64, dh: 32, W: 64, H: 32, fade: 1, color: '#e05555', amp: 1, span0: 0, spanS: 25600, samples: null }, extra || {});
        handle({ op: 'draw', seq: 1, w: 64, h: 64, lanes: [lane('a'), lane('b', { y: 32 }), lane('missing')] });
        const w = new Float32Array(2000); for (let i = 0; i < w.length; i++) w[i] = Math.sin(i / 20);
        handle({ op: 'window', key: 'a', start: 100, data: w });
        handle({ op: 'window', key: 'b', start: 0, data: w.slice() });
        handle({ op: 'window', key: 'c', start: 0, data: w.slice() });      // third window with a cache of two: 'a' goes
        handle({ op: 'draw', seq: 2, w: 64, h: 64, lanes: [lane('b', { span0: 0, spanS: 1000, samples: { start: 0, end: 2000 } }), lane('c', { y: 32, span0: 0, spanS: 100, samples: { start: 0, end: 2000 } })] });
        handle({ op: 'read', id: 7, w: 64, h: 64, lane: lane('b') });
        handle({ op: 'info', id: 8 });
        handle({ op: 'prune', keys: ['a', 'b'] });
        handle({ op: 'lose', id: 9 });
        await new Promise(r => setTimeout(r, 50));
        return sent.map(m => ({ op: m.op, id: m.id, seq: m.seq, ok: m.ok, key: m.key, tex: m.textures, w: m.width, bytes: m.data && m.data.byteLength, draws: m.stats && m.stats.draws }));
    }""")
    ops = [m["op"] for m in res]
    assert ops[0] == "ready" and res[0]["ok"] is True
    assert "evicted" in ops and [m["key"] for m in res if m["op"] == "evicted"] == ["a"]
    assert [m["seq"] for m in res if m["op"] == "drawn"] == [1, 2]
    rd = next(m for m in res if m["op"] == "read")
    assert rd["id"] == 7 and rd["w"] == 64 and rd["bytes"] == 64 * 32 * 4
    assert next(m for m in res if m["op"] == "info")["id"] == 8
    assert next(m for m in res if m["op"] == "lose")["ok"] is True
    assert res[-1]["op"] in ("lose", "lost", "textures")
    assert any(m["op"] == "textures" and m["tex"]["peaks"] >= 3 for m in res)


def test_the_worker_host_reports_a_failed_draw_and_an_init_without_webgl2(page, bridge):
    page.goto(bridge.url + "/")
    page.wait_for_function("typeof LanesGL !== 'undefined'")
    res = page.evaluate("""async () => {
        const sent = [], post = m => sent.push(m);
        const g = HTMLCanvasElement.prototype.getContext;
        HTMLCanvasElement.prototype.getContext = function (t, ...a) { return t === 'webgl2' ? null : g.call(this, t, ...a); };
        const h1 = LanesGL.host(post);
        h1({ op: 'init', canvas: document.createElement('canvas') });
        h1({ op: 'draw', seq: 1, w: 8, h: 8, lanes: [] });          // ignored: no engine
        HTMLCanvasElement.prototype.getContext = g;
        const h2 = LanesGL.host(post);
        h2({ op: 'init', canvas: document.createElement('canvas') });
        h2({ op: 'draw', seq: 2, w: 8, h: 8, lanes: null });         // a draw that throws
        h2({ op: 'read', id: 3, w: 8, h: 8, lane: null });           // a read that throws
        return sent.map(m => [m.op, m.ok, m.id, m.error ? 'error' : '', m.width === null]);
    }""")
    assert res[0][:2] == ["ready", False]
    assert res[1][:2] == ["ready", True]
    assert res[2][0] == "failed" and res[2][3] == "error"
    assert res[3][0] == "read" and res[3][2] == 3 and res[3][4] is True
    assert len(res) == 4


def test_only_lanes_near_the_view_keep_a_canvas_store(app, make_project):
    """P3-6b: 30 lanes, 6 on screen. The others hold a 1 x 1 canvas (the store of a 2D canvas costs graphics memory
    in the WebContent process); a scroll wakes the lanes that come near and puts the ones that left to sleep. The
    WebGL2 lane is still read back at full size for every lane."""
    open_project(app, make_project(tracks=THIRTY, seconds=2.0))
    app.evaluate(SETTLE)
    stores = "() => S.order.map(n => document.querySelector(`.track[data-name='${n}'] canvas.wave`).width > 1)"
    first = app.evaluate(stores)
    assert 3 <= sum(first) < 15 and first[0] is True and first[-1] is False, first
    assert app.evaluate("() => document.querySelectorAll('#lanesScroll canvas.spike').length") == 30
    app.evaluate("() => { document.querySelector('#lanesScroll').scrollTop = 1e6; }")
    app.wait_for_function("() => document.querySelector(\".track[data-name='T29'] canvas.wave\").width > 1")
    last = app.evaluate(stores)
    assert last[-1] is True and last[0] is False, last
    # a sleeping lane still reads back its GL pixels at full size; the awake lanes have event lines on top
    n = app.evaluate("""async () => { const im = await laneImage('T00'); let k = 0; for (let i = 3; i < im.data.length; i += 4) if (im.data[i]) k++;
        return [im.width === Math.floor(GEO.laneW * devicePixelRatio) && im.width >= 300, im.height > 100, k > 1000]; }""")   # P3-14a: the lane is narrower with the Inspector shown
    assert n == [True, True, True]
    # the stores come back at the size of a lane (device pixels) and the event lines are drawn again
    app.evaluate("() => { S.events['T29'] = [{t: 1.0, v: 100, id: 1}]; redrawAll(); }")
    sz = app.evaluate("() => { const c = document.querySelector(\".track[data-name='T29'] canvas.spike\"); return [c.width, c.height, c.clientWidth]; }")
    assert sz[0] == sz[2] * app.evaluate("() => devicePixelRatio") and sz[1] > 1


def test_a_closed_project_gives_its_canvases_back(app, make_project):
    open_project(app, make_project())
    held = app.evaluate("() => { window.__kept = [...document.querySelectorAll('#lanesScroll canvas')]; return window.__kept.length; }")
    # P4-40f: the velocity lane owns a canvas per track while it is on; P4-40h: the meter owns one canvas per row
    assert held == app.evaluate("() => 2 * (3 + (VL.on ? 1 : 0))"), held
    app.evaluate("() => { buildTrackDOM(); }")
    assert app.evaluate("() => window.__kept.every(c => c.width === 0 && c.height === 0)") is True


def test_a_stale_draw_after_the_close_does_not_resize_a_released_canvas(app, make_project):
    """#110: a draw that was queued before the close (a wheel, an edit, or the final ResizeObserver
    delivery of a removed meter box) runs after buildTrackDOM under load. It must not size a canvas
    that left the DOM: its backing store was released, and a late 1 x 1 write brings it back."""
    open_project(app, make_project())
    held = app.evaluate("() => { window.__kept = [...document.querySelectorAll('#lanesScroll canvas')]; return window.__kept.length; }")
    app.evaluate("""async () => {
      invalidate(F.SIZE | F.VIEW | F.GL | F.METER);   // a draw queued before the close, like a wheel in the same frame
      invalidateHits();
      buildTrackDOM();
      // let the queued frame and the final ResizeObserver delivery of the removed rows run
      await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
      await new Promise(r => setTimeout(r, 300));
    }""")
    assert app.evaluate("() => window.__kept.every(c => c.width === 0 && c.height === 0)") is True, \
        app.evaluate("() => window.__kept.map(c => [c.className, c.width, c.height])")


def test_window_fetches_of_a_zoom_gesture_are_coalesced(app, make_project):
    """P3-6b: ten requests for ranges of one track in a row make at most two fetches (the running one and one for the
    newest range); with GLQ.noCoalesce each request fetches."""
    open_project(app, make_project())
    n0 = app.invokes.count("samples_window")
    app.evaluate("""async () => { const n = S.order[0]; const ps = [];
        for (let i = 0; i < 10; i++) ps.push(ensureSamples(n, 40000 + i * 3000, 50000 + i * 3000));
        await Promise.all(ps); }""")
    assert 1 <= app.invokes.count("samples_window") - n0 <= 2
    assert app.evaluate("() => windowCovers(S.order[0], 67000, 77000)") is True
    n1 = app.invokes.count("samples_window")
    app.evaluate("""async () => { GLQ.noCoalesce = true; const n = S.order[1]; const ps = [];
        for (let i = 0; i < 4; i++) ps.push(ensureSamples(n, 100000 + i * 200000, 110000 + i * 200000));
        await Promise.all(ps); GLQ.noCoalesce = false; }""")
    assert app.invokes.count("samples_window") - n1 == 4


def test_windows_can_be_dropped_and_the_page_fetches_them_again(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => setView(0.45, 0.6)")
    app.wait_for_function("() => GLQ.eng.textures().samples === 2")
    app.evaluate("() => { GLQ.eng.dropWindows(); S.samplesCache = {}; GLQ.fetched = {}; redrawAll(); }")
    app.wait_for_function("() => GLQ.eng.textures().samples === 2 && Object.keys(S.samplesCache).length === 2")


def test_a_project_switch_while_a_window_is_on_its_way_does_not_write_into_the_new_project(app, make_project):
    open_project(app, make_project())
    app.evaluate("""async () => { const n = S.order[0], sid = S.sid;
        const p = fetchWindow(n, 40000, 50000); S.sid = 'someone-else'; await p; S.sid = sid;
        window.__late = !!S.samplesCache[n]; }""")
    assert app.evaluate("() => window.__late") is False


def test_a_burst_of_redraws_is_coalesced_and_the_newest_lanes_are_drawn(app, make_project):
    """P3-6b: at most 2 draws wait in the worker; a request that comes while they do replaces the one that waits in the
    page. 100 redraws in one task make a few draws, and the last one is drawn (stats.drawn is the newest request)."""
    open_project(app, make_project())
    app.wait_for_function("() => GLQ.eng.stats.drawn >= GLQ.eng.stats.posted")
    r = app.evaluate("""async () => { const e = GLQ.eng, d0 = e.stats.draws, p0 = e.stats.posted;
        for (let i = 0; i < 100; i++) glRepaint();
        while (e.stats.drawn < e.stats.posted) await new Promise(r => setTimeout(r, 10));
        return [e.stats.posted - p0, e.stats.draws - d0, e.stats.drawn === e.stats.posted]; }""")
    assert r[0] == 100 and 1 <= r[1] <= 4 and r[2] is True, r
