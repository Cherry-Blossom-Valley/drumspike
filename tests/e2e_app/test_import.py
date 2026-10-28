"""P3-4e: the importer of old sessions in the page, through the bridge. The page runs the IPC backend
(B.app); the commands are the Rust ones of crates/app (import.rs). Route map rows 3 and 13. P4-30 removed the
zip import and export (owner decision 2026-10-14)."""
import json
from pathlib import Path

from playwright.sync_api import expect

from app_support import invoke, load_project, menu
from helpers import SR, synth_track, write_wav

EVENTS = [{"t": 0.25, "v": 100}, {"t": 0.75, "v": 90, "added": True}]


def make_session(root, name="Song: Take 2"):
    sdir = Path(root) / "sessions" / "ab12cd34"
    td = sdir / "tracks" / "Kick_In"
    td.mkdir(parents=True)
    x, _ = synth_track([0.25, 0.75, 1.25], 2.0)
    write_wav(td / "source.wav", x)
    (td / "events.json").write_text(json.dumps({"sr": SR, "duration": 2.0, "n_samples": 2 * SR, "events": EVENTS}))
    (td / "events.json.prev.json").write_text(json.dumps({"events": [{"t": 0.5, "v": 1}]}))
    meta = {"tracks": {"Kick_In": {"role": "kick", "note": 36, "sr": SR, "n_samples": 2 * SR, "duration": 2.0,
                                   "color": "#e05555", "params": {"min_gap_ms": 20}}},
            "name": name, "created": 1759950000.5, "order": ["Kick_In"]}
    (sdir / "meta.json").write_text(json.dumps(meta))
    return sdir


def call(page, expr, *args):
    """Run an async B call in the page; a rejection comes back as ["error", status, message]."""
    return page.evaluate("""async ([expr, args]) => { try { return await (new Function('B', 'a', 'return ' + expr))(B, args); }
        catch (e) { return ['error', e.status, e.message]; } }""", [expr, list(args)])


def check_folder(folder, sdir):
    p = load_project(folder)
    doc = p.doc
    assert doc["format"] == 1 and doc["created"] == 1759950000.5 and doc["settings"] == {"mix_mode": "ffmpeg"}
    k = doc["tracks"]["Kick_In"]
    assert [e["t"] for e in k["events"]] == [0.25, 0.75] and k["mix_mode"] == "ffmpeg"
    assert k["source"]["mode"] == "copied" and k["source"]["relative"] == "Audio Files/Kick_In.wav"
    assert (Path(folder) / "Audio Files/Kick_In.wav").read_bytes() == (Path(sdir) / "tracks/Kick_In/source.wav").read_bytes()
    assert json.loads((Path(folder) / "Backups/Kick_In.events.prev.json").read_text()) == {"events": [{"t": 0.5, "v": 1}]}


def test_import_of_a_session_folder_through_the_list(app, tmp_path):
    sdir = make_session(tmp_path)
    assert call(app, "B.legacyList()") == []                       # no folder, no list
    rows = call(app, "B.legacyList(a[0])", str(tmp_path / "sessions"))
    assert rows == [{"session_id": "ab12cd34", "name": "Song: Take 2", "tracks": ["Kick_In"], "dir": str(sdir)}]
    assert call(app, "B.legacyList(a[0])", str(tmp_path / "none"))[:2] == ["error", 404]
    sum_ = call(app, "B.importLegacy('ab12cd34')")
    assert sum_["name"] == "Song: Take 2" and sum_["dirty"] is False and sum_["tracks"]["Kick_In"]["status"] == "ok"
    check_folder(sum_["folder"], sdir)
    assert Path(sum_["folder"]).parent == app.home / "projects"
    assert (sdir / "tracks/Kick_In/events.json.prev.json").exists()                     # the session is only read
    assert invoke(app, "project_get", sid=sum_["sid"])["name"] == "Song: Take 2"
    again = call(app, "B.importLegacy(a[0])", str(sdir))                                 # a path works too
    assert Path(again["folder"]).name.endswith("Song- Take 2 2")


def test_the_import_folder_dialog_and_the_removed_zip_methods(app, tmp_path):
    make_session(tmp_path)
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(tmp_path / "sessions"))
    assert call(app, "B.pickImportFolder()") == str(tmp_path / "sessions")
    assert app.evaluate("() => window.__dialogOpts.directory") is True
    app.evaluate("() => { window.__DS_PICK__ = null; }")
    assert call(app, "B.pickImportFolder()") is None
    # P4-30: no zip method is left in the page, and the service refuses zip_path
    assert app.evaluate("() => ['importZip', 'exportZip', 'pickImportZip', 'pickExportZip'].filter(k => k in B)") == []
    assert invoke(app, "project_import", zip_path="/x.zip")["code"] == 400


# ---------- the menu item Import Old Session (P3-4b item, P3-4e commands) ----------

def msg(page):
    return page.locator("#stMsg")


def no_status_code(page):
    return page.evaluate(r"() => !/failed: \d{3}:/.test(document.querySelector('#stMsg').textContent)")


def test_menu_import_old_session_picks_the_folder_lists_it_and_imports(app, tmp_path):
    sdir = make_session(tmp_path)
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(tmp_path / "sessions"))
    assert menu(app, "import_old") is True
    assert app.evaluate("() => window.__dialogOpts.directory") is True
    expect(app.locator("#importModal")).to_be_visible()
    expect(app.locator("#importList .imRow .imName")).to_have_text(["Song: Take 2"])
    expect(app.locator("#importList .imRow .imSub")).to_contain_text("ab12cd34 · 1 tracks: Kick_In")
    expect(msg(app)).to_have_text(f"1 old session in {tmp_path / 'sessions'}")
    app.locator("#importList .imRow button").click()
    app.wait_for_function("() => S.name === 'Song: Take 2' && S.loaded > 0")
    expect(app.locator("#importModal")).to_be_hidden()
    expect(msg(app)).to_contain_text("imported Song: Take 2 into ")
    check_folder(invoke(app, "project_get", sid=app.evaluate("() => S.sid"))["folder"], sdir)


def test_menu_import_old_session_says_cancelled_empty_and_clear_errors(app, tmp_path):
    app.evaluate("() => { window.__DS_PICK__ = null; }")
    menu(app, "import_old")
    expect(msg(app)).to_have_text("import cancelled")
    expect(app.locator("#importModal")).to_be_hidden()
    (tmp_path / "empty").mkdir()
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(tmp_path / "empty"))
    menu(app, "import_old")
    expect(app.locator("#importList .dirEmpty")).to_contain_text(f"no old Drum Spike sessions in {tmp_path / 'empty'}")
    expect(msg(app)).to_have_text(f"0 old sessions in {tmp_path / 'empty'}")
    app.click("#importCancel")
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(tmp_path / "gone"))
    menu(app, "import_old")
    expect(msg(app)).to_contain_text("import failed: ")
    assert no_status_code(app), "the status code stays out of the message"
    expect(app.locator("#importModal")).to_be_hidden()


def test_the_zip_menu_items_and_the_import_button_are_gone(app):
    for gone in ('import_zip', 'export_zip'):
        assert menu(app, gone) is False
    expect(app.locator('#importZip')).to_have_count(0)
