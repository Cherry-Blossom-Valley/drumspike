"""P4-40a: the named undo history. Every undo step has a label and a place ({track, t}); Edit > Undo and Redo name the step
("Undo Delete Hit (A, 1.000 s)", the page sends `undoTitle` and `redoTitle` with the menu state); an undo selects the track
and the hit, shows the place and says what it undid in the status line; an undo back to the saved document clears the dirty
mark (the service compares the documents: `project_clean_check`) and a redo, an edit or a save moves it as before; a
selection or a view change is never a step; Edit > Undo History lists the steps newest first and a click goes back to a row.
Every wait is on the page's own state; no sleeps."""
import pytest
from playwright.sync_api import expect

from app_support import inspect_track, invoke, menu, open_project, wait_until
from test_histograms import hist  # noqa: F401  (fixture: model, analysed project)
from test_learn import lrn  # noqa: F401  (fixture: a Learn project)
from test_menus import select_track
from test_tracks_remove import DIALOG

HITS = {"A": [0.5, 1.0, 1.5], "B": [0.6, 1.1, 1.6, 2.1]}
TRACKS = (("A", "kick", 36), ("B", "snare", 38))


def js(page, expr, arg=None):
    return page.evaluate(expr, arg)


def setup(app, make_project):
    sid = open_project(app, make_project(tracks=TRACKS))
    for name, ts in HITS.items():                                           # no ids in the document: the page makes its own, as for detected hits
        invoke(app, "events_set", sid=sid, name=name, events=[{"t": t, "v": 90} for t in ts])
    app.evaluate("async () => { await loadProject(); }")
    app.wait_for_function("h => Object.entries(h).every(([n, ts]) => (S.events[n] || []).length === ts.length)", arg=HITS)
    return sid


def top(app, stack="undo"):
    return js(app, "s => { const o = S[s][S[s].length - 1]; return o ? { type: o.type, label: o.label, where: o.where, from: o.from, to: o.to } : null; }", stack)


def titles(app):
    return js(app, "() => { const m = DSShell.menuState(); return [m.undoTitle, m.redoTitle]; }")


def away(app, track="B"):
    """The view and the selected track are somewhere else: the reveal of an undo has to bring them back."""
    js(app, "() => { setView(3.0, 3.9); S.sel = null; }")
    select_track(app, track)


def revealed(app, track, t):
    assert js(app, "() => S.selTrack") == track
    v = js(app, "() => S.view")
    assert v["t0"] <= t <= v["t1"], (v, t)


def txt(label, track, t):
    return f"{label} ({track}, {t:.3f} s)"


# ---------- points 1 and 5: label, place, menu title, reveal, status ----------

def test_delete_hit_is_named_with_its_place_and_undo_reveals_it(app, make_project):
    setup(app, make_project)
    assert titles(app) == ["Undo", "Redo"]                                  # nothing to undo: the plain words
    select_track(app, "A")
    js(app, "() => { S.sel = { track: 'A', index: 1 }; deleteSelectedHit(); }")
    app.wait_for_function("() => S.events['A'].length === 2")
    assert top(app)["label"] == "Delete Hit" and top(app)["where"] == {"track": "A", "t": 1.0}
    assert titles(app) == [f"Undo {txt('Delete Hit', 'A', 1.0)}", "Redo"]
    away(app)
    js(app, "() => undo()")
    app.wait_for_function("() => S.redo.length === 1")
    revealed(app, "A", 1.0)
    assert js(app, "() => [S.sel.track, S.events[S.sel.track][S.sel.index].t]") == ["A", 1.0]   # the hit is back and selected
    expect(app.locator("#stMsg")).to_have_text(f"undid {txt('Delete Hit', 'A', 1.0)}")
    assert titles(app) == ["Undo", f"Redo {txt('Delete Hit', 'A', 1.0)}"]
    away(app)
    js(app, "() => redo()")
    app.wait_for_function("() => S.redo.length === 0")
    expect(app.locator("#stMsg")).to_have_text(f"redid {txt('Delete Hit', 'A', 1.0)}")
    revealed(app, "A", 1.0)


