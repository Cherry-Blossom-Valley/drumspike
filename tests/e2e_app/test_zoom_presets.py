"""#169: View > Zoom Presets > Add Current, Organize and the own presets. Add Current saves the span of the view under a
name (the app setting zoom_presets); the preset is an entry of the zoom dropdown and of the menu (the first five with
Option+1 to Option+5: the menu item takes the index); Organize renames and removes; the presets survive a restart.
The accelerators are checked by the Rust tests (menu/tests.rs) and the self-test; here the menu item is run."""
from app_support import menu, open_project, wait_until
from test_inspector import js, settings_json
from test_keypreset import reload
from test_menus import with_hits


def saved_presets(app):
    """The zoom_presets of the settings file: [] while the async write is in the air
    (saveZoomOwn is an app_settings_set invoke, app.js:7044; the mixer's settings_key waits
    on the file the same way)."""
    try:
        return settings_json(app).get("zoom_presets") or []
    except (OSError, ValueError):   # not there yet, or a half-written file
        return []


def setup(app, make_project):
    sid = open_project(app, make_project(seconds=12.0))
    with_hits(app, sid, {"Kick In": [0.5, 1.5]})
    app.evaluate("() => { S.zoomOwn = []; renderZoomOwn(); DSShell.sync(); }")   # an earlier test may have saved presets
    app.evaluate("() => applyZoomSpan('5')")
    wait_until(lambda: abs((js(app, "() => S.view.t1 - S.view.t0")) - 5) < 0.01, msg="a 5 s view")
    return sid


def span(app):
    return js(app, "() => S.view.t1 - S.view.t0")


def dialog_open(app):
    return js(app, "() => !document.querySelector('#presetModal').classList.contains('hidden')")


def test_add_current_saves_the_span_as_a_dropdown_entry_and_a_menu_item(app, make_project):
    setup(app, make_project)
    menu(app, "zoom_add")
    wait_until(lambda: dialog_open(app), msg="Add Current opens the dialog")
    assert js(app, "() => document.querySelector('#presetName').value") == "Span 1"
    app.fill("#presetName", "Snare hit")
    app.click("#presetAccept")
    wait_until(lambda: js(app, "() => S.zoomOwn.length") == 1, msg="the preset is saved")
    assert dialog_open(app) is False
    own = js(app, "() => S.zoomOwn[0]")
    assert own["name"] == "Snare hit" and abs(own["span"] - 5) < 0.01, own   # planted fault: the span of the view is not saved
    assert js(app, "() => [...document.querySelectorAll('#zoomSel option[data-own]')].map(o => o.textContent)") == ["Snare hit"]
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.zoomOwn") == ["Snare hit"], msg="the menu has the preset")
    wait_until(lambda: saved_presets(app)[:1] and saved_presets(app)[0]["name"] == "Snare hit"
               and abs(saved_presets(app)[0]["span"] - 5) < 0.01, msg="the setting is saved")


def test_the_menu_item_of_a_preset_zooms_to_its_span_and_the_dropdown_too(app, make_project):
    setup(app, make_project)
    app.evaluate("() => { S.zoomOwn = [{ name: 'Wide', span: 8 }]; saveZoomOwn(); }")   # 8: no built-in preset has it; the project is 12 s (a longer span is clamped)
    app.evaluate("() => applyZoomSpan('0.5')")
    wait_until(lambda: abs(span(app) - 0.5) < 0.001, msg="the view is 500 ms")
    menu(app, "zoom_own", n=0)
    wait_until(lambda: abs(span(app) - 8) < 0.01, msg="the own preset zooms to 8 s")
    app.evaluate("() => applyZoomSpan('5')")
    app.select_option("#zoomSel", "8")   # the dropdown entry of the same preset
    wait_until(lambda: abs(span(app) - 8) < 0.01, msg="the dropdown entry zooms to 8 s")


def test_organize_renames_and_removes_the_presets(app, make_project):
    setup(app, make_project)
    app.evaluate("() => { S.zoomOwn = [{ name: 'A', span: 1 }, { name: 'B', span: 2 }]; saveZoomOwn(); }")
    menu(app, "zoom_organize")
    wait_until(lambda: dialog_open(app), msg="Organize opens the dialog")
    assert app.locator("#presetCancel").is_hidden(), "Organize has no Cancel: the changes are saved at once"
    rows = app.locator("#presetBody .presetRow input")
    assert rows.count() == 2
    rows.nth(0).fill("Kick span")
    rows.nth(0).press("Tab")   # leaves the field: the change is saved
    wait_until(lambda: js(app, "() => S.zoomOwn[0].name") == "Kick span", msg="the rename is saved")
    wait_until(lambda: saved_presets(app)[:1] and saved_presets(app)[0]["name"] == "Kick span",
               msg="the rename is saved to the settings")
    app.locator("#presetBody .presetRow button").nth(1).click()   # remove B
    wait_until(lambda: js(app, "() => S.zoomOwn.map(p => p.name)") == ["Kick span"], msg="B is removed")
    assert app.locator("#presetBody .presetRow").count() == 1
    app.click("#presetAccept")
    wait_until(lambda: dialog_open(app) is False, msg="OK closes the dialog")
    assert js(app, "() => [...document.querySelectorAll('#zoomSel option[data-own]')].map(o => o.textContent)") == ["Kick span"]


def test_the_presets_survive_a_restart(app, make_project):
    setup(app, make_project)
    app.evaluate("() => { S.zoomOwn = [{ name: 'Hat', span: 0.05 }]; saveZoomOwn(); }")
    wait_until(lambda: settings_json(app).get("zoom_presets") == [{"name": "Hat", "span": 0.05}], msg="the setting is saved")
    reload(app)
    assert js(app, "() => S.zoomOwn") == [{"name": "Hat", "span": 0.05}]
    assert js(app, "() => [...document.querySelectorAll('#zoomSel option[data-own]')].map(o => o.textContent)") == ["Hat"]
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.zoomOwn") == ["Hat"], msg="the menu has the preset after a restart")
