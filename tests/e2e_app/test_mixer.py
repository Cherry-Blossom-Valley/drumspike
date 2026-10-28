"""P3-16: the mixer of the page on the native transport, through the bridge with the null device (no sound:
DRUMSPIKE_AUDIO=null and DRUMSPIKE_MASTER_GAIN=0). Settings > Audio output (the device list, the choice kept in
the app settings, the fallback message), the track meters, volume and pan (kept in the project), the master
fader and its meter. The bridge is shared by the session: every test puts the output state back."""
import json
import time

import pytest
from playwright.sync_api import expect

from app_support import invoke, load_project, menu, open_project, wait_until

DEVICES = [{"name": "Speakers", "rate": 48000, "channels": 2}, {"name": "AirPods", "rate": 44100, "channels": 2}]


@pytest.fixture
def audio_reset(app):
    """Put the output state of the shared bridge back: no test devices, the system default, master 0 dB, no
    open transport (the playback tests start without one)."""
    # P4-42: an earlier test's page can die and leave the shared bridge's transport open (the null
    # device is then `current`, and "Plays on: null, ..." hides "opens at the next play"); close it.
    app.evaluate("async () => { await B.transportStop(true); }")
    yield
    app.evaluate("() => { if (S.playing) pause(); }")
    app.wait_for_function("() => S.playing === false")
    invoke(app, "transport_stop", close=True)
    invoke(app, "outputs_test_list", devices=None)
    invoke(app, "output_select", device=None)
    invoke(app, "transport_master", volume_db=0.0)


def status(page):
    return page.evaluate("() => B.transportStatus()")


def wait_playing(page, ms=10000):
    page.wait_for_function("() => S.playing === true", timeout=ms)
    page.wait_for_function("() => AC.running === true", timeout=ms)


def settings_json(page):
    return json.loads((page.home / "settings.json").read_text())


def settings_key(page, key, want):
    """The app settings file after the service wrote `key`: output_select sets the choice in memory, answers
    from it, and writes settings.json after that (transport.rs, save_audio_settings), so wait for the file."""
    def read():
        try:
            return settings_json(page).get(key, "<absent>") == want
        except (OSError, ValueError):   # not there yet, or a half-written file (the app writes it atomically)
            return False
    wait_until(read, msg=f"settings.json {key} == {want!r}")
    return settings_json(page)


def test_settings_lists_the_outputs_and_keeps_the_choice(app, audio_reset, make_project):
    invoke(app, "outputs_test_list", devices=DEVICES)
    menu(app, "settings")
    app.wait_for_function("() => document.querySelectorAll('#setOutput option').length === 3")
    opts = app.eval_on_selector_all("#setOutput option", "os => os.map(o => [o.value, o.textContent])")
    assert opts == [["", "System default (Speakers)"], ["Speakers", "Speakers, 48 kHz, 2 ch"],
                    ["AirPods", "AirPods, 44.1 kHz, 2 ch"]]
    assert app.input_value("#setOutput") == ""
    assert "opens at the next play" in app.text_content("#setOutputNow")
    assert app.is_hidden("#setOutputNote")
    # choose AirPods: kept in the app settings
    app.select_option("#setOutput", "AirPods")
    wait_until(lambda: invoke(app, "outputs_list")["choice"] == "AirPods", msg="the choice")
    assert settings_key(app, "audio_output", "AirPods")["audio_output"] == "AirPods"
    expect(app.locator("#stMsg")).to_have_text("audio output: AirPods")   # the message follows the answer (race hunt run 6, x86-mac)
    # the device goes away: the list keeps it as not connected, and the message says the default plays
    invoke(app, "outputs_test_list", devices=DEVICES[:1])
    app.click("#settingsDone")
    menu(app, "settings")
    app.wait_for_function("() => !document.querySelector('#setOutputNote').classList.contains('hidden')")
    assert app.input_value("#setOutput") == "AirPods"
    assert app.eval_on_selector("#setOutput", "s => s.selectedOptions[0].textContent") == "AirPods (not connected)"
    note = app.text_content("#setOutputNote")
    assert '"AirPods" is not connected' in note and "system default" in note
    app.click("#settingsDone")
    # play: the fallback plays (the null device here) and the status line gives the message
    open_project(app, make_project())
    app.click("#playBtn")
    wait_playing(app)
    st = status(app)
    assert st["choice"] == "AirPods" and "not connected" in st["note"]
    assert "not connected" in app.text_content("#stMsg")
    menu(app, "settings")
    app.wait_for_function("() => document.querySelector('#setOutputNow').textContent.startsWith('Plays on: null')")
    # back to the system default: the transport reopens on it, the message goes away
    app.select_option("#setOutput", "")
    app.wait_for_function("() => document.querySelector('#setOutputNote').classList.contains('hidden')")
    assert settings_key(app, "audio_output", None)["audio_output"] is None
    assert status(app)["note"] is None
    app.click("#settingsDone")
    app.click("#playBtn")


