"""P3-4a: the project lifecycle in the page, through the bridge: New (native save dialog), Save, Save As
(referenced), Close, Rename, Reorder, Note, Open Recent, the dirty event, the autosave, and the progress
of the first lane build. The page runs the IPC backend of the app (B.app) over HTTP; the commands are the
Rust ones of crates/app. Route map rows 2, 4, 7, 8, 10, 15, 16, 17, 22, 24 (and 1, 6, 19 through the list)."""
import hashlib
import json
import os
import re
from pathlib import Path

import httpx
from playwright.sync_api import expect

from app_support import invoke, load_project, menu, open_project, resolve_source, wait_until


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def backups(folder):
    return sorted(Path(folder, "Backups").glob("*.drumspike"))


def project_get(page, sid):
    return invoke(page, "project_get", sid=sid)


def test_new_project_from_the_file_menu_uses_the_save_dialog(app, tmp_path):
    target = tmp_path / "pick" / "My New Song"
    target.parent.mkdir()
    app.evaluate("p => { window.__DS_SAVE__ = p; setDirtyState(false); }", str(target))
    menu(app, "new")
    wait_until(lambda: app.evaluate("() => S.name") == "My New Song", msg="the new project")
    assert app.evaluate("() => window.__saveOpts.title") == "New project"
    for d in ("Audio Files", "Analysis", "Exports", "Backups"):
        assert (target / d).is_dir(), d
    assert (target / "My New Song.drumspike").is_file()
    sid = app.evaluate("() => S.sid")
    got = project_get(app, sid)
    assert got["folder"] == os.path.realpath(target) or got["folder"] == str(target)
    assert got["dirty"] is False and got["order"] == []
    assert app.evaluate("() => S.order.length") == 0
    expect(app.locator("#lanesScroll")).to_contain_text("No tracks yet")
    assert app.evaluate("() => document.querySelector('#dirtyDot').classList.contains('hidden')") is True
    assert "project_new" in app.invokes
    # the folder is in Open Recent now
    recent = invoke(app, "project_recent")
    assert recent[0]["name"] == "My New Song" and Path(recent[0]["folder"]).resolve() == target.resolve()
    # the same name again: 409 from the service, shown as a message, the open project stays
    app.evaluate("() => { window.__DS_SAVE__ = window.__DS_SAVE__; }")
    menu(app, "new")
    expect(app.locator("#stMsg")).to_contain_text("new project failed: 409")
    assert app.evaluate("() => S.sid") == sid


def test_new_project_dialog_cancel_changes_nothing(app):
    before = app.evaluate("() => S.sid")
    n = app.invokes.count("project_new")
    app.evaluate("() => { window.__DS_SAVE__ = null; }")
    menu(app, "new")
    app.evaluate("() => new Promise(r => setTimeout(r, 200))")
    assert app.evaluate("() => S.sid") == before and app.invokes.count("project_new") == n


def test_new_project_without_a_name_is_untitled_in_the_default_folder(app):
    a = invoke(app, "project_new")
    b = invoke(app, "project_new", name="   ")
    assert a["name"].startswith("Untitled project") and b["name"].startswith("Untitled project")
    assert a["name"] != b["name"]
    assert Path(a["folder"]).parent == (app.home / "projects").resolve() or Path(a["folder"]).parent == app.home / "projects"
    assert Path(a["document"]).is_file()
    assert invoke(app, "project_close", sid=a["sid"])["closed"] == a["sid"]       # a clean project closes
    assert invoke(app, "project_close", sid=b["sid"], force=True)["closed"] == b["sid"]


