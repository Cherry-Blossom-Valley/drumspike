"""R05 (#237, ticket #251): undo restores the event exactly. A hand velocity marks the hit
`edited: true` and a move stores `orig`; the undo used to remember only the number and left the
fields on the hit, so the document stayed different from the saved one and a re-pick kept
treating the hit as edited. The page now sends the field presence the hit had before the edit
(`was` on the inverse op) and the service puts the hit back byte-identical:
undo(edit(doc)) == doc, the dirty flag clears, and a re-pick treats the hit as before the edit.
"""
from app_support import edits_done, invoke, menu, open_project, wait_until
from test_menus import with_hits  # noqa: F401
from test_tempo_map import js  # noqa: F401
from test_velocity_lane import setup, vel_xy  # noqa: F401

TS = (1.0, 3.0, 5.0)
PLAIN = [{"t": 1.0, "v": 90}, {"t": 3.0, "v": 90}, {"t": 5.0, "v": 90}]


def service_events(app, sid, name="Kick In"):
    return invoke(app, "events_get", sid=sid, name=name)["events"]


def deep(app, sid, want, name="Kick In"):
    """The service's hits equal the expected document, key for key (no extra `edited`/`orig`)."""
    got = service_events(app, sid, name)
    assert [{k: e[k] for k in e if k != "id"} for e in got] == want, got


def load(app, sid, events):
    invoke(app, "events_set", sid=sid, name="Kick In", events=events)
    app.evaluate("async () => { await loadProject(); }")
    app.wait_for_function(
        "w => (S.events['Kick In'] || []).every((e, i) => e.t === w[i].t && e.v === w[i].v"
        " && !!e.bleed === !!w[i].bleed && !!e.roll === !!w[i].roll && (e.orig ?? null) === (w[i].orig ?? null))",
        arg=events,
    )
    invoke(app, "project_save", sid=sid)   # the baseline the undo must come back to


def drag_stalk(app, t, dy=-40):
    x, y = vel_xy(app, "Kick In", t)
    app.mouse.move(x, y)
    app.mouse.down()
    app.mouse.move(x, y + dy, steps=3)
    app.mouse.up()


def wait_setv(app, kind="setv"):
    wait_until(lambda: js(app, "() => S.undo[S.undo.length - 1]?.type") == kind, msg=f"one {kind} step")
    edits_done(app)


def test_undo_of_a_hand_velocity_restores_the_event_exactly(app, make_project):
    sid = setup(app, make_project)
    load(app, sid, PLAIN)
    drag_stalk(app, 3.0)
    wait_setv(app)
    v = service_events(app, sid)[1]["v"]
    assert v != 90
    deep(app, sid, [{"t": 1.0, "v": 90}, {"t": 3.0, "v": v, "edited": True}, {"t": 5.0, "v": 90}])
    menu(app, "undo")
    edits_done(app)
    wait_until(lambda: service_events(app, sid)[1]["v"] == 90, msg="undo restores the velocity")
    deep(app, sid, PLAIN)   # no `edited` key left: the document is the saved one again
    menu(app, "redo")
    edits_done(app)
    wait_until(lambda: service_events(app, sid)[1]["v"] == v, msg="redo restores the velocity")
    deep(app, sid, [{"t": 1.0, "v": 90}, {"t": 3.0, "v": v, "edited": True}, {"t": 5.0, "v": 90}])


def test_undo_of_a_velocity_on_a_bleed_hit_brings_the_mark_back(app, make_project):
    sid = setup(app, make_project)
    load(app, sid, [{"t": 1.0, "v": 90}, {"t": 3.0, "v": 90, "bleed": True}, {"t": 5.0, "v": 90}])
    drag_stalk(app, 3.0)
    wait_setv(app)
    v = service_events(app, sid)[1]["v"]
    deep(app, sid, [{"t": 1.0, "v": 90}, {"t": 3.0, "v": v, "edited": True}, {"t": 5.0, "v": 90}])
    menu(app, "undo")
    edits_done(app)
    wait_until(lambda: service_events(app, sid)[1]["v"] == 90, msg="undo restores the velocity")
    deep(app, sid, [{"t": 1.0, "v": 90}, {"t": 3.0, "v": 90, "bleed": True}, {"t": 5.0, "v": 90}])
    assert invoke(app, "project_clean_check", sid=sid)["dirty"] is False