def test_add_hit_and_set_velocity_are_named(app, make_project):
    setup(app, make_project)
    select_track(app, "B")
    js(app, "() => addHitAt('B', 2.5)")
    app.wait_for_function("() => S.events['B'].length === 5")
    assert top(app)["label"] == "Add Hit" and top(app)["where"] == {"track": "B", "t": 2.5}
    assert titles(app)[0] == f"Undo {txt('Add Hit', 'B', 2.5)}"
    away(app, "A")
    js(app, "() => undo()")
    app.wait_for_function("() => S.events['B'].length === 4")
    revealed(app, "B", 2.5)                                                 # the hit is gone, its place is shown
    expect(app.locator("#stMsg")).to_have_text(f"undid {txt('Add Hit', 'B', 2.5)}")
    js(app, "() => { S.undo.length = 0; S.redo.length = 0; S.sel = { track: 'A', index: 2 }; }")
    js(app, "() => { $('#ilVel').value = '55'; infoFieldApply('vel'); }")   # the info line field (hidden here), as Return in it
    app.wait_for_function("() => S.events['A'][2].v === 55")
    assert top(app)["label"] == "Set Velocity" and top(app)["where"] == {"track": "A", "t": 1.5}
    away(app, "B")
    js(app, "() => undo()")
    app.wait_for_function("() => S.events['A'][2].v === 90")
    revealed(app, "A", 1.5)
    assert js(app, "() => S.sel && S.events[S.sel.track][S.sel.index].t") == 1.5
    expect(app.locator("#stMsg")).to_have_text(f"undid {txt('Set Velocity', 'A', 1.5)}")


def test_a_nudge_shows_the_place_it_goes_back_to(app, make_project):
    setup(app, make_project)
    select_track(app, "A")
    js(app, "() => { S.sel = { track: 'A', index: 0 }; nudgeSelected(480); }")
    app.wait_for_function("() => S.undo.length === 1")
    o = top(app)
    assert o["type"] == "move" and o["label"] == "Nudge Hit" and o["where"]["t"] == o["to"] and o["to"] > o["from"]
    assert titles(app)[0] == f"Undo {txt('Nudge Hit', 'A', o['from'])}"    # the undo goes back to `from`
    away(app)
    js(app, "() => undo()")
    app.wait_for_function("() => S.redo.length === 1")
    revealed(app, "A", o["from"])
    assert titles(app)[1] == f"Redo {txt('Nudge Hit', 'A', o['to'])}"
    expect(app.locator("#stMsg")).to_have_text(f"undid {txt('Nudge Hit', 'A', o['from'])}")


def test_a_hand_move_by_the_info_line_is_a_move_hit(app, make_project):
    setup(app, make_project)
    js(app, "() => { S.sel = { track: 'A', index: 1 }; setHitTime('A', 1, 1.25); }")
    app.wait_for_function("() => S.undo.length === 1")
    assert top(app)["label"] == "Move Hit" and top(app)["where"] == {"track": "A", "t": 1.25}


def test_recompute_velocities_shift_and_remove_track_are_named(app, make_project):
    sid = setup(app, make_project)
    # Recompute Velocities: all tracks in one step, the place is the first changed hit
    assert menu(app, "recompute_velocities") is True
    app.wait_for_function("() => S.undo.length === 1")
    o = top(app)
    assert o["type"] == "vel" and o["label"] == "Recompute Velocities" and o["where"]["track"] in ("A", "B")
    assert o["where"]["t"] in HITS[o["where"]["track"]]
    js(app, "() => { S.undo.length = 0; }")
    # the shift field of the Inspector
    inspect_track(app, "A")
    app.evaluate("""() => { const f = document.querySelector('#insParams .pShift'); f.value = '48';
                          f.dispatchEvent(new Event('change', { bubbles: true })); }""")
    wait_until(lambda: top(app) and top(app)["type"] == "shift", msg="the shift step")
    assert top(app)["label"] == "Shift Hits" and top(app)["where"] == {"track": "A", "t": None}
    assert titles(app)[0] == "Undo Shift Hits (A)"                          # no time: the track alone
    # Remove Track: the track is gone, the undo brings it back and selects it
    js(app, "() => { S.undo.length = 0; }")
    select_track(app, "B")
    menu(app, "tracks_remove", wait=False)
    app.click(f"{DIALOG} button[data-act='remove']")
    app.wait_for_function("() => S.undo.length === 1 && !S.tracks['B']")
    assert top(app)["label"] == "Remove Track B" and titles(app)[0] == "Undo Remove Track B (B)"
    js(app, "() => undo()")
    app.wait_for_function("() => !!S.tracks['B'] && S.redo.length === 1")
    assert js(app, "() => S.selTrack") == "B"
    expect(app.locator("#stMsg")).to_have_text("undid Remove Track B (B)")


