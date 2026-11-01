#!/usr/bin/env python3
"""#255: build static/help/ from docs/guide — the one source of the in-app manual.

The help is prebuilt HTML at build time (why: no markdown renderer ships in the bundle, the
output is deterministic and the images are resolved once, at build time; the app reads only
static/, and static/ is the Tauri frontendDist). The generator converts every chapter of
docs/guide (all *.md except README.md) into static/help/<stem>.html, copies docs/guide/img
into static/help/img/ and writes static/help/index.json (the manifest the page reads: the
chapter list with their titles and the images of each chapter). The Shortcuts cheat sheet is
NOT generated: static/help.js renders it from static/keymap.js, the one source of the keys.

  uv run --project . python tools/build_help.py           # build (regenerates everything)
  uv run --project . python tools/build_help.py --check   # exit 1 when static/help is missing or stale

The markdown subset is what the guide uses: #..#### headings, paragraphs, `-`/`*` bullet
lists, `1.` ordered lists, ``` fences, ![alt](img/…) images, [text](page.md) links and
http(s) links (marked `ext`: the page opens them in the default browser), **bold**,
*italic* and `inline code`, and simple pipe tables. Everything is HTML-escaped first.
"""
import argparse
import json
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GUIDE = ROOT / "docs" / "guide"
OUT = ROOT / "static" / "help"


def title_of(md: str) -> str:
    """The text of the first `# ` heading, else the file stem."""
    m = re.search(r"^# (.+)$", md, re.M)
    return m.group(1).strip() if m else ""


def inline(text: str) -> str:
    """Inline markdown to HTML. The text is HTML-escaped first, so nothing can inject markup."""
    text = (text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
                .replace('"', "&quot;"))
    text = re.sub(r"`([^`]+)`", r"<code>\1</code>", text)
    text = re.sub(r"!\[([^\]]*)\]\(([^)]+)\)", r'<img src="\2" alt="\1">', text)
    text = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r'<a href="\2">\1</a>', text)
    text = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", text)
    text = re.sub(r"(?<!\*)\*([^*]+)\*(?!\*)", r"<em>\1</em>", text)

    # Targets of images and links resolve against docs/guide; the help pages live at /help/:
    # images become help/img/…, chapter links *.md become their help page, http(s) links get
    # class ext (the page opens them in the default browser, never inside the app window).
    def img(m):
        src, alt = m.group(1), m.group(2)
        return f'<img src="help/{src}" alt="{alt}">'

    def link(m):
        href, body = m.group(1), m.group(2)
        if href.startswith(("http://", "https://")):
            return f'<a class="ext" href="{href}">{body}</a>'
        if href.endswith(".md"):
            return f'<a class="chapter" href="{href[:-3]}.html">{body}</a>'
        return f'<a href="help/{href}">{body}</a>'

    text = re.sub(r'<img src="([^"]+)" alt="([^"]*)">', img, text)
    text = re.sub(r'<a href="([^"]+)">([^<]*)</a>', link, text)
    return text


def read_list(first: str, i: int, pat: str, lines: list[str]) -> tuple[list[str], int]:
    """A list started at lines[i-?]... reads `pat` items plus their indented continuation lines."""
    items = [first]
    while i < len(lines):
        m = re.match(pat, lines[i])
        if m:
            items.append(m.group(1))
        elif lines[i].startswith(" ") and lines[i].strip():
            items[-1] += " " + lines[i].strip()
        else:
            break
        i += 1
    return items, i


