"""P4-6c: the Learn filter in the page (concept-learn-mode.md 2.4, 2.5, 3.2 to 3.6). The filter runs in the service: a
Learn change with the filter on re-picks the track there, and a hit the filter takes out stays a hit marked
`"filtered": true` (grey in the lane, skipped by Tab, out of the output). The fixture builds one kick track with REAL
hits (short attacks, loud) and BLEED-like ones (long attacks, a phase difference, softer), analyses it and sets the
Sensitivity to 150, so every detection candidate is a hit.

Every wait is on the page's own state: `DSLearn.idle()` (the Learn queue and the hit edits), `DSLearn.scoresReady()`
(the scores of the current learn object), `hgRepickBusy()` (the Sensitivity re-pick). No sleeps. Each test has a
planted-fault check: an assertion that fails if the feature it tests were missing."""
import pytest

from app_support import inspect_track, invoke, open_project
from helpers import synth_track, write_wav
from test_subtracks import analyse, has_model

REAL = [0.5, 1.5, 2.5, 3.5]      # the real hits: short attacks, loud
BLEED = [1.0, 2.0, 3.0]          # the bleed-like ones: long attacks, a phase difference, softer
DUR = 6.0
NAME = "Kick In"


def build(make_project, tmp_path, name="LearnF"):
    src = tmp_path / f"src_{name}"
    src.mkdir()
    on = sorted(REAL + BLEED)
    x, _ = synth_track(on, DUR, gains_db=[-4.0 if t in REAL else -9.0 for t in on],
                       attacks_ms=[0.5 if t in REAL else 12.0 for t in on],
                       phases=[0.0 if t in REAL else 0.35 for t in on])
    specs = [(NAME, write_wav(src / f"{NAME}.wav", x), "kick", 36, None)]
    return make_project.builder.build_project(tmp_path / f"projects_{name}", name, specs, "copied")


def js(page, expr, arg=None):
    return page.evaluate(expr, arg)


def idle(page):
    """Every queued Learn change, its re-pick and the hit edits are applied."""
    js(page, "() => DSLearn.idle()")


@pytest.fixture
def lrf(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path))
    analyse(app)
    inspect_track(app, NAME)
    # the Sensitivity to 150: every candidate is a hit; the change stores a re-pick (HG's pump)
    app.locator("#insParams .pSens").fill("150")
    app.locator("#insParams .pSens").dispatch_event("change")
    app.wait_for_function("() => !hgRepickBusy()")
    js(app, "() => { if (document.activeElement && document.activeElement.blur) document.activeElement.blur(); }")   # K must not land in the slider
    assert invoke(app, "tracks_get", sid=sid)["tracks"][NAME]["params"]["sensitivity"] == 150
    return app, sid


def events(page):
    """The page's hits: [(t, filtered)]."""
    return [tuple(x) for x in js(page, "n => S.events[n].map(e => [e.t, !!e.filtered])", NAME)]


def stored(page, sid):
    return [(e["t"], bool(e.get("filtered"))) for e in invoke(page, "events_get", sid=sid, name=NAME)["events"]]


def grey_times(page):
    return [t for t, f in events(page) if f]


def near(ts, t, tol=0.03):
    return any(abs(x - t) <= tol for x in ts)


def learn_of(page, sid):
    return invoke(page, "tracks_get", sid=sid)["tracks"][NAME].get("learn")


def scores(page):
    return js(page, "n => DSLearn.scoresReady(n)", NAME)


def hit_near(page, t):
    return js(page, "([n, t]) => S.events[n].findIndex(e => Math.abs(e.t - t) < 0.03)", [NAME, t])


def mark_hit(page, t, key):
    """Select the hit nearest to t (as a click does) and press K or Shift+K."""
    i = hit_near(page, t)
    assert i >= 0, f"no hit near {t}"
    js(page, "([n, i]) => { lnSetSel(null); S.sel = { track: n, index: i }; S.playhead = S.events[n][i].t; drawInfo(); }", [NAME, i])
    page.keyboard.press(key)
    idle(page)