def test_a_failed_output_choice_says_why(app, audio_reset):
    menu(app, "settings")
    app.wait_for_function("() => document.querySelectorAll('#setOutput option').length >= 1")
    app.evaluate("() => { B._audioOutput = B.audioOutput; B.audioOutput = () => Promise.reject(new Error('500: boom')); }")
    try:
        app.evaluate("() => chooseAudioOutput('X')")
        app.wait_for_function("() => document.querySelector('#stMsg').textContent === 'audio output failed: boom'")
        app.evaluate("() => { B.audioDevices = () => Promise.reject(new Error('500: gone')); }")
        app.evaluate("() => refreshAudioOutput()")
        app.wait_for_function("() => document.querySelector('#stMsg').textContent.startsWith('audio devices:')")
    finally:
        app.evaluate("() => { B.audioOutput = B._audioOutput; }")
        app.click("#settingsDone")


# ---------- track header: volume, pan, meter; master fader and meter ----------

def drag(page, sel, dx, alt=False):
    """A horizontal drag on a control, from its centre by dx pixels (with Option when alt)."""
    box = page.locator(sel).bounding_box()
    x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    page.mouse.move(x, y)
    if alt:
        page.keyboard.down("Alt")
    page.mouse.down()
    for k in range(1, 5):
        page.mouse.move(x + dx * k / 4, y)
    page.mouse.up()
    if alt:
        page.keyboard.up("Alt")


def doc_track(page, sid, name):
    return invoke(page, "tracks_get", sid=sid)["tracks"][name]


def meter_scale(page, sel):
    """The level of the first bar of a meter (the drawn 1/200 fraction), 0 when none is drawn.
    P4-40h: the bars are painted on one canvas per meter; the drawn fraction lives on the model."""
    return page.evaluate("""s => { const el = document.querySelector(s);
        const row = el && el.closest('.track');
        const m = row && MT.tracks[row.dataset.name];
        return m ? m.drawn[0] : 0; }""", sel)


KICK = '.track[data-name="Kick In"]'


def test_volume_and_pan_controls_are_kept_in_the_project(app, audio_reset, make_project):
    folder = make_project()
    sid = open_project(app, folder)
    # format 1 without the fields: 0 dB and the centre
    assert app.text_content(f"{KICK} [data-for=vol]") == "0.0"
    assert app.text_content(f"{KICK} [data-for=pan]") == "C"   # P3-14c: C, L50, R50
    # a drag to the left lowers the volume; the tooltip shows the value
    drag(app, f"{KICK} .mslider.vol", -30)
    db = app.evaluate("() => S.tracks['Kick In'].volume_db")
    assert db < -3, db
    wait_until(lambda: doc_track(app, sid, "Kick In").get("volume_db") == db, msg="the volume in the project")
    assert app.text_content(f"{KICK} [data-for=vol]") == f"{db:.1f}"
    app.hover(f"{KICK} .mslider.vol")
    assert app.text_content("#mixTip") == f"Volume {db:.1f}"
    # with Option the same drag moves a tenth as far (on the fader position)
    pos = lambda: app.evaluate("() => volPos(S.tracks['Kick In'].volume_db)")
    p0 = pos()
    drag(app, f"{KICK} .mslider.vol", 40)
    p1 = pos()
    drag(app, f"{KICK} .mslider.vol", 40, alt=True)
    p2 = pos()
    coarse, fine = p1 - p0, p2 - p1
    assert coarse > 0.2 and 0 < fine < coarse / 5, (coarse, fine)
    # pan to the right, then Option-click resets it; a double-click resets the volume
    drag(app, f"{KICK} .mslider.pan", 15)
    pan = app.evaluate("() => S.tracks['Kick In'].pan")
    assert pan > 0.3, pan
    assert app.text_content(f"{KICK} [data-for=pan]") == f"R{round(pan * 100)}"
    wait_until(lambda: doc_track(app, sid, "Kick In").get("pan") == pan, msg="the pan in the project")
    app.click(f"{KICK} .mslider.pan", modifiers=["Alt"])
    wait_until(lambda: doc_track(app, sid, "Kick In").get("pan") == 0, msg="pan reset")
    app.dblclick(f"{KICK} .mslider.vol")
    wait_until(lambda: doc_track(app, sid, "Kick In").get("volume_db") == 0, msg="volume reset")
    # a click on a control does not select the track (P4-39c: the first track is selected at open)
    assert app.evaluate("() => S.selTrack") == "Kick In"
    # set, save, open again: the header shows the kept values
    drag(app, f'.track[data-name="Snare Top"] .mslider.pan', -10)
    span = app.evaluate("() => S.tracks['Snare Top'].pan")
    app.evaluate("() => setTrackMix('Snare Top', -12.5, null)")
    wait_until(lambda: doc_track(app, sid, "Snare Top").get("volume_db") == -12.5, msg="the snare volume")
    menu(app, "save")
    doc = load_project(folder).doc
    assert doc["tracks"]["Snare Top"]["volume_db"] == -12.5 and doc["tracks"]["Snare Top"]["pan"] == span
    assert doc["format"] == 1
    assert menu(app, "close") is True   # File > Close Project: the page's own close, then the open
    app.wait_for_function("sid => S.sid !== sid", arg=sid)
    open_project(app, folder)
    assert app.text_content('.track[data-name="Snare Top"] [data-for=vol]') == "-12.5"
    assert app.text_content('.track[data-name="Snare Top"] [data-for=pan]') == f"L{round(-span * 100)}"


