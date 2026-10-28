"""P4-5, P4-9: the page and the agent commands give the same events. The Hitpoint shift (`track_shift_set`), the merge of the
hand edits (`keep_edited` of `track_repick` and `analyze_start`, Rust `merge_edited`) and, since stage 2, the hit edits (`addHitAt`,
`setHitTime`, `deleteSelectedHit`, the velocity field, the hit drag: `events_edit`) run in the service; the page only shows
them. These tests check the page against the commands only: the page has no edit code of its own."""
import json
from pathlib import Path

from app_support import edits_done, export_dialog, invoke, open_project, inspect_track, wait_until
from test_histograms import build as hist_build, settled
from test_subtracks import analyse, has_model, js
import pytest

FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "agent-parity" / "merge_edited.json"
KEYS = ("t", "v", "added", "orig", "rise", "cut")


def keep_keys(events):
    return [{k: e[k] for k in KEYS if e.get(k) is not None} for e in events]


def test_the_merge_fixture_cases_are_the_cases_of_the_rust_function():
    """`merge_edited` (crates/app/src/edit.rs) is the one merge: its Rust test runs these cases; the page has no copy."""
    cases = json.loads(FIXTURE.read_text())["cases"]
    assert len(cases) == 4 and all({"current", "picked", "expected"} <= set(c) for c in cases)


def py_merge(picked, current):
    """The fixture rule in Python, to check the stored events of a re-pick and of a Re-analyze with Keep."""
    man = [e for e in current if e.get("added") or (e.get("orig") is not None and abs(e["t"] - e["orig"]) > 1e-9)]
    out = [e for e in picked if not any(abs(m["t"] - e["t"]) < 0.020 for m in man)] + man
    return sorted(out, key=lambda e: e["t"])


def two_tracks(app, make_project):
    """Two tracks with the same hits on the sample grid; returns the sid."""
    sid = open_project(app, make_project(tracks=(("A", "kick", 36), ("B", "kick", 36))))
    ev = [{"t": 0.5 + 0.5 * i, "v": 80 + i} for i in range(5)]
    for n in ("A", "B"):
        assert invoke(app, "events_set", sid=sid, name=n, events=ev)["saved"] == 5
    js(app, "() => loadEventsOnly()")
    return sid


def stored(app, sid, name):
    js(app, "() => flushEvents()")
    return keep_keys(invoke(app, "events_get", sid=sid, name=name)["events"])


def test_the_page_shift_and_track_shift_set_move_the_hits_alike(app, make_project):
    sid = two_tracks(app, make_project)
    inspect_track(app, "A")
    for shift in (48, -96, 0):
        app.evaluate("""n => { const f = document.querySelector('#insParams .pShift'); f.value = String(n);
                              f.dispatchEvent(new Event('change', { bubbles: true })); }""", shift)
        assert invoke(app, "track_shift_set", sid=sid, name="B", shift_samples=shift)["shift_samples"] == shift
        a, b = stored(app, sid, "A"), stored(app, sid, "B")
        assert a == b, f"shift {shift}"
    app.evaluate("""() => { const f = document.querySelector('#insParams .pShift'); f.value = '48';
                          f.dispatchEvent(new Event('change', { bubbles: true })); }""")
    invoke(app, "track_shift_set", sid=sid, name="B", shift_samples=48)
    assert stored(app, sid, "A") == stored(app, sid, "B") != keep_keys([{"t": 0.5 + 0.5 * i, "v": 80 + i} for i in range(5)])
    # the page keeps the shift in its params and sends them with the next analysis; the command stores it itself
    assert invoke(app, "tracks_get", sid=sid)["tracks"]["B"]["params"]["shift_samples"] == 48
    assert invoke(app, "tracks_get", sid=sid)["tracks"]["A"]["params"]["shift_samples"] == 48   # P4-9: the service stores it too
    assert js(app, "() => S.events['A'].map(e => e.t)") == [e["t"] for e in invoke(app, "events_get", sid=sid, name="A")["events"]]


