"""The script checker (#312): every docs/product/scripts/rec-*.md scene has scenario lines
and spoken lines, scene ids are unique, every `Caption: <menu path> > <title>` names a row
of the keymap (static/keymap.js, parsed like tests/test_media_guide.py does), and the spoken
words give an estimated length (words / WORDS_PER_SECOND per second).

Reusable check functions take a list of paths, so tests can plant a defect in a temporary
script and assert on the returned hits.
"""
import re
from pathlib import Path

import pytest

pytestmark = pytest.mark.media

from test_docs_wording import check_docs_wording
from test_media_guide import menu_titles

ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = ROOT / "docs" / "product" / "scripts"

SCENE_RE = re.compile(r"^##\s+(\S+)\s*$")
CAPTION_RE = re.compile(r"Caption:\s*(.*)$")

# Length calibration (#314): the real Gemini narration of rec-part1.md is 93.65 s for 198
# spoken words and rec-part2.md is 97.63 s for 214 spoken words. The constant is the fit to
# these two: (198 + 214) / (93.65 + 97.63) = 2.15 words per second. rec-part1 and rec-part2
# sit within 5 s of their real values at this constant; the 60 to 95 s check applies to
# rec-part3.md only (the target of the text is 75 to 90 s, the gate is wider).
WORDS_PER_SECOND = 2.15
REAL_SECONDS = {"rec-part1.md": 93.65, "rec-part2.md": 97.63}
PART3_RANGE_S = (60.0, 95.0)
PART3_TOLERANCE_S = 5.0


def scenes_of(path):
    """The scenes of a script as dicts: id, scenario lines (>), spoken lines."""
    scenes = []
    scene = None
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        m = SCENE_RE.match(line)
        if m:
            scene = {"id": m.group(1), "file": str(path), "line": lineno,
                     "scenario": [], "spoken": []}
            scenes.append(scene)
        elif scene is None:
            continue  # the intro before the first scene is meta, not spoken
        elif line.startswith("> "):
            scene["scenario"].append(line)
        elif line.strip():
            scene["spoken"].append(line)
    return scenes


def check_scenes(paths):
    """Hits: a scene without a > line or a spoken line, or a duplicate scene id."""
    hits = []
    for path in paths:
        scenes = scenes_of(path)
        ids = [s["id"] for s in scenes]
        for dup in sorted({i for i in ids if ids.count(i) > 1}):
            hits.append({"file": str(path), "line": 0, "term": f"duplicate scene id {dup}"})
        for s in scenes:
            if not s["scenario"]:
                hits.append({"file": s["file"], "line": s["line"],
                             "term": f"scene {s['id']} has no > line"})
            if not s["spoken"]:
                hits.append({"file": s["file"], "line": s["line"],
                             "term": f"scene {s['id']} has no spoken line"})
    return hits


def check_captions(paths, titles):
    """Hits: a `Caption: X > Y` whose (X, Y) is not a keymap row."""
    hits = []
    for path in paths:
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            m = CAPTION_RE.search(line)
            if not m:
                continue
            caption = m.group(1).strip().rstrip(".").strip('"')
            if " > " not in caption:
                continue  # a caption that names no menu, e.g. "Next: ..."
            parts = caption.rsplit(" > ", 1)
            if (parts[0], parts[1]) not in titles:
                hits.append({"file": str(path), "line": lineno, "term": caption})
    return hits


def spoken_words(path):
    """The spoken word count of a script: after the first scene, without > lines."""
    return sum(len(" ".join(s["spoken"]).split()) for s in scenes_of(path))


def estimated_seconds(path):
    return spoken_words(path) / WORDS_PER_SECOND


def script_files():
    return sorted(SCRIPTS.glob("rec-*.md"))


def test_scripts_structure_captions_length(capsys):
    paths = script_files()
    titles = menu_titles()

    scene_hits = check_scenes(paths)
    caption_hits = check_captions(paths, titles)
    wording_hits = check_docs_wording(paths)

    for hit in scene_hits + caption_hits + wording_hits:
        print(f"{hit['file']}:{hit['line']}: {hit['term']}")

    with capsys.disabled():
        for path in paths:
            n_scenes = len(scenes_of(path))
            n_scenario = sum(len(s["scenario"]) for s in scenes_of(path))
            n_spoken = sum(len(s["spoken"]) for s in scenes_of(path))
            n_captions = sum(len(CAPTION_RE.findall(p))
                             for p in path.read_text(encoding="utf-8").splitlines())
            print(f"{path.name}: {n_scenes} scenes, {n_scenario} scenario lines, "
                  f"{n_spoken} spoken lines, {n_captions} captions, "
                  f"estimated {estimated_seconds(path):.0f} s")

    assert len(paths) >= 2, f"expected rec-part1, rec-part2 and rec-part3, found {len(paths)}"
    assert scene_hits == []
    assert caption_hits == []
    assert wording_hits == []
    for path in paths:
        assert spoken_words(path) > 0, f"no spoken words in {path.name}"
    for name, real in REAL_SECONDS.items():
        length = estimated_seconds(SCRIPTS / name)
        assert abs(length - real) <= PART3_TOLERANCE_S, \
            f"{name} estimated {length:.0f} s, more than {PART3_TOLERANCE_S} s from {real} s"
    part3 = SCRIPTS / "rec-part3.md"
    length = estimated_seconds(part3)
    assert PART3_RANGE_S[0] <= length <= PART3_RANGE_S[1], \
        f"rec-part3.md estimated {length:.0f} s, outside {PART3_RANGE_S} s"


def test_planted_unknown_caption_is_reported(tmp_path):
    bad = tmp_path / "bad-script.md"
    bad.write_text(
        "## p2-bad\n\n> Press the key. Caption: Edit > Frobnicate…\n\nSay something.\n",
        encoding="utf-8")
    hits = check_captions([bad], menu_titles())
    assert len(hits) == 1
    assert "Frobnicate" in hits[0]["term"]
    assert hits[0]["line"] == 3


def test_planted_empty_scene_is_reported(tmp_path):
    bad = tmp_path / "bad-script.md"
    bad.write_text("## p2-empty\n\nSay something with no scenario line.\n", encoding="utf-8")
    hits = check_scenes([bad])
    assert len(hits) == 1
    assert "no > line" in hits[0]["term"]
