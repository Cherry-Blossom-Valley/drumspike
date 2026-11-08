"""#371: window.DSAgent, the page helpers of an agent's hit-by-hit fine-tune pass (static/agent.js). Every move and confirm
goes through the page's own path, so the training record is the one a person's nudge writes; with the mode off every
helper that changes the document refuses."""
import json
import shutil
from pathlib import Path

import numpy as np
import pytest
from playwright.sync_api import expect

from app_support import edits_done, menu, open_project
from helpers import SR, synth_track, write_wav
from test_editor import IDX, ONSETS, build, exact_events
from test_training_mode import project_log_lines

TRACK = "Kick In"


def call(app, js, *args):
    return app.evaluate(f"async (a) => {{ const r = await ({js}); return r === undefined ? null : r; }}", list(args))


def training_on(app):
    menu(app, "training_mode")
    assert app.evaluate("() => DSShell.menuState().trainingMode") is True


def test_training_mode_turns_on_from_the_page_globals(app, make_project, tmp_path):
    """#370: the shipped page mode has no native menu and the bridge's `training_mode` is debug-only:
    the agent turns the mode on with the page's own globals and the helpers follow."""
    opened(app, make_project, tmp_path)
    assert call(app, "DSAgent.status()")["training"] is False
    app.evaluate("() => setTraining(true)")
    st = call(app, "DSAgent.status()")
    assert st["training"] is True
    call(app, "DSAgent.confirm('Kick In', 0)")   # the writing helpers accept it
    assert app.evaluate("() => DSShell.menuState().trainingMode") is True


def opened(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path, events={TRACK: exact_events()}))


def test_hits_lists_the_hits_with_a_confidence_when_the_mode_is_on(app, make_project, tmp_path):
    opened(app, make_project, tmp_path)
    plain = call(app, "DSAgent.hits('Kick In')")
    assert [h["index"] for h in plain] == [0, 1, 2] and [h["sample"] for h in plain] == IDX
    assert all("confidence" not in h for h in plain)                       # the review rating is a Training Mode field
    training_on(app)
    hits = call(app, "DSAgent.hits('Kick In')")
    assert [h["sample"] for h in hits] == IDX and hits[1]["v"] == 100
    for h in hits:
        assert 0 <= h["confidence"] <= 1 and isinstance(h["why"], list)
        assert {"added", "edited", "flam", "bleed"} <= set(h)


def test_look_returns_the_samples_with_the_hit_marked_and_the_view_of_the_record(app, make_project, tmp_path):
    opened(app, make_project, tmp_path)
    r = call(app, "DSAgent.look('Kick In', 1)")
    assert r["sample"] == IDX[1] and r["start"] + r["hit_at"] == IDX[1]
    assert 0 <= r["hit_at"] < len(r["samples"]) and max(abs(v) for v in r["samples"]) == pytest.approx(1.0)
    assert r["judge"]["suggest"] is None or isinstance(r["judge"]["suggest"], int)
    assert r["trainingView"]["px_per_sample"] >= 1
    assert app.evaluate("() => S.sel.index") == 1
    w = call(app, "DSAgent.wide('Kick In', 1)")
    assert w["trainingView"]["ms"] == pytest.approx(20) and len(w["samples"]) > len(r["samples"])


def test_move_writes_one_move_record_at_sample_precision_and_undo_writes_none(app, make_project, tmp_path):
    opened(app, make_project, tmp_path)
    training_on(app)
    before = len(project_log_lines(app))
    out = call(app, "DSAgent.move('Kick In', 1, 2)")
    edits_done(app)
    assert out["moved"] and out["to"] - out["from"] == 2
    (r,) = project_log_lines(app)[before:]
    assert (r["op"], r["to"] - r["from"], r["precision"]) == ("move", 2, "sample")
    assert r["by"] == "agent", r   # #389: a nudge through DSAgent.move says who made it
    assert call(app, "DSAgent.undo()")["undone"] is True
    edits_done(app)
    assert len(project_log_lines(app)) == before + 1
    assert app.evaluate("() => S.events['Kick In'].map(e => Math.round(e.t * S.tracks['Kick In'].sr))") == IDX


def test_the_keyboard_nudge_records_person_and_the_agent_move_records_agent(app, make_project, tmp_path):
    """#389: the same nudge through the two paths of the page: nudgeSelected (the keyboard path) writes by
    person, DSAgent.move writes by agent."""
    opened(app, make_project, tmp_path)
    training_on(app)
    before = len(project_log_lines(app))
    app.evaluate("() => { setSelTrack('Kick In'); S.sel = { track: 'Kick In', index: 1 }; }")
    app.evaluate("() => nudgeSelected(2)")   # the keyboard path: the hit selected, the nudge by samples
    edits_done(app)
    (r,) = project_log_lines(app)[before:]
    assert (r["op"], r["by"]) == ("move", "person"), r
    out = call(app, "DSAgent.move('Kick In', 1, -1)")
    edits_done(app)
    assert out["moved"] is True
    (r,) = project_log_lines(app)[before + 1:]
    assert (r["op"], r["by"]) == ("move", "agent"), r


