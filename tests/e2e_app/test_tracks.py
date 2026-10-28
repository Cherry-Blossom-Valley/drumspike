"""P3-4c: track import by drop or dialog, the role guess, kit presets. The page runs the IPC backend (B.app)
over the bridge; the commands are the Rust ones of crates/app (tracks.rs, roles.rs). Route map row 23.
A window drop reaches the page through the shell's open-paths queue and DSShell.hooks.otherPaths (an HTML5
drop does not reach the page in the app), so the tests call the hook with the paths a drop would carry."""
import hashlib
import os
import re
from pathlib import Path

import numpy as np
import pytest
from playwright.sync_api import expect

from app_support import invoke, menu, open_project, shot_dir, wait_until
from helpers import SR, synth_track, write_wav

NAMES = [("k in_02", "kick"), ("k out_02", "kick"), ("s top_02", "snare"), ("t2_02", "tom"), ("hh_03", "hat"),
         ("oh_01", "cymbal"), ("Clap_01", "other")]


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def hat_hit(n, sr=SR, decay=0.06):
    """Deterministic hat-like hit: high-passed noise (the DSP hat band is 2-20 kHz, which a kick
    sweep never reaches, #294), short attack, exponential decay."""
    t = np.arange(n) / sr
    x = np.random.default_rng(7).standard_normal(n)
    rc = 1.0 / (2 * np.pi * 4000)
    alpha = rc / (rc + 1 / sr)
    y = np.empty_like(x)
    y[0] = 0.0
    y[1:] = alpha * (y[:-1] + x[1:] - x[:-1])
    return y * np.minimum(1.0, t / 0.0005) * np.exp(-t / decay)


def make_folder(root, names=NAMES, sr=SR, seconds=3.0, name="drop"):
    d = Path(root) / name
    d.mkdir(parents=True, exist_ok=True)
    for n, role in names:
        if role in ("hat", "cymbal"):
            # the DSP hat detector needs high-frequency energy: a kick sweep reads 0 hits, so the
            # export of the flow would skip the track (no-hit tracks are not exported)
            x = np.zeros(int(seconds * sr))
            for t in (0.5, 1.5, 2.5):
                i = int(t * sr)
                h = hat_hit(int(0.2 * sr), sr) * 10 ** (-3.0 / 20)
                x[i:i + len(h)] += h[: len(x) - i]
        else:
            x, _i = synth_track([0.5, 1.5, 2.5], seconds, sr=sr)
        write_wav(d / f"{n}.wav", x, sr=sr)
    return d


def empty_page(page):
    """Close every project of the bridge and show the page with no project open."""
    for s in invoke(page, "project_list"):
        invoke(page, "project_close", sid=s["sid"], force=True)
    page.evaluate("async () => { S.sid = null; setDirtyState(false); await initSessions(); await loadProject(); }")
    page.wait_for_function("() => !S.sid && S.loaded > 0")


def drop(page, *paths):
    """What the shell does with a window drop: the paths reach hooks.otherPaths (see shell.js takeOpenPaths)."""
    page.evaluate("p => { DSShell.hooks.otherPaths(p); }", [str(x) for x in paths])


def rows(page):
    return page.evaluate("""() => [...document.querySelectorAll('#tracksRows .urow')].map(r => ({
        stem: r.dataset.stem, role: r.querySelector('.urole').value, note: r.querySelector('.unote').value,
        use: r.querySelector('.uuse').checked, noguess: r.classList.contains('noguess')}))""")


def loaded(page, n):
    page.wait_for_function("n => S.loaded > 0 && S.order.length === n && S.order.every(k => k in S.peaks)", arg=n, timeout=30000)


