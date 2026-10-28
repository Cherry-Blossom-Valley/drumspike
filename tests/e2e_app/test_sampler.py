"""P4-7c: the sampler calibration in the page on the bridge (silent: DRUMSPIKE_AUDIO=null). The Layer section's
Calibration MIDI... writes one note on a bar line; Load bounce... (and a drop on the zone) measures the delay of the sampler's
transient and its polarity against the close mic; the Export dialog's "Compensate sampler offset" puts each MIDI note at
the aligned attack minus that offset; a layer or sampler set through the agent commands shows without a reload.
Every check runs once more against a planted fault and must fail there."""
import re
from pathlib import Path

import mido
import numpy as np
import pytest
import soundfile as sf
from playwright.sync_api import expect

from app_support import ex_run, export_dialog, invoke, load_project, open_export, save_to, wait_until
from helpers import SR
from test_layer import HITS, layered, one_shot, shot_file  # noqa: F401  (the fixture and the helpers of the layer tests)

NOTE_TIME = 2.0   # the calibration note: bar 2 of the default tempo map


def sec_of(app, pdir):
    from test_layer import show
    return show(app, pdir)


def note_ticks(path):
    out = []
    for tr in mido.MidiFile(str(path)).tracks:
        t = 0
        for m in tr:
            t += m.time
            if m.type == "note_on" and m.velocity > 0:
                out.append((t, m.note, m.velocity))
    return out


def bounce_file(shots, name, delay, sign=1.0):
    """A bounce of the sampler from the session start: the close mic's first hit, `delay` samples after the note at 2.000 s."""
    x, _ = sf.read(shots.parent / "src" / "Kick In.wav", dtype="float32")
    h = int(round(HITS[0] * SR))
    seg = x[h - 240: h + int(0.4 * SR)] * sign
    out = np.zeros(int(4.0 * SR), dtype="float32")
    start = int(NOTE_TIME * SR) + delay - 240
    out[start:start + len(seg)] = seg
    p = shots / name
    p.parent.mkdir(parents=True, exist_ok=True)
    sf.write(p, out, SR, subtype="FLOAT")
    return str(p)


def load_bounce(app, path, via="button"):
    if via == "button":
        app.evaluate("p => { window.__DS_PICK__ = p; }", path)
        app.locator("#inspector .smLoad").click()
    else:
        box = app.locator("#inspector .smZone").bounding_box()
        assert app.evaluate("([p, pos]) => lyDrop([p], pos)", [path, {"x": box["x"] + box["width"] / 2, "y": box["y"] + box["height"] / 2}])
    app.wait_for_function("p => S.tracks['Kick In'].sampler && S.tracks['Kick In'].sampler.file === p", arg=path, timeout=10000)


def sampler(app):
    return app.evaluate("() => S.tracks['Kick In'].sampler")


def test_calibration_midi_writes_one_note_on_a_bar_line_and_says_what_to_do(app, layered):
    pdir, _ = layered
    sec = sec_of(app, pdir)
    expect(sec.locator(".smFile")).to_have_text("no sampler calibration")
    f = Path(pdir) / "Exports" / "Kick In_calibration.mid"
    save_to(app, f)
    sec.locator(".smCal").click()
    app.wait_for_function("() => document.querySelector('#insParams .smInfo').textContent.includes('bounce')")
    expect(app.locator("#stMsg")).to_contain_text("calibration MIDI written")   # the file is there (the info text can say "bounce" before the write)
    assert note_ticks(f) == [(96000, 36, 100)]                       # bar 2 at 120 BPM, 4/4: 2.000 s = tick 96,000 at 24,000 PPQ
    text = sec.locator(".smInfo").inner_text()
    assert "session start" in text and "bounce" in text and "load the bounce" in text
    expect(app.locator("#stMsg")).to_contain_text("calibration MIDI written")
    # planted fault: with a tempo map that puts bar 1 at 1.000 s the note is elsewhere (bar 2 = 1 s + 4 beats of 90 BPM)
    invoke(app, "tempo_map_set", sid=app.evaluate("() => S.sid"), map={"bpm": 90.0, "bar1_s": 1.0})
    save_to(app, f)
    sec.locator(".smCal").click()
    wait_until(lambda: note_ticks(f) != [(96000, 36, 100)], msg="the file with the new map")
    assert note_ticks(f) == [(36000 + 96000, 36, 100)]               # bar 1 at 1 s = tick 36,000 at 90 BPM; one bar is 96,000 ticks


