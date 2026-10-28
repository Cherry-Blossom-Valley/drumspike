"""P3-18: soft tracks (owner decision 2026-10-12). The Sensitivity slider reaches 150 (thresholds down to 0.01; 0 to 100
is the mapping of P3-14a, so old projects give the same thresholds), a track without a hit gets its sensitivity raised
by the service in steps (95, 113, 134, 150) and says so, and a track can follow another track's hits (Inspector "Hits",
Track > Follow Hits Of, Own Detection): the same sample positions, velocities from its own signal, read-only lanes,
exports of the derived hits, the follow saved in the document. The Add Tracks list suggests the follow for a sub mic."""
import json
import wave
from pathlib import Path

import pytest
from playwright.sync_api import expect

from app_support import inspect_track, invoke, menu, open_project, track_menu, wait_until
from helpers import SR, synth_track, write_wav
from test_editor import IDX, ONSETS, exact_events, service_events
from test_menus import state as menu_state
from test_tracks import drop, empty_page, loaded, rows

OLD_AT_100 = [0.088, 0.096, 0.128, 0.088, 0.12]   # the end of the slider before P3-18: base * 0.4


def js(page, expr, arg=None):
    return page.evaluate(expr, arg)


def doc_of(folder):
    return json.loads(next(Path(folder).glob("*.drumspike")).read_text())


def build(make_project, tmp_path, tracks, name="Sub", seconds=4.0):
    """A copied project. tracks: (name, role, note, gain_db or None for silence, events or None)."""
    src = tmp_path / f"src_{name}"
    src.mkdir()
    specs = []
    for tname, role, note, gain, events in tracks:
        if gain is None:
            x = [0.0] * int(seconds * SR)
        else:
            x, _ = synth_track(ONSETS, seconds, gains_db=[gain] * len(ONSETS))
        specs.append((tname, write_wav(src / f"{tname}.wav", x), role, note, events))
    return Path(make_project.builder.build_project(tmp_path / f"projects_{name}", name, specs, "copied"))


def kick_and_sub(make_project, tmp_path, sub_gain=-30.0, **kw):
    """Kick In with exact events; Kick Sub the same hits 27 dB softer, no events."""
    return build(make_project, tmp_path, [("Kick In", "kick", 36, -3.0, exact_events()),
                                          ("Kick Sub", "kick", 36, sub_gain, None)], **kw)


def role_line(page, name):
    return page.locator(f'.track[data-name="{name}"] .tnote').inner_text()


def has_model(page):
    info = invoke(page, "app_info")
    return bool(info.get("model") and info["model"].get("ok"))


def analyse(page):
    page.click("#analyzeAllBtn")
    expect(page.locator("#progressText")).to_have_text("analysis done", timeout=90000)
    page.wait_for_function("() => !analysisRunning")


def test_the_sensitivity_slider_reaches_150_and_old_thresholds_keep_their_value(app, make_project, tmp_path):
    folder = build(make_project, tmp_path, [("Kick In", "kick", 36, -3.0, exact_events()), ("Old", "kick", 36, -3.0, None)])
    doc = doc_of(folder)   # an older project with thresholds from the P3-14a slider at 100
    doc["tracks"]["Old"]["params"] = {"thresholds": OLD_AT_100, "min_gap_ms": 20, "refine": True, "vel_lo_db": -40, "vel_hi_db": -6}
    next(Path(folder).glob("*.drumspike")).write_text(json.dumps(doc, indent=1))
    open_project(app, folder)
    ins = inspect_track(app, "Kick In")
    sens = ins.locator(".pSens")
    assert sens.get_attribute("max") == "150"
    expect(sens).to_have_value("50")                                     # the defaults stand at 50 (the P3-14a readout showed 66)
    expect(ins.locator(".pSensVal")).to_have_text("thresholds ≈ 0.22 … 0.32")
    expect(ins.locator(".pSensNoteRow")).to_be_hidden()
    sens.fill("100")
    p = js(app, "() => S.tracks['Kick In'].params")
    assert [round(t, 6) for t in p["thresholds"]] == OLD_AT_100           # 100 is the old end: the same values
    assert p["sensitivity"] == 100
    expect(ins.locator(".pSensVal")).to_have_text("thresholds ≈ 0.09 … 0.13")
    sens.fill("150")
    p = js(app, "() => S.tracks['Kick In'].params")
    assert min(p["thresholds"]) == pytest.approx(0.01, abs=1e-9) and max(p["thresholds"]) < 0.015
    expect(ins.locator(".pSensVal")).to_have_text("thresholds ≈ 0.01 … 0.01")
    sens.fill("125")
    p = js(app, "() => S.tracks['Kick In'].params")
    assert 0.01 < min(p["thresholds"]) < 0.088                           # between the old end and the floor
    assert js(app, "() => [sensThresholds(0)[0], sensThresholds(50), Math.round(sensOf([0.3,0.3,0.3,0.3,0.3]))]") == \
        [pytest.approx(0.352), [0.22, 0.24, 0.32, 0.22, 0.30], 37]
    # the old track: the slider stands where its thresholds are, and the thresholds are untouched until the slider moves
    ins = inspect_track(app, "Old")
    expect(ins.locator(".pSens")).to_have_value("100")
    expect(ins.locator(".pSensVal")).to_have_text("thresholds ≈ 0.09 … 0.13")
    assert js(app, "() => S.tracks['Old'].params.thresholds") == OLD_AT_100
    assert "sensitivity" not in js(app, "() => S.tracks['Old'].params")
    invoke(app, "project_save", sid=js(app, "() => S.sid"))
    assert doc_of(folder)["tracks"]["Old"]["params"]["thresholds"] == OLD_AT_100


