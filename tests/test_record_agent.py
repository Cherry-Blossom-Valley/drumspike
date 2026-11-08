"""#373: the record.py "agent" step kind: the scenario validation, the page dispatch and the
recording standard's hold rule. Offline: the page is a stub that records its evaluate calls."""
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools" / "media"))
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


def test_the_part_agent_scenario_parses():
    path = Path(__file__).resolve().parents[1] / "tools" / "media" / "scenarios" / "part-agent.json"
    import json as _json
    scenario = record.parse_scenario(_json.loads(path.read_text(encoding="utf-8")))
    ids = [sc["id"] for sc in scenario["scenes"]]
    assert ids == ["ra-mode", "ra-install", "ra-mcp", "ra-look", "ra-move", "ra-confirm",
                   "ra-doubt", "ra-save"]
    for sc in scenario["scenes"]:
        assert record.standard_problems(sc) == [], sc["id"]
    commands = record.keymap_commands(
        (Path(__file__).resolve().parents[1] / "static" / "keymap.js").read_text(encoding="utf-8"))
    assert record.caption_problems(scenario["scenes"], commands) == []
