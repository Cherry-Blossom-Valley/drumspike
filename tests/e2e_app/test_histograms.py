"""P4-3: the Sensitivity and loudness histograms of the Inspector (owner request 2026-10-12, after Massey DRT). The
Sensitivity slider runs over the histogram of every candidate hit (coloured = detected, grey = filtered); moving it
picks the hits again from the activations the analysis kept (no model run) and shows them live in the lane, with the
hits that go as grey lines; letting go stores them (one undo step). The loudness histogram shows the peak level of every
hit with the floor and ceiling handles of the velocity map; the velocities follow live and are stored on release.
A follower shows its loudness histogram only. Both draw only on a change."""
import math

import pytest
from playwright.sync_api import expect

from app_support import inspect_track, invoke, open_project, wait_until
from helpers import SR, synth_track, write_wav
from test_subtracks import analyse, js

LOUD = [0.5, 1.5, 2.5, 3.5, 4.5]
SOFT = [1.0, 2.0, 3.0, 4.0]          # -30 dB between the loud hits: a second cluster of candidates


def build(make_project, tmp_path, name="Hist"):
    """Kick In: loud and soft kicks; Kick Sub: the loud kicks 20 dB softer (a follower in the tests)."""
    src = tmp_path / f"src_{name}"
    src.mkdir()
    on = sorted(LOUD + SOFT)
    x, _ = synth_track(on, 6.0, gains_db=[-3.0 if t in LOUD else -36.0 for t in on])
    sub, _ = synth_track(LOUD, 6.0, gains_db=[-23.0] * len(LOUD))
    specs = [("Kick In", write_wav(src / "Kick In.wav", x), "kick", 36, None),
             ("Kick Sub", write_wav(src / "Kick Sub.wav", sub), "kick", 36, None)]
    return make_project.builder.build_project(tmp_path / f"projects_{name}", name, specs, "copied")


@pytest.fixture
def hist(app, make_project, tmp_path):
    # A4: the model-free detection keeps its candidates, so the histograms work without the
    # model too; the skip is only for a detection that did not run at all.
    sid = open_project(app, build(make_project, tmp_path))
    analyse(app)
    ins = inspect_track(app, "Kick In")
    app.wait_for_function("() => DSHist.state().detection === 'ok' && DSHist.state().loud && DSHist.state().loud.n > 0")
    return sid, ins


def state(page):
    return js(page, "() => DSHist.state()")


def settled(page):
    page.wait_for_function("() => !HG.sens.base && !DSHist.state().busy")


def page_events(page, name="Kick In"):
    return js(page, "n => S.events[n].map(e => ({t: e.t, v: e.v}))", name)


def service_tv(page, sid, name="Kick In"):
    js(page, "() => flushEvents()")
    return [{"t": e["t"], "v": e["v"]} for e in invoke(page, "events_get", sid=sid, name=name)["events"]]


def params_at(page, v):
    return js(page, "v => ({...S.tracks['Kick In'].params, thresholds: sensThresholds(v), sensitivity: v})", v)


def thumb_x(page, v):
    """The page x of the slider thumb at value v (P4-25: the slider lies under the canvas; its thumb travel is HG_THUMB / 2 + v / 150 * (W - HG_THUMB))."""
    box = page.locator("#insParams .pSens").bounding_box()
    return box["x"] + js(page, "([v, w]) => DSHist.sensX(v, w, 150)", [v, box["width"]]), box["y"] + box["height"] / 2


def pixel(page, sel, x, y):
    """The RGBA of a canvas pixel at CSS px (x, y) of the canvas."""
    return js(page, """([sel, x, y]) => { const c = document.querySelector(sel), d = window.devicePixelRatio || 1;
      return [...c.getContext('2d').getImageData(Math.round(x * d), Math.round(y * d), 1, 1).data]; }""", [sel, x, y])


