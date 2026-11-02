"""Tests of tools/media/compose.py: the clip-list parser, the scene timing on the 30 fps
frame grid, and a composed file on stub clips (tiny lavfi videos, stub narration WAVs).
The parser and timing tests are pure; the end-to-end tests render the title card with
Playwright (as the real run does). No network, no TTS API."""
import json
import math
import os
import shutil
import subprocess
import sys
import wave
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools" / "media"))
import compose  # noqa: E402
import record  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
# The gate host's media dir; DRUMSPIKE_GATE_MEDIA redirects it (the skip-per-missing-clip
# check runs against a temp set that has the recording clip but no explainer clip).
GATE_MEDIA = Path(os.environ.get(
    "DRUMSPIKE_GATE_MEDIA", str(Path.home() / ".cache" / "drumspike" / "media")))
MEDIA_ROOT = str(Path.home() / ".cache" / "drumspike" / "media")
GATE_TTS = Path("/Users/Shared/drumspike-media/tts-out")


def gated_plan_raw():
    """The product-video plan with its media root rewritten to GATE_MEDIA."""
    raw = json.loads((ROOT / "tools/media/product-video.json").read_text(encoding="utf-8"))
    for sc in raw["scenes"]:
        for cl in sc["clips"]:
            src = cl.get("source")
            if isinstance(src, str) and os.path.expanduser(src).startswith(MEDIA_ROOT):
                exp = os.path.expanduser(src)
                cl["source"] = str(GATE_MEDIA / exp[len(MEDIA_ROOT):].lstrip("/"))
    return raw


def skip_when_a_plan_clip_is_missing(raw):
    """Skip naming the first clip file of the plan that the host is missing."""
    missing = next((cl["source"] for sc in raw["scenes"] for cl in sc["clips"]
                    if isinstance(cl.get("source"), str)
                    and not Path(cl["source"]).expanduser().is_file()),
                   None)
    if missing is not None:
        pytest.skip(f"the gate host's media is missing {missing}")


def stub_clip(path, seconds):
    subprocess.run(["ffmpeg", "-y", "-v", "error", "-f", "lavfi",
                    "-i", f"testsrc2=size=320x180:rate=30:duration={seconds}",
                    "-c:v", "libx264", "-pix_fmt", "yuv420p", str(path)],
                   check=True, capture_output=True)


