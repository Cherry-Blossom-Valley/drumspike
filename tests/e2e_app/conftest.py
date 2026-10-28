"""App e2e fixtures (P3-7a, G4): the page in Chromium against `ds-bridge`, the debug binary of
crates/app that serves static/ and the app commands (`POST /invoke/<command>`) on 127.0.0.1. The page
takes the IPC backend over the bridge transport (static/backend.js), so these tests run the code
path of the app (B.app, the byte layouts, the Rust commands) on both hosts without a WebView.

Without the binary the tests skip with the reason; DRUMSPIKE_REQUIRE_APP=1 turns the skip into a
failure. Binary: $DRUMSPIKE_BRIDGE, else <target>/debug/ds-bridge (target = $CARGO_TARGET_DIR or
./target); build it with `cargo build -p drumspike-devtools --bin ds-bridge`.
Projects are written by apps/desktop/selftest/make_project.py into temp folders.

X-3 (P3-9b): the first Page.goto of a session could take more than 30 s on a loaded host (cold Chromium,
cold bridge, first compile of the page). The session fixture `warm_page` loads the page once with a long
timeout before the first test. A test that fails keeps a Playwright trace (screenshots, DOM snapshots,
network) in $DRUMSPIKE_TRACE_DIR, default test-results/e2e_app-traces/ (git-ignored); open it with
`uv run playwright show-trace <file>.zip`."""
import importlib.util
import json
import os
import re
import subprocess
import time
from pathlib import Path
from types import SimpleNamespace

import pytest

import app_support
from helpers import ROOT, SR, synth_track, write_wav

WARM_GOTO_MS = 180_000   # the first load of the session: cold browser and bridge on a loaded host
READY = "typeof S !== 'undefined' && typeof B !== 'undefined' && B.app === true && S.loaded > 0"
TRACE_DIR = Path(os.environ.get("DRUMSPIKE_TRACE_DIR") or ROOT / "test-results" / "e2e_app-traces")

SKIP = ("ds-bridge not built: run 'cargo build -p drumspike-devtools --bin ds-bridge' (or set DRUMSPIKE_BRIDGE); "
        "DRUMSPIKE_REQUIRE_APP=1 makes this a failure")


SHARD_FILE = Path(__file__).resolve().parent.parent / "e2e_shards.json"


def shard_selects(name, shard, files):
    """Is the test file `name` (e.g. test_menus.py) in `shard`? `files` is the sorted list of all e2e_app test files.
    Shard forms: "<i>/<n>" (1-based, round robin over the sorted list), or a name from tests/e2e_shards.json
    ({"x86": [files]}): that list is the shard, every other file belongs to the shard "rest" (so a new file is
    never lost). Unknown name: ValueError."""
    if "/" in shard:
        i, n = (int(x) for x in shard.split("/"))
        if not 1 <= i <= n:
            raise ValueError(f"DRUMSPIKE_E2E_SHARD={shard}: need 1 <= i <= n")
        return files.index(name) % n == i - 1
    named = json.loads(SHARD_FILE.read_text())
    if shard == "rest":
        return all(name not in v for v in named.values())
    if shard not in named:
        raise ValueError(f"DRUMSPIKE_E2E_SHARD={shard}: not 'rest' and not in {SHARD_FILE.name} ({sorted(named)})")
    return name in named[shard]


def pytest_collection_modifyitems(config, items):
    for item in items:
        if "/e2e_app/" in str(item.fspath):
            item.add_marker(pytest.mark.e2e_app)
    shard = os.environ.get("DRUMSPIKE_E2E_SHARD")
    if not shard:
        return
    files = sorted(p.name for p in Path(__file__).parent.glob("test_*.py"))
    keep, drop = [], []
    for item in items:
        e2e = "/e2e_app/" in str(item.fspath)
        (keep if not e2e or shard_selects(Path(str(item.fspath)).name, shard, files) else drop).append(item)
    if drop:
        config.hook.pytest_deselected(items=drop)
        items[:] = keep


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item, call):
    """Keep the report of each phase on the item: the fixture `app` reads it to keep a trace on failure."""
    outcome = yield
    rep = outcome.get_result()
    setattr(item, f"rep_{rep.when}", rep)


