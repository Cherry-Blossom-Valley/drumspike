"""P4-25: the Inspector has collapsible sections (Track, Hits, Detection, Velocity, Timing, Learn, Output) and a width
the user sets. Header: a button with aria-expanded and a muted summary of the key values while the section is closed.
Click toggles, Option-click toggles all. The open state (per section, not per track) and the width (260 to 480 px,
double-click on the handle: 320) are kept in localStorage and survive a reload. No control changes its behaviour."""
from playwright.sync_api import expect

from app_support import open_project

SECTIONS = ["track", "hits", "detection", "velocity", "timing", "learn", "output", "layer"]   # P4-7a: Layer
DEFAULT_OPEN = {"track": False, "hits": True, "detection": True, "velocity": True, "timing": False, "learn": False, "output": False,
                "layer": False}
READY = "() => typeof S !== 'undefined' && S.loaded > 0"


def select(app, name):
    app.locator(f'.track[data-name="{name}"] .thead .tnote').click()
    app.wait_for_function("n => S.selTrack === n && document.querySelector('#insName').value === n", arg=name)


def show(app, make_project):
    open_project(app, make_project())
    app.evaluate("() => setInspector(true)")
    select(app, "Kick In")


def reopen(app):
    app.reload()
    app.wait_for_function(READY)
    app.evaluate("() => setInspector(true)")
    select(app, "Kick In")


def sec(app, name):
    return app.locator(f'#inspector .insSec[data-sec="{name}"]')


def opened(app):
    return {k: sec(app, k).locator(".insSecHead").get_attribute("aria-expanded") == "true" for k in SECTIONS}


def drag_handle(app, dx):
    box = app.locator("#insHandle").bounding_box()
    x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    app.mouse.move(x, y)
    app.mouse.down()
    app.mouse.move(x + dx / 2, y, steps=4)
    app.mouse.move(x + dx, y, steps=4)
    app.mouse.up()


def width(app):
    return app.evaluate("() => document.querySelector('#inspector').getBoundingClientRect().width")


def test_the_sections_come_in_order_with_the_default_state(app, make_project):
    show(app, make_project)
    order = app.evaluate("() => [...document.querySelectorAll('#inspector .insSec')].map(s => s.dataset.sec)")
    assert order == SECTIONS
    assert opened(app) == DEFAULT_OPEN
    for k, on in DEFAULT_OPEN.items():
        body = sec(app, k).locator(".insSecBody")
        expect(body).to_be_visible() if on else expect(body).to_be_hidden()
    expect(app.locator("#insName")).to_be_visible()                 # the header of the track stays above the sections
    assert app.evaluate("() => document.querySelector('#insMixBox').closest('.insSec').dataset.sec") == "output"
    assert app.evaluate("() => { const e = document.querySelector('#inspector'); return e.scrollWidth <= e.clientWidth; }")   # no horizontal scroll


def test_a_click_toggles_one_section_and_the_summary_shows_while_it_is_closed(app, make_project):
    show(app, make_project)
    det = sec(app, "detection")
    expect(det.locator(".insSum")).to_be_hidden()                   # open: no summary
    det.locator(".insSecHead").click()
    assert opened(app)["detection"] is False and opened(app)["hits"] is True
    expect(det.locator(".insSecBody")).to_be_hidden()
    expect(det.locator(".insSum")).to_be_visible()
    expect(det.locator(".insSum")).to_contain_text("sens")
    expect(det.locator(".insSum")).to_contain_text("gap 20 ms")
    expect(sec(app, "hits").locator(".insSum")).to_be_hidden()
    sec(app, "velocity").locator(".insSecHead").click()
    expect(sec(app, "velocity").locator(".insSum")).to_contain_text("-40 to -6 dB")
    expect(sec(app, "timing").locator(".insSum")).to_have_text("no shift")
    expect(sec(app, "learn").locator(".insSum")).to_have_text("no marks")
    expect(sec(app, "output").locator(".insSum")).to_contain_text("note C1 (36)")
    expect(sec(app, "layer").locator(".insSum")).to_have_text("none")   # P4-7a
    sec(app, "hits").locator(".insSecHead").click()
    n = app.evaluate("() => (S.events['Kick In'] || []).length")
    expect(sec(app, "hits").locator(".insSum")).to_have_text(str(n))
    det.locator(".insSecHead").click()                              # open again
    assert opened(app)["detection"] is True
    expect(det.locator(".insSecBody")).to_be_visible()


