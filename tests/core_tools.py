"""The Rust tools the Python tests call (P3-12: the Python binding drumspike_core is gone).

parity        target/release/parity (crates/devtools/src/bin/parity.rs): decode, peaks, refine, MIDI, spike,
              detect, project documents
service_tool  target/release/service_tool (crates/devtools/src/bin/service_tool.rs): the app's service
              commands and `core-analyze`, the analysis of the app's job on one file

Without a tool the test is skipped; with DRUMSPIKE_REQUIRE_CORE=1 (CI) it fails, so a run
without the binaries cannot pass. Build them with `cargo build --release -p drumspike-devtools`.
"""
import json
import os
import subprocess
import tempfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
TARGET = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
PARITY = TARGET / "release" / "parity"
SERVICE_TOOL = TARGET / "release" / "service_tool"
def require(tool):
    """The tool's path; skip (or fail with DRUMSPIKE_REQUIRE_CORE=1) when it is not built."""
    if not tool.is_file():
        msg = f"{tool} not built: cargo build --release -p drumspike-devtools"
        if os.environ.get("DRUMSPIKE_REQUIRE_CORE") == "1":
            pytest.fail("DRUMSPIKE_REQUIRE_CORE=1 and " + msg, pytrace=False)
        pytest.skip(msg)
    return tool


def core_analyze(jobs):
    """`service_tool core-analyze`: per job {wav, mix?, role, params?, model?, detected?} the answer
    {events, branches, detected, detector, sr} or {error}."""
    tool = require(SERVICE_TOOL)
    with tempfile.TemporaryDirectory() as td:
        jf = Path(td) / "jobs.json"
        jf.write_text(json.dumps(jobs))
        r = subprocess.run([str(tool), "core-analyze", str(jf)], capture_output=True, text=True)
    assert r.returncode == 0, r.stderr[-800:]
    out = [json.loads(line) for line in r.stdout.splitlines() if line.strip()]
    assert len(out) == len(jobs) and all(isinstance(o, dict) for o in out), (len(out), len(jobs))
    return out
