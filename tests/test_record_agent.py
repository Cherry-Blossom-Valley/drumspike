"""#373: the record.py "agent" step kind: the scenario validation, the page dispatch and the
recording standard's hold rule. Offline: the page is a stub that records its evaluate calls."""
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools" / "media"))
import check_paths  # noqa: E402
import record  # noqa: E402


class StubPage:
    """A page stub: evaluate returns a canned value for the nearest-hit probe and the lane x,
    a bounding box for the lane locator, and records every (js, arg) pair."""

    def __init__(self, nearest=7):
        self.nearest = nearest
        self.calls = []

    def evaluate(self, js, arg=None):
        self.calls.append((js, arg))
        if js == record.NEAREST_HIT_JS:
            return self.nearest
        if js == record.LANE_X_JS:
            return 400.0
        return None

    def locator(self, sel):
        return self

    def scroll_into_view_if_needed(self):
        pass

    def bounding_box(self):
        return {"x": 0, "y": 0, "width": 100, "height": 20}

    def wait_for_function(self, js, timeout=0, arg=None):
        self.calls.append((js, arg))

    def wait_for_timeout(self, ms):
        pass


def scene(steps):
    return {"id": "ra-test", "steps": steps}


def test_agent_step_dispatches_the_helper_with_the_resolved_index():
    pg = StubPage(nearest=5)
    record.run_step(pg, {"do": "agent", "fn": "move", "track": "s_top", "t": 3.125, "args": [1]}, {})
    js, arg = pg.calls[-1]
    assert js == record.AGENT_JS and arg == ["move", ["s_top", 5, 1]]


def test_agent_step_without_a_hit_names_the_hit_only_through_args():
    pg = StubPage()
    record.run_step(pg, {"do": "agent", "fn": "save", "args": []}, {})
    js, arg = pg.calls[-1]
    assert js == record.AGENT_JS and arg == ["save", []]


def test_agent_step_glints_the_pointer_at_the_hit_lane_first():
    """The pointer glide and ring come before the helper call (the recording standard)."""
    pg = StubPage()
    record.run_step(pg, {"do": "agent", "fn": "look", "track": "s_top", "t": 4.375}, {})
    kinds = [js for js, _ in pg.calls]
    assert kinds.index(record.GLIDE_JS) < kinds.index(record.AGENT_JS)
    assert record.RING_JS in kinds


def test_parse_scenario_accepts_the_agent_step_and_refuses_an_unknown_helper():
    ok = {"scenes": [scene([{"do": "agent", "fn": "confirm"}])]}
    assert record.parse_scenario(ok)["scenes"] == ok["scenes"]
    bad = {"scenes": [scene([{"do": "agent", "fn": "delete_all"}])]}
    with pytest.raises(record.RecordError, match="an agent step needs fn"):
        record.parse_scenario(bad)
    with pytest.raises(record.RecordError, match="args are a list"):
        record.parse_scenario({"scenes": [scene([{"do": "agent", "fn": "save", "args": "x"}])]})


def test_the_standard_needs_a_hold_after_an_agent_step():
    held = {"scenes": [scene([{"do": "agent", "fn": "move", "track": "s_top", "t": 3.125, "args": [1]},
                              {"do": "hold", "s": 2.0}])]}
    assert record.standard_problems(record.parse_scenario(held)["scenes"][0]) == []
    bare = {"scenes": [scene([{"do": "agent", "fn": "move", "track": "s_top", "t": 3.125, "args": [1]}])]}
    problems = record.standard_problems(record.parse_scenario(bare)["scenes"][0])
    assert problems and "no hold of 1.5 s" in problems[0]


def test_show_text_keeps_the_neutral_text_against_app_rewrites():
    """The app re-asserts its own text (agentSkillShow writes the real binary path into
    #setSkillCmd after every status refresh), so show_text installs an observer that
    re-asserts the shown text, like show_dest's exRender hook."""
    pg = StubPage()
    record.run_step(pg, {"do": "show_text", "sel": "#setSkillCmd", "text": "neutral"}, {})
    js, arg = pg.calls[-1]
    assert js == record.SHOW_TEXT_JS and arg == ["#setSkillCmd", "neutral"]
    assert "MutationObserver" in record.SHOW_TEXT_JS
    assert "textContent" in record.SHOW_TEXT_JS


