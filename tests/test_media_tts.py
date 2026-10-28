"""#241: tools/media/tts.py against a fake Gemini API. No network, no real key."""
import base64
import io
import json
import struct
import sys
import urllib.error
from pathlib import Path
from types import SimpleNamespace

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools" / "media"))
import tts  # noqa: E402

KEY = "planted-fake-key-0123456789"
SCRIPT = """Preamble, not spoken.

## 01-hook
> visual: title card
Drum hits are fast.
Really fast.

## 02-end
One line.
"""


def wav_bytes(samples, data_size=0, trailing=b""):
    """A WAV as the API streams it: the data length is data_size (0 by default), then the audio, then trailing chunks."""
    pcm = struct.pack(f"<{len(samples)}h", *samples)
    fmt = struct.pack("<HHIIHH", 1, 1, tts.RATE, tts.RATE * 2, 2, 16)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", data_size) + pcm + trailing
    return b"RIFF" + struct.pack("<I", len(body)) + body


def c2pa_chunk(payload_len=600):
    return b"C2PA" + struct.pack("<I", payload_len) + b"\x01" * payload_len


def audio_reply(samples=None):
    raw = wav_bytes(samples or [1000] * 2400)
    return json.dumps({"candidates": [{"content": {"parts": [
        {"inlineData": {"mimeType": "audio/wav", "data": base64.b64encode(raw).decode()}}]}}]}).encode()


class FakeResponse:
    def __init__(self, payload):
        self.payload = payload

    def read(self):
        return self.payload

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


@pytest.fixture
def api(monkeypatch, tmp_path):
    calls, replies, sleeps = [], [], []

    def fake_urlopen(req, timeout=None):
        calls.append(req)
        reply = replies.pop(0) if replies else audio_reply()
        if isinstance(reply, Exception):
            raise reply
        return FakeResponse(reply)

    monkeypatch.setattr(tts.urllib.request, "urlopen", fake_urlopen)
    monkeypatch.setattr(tts.time, "sleep", sleeps.append)
    monkeypatch.setattr(tts, "CACHE_ROOT", tmp_path / "cache")
    monkeypatch.setenv("GEMINI_API_KEY", KEY)
    script = tmp_path / "explainer-test.md"
    script.write_text(SCRIPT, encoding="utf-8")
    return SimpleNamespace(calls=calls, replies=replies, sleeps=sleeps, script=script, out=tmp_path / "out")


def http_error(code):
    return urllib.error.HTTPError("https://example.invalid", code, "err", {}, io.BytesIO(b""))


def test_scenes_are_the_spoken_lines_under_each_heading():
    assert tts.parse_scenes(SCRIPT) == [
        ("01-hook", "Drum hits are fast.\nReally fast."),
        ("02-end", "One line."),
    ]


def test_request_is_the_bare_scene_text_with_the_voice(api):
    assert tts.main([str(api.script), "--out", str(api.out)]) == 0
    body = json.loads(api.calls[0].data)
    assert body["contents"][0]["parts"][0]["text"] == "Drum hits are fast.\nReally fast."
    assert "systemInstruction" not in body
    assert body["generationConfig"]["responseModalities"] == ["AUDIO"]
    voice = body["generationConfig"]["speechConfig"]["voiceConfig"]["prebuiltVoiceConfig"]["voiceName"]
    assert voice == tts.VOICE
    assert api.calls[0].full_url.endswith(f"/{tts.MODEL}:generateContent")
    assert api.calls[0].get_header("X-goog-api-key") == KEY


def test_one_wav_per_scene_and_scenes_json_with_durations(api):
    assert tts.main([str(api.script), "--out", str(api.out)]) == 0
    assert sorted(p.name for p in api.out.glob("*.wav")) == ["01-hook.wav", "02-end.wav"]
    data = json.loads((api.out / "scenes.json").read_text(encoding="utf-8"))
    assert [s["id"] for s in data["scenes"]] == ["01-hook", "02-end"]
    assert data["scenes"][0]["seconds"] == pytest.approx(0.1)


def test_a_cached_scene_makes_no_request(api, capsys):
    assert tts.main([str(api.script), "--out", str(api.out)]) == 0
    assert len(api.calls) == 2
    capsys.readouterr()
    assert tts.main([str(api.script), "--out", str(api.out)]) == 0
    assert len(api.calls) == 2
    assert capsys.readouterr().out.count("(cached)") == 2


def test_missing_key_gives_a_clear_error_without_a_request(api, monkeypatch, capsys):
    monkeypatch.delenv("GEMINI_API_KEY")
    assert tts.main([str(api.script), "--out", str(api.out)]) == 1
    err = capsys.readouterr().err
    assert "GEMINI_API_KEY is not set" in err
    assert api.calls == []


def test_429_then_200_retries_with_backoff(api):
    api.replies.extend([http_error(429), audio_reply()])
    rate, pcm = tts.generate("Hello.", tts.VOICE, tts.MODEL, KEY)
    assert rate == tts.RATE
    assert len(pcm) // 2 == 2400
    assert len(api.calls) == 2
    assert api.sleeps == [2.0]


def test_500_gives_up_after_the_retries(api):
    api.replies.extend([http_error(503)] * tts.RETRIES)
    with pytest.raises(tts.TTSError, match="HTTP 503"):
        tts.generate("Hello.", tts.VOICE, tts.MODEL, KEY)
    assert len(api.calls) == tts.RETRIES