def shift_field(app, n):
    app.evaluate("""n => { const f = document.querySelector('#insParams .pShift'); f.value = String(n);
                          f.dispatchEvent(new Event('change', { bubbles: true })); }""", n)


def test_the_page_shift_is_one_undo_step_and_redo_gives_it_back(app, make_project):
    sid = two_tracks(app, make_project)
    inspect_track(app, "A")
    start = stored(app, sid, "A")
    ids = js(app, "() => S.events['A'].map(e => e.id)")
    shift_field(app, 48)
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["A"].get("params", {}).get("shift_samples") == 48, msg="the shift")
    moved = stored(app, sid, "A")
    assert moved != start and js(app, "() => S.undo.length") == 1 and js(app, "() => S.undo[0].type") == "shift"
    assert js(app, "() => S.events['A'].map(e => e.id)") == ids      # the hits keep their ids: the undo steps stay valid
    js(app, "() => undo()")
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["A"].get("params", {}).get("shift_samples") == 0, msg="undo: shift 0")
    assert stored(app, sid, "A") == start
    assert js(app, "() => S.events['A'].map(e => e.t)") == [e["t"] for e in invoke(app, "events_get", sid=sid, name="A")["events"]]
    assert app.evaluate("() => document.querySelector('#insParams .pShift').value") == "0"
    js(app, "() => redo()")
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["A"].get("params", {}).get("shift_samples") == 48, msg="redo: shift 48")
    assert stored(app, sid, "A") == moved
    assert js(app, "() => S.events['A'].map(e => e.t)") == [e["t"] for e in invoke(app, "events_get", sid=sid, name="A")["events"]]


def test_the_page_shift_moves_a_hit_edited_a_moment_ago(app, make_project):
    """A pending hand edit (the 400 ms write is not done yet) reaches the service before the shift."""
    sid = two_tracks(app, make_project)
    inspect_track(app, "A")
    js(app, "() => { S.events['A'][1].v = 33; markDirty('A'); }")
    shift_field(app, 48)
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["A"].get("params", {}).get("shift_samples") == 48, msg="the shift")
    got = stored(app, sid, "A")
    assert [e["v"] for e in got][1] == 33
    assert [round(e["t"], 6) for e in got] == [round(0.501 + 0.5 * i, 6) for i in range(5)]   # shifted, not the stale copy
    assert [round(e["t"], 6) for e in js(app, "() => S.events['A']")] == [round(0.501 + 0.5 * i, 6) for i in range(5)]


@pytest.fixture
def hist(app, make_project, tmp_path):
    if not has_model(app):
        pytest.skip("the re-pick needs the model (the DSP fallback keeps no activations)")
    sid = open_project(app, hist_build(make_project, tmp_path, "Parity"))
    analyse(app)
    return sid, inspect_track(app, "Kick In")


def edit_two_hits(app, sid):
    """A new hit and a moved one, through events_edit; the page loads them."""
    base = keep_keys(invoke(app, "events_get", sid=sid, name="Kick In")["events"])
    near = next(i for i, e in enumerate(base) if abs(e["t"] - 1.5) < 0.05)
    r = invoke(app, "events_edit", sid=sid, name="Kick In",
               ops=[{"op": "add", "t": 5.25, "v": 99}, {"op": "move", "index": near, "t": base[near]["t"] + 0.012}])
    assert r["added"] == 1 and r["moved"] == 1, r
    js(app, "() => loadEventsOnly()")
    return base, keep_keys(invoke(app, "events_get", sid=sid, name="Kick In")["events"])


