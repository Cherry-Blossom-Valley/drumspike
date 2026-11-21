"""#559: the menu of the page in host mode (DAW plugin M1b11).

A plugin window has no menu bar, so the settings and the view options of the macOS menu were out of reach. In host mode
the page adds a menu button (static/hostmenu.js) that lists the commands of the app menu: the table of static/keymap.js
minus HOST_OWNED and the menu ids of HOST_ABSENT (static/state.js). These tests start the page the way the plugin shell
does (GET /?host_mode=1), walk the panel with the mouse and the keyboard, use the settings and the view options, and
check that a command of the table is in the panel or in one of the two lists. A planted command and a stale name in a
list are the negative cases. With the mode off the page has no button and no panel (test_standalone_*).

The second part is the lane colour (the finding of the owner in the DAW): a track record without a colour made the
spike lane paint in the colour of the last hit. The tests read the drawn pixels of the lane after a new hit."""
import json
from pathlib import Path

import pytest

from app_support import open_project, shot_dir
from test_host_page import FOUR, analyzed, host_open

SHOTS = Path.home() / ".cache" / "drumspike" / "gate" / "pr559-shots"


def shoot(page, name):
    """The review screenshots go to the gate folder outside the repository; the test-shots folder has a copy."""
    SHOTS.mkdir(parents=True, exist_ok=True)
    page.screenshot(path=str(SHOTS / name))
    page.screenshot(path=str(shot_dir(name)))


RESTORE = """async () => { setOverview(true); setInspector(true); setTrackHeight('medium'); setZoomTall(false); setCounter('seconds'); setKeyPreset('default');
  if (VL.on) vlToggle(); if (!tmShown('grid')) tmToggleUi('grid'); await new Promise(r => setTimeout(r, 200)); }"""


@pytest.fixture
def restore_settings(host_app):
    """The menu changes app settings that the bridge keeps for the whole session (overview, Inspector, track height, the
    velocity lane, Zoom Toggle): put them back so that the tests after this file see the defaults."""
    yield
    host_app.evaluate(RESTORE)


def open_menu(page):
    page.click("#hostMenuBtn")
    page.wait_for_selector("#hostMenu:not(.hidden)")


def group(page, name):
    page.click(f'#hostMenu [data-group="{name}"]')
    page.wait_for_selector("#hostMenu[data-level='1']")


def item(page, cmd):
    return page.locator(f'#hostMenu [data-cmd="{cmd}"]')


def pick(page, grp, cmd):
    open_menu(page)
    group(page, grp)
    item(page, cmd).click()
    page.wait_for_selector("#hostMenu.hidden", state="attached")


def groups_of(page):
    return page.evaluate("() => DSHostMenu.model().map(g => [g.name, g.items.map(i => i.id)])")


# ---------- the panel is built from the command table ----------

def test_every_command_of_the_table_is_in_the_menu_or_in_one_of_the_two_lists(host_app, make_project):
    """Gate 1: no second list. The audit finds no hole, and the ids on screen are the ids of the model."""
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    assert host_app.evaluate("() => DSHostMenu.audit()") == {"missing": [], "both": [], "stale": [], "unhandled": [], "noCheck": []}
    shown = []
    open_menu(host_app)
    for g, _ids in groups_of(host_app):
        group(host_app, g)
        shown += host_app.evaluate("() => [...document.querySelectorAll('#hostMenu [data-cmd]')].map(e => e.dataset.cmd)")
        host_app.keyboard.press("Escape")
    expect = [i for _g, ids in groups_of(host_app) for i in ids]
    assert shown == expect and len(shown) > 50
    table = host_app.evaluate("() => DSKeys.COMMANDS.filter(c => c.menu).map(c => c.id)")
    owned = host_app.evaluate("() => [...HOST_OWNED, ...HOST_ABSENT.flatMap(g => g.menu)]")
    dynamic = {"follow", "zoom_own"}   # the entries of the native menu that are made from the project: a track to follow, an own zoom preset
    assert sorted(set(table)) == sorted((set(shown) - dynamic) | (set(table) & set(owned))), "a command of the table is in neither"
    assert not (set(shown) & set(owned))
    names = [g for g, _ in groups_of(host_app)]
    assert names[0] == "Drum Spike" and {"File", "Edit", "View", "Transport", "Hits", "Track", "Help"} <= set(names)   # #560: File holds Export
    assert [i for g, ids in groups_of(host_app) if g == "Drum Spike" for i in ids] == ["settings"]   # About and Quit belong to the host
    # every check row of the table has a check state here, and no state without a row
    checks = host_app.evaluate("() => DSKeys.COMMANDS.filter(c => c.kind === 'check').map(c => c.id).sort()")
    assert checks == host_app.evaluate("() => Object.keys(DSHostMenu.CHECKS).sort()")


