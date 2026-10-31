"""P4-8 stage C: the tempo and meter map on the page (card steps 2 and 6, plus the page side of 3, 4 and 5).

The commands are the Rust ones (stages A and B, `crates/app/src/tempo.rs`); the page only shows them: the Tempo, Meter and
Bars|Beats rulers above the lanes (View menu, app settings), a double-click adds or edits a value in place, Delete removes
the selected one, each change is one undo step (the re-pick pattern: `tempo_map_set` answers `previous`), the bar and beat
grid in the lanes under the hit lines, Track > Detect Tempo (suggestion, confidence, warnings, Accept or Cancel, "Bar 1
two beats later" when bar 1 is a coin flip) and File > Import Tempo Map. The export of a track stops sending `tempo`.

The grid pixels are compared as in test_follow_strip.py (a screenshot of the lane box, the PNG decoded to rows)."""
import json
import re

import pytest

from app_support import export_dialog, invoke, menu, open_project, wait_until
from test_follow_strip import IDLE, lane_shot, pixels

SETTINGS = ("ruler_tempo", "ruler_meter", "ruler_beats", "grid_lines")
MAP = {"bpm": 120, "steps": [{"t": 8.0, "bpm": 90}], "meter": [4, 4], "changes": [{"bar": 3, "meter": [3, 4]}], "bar1_s": 0.0}
RULER_Y = {"tempo": 9, "meter": 9, "beats": 9}


def js(page, expr, arg=None):
    return page.evaluate(expr, arg)


@pytest.fixture
def song(app, make_project):
    """A 12 s project with the default map and the three rulers and the grid on automatic (no setting)."""
    for k in SETTINGS:
        invoke(app, "app_settings_set", key=k, value=None)
    sid = open_project(app, make_project(seconds=12.0))
    app.evaluate("async () => { await loadViewSettings(); }")
    app.evaluate("() => { setView(0, 12); }")
    yield sid
    for k in SETTINGS:
        invoke(app, "app_settings_set", key=k, value=None)


def set_map(app, sid, m):
    r = invoke(app, "tempo_map_set", sid=sid, map=m)
    assert "map" in r, r
    app.evaluate("async () => { await tmLoad(); }")
    app.wait_for_function(IDLE)
    return r


def stored(app, sid):
    return invoke(app, "tempo_map_get", sid=sid)["map"]


def settle(app):
    app.wait_for_function("() => !FR.raf && FR.flags === 0")
    app.evaluate("async () => { await TM.chain; }")
    app.wait_for_function(IDLE)


def ruler_x(app, kind, t):
    """The page x of time t on a ruler (client coordinates)."""
    return js(app, "([k, t]) => { const r = TM.cans[k].getBoundingClientRect(); return r.left + tToX(t, GEO.rulerW); }", [kind, t])


def ruler_y(app, kind):
    return js(app, "k => { const r = TM.cans[k].getBoundingClientRect(); return r.top + r.height / 2; }", kind)


def dbl(app, kind, t):
    app.mouse.dblclick(ruler_x(app, kind, t), ruler_y(app, kind))


def type_in_field(app, text):
    app.wait_for_function("() => !$('#mapEdit').classList.contains('hidden') && document.activeElement === $('#mapEdit')")
    app.keyboard.type(text)   # the field selects its text when it opens: typing replaces it
    app.keyboard.press("Enter")
    settle(app)


def msg(app):
    return app.inner_text("#stMsg")


# ---------- the settings and the View menu ----------

def test_without_a_saved_choice_the_rulers_and_the_grid_show_also_with_the_default_map(app, song):
    """P4-29: no setting = on. The default map (120 BPM, 4/4) shows the rulers and the grid; a saved choice still wins."""
    assert js(app, "() => TM.isDefault && TM.ui.tempo === null")
    assert js(app, "() => [TM.vis.tempo, TM.vis.meter, TM.vis.beats, TM.gridOn]") == [True] * 4
    assert not js(app, "() => $('#mapRulers').classList.contains('hidden')")
    app.evaluate("() => DSShell.sync()")
    st = js(app, "() => window.__DS_MENU_STATE__")
    assert [st[k] for k in ("tempoRuler", "meterRuler", "beatsRuler", "grid")] == [True] * 4, st
    # a saved off wins over the default (planted fault: the check can see a hidden state)
    app.evaluate("() => { TM.ui.meter = false; tmApplyUi(true); }")
    assert js(app, "() => [TM.vis.tempo, TM.vis.meter]") == [True, False]


def test_a_map_shows_the_rulers_and_the_grid_and_the_view_items_toggle_them_as_app_settings(app, song):
    set_map(app, song, MAP)
    assert js(app, "() => [TM.vis.tempo, TM.vis.meter, TM.vis.beats, TM.gridOn]") == [True] * 4
    assert js(app, "() => ['tempo', 'meter', 'beats'].every(k => !TM.cans[k].closest('.mrow').classList.contains('hidden'))")
    for item, key, setting, flag in (("ruler_tempo", "tempoRuler", "ruler_tempo", "tempo"), ("ruler_meter", "meterRuler", "ruler_meter", "meter"),
                                     ("ruler_beats", "beatsRuler", "ruler_beats", "beats"), ("grid", "grid", "grid_lines", "grid")):
        assert menu(app, item) is True
        wait_until(lambda: js(app, "k => tmShown(k) === false", flag), msg=f"{item} off")
        assert js(app, "() => window.__DS_MENU_STATE__")[key] is False
        wait_until(lambda: invoke(app, "app_settings_get").get(setting) is False, msg=f"{setting} saved")
        if flag != "grid":
            assert js(app, "k => TM.cans[k].closest('.mrow').classList.contains('hidden')", flag)
    assert js(app, "() => $('#mapRulers').classList.contains('hidden')")   # all three rulers off: no block
    menu(app, "ruler_meter")
    wait_until(lambda: js(app, "() => TM.vis.meter"), msg="meter ruler on again")
    wait_until(lambda: invoke(app, "app_settings_get").get("ruler_meter") is True, msg="ruler_meter saved")   # the save follows the menu (race hunt run 4, x86-mac)
    assert not js(app, "() => $('#mapRulers').classList.contains('hidden')")
    # a new page start reads the settings back (and a project without a map still hides what is not set)
    app.evaluate("async () => { TM.ui = { tempo: null, meter: null, beats: null, grid: null }; await loadViewSettings(); }")
    assert js(app, "() => [TM.ui.tempo, TM.ui.meter, TM.ui.beats, TM.ui.grid]") == [False, True, False, False]


