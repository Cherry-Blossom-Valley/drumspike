"""Agent fine-tune fixture (#380): one real agent run on the demo session, the tutorial's transcript.

The gate: the prompt is not empty; the transcript names the skill (drumspike-finetune), has a
move call and the final counts table (Hits reviewed, Moved, Confirmed, Doubtful); the word
"model" does not appear in any case (the recorder's visible-text guard fails on it); the
actions have a move of two samples or more and a confirm; every move row's track, index, from
and to are in the transcript.
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
    for word in ("Hits reviewed", "Moved", "Confirmed", "Doubtful"):
        assert word in t, word


def test_transcript_has_no_model_word():
    for name in ("prompt.txt", "transcript.txt"):
        assert "model" not in _text(name).lower(), name


def test_actions_have_far_move_and_confirm():
    rows = _actions()
    assert any(r["fn"] == "move" and abs(r["to"] - r["from"]) >= 2 for r in rows)
    assert any(r["fn"] == "confirm" for r in rows)


def test_every_move_is_in_the_transcript():
    t = _text("transcript.txt")
    moves = [r for r in _actions() if r["fn"] == "move"]
    assert moves
    for r in moves:
        line = f"{r['track']} {r['index']} "
        assert line in t, r
        assert f"moved {r['from']} to {r['to']}" in t, r