def test_load_bounce_shows_the_offset_and_the_polarity_and_keeps_them(app, layered):
    pdir, shots = layered
    sec = sec_of(app, pdir)
    base_p = bounce_file(shots, "b0.wav", 0)
    load_bounce(app, base_p)
    base = sampler(app)["offset_samples"]
    assert abs(base) < 30                                            # the onset rule's own bias, the same for every delay
    # a planted delay of 7.5 ms (360 samples) is recovered exactly, as the difference to the undelayed bounce
    p = bounce_file(shots, "b360.wav", 360)
    load_bounce(app, p)
    s = sampler(app)
    assert s["offset_samples"] - base == 360 and s["sr"] == SR and s["polarity"] == "normal"
    ms = 1000 * s["offset_samples"] / SR
    expect(sec.locator(".smInfo")).to_have_text(re.compile(r"sampler offset \+\d+\.\d\d ms, polarity normal"))
    assert abs(float(re.search(r"\+([\d.]+) ms", sec.locator(".smInfo").inner_text()).group(1)) - ms) < 0.006
    expect(sec.locator(".smFile")).to_have_text("b360.wav")
    # a drop on the zone does the same; an inverted bounce is called inverted
    inv = bounce_file(shots, "inv.wav", 360, sign=-1.0)
    load_bounce(app, inv, via="drop")
    s = sampler(app)
    assert s["polarity"] == "inverted" and s["offset_samples"] - base == 360
    expect(sec.locator(".smInfo")).to_contain_text("polarity inverted")
    # planted fault: a bounce with another delay gives another offset (the check can fail)
    other = bounce_file(shots, "b361.wav", 361)
    load_bounce(app, other)
    assert sampler(app)["offset_samples"] - base == 361
    with pytest.raises(AssertionError):
        assert sampler(app)["offset_samples"] - base == 360
    # the project keeps it (a save and a read of the document)
    sid = app.evaluate("() => S.sid")
    invoke(app, "project_save", sid=sid)
    saved = load_project(pdir).doc["tracks"]["Kick In"]
    assert saved["sampler"]["offset_samples"] == sampler(app)["offset_samples"] and saved["sampler"]["polarity"] == "normal"
    # Use as one-shot: the bounce becomes the layer, with its polarity
    load_bounce(app, inv, via="button")
    sec.locator(".smShot").click()
    app.wait_for_function("() => S.tracks['Kick In'].layer && S.tracks['Kick In'].layer.source.path.endsWith('inv.wav')", timeout=10000)
    assert app.evaluate("() => S.tracks['Kick In'].layer.polarity") == "invert"
    expect(sec.locator(".lyFile")).to_have_text("inv.wav")
    # Clear removes the calibration
    sec.locator(".smClear").click()
    app.wait_for_function("() => !S.tracks['Kick In'].sampler")
    expect(sec.locator(".smFile")).to_have_text("no sampler calibration")


def exported_ticks(pdir, name="Kick In_spike.mid"):
    return [t for t, _, _ in sorted(note_ticks(Path(pdir) / "Exports" / name))]