def test_the_sensitivity_histogram_counts_every_candidate_and_marks_the_threshold(app, hist):
    sid, ins = hist
    data = invoke(app, "track_histograms", sid=sid, name="Kick In")["detection"]
    cands = data["candidates"]
    st = state(app)
    s = st["sens"]
    assert s["cur"] == 50 and s["max"] == 150
    assert s["total"] == len(cands) > len(LOUD), cands                 # every candidate is counted, the soft ones too
    assert sum(s["bins"]) == s["total"]
    for v in range(151):
        assert s["bins"][v] == sum(1 for c in cands if c[3] == v), v
    assert s["active"] == sum(1 for c in cands if c[3] <= 50)
    assert s["x"] == pytest.approx(6 + 50 / 150 * (s["width"] - 12))   # the threshold line is the slider thumb
    n = len(page_events(app))
    weak = sum(1 for c in cands if data["gap"]["from"] < c[3] <= 50)   # P4-25: "N hits" and, beyond the gap, "· W weak"
    assert n == s["active"] and st["text"] == f"{n} hits" + (f" · {weak} weak" if weak else "")
    tip = ins.locator(".hgSensWrap").get_attribute("title")
    assert f"{s['active']} of {s['total']} candidates on" in tip and "thresholds" in tip   # the counts moved into the tooltip
    assert not ins.locator(".hgSensCount").evaluate("e => /candidates|thresholds/.test(e.parentElement.parentElement.innerText)")
    # P4-25: the bars are on the strength axis (60 bins, strong at the left); every candidate is in one bin
    assert s["nb"] == 80 and sum(s["sb"]) == s["total"] and sum(s["son"]) == s["active"]
    top = lambda a: sorted(c[2] for c in a)
    strengths = [c[2] for c in cands]
    first = next(b for b, n in enumerate(s["sb"]) if n)
    last = max(b for b, n in enumerate(s["sb"]) if n)
    assert 1 <= first <= 3 and 76 <= last <= 78 and min(strengths) < max(strengths)   # the axis spans the actual strongest to the weakest (3 % margin): the bars use the full width
    assert s["lineX"] is not None and 0 <= s["lineX"] <= s["width"]
    # the bars: an active candidate in the track colour, a filtered one grey (the canvas pixels)
    colour = js(app, "() => S.tracks['Kick In'].color")
    want = [int(colour[i:i + 2], 16) for i in (1, 3, 5)]
    on = next(b for b in range(80) if s["son"][b])
    off = next((b for b in range(80) if s["sb"][b] and not s["son"][b]), None)
    bx = lambda b: (b + 0.5) * s["width"] / 80
    h = js(app, "() => document.querySelector('#insParams .hgSensCanvas').clientHeight")
    near = lambda got, rgb: all(abs(a - b) <= 2 for a, b in zip(got[:3], rgb))   # the canvas may round a channel by 1
    got = pixel(app, "#insParams .hgSensCanvas", bx(on), h - 1)
    assert near(got, want), (got, want)
    assert off is not None, "the fixture has candidates that are off at 50"
    got = pixel(app, "#insParams .hgSensCanvas", bx(off), h - 1)
    assert near(got, [0x4a, 0x50, 0x5c]), got
    got = pixel(app, "#insParams .hgSensCanvas", s["lineX"], 2)                # the threshold line (accent)
    assert got[2] > 200 and got[3] > 0, got
    # the gap band and the button
    assert data["gap"] and st["sens"]["gap"] == data["gap"]
    expect(ins.locator(".hgGapBtn")).to_be_visible()


def test_dragging_the_sensitivity_repicks_the_lane_live_and_stores_on_release(app, hist):
    sid, ins = hist
    before = service_tv(app, sid)
    assert len(before) == state(app)["sens"]["active"]                 # the default keeps the candidates on at 50
    sens = ins.locator(".pSens")
    sens.scroll_into_view_if_needed()
    x0, y = thumb_x(app, 50)
    app.mouse.move(x0, y)
    app.mouse.down()
    x1, _ = thumb_x(app, 150)
    app.mouse.move(x1 + 20, y, steps=8)                                 # to the end: the soft hits come in, live
    expect(sens).to_have_value("150")
    want = invoke(app, "track_repick", sid=sid, name="Kick In", params=params_at(app, 150), store=False)
    assert want["n_events"] > len(before)
    wait_until(lambda: len(page_events(app)) == want["n_events"], msg="the live re-pick in the lane")
    assert page_events(app) == [{"t": e["t"], "v": e["v"]} for e in want["events"]]
    assert service_tv(app, sid) == before                              # shown, not stored while the slider moves
    st = state(app)
    assert st["hits"] == want["n_events"] and st["sens"]["active"] == st["sens"]["total"]
    x2, _ = thumb_x(app, 0)
    app.mouse.move(x2 - 20, y, steps=8)                                 # to the strict end: hits go, they stay grey
    expect(sens).to_have_value("0")
    low = invoke(app, "track_repick", sid=sid, name="Kick In", params=params_at(app, 0), store=False)
    wait_until(lambda: len(page_events(app)) == low["n_events"], msg="the live re-pick at 0")
    gone = len(before) - sum(1 for b in before if any(abs(b["t"] - e["t"]) <= 5e-4 for e in low["events"]))
    assert gone > 0 and state(app)["ghosts"] == gone
    # back to 120 and let go: stored as an analysis stores it
    x3, _ = thumb_x(app, 120)
    app.mouse.move(x3, y, steps=4)
    app.mouse.up()
    expect(sens).to_have_value("120")
    settled(app)
    stored = invoke(app, "track_repick", sid=sid, name="Kick In", params=params_at(app, 120), store=False)
    assert service_tv(app, sid) == [{"t": e["t"], "v": e["v"]} for e in stored["events"]]
    assert state(app)["ghosts"] == 0
    p = invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]
    assert p["sensitivity"] == 120
    # one undo step back to the hits and the slider of the drag start
    js(app, "() => undo()")
    assert [round(e["t"] * SR) for e in page_events(app)] == [round(b["t"] * SR) for b in before]
    expect(inspect_track(app, "Kick In").locator(".pSens")).to_have_value("50")
    assert [round(e["t"] * SR) for e in service_tv(app, sid)] == [round(b["t"] * SR) for b in before]
    js(app, "() => redo()")
    assert len(page_events(app)) == stored["n_events"]


