"""R17 step 2 (#55): the js_unit module tests as plain node scripts.

tests/js_unit/run.js loads the static/ modules into a fresh vm context per test - a stubbed
B, a minimal document/window stand-in, no browser, no full DOM - and exercises the persist
transitions the plan names for this step (the save cycle, a refused write, a late ack after
a switch, an edit during a write). This file is the pytest wrapper with its own marker, so
the -m e2e_app shards and the coverage gate stay untouched while the full run (just ci)
collects it.
"""
import glob
import shutil
import subprocess
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent


def _node():
    """The node binary: on PATH, or the local tools install (~/.hermes/tools)."""
    node = shutil.which("node")
    if node:
        return node
    found = sorted(glob.glob(str(Path.home() / ".hermes" / "tools" / "node-*" / "bin" / "node")))
    return found[0] if found else None


@pytest.mark.js_unit
def test_js_unit_suite():
    node = _node() or ""
    if not node:
        pytest.skip("node is not available (the plan names it a prerequisite)")
    r = subprocess.run([node, str(HERE / "run.js")], capture_output=True, text=True, timeout=120)
    assert r.returncode == 0, f"js_unit failed (node: {node}):\n{r.stdout}\n{r.stderr}"
