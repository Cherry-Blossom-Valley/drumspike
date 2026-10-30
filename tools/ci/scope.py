"""#125: which ci.yml jobs a push needs.

The base is the nearest first-parent ancestor with a green ci run (a cancelled, failed or running
run does not count), so a change that a skipped or cancelled run never checked is always in the
diff. The changed files decide the mode:

- `none`: only docs, the root text files, and version-only edits of the version files (the
  release commit). Every job is skipped and the run is green.
- `py`: also Python tools, the non-e2e tests or the landing site. pytest runs without the e2e
  tests; no Rust step, no e2e job, no JS coverage.
- `full`: anything else (Rust, the app, static/, the e2e tests and what they import, the
  workflows, the Python environment), every by-hand run and every pull request.

    python3 tools/ci/scope.py --event push --repo OWNER/NAME [--output $GITHUB_OUTPUT]

Prints the base, the changed files and the mode; with --output it appends
`mode`, `rust`, `e2e` and `base` for the jobs. Needs git history (fetch-depth 0) and gh
with a token that can read actions.
"""
import argparse
import json
import re
import subprocess
import sys

# no run of their own (ci.yml paths-ignore) and no test reads them
DOC_PREFIXES = ("docs/",)
DOC_FILES = {"AGENTS.md", "CLAUDE.md", "README.md", "CHANGELOG.md", "CODE_OF_CONDUCT.md",
             "CONTRIBUTING.md", "SECURITY.md", ".crewboard.toml", "AUTHORITY.md"}

# the release commit: the version in these files may change and nothing else in them
VERSION_FILES = {
    "Cargo.toml": re.compile(r'^version = "[0-9][^"]*"$'),
    "Cargo.lock": re.compile(r'^version = "[0-9][^"]*"$'),
    "apps/desktop/package.json": re.compile(r'"version": "[0-9][^"]*"'),
    "apps/desktop/package-lock.json": re.compile(r'"version": "[0-9][^"]*"'),
    "apps/desktop/selftest/shell.json": re.compile(r'"version": "[0-9][^"]*"'),
    "apps/desktop/src-tauri/tauri.conf.json": re.compile(r'"version": "[0-9][^"]*"'),
}
SEMVER = re.compile(r'[0-9]+\.[0-9]+\.[0-9]+[0-9A-Za-z.+-]*')
# Cargo.lock: only the version lines of our own crates (the hunk names the package)
OWN_CRATE = re.compile(r'^ ?name = "drumspike[a-z-]*"$')

# pytest covers these; the e2e tests do not import them
LIGHT_PREFIXES = ("tools/", "site/")
# the e2e tests import these, so a change runs the e2e job
E2E_DEPS_PREFIXES = ("tests/e2e_app/", "tests/golden/")
E2E_DEPS_FILES = {"tests/conftest.py", "tests/helpers.py", "tests/frozen.py", "tests/jscov.py",
                  "tests/core_tools.py", "tests/golden_corpus.py", "tests/e2e_shards.json",
                  "tools/check_exports.py", "tools/media/demo_session.py"}


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def green(repo, sha, gh="gh"):
    """True when ci.yml has a completed, successful run on `sha`."""
    out = subprocess.run([gh, "run", "list", "--repo", repo, "--workflow", "ci.yml", "--commit", sha,
                          "--limit", "30", "--json", "status,conclusion"],
                         check=True, capture_output=True, text=True).stdout
    runs = json.loads(out)
    if not isinstance(runs, list):
        raise SystemExit(f"scope: unexpected answer from gh run list: {out[:200]}")
    return any(r.get("status") == "completed" and r.get("conclusion") == "success" for r in runs)


def find_base(repo, head="HEAD", depth=30, gh="gh"):
    """The nearest first-parent ancestor of `head` (not `head` itself) with a green ci run, or None."""
    for sha in git("rev-list", "--first-parent", "-n", str(depth), f"{head}~1").split():
        if green(repo, sha, gh):
            return sha
    return None


def version_only(path, diff):
    """True when the unified diff (-U1) of `path` changes only version values: every removed line has an
    added twin that differs from it only in the version number, and the line carries the file's version key."""
    rule = VERSION_FILES[path]
    hunks = []
    for line in diff.splitlines():
        if line.startswith("@@"):
            hunks.append({"own": path != "Cargo.lock", "minus": [], "plus": []})
        elif line.startswith(("+++", "---", "diff ", "index ")) or not hunks:
            continue
        elif line[:1] == "-":
            hunks[-1]["minus"].append(line[1:])
        elif line[:1] == "+":
            hunks[-1]["plus"].append(line[1:])
        elif OWN_CRATE.match(line):
            hunks[-1]["own"] = True   # a lock hunk must name one of our crates
    if not hunks:
        return False
    for h in hunks:
        if not h["own"] or len(h["minus"]) != len(h["plus"]) or not h["minus"]:
            return False
        for a, b in zip(h["minus"], h["plus"]):
            if not (rule.search(a) and rule.search(b)) or SEMVER.sub("V", a) != SEMVER.sub("V", b) or a == b:
                return False
    return True


def kind(path, base, head):
    if path.startswith(DOC_PREFIXES) or path in DOC_FILES:
        return "doc"
    if path in VERSION_FILES and version_only(path, git("diff", "-U1", base, head, "--", path)):
        return "doc"
    if path in E2E_DEPS_FILES or path.startswith(E2E_DEPS_PREFIXES):
        return "full"
    if path.startswith(LIGHT_PREFIXES) or (path.startswith("tests/") and path.count("/") == 1):
        return "py"
    return "full"


def decide(files, base, head):
    kinds = {kind(f, base, head) for f in files}
    if "full" in kinds:
        return "full"
    if "py" in kinds:
        return "py"
    return "none"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--event", required=True)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--head", default="HEAD")
    ap.add_argument("--output", help="append mode, rust, e2e and base here ($GITHUB_OUTPUT)")
    ap.add_argument("--gh", default="gh")
    a = ap.parse_args(argv)
    base, files = None, []
    if a.event != "push":
        mode = "full"
        why = f"event {a.event}: every job runs"
    else:
        base = find_base(a.repo, a.head, gh=a.gh)
        if base is None:
            mode, why = "full", "no green ci run on the last 30 first-parent commits"
        else:
            files = [f for f in git("diff", "--name-only", base, a.head).splitlines() if f]
            mode = decide(files, base, a.head)
            why = f"{len(files)} files changed since {base[:8]} (the nearest green run)"
    print(f"scope: {mode} ({why})")
    for f in files:
        print(f"  {kind(f, base, a.head):4} {f}")
    out = {"mode": mode, "rust": str(mode == "full").lower(), "e2e": str(mode == "full").lower(),
           "base": base or ""}
    if a.output:
        with open(a.output, "a") as fh:
            fh.writelines(f"{k}={v}\n" for k, v in out.items())
    return 0


if __name__ == "__main__":
    sys.exit(main())
