"""#300: the menu shows the plain keys (V, Space, K, Tab) as accelerators. The page tells the shell when a text field or
a dialog has the focus (shell_typing), and the shell takes the plain-key accelerators off the menu then. These tests
cover the page side: the report, and that the plain keys are text in a field and commands outside it, once."""
from app_support import invoke, open_project, wait_until
from test_menus import select_track, with_hits
from test_tempo_map import js  # noqa: F401

COUNT = """() => {
  window.__n = { learn: 0, play: 0 };
  const l = window.learnMark, p = window.playStop;
  window.learnMark = (...a) => { window.__n.learn++; return l(...a); };
  window.playStop = (...a) => { window.__n.play++; return p(...a); };
}"""


def typing(app):
    return app.evaluate("() => window.__DS_TYPING__ === true")


def test_plain_keys_are_text_in_a_field_and_commands_outside_it(app, make_project):
    sid = open_project(app, make_project(seconds=8.0))
    with_hits(app, sid, {"Kick In": [1.0, 3.0]})
    invoke(app, "app_settings_set", key="velocity_lane", value=False)
    app.reload()
    app.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0")
    select_track(app, "Kick In")
    app.evaluate("() => { window.__DS_NATIVE_MENU__ = true; }")   # the app: the menu runs its keys, the page the rest
    app.evaluate(COUNT)
    assert js(app, "() => VL.on") is False
    assert typing(app) is False

    # the track rename field: v, a space and k are text; nothing else happens
    name_was = js(app, "() => document.querySelector('#insName').value")
    for sel in ("#insName",):
        field = app.locator(sel)
        field.focus()
        wait_until(lambda: typing(app) is True, msg=f"{sel}: the page reports the focus in a text field")
        field.fill("")
        app.keyboard.type("v k")
        assert field.input_value() == "v k", sel
        assert js(app, "() => [VL.on, window.__n.play, window.__n.learn]") == [False, 0, 0], sel
        app.evaluate("sel => document.querySelector(sel).blur()", sel)
        wait_until(lambda: typing(app) is False, msg=f"{sel}: the page reports the focus out of the field")
    # a number field takes no letters: the keys still run nothing
    app.locator('#inspector .insSec[data-sec="timing"] .insSecHead').click()
    num = app.locator("#inspector input.pShift")
    num.focus()
    wait_until(lambda: typing(app) is True, msg="a number field")
    assert app.evaluate("() => document.activeElement.className") == "pShift"
    app.keyboard.type("v k")
    assert js(app, "() => [VL.on, window.__n.play, window.__n.learn]") == [False, 0, 0]
    app.evaluate("() => document.querySelector('#inspector input.pShift').blur()")
    app.evaluate("n => { const f = document.querySelector('#insName'); f.value = n; }", name_was)   # nothing renames
    wait_until(lambda: typing(app) is False, msg="out of the number field")

    # outside a text field V toggles the velocity lane exactly once
    app.evaluate("() => document.activeElement && document.activeElement.blur()")
    app.keyboard.press("v")
    wait_until(lambda: js(app, "() => VL.on") is True, msg="V turns the velocity lane on")
    app.wait_for_timeout(300)
    assert js(app, "() => VL.on") is True, "V ran twice"
    app.keyboard.press("v")
    wait_until(lambda: js(app, "() => VL.on") is False, msg="and off again")
    app.evaluate("() => { delete window.__DS_NATIVE_MENU__; }")


def test_a_dialog_counts_as_typing(app, make_project):
    open_project(app, make_project(seconds=8.0))
    app.evaluate("() => { const m = document.createElement('div'); m.id = 'tmpModal'; m.className = 'modal'; document.body.appendChild(m); }")
    wait_until(lambda: typing(app) is True, msg="an open dialog keeps Return and Space for itself")
    assert app.evaluate("() => DSShell.typingNow()") is True
    app.evaluate("() => document.querySelector('#tmpModal').classList.add('hidden')")
    wait_until(lambda: typing(app) is False, msg="and the keys are the menu's again")
    app.evaluate("() => document.querySelector('#tmpModal').remove()")