def test_a_page_repick_keeps_the_edited_hits_as_the_command_does(app, hist):
    sid, ins = hist
    _, edited = edit_two_hits(app, sid)
    ins.locator(".pSens").fill("140")
    settled(app)
    params = js(app, "() => S.tracks['Kick In'].params")   # the thresholds the slider set
    pick = invoke(app, "track_repick", sid=sid, name="Kick In", params=params, store=False, keep_edited=False)["events"]
    want = keep_keys(py_merge(pick, edited))
    wait_until(lambda: stored(app, sid, "Kick In") == want, msg="the stored re-pick with the edited hits")
    assert keep_keys(js(app, "() => S.events['Kick In']")) == want
    assert any(e.get("added") and e["t"] == 5.25 for e in want) and len(want) != len(edited)
    # one undo step: undo gives the hits before the re-pick, redo the merged ones
    js(app, "() => undo()")
    wait_until(lambda: stored(app, sid, "Kick In") == edited, msg="undo of the re-pick")
    js(app, "() => redo()")
    wait_until(lambda: stored(app, sid, "Kick In") == want, msg="redo of the re-pick")
    assert js(app, "() => S.undo.filter(o => o.type === 'repick').length") == 1


def test_a_page_reanalyze_with_keep_merges_in_the_service(app, hist):
    sid, ins = hist
    base, edited = edit_two_hits(app, sid)
    want = keep_keys(py_merge(base, edited))
    ins.locator(".bReanalyze").click()
    app.click("#raKeep")
    expect_done(app)
    wait_until(lambda: stored(app, sid, "Kick In") == want, msg="the analysis with the edited hits kept")
    assert keep_keys(js(app, "() => S.events['Kick In']")) == want
    assert any(e.get("added") and e["t"] == 5.25 for e in want) and want != base
    assert "edited hits kept" in app.locator("#stMsg").inner_text()


def expect_done(app):
    app.wait_for_function("() => !analysisRunning", timeout=90000)


# ---------- P4-9 stage 1 fix round ----------

def shift_of(app, sid, name="A"):
    return invoke(app, "tracks_get", sid=sid)["tracks"][name].get("params", {}).get("shift_samples", 0)


def shift_steps(app):
    return js(app, "() => S.undo.filter(o => o.type === 'shift').map(o => [o.from, o.to])")


def wait_shift_idle(app):
    app.wait_for_function("() => SH.pending === 0")


def test_two_quick_shifts_make_two_steps_from_the_value_the_last_one_stored(app, make_project):
    sid = two_tracks(app, make_project)
    inspect_track(app, "A")
    app.evaluate("""() => { const f = document.querySelector('#insParams .pShift');
        for (const v of ['48', '96']) { f.value = v; f.dispatchEvent(new Event('change', { bubbles: true })); } }""")
    wait_shift_idle(app)
    assert shift_steps(app) == [[0, 48], [48, 96]] and shift_of(app, sid) == 96
    app.evaluate("""() => { const f = document.querySelector('#insParams .pShift');
        for (const v of ['0', '48', '0']) { f.value = v; f.dispatchEvent(new Event('change', { bubbles: true })); } }""")
    wait_shift_idle(app)
    assert shift_steps(app)[2:] == [[96, 0], [0, 48], [48, 0]] and shift_of(app, sid) == 0
    assert stored(app, sid, "A") == keep_keys([{"t": 0.5 + 0.5 * i, "v": 80 + i} for i in range(5)])


def test_undo_waits_for_a_shift_that_is_on_its_way(app, make_project):
    """An undo asked while a shift is on its way runs after it and takes the step that is on top then: the shift."""
    sid = two_tracks(app, make_project)
    inspect_track(app, "A")
    js(app, "() => addHitAt('A', 0.1)")
    edits_done(app)
    app.evaluate("""() => { const f = document.querySelector('#insParams .pShift'); f.value = '48';
        f.dispatchEvent(new Event('change', { bubbles: true })); undo(); }""")
    edits_done(app)
    assert js(app, "() => S.undo.map(o => o.type)") == ["add"] and js(app, "() => S.redo.map(o => o.type)") == ["shift"], "the undo did not wait for the shift"
    assert shift_of(app, sid) == 0 and any(abs(e["t"] - 0.1) < 0.002 for e in js(app, "() => S.events['A']"))


