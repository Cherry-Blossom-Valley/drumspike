"""#244 (Media 3): the pictures of the user guide, from the demo session through the test bridge.

    uv run --project . python tools/media/shots.py --chapters 1-5

Writes docs/guide/img/<name>.png for every entry of docs/guide/shots.json whose chapter is in the
range. Chapters 2 to 5 are app screens: ds-bridge (the debug binary of crates/app) serves the page,
Playwright drives it at 1600x1000 css pixels, 2x device scale. Chapter 1 pictures are stills of the
explainer 01 render (#246, tools/media/explainers/build.py), not of the app. The 3b pictures (#394)
use the example plugin (tests/fixtures/plugins/json_hits.py), installed into the shot home's
Detectors folder.

Chapters 6 to 13 (#497) are a second session on the same bridge (guide_pictures): the zoom and the
nudge, the velocity lane, the review queue, the bleed panel, a follower with a layer, the tempo rulers.

Needs the demo session (tools/media/demo_session.py --out ~/.cache/drumspike/media/demo) and the
debug bridge (cargo build -p drumspike-devtools --bin ds-bridge). Before every screenshot a guard
reads the status bar, the dialogs and every input value from the DOM and FAILs on "model", "/Users/", "x86-mac", "/var/folders", "/tmp/" or "/private/" (reviews 6396240075 and 6403628912); it prints the count of checked shots and FAILs on 0.
FAIL (exit 1) on a picture that cannot be made, a picture over 400 KB, or a listed picture of the
range that is missing.
"""
import argparse
import importlib.util
import json
import math
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GUIDE = ROOT / "docs" / "guide"
SHOTS = GUIDE / "shots.json"
DEMO = Path.home() / ".cache" / "drumspike" / "media" / "demo"
FRAMES = Path.home() / ".cache" / "drumspike" / "media" / "explainers" / "frames" / "explainer-01-why-one-ms"
BRIDGE = Path(os.environ.get("DRUMSPIKE_BRIDGE") or ROOT / "target" / "debug" / "ds-bridge")
MAX_BYTES = 400 * 1024
READY = "() => typeof S !== 'undefined' && typeof B !== 'undefined' && B.app === true && S.loaded > 0"
# chapter 1: the explainer 01 still that each picture shows (the late copy; the aligned first motion)
EXPLAINER_STILLS = {"img/01-why-sample-exact.png": "s03-sum.png", "img/01-attack-aligned.png": "s07-first-motion.png"}
# the demo session's role of each mic (demo_session.ROLE) and a MIDI note per role
ROLE = {"k in": "kick", "k out": "kick", "s top": "snare", "s bot": "snare", "t1": "tom", "t2": "tom",
        "t3": "tom", "hh": "hat", "ride": "cymbal", "oh l": "cymbal", "oh r": "cymbal", "room": "other",
        "new take": "kick"}
NOTE = {"kick": 36, "snare": 38, "tom": 45, "hat": 42, "cymbal": 51, "other": 60}
SNARE = "s top"
# #394: the example plugin, installed into the shot home's Detectors folder for the 3b pictures
PLUGIN_ID = "json-hits"
PLUGIN_FIXTURE = ROOT / "tests" / "fixtures" / "plugins"
# the export folder the guide shows: a real folder in the user's Music, shown as "~/Music/..." in
# the picture (no host user name, no /Users/ in any screenshot)
EXPORTS = Path.home() / "Music" / "Drum Spike" / "Exports"
EXPORTS_SHOWN = "~/Music/Drum Spike/Exports"
# the guard: text a screenshot must never show (review 6396240075)
FORBIDDEN = ("model", "/Users/", "x86-mac", "/var/folders", "/tmp/", "/private/")


def chapters(text):
    """'1-5' -> {1, 2, 3, 4, 5}; '2,4' -> {2, 4}."""
    out = set()
    for part in text.split(","):
        lo, _, hi = part.partition("-")
        out.update(range(int(lo), int(hi or lo) + 1))
    return out