def test_a_gone_model_falls_back_for_the_rest_of_the_run(api):
    api.replies.extend([http_error(404), audio_reply(), audio_reply()])
    assert tts.main([str(api.script), "--out", str(api.out)]) == 0
    assert tts.MODEL in api.calls[0].full_url
    assert all(tts.FALLBACK_MODEL in c.full_url for c in api.calls[1:])
    assert json.loads((api.out / "scenes.json").read_text(encoding="utf-8"))["model"] == tts.FALLBACK_MODEL


def test_the_key_never_appears_in_output_or_files(api, capsys):
    api.replies.extend([http_error(403)])
    assert tts.main([str(api.script), "--out", str(api.out)]) == 1
    assert tts.main([str(api.script), "--out", str(api.out)]) == 0
    captured = capsys.readouterr()
    assert KEY not in captured.out + captured.err
    for path in list(api.out.rglob("*")) + list((tts.CACHE_ROOT).rglob("*")):
        if path.is_file():
            assert KEY.encode() not in path.read_bytes(), path


def test_zero_scenes_is_a_failure_without_a_request(api, capsys):
    api.script.write_text("No headings here.\n", encoding="utf-8")
    assert tts.main([str(api.script), "--out", str(api.out)]) == 1
    assert "FAIL: 0 scenes" in capsys.readouterr().err
    assert api.calls == []


def test_a_wav_with_data_length_0_parses_to_the_full_audio():
    samples = [300, -300] * 1200
    rate, pcm = tts.decode_audio(wav_bytes(samples, data_size=0))
    assert rate == tts.RATE
    assert struct.unpack(f"<{len(samples)}h", pcm) == tuple(samples)


def test_a_planted_trailing_chunk_is_cut_and_the_end_is_faded():
    samples = [1000] * 2400
    raw = wav_bytes(samples, data_size=0, trailing=c2pa_chunk())
    _, pcm = tts.decode_audio(raw)
    assert len(pcm) // 2 == 2400
    assert b"C2PA" not in pcm
    faded = struct.unpack(f"<{len(samples)}h", tts.fade_tail(pcm))
    assert faded[0] == 1000
    assert faded[-1] == 0


def test_a_trailing_chunk_after_a_known_data_length_is_cut():
    samples = [500] * 2400
    raw = wav_bytes(samples, data_size=len(samples) * 2, trailing=c2pa_chunk())
    _, pcm = tts.decode_audio(raw)
    assert struct.unpack(f"<{len(samples)}h", pcm) == tuple(samples)


def test_raw_pcm_is_accepted_with_its_rate():
    raw = struct.pack("<4h", 1, 2, 3, 4)
    assert tts.decode_audio(raw, "audio/L16;codec=pcm;rate=16000") == (16000, raw)


def text_reply(text=None, block="SAFETY"):
    """A transcription answer: the heard text, or a block (no candidates)."""
    body = {"candidates": [{"content": {"parts": [{"text": text}]}}]} if text is not None else {
        "promptFeedback": {"blockReason": block}}
    return json.dumps(body).encode()


def test_word_diffs_ignore_number_and_unit_spelling():
    script = "A 1 ms gap at 48 kHz, drumspike and session's 2 cm mic."
    heard = "a one millisecond gap at 48 kilohertz, Drum Spike and sessions two centimetres mic"
    assert tts.word_diffs(script, heard) == []


def test_word_diffs_name_a_changed_word():
    assert tts.word_diffs("Drum hits are fast.", "Drum hits are slow.") == [("replace", "fast", "slow")]


def test_check_reports_a_word_diff_against_a_fake_transcript(api, capsys):
    api.replies.extend([audio_reply(), text_reply("Drum hits are slow. Really fast."),
                        audio_reply(), text_reply("One line.")])
    assert tts.main([str(api.script), "--out", str(api.out), "--check"]) == 1
    out, err = capsys.readouterr()
    assert "01-hook: 6 script words, 6 heard, 1 diffs" in out
    assert "replace: 'fast' -> 'slow'" in out
    assert "TOTAL diffs 1" in out
    assert "FAIL: 1 word diff" in err
    assert (api.out / "01-hook.txt").read_text(encoding="utf-8") == "Drum hits are slow. Really fast.\n"


def test_check_passes_when_the_transcript_matches(api):
    api.replies.extend([audio_reply(), text_reply("Drum hits are fast. Really fast."),
                        audio_reply(), text_reply("One line.")])
    assert tts.main([str(api.script), "--out", str(api.out), "--check"]) == 0


def test_a_blocked_transcription_retries_then_takes_the_next_model(api):
    api.replies.extend([text_reply(block="OTHER")] * 3 + [text_reply("Hello.")])
    assert tts.transcribe(b"RIFF", KEY) == "Hello."
    models = [c.full_url.rsplit("/", 1)[1].split(":")[0] for c in api.calls]
    assert models == [tts.TRANSCRIBE_MODEL] * 3 + [tts.TRANSCRIBE_FALLBACKS[0]]
    body = json.loads(api.calls[0].data)
    assert body["contents"][0]["parts"][0]["inlineData"]["mimeType"] == "audio/wav"
    assert body["contents"][0]["parts"][1]["text"] == tts.TRANSCRIBE_PROMPT
