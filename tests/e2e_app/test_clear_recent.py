"""#441: the Clear Recent control of the start screen empties the stored recent list, the empty
state shows the "no recent projects" text, the control hides while the list is empty and a project
opened afterwards starts a new list. The File > Open Recent menu part is native and runs in the real
.app (self-test scenario shell, step recent_clear): the bridge has no menu bar."""

from app_support import invoke, open_project


def start_screen(page):
    """Close every project of the bridge and show the empty start page."""
    for s in invoke(page, "project_list"):
        invoke(page, "project_close", sid=s["sid"], force=True)
    page.evaluate("async () => { S.sid = null; setDirtyState(false); await initSessions(); await loadProject(); }")
    page.wait_for_function("() => document.querySelector('#esRecentList')")


def test_the_clear_recent_control_empties_the_list_and_shows_the_empty_state(app, make_project):
    a = make_project(tracks=(("Kick In", "kick", 36),), mode="copied")
    b = make_project(tracks=(("Snare Top", "snare", 38),), mode="copied")
    open_project(app, a)
    open_project(app, b)
    start_screen(app)
    app.wait_for_function("() => document.querySelectorAll('#esRecentList .sbRecent').length >= 1")
    names = app.evaluate("() => [...document.querySelectorAll('#esRecentList .sbProjName')].map(e => e.textContent)")
    assert len(names) >= 2, f"the start screen does not list the recent projects: {names}"
    assert app.evaluate("() => !document.querySelector('#esRecentClear').hidden"), "Clear Recent is hidden while the list has entries"
    assert invoke(app, "project_recent"), "the stored list is empty before the clear"
    app.click("#esRecentClear")
    app.wait_for_function(
        "() => document.querySelector('#esRecentList .phint') && "
        "document.querySelector('#esRecentList .phint').textContent === 'no recent projects'")
    assert invoke(app, "project_recent") == [], "the stored list is not empty after the clear"
    assert app.evaluate("() => document.querySelector('#esRecentClear').hidden"), "Clear Recent is visible while the list is empty"
    # a project opened afterwards starts a new list
    open_project(app, a)
    start_screen(app)
    app.wait_for_function("() => document.querySelectorAll('#esRecentList .sbRecent').length === 1")
    only = app.evaluate("() => document.querySelector('#esRecentList .sbProjName').textContent")
    assert only == "Song 1", f"the new list holds more than the one project: {only}"


def test_the_clear_command_answers_the_number_of_entries(app):
    assert "cleared" in invoke(app, "project_recent_clear")
    assert invoke(app, "project_recent") == []
