"""#245: the scenario parser and the scene timing of tools/media/record.py, against a stub page.
No bridge, no browser, no TTS."""
import asyncio
import json
import subprocess
import sys
import wave
from pathlib import Path
from types import SimpleNamespace

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools" / "media"))
import record  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools" / "media" / "explainers"))

ROOT = Path(__file__).resolve().parents[1]
STEP_S = 0.02   # every stub step costs this much fake time


class Clock:
    """A fake monotonic clock the stub advances."""

    def __init__(self):
        self.t = 0.0

    def __call__(self):
        return self.t

    def advance(self, dt):
        self.t += dt


class StubPage:
    """A page that consumes the fake clock: every step costs STEP_S, a hold costs its ms."""

    def __init__(self, clock, step_s=STEP_S):
        self.clock = clock
        self.step_s = step_s
        self.calls = []
        self.hold_calls = []
        self.hidden_flag = False          # what an "is the element hidden" evaluate returns
        self.waited_hidden = []           # selectors wait_for_selector saw with state="hidden"
        self.text: "str | None" = None   # the page's visible text, returned for the model-text check
        self.dom_text: "list | None" = None   # what the DOM guard reads (the status bar, dialogs, input values)
        self.keyboard = SimpleNamespace(press=lambda k: self._tick(("key", k)))

    def _tick(self, what):
        self.calls.append(what)
        self.clock.advance(self.step_s)

    def evaluate(self, js, *a) -> "str | bool | list | float | None":
        self._tick(("evaluate", js, a))
        if js == record.VISIBLE_TEXT_JS:
            return self.text
        if js == record.DOM_TEXTS_JS:
            return self.dom_text
        if isinstance(js, str) and "classList.contains('hidden')" in js:
            return self.hidden_flag
        return None

    def wait_for_function(self, cond, arg=None, timeout=None):
        self._tick(("wait_function", cond))

    def wait_for_selector(self, sel, state=None, timeout=None):
        self._tick(("wait_selector", sel))
        if state == "hidden":
            self.waited_hidden.append(sel)

    def wait_for_timeout(self, ms):
        self.hold_calls.append(ms)
        self.clock.advance(ms / 1000)

    def click(self, sel, timeout=None):
        self._tick(("click", sel))

    def select_option(self, sel, value, timeout=None):
        self._tick(("select", sel, value))

    def holds(self):
        return list(self.hold_calls)

    def evaluated(self):
        return [c for c in self.calls if c[0] == "evaluate"]


class LanePage(StubPage):
    """A page with the mouse and the lane geometry the real lane steps use."""

    BOX = {"x": 90.0, "y": 200.0, "width": 1800.0, "height": 120.0}

    def __init__(self, clock):
        super().__init__(clock)
        self.mouse = SimpleNamespace(
            click=lambda x, y: self._tick(("mouse", "click", x, y)),
            move=lambda x, y, steps=1: self._tick(("mouse", "move", x, y)),
            down=lambda: self._tick(("mouse", "down")),
            up=lambda: self._tick(("mouse", "up")))

    def evaluate(self, js, *a):
        if js == record.LANE_X_JS:
            self._tick(("evaluate", js, a))
            return 900.0   # the page x of the asked time
        return super().evaluate(js, *a)

    def locator(self, sel):
        self._tick(("locator", sel))
        # the real locator scrolls the row into view first (13 rows scroll); the stub models it
        return SimpleNamespace(scroll_into_view_if_needed=lambda: self._tick(("scroll_into_view", sel)),
                               bounding_box=lambda: dict(self.BOX))


def caption_scene(sid, text):
    return {"id": sid, "steps": [{"do": "caption", "text": text}]}


# ---------------- the scenario ----------------

def test_the_part1_scenario_parses_and_matches_the_script():
    raw = json.loads((ROOT / "tools/media/scenarios/part1.json").read_text())
    headings = [line[3:].strip() for line in
                (ROOT / "docs/product/scripts/rec-part1.md").read_text().splitlines() if line.startswith("## ")]
    # #294 removes the role suggestion from activations: p1-role shows a screen the shipped app no
    # longer has, so the recording skips it and its narration WAV stays unused.
    assert raw["skip"] == ["p1-role"]
    # the media plan: the video is 1920x1080 at 2x device scale
    assert raw["viewport"] == {"width": 1920, "height": 1080} and raw["device_scale_factor"] == 2
    recorded = record.parse_scenario(raw)
    assert [sc["id"] for sc in recorded["scenes"]] == [h for h in headings if h not in raw["skip"]]


def test_a_skip_must_name_scenes_of_the_scenario():
    ok = {"scenes": [{"id": "a", "steps": []}, {"id": "b", "steps": []}], "skip": ["b"]}
    assert [sc["id"] for sc in record.parse_scenario(ok)["scenes"]] == ["a"]
    with pytest.raises(record.RecordError, match="skip"):
        record.parse_scenario({"scenes": [{"id": "s", "steps": []}], "skip": ["nope"]})
    with pytest.raises(record.RecordError, match="every scene is skipped"):
        record.parse_scenario({"scenes": [{"id": "s", "steps": []}], "skip": ["s"]})


def test_a_scene_with_explicit_shot_steps_takes_its_pictures_at_them(tmp_path, monkeypatch):
    # A dialog scene names the dialog as its area: the dialog is not present at the first
    # action, so both pictures are taken at shot steps while the dialog is open.
    shots = []
    monkeypatch.setattr(record, "shoot_scene",
                        lambda page, sc, path: (Path(path).write_bytes(b"png"), shots.append(Path(path).name)))
    monkeypatch.setattr(record, "run_step", lambda page, st, env: None)
    clock = Clock()
    page = StubPage(clock)
    scene = {"id": "pl-shot", "area_sel": "#someModal", "steps": [
        {"do": "caption", "text": "c"},
        {"do": "key", "press": "k"},
        {"do": "visible", "sel": "#someModal", "timeout": 15, "err": "the dialog"},
        {"do": "shot"},
        {"do": "wait", "cond": "() => true", "timeout": 15, "err": "the change"},
        {"do": "hold", "s": 1.5},
        {"do": "shot"},
        {"do": "hold", "s": 1.5}]}
    record.run_scenes(page, [scene], [8.0], clock=clock, shots=tmp_path)
    assert shots == ["pl-shot-before.png", "pl-shot-after.png"]
    assert not (tmp_path / "pl-shot-after.cand.png").exists()


def test_an_unknown_step_kind_is_rejected():
    with pytest.raises(record.RecordError, match="unknown step"):
        record.parse_scenario({"scenes": [{"id": "s", "steps": [{"do": "dance"}]}]})


def test_keymap_commands_come_from_the_key_table():
    commands = record.keymap_commands((ROOT / "static/keymap.js").read_text())
    assert ("File", "New Project…") in commands and ("File", "Add Tracks…") in commands
    assert ("File", "Export…") in commands and ("View", "Show Inspector") in commands


def test_a_caption_menu_title_must_be_in_the_keymap():
    ok = [caption_scene("a", "File > New Project…"), caption_scene("b", "Next: check and fix (part 2)")]
    assert record.caption_problems(ok, {("File", "New Project…")}) == []
    deep = [caption_scene("c", "View > Waveform Zoom > In")]
    assert record.caption_problems(deep, {("View > Waveform Zoom", "In")}) == []
    bad = [caption_scene("d", "File > Nope…")]
    problems = record.caption_problems(bad, {("File", "New Project…")})
    assert len(problems) == 1 and "Nope…" in problems[0] and problems[0].startswith("d:")


# ---------------- the part 2 and part 3 scenarios (#337) ----------------

def script_ids(name):
    lines = (ROOT / "docs/product/scripts" / name).read_text().splitlines()
    return [line[3:].strip() for line in lines if line.startswith("## ")]


def check_scenario(scenario_file, script_file):
    raw = json.loads((ROOT / "tools/media/scenarios" / scenario_file).read_text())
    ids = [sc["id"] for sc in raw["scenes"]]
    assert ids == script_ids(script_file), \
        f"{scenario_file}: the scene ids must be the ## ids of {script_file}, missing " \
        f"{sorted(set(script_ids(script_file)) - set(ids))}, extra {sorted(set(ids) - set(script_ids(script_file)))}"
    recorded = record.parse_scenario(raw)
    assert [sc["id"] for sc in recorded["scenes"]] == script_ids(script_file)   # nothing skipped
    # the media plan: the video is 1920x1080 at 2x device scale
    assert raw["viewport"] == {"width": 1920, "height": 1080} and raw["device_scale_factor"] == 2
    problems = record.caption_problems(recorded["scenes"],
                                       record.keymap_commands((ROOT / "static/keymap.js").read_text()))
    assert problems == []
    return recorded