def stub_narration(tmp_path, narr):
    d = tmp_path / "tts-out"
    d.mkdir()
    scenes = []
    for sid, seconds in narr:
        wav = d / f"{sid}.wav"
        with wave.open(str(wav), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(24000)
            w.writeframes(b"\x00\x00" * int(seconds * 24000))
        scenes.append({"id": sid, "file": wav.name, "seconds": seconds})
    (d / "scenes.json").write_text(json.dumps({"scenes": scenes}))
    return d


def script_ids(md):
    return [ln[3:].strip() for ln in md.read_text(encoding="utf-8").splitlines()
            if ln.startswith("## ")]


# ---------------- the real plan ----------------

def test_the_product_video_plan_parses_and_matches_the_script():
    raw = gated_plan_raw()
    skip_when_a_plan_clip_is_missing(raw)
    plan = compose.parse_plan(raw)
    assert [sc["id"] for sc in plan["scenes"]] == \
        script_ids(ROOT / "docs/product/scripts/product-video.md")
    items = compose.load_narration(GATE_TTS / "product-video", [sc["id"] for sc in plan["scenes"]])
    assert len(items) == 7
    # every clip ends inside its file
    compose.check_clip_ends(plan)


# ---------------- the parser ----------------

def test_the_parser_refuses_a_missing_clip_file():
    with pytest.raises(compose.ComposeError, match="missing"):
        compose.parse_plan({"scenes": [{"id": "s", "clips": [
            {"source": "/nonexistent/clip.mp4", "start": 0.0, "end": 1.0}]}]})


def test_the_parser_refuses_a_speed_over_the_cap():
    for speed in (0.0, -1.0, 2.1, "2"):
        with pytest.raises(compose.ComposeError, match="speed"):
            compose.parse_plan({"scenes": [{"id": "s", "clips": [
                {"source": __file__, "start": 0.0, "end": 1.0, "speed": speed}]}]})


def test_the_parser_refuses_a_clip_outside_its_file():
    with pytest.raises(compose.ComposeError, match="0 <= start < end"):
        compose.parse_plan({"scenes": [{"id": "s", "clips": [
            {"source": __file__, "start": 2.0, "end": 1.0}]}]})


def test_the_parser_refuses_broken_scenes():
    for plan in ({"scenes": []}, {}, {"scenes": [{"id": "s"}]},
                 {"scenes": [{"id": "s", "clips": [{}]}]},
                 {"scenes": [{"id": "s", "clips": [{"title": [], "seconds": 1.0}]}]},
                 {"scenes": [{"id": "s", "clips": [{"title": ["T"]}]}]},
                 {"scenes": [{"id": "s", "clips": [{"title": ["T"], "seconds": 1.0, "accent": "x"}]}]}):
        with pytest.raises(compose.ComposeError):
            compose.parse_plan(plan)


def test_load_narration_refuses_a_scene_without_narration(tmp_path):
    tts = stub_narration(tmp_path, [("s1", 1.0)])
    with pytest.raises(compose.ComposeError, match="no narration for scene s2"):
        compose.load_narration(tts, ["s1", "s2"])


# ---------------- the scene timing ----------------

def test_a_short_clip_list_holds_its_last_frame():
    clips = [{"source": "a.mp4", "start": 6.2, "end": 25.49, "speed": 2.0}]
    sp = compose.scene_plan(clips, 10.011)
    assert sp["frames"] == 300
    assert sp["video_s"] == 10.0
    assert sp["raw_s"] == pytest.approx(9.645)
    assert sp["hold"] == pytest.approx(0.355, abs=1e-9)
    assert sp["cut"] == 0.0
    assert sp["badges"] == [(0.0, pytest.approx(9.645))]


def test_a_long_clip_list_is_cut_at_the_scene_end():
    clips = [{"source": "a.mp4", "start": 45.84, "end": 53.531}]
    sp = compose.scene_plan(clips, 7.6)   # video_s = 7.6: the 7.691 s clip is cut by 0.091
    assert sp["video_s"] == pytest.approx(7.6)
    assert sp["cut"] == pytest.approx(0.091)
    assert sp["hold"] == 0.0


def test_badge_windows_cover_the_sped_up_clips_in_order():
    clips = [{"source": "a.mp4", "start": 0.0, "end": 4.0, "speed": 2.0},
             {"title": ["T"], "seconds": 1.0},
             {"source": "b.mp4", "start": 0.0, "end": 3.0, "speed": 1.5}]
    sp = compose.scene_plan(clips, 7.0)
    assert sp["badges"] == [(0.0, pytest.approx(2.0)), (3.0, pytest.approx(5.0))]


def test_the_drift_stays_on_the_frame_grid():
    for narr in (6.851, 7.731, 10.011, 8.491, 6.611, 7.691, 4.691, 0.5, 3.0):
        sp = compose.scene_plan([{"title": ["T"], "seconds": narr}], narr)
        assert abs(sp["video_s"] - narr) <= 1.0 / (2 * compose.FPS) + 1e-9


# ---------------- a composed stub video ----------------

def test_a_stub_plan_composes_end_to_end(tmp_path):
    if shutil.which("ffmpeg") is None:
        pytest.skip("no ffmpeg on PATH (the e2e compose renders real files)")
    (tmp_path / "clips").mkdir()
    stub_clip(tmp_path / "clips" / "a.mp4", 2.0)
    stub_clip(tmp_path / "clips" / "b.mp4", 3.0)
    tts = stub_narration(tmp_path, [("s1", 1.5), ("s2", 1.2), ("s3", 1.0)])
    plan = {"name": "stub", "out": str(tmp_path / "out.mp4"),
            "scenes": [
                {"id": "s1", "clips": [{"source": str(tmp_path / "clips" / "a.mp4"),
                                        "start": 0.0, "end": 2.0}]},
                {"id": "s2", "clips": [{"source": str(tmp_path / "clips" / "b.mp4"),
                                        "start": 1.0, "end": 2.2}]},
                {"id": "s3", "clips": [{"title": ["Drum Spike"], "seconds": 0.8},
                                       {"source": str(tmp_path / "clips" / "a.mp4"),
                                        "start": 1.0, "end": 1.9}]}]}
    out = tmp_path / "out.mp4"
    scenes_plan, items, total = compose.compose(plan, tts, out, tmp_path / "work")
    assert out.is_file()
    assert total == pytest.approx((45 + 36 + 30) / 30)
    assert [sid for sid, _, _ in items] == ["s1", "s2", "s3"]
    for sp, narr in zip(scenes_plan, (1.5, 1.2, 1.0)):
        assert abs(sp["video_s"] - narr) <= 1.0 / (2 * compose.FPS) + 1e-9
    real = compose.ffprobe_seconds(out)
    assert abs(real - total) <= compose.DRIFT_LIMIT_S
    # the narration wav of the run is gone with the work dir, the mp4 carries aac
    assert not (tmp_path / "work").exists()


def test_the_stub_output_is_1080p_h264_aac(tmp_path):
    if shutil.which("ffmpeg") is None:
        pytest.skip("no ffmpeg on PATH (the e2e compose renders real files)")
    (tmp_path / "clips").mkdir()
    stub_clip(tmp_path / "clips" / "a.mp4", 1.0)
    tts = stub_narration(tmp_path, [("s1", 0.8)])
    plan = {"name": "stub", "out": str(tmp_path / "out.mp4"),
            "scenes": [{"id": "s1", "clips": [
                {"source": str(tmp_path / "clips" / "a.mp4"), "start": 0.0, "end": 1.0}]}]}
    out = tmp_path / "out.mp4"
    compose.compose(plan, tts, out, tmp_path / "work")
    r = subprocess.run(["ffprobe", "-v", "error", "-show_entries",
                        "stream=codec_name,width,height,channels", "-of", "json", str(out)],
                       check=True, capture_output=True, text=True)
    streams = json.loads(r.stdout)["streams"]
    assert streams[0]["codec_name"] == "h264" and streams[0]["width"] == 1920 \
        and streams[0]["height"] == 1080
    assert streams[1]["codec_name"] == "aac"


# ---------------- the disclosure note on every frame and the end card's url (review 6403564803) ----------------

def test_the_parser_refuses_a_non_string_url():
    with pytest.raises(compose.ComposeError, match="url"):
        compose.parse_plan({"scenes": [{"id": "s", "clips": [
            {"title": ["T"], "seconds": 1.0, "url": 3}]}]})


def test_the_end_card_carries_the_licence_and_an_empty_url():
    raw = gated_plan_raw()
    skip_when_a_plan_clip_is_missing(raw)
    end = next(cl for sc in raw["scenes"] if sc["id"] == "pv-end" for cl in sc["clips"])
    assert end["url"] == ""   # the repo is private for now: no URL line
    assert "Drum Spike is free and open source (Apache-2.0)." in end["title"]


def test_the_url_line_is_omitted_while_empty_and_accented_when_set():
    lines, accent = compose.card_lines({"title": ["Drum Spike."], "url": ""})
    assert lines == ["Drum Spike."] and accent is None
    lines, accent = compose.card_lines({"title": ["Drum Spike.", "Licence."], "url": "github.com/x/y"})
    assert lines == ["Drum Spike.", "Licence.", "github.com/x/y"] and accent == 2


def test_the_note_does_not_double_on_source_clips_and_sits_on_the_cards():
    # the coordinator's verify finding: the explainers carry their own note, the recordings
    # record.py's identical one — a second overlay would double them. The graph overlays the
    # shared PNG on no source clip and on every title-card segment.
    plan = {"scenes": [{"id": "s", "clips": [
        {"title": ["T"], "seconds": 1.0},
        {"source": "a.mp4", "start": 0.0, "end": 1.0}]}]}
    sp = compose.scene_plan(plan["scenes"][0]["clips"], 1.0)
    parts, inputs, last = compose.build_graph(plan, [sp], {}, {(0, 0): "card.png"}, "note.png")
    note_idx = next(i for i, e in enumerate(inputs) if e["path"] == "note.png")
    assert last == "[vout]"
    assert f"[{note_idx}:v]" not in parts[-1]   # no second note after the global concat
    card_idx = next(i for i, e in enumerate(inputs) if e["path"] == "card.png")
    card = next(p for p in parts if p.startswith(f"[{card_idx}:v]"))
    assert card == (f"[{card_idx}:v][{note_idx}:v]overlay=x=main_w-overlay_w-{record.NOTE_MARGIN_PX}"
                    f":y=main_h-overlay_h-{record.NOTE_BOTTOM_PX},fps=30,format=yuv420p[t0_0]")
    src_idx = len(inputs) - 1   # the source clip was added last
    src = next(p for p in parts if p.startswith(f"[{src_idx}:v]trim="))
    assert f"[{note_idx}:v]" not in src


def test_composes_note_png_is_records_shared_render(tmp_path):
    # the shared overlay helper both use: compose's note comes from record.py's renderer, so a
    # recording's burned-in note and the compose's overlay are the same pixels at the same place
    try:
        import playwright.sync_api  # noqa: F401
    except ImportError:
        pytest.skip("no playwright here (the note PNG renders in Chromium)")
    png = compose.render_note(tmp_path)
    assert png.is_file() and png.name == "note.png" and png.stat().st_size > 0