# ---------- the grid ----------

def test_the_grid_lines_come_from_the_map_bars_stronger_than_beats_and_under_the_hit_lines(app, song):
    name = "Kick In"
    app.evaluate("() => { S.events['Kick In'] = [{id: 'g1', t: 2.0, v: 100}, {id: 'g2', t: 3.5, v: 100}]; invalidate(F.VIEW); }")
    app.evaluate("() => { TM.ui.grid = false; TM.ui.tempo = TM.ui.meter = TM.ui.beats = false; }")
    set_map(app, song, {"bpm": 120})   # bar lines at 0, 2, 4, ... and beat lines every 0.5 s
    app.evaluate("() => { tmApplyUi(true); }")
    app.wait_for_function(IDLE)
    off = pixels(lane_shot(app, name))
    app.evaluate("() => { TM.ui.grid = true; tmApplyUi(true); }")
    app.wait_for_function(IDLE)
    on = pixels(lane_shot(app, name))
    px = lambda t: js(app, "t => Math.round(tToX(t, GEO.laneW))", t)
    bar, beat, plain, hit_bar, hit_beat = px(4.0), px(4.5), px(4.3), px(2.0), px(3.5)
    y = 20
    assert sum(off[y][bar]) == sum(off[y][plain]) == sum(off[y][beat]), "grid off: the lane is the same everywhere"
    assert sum(on[y][bar]) > sum(on[y][beat]) > sum(on[y][plain]), (on[y][bar], on[y][beat], on[y][plain])
    # the grid lies under the hit lines: a hit on a bar line and a hit on a beat line look as without the grid
    assert on[y][hit_bar] == off[y][hit_bar] and on[y][hit_beat] == off[y][hit_beat]
    # the wave lane shows the lines as well (the hit lines are the top layer there too)
    wy = js(app, "() => GEO.spikeH + 10")
    assert any(on[wy][x] != off[wy][x] for x in range(bar - 1, bar + 2))


def test_a_beat_wider_than_48_px_shows_its_8th_and_16th_lines_a_beat_of_20_px_none(app, song):
    """#469: between two beat lines the lane shows the subdivisions of the beat by its width: 3 lines (8th and 16th) at 60 px, 1 (the
    8th) at 40 px, none at 20 px. The beat lines themselves are not counted (the columns 2 px inside each of them)."""
    app.evaluate("() => { S.events['Kick In'] = []; TM.ui.grid = false; TM.ui.tempo = TM.ui.meter = TM.ui.beats = false; }")
    set_map(app, song, {"bpm": 240})   # a beat is 0.25 s: the beat lines at 1.0 and 1.25 s (a 20 px beat needs a 7.5 s view, under the 10 s cap)
    counts = {}
    for beat_px in (60, 40, 20):
        app.evaluate("px => { TM.ui.grid = false; setView(0, 0.25 * GEO.laneW / px); tmApplyUi(true); }", beat_px)
        app.wait_for_function(IDLE)
        off = pixels(lane_shot(app, "Kick In"))
        app.evaluate("() => { TM.ui.grid = true; tmApplyUi(true); }")
        app.wait_for_function(IDLE)
        on = pixels(lane_shot(app, "Kick In"))
        x0, x1 = js(app, "() => [Math.round(tToX(1.0, GEO.laneW)), Math.round(tToX(1.25, GEO.laneW))]")
        y = 20
        inside = [x for x in range(x0 + 2, x1 - 1) if on[y][x] != off[y][x]]
        counts[beat_px] = len(inside)
        if beat_px == 60:   # the subdivisions are fainter than the beat line (TM.col.sub under TM.col.beat)
            beat_step = abs(sum(on[y][x0]) - sum(off[y][x0]))
            assert all(0 < abs(sum(on[y][x]) - sum(off[y][x])) < beat_step for x in inside), (beat_step, inside)
    assert counts == {60: 3, 40: 1, 20: 0}, counts


def test_the_beat_walk_splits_a_beat_at_a_step_and_follows_a_meter_change(app, song):
    """The grid arithmetic of the page (drawing only). Bar 1 is 4/4 at 120 BPM; from bar 2 the beat is an eighth (3/8); at 2.25 s
    the tempo is 60 BPM."""
    set_map(app, song, {"bpm": 120, "steps": [{"t": 2.25, "bpm": 60}], "changes": [{"bar": 2, "meter": [3, 8]}]})
    g = js(app, "() => { const g = tmGrid(); return {t: g.t.slice(0, 9), bar: g.bar.slice(0, 9), beat: g.beat.slice(0, 9), bars: g.bars.slice(0, 4)}; }")
    assert g["t"][:5] == [0, 0.5, 1.0, 1.5, 2.0] and g["bars"][:2] == [0, 2.0]
    assert g["beat"][:8] == [1, 2, 3, 4, 1, 2, 3, 1] and g["bar"][:8] == [1, 1, 1, 1, 2, 2, 2, 3]
    # the eighth that starts at 2.0 s is 0.25 s at 120 BPM and ends on the step; the next eighth is 0.5 s at 60 BPM
    assert abs(g["t"][5] - 2.25) < 1e-9 and abs(g["t"][6] - 2.75) < 1e-9 and abs(g["bars"][2] - 3.25) < 1e-9, g


def test_a_step_inside_a_beat_splits_it_and_bar_1_can_lie_after_zero(app, song):
    set_map(app, song, {"bpm": 120, "steps": [{"t": 2.25, "bpm": 60}], "bar1_s": 1.0})
    g = js(app, "() => { const g = tmGrid(); return {t: g.t.slice(0, 5), bars: g.bars.slice(0, 3)}; }")
    # the quarter beat at 2.0 s is half done (0.25 s at 120 BPM) at the step; its other half takes 0.5 s at 60 BPM
    assert g["t"][:3] == [1.0, 1.5, 2.0] and abs(g["t"][3] - 2.75) < 1e-9 and abs(g["bars"][1] - 3.75) < 1e-9, g
    assert g["bars"][0] == 1.0 and js(app, "() => tmGrid().t.every(t => t >= 1.0)")