def make_builder():
    spec = importlib.util.spec_from_file_location("ds_make_project", ROOT / "apps" / "desktop" / "selftest" / "make_project.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def record_helpers():
    """record.py's lane_point and pw_press: the Learn still makes the same real mouse gestures
    as the recording (the ghost spike click and the K-add range address no element)."""
    spec = importlib.util.spec_from_file_location("ds_record", ROOT / "tools" / "media" / "record.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# #459: the path guard (check_paths.py) as the last step of the shot tool: every written picture
# is OCR'd; a private path or host name in a picture FAILs the run (non-zero).
_CHECK_PATHS = None


def path_guard():
    """The check_paths module, loaded once (the tests patch its _engine on this instance)."""
    global _CHECK_PATHS
    if _CHECK_PATHS is None:
        spec = importlib.util.spec_from_file_location(
            "ds_check_paths", ROOT / "tools" / "media" / "check_paths.py")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        _CHECK_PATHS = mod
    return _CHECK_PATHS


def run_path_guard(paths):
    """The wired-in guard on the finished picture(s): exits non-zero on a finding."""
    path_guard().guard([str(p) for p in paths])


def demo_project(work):
    """The demo session as a project folder (referenced audio, the roles of the kit)."""
    if not (DEMO / "truth.json").is_file():
        sys.exit(f"no demo session in {DEMO}: run tools/media/demo_session.py --out {DEMO}")
    tracks = [(name, str(DEMO / f"{name}.wav"), role, NOTE[role]) for name, role in ROLE.items()]
    return Path(make_builder().build_project(work / "projects", "Demo Session", tracks, "referenced"))


def install_plugin(home):
    """The example plugin as a real install (the shape of tests/e2e_app/test_plugins.py):
    `~/Library/Application Support/Drum Spike/Detectors/<id>/` in the shot home: manifest,
    executable (chmod +x), times.json, licence text."""
    folder = home / "Library" / "Application Support" / "Drum Spike" / "Detectors" / PLUGIN_ID
    folder.mkdir(parents=True, exist_ok=True)
    shutil.copy(PLUGIN_FIXTURE / "json_hits.py", folder / "json_hits.py")
    (folder / "json_hits.py").chmod(0o755)
    (folder / "times.json").write_text(json.dumps({}))
    (folder / "LICENCE").write_text("MIT License\n\nPermission is hereby granted, free of charge.\n")
    (folder / "manifest.json").write_text(json.dumps({
        "id": PLUGIN_ID,
        "name": "JSON Hits",
        "version": "1.0.0",
        "protocol": 1,
        "executable": "json_hits.py",
        "roles": ["kick", "snare", "tom", "hat", "cymbal", "other"],
        "sample_rates": [44100, 48000, 88200, 96000, 192000],
        "licence": "MIT",
        "licence_file": "LICENCE",
    }))
    return folder


def start_bridge(home):
    env = dict(os.environ, HOME=str(home), DRUMSPIKE_HOME=str(home), DRUMSPIKE_PROJECTS=str(home / "projects"),
               DRUMSPIKE_AUTOSAVE_SECONDS="1", DRUMSPIKE_AUDIO="null", DRUMSPIKE_MASTER_GAIN="0",
               DRUMSPIKE_DETECTORS_DIR=str(home / "Library" / "Application Support" / "Drum Spike" / "Detectors"))
    proc = subprocess.Popen([str(BRIDGE)], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
    line = proc.stdout.readline()
    m = re.search(r"http://127\.0\.0\.1:\d+", line)
    if not m:
        proc.kill()
        sys.exit(f"ds-bridge did not start: {line!r}")
    return proc, m.group(0)


def invoke(page, cmd, **args):
    """A command through the bridge, as the page's B does."""
    return page.evaluate("async ([c, a]) => (await fetch('/invoke/' + c, {method: 'POST', "
                         "headers: {'Content-Type': 'application/json'}, body: JSON.stringify(a)})).json()", [cmd, args])


def menu(page, item, wait=True):
    """A click on a native menu item: the bridge logs it, the page runs the item (as app_support.menu)."""
    page.wait_for_function("() => window.DSShell && DSShell.listening()")
    seq = page.evaluate("() => DSShell.log.seen") + 1
    sent = invoke(page, "selftest_menu", id=item)
    assert sent.get("id") == item, sent
    page.wait_for_function("n => DSShell.log.seen >= n", arg=seq)
    if wait:
        page.wait_for_function("n => n in DSShell.log.results", arg=seq, timeout=60000)


def open_project(page, folder):
    page.evaluate("f => { window.__DS_PICK__ = f; }", str(folder))
    menu(page, "open", wait=False)
    page.wait_for_function("() => S.loaded > 0 && S.order.length === 13 && S.order.every(n => n in S.peaks)",
                           timeout=180000)


def open_project_dropping_edits(page, folder):
    """open_project for a page that was reloaded: the restored project is dirty (the first session's
    edits), so the app asks 'Save before you open another project?'. Those edits are throwaway:
    Continue without saving. The wait is for the new session id, not for the old project's state."""
    old = page.evaluate("() => S.sid")
    page.evaluate("f => { window.__DS_PICK__ = f; }", str(folder))
    menu(page, "open", wait=False)
    page.wait_for_function("() => !S.dirty || !document.querySelector('#confirmModal').classList.contains('hidden')",
                           timeout=60000)
    if page.evaluate("() => !document.querySelector('#confirmModal').classList.contains('hidden')"):
        page.click("#confirmModal button[data-act='skip']")
    page.wait_for_function("old => S.sid !== old && S.loaded > 0 && S.order.length === 13 && "
                           "S.order.every(n => n in S.peaks)", arg=old, timeout=180000)


def inspect_track(page, name, sections):
    """The Inspector shows the track, selected as a user does; only the given sections are open."""
    page.evaluate("() => { if (!S.inspector) setInspector(true); }")
    if page.evaluate("n => S.selTrack !== n", name):
        if page.evaluate("() => !document.querySelector('#confirmModal').classList.contains('hidden')"):
            title, msg = page.evaluate("() => [document.querySelector('#confirmTitle').textContent, "
                                       "document.querySelector('#confirmMsg').textContent]")
            raise RuntimeError(f"a dialog is open before the click on {name}: {title!r}: {msg!r}")
        page.locator(f'.track[data-name="{name}"] .thead .tnote').click()
    page.wait_for_function("n => S.selTrack === n && document.querySelector('#insName').value === n", arg=name)
    page.evaluate("s => { for (const k of INS_SEC_ORDER) INSSEC.open[k] = s.includes(k); insSecApply(); }", list(sections))


def check_dom(page, name):
    """The review guard: before a screenshot, read every text the picture would show (the status
    bar, the dialogs, every input value) and FAIL on a forbidden string."""
    texts = page.evaluate("""() => {
      const t = [document.body.innerText];
      for (const el of document.querySelectorAll('input, textarea, [contenteditable]'))
        if (el.value) t.push(el.value);
      return t;
    }""")
    joined = " ".join(t for t in texts if t).lower()
    bad = [w for w in FORBIDDEN if w.lower() in joined]
    if bad:
        raise RuntimeError(f"{name}: the page shows forbidden text {bad}")


def shooter(page, out, wanted, written, checked):
    """shoot(name, element=None) of one session: the picture of a wanted name, after the guard."""
    def shoot(name, element=None):
        if name in wanted:
            check_dom(page, name)
            checked[0] += 1
            if element is not None:
                element.screenshot(path=str(out / name), type="png")
            else:
                page.screenshot(path=str(out / name), type="png")
            written.append(name)
    return shoot


def make_one_shot(path):
    """A 0.4 s decaying 90 Hz sine as a mono WAV: the one-shot of the chapter 12 layer."""
    rate = 48000
    frames = b"".join(struct.pack("<h", int(20000 * math.exp(-12 * i / rate) * math.sin(2 * math.pi * 90 * i / rate)))
                      for i in range(int(0.4 * rate)))
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(frames)
    return path


def plant_guard_demo(page):
    """Prove the guard fails on a planted string: put 'model', '/Users/' and the old picture's
    temp-home path form into the live page and expect check_dom to raise. Returns True when it did."""
    page.evaluate("""() => {
      document.body.insertAdjacentHTML('beforeend',
        '<div id="__guard_plant">No usable model: fallback detector, see /Users/x ' +
        '(/var/folders/fl/8_r61yl/T/shots-v7gv_mxd/home/detectors/json-hits/json_hits.py, /tmp/x, /private/x)</div>');
    }""")
    try:
        check_dom(page, "guard-demo")
    except RuntimeError as e:
        print(f"guard demo: the guard failed as expected: {e}")
        return True
    finally:
        page.evaluate("() => document.getElementById('__guard_plant').remove()")
    print("guard demo: FAIL - the guard did not catch the planted string")
    return False


def app_pictures(page, work, out, wanted):
    """The app pictures of `wanted` (a set of names in docs/guide/), each taken at its step of one session.
    Returns the names written."""
    written = []
    checked = [0]
    shoot = shooter(page, out, wanted, written, checked)

    open_project(page, demo_project(work))

    inspect_track(page, "k in", sections=("track",))
    shoot("img/02-inspector-role.png")

    page.evaluate("p => { window.__DS_PICK__ = p; }", str(DEMO))
    menu(page, "add_tracks", wait=False)
    page.wait_for_selector("#tracksModal:not(.hidden)")
    page.wait_for_function("() => document.querySelectorAll('#tracksRows > *').length >= 13", timeout=60000)
    try:   # best effort: the transient 'reading the files…' line, when it clears at all
        page.wait_for_function("""() => !((document.querySelector('#stMsg') || {}).textContent || '')
                                      .includes('reading')""", timeout=15000)
    except Exception:
        pass
    shoot("img/02-add-tracks.png")
    page.click("#tracksCancel")

    # Hits > Analyze All: the Export dialog lists every track with its hits, not only the one shown
    menu(page, "analyze_all", wait=False)
    page.wait_for_function("() => document.querySelector('#progressText')?.textContent === 'analysis done'",
                           timeout=900000)
    inspect_track(page, SNARE, sections=("track", "hits", "detection"))
    page.wait_for_function("n => (S.events[n] || []).length > 0", arg=SNARE, timeout=60000)
    shoot("img/03-detection.png")

    # 11 (#428): the Inspector's Learn section at the pl-histogram state of the recording
    # (the ghost forgotten, the missed hit added): the Similarity histogram with the Match
    # and Add lines and the counts line. The lane gestures are record.py's — the same real
    # mouse work the recording does; the names keep the build_project spaces here (the
    # recording's seed renames them, safe_name). The demo facts (#428 probe): the ghost
    # 16.0938 is detected, the 0.168 hit at 18.5938 is missed; the first Forget This greys
    # 5 hits and turns the Learn filter on; the Add line draws with a Learn This example.
    rec = record_helpers()
    x, y = rec.lane_point(page, SNARE, "spike", 16.0938)
    page.mouse.click(x, y)
    page.wait_for_function("() => S.sel && S.sel.track === 's top'", timeout=10000)
    page.keyboard.press(rec.pw_press("Shift+K"))
    page.wait_for_function("() => DSLearn.state().pending === 0", timeout=30000)
    page.wait_for_function("n => (S.events[n] || []).some(e => e.filtered && Math.abs(e.t - 16.0938) <= 0.025)"
                           " && (S.events[n] || []).filter(e => e.filtered).length === 5",
                           arg=SNARE, timeout=15000)
    x1, y = rec.lane_point(page, SNARE, "wave", 18.4438)
    x2, _ = rec.lane_point(page, SNARE, "wave", 18.7438)
    page.mouse.move(x1, y)
    page.mouse.down()
    page.mouse.move((x1 + x2) / 2, y, steps=4)
    page.mouse.move(x2, y, steps=4)
    page.mouse.up()
    page.wait_for_function("() => DSLearn.state().sel !== null", timeout=10000)
    page.keyboard.press(rec.pw_press("k"))
    page.wait_for_function("() => DSLearn.state().pending === 0", timeout=30000)
    page.wait_for_function("n => (S.events[n] || []).some(e => e.learned && Math.abs(e.t - 18.5938) <= 0.025)",
                           arg=SNARE, timeout=15000)
    page.locator(f'.track[data-name="{SNARE}"] .thead .tname').click()
    page.evaluate("() => { if (!S.inspector) setInspector(true); }")
    page.click('.insSec[data-sec="learn"] .insSecHead')
    page.wait_for_function("""() => { const c = document.querySelector('#insParams .lnHist'); if (!c || c.classList.contains('hidden')) return false; const im = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; for (let i = 3; i < im.length; i += 4) if (im[i] > 0) return true; return false; }""",
                           timeout=30000)
    page.wait_for_function("""() => { const r = document.querySelector('#insParams .lnRow'); return !!r && (r.querySelector('.lnCount')||{textContent:''}).textContent.includes('✓'); }""",
                           timeout=15000)
    # the Learn section sits at the panel's bottom edge at 1600x1000: the paint lands in the
    # buffer either way, but the picture must show the canvas and its info row — scroll the
    # row into the panel's view.
    page.locator("#insParams .lnHistRow").scroll_into_view_if_needed()
    shoot("img/11-learn.png")
    # the 3b block below expects its own state: the Detection sections back on, the panel
    # scroll back at the Detection row, the range selection cleared (Escape, 11-learn.md step 1).
    inspect_track(page, SNARE, sections=("track", "hits", "detection"))
    page.locator("#insParams .pSens").scroll_into_view_if_needed()
    page.keyboard.press("Escape")

    # 3b (#394): the Detector select asks for the licence before the first use; the Settings
    # sheet lists the installed plug-ins. The plugin runs nowhere: the dialog is cancelled.
    page.select_option("#insParams .pDetector", PLUGIN_ID)
    page.wait_for_selector("#pluginLicenceModal:not(.hidden)")
    page.wait_for_function("() => (document.querySelector('#plText').textContent || '').includes('MIT')")
    shoot("img/03b-plugin-licence.png")
    page.click("#plCancel")
    menu(page, "settings", wait=False)
    page.wait_for_selector("#settingsModal:not(.hidden)")
    page.wait_for_selector("#setPlugins .prow")
    # the Detector plug-ins group alone: the full sheet is over the 400 KB cap
    shoot("img/03b-plugin-settings.png",
          element=page.locator(".sbSetGroup").filter(has=page.locator("#setPlugins")))
    page.click("#settingsDone")
    page.wait_for_function("() => document.querySelector('#settingsModal').classList.contains('hidden')")

    # the 5 ms zoom around the fourth hit of the snare: the hit line on the first motion of its attack
    t = page.evaluate("n => S.events[n][3].t", SNARE)
    page.evaluate("([n, t]) => { S.snap = false; S.selTrack = n; S.sel = { track: n, index: 3 }; "
                  "setView(t - 0.001, t + 0.004); }", [SNARE, t])
    page.wait_for_timeout(1500)   # the zoom tiles are drawn after the view moves: a shot at once shows the whole song
    shoot("img/04-first-sample.png")

    EXPORTS.mkdir(parents=True, exist_ok=True)
    invoke(page, "export_dest_set", sid=page.evaluate("() => S.sid"), dest=str(EXPORTS))
    menu(page, "export", wait=False)
    page.wait_for_selector("#exportModal:not(.hidden)")
    page.wait_for_function("() => document.querySelector('#exGo') && !document.querySelector('#exGo').disabled",
                           timeout=60000)
    # the settled state: the drag tiles prepared, no analysis line left in the status bar
    page.wait_for_function("""() => ['midi', 'spikes'].every(k => {
        const t = document.querySelector(k === 'midi' ? '#exDragMidi' : '#exDragSpikes');
        return t && t.dataset.state === 'ready'; })""", timeout=120000)
    # the picture shows the neutral home-relative form (no /Users/, no host name)
    page.evaluate("t => { const el = document.querySelector('#exDest'); el.value = t; el.title = t; }",
                  EXPORTS_SHOWN)
    shoot("img/05-export.png")
    return written, checked[0]


def guide_pictures(page, work, out, wanted):
    """#497: the pictures of chapters 6 to 13 that app_pictures does not take (the zoom and the nudge,
    the velocity lane, the review queue, the bleed panel, a follower and a layer, the tempo rulers).
    One session on the demo project; the keys are the app's own, the ones the guide names."""
    written = []
    checked = [0]
    shoot = shooter(page, out, wanted, written, checked)
    open_project_dropping_edits(page, demo_project(work / "guide"))
    sid = page.evaluate("() => S.sid")
    # the reopened project has no hits yet: Analyze All as app_pictures does, then the snare's hits
    menu(page, "analyze_all", wait=False)
    page.wait_for_function("() => document.querySelector('#progressText')?.textContent === 'analysis done'",
                           timeout=900000)
    inspect_track(page, SNARE, sections=("track", "hits"))
    page.wait_for_function("n => (S.events[n] || []).length > 0", arg=SNARE, timeout=60000)

    # 06: a kick and a snare track at the 50 ms zoom; the playhead sits on a snare hit
    t = page.evaluate("n => S.events[n][3].t", SNARE)
    page.evaluate("([n, t]) => { S.snap = false; S.selTrack = n; S.sel = { track: n, index: 3 }; "
                  "setView(t - 0.025, t + 0.025); S.playhead = t; }", [SNARE, t])
    page.wait_for_timeout(1500)
    shoot("img/06-find-your-way.png")

    # 07: the snare hit at the 5 ms zoom, nudged one sample earlier (Edit > Nudge Value: 1 Sample)
    page.evaluate("([n, t]) => { S.snap = false; S.selTrack = n; S.sel = { track: n, index: 3 }; "
                  "setView(t - 0.001, t + 0.004); }", [SNARE, t])
    page.evaluate("() => { const s = document.querySelector('#nudgeSel'); "
                  "s.value = '1smp'; s.dispatchEvent(new Event('change')); }")
    before = page.evaluate("n => S.events[n][3].t", SNARE)
    page.keyboard.press("ArrowLeft")
    page.wait_for_function("([n, b]) => S.events[n][3].t < b", arg=[SNARE, before], timeout=15000)
    page.wait_for_timeout(1500)
    shoot("img/07-edit-hits.png")

    # 08: the velocity lane under the wave lanes; one stalk of the snare dragged up
    page.evaluate("() => { if (!VL.on) vlToggle(); }")
    start = page.evaluate("n => S.events[n][0].t - 0.2", SNARE)
    page.evaluate("t => setView(t, t + 6)", start)
    page.wait_for_timeout(1500)
    vel = page.locator(f'.track[data-name="{SNARE}"] canvas.vel').bounding_box()
    x = page.evaluate("([n, t]) => GEO.laneX + tToX(t, GEO.laneW)", [SNARE, page.evaluate("n => S.events[n][4].t", SNARE)])
    page.mouse.move(x, vel["y"] + vel["height"] / 2)
    page.mouse.down()
    page.mouse.move(x, vel["y"] + vel["height"] * 0.1, steps=4)
    page.mouse.up()
    page.wait_for_timeout(1500)
    shoot("img/08-velocity.png")

    # 09: the review queue at its first doubtful hit: the playhead on the hit, the Doubt field of the info line
    page.keyboard.press("d")
    page.wait_for_function("() => /^doubtful hit 1 of/.test(document.querySelector('#stMsg').textContent)",
                           timeout=30000)
    page.wait_for_function("() => !document.querySelector('#ilDoubtF').classList.contains('hidden')", timeout=10000)
    page.evaluate("() => { const e = S.events[S.sel.track][S.sel.index]; S.playhead = e.t; "
                  "setView(e.t - 0.5, e.t + 0.5); }")
    page.wait_for_timeout(1500)
    print("doubt field:", page.locator("#ilDoubt").inner_text())
    shoot("img/09-doubtful-hits.png")

    # 10: a tom hit that the other mics explain as bleed (grey in the lane), with the Other Mics Under Hit panel
    bleed = page.evaluate("""() => { for (const n of ['t1', 't2', 't3']) {
        const i = (S.events[n] || []).findIndex(e => e.bleed); if (i >= 0) return [n, i, S.events[n][i].t]; }
        return null; }""")
    if bleed is None:
        raise RuntimeError("no bleed hit on the tom mics of the demo session")
    name, i, t = bleed
    page.evaluate("([n, i, t]) => { S.snap = false; S.selTrack = n; S.sel = { track: n, index: i }; "
                  "setView(t - 0.6, t + 0.6); }", [name, i, t])
    page.evaluate("() => { if (!MX.on) mxToggle(); }")
    page.wait_for_timeout(1500)
    # the editor alone (lanes, rulers, the Other Mics panel): the full window is over the 400 KB cap
    shoot("img/10-bleed.png", element=page.locator("#editor"))
    page.evaluate("() => { if (MX.on) mxToggle(); }")

    # 12: the sub mic follows the kick in; the Layer section holds a one-shot with its onset line
    invoke(page, "track_set_follow", sid=sid, name="k out", source="k in")
    invoke(page, "track_layer_set", sid=sid, name="k out", path=str(make_one_shot(work / "kick-layer.wav")))
    inspect_track(page, "k out", sections=("track", "layer"))
    page.wait_for_timeout(1500)
    shoot("img/12-followers-and-layers.png")

    # 13: the tempo and bar rulers over the lanes, the bar and beat grid (8th lines at this zoom) under the hits
    page.evaluate("() => { TM.ui.tempo = true; TM.ui.beats = true; TM.ui.grid = true; tmApplyUi(true); "
                  "setView(0, 8); }")
    page.wait_for_function("() => !FR.raf && FR.flags === 0", timeout=30000)
    page.wait_for_timeout(1000)
    shoot("img/13-tempo.png")
    return written, checked[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--chapters", default="1-5", help="chapter range, e.g. 1-5 or 2,4")
    ap.add_argument("--guard-demo", action="store_true",
                    help="after the shots, plant a forbidden string into the page and prove the guard fails")
    args = ap.parse_args()
    want = chapters(args.chapters)
    names = [k for k, v in json.loads(SHOTS.read_text()).items() if v["chapter"] in want]
    if not names:
        sys.exit(f"no picture of chapters {args.chapters} in {SHOTS.name}")
    failures = []
    stills_written = []
    (GUIDE / "img").mkdir(exist_ok=True)

    for name in names:
        if name in EXPLAINER_STILLS:
            src = FRAMES / EXPLAINER_STILLS[name]
            if src.is_file():
                shutil.copyfile(src, GUIDE / name)
                stills_written.append(name)
            else:
                failures.append(f"{name}: explainer still {src} missing (render tools/media/explainers/build.py 01)")

    app_names = {n for n in names if n not in EXPLAINER_STILLS}
    # app_pictures takes chapters 2 to 5 and 11 (one session); guide_pictures the rest of the app pictures
    chapter = {k: v["chapter"] for k, v in json.loads(SHOTS.read_text()).items()}
    first = {n for n in app_names if chapter[n] <= 5 or chapter[n] == 11}
    later = app_names - first
    written = []
    if app_names:
        if not BRIDGE.is_file():
            sys.exit(f"ds-bridge not built: cargo build -p drumspike-devtools --bin ds-bridge (looked for {BRIDGE})")
        from playwright.sync_api import sync_playwright
        work = Path(tempfile.mkdtemp(prefix="shots-"))
        install_plugin(work / "home")
        proc, url = start_bridge(work / "home")
        try:
            with sync_playwright() as p:
                browser = p.chromium.launch()
                ctx = browser.new_context(viewport={"width": 1600, "height": 1000}, device_scale_factor=2)
                page = ctx.new_page()
                page.goto(url + "/", timeout=180000)
                page.wait_for_function(READY, timeout=180000)
                checked = 0
                if first:
                    wr, ch = app_pictures(page, work, GUIDE, first)
                    written += wr
                    checked += ch
                if later:
                    page.reload(timeout=180000)
                    page.wait_for_function(READY, timeout=180000)
                    wr, ch = guide_pictures(page, work, GUIDE, later)
                    written += wr
                    checked += ch
                print(f"checked shots: {checked}")
                if checked == 0:
                    failures.append("guard: 0 shots were checked against the forbidden text")
                if args.guard_demo and not plant_guard_demo(page):
                    failures.append("guard demo: the planted string was not caught")
                browser.close()
        finally:
            proc.terminate()
            proc.wait(10)
            shutil.rmtree(work, ignore_errors=True)

    # #459: the last step — every written picture is OCR'd; a private path or host name in a
    # picture FAILs the run (non-zero).
    guarded = [GUIDE / n for n in stills_written] + [GUIDE / n for n in written]
    if guarded:
        run_path_guard(guarded)

    for name in names:
        path = GUIDE / name
        if not path.is_file():
            failures.append(f"{name}: not written")
        elif path.stat().st_size > MAX_BYTES:
            failures.append(f"{name}: {path.stat().st_size} bytes, over 400 KB")
        else:
            print(f"{name}: {path.stat().st_size} bytes")
    for line in failures:
        print("FAIL " + line)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