def test_a_track_without_hits_gets_its_sensitivity_raised_or_says_no_hits_found(app, make_project, tmp_path):
    if not has_model(app):
        pytest.skip("the retry runs on the model detector only (the DSP fallback ignores the thresholds)")
    folder = build(make_project, tmp_path, [("Kick In", "kick", 36, -3.0, None), ("Kick Sub", "kick", 36, -18.0, None),   # -18 dB: 0 hits at 50, 95 and 113, 3 at 134 (measured with service_tool core-analyze)
                                             ("Room", "other", 48, None, None)], name="Weak")
    sid = open_project(app, folder)
    analyse(app)
    expect(app.locator("#stMsg")).to_contain_text("Kick Sub: weak signal: sensitivity raised to ")
    expect(app.locator("#stMsg")).to_contain_text("Room: no hits found (sensitivity up to 150 tried)")
    assert "Kick In:" not in app.locator("#stMsg").inner_text()           # a track with hits at its sensitivity says nothing
    tracks = invoke(app, "tracks_get", sid=sid)["tracks"]
    sub, room, kick = tracks["Kick Sub"]["params"], tracks["Room"]["params"], tracks["Kick In"]["params"]
    assert sub["weak_signal"]["from"] == 50 and sub["weak_signal"]["to"] in (95, 113, 134, 150)
    assert sub["sensitivity"] == sub["weak_signal"]["to"] and min(sub["thresholds"]) < 0.11
    assert tracks["Kick Sub"]["n_events"] >= 1
    assert room["weak_signal"] == {"from": 50, "to": None} and tracks["Room"]["n_events"] == 0
    assert "weak_signal" not in kick and "sensitivity" not in kick
    # the Inspector: the slider stands at the raised value, the note says so; the page's params follow the service
    ins = inspect_track(app, "Kick Sub")
    expect(ins.locator(".pSens")).to_have_value(str(sub["sensitivity"]))
    expect(ins.locator(".pSensNote")).to_have_text(f"weak signal: sensitivity raised to {sub['sensitivity']} (from 50)")
    assert js(app, "() => S.tracks['Kick Sub'].params.sensitivity") == sub["sensitivity"]
    ins = inspect_track(app, "Room")
    expect(ins.locator(".pSensNote")).to_have_text("no hits found (sensitivity up to 150 tried)")
    expect(ins.locator(".pSens")).to_have_value("50")
    # the next analysis starts at the raised sensitivity: no retry, the note is kept with its values
    analyse(app)
    assert invoke(app, "tracks_get", sid=sid)["tracks"]["Kick Sub"]["params"]["sensitivity"] == sub["sensitivity"]
    # a move of the slider drops the note
    ins = inspect_track(app, "Kick Sub")
    js(app, "() => { const s = document.querySelector('#insParams .pSens'); s.value = 60; s.dispatchEvent(new Event('input')); }")
    expect(ins.locator(".pSensNoteRow")).to_be_hidden()
    assert "weak_signal" not in js(app, "() => S.tracks['Kick Sub'].params")
    # P4-3: letting go re-picks the hits from the kept activations and stores them, as a Re-analyze at 60 does:
    # no hit at 60, so the retry raises the sensitivity again and says so (from 60)
    js(app, "() => document.querySelector('#insParams .pSens').dispatchEvent(new Event('change'))")
    app.wait_for_function("() => !HG.sens.base && !DSHist.state().busy")
    p = js(app, "() => S.tracks['Kick Sub'].params")
    assert p["weak_signal"]["from"] == 60 and p["sensitivity"] == p["weak_signal"]["to"] >= 95, p
    expect(inspect_track(app, "Kick Sub").locator(".pSensNote")).to_have_text(
        f"weak signal: sensitivity raised to {p['sensitivity']} (from 60)")
    invoke(app, "project_save", sid=sid)
    saved = doc_of(folder)["tracks"]
    assert saved["Room"]["params"]["weak_signal"] == {"from": 50, "to": None}
    assert saved["Kick Sub"]["params"]["sensitivity"] == p["sensitivity"]   # P4-3: stored by the re-pick at once
    assert saved["Kick Sub"]["params"]["weak_signal"]["from"] == 60