def test_set_in_gap_puts_the_slider_in_the_largest_gap_and_never_on_its_own(app, hist):
    sid, ins = hist
    gap = state(app)["sens"]["gap"]
    assert gap and gap["from"] <= gap["mid"] <= gap["to"]
    expect(ins.locator(".pSens")).to_have_value("50")                  # not applied on its own
    ins.locator(".hgGapBtn").click()
    expect(ins.locator(".pSens")).to_have_value(str(gap["mid"]))
    settled(app)
    assert invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]["sensitivity"] == gap["mid"]
    s = state(app)["sens"]
    assert s["cur"] == gap["mid"] and sum(s["bins"][gap["from"]:gap["to"] + 1]) == 0   # the gap holds no candidate


def test_the_canvas_has_no_text_and_the_gap_is_a_marker_under_it(app, hist):
    """P4-25 (owner): no label on the canvas; the gap is a triangle marker under it (click: snap into the gap)."""
    sid, ins = hist
    js(app, """() => { window.__ft = []; const f = CanvasRenderingContext2D.prototype.fillText;
      CanvasRenderingContext2D.prototype.fillText = function (t, ...r) { window.__ft.push(String(t)); return f.call(this, t, ...r); }; hgDrawSens(); }""")
    assert js(app, "() => window.__ft") == []
    gap = state(app)["sens"]["gap"]
    marker = ins.locator(".hgGapBtn")
    expect(marker).to_be_visible()
    below = sum(n for v, n in enumerate(state(app)["sens"]["bins"]) if v < gap["from"])
    assert marker.get_attribute("title") == f"Snap into the gap: {below} hits"
    assert "Set in gap" not in ins.inner_text()
    gx = state(app)["sens"]["gapX"]                                                # on the strength axis: between the strong and the weak cluster
    box = marker.bounding_box(); cv = ins.locator(".hgSensCanvas").bounding_box()
    assert abs(box["x"] + box["width"] / 2 - (cv["x"] + gx)) <= 1.5               # centred under the gap
    sb = state(app)["sens"]["sb"]; w = state(app)["sens"]["width"]
    assert sb[int(gx / (w / 80))] == 0                                              # the gap is an empty stretch
    marker.click()
    expect(ins.locator(".pSens")).to_have_value(str(gap["mid"]))
    settled(app)
    # the header count follows: weak candidates beyond the gap are none now
    expect(ins.locator(".hgSensCount")).to_have_text(f"{len(page_events(app))} hits")


def test_a_bar_has_a_tooltip_with_its_count(app, hist):
    sid, ins = hist
    s = state(app)["sens"]
    b = next(b for b in range(80) if s["son"][b] and s["son"][b] == s["sb"][b])
    wrap = ins.locator(".hgSensWrap")
    # #280: the hover must land on the canvas of the wrap (the pointermove handler reads the bar
    # from the event's x); a wrap below the fold swallows the move and the title never changes.
    # The bound is 30 s: the handler runs on the page's main thread, and on a loaded gate it can
    # lag seconds behind the move (measured: a timeout at the 10 s default on a arm64-mac full gate).
    wrap.scroll_into_view_if_needed()
    box = wrap.bounding_box()
    app.mouse.move(box["x"] + (b + 0.5) * s["width"] / 80, box["y"] + 10)
    n = s["sb"][b]
    wait_until(lambda: wrap.get_attribute("title").startswith(f"{n} candidate"), msg="the bar tooltip", timeout=30)
    assert f"here: {n} on, 0 off\n" in wrap.get_attribute("title")