def test_the_mcp_line_is_neutral_from_the_apps_first_write():
    """The pre-seed (NEUTRAL_JS) runs before the scenes, so agentSkillFill's write on the
    Settings sheet's first open (seconds before the show_text step) is neutralized too."""
    import json as _json
    path = Path(__file__).resolve().parents[1] / "tools" / "media" / "scenarios" / "part-agent.json"
    scenes = record.parse_scenario(_json.loads(path.read_text(encoding="utf-8")))["scenes"]
    neutral = record.neutral_texts(scenes)
    assert neutral == {"#setSkillCmd": 'claude mcp add --scope user drumspike -- "<the Drum Spike binary>" mcp'}
    # the pre-seed keeps the shown text against app writes and marks the observer in, so the
    # later show_text step adds no second one
    assert "agentSkillShow" in record.NEUTRAL_JS and "MutationObserver" in record.NEUTRAL_JS
    assert "__dsTextKeep' + sel" in record.NEUTRAL_JS
    # only the mcp line is pre-seeded: the status element keeps the app's real states
    assert list(record.neutral_texts([{"id": "s", "steps": [
        {"do": "show_text", "sel": "#setSkillStatus", "text": "installed"}]}])) == []


def test_the_part_agent_shows_no_host_path():
    """The scenario's shown texts are neutral: no absolute host path in any show_text or
    caption of part-agent.json (the path guard OCRs every frame of the render)."""
    import json as _json
    path = Path(__file__).resolve().parents[1] / "tools" / "media" / "scenarios" / "part-agent.json"
    raw = path.read_text(encoding="utf-8")
    banned = [w for w in check_paths.BANNED_BASE if w.startswith("/")]
    assert banned and not any(w in raw for w in banned)
    scenario = _json.loads(raw)
    for sc in scenario["scenes"]:
        for st in sc["steps"]:
            shown = st.get("text", "")
            assert not any(w.lower() in shown.lower() for w in banned), st


def test_the_part_agent_scenario_parses():
    path = Path(__file__).resolve().parents[1] / "tools" / "media" / "scenarios" / "part-agent.json"
    import json as _json
    scenario = record.parse_scenario(_json.loads(path.read_text(encoding="utf-8")))
    ids = [sc["id"] for sc in scenario["scenes"]]
    assert ids == ["ra-prompt", "ra-skill", "ra-setup", "ra-look", "ra-move", "ra-confirm",
                   "ra-doubt", "ra-hand", "ra-opinions", "ra-save", "ra-table"]
    for sc in scenario["scenes"]:
        assert record.standard_problems(sc) == [], sc["id"]
    commands = record.keymap_commands(
        (Path(__file__).resolve().parents[1] / "static" / "keymap.js").read_text(encoding="utf-8"))
    assert record.caption_problems(scenario["scenes"], commands) == []


# ---------------- the #381 re-take: prompt first, one visible move, no Training Mode ----------------

def _part_agent_scenarios():
    import json as _json
    path = (Path(__file__).resolve().parents[1] / "tools" / "media" / "scenarios"
            / "part-agent.json")
    return record.parse_scenario(_json.loads(path.read_text(encoding="utf-8")))["scenes"]


def test_the_part_agent_has_no_training_mode_topic():
    """#381 gate (a): Training Mode is not a topic of the tutorial: no menu training_mode
    step, no caption with Training, no step on the training count. The agent's skill turns it
    on as its own step 2 (the silent set_training plumbing), narration says so once."""
    import json as _json
    for sc in _part_agent_scenarios():
        for st in sc["steps"]:
            assert st.get("id") != "training_mode", st
            if st["do"] == "caption":
                assert "training" not in st["text"].lower(), st
            assert "#stTraining" not in _json.dumps(st), st
    # the plumbing is there and silent: setTraining(true) through the page's own function
    assert "setTraining(true)" in record.SET_TRAINING_JS


