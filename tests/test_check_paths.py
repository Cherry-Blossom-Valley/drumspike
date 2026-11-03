"""The OCR private-path guard (tools/media/check_paths.py, #458) and its wiring into the
render tools record.py, shots.py and explainers/build.py (#459).

Run the OCR tests with the media extra:
    uv run --extra media pytest tests/test_check_paths.py -q
"""
import subprocess
import sys
from pathlib import Path

import pytest

pytestmark = pytest.mark.media

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools" / "media"))
import record  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools" / "media" / "explainers"))


# ---------------- #458: the pre-done guards ----------------

import check_paths  # noqa: E402

# The planted home-folder token, built from parts so this file passes tools/check_private.py
# (the same split as MODEL_TERMS in test_docs_wording.py).
USERS = "/" + "Users"


def _fake_engine(text):
    """An OCR engine stub in the RapidOCR result shape: [[box, line], ...]."""
    return lambda image: ([[0, text]], None)


def _font_file():
    import glob
    for pattern in ("/usr/share/fonts/**/DejaVuSans.ttf", "/usr/share/fonts/**/*.ttf"):
        found = glob.glob(pattern, recursive=True)
        if found:
            return found[0]
    return None


def _png_with_text(tmp_path, text, name):
    """A PNG with the text rendered into it (ffmpeg drawtext); None when no font is here."""
    font = _font_file()
    if font is None:
        return None
    out = tmp_path / name
    done = subprocess.run(
        ["ffmpeg", "-y", "-v", "error", "-f", "lavfi", "-i", "color=white:s=640x160",
         "-vf", f"drawtext=fontfile={font}:text={text}:fontcolor=black:fontsize=48:x=20:y=40",
         "-frames:v", "1", str(out)], capture_output=True, text=True)
    if done.returncode != 0:
        return None
    return out


def test_a_planted_path_png_fails_the_guard(tmp_path):
    pytest.importorskip("rapidocr_onnxruntime", reason="needs the media extra (the OCR engine)")
    png = _png_with_text(tmp_path, f"{USERS}/x", "planted.png")
    if png is None:
        pytest.skip("no ttf font for the ffmpeg drawtext plant")
    findings = check_paths.check_file(png, check_paths._engine())
    assert findings, f"the planted {USERS}/x was not read back"
    assert findings[0][2] == USERS


def test_a_clean_png_passes_the_guard(tmp_path):
    pytest.importorskip("rapidocr_onnxruntime", reason="needs the media extra (the OCR engine)")
    png = _png_with_text(tmp_path, "Drum Spike", "clean.png")
    if png is None:
        pytest.skip("no ttf font for the ffmpeg drawtext plant")
    assert check_paths.check_file(png, check_paths._engine()) == []


def test_the_guard_reads_the_engine_text(tmp_path):
    """The guard logic against an engine stub: the banned token and the read text come back."""
    findings = check_paths.check_file(tmp_path / "shot.png", _fake_engine(f"created in {USERS}/x"))
    assert findings and findings[0][2] == USERS and findings[0][3] == f"created in {USERS}/x"
    assert check_paths.check_file(tmp_path / "shot.png", _fake_engine("Drum Spike")) == []


def test_the_guard_fails_loudly_without_the_engine(tmp_path, monkeypatch):
    def no_engine():
        raise check_paths.PathCheckError("the OCR engine rapidocr-onnxruntime is not installed")
    monkeypatch.setattr(check_paths, "_engine", no_engine)
    with pytest.raises(check_paths.PathCheckError, match="rapidocr"):
        check_paths.check_files([tmp_path / "x.png"])


# ---------------- #459: the path guard is the last step of the render tools ----------------

import shots  # noqa: E402
import build  # noqa: E402


def _planted_media(tmp_path, name):
    """A real 1 s media file (any pixels; the engine below is a stub): None without ffmpeg."""
    out = tmp_path / name
    done = subprocess.run(
        ["ffmpeg", "-y", "-v", "error", "-f", "lavfi", "-i", "color=white:s=320x240:d=1",
         "-c:v", "libx264", "-pix_fmt", "yuv420p", "-frames:v", "30", str(out)],
        capture_output=True, text=True)
    if done.returncode != 0:
        return None
    return out


def _planted_png(tmp_path, name):
    out = tmp_path / name
    done = subprocess.run(
        ["ffmpeg", "-y", "-v", "error", "-f", "lavfi", "-i", "color=white:s=320x240",
         "-frames:v", "1", str(out)], capture_output=True, text=True)
    if done.returncode != 0:
        return None
    return out


def _plant_engine(monkeypatch, tool):
    """The tool's loaded check_paths instance reads the stub engine: a planted home-folder line."""
    monkeypatch.setattr(tool.path_guard(), "_engine",
                        lambda: _fake_engine(f"created in {USERS}/x/y"))


def test_record_py_fails_on_a_planted_frame(tmp_path, monkeypatch, capsys):
    """The recorder's wired-in guard exits non-zero and names the frame (a real MP4, stub OCR)."""
    mp4 = _planted_media(tmp_path, "rec.mp4")
    if mp4 is None:
        pytest.skip("no ffmpeg to plant a frame")
    _plant_engine(monkeypatch, record)
    with pytest.raises(SystemExit) as exc:
        record.run_path_guard([mp4])
    assert exc.value.code == 1
    err = capsys.readouterr().err
    assert "frame 1" in err and USERS in err


def test_shots_py_fails_on_a_planted_picture(tmp_path, monkeypatch, capsys):
    """The shot tool's wired-in guard exits non-zero on a planted picture (a real PNG, stub OCR)."""
    png = _planted_png(tmp_path, "shot.png")
    if png is None:
        pytest.skip("no ffmpeg to plant a picture")
    _plant_engine(monkeypatch, shots)
    with pytest.raises(SystemExit) as exc:
        shots.run_path_guard([png])
    assert exc.value.code == 1
    err = capsys.readouterr().err
    assert USERS in err and str(png) in err


def test_build_py_fails_on_a_planted_frame(tmp_path, monkeypatch, capsys):
    """The explainer builder's wired-in guard exits non-zero and names the frame (stub OCR)."""
    mp4 = _planted_media(tmp_path, "explainer.mp4")
    if mp4 is None:
        pytest.skip("no ffmpeg to plant a frame")
    _plant_engine(monkeypatch, build)
    with pytest.raises(SystemExit) as exc:
        build.run_path_guard([mp4])
    assert exc.value.code == 1
    err = capsys.readouterr().err
    assert "frame 1" in err and USERS in err


def test_a_clean_render_passes_the_wired_guard(tmp_path, monkeypatch):
    """The same wiring passes a clean render (stub OCR reads nothing banned)."""
    mp4 = _planted_media(tmp_path, "clean.mp4")
    if mp4 is None:
        pytest.skip("no ffmpeg to plant a frame")
    monkeypatch.setattr(record.path_guard(), "_engine", lambda: _fake_engine("Drum Spike"))
    record.run_path_guard([mp4])   # no SystemExit
