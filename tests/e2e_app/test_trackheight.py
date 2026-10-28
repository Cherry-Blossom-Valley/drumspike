"""P3-14c: the compact track header and the track height (docs/migration/ux-daw-reference.md 3.4 rows "Track header"
and "Track height", 3.2 View > Track Height; Pro Tools and Cubase models). The header: a colour strip at the left edge,
the name, the role line, M and S, the volume and the pan on one grid for every track, the P3-16 meter; the link icon and
the hit count are gone (the Inspector shows them), the offline badge stays and relinks. The height: View > Track Height
(Taller Cmd+Down, Shorter Cmd+Up, Small, Medium, Large) applies to every track; the lanes (2D and GL) and the header
follow; the choice is an app setting; the geometry comes from the P3-14d ResizeObserver, no frame reads layout."""
import pytest
from playwright.sync_api import expect

from app_support import invoke, menu, open_project, wait_until
from test_frames import PROBE, READS, settle
from test_inspector import js, settings_json
from test_menus import empty_page
from test_relink import moved_project
from test_render import SETTLE, agreement, draw, on_screen_soon

HEIGHTS = {"small": (34, 56), "medium": (34, 120), "large": (34, 240)}   # spike, wave (CSS px); the spike lane stays

# The layout against GEO and the height table: every row's place, the CSS heights and the canvas stores of the near lanes,
# the header as tall as the lanes with no content cut off (the header must fit the small height).
GEO_H = """h => { const sc = document.querySelector('#lanesScroll'), sr = sc.getBoundingClientRect(), y0 = sc.scrollTop - sr.top - sc.clientTop, bad = [];
  const want = TRACK_HEIGHTS[h];
  for (const n of S.order) { const row = document.querySelector(`.track[data-name='${n}']`), w = row.querySelector('canvas.wave'), s = row.querySelector('canvas.spike');
    const g = GEO.rows[n], r = row.getBoundingClientRect(), wr = w.getBoundingClientRect(), th = row.querySelector('.thead'), tr = th.getBoundingClientRect();
    const rr = row.querySelector('.track-row').getBoundingClientRect();   // the row without the 1 px border of .track
    if (!g || Math.abs(g.wave - (wr.top + y0)) > 0.5 || Math.abs(g.top - (r.top + y0)) > 0.5 || Math.abs(g.bottom - (r.bottom + y0)) > 0.5) bad.push([n, 'place', g, r.top + y0, r.bottom + y0]);
    // the spike lane keeps its 1 px dashed border inside the 34 px (clientHeight 33, as before P3-14c): GEO.spikeH is the client height
    if (w.offsetHeight !== want.wave || s.offsetHeight !== want.spike || GEO.waveH !== want.wave || GEO.spikeH !== s.clientHeight) bad.push([n, 'height', w.offsetHeight, s.offsetHeight, GEO.waveH, GEO.spikeH]);
    if (row._live !== false && (w.height !== Math.round(want.wave * GEO.dpr) || s.height !== Math.round(want.spike * GEO.dpr))) bad.push([n, 'store', w.height, s.height]);
    if (Math.abs(tr.height - rr.height) > 0.5 || Math.abs(rr.height - (want.wave + want.spike)) > 0.5) bad.push([n, 'header or row height', tr.height, rr.height]);
    if (th.scrollHeight > th.clientHeight + 0.5) bad.push([n, 'header content cut off', th.scrollHeight, th.clientHeight]); }
  if (GEO.scW !== sc.clientWidth || GEO.scH !== sc.clientHeight) bad.push(['scroll', GEO.scW, sc.clientWidth, GEO.scH, sc.clientHeight]);
  return bad; }"""

WAVE_H = "() => document.querySelector('#lanesScroll canvas.wave').clientHeight"


def rgb(hex_colour):
    r, g, b = (int(hex_colour[i:i + 2], 16) for i in (1, 3, 5))
    return f"rgb({r}, {g}, {b})"


def set_height(page, h):
    page.evaluate("h => setTrackHeight(h)", h)
    page.wait_for_function("h => GEO.waveH === TRACK_HEIGHTS[h].wave", arg=h)
    settle(page)


