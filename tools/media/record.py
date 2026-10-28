"""#245: the screen recorder: a scenario JSON in, a narrated MP4 out.

    uv run --project . python tools/media/record.py tools/media/scenarios/part1.json
    [--out MP4] [--tts-out DIR] [--keep]

A scenario lists the scenes of a product video (docs/product/scripts/rec-part1.md is the part 1
script: one `##` per scene, its `>` lines are the steps and captions). The recorder

- builds the demo session of tools/media/demo_session.py when ~/.cache/drumspike/media/demo/ is
  empty, and lays a session folder with an Audio Files/ dir for the Add Tracks scene,
- a scenario with "seed": "demo" first builds the demo session of part 1 in a video-less page
  (project Demo, the 13 demo WAVs with their roles, new take as kick, Analyze all) and boots the
  recording page into that session: parts 2 and 3 record on it (part 1 builds it on camera),
- a "menu" step runs a menu-only command (no key in static/keymap.js: Detect Tempo…, Set Bar 1
  at Playhead, Import Tempo Map…) through the bridge's menu event, so the page's own listener
  (static/shell.js) runs the real item; the Import scene's DAW MIDI (tempo.mid: 87 BPM 4/4,
  100 BPM and 3/4 from bar 5) and the Layer scene's one-shot (one-shot.wav, the first 1.5 s of
  the demo sub mic — a layer one-shot is at most 20 s) are written into the work dir,
- starts ds-bridge with the DSP detector (the video shows what ships), drives the page with
  Playwright and the real keys, under the key/caption
  overlay of the coordinator's prototype (rec_demo-prototype.py),
- records the page video, muxes the narration WAVs of --tts-out (the out dir of tools/media/tts.py;
  the TTS API is NEVER called here) at the scene boundaries of the narration schedule and writes
  the MP4.

Each scene is held to its narration length: a scene whose steps overrun pushes every later scene
late against the fixed audio plan. A scene that overruns anyway (a slow host, a long analysis)
is repaired in the mux: the frames of its wait steps (wait for an event or a condition) are sped
up with ffmpeg setpts so the scene lands on its narration slot again, and a small badge in the
top-right corner shows the factor (for example 2x) while the compressed part plays. The badge
never covers the progress line (the status bar is at the bottom). The factor is capped: a scene
that needs more than 3x FAILs with its measured factor, so a slow host still shows up. The
factor per scene is printed next to the drift. Scenes that fit are left at 1x.

A scenario with "plugins": true installs the example plugin
(tests/fixtures/plugins/json_hits.py) into the recording home's Detectors folder before the
bridge starts and runs the bridge with that home (DRUMSPIKE_DETECTORS_DIR, HOME), so the
Detector select lists JSON Hits, the licence dialog shows the neutral "~/Library/..." path
(shown_path) and the plug-in proposes its own hits (times.json, every third kick of the demo
session) that Drum Spike refines to the attack.

The caption of a scene ends with the scene (the overlay fades it at the scene's end), and every
key a step presses is shown in the key overlay for 1.5 s or more. Every frame of the written
video carries the "AI-generated" note (the media plan's disclosure): the mux overlays the note
PNG at the explainers' corner (right 28 / bottom 20) — the shared renderer of compose.py, because
the landing page plays the recordings as their own videos. The scenarios show the neutral
forms of the paths themselves: a show_dest step sets the Export dialog's destination to the
home-relative text of the guide pictures (shots.py's EXPORTS_SHOWN; the bridge-side dest stays
the project default, so the app's own summary keeps its neutral "Exports/ in the project folder"
phrase) and keeps it there — the app re-renders the input on every exRender, so each of them
re-asserts the text; a show_text step rewrites an element the app printed a host path into (the
status bar's "created Demo in …"). The video is the media plan's 1920x1080 at 2x device scale
(docs/product/media-plan.md). FAIL (exit 1) when a step's element or event is not found, when a
menu title in a caption is not in static/keymap.js, when a scene starts with a modal still open
(a dialog the scene before it left open), when a scene still drifts more than 100 ms after the
compression, or when the page shows forbidden text at a scene's start — the guard of shots.py
(reviews 6396240075 and 6403628912) reads the status bar, the dialogs and every input value and
FAILs on "model", "/Users/", "x86-mac", "/var/folders", "/tmp/" or "/private/";
--allow-model-text drops only "model" for the test recording on
pre-#294 main (the path guard stays; the final recording runs on model-free main without the
flag). The drift per scene is printed. A failed run keeps the raw video as <out>.failed.webm.

Heavy: run under the host lock (crewboard lock run ...).
"""
import argparse
import html
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
import wave
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CACHE = Path.home() / ".cache" / "drumspike" / "media"
DRIFT_LIMIT_S = 0.1
MAX_WAIT_SPEED = 3.0     # a scene that needs more than 3x FAILs: a slow host must show up
BADGE_X_S = 28           # the badge sits in the top-right corner, clear of the status bar
BADGE_Y_S = 28
BADGE_FONT_S = 22
NOTE_TEXT = "AI-generated"     # the media plan's disclosure corner note (owner, 2026-10-23)
NOTE_PX = 20
NOTE_MARGIN_PX = 28
NOTE_BOTTOM_PX = NOTE_MARGIN_PX - 8   # the explainers' corner note: right 28, bottom 20
NOTE_COLOR = "#7d8593"         # the explainers' DIM, for the note
NOTE_FONT = "Helvetica, Arial, sans-serif"   # one sans-serif family, like the explainers
STEP_KINDS = ("caption", "key", "show_key", "hide_key", "click", "select", "menu", "answer_save", "answer_pick",
              "show_dest", "show_text", "lane_click", "drag", "wait", "visible", "text", "hold", "blur")
