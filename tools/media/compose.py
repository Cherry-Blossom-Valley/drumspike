"""#341: the product video compositor: a clip list in, the narrated MP4 out.

    uv run --project . python tools/media/compose.py tools/media/product-video.json \
        [--tts-out DIR] [--out MP4] [--keep]

The plan JSON (tools/media/product-video.json) lists the scenes of the product video
(docs/product/scripts/product-video.md); each scene carries a clip list, so a cut is data, not
code. A source clip is {source, start, end, speed} (speed at most 2x, the "2x" badge of
record.py over the sped-up span); a title card is {title: [lines], seconds, accent: line} — a
1920x1080 card PNG of the media plan's dark look, rendered with Playwright like record.py's
badges (this ffmpeg has no drawtext), and may carry url: the download address as its own
accent line, omitted while the field is empty (the repo is private for now). The "AI-generated" corner note: every source clip already carries one baked in — the
explainers render their own (explainer_common.py's dim DR-corner note) and the recordings
carry record.py's identical PNG at right 28 / bottom 20 — so the compose overlays the shared
PNG only on the title cards, which are rendered here and carry none. The narration comes from tools/media/tts.py's out dir
(--tts-out plus the plan's narration.dir; the TTS API is NEVER called here). Every scene
lands on its narration: a clip list shorter than the scene holds its last frame, a longer
one is cut at the scene end. The
video is the media plan's 1920x1080, h264 + aac; the scene audio sits at the scene starts of
the 30 fps frame grid, so the drift per scene is the frame rounding only (FAIL over 100 ms).
FAIL also on a missing clip file, a clip beyond its file's end, a speed over 2x, or a scene id
without narration. The clip audio is dropped: the narration is the only audio (no music).
Heavy: run under the host lock (crewboard lock run ...).
"""
import argparse
import html
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import wave
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CACHE = Path.home() / ".cache" / "drumspike" / "media"
OUT_W, OUT_H = 1920, 1080     # the media plan: 1920x1080
FPS = 30
DRIFT_LIMIT_S = 0.1
MAX_SPEED = 2.0               # a clip is sped up at most 2x (the ticket)
BADGE_X_S = 28                # record.py's badge corner (top right, clear of the status bar)
BADGE_Y_S = 28
BG = "#14161a"                # the app's background (static/style.css), like the explainers
TEXT = "#f1f4f9"
SUB = "#cdd3de"
ACCENT = "#ff9f43"            # the app's hit orange, for the accent line (the download address)
TITLE_PX = (120, 96)          # lone title, first line of a card
SUB_PX = 48
LINE_GAP_PX = 40
FONT_STACK = "Helvetica, Arial, sans-serif"   # one sans-serif family, like the explainers


class ComposeError(Exception):
    pass


def expand(path):
    return Path(os.path.expanduser(str(path)))


def badge_key(speed):
    """The dict key of a badge PNG: the factor at 3 decimals."""
    return round(float(speed), 3)


# ---------------- the plan ----------------

def clip_lines(cl):
    return [cl["title"]] if isinstance(cl["title"], str) else list(cl["title"])


def card_lines(cl):
    """The lines of a title card: the title lines plus the URL when the card carries one —
    the URL is its own line (the download address) and the accent line. An empty or missing
    url omits the line (the repo is private for now; the owner fills the field later)."""
    lines = clip_lines(cl)
    url = cl.get("url") or ""
    if url:
        return lines + [url], len(lines)
    return lines, cl.get("accent")


