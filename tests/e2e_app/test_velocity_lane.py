"""P4-40f: the velocity lane. A thin lane under each spike lane (View > Velocity Lane), one stalk per
hit (height = velocity 1 to 127), grey stalks for hits that leave the output, the floor and the ceiling
lines of the loudness settings. Drag a stalk (one setv step), Option-drag draws a straight line over a
range, Cmd-drag freehand; one gesture is one undo step (one events_edit call) and a hand velocity
survives Recompute Velocities. Each test has a planted-fault check (the stash test of the card)."""
from app_support import invoke, menu, open_project, wait_until
from test_menus import with_hits
from test_tempo_map import js  # noqa: F401
from test_follow_strip import IDLE


def vel_xy(app, name, t, frac=0.5):
    """Client coordinates of time t on the velocity lane of a track (frac: 0 top, 1 bottom)."""
    return app.evaluate(
        """([n, t, f]) => { const r = document.querySelector(`.track[data-name="${n}"] canvas.vel`).getBoundingClientRect();
          return [GEO.laneX + tToX(t, GEO.laneW), r.top + f * r.height]; }""",
        [name, t, frac],
    )


def vel_y(app, name, v):
    """Client y of velocity v on the velocity lane of a track (the lane's own mapping, vlYOf)."""
    return app.evaluate(
        """([n, v]) => { const c = document.querySelector(`.track[data-name="${n}"] canvas.vel`);
          const r = c.getBoundingClientRect(), [W, H] = laneSize(c); return r.top + vlYOf(v, H); }""",
        [name, v],
    )


def setup(app, make_project, hits=(1.0, 3.0, 5.0)):
    sid = open_project(app, make_project(seconds=8.0))
    with_hits(app, sid, {"Kick In": list(hits)})
    app.evaluate("() => { if (!VL.on) vlToggle(); }")
    app.wait_for_function(IDLE)
    return sid


def set_velocities(app, sid, vs, ts=(1.0, 3.0, 5.0)):
    """The hits of Kick In with the velocities vs (the service and the page)."""
    invoke(app, "events_set", sid=sid, name="Kick In", events=[{"t": t, "v": v, "id": f"K{i}"} for i, (t, v) in enumerate(zip(ts, vs))])
    app.evaluate("async () => { await loadProject(); }")
    app.wait_for_function("v => (S.events['Kick In'] || []).map(e => e.v).join() === v.join()", arg=list(vs))


def service_tv(app, sid, name="Kick In"):
    return [e["v"] for e in invoke(app, "events_get", sid=sid, name=name)["events"]]


def test_the_line_tool_draws_over_a_range_as_one_undo_step(app, make_project):
    sid = setup(app, make_project)
    u0 = js(app, "() => S.undo.length")
    app.evaluate("() => { window.__calls = []; const c = B.eventsEdit; B.eventsEdit = (...a) => { window.__calls.push(a[2]); return c(...a); }; }")
    x0, y0 = vel_xy(app, "Kick In", 1.0, 2 / 22)    # near the top: a high velocity
    x1, y1 = vel_xy(app, "Kick In", 5.0, 20 / 22)   # near the bottom: a low one
    app.keyboard.down("Alt")
    app.mouse.move(x0, y0)
    app.mouse.down()
    app.mouse.move(x1, y1, steps=8)
    app.mouse.up()
    app.keyboard.up("Alt")
    wait_until(lambda: js(app, "() => window.__calls.length") == 1 and len(js(app, "() => window.__calls[0] || []")) == 3, msg="one events_edit call with one op per hit")
    wait_until(lambda: service_tv(app, sid) == [e["v"] for e in js(app, "() => S.events['Kick In']")], msg="the service stored the line")
    vs = service_tv(app, sid)
    assert vs[0] > 100 and vs[2] < 30, vs
    assert abs(vs[1] - (vs[0] + vs[2]) / 2) <= 2, (vs, "the middle hit is on the straight line")
    assert js(app, "() => S.undo.length") == u0 + 1 and js(app, "() => S.undo[S.undo.length - 1].type") == "setv-many"
    assert js(app, "() => S.undo[S.undo.length - 1].label") == "Set Velocity (Kick In, 3 hits)"
    menu(app, "undo")   # one undo takes the whole gesture back
    wait_until(lambda: service_tv(app, sid) == [90, 90, 90], msg="undo restores the velocities")


