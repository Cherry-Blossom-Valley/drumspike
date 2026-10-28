#!/usr/bin/env python3
"""Build the website of Drum Spike into one folder, ready to serve under any sub-path.

    uv run --no-project python tools/media/build_site.py [--out dist/site] [--videos DIR] [--no-videos]

The page (site/index.html) uses relative links only, so the folder works at any path; the
public copy lives at https://drumspike.cherry-blossom-valley.com/ (the Cherry Blossom Valley
website serves the folder under /tools/drum-spike/, and that subdomain shows it). The videos never go into git: --videos names the folder with
the rendered explainers (default ~/.cache/drumspike/media/explainers, the output of
tools/media/explainers/). Every local link and asset of the page must exist in the output;
a missing file is a FAIL (exit 1), unless --no-videos leaves the videos out on purpose.
Prints the file count and the total size.
"""
import argparse
import html.parser
import shutil
import sys
from pathlib import Path
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parent.parent.parent
SITE = ROOT / "site"
DEFAULT_VIDEOS = Path.home() / ".cache/drumspike/media/explainers"


class Refs(html.parser.HTMLParser):
    """Every local href/src/poster of the page."""

    def __init__(self):
        super().__init__()
        self.refs = set()

    def handle_starttag(self, tag, attrs):
        for k, v in attrs:
            if k in ("href", "src", "poster") and v and not v.startswith(("http://", "https://", "mailto:", "#", "data:")):
                self.refs.add(urlparse(v).path)


def local_refs(index: Path) -> set:
    p = Refs()
    p.feed(index.read_text(encoding="utf-8"))
    return p.refs


def build(out: Path, videos: Path | None) -> tuple[int, int, list]:
    """Copy the page, its assets and its videos into out; returns (files, bytes, missing refs)."""
    if out.exists():
        shutil.rmtree(out)
    shutil.copytree(SITE, out, ignore=shutil.ignore_patterns("videos", ".DS_Store"))
    refs = local_refs(SITE / "index.html")
    for ref in sorted(r for r in refs if r.startswith("videos/")):
        if videos is None:
            continue
        src = videos / Path(ref).name
        if src.is_file():
            (out / "videos").mkdir(exist_ok=True)
            shutil.copy2(src, out / ref)
    missing = sorted(r for r in refs if not (out / r).is_file() and not (videos is None and r.startswith("videos/")))
    files = [p for p in out.rglob("*") if p.is_file()]
    return len(files), sum(p.stat().st_size for p in files), missing


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="build the Drum Spike website into one folder")
    ap.add_argument("--out", default=str(ROOT / "dist" / "site"))
    ap.add_argument("--videos", default=str(DEFAULT_VIDEOS), help="folder with the rendered explainer videos")
    ap.add_argument("--no-videos", action="store_true", help="leave the videos out (the posters still show)")
    a = ap.parse_args(argv)
    n, size, missing = build(Path(a.out), None if a.no_videos else Path(a.videos).expanduser())
    for m in missing:
        print(f"FAIL: missing {m}")
    print(f"built {a.out}: {n} files, {size / 1e6:.1f} MB")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
