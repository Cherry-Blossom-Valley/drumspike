"""#469: the repaint-glitch repair of record.py. The test plants one isolated glitch frame
and one real scene change in a video and shows that the repair replaces only the glitch
frame (the scene change stays). Runs with the media extra:

    uv run --extra media pytest tests/test_record_glitch.py -q
"""
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

pytestmark = pytest.mark.media

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools" / "media"))
import record  # noqa: E402

W, H, N = 160, 90, 30
GLITCH_AT = 10        # the isolated mid-repaint frame: luma jumps and comes back
SCENE_AT = 20         # the real scene change: luma jumps and stays
DIM, GLITCH, SCENE = 20, 220, 120


def _write_video(path):
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        pytest.skip("no ffmpeg on PATH")
    # N flat gray frames; frame GLITCH_AT is bright (the planted repaint glitch),
    # from SCENE_AT on the luma stays at the scene level (the real scene change).
    frames = bytearray()
    for i in range(N):
        luma = GLITCH if i == GLITCH_AT else (SCENE if i >= SCENE_AT else DIM)
        frames += bytes([luma]) * (W * H)
    cmd = [ffmpeg, "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "gray",
           "-s", f"{W}x{H}", "-r", "25", "-i", "-",
           "-c:v", "libx264", "-qp", "0", "-pix_fmt", "yuv420p", str(path)]
    subprocess.run(cmd, input=bytes(frames), check=True, capture_output=True)


def test_patch_replaces_only_the_isolated_glitch_frame(tmp_path):
    video = tmp_path / "in.mp4"
    _write_video(video)

    runs = record.glitch_runs(video, tmp_path)
    # The planted glitch is found, and nothing else: the scene change is not an isolated run.
    assert runs == [(GLITCH_AT, GLITCH_AT)]

    out, replaced = record.patch_glitches(video, tmp_path)
    assert out == video and replaced == [(GLITCH_AT, GLITCH_AT)]

    means = record.frame_means(video, tmp_path)
    # The glitch frame now matches its neighbours (the frame before it), so no run remains.
    assert record.glitch_runs(video, tmp_path) == []
    assert abs(means[GLITCH_AT] - means[GLITCH_AT - 1]) <= record.GLITCH_MEAN_DIFF
    # The real scene change is untouched: the luma still jumps at SCENE_AT and stays.
    assert means[SCENE_AT - 1] < DIM + 10 and means[SCENE_AT] > SCENE - 10
    assert means[-1] > SCENE - 10
    # GLITCH_MAX_RUN is the longest run the repair may replace (the review's stated bound).
    assert record.GLITCH_MAX_RUN == 4
