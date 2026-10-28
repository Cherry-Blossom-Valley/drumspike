"""Docs and UI wording guards (#301, #324): docs/product names no model and uses no
em-dash; the user-visible strings of static/ name no model.

Plain hyphens only in documents (AGENTS.md), and the product docs name no trained
model (owner decision 2026-10-24: "Name no model licence: main has no model").
The static/ strings (#324; widened to all of static/ on the coordinator's request
of 2026-10-25) are checked with the comments stripped first: a comment may discuss
the model; what the user reads may not name it. The word "model" is matched
word-bounded and case-insensitively ("no_model", "models", a `DRUMSPIKE_`-prefixed
env var name pass; "model", "Model", "model error" fail). The ${...} interpolations of a template
literal are code, not visible text. In HTML the text nodes and the user-facing
attributes (title, placeholder, alt, aria-label) are what the user reads.
"""

import re
from pathlib import Path

import pytest

PRODUCT_DOCS = Path(__file__).resolve().parent.parent / "docs" / "product"
STATIC_DIR = Path(__file__).resolve().parent.parent / "static"

# Model names the product docs must not mention (case-insensitive); split so this
# checker itself does not carry the model names on main (owner rule, #349/#365).
MODEL_TERMS = ("ad" + "tof", "on" + "nx", "frame" + "_rnn")
EM_DASH = "\u2014"

# The word a static/ string must not say to the user (#324: no model wording).
MODEL_WORD = re.compile(r"\bmodel\b", re.IGNORECASE)

# The HTML attributes whose values the user reads (the others are code).
HTML_USER_ATTRS = ("title", "placeholder", "alt", "aria-label")


def check_docs_wording(files):
    """Check .md files; return the hits as dicts with file, line and term.

    files: an iterable of Path objects to check (the product docs, or a
    temporary file from a test).
    """
    hits = []
    for path in sorted(files):
        text = Path(path).read_text(encoding="utf-8")
        for lineno, line in enumerate(text.splitlines(), start=1):
            lowered = line.lower()
            for term in MODEL_TERMS:
                if term in lowered:
                    hits.append({"file": str(path), "line": lineno, "term": term})
            if EM_DASH in line:
                hits.append({"file": str(path), "line": lineno, "term": "em-dash"})
    return hits


def strip_js_comments(source):
    """Remove // and /* */ comments from JS source, aware of strings.

    ", ' and ` literals with backslash escapes are kept whole, so a comment
    marker inside a string survives and a quoted text inside a comment does
    not open a string. Template literals are scanned like strings.
    """
    out = []
    i, n = 0, len(source)
    while i < n:
        c = source[i]
        if c in "\"'`":
            quote = c
            out.append(c)
            i += 1
            while i < n:
                c = source[i]
                out.append(c)
                if c == "\\" and i + 1 < n:
                    out.append(source[i + 1])
                    i += 2
                    continue
                i += 1
                if c == quote:
                    break
        elif c == "/" and source[i + 1 : i + 2] == "/":
            i += 2
            while i < n and source[i] != "\n":
                i += 1
        elif c == "/" and source[i + 1 : i + 2] == "*":
            i += 2
            while i + 1 < n and source[i : i + 2] != "*/":
                i += 1
            i += 2
        else:
            out.append(c)
            i += 1
    return "".join(out)


def strip_css_comments(source):
    """Remove /* */ comments from CSS source, aware of strings (CSS has no // comment)."""
    out = []
    i, n = 0, len(source)
    while i < n:
        c = source[i]
        if c in "\"'":
            quote = c
            out.append(c)
            i += 1
            while i < n:
                c = source[i]
                out.append(c)
                if c == "\\" and i + 1 < n:
                    out.append(source[i + 1])
                    i += 2
                    continue
                i += 1
                if c == quote:
                    break
        elif c == "/" and source[i + 1 : i + 2] == "*":
            i += 2
            while i + 1 < n and source[i : i + 2] != "*/":
                i += 1
            i += 2
        else:
            out.append(c)
            i += 1
    return "".join(out)


def js_string_literals(source):
    """The string literals (", ', `) of JS source; call it on comment-free source."""
    literals = []
    i, n = 0, len(source)
    while i < n:
        c = source[i]
        if c in "\"'`":
            quote = c
            i += 1
            buf = []
            while i < n:
                c = source[i]
                if c == "\\" and i + 1 < n:
                    buf.append(source[i + 1])
                    i += 2
                    continue
                if c == quote:
                    i += 1
                    break
                buf.append(c)
                i += 1
            literals.append("".join(buf))
        else:
            i += 1
    return literals