def test_learn_this_and_the_bar_1_step_are_named(lrn):
    app, sid, _ = lrn
    app.evaluate("() => { S.selTrack = 'Kick In'; S.sel = { track: 'Kick In', index: 1 }; learnMark('this'); }")
    app.wait_for_function("() => S.undo.length === 1 && DSLearn.state().pending === 0")
    o = top(app)
    assert o["type"] == "learn" and o["label"] == "Learn This" and o["where"]["track"] == "Kick In" and o["where"]["t"] == pytest.approx(1.0, abs=0.01)
    assert titles(app)[0].startswith("Undo Learn This (Kick In, 1.0")
    away(app, "Snare")
    js(app, "() => undo()")
    app.wait_for_function("() => S.redo.length === 1")
    assert js(app, "() => S.selTrack") == "Kick In"
    # Set Bar 1 (the tempo map): a step without a place
    js(app, "() => { S.undo.length = 0; S.playhead = 2.025; }")
    assert menu(app, "set_bar1") is True
    app.wait_for_function("() => S.undo.length === 1")
    assert top(app)["type"] == "tempo" and top(app)["label"] == "Set Bar 1" and top(app)["where"] == {"track": None, "t": None}
    assert titles(app)[0] == "Undo Set Bar 1"


def test_a_repick_and_a_loudness_change_are_named(hist):
    sid, ins = hist
    app = ins.page
    ins.locator(".pSens").fill("120")
    app.wait_for_function("() => !HG.sens.base && !DSHist.state().busy")
    o = top(app)
    assert o["type"] == "repick" and o["label"] == "Re-pick Hits" and o["where"] == {"track": "Kick In", "t": None}
    assert titles(app)[0] == "Undo Re-pick Hits (Kick In)"
    ins.locator(".pVLo").fill("-45")
    ins.locator(".pVLo").press("Tab")
    wait_until(lambda: top(app)["type"] == "loud", msg="the loudness step")
    assert top(app)["label"] == "Set Loudness Range" and top(app)["where"] == {"track": "Kick In", "t": None}


# ---------- point 2: the saved position ----------

def dirty(app, sid):
    return invoke(app, "project_get", sid=sid)["dirty"]


def test_undo_back_to_the_saved_state_clears_the_dirty_mark_and_a_save_sets_it(app, make_project):
    sid = setup(app, make_project)
    app.evaluate("async () => { await B.projectSave(S.sid); setDirtyState(false); }")
    assert dirty(app, sid) is False
    select_track(app, "A")
    js(app, "() => { S.sel = { track: 'A', index: 1 }; deleteSelectedHit(); }")
    app.wait_for_function("() => S.dirty === true && S.undo.length === 1")
    assert dirty(app, sid) is True
    js(app, "() => undo()")
    app.wait_for_function("() => S.dirty === false")                         # the saved document again
    assert dirty(app, sid) is False and js(app, "() => document.title.startsWith('●')") is False
    js(app, "() => redo()")
    app.wait_for_function("() => S.dirty === true")                          # a redo is a change again
    assert dirty(app, sid) is True
    # a save sets the position: an undo after the save is dirty, the redo is clean
    app.evaluate("async () => { await B.projectSave(S.sid); }")
    app.wait_for_function("() => S.dirty === false")
    js(app, "() => undo()")
    app.wait_for_function("() => S.redo.length === 1 && S.events['A'].length === 3")
    app.wait_for_function("() => S.dirty === true")
    js(app, "() => redo()")
    app.wait_for_function("() => S.dirty === false && S.events['A'].length === 2")
    assert dirty(app, sid) is False
    # two edits and one undo are not the saved state
    js(app, "() => { S.sel = { track: 'B', index: 0 }; deleteSelectedHit(); }")
    app.wait_for_function("() => S.events['B'].length === 3")
    js(app, "() => { S.sel = { track: 'B', index: 0 }; deleteSelectedHit(); }")
    app.wait_for_function("() => S.events['B'].length === 2")
    js(app, "() => undo()")
    app.wait_for_function("() => S.events['B'].length === 3 && S.redo.length === 1")
    app.wait_for_timeout(900)                                                # the check has surely run
    assert dirty(app, sid) is True and js(app, "() => S.dirty") is True
    js(app, "() => undo()")
    app.wait_for_function("() => S.dirty === false")


# ---------- point 3: the tail rule ----------