# ---------- the tempo ruler ----------

def test_a_double_click_adds_a_tempo_step_edits_it_in_place_and_deletes_it_each_change_one_undo_step(app, song):
    set_map(app, song, {"bpm": 120, "steps": [], "bar1_s": 0.0})
    assert js(app, "() => TM.ui.tempo") is None and js(app, "() => TM.vis.tempo") is True   # P4-29: the default map shows too
    app.evaluate("() => { TM.ui.tempo = true; TM.ui.beats = true; tmApplyUi(true); }")   # as the View menu would
    app.wait_for_function(IDLE)
    u0 = js(app, "() => S.undo.length")
    # add: a double-click on an empty place puts a step on the nearest beat (4.1 s: the beat at 4.0 s), the field opens
    dbl(app, "tempo", 4.1)
    type_in_field(app, "140")
    m = stored(app, song)
    assert m["steps"] == [{"t": 4.0, "bpm": 140}] and m["bpm"] == 120, m
    assert js(app, "() => S.undo.length") == u0 + 1 and js(app, "() => S.dirty") is True
    # a second double-click on the value edits it in place
    dbl(app, "tempo", 4.0)
    type_in_field(app, "133.5")
    assert stored(app, song)["steps"] == [{"t": 4.0, "bpm": 133.5}]
    assert js(app, "() => S.undo.length") == u0 + 2
    # Return on an unchanged value is no change
    dbl(app, "tempo", 4.0)
    type_in_field(app, "133.5")
    assert js(app, "() => S.undo.length") == u0 + 2
    # the start tempo is edited in place too (it cannot be deleted)
    dbl(app, "tempo", 0.0)
    type_in_field(app, "100")
    assert stored(app, song)["bpm"] == 100
    # undo, undo, undo: the steps and the start tempo come back one by one; redo goes forward again
    for want in ({"bpm": 120, "steps": [{"t": 4.0, "bpm": 133.5}]}, {"bpm": 120, "steps": [{"t": 4.0, "bpm": 140}]}, {"bpm": 120, "steps": []}):
        menu(app, "undo")
        settle(app)
        wait_until(lambda: {k: stored(app, song)[k] for k in want} == want, msg=f"undo to {want}")
    menu(app, "redo")
    settle(app)
    wait_until(lambda: stored(app, song)["steps"] == [{"t": 4.0, "bpm": 140}], msg="redo")
    # delete: click the value, press Delete: the step is gone (one undo step), the start tempo stays
    app.mouse.click(ruler_x(app, "tempo", 4.0) + 6, ruler_y(app, "tempo"))
    assert js(app, "() => TM.sel && TM.sel.kind") == "tempo"
    app.keyboard.press("Delete")
    settle(app)
    wait_until(lambda: stored(app, song)["steps"] == [], msg="delete")
    menu(app, "undo")
    settle(app)
    wait_until(lambda: stored(app, song)["steps"] == [{"t": 4.0, "bpm": 140}], msg="undo of the delete")
    app.mouse.click(ruler_x(app, "tempo", 0.0) + 6, ruler_y(app, "tempo"))
    app.keyboard.press("Delete")
    assert "stays" in msg(app) and stored(app, song)["bpm"] == 120


def test_an_imported_value_shows_rounded_and_stays_exact_until_it_is_edited(app, song):
    set_map(app, song, {"bpm": 94.99999208, "steps": [{"t": 6.0, "bpm": 133.70004}]})
    assert js(app, "() => tmMarkers('tempo').map(m => m.text)") == ["95", "133.7"]
    app.evaluate("() => { TM.ui.tempo = true; tmApplyUi(true); }")
    app.wait_for_function(IDLE)
    dbl(app, "tempo", 0.0)
    app.wait_for_function("() => $('#mapEdit').value === '95'")
    app.keyboard.press("Enter")
    settle(app)
    assert stored(app, song)["bpm"] == 94.99999208, "unchanged: the exact value is kept"


def test_a_tempo_the_service_refuses_changes_nothing(app, song):
    set_map(app, song, {"bpm": 120, "steps": [{"t": 6.0, "bpm": 100}]})
    app.evaluate("() => { TM.ui.tempo = true; tmApplyUi(true); }")
    app.wait_for_function(IDLE)
    u0 = js(app, "() => S.undo.length")
    dbl(app, "tempo", 6.0)
    type_in_field(app, "2")   # below the 4 BPM floor of the map
    assert "refused" in msg(app), msg(app)
    assert js(app, "() => S.undo.length") == u0 and stored(app, song)["steps"] == [{"t": 6.0, "bpm": 100}]
    dbl(app, "tempo", 6.0)
    type_in_field(app, "fast")
    assert "type a number of BPM" in msg(app) and js(app, "() => S.undo.length") == u0


# ---------- the meter ruler ----------

def test_a_double_click_adds_a_meter_change_at_a_bar_edits_and_deletes_it(app, song):
    set_map(app, song, {"bpm": 120, "bar1_s": 0.0})
    app.evaluate("() => { TM.ui.meter = true; tmApplyUi(true); }")
    app.wait_for_function(IDLE)
    u0 = js(app, "() => S.undo.length")
    dbl(app, "meter", 6.1)   # the bar line at 6 s is bar 4
    type_in_field(app, "7/8")
    assert stored(app, song)["changes"] == [{"bar": 4, "meter": [7, 8]}]
    assert js(app, "() => S.undo.length") == u0 + 1
    t4 = js(app, "() => tmGrid().bars[3]")
    dbl(app, "meter", t4)
    type_in_field(app, "5/4")
    assert stored(app, song)["changes"] == [{"bar": 4, "meter": [5, 4]}]
    dbl(app, "meter", 0.0)
    type_in_field(app, "3/4")
    assert stored(app, song)["meter"] == [3, 4]
    dbl(app, "meter", 7.0)   # an empty place: a field for a new change opens; Escape drops it
    app.wait_for_function("() => !$('#mapEdit').classList.contains('hidden')")
    app.keyboard.press("Escape")
    app.wait_for_function("() => $('#mapEdit').classList.contains('hidden')")
    assert stored(app, song)["changes"] == [{"bar": 4, "meter": [5, 4]}]
    # a bad text is refused on the page, a bad value by the service
    dbl(app, "meter", js(app, "() => tmGrid().bars[3]"))
    type_in_field(app, "seven")
    assert "type beats and note value" in msg(app)
    for _ in range(3):
        menu(app, "undo")
        settle(app)
    wait_until(lambda: stored(app, song)["changes"] == [] and stored(app, song)["meter"] == [4, 4], msg="the three undos")
    for _ in range(2):
        menu(app, "redo")
        settle(app)
    wait_until(lambda: stored(app, song)["changes"] == [{"bar": 4, "meter": [5, 4]}], msg="two redos")
    app.mouse.click(ruler_x(app, "meter", js(app, "() => tmGrid().bars[3]")) + 6, ruler_y(app, "meter"))
    app.keyboard.press("Delete")
    settle(app)
    wait_until(lambda: stored(app, song)["changes"] == [], msg="delete")
    menu(app, "undo")
    settle(app)
    wait_until(lambda: len(stored(app, song)["changes"]) == 1, msg="undo of the delete")


