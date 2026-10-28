"""#4: a SIGKILLed pytest run leaves no `ds-bridge` behind. The fixtures start the bridge through
`start_bridge` with `--exit-on-stdin-eof` and its stdin on an open pipe. When the pytest process is
killed, the fixture teardown never runs, but the pipe's write end dies with it: the bridge reads an
EOF and ends on its own (within seconds). Without a kill, the normal teardown still stops it.

The parent test runs a child pytest that opens the bridge (through the session fixture, so exactly
the code path of the suite), writes the bridge pid to $DRUMSPIKE_BRIDGE_PID_FILE, and then is
killed with SIGKILL before it can tear down. The parent asserts that the pid is gone within 5 s."""
import os
import subprocess
import sys
import time

import pytest

from conftest import SKIP, bridge_binary
from helpers import ROOT


def test_child_opens_bridge(bridge):
    """The child pytest run: it only holds the session bridge and reports its pid. Then it waits,
    so the parent can SIGKILL it and the fixture teardown never runs."""
    if "DRUMSPIKE_BRIDGE_PID_FILE" not in os.environ:
        pytest.skip("child-only test: run via test_sigkilled_child_pytest_leaves_no_bridge")
    path = os.environ["DRUMSPIKE_BRIDGE_PID_FILE"]
    with open(path, "w") as fh:
        fh.write(str(bridge.proc.pid))
    assert bridge.proc.poll() is None
    time.sleep(600)


def test_sigkilled_child_pytest_leaves_no_bridge(tmp_path):
    """A SIGKILLed child pytest leaves no ds-bridge alive after 5 s (red before the fix: the bridge
    kept running with parent 1)."""
    exe = bridge_binary()
    if not exe.is_file():
        if os.environ.get("DRUMSPIKE_REQUIRE_APP") == "1":
            pytest.fail(SKIP, pytrace=False)
        pytest.skip(SKIP)
    pid_file = tmp_path / "bridge.pid"
    env = dict(os.environ, DRUMSPIKE_NO_WARM_PAGE="1", DRUMSPIKE_BRIDGE_PID_FILE=str(pid_file))
    child = subprocess.Popen(
        [sys.executable, "-m", "pytest", "-q", "--no-header", "-p", "no:cacheprovider",
         str(ROOT / "tests" / "e2e_app" / "test_bridge_stdin_exit.py::test_child_opens_bridge")],
        cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
    try:
        out = ""
        deadline = time.monotonic() + 120
        while not pid_file.exists():
            if child.poll() is not None:
                raise RuntimeError(f"child pytest ended before the bridge pid arrived:\n{child.stdout.read()}")
            if time.monotonic() > deadline:
                child.kill()
                raise RuntimeError(f"no bridge pid after 120 s:\n{child.stdout.read()}")
            time.sleep(0.2)
        pid = int(pid_file.read_text())
        child.kill()   # SIGKILL: no teardown, the stdin pipe's write end dies with the child
        child.wait(30)
        # The bridge pid must be gone within 5 s (it reads the EOF and exits).
        gone, deadline = False, time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                gone = True
                break
            time.sleep(0.1)
        if child.stdout:
            out = child.stdout.read()
        assert gone, f"ds-bridge pid {pid} still alive 5 s after the child was killed:\n{out}"
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(30)
