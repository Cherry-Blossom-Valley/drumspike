"""P4-6a: range selection and Learn marks (owner request 2026-10-12, docs/migration/concept-learn-mode.md). A drag in the
wave lane selects a range on that track (a click is still the playhead, the other pan gestures stay); K (Hits > Learn
This) and Shift+K (Forget This) turn the range or the selected hit into a mark that the service keeps in the track's `learn`
object; the lanes draw the band, the mark bars and the example ticks; every step is one undo step; a follower marks its
source. No filter yet (P4-6b)."""
import json
from pathlib import Path

import pytest
from playwright.sync_api import expect

from app_support import edits_done, inspect_track, invoke, menu, open_project, wait_until
from helpers import SR, synth_track, write_wav


HITS = [0.5, 1.0, 1.5, 2.0, 2.5, 3.0]     # the kicks with a stored hit
EXTRA = 3.5                               # a soft kick in the audio with no stored hit
DUR = 5.0
GREEN, RED = (0x4c, 0xc3, 0x8a), (0xe5, 0x53, 0x4b)


def js(page, expr, arg=None):
    return page.evaluate(expr, arg)


def build(make_project, tmp_path, name="Learn"):
    src = tmp_path / f"src_{name}"
    src.mkdir()
    on = HITS + [EXTRA]
    x, _ = synth_track(on, DUR, gains_db=[-3.0] * len(HITS) + [-12.0])
    sub, _ = synth_track(HITS, DUR, gains_db=[-20.0] * len(HITS))
    events = [{"t": round(t * SR) / SR, "v": 100} for t in HITS]
    specs = [("Kick In", write_wav(src / "Kick In.wav", x), "kick", 36, events),
             ("Kick Sub", write_wav(src / "Kick Sub.wav", sub), "kick", 36, None),
             ("Snare", write_wav(src / "Snare.wav", x), "snare", 38, None)]
    return make_project.builder.build_project(tmp_path / f"projects_{name}", name, specs, "copied")


@pytest.fixture
def lrn(app, make_project, tmp_path):
    folder = build(make_project, tmp_path)
    sid = open_project(app, folder)
    app.evaluate("() => { window.__mx = []; for (const t of ['mousedown', 'mousemove', 'mouseup']) window.addEventListener(t, e => window.__mx.push([t, e.clientX, e.clientY]), true); }")
    return app, sid, Path(folder)


def state(page):
    return js(page, "() => DSLearn.state()")


def settled(page):
    page.wait_for_function("() => DSLearn.state().pending === 0")


def learn_of(page, sid, name="Kick In"):
    return invoke(page, "tracks_get", sid=sid)["tracks"][name].get("learn")


def marks(page, sid, name="Kick In"):
    return (learn_of(page, sid, name) or {}).get("marks", [])


def lane_box(page, name, lane="wave"):
    return page.locator(f'.track[data-name="{name}"] .{lane}').bounding_box()


def px(page, t):
    """The page x of the time t in a lane."""
    return js(page, "t => GEO.laneX + tToX(t, laneWidth())", t)


def drag(page, name, a, b, lane="wave"):
    """A real mouse drag in the lane of a track from time a to time b; returns the x of the press and the release."""
    box = lane_box(page, name, lane)
    y = box["y"] + box["height"] / 2
    js(page, "() => { window.__mx.length = 0; }")
    page.mouse.move(px(page, a), y)
    page.mouse.down()
    page.mouse.move((px(page, a) + px(page, b)) / 2, y, steps=4)
    page.mouse.move(px(page, b), y, steps=4)
    page.mouse.up()
    mx = js(page, "() => window.__mx")
    return [m[1] for m in mx if m[0] == "mousedown"][-1], [m[1] for m in mx if m[0] == "mouseup"][-1]


def t_of(page, x):
    return js(page, "x => xToT(x - GEO.laneX, laneWidth())", x)


def pixel(page, sel, x, y):
    return js(page, """([sel, x, y]) => { const c = document.querySelector(sel), d = window.devicePixelRatio || 1;
      return [...c.getContext('2d').getImageData(Math.round(x * d), Math.round(y * d), 1, 1).data]; }""", [sel, x, y])


def near(got, rgb, tol=40):
    return all(abs(a - b) <= tol for a, b in zip(got[:3], rgb))


def mark(page, key="K"):
    page.keyboard.press("Shift+K" if key == "Shift+K" else "K")
    settled(page)


def select(page, name, a, b):
    drag(page, name, a, b)
    page.wait_for_function("() => DSLearn.state().sel !== null")


def msg(page):
    return page.locator("#stMsg").inner_text()


def hit_x(page, name, i):
    return px(page, js(page, "([n, i]) => S.events[n][i].t", [name, i]))


# ---------------------------------------------------------------------------- range selection