def test_the_part2_scenario_parses_and_matches_the_script():
    recorded = check_scenario("part2.json", "rec-part2.md")
    assert len(recorded["scenes"]) == 10


def test_the_part3_scenario_parses_and_matches_the_script():
    recorded = check_scenario("part3.json", "rec-part3.md")
    assert len(recorded["scenes"]) == 10


def test_the_part3_session_file_is_the_sub_mic_the_wait_targets():
    raw = json.loads((ROOT / "tools/media/scenarios/part3.json").read_text())
    assert raw["session_files"] == [{"from": "{demo}/k out.wav", "as": "k sub.wav"}]
    cond = next(st["cond"] for st in raw["scenes"][0]["steps"] if "ufollowWrap" in st.get("cond", ""))  # the standard's zoom_to sits before it
    assert 'data-stem="k sub"' in cond   # a plain CSS attribute selector; the over-escaped \\" form matched no element in the DOM


def test_session_files_are_placed_after_the_seed_into_the_dialog_folder(tmp_path):
    (tmp_path / "demo").mkdir()
    wav_file(tmp_path / "demo" / "k out.wav", 0.5)
    (tmp_path / "session" / "Audio Files").mkdir(parents=True)
    env = {"session": str(tmp_path / "session"), "demo": str(tmp_path / "demo")}
    record.place_session_files({"session_files": [{"from": "{demo}/k out.wav", "as": "k sub.wav"}]}, env)
    assert (tmp_path / "session" / "Audio Files" / "k sub.wav").is_file()
    assert not (tmp_path / "demo" / "k sub.wav").exists()   # the demo seed itself stays the 13 WAVs
    record.place_session_files({}, env)                     # no key: nothing is placed


def test_the_daw_midi_carries_the_tempo_and_meter_events(tmp_path):
    # the Import Tempo Map scene's {work}/tempo.mid: the import refuses a file without both
    path = tmp_path / "tempo.mid"
    record.write_daw_midi(path)
    data = path.read_bytes()
    assert data[:4] == b"MThd" and data[4:8] == (6).to_bytes(4, "big")
    assert data[8:10] == (0).to_bytes(2, "big")             # format 0
    assert data[10:12] == (1).to_bytes(2, "big")            # one track
    assert data[12:14] == (480).to_bytes(2, "big")          # PPQ
    assert data[14:18] == b"MTrk"
    track = data[22:]
    assert b"\x00\xff\x58\x04\x04\x02\x18\x08" in track     # 4/4 at tick 0
    assert b"\x00\xff\x51\x03\x0a\x85\xf7" in track         # 87 BPM at tick 0
    assert b"\xbc\x00\xff\x58\x04\x03\x02\x18\x08" in track # 3/4 at tick 7680 (bar 5)
    assert b"\xbc\x00\xff\x51\x03\x09\x27\xc0" in track     # 100 BPM at tick 7680


def test_the_one_shot_is_the_head_of_the_sub_mic(tmp_path):
    # the Layer scene's {work}/one-shot.wav: a layer one-shot is at most 20 s, so the 64 s
    # sub mic recording itself is refused; record.py trims its head into the work dir
    demo = tmp_path / "demo"
    demo.mkdir()
    wav_file(demo / "k out.wav", 2.0, rate=48000)
    dest = record.write_one_shot(tmp_path, demo)
    with wave.open(str(dest)) as w:
        assert w.getnframes() == int(1.5 * w.getframerate()) and w.getframerate() == 48000


def test_the_part3_layer_scene_picks_the_work_one_shot():
    raw = json.loads((ROOT / "tools/media/scenarios/part3.json").read_text())
    layer = next(sc for sc in raw["scenes"] if sc["id"] == "p3-layer")
    pick = [st for st in layer["steps"] if st["do"] == "answer_pick"]  # the hold of the standard sits before it
    assert pick == [{"do": "answer_pick", "path": "{work}/one-shot.wav"}]


# ---------------- the scene timing (a stub page) ----------------

def two_caption_scenes():
    return [caption_scene("s1", "File > New Project…"), caption_scene("s2", "Next: check and fix (part 2)")]


def test_a_scene_is_held_to_its_narration_length():
    clock = Clock()
    page = StubPage(clock)                     # the stub advances the clock the runner reads
    marks, drifts, waits = record.run_scenes(page, two_caption_scenes(), [1.0, 0.5], clock=clock)
    # per scene: the modal guard, the DOM guard, the caption step, the model-text check and the
    # caption clear run inside the scene, then the hold puts the scene's span on its narration
    assert marks == [0.0, 1.0] and drifts == [0.0, 0.0]
    assert page.holds() == [900, 400]          # five STEP_S evaluates per scene, the rest waits


def test_an_overrunning_scene_drifts_and_is_reported():
    clock = Clock()
    page = StubPage(clock, step_s=0.3)         # one caption costs more than the whole scene
    scenes = [caption_scene("s1", "A > B"), caption_scene("s2", "no menu")]
    marks, drifts, waits = record.run_scenes(page, scenes, [0.1, 0.1], clock=clock)
    assert marks == pytest.approx([0.0, 1.5]) and page.holds() == [0, 0]
    assert drifts == pytest.approx([1.4, 1.4])   # each scene's own overrun (1.5 s of steps over 0.1 s)
    problems = record.drift_problems(scenes, drifts)
    assert len(problems) == 2 and all("limit 100 ms" in p for p in problems)


def test_a_scene_after_an_overrun_keeps_its_full_span():
    # the bug the first live repair ran into: the old hold targeted the global schedule, so a
    # scene after an overrun lost its slack (its raw span came up short against its narration)
    clock = Clock()
    page = StubPage(clock)
    over = {"id": "s1", "steps": [{"do": "caption", "text": "no menu"}] * 30}   # 0.68 s over a 0.5 s narration
    scenes = [over, caption_scene("s2", "no menu")]
    marks, drifts, _ = record.run_scenes(page, scenes, [0.5, 0.5], clock=clock)
    assert marks == pytest.approx([0.0, 0.68])
    assert page.holds() == [0, 400]            # s2 is held to its full narration length
    assert drifts == pytest.approx([0.18, 0.0])


def test_the_caption_ends_with_its_own_scene():
    clock = Clock()
    page = StubPage(clock)
    record.run_scenes(page, two_caption_scenes(), [1.0, 0.5], clock=clock)
    clears = [(js, args) for js, args in
              ((c[1], c[2]) for c in page.evaluated()) if "__cap" in js and args == ("",)]
    assert len(clears) == 2   # one fade-out per scene, after the scene's narration


# ---------------- the keys and the model text (the review's 2 and 5) ----------------

def test_the_key_label_names_the_keys_like_the_review():
    assert record.key_label("Meta+N") == "Cmd+N"
    assert record.key_label("Shift+Meta+I") == "Shift+Cmd+I"
    assert record.key_label("Alt+Meta+E") == "Option+Cmd+E"
    assert record.key_label("Meta+E") == "Cmd+E"
    assert record.key_label("Tab") == "Tab" and record.key_label("6") == "6"


def test_a_key_step_shows_the_key_in_the_overlay():
    clock = Clock()
    page = StubPage(clock)
    record.run_step(page, {"do": "key", "press": "Shift+Meta+I"}, {})
    shows = [(js, args) for js, args in ((c[1], c[2]) for c in page.evaluated()) if "dsShowKey" in js]
    assert len(shows) == 1 and shows[0][1] == (("Shift+Cmd+I", record.KEY_HOLD_MS),)
    assert record.KEY_HOLD_MS >= 1500   # the review: every key visible for at least 1.5 s


def test_pw_press_translates_the_keymap_notation_to_playwright():
    # Playwright rejects the key table's names for the arrows and Return (p2-nudge rc=1)
    assert record.pw_press("Left") == "ArrowLeft" and record.pw_press("Right") == "ArrowRight"
    assert record.pw_press("Return") == "Enter" and record.pw_press("NumpadMinus") == "NumpadSubtract"
    assert record.pw_press("Shift+Meta+Z") == "Shift+Meta+Z" and record.pw_press("Alt+O") == "Alt+O"
    assert record.pw_press("Tab") == "Tab" and record.pw_press("6") == "6"