def test_a_dropped_folder_shows_the_guessed_roles_and_one_click_adds_them(app, tmp_path):
    folder = make_folder(tmp_path)
    before = {p.name: sha(p) for p in folder.iterdir()}
    empty_page(app)
    drop(app, folder)
    expect(app.locator("#tracksModal")).to_be_visible()
    expect(app.locator("#tracksTitle")).to_have_text("Add 7 tracks")    # #406: Clap_01's audio suggests kick, so it is checked
    expect(app.locator("#tracksGuess")).to_contain_text("6 of 7 roles come from the names. 1 from the audio.")
    got = {r["stem"]: r for r in rows(app)}
    expected = dict(NAMES)
    expected["Clap_01"] = "kick"                                          # #225 from audio: the model-free core (a kick on the mic)
    assert {k: v["role"] for k, v in got.items()} == expected
    assert got["k in_02"]["note"] == "C1" and got["s top_02"]["note"] == "D1" and got["t2_02"]["note"] == "C2"
    assert got["Clap_01"]["noguess"] is False and got["k in_02"]["noguess"] is False
    assert all(v["use"] for v in got.values())
    assert app.locator("#tracksCopy").is_checked() is False               # referenced by default (decision 6)
    expect(app.locator("#tracksAdd")).to_have_text("Add 7 tracks")
    assert app.evaluate("() => document.activeElement.id") == "tracksAdd"  # Enter accepts
    app.click("#tracksAll")                                                # All keeps every row checked
    expect(app.locator("#tracksAdd")).to_have_text("Add 7 tracks")
    expect(app.locator("#tracksTitle")).to_have_text("Add 7 tracks")
    app.click("#tracksAdd")
    loaded(app, 7)
    sid = app.evaluate("() => S.sid")
    p = invoke(app, "project_get", sid=sid)
    assert p["name"] == "drop"                                            # named after the folder
    assert p["dirty"] is True
    t = p["tracks"]["k_in_02"]
    assert (t["role"], t["note"], t["status"], t["source"]["mode"]) == ("kick", 36, "ok", "referenced")
    assert t["source"]["path"] == str(folder / "k in_02.wav")
    assert [(p["tracks"][k]["role"], p["tracks"][k]["note"]) for k in ("s_top_02", "t2_02", "hh_03", "oh_01", "Clap_01")] == \
        [("snare", 38), ("tom", 48), ("hat", 42), ("cymbal", 49), ("kick", 36)]
    assert not (Path(p["folder"]) / "Audio Files" / "k in_02.wav").exists()
    assert {x.name: sha(x) for x in folder.iterdir()} == before            # the sources are untouched
    expect(app.locator("#stMsg")).to_contain_text("added 7 tracks (audio referenced where it is)")
    expect(app.locator("#tracksModal")).to_be_hidden()
    assert app.evaluate("() => S.tracks.k_in_02.role") == "kick"


def test_four_user_actions_from_the_folder_drop_to_export_all(app, tmp_path):
    """E14: drop, accept the roles, Analyze all, Export all. No typing: no key is pressed, no text field is filled."""
    folder = make_folder(tmp_path, names=NAMES[:5])
    empty_page(app)
    app.evaluate("""() => { window.__actions = []; window.__keys = 0;
        document.addEventListener('keydown', () => { window.__keys++; }, true);
        document.addEventListener('input', () => { window.__keys++; }, true); }""")
    drop(app, folder)                                                       # action 1: the drop
    app.evaluate("() => window.__actions.push('drop')")
    app.click("#tracksAdd")                                                 # action 2: accept the guessed roles
    app.evaluate("() => window.__actions.push('accept')")
    loaded(app, 5)
    app.click("#analyzeAllBtn")                                             # action 3
    app.evaluate("() => window.__actions.push('analyze')")
    expect(app.locator("#progressText")).to_have_text("analysis done", timeout=120000)
    app.click("#exportAllBtn")                                              # action 4: the dialog opens with everything chosen (P4-30: one more click)
    app.click("#exGo")
    app.evaluate("() => window.__actions.push('export')")
    sid = app.evaluate("() => S.sid")
    out = Path(invoke(app, "project_get", sid=sid)["folder"]) / "Exports"
    # #96: 60 s starved at loadavg 20+ (attempt 1 run 1); the analysis step already
    # allows 120 s, so the export gets the same bound.
    wait_until(lambda: out.exists() and len(list(out.glob("*_spike.*"))) == 10, timeout=120, msg="10 export files")
    assert app.evaluate("() => window.__actions") == ["drop", "accept", "analyze", "export"]
    assert app.evaluate("() => window.__keys") == 0
    assert sum(app.evaluate("() => Object.values(S.events).map(e => e.length)")) >= 5