def test_set_in_gap_stores_once_and_does_not_rebuild_the_panel(app, hist):
    """P4-25 (owner finding): no flicker: the panel keeps its DOM, one re-pick runs, the draws stay few."""
    sid, ins = hist
    js(app, """() => { document.querySelector('#insParams .pbody').__mark = 1; window.__rp = [];
      const r = B.repick; B.repick = function (...a) { window.__rp.push(a[3]); return r.apply(this, a); }; window.__d0 = HG.draws; }""")
    ins.locator(".hgGapBtn").click()
    settled(app)
    app.wait_for_function("() => window.__rp.length >= 1 && !HG.sens.busy")
    assert js(app, "() => document.querySelector('#insParams .pbody').__mark") == 1
    assert js(app, "() => window.__rp") == [True]                      # one re-pick, the store
    assert js(app, "() => HG.draws - window.__d0") <= 12


def test_the_loudness_handles_set_floor_and_ceiling_with_live_velocities(app, hist):
    sid, ins = hist
    levels = invoke(app, "track_histograms", sid=sid, name="Kick In")["loudness"]["levels"]
    lo = state(app)["loud"]
    assert lo["n"] == len(levels) and sum(lo["bins"]) == lo["n"]
    for lv in levels:
        assert lo["bins"][int(lv // 1) - lo["range"]["lo"]] >= 1
    assert (lo["floor"], lo["ceiling"]) == (-40, -6)
    loud_x = lambda db: js(app, "([d, w, r]) => DSHist.loudX(d, w, r)", [db, lo["width"], lo["range"]])
    assert lo["xFloor"] == pytest.approx(loud_x(-40)) and lo["xCeiling"] == pytest.approx(loud_x(-6))
    before = service_tv(app, sid)
    ins.locator(".hgLoudCanvas").scroll_into_view_if_needed()
    box = ins.locator(".hgLoudCanvas").bounding_box()
    y = box["y"] + box["height"] / 2
    app.mouse.move(box["x"] + lo["xCeiling"], y)
    app.mouse.down()
    app.mouse.move(box["x"] + loud_x(-1) - 0.3, y, steps=5)            # the ceiling to -1 dB: softer velocities, live
    expect(ins.locator(".pVHi")).to_have_value("-1")
    live = [e["v"] for e in page_events(app)]
    assert live == [js(app, "([d]) => DSHist.velOfDb(d, -40, -1)", [lv]) for lv in levels]
    assert live != [e["v"] for e in before]
    assert service_tv(app, sid) == before                              # not stored while the handle moves
    app.mouse.up()
    wait_until(lambda: [e["v"] for e in service_tv(app, sid)] == live, msg="the velocities stored on release")
    rec = invoke(app, "velocities_recompute", sid=sid, name="Kick In")["events"]
    assert [e["v"] for e in rec] == live                               # the page's numbers are the core's
    now = state(app)["loud"]                                                # P4-25: the axis follows the handles, so the range may have moved
    assert now["xCeiling"] == pytest.approx(js(app, "([d, w, r]) => DSHist.loudX(d, w, r)", [-1, now["width"], now["range"]]))
    # the number field moves the handle
    ins.locator(".pVLo").fill("-50")
    ins.locator(".pVLo").blur()
    wait_until(lambda: state(app)["loud"]["floor"] == -50, msg="the floor from the field")
    assert state(app)["loud"]["xFloor"] == pytest.approx(js(app, "([d, w, r]) => DSHist.loudX(d, w, r)", [-50, lo["width"], state(app)["loud"]["range"]]))
    # undo: the velocities and the field come back
    js(app, "() => undo()")
    expect(inspect_track(app, "Kick In").locator(".pVLo")).to_have_value("-40")
    js(app, "() => undo()")
    expect(inspect_track(app, "Kick In").locator(".pVHi")).to_have_value("-6")
    assert [e["v"] for e in page_events(app)] == [e["v"] for e in before]
    invoke(app, "project_save", sid=sid)


def test_a_follower_shows_its_loudness_histogram_and_no_detection_histogram(app, hist):
    sid, ins = hist
    invoke(app, "track_set_follow", sid=sid, name="Kick Sub", source="Kick In")
    js(app, "() => refreshTrackMeta().then(() => reloadFollowers(['Kick Sub']))")
    ins = inspect_track(app, "Kick Sub")
    app.wait_for_function("() => DSHist.state().name === 'Kick Sub' && DSHist.state().detection === 'follows'")
    assert "hgEmpty" in ins.locator(".hgSensWrap").get_attribute("class")
    expect(ins.locator(".hgGapBtn")).to_be_hidden()
    assert state(app)["sens"] is None and state(app)["text"] == ""
    sub = invoke(app, "events_get", sid=sid, name="Kick Sub")["events"]
    lo = state(app)["loud"]
    assert lo["n"] == len(sub)
    kick = invoke(app, "track_histograms", sid=sid, name="Kick In")["loudness"]["levels"]
    mine = invoke(app, "track_histograms", sid=sid, name="Kick Sub")["loudness"]["levels"]
    loud_idx = [i for i, e in enumerate(sub) if any(abs(e["t"] - t) < 0.021 for t in LOUD)]
    assert loud_idx and all(abs((kick[i] - mine[i]) - 20.0) < 1.5 for i in loud_idx)   # its own signal, 20 dB softer
    # the floor on the follower: its hits are derived again with the new map and stored
    ins.locator(".pVLo").fill("-70")
    ins.locator(".pVLo").blur()
    want = [js(app, "([d]) => DSHist.velOfDb(d, -70, -6)", [m]) for m in mine]
    wait_until(lambda: [e["v"] for e in invoke(app, "events_get", sid=sid, name="Kick Sub")["events"]] == want,
               msg="the follower's velocities")
    assert [e["v"] for e in page_events(app, "Kick Sub")] == want


def test_a_repick_keeps_the_edited_hits(app, hist):
    sid, ins = hist
    js(app, "() => { S.events['Kick In'].push({t: 5.25, v: 99, id: newId(), added: true}); S.events['Kick In'].sort((a, b) => a.t - b.t); markDirty('Kick In'); }")
    js(app, "() => flushEvents()")
    ins.locator(".pSens").fill("140")
    settled(app)
    evs = js(app, "() => S.events['Kick In']")
    assert any(e.get("added") and e["t"] == 5.25 and e["v"] == 99 for e in evs)
    assert len(evs) > len(LOUD) + 1
    wait_until(lambda: any(e.get("added") and e["t"] == 5.25 for e in invoke(app, "events_get", sid=sid, name="Kick In")["events"]),
               msg="the edited hit in the service")


def test_the_histograms_draw_only_on_a_change(app, hist):
    sid, ins = hist
    # the fixture's last draws can land late on a loaded runner (CI x86: 8 for 6, 3 of about 7 runs): start
    # from a count that holds for 500 ms; a draw loop never holds and still fails here
    d0 = state(app)["draws"]
    for _ in range(20):
        app.wait_for_timeout(500)
        d, d0 = d0, state(app)["draws"]
        if d == d0:
            break
    else:
        raise AssertionError(f"the histograms draw without a change: {d0} draws after 10 s")
    app.wait_for_timeout(800)
    assert state(app)["draws"] == d0                                   # idle: nothing draws
    js(app, "() => { requestFrame(); redrawAll && redrawAll(); }")
    app.wait_for_timeout(200)
    assert state(app)["draws"] == d0                                   # a lane frame does not draw the histograms
    js(app, "() => { const s = document.querySelector('#insParams .pSens'); s.value = 55; s.dispatchEvent(new Event('input')); }")
    assert state(app)["draws"] > d0                                    # a control draws
    js(app, "() => document.querySelector('#insParams .pSens').dispatchEvent(new Event('change'))")
    settled(app)


def test_the_loudness_cut_greys_the_quiet_hits_and_leaves_them_out_of_the_output(app, hist):
    sid, ins = hist
    ins.locator(".pSens").fill("150")                                   # the soft hits come in (-33 dBFS)
    settled(app)
    app.wait_for_function("() => DSHist.state().loud && DSHist.state().loud.n === S.events['Kick In'].length")
    ins = inspect_track(app, "Kick In")
    lo = state(app)["loud"]
    assert lo["cut"] is None and lo["nCut"] == 0 and "no cut" in js(app, "() => document.querySelector('.hgLoudInfo').textContent")
    assert lo["xCut"] == pytest.approx(js(app, "([d, w, r]) => DSHist.loudX(d, w, r)", [lo["range"]["lo"], lo["width"], lo["range"]]))
    levels = invoke(app, "track_histograms", sid=sid, name="Kick In")["loudness"]["levels"]
    quiet = sum(1 for lv in levels if lv < -20)
    assert 0 < quiet < len(levels), levels
    before = invoke(app, "events_get", sid=sid, name="Kick In")["events"]
    assert not any(e.get("cut") for e in before)
    # drag the cut handle from the left edge to -20 dB: the quiet hits turn grey, live
    ins.locator(".hgLoudCanvas").scroll_into_view_if_needed()
    box = ins.locator(".hgLoudCanvas").bounding_box()
    y = box["y"] + box["height"] / 2
    app.mouse.move(box["x"] + lo["xCut"], y)
    app.mouse.down()
    app.mouse.move(box["x"] + js(app, "([d, w, r]) => DSHist.loudX(d, w, r)", [-20, lo["width"], lo["range"]]) + 0.3, y, steps=5)
    expect(ins.locator(".pVCut")).to_have_value("-20")
    st = state(app)["loud"]
    assert st["cut"] == -20 and st["nCut"] == quiet and sum(st["cutBins"]) == quiet
    assert js(app, "() => S.events['Kick In'].map(e => !!e.cut)") == [lv < -20 for lv in levels]   # exactly the quiet ones
    assert invoke(app, "events_get", sid=sid, name="Kick In")["events"] == before   # not stored while dragging
    app.mouse.up()
    wait_until(lambda: [bool(e.get("cut")) for e in service_events_raw(app, sid)] == [lv < -20 for lv in levels],
               msg="the cut marks stored")
    assert invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]["vel_cut_db"] == -20
    # the output leaves them out
    midi = invoke(app, "export_midi", sid=sid, name="Kick In", note=36, events=js(app, "() => S.events['Kick In']"), ppq=24000, tempo=120)
    assert midi["n_events"] == len(levels) - quiet
    # undo: back to no cut
    js(app, "() => undo()")
    assert js(app, "() => S.events['Kick In'].filter(e => e.cut).length") == 0
    expect(inspect_track(app, "Kick In").locator(".pVCut")).to_have_value("")
    wait_until(lambda: not any(e.get("cut") for e in service_events_raw(app, sid)), msg="the marks gone in the service")
    # the field sets the same; empty is off
    ins = inspect_track(app, "Kick In")
    ins.locator(".pVCut").fill("-20")
    ins.locator(".pVCut").blur()
    wait_until(lambda: state(app)["loud"]["nCut"] == quiet, msg="the cut from the field")
    ins.locator(".pVCut").fill("")
    ins.locator(".pVCut").blur()
    wait_until(lambda: state(app)["loud"]["cut"] is None, msg="the cut off from the field")


def service_events_raw(page, sid, name="Kick In"):
    js(page, "() => flushEvents()")
    return invoke(page, "events_get", sid=sid, name=name)["events"]


# ---------- fix round (controller and verifier of ba4d745) ----------

def test_undo_and_redo_of_a_re_pick_give_the_slider_params_back_to_the_service(app, hist):
    sid, ins = hist
    start = invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]
    ins.locator(".pSens").fill("120")
    settled(app)
    assert invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]["sensitivity"] == 120
    js(app, "() => undo()")
    js(app, "() => flushEvents()")
    want = [round(t, 9) for t in js(app, "() => sensThresholds(50)")]
    wait_until(lambda: [round(t, 9) for t in invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]["thresholds"]] == want,
               msg="the drag-start thresholds in the service")
    p = invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]
    assert p.get("sensitivity") == start.get("sensitivity") and p.get("sensitivity") != 120, p
    invoke(app, "project_save", sid=sid)
    js(app, "() => redo()")
    wait_until(lambda: invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"].get("sensitivity") == 120,
               msg="redo: 120 in the service again")


