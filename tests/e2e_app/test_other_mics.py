"""#157: Other Mics Under Hit (View > Other Mics Under Hit, key O). A panel under the lanes with one small waveform
lane per other track, for the window of the selected hit (the hit time +-50 ms); the hit time is the centre line.
The panel shows only with a selected hit and follows the selection by click and by the hit-step keys. The window
comes from one bridge read per track (B.samples). Each test has a planted-fault check (the stash test of the card)."""
from app_support import menu, open_project, wait_until
from test_menus import with_hits
from test_tempo_map import js  # noqa: F401
from test_follow_strip import IDLE

FOUR = (("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Tom 1", "tom", 48), ("Hat", "hat", 42))


def setup(app, make_project):
    sid = open_project(app, make_project(tracks=FOUR, seconds=4.0))
    with_hits(app, sid, {"Snare Top": [1.0, 2.0]})
    app.evaluate("() => { if (MX.on) mxToggle(); }")   # the app setting of an earlier test may have it on
    app.wait_for_function(IDLE)
    return sid


def panel_hidden(app):
    return js(app, "() => document.querySelector('#micsPanel').classList.contains('hidden')")


def test_the_panel_shows_nothing_without_a_selected_hit_and_the_menu_turns_it_on(app, make_project):
    setup(app, make_project)
    assert js(app, "() => MX.on") is False
    menu(app, "other_mics")
    wait_until(lambda: js(app, "() => MX.on") is True, msg="the menu turns the panel on")
    assert panel_hidden(app) is True, "no hit selected: the panel shows nothing"   # planted fault: shown without a hit
    app.evaluate("() => { S.sel = { track: 'Snare Top', index: 0 }; updateStatus(); }")
    wait_until(lambda: panel_hidden(app) is False, msg="a selected hit shows the panel")
    names = js(app, "() => [...document.querySelectorAll('#micsPanel .mxName')].map(e => e.textContent)")
    assert names == ["Kick In", "Tom 1", "Hat"], names   # one lane per OTHER track, the selected track not repeated
    # every lane is drawn: a column of a waveform and the centre line make opaque pixels on each canvas
    for n in names:
        ink = js(app, f"""() => {{ const c = document.querySelector('#micsPanel .mxRow[data-name="{n}"] canvas');
          const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let k = 0;
          for (let i = 3; i < d.length; i += 4) if (d[i] === 255) k++; return k; }}""")
        assert ink >= 240, (n, ink)
    app.evaluate("() => { S.sel = null; updateStatus(); }")
    wait_until(lambda: panel_hidden(app) is True, msg="no selection: the panel hides")


def test_the_lanes_follow_a_new_selection_with_the_window_of_the_hit(app, make_project):
    setup(app, make_project)
    menu(app, "other_mics")
    app.evaluate("() => { window.__win = []; const s = B.samples; B.samples = (sid, n, a, b) => { window.__win.push([n, a, b]); return s(sid, n, a, b); }; }")
    app.evaluate("() => { S.selTrack = 'Snare Top'; S.sel = { track: 'Snare Top', index: 0 }; S.playhead = 1.0; updateStatus(); }")   # Tab counts from the playhead, within the selected track
    wait_until(lambda: js(app, "() => window.__win.length") >= 3, msg="one read per other track")
    sr = js(app, "() => S.peaks['Kick In'].sr")
    want = [round((1.0 - 0.05) * sr), round((1.0 + 0.05) * sr)]
    assert all([a, b] == want for _, a, b in js(app, "() => window.__win")), (want, js(app, "() => window.__win"))   # +-50 ms
    # the hit-step key moves the selection to the next hit (2.0 s): the lanes read the new window
    app.evaluate("() => { window.__win = []; }")
    app.keyboard.press("Tab")
    wait_until(lambda: js(app, "() => window.__win.length") >= 3, msg="the hit-step key reads the lanes again")
    want2 = [round((2.0 - 0.05) * sr), round((2.0 + 0.05) * sr)]
    assert all([a, b] == want2 for _, a, b in js(app, "() => window.__win")), (want2, js(app, "() => window.__win"))
    # a click on the hit at 1.0 s selects it: the window of 1.0 s again
    app.evaluate("() => { window.__win = []; }")
    app.evaluate("() => { S.sel = null; S.playhead = 0; updateStatus(); }")
    box = app.locator(".track[data-name='Snare Top'] canvas.spike").bounding_box()
    x = app.evaluate("() => GEO.laneX + tToX(1.0, GEO.laneW)")
    app.mouse.click(x, box["y"] + box["height"] / 2)
    wait_until(lambda: js(app, "() => window.__win.length") >= 3, msg="a click on a hit reads the lanes")
    assert all([a, b] == want for _, a, b in js(app, "() => window.__win")), js(app, "() => window.__win")