def test_the_header_is_compact_with_a_colour_strip_and_the_offline_badge_relinks(app, make_project, tmp_path):
    open_project(app, make_project())
    head = app.locator('.track[data-name="Kick In"] .thead')
    strip = js(app, """() => { const row = document.querySelector('.track[data-name="Kick In"]'), s = row.querySelector('.thead .tstrip'), h = row.querySelector('.thead');
        const sr = s.getBoundingClientRect(), hr = h.getBoundingClientRect();
        return { color: getComputedStyle(s).backgroundColor, track: S.tracks['Kick In'].color, x: sr.left - hr.left, w: sr.width,
                 full: Math.abs(sr.height - hr.height) < 0.5 && Math.abs(sr.top - hr.top) < 0.5 }; }""")
    assert strip["x"] == 0 and strip["w"] == 4 and strip["full"], strip         # the role colour on a 4 px strip, the full height
    assert strip["color"] == rgb(strip["track"])
    text = head.inner_text()
    assert "Kick In" in text and "kick · C1 (36)" in text
    assert "events" not in text and "edited" not in text and "🔗" not in text  # the counts and the link icon are in the Inspector
    assert head.locator(".tlink, .tcount, .tdot").count() == 0
    assert head.locator("button").all_text_contents() == ["M", "S"]
    assert head.locator(".meter canvas").count() == 1 and head.locator(".mslider.vol").count() == 1 and head.locator(".mslider.pan").count() == 1   # P4-40h: one canvas per meter, the two bars are painted on it
    assert head.locator("[data-for=vol]").text_content() == "0.0" and head.locator("[data-for=pan]").text_content() == "C"
    for pan, want in ((-0.5, "L50"), (0.5, "R50"), (-0.07, "L7"), (1, "R100"), (0, "C")):   # the pan readout
        app.evaluate("p => setTrackMix('Kick In', null, p)", pan)
        assert head.locator("[data-for=pan]").text_content() == want, (pan, want)
    # one grid for every track: the controls are in the same columns, the readouts under their sliders, inside the header
    cols = js(app, """() => S.order.map(n => { const h = document.querySelector(`.track[data-name="${n}"] .thead`), hr = h.getBoundingClientRect();
        return ['.bMute', '.bSolo', '.mslider.vol', '.mslider.pan', '[data-for=vol]', '[data-for=pan]', '.meter'].map(s => {
          const r = h.querySelector(s).getBoundingClientRect(); return [Math.round(r.left - hr.left), Math.round(r.width), r.right <= hr.right + 0.5 && r.bottom <= hr.bottom + 0.5]; }); })""")
    assert len(cols) == 2 and cols[0] == cols[1], cols
    assert cols[0][4][0] == cols[0][2][0] and cols[0][5][0] == cols[0][3][0], cols[0]
    assert all(c[2] for c in cols[0]), cols[0]
    assert cols[0][0][0] < cols[0][1][0] < cols[0][2][0] < cols[0][3][0] < cols[0][6][0]
    # the offline badge: it stays, and a click relinks the track
    sid, folder, new = moved_project(app, make_project, tmp_path, n=2)
    badge = app.locator('.track[data-name="T01"] .thead .tOffline')
    expect(badge).to_be_visible()
    assert badge.text_content() == "offline"
    expect(app.locator('.track[data-name="T02"] .thead .tOffline')).to_be_visible()
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(new / "T01.wav"))
    badge.click()
    expect(app.locator("#stMsg")).to_contain_text("relinked T01 ->", timeout=15000)
    app.wait_for_function("() => S.loaded > 0 && S.tracks['T01'] && !S.tracks['T01'].offline")
    expect(app.locator('.track[data-name="T01"] .thead .tOffline')).to_have_count(0)
    expect(app.locator('.track[data-name="T02"] .thead .tOffline')).to_have_count(1)
    assert js(app, "() => S.selTrack") == "T01"                                # P4-39c: the first track stays selected
    app.evaluate("() => { window.__DS_PICK__ = null; }")