def parse_plan(plan):
    """The plan dict, validated: scenes with ids and non-empty clip lists; a clip is a source
    cut (source/start/end/speed of an existing file) or a title card (title lines + seconds)."""
    if not isinstance(plan, dict) or not isinstance(plan.get("scenes"), list) or not plan["scenes"]:
        raise ComposeError("the plan needs a non-empty 'scenes' list")
    for sc in plan["scenes"]:
        if not sc.get("id") or not isinstance(sc.get("clips"), list) or not sc["clips"]:
            raise ComposeError(f"every scene needs an id and a non-empty clips list: {sc!r}")
        for cl in sc["clips"]:
            if not isinstance(cl, dict):
                raise ComposeError(f"{sc['id']}: a clip is a dict: {cl!r}")
            if "title" in cl:
                if not clip_lines(cl) or not all(isinstance(t, str) and t for t in clip_lines(cl)):
                    raise ComposeError(f"{sc['id']}: a title card needs at least one line")
                if not isinstance(cl.get("seconds"), (int, float)) or cl["seconds"] <= 0:
                    raise ComposeError(f"{sc['id']}: a title card needs positive seconds")
                if "accent" in cl and not isinstance(cl["accent"], int):
                    raise ComposeError(f"{sc['id']}: accent is the line index")
                if "url" in cl and not isinstance(cl["url"], str):
                    raise ComposeError(f"{sc['id']}: the url must be a string (empty omits the line)")
            else:
                src = cl.get("source")
                if not src:
                    raise ComposeError(f"{sc['id']}: a clip needs a source or title lines: {cl!r}")
                start, end = cl.get("start"), cl.get("end")
                if not isinstance(start, (int, float)) or not isinstance(end, (int, float)) \
                        or not 0 <= start < end:
                    raise ComposeError(f"{sc['id']}: {src} needs 0 <= start < end")
                speed = cl.get("speed", 1.0)
                if not isinstance(speed, (int, float)) or speed <= 0 or speed > MAX_SPEED:
                    raise ComposeError(f"{sc['id']}: {src}: speed {speed} over the {MAX_SPEED}x cap")
                if not expand(src).is_file():
                    raise ComposeError(f"{sc['id']}: the clip file is missing: {src}")
    return plan


def load_narration(tts_dir, scene_ids):
    """[(scene id, wav path, seconds)] in plan order, from tts.py's scenes.json."""
    meta = json.loads((tts_dir / "scenes.json").read_text(encoding="utf-8"))
    by_id = {r["id"]: r for r in meta["scenes"]}
    items = []
    for sid in scene_ids:
        if sid not in by_id:
            raise ComposeError(f"no narration for scene {sid} in {tts_dir} "
                               f"(run tools/media/tts.py, or pass --tts-out)")
        items.append((sid, tts_dir / by_id[sid]["file"], float(by_id[sid]["seconds"])))
    return items


# ---------------- the scene timing ----------------

def clip_length(cl):
    """The clip's playback length: (end - start) / speed, or the card's seconds."""
    if "title" in cl:
        return float(cl["seconds"])
    return (cl["end"] - cl["start"]) / float(cl.get("speed", 1.0))


def frame_count(seconds):
    """The scene's frame count on the 30 fps grid."""
    return max(1, round(seconds * FPS))


def scene_plan(clips, narration_s):
    """One scene's timing on the frame grid: the scene lands on exactly
    frame_count(narration) frames — a short clip list holds its last frame (hold), a long one
    is cut at the scene end (cut). Returns the frame count, the video seconds, hold, cut, and
    the badge windows of the speed > 1x clips, in clip order, on the scene-local timeline."""
    video_s = frame_count(narration_s) / FPS
    raw = sum(clip_length(cl) for cl in clips)
    hold = max(0.0, video_s - raw)
    cut = max(0.0, raw - video_s)
    badges, t = [], 0.0
    for i, cl in enumerate(clips):
        L = clip_length(cl)
        if i == len(clips) - 1:
            L = min(L, video_s - t)   # the last clip fills the scene or is cut at its end
        if "title" not in cl and float(cl.get("speed", 1.0)) > 1.0 and L > 0:
            badges.append((t, t + L))
        t += L
    return {"frames": frame_count(narration_s), "video_s": video_s, "raw_s": raw,
            "hold": hold, "cut": cut, "badges": badges}


def scene_starts(scenes_plan):
    """The scene starts on the frame grid: the exact video boundaries the narration sits on."""
    starts, t = [], 0.0
    for sp in scenes_plan:
        starts.append(t)
        t += sp["video_s"]
    return starts


# ---------------- the inputs ----------------

def ffprobe_seconds(path):
    ffprobe = shutil.which("ffprobe")
    if not ffprobe:
        raise ComposeError("no ffprobe on PATH (ffmpeg is needed for the compose)")
    r = subprocess.run([ffprobe, "-v", "error", "-show_entries", "format=duration",
                        "-of", "default=nw=1:nk=1", str(path)], capture_output=True, text=True)
    if r.returncode or not r.stdout.strip():
        raise ComposeError(f"ffprobe failed on {path}: {r.stderr.strip()}")
    return float(r.stdout.strip())