def test_a_refused_shift_leaves_no_undo_step_and_the_field_goes_back(app, make_project):
    sid = two_tracks(app, make_project)
    inspect_track(app, "A")
    js(app, "() => { B.shiftSet = () => Promise.reject(new Error('refused')); }")
    shift_field(app, 48)
    wait_shift_idle(app)
    assert js(app, "() => S.undo.length") == 0 and shift_of(app, sid) == 0
    assert app.evaluate("() => document.querySelector('#insParams .pShift').value") == "0"


def test_a_shift_of_a_source_shows_its_followers_new_hits(app, make_project):
    sid = two_tracks(app, make_project)
    assert js(app, "() => setTrackFollow('B', 'A')") is not False
    inspect_track(app, "A")
    shift_field(app, 48)
    wait_until(lambda: shift_of(app, sid) == 48, msg="the shift")
    wait_shift_idle(app)
    want = [e["t"] for e in invoke(app, "events_get", sid=sid, name="B")["events"]]
    assert want[0] == pytest.approx(0.501, abs=1e-6)
    app.wait_for_function("w => S.events['B'].map(e => e.t).every((t, i) => Math.abs(t - w[i]) < 1e-9)", arg=want)


def test_a_shift_while_a_repick_is_stored_is_refused_and_the_repick_keeps_its_step(app, hist):
    sid, ins = hist
    errors = []
    app.on("pageerror", lambda e: errors.append(str(e)))
    js(app, """() => { window.__go = null; const o = B.repick.bind(B), first = { done: false };
        B.repick = (...a) => { if (first.done) return o(...a); first.done = true; return new Promise(r => { window.__go = () => r(o(...a)); }); }; }""")
    ins.locator(".pSens").fill("140")
    app.wait_for_function("() => window.__go !== null")
    shift_field(app, 48)
    wait_shift_idle(app)
    assert shift_steps(app) == [] and shift_of(app, sid, "Kick In") == 0
    js(app, "() => window.__go()")
    ins.locator(".pSens").dispatch_event("change")
    settled(app)
    wait_until(lambda: js(app, "() => S.undo.filter(o => o.type === 'repick').length") >= 1, msg="the re-pick step")
    assert errors == []


def test_a_reanalyze_sends_the_pending_edits_first(app, hist):
    sid, ins = hist
    js(app, "() => { window.__pend = null; const o = B.analyze.bind(B); B.analyze = (...a) => { window.__pend = Object.keys(saveTimers); return o(...a); }; }")
    js(app, "() => { S.events['Kick In'].push({t: 5.25, v: 99, id: newId(), added: true}); S.events['Kick In'].sort((a, b) => a.t - b.t); markDirty('Kick In'); }")
    ins.locator(".bReanalyze").click()
    app.click("#raDiscard")
    app.wait_for_function("() => window.__pend !== null")
    assert js(app, "() => window.__pend") == [], "a write of the page was still waiting when the analysis started"
    expect_done(app)


# ---------- P4-9 stage 2: the hit edits ----------

def start_of(app, sid, name="A"):
    return stored(app, sid, name)


def page_matches_service(app, sid, name="A"):
    """The hits the page shows are the stored ones (times and markers), and the ids are unique."""
    page = js(app, "n => S.events[n].map(e => ({ t: e.t, v: e.v, ...(e.added ? { added: true } : {}), ...(e.orig !== undefined ? { orig: e.orig } : {}), id: e.id }))", name)
    assert [{k: e[k] for k in e if k != "id"} for e in page] == [{k: e[k] for k in e if k in ("t", "v", "added", "orig")} for e in invoke(app, "events_get", sid=sid, name=name)["events"]]
    assert len({e["id"] for e in page}) == len(page)


