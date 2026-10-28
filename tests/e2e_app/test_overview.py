"""#169: View > Show Overview (the overview strip over the lanes). The menu item hides and shows it; with it hidden the
lanes take its height (+-2 px); the choice is the app setting `overview` and survives a restart. The native key
Option+O is an accelerator of the menu, so the bridge checks the menu item (the row of the key table has it)."""
from app_support import menu, open_project, wait_until
from test_inspector import js, settings_json
from test_keypreset import reload
from test_menus import with_hits

OVERVIEW_H = 52   # #overviewWrap in style.css


def setup(app, make_project):
    sid = open_project(app, make_project(seconds=4.0))
    with_hits(app, sid, {"Kick In": [0.5, 1.5]})
    app.evaluate("() => { if (!S.overviewOn) setOverview(true); }")   # the app setting of an earlier test may hide it
    wait_until(lambda: js(app, "() => S.overviewOn") is True, msg="the overview is shown at the start")
    return sid


def lanes_h(app):
    return js(app, "() => document.querySelector('#lanesScroll').getBoundingClientRect().height")


def test_the_menu_item_hides_the_overview_and_the_lanes_take_its_height(app, make_project):
    setup(app, make_project)
    shown = lanes_h(app)
    assert js(app, "() => document.querySelector('#overviewWrap').classList.contains('hidden')") is False
    menu(app, "overview")
    wait_until(lambda: js(app, "() => S.overviewOn") is False, msg="the menu item hides the overview")
    assert js(app, "() => document.querySelector('#overviewWrap').classList.contains('hidden')") is True
    wait_until(lambda: lanes_h(app) - shown >= OVERVIEW_H - 2, msg="the lanes take the height")
    assert abs((lanes_h(app) - shown) - OVERVIEW_H) <= 2, (shown, lanes_h(app))   # planted fault: no height change
    # the check mark of the menu follows the page
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.overview") is False, msg="the check mark follows")
    menu(app, "overview")
    wait_until(lambda: js(app, "() => S.overviewOn") is True, msg="the menu item shows it again")
    wait_until(lambda: abs(lanes_h(app) - shown) <= 2, msg="the lanes give the height back")


def test_the_choice_is_kept_across_a_restart(app, make_project):
    setup(app, make_project)
    menu(app, "overview")
    wait_until(lambda: settings_json(app).get("overview") is False, msg="the setting is saved")
    reload(app)
    assert js(app, "() => S.overviewOn") is False
    assert js(app, "() => document.querySelector('#overviewWrap').classList.contains('hidden')") is True
    assert js(app, "() => window.__DS_MENU_STATE__ ? window.__DS_MENU_STATE__.overview : null") is False
    menu(app, "overview")
    wait_until(lambda: settings_json(app).get("overview") is True, msg="the setting is saved again")
