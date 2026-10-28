"""P4-7a and P4-7b: the Inspector's Layer section in the page on the bridge (silent: DRUMSPIKE_AUDIO=null,
master gain 0). A one-shot is chosen (native dialog answered by window.__DS_PICK__) or dropped on the zone
(the shell's drop hook with the drop position); the transport starts a voice at each hit (transport_status
counts them and gives the output frame of the last one); Render writes Exports/<track>_layer.wav, which the
P4-4 checker (tools/check_exports.py --layers) reads independently; polarity and the velocity curve change
the render; Align (P4-7b) measures the per-hit match against the close mic and the info line shows it.
Every check runs once more against a planted fault and must fail there."""
import json
import sys
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf
from playwright.sync_api import expect

from app_support import inspect_track, invoke, open_project, render_layer, shot_dir, wait_until
from helpers import ROOT, SR, synth_track, write_wav

sys.path.insert(0, str(ROOT / "tools"))
import check_exports  # noqa: E402

HITS = [0.5, 1.5, 2.5]


def one_shot(sr=SR, pre_s=0.005, secs=0.15, seed=1):
    n = int(round(secs * sr))
    t = (np.arange(n) - int(round(pre_s * sr))) / sr
    rng = np.random.default_rng(seed)
    x = 0.6 * np.sin(2 * np.pi * 160 * t) * np.exp(-t / 0.04) + 0.4 * rng.uniform(-1, 1, n) * np.exp(-t / 0.003)
    x[t < 0] = 0.0
    return x


def shot_file(folder, name, sr=SR, seed=1):
    p = Path(folder) / name
    p.parent.mkdir(parents=True, exist_ok=True)
    sf.write(p, one_shot(sr, seed=seed), sr, subtype="FLOAT")
    return str(p)


@pytest.fixture
def layered(tmp_path, make_project):
    """A project whose Kick In has hits at 0.5, 1.5 and 2.5 s with velocities 127, 64, 1."""
    x, _ = synth_track(HITS, 4.0)
    (tmp_path / "src").mkdir()
    src = write_wav(tmp_path / "src" / "Kick In.wav", x)
    ev = [{"t": t, "v": v} for t, v in zip(HITS, [127, 64, 1])]
    pdir = make_project.builder.build_project(tmp_path / "projects", "Layered", [("Kick In", str(src), "kick", 36, ev)])
    return Path(pdir), tmp_path / "shots"


def show(app, pdir):
    open_project(app, pdir)
    inspect_track(app, "Kick In")
    return app.locator("#inspector .insSec[data-sec='layer']")


def choose(app, path):
    app.evaluate("p => { window.__DS_PICK__ = p; }", path)
    app.locator("#inspector .lyChoose").click()
    app.wait_for_function("p => S.tracks['Kick In'].layer && S.tracks['Kick In'].layer.source.path === p", arg=path)


def layer_of(app):
    return app.evaluate("() => S.tracks['Kick In'].layer")


def save(app):
    invoke(app, "project_save", sid=app.evaluate("() => S.sid"))


def checked(pdir, align=None):
    rep = check_exports.check_layers(pdir, align)
    return rep["problems"] + [p for r in rep["tracks"].values() for p in r["problems"]], rep


def status(app):
    return app.evaluate("() => B.transportStatus()")


def test_the_layer_is_set_by_choose_and_by_a_drop_on_the_zone(app, layered):
    pdir, shots = layered
    sec = show(app, pdir)
    expect(sec.locator(".lyFile")).to_have_text("no one-shot")
    expect(sec.locator(".lyOn")).to_be_hidden()                       # no settings without a one-shot
    a = shot_file(shots, "kick a.wav")
    choose(app, a)
    expect(sec.locator(".lyFile")).to_have_text("kick a.wav")
    expect(sec.locator(".lyInfo")).to_contain_text("onset")
    expect(sec.locator(".lyInfo")).to_contain_text("48 kHz · mono")
    expect(sec.locator(".lyOn")).to_be_checked()
    lay = layer_of(app)
    assert abs(lay["onset"] - 240) <= 24 and lay["sr"] == SR       # the attack after 5 ms of silence
    assert lay["polarity"] == "normal" and sec.locator(".lyPol").input_value() == "normal"   # fix 4: normal stays the default

    def drop_sets(path, pos):
        before = layer_of(app)["source"]["path"]
        handled = app.evaluate("([p, pos]) => lyDrop([p], pos)", [path, pos])
        assert handled, "the drop was not taken by the Layer zone"
        app.wait_for_function("p => S.tracks['Kick In'].layer.source.path === p", arg=path, timeout=5000)
        assert layer_of(app)["source"]["path"] != before

    b = shot_file(shots, "kick b.wav", seed=2)
    box = sec.locator(".lyZone").bounding_box()
    drop_sets(b, {"x": box["x"] + box["width"] / 2, "y": box["y"] + box["height"] / 2})
    expect(sec.locator(".lyFile")).to_have_text("kick b.wav")
    # planted fault: a drop outside the zone is not the layer's (it goes to Add Tracks)
    with pytest.raises(AssertionError):
        drop_sets(a, {"x": box["x"] - 50, "y": box["y"] - 200})
    assert layer_of(app)["source"]["path"] == b
    # the shell passes the drop position to the hook
    assert app.evaluate("() => DSShell.hooks.otherPaths.length") == 2
    # closed: the summary names the file; Remove takes the layer away
    app.evaluate("() => { INSSEC.open.layer = false; insSecApply(); }")
    expect(sec.locator(".insSum")).to_have_text("kick b.wav")
    app.evaluate("() => { INSSEC.open.layer = true; insSecApply(); }")
    sec.locator(".lyRemove").click()
    app.wait_for_function("() => !S.tracks['Kick In'].layer")
    expect(sec.locator(".lyFile")).to_have_text("no one-shot")


