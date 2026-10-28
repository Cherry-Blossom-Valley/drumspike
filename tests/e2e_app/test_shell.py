"""P3-4b: the native shell as far as the page and the service go, through the bridge: the menu handlers
(P3-13a: the event "menu" through the bridge's event log, as the native menu sends it), the quit flow, the
inline project rename, the Export all replace prompt, Recompute Velocities, the native pickers instead of the
folder browser (route map row 12), the keys the native menu owns, and the paths from the Finder. The menu bar
itself, the close button and `open -a` are checked in the real .app by the self-test scenario `shell`.
The P3-13a items (View, Transport, Hits, Track, Window, Settings, Help) are in test_menus.py."""
import hashlib
import os
from pathlib import Path

from playwright.sync_api import expect

from app_support import invoke, menu, open_project, shot_dir, wait_until


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def close_all(page):
    """Close every project of the bridge (earlier tests leave dirty ones), then show the empty page."""
    for s in invoke(page, "project_list"):
        invoke(page, "project_close", sid=s["sid"], force=True)
    page.evaluate("async () => { S.sid = null; setDirtyState(false); await initSessions(); await loadProject(); }")


def with_hits(page, sid, name="Kick In", v=1):
    """Give a track three hits with a velocity that the recompute will change, and reload the lanes."""
    ev = [{"t": t, "v": v, "id": f"e{i}"} for i, t in enumerate((0.5, 1.5, 2.5))]
    invoke(page, "events_set", sid=sid, name=name, events=ev)
    page.evaluate("async () => { await loadProject(); }")
    page.wait_for_function("n => S.events[n] && S.events[n].length === 3", arg=name)
    return ev


# ---------- the Export dialog says how many files it replaces (P4-30; before: a confirm dialog) ----------

