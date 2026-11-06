"""#343: View > Training Mode. Off by default (no request parameter, no file). Turned on it logs the owner's corrections
(`training: true` with every events_edit and Learn mark) into `<home>/training/<project id>.jsonl`; the status bar reads
"training: N records"; off again, an edit writes nothing more. tools/training_export.py reads the file."""
import json
import subprocess
import sys
from pathlib import Path

import pytest
from playwright.sync_api import expect

from app_support import edits_done, menu, open_project
from helpers import SR
from test_editor import IDX, build, exact_events

ROOT = Path(__file__).resolve().parents[2]


def state(page):
    return page.evaluate("() => DSShell.menuState()")


def log_files(app):
    return sorted((Path(app.home) / "training").glob("*.jsonl"))


def log_lines(app):
    return [json.loads(line) for f in log_files(app) for line in f.read_text().splitlines()]


def project_log_lines(app):
    """The records of the open project's own training file: the training dir keeps other projects'
    and older runs' files, so the glob is not the project's; training_status answers the path."""
    path = Path(app.evaluate("async () => (await B.trainingStatus(S.sid)).path"))
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def type_sample(app, sample):
    smp = app.locator("#ilSample")
    smp.click(); smp.fill(str(sample)); smp.press("Enter")
    edits_done(app)


def test_training_mode_logs_a_move_shows_the_count_and_stops_when_turned_off(app, make_project, tmp_path):
    folder = build(make_project, tmp_path, events={"Kick In": exact_events()})
    open_project(app, folder)
    assert state(app)["trainingMode"] is False                      # off by default
    expect(app.locator("#stTraining")).to_be_hidden()
    assert app.evaluate("() => SETTINGS().ui.training") is False
    before = len(project_log_lines(app))            # the project's own file: the dir keeps other projects' files
    menu(app, "training_mode")                                      # View > Training Mode on
    assert state(app)["trainingMode"] is True
    assert app.evaluate("() => JSON.parse(localStorage.getItem('dsg_settings')).ui.training") is True
    expect(app.locator("#stTraining")).to_have_text("training: 0 records")
    app.keyboard.press("Tab"); app.keyboard.press("Tab")            # the second hit
    type_sample(app, 40000)                                         # the move
    expect(app.locator("#stTraining")).to_have_text("training: 1 records")
    recs = project_log_lines(app)[before:]
    assert len(recs) == 1
    r = recs[0]
    assert (r["op"], r["from"], r["to"], r["role"], r["track"]) == ("move", IDX[1], 40000, "kick", "Kick In")
    assert len(r["audio"]["at_from"]) == len(r["audio"]["at_to"]) == round(0.05 * SR)
    assert r["event"]["v"] == 100 and 0 <= r["confidence"]["c"] <= 1
    # the undo is not a new correction
    app.keyboard.press("Control+z")
    edits_done(app)
    assert len(project_log_lines(app)) == before + 1
    # the tool reads the log: aggregates and the truth list
    out = tmp_path / "truth.json"
    p = subprocess.run([sys.executable, "-I", str(ROOT / "tools" / "training_export.py"), *map(str, log_files(app)), "--out", str(out)],
                       capture_output=True, text=True, check=True)
    assert "kick move:" in p.stdout and "to-from (samples):" in p.stdout
    truth = json.loads(out.read_text())
    assert {"track": "Kick In", "role": "kick", "from": IDX[1], "to": 40000, "op": "move",
            "precision": "ms"} in truth   # the default zoom: a sample far below a pixel
    # off again: the status text goes, a move writes nothing
    menu(app, "training_mode")
    assert state(app)["trainingMode"] is False
    expect(app.locator("#stTraining")).to_be_hidden()
    n = len(project_log_lines(app))
    type_sample(app, 41000)
    assert len(project_log_lines(app)) == n
    assert app.evaluate("() => SETTINGS().ui.training") is False


def test_a_move_at_sample_zoom_records_the_view_and_the_precision(app, make_project, tmp_path):
    """#353 (d): with the mode on, the request carries the view {ms, px_per_sample} and a move at sample zoom (a
    sample is its own pixel or wider) reads back a record with precision: "sample"."""
    folder = build(make_project, tmp_path, events={"Kick In": exact_events()})
    open_project(app, folder)
    menu(app, "training_mode")                                      # View > Training Mode on
    # sample zoom: the span is half the samples the lane's width shows, so one sample is two pixels
    app.evaluate("() => { const sr = S.tracks[\"Kick In\"].sr; S.sel = null; setView(1.5 - 0.5 * laneWidth() / sr, 1.5 + 0.5 * laneWidth() / sr); }")
    v = app.evaluate("() => trainingView(\"Kick In\")")
    assert v["px_per_sample"] >= 1
    app.keyboard.press("Tab"); app.keyboard.press("Tab")            # the second hit (1.5 s)
    type_sample(app, 73000)                                         # the move
    expect(app.locator("#stTraining")).to_have_text("training: 1 records")
    (r,) = project_log_lines(app)
    assert (r["op"], r["from"], r["to"], r["role"], r["track"]) == ("move", IDX[1], 73000, "kick", "Kick In")
    assert r["precision"] == "sample"
    assert r["view"]["ms"] == pytest.approx(v["ms"])
    assert r["view"]["px_per_sample"] == pytest.approx(v["px_per_sample"])