def test_two_overlapping_agent_moves_both_record_agent(app, make_project, tmp_path):
    """#389: the agent author is a depth counter, not a flag: when two helper calls overlap, the first one to
    end must not reset the author while the second's edit is still queued - both records say agent."""
    opened(app, make_project, tmp_path)
    training_on(app)
    before = len(project_log_lines(app))
    p1 = call(app, "DSAgent.move('Kick In', 1, 2)")
    p2 = call(app, "DSAgent.move('Kick In', 2, 2)")
    assert p1["moved"] is True and p2["moved"] is True
    edits_done(app)
    recs = project_log_lines(app)[before:]
    assert [r["op"] for r in recs] == ["move", "move"], recs
    assert all(r["by"] == "agent" for r in recs), recs


def test_confirm_writes_one_confirm_record_and_changes_no_event(app, make_project, tmp_path):
    opened(app, make_project, tmp_path)
    training_on(app)
    events = app.evaluate("() => S.events['Kick In'].map(e => e.t)")
    before = len(project_log_lines(app))
    assert call(app, "DSAgent.confirm('Kick In', 0)")["confirmed"] is True
    edits_done(app)
    (r,) = project_log_lines(app)[before:]
    assert (r["op"], r["from"], r["to"]) == ("confirm", IDX[0], IDX[0])
    assert app.evaluate("() => S.events['Kick In'].map(e => e.t)") == events


def test_with_the_mode_off_every_writing_helper_refuses_and_the_log_stays(app, make_project, tmp_path):
    opened(app, make_project, tmp_path)
    before = len(project_log_lines(app))
    for js in ("DSAgent.move('Kick In', 1, 2)", "DSAgent.confirm('Kick In', 0)", "DSAgent.undo()",
               "DSAgent.apply('Kick In', [{index: 1, samples: 2}])"):
        msg = app.evaluate(f"async () => {{ try {{ await {js}; return ''; }} catch (e) {{ return e.message; }} }}")
        assert "Training Mode is off" in msg and "setTraining(true)" in msg, js
    assert len(project_log_lines(app)) == before
    assert call(app, "DSAgent.status()")["training"] is False
    assert app.evaluate("() => S.events['Kick In'].map(e => Math.round(e.t * S.tracks['Kick In'].sr))") == IDX


def test_plan_gives_one_row_per_hit_and_marks_the_hit_on_the_noisy_line_doubtful(app, make_project, tmp_path):
    x, _ = synth_track(ONSETS, 4.0)
    rng = np.random.default_rng(7)
    a, b = int(2.4 * SR), IDX[2]
    x[a:b] += rng.uniform(-0.06, 0.06, b - a)                              # a line that is not a line before the third hit
    src = tmp_path / "noisy"
    src.mkdir()
    wav = write_wav(src / f"{TRACK}.wav", x)
    open_project(app, make_project.builder.build_project(tmp_path / "noisy_projects", "Noisy", [(TRACK, wav, "kick", 36, exact_events())], "copied"))
    training_on(app)
    rows = call(app, "DSAgent.plan('Kick In', 0, 2)")
    assert [r["index"] for r in rows] == [0, 1, 2]
    assert all("suggest" in r for r in rows)
    assert rows[2]["line_ok"] is False and rows[2]["suggest"] is None and rows[2]["why"]
    assert all(isinstance(r["queue_doubt"], bool) and isinstance(r["queue_why"], list) for r in rows)
    assert rows[0]["suggest"] is not None and rows[1]["suggest"] is not None
    before = len(project_log_lines(app))
    assert len(project_log_lines(app)) == before                           # a plan moves nothing
    done = call(app, "DSAgent.apply('Kick In', a[0])", [{"index": 2, "samples": 0}, {"index": 1, "samples": -1}])
    edits_done(app)
    assert [d["moved"] for d in done] == [False, True]
    assert len(project_log_lines(app)) == before + 1
    st = call(app, "DSAgent.status()")
    assert st["training"] is True and st["records"] >= 1


