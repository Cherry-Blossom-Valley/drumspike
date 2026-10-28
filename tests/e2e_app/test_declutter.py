"""P3-14e: the decluttered main window (owner decision 2026-10-11). The hint lines, the six zoom preset buttons, the
text of the toolbar buttons, the header buttons Params, MIDI, Spikes and the re-analyze arrow, and the status bar
readouts of the view range, the hover, the zoom span and the context are gone. Each action that moved is still
reachable: the toolbar icon, the dropdown, the context menu of the track header, and the native menu."""
from pathlib import Path

from playwright.sync_api import expect

from app_support import inspect_track, menu, open_project, track_menu


def text(page):
    return page.evaluate("() => document.body.innerText")


def test_the_removed_texts_and_elements_are_gone(app, make_project):
    open_project(app, make_project())
    body = text(app)
    body = body.replace("follow Snare Top", "")                                      # P4-39c: the Inspector shows the first track, its Hits select has this option
    for gone in ("overview — drag window", "click ruler: playhead", "Add tracks", "Analyze all", "Export all", "Params",
                 "⬇ MIDI", "⬇ Spikes", "view:", "zoom span:", "context:", "no track context", "snap", "follow"):
        assert gone not in body, gone
    for sel in ("#overviewHint", ".zP", "#stCursor", "#stHover", "#stZoom", "#stCtx", "#followSel",
                ".bParams", ".bExport", ".bSpikes", ".bAnalyze", "#rulerRow .hint", ".params"):   # .params: the drawer (P3-14a)
        expect(app.locator(sel)).to_have_count(0)
    expect(app.locator("#statusbar > span:not(.spacer)")).to_have_count(6)         # selection, detector, message; P3-20b: "half speed"; #121: "changed on disk"; #249: the refused writes
    expect(app.locator("#statusbar > span:not(.spacer):not(#stSpeed):not(#stStale):not(#stEvFail)")).to_have_count(3)
    expect(app.locator("#stSpeed")).to_be_hidden()                                 # it shows only while the transport plays at half speed
    expect(app.locator("#stStale")).to_be_hidden()                                 # it shows only while the project file changed on disk
    expect(app.locator("#stEvFail")).to_be_hidden()                                # it shows only while a hit edit did not reach the project
    for sel in ("#addTracksBtn", "#analyzeAllBtn", "#exportAllBtn", "#snapTgl", "#followBtn"):
        assert len(app.locator(sel).inner_text().strip()) <= 1, sel                # an icon, no words
        assert app.locator(sel).get_attribute("title"), sel                        # the tooltip keeps the information
    assert app.locator(".track").first.locator(".thead button").all_text_contents() == ["M", "S"]
    assert "playhead" in app.locator("#ruler").get_attribute("title")
    assert "drag" in app.locator("#overview").get_attribute("title")
    app.mouse.move(300, 400)                                                         # a hover writes to no removed element
    assert not app.evaluate("() => !!document.querySelector('#stHover')")


def test_the_zoom_dropdown_replaces_the_preset_buttons(app, make_project):
    open_project(app, make_project())
    for value, samples in (("0.005", 240), ("0.5", 24000), ("0.05", 2400)):
        app.select_option("#zoomSel", value)
        assert app.evaluate("() => Math.round(viewDur() * S.tracks[S.order[0]].sr)") == samples
        assert app.locator("#zoomSel").input_value() == ""                           # the box shows "Zoom" again
    app.select_option("#zoomSel", "full")
    assert abs(app.evaluate("() => viewDur() - maxDur()")) < 1e-6
    menu(app, "zoom_5ms")                                                            # the menu uses the same list
    assert app.evaluate("() => Math.round(viewDur() * S.tracks[S.order[0]].sr)") == 240


def test_the_toolbar_icons_run_add_analyze_and_export(app, make_project):
    folder = make_project()
    open_project(app, folder)
    app.click("#analyzeAllBtn")
    expect(app.locator("#progressText")).to_have_text("analysis done", timeout=60000)
    app.click("#exportAllBtn")
    expect(app.locator("#exportModal")).to_be_visible()                              # P4-30: the arrow opens the Export dialog
    app.click("#exGo")
    expect(app.locator("#stMsg")).to_contain_text("exported 2 MIDI and 2 spike files to Exports/", timeout=30000)
    app.click("#exCancel")
    app.click("#addTracksBtn")                                                       # the file dialog of the bridge: cancelled
    expect(app.locator("#stMsg")).not_to_contain_text("failed")


def test_snap_and_follow_icons_follow_the_menu(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => { setSnap(true); setFollow('smooth'); }")
    app.click("#snapChk")
    assert app.evaluate("() => S.snap") is False
    menu(app, "snap")
    assert app.evaluate("() => S.snap") is True
    modes = []
    for _ in range(3):
        app.click("#followBtn")
        modes.append(app.evaluate("() => S.followMode"))
    assert modes == ["page", "off", "smooth"]
    menu(app, "follow_off")
    assert app.locator("#followBtn").get_attribute("data-mode") == "off"
    assert app.evaluate("() => window.__DS_MENU_STATE__.follow") == "off"


def test_the_context_menu_of_the_track_header_runs_the_moved_actions(app, make_project):
    folder = make_project()
    open_project(app, folder)
    app.click("#analyzeAllBtn")
    expect(app.locator("#progressText")).to_have_text("analysis done", timeout=60000)
    app.locator('.track[data-name="Kick In"] .thead').click(button="right")
    assert app.locator("#ctxMenu button").all_text_contents() == ["Export…", "Re-analyze", "Reset Volume and Pan"]   # P3-16; P3-14a: Params is in the Inspector
    app.keyboard.press("Escape")
    expect(app.locator("#ctxMenu")).to_have_count(0)
    expect(app.locator(".params")).to_have_count(0)                                  # the drawer is gone (P3-14a)
    expect(inspect_track(app, "Kick In").locator("#insParams .pSens")).to_be_visible()   # the parameters are in the Inspector
    track_menu(app, "Kick In", "midi")
    expect(app.locator("#stMsg")).to_contain_text("exported 1 MIDI file to Exports/")
    assert Path(folder, "Exports", "Kick In_spike.mid").is_file()
    track_menu(app, "Snare Top", "spikes")
    expect(app.locator("#stMsg")).to_contain_text("exported 1 spike file to Exports/")
    assert Path(folder, "Exports", "Snare Top_spike.wav").is_file()
    app.evaluate("() => { document.querySelector('#progressText').textContent = ''; }")
    track_menu(app, "Kick In", "reanalyze")
    expect(app.locator("#progressText")).to_have_text("analysis done", timeout=60000)
    app.mouse.click(5, 5)                                                            # a click away closes the menu
    app.locator('.track[data-name="Kick In"] .thead').click(button="right")
    app.mouse.click(5, 5)
    expect(app.locator("#ctxMenu")).to_have_count(0)
    menu(app, "export_selected")                                                     # the Track menu has Export Selected Tracks... (P4-30)
    expect(app.locator("#exportModal")).to_be_visible()
    app.click("#exCancel")