def strip_template_exprs(literal):
    """The visible chunks of a template literal: the ${...} interpolations are code."""
    return re.sub(r"\$\{[^}]*\}", "", literal)


def strip_html_comments(source):
    """Remove <!-- --> comments from HTML source."""
    return re.sub(r"<!--.*?-->", "", source, flags=re.S)


def html_visible_strings(source):
    """The strings of an HTML document the user reads: the text nodes and the
    user-facing attribute values. Inline <script>/<style> bodies are code.
    """
    text = strip_html_comments(source)
    text = re.sub(r"<(script|style)\b[^>]*>.*?</\1\s*>", "", text, flags=re.S | re.I)
    strings = []
    for m in re.finditer(r"\b([a-zA-Z][\w-]*)\s*=\s*(\"([^\"]*)\"|'([^']*)')", text):
        if m.group(1).lower() in HTML_USER_ATTRS:
            strings.append(m.group(3) if m.group(3) is not None else m.group(4))
    strings.append(re.sub(r"<[^>]*>", "\n", text))   # the tags go, the text between them stays
    return strings


def model_word_hits(strings):
    """The strings that say the word "model" (word-bounded, case-insensitive)."""
    return [s for s in strings if MODEL_WORD.search(s)]


def static_files():
    """Every file of static/ (the user-visible surface ships there whole)."""
    return sorted(p for p in STATIC_DIR.iterdir() if p.is_file())


def static_user_strings():
    """The user-visible strings of static/ as (file name, string) pairs.

    .js: the string literals of the comment-free source, template interpolations
    stripped. .html: the text nodes and the user-facing attribute values.
    .css: the string literals of the comment-free source.
    """
    collected = []
    for path in static_files():
        source = path.read_text(encoding="utf-8")
        if path.suffix == ".js":
            for lit in js_string_literals(strip_js_comments(source)):
                collected.append((path.name, strip_template_exprs(lit)))
        elif path.suffix == ".html":
            for s in html_visible_strings(source):
                collected.append((path.name, s))
        elif path.suffix == ".css":
            for lit in js_string_literals(strip_css_comments(source)):
                collected.append((path.name, lit))
    return collected


def product_doc_files():
    return sorted(PRODUCT_DOCS.rglob("*.md"))


def test_product_docs_have_no_model_mentions_and_no_em_dash(capsys):
    files = product_doc_files()
    hits = check_docs_wording(files)
    for hit in hits:
        print(f"{hit['file']}:{hit['line']}: {hit['term']}")
    assert len(files) > 0, "no docs/product/*.md found"
    assert hits == []
    # The count must reach the real output (capsys would swallow it).
    with capsys.disabled():
        print(f"checked {len(files)} files")


def test_static_strings_name_no_model(capsys):
    collected = static_user_strings()
    assert collected, "no user-visible strings found in static/"
    hits = [(name, s) for name, s in collected if MODEL_WORD.search(s)]
    for name, s in hits:
        print(f"{name} string: {s[:100]}")
    assert hits == []
    # The count must reach the real output (capsys would swallow it).
    with capsys.disabled():
        per_file = {}
        for name, _s in collected:
            per_file[name] = per_file.get(name, 0) + 1
        breakdown = ", ".join(f"{k} {v}" for k, v in sorted(per_file.items()))
        print(f"checked {len(collected)} strings across {len(per_file)} static/ files ({breakdown})")


def test_model_word_is_word_bounded_and_case_insensitive():
    assert MODEL_WORD.search("No usable model: fallback detector")
    assert MODEL_WORD.search("model error: x")
    assert MODEL_WORD.search("the Model finds the hits of this track")
    assert not MODEL_WORD.search("no_model")
    assert not MODEL_WORD.search("DRUMSPIKE_" + "MODEL")
    assert not MODEL_WORD.search("models/ad" + "tof.on" + "nx")


def test_template_interpolations_are_code_not_visible_text():
    assert not MODEL_WORD.search(strip_template_exprs("detector error: ${b.model.error}"))
    assert MODEL_WORD.search(strip_template_exprs("model: ${x}"))


def test_js_comment_stripper_keeps_strings_and_drops_comments():
    source = (
        "// a comment with the model\n"
        'const a = "keep the model"; /* block with the model */\n'
    )
    literals = js_string_literals(strip_js_comments(source))
    assert literals == ["keep the model"]


def test_html_strings_are_text_nodes_and_user_attrs_only():
    source = (
        "<!-- a comment with the model -->\n"
        '<div class="model-view" title="the model drawer">pick the model</div>\n'
        '<input placeholder="model name">'
    )
    hits = model_word_hits(html_visible_strings(source))
    assert len(hits) == 3, hits   # title, text node, placeholder; class and comment are code


