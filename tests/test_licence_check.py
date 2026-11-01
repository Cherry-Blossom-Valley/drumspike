"""Tests of tools/release/licence_check.py (CBV migration 3, ticket 485).

The Gate: the check lists every file under static/, site/, docs/guide/img/
and the media folders, passes on the real tree, and FAILs on a planted file
that no licensing.md row covers. Synthetic trees in temp dirs, no git.
"""

import sys
from pathlib import Path

TOOLS_RELEASE = Path(__file__).resolve().parent.parent / "tools" / "release"
sys.path.insert(0, str(TOOLS_RELEASE))

import licence_check  # noqa: E402


def _touch(root: Path, rel: str, content: str = "x\n") -> None:
    p = root / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(content, encoding="utf-8")


LICENSING_TABLE = """# Licensing

| Path group | Licence or terms | Source |
|---|---|---|
| `static/*.js` | Apache-2.0 | this repository |
| `static/brand/*` | (c) Cherry Blossom Valley | made for Drum Spike |
| `site/index.html` | Apache-2.0 | this repository |
| `site/assets/posters/*` | (c) Cherry Blossom Valley, AI-generated | made for Drum Spike |
| `docs/guide/img/*` | (c) Cherry Blossom Valley | made for Drum Spike |
"""


def _fake_root(tmp_path: Path) -> Path:
    root = tmp_path / "repo"
    _touch(root, "docs/licensing.md", LICENSING_TABLE)
    _touch(root, "static/app.js")
    _touch(root, "static/brand/cbv-logo-white.png")
    _touch(root, "site/index.html")
    _touch(root, "site/assets/posters/part1.jpg")
    _touch(root, "docs/guide/img/05-export.png")
    return root


def test_rows_read_from_the_table():
    globs = licence_check.rows_from_licensing(
        Path(__file__).resolve().parent.parent / "docs" / "licensing.md"
    )
    assert "static/*.js" in globs
    assert "site/assets/posters/*" in globs
    assert len(globs) == 12  # #255: static/help/img/*, static/help/index.json


def test_real_tree_is_fully_covered():
    root = Path(__file__).resolve().parent.parent
    assert licence_check.uncovered(root) == []
    total = sum(
        len(licence_check.scan(f, root))
        for f in licence_check.SCAN_FOLDERS + licence_check.MEDIA_FOLDERS
    )
    assert total > 30


def test_planted_uncovered_file_fails(tmp_path):
    root = _fake_root(tmp_path)
    assert licence_check.uncovered(root) == []
    _touch(root, "site/assets/cbv-other.bin")
    assert licence_check.uncovered(root) == ["site/assets/cbv-other.bin"]


def test_cli_passes_clean_and_fails_on_plant(tmp_path, capsys):
    root = _fake_root(tmp_path)
    assert licence_check.main(["--root", str(root)]) == 0
    assert "all covered" in capsys.readouterr().out
    _touch(root, "static/vendored-unknown.bin")
    assert licence_check.main(["--root", str(root)]) == 1
    assert "static/vendored-unknown.bin" in capsys.readouterr().err


def test_cli_count(tmp_path, capsys):
    root = _fake_root(tmp_path)
    assert licence_check.main(["--root", str(root), "--count"]) == 0
    assert "6 files" in capsys.readouterr().out
