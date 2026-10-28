"""P4-40g: the review markers (docs/migration/phase4-tasks.md P4-40g, openDAW O-5).

M sets a marker at the selected hit or the playhead, the Markers ruler shows them (a click puts the
playhead, a click on a flag selects it, Delete removes it, a double-click takes its note), Option+M and
Option+Shift+M jump to the next and the previous marker, and the markers live in the project document
(`markers_get`, `markers_set` with `previous` as the undo). The MIDI export of markers is point 3 of the
card and waits for the owner's Pro Tools and Logic check."""
import pytest

from app_support import edits_done, invoke, menu, open_project, wait_until
from test_menus import select_track, with_hits
from test_tempo_map import IDLE, ruler_x, ruler_y

SR = 48000


def js(page, expr, arg=None):
    return page.evaluate(expr, arg) if arg is not None else page.evaluate(expr)


def markers(app, sid):
    return invoke(app, "markers_get", sid=sid)["markers"]


def settle(app):
    """The marker writes run in order (MK.chain); wait for them and for the frame."""
    app.evaluate("async () => { await MK.chain; }")
    app.wait_for_function(IDLE)


def set_markers(app, sid, ms):
    r = invoke(app, "markers_set", sid=sid, markers=ms)
    assert r["changed"] and r["n"] == len(ms), r
    app.evaluate("async () => { await tmLoad(); }")   # the page holds a copy, as of the map
    settle(app)
    return r


def marker_row():
    return "#mapRulers .mrow[data-ruler=markers]"


def shown(app):
    return not js(app, f"() => $({marker_row()!r}).classList.contains('hidden')")


@pytest.fixture
def song(app, make_project):
    """A 12 s project with no saved ruler choice (the bridge home is shared by the tests)."""
    invoke(app, "app_settings_set", key="ruler_markers", value=None)
    sid = open_project(app, make_project(seconds=12.0))
    app.evaluate("async () => { await loadViewSettings(); }")   # the page's copy of the setting follows
    app.evaluate("() => { setView(0, 12); }")
    yield sid
    invoke(app, "app_settings_set", key="ruler_markers", value=None)


def test_m_sets_a_marker_at_the_selected_hit_or_the_playhead(app, song):
    with_hits(app, song, {"Kick In": [1.0, 2.0]})
    select_track(app, "Kick In")
    app.evaluate("() => { S.playhead = 0; S.sel = null; }")
    app.keyboard.press("Tab")                                             # select the hit at 1.0 s
    app.keyboard.press("m")                                               # M: a marker at the selected hit
    settle(app)
    assert markers(app, song) == [{"t": 1.0}]
    assert "marker at 1.000 s" in app.inner_text("#stMsg")
    assert invoke(app, "project_get", sid=song)["dirty"] is True          # the document changed
    assert shown(app)                                                     # the ruler shows the first marker
    ink = js(app, """() => {
        const c = TM.cans.markers, d = devicePixelRatio;
        return c.getContext('2d').getImageData(Math.round(tToX(1.0, GEO.rulerW) * d), Math.round(9 * d), 1, 1).data[3] > 0;
    }""")
    assert ink                                                            # a flag line at 1.0 s
    app.keyboard.press("Escape")                                          # Deselect the Hit
    app.evaluate("() => { S.playhead = 5.5; }")
    app.keyboard.press("m")                                               # no hit selected: the playhead
    settle(app)
    assert markers(app, song) == [{"t": 1.0}, {"t": 5.5}]
    edits_done(app)


def test_a_click_on_the_markers_ruler_puts_the_playhead_and_delete_removes_a_selected_marker(app, song):
    set_markers(app, song, [{"t": 4.0}])
    app.evaluate("() => { S.playhead = 0; S.sel = null; }")
    select_track(app, None)
    app.mouse.click(ruler_x(app, "markers", 6.0), ruler_y(app, "markers"))
    wait_until(lambda: abs(js(app, "() => S.playhead") - 6.0) < 0.05, msg="the playhead from the Markers ruler")
    app.mouse.click(ruler_x(app, "markers", 4.0) + 6, ruler_y(app, "markers"))   # on the flag: select it
    app.wait_for_function("() => TM.sel && TM.sel.kind === 'markers'")
    app.keyboard.press("Delete")
    settle(app)
    assert markers(app, song) == []
    assert js(app, "() => TM.sel") is None
    assert not shown(app)   # empty again: the ruler hides (until a toggle stores the choice)