def test_a_re_pick_keeps_the_loudness_cut_marks(app, hist):
    sid, ins = hist
    ins.locator(".pVCut").fill("-20")
    ins.locator(".pVCut").blur()
    wait_until(lambda: state(app)["loud"]["cut"] == -20, msg="the cut")
    ins.locator(".pSens").fill("150")                                  # the soft hits come in: below the cut
    settled(app)
    levels = invoke(app, "track_histograms", sid=sid, name="Kick In")["loudness"]["levels"]
    marks = js(app, "() => S.events['Kick In'].map(e => !!e.cut)")
    assert any(marks) and marks == [lv < -20 for lv in levels], (marks, levels)
    wait_until(lambda: [bool(e.get("cut")) for e in service_events_raw(app, sid)] == marks, msg="the marks stored")


def test_the_sensitivity_bars_use_a_log_height_without_a_cap(app, hist):
    sid, ins = hist
    s = state(app)["sens"]
    h = js(app, "() => document.querySelector('#insParams .hgSensCanvas').clientHeight")
    full, top = h - 3, max(s["sb"])
    for b, n in enumerate(s["sb"]):
        want = 0 if not n else min(full, max(2, round(math.log1p(n) / math.log1p(top) * full)))
        assert s["heights"][b] == want, (b, n)
    assert max(s["heights"]) == full and "capped" not in s                      # the tallest bin fills the canvas; no cap, no break
    b = js(app, "() => DSHist.barHeights([300, 1, 0, 5, 20], 44)")
    assert b["heights"] == [41, round(math.log(2) / math.log(301) * 41), 0, round(math.log(6) / math.log(301) * 41), round(math.log(21) / math.log(301) * 41)]


