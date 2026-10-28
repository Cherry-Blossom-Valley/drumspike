"""#411: the audio-spike texts of the UI match the #378 snippet rule.

The Export dialog (static/index.html) and the export_all command description
(crates/app/src/api.rs) must not carry the pre-#378 wording: "1.5 cycles of
the track", "1.5 cycles per hit" and "2-30 ms" describe the old snippet (the
attack click's period, capped at 30 ms). Since #378 the snippet covers the
body of the hit: about 1.5 cycles of the drum's low tone, ending on a zero
crossing, clamped to 2-60 ms.
"""

from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FILES = (
    ROOT / "static" / "index.html",
    ROOT / "crates" / "app" / "src" / "api.rs",
)

# The pre-#378 wordings, as the files said them on main before #411.
STALE = ("1.5 cycles of the track", "1.5 cycles per hit", "2-30 ms")


def test_no_stale_audio_spike_texts():
    hits = [
        (str(f.relative_to(ROOT)), s)
        for f in FILES
        for s in STALE
        if s in f.read_text(encoding="utf-8")
    ]
    assert hits == [], f"stale pre-#378 audio-spike wording: {hits}"
