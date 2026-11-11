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


# ---- #464: the facts of an explainer script against the code (the placement rule) ----

REFINE = ROOT / "crates" / "core" / "src" / "refine"


def current_rule_version():
    """The version of `RefineRule::CURRENT`, read from `refine/track.rs`: the variant that CURRENT names and
    the number that `version()` gives that variant."""
    src = (REFINE / "track.rs").read_text(encoding="utf-8")
    variant = re.search(r"pub const CURRENT: RefineRule = RefineRule::(\w+);", src).group(1)
    return int(re.search(rf"RefineRule::{variant} => (\d+),", src).group(1))


def rust_constants():
    """The numeric constants of the refine module: name -> value; one alias to another constant is resolved."""
    raw = {}
    for path in REFINE.glob("*.rs"):
        for name, value in re.findall(r"const (\w+): [\w<>]+ = ([^;]+);", path.read_text(encoding="utf-8")):
            raw[name] = value.strip()
    out = {}
    for name, value in raw.items():
        value = raw.get(value, value)
        try:
            out[name] = float(value.replace("_", ""))
        except ValueError:
            continue
    return out


def header_of(text):
    """The meta text before the first scene: the facts used."""
    return text.split("\n## ", 1)[0]


def header_rule_version(text):
    """The first rule version that the header names ("version 10"), or None."""
    m = re.search(r"version (\d+)", header_of(text))
    return int(m.group(1)) if m else None


def check_rule_version(text, current):
    """The hits for a script whose header names a rule version: it must be the code's current one."""
    named = header_rule_version(text)
    if named is None:
        return []
    if named != current:
        return [f"the header names rule version {named}, the code runs version {current}"]
    return []


def check_header_constants(text, constants):
    """The hits for the facts lines (`NAME` = value): the constant must exist and hold the stated value."""
    hits = []
    for name, value in re.findall(r"`([A-Z][A-Z0-9_]+)` = ([0-9.]+)", header_of(text)):
        if name not in constants:
            hits.append(f"the header names the constant {name}, which no refine module defines")
        elif abs(constants[name] - float(value)) > 1e-12:
            hits.append(f"the header says {name} = {value}, the code has {constants[name]:g}")
    return hits


def check_no_kick_threshold(text):
    """The hits for a sentence of the script that gives the kick its own threshold: the analysis has one
    peak margin for every track (`analyze.rs`)."""
    hits = []
    for sentence in re.split(r"(?<=[.!?])\s+|\n", text):
        low = sentence.lower()
        if "kick" in low and re.search(r"threshold|\bdb\b|decibel", low):
            hits.append(f"a kick-specific threshold: {sentence.strip()!r}")
    return hits


def numbers_of(text):
    return set(re.findall(r"\d+(?:\.\d+)?", text))


def check_numbers_in_header(text):
    """The hits for a number that the narration speaks and the facts header does not carry."""
    header = numbers_of(header_of(text))
    spoken = numbers_of(" ".join(s for _, _, s in scenes_of(text)))
    return [f"the narration says {n}, which the facts header does not carry" for n in sorted(spoken - header)]


MAX_SPOKEN_NUMBERS = 5  # the idea carries a scene; a listener keeps two or three numbers, the rest is on screen


def check_spoken_number_count(text):
    """The hits for a narration that speaks more numbers than a listener keeps (numbers go on screen)."""
    spoken = re.findall(r"\d+(?:\.\d+)?", " ".join(s for _, _, s in scenes_of(text)))
    if len(spoken) > MAX_SPOKEN_NUMBERS:
        return [f"the narration speaks {len(spoken)} numbers, {MAX_SPOKEN_NUMBERS} at most: {spoken}"]
    return []


def builder_labels(source):
    """The text that a builder puts on screen: every string of the module but its docstrings and the
    scene ids (`s01-recap`)."""
    import ast
    tree = ast.parse(source)
    skip = set()
    for node in ast.walk(tree):
        if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef)) and node.body \
                and isinstance(node.body[0], ast.Expr) and isinstance(node.body[0].value, ast.Constant):
            skip.add(id(node.body[0].value))
    return [n.value for n in ast.walk(tree)
            if isinstance(n, ast.Constant) and isinstance(n.value, str) and id(n) not in skip
            and not re.fullmatch(r"s\d\d-[\w-]+", n.value)]


def check_labels_in_header(text, source):
    """The hits for a number on the builder's screen that the facts header does not carry."""
    header = numbers_of(header_of(text))
    shown = numbers_of(" ".join(builder_labels(source)))
    return [f"the picture shows {n}, which the facts header does not carry" for n in sorted(shown - header)]


def test_rule_version_is_read_from_the_code():
    assert current_rule_version() == 10


def test_explainer_scripts_name_the_current_rule_version():
    """Every script whose header names a placement rule version names the code's current one; Explainer 4 names one."""
    current = current_rule_version()
    assert header_rule_version(script_text("explainer-04-first-sample.md")) is not None, \
        "Explainer 4's header names no rule version"
    for name in ("explainer-04-first-sample.md", "explainer-15-agent.md"):
        assert check_rule_version(script_text(name), current) == [], name