def test_css_strings_are_checked_comments_are_not():
    source = '/* a comment with the model */\n.x::after { content: "the model"; }'
    literals = js_string_literals(strip_css_comments(source))
    assert literals == ["the model"]
    assert model_word_hits(literals) == literals


def test_temp_app_js_with_the_old_string_is_reported(tmp_path):
    bad = tmp_path / "app.js"
    bad.write_text(
        'el.count.textContent = "no model: the slider applies on the next re-analyze";\n',
        encoding="utf-8",
    )
    literals = js_string_literals(strip_js_comments(bad.read_text(encoding="utf-8")))
    assert literals == ["no model: the slider applies on the next re-analyze"]
    assert model_word_hits([strip_template_exprs(l) for l in literals]) == literals


def test_temp_html_with_the_old_string_is_reported(tmp_path):
    bad = tmp_path / "index.html"
    bad.write_text("<div title=\"Own detection: the model finds the hits.\"></div>\n", encoding="utf-8")
    hits = model_word_hits(html_visible_strings(bad.read_text(encoding="utf-8")))
    assert hits == ["Own detection: the model finds the hits."]


def test_temp_file_with_model_name_is_reported(tmp_path):
    bad = tmp_path / "bad-model.md"
    # the forbidden name, split so this checker file carries no model name itself
    bad.write_text("A clean line.\nMentions the " + "AD" + "TOF" + " model here.\n", encoding="utf-8")
    hits = check_docs_wording([bad])
    assert len(hits) == 1
    assert Path(hits[0]["file"]) == bad
    assert hits[0]["line"] == 2
    assert hits[0]["term"] == "ad" + "tof"


def test_temp_file_with_em_dash_is_reported(tmp_path):
    bad = tmp_path / "bad-dash.md"
    bad.write_text("First line.\nA range 1\u20142 and a break \u2014 here.\n", encoding="utf-8")
    hits = check_docs_wording([bad])
    em_dash_hits = [h for h in hits if h["term"] == "em-dash"]
    assert len(em_dash_hits) == 1
    assert Path(hits[0]["file"]) == bad
    assert em_dash_hits[0]["line"] == 2


def test_audio_spike_guide_line_says_the_body_not_1_5_cycles():
    """#402: the Audio line of the export guide describes what #378 does (the hit's body:
    the smoothed low tone over its first three clear swings, cut at the zero crossing). The
    old "about 1.5 cycles" line was wrong and must not come back."""
    page = Path(__file__).resolve().parent.parent / "docs" / "guide" / "05-export.md"
    text = page.read_text(encoding="utf-8")
    assert "1.5 cycles" not in text, (
        f"{page.name} says '1.5 cycles' again; the Audio Spike (#378) covers the hit's body, "
        "not a fixed cycle count")
    assert "zero crossing" in text and "800 Hz" in text, (
        f"{page.name} lost the Audio Spike rule: the smoothed low tone below 800 Hz, three "
        "clear swings, the cut at the zero crossing")


def test_demo_kick_length_is_the_measured_value():
    """#402 review: the demo kick's snippet length in the guide caption and the explainer 05
    script is the value the Audio Spike rule measures on that hit (crates/core/src/spike.rs,
    mirrored by tools/media/explainers/audio_spike.py), not a typed number."""
    import json
    import sys

    root = Path(__file__).resolve().parent.parent
    demo = Path.home() / ".cache" / "drumspike" / "media" / "demo"
    if not (demo / "truth.json").is_file():
        pytest.skip("no demo session on this host")
    sys.path.insert(0, str(root / "tools" / "media" / "explainers"))
    import audio_spike as A
    import physics as P

    truth = json.loads((demo / "truth.json").read_text(encoding="utf-8"))
    kicks = [h["t"] / truth["sr"] for h in truth["hits"] if h["drum"] == "kick"]
    x = P.demo_track("k in")
    start = round(kicks[3] * A.RATE)
    end, found = A.snippet_end(x, A.low_component(x), start)
    assert found, "the demo kick has no counted-lobe snippet"
    ms = f"{(end - start) / A.RATE * 1000:.1f}"

    guide = (root / "docs" / "guide" / "05-export.md").read_text(encoding="utf-8")
    assert f"this demo kick: {ms} ms" in guide, (
        f"05-export.md does not carry the measured demo kick length {ms} ms; re-run "
        "tools/media/fig_audio_spike.py and update the caption")

    script = (root / "docs" / "product" / "scripts" /
              "explainer-05-audio-spike.md").read_text(encoding="utf-8")
    assert f"measures {ms} ms" in script, (
        f"explainer-05 script does not say the measured demo kick length {ms} ms")