def test_option_m_and_option_shift_m_jump_to_the_next_and_previous_marker(app, song):
    set_markers(app, song, [{"t": 1.0, "note": "check flam"}, {"t": 3.0}, {"t": 8.0}])
    app.evaluate("() => { S.playhead = 0; S.sel = null; }")
    app.keyboard.press("Alt+M")
    assert js(app, "() => S.playhead") == pytest.approx(1.0)
    app.keyboard.press("Alt+M")
    assert js(app, "() => S.playhead") == pytest.approx(3.0)
    app.keyboard.press("Alt+M")
    assert js(app, "() => S.playhead") == pytest.approx(8.0)
    app.keyboard.press("Alt+M")                                           # the last marker: the message says so
    assert js(app, "() => S.playhead") == pytest.approx(8.0)
    assert "no next marker" in app.inner_text("#stMsg")
    app.keyboard.press("Alt+Shift+M")
    assert js(app, "() => S.playhead") == pytest.approx(3.0)
    app.keyboard.press("Alt+Shift+M")
    assert js(app, "() => S.playhead") == pytest.approx(1.0)
    assert "check flam" in app.inner_text("#stMsg")   # the status says the marker's note
    app.keyboard.press("Alt+Shift+M")
    assert js(app, "() => S.playhead") == pytest.approx(1.0)
    assert "no previous marker" in app.inner_text("#stMsg")


def test_a_double_click_takes_the_note_of_a_marker(app, song):
    set_markers(app, song, [{"t": 4.0}])
    app.mouse.dblclick(ruler_x(app, "markers", 4.0) + 6, ruler_y(app, "markers"))
    app.wait_for_function("() => !$('#mapEdit').classList.contains('hidden') && document.activeElement === $('#mapEdit')")
    app.keyboard.type("check flam")   # the field selects its text when it opens: typing replaces it
    app.keyboard.press("Enter")
    settle(app)
    assert markers(app, song) == [{"t": 4.0, "note": "check flam"}]
    app.mouse.dblclick(ruler_x(app, "markers", 4.0) + 6, ruler_y(app, "markers"))
    app.wait_for_function("() => !$('#mapEdit').classList.contains('hidden')")
    app.keyboard.press("Escape")   # the note stays
    settle(app)
    assert markers(app, song) == [{"t": 4.0, "note": "check flam"}]


def test_the_view_menu_toggles_the_markers_ruler_and_stores_the_choice(app, song):
    set_markers(app, song, [{"t": 4.0}])
    assert shown(app)
    menu(app, "ruler_markers")
    app.wait_for_function(f"() => $({marker_row()!r}).classList.contains('hidden')")
    wait_until(lambda: invoke(app, "app_settings_get").get("ruler_markers") is False, msg="the stored choice")
    menu(app, "ruler_markers")   # a stored choice wins over the marker count
    app.wait_for_function(f"() => !$({marker_row()!r}).classList.contains('hidden')")
    assert shown(app)


def test_the_agent_api_sets_and_gets_and_previous_is_the_undo(app, song):
    set_markers(app, song, [{"t": 2.0}, {"t": 1.0, "note": "tom 2 bleed?"}])
    assert markers(app, song) == [{"t": 1.0, "note": "tom 2 bleed?"}, {"t": 2.0}]   # stored sorted by t
    r = invoke(app, "markers_set", sid=song, markers=[{"t": 3.0}])
    assert r["changed"] and r["n"] == 1 and r["previous"] == [{"t": 1.0, "note": "tom 2 bleed?"}, {"t": 2.0}]
    undo = invoke(app, "markers_set", sid=song, markers=r["previous"])   # one more call with `previous`
    assert undo["changed"] and markers(app, song) == [{"t": 1.0, "note": "tom 2 bleed?"}, {"t": 2.0}]
    invoke(app, "markers_set", sid=song, markers=[])   # an empty list clears, the key goes
    assert markers(app, song) == []
    assert invoke(app, "project_save", sid=song) is not None
    from app_support import document_path
    import json
    text = json.loads(document_path(invoke(app, "project_get", sid=song)["folder"]).read_text())
    assert "markers" not in text   # an empty list is not stored: the document is as before the feature
    assert text["format"] == 1