def test_the_summary_follows_a_changed_value(app, make_project):
    show(app, make_project)
    sec(app, "timing").locator(".insSecHead").click()               # open: edit the shift
    app.locator("#inspector .pShift").fill("12")
    app.locator("#inspector .pShift").press("Tab")
    sec(app, "timing").locator(".insSecHead").click()               # closed again
    expect(sec(app, "timing").locator(".insSum")).to_have_text("shift +12 samples")


def test_the_header_is_a_button_and_the_keyboard_toggles(app, make_project):
    show(app, make_project)
    head = sec(app, "timing").locator(".insSecHead")
    assert head.evaluate("e => e.tagName") == "BUTTON"
    assert head.get_attribute("aria-expanded") == "false"
    head.focus()
    app.keyboard.press("Enter")
    assert head.get_attribute("aria-expanded") == "true"
    app.keyboard.press("Space")
    assert head.get_attribute("aria-expanded") == "false"


def test_option_click_toggles_all_sections(app, make_project):
    show(app, make_project)
    sec(app, "learn").locator(".insSecHead").click(modifiers=["Alt"])   # closed: all open
    assert all(opened(app).values())
    sec(app, "learn").locator(".insSecHead").click(modifiers=["Alt"])   # open: all closed
    assert not any(opened(app).values())
    for k in SECTIONS:
        expect(sec(app, k).locator(".insSum")).to_be_visible()


def test_the_open_state_is_per_section_and_survives_a_track_change_and_a_reload(app, make_project):
    show(app, make_project)
    sec(app, "learn").locator(".insSecHead").click()                # open
    sec(app, "hits").locator(".insSecHead").click()                 # closed
    want = dict(DEFAULT_OPEN, learn=True, hits=False)
    assert opened(app) == want
    select(app, "Snare Top")                                        # another track: the same state
    assert opened(app) == want
    reopen(app)
    assert opened(app) == want


def test_the_state_works_without_storage(app, make_project):
    # the Inspector keys throw (a private window, blocked site data); the page keeps its other keys
    app.add_init_script("""const g = Storage.prototype.getItem, s = Storage.prototype.setItem;
      Storage.prototype.getItem = function (k) { if (String(k).startsWith('dsg_ins')) throw new Error('no storage'); return g.call(this, k); };
      Storage.prototype.setItem = function (k, v) { if (String(k).startsWith('dsg_ins')) throw new Error('no storage'); return s.call(this, k, v); };""")
    app.reload()
    app.wait_for_function(READY)
    r = app.evaluate("() => { try { insSecToggle('timing'); insWidthSet(400, true); return [INSSEC.open.timing, document.querySelector('#inspector').style.width]; } catch (e) { return 'threw ' + e; } }")
    assert r == [True, "400px"]


def test_the_handle_resizes_between_260_and_480_and_a_double_click_resets(app, make_project):
    show(app, make_project)
    assert abs(width(app) - 320) < 1
    drag_handle(app, 60)
    assert abs(width(app) - 380) < 2
    drag_handle(app, 400)
    assert abs(width(app) - 480) < 1                                # upper clamp
    drag_handle(app, -600)
    assert abs(width(app) - 260) < 1                                # lower clamp
    assert app.evaluate("() => { const e = document.querySelector('#inspector'); return e.scrollWidth <= e.clientWidth; }")
    assert app.evaluate("() => { const i = document.querySelector('#inspector').getBoundingClientRect(), e = document.querySelector('#editor').getBoundingClientRect(); return Math.abs(e.left - i.right) < 6; }")   # the lanes take the rest
    drag_handle(app, 90)
    assert abs(width(app) - 350) < 2
    reopen(app)
    assert abs(width(app) - 350) < 2                                # kept
    app.locator("#insHandle").dblclick()
    assert abs(width(app) - 320) < 1
    reopen(app)
    assert abs(width(app) - 320) < 1