def canon(events):
    """A hit moved back to its detected time keeps `orig` (the page did the same): it is not an edited hit then."""
    return [{k: v for k, v in e.items() if not (k == "orig" and v == e["t"])} for e in events]


def undo_redo_cycle(app, sid, start, edited, steps):
    """Undo gives the hits before the edits, redo the edited ones; `steps` undo steps in between."""
    assert js(app, "() => S.undo.length") == steps
    for _ in range(steps):
        js(app, "() => undo()")
    edits_done(app)
    assert canon(stored(app, sid, "A")) == start and js(app, "() => S.undo.length") == 0 and js(app, "() => S.redo.length") == steps
    page_matches_service(app, sid)
    for _ in range(steps):
        js(app, "() => redo()")
    edits_done(app)
    assert stored(app, sid, "A") == edited and js(app, "() => S.undo.length") == steps and js(app, "() => S.redo.length") == 0
    page_matches_service(app, sid)


def test_a_page_add_is_events_edit_add_with_undo_and_redo(app, make_project):
    sid = two_tracks(app, make_project)
    start = start_of(app, sid)
    js(app, "() => addHitAt('A', 0.1)")
    edits_done(app)
    assert invoke(app, "events_edit", sid=sid, name="B", ops=[{"op": "add", "t": 0.1}])["added"] == 1
    edited = stored(app, sid, "A")
    assert edited == stored(app, sid, "B") != start and {"t": 0.1, "v": 100, "added": True} in edited
    assert js(app, "() => S.events['A'][S.sel.index].t") == 0.1      # the new hit is selected
    page_matches_service(app, sid)
    undo_redo_cycle(app, sid, start, edited, 1)


def lane_point(app, t):
    # #96: the click point must come from ONE layout read. x from a live evaluate and
    # y from an earlier bounding_box are two separate snapshots; under load a reflow
    # between them aims the click at a moved lane (run 5: the right-click deleted
    # nothing). One evaluate returns both from the same layout state.
    return js(app, """() => { const r = document.querySelector(".track .spike").getBoundingClientRect();
        return { x: GEO.laneX + tToX(%s, laneWidth()), y: r.y + 20 }; }""" % t)


def test_a_page_drag_commits_one_move_on_release_as_events_edit_does(app, make_project):
    sid = two_tracks(app, make_project)
    start = start_of(app, sid)
    js(app, "() => { window.__calls = 0; const o = B.eventsEdit.bind(B); B.eventsEdit = (...a) => { window.__calls++; return o(...a); }; }")
    pt = lane_point(app, 1.0)
    app.mouse.move(pt["x"], pt["y"])
    app.mouse.down()
    app.mouse.move(pt["x"] + 60, pt["y"], steps=8)
    assert js(app, "() => window.__calls") == 0, "the drag wrote before the release"
    assert stored(app, sid, "A") == start
    app.mouse.up()
    edits_done(app)
    assert js(app, "() => window.__calls") == 1
    edited = stored(app, sid, "A")
    moved = [e for e in edited if e.get("orig") == 1.0]
    assert len(moved) == 1 and moved[0]["t"] > 1.0
    invoke(app, "events_edit", sid=sid, name="B", ops=[{"op": "move", "index": 1, "t": moved[0]["t"]}])
    assert edited == stored(app, sid, "B")
    page_matches_service(app, sid)
    undo_redo_cycle(app, sid, start, edited, 1)