def test_reset_volume_and_pan_from_the_track_menu_and_the_header_menu(app, audio_reset, make_project):
    from app_support import track_menu
    sid = open_project(app, make_project())
    app.evaluate("() => { setTrackMix('Kick In', -6, 0.5); S.selTrack = 'Kick In'; }")
    wait_until(lambda: doc_track(app, sid, "Kick In").get("volume_db") == -6, msg="set")
    assert menu(app, "reset_mix") is True
    wait_until(lambda: doc_track(app, sid, "Kick In").get("volume_db") == 0 and doc_track(app, sid, "Kick In")["pan"] == 0,
               msg="Track > Reset Volume and Pan")
    expect(app.locator("#stMsg")).to_have_text("Kick In: volume 0 dB, pan centre")
    app.evaluate("() => { setTrackMix('Kick In', -20, -1); S.selTrack = null; }")
    wait_until(lambda: doc_track(app, sid, "Kick In").get("pan") == -1, msg="set again")
    track_menu(app, "Kick In", "mix")
    wait_until(lambda: doc_track(app, sid, "Kick In").get("pan") == 0, msg="the header menu reset")
    assert menu(app, "reset_mix") is True   # no selected track: the page says so
    assert "track" in app.text_content("#stMsg").lower()


def test_meters_move_while_playing_hold_and_fall_after_the_stop(app, audio_reset, make_project):
    open_project(app, make_project(seconds=6.0))
    assert meter_scale(app, f"{KICK} .meter") == 0
    app.click("#playBtn")
    wait_playing(app)
    # #114: under host load the meter arrival rate collapses (11, 7, 6, 7, 7 per 1 s on
    # x86-mac; 25/s unloaded): the playhead sender thread wakes late (its 20 ms poll sleep
    # stretches) and next_meter_due sends at most one meter per wake (the no-burst rule),
    # so the arrival rate is the wake rate. The bound `15 <= n1 - n0 <= 32` assumed an
    # unloaded host. Wait for 15 messages with a deadline instead of assuming a rate; the
    # runaway bound scales with the window it actually took. A stopped meter path still
    # times out here (planted-fault check, docs/sweeps/sweep-114-meter-arrival.sh).
    n0 = app.evaluate("() => AC.meterN || 0")
    t0 = time.monotonic()
    app.wait_for_function(f"() => (AC.meterN || 0) - {n0} >= 15", timeout=10000)
    n1 = app.evaluate("() => AC.meterN || 0")
    elapsed = max(1.0, time.monotonic() - t0)
    assert n1 - n0 <= 32 * elapsed, ("meter messages runaway", n1 - n0, elapsed)
    # the null device with master gain 0 still gives meter values (the hit at 0.5 s and 1.5 s)
    app.wait_for_function("() => MT.tracks['Kick In'].hold[0] > -12")
    assert app.evaluate("() => MT.master.hold[0]") > -12
    app.wait_for_function("() => MT.master.drawn[2] > 0")   # P4-40h: the hold line is painted on the master canvas
    assert status(app)["master"] == 0.0, "silent"
    # +12 dB on the kick clips its meter; a click on the light clears it
    app.evaluate("() => setTrackMix('Kick In', 12, null)")
    app.wait_for_function(f"() => document.querySelector('{KICK} .mclip').classList.contains('on')", timeout=5000)
    n = app.evaluate("() => FR.n.meter")
    app.click(f"{KICK} .mclip")
    app.wait_for_function(f"() => !document.querySelector('{KICK} .mclip').classList.contains('on')", timeout=2000)
    assert app.evaluate("() => FR.n.meter") > n, "the meters are drawn in the frame (F.METER)"
    app.evaluate("() => setTrackMix('Kick In', 0, null)")
    # stop: no more messages, the levels and the hold lines fall to the floor
    app.click("#playBtn")
    app.wait_for_function("() => S.playing === false")
    app.wait_for_function(f"""() => MT.tracks['Kick In'].drawn.every(v => v === 0) && MT.master.drawn.every(v => v === 0)""",
                          timeout=8000)
    assert meter_scale(app, f"{KICK} .meter") == 0
    # the bars draw as 0 a little before the levels reach the -60 dB floor (the drawn fraction rounds to 1/200), so the
    # timer can run one more 40 ms step after the bars are at 0: poll for its end (one CI failure on a loaded x86 host)
    app.wait_for_function("() => MT.timer === null", timeout=3000)


