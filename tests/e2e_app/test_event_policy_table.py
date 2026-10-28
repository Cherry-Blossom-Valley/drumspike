"""R16 (#40): the event policy truth table drives the page predicates too. One fixture
(tests/fixtures/event-policy/table.json) drives the Rust test (crates/app/src/policy.rs) and
this e2e test: per row, hgIsCut, isKept, lnGreyHit and the tempo predicate must agree with the
row, so the page's predicates and the Rust policy cannot drift."""
import json
from pathlib import Path

import pytest

from app_support import open_project
from helpers import SR, synth_track, write_wav

ONSETS = [0.5, 1.5, 2.5]
IDX = [int(round(t * SR)) for t in ONSETS]
TABLE = Path(__file__).parents[1] / "fixtures" / "event-policy" / "table.json"


@pytest.fixture()
def policy_app(app, make_project, tmp_path):
    """The app with a plain one-track project open (the predicates read the event object only)."""
    src = tmp_path / "src"
    src.mkdir()
    x, _ = synth_track(ONSETS, 4.0)
    wav = write_wav(src / "Kick In.wav", x)
    builder = make_project.builder
    folder = builder.build_project(tmp_path / "projects_policy", "Policy", [("Kick In", wav, "kick", 36, None)], "copied")
    open_project(app, folder)
    return app


def test_page_predicates_match_the_event_policy_table(policy_app):
    app = policy_app
    fx = json.loads(TABLE.read_text())
    assert len(fx["rows"]) >= 10
    for row in fx["rows"]:
        ev = row["event"]
        assert app.evaluate("e => hgIsCut(e)", ev) == row["out"], row["name"]
        assert app.evaluate("e => isKept(e)", ev) == row["kept"], row["name"]
        assert app.evaluate("e => lnGreyHit('Kick In', e)", ev) == row["grey"], row["name"]
        # the tempo gate is the same one predicate: a hit out of the output is no tempo candidate
        assert app.evaluate("e => !hgIsCut(e)", ev) == row["tempo"], row["name"]
        # audible and exportable are the same gate as `out`
        assert (not row["out"]) == row["audible"] == row["exportable"], row["name"]
