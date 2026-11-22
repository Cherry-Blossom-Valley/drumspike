"""#575 (DAW plugin M1d1): the page in a DAW shows no standalone leftovers.

Line 1: every text that names a track shows the name of the DAW track (`display_name`), through the one helper `trackLabel`; the key shows nowhere.
Line 2: the tempo rulers, Set Bar 1 and the bar-1 drag are read-only in a DAW; an edit can no longer look lost.
Line 3: the "unsaved changes" dot does not show in a DAW.
Line 4: the export dialog in a DAW has no development note, says where to drop the files and has the destination row collapsed.
Line 5: the help in a DAW is a path of five steps; the first open shows one line that goes away after the first action.
Line 6: after a reload that the DAW caused (changed audio) one message says what happened and that the hand edits are kept.
Each line has a test with the host mode on (the page as the plugin shell serves it, GET /?host_mode=1) and a test with the mode off that proves the
standalone page shows the element as before. The plugin side has its own stage (plugins/ara/check.py, editor-leftovers)."""


import pytest

from app_support import open_project, wait_until
from test_host_page import FOUR, analyzed

DAW = {"Kick In": "Drum A", "Snare Top": "Drum B", "Tom 1": "Drum C", "Overhead L": "Drum D"}


def name_the_tracks(page):
    """Every answer of tracks_get carries the display name that the plugin gives (none of them holds its key), then the page loads again."""
    def add(route):
        r = route.fetch()
        body = r.json()
        for key, track in body["tracks"].items():
            track["display_name"] = DAW[key]
        route.fulfill(response=r, json=body)
    page.route("**/invoke/tracks_get", add)
    page.evaluate("async () => { await loadProject(); }")
    page.wait_for_function("() => !FR.raf && FR.flags === 0")


@pytest.fixture
def host(host_app, make_project):
    analyzed(host_app, make_project(tracks=FOUR, seconds=6.0))
    name_the_tracks(host_app)
    yield host_app
    host_app.evaluate("() => { B.appSettingsSet('host_intro_done', true); }")


@pytest.fixture
def plain(app, make_project):
    open_project(app, make_project(tracks=FOUR, seconds=6.0))
    app.click("#analyzeAllBtn")
    app.wait_for_function("() => S.order.length > 0 && S.order.every(n => (S.events[n] || []).length > 0) && !analysisRunning", timeout=60000)
    yield app


PROBE = """async () => {
  const out = {}, text = id => document.querySelector(id).textContent;
  // the progress text of a running analysis
  const real = B.job;
  B.job = async () => ({ state: "running", tracks: { "Kick In": { state: "done" }, "Snare Top": { state: "running", stage: "detecting" }, "Tom 1": { state: "queued" } } });
  pollJob(); await new Promise(r => setTimeout(r, 300)); clearTimeout(pollTimer); B.job = real;
  out.progress = text("#progressText"); hideProgress();
  // the info line and the status bar of a selected hit
  S.selTrack = "Snare Top"; S.sel = { track: "Snare Top", index: 0 }; updateStatus(); drawInfo();
  out.info = text("#ilTrack"); out.sel = text("#stSel");
  S.sel = null; updateStatus();
  // the label of an undo step, in the title of Edit > Undo
  pushUndo({ type: "del", track: "Snare Top", t: 1.5 }); out.undo = undoTitles().undoTitle; S.undo.pop();
  pushUndo({ type: "setv-many", track: "Tom 1", list: [{ id: 1 }, { id: 2 }] }); out.undoMany = undoTitles().undoTitle; S.undo.pop();
  // a status text of the page about a follower, and one with the key in a text of the service
  S.tracks["Tom 1"].follows = "Kick In"; followerLocked("Tom 1"); out.follower = text("#stMsg"); delete S.tracks["Tom 1"].follows;
  setMsg("Snare Top: 12 hits, Kick In: 9 hits"); out.service = text("#stMsg");
  out.header = document.querySelector('.track[data-name="Snare Top"] .tnm').textContent;
  out.headerTitle = document.querySelector('.track[data-name="Snare Top"] .tnm').title;
  // the Re-analyze dialog text
  S.selTrack = "Snare Top";
  return out;
}"""


