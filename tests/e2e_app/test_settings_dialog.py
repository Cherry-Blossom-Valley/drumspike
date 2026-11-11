"""#387: the Settings dialog is a preferences window: a category list on the left, one category's
rows on the right, sized to the screen (width like the Help dialog, height 78 vh so the right pane
scrolls), the last open category remembered, Esc and Done close. The page in a 1280x800 and a
1920x1080 viewport."""
import pytest
from playwright.sync_api import expect

from app_support import menu, settings_cat
from test_plugins import install_plugin

CATS = ["audio", "keyboard", "view", "audition", "pitches", "plugins", "agent"]
# every pre-existing #set* control id and the category that holds it (the ids the tests, the
# record scenarios and DSAgent read)
CONTROL_IN_CAT = {
    "audio": ["#setOutput", "#setOutputNow", "#setOutputNote"],
    "keyboard": ["#setKeyPreset"],
    "view": ["#setZoomTall", "#setTabMode"],
    "audition": ["#setAuditionTab", "#setClickOn", "#setClickVol"],
    "pitches": ["#setPitches"],
    "plugins": ["#setPlugins"],
    "agent": ["#setAgentSkill", "#setSkillStatus", "#setSkillInstall", "#setSkillCmd", "#setSkillCopy"],
}


def check_dialog_size(app, vw):
    box = app.locator("#settingsModal .modal-box").bounding_box()
    assert box["width"] >= 800, box
    assert box["width"] <= vw - 40, box


def test_the_dialog_is_sized_to_the_screen(app):
    for vw, vh in [(1280, 800), (1920, 1080)]:
        app.set_viewport_size({"width": vw, "height": vh})
        menu(app, "settings")
        expect(app.locator("#settingsModal")).to_be_visible()
        check_dialog_size(app, vw)
        box = app.locator("#settingsModal .modal-box").bounding_box()
        assert box["height"] <= vh, box
        page_h = app.evaluate("() => document.querySelector('#setPage').clientHeight")
        box_h = app.evaluate("() => document.querySelector('#settingsModal .modal-box').clientHeight")
        assert page_h < box_h   # the category pane scrolls, not the dialog
        app.click("#settingsDone")
        expect(app.locator("#settingsModal")).to_be_hidden()


def test_each_category_shows_its_rows_and_hides_the_others(app):
    menu(app, "settings")
    for cat in CATS:
        settings_cat(app, cat)
        expect(app.locator(f"#settingsModal .setCat[data-cat={cat}]")).to_be_visible()
        for other in CATS:
            if other != cat:
                expect(app.locator(f"#settingsModal .setCat[data-cat={other}]")).to_be_hidden()
        for sel in CONTROL_IN_CAT[cat]:
            if sel == "#setOutputNote":
                # the warning sits in its fixed slot and is hidden until a warning exists
                assert app.locator(sel).count() == 1
                continue
            expect(app.locator(sel)).to_be_visible()
    app.click("#settingsDone")
    expect(app.locator("#settingsModal")).to_be_hidden()


def test_every_preexisting_control_id_still_exists_and_is_visible_in_its_category(app):
    menu(app, "settings")
    for cat, sels in CONTROL_IN_CAT.items():
        settings_cat(app, cat)
        for sel in sels:
            assert app.locator(sel).count() == 1, sel
            if sel == "#setOutputNote":
                continue   # the warning: hidden until a warning exists, its slot is fixed
            expect(app.locator(sel)).to_be_visible()
    # every label is bound to its control (for -> id), inside the dialog
    unbound = app.evaluate("""() => [...document.querySelectorAll('#settingsModal label[for]')]
        .filter(l => { const c = document.getElementById(l.htmlFor); return !c || !c.closest('#settingsModal'); })
        .map(l => l.htmlFor)""")
    assert unbound == [], unbound
    app.click("#settingsDone")


