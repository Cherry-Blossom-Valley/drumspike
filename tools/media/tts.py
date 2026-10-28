"""#241: Gemini TTS for the product videos: one WAV per scene of a narration script.

    GEMINI_API_KEY=<key> uv run --project . python tools/media/tts.py docs/product/scripts/explainer-01-why-one-ms.md

A script has one `## <scene-id>` heading per scene. The text under a heading is spoken, except the
lines that start with `>` (visual notes). The text before the first heading is not spoken. The
text is sent bare: a style prefix is read aloud by the model.

Writes `<out>/<scene-id>.wav` (24 kHz mono 16-bit) and `<out>/scenes.json` (each scene's duration).
Default out: `~/.cache/drumspike/media/tts-out/<script stem>/`. A scene's WAV is cached by the
sha256 of its text, voice and model in `~/.cache/drumspike/media/tts/`: a cached scene makes no
request. The key comes only from GEMINI_API_KEY; it is never printed, logged or written.

`--check` transcribes each scene's WAV with a Gemini flash model, writes `<out>/<scene-id>.txt` and
compares the words heard with the script's spoken words (number and unit spelling does not count).
It exits 1 on a word diff.
"""
import argparse
import array
import base64
import difflib
import hashlib
import json
import os
import re
import shutil
import struct
import sys
import time
import urllib.error
import urllib.request
import wave
from pathlib import Path

API = "https://generativelanguage.googleapis.com/v1beta/models"
MODEL = "gemini-2.5-pro-preview-tts"  # owner 2026-10-23; a preview, so it can disappear
FALLBACK_MODEL = "gemini-3.8-flash-tts"  # same voice; used for the rest of a run when MODEL is gone (404)
VOICE = "Iapetus"  # owner 2026-10-23
TRANSCRIBE_MODEL = "gemini-3.8-flash"  # --check; the auto-pick chose gemini-omni-*, which needs another API
TRANSCRIBE_FALLBACKS = ("gemini-3.7-flash", "gemini-3.6-flash", "gemini-3.5-flash")  # after a block
TRANSCRIBE_PROMPT = "Transcribe this audio verbatim, every word that is spoken, nothing else."
RATE = 24000
TAIL_FADE = 240  # samples: 10 ms at 24 kHz, faded out at the end of every scene
RETRIES = 4
CACHE_ROOT = Path.home() / ".cache" / "drumspike" / "media"


class TTSError(Exception):
    pass


class ModelGone(TTSError):
    pass


def parse_scenes(text):
    """[(scene id, spoken text)] in script order."""
    scenes = []
    for line in text.splitlines():
        if line.startswith("## "):
            scenes.append((line[3:].strip(), []))
        elif scenes and not line.lstrip().startswith(">"):
            scenes[-1][1].append(line)
    return [(sid, "\n".join(lines).strip()) for sid, lines in scenes]


def request_body(text, voice):
    return {
        "contents": [{"parts": [{"text": text}]}],
        "generationConfig": {
            "responseModalities": ["AUDIO"],
            "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": voice}}},
        },
    }


def post(model, body, key):
    """One generateContent call, as its JSON. Retries 429 and 5xx with backoff. A 404 means the model is gone."""
    if not key:
        raise TTSError("GEMINI_API_KEY is not set: export it in your shell (the value is never printed)")
    data = json.dumps(body).encode()
    delay = 2.0
    for attempt in range(RETRIES):
        req = urllib.request.Request(
            f"{API}/{model}:generateContent",
            data=data,
            method="POST",
            headers={"Content-Type": "application/json", "x-goog-api-key": key},
        )
        last = attempt == RETRIES - 1
        try:
            with urllib.request.urlopen(req, timeout=180) as resp:
                return json.loads(resp.read())
        except urllib.error.HTTPError as e:
            if e.code == 404:
                raise ModelGone(f"model {model} is not available (HTTP 404)")
            if last or not (e.code == 429 or e.code >= 500):
                raise TTSError(f"{model}: HTTP {e.code}")
        except urllib.error.URLError as e:
            if last:
                raise TTSError(f"network error: {e.reason}")
        time.sleep(delay)
        delay *= 2
    raise TTSError(f"{model}: no answer after {RETRIES} tries")


