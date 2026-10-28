"""#246: render the 3Blue1Brown-style explainers (Manim CE) and gate each scene's picture against its narration.

    uv run --project . python tools/media/explainers/build.py 01 04

Needs the narration first: tools/media/tts.py writes the scenes.json that this reads (default
~/.cache/drumspike/media/tts-out/<script stem>/). Writes ~/.cache/drumspike/media/explainers/
explainer-<id>.mp4. Prints per video the duration, the scene count and each scene's drift.
FAIL (exit 1) on a scene without narration, a picture scene that is not in the script, or a
drift over 100 ms. The drift is the planned timeline of the scene, not a measurement of the MP4.
The narration goes on the picture: the scene WAVs are laid on one track in script order, each at
its scene's planned start, and muxed as the MP4's one audio stream. The MP4 must have one video and
one audio stream, their lengths within 50 ms, and each scene's audio within 100 ms of its picture.
It also writes one PNG per scene, taken at the scene's middle, under
~/.cache/drumspike/media/explainers/frames/<script stem>/ for the check of one caption per frame.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import timing  # noqa: E402

ROOT = HERE.parent.parent.parent
SCRIPTS = ROOT / "docs" / "product" / "scripts"
TTS_OUT = Path.home() / ".cache" / "drumspike" / "media" / "tts-out"
OUT = Path.home() / ".cache" / "drumspike" / "media" / "explainers"
FRAMES = OUT / "frames"

# video id -> (script stem, module file, Scene class)
VIDEOS = {
    "01": ("explainer-01-why-one-ms", "explainer_01.py", "Explainer01"),
    "03": ("explainer-03-detection", "explainer_03.py", "Explainer03"),
    "04": ("explainer-04-first-sample", "explainer_04.py", "Explainer04"),
    "08": ("explainer-08-velocity", "explainer_08.py", "Explainer08"),
    "05": ("explainer-05-audio-spike", "explainer_05.py", "Explainer05"),
    "10": ("explainer-10-bleed", "explainer_10.py", "Explainer10"),
}


def render(module, scene, narration, plan, media_dir, quality):
    """Run Manim on one scene class; returns the mp4 path or raises SystemExit with the reason."""
    env = dict(os.environ, DRUMSPIKE_EXPLAINER_NARRATION=str(narration), DRUMSPIKE_EXPLAINER_PLAN=str(plan),
               PYTHONPATH=str(HERE))
    cmd = ["uv", "run", "--with", "manim", "manim", "render", f"-q{quality}", "--fps", "60",
           "--media_dir", str(media_dir), str(HERE / module), scene]
    done = subprocess.run(cmd, cwd=HERE, env=env)
    if done.returncode != 0:
        raise SystemExit(f"FAIL: manim exited {done.returncode} for {module} (see its output above)")
    found = sorted(media_dir.rglob(f"{scene}.mp4"), key=lambda p: p.stat().st_mtime)
    if not found:
        raise SystemExit(f"FAIL: manim wrote no {scene}.mp4 under {media_dir}")
    return found[-1]


def scene_frames(mp4, picture, script_ids, out_dir):
    """One PNG per scene, from the middle of its planned span in the MP4 (ffmpeg); returns the paths."""
    out_dir.mkdir(parents=True, exist_ok=True)
    paths = []
    start = 0.0
    for sid in script_ids:
        if sid not in picture:
            continue
        at = start + picture[sid] / 2
        png = out_dir / f"{sid}.png"
        done = subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-ss", f"{at:.2f}", "-i", str(mp4),
                               "-frames:v", "1", str(png)])
        if done.returncode != 0:
            raise SystemExit(f"FAIL: ffmpeg wrote no frame for {sid} (exit {done.returncode})")
        paths.append(png)
        start += picture[sid]
    return paths


def lay_track(wavs, picture, script_ids, out_wav):
    """Lay the scene WAVs on one track, in script order: each at its scene's planned start in the picture,
    silence where the picture is longer. A WAV longer than its picture slot pushes the scenes after it later,
    and the track gate reports that as drift. Returns {scene id: (planned start, audio start)} in seconds."""
    pcm = bytearray()
    fmt = None
    placed = {}
    planned = 0.0
    for sid in script_ids:
        if sid not in picture:
            continue
        with wave.open(str(wavs[sid]), "rb") as w:
            this = (w.getnchannels(), w.getsampwidth(), w.getframerate())
            data = w.readframes(w.getnframes())
        if fmt is None:
            fmt = this
        if this != fmt:
            raise SystemExit(f"FAIL: {wavs[sid]} is {this} (channels, bytes, rate), the track is {fmt}")
        channels, width, rate = fmt
        frame = channels * width
        target = round(planned * rate) * frame
        if len(pcm) < target:
            pcm += bytes(target - len(pcm))
        placed[sid] = (planned, len(pcm) / frame / rate)
        pcm += data
        planned += picture[sid]
    if fmt is None:
        raise SystemExit("FAIL: no scene of the script has a picture, so there is no track")
    channels, width, rate = fmt
    total = round(planned * rate) * channels * width
    if len(pcm) < total:
        pcm += bytes(total - len(pcm))
    with wave.open(str(out_wav), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(width)
        w.setframerate(rate)
        w.writeframes(bytes(pcm))
    return placed


def mux(mp4, track, out, length):
    """The picture with the track as its one audio stream. Manim's frame count runs a few frames short of the
    planned length, so the picture is held (its last frame cloned) or cut to `length`, the track's length.
    No -shortest: the audio must not cut the picture."""
    video = probe(mp4)[1].get("video", 0.0)
    pad = max(0.0, length - video)
    vf = f"tpad=stop_mode=clone:stop_duration={pad:.4f},trim=duration={length:.4f},setpts=PTS-STARTPTS"
    done = subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", str(mp4), "-i", str(track),
                           "-map", "0:v:0", "-map", "1:a:0", "-vf", vf, "-c:v", "libx264", "-pix_fmt", "yuv420p",
                           "-c:a", "aac", "-b:a", "192k", str(out)])
    if done.returncode != 0:
        raise SystemExit(f"FAIL: ffmpeg could not mux the audio into {mp4} (exit {done.returncode})")


def probe(mp4):
    """(stream codec types, {codec type: seconds}) of an MP4, from ffprobe."""
    done = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "stream=codec_type,duration",
                           "-of", "json", str(mp4)], capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit(f"FAIL: ffprobe could not read {mp4} (exit {done.returncode})")
    streams = json.loads(done.stdout)["streams"]
    kinds = [s["codec_type"] for s in streams]
    durations = {s["codec_type"]: float(s["duration"]) for s in streams if "duration" in s}
    return kinds, durations


def build(video_id, work, tts_out, quality):
    stem, module, scene = VIDEOS[video_id]
    script = SCRIPTS / f"{stem}.md"
    scenes_json = tts_out / stem / "scenes.json"
    if not scenes_json.exists():
        print(f"FAIL: no narration for {stem}: run tts.py on {script} first", file=sys.stderr)
        return 1

    script_ids = timing.script_scene_ids(script)
    narration = timing.narration_seconds(scenes_json)
    plan = work / f"{stem}-plan.json"
    mp4 = render(module, scene, scenes_json, plan, work / stem, quality)
    picture = {sid: float(sec) for sid, sec in json.loads(plan.read_text(encoding="utf-8"))["scenes"].items()}

    print(f"{stem}: {sum(picture.values()):.2f} s, {len(picture)} scenes")
    for sid in script_ids:
        if sid in picture and sid in narration:
            drift = picture[sid] - narration[sid]
            print(f"  {sid}: narration {narration[sid]:.2f} s, picture {picture[sid]:.2f} s, drift {drift * 1000:+.0f} ms")
    frames = scene_frames(mp4, picture, script_ids, FRAMES / stem)
    print(f"  frames: {len(frames)} PNG in {FRAMES / stem}")
    for png in frames:
        print(f"    {png}")
    errors = timing.timing_errors(script_ids, narration, picture)
    for line in errors:
        print(f"FAIL: {line}", file=sys.stderr)
    if errors:
        return 1

    track = work / f"{stem}-track.wav"
    placed = lay_track(timing.scene_files(scenes_json), picture, script_ids, track)
    muxed = work / f"{stem}-muxed.mp4"
    mux(mp4, track, muxed, sum(picture.values()))
    kinds, durations = probe(muxed)
    print(f"  streams: {sorted(kinds)}, durations: " + ", ".join(f"{k} {v:.3f} s" for k, v in sorted(durations.items())))
    for sid, (planned, at) in placed.items():
        print(f"  {sid}: audio at {at:.2f} s, picture at {planned:.2f} s, drift {(at - planned) * 1000:+.0f} ms")
    errors = timing.track_errors(placed, kinds, durations)
    for line in errors:
        print(f"FAIL: {line}", file=sys.stderr)
    if errors:
        return 1
    OUT.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(muxed, OUT / f"explainer-{video_id}.mp4")
    print(f"wrote {OUT / f'explainer-{video_id}.mp4'}")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("videos", nargs="+", choices=sorted(VIDEOS))
    ap.add_argument("--tts-out", type=Path, default=TTS_OUT, help="dir of the narration per script (default %(default)s)")
    ap.add_argument("--quality", default="h", choices=["l", "m", "h"],
                    help="manim quality flag: h = 1080p (the release), l = 480p scratch check (default %(default)s)")
    args = ap.parse_args(argv)
    status = 0
    for video_id in args.videos:
        with tempfile.TemporaryDirectory(prefix="explainer-") as tmp:
            status |= build(video_id, Path(tmp), args.tts_out, args.quality)
    return status


if __name__ == "__main__":
    sys.exit(main())
