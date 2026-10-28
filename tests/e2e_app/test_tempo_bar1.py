"""P4-39d: bar 1 by hand. Track > Set Bar 1 at Playhead (or at the selected hit), a double-click on the bar-1 marker of the
Bars|Beats ruler (a field with the time in seconds) and a drag of that marker move bar 1 only; the tempo, the steps, the meter
and the meter changes stay. Each change is one undo step (tmCommit, the tempo edit of the page), the grid and the rulers
come from the new map. No new service command: the page sends the whole map with `tempo_map_set`."""
from app_support import invoke, menu, open_project, wait_until
from test_follow_strip import IDLE
from test_tempo_map import dbl, js, msg, ruler_x, ruler_y, set_map, settle, song, stored, type_in_field  # noqa: F401 (song is a fixture)

# Session E (P4-39e): 118 BPM; the old detection put bar 1 a sixteenth late, at 2.152 s, the true bar 1 is at 2.025 s.
MAP = {"bpm": 118, "steps": [{"t": 8.0, "bpm": 90}], "meter": [4, 4], "changes": [{"bar": 3, "meter": [3, 4]}], "bar1_s": 2.152}
# #468: the owner case, the 46 BPM suggestion with bar 1 at 2.555 s (53 ms off a beat of the 92 grid).
MAP46 = {"bpm": 46, "steps": [], "meter": [4, 4], "changes": [], "bar1_s": 2.555}


def rest(m):
    return {k: v for k, v in m.items() if k != "bar1_s"}


def test_set_bar_1_at_playhead_moves_bar_1_keeps_the_tempo_and_undo_restores_it(app, song):
    set_map(app, song, MAP)
    u0 = js(app, "() => S.undo.length")
    app.evaluate("() => { S.sel = null; S.playhead = 2.025; }")
    assert menu(app, "set_bar1") is True
    settle(app)
    m = stored(app, song)
    assert m["bar1_s"] == 2.025 and rest(m) == rest(MAP), m
    assert msg(app) == "bar 1 at 2.025 s"
    assert js(app, "() => S.undo.length") == u0 + 1 and js(app, "() => S.dirty") is True
    assert js(app, "() => S.undo[S.undo.length - 1].type") == "tempo"
    # the grid and the rulers come from the new map
    assert js(app, "() => [TM.map.bar1_s, tmGrid().bars[0], tmGrid().t[1]]") == [2.025, 2.025, 2.025 + 60 / 118]
    # undo gives bar 1 back, redo moves it again
    menu(app, "undo")
    settle(app)
    wait_until(lambda: stored(app, song) == {**MAP, "bar1_s": 2.152}, msg="undo")
    assert js(app, "() => tmGrid().bars[0]") == 2.152
    menu(app, "redo")
    settle(app)
    wait_until(lambda: stored(app, song)["bar1_s"] == 2.025, msg="redo")
    # the same place again is no change and no undo step
    u1 = js(app, "() => S.undo.length")
    menu(app, "set_bar1")
    settle(app)
    assert "no change" in msg(app) and js(app, "() => S.undo.length") == u1


def test_with_a_hit_selected_bar_1_goes_to_the_hit(app, song):
    set_map(app, song, MAP)
    app.evaluate("""() => { S.events['Kick In'] = [{id: 'b1', t: 3.5, v: 100}];
                            S.sel = {track: 'Kick In', index: 0}; S.playhead = 1.0; invalidate(F.VIEW); }""")
    menu(app, "set_bar1")
    settle(app)
    m = stored(app, song)
    assert m["bar1_s"] == 3.5 and rest(m) == rest(MAP), m
    assert msg(app) == "bar 1 at 3.500 s"


