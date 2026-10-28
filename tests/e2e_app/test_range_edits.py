"""P4-41a: Delete and the nudge keys act on every hit of the range selection (workflow polish rank 3).

A range over 5 hits on one track plus Delete removes exactly those 5 in one undo step, and one undo
gives back a document byte-identical to the one before (the service's own definition, P4-40a:
project_clean_check answers dirty false again; the file content is compared too, without the
`modified` stamp that every save writes). The nudge keys move exactly those 5 hits by the nudge step
in one undo step. Hits outside the range and on other tracks do not change. With no range the keys
keep today's single-hit behaviour (and the track confirm stays behind them).
"""
import json
from pathlib import Path

import pytest

from app_support import edits_done, invoke, open_project, wait_until
from helpers import SR, synth_track, write_wav

INSIDE = [0.9, 1.2, 1.5, 1.8, 2.1]           # the 5 hits of the range
OUTSIDE = [0.3, 3.5]                         # two hits of the same track outside it
OTHER = [0.7, 2.0]                           # the hits of the other track
TRACKS = (("Kick In", "kick", 36), ("Snare Top", "snare", 38))


def evs(ts):
    return [{"t": t, "v": 90} for t in ts]   # no ids: the undo must not lose one


def js(page, expr, arg=None):
    return page.evaluate(expr, arg) if arg is not None else page.evaluate(expr)


def samples(page, sid, name):
    return sorted(round(e["t"] * SR) for e in invoke(page, "events_get", sid=sid, name=name)["events"])


def doc_normalized(folder):
    """The document on disk without the `modified` stamp every save writes (the content identity)."""
    doc = json.loads(next(Path(folder).glob("*.drumspike")).read_text())
    doc.pop("modified", None)
    return json.dumps(doc, sort_keys=True, separators=(",", ":"))


def top_step(page):
    return js(page, "() => S.undo[S.undo.length - 1]")


@pytest.fixture
def project(app, make_project, tmp_path):
    """A copied project with the hits above already in the document, opened and saved once (the baseline)."""
    src = tmp_path / "src_range"
    src.mkdir()
    specs = []
    for name, role, note, hits in (("Kick In", "kick", 36, sorted(INSIDE + OUTSIDE)), ("Snare Top", "snare", 38, OTHER)):
        x, _ = synth_track([0.5, 1.5, 2.5], 6.0)
        specs.append((name, write_wav(src / f"{name}.wav", x), role, note, evs(hits)))
    folder = Path(make_project.builder.build_project(tmp_path / "projects_range", "Range", specs, "copied"))
    open_project(app, folder)
    sid = js(app, "() => S.sid")
    invoke(app, "project_save", sid=sid)          # the baseline the undo must come back to
    yield app, sid, folder


def wait_step(page, kind, n):
    """The top undo step is the expected one (the edit is on its way when the key lands)."""
    wait_until(lambda: (lambda s: s is not None and s["type"] == kind and len(s.get("list", [])) == n)(top_step(page)),
               msg=f"the {kind} step with {n} hits")


def test_a_range_delete_removes_exactly_its_hits_in_one_undo_step_and_undo_restores_the_document(project):
    app, sid, folder = project
    js(app, "() => { S.sel = null; lnSetSel('Kick In', 0.8, 2.4); }")
    content0 = doc_normalized(folder)
    undo_n = js(app, "() => S.undo.length")
    app.keyboard.press("Delete")
    edits_done(app)
    wait_step(app, "del-many", 5)
    wait_until(lambda: samples(app, sid, "Kick In") == [int(t * SR) for t in OUTSIDE], msg="the hits of the range are gone")
    assert samples(app, sid, "Snare Top") == [int(t * SR) for t in OTHER]   # the other track does not change
    step = top_step(app)
    assert js(app, "() => S.undo.length") == undo_n + 1                     # one undo step, not five
    assert step["type"] == "del-many" and step["track"] == "Kick In"
    assert len(step["list"]) == 5 and sorted(round(s["t"] * SR) for s in step["list"]) == [int(t * SR) for t in INSIDE]
    app.keyboard.press("Z")                                                 # one undo, no save of the deleted state in between
    edits_done(app)
    wait_until(lambda: samples(app, sid, "Kick In") == [int(t * SR) for t in sorted(INSIDE + OUTSIDE)], msg="the undo puts the hits back")
    assert js(app, "() => S.undo.length") == undo_n
    assert invoke(app, "project_clean_check", sid=sid)["dirty"] is False    # the document is the saved one again (P4-40a)
    invoke(app, "project_save", sid=sid)
    assert doc_normalized(folder) == content0                               # byte-identical (without the save stamp)
    app.keyboard.press("Shift+Z")                                           # the redo deletes the 5 again, one step
    edits_done(app)
    wait_step(app, "del-many", 5)
    wait_until(lambda: samples(app, sid, "Kick In") == [int(t * SR) for t in OUTSIDE], msg="the redo deletes them again")
    app.keyboard.press("Z")                                                 # and one more undo puts them back
    edits_done(app)
    wait_until(lambda: samples(app, sid, "Kick In") == [int(t * SR) for t in sorted(INSIDE + OUTSIDE)], msg="the second undo puts them back")


