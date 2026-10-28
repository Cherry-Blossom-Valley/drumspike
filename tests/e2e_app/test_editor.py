"""P3-12: the editor flows of the retired server e2e suite (tests/e2e: test_ui.py, test_ui_controls.py,
test_ui_project.py, test_app_mode.py), on the app through the bridge. Hits, nudges, undo and redo, re-analysis
with kept edits, zoom and pan, the overview, the parameters in the Inspector (P3-14a; before that a params drawer), the hitpoint shift, track drag, the settings,
help, the dirty dot, the close prompt and a pending event write across a project switch. The page runs the IPC
backend of the app (B.app); the events reach the Rust service (events_get answers what the page sent)."""
import json
import re
from pathlib import Path

import pytest
from playwright.sync_api import expect

from app_support import edits_done, inspect_track, invoke, menu, open_project, wait_until
from helpers import SR, synth_track, write_wav

ONSETS = [0.5, 1.5, 2.5]                    # the synthetic kicks of the make_project fixture
IDX = [int(round(t * SR)) for t in ONSETS]


def exact_events():
    return [{"t": i / SR, "v": 100} for i in IDX]


def span_samples(page):
    """The span of the view in samples of the first track (the status bar showed it before P3-14e)."""
    return page.evaluate("() => Math.round(viewDur() * S.tracks[S.order[0]].sr)")


def build(make_project, tmp_path, tracks=(("Kick In", "kick", 36),), events=None, name="Song", seconds=4.0):
    """A copied project whose tracks have the given events (by track name) in the document."""
    src = tmp_path / f"src_{name}"
    src.mkdir()
    specs = []
    for tname, role, note in tracks:
        x, _ = synth_track(ONSETS, seconds)
        wav = write_wav(src / f"{tname}.wav", x)
        specs.append((tname, wav, role, note, (events or {}).get(tname)))
    return Path(make_project.builder.build_project(tmp_path / f"projects_{name}", name, specs, "copied"))


def service_events(page, sid, track="Kick In"):
    return invoke(page, "events_get", sid=sid, name=track)["events"]


def service_samples(page, sid, track="Kick In"):
    return [round(e["t"] * SR) for e in service_events(page, sid, track)]


def js(page, expr, arg=None):
    return page.evaluate(expr, arg)


def span(page):
    return js(page, "() => S.view.t1 - S.view.t0")


def doc_of(folder):
    return json.loads(next(Path(folder).glob("*.drumspike")).read_text())


def expect_counts(page, text, name="Kick In", timeout=5000):
    """The hit counts of the track as the header showed them before P3-14c ("3 events · 1 edited"): the header is
    compact now, the Inspector shows Detected (the hits that are not edited) and Edited for the selected track."""
    m = re.fullmatch(r"(\d+) events(?: · (\d+) edited)?", text)
    total, edited = int(m.group(1)), int(m.group(2) or 0)
    inspect_track(page, name)
    expect(page.locator("#insDetected")).to_have_text(str(total - edited), timeout=timeout)
    expect(page.locator("#insEdited")).to_have_text(str(edited), timeout=timeout)


# ---------- hits: select, nudge, undo, add, delete (test_ui.py) ----------

