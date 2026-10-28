"""P4-40b: modifier cursors and Option+Shift-click delete. The lane cursor says what a click does: #lanesScroll carries
curAdd (Option held) or curDelete (Option+Shift over a hit); the spike lane over a hit has .onHit (resize cursor); the bar-1
marker of the Bars|Beats ruler has .onMarker; the Match and Add lines of the Similarity histogram give .onLine. Option+Shift-click
on a hit deletes it as one undo step, as the Delete key. Each test has a planted-fault check (the stash test of the card)."""
from app_support import invoke, menu, open_project, wait_until
from test_learn_filter import NAME as LNAME, idle, lrf  # noqa: F401  (the fixture)
from test_menus import with_hits
from test_tempo_bar1 import MAP
from test_tempo_map import js, ruler_x, ruler_y, set_map, settle, song  # noqa: F401  (song is a fixture)
from test_follow_strip import IDLE

CLASSES = """() => { const c = document.getElementById('lanesScroll').classList; return [c.contains('curAdd'), c.contains('curDelete')]; }"""


def spike_xy(app, name, t):
    """Client coordinates of time t on the spike lane of a track."""
    return app.evaluate("""([n, t]) => { const r = document.querySelector(`.track[data-name="${n}"] canvas.spike`).getBoundingClientRect();
      return [GEO.laneX + tToX(t, GEO.laneW), r.top + r.height / 2]; }""", [name, t])


def cursor(app, name, kind="spike"):
    return app.evaluate("n => getComputedStyle(document.querySelector(`.track[data-name=\"${n}\"] canvas.%s`)).cursor" % kind, name)


def setup(app, make_project):
    sid = open_project(app, make_project(seconds=8.0))
    with_hits(app, sid, {"Kick In": [1.0, 3.0, 5.0], "Snare Top": [2.0]})
    app.evaluate("() => { S.playhead = 0.1; setView(0, 8); }")
    app.wait_for_function("() => !FR.raf && FR.flags === 0")
    return sid


def test_the_lane_cursor_follows_the_modifier_keys_and_the_pointer(app, make_project):
    setup(app, make_project)
    x, y = spike_xy(app, "Kick In", 3.0)
    xe, _ = spike_xy(app, "Kick In", 4.0)                       # no hit there
    app.mouse.move(xe, y)
    assert js(app, CLASSES) == [False, False] and "crosshair" in cursor(app, "Kick In")
    app.mouse.move(x, y)                                        # over a hit: it drags
    assert js(app, CLASSES) == [False, False] and cursor(app, "Kick In") == "ew-resize"
    app.keyboard.down("Alt")                                    # Option: add
    assert js(app, CLASSES) == [True, False] and cursor(app, "Kick In") == "copy"
    assert cursor(app, "Kick In", "wave") == "copy"
    app.keyboard.down("Shift")                                  # Option+Shift over a hit: delete
    assert js(app, CLASSES) == [False, True] and "data:image/svg" in cursor(app, "Kick In")
    assert cursor(app, "Kick In", "wave") != "copy"
    app.mouse.move(xe, y)                                       # Option+Shift off a hit: nothing to delete
    assert js(app, CLASSES) == [False, False]
    app.mouse.move(x, y)
    assert js(app, CLASSES) == [False, True]
    app.keyboard.up("Shift")                                    # the key alone (no mouse move) changes the cursor
    assert js(app, CLASSES) == [True, False]
    app.keyboard.up("Alt")
    assert js(app, CLASSES) == [False, False] and cursor(app, "Kick In") == "ew-resize"
    app.mouse.move(x, 5)                                        # off the lanes: no leftover hit state
    app.keyboard.down("Alt"); app.keyboard.down("Shift")
    assert js(app, CLASSES)[1] is False
    app.keyboard.up("Shift"); app.keyboard.up("Alt")


def test_a_followed_hit_shows_no_delete_cursor(app, make_project):
    setup(app, make_project)
    x, y = spike_xy(app, "Kick In", 3.0)
    app.evaluate("() => { followerLocked = () => true; }")      # a follower's hits are not deleted here
    app.mouse.move(x - 20, y); app.mouse.move(x, y)
    app.keyboard.down("Alt"); app.keyboard.down("Shift")
    assert js(app, CLASSES) == [False, False]
    app.keyboard.up("Shift"); app.keyboard.up("Alt")