def test_a_stalk_drag_is_one_step_and_survives_recompute(app, make_project):
    sid = setup(app, make_project)
    u0 = js(app, "() => S.undo.length")
    x, y = vel_xy(app, "Kick In", 3.0)
    app.mouse.move(x, y)
    app.mouse.down()
    app.mouse.move(x, y - 40, steps=3)   # up to the top of the 80 px lane: a velocity above 90
    app.mouse.up()
    wait_until(lambda: js(app, "() => S.undo.length") == u0 + 1, msg="one gesture, one step")
    assert js(app, "() => S.undo[S.undo.length - 1].type") == "setv"
    v = js(app, "() => S.events['Kick In'][1].v")
    assert v > 90 and service_tv(app, sid) == [90, v, 90]
    want = invoke(app, "velocities_recompute", sid=sid, name="Kick In",
                  events=js(app, "() => S.events['Kick In']"))["events"]
    assert [e["v"] for e in want][1] == v, "the hand velocity survives Recompute Velocities"
    assert [e["v"] for e in want] != [v, v, v]
    menu(app, "undo")
    wait_until(lambda: service_tv(app, sid) == [90, 90, 90], msg="undo restores")


def test_stalks_that_leave_the_output_are_grey_and_the_lines_show_the_floor_and_the_ceiling(app, make_project):
    setup(app, make_project)
    app.evaluate("""() => { const e = S.events['Kick In'][0]; e.cut = true; invalidateHits('Kick In'); }""")
    app.wait_for_function(IDLE)
    # the stalk of the cut hit: the pixel above its base is the grey (planted fault: the colour falls back to the track colour)
    rgb = app.evaluate(
        """() => { const c = document.querySelector('.track[data-name="Kick In"] canvas.vel');
          const ctx = c.getContext('2d'), [W, H] = laneSize(c);
          const x = Math.round(tToX(S.events['Kick In'][0].t, W));
          const p = ctx.getImageData(x, H - 3, 1, 1).data; return [p[0], p[1], p[2]]; }"""
    )
    assert rgb == [74, 80, 92], rgb
    # the second stalk is in the track colour, the floor and the ceiling lines are drawn (dashed, faint)
    rgb2 = app.evaluate(
        """() => { const c = document.querySelector('.track[data-name="Kick In"] canvas.vel');
          const ctx = c.getContext('2d'), [W, H] = laneSize(c);
          const x = Math.round(tToX(S.events['Kick In'][1].t, W));
          const p = ctx.getImageData(x, H - 3, 1, 1).data; return [p[0], p[1], p[2]]; }"""
    )
    assert rgb2 != [74, 80, 92] and rgb2 != [0, 0, 0], rgb2
    line = app.evaluate(
        """() => { const c = document.querySelector('.track[data-name="Kick In"] canvas.vel');
          const ctx = c.getContext('2d'), [W, H] = laneSize(c);
          const y = vlYOf(20, H); let n = 0;
          for (let x = 0; x < W; x += 2) { const p = ctx.getImageData(x, y, 1, 1).data; if (p[0] !== 0 || p[1] !== 0) n++; }
          return n; }"""
    )
    assert line > 0, "the floor line is on the lane"   # the dashed line, scanned across the dashes


def test_the_lane_starts_off_and_the_menu_toggles_it(app, make_project):
    sid = open_project(app, make_project(seconds=8.0))
    invoke(app, "app_settings_set", key="velocity_lane", value=False)   # a earlier gesture of the session saved it on
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0")
    assert js(app, "() => VL.on") is False
    assert js(app, "() => document.querySelector('.track[data-name=\\'Kick In\\'] canvas.vel')") is None   # the lane is off: no vel canvas
    menu(app, "velocity_lane")
    wait_until(lambda: js(app, "() => VL.on") is True, msg="the menu turns the lane on")
    assert js(app, "() => getComputedStyle(document.querySelector('.track[data-name=\\'Kick In\\'] canvas.vel')).display") == "block"
    menu(app, "velocity_lane")
    wait_until(lambda: js(app, "() => VL.on") is False, msg="and off again")


def lane_h(app, name="Kick In"):
    return app.evaluate(f"() => laneSize(document.querySelector('.track[data-name=\"{name}\"] canvas.vel'))[1]")


def test_the_lane_is_80_px_by_default_and_dragging_down_grows_it(app, make_project):
    # #208/#437: the height is an app setting; none stored means the 80 px default. The handle is the
    # lane's bottom border: a drag down makes the lane taller, up smaller (like the wave lane's row drag).
    open_project(app, make_project(seconds=8.0))
    invoke(app, "app_settings_set", key="velocity_lane_height", value=None)
    invoke(app, "app_settings_set", key="velocity_lane", value=True)
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0")
    assert js(app, "() => VL.h") == 80 and js(app, "() => VL.on") is True, "the default height is 80 px"
    wait_until(lambda: lane_h(app) == 80, msg="the lane is drawn at 80 px")
    # after the reload GEO can still hold the pre-layout lane place: the drag then starts on the track head (flake, 0.9.1)
    wait_until(
        lambda: app.evaluate(
            """() => { const r = document.querySelector('.track[data-name="Kick In"] canvas.vel').getBoundingClientRect();
              const x = GEO.laneX + tToX(6.5, GEO.laneW); return GEO.ok && x > r.left && x < r.right; }"""
        ),
        msg="the lane place is measured",
    )
    x, y = vel_xy(app, "Kick In", 6.5, 1.0)   # the bottom border of the lane
    y = y - 1
    app.mouse.move(x, y)
    app.mouse.down()
    app.mouse.move(x, y + 40, steps=4)   # down: the lane grows by the drag
    app.mouse.up()
    wait_until(lambda: lane_h(app) == 120, msg="a 40 px drag down makes the lane 120 px")
    assert js(app, "() => VL.h") == 120
    x, y = vel_xy(app, "Kick In", 6.5, 1.0)   # the bottom border moved down with the lane
    y = y - 1
    app.mouse.move(x, y)
    app.mouse.down()
    app.mouse.move(x, y - 40, steps=4)   # up: the lane shrinks again
    app.mouse.up()
    wait_until(lambda: lane_h(app) == 80, msg="a 40 px drag up makes the lane 80 px again")
    assert js(app, "() => VL.h") == 80
    invoke(app, "app_settings_set", key="velocity_lane_height", value=None)   # the default for the other tests


