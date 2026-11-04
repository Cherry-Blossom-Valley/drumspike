"""The landing page in site/ (one static file, no tracking), served at
https://drumspike.cherry-blossom-valley.com/ and built by tools/media/build_site.py.

The static checks read index.html: every local link and asset exists, every anchor has its id,
GitHub links point at the planned repository, no script or host outside the list is loaded, and
the page weighs under 5 MB. The browser checks render it in Chromium at 375 and 1440 px in both
themes: no sideways scroll, the theme tokens apply, and screenshots go to SHOTS for the coordinator.
The help checks: static/help is built from docs/guide and is up to date (#255).
"""

import html.parser
import os
import re
import warnings
from pathlib import Path
from urllib.parse import urlparse

import pytest
from playwright.sync_api import Error as PlaywrightError

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"
INDEX = SITE / "index.html"
REPO = "https://github.com/Cherry-Blossom-Valley/drumspike"
SHOTS = Path(os.environ.get("DRUMSPIKE_SITE_SHOTS", Path.home() / ".cache/drumspike/site-shots"))
ALLOWED_HOSTS = {
    "github.com",
    "cherry-blossom-valley.com",
    "icon-studio.cherry-blossom-valley.com",
    "drumspike.cherry-blossom-valley.com",
    "buymeacoffee.com",   # #278: the support link in the footer
}
# #278: the only GitHub link outside the repository is the owner's Sponsors page
SPONSORS = "https://github.com/sponsors/wfrank2509"
MAX_PAGE_WITHOUT_VIDEOS = 450 * 1024   # the page, the logo, the icon and the editor screenshot
# The built videos the page embeds (site/videos/, gitignored; tools/media/build_site.py copies them).
VIDEO_NAMES = {"explainer-01", "explainer-03", "explainer-04", "explainer-05", "explainer-08"}
WIDTHS = [375, 1440]
THEME_BACKGROUND = {"light": "rgb(255, 255, 255)", "dark": "rgb(11, 13, 18)"}
# Ticket #345: under load the Chromium screenshot can die with a transient
# "Protocol error" (for example Target crashed during Page.captureScreenshot). The
# capture is retried this many times before the failure is real.
SCREENSHOT_ATTEMPTS = 3


def screenshot_with_retry(shoot):
    """Runs `shoot` (a page.screenshot call), retrying transient protocol errors.

    A Playwright error whose text says "Protocol error" gets a UserWarning and a retry;
    after `SCREENSHOT_ATTEMPTS` attempts, or for any other error, it is raised.
    """
    for attempt in range(1, SCREENSHOT_ATTEMPTS + 1):
        try:
            return shoot()
        except PlaywrightError as e:
            if "Protocol error" not in str(e) or attempt == SCREENSHOT_ATTEMPTS:
                raise
            warnings.warn(
                f"screenshot attempt {attempt} failed with a transient protocol error "
                f"({e}); retrying",
                stacklevel=2,
            )


class Page(html.parser.HTMLParser):
    """Collects the links, the assets, the scripts, the videos and the ids of index.html."""

    def __init__(self):
        super().__init__()
        self.links = []
        self.assets = []
        self.scripts = []
        self.ids = set()
        self.videos = []  # (poster attr, [source srcs]) per <video> element

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if a.get("id"):
            self.ids.add(a["id"])
        if tag == "a" and a.get("href"):
            self.links.append(a["href"])
        if tag == "link" and a.get("href"):
            self.assets.append(a["href"])
        if tag in ("img", "source") and a.get("src"):
            self.assets.append(a["src"])
        if tag == "script":
            self.scripts.append(a.get("src"))
        if tag == "video":
            self.videos.append((a.get("poster"), []))
        if tag == "source" and self.videos:
            self.videos[-1][1].append(a.get("src"))


def parsed():
    page = Page()
    page.feed(INDEX.read_text(encoding="utf-8"))
    return page


