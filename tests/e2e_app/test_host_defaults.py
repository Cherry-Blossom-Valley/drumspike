"""#577 (DAW plugin M1d3): the user defaults and the presets of the plugin, the page side.

The plugin owns the files (plugins/ara/src/userdefaults.rs); the page asks it through B.defaults* and B.preset* (the test bridge has none, so these
tests stub them). What the page does: the Inspector has "Save as default" and "Reset to default" for the ROLE of the selected row, the menu (Track)
has "Load Preset..." and "Save Preset...", a load shows what will change and applies after a confirm as ONE undo step, a user preset has a delete
control with a confirm and the built-in one has none, a failed save says why in the status line. With the mode off the page has none of it
(test_standalone_*), and a planted host mode shows that the standalone check can fail (test_the_standalone_check_can_fail)."""
import json

import pytest

from test_host_page import FOUR, analyzed

PRESETS = [
    {"name": "General MIDI", "builtin": True, "roles": {"kick": {"notes": [36], "params": {}}, "snare": {"notes": [38], "params": {}},
                                                           "tom": {"notes": [50, 48, 47], "params": {}}}},
    {"name": "Mine", "builtin": False, "roles": {"kick": {"notes": [35], "params": {"min_gap_ms": 41}}, "snare": {"notes": [40], "params": {}}}},
]

STUBS = """(presets) => {
  window.__calls = [];
  B.defaultsGet = async () => ({ status: 'ok', text: '', roles: {}, builtin: 'General MIDI', presets });
  B.defaultsSaveRole = async (role, notes, params) => { window.__calls.push(['save', role, notes, params]); return {}; };
  B.defaultsResetRole = async role => { window.__calls.push(['reset', role]); return {}; };
  B.presetSave = async (name, roles, overwrite) => { window.__calls.push(['preset', name, roles, !!overwrite]); return { exists: false }; };
  B.presetDelete = async name => { window.__calls.push(['delete', name]); return {}; };
}"""