def test_the_layer_plays_silent_with_each_voice_on_its_hit_frame(app, layered):
    pdir, shots = layered
    show(app, pdir)
    choose(app, shot_file(shots, "kick.wav"))
    onset = layer_of(app)["onset"]

    def play_and_check(speed, start, want_voices):
        before = status(app).get("layer_voices", 0) if status(app)["open"] else 0
        app.evaluate("([s, p]) => { S.playhead = p; return appPlay(s); }", [speed, start])
        wait_until(lambda: status(app).get("pos", 0) > 2.6, 15, "the playhead past the last hit")
        st = status(app)
        app.evaluate("() => appPause()")
        app.wait_for_function("() => S.playing === false")
        assert st["layers"] == [{"name": "Kick In", "voices": 3}], st["layers"]
        assert st["layer_voices"] - before == want_voices, st
        # the last voice: the one-shot's first sample on the frame of the hit minus its onset (sample-exact);
        # at half speed the output frames run at twice the device rate
        slow = 1 if speed is None else 2
        assert st["layer_last_start"] == slow * (round(2.5 * st["rate"]) - onset), st
        return st

    try:
        play_and_check(None, 0.0, 3)
        play_and_check(0.5, 2.3, 1)                                   # half speed: the voice of 2.5 s only
        # planted fault: Play off, the transport drops the voices: the check fails
        app.locator("#inspector .lyOn").uncheck()
        app.wait_for_function("() => S.tracks['Kick In'].layer.on === false")
        with pytest.raises(AssertionError):
            play_and_check(None, 2.3, 1)
    finally:
        app.evaluate("() => B.transportStop(true)")                   # later tests start with no transport open


def test_render_puts_the_onset_on_every_hit_sample(app, layered):
    pdir, shots = layered
    show(app, pdir)
    choose(app, shot_file(shots, "kick.wav"))
    render_layer(app, pdir)
    expect(app.locator("#stMsg")).to_contain_text("layer rendered to")
    save(app)
    out = pdir / "Exports" / "Kick In_layer.wav"
    assert out.is_file()
    found, rep = checked(pdir)
    assert found == [] and rep["tracks"]["Kick In"]["layer"]["checked"] == 3
    # planted fault: the render one sample late
    x, sr = sf.read(out, dtype="float32")
    sf.write(out, np.roll(x, 1), sr, subtype="FLOAT")
    assert checked(pdir)[0], "the checker missed a layer one sample late"


def test_polarity_inverts_the_render(app, layered):
    pdir, shots = layered
    show(app, pdir)
    path = shot_file(shots, "kick.wav")
    choose(app, path)
    app.locator("#inspector .lyPol").select_option("invert")
    app.wait_for_function("() => S.tracks['Kick In'].layer.polarity === 'invert'")
    render_layer(app, pdir)
    expect(app.locator("#stMsg")).to_contain_text("layer rendered to")
    save(app)
    found, rep = checked(pdir)
    assert found == [] and rep["tracks"]["Kick In"]["layer"]["sign"] == -1.0
    out, _ = sf.read(pdir / "Exports" / "Kick In_layer.wav", dtype="float32")
    shot, _ = sf.read(path, dtype="float32")
    onset = layer_of(app)["onset"]
    assert out[round(0.5 * SR)] == pytest.approx(-shot[onset], rel=1e-6)   # velocity 127: the level, inverted
    # planted fault: the document says normal, the file is inverted
    doc_path = next(pdir.glob("*.drumspike"))
    doc = json.loads(doc_path.read_text())
    doc["tracks"]["Kick In"]["layer"]["polarity"] = "normal"
    doc_path.write_text(json.dumps(doc))
    assert checked(pdir)[0], "the checker missed the wrong polarity"


