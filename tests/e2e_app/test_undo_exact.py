"""#154 (campaign #117 S2): undo and redo restore the exact event state.

The campaign found that after add, delete, nudge, move and a role change the undo (and the
redo) left an event state whose hash differed from the state before the step, in 4 of 4 steps,
without naming the field. This test does the same steps through the page (so the page pushes
its undo steps) and compares the full service event state field by field after every undo and
redo; the failure message names the differing field.
"""
from app_support import edits_done, invoke, menu, open_project, wait_until
from test_menus import with_hits  # noqa: F401
from test_tempo_map import js  # noqa: F401

MISSING = object()


def service_events(app, sid, name="Kick In"):
    return invoke(app, "events_get", sid=sid, name=name)["events"]


def field_diffs(got, want):
    """The differing fields of two event lists, as the failure message: `events[i].key: got != want`.
    Field presence counts: a key the one list has and the other has not is a difference too."""
    if len(got) != len(want):
        return [f"event count {len(got)} != {len(want)}"]
    out = []
    for i, (g, w) in enumerate(zip(got, want)):
        for k in sorted(set(g) | set(w)):
            gv = g[k] if k in g else MISSING
            wv = w[k] if k in w else MISSING
            if gv != wv or (k in g) != (k in w):
                out.append(f"events[{i}].{k}: {gv!r} != {wv!r}")
    return out


def assert_exact(got, want, when):
    diffs = field_diffs(got, want)
    assert not diffs, f"{when}: the event state differs:\n  " + "\n  ".join(diffs) + f"\n  got: {got}\n  want: {want}"


def wait_step(app, kind):
    wait_until(lambda: js(app, "() => S.undo[S.undo.length - 1]?.type") == kind, msg=f"one {kind} step")
    edits_done(app)


def wait_moved(app, expr, n0, msg):
    """The stack length drops by one, or the failure names the stacks and the last message."""
    def ok():
        return js(app, expr) == n0 - 1
    try:
        wait_until(ok, timeout=10, msg=msg)
    except AssertionError:
        diag = js(app, "() => ({undo: S.undo.map(o => o.type), redo: S.redo.map(o => o.type), msg: document.querySelector('#stMsg')?.textContent})")
        raise AssertionError(f"{msg}: the stacks did not move: {diag}") from None


def snap(app, sid):
    edits_done(app)
    return service_events(app, sid)


def test_undo_and_redo_restore_the_exact_event_state(app, make_project):
    sid = open_project(app, make_project(seconds=8.0))
    with_hits(app, sid, {"Kick In": [1.0, 3.0, 5.0]})
    invoke(app, "project_save", sid=sid)   # the baseline every undo must come back to

    states = {"start": snap(app, sid)}
    # the role change first: it reloads the project (a re-pick can rewrite the hits), so the page
    # keeps no steps across it — the campaign's four edit steps come after it and stay undoable
    js(app, "() => setTrackRole('Kick In', 'tom')")
    wait_step(app, "role")
    states["after role"] = snap(app, sid)

    # the four edit steps of the campaign check, each through the page (its undo step)
    js(app, "() => addHitAt('Kick In', 2.0, false)")
    wait_step(app, "add")
    states["after add"] = snap(app, sid)

    js(app, "() => setHitTime('Kick In', 0, 1.5)")   # move (the info line's move)
    wait_step(app, "move")
    states["after move"] = snap(app, sid)

    js(app, "() => { setSelTrack('Kick In'); S.sel = { track: 'Kick In', index: 1 }; return nudgeUnits(1); }")
    wait_step(app, "move")
    states["after nudge"] = snap(app, sid)

    js(app, "() => edDelete('Kick In', S.events['Kick In'].at(-1).id)")
    wait_step(app, "del")
    states["after delete"] = snap(app, sid)

    # undo every step; after each one the state is the one before the step, field by field
    for kind, before in (("del", "after nudge"), ("move", "after move"),
                         ("move", "after add"), ("add", "after role"), ("role", "start")):
        n0 = js(app, "() => S.undo.length")
        menu(app, "undo")
        wait_moved(app, "() => S.undo.length", n0, f"the undo of {kind}")
        edits_done(app)
        assert_exact(snap(app, sid), states[before], f"undo {kind}")

    # and the redo of every step (the redo stack is LIFO: role, add, the set-time move, the nudge, the delete)
    for kind, after in (("role", "after role"), ("add", "after add"), ("move", "after move"),
                        ("move", "after nudge"), ("del", "after delete")):
        r0 = js(app, "() => S.redo.length")
        menu(app, "redo")
        wait_moved(app, "() => S.redo.length", r0, f"the redo of {kind}")
        edits_done(app)
        assert_exact(snap(app, sid), states[after], f"redo {kind}")