def test_tab_selects_hits_and_arrow_nudges_one_sample_with_undo(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    app.keyboard.press("Tab")
    expect(app.locator("#stSel")).to_contain_text("1/3")
    app.keyboard.press("Tab")
    expect(app.locator("#stSel")).to_contain_text("2/3")
    app.keyboard.press("ArrowRight")
    expect(app.locator("#stSel")).to_have_text("Kick In: hit 2/3")                 # P3-14b: the status bar is short
    expect(app.locator("#ilSample")).to_have_value(str(IDX[1] + 1))                # the info line has the sample
    wait_until(lambda: service_samples(app, sid)[1] == IDX[1] + 1, msg="the nudge reaches the service")
    assert service_events(app, sid)[1]["orig"] == pytest.approx(IDX[1] / SR)      # marked as edited
    expect_counts(app, "3 events · 1 edited")
    app.keyboard.press("Control+z")
    wait_until(lambda: service_samples(app, sid)[1] == IDX[1], msg="the undo reaches the service")
    expect_counts(app, "3 events")
    app.keyboard.press("Shift+Tab")
    expect(app.locator("#stSel")).to_contain_text("1/3")


def test_double_click_adds_a_hit_on_a_sample_and_delete_removes_it(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    spike = app.locator(".track .spike")
    box = spike.bounding_box()
    spike.dblclick(position={"x": box["width"] * 0.3, "y": box["height"] / 2})
    expect_counts(app, "4 events · 1 edited")
    wait_until(lambda: len(service_events(app, sid)) == 4, msg="the add reaches the service")
    added = [e for e in service_events(app, sid) if e.get("added")]
    assert len(added) == 1 and abs(added[0]["t"] * SR - round(added[0]["t"] * SR)) < 1e-6   # snapped to a sample (round(t * sr) / sr * sr is not always an exact float)
    edits_done(app)                                                                          # P4-9 stage 2: the added hit is selected when the stored hits are shown
    app.keyboard.press("Delete")
    expect_counts(app, "3 events")
    wait_until(lambda: len(service_events(app, sid)) == 3, msg="the delete reaches the service")


def test_redo_of_an_added_hit_keeps_it_marked_as_edited(app, make_project, tmp_path):   # closed by P4-9 stage 2: the service restores the markers
    open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    spike = app.locator(".track .spike")
    box = spike.bounding_box()
    spike.dblclick(position={"x": box["width"] * 0.3, "y": box["height"] / 2})
    app.keyboard.press("Control+z")
    app.keyboard.press("Control+y")
    expect_counts(app, "4 events · 1 edited", timeout=2000)


def test_edits_survive_a_page_reload(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    app.keyboard.press("Tab")
    app.keyboard.press("Shift+ArrowRight")                                       # +10 samples
    wait_until(lambda: service_samples(app, sid)[0] == IDX[0] + 10, msg="the edit reaches the service")
    app.evaluate("sid => localStorage.setItem('dsg_sid', sid)", sid)
    app.reload()
    app.wait_for_function("sid => typeof S !== 'undefined' && S.sid === sid && (S.events['Kick In'] || []).length === 3", arg=sid)
    assert round(js(app, "() => S.events['Kick In'][0].t") * SR) == IDX[0] + 10
    expect_counts(app, "3 events · 1 edited")


# ---------- re-analysis keeps or drops the hand edits ----------

def test_reanalysis_can_keep_hand_edited_hits(app, make_project, tmp_path):
    evs = [{"t": (i + 500) / SR, "v": 100} for i in IDX] + [{"t": 3.6, "v": 64, "added": True}]
    sid = open_project(app, build(make_project, tmp_path, events={"Kick In": evs}))
    app.click("#analyzeAllBtn")
    expect(app.locator("#reanalyzeModal")).to_be_visible()
    expect(app.locator("#reanalyzeMsg")).to_contain_text("Kick In (1 edited)")
    app.click("#raKeep")
    app.wait_for_function("idx => !analysisRunning && S.events['Kick In'].length === 4 && S.events['Kick In']"
                          ".filter(e => !e.added).every(e => Math.min(...idx.map(i => Math.abs(e.t * 48000 - i))) <= 24)",
                          arg=IDX, timeout=60000)
    expect_counts(app, "4 events · 1 edited")

    def merged():
        ev = service_events(app, sid)
        auto = [round(e["t"] * SR) for e in ev if not e.get("added")]
        return (len(ev) == 4 and any(e.get("added") and e["t"] == 3.6 for e in ev)
                and all(abs(a - i) <= 24 for a, i in zip(auto, IDX)))
    wait_until(merged, msg="the merged events reach the service")


def test_reanalysis_discard_drops_hand_edited_hits(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path, events={"Kick In": [{"t": 3.6, "v": 64, "added": True}]}))
    app.click("#analyzeAllBtn")
    app.click("#raDiscard")
    expect_counts(app, "3 events", timeout=60000)
    wait_until(lambda: not any(e.get("added") for e in service_events(app, sid)) and len(service_events(app, sid)) == 3,
               msg="the discarded hit is gone in the service")


# ---------- R04 (#237, ticket #250): velocity-only and restored edits get the Keep/Discard choice ----------

def test_page_is_kept_matches_the_shared_predicate_table(app, make_project, tmp_path):
    """isKept (the page copy of edit::is_kept) and the Rust predicate read one table
    (tests/fixtures/agent-parity/is_kept.json); the Rust side runs it in crates/app/src/edit/tests.rs."""
    open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    fx = json.loads((Path(__file__).parents[1] / "fixtures" / "agent-parity" / "is_kept.json").read_text())
    for case in fx["cases"]:
        assert js(app, "hit => isKept(hit)", case["hit"]) == case["kept"], case["name"]


def velocity_only_events():
    evs = exact_events()
    evs[0] = {"t": IDX[0] / SR, "v": 64, "edited": True}   # a hand velocity, no timing edit
    return evs


def restored_only_events():
    return exact_events() + [{"t": 3.7, "v": 80, "edited": True}]   # restored hit where the detector finds nothing


@pytest.mark.parametrize("events,edited_n", [(velocity_only_events(), 1), (restored_only_events(), 1),
                                             (exact_events() + [{"t": 3.6, "v": 64, "added": True}], 1)])
def test_reanalysis_offers_keep_for_velocity_only_restored_and_added_edits(app, make_project, tmp_path,
                                                                          events, edited_n):
    open_project(app, build(make_project, tmp_path, events={"Kick In": events}))
    app.click("#analyzeAllBtn")
    expect(app.locator("#reanalyzeModal")).to_be_visible()
    expect(app.locator("#reanalyzeMsg")).to_contain_text(f"Kick In ({edited_n} edited)")


def wait_analysis_done(app, sid, n_events, msg):
    """The modal click started an analysis: wait until it ran and ended. `len(service_events) == n`
    is true before the analysis too (the fixture has the same count), so the invoke record and
    `!analysisRunning` are the real wait (ticket #265)."""
    wait_until(lambda: "analyze_start" in app.invokes, timeout=60000, msg="the click started the analysis")
    app.wait_for_function("() => !analysisRunning", timeout=60000)
    wait_until(lambda: len(service_events(app, sid)) == n_events, msg=msg)
    assert invoke(app, "analyze_status", sid=sid)["state"] == "done"   # the analysis ran, not a no-op


def test_reanalysis_keep_preserves_a_velocity_only_edit(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path, events={"Kick In": velocity_only_events()}))
    app.click("#analyzeAllBtn")
    expect(app.locator("#reanalyzeModal")).to_be_visible()
    app.click("#raKeep")
    wait_analysis_done(app, sid, 3, "the analysis is done")
    hits = [e for e in service_events(app, sid) if round(e["t"] * SR) == IDX[0]]
    assert len(hits) == 1 and hits[0]["v"] == 64 and hits[0].get("edited") is True   # the Inspector counts timing edits only (insCounts)


def test_reanalysis_discard_replaces_a_velocity_only_edit(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path, events={"Kick In": velocity_only_events()}))
    app.click("#analyzeAllBtn")
    expect(app.locator("#reanalyzeModal")).to_be_visible()
    app.click("#raDiscard")
    wait_analysis_done(app, sid, 3, "the analysis is done")
    hits = [e for e in service_events(app, sid) if abs(e["t"] * SR - IDX[0]) <= 24]   # the detector's time is within a few samples
    assert len(hits) == 1 and hits[0]["v"] != 64 and not hits[0].get("edited")


