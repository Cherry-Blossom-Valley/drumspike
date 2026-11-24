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


def track_listeners(page):
    """Counts the listeners that the page adds to `document` and has not removed (type, function, capture)."""
    page.evaluate("""() => { if (window.__ls) return; window.__ls = new Map(); window.__synth = 0;
      const add = document.addEventListener.bind(document), rem = document.removeEventListener.bind(document);
      const key = (t, o) => t + '|' + (o === true || (o && o.capture) ? 'c' : 'b');
      document.addEventListener = (t, f, o) => { const m = window.__ls.get(key(t, o)) || new Set(); m.add(f); window.__ls.set(key(t, o), m); return add(t, f, o); };
      document.removeEventListener = (t, f, o) => { const m = window.__ls.get(key(t, o)); if (m) m.delete(f); return rem(t, f, o); };
      add('mousedown', e => { if (!e.isTrusted) window.__synth++; }, true); }""")


def listeners(page):
    return page.evaluate("() => [...window.__ls.values()].reduce((n, s) => n + s.size, 0)")


def open_menu(host, name):
    host.locator("#lanesScroll .track[data-name='%s'] .dlvM" % name).click()
    host.wait_for_selector("#dlvMenu .dlvItem")


def test_a_small_row_and_a_narrow_window_collapse_the_three_handles_to_one_and_a_menu(host):
    give(host, payload())
    host.evaluate("() => { setTrackHeight('small'); DLV.paint(); }")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    row = handles(host, "#lanesScroll .track[data-name='%s'] .tdel" % FOUR[0][0])
    assert [h["kind"] for h in row] == ["one"], row
    open_menu(host, FOUR[0][0])
    labels = host.evaluate("() => [...document.querySelectorAll('#dlvMenu .dlvItem')].map(b => b.textContent)")
    assert [l.split(":")[0] for l in labels] == ["MIDI", "Spike", "Audio spike"]
    host.keyboard.press("Escape")
    assert host.locator("#dlvMenu").count() == 0
    host.evaluate("() => { setTrackHeight('large'); DLV.paint(); }")
    assert len(handles(host, "#lanesScroll .track[data-name='%s'] .tdel" % FOUR[0][0])) == 3


def test_a_ready_menu_item_is_a_drag_source_and_no_listener_is_left_after_the_menu_closes(host):
    give(host, payload())
    host.evaluate("() => { setTrackHeight('small'); DLV.paint(); }")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    track_listeners(host)
    before = listeners(host)
    open_menu(host, FOUR[0][0])
    assert listeners(host) > before, "the open menu has its listeners"
    press_move(host, "#dlvMenu .dlvItem[data-kind='midi']")
    host.wait_for_function("() => window.__calls.length > 0")
    assert host.evaluate("() => window.__calls[0]") == ["take", [{"row": "q1", "kind": "midi"}]], "a press and a move on the item is the drag: no click first"
    assert host.locator("#dlvMenu").count() == 0, "the menu closed with the drag"
    assert listeners(host) == before, "no listener of the handles or the menu is left on the document"
    assert host.evaluate("() => window.__synth") == 0, "no synthetic mouse event anywhere"
    # closed by a click away
    open_menu(host, FOUR[0][0])
    host.mouse.click(5, 300)
    assert host.locator("#dlvMenu").count() == 0
    assert listeners(host) == before
    host.evaluate("() => { setTrackHeight('large'); DLV.paint(); }")


def test_a_mouse_move_after_a_click_on_a_menu_item_starts_no_drag(host):
    give(host, payload())
    host.evaluate("() => { setTrackHeight('small'); DLV.paint(); }")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    open_menu(host, FOUR[0][0])
    box = host.locator("#dlvMenu .dlvItem[data-kind='spike']").bounding_box()
    host.mouse.click(box["x"] + 5, box["y"] + 5)   # press to prepare: the work starts, nothing is dragged
    host.wait_for_function("() => window.__calls.length > 0")
    assert host.evaluate("() => window.__calls") == [["want", "spike"]]
    host.evaluate("() => { window.__calls.length = 0; window.__DS_DRAG__ = []; }")
    host.mouse.move(box["x"] + 200, box["y"] + 100, steps=5)
    host.mouse.move(box["x"] + 10, box["y"] + 10, steps=5)
    host.wait_for_timeout(200)
    assert host.evaluate("() => [window.__calls.length, (window.__DS_DRAG__ || []).length]") == [0, 0]
    host.evaluate("() => { setTrackHeight('large'); DLV.paint(); }")


def test_the_narrow_toolbar_menu_works_the_same_way(host):
    give(host, payload())
    host.set_viewport_size({"width": 480, "height": 700})
    host.evaluate("() => DLV.paint()")
    track_listeners(host)
    before = listeners(host)
    host.locator("#dlvBar .dlvM").click()
    host.wait_for_selector("#dlvMenu .dlvItem")
    labels = host.evaluate("() => [...document.querySelectorAll('#dlvMenu .dlvItem')].map(b => b.dataset.kind)")
    assert labels == ["midi", "spike", "all", "merged"]
    press_move(host, "#dlvMenu .dlvItem[data-kind='merged']")
    host.wait_for_function("() => window.__calls.length > 0")
    assert host.evaluate("() => window.__calls[0]") == ["take", [{"row": "", "kind": "merged"}]]
    assert listeners(host) == before
    host.set_viewport_size({"width": 1280, "height": 720})
    host.evaluate("() => DLV.paint()")


