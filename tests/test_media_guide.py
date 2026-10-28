"""The guide checker (#303): every link resolves, every picture is listed, every menu title
and key exists in the keymap (static/keymap.js, Pro Tools preset), the wording is clean.

Reusable check functions take a list of paths, so tests can plant a defect in a temporary
page and assert on the returned hits.
"""
import json
import re
import sys
from pathlib import Path

import pytest

from test_docs_wording import MODEL_TERMS, EM_DASH

ROOT = Path(__file__).resolve().parent.parent
GUIDE = ROOT / "docs" / "guide"
KEYMAP_JS = ROOT / "static" / "keymap.js"

# A menu title in a page: **File > Add Tracks...** (path > title; a submenu path has its own >).
MENU_TITLE_RE = re.compile(r"\*\*([^*]+?)\*\*")
# A key in a page: `Shift+Cmd+I` or `6` (inline code that is a key: has a + or is one char).
CODE_RE = re.compile(r"`([^`]+)`")
# A picture link: ![alt](img/name.png)
PICTURE_RE = re.compile(r"!\[[^\]]*\]\(([^)]+)\)")
# A normal link: [text](target) that is not a picture link.
LINK_RE = re.compile(r"(?<!\!)\[[^\]]*\]\(([^)]+)\)")


def keymap_rows():
    """The strict JSON table between the KEYMAP markers."""
    text = KEYMAP_JS.read_text(encoding="utf-8")
    m = re.search(r"KEYMAP-BEGIN\*/(\[.*\])/\*KEYMAP-END", text, re.S)
    assert m, "KEYMAP markers not found in static/keymap.js"
    return json.loads(m.group(1))


def menu_titles():
    """{(path, title)} of every menu item."""
    return {(r["path"], r["title"]) for r in keymap_rows() if r.get("menu") is not None}


def keys_of_rows():
    """Every key of the Pro Tools preset: menu_key plus page_keys of all rows."""
    keys = set()
    for r in keymap_rows():
        if r.get("menu_key"):
            keys.add(r["menu_key"])
        keys.update(r.get("page_keys") or [])
    return keys


def is_key_code(code):
    return ("+" in code) or (len(code) == 1)


def check_menu_titles(pages, titles):
    hits = []
    for path in pages:
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            for bold in MENU_TITLE_RE.findall(line):
                if " > " not in bold:
                    continue
                parts = bold.rsplit(" > ", 1)
                row = (parts[0], parts[1])
                if row not in titles:
                    hits.append({"file": str(path), "line": lineno, "term": bold})
    return hits


def check_keys(pages, keys):
    hits = []
    for path in pages:
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            for code in CODE_RE.findall(line):
                if is_key_code(code) and code not in keys:
                    hits.append({"file": str(path), "line": lineno, "term": code})
    return hits


def check_pictures(pages, shots):
    hits = []
    listed = set(shots)
    linked = set()
    for path in pages:
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            for target in PICTURE_RE.findall(line):
                linked.add(target)
                if target not in listed:
                    hits.append({"file": str(path), "line": lineno, "term": f"{target} not in shots.json"})
    for target in sorted(listed - linked):
        hits.append({"file": str(GUIDE / "shots.json"), "line": 0, "term": f"{target} not linked by a page"})
    return hits


def check_links(pages):
    hits = []
    for path in pages:
        text = path.read_text(encoding="utf-8")
        for lineno, line in enumerate(text.splitlines(), start=1):
            for target in LINK_RE.findall(line):
                if PICTURE_RE.search(f"![x]({target})"):
                    continue
                resolved = (path.parent / target.split("#")[0]).resolve()
                if not resolved.exists():
                    hits.append({"file": str(path), "line": lineno, "term": target})
    return hits


def check_wording(pages):
    hits = []
    for path in pages:
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            lowered = line.lower()
            for term in MODEL_TERMS:
                if term in lowered:
                    hits.append({"file": str(path), "line": lineno, "term": term})
            if EM_DASH in line:
                hits.append({"file": str(path), "line": lineno, "term": "em-dash"})
    return hits


def guide_pages():
    return sorted(GUIDE.glob("*.md"))


def shots():
    return json.loads((GUIDE / "shots.json").read_text(encoding="utf-8"))


def test_guide_links_menu_keys_pictures_wording(capsys):
    pages = guide_pages()
    titles = menu_titles()
    keys = keys_of_rows()

    link_hits = check_links(pages)
    title_hits = check_menu_titles(pages, titles)
    key_hits = check_keys(pages, keys)
    picture_hits = check_pictures(pages, shots())
    wording_hits = check_wording(pages)

    for hit in link_hits + title_hits + key_hits + picture_hits + wording_hits:
        print(f"{hit['file']}:{hit['line']}: {hit['term']}")

    # Counts must reach the real output, and none may be 0.
    n_menu_names = sum(
        len([b for b in MENU_TITLE_RE.findall(p.read_text(encoding="utf-8")) if " > " in b])
        for p in pages
    )
    n_keys = sum(len([c for c in CODE_RE.findall(p.read_text(encoding="utf-8")) if is_key_code(c)])
                 for p in pages)
    with capsys.disabled():
        print(f"checked {len(pages)} pages, {n_menu_names} menu names, {n_keys} keys, "
              f"{len(shots())} pictures")

    # 15 chapters plus the README and the best-results page (#323); 3b is the detector
    # plug-in page (#394).
    assert len(pages) == 17, f"expected 17 guide pages, found {len(pages)}"
    assert n_menu_names > 0, "no menu names checked"
    assert n_keys > 0, "no keys checked"
    assert link_hits == []
    assert title_hits == []
    assert key_hits == []
    assert picture_hits == []
    assert wording_hits == []