def test_the_track_height_changes_by_menu_and_key_and_the_lanes_and_the_header_follow(app, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Tom 1", "tom", 48))))
    set_height(app, "medium")
    assert app.evaluate(GEO_H, "medium") == []
    assert menu(app, "height_large") is True                                   # View > Track Height > Large
    app.wait_for_function("() => GEO.waveH === 240")
    settle(app)
    assert js(app, "() => S.trackHeight") == "large" and app.evaluate(GEO_H, "large") == []
    expect(app.locator("#stMsg")).to_have_text("track height: large")
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.trackHeight") == "large", msg="the radio item state")
    assert js(app, "() => [...document.querySelectorAll('#lanesScroll .track-row')].map(r => r.getBoundingClientRect().height)") == [274, 274, 274]
    app.keyboard.press("Meta+ArrowUp")                                         # Shorter (Cmd+Up; the menu key runs in the page here)
    app.wait_for_function("() => GEO.waveH === 120")
    settle(app)
    assert js(app, "() => S.trackHeight") == "medium" and app.evaluate(GEO_H, "medium") == []
    app.keyboard.press("Meta+ArrowUp")
    app.wait_for_function("() => GEO.waveH === 56")
    settle(app)
    assert js(app, "() => S.trackHeight") == "small" and app.evaluate(GEO_H, "small") == []
    expect(app.locator("#stMsg")).to_have_text("track height: small")
    assert js(app, "() => [...document.querySelectorAll('#lanesScroll .track-row')].map(r => r.getBoundingClientRect().height)") == [90, 90, 90]
    app.keyboard.press("Meta+ArrowUp")                                         # the smallest: nothing changes, the message says so
    expect(app.locator("#stMsg")).to_have_text("track height: small is the smallest")
    assert js(app, "() => S.trackHeight") == "small" and app.evaluate(WAVE_H) == 56
    app.keyboard.press("Meta+ArrowDown")                                       # Taller (Cmd+Down)
    app.wait_for_function("() => GEO.waveH === 120")
    assert js(app, "() => S.trackHeight") == "medium"
    assert menu(app, "height_taller") is True
    app.wait_for_function("() => GEO.waveH === 240")
    assert menu(app, "height_taller") is True
    expect(app.locator("#stMsg")).to_have_text("track height: large is the largest")
    assert js(app, "() => S.trackHeight") == "large" and app.evaluate(WAVE_H) == 240
    assert menu(app, "height_small") is True
    app.wait_for_function("() => GEO.waveH === 56")
    settle(app)
    assert app.evaluate(GEO_H, "small") == []
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.trackHeight") == "small", msg="the radio item state")
    # a text field keeps its keys: Cmd+Up in a field does not change the height
    app.evaluate("() => { document.body.insertAdjacentHTML('beforeend', '<input id=tmpKeys>'); document.querySelector('#tmpKeys').focus(); }")
    app.keyboard.press("Meta+ArrowDown")
    assert js(app, "() => S.trackHeight") == "small"
    app.evaluate("() => document.querySelector('#tmpKeys').remove()")
    # the items work without a project too (the height is an app setting, not project data)
    empty_page(app)
    assert menu(app, "height_medium") is True
    assert js(app, "() => S.trackHeight") == "medium"
    assert menu(app, "height_shorter") is True
    assert js(app, "() => S.trackHeight") == "small"
    app.evaluate("() => setTrackHeight('medium')")


def test_the_track_height_is_an_app_setting_read_at_boot(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => setInspector(true)")
    wait_until(lambda: settings_json(app).get("inspector") is True, msg="the Inspector setting")
    assert menu(app, "height_small") is True
    wait_until(lambda: settings_json(app).get("track_height") == "small", msg="the setting small")
    assert settings_json(app)["inspector"] is True                            # the other keys stay
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0 && GEO.ok")
    assert js(app, "() => S.trackHeight") == "small"
    app.wait_for_function("() => GEO.waveH === 56")
    settle(app)
    assert app.evaluate(WAVE_H) == 56 and app.evaluate(GEO_H, "small") == []
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.trackHeight") == "small", msg="the radio item state after the reload")
    app.evaluate("() => setTrackHeight('large')")
    wait_until(lambda: settings_json(app).get("track_height") == "large", msg="the setting large")
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0 && GEO.waveH === 240")
    assert js(app, "() => S.trackHeight") == "large" and app.evaluate(WAVE_H) == 240
    # a value the page does not know is ignored: the default height
    invoke(app, "app_settings_set", key="track_height", value="huge")
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0 && GEO.ok")
    assert js(app, "() => S.trackHeight") == "medium" and app.evaluate(WAVE_H) == 120
    app.evaluate("() => setTrackHeight('medium')")
    wait_until(lambda: settings_json(app).get("track_height") == "medium", msg="the setting medium")


@pytest.mark.parametrize("height", ["small", "medium", "large"])
def test_the_gl_lanes_and_the_2d_lanes_draw_at_the_height(app, make_project, height):
    """At each height the wave lane (WebGL2 and Canvas 2D) and the spike lane are drawn over the whole lane: the
    image has the lane's height, the wave fills most of it, both renderers agree, and the lane is on screen."""
    open_project(app, make_project())
    set_height(app, height)
    spike, wave = HEIGHTS[height]
    app.evaluate("() => setView(0.3, 1.1)")
    app.wait_for_function("() => S.order.every(n => S.samplesCache[n] && S.samplesCache[n].start <= S.view.t0 * S.peaks[n].sr)")
    app.evaluate(SETTLE)
    dpr = js(app, "() => window.devicePixelRatio || 1")
    for name in app.evaluate("() => S.order"):
        gl = draw(app, True, name)
        cv = draw(app, False, name)
        size = js(app, "async n => { const im = await laneImage(n); return [im.width, im.height]; }", name)
        assert size[1] == round(wave * dpr) and size[0] == js(app, "() => Math.floor(GEO.laneW * (window.devicePixelRatio || 1))"), size
        assert len(gl) == size[0]
        tops = [c[0] for c in gl if c[0] >= 0]
        bots = [c[1] for c in gl if c[1] >= 0]
        assert tops and max(bots) - min(tops) > 0.6 * wave * dpr, (name, min(tops), max(bots))   # the wave spans the lane
        share = agreement(gl, cv, False)
        print(f"agreement {name} {height}: {share:.4f}")
        assert share >= 0.99, (name, height, share)
    app.evaluate("() => setLanesGL(true)")
    app.evaluate(SETTLE)
    spk = js(app, """() => { const c = document.querySelector('.track[data-name="Kick In"] canvas.spike'), d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        const at = (x, y) => { const i = (y * c.width + x) * 4; return [d[i], d[i + 1], d[i + 2], d[i + 3]]; };
        const dpr = GEO.dpr, H = Math.round(GEO.spikeH * dpr);   // the lane draws its client height (33 px: the dashed border is inside the 34 px, as before P3-14c)
        return { w: c.width, h: c.height, css: c.clientHeight, line: at(10, H - 1), below: at(10, H), above: at(10, Math.floor(H / 2)) }; }""")
    assert spk["css"] == spike - 1 and spk["h"] == round(spike * dpr), spk
    assert spk["line"][:3] == [0x2c, 0x31, 0x3c] and spk["above"][3] > 0 and spk["below"][3] == 0, spk   # the base line on the last drawn row, the tint above it
    box = js(app, "() => { const r = document.querySelector('.track[data-name=\"Kick In\"] canvas.wave').getBoundingClientRect(); return [r.width, r.height]; }")
    assert box[1] == wave
    assert on_screen_soon(app, "Kick In", [16, 19, 24], 4, 0.3 * box[0] * wave) > 0.3 * box[0] * wave   # the lane background on screen
    set_height(app, "medium")


def test_a_height_change_reads_no_layout_in_a_frame_and_keeps_the_geometry(app, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Tom 1", "tom", 48)), seconds=6.0))
    set_height(app, "medium")
    app.evaluate("() => { setView(0, 6); S.playhead = 2.5; invalidate(F.HEAD); }")
    settle(app)
    app.evaluate(PROBE)                                                        # counts the layout reads inside frames from here
    overlay = """() => { const o = document.querySelector('#playheadOverlay').getBoundingClientRect(), w = document.querySelector('.track canvas.wave').getBoundingClientRect(),
        ov = document.querySelector('#overview'); return [o.left >= w.left - 1 && o.right <= w.right + 1, Math.round(o.left - w.left), ov.clientHeight, ov.height]; }"""
    ref = app.evaluate(overlay)
    for h in ("large", "small", "medium", "large", "medium"):
        n0 = js(app, "() => [FR.n.size, GLQ.eng ? GLQ.eng.stats.draws : 0]")
        app.evaluate("h => setTrackHeight(h)", h)
        app.wait_for_function("h => GEO.waveH === TRACK_HEIGHTS[h].wave", arg=h)
        settle(app)
        assert app.evaluate(GEO_H, h) == [], h
        n1 = js(app, "() => [FR.n.size, GLQ.eng ? GLQ.eng.stats.draws : 0]")
        assert n1[0] > n0[0] and n1[1] > n0[1], (h, n0, n1)                   # the canvases were sized again and the GL lanes drawn again
        assert js(app, "() => GLQ.eng ? [parseFloat(GLQ.eng.canvas.style.height) === GEO.scH, parseFloat(GLQ.eng.canvas.style.top) === GEO.scT] : [true, true]") == [True, True]
        assert app.evaluate(overlay) == ref, h                                 # the playhead overlay and the overview are as before
    reads = app.evaluate(READS)
    assert reads["n"] == 0, reads["where"]
    # a hit edit and a zoom at the large height read no layout either
    set_height(app, "large")
    app.evaluate("() => { addHitAt('Kick In', 1.0); setView(0.5, 2.5); }")
    settle(app)
    assert app.evaluate(GEO_H, "large") == []
    reads = app.evaluate(READS)
    assert reads["n"] == 0, reads["where"]
    set_height(app, "medium")
