"""P4-37b: Remove Example (Hits > Remove Example, Option+K; Inspector Learn buttons Learn This, Forget This, Remove Example, Forget All).
Remove Example removes the Learn example at the selected hit or range, or the selected mark bar; one undo step. K then Option+K
on the same hit gives the state before; a hit without an example changes nothing (no undo step, a status message).
Every wait is on the page's own state; each test has a planted-fault check (see the comments)."""
from playwright.sync_api import expect

import pytest

from app_support import inspect_track, menu
from test_learn import HITS, doc_hits, js, learn_of, lrn, marks, mark, msg, page_times, select, settled  # noqa: F401  (the fixture)



def pick(page, index):
    page.evaluate("i => { S.sel = {track: 'Kick In', index: i}; invalidateHits('Kick In'); updateStatus(); }", index)


def forget_this(page):
    page.keyboard.press("Alt+K")
    settled(page)


def test_k_then_option_k_on_the_same_hit_gives_the_state_before(lrn):
    page, sid, _ = lrn
    before = learn_of(page, sid)
    pick(page, 2)
    mark(page)
    assert len(marks(page, sid)) == 1
    forget_this(page)
    assert marks(page, sid) == [], "the example is gone"
    assert "forgot 1 example" in msg(page)
    assert (learn_of(page, sid) or {}).get("marks", []) == (before or {}).get("marks", [])
    page.keyboard.press("Z")                              # one undo step brings the example back
    settled(page)
    assert [round(a, 3) for m in marks(page, sid) for a in m["at"]] == [1.5]


def test_forget_this_on_a_hit_without_an_example_changes_nothing(lrn):
    page, sid, _ = lrn
    pick(page, 0)
    mark(page)
    state = learn_of(page, sid)
    pick(page, 3)                                         # the hit at 2.0 s has no example
    page.evaluate("() => { window.__undo0 = S.undo ? S.undo.length : null; }")
    forget_this(page)
    assert learn_of(page, sid) == state
    assert "no Learn example" in msg(page)
    assert js(page, "() => S.undo ? S.undo.length : null") == js(page, "() => window.__undo0"), "no undo step"
    page.keyboard.press("Escape")
    js(page, "() => { S.sel = null; LN.sel = null; LN.mark = null; }")
    forget_this(page)                                     # nothing selected: a message, nothing changes
    assert learn_of(page, sid) == state
    assert "select a hit, a range or a mark" in msg(page)


def test_forget_this_takes_a_range_and_a_selected_mark_bar_and_the_menu_item_runs_it(lrn):
    page, sid, _ = lrn
    select(page, "Kick In", 0.95, 1.55)
    mark(page)                                            # 1.0 and 1.5
    pick(page, 5)
    mark(page, "Shift+K")                                 # 3.0 as not
    select(page, "Kick In", 0.95, 1.05)
    assert menu(page, "learn_forget_this") is True        # the menu item: the example at 1.0 only
    settled(page)
    assert sorted(round(a, 3) for m in marks(page, sid) for a in m["at"]) == [1.5, 3.0]
    select(page, "Kick In", 1.4, 3.1)
    forget_this(page)                                     # a range: both kinds
    assert marks(page, sid) == []
    page.keyboard.press("Z"); settled(page)
    assert len(marks(page, sid)) == 2
    js(page, "() => { LN.sel = null; S.sel = null; lnSetMark('Kick In', 'm1'); }")
    forget_this(page)                                     # a selected mark bar goes whole
    assert all(m["id"] != "m1" for m in marks(page, sid))


def test_remove_example_drops_the_learned_hit_from_the_lane_at_once(lrn):
    # the sweep of the service (P4-38b) deletes the learned hit with the example; the page must follow at once,
    # not keep the hit in the lane until the next poll
    page, sid, _ = lrn
    n = len(doc_hits(page, sid))
    js(page, "() => { LN.sel = { track: 'Kick In', t0: 4.4, t1: 4.45 }; S.sel = null; learnMark('this'); }")
    settled(page)
    assert len(doc_hits(page, sid)) == n + 1 and len(page_times(page)) == n + 1   # the learned hit IS in the lane (no wait)
    js(page, "() => { const i = S.events['Kick In'].findIndex(e => e.learned); S.sel = { track: 'Kick In', index: i }; invalidateHits('Kick In'); updateStatus(); }")
    page.keyboard.press("Alt+K")
    settled(page)
    assert marks(page, sid) == []
    page.wait_for_function("n => S.events['Kick In'].length === n", arg=n, timeout=1000)   # within 1 s, not 5 s
    assert len(page_times(page)) == n, "the lane has no learned hit any more"
    assert [e for e in doc_hits(page, sid) if e.get("learned")] == [] and len(doc_hits(page, sid)) == n   # events_get agrees
    # one undo brings the example and the hit back
    page.keyboard.press("Z"); settled(page)
    assert len(marks(page, sid)) == 1 and len(doc_hits(page, sid)) == n + 1 and len(page_times(page)) == n + 1


def test_the_inspector_buttons_are_this_not_forget_this_forget_all(lrn):
    page, sid, _ = lrn
    inspect_track(page, "Kick In")
    pick(page, 1)
    mark(page)
    row = page.locator("#insParams .lnBtns")
    expect(row.locator("button")).to_have_text(["Learn This", "Forget This", "Remove Example", "Forget All"])
    row.locator(".lnForgetThis").click()
    settled(page)
    assert marks(page, sid) == []
    mark(page)
    row.locator(".lnForget").click()                      # Forget All: unchanged
    settled(page)
    assert marks(page, sid) == []
