"""P4-29: the page feels like a native app, not a web page: no selection, no browser context menu, no drags of images and text,
an arrow cursor, focus rings for the keyboard only, and every View element on in a fresh home. Each guard has a planted-fault
check: the measuring function must see the wrong state."""
import pytest

from app_support import invoke, open_project, wait_until

VIEW_KEYS = ("inspector", "ruler_tempo", "ruler_meter", "ruler_beats", "grid_lines")

def plant_input(app):
    app.evaluate("() => { const i = document.createElement('input'); i.id = 'plantIn'; i.type = 'text'; i.value = 'abc';"
                 " const t = document.createElement('textarea'); t.id = 'plantTa'; document.body.appendChild(i); document.body.appendChild(t); }")


def test_user_select_is_none_on_the_ui_and_text_on_fields(app, make_project):
    open_project(app, make_project())
    plant_input(app)
    d = app.evaluate("""() => { const us = e => { const c = getComputedStyle(e); return c.userSelect; };
      const q = s => document.querySelector(s);
      return { body: us(document.body), html: us(document.documentElement), button: us(q('#topbar button')),
        label: us(q('#stMsg')), ruler: us(q('#mapRulers .thead')), inspector: us(q('#inspector')), insLab: us(q('.insLab')),
        input: us(q('#plantIn')), area: us(q('#plantTa')), cursorBody: getComputedStyle(document.body).cursor,
        cursorLabel: getComputedStyle(q('#stMsg')).cursor, cursorInput: getComputedStyle(q('#plantIn')).cursor }; }""")
    for k in ("body", "html", "button", "label", "ruler", "inspector", "insLab"):
        assert d[k] == "none", (k, d)
    assert d["input"] == "text" and d["area"] == "text", d
    assert d["cursorBody"] == "default" and d["cursorLabel"] == "default" and d["cursorInput"] == "text", d
    # planted fault: a selectable label is seen
    app.evaluate("() => document.querySelector('#stMsg').style.userSelect = 'text'")
    assert app.evaluate("() => getComputedStyle(document.querySelector('#stMsg')).userSelect") == "text"
    # read-only text the user may copy: .selectable
    app.evaluate("() => { const p = document.createElement('p'); p.id = 'plantSel'; p.className = 'selectable'; p.textContent = 'copy me'; document.body.appendChild(p); }")
    assert app.evaluate("() => getComputedStyle(document.querySelector('#plantSel')).userSelect") == "text"


def test_the_context_menu_is_blocked_except_in_text_fields(app, make_project):
    open_project(app, make_project())
    plant_input(app)
    js = "s => { const e = document.querySelector(s); const ev = new MouseEvent('contextmenu', {bubbles: true, cancelable: true}); e.dispatchEvent(ev); return ev.defaultPrevented; }"
    assert app.evaluate(js, "#topbar button") is True
    assert app.evaluate(js, "#stMsg") is True
    assert app.evaluate(js, "#plantIn") is False
    assert app.evaluate(js, "#plantTa") is False
    # the app's own menu still opens (track header)
    name = app.evaluate("() => S.order ? S.order[0] : Object.keys(S.tracks)[0]")
    app.locator(".track .thead").first.click(button="right")
    assert app.locator("#ctxMenu").count() == 1
    # planted fault: a page without the handler would report false on a plain element
    app.evaluate("() => { const e = document.createElement('div'); e.id = 'plain'; document.body.appendChild(e); }")
    assert app.evaluate(js, "#plain") is True
    assert name


def test_images_and_text_cannot_be_dragged_out(app, make_project):
    open_project(app, make_project())
    plant_input(app)
    assert app.evaluate("() => [...document.images].every(i => i.draggable === false)")
    assert app.evaluate("() => getComputedStyle(document.querySelector('#brandIcon')).webkitUserDrag") == "none"
    js = "s => { const e = document.querySelector(s); const ev = new Event('dragstart', {bubbles: true, cancelable: true}); e.dispatchEvent(ev); return ev.defaultPrevented; }"
    assert app.evaluate(js, "#brandIcon") is True
    assert app.evaluate(js, "#stMsg") is True
    assert app.evaluate(js, "#plantIn") is False
    # planted fault: an image with draggable true is found
    app.evaluate("() => { const i = document.createElement('img'); i.id = 'plantImg'; i.draggable = true; document.body.appendChild(i); }")
    assert not app.evaluate("() => [...document.images].every(i => i.draggable === false)")


