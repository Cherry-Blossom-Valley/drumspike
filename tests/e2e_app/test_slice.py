"""P3-7a: the M1 slice through the bridge: open, lanes, analyse, edit and undo, export MIDI, an offline
track, an unsupported file, the error answers of the bridge. The page runs the IPC backend of the app
(B.app) over HTTP; the commands are the Rust ones of crates/app."""
import json
import struct
import wave
from pathlib import Path

import httpx
import pytest
from playwright.sync_api import expect

from app_support import edits_done, invoke, menu, midi_reference, open_project, track_menu, wait_until
from helpers import SR


def wait_analysis(page):
    expect(page.locator("#progressText")).to_have_text("analysis done", timeout=60000)


def test_open_draws_lanes_from_bytes(app, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    assert app.evaluate("() => S.order") == ["Kick In", "Snare Top"]
    assert app.evaluate("() => document.querySelectorAll('#lanesScroll .track').length") == 2
    # peaks: the server's shape with real arrays, built from peaks_tile bytes
    # P3-6b: the page keeps the counts of every level and the arrays of the coarsest level; the tiles are in the engine
    shape = app.evaluate("""() => { const p = S.peaks[S.order[0]], L = p.levels[256], C = p.levels[65536];
        return [p.sr, p.n_samples, Object.keys(p.levels), p.tiles === true, L.n > 0 && L.min === undefined, Array.isArray(C.rms) && C.min.length === C.n,
                S.peakAmp[S.order[0]] > 0.1]; }""")
    assert shape[0] == SR and shape[1] == 4 * SR and shape[2] == ["256", "4096", "65536"] and shape[3:] == [True] * 4
    # nothing of the PCM prefetch ran; every call went to /invoke
    assert app.evaluate("() => S.pcm === undefined && typeof prefetchPCM === \"undefined\"")   # P3-12: no PCM store in the page
    assert "project_open" in app.invokes and "peaks_tile" in app.invokes and "pcm" not in app.invokes
    assert app.evaluate("() => B.app") is True
    assert invoke(app, "project_get", sid=sid)["dirty"] is False


def test_lanes_have_drawn_pixels(app, make_project):
    open_project(app, make_project())
    # a wave lane is drawn by WebGL2 under its transparent 2D canvas (P3-6a): laneImage gives both together
    drawn = app.evaluate("""async () => { const out = []; for (const c of document.querySelectorAll('#lanesScroll canvas')) {
        const d = c.classList.contains('wave') ? (await laneImage(c.closest('.track').dataset.name)).data
                                                : c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let n = 0;
        for (let i = 3; i < d.length; i += 4) if (d[i]) n++; out.push(n); } return out; }""")
    assert len(drawn) >= 4 and all(n > 0 for n in drawn), drawn      # a wave and a spike canvas per lane


def test_sample_window_equals_the_wav_samples(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    open_project(app, folder)
    app.evaluate("() => setView(0.49, 0.52)")
    app.wait_for_function("() => Object.keys(S.samplesCache).length > 0")
    # P3-6b: the window is in the engine (a texture); the page keeps its range and no samples
    app.wait_for_function("() => GLQ.eng.textures().samples === 1 && GLQ.eng.stats.modes[1] + GLQ.eng.stats.modes[2] > 0")
    got = app.evaluate("""async () => { const n = S.order[0], c = S.samplesCache[n], r = await B.samples(S.sid, n, c.start, c.start + 2000);
        return {start: c.start, held: 'data' in c, n: c.end - c.start, isF32: r.data instanceof Float32Array, v: Array.from(r.data.slice(0, 2000))}; }""")
    assert got["held"] is False and got["isF32"] is True and got["n"] > 1000
    wav = next(Path(folder, "Audio Files").glob("*.wav"))
    with wave.open(str(wav), "rb") as w:
        w.setpos(got["start"])
        raw = w.readframes(2000)
    ref = [s / 32768 for s in struct.unpack(f"<{len(raw) // 2}h", raw)]
    assert max(abs(a - b) for a, b in zip(got["v"], ref)) < 1e-4
    assert "samples_window" in app.invokes


def test_analyze_all_gives_events_and_the_detector(app, make_project):
    open_project(app, make_project())
    app.click("#analyzeAllBtn")
    wait_analysis(app)
    det = app.evaluate("() => document.querySelector('#stDet').textContent")
    assert det.startswith("detector: Built-in"), det                   # #411: the built-in detector is named like the Inspector's select (the API value stays rust-dsp)
    assert app.evaluate("() => S.events['Kick In'].length") >= 1       # the synthetic kicks; the snare track may stay empty
    sid = app.evaluate("() => S.sid")
    st = invoke(app, "analyze_status", sid=sid)
    assert st["state"] == "done" and all(t["state"] == "done" for t in st["tracks"].values())
    # the page shows what the service holds
    for n in ("Kick In", "Snare Top"):
        held = invoke(app, "events_get", sid=sid, name=n)["events"]
        assert len(held) == app.evaluate("n => S.events[n].length", n)
    assert "analyze_start" in app.invokes and "analyze_status" in app.invokes


def test_channel_progress_reaches_the_page(app, make_project, bridge):
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    sid = open_project(app, folder)
    r = httpx.post(bridge.url + "/invoke/analyze_start", json={"sid": sid, "tracks": ["Kick In"], "progress": {"__channel": "t-chan"}})
    assert r.status_code == 200 and r.json()["started"] == ["Kick In"]
    states = []
    for _ in range(60):
        for m in httpx.get(bridge.url + "/channel/t-chan?timeout_ms=1000", timeout=10).json():
            assert m["sid"] == sid
            states.append(m["job"]["state"])
        if states and states[-1] == "done":
            break
    assert states and states[-1] == "done", states


def test_edit_and_undo_reach_the_service(app, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    app.click("#analyzeAllBtn")
    wait_analysis(app)
    sid = app.evaluate("() => S.sid")
    count = lambda: len(invoke(app, "events_get", sid=sid, name="Kick In")["events"])
    n0 = app.evaluate("() => S.events['Kick In'].length")
    assert count() == n0
    # add a hit with a double click on the spike canvas (the UI path), then undo and redo with the keys
    box = app.locator(".track[data-name='Kick In'] .spike")
    box.dblclick(position={"x": 30, "y": 10})
    edits_done(app)
    assert app.evaluate("() => S.events['Kick In'].length") == n0 + 1
    expect_count(app, count, n0 + 1)
    app.keyboard.press("Control+z")
    edits_done(app)
    assert app.evaluate("() => S.events['Kick In'].length") == n0
    expect_count(app, count, n0)
    app.keyboard.press("Control+y")
    edits_done(app)
    assert app.evaluate("() => S.events['Kick In'].length") == n0 + 1
    expect_count(app, count, n0 + 1)
    assert app.evaluate("() => S.dirty") is True
    assert invoke(app, "project_get", sid=sid)["dirty"] is True
    assert "events_edit" in app.invokes   # P4-9 stage 2: the page edits through events_edit


def expect_count(page, count, want, timeout=5.0):
    import time
    end = time.time() + timeout
    while time.time() < end:
        if count() == want:
            return
        time.sleep(0.1)
    assert count() == want


def test_export_midi_writes_the_file_of_the_engine(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    open_project(app, folder)
    app.click("#analyzeAllBtn")
    wait_analysis(app)
    track_menu(app, "Kick In", "midi")
    expect(app.locator("#stMsg")).to_contain_text("exported 1 MIDI file to Exports/ in the project folder")
    out = Path(folder, "Exports", "Kick In_spike.mid")
    assert out.is_file() and out.read_bytes()[:4] == b"MThd"
    events = app.evaluate("() => S.events['Kick In'].map(e => ({t: e.t, v: e.v}))")
    assert out.read_bytes() == midi_reference("slice/Kick In", events, 36)   # pretty_midi, the server's export (frozen)


def test_offline_track_shows_the_error_and_the_others_work(app, make_project):
    folder = make_project(mode="copied")
    next(Path(folder, "Audio Files").glob("Snare Top*.wav")).unlink()
    open_project(app, folder)
    assert app.evaluate("() => S.tracks['Snare Top'].offline") is True
    assert app.evaluate("() => S.peaks['Snare Top']") is None
    assert app.evaluate("() => S.peakErr['Snare Top']")             # a message in the lane
    assert app.evaluate("() => S.peaks['Kick In'] !== null")
    sid = app.evaluate("() => S.sid")
    r = httpx.post(app.url_base + "/invoke/peaks_info", json={"sid": sid, "name": "Snare Top"})
    assert r.status_code == 409 and r.json()["code"] == 409
    app.click("#analyzeAllBtn")
    wait_analysis_or_failed(app)
    st = invoke(app, "analyze_status", sid=sid)
    assert st["tracks"]["Snare Top"]["state"] == "error" and st["tracks"]["Kick In"]["state"] == "done"
    expect(app.locator("#stMsg")).to_contain_text("1 of 2 tracks failed")


def wait_analysis_or_failed(page):
    expect(page.locator("#progressText")).to_contain_text("analysis done", timeout=60000)


def test_unsupported_sample_rate_gets_the_clear_message(app, make_project):
    folder = make_project(tracks=(("Kick In", "kick", 36),), sr=22050, seconds=2.0)
    open_project(app, folder)
    assert app.evaluate("() => S.peaks['Kick In'] !== null")          # the lane still draws (as in P3-3b)
    app.click("#analyzeAllBtn")
    expect(app.locator("#stMsg")).to_contain_text("1 of 1 tracks failed", timeout=60000)
    text = app.evaluate("() => document.querySelector('#stMsg').textContent")
    assert "44.1, 48 or 96 kHz" in text and "Convert the file in your DAW" in text, text


def test_open_of_a_wrong_folder_shows_the_error(app, tmp_path):
    app.evaluate("f => { window.__DS_PICK__ = f; setDirtyState(false); }", str(tmp_path / "nothing here"))
    menu(app, "open", wait=False)
    expect(app.locator("#stMsg")).to_contain_text("open failed: 404")


def test_add_tracks_dialog_and_consolidate_have_their_commands(app, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    app.evaluate("() => { window.__DS_PICK__ = null; window.__dialogOpts = null; }")
    app.click("#addTracksBtn")   # P3-4c: the native file dialog (several audio files); the user cancels it
    wait_until(lambda: app.evaluate("() => window.__dialogOpts && window.__dialogOpts.multiple"), msg="the file dialog")
    expect(app.locator("#tracksModal")).to_be_hidden()
    menu(app, "consolidate", wait=False)   # P3-4d: the project is copied already, so there is nothing to do
    expect(app.locator("#stMsg")).to_contain_text("nothing to consolidate")


def test_bridge_answers_errors_like_the_server(bridge):
    c = httpx.Client(base_url=bridge.url, timeout=10)
    r = c.post("/invoke/no_such_command", json={})
    assert r.status_code == 404 and r.json()["code"] == 404
    r = c.post("/invoke/project_get", json={"sid": "nope"})
    assert r.status_code == 404 and set(r.json()) == {"code", "message"}
    assert c.post("/invoke/project_get", json={}).status_code == 400
    assert c.post("/invoke/project_list", content=b"{").status_code == 400
    assert c.get("/../Cargo.toml").status_code in (400, 404)
    assert c.get("/%2e%2e/Cargo.toml").status_code in (400, 404)
    assert c.get("/nothing.js").status_code == 404
    page = c.get("/")
    assert page.status_code == 200 and "window.__DS_BRIDGE__ = true" in page.text
    assert "window.__DS_BRIDGE__" not in (Path(__file__).parents[2] / "static" / "index.html").read_text()