def undo(page):
    page.keyboard.press("Z")
    idle(page)


def redo(page):
    page.keyboard.press("Shift+Z")
    idle(page)


def greys_in_preview(page):
    """The hits the lanes draw grey right now: the stored marks, or the live preview of a threshold drag."""
    return js(page, "n => S.events[n].filter(e => DSLearn.greyHit(n, e.t)).map(e => e.t)", NAME)


def hist_box(page):
    loc = page.locator("#insParams .lnHist")
    loc.scroll_into_view_if_needed()
    return loc.bounding_box()


def test_the_first_shift_k_turns_the_filter_on_and_the_marked_hit_goes_grey(lrf):
    app, sid = lrf
    before = events(app)
    assert not grey_times(app) and len(before) >= len(REAL + BLEED)
    mark_hit(app, 1.0, "Shift+K")
    learn = learn_of(app, sid)
    assert learn["on"] is True and learn["auto"] is True
    assert "Learn filter on" in app.locator("#stMsg").inner_text()
    grey = grey_times(app)
    assert near(grey, 1.0), grey                                  # the marked hit is grey
    assert not any(near(grey, t, 0.005) for t in REAL), grey      # no real hit goes (one Forget This: near copies only)
    assert [t for t, _ in events(app)] == [t for t, _ in before]  # the filter marks hits, it never moves one
    assert events(app) == stored(app, sid)                        # planted fault: the page shows what the service stored
    assert js(app, "n => DSLearn.greyHit(n, 1.0)", NAME)          # drawn grey


def test_k_alone_never_turns_the_filter_on(lrf):
    app, sid = lrf
    mark_hit(app, 0.5, "K")
    learn = learn_of(app, sid)
    assert learn["marks"] and learn["on"] is False                 # planted fault: the mark is there
    assert not grey_times(app) and not any(f for _, f in stored(app, sid))
    assert js(app, "() => document.querySelector('#insParams .lnHist').classList.contains('hidden')")   # off: no histogram


def test_one_undo_reverts_the_mark_and_the_filter_and_redo_brings_both(lrf):
    app, sid = lrf
    before = events(app)
    mark_hit(app, 1.0, "Shift+K")
    after = events(app)
    assert grey_times(app)
    undo(app)
    assert not (learn_of(app, sid) or {}).get("marks")
    assert events(app) == before and stored(app, sid) == before   # the hits of the filter off, in the service too
    redo(app)
    assert learn_of(app, sid)["on"] is True and events(app) == after


def test_k_and_shift_k_together_follow_the_examples(lrf):
    app, sid = lrf
    mark_hit(app, 0.5, "K")
    mark_hit(app, 1.0, "Shift+K")
    sc = scores(app)
    assert sc["auto_on"] is True and sc["threshold"] == sc["suggest"] == 0.4   # both kinds: the 0.4 rule
    grey = grey_times(app)
    assert near(grey, 1.0) and not near(grey, 0.5, 0.005)         # the not example goes, the this example stays
    # the stored grey set is the score rule: every hit whose candidate scores under 0.4, plus the not example
    want = js(app, """n => S.events[n].filter(e => { const s = lnScoreAt(n, e.t - lnShift(n));
      return (s !== null && s < 0.4 && Math.abs(e.t - 0.5) > 0.03) || Math.abs(e.t - 1.0) < 0.03; }).map(e => e.t)""", NAME)
    assert sorted(grey) == sorted(want)                           # planted fault: a different threshold differs here