def test_a_selection_or_a_view_change_is_never_a_step(app, make_project):
    setup(app, make_project)
    js(app, """() => {
        S.sel = { track: 'A', index: 1 }; setSelTrack('B'); setView(1, 2); centerOn(2.5); zoomStep(true); zoomStep(false);
        gotoEvent(1); gotoEvent(-1); S.playhead = 1.2; zoomPreset(2); lnSetSel(null); }""")
    assert js(app, "() => [S.undo.length, S.redo.length]") == [0, 0]
    # the steps stay what the edits made: one selection change between two edits adds no step
    js(app, "() => { S.sel = { track: 'A', index: 0 }; deleteSelectedHit(); }")
    app.wait_for_function("() => S.undo.length === 1")
    js(app, "() => { setSelTrack('B'); setView(0, 3); S.sel = { track: 'B', index: 1 }; }")
    assert js(app, "() => S.undo.map(o => o.type)") == ["del"]


# ---------- point 4: Edit > Undo History ----------

def rows(app):
    return app.locator("#undoHistList button").all_inner_texts()


def test_the_history_lists_the_steps_newest_first_and_a_click_goes_back_to_the_row(app, make_project):
    setup(app, make_project)
    select_track(app, "A")
    js(app, "() => { S.sel = { track: 'A', index: 0 }; deleteSelectedHit(); }")
    app.wait_for_function("() => S.undo.length === 1")
    js(app, "() => addHitAt('A', 2.0)")
    app.wait_for_function("() => S.undo.length === 2")
    js(app, "() => { S.sel = { track: 'A', index: 0 }; nudgeSelected(480); }")
    app.wait_for_function("() => S.undo.length === 3")
    assert menu(app, "undo_history") is True
    expect(app.locator("#undoHistModal")).to_be_visible()
    got = rows(app)
    assert len(got) == 3 and got[0].startswith("Nudge Hit (A, ") and got[1] == txt("Add Hit", "A", 2.0) and got[2] == txt("Delete Hit", "A", 0.5)
    assert app.locator("#undoHistList .uhRedo").count() == 0
    app.locator("#undoHistList button").nth(1).click()                      # back to (and with) the Add Hit step: two steps
    expect(app.locator("#undoHistModal")).to_be_hidden()
    app.wait_for_function("() => S.undo.length === 1 && S.redo.length === 2")
    assert js(app, "() => S.events['A'].map(e => e.t)") == [1.0, 1.5]       # the nudge and the add are undone, the delete stays
    assert menu(app, "undo_history") is True
    got = rows(app)
    assert len(got) == 3 and got[2] == txt("Delete Hit", "A", 0.5)          # the undone steps above, newest first
    assert app.locator("#undoHistList .uhRedo").count() == 2
    assert got[0].startswith("Nudge Hit") and got[1] == txt("Add Hit", "A", 2.0)
    app.locator("#undoHistList button").nth(1).click()                      # a grey row: redo up to it
    app.wait_for_function("() => S.undo.length === 2 && S.redo.length === 1")
    assert js(app, "() => S.events['A'].map(e => e.t)") == [1.0, 1.5, 2.0]


def test_the_history_of_a_project_without_steps_says_so(app, make_project):
    setup(app, make_project)
    assert menu(app, "undo_history") is True
    expect(app.locator("#undoHistEmpty")).to_be_visible()
    assert rows(app) == []
    app.keyboard.press("Escape")
    expect(app.locator("#undoHistModal")).to_be_hidden()


def test_undo_of_a_delete_on_an_opened_project_restores_the_hit_and_clears_the_mark(app, make_project):
    """The opened project (no save yet): Delete Hit and its undo give the document back, rise fields included."""
    folder = make_project(tracks=TRACKS)
    sid = open_project(app, folder)
    invoke(app, "events_set", sid=sid, name="A", events=[{"t": t, "v": 90, "rise": 2.5 + t, "rise_db": 6.0} for t in HITS["A"]])
    invoke(app, "project_save", sid=sid)
    app.evaluate("async () => { await loadProject(); setDirtyState(false); }")
    app.wait_for_function("() => S.events['A'].length === 3")
    before = invoke(app, "events_get", sid=sid, name="A")["events"]
    select_track(app, "A")
    js(app, "() => { S.sel = { track: 'A', index: 1 }; deleteSelectedHit(); }")
    app.wait_for_function("() => S.events['A'].length === 2 && S.dirty")
    js(app, "() => undo()")
    app.wait_for_function("() => S.dirty === false")
    assert invoke(app, "events_get", sid=sid, name="A")["events"] == before and dirty(app, sid) is False
