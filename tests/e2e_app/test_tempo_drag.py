"""#173: a drag of a tempo step marker on the Tempo ruler moves that tempo change. The pointer delta is
the move (no snap: a drag of 200 ms moves the step 200 ms, +-1 ms at the 500 ms view), clamped between
the neighbours with the map's 1 ms gap; the preview map redraws the ruler, the bars and the grid at
once; the drop is one tmCommit, so one undo step restores the map byte-identical; the MIDI export after
the drag writes the moved tempo event (tools/check_exports.py clean). No new service command: the page
sends the whole map with `tempo_map_set`, as every tempo edit. The start marker is the tempo in force,
not a change: not draggable (as Delete refuses it)."""
import sys
from pathlib import Path

from app_support import invoke, menu, wait_until
from test_follow_strip import IDLE
from test_tempo_map import js, msg, ruler_x, ruler_y, set_hits, set_map, settle, song, stored  # noqa: F401 (song is a fixture)

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
import check_exports  # noqa: E402

MAP = {"bpm": 120, "steps": [{"t": 4.0, "bpm": 90}, {"t": 8.0, "bpm": 100}], "meter": [4, 4], "changes": [], "bar1_s": 0.0}


def show_tempo(app):
    app.evaluate("() => { TM.ui.tempo = true; TM.ui.beats = true; tmApplyUi(true); }")
    app.wait_for_function(IDLE)


def frames(app):
    app.wait_for_function("() => !FR.raf && FR.flags === 0")


def tick_ink(app, t):
    """True when the Bars|Beats ruler has a line at time t (a bar or a beat tick at the bottom row)."""
    return js(app, """(t) => {
        const c = TM.cans.beats, d = devicePixelRatio, ctx = c.getContext('2d'), H = TM_H;
        const x = Math.round(tToX(t, GEO.rulerW));
        return ctx.getImageData(Math.round(x * d), Math.round((H - 2) * d), 1, 1).data[3] > 0; }""", t)


def test_a_drag_of_a_tempo_marker_moves_that_step_the_grid_follows_and_one_undo_restores_the_map(app, song):
    set_map(app, song, MAP)
    show_tempo(app)
    u0 = js(app, "() => S.undo.length")
    assert abs(js(app, "() => tmGrid().bars[3]") - 20 / 3) < 1e-6   # bar 4 at 4 s + 4 beats of 2/3 s
    y = ruler_y(app, "tempo")
    app.mouse.move(ruler_x(app, "tempo", 4.0), y)
    app.mouse.down()
    app.mouse.move(ruler_x(app, "tempo", 4.2), y, steps=4)   # a drag of 200 ms
    frames(app)
    px = js(app, "() => viewDur() / GEO.rulerW")   # one pixel in seconds
    tp = js(app, "() => TM.map.steps[0].t")
    assert abs(tp - 4.2) <= 2 * px, "the map follows the drag"
    assert abs(js(app, "() => TM.map.steps[1].t") - 8.0) < 1e-12, "the other step stays"
    # the bars and the grid redraw at once: bar 4 follows the dragged step, the old line is gone
    t9 = tp + (1 - (tp - 4.0) * 120 / 60) * 60 / 90   # bar 3's beat 2: the split quarter's rest at 90 BPM
    bar4 = t9 + 3 * (60 / 90)                          # then 3 full beats of 2/3 s
    assert abs(js(app, "() => tmGrid().bars[3]") - bar4) < 1e-6
    assert abs(js(app, "() => tmGrid().t[9]") - t9) < 1e-6
    assert tick_ink(app, bar4) and not tick_ink(app, 20 / 3)
    app.mouse.up()
    settle(app)
    m = stored(app, song)
    assert abs(m["steps"][0]["t"] - tp) < 1e-12 and m["steps"][0]["bpm"] == 90, m
    assert m["steps"][1] == {"t": 8.0, "bpm": 100} and m["bpm"] == 120, m
    assert m["meter"] == [4, 4] and m["changes"] == [] and m["bar1_s"] == 0.0, m
    assert js(app, "() => S.undo.length") == u0 + 1 and js(app, "() => S.dirty") is True
    assert js(app, "() => S.undo[S.undo.length - 1].type") == "tempo"
    assert msg(app) == f"tempo step 90 BPM at {m['steps'][0]['t']:.3f} s"
    # one undo step restores the map byte-identical
    menu(app, "undo")
    settle(app)
    wait_until(lambda: stored(app, song) == MAP, msg="undo restores the map")
    assert abs(js(app, "() => TM.map.steps[0].t") - 4.0) < 1e-12
    assert abs(js(app, "() => tmGrid().bars[3]") - 20 / 3) < 1e-6


def test_a_drag_past_the_next_marker_is_clamped_and_so_is_a_drag_before_zero(app, song):
    set_map(app, song, MAP)
    show_tempo(app)
    y = ruler_y(app, "tempo")
    app.mouse.move(ruler_x(app, "tempo", 4.0), y)
    app.mouse.down()
    app.mouse.move(ruler_x(app, "tempo", 9.0), y, steps=4)   # past the next marker at 8.0
    assert js(app, "() => TM.map.steps[0].t") == 8.0 - 0.001, "clamped 1 ms before the next marker"
    app.mouse.up()
    settle(app)
    m = stored(app, song)
    assert abs(m["steps"][0]["t"] - 7.999) < 1e-9 and abs(m["steps"][1]["t"] - 8.0) < 1e-12, m
    menu(app, "undo")
    settle(app)
    wait_until(lambda: stored(app, song) == MAP, msg="undo of the clamp")
    # the only step dragged left over 0: clamped 1 ms after 0 (the map's rule, MIN_STEP_GAP_S)
    set_map(app, song, {"bpm": 120, "steps": [{"t": 4.0, "bpm": 90}]})
    show_tempo(app)
    app.mouse.move(ruler_x(app, "tempo", 4.0), y)
    app.mouse.down()
    app.mouse.move(ruler_x(app, "tempo", 0.0) - 30, y, steps=4)   # well past the ruler's left edge
    assert js(app, "() => TM.map.steps[0].t") == 0.001
    app.mouse.up()
    settle(app)
    m = stored(app, song)
    assert abs(m["steps"][0]["t"] - 0.001) < 1e-9 and m["bpm"] == 120, m