def trace_path(nodeid):
    name = re.sub(r"[^A-Za-z0-9_.-]+", "_", nodeid.split("/")[-1])[:150]
    return TRACE_DIR / f"{name}.zip"


def save_trace(context, nodeid):
    """Stop the trace of the context and write it; prints the path for the log. Never raises."""
    path = trace_path(nodeid)
    try:
        TRACE_DIR.mkdir(parents=True, exist_ok=True)
        context.tracing.stop(path=str(path))
        print(f"\nPlaywright trace of the failed test: {path}")
        return path
    except Exception as e:   # a closed context must not hide the test's own error
        print(f"\nno Playwright trace for {nodeid}: {e}")
        return None


def bridge_binary():
    env = os.environ.get("DRUMSPIKE_BRIDGE")
    if env:
        return Path(env)
    return Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target") / "debug" / "ds-bridge"


def start_bridge(exe, env):
    """The one way the e2e fixtures start `ds-bridge` (#4): with `--exit-on-stdin-eof` and its
    stdin on an open pipe. The pipe dies with the pytest process, so a SIGKILLed run (no teardown)
    gives the bridge an EOF and it ends; the teardown (`terminate()` + `wait`) stays as it was.
    Never close `proc.stdin` while the tests run."""
    return subprocess.Popen([str(exe), "--exit-on-stdin-eof"], cwd=ROOT, stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)