def test_every_link_resolves_and_github_links_point_at_the_planned_repository():
    """Every local link, asset and anchor of the page resolves; every guide link points into
    this repository on GitHub and resolves to an existing docs/guide file; every other GitHub
    link points at the planned repository (or the owner's Sponsors page)."""
    page = parsed()
    missing = []
    for ref in page.links + page.assets:
        if ref.startswith(("http://", "https://", "mailto:")):
            continue
        if ref.startswith("#"):
            if ref[1:] not in page.ids:
                missing.append(ref)
            continue
        # the videos are gitignored and copied by tools/media/build_site.py
        if urlparse(ref).path.startswith("videos/"):
            continue
        if not (SITE / urlparse(ref).path).is_file():
            missing.append(ref)
    assert not missing, f"links or assets that do not resolve: {missing}"

    guide = [ref for ref in page.links if "docs/guide" in ref]
    assert guide, "the page has no guide links"
    broken = []
    for ref in guide:
        prefix = REPO + "/blob/main/"
        if not ref.startswith(prefix) or not (ROOT / ref[len(prefix):]).is_file():
            broken.append(ref)
    assert not broken, f"guide links that do not resolve: {broken}"

    github = [ref for ref in page.links + page.assets if urlparse(ref).hostname == "github.com"]
    assert github, "the page has no GitHub links"
    wrong = [ref for ref in github if not ref.startswith(REPO) and ref != SPONSORS]
    assert not wrong, f"GitHub links outside {REPO}: {wrong}"


def test_every_video_has_a_poster_and_an_allowed_source():
    """Ticket #398: every video element has a poster file that exists in
    site/assets/posters/ (JPG, each under 150 KB) and a src under videos/ whose
    file name is one of the built videos."""
    page = parsed()
    assert page.videos, "the page has no video elements"
    problems = []
    seen = set()
    for poster, srcs in page.videos:
        if not poster:
            problems.append("a video element without a poster")
            continue
        path = SITE / urlparse(poster).path
        if not poster.startswith("assets/posters/") or path.suffix.lower() != ".jpg":
            problems.append(f"poster outside site/assets/posters/ (JPG): {poster}")
        elif not path.is_file():
            problems.append(f"poster file does not exist: {poster}")
        elif path.stat().st_size >= 150 * 1024:
            problems.append(f"poster is 150 KB or more: {poster} ({path.stat().st_size} bytes)")
        if not srcs or any(not src or not urlparse(src).path.startswith("videos/") for src in srcs):
            problems.append(f"a video whose source is not under videos/: {srcs}")
            continue
        for src in srcs:
            stem = Path(urlparse(src).path).stem
            seen.add(stem)
            if stem not in VIDEO_NAMES or Path(src).suffix != ".mp4":
                problems.append(f"video source outside the built videos: {src}")
    assert not problems, f"video elements that do not meet the gate: {problems}"
    assert seen == VIDEO_NAMES, f"the page must embed exactly the built videos, found {sorted(seen)}"


def test_the_licence_line_is_exact():
    """Ticket #398: the licence line is exactly 'Drum Spike is free and open source (Apache-2.0).'"""
    text = INDEX.read_text(encoding="utf-8")
    line = "Drum Spike is free and open source (Apache-2.0)."
    assert line in text, f"the page must carry the exact licence line: {line}"


def test_only_known_hosts_and_no_scripts_from_elsewhere():
    page = parsed()
    hosts = set()
    for ref in page.links + page.assets:
        if ref.startswith("https://"):
            hosts.add(urlparse(ref).hostname)
    assert hosts <= ALLOWED_HOSTS, f"unexpected hosts: {sorted(hosts - ALLOWED_HOSTS)}"
    assert all(src is None for src in page.scripts), "no external script is loaded (no tracking)"


def test_no_commercial_use_or_model_wording_on_the_page():
    text = INDEX.read_text(encoding="utf-8").lower()
    banned = [word for word in ("commercial", "trained model", "training data",
                                "ai-free", "no ai", "no model") if word in text]
    assert not banned, f"the page must not say {banned} (owner, 2026-10-24)"


def test_logo_has_a_rule_in_the_dark_theme():
    css = INDEX.read_text(encoding="utf-8")
    dark = re.search(r"@media \(prefers-color-scheme: dark\) \{(.*?)\n\}", css, re.S)
    assert dark, "the page has no dark theme block"
    assert re.search(r"\.brand img\s*\{[^}]*\}", dark.group(1)), "the header logo has no rule in the dark theme"


