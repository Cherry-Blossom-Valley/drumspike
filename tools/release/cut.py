"""Release cut tool (owner 2026-10-27, #476): one cut is one command plus a review of the notes.

Standard library and the gh CLI only. Run from the repository root:

    uv run --no-project python tools/release/cut.py plan
    uv run --no-project python tools/release/cut.py notes
    uv run --no-project python tools/release/cut.py bump
    uv run --no-project python tools/release/cut.py status v0.9.3

`plan` prints the state since the last release tag: the merged PRs, which of them are
user-visible, the next patch version, and whether main's last code commit has a green ci.yml
run. It prints "skip: no user-visible change" when there is nothing to release.
`notes` writes docs/release-notes/release-notes-<next>.md as a draft, one line per ticket, grouped
by path (playback, tempo, detection, export, UI, general).
`bump` sets the version in the four files release.yml checks (Cargo.toml workspace,
tauri.conf.json, apps/desktop/package.json, apps/desktop/selftest/shell.json) and prints the
diff. It refuses a dirty tree.
`status <tag>` prints the release.yml run of the tag with each job's duration and the draft's
assets with their sha256 against SHA256SUMS.

No .github change, no tag push: the coordinator pushes tags.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

# The visibility list (a PR that touches only these is not user-visible) is WIDER than the CI
# ignore list: tests/, tools/ and .github/ do start a ci.yml run. Finding 1 of #476: the CI
# checks of last_code_commit and ci_green must use the paths-ignore of ci.yml itself
# (ci_ignored), not this list.
IGNORED_PREFIXES = ("docs/", "tests/", "tools/", ".github/")
IGNORED_ROOT = ("AGENTS.md", "CLAUDE.md", "README.md")

CI_FALLBACK_PREFIXES = ("docs/",)
CI_FALLBACK_ROOT = ("AGENTS.md", "CLAUDE.md", "README.md", ".crewboard.toml", "AUTHORITY.md")

VERSION_FILES = [
    ("Cargo.toml", re.compile(r'(\[workspace\.package\][^[]*?^version = ")([^"]+)(")', re.M | re.S)),
    ("apps/desktop/src-tauri/tauri.conf.json", re.compile(r'(^  "version": ")([^"]+)(",)', re.M)),
    ("apps/desktop/package.json", re.compile(r'(^  "version": ")([^"]+)(",)', re.M)),
    ("apps/desktop/selftest/shell.json", re.compile(r'("expect": \{[^}]*"version": ")([^"]+)(")', re.S)),
]

# Groups of the notes, checked against the title and the changed paths; first match wins.
GROUPS = [
    ("Playback", ("transport", "playback", "playhead", "play ", "mute", "solo", "click", "speed",
                  "lanes", "wave", "audio", "loop", "scrub", "seek", "half")),
    ("Tempo", ("tempo", "meter", "bar ", "bars", "beat", "bpm", "time signature")),
    ("Detection", ("detect", "peak", "onset", "refine", "velocity", "hit", "kick", "snare", "tom",
                   "roll", "flam", "bleed", "sensitivity", "confidence", "learn", "rescue")),
    ("Export", ("export", "midi", "spike file", "release")),
    ("UI", ("page", "menu", "dialog", "editor", "keymap", "static/", "undo", "redo", "ruler")),
]


class CutError(Exception):
    """A refusal the tool prints as its own line."""


# ---------------------------------------------------------------- plumbing

def sh(args, root, check=True):
    """git and friends with captured output."""
    r = subprocess.run(args, cwd=str(root), capture_output=True, text=True)
    if check and r.returncode != 0:
        raise CutError(f"{' '.join(args)} failed: {r.stderr.strip() or r.stdout.strip()}")
    return r.stdout


def gh(args, root):
    """One gh call with captured output; the error carries the stderr for the refusal.
    The environment is passed unchanged: stripping it dropped GH_TOKEN (finding 3 of #476)."""
    r = subprocess.run(["gh", *args], cwd=str(root), capture_output=True, text=True)
    if r.returncode != 0:
        raise CutError(f"gh {' '.join(args[:3])} failed: {r.stderr.strip()[-400:]}")
    return r.stdout


def gh_json(args, root):
    return json.loads(gh(args, root))


def find_root(explicit):
    """The repository root: --root, else git from the working directory."""
    if explicit:
        return Path(explicit).resolve()
    out = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    if out.returncode != 0:
        raise CutError("not in a git repository (use --root)")
    return Path(out.stdout.strip())


def repo_slug(root):
    """owner/name of origin."""
    url = sh(["git", "remote", "get-url", "origin"], root).strip()
    m = re.search(r"[:/]([^/:]+/[^/]+?)(?:\.git)?$", url)
    return m.group(1)


# ---------------------------------------------------------------- versions

def current_version(root):
    """The version of Cargo.toml [workspace.package]."""
    text = (root / "Cargo.toml").read_text()
    m = VERSION_FILES[0][1].search(text)
    if not m:
        raise CutError("Cargo.toml has no [workspace.package] version")
    return m.group(2)


def next_version(version):
    """The next patch: 0.9.3 -> 0.9.4."""
    parts = version.split(".")
    if len(parts) != 3 or not all(p.isdigit() for p in parts):
        raise CutError(f"the version {version!r} is not X.Y.Z")
    parts[2] = str(int(parts[2]) + 1)
    return ".".join(parts)


def ci_ignored(root):
    """The paths-ignore of the ci.yml push trigger, read from the file: (prefixes, roots).
    A commit that changes only these gets no ci.yml run, so the CI walk must skip it.
    Without a readable ci.yml the last known list is the fallback."""
    try:
        text = (root / ".github" / "workflows" / "ci.yml").read_text()
    except OSError:
        return CI_FALLBACK_PREFIXES, CI_FALLBACK_ROOT
    m = re.search(r"paths-ignore:\s*\n((?:[ \t]*-[ \t]*[^\n#]+\n)+)", text)
    if not m:
        return CI_FALLBACK_PREFIXES, CI_FALLBACK_ROOT
    prefixes, roots = [], []
    for line in m.group(1).splitlines():
        item = line.split("-", 1)[1].strip().strip("\"'").rstrip("*").rstrip("/")
        if not item or item == "docs":
            item = "docs/"
        if item.endswith("/"):
            prefixes.append(item)
        else:
            roots.append(item)
    return tuple(prefixes), tuple(roots)


def last_tag(root):
    """The last release tag on origin by version, or None. The tags come from ls-remote:
    the local clone may hold tags the owner deleted on origin (finding 2 of #476)."""
    out = sh(["git", "ls-remote", "--tags", "origin"], root, check=False)
    tags = []
    for line in out.splitlines():
        ref = line.split("\t", 1)[-1]
        if ref.startswith("refs/tags/v") and not ref.endswith("^{}"):
            tags.append(ref.rsplit("/", 1)[-1])
    if not tags:
        tags = sh(["git", "tag", "-l", "v*"], root).split()
    if not tags:
        return None
    def key(t):
        return [int(x) if x.isdigit() else x for x in re.split(r"[.-]", t[1:])]
    return sorted(tags, key=key)[-1]


def tag_time(root, tag):
    """The commit time of the tag, ISO 8601."""
    return sh(["git", "log", "-1", "--format=%cI", tag], root).strip()


# ---------------------------------------------------------------- plan

def pr_is_user_visible(files):
    """A PR is user-visible when it changes anything outside the ignored paths."""
    for f in files:
        p = f["path"] if isinstance(f, dict) else str(f)
        if "/" not in p:
            if p not in IGNORED_ROOT:
                return True
            continue
        if not p.startswith(IGNORED_PREFIXES):
            return True
    return False


def merged_prs(root, since_iso):
    """The PRs merged on main after `since_iso`: (number, title, issue or None, files, visible)."""
    rows = gh_json(["pr", "list", "--repo", repo_slug(root), "--state", "merged", "--base", "main",
                    "--limit", "100", "--json", "number,title,body,files,mergedAt"], root)
    out = []
    for r in rows:
        if not r.get("mergedAt") or r["mergedAt"] <= since_iso:
            continue
        issue = None
        m = re.search(r"(?:[Cc]loses|[Ff]ixes|[Rr]esolves)\s+#(\d+)", r.get("body") or "")
        if m:
            issue = int(m.group(1))
        files = [f["path"] for f in r.get("files") or []]
        out.append({"number": r["number"], "title": r["title"], "issue": issue,
                    "files": files, "visible": pr_is_user_visible(files)})
    out.sort(key=lambda r: r["number"])
    return out


def last_code_commit(root):
    """The last first-parent commit of origin/main that starts a ci.yml run: anything outside
    the paths-ignore of ci.yml (finding 1 of #476: tests/, tools/ and .github/ DO get a run,
    the visibility list is wider). Walks back 40 commits, like release.yml."""
    ign_p, ign_r = ci_ignored(root)
    base = "origin/main"
    if subprocess.run(["git", "rev-parse", "--verify", "-q", base], cwd=str(root),
                      capture_output=True).returncode != 0:
        base = "main"
    for line in sh(["git", "rev-list", "--first-parent", "-n", "40", base], root).split():
        files = sh(["git", "show", "--name-only", "--format=", line], root).split()
        if any(f not in ign_r and not f.startswith(ign_p) for f in files):
            return line
    return None


def is_ci_code(root, files):
    """True when the changed paths start a ci.yml run (the paths-ignore of ci.yml)."""
    ign_p, ign_r = ci_ignored(root)
    return any(f not in ign_r and not f.startswith(ign_p) for f in files)


def ci_green(root, commit):
    """(bool, text): walk back from `commit` along first parent to the nearest commit that has
    a ci.yml run (like release.yml's check); it must be a successful completed run. Commits
    without a run that change code are noted (a [skip ci] code commit, against the rule of
    2026-10-27)."""
    slug = repo_slug(root)
    walked = []
    for c in [commit] + sh(["git", "rev-list", "--first-parent", "-n", "40", commit], root).split()[1:]:
        runs = gh_json(["run", "list", "--repo", slug, "--workflow", "ci.yml",
                        "--commit", c, "--limit", "30", "--json", "conclusion,status,databaseId"], root)
        if not runs:
            files = sh(["git", "show", "--name-only", "--format=", c], root).split()
            code = is_ci_code(root, files)
            walked.append(f"{c[:8]} has no ci run" + (" (a code commit: [skip ci]?)" if code else " (docs only)"))
            continue
        done = [r for r in runs if r["status"] == "completed"]
        if any(r["conclusion"] == "success" for r in done):
            return True, "; ".join(walked + [f"green on {c[:8]} (run {done[0]['databaseId']})"])
        if any(r["status"] != "completed" for r in runs):
            return False, "; ".join(walked + [f"still running on {c[:8]}"])
        return False, "; ".join(walked + [f"not green on {c[:8]}"])
    return False, "; ".join(walked + ["no ci run on the last 40 commits"])


def cmd_plan(args):
    root = find_root(args.root)
    tag = last_tag(root)
    print(f"last tag: {tag or 'none'}")
    version = current_version(root)
    nxt = next_version(version)
    print(f"current version: {version}   next: {nxt}")
    prs = merged_prs(root, tag_time(root, tag)) if tag else []
    if not any(p["visible"] for p in prs):
        print("skip: no user-visible change")
        return 0
    print(f"merged PRs on main since {tag}: {len(prs)}")
    for p in prs:
        closes = f" (closes #{p['issue']})" if p["issue"] else ""
        kind = "user-visible" if p["visible"] else "not user-visible"
        nf = f"{len(p['files'])} file" + ("s" if len(p["files"]) != 1 else "")
        print(f"  #{p['number']} {p['title']}{closes} - {kind} ({nf})")
    print(f"user-visible: {sum(1 for p in prs if p['visible'])}")
    c = last_code_commit(root)
    if c is None:
        print("ci on main: no code commit on origin/main")
        return 0
    ok, text = ci_green(root, c)
    print(f"ci on main: {text}")
    if not ok:
        print("WAIT: main has no green ci run; do not tag yet")
    return 0


# ---------------------------------------------------------------- notes

def group_of(pr):
    """The notes group of a PR: the title first, then the paths; 'general' as the fallback."""
    hay = pr["title"].lower() + " " + " ".join(pr["files"])
    for name, words in GROUPS:
        if any(w in hay for w in words):
            return name
    return "General"


def cmd_notes(args):
    root = find_root(args.root)
    tag = last_tag(root)
    version = current_version(root)
    nxt = next_version(version)
    prs = [p for p in merged_prs(root, tag_time(root, tag)) if p["visible"]] if tag else []
    path = root / "docs" / "release-notes" / f"release-notes-{nxt}.md"
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists() and not args.force:
        raise CutError(f"{path.name} exists; edit it, or use --force to overwrite the draft")
    groups = {}
    for p in prs:
        groups.setdefault(group_of(p), []).append(p)
    lines = [f"# Drum Spike {nxt}", "",
             f"Draft release notes for {tag or 'the first release'}..v{nxt}. One line per ticket;",
             "edit freely before the tag.", ""]
    for name in [g for g, _ in GROUPS] + ["General"]:
        if name not in groups:
            continue
        lines.append(f"## {name}")
        lines.append("")
        for p in groups[name]:
            n = p["issue"] or p["number"]
            lines.append(f"- #{n} {p['title']}")
        lines.append("")
    path.write_text("\n".join(lines))
    print(f"wrote {path.relative_to(root)} ({len(prs)} tickets)")
    return 0


# ---------------------------------------------------------------- bump

def bump_files(root, version):
    """Set the version in the four files; returns the changed paths."""
    changed = []
    for name, rx in VERSION_FILES:
        p = root / name
        text = p.read_text()
        new, n = rx.subn(lambda m: m.group(1) + version + m.group(3), text, count=1)
        if n != 1:
            raise CutError(f"{name}: no version line matched")
        if new != text:
            p.write_text(new)
            changed.append(name)
    return changed


def cmd_bump(args):
    root = find_root(args.root)
    if sh(["git", "status", "--porcelain"], root).strip():
        raise CutError("the tree is dirty: commit or stash first (bump refuses a dirty tree)")
    version = next_version(current_version(root))
    changed = bump_files(root, version)
    print(f"version {version} in {len(VERSION_FILES)} files ({len(changed)} changed)")
    print(sh(["git", "diff"], root), end="")
    return 0


# ---------------------------------------------------------------- status

def parse_ts(s):
    """Seconds since the epoch of a GitHub ISO timestamp."""
    import datetime
    return datetime.datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def duration(a, b):
    return max(0, int(parse_ts(b) - parse_ts(a)))


def cmd_status(args):
    root = find_root(args.root)
    tag = args.tag
    runs = gh_json(["run", "list", "--repo", repo_slug(root), "--workflow", "release.yml",
                    "--limit", "30", "--json", "databaseId,headBranch,status,conclusion,event"], root)
    mine = [r for r in runs if r["headBranch"] == tag]
    if not mine:
        raise CutError(f"no release.yml run for {tag}")
    run = mine[0]
    print(f"run {run['databaseId']}: {run['event']} {run['status']} {run['conclusion']}")
    jobs = gh_json(["run", "view", str(run["databaseId"]), "--repo", repo_slug(root), "--json", "jobs"], root)["jobs"]
    total = 0
    for j in jobs:
        d = duration(j["startedAt"], j["completedAt"])
        total += d
        print(f"  job {j['name']:<12} {j['conclusion'] or j['status']:<9} {d:>4} s")
        steps = sorted(j["steps"], key=lambda s: duration(s["startedAt"], s["completedAt"]), reverse=True)
        for s in steps[:3]:
            sd = duration(s["startedAt"], s["completedAt"])
            if sd > 1:
                print(f"    step {s['name']}: {sd} s")
    print(f"  jobs sum: {total} s")
    rel = gh_json(["api", f"repos/{repo_slug(root)}/releases/tags/{tag}"], root)
    print(f"draft: {rel.get('draft')}")
    sums = {}
    for a in rel.get("assets") or []:
        if a["name"] == "SHA256SUMS":
            body = gh(["api", f"repos/{repo_slug(root)}/releases/assets/{a['id']}",
                       "-H", "Accept: application/octet-stream"], root)
            for line in body.splitlines():
                parts = line.split(None, 1)
                if len(parts) == 2:
                    sums[parts[1].strip().lstrip("*")] = parts[0]
    for a in rel.get("assets") or []:
        if a["name"] == "SHA256SUMS":
            continue
        want = sums.get(a["name"])
        got = (a.get("digest") or "").split(":", 1)[-1]
        state = "ok" if want and got and want == got else ("no digest" if not got else "MISMATCH")
        print(f"  asset {a['name']}: {state}" + (f" (sha256 {got[:16]}...)" if got else ""))
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description="the release cut tool")
    ap.add_argument("--root", help="the repository root (default: git from the working directory)")
    sub = ap.add_subparsers(dest="cmd", required=True)  # type: ignore[attr-defined]
    sub.add_parser("plan")
    n = sub.add_parser("notes")
    n.add_argument("--force", action="store_true", help="overwrite an existing draft")
    sub.add_parser("bump")
    s = sub.add_parser("status")
    s.add_argument("tag")
    args = ap.parse_args(argv)
    try:
        return {"plan": cmd_plan, "notes": cmd_notes, "bump": cmd_bump, "status": cmd_status}[args.cmd](args)
    except CutError as e:
        print(f"cut: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
