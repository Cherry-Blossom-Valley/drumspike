"""#104: the private-name text guard: no private name reaches a commit, a PR body or a
ticket comment.

    uv run python tools/check_private.py --diff origin/main
    uv run python tools/check_private.py --text-file <pr-body-or-comment-draft>
    uv run python tools/check_private.py <file>...

The list is the one shared definition of tools/media/check_paths.py (BANNED_BASE plus the
comma-separated words of DRUMSPIKE_PRIVATE_WORDS, set per host, never in the repo), plus
the comma-separated regexes of DRUMSPIKE_PRIVATE_PATTERNS (dataset file names). An unset
DRUMSPIKE_PRIVATE_WORDS is a loud FAIL (exit 2), never a silent pass, unless
--allow-empty is given. A hit prints file:line and the matched word and exits 1.
"""
import importlib.util
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# tools/media/check_paths.py: the one definition of the private list (BANNED_BASE +
# DRUMSPIKE_PRIVATE_WORDS). Not a second copy.
_spec = importlib.util.spec_from_file_location(
    "check_paths", ROOT / "tools" / "media" / "check_paths.py")
assert _spec is not None and _spec.loader is not None
_check_paths = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_check_paths)
BANNED_BASE = _check_paths.BANNED_BASE
private_words = _check_paths.private_words


def private_patterns():
    """The regexes of DRUMSPIKE_PRIVATE_PATTERNS (comma separated), for dataset file
    names; empty when the variable is unset."""
    raw = __import__("os").environ.get("DRUMSPIKE_PRIVATE_PATTERNS", "")
    return [r.strip() for r in raw.split(",") if r.strip()]


def words_are_set():
    """DRUMSPIKE_PRIVATE_WORDS is set (possibly empty is still 'set': the host chose
    no extra words; the base list still applies)."""
    return "DRUMSPIKE_PRIVATE_WORDS" in __import__("os").environ


# #108: a host alias inside an agent name or branch name is fine (claude@arm64-mac,
# crew/hermes@arm64-mac/108-...); only the bare alias stays banned. Mask ONLY the
# @<alias> part: the part before @ stays in the search text, so a banned login token
# plus an alias still hits (the #108 send-back finding: `ssh <token>@<alias>` must
# FAIL). A bare alias (no @ before it) still hits.
HOST_ALIASES = ("arm64-mac", "x86-mac", "linux-arm64")
_AGENT_AT_ALIAS = re.compile(
    r"([A-Za-z0-9._+-]+)@(?:" + "|".join(HOST_ALIASES) + r")(?![A-Za-z0-9-])",
    re.IGNORECASE)


def hits_in_line(line):
    """The matched words and patterns of one line: [(term, match)]."""
    out = []
    lowered = _AGENT_AT_ALIAS.sub(r"\1@", line).lower()
    for w in private_words():
        if w.lower() in lowered:
            out.append((w, w))
    for pat in private_patterns():
        try:
            m = re.search(pat, line)
        except re.error as e:
            print(f"FAIL: bad DRUMSPIKE_PRIVATE_PATTERNS regex {pat!r}: {e}", file=sys.stderr)
            raise SystemExit(2) from None
        if m:
            out.append((f"pattern {pat!r}", m.group(0)))
    return out


def check_lines(lines, label):
    """The hits of one text: [(label, lineno, term, match)]."""
    return [(label, i, term, match)
            for i, line in enumerate(lines, start=1)
            for term, match in hits_in_line(line)]


def check_text_file(path):
    return check_lines(Path(path).read_text(encoding="utf-8").splitlines(), str(path))


def check_diff(base):
    """The added lines of `git diff <base>...HEAD` scanned as pseudo-file '+<base>...HEAD'."""
    diff = subprocess.run(["git", "diff", f"{base}...HEAD"], capture_output=True,
                          text=True, check=True).stdout
    lines = [l[1:] for l in diff.splitlines() if l.startswith("+") and not l.startswith("+++")]
    return check_lines(lines, f"git diff {base}...HEAD (added lines)")


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    allow_empty = "--allow-empty" in argv
    argv = [a for a in argv if a != "--allow-empty"]
    if not words_are_set() and not allow_empty:
        print("FAIL: DRUMSPIKE_PRIVATE_WORDS is unset: an unset list must never read as "
              "clean. Set it per host (never in the repo) or pass --allow-empty.",
              file=sys.stderr)
        return 2
    if not argv:
        print("FAIL: nothing to check: --diff <base>, --text-file <f> or <file>...",
              file=sys.stderr)
        return 2
    hits = []
    if argv[0] == "--diff":
        if len(argv) != 2:
            print("FAIL: --diff takes exactly one base ref", file=sys.stderr)
            return 2
        hits = check_diff(argv[1])
    elif argv[0] == "--text-file":
        if len(argv) != 2:
            print("FAIL: --text-file takes exactly one file", file=sys.stderr)
            return 2
        hits = check_text_file(argv[1])
    else:
        for f in argv:
            hits.extend(check_text_file(f))
    for label, lineno, term, match in hits:
        print(f"FAIL: {label}:{lineno}: {term} (matched {match!r})", file=sys.stderr)
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main())