def test_a_quiet_other_mic_lane_is_normalised_to_a_visible_amplitude(app, make_project):
    """#438: each lane scales to its own peak, with the gain as a label. A bleed-level other mic
    (peak 0.05 = -26 dBFS) must draw with a visible amplitude (>= 30 % of the lane height) and
    the row shows the normalisation gain."""
    setup(app, make_project)
    menu(app, "other_mics")
    app.evaluate("""() => { const s = B.samples;   // the mock first: the selection's reads must hit it
      B.samples = (sid, n, a, b) => s(sid, n, a, b).then(r => {
        const amp = n === 'Kick In' ? 0.05 : n === 'Tom 1' ? 1.0 : null;
        if (amp === null) return r;
        const d = new Float32Array(r.data.length);
        for (let i = 0; i < d.length; i++) d[i] = amp * Math.sin(2 * Math.PI * i / 40);
        return { start: r.start, data: d };
      });
      S.sel = { track: 'Snare Top', index: 0 }; updateStatus(); }""")   # the Kick In window is a quiet bleed signal
    def state(name):
        return js(app, f"""() => {{ const row = document.querySelector('#micsPanel .mxRow[data-name="{name}"]');
          if (!row) return null; const c = row.querySelector('canvas');
          const ctx = c.getContext('2d'), d = ctx.getImageData(0, 0, c.width, c.height).data;
          const hitX = Math.round(0.5 * c.width);   // the centre line fills its column: skip it
          let best = 0;
          for (let x = 0; x < c.width; x++) {{
            if (Math.abs(x - hitX) <= 1) continue;
            let lo = c.height, hi = -1;
            for (let y = 0; y < c.height; y++) if (d[(y * c.width + x) * 4 + 3] === 255) {{ if (y < lo) lo = y; if (y > hi) hi = y; }}
            if (hi >= lo && hi - lo + 1 > best) best = hi - lo + 1;
          }}
          return [best, row.querySelector('.mxGain') ? row.querySelector('.mxGain').textContent : null]; }}""")
    wait_until(lambda: (lambda s: s is not None and s[0] >= 0.3 * 40)(state("Kick In")),
               msg="the quiet lane is drawn with a visible amplitude")
    s = state("Kick In")
    assert s[0] >= 12, s   # 30 % of the 40 px lane height
    assert s[1] == "+26 dB", s   # 20*log10(1/0.05), the normalisation gain of the lane
    # a full-scale lane keeps its amplitude: the normalisation does not shrink a loud lane
    wait_until(lambda: (lambda s2: s2 is not None and s2[0] >= 0.3 * 40)(state("Tom 1")),
               msg="the loud lane is drawn too")
    assert state("Tom 1")[1] == "+0 dB", state("Tom 1")


def test_the_key_o_toggles_the_panel_and_the_menu_state_follows(app, make_project):
    setup(app, make_project)
    app.keyboard.press("o")
    wait_until(lambda: js(app, "() => MX.on") is True, msg="O turns the panel on")
    app.evaluate("() => { S.sel = { track: 'Snare Top', index: 1 }; updateStatus(); }")
    wait_until(lambda: panel_hidden(app) is False, msg="the selected hit shows the panel")
    app.keyboard.press("o")
    wait_until(lambda: js(app, "() => MX.on") is False, msg="O turns it off again")
    assert panel_hidden(app) is True
    assert js(app, "() => document.body.classList.contains('mxOn')") is False
