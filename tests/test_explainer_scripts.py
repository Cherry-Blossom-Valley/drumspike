"""The explainer script checker (#382): the scene ids of the Training Mode explainer are
present and in order, the purpose scene comes first, and its narration is 3 sentences at most
and names no API (no DSAgent, no .jsonl, no function name); the process scenes follow in the
order on, work, log, export, measure, loop. Explainer 15's Training Mode scenes are one
sentence each and point to chapter 16.

Reusable: the checker takes the script text, so tests can plant a defect in a temporary
script and assert on the hits (the shape of tests/test_guide.py).
"""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = ROOT / "docs" / "product" / "scripts"

# The scenes of explainer 16 in order: the purpose first, then the user's process end to
# end (on, work, log, export, measure, loop).
TRAINING_SCENES = ("s01-why", "s02-on", "s03-work", "s04-log", "s05-export", "s06-measure",
                   "s07-loop")

# An API name must not reach the purpose scene's narration: the page helper object, the log
# file's suffix, and the function, command and tool names of the feature. "Training Mode" the
# feature name is fine; the snake_case ids are not spoken.
API_NAMES = ("DSAgent", ".jsonl", "training_status", "training_export", "setTraining",
             "training_mode", "training_log", "events_edit", "placement_diag",
             "trainingTruth", "trainingExport")


def scenes_of(text):
    """The scenes of a script as (id, visual notes, spoken text) in script order; the `>` lines
    are visual notes and are not spoken, the lines before the first scene are meta."""
    scenes = []
    for line in text.splitlines():
        if line.startswith("## "):
            scenes.append((line[3:].strip(), [], []))
        elif not scenes:
            continue
        elif line.lstrip().startswith(">"):
            scenes[-1][1].append(line.lstrip()[1:].strip())
        elif not line.startswith(("#", "-")) and line.strip():
            scenes[-1][2].append(line.strip())
    return [(sid, notes, " ".join(spoken)) for sid, notes, spoken in scenes]


def sentences_of(spoken):
    """The sentences of a narration: split after a full stop, a question or an exclamation mark."""
    return [s for s in re.split(r"(?<=[.!?])\s+", spoken.strip()) if s]


def check_training_script(text):
    """The hits of the #382 gate over the script text: the scene ids in order, the purpose
    scene first with 3 sentences at most and no API name in it."""
    hits = []
    scenes = scenes_of(text)
    ids = [sid for sid, _, _ in scenes]
    if ids != list(TRAINING_SCENES):
        hits.append(f"scene ids {ids} are not the gate's scenes in order {list(TRAINING_SCENES)}")
        return hits
    for sid, notes, spoken in scenes:
        if not notes:
            hits.append(f"scene {sid} has no visual note")
        if not spoken:
            hits.append(f"scene {sid} has no spoken text")
    purpose = scenes[0][2]
    n = len(sentences_of(purpose))
    if not 1 <= n <= 3:
        hits.append(f"the purpose scene has {n} sentences, 3 at most")
    for api in API_NAMES:
        if api in purpose:
            hits.append(f"the purpose scene names the API {api!r}")
    if "(" in purpose or ")" in purpose:
        hits.append("the purpose scene narration carries a function call")
    return hits


def script_text(name):
    return (SCRIPTS / name).read_text(encoding="utf-8")


def test_explainer_16_purpose_first_process_in_order():
    hits = check_training_script(script_text("explainer-16-training-mode.md"))
    assert hits == []


def test_explainer_15_training_scenes_point_to_chapter_16():
    """Explainer 15's Training Mode scenes are one sentence each and point to chapter 16 (#382)."""
    scenes = {sid: spoken for sid, _, spoken in scenes_of(script_text("explainer-15-agent.md"))}
    for sid in ("s05-training", "s06-loop"):
        spoken = scenes[sid]
        sents = sentences_of(spoken)
        assert len(sents) == 1, f"{sid} narrates {len(sents)} sentences, one at most: {sents}"
        assert "chapter 16" in sents[0], f"{sid} does not point to chapter 16: {spoken}"


def test_builder_carries_explainer_16():
    """build.py renders the id 16 from the module and the scene class of the new explainer."""
    text = (ROOT / "tools" / "media" / "explainers" / "build.py").read_text(encoding="utf-8")
    m = re.search(r'"16":\s*\("explainer-16-training-mode",\s*"explainer_16\.py",'
                  r'\s*"Explainer16"\)', text)
    assert m, "build.py has no entry for explainer 16"


