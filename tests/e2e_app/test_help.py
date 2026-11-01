"""#255: the in-app manual (Help > Drum Spike Help) and its Shortcuts section.

The help is prebuilt from docs/guide into static/help (tools/build_help.py) and served like any
static file, so these tests run the real code path: the chapter list has one entry per guide
chapter (counted from the file list, the test fails on 0), every chapter renders with all its
images loaded, no request leaves the app origin while the help is open, the Shortcuts section
lists every row of static/keymap.js for the active preset, and switching the preset changes the
shown keys. Screenshots of one chapter and the Shortcuts section go to test-shots/.
"""
import re
from pathlib import Path

from playwright.sync_api import expect

from app_support import menu, shot_dir

ROOT = Path(__file__).resolve().parents[2]

# The chapters of the help: every *.md of docs/guide except README.md — the same rule the
# builder (tools/build_help.py) uses; the gate counts from the file list and fails on 0.
GUIDE_CHAPTERS = sorted(p.stem + ".html" for p in (ROOT / "docs" / "guide").glob("*.md")
                        if p.name != "README.md")
assert GUIDE_CHAPTERS, "no guide chapters found"


def test_the_manual_lists_every_guide_chapter(app):
    menu(app, "drumspike_help")
    expect(app.locator("#helpModal")).to_be_visible()
    app.wait_for_function(
        "n => document.querySelectorAll('#helpChapters .helpChapter').length === n + 1",
        arg=len(GUIDE_CHAPTERS))   # +1: the Shortcuts section
    listed = app.evaluate("() => [...document.querySelectorAll('#helpChapters .helpChapter')]"
                          ".map(b => b.dataset.section)")
    assert listed[:-1] == GUIDE_CHAPTERS, listed
    assert listed[-1] == "shortcuts"


def test_every_chapter_renders_with_all_its_images_loaded(app):
    menu(app, "drumspike_help")
    app.wait_for_function("() => document.querySelectorAll('#helpChapters .helpChapter').length > 0")
    imgs_total = 0
    for f in GUIDE_CHAPTERS:
        app.evaluate("id => DSHelp.open(id)", f)
        app.wait_for_function(
            "id => document.querySelector('#helpPage .helpChapterPage') !== null"
            " && [...document.querySelectorAll('#helpPage img')].every(i => i.complete)",
            arg=f)
        n, broken = app.evaluate("""() => {
            const imgs = [...document.querySelectorAll('#helpPage img')];
            return [imgs.length, imgs.filter(i => i.naturalWidth <= 0).map(i => i.src)];
        }""")
        assert not broken, f"{f}: images not loaded: {broken}"
        imgs_total += n
    assert imgs_total > 0, "no chapter shows an image; the img/ copy is missing"


def test_no_network_request_leaves_the_app_while_the_help_is_open(app):
    origin = app.evaluate("() => location.origin")
    seen = []
    app.on("request", lambda r: seen.append(r.url) if not r.url.startswith(origin) else None)
    menu(app, "drumspike_help")
    app.wait_for_function("() => document.querySelectorAll('#helpChapters .helpChapter').length > 0")
    for f in GUIDE_CHAPTERS + ["shortcuts"]:
        app.evaluate("id => DSHelp.open(id)", f)
        app.wait_for_timeout(60)   # the requests flush
    app.evaluate("() => { const a = document.createElement('a'); a.className = 'ext'; a.href = 'https://example.com/drumspike';"
                 " document.querySelector('#helpPage').appendChild(a); a.click(); }")
    app.wait_for_timeout(60)
    assert seen == [], f"requests left the app while the help was open: {seen}"


def test_links_out_go_to_the_default_browser(app):
    menu(app, "drumspike_help")
    app.wait_for_function("() => document.querySelectorAll('#helpChapters .helpChapter').length > 0")
    app.evaluate("() => { const a = document.createElement('a'); a.className = 'ext';"
                 " a.href = 'https://drumspike.cherry-blossom-valley.com/'; a.textContent = 'x';"
                 " document.querySelector('#helpPage').appendChild(a); }")
    app.evaluate("() => document.querySelector('#helpPage a.ext').click()")
    app.wait_for_timeout(60)
    got = app.evaluate("() => window.__DS_OPEN_URL__ || []")
    assert got == ["https://drumspike.cherry-blossom-valley.com/"], got


def test_the_shortcuts_section_lists_every_keymap_row_and_follows_the_preset(app):
    menu(app, "shortcuts")   # Help > Keyboard Shortcuts opens the section of the manual
    expect(app.locator("#helpModal")).to_be_visible()
    expect(app.locator("#helpShortcuts #keymapTable")).to_be_visible()   # in the manual, not the old modal
    def rows():
        return app.evaluate("""() => Object.fromEntries([...document.querySelectorAll('#keymapTable tr[data-cmd]')]
            .map(r => [r.dataset.cmd, r.children[0].textContent]))""")
    got = rows()
    want = app.evaluate("() => Object.fromEntries(DSKeys.COMMANDS.map(c => [c.id, true]))")
    assert set(got) == set(want), set(want) ^ set(got)   # one row per keymap row
    before = got
    # switch the preset and re-open: the shown keys change with it
    app.evaluate("() => setKeyPreset('cubase')")
    menu(app, "shortcuts")
    after = rows()
    assert after != before, "the Shortcuts section does not follow the key preset"
    zc = app.evaluate("() => { const c = DSKeys.COMMANDS.find(c => c.id === 'zoom_in');"
                      " return DSKeys.keysOf(c, 'cubase').map(k => DSKeys.display(k)).join(', '); }")
    assert after["zoom_in"] == zc, (after["zoom_in"], zc)
    app.evaluate("() => setKeyPreset('default')")



def test_drum_spike_help_after_the_shortcuts_opens_a_chapter(app):
    # Help > Keyboard Shortcuts, Done, then Help > Drum Spike Help: the manual shows a chapter,
    # not the Shortcuts section again (the app self-test shell scenario runs this order)
    menu(app, "shortcuts")
    expect(app.locator("#helpShortcuts")).to_be_visible()
    app.locator("#helpDone").click()
    expect(app.locator("#helpModal")).to_be_hidden()
    menu(app, "drumspike_help")
    expect(app.locator("#helpModal .helpChapterPage")).to_be_visible()
    expect(app.locator("#helpShortcuts")).to_have_count(0)
    app.locator("#helpDone").click()

def test_the_screenshots_of_the_help_for_the_coordinator(app):
    menu(app, "drumspike_help")
    app.wait_for_function("() => document.querySelectorAll('#helpChapters .helpChapter').length > 0")
    app.evaluate("id => DSHelp.open(id)", "06-find-your-way.html")
    app.wait_for_function("() => [...document.querySelectorAll('#helpPage img')].every(i => i.complete)")
    app.locator("#helpModal .modal-box").screenshot(path=str(shot_dir("255-help-chapter.png")))
    menu(app, "shortcuts")
    expect(app.locator("#keymapTable")).to_be_visible()
    app.locator("#helpModal .modal-box").screenshot(path=str(shot_dir("255-help-shortcuts.png")))