def test_a_drag_in_the_wave_lane_selects_the_exact_range_and_a_click_stays_the_playhead(lrn):
    page, sid, _ = lrn
    page.evaluate("() => { S.playhead = 4; S.followHold = true; invalidate(F.HEAD); }")
    view0 = js(page, "() => ({...S.view})")
    x0, x1 = drag(page, "Kick In", 0.9, 1.6)
    st = state(page)
    assert st["sel"]["track"] == "Kick In"
    assert st["sel"]["t0"] == pytest.approx(t_of(page, x0), abs=1e-9)    # the times of the two mouse positions
    assert st["sel"]["t1"] == pytest.approx(t_of(page, x1), abs=1e-9)
    assert st["sel"]["t0"] == pytest.approx(0.9, abs=0.03) and st["sel"]["t1"] == pytest.approx(1.6, abs=0.03)
    assert js(page, "() => S.view") == view0, "a drag in the wave lane no longer pans"
    assert js(page, "() => S.selTrack") == "Kick In"
    # the info line: the range and its hit count (hits at 1.0 and 1.5)
    t0, t1 = st["sel"]["t0"], st["sel"]["t1"]
    assert st["text"] == f"selection {t0:.3f} to {t1:.3f} s ({t1 - t0:.3f} s), 2 hits"
    expect(page.locator("#ilSel")).to_have_text(st["text"])
    # the band: the accent colour at 20 % in the wave lane and the spike lane of that track, none on another track
    mid = (px(page, 1.3125) - GEO_X(page))   # between the 8th and 16th lines of the grid (1.25 s is an 8th line: its grey line would sit on the sample)
    for lane in ("wave", "spike"):
        got = pixel(page, f'.track[data-name="Kick In"] .{lane}', mid, 8)
        assert got[3] > 0 and got[2] > got[0], (lane, got)               # blue-ish and visible
    page.evaluate("() => redrawAll()")                                    # every lane drawn again: only the selected track has the band
    for other in ("Snare", "Kick Sub"):
        for lane in ("wave", "spike"):
            got = pixel(page, f'.track[data-name="{other}"] .{lane}', mid, 8)
            assert not (got[3] > 0 and got[2] > got[0] and near(got, (0x4d, 0xa3, 0xff), 90)), (other, lane, got)
    # the ruler has two edge marks
    ex = [pixel(page, "#ruler", px(page, t) - GEO_RULER(page), 1) for t in (t0, t1)]
    assert all(near(p, (0x4d, 0xa3, 0xff), 60) for p in ex), ex
    # Escape clears it
    page.keyboard.press("Escape")
    assert state(page)["sel"] is None and not page.locator("#ilSel").is_visible()
    # a click sets the playhead and clears a selection
    select(page, "Kick In", 2.0, 2.6)
    box = lane_box(page, "Kick In")
    page.mouse.click(px(page, 3.2), box["y"] + box["height"] / 2)
    assert state(page)["sel"] is None
    assert js(page, "() => S.playhead") == pytest.approx(3.2, abs=0.02)


def GEO_X(page):
    return js(page, "() => GEO.laneX")


def GEO_RULER(page):
    return js(page, "() => GEO.rulerX")


def test_the_other_pan_gestures_stay_and_the_selection_stays_on_its_track(lrn):
    page, sid, _ = lrn
    page.evaluate("() => { S.view = viewClamp(1, 3); invalidate(F.VIEW); }")
    v0 = js(page, "() => S.view.t0")
    drag(page, "Kick In", 2.75, 1.75, lane="spike")        # an empty part of the spike lane pans
    assert js(page, "() => S.view.t0") != pytest.approx(v0)
    assert state(page)["sel"] is None
    page.evaluate("() => { S.view = viewClamp(0, 5); invalidate(F.VIEW); }")
    select(page, "Kick In", 1.2, 2.2)
    # the selection stays where it began: a drag that ends over another track's lane changes only the time
    box = lane_box(page, "Snare")
    page.mouse.move(px(page, 3.0), lane_box(page, "Kick In")["y"] + 20)
    page.mouse.down()
    page.mouse.move(px(page, 3.5), box["y"] + 20, steps=5)
    page.mouse.up()
    assert state(page)["sel"]["track"] == "Kick In"
    # Shift+wheel pans
    page.evaluate("() => { S.view = viewClamp(1, 3); invalidate(F.VIEW); }")
    v1 = js(page, "() => S.view.t0")
    b = lane_box(page, "Kick In")
    page.mouse.move(b["x"] + 100, b["y"] + 20)
    page.keyboard.down("Shift")
    page.mouse.wheel(0, 120)
    page.keyboard.up("Shift")
    wait_until(lambda: js(page, "() => S.view.t0") != pytest.approx(v1), msg="Shift+wheel pans")


# ---------------------------------------------------------------------------- marks