def test_a_page_delete_and_a_right_click_delete_are_events_edit_delete(app, make_project):
    sid = two_tracks(app, make_project)
    start = start_of(app, sid)
    js(app, "() => { S.sel = { track: 'A', index: 2 }; deleteSelectedHit(); }")
    edits_done(app)
    invoke(app, "events_edit", sid=sid, name="B", ops=[{"op": "delete", "index": 2}])
    edited = stored(app, sid, "A")
    assert edited == stored(app, sid, "B") and len(edited) == 4 and not any(e["t"] == 1.5 for e in edited)
    assert js(app, "() => S.sel") is None
    page_matches_service(app, sid)
    undo_redo_cycle(app, sid, start, edited, 1)
    # the right-click: one delete, no menu
    pt = lane_point(app, 2.0)
    app.mouse.click(pt["x"], pt["y"], button="right")
    edits_done(app)
    assert [e["t"] for e in stored(app, sid, "A")] == [0.5, 1.0, 2.5]
    js(app, "() => undo()")
    edits_done(app)
    assert stored(app, sid, "A") == edited and not any(e.get("added") for e in edited)     # a detected hit comes back as a detected hit


def test_a_page_velocity_is_events_edit_velocity_with_undo_and_redo(app, make_project):
    sid = two_tracks(app, make_project)
    start = start_of(app, sid)
    js(app, "() => { S.sel = { track: 'A', index: 1 }; document.querySelector('#ilVel').value = '64'; infoFieldApply('vel'); }")
    edits_done(app)
    invoke(app, "events_edit", sid=sid, name="B", ops=[{"op": "velocity", "index": 1, "v": 64}])
    edited = stored(app, sid, "A")
    assert edited == stored(app, sid, "B") and edited[1]["v"] == 64 and edited != start
    page_matches_service(app, sid)
    undo_redo_cycle(app, sid, start, edited, 1)


def test_two_quick_edits_in_a_row_each_use_the_hits_the_one_before_stored(app, make_project):
    """The second edit names its hit by id: the add before it moved the index."""
    sid = two_tracks(app, make_project)
    start = start_of(app, sid)
    js(app, """() => { addHitAt('A', 0.1);
        S.sel = { track: 'A', index: S.events['A'].findIndex(e => e.t === 1.0) }; deleteSelectedHit();
        S.sel = { track: 'A', index: S.events['A'].findIndex(e => e.t === 1.5) }; document.querySelector('#ilVel').value = '55'; infoFieldApply('vel'); }""")
    edits_done(app)
    invoke(app, "events_edit", sid=sid, name="B", ops=[{"op": "add", "t": 0.1}, {"op": "delete", "index": 1}, {"op": "velocity", "index": 2, "v": 55}])
    edited = stored(app, sid, "A")
    assert edited == stored(app, sid, "B") and len(edited) == 5 and not any(e["t"] == 1.0 for e in edited)
    assert [e["v"] for e in edited if e["t"] == 1.5] == [55]
    page_matches_service(app, sid)
    undo_redo_cycle(app, sid, start, edited, 3)


def test_two_undos_asked_at_once_are_both_done(app, make_project):
    """Item c of the stage 1 re-review: a second Cmd+Z while the first is on its way is queued, not dropped."""
    sid = two_tracks(app, make_project)
    start = start_of(app, sid)
    js(app, "() => { addHitAt('A', 0.1); addHitAt('A', 0.2); }")
    edits_done(app)
    assert js(app, "() => S.undo.length") == 2
    js(app, "() => { undo(); undo(); }")
    edits_done(app)
    assert stored(app, sid, "A") == start and js(app, "() => S.redo.length") == 2 and js(app, "() => S.undo.length") == 0


def test_an_undo_asked_while_a_shift_is_held_and_a_second_one_are_both_done(app, make_project):
    sid = two_tracks(app, make_project)
    inspect_track(app, "A")
    js(app, "() => { addHitAt('A', 0.1); addHitAt('A', 0.2); }")
    edits_done(app)
    js(app, "() => { window.__rel = null; const o = B.shiftSet.bind(B); B.shiftSet = (...a) => new Promise(r => { window.__rel = () => { B.shiftSet = o; r(o(...a)); }; }); }")
    shift_field(app, 48)
    app.wait_for_function("() => window.__rel !== null")
    js(app, "() => { undo(); undo(); }")      # both wait behind the shift
    js(app, "() => window.__rel()")
    edits_done(app)
    assert js(app, "() => S.undo.map(o => o.type)") == ["add"] and js(app, "() => S.redo.map(o => o.type)") == ["shift", "add"]
    assert shift_of(app, sid) == 0


