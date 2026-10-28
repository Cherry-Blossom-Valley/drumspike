"""#246: the timing gate of the explainers. Narration per scene (tts.py's scenes.json) against the picture per scene.

No Manim import: the gate and its tests run without the render.
"""
import json
import sys
from pathlib import Path

DRIFT_LIMIT_S = 0.100  # the gate: a scene whose picture and narration differ by more is a FAIL
AUDIO_LENGTH_LIMIT_S = 0.050  # the muxed MP4: its audio and its picture differ by at most this

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import tts  # noqa: E402  (tools/media/tts.py: the script parser, one `##` heading per scene)


def script_scene_ids(script):
    return [sid for sid, _ in tts.parse_scenes(Path(script).read_text(encoding="utf-8"))]


def narration_seconds(scenes_json):
    """{scene id: seconds} from the scenes.json that tts.py writes next to the scene WAVs."""
    data = json.loads(Path(scenes_json).read_text(encoding="utf-8"))
    return {row["id"]: float(row["seconds"]) for row in data["scenes"]}


def scene_files(scenes_json):
    """{scene id: path of the scene's WAV}, the files that scenes.json names next to it."""
    base = Path(scenes_json).parent
    data = json.loads(Path(scenes_json).read_text(encoding="utf-8"))
    return {row["id"]: base / row["file"] for row in data["scenes"]}


def timing_errors(script_ids, narration, picture):
    """The gate's FAIL lines (empty when it passes). picture: {scene id: seconds of the picture}."""
    errors = []
    for sid in script_ids:
        if sid not in narration:
            errors.append(f"{sid}: no narration (run tts.py on the script)")
        if sid not in picture:
            errors.append(f"{sid}: no picture (the video has no scene for it)")
    for sid in picture:
        if sid not in script_ids:
            errors.append(f"{sid}: picture scene is not a scene of the script")
    for sid in script_ids:
        if sid in narration and sid in picture:
            drift = picture[sid] - narration[sid]
            if abs(drift) > DRIFT_LIMIT_S:
                errors.append(f"{sid}: drift {drift * 1000:+.0f} ms (limit {DRIFT_LIMIT_S * 1000:.0f} ms)")
    return errors


def track_errors(placed, kinds, durations):
    """The FAIL lines of the muxed MP4 (empty when it passes).

    placed: {scene id: (picture start, audio start in the track)} in seconds;
    kinds: the codec type of each stream of the MP4; durations: {codec type: seconds}.
    """
    errors = []
    if sorted(kinds) != ["audio", "video"]:
        errors.append(f"output streams are {sorted(kinds)}: want one video and one audio")
    elif set(durations) != {"audio", "video"}:
        errors.append("output has no duration for its audio or its video stream")
    else:
        diff = durations["audio"] - durations["video"]
        if abs(diff) > AUDIO_LENGTH_LIMIT_S:
            errors.append(f"audio {durations['audio']:.3f} s and video {durations['video']:.3f} s differ by {diff * 1000:+.0f} ms")
    for sid, (planned, at) in placed.items():
        drift = at - planned
        if abs(drift) > DRIFT_LIMIT_S:
            errors.append(f"{sid}: audio drift {drift * 1000:+.0f} ms in the track (limit {DRIFT_LIMIT_S * 1000:.0f} ms)")
    return errors