def test_the_inspector_follow_option_derives_the_hits_and_locks_the_lanes(app, make_project, tmp_path):
    sid = open_project(app, kick_and_sub(make_project, tmp_path))
    assert js(app, "() => S.events['Kick Sub'].length") == 0
    ins = inspect_track(app, "Kick Sub")
    follow = ins.locator(".pFollow")
    assert follow.locator("option").all_inner_texts() == ["own detection", "follow Kick In"]
    expect(follow).to_have_value("")
    follow.select_option("Kick In")
    expect(app.locator("#stMsg")).to_have_text("Kick Sub follows Kick In: 3 hits, velocities from its own signal")
    assert js(app, "() => S.tracks['Kick Sub'].follows") == "Kick In"
    sub = js(app, "() => S.events['Kick Sub'].map(e => ({t: e.t, v: e.v}))")
    assert [round(e["t"] * SR) for e in sub] == IDX                      # the same sample positions
    own = invoke(app, "velocities_recompute", sid=sid, name="Kick Sub", events=sub)["events"]
    assert [e["v"] for e in sub] == [e["v"] for e in own]                 # from the sub's own signal (27 dB softer)
    assert all(e["v"] < 100 for e in sub)
    assert role_line(app, "Kick Sub") == "kick · C1 (36) · follows Kick In"
    expect(app.locator("#insRole")).to_have_text("kick · C1 (36) · follows Kick In")
    assert role_line(app, "Kick In") == "kick · C1 (36)"
    expect(ins.locator(".pFollow")).to_have_value("Kick In")
    expect(ins.locator(".pSens")).to_be_disabled()
    expect(ins.locator(".pSensNote")).to_contain_text("follows Kick In: the hits are derived from it")
    expect(app.locator("#insDetected")).to_have_text("3")
    # read-only lanes: a double-click adds nothing, Delete removes nothing, the message explains
    lane = app.locator('.track[data-name="Kick Sub"] canvas.spike')
    lane.dblclick(position={"x": 30, "y": 10})
    expect(app.locator("#stMsg")).to_have_text("Kick Sub follows Kick In: its hits are derived, edit the hits of Kick In")
    assert js(app, "() => S.events['Kick Sub'].length") == 3
    app.evaluate("() => { S.sel = {track: 'Kick Sub', index: 0}; setMsg(''); }")
    app.keyboard.press("Delete")
    expect(app.locator("#stMsg")).to_contain_text("Kick Sub follows Kick In")
    assert js(app, "() => S.events['Kick Sub'].length") == 3
    app.keyboard.press("ArrowRight")                                       # a nudge moves nothing
    assert [round(e["t"] * SR) for e in js(app, "() => S.events['Kick Sub']")] == IDX
    assert invoke(app, "events_set", sid=sid, name="Kick Sub", events=[])["code"] == 409
    # an edit of the source reaches the follower
    app.evaluate("() => { S.sel = {track: 'Kick In', index: 0}; }")
    app.keyboard.press("ArrowRight")
    wait_until(lambda: [round(e["t"] * SR) for e in service_events(app, sid, "Kick Sub")] == [IDX[0] + 1] + IDX[1:],
               msg="the follower in the service")
    wait_until(lambda: [round(e["t"] * SR) for e in js(app, "() => S.events['Kick Sub']")] == [IDX[0] + 1] + IDX[1:],
               msg="the follower in the page")
    # the menus: Own Detection and Follow Hits Of mirror the Inspector; the menu state carries the lists
    app.evaluate("() => { S.selTrack = 'Kick Sub'; updateStatus(); }")
    wait_until(lambda: menu_state(app)["follows"] == "Kick In" and menu_state(app)["tracks"] == ["Kick In", "Kick Sub"]
               and menu_state(app)["selected"] == "Kick Sub", msg="the menu state")
    assert menu(app, "follow_own") is True
    expect(app.locator("#stMsg")).to_have_text("Kick Sub: own detection (re-analyze to detect its hits)")
    assert js(app, "() => S.tracks['Kick Sub'].follows") is None
    assert role_line(app, "Kick Sub") == "kick · C1 (36)"
    assert js(app, "() => S.events['Kick Sub'].length") == 3           # the hits stay until the next analysis
    wait_until(lambda: menu_state(app)["follows"] is None, msg="the menu state after own detection")
    assert menu(app, "follow", name="Kick In") is True
    assert js(app, "() => S.tracks['Kick Sub'].follows") == "Kick In"
    expect(ins.locator(".pFollow")).to_have_value("Kick In")
    # the rules: a source cannot follow its follower
    ins = inspect_track(app, "Kick In")
    ins.locator(".pFollow").select_option("Kick Sub")
    expect(app.locator("#stMsg")).to_have_text("follow failed: Kick In has followers (Kick Sub): set them to their own detection first")
    assert js(app, "() => S.tracks['Kick In'].follows") is None
    expect(ins.locator(".pFollow")).to_have_value("")