READY = "typeof S !== 'undefined' && typeof B !== 'undefined' && B.app === true && S.loaded > 0"
CAP_JS = "s => { const c = document.getElementById('__cap'); c.textContent = s; c.style.opacity = s ? 1 : 0; }"
SHOWKEY_JS = "(a) => window.__dsShowKey(a[0], a[1])"
VISIBLE_TEXT_JS = "() => document.body ? document.body.innerText : ''"
# No modal may be visible at a scene's start: a dialog is a .modal div that hides behind the
# 'hidden' class (showModal removes it, its Close button or Escape adds it back). The undo
# history of the part2 review opened at Alt+Meta+Z and was never closed, so a third of the
# video played dimmed behind it.
MODALS_JS = "() => [...document.querySelectorAll('.modal:not(.hidden)')].map(m => m.id || m.className)"
MODEL_RE = re.compile("model", re.IGNORECASE)
# The review guard of shots.py (reviews 6396240075 and 6403628912): text a recording must never
# show. Before every scene the status bar, the dialogs and every input value are read; a hit FAILs
# the run (the product video must not show a host path, a user name, a temp dir or a model text).
# The same list as shots.py FORBIDDEN, one source of truth for both tools.
FORBIDDEN = ("model", "/Users/", "x86-mac", "/var/folders", "/tmp/", "/private/")
DOM_TEXTS_JS = """() => {
  const t = [document.body ? document.body.innerText : ''];
  for (const el of document.querySelectorAll('input, textarea, [contenteditable]'))
    if (el.value) t.push(el.value);
  return t;
}"""
# The neutral destination form of the guide pictures (shots.py EXPORTS_SHOWN): the Export
# dialog shows the home-relative path, never the host's absolute one. The bridge-side dest
# stays the project default, so the app's own summary keeps its neutral
# "Exports/ in the project folder" phrase (an absolute dest would print it into the status
# bar). The app re-renders #exDest from its own state on every exRender — the Export click
# and the awaited answers of exInfo/exDragSchedule re-render it (the 42a7534 gate's
# p1-again) — so a one-time write loses that race: the shown text is kept in a page global
# and every exRender re-asserts it.
SHOW_DEST_JS = """t => {
  const el = document.querySelector('#exDest');
  if (!el) throw new Error('no #exDest input: the Export dialog is not in the page');
  window.__dsDestShown = t;
  el.value = t; el.title = t;
  if (!window.__dsDestKeep) {
    window.__dsDestKeep = true;
    const orig = window.exRender;
    if (typeof orig === 'function') window.exRender = function () {
      const r = orig.apply(this, arguments);
      const e = document.querySelector('#exDest');
      if (e && window.__dsDestShown) { e.value = window.__dsDestShown; e.title = window.__dsDestShown; }
      return r;
    };
  }
}"""
# A path the app printed, shown in its neutral form: the scenario names the element and the text
# (the status bar keeps "created Demo in <host path>" otherwise).
SHOW_TEXT_JS = ("([sel, text]) => { const el = document.querySelector(sel);"
                " if (!el) throw new Error('no element ' + sel); el.textContent = text; }")
KEY_HOLD_MS = 1600   # the review wants every pressed key visible for at least 1.5 s


class RecordError(Exception):
    pass


# ---------------- the scenario ----------------

def parse_scenario(scenario):
    """The scenario dict, validated: scenes with ids and steps of known kinds. 'skip' names scene
    ids of the script that this recording leaves out (their narration WAVs stay unused)."""
    if not isinstance(scenario, dict) or not isinstance(scenario.get("scenes"), list) or not scenario["scenes"]:
        raise RecordError("the scenario needs a non-empty 'scenes' list")
    skip = scenario.get("skip", [])
    ids = {sc.get("id") for sc in scenario["scenes"]}
    if not isinstance(skip, list) or any(s not in ids for s in skip):
        raise RecordError("skip names scene ids the scenario does not have")
    scenes = [sc for sc in scenario["scenes"] if sc.get("id") not in skip]
    if not scenes:
        raise RecordError("every scene is skipped")
    for sc in scenes:
        if not sc.get("id") or not isinstance(sc.get("steps"), list):
            raise RecordError(f"every scene needs an id and steps: {sc!r}")
        for st in sc["steps"]:
            if not isinstance(st, dict) or st.get("do") not in STEP_KINDS:
                raise RecordError(f"{sc['id']}: unknown step (do must be one of {', '.join(STEP_KINDS)}): {st!r}")
            if st["do"] == "caption" and not isinstance(st.get("text"), str):
                raise RecordError(f"{sc['id']}: a caption step needs text")
            if st["do"] == "show_dest" and not isinstance(st.get("text"), str):
                raise RecordError(f"{sc['id']}: a show_dest step needs the shown text")
            if st["do"] == "show_text" and (not isinstance(st.get("sel"), str)
                                            or not isinstance(st.get("text"), str)):
                raise RecordError(f"{sc['id']}: a show_text step needs the element and the shown text")
            if st["do"] == "lane_click" and (not isinstance(st.get("track"), str)
                                             or not isinstance(st.get("t"), (int, float))):
                raise RecordError(f"{sc['id']}: a lane_click step needs the track and the time t")
            if st["do"] == "drag" and (not isinstance(st.get("track"), str)
                                       or not isinstance(st.get("from"), (int, float))
                                       or not isinstance(st.get("to"), (int, float))):
                raise RecordError(f"{sc['id']}: a drag step needs the track and the from/to times")
    seed = scenario.get("seed")
    if seed is not None and seed != "demo":
        raise RecordError("seed must be \"demo\" (the demo session of part 1, built before the video)")
    if "plugins" in scenario and scenario["plugins"] is not True:
        raise RecordError("plugins must be true (the example plugin installs before the video)")
    return {**scenario, "scenes": scenes}


def menu_pairs(caption):
    """The menu reference of a caption: 'File > New Project…' -> ('File', 'New Project…'); a
    caption without '>' names no menu. Deeper paths keep the chain: 'View > Waveform Zoom'."""
    if ">" not in caption:
        return []
    parts = [p.strip() for p in caption.split(">")]
    return [(" > ".join(parts[:-1]), parts[-1])]


def keymap_commands(text):
    """The (path, title) pairs of static/keymap.js, one per command object."""
    out = set()
    for chunk in text.split('{"id"')[1:]:
        p = re.search(r'"path":\s*"([^"]+)"', chunk)
        t = re.search(r'"title":\s*"([^"]+)"', chunk)
        if p and t:
            out.add((p.group(1), t.group(1)))
    return out


def caption_problems(scenes, commands):
    """Every menu title of a caption must be a command of the key table."""
    problems = []
    for sc in scenes:
        for st in sc["steps"]:
            if st["do"] != "caption":
                continue
            for path, title in menu_pairs(st["text"]):
                if (path, title) not in commands:
                    problems.append(f"{sc['id']}: the caption {st['text']!r} names no menu "
                                    f"{title!r} under {path!r} in static/keymap.js")
    return problems


def load_narration(tts_dir, scene_ids):
    """[(scene id, wav path, seconds)] in scenario order, from tts.py's scenes.json."""
    meta = json.loads((tts_dir / "scenes.json").read_text(encoding="utf-8"))
    by_id = {r["id"]: r for r in meta["scenes"]}
    items = []
    for sid in scene_ids:
        if sid not in by_id:
            raise RecordError(f"no narration for scene {sid} in {tts_dir} (run tools/media/tts.py, or pass --tts-out)")
        items.append((sid, tts_dir / by_id[sid]["file"], float(by_id[sid]["seconds"])))
    return items


