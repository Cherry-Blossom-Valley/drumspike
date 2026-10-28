"""#387: detector plug-ins in the app — the missing-plugin ask on a re-analyze (the design
section 6) and the Detector select of the Inspector with the licence dialog before the
first use (section 7). The example plugin tests/fixtures/plugins/json_hits.py is installed
into the bridge's Detectors folder (DRUMSPIKE_DETECTORS_DIR, conftest).

Every wait is on the page's own state; no sleeps."""
import json
import shutil
from pathlib import Path

from app_support import inspect_track, open_project

FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "plugins"
PLUGIN_ID = "json-hits"


def doc_of(folder):
    return json.loads(next(Path(folder).glob("*.drumspike")).read_text())


def install_plugin(home, times):
    """The example plugin as a real install: manifest, executable (chmod +x), times.json."""
    folder = home / "detectors" / PLUGIN_ID
    folder.mkdir(parents=True, exist_ok=True)
    shutil.copy(FIXTURE / "json_hits.py", folder / "json_hits.py")
    (folder / "json_hits.py").chmod(0o755)
    (folder / "times.json").write_text(json.dumps(times))
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


def test_missing_plugin_asks_and_analyzes_builtin(app, make_project):
    """A project whose track names a missing plugin opens and shows the stored hits; a
    re-analyze asks, and 'built-in' runs the DSP detector (the design section 6)."""
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    doc = doc_of(folder)
    doc["tracks"]["Kick In"].setdefault("params", {})["detector"] = "gone-plugin"
    next(Path(folder).glob("*.drumspike")).write_text(json.dumps(doc, indent=1))
    sid = open_project(app, folder)
    app.click("#analyzeAllBtn")   # one track: the same flow as Track > Analyze
    modal = app.locator("#pluginMissingModal")
    modal.wait_for(state="visible", timeout=15000)
    assert "gone-plugin" in app.locator("#pmMsg").inner_text()
    app.click("#pmBuiltin")
    app.wait_for_function("() => !analysisRunning", timeout=90000)
    app.wait_for_function("() => (S.events['Kick In'] || []).length > 0", timeout=30000)
    st = app.evaluate("s => S.events['Kick In'].length", sid)
    assert st == 3   # the synthetic kick at 0.5, 1.5 and 2.5 s: the built-in detector ran
    p = app.evaluate("() => S.tracks['Kick In'].params.detector")
    assert p == "builtin"


def test_detector_select_runs_the_plugin(app, make_project):
    """The Detector select lists the installed plugin; the licence dialog comes before the
    first use; the re-analyze then returns the plugin's hits (the gate's second test)."""
    install_plugin(app.home, {"kick": [0.5, 1.5, 2.5]})
    folder = make_project(tracks=(("Kick In", "kick", 36),))
    open_project(app, folder)
    ins = inspect_track(app, "Kick In")
    sel = ins.locator(".pDetector")
    sel.wait_for(state="visible", timeout=15000)
    app.wait_for_function(
        "() => [...document.querySelectorAll('#insParams .pDetector option')].some(o => o.value === '%s')" % PLUGIN_ID
    )
    assert sel.input_value() == ""   # Built-in is the current detector
    sel.select_option(PLUGIN_ID)
    licence = app.locator("#pluginLicenceModal")
    licence.wait_for(state="visible", timeout=15000)   # the licence before the first use
    assert "json_hits.py" in app.locator("#plIntro").inner_text()
    app.click("#plAccept")
    licence.wait_for(state="hidden")
    app.click("#analyzeAllBtn")
    app.wait_for_function("() => !analysisRunning", timeout=90000)
    app.wait_for_function("() => (S.events['Kick In'] || []).length > 0", timeout=30000)
    times = app.evaluate("() => S.events['Kick In'].map(e => e.t)")
    assert len(times) == 3
    for t, want in zip(sorted(times), [0.5, 1.5, 2.5]):
        assert abs(t - want) < 0.01, (times, want)   # the plugin's hits, refined to the attack
    assert app.evaluate("() => S.tracks['Kick In'].params.detector") == PLUGIN_ID