def test_a_hide_key_presses_only_when_the_element_is_visible():
    # the p2-step fix: the overview shows by default, so the demoed Alt+O must find it hidden
    # first — hide_key presses the toggle only when visible (no badge, a state normalization)
    clock = Clock()
    page = StubPage(clock)
    page.hidden_flag = True
    record.run_step(page, {"do": "hide_key", "sel": "#overviewWrap", "press": "Alt+O"}, {})
    assert not any(c[0] == "key" for c in page.calls) and page.waited_hidden == []   # already hidden: no press
    page.hidden_flag = False
    record.run_step(page, {"do": "hide_key", "sel": "#overviewWrap", "press": "Alt+O"}, {})
    assert ("key", "Alt+O") in page.calls and page.waited_hidden == ["#overviewWrap"]
    shows = [(js, args) for js, args in ((c[1], c[2]) for c in page.evaluated()) if "dsShowKey" in js]
    assert shows == []   # the normalization press shows no key badge


def test_a_menu_step_runs_the_item_through_the_bridge_event():
    # the menu-only commands (no key in keymap.js: Detect Tempo…, Set Bar 1, Import Tempo Map…)
    clock = Clock()
    page = StubPage(clock)
    record.run_step(page, {"do": "menu", "id": "detect_tempo"}, {})
    fetches = [(js, args) for js, args in ((c[1], c[2]) for c in page.evaluated()) if "selftest_menu" in js]
    assert len(fetches) == 1 and fetches[0][1] == ("detect_tempo",)
    assert "/invoke/selftest_menu" in fetches[0][0]
    waits = [c for c in page.calls if c[0] == "wait_function" and "DSShell" in c[1]]
    assert len(waits) == 1 and "seen" in waits[0][1]   # the wait is for the handler's start


def test_visible_model_text_fails_the_scene():
    clock = Clock()
    page = StubPage(clock)
    page.text = "No usable model: fallback detector, check toms and cymbals"
    with pytest.raises(record.RecordError, match="model"):
        record.run_scenes(page, [caption_scene("s1", "no menu")], [1.0], clock=clock)


def test_the_model_check_can_be_turned_off():
    clock = Clock()
    page = StubPage(clock)
    page.text = "no model: the slider applies to the hits"
    record.run_scenes(page, [caption_scene("s1", "no menu")], [1.0], clock=clock, check_model=False)


# ---------------- the modal guard (the part2 undo history left its dialog open) ----------------

class ModalPage(LanePage):
    """A stub that tracks .modal visibility: Alt+Meta+Z opens the undo history (the app's real
    Option+Cmd+Z binding, static/keymap.js undo_history) and #undoHistDone closes it, like the
    app's showModal and its Close button do."""

    def __init__(self, clock):
        super().__init__(clock)
        self.open_modals = []
        self.keyboard = SimpleNamespace(press=self._press)

    def _press(self, k):
        self._tick(("key", k))
        if k == "Alt+Meta+Z":
            self.open_modals = ["undoHistModal"]

    def evaluate(self, js, *a):
        if js == record.MODALS_JS:
            return list(self.open_modals)
        return super().evaluate(js, *a)

    def click(self, sel, timeout=None):
        super().click(sel, timeout)
        if sel == "#undoHistDone":
            self.open_modals = []


def old_part2_tail():
    """The p2-undo of the gate5 render (verbatim) plus the scene that followed it: the undo
    history modal opens at Alt+Meta+Z and nothing ever closes it."""
    return [
        {"id": "p2-undo", "steps": [
            {"do": "caption", "text": "Edit > Undo History…"},
            {"do": "key", "press": "Meta+Z"},
            {"do": "hold", "s": 0.8},
            {"do": "key", "press": "Shift+Meta+Z"},
            {"do": "hold", "s": 0.8},
            {"do": "key", "press": "Alt+Meta+Z"},
            {"do": "hold", "s": 1.2},
        ]},
        {"id": "p2-velocity", "steps": [
            {"do": "caption", "text": "View > Velocity Lane"},
            {"do": "key", "press": "V"},
            {"do": "wait", "cond": "() => typeof VL !== 'undefined' && VL.on", "timeout": 15, "err": "the velocity lane"},
        ]},
    ]


def test_the_old_part2_fails_the_modal_guard_at_the_next_scene():
    # the gate5 defect: the undo history opened in p2-undo and dimmed every later scene (66-99 s)
    clock = Clock()
    page = ModalPage(clock)
    with pytest.raises(record.RecordError,
                       match="p2-velocity: a modal is open at the start of the scene: undoHistModal"):
        record.run_scenes(page, old_part2_tail(), [8.411, 11.811], clock=clock)


def test_the_new_part2_closes_the_modal_and_passes_the_guard():
    raw = json.loads((ROOT / "tools/media/scenarios/part2.json").read_text())
    clock = Clock()
    page = ModalPage(clock)
    record.run_scenes(page, record.parse_scenario(raw)["scenes"], [8.411] * 10, clock=clock)
    assert page.open_modals == []              # the Close click ran, the check step saw it hidden


def test_the_part2_undo_scene_closes_the_dialog_about_3_s_after_it_opens():
    raw = json.loads((ROOT / "tools/media/scenarios/part2.json").read_text())
    steps = next(sc for sc in raw["scenes"] if sc["id"] == "p2-undo")["steps"]
    opened = next(i for i, st in enumerate(steps) if st.get("press") == "Alt+Meta+Z")
    assert steps[opened + 1] == {"do": "hold", "s": 3.0}        # ~3 s with the modal open
    assert steps[opened + 2] == {"do": "click", "sel": "#undoHistDone"}
    check = steps[opened + 3]
    assert check["do"] == "wait" and "undoHistModal" in check["cond"] and "hidden" in check["cond"]


# ---------------- the DOM guard (the part1 private paths, coordinator review 6403564803) ----------------

class ExportPage(LanePage):
    """A stub that models the two private-path leaks of the old part1: the status bar keeps
    'created Demo in <work home>' after the new project (the app's own setMsg) and #exDest
    holds the work-home path after exRender re-renders the input — at the Export click and
    in the awaited answers that follow the scenario's last show_dest (exInfo's reply lands
    while the dialog is closing; the 42a7534 gate's p1-again guard read it there)."""

    OLD_ST_MSG = "created Demo in /Users/x86-mac/.cache/drumspike/media/rec/work/home/projects/Demo"
    OLD_EX_DEST = "/Users/x86-mac/.cache/drumspike/media/rec/work/home/projects/Demo/Exports"

    def __init__(self, clock):
        super().__init__(clock)
        self.st_msg = None   # the #stMsg text the page shows
        self.ex_dest = None  # the #exDest input value the page shows
        self.dest_shown = None   # the page global a show_dest keeps (the exRender hook)
        self.keyboard = SimpleNamespace(press=self._press)

    def _press(self, k):
        self._tick(("key", k))
        if k == "Meta+N":
            self.st_msg = self.OLD_ST_MSG

    def _ex_render(self):
        self.ex_dest = self.OLD_EX_DEST          # exRender writes the app's own dest first
        if self.dest_shown is not None:
            self.ex_dest = self.dest_shown       # the hook re-asserts the neutral form

    def click(self, sel, timeout=None):
        super().click(sel, timeout)
        if sel == "#exGo":
            self._ex_render()   # exRender's rewrite at the Export click
        elif sel == "#exCancel":
            self._ex_render()   # the awaited answers re-render once more, after the last show_dest

    def evaluate(self, js, *a):
        if js == record.LANE_X_JS:
            return super().evaluate(js, *a)   # LanePage answers the lane geometry
        self._tick(("evaluate", js, a))
        if js == record.MODALS_JS:
            return []
        if js == record.DOM_TEXTS_JS:
            return [t for t in (self.text, self.st_msg, self.ex_dest) if t]
        if js == record.SHOW_TEXT_JS:
            self.st_msg = a[0][1]
            return None
        if js == record.SHOW_DEST_JS:
            self.dest_shown = a[0]
            self.ex_dest = a[0]
            return None
        return None


def test_the_dom_guard_fails_on_a_private_path():
    clock = Clock()
    page = StubPage(clock)
    page.dom_text = ["created Demo in /Users/x86-mac/.cache/drumspike/media/rec/work/home/projects"]
    with pytest.raises(record.RecordError) as e:
        record.run_scenes(page, [caption_scene("s1", "no menu")], [1.0], clock=clock)
    assert "s1: the page shows forbidden text" in str(e.value)
    assert "/Users/" in str(e.value) and "x86-mac" in str(e.value)