def test_edit_marks_dirty_and_cmd_s_saves_with_a_backup(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    sid = open_project(app, folder)
    doc = Path(folder, f"{Path(folder).name}.drumspike")
    n0 = app.evaluate("() => S.events['Kick In'].length")
    app.locator(".track[data-name='Kick In'] .spike").dblclick(position={"x": 30, "y": 10})
    wait_until(lambda: project_get(app, sid)["dirty"], msg="dirty in the service")
    assert app.evaluate("() => document.querySelector('#dirtyDot').classList.contains('hidden')") is False
    assert "●" in app.evaluate("() => document.title")
    n_backups = len(backups(folder))
    app.keyboard.press("Control+s")
    expect(app.locator("#stMsg")).to_contain_text("project saved")
    assert project_get(app, sid)["dirty"] is False
    assert app.evaluate("() => document.querySelector('#dirtyDot').classList.contains('hidden')") is True
    saved = json.loads(doc.read_text())
    assert len(saved["tracks"]["Kick In"]["events"]) == n0 + 1
    assert len(backups(folder)) >= n_backups + 1 or len(backups(folder)) == 10
    # the server's layer reads the document the app wrote
    p = load_project(folder)
    assert len(p.doc["tracks"]["Kick In"]["events"]) == n0 + 1
    assert "project_save" in app.invokes


def test_the_dirty_event_reaches_the_page_without_a_page_action(app, make_project):
    sid = open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    assert app.evaluate("() => S.dirty") is False
    invoke(app, "track_set_note", sid=sid, name="Kick In", note=40)        # not through the page
    wait_until(lambda: app.evaluate("() => S.dirty"), msg="the project-dirty event")
    invoke(app, "project_save", sid=sid)
    wait_until(lambda: app.evaluate("() => !S.dirty"), msg="the clean event")
    assert app.evaluate("() => document.querySelector('#dirtyDot').classList.contains('hidden')") is True


def test_rename_through_the_file_menu_changes_the_name_only(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    sid = open_project(app, folder)
    menu(app, "rename")                                                    # P3-13a: an in-page field on the name
    app.fill("#renameField", "Renamed Song")
    app.keyboard.press("Enter")
    wait_until(lambda: app.evaluate("() => S.name") == "Renamed Song", msg="rename")
    got = project_get(app, sid)
    assert got["name"] == "Renamed Song" and got["dirty"] is True
    assert Path(got["folder"]).name == Path(folder).name, "the folder keeps its name"
    app.keyboard.press("Control+s")
    expect(app.locator("#stMsg")).to_contain_text("project saved")
    p = load_project(folder)
    assert p.name == "Renamed Song" and Path(p.path).name == f"{Path(folder).name}.drumspike"
    # a bad name is a message, not a change
    bad = app.evaluate("async sid => { try { await B.renameSession(sid, 'a/b'); return 'ok'; } catch (e) { return e.message; } }", sid)
    assert bad.startswith("400: bad project name")


def test_reorder_and_note_reach_the_document(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    app.evaluate("() => applyOrder(['Snare Top', 'Kick In'])")
    wait_until(lambda: project_get(app, sid)["order"] == ["Snare Top", "Kick In"], msg="order in the service")
    app.evaluate("() => { S.tracks['Kick In'].note = 35; return saveMetaNote('Kick In'); }")
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["note"] == 35, msg="note in the service")
    assert project_get(app, sid)["dirty"] is True
    app.keyboard.press("Control+s")
    expect(app.locator("#stMsg")).to_contain_text("project saved")
    p = load_project(folder)
    assert load_project(folder).doc["order"] == ["Snare Top", "Kick In"]
    assert p.doc["tracks"]["Kick In"]["note"] == 35
    # the order survives a reopen in the page
    app.evaluate("async () => { await closeProject(S.sid, S.name); }")
    wait_until(lambda: project_get(app, sid).get("code") == 404, msg="closed")
    sid2 = open_project(app, folder)
    assert app.evaluate("() => S.order") == ["Snare Top", "Kick In"]
    assert sid2
    # a refused call changes nothing
    r = httpx.post(app.url_base + "/invoke/tracks_reorder", json={"sid": sid2, "order": ["Kick In"]})
    assert r.status_code == 400
    r = httpx.post(app.url_base + "/invoke/track_set_note", json={"sid": sid2, "name": "Kick In", "note": 200})
    assert r.status_code == 400


def test_closing_a_dirty_project_asks_and_discard_drops_the_edit(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    sid = open_project(app, folder)
    invoke(app, "track_set_note", sid=sid, name="Kick In", note=44)
    wait_until(lambda: app.evaluate("() => S.dirty"), msg="dirty")
    r = httpx.post(app.url_base + "/invoke/project_close", json={"sid": sid})
    assert r.status_code == 409 and "unsaved changes" in r.json()["message"]
    menu(app, "close", wait=False)
    expect(app.locator("#confirmModal")).not_to_have_class("hidden")
    expect(app.locator("#confirmTitle")).to_have_text("Close project")
    app.click("#confirmActions button[data-act=discard]")
    wait_until(lambda: project_get(app, sid).get("code") == 404, msg="closed")
    assert app.evaluate("() => S.sid") != sid
    assert load_project(folder).doc["tracks"]["Kick In"]["note"] == 36        # the discarded edit never reached the disk
    # the folder is in the recent list: open it again from there
    recent = invoke(app, "project_recent")
    assert any(Path(r["folder"]).resolve() == Path(folder).resolve() for r in recent)


def test_closing_a_clean_project_does_not_ask(app, make_project):
    sid = open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    menu(app, "close")
    wait_until(lambda: project_get(app, sid).get("code") == 404, msg="closed without a prompt")
    expect(app.locator("#confirmModal")).to_have_class(re.compile(r"\bhidden\b"))


def test_save_as_writes_a_referenced_copy_and_switches_to_it_and_copied_audio_is_a_job(app, make_project, tmp_path):
    folder = make_project(mode="copied")
    sid = open_project(app, folder)
    invoke(app, "track_set_note", sid=sid, name="Kick In", note=41)
    target = tmp_path / "saved as" / "Song Copy"
    target.parent.mkdir()
    app.evaluate("p => { window.__DS_SAVE__ = p; }", str(target))
    menu(app, "save_as", wait=False)
    app.click("#confirmModal button[data-act='ref']")   # P3-4d: the app asks whether to copy the audio
    # 30 s: project_save_as took over 5 s on the x86 runner while the shared bridge's 1 s autosave wrote its other
    # dirty projects (Hermes' race hunt, run 8 trace)
    expect(app.locator("#stMsg")).to_contain_text("(audio referenced)", timeout=30000)
    assert app.evaluate("() => S.sid") == sid, "the id stays"
    assert app.evaluate("() => S.name") == "Song Copy"
    got = project_get(app, sid)
    assert Path(got["folder"]).resolve() == target.resolve() and got["dirty"] is False
    p = load_project(target)
    assert p.doc["tracks"]["Kick In"]["note"] == 41, "the unsaved edit is in the copy"
    assert {t["source"]["mode"] for t in p.doc["tracks"].values()} == {"referenced"}
    assert all(resolve_source(t["source"], p.dir)[1] == "ok" for t in p.doc["tracks"].values())
    assert "project_save_as" in app.invokes
    assert load_project(folder).doc["tracks"]["Kick In"]["note"] == 36, "the old folder is unchanged"
    # P3-4d: copied audio is a job; the answer is {job} and the end comes from job_status
    r = httpx.post(app.url_base + "/invoke/project_save_as", json={"sid": sid, "dir": str(tmp_path), "name": "X", "copy_audio": True})
    assert r.status_code == 200 and r.json()["job"]["kind"] == "save_as"
    wait_until(lambda: invoke(app, "job_status", sid=sid)["state"] == "done", msg="the copy job")
    q = load_project(tmp_path / "X")
    assert {t["source"]["mode"] for t in q.doc["tracks"].values()} == {"copied"}
    assert all(resolve_source(t["source"], q.dir)[1] == "ok" for t in q.doc["tracks"].values())


def test_open_recent_lists_ten_projects_and_opens_one(app, make_project):
    folders = [make_project(tracks=(("Kick In", "kick", 36),)) for _ in range(12)]
    for f in folders:
        invoke(app, "project_open", path=str(f))
    recent = invoke(app, "project_recent")
    assert len(recent) == 10
    assert Path(recent[0]["folder"]).resolve() == Path(folders[-1]).resolve()
    assert Path(recent[9]["folder"]).resolve() == Path(folders[2]).resolve()
    open_project(app, folders[0])           # a clean current project, so the open below does not ask
    app.evaluate("() => setDirtyState(false)")
    listed = app.evaluate("() => B.recentProjects()")                     # the list that File > Open Recent shows
    assert len(listed) == 10
    want = Path(folders[-1]).resolve()          # opening folders[0] put it on top; the newest of the rest is second
    assert Path(listed[1]["folder"]).resolve() == want
    menu(app, "recent", path=listed[1]["folder"])                         # an entry of Open Recent
    wait_until(lambda: app.evaluate("async () => S.sid ? (await B.projectGet(S.sid)).folder : ''")
               and Path(app.evaluate("async () => (await B.projectGet(S.sid)).folder")).resolve() == want,
               msg="open from the recent list")


def test_autosave_writes_a_backup_and_leaves_the_document_alone(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    sid = open_project(app, folder)
    doc = Path(folder, f"{Path(folder).name}.drumspike")
    before = sha(doc)
    n0 = len(backups(folder))
    invoke(app, "track_set_note", sid=sid, name="Kick In", note=45)
    # 30 s: each 1 s tick writes every dirty project of the shared test bridge, and on the x86 runner a tick can take
    # seconds (Hermes' race hunt, run 7: no backup within 10 s)
    wait_until(lambda: len(backups(folder)) > n0, timeout=30, msg="an autosave within 30 s (interval 1 s)")
    assert sha(doc) == before, "the main document is unchanged"
    newest = backups(folder)[-1]
    assert json.loads(newest.read_text())["tracks"]["Kick In"]["note"] == 45, "the backup holds the unsaved edit"
    assert project_get(app, sid)["dirty"] is True, "an autosave does not clean the project"
    assert len(backups(folder)) <= 10


def write_outside(doc, note):
    """#121: another writer (a second instance, a sync tool, a restored backup) puts its own document in place."""
    v = json.loads(Path(doc).read_text())
    v["tracks"]["Kick In"]["note"] = note
    v["outside_writer"] = True
    Path(doc).write_text(json.dumps(v, indent=2))
    return sha(doc)


def test_a_file_changed_on_disk_is_not_overwritten_and_the_status_line_offers_reload_or_save_anyway(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    sid = open_project(app, folder)
    doc = Path(folder, f"{Path(folder).name}.drumspike")
    stale = app.locator("#stStale")
    expect(stale).to_have_class(re.compile(r"\bhidden\b"))
    # round 1: the autosave finds the change first (interval 1 s), then Save is refused, then Save anyway
    invoke(app, "track_set_note", sid=sid, name="Kick In", note=40)
    wait_until(lambda: app.evaluate("() => S.dirty"), msg="dirty")
    outside = write_outside(doc, 50)
    # #280: every expect below waits on a bridge round-trip or on the shared bridge's autosave
    # tick, and both stall for seconds on a loaded host (the #249 watch item: #stMsg kept the
    # round-1 text for the whole 5 s default after Ctrl+S, no setMsg ran). 30 s each; the message
    # content is still the assertion, so a wrong message or a save that must not happen still fails.
    expect(stale).not_to_have_class(re.compile(r"\bhidden\b"), timeout=30000)   # project-stale from the autosave
    expect(app.locator("#stMsg")).to_contain_text("the project file changed on disk", timeout=30000)
    assert sha(doc) == outside, "the autosave never writes the document"
    app.keyboard.press("Control+s")
    expect(app.locator("#stMsg")).to_contain_text("save failed: 409", timeout=30000)
    expect(app.locator("#stMsg")).to_contain_text("Reload from disk, or Save anyway", timeout=30000)
    assert sha(doc) == outside, "the refused save leaves the outside content"
    assert project_get(app, sid)["dirty"] is True
    app.click("#stSaveAnyway")
    expect(app.locator("#stMsg")).to_contain_text("project saved over the file on disk", timeout=30000)
    expect(stale).to_have_class(re.compile(r"\bhidden\b"), timeout=30000)
    saved = json.loads(doc.read_text())
    assert saved["tracks"]["Kick In"]["note"] == 40 and "outside_writer" not in saved
    assert project_get(app, sid)["dirty"] is False
    # round 2: Save is refused, then Reload from disk opens the outside document
    invoke(app, "track_set_note", sid=sid, name="Kick In", note=41)
    wait_until(lambda: app.evaluate("() => S.dirty"), msg="dirty again")
    outside = write_outside(doc, 52)
    app.keyboard.press("Control+s")
    expect(app.locator("#stMsg")).to_contain_text("save failed: 409", timeout=30000)
    expect(stale).not_to_have_class(re.compile(r"\bhidden\b"), timeout=30000)
    assert sha(doc) == outside
    app.click("#stReload")
    expect(app.locator("#stMsg")).to_contain_text("reloaded", timeout=30000)
    expect(stale).to_have_class(re.compile(r"\bhidden\b"), timeout=30000)
    assert sha(doc) == outside, "a reload writes nothing"
    new_sid = app.evaluate("() => S.sid")
    assert project_get(app, new_sid)["dirty"] is False
    assert invoke(app, "tracks_get", sid=new_sid)["tracks"]["Kick In"]["note"] == 52
    assert "project_save" in app.invokes


def test_a_failed_backup_cleanup_is_a_warning_on_the_status_line(app, make_project):
    # R12: the save is committed; the failed rotation comes back as a warning, the
    # next save is not a stale-write conflict.
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    sid = open_project(app, folder)
    doc = Path(folder, f"{Path(folder).name}.drumspike")
    bdir = Path(folder, "Backups")
    stuck = bdir / "aaa-stuck.drumspike"
    stuck.mkdir()
    os.utime(stuck, (1_000_000, 1_000_000))
    for i in range(10):
        (bdir / f"old {i} 2000-01-01 00-00-00.drumspike").write_text("{}")
    invoke(app, "track_set_note", sid=sid, name="Kick In", note=40)
    wait_until(lambda: app.evaluate("() => S.dirty"), msg="dirty")
    app.keyboard.press("Control+s")
    expect(app.locator("#stMsg")).to_contain_text("backup cleanup failed")
    expect(app.locator("#stMsg")).to_contain_text("aaa-stuck.drumspike")
    assert project_get(app, sid)["dirty"] is False
    saved = json.loads(doc.read_text())
    assert saved["tracks"]["Kick In"]["note"] == 40, "the committed save is on disk"
    app.keyboard.press("Control+s")
    expect(app.locator("#stMsg")).to_contain_text("backup cleanup failed")
    assert "the project file changed on disk" not in app.evaluate("() => document.querySelector('#stMsg').textContent")
    assert project_get(app, sid)["dirty"] is False
    assert stuck.is_dir()


STALE_CHOICES = ["Save anyway", "Reload from disk", "{} without saving", "Cancel"]


def stale_prompt(app, leave):
    """#127: the save of the quit or close flow found the outside file: the modal names it and offers the choices."""
    expect(app.locator("#confirmTitle")).to_have_text("Project file changed on disk")
    expect(app.locator("#confirmMsg")).to_contain_text("the project file changed on disk")
    assert app.locator("#confirmActions button").all_text_contents() == [c.format(leave) for c in STALE_CHOICES]


def test_quit_and_close_of_a_background_project_changed_on_disk_offer_the_choices(app, make_project):
    for s in invoke(app, "project_list"):   # earlier tests leave dirty projects; the quit flow asks for each
        invoke(app, "project_close", sid=s["sid"], force=True)
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    doc = Path(folder, f"{Path(folder).name}.drumspike")
    open_project(app, make_project(tracks=(("Snare Top", "snare", 38),)))   # the current project stays clean
    quits = lambda: app.evaluate("() => window.__DS_QUIT__ || 0")

    def background_stale(note):
        sid = invoke(app, "project_open", path=str(folder))["sid"]
        app.evaluate("async () => { await initSessions(); }")
        assert app.evaluate("() => S.sid") != sid
        invoke(app, "track_set_note", sid=sid, name="Kick In", note=note)
        assert project_get(app, sid)["dirty"] is True
        return sid, write_outside(doc, note + 10)

    def answer(act):
        app.click(f"#confirmModal button[data-act='{act}']")

    # close: Save and close -> the choices -> Close without saving leaves the outside file
    sid, outside = background_stale(40)
    app.evaluate("n => { window.__c = closeProject(n.sid, n.name); }", {"sid": sid, "name": project_get(app, sid)["name"]})
    expect(app.locator("#confirmTitle")).to_have_text("Close project")
    answer("save")
    stale_prompt(app, "Close")
    assert sha(doc) == outside, "no write before a choice"
    answer("leave")
    assert app.evaluate("async () => await window.__c") is True
    assert project_get(app, sid).get("code") == 404
    assert sha(doc) == outside, "Close without saving leaves the outside file"

    # quit: Save -> the choices -> Cancel stops the quit and writes nothing
    sid, outside = background_stale(41)
    before = quits()
    app.evaluate("() => { window.__q = DSShell.quitFlow(); }")
    answer("save")
    stale_prompt(app, "Quit")
    answer("cancel")
    assert app.evaluate("async () => await window.__q") is False and quits() == before
    expect(app.locator("#stMsg")).to_contain_text("quit cancelled")
    assert sha(doc) == outside and project_get(app, sid)["dirty"] is True

    # quit again: Save anyway writes the open project and the app ends
    app.evaluate("() => { window.__q = DSShell.quitFlow(); }")
    answer("save")
    stale_prompt(app, "Quit")
    answer("force")
    assert app.evaluate("async () => await window.__q") is True and quits() == before + 1
    assert sha(doc) != outside and json.loads(doc.read_text())["tracks"]["Kick In"]["note"] == 41   # not the outside 51
    assert project_get(app, sid)["dirty"] is False

    # quit: Reload from disk stops the quit and opens the outside file in the background
    invoke(app, "project_close", sid=sid, force=True)
    sid, outside = background_stale(42)
    app.evaluate("() => { window.__q = DSShell.quitFlow(); }")
    answer("save")
    stale_prompt(app, "Quit")
    answer("reload")
    assert app.evaluate("async () => await window.__q") is False and quits() == before + 1
    expect(app.locator("#stMsg")).to_contain_text("reloaded")
    assert sha(doc) == outside, "a reload writes nothing"
    assert project_get(app, sid).get("code") == 404
    (new,) = [s for s in invoke(app, "project_list") if Path(s["folder"]).resolve() == Path(folder).resolve()]
    assert new["dirty"] is False and invoke(app, "tracks_get", sid=new["sid"])["tracks"]["Kick In"]["note"] == 52

    # quit: Quit without saving leaves the outside file and the app ends
    invoke(app, "project_close", sid=new["sid"], force=True)
    sid, outside = background_stale(43)
    app.evaluate("() => { window.__q = DSShell.quitFlow(); }")
    answer("save")
    stale_prompt(app, "Quit")
    answer("leave")
    assert app.evaluate("async () => await window.__q") is True and quits() == before + 2
    assert sha(doc) == outside, "Quit without saving leaves the outside file"


def test_first_open_shows_the_lane_build_per_track_and_the_second_does_not(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Tom 1", "tom", 45), ("Hat", "hat", 42)),
                          mode="copied")
    app.evaluate("""() => {
      window.__lp = []; window.__txt = [];
      const o = B.prepareLanes;
      B.prepareLanes = (sid, cb) => o(sid, j => { window.__lp.push(JSON.parse(JSON.stringify(j))); cb(j); });
      const show = window.showProgress;          // every call, not only the last text of a batch of messages
      window.showProgress = (pct, text) => { window.__txt.push(text); show(pct, text); };
    }""")
    open_project(app, folder)
    names = ["Kick In", "Snare Top", "Tom 1", "Hat"]
    lp = app.evaluate("() => window.__lp")
    assert lp[-1]["state"] == "done" and lp[-1]["total"] == 4
    for n in names:
        assert any(j["tracks"].get(n, {}).get("state") == "running" and j["tracks"][n].get("stage") == "building" for j in lp), n
        assert lp[-1]["tracks"][n]["state"] == "done"
    # P3-15d: the tracks build in parallel (up to 6 at once); each track is running before it
    # is done, and the count of finished tracks never goes down from one message to the next
    for n in names:
        run = next(i for i, j in enumerate(lp) if j["tracks"].get(n, {}).get("state") == "running")
        done = next(i for i, j in enumerate(lp) if j["tracks"].get(n, {}).get("state") == "done")
        assert run < done, (n, run, done)
    finished = [sum(t["state"] in ("done", "error") for t in j["tracks"].values()) for j in lp]
    assert finished == sorted(finished), finished
    # the bar counts "n of m" through every step and names a track that is building
    txt = [t for t in app.evaluate("() => window.__txt") if t]
    for k in range(1, 5):
        assert any(t.startswith(f"building waveforms: {k} of 4") for t in txt), (k, txt)
    for t in txt:
        assert re.fullmatch(r"building waveforms: [1-4] of 4( \((Kick In|Snare Top|Tom 1|Hat)\))?", t), t
    assert app.evaluate("() => document.querySelector('#progressWrap').classList.contains('hidden')") is True
    for n in names:
        assert (Path(folder, "Analysis", n, "peaks.bin")).is_file() and (Path(folder, "Analysis", n, "key.json")).is_file()
    assert app.evaluate("() => S.order.every(n => S.peaks[n] && S.peaks[n].levels[256].n > 0)") is True
    assert "peaks_build_start" in app.invokes
    # the second open finds the caches: the job is done at once and the bar never shows
    app.evaluate("() => { window.__lp = []; window.__txt = []; }")
    open_project(app, folder)
    lp = app.evaluate("() => window.__lp")
    assert all(not j["tracks"] for j in lp), lp
    assert [t for t in app.evaluate("() => window.__txt") if t] == []


def test_lane_build_skips_an_offline_track_and_counts_the_rest(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Hat", "hat", 42)), mode="copied")
    next(Path(folder, "Audio Files").glob("Snare Top*.wav")).unlink()
    app.evaluate("""() => { window.__txt = [];
      const show = window.showProgress;
      window.showProgress = (pct, text) => { window.__txt.push(text); show(pct, text); }; }""")
    open_project(app, folder)
    txt = [t for t in app.evaluate("() => window.__txt") if t]
    # P3-15d: the two tracks build in parallel, so either one can be named at a step
    assert any(t.startswith("building waveforms: 1 of 2") for t in txt), txt
    assert any(t.startswith("building waveforms: 2 of 2") for t in txt), txt
    assert all(re.fullmatch(r"building waveforms: [12] of 2( \((Kick In|Hat)\))?", t) for t in txt), txt
    assert app.evaluate("() => S.tracks['Snare Top'].offline") is True
    assert app.evaluate("() => S.peaks['Kick In'] !== null && S.peaks['Hat'] !== null") is True


def test_peaks_build_commands_answer_the_job_shape(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36),), mode="copied")
    sid = open_project(app, folder)
    st = invoke(app, "peaks_build_status", sid=sid)
    assert st["state"] == "done" and st["total"] == 0 or st["tracks"]["Kick In"]["state"] == "done"
    r = httpx.post(app.url_base + "/invoke/peaks_build_start", json={"sid": sid, "tracks": ["Nope"]})
    assert r.status_code == 400
    r = httpx.post(app.url_base + "/invoke/peaks_build_start", json={"sid": "zzzzzzzz"})
    assert r.status_code == 404


def test_a_late_project_list_does_not_undo_an_open(app, make_project):
    """The dirty event's list refresh (initSessions(true), 100 ms late) can answer after File > Open set S.sid:
    its list is older than the open and must not reset S.sid (the flake 'the page did not open', CI 38664894264)."""
    first = open_project(app, make_project())
    invoke(app, "project_close", sid=first, force=True)       # the page still holds `first`; the service has none
    app.evaluate("""() => { const real = B.sessions;
        B.sessions = async () => { B.sessions = real; const l = await real();   // the list as of now, answered 3 s late
          await new Promise(r => setTimeout(r, 3000)); return l; }; }""")
    app.evaluate("() => { window.__late = initSessions(true).then(() => 'done'); }")
    want = make_project()
    sid = open_project(app, want)                             # opens while the late list is still on its way
    app.evaluate("() => window.__late")                       # the stale list answers now
    assert app.evaluate("() => S.sid") == sid
    app.wait_for_function("() => S.loaded > 0 && S.order.every(n => n in S.peaks)")


def test_a_project_closed_by_another_client_never_switches_the_page_to_another_project(app, make_project):
    """CI 38671324099: an agent command closed the open project; the dirty event's list refresh then picked list[0], a
    dirty project of an earlier step, and the next File > Open stopped at "Unsaved changes" ('the page did not open')."""
    other = open_project(app, make_project())
    invoke(app, "project_rename", sid=other, name="Other dirty")   # stays open and dirty in the service
    # its dirty event and the list refresh it starts must reach the page first, else they land after open_project
    # cleared the flag and File > Open asks "Unsaved changes" (CI 38679096723, x86-mac)
    app.wait_for_function("() => S.dirty === true && S.name === 'Other dirty'")
    first = open_project(app, make_project())
    assert first != other
    invoke(app, "project_close", sid=first, force=True)          # the page still holds `first`
    app.evaluate("() => initSessions(true)")                     # the refresh a dirty event starts
    assert app.evaluate("() => S.sid") is None                   # planted fault: the old code switched to `other`
    assert app.evaluate("() => S.dirty") is False
    assert app.is_hidden("#confirmModal")
    want = make_project()
    sid = open_project(app, want)                                # File > Open goes through without a prompt
    assert app.evaluate("() => S.sid") == sid and sid != other

