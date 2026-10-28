"""P4-38b: Learn This on a missed hit adds it and its look-alikes.

K on a range with no hit inserts a hit flagged learned (and the example); the Add rule (a
candidate below the Sensitivity scoring at or over learn.add, 0.85) adds its look-alikes, with the
filter on or off (P4-38d: a K-add never turns the filter on and never takes a detected hit out); the histogram has a draggable Add line with the live count; Remove Example takes the
added hits away; the MIDI export holds the added hit. Every test asserts the new behaviour
directly, so a planted fault (the old refusal, a missing Add line) fails it. Needs the model (the
DSP fallback keeps no candidates)."""
import mido
import pytest

from app_support import inspect_track, open_project
from helpers import synth_track, write_wav
from test_learn_filter import NAME, idle, invoke, js, learn_of, lrf  # noqa: F401  (the fixture)
from test_subtracks import analyse, has_model

BUSY = [0.25 + 0.2 * i for i in range(330)]   # P4-38d: a busy kick track (300+ hits), then silence for the K-add


def doc_events(page, sid):
    """The document events of the fixture's track, as the project stores them."""
    return invoke(page, "events_get", sid=sid, name=NAME)["events"]


def learned_hits(page, sid):
    return [e for e in doc_events(page, sid) if e.get("learned")]


def k_on_empty_range(page):
    js(page, "() => { lnSetSel(%r, 5.0, 5.4); }" % NAME)   # an empty stretch of the lane
    page.keyboard.press("k")
    idle(page)


def test_k_on_an_empty_range_adds_a_learned_hit_and_the_example(lrf):
    page, sid = lrf
    before = len(doc_events(page, sid))
    k_on_empty_range(page)
    now = doc_events(page, sid)
    assert len(now) == before + 1                          # planted fault: the old refusal adds nothing
    added = [e for e in now if e.get("learned")]
    assert len(added) == 1 and 5.0 <= added[0]["t"] <= 5.4
    assert added[0].get("added") is True
    assert len(learn_of(page, sid)["marks"]) == 1          # the example is stored
    assert "added 1 hit" in page.locator("#stMsg").inner_text()


def test_remove_example_takes_the_added_hits_away(lrf):
    page, sid = lrf
    k_on_empty_range(page)
    assert len(learned_hits(page, sid)) >= 1
    js(page, "() => { lnSetSel(%r, 5.0, 5.4); }" % NAME)   # the K cleared the selection: select again
    page.keyboard.press("Alt+K")                           # Remove Example: the sweep drops the adds
    idle(page)
    assert learned_hits(page, sid) == []


def test_the_add_line_is_labelled_and_the_count_falls_with_a_stricter_threshold(lrf):
    page, sid = lrf
    k_on_empty_range(page)
    page.locator("#insParams .lnFilterBox").check()        # the histogram shows with the filter on
    idle(page)
    page.wait_for_function("() => (document.querySelector('#insParams .lnHistInfo') || {textContent:''}).textContent.includes('Add 0.85')")
    info = page.locator("#insParams .lnHistInfo").inner_text()
    assert "adds" in info                                  # planted fault: no Add line -> fails
    loose = js(page, "() => lnAddCount(%r, 0.3)" % NAME)
    strict = js(page, "() => lnAddCount(%r, 0.95)" % NAME)
    assert loose >= strict


def test_the_midi_export_holds_the_added_hit(lrf, tmp_path):
    page, sid = lrf
    k_on_empty_range(page)
    t = learned_hits(page, sid)[0]["t"]
    evs = invoke(page, "events_get", sid=sid, name=NAME)["events"]
    out = invoke(page, "export_midi", sid=sid, name=NAME, events=evs, dest=str(tmp_path))
    path = out.get("path")
    assert path, out
    mid = mido.MidiFile(path)
    on = [msg.time for tr in mid.tracks for msg in tr if msg.type == "note_on" and msg.velocity]
    # the added hit at ~5.0 s is the LATEST hit of the track: the export's last note must be it
    # (the tempo map makes the exact tick non-linear, so no fixed conversion is asserted)
    out_n = [e for e in evs if not e.get("filtered") and not e.get("cut")]   # filtered hits stay out of the file (P4-38d: a K-add leaves none)
    assert len(on) == len(out_n), (len(on), len(out_n))
    assert out_n[-1].get("learned") and on[-1] == max(on) and on[-1] > 2 * min(on), on    # the last note is the added hit (a long delta after the last kept hit); planted fault: no added hit in the file


@pytest.fixture
def busy(app, make_project, tmp_path):
    src = tmp_path / "src_busy"
    src.mkdir()
    x, _ = synth_track(BUSY, BUSY[-1] + 2.5)
    specs = [(NAME, write_wav(src / f"{NAME}.wav", x), "kick", 36, None)]
    sid = open_project(app, make_project.builder.build_project(tmp_path / "projects_busy", "Busy", specs, "copied"))
    analyse(app)
    inspect_track(app, NAME)
    js(app, "() => { if (document.activeElement && document.activeElement.blur) document.activeElement.blur(); }")
    return app, sid


def test_a_k_add_on_a_busy_track_keeps_every_detected_hit_and_the_filter_off(busy):
    # P4-38d: P4-38c found the first K-add turned the filter on and cut a busy kick track from 357 to 108 output hits
    page, sid = busy
    before = doc_events(page, sid)
    assert len(before) >= 300, len(before)
    js(page, "([n, t]) => { lnSetSel(n, t, t + 0.4); }", [NAME, BUSY[-1] + 1.0])
    page.keyboard.press("k")
    idle(page)
    now = doc_events(page, sid)
    assert learn_of(page, sid)["on"] is False              # planted fault: the old switch turns it on
    assert [e["t"] for e in now if e.get("filtered")] == [], "no detected hit goes grey"
    add_rule = [e for e in now if e.get("learned") and not e.get("added")]
    assert len(now) == len(before) + 1 + len(add_rule), (len(before), len(now), len(add_rule))
    times = [e["t"] for e in now]
    assert all(any(abs(t - e["t"]) < 1e-9 for t in times) for e in before), "every detected hit stays"
    text = page.locator("#stMsg").inner_text()
    assert "Learn adds " in text and "Learn filter on" not in text, text
