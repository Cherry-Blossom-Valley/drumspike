"""#255: the help is built from docs/guide — the one source of the text.

`tools/build_help.py --check` fails when a guide chapter is missing from static/help or a
page is stale, so a new chapter (or an edit) cannot ship without the manual. The manifest
titles must be the first heading of the chapter.
"""
import importlib.util
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def build_help():
    spec = importlib.util.spec_from_file_location("build_help", ROOT / "tools" / "build_help.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_every_guide_chapter_is_in_the_help():
    mod = build_help()
    chapters = mod.chapters()
    assert chapters, "docs/guide has no chapters"
    manifest = __import__("json").loads((ROOT / "static" / "help" / "index.json").read_text())["chapters"]
    files = [c["file"] for c in manifest]
    for p in chapters:
        assert p.stem + ".html" in files, f"guide chapter missing from the help: {p.name}"
    for c in manifest:
        assert (ROOT / "static" / "help" / c["file"]).is_file(), f"help page missing: {c['file']}"


def test_the_build_is_up_to_date():
    r = subprocess.run([sys.executable, str(ROOT / "tools" / "build_help.py"), "--check"],
                       capture_output=True, text=True)
    assert r.returncode == 0, r.stdout + r.stderr