def test_reanalysis_keep_preserves_a_restored_hit(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path, events={"Kick In": restored_only_events()}))
    app.click("#analyzeAllBtn")
    expect(app.locator("#reanalyzeModal")).to_be_visible()
    app.click("#raKeep")
    wait_analysis_done(app, sid, 4, "the analysis is done")
    hits = [e for e in service_events(app, sid) if e["t"] == 3.7]
    assert len(hits) == 1 and hits[0]["v"] == 80 and hits[0].get("edited") is True


def test_reanalysis_discard_drops_a_restored_hit(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path, events={"Kick In": restored_only_events()}))
    app.click("#analyzeAllBtn")
    expect(app.locator("#reanalyzeModal")).to_be_visible()
    app.click("#raDiscard")
    wait_analysis_done(app, sid, 3, "the analysis is done")
    assert not [e for e in service_events(app, sid) if abs(e["t"] - 3.7) < 0.02]


# ---------- zoom, pan, overview (test_ui.py, test_ui_controls.py) ----------

def test_zoom_presets_by_keyboard(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    app.keyboard.press("6")                          # playhead at 0: the span must not be cut in half
    assert span_samples(app) == 240
    app.keyboard.press("Tab")                        # playhead to the first hit
    app.keyboard.press("6")
    assert span_samples(app) == 240
    app.keyboard.press("4")
    assert span_samples(app) == 24000
    app.keyboard.press("1")
    assert abs(app.evaluate("() => viewDur()") - 10.0) < 1e-6


def test_keyboard_zoom_and_pan_keep_the_span(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    app.keyboard.press("Tab")                                         # playhead to the first hit
    app.keyboard.press("Escape")                                      # keep the playhead, drop the selection
    app.keyboard.press("3")                                           # 5 s
    assert span(app) == pytest.approx(5.0)
    app.keyboard.press("t")
    assert span(app) == pytest.approx(2.5)
    app.keyboard.press("r")
    app.keyboard.press("-")
    assert span(app) == pytest.approx(10.0)
    app.keyboard.press("+")
    t0 = js(app, "() => S.view.t0")
    app.keyboard.press("ArrowRight")                                  # no selection: pan 20 %
    assert js(app, "() => S.view.t0") == pytest.approx(t0 + 1.0) and span(app) == pytest.approx(5.0)
    app.keyboard.press("ArrowLeft")
    app.keyboard.press("ArrowLeft")                                   # clamped at 0, span kept
    assert js(app, "() => S.view.t0") == 0 and span(app) == pytest.approx(5.0)
    app.locator(".track .wave").hover()
    app.mouse.wheel(300, 0)                                           # a sideways swipe pans
    assert js(app, "() => S.view.t0") > 0 and span(app) == pytest.approx(5.0)
    before = js(app, "() => document.querySelector('#lanesScroll').scrollTop")
    app.mouse.wheel(0, 100)                                           # the vertical wheel scrolls the tracks
    assert js(app, "() => S.view.t0") > 0 and before == 0


def test_cmd_wheel_and_pinch_zoom_around_the_pointer(app, make_project, tmp_path):
    """P4 #145: Cmd+wheel and the pinch (a ctrl+wheel) keep the time under the pointer within 1 px; the zoom keys keep
    the playhead anchor."""
    open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    js(app, "() => setView(1.0, 3.0)")
    box = app.locator(".track .wave").bounding_box()
    x = js(app, "() => Math.round(GEO.laneX + 0.7 * laneWidth())")      # wheel events carry whole pixels
    y = round(box["y"] + box["height"] / 2)
    anchor = js(app, f"() => xToT({x} - GEO.laneX, laneWidth())")
    assert anchor == pytest.approx(2.4, abs=0.005)                     # 70 % of the lane width
    drift = f"() => Math.abs(tToX({anchor}, laneWidth()) + GEO.laneX - {x})"
    app.mouse.move(x, y)
    app.keyboard.down("Meta")
    for _ in range(5):                                                  # Cmd+wheel up zooms in
        before = span(app)
        app.mouse.wheel(0, -100)
        assert span(app) < before and js(app, drift) <= 1.0
    assert span(app) < 0.25
    for _ in range(5):                                                  # Cmd+wheel down zooms out
        before = span(app)
        app.mouse.wheel(0, 100)
        assert span(app) > before and js(app, drift) <= 1.0
    app.keyboard.up("Meta")
    assert span(app) == pytest.approx(2.0) and js(app, "() => S.view.t0") == pytest.approx(1.0)
    pinch = f"""d => {{ const ev = new WheelEvent('wheel', {{ctrlKey: true, deltaY: d, clientX: {x}, clientY: {y},
                bubbles: true, cancelable: true}}); document.querySelector('.track .wave').dispatchEvent(ev);
                return ev.defaultPrevented; }}"""
    for d in (-100, -10, 100, 10):                                     # a pinch acts as Cmd+wheel: in, then out
        before = span(app)
        assert js(app, pinch, d) is True
        assert (span(app) < before if d < 0 else span(app) > before) and js(app, drift) <= 1.0
    assert span(app) == pytest.approx(2.0)
    # the zoom keys have no pointer: they keep the playhead anchor of today
    js(app, "() => { setView(1.0, 3.0); S.playhead = 1.5; }")
    app.keyboard.press("t")
    assert span(app) == pytest.approx(1.0) and js(app, "() => S.view.t0") == pytest.approx(1.0)
    # a plain vertical wheel still scrolls the tracks and does not zoom
    app.mouse.wheel(0, 100)
    assert span(app) == pytest.approx(1.0)


def test_click_on_a_lane_moves_the_playhead_and_a_drag_pans(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path))
    app.keyboard.press("3")
    wave = app.locator(".track .wave").bounding_box()
    app.mouse.click(wave["x"] + wave["width"] / 2, wave["y"] + 20)
    ph = js(app, "() => S.playhead")
    assert 2.0 < ph < 3.0 and float(ph * SR).is_integer()                # snapped to a sample
    t0 = js(app, "() => S.view.t0")
    # P4-6a: a drag in the wave lane selects a range (tests/e2e_app/test_learn.py); it no longer pans
    app.mouse.move(wave["x"] + wave["width"] / 2, wave["y"] + 20)
    app.mouse.down()
    app.mouse.move(wave["x"] + wave["width"] / 4, wave["y"] + 20, steps=5)
    app.mouse.up()
    assert js(app, "() => S.view.t0") == t0 and js(app, "() => DSLearn.state().sel") is not None
    assert js(app, "() => S.playhead") == ph                            # a drag is not a click
    app.keyboard.press("Escape")
    # a drag in an empty part of the spike lane still pans
    spike = app.locator(".track .spike").bounding_box()
    x = js(app, """() => { const W = laneWidth(), evs = S.events[S.order[0]];
      for (const f of [0.3, 0.5, 0.7, 0.2, 0.8]) { const t = xToT(f * W, W); if (evs.every(e => Math.abs(e.t - t) > 0.15)) return GEO.laneX + f * W; } }""")
    app.mouse.move(x, spike["y"] + 20)
    app.mouse.down()
    app.mouse.move(x - 150, spike["y"] + 20, steps=5)
    app.mouse.up()
    assert js(app, "() => S.view.t0") > t0                              # dragged left = later in time
    assert js(app, "() => S.playhead") == ph                            # a drag is not a click


def test_overview_click_jumps_and_drag_moves_the_view(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path))
    app.keyboard.press("3")                                             # 5 s window of a 10 s overview
    ov = app.locator("#overview").bounding_box()
    y = ov["y"] + ov["height"] / 2
    app.mouse.click(ov["x"] + ov["width"] * 0.9, y)
    t0, t1 = js(app, "() => [S.view.t0, S.view.t1]")
    assert t1 == pytest.approx(10) and t1 - t0 == pytest.approx(5)      # jumped right, span kept
    app.mouse.move(ov["x"] + ov["width"] * 0.75, y)
    app.mouse.down()
    app.mouse.move(ov["x"] + ov["width"] * 0.55, y, steps=5)
    app.mouse.up()
    assert js(app, "() => S.view.t0") < t0 and span(app) == pytest.approx(5)
    t0 = js(app, "() => S.view.t0")
    app.mouse.move(ov["x"] + ov["width"] * 0.3, y)
    app.mouse.wheel(40, 0)                                              # a sideways wheel on the overview pans
    assert js(app, "() => S.view.t0") > t0 and span(app) == pytest.approx(5)


