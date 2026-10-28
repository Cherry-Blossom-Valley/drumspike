"""P3-14a: the Inspector (docs/migration/ux-daw-reference.md 3.4, the left zone of the Cubase model). It shows the
selected track: name (editable), colour, role and note, source file and state (Relink when offline), hit counts, the
detection parameters (the controls of the old params drawer; their round trip is in test_editor.py) and the volume
and pan of P3-16. View > Show Inspector (check item, Option+I) toggles it; the state is an app setting
(<home>/settings.json) that a reload reads back. The lanes shrink by the Inspector's width through the
ResizeObserver of P3-14d: no layout read in a frame."""
import json
from pathlib import Path

from playwright.sync_api import expect

from app_support import inspect_track, invoke, load_project, menu, open_project, wait_until
from test_frames import PROBE, READS, settle
from test_menus import with_hits
from test_mixer import drag as drag_control
from test_relink import moved_project

GEO_CHECK = """() => { const sc = document.querySelector('#lanesScroll'), sr = sc.getBoundingClientRect(), y0 = sc.scrollTop - sr.top - sc.clientTop, bad = [];
  for (const n of S.order) { const w = document.querySelector(`.track[data-name='${n}'] canvas.wave`), g = GEO.rows[n], r = w.getBoundingClientRect();
    if (!g || Math.abs(g.wave - (r.top + y0)) > 0.5) bad.push([n, g && g.wave, r.top + y0]);
    if (w.clientWidth !== GEO.laneW) bad.push([n, 'width', w.clientWidth, GEO.laneW]);
    if (Math.abs(r.left - GEO.laneX) > 0.5) bad.push([n, 'x', r.left, GEO.laneX]); }
  if (GEO.scW !== sc.clientWidth || GEO.scH !== sc.clientHeight) bad.push(['scroll', GEO.scW, sc.clientWidth, GEO.scH, sc.clientHeight]);
  return bad; }"""


def settings_json(page):
    p = page.home / "settings.json"
    return json.loads(p.read_text()) if p.exists() else {}


def js(page, expr, arg=None):
    return page.evaluate(expr, arg) if arg is not None else page.evaluate(expr)


def drag(page, sel, dx):
    """A drag on a control after it is scrolled into view (the Inspector scrolls when it is taller than the window)."""
    page.locator(sel).scroll_into_view_if_needed()
    drag_control(page, sel, dx)