def test_the_part_agent_opens_with_the_prompt_and_the_skill():
    """The user's prompt and the skill load open the tutorial, typed from the #380 fixture
    files (not inline), and the set up is one short scene."""
    scenes = _part_agent_scenarios()
    assert scenes[0]["id"] == "ra-prompt" and scenes[1]["id"] == "ra-skill"
    for sc in (scenes[0], scenes[1]):
        terms = [st for st in sc["steps"] if st["do"] == "terminal"]
        assert terms, sc["id"]
        for st in terms:
            assert st["file"].startswith("tools/media/fixtures/agent-run/"), st
            assert Path(st["file"]).name in ("prompt.txt", "transcript.txt")
            assert "text" not in st and "equals" not in st   # no inline agent text
    assert [sc["id"] for sc in scenes if sc["id"] == "ra-setup"] == ["ra-setup"]


def test_the_part_agent_pane_wraps_and_leaves_the_six_scenes():
    """#381 send-back round 2: the pane wraps its lines (the full prompt is visible in the
    ra-prompt frame), ra-setup clears the pane and keeps it off through ra-look, ra-move,
    ra-confirm, ra-doubt and ra-save, and ra-table starts on a cleared pane."""
    scenes = _part_agent_scenarios()
    # the prompt wraps, so the whole 178-character line is in the frame
    assert "pre-wrap" in record.TERMINAL_JS and "overflowWrap" in record.TERMINAL_JS
    # the last typed line stays visible: the pane trims from the top at max-height
    assert "scrollHeight > pane.clientHeight" in record.TERMINAL_JS
    by_id = {sc["id"]: sc for sc in scenes}
    setup = by_id["ra-setup"]["steps"]
    assert setup[1] == {"do": "terminal",
                        "file": "tools/media/fixtures/agent-run/transcript.txt",
                        "clear": True}, setup[:2]
    assert setup[2] == {"do": "hold", "s": 1.5}   # the recording standard's hold after the clear
    for sid in ("ra-look", "ra-move", "ra-confirm", "ra-doubt", "ra-save"):
        terms = [st for st in by_id[sid]["steps"]
                 if st["do"] == "terminal" and st.get("clear") is not True]
        assert not terms, (sid, terms)
    table_terms = [st for st in by_id["ra-table"]["steps"] if st["do"] == "terminal"]
    assert table_terms[0]["lines"] == [83, 84] and table_terms[1]["lines"] == [86, 90]
    assert table_terms[2]["lines"] == [110, 110]   # the export question closes the report
    # the move lands on the spoken words: about 10 s of hold before it in the 26 s scene
    move_steps = by_id["ra-move"]["steps"]
    pre = [st["s"] for st in move_steps if st["do"] == "hold"][0]
    assert pre >= 8.0, move_steps


def _fixture_left_row():
    import json as _json
    path = (Path(__file__).resolve().parents[1] / "tools" / "media" / "fixtures"
            / "agent-run" / "actions.jsonl")
    rows = [_json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]
    return next(row for row in rows if row.get("fn") == "left")


def test_the_part_agent_shows_the_hand_placed_hit_untouched():
    """#483 gate: a scene of its own for the hand-placed hit: the agent looks at it and the
    report line from the fixture (transcript line 96) names it; the scenario never moves or
    confirms a hit at the hand-placed time. Red on the pre-#483 scenario (no such scene)."""
    scenes = _part_agent_scenarios()
    by_id = {sc["id"]: sc for sc in scenes}
    sc = by_id["ra-hand"]
    looks = [st for st in sc["steps"] if st["do"] == "agent" and st.get("fn") == "look"]
    assert looks and looks[0]["track"] == "k_in" and abs(looks[0]["t"] - 2.4997) < 0.001, sc
    terms = [st for st in sc["steps"] if st["do"] == "terminal"]
    assert terms and terms[0]["file"].endswith("transcript.txt") and terms[0]["lines"] == [96, 96], sc
    assert record.standard_problems(sc) == [], sc["id"]
    for other in scenes:
        for st in other["steps"]:
            if st["do"] == "agent" and st.get("fn") in ("move", "confirm"):
                assert abs(st.get("t", 0.0) - 2.4997) > 0.01, (other["id"], st)


