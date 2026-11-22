"""#562: in host mode the page follows the selection of the DAW (DAW plugin M1b13).

The plugin turns what the host selected (region sequences, playback regions, a time range) and what it hides into the payload of the
event `plugin-selection` (plugins/ara/src/selection.rs). static/hostsel.js shows it: the selected track and the marks of the others, the
range of the host as a band that is not the hit selection of the page, the hidden tracks with one text, the switch in the page menu, and
the rule that a pointer or key action of the user is never overwritten. These tests start the page the way the plugin shell does
(GET /?host_mode=1) and send the payloads the plugin sends. The plugin side has its own stages (plugins/ara/check.py, editor-selection).
With the mode off nothing exists (the last tests)."""
import pytest

from app_support import open_project
from test_host_page import FOUR, host_open
from test_host_menu import group, open_menu

NAMES = [n for n, *_ in FOUR]


def payload(tracks=(), range_=None, hidden=(), n=None, none=False):
    n = payload.n = (payload.n + 1) if n is None else n
    return {"n": n, "seen": True, "tracks": list(tracks), "none": none, "range": range_, "range_why": "ok" if range_ else "none",
            "hidden": list(hidden)}


payload.n = 0


def send(page, p):
    page.evaluate("p => DSHostSel.event(p)", p)
    page.wait_for_function("() => !FR.raf && FR.flags === 0")


def state(page):
    return page.evaluate("""() => ({ sel: S.selTrack, marked: DSHostSel.state().marked, hidden: DSHostSel.state().hidden, note: DSHostSel.state().note,
      range: DSHostSel.state().range, pending: DSHostSel.state().pending, own: LN.sel, hit: S.sel, view: [S.view.t0, S.view.t1], follow: DSHostSel.follows() })""")


def band(page):
    return page.evaluate("""() => { const b = document.getElementById('hostRange'); const cs = getComputedStyle(b), r = b.getBoundingClientRect(), e = document.getElementById('editor').getBoundingClientRect();
      return { shown: cs.display !== 'none', left: r.left - e.left, width: r.width, top: r.top - e.top, bottom: r.bottom - e.top, h: e.height,
        want0: S.theadW + tToX(DSHostSel.state().range ? DSHostSel.state().range.t0 : 0, laneWidth()) }; }""")


@pytest.fixture
def host(host_app, make_project):
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.evaluate("() => { DSHostSel.setFollow(true); setView(0, 6); }")
    host_app.evaluate("() => { DSHostSel.event({ n: 0, seen: false, tracks: [], none: false, range: null, hidden: [] }); }")
    payload.n = 10
    yield host_app
    host_app.evaluate("() => { DSHostSel.setFollow(false); DSHostSel.setFollow(true); localStorage.removeItem('dsHostFollowSel'); setTrackHeight('medium'); }")


def test_the_selected_track_of_the_daw_is_the_selected_track_of_the_page_and_scrolls_into_view(host):
    host.set_viewport_size({"width": 1400, "height": 640})
    host.evaluate("() => { setTrackHeight('large'); setInspector(false, false); }")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    sc = lambda: host.evaluate("""n => { const row = document.querySelector(`.track[data-name="${n}"]`), c = document.getElementById('lanesScroll').getBoundingClientRect(), r = row.getBoundingClientRect();
      return { inside: r.top >= c.top - 1 && r.bottom <= c.bottom + 1, top: document.getElementById('lanesScroll').scrollTop }; }""", NAMES[3])
    assert host.evaluate("() => document.getElementById('lanesScroll').scrollHeight > document.getElementById('lanesScroll').clientHeight"), "the test needs rows that do not all fit"
    assert sc()["inside"] is False
    send(host, payload([NAMES[3]]))
    st = state(host)
    assert st["sel"] == NAMES[3] and st["marked"] == [] and st["range"] is None
    host.wait_for_function("() => document.getElementById('lanesScroll').scrollTop > 0")
    assert sc()["inside"] is True, "the selected track scrolls into view"
    send(host, payload([NAMES[0]]))
    assert state(host)["sel"] == NAMES[0]
    host.wait_for_function("() => document.getElementById('lanesScroll').scrollTop === 0")