def test_the_steppers_step_by_the_field_step_with_buttons_and_keys(app, make_project):
    """P4-25 (owner): minus and plus buttons (at least 20 px) and Up/Down keys (Shift: x10) on every number field; the
    value and the param change by one step, within min and max."""
    show(app, make_project)
    sec(app, "timing").locator(".insSecHead").click()
    shift = app.locator("#inspector .pShift")
    box = app.locator("#inspector .pShift ~ .stepUp, #inspector .stepper:has(.pShift) .stepUp").first.bounding_box()
    assert box["width"] >= 20 and box["height"] >= 20
    up, dn = app.locator("#inspector .stepper:has(.pShift) .stepUp"), app.locator("#inspector .stepper:has(.pShift) .stepDn")
    up.click()
    expect(shift).to_have_value("1")
    app.wait_for_function("() => shiftNow('Kick In') === 1")
    up.click()
    dn.click(modifiers=["Shift"])
    expect(shift).to_have_value("-8")                                       # 1, 2, then 10 down
    shift.focus()
    app.keyboard.press("ArrowUp")
    expect(shift).to_have_value("-7")
    app.keyboard.press("Shift+ArrowDown")
    expect(shift).to_have_value("-17")
    app.wait_for_function("() => shiftNow('Kick In') === -17")
    gap = app.locator("#inspector .stepper:has(.pGap)")                     # the gap: min 1, step 1; the param follows
    gap.locator(".stepDn").click()
    expect(app.locator("#inspector .pGap")).to_have_value("19")
    assert app.evaluate("() => S.tracks['Kick In'].params.min_gap_ms") == 19
    cut = app.locator("#inspector .stepper:has(.pVCut)")                    # empty (off), max 0: minus starts at 0, plus stays 0
    cut.locator(".stepUp").click()
    expect(app.locator("#inspector .pVCut")).to_have_value("0")


def test_the_toolbar_has_the_icon_and_an_inspector_button_in_sync_with_the_menu(app, make_project):
    """P4-25 (owner): the brand icon (the SVG, no emoji) and the Inspector toggle at the far left of the toolbar."""
    from app_support import menu
    show(app, make_project)
    icon = app.locator("#topbar .logo img#brandIcon")
    assert icon.get_attribute("alt") == "Drum Spike"
    assert app.evaluate("() => { const i = document.querySelector('#brandIcon'); return i.complete && i.naturalWidth > 0; }")
    assert app.evaluate("() => !document.querySelector('#topbar').textContent.includes('\U0001F941')")
    btn = app.locator("#inspectorBtn")
    assert app.evaluate("() => document.querySelector('#topbar').firstElementChild.id") == "inspectorBtn"   # far left
    assert "Option+I" in btn.get_attribute("title")
    expect(btn).to_have_class(__import__("re").compile(r"\bon\b"))
    assert btn.get_attribute("aria-pressed") == "true"
    btn.click()
    expect(app.locator("#inspector")).to_be_hidden()
    assert btn.get_attribute("aria-pressed") == "false" and not app.evaluate("() => S.inspector")
    app.wait_for_function("() => window.__DS_MENU_STATE__ && window.__DS_MENU_STATE__.inspector === false")
    assert menu(app, "inspector") is True                                  # the View menu item shows it again
    expect(app.locator("#inspector")).to_be_visible()
    assert btn.get_attribute("aria-pressed") == "true"
    app.keyboard.press("Alt+i")                                            # the key hides it
    assert btn.get_attribute("aria-pressed") == "false"