def test_the_part_agent_shows_the_two_opinions_hit_and_its_reason():
    """#483 gate: a scene of its own for the hit where the two opinions differ: the agent
    looks at the s_top hit at 1.875083 s and the reason (the fixture's left row, transcript
    lines 102-103) is readable on screen for 2 s or more. Red on the pre-#483 scenario."""
    scenes = _part_agent_scenarios()
    by_id = {sc["id"]: sc for sc in scenes}
    sc = by_id["ra-opinions"]
    looks = [st for st in sc["steps"] if st["do"] == "agent" and st.get("fn") == "look"]
    assert looks and looks[0]["track"] == "s_top" and abs(looks[0]["t"] - 1.875083) < 0.001, sc
    terms = [st for st in sc["steps"] if st["do"] == "terminal"]
    assert terms and terms[0]["lines"] == [102, 103], sc
    ti = next(i for i, st in enumerate(sc["steps"]) if st["do"] == "terminal")
    after = next(st["s"] for st in sc["steps"][ti + 1:] if st["do"] == "hold")
    assert after >= 2.0, sc["steps"]
    # the on-screen reason is the fixture's left row: two opinions differ
    assert _fixture_left_row()["track"] == "s_top" and _fixture_left_row()["index"] == 0
    assert _fixture_left_row()["reason"].startswith("two opinions differ")
    assert record.standard_problems(sc) == [], sc["id"]


def test_the_part_agent_doubt_caption_carries_the_runs_reason():
    """#483: the doubt scene's reason is the 0.9.33 run's (14 dB under the median, a noisy
    line), not the old plan row's why."""
    by_id = {sc["id"]: sc for sc in _part_agent_scenarios()}
    caption = next(st["text"] for st in by_id["ra-doubt"]["steps"] if st["do"] == "caption")
    assert "14 dB under the median" in caption and "noisy line" in caption, caption
    assert "no clear line before the hit" not in caption


def test_terminal_step_types_the_fixture_file_not_inline_text(tmp_path, monkeypatch):
    """#381 gate (c): the terminal pane text is the fixture transcript, read from the file at
    the run: the step carries the path and the 1-based lines, never the text itself."""
    (tmp_path / "fixtures" / "agent-run").mkdir(parents=True)
    fx = tmp_path / "fixtures" / "agent-run" / "transcript.txt"
    fx.write_text("first line\nsecond line\nthird line\n", encoding="utf-8")
    monkeypatch.setattr(record, "ROOT", tmp_path)
    pg = StubPage()
    record.run_step(pg, {"do": "terminal", "file": "fixtures/agent-run/transcript.txt",
                         "lines": [2, 3], "title": "claude -p"}, {})
    typed = [arg for js, arg in pg.calls if js == record.TERMINAL_JS]
    assert typed == [["claude -p", "second line"], ["claude -p", "third line"]]
    # the fixture is read from the file, so a later fixture edit changes the pane: the same
    # step typed against an edited file types the edited line
    fx.write_text("first line\nedited line\nthird line\n", encoding="utf-8")
    pg2 = StubPage()
    record.run_step(pg2, {"do": "terminal", "file": "fixtures/agent-run/transcript.txt",
                          "lines": [2, 2]}, {})
    assert [arg for js, arg in pg2.calls if js == record.TERMINAL_JS] == [[None, "edited line"]]
    # a missing fixture is a clear FAIL, never a placeholder
    with pytest.raises(record.RecordError, match="the fixture is missing"):
        record.run_step(StubPage(), {"do": "terminal",
                                     "file": "fixtures/agent-run/absent.txt",
                                     "lines": [1, 1]}, {})
    # out-of-range lines FAIL with the count
    with pytest.raises(record.RecordError, match="the fixture has 3 lines"):
        record.run_step(pg, {"do": "terminal", "file": "fixtures/agent-run/transcript.txt",
                             "lines": [1, 9]}, {})


