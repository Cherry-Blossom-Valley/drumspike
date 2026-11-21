"""#539: the page in host mode is complete (DAW plugin M1b3).

In a DAW the page is a window of the plugin. The host plays the audio, owns the window, the
files and the track set, and runs no agent. static/state.js HOST_ABSENT is the one list of the
command groups that are absent; these tests start the page in host mode the way the plugin
shell does (the test bridge sets window.__DS_HOST_MODE__ before the first page script, GET
/?host_mode=1), walk the list, and check what the page offers instead: an empty state that says
what to do in the DAW, analysis controls that work, a layout for a low and wide window, texts
that name no absent command, and a help section on the use inside a DAW.

On main these tests fail: the bridge has no host-mode start, state.js has no HOST_ABSENT list
and the page calls the app service from its menus, keys and controls (the red proof is in the PR).
With the mode off the page is the page of today: test_host_mode.py (pixel test of #509) and the
rest of the suite stay green."""
import re
from pathlib import Path

import pytest

from app_support import menu, shot_dir

ROOT = Path(__file__).resolve().parents[2]

# The four groups of the ticket (audio of the app, process and window, files, agent and training) and the
# track set (owner addition, Gate line 9): the names the page list must carry.
GROUPS = ["audio", "process", "files", "agent", "tracks"]


def host_open(page, folder):
    """The host opens the project (in the plugin: the shell, before the page loads). The page has no Open in host
    mode, so the test calls openProjectPath directly, and answers "Continue without saving" when the project
    that the bridge had open for an earlier test is unsaved."""
    page.evaluate("f => { window.__open = openProjectPath(f); }", str(folder))
    want = Path(folder).resolve()
    for _ in range(150):
        page.evaluate("""() => { const b = [...document.querySelectorAll('#confirmActions button')]
            .find(b => b.dataset.act === 'skip'); if (b) b.click(); }""")
        got = page.evaluate("async () => S.sid ? (await B.projectGet(S.sid).catch(() => null))?.folder ?? null : null")
        if got and Path(got).resolve() == want and page.evaluate("() => S.loaded > 0"):
            break
        page.wait_for_timeout(100)
    else:
        raise AssertionError(f"the project did not open: {got}")
    page.wait_for_function("() => !FR.raf && FR.flags === 0")


FOUR = (("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Tom 1", "tom", 48), ("Overhead L", "cymbal", 49))


@pytest.mark.parametrize("height", [320, 480])
def test_the_page_fits_a_low_and_wide_window(host_app, make_project, height):
    """Gate 11: in LUNA the editor is a strip below the timeline, 300 to 400 px high, the full width. At 1600 x 320 and
    1600 x 480 the lanes are visible, no control overlaps another, the inspector does not cover the lanes and the
    page has no vertical scroll bar. The screenshot goes to the shot folder."""
    host_app.set_viewport_size({"width": 1600, "height": height})
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.wait_for_function("() => !FR.raf && FR.flags === 0")
    geo = host_app.evaluate("""() => {
      const box = e => { const r = e.getBoundingClientRect(); return { l: r.left, t: r.top, r: r.right, b: r.bottom, w: r.width, h: r.height }; };
      const vis = e => { const r = e.getBoundingClientRect(); const cs = getComputedStyle(e);
        return r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && e.getClientRects().length > 0; };
      const controls = [...document.querySelectorAll('#topbar button, #topbar select, #topbar input, #topbar label, #topbar .time, #topbar .samplenote, #statusbar button, #statusbar span')]
        .filter(vis).map(e => ({ id: e.id || e.className || e.tagName, ...box(e) }));
      const lanes = document.querySelector('#lanesScroll'), ins = document.querySelector('#inspector');
      const rows = [...document.querySelectorAll('.track')].map(e => box(e));
      const de = document.documentElement;
      return { lanes: box(lanes), insVisible: vis(ins), ins: box(ins), rows, controls,
        vScroll: Math.max(de.scrollHeight, document.body.scrollHeight) - innerHeight, innerHeight, innerWidth,
        scrollBars: { doc: de.scrollHeight > de.clientHeight, body: document.body.scrollHeight > document.body.clientHeight } };
    }""")
    shot = shot_dir(f"host-mode-1600x{height}.png")
    host_app.screenshot(path=str(shot))
    assert geo["vScroll"] <= 0, geo["vScroll"]
    assert geo["lanes"]["h"] >= 60, f"the lanes are {geo['lanes']['h']} px high"
    # the lanes are visible: whole tracks (header and lanes) fit in the lanes area, two at 320 px and three at 480 px
    whole = [r for r in geo["rows"] if r["t"] >= geo["lanes"]["t"] - 1 and r["b"] <= geo["lanes"]["b"] + 1]
    assert len(whole) >= (2 if height < 400 else 3), (len(whole), geo["lanes"])
    assert geo["lanes"]["w"] > 600
    # the host-low rules apply: the class on <body>, the small height and the Overview and Inspector stepped back
    assert host_app.evaluate("() => [document.body.classList.contains('hostLow'), S.trackHeight, S.overviewOn, S.inspector]") == [True, "small", False, False]
    # the inspector does not cover the lanes
    if geo["insVisible"]:
        a, b = geo["ins"], geo["lanes"]
        assert a["r"] <= b["l"] + 1 or a["l"] >= b["r"] - 1 or a["b"] <= b["t"] or a["t"] >= b["b"], (a, b)
    # no two controls overlap
    cs = geo["controls"]
    bad = []
    for i, a in enumerate(cs):
        for b in cs[i + 1:]:
            if a["l"] < b["r"] - 0.5 and b["l"] < a["r"] - 0.5 and a["t"] < b["b"] - 0.5 and b["t"] < a["b"] - 0.5:
                # a label that holds its own input is one control
                bad.append((a["id"], b["id"]))
    bad = [p for p in bad if not ({"snapTgl", "snapChk"} & set(p))]
    assert bad == [], bad