def test_a_zoomed_view_past_the_end_of_a_short_track_draws_without_errors(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path, seconds=2.0))      # 2 s tracks on an at least 10 s timeline
    app.keyboard.press("4")                                             # 500 ms, sample-level drawing
    ov = app.locator("#overview").bounding_box()
    app.mouse.click(ov["x"] + ov["width"] * 0.8, ov["y"] + ov["height"] / 2)   # about 8 s
    app.keyboard.press("ArrowRight")
    app.evaluate("() => new Promise(r => setTimeout(r, 300))")
    assert js(app, "() => S.view.t0") > 2.0


# ---------- the parameters in the Inspector (P3-14a; before: the params drawer) and the hitpoint shift (test_ui_controls.py) ----------

def test_params_in_the_inspector_set_note_and_analysis_params(app, make_project, tmp_path):
    """The same round trip the drawer test had: note by name and number, gap, refine, sensitivity, re-analyze, save."""
    folder = build(make_project, tmp_path)
    sid = open_project(app, folder)
    row = app.locator(".track[data-name='Kick In']")
    ins = inspect_track(app, "Kick In")
    ins.locator(".pNote").fill("D1")
    ins.locator(".pNote").blur()
    expect(row.locator(".nnote")).to_have_text("D1")
    expect(ins.locator("#insRole")).to_have_text("kick · D1 (38)")                # the Inspector's role line follows
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["note"] == 38, msg="the note in the service")
    ins.locator(".pNote").fill("H9")                                    # an invalid name is reverted
    ins.locator(".pNote").blur()
    expect(ins.locator(".pNote")).to_have_value("D1")
    ins.locator(".pNoteNum").fill("200")
    ins.locator(".pNoteNum").blur()
    expect(ins.locator(".pNote")).to_have_value("G8")                   # clamped to 127
    ins.locator(".pGap").fill("35")
    ins.locator(".pGap").blur()
    ins.locator(".pRefine").uncheck()
    ins.locator(".pSens").fill("100")
    params = js(app, "() => S.tracks['Kick In'].params")
    assert params["min_gap_ms"] == 35 and params["refine"] is False
    assert max(params["thresholds"]) < 0.32                             # higher sensitivity, lower thresholds
    ins.locator(".bReanalyze").click()
    expect(ins.locator("#insDetected")).to_have_text("3", timeout=60000)   # P3-14c: the header shows no counts; the Inspector does
    invoke(app, "project_save", sid=sid)
    saved = doc_of(folder)["tracks"]["Kick In"]["params"]
    assert saved["min_gap_ms"] == 35 and saved["refine"] is False