def generate(text, voice, model, key):
    """The audio of one scene as (rate, pcm)."""
    return audio_from_response(post(model, request_body(text, voice), key))


def transcribe(wav_raw, key):
    """The words heard in a WAV. A filter can block the input: retry the model, then the next one.
    A blocked answer is never an empty transcript, since the word check would read it as nothing said."""
    body = {"contents": [{"parts": [
        {"inlineData": {"mimeType": "audio/wav", "data": base64.b64encode(wav_raw).decode()}},
        {"text": TRANSCRIBE_PROMPT}]}]}
    for model in (TRANSCRIBE_MODEL,) * 3 + TRANSCRIBE_FALLBACKS:
        try:
            resp = post(model, body, key)
        except ModelGone:
            continue
        if resp.get("candidates"):
            return resp["candidates"][0]["content"]["parts"][0]["text"]
        print(f"blocked on {model}: {resp.get('promptFeedback')}", file=sys.stderr)
    raise TTSError("every transcription model blocked this audio")


UNITS = {"khz": "kilohertz", "hz": "hertz", "cm": "centimetres", "ms": "milliseconds", "db": "decibels",
         "centimeters": "centimetres", "millisecond": "milliseconds"}
WORDS = {"one": "1", "zero": "0", "two": "2", "three": "3", "five": "5", "eight": "8", "ten": "10"}


def norm_words(text):
    """The words of a text as the ear hears them: lower case, digits joined, 'drumspike' spelled as the
    speaker says it, units and small numbers spelled the same way on both sides."""
    t = re.sub(r"(\d),(\d)", r"\1\2", text.lower().replace("drumspike", "drum spike"))
    t = t.replace("mic'd", "miked")
    t = re.sub(r"(\w)['’](\w)", r"\1\2", t)  # session's = sessions: the ear cannot hear the apostrophe
    return [UNITS.get(w, WORDS.get(w, w)) for w in re.findall(r"[a-z0-9]+", t)]


def word_diffs(script, heard):
    """[(op, script words, heard words)] for every part where the transcript differs from the script."""
    a, b = norm_words(script), norm_words(heard)
    ops = difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes()
    return [(op, " ".join(a[i1:i2]), " ".join(b[j1:j2])) for op, i1, i2, j1, j2 in ops if op != "equal"]


def audio_from_response(resp):
    for cand in resp.get("candidates") or []:
        for part in (cand.get("content") or {}).get("parts") or []:
            inline = part.get("inlineData") or part.get("inline_data")
            if inline and inline.get("data"):
                mime = inline.get("mimeType") or inline.get("mime_type") or ""
                return decode_audio(base64.b64decode(inline["data"]), mime)
    reason = (resp.get("promptFeedback") or {}).get("blockReason", "no candidates")
    raise TTSError(f"no audio in the response ({reason})")


def decode_audio(raw, mime=""):
    """(rate, pcm) from a WAV, or from raw 16-bit little-endian PCM."""
    if raw[:4] == b"RIFF":
        return _wav_pcm(raw)
    m = re.search(r"rate=(\d+)", mime)
    return (int(m.group(1)) if m else RATE), _even(raw)


def _wav_pcm(raw):
    """The samples of a WAV. A streamed header can give a data length of 0: the audio then runs to a
    trailing chunk (such as the C2PA manifest after it) or to the end of the file."""
    rate = channels = bits = None
    pos = 12
    while pos + 8 <= len(raw):
        cid = raw[pos:pos + 4]
        size = struct.unpack_from("<I", raw, pos + 4)[0]
        body = pos + 8
        if cid == b"fmt ":
            _, channels, rate, _, _, bits = struct.unpack_from("<HHIIHH", raw, body)
        elif cid == b"data":
            if channels != 1 or bits != 16:
                raise TTSError(f"unsupported WAV: {channels} channel(s), {bits} bit")
            end = body + size if size else _audio_end(raw, body)
            return rate, _even(raw[body:end])
        pos = body + size + (size & 1)
    raise TTSError("WAV without a data chunk")


def _audio_end(raw, start):
    """The start of the first trailing chunk: a printable 4-character id whose size runs exactly to
    the end of the file (a RIFF pad byte allowed). Without one, the audio runs to the end."""
    n = len(raw)
    for p in range(start, n - 7):
        size = struct.unpack_from("<I", raw, p + 4)[0]
        if size and p + 8 + size in (n, n - 1) and all(0x20 <= c < 0x7F for c in raw[p:p + 4]):
            return p
    return n