def test_the_velocity_curve_sets_the_gain_of_each_hit(app, layered):
    pdir, shots = layered
    show(app, pdir)
    path = shot_file(shots, "kick.wav")
    choose(app, path)
    shot, _ = sf.read(path, dtype="float32")
    onset = layer_of(app)["onset"]
    i64 = round(1.5 * SR)                                              # the hit with velocity 64

    def render():
        app.evaluate("() => { document.querySelector('#stMsg').textContent = ''; }")
        render_layer(app, pdir)
        expect(app.locator("#stMsg")).to_contain_text("layer rendered to")
        save(app)
        return sf.read(pdir / "Exports" / "Kick In_layer.wav", dtype="float32")[0]

    out = render()                                                     # default: dB over 24 dB
    assert out[i64] == pytest.approx(10 ** (-24 * (1 - 64 / 127) / 20) * shot[onset], rel=1e-5)
    assert checked(pdir)[0] == []
    app.locator("#inspector .lyCurve").select_option("linear")
    app.wait_for_function("() => S.tracks['Kick In'].layer.curve.kind === 'linear'")
    expect(app.locator("#inspector .lyRange")).to_be_hidden()
    out = render()
    assert out[i64] == pytest.approx(64 / 127 * shot[onset], rel=1e-5)
    assert checked(pdir)[0] == []
    app.locator("#inspector .lyCurve").select_option("db")
    app.locator("#inspector .lyRange").fill("6")
    app.locator("#inspector .lyRange").press("Tab")
    app.wait_for_function("() => S.tracks['Kick In'].layer.curve.range_db === 6")
    out = render()
    assert out[i64] == pytest.approx(10 ** (-6 * (1 - 64 / 127) / 20) * shot[onset], rel=1e-5)
    assert checked(pdir)[0] == []
    app.locator("#inspector .lyLevel").fill("-6")                     # the level comes before the curve
    app.locator("#inspector .lyLevel").press("Tab")
    app.wait_for_function("() => S.tracks['Kick In'].layer.level_db === -6")
    out = render()
    assert out[i64] == pytest.approx(10 ** (-6 / 20) * 10 ** (-6 * (1 - 64 / 127) / 20) * shot[onset], rel=1e-5)
    assert checked(pdir)[0] == []
    # planted fault: the document's curve is not the one of the file
    doc_path = next(pdir.glob("*.drumspike"))
    doc = json.loads(doc_path.read_text())
    doc["tracks"]["Kick In"]["layer"]["curve"] = {"kind": "fixed"}
    doc_path.write_text(json.dumps(doc))
    assert checked(pdir)[0], "the checker missed the wrong velocity curve"


