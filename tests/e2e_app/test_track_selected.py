"""P4-39c: a project with tracks always has a selected track (the track of the Inspector, the key commands, Learn and
the follow menus). The first track is selected at open; a click on empty lane space, Escape and lnSetSel(null) clear the
hit selection only; a selected track that leaves passes the selection to the next track in order, else the previous one;
a project without tracks keeps the empty state of the Inspector."""
import re

from playwright.sync_api import expect

from app_support import open_project
from test_menus import empty_page, with_hits

THREE = (("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Tom 1", "tom", 48))


def sel(page):
    return page.evaluate("() => [S.selTrack, document.querySelector('#insName').value, S.sel]")


def test_the_first_track_is_selected_after_open_and_the_inspector_shows_it(app, make_project):
    sid = open_project(app, make_project(tracks=THREE))
    app.evaluate("() => setInspector(true)")
    app.wait_for_function("() => S.selTrack === 'Kick In' && document.querySelector('#insName').value === 'Kick In'")
    expect(app.locator("#insEmpty")).to_be_hidden()
    expect(app.locator("#insBody")).to_be_visible()
    expect(app.locator(".track[data-name='Kick In']")).to_have_class(re.compile(r"selTrack"))
    # a second click on the selected header keeps it; a click on another header moves it
    app.locator(".track[data-name='Kick In'] .thead .tnote").click()
    assert app.evaluate("() => S.selTrack") == "Kick In"
    app.locator(".track[data-name='Snare Top'] .thead .tnote").click()
    app.wait_for_function("() => S.selTrack === 'Snare Top'")
    # an analysis or a reload of the lanes keeps the selection; a new project selects its first track
    with_hits(app, sid, {"Snare Top": [1.0, 2.0]})
    assert app.evaluate("() => S.selTrack") == "Snare Top"
    open_project(app, make_project(tracks=(("Tom 2", "tom", 47), ("Tom 3", "tom", 45))))
    app.wait_for_function("() => S.selTrack === 'Tom 2' && document.querySelector('#insName').value === 'Tom 2'")


def test_escape_and_a_click_on_empty_lane_space_clear_the_hit_only(app, make_project):
    sid = open_project(app, make_project(tracks=THREE))
    with_hits(app, sid, {"Kick In": [0.5, 1.5], "Snare Top": [1.0]})
    app.evaluate("() => setInspector(true)")
    app.evaluate("() => { S.sel = { track: 'Kick In', index: 0 }; updateStatus(); }")
    assert sel(app)[2] == {"track": "Kick In", "index": 0}
    app.keyboard.press("Escape")
    app.wait_for_function("() => S.sel === null")
    assert sel(app)[:2] == ["Kick In", "Kick In"]
    # a click on empty lane space (far from every hit) takes the range selection away; the track stays selected
    app.evaluate("() => lnSetSel('Kick In', 0.4, 0.6)")
    assert app.evaluate("() => LN.sel !== null") is True
    box = app.locator(".track[data-name='Kick In'] canvas.wave").bounding_box()
    app.mouse.click(box["x"] + box["width"] * 0.9, box["y"] + box["height"] / 2)
    app.wait_for_function("() => LN.sel === null")
    assert sel(app)[:2] == ["Kick In", "Kick In"]
    app.evaluate("() => { S.sel = { track: 'Kick In', index: 1 }; updateStatus(); }")
    spike = app.locator(".track[data-name='Kick In'] canvas.spike").bounding_box()
    app.mouse.click(spike["x"] + spike["width"] * 0.9, spike["y"] + spike["height"] / 2)
    assert sel(app)[:2] == ["Kick In", "Kick In"]
    # lnSetSel(null) is the Learn range selection: the track stays too
    app.evaluate("() => { lnSetSel('Kick In', 0.4, 0.6); lnSetSel(null); }")
    assert sel(app)[:2] == ["Kick In", "Kick In"]
    expect(app.locator("#insEmpty")).to_be_hidden()


def test_a_selected_track_that_leaves_passes_the_selection_on(app, make_project):
    sid = open_project(app, make_project(tracks=THREE))
    app.evaluate("() => setInspector(true)")

    def leave(name):
        """The track leaves the project the way a removal or a hide leaves the page: tracks and order lose it."""
        app.evaluate("n => { delete S.tracks[n]; S.order = S.order.filter(x => x !== n); updateStatus(); }", name)

    app.locator(".track[data-name='Snare Top'] .thead .tnote").click()
    app.wait_for_function("() => S.selTrack === 'Snare Top'")
    leave("Snare Top")
    app.wait_for_function("() => S.selTrack === 'Tom 1' && document.querySelector('#insName').value === 'Tom 1'")   # the next in order
    leave("Tom 1")
    app.wait_for_function("() => S.selTrack === 'Kick In' && document.querySelector('#insName').value === 'Kick In'")   # none after it: the previous
    expect(app.locator("#insEmpty")).to_be_hidden()
    # setSelTrack(null) is not a way to none
    app.evaluate("() => setSelTrack(null)")
    assert app.evaluate("() => S.selTrack") == "Kick In"


def test_a_project_without_tracks_shows_the_empty_inspector(app, make_project):
    open_project(app, make_project(tracks=THREE))
    app.evaluate("() => setInspector(true)")
    assert app.evaluate("() => S.selTrack") == "Kick In"
    empty_page(app)
    assert app.evaluate("() => S.selTrack") is None
    expect(app.locator("#insEmpty")).to_be_visible()
    expect(app.locator("#insBody")).to_be_hidden()
    assert "No track selected" in app.evaluate("() => document.body.innerText")