def test_the_export_dialog_counts_the_files_it_replaces_and_asks_no_second_question(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    with_hits(app, sid)
    with_hits(app, sid, "Snare Top")                                        # the dialog skips a track without hits (P4-30)
    assert invoke(app, "export_existing", sid=sid)["count"] == 0
    app.click("#exportAllBtn")
    expect(app.locator("#exReplace")).to_have_text("No file will be replaced.")
    app.click("#exGo")                                                      # nothing to replace
    wait_until(lambda: invoke(app, "export_existing", sid=sid)["count"] == 4, msg="the first export")
    # the first export has ended (its message), then the message goes: both exports write the same text, so the second
    # wait must not see the first one's (CI 38681872052, x86-mac)
    expect(app.locator("#stMsg")).to_contain_text("exported 2 MIDI and 2 spike files")
    app.evaluate("() => setMsg('')")
    mid = folder / "Exports" / "Kick In_spike.mid"
    before = sha(mid)
    os.utime(mid, (1, 1))
    app.click("#exCancel")
    app.click("#exportAllBtn")                                              # files exist: the dialog says so
    expect(app.locator("#exReplace")).to_have_text("4 files will be replaced")
    app.click("#exGo")                                                      # no confirm dialog
    expect(app.locator("#stMsg")).to_contain_text("exported 2 MIDI and 2 spike files")
    expect(app.locator("#confirmModal")).to_be_hidden()
    wait_until(lambda: mid.stat().st_mtime > 1, msg="the second export writes the file again")
    assert sha(mid) == before                                               # written again, same bytes


def test_export_from_the_menu_opens_the_dialog(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    with_hits(app, sid)
    with_hits(app, sid, "Snare Top")
    menu(app, "export")
    expect(app.locator("#exportModal")).to_be_visible()
    app.click("#exGo")
    wait_until(lambda: invoke(app, "export_existing", sid=sid)["count"] == 4, msg="the first export")


def test_export_existing_checks_the_arguments(app):
    assert invoke(app, "export_existing")["code"] == 400
    assert invoke(app, "export_existing", sid="nope")["code"] == 404


# ---------- File > Rename Project: a small dialog (P4-28; before: an in-page field on the name) ----------

def test_rename_is_a_dialog_and_shows_no_browser_dialog(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    dialogs = []
    app.on("dialog", lambda d: dialogs.append(d.type))
    old = app.evaluate("() => S.name")
    menu(app, "rename")
    field = app.locator("#renameField")
    expect(field).to_be_focused()
    assert field.input_value() == old
    app.keyboard.press("Escape")                                           # Escape keeps the old name
    expect(app.locator("#renameModal")).to_be_hidden()
    assert app.evaluate("() => S.name") == old
    menu(app, "rename")
    menu(app, "rename")                                                    # a second request keeps the one open dialog
    expect(app.locator("#renameModal")).to_be_visible()
    field.fill("Renamed Dialog")
    app.keyboard.press("Enter")
    wait_until(lambda: invoke(app, "project_get", sid=sid)["name"] == "Renamed Dialog", msg="the rename")
    assert app.evaluate("() => S.name") == "Renamed Dialog"
    expect(app.locator("#dirtyDot")).to_be_visible()
    # the Rename button works; an empty name keeps the old one
    menu(app, "rename")
    field.fill("By Button")
    app.click("#renameOk")
    wait_until(lambda: invoke(app, "project_get", sid=sid)["name"] == "By Button", msg="the rename by button")
    menu(app, "rename")
    field.fill("   ")
    app.keyboard.press("Enter")
    app.evaluate("() => new Promise(r => setTimeout(r, 150))")
    assert invoke(app, "project_get", sid=sid)["name"] == "By Button"
    assert dialogs == []                                                   # no window.prompt, no alert


def test_a_list_refresh_leaves_the_rename_dialog_open(app, make_project):
    """A project-dirty event of any project refreshes the project list (P3-4a); the refresh must not close or
    reset the open rename dialog while the user types (found on x86-mac: the rename was lost)."""
    folder = make_project()
    sid = open_project(app, folder)
    other = invoke(app, "project_open", path=str(make_project()))["sid"]
    menu(app, "rename")
    field = app.locator("#renameField")
    field.fill("Typed While")
    invoke(app, "project_rename", sid=other, name="Other Dirty")              # another project becomes dirty
    app.evaluate("async () => { await initSessions(); }")                     # the refresh that event starts
    expect(field).to_have_value("Typed While")
    expect(field).to_be_focused()
    app.keyboard.press("Enter")
    wait_until(lambda: invoke(app, "project_get", sid=sid)["name"] == "Typed While", msg="the rename")
    assert app.evaluate("() => S.name") == "Typed While"
    assert any(s["name"] == "Other Dirty" for s in app.evaluate("() => B.sessions()"))


# ---------- Recompute Velocities (Edit menu) ----------

def test_recompute_velocities_changes_the_hits_in_one_undo_step(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    with_hits(app, sid)
    want = invoke(app, "velocities_recompute", sid=sid, name="Kick In",
                  events=app.evaluate("() => S.events['Kick In']"))["events"]
    assert [e["v"] for e in want] != [1, 1, 1]
    assert menu(app, "recompute_velocities") is True
    expect(app.locator("#stMsg")).to_contain_text("velocities recomputed for 1 track(s): 3 hit(s) changed")
    assert app.evaluate("() => S.events['Kick In'].map(e => e.v)") == [e["v"] for e in want]
    assert app.evaluate("() => S.events['Kick In'].map(e => e.t)") == [e["t"] for e in want]    # the times stay
    wait_until(lambda: [e["v"] for e in invoke(app, "events_get", sid=sid, name="Kick In")["events"]]
               == [e["v"] for e in want], msg="the service holds the new velocities")
    app.keyboard.press("Control+z")                                        # one step back
    assert app.evaluate("() => S.events['Kick In'].map(e => e.v)") == [1, 1, 1]
    app.keyboard.press("Control+y")
    assert app.evaluate("() => S.events['Kick In'].map(e => e.v)") == [e["v"] for e in want]
    # nothing to change the second time
    menu(app, "recompute_velocities")
    expect(app.locator("#stMsg")).to_contain_text("0 hit(s) changed")


def test_recompute_velocities_without_hits_says_so(app, make_project):
    folder = make_project()
    open_project(app, folder)
    app.evaluate("() => { for (const n of S.order) S.events[n] = []; }")
    menu(app, "recompute_velocities")
    expect(app.locator("#stMsg")).to_contain_text("no hits to recompute")


# ---------- menu handlers ----------

def test_save_and_close_from_the_menu(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    with_hits(app, sid)
    app.evaluate("() => markDirty('Kick In')")
    expect(app.locator("#dirtyDot")).to_be_visible()
    menu(app, "save")
    expect(app.locator("#stMsg")).to_contain_text("project saved")
    assert invoke(app, "project_get", sid=sid)["dirty"] is False
    menu(app, "close")
    wait_until(lambda: sid not in [s["sid"] for s in invoke(app, "project_list")], msg="the close")


def test_close_asks_save_discard_or_cancel_for_a_dirty_project(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    invoke(app, "project_rename", sid=sid, name="Dirty one")
    app.evaluate("async () => { await initSessions(); }")
    menu(app, "close", wait=False)
    expect(app.locator("#confirmModal")).to_be_visible()
    assert app.locator("#confirmActions button").all_text_contents() == ["Save and close", "Discard changes", "Cancel"]
    app.click("#confirmModal button[data-act='cancel']")
    assert sid in [s["sid"] for s in invoke(app, "project_list")]


def test_consolidate_is_a_working_menu_item_since_p3_4d(app, make_project):
    sid = open_project(app, make_project(mode="referenced"))
    assert menu(app, "consolidate") is True
    expect(app.locator("#stMsg")).to_contain_text("consolidated: Kick In, Snare Top")
    modes = {t["source"]["mode"] for t in invoke(app, "project_get", sid=sid)["tracks"].values()}
    assert modes == {"copied"}
    assert menu(app, "consolidate") is True
    expect(app.locator("#stMsg")).to_contain_text("nothing to consolidate")


def test_new_open_and_recent_from_the_menu(app, make_project, tmp_path):
    folder = make_project()
    target = tmp_path / "from menu" / "Menu Song"
    target.parent.mkdir()
    app.evaluate("p => { window.__DS_SAVE__ = p; setDirtyState(false); }", str(target))
    menu(app, "new")
    wait_until(lambda: app.evaluate("() => S.name") == "Menu Song", msg="New")
    app.evaluate("f => { window.__DS_PICK__ = f; setDirtyState(false); }", str(folder))
    menu(app, "open")
    wait_until(lambda: app.evaluate("() => S.name") == folder.name, msg="Open")
    app.evaluate("() => { setDirtyState(false); }")
    menu(app, "recent", path=str(target))
    wait_until(lambda: app.evaluate("() => S.name") == "Menu Song", msg="Open Recent")
    assert menu(app, "recent") is True                                     # an entry without a path does nothing
    assert app.evaluate("() => S.name") == "Menu Song"
    assert menu(app, "no_such_item") is False


def test_a_menu_item_waits_while_a_dialog_of_the_page_is_open(app, make_project):
    folder = make_project()
    open_project(app, folder)
    app.evaluate("() => { askConfirm({title: 'T', msg: 'm', buttons: [{act: 'ok', label: 'OK'}]}); }")
    assert menu(app, "save") is False
    expect(app.locator("#stMsg")).to_contain_text("answer the open dialog first")
    app.click("#confirmModal button[data-act='ok']")


def test_about_window_shows_the_summary_and_third_party(app):
    menu(app, "about", wait=False)
    expect(app.locator("#aboutTitle")).to_have_text("About Drum Spike")
    expect(app.locator("#aboutVersion")).to_contain_text("Version")
    expect(app.locator("#aboutModal")).to_contain_text("finds the hits in your close-mic drum tracks")
    expect(app.locator("#aboutModal")).not_to_contain_text("ADT" + "OF")   # #294: DSP-only About (no detector credit)
    app.click("#aboutActions button[data-act='third']")
    expect(app.locator("#aboutThird")).to_be_visible()
    app.click("#aboutActions button[data-act='third']")
    expect(app.locator("#aboutThird")).to_be_hidden()
    app.click("#aboutActions button[data-act='ok']")
    expect(app.locator("#aboutModal")).to_be_hidden()
    menu(app, "about", wait=False)
    app.keyboard.press("Escape")                                           # Escape closes it like OK
    expect(app.locator("#aboutModal")).to_be_hidden()


def test_about_shows_the_cbv_logo_licence_and_links_and_opens_them_by_id(app):
    # #18: the logo, the licence line, the two links as short addresses, and every click asks
    # open_link with one fixed id (the page never passes an address).
    menu(app, "about", wait=False)
    expect(app.locator("#aboutModal")).to_be_visible()
    logo = app.locator("#aboutLogo")
    expect(logo).to_be_visible()
    # #18: one screenshot of the dialog for the coordinator's visual check.
    app.locator("#aboutModal").screenshot(path=str(shot_dir("18-about-dialog.png")))
    assert "/brand/cbv-logo-white.png" in logo.get_attribute("src")
    expect(app.locator("#aboutLicence")).to_have_text("Drum Spike is free and open source (Apache-2.0).")
    expect(app.locator("#aboutLinkWebsite")).to_have_text("drumspike.cherry-blossom-valley.com")
    expect(app.locator("#aboutLinkSource")).to_have_text("github.com/Cherry-Blossom-Valley/drumspike")
    app.evaluate("() => { window.__DS_OPEN_LINK__ = []; }")
    for el_id, id in [("aboutLinkWebsite", "website"), ("aboutLinkSource", "source"), ("aboutLogo", "cbv")]:
        app.click(f"#{el_id}")
        wait_until(lambda id=id: id in app.evaluate("() => window.__DS_OPEN_LINK__ || []"), msg=f"open_link {id}")
    app.click("#aboutActions button[data-act='ok']")
    expect(app.locator("#aboutModal")).to_be_hidden()


# ---------- quit: Save, Discard or Cancel for each dirty project ----------

def two_dirty_projects(app, make_project):
    close_all(app)
    names = []
    for _ in range(2):
        folder = make_project()
        sid = invoke(app, "project_open", path=str(folder))["sid"]
        invoke(app, "project_rename", sid=sid, name=f"Dirty {len(names) + 1}")
        names.append(sid)
    app.evaluate("async () => { await initSessions(); await loadProject(); }")
    return names


def test_quit_asks_once_for_each_dirty_project_and_cancel_keeps_the_app(app, make_project):
    sids = two_dirty_projects(app, make_project)
    app.evaluate("() => { window.__quit = DSShell.quitFlow(); }")
    expect(app.locator("#confirmModal")).to_be_visible()
    first = app.locator("#confirmMsg").text_content()
    assert "has unsaved changes" in first and "Save before you quit" in first
    assert app.locator("#confirmActions button").all_text_contents() == ["Save", "Discard", "Cancel"]
    app.click("#confirmModal button[data-act='discard']")                  # first: Discard, ask again
    wait_until(lambda: app.locator("#confirmMsg").text_content() != first or app.locator("#confirmModal").is_visible(),
               msg="the second prompt")
    expect(app.locator("#confirmModal")).to_be_visible()
    second = app.locator("#confirmMsg").text_content()
    assert second != first                                                 # a different project
    app.click("#confirmModal button[data-act='cancel']")                   # second: Cancel
    assert app.evaluate("() => window.__quit") is not None
    assert app.evaluate("async () => await window.__quit") is False
    assert app.evaluate("() => window.__DS_QUIT__ || 0") == 0              # the app was not told to end
    assert {s["sid"] for s in invoke(app, "project_list")} >= set(sids)    # both projects are still open
    expect(app.locator("#stMsg")).to_contain_text("quit cancelled")


def test_quit_saves_and_discards_and_then_ends_the_app(app, make_project):
    sids = two_dirty_projects(app, make_project)
    app.evaluate("() => { window.__quit = DSShell.quitFlow(); }")
    prompts = 0
    for act in ("save", "discard"):
        expect(app.locator("#confirmModal")).to_be_visible()
        prompts += 1
        app.click(f"#confirmModal button[data-act='{act}']")
        app.evaluate("() => new Promise(r => setTimeout(r, 100))")
    assert app.evaluate("async () => await window.__quit") is True
    assert prompts == 2 and app.evaluate("() => window.__DS_QUIT__") == 1
    states = {s["sid"]: s for s in invoke(app, "project_list")}
    saved = [s for s in states.values() if s["dirty"] is False]
    assert len(saved) >= 1                                                 # the saved one is clean
    assert sum(1 for s in sids if states[s]["dirty"]) == 1                 # the discarded one stays unsaved on disk


def test_quit_without_dirty_projects_ends_at_once(app, make_project):
    close_all(app)
    folder = make_project()
    invoke(app, "project_open", path=str(folder))
    app.evaluate("async () => { await initSessions(); await loadProject(); }")
    before = app.evaluate("() => window.__DS_QUIT__ || 0")
    assert app.evaluate("async () => await DSShell.quitFlow()") is True
    assert app.evaluate("() => window.__DS_QUIT__") == before + 1
    expect(app.locator("#confirmModal")).to_be_hidden()


def test_a_second_quit_request_while_the_flow_runs_is_ignored(app, make_project):
    two_dirty_projects(app, make_project)
    app.evaluate("() => { window.__q1 = DSShell.quitFlow(); }")
    expect(app.locator("#confirmModal")).to_be_visible()
    assert app.evaluate("async () => await DSShell.quitFlow()") is False   # the window close button again
    app.click("#confirmModal button[data-act='cancel']")
    assert app.evaluate("async () => await window.__q1") is False


# ---------- keys that the native menu owns ----------

def test_native_menu_keys_run_once_the_menu_owns_them(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    with_hits(app, sid)
    app.evaluate("() => markDirty('Kick In')")
    n = app.invokes.count("project_save")
    app.keyboard.press("Control+s")                                        # the web page: app.js saves
    app.wait_for_timeout(500)                                              # (the request events arrive while Playwright runs)
    assert app.invokes.count("project_save") == n + 1
    app.evaluate("() => { window.__DS_NATIVE_MENU__ = true; markDirty('Kick In'); }")
    app.keyboard.press("Meta+s")                                           # the menu owns Cmd-S: app.js stays out
    app.wait_for_timeout(300)
    assert app.invokes.count("project_save") == n + 1
    app.keyboard.press("Control+s")                                        # Ctrl is not the menu's key on a Mac
    app.wait_for_timeout(500)
    assert app.invokes.count("project_save") == n + 2
    assert app.evaluate("() => DSShell.menuOwnsKey(new KeyboardEvent('keydown', {key: 'y', metaKey: true}))") is False
    assert app.evaluate("() => DSShell.menuOwnsKey(new KeyboardEvent('keydown', {key: 'Z', metaKey: true, shiftKey: true}))") is True
    assert app.evaluate("() => DSShell.menuOwnsKey(new KeyboardEvent('keydown', {key: 'n', metaKey: true}))") is True
    assert app.evaluate("() => DSShell.menuOwnsKey(new KeyboardEvent('keydown', {key: 'n'}))") is False
    # P3-13a: the accelerators with Control or Option; the keys without one stay page keys
    owns = lambda o: app.evaluate("o => DSShell.menuOwnsKey(new KeyboardEvent('keydown', o))", o)
    assert owns({"key": "Tab", "code": "Tab", "ctrlKey": True}) is True               # Next Hit on All Tracks
    assert owns({"key": "Tab", "code": "Tab", "ctrlKey": True, "altKey": True}) is True
    assert owns({"key": "Tab", "code": "Tab", "altKey": True}) is True                # Previous Hit
    assert owns({"key": "Tab", "code": "Tab"}) is False                               # Next Hit: a page key
    assert owns({"key": "Enter", "code": "Enter", "altKey": True}) is True            # Go to Project End
    assert owns({"key": "Enter", "code": "Enter"}) is False                           # Go to Project Start: a page key
    assert owns({"key": "Enter", "code": "Enter", "shiftKey": True}) is False         # Go to Playback Start: a page key
    assert owns({"key": "1", "code": "Digit1", "ctrlKey": True}) is True              # Zoom Presets > Full
    assert owns({"key": "1", "code": "Digit1"}) is False
    assert owns({"key": "]", "code": "BracketRight", "metaKey": True}) is True        # Zoom In
    assert owns({"key": "[", "code": "BracketLeft", "metaKey": True, "ctrlKey": True}) is True
    assert owns({"key": ",", "metaKey": True}) is True                                # Settings
    assert owns({"key": "I", "code": "KeyI", "metaKey": True, "shiftKey": True}) is True
    for k in ("c", "j", "f", " ", "Delete"):
        assert owns({"key": k}) is False, k
    # the page stays out of a key that the menu owns: Control+3 runs once (the menu), not in the page too
    app.evaluate("() => setView(0, 10)")
    app.keyboard.press("Control+3")
    assert app.evaluate("() => S.view.t1 - S.view.t0") == 10
    app.evaluate("() => { delete window.__DS_NATIVE_MENU__; }")
    app.keyboard.press("Control+3")                                       # the web page: the page's alias of 3 (5 s)
    app.wait_for_function("() => Math.abs((S.view.t1 - S.view.t0) - 5) < 1e-6")
    assert app.evaluate("() => DSShell.menuOwnsKey(new KeyboardEvent('keydown', {key: 's', metaKey: true}))") is False


def test_undo_and_redo_in_a_text_field_go_to_the_field(app, make_project):
    folder = make_project()
    open_project(app, folder)
    app.evaluate("() => { document.body.insertAdjacentHTML('beforeend', '<input id=tmpField>'); document.querySelector('#tmpField').focus(); }")
    app.evaluate("() => { window.__called = 0; const u = window.undo; window.undo = () => { window.__called++; }; }")
    menu(app, "undo")
    assert app.evaluate("() => window.__called") == 0                      # the page's undo stays out of a text field
    app.evaluate("() => { document.querySelector('#tmpField').blur(); }")
    menu(app, "undo")
    assert app.evaluate("() => window.__called") == 1


# ---------- native pickers replace the folder browser (route map row 12) ----------

def test_the_app_never_shows_the_folder_browser(app, tmp_path):
    f = tmp_path / "Kick.wav"
    f.write_bytes(b"x")
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(f))
    got = app.evaluate("() => pickFolder({title: 'Relink Kick', file: true, start: '/x'})")
    assert got == {"dir": str(tmp_path), "file": "Kick.wav"}
    opts = app.evaluate("() => window.__dialogOpts")
    assert opts["directory"] is False and opts["title"] == "Relink Kick" and opts["defaultPath"] == "/x"
    assert opts["filters"][0]["extensions"][:2] == ["wav", "wave"]
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(tmp_path))
    assert app.evaluate("() => pickFolder({title: 'Pick a folder', start: '/y'})") == {"dir": str(tmp_path)}
    assert app.evaluate("() => window.__dialogOpts.directory") is True
    app.evaluate("p => { window.__DS_SAVE__ = p; }", str(tmp_path / "Name"))
    got = app.evaluate("() => pickFolder({title: 'Name it', name: 'Song'})")
    assert got == {"dir": str(tmp_path), "name": "Name", "copy": False}
    assert app.evaluate("() => window.__saveOpts.defaultPath") == "Song"
    app.evaluate("() => { window.__DS_PICK__ = null; window.__DS_SAVE__ = null; }")
    assert app.evaluate("() => pickFolder({file: true})") is None
    assert app.evaluate("() => pickFolder({name: true})") is None
    assert app.evaluate("() => pickFolder({})") is None
    expect(app.locator("#dirModal")).to_have_count(0)   # P3-12: the page has no folder browser


def test_relink_uses_the_native_file_dialog(app, make_project, tmp_path):
    folder = make_project()
    open_project(app, folder)
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(tmp_path / "Kick In.wav"))
    app.evaluate("() => { relinkTrack('Kick In'); }")
    wait_until(lambda: app.evaluate("() => window.__dialogOpts && window.__dialogOpts.directory === false"),
               msg="the native file dialog")
    expect(app.locator("#dirModal")).to_have_count(0)   # P3-12: the page has no folder browser


# ---------- paths from the Finder, the Dock and drops ----------

def test_paths_from_the_finder_open_the_project_and_report_the_rest(app, make_project):
    folder = make_project()
    other = make_project()
    sid = open_project(app, folder)
    app.evaluate("() => setDirtyState(false)")
    app.evaluate("p => { B.takeOpenPaths = async () => ({projects: [p], others: ['/x/a.wav', '/x/b.wav']}); }", str(other))
    got = app.evaluate("async () => await DSShell.takeOpenPaths()")
    assert got["projects"] == [str(other)] and got["others"] == ["/x/a.wav", "/x/b.wav"]
    wait_until(lambda: app.evaluate("() => S.sid") != sid, msg="the project from the Finder")
    assert app.evaluate("async () => (await B.projectGet(S.sid)).folder") in (str(other), os.path.realpath(other))
    # P3-4c: the rest goes to Add Tracks; two files that do not exist give the list with the reasons
    expect(app.locator("#tracksTitle")).to_have_text("No track can be added")
    assert app.locator("#tracksRejected .rejrow").all_text_contents() == [
        "not added: a.wav - the file does not exist", "not added: b.wav - the file does not exist"]
    app.click("#tracksCancel")
    app.evaluate("() => { DSShell.hooks.otherPaths = ps => setMsg('hook ' + ps.length); }")
    app.evaluate("async () => { B.takeOpenPaths = async () => ({projects: [], others: ['/y']}); await DSShell.takeOpenPaths(); }")
    expect(app.locator("#stMsg")).to_have_text("hook 1")


def test_the_bridge_has_no_finder_paths_and_no_sync(app):
    assert app.evaluate("async () => await B.takeOpenPaths()") == {"projects": [], "others": []}
    assert app.evaluate("async () => await B.shellSync(true)") == 0
    assert app.evaluate("async () => await DSShell.sync()") in (0, None)


def test_the_window_has_no_in_page_menu_sidebar_or_help_button(app):
    """P3-13a: the native menu bar is the only menu; the page keeps the toolbar of the reference."""
    for sel in ("#menuBtn", "#fileBtn", "#fileMenu", "#helpBtn", "#sidebar", "#sidebarBackdrop", ".menuWrap", ".sbProj"):
        expect(app.locator(sel)).to_have_count(0)
    for sel in ("#dirtyDot", "#addTracksBtn", "#analyzeAllBtn", "#exportAllBtn", ".zBtn", "#zoomSel", "#snapChk",
                "#followBtn", "#clickBtn", "#playBtn", "#stopBtn", "#timeDisplay", "#sampleDisplay"):
        assert app.locator(sel).count() >= 1, sel