def test_a_double_click_on_the_bar_1_marker_edits_its_time(app, song):
    set_map(app, song, MAP)
    app.evaluate("() => { TM.ui.beats = true; tmApplyUi(true); }")
    app.wait_for_function(IDLE)
    u0 = js(app, "() => S.undo.length")
    dbl(app, "beats", 2.152)
    app.wait_for_function("() => $('#mapEdit').value === '2.152'")
    type_in_field(app, "2.025")
    m = stored(app, song)
    assert m["bar1_s"] == 2.025 and rest(m) == rest(MAP), m
    assert js(app, "() => S.undo.length") == u0 + 1 and msg(app) == "bar 1 at 2.025 s"
    # a text that is no time is refused on the page; Escape drops the field
    dbl(app, "beats", 2.025)
    type_in_field(app, "soon")
    assert "type a time in seconds" in msg(app) and js(app, "() => S.undo.length") == u0 + 1
    dbl(app, "beats", 2.025)
    app.wait_for_function("() => !$('#mapEdit').classList.contains('hidden')")
    app.keyboard.press("Escape")
    app.wait_for_function("() => $('#mapEdit').classList.contains('hidden')")
    assert stored(app, song)["bar1_s"] == 2.025
    # a double-click away from the marker opens no field
    dbl(app, "beats", 6.0)
    assert js(app, "() => $('#mapEdit').classList.contains('hidden')")
    # the edit is one step: undo gives 2.152 back
    menu(app, "undo")
    settle(app)
    wait_until(lambda: stored(app, song)["bar1_s"] == 2.152, msg="undo of the edit")


def test_a_drag_of_the_bar_1_marker_moves_bar_1_and_a_click_elsewhere_still_puts_the_playhead(app, song):
    set_map(app, song, MAP)
    app.evaluate("() => { S.snap = false; TM.ui.beats = true; tmApplyUi(true); }")
    app.wait_for_function(IDLE)
    u0 = js(app, "() => S.undo.length")
    y = ruler_y(app, "beats")
    app.mouse.move(ruler_x(app, "beats", 2.152) + 1, y)
    app.mouse.down()
    app.mouse.move(ruler_x(app, "beats", 3.0), y, steps=4)
    assert abs(js(app, "() => tmGrid().bars[0]") - 3.0) < 0.05, "the grid follows the drag"
    app.mouse.move(ruler_x(app, "beats", 4.0), y, steps=4)
    app.mouse.up()
    settle(app)
    m = stored(app, song)
    px = js(app, "() => viewDur() / GEO.rulerW")   # one pixel in seconds
    assert abs(m["bar1_s"] - 4.0) <= 2 * px and rest(m) == rest(MAP), m
    assert js(app, "() => S.undo.length") == u0 + 1 and msg(app).startswith("bar 1 at ")
    # a click on the marker without a move changes nothing
    app.mouse.click(ruler_x(app, "beats", m["bar1_s"]) + 1, y)
    settle(app)
    assert stored(app, song) == m and js(app, "() => S.undo.length") == u0 + 1
    # a click away from the marker puts the playhead as before
    app.mouse.click(ruler_x(app, "beats", 9.0), y)
    wait_until(lambda: abs(js(app, "() => S.playhead") - 9.0) < 0.05, msg="the playhead from the Bars|Beats ruler")
    assert stored(app, song) == m


def test_the_page_stores_the_map_the_command_stores(app, song, make_project):
    """Agent parity: Set Bar 1 at Playhead on the page and tempo_map_set with the same map store the same map."""
    set_map(app, song, MAP)
    app.evaluate("() => { S.sel = null; S.playhead = 2.025; }")
    menu(app, "set_bar1")
    settle(app)
    page_map = stored(app, song)
    other = open_project(app, make_project(seconds=12.0))
    r = invoke(app, "tempo_map_set", sid=other, map={**MAP, "bar1_s": 2.025})
    assert r["changed"] is True
    assert stored(app, other) == page_map