def test_files_the_app_cannot_read_are_listed_with_the_reason_and_not_added(app, tmp_path):
    folder = make_folder(tmp_path, names=NAMES[:2])
    x, _ = synth_track([0.5], 2.0, sr=88200)
    write_wav(folder / "rate88.wav", x, sr=88200)
    (folder / "old.aif").write_bytes(b"FORM\x00\x00\x00\x04AIFF")
    (folder / "garbage.wav").write_bytes(b"this is not a wav file")
    empty_page(app)
    drop(app, folder)
    expect(app.locator("#tracksTitle")).to_have_text("Add 2 tracks")
    rej = app.locator("#tracksRejected .rejrow").all_text_contents()
    assert len(rej) == 3
    text = "\n".join(rej)
    assert re.search(r"old\.aif - AIFF files are not supported", text), text
    assert re.search(r"rate88\.wav - the sample rate 88200 Hz is not supported: Drum Spike reads 44\.1, 48 or 96 kHz", text), text
    assert re.search(r"garbage\.wav - this WAV file is not supported", text), text
    app.click("#tracksAdd")
    loaded(app, 2)
    assert app.evaluate("() => S.order") == ["k_in_02", "k_out_02"]
    expect(app.locator("#stMsg")).to_contain_text("3 not added")
    assert "AIFF" in app.locator("#stMsg").get_attribute("title")


def test_a_drop_of_only_unreadable_files_has_nothing_to_add(app, tmp_path):
    aiff = tmp_path / "x.aiff"
    aiff.write_bytes(b"FORM\x00\x00\x00\x04AIFF")
    empty_page(app)
    drop(app, aiff)
    expect(app.locator("#tracksTitle")).to_have_text("No track can be added")
    expect(app.locator("#tracksAdd")).to_be_hidden()
    expect(app.locator("#tracksCancel")).to_have_text("Close")
    app.click("#tracksCancel")
    expect(app.locator("#tracksModal")).to_be_hidden()
    assert app.evaluate("() => S.sid") is None                              # no project was made for nothing


def test_cancel_adds_nothing_and_makes_no_project(app, tmp_path):
    folder = make_folder(tmp_path, names=NAMES[:2])
    empty_page(app)
    n = len(invoke(app, "project_list"))
    drop(app, folder)
    app.click("#tracksCancel")
    expect(app.locator("#stMsg")).to_have_text("adding tracks cancelled")
    assert app.evaluate("() => S.sid") is None and len(invoke(app, "project_list")) == n


def test_a_drop_into_an_open_project_adds_to_it_and_the_box_copies_the_audio(app, tmp_path, make_project):
    sid = open_project(app, make_project())
    n0 = len(app.evaluate("() => S.order"))
    folder = make_folder(tmp_path, names=NAMES[:2])
    src = {p.name: p.read_bytes() for p in folder.iterdir()}
    drop(app, folder)
    app.check("#tracksCopy")
    app.click("#tracksAdd")
    loaded(app, n0 + 2)
    assert app.evaluate("() => S.sid") == sid                               # the same project, no new one
    p = invoke(app, "project_get", sid=sid)
    t = p["tracks"]["k_in_02"]
    assert t["source"]["mode"] == "copied" and t["source"]["relative"] == "Audio Files/k in_02.wav"
    assert (Path(p["folder"]) / "Audio Files" / "k in_02.wav").read_bytes() == src["k in_02.wav"]
    expect(app.locator("#stMsg")).to_contain_text("audio copied into the project")


def test_a_second_drop_of_the_same_name_gets_unique_track_and_project_names(app, tmp_path):
    folder = make_folder(tmp_path, names=NAMES[:1], name="Song")
    empty_page(app)
    drop(app, folder)
    app.click("#tracksAdd")
    loaded(app, 1)
    first = invoke(app, "project_get", sid=app.evaluate("() => S.sid"))["name"]
    # the first project stays open in the registry; a page with no project takes the next drop
    app.evaluate("async () => { S.sid = null; setDirtyState(false); await loadProject(); }")
    app.wait_for_function("() => !S.sid && S.loaded > 0")
    drop(app, folder)
    app.click("#tracksAdd")
    loaded(app, 1)
    assert first == "Song"
    assert invoke(app, "project_get", sid=app.evaluate("() => S.sid"))["name"] == "Song 2"
    drop(app, folder)                                                       # the same file again: the name gets a number
    app.click("#tracksAdd")
    loaded(app, 2)
    assert app.evaluate("() => S.order") == ["k_in_02", "k_in_02_2"]