def _builder():
    spec = importlib.util.spec_from_file_location("ds_make_project", ROOT / "apps/desktop/selftest/make_project.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture(scope="session")
def bridge(tmp_path_factory):
    """The running bridge: .url, .proc."""
    exe = bridge_binary()
    if not exe.is_file():
        if os.environ.get("DRUMSPIKE_REQUIRE_APP") == "1":
            pytest.fail(f"DRUMSPIKE_REQUIRE_APP=1 and {SKIP.split(' (')[0]} (looked for {exe})", pytrace=False)
        pytest.skip(SKIP)
    # P3-4a: the bridge keeps recent.json in its own home folder and autosaves dirty projects every second
    # P3-5b: the bridge never plays to a real device: the null device and master gain 0 (also if a caller forgot)
    home = tmp_path_factory.mktemp("dshome")
    # P4-12 stage 2: the frozen MIDI and spike references of the export tests are the retired engine's refinement
    # (version 1; the reach rule of version 2 gave the same events here, the first-motion back-track of version 3
    # does not), so the bridge's analyses run version 1 (as check_service.py and exit_flow.py)
    env = dict(os.environ, DRUMSPIKE_HOME=str(home), DRUMSPIKE_PROJECTS=str(home / "projects"),
               DRUMSPIKE_AUTOSAVE_SECONDS="1", DRUMSPIKE_AUDIO="null", DRUMSPIKE_MASTER_GAIN="0",
               DRUMSPIKE_REFINE_RULE="1", DRUMSPIKE_ROLL_RULE="0",   # P4-19: the frozen references have no roll fill
               DRUMSPIKE_FLAM_RULE="0",   # P4-13 stage B: nor the flam rule
               DRUMSPIKE_KICK_RESCUE="0",   # P4-10 stage 5: nor the kick rescue
               DRUMSPIKE_SNARE_RESCUE="0",   # P4-10 stage 7: nor the snare rescue
               DRUMSPIKE_DETECTORS_DIR=str(home / "detectors"))  # #387: the tests install their plugins here
    proc = start_bridge(exe, env)
    line = proc.stdout.readline()
    m = re.search(r"http://127\.0\.0\.1:\d+", line)
    if not m:
        proc.kill()
        raise RuntimeError(f"ds-bridge did not start: {line!r} {proc.stdout.read()}")
    yield SimpleNamespace(url=m.group(0), proc=proc, home=home)
    proc.terminate()
    proc.wait(10)


@pytest.fixture(scope="session", autouse=True)
def warm_page(request, bridge, browser_context_args):
    """X-3: load the page once per session with a long timeout, so that the first test's Page.goto finds a
    warm browser and a warm bridge (autouse: also the tests that load the page themselves, test_tiles.py).
    Records the time in bridge.warm_s. #4: DRUMSPIKE_NO_WARM_PAGE=1 (the child pytest of
    test_bridge_stdin_exit.py) only holds the bridge: no browser, no page load."""
    if os.environ.get("DRUMSPIKE_NO_WARM_PAGE") == "1":
        return bridge   # #4: the child pytest only holds the bridge; no browser, no page load
    browser = request.getfixturevalue("browser")
    t0 = time.monotonic()
    ctx = browser.new_context(**browser_context_args)
    try:
        pg = ctx.new_page()
        pg.goto(bridge.url + "/", timeout=WARM_GOTO_MS)
        pg.wait_for_function(READY, timeout=WARM_GOTO_MS)
    finally:
        ctx.close()
    bridge.warm_s = time.monotonic() - t0
    return bridge


@pytest.fixture
def make_project(tmp_path):
    """make(tracks=(("Kick In", "kick", 36), ...), mode="copied", seconds=4.0, sr=SR) -> project folder.
    Each track is a synthetic kick at 0.5 s, 1.5 s and 2.5 s (a WAV file of the given rate)."""
    mod = _builder()
    count = [0]

    def make(tracks=(("Kick In", "kick", 36), ("Snare Top", "snare", 38)), mode="copied", seconds=4.0, sr=SR):
        count[0] += 1
        src = tmp_path / f"src{count[0]}"
        src.mkdir()
        specs = []
        for spec in tracks:
            name, role, note = spec[0], spec[1], spec[2]
            x, _ = synth_track([0.5, 1.5, 2.5], seconds, sr=sr)
            specs.append((name, write_wav(src / f"{name}.wav", x, sr=sr), role, note) + tuple(spec[3:]))
        return Path(mod.build_project(tmp_path / f"projects{count[0]}", f"Song {count[0]}", specs, mode))

    make.builder = mod
    return make


@pytest.fixture
def app(page, warm_page, request):
    """The page on the bridge. Fails the test on a page error, on a console error (a 4xx answer of a
    command is expected: the bridge answers errors as the server does) and on any request to /api/
    (app.js must go through B). X-3: a Playwright trace runs during the test; it is kept only when the
    setup or the test fails (unless pytest-playwright's own --tracing is on)."""
    bridge = warm_page
    tracing = request.config.getoption("--tracing", default="off") == "off"
    if tracing:
        page.context.tracing.start(screenshots=True, snapshots=True)
    errors, direct, invokes = [], [], []
    page.on("pageerror", lambda e: errors.append(f"pageerror: {e}"))
    page.on("console", lambda m: errors.append(f"console: {m.text}")
            if m.type == "error" and not re.match(r"Failed to load resource.*status of 4\d\d", m.text) else None)
    page.on("request", lambda r: direct.append(r.url) if "/api/" in r.url else None)
    page.on("request", lambda r: invokes.append(r.url.rsplit("/invoke/", 1)[1]) if "/invoke/" in r.url else None)
    page.on("dialog", lambda d: d.accept())
    page.add_init_script("localStorage.removeItem('dsg_sid')")
    try:
        page.goto(bridge.url + "/")
        # the page opens the newest project of the bridge at start-up (the registry holds the earlier tests'
        # projects, dirty): wait for that load, else it sets the dirty flag again and File > Open asks first
        page.wait_for_function(READY)
    except Exception:
        if tracing:
            save_trace(page.context, request.node.nodeid)
        raise
    page.url_base = bridge.url
    page.home = bridge.home
    page.invokes = invokes
    page.errors = errors   # P4-67: a failure message can show them
    app_support.PUMP = lambda: page.wait_for_timeout(100)   # P4-67b: wait_until lets the page's events in
    try:
        yield page
    finally:
        app_support.PUMP = None
    failed = getattr(request.node, "rep_call", None) is not None and request.node.rep_call.failed
    if tracing:
        if failed or errors or direct:
            save_trace(page.context, request.node.nodeid)
        else:
            page.context.tracing.stop()
    assert errors == [], errors
    assert direct == [], f"app.js reached the Python server routes: {direct}"