def test_k_on_a_tight_range_gives_one_example_and_draws_the_mark(lrn):
    page, sid, _ = lrn
    select(page, "Kick In", 0.95, 1.05)
    mark(page)
    learn = learn_of(page, sid)
    # P4-6c (controller 2026-10-13): K alone never turns the filter on; the threshold follows the examples (auto)
    assert len(learn["marks"]) == 1 and learn["on"] is False and learn["auto"] is True and learn["method"] == "patch-2"
    m = learn["marks"][0]
    assert m["id"] == "m1" and m["kind"] == "this" and m["at"] == [pytest.approx(1.0, abs=1e-4)]
    assert m["t0"] == pytest.approx(0.95, abs=0.03) and m["t1"] == pytest.approx(1.05, abs=0.03)
    assert state(page)["sel"] is None, "K clears the selection"
    assert "learned 1 hit" in msg(page) and "(1 learn, 0 forget)" in msg(page)
    # the bar (top 3 px of the spike lane, green) and the dot on the hit; the filter is off after K alone, so the
    # marks are drawn at 40 % (concept 2.4) until it is on
    bx = js(page, "() => DSLearn.barX('Kick In', 'm1')")
    cx = (bx[0] + bx[1]) / 2 - GEO_X(page)
    got = pixel(page, '.track[data-name="Kick In"] .spike', cx, 1)
    assert not near(got, GREEN, 30) and got[1] > got[0] + 15 and got[1] > got[2] + 15, got   # dimmed green
    page.evaluate("() => lnSetLearn('Kick In', { on: true }, 'on')")
    settled(page)
    page.evaluate("() => flushFrame()")
    got = pixel(page, '.track[data-name="Kick In"] .spike', cx, 1)
    assert near(got, GREEN) and got[3] > 200, got
    hx = hit_x(page, "Kick In", 1) - GEO_X(page)
    assert near(pixel(page, '.track[data-name="Kick In"] .spike', hx, 2), GREEN, 60)    # the dot sits in the top 4 px ...
    assert not near(pixel(page, '.track[data-name="Kick In"] .spike', hx, 10), GREEN, 60)   # ... not over the hit line
    # the Inspector row
    ins = inspect_track(page, "Kick In")
    expect(ins.locator(".lnCount")).to_contain_text("✓ 1  ✗ 0")


def test_a_wide_range_over_three_hits_gives_three_examples(lrn):
    page, sid, _ = lrn
    select(page, "Kick In", 0.9, 2.1)
    expect(page.locator("#ilSel")).to_contain_text("3 hits")
    mark(page)
    m = learn_of(page, sid)["marks"][0]
    assert len(m["at"]) == 3 and [round(a, 3) for a in m["at"]] == [1.0, 1.5, 2.0]
    st = state(page)
    assert js(page, "() => DSLearn.times('Kick In')")["this"] == pytest.approx([1.0, 1.5, 2.0], abs=1e-4)
    assert "learned 3 hits" in msg(page)


def test_a_range_with_no_hit_takes_the_strongest_rise_and_a_range_with_nothing_makes_no_mark(lrn):
    page, sid, _ = lrn
    # the soft kick at 3.5 s is in the audio, not in the hits, and no analysis kept candidates: the rise rule
    select(page, "Kick In", 3.4, 3.7)
    mark(page)
    m = learn_of(page, sid)["marks"][0]
    assert len(m["at"]) == 1 and 3.49 < m["at"][0] < 3.56, m
    assert "added 1 hit" in msg(page)   # P4-38b: the rise range is no hit yet: K adds it
    # P4-38b: a stretch of silence is no longer refused for K: the hit at the range centre (#81) is
    # added (learned) and the example stored; Shift+K keeps today's no-mark answer
    select(page, "Kick In", 4.4, 4.9)
    mark(page)
    marks = learn_of(page, sid)["marks"]
    assert len(marks) == 2
    assert "added 1 hit" in msg(page)
    evs = invoke(page, "events_get", sid=sid, name="Kick In")["events"]
    added = [e for e in evs if e.get("learned")]
    assert len(added) == 2 and 3.49 <= added[0]["t"] <= 3.56 and 4.64 <= added[1]["t"] <= 4.66  # the rise range adds too
    select(page, "Kick In", 5.2, 5.6)
    page.keyboard.press("Shift+K")
    settled(page)
    assert len(learn_of(page, sid)["marks"]) == 2
    assert msg(page).endswith("no hit in the selection")
    assert state(page)["sel"] is not None


def test_k_without_a_range_or_a_hit_says_so(lrn):
    page, sid, _ = lrn
    page.keyboard.press("K")
    assert "select a range or a hit first" in msg(page)
    assert learn_of(page, sid) is None


def test_a_selected_hit_is_one_example_and_shift_k_makes_it_not_this(lrn):
    page, sid, _ = lrn
    page.evaluate("() => { S.view = viewClamp(0.5, 3.1); invalidate(F.VIEW); }")
    page.evaluate("() => flushFrame()")
    box = lane_box(page, "Kick In", "spike")
    page.mouse.click(hit_x(page, "Kick In", 2), box["y"] + box["height"] / 2)   # the hit at 1.5 s
    assert js(page, "() => S.sel.index") == 2
    mark(page, "Shift+K")
    m = learn_of(page, sid)["marks"][0]
    assert m["kind"] == "not" and m["at"] == [pytest.approx(1.5, abs=1e-4)] and m["t0"] == m["t1"]
    assert learn_of(page, sid)["on"] is True, "P4-6c: the first not example turns the filter on"
    assert js(page, "() => DSLearn.hitKind('Kick In', 1.5)") == "not"
    # the info line names the hit; once it is deselected it is tinted red in the spike lane: this session keeps no
    # activations (no filter can run), so the hit is still in the output (P4-6c fix round: not the filtered grey)
    expect(page.locator("#ilLearn")).to_have_text("not this")
    page.keyboard.press("Escape")
    page.evaluate("() => flushFrame()")
    hx = hit_x(page, "Kick In", 2) - GEO_X(page)
    got = pixel(page, '.track[data-name="Kick In"] .spike', hx, 25)
    assert near(got, (0xb8, 0x70, 0x6c), 30), got
    # the other hits keep the track colour
    other = pixel(page, '.track[data-name="Kick In"] .spike', hit_x(page, "Kick In", 1) - GEO_X(page), 25)
    assert not near(other, (0x4a, 0x50, 0x5c), 20), other


