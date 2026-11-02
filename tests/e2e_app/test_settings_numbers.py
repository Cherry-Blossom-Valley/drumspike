"""#232: the four settings-as-numbers gaps of the #120 inventory (docs/review/120-settings-inventory.md),
closed: the refine window low/high in the Inspector's Detection section (stored at once, one undo
step, read back with tracks_get), the role's name pattern in the Track section (removed again in
#285: the field confused users; the test now checks it is gone), the layer's dB range as a plain
number (track_layer_set {range}) and
the kept Compensate checkbox of the Export dialog (export_compensate_set, the document field
export_compensate; the dialog shows it again after a reopen). The coordinator's screenshots of the
Inspector and the Export dialog go to the shot dir."""
from playwright.sync_api import expect

from app_support import inspect_track, invoke, open_project, shot_dir, wait_until
from helpers import synth_track, write_wav
from test_editor import build, js

HITS = [0.5, 1.5, 2.5]


def test_the_refine_window_fields_show_store_and_undo(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path))
    ins = inspect_track(app, "Kick In")
    # the stored value (the defaults) shows
    expect(ins.locator(".pRefineLo")).to_have_value("-2")
    expect(ins.locator(".pRefineHi")).to_have_value("12")
    # a typed number reaches the project (tracks_get)
    ins.locator(".pRefineLo").fill("-8")
    ins.locator(".pRefineLo").blur()
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]["refine_lo_ms"] == -8,
               msg="the low window in the service")
    # and Undo restores the default (one step per field)
    app.evaluate("() => undo()")
    wait_until(lambda: js(app, "() => S.tracks['Kick In'].params.refine_lo_ms") == -2, msg="the page's copy")
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]["refine_lo_ms"] == -2,
               msg="the low window undone")
    ins.locator(".pRefineHi").fill("20")
    ins.locator(".pRefineHi").blur()
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]["refine_hi_ms"] == 20,
               msg="the high window in the service")
    app.evaluate("() => undo()")
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]["refine_hi_ms"] == 12,
               msg="the high window undone")
    ins.locator("#insParams").scroll_into_view_if_needed()
    ins.locator("#insParams").screenshot(path=str(shot_dir("232-inspector-refine-window.png")))
    invoke(app, "project_save", sid=sid)


def test_the_inspector_has_no_name_pattern_field_and_a_hand_set_role_stays(app, make_project, tmp_path):
    """#285: the Name pattern field of #232 is gone; the role hint stays, and a role set by hand
    wins (it survives a reload of the track list)."""
    sid = open_project(app, build(make_project, tmp_path))
    ins = inspect_track(app, "Kick In")
    expect(ins.locator("#insRoleMatch")).to_have_count(0)
    expect(ins.locator("#insRoleSel")).to_be_visible()
    app.evaluate("() => setTrackRole('Kick In', 'snare', {source: 'user'})")
    wait_until(lambda: js(app, "() => S.tracks['Kick In'].role_source") == "user", msg="the user role")
    expect(ins.locator("#insRoleSrc")).to_have_text("set by you")
    got = invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]
    assert (got["role"], got["role_source"]) == ("snare", "user")
    assert got.get("role_match") in (None, "")


def test_the_layer_range_number_reaches_the_curve(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path))
    x, _ = synth_track(HITS, 1.0)
    shots = tmp_path / "shots"
    shots.mkdir()
    shot = write_wav(shots / "one.wav", x)
    invoke(app, "track_layer_set", sid=sid, name="Kick In", path=str(shot))
    # the plain number sets the range of the db curve (read back with track_layer_get)
    invoke(app, "track_layer_set", sid=sid, name="Kick In", range=48)
    layer = invoke(app, "track_layer_get", sid=sid, name="Kick In")
    assert layer["layer"]["curve"] == {"kind": "db", "range_db": 48.0}
    # outside 0 to 96 is a 400
    assert invoke(app, "track_layer_set", sid=sid, name="Kick In", range=97)["code"] == 400
    # the Inspector's dB-range field shows the stored value
    inspect_track(app, "Kick In")
    sec = app.locator("#inspector .insSec[data-sec='layer']")
    sec.scroll_into_view_if_needed()
    expect(sec.locator(".lyRange")).to_have_value("48")


def test_the_export_dialog_keeps_the_compensate_choice(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path))
    app.evaluate("() => openExport('all')")
    app.wait_for_function("() => EX.open")
    # no sampler calibration and no kept choice: the dialog's own default, off (test_sampler)
    expect(app.locator("#exComp")).not_to_be_checked()
    assert invoke(app, "export_existing", sid=sid)["export_compensate"] is None
    # a kept "on" beats that default after a reopen
    app.locator("#exComp").check()
    wait_until(lambda: invoke(app, "export_existing", sid=sid)["export_compensate"] is True,
               msg="the kept choice on in the service")
    app.evaluate("() => exClose()")
    app.evaluate("() => openExport('all')")
    app.wait_for_function("() => EX.open")
    expect(app.locator("#exComp")).to_be_checked()
    app.locator("#exComp").uncheck()
    wait_until(lambda: invoke(app, "export_last_get", sid=sid)["export_compensate"] is False,
               msg="the kept choice in the service")
    # the dialog shows the stored value again (close, reopen)
    app.evaluate("() => exClose()")
    app.evaluate("() => openExport('all')")
    app.wait_for_function("() => EX.open")
    expect(app.locator("#exComp")).not_to_be_checked()
    app.locator("#exportModal").screenshot(path=str(shot_dir("232-export-dialog-compensate.png")))
    # the agent command puts it back
    invoke(app, "export_compensate_set", sid=sid, compensate=True)
    wait_until(lambda: invoke(app, "export_last_get", sid=sid)["export_compensate"] is True,
               msg="back to the default")