def test_a_command_in_neither_place_is_found(host_app, make_project):
    """The negative cases of Gate 1: a panel without one command, a planted command with no handler, a stale name."""
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    ids = host_app.evaluate("() => DSHostMenu.model().flatMap(g => g.items.map(i => i.id))")
    assert "zoom_in" in ids
    holes = host_app.evaluate("ids => DSHostMenu.audit(ids)", [i for i in ids if i != "zoom_in"])
    assert holes["missing"] == ["zoom_in"]
    # a new command in the table appears in the panel with no second list; with no handler the audit names it
    got = host_app.evaluate("""() => { DSKeys.COMMANDS.push({ id: "zz_new", menu: "zz_new", path: "View", needs_project: false, kind: "plain",
        title: "Planted", menu_key: null, page_keys: [], cubase: null, note: null });
      const inPanel = DSHostMenu.model().some(g => g.items.some(i => i.id === "zz_new"));
      const audit = DSHostMenu.audit(); DSKeys.COMMANDS.pop(); return { inPanel, audit }; }""")
    assert got["inPanel"] is True and got["audit"]["unhandled"] == ["zz_new"] and got["audit"]["missing"] == []
    stale = host_app.evaluate("() => { HOST_OWNED.push('no_such_command'); const a = DSHostMenu.audit(); HOST_OWNED.pop(); return a.stale; }")
    assert stale == ["no_such_command"]
    both = host_app.evaluate("ids => { HOST_OWNED.push('zoom_in'); const a = DSHostMenu.audit(ids); HOST_OWNED.pop(); return a.both; }", ids)   # a panel that still shows an owned command
    assert both == ["zoom_in"]


def test_standalone_page_has_no_menu_button_and_no_panel(app, make_project):
    """Gate 5: with the mode off the page is the page of today."""
    open_project(app, make_project(tracks=FOUR, seconds=6.0))
    assert app.evaluate("() => [!!document.getElementById('hostMenuBtn'), !!document.getElementById('hostMenu'), !!document.querySelector('.hostMenu')]") == [False, False, False]
    assert app.evaluate("() => { DSHostMenu.init(); DSHostMenu.open(); return [!!document.getElementById('hostMenuBtn'), DSHostMenu.isOpen()]; }") == [False, False]


# ---------- the settings and the view options ----------

def test_the_settings_open_from_the_menu_with_only_the_categories_of_host_mode(host_app, make_project, restore_settings):
    """Gate 2: Settings opens, the audio categories are absent, the view settings work, and the page calls no command of an
    absent group (the audio output list, the agent skill) on the way."""
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.evaluate("() => localStorage.setItem('dsg_setcat', 'audio')")   # the last open category was an absent one
    before = len(host_app.invokes)
    shoot_open = lambda: open_menu(host_app)
    shoot_open()
    shoot(host_app, "pr559-menu-open.png")
    group(host_app, "Drum Spike")
    shoot(host_app, "pr559-menu-group.png")
    item(host_app, "settings").click()
    host_app.wait_for_selector("#settingsModal:not(.hidden)")
    cats = host_app.evaluate("() => [...document.querySelectorAll('#setCats .setCatBtn')].filter(b => b.checkVisibility({ visibilityProperty: true })).map(b => b.dataset.cat)")
    assert "keyboard" in cats and "view" in cats and "plugins" in cats
    assert not {"audio", "audition", "pitches", "agent"} & set(cats), cats
    assert host_app.evaluate("() => document.querySelector('#setCats .setCatBtn.active').dataset.cat") in cats   # not the hidden one it had
    host_app.click("#setCat-view")
    host_app.check("#setZoomTall")
    assert host_app.evaluate("() => S.zoomTall") is True
    host_app.click("#setCat-keyboard")
    host_app.select_option("#setKeyPreset", "cubase")
    assert host_app.evaluate("() => DSKeys.preset") == "cubase"
    host_app.select_option("#setKeyPreset", "default")
    shoot(host_app, "pr559-menu-settings.png")
    host_app.click("#settingsDone")
    host_app.wait_for_selector("#settingsModal.hidden", state="attached")
    host_app.wait_for_timeout(300)
    absent = {c for g in host_app.evaluate("() => HOST_ABSENT") for c in g["commands"]}
    assert [c for c in host_app.invokes[before:] if c in absent] == []