def test_align_measures_the_match_and_the_info_line_shows_it(app, tmp_path, make_project):
    """P4-7b: a close mic made of the one-shot at planted lags, inverted: Align finds the polarity, the info line
    shows the match of the selected hit, and the aligned render sits at the lags."""
    shots = tmp_path / "shots"
    path = shot_file(shots, "kick.wav", seed=4)
    shot = one_shot(seed=4)
    # the one-shot's onset as the app measures it: a layer on a silent project first
    silent = write_wav(tmp_path / "silent.wav", np.zeros(SR))
    p0 = make_project.builder.build_project(tmp_path / "p0", "Silent", [("S", str(silent), "kick", 36, [])])
    sid0 = invoke(app, "project_open", path=str(p0))["sid"]
    onset = invoke(app, "track_layer_set", sid=sid0, name="S", path=path)["layer"]["onset"]
    invoke(app, "project_close", sid=sid0, force=True)
    lags = [4.0, -9.5, 7.25]
    n = len(shot)
    spec = np.fft.rfft(np.concatenate([shot, np.zeros(n)]))
    f = np.fft.rfftfreq(2 * n)
    mic = np.zeros(4 * SR)
    for t, lag in zip(HITS, lags):
        y = np.fft.irfft(spec * np.exp(-2j * np.pi * f * lag), 2 * n)[:n + 40]
        a = round(t * SR) - onset
        mic[a:a + len(y)] -= 0.7 * y
    mic_path = tmp_path / "mic.wav"
    sf.write(mic_path, mic, SR, subtype="FLOAT")
    ev = [{"t": t, "v": 127} for t in HITS]
    pdir = Path(make_project.builder.build_project(tmp_path / "p1", "Aligned", [("Kick In", str(mic_path), "kick", 36, ev)]))
    show(app, pdir)
    choose(app, path)
    app.locator("#inspector .lyPol").select_option("auto")
    app.wait_for_function("() => S.tracks['Kick In'].layer.polarity === 'auto'")
    app.locator("#inspector .lyAlign").check()
    expect(app.locator("#inspector .lyAlignInfo")).to_contain_text("polarity inverted (3 of 3) · 0 flips")
    # P4-7a fix 4: auto is marked experimental and a warning shows when it picks inverted
    info = app.locator("#inspector .lyAlignInfo")
    expect(info).to_contain_text("auto is experimental")
    expect(info).to_contain_text("WARNING: auto picked inverted")
    assert "lyWarn" in info.get_attribute("class")
    al = app.evaluate("() => LY.align['Kick In']")
    for h, lag in zip(al["hits"], lags):
        assert abs(h["lag"] - lag) < 0.2 and h["match"] > 0.97, h

    def match_shown(index):
        """The info line shows the hit's match; the hit's alignment is the planted lag with a match near 1."""
        app.evaluate("i => { S.sel = { track: 'Kick In', index: i }; updateStatus(); }", index)
        expect(app.locator("#ilMatchF")).to_be_visible()
        v = float(app.locator("#ilMatch").text_content().split()[0])
        h = app.evaluate("i => LY.align['Kick In'].hits[i]", index)
        assert abs(v - h["match"]) < 0.006, (v, h)
        assert v > 0.95 and abs(h["lag"] - lags[index]) < 0.2, f"match {v}, lag {h['lag']} for {lags[index]}"
        return v

    match_shown(1)
    # the aligned render: the checker with the alignment answer
    render_layer(app, pdir)
    expect(app.locator("#stMsg")).to_contain_text("layer rendered to")
    save(app)
    found, rep = checked(pdir, {"Kick In": al})
    assert found == [] and rep["tracks"]["Kick In"]["layer"]["checked"] == 3
    # planted fault: polarity normal against an inverted mic: the match of the hit is low, the check fails
    app.locator("#inspector .lyPol").select_option("normal")
    app.wait_for_function("() => S.tracks['Kick In'].layer.polarity === 'normal'")
    app.locator("#inspector .lyMeasure").click()
    app.wait_for_function("() => LY.align['Kick In'] && LY.align['Kick In'].sign === 1")
    # planted fault of the warning: normal polarity picks nothing, so no warning (the warning follows `auto` only)
    assert "WARNING" not in info.text_content() and "lyWarn" not in info.get_attribute("class")
    with pytest.raises(AssertionError):
        match_shown(1)                                                 # the as-is sign: another lag, a lower match


def test_a_missing_layer_file_shows_the_relink_bar_and_find_moved_audio_relinks_it(app, layered):
    """P4-7a fix 6: the layer file is part of the relink flow: the bar offers it, one search finds the moved file."""
    pdir, shots = layered
    show(app, pdir)
    expect(app.locator("#relinkBar")).to_be_hidden()                  # nothing is offline yet
    path = shot_file(shots, "kick.wav", seed=2)
    choose(app, path)
    expect(app.locator("#relinkBar")).to_be_hidden()
    moved = Path(shots) / "elsewhere" / "deep"
    moved.mkdir(parents=True)
    (Path(path)).rename(moved / "kick.wav")
    app.evaluate("() => loadProject()")
    app.wait_for_function("() => S.tracks['Kick In'].layer_status === 'missing'")
    expect(app.locator("#relinkBar")).to_be_visible()
    app.evaluate("f => { window.__DS_PICK__ = f; }", str(Path(shots) / "elsewhere"))
    app.click("#relinkFind")
    expect(app.locator("#relinkSummary")).to_contain_text("Found 1 of 1 offline track")
    app.click("#relinkClose")
    app.wait_for_function("() => S.tracks['Kick In'].layer_status === 'ok'")
    expect(app.locator("#relinkBar")).to_be_hidden()
    assert app.evaluate("() => B.layerGet(S.sid, 'Kick In')")["status"] == "ok"


def layers_of_transport(app):
    return {x["name"]: x["voices"] for x in status(app).get("layers", [])}


