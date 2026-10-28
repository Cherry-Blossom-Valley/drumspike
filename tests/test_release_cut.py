"""#476: the release cut tool, against a fake gh (a stub script on PATH) and a small git repo."""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
CUT = ROOT / "tools" / "release" / "cut.py"

FAKE_GH_DATA = {
    "prs_visible": [
        {"number": 470, "title": "MIDI export: the note ticks follow the exact tempo of the map",
         "body": "Closes #466", "mergedAt": "2026-10-27T10:00:00Z",
         "files": [{"path": "crates/core/src/midi.rs"}]},
        {"number": 471, "title": "docs: the release procedure", "body": "",
         "mergedAt": "2026-10-27T11:00:00Z", "files": [{"path": "docs/RELEASING.md"}]},
    ],
    "prs_skip": [
        {"number": 472, "title": "docs: the release procedure", "body": "",
         "mergedAt": "2026-10-27T11:00:00Z", "files": [{"path": "docs/RELEASING.md"}]},
        {"number": 473, "title": "tests: more tempo cases", "body": "",
         "mergedAt": "2026-10-27T11:30:00Z", "files": [{"path": "tests/test_exports.py"}]},
    ],
    "ci_green": [{"databaseId": 111, "status": "completed", "conclusion": "success"}],
    "ci_running": [{"databaseId": 333, "status": "in_progress", "conclusion": None}],
    "ci_none": [],
    "run_list": [{"databaseId": 222, "headBranch": "v0.9.3", "status": "completed",
                  "conclusion": "success", "event": "push"}],
    "jobs": {"jobs": [
        {"name": "check", "conclusion": "success", "status": "completed",
         "startedAt": "2026-10-26T13:38:29Z", "completedAt": "2026-10-26T13:38:40Z", "steps": []},
        {"name": "build", "conclusion": "success", "status": "completed",
         "startedAt": "2026-10-26T13:38:42Z", "completedAt": "2026-10-26T13:46:49Z", "steps": [
             {"name": "release.sh (build, sign, notarize, both architectures)", "status": "completed",
              "conclusion": "success", "startedAt": "2026-10-26T13:38:48Z",
              "completedAt": "2026-10-26T13:46:34Z"}]},
    ]},
    "release": {"draft": True, "assets": [
        {"id": 5, "name": "SHA256SUMS", "digest": "sha256:aaa"},
        {"id": 6, "name": "Drum Spike_0.9.3_aarch64.dmg", "digest": "sha256:bbb"},
    ]},
    "sha256sums": "bbb  Drum Spike_0.9.3_aarch64.dmg\n",
}

# the fake gh: one canned answer per argument signature; the mode file picks the PR list and
# whether the ci run is green
_FAKE_GH = """
import json, sys
from pathlib import Path
a = sys.argv[1:]
mode_file = Path(__file__).with_name("mode.txt")
d = json.loads(r'''__DATA__''')
mode = mode_file.read_text().strip() if mode_file.exists() else "visible"
if a[:2] == ["pr", "list"]:
    print(json.dumps(d["prs_visible"] if mode in ("visible", "no_ci", "running") else d["prs_skip"]))
elif a[:2] == ["run", "list"]:
    if "--commit" in a:
        if mode == "no_ci":
            print(json.dumps([]))
        elif mode == "running":
            print(json.dumps(d["ci_running"]))
        else:
            print(json.dumps(d["ci_green"]))
    else:
        print(json.dumps(d["run_list"]))
elif a[:2] == ["run", "view"]:
    print(json.dumps(d["jobs"]))
elif a[:2] == ["api", "repos/owner/drumspike/releases/tags/v0.9.3"]:
    print(json.dumps(d["release"]))
elif a[:2] == ["api", "repos/owner/drumspike/releases/assets/5"]:
    print(d["sha256sums"], end="")
else:
    print("fake gh: unexpected args: " + " ".join(a), file=sys.stderr)
    sys.exit(1)
""".replace("__DATA__", json.dumps(FAKE_GH_DATA))


