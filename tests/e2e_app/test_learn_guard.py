"""P4-35: the Learn guard in the page. One Learn step that takes out more than max(10, 25 % of the output hits before)
additional hits asks: "Learn now takes N of M hits out (before: K). The examples may be mixed up." with Keep and Undo.
The fixture has 20 real and 20 bleed-like hits, so one Forget This on a bleed hit takes out all 20 (a planted mass cut).
Every wait is on the page's own state (`DSLearn.idle()`, the dialog); no sleeps. Each test has a planted-fault check."""
import math

import pytest
from playwright.sync_api import expect

from app_support import inspect_track, invoke, open_project
from helpers import synth_track, write_wav
from test_learn_filter import lrf  # noqa: F401  (the fixture)
from test_learn_filter import NAME, events, grey_times, hit_near, idle, js, learn_of, mark_hit, near, scores, stored, undo
from test_subtracks import analyse, has_model

REAL = [0.5 + 0.8 * k for k in range(20)]
BLEED = [0.9 + 0.8 * k for k in range(20)]
DUR = 17.0
DIALOG = "#confirmModal"


@pytest.fixture
def lrg(app, make_project, tmp_path):
    src = tmp_path / "src_guard"
    src.mkdir()
    on = sorted(REAL + BLEED)
    x, _ = synth_track(on, DUR, gains_db=[-4.0 if t in REAL else -9.0 for t in on],
                       attacks_ms=[0.5 if t in REAL else 12.0 for t in on],
                       phases=[0.0 if t in REAL else 0.35 for t in on])
    specs = [(NAME, write_wav(src / f"{NAME}.wav", x), "kick", 36, None)]
    sid = open_project(app, make_project.builder.build_project(tmp_path / "projects_guard", "Guard", specs, "copied"))
    analyse(app)
    inspect_track(app, NAME)
    app.locator("#insParams .pSens").fill("150")
    app.locator("#insParams .pSens").dispatch_event("change")
    app.wait_for_function("() => !hgRepickBusy()")
    js(app, "() => { if (document.activeElement && document.activeElement.blur) document.activeElement.blur(); }")
    assert len(events(app)) >= 36
    return app, sid


def shift_k_on_bleed(app):
    """Shift+K on a bleed hit; the Learn queue is idle when it returns (the dialog may be open)."""
    mark_hit(app, BLEED[0], "Shift+K")


def test_a_mass_cut_shows_the_dialog_and_undo_restores_the_hits_exactly(lrg):
    app, sid = lrg
    before = events(app)
    assert not grey_times(app)
    shift_k_on_bleed(app)
    expect(app.locator(DIALOG)).to_be_visible()
    out = len(grey_times(app))
    assert out >= 11, out                                                 # planted fault: the step really is a mass cut
    msg = app.locator("#confirmMsg").inner_text()
    assert f"Learn now takes {out} of " in msg and "(before: 0)" in msg and "may be mixed up" in msg, msg
    assert app.locator(f"{DIALOG} button").all_inner_texts() == ["Keep", "Undo"]
    app.click(f"{DIALOG} button[data-act='undo']")
    expect(app.locator(DIALOG)).to_be_hidden()
    app.wait_for_function("() => !(S.events[%r] || []).some(e => e.filtered)" % NAME)
    idle(app)
    assert events(app) == before and stored(app, sid) == before           # one Z restored marks and hits exactly
    assert not (learn_of(app, sid) or {}).get("marks")


def test_keep_leaves_the_hits_out_and_the_step_stays_undoable(lrg):
    app, sid = lrg
    before = events(app)
    shift_k_on_bleed(app)
    expect(app.locator(DIALOG)).to_be_visible()
    out = grey_times(app)
    app.click(f"{DIALOG} button[data-act='keep']")
    expect(app.locator(DIALOG)).to_be_hidden()
    idle(app)
    assert grey_times(app) == out and len(out) >= 11                      # planted fault: Keep changes nothing
    assert stored(app, sid) == events(app)
    undo(app)                                                             # the normal undo step is still there
    assert events(app) == before


def test_a_threshold_drag_that_takes_many_hits_out_shows_no_dialog(lrg):
    app, sid = lrg
    # a "not" example on a real stroke: the attack gate (#293 B8) keeps the bleed hits and leaves the other real hits open
    mark_hit(app, REAL[0], "Shift+K")
    expect(app.locator(DIALOG)).to_be_hidden()
    n = len(grey_times(app))
    limit = js(app, "b => lnGuardLimit(b)", len(events(app)) - n)       # the limit of the dialog, read from the page
    need = math.ceil(limit) + 4                                           # limit + 4 hits must go out in one drag
    grey = grey_times(app)
    # the open hits: the candidates the gate does not keep and that are not grey yet, lowest score first
    open_scores = sorted(row[2] for row in scores(app)["candidates"] if row[4] is False and not near(grey, row[0], 0.005))
    th = round(open_scores[need - 1] + 0.001, 3)                          # just above the need-th lowest: that many go grey
    js(app, "([n, th]) => lnHistSetThreshold(n, th)", [NAME, th])         # the release of a drag: an explicit choice
    idle(app)
    assert len(grey_times(app)) - n >= limit + 4                          # planted fault: the drag took more than the limit out
    expect(app.locator(DIALOG)).to_be_hidden()


def test_a_small_step_shows_no_dialog_and_the_limit_is_max_10_or_25_percent(lrf):
    app, sid = lrf
    mark_hit(app, 1.0, "Shift+K")                                         # 7 hits: this takes a few out, far under 10
    assert grey_times(app)                                                # planted fault: the step did take hits out
    expect(app.locator(DIALOG)).to_be_hidden()
    cuts = "([b, a, h, hb]) => lnGuardCuts({filter: {}, learn_out_before: b, learn_out_after: a, hits: h, hits_before: hb})"
    assert js(app, cuts, [0, 10, 40, 40]) == 0
    assert js(app, cuts, [0, 11, 40, 40]) == 11                           # planted fault: the guard can fire
    assert js(app, cuts, [0, 25, 100, 100]) == 0 and js(app, cuts, [0, 26, 100, 100]) == 26
    assert js(app, cuts, [30, 47, 100, 100]) == 0 and js(app, cuts, [30, 48, 100, 100]) == 18   # 25 % of the 70 output hits before: 17.5