def test_the_last_kind_wins_and_one_undo_step_restores_the_first(lrn):
    page, sid, _ = lrn
    select(page, "Kick In", 0.95, 1.55)
    mark(page)                                            # hits 1.0 and 1.5 as this
    first = learn_of(page, sid)
    page.evaluate("() => { S.sel = {track: 'Kick In', index: 2}; invalidateHits('Kick In'); updateStatus(); }")
    mark(page, "Shift+K")                                 # the hit at 1.5 again, now not
    learn = learn_of(page, sid)
    kinds = {m["id"]: (m["kind"], [round(a, 3) for a in m["at"]]) for m in learn["marks"]}
    assert kinds == {"m1": ("this", [1.0]), "m2": ("not", [1.5])}, kinds
    assert "1 changed kind" in msg(page)
    page.keyboard.press("Z")
    settled(page)
    assert learn_of(page, sid) == first
    page.keyboard.press("Shift+Z")
    settled(page)
    assert learn_of(page, sid) == learn


def test_undo_and_redo_of_a_mark_a_removal_a_flip_and_forget(lrn):
    page, sid, _ = lrn
    select(page, "Kick In", 0.95, 1.05); mark(page)
    s1 = learn_of(page, sid)
    select(page, "Kick In", 1.95, 2.05); mark(page, "Shift+K")
    s2 = learn_of(page, sid)
    assert len(s2["marks"]) == 2
    menu(page, "learn_forget")
    settled(page)
    forgot = learn_of(page, sid)
    assert forgot["marks"] == [] and forgot["on"] is True and forgot["threshold"] == 0.5 and forgot["auto"] is True   # the settings stay
    assert "forgot 2 examples" in msg(page)
    for want in (s2, s1, None):                           # Cmd+Z three times: Forget, the second mark, the first mark
        page.keyboard.press("Z"); settled(page)
        assert learn_of(page, sid) == want
    for want in (s1, s2, forgot):                           # redo them again
        page.keyboard.press("Shift+Z"); settled(page)
        assert learn_of(page, sid) == want
    for want in (s2,):
        page.keyboard.press("Z"); settled(page)
        assert learn_of(page, sid) == want
    # the Inspector counts follow
    ins = inspect_track(page, "Kick In")
    expect(ins.locator(".lnCount")).to_contain_text("✓ 1  ✗ 1")
    # the next change after an undo drops the redo steps
    page.keyboard.press("Z"); settled(page)
    select(page, "Kick In", 2.45, 2.55); mark(page)
    after = learn_of(page, sid)
    page.keyboard.press("Shift+Z"); settled(page)
    assert len(after["marks"]) == 2 and learn_of(page, sid) == after, "a new step drops the redo steps"


def test_the_marks_are_saved_with_the_project_and_come_back_on_reopen(lrn):
    page, sid, folder = lrn
    select(page, "Kick In", 0.9, 1.6); mark(page)
    select(page, "Kick In", 2.45, 2.55); mark(page, "Shift+K")
    want = learn_of(page, sid)
    page.evaluate("() => saveProjectNow()")
    wait_until(lambda: not js(page, "() => S.dirty") if js(page, "() => 'dirty' in S") else True)
    doc = json.loads(next(folder.glob("*.drumspike")).read_text())
    assert doc["format"] == 1
    assert doc["tracks"]["Kick In"]["learn"] == want
    assert "learn" not in doc["tracks"]["Snare"]
    open_project_again(page, folder)
    assert learn_of(page, js(page, "() => S.sid")) == want
    assert js(page, "() => S.tracks['Kick In'].learn") == want
    page.wait_for_function("() => DSLearn.times('Kick In').this.length === 2")
    ins = inspect_track(page, "Kick In")
    expect(ins.locator(".lnCount")).to_contain_text("✓ 2  ✗ 1")


def open_project_again(page, folder):
    page.evaluate("() => { window.__DS_PICK__ = null; }")
    sid = js(page, "() => S.sid")
    invoke(page, "project_close", sid=sid, force=True)
    return open_project(page, folder)


def test_a_follower_marks_its_source_track(lrn):
    page, sid, _ = lrn
    invoke(page, "track_set_follow", sid=sid, name="Kick Sub", source="Kick In")
    page.evaluate("async () => { await loadProject(); }")
    page.wait_for_function("() => S.loaded > 0 && S.events['Kick Sub'] && S.events['Kick Sub'].length > 0")
    select(page, "Kick Sub", 0.9, 1.6)
    mark(page)
    assert learn_of(page, sid, "Kick Sub") is None
    m = learn_of(page, sid, "Kick In")["marks"][0]
    assert len(m["at"]) == 2
    assert "learned on Kick In (Kick Sub follows it)" in msg(page)
    ins = inspect_track(page, "Kick Sub")
    expect(ins.locator(".lnCount")).to_contain_text("on Kick In: ✓ 2")
    # undo goes to the source too
    page.keyboard.press("Z"); settled(page)
    assert learn_of(page, sid, "Kick In") is None