def test_every_text_that_names_a_track_shows_the_name_of_the_daw_track(host):
    r = host.evaluate(PROBE)
    assert "Drum B" in r["progress"] and "Snare Top" not in r["progress"], r["progress"]
    assert r["info"] == "Drum B" and r["sel"].startswith("Drum B:"), r
    assert r["undo"] == "Undo Delete Hit (Drum B, 1.500 s)", r["undo"]
    assert r["undoMany"] == "Undo Set Velocity (Drum C, 2 hits) (Drum C)", r["undoMany"]
    assert r["follower"].startswith("Drum C follows Drum A:") and "Kick In" not in r["follower"] and "Tom 1" not in r["follower"], r["follower"]
    assert r["service"] == "Drum B: 12 hits, Drum A: 9 hits", r["service"]
    assert r["header"] == r["headerTitle"] == "Drum B"
    for text in (r["progress"], r["info"], r["sel"], r["undo"], r["undoMany"], r["follower"], r["service"]):
        assert not any(k in text for k in DAW), text


def test_the_export_dialog_the_follow_menus_and_the_inspector_show_the_name_of_the_daw_track(host):
    host.evaluate("() => { S.selTrack = 'Snare Top'; openExport('selected', 'Snare Top'); }")
    host.wait_for_function("() => !document.querySelector('#exportModal').classList.contains('hidden')")
    assert host.locator("#exSelText").inner_text() == "Selected track (1): Drum B"
    host.evaluate("() => { EX.skipped = null; }")
    host.evaluate("() => exCancel()")
    # Track > Follow Hits Of, in the page menu: one entry per other track, by its DAW name
    titles = host.evaluate("() => DSHostMenu.model().flatMap(g => g.items).filter(i => i.id === 'follow').map(i => i.title)")
    assert sorted(titles) == ["Drum A", "Drum C", "Drum D"], titles
    # the Inspector: the follow selector names the other tracks by their DAW names, the values stay the keys
    host.evaluate("() => { if (!S.inspector) setInspector(true); setSelTrack('Snare Top'); }")
    host.wait_for_function("() => document.querySelector('#insParams .pFollow')")
    opts = host.evaluate("() => [...document.querySelectorAll('#insParams .pFollow option')].map(o => [o.value, o.textContent])")
    assert opts == [["", "own detection"], ["Kick In", "follow Drum A"], ["Tom 1", "follow Drum C"], ["Overhead L", "follow Drum D"]], opts
    # the tooltip of a failed track and the status text of a rejected analysis
    host.evaluate("""() => { showDetector({ "Snare Top": { state: "error", error: "no audio" } }, null); }""")
    assert "Drum B" in host.evaluate("() => document.querySelector('#stDet').title")
    assert "Snare Top" not in host.evaluate("() => document.querySelector('#stDet').title")


def test_trackText_changes_a_key_once_and_keeps_a_label_that_holds_another_key(host):
    host.evaluate("() => { S.tracks['Kick In'].display_name = 'Kick In (2)'; S.tracks['Tom 1'].display_name = 'Kick'; S.tracks['Overhead L'].display_name = 'Overhead L'; }")
    got = host.evaluate("() => [trackText('Kick In and Tom 1'), trackText('Kick In (2)'), trackText('no key here'), trackText('')]")
    assert got == ["Kick In (2) and Kick", "Kick In (2)", "no key here", ""], got


def test_standalone_texts_show_the_key_as_before(plain):
    r = plain.evaluate(PROBE)
    assert "Snare Top" in r["progress"] and "Drum B" not in r["progress"]
    assert r["info"] == "Snare Top" and r["sel"].startswith("Snare Top:")
    assert r["undo"] == "Undo Delete Hit (Snare Top, 1.500 s)" and r["undoMany"] == "Undo Set Velocity (Tom 1, 2 hits) (Tom 1)"
    assert r["follower"].startswith("Tom 1 follows Kick In:")
    assert r["service"] == "Snare Top: 12 hits, Kick In: 9 hits"
    assert r["header"] == "Snare Top"
    assert plain.evaluate("() => [trackLabel('Snare Top'), trackText('Kick In: 3 hits'), hostMode()]") == ["Snare Top", "Kick In: 3 hits", False]
    assert plain.evaluate("() => DSShell.menuState().tracks") == ["Kick In", "Snare Top", "Tom 1", "Overhead L"], "the native menu is made from the keys"