def test_a_layer_changed_through_the_agent_route_shows_in_the_open_page_without_a_reload(app, layered):
    """P4-33: the service sends track-layer after every layer change; the page shows it with the Layer section closed
    (summary), then open (file, level), and the playing transport follows (voices). No poll."""
    pdir, shots = layered
    sec = show(app, pdir)
    sid = app.evaluate("() => S.sid")
    a = shot_file(shots, "agent a.wav")
    app.evaluate("() => { INSSEC.open.layer = false; insSecApply(); }")    # closed: the old poll ran only while it was open
    try:
        app.evaluate("() => { S.playhead = 0; return appPlay(); }")
        wait_until(lambda: status(app).get("open"), 15, "the transport open")
        assert layers_of_transport(app) == {}

        def agent(**kw):
            r = invoke(app, "track_layer_set", sid=sid, name="Kick In", **kw)
            assert "error" not in r, r
            return r

        agent(path=a)
        expect(sec.locator(".insSum")).to_have_text("agent a.wav", timeout=5000)
        wait_until(lambda: layers_of_transport(app) == {"Kick In": 3}, 5, "three live voices")
        agent(level_db=-6.0)
        expect(sec.locator(".insSum")).to_have_text("agent a.wav \u00b7 -6 dB", timeout=5000)
        agent(polarity="invert", on=False)
        app.wait_for_function("() => S.tracks['Kick In'].layer.polarity === 'invert' && S.tracks['Kick In'].layer.on === false", timeout=5000)
        wait_until(lambda: layers_of_transport(app) == {}, 5, "no voices while the layer is off")
        agent(on=True, curve={"kind": "linear"}, align=True)
        app.wait_for_function("() => S.tracks['Kick In'].layer.align === true && S.tracks['Kick In'].layer.curve.kind === 'linear'", timeout=5000)
        wait_until(lambda: layers_of_transport(app) == {"Kick In": 3}, 5, "the voices back")
        # open: the Inspector shows the file and the level
        app.evaluate("() => { INSSEC.open.layer = true; insSecApply(); }")
        expect(sec.locator(".lyFile")).to_have_text("agent a.wav")
        expect(sec.locator(".lyLevel")).to_have_value("-6")
        # a change while it is open shows too, and Remove by the agent clears page and transport
        agent(level_db=-9.0)
        expect(sec.locator(".lyLevel")).to_have_value("-9", timeout=5000)
        agent(path="")
        expect(sec.locator(".lyFile")).to_have_text("no one-shot", timeout=5000)
        wait_until(lambda: layers_of_transport(app) == {}, 5, "no voices after Remove")
        # planted fault: without the page's refresh the summary stays on the old state
        app.evaluate("() => { INSSEC.open.layer = false; insSecApply(); window.__lyRefresh = lyRefresh; lyRefresh = async () => false; }")
        agent(path=a)
        app.wait_for_timeout(1500)
        with pytest.raises(AssertionError):
            expect(sec.locator(".insSum")).to_have_text("agent a.wav", timeout=500)
        app.evaluate("() => { lyRefresh = window.__lyRefresh; }")
    finally:
        app.evaluate("() => B.transportStop(true)")


READY = "typeof S !== 'undefined' && typeof B !== 'undefined' && B.app === true && S.loaded > 0"


def test_the_no_sampler_calibration_label_is_not_clipped_at_the_default_inspector_width(app, make_project):
    """#243 (gate 2): at the default Inspector width (320 px) the label "no sampler calibration"
    (.smFile in the .smZone of the Layer section) is not clipped, at 1x and at 2x; screenshots of
    the zone go to the shot dir for the coordinator."""
    def check(page, pdir, shot):
        open_project(page, pdir)
        inspect_track(page, "Kick In")
        zone = page.locator("#inspector .smZone")
        expect(zone).to_be_visible()
        got = page.evaluate("""() => { const f = document.querySelector('#inspector .smFile');
            return { text: f.textContent, sw: f.scrollWidth, cw: f.clientWidth }; }""")
        assert got["text"] == "no sampler calibration", got
        assert got["sw"] <= got["cw"], f"the label is clipped: {got}"
        zone.screenshot(path=str(shot))

    pdir2 = make_project()   # the 2x page first, with its own project (its own bridge session)
    ctx = app.context.browser.new_context(device_scale_factor=2)
    try:
        p2 = ctx.new_page()
        p2_errors = []
        p2.on("pageerror", lambda e: p2_errors.append(str(e)))
        p2.goto(app.url_base + "/")
        p2.wait_for_function(READY, timeout=120000)
        check(p2, pdir2, shot_dir("243-smzone-2x.png"))
        assert p2_errors == [], p2_errors
    finally:
        ctx.close()
    check(app, make_project(), shot_dir("243-smzone-1x.png"))