# ---------------------------------------------------------------- Gate 1: the host-mode start

def test_the_flag_is_true_before_backend_js_runs(page, warm_page):
    """Gate 1: GET /?host_mode=1 puts window.__DS_HOST_MODE__ before the first page script. The probe is a route
    that puts one line in front of backend.js: it records the flag at the moment backend.js starts. The same probe
    on the plain page records undefined, so it can tell the two apart (a probe that cannot fire proves nothing)."""
    probe = "window.__flagAtBackend = window.__DS_HOST_MODE__;\n"

    def front(route):
        r = route.fetch()
        route.fulfill(response=r, body=probe + r.text())

    page.route(re.compile(r"/backend\.js(\?.*)?$"), front)
    page.goto(warm_page.url + "/?host_mode=1")
    page.wait_for_function("() => typeof B !== 'undefined' && B.app === true")
    assert page.evaluate("() => window.__flagAtBackend") is True
    assert page.evaluate("() => B.hostMode") is True
    assert page.evaluate("() => document.body.classList.contains('host')") is True   # boot ran hostApply with the mode on
    page.goto(warm_page.url + "/")
    page.wait_for_function("() => typeof B !== 'undefined' && B.app === true")
    assert page.evaluate("() => window.__flagAtBackend") is None   # undefined: the plain page never sets it
    assert page.evaluate("() => B.hostMode") is False
    assert page.evaluate("() => document.body.classList.contains('host')") is False
    assert page.evaluate("() => !document.getElementById('hostStyle')") is True


# ---------------------------------------------------------------- Gate 2 and 4: the list of absent groups

PW_MODS = {"Cmd": "Meta", "Option": "Alt", "Control": "Control", "Shift": "Shift"}
PW_KEYS = {"Return": "Enter", "Left": "ArrowLeft", "Right": "ArrowRight", "Up": "ArrowUp", "Down": "ArrowDown", "Plus": "+",
           "Minus": "-", "NumpadMinus": "NumpadSubtract", "NumpadPlus": "NumpadAdd", "NumpadPeriod": "NumpadDecimal"}


def pw_key(k):
    """A key of the table (static/keymap.js) in Playwright's notation."""
    parts = k.split("+") if not k.endswith("++") else k[:-2].split("+") + ["+"]
    key = parts.pop()
    return "+".join([PW_MODS[m] for m in parts] + [PW_KEYS.get(key, key)])


def analyzed(app, folder):
    """The project open in host mode with the hits found (the plugin analyses before the page asks; here the page does)."""
    host_open(app, folder)
    app.click("#analyzeAllBtn")
    app.wait_for_function("() => S.order.length > 0 && S.order.every(n => (S.events[n] || []).length > 0) && !analysisRunning",
                          timeout=60000)


def visible_count(page, selector):
    """(elements, visible elements) of a selector."""
    return page.evaluate("""s => { const els = [...document.querySelectorAll(s)];
      return [els.length, els.filter(e => e.checkVisibility({ visibilityProperty: true })).length]; }""", selector)


def use_control(page, selector):
    """Use every element of the selector the only way a hidden control can be used: the events of a click, a double
    click (the reset of a fader), a changed name field. A control that the page still wires would call the service."""
    return page.evaluate("""s => { let n = 0;
      for (const e of document.querySelectorAll(s)) {
        e.click();
        e.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
        e.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0 }));
        document.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
        if (e.tagName === 'INPUT' && e.type === 'text') { e.value = 'Renamed by the test'; e.dispatchEvent(new Event('blur')); }
        n++;
      } return n; }""", selector)