def test_the_view_options_work_from_the_menu(host_app, make_project, restore_settings):
    """Gate 2: track height, overview, Inspector, zoom and the lanes, each through the panel; the check marks follow."""
    analyzed(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.set_viewport_size({"width": 1400, "height": 800})
    host_app.wait_for_function("() => !FR.raf && FR.flags === 0")
    pick(host_app, "View", "height_large")
    assert host_app.evaluate("() => S.trackHeight") == "large"
    open_menu(host_app); group(host_app, "View")
    assert item(host_app, "height_large").get_attribute("aria-checked") == "true"
    assert item(host_app, "height_small").get_attribute("aria-checked") == "false"
    host_app.keyboard.press("Escape"); host_app.keyboard.press("Escape")
    on = host_app.evaluate("() => S.overviewOn")
    pick(host_app, "View", "overview")
    assert host_app.evaluate("() => S.overviewOn") is (not on)
    on = host_app.evaluate("() => S.inspector")
    pick(host_app, "View", "inspector")
    assert host_app.evaluate("() => S.inspector") is (not on)
    span = host_app.evaluate("() => S.view.t1 - S.view.t0")
    pick(host_app, "View", "zoom_in")
    assert host_app.evaluate("() => S.view.t1 - S.view.t0") < span
    pick(host_app, "View", "zoom_5s")
    assert host_app.evaluate("() => S.view.t1 - S.view.t0") == pytest.approx(5.0)
    vl = host_app.evaluate("() => VL.on")
    pick(host_app, "View", "velocity_lane")
    assert host_app.evaluate("() => VL.on") is (not vl)
    pick(host_app, "View", "counter_samples")
    assert host_app.evaluate("() => SETTINGS().ui.counter") == "samples"
    pick(host_app, "View", "grid")
    # a command that needs a hit is greyed without one, and the Edit menu says so
    open_menu(host_app); group(host_app, "Edit")
    assert item(host_app, "delete_hit").get_attribute("aria-disabled") == "true"
    host_app.keyboard.press("Escape"); host_app.keyboard.press("Escape")
    # Hits: Analyze All is in the panel and runs the analysis of the host mode
    open_menu(host_app); group(host_app, "Hits")
    assert item(host_app, "analyze_all").count() == 1 and item(host_app, "audition").count() == 0   # Audition Hit belongs to the host
    host_app.keyboard.press("Escape"); host_app.keyboard.press("Escape")


def test_the_menu_works_with_the_keyboard_and_the_keys_do_not_reach_the_page(host_app, make_project, restore_settings):
    """Gate 3: open, move, choose, Escape. Down, Up, Return and Space in the panel must not nudge, pan, locate or play."""
    analyzed(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.evaluate("() => { S.selTrack = S.order[0]; S.sel = { track: S.order[0], index: 0 }; updateStatus(); }")
    state = host_app.evaluate("() => [S.view.t0, S.view.t1, PH.v, S.playing, S.events[S.order[0]][0].t, S.undo.length]")
    host_app.focus("#hostMenuBtn")
    host_app.keyboard.press("Enter")
    assert host_app.evaluate("() => [DSHostMenu.isOpen(), document.activeElement.dataset.group]") == [True, "Drum Spike"]
    host_app.keyboard.press("ArrowDown")   # #560: File (with Export) is the second menu
    host_app.keyboard.press("ArrowDown")
    host_app.keyboard.press("ArrowDown")
    assert host_app.evaluate("() => document.activeElement.dataset.group") == "View"
    host_app.keyboard.press("ArrowRight")   # into View
    assert host_app.evaluate("() => [document.querySelector('#hostMenu').dataset.level, !!document.activeElement.dataset.back]") == ["1", True]
    host_app.keyboard.press("End")
    last = host_app.evaluate("() => document.activeElement.dataset.cmd")
    host_app.keyboard.press("Home")
    host_app.keyboard.press("ArrowUp")
    assert host_app.evaluate("() => document.activeElement.dataset.cmd") == last   # the focus wraps
    host_app.keyboard.press("Home"); host_app.keyboard.press("Space")   # the back item
    assert host_app.evaluate("() => document.querySelector('#hostMenu').dataset.level") == "0"
    host_app.keyboard.press("ArrowRight")
    host_app.keyboard.press("ArrowLeft")   # back to the list of menus, the focus on View
    assert host_app.evaluate("() => document.activeElement.dataset.group") == "View"
    host_app.keyboard.press("Escape")   # level 0: closes, the focus goes back to the button
    assert host_app.evaluate("() => [DSHostMenu.isOpen(), document.activeElement.id]") == [False, "hostMenuBtn"]
    # choose an item with the keyboard: View > Track Height > Large
    host_app.keyboard.press("ArrowDown")   # Down on the button opens the menu
    host_app.keyboard.press("ArrowDown"); host_app.keyboard.press("ArrowDown"); host_app.keyboard.press("ArrowDown"); host_app.keyboard.press("ArrowRight")   # #560: File is a menu now
    assert host_app.evaluate("() => document.querySelector('#hostMenu').dataset.level") == "1"
    host_app.focus('#hostMenu [data-cmd="height_large"]')
    host_app.keyboard.press("Enter")
    host_app.wait_for_selector("#hostMenu.hidden", state="attached")
    assert host_app.evaluate("() => S.trackHeight") == "large"
    # the page did not see a key
    assert host_app.evaluate("() => [S.view.t0, S.view.t1, PH.v, S.playing, S.events[S.order[0]][0].t, S.undo.length]") == state
    assert host_app.evaluate("() => !!S.sel") is True


def test_the_menu_fits_a_low_window_and_scrolls(host_app, make_project, restore_settings):
    """Gate 3: below 520 px the panel stays inside the window, scrolls, and its last item can be used."""
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.set_viewport_size({"width": 1600, "height": 320})
    host_app.wait_for_function("() => document.body.classList.contains('hostLow')")
    open_menu(host_app)
    group(host_app, "View")
    geo = host_app.evaluate("""() => { const p = document.querySelector('#hostMenu'), r = p.getBoundingClientRect();
      return { top: r.top, bottom: r.bottom, innerHeight, scroll: p.scrollHeight > p.clientHeight, w: r.width, right: r.right, innerWidth }; }""")
    assert geo["bottom"] <= geo["innerHeight"] and geo["top"] >= 0 and geo["scroll"] is True and geo["right"] <= geo["innerWidth"], geo
    shoot(host_app, "pr559-menu-low-window.png")
    last = host_app.evaluate("() => [...document.querySelectorAll('#hostMenu [data-cmd]')].pop().dataset.cmd")
    host_app.locator(f'#hostMenu [data-cmd="{last}"]').scroll_into_view_if_needed()
    box = host_app.locator(f'#hostMenu [data-cmd="{last}"]').bounding_box()
    assert box["y"] + box["height"] <= 320 + 1
    # the keyboard reaches it too, and the item of the low window works: the track height
    host_app.keyboard.press("End")
    assert host_app.evaluate("() => document.activeElement.dataset.cmd") == last
    item(host_app, "height_medium").scroll_into_view_if_needed()
    item(host_app, "height_medium").click()
    assert host_app.evaluate("() => S.trackHeight") == "medium"


# ---------- the lane colour of a track without a colour (the finding of the owner in the DAW) ----------

LANE = """() => { const row = document.querySelector('.track'), out = {};
  for (const c of row.querySelectorAll('canvas.lane')) {
    const g = c.getContext('2d'), d = g.getImageData(0, 0, c.width, c.height).data, cnt = new Map(); let n = 0, opaque = 0;
    for (let i = 0; i < d.length; i += 4) { const k = d[i] + ',' + d[i + 1] + ',' + d[i + 2] + ',' + d[i + 3]; cnt.set(k, (cnt.get(k) || 0) + 1); n++; if (d[i + 3] > 200) opaque++; }
    const top = [...cnt.entries()].sort((a, b) => b[1] - a[1])[0];
    out[c.classList.contains('spike') ? 'spike' : 'wave'] = { top: top[0], share: top[1] / n, opaque: opaque / n };
  } return out; }"""


def drop_colors(page):
    """The service answers tracks_get with no colour (what the plugin's project does), then the project loads again."""
    page.evaluate("""async () => { const o = B.tracks; B.tracks = async sid => { const r = await o(sid); for (const t of Object.values(r.tracks)) delete t.color; return r; };
      await loadProject(); }""")
    page.wait_for_function("() => S.loaded > 0 && !FR.raf && FR.flags === 0")


def add_hit_with_pointer(page):
    spike = page.locator(".track").first.locator(".spike")
    box = spike.bounding_box()
    page.mouse.dblclick(box["x"] + box["width"] * 0.3, box["y"] + box["height"] / 2)
    page.wait_for_function("() => S.sel && S.events[S.sel.track].some(e => e.added)")
    page.wait_for_function("() => !FR.raf && FR.flags === 0")
    page.wait_for_timeout(300)


@pytest.mark.parametrize("fixture", ["host_app", "app"])
@pytest.mark.parametrize("role", ["other", "kick"])
def test_a_new_hit_does_not_colour_the_lane_of_a_track_without_a_colour(request, make_project, fixture, role):
    """Item 4 of the ticket. The lane code draws the spike lane in `color + "0d"`. A track without a colour made that
    "undefined0d", the 2D context kept its last fill style, and the lane was black, then magenta after a hand-added hit
    and yellow for the selected one, with the hits as dark lines. The page now gives such a track the colour of its role.
    The test reads the pixels of the lane (a share of one opaque colour) after a hit made with the pointer, in the host
    mode and in the app, for a track of the role "other" and of a drum role."""
    page = request.getfixturevalue(fixture)
    folder = make_project(tracks=(("Trk A", role, 36), ("Trk B", "snare", 38)), seconds=6.0)
    if fixture == "host_app":
        analyzed(page, folder)
    else:
        open_project(page, folder)
        page.click("#analyzeAllBtn")
        page.wait_for_function("() => S.order.every(n => (S.events[n] || []).length > 0) && !analysisRunning", timeout=60000)
    drop_colors(page)
    assert page.evaluate("() => S.tracks['Trk A'].color") == {"other": "#8a93a6", "kick": "#e05555"}[role]
    add_hit_with_pointer(page)
    lane = page.evaluate(LANE)
    assert page.evaluate("() => !!S.sel") is True
    assert lane["spike"]["opaque"] < 0.5 and lane["spike"]["share"] > 0.5, lane   # a translucent tint, not a filled lane
    assert lane["spike"]["top"].endswith(",0") or int(lane["spike"]["top"].split(",")[3]) < 100, lane
    shoot(page, f"pr559-new-hit-{fixture}-{role}.png")


def test_the_lane_check_sees_the_defect_when_the_colour_is_missing(host_app, make_project):
    """The negative case: with the repair off (no trackColors, no trackColor) the same steps give an opaque lane, so the check above can fail."""
    analyzed(host_app, make_project(tracks=(("Trk A", "other", 36), ("Trk B", "snare", 38)), seconds=6.0))
    host_app.evaluate("() => { window.trackColors = () => {}; window.trackColor = () => undefined; }")   # the repair off: at load and at the draw sites
    drop_colors(host_app)
    assert host_app.evaluate("() => S.tracks['Trk A'].color") is None
    add_hit_with_pointer(host_app)
    lane = host_app.evaluate(LANE)
    assert lane["spike"]["opaque"] > 0.5, lane   # the lane is filled with one opaque colour
    shoot(host_app, "pr559-new-hit-defect-reproduced.png")


def test_a_track_colour_that_the_service_gave_is_kept(host_app, make_project):
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    assert host_app.evaluate("() => S.order.map(n => S.tracks[n].color)") == ["#e05555", "#e0a13c", "#57b8e0", "#c9a0e0"]
    assert host_app.evaluate("() => { const t = { a: { role: 'hat' }, b: { role: 'x', color: '#123456' }, c: {} }; trackColors(t); return [t.a.color, t.b.color, t.c.color]; }") == ["#9be055", "#123456", "#8a93a6"]


# ---------- round 2 (V2, V4, V5, V6) ----------

def test_a_dialog_opened_from_the_menu_gives_the_focus_back_to_the_button(host_app, make_project, restore_settings):
    """V2: Settings from the keyboard, then Escape: the focus is on the menu button, not on the page body."""
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.focus("#hostMenuBtn")
    host_app.keyboard.press("Enter")
    host_app.keyboard.press("ArrowRight")   # into Drum Spike
    host_app.keyboard.press("ArrowDown")
    assert host_app.evaluate("() => document.activeElement.dataset.cmd") == "settings"
    host_app.keyboard.press("Enter")
    host_app.wait_for_selector("#settingsModal:not(.hidden)")
    host_app.keyboard.press("Escape")
    host_app.wait_for_selector("#settingsModal.hidden", state="attached")
    assert host_app.evaluate("() => document.activeElement.id") == "hostMenuBtn"


def test_a_click_inside_the_panel_keeps_the_focus_in_the_panel(host_app, make_project):
    """V4: a click on a heading or the padding must not move the focus to the body: Escape still closes, keys stay out of the page."""
    analyzed(host_app, make_project(tracks=FOUR, seconds=6.0))
    open_menu(host_app); group(host_app, "View")
    state = host_app.evaluate("() => [PH.v, S.playing, S.view.t0]")
    host_app.locator("#hostMenu .hmHead").first.scroll_into_view_if_needed()
    head = host_app.locator("#hostMenu .hmHead").first.bounding_box()
    host_app.mouse.click(head["x"] + 5, head["y"] + 3)
    pad = host_app.locator("#hostMenu").bounding_box()
    host_app.mouse.click(pad["x"] + pad["width"] / 2, pad["y"] + 2)   # the padding above the first item
    assert host_app.evaluate("() => document.querySelector('#hostMenu').contains(document.activeElement)") is True
    host_app.keyboard.press("Space")
    assert host_app.evaluate("() => [PH.v, S.playing, S.view.t0]") == state
    host_app.keyboard.press("Escape"); host_app.keyboard.press("Escape")
    assert host_app.evaluate("() => [DSHostMenu.isOpen(), document.activeElement.id]") == [False, "hostMenuBtn"]


def test_an_open_panel_follows_a_change_of_the_page(host_app, make_project, restore_settings):
    """V6: the marks and the greyed items of an open panel follow a change that comes from outside (a key, the host)."""
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    open_menu(host_app); group(host_app, "View")
    assert item(host_app, "height_large").get_attribute("aria-checked") == "false"
    host_app.evaluate("() => setTrackHeight('large')")
    host_app.wait_for_function("() => document.querySelector('#hostMenu [data-cmd=height_large]').getAttribute('aria-checked') === 'true'")
    assert item(host_app, "height_medium").get_attribute("aria-checked") == "false"
    host_app.keyboard.press("Escape"); host_app.keyboard.press("Escape")


def test_half_speed_is_absent_in_host_mode_and_the_other_return_commands_stay(host_app, make_project):
    """V5c: the host has no half speed; Stop and Return and its mode act through the host transport and stay."""
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    ids = host_app.evaluate("() => DSHostMenu.model().flatMap(g => g.items.map(i => i.id))")
    assert "half_speed" not in ids and {"stop_return", "stop_return_mode", "play_stop"} <= set(ids)
    host_app.evaluate("() => { setMsg(''); halfSpeedPlayStop(); }")
    assert host_app.evaluate("() => document.querySelector('#stMsg').textContent") == "the host plays the audio"
    assert host_app.evaluate("() => S.playing") is False


def test_a_help_link_that_cannot_open_says_its_address(host_app, make_project):
    """V5a: an external link of the manual that the shell refuses shows the address on the status line, not silence."""
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.evaluate("() => { B.openUrl = async () => { throw new Error('refused'); }; DSHelp.open('daw'); }")
    host_app.wait_for_selector("#helpModal:not(.hidden)")
    host_app.evaluate("""() => { const a = document.createElement('a'); a.className = 'ext'; a.href = 'https://example.org/x'; a.textContent = 'x'; a.id = 'zzLink';
      document.querySelector('#helpPage').appendChild(a); setMsg(''); }""")
    host_app.click("#zzLink")
    host_app.wait_for_function("() => document.querySelector('#stMsg').textContent.includes('https://example.org/x')")