def test_the_nudge_keys_move_exactly_the_hits_of_the_range_in_one_undo_step(project):
    app, sid, folder = project
    js(app, "() => { S.sel = null; lnSetSel('Kick In', 0.8, 2.4); }")
    undo_n = js(app, "() => S.undo.length")
    app.keyboard.press(".")                                                 # nudge later by the value (1 smp)
    edits_done(app)
    wait_until(lambda: samples(app, sid, "Kick In") == sorted([int(t * SR) for t in OUTSIDE] + [int(t * SR) + 1 for t in INSIDE]),
               msg="the 5 hits moved by one sample")
    assert samples(app, sid, "Snare Top") == [int(t * SR) for t in OTHER]
    wait_step(app, "move-many", 5)
    step = top_step(app)
    assert js(app, "() => S.undo.length") == undo_n + 1
    assert step["type"] == "move-many" and step["track"] == "Kick In"
    assert all(round(s["to"] * SR) - round(s["from"] * SR) == 1 for s in step["list"])
    app.keyboard.press("Z")
    edits_done(app)
    wait_until(lambda: samples(app, sid, "Kick In") == [int(t * SR) for t in sorted(INSIDE + OUTSIDE)], msg="the undo moves them back")
    assert js(app, "() => S.undo.length") == undo_n
    # the x10 keys move the same 5 hits by 10 x the value, one step again
    app.keyboard.press("Shift+ArrowRight")                                   # the app's "Shift+Right" (keymap.js)
    edits_done(app)
    wait_until(lambda: samples(app, sid, "Kick In") == sorted([int(t * SR) for t in OUTSIDE] + [int(t * SR) + 10 for t in INSIDE]),
               msg="the 5 hits moved by 10 samples")
    assert top_step(app)["type"] == "move-many" and len(top_step(app)["list"]) == 5
    assert samples(app, sid, "Snare Top") == [int(t * SR) for t in OTHER]


def test_without_a_range_delete_and_the_nudge_keys_keep_the_single_hit_behaviour(project):
    app, sid, folder = project
    js(app, "() => { S.sel = null; S.playhead = 0; lnSetSel(null); setView(0, 6); }")
    undo_n = js(app, "() => S.undo.length")
    app.keyboard.press("Tab")                                               # the first hit (0.3 s)
    app.keyboard.press("Delete")
    edits_done(app)
    wait_until(lambda: samples(app, sid, "Kick In") == [int(t * SR) for t in sorted(INSIDE + OUTSIDE[1:])], msg="only the selected hit is gone")
    step = top_step(app)
    assert js(app, "() => S.undo.length") == undo_n + 1 and step["type"] == "del"
    app.keyboard.press("Z")
    edits_done(app)
    wait_until(lambda: samples(app, sid, "Kick In") == [int(t * SR) for t in sorted(INSIDE + OUTSIDE)], msg="the undo puts it back")
    js(app, "() => { S.sel = null; S.playhead = 0; }")                      # undoReveal selected the restored hit
    app.keyboard.press("Tab")
    app.keyboard.press(".")                                                 # nudge the selected hit only
    edits_done(app)
    wait_until(lambda: js(app, "(sr) => Math.round(S.events['Kick In'][S.sel.index].t * sr)", SR) == int(0.3 * SR) + 1,
               msg="the selected hit moved by one sample")
    assert top_step(app)["type"] == "move"
    assert sorted(round(e["t"] * SR) for e in invoke(app, "events_get", sid=sid, name="Kick In")["events"]) \
        == [int(t * SR) + (1 if t == 0.3 else 0) for t in sorted(INSIDE + OUTSIDE)]


def test_a_range_without_hits_takes_no_step(project):
    app, sid, folder = project
    js(app, "() => { S.sel = null; lnSetSel('Kick In', 2.6, 2.8); }")       # an empty stretch of the lane
    undo_n = js(app, "() => S.undo.length")
    app.keyboard.press("Delete")
    edits_done(app)
    wait_until(lambda: "no hits in the range" in js(app, "() => document.querySelector('#stMsg').textContent"), msg="the guard message")
    assert js(app, "() => S.undo.length") == undo_n                         # no step: the track confirm must not fire either
    assert samples(app, sid, "Kick In") == [int(t * SR) for t in sorted(INSIDE + OUTSIDE)]
    app.keyboard.press(".")
    edits_done(app)
    wait_until(lambda: "no hits in the range" in js(app, "() => document.querySelector('#stMsg').textContent"), msg="the guard message")
    assert js(app, "() => S.undo.length") == undo_n
    assert samples(app, sid, "Kick In") == [int(t * SR) for t in sorted(INSIDE + OUTSIDE)]
