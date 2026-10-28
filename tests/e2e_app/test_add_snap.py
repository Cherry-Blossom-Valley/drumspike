"""#144: an Option-click adds a hit on the attack near the pointer (the refinement onset of the analysis) with the velocity
of the audio there (`events_edit add` with `snap: true`, `crates/app/src/edit.rs` `snap_onset`). A click with no attack
within 10 ms keeps the click time and says so; undo gives back the document as it was."""
import json

import pytest
from playwright.sync_api import expect

from app_support import edits_done, invoke, menu, open_project
from test_slice import wait_analysis

SR = 48000
NAME = "Kick In"


def option_click(app, t):
    """Option-click on the spike lane of the kick track at time t (the page's own lane geometry)."""
    x = app.evaluate("t => GEO.laneX + tToX(t, laneWidth())", t)
    box = app.locator(".track").first.locator(".spike").bounding_box()
    app.keyboard.down("Alt")
    app.mouse.click(x, box["y"] + box["height"] / 2)
    app.keyboard.up("Alt")
    edits_done(app)


def held(app, sid):
    return invoke(app, "events_get", sid=sid, name=NAME)["events"]


def test_option_click_snaps_to_the_attack_with_the_audio_velocity_and_undo_restores_the_document(app, make_project):
    sid = open_project(app, make_project(tracks=((NAME, "kick", 36),)))
    app.click("#analyzeAllBtn")
    wait_analysis(app)
    auto = held(app, sid)
    i = next(k for k, e in enumerate(auto) if abs(e["t"] - 1.5) < 0.01)
    onset = auto[i]["t"]
    # the missed hit: the analysis hit at 1.5 s is gone, the document is saved
    invoke(app, "events_edit", sid=sid, name=NAME, ops=[{"op": "delete", "index": i}])
    app.evaluate("async () => { await loadProject(); }")
    app.wait_for_function("([n, k]) => S.events[n].length === k", arg=[NAME, len(auto) - 1])
    invoke(app, "project_save", sid=sid)
    before = json.dumps(held(app, sid), sort_keys=True)
    app.evaluate("([n, o]) => { S.snap = false; S.selTrack = n; setView(o - 0.25, o + 0.25); }", [NAME, onset])
    assert app.evaluate("() => S.view.t1 - S.view.t0") == pytest.approx(0.5)   # the 500 ms view

    option_click(app, onset - 0.003)
    ev = held(app, sid)
    assert len(ev) == len(auto)
    added = [e for e in ev if e.get("added")]
    assert len(added) == 1, ev
    t = added[0]["t"]
    assert abs(t - onset) * SR <= 2, (t, onset)
    want = invoke(app, "velocities_recompute", sid=sid, name=NAME, events=[{"t": t}])["events"][0]["v"]
    assert added[0]["v"] == want and want != 100, (added[0], want)
    expect(app.locator("#stMsg")).to_contain_text("added hit on the attack")
    expect(app.locator("#stMsg")).to_contain_text(f"velocity {want}")
    assert app.evaluate("n => S.events[n][S.sel.index].t", NAME) == t   # the new hit is selected

    # undo: the document is the saved one again, byte for byte; redo puts the hit back where it landed
    menu(app, "undo")
    edits_done(app)
    assert json.dumps(held(app, sid), sort_keys=True) == before
    app.wait_for_function("async () => (await B.projectCleanCheck(S.sid)).dirty === false")
    menu(app, "redo")
    edits_done(app)
    again = [e for e in held(app, sid) if e.get("added")]
    assert [(e["t"], e["v"]) for e in again] == [(t, want)]
    menu(app, "undo")
    edits_done(app)
    assert json.dumps(held(app, sid), sort_keys=True) == before

    # 15 ms before the attack: no attack within 10 ms, the hit stays at the click with velocity 100
    option_click(app, onset - 0.015)
    added = [e for e in held(app, sid) if e.get("added")]
    assert len(added) == 1 and added[0]["v"] == 100, added
    click = app.evaluate("n => S.events[n][S.sel.index].t", NAME)
    assert added[0]["t"] == click and abs(click - (onset - 0.015)) < 0.001, (added, click)
    expect(app.locator("#stMsg")).to_contain_text("no attack within 10 ms: the hit stays at the click")
    menu(app, "undo")
    edits_done(app)
    assert json.dumps(held(app, sid), sort_keys=True) == before