def test_the_add_tracks_list_suggests_the_follow_for_a_sub_mic(app, tmp_path, make_project):
    empty_page(app)
    folder = tmp_path / "sub session"                                        # not "drop": test_tracks expects that project name
    folder.mkdir()
    for n in ("k in_02", "k sub_02", "s top_02"):
        x, _ = synth_track([0.5, 1.5, 2.5], 3.0)
        write_wav(folder / f"{n}.wav", x)
    drop(app, folder)
    expect(app.locator("#tracksAdd")).to_have_text("Add 3 tracks")
    wraps = app.evaluate("""() => [...document.querySelectorAll('#tracksRows .urow')].map(r => ({stem: r.dataset.stem,
        shown: !r.querySelector('.ufollowWrap').classList.contains('hidden'), on: r.querySelector('.ufollow').checked,
        text: r.querySelector('.ufollowTxt').textContent}))""")
    assert wraps == [{"stem": "k in_02", "shown": False, "on": True, "text": ""},
                     {"stem": "k sub_02", "shown": True, "on": True, "text": "follow k in_02"},
                     {"stem": "s top_02", "shown": False, "on": True, "text": ""}]
    # the kick in unchecked: no source in the list, the box goes
    app.locator('#tracksRows .urow[data-stem="k in_02"] .uuse').uncheck()
    expect(app.locator('#tracksRows .urow[data-stem="k sub_02"] .ufollowWrap')).to_be_hidden()
    app.locator('#tracksRows .urow[data-stem="k in_02"] .uuse').check()
    expect(app.locator('#tracksRows .urow[data-stem="k sub_02"] .ufollowWrap')).to_be_visible()
    app.click("#tracksAdd")
    loaded(app, 3)
    assert js(app, "() => S.tracks['k_sub_02'].follows") == "k_in_02"
    assert js(app, "() => S.tracks['k_in_02'].follows") is None
    assert role_line(app, "k_sub_02") == "kick · C1 (36) · follows k_in_02"
    expect(app.locator("#stMsg")).to_contain_text("k_sub_02 follows k_in_02")
    sid = js(app, "() => S.sid")
    assert invoke(app, "tracks_get", sid=sid)["tracks"]["k_sub_02"]["follows"] == "k_in_02"
    # into an open project with a kick track: the existing track is the source; unchecked, no follow is set
    sid = open_project(app, make_project())
    folder2 = tmp_path / "sub session 2"
    folder2.mkdir()
    x, _ = synth_track([0.5, 1.5, 2.5], 3.0)
    write_wav(folder2 / "k sub_03.wav", x)
    drop(app, folder2)
    expect(app.locator('#tracksRows .urow .ufollowTxt')).to_have_text("follow Kick In")
    expect(app.locator('#tracksRows .urow .ufollow')).to_be_checked()
    app.locator('#tracksRows .urow .ufollow').uncheck()
    app.click("#tracksAdd")
    loaded(app, 3)
    assert js(app, "() => S.tracks['k_sub_03'].follows") is None
    assert "follows" not in app.locator("#stMsg").inner_text()