def test_the_bars_beats_ruler_counts_bars_and_a_click_puts_the_playhead(app, song):
    set_map(app, song, MAP)
    app.wait_for_function(IDLE)
    assert js(app, "() => tmGrid().bars.slice(0, 4)") == [0, 2.0, 4.0, 5.5]
    x = ruler_x(app, "beats", 6.0)
    app.mouse.click(x, ruler_y(app, "beats"))
    wait_until(lambda: abs(js(app, "() => S.playhead") - 6.0) < 0.05, msg="the playhead from the Bars|Beats ruler")
    # the ruler's pixels: a tick at every bar and beat line (the bottom row), a label right of each bar line, nothing between the lines
    got = js(app, """() => {
        const c = TM.cans.beats, g = tmGrid(), d = devicePixelRatio, ctx = c.getContext('2d'), H = TM_H;
        const ink = (x, y, w = 1, h = 1) => ctx.getImageData(Math.round(x * d), Math.round(y * d), Math.max(1, Math.round(w * d)), Math.max(1, Math.round(h * d))).data.some((v, i) => i % 4 === 3 && v);
        const out = { bars: [], beats: [], labels: [], gaps: [] };
        for (let i = 0; i < g.t.length && g.t[i] < 11.5; i++) {
          if (g.t[i] < 0.05) continue;
          const x = Math.round(tToX(g.t[i], GEO.rulerW));
          (g.beat[i] === 1 ? out.bars : out.beats).push(ink(x, H - 2));
          if (g.beat[i] === 1) out.labels.push(ink(x + 4, 4, 8, 10));
        }
        for (const t of [0.25, 0.75, 3.25, 6.3]) out.gaps.push(ink(Math.round(tToX(t, GEO.rulerW)), H - 2));
        return out; }""")
    assert all(got["bars"]) and len(got["bars"]) >= 4 and all(got["beats"]) and len(got["beats"]) >= 10, got
    assert all(got["labels"]) and not any(got["gaps"]), got


# ---------- undo of a change that the service refused to merge / project switch ----------

def test_a_new_project_has_its_own_map_and_no_undo_of_the_old_one(app, song, make_project):
    set_map(app, song, MAP)
    other = open_project(app, make_project(seconds=8.0))
    assert other != song
    assert js(app, "() => TM.isDefault") is True and js(app, "() => TM.vis.tempo") is True
    assert js(app, "() => S.undo.length") == 0


# ---------- detection ----------

def hit_train(bpm=100.0, bars=24, start=1.2):
    beat = 60.0 / bpm
    kick, snare = [], []
    for b in range(bars):
        t0 = start + b * 4 * beat
        kick += [round(t0, 6), round(t0 + 2 * beat, 6)]
        snare += [round(t0 + beat, 6), round(t0 + 3 * beat, 6)]
    return kick, snare


def set_hits(app, sid, name, times):
    invoke(app, "events_set", sid=sid, name=name, events=[{"t": t, "v": 100, "id": f"{name[0]}{i}"} for i, t in enumerate(times)])


def test_detect_tempo_shows_the_suggestion_and_accept_is_one_undo_step_cancel_changes_nothing(app, make_project):
    sid = open_project(app, make_project(seconds=62.0))
    kick, snare = hit_train()
    set_hits(app, sid, "Kick In", kick)
    set_hits(app, sid, "Snare Top", snare)
    app.evaluate("async () => { await loadProject(); }")
    app.wait_for_function("() => S.loaded > 0 && S.events['Kick In'].length === 48")
    u0 = js(app, "() => S.undo.length")
    menu(app, "detect_tempo", wait=False)
    app.wait_for_function("() => !$('#tempoModal').classList.contains('hidden')", timeout=60000)
    text = app.inner_text("#tempoBody")
    assert "Start tempo: 100 BPM" in text and "confidence" in text and "Bar 1 at 1.2" in text and "Meter: 4/4" in text, text
    app.click("#tempoCancel")
    assert js(app, "() => S.undo.length") == u0 and invoke(app, "tempo_map_get", sid=sid)["default"] is True
    menu(app, "detect_tempo", wait=False)
    app.wait_for_function("() => !$('#tempoModal').classList.contains('hidden')", timeout=60000)
    app.click("#tempoAccept")
    wait_until(lambda: invoke(app, "tempo_map_get", sid=sid)["default"] is False, msg="Accept")
    settle(app)
    m = stored(app, sid)
    assert abs(m["bpm"] - 100) <= 0.5 and abs(m["bar1_s"] - 1.2) < 0.05 and m["meter"] == [4, 4], m
    assert js(app, "() => S.undo.length") == u0 + 1
    assert js(app, "() => [TM.vis.tempo, TM.vis.meter, TM.vis.beats, TM.gridOn]") == [True] * 4   # a map is set: the rulers and the grid show
    menu(app, "undo")
    settle(app)
    wait_until(lambda: invoke(app, "tempo_map_get", sid=sid)["default"] is True, msg="undo of the accept")