# ---------------------------------------------------------------------------- menu, keys, context menus, Inspector

def test_the_hits_menu_items_and_the_greyed_learn_filter(lrn):
    page, sid, _ = lrn
    select(page, "Kick In", 0.95, 1.05)
    assert menu(page, "learn_this") is True
    settled(page)
    assert len(learn_of(page, sid)["marks"]) == 1
    select(page, "Kick In", 1.95, 2.05)
    assert menu(page, "learn_not") is True
    settled(page)
    assert [m["kind"] for m in learn_of(page, sid)["marks"]] == ["this", "not"]
    assert menu(page, "learn_filter") is True   # P4-6c: the item toggles the filter of the track (the Shift+K turned it on)
    settled(page)
    assert learn_of(page, sid)["on"] is False and "Learn filter off" in msg(page)
    assert menu(page, "learn_forget") is True
    settled(page)
    assert marks(page, sid) == []
    rows = {c["id"]: c for c in js(page, "() => DSKeys.COMMANDS")}
    assert rows["learn_this"]["page_keys"] == ["K"] and rows["learn_not"]["page_keys"] == ["Shift+K"]
    assert rows["learn_filter"]["menu"] == "learn_filter" and rows["learn_forget"]["path"] == "Hits"
    gestures = js(page, "() => DSKeys.GESTURES.map(g => g[0] + ': ' + g[1])")
    assert any("drag wave lane" in g and "range" in g for g in gestures)


def test_the_context_menus_of_the_selection_and_of_a_mark_bar(lrn):
    page, sid, _ = lrn
    select(page, "Kick In", 0.9, 1.6)
    box = lane_box(page, "Kick In", "wave")
    page.mouse.click(px(page, 1.2), box["y"] + 30, button="right")
    items = page.locator("#ctxMenu button")
    assert [t.strip() for t in items.all_inner_texts()] == ["Learn This", "Forget This", "Clear Selection"]
    page.locator('#ctxMenu [data-act="learn"]').click()
    settled(page)
    assert len(learn_of(page, sid)["marks"][0]["at"]) == 2
    # a right-click outside any selection opens nothing in the wave lane
    page.mouse.click(px(page, 4.0), box["y"] + 30, button="right")
    assert page.locator("#ctxMenu").count() == 0
    # the mark bar: Change to Not This, then Remove Example
    sbox = lane_box(page, "Kick In", "spike")
    bx = js(page, "() => DSLearn.barX('Kick In', 'm1')")
    page.mouse.click((bx[0] + bx[1]) / 2, sbox["y"] + 2, button="right")
    assert [t.strip() for t in page.locator("#ctxMenu button").all_inner_texts()] == ["Remove Example", "Change to Forget This"]
    page.locator('#ctxMenu [data-act="flip"]').click()
    settled(page)
    assert learn_of(page, sid)["marks"][0]["kind"] == "not"
    bx = js(page, "() => DSLearn.barX('Kick In', 'm1')")
    page.mouse.click((bx[0] + bx[1]) / 2, sbox["y"] + 2, button="right")
    assert page.locator('#ctxMenu [data-act="flip"]').inner_text() == "Change to Learn This"
    page.locator('#ctxMenu [data-act="remove"]').click()
    settled(page)
    assert marks(page, sid) == []
    # Clear Selection
    select(page, "Kick In", 0.9, 1.6)
    page.mouse.click(px(page, 1.2), box["y"] + 30, button="right")
    page.locator('#ctxMenu [data-act="clear"]').click()
    assert state(page)["sel"] is None


def test_a_click_on_a_mark_bar_selects_it_and_delete_removes_it_and_the_fast_hit_delete_stays(lrn):
    page, sid, _ = lrn
    select(page, "Kick In", 0.95, 1.55); mark(page)
    sbox = lane_box(page, "Kick In", "spike")
    bx = js(page, "() => DSLearn.barX('Kick In', 'm1')")
    page.mouse.click((bx[0] + bx[1]) / 2, sbox["y"] + 2)
    assert state(page)["mark"] == {"track": "Kick In", "id": "m1"}
    page.keyboard.press("Escape")
    assert state(page)["mark"] is None
    page.mouse.click((bx[0] + bx[1]) / 2, sbox["y"] + 2)
    page.keyboard.press("Delete")
    settled(page)
    assert marks(page, sid) == [] and state(page)["mark"] is None
    assert js(page, "() => S.events['Kick In'].length") == len(HITS), "Delete took the mark, not a hit"
    # the right-click on a hit still deletes it at once
    page.evaluate("() => { S.view = viewClamp(0.5, 3.1); invalidate(F.VIEW); flushFrame(); }")
    page.mouse.click(hit_x(page, "Kick In", 3), sbox["y"] + 20, button="right")
    edits_done(page)
    # P4-38d: the K and the Remove also run the Learn pick now (the Add rule); wait for the delete's answer, not a sleep
    try:
        wait_until(lambda: js(page, "() => S.events['Kick In'].length") == len(HITS) - 1, msg="the right-click deletes the hit")
    except AssertionError as e:
        raise AssertionError(f"{e} (status: {msg(page)!r})") from None
    assert page.locator("#ctxMenu").count() == 0


