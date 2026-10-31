"""#242: Hits > Toggle Hit Active (X): the selected hit flips between active and deactivated.

The middle of three hits is deactivated by the loudness cut. Pressing the key makes it active:
no "deactivated" in the status line (#stSel) and the exported MIDI carries it again. Pressing
again deactivates it with the user's own mark (`deactivated: true`) and the export drops it.
Undo takes each step back (the flags the hit had come back, the cut mark included). The choice
survives a re-analyze (the kept hit of the service's merge) and a change of the cut threshold
(the marking rule leaves a kept hit alone, while the other hits are marked). Every condition is
asserted false before the action that turns it.
"""
import math
from pathlib import Path

import mido
import pytest

from app_support import edits_done, invoke, open_project, track_menu, wait_until
from test_menus import with_hits

CUT_T = 2.0


def song(app, make_project):
    """(sid, folder): three hits at 1, 2, 3 s; the middle one deactivated by the loudness cut."""
    folder = make_project(seconds=6.0)
    sid = open_project(app, folder)
    with_hits(app, sid, {"Kick In": [1.0, CUT_T, 3.0]})
    assert app.evaluate(
        """async (t) => {
          const evs = (S.events['Kick In'] || []).map(e => ({ t: e.t, v: e.v, id: e.id }));
          if (evs.length !== 3) return false;
          const hit = evs.find(e => Math.abs(e.t - t) < 1e-9);
          if (!hit) return false;
          hit.cut = true;
          await B.setEvents(S.sid, 'Kick In', { events: evs });
          await loadProject();
          return true;
        }""",
        CUT_T,
    )
    return sid, folder


@pytest.fixture()
def cut_song(app, make_project):
    return (app,) + song(app, make_project)


def select_cut_hit(app):
    app.evaluate(
        "t => { const i = (S.events['Kick In'] || []).findIndex(e => Math.abs(e.t - t) < 1e-9);"
        " S.sel = { track: 'Kick In', index: i }; updateStatus(); }",
        CUT_T,
    )
    app.wait_for_timeout(150)


def stored_hit(app, sid):
    evs = invoke(app, "events_get", sid=sid, name="Kick In")["events"]
    return next(e for e in evs if abs(e["t"] - CUT_T) < 1e-9)


def exported_times(app, folder):
    """The hit times of the track's exported MIDI: the service's export list, written out."""
    track_menu(app, "Kick In", "midi")
    mid = mido.MidiFile(str(Path(folder) / "Exports" / "Kick In_spike.mid"))
    out, t = [], 0.0
    for msg in mid:
        t += msg.time
        if msg.type == "note_on" and msg.velocity > 0:
            out.append(round(t, 6))
    return out


def press(app):
    app.keyboard.press("x")
    edits_done(app)


def test_the_key_makes_a_deactivated_hit_active_and_back(cut_song):
    app, sid, folder = cut_song
    select_cut_hit(app)
    assert "deactivated" in app.locator("#stSel").text_content()
    assert CUT_T not in exported_times(app, folder)   # false before the key: the export has no 2.0
    press(app)
    assert "deactivated" not in app.locator("#stSel").text_content()
    hit = stored_hit(app, sid)
    assert hit.get("keep") is True and "deactivated" not in hit and "cut" not in hit
    wait_until(lambda: CUT_T in exported_times(app, folder), msg="the export carries the kept hit")
    press(app)
    assert "deactivated" in app.locator("#stSel").text_content()
    hit = stored_hit(app, sid)
    assert hit.get("deactivated") is True and "keep" not in hit
    assert CUT_T not in exported_times(app, folder)


def test_undo_takes_each_toggle_back(cut_song):
    app, sid, folder = cut_song
    select_cut_hit(app)
    press(app)
    press(app)
    hit = stored_hit(app, sid)
    assert hit.get("deactivated") is True
    app.evaluate("() => undo()")
    edits_done(app)
    hit = stored_hit(app, sid)
    assert hit.get("keep") is True and "deactivated" not in hit   # the first step: active again
    select_cut_hit(app)
    assert "deactivated" not in app.locator("#stSel").text_content()
    app.evaluate("() => undo()")
    edits_done(app)
    hit = stored_hit(app, sid)
    assert "keep" not in hit and "deactivated" not in hit and hit.get("cut") is True   # the original cut state
    select_cut_hit(app)
    assert "deactivated" in app.locator("#stSel").text_content()


def test_the_choice_survives_a_re_analyze(cut_song):
    app, sid, folder = cut_song
    select_cut_hit(app)
    press(app)   # the cut hit becomes active (keep: true)
    assert stored_hit(app, sid).get("keep") is True
    app.evaluate("() => runAnalysis(['Kick In'], true)")
    app.wait_for_function("() => !analysisRunning", timeout=120000)
    wait_until(
        lambda: (stored_hit(app, sid).get("keep") is True and "deactivated" not in stored_hit(app, sid)),
        timeout=30000,
        msg="the kept hit survives the re-analyze",
    )
    select_cut_hit(app)
    assert "deactivated" not in app.locator("#stSel").text_content()
    assert CUT_T in exported_times(app, folder)


def test_the_choice_survives_a_cut_threshold_change(cut_song):
    app, sid, folder = cut_song
    select_cut_hit(app)
    press(app)   # the cut hit becomes active (keep: true)
    assert stored_hit(app, sid).get("keep") is True
    levels = [l for l in invoke(app, "track_histograms", sid=sid, name="Kick In")["loudness"]["levels"] if math.isfinite(l)]
    assert levels
    # a cut at 0 dBFS marks every hit of the track; the kept hit must come back without one
    r = invoke(app, "track_velocity_set", sid=sid, name="Kick In", lo_db=-40.0, hi_db=-6.0, cut_db=0.0)
    evs = r["events"]
    kept = next(e for e in evs if abs(e["t"] - CUT_T) < 1e-9)
    assert kept.get("keep") is True and "cut" not in kept, "the marking rule leaves a kept hit alone"
    others = [e for e in evs if abs(e["t"] - CUT_T) >= 1e-9]
    assert others and all(e.get("cut") is True for e in others), "the threshold marked the other hits"
    assert CUT_T in exported_times(app, folder)