def test_a_drag_of_200_ms_at_the_500_ms_view_moves_the_step_200_ms(app, song):
    set_map(app, song, {"bpm": 120, "steps": [{"t": 4.1, "bpm": 90}]})
    app.evaluate("() => { TM.ui.tempo = true; tmApplyUi(true); setView(4.0, 4.5); }")
    app.wait_for_function(IDLE)
    y = ruler_y(app, "tempo")
    app.mouse.move(ruler_x(app, "tempo", 4.1), y)
    app.mouse.down()
    app.mouse.move(ruler_x(app, "tempo", 4.3), y, steps=4)
    app.mouse.up()
    settle(app)
    m = stored(app, song)
    assert abs(m["steps"][0]["t"] - 4.3) <= 0.001, m   # the gate: +-1 ms at the 500 ms view
    assert m["bpm"] == 120 and m["steps"][0]["bpm"] == 90, m


def test_a_click_without_a_move_changes_nothing_and_the_start_marker_is_not_dragged(app, song):
    set_map(app, song, MAP)
    show_tempo(app)
    u0 = js(app, "() => S.undo.length")
    y = ruler_y(app, "tempo")
    app.mouse.click(ruler_x(app, "tempo", 4.0) + 1, y)   # a click selects, no drag, no undo step
    settle(app)
    assert stored(app, song) == MAP and js(app, "() => S.undo.length") == u0
    assert js(app, "() => TM.sel && TM.sel.id") == "s0"
    app.mouse.move(ruler_x(app, "tempo", 0.0) + 1, y)   # the start marker: a press and a move drag nothing
    app.mouse.down()
    app.mouse.move(ruler_x(app, "tempo", 2.0), y, steps=4)
    app.mouse.up()
    settle(app)
    assert stored(app, song) == MAP and js(app, "() => S.undo.length") == u0


def test_a_tempo_step_marker_shows_the_drag_cursor_and_the_start_marker_none(app, song):
    set_map(app, song, MAP)
    show_tempo(app)
    y = ruler_y(app, "tempo")
    cur = lambda: js(app, "() => getComputedStyle(TM.cans.tempo).cursor")
    app.mouse.move(ruler_x(app, "tempo", 6.0), y)
    assert not js(app, "() => TM.cans.tempo.classList.contains('onMarker')") and cur() != "ew-resize"
    app.mouse.move(ruler_x(app, "tempo", 4.0) + 1, y)
    assert js(app, "() => TM.cans.tempo.classList.contains('onMarker')") and cur() == "ew-resize"
    app.mouse.move(ruler_x(app, "tempo", 0.0) + 1, y)   # the start marker is not draggable
    assert not js(app, "() => TM.cans.tempo.classList.contains('onMarker')")
    app.mouse.move(ruler_x(app, "tempo", 6.0), y)
    assert not js(app, "() => TM.cans.tempo.classList.contains('onMarker')")


def test_the_midi_export_after_the_drag_writes_the_moved_tempo_event_and_the_checker_is_clean(app, song):
    set_map(app, song, MAP)
    set_hits(app, song, "Kick In", [0.5, 1.5, 2.5])
    show_tempo(app)
    drag_t = 4.2
    y = ruler_y(app, "tempo")
    app.mouse.move(ruler_x(app, "tempo", 4.0), y)
    app.mouse.down()
    app.mouse.move(ruler_x(app, "tempo", drag_t), y, steps=4)
    app.mouse.up()
    settle(app)
    m = stored(app, song)
    px = js(app, "() => viewDur() / GEO.rulerW")
    assert abs(m["steps"][0]["t"] - drag_t) <= 2 * px, m
    invoke(app, "project_save", sid=song)   # the checker reads the document from disk
    r = invoke(app, "export_all", sid=song)   # both tracks: the checker wants a file per track
    assert not r.get("errors"), r
    folder = invoke(app, "project_get", sid=song)["folder"]
    rep = check_exports.check_project(folder)
    problems = rep["problems"] + [p for tr in rep["tracks"].values() for p in tr["problems"]]
    assert problems == [], rep
    # the file itself: the moved step as a set_tempo at the dragged time, the other step and the start untouched
    ppq, tempos, meta, notes, problems = check_exports.midi_notes(str(Path(folder, "Exports", "Kick In_spike.mid")))
    assert not problems and len(tempos) == 3, (tempos, problems)
    assert tempos[0] == (0, round(60e6 / 120)), tempos
    assert abs(float(meta["seconds"](tempos[1][0])) - m["steps"][0]["t"]) <= 0.001, tempos
    assert tempos[1][1] == round(60e6 / 90), tempos
    assert abs(float(meta["seconds"](tempos[2][0])) - 8.0) <= 0.001 and tempos[2][1] == round(60e6 / 100), tempos