def test_the_inspector_learn_row_has_the_buttons_the_list_and_forget(lrn):
    page, sid, _ = lrn
    ins = inspect_track(page, "Kick In")
    expect(ins.locator(".lnCount")).to_have_text("Select hits you want and press K")
    expect(ins.locator(".lnThis")).to_be_disabled()
    expect(ins.locator(".lnForget")).to_be_disabled()
    expect(ins.locator(".lnFilterBox")).to_be_disabled()
    select(page, "Kick In", 0.95, 1.05)
    expect(ins.locator(".lnThis")).to_be_enabled()
    ins.locator(".lnThis").click()
    settled(page)
    expect(ins.locator(".lnCount")).to_contain_text("✓ 1  ✗ 0")
    expect(ins.locator(".lnCount")).to_contain_text("1 example: add 2 more")
    select(page, "Kick In", 1.95, 2.05)
    ins.locator(".lnNot").click()
    settled(page)
    expect(ins.locator(".lnCount")).to_contain_text("✓ 1  ✗ 1")
    ins.locator(".lnList summary").click()
    rows = ins.locator(".lnMk")
    assert rows.count() == 2 and rows.nth(0).get_attribute("data-kind") == "this" and rows.nth(1).get_attribute("data-kind") == "not"
    rows.nth(1).locator(".lnX").click()
    settled(page)
    assert [m["kind"] for m in learn_of(page, sid)["marks"]] == ["this"]
    ins.locator(".lnForget").click()
    settled(page)
    assert marks(page, sid) == []
    expect(ins.locator(".lnCount")).to_have_text("Select hits you want and press K")


def test_more_than_64_examples_of_a_kind_are_refused_and_nothing_changes(lrn):
    page, sid, _ = lrn
    ev = [{"t": round((0.05 + i * 0.05) * SR) / SR, "v": 90} for i in range(65)]
    invoke(page, "events_set", sid=sid, name="Kick In", events=ev)
    page.evaluate("async () => { await loadProject(); }")
    page.wait_for_function("() => S.loaded > 0 && S.events['Kick In'].length === 65")
    select(page, "Kick In", 0.0, 3.6)
    mark(page)
    assert learn_of(page, sid) is None
    assert "at most 64" in msg(page) and "Forget" in msg(page)


# ---------------------------------------------------------------------------- fix round

def test_the_mark_colours_are_the_theme_tokens(lrn):
    page, sid, _ = lrn
    tok = js(page, """() => { const c = getComputedStyle(document.documentElement);
      return {this: c.getPropertyValue('--learn-this').trim(), not: c.getPropertyValue('--learn-not').trim()}; }""")
    assert tok == {"this": "#4cc38a", "not": "#e5534b"}
    assert js(page, "() => DSLearn.colors()") == tok


def test_the_ticks_of_a_selected_mark_pulse_once(lrn):
    page, sid, _ = lrn
    select(page, "Kick In", 0.95, 1.05); mark(page)
    # P4-29: the default bar and beat grid lines are lane pixels beside the hit line; this test measures the dot,
    # drawn at full colour with the filter on (K alone leaves it off: 40 %)
    page.evaluate("() => { TM.ui.grid = false; tmApplyUi(true); }")
    page.evaluate("() => lnSetLearn('Kick In', { on: true }, 'on')")
    settled(page)
    page.evaluate("() => { S.view = viewClamp(0.9, 1.1); invalidate(F.VIEW); flushFrame(); }")
    sbox = lane_box(page, "Kick In", "spike")
    bx = js(page, "() => DSLearn.barX('Kick In', 'm1')")
    hx = hit_x(page, "Kick In", 1) - GEO_X(page)
    def side():   # a pixel below the bar and beside the hit line: the dot reaches it only while it pulses (radius 3.5, 2 at rest)
        p = pixel(page, '.track[data-name="Kick In"] .spike', hx + 2, 4)
        return p[3] > 150 and near(p, GREEN, 60)
    assert not js(page, "() => DSLearn.pulsing()") and not side()
    page.mouse.click((bx[0] + bx[1]) / 2 + 6, sbox["y"] + 2)
    assert state(page)["mark"]["id"] == "m1" and js(page, "() => DSLearn.pulsing()")
    page.evaluate("() => flushFrame()")
    assert side()
    wait_until(lambda: not js(page, "() => DSLearn.pulsing()"), msg="the pulse ends")
    page.wait_for_timeout(100)
    assert not side(), "the dot is back to rest"
    assert state(page)["mark"] is not None