def test_the_histogram_shows_the_scores_the_ticks_the_line_and_the_counts(lrf):
    app, sid = lrf
    mark_hit(app, 0.5, "K")
    mark_hit(app, 1.0, "Shift+K")
    sc = scores(app)
    assert sc["state"] == "ok" and len(sc["candidates"]) >= len(REAL + BLEED)
    assert len(sc["examples"]["this"]) == 1 and len(sc["examples"]["not"]) == 1
    js(app, "n => lnHistDraw(n)", NAME)
    px = js(app, """th => { const c = document.querySelector('#insParams .lnHist'), x = c.getContext('2d');
      const d = x.getImageData(0, 0, c.width, c.height).data; let n = 0; for (let i = 3; i < d.length; i += 4) if (d[i]) n++;
      const lx = Math.round(th * c.width), col = x.getImageData(lx, 2, 1, 1).data;
      return { n, line: [col[0], col[1], col[2]] }; }""", sc["threshold"])
    assert px["n"] > 100                                          # bars, ticks and the line are drawn
    assert px["line"][0] > 180 and px["line"][1] < 120            # the red threshold line at the threshold
    grey = len(grey_times(app))
    info = app.locator("#insParams .lnHistInfo").inner_text()
    assert info.startswith(f"{len(events(app)) - grey} hits · Learn takes {grey} out") and "auto" in info
    # owner check 2026-10-15: a hit the bleed rule already took out is not counted as taken by Learn
    moved = js(app, """n => { const e = S.events[n].find(x => lnGreyHit(n, x)); if (!e) return false; e.bleed = true; lnHistDraw(n); return true; }""", NAME)
    assert moved
    info2 = app.locator("#insParams .lnHistInfo").inner_text()
    assert info2.startswith(f"{len(events(app)) - grey} hits · Learn takes {grey - 1} out"), info2
    js(app, """n => { for (const e of S.events[n]) delete e.bleed; lnHistDraw(n); }""", NAME)
    # planted fault: the filter off hides the histogram
    app.locator("#insParams .lnFilterBox").uncheck()
    idle(app)
    assert js(app, "() => document.querySelector('#insParams .lnHist').classList.contains('hidden')")


def test_the_threshold_drag_previews_live_and_commits_one_undo_step_on_release(lrf):
    app, sid = lrf
    mark_hit(app, 1.0, "Shift+K")
    scores(app)
    before_grey = grey_times(app)
    box = hist_box(app)
    y = box["y"] + box["height"] / 2
    app.mouse.move(box["x"] + box["width"] * 0.05, y)
    app.mouse.down()
    app.mouse.move(box["x"] + box["width"] * 0.9, y, steps=4)
    # during the drag: nothing stored yet, the lanes preview the live threshold
    assert learn_of(app, sid)["auto"] is True
    assert grey_times(app) == before_grey
    preview = greys_in_preview(app)
    assert preview != before_grey                                   # the preview follows the drag
    app.mouse.up()
    idle(app)
    learn = learn_of(app, sid)
    assert learn["auto"] is False and abs(learn["threshold"] - 0.9) < 0.01
    grey = grey_times(app)
    assert sorted(grey) == sorted(preview)                          # committed: the service stores exactly the preview
    assert len(grey) > len(before_grey)
    undo(app)                                                       # one undo step: the threshold and the hits
    learn = learn_of(app, sid)
    assert learn["auto"] is True and grey_times(app) == before_grey
    # the Auto link: after a drag, the threshold follows the examples again
    app.mouse.move(box["x"] + box["width"] * 0.05, y)
    app.mouse.down()
    app.mouse.move(box["x"] + box["width"] * 0.9, y, steps=2)
    app.mouse.up()
    idle(app)
    assert learn_of(app, sid)["auto"] is False
    app.locator("#insParams .lnAuto").click()
    idle(app)
    assert learn_of(app, sid)["auto"] is True and grey_times(app) == before_grey