def fake_detection(app, bar1_conf, warnings):
    app.evaluate("""([c, w]) => {
        B.tempoMapDetect = async () => ({
          suggestion: {bpm: 92, steps: [{t: 30, bpm: 96}], meter: [4, 4], changes: [], bar1_s: 1.5},
          start: {bpm: 91.97, error_bar: 0.05, confidence: 0.83, range_edge: false, alternatives: [{bpm: 184, score: 0.7, comb: 0.9, gated: false}, {bpm: 46, score: 0.2, comb: 0.3, gated: true}]},
          bar1: {t: 1.5, confidence: c, backbeat: 0.5, downbeat: 0.4, alternative: 2.5},
          meter: {value: [4, 4], confidence: 0.8, feel: null},
          steps: [{t: 30, bpm: 96, bar: 14, bars: 8, confidence: 0.7}], excursions: [{t: 50, bars: 2, bpm: 80, map_bpm: 92}],
          fit: {on_beat: 0.62, jitter_ms: 12.4, hits: 400, beats: 120, late: 0, isolated: 0}, tracks: [], params: {}, warnings: w, changed: false });
      }""", [bar1_conf, warnings])


def test_a_low_bar_1_confidence_offers_the_other_candidate_and_the_warnings_are_listed(app, song):
    warn = ["A short excursion to 80 BPM at 50 s (2 bars): ask the user"]
    fake_detection(app, 0.2, warn)
    menu(app, "detect_tempo", wait=False)
    app.wait_for_function("() => !$('#tempoModal').classList.contains('hidden')")
    text = app.inner_text("#tempoBody")
    assert warn[0] in text and "bar 14" in text and "96 BPM" in text and "184" in text and "46 BPM" not in text and "coin flip" in text, text
    assert not js(app, "() => $('#tempoBar1').classList.contains('hidden')")
    app.click("#tempoBar1")
    assert "Bar 1 at 2.500 s" in app.inner_text("#tempoBody") and app.inner_text("#tempoBar1") == "Bar 1 as first guessed"
    app.click("#tempoAccept")
    wait_until(lambda: stored(app, song)["bar1_s"] == 2.5, msg="the other candidate")
    assert stored(app, song)["steps"] == [{"t": 30, "bpm": 96}] and stored(app, song)["bpm"] == 92


def test_a_sure_bar_1_has_no_other_candidate_button_and_escape_cancels(app, song):
    fake_detection(app, 0.7, [])
    menu(app, "detect_tempo", wait=False)
    app.wait_for_function("() => !$('#tempoModal').classList.contains('hidden')")
    assert js(app, "() => $('#tempoBar1').classList.contains('hidden')")
    app.keyboard.press("Escape")
    app.wait_for_function("() => $('#tempoModal').classList.contains('hidden')")
    assert invoke(app, "tempo_map_get", sid=song)["default"] is True


def test_the_dialog_shows_whole_number_tempos_only_even_when_the_fit_is_fractional(app, song):
    app.evaluate("""() => {
        B.tempoMapDetect = async () => ({
          suggestion: {bpm: 91.9999, steps: [{t: 30, bpm: 95.5}], meter: [4, 4], changes: [], bar1_s: 1.5},
          start: {bpm: 91.97, error_bar: 0.05, confidence: 0.83, range_edge: false, alternatives: [{bpm: 183.7, score: 0.7, comb: 0.9, gated: false}]},
          bar1: {t: 1.5, confidence: 0.7, backbeat: 0.5, downbeat: 0.4, alternative: 2.5},
          meter: {value: [4, 4], confidence: 0.8, feel: null},
          steps: [{t: 30, bpm: 95.5, bar: 14, bars: 8, confidence: 0.7}], excursions: [],
          fit: {on_beat: 0.62, jitter_ms: 12.4, hits: 400, beats: 120, late: 0, isolated: 0}, tracks: [], params: {}, warnings: [], changed: false });
      }""")
    menu(app, "detect_tempo", wait=False)
    app.wait_for_function("() => !$('#tempoModal').classList.contains('hidden')")
    text = app.inner_text("#tempoBody")
    bpms = re.findall(r"([0-9.]+) BPM", text)
    assert bpms and all("." not in b for b in bpms), text
    assert "Start tempo: 92 BPM" in text and "96 BPM" in text and "184 BPM" in text, text
    app.click("#tempoAccept")
    wait_until(lambda: abs(stored(app, song)["bpm"] - 91.9999) < 1e-9, msg="the map keeps the exact value")


DOUBLE_TIME = """(bpm) => {
    B.tempoMapDetect = async () => ({
      suggestion: {bpm, steps: [], meter: [4, 4], changes: [], bar1_s: 0},
      start: {bpm, error_bar: 0.05, confidence: 0.22, range_edge: false, alternatives: [{bpm: bpm / 2, score: 0.7, comb: 0.9, gated: false}]},
      bar1: {t: 0, confidence: 1, backbeat: 0.5, downbeat: 0.4, alternative: 1.3},
      meter: {value: [4, 4], confidence: 0.35, feel: null},
      steps: [], excursions: [],
      fit: {on_beat: 0.72, jitter_ms: 3.7, hits: 736, beats: 874, late: 0, isolated: 0}, tracks: [], params: {}, warnings: [], changed: false });
  }"""


def test_the_dialog_offers_the_detected_tempo_half_time_and_an_own_tempo(app, song):
    # Hotfix 0.9.10 (owner): a song at 92 BPM detected as 184 (double time). Half time and a typed
    # tempo commit like a start tempo typed in the Tempo ruler; half time only above 40 BPM.
    app.evaluate(DOUBLE_TIME, 184)
    menu(app, "detect_tempo", wait=False)
    app.wait_for_function("() => !$('#tempoModal').classList.contains('hidden')")
    assert app.inner_text("#tempoAccept") == "Use 184 BPM"
    assert app.is_visible("#tempoHalf") and app.inner_text("#tempoHalf") == "Half time: 92 BPM"
    app.click("#tempoHalf")
    wait_until(lambda: stored(app, song)["bpm"] == 92, msg="half time is stored")
    settle(app)
    assert stored(app, song)["bar1_s"] >= 0
    menu(app, "detect_tempo", wait=False)
    app.wait_for_function("() => !$('#tempoModal').classList.contains('hidden')")
    app.fill("#tempoOwn", "fast")
    app.click("#tempoOwnUse")
    assert "type a number of BPM" in msg(app) and app.is_visible("#tempoModal"), msg(app)
    app.fill("#tempoOwn", "95")
    app.press("#tempoOwn", "Enter")
    wait_until(lambda: stored(app, song)["bpm"] == 95, msg="the own tempo is stored")
    settle(app)
    app.evaluate(DOUBLE_TIME, 70)
    menu(app, "detect_tempo", wait=False)
    app.wait_for_function("() => !$('#tempoModal').classList.contains('hidden')")
    assert not app.is_visible("#tempoHalf"), "35 BPM is not offered as half time"
    app.click("#tempoCancel")
    assert stored(app, song)["bpm"] == 95