@pytest.fixture
def host(host_app, make_project):
    analyzed(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.evaluate(STUBS, PRESETS)
    host_app.evaluate("async () => { await DSHostDefaults.load(); DSHostDefaults.inspector(); }")
    yield host_app
    host_app.evaluate("() => { B.appSettingsSet('host_intro_done', true); DSHostDefaults.setFault(''); }")


def notes(page):
    return page.evaluate("() => hdRows().map(h => S.tracks[h].note)")


def open_track_menu(page):
    page.click("#hostMenuBtn")
    page.wait_for_selector("#hostMenu:not(.hidden)")
    page.click('#hostMenu [data-group="Track"]')
    page.wait_for_selector("#hostMenu[data-level='1']")


def test_the_menu_has_the_two_entries_and_the_audit_still_passes(host):
    open_track_menu(host)
    texts = host.evaluate("() => [...document.querySelectorAll('#hostMenu [data-host]')].map(e => [e.dataset.host, e.textContent, e.getAttribute('role')])")
    assert ["host_load_preset", "Load Preset…", "menuitem"] in [[a, b.strip("✓ ").strip(), c] for a, b, c in texts], texts
    assert any(t[0] == "host_save_preset" for t in texts)
    host.keyboard.press("Escape")
    host.keyboard.press("Escape")
    assert host.evaluate("() => DSHostMenu.audit()") == {"missing": [], "both": [], "stale": [], "unhandled": [], "noCheck": []}


def test_load_preset_cancel_changes_nothing_and_load_is_one_undo_step(host):
    before = notes(host)
    undo0 = host.evaluate("() => S.undo.length")
    open_track_menu(host)
    host.click('#hostMenu [data-host="host_load_preset"]')
    host.wait_for_selector("#hdModal:not(.hidden)")
    assert host.evaluate("() => [...document.querySelectorAll('#hdBody [data-preset]')].map(r => r.dataset.preset)") == ["General MIDI", "Mine"]
    host.click('#hdBody [data-preset="Mine"] [data-act="load"]')
    host.wait_for_selector("#confirmModal:not(.hidden)")
    msg = host.inner_text("#confirmMsg")
    assert msg.startswith('Load preset "Mine"? 2 tracks change: the MIDI note of 2 tracks, the detection settings of 1 track.'), msg
    assert "You can undo this in one step." in msg
    host.click('#confirmActions [data-act="cancel"]')
    assert notes(host) == before and host.evaluate("() => S.undo.length") == undo0
    # again, and load
    host.evaluate("() => DSHostDefaults.openLoad()")
    host.wait_for_selector("#hdModal:not(.hidden)")
    host.click('#hdBody [data-preset="Mine"] [data-act="load"]')
    host.wait_for_selector("#confirmModal:not(.hidden)")
    host.click('#confirmActions [data-act="load"]')
    host.wait_for_function("(n) => S.undo.length === n", arg=undo0 + 1)
    after = notes(host)
    assert after != before and 35 in after and 40 in after
    kick = host.evaluate("() => S.tracks[hdRows().find(h => S.tracks[h].role === 'kick')].params.min_gap_ms")
    assert kick == 41
    assert host.evaluate("() => S.undo[S.undo.length - 1].type") == "group"
    host.evaluate("() => undo()")
    host.wait_for_function("(n) => S.undo.length === n", arg=undo0)
    assert notes(host) == before, "one undo takes back every note"
    host.evaluate("() => redo()")
    host.wait_for_function("(n) => S.undo.length === n", arg=undo0 + 1)
    assert notes(host) == after


def test_the_fault_runs_of_load_preset_fail_the_test(host):
    undo0 = host.evaluate("() => S.undo.length")
    host.evaluate("() => DSHostDefaults.setFault('two-steps')")
    host.evaluate("() => DSHostDefaults.openLoad()")
    host.wait_for_selector("#hdModal:not(.hidden)")
    host.click('#hdBody [data-preset="Mine"] [data-act="load"]')
    host.wait_for_selector("#confirmModal:not(.hidden)")
    host.click('#confirmActions [data-act="load"]')
    host.wait_for_function("(n) => S.undo.length > n", arg=undo0)
    assert host.evaluate("() => S.undo.length") - undo0 > 1, "the fault makes one entry per track"
    host.evaluate("() => DSHostDefaults.setFault('no-confirm')")
    host.evaluate("() => DSHostDefaults.openLoad()")
    host.wait_for_selector("#hdModal:not(.hidden)")
    host.click('#hdBody [data-preset="General MIDI"] [data-act="load"]')
    assert host.evaluate("() => document.querySelector('#confirmModal').classList.contains('hidden')"), "no confirm in the fault run"


def test_a_user_preset_has_a_delete_with_a_confirm_and_the_built_in_one_has_none(host):
    host.evaluate("() => DSHostDefaults.openLoad()")
    host.wait_for_selector("#hdModal:not(.hidden)")
    assert host.locator('#hdBody [data-preset="General MIDI"] [data-act="delete"]').count() == 0
    host.click('#hdBody [data-preset="Mine"] [data-act="delete"]')
    host.wait_for_selector("#confirmModal:not(.hidden)")
    assert 'Delete preset "Mine"? The file is removed.' in host.inner_text("#confirmMsg")
    host.click('#confirmActions [data-act="cancel"]')
    assert host.evaluate("() => window.__calls") == []
    host.evaluate("() => DSHostDefaults.openLoad()")
    host.wait_for_selector("#hdModal:not(.hidden)")
    host.click('#hdBody [data-preset="Mine"] [data-act="delete"]')
    host.wait_for_selector("#confirmModal:not(.hidden)")
    host.click('#confirmActions [data-act="delete"]')
    host.wait_for_function("() => window.__calls.length === 1")
    assert host.evaluate("() => window.__calls[0]") == ["delete", "Mine"]


def test_save_preset_takes_a_typed_name_and_the_first_track_of_each_role(host):
    open_track_menu(host)
    host.click('#hostMenu [data-host="host_save_preset"]')
    host.wait_for_selector("#hdModal:not(.hidden)")
    host.fill("#hdName", "My Kit")
    host.click("#hdOk")
    host.wait_for_function("() => window.__calls.length === 1")
    call = host.evaluate("() => window.__calls[0]")
    assert call[0] == "preset" and call[1] == "My Kit" and call[3] is False
    assert call[2]["kick"]["notes"] == [36] and call[2]["snare"]["notes"] == [38]
    assert "params" in call[2]["tom"]
    # a refusal of the plugin stays in the dialog
    host.evaluate("() => { B.presetSave = async () => { throw new Error('409: A preset named General MIDI is built in. Choose another name.'); }; }")
    open_track_menu(host)
    host.click('#hostMenu [data-host="host_save_preset"]')
    host.wait_for_selector("#hdModal:not(.hidden)")
    host.fill("#hdName", "General MIDI")
    host.keyboard.press("Enter")
    host.wait_for_function("() => document.querySelector('#hdErr').textContent.length > 0")
    assert "built in" in host.inner_text("#hdErr")
    assert not host.evaluate("() => document.querySelector('#hdModal').classList.contains('hidden')")


@pytest.mark.parametrize("track,role", [(0, "kick"), (1, "snare"), (2, "tom"), (3, "cymbal")])
def test_the_inspector_buttons_name_the_role_and_act_on_it(host, track, role):
    host.evaluate("([i]) => { setSelTrack(S.order[i]); syncInspector(true); }", [track])
    save, reset = host.inner_text("#hdSaveBtn"), host.inner_text("#hdResetBtn")
    assert save.startswith(f"Save as default for new {role} tracks: "), save
    assert reset == f"Reset to default for new {role} tracks"
    host.click("#hdSaveBtn")
    host.wait_for_function("() => window.__calls.length === 1")
    c = host.evaluate("() => window.__calls[0]")
    assert c[0] == "save" and c[1] == role and len(c[2]) >= 1
    assert "shift_samples" not in c[3], "the shift is no default"
    assert f"new {role} tracks" in host.inner_text("#stMsg")
    host.click("#hdResetBtn")
    host.wait_for_function("() => window.__calls.length === 2")
    assert host.evaluate("() => window.__calls[1]") == ["reset", role]
    assert f"New {role} tracks start with the built-in values." in host.inner_text("#stMsg")


def test_saving_over_an_existing_preset_asks_first_and_cancel_writes_nothing(host):
    host.evaluate("() => { B.presetSave = async (name, roles, overwrite) => { window.__calls.push(['preset', name, !!overwrite]); return { exists: !overwrite, name: 'My Kit' }; }; }")
    open_track_menu(host)
    host.click('#hostMenu [data-host="host_save_preset"]')
    host.wait_for_selector("#hdModal:not(.hidden)")
    host.fill("#hdName", "my kit")
    host.click("#hdOk")
    host.wait_for_selector("#confirmModal:not(.hidden)")
    assert 'Replace the preset "My Kit"?' in host.inner_text("#confirmMsg")
    host.click('#confirmActions [data-act="cancel"]')
    assert host.evaluate("() => window.__calls") == [["preset", "my kit", False]], "cancel: the plugin was asked once and nothing was replaced"
    assert not host.evaluate("() => document.querySelector('#hdModal').classList.contains('hidden')"), "the dialog stays open"
    host.click("#hdOk")
    host.wait_for_selector("#confirmModal:not(.hidden)")
    host.click('#confirmActions [data-act="replace"]')
    host.wait_for_function("() => window.__calls.length === 3")
    assert host.evaluate("() => window.__calls[2]") == ["preset", "my kit", True]


def test_the_button_is_disabled_when_the_role_has_no_note(host):
    host.evaluate("() => { const t = S.tracks[S.order[1]]; t.note = null; setSelTrack(S.order[1]); syncInspector(true); }")
    assert host.locator("#hdSaveBtn").is_disabled()
    assert "no MIDI note" in host.locator("#hdSaveBtn").get_attribute("title")


def test_a_role_with_no_role_uses_the_words_tracks_with_no_role(host):
    assert host.evaluate("() => DSHostDefaults.word('other')") == "tracks with no role"


def test_a_failed_save_says_why_in_the_status_line_and_breaks_nothing(host):
    host.evaluate("() => { B.defaultsSaveRole = async () => { throw new Error('409: the defaults could not be saved: Permission denied'); }; setSelTrack(S.order[1]); syncInspector(true); }")
    host.click("#hdSaveBtn")
    host.wait_for_function("() => document.querySelector('#stMsg').textContent.startsWith('error: ')")
    assert host.inner_text("#stMsg") == "error: the defaults could not be saved: Permission denied"
    assert not host.locator("#hdSaveBtn").is_disabled()


# ---------- the standalone page ----------

def test_standalone_has_no_element_no_menu_entry_and_asks_the_plugin_nothing(app, make_project):
    analyzed(app, make_project(tracks=FOUR, seconds=6.0))
    app.evaluate("() => { setSelTrack(S.order[1]); syncInspector(true); DSHostDefaults.inspector(); DSHostDefaults.openLoad(); DSHostDefaults.openSave(); }")
    assert app.evaluate("() => document.querySelectorAll('[id^=hd]').length") == 0
    assert app.evaluate("() => document.getElementById('hostMenuBtn')") is None
    assert not [i for i in app.invokes if "plugin_defaults" in i or "plugin_preset" in i], app.invokes
    # the standalone undo list: an Inspector note change has no undo step, and nothing of this ticket adds one
    n0 = app.evaluate("() => S.undo.length")
    app.evaluate("() => { const t = S.tracks[S.selTrack]; document.querySelector('#insParams .pNoteNum').value = 60; document.querySelector('#insParams .pNoteNum').dispatchEvent(new Event('change')); }")
    app.wait_for_timeout(300)
    assert app.evaluate("() => S.undo.length") == n0
    assert app.evaluate("() => S.undo.filter(o => o.type === 'note').length") == 0


def test_the_standalone_check_can_fail(app, make_project):
    """The fault run of the line above: with a planted host mode the same elements DO appear, so the check is able to see them."""
    analyzed(app, make_project(tracks=FOUR, seconds=6.0))
    app.evaluate("() => { window.hostMode = () => true; setSelTrack(S.order[1]); syncInspector(true); }")
    assert app.evaluate("() => document.querySelectorAll('[id^=hd]').length") >= 1