def test_a_dropped_file_in_audio_files_names_the_project_after_the_session(app, tmp_path):
    session = tmp_path / "Session A - Mix"
    d = session / "Audio Files"
    d.mkdir(parents=True)
    x, _ = synth_track([0.5], 2.0)
    write_wav(d / "oh_01.wav", x)
    empty_page(app)
    drop(app, d / "oh_01.wav")
    app.click("#tracksAdd")
    loaded(app, 1)
    assert invoke(app, "project_get", sid=app.evaluate("() => S.sid"))["name"] == "Session A - Mix"
    assert app.evaluate("() => S.tracks.oh_01.role") == "cymbal"
    empty_page(app)
    drop(app, session)                                                      # a session folder stands for its Audio Files
    expect(app.locator("#tracksTitle")).to_have_text("Add 1 track")
    app.click("#tracksCancel")


def test_roles_and_notes_can_be_changed_and_unchecked_files_stay_out(app, tmp_path, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    folder = make_folder(tmp_path, names=NAMES[:3])
    drop(app, folder)
    row = app.locator("#tracksRows .urow").nth(0)                           # k in_02
    row.locator(".urole").select_option("tom")
    assert row.locator(".unote").input_value() == "C2"                      # the note follows the role
    app.locator("#tracksRows .urow").nth(1).locator(".uuse").uncheck()
    expect(app.locator("#tracksAdd")).to_have_text("Add 2 tracks")
    app.locator("#tracksRows .urow").nth(2).locator(".unote").fill("F#1")
    app.click("#tracksAdd")
    loaded(app, 3)
    s = app.evaluate("() => [S.order, S.tracks.k_in_02.role, S.tracks.k_in_02.note, S.tracks.s_top_02.note]")
    assert s == [["Kick In", "k_in_02", "s_top_02"], "tom", 48, 42]
    app.evaluate("() => { DSShell.hooks.otherPaths([]); }")                 # an empty drop does nothing
    expect(app.locator("#tracksModal")).to_be_hidden()


def test_unchecking_every_file_disables_add(app, tmp_path, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    drop(app, make_folder(tmp_path, names=NAMES[:1]))
    app.locator("#tracksRows .urow .uuse").uncheck()
    expect(app.locator("#tracksAdd")).to_be_disabled()
    expect(app.locator("#tracksAdd")).to_have_text("Add 0 tracks")
    app.click("#tracksCancel")


def test_a_second_drop_while_the_list_is_open_is_refused(app, tmp_path, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    folder = make_folder(tmp_path, names=NAMES[:1])
    drop(app, folder)
    drop(app, folder)
    expect(app.locator("#stMsg")).to_have_text("answer the open dialog first")
    expect(app.locator("#tracksTitle")).to_have_text("Add 1 track")
    app.click("#tracksCancel")


def test_kit_presets_are_saved_listed_and_applied_without_typing(app, tmp_path, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    folder = Path(tmp_path) / "Studio Kit"
    folder.mkdir()
    # Clap_01 and Sample_01 carry audio the core scores None (steady noise, no hits, #406):
    # the preset test needs files whose name gives no role and whose audio gives no suggestion.
    n = int(3.0 * SR)
    lp = np.zeros(n)
    noise = np.random.default_rng(7).standard_normal(n)
    for i in range(1, n):
        lp[i] = 0.995 * lp[i - 1] + 0.005 * noise[i]
    write_wav(folder / "Clap_01.wav", (lp * 30.0).astype(np.float32))
    write_wav(folder / "Sample_01.wav", (lp * 30.0).astype(np.float32))
    x, _i = synth_track([0.5, 1.5, 2.5], 3.0)
    write_wav(folder / "k in_02.wav", x)
    drop(app, str(folder))
    expect(app.locator("#tracksRows .urow")).to_have_count(3)
    rws = {r["stem"]: r for r in rows(app)}
    assert rws["Clap_01"]["role"] == "other" and rws["Sample_01"]["role"] == "other"
    # the engineer picks the role of Clap once and saves the list as a kit preset (one click, named after the folder)
    app.locator("#tracksRows .urow", has_text="Clap_01").locator(".urole").select_option("cymbal")
    app.click("#tracksPresetSave")
    expect(app.locator("#stMsg")).to_contain_text('saved the kit preset "Studio Kit" (3 names)')
    assert app.locator("#tracksPreset option").all_text_contents() == ["Kit preset: names of the scheme", "Studio Kit"]
    assert app.locator("#tracksPreset").input_value() == "Studio Kit"
    app.click("#tracksCancel")
    assert invoke(app, "kit_presets_list") == [{"name": "Studio Kit", "rules": 3}]
    assert (app.home / "kit-presets" / "Studio Kit.json").is_file()
    # a later drop: choose the preset in the list, the roles follow it
    drop(app, folder)
    expect(app.locator("#tracksModal")).to_be_visible()
    wait_until(lambda: {r["stem"]: r["role"] for r in rows(app)}["Clap_01"] == "other", msg="the new list")
    app.locator("#tracksPreset").select_option("Studio Kit")
    wait_until(lambda: {r["stem"]: r["role"] for r in rows(app)}["Clap_01"] == "cymbal", msg="preset applied")
    app.locator("#tracksPreset").select_option("")
    wait_until(lambda: {r["stem"]: r["role"] for r in rows(app)}["Clap_01"] == "other", msg="preset removed")
    app.click("#tracksCancel")
    loaded_rules = invoke(app, "kit_preset_load", name="Studio Kit")
    assert {"pattern": "clap", "role": "cymbal", "note": 49} in loaded_rules["rules"]
    err = invoke(app, "kit_preset_load", name="Nope")
    assert err["code"] == 404


def test_the_commands_answer_their_argument_errors_over_the_bridge(app):
    assert invoke(app, "tracks_scan", paths=3)["code"] == 400
    assert invoke(app, "tracks_add", sid="nope", paths=[])["code"] == 404
    assert invoke(app, "kit_preset_save", name="x", rules="nope")["code"] == 400
    assert invoke(app, "tracks_scan", paths=[])["files"] == []


def test_the_menu_and_the_button_open_the_native_file_dialog_and_use_its_files(app, tmp_path, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    folder = make_folder(tmp_path, names=NAMES[:2])
    files = [str(folder / "k in_02.wav"), str(folder / "k out_02.wav")]
    app.evaluate("f => { window.__DS_PICK__ = f; window.__dialogOpts = null; }", files)
    menu(app, "add_tracks", wait=False)                                     # the menu call waits for the user's answer: not awaited here
    expect(app.locator("#tracksModal")).to_be_visible()
    opts = app.evaluate("() => window.__dialogOpts")
    assert opts["multiple"] is True and opts["directory"] is False and "wav" in opts["filters"][0]["extensions"]
    expect(app.locator("#tracksTitle")).to_have_text("Add 2 tracks")
    app.click("#tracksCancel")
    app.evaluate("f => { window.__DS_PICK__ = f[0]; }", files)               # a single path from the dialog
    app.click("#addTracksBtn")
    expect(app.locator("#tracksTitle")).to_have_text("Add 1 track")
    app.click("#tracksAdd")
    loaded(app, 2)
    app.evaluate("() => { window.__DS_PICK__ = null; }")
    app.click("#addTracksBtn")                                              # the user cancels the file dialog
    expect(app.locator("#tracksModal")).to_be_hidden()
    # without a project the menu item asks for one
    empty_page(app)
    assert menu(app, "add_tracks") is True
    expect(app.locator("#tracksModal")).to_be_hidden()
    expect(app.locator("#stMsg")).to_contain_text("no project")


def test_the_empty_state_names_the_drop(app):
    empty_page(app)
    app.evaluate("async () => { const r = await B.newProject('Empty', null); await useProject(r); }")
    expect(app.locator(".empty-state")).to_contain_text("drop audio files or a folder here")


def test_the_scan_reads_only_headers_and_the_words_are_plain(app, tmp_path):
    folder = make_folder(tmp_path, names=NAMES[:1])
    r = invoke(app, "tracks_scan", paths=[str(folder)])
    f = r["files"][0]
    assert (f["sr"], f["channels"], f["bits"], f["n_samples"], f["guess"], f["track"]) == (SR, 1, 16, 3 * SR, "scheme", "k_in_02")
    assert abs(f["duration"] - 3.0) < 1e-9
    assert r["guessed"] == 1 and r["rejected"] == [] and r["elapsed_ms"] >= 0


# X-1 (P3-9b): the 70 file names of the real Pro Tools session "Session A" (Audio Files). 20 have a drum role
# from the names; the other 50 are guitars, vocals, rooms, samples and a rough mix.
SESSION_A = [
    "Bass_01", "Clap_01", "GTR 1 DBL_01", "GTR 1_01", "GTR 2 DBL_01", "GTR 2_01", "GTR low center_01", "Gtr Harm 1_01",
    "Gtr Harm 2_01", "Horns_01", "Orch_01", "Organ_01", "Piano low_01", "Piano melody_01", "Rough_02", "Shaker_01",
    "String Low_01", "Strings_01", "Synths_01", "Tambourine_01", "Timpani Roll_01", "Timpani_01", "Trem Strings_01",
    "ac 1 St_01", "ac 2 St_01", "choir carsten_01", "choir chris_01", "harm 1_01", "harm 2_01", "harm 3_01",
    "harm 4_01", "hh_03", "k in_02", "k out_02", "k sampl savior_01", "k sampl sd3_01", "k sampl trig_01", "k sub_02",
    "lv br ch_01", "lv harm vers_01", "lv vers_01", "oh_01", "ohh 1_01", "ohh 2_01", "ohh 3_01", "ohh 4_01",
    "ohh 5_01", "r  toms soap_01", "r efx_02", "r far_01", "r k s soap_01", "r mono_02", "r sampl_01", "r tba_02",
    "ro_02", "s bot_02", "s sampl savior_01", "s sampl sd3_01", "s sampl trig_01", "s top_02", "solo dbl_01",
    "solo_01", "t1 hh_02", "t1 sampl_01", "t2 sampl_01", "t2_02", "t3 sampl_01", "t3_02", "t4 ri_02", "t4 sampl_01",
]
SESSION_A_DRUMS = {"hh_03", "k in_02", "k out_02", "k sub_02", "oh_01", "ohh 1_01", "ohh 2_01", "ohh 3_01", "ohh 4_01",
                 "ohh 5_01", "s bot_02", "s top_02", "t1 hh_02", "t1 sampl_01", "t2 sampl_01", "t2_02", "t3 sampl_01",
                 "t3_02", "t4 ri_02", "t4 sampl_01"}


def checked(page):
    return {r["stem"] for r in rows(page) if r["use"]}


def test_a_full_session_folder_checks_only_the_drum_files_and_the_buttons_switch(app, tmp_path):
    """X-1: a drop of a Pro Tools "Audio Files" folder with drum and other files checks only the files with a
    drum role. "All" checks every file, "Drums only" the rows whose role is not other; the count follows."""
    assert len(SESSION_A) == 70 and len(SESSION_A_DRUMS) == 20 and SESSION_A_DRUMS <= set(SESSION_A)
    folder = make_folder(tmp_path, names=[(n, "") for n in SESSION_A], seconds=0.5, name="Audio Files")
    empty_page(app)
    drop(app, folder)
    expect(app.locator("#tracksModal")).to_be_visible()
    expect(app.locator("#tracksRows .urow")).to_have_count(70)
    assert checked(app) == SESSION_A_DRUMS
    expect(app.locator("#tracksCount")).to_have_text("20 of 70 checked")
    expect(app.locator("#tracksAdd")).to_have_text("Add 20 tracks")
    expect(app.locator("#tracksTitle")).to_have_text("Add 20 tracks")
    expect(app.locator("#tracksGuess")).to_contain_text("20 of 70 roles come from the names")
    app.click("#tracksAll")
    expect(app.locator("#tracksCount")).to_have_text("70 of 70 checked")
    expect(app.locator("#tracksAdd")).to_have_text("Add 70 tracks")
    assert len(checked(app)) == 70
    app.click("#tracksDrums")
    expect(app.locator("#tracksCount")).to_have_text("20 of 70 checked")
    assert checked(app) == SESSION_A_DRUMS
    # a role picked by hand makes the file a drum file: the row is checked, and "Drums only" keeps it
    app.locator("#tracksRows .urow", has_text="Shaker_01").locator(".urole").select_option("hat")
    expect(app.locator("#tracksCount")).to_have_text("21 of 70 checked")
    app.locator("#tracksRows .urow", has_text="hh_03").locator(".uuse").uncheck()
    expect(app.locator("#tracksCount")).to_have_text("20 of 70 checked")
    app.click("#tracksDrums")
    assert checked(app) == SESSION_A_DRUMS | {"Shaker_01"}
    app.click("#tracksAdd")
    loaded(app, 21)
    s = app.evaluate("() => [S.order.length, S.tracks.Shaker_01.role, 'Bass_01' in S.tracks, 'r_toms_soap_01' in S.tracks]")
    assert s == [21, "hat", False, False]


def test_a_folder_without_any_drum_name_checks_every_file(app, tmp_path):
    """X-1: when no file has a drum role (names outside the scheme, no preset), every file stays checked."""
    folder = make_folder(tmp_path, names=[("Clap_01", ""), ("Bass_01", "")], name="no drums")
    empty_page(app)
    drop(app, folder)
    expect(app.locator("#tracksCount")).to_have_text("2 of 2 checked")
    expect(app.locator("#tracksAdd")).to_have_text("Add 2 tracks")
    app.click("#tracksCancel")


def test_the_file_dialog_with_drum_and_other_files_checks_only_the_drums(app, tmp_path, make_project):
    """X-1: File > Add Tracks uses the same list: the picked files without a drum role are not checked."""
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    folder = make_folder(tmp_path, names=NAMES)
    app.evaluate("f => { window.__DS_PICK__ = f; }", [str(folder / f"{n}.wav") for n, _ in NAMES])
    app.click("#addTracksBtn")
    expect(app.locator("#tracksModal")).to_be_visible()
    expect(app.locator("#tracksCount")).to_have_text("7 of 7 checked")     # #406: Clap_01's audio suggests kick
    assert checked(app) == {n for n, _ in NAMES}
    app.click("#tracksCancel")
    app.evaluate("() => { window.__DS_PICK__ = null; }")


def test_an_unnamed_file_gets_the_audio_suggestion_and_the_mark(app, tmp_path):
    """#225 (gate c): a file whose name gives no role gets the role the audio suggests in the
    Add Tracks dialog: the select is preset to it, the row is marked "from audio", the summary
    counts it, and picking a role by hand removes the mark.

    The fixture is a synthetic kick (helpers.synth_track) that the model-free core
    (drumspike_core::role::role_from_audio, #393) suggests Kick for: the core-level
    assertion is `crates/core/src/role/tests.rs::the_add_tracks_e2e_fixture_is_a_kick`
    (the same pitch sweep and envelope; role-from-audio on this exact file gives
    kick, confidence 1.0)."""
    folder = Path(tmp_path) / "audio roles"
    folder.mkdir()
    x, _i = synth_track([0.5, 1.0, 1.5, 2.0, 2.5, 3.0], 4.0)
    write_wav(folder / "new take.wav", x)
    scan = invoke(app, "tracks_scan", paths=[str(folder)])
    row = scan["files"][0]
    assert row["guess"] == "audio" and row["role"] == "kick", row

    def mark_count():
        return app.evaluate("() => document.querySelectorAll('#tracksRows .ufromaudio').length")

    empty_page(app)
    drop(app, folder)
    expect(app.locator("#tracksModal")).to_be_visible()
    got = {r["stem"]: r for r in rows(app)}
    assert got["new take"]["role"] == row["role"]          # the select is preset to the suggestion
    assert got["new take"]["noguess"] is False
    assert got["new take"]["note"] == app.evaluate("n => nameFromNote(n)", row["note"])
    assert mark_count() == 1
    expect(app.locator(".ufromaudio")).to_have_text("from audio")
    expect(app.locator("#tracksGuess")).to_contain_text("1 from the audio")
    expect(app.locator("#tracksCount")).to_have_text("1 of 1 checked")
    # the user can change it before the add: the mark goes, the row is no "no guess" row
    app.select_option(".urole", "other")
    assert mark_count() == 0
    assert rows(app)[0]["noguess"] is False


def test_a_file_the_core_scores_none_gets_no_suggestion(app, tmp_path):
    """#406: a file whose audio the model-free core scores None (steady pink-ish noise, no
    hits) gets no suggestion: the row stays a "no guess" row, no "from audio" mark."""
    folder = Path(tmp_path) / "no role"
    folder.mkdir()
    n = int(4.0 * SR)
    rng = np.random.default_rng(7)
    lp = np.zeros(n)
    noise = rng.standard_normal(n)
    for i in range(1, n):
        lp[i] = 0.995 * lp[i - 1] + 0.005 * noise[i]
    write_wav(folder / "steady hum.wav", (lp * 30.0).astype(np.float32))
    scan = invoke(app, "tracks_scan", paths=[str(folder)])
    row = scan["files"][0]
    assert row["guess"] == "none" and row["role"] == "other", row

    empty_page(app)
    drop(app, folder)
    expect(app.locator("#tracksModal")).to_be_visible()
    got = {r["stem"]: r for r in rows(app)}
    assert got["steady hum"]["noguess"] is True
    assert app.evaluate("() => document.querySelectorAll('#tracksRows .ufromaudio').length") == 0


def test_tracks_scan_of_20_unnamed_files_stays_under_2_s(app, tmp_path):
    """#406 (gate): the audio suggestion of 20 unnamed files does not blow the scan's time
    budget: `tracks_scan` answers under 2 s (the time is printed for the gate report)."""
    folder = Path(tmp_path) / "twenty"
    folder.mkdir()
    x, _i = synth_track([0.5, 1.0, 1.5, 2.0, 2.5, 3.0], 4.0)
    for i in range(1, 21):
        write_wav(folder / f"new take {i:02d}.wav", x)
    scan = invoke(app, "tracks_scan", paths=[str(folder)])
    assert len(scan["files"]) == 20
    assert all(f["guess"] == "audio" and f["role"] == "kick" for f in scan["files"])
    print(f"tracks_scan of 20 files: {scan['elapsed_ms']:.0f} ms")
    assert scan["elapsed_ms"] < 2000.0, scan["elapsed_ms"]


READY = "typeof S !== 'undefined' && typeof B !== 'undefined' && B.app === true && S.loaded > 0"


def change_role(page, stem, role):
    """Picks `role` in the row's role select; if the row already has it, another drum role (a select
    to the same value fires no change)."""
    row = page.locator("#tracksRows .urow", has_text=stem)
    current = row.locator(".urole").input_value()
    row.locator(".urole").select_option(role if role != current else "tom")


def test_a_role_picked_by_hand_leaves_the_audio_count_and_counts_as_set_by_you(app, tmp_path):
    """#243 (gate 1): the summary line is recounted on every role change. A folder with one
    name-guessed file and one whose role the audio suggests: after a hand change the audio
    suggestion no longer counts as "from the audio"; the hand-picked roles count as "set by you".
    Screenshots of the dialog go to the shot dir for the coordinator (1x and 2x)."""
    folder = Path(tmp_path) / "roles recount"
    folder.mkdir()
    x, _i = synth_track([0.5, 1.0, 1.5, 2.0, 2.5, 3.0], 4.0)
    write_wav(folder / "k in_02.wav", x)      # the name gives the role
    write_wav(folder / "new take.wav", x)     # the name gives none: the audio suggests the role
    scan = invoke(app, "tracks_scan", paths=[str(folder)])
    audio = [f for f in scan["files"] if f["guess"] == "audio"]
    if not audio:
        pytest.skip(f"no model in this run: the scan guessed {sorted({f['guess'] for f in scan['files']})}")
    assert [f["stem"] for f in audio] == ["new take"]

    def recount_asserts(page):
        text = page.locator("#tracksGuess").text_content()
        assert "from the audio" not in text, text
        assert "1 of 2 roles come from the names" in text, text
        assert "1 set by you" in text, text

    empty_page(app)
    drop(app, folder)
    expect(app.locator("#tracksModal")).to_be_visible()
    expect(app.locator("#tracksGuess")).to_contain_text("1 of 2 roles come from the names")
    expect(app.locator("#tracksGuess")).to_contain_text("1 from the audio")
    change_role(app, "new take", "tom")
    recount_asserts(app)
    app.locator("#tracksModal").screenshot(path=str(shot_dir("243-addtracks-summary-1x.png")))
    app.click("#tracksCancel")
    expect(app.locator("#tracksModal")).to_be_hidden()
    # the same recount at 2x (Retina), in its own page over the same bridge
    ctx = app.context.browser.new_context(device_scale_factor=2)
    try:
        p2 = ctx.new_page()
        p2_errors = []
        p2.on("pageerror", lambda e: p2_errors.append(str(e)))
        p2.goto(app.url_base + "/")
        p2.wait_for_function(READY, timeout=120000)
        empty_page(p2)
        drop(p2, folder)
        expect(p2.locator("#tracksModal")).to_be_visible()
        change_role(p2, "new take", "tom")
        recount_asserts(p2)
        p2.locator("#tracksModal").screenshot(path=str(shot_dir("243-addtracks-summary-2x.png")))
        p2.click("#tracksCancel")
        assert p2_errors == [], p2_errors
    finally:
        ctx.close()
    # a name-guessed row changed by hand leaves the names count too (the recount covers every row)
    drop(app, folder)
    expect(app.locator("#tracksModal")).to_be_visible()
    change_role(app, "new take", "tom")
    change_role(app, "k in_02", "snare")
    text = app.locator("#tracksGuess").text_content()
    assert "0 of 2 roles come from the names" in text, text
    assert "2 set by you" in text, text
    app.click("#tracksCancel")
