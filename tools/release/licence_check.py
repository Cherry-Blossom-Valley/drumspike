#!/usr/bin/env python3
"""Licence coverage check for the non-code assets (CBV migration 3, ticket 485).

Lists every file under static/, site/, docs/guide/img/ and the media folders
and fails on one that no row of docs/licensing.md covers.

    uv run --no-project python tools/release/licence_check.py            # check, exit 1 on a gap
    uv run --no-project python tools/release/licence_check.py --count    # the scanned file count

Standard library only. The rows are read from the first column of the table
in docs/licensing.md: a backticked glob per row.
"""

from __future__ import annotations

import argparse
import fnmatch
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LICENSING_NAME = "docs/licensing.md"

# The scanned folders. MEDIA_FOLDERS are the asset folders of the media
# pipeline that live in the repository (the video posters of the site).
SCAN_FOLDERS = ["static", "site", "docs/guide/img"]
MEDIA_FOLDERS = ["site/assets/posters"]


def rows_from_licensing(path: Path) -> list[str]:
    """The backticked globs of the first column of the licensing table."""
    globs: list[str] = []
    in_table = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("|"):
            in_table = True
            cells = [c.strip() for c in line.split("|")[1:-1]]
            if len(cells) < 3:
                continue  # the header and its separator
            first = cells[0]
            if first.startswith("`") and first.endswith("`"):
                globs.append(first[1:-1])
        elif in_table and not line.startswith("|"):
            in_table = False
    return globs


def scan(folder: str, root: Path) -> list[Path]:
    base = root / folder
    if not base.is_dir():
        return []
    return sorted(p for p in base.rglob("*") if p.is_file())


def uncovered(root: Path, globs: list[str] | None = None) -> list[str]:
    """The repo-relative paths under the scanned folders that no row covers."""
    if globs is None:
        globs = rows_from_licensing(root / LICENSING_NAME)
    missed: list[str] = []
    for folder in SCAN_FOLDERS + MEDIA_FOLDERS:
        for path in scan(folder, root):
            rel = path.relative_to(root).as_posix()
            if not any(fnmatch.fnmatch(rel, g) for g in globs):
                missed.append(rel)
    return missed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--root", type=Path, default=ROOT, help="the repository root")
    parser.add_argument("--count", action="store_true", help="print the scanned file count")
    args = parser.parse_args(argv)
    globs = rows_from_licensing(args.root / LICENSING_NAME)
    if not globs:
        print(f"FAIL: no path groups read from {args.root / LICENSING_NAME}", file=sys.stderr)
        return 1
    files = [p for folder in SCAN_FOLDERS + MEDIA_FOLDERS for p in scan(folder, args.root)]
    if args.count:
        print(f"{len(files)} files under {SCAN_FOLDERS + MEDIA_FOLDERS}")
        return 0
    print(f"scanned {len(files)} files against {len(globs)} path groups")
    missed = uncovered(args.root, globs)
    if missed:
        listed = "\n".join(f"  {m}" for m in missed)
        print(
            f"FAIL: {len(missed)} file(s) covered by no licensing.md row:\n{listed}",
            file=sys.stderr,
        )
        return 1
    print("all covered")
    return 0


if __name__ == "__main__":
    sys.exit(main())