def test_hitpoint_shift_moves_every_hit_by_whole_samples(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    ins = inspect_track(app, "Kick In")
    ins.locator(".pShift").fill("-12")
    ins.locator(".pShift").blur()
    wait_until(lambda: service_samples(app, sid) == [i - 12 for i in IDX], msg="the shift in the service")
    ins.locator(".pShift").fill("8")
    ins.locator(".pShift").blur()
    wait_until(lambda: service_samples(app, sid) == [i + 8 for i in IDX], msg="the second shift as a delta")


# P4-9: closed. The shift is stored by track_shift_set, so a reload shows it again (it was an open case of the retired server).
def test_hitpoint_shift_setting_survives_a_reload(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path, events={"Kick In": exact_events()}))
    ins = inspect_track(app, "Kick In")
    ins.locator(".pShift").fill("-12")
    ins.locator(".pShift").blur()
    wait_until(lambda: service_samples(app, sid)[0] == IDX[0] - 12, msg="the shift in the service")
    app.evaluate("sid => localStorage.setItem('dsg_sid', sid)", sid)
    app.reload()
    app.wait_for_function("sid => typeof S !== 'undefined' && S.sid === sid && S.order.length === 1", arg=sid)
    assert js(app, "() => S.tracks['Kick In'].params.shift_samples || 0") == -12


