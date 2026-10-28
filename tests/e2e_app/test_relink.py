"""P3-4d: copy jobs (Save As with copied audio, Consolidate), the cancel, and the relink flow, in the page
against the bridge. The commands are the Rust ones of crates/app (copyjob.rs, relink.rs). Route map rows 8, 9, 11.

The bridge command `test_copy_delay {ms}` (debug bridge only) makes every copy job pause after each chunk, so
that a job lasts long enough for the page to act on it; the tests reset it to 0. The page-level cancel clicks the
real `#progressCancel` button from inside a progress callback of the page."""
import hashlib
import shutil
from pathlib import Path

import pytest
from playwright.sync_api import expect

import frozen
from app_support import document_path, file_sha, invoke, load_project, menu, open_project, resolve_source, wait_until
from helpers import synth_track, write_wav

CONSOLIDATE = frozen.store("consolidate")   # project.consolidate of the retired server (P3-12)

TRACKS = tuple((f"T{i:02d}", "kick", 36) for i in range(1, 7))


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


@pytest.fixture
def slow(app):
    """Copy jobs pause 250 ms after each chunk (one chunk per file here) for the test."""
    invoke(app, "test_copy_delay", ms=250)
    yield
    invoke(app, "test_copy_delay", ms=0)


def src_dir(page, sid):
    rec = invoke(page, "project_get", sid=sid)["tracks"]
    return Path(next(iter(rec.values()))["source"]["path"]).parent


def files_with_ext(root, ext):
    return sorted(Path(root).rglob(f"*.{ext}")) if Path(root).exists() else []


def hook_progress(page, cancel_at=None):
    """Record the copy progress messages in window.__prog; with cancel_at the Cancel button is pressed
    when that many files are done."""
    page.evaluate("""cancelAt => {
        const orig = copyProgress; window.__prog = [];
        window.copyProgress = label => { const f = orig(label); return p => {
            window.__prog.push(p); f(p);
            if (cancelAt && p.done >= cancelAt && !window.__cancelled) { window.__cancelled = true;
              document.querySelector('#progressCancel').click(); } }; };
        window.__cancelled = false; window.__origCopyProgress = orig; }""", cancel_at)


def unhook_progress(page):
    page.evaluate("() => { window.copyProgress = window.__origCopyProgress; }")


# ---------- Find moved audio ----------

def distinct_project(make_project, tmp_path, n, mode="referenced"):
    """A project of n tracks T01 ... whose audio differs from track to track (the files of make_project are
    all the same synthetic kick, which a content search could not tell apart)."""
    src = tmp_path / "distinct src"
    src.mkdir()
    specs = []
    for i in range(1, n + 1):
        x, _ = synth_track([0.3 + 0.07 * i, 1.4 + 0.05 * i, 2.5], 4.0)
        specs.append((f"T{i:02d}", write_wav(src / f"T{i:02d}.wav", x), "kick", 36))
    return Path(make_project.builder.build_project(tmp_path / "distinct projects", "Song D", specs, mode))


def empty_page(page):
    """Close every project of the bridge (earlier tests leave dirty ones, and the page would pick one) and
    show the page with no project open."""
    for s in invoke(page, "project_list"):
        invoke(page, "project_close", sid=s["sid"], force=True)
    page.evaluate("async () => { S.sid = null; setDirtyState(false); await initSessions(); await loadProject(); }")
    page.wait_for_function("() => !S.sid && S.loaded > 0")


def moved_project(app, make_project, tmp_path, n=6, mode="referenced"):
    """A project of n referenced tracks with one hit each, opened, then its audio folder moved away and the
    project opened again: every track is offline. Returns (sid, folder, new audio folder)."""
    folder = distinct_project(make_project, tmp_path, n, mode)
    empty_page(app)
    sid = open_project(app, folder)
    for i in range(1, n + 1):
        invoke(app, "events_set", sid=sid, name=f"T{i:02d}", events=[{"t": 0.5 * i, "v": 90, "id": "e0"}])
    invoke(app, "project_save", sid=sid)
    old = src_dir(app, sid)
    empty_page(app)
    new = tmp_path / "elsewhere" / "deep" / "audio moved"
    new.parent.mkdir(parents=True)
    shutil.move(str(old), str(new))
    sid = open_project(app, folder)
    return sid, folder, new