def test_a_refused_undo_leaves_the_stacks_as_the_service_is(app, make_project):
    """Item b: the step moves to the redo stack after the service applied it."""
    sid = two_tracks(app, make_project)
    inspect_track(app, "A")
    shift_field(app, 48)
    edits_done(app)
    assert shift_steps(app) == [[0, 48]]
    js(app, "() => { window.__ok = B.shiftSet; B.shiftSet = () => Promise.reject(new Error('refused')); }")
    js(app, "() => undo()")
    edits_done(app)
    assert js(app, "() => S.undo.map(o => o.type)") == ["shift"] and js(app, "() => S.redo.length") == 0 and shift_of(app, sid) == 48
    js(app, "() => { B.shiftSet = window.__ok; undo(); }")
    edits_done(app)
    assert js(app, "() => S.undo.length") == 0 and js(app, "() => S.redo.map(o => o.type)") == ["shift"] and shift_of(app, sid) == 0


def test_a_refused_edit_makes_no_step_and_the_page_shows_the_stored_hits(app, make_project):
    sid = two_tracks(app, make_project)
    start = start_of(app, sid)
    js(app, "() => addHitAt('A', 99)")      # after the end of the track: the service refuses
    edits_done(app)
    assert js(app, "() => S.undo.length") == 0 and stored(app, sid, "A") == start
    assert "after the end" in app.locator("#stMsg").inner_text()
    page_matches_service(app, sid)


def test_a_shift_in_flight_keeps_the_resensitivity_from_starting(app, hist):
    """Item a: while a shift call is queued or running, the Sensitivity slider starts no re-pick (`!SH.pending` in hgCanRepick)."""
    sid, ins = hist
    js(app, "() => { window.__rel = null; const o = B.shiftSet.bind(B); B.shiftSet = (...a) => new Promise(r => { window.__rel = () => r(o(...a)); }); }")
    shift_field(app, 48)
    app.wait_for_function("() => window.__rel !== null")
    ins.locator(".pSens").fill("140")
    assert js(app, "() => HG.sens.base === null && !HG.sens.want && !HG.sens.busy"), "a re-pick started while the shift was on its way"
    js(app, "() => window.__rel()")
    edits_done(app)
    assert shift_of(app, sid, "Kick In") == 48


# ---------- P4-9 stage 2 fix round ----------

def own_project(app, make_project):
    """Like `two_tracks`, with the project folder (for the saved document)."""
    folder = make_project(tracks=(("A", "kick", 36), ("B", "kick", 36)))
    sid = open_project(app, folder)
    ev = [{"t": 0.5 + 0.5 * i, "v": 80 + i} for i in range(5)]
    for n in ("A", "B"):
        assert invoke(app, "events_set", sid=sid, name=n, events=ev)["saved"] == 5
    js(app, "() => loadEventsOnly()")
    return sid, Path(folder)


def hold_edits(app, ms=400):
    """The next events_edit answers after `ms` (an edit that is on its way)."""
    js(app, "ms => { const o = B.eventsEdit.bind(B); B.eventsEdit = async (...a) => { await new Promise(r => setTimeout(r, ms)); return o(...a); }; }", ms)


def test_a_learn_mark_on_a_hit_takes_the_time_the_hit_has_after_a_nudge_on_its_way(app, make_project):
    sid = two_tracks(app, make_project)
    js(app, "() => { S.sel = { track: 'A', index: 1 }; nudgeSelected(480); learnMark('this'); }")
    edits_done(app)
    app.wait_for_function("() => DSLearn.state().pending === 0")
    hit = stored(app, sid, "A")[1]["t"]
    assert hit != 1.0
    marks = invoke(app, "tracks_get", sid=sid)["tracks"]["A"]["learn"]["marks"]
    assert len(marks) == 1 and [round(t, 6) for t in marks[0]["at"]] == [round(hit, 6)]


