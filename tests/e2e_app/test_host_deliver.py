"""#576: the drag handles of the DAW (DAW plugin M1c3).

The plugin makes the MIDI and spike files of every DAW track in the background and gives their states (plugins/ara/src/prepare.rs: the shell
command plugin_deliver_state and the page event plugin-deliver). static/deliver.js shows them as handles: three in the header of a row, four in
the toolbar. These tests send the payload the plugin sends. The plugin side has its own stages (plugins/ara/check.py, deliver-*).
The standalone app has none of it: the last tests (they also run once with the fault `leak`, which must make the check fail)."""
import pytest

from test_host_page import FOUR, host_open
from test_host_song_time import TRACKS, rows_payload

S_READY = {"state": "ready", "n": 1, "why": ""}
S_PREP = {"state": "preparing", "n": 0, "why": ""}
S_OFF = {"state": "off", "n": 0, "why": "press to prepare"}
S_NONE = {"state": "none", "n": 0, "why": "no hits"}


def err(why):
    return {"state": "error", "n": 0, "why": why}


def payload(gen=1, midi=S_READY, spike=S_OFF, audio=S_OFF, merged=S_READY, used=None):
    ids = [("q1", "Drums"), ("q2", "Toms"), ("s3", FOUR[3][0])]
    srcs = {"q1": [FOUR[0][0], FOUR[1][0]], "q2": [FOUR[2][0]], "s3": [FOUR[3][0]]}
    return {"gen": gen, "rows": [{"id": i, "name": n, "srcs": srcs[i], "midi": midi, "spike": spike, "audio": audio} for i, n in ids],
            "merged": merged, "used": used or {"spike": False, "audio": False}, "hint": "Drop at the start of the session"}


@pytest.fixture
def host(host_app, make_project):
    host_open(host_app, make_project(tracks=TRACKS, seconds=6.0))
    host_app.evaluate("() => { setTrackHeight('large'); }")
    host_app.evaluate("p => { HT.take(p); }", rows_payload())
    host_app.evaluate("""() => {
      window.__calls = []; DLV.fault = '';
      B.deliverTake = async items => { window.__calls.push(['take', items]); return { files: items.map((it, i) => '/x/' + it.row + '_' + it.kind + i) }; };
      B.deliverWant = async kind => { window.__calls.push(['want', kind]); return null; };
      DLV.init(); }""")
    yield host_app
    host_app.evaluate("() => { DLV.st = null; DLV.paint(); HT.rows = []; HT.gen = 0; HT.rebuild(); HT.paint(); }")


def give(page, p):
    page.evaluate("p => DLV.take(p)", p)
    page.wait_for_function("() => !FR.raf && FR.flags === 0")


def handles(page, where):
    return page.evaluate("""w => [...document.querySelectorAll(w + ' .dlvH')].map(h => ({kind: h.dataset.kind, state: h.dataset.state,
        text: h.querySelector('.dlvT').textContent, title: h.title}))""", where)


def press_move(page, selector, move=12):
    box = page.locator(selector).first.bounding_box()
    x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    page.mouse.move(x, y); page.mouse.down(); page.mouse.move(x + move, y + 2, steps=3); page.mouse.up()


def test_every_row_header_has_three_handles_and_the_toolbar_four(host):
    give(host, payload())
    home = host.evaluate("() => [...document.querySelectorAll('#lanesScroll .track')].filter(r => !r.classList.contains('hostMerged') && r.querySelector('.tdel')).length")
    assert home == 3, "one strip per DAW track (the merged DOM row of the second source has none)"
    row = handles(host, "#lanesScroll .track[data-name='%s'] .tdel" % FOUR[0][0])
    assert [h["kind"] for h in row] == ["midi", "spike", "audio"]
    assert [h["state"] for h in row] == ["ready", "off", "off"]
    assert [h["text"] for h in row] == ["ready 1", "press to prepare", "press to prepare"]
    bar = handles(host, "#dlvBar")
    assert [h["kind"] for h in bar] == ["midi", "spike", "all", "merged"]
    assert [h["state"] for h in bar] == ["ready", "off", "off", "ready"]
    assert all("Drop at the start of the session" in h["title"] for h in row + bar), "the hint is in every tooltip"
    assert host.inner_text("#dlvHint") == "Drop at the start of the session"
    assert "current tempo map" in row[0]["title"] and "current tempo map" in bar[0]["title"]


def test_a_state_is_one_of_ready_preparing_error_and_press_to_prepare(host):
    give(host, payload(midi=err("sources of one DAW track have different sample rates"), spike=S_PREP))
    row = handles(host, "#lanesScroll .track[data-name='%s'] .tdel" % FOUR[0][0])
    assert row[0]["state"] == "error" and row[0]["text"].startswith("error: ")
    assert row[1]["state"] == "preparing" and row[1]["text"] == "preparing…"
    bar = handles(host, "#dlvBar")
    assert bar[0]["state"] == "error", "one row in error makes the toolbar handle an error"
    assert bar[1]["state"] == "preparing"
    give(host, payload(gen=2, midi=S_NONE, spike=S_NONE, audio=S_NONE, merged=S_NONE))
    assert handles(host, "#dlvBar")[0]["state"] == "none"