# ---------------------------------------------------------------- line 2: the tempo

MAP = {"bpm": 101, "steps": [{"t": 2.0, "bpm": 120}], "meter": [4, 4], "changes": [], "bar1_s": 0.0}


def give_tempo(page, tempo_map):
    page.evaluate("""m => { B.hostTransport = B.hostTransport || {}; B.hostTransport.tempoMap = async () => m;
      TM.ui.tempo = true; TM.ui.meter = true; TM.ui.beats = true; tmApplyUi(true); }""", tempo_map)
    page.evaluate("async () => { await tmLoad(); }")
    page.wait_for_function("() => !FR.raf && FR.flags === 0")


def tempo_calls(page):
    return [c for c in page.invokes if c in ("tempo_map_set", "tempo_map_refit_bar1")]


def test_the_tempo_is_read_only_when_the_daw_gives_it(host):
    give_tempo(host, MAP)
    before = host.evaluate("() => JSON.stringify(TM.map)")
    n = len(tempo_calls(host))
    assert host.evaluate("() => TM.map !== null && TM.isDefault === false")
    # Set Bar 1 (the page menu and the page functions): stopped with the text, nothing is written
    assert host.evaluate("async () => await tmSetBar1(1.25)") is False
    assert "The DAW gives the tempo" in host.locator("#stMsg").inner_text()
    host.evaluate("() => setMsg('')")
    assert host.evaluate("async () => await setBar1AtAudioStart()") is False and "The DAW gives the tempo" in host.locator("#stMsg").inner_text()
    items = host.evaluate("() => DSHostMenu.model().flatMap(g => g.items).filter(i => /^set_bar1/.test(i.id)).map(i => [i.id, i.enabled])")
    assert items == [["set_bar1", False], ["set_bar1_audio", False]], items
    # a typed tempo, a typed meter and a typed bar 1: no field opens
    for kind, it in (("tempo", { "id": "start", "i": -1, "t": 0, "text": "101" }), ("meter", { "id": "start", "i": -1, "t": 0, "text": "4/4" }),
                     ("beats", { "id": "bar1", "i": -1, "t": 0, "text": "0.000" })):
        host.evaluate("([k, it]) => tmEditStart(k, it)", [kind, it])
        assert host.evaluate("() => TM.edit") is None and host.evaluate("() => document.getElementById('mapEdit').classList.contains('hidden')")
    # the real pointer: a double click on the tempo ruler, a double click and a drag on the bar-1 marker
    c = host.locator('#mapRulers .mrow[data-ruler="tempo"] canvas')
    box = c.bounding_box()
    host.mouse.dblclick(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    assert host.evaluate("() => TM.edit") is None
    b = host.locator('#mapRulers .mrow[data-ruler="beats"] canvas').bounding_box()
    host.mouse.dblclick(b["x"] + 2, b["y"] + b["height"] / 2)
    assert host.evaluate("() => TM.edit") is None
    host.mouse.move(b["x"] + 2, b["y"] + b["height"] / 2)
    host.mouse.down()
    host.mouse.move(b["x"] + 80, b["y"] + b["height"] / 2, steps=4)
    host.mouse.up()
    host.wait_for_timeout(200)
    assert host.evaluate("() => JSON.stringify(TM.map)") == before, "the map of the host is untouched"
    assert tempo_calls(host)[n:] == [], "no tempo command reached the service"
    assert "The DAW gives the tempo" in host.locator("#stMsg").inner_text()
    assert host.evaluate("() => S.undo.filter(o => o.type === 'tempo').length") == 0, "no undo step"


def test_a_daw_without_a_tempo_says_so_and_edits_nothing(host):
    host.evaluate("() => { B.hostTransport = B.hostTransport || {}; B.hostTransport.tempoMap = async () => null; }")
    host.evaluate("async () => { await tmLoad(); }")
    assert host.evaluate("() => TM.map") is None
    n = len(tempo_calls(host))
    assert host.evaluate("async () => await tmSetBar1(1.0)") is False
    text = host.locator("#stMsg").inner_text()
    assert "The DAW gives no tempo" in text and "120 BPM" not in text, text
    assert tempo_calls(host)[n:] == []


def test_standalone_tempo_edits_work_as_before(plain):
    plain.wait_for_function("() => TM.map !== null")
    assert plain.evaluate("async () => await tmSetBar1(0.25)") is True
    assert plain.evaluate("() => TM.map.bar1_s") == pytest.approx(0.25)
    assert "tempo_map_set" in plain.invokes
    items = plain.evaluate("() => DSShell.menuState()")
    assert items["undo"] is True
    plain.evaluate("() => { TM.ui.tempo = true; tmApplyUi(true); }")
    plain.wait_for_function("() => !FR.raf && FR.flags === 0")
    c = plain.locator('#mapRulers .mrow[data-ruler="tempo"] canvas').bounding_box()
    plain.mouse.dblclick(c["x"] + c["width"] / 2, c["y"] + c["height"] / 2)
    plain.wait_for_function("() => TM.edit !== null")
    plain.keyboard.press("Escape")


# ---------------------------------------------------------------- line 3: the dot

def test_a_daw_session_shows_no_unsaved_dot(host):
    host.evaluate("() => setDirtyState(true)")
    assert host.evaluate("() => S.dirty") is True, "the flag itself stays"
    assert host.evaluate("() => document.getElementById('dirtyDot').classList.contains('hidden')") is True
    assert "●" not in host.title()
    host.evaluate("() => setDirtyState(false)")


def test_standalone_shows_the_unsaved_dot_as_before(plain):
    plain.evaluate("() => setDirtyState(true)")
    assert plain.evaluate("() => document.getElementById('dirtyDot').classList.contains('hidden')") is False
    assert plain.title().startswith("●")
    plain.evaluate("() => setDirtyState(false)")
    assert plain.evaluate("() => document.getElementById('dirtyDot').classList.contains('hidden')") is True


# ---------------------------------------------------------------- line 4: the export dialog

def test_the_daw_export_dialog_has_a_drop_hint_no_development_note_and_a_collapsed_destination(host):
    host.evaluate("() => openExport('all')")
    host.wait_for_function("() => !document.querySelector('#exportModal').classList.contains('hidden')")
    host.wait_for_function("() => !document.querySelector('#exHostNote').classList.contains('hidden')")
    note = host.locator("#exHostNote").inner_text()
    assert "start of the session" in note and "song position 0" in note, note
    assert "later part" not in note and "Spike audio types" not in note, note
    assert host.evaluate("() => document.querySelector('#exDest').checkVisibility({ visibilityProperty: true })") is False
    assert host.evaluate("() => document.querySelector('#exChoose').checkVisibility({ visibilityProperty: true })") is False
    toggle = host.locator("#exDestToggle")
    assert toggle.is_visible() and toggle.get_attribute("aria-expanded") == "false"
    toggle.click()
    assert host.locator("#exDest").is_visible() and host.locator("#exChoose").is_visible() and host.locator("#exReset").is_visible()
    assert not toggle.is_visible()
    host.evaluate("() => exCancel()")
    host.evaluate("() => openExport('all')")
    host.wait_for_function("() => !document.querySelector('#exportModal').classList.contains('hidden')")
    assert host.evaluate("() => document.querySelector('#exDest').checkVisibility({ visibilityProperty: true })") is False, "a new dialog starts collapsed"
    # a chosen folder keeps the row open
    host.evaluate("() => { EX.dest = EX.def + '/elsewhere'; exRender(); }")
    assert host.locator("#exDest").is_visible()
    host.evaluate("() => { EX.dest = EX.def; exRender(); exCancel(); }")


def test_the_standalone_export_dialog_shows_the_destination_row_as_before(plain):
    plain.evaluate("() => openExport('all')")
    plain.wait_for_function("() => !document.querySelector('#exportModal').classList.contains('hidden')")
    assert plain.locator("#exDest").is_visible() and plain.locator("#exChoose").is_visible() and plain.locator("#exReset").is_visible()
    assert plain.locator("#exDestSec .exLbl").is_visible()
    assert plain.locator("#exDestToggle").is_hidden()
    assert plain.evaluate("() => document.querySelector('#exDestSec').classList.contains('exFold')") is False
    assert plain.locator("#exHostNote").is_hidden()
    plain.evaluate("() => exCancel()")


# ---------------------------------------------------------------- line 5: the help and the first line

def test_the_daw_help_is_a_path_of_five_steps(host):
    host.evaluate("() => DSHelp.open()")
    host.wait_for_selector("#helpPage #dawSteps")
    steps = host.evaluate("() => [...document.querySelectorAll('#dawSteps li')].map(l => l.textContent)")
    assert len(steps) == 5, steps
    for step, words in zip(steps, (["plugin", "drum clips"], ["analysis"], ["Check", "correct"], ["note", "role"], ["MIDI", "spikes", "start of the session"])):
        assert all(w in step for w in words), (step, words)


def test_the_standalone_help_has_no_daw_path(plain):
    plain.evaluate("() => DSHelp.open()")
    plain.wait_for_selector("#helpPage .helpChapterPage")
    assert plain.evaluate("() => document.getElementById('dawSteps')") is None


def test_the_first_open_line_names_the_first_step_and_goes_after_the_first_action(host):
    host.evaluate("async () => { document.getElementById('hostIntro')?.remove(); await B.appSettingsSet('host_intro_done', false); await hostIntroInit(); }")
    line = host.locator("#hostIntro")
    assert line.count() == 1 and "Step 1 of 5" in line.inner_text() and "plugin" in line.inner_text() and "drum clips" in line.inner_text()
    assert host.evaluate("() => document.querySelectorAll('#hostIntro').length") == 1
    host.keyboard.press("ArrowRight")
    assert host.locator("#hostIntro").count() == 0
    wait_until(lambda: host.evaluate("async () => !!(await B.appSettingsGet()).host_intro_done"), msg="the dismissal is kept")
    host.evaluate("async () => { await hostIntroInit(); }")
    assert host.locator("#hostIntro").count() == 0, "it does not come back after the first action"
    # a click is an action too
    host.evaluate("async () => { await B.appSettingsSet('host_intro_done', false); await hostIntroInit(); }")
    assert host.locator("#hostIntro").count() == 1
    host.mouse.click(5, 300)
    assert host.locator("#hostIntro").count() == 0


def test_the_standalone_page_has_no_first_open_line(plain):
    plain.evaluate("async () => { await B.appSettingsSet('host_intro_done', false); await hostIntroInit(); }")
    assert plain.locator("#hostIntro").count() == 0
    plain.keyboard.press("ArrowRight")
    assert plain.evaluate("async () => !!(await B.appSettingsGet()).host_intro_done") is False, "a key in the standalone page keeps nothing"


# ---------------------------------------------------------------- line 6: the message after a change of the audio

def test_a_change_of_the_audio_says_what_happened_and_that_hand_edits_are_kept(host):
    host.evaluate("() => setMsg('')")
    host.evaluate("() => hostAudioChanged(['Snare Top', 'Tom 1', 'No Such Track'])")
    text = host.locator("#stMsg").inner_text()
    assert text == ("The DAW changed the audio of Drum B, Drum C. The hits were detected again. "
                    "The hits you added, moved or edited are kept."), text
    host.evaluate("() => hostAudioChanged(['Kick In', 'Snare Top', 'Tom 1', 'Overhead L'])")
    assert "Drum A, Drum B, Drum C and 1 more tracks" in host.locator("#stMsg").inner_text()
    host.evaluate("() => { setMsg('unchanged'); hostAudioChanged([]); hostAudioChanged(['No Such Track']); hostAudioChanged(null); }")
    assert host.locator("#stMsg").inner_text() == "unchanged"


def test_the_standalone_page_says_nothing_about_the_audio_of_the_daw(plain):
    plain.evaluate("() => { setMsg('unchanged'); hostAudioChanged(['Snare Top']); }")
    assert plain.locator("#stMsg").inner_text() == "unchanged"
