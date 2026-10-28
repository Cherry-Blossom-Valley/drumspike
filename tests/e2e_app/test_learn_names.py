"""P4-37b: the owner's names for Learn. Shift+K is "Forget This" (id learn_not), Option+K is
"Remove Example" (id learn_forget_this); "Not This" appears nowhere in the UI. The histogram's
threshold line is labelled "Match" with its value (the info line), and a Forget This click says
how many similar hits the current Match takes. Every test asserts the new behaviour directly, so
a planted fault (the old title, a missing label) fails it."""
import re

import test_learn_filter as lf
from app_support import inspect_track, open_project
from test_learn_filter import NAME, idle, js, lrf, mark_hit  # noqa: F401  (the fixture)
from test_subtracks import has_model

MENU_RS = "apps/desktop/src-tauri/src/menu.rs"


def key_titles(page):
    return {c["id"]: c["title"] for c in page.evaluate("() => DSKeys.COMMANDS")}


def test_the_menu_and_key_titles_are_forget_this_and_remove_example(app):
    t = key_titles(app)
    assert t["learn_not"] == "Forget This", t["learn_not"]            # planted fault: the old "Not This" fails
    assert t["learn_forget_this"] == "Remove Example", t["learn_forget_this"]
    src = open(MENU_RS).read()
    assert 'item("learn_not", "Forget This"' in src
    assert 'item("learn_forget_this", "Remove Example"' in src
    assert '"Not This"' not in src


def test_no_not_this_anywhere_in_the_page(lrf):
    page, sid = lrf
    inspect_track(page, NAME)
    body = page.locator("body").inner_text()
    assert "Not This" not in body                                     # planted fault: any leftover title fails
    assert "Forget This" in body and "Remove Example" in body


def test_shift_k_status_says_what_the_current_match_takes(lrf):
    page, sid = lrf
    mark_hit(page, 0.9, "Shift+K")                                    # a bleed-like hit, alone: the "Forget This alone" hint, no Match hint
    idle(page)
    msg = page.locator("#stMsg").inner_text()
    assert "Forget This alone" in msg and "current Match" not in msg, msg
    mark_hit(page, 0.5, "K")                                          # a Learn This example: the Match hint takes over
    mark_hit(page, 1.0, "Shift+K")
    idle(page)
    msg = page.locator("#stMsg").inner_text()
    m = re.search(r"The current Match takes (\d+) similar hits? out", msg)
    assert m and "Forget This alone" not in msg and ".." not in msg, msg   # planted fault: no hint -> fail
    assert int(m.group(1)) >= 1


def test_the_histogram_info_line_labels_the_match(lrf):
    page, sid = lrf
    mark_hit(page, 0.9, "Shift+K")
    idle(page)
    page.wait_for_function("() => (document.querySelector('#insParams .lnHistInfo') || {textContent:''}).textContent.includes('Match ')")
    info = page.locator("#insParams .lnHistInfo").inner_text()
    assert re.search(r"Match 0\.\d\d", info), info                    # planted fault: the bare number fails
