"""#317: in the app (native menu on), Option+Tab steps back to the previous hit and does not move the focus.

The web view takes Tab with a modifier as focus traversal before the native menu sees the key equivalent, so the
page runs every Tab key itself (DSKeys.menuOwns) and stops the focus move. The tests before #317 pressed Option+Tab
in a page WITHOUT the native-menu flag, where the page always ran the key; these set the flag the app sets. A menu
event for the same command right after the key is dropped (one press never steps twice); a later one runs."""
from app_support import menu, open_project
from test_menus import with_hits


def playhead(page):
    return page.evaluate("() => S.playhead")


def native(page):
    page.evaluate("() => { window.__DS_NATIVE_MENU__ = true; }")


def focus_a_button(page):
    """The focus on a toolbar button, as after a click: the place where Option+Tab walked the buttons."""
    return page.evaluate("""() => {
      const b = [...document.querySelectorAll('button')].find(x => x.offsetParent !== null);
      b.focus();
      return document.activeElement === b ? (b.id || b.textContent.trim().slice(0, 20) || 'button') : null;
    }""")


def active(page):
    return page.evaluate("() => { const a = document.activeElement; return a ? (a.id || a.tagName + ':' + a.textContent.trim().slice(0, 20)) : null; }")


def test_option_tab_steps_back_in_the_app_and_keeps_the_focus(app, make_project):
    sid = open_project(app, make_project(seconds=6.0))
    with_hits(app, sid, {"Kick In": [1.0, 2.0, 3.0]})
    app.evaluate("() => { S.playhead = 3.5; S.sel = null; S.selTrack = null; setView(0, 6); }")
    native(app)
    assert app.evaluate("() => DSKeys.menuOwns(new KeyboardEvent('keydown', {key: 'Tab', code: 'Tab', altKey: true}))") is False
    assert app.evaluate("() => DSKeys.menuOwns(new KeyboardEvent('keydown', {key: 'Tab', code: 'Tab', ctrlKey: true}))") is False
    assert app.evaluate("() => DSKeys.menuOwns(new KeyboardEvent('keydown', {key: 'z', code: 'KeyZ', metaKey: true}))") is True   # other menu keys stay the menu's

    was = focus_a_button(app)
    assert was, "a visible button takes the focus"
    app.keyboard.press("Alt+Tab")
    assert abs(playhead(app) - 3.0) < 1e-6   # the previous hit
    assert active(app) == was                  # the focus did not walk to the next button
    app.keyboard.press("Alt+Tab")
    assert abs(playhead(app) - 2.0) < 1e-6
    assert active(app) == was


def test_a_menu_event_right_after_the_key_is_dropped_and_a_later_one_runs(app, make_project):
    sid = open_project(app, make_project(seconds=6.0))
    with_hits(app, sid, {"Kick In": [1.0, 2.0, 3.0]})
    app.evaluate("() => { S.playhead = 3.5; S.sel = null; S.selTrack = null; setView(0, 6); }")
    native(app)
    app.keyboard.press("Alt+Tab")
    assert abs(playhead(app) - 3.0) < 1e-6
    app.evaluate("() => DSKeys.notePageRun('prev_hit')")   # as if the key ran this instant
    assert menu(app, "prev_hit") is True                     # the dispatcher takes it ...
    assert abs(playhead(app) - 3.0) < 1e-6                   # ... and does not step a second time
    app.wait_for_timeout(200)
    assert menu(app, "prev_hit") is True                     # a real menu click later runs
    assert abs(playhead(app) - 2.0) < 1e-6


def test_control_tab_runs_in_the_page_with_the_native_menu(app, make_project):
    sid = open_project(app, make_project(seconds=6.0))
    with_hits(app, sid, {"Kick In": [1.0, 2.0], "Snare Top": [1.5]})
    app.evaluate("() => { S.playhead = 0.5; S.sel = null; S.selTrack = null; setView(0, 6); }")
    native(app)
    app.keyboard.press("Control+Tab")   # Next Hit on All Tracks
    assert abs(playhead(app) - 1.0) < 1e-6
    app.keyboard.press("Control+Tab")
    assert abs(playhead(app) - 1.5) < 1e-6
