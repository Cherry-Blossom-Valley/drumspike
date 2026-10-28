"""P4-wf10: Transport > Suspend Follow when Editing.

With the item checked and the transport playing, an add, move or delete of a hit suspends the
follow: the view start does not change for 2 s of playback, then the follow resumes at the next
page (the view turns to the page of the playhead). With the item unchecked the view follows as
today. The check state is a user setting: it persists across a restart. The test avoids frame
counts (the smooth-follow frame test is flaky under load on arm64-mac).
"""
from app_support import invoke, menu, open_project


def play(app):
    app.evaluate("() => { window.__DS_NO_STRIP__ = true; window.__DS_NO_CP__ = true; setFollow('smooth'); }")
    app.click("#playBtn")
    app.wait_for_function("() => S.playing && AC.running")


def zoomed(app):
    """A view that can turn a page (the whole-project view has no next page)."""
    app.evaluate("() => setView(0, 8)")


def stop(app):
    app.click("#playBtn")
    app.wait_for_function("() => !S.playing")
    invoke(app, "transport_stop", close=True)   # the next tests start without an open transport


def watch_views(app):
    """Record every setView (with the playhead of the call) until the test reads the list."""
    app.evaluate("""() => { window.__views = []; const orig = setView;
        window.setView = (a, b) => { window.__views.push([a, b, PH.v]); return orig(a, b); }; }""")


def edit(app, kind):
    """One hit edit while the transport plays (the user paths: the menu items)."""
    if kind == "add":
        menu(app, "add_hit")
    else:
        app.evaluate("() => { S.sel = { track: S.order[0], index: 0 }; }")
        menu(app, "delete_hit" if kind == "delete" else "nudge_later")


def test_an_add_during_playback_holds_the_view_for_two_seconds_then_the_next_page(app, make_project):
    open_project(app, make_project(seconds=30.0))
    menu(app, "suspend_follow")
    assert app.evaluate("() => SETTINGS().ui.suspendFollowOnEdit") is True
    play(app)
    app.wait_for_function("() => S.playhead > 3.0")
    zoomed(app)
    edit(app, "add")
    assert app.evaluate("() => S.followHold") is True
    t0 = app.evaluate("() => S.view.t0")   # read after the edit: the hold has frozen the view
    watch_views(app)   # from here on: no setView but the resume turn
    at = app.evaluate("() => S.followResumeAt")
    assert at is not None and 0 < at - app.evaluate("() => S.playhead") <= 2.0 + 1e-9
    # the view start does not change for 2 s of playback (checked just before the resume: a small margin)
    app.wait_for_function("p => S.playhead > p + 1.5", arg=at - 2.0)
    assert app.evaluate("() => S.view.t0") == t0 and len(app.evaluate("() => window.__views")) == 0
    # then the follow resumes at the next page: one turn, the playhead at the follow fraction of the new view
    app.wait_for_function("() => !S.followHold && S.followResumeAt === null && window.__views.length > 0")
    a, b, ph = app.evaluate("() => window.__views[window.__views.length - 1]")
    assert a != t0 and abs(ph - (a + (b - a) * 0.25)) < 1e-6   # FOLLOW_FRAC
    stop(app)


def test_a_move_and_a_delete_during_playback_suspend_the_follow_too(app, make_project):
    open_project(app, make_project(seconds=30.0))
    menu(app, "suspend_follow")
    menu(app, "add_hit")   # a hit to move and to delete (make_project tracks carry none)
    play(app)
    app.wait_for_function("() => S.playhead > 3.0")
    t0 = app.evaluate("() => S.view.t0")
    watch_views(app)
    edit(app, "move")
    assert app.evaluate("() => S.followHold") is True
    edit(app, "delete")
    assert app.evaluate("() => S.followHold") is True   # the second edit extends the suspension
    app.evaluate("() => { S.followResumeAt = null; S.followHold = false; }")
    assert app.evaluate("() => S.view.t0") == t0 and len(app.evaluate("() => window.__views")) == 0
    stop(app)


def test_unchecked_the_view_follows_as_today(app, make_project):
    open_project(app, make_project(seconds=30.0))
    assert app.evaluate("() => SETTINGS().ui.suspendFollowOnEdit") is False
    play(app)
    app.wait_for_function("() => S.playhead > 3.0")
    zoomed(app)
    t0 = app.evaluate("() => S.view.t0")
    edit(app, "add")
    assert app.evaluate("() => S.followHold") is False
    app.wait_for_function("t0 => S.view.t0 > t0", arg=t0)   # the smooth follow moves the view as before
    stop(app)


def test_the_check_state_persists_across_a_restart(app, make_project):
    open_project(app, make_project())
    menu(app, "suspend_follow")
    assert app.evaluate("() => SETTINGS().ui.suspendFollowOnEdit") is True
    app.reload()
    app.wait_for_function("() => window.DSShell && DSShell.listening()")
    assert app.evaluate("() => SETTINGS().ui.suspendFollowOnEdit") is True
    menu(app, "suspend_follow")
    assert app.evaluate("() => SETTINGS().ui.suspendFollowOnEdit") is False