def test_the_dom_guard_reads_the_dialog_input_values():
    # the leak the coordinator saw at 41 s: the Export dialog's destination is an input value,
    # invisible to the body-text check of the model guard
    clock = Clock()
    page = StubPage(clock)
    page.dom_text = [ExportPage.OLD_EX_DEST]
    with pytest.raises(record.RecordError) as e:
        record.run_scenes(page, [caption_scene("s1", "no menu")], [1.0], clock=clock)
    assert "/Users/" in str(e.value) and "x86-mac" in str(e.value)


def test_the_dom_guard_keeps_the_path_words_without_the_model_check():
    clock = Clock()
    page = StubPage(clock)
    page.dom_text = ["no model: the slider applies to the hits"]
    record.run_scenes(page, [caption_scene("s1", "no menu")], [1.0], clock=clock,
                      check_model=False)   # --allow-model-text: the model word is allowed
    page.dom_text = [ExportPage.OLD_EX_DEST]
    with pytest.raises(record.RecordError, match="forbidden text"):
        record.run_scenes(page, [caption_scene("s1", "no menu")], [1.0], clock=clock,
                          check_model=False)   # the path words always fail


def old_part1_tail():
    """The p1-new, p1-export and p1-again of the gate7 part1 (verbatim): the status bar keeps
    'created Demo in <work home>' from the new project on, and the export dialog's #exDest
    holds the work-home path from exRender's rewrite on."""
    return [
        {"id": "p1-new", "steps": [
            {"do": "caption", "text": "File > New Project…"},
            {"do": "hold", "s": 0.6},
            {"do": "answer_save", "path": "{projects}/Demo"},
            {"do": "key", "press": "Meta+N"},
            {"do": "wait", "cond": "() => S.name === 'Demo'", "timeout": 60, "err": "the new project Demo"},
        ]},
        {"id": "p1-export", "steps": [
            {"do": "key", "press": "Meta+E"},
            {"do": "visible", "sel": "#exportModal", "timeout": 30, "err": "the Export dialog"},
            {"do": "wait", "cond": "() => true", "timeout": 15, "err": "the defaults"},
            {"do": "hold", "s": 3.0},
            {"do": "click", "sel": "#exGo"},
            {"do": "wait", "cond": "d0 => !EX.busy && EX.done > d0", "arg": "() => EX.done",
             "timeout": 120, "err": "the export finished"},
            {"do": "visible", "sel": "#exResult", "timeout": 15, "err": "the summary lists the files"},
            {"do": "hold", "s": 2.0},
            {"do": "click", "sel": "#exCancel"},
            {"do": "wait", "cond": "() => document.querySelector('#exportModal').classList.contains('hidden')",
             "timeout": 15, "err": "the dialog closed"},
        ]},
        {"id": "p1-again", "steps": [
            {"do": "key", "press": "Alt+Meta+E"},
            {"do": "wait", "cond": "d0 => EX.done > d0", "arg": "() => EX.done",
             "timeout": 120, "err": "the second export finished"},
            {"do": "wait", "cond": "() => document.querySelector('#stMsg').textContent.startsWith('exported')",
             "timeout": 15, "err": "the summary shows the files again"},
            {"do": "hold", "s": 2.5},
        ]},
    ]


def test_the_old_part1_fails_the_dom_guard_at_the_next_scene():
    # the status-bar leak: 'created Demo in /Users/x86-mac/...' shows from the new project on
    clock = Clock()
    page = ExportPage(clock)
    with pytest.raises(record.RecordError,
                       match="p1-export: the page shows forbidden text"):
        record.run_scenes(page, old_part1_tail()[:2], [1.0, 1.0], clock=clock)


def test_the_old_part1_export_dialog_fails_the_dom_guard():
    # the dialog leak the coordinator saw at 41 s: exRender rewrote #exDest after the export
    clock = Clock()
    page = ExportPage(clock)
    with pytest.raises(record.RecordError,
                       match="p1-again: the page shows forbidden text"):
        record.run_scenes(page, old_part1_tail()[1:], [1.0, 1.0], clock=clock)


def test_the_new_part1_shows_no_private_path_at_any_scene_start():
    raw = json.loads((ROOT / "tools/media/scenarios/part1.json").read_text())
    clock = Clock()
    page = ExportPage(clock)
    record.run_scenes(page, record.parse_scenario(raw)["scenes"], [1.0] * 7, clock=clock)
    assert page.st_msg == "created Demo in ~/Music/Drum Spike/Projects/Demo"
    assert page.ex_dest == "~/Music/Drum Spike/Exports"   # kept through the export's re-renders


def test_the_neutral_dest_survives_a_re_render_after_the_last_show_dest():
    # the 42a7534 gate's p1-again: the awaited exInfo/exDrag answers exRender-rewrite #exDest
    # after the scenario's last show_dest (the dialog is closing then, the guard still reads
    # the input value) — a one-time write loses that race, the kept show does not
    raw = json.loads((ROOT / "tools/media/scenarios/part1.json").read_text())
    clock = Clock()
    page = ExportPage(clock)
    record.run_scenes(page, record.parse_scenario(raw)["scenes"], [1.0] * 7, clock=clock)
    assert page.ex_dest != ExportPage.OLD_EX_DEST
    assert page.ex_dest == "~/Music/Drum Spike/Exports"


def test_the_dest_is_set_before_the_dialog_opens():
    # the coordinator's send-back: the neutral destination is in place before Meta+E, so the
    # dialog's own opening re-renders (exOpen's exChanged) show the neutral form too
    raw = json.loads((ROOT / "tools/media/scenarios/part1.json").read_text())
    steps = next(sc for sc in raw["scenes"] if sc["id"] == "p1-export")["steps"]
    dest = next(i for i, st in enumerate(steps) if st.get("do") == "show_dest")
    key = next(i for i, st in enumerate(steps) if st.get("press") == "Meta+E")
    assert dest < key and steps[dest]["text"] == "~/Music/Drum Spike/Exports"


def test_a_show_dest_and_show_text_step_carry_the_neutral_form():
    clock = Clock()
    page = ExportPage(clock)
    record.run_step(page, {"do": "show_dest", "text": "~/Music/Drum Spike/Exports"}, {})
    record.run_step(page, {"do": "show_text", "sel": "#stMsg",
                           "text": "created Demo in ~/Music/Drum Spike/Projects/Demo"}, {})
    assert page.ex_dest == "~/Music/Drum Spike/Exports"
    assert page.st_msg == "created Demo in ~/Music/Drum Spike/Projects/Demo"


def test_the_show_dest_js_keeps_the_text_against_the_app_s_re_renders():
    # a one-time write lost the 42a7534 race: the JS hooks the app's exRender, so every
    # re-render of #exDest re-asserts the neutral text
    assert "__dsDestShown" in record.SHOW_DEST_JS
    assert record.SHOW_DEST_JS.index("__dsDestKeep") < record.SHOW_DEST_JS.index("window.exRender")


def test_show_steps_need_their_text():
    with pytest.raises(record.RecordError, match="show_dest step needs"):
        record.parse_scenario({"scenes": [{"id": "s", "steps": [{"do": "show_dest"}]}]})
    with pytest.raises(record.RecordError, match="show_text step needs"):
        record.parse_scenario({"scenes": [{"id": "s", "steps": [{"do": "show_text", "text": "x"}]}]})


def test_a_lane_click_and_a_drag_move_the_real_mouse():
    # the Learn scenes: the ghost selection clicks a hit's spike, the K-add drags a wave-lane
    # range — both at lane positions no element selector addresses (the 428 plan's two kinds)
    clock = Clock()
    page = LanePage(clock)
    record.run_step(page, {"do": "lane_click", "track": "s top", "t": 3.2}, {})
    assert ("mouse", "click", 900.0, 260.0) in page.calls   # the lane's middle height
    record.run_step(page, {"do": "drag", "track": "s top", "from": 3.0, "to": 3.4}, {})
    kinds = [c[1] for c in page.calls if c[0] == "mouse"]
    assert kinds[-5:] == ["move", "down", "move", "move", "up"]
    assert not any(c[0] == "wait_function" and "DSLearn" in c[1] for c in page.calls)
    record.run_step(page, {"do": "drag", "track": "s top", "from": 3.0, "to": 3.4, "wait_sel": True}, {})
    waits = [c[1] for c in page.calls if c[0] == "wait_function" and "DSLearn" in c[1]]
    assert waits == ["() => DSLearn.state().sel !== null"]


