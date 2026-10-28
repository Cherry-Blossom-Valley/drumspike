"""P4-28: the toolbar shows no project name (the name is in the window title only; no inline rename), the brand icon is 30 px,
and File > Rename Project is a small dialog (#renameModal, field #renameField). Each guard has a planted-fault check: the
measuring function must fail on a wrong state."""
import pytest
from playwright.sync_api import expect

from app_support import invoke, menu, open_project, wait_until

PROJECT_TEXT = """() => document.querySelector('#topbar').textContent.includes(S.name)"""
ICON = """() => { const i = document.querySelector('#brandIcon'); const r = i.getBoundingClientRect();
  return { w: r.width, h: r.height, ok: i.complete && i.naturalWidth > 0 }; }"""


def icon_ok(page):
    d = page.evaluate(ICON)
    return d["w"] >= 28 and d["h"] >= 28 and d["ok"]


def name_in_topbar(page):
    return page.evaluate(PROJECT_TEXT)


def test_the_toolbar_holds_no_project_name(app, make_project):
    open_project(app, make_project())
    assert app.evaluate("() => S.name") and not name_in_topbar(app)
    assert app.locator("#projName").count() == 0
    # planted fault: the name put into the toolbar is found
    app.evaluate("() => { const s = document.createElement('span'); s.id = 'plant'; s.textContent = S.name; document.querySelector('#topbar .logo').appendChild(s); }")
    assert name_in_topbar(app)
    app.evaluate("() => document.querySelector('#plant').remove()")
    # the name stays in the window title, also after a rename
    menu(app, "rename")
    app.fill("#renameField", "Title Only")
    app.keyboard.press("Enter")
    wait_until(lambda: "Title Only" in app.evaluate("() => document.title"), msg="the title")
    assert not name_in_topbar(app)


def test_the_brand_icon_is_at_least_28_px_and_loaded(app, make_project):
    open_project(app, make_project())
    assert icon_ok(app)
    bar = "() => document.querySelector('#topbar').getBoundingClientRect().height"
    top = app.evaluate(bar)
    app.evaluate("() => { const i = document.querySelector('#brandIcon'); i.style.width = '20px'; i.style.height = '20px'; i.style.margin = '0'; }")
    assert top <= app.evaluate(bar), "the negative margin keeps the top bar from growing (height with the old 20 px icon)"
    app.evaluate("() => { const i = document.querySelector('#brandIcon'); i.style.width = ''; i.style.height = ''; i.style.margin = '20px 0'; }")
    assert app.evaluate(bar) > top, "planted fault: a large margin makes the bar grow"
    app.evaluate("() => { document.querySelector('#brandIcon').style.margin = ''; }")
    # planted faults: a small icon, a broken image
    app.evaluate("() => { const i = document.querySelector('#brandIcon'); i.style.width = '20px'; i.style.height = '20px'; }")
    assert not icon_ok(app)
    app.evaluate("() => { const i = document.querySelector('#brandIcon'); i.style.width = ''; i.style.height = ''; i.src = '/brand/missing.svg'; }")
    app.wait_for_function("() => document.querySelector('#brandIcon').complete")
    assert not icon_ok(app)


def test_the_rename_dialog_renames_and_escape_keeps_the_name(app, make_project):
    sid = open_project(app, make_project())
    old = app.evaluate("() => S.name")
    menu(app, "rename")
    expect(app.locator("#renameModal")).to_be_visible()
    assert app.locator("#renameModal h3").inner_text() == "Rename Project"
    field = app.locator("#renameField")
    expect(field).to_be_focused()
    assert field.input_value() == old and field.get_attribute("maxlength") == "80"
    assert app.evaluate("() => { const f = document.querySelector('#renameField'); return f.selectionStart === 0 && f.selectionEnd === f.value.length; }")
    field.fill("Not Kept")
    app.keyboard.press("Escape")
    expect(app.locator("#renameModal")).to_be_hidden()
    assert app.evaluate("() => S.name") == old and app.evaluate("() => document.title").count("Not Kept") == 0
    assert invoke(app, "project_get", sid=sid)["name"] == old
    menu(app, "rename")
    field.fill("Kept Name")
    app.click("#renameCancel")
    assert invoke(app, "project_get", sid=sid)["name"] == old
    menu(app, "rename")
    field.fill("New Name")
    app.keyboard.press("Enter")
    wait_until(lambda: invoke(app, "project_get", sid=sid)["name"] == "New Name", msg="the service name")
    wait_until(lambda: "New Name" in app.evaluate("() => document.title"), msg="document.title")
    expect(app.locator("#renameModal")).to_be_hidden()
    # planted fault: a wrong expectation on the service name must fail
    assert invoke(app, "project_get", sid=sid)["name"] != old


def test_typing_in_the_rename_field_runs_no_page_key(app, make_project):
    open_project(app, make_project())
    before = app.evaluate("() => [S.view.t0, S.view.t1, S.playing]")
    menu(app, "rename")
    field = app.locator("#renameField")
    field.fill("")
    app.keyboard.type("e e+-")
    app.keyboard.press("Space")
    app.evaluate("() => new Promise(r => setTimeout(r, 150))")
    assert field.input_value() == "e e+- "
    assert app.evaluate("() => [S.view.t0, S.view.t1, S.playing]") == before
    # planted fault: the same keys on the page without the dialog do act (E zooms, Space plays)
    app.keyboard.press("Escape")
    app.locator("#statusbar").click()
    app.keyboard.press("Space")
    wait_until(lambda: app.evaluate("() => S.playing") is True, msg="Space plays without the dialog")
    app.keyboard.press("Space")