def test_part_1_pictures_exist_and_are_under_400_kb():
    """#244: every picture of chapters 1 to 5 is in docs/guide/img/, under 400 KB (the pictures of
    chapters 6 to 14 come with their own tickets)."""
    part1 = [name for name, v in shots().items() if v["chapter"] <= 5]
    assert part1, "no picture of chapters 1 to 5 listed"
    missing = [n for n in part1 if not (GUIDE / n).is_file()]
    big = [f"{n} {(GUIDE / n).stat().st_size} bytes" for n in part1
           if (GUIDE / n).is_file() and (GUIDE / n).stat().st_size > 400 * 1024]
    assert missing == [], f"missing pictures: {missing}"
    assert big == [], f"pictures over 400 KB: {big}"


def test_planted_unknown_menu_title_is_reported(tmp_path):
    bad = tmp_path / "bad-page.md"
    bad.write_text("Open the dialog: **File > Frobnicate...** (`Cmd+N`).\n", encoding="utf-8")
    hits = check_menu_titles([bad], menu_titles())
    assert len(hits) == 1
    assert "Frobnicate" in hits[0]["term"]
    assert hits[0]["line"] == 1


def test_planted_unknown_key_is_reported(tmp_path):
    bad = tmp_path / "bad-page.md"
    bad.write_text("Press `Shift+Cmd+F9` to frobnicate.\n", encoding="utf-8")
    hits = check_keys([bad], keys_of_rows())
    assert len(hits) == 1
    assert hits[0]["term"] == "Shift+Cmd+F9"
    assert hits[0]["line"] == 1


# ---------------- the banned-word guard (#458, tools/check_wording.py) ----------------

def test_banned_words_over_docs_and_site():
    """The guard over its default scope (docs/guide, docs/product/scripts, the current
    release notes, site/), and the coverage: every doc file is either in the scope or in
    docs/product/banned-words-allowed.txt."""
    sys.path.insert(0, str(ROOT / "tools"))
    import check_wording   # noqa: E402

    paths = check_wording.scope_files()
    hits = check_wording.check_wording(paths)
    for hit in hits:
        print(f"{hit['file']}:{hit['line']}: {hit['term']}")
    print(f"check_wording: {len(paths)} files, {len(hits)} findings")
    assert len(paths) > 20, f"the wording guard found only {len(paths)} files to check"
    assert hits == [], f"banned-word findings on main's docs: {hits}"

    seen = {p.relative_to(ROOT).as_posix() for p in paths}
    candidates = [p.relative_to(ROOT).as_posix()
                  for pat in ("docs/guide/*.md", "docs/product/*.md", "docs/product/scripts/*",
                              "docs/release-notes/release-notes-*.md", "site/*.html")
                  for p in ROOT.glob(pat) if p.is_file()]
    uncovered = [c for c in candidates if c not in seen
                 and c not in check_wording.load_allowed()]
    assert uncovered == [], f"docs neither in scope nor in the allow-list: {uncovered}"


def test_planted_banned_word_is_reported(tmp_path):
    sys.path.insert(0, str(ROOT / "tools"))
    import check_wording   # noqa: E402

    bad = tmp_path / "bad.md"
    bad.write_text("A clean line.\nThis is a commercial product.\n", encoding="utf-8")
    hits = check_wording.check_wording([bad])
    assert len(hits) == 1
    assert hits[0]["term"] == "commercial"
    assert hits[0]["line"] == 2
    # And through the CLI: exit 1, the finding names file and line.
    assert check_wording.main([str(bad)]) == 1


def test_planted_em_dash_is_reported(tmp_path):
    sys.path.insert(0, str(ROOT / "tools"))
    import check_wording   # noqa: E402

    bad = tmp_path / "bad.md"
    bad.write_text("sample-exact \u2014 always.\n", encoding="utf-8")
    hits = check_wording.check_wording([bad])
    assert len(hits) == 1
    assert hits[0]["term"] == "em-dash"
    assert hits[0]["line"] == 1


def test_planted_transliteration_is_reported(tmp_path):
    sys.path.insert(0, str(ROOT / "tools"))
    import check_wording   # noqa: E402

    bad = tmp_path / "bad.md"
    bad.write_text("Die Groesse ist gross.\n", encoding="utf-8")
    hits = check_wording.check_wording([bad])
    assert len(hits) == 1
    assert "Groesse" in hits[0]["term"]