def test_the_filter_checkbox_toggles_with_one_undo_step_and_off_is_the_unfiltered_hits(lrf):
    app, sid = lrf
    plain = events(app)
    mark_hit(app, 1.0, "Shift+K")
    on = events(app)
    box = app.locator("#insParams .lnFilterBox")
    assert box.is_checked() and grey_times(app)
    box.uncheck()
    idle(app)
    assert learn_of(app, sid)["on"] is False
    assert events(app) == plain and stored(app, sid) == plain     # off: as if Learn never ran
    undo(app)
    assert learn_of(app, sid)["on"] is True and events(app) == on and box.is_checked()


def test_forget_clears_the_filter_and_undo_brings_it_back(lrf):
    app, sid = lrf
    plain = events(app)
    mark_hit(app, 1.0, "Shift+K")
    on = events(app)
    app.locator("#insParams .lnForget").click()
    idle(app)
    assert not learn_of(app, sid)["marks"] and events(app) == plain
    undo(app)
    assert learn_of(app, sid)["marks"] and events(app) == on


def test_tab_skips_the_grey_hits(lrf):
    app, sid = lrf
    mark_hit(app, 1.0, "Shift+K")
    js(app, "n => lnHistSetThreshold(n, 0.495)", NAME)            # most bleed-like hits go grey
    idle(app)
    evs = events(app)
    grey = [t for t, f in evs if f]
    i = next(k for k, (t, f) in enumerate(evs) if f and any(not g for _, g in evs[k + 1:]))
    want = next(t for t, f in evs[i + 1:] if not f)
    js(app, "t => { S.sel = null; lnSetSel(null); S.playhead = t - 0.001; }", evs[i][0])
    app.keyboard.press("Tab")
    got = js(app, "() => S.sel && S.events[S.sel.track][S.sel.index].t")
    assert got == want and got not in grey                        # planted fault: the grey hit right after the playhead is skipped
    assert evs[i][0] < got


def test_the_info_line_shows_the_sim_and_why_a_hit_is_grey(lrf):
    app, sid = lrf
    js(app, "() => { if (!S.infoLine) setInfoLine(true); }")
    mark_hit(app, 1.0, "Shift+K")
    js(app, "n => lnHistSetThreshold(n, 0.48)", NAME)
    idle(app)
    scores(app)
    seen = set()
    for t, f in events(app):
        js(app, "([n, i]) => { S.sel = { track: n, index: i }; drawInfo(); }", [NAME, hit_near(app, t)])
        txt = app.locator("#ilLearn").inner_text()
        if near([1.0], t, 0.005):
            assert txt == "not this"
        elif f:
            assert txt.startswith("filtered: sim ") and txt.endswith("< 0.48"), txt
            seen.add("grey")
        else:
            assert txt.startswith("sim ") and "filtered" not in txt, txt   # planted fault: a kept hit never says filtered
            seen.add("kept")
    assert seen == {"grey", "kept"}


def test_a_hit_edit_is_never_filtered(lrf):
    app, sid = lrf
    mark_hit(app, 1.0, "Shift+K")
    top = max(row[2] for row in scores(app)["candidates"])          # the highest score the page reports
    js(app, "([n, th]) => lnHistSetThreshold(n, th)", [NAME, round(top + 0.005, 3)])
    idle(app)
    # the Forget-only attack gate (#293 B8) keeps the REAL hits (short attacks, far from the mark's): the BLEED hits go grey
    assert all(f for t, f in events(app) if near(BLEED, t))
    i = hit_near(app, 2.0)                                        # a BLEED hit: the filter takes it out
    t = js(app, "([n, i]) => S.events[n][i].t", [NAME, i])
    invoke(app, "events_edit", sid=sid, name=NAME, ops=[{"op": "move", "index": i, "t": t + 0.002}])
    # the re-pick of the next Learn change keeps the moved hit, and never filters it
    js(app, "([n, th]) => lnHistSetThreshold(n, th)", [NAME, round(top + 0.002, 3)])
    idle(app)
    moved = [e for e in invoke(app, "events_get", sid=sid, name=NAME)["events"] if "orig" in e]
    assert len(moved) == 1 and not moved[0].get("filtered"), moved
    kept = [t for t, f in events(app) if not f]
    assert all(near(kept, r, 0.005) for r in REAL) and near(kept, 2.0, 0.03)   # the REAL hits and the moved hit stay out of the grey (planted fault: the moved hit would be grey without the edit)