def test_lane_steps_need_track_and_times():
    with pytest.raises(record.RecordError, match="lane_click step needs"):
        record.parse_scenario({"scenes": [{"id": "s", "steps": [{"do": "lane_click", "t": 1.0}]}]})
    with pytest.raises(record.RecordError, match="drag step needs"):
        record.parse_scenario({"scenes": [{"id": "s", "steps": [{"do": "drag", "track": "s top"}]}]})


class LateStatusPage(ExportPage):
    """The app writes 'created Demo in <host>' in a task of its own: setMsg runs after the
    `await useProject(r)` that flips S.name (static/app.js), so the first #341 fix's rewrite
    right after the S.name wait was overwritten by it — the failed gate run's p1-add guard."""

    def __init__(self, clock):
        super().__init__(clock)
        self._pending_write = False   # S.name flipped; the app's setMsg is still in flight

    def _press(self, k):
        self._tick(("key", k))
        if k == "Meta+N":
            self._pending_write = True

    def _flush(self):
        if self._pending_write:
            self._pending_write = False
            self.st_msg = self.OLD_ST_MSG

    def wait_for_function(self, cond, arg=None, timeout=None):
        if self._pending_write and "created Demo" in cond:
            self._flush()   # the write lands while the page waits for the app's own line
        return super().wait_for_function(cond, arg=arg, timeout=timeout)

    def evaluate(self, js, *a):
        out = super().evaluate(js, *a)
        self._flush()   # the app's task runs after whatever evaluate was in flight
        return out


def rewritten_without_waiting():
    """The p1-new of the first #341 fix: the neutral rewrite right after the S.name wait,
    with no step that waits for the app's own 'created Demo' line first."""
    return [
        {"id": "p1-new", "steps": [
            {"do": "caption", "text": "File > New Project…"},
            {"do": "answer_save", "path": "{projects}/Demo"},
            {"do": "key", "press": "Meta+N"},
            {"do": "wait", "cond": "() => S.name === 'Demo'", "timeout": 60, "err": "the new project Demo"},
            {"do": "show_text", "sel": "#stMsg", "text": "created Demo in ~/Music/Drum Spike/Projects/Demo",
             "err": "the status bar of the new project, in the neutral home-relative form"},
        ]},
        {"id": "p1-add", "steps": [{"do": "caption", "text": "File > Add Tracks…"}]},
    ]


def test_a_rewrite_before_the_app_s_own_line_is_overwritten():
    # the failed gate run: the app's setMsg is a later task than the S.name flip, so the
    # rewrite must wait for the line it replaces
    clock = Clock()
    page = LateStatusPage(clock)
    with pytest.raises(record.RecordError, match="p1-add: the page shows forbidden text"):
        record.run_scenes(page, rewritten_without_waiting(), [1.0, 1.0], clock=clock)


def test_the_new_part1_waits_for_the_app_s_line_before_the_rewrite():
    raw = json.loads((ROOT / "tools/media/scenarios/part1.json").read_text())
    clock = Clock()
    page = LateStatusPage(clock)
    record.run_scenes(page, record.parse_scenario(raw)["scenes"], [1.0] * 7, clock=clock)
    assert page.st_msg == "created Demo in ~/Music/Drum Spike/Projects/Demo"   # not overwritten


# ---------------- the steps on the stub (overlay and dialogs) ----------------

def test_the_caption_goes_through_the_overlay_element():
    clock = Clock()
    page = StubPage(clock)
    record.run_step(page, {"do": "caption", "text": "File > Add Tracks…"}, {})
    ((js, args),) = [(c[1], c[2]) for c in page.evaluated()]
    assert "__cap" in js and args == ("File > Add Tracks…",)


def test_answer_save_and_pick_expand_placeholders():
    clock = Clock()
    page = StubPage(clock)
    env = {"projects": "/w/home/projects", "session": "/w/session"}
    record.run_step(page, {"do": "answer_save", "path": "{projects}/Demo"}, env)
    record.run_step(page, {"do": "answer_pick", "path": "{session}/Audio Files"}, env)
    js_args = [(c[1], c[2]) for c in page.evaluated()]
    assert "__DS_SAVE__" in js_args[0][0] and js_args[0][1] == ("/w/home/projects/Demo",)
    assert "__DS_PICK__" in js_args[1][0] and js_args[1][1] == ("/w/session/Audio Files",)


def test_expand_tildes():
    assert str(record.expand("~/x", {})) == str(Path.home() / "x")
    assert str(record.expand("{projects}/Demo", {"projects": "/w/p"})) == "/w/p/Demo"


# ---------------- the narration ----------------

def wav_file(path, seconds, rate=8000):
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b"\x01\x01" * int(seconds * rate))


def test_narration_missing_scene_fails(tmp_path):
    wav_file(tmp_path / "a.wav", 0.5)
    (tmp_path / "scenes.json").write_text(json.dumps({"scenes": [{"id": "a", "file": "a.wav", "seconds": 0.5}]}))
    items = record.load_narration(tmp_path, ["a"])
    assert items == [("a", tmp_path / "a.wav", 0.5)]
    with pytest.raises(record.RecordError, match="no narration for scene b"):
        record.load_narration(tmp_path, ["b"])


def test_the_narration_is_placed_at_the_planned_positions(tmp_path):
    wav_file(tmp_path / "a.wav", 0.5)
    wav_file(tmp_path / "b.wav", 0.25)
    (tmp_path / "scenes.json").write_text(json.dumps(
        {"scenes": [{"id": "a", "file": "a.wav", "seconds": 0.5}, {"id": "b", "file": "b.wav", "seconds": 0.25}]}))
    items = record.load_narration(tmp_path, ["a", "b"])
    record.build_narration(items, [0.1, 0.6], 1.0, tmp_path / "narration.wav")
    with wave.open(str(tmp_path / "narration.wav"), "rb") as w:
        assert (w.getnchannels(), w.getsampwidth(), w.getframerate()) == (1, 2, 8000)
        pcm = w.readframes(w.getnframes())
    assert len(pcm) == 8000 * 2                        # the video length decides
    off_a, off_b = int(0.1 * 8000) * 2, int(0.6 * 8000) * 2
    assert pcm[:off_a] == b"\x00" * off_a              # silence before scene 1
    assert pcm[off_a:off_a + 4] == b"\x01\x01\x01\x01" and pcm[off_b:off_b + 4] == b"\x01\x01\x01\x01"


def test_the_narration_shifts_by_the_video_lead(tmp_path):
    wav_file(tmp_path / "a.wav", 0.5)
    (tmp_path / "scenes.json").write_text(json.dumps({"scenes": [{"id": "a", "file": "a.wav", "seconds": 0.5}]}))
    items = record.load_narration(tmp_path, ["a"])
    record.build_narration(items, [0.0], 1.0, tmp_path / "narration.wav", lead=0.25)
    with wave.open(str(tmp_path / "narration.wav"), "rb") as w:
        pcm = w.readframes(w.getnframes())
    off_a = int(0.25 * 8000) * 2                       # the head of the video before scene 1
    assert pcm[:off_a] == b"\x00" * off_a and pcm[off_a:off_a + 4] == b"\x01\x01\x01\x01"


# ---------------- the wait compression (option d: the mux repairs a slow scene) ----------------

def wait_scene(sid):
    return {"id": sid, "steps": [{"do": "caption", "text": "no menu"}, {"do": "wait", "cond": "() => ok"}]}


def test_the_wait_spans_of_a_scene_are_recorded():
    clock = Clock()
    page = StubPage(clock)                     # a stub wait step costs STEP_S like any step
    _, _, waits = record.run_scenes(page, [wait_scene("s1")], [1.0], clock=clock)
    assert waits == [[(0.06, 0.08)]]          # the span on the clock, the raw video plays it at 1x


def test_an_overrunning_scene_is_repaired_by_its_waits():
    # narration 0.6 s, the scene runs 0.2 s over (0.4 s of it a wait): the wait plays 2x
    assert record.wait_speeds([0.2], [[(0.0, 0.4)]]) == ([2.0], [0.2])


def test_each_scene_is_repaired_by_its_own_overrun():
    # the drifts are per scene: scene 1's repair does not see scene 0's overrun (and vice versa)
    factors, reductions = record.wait_speeds([0.2, 0.3], [[(0.0, 0.4)], [(0.8, 1.3)]])
    assert factors == [2.0, 2.5] and reductions == [0.2, 0.3]


def test_a_scene_that_fits_is_left_at_1x():
    assert record.wait_speeds([0.05], [[(0.0, 0.9)]]) == ([1.0], [0.0])