def test_several_tracks_select_the_first_and_mark_the_others(host):
    send(host, payload([NAMES[2], NAMES[0], NAMES[1]]))
    st = state(host)
    assert st["sel"] == NAMES[2] and st["marked"] == [NAMES[0], NAMES[1]]
    assert host.evaluate("() => [...document.querySelectorAll('.track')].filter(r => getComputedStyle(r).outlineStyle === 'dashed').map(r => r.dataset.name)") == [NAMES[0], NAMES[1]]
    send(host, payload([NAMES[3]]))
    assert state(host)["marked"] == [] and state(host)["sel"] == NAMES[3]


def test_a_selection_of_a_track_with_no_source_changes_nothing(host):
    send(host, payload([NAMES[1], NAMES[2]], {"t0": 1.0, "t1": 2.0}))
    before = state(host)
    send(host, {**payload(), "tracks": [], "none": True, "range": None, "range_why": "none"})
    assert state(host) == before, "the selection, the marks, the band and the view stay"
    # a track name the page does not have: the page keeps what it has (no error)
    send(host, payload(["No Such Track"]))
    assert state(host)["sel"] == NAMES[1]


def test_the_range_of_the_daw_is_a_band_of_the_host_and_not_the_hit_selection(host):
    send(host, payload([NAMES[1]], {"t0": 1.0, "t1": 2.5}))
    b = band(host)
    st = state(host)
    assert b["shown"] and abs(b["left"] - b["want0"]) < 1.0 and b["width"] > 20
    assert b["top"] <= 1 and b["bottom"] >= b["h"] - 1, "the band spans the ruler and the lanes"
    assert st["own"] is None and st["hit"] is None, "the range of the host is not a selection of the page"
    assert host.evaluate("() => { const w = tToX(2.5, laneWidth()) - tToX(1.0, laneWidth()); return Math.abs(document.getElementById('hostRange').getBoundingClientRect().width - w) < 1.0; }")
    # it follows the view: a zoom moves and widens the band
    host.evaluate("() => setView(0.5, 3.5)")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    b2 = band(host)
    assert b2["shown"] and b2["width"] > b["width"] * 1.5 and abs(b2["left"] - b2["want0"]) < 1.0
    # an empty range, no range, and a range in no region (the plugin sends none) remove it
    send(host, payload([NAMES[1]], None))
    assert band(host)["shown"] is False and state(host)["range"] is None
    send(host, payload([NAMES[1]], {"t0": 1.0, "t1": 2.0}))
    assert band(host)["shown"] is True
    send(host, {**payload([NAMES[1]]), "range_why": "outside"})
    assert band(host)["shown"] is False
    send(host, payload([NAMES[1]], {"t0": 2.0, "t1": 2.0}))
    assert band(host)["shown"] is False, "a range of no length is none"


def test_a_range_out_of_view_scrolls_the_view_and_one_in_view_does_not(host):
    host.evaluate("() => setView(0, 1.0)")
    send(host, payload([NAMES[0]], {"t0": 4.0, "t1": 4.5}))
    v = state(host)["view"]
    assert v[0] <= 4.0 and v[1] >= 4.5 and abs((v[1] - v[0]) - 1.0) < 1e-6, v
    send(host, payload([NAMES[0]], {"t0": 4.1, "t1": 4.3}))
    assert state(host)["view"] == v, "a range that is in view leaves the view"
    # a range that is partly in view stays too
    send(host, payload([NAMES[0]], {"t0": v[1] - 0.1, "t1": v[1] + 0.5}))
    assert state(host)["view"] == v
    # a range longer than the view: its start is at the left
    send(host, payload([NAMES[0]], {"t0": 0.2, "t1": 3.5}))
    v2 = state(host)["view"]
    assert v2[0] <= 0.2 and abs((v2[1] - v2[0]) - 1.0) < 1e-6


