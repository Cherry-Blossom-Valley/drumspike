"""#125: tools/ci/scope.py against a small git repo and a fake gh (green commits from FAKE_GREEN)."""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
SCOPE = ROOT / "tools" / "ci" / "scope.py"

FAKE_GH = """#!/usr/bin/env python3
import json, os, sys
sha = sys.argv[sys.argv.index("--commit") + 1]
green = os.environ.get("FAKE_GREEN", "").split(",")
print(json.dumps([{"status": "completed", "conclusion": "success"}] if sha in green else
                 [{"status": "completed", "conclusion": "failure"}]))
"""

CARGO_TOML = '[workspace.package]\nversion = "0.9.5"\nedition = "2021"\n\n[workspace.dependencies]\nserde = "1.0.200"\n'
CARGO_LOCK = ('[[package]]\nname = "drumspike-core"\nversion = "0.9.5"\n\n'
              '[[package]]\nname = "serde"\nversion = "1.0.200"\n')
PKG = '{\n  "name": "drum-spike-desktop",\n  "version": "0.9.5",\n  "private": true\n}\n'


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, text=True).stdout.strip()


def commit(repo, files, msg):
    for name, text in files.items():
        p = repo / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)
    git(repo, "add", "-A")
    git(repo, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "-m", msg)
    return git(repo, "rev-parse", "HEAD")


@pytest.fixture
def repo(tmp_path):
    r = tmp_path / "repo"
    r.mkdir()
    git(r, "init", "-q", "-b", "main")
    base = commit(r, {"Cargo.toml": CARGO_TOML, "Cargo.lock": CARGO_LOCK, "apps/desktop/package.json": PKG,
                      "crates/core/src/lib.rs": "pub fn a() {}\n", "tools/eval/x.py": "X = 1\n",
                      "tests/test_x.py": "def test_x():\n    pass\n", "tests/helpers.py": "H = 1\n",
                      "docs/a.md": "a\n"}, "base")
    gh = tmp_path / "gh"
    gh.write_text(FAKE_GH)
    gh.chmod(0o755)
    return r, base, gh


def run(repo, gh, green, event="push"):
    out = repo.parent / "out.txt"
    out.write_text("")
    env = dict(os.environ, FAKE_GREEN=",".join(green))
    p = subprocess.run([sys.executable, str(SCOPE), "--event", event, "--repo", "o/n", "--gh", str(gh),
                        "--output", str(out)], cwd=repo, env=env, capture_output=True, text=True)
    assert p.returncode == 0, p.stderr
    kv = dict(line.split("=", 1) for line in out.read_text().splitlines())
    return kv, p.stdout


@pytest.mark.parametrize("files, mode", [
    ({"docs/a.md": "b\n"}, "none"),
    ({"README.md": "r\n"}, "none"),
    ({"tools/eval/x.py": "X = 2\n"}, "py"),
    ({"tests/test_x.py": "def test_x():\n    assert 1\n"}, "py"),
    ({"site/index.html": "<p>\n"}, "py"),
    ({"tests/helpers.py": "H = 2\n"}, "full"),
    ({"tests/e2e_app/test_y.py": "def test_y():\n    pass\n"}, "full"),
    ({"tools/media/demo_session.py": "D = 1\n"}, "full"),
    ({"crates/core/src/lib.rs": "pub fn b() {}\n"}, "full"),
    ({"static/app.js": "let a;\n"}, "full"),
    ({".github/workflows/ci.yml": "name: ci\n"}, "full"),
    ({"uv.lock": "x\n"}, "full"),
])
def test_the_changed_files_decide_the_mode(repo, files, mode):
    r, base, gh = repo
    commit(r, files, "change")
    kv, out = run(r, gh, [base])
    assert kv["mode"] == mode, out
    assert kv["rust"] == kv["e2e"] == str(mode == "full").lower()
    assert kv["base"] == base


def test_a_release_commit_runs_nothing(repo):
    r, base, gh = repo
    commit(r, {"Cargo.toml": CARGO_TOML.replace("0.9.5", "0.9.6"),
               "Cargo.lock": CARGO_LOCK.replace('"0.9.5"', '"0.9.6"'),
               "apps/desktop/package.json": PKG.replace("0.9.5", "0.9.6"),
               "docs/release-notes/release-notes-0.9.6.md": "Changes since 0.9.5:\n"}, "Release 0.9.6")
    kv, out = run(r, gh, [base])
    assert kv["mode"] == "none", out


@pytest.mark.parametrize("files", [
    {"Cargo.toml": CARGO_TOML.replace('serde = "1.0.200"', 'serde = "1.0.201"')},   # a dependency, not the version
    {"Cargo.lock": CARGO_LOCK.replace('"1.0.200"', '"1.0.201"')},                    # another crate's version
    {"apps/desktop/package.json": PKG.replace('"private": true', '"private": false')},
])
def test_a_version_file_with_another_change_runs_everything(repo, files):
    r, base, gh = repo
    commit(r, files, "deps")
    kv, out = run(r, gh, [base])
    assert kv["mode"] == "full", out


def test_the_base_skips_a_commit_without_a_green_run(repo):
    # the parent changed Rust and its run failed (or was cancelled): the docs-only head still runs everything
    r, base, gh = repo
    commit(r, {"crates/core/src/lib.rs": "pub fn c() {}\n"}, "rust, run failed")
    commit(r, {"docs/a.md": "c\n"}, "docs")
    kv, out = run(r, gh, [base])
    assert kv["mode"] == "full", out
    assert kv["base"] == base


def test_no_green_ancestor_runs_everything(repo):
    r, _, gh = repo
    commit(r, {"docs/a.md": "d\n"}, "docs")
    kv, out = run(r, gh, [])
    assert (kv["mode"], kv["base"]) == ("full", ""), out


@pytest.mark.parametrize("event", ["workflow_dispatch", "pull_request"])
def test_by_hand_and_pull_request_runs_everything(repo, event):
    r, base, gh = repo
    commit(r, {"docs/a.md": "e\n"}, "docs")
    kv, _ = run(r, gh, [base], event=event)
    assert kv["mode"] == "full"


def test_an_unexpected_gh_answer_fails(repo, tmp_path):
    r, _, _ = repo
    commit(r, {"docs/a.md": "f\n"}, "docs")
    bad = tmp_path / "badgh"
    bad.write_text("#!/bin/sh\necho '{\"message\": \"Bad credentials\"}'\n")
    bad.chmod(0o755)
    p = subprocess.run([sys.executable, str(SCOPE), "--event", "push", "--repo", "o/n", "--gh", str(bad)],
                       cwd=r, capture_output=True, text=True)
    assert p.returncode != 0
    assert "unexpected answer" in p.stderr + p.stdout