def test_a_typed_start_tempo_commits_when_the_refit_puts_bar_1_before_0_s(app, song):
    # Hotfix 0.9.10: on a real session the refit answered bar 1 at -0.0008 s, the service refused
    # the map and the detected 184 BPM stayed. The page keeps bar 1 at 0 s.
    set_map(app, song, {"bpm": 184, "bar1_s": 0})
    app.evaluate("() => { B.tempoMapRefitBar1 = async (sid, bpm) => ({bar1_s: -0.0008, support: 1, hits: 700, previous_bar1_s: 0, bpm, changed: false}); }")
    app.evaluate("() => { TM.ui.tempo = true; tmApplyUi(true); tmEditStart('tempo', tmMarkers('tempo').find(x => x.id === 'start')); }")
    type_in_field(app, "92")
    m = stored(app, song)
    assert m["bpm"] == 92 and m["bar1_s"] == 0, (m, msg(app))


def test_detect_tempo_without_hits_says_why(app, song):
    for name in ("Kick In", "Snare Top"):
        set_hits(app, song, name, [])
    app.evaluate("async () => { await loadProject(); }")
    menu(app, "detect_tempo")
    assert "detect tempo failed" in msg(app), msg(app)
    assert js(app, "() => $('#tempoModal').classList.contains('hidden')")


# ---------- import ----------

NO_EVENTS = bytes.fromhex("4d546864000000060000000101e0" "4d54726b00000004" "00ff2f00")   # a header and an empty track


def test_import_tempo_map_replaces_the_map_in_one_undo_step_and_a_refusal_shows_its_message(app, song, tmp_path):
    set_map(app, song, MAP)
    set_hits(app, song, "Kick In", [0.5, 1.0, 9.0, 11.0])
    app.evaluate("async () => { await loadProject(); }")
    out = invoke(app, "export_midi", sid=song, name="Kick In", note=36, events=js(app, "() => S.events['Kick In']"), ppq=24000)
    mid = out["path"]
    want = stored(app, song)

    def same(a, b):   # the file holds whole microseconds per quarter: 90 BPM comes back as 89.9999955
        return (a["meter"] == b["meter"] and a["changes"] == b["changes"] and abs(a["bpm"] - b["bpm"]) < 1e-3 and abs(a["bar1_s"] - b["bar1_s"]) < 1e-3
                and len(a["steps"]) == len(b["steps"]) and all(abs(x["t"] - y["t"]) < 1e-3 and abs(x["bpm"] - y["bpm"]) < 1e-3 for x, y in zip(a["steps"], b["steps"])))
    # another map in the project, then the file of the first one brings it back
    set_map(app, song, {"bpm": 100})
    u0 = js(app, "() => S.undo.length")
    app.evaluate("p => { window.__DS_PICK__ = p; }", mid)
    assert menu(app, "import_tempo_map") is True
    settle(app)
    wait_until(lambda: same(stored(app, song), want), msg="the imported map")
    assert js(app, "() => S.undo.length") == u0 + 1 and "imported" in msg(app) and "tempo" in msg(app), msg(app)
    menu(app, "undo")
    settle(app)
    wait_until(lambda: stored(app, song)["bpm"] == 100 and stored(app, song)["steps"] == [], msg="undo of the import")
    # a MIDI file without tempo and meter events is refused with the service's message; the map stays
    bad = tmp_path / "plain.mid"
    bad.write_bytes(NO_EVENTS)
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(bad))
    u1 = js(app, "() => S.undo.length")
    menu(app, "import_tempo_map")
    settle(app)
    assert "refused" in msg(app), msg(app)
    assert stored(app, song)["bpm"] == 100 and js(app, "() => S.undo.length") == u1
    junk = tmp_path / "junk.mid"
    junk.write_bytes(b"not a midi file")
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(junk))
    menu(app, "import_tempo_map")
    settle(app)
    assert "refused" in msg(app) and stored(app, song)["bpm"] == 100
    # a cancelled dialog changes nothing
    app.evaluate("() => { window.__DS_PICK__ = null; }")
    menu(app, "import_tempo_map")
    assert msg(app) == "import cancelled"


# ---------- export: the page sends no tempo ----------

def test_the_export_of_a_track_sends_no_tempo_and_the_default_map_writes_the_bytes_of_before(app, song):
    sent = []
    app.on("request", lambda r: sent.append(r.post_data) if r.url.endswith("/invoke/export_all") else None)
    set_hits(app, song, "Kick In", [0.5, 1.5, 2.5])
    app.evaluate("async () => { await loadProject(); S.selTrack = 'Kick In'; }")
    export_dialog(app, formats=("midi",), scope="selected", track="Kick In", via="menu")   # P4-30: the dialog sends export_all
    wait_until(lambda: sent, msg="the export request")
    body = json.loads(sent[0])
    assert "tempo" not in body and "ppq" not in body and body["tracks"] == ["Kick In"] and body["formats"] == ["midi"], body
    path = invoke(app, "project_get", sid=song)["folder"] + "/Exports/Kick In_spike.mid"
    page_bytes = open(path, "rb").read()
    # the same file as the old call with the tempo 120 (the default map takes the old code path)
    invoke(app, "export_midi", sid=song, name="Kick In", note=36, events=js(app, "() => S.events['Kick In']"), ppq=24000, tempo=120)
    assert open(path, "rb").read() == page_bytes


# ---------- the strip ----------