@pytest.fixture
def repo(tmp_path):
    """A mini repository with the four version files at 0.9.3, a bare remote (owner/drumspike)
    that carries the tags v0.9.2 and v0.9.3, a local-only tag v0.9.9 that ls-remote must
    ignore, one code commit on main, and the fake gh first on PATH; returns (root, env, mode)."""
    r = tmp_path / "repo"
    (r / "apps/desktop/src-tauri").mkdir(parents=True)
    (r / "apps/desktop/selftest").mkdir(parents=True)
    (r / "docs/migration").mkdir(parents=True)
    (r / "Cargo.toml").write_text('[workspace.package]\nversion = "0.9.3"\nedition = "2021"\n')
    (r / "apps/desktop/src-tauri/tauri.conf.json").write_text('{\n  "version": "0.9.3",\n  "name": "x"\n}\n')
    (r / "apps/desktop/package.json").write_text('{\n  "name": "x",\n  "version": "0.9.3",\n  "scripts": {}\n}\n')
    (r / "apps/desktop/selftest/shell.json").write_text('{"expect": {"tracks": 3, "version": "0.9.3"}}\n')
    # a ci.yml with the real paths-ignore, so cut.py parses the ignore list from the file
    (r / ".github/workflows").mkdir(parents=True)
    (r / ".github/workflows/ci.yml").write_text(
        'name: ci\non:\n  push:\n    branches: [main, "feat/**"]\n'
        '    paths-ignore:\n      - "docs/**"\n      - "AGENTS.md"\n      - "CLAUDE.md"\n'
        '      - "README.md"\n      - ".crewboard.toml"\n      - "AUTHORITY.md"\n')
    def git(*a):
        subprocess.run(["git", *a], cwd=r, check=True, capture_output=True)
    git("init", "-q")
    git("config", "user.email", "t@t")
    git("config", "user.name", "t")
    # the remote is a local bare repo whose path ends in owner/drumspike.git: no network, and
    # repo_slug() still reads owner/drumspike from the URL
    remote = tmp_path / "origin" / "owner" / "drumspike.git"
    remote.mkdir(parents=True)
    subprocess.run(["git", "init", "-q", "--bare", str(remote)], check=True, capture_output=True)
    git("remote", "add", "origin", str(remote))
    # one code commit on origin/main, then the tag, then a docs-only commit
    (r / "crates").mkdir()
    (r / "crates/x.rs").write_text("// code\n")
    git("add", "-A")
    env_dates = dict(os.environ, GIT_AUTHOR_DATE="2026-10-26T12:00:00Z",
                     GIT_COMMITTER_DATE="2026-10-26T12:00:00Z")
    subprocess.run(["git", "commit", "-qm", "code"], cwd=r, check=True, capture_output=True,
                   env=env_dates)
    git("tag", "v0.9.3")  # the release tag, on the code commit
    git("tag", "v0.9.2")  # an older tag, also on origin
    git("commit", "-q", "--allow-empty", "-m", "docs only")
    git("branch", "-M", "main")
    git("push", "-q", "origin", "main")
    git("push", "-q", "origin", "refs/tags/v0.9.3", "refs/tags/v0.9.2")
    git("tag", "v0.9.9")  # a local-only tag: never on origin, so last_tag ignores it
    stub = tmp_path / "stub"
    stub.mkdir()
    (stub / "gh").write_text(f"""#!/bin/sh
exec python3 {tmp_path / 'fake_gh.py'} "$@"
""")
    (stub / "gh").chmod(0o755)
    (tmp_path / "fake_gh.py").write_text(_FAKE_GH)
    env = dict(os.environ, PATH=f"{stub}:{os.environ['PATH']}")
    return r, env, tmp_path / "mode.txt"


def set_mode(repo, mode):
    repo[2].write_text(mode)


def run_cut(args, repo):
    r, env, _ = repo
    return subprocess.run([sys.executable, str(CUT), *args], cwd=r, env=env,
                          capture_output=True, text=True)


# ---------------------------------------------------------------- plan

def test_plan_lists_the_user_visible_pr_and_the_next_version(repo):
    p = run_cut(["plan"], repo)
    assert p.returncode == 0, p.stderr
    assert "last tag: v0.9.3" in p.stdout
    assert "next: 0.9.4" in p.stdout
    assert "#470 MIDI export: the note ticks follow the exact tempo of the map (closes #466) - user-visible" in p.stdout
    assert "#471" in p.stdout and "not user-visible" in p.stdout
    assert "user-visible: 1" in p.stdout
    assert "ci on main: green on" in p.stdout


def test_plan_skips_when_nothing_is_user_visible(repo):
    set_mode(repo, "skip")
    p = run_cut(["plan"], repo)
    assert p.returncode == 0, p.stderr
    assert "skip: no user-visible change" in p.stdout