def check_clip_ends(plan):
    """A clip must end inside its file: a drift FAIL would only say 'short' otherwise."""
    for sc in plan["scenes"]:
        for cl in sc["clips"]:
            if "title" in cl:
                continue
            dur = ffprobe_seconds(expand(cl["source"]))
            if cl["end"] > dur + 0.05:
                raise ComposeError(f"{sc['id']}: {cl['source']} ends at {cl['end']} s "
                                   f"but the file is {dur:.3f} s")


def build_narration(items, starts, total_s, path):
    """One narration WAV: every scene's audio at its scene's video start (the frame grid), so
    the audio never drifts against the video by more than the scene's own rounding."""
    rate = None
    for _, wav, _ in items:
        with wave.open(str(wav), "rb") as w:
            if (w.getnchannels(), w.getsampwidth()) != (1, 2):
                raise ComposeError(f"{wav.name}: the narration must be mono 16-bit (tts.py writes that)")
            rate = w.getframerate() if rate is None else rate
            if w.getframerate() != rate:
                raise ComposeError(f"{wav.name}: rate {w.getframerate()} != {rate}")
    buf = bytearray(int(round(total_s * rate)) * 2)
    for (sid, wav, _), start in zip(items, starts):
        with wave.open(str(wav), "rb") as w:
            data = w.readframes(w.getnframes())
        off = int(round(start * rate)) * 2
        end = min(len(buf), off + len(data))
        if end > off:
            buf[off:end] = data[:end - off]
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(bytes(buf))


# ---------------- the rendered stills (record.py's way: PNGs via Playwright) ----------------