def test_render_answers_a_non_empty_picture_of_the_view(app, make_project, tmp_path):
    """#409: render is a picture of the view as the user sees it: the sample-zoom window of the hit, the hit marked."""
    opened(app, make_project, tmp_path)
    r = call(app, "DSAgent.render('Kick In', 1)")
    assert r["sample"] == IDX[1] and r["index"] == 1 and r["track"] == TRACK
    assert r["format"] == "image/png" and r["height"] == 320 and r["width"] > 0
    assert r["image"].startswith("data:image/png;base64,") and len(r["image"]) > 1000
    assert app.evaluate("() => S.sel.index") == 1
    # the wave is drawn as the lane draws it: a signed line, not rectified bars. At a clearly negative
    # sample the wave colour sits below the middle line and nowhere well above it (a rectified picture
    # would mirror the bar above the line).
    sides = app.evaluate("""async () => {
      // the kick's first half-cycle is positive, so the pixel check opens a wider window that holds the
      // descending half of the sweep
      const r = await DSAgent.render('Kick In', 1, { after_ms: 6 });
      const l = await DSAgent.look('Kick In', 1, { after_ms: 6 });
      if (!l.samples.some(s => s < -0.05)) return { negative: false };
      const img = new Image(); img.src = r.image; await img.decode();
      const c = document.createElement('canvas'); c.width = r.width; c.height = r.height;
      const ctx = c.getContext('2d'); ctx.drawImage(img, 0, 0);
      const d = ctx.getImageData(0, 0, c.width, c.height).data;
      const wave = i => Math.abs(d[i] - 138) <= 30 && Math.abs(d[i + 1] - 148) <= 30 && Math.abs(d[i + 2] - 166) <= 30;
      const mid = c.height / 2, above = [], below = [];
      for (let i = 0; i < l.samples.length; i++) {
        if (l.samples[i] >= -0.05) continue;
        for (const x of [i * r.width / l.samples.length, Math.min(c.width - 1, i + 1)]) {
          for (let y = 0; y < c.height; y++) if (wave(((y * c.width) + Math.round(x)) * 4)) (y < mid - 2 ? above : below).push(y);
        }
      }
      return { negative: true, above: above.length, below: below.length };
    }""")
    assert sides["negative"] is True and sides["below"] > 0 and sides["above"] == 0


def test_backups_names_the_backup_that_holds_the_state_before_the_session(app, make_project, tmp_path):
    """#409: a backup is named with the time of the save that replaced it but holds the state BEFORE that save;
    backups() names the one that holds the state before this session (the session's first save wrote it)."""
    folder = build(make_project, tmp_path, events={TRACK: exact_events()})
    open_project(app, folder)
    assert call(app, "DSAgent.backups()")["before"] is None   # no save of this session yet
    training_on(app)
    call(app, "DSAgent.move('Kick In', 1, 2)")
    edits_done(app)
    assert call(app, "DSAgent.save()")["saved"] is True
    b = call(app, "DSAgent.backups()")
    assert b["rule"] and b["before"] and b["before"].endswith(".drumspike") and b["before"].startswith("Song")
    assert b["before"] in [p.name for p in (folder / "Backups").iterdir()]
    # the named backup holds the state before the session: the hit still at its old place
    doc = json.loads((folder / "Backups" / b["before"]).read_text())
    samples = [round(e["t"] * SR) for e in doc["tracks"]["Kick In"]["events"]]
    assert samples == IDX


def test_settings_row_installs_the_bundled_skill_and_shows_the_version_and_the_mcp_line(app, tmp_path):
    """#371 (h): the Settings row reads "not installed", Install copies the bundled folder to the target (a temp folder
    through the bridge's environment), bakes the binary path into SKILL.md, and the row then names the version; the Copy
    button puts the one `claude mcp add` line on the clipboard."""
    target = Path(app.evaluate("async () => (await B.agentSkillStatus()).path"))
    shutil.rmtree(target.parent, ignore_errors=True)
    app.evaluate("() => openSettings()")
    app.click("#setCat-agent")   # #387: the Agent category
    expect(app.locator("#setSkillStatus")).to_have_text("not installed")
    expect(app.locator("#setSkillInstall")).to_have_text("Install")
    line = app.locator("#setSkillCmd").text_content()
    assert line.startswith('claude mcp add --scope user drumspike -- "') and line.endswith('" mcp')
    hint = app.locator("#setAgentSkill .phint").text_content()
    assert "Keep the Drum Spike window open with the song in it" in hint and "Quit" not in hint, hint   # #388
    assert "The line is per user" in hint, hint
    app.locator("#setSkillInstall").click()
    version = app.evaluate("async () => (await B.agentSkillStatus()).bundled_version")
    expect(app.locator("#setSkillStatus")).to_contain_text(f"installed {version}")
    skill = (target / "SKILL.md").read_text()
    binary = line.split('"')[1]
    assert f'"{binary}" bridge' in skill and "<binary>" not in skill and f"version: {version}" in skill
    assert (target / "README.md").is_file()
    app.evaluate("() => { window.__copied = null; Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async t => { window.__copied = t; } } }); }")
    app.locator("#setSkillCopy").click()
    app.wait_for_function("() => window.__copied !== null")
    assert app.evaluate("() => window.__copied") == line
    # an older copy reads as such and the button says Update
    (target / "SKILL.md").write_text(skill.replace(f"version: {version}", "version: 0.0.1"))
    app.evaluate("() => agentSkillFill()")
    expect(app.locator("#setSkillStatus")).to_contain_text("older copy installed (0.0.1)")
    expect(app.locator("#setSkillInstall")).to_have_text("Update")
    app.locator("#setSkillInstall").click()
    expect(app.locator("#setSkillStatus")).to_contain_text(f"installed {version}")
    expect(app.locator("#setSkillInstall")).to_have_text("Install")
    shutil.rmtree(target.parent, ignore_errors=True)
