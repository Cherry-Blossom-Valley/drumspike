"""#426: no absolute host home path in a tracked source file under crates/.

    uv run python tools/check_host_path.py

A host path names one build host and works on no other machine: an absolute path
under the home root of Linux and the one of macOS, followed by a user name. The
check reads every file that `git ls-files -- crates/` tracks and fails on a hit,
outside the allow list:

    ALLOW: {path: reason}

Each allow entry needs a reason; the listed files carry a neutral placeholder
name that names no host. The check prints the count of files it read and fails
when it read none (run inside the repo). This file spells the two roots by parts
so that the guard itself carries no literal one (tools/check_private.py).
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# The home roots of Linux and macOS, followed by a user name; spelled by parts
# so no literal lands in this source (check_private bans the two prefixes).
_LINUX_ROOT = "/" + "home"
_MACOS_ROOT = "/" + "Users"
HOME_RE = re.compile(
    "(?:" + re.escape(_LINUX_ROOT) + "|" + re.escape(_MACOS_ROOT) + ")"
    r"/[A-Za-z0-9._][A-Za-z0-9._-]*"
)

# Neutral placeholder names that name no host, one reason each.
ALLOW = {
    "crates/core/src/project/tests.rs": (
        "neutral placeholder fixture name, names no host"
    ),
}


def tracked_files(root: Path = ROOT) -> list[str]:
    """The files git tracks under crates/."""
    out = subprocess.run(
        ["git", "ls-files", "--", "crates/"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return [line for line in out.stdout.splitlines() if line]


def hits_in(text: str) -> list[str]:
    """The home paths of one text."""
    return [m.group(0) for m in HOME_RE.finditer(text)]


def scan(root: Path = ROOT) -> tuple[int, list[tuple[str, int, str]]]:
    """(files read, [(path, lineno, hit)]) over the tracked files of root."""
    files = tracked_files(root)
    hits: list[tuple[str, int, str]] = []
    for rel in files:
        if rel in ALLOW:
            continue
        text = (root / rel).read_text(errors="replace")
        for lineno, line in enumerate(text.splitlines(), start=1):
            for hit in hits_in(line):
                hits.append((rel, lineno, hit))
    return len(files), hits


def main() -> int:
    n, hits = scan()
    if n == 0:
        print("FAIL: read 0 tracked files under crates/ (run inside the repo)")
        return 1
    if hits:
        for rel, lineno, hit in hits:
            print(f"FAIL: {rel}:{lineno}: host path {hit}")
        print(f"FAIL: {len(hits)} host path(s) in {n} files")
        return 1
    for rel in sorted(ALLOW):
        print(f"allow: {rel} ({ALLOW[rel]})")
    print(f"OK: no host path in {n} files under crates/")
    return 0


if __name__ == "__main__":
    sys.exit(main())