def test_hidden_tracks_are_hidden_with_one_short_text_that_names_how_many(host):
    send(host, payload([NAMES[0]], None, [NAMES[1], NAMES[2]]))
    st = state(host)
    assert st["hidden"] == [NAMES[1], NAMES[2]] and st["note"] == "2 tracks hidden in the DAW"
    shown = host.evaluate("() => [...document.querySelectorAll('.track')].filter(r => r.getBoundingClientRect().height > 0).map(r => r.dataset.name)")
    assert shown == [NAMES[0], NAMES[3]]
    assert host.evaluate("() => { const n = document.getElementById('hostHiddenNote'); return [n.classList.contains('hidden'), n.getBoundingClientRect().width > 0]; }") == [False, True]
    # the rows that stay are measured and drawn at their new places
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    assert host.evaluate("() => ['" + NAMES[0] + "', '" + NAMES[3] + "'].every(n => { const g = GEO.rows[n]; return g && g.waveH > 0 && Math.abs(g.top - (document.querySelector(`.track[data-name=\"${n}\"]`).getBoundingClientRect().top - GEO.scY + GEO.top)) < 2; })")
    send(host, payload([NAMES[0]], None, [NAMES[3]]))
    assert state(host)["note"] == "1 track hidden in the DAW" and state(host)["hidden"] == [NAMES[3]]
    assert state(host)["sel"] == NAMES[0], "a change of the hidden list alone does not select"
    send(host, payload([NAMES[0]], None, []))
    st = state(host)
    assert st["hidden"] == [] and st["note"] == ""
    assert host.evaluate("() => document.getElementById('hostHiddenNote').classList.contains('hidden')")


def test_the_follow_has_a_switch_in_the_menu_of_the_page(host):
    send(host, payload([NAMES[1], NAMES[2]], {"t0": 1.0, "t1": 2.0}, [NAMES[3]]))
    open_menu(host)
    group(host, "View")
    item = host.locator('#hostMenu [data-host="host_follow_selection"]')
    assert item.get_attribute("aria-checked") == "true"
    item.click()
    host.wait_for_selector("#hostMenu.hidden", state="attached")
    assert not host.evaluate("() => DSHostSel.follows()")
    st = state(host)
    assert st["marked"] == [] and st["hidden"] == [] and band(host)["shown"] is False and st["sel"] == NAMES[1], "off: the marks, the band and the hidden rows go, the selection stays"
    send(host, payload([NAMES[0]], {"t0": 0.5, "t1": 1.0}, [NAMES[2]]))
    st = state(host)
    assert st["sel"] == NAMES[1] and st["hidden"] == [] and band(host)["shown"] is False, "off: nothing follows"
    open_menu(host)
    group(host, "View")
    host.locator('#hostMenu [data-host="host_follow_selection"]').click()
    host.wait_for_selector("#hostMenu.hidden", state="attached")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    st = state(host)
    assert st["sel"] == NAMES[0] and st["hidden"] == [NAMES[2]] and band(host)["shown"] is True, "on: the page takes the last selection of the host"


def test_the_menu_item_is_not_in_the_command_table_and_the_audit_stays_clean(host):
    assert host.evaluate("() => DSHostMenu.audit()") == {"missing": [], "both": [], "stale": [], "unhandled": [], "noCheck": []}
    assert "host_follow_selection" not in host.evaluate("() => DSHostMenu.model().flatMap(g => g.items.map(i => i.id))")


