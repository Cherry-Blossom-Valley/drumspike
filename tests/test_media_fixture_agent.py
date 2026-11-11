"""Agent fine-tune fixture (#380): one real agent run on the demo session, the tutorial's transcript.

The gate: the prompt is not empty; the transcript names the skill (drumspike-finetune), has a
move call and the final counts table (Hits, Moved, Confirmed, Hand-placed, Not looked at); the word
"model" does not appear in any case (the recorder's visible-text guard fails on it); the
actions have a move of two samples or more and a confirm; every move row's track, index, from
and to are in the transcript.

The run is the 0.9.33 one (#482): the transcript has a handPlaced call and a hand-placed count of
1 or more in the report; actions.jsonl holds one hit left for the user (`left`) with a reason that
starts with "two opinions differ"; neither prompt nor transcript says "demo". One tool call was
denied in that run (see HOW.md), so no check counts denials.
"""
import json
import re

from helpers import ROOT

FIX = ROOT / "tools" / "media" / "fixtures" / "agent-run"


def _text(name):
    return (FIX / name).read_text(encoding="utf-8")


def _actions():
    return [json.loads(line) for line in _text("actions.jsonl").splitlines() if line.strip()]


def test_prompt_not_empty():
    assert _text("prompt.txt").strip()


def test_transcript_shape():
    t = _text("transcript.txt")
    assert "drumspike-finetune" in t
    assert re.search(r"^> agent_call move ", t, re.M)
    for word in ("Hits", "Moved", "Confirmed", "Hand-placed", "Not looked at"):
        assert word in t, word


def test_transcript_has_no_model_word():
    for name in ("prompt.txt", "transcript.txt"):
        assert "model" not in _text(name).lower(), name


def test_actions_have_far_move_and_confirm():
    rows = _actions()
    assert any(r["fn"] == "move" and abs(r["to"] - r["from"]) >= 2 for r in rows)
    assert any(r["fn"] == "confirm" for r in rows)


def test_transcript_has_hand_placed_call_and_count():
    t = _text("transcript.txt")
    assert re.search(r"^> agent_call handPlaced ", t, re.M)
    row = re.search(r"^\s*\|\s*\*\*Total\*\*\s*\|(.*)\|\s*$", t, re.M)
    assert row, "no Total row in the report table"
    head = re.search(r"^\s*\|\s*Track\s*\|(.*)\|\s*$", t, re.M).group(1)
    column = [c.strip() for c in head.split("|")].index("Hand-placed")
    assert int([c.strip() for c in row.group(1).split("|")][column].strip("*")) >= 1


def test_one_hit_is_left_with_two_opinions():
    left = [r for r in _actions() if r["fn"] == "left"]
    assert any(r["reason"].startswith("two opinions differ") for r in left), left


def test_no_demo_word():
    for name in ("prompt.txt", "transcript.txt"):
        assert "demo" not in _text(name).lower(), name


def test_every_move_is_in_the_transcript():
    t = _text("transcript.txt")
    moves = [r for r in _actions() if r["fn"] == "move"]
    assert moves
    for r in moves:
        line = f"{r['track']} {r['index']} "
        assert line in t, r
        assert f"moved {r['from']} to {r['to']}" in t, r