def test_open_project_answers_a_real_unsaved_changes_prompt(app, make_project):
    """P4-55: the previous project can be genuinely unsaved (a real edit, no save); guardDirty then
    shows its true #confirmModal inside the open and nothing answers it, so the open waits forever
    (CI 38976450101 attempt 1). open_project asks the service (project_clean_check) and answers the
    explicit "Continue without saving" button (no disk write); the open goes through."""
    sid0 = open_project(app, make_project())
    folder = make_project()
    # a real edit: the open project is now unsaved (the service's document differs from the saved one)
    app.evaluate("async () => { await B.setNote(S.sid, 'Kick In', 40); }")
    assert app.evaluate("async () => (await B.projectCleanCheck(S.sid)).dirty") is True
    # record that the page really showed the "Unsaved changes" prompt inside the open
    app.evaluate("""() => { window.__promptShown = false; const m = document.querySelector('#confirmModal');
        new MutationObserver(() => { if (!m.classList.contains('hidden') &&
            document.querySelector('#confirmTitle').textContent === 'Unsaved changes') window.__promptShown = true;
        }).observe(m, {attributes: true, attributeFilter: ['class']}); }""")
    sid = open_project(app, folder)
    assert sid and sid != sid0
    assert app.evaluate("() => window.__promptShown") is True
    assert app.evaluate("() => [...document.querySelectorAll('.modal:not(.hidden)')].map(m => m.id)") == []


def test_a_follow_is_saved_and_the_exports_carry_the_derived_hits(app, make_project, tmp_path):
    folder = kick_and_sub(make_project, tmp_path, name="Saved")
    sid = open_project(app, folder)
    inspect_track(app, "Kick Sub").locator(".pFollow").select_option("Kick In")
    wait_until(lambda: js(app, "() => S.tracks['Kick Sub'].follows") == "Kick In", msg="the follow")
    invoke(app, "project_save", sid=sid)
    doc = doc_of(folder)
    assert doc["tracks"]["Kick Sub"]["follows"] == "Kick In"
    assert "follows" not in doc["tracks"]["Kick In"]
    assert [round(e["t"] * SR) for e in doc["tracks"]["Kick Sub"]["events"]] == IDX
    # a fresh open reads the follow back
    invoke(app, "project_close", sid=sid, force=True)
    sid = open_project(app, folder)
    assert js(app, "() => S.tracks['Kick Sub'].follows") == "Kick In"
    assert role_line(app, "Kick Sub") == "kick · C1 (36) · follows Kick In"
    expect(inspect_track(app, "Kick Sub").locator(".pFollow")).to_have_value("Kick In")
    # exports: MIDI and spikes of the follower carry the derived hits
    sub = js(app, "() => S.events['Kick Sub'].map(e => ({t: e.t, v: e.v}))")
    track_menu(app, "Kick Sub", "midi")
    expect(app.locator("#stMsg")).to_contain_text("exported 1 MIDI file to Exports/ in the project folder")
    midi = Path(folder, "Exports", "Kick Sub_spike.mid").read_bytes()
    expected = invoke(app, "export_midi", sid=sid, name="Kick Sub", note=36, events=sub, ppq=24000, tempo=120.0)
    assert expected["n_events"] == 3 and len(midi) == expected["size"]
    assert midi == Path(folder, "Exports", "Kick Sub_spike.mid").read_bytes()   # the menu export wrote the derived hits
    assert all(e["v"] < 100 for e in sub) and bytes([sub[0]["v"]]) in midi      # the sub's own velocity is in the file
    track_menu(app, "Kick Sub", "spikes")
    expect(app.locator("#stMsg")).to_contain_text("exported 1 spike file to Exports/ in the project folder")
    with wave.open(str(Path(folder, "Exports", "Kick Sub_spike.wav"))) as w:
        assert (w.getframerate(), w.getnchannels(), w.getsampwidth()) == (SR, 1, 2)
        pcm = w.readframes(w.getnframes())
    nonzero = [i for i in range(len(pcm) // 2) if pcm[2 * i:2 * i + 2] != b"\x00\x00"]
    starts = [i for k, i in enumerate(nonzero) if k == 0 or nonzero[k - 1] != i - 1]
    assert starts == IDX                                                   # one blip at every followed hit