def test_the_rulers_move_with_the_follow_strip(app, make_project):
    song = open_project(app, make_project(seconds=40.0))
    set_map(app, song, {"bpm": 110})
    app.evaluate("() => { setFollow('smooth'); S.followHold = false; S.playhead = 0; setView(0, 8); }")
    app.wait_for_function(IDLE)
    app.click("#playBtn")
    app.wait_for_function("() => S.playing && AC.running", timeout=20000)
    app.wait_for_function("() => SP.on", timeout=20000)
    got = js(app, "() => TM_KINDS.map(k => [SP.els.some(e => e.el === TM.cans[k]), TM.cans[k].parentNode.style.clipPath !== '', TM.cans[k].clientWidth === 2 * GEO.rulerW])")
    # P4-40g: the markers ruler rides in TM_KINDS; this project has no markers, so it stays hidden
    assert got == [[True, True, True]] * 3 + [[False, False, False]], got
    # the bar lines of the Bars|Beats ruler and of the lane are on the same columns of the strip
    al = js(app, """() => {
        const rc = TM.cans.beats.getContext('2d'), row = document.querySelector('.track .spike'), lc = row.getContext('2d'), d = devicePixelRatio, g = tmGrid(), out = [];
        for (const t of g.bars) {
          if (t < SP.s + 0.5 || t > SP.s + 2 * SP.d - 0.5) continue;
          const x = Math.round((t - SP.s) / SP.d * GEO.rulerW) + 0.5;   // the strip draws S.view = {s, s + 2d} on twice the width
          const r = rc.getImageData(Math.floor(x * d), (TM_H - 2) * d, 1, 1).data[3] > 0;
          const a = lc.getImageData(Math.floor(x * d), 20 * d, 1, 1).data, b = lc.getImageData(Math.floor((x + 5) * d), 20 * d, 1, 1).data;
          out.push([r, a[0] + a[1] + a[2] !== b[0] + b[1] + b[2]]);
        }
        return out; }""")
    assert len(al) >= 3 and all(r and l for r, l in al), al
    app.click("#playBtn")
    app.wait_for_function("() => !S.playing && !SP.on")
    assert js(app, "() => TM_KINDS.every(k => TM.cans[k].parentNode.style.clipPath === '')")


def test_the_carry_label_stays_in_view_while_the_follow_strip_plays(app, make_project):
    """#150: the map has no step in view, so each ruler shows only the carry label (the value in force at the left
    edge). Stopped it sits at x 3 of the draw. Under the follow strip the canvas holds two views on twice the width
    (spApply): a label only at x 3 glides off-screen with the first view, so the second view's left edge must carry
    its own label. Checked as text pixels in the left 40 px of the visible Tempo and Meter clips."""
    song = open_project(app, make_project(seconds=40.0))
    set_map(app, song, {"bpm": 92, "meter": [4, 4]})   # no steps, no changes: no marker in view but the starts at t 0
    assert js(app, "() => tmMarkers('tempo').map(m => m.text)") == ["92"] and js(app, "() => tmMarkers('meter').map(m => m.text)") == ["4/4"]
    app.evaluate("() => { TM.ui.tempo = TM.ui.meter = true; TM.ui.beats = false; tmApplyUi(true); }")

    def clip_text_pixels(kind, png):
        from collections import Counter
        rows = pixels(png)
        reg = [px for row in rows for px in row[:40]]
        bg = Counter(reg).most_common(1)[0][0]
        n = sum(1 for px in reg if sum(abs(a - b) for a, b in zip(px, bg)) > 60)
        return n

    def shot(kind):
        return app.locator(f'#mapRulers .mrow[data-ruler="{kind}"] .mclip').screenshot(animations="allow")

    def hold_strip_12px_into_the_second_view():
        held = js(app, """() => {
            const d = SP.d, RW = GEO.rulerW, s = SP.s;
            const v0 = s + (RW - 12) / RW * d;   // the model time the visible left edge shows: 12 px short of view 2's first label column
            const t = CP.at0 + (v0 + FOLLOW_FRAC * d - CP.pos0) / (CP.rate * CP.pr) * 1000;
            spFreeze(t);
            return { d, RW, s, v0 };
        }""")
        app.evaluate("() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))")
        return held

    # (a) stopped, the view left of the starts: the carry label shows in the left 40 px
    app.evaluate("() => { S.playhead = 0; setView(4, 8); }")
    app.wait_for_function(IDLE)
    for kind in ("tempo", "meter"):
        assert clip_text_pixels(kind, shot(kind)) > 15, f"{kind}: no carry label while stopped"

    # (b) playing under the strip: the visible window sits inside the second view; its left edge holds the label too
    app.evaluate("() => { setFollow('smooth'); S.followHold = false; S.playhead = 0; setView(0, 8); }")
    app.wait_for_function(IDLE)
    app.click("#playBtn")
    app.wait_for_function("() => S.playing && AC.running", timeout=20000)
    app.wait_for_function("() => SP.on", timeout=20000)
    hold_strip_12px_into_the_second_view()
    assert app.evaluate("() => GEO.dpr") == 1, "the pixel checks assume a device scale of 1 (the test browser)"
    for kind in ("tempo", "meter"):
        assert clip_text_pixels(kind, shot(kind)) > 15, f"{kind}: the strip draws the carry label off-screen"
    app.locator("#mapRulers").screenshot(path="test-shots/150-strip-carry-label.png", animations="allow")
    app.evaluate("() => spFreeze(null)")

    # (c) after stop: the frame path draws again, the label is back
    app.click("#playBtn")
    app.wait_for_function("() => !S.playing && !SP.on")
    app.wait_for_function(IDLE)
    for kind in ("tempo", "meter"):
        assert clip_text_pixels(kind, shot(kind)) > 15, f"{kind}: no carry label after the stop"


# ---------- fix round: undo while a change is stored, click away, equal values, dialog ----------

def delay(app, fn, ms):
    app.evaluate("([f, ms]) => { const o = B[f]; B[f] = async (...a) => { await new Promise(r => setTimeout(r, ms)); return o(...a); }; }", [fn, ms])