def test_the_threshold_line_follows_the_slider_on_the_strength_axis(app, hist):
    sid, ins = hist
    x50 = state(app)["sens"]["lineX"]
    ins.locator(".pSens").fill("0")
    settled(app)
    x0 = state(app)["sens"]["lineX"]
    ins.locator(".pSens").fill("150")
    settled(app)
    x150 = state(app)["sens"]["lineX"]
    assert x0 <= x50 <= x150 and x0 < x150                                   # a higher sensitivity: the line moves to the weak side


def test_the_loudness_count_follows_the_slider_preview(app, hist):
    sid, ins = hist
    n0 = state(app)["loud"]["n"]
    sens = ins.locator(".pSens")
    sens.scroll_into_view_if_needed()
    x0, y = thumb_x(app, 50)
    app.mouse.move(x0, y)
    app.mouse.down()
    x1, _ = thumb_x(app, 150)
    app.mouse.move(x1 + 20, y, steps=6)
    wait_until(lambda: state(app)["hits"] is not None and state(app)["hits"] > n0, msg="the preview")
    wait_until(lambda: state(app)["loud"]["n"] == js(app, "() => S.events['Kick In'].length"), msg="the loudness count with the preview")
    assert state(app)["loud"]["n"] > n0
    assert f"{state(app)['loud']['n']} hits" in js(app, "() => document.querySelector('.hgLoudInfo').textContent")
    app.mouse.up()
    settled(app)