# ---------- track order, name, notes, settings, help (test_ui.py, test_ui_controls.py) ----------

def test_track_order_drag_reaches_the_document(app, make_project, tmp_path):
    folder = build(make_project, tmp_path, tracks=(("a", "kick", 36), ("b", "snare", 38)))
    sid = open_project(app, folder)
    first = app.locator(".track .thead").nth(0).bounding_box()
    second = app.locator(".track").nth(1).bounding_box()
    app.mouse.move(first["x"] + 40, first["y"] + 10)
    app.mouse.down()
    app.mouse.move(first["x"] + 40, first["y"] + 30, steps=3)
    app.mouse.move(first["x"] + 40, second["y"] + second["height"] - 5, steps=10)
    app.mouse.up()
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["order"] == ["b", "a"], msg="the order in the service")
    assert js(app, "() => S.order") == ["b", "a"]


def test_the_project_name_is_rendered_as_text_not_html(app, make_project, tmp_path):
    evil = '<img src=x onerror="window.__xss=1">'
    open_project(app, build(make_project, tmp_path))
    menu(app, "rename")                                                 # P3-13a: File > Rename Project
    field = app.locator("#renameField")
    field.fill(evil)
    field.press("Enter")
    wait_until(lambda: any(s["name"] == evil for s in app.evaluate("() => B.sessions()")), msg="the new name in the service")
    assert app.evaluate("() => S.name") == evil
    assert evil in js(app, "() => document.title")
    assert js(app, "() => window.__xss") is None


def test_note_names_are_the_servers(app):
    """nameFromNote equals engine.name_from_note of the retired server (frozen list) and noteFromName inverts it."""
    import frozen
    want = frozen.store("note_names").ref("0..127", {"notes": 128}, canonical_host=True)   # recorded on arm64-mac only
    names = js(app, "() => Array.from({length: 128}, (_, n) => nameFromNote(n))")
    assert names == want
    assert js(app, "ns => ns.map(noteFromName)", names) == list(range(128))