def expand(path, values):
    """{placeholders} then ~."""
    for k, v in values.items():
        path = path.replace("{" + k + "}", v)
    return Path(os.path.expanduser(path))


# ---------------- the inputs ----------------

def ensure_demo_session(demo):
    """The demo session of tools/media/demo_session.py, generated when missing."""
    if (demo / "truth.json").is_file():
        return
    spec = importlib.util.spec_from_file_location("demo_session", ROOT / "tools" / "media" / "demo_session.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    print(f"demo session: generating {demo}")
    mod.generate(demo)


def build_session_folder(demo, session):
    """A session folder with the demo WAVs in Audio Files/ (the folder the Add Tracks scene picks)."""
    audio = session / "Audio Files"
    audio.mkdir(parents=True, exist_ok=True)
    for wav in sorted(demo.glob("*.wav")):
        shutil.copyfile(wav, audio / wav.name)
    return audio


def place_session_files(scenario, env):
    """Files the Add Tracks scene lists but the seed must not add as tracks, written after the
    seed: SEED_JS scans the same Audio Files folder and adds every file it finds as a track."""
    for spec in scenario.get("session_files", []):
        shutil.copyfile(expand(spec["from"], env), Path(env["session"]) / "Audio Files" / spec["as"])


def write_daw_midi(path):
    """The DAW MIDI of the Import Tempo Map scene ({work}/tempo.mid): a format-0 file with the
    tempo and meter events of a DAW (87 BPM 4/4 from bar 1, 100 BPM and 3/4 from bar 5, PPQ 480).
    The import refuses a file without tempo and meter events, so the DAW file carries both."""
    def vlq(n):
        out = bytearray([n & 0x7F])
        n >>= 7
        while n:
            out.append(0x80 | (n & 0x7F))
            n >>= 7
        return bytes(reversed(out))

    def meta(tick, kind, data):
        return vlq(tick) + bytes([0xFF, kind]) + vlq(len(data)) + data

    track = b"".join([
        meta(0, 0x58, bytes([4, 2, 24, 8])),             # 4/4 at bar 1
        meta(0, 0x51, (689655).to_bytes(3, "big")),      # 87 BPM (6e7 / 689655)
        meta(7680, 0x58, bytes([3, 2, 24, 8])),          # 3/4 at bar 5 (4 bars * 4 beats * 480)
        meta(7680, 0x51, (600000).to_bytes(3, "big")),   # 100 BPM
        meta(0, 0x2F, b""),                              # end of track
    ])
    hdr = b"MThd" + (6).to_bytes(4, "big") + (0).to_bytes(2, "big") + (1).to_bytes(2, "big") + (480).to_bytes(2, "big")
    path.write_bytes(hdr + b"MTrk" + len(track).to_bytes(4, "big") + track)


def write_one_shot(work, demo):
    """The Layer scene's one-shot ({work}/one-shot.wav): the first 1.5 s of the demo's sub mic
    (k out.wav). A layer one-shot is at most 20 s (read_one_shot refuses longer files), so the
    64 s recording itself cannot go on the zone; the trimmed head is the engineer's own sample
    of the same drum, and the Onset view and Align get real material."""
    with wave.open(str(demo / "k out.wav")) as r:
        p = r.getparams()
        frames = r.readframes(min(int(1.5 * p.framerate), p.nframes))
    dest = work / "one-shot.wav"
    with wave.open(str(dest), "wb") as w:
        w.setnchannels(p.nchannels)
        w.setsampwidth(p.sampwidth)
        w.setframerate(p.framerate)
        w.writeframes(frames)
    return dest


# #394: the example plugin (tests/fixtures/plugins/json_hits.py, "tests only, never bundled"),
# installed into the recording home's Detectors folder for the scenarios that ask for it
# ("plugins": true). The install is the real form (the shape of shots.py install_plugin and
# tests/e2e_app/test_plugins.py): manifest.json, the executable with +x, the licence text, and
# times.json with every third kick of the demo session — the plugin proposes its own list, Drum
# Spike puts each hit on the first sample of its attack.
PLUGIN_ID = "json-hits"
PLUGIN_FIXTURE = ROOT / "tests" / "fixtures" / "plugins"


def install_plugin(home, demo):
    """The example plugin as a real install in `home`'s Detectors folder; returns the folder."""
    folder = home / "Library" / "Application Support" / "Drum Spike" / "Detectors" / PLUGIN_ID
    folder.mkdir(parents=True, exist_ok=True)
    shutil.copy(PLUGIN_FIXTURE / "json_hits.py", folder / "json_hits.py")
    (folder / "json_hits.py").chmod(0o755)
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
    times = []
    truth = demo / "truth.json"
    if truth.is_file():
        data = json.loads(truth.read_text(encoding="utf-8"))
        sr = data.get("sr", 48000)
        times = [round(h["t"] / sr, 6) for h in data.get("hits", []) if h.get("drum") == "kick"][::3]
    (folder / "times.json").write_text(json.dumps({"kick": times}))
    return folder


def start_bridge(root, env_extra):
    """ds-bridge of the repo (debug, else release) on its printed URL."""
    binary = root / "target/debug/ds-bridge"
    if not binary.is_file():
        binary = root / "target/release/ds-bridge"
    if not binary.is_file():
        raise RecordError("no ds-bridge: run cargo build -p drumspike-app")
    env = dict(os.environ, **env_extra)
    proc = subprocess.Popen([str(binary)], cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            text=True, env=env)
    url = None
    end = time.time() + 60
    while time.time() < end:
        line = proc.stdout.readline()
        if not line:
            break
        m = re.search(r"http://127\.0\.0\.1:\d+", line)
        if m:
            url = m.group(0)
            break
    if not url:
        proc.terminate()
        raise RecordError("ds-bridge printed no URL")
    return proc, url


# ---------------- the run ----------------

def key_label(press):
    """The overlay label of a Playwright press string: 'Shift+Meta+I' -> 'Shift+Cmd+I'."""
    names = {"Meta": "Cmd", "Cmd": "Cmd", "Alt": "Option", "Option": "Option", "Shift": "Shift",
             "Ctrl": "Ctrl", "Control": "Ctrl"}
    order = ["Shift", "Option", "Ctrl", "Cmd"]
    parts = press.split("+")
    mods = [names[p] if p in names else p for p in parts[:-1]]
    return "+".join([m for m in order if m in mods] + [m for m in mods if m not in order] + [parts[-1]])


def show_key(page, press):
    """The key overlay shows the pressed key for KEY_HOLD_MS (the JS timer hides it again)."""
    page.evaluate(SHOWKEY_JS, (key_label(press), KEY_HOLD_MS))


# The key table's notation -> Playwright's key names. The scenarios write the keymap notation
# (static/keymap.js: "Left", "Return"), Playwright names the arrows "ArrowLeft" and rejects "Return".
PLAYWRIGHT_KEY = {"Return": "Enter", "Left": "ArrowLeft", "Right": "ArrowRight", "Up": "ArrowUp",
                  "Down": "ArrowDown", "Plus": "+", "Minus": "-", "NumpadMinus": "NumpadSubtract",
                  "NumpadPlus": "NumpadAdd", "NumpadPeriod": "NumpadDecimal", "Option": "Alt",
                  "Ctrl": "Control"}


def pw_press(press):
    """The scenario's press string in Playwright's notation: 'Shift+Return' -> 'Shift+Enter'."""
    parts = press.split("+")
    key = PLAYWRIGHT_KEY.get(parts[-1], parts[-1])
    return "+".join(parts[:-1] + [key])


# A real lane position: the page x of the time t in a lane (the app's own mapping), refusing a
# time the view does not show — the scenario zooms or pans first, the step never moves the view
# on its own.
LANE_X_JS = """([track, t]) => {
  if (typeof S === 'undefined' || !S.tracks || !S.tracks[track]) throw new Error('no track ' + track);
  const x = tToX(t, laneWidth());
  if (x < 4 || x > laneWidth() - 4) throw new Error('t ' + t + ' is outside the lane view');
  return GEO.laneX + x;
}"""


def lane_point(page, track, lane, t):
    """The (x, y) page point of the time t in a track's lane (the lane canvas's middle height)."""
    loc = page.locator(f'.track[data-name={json.dumps(track)}] canvas.{lane}')
    loc.scroll_into_view_if_needed()   # 13 rows scroll: the raw mouse click needs the row on screen
    box = loc.bounding_box()
    if not box:
        raise RecordError(f"no {lane} lane of {track}")
    x = page.evaluate(LANE_X_JS, [track, t])
    return x, box["y"] + box["height"] / 2


def run_step(page, st, env):
    """One scenario step on the page. A not-found element or event raises (the label names it)."""
    kind = st["do"]
    timeout = int(st.get("timeout", 30) * 1000)
    if kind == "caption":
        page.evaluate(CAP_JS, st["text"])
    elif kind == "key":
        show_key(page, st["press"])
        page.keyboard.press(pw_press(st["press"]))
    elif kind == "show_key":   # press the key only when the element is hidden (the Inspector is open by default)
        hidden = page.evaluate("s => !!document.querySelector(s)?.classList.contains('hidden')", st["sel"])
        if hidden:
            show_key(page, st["press"])
            page.keyboard.press(pw_press(st["press"]))
            page.wait_for_selector(st["sel"], state="visible", timeout=timeout)
    elif kind == "hide_key":   # the mirrored normalization: press only when the element is visible, no badge
        hidden = page.evaluate("s => !!document.querySelector(s)?.classList.contains('hidden')", st["sel"])
        if not hidden:
            page.keyboard.press(pw_press(st["press"]))
            page.wait_for_selector(st["sel"], state="hidden", timeout=timeout)
    elif kind == "click":
        page.click(st["sel"], timeout=timeout)
    elif kind == "lane_click":
        # A real mouse click at a lane position: the ghost-note selection of the Learn scenes
        # clicks a hit's spike, which no element selector addresses.
        x, y = lane_point(page, st["track"], st.get("lane", "spike"), st["t"])
        page.mouse.click(x, y)
    elif kind == "drag":
        # A real mouse drag in a lane: the wave lane's range select (the Learn add range).
        # wait_sel waits for the selection the gesture leaves (DSLearn.state().sel).
        lane = st.get("lane", "wave")
        x1, y = lane_point(page, st["track"], lane, st["from"])
        x2, _ = lane_point(page, st["track"], lane, st["to"])
        page.mouse.move(x1, y)
        page.mouse.down()
        page.mouse.move((x1 + x2) / 2, y, steps=4)
        page.mouse.move(x2, y, steps=4)
        page.mouse.up()
        if st.get("wait_sel"):
            page.wait_for_function("() => DSLearn.state().sel !== null", timeout=timeout)
    elif kind == "blur":
        # Drop the focused element (plumbing, no user action on screen): a scene that follows a
        # slider or a select carries its focus, and the app's key guard (P3-13a) swallows the
        # next app key on an input target. blur restores the fresh-page state the key expects.
        page.evaluate("() => { if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur(); }")
    elif kind == "select":
        page.select_option(st["sel"], st["value"], timeout=timeout)
    elif kind == "menu":
        # A menu-only command (no key in static/keymap.js) through the bridge's menu event
        # (P3-13a): the page's own listener (static/shell.js) runs the real item. The wait is
        # for the handler's start: one that opens a dialog ends only when it is answered.
        before = page.evaluate("() => (window.DSShell && DSShell.log) ? DSShell.log.seen : -1")
        page.evaluate("async id => (await fetch('/invoke/selftest_menu', {method: 'POST', "
                      "headers: {'Content-Type': 'application/json'}, body: JSON.stringify({id})})).json()",
                      st["id"])
        page.wait_for_function("n => window.DSShell && DSShell.log && DSShell.log.seen > n",
                               arg=before, timeout=timeout)
    elif kind == "answer_save":
        page.evaluate("p => { window.__DS_SAVE__ = p; }", str(expand(st["path"], env)))
    elif kind == "answer_pick":
        page.evaluate("p => { window.__DS_PICK__ = p; }", str(expand(st["path"], env)))
    elif kind == "show_dest":
        # The Export dialog's destination in the neutral home-relative form (shots.py's
        # EXPORTS_SHOWN): the bridge-side dest stays the project default, so the app's own
        # summary keeps its neutral "Exports/ in the project folder" phrase. The show keeps:
        # every exRender of the app re-asserts the text (the export's own re-renders would
        # otherwise write the work-home path back into the input).
        page.evaluate(SHOW_DEST_JS, st["text"])
    elif kind == "show_text":
        # A path the app printed, in its neutral form: the element and the shown text are data
        # of the scenario (the status bar keeps the host path the app wrote otherwise).
        page.evaluate(SHOW_TEXT_JS, [st["sel"], st["text"]])
    elif kind == "wait":
        if "arg" in st:
            page.wait_for_function(st["cond"], arg=page.evaluate(st["arg"]), timeout=timeout)
        else:
            page.wait_for_function(st["cond"], timeout=timeout)
    elif kind == "visible":
        page.wait_for_selector(st["sel"], state="visible", timeout=timeout)
    elif kind == "text":
        sel, val = json.dumps(st["sel"]), json.dumps(st.get("equals") or st["contains"])
        if "equals" in st:
            cond = f"() => (document.querySelector({sel})||{{}}).textContent?.trim() === {val}"
        else:
            cond = f"() => ((document.querySelector({sel})||{{}}).textContent||'').includes({val})"
        page.wait_for_function(cond, timeout=timeout)
    elif kind == "hold":
        page.wait_for_timeout(round(st["s"] * 1000))


def dom_problems(page, forbidden):
    """The forbidden strings the page shows now (the status bar, the dialogs, every input
    value), compared lowercased like shots.py's guard; [] when the DOM is clean."""
    texts = page.evaluate(DOM_TEXTS_JS) or []
    joined = " ".join(t for t in texts if t).lower()
    return [w for w in forbidden if w.lower() in joined]


def run_scenes(page, scenes, durations, env=None, clock=time.monotonic, check_model=True):
    """The scenes in order; every scene is held to its narration length. Returns (marks, drifts,
    waits): the scene start times, the per-scene overrun (the scene's span over its narration
    length; the modal guard, the DOM guard, the model-text check and the caption fade are part
    of the span), and per scene the [start, end] spans of its wait steps on the clock (the raw
    video shows them at full speed; the mux may compress them). At the start of every scene no
    .modal may be visible: a dialog the scene before it left open FAILs the run with the
    scene's and the modal's id. The DOM guard of shots.py (review 6396240075) then reads the
    status bar, the dialogs and every input value and FAILs on a forbidden string (the paths
    and the user name always; "model" unless check_model). At the end of every scene the page's
    visible text must not contain "model" (check_model) and the caption fades: a caption ends
    with its own scene."""
    env = env or {}
    forbidden = FORBIDDEN if check_model else tuple(w for w in FORBIDDEN if w != "model")
    t0 = clock()
    marks, drifts, waits = [], [], []
    for i, (sc, dur) in enumerate(zip(scenes, durations)):
        marks.append(clock() - t0)
        open_modals = page.evaluate(MODALS_JS) or []
        if open_modals:
            raise RecordError(f"{sc['id']}: a modal is open at the start of the scene: "
                              f"{', '.join(open_modals)}")
        shown = dom_problems(page, forbidden)
        if shown:
            raise RecordError(f"{sc['id']}: the page shows forbidden text {shown}")
        sc_waits = []
        for st in sc["steps"]:
            if st["do"] == "wait":
                s = clock() - t0
                try:
                    run_step(page, st, env)
                except RecordError:
                    raise
                except Exception as e:
                    raise RecordError(f"{sc['id']}: {st.get('err') or st['do']}: {e}") from e
                sc_waits.append((s, clock() - t0))
            else:
                try:
                    run_step(page, st, env)
                except RecordError:
                    raise
                except Exception as e:
                    raise RecordError(f"{sc['id']}: {st.get('err') or st['do']}: {e}") from e
        waits.append(sc_waits)
        if check_model:
            text = page.evaluate(VISIBLE_TEXT_JS) or ""
            hits = [ln.strip() for ln in text.splitlines() if MODEL_RE.search(ln)]
            if hits:
                raise RecordError(f"{sc['id']}: the page's visible text contains 'model': {hits[0][:120]}")
        page.evaluate(CAP_JS, "")   # the caption ends with its own scene
        # The scene is held to its own narration length from its own start (not to the global
        # schedule): a scene after an overrun keeps its full span in the raw video, so the wait
        # compression can put every scene back on the repaired plan. The drift is the scene's
        # own overrun (its span minus its narration, check and fade included) — the repair
        # removes exactly what the scene ran over.
        page.wait_for_timeout(max(0, round((marks[i] + dur - (clock() - t0)) * 1000)))
        drifts.append((clock() - t0) - (marks[i] + dur))
    return marks, drifts, waits


def drift_problems(scenes, drifts):
    return [f"{sc['id']}: drift {d * 1000:+.0f} ms (limit {DRIFT_LIMIT_S * 1000:.0f} ms)"
            for sc, d in zip(scenes, drifts) if abs(d) > DRIFT_LIMIT_S]


# ---------------- the wait compression (option d: the mux repairs a slow scene) ----------------

def speed_label(factor):
    """'2.0' -> '2x', '1.45' -> '1.45x'."""
    return f"{factor:.2f}".rstrip("0").rstrip(".") + "x"


def wait_speeds(drifts, waits, limit=DRIFT_LIMIT_S, cap=MAX_WAIT_SPEED):
    """Per scene the wait-compression factor (1.0 when none) that removes the scene's own
    overrun (its span over its narration length), from the measured overruns and the wait
    spans. Raises when a scene needs more than cap (or has no wait to compress): a slow host
    must show up, not hide."""
    factors, reductions = [], []
    for i, drift in enumerate(drifts):
        w = sum(e - s for s, e in waits[i])
        if drift <= limit:
            factors.append(1.0)
            reductions.append(0.0)
            continue
        if w <= 0:
            raise RecordError(f"{i}: overruns {drift * 1000:+.0f} ms with no wait step to compress")
        if w <= drift:
            raise RecordError(f"{i}: needs an infinite wait speed (drift {drift * 1000:+.0f} ms, "
                              f"wait {w:.2f} s): the steps alone overrun the narration")
        f = w / (w - drift)
        if f > cap:
            raise RecordError(f"{i}: needs a wait speed of {speed_label(f)} (drift {drift * 1000:+.0f} ms, "
                              f"wait {w:.2f} s), over the {speed_label(cap)} cap")
        factors.append(f)
        reductions.append(drift)
    return factors, reductions


def compression_segments(waits, lead, factors, total):
    """[(start, end, factor)] covering the whole raw video: every wait span (moved to video time
    by the lead) at its scene's factor, the rest at 1x."""
    spans = [(s + lead, e + lead, factors[i]) for i, sc_waits in enumerate(waits)
             for (s, e) in sc_waits if factors[i] > 1.0 and e > s]
    spans.sort()
    segs, prev = [], 0.0
    for s, e, f in spans:
        if s > prev:
            segs.append((prev, s, 1.0))
        segs.append((s, e, f))
        prev = e
    if total > prev:
        segs.append((prev, total, 1.0))
    return segs


def compressed_starts(marks0, lead, durations, drifts, reductions):
    """The scene starts on the repaired timeline: the video's lead, then every scene's span on
    that timeline (its narration length plus its measured overrun minus its reduction)."""
    starts = [lead + marks0]
    for i in range(len(durations) - 1):
        starts.append(starts[-1] + durations[i] + drifts[i] - reductions[i])
    return starts


def compression_filter(segs, badge_inputs):
    """The -filter_complex of the repair: trim+setpts per segment, concat, then every distinct
    factor's badge PNG over its compressed spans (enable on the repaired timeline)."""
    parts = []
    for i, (s, e, f) in enumerate(segs):
        chain = f"trim=start={s:.3f}:end={e:.3f},setpts=(PTS-STARTPTS)/{f:.6f}"
        parts.append(f"[0:v]{chain}[s{i}]")
    n = len(segs)
    parts.append("".join(f"[s{i}]" for i in range(n)) + f"concat=n={n}:v=1:a=0[cat]")
    cur, windows = "cat", {}
    for i, (s, e, f) in enumerate(segs):
        if f > 1.0:
            t = sum((e2 - s2) / f2 for s2, e2, f2 in segs[:i])    # the span's start on the repaired timeline
            windows.setdefault(f, []).append((t, t + (e - s) / f))
    for k, (f, wins) in enumerate(sorted(windows.items())):
        expr = "+".join(f"between(t,{a:.3f},{b:.3f})" for a, b in wins)
        idx = badge_inputs[f]
        parts.append(f"[{cur}][{idx}:v]overlay=x=main_w-overlay_w-{BADGE_X_S}:y={BADGE_Y_S}:"
                     f"enable='{expr}'[v{k}]")
        cur = f"v{k}"
    return ";".join(parts), cur


def _render_off_thread(build):
    """A PNG render on its own thread. The sync Playwright API refuses a second event loop in
    a thread that already runs one, and the e2e suite keeps pytest-playwright's session loop
    open on the calling thread (the red of run 40241732523). The loop is thread-local, so a
    fresh thread always renders."""
    box = {}

    def run():
        try:
            box["out"] = build()
        except BaseException as e:   # re-raised on the caller's thread below
            box["err"] = e

    t = threading.Thread(target=run, name="ds-media-render")
    t.start()
    t.join()
    if "err" in box:
        raise box["err"]
    return box["out"]


def render_badges(work, factors):
    """A transparent PNG per distinct factor > 1, in the key overlay's style (Playwright)."""
    def build():
        from playwright.sync_api import sync_playwright
        out = {}
        with sync_playwright() as pw:
            br = pw.chromium.launch()
            pg = br.new_page(viewport={"width": 260, "height": 90})
            for f in sorted(set(f for f in factors if f > 1.0)):
                path = work / f"badge-{speed_label(f)}.png"
                pg.set_content(f'<body style="margin:0;background:transparent">'
                               f'<div style="display:inline-block;background:#ff9f43;color:#14161a;'
                               f'font:600 {BADGE_FONT_S}px Inter,Helvetica,sans-serif;border-radius:10px;'
                               f'padding:8px 16px;">{speed_label(f)}</div></body>')
                pg.locator("div").screenshot(path=str(path), omit_background=True)
                out[f] = path
            br.close()
        return out
    return _render_off_thread(build)


def render_note(work):
    """The transparent "AI-generated" note PNG (the explainers' disclosure corner note, owner
    2026-10-23): the shared renderer — record.py burns it into every frame of its own output
    (the landing page plays the three recordings as their own videos) and compose.py overlays
    it on every frame of the product video, the title cards included. Same place and style as
    the cards' note was: right 28 px, bottom 20 px."""
    def build():
        from playwright.sync_api import sync_playwright

        path = work / "note.png"
        with sync_playwright() as pw:
            br = pw.chromium.launch()
            pg = br.new_page(viewport={"width": 360, "height": 80})
            pg.set_content('<body style="margin:0;background:transparent">'
                           f'<div style="display:inline-block;font-size:{NOTE_PX}px;'
                           f'font-family:{NOTE_FONT};color:{NOTE_COLOR};">{html.escape(NOTE_TEXT)}</div></body>')
            pg.locator("div").screenshot(path=str(path), omit_background=True)
            br.close()
        return path
    return _render_off_thread(build)


def compress_video(raw, out, segs, factors, work):
    """The repaired video: the wait spans of the plan sped up, the factor badge over them."""
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise RecordError("no ffmpeg on PATH")
    badges = render_badges(work, factors)
    badge_inputs = {f: i + 1 for i, f in enumerate(sorted(badges))}
    fc, last = compression_filter(segs, badge_inputs)
    cmd = [ffmpeg, "-y", "-v", "error", "-i", str(raw)]
    for f in sorted(badges):
        cmd += ["-i", str(badges[f])]
    cmd += ["-filter_complex", fc, "-map", f"[{last}]", "-an",
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", str(out)]
    subprocess.run(cmd, check=True, capture_output=True)


# ---------------- the mux ----------------

def ffprobe_seconds(path):
    ffprobe = shutil.which("ffprobe")
    if not ffprobe:
        raise RecordError("no ffprobe on PATH (ffmpeg is needed for the mux)")
    r = subprocess.run([ffprobe, "-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1",
                        str(path)], capture_output=True, text=True)
    if r.returncode or not r.stdout.strip():
        raise RecordError(f"ffprobe failed on {path}: {r.stderr.strip()}")
    return float(r.stdout.strip())


def build_narration(items, marks, total_s, path, lead=0.0, starts=None):
    """One narration WAV: every scene's audio at its planned position (the video's head before
    scene 1 shifts every scene by `lead`), silence where no scene plays, so the audio plan is
    fixed and a late scene drifts against it. `starts` gives the scene positions of a repaired
    (wait-compressed) timeline directly; the default is the narration schedule."""
    rate = None
    for _, wav, _ in items:
        with wave.open(str(wav), "rb") as w:
            if (w.getnchannels(), w.getsampwidth()) != (1, 2):
                raise RecordError(f"{wav.name}: the narration must be mono 16-bit (tts.py writes that)")
            rate = w.getframerate() if rate is None else rate
            if w.getframerate() != rate:
                raise RecordError(f"{wav.name}: rate {w.getframerate()} != {rate}")
    buf = bytearray(int(total_s * rate) * 2)
    for i, (sid, wav, _) in enumerate(items):
        with wave.open(str(wav), "rb") as w:
            data = w.readframes(w.getnframes())
        off = int((starts[i] if starts else lead + marks[0] + sum(item[2] for item in items[:i])) * rate) * 2
        end = min(len(buf), off + len(data))
        if end > off:
            buf[off:end] = data[:end - off]
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(bytes(buf))


def mux(video, narration, out, total_s, note=None):
    """The final MP4: the (repaired) video and its narration. The note (the "AI-generated"
    disclosure PNG of render_note) overlays every frame when given: the landing page plays the
    recordings as their own videos, so the recorder burns the note itself — the same PNG at the
    same place and style compose.py overlays it on the product video."""
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise RecordError("no ffmpeg on PATH")
    tmp = out.with_name(out.name + ".tmp.mp4")
    cmd = [ffmpeg, "-y", "-v", "error", "-i", str(video), "-i", str(narration)]
    if note is not None:
        cmd += ["-i", str(note), "-filter_complex",
                f"[0:v][2:v]overlay=x=main_w-overlay_w-{NOTE_MARGIN_PX}"
                f":y=main_h-overlay_h-{NOTE_BOTTOM_PX},format=yuv420p[v]",
                "-map", "[v]", "-map", "1:a"]
    cmd += ["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart", "-t", f"{total_s:.3f}",
            str(tmp)]
    subprocess.run(cmd, check=True, capture_output=True)
    os.replace(tmp, out)


# ---------------- the recording ----------------

SEED_JS = """
async ({projects, session}) => {
  await useProject(await B.newProject("Demo", projects, null));
  const folder = session + "/Audio Files";
  const scan = await B.tracksScan([folder], "");
  if (!scan.files.length) throw new Error("no audio files in " + folder);
  const roles = scan.files.map(f => f.stem === "new take" ? "kick" : f.role);
  await flushEvents();
  const r = await B.tracksAdd(S.sid, scan.files.map(f => f.path), roles,
                              scan.files.map(f => f.note), false, null);
  if (r.rejected && r.rejected.length) {
    throw new Error("tracks_add rejected " + r.rejected[0].name + ": " + r.rejected[0].reason);
  }
  await initSessions();
  await loadProject();
  startAnalysis(null);
  await new Promise((res, rej) => {
    const t0 = Date.now();
    const iv = setInterval(() => {
      if (typeof analysisRunning === "undefined" || !analysisRunning) { clearInterval(iv); res(true); }
      else if (Date.now() - t0 > 300000) { clearInterval(iv); rej(new Error("the demo analysis did not finish in 300 s")); }
    }, 200);
  });
  await loadProject();
  const hits = S.order.reduce((a, n) => a + (S.events[n] || []).length, 0);
  if (!hits) throw new Error("the demo analysis found no hits");
  return { sid: S.sid, tracks: S.order.length, hits };
}
"""


def seed_demo(br, url, env):
    """The demo session of part 1 (project Demo, the 13 demo WAVs, Analyze all), built in a
    video-less context before the recording page exists. Returns the session id."""
    with br.new_context() as seed_ctx:
        pg = seed_ctx.new_page()
        pg.goto(url + "/")
        pg.wait_for_function(READY, timeout=60000)
        out = pg.evaluate(SEED_JS, {"projects": env["projects"], "session": env["session"]})
    print(f"seed: {out['tracks']} tracks, {out['hits']} hits")
    return out["sid"]


def overlay_js(sid=None):
    """The recording overlay. A seeded run boots into the seeded session (dsg_sid), an
    unseeded one starts with no project (part 1 creates its own)."""
    line = (f"localStorage.setItem('dsg_sid', {json.dumps(sid)});" if sid
            else "localStorage.removeItem('dsg_sid');")
    return OVERLAY_JS.replace("localStorage.removeItem('dsg_sid');", line, 1)


def record(root, scenario, items, out, work, keep, check_model=True):
    """The live run: bridge, page, scenes, repair, mux. Returns (drifts, factors, seconds);
    the drifts are the per-scene drifts of the written video (each scene's span over its
    narration length, after the repair), the factors the wait-compression per scene
    (1.0 when the scene needed none)."""
    from playwright.sync_api import sync_playwright

    scenes = scenario["scenes"]
    durations = [secs for _, _, secs in items]
    raw_vp = scenario.get("viewport")
    vp_w = int(raw_vp["width"]) if raw_vp else 1920
    vp_h = int(raw_vp["height"]) if raw_vp else 1080
    dsf = int(scenario.get("device_scale_factor", 2))   # the media plan: 1920x1080 at 2x device scale
    home = work / "home"
    (home / "projects").mkdir(parents=True, exist_ok=True)
    env = {"work": str(work), "home": str(home), "projects": str(home / "projects"),
           "session": str(work / "session"), "demo": str(CACHE / "demo")}
    build_session_folder(CACHE / "demo", work / "session")
    write_daw_midi(work / "tempo.mid")   # the Import Tempo Map scene picks this DAW file
    write_one_shot(work, CACHE / "demo")   # the Layer scene picks this one-shot
    raw = work / (out.stem + "-raw.webm")
    proc = None
    video = None
    bridge_env = {"DRUMSPIKE_HOME": env["home"], "DRUMSPIKE_PROJECTS": env["projects"],
                  "DRUMSPIKE_AUDIO": "null", "DRUMSPIKE_MASTER_GAIN": "0"}
    if scenario.get("plugins"):
        # The Detector plug-in scenes: the example plugin in the recording home's Detectors
        # folder, and the bridge runs with that home, so the dialogs show the neutral
        # "~/Library/..." form (shown_path) and no host path reaches the video.
        install_plugin(home, CACHE / "demo")
        bridge_env["HOME"] = str(home)
        bridge_env["DRUMSPIKE_DETECTORS_DIR"] = str(home / "Library" / "Application Support" / "Drum Spike" / "Detectors")
    try:
        proc, url = start_bridge(root, bridge_env)
        with sync_playwright() as pw:
            br = pw.chromium.launch()
            sid = seed_demo(br, url, env) if scenario.get("seed") == "demo" else None
            place_session_files(scenario, env)
            ctx = br.new_context(viewport={"width": vp_w, "height": vp_h}, device_scale_factor=dsf,
                                 record_video_dir=str(work / "raw"),
                                 record_video_size={"width": vp_w, "height": vp_h})
            t_video0 = time.monotonic()   # the video starts with the page
            pg = ctx.new_page()
            vid = pg.video
            # #403: the app's one native dialog is the re-analyze overwrite confirm
            # (startAnalysisGo, keptTotal === 0): a scene that clicks Re-analyze on a track
            # with unedited events means yes. Without a handler Playwright auto-dismisses
            # the dialog and the re-analyze silently does not run.
            pg.on("dialog", lambda d: d.accept())
            pg.add_init_script(overlay_js(sid))
            pg.goto(url + "/")
            pg.wait_for_function(READY, timeout=60000)
            pg.wait_for_timeout(500)   # the empty state paints before scene 1
            lead = time.monotonic() - t_video0   # the video's head before scene 1: the narration shifts by it
            try:
                marks, drifts, waits = run_scenes(pg, scenes, durations, env, check_model=check_model)
                pg.evaluate(CAP_JS, "")   # the caption fades before the cut
                pg.wait_for_timeout(600)
            finally:
                ctx.close()
                br.close()
            try:
                video = vid.path()   # the file is on disk now
            except Exception:
                video = None
    finally:
        if proc:
            proc.terminate()
            try:
                proc.wait(10)
            except Exception:
                proc.kill()
    if not video:
        raise RecordError("the browser wrote no video")
    os.replace(video, raw)
    total = ffprobe_seconds(raw)
    try:
        factors, reductions = wait_speeds(drifts, waits)
    except RecordError as e:
        table = "; ".join(f"{i}: drift {d * 1000:+.0f} ms, wait {sum(e2 - s2 for s2, e2 in waits[i]):.2f} s"
                          for i, d in enumerate(drifts))
        raise RecordError(f"{e}; every scene: {table}") from e
    starts = compressed_starts(marks[0], lead, durations, drifts, reductions)
    final_drifts = [drifts[i] - reductions[i] for i in range(len(durations))]
    if any(f > 1.0 for f in factors):
        segs = compression_segments(waits, lead, factors, total)
        repaired = work / (out.stem + "-repaired.mp4")
        compress_video(raw, repaired, segs, factors, work)
        os.replace(repaired, raw)
        total = ffprobe_seconds(raw)
    build_narration(items, marks, total, work / "narration.wav", lead, starts)
    mux(raw, work / "narration.wav", out, total, note=render_note(work))
    if not keep:
        shutil.rmtree(work, ignore_errors=True)
    return final_drifts, factors, total


OVERLAY_JS = """localStorage.removeItem('dsg_sid');
  addEventListener('DOMContentLoaded', () => {
    const o = document.createElement('div'); o.id = '__keys';
    // bottom 72 clears the AI-generated note (right 28 / bottom 20): the mux burns the note
    // over every frame of that corner, so the badge sits above it to keep the note readable.
    o.style.cssText = 'position:fixed;right:28px;bottom:72px;z-index:99999;font:600 22px Inter,Helvetica,sans-serif;'
      + 'color:#14161a;background:#ff9f43;border-radius:10px;padding:8px 16px;opacity:0;transition:opacity .25s';
    const c = document.createElement('div'); c.id = '__cap';
    c.style.cssText = 'position:fixed;left:50%;bottom:28px;transform:translateX(-50%);z-index:99999;'
      + 'font:500 22px Inter,Helvetica,sans-serif;color:#f1f4f9;background:rgba(20,22,26,.86);border-radius:10px;padding:8px 18px;opacity:0;transition:opacity .25s';
    document.body.append(o, c);
    window.__dsShowKey = (label, ms) => { o.textContent = label; o.style.opacity = 1;
      clearTimeout(o._t); o._t = setTimeout(() => o.style.opacity = 0, ms); };
  });"""


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("scenario", type=Path, help="tools/media/scenarios/<name>.json")
    ap.add_argument("--out", type=Path, help="the MP4 (default: the scenario's out)")
    ap.add_argument("--tts-out", type=Path, help="the narration dir of tools/media/tts.py (default: the scenario's)")
    ap.add_argument("--allow-model-text", action="store_true",
                    help="skip the no-model-text check (only for the test recording on pre-#294 main,"
                         " whose fallback detector shows the model texts)")
    ap.add_argument("--keep", action="store_true", help="keep the work dir and the raw video")
    args = ap.parse_args(argv)
    out = work = None
    try:
        scenario = parse_scenario(json.loads(args.scenario.read_text(encoding="utf-8")))
        problems = caption_problems(scenario["scenes"], keymap_commands((ROOT / "static" / "keymap.js").read_text(encoding="utf-8")))
        if problems:
            for p in problems:
                print(f"FAIL: {p}", file=sys.stderr)
            return 1
        ids = [sc["id"] for sc in scenario["scenes"]]
        tts_dir = args.tts_out or expand(scenario.get("narration", {}).get("tts_out", ""), {})
        if not tts_dir or Path(tts_dir) == Path("."):
            raise RecordError("no --tts-out and no narration.tts_out in the scenario")
        items = load_narration(Path(tts_dir), ids)
        out = args.out or expand(scenario.get("out", str(CACHE / "rec" / (args.scenario.stem + ".mp4"))), {})
        work = out.parent / "work"
        prepare_work(work, args.keep)
        ensure_demo_session(CACHE / "demo")
        drifts, factors, total = record(ROOT, scenario, items, out, work, args.keep,
                                        check_model=not args.allow_model_text)
        print(f"{args.scenario.stem}: {len(scenario['scenes'])} scenes, {total:.1f} s -> {out}")
        for (sid, _, secs), drift, factor in zip(items, drifts, factors):
            line = f"  {sid}: planned {secs:.2f} s, drift {drift * 1000:+.0f} ms"
            line += f", {speed_label(factor)} wait speed" if factor > 1.0 else ", 1x"
            print(line)
        bad = drift_problems(scenario["scenes"], drifts)
        if bad:
            for b in bad:
                print(f"FAIL: {b}", file=sys.stderr)
            return 1
        return 0
    except RecordError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        save_failed_video(out, work)
        return 1


def save_failed_video(out, work):
    """Keep the raw video of a failed run as <out>.failed.webm (the coordinator can look at it)."""
    if not out:
        return
    raws = sorted((work / "raw").glob("*.webm")) if (work / "raw").is_dir() else []
    if not raws:
        return
    failed = out.parent / (out.stem + ".failed.webm")
    try:
        os.replace(raws[-1], failed)
        print(f"raw video of the failed run: {failed}", file=sys.stderr)
    except OSError:
        pass


def prepare_work(work, keep):
    """A clean work dir per run; --keep leaves what is there."""
    if keep:
        work.mkdir(parents=True, exist_ok=True)
    else:
        shutil.rmtree(work, ignore_errors=True)
        work.mkdir(parents=True)


if __name__ == "__main__":
    sys.exit(main())