def _record_module():
    spec = importlib.util.spec_from_file_location("ds_record", ROOT / "tools" / "media" / "record.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def render_badges(work, factors):
    """record.py's badge PNGs (the same look), one per distinct factor over 1x."""
    return {badge_key(f): p for f, p in _record_module().render_badges(work, list(factors)).items()}


def render_title_card(work, name, lines, accent):
    """A 1920x1080 title card PNG: the media plan's dark card, the lines centred. The
    "AI-generated" corner note is not baked in here — build_graph overlays the shared note
    PNG on every card segment, so the cards match the sources' baked-in notes."""
    sizes = [TITLE_PX[0] if len(lines) == 1 else TITLE_PX[1]] + [SUB_PX] * (len(lines) - 1)
    rows = []
    for i, (text, size) in enumerate(zip(lines, sizes)):
        if accent is not None and i == accent:
            color = ACCENT
        elif i == 0:
            color = TEXT
        else:
            color = SUB
        weight = 700 if i == 0 else 400
        top = f"margin-top:{LINE_GAP_PX}px;" if i else ""
        rows.append(f'<div style="{top}font-size:{size}px;font-weight:{weight};'
                    f'color:{color};line-height:1.15">{html.escape(text)}</div>')
    card = (f'<body style="margin:0;width:{OUT_W}px;height:{OUT_H}px;background:{BG};'
            f'font-family:{FONT_STACK}">'
            f'<div style="display:flex;align-items:center;justify-content:center;'
            f'width:{OUT_W}px;height:{OUT_H}px">'
            f'<div style="text-align:center">{"".join(rows)}</div></div>')
    path = work / f"card-{name}.png"

    def build():
        from playwright.sync_api import sync_playwright

        with sync_playwright() as pw:
            br = pw.chromium.launch()
            pg = br.new_page(viewport={"width": OUT_W, "height": OUT_H})
            pg.set_content(card)
            pg.locator("body").screenshot(path=str(path))
            br.close()
        return path
    return _record_module()._render_off_thread(build)


def render_note(work):
    """The transparent "AI-generated" note PNG (the explainers' disclosure corner note, owner
    2026-10-23) — record.py's shared renderer: the recorder burns the same PNG into its own
    output (the landing page plays the three recordings as their own videos) and the compose
    overlays it on every frame of the product video, the title cards included. Right 28 px,
    bottom 20 px."""
    return _record_module().render_note(work)


# ---------------- the ffmpeg graph ----------------

def build_graph(plan, scenes_plan, badge_files, cards, note):
    """The -filter_complex: per scene, the clips trimmed and sped onto the 30 fps grid (concat,
    last-frame hold or cut at the scene end, the badge over each sped-up span), then the global
    concat. Every source clip carries its own baked-in "AI-generated" note (the explainers
    render theirs, the recordings record.py's identical one), so the shared note PNG goes only
    on the title-card segments, which are rendered here and carry none. Returns (parts, inputs,
    last); inputs carries one entry per -i, in order."""
    rec = _record_module()   # the note's constants: the shared renderer's (record.py burns it too)
    inputs, parts, chains = [], [], []

    def add_input(entry):
        inputs.append(entry)
        return len(inputs) - 1

    badge_of = {f: add_input({"kind": "file", "path": p}) for f, p in badge_files.items()}
    note_i = add_input({"kind": "file", "path": note})
    for k, (sc, sp) in enumerate(zip(plan["scenes"], scenes_plan)):
        segs = []
        for j, cl in enumerate(sc["clips"]):
            if "title" in cl:
                i = add_input({"kind": "image", "path": cards[(k, j)],
                               "seconds": float(cl["seconds"])})
                parts.append(f"[{i}:v][{note_i}:v]overlay=x=main_w-overlay_w-{rec.NOTE_MARGIN_PX}"
                             f":y=main_h-overlay_h-{rec.NOTE_BOTTOM_PX},fps={FPS},"
                             f"format=yuv420p[t{k}_{j}]")
                segs.append(f"[t{k}_{j}]")
            else:
                i = add_input({"kind": "file", "path": expand(cl["source"])})
                speed = float(cl.get("speed", 1.0))
                parts.append(f"[{i}:v]trim=start={cl['start']:.6f}:end={cl['end']:.6f},"
                             f"setpts=(PTS-STARTPTS)/{speed:.6f},fps={FPS},"
                             f"scale={OUT_W}:{OUT_H}:flags=lanczos,setsar=1[c{k}_{j}]")
                segs.append(f"[c{k}_{j}]")
        cur = segs[0]
        if len(segs) > 1:
            parts.append("".join(segs) + f"concat=n={len(segs)}:v=1:a=0[sc{k}]")
            cur = f"[sc{k}]"
        filters = []
        if sp["hold"] > 0:
            filters.append(f"tpad=stop_mode=clone:stop_duration={sp['hold']:.6f}")
        filters.append(f"trim=end={sp['video_s']:.6f}")
        parts.append(f"{cur}" + ",".join(filters) + f"[sc{k}s]")
        cur = f"[sc{k}s]"
        badge_i = 0
        for cl in sc["clips"]:
            if "title" not in cl and float(cl.get("speed", 1.0)) > 1.0:
                a, b = sp["badges"][badge_i]
                badge_i += 1
                parts.append(f"{cur}[{badge_of[badge_key(cl['speed'])]}:v]"
                             f"overlay=x=main_w-overlay_w-{BADGE_X_S}:y={BADGE_Y_S}"
                             f":enable='between(t,{a:.3f},{b:.3f})'[sc{k}b]")
                cur = f"[sc{k}b]"
        chains.append(cur)
    parts.append("".join(chains) + f"concat=n={len(chains)}:v=1:a=0,format=yuv420p[vout]")
    return parts, inputs, "[vout]"


# ---------------- the compose ----------------

def compose(plan, tts_dir, out, work, keep=False):
    """The compose. Returns (scenes_plan, items, total); writes out."""
    plan = parse_plan(plan)
    items = load_narration(tts_dir, [sc["id"] for sc in plan["scenes"]])
    check_clip_ends(plan)
    scenes_plan = [scene_plan(sc["clips"], narr_s)
                   for sc, (_, _, narr_s) in zip(plan["scenes"], items)]
    for sc, sp, (_, _, narr_s) in zip(plan["scenes"], scenes_plan, items):
        if abs(sp["video_s"] - narr_s) > DRIFT_LIMIT_S:
            raise ComposeError(f"{sc['id']}: the scene is {sp['video_s']:.3f} s against a "
                               f"narration of {narr_s:.3f} s (over the {DRIFT_LIMIT_S * 1000:.0f} ms limit)")
    total = sum(sp["video_s"] for sp in scenes_plan)
    if not keep:
        shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True, exist_ok=True)
    build_narration(items, scene_starts(scenes_plan), total, work / "narration.wav")
    factors = sorted({float(cl.get("speed", 1.0)) for sc in plan["scenes"] for cl in sc["clips"]
                      if "title" not in cl and float(cl.get("speed", 1.0)) > 1.0})
    badge_files = render_badges(work, factors) if factors else {}
    cards = {}
    for k, sc in enumerate(plan["scenes"]):
        for j, cl in enumerate(sc["clips"]):
            if "title" in cl:
                lines, accent = card_lines(cl)
                cards[(k, j)] = render_title_card(work, f"{k}-{j}", lines, accent)
    parts, inputs, last = build_graph(plan, scenes_plan, badge_files, cards, render_note(work))
    narration_idx = len(inputs)
    cmd = ["ffmpeg", "-y", "-v", "error"]
    for entry in inputs:
        if entry["kind"] == "lavfi":
            cmd += ["-f", "lavfi", "-i", entry["src"]]
        elif entry["kind"] == "image":
            cmd += ["-loop", "1", "-t", f"{entry['seconds']:.6f}", "-i", str(entry["path"])]
        else:
            cmd += ["-i", str(entry["path"])]
    cmd += ["-i", str(work / "narration.wav"), "-filter_complex", ";".join(parts),
            "-map", last, "-map", f"{narration_idx}:a",
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart", "-t", f"{total:.6f}",
            str(out)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        raise ComposeError(f"ffmpeg failed: {r.stderr.strip()[-800:]}")
    if not keep:
        shutil.rmtree(work, ignore_errors=True)
    return scenes_plan, items, total


def prepare_work(work, keep):
    if keep:
        work.mkdir(parents=True, exist_ok=True)
    else:
        shutil.rmtree(work, ignore_errors=True)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("plan", type=Path, help="tools/media/product-video.json")
    ap.add_argument("--tts-out", type=Path, help="the narration dir of tools/media/tts.py")
    ap.add_argument("--out", type=Path, help="the MP4 (default: the plan's out)")
    ap.add_argument("--keep", action="store_true", help="keep the work dir")
    args = ap.parse_args(argv)
    try:
        plan = parse_plan(json.loads(args.plan.read_text(encoding="utf-8")))
        if not args.tts_out:
            raise ComposeError("no --tts-out (the plan's narration.dir is relative to it)")
        tts_dir = args.tts_out / plan.get("narration", {}).get("dir", "")
        out = args.out or expand(plan.get("out", str(CACHE / "product" / (plan["name"] + ".mp4"))))
        out.parent.mkdir(parents=True, exist_ok=True)
        work = out.parent / "work"
        prepare_work(work, args.keep)
        scenes_plan, items, total = compose(plan, tts_dir, out, work, args.keep)
        real = ffprobe_seconds(out)
        print(f"{plan['name']}: {len(plan['scenes'])} scenes, {real:.1f} s -> {out}")
        for (sid, _, narr_s), sp in zip(items, scenes_plan):
            line = f"  {sid}: narration {narr_s:.2f} s, video {sp['video_s']:.3f} s, " \
                   f"drift {(sp['video_s'] - narr_s) * 1000:+.0f} ms"
            if sp["hold"] > 0:
                line += f", last frame held {sp['hold']:.2f} s"
            if sp["cut"] > 0:
                line += f", cut {sp['cut']:.2f} s"
            print(line)
        bad = [(sid, sp, narr_s) for (sid, _, narr_s), sp in zip(items, scenes_plan)
               if abs(sp["video_s"] - narr_s) > DRIFT_LIMIT_S]
        if bad:
            for sid, sp, narr_s in bad:
                print(f"FAIL: {sid}: drift {(sp['video_s'] - narr_s) * 1000:+.0f} ms "
                      f"(limit {DRIFT_LIMIT_S * 1000:.0f} ms)", file=sys.stderr)
            return 1
        if abs(real - total) > DRIFT_LIMIT_S:
            print(f"FAIL: the file is {real:.3f} s against a plan of {total:.3f} s", file=sys.stderr)
            return 1
        return 0
    except ComposeError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