def test_the_velocity_fields_wait_for_a_running_analysis(app, hist):
    sid, ins = hist
    js(app, "() => { analysisRunning = true; }")
    try:
        ins.locator(".pVLo").fill("-50")
        ins.locator(".pVLo").blur()
        expect(app.locator("#stMsg")).to_contain_text("wait for the analysis")
        assert js(app, "() => S.tracks['Kick In'].params.vel_lo_db") == -40
        expect(ins.locator(".pVLo")).to_have_value("-40")
        assert invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]["vel_lo_db"] == -40
    finally:
        js(app, "() => { analysisRunning = false; }")


# ---------- fix round 2 (re-review of 92510a3) ----------

def test_undo_and_redo_of_a_re_pick_reach_the_service_in_order(app, hist):
    sid, ins = hist
    ins.locator(".pSens").fill("120")
    settled(app)
    # the first params call is slow: without the queue the redo's call would arrive first and lose
    js(app, """() => { const orig = B.paramsSet; let n = 0; window.__origParamsSet = orig;
        B.paramsSet = (...a) => (++n === 1 ? new Promise(r => setTimeout(r, 500)).then(() => orig(...a)) : orig(...a)); }""")
    try:
        js(app, "() => { undo(); redo(); }")
        js(app, "async () => { await HG.queue['Kick In']; }")
        app.wait_for_timeout(800)                                       # the slow first call is surely in by now
        assert invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]["sensitivity"] == 120
    finally:
        js(app, "() => { B.paramsSet = window.__origParamsSet; }")


def test_undo_of_a_re_pick_waits_while_a_re_pick_is_stored(app, hist):
    sid, ins = hist
    ins.locator(".pSens").fill("100")
    settled(app)
    n = js(app, "() => S.undo.length")
    js(app, """() => { const orig = B.repick; window.__origRepick = orig;
        B.repick = (...a) => new Promise(r => setTimeout(r, 600)).then(() => orig(...a)); }""")
    try:
        ins.locator(".pSens").fill("120")
        js(app, "() => undo()")                                          # the store of 120 is on its way
        assert js(app, "() => S.undo.length") == n
        expect(app.locator("#stMsg")).to_contain_text("undo and redo wait")
        settled(app)
        assert js(app, "() => S.undo.length") == n + 1
        js(app, "() => undo()")                                          # now it goes
        assert js(app, "() => S.undo.length") == n
        expect(inspect_track(app, "Kick In").locator(".pSens")).to_have_value("100")
    finally:
        js(app, "() => { B.repick = window.__origRepick; }")