def test_the_undo_of_a_delete_keeps_the_orig_of_a_moved_hit(app, make_project):
    sid = two_tracks(app, make_project)
    js(app, "() => { setHitTime('A', 1, 1.25); }")
    edits_done(app)
    moved = stored(app, sid, "A")
    assert {"t": 1.25, "v": 81, "orig": 1.0} in moved
    js(app, "() => { S.sel = { track: 'A', index: 1 }; deleteSelectedHit(); }")
    edits_done(app)
    js(app, "() => undo()")
    edits_done(app)
    assert stored(app, sid, "A") == moved


def test_an_edit_sends_the_pending_400_ms_write_of_another_step_first(app, make_project):
    """A velocity changed by a step that still writes the page's copy (`markDirty`) is not overwritten by the edit."""
    sid = two_tracks(app, make_project)
    js(app, "() => { S.events['A'][0].v = 33; markDirty('A'); addHitAt('A', 0.1); }")
    edits_done(app)
    app.wait_for_timeout(700)      # the 400 ms timer would write the old copy now
    got = stored(app, sid, "A")
    assert got[0] == {"t": 0.1, "v": 100, "added": True} and got[1]["v"] == 33 and len(got) == 6


def test_a_save_waits_for_a_hit_edit_on_its_way(app, make_project):
    sid, folder = own_project(app, make_project)
    hold_edits(app)
    js(app, "() => { addHitAt('A', 0.1); window.__saved = saveProjectNow(); }")
    js(app, "() => window.__saved")
    doc = json.loads(next(folder.glob("*.drumspike")).read_text())
    assert any(abs(e["t"] - 0.1) < 1e-6 for e in doc["tracks"]["A"]["events"]), "the save ran before the edit"


def test_a_midi_export_waits_for_a_hit_edit_on_its_way(app, make_project):
    sid, folder = own_project(app, make_project)
    hold_edits(app)
    js(app, "() => { addHitAt('A', 0.1); }")
    export_dialog(app, formats=("midi",), scope="selected", track="A")   # P4-30: the dialog flushes the edit on its way first
    from test_infoline import midi_note_ons
    assert len(midi_note_ons(folder / "Exports" / "A_spike.mid")) == 6, "the MIDI file was written from the hits before the edit"


def test_an_edit_during_an_analysis_is_refused_and_the_refusal_shows_beside_the_progress(app, make_project):
    sid = two_tracks(app, make_project)
    js(app, "() => { analysisRunning = true; showProgress(10, 'Analyzing 0/1'); addHitAt('A', 0.1); }")
    edits_done(app)
    assert "refused while an analysis runs" in app.locator("#progressText").inner_text()
    assert js(app, "() => S.undo.length") == 0 and len(stored(app, sid, "A")) == 5
    js(app, "() => { analysisRunning = false; hideProgress(); }")


@pytest.mark.parametrize("released_while_it_runs", [False, True])
def test_a_drag_dropped_because_another_edit_changed_the_hits_says_so(app, make_project, released_while_it_runs):
    sid = two_tracks(app, make_project)
    if released_while_it_runs:
        hold_edits(app, 1500)
    start = start_of(app, sid)
    pt = lane_point(app, 1.0)
    app.mouse.move(pt["x"], pt["y"])
    app.mouse.down()
    app.mouse.move(pt["x"] + 60, pt["y"], steps=8)
    js(app, "() => { addHitAt('A', 0.1); }")      # braces: the page must not wait for the call
    if not released_while_it_runs:
        edits_done(app)
    app.mouse.up()
    edits_done(app)
    assert "move was dropped" in app.locator("#stMsg").inner_text()
    assert [e for e in stored(app, sid, "A") if "orig" in e] == [] and len(stored(app, sid, "A")) == len(start) + 1
    page_matches_service(app, sid)