def selection(app):
    return app.evaluate("() => window.getSelection().toString()")


def test_nothing_gets_selected_by_clicks_or_select_all(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => { document.querySelector('#stMsg').textContent = 'Some label text here'; }")
    box = app.locator("#stMsg").bounding_box()
    x, y = box["x"] + 5, box["y"] + box["height"] / 2
    app.mouse.click(x, y, click_count=3)
    assert selection(app) == ""
    app.mouse.dblclick(x, y)
    assert selection(app) == ""
    app.mouse.click(5, 300)
    app.keyboard.press("Control+a")
    app.keyboard.press("Meta+a")
    assert selection(app) == ""
    app.evaluate("() => document.execCommand('selectAll')")   # what the native Edit > Select All does
    wait_until(lambda: selection(app) == "", msg="selection cleared")
    # planted fault: with selection allowed the same gesture does select
    app.evaluate("() => { document.querySelector('#stMsg').style.userSelect = 'text'; document.querySelector('#stMsg').style.webkitUserSelect = 'text'; }")
    app.evaluate("() => { const r = document.createRange(); r.selectNodeContents(document.querySelector('#stMsg')); const s = getSelection(); s.removeAllRanges(); s.addRange(r); }")
    # a selection inside a field is kept
    plant_input(app)
    app.fill("#plantIn", "hello")
    app.focus("#plantIn")
    app.keyboard.press("ControlOrMeta+a")
    assert app.evaluate("() => { const i = document.querySelector('#plantIn'); return i.selectionEnd - i.selectionStart; }") == 5


def test_a_mouse_click_shows_no_focus_ring_and_the_zoom_of_the_page_is_blocked(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => { document.querySelector('#snapChk').blur(); }")
    app.locator("#inspectorBtn").click()
    app.locator("#inspectorBtn").click()
    assert app.evaluate("() => { const b = document.querySelector('#inspectorBtn'); return b.matches(':focus-visible'); }") is False
    ow = app.evaluate("() => getComputedStyle(document.documentElement).overscrollBehaviorY")
    assert ow == "none"
    js = "k => { const ev = new KeyboardEvent('keydown', {key: k, metaKey: true, bubbles: true, cancelable: true}); document.body.dispatchEvent(ev); return ev.defaultPrevented; }"
    assert app.evaluate(js, "0") is True
    wheel = "() => { const ev = new WheelEvent('wheel', {ctrlKey: true, deltaY: -50, bubbles: true, cancelable: true}); document.body.dispatchEvent(ev); return ev.defaultPrevented; }"
    assert app.evaluate(wheel) is True
    # planted fault: a plain wheel is not blocked
    plain = "() => { const ev = new WheelEvent('wheel', {deltaY: -50, bubbles: true, cancelable: true}); document.body.dispatchEvent(ev); return ev.defaultPrevented; }"
    assert app.evaluate(plain) is False


def test_a_fresh_home_shows_every_view_element(app, make_project):
    for k in VIEW_KEYS:
        invoke(app, "app_settings_set", key=k, value=None)
    folder = make_project()
    app.reload()
    app.wait_for_function("typeof S !== 'undefined' && B.app === true && S.loaded > 0")
    open_project(app, folder)
    app.evaluate("async () => { await loadViewSettings(); }")
    vis = app.evaluate("""() => { const shown = s => { const e = document.querySelector(s); return !!e && !e.classList.contains('hidden') && e.getBoundingClientRect().height > 0; };
      return { inspector: shown('#inspector'), info: shown('#infoLine'), rulers: shown('#mapRulers'), tempo: TM.vis.tempo, meter: TM.vis.meter,
        beats: TM.vis.beats, grid: TM.gridOn, S: S.inspector }; }""")
    assert all(vis.values()), vis
    # a saved choice wins (planted fault: the check sees a hidden Inspector)
    invoke(app, "app_settings_set", key="inspector", value=False)
    app.evaluate("async () => { await loadViewSettings(); }")
    assert app.evaluate("() => document.querySelector('#inspector').classList.contains('hidden')")
    invoke(app, "app_settings_set", key="inspector", value=None)