def test_terminal_clear_and_parse_rules():
    pg = StubPage()
    record.run_step(pg, {"do": "terminal", "clear": True}, {})
    assert pg.calls[-1][0] == record.TERMINAL_CLEAR_JS
    with pytest.raises(record.RecordError, match="needs file and lines"):
        record.parse_scenario({"scenes": [scene([{"do": "terminal", "text": "invented"}])]})
    with pytest.raises(record.RecordError, match="lines are \\[first, last\\]"):
        record.parse_scenario({"scenes": [scene([{"do": "terminal", "file": "f.txt",
                                                  "lines": [5, 2]}])]})


def _view(pps, start=100.0, mx=400.0):
    return {"px_per_sample": pps, "window_start": start, "marker_x": mx}


def test_move_view_problems_holds_the_sample_zoom_gate():
    """#381 gate (b): the same px_per_sample (>= 2) and window start over the move, the marker
    shifts by exactly N samples within 1 px."""
    good0, good1 = _view(3.0, mx=400.0), _view(3.0, mx=406.0)
    assert record.move_view_problems(good0, good1, 2) == []
    # the window moved: a re-zoom between the shots
    assert "the window moved" in record.move_view_problems(good0, _view(3.0, start=99.0, mx=406.0), 2)[0]
    # not sample zoom
    assert "not sample zoom" in record.move_view_problems(_view(0.4), _view(0.4, mx=400.8), 2)[0]
    # the marker did not step N samples
    assert "expected 2 samples" in record.move_view_problems(good0, _view(3.0, mx=401.0), 2)[0]
    # no view data (the test stubs): silent
    assert record.move_view_problems(None, None, 2) == []


def _frame(width=60, height=20, marker=None):
    import numpy as _np
    a = _np.full((height, width, 3), 30, _np.uint8)
    if marker is not None:
        a[:, marker] = 200
    return a


def test_move_diff_problems_want_the_marker_columns_only():
    """The frame diff of the move scene changes pixels only in the marker columns: the old and
    the new stalk of the spike and the wave lane. A wide run, a spread beyond the marker or a
    many-column diff is a visible defect."""
    before, after = _frame(marker=10), _frame(marker=16)
    assert record.move_diff_problems(before, after) == []
    # the stalks smear over 5+ px: not a clean step
    smear = _frame()
    smear[:, 10:17] = 200
    assert any("column run" in p for p in record.move_diff_problems(before, smear))
    # a stray change elsewhere (the status bar or a re-render): the diff spreads beyond the
    # marker's columns
    wide = _frame(marker=16)
    wide[:, 40] = 220
    assert any("spreads" in p for p in record.move_diff_problems(before, wide, max_span_px=10))
    # the app's own selection annotation (the top band of the clip, wave.js) is not a defect
    noted = _frame(marker=16)
    noted[:4, 30:40] = 200
    assert record.move_diff_problems(before, noted, mask_top=4) == []
    # no change at all: the move is not visible
    assert record.move_diff_problems(before, before) == ["no pixel changed: the move is not visible"]


def test_the_move_scene_is_at_sample_zoom_and_holds_each_side():
    """The move scene zooms to sample zoom (a 10 ms span: px_per_sample well over 2 on the
    1920 viewport) and holds 2 s or more on each side of the move."""
    scenes = _part_agent_scenarios()
    moves = record.move_scenes(scenes)
    assert [sc["id"] for sc in moves] == ["ra-move"]
    steps = moves[0]["steps"]
    zoom = next(st for st in steps if st["do"] == "zoom_to")
    assert zoom["span"] <= 0.02   # 10 ms: 480 samples over a >960 px lane -> px_per_sample >= 2
    mi = next(i for i, st in enumerate(steps)
              if st["do"] == "agent" and st.get("fn") == "move")
    before = next(st["s"] for st in steps[:mi] if st["do"] == "hold")
    after = next(st["s"] for st in steps[mi + 1:] if st["do"] == "hold")
    assert before >= 2.0 and after >= 2.0
    assert abs(steps[mi]["args"][-1]) >= 2
    # the recorder enforces the gate on the live page: the view data before and after the move
    assert "px_per_sample" in record.VIEW_DATA_JS and "tToX" in record.HIT_X_JS