def test_a_click_on_a_row_of_the_mark_list_moves_the_playhead(lrn):
    page, sid, _ = lrn
    select(page, "Kick In", 1.95, 2.55); mark(page)
    ins = inspect_track(page, "Kick In")
    ins.locator(".lnList summary").click()
    page.evaluate("() => { S.playhead = 4; }")
    ins.locator(".lnMk").first.click()
    assert js(page, "() => S.playhead") == pytest.approx(2.0, abs=1e-3)
    assert state(page)["mark"] == {"track": "Kick In", "id": "m1"}


def test_the_example_ticks_are_drawn_with_the_hitpoint_shift_and_a_hit_mark_takes_it_off(lrn):
    page, sid, _ = lrn
    invoke(page, "track_params_set", sid=sid, name="Kick In", params={"shift_samples": 480, "min_gap_ms": 20})
    page.evaluate("async () => { await loadProject(); }")
    page.wait_for_function("() => S.loaded > 0 && S.tracks['Kick In'].params.shift_samples === 480")
    page.evaluate("() => { S.view = viewClamp(0.4, 0.6); invalidate(F.VIEW); flushFrame(); }")
    page.evaluate("() => { S.sel = {track: 'Kick In', index: 0}; updateStatus(); }")
    mark(page)                                            # the hit at 0.5 s as a single hit
    at = learn_of(page, sid)["marks"][0]["at"][0]
    assert at == pytest.approx(0.49, abs=1e-6), "the stored example is before the shift"
    assert js(page, "() => DSLearn.times('Kick In')")["this"] == pytest.approx([0.5], abs=1e-6)
    page.keyboard.press("Escape")
    page.evaluate("() => redrawAll()")                    # the lane is drawn again now, not in a frame the test may beat
    hx = hit_x(page, "Kick In", 0) - GEO_X(page)
    spike = '.track[data-name="Kick In"] .spike'
    assert near(pixel(page, spike, hx, 2), GREEN, 60), "the tick is on the hit"
    off = js(page, "() => tToX(0.49, laneWidth()) - tToX(0.5, laneWidth())")
    assert off < -20 and not near(pixel(page, spike, hx + off, 2), GREEN, 60), "nothing at the unshifted time"


def test_a_mark_makes_the_page_dirty_and_the_service_too(lrn):
    page, sid, _ = lrn
    page.evaluate("() => saveProjectNow()")
    wait_until(lambda: not js(page, "() => S.dirty"), msg="saved")
    assert invoke(page, "tracks_get", sid=sid)["dirty"] is False
    # the service's own dirty event may set the flag as well: the page's call must be there too (the stack names lnApply)
    page.evaluate("""() => { window.__dirty = []; const o = window.setDirtyState;
      window.setDirtyState = d => { window.__dirty.push([d, new Error().stack]); o(d); }; }""")
    select(page, "Kick In", 0.95, 1.05); mark(page)
    assert js(page, "() => S.dirty") is True
    assert any(d and "lnApply" in st for d, st in js(page, "() => window.__dirty")), "the page sets its dirty state itself"
    assert invoke(page, "tracks_get", sid=sid)["dirty"] is True


def test_undo_waits_while_a_mark_is_on_its_way(lrn):
    page, sid, _ = lrn
    select(page, "Kick In", 0.95, 1.05); mark(page)
    page.evaluate("""() => { const orig = B.learnMark.bind(B); window.__held = null;
      B.learnMark = (...a) => new Promise(r => { window.__release = () => { B.learnMark = orig; r(orig(...a)); }; }); }""")
    select(page, "Kick In", 1.95, 2.05)
    page.keyboard.press("K")
    page.wait_for_function("() => DSLearn.state().pending === 1")
    page.keyboard.press("Z")                              # while the mark is on its way: no step is taken
    assert len(learn_of(page, sid)["marks"]) == 1 and js(page, "() => S.undo.length") == 1
    page.evaluate("() => window.__release()")
    settled(page)
    assert len(learn_of(page, sid)["marks"]) == 2 and js(page, "() => S.undo.length") == 2
    page.keyboard.press("Z"); settled(page)
    assert len(learn_of(page, sid)["marks"]) == 1


def test_the_info_line_count_leaves_out_hits_cut_by_the_loudness_cut(lrn):
    page, sid, _ = lrn
    ev = [{"t": round(t * SR) / SR, "v": 100, **({"cut": True} if t == 1.0 else {})} for t in HITS]
    invoke(page, "events_set", sid=sid, name="Kick In", events=ev)
    page.evaluate("async () => { await loadProject(); }")
    page.wait_for_function("() => S.loaded > 0 && S.events['Kick In'].some(e => e.cut)")
    select(page, "Kick In", 0.9, 1.6)
    assert state(page)["text"].endswith(", 1 hit")        # 1.0 s is cut, 1.5 s counts
    mark(page)
    assert [round(a, 3) for a in learn_of(page, sid)["marks"][0]["at"]] == [1.5]


def test_a_right_click_on_an_empty_part_of_a_spike_lane_shows_no_browser_menu(lrn):
    page, sid, _ = lrn
    prevented = page.evaluate("""([x, y]) => { const el = document.querySelector('.track[data-name="Kick In"] .spike');
      const ev = new MouseEvent('contextmenu', {bubbles: true, cancelable: true, clientX: x, clientY: y, button: 2});
      el.dispatchEvent(ev); return ev.defaultPrevented; }""", [px(page, 4.4), lane_box(page, "Kick In", "spike")["y"] + 20])
    assert prevented is True and page.locator("#ctxMenu").count() == 0