def test_undo_and_redo_wait_while_a_tempo_change_is_being_stored(app, song):
    set_map(app, song, {"bpm": 100})
    app.evaluate("() => { TM.ui.tempo = true; tmApplyUi(true); }")
    js(app, "async () => { await tmCommit({bpm: 111}, 'A'); }")
    assert js(app, "() => S.undo.length") == 1
    delay(app, "tempoMapSet", 400)
    app.evaluate("() => { window.__b = tmCommit({bpm: 122}, 'B'); undo(); }")   # undo right after the commit started
    assert "wait for it" in msg(app), msg(app)
    app.evaluate("async () => { await window.__b; }")
    settle(app)
    assert stored(app, song)["bpm"] == 122 and js(app, "() => S.undo.map(o => o.what)") == ["A", "B"] and js(app, "() => S.redo.length") == 0
    menu(app, "undo")
    settle(app)
    wait_until(lambda: stored(app, song)["bpm"] == 111, msg="undo of B")
    app.evaluate("() => { window.__b = tmApplyOp({sid: S.sid, before: {bpm: 100}, after: {bpm: 111}, what: 'x'}, true); redo(); }")
    assert "wait for it" in msg(app)
    settle(app)


def test_undo_waits_for_an_import_that_is_being_stored(app, song, tmp_path):
    set_map(app, song, {"bpm": 90})
    set_hits(app, song, "Kick In", [0.5, 1.5])
    app.evaluate("async () => { await loadProject(); }")
    mid = invoke(app, "export_midi", sid=song, name="Kick In", note=36, events=js(app, "() => S.events['Kick In']"), ppq=24000)["path"]
    set_map(app, song, {"bpm": 130})
    app.evaluate("p => { window.__DS_PICK__ = p; }", mid)
    delay(app, "tempoMapImportMidi", 400)
    app.evaluate("() => { window.__i = importTempoMap(); }")
    app.wait_for_function("() => TM.pending === 1")
    app.evaluate("() => undo()")
    assert "wait for it" in msg(app)
    app.evaluate("async () => { await window.__i; }")
    settle(app)
    assert abs(stored(app, song)["bpm"] - 90) < 1e-3 and js(app, "() => S.undo.map(o => o.what)") == ["import tempo map"]


def test_a_click_on_a_lane_or_the_time_ruler_applies_the_open_field_and_space_plays(app, song):
    set_map(app, song, {"bpm": 120, "steps": [{"t": 6.0, "bpm": 100}]})
    app.evaluate("() => { TM.ui.tempo = true; tmApplyUi(true); }")
    app.wait_for_function(IDLE)
    for target, want in (('.track[data-name="Kick In"] canvas.wave', 111), ("#ruler", 112)):
        dbl(app, "tempo", 6.0)
        app.wait_for_function("() => document.activeElement === $('#mapEdit')")
        app.keyboard.type(str(want))
        box = app.locator(target).bounding_box()
        app.mouse.click(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
        wait_until(lambda: stored(app, song)["steps"] == [{"t": 6.0, "bpm": want}], msg=f"the field applied by a click on {target}")
        assert js(app, "() => $('#mapEdit').classList.contains('hidden')")
        app.keyboard.press("Space")
        app.wait_for_function("() => S.playing", timeout=10000)
        assert js(app, "() => $('#mapEdit').value") != " "
        app.keyboard.press("Space")
        app.wait_for_function("() => !S.playing")
        settle(app)


def test_a_value_equal_to_the_one_in_force_adds_nothing(app, song):
    set_map(app, song, {"bpm": 120})
    app.evaluate("() => { TM.ui.tempo = TM.ui.meter = true; tmApplyUi(true); }")
    app.wait_for_function(IDLE)
    for kind, t, text in (("tempo", 4.0, "120"), ("meter", 4.0, "4/4")):
        u0 = js(app, "() => S.undo.length")
        dbl(app, kind, t)
        type_in_field(app, text)
        assert "the same as" in msg(app), msg(app)
        assert js(app, "() => S.undo.length") == u0 and stored(app, song)["steps"] == [] and stored(app, song)["changes"] == []


def test_the_dialog_names_the_other_candidate_when_the_meter_is_not_4_4_and_a_second_detect_does_not_stack(app, song):
    fake_detection(app, 0.2, [])
    app.evaluate("() => { const o = B.tempoMapDetect; B.tempoMapDetect = async () => { const d = await o(); d.suggestion.meter = [3, 4]; d.meter.value = [3, 4]; return d; }; }")
    menu(app, "detect_tempo", wait=False)
    app.wait_for_function("() => !$('#tempoModal').classList.contains('hidden')")
    assert app.inner_text("#tempoBar1") == "Bar 1 at the other candidate"
    app.click("#tempoBar1")
    assert "two beats" not in app.inner_text("#tempoBody") and "at the other candidate than the first guess" in app.inner_text("#tempoBody")
    app.evaluate("() => { window.__again = detectTempo(); }")
    assert "answer the Detect Tempo dialog first" in msg(app)
    assert js(app, "async () => (await window.__again) === null")
    app.click("#tempoCancel")
    assert invoke(app, "tempo_map_get", sid=song)["default"] is True


def test_the_detect_button_on_the_tempo_ruler_opens_detect_tempo(app, make_project):
    """owner 2026-10-14: a Detect button in the tempo ruler header does what Track > Detect Tempo does."""
    sid = open_project(app, make_project(seconds=62.0))
    kick, snare = hit_train()
    set_hits(app, sid, "Kick In", kick)
    set_hits(app, sid, "Snare Top", snare)
    app.evaluate("async () => { await loadProject(); }")
    app.wait_for_function("() => S.loaded > 0 && S.events['Kick In'].length === 48")
    app.evaluate("() => { $('#mapRulers').classList.remove('hidden'); $('#mapRulers .mrow[data-ruler=tempo]').classList.remove('hidden'); }")
    btn = app.locator("#tempoDetectBtn")
    assert btn.is_visible() and btn.bounding_box()["height"] <= 18     # fits the 18 px ruler row
    # really on screen: not clipped by the header (a column layout once put it under the label, hidden by overflow)
    assert app.evaluate("""() => { const b = $('#tempoDetectBtn').getBoundingClientRect(), h = $('#tempoDetectBtn').parentElement.getBoundingClientRect();
      return b.top >= h.top && b.bottom <= h.bottom + 0.5 && document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2) === $('#tempoDetectBtn'); }""")
    btn.click()
    app.wait_for_function("() => !$('#tempoModal').classList.contains('hidden')", timeout=60000)
    assert "Start tempo: 100 BPM" in app.inner_text("#tempoBody")
    app.click("#tempoCancel")
    assert invoke(app, "tempo_map_get", sid=sid)["default"] is True