def test_a_handle_is_a_button_with_a_label_and_the_keyboard_prepares_and_retries(host):
    give(host, payload(midi=err("it failed"), spike=S_OFF))
    h = host.locator("#lanesScroll .track[data-name='%s'] .dlvH[data-kind='midi']" % FOUR[0][0])
    assert h.get_attribute("role") == "button" and h.get_attribute("tabindex") == "0"
    assert h.get_attribute("aria-label") == "MIDI, error, press to retry"
    assert "Press to try again" in h.get_attribute("title")
    h.focus(); host.keyboard.press("Enter")   # error: a new try
    host.wait_for_function("() => window.__calls.length > 0")
    assert host.evaluate("() => window.__calls[0]") == ["want", "midi"]
    host.evaluate("() => { window.__calls.length = 0; }")
    s = host.locator("#lanesScroll .track[data-name='%s'] .dlvH[data-kind='spike']" % FOUR[0][0])
    assert s.get_attribute("aria-label") == "Spike, press to prepare"
    s.focus(); host.keyboard.press(" ")
    host.wait_for_function("() => window.__calls.length > 0")
    assert host.evaluate("() => window.__calls[0]") == ["want", "spike"]
    # a press on an error handle (mouse) is a new try as well
    host.evaluate("() => { window.__calls.length = 0; }")
    box = h.bounding_box()
    host.mouse.click(box["x"] + 5, box["y"] + 5)
    host.wait_for_function("() => window.__calls.length > 0")
    assert host.evaluate("() => window.__calls[0]") == ["want", "midi"]


def test_a_press_on_drag_all_with_the_spike_kinds_off_prepares_both(host):
    give(host, payload())
    box = host.locator("#dlvBar .dlvH[data-kind='all']").bounding_box()
    assert "prepares the spike and the audio spike" in host.locator("#dlvBar .dlvH[data-kind='all']").get_attribute("title")
    host.mouse.click(box["x"] + 5, box["y"] + 5)
    host.wait_for_function("() => window.__calls.length >= 2")
    kinds = [c[1] for c in host.evaluate("() => window.__calls")]
    assert "spike" in kinds and "audio" in kinds


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


# ---- #592: the handles of a row header fit the header ----

def strip_boxes(page, name):
    return page.evaluate("""n => {
      const row = document.querySelector(`#lanesScroll .track[data-name="${n}"]`), th = row.querySelector('.thead'), art = th.querySelector('.tart');
      const box = e => { const b = e.getBoundingClientRect(); return { l: b.left, r: b.right, t: b.top, b: b.bottom }; };
      const shown = e => !!e && e.checkVisibility({ visibilityProperty: true });
      return { head: box(th), art: shown(art) ? box(art) : null, collapsed: th.querySelector('.tdel').classList.contains('collapsed'),
        hs: [...th.querySelectorAll('.tdel .dlvH')].map(h => ({ kind: h.dataset.kind, box: box(h), badge: h.querySelector('.dlvB').textContent,
          label: h.querySelector('.dlvL').textContent, icon: shown(h.querySelector('.dlvI svg')), text_shown: shown(h.querySelector('.dlvT')),
          aria: h.getAttribute('aria-label'), title: h.title })) };
    }""", name)


@pytest.mark.parametrize("height", ["medium", "large"])
def test_the_three_handles_fit_the_row_header_and_do_not_lie_on_the_role_drawing(host, height):
    host.evaluate("h => { setTrackHeight(h); }", height)
    give(host, payload())
    host.evaluate("() => DLV.paint()")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    s = strip_boxes(host, FOUR[0][0])
    assert s["art"] is not None, "the role drawing shows at this height (else the test proves nothing)"
    assert not s["collapsed"] and [h["kind"] for h in s["hs"]] == ["midi", "spike", "audio"], s
    for h in s["hs"]:
        b = h["box"]
        assert b["l"] >= s["head"]["l"] and b["r"] <= s["head"]["r"] and b["t"] >= s["head"]["t"] and b["b"] <= s["head"]["b"], (h["kind"], b, s["head"])
        a = s["art"]
        assert b["r"] <= a["l"] or b["l"] >= a["r"] or b["b"] <= a["t"] or b["t"] >= a["b"], f"the handle {h['kind']} lies on the role drawing: {b} {a}"
        assert h["icon"] and not h["text_shown"], h
    assert all(x["box"]["r"] <= y["box"]["l"] for x, y in zip(s["hs"], s["hs"][1:])), "no handle lies on the next one"
    assert [h["label"] for h in s["hs"]] == ["MIDI", "Spike", "Audio"]
    assert [h["badge"] for h in s["hs"]] == ["1", "+", "+"]
    assert s["hs"][2]["aria"] == "Audio spike, press to prepare" and s["hs"][2]["title"].startswith("Audio spike: press to prepare"), "the whole text is in the label and the tooltip"
    give(host, payload(gen=2, midi=S_PREP, spike=err("no room"), audio=S_NONE))
    assert [h["badge"] for h in strip_boxes(host, FOUR[0][0])["hs"]] == ["…", "!", "–"]
    host.evaluate("() => { setTrackHeight('large'); DLV.paint(); }")


def test_handles_that_do_not_fit_fall_back_to_the_one_handle_with_the_menu(host):
    host.evaluate("() => { setTrackHeight('medium'); }")
    wide = {"state": "ready", "n": 10 ** 14, "why": ""}
    give(host, payload(midi=wide, spike=wide, audio=wide))
    host.evaluate("() => DLV.paint()")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    s = strip_boxes(host, FOUR[0][0])
    assert s["collapsed"] and [h["kind"] for h in s["hs"]] == ["one"], s
    b, a = s["hs"][0]["box"], s["art"]
    assert b["r"] <= s["head"]["r"] and (a is None or b["r"] <= a["l"] or b["b"] <= a["t"]), (b, a, s["head"])
    host.evaluate("() => { setTrackHeight('large'); DLV.paint(); }")