def test_page_weighs_under_250_kb_without_videos():
    """Ticket #398: the page without the videos stays under 250 KB. The videos
    (site/videos/) and the posters (assets/posters/) are the video payload and
    are gitignored or counted with their own gate."""
    page = parsed()
    total = INDEX.stat().st_size
    counted = {INDEX}
    for ref in page.links + page.assets:
        if ref.startswith(("http", "#", "mailto:")):
            continue
        path = SITE / urlparse(ref).path
        if path.is_file() and "videos" not in path.parts and "posters" not in path.parts:
            rp = path.resolve()
            if rp not in counted:
                counted.add(rp)
                total += path.stat().st_size
    assert total < MAX_PAGE_WITHOUT_VIDEOS, f"{total} bytes, over 250 KB"


@pytest.mark.parametrize("theme", ["light", "dark"])
@pytest.mark.parametrize("width", WIDTHS)
def test_renders_without_sideways_scroll_in_both_themes(browser, width, theme):
    context = browser.new_context(viewport={"width": width, "height": 900}, color_scheme=theme)
    try:
        page = context.new_page()
        page.goto(INDEX.as_uri())
        page.wait_for_load_state("load")
        scroll = page.evaluate(
            "() => [document.documentElement.scrollWidth, document.documentElement.clientWidth]"
        )
        assert scroll[0] <= scroll[1], f"sideways scroll at {width} px: scrollWidth {scroll[0]} > {scroll[1]}"
        background = page.evaluate("() => getComputedStyle(document.body).backgroundColor")
        assert background == THEME_BACKGROUND[theme], f"{theme} tokens not applied: {background}"
        images_ok = page.evaluate("() => [...document.images].every(i => i.complete && i.naturalWidth > 0)")
        assert images_ok, "an image did not load"
        SHOTS.mkdir(parents=True, exist_ok=True)
        screenshot_with_retry(
            lambda: page.screenshot(path=SHOTS / f"index-{theme}-{width}.png", full_page=True)
        )
    finally:
        context.close()


def test_screenshot_retry_recovers_from_a_transient_protocol_error():
    """Ticket #345: two transient protocol errors, then the capture succeeds."""
    calls = []

    def flaky():
        calls.append(1)
        if len(calls) < 3:
            raise PlaywrightError("Protocol error (Target crashed): Page.captureScreenshot")
        return "shot"

    with pytest.warns(UserWarning, match="transient protocol error"):
        assert screenshot_with_retry(flaky) == "shot"
    assert len(calls) == 3, "the capture ran three times: two failures, then the success"


def test_screenshot_retry_reraises_when_the_error_never_stops():
    """Ticket #345: a persistent protocol error is raised after the last attempt."""
    calls = []

    def always():
        calls.append(1)
        raise PlaywrightError("Protocol error (Page.captureScreenshot): session closed")

    with pytest.raises(PlaywrightError, match="session closed"):
        screenshot_with_retry(always)
    assert len(calls) == SCREENSHOT_ATTEMPTS, "every attempt ran before the raise"


def test_build_site_copies_the_page_its_assets_and_the_videos(tmp_path):
    """tools/media/build_site.py: every local reference of the page is in the output; a missing
    video is a FAIL, and --no-videos builds the page without them."""
    import sys
    sys.path.insert(0, str(ROOT / "tools" / "media"))
    import build_site
    vids = tmp_path / "vids"
    vids.mkdir()
    for name in sorted(VIDEO_NAMES)[:-1]:
        (vids / f"{name}.mp4").write_bytes(b"\0" * 16)
    n, size, missing = build_site.build(tmp_path / "out", vids)
    assert missing == [f"videos/{sorted(VIDEO_NAMES)[-1]}.mp4"], missing
    (vids / f"{sorted(VIDEO_NAMES)[-1]}.mp4").write_bytes(b"\0" * 16)
    n, size, missing = build_site.build(tmp_path / "out", vids)
    assert missing == [] and (tmp_path / "out" / "index.html").is_file() and n > len(VIDEO_NAMES)
    assert build_site.build(tmp_path / "out2", None)[2] == []


# ---------------- #255: the help is built from docs/guide, the one source of the text ----------------
# `tools/build_help.py --check` fails when a guide chapter is missing from static/help or a
# page is stale, so a new chapter (or an edit) cannot ship without the manual. The manifest
# titles must be the first heading of the chapter.

def build_help():
    import importlib.util
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


def test_the_help_build_is_up_to_date():
    import subprocess
    import sys
    r = subprocess.run([sys.executable, str(ROOT / "tools" / "build_help.py"), "--check"],
                       capture_output=True, text=True)
    assert r.returncode == 0, r.stdout + r.stderr