def test_undo_of_a_move_takes_orig_off_and_restores_a_former_orig(app, make_project):
    sid = setup(app, make_project)
    load(app, sid, PLAIN)
    js(app, "() => { S.sel = { track: 'Kick In', index: 1 }; }")
    undo_n = js(app, "() => S.undo.length")
    app.keyboard.press("ArrowRight")   # nudge later by the nudge value (one move step)
    edits_done(app)
    wait_until(lambda: js(app, "() => S.undo.length") == undo_n + 1, msg="the move step")
    edits_done(app)
    deep(app, sid, [{"t": 1.0, "v": 90}, {"t": 3.0 + 1.0 / 48000, "v": 90, "orig": 3.0}, {"t": 5.0, "v": 90}])
    menu(app, "undo")
    edits_done(app)
    wait_until(lambda: js(app, "() => S.undo.length") == undo_n, msg="the undo ran")
    deep(app, sid, PLAIN)
    menu(app, "redo")
    edits_done(app)
    wait_until(lambda: service_events(app, sid)[1].get("orig") == 3.0, msg="redo restores the orig")
    deep(app, sid, [{"t": 1.0, "v": 90}, {"t": 3.0 + 1.0 / 48000, "v": 90, "orig": 3.0}, {"t": 5.0, "v": 90}])
    # a move of an already moved hit: the undo brings the former `orig` value back
    load(app, sid, [{"t": 1.0, "v": 90}, {"t": 3.0, "v": 90, "orig": 2.5}, {"t": 5.0, "v": 90}])
    js(app, "() => { S.sel = { track: 'Kick In', index: 1 }; }")
    undo_n = js(app, "() => S.undo.length")
    app.keyboard.press("ArrowRight")
    edits_done(app)
    deep(app, sid, [{"t": 1.0, "v": 90}, {"t": 3.0 + 1.0 / 48000, "v": 90, "orig": 2.5}, {"t": 5.0, "v": 90}])
    menu(app, "undo")
    edits_done(app)
    wait_until(lambda: service_events(app, sid)[1].get("orig") == 2.5, msg="the former orig is back")
    deep(app, sid, [{"t": 1.0, "v": 90}, {"t": 3.0, "v": 90, "orig": 2.5}, {"t": 5.0, "v": 90}])


def test_undo_of_a_velocity_gesture_over_several_hits_restores_every_hit(app, make_project):
    sid = setup(app, make_project)
    load(app, sid, [{"t": 1.0, "v": 90, "roll": True}, {"t": 3.0, "v": 90}, {"t": 5.0, "v": 90}])
    x0, y0 = vel_xy(app, "Kick In", 1.0, 2 / 22)
    x1, y1 = vel_xy(app, "Kick In", 5.0, 20 / 22)
    app.keyboard.down("Alt")
    app.mouse.move(x0, y0)
    app.mouse.down()
    app.mouse.move(x1, y1, steps=8)
    app.mouse.up()
    app.keyboard.up("Alt")
    wait_setv(app, "setv-many")
    vs = [e["v"] for e in service_events(app, sid)]
    assert vs[0] > 100 and vs[2] < 30, vs
    menu(app, "undo")
    edits_done(app)
    wait_until(lambda: [e["v"] for e in service_events(app, sid)] == [90, 90, 90], msg="undo restores the velocities")
    deep(app, sid, [{"t": 1.0, "v": 90, "roll": True}, {"t": 3.0, "v": 90}, {"t": 5.0, "v": 90}])
    assert invoke(app, "project_clean_check", sid=sid)["dirty"] is False


def test_after_the_undo_a_repick_treats_the_hit_as_before_the_edit(app, make_project):
    sid = setup(app, make_project)
    load(app, sid, PLAIN)
    drag_stalk(app, 3.0)
    wait_setv(app)
    assert js(app, "() => S.events['Kick In'].filter(isKept).length") == 1, "the hand velocity is kept"
    menu(app, "undo")
    edits_done(app)
    wait_until(lambda: invoke(app, "project_clean_check", sid=sid)["dirty"] is False, msg="clean again")
    assert js(app, "() => S.events['Kick In'].filter(isKept).length") == 0, "the undo takes the edit off"
    assert all("edited" not in e for e in service_events(app, sid))
