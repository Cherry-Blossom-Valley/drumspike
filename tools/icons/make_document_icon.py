"""Make the Finder document icon of .drumspike project files (`document.icns`).

A white page with a folded corner (the macOS document style) with the Drum Spike mark of
`drumspike-icon.svg` centered on it. All sizes 16 to 1024 (@1x and @2x) go into an iconset, and
`iconutil` (macOS) packs it.

    uv run --with cairosvg --with pillow python tools/icons/make_document_icon.py [OUT.icns]

The default output is `apps/desktop/src-tauri/icons/document.icns`. `--png SIZE OUT.png` writes one
render instead (for a look at the design).
"""

import io
import subprocess
import sys
import tempfile
from pathlib import Path

import cairosvg
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
ICONS = ROOT / "apps/desktop/src-tauri/icons"
MARK = ICONS / "source/drumspike-icon.svg"
CANVAS = 1024
# The mark file has a 100 px margin; its visible square is 824 px. Shown at this scale on the page.
MARK_SCALE = 0.62

PAGE_SVG = """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
<defs>
 <linearGradient id="pg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#e9ecf1"/></linearGradient>
 <linearGradient id="fd" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#c3c9d4"/></linearGradient>
 <filter id="sh" x="-20%" y="-10%" width="140%" height="130%"><feDropShadow dx="0" dy="14" stdDeviation="16" flood-color="#000" flood-opacity="0.35"/></filter>
</defs>
<path d="M176,100 H684 L848,264 V924 H176 Z" fill="url(#pg)" stroke="#aeb5c2" stroke-width="4" stroke-linejoin="round" filter="url(#sh)"/>
<path d="M684,100 V264 H848 Z" fill="url(#fd)" stroke="#aeb5c2" stroke-width="4" stroke-linejoin="round"/>
</svg>"""


def render(size: int) -> Image.Image:
    """The icon at `size` x `size` pixels: the page, then the mark centered under the fold."""
    page = Image.open(io.BytesIO(cairosvg.svg2png(bytestring=PAGE_SVG.encode(), output_width=size)))
    full = round(size * MARK_SCALE * CANVAS / 824)  # whole mark canvas, so the visible square is MARK_SCALE*size
    mark = Image.open(io.BytesIO(cairosvg.svg2png(url=str(MARK), output_width=full)))
    cx, cy = size * 512 / CANVAS, size * 560 / CANVAS
    page.alpha_composite(mark, (round(cx - full / 2), round(cy - full / 2)))
    return page


def make_icns(out: Path) -> None:
    sizes = {  # iconset file name: pixel size
        "icon_16x16.png": 16,
        "icon_16x16@2x.png": 32,
        "icon_32x32.png": 32,
        "icon_32x32@2x.png": 64,
        "icon_128x128.png": 128,
        "icon_128x128@2x.png": 256,
        "icon_256x256.png": 256,
        "icon_256x256@2x.png": 512,
        "icon_512x512.png": 512,
        "icon_512x512@2x.png": 1024,
    }
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "document.iconset"
        iconset.mkdir()
        cache: dict[int, Image.Image] = {}
        for name, px in sizes.items():
            img = cache.setdefault(px, render(px))
            img.save(iconset / name)
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(out)], check=True)


def main(argv: list[str]) -> int:
    if argv[:1] == ["--png"] and len(argv) == 3:
        render(int(argv[1])).save(argv[2])
        return 0
    make_icns(Path(argv[0]) if argv else ICONS / "document.icns")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
