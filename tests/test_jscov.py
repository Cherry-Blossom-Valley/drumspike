"""Tests of the JS coverage gate (tests/jscov.py, #13): a node-ID run of one passing test is a
partial run and must not fail the floor; a full run still fails when a file is under the floor."""
import os
import subprocess
import sys
from types import SimpleNamespace

import pytest

import jscov
from helpers import ROOT

NODE_ID = "tests/e2e_app/test_render.py::test_fallback_without_webgl2"   # a `page` test: the gate sees coverage


def test_a_node_id_run_of_one_passing_test_exits_0_with_the_floor_at_85():
    """Red before the fix: the gate read a node-ID run as a full run and failed the floor."""
    env = dict(os.environ, DRUMSPIKE_JS_FAIL_UNDER="85")
    # the CI job's shard variables would deselect NODE_ID (shard "rest") and switch the gate off
    for k in ("DRUMSPIKE_E2E_SHARD", "DRUMSPIKE_JS_COV_OUT"):
        env.pop(k, None)
    r = subprocess.run([sys.executable, "-m", "pytest", NODE_ID, "-q", "--no-header",
                        "-p", "no:cacheprovider"],
                       cwd=ROOT, env=env, capture_output=True, text=True, timeout=900)
    assert r.returncode == 0, (r.stdout + r.stderr)[-4000:]


def _session(keyword="", markexpr="", file_or_dir=()):
    """A fake session for jscov.pytest_sessionfinish; the terminal reporter collects its lines."""
    lines = []
    tr = SimpleNamespace(write_line=lambda line, red=False: lines.append(line))
    return SimpleNamespace(
        config=SimpleNamespace(option=SimpleNamespace(keyword=keyword, markexpr=markexpr,
                                                      file_or_dir=list(file_or_dir)),
                               pluginmanager=SimpleNamespace(get_plugin=lambda name: tr)),
        exitstatus=0,
        lines=lines), lines


@pytest.fixture(autouse=True)
def _no_shard(monkeypatch):
    """A CI shard run does not gate (jscov.pytest_sessionfinish): these tests check the gate itself."""
    monkeypatch.delenv("DRUMSPIKE_E2E_SHARD", raising=False)


def test_a_full_run_still_fails_when_a_file_is_under_the_floor(monkeypatch):
    monkeypatch.setitem(jscov.JS_COV, "report", {"files": {"app.js": {"percent": 84.0}}})
    session, lines = _session()
    jscov.pytest_sessionfinish(session, 0)
    assert session.exitstatus == pytest.ExitCode.TESTS_FAILED
    assert any("FAIL: coverage" in ln and "app.js" in ln for ln in session.lines)


def test_a_full_run_shows_the_fail_line_when_the_floor_is_raised_to_101(monkeypatch):
    """Even a healthy report fails when DRUMSPIKE_JS_FAIL_UNDER=101: the gate is live."""
    monkeypatch.setattr(jscov, "JS_FAIL_UNDER", 101.0)
    monkeypatch.setitem(jscov.JS_COV, "report", {"files": {"app.js": {"percent": 90.0}}})
    session, lines = _session()
    jscov.pytest_sessionfinish(session, 0)
    assert session.exitstatus == pytest.ExitCode.TESTS_FAILED
    assert any("FAIL: coverage" in ln and "app.js" in ln for ln in session.lines)


def test_the_gate_stays_quiet_for_a_node_id_run(monkeypatch):
    """The fix itself: a command line that names a node ID (or a file) does not gate."""
    monkeypatch.setitem(jscov.JS_COV, "report", {"files": {"app.js": {"percent": 1.0}}})
    session, lines = _session(file_or_dir=[NODE_ID])
    jscov.pytest_sessionfinish(session, 0)
    assert session.exitstatus == 0 and not any("FAIL" in ln for ln in session.lines)
    session, lines = _session(file_or_dir=["tests/e2e_app/test_render.py"])
    jscov.pytest_sessionfinish(session, 0)
    assert session.exitstatus == 0 and not any("FAIL" in ln for ln in session.lines)
    session, lines = _session(keyword="ready")
    jscov.pytest_sessionfinish(session, 0)
    assert session.exitstatus == 0 and not any("FAIL" in ln for ln in session.lines)