def test_a_project_with_offline_tracks_offers_to_find_moved_audio_and_one_search_relinks_all(app, make_project, tmp_path):
    sid, folder, new = moved_project(app, make_project, tmp_path)
    expect(app.locator("#relinkBar")).to_be_visible()
    expect(app.locator("#relinkText")).to_contain_text("6 of 6 tracks are offline")
    assert app.evaluate("() => S.order.filter(n => S.tracks[n].offline).length") == 6
    events = {f"T{i:02d}": invoke(app, "events_get", sid=sid, name=f"T{i:02d}")["events"] for i in range(1, 7)}
    app.evaluate("f => { window.__DS_PICK__ = f; }", str(new.parent))
    app.click("#relinkFind")
    expect(app.locator("#relinkModal")).to_be_visible()
    opts = app.evaluate("() => window.__dialogOpts")
    assert opts["directory"] is True and "Find moved audio" in opts["title"]
    expect(app.locator("#relinkSummary")).to_contain_text("Found 6 of 6 offline tracks")
    expect(app.locator("#relinkSummary")).to_contain_text("Every offline track is linked again")
    expect(app.locator("#relinkAmbiguous .rlRow")).to_have_count(0)
    expect(app.locator("#relinkMissing .rlRow")).to_have_count(0)
    app.click("#relinkClose")
    expect(app.locator("#relinkModal")).to_be_hidden()
    expect(app.locator("#relinkBar")).to_be_hidden()
    app.wait_for_function("() => S.loaded > 0 && S.order.every(n => S.tracks[n].status === 'ok' && S.peaks[n])")
    assert app.evaluate("() => S.dirty") is True
    for n, ev in events.items():
        assert invoke(app, "events_get", sid=sid, name=n)["events"] == ev, "the events stay unchanged"
    p = invoke(app, "project_get", sid=sid)
    assert all(Path(t["path"]).parent == new for t in p["tracks"].values())
    expect(app.locator("#stMsg")).to_contain_text("found 6 of 6 offline tracks")
    # a save makes it permanent; the file picker flow of one track still works
    invoke(app, "project_save", sid=sid)
    assert all(resolve_source(t["source"], load_project(folder).dir)[1] == "ok"
               for t in load_project(folder).doc["tracks"].values())


def test_ambiguous_and_missing_tracks_are_listed_and_a_candidate_can_be_picked(app, make_project, tmp_path):
    sid, folder, new = moved_project(app, make_project, tmp_path, n=3)
    # T01 exists twice, T02 is deleted (a changed file of that name stays), T03 is where the search looks
    twin = tmp_path / "elsewhere" / "twin"
    twin.mkdir()
    shutil.copy(new / "T01.wav", twin / "T01.wav")
    (new / "T02.wav").unlink()
    (twin / "T02.wav").write_bytes(b"RIFFxxxxWAVE")
    app.evaluate("f => { window.__DS_PICK__ = f; }", str(new.parent.parent))
    app.click("#relinkFind")
    expect(app.locator("#relinkSummary")).to_contain_text("Found 1 of 3 offline tracks")
    expect(app.locator("#relinkSummary")).to_contain_text("1 ambiguous (more than one matching file), 1 still missing.")
    amb = app.locator("#relinkAmbiguous .rlRow")
    expect(amb).to_have_count(1)
    expect(amb.locator(".rlName")).to_contain_text("T01: more than one file matches")
    cands = amb.locator("button.rlPick")
    expect(cands).to_have_count(2)
    expect(app.locator("#relinkMissing .rlRow")).to_have_count(1)
    expect(app.locator("#relinkMissing .rlName")).to_contain_text("T02: not found in this folder.")
    expect(app.locator("#relinkMissing .rlDiff")).to_contain_text("a file with this name but other content")
    st = {n: app.evaluate("n => S.tracks[n].status", n) for n in ("T01", "T02", "T03")}
    assert st == {"T01": "missing", "T02": "missing", "T03": "ok"}, "ambiguous and missing stay offline; none was chosen"
    expect(app.locator("#relinkBar")).to_be_visible()
    expect(app.locator("#relinkText")).to_contain_text("2 of 3 tracks are offline")
    # the user picks a candidate
    path = cands.nth(1).inner_text()
    cands.nth(1).click()
    wait_until(lambda: app.evaluate("() => S.tracks.T01.status") == "ok", msg="T01 relinked")
    assert invoke(app, "project_get", sid=sid)["tracks"]["T01"]["path"] == path
    expect(app.locator("#relinkAmbiguous .rlRow")).to_have_count(0)
    expect(app.locator("#relinkText")).to_contain_text("1 of 3 tracks is offline")
    # a second search of the same folder keeps the missing one missing
    app.click("#relinkAgain")
    expect(app.locator("#relinkSummary")).to_contain_text("Found 0 of 1 offline track")