def test_a_pointer_action_of_the_user_is_never_overwritten(host):
    send(host, payload([NAMES[0]]))
    host.mouse.move(700, 300)
    host.mouse.down()
    assert host.evaluate("() => DSHostSel.busy()")
    send(host, payload([NAMES[2]], {"t0": 1.0, "t1": 2.0}, [NAMES[3]]))
    st = state(host)
    assert st["sel"] == NAMES[0] and st["range"] is None and st["pending"] is True, "the selection of the host waits while the button is down"
    host.mouse.up()
    host.wait_for_function("() => S.selTrack === '" + NAMES[2] + "'")
    st = state(host)
    assert st["range"] == {"t0": 1.0, "t1": 2.0} and st["hidden"] == [NAMES[3]] and st["pending"] is False, "when the action ends the page takes it"


def test_a_selection_the_user_made_during_the_action_stays(host):
    send(host, payload([NAMES[0]]))
    host.mouse.move(700, 300)
    host.mouse.down()
    host.evaluate("n => setSelTrack(n)", NAMES[1])   # the user selects a track while the button is down
    send(host, payload([NAMES[2]], {"t0": 1.0, "t1": 2.0}, [NAMES[3]]))
    host.mouse.up()
    host.wait_for_timeout(400)
    st = state(host)
    assert st["sel"] == NAMES[1] and st["range"] is None, "the user's selection wins"
    assert st["hidden"] == [NAMES[3]], "the hidden tracks are not a selection of the user: they are applied"


def test_a_key_action_of_the_user_is_never_overwritten(host):
    send(host, payload([NAMES[0]]))
    host.keyboard.down("Shift")
    assert host.evaluate("() => DSHostSel.busy()")
    send(host, payload([NAMES[3]]))
    assert state(host)["sel"] == NAMES[0] and state(host)["pending"] is True
    host.keyboard.up("Shift")
    host.wait_for_function("() => S.selTrack === '" + NAMES[3] + "'", timeout=3000)


def test_an_older_message_is_ignored_and_a_reload_of_the_project_shows_the_state_again(host):
    send(host, payload([NAMES[1], NAMES[2]], {"t0": 1.0, "t1": 2.0}, [NAMES[3]], n=50))
    send(host, payload([NAMES[0]], None, [], n=49))
    assert state(host)["sel"] == NAMES[1], "a message older than the last one is ignored"
    host.evaluate("async () => { await loadProject(); }")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    st = state(host)
    assert st["marked"] == [NAMES[2]] and st["hidden"] == [NAMES[3]] and band(host)["shown"] is True, "the new rows get the marks and the hidden state"
    assert st["sel"] == NAMES[1], "a reload does not select again"


def test_a_click_on_a_track_in_the_page_calls_no_host_selection(host):
    """The other direction does not exist: ARA has no call for it. The page changes itself only."""
    n0 = len(host.invokes)
    host.click(f'.track[data-name="{NAMES[2]}"] .thead .tname')
    assert state(host)["sel"] == NAMES[2]
    after = host.invokes[n0:]
    assert not [c for c in after if c.startswith("plugin_host_") and c != "plugin_host_track"], after


# ---------- the standalone page ----------

def test_standalone_page_has_nothing_of_the_follow(app, make_project):
    open_project(app, make_project(tracks=FOUR, seconds=6.0))
    assert app.evaluate("() => [!!document.getElementById('hostRange'), !!document.getElementById('hostHiddenNote'), DSHostSel.ready]") == [False, False, False]
    app.evaluate("p => DSHostSel.event(p)", payload([NAMES[1]], {"t0": 1.0, "t1": 2.0}, [NAMES[2]]))
    app.evaluate("() => { DSHostSel.init(); DSHostSel.reapply(); DSHostSel.setFollow(true); }")
    assert app.evaluate("() => [document.querySelectorAll('.hostSel, .hostHidden').length, !!document.getElementById('hostRange'), S.selTrack]")[:2] == [0, False]
    assert app.evaluate("() => DSHostSel.state().payload") is None
    assert app.evaluate("() => DSHostMenu.init() || document.querySelectorAll('[data-host]').length") == 0