def _even(raw):
    return raw[:len(raw) - (len(raw) & 1)]


def fade_tail(pcm):
    """Fade the last TAIL_FADE samples to silence, so a cut at the scene end never clicks."""
    samples = array.array("h")
    samples.frombytes(pcm)  # the hosts are little-endian (arm64, x86_64)
    k = min(TAIL_FADE, len(samples))
    start = len(samples) - k
    for i in range(k):
        samples[start + i] = int(samples[start + i] * (k - 1 - i) / k)
    return samples.tobytes()


def write_wav(path, rate, pcm):
    tmp = path.with_name(path.name + ".tmp")
    with wave.open(str(tmp), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(pcm)
    os.replace(tmp, path)


def cache_path(text, voice, model):
    digest = hashlib.sha256(json.dumps([text, voice, model]).encode()).hexdigest()
    return CACHE_ROOT / "tts" / f"{digest}.wav"


def seconds_of(path):
    with wave.open(str(path), "rb") as w:
        return w.getnframes() / w.getframerate()


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("script", type=Path)
    ap.add_argument("--out", type=Path, help="output dir (default ~/.cache/drumspike/media/tts-out/<script stem>)")
    ap.add_argument("--voice", default=VOICE)
    ap.add_argument("--model", default=MODEL)
    ap.add_argument("--check", action="store_true", help="transcribe each scene and report the word diffs (exit 1 on a diff)")
    args = ap.parse_args(argv)

    scenes = parse_scenes(args.script.read_text(encoding="utf-8"))
    if not scenes:
        print(f"FAIL: 0 scenes in {args.script}", file=sys.stderr)
        return 1
    empty = [sid for sid, body in scenes if not body]
    if empty:
        print(f"FAIL: no spoken text in scene(s) {', '.join(empty)}", file=sys.stderr)
        return 1

    out = args.out or CACHE_ROOT / "tts-out" / args.script.stem
    out.mkdir(parents=True, exist_ok=True)
    key = os.environ.get("GEMINI_API_KEY", "").strip()
    model = args.model
    rows = []
    diffs = 0
    try:
        for sid, body in scenes:
            name = re.sub(r"[^A-Za-z0-9._-]", "_", sid)
            while True:
                cache = cache_path(body, args.voice, model)
                if cache.exists():
                    state = "cached"
                    break
                try:
                    rate, pcm = generate(body, args.voice, model, key)
                except ModelGone as e:
                    if model == FALLBACK_MODEL:
                        raise
                    print(f"{e}: using {FALLBACK_MODEL} for the rest of this run", file=sys.stderr)
                    model = FALLBACK_MODEL
                    continue
                cache.parent.mkdir(parents=True, exist_ok=True)
                write_wav(cache, rate, fade_tail(pcm))
                state = "new"
                break
            shutil.copyfile(cache, out / f"{name}.wav")
            secs = seconds_of(cache)
            rows.append({"id": sid, "file": f"{name}.wav", "seconds": round(secs, 3), "state": state})
            print(f"{name}: {secs:.2f} s ({state})")
            if args.check:
                heard = transcribe((out / f"{name}.wav").read_bytes(), key)
                (out / f"{name}.txt").write_text(heard + "\n", encoding="utf-8")
                ops = word_diffs(body, heard)
                print(f"{name}: {len(norm_words(body))} script words, {len(norm_words(heard))} heard, {len(ops)} diffs")
                for op, want, got in ops:
                    print(f"    {op}: {want!r} -> {got!r}")
                diffs += len(ops)
    except TTSError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        return 1

    total = sum(r["seconds"] for r in rows)
    (out / "scenes.json").write_text(
        json.dumps({"model": model, "voice": args.voice, "scenes": rows}, indent=2) + "\n", encoding="utf-8"
    )
    print(f"{len(rows)} scenes, {total:.2f} s total -> {out}")
    if args.check:
        print(f"TOTAL diffs {diffs}")
        if diffs:
            print(f"FAIL: {diffs} word diff(s) between the script and the audio", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