def test_every_absent_group_calls_no_command_through_its_controls_and_keys(host_app, make_project, capsys):
    """Gate 2 and 4. The list HOST_ABSENT (static/state.js) names the command groups that are absent in host mode. For
    each group the test uses the controls (their events, also while hidden) and the keys of its menu ids, and counts the
    calls of the group's commands on the one invoke, seen as the requests of the page to the service from the first
    byte of the page. Each count is 0. The test prints the number of groups, controls and keys and fails on 0. The last
    part is Gate 4: a flow of the group that the page still has says the one short text that names the host."""
    analyzed(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.evaluate("() => { const n = S.order[0]; S.selTrack = n; S.sel = { track: n, index: 0 }; updateStatus(); }")
    groups = host_app.evaluate("() => JSON.parse(JSON.stringify(HOST_ABSENT))")
    assert [g["group"] for g in groups] == GROUPS
    n_controls = n_keys = n_menus = n_hidden = 0
    for g in groups:
        assert g["commands"] and g["menu"] and g["say"] and g["controls"], g["group"]
        for sel in g["controls"]:
            total, shown = visible_count(host_app, sel)
            assert shown == 0, f"{g['group']}: {sel} is visible in host mode"
            n_hidden += total
            n_controls += use_control(host_app, sel)
        for id in g["menu"]:
            assert menu(host_app, id) is False, id   # absent from the menu
            n_menus += 1
            for preset in ("default", "cubase"):   # a key of the command in a preset is not the command's in the host key table
                owned = host_app.evaluate("""([id, p]) => { const c = DSKeys.COMMANDS.find(c => c.id === id);
                    return DSKeys.keysOf(c, p).filter(k => { const h = DSKeys.table(p).get(k); return h && h.cmd.id === id; }); }""", [id, preset])
                assert owned == [], f"{id} has keys in the host table of {preset}: {owned}"
            keys = host_app.evaluate("id => DSKeys.keysOf(DSKeys.COMMANDS.find(c => c.id === id), 'default')", id)
            for k in keys:
                host_app.evaluate("() => document.activeElement && document.activeElement.blur()")
                host_app.keyboard.press(pw_key(k))
                n_keys += 1
    host_app.wait_for_timeout(400)   # the delayed calls (the click settings send after 100 ms)
    for g in groups:
        calls = [c for c in host_app.invokes if c in g["commands"]]
        assert calls == [], f"{g['group']}: {calls}"
    assert host_app.evaluate("() => window.__DS_QUIT__ || 0") == 0   # the bridge counts a quit instead of calling quit_app
    assert host_app.evaluate("() => !document.getElementById('aboutModal') && !document.getElementById('fbModal')") is True
    assert host_app.evaluate("() => S.order.length") == 4 and host_app.evaluate("() => S.order[0]") == "Kick In"   # nothing was renamed or removed
    assert host_app.evaluate("() => !Object.keys(S.muted).length && !Object.keys(S.solo).length") is True
    with capsys.disabled():
        print(f"\nhost-mode walk: {len(groups)} groups, {n_controls} controls used ({n_hidden} elements hidden), "
              f"{n_keys} keys pressed, {n_menus} menu ids")
    assert len(groups) > 0 and n_controls > 0 and n_keys > 0 and n_menus > 0
    # Gate 4: a flow that the page still has says what the host owns
    flows = {"audio": "audition(false)", "process": "DSShell.quitFlow()", "files": "exportAgain()",
             "agent": "toggleTraining()", "tracks": "removeTrack(S.order[0])"}
    for g in groups:
        text = host_app.evaluate("async f => { setMsg(''); await eval(f); return document.querySelector('#stMsg').textContent; }",
                                 flows[g["group"]])
        assert text == g["say"], (g["group"], text)
    assert [c for g in groups for c in host_app.invokes if c in g["commands"]] == []


def test_the_export_dialog_works_in_host_mode_with_the_host_choices_only(host_app, make_project):
    """#560: in a DAW the Export dialog is there: MIDI and synthetic spikes, the folder row, the drag tiles. The sampler offset,
    the markers and the audio-spike types are not; the dialog says where the tempo comes from and that the audio types come
    later. A press and a drag of the MIDI tile asks no question (the default-tempo question of the app is for the Export
    button in a DAW) and starts the drag with the files of the dialog."""
    analyzed(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.evaluate("() => openExport('all')")
    host_app.wait_for_function("() => !document.querySelector('#exportModal').classList.contains('hidden')")
    for sel in ("#exMidi", "#exSpikes", "#exModeTrack", "#exModeSingle", "#exSpikeSynth", "#exChoose", "#exDragMidi", "#exDragSpikes"):
        assert host_app.evaluate("s => document.querySelector(s).checkVisibility({ visibilityProperty: true })", sel) is True, sel
    for sel in ("#exMarkers", "#exComp", "#exSpikeAudio", "#exSpikeBoth"):
        assert host_app.evaluate("s => document.querySelector(s).closest('label').checkVisibility({ visibilityProperty: true })", sel) is False, sel
    note = host_app.locator("#exHostNote")
    host_app.wait_for_function("() => !document.querySelector('#exHostNote').classList.contains('hidden')")
    text = note.inner_text()
    assert "none from the host" in text and "song position 0" in text and "later part" in text, text
    host_app.wait_for_function("() => document.querySelector('#exDragMidi').dataset.state === 'ready'", timeout=60000)
    box = host_app.locator("#exDragMidi").bounding_box()
    host_app.mouse.move(box["x"] + 5, box["y"] + 5)
    host_app.mouse.down()
    host_app.mouse.move(box["x"] + 60, box["y"] + 20, steps=4)
    host_app.mouse.up()
    host_app.wait_for_function("() => (window.__DS_DRAG__ || []).length > 0", timeout=60000)   # no confirm notice came between press and drag
    assert host_app.evaluate("() => document.querySelector('#confirmModal').classList.contains('hidden')") is True
    # the options that a DAW has no use for are never sent
    opts = host_app.evaluate("() => exOpts(true)")
    assert opts["compensate"] is False and opts["formats"] == ["midi", "spikes"]
    host_app.evaluate("() => exClose()")


def test_the_same_actions_call_the_commands_with_the_mode_off(app, make_project):
    """The control of the walk above: with the mode off, the actions of the groups call the commands. A count of 0 in host
    mode means something only if the same spy sees a count above 0 here."""
    open_project_off(app, make_project(tracks=FOUR, seconds=6.0))
    app.click("#analyzeAllBtn")
    app.wait_for_function("() => S.order.every(n => (S.events[n] || []).length > 0) && !analysisRunning", timeout=60000)
    app.evaluate("() => { const n = S.order[0]; S.selTrack = n; S.sel = { track: n, index: 0 }; updateStatus(); }")
    app.evaluate("() => setTrackMix(S.order[0], -3, null)")
    app.evaluate("() => setClick(true)")
    app.evaluate("() => document.activeElement && document.activeElement.blur()")
    app.keyboard.press("Shift+A")
    app.wait_for_timeout(500)
    app.evaluate("() => setClick(false)")
    # one real call of each other group, so that a wrong command name in the list fails here
    app.evaluate("""async () => { await B.exportExisting(S.sid, {}); await B.exportMarkersSet(S.sid, false); await B.reorder(S.sid, S.order.slice());
      await B.trainingStatus(S.sid); }""")
    seen = set(app.invokes)
    groups = app.evaluate("() => JSON.parse(JSON.stringify(HOST_ABSENT))")
    assert [g["group"] for g in groups] == GROUPS
    # "process" cannot reach the wire on the test bridge: B.quitApp, B.takeOpenPaths, B.aboutThirdParty and B.openLink answer
    # inside backend.js there (a counter, an empty list, a fixed text), and about_info is app_info on the bridge. For it the
    # check is the second one below.
    for g in groups:
        if g["group"] != "process":
            assert seen & set(g["commands"]), (g["group"], sorted(seen))
    # every command name of every group is a name that backend.js calls (a misspelt name fails here, for process too)
    backend = (ROOT / "static" / "backend.js").read_text()
    missing = [c for g in groups for c in g["commands"] if f'invoke("{c}"' not in backend]
    assert missing == [], missing


def open_project_off(page, folder):
    from app_support import open_project
    open_project(page, folder)


def test_the_standalone_page_keeps_every_control_in_the_same_window(app, make_project):
    """The comparison shot for Gate 11 and a part of Gate 5: the same window (1600 x 480) with the mode off. Every
    control that the host mode hides is there, the page has no host class and no host style, the track height stays as the
    user set it (the low-window rules are host mode only)."""
    from app_support import open_project
    app.set_viewport_size({"width": 1600, "height": 480})
    open_project(app, make_project(tracks=FOUR, seconds=6.0))
    app.wait_for_function("() => !FR.raf && FR.flags === 0")
    for sel in ("#addTracksBtn", "#exportAllBtn", "#clickBtn", "#auditionTabBtn", ".master", "#feedbackBtn", ".track .bMute", ".track .bSolo", ".track .meter"):
        assert app.locator(sel).first.is_visible(), sel
    assert app.evaluate("() => document.body.classList.contains('host') || document.body.classList.contains('hostLow')") is False
    assert app.evaluate("() => !document.getElementById('hostStyle')") is True
    assert app.evaluate("() => S.trackHeight") == "medium"   # the host mode would have chosen the small height in a window this low
    app.screenshot(path=str(shot_dir("standalone-mode-off-1600x480.png")))


# ---------------------------------------------------------------- Gate 8: the empty state

EMPTY_BAD = re.compile(r"add tracks|drop|analyze all|click|new project|open recent", re.I)


def test_the_empty_state_names_what_to_do_in_the_daw_and_says_when_the_analysis_runs(host_app, make_project):
    """Gate 8. A project without tracks: the text says what the user does in the DAW (set the plugin on a drum track) and
    offers no button. While an analysis runs (the plugin analyses without a request of the page, a page job of the
    service) the same place says that it runs. No word of the standalone text (Add tracks, drop, Analyze all) is left."""
    host_open(host_app, make_project(tracks=()))
    host_app.wait_for_function("() => S.order.length === 0 && document.querySelector('.empty-state')")
    es = host_app.locator("#lanesScroll .empty-state")
    text = es.inner_text()
    assert "DAW" in text and "plugin" in text and "drum track" in text, text
    assert EMPTY_BAD.search(text) is None, text
    assert es.locator("button, a, input").count() == 0   # no control in it
    assert host_app.locator("#esNew, #esOpen, #esRecentClear").count() == 0
    # the first analysis runs: a job of the service that the page did not start
    host_app.evaluate("""() => { window.__jobState = 'running';
      B.job = async () => ({ state: window.__jobState, tracks: {} }); return hostWatchAnalysis(); }""")
    host_app.wait_for_function("() => analysisRunning === true")
    busy = es.inner_text()
    assert "Analyzing" in busy and busy != text, busy
    assert EMPTY_BAD.search(busy) is None, busy
    assert host_app.locator("#progressWrap").is_visible()   # the bar of the page shows the job too
    host_app.evaluate("() => { window.__jobState = 'idle'; }")
    host_app.wait_for_function("() => analysisRunning === false")
    assert es.inner_text() == text   # the job ended: the text asks for a track again


def test_with_tracks_but_before_the_first_analysis_the_progress_shows_that_it_runs(host_app, make_project):
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.evaluate("""() => { window.__jobState = 'running';
      B.job = async () => ({ state: window.__jobState, tracks: { 'Kick In': { state: 'running', stage: 'analyzing' }, 'Snare Top': { state: 'queued' } } });
      return hostWatchAnalysis(); }""")
    host_app.wait_for_function("() => analysisRunning === true && document.querySelector('#progressText').textContent.startsWith('Analyzing')")
    assert host_app.locator("#analyzeAllBtn").is_disabled()   # one analysis at a time, as with the page's own run
    host_app.evaluate("() => { window.__jobState = 'idle'; }")
    host_app.wait_for_function("() => analysisRunning === false")


# ---------------------------------------------------------------- Gate 9: the track set belongs to the host

def test_the_track_set_cannot_be_changed_by_the_gestures_of_the_page(host_app, make_project):
    """Gate 9, with the real gestures: the Add tracks button is gone, a drag of a header does not reorder, the name field
    is read only, the Delete key and the right-click menu offer no remove, the drop of audio files on the page is taken
    by no handler. The plugin refuses these commands with 403; the page does not even ask."""
    analyzed(host_app, make_project(tracks=FOUR, seconds=6.0))
    order = host_app.evaluate("() => S.order.slice()")
    assert host_app.locator("#addTracksBtn").is_hidden()
    host_app.evaluate("() => { S.sel = null; LN.sel = null; }")
    box = host_app.locator(".track .thead").first.bounding_box()
    x, y = box["x"] + 40, box["y"] + 20
    host_app.mouse.move(x, y)
    host_app.mouse.down()
    host_app.mouse.move(x, y + 140, steps=8)
    host_app.mouse.up()
    assert host_app.evaluate("() => S.order") == order
    assert [n for n in host_app.evaluate("() => [...document.querySelectorAll('.track')].map(r => r.dataset.name)")] == order
    host_app.click(".track:nth-child(2) .thead")   # select the second track
    assert host_app.evaluate("() => S.selTrack") == order[1]
    assert host_app.locator("#insName").evaluate("e => e.readOnly") is True
    assert "host" in host_app.locator("#insName").get_attribute("title")
    host_app.evaluate("() => { document.activeElement && document.activeElement.blur(); }")
    host_app.keyboard.press("Delete")
    host_app.keyboard.press("Backspace")
    host_app.wait_for_timeout(200)
    assert host_app.locator("#confirmModal:not(.hidden)").count() == 0   # no "Remove track" question
    assert host_app.evaluate("() => S.order") == order
    host_app.locator(".track .thead").first.click(button="right")
    assert host_app.locator("#ctxMenu button").evaluate_all("els => els.map(e => e.dataset.act)") == ["reanalyze"]
    host_app.keyboard.press("Escape")
    # the drop of files on the window: the shell takes no Finder paths in host mode, and the flow says who owns the files
    assert host_app.evaluate("async () => JSON.stringify(await DSShell.takeOpenPaths())") == '{"projects":[],"others":[]}'
    host_app.evaluate("() => { setMsg(''); DSShell.hooks.otherPaths(['/x/a.wav']); }")
    host_app.wait_for_function("() => document.querySelector('#stMsg').textContent === 'the host owns the files'")
    bad = [c for c in host_app.invokes if c in ("tracks_add", "tracks_remove", "tracks_restore", "tracks_reorder", "track_rename", "tracks_scan")]
    assert bad == [], bad


# ---------------------------------------------------------------- Gate 10: the analysis controls work

def test_analyze_all_and_the_analysis_of_one_track_stay_and_work(host_app, make_project):
    """Gate 10. What stays: Analyze All, the re-analysis of one track (the right-click menu of its header), Analyze
    Selected Track. What goes: everything that would say files or a session folder. The test runs them: each is a page job
    (analyze_start), the hits appear, and no visible text in the page names a file or a folder."""
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    assert host_app.locator("#analyzeAllBtn").is_visible()
    assert "files" not in (host_app.locator("#analyzeAllBtn").get_attribute("title") or "")
    host_app.click("#analyzeAllBtn")
    host_app.wait_for_function("() => S.order.every(n => (S.events[n] || []).length > 0) && !analysisRunning", timeout=60000)
    n_all = host_app.invokes.count("analyze_start")
    assert n_all == 1, n_all
    host_app.locator(".track .thead").nth(1).click(button="right")
    host_app.click("#ctxMenu [data-act=reanalyze]")
    host_app.click("#confirmActions button[data-act=go]")   # the hits are there: "Re-analyze ... overwrites them. Continue?"
    host_app.wait_for_function("() => !analysisRunning", timeout=60000)
    host_app.wait_for_timeout(300)
    assert host_app.invokes.count("analyze_start") == 2
    # Analyze Selected Track through its menu id
    assert menu(host_app, "analyze_track") is True
    host_app.click("#confirmActions button[data-act=go]")
    host_app.wait_for_function("() => !analysisRunning", timeout=60000)
    host_app.wait_for_timeout(300)
    assert host_app.invokes.count("analyze_start") == 3
    # no visible text names a file or a folder
    words = host_app.evaluate(r"""() => { const bad = /\bfiles?\b|session folder|\bfolder\b|\.wav\b/i, out = [];
      for (const e of document.querySelectorAll('body *')) {
        if (!e.checkVisibility({ visibilityProperty: true })) continue;
        const own = [...e.childNodes].filter(n => n.nodeType === 3).map(n => n.textContent).join(' ');
        for (const t of [e.getAttribute('title') || '', own, e.getAttribute('aria-label') || ''])
          if (bad.test(t)) out.push(`${e.id || e.className || e.tagName}: ${t.slice(0, 90)}`);
      } return out; }""")
    assert words == [], words


# ---------------------------------------------------------------- Gate 12: the texts, the menu and the help

def command_phrases(page):
    """The names of the commands of the list (the titles of the keymap rows of HOST_ABSENT and HOST_OWNED), as the page
    and the manual would write them. A word that is also a plain word of the language (Open, Save, Close, New) is not
    a phrase of its own: Open Most Recent, Save As, Close Project and New Project are."""
    rows = page.evaluate("""() => { const ids = new Set(HOST_OWNED.concat(HOST_ABSENT.flatMap(g => g.menu)));
      return DSKeys.COMMANDS.filter(c => ids.has(c.id)).map(c => [c.id, c.title]); }""")
    plain = {"Open", "Save", "Close", "New", "Rename"}
    out = {}
    for id, title in rows:
        t = title.replace("…", "").strip()
        if t not in plain:
            out[id] = t
    return out


def phrase_hits(texts, phrases):
    hits = []
    for where, text in texts:
        for id, ph in phrases.items():
            if re.search(r"(?<![\w])" + re.escape(ph) + r"(?![\w])", text):
                hits.append((where, id, ph))
    return hits


VISIBLE_TEXTS = r"""() => { const out = [['document.title', document.title]];
  for (const e of document.querySelectorAll('body *')) {
    if (!e.checkVisibility({ visibilityProperty: true })) continue;
    const w = e.id || e.className || e.tagName;
    for (const a of ['title', 'aria-label', 'placeholder']) if (e.getAttribute(a)) out.push([`${w}[${a}]`, e.getAttribute(a)]);
    const own = [...e.childNodes].filter(n => n.nodeType === 3).map(n => n.textContent.trim()).filter(Boolean).join(' ');
    if (own) out.push([`${w} text`, own]);
  } return out; }"""


def test_the_title_texts_the_menu_and_the_help_name_no_command_of_the_list(host_app, make_project, capsys):
    """Gate 12. In host mode the visible texts of the page (the tooltips, the labels, the window title), the shortcut list
    (the one list of the menu items and keys) and every chapter of the manual that the mode shows name no command of the
    list. The help states the use inside a DAW in one short section: the host owns the files, the tracks, the tempo and
    the playback."""
    analyzed(host_app, make_project(tracks=FOUR, seconds=6.0))
    phrases = command_phrases(host_app)
    assert len(phrases) > 15
    host_app.evaluate("""() => { for (const b of document.querySelectorAll('#inspector .insSecHead[aria-expanded=false]')) b.click(); }""")   # every section of the Inspector open
    texts = host_app.evaluate(VISIBLE_TEXTS)
    host_app.evaluate("() => openSettings()")   # and every category of Settings that the mode keeps
    for cat in host_app.evaluate("() => [...document.querySelectorAll('#setCats .setCatBtn')].filter(b => b.checkVisibility()).map(b => b.dataset.cat)"):
        host_app.evaluate("c => settingsShowCat(c)", cat)
        texts += host_app.evaluate(VISIBLE_TEXTS)
    host_app.evaluate("() => settingsClose()")
    assert len(texts) > 40
    assert phrase_hits(texts, phrases) == [], [(h, [t for w, t in texts if w == h[0]]) for h in phrase_hits(texts, phrases)]
    # the shortcut list is made from the same rows: none of the list is in it
    host_app.evaluate("() => DSHelp.open('shortcuts')")
    host_app.wait_for_function("() => document.querySelectorAll('#keymapTable tr[data-cmd]').length > 10")
    rows = host_app.evaluate("() => [...document.querySelectorAll('#keymapTable tr')].map(r => [r.dataset.cmd || '', r.textContent])")
    ids = host_app.evaluate("() => HOST_OWNED.concat(HOST_ABSENT.flatMap(g => g.menu))")
    assert [r for r in rows if r[0] in ids] == []
    assert phrase_hits([(f"keys: {c}", t) for c, t in rows], phrases) == []
    # the manual: the section on the use in a DAW first, then the chapters that name no command of the list
    host_app.evaluate("() => DSHelp.open()")
    host_app.wait_for_function("() => document.querySelector('#helpPage .helpChapterPage') !== null")
    nav = host_app.evaluate("() => [...document.querySelectorAll('#helpChapters .helpChapter')].map(b => b.dataset.section)")
    assert nav[0] == "daw" and nav[-1] == "shortcuts" and len(nav) > 3, nav
    daw = host_app.locator("#helpPage").inner_text()
    assert len(daw.split()) < 200, "the section is short"
    for word in ("host", "files", "tracks", "tempo", "plays the audio"):
        assert word in daw, word
    checked = 0
    for section in nav:
        host_app.evaluate("id => DSHelp.open(id)", section)
        host_app.wait_for_function("id => document.querySelector('#helpChapters .active')?.dataset.section === id", arg=section)
        body = host_app.locator("#helpPage").inner_text()
        assert body.strip(), section
        assert phrase_hits([(section, body)], phrases) == [], section
        checked += 1
    with capsys.disabled():
        print(f"\nhost-mode texts: {len(texts)} visible texts, {len(rows)} shortcut rows, {checked} help sections, {len(phrases)} command names")
    assert checked == len(nav)
    # a link to a chapter that the mode leaves out lands on the section on the DAW
    host_app.evaluate("() => DSHelp.open('05-export.html')")
    host_app.wait_for_function("() => document.querySelector('#helpChapters .active')?.dataset.section === 'daw'")


def test_the_text_scan_sees_the_commands_when_the_mode_is_off(app, make_project):
    """The control of the scan above: the same phrases and the same reader find the commands on the standalone page (the
    tooltip of the Add tracks button names File > Add Tracks, the toolbar names Export, the manual has the chapters)."""
    from app_support import open_project
    open_project(app, make_project(tracks=FOUR, seconds=6.0))
    phrases = command_phrases(app)
    hits = phrase_hits(app.evaluate(VISIBLE_TEXTS), phrases)
    assert {h[1] for h in hits} >= {"add_tracks", "click", "mute_track"}, hits   # #560: Export is no command of the list any more
    app.evaluate("() => DSHelp.open('05-export.html')")
    app.wait_for_function("() => document.querySelector('#helpChapters .active')?.dataset.section === '05-export.html'")
    assert phrase_hits([("export", app.locator("#helpPage").inner_text())], phrases) != []
    assert app.evaluate("() => [...document.querySelectorAll('#helpChapters .helpChapter')].some(b => b.dataset.section === 'daw')") is False


# ---------------------------------------------------------------- round 2 (S1 to S4)

def test_the_empty_state_is_centred_and_readable_in_a_low_window(host_app, make_project):
    host_app.set_viewport_size({"width": 1600, "height": 320})
    host_open(host_app, make_project(tracks=()))
    host_app.wait_for_function("() => document.querySelector('#lanesScroll .hostEmpty')")
    st = host_app.evaluate("""() => { const e = document.querySelector('#lanesScroll .hostEmpty'), c = getComputedStyle(e), r = e.getBoundingClientRect(),
      l = document.querySelector('#lanesScroll').getBoundingClientRect();
      return { align: c.textAlign, size: parseFloat(c.fontSize), padTop: parseFloat(c.paddingTop), fits: r.top >= l.top - 1 && r.bottom <= l.bottom + 1 }; }""")
    assert st["align"] == "center" and st["size"] >= 14 and st["padTop"] == 12 and st["fits"], st
    host_app.screenshot(path=str(shot_dir("host-mode-empty-1600x320.png")))


def test_a_boot_error_does_not_stop_the_analysis_watch(page, warm_page):
    """S2: project_list fails at boot (initSessions throws); the watcher still starts and shows a plugin job."""
    page.route("**/invoke/project_list", lambda r: r.fulfill(status=500, content_type="application/json", body='{"code":500,"message":"no"}'))
    page.add_init_script("localStorage.removeItem('dsg_sid')")
    page.goto(warm_page.url + "/?host_mode=1")
    page.wait_for_function("() => typeof B !== 'undefined' && B.hostMode === true && document.querySelector('#stMsg')")
    page.wait_for_function("() => /init failed/.test(document.querySelector('#stMsg').textContent)")   # the boot did fail
    page.evaluate("""() => { window.__jobState = 'running'; B.job = async () => ({ state: window.__jobState, tracks: {} });
      if (!S.sid) S.sid = 'x'; }""")
    page.wait_for_function("() => analysisRunning === true", timeout=8000)   # the 2 s interval of the watcher


def test_tab_steps_silently_with_audition_on_tab_saved_in_host_mode(host_app, make_project):
    """S3: a saved ui.auditionTab true must not make a Tab step call audition (which says "the host plays the audio")."""
    analyzed(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.evaluate("() => { SETTINGS().ui.auditionTab = true; setMsg(''); }")
    n0 = len(host_app.invokes)
    host_app.evaluate("() => document.activeElement && document.activeElement.blur()")
    host_app.keyboard.press("Tab")
    host_app.wait_for_function("() => !!S.sel")
    assert host_app.locator("#stMsg").inner_text() != "the host plays the audio"
    assert not [c for c in host_app.invokes[n0:] if c.startswith("transport_")]
    host_app.evaluate("() => { SETTINGS().ui.auditionTab = false; }")


def test_the_window_growing_keeps_what_the_user_did_while_it_was_low(host_app, make_project):
    """S4: the user opens the Inspector in the low window and closes it again; growing the window must not open it."""
    host_app.set_viewport_size({"width": 1600, "height": 1000})
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.evaluate("() => { setInspector(true, false); setOverview(true, false); setTrackHeight('medium', false); }")
    host_app.set_viewport_size({"width": 1600, "height": 320})
    host_app.wait_for_function("() => document.body.classList.contains('hostLow') && !S.inspector")
    host_app.evaluate("() => { setInspector(true, false); setInspector(false, false); }")   # the user opens it and closes it again
    host_app.set_viewport_size({"width": 1600, "height": 1000})
    host_app.wait_for_function("() => !document.body.classList.contains('hostLow')")
    assert host_app.evaluate("() => [S.inspector, S.overviewOn, S.trackHeight]") == [False, True, "medium"]   # inspector stays closed; the others come back