def test_explainer_4_facts_match_the_code():
    text = script_text("explainer-04-first-sample.md")
    assert check_header_constants(text, rust_constants()) == []
    assert check_no_kick_threshold(text) == []
    assert check_numbers_in_header(text) == []
    assert check_spoken_number_count(text) == []
    source = (ROOT / "tools" / "media" / "explainers" / "explainer_04.py").read_text(encoding="utf-8")
    assert check_labels_in_header(text, source) == []


def test_explainer_4_picture_shows_the_numbers_the_narration_leaves_out():
    """The idea is spoken, the numbers are on screen: each number of the facts header that the narration
    leaves out is in a label of the builder."""
    text = script_text("explainer-04-first-sample.md")
    source = (ROOT / "tools" / "media" / "explainers" / "explainer_04.py").read_text(encoding="utf-8")
    shown = numbers_of(" ".join(builder_labels(source)))
    for n in ("18", "0.1", "5", "0.5", "8", "10", "0.2", "1", "2", "0.06", "0.8", "96"):
        assert n in shown, f"the number {n} is on no label of the picture"
    assert len(numbers_of(" ".join(s for _, _, s in scenes_of(text)))) <= MAX_SPOKEN_NUMBERS


OVERSTATED_EDGE = re.compile(r"almost to the (peak|full height)", re.IGNORECASE)


def test_explainer_4_does_not_say_the_edge_reaches_the_peak():
    """The steep edge is a jump of about one eighth of the peak inside 0.1 ms (EDGE_RISE_DB 18 under the peak),
    not a climb almost to the peak: neither the narration nor a caption of the builder may say so."""
    text = script_text("explainer-04-first-sample.md")
    source = (ROOT / "tools" / "media" / "explainers" / "explainer_04.py").read_text(encoding="utf-8")
    assert not OVERSTATED_EDGE.search(text), "the narration says the edge climbs almost to the peak"
    assert not OVERSTATED_EDGE.search(source), "a caption of the builder says the edge climbs almost to the peak"


def test_planted_overstated_edge_is_matched():
    assert OVERSTATED_EDGE.search("A steep edge: the signal climbs almost to the peak at once.")
    assert OVERSTATED_EDGE.search("climbs almost to the full height")
    assert not OVERSTATED_EDGE.search("the signal jumps by a clear part of the hit's height")


def test_planted_label_number_and_spoken_overload_are_reported():
    header = "Facts. 18 dB.\n\n## s01-x\n\n> a\n\nSpoken.\n"
    assert check_labels_in_header(header, 'Text("a rise of 18 dB in 7 ms")') == [
        "the picture shows 7, which the facts header does not carry"]
    assert check_labels_in_header(header, 'def f():\n    """A docstring of 99."""\n    return "s01-recap"') == []
    many = "Facts.\n\n## s01-x\n\n> a\n\n" + " ".join(str(n) for n in range(1, 8)) + " words.\n"
    assert len(check_spoken_number_count(many)) == 1


def test_explainer_4_facts_list_is_not_empty():
    """The constants check must see the facts: a header whose lines the pattern misses passes it for nothing."""
    names = re.findall(r"`([A-Z][A-Z0-9_]+)` = ([0-9.]+)", header_of(script_text("explainer-04-first-sample.md")))
    assert len(names) >= 12, names


def test_planted_stale_version_is_reported():
    text = 'Facts used (version 3 "first motion", the shipped rule).\n\n## s01-x\n\n> a\n\nSpoken.\n'
    assert check_rule_version(text, 10) == ["the header names rule version 3, the code runs version 10"]
    assert check_rule_version(text.replace("version 3", "version 10"), 10) == []


def test_planted_kick_threshold_is_reported():
    text = "Header.\n\n## s01-x\n\n> a\n\nA kick drum is special, so its threshold is 29 decibels under the peak.\n"
    assert len(check_no_kick_threshold(text)) == 1
    assert check_no_kick_threshold(text.replace("kick", "snare")) == []


def test_planted_wrong_constant_is_reported():
    consts = rust_constants()
    good = "Facts.\n- `EDGE_RISE_DB` = 18 dB.\n\n## s01-x\n\n> a\n\nSpoken.\n"
    assert check_header_constants(good, consts) == []
    assert len(check_header_constants(good.replace("= 18", "= 17"), consts)) == 1
    assert len(check_header_constants(good.replace("EDGE_RISE_DB", "NO_SUCH_CONSTANT"), consts)) == 1


def test_planted_number_missing_from_the_header_is_reported():
    text = "Facts. 18 dB.\n\n## s01-x\n\n> a\n\nA rise of 18 decibels in 7 milliseconds.\n"
    assert check_numbers_in_header(text) == ["the narration says 7, which the facts header does not carry"]


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