def test_the_meter_canvas_paints_the_level_of_its_row(app, audio_reset, make_project):
    """P4-40h: the bars and the hold lines live on one canvas per meter. Planted fault: a drawMeter
    that never paints leaves the canvas transparent and the read below fails (alpha 0)."""
    open_project(app, make_project(seconds=6.0))
    app.click("#playBtn")
    wait_playing(app)
    app.wait_for_function("() => MT.tracks['Kick In'].drawn[0] > 0")
    px = app.evaluate(f"""() => {{
        const c = document.querySelector('{KICK} .meter canvas');
        const d = c.getContext('2d').getImageData(Math.floor(c.width / 4), c.height - 2, 1, 1).data;
        return [d[0], d[1], d[2], d[3]];
    }}""")
    assert px[3] == 255, ("the bar bottom is painted", px)
    assert px[1] > px[0] and px[1] > 100, ("the bar bottom is the green of the gradient", px)


def test_the_master_fader_sets_the_master_volume_and_keeps_it(app, audio_reset, make_project):
    open_project(app, make_project())
    assert app.text_content("[data-for=master]") == "0.0"
    drag(app, "#masterVol", -25)
    db = app.evaluate("() => S.masterDb")
    assert db < 0
    wait_until(lambda: status(app)["master_db"] == db, msg="the master volume")
    wait_until(lambda: settings_json(app).get("master_db") == db, msg="kept in the app settings")
    # the master meter follows the master volume: at -inf it stays down while the track meters move
    app.evaluate("() => setMasterDb(-120, true)")
    wait_until(lambda: status(app)["master_db"] == -120, msg="-inf")
    assert app.text_content("[data-for=master]") == "-inf"
    app.click("#playBtn")
    wait_playing(app)
    app.wait_for_function(f"() => MT.tracks['Kick In'].hold[0] > -12")
    assert app.evaluate("() => MT.master.hold[0]") == float("-inf") or app.evaluate("() => MT.master.drawn[2]") == 0
    app.click("#playBtn")
    app.dblclick("#masterVol")
    wait_until(lambda: status(app)["master_db"] == 0, msg="reset to 0 dB")


def test_a_stereo_track_meters_left_and_right_and_its_pan_is_a_balance(app, audio_reset, make_project, tmp_path):
    """P3-16 (controller addition): a stereo file plays as stereo; its meter has two bars and its pan is a balance.
    A mono track's meter has one bar."""
    from helpers import synth_track, write_wav
    x, _ = synth_track([0.5, 1.5, 2.5], 4.0)
    src = tmp_path / "stereo_src"
    src.mkdir()
    specs = [("OH", write_wav(src / "OH.wav", x, channels=2), "cymbal", 49),
             ("Kick In", write_wav(src / "Kick In.wav", x), "kick", 36)]
    folder = make_project.builder.build_project(tmp_path / "stereo_projects", "Stereo Song", specs, "copied")
    open_project(app, folder)
    app.click("#playBtn")
    wait_playing(app)
    assert status(app)["channels"] == [2, 1]
    oh, kick = '.track[data-name="OH"]', KICK
    assert not app.evaluate(f"() => document.querySelector('{oh} .meter').classList.contains('mono')")
    assert app.evaluate(f"() => document.querySelector('{kick} .meter').classList.contains('mono')")
    app.hover(f"{oh} .mslider.pan")
    assert app.text_content("#mixTip").startswith("Balance")
    # balance hard left: the right side of the stereo meter stays at 0, the left side moves
    app.evaluate("() => setTrackMix('OH', null, -1)")
    app.wait_for_timeout(200)
    app.evaluate("() => { const m = MT.tracks['OH']; m.lvl = [-Infinity, -Infinity]; m.hold = [-Infinity, -Infinity]; }")
    app.wait_for_function("() => MT.tracks['OH'].hold[0] > -20", timeout=5000)
    assert app.evaluate("() => MT.tracks['OH'].hold[1]") == float("-inf")
