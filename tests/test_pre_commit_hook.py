"""The pre-commit hook prototype (tools/hooks/pre-commit, ticket #254) blocks
an unformatted Rust file, lets a formatted one through, and ignores a commit
without Rust files.
"""

import os
import shutil
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[1]
HOOKS_DIR = REPO / "tools" / "hooks"
HOOK = HOOKS_DIR / "pre-commit"

pytestmark = [
    pytest.mark.skipif(not HOOK.is_file(), reason="the hook prototype is not committed"),
    pytest.mark.skipif(shutil.which("rustfmt") is None, reason="rustfmt is not on PATH"),
]

UNFORMATTED = "fn main(){let x=1;}\n"
FORMATTED = "fn main() {\n    let x = 1;\n}\n"


def _git(tmp_path: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], cwd=tmp_path, capture_output=True, text=True)


@pytest.fixture()
def repo(tmp_path: Path) -> Path:
    """A throwaway clone-shaped repo whose pre-commit hook is the prototype."""
    assert _git(tmp_path, "init", "-q").returncode == 0
    assert _git(tmp_path, "config", "user.email", "hook-test@example.invalid").returncode == 0
    assert _git(tmp_path, "config", "user.name", "hook test").returncode == 0
    assert _git(tmp_path, "config", "commit.gpgsign", "false").returncode == 0
    assert _git(tmp_path, "config", "core.hooksPath", str(HOOKS_DIR)).returncode == 0
    return tmp_path


def test_the_hook_file_is_executable():
    assert os.access(HOOK, os.X_OK), (
        "tools/hooks/pre-commit must be executable: git refuses a non-executable hook file")


def test_the_hook_blocks_an_unformatted_rust_file(repo):
    (repo / "main.rs").write_text(UNFORMATTED, encoding="utf-8")
    assert _git(repo, "add", "main.rs").returncode == 0
    r = _git(repo, "commit", "-q", "-m", "unformatted")
    assert r.returncode != 0, "the commit of an unformatted Rust file must be refused"
    assert "not rustfmt-clean" in r.stdout + r.stderr


def test_the_hook_allows_a_formatted_rust_file(repo):
    (repo / "main.rs").write_text(FORMATTED, encoding="utf-8")
    assert _git(repo, "add", "main.rs").returncode == 0
    r = _git(repo, "commit", "-q", "-m", "formatted")
    assert r.returncode == 0, r.stdout + r.stderr


def test_the_hook_ignores_a_commit_without_rust_files(repo):
    (repo / "notes.txt").write_text("no rust here\n", encoding="utf-8")
    assert _git(repo, "add", "notes.txt").returncode == 0
    r = _git(repo, "commit", "-q", "-m", "no rust")
    assert r.returncode == 0, r.stdout + r.stderr