# ---------------------------------------------------------------------------- fix round

def test_a_learn_change_keeps_hand_edits_and_undo_restores_the_snapshot(lrf):
    app, sid = lrf
    mark_hit(app, 1.0, "Shift+K")
    # a hand deletion (the page's Delete) and a velocity edit (the service, then the page reloads)
    js(app, "([n, i]) => edDelete(n, S.events[n][i].id)", [NAME, hit_near(app, 0.5)])
    i = hit_near(app, 1.5)
    invoke(app, "events_edit", sid=sid, name=NAME, ops=[{"op": "velocity", "index": i, "v": 127}])
    js(app, "n => edReload(S.sid, n)", NAME)
    edited = js(app, "n => lnSnap(n)", NAME)
    invoke(app, "track_learn_set", sid=sid, name=NAME, learn=dict(learn_of(app, sid), add=1.0))   # P4-38b: pin the Add threshold, the test is about the edits
    mark_hit(app, 2.5, "K")                                       # a Learn change with the filter on
    assert hit_near(app, 0.5) < 0, "the deleted hit stays deleted"
    assert js(app, "([n, t]) => S.events[n].find(e => Math.abs(e.t - t) < 0.03).v", [NAME, 1.5]) == 127
    evs = invoke(app, "events_get", sid=sid, name=NAME)["events"]
    # P4-38b: the K step's added hits (learned) are new; every non-learned hit keeps its time
    assert [e["t"] for e in evs if not e.get("learned")] == [e["t"] for e in edited]
    step = js(app, "() => S.undo[S.undo.length - 1]")
    assert step["type"] == "learn" and step["evBefore"] == edited  # the step holds the snapshot (concept 2.5)
    js(app, "() => { window.__ls = []; const f = B.learnSet; B.learnSet = (...a) => { window.__ls.push(a[3] || null); return f(...a); }; }")
    undo(app)                                                     # the K: the snapshot, exactly
    assert js(app, "() => window.__ls") == [edited]               # sent as the hits to store
    assert js(app, "n => lnSnap(n)", NAME) == edited
    stored = invoke(app, "events_get", sid=sid, name=NAME)["events"]
    assert [(e["t"], e["v"], bool(e.get("filtered"))) for e in stored] == [(e["t"], e["v"], bool(e.get("filtered"))) for e in edited]
    redo(app)
    assert hit_near(app, 0.5) < 0 and len(learn_of(app, sid)["marks"]) == 2


def test_an_edit_is_refused_while_a_learn_change_is_on_its_way(lrf):
    app, sid = lrf
    before = events(app)
    js(app, "() => { LN.busy++; }")                               # a Learn change being applied (one that still waits for the edits on their way does not refuse them)
    ok = js(app, "([n, i]) => edDelete(n, S.events[n][i].id)", [NAME, hit_near(app, 0.5)])
    js(app, "() => { LN.busy--; }")
    assert ok is False and "a Learn change is being applied" in app.locator("#stMsg").inner_text()
    assert events(app) == before == stored(app, sid)
    # planted fault: with nothing on its way the same delete runs
    assert js(app, "([n, i]) => edDelete(n, S.events[n][i].id)", [NAME, hit_near(app, 0.5)]) is True
    assert hit_near(app, 0.5) < 0