def test_only_a_ready_handle_drags_and_the_drag_asks_for_its_items(host):
    give(host, payload(spike=S_PREP))
    sel = "#lanesScroll .track[data-name='%s'] .dlvH[data-kind='midi']" % FOUR[0][0]
    press_move(host, sel)
    host.wait_for_function("() => window.__calls.length > 0")
    assert host.evaluate("() => window.__calls[0]") == ["take", [{"row": "q1", "kind": "midi"}]]
    assert host.evaluate("() => (window.__DS_DRAG__ || []).length") == 1
    host.evaluate("() => { window.__calls.length = 0; window.__DS_DRAG__ = []; }")
    press_move(host, "#lanesScroll .track[data-name='%s'] .dlvH[data-kind='spike']" % FOUR[0][0])   # preparing: no drag
    press_move(host, "#dlvBar .dlvH[data-kind='spike']")
    host.wait_for_timeout(200)
    assert host.evaluate("() => window.__calls") == []


def test_a_press_on_press_to_prepare_starts_the_preparation_and_drags_nothing(host):
    give(host, payload())
    box = host.locator("#dlvBar .dlvH[data-kind='spike']").bounding_box()
    host.mouse.click(box["x"] + 5, box["y"] + 5)
    host.wait_for_function("() => window.__calls.length > 0")
    assert host.evaluate("() => window.__calls") == [["want", "spike"]]
    assert host.evaluate("() => (window.__DS_DRAG__ || []).length") == 0


def test_the_toolbar_handles_carry_all_rows_and_drag_all_carries_midi_and_spikes(host):
    give(host, payload(spike=S_READY, used={"spike": True, "audio": False}))
    press_move(host, "#dlvBar .dlvH[data-kind='all']")
    host.wait_for_function("() => window.__calls.length > 0")
    items = host.evaluate("() => window.__calls[0][1]")
    assert sorted((i["row"], i["kind"]) for i in items) == sorted([(r, k) for r in ("q1", "q2", "s3") for k in ("midi", "spike")])
    host.evaluate("() => { window.__calls.length = 0; }")
    press_move(host, "#dlvBar .dlvH[data-kind='merged']")
    host.wait_for_function("() => window.__calls.length > 0")
    assert host.evaluate("() => window.__calls[0]") == ["take", [{"row": "", "kind": "merged"}]]


def test_a_refused_take_shows_the_reason_and_asks_the_plugin_again(host):
    give(host, payload())
    host.evaluate("""() => { window.__asked = 0; B.deliverTake = async () => { throw new Error('Kick In: preparing'); };
      B.deliverState = async () => { window.__asked++; return null; }; }""")
    press_move(host, "#lanesScroll .track[data-name='%s'] .dlvH[data-kind='midi']" % FOUR[0][0])
    host.wait_for_function("() => window.__asked > 0")
    assert "preparing" in host.inner_text("#stMsg")


def test_a_preparing_handle_is_asked_about_again_by_the_page(host):
    host.evaluate("() => { window.__asked = 0; B.deliverState = async () => { window.__asked++; return null; }; }")
    give(host, payload(midi=S_PREP))
    host.wait_for_function("() => window.__asked > 0", timeout=4000)


def test_a_small_row_and_a_narrow_window_collapse_the_three_handles_to_one_and_a_menu(host):
    give(host, payload())
    host.evaluate("() => { setTrackHeight('small'); DLV.paint(); }")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    row = handles(host, "#lanesScroll .track[data-name='%s'] .tdel" % FOUR[0][0])
    assert [h["kind"] for h in row] == ["one"], row
    host.locator("#lanesScroll .track[data-name='%s'] .dlvM" % FOUR[0][0]).click()
    labels = host.evaluate("() => [...document.querySelectorAll('#ctxMenu button')].map(b => b.textContent)")
    assert [l.split(":")[0] for l in labels] == ["MIDI", "Spike", "Audio spike"]
    host.keyboard.press("Escape")
    host.evaluate("() => { setTrackHeight('large'); DLV.paint(); }")
    assert len(handles(host, "#lanesScroll .track[data-name='%s'] .tdel" % FOUR[0][0])) == 3


def test_a_row_the_page_cannot_show_is_named_in_the_hint(host):
    give(host, payload())
    host.evaluate("() => { HT.dropped = 2; DLV.repaint(); }")
    assert "2 DAW tracks have no handle in the header: use the toolbar handles" in host.inner_text("#dlvHint")
    host.evaluate("() => { HT.dropped = 0; }")


def standalone_state(page):
    return page.evaluate("""() => { DLV.take(%s); DLV.paint();
      return { strips: document.querySelectorAll('.tdel').length, bar: !!document.getElementById('dlvBar'), handles: document.querySelectorAll('.dlvH').length,
               focusable: [...document.querySelectorAll('.dlvH, .tdel *, .dlvBar *')].filter(e => e.tabIndex >= 0).length, host: hostMode() }; }""" % __import__("json").dumps(payload()))


def test_the_standalone_page_has_no_handle_and_nothing_focusable(app, make_project):
    app.evaluate("() => { DLV.fault = ''; }")
    s = standalone_state(app)
    assert s == {"strips": 0, "bar": False, "handles": 0, "focusable": 0, "host": False}, s


def test_the_fault_run_leak_makes_the_standalone_check_fail(app):
    """deliver-standalone proves its own reach: with the handles leaking out of the mode the check above would fail."""
    app.evaluate("() => { DLV.fault = 'leak'; DLV.ready = true; }")
    try:
        s = standalone_state(app)
    finally:
        app.evaluate("() => { DLV.fault = ''; DLV.ready = false; DLV.st = null; document.getElementById('dlvBar')?.remove(); }")
    assert s["bar"] is True, "the fault shows the toolbar handles in the standalone page, so the check can fail"
