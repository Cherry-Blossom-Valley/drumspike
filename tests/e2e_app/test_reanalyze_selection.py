"""#265: Hits > Re-analyze Selection (and Shift+R, and the range context menu entry).

The item is enabled only while a range selection exists (LN.sel); Shift+R runs it on the bridge's
real detector: only the hits of the range are replaced, the info line names the track and the
range, the Keep dialog opens only when an edited hit is inside the range (its text names the
range), and the run is one undo step "Re-analyze Selection" that restores the hits of the range.
"""
import pytest

from app_support import edits_done, invoke, menu, open_project, shot_dir, wait_until
from helpers import SR, synth_track, write_wav

OUTSIDE = [0.5, 2.5]
INSIDE_AT = 1.5
ALL = sorted(OUTSIDE + [INSIDE_AT])
T0, T1 = 1.2, 2.4
INFO = "re-analyzed 1.200 to 2.400 s"


def js(page, expr):
    return page.evaluate(expr)


def msg(page):
    return js(page, "() => document.querySelector('#stMsg').textContent")


def samples(page, sid, name):
    return sorted(round(e["t"] * SR) for e in invoke(page, "events_get", sid=sid, name=name)["events"])


def top_step(page):
    return js(page, "() => S.undo[S.undo.length - 1]")


def modal_open(page):
    return "hidden" not in (page.locator("#reanalyzeModal").get_attribute("class") or "")


@pytest.fixture
def song(app, make_project, tmp_path):
    """A project whose Kick In holds the hits above; the audio carries onsets at 0.5, 1.5, 2.5."""
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    sid = open_project(app, folder)
    invoke(app, "events_set", sid=sid, name="Kick In",
           events=[{"t": t, "v": 90, "id": f"k{i}"} for i, t in enumerate(ALL)])
    app.evaluate("async () => { await loadProject(); }")
    app.wait_for_function("n => (S.events['Kick In'] || []).length === n", arg=len(ALL))
    return app, sid


def set_range(app, t0=T0, t1=T1):
    app.evaluate(f"() => {{ S.sel = null; lnSetSel('Kick In', {t0}, {t1}); }}")


def test_the_item_is_greyed_without_a_range_and_enabled_with_one(song):
    app, _sid = song
    assert js(app, "() => window.__DS_MENU_STATE__.selRange") is False
    menu(app, "reanalyze_selection")
    wait_until(lambda: "no range selected" in msg(app), msg="the refusal without a range")
    set_range(app)
    assert js(app, "() => window.__DS_MENU_STATE__.selRange") is True
    app.keyboard.press("Escape")   # a click or Escape clears the range again
    assert js(app, "() => window.__DS_MENU_STATE__.selRange") is False


def test_shift_r_runs_the_item_and_the_info_line_names_the_range(song):
    app, sid = song
    set_range(app)
    undo_n = js(app, "() => S.undo.length")
    app.keyboard.press("Shift+R")
    wait_until(lambda: INFO in msg(app), msg="the info line names the track and the range")
    assert "Kick In" in msg(app)
    # the hits outside the range kept their times, the one inside was replaced by the detection
    ts = samples(app, sid, "Kick In")
    assert [t for t in ts if t < int(T0 * SR) or t > int(T1 * SR)] == [int(t * SR) for t in OUTSIDE]
    inside = [t for t in ts if int(T0 * SR) <= t <= int(T1 * SR)]
    assert len(inside) == 1 and abs(inside[0] - int(INSIDE_AT * SR)) <= int(0.005 * SR)
    step = top_step(app)
    assert step["type"] == "reanalyze" and step["track"] == "Kick In"
    assert js(app, "() => S.undo.length") == undo_n + 1
    app.keyboard.press("Z")
    edits_done(app)
    wait_until(lambda: samples(app, sid, "Kick In") == [int(t * SR) for t in ALL],
               msg="the undo restores the hits of the range")


def test_screenshots_the_range_the_context_menu_and_the_info_line(song):
    """The coordinator's shots: the band with the context menu entry, and the info line after a run."""
    app, sid = song
    set_range(app)
    pt = app.evaluate("() => { const el = document.querySelector('.track[data-name=\"Kick In\"] .wave'); "
                      "const r = el.getBoundingClientRect(); "
                      "return {x: GEO.laneX + tToX(1.8, laneWidth()), y: r.y + r.height / 2}; }")
    app.mouse.click(pt["x"], pt["y"], button="right")
    app.locator('#ctxMenu [data-act="resel"]').wait_for()
    app.screenshot(path=str(shot_dir("265-range-and-context-menu.png")))
    app.evaluate("() => { S.sel = null; lnSetSel('Kick In', 1.2, 2.4); }")   # Escape would clear the range
    app.keyboard.press("Shift+R")
    wait_until(lambda: INFO in msg(app), msg="the info line names the range")
    app.screenshot(path=str(shot_dir("265-info-line.png")))


def test_the_keep_dialog_appears_only_with_an_edited_hit_inside_the_range(song):
    app, sid = song
    # an edited hit OUTSIDE the range: no dialog (the whole-track dialog would count it)
    invoke(app, "events_edit", sid=sid, name="Kick In",
           ops=[{"op": "move", "index": 0, "t": OUTSIDE[0] + 0.01}])
    set_range(app)
    menu(app, "reanalyze_selection", wait=False)
    app.wait_for_timeout(500)
    assert not modal_open(app)
    wait_until(lambda: INFO in msg(app), msg="the run without edited hits in the range went straight through")
    # an edited hit INSIDE the range: the dialog opens and names the range
    invoke(app, "events_edit", sid=sid, name="Kick In",
           ops=[{"op": "move", "index": 1, "t": INSIDE_AT + 0.004}])
    set_range(app)
    menu(app, "reanalyze_selection", wait=False)
    app.wait_for_function("() => document.querySelector('#reanalyzeModal') && !document.querySelector('#reanalyzeModal').classList.contains('hidden')")
    assert f"{T0:.3f} to {T1:.3f} s" in app.locator("#reanalyzeMsg").inner_text()
    app.click("#raKeep")
    wait_until(lambda: INFO in msg(app), msg="the Keep answer runs the range")
    ts = samples(app, sid, "Kick In")
    outside = [t for t in ts if t < int(T0 * SR) or t > int(T1 * SR)]
    # the edited hit outside kept its move, the hit inside the range is the detection's
    assert len(ts) == 3 and outside == [int((OUTSIDE[0] + 0.01) * SR), int(OUTSIDE[1] * SR)]