def test_the_inspector_opens_and_closes_by_menu_and_key_and_keeps_its_state(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => setInspector(true)")
    expect(app.locator("#inspector")).to_be_visible()
    wait_until(lambda: settings_json(app).get("inspector") is True, msg="the setting on")
    assert menu(app, "inspector") is True                                     # View > Show Inspector: off
    expect(app.locator("#inspector")).to_be_hidden()
    assert js(app, "() => S.inspector") is False
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.inspector") is False, msg="the check item state")
    wait_until(lambda: settings_json(app).get("inspector") is False, msg="the setting off")
    app.evaluate("() => setMsg('')")
    app.keyboard.press("Alt+i")                                               # Option+I (the menu key runs in the page here)
    expect(app.locator("#inspector")).to_be_visible()
    expect(app.locator("#stMsg")).to_have_text("Inspector shown")
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.inspector") is True, msg="the check item state")
    wait_until(lambda: settings_json(app).get("inspector") is True, msg="the setting on again")
    app.keyboard.press("Alt+i")
    expect(app.locator("#inspector")).to_be_hidden()
    wait_until(lambda: settings_json(app).get("inspector") is False, msg="the setting off again")
    # the other settings keys stay (P3-16 writes audio_output and master_db)
    invoke(app, "transport_master", volume_db=-3.0)
    wait_until(lambda: settings_json(app).get("master_db") == -3.0, msg="the master volume")
    assert settings_json(app)["inspector"] is False
    invoke(app, "transport_master", volume_db=0.0)
    # a reload reads the state back from the app settings
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0")
    expect(app.locator("#inspector")).to_be_hidden()
    assert js(app, "() => S.inspector") is False
    app.evaluate("() => setInspector(true)")
    wait_until(lambda: settings_json(app).get("inspector") is True, msg="the setting on before the reload")
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0")
    expect(app.locator("#inspector")).to_be_visible()
    assert js(app, "() => S.inspector") is True
    # the menu item works without a project too (the zone shows its empty state)
    for s in invoke(app, "project_list"):
        invoke(app, "project_close", sid=s["sid"], force=True)
    app.evaluate("async () => { S.sid = null; setDirtyState(false); await initSessions(); await loadProject(); }")
    app.wait_for_function("() => !S.sid && S.loaded > 0")
    expect(app.locator("#insEmpty")).to_be_visible()
    assert menu(app, "inspector") is True
    expect(app.locator("#inspector")).to_be_hidden()
    app.evaluate("() => setInspector(true)")


def test_the_inspector_shows_the_selected_track_and_an_empty_state(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    with_hits(app, sid, {"Kick In": [1.0, 2.0, 3.0]})
    app.evaluate("() => setInspector(true)")
    expect(app.locator("#insEmpty")).to_be_hidden()                            # P4-39c: the first track is selected at open
    expect(app.locator("#insBody")).to_be_visible()
    body = app.evaluate("() => document.body.innerText")
    assert "No track selected" not in body and "Params" not in body
    ins = inspect_track(app, "Kick In")
    expect(ins.locator("#insEmpty")).to_be_hidden()
    expect(ins.locator("#insName")).to_have_value("Kick In")
    expect(ins.locator("#insRole")).to_have_text("kick · C1 (36)")
    colours = js(app, """() => [getComputedStyle(document.querySelector('#insColor')).backgroundColor,
        getComputedStyle(document.querySelector(".track[data-name='Kick In'] .tstrip")).backgroundColor]""")   # P3-14c: the colour strip
    assert colours[0] == colours[1] and colours[0] != "rgba(0, 0, 0, 0)"
    expect(ins.locator("#insState")).to_have_text("copied")
    expect(ins.locator("#insRelink")).to_be_hidden()
    expect(ins.locator("#insFile")).to_have_text("Kick In.wav")
    src = load_project(folder).doc["tracks"]["Kick In"]["source"]
    expect(ins.locator("#insFile")).to_have_attribute("title", src.get("path") or src.get("relative"))
    expect(ins.locator("#insDetected")).to_have_text("3")
    expect(ins.locator("#insEdited")).to_have_text("0")
    app.evaluate("() => addHitAt('Kick In', 1.5)")                            # an added hit counts as edited
    expect(ins.locator("#insDetected")).to_have_text("3")
    expect(ins.locator("#insEdited")).to_have_text("1")
    menu(app, "undo")
    expect(ins.locator("#insEdited")).to_have_text("0")
    app.keyboard.press("Tab")                                                 # the Tab context stays the track
    assert js(app, "() => [S.selTrack, S.sel.track]") == ["Kick In", "Kick In"]
    expect(ins.locator("#insName")).to_have_value("Kick In")
    ins.locator(".pNoteNum").fill("40")                                       # the note in the Output section: the role line follows
    ins.locator(".pNoteNum").blur()
    expect(ins.locator("#insRole")).to_have_text("kick · E1 (40)")
    expect(app.locator(".track[data-name='Kick In'] .nnote")).to_have_text("E1")
    # the parameters are the track's own: a second track shows its own values
    ins.locator(".pGap").fill("33")
    ins.locator(".pGap").blur()
    inspect_track(app, "Snare Top")
    expect(ins.locator("#insRole")).to_have_text("snare · D1 (38)")
    expect(ins.locator("#insDetected")).to_have_text("0")
    expect(ins.locator(".pGap")).to_have_value("20")
    inspect_track(app, "Kick In")
    expect(ins.locator(".pGap")).to_have_value("33")
    app.locator(".track[data-name='Kick In'] .thead").click(position={"x": 60, "y": 8})   # P4-39c: the same header again keeps the track selected
    assert js(app, "() => S.selTrack") == "Kick In"
    expect(ins.locator("#insEmpty")).to_be_hidden()
    expect(ins.locator("#insBody")).to_be_visible()
    # a new project: its first track is selected (the empty state is only for a project without tracks)
    open_project(app, make_project(tracks=(("Tom 1", "tom", 48),)))
    expect(ins.locator("#insEmpty")).to_be_hidden()
    inspect_track(app, "Tom 1")
    expect(ins.locator("#insRole")).to_have_text("tom · C2 (48)")


def test_an_offline_track_shows_its_state_and_a_relink_button(app, make_project, tmp_path):
    sid, folder, new = moved_project(app, make_project, tmp_path, n=2)
    ins = inspect_track(app, "T01")
    expect(ins.locator("#insState")).to_have_text("offline (missing)")
    assert "offline" in ins.locator("#insState").get_attribute("class")
    expect(ins.locator("#insRelink")).to_be_visible()
    expect(ins.locator("#insFile")).to_have_text("T01.wav")                 # the last known file
    app.evaluate("p => { window.__DS_PICK__ = p; }", str(new / "T01.wav"))
    ins.locator("#insRelink").click()                                         # the Relink button: the file picker, then the relink
    expect(app.locator("#stMsg")).to_contain_text("relinked T01 ->", timeout=15000)
    app.wait_for_function("() => S.loaded > 0 && S.tracks['T01'] && !S.tracks['T01'].offline")
    inspect_track(app, "T01")
    expect(ins.locator("#insState")).to_have_text("referenced")
    expect(ins.locator("#insRelink")).to_be_hidden()
    expect(ins.locator("#insFile")).to_have_attribute("title", str(new / "T01.wav"))
    app.evaluate("() => { window.__DS_PICK__ = null; }")
    assert invoke(app, "tracks_get", sid=sid)["tracks"]["T02"]["offline"] is True   # the other track stays offline


def test_the_track_name_is_edited_in_the_inspector(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    with_hits(app, sid, {"Kick In": [1.0, 2.5]})
    assert (Path(folder) / "Analysis" / "Kick In" / "peaks.bin").is_file()   # the lane cache carries the name
    ins = inspect_track(app, "Kick In")
    app.evaluate("() => { setView(1, 3); S.playhead = 2; }")
    ins.locator("#insName").fill("Kick Inside")
    app.keyboard.press("Enter")
    expect(app.locator("#stMsg")).to_have_text("renamed Kick In to Kick Inside", timeout=15000)
    expect(app.locator(".track[data-name='Kick Inside'] .tname")).to_contain_text("Kick Inside")
    expect(app.locator(".track[data-name='Kick In']")).to_have_count(0)
    t = invoke(app, "tracks_get", sid=sid)
    assert list(t["tracks"]) == ["Kick Inside", "Snare Top"] and t["order"] == ["Kick Inside", "Snare Top"]
    assert t["tracks"]["Kick Inside"]["role"] == "kick" and t["dirty"] is True
    assert (Path(folder) / "Analysis" / "Kick Inside" / "peaks.bin").is_file()
    assert not (Path(folder) / "Analysis" / "Kick In").exists()
    assert js(app, "() => [S.selTrack, S.events['Kick Inside'].length, S.view.t0, S.view.t1, S.playhead]") == ["Kick Inside", 2, 1, 3, 2]
    expect(ins.locator("#insName")).to_have_value("Kick Inside")
    assert "selTrack" in app.locator(".track[data-name='Kick Inside']").get_attribute("class")
    expect(app.locator("#insDetected")).to_have_text("2")
    invoke(app, "project_save", sid=sid)
    doc = load_project(folder).doc
    assert "Kick Inside" in doc["tracks"] and "Kick In" not in doc["tracks"] and doc["order"] == ["Kick Inside", "Snare Top"]
    assert len(doc["tracks"]["Kick Inside"]["events"]) == 2
    # a taken name is refused; the field shows the name again
    ins.locator("#insName").fill("Snare Top")
    app.keyboard.press("Enter")
    expect(app.locator("#stMsg")).to_have_text('a track named "Snare Top" exists')
    expect(ins.locator("#insName")).to_have_value("Kick Inside")
    # Escape cancels the edit
    ins.locator("#insName").fill("Zzz")
    app.keyboard.press("Escape")
    expect(ins.locator("#insName")).to_have_value("Kick Inside")
    app.wait_for_timeout(200)
    assert list(invoke(app, "tracks_get", sid=sid)["tracks"]) == ["Kick Inside", "Snare Top"]
    # a name the service refuses (a path separator): the message and the old name
    ins.locator("#insName").fill("a/b")
    app.keyboard.press("Enter")
    expect(app.locator("#stMsg")).to_contain_text("rename failed: bad track name")
    expect(ins.locator("#insName")).to_have_value("Kick Inside")
    # leaving the field keeps the new name as Enter does; the Tab context follows the name
    ins.locator("#insName").fill("Kick")
    ins.locator("#insName").blur()
    expect(app.locator("#stMsg")).to_have_text("renamed Kick Inside to Kick", timeout=15000)
    app.keyboard.press("Tab")
    assert js(app, "() => S.sel.track") == "Kick"


def test_the_inspector_mirrors_volume_and_pan(app, make_project):
    sid = open_project(app, make_project())
    ins = inspect_track(app, "Kick In")
    assert app.text_content("#inspector [data-for=vol]") == "0.0" and app.text_content("#inspector [data-for=pan]") == "C"   # P3-14c: C, L50, R50
    header = app.locator(".track[data-name='Kick In']")
    drag(app, "#inspector .mslider.vol", -30)                                  # the Inspector's fader
    db = js(app, "() => S.tracks['Kick In'].volume_db")
    assert db < 0
    text = app.text_content("#inspector [data-for=vol]")
    assert text == header.locator("[data-for=vol]").text_content() == f"{db:.1f}"   # the header shows the same value
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["volume_db"] == db, msg="kept in the project")
    drag(app, ".track[data-name='Kick In'] .mslider.pan", 12)                 # the header's pan: the Inspector follows
    pan = js(app, "() => S.tracks['Kick In'].pan")
    assert pan > 0
    assert app.text_content("#inspector [data-for=pan]") == header.locator("[data-for=pan]").text_content() == f"R{round(pan * 100)}"
    app.dblclick("#inspector .mslider.pan")                                   # a double-click resets
    assert js(app, "() => S.tracks['Kick In'].pan") == 0
    assert app.text_content("#inspector [data-for=pan]") == header.locator("[data-for=pan]").text_content() == "C"
    menu(app, "reset_mix")                                                    # Track > Reset Volume and Pan: both views
    assert js(app, "() => S.tracks['Kick In'].volume_db") == 0
    assert app.text_content("#inspector [data-for=vol]") == header.locator("[data-for=vol]").text_content() == "0.0"
    inspect_track(app, "Snare Top")                                           # another track: its own values
    app.evaluate("() => setTrackMix('Snare Top', -6, -0.25)")
    assert app.text_content("#inspector [data-for=vol]") == "-6.0" and app.text_content("#inspector [data-for=pan]") == "L25"
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["Snare Top"]["pan"] == -0.25, msg="the pan in the project")
    app.evaluate("() => setTrackMix('Snare Top', 0, 0)")


def test_the_lanes_shrink_by_the_inspector_width_with_no_layout_read_in_a_frame(app, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Tom 1", "tom", 48))))
    app.evaluate("() => setInspector(false)")
    settle(app)
    assert app.evaluate(GEO_CHECK) == []
    w0, x0 = js(app, "() => [GEO.laneW, GEO.laneX]")
    app.evaluate(PROBE)                                                       # counts the layout reads inside frames from here
    n0 = js(app, "() => FR.n.size")
    app.evaluate("() => setInspector(true)")
    settle(app)
    width = js(app, "() => document.querySelector('#inspector').getBoundingClientRect().width")
    assert width == 320                                                       # P4-25: the default width
    assert js(app, "() => [GEO.laneW, GEO.laneX]") == [w0 - width, x0 + width]
    assert app.evaluate(GEO_CHECK) == []                                       # the lanes and GEO agree
    assert js(app, "() => FR.n.size") > n0                                     # the canvases were sized again
    assert js(app, "() => GLQ.eng ? Math.abs(parseFloat(GLQ.eng.canvas.style.left) - GEO.scL) < 0.5 : true")   # the GL canvas moved with the lanes
    app.evaluate("() => setInspector(false)")
    settle(app)
    assert js(app, "() => [GEO.laneW, GEO.laneX]") == [w0, x0]
    assert app.evaluate(GEO_CHECK) == []
    app.evaluate("() => setInspector(true)")
    settle(app)
    inspect_track(app, "Kick In")                                             # the zone fills; a hit edit; a zoom
    app.evaluate("() => { addHitAt('Kick In', 1.0); setView(0.5, 2.5); }")
    settle(app)
    assert app.evaluate(GEO_CHECK) == []
    reads = app.evaluate(READS)
    assert reads["n"] == 0, reads["where"]


def test_the_close_button_hides_the_inspector_and_keeps_the_setting(app, make_project):
    """Owner report on v0.2.0: the drawer had no visible close. The × in the title bar hides the Inspector; the check
    item (shell_sync) and the app setting follow, and Option+I shows it again."""
    open_project(app, make_project())
    app.evaluate("() => setInspector(true)")
    wait_until(lambda: settings_json(app).get("inspector") is True, msg="the setting on")
    close = app.locator("#insClose")
    expect(close).to_be_visible()
    assert close.get_attribute("title") == "Hide Inspector (Option+I)"
    close.click()
    expect(app.locator("#inspector")).to_be_hidden()
    assert js(app, "() => S.inspector") is False
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.inspector") is False, msg="the check item state")
    wait_until(lambda: settings_json(app).get("inspector") is False, msg="the setting off")
    expect(app.locator("#stMsg")).to_have_text("Inspector hidden")
    app.keyboard.press("Alt+i")
    expect(app.locator("#inspector")).to_be_visible()
    wait_until(lambda: js(app, "() => window.__DS_MENU_STATE__.inspector") is True, msg="the check item state again")
    wait_until(lambda: settings_json(app).get("inspector") is True, msg="the setting on again")


def test_the_playhead_overlay_stays_in_the_lane_area_next_to_the_inspector(app, make_project):
    """Owner report on v0.2.0: the playhead line drew through the params drawer. The overlay lives in #editor and shows
    only inside the lane area: never over the Inspector, the header column or a panel, at any playhead position."""
    open_project(app, make_project(seconds=6.0))
    check = """() => { const o = document.querySelector('#playheadOverlay'), sc = document.querySelector('#lanesScroll'),
      ins = document.querySelector('#inspector'), ed = document.querySelector('#editor');
      if (o.style.display === 'none') return 'hidden';
      const r = o.getBoundingClientRect(), s = sc.getBoundingClientRect(), i = ins.getBoundingClientRect(), e = ed.getBoundingClientRect();
      const lane = document.querySelector('.track .tlanes').getBoundingClientRect(), bad = [];
      if (r.left < s.left + S.theadW - 1 || r.right > s.right + 1) bad.push(['x outside the lane column', r.left, r.right, s.left + S.theadW, s.right]);
      if (r.left < lane.left - 1) bad.push(['over the header column', r.left, lane.left]);
      if (ins.offsetParent !== null && r.left < i.right && r.right > i.left) bad.push(['over the Inspector', r.left, r.right, i.left, i.right]);
      if (r.top < e.top - 1 || r.bottom > e.bottom + 1) bad.push(['y outside the editor', r.top, r.bottom, e.top, e.bottom]);
      return bad.length ? bad : 'ok'; }"""
    for on in (True, False):
        app.evaluate("on => setInspector(on)", on)
        settle(app)
        app.evaluate("() => setView(0, 6)")
        for t in (0, 0.001, 1.5, 3, 5.999, 6):
            app.evaluate("t => { S.playhead = t; invalidate(F.HEAD); }", t)
            settle(app)
            assert app.evaluate(check) == "ok", (on, t, app.evaluate(check))
        app.evaluate("() => { setView(2, 4); S.playhead = 1; invalidate(F.HEAD); }")   # out of the view: not drawn at all
        settle(app)
        assert app.evaluate(check) == "hidden", on
        app.evaluate("() => { S.playhead = 2; invalidate(F.HEAD); }")                  # the left edge of the view: on the lane's first pixel
        settle(app)
        assert app.evaluate(check) == "ok", on
    app.evaluate("() => setInspector(true)")