def test_a_scene_needing_more_than_3x_fails():
    with pytest.raises(record.RecordError, match="over the 3x cap"):
        record.wait_speeds([0.3], [[(0.0, 0.4)]])          # 0.1 s wait against 0.3 s: 4x


def test_a_scene_without_waits_cannot_be_repaired():
    with pytest.raises(record.RecordError, match="no wait step"):
        record.wait_speeds([0.3], [[]])
    with pytest.raises(record.RecordError, match="infinite"):
        record.wait_speeds([0.3], [[(0.0, 0.3)]])          # the wait is the whole overrun


def test_the_segments_cover_the_whole_video():
    segs = record.compression_segments([[(0.5, 1.0)]], 0.25, [2.0], 2.0)
    assert segs == [(0.0, 0.75, 1.0), (0.75, 1.25, 2.0), (1.25, 2.0, 1.0)]
    assert segs[0][0] == 0.0 and segs[-1][1] == 2.0               # head and tail stay at 1x


def test_the_badge_overlays_only_the_compressed_spans():
    segs = [(0.0, 0.75, 1.0), (0.75, 1.25, 2.0), (1.25, 2.0, 1.0)]
    fc, last = record.compression_filter(segs, {2.0: 1})
    assert "trim=start=0.750:end=1.250,setpts=(PTS-STARTPTS)/2.000000[s1]" in fc
    assert "concat=n=3:v=1:a=0[cat]" in fc
    # the 0.5 s span at 2x plays in 0.25 s, starting after the 0.75 s head
    assert "enable='between(t,0.750,1.000)'" in fc and last == "v0"
    assert "overlay=x=main_w-overlay_w-28:y=28" in fc


def test_the_repaired_scene_starts():
    # per scene: the narration length plus its own overrun minus its reduction — a compressed
    # scene lands exactly on its narration slot, a held scene never moves
    starts = record.compressed_starts(0.1, 0.25, [0.6, 0.5, 0.4], [0.2, 0.3, 0.0], [0.2, 0.3, 0.0])
    assert starts == pytest.approx([0.35, 0.95, 1.45])


def test_the_narration_follows_the_repaired_timeline(tmp_path):
    wav_file(tmp_path / "a.wav", 0.5)
    (tmp_path / "scenes.json").write_text(json.dumps({"scenes": [{"id": "a", "file": "a.wav", "seconds": 0.5}]}))
    items = record.load_narration(tmp_path, ["a"])
    record.build_narration(items, [0.0], 1.0, tmp_path / "narration.wav", starts=[0.25])
    with wave.open(str(tmp_path / "narration.wav"), "rb") as w:
        pcm = w.readframes(w.getnframes())
    off_a = int(0.25 * 8000) * 2
    assert pcm[:off_a] == b"\x00" * off_a and pcm[off_a:off_a + 4] == b"\x01\x01\x01\x01"


def test_the_speed_label_reads_like_the_answer():
    assert record.speed_label(2.0) == "2x"
    assert record.speed_label(1.45) == "1.45x"
    assert record.MAX_WAIT_SPEED == 3.0


# ---------------- the disclosure note on every frame (the landing page plays the recordings bare) ----------------

def fake_ffmpeg_run(calls):
    """A subprocess.run stub for mux: records the command, touches the tmp file os.replace moves."""
    def run(cmd, check=False, capture_output=False):
        calls.append(cmd)
        Path(cmd[-1]).touch()
        return SimpleNamespace(returncode=0)
    return run


def test_the_mux_burns_the_note_into_its_output(tmp_path, monkeypatch):
    # the coordinator's addition to the #341 send-back (6404180407): the landing page shows the
    # three recordings as their own videos, so record.py's own output carries the note — the
    # same PNG at the same place compose.py overlays it on the product video
    calls = []
    monkeypatch.setattr(record.shutil, "which",
                        lambda name: "/usr/bin/ffmpeg" if name == "ffmpeg" else None)
    monkeypatch.setattr(record.subprocess, "run", fake_ffmpeg_run(calls))
    note = tmp_path / "note.png"
    record.mux(tmp_path / "raw.mp4", tmp_path / "narration.wav", tmp_path / "out.mp4", 2.0, note=note)
    cmd = calls[0]
    assert str(note) in cmd
    assert (f"[0:v][2:v]overlay=x=main_w-overlay_w-{record.NOTE_MARGIN_PX}"
            f":y=main_h-overlay_h-{record.NOTE_BOTTOM_PX},format=yuv420p[v]") in cmd
    assert cmd.count("-map") == 2
    assert cmd[cmd.index("-map") + 1] == "[v]"
    assert cmd[cmd.index("-map", cmd.index("-map") + 1) + 1] == "1:a"


def test_the_mux_without_a_note_keeps_the_plain_narration_mux(tmp_path, monkeypatch):
    calls = []
    monkeypatch.setattr(record.shutil, "which",
                        lambda name: "/usr/bin/ffmpeg" if name == "ffmpeg" else None)
    monkeypatch.setattr(record.subprocess, "run", fake_ffmpeg_run(calls))
    record.mux(tmp_path / "raw.mp4", tmp_path / "narration.wav", tmp_path / "out.mp4", 2.0)
    cmd = calls[0]
    assert "-filter_complex" not in cmd and "-map" not in cmd
    assert str(tmp_path / "note.png") not in cmd


def test_the_note_is_the_explainers_disclosure_note():
    # the same place and style as compose.py/the explainers: right 28 / bottom 20, dim 20 px text
    assert record.NOTE_TEXT == "AI-generated"
    assert record.NOTE_PX == 20
    assert record.NOTE_MARGIN_PX == 28 and record.NOTE_BOTTOM_PX == 20


def test_the_note_renders_while_a_playwright_session_is_open(tmp_path):
    # the red of run 40241732523: the e2e suite keeps pytest-playwright's session loop open on
    # the calling thread and the sync API refuses a second loop there — the shared renderer
    # renders on its own thread, so it works in that suite too. That session loop parks in its
    # dispatcher greenlet mid-run_forever, so the calling thread keeps a RUNNING loop between
    # the tests. Opening a second session here to stage the condition raises exactly that
    # refusal when the e2e suite ran before this file (the red of run 40246501334), so: when a
    # loop already runs on this thread, that IS the condition; when none does, park one in a
    # greenlet the way the session loop does — a plain run_forever refuses to start over one.
    try:
        from playwright.sync_api import sync_playwright  # noqa: F401
        import greenlet
    except ImportError:
        pytest.skip("no playwright here (the note PNG renders in Chromium)")
    try:
        held = asyncio.get_running_loop()   # the e2e session's loop, when it ran before us
    except RuntimeError:
        held = None
    if held is None:
        loop = asyncio.new_event_loop()
        back = greenlet.getcurrent()

        def parked():
            loop.call_soon_threadsafe(back.switch)   # hand control back with the loop running
            loop.run_forever()                       # parks here, as the session loop does

        gr = greenlet.greenlet(parked)
        gr.switch()
        assert asyncio.get_running_loop() is loop and loop.is_running()
        try:
            png = record.render_note(tmp_path)   # the pre-fix render raised the refusal here
        finally:
            loop.call_soon_threadsafe(loop.stop)
            gr.switch()
            loop.close()
    else:
        png = record.render_note(tmp_path)
    assert png.is_file() and png.name == "note.png" and png.stat().st_size > 0


# ---------------- the recording standard (docs/product/recording-standard.md) ----------------

class CenterPage(StubPage):
    """A page whose click target has a centre (the real page answers CENTER_JS with its box)."""

    def evaluate(self, js, *a):
        if js == record.CENTER_JS:
            self._tick(("evaluate", js, a))
            return [100.0, 200.0]
        return super().evaluate(js, *a)


def test_a_zoom_to_step_centres_the_view_before_the_action():
    clock = Clock()
    page = StubPage(clock)
    record.run_step(page, {"do": "zoom_to", "track": "s top", "t": 16.09, "span": 5}, {})
    assert ("evaluate", record.ZOOM_TO_JS, (["s top", 16.09, 5],)) in page.calls


def test_a_zoom_span_outside_500_ms_to_5_s_is_refused():
    with pytest.raises(record.RecordError, match="zoom_to step needs"):
        record.parse_scenario({"scenes": [{"id": "s", "steps": [{"do": "zoom_to", "track": "t", "t": 1.0, "span": 60}]}]})
    with pytest.raises(record.RecordError, match="zoom_to step needs"):
        record.parse_scenario({"scenes": [{"id": "s", "steps": [{"do": "zoom_to", "track": "t", "t": 1.0, "span": 0.2}]}]})