def test_export_with_compensation_moves_the_notes_by_the_offset(app, layered):
    pdir, shots = layered
    sec_of(app, pdir)
    plain = [round(t * SR) for t in HITS]
    # no calibration: the checkbox is off and the notes are on the hits
    open_export(app)
    expect(app.locator("#exComp")).not_to_be_checked()
    app.locator("#exSpikes").uncheck()
    ex_run(app)   # 172: the export-finished event (EX.done), not the busy flag
    app.click("#exCancel")
    assert exported_ticks(pdir) == plain
    # a calibration of 400 samples: the checkbox is on by default
    load_bounce(app, bounce_file(shots, "b.wav", 400))
    off = sampler(app)["offset_samples"]
    assert 400 - 30 < off < 400 + 30
    want = [round(p - off) for p in plain]
    export_dialog(app, formats=("midi",), go=False)
    expect(app.locator("#exComp")).to_be_checked()
    ex_run(app)   # 172: the export-finished event (EX.done), not the busy flag
    got = exported_ticks(pdir)
    assert got == want, (got, want)
    assert all(p - g > 300 for p, g in zip(plain, got))              # the notes moved earlier by the offset
    expect(app.locator("#stMsg")).to_contain_text("sampler offset applied to Kick In")
    # planted fault: with the box off the notes are back on the hits (so the check above sees the offset)
    app.locator("#exComp").uncheck()
    ex_run(app)   # 172: the export-finished event (EX.done), not the busy flag
    assert exported_ticks(pdir) == plain
    assert exported_ticks(pdir) != want
    # the multi-track file and the drag files carry the offset too
    app.locator("#exComp").check()
    app.locator("#exModeSingle").check()
    ex_run(app)   # 172: the export-finished event (EX.done), not the busy flag
    multi = next(p for p in (Path(pdir) / "Exports").glob("*_drums.mid"))
    assert sorted(t for t, _, _ in note_ticks(multi)) == want
    app.wait_for_function("() => EX.drag.midi && EX.drag.midi.length === 1", timeout=20000)
    drag = app.evaluate("() => EX.drag.midi[0]")
    assert sorted(t for t, _, _ in note_ticks(drag)) == want
    app.click("#exCancel")


def test_a_layer_and_a_sampler_set_by_the_agent_commands_show_without_a_reload(app, layered):
    pdir, shots = layered
    sec = sec_of(app, pdir)
    sid = app.evaluate("() => S.sid")
    expect(sec.locator(".lyFile")).to_have_text("no one-shot")
    expect(sec.locator(".smFile")).to_have_text("no sampler calibration")
    a = shot_file(shots, "agent.wav")
    r = invoke(app, "track_layer_set", sid=sid, name="Kick In", path=a)
    assert r["layer"]["source"]["path"] == a
    expect(sec.locator(".lyFile")).to_have_text("agent.wav", timeout=8000)
    assert app.evaluate("() => !!S.tracks['Kick In'].layer")
    b = bounce_file(shots, "agent bounce.wav", 240)
    r = invoke(app, "track_sampler_set", sid=sid, name="Kick In", path=b)
    ms = r["offset_ms"]
    expect(sec.locator(".smInfo")).to_have_text(re.compile(r"sampler offset \+\d+\.\d\d ms, polarity normal"), timeout=8000)
    assert abs(float(re.search(r"\+([\d.]+) ms", sec.locator(".smInfo").inner_text()).group(1)) - ms) < 0.006
    expect(sec.locator(".smFile")).to_have_text("agent bounce.wav")
    assert app.evaluate("() => !!S.tracks['Kick In'].sampler")
    # a setting changed by the agent shows too, and a removed layer goes
    invoke(app, "track_layer_set", sid=sid, name="Kick In", level_db=-6.0)
    app.wait_for_function("() => S.tracks['Kick In'].layer.level_db === -6", timeout=8000)
    expect(sec.locator(".lyLevel")).to_have_value("-6")
    invoke(app, "track_layer_set", sid=sid, name="Kick In", path="")
    expect(sec.locator(".lyFile")).to_have_text("no one-shot", timeout=8000)
    invoke(app, "track_sampler_set", sid=sid, name="Kick In", path="")
    expect(sec.locator(".smFile")).to_have_text("no sampler calibration", timeout=8000)
    # planted fault: without the refresh the page would keep showing the old state
    app.evaluate("() => { window.__lyRefresh = lyRefresh; lyRefresh = async () => false; }")
    invoke(app, "track_layer_set", sid=sid, name="Kick In", path=a)
    app.wait_for_timeout(1500)
    expect(sec.locator(".lyFile")).to_have_text("no one-shot")
    app.evaluate("() => { lyRefresh = window.__lyRefresh; }")
    invoke(app, "track_layer_set", sid=sid, name="Kick In", level_db=-3.0)
    expect(sec.locator(".lyFile")).to_have_text("agent.wav", timeout=8000)