def test_option_shift_click_on_a_hit_deletes_it_as_one_undo_step(app, make_project):
    sid = setup(app, make_project)
    u0 = js(app, "() => S.undo.length")
    x, y = spike_xy(app, "Kick In", 3.0)
    app.keyboard.down("Alt"); app.keyboard.down("Shift")
    app.mouse.click(x, y)
    app.keyboard.up("Shift"); app.keyboard.up("Alt")
    wait_until(lambda: js(app, "() => S.events['Kick In'].map(e => e.t)") == [1.0, 5.0], msg="the hit is deleted")
    wait_until(lambda: [e["t"] for e in invoke(app, "events_get", sid=sid, name="Kick In")["events"]] == [1.0, 5.0], msg="the service")
    assert js(app, "() => S.undo.length") == u0 + 1 and js(app, "() => S.undo[S.undo.length - 1].type") == "del"
    assert js(app, "() => S.playhead") == 0.1                   # not the Shift click: the playhead stays
    menu(app, "undo")                                           # one undo gives it back
    wait_until(lambda: js(app, "() => S.events['Kick In'].map(e => e.t)") == [1.0, 3.0, 5.0], msg="undo")


def test_the_delete_key_and_the_other_clicks_keep_their_bindings(app, make_project):
    setup(app, make_project)
    u0 = js(app, "() => S.undo.length")
    x, y = spike_xy(app, "Kick In", 3.0)
    xe, _ = spike_xy(app, "Kick In", 6.5)
    app.keyboard.down("Shift"); app.mouse.click(x, y); app.keyboard.up("Shift")       # Shift-click: the playhead
    assert abs(js(app, "() => S.playhead") - 3.0) < 0.05 and js(app, "() => S.events['Kick In'].length") == 3
    app.keyboard.down("Alt"); app.mouse.click(xe, y); app.keyboard.up("Alt")          # Option-click: adds
    wait_until(lambda: js(app, "() => S.events['Kick In'].length") == 4, msg="Option-click adds")
    app.keyboard.down("Alt"); app.keyboard.down("Shift"); app.mouse.click(xe + 60, y); app.keyboard.up("Shift"); app.keyboard.up("Alt")
    assert js(app, "() => S.events['Kick In'].length") == 4                           # Option+Shift off a hit deletes nothing
    assert abs(js(app, "() => S.playhead") - js(app, "t => xToT(t, GEO.laneW)", xe + 60 - app.evaluate("() => GEO.laneX")) ) < 0.05   # it stays the Shift click
    app.mouse.click(x, y)                                                             # a plain click selects
    assert js(app, "() => S.sel") == {"track": "Kick In", "index": 1}
    app.keyboard.press("Delete")                                                      # the Delete key: one undo step
    wait_until(lambda: js(app, "() => S.events['Kick In'].length") == 3, msg="Delete")
    assert js(app, "() => S.undo.length") == u0 + 2                                   # add + delete


def test_the_bar_1_marker_shows_the_resize_cursor(app, song):
    set_map(app, song, MAP)
    app.evaluate("() => { TM.ui.beats = true; tmApplyUi(true); }")
    app.wait_for_function(IDLE)
    y = ruler_y(app, "beats")
    cur = lambda: js(app, "() => getComputedStyle(TM.cans.beats).cursor")
    app.mouse.move(ruler_x(app, "beats", 6.0), y)
    assert not js(app, "() => TM.cans.beats.classList.contains('onMarker')") and cur() != "ew-resize"
    app.mouse.move(ruler_x(app, "beats", 2.152) + 1, y)
    assert js(app, "() => TM.cans.beats.classList.contains('onMarker')") and cur() == "ew-resize"
    app.mouse.move(ruler_x(app, "beats", 6.0), y)
    assert not js(app, "() => TM.cans.beats.classList.contains('onMarker')")


def test_the_match_and_add_lines_of_the_histogram_show_the_resize_cursor(lrf):
    page, sid = lrf
    js(page, "n => { lnSetSel(n, 5.0, 5.4); }", LNAME)
    page.keyboard.press("k")                                  # an example on an empty range: the Add line shows
    idle(page)
    page.locator("#insParams .lnFilterBox").check()
    idle(page)
    page.wait_for_function("() => (document.querySelector('#insParams .lnHistInfo') || {textContent:''}).textContent.includes('Add 0.85')")
    page.locator("#insParams .lnHist").scroll_into_view_if_needed()
    box = page.locator("#insParams .lnHist").bounding_box()
    pos = js(page, "n => [LN_SC[lnTarget(n)].threshold, lnAddOf(n)]", LNAME)
    y = box["y"] + box["height"] / 2
    off = lambda: page.evaluate("() => document.querySelector('#insParams .lnHist').classList.contains('onLine')")
    cur = lambda: page.evaluate("() => getComputedStyle(document.querySelector('#insParams .lnHist')).cursor")
    for frac in pos:                                          # on each line
        page.mouse.move(box["x"] + 1, y)
        page.mouse.move(box["x"] + frac * box["width"], y)
        assert off() and cur() == "col-resize", (frac, pos)
    gap = [f for f in (0.02, 0.5, 0.98) if all(abs(f - p) * box["width"] > 12 for p in pos)][0]
    page.mouse.move(box["x"] + gap * box["width"], y)
    assert not off() and cur() == "crosshair"