def test_a_click_glides_the_pointer_to_its_target_before_it_lands():
    clock = Clock()
    page = CenterPage(clock)
    record.run_step(page, {"do": "click", "sel": "#inspector .x"}, {})
    glide = ("evaluate", record.GLIDE_JS, ([100.0, 200.0, record.GLIDE_MS],))
    assert glide in page.calls and ("evaluate", record.RING_JS, ([100.0, 200.0],)) in page.calls
    assert page.calls.index(glide) < page.calls.index(("click", "#inspector .x"))
    assert page.holds() == [record.GLIDE_MS]   # the step waits for the glide to land


def test_a_lane_click_glides_before_the_real_mouse_click():
    clock = Clock()
    page = LanePage(clock)
    record.run_step(page, {"do": "lane_click", "track": "s top", "t": 3.2}, {})
    glide = ("evaluate", record.GLIDE_JS, ([900.0, 260.0, record.GLIDE_MS],))
    assert page.calls.index(glide) < page.calls.index(("mouse", "click", 900.0, 260.0))


def test_a_drag_shows_its_line_and_ends_it_after_the_mouse_is_up():
    clock = Clock()
    page = LanePage(clock)
    record.run_step(page, {"do": "drag", "track": "s top", "from": 3.0, "to": 3.4}, {})
    line = page.calls.index(("evaluate", record.DRAG_LINE_JS, ([900.0, 900.0, 260.0],)))
    up = page.calls.index(("mouse", "up"))
    end = page.calls.index(("evaluate", record.DRAG_END_JS, ()))
    assert line < page.calls.index(("mouse", "down")) < up < end


def test_the_change_share_counts_the_pixels_that_moved_not_the_noise():
    import numpy as np
    a = np.zeros((10, 10, 3), np.uint8)
    b = a.copy()
    b[0, :5] = 40                     # 5 of 100 pixels moved a lot
    b[1, :5] = record.CHANGE_LEVEL - 1   # noise under the level does not count
    assert record.changed_share(a, b) == pytest.approx(5.0)
    with pytest.raises(record.RecordError, match="differ in size"):
        record.changed_share(a, np.zeros((5, 5, 3), np.uint8))


def test_an_action_scene_that_barely_changes_fails_the_gate(tmp_path, monkeypatch):
    import numpy as np
    shots = {"pl-add-before.png": np.zeros((100, 100, 3), np.uint8),
             "pl-add-after.png": np.zeros((100, 100, 3), np.uint8),
             "pl-ghost-before.png": np.zeros((100, 100, 3), np.uint8),
             "pl-ghost-after.png": np.zeros((100, 100, 3), np.uint8)}
    shots["pl-ghost-after.png"][:20] = 200   # 20 % changed
    shots["pl-add-after.png"][:1, :20] = 200  # 0.2 % changed: under the 0.5 % standard
    for name in shots:
        (tmp_path / name).write_bytes(b"png")
    monkeypatch.setattr(record, "png_rgb", lambda p: shots[Path(p).name])
    scenes = [{"id": "pl-add", "steps": [{"do": "key", "press": "k"}]},
              {"id": "pl-ghost", "steps": [{"do": "key", "press": "Shift+K"}]},
              {"id": "pl-end", "steps": [{"do": "hold", "s": 4.0}]}]
    problems = record.change_problems(tmp_path, scenes)
    assert len(problems) == 1 and problems[0].startswith("pl-add: 0.20 % of the action area changed")


def test_the_action_area_is_the_scene_s_panel_or_its_first_track():
    assert record.area_selector({"id": "p", "area_sel": "#inspector", "steps": []}) == "#inspector"
    assert record.area_selector({"id": "p", "steps": [{"do": "drag", "track": "s top", "from": 1, "to": 2}]}) \
        == '.track[data-name="s top"]'
    assert record.area_selector({"id": "p", "steps": [{"do": "hold", "s": 1}]}) == "body"


def test_a_scene_with_an_action_takes_its_before_and_after_pictures(tmp_path, monkeypatch):
    shots = []
    monkeypatch.setattr(record, "shoot_area", lambda page, sel, path: shots.append((sel, Path(path).name)))
    clock = Clock()
    page = StubPage(clock)
    scene = {"id": "pl-x", "steps": [{"do": "caption", "text": "c"}, {"do": "key", "press": "k"},
                                     {"do": "hold", "s": 2.0}]}
    record.run_scenes(page, [scene], [5.0], clock=clock, shots=tmp_path)
    assert shots == [("body", "pl-x-before.png"), ("body", "pl-x-after.png")]


def test_a_scene_with_a_later_zoom_takes_its_after_picture_before_the_zoom_moves(tmp_path, monkeypatch):
    # pl-grey zooms again after its first action: the after PNG must come while the view still
    # matches the scene's first zoom, else the measured window throws or compares wrong areas.
    shots = []
    monkeypatch.setattr(record, "shoot_scene", lambda page, sc, path: shots.append(Path(path).name))
    monkeypatch.setattr(record, "run_step", lambda page, st, env: None)
    clock = Clock()
    page = StubPage(clock)
    scene = {"id": "pl-grey", "steps": [
        {"do": "caption", "text": "c"},
        {"do": "zoom_to", "track": "s_top", "t": 8.5938, "span": 1},
        {"do": "lane_click", "track": "s_top", "lane": "spike", "t": 8.5938},
        {"do": "hold", "s": 1.5},
        {"do": "zoom_to", "track": "s_top", "t": 9.5312, "span": 1},
        {"do": "lane_click", "track": "s_top", "lane": "spike", "t": 9.5312},
        {"do": "hold", "s": 2.0}]}
    record.run_scenes(page, [scene], [5.0], clock=clock, shots=tmp_path)
    assert shots == ["pl-grey-before.png", "pl-grey-after.png"]


def test_the_after_picture_survives_a_view_moving_key(tmp_path, monkeypatch):
    # p1-zoom: Tab and the 5 ms preset move the view after the lane_click; the after PNG is the
    # last one taken while the view still matched the scene's first zoom, else the measured
    # window of the moved view throws (the 10 render's window_clip crash).
    class HoldsView(StubPage):
        def __init__(self, clock):
            super().__init__(clock)
            self.view_still = True   # the page's view still matches the scene's first zoom

        def evaluate(self, js, *a):
            if js == record.VIEW_HOLDS_JS:
                self._tick(("evaluate", js, a))
                return self.view_still
            return super().evaluate(js, *a)

    shots = []
    monkeypatch.setattr(record, "shoot_scene",
                        lambda page, sc, path: (Path(path).write_bytes(b"png"), shots.append(Path(path).name)))

    def run_step(page, st, env):
        if st.get("press") == "6":
            page.view_still = False   # the 5 ms preset moves the view

    monkeypatch.setattr(record, "run_step", run_step)
    clock = Clock()
    page = HoldsView(clock)
    scene = {"id": "pl-zoom", "steps": [
        {"do": "caption", "text": "c"},
        {"do": "zoom_to", "track": "s_top", "t": 8.5938, "span": 5},
        {"do": "lane_click", "track": "s_top", "lane": "wave", "t": 8.5},
        {"do": "hold", "s": 1.5},
        {"do": "key", "press": "Tab"},
        {"do": "hold", "s": 1.8},
        {"do": "key", "press": "6"},
        {"do": "hold", "s": 1.5}]}
    record.run_scenes(page, [scene], [8.0], clock=clock, shots=tmp_path)
    # a candidate before every step while the view held (steps 3-6), promoted at the scene end
    assert shots == ["pl-zoom-before.png"] + ["pl-zoom-after.cand.png"] * 4


class _Lane:
    def __init__(self, box):
        self.box = box

    def scroll_into_view_if_needed(self):
        pass

    def bounding_box(self):
        return self.box


class _WindowPage:
    """A page with one track's spike and wave lanes, at a 2x capture (the window's +/- 300 px)."""
    def __init__(self):
        self.shots = []

    def locator(self, sel):
        if "canvas.spike" in sel:
            return _Lane({"x": 100.0, "y": 50.0, "width": 800.0, "height": 40.0})
        return _Lane({"x": 100.0, "y": 90.0, "width": 800.0, "height": 120.0})

    def evaluate(self, js, arg=None):
        if js == record.LANE_X_JS:
            return 300.0
        if "devicePixelRatio" in js:
            return 2.0
        return None

    def screenshot(self, path, clip):
        self.shots.append((path, clip))