def test_not_now_hides_the_bar_and_the_search_can_be_cancelled_in_the_dialog(app, make_project, tmp_path):
    sid, folder, new = moved_project(app, make_project, tmp_path, n=2)
    app.click("#relinkDismiss")
    expect(app.locator("#relinkBar")).to_be_hidden()
    app.evaluate("() => { window.__DS_PICK__ = null; }")
    app.evaluate("() => findMovedAudio()")      # the dialog is cancelled: nothing changes
    expect(app.locator("#stMsg")).to_have_text("search cancelled")
    assert app.evaluate("() => S.order.filter(n => S.tracks[n].offline).length") == 2
    app.evaluate("async () => { await loadProject(); }")
    expect(app.locator("#relinkBar")).to_be_hidden()      # hidden for this project
    # a project without offline tracks has no bar
    ok = open_project(app, make_project())
    assert ok != sid
    expect(app.locator("#relinkBar")).to_be_hidden()


def test_relink_search_errors_are_shown_in_the_status_line(app, make_project, tmp_path):
    sid, folder, new = moved_project(app, make_project, tmp_path, n=2)
    app.evaluate("f => { window.__DS_PICK__ = f; }", str(tmp_path / "no such folder"))
    app.click("#relinkFind")
    expect(app.locator("#stMsg")).to_contain_text("search failed: no such folder")
    expect(app.locator("#relinkModal")).to_be_hidden()


def test_relink_of_one_track_through_the_command_keeps_the_force_rule(app, make_project, tmp_path):
    sid, folder, new = moved_project(app, make_project, tmp_path, n=2)
    other = tmp_path / "other.wav"
    shutil.copy(new / "T02.wav", other)
    raw = bytearray(other.read_bytes())
    raw[2000:2016] = b"\x55" * 16                                     # the same size, another head hash
    other.write_bytes(bytes(raw))
    bad = invoke(app, "project_relink", sid=sid, track="T01", path=str(other), force=False)
    assert bad["code"] == 409 and "force=true" in bad["message"]
    ok = invoke(app, "project_relink", sid=sid, track="T01", path=str(new / "T01.wav"), force=False)
    assert ok["matched"] is True and ok["status"] == "ok"
    # the page's own flow: the file picker, then the answer 409 asks before a different file is accepted
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(other))
    app.evaluate("() => { relinkTrack('T02'); }")
    expect(app.locator("#confirmModal")).to_be_visible()
    expect(app.locator("#confirmTitle")).to_have_text("File does not match")
    app.click("#confirmModal button[data-act='cancel']")
    expect(app.locator("#stMsg")).to_have_text("relink cancelled")
    assert app.evaluate("() => S.tracks.T02.status") != "ok"


# ---------- Save As with copied audio ----------

def test_save_as_with_copied_audio_shows_progress_and_the_copy_is_self_contained(app, make_project, tmp_path):
    folder = make_project(tracks=TRACKS, mode="referenced")
    sid = open_project(app, folder)
    sources = sorted(src_dir(app, sid).glob("*.wav"))
    before = [sha(p) for p in sources]
    target = tmp_path / "saved" / "Song Copy"
    target.parent.mkdir()
    hook_progress(app)
    app.evaluate("p => { window.__DS_SAVE__ = p; }", str(target))
    menu(app, "save_as", wait=False)
    expect(app.locator("#confirmModal")).to_be_visible()
    assert app.locator("#confirmActions button").all_text_contents() == [
        "Copy audio into the project", "Keep the audio where it is", "Cancel"]
    app.click("#confirmModal button[data-act='copy']")
    expect(app.locator("#stMsg")).to_contain_text("(audio copied)", timeout=30000)
    prog = app.evaluate("() => window.__prog")
    assert len(prog) >= 10, f"at least 10 progress updates, got {len(prog)}"
    assert prog[-1]["state"] == "done" and prog[-1]["done"] == 6 and prog[-1]["bytes_done"] == prog[-1]["bytes_total"]
    unhook_progress(app)
    assert app.evaluate("() => S.sid") == sid and app.evaluate("() => S.name") == "Song Copy"
    q = load_project(target)
    assert {t["source"]["mode"] for t in q.doc["tracks"].values()} == {"copied"}
    assert len(files_with_ext(target, "tmp")) == 0 and len(files_with_ext(target / "Audio Files", "wav")) == 6
    # the original folder and its audio are renamed: the copy still opens 30 / 30 (here 6 / 6) online
    invoke(app, "project_close", sid=sid, force=True)
    folder.rename(folder.with_name("renamed original"))
    src_dir_old = sources[0].parent
    src_dir_old.rename(src_dir_old.with_name("renamed audio"))
    again = invoke(app, "project_open", path=str(target))
    assert [t["status"] for t in again["tracks"].values()] == ["ok"] * 6
    assert before == [sha(p.parent.with_name("renamed audio") / p.name) for p in sources], "the sources are unchanged"