def test_planted_wrong_scene_order_is_reported(tmp_path):
    bad = tmp_path / "bad-order.md"
    body = "\n\n".join(f"## {sid}\n\n> a note\n\nSome narration here.\n"
                       for sid in TRAINING_SCENES[1:] + TRAINING_SCENES[:1])
    bad.write_text(body, encoding="utf-8")
    hits = check_training_script(bad.read_text(encoding="utf-8"))
    assert len(hits) == 1 and "in order" in hits[0]


def test_planted_api_name_and_long_purpose_are_reported(tmp_path):
    body = "\n\n".join(
        f"## {sid}\n\n> a note\n\n{text}\n"
        for sid, text in [
            ("s01-why", "One. Two. Three. Four with DSAgent and the .jsonl file."),
            ("s02-on", "Turn it on."),
            ("s03-work", "Work."),
            ("s04-log", "The log."),
            ("s05-export", "The export."),
            ("s06-measure", "The measure."),
            ("s07-loop", "The loop."),
        ])
    bad = tmp_path / "bad-purpose.md"
    bad.write_text(body, encoding="utf-8")
    hits = check_training_script(bad.read_text(encoding="utf-8"))
    assert any("4 sentences" in h for h in hits), hits
    assert any("DSAgent" in h for h in hits), hits
    assert any(".jsonl" in h for h in hits), hits


def test_planted_scene_without_visual_note_is_reported(tmp_path):
    body = "\n\n".join(
        f"## {sid}\n\n{text}\n" for sid, text in
        [(sid, "Some narration.") for sid in TRAINING_SCENES])
    bad = tmp_path / "no-notes.md"
    bad.write_text(body, encoding="utf-8")
    hits = check_training_script(bad.read_text(encoding="utf-8"))
    assert len(hits) == len(TRAINING_SCENES), hits
    assert all("no visual note" in h for h in hits), hits


# ---- #465: the facts of Explainers 10, 15 and 16 against the code and the product's words ----

AGENT_JS = ROOT / "static" / "agent.js"
TRAINING_RS = ROOT / "crates" / "app" / "src" / "training.rs"


def constants_defined(*paths):
    """The names that the files define as a constant: `const NAME: ...` of Rust, and every `NAME = ...` of a JS
    `const A = 1, B = 2;` list."""
    names = set()
    for path in paths:
        text = path.read_text(encoding="utf-8")
        names |= set(re.findall(r"\bconst\s+([A-Z][A-Z0-9_]+)\s*:", text))
        names |= set(re.findall(r"\b([A-Z][A-Z0-9_]{3,})\s*=[^=>]", text))
    return names


def check_constants_exist(text, defined):
    """The hits for a backticked CONSTANT_NAME of the script that none of the named source files defines."""
    named = set(re.findall(r"`([A-Z][A-Z0-9_]{3,})`", text))
    return [f"the script names the constant {n}, which the code does not define" for n in sorted(named - defined)]


def check_no_stale_phrase(text):
    """The hits for the claim that an agent's record is byte for byte a person's (every record carries `by`)."""
    if "byte for byte the record" in text:
        return ["the script says the record of an agent is byte for byte the record of a person"]
    return []


def test_explainer_15_names_no_constant_absent_from_the_code():
    defined = constants_defined(AGENT_JS, TRAINING_RS)
    assert check_constants_exist(script_text("explainer-15-agent.md"), defined) == []


def test_no_script_says_the_agent_record_is_byte_for_byte_a_persons():
    for path in sorted(SCRIPTS.glob("*.md")):
        assert check_no_stale_phrase(path.read_text(encoding="utf-8")) == [], path.name


def test_no_script_names_a_commit_as_its_source():
    """A facts header checks against the code of a release, not against a commit that moves on."""
    for path in sorted(SCRIPTS.glob("explainer-*.md")):
        assert not re.search(r"commit [0-9a-f]{7,}", path.read_text(encoding="utf-8")), path.name


def test_planted_missing_constant_and_stale_claim_are_reported():
    assert check_constants_exist("Facts: `LINE_MAX_SPREAD` and `LINE_LEAD`.", {"LINE_LEAD"}) == [
        "the script names the constant LINE_MAX_SPREAD, which the code does not define"]
    assert len(check_no_stale_phrase("so the record of an agent is byte for byte the record of a person")) == 1
    assert check_no_stale_phrase("the record is the same kind, marked as made by an agent") == []