def test_settings_turn_the_metronome_click_off_and_persist(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path))
    menu(app, "settings")                                               # P3-13a: Drum Spike > Settings (Cmd+,)
    expect(app.locator("#settingsModal")).to_be_visible()
    app.locator("#setClickOn").uncheck()
    assert js(app, "() => $('#clickBtn').classList.contains('on')") is False      # the toolbar button follows
    assert js(app, "() => window.__DS_MENU_STATE__.click") is False                # and the menu's check item
    app.locator("#setClickVol").fill("25")
    pitch = app.locator("#setPitches input").first
    pitch.fill("20000")
    pitch.blur()                                                        # clamped to 8000 Hz
    saved = js(app, "() => JSON.parse(localStorage.getItem('dsg_settings')).click")
    assert saved["enabled"] is False and saved["volume"] == 0.25
    assert 8000 in saved["pitches"].values()
    app.click("#settingsDone")
    expect(app.locator("#settingsModal")).to_be_hidden()
    app.evaluate("sid => localStorage.setItem('dsg_sid', sid)", sid)
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.order.length === 1")
    assert js(app, "() => SETTINGS().click.enabled") is False
    assert js(app, "() => SETTINGS().click.volume") == 0.25
    app.evaluate("() => localStorage.removeItem('dsg_settings')")


def test_help_modal_opens_and_closes(app):
    menu(app, "shortcuts")                                              # P3-13a: Help > Keyboard Shortcuts
    expect(app.locator("#helpModal")).to_be_visible()
    expect(app.locator("#helpModal")).to_contain_text("Tab")
    expect(app.locator("#helpModal")).to_contain_text("Go to Project Start")
    app.locator("#helpDone").click()
    expect(app.locator("#helpModal")).to_be_hidden()
    menu(app, "shortcuts")
    app.keyboard.press("Escape")                                        # Escape closes it as well
    expect(app.locator("#helpModal")).to_be_hidden()


# ---------- dirty dot, close prompt, pending writes (test_ui_project.py) ----------

def test_edit_shows_the_dirty_dot_in_the_title_and_the_list_and_save_clears_it(app, make_project, tmp_path):
    folder = build(make_project, tmp_path, events={"Kick In": exact_events()})
    sid = open_project(app, folder)
    expect(app.locator("#dirtyDot")).to_be_hidden()
    assert js(app, "() => document.title") == js(app, "() => S.name") + " - Drum Spike"
    app.keyboard.press("Tab")
    app.keyboard.press("ArrowRight")                                    # +1 sample, still in the 400 ms debounce
    expect(app.locator("#dirtyDot")).to_be_visible()
    assert js(app, "() => document.title.startsWith('\\u25cf ')")
    assert js(app, "() => { const ev = new Event('beforeunload', {cancelable: true}); window.dispatchEvent(ev); return ev.defaultPrevented; }")
    app.keyboard.press("Control+s")
    expect(app.locator("#dirtyDot")).to_be_hidden()
    wait_until(lambda: invoke(app, "project_get", sid=sid)["dirty"] is False, msg="saved")
    doc = doc_of(folder)
    assert round(doc["tracks"]["Kick In"]["events"][0]["t"] * SR) == IDX[0] + 1
    assert doc["tracks"]["Kick In"]["events"][0]["orig"] == pytest.approx(IDX[0] / SR)
    # File > Save does the same; the project list marks a dirty project too
    app.keyboard.press("Delete")
    expect(app.locator("#dirtyDot")).to_be_visible()
    wait_until(lambda: invoke(app, "project_get", sid=sid)["dirty"] is True, msg="the event write")
    assert [s["dirty"] for s in app.evaluate("() => B.sessions()") if s["sid"] == sid] == [True]
    menu(app, "save")
    expect(app.locator("#stMsg")).to_have_text("project saved")
    expect(app.locator("#dirtyDot")).to_be_hidden()