def test_save_as_keeps_the_audio_referenced_when_asked_and_cancel_in_the_question_changes_nothing(app, make_project, tmp_path):
    folder = make_project(tracks=TRACKS[:2], mode="referenced")
    sid = open_project(app, folder)
    target = tmp_path / "saved2" / "Song Ref"
    target.parent.mkdir()
    app.evaluate("p => { window.__DS_SAVE__ = p; }", str(target))
    menu(app, "save_as", wait=False)
    app.click("#confirmModal button[data-act='cancel']")
    expect(app.locator("#stMsg")).to_have_text("save as cancelled")
    assert not target.exists() and app.evaluate("() => S.name") != "Song Ref"
    menu(app, "save_as", wait=False)
    app.click("#confirmModal button[data-act='ref']")
    expect(app.locator("#stMsg")).to_contain_text("(audio referenced)")
    assert {t["source"]["mode"] for t in load_project(target).doc["tracks"].values()} == {"referenced"}
    assert sid == app.evaluate("() => S.sid")


def test_save_as_copy_can_be_cancelled_with_the_button_and_leaves_nothing(app, make_project, tmp_path, slow):
    folder = make_project(tracks=TRACKS, mode="referenced")
    sid = open_project(app, folder)
    doc_before = sha(document_path(folder))
    target = tmp_path / "saved3" / "Song Cancelled"
    target.parent.mkdir()
    hook_progress(app, cancel_at=2)
    app.evaluate("p => { window.__DS_SAVE__ = p; }", str(target))
    menu(app, "save_as", wait=False)
    app.click("#confirmModal button[data-act='copy']")
    expect(app.locator("#stMsg")).to_contain_text("Save As cancelled: nothing was copied", timeout=30000)
    unhook_progress(app)
    assert app.evaluate("() => window.__prog.at(-1).state") == "cancelled"
    expect(app.locator("#progressWrap")).to_be_hidden()
    assert len(files_with_ext(target.parent, "tmp")) == 0, "0 .tmp files"
    assert not target.exists(), "the folder of the job is removed"
    p = invoke(app, "project_get", sid=sid)
    assert Path(p["folder"]).resolve() == folder.resolve() and p["dirty"] is False
    assert sha(document_path(folder)) == doc_before
    assert app.evaluate("() => S.name") != "Song Cancelled"
    assert invoke(app, "job_status", sid=sid)["state"] == "cancelled"


def test_an_edit_during_the_copy_succeeds_and_is_in_the_copy(app, make_project, tmp_path, slow):
    folder = make_project(tracks=TRACKS, mode="referenced")
    sid = open_project(app, folder)
    target = tmp_path / "saved4"
    target.mkdir()
    r = invoke(app, "project_save_as", sid=sid, dir=str(target), name="Song During", copy_audio=True)
    assert r["job"]["state"] == "running"
    ev = [{"t": 1.25, "v": 77, "id": "x1"}]
    assert wait_until(lambda: invoke(app, "job_status", sid=sid)["done"] >= 1, msg="the copy to start")
    assert invoke(app, "job_status", sid=sid)["state"] == "running"
    assert invoke(app, "events_set", sid=sid, name="T01", events=ev) == {"saved": 1}, "an events_set during the copy"
    assert invoke(app, "project_consolidate", sid=sid)["code"] == 409, "a second job is refused"
    st = wait_until(lambda: (lambda s: s if s["state"] != "running" else None)(invoke(app, "job_status", sid=sid)), timeout=60)
    assert st["state"] == "done" and st["result"]["dirty"] is False
    assert load_project(target / "Song During").doc["tracks"]["T01"]["events"] == ev


# ---------- Consolidate ----------

