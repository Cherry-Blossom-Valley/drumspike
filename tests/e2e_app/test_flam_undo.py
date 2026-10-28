"""P4-13 stage B fix round: one undo step after a stored re-pick that holds flam hits (the rule on). The shared bridge of
the e2e suite pins the flam rule off (the frozen references), so this module starts its own bridge without the pin: the
analysis adds the flam hits, a stored re-pick (the Sensitivity slider) runs the rule again, undo gives the hits and their
`flam` tags from before the re-pick back, redo the re-pick's."""
import os
import re
import subprocess
from types import SimpleNamespace

import numpy as np
import pytest

from app_support import inspect_track, open_project, wait_until
from conftest import READY, WARM_GOTO_MS, bridge_binary, start_bridge
from helpers import ROOT, SR, write_wav
from test_editor import service_events
from test_histograms import settled
from test_subtracks import analyse, has_model, js


@pytest.fixture(scope="module")
def flam_bridge(tmp_path_factory):
    exe = bridge_binary()
    if not exe.is_file():
        pytest.skip("ds-bridge not built")
    home = tmp_path_factory.mktemp("dshome_flam")
    env = dict(os.environ, DRUMSPIKE_HOME=str(home), DRUMSPIKE_PROJECTS=str(home / "projects"),
               DRUMSPIKE_AUTOSAVE_SECONDS="1", DRUMSPIKE_AUDIO="null", DRUMSPIKE_MASTER_GAIN="0")
    env.pop("DRUMSPIKE_FLAM_RULE", None)                      # the rule on, as in the product
    proc = start_bridge(exe, env)
    m = re.search(r"http://127\.0\.0\.1:\d+", proc.stdout.readline())
    if not m:
        proc.kill()
        raise RuntimeError("ds-bridge did not start")
    yield SimpleNamespace(url=m.group(0), proc=proc, home=home)
    proc.terminate()
    proc.wait(10)


@pytest.fixture(scope="module")
def warm_page(browser, flam_bridge, browser_context_args):
    """The conftest's `warm_page` on this module's bridge (the `app` fixture reads it)."""
    ctx = browser.new_context(**browser_context_args)
    try:
        pg = ctx.new_page()
        pg.goto(flam_bridge.url + "/", timeout=WARM_GOTO_MS)
        pg.wait_for_function(READY, timeout=WARM_GOTO_MS)
    finally:
        ctx.close()
    return flam_bridge


def snare(rng, decay):
    """The DRT-like snare of the Rust tests: a 190 Hz body (40 ms decay) and noise (`decay` s), 250 ms long."""
    t = np.arange(SR // 4) / SR
    return (np.sin(2 * np.pi * 190 * t) * np.exp(-t / 0.04) + 0.6 * rng.standard_normal(len(t)) * np.exp(-t / decay)) / 1.6


def flam_wav(path, seed=3):
    """A backbeat every second at -6 dB from 1 s; at 3, 6 and 9 s a close pair (a short stroke at -8 dB, the main stroke
    30 ms later): the model keeps the first, the flam rule adds the main stroke. The decay is 60 ms (a slow top mic)."""
    rng = np.random.default_rng(seed)
    x = np.zeros(12 * SR)

    def place(sig, at, db):
        a = int(round(at * SR))
        x[a:a + len(sig)] += 10 ** (db / 20) * sig

    for i in range(10):
        t = 1.0 + i
        if i % 3 == 2:
            place(snare(rng, 0.008), t, -8.0)
            place(snare(rng, 0.06), t + 0.03, -6.0)
        else:
            place(snare(rng, 0.06), t, -6.0)
    return write_wav(path, x + 1e-4 * rng.standard_normal(len(x)))


def tagged(page, sid):
    return [(round(e["t"] * SR), bool(e.get("flam"))) for e in service_events(page, sid, "Snare Top")]


def test_undo_and_redo_of_a_stored_re_pick_with_flam_hits(app, make_project, tmp_path):
    if not has_model(app):
        pytest.skip("the flam rule needs the analysis with the model")
    src = tmp_path / "src_flam_undo"
    src.mkdir()
    specs = [("Snare Top", flam_wav(src / "Snare Top.wav"), "snare", 38, None)]
    sid = open_project(app, make_project.builder.build_project(tmp_path / "projects_flam_undo", "FlamUndo", specs, "copied"))
    analyse(app)
    before = tagged(app, sid)
    assert sum(f for _, f in before) == 3, before                 # the rule is on: the analysis added the main strokes
    assert inspect_track(app, "Snare Top").locator(".insFlags").inner_text().strip() == "Flams: 3 hits added"
    inspect_track(app, "Snare Top").locator(".pSens").fill("100")
    settled(app)
    js(app, "() => flushEvents()")
    stored = tagged(app, sid)
    assert sum(f for _, f in stored) == 2, stored   # the rule ran again; the model now picks one main stroke itself
    assert stored != before, "the re-pick changed the hit set"
    js(app, "() => undo()")
    js(app, "() => flushEvents()")
    wait_until(lambda: tagged(app, sid) == before, msg="undo: the hits and the flam tags before the re-pick")
    assert [(round(e["t"] * SR), bool(e.get("flam"))) for e in js(app, "() => S.events['Snare Top']")] == before
    js(app, "() => redo()")
    js(app, "() => flushEvents()")
    wait_until(lambda: tagged(app, sid) == stored, msg="redo: the re-pick's hits and flam tags")