def test_a_typed_start_tempo_refits_bar_1_to_the_hits_and_one_undo_restores_both(app, song):
    """#468: typing 92 on the Tempo ruler of the owner case (46 BPM, bar 1 at 2.555 s) moves bar 1
    onto a beat of the 92 BPM grid the kick hits sit on; the commit is one step, so one undo
    restores the tempo and bar 1 together."""
    beat = 60.0 / 92.0
    train = [{"t": round(k * beat, 6), "v": 100} for k in range(16)]   # kicks on the 92 grid from 0 s, inside the 12 s tracks
    assert invoke(app, "events_set", sid=song, name="Kick In", events=train)["saved"] is not None
    set_map(app, song, MAP46)
    u0 = js(app, "() => S.undo.length")
    dbl(app, "tempo", 0)   # the start marker of the Tempo ruler sits at 0
    app.wait_for_function("() => $('#mapEdit').value === '46'")
    type_in_field(app, "92")
    wait_until(lambda: stored(app, song)["bpm"] == 92, msg="the typed tempo with the refitted bar 1 stored")
    m = stored(app, song)
    off = m["bar1_s"] % beat
    off = min(off, beat - off)
    assert m["bpm"] == 92 and off * 1000 <= 5.0, m   # bar 1 on a beat of the 92 grid, within 5 ms
    assert rest(m) == {"bpm": 92, "steps": [], "meter": [4, 4], "changes": []}, m
    assert js(app, "() => S.undo.length") == u0 + 1 and js(app, "() => S.dirty") is True
    assert js(app, "() => S.undo[S.undo.length - 1].type") == "tempo"
    assert "bar 1 moved from 2.555 s to" in msg(app), msg(app)
    # the grid comes from the new map: the first bar line is the new bar 1
    assert abs(js(app, "() => tmGrid().bars[0]") - m["bar1_s"]) < 1e-6
    # one undo restores the tempo and bar 1 together, redo moves both again
    menu(app, "undo")
    settle(app)
    wait_until(lambda: stored(app, song) == MAP46, msg="undo restores the tempo and bar 1")
    menu(app, "redo")
    settle(app)
    wait_until(lambda: stored(app, song)["bar1_s"] == m["bar1_s"], msg="redo")


def test_a_map_with_steps_keeps_bar_1_when_the_start_tempo_is_typed(app, song):
    """#468: a map with steps keeps bar 1 (the status line says so); the tempo changes alone."""
    set_map(app, song, MAP)   # 118 BPM, a step at 8 s, bar 1 at 2.152 s
    train = [{"t": round(k * 0.5, 6), "v": 100} for k in range(20)]
    assert invoke(app, "events_set", sid=song, name="Kick In", events=train)["saved"] is not None
    app.evaluate("async () => { await tmLoad(); }")
    dbl(app, "tempo", 0)
    app.wait_for_function("() => $('#mapEdit').value === '118'")
    type_in_field(app, "96")
    m = stored(app, song)
    assert m["bpm"] == 96 and m["bar1_s"] == 2.152, m
    assert m["steps"] == MAP["steps"] and m["meter"] == MAP["meter"] and m["changes"] == MAP["changes"], m
    assert "bar 1 stays: the map has steps" in msg(app), msg(app)


def test_set_bar_1_at_audio_start_sets_0_and_undoes(app, song):
    """#468: Track > Set Bar 1 at Audio Start puts bar 1 at 0 s (the DAW convention); one undo step."""
    set_map(app, song, MAP)
    u0 = js(app, "() => S.undo.length")
    assert menu(app, "set_bar1_audio") is True
    settle(app)
    m = stored(app, song)
    assert m["bar1_s"] == 0 and rest(m) == rest(MAP), m
    assert msg(app) == "bar 1 at 0.000 s"
    assert js(app, "() => S.undo.length") == u0 + 1 and js(app, "() => S.dirty") is True
    assert js(app, "() => S.undo[S.undo.length - 1].type") == "tempo"
    menu(app, "undo")
    settle(app)
    wait_until(lambda: stored(app, song) == MAP, msg="undo of Set Bar 1 at Audio Start")