def test_a_refused_analysis_shows_the_service_text(app, hist):
    sid, ins = hist
    js(app, """() => { window.__origAnalyze = B.analyze;
        B.analyze = () => Promise.reject(Object.assign(new Error("409: a re-pick is being stored: try again"), {status: 409})); }""")
    try:
        js(app, "() => runAnalysis(['Kick In'], false)")
        expect(app.locator("#stMsg")).to_have_text("analyze failed: 409: a re-pick is being stored: try again")
        assert js(app, "() => analysisRunning") is False
    finally:
        js(app, "() => { B.analyze = window.__origAnalyze; }")


def test_a_refused_velocity_change_puts_the_fields_and_velocities_back(app, hist):
    sid, ins = hist
    before = page_events(app)
    js(app, """() => { window.__origVelSet = B.velocitySet;
        B.velocitySet = () => Promise.reject(Object.assign(new Error("409: a re-pick is being stored: try again"), {status: 409})); }""")
    try:
        ins.locator(".pVLo").fill("-50")
        ins.locator(".pVLo").blur()
        expect(app.locator("#stMsg")).to_contain_text("a re-pick is being stored")
        expect(ins.locator(".pVLo")).to_have_value("-40")
        assert js(app, "() => S.tracks['Kick In'].params.vel_lo_db") == -40
        assert state(app)["loud"]["floor"] == -40
        assert page_events(app) == before
    finally:
        js(app, "() => { B.velocitySet = window.__origVelSet; }")


def test_the_loudness_axis_spans_the_actual_levels_and_holds_the_handles(app, hist):
    """P4-25: x range = the dBFS range of the hits plus 2 dB, widened to hold the floor, the ceiling and the cut."""
    sid, ins = hist
    lo = state(app)["loud"]
    levels = [l for l in invoke(app, "track_histograms", sid=sid, name="Kick In")["loudness"]["levels"] if math.isfinite(l)]
    pts = levels + [lo["floor"], lo["ceiling"]]
    r = lo["range"]
    assert r["lo"] <= min(pts) and r["hi"] >= max(pts) and r["hi"] <= 0
    assert r["lo"] >= min(pts) - 3 and r["hi"] - r["lo"] >= 12                 # tight: not the old fixed -60..0 (unless the levels need it)
    assert r["lo"] != -60 or min(pts) < -57


# ---------- A4: the model-free re-pick ----------

def test_track_repick_picks_from_the_kept_candidates_without_the_model(app, make_project, tmp_path):
    """A4: without the model the slider and `track_repick` pick from the kept candidates:
    no 409, the hit count is monotonic over three settings and equals the analysis result
    at the default, and the detection histogram reads the candidates."""
    sid = open_project(app, build(make_project, tmp_path))
    analyse(app)
    ins = inspect_track(app, "Kick In")
    app.wait_for_function("() => DSHist.state().detection === 'ok' && DSHist.state().loud && DSHist.state().loud.n > 0")
    data = invoke(app, "track_histograms", sid=sid, name="Kick In")["detection"]
    assert data["state"] == "ok"
    cands = data["candidates"]
    at_default = service_events_raw(app, sid)
    assert at_default, "the analysis found the hits"
    # every hit of the analysis is a candidate on at the default or looser
    for e in at_default:
        assert any(abs(c[0] - e["t"]) < 0.021 for c in cands), (e, cands)
    # the slider over three settings: no 409, monotonic, the default reproduces the analysis
    counts = []
    for v in (0, 50, 150):
        r = invoke(app, "track_repick", sid=sid, name="Kick In", params=params_at(app, v), store=False)
        counts.append(r["n_events"])
    assert counts[0] <= counts[1] <= counts[2], counts
    assert counts[1] == len(at_default), (counts, at_default)
    # a stored re-pick keeps working (the model-free path stores as an analysis does)
    stored = invoke(app, "track_repick", sid=sid, name="Kick In", params=params_at(app, 150), store=True)
    assert len(invoke(app, "events_get", sid=sid, name="Kick In")["events"]) == counts[2]
    assert "entry" in stored
    # the store is a full citizen: the hits and the params are in the document; the page takes
    # them over on its next load (a store through the bridge gives no undo step of its own —
    # the one-step undo of a store the page made is the drag test's)
    js(app, "() => edReload(S.sid, 'Kick In')")
    assert service_tv(app, sid) == [{"t": e["t"], "v": e["v"]} for e in stored["events"]]
    p = invoke(app, "tracks_get", sid=sid)["tracks"]["Kick In"]["params"]
    assert p["sensitivity"] == 150 and p["thresholds"] == js(app, "v => sensThresholds(v)", 150)