def test_a_zoomed_scene_measures_the_hit_window_over_the_spike_and_wave_lanes(tmp_path):
    sc = {"id": "pl-add", "steps": [{"do": "zoom_to", "track": "s_top", "t": 18.5938, "span": 0.5},
                                    {"do": "drag", "track": "s_top", "from": 18.4, "to": 18.7}]}
    page = _WindowPage()
    record.shoot_scene(page, sc, tmp_path / "pl-add-before.png")
    assert page.shots == [(str(tmp_path / "pl-add-before.png"), {"x": 150.0, "y": 50.0, "width": 300.0, "height": 160.0})]
    panel = {"id": "pl-h", "area_sel": "#inspector", "steps": [{"do": "zoom_to", "track": "s_top", "t": 1.0, "span": 5}]}
    assert record.zoom_t(panel) == 1.0 and record.area_selector(panel) == "#inspector"


def test_the_standard_needs_a_zoom_before_a_pointer_and_a_hold_after_each_action():
    with pytest.raises(record.RecordError, match="needs a zoom_to before it"):
        record.parse_scenario({"standard": True, "scenes": [{"id": "s", "steps": [
            {"do": "lane_click", "track": "s_top", "t": 1.0}, {"do": "hold", "s": 2.0}]}]})
    with pytest.raises(record.RecordError, match="no hold of 1.5 s"):
        record.parse_scenario({"standard": True, "scenes": [{"id": "s", "steps": [
            {"do": "zoom_to", "track": "s_top", "t": 1.0, "span": 5},
            {"do": "lane_click", "track": "s_top", "t": 1.0}, {"do": "key", "press": "k"},
            {"do": "hold", "s": 2.0}]}]})


def test_the_learn_recording_meets_the_standard_and_its_drift_limit():
    raw = json.loads((ROOT / "tools" / "media" / "scenarios" / "part-learn.json").read_text(encoding="utf-8"))
    recorded = record.parse_scenario(raw)
    assert [sc["id"] for sc in recorded["scenes"]][:5] == ["pl-ghost", "pl-grey", "pl-add", "pl-histogram", "pl-remove"]
    assert record.drift_limit(raw) == pytest.approx(0.010)
    assert record.drift_limit({}) == record.DRIFT_LIMIT_S


# ---------------- #458: the pre-done guards ----------------

import shutil  # noqa: E402

import check_paths  # noqa: E402


def _fake_engine(text):
    """An OCR engine stub in the RapidOCR result shape: [[box, line], ...]."""
    return lambda image: ([[0, text]], None)


def _font_file():
    import glob
    for pattern in ("/usr/share/fonts/**/DejaVuSans.ttf", "/usr/share/fonts/**/*.ttf"):
        found = glob.glob(pattern, recursive=True)
        if found:
            return found[0]
    return None


def _png_with_text(tmp_path, text, name):
    """A PNG with the text rendered into it (ffmpeg drawtext); None when no font is here."""
    font = _font_file()
    if font is None:
        return None
    out = tmp_path / name
    done = subprocess.run(
        ["ffmpeg", "-y", "-v", "error", "-f", "lavfi", "-i", "color=white:s=640x160",
         "-vf", f"drawtext=fontfile={font}:text={text}:fontcolor=black:fontsize=48:x=20:y=40",
         "-frames:v", "1", str(out)], capture_output=True, text=True)
    if done.returncode != 0:
        return None
    return out


def test_a_planted_path_png_fails_the_guard(tmp_path):
    pytest.importorskip("rapidocr_onnxruntime", reason="needs the media extra (the OCR engine)")
    png = _png_with_text(tmp_path, "/Users/x", "planted.png")
    if png is None:
        pytest.skip("no ttf font for the ffmpeg drawtext plant")
    findings = check_paths.check_file(png, check_paths._engine())
    assert findings, "the planted /Users/x was not read back"
    assert findings[0][2] == "/Users"


def test_a_clean_png_passes_the_guard(tmp_path):
    pytest.importorskip("rapidocr_onnxruntime", reason="needs the media extra (the OCR engine)")
    png = _png_with_text(tmp_path, "Drum Spike", "clean.png")
    if png is None:
        pytest.skip("no ttf font for the ffmpeg drawtext plant")
    assert check_paths.check_file(png, check_paths._engine()) == []


def test_the_guard_reads_the_engine_text(tmp_path):
    """The guard logic against an engine stub: the banned token and the read text come back."""
    findings = check_paths.check_file(tmp_path / "shot.png", _fake_engine("created in /Users/x"))
    assert findings and findings[0][2] == "/Users" and findings[0][3] == "created in /Users/x"
    assert check_paths.check_file(tmp_path / "shot.png", _fake_engine("Drum Spike")) == []


def test_the_guard_fails_loudly_without_the_engine(tmp_path, monkeypatch):
    def no_engine():
        raise check_paths.PathCheckError("the OCR engine rapidocr-onnxruntime is not installed")
    monkeypatch.setattr(check_paths, "_engine", no_engine)
    with pytest.raises(check_paths.PathCheckError, match="rapidocr"):
        check_paths.check_files([tmp_path / "x.png"])


# ---------------- #459: the path guard is the last step of the render tools ----------------

import shots  # noqa: E402
import build  # noqa: E402


def _planted_media(tmp_path, name):
    """A real 1 s media file (any pixels; the engine below is a stub): None without ffmpeg."""
    out = tmp_path / name
    done = subprocess.run(
        ["ffmpeg", "-y", "-v", "error", "-f", "lavfi", "-i", "color=white:s=320x240:d=1",
         "-c:v", "libx264", "-pix_fmt", "yuv420p", "-frames:v", "30", str(out)],
        capture_output=True, text=True)
    if done.returncode != 0:
        return None
    return out


def _planted_png(tmp_path, name):
    out = tmp_path / name
    done = subprocess.run(
        ["ffmpeg", "-y", "-v", "error", "-f", "lavfi", "-i", "color=white:s=320x240",
         "-frames:v", "1", str(out)], capture_output=True, text=True)
    if done.returncode != 0:
        return None
    return out


def _plant_engine(monkeypatch, tool):
    """The tool's loaded check_paths instance reads the stub engine: a planted /Users line."""
    monkeypatch.setattr(tool.path_guard(), "_engine",
                        lambda: _fake_engine("created in /Users/arm64-mac/x"))


def test_record_py_fails_on_a_planted_frame(tmp_path, monkeypatch, capsys):
    """The recorder's wired-in guard exits non-zero and names the frame (a real MP4, stub OCR)."""
    mp4 = _planted_media(tmp_path, "rec.mp4")
    if mp4 is None:
        pytest.skip("no ffmpeg to plant a frame")
    _plant_engine(monkeypatch, record)
    with pytest.raises(SystemExit) as exc:
        record.run_path_guard([mp4])
    assert exc.value.code == 1
    err = capsys.readouterr().err
    assert "frame 1" in err and "/Users" in err


def test_shots_py_fails_on_a_planted_picture(tmp_path, monkeypatch, capsys):
    """The shot tool's wired-in guard exits non-zero on a planted picture (a real PNG, stub OCR)."""
    png = _planted_png(tmp_path, "shot.png")
    if png is None:
        pytest.skip("no ffmpeg to plant a picture")
    _plant_engine(monkeypatch, shots)
    with pytest.raises(SystemExit) as exc:
        shots.run_path_guard([png])
    assert exc.value.code == 1
    err = capsys.readouterr().err
    assert "/Users" in err and str(png) in err


def test_build_py_fails_on_a_planted_frame(tmp_path, monkeypatch, capsys):
    """The explainer builder's wired-in guard exits non-zero and names the frame (stub OCR)."""
    mp4 = _planted_media(tmp_path, "explainer.mp4")
    if mp4 is None:
        pytest.skip("no ffmpeg to plant a frame")
    _plant_engine(monkeypatch, build)
    with pytest.raises(SystemExit) as exc:
        build.run_path_guard([mp4])
    assert exc.value.code == 1
    err = capsys.readouterr().err
    assert "frame 1" in err and "/Users" in err


def test_a_clean_render_passes_the_wired_guard(tmp_path, monkeypatch):
    """The same wiring passes a clean render (stub OCR reads nothing banned)."""
    mp4 = _planted_media(tmp_path, "clean.mp4")
    if mp4 is None:
        pytest.skip("no ffmpeg to plant a frame")
    monkeypatch.setattr(record.path_guard(), "_engine", lambda: _fake_engine("Drum Spike"))
    record.run_path_guard([mp4])   # no SystemExit
