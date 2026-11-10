"""Tests of tools/check_host_path.py (#426): the check finds a host home path, allows the
neutral placeholder fixtures, and the repo itself is clean. The two roots are spelled by
parts so no literal lands in this source (tools/check_private.py)."""
import importlib.util
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
_spec = importlib.util.spec_from_file_location("check_host_path", ROOT / "tools" / "check_host_path.py")
assert _spec is not None and _spec.loader is not None
check = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check)

LINUX = "/" + "home"
MACOS = "/" + "Users"


def test_finds_the_home_roots_of_both_platforms():
    text = f'let p = "{LINUX}/alice/.cache/x.json";\nlet q = Path::new("{MACOS}/bob/Music/a.wav");\n'
    hits = check.hits_in(text)
    assert hits == [f"{LINUX}/alice", f"{MACOS}/bob"]


def test_ignores_relative_paths_and_the_lookup_helpers():
    assert check.hits_in('let p = Path::new("home/alice");') == []
    assert check.hits_in("std::env::home_dir()") == []


def test_a_host_path_fails(tmp_path: Path):
    (tmp_path / "crates").mkdir()
    f = tmp_path / "crates" / "probe.rs"
    f.write_text(f'let p = "{LINUX}/whoever/.cache/a5_tom_map.json";\n')
    subprocess.run(["git", "init", "-q", str(tmp_path)], check=True)
    subprocess.run(["git", "-C", str(tmp_path), "add", "crates/probe.rs"], check=True)
    n, hits = check.scan(tmp_path)
    assert n == 1
    assert hits == [("crates/probe.rs", 1, f"{LINUX}/whoever")]


def test_the_placeholder_fixture_is_on_the_allow_list():
    assert "crates/core/src/project/tests.rs" in check.ALLOW
    assert check.ALLOW["crates/core/src/project/tests.rs"]
    # the fixture line itself carries a home-shaped path; the allow list keeps it out
    text = (ROOT / "crates/core/src/project/tests.rs").read_text()
    assert check.hits_in(text)


def test_the_repo_is_clean():
    n, hits = check.scan()
    assert n > 0
    assert hits == []