def test_the_inspector_opens_after_a_hitpoint_shift_on_a_track_without_params(lrn):
    page, sid, _ = lrn
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    invoke(page, "track_shift_set", sid=sid, name="Snare", shift_samples=48)
    page.evaluate("async () => { await loadProject(); }")
    page.wait_for_function("() => S.loaded > 0 && S.tracks['Snare'].params && S.tracks['Snare'].params.shift_samples === 48")
    ins = inspect_track(page, "Snare")
    expect(ins.locator(".pVLo")).to_have_value("-40")
    expect(ins.locator(".pVHi")).to_have_value("-6")
    assert errors == [], errors


# ---------------------------------------------------------------------------- P4-38b fix round 2

def doc_hits(page, sid):
    return invoke(page, "events_get", sid=sid, name="Kick In")["events"]


def page_times(page):
    return js(page, "() => S.events['Kick In'].map(e => e.t)")


def test_k_on_a_detected_hit_adds_no_second_hit(lrn):
    page, sid, _ = lrn
    n = len(doc_hits(page, sid))
    js(page, "() => { S.sel = { track: 'Kick In', index: 2 }; LN.sel = null; learnMark('this'); }")
    settled(page)
    assert len(doc_hits(page, sid)) == n, "no hit was added on a detected hit"   # planted fault: the fallback adds a duplicate
    assert [e for e in doc_hits(page, sid) if e.get("learned")] == []
    assert len(page_times(page)) == n
    m = learn_of(page, sid)
    assert len(m["marks"]) == 1 and m["marks"][0]["at"] == [1.5] and m["on"] is False   # K on a detected hit: no filter switch
    assert "added" not in msg(page) and "learned 1 hit" in msg(page)


def test_the_page_shows_the_hit_a_k_added_and_one_undo_removes_it_and_the_example(lrn):
    page, sid, _ = lrn
    n = len(doc_hits(page, sid))
    js(page, "() => { LN.sel = { track: 'Kick In', t0: 4.4, t1: 4.45 }; S.sel = null; learnMark('this'); }")
    settled(page)
    assert len(doc_hits(page, sid)) == n + 1
    assert len(page_times(page)) == n + 1, "the page lists the added hit without a reload"   # planted fault: S.events stays stale
    assert any(abs(t - 4.4) < 0.06 for t in page_times(page))
    page.keyboard.press("Z"); settled(page)           # one undo step: the hit and the example
    assert learn_of(page, sid) is None
    assert len(doc_hits(page, sid)) == n and len(page_times(page)) == n
    page.keyboard.press("Shift+Z"); settled(page)     # the redo brings all back
    assert len(doc_hits(page, sid)) == n + 1 and len(page_times(page)) == n + 1
    l = learn_of(page, sid)
    assert len(l["marks"]) == 1 and l["on"] is False   # P4-38d: a K-add never turns the filter on
    assert [e["t"] for e in doc_hits(page, sid) if e.get("learned")] and all(e.get("learned") for e in doc_hits(page, sid) if e["t"] > 4.0)   # planted fault: the redo drops the flag


def test_a_k_add_keeps_the_filter_off_and_the_status_says_what_the_add_rule_does(lrn):
    # P4-38d: the Add rule runs with the filter off, so a K-add no longer turns the filter on (it cut busy tracks)
    page, sid, _ = lrn
    js(page, "() => { LN.sel = { track: 'Kick In', t0: 4.4, t1: 4.45 }; S.sel = null; learnMark('this'); }")
    settled(page)
    assert learn_of(page, sid)["on"] is False              # planted fault: the old switch turns it on
    text = msg(page)
    assert text.startswith("Kick In: added 1 hit at ") and "and learned it (1 learn, 0 forget)" in text, text
    assert "Learn filter on" not in text and "Learn filter off" not in text, text
    assert "Learn adds " in text and "(Add 0.85) - drag the Add line for more or fewer" in text, text
    # a second K-add keeps it off and says the same
    js(page, "() => { LN.sel = { track: 'Kick In', t0: 4.7, t1: 4.75 }; S.sel = null; learnMark('this'); }")
    settled(page)
    assert learn_of(page, sid)["on"] is False and "Learn adds " in msg(page), msg(page)
    assert len(page_times(page)) == len(doc_hits(page, sid))


def test_a_k_on_a_detected_hit_then_a_k_add_keep_the_filter_off(lrn):
    page, sid, _ = lrn
    js(page, "() => { S.sel = { track: 'Kick In', index: 2 }; LN.sel = null; learnMark('this'); }")
    settled(page)
    assert learn_of(page, sid)["on"] is False
    js(page, "() => { LN.sel = { track: 'Kick In', t0: 4.4, t1: 4.45 }; S.sel = null; learnMark('this'); }")
    settled(page)
    assert learn_of(page, sid)["on"] is False              # planted fault: the old switch turns it on
    text = msg(page)
    assert "(2 learn, 0 forget)" in text and "Learn filter on" not in text and "Learn adds " in text, text
