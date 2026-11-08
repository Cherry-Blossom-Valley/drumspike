"""#383: the release binary answers `--help`, `-h`, `help` and `--version` and never opens a
window. The owner's test: the agent ran the shipped binary with `--help` to learn its commands
and a second app instance started.

Binary: <target>/release/drumspike-desktop (target = $CARGO_TARGET_DIR or ./target); the Gate of
the ticket builds it with `cargo build --release -p drumspike-desktop --bin drumspike-desktop`.
Without the binary the tests skip (CI builds no desktop release binary; the macOS Gate does, so
the skip there means the build step did not run).
"""

import os
import subprocess
import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
TARGET = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
BIN = TARGET / "release" / "drumspike-desktop"


def require_binary() -> None:
    if not BIN.is_file():
        pytest.skip(f"{BIN} not built: cargo build --release -p drumspike-desktop --bin drumspike-desktop")


def desktop_processes() -> list[str]:
    """The running command lines that name the release binary (a started window would be one)."""
    r = subprocess.run(["ps", "-axo", "args="], capture_output=True, text=True, timeout=10)
    return [line.strip() for line in r.stdout.splitlines() if str(BIN) in line]


def run_flag(flag: str) -> subprocess.CompletedProcess[str]:
    """The binary with one argument; the 5 s bound is the Gate's (a window would not exit)."""
    return subprocess.run([str(BIN), flag], capture_output=True, text=True, timeout=5)


def test_help_prints_the_windowless_modes_and_no_window_stays():
    require_binary()
    before = desktop_processes()
    for flag in ("--help", "-h", "help"):
        r = run_flag(flag)
        assert r.returncode == 0, flag
        for mode in ("bridge", "cli", "mcp", "--app-info", "--analyze"):
            assert mode in r.stdout, f"{flag} names {mode}"
    assert not [p for p in desktop_processes() if p not in before]


def test_version_prints_the_workspace_version():
    require_binary()
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())
    version = workspace["workspace"]["package"]["version"]
    r = run_flag("--version")
    assert r.returncode == 0
    assert r.stdout.strip() == f"drumspike-desktop {version}"


def test_an_unknown_flag_prints_the_usage_to_stderr_and_exits_2():
    require_binary()
    r = run_flag("--bogus")
    assert r.returncode == 2
    assert "--app-info" in r.stderr
    assert not [p for p in desktop_processes() if "drumspike-desktop" in p]
