"""#577 (Gate line 6): the standalone app never reads or writes the defaults file or the presets folder of the plugin.

(a) No source of the standalone code names the folder or the files of the plugin. (b) The bridge (the standalone service) runs a session with a
temporary HOME that holds sentinel files in the plugin's folder: tracks, notes, kit presets, settings, a save. Afterwards the folder is as it was
(bytes, mtime, list) and no other file of the plugin's names exists below HOME. The comparison is proved able to fail: its unit tests plant a changed
byte, a changed mtime, a new file and a missing one (the fault run `touch`). (c) The positive control (a plugin run that writes the folder makes the
comparison fire) is the stage `defaults-standalone-control` of plugins/ara/check.py. The test never uses the real HOME."""
import json
import os
import re
import subprocess
import urllib.request
from pathlib import Path

import pytest

from plugin_files import FOLDER, changed, plugin_folder, snapshot

ROOT = Path(__file__).resolve().parent.parent


def test_the_comparison_fires_on_every_kind_of_touch(tmp_path):
    f = plugin_folder(tmp_path)
    (f / "Presets").mkdir(parents=True)
    (f / "defaults.json").write_text('{"v":1}')
    (f / "Presets" / "S.json").write_text("{}")
    base = snapshot(f)
    assert changed(base, snapshot(f)) == [], "nothing touched: nothing named"
    (f / "defaults.json").write_text('{"v":2}')
    assert changed(base, snapshot(f)) == ["content: defaults.json"]
    (f / "defaults.json").write_text('{"v":1}')
    os.utime(f / "defaults.json", ns=(1, 1))
    assert changed(base, snapshot(f)) == ["mtime: defaults.json"]
    (f / "new.json").write_text("x")
    assert "new: new.json" in changed(base, snapshot(f))
    (f / "Presets" / "S.json").unlink()
    assert "missing: Presets/S.json" in changed(base, snapshot(f))


def test_no_source_of_the_standalone_code_names_the_plugin_folder_or_its_files():
    names = [FOLDER, "defaults.bad.json"]
    hits = []
    for top in ("crates", "apps"):
        for p in (ROOT / top).rglob("*"):
            if p.suffix not in (".rs", ".js", ".py", ".toml", ".json", ".html") or "target" in p.parts or "node_modules" in p.parts or not p.is_file():
                continue
            text = p.read_text(errors="ignore")
            if any(n in text for n in names) or re.search(r"Application Support/[\"']?DrumSpike ARA", text):
                hits.append(str(p.relative_to(ROOT)))
    assert hits == [], hits


def bridge_exe():
    exe = Path(os.environ.get("DRUMSPIKE_BRIDGE") or (Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target") / "debug" / "ds-bridge"))
    if not exe.is_file():
        if os.environ.get("DRUMSPIKE_REQUIRE_APP") == "1":
            pytest.fail(f"DRUMSPIKE_REQUIRE_APP=1 and ds-bridge is not built (looked for {exe})", pytrace=False)
        pytest.skip("ds-bridge is not built")
    return exe


def call(url, cmd, **args):
    req = urllib.request.Request(f"{url}/invoke/{cmd}", data=json.dumps(args).encode(), headers={"Content-Type": "application/json"}, method="POST")
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read() or "null")


def test_a_standalone_session_leaves_the_plugin_folder_as_it_was(tmp_path):
    exe = bridge_exe()
    home = tmp_path / "home"
    folder = plugin_folder(home)
    (folder / "Presets").mkdir(parents=True)
    (folder / "defaults.json").write_text('{"v":1,"roles":{"snare":{"notes":[99]}}}')
    (folder / "Presets" / "Sentinel.json").write_text('{"v":1,"name":"Sentinel","roles":{}}')
    os.utime(folder / "defaults.json", ns=(1_000_000_000, 1_000_000_000))
    os.utime(folder / "Presets" / "Sentinel.json", ns=(1_000_000_000, 1_000_000_000))
    before = snapshot(folder)
    data = tmp_path / "data"
    env = dict(os.environ, HOME=str(home), DRUMSPIKE_HOME=str(data), DRUMSPIKE_PROJECTS=str(data / "projects"), DRUMSPIKE_AUDIO="null", DRUMSPIKE_MASTER_GAIN="0")
    proc = subprocess.Popen([str(exe), "--static", str(ROOT / "static"), "--exit-on-stdin-eof"], cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, env=env)
    try:
        m = re.search(r"http://127\.0\.0\.1:\d+", proc.stdout.readline())
        assert m, "the bridge did not start"
        url = m.group(0)
        import importlib.util
        import struct
        import wave
        spec = importlib.util.spec_from_file_location("ds_make_project", ROOT / "apps/desktop/selftest/make_project.py")
        builder = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(builder)
        src = tmp_path / "src"
        src.mkdir()
        specs = []
        for name, role, note in (("Kick In", "kick", 36), ("Snare Top", "snare", 38)):
            w = src / f"{name}.wav"
            x = [0] * (48_000 * 3)
            for i in range(24_000, 24_200):
                x[i] = 20_000
            with wave.open(str(w), "wb") as f:
                f.setnchannels(1); f.setsampwidth(2); f.setframerate(48_000); f.writeframes(struct.pack(f"<{len(x)}h", *x))
            specs.append((name, str(w), role, note))
        proj = builder.build_project(tmp_path / "projects", "Song", specs, "copied")
        sid = call(url, "project_open", path=str(proj))["sid"]
        call(url, "track_set_note", sid=sid, name="Snare Top", note=60)
        call(url, "kit_preset_save", name="Mine", rules=[{"pattern": "k", "role": "kick", "note": 35}])
        call(url, "kit_presets_list")
        call(url, "app_settings_set", key="inspector", value=True)
        call(url, "project_save", sid=sid, force=False)
    finally:
        proc.terminate()
        proc.wait(10)
    assert changed(before, snapshot(folder)) == []
    stray = [p for p in home.rglob("*") if folder not in p.parents and p != folder and (p.name == "defaults.json" or p.parent.name == "Presets")]
    assert stray == [], stray
    assert (data / "kit-presets" / "Mine.json").is_file(), "the session did write its own kit preset: the run was real"
