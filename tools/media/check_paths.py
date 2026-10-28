"""#458: the path guard: OCR every given PNG and every frame (1 fps) of every given MP4 and
FAIL on a private path or host name in the rendered text.

    uv run --extra media python tools/media/check_paths.py <png|mp4>...

A standalone tool (the wiring into the render tools is #459): a render that shows `/Users`,
`/var/folders`, `/home`, `/private/tmp`, `admin@`, a host alias (`x86-mac`, `arm64-mac`, `linux-arm64`) or a
word of the environment variable DRUMSPIKE_PRIVATE_WORDS (comma separated: your login, host names) exits non-zero and names the file, the frame and the text. The OCR engine is
rapidocr-onnxruntime (a pip package, the `media` extra): a missing engine is a loud FAIL
(exit 2), never a silent pass. The mp4 frames come from ffmpeg; a missing ffmpeg is the
same loud FAIL.
"""
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

BANNED = ("/Users", "/var/folders", "/home", "/private/tmp", "x86-mac",
          "admin@", "arm64-mac", "linux-arm64") + tuple(
    w.strip() for w in __import__("os").environ.get("DRUMSPIKE_PRIVATE_WORDS", "").split(",") if w.strip())
FRAMES_FPS = 1   # the mp4 frames OCR'd per second


class PathCheckError(Exception):
    """The check could not run (the OCR engine or ffmpeg missing, a decode failed)."""


def _engine():
    """The RapidOCR instance, or a loud PathCheckError when the engine is not installed."""
    try:
        from rapidocr_onnxruntime import RapidOCR
    except ImportError:
        raise PathCheckError(
            "the OCR engine rapidocr-onnxruntime is not installed: the path guard cannot run "
            "(uv run --extra media ..., or add the media extra)") from None
    return RapidOCR()


def ocr_text(engine, image):
    """The text the engine reads from one image; '' when it reads nothing."""
    result, _elapsed = engine(str(image))
    if result is None:
        return ""
    return "\n".join(line[1] for line in result)


def mp4_frames(mp4, out_dir):
    """The 1 fps frame PNGs of an MP4, written into out_dir; raises when ffmpeg fails."""
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise PathCheckError("ffmpeg is not on PATH: the path guard cannot read the mp4 frames")
    done = subprocess.run([ffmpeg, "-y", "-v", "error", "-i", str(mp4), "-vf", f"fps={FRAMES_FPS}",
                           str(out_dir / "frame-%05d.png")], capture_output=True, text=True)
    if done.returncode != 0:
        raise PathCheckError(f"ffmpeg failed on {mp4}: {done.stderr.strip()[:200]}")
    return sorted(out_dir.glob("frame-*.png"))


def banned_in(text):
    """The banned tokens the text carries, compared case-insensitively."""
    lowered = text.lower()
    return [w for w in BANNED if w.lower() in lowered]


def check_file(path, engine):
    """The findings of one PNG or MP4: [(file, frame, word, text)]; frame is None for a PNG."""
    path = Path(path)
    findings = []
    if path.suffix.lower() == ".mp4":
        with tempfile.TemporaryDirectory(prefix="path-check-") as tmp:
            frames = mp4_frames(path, Path(tmp))
            for i, frame in enumerate(frames, start=1):
                text = ocr_text(engine, frame)
                for word in banned_in(text):
                    findings.append((str(path), f"frame {i}", word, text))
    else:
        text = ocr_text(engine, path)
        for word in banned_in(text):
            findings.append((str(path), None, word, text))
    return findings


def check_files(paths, engine=None):
    """The findings of every file, in order; raises PathCheckError when the OCR cannot run."""
    engine = engine or _engine()
    out = []
    for p in paths:
        out.extend(check_file(p, engine))
    return out


def guard(paths, engine=None):
    """The wired-in guard (#459): the finished render(s) must show no private path or host name.
    On a finding, print the FAIL lines (the file, the frame, the word, the text) and exit
    non-zero; a clean render prints one clean line per file. Called as the last step of
    record.py, shots.py and build.py."""
    try:
        findings = check_files([Path(p) for p in paths], engine=engine)
    except PathCheckError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        raise SystemExit(2) from None
    for path, frame, word, text in findings:
        where = f" {frame}" if frame else ""
        snippet = " ".join(text.split())[:120]
        print(f"FAIL: {path}{where}: shows {word!r}: {snippet}", file=sys.stderr)
    if findings:
        raise SystemExit(1)
    for p in paths:
        print(f"path guard: clean: {p}")


def main(argv=None):
    paths = [Path(a) for a in (argv if argv is not None else sys.argv[1:])]
    if not paths:
        print("FAIL: no files given: check_paths.py <png|mp4>...", file=sys.stderr)
        return 2
    try:
        findings = check_files(paths)
    except PathCheckError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        return 2
    for path, frame, word, text in findings:
        where = f" {frame}" if frame else ""
        snippet = " ".join(text.split())[:120]
        print(f"FAIL: {path}{where}: shows {word!r}: {snippet}", file=sys.stderr)
    if findings:
        return 1
    for p in paths:
        print(f"path guard: clean: {p}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