def test_close_prompt_while_dirty_cancel_continue_save_and_discard(app, make_project, tmp_path):
    dirty_folder = build(make_project, tmp_path, events={"Kick In": exact_events()}, name="Dirty")
    other_folder = build(make_project, tmp_path, name="Other")
    other = open_project(app, other_folder)
    sid = open_project(app, dirty_folder)
    app.keyboard.press("Tab")
    app.keyboard.press("Shift+ArrowRight")                              # +10 samples
    expect(app.locator("#dirtyDot")).to_be_visible()
    menu(app, "close", wait=False)
    expect(app.locator("#confirmModal")).to_be_visible()
    expect(app.locator("#confirmMsg")).to_contain_text('"Dirty" has unsaved changes')
    app.click("#confirmModal button[data-act='cancel']")
    expect(app.locator("#confirmModal")).to_be_hidden()
    expect(app.locator("#stMsg")).to_have_text("close cancelled")
    assert js(app, "() => S.sid") == sid
    wait_until(lambda: invoke(app, "project_get", sid=sid)["dirty"] is True, msg="the event write")
    # switching to another project (Window menu) asks too; "continue" keeps the edits in memory
    menu(app, "project", wait=False, sid=other)
    expect(app.locator("#confirmModal")).to_be_visible()
    app.click("#confirmModal button[data-act='skip']")
    app.wait_for_function("o => S.sid === o", arg=other)
    expect(app.locator("#dirtyDot")).to_be_hidden()
    assert invoke(app, "project_get", sid=sid)["dirty"] is True
    # back to the dirty project (Window menu; the clean one asks nothing), File > Close: Save and close
    # writes the edit, then closes
    menu(app, "project", sid=sid)
    app.wait_for_function("s => S.sid === s", arg=sid)
    menu(app, "close", wait=False)
    expect(app.locator("#confirmModal")).to_be_visible()
    app.click("#confirmModal button[data-act='save']")
    wait_until(lambda: sid not in {s["sid"] for s in invoke(app, "project_list")}, msg="closed")
    assert round(doc_of(dirty_folder)["tracks"]["Kick In"]["events"][0]["t"] * SR) == IDX[0] + 10
    menu(app, "project", sid=other)
    app.wait_for_function("o => S.sid === o", arg=other)
    # a rename that is cancelled with Escape changes nothing; Discard keeps the saved name on disk
    menu(app, "rename")
    app.fill("#renameField", "Not kept")
    app.keyboard.press("Escape")
    assert app.evaluate("() => S.name") == "Other"
    menu(app, "rename")
    app.fill("#renameField", "Other 2")
    app.keyboard.press("Enter")
    app.wait_for_function("() => S.name === 'Other 2'")
    expect(app.locator("#dirtyDot")).to_be_visible()
    menu(app, "close", wait=False)
    app.click("#confirmModal button[data-act='discard']")
    app.wait_for_function("o => S.sid !== o", arg=other)
    assert doc_of(other_folder)["name"] == "Other"
    assert other not in {s["sid"] for s in invoke(app, "project_list")}


def test_a_pending_event_write_follows_its_project_across_switch_and_discard(app, make_project, tmp_path):
    """The debounced (400 ms) event write captures its project when it is scheduled. Switching away with
    "Continue without saving" flushes it into that project; "Discard changes" drops it. Neither may ever send one
    project's hits to another project with a track of the same name."""
    fa = build(make_project, tmp_path, events={"Kick In": exact_events()}, name="A")
    fb = build(make_project, tmp_path, events={"Kick In": exact_events()}, name="B")
    b = open_project(app, fb)
    a = open_project(app, fa)
    js(app, "b => { const n = 'Kick In'; S.events[n][0].t += 1 / S.tracks[n].sr; markDirty(n); window._sw = switchTo(b); }", b)
    expect(app.locator("#confirmModal")).to_be_visible()
    app.click("#confirmModal button[data-act='skip']")
    app.wait_for_function("b => S.sid === b", arg=b)
    app.wait_for_timeout(700)                                            # past the debounce
    assert service_samples(app, b) == IDX                                # B untouched
    assert service_samples(app, a) == [IDX[0] + 1] + IDX[1:]             # A got its edit before the switch
    assert invoke(app, "project_get", sid=a)["dirty"] is True and invoke(app, "project_get", sid=b)["dirty"] is False
    expect(app.locator("#stMsg")).not_to_contain_text("save failed")
    # discard inside the debounce: the pending write of B is cancelled, no other project changes
    js(app, "() => { const n = 'Kick In'; S.events[n][1].t += 2 / S.tracks[n].sr; markDirty(n); window._cl = closeProject(S.sid, S.name); }")
    expect(app.locator("#confirmModal")).to_be_visible()
    app.click("#confirmModal button[data-act='discard']")
    app.wait_for_function("b => S.sid !== b", arg=b)
    app.wait_for_timeout(700)
    assert b not in {s["sid"] for s in invoke(app, "project_list")}
    assert service_samples(app, a) == [IDX[0] + 1] + IDX[1:]
    assert [round(e["t"] * SR) for e in doc_of(fb)["tracks"]["Kick In"]["events"]] == IDX     # on disk: the saved state
    expect(app.locator("#stMsg")).not_to_contain_text("save failed")