def render(md: str) -> tuple[str, list[str]]:
    """Markdown body -> (html, image paths). Line-based; the subset above."""
    lines = md.split("\n")
    out, imgs, i = [], [], 0
    para, in_code, code = [], False, []

    def flush():
        nonlocal para
        if para:
            out.append(f"<p>{inline(' '.join(para))}</p>")
            para = []

    while i < len(lines):
        line = lines[i]
        if in_code:
            if line.strip().startswith("```"):
                out.append("<pre><code>" + "\n".join(code) + "</code></pre>")
                code, in_code = [], False
            else:
                code.append(line)
            i += 1
            continue
        if line.strip().startswith("```"):
            flush()
            in_code = True
            i += 1
            continue
        m = re.match(r"^(#{1,4}) (.+)$", line)
        if m:
            flush()
            out.append(f"<h{len(m.group(1))}>{inline(m.group(2))}</h{len(m.group(1))}>")
            i += 1
            continue
        if re.match(r"^\|.*\|$", line) and i + 1 < len(lines) and re.match(r"^\|[\s:-]+\|", lines[i + 1]):
            flush()
            head = [c.strip() for c in line.strip().strip("|").split("|")]
            i += 2
            rows = []
            while i < len(lines) and lines[i].strip().startswith("|"):
                rows.append([c.strip() for c in lines[i].strip().strip("|").split("|")])
                i += 1
            out.append("<table><thead><tr>" + "".join(f"<th>{inline(c)}</th>" for c in head) +
                       "</tr></thead><tbody>" +
                       "".join("<tr>" + "".join(f"<td>{inline(c)}</td>" for c in r) + "</tr>" for r in rows) +
                       "</tbody></table>")
            continue
        m = re.match(r"^[-*] (.+)$", line)
        if m:
            flush()
            items, i = read_list(m.group(1), i + 1, r"^[-*] (.+)$", lines)
            out.append("<ul>" + "".join(f"<li>{inline(t)}</li>" for t in items) + "</ul>")
            continue
        m = re.match(r"^\d+\. (.+)$", line)
        if m:
            flush()
            items, i = read_list(m.group(1), i + 1, r"^\d+\. (.+)$", lines)
            out.append("<ol>" + "".join(f"<li>{inline(t)}</li>" for t in items) + "</ol>")
            continue
        for im in re.finditer(r"!\[[^\]]*\]\(([^)]+)\)", line):
            imgs.append(im.group(1))
        if line.strip() == "":
            flush()
        else:
            para.append(line.strip())
        i += 1
    flush()
    if in_code:
        raise ValueError("unterminated code fence")
    return "\n".join(out), imgs


def chapters() -> list[Path]:
    return sorted(p for p in GUIDE.glob("*.md") if p.name != "README.md")


def build() -> list[dict]:
    if OUT.exists():
        shutil.rmtree(OUT)
    OUT.mkdir(parents=True)
    shutil.copytree(GUIDE / "img", OUT / "img")
    manifest = []
    for p in chapters():
        md = p.read_text()
        html, _ = render(md)
        (OUT / (p.stem + ".html")).write_text(html)
        manifest.append({"file": p.stem + ".html", "title": title_of(md) or p.stem, "source": f"docs/guide/{p.name}"})
    (OUT / "index.json").write_text(json.dumps({"chapters": manifest}, indent=1) + "\n")
    return manifest


def stale() -> str:
    """Why static/help is out of date, or ''."""
    if not (OUT / "index.json").is_file():
        return "static/help/index.json is missing: run tools/build_help.py"
    have = json.loads((OUT / "index.json").read_text())["chapters"]
    want = [{"file": p.stem + ".html", "title": title_of(p.read_text()) or p.stem, "source": f"docs/guide/{p.name}"} for p in chapters()]
    if [c["file"] for c in have] != [c["file"] for c in want]:
        missing = sorted(set(c["file"] for c in want) - set(c["file"] for c in have))
        return f"guide chapters missing from the help: {missing}; run tools/build_help.py"
    for c in have:
        if not (OUT / c["file"]).is_file():
            return f"help page missing: {c['file']}; run tools/build_help.py"
        src = (GUIDE / Path(c["source"]).name).read_text()
        if (OUT / c["file"]).read_text() != render(src)[0]:
            return f"help page stale: {c['file']}; run tools/build_help.py"
    return ""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="fail instead of building when stale")
    args = ap.parse_args()
    if args.check:
        why = stale()
        if why:
            print(why)
            return 1
        print(f"static/help up to date ({len(chapters())} chapters)")
        return 0
    manifest = build()
    print(f"built static/help: {len(manifest)} chapters, {len(list((OUT / 'img').iterdir()))} images")
    return 0


if __name__ == "__main__":
    sys.exit(main())