def test_every_input_and_select_in_the_dialog_has_a_bound_label_or_aria_label(app):
    menu(app, "settings")
    empty = app.evaluate("""() => [...document.querySelectorAll('#settingsModal input, #settingsModal select')]
        .filter(c => c.type !== 'hidden')
        .filter(c => { const l = c.labels && c.labels.length > 0; const a = c.hasAttribute('aria-label') || c.getAttribute('aria-labelledby');
            return !l && !a; })
        .map(c => c.id || c.name || c.type)""")
    assert empty == [], empty
    app.keyboard.press("Escape")


def test_esc_closes_and_the_last_category_is_restored_after_reopen(app):
    menu(app, "settings")
    settings_cat(app, "agent")
    assert app.evaluate("() => document.querySelector('#setCat-agent').classList.contains('active')")
    app.keyboard.press("Escape")
    expect(app.locator("#settingsModal")).to_be_hidden()
    menu(app, "settings")
    expect(app.locator("#settingsModal")).to_be_visible()
    assert app.evaluate("() => document.querySelector('#setCat-agent').classList.contains('active')")
    assert app.evaluate("() => localStorage.getItem('dsg_setcat')") == "agent"
    # another category, then Done, keeps the same rule
    settings_cat(app, "audio")
    app.click("#settingsDone")
    menu(app, "settings")
    assert app.evaluate("() => document.querySelector('#setCat-audio').classList.contains('active')")
    app.keyboard.press("Escape")
    expect(app.locator("#settingsModal")).to_be_hidden()


@pytest.mark.parametrize("vw,vh", [(1280, 800), (1920, 1080)])
def test_the_rows_stay_label_left_control_right_at_both_viewports(app, vw, vh):
    """The row grid holds at both sizes: the control column starts right of the label column."""
    app.set_viewport_size({"width": vw, "height": vh})
    menu(app, "settings")
    settings_cat(app, "audition")
    row = app.evaluate("""() => { const r = document.querySelector('#settingsModal .setCat.active .prow');
        const l = r.querySelector('label').getBoundingClientRect();
        const c = r.querySelector('.pctl').getBoundingClientRect(); return [l.right, c.left]; }""")
    assert row[1] >= row[0], row
    app.keyboard.press("Escape")


# The name of one plug-in row and its geometry: the name text of the label (its first non-blank
# text node), the count of the line boxes it spans (a Range over the text yields one rect per
# line box), the Licence button's width and the right edges of the row and the button.
ROW_GEOMETRY = """() => {
  const row = document.querySelector('#setPlugins .prow');
  if (!row) return null;
  const nameNode = [...row.querySelector('label').childNodes].find(n => n.nodeType === 3 && n.textContent.trim());
  const rng = document.createRange();
  rng.selectNodeContents(nameNode);
  const rowBox = row.getBoundingClientRect();
  const btnBox = row.querySelector('button').getBoundingClientRect();
  return { name: nameNode.textContent.trim(), nameLines: rng.getClientRects().length,
           btnW: btnBox.width, rowRight: rowBox.right, btnRight: btnBox.right };
}"""


@pytest.mark.parametrize("vw,vh", [(1280, 800), (1920, 1080)])
def test_a_plugin_row_reads_as_one_row_at_both_viewports(app, vw, vh):
    """#479: with the example plug-in installed, a Detector plug-ins row is one row at the small
    and the large viewport: the name on one line box, the Licence button at its own width (not
    stretched over the row) and flush with the row's right edge."""
    install_plugin(app.home, {"kick": [0.5]})
    app.set_viewport_size({"width": vw, "height": vh})
    menu(app, "settings")
    settings_cat(app, "plugins")
    row = app.locator("#setPlugins .prow")
    row.wait_for(state="visible", timeout=15000)
    m = app.evaluate(ROW_GEOMETRY)
    assert m is not None, "no plug-in row in #setPlugins"
    assert m["nameLines"] == 1, m
    assert m["btnW"] <= 160, m
    assert abs(m["rowRight"] - m["btnRight"]) < 0.5, m
    app.keyboard.press("Escape")