def test_consolidate_gives_the_records_of_the_servers_consolidate(app, make_project, tmp_path):
    folder = make_project(tracks=TRACKS, mode="referenced")
    sid = open_project(app, folder)
    # the records of the server's Consolidate (project.consolidate) on the same project, frozen (P3-12 step 2)
    inputs = {n: file_sha(Path(tr["source"]["path"])) for n, tr in load_project(folder).doc["tracks"].items()}
    server = CONSOLIDATE.ref("six referenced kicks", inputs, canonical_host=True)   # host independent: arm64-mac only
    hook_progress(app)
    menu(app, "consolidate", wait=False)
    expect(app.locator("#stMsg")).to_contain_text("consolidated: T01, T02, T03, T04, T05, T06 copied into Audio Files/", timeout=30000)
    # the progress channel and the command answer are separate routes: the last record ("done") can arrive after the
    # answer (Hermes' x86 race hunt, run 7)
    app.wait_for_function("() => window.__prog.length && window.__prog[window.__prog.length - 1].state === 'done'")
    prog = app.evaluate("() => window.__prog")
    unhook_progress(app)
    assert len(prog) >= 12 and prog[-1]["state"] == "done"
    invoke(app, "project_save", sid=sid)
    mine = load_project(folder)
    assert server["changed"] == [f"T{i:02d}" for i in range(1, 7)]
    for name, a in server["records"].items():
        b = dict(mine.doc["tracks"][name]["source"])
        b.pop("path"), b.pop("mtime")        # the two projects are in two folders; the copies are new files
        assert a == b, name
        assert b["mode"] == "copied" and b["relative"] == f"Audio Files/{name}.wav" or b["relative"].startswith("Audio Files/")
    assert [t["status"] for t in invoke(app, "project_get", sid=sid)["tracks"].values()] == ["ok"] * 6


def test_consolidate_can_be_cancelled_and_changes_no_record(app, make_project, slow):
    folder = make_project(tracks=TRACKS, mode="referenced")
    sid = open_project(app, folder)
    hook_progress(app, cancel_at=3)
    menu(app, "consolidate", wait=False)
    expect(app.locator("#stMsg")).to_contain_text("Consolidate cancelled", timeout=30000)
    unhook_progress(app)
    assert len(files_with_ext(folder, "tmp")) == 0, "0 .tmp files"
    assert files_with_ext(folder / "Audio Files", "wav") == [], "the copied files are removed"
    p = invoke(app, "project_get", sid=sid)
    assert {t["source"]["mode"] for t in p["tracks"].values()} == {"referenced"} and p["dirty"] is False


def test_consolidate_refuses_an_offline_source_with_the_reason(app, make_project, tmp_path):
    sid, folder, new = moved_project(app, make_project, tmp_path, n=2)
    menu(app, "consolidate", wait=False)
    expect(app.locator("#stMsg")).to_contain_text("consolidate failed: 409: cannot copy source (missing)")
    expect(app.locator("#progressWrap")).to_be_hidden()


# ---------- Add Tracks with the copy: the same cancel ----------

def test_adding_tracks_with_the_copy_can_be_cancelled(app, make_project, tmp_path, slow):
    sid = open_project(app, make_project(tracks=(("Kick In", "kick", 36),), mode="referenced"))
    drop = tmp_path / "drop"
    drop.mkdir()
    for i in range(1, 6):
        shutil.copy(src_dir(app, sid) / "Kick In.wav", drop / f"k in_0{i}.wav")
        (drop / f"k in_0{i}.wav").write_bytes((drop / f"k in_0{i}.wav").read_bytes() + bytes([i]) * 8)   # other bytes each
    app.evaluate("p => { DSShell.hooks.otherPaths([p]); }", str(drop))
    expect(app.locator("#tracksModal")).to_be_visible()
    app.check("#tracksCopy")
    app.evaluate("""() => { window.__t = setInterval(() => { const b = document.querySelector('#progressCancel');
        if (b && !b.classList.contains('hidden') && /copying [2-9]\\//.test(document.querySelector('#progressText').textContent)) { b.click(); } }, 20); }""")
    app.click("#tracksAdd")
    expect(app.locator("#stMsg")).to_contain_text("adding tracks cancelled", timeout=30000)
    app.evaluate("() => clearInterval(window.__t)")
    p = invoke(app, "project_get", sid=sid)
    assert list(p["tracks"]) == ["Kick In"], "no track was added"
    folder = Path(p["folder"])
    assert files_with_ext(folder, "tmp") == []
    assert [f.name for f in files_with_ext(folder / "Audio Files", "wav") if f.name.startswith("k in")] == []
    assert invoke(app, "job_status", sid=sid)["state"] == "cancelled"


def test_job_cancel_with_no_job_running_says_so(app, make_project):
    sid = open_project(app, make_project())
    assert invoke(app, "job_cancel", sid=sid) == {"sid": sid, "cancelling": False}
    assert invoke(app, "job_status", sid=sid)["state"] in ("idle", "done", "cancelled")