def test_plan_says_to_wait_without_a_green_ci_run(repo):
    set_mode(repo, "no_ci")
    p = run_cut(["plan"], repo)
    assert p.returncode == 0, p.stderr
    assert "WAIT: main has no green ci run" in p.stdout


def test_last_tag_comes_from_origin_and_ignores_a_local_only_tag(repo):
    # v0.9.9 exists only locally; the plan must read the tags of origin (finding 2 of #476)
    p = run_cut(["plan"], repo)
    assert p.returncode == 0, p.stderr
    assert "last tag: v0.9.3" in p.stdout


def test_plan_waits_when_the_tools_only_head_commit_has_a_running_ci_run(repo):
    # a tools-only commit is a code commit for ci.yml (finding 1 of #476): the head has a
    # running run, so the plan must report WAIT on it instead of walking past it
    r, env, mode = repo
    (r / "tools_rel").mkdir()
    (r / "tools_rel" / "x.py").write_text("# tools\n")
    subprocess.run(["git", "add", "-A"], cwd=r, check=True, capture_output=True)
    subprocess.run(["git", "commit", "-qm", "tools only"], cwd=r, check=True, capture_output=True)
    subprocess.run(["git", "push", "-q", "origin", "main"], cwd=r, check=True, capture_output=True)
    head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=r, check=True,
                          capture_output=True, text=True).stdout.strip()
    mode.write_text("running")
    p = subprocess.run([sys.executable, str(CUT), "plan"], cwd=r, env=env,
                       capture_output=True, text=True)
    assert p.returncode == 0, p.stderr
    assert "WAIT: main has no green ci run" in p.stdout
    # the run is reported on the tools-only head itself, not on an older commit
    assert f"still running on {head[:8]}" in p.stdout


# ---------------------------------------------------------------- notes

def test_notes_writes_the_draft_grouped_by_path(repo):
    p = run_cut(["notes"], repo)
    assert p.returncode == 0, p.stderr
    text = (repo[0] / "docs" / "release-notes" / "release-notes-0.9.4.md").read_text()
    assert "## Tempo" in text
    assert "- #466 MIDI export: the note ticks follow the exact tempo of the map" in text
    assert "#471" not in text   # docs-only PRs are not in the notes


def test_notes_refuses_to_overwrite(repo):
    assert run_cut(["notes"], repo).returncode == 0
    p = run_cut(["notes"], repo)
    assert p.returncode == 2
    assert "exists" in p.stderr


# ---------------------------------------------------------------- bump

def test_bump_sets_the_version_in_all_four_files(repo):
    p = run_cut(["bump"], repo)
    assert p.returncode == 0, p.stderr
    r = repo[0]
    assert 'version = "0.9.4"' in (r / "Cargo.toml").read_text()
    assert '"version": "0.9.4"' in (r / "apps/desktop/src-tauri/tauri.conf.json").read_text()
    assert '"version": "0.9.4"' in (r / "apps/desktop/package.json").read_text()
    assert '"version": "0.9.4"' in (r / "apps/desktop/selftest/shell.json").read_text()
    # the diff is printed (the changed lines carry the new version, the removed ones the old)
    assert '"0.9.4"' in p.stdout and '"0.9.3"' in p.stdout


def test_bump_refuses_a_dirty_tree(repo):
    r = repo[0]
    (r / "crates/x.rs").write_text("// changed\n")
    p = run_cut(["bump"], repo)
    assert p.returncode == 2
    assert "dirty" in p.stderr
    assert 'version = "0.9.3"' in (r / "Cargo.toml").read_text()


# ---------------------------------------------------------------- status

def test_status_prints_jobs_and_checks_the_asset_digests(repo):
    p = run_cut(["status", "v0.9.3"], repo)
    assert p.returncode == 0, p.stderr
    assert "run 222: push completed success" in p.stdout
    assert "job check" in p.stdout and " 11 s" in p.stdout
    assert "job build" in p.stdout and " 487 s" in p.stdout
    assert "release.sh (build, sign, notarize, both architectures): 466 s" in p.stdout
    assert "draft: True" in p.stdout
    # the digest of the asset matches the SHA256SUMS line
    assert "asset Drum Spike_0.9.3_aarch64.dmg: ok" in p.stdout