def test_hover_names_the_velocity_and_the_stalk_under_the_pointer_is_the_hit(app, make_project):
    setup(app, make_project)
    x, y = vel_xy(app, "Kick In", 3.0, 0.5)
    app.mouse.move(x, y)
    wait_until(lambda: js(app, "() => VL.hover && VL.hover.track") == "Kick In", msg="the hover follows the stalk")
    assert js(app, "() => S.events['Kick In'].find(e => e.id === VL.hover.id).v") == 90
    assert js(app, "() => S.events['Kick In'][1].id") == js(app, "() => VL.hover.id")
    app.mouse.move(x + 60, y)   # off the stalk: the label goes
    wait_until(lambda: js(app, "() => VL.hover") is None, msg="the label goes off with the stalk")


def test_a_stalk_dragged_to_100_is_100_in_the_events_and_in_the_midi_note_on(app, make_project):
    from test_infoline import midi_note_ons
    sid = setup(app, make_project)
    x, y = vel_xy(app, "Kick In", 3.0)
    app.mouse.move(x, y)
    app.mouse.down()
    app.mouse.move(x, vel_y(app, "Kick In", 100), steps=4)
    app.mouse.up()
    wait_until(lambda: service_tv(app, sid) == [90, 100, 90], msg="the service stored 100")
    assert js(app, "() => S.events['Kick In'][1].v") == 100
    r = invoke(app, "export_midi", sid=sid, name="Kick In")
    notes = sorted(v for _, v in midi_note_ons(r["path"]))
    assert notes == [90, 90, 100], notes   # the MIDI note-on of the stalk is 100
    menu(app, "undo")
    wait_until(lambda: service_tv(app, sid) == [90, 90, 90], msg="undo restores")


def test_an_option_line_over_five_hits_is_a_monotonic_ramp(app, make_project):
    sid = setup(app, make_project, hits=(1.0, 2.0, 3.0, 4.0, 5.0))
    x0, y0 = vel_xy(app, "Kick In", 1.0, 0.05)
    x1, y1 = vel_xy(app, "Kick In", 5.0, 0.95)
    app.keyboard.down("Alt")
    app.mouse.move(x0, y0)
    app.mouse.down()
    app.mouse.move(x1, y1, steps=8)
    app.mouse.up()
    app.keyboard.up("Alt")
    wait_until(lambda: service_tv(app, sid) == [e["v"] for e in js(app, "() => S.events['Kick In']")], msg="the service stored the line")
    vs = service_tv(app, sid)
    assert len(vs) == 5 and vs[0] > vs[-1] + 60, vs
    assert all(a >= b for a, b in zip(vs, vs[1:])), (vs, "the ramp falls from the first hit to the last")


def test_dragging_a_selected_stalk_scales_the_selected_hits_by_one_ratio_and_undo_restores_them(app, make_project):
    sid = setup(app, make_project)
    set_velocities(app, sid, [60, 90, 120])
    # the range selection of the Select tool covers the three hits
    app.evaluate("() => { LN.sel = { track: 'Kick In', t0: 0.5, t1: 5.5 }; invalidateHits('Kick In'); }")
    x, y = vel_xy(app, "Kick In", 1.0)
    app.mouse.move(x, y)
    app.mouse.down()
    app.mouse.move(x, vel_y(app, "Kick In", 30), steps=4)   # the first hit: 60 to about 30
    app.mouse.up()
    wait_until(lambda: js(app, "() => S.undo.length") > 0 and js(app, "() => S.undo[S.undo.length - 1].type") == "setv-many", msg="one scaled gesture")
    vs = service_tv(app, sid)
    ratio = vs[0] / 60
    assert 0.4 < ratio < 0.6, vs
    assert abs(vs[1] - round(90 * ratio)) <= 1 and abs(vs[2] - round(120 * ratio)) <= 1, (vs, ratio, "the three hits keep their ratio")
    menu(app, "undo")   # one undo takes the whole scale back
    wait_until(lambda: service_tv(app, sid) == [60, 90, 120], msg="undo restores all three hits")