def test_the_hints_for_not_this_alone_and_k_with_the_filter_off(lrf):
    app, sid = lrf
    mark_hit(app, 0.5, "K")
    assert "Learn filter off: Shift+K on bleed or tick Filter to use it" in app.locator("#stMsg").inner_text()
    hint = app.locator("#insParams .lnHintMsg")
    assert hint.is_hidden()
    undo(app)
    mark_hit(app, 1.0, "Shift+K")
    want = "Forget This alone takes only this hit and near copies: add a Learn This example (K) so similar bleed goes too."
    assert want in app.locator("#stMsg").inner_text()
    assert hint.is_visible() and hint.inner_text() == want
    mark_hit(app, 0.5, "K")                                       # planted fault: with a This example the hint goes
    assert hint.is_hidden() and want not in app.locator("#stMsg").inner_text()


def test_a_not_example_still_in_the_output_is_tinted_not_grey_and_the_menu_follows(lrf):
    app, sid = lrf
    mark_hit(app, 1.0, "Shift+K")
    assert js(app, "() => DSShell.menuState().learnFilter") is True
    app.locator("#insParams .lnFilterBox").uncheck()
    idle(app)
    assert js(app, "() => DSShell.menuState().learnFilter") is False   # Hits > Learn Filter shows the state
    i = hit_near(app, 1.0)
    js(app, "() => { S.sel = null; TM.ui.grid = false; tmApplyUi(true); }")
    js(app, "([n, i]) => { const t = S.events[n][i].t; S.view = viewClamp(t - 0.05, t + 0.05); invalidate(F.VIEW); flushFrame(); }", [NAME, i])
    x = js(app, "([n, i]) => Math.round(tToX(S.events[n][i].t, laneWidth())) + 0.5", [NAME, i])
    h = js(app, "n => document.querySelector(`.track[data-name=\"${n}\"] .spike`).clientHeight", NAME)
    p = js(app, """([n, x, y]) => { const c = document.querySelector(`.track[data-name="${n}"] .spike`), d = window.devicePixelRatio || 1;
      return [...c.getContext('2d').getImageData(Math.floor(x * d), Math.round(y * d), 1, 1).data]; }""", [NAME, x, h * 0.6])
    tint, grey = (0xb8, 0x70, 0x6c), (0x4a, 0x50, 0x5c)
    assert all(abs(a - b) <= 30 for a, b in zip(p[:3], tint)), p   # the not tint: exported, Tab stops on it
    assert not js(app, "n => DSLearn.greyHit(n, 1.0)", NAME)
    app.locator("#insParams .lnFilterBox").check()                  # planted fault: filtered again, it is the grey
    idle(app)
    js(app, "() => flushFrame()")
    p = js(app, """([n, x, y]) => { const c = document.querySelector(`.track[data-name="${n}"] .spike`), d = window.devicePixelRatio || 1;
      return [...c.getContext('2d').getImageData(Math.floor(x * d), Math.round(y * d), 1, 1).data]; }""", [NAME, x, h * 0.6])
    assert all(abs(a - b) <= 30 for a, b in zip(p[:3], grey)), p


def test_the_histogram_is_drawn_when_the_learn_section_opens(lrf):
    """Owner check 2026-10-15: a Learn change while the section was closed drew the histogram at width 0, and opening
    the section left it empty. Opening must draw it, and the status line counts like the Learn row."""
    app, sid = lrf
    head = app.locator('#inspector .insSec[data-sec="learn"] .insSecHead')
    if js(app, "() => !!INSSEC.open.learn"):
        head.click()
    mark_hit(app, 1.0, "Shift+K")                      # the change draws the histogram while the section is closed
    scores(app)
    assert js(app, "() => document.querySelector('#inspector .lnHist').width") == 0   # planted fault: the closed draw has no size
    head.click()
    ink = js(app, """() => { const c = document.querySelector('#inspector .lnHist'); if (!c.width) return 0;
      const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let n = 0;
      for (let i = 3; i < d.length; i += 4) if (d[i]) n++; return n; }""")
    assert ink > 0
    info = app.locator("#insParams .lnHistInfo").inner_text()
    counts = info.split(" · ")[:2]
    msg = app.locator("#stMsg").inner_text()
    assert ", ".join(counts) in msg, (info, msg)
