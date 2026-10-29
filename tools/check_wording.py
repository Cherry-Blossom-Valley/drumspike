"""#458: the banned-word guard: docs/product/banned-words.txt over the user-facing docs.

    uv run python tools/check_wording.py [files...]
    uv run python tools/check_wording.py          # the default scope below

Scope (coordinator, #458): docs/guide, docs/product/scripts, the CURRENT release notes
(docs/release-notes/release-notes-0.9.3.md) and site/. Every other doc is listed in
docs/product/banned-words-allowed.txt (one path per line, repo-relative, a `#` comment per
entry on why it stays out) - old release notes and internal plans keep their text, the
guard never rewrites history. FAILs with file:line per hit, exit 1. Wired into
tests/test_media_guide.py. Reusable: check_wording(paths) returns the hits, so tests can
plant a defect in a temporary file.

The rules come from the list file: one entry per line with a `#` why. A plain word is
matched word-bounded and case-insensitively, a multi-word phrase as a substring; the
em-dash entry and the ae/oe/ue transliteration entry are the two special rules. The
transliteration rule compares against an English allow-list: words like "does", "value"
or "queue" carry the same digraphs and are fine; "Groesse" or "fuer" are not.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BANNED_WORDS = ROOT / "docs" / "product" / "banned-words.txt"
ALLOWED_FILES = ROOT / "docs" / "product" / "banned-words-allowed.txt"

# The default scope, relative to the repo root.
SCOPE_GLOBS = (
    "docs/guide/*.md",
    "docs/product/*.md",   # the recording standard joins the guard; the other two are allow-listed
    "docs/product/scripts/*",
    "docs/release-notes/release-notes-*.md",   # the old notes are on the allow-list
    "site/*.html",
)

# The sentinel entries of the list file and the rules they map to.
EM_DASH = "\u2014"
TRANSLIT = "ae/oe/ue"
TRANSLIT_RE = re.compile(r"[aeiou]*?(?:ae|oe|ue)", re.IGNORECASE)

# English words that legitimately carry ae/oe/ue (the transliteration rule only flags words
# outside this set). Extend when a false positive shows up on real text.
TRANSLIT_ALLOWED = frozenset("""
aerobic aerial aerospace aerosol aesthetic afghan algae amazon amazonian antiquity aqueous
archaeology baroque blue blues caesar canoe canoeing clue clueing coerce coexist continue
continued continues crystal doe does dominion due dues duel duet echo eloquent evaluate
evaluation fluent foil foe frequency frequencies fuel fuels goes guest guests heroes hoe
hoes issue issues joey league leagues maelstrom minute oboe oeuvre oregano phoenix poem poet
poetry quaint quartet query quest question questionable queue queued queues quiet quietly
quote quoted quotes request requests require requires rescue rescued rescues roe roentgen
sequence sequel shoe shoes subpoena subsequent suede tahoe tissue tissues toe toes tomatoes
tongue tongues true undoes unique uniquely valuation valuations value valued values vague
vaguely venus virtue segoe
""".split())


def _no_env_names(line):
    """The line without its DRUMSPIKE_-prefixed identifiers: an env var name is code, not
    visible text (the convention of tests/test_docs_wording.py)."""
    return re.sub(r"DRUMSPIKE_[A-Z0-9_]+", "", line)


def load_banned(path=BANNED_WORDS):
    """The rules of the list file: [(kind, token)] with kind in word|phrase|em-dash|translit."""
    rules = []
    for raw in Path(path).read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if line == EM_DASH:
            rules.append(("em-dash", EM_DASH))
        elif line == TRANSLIT:
            rules.append(("translit", TRANSLIT))
        elif " " in line:
            rules.append(("phrase", line))
        else:
            rules.append(("word", line))
    return rules


def load_allowed(path=ALLOWED_FILES):
    """The excluded paths of the allow-list file: {repo-relative path} (a `#` comment per
    entry on why the file stays out of the guard)."""
    out = set()
    if not Path(path).exists():
        return out
    for raw in Path(path).read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].strip()
        if line:
            out.add(line)
    return out


def scope_files(root=ROOT):
    """The default-scope files: SCOPE_GLOBS under root, minus the allow-list. The caller of
    the guard (the test) asserts that every doc file is in the scope or the allow-list."""
    allowed = load_allowed()
    files = []
    for pattern in SCOPE_GLOBS:
        files.extend(p for p in sorted(root.glob(pattern)) if p.is_file())
    return [p for p in files if p.relative_to(root).as_posix() not in allowed]


def hits_in_line(line, rules):
    """The tokens of one line that break a rule."""
    found = []
    text = _no_env_names(line)
    lowered = text.lower()
    for kind, token in rules:
        if kind == "word":
            if re.search(rf"\b{re.escape(token)}\b", lowered):
                found.append(token)
        elif kind == "phrase":
            if token.lower() in lowered:
                found.append(token)
        elif kind == "em-dash":
            if EM_DASH in line:
                found.append("em-dash")
        elif kind == "translit":
            for m in re.finditer(r"[A-Za-z][A-Za-z'\-]*", text):
                word = m.group(0)
                if word[1:].lower() != word[1:]:   # camelCase etc: a code identifier, not prose
                    continue
                if TRANSLIT_RE.search(word) and word.lower() not in TRANSLIT_ALLOWED \
                        and len(word) > 4:
                    found.append(f"transliteration {word!r}")
                    break
    return found


def check_wording(paths, rules=None):
    """The hits over the given files: dicts with file, line and term (the planted-sample
    shape of the guide tests)."""
    rules = rules if rules is not None else load_banned()
    hits = []
    for path in paths:
        for lineno, line in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), start=1):
            for term in hits_in_line(line, rules):
                hits.append({"file": str(path), "line": lineno, "term": term})
    return hits


def main(argv=None):
    if argv is None:
        argv = sys.argv[1:]
    if not argv:   # the default scope
        argv = [str(p) for p in scope_files()]
    if not argv:
        print("FAIL: no files to check (empty scope?)", file=sys.stderr)
        return 2
    rules = load_banned()
    if not rules:
        print(f"FAIL: no rules in {BANNED_WORDS}", file=sys.stderr)
        return 2
    hits = check_wording([Path(a) for a in argv], rules)
    for h in hits:
        print(f"FAIL: {h['file']}:{h['line']}: {h['term']}", file=sys.stderr)
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main())
