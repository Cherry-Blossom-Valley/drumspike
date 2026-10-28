"""#402: the Audio Spike figure of docs/guide/05-export.md, from the demo session.

    uv run --project . python tools/media/fig_audio_spike.py

One kick hit of the demo session (`k in.wav`): about 5 ms before the onset to 80 ms after it,
the raw waveform in the app's cobalt, the smoothed low tone (below 800 Hz) in the playhead
orange, the snippet range shaded with its zero-crossing cut marked. The x axis is milliseconds
relative to the hit's onset, with readable ticks. The snippet range comes from
audio_spike.py, the Python mirror of crates/core/src/spike.rs (#378). Writes
docs/guide/img/05-audio-spike.png (in git) and the landing-page poster
site/assets/posters/explainer-05.jpg (in git, under 150 KB).
"""
import json
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(ROOT / "tools" / "media" / "explainers"))
sys.path.insert(0, str(ROOT / "tools" / "media"))

import audio_spike as A  # noqa: E402
import physics as P  # noqa: E402

COBALT = "#4da3ff"
ORANGE = "#ff9f43"
BG = "#14161a"
TEXT = "#cdd3de"
DIM = "#7d8593"
PRE_S = 0.005  # ms of silence before the onset
SPAN_S = 0.080  # the hit's body after the onset


def pick_kick():
    """(samples, start index) of a clear kick hit of the demo session."""
    truth = json.loads((P.DEMO_DIR / "truth.json").read_text(encoding="utf-8"))
    kicks = [h["t"] / truth["sr"] for h in truth["hits"] if h["drum"] == "kick"]
    x = P.demo_track("k in")
    return x, round(kicks[3] * A.RATE)


def main():
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    x, start = pick_kick()
    smooth = A.low_component(x)
    end, found = A.snippet_end(x, smooth, start)
    assert found, "the kick hit has no counted-lobe snippet"

    lo = max(0, start - round(PRE_S * A.RATE))
    hi = min(len(x), start + round(SPAN_S * A.RATE))
    step = 2  # 24 kHz draw rate
    t = (np.arange(lo, hi, step) - start) / A.RATE * 1000.0  # ms relative to the onset

    fig, ax = plt.subplots(figsize=(9.0, 3.4), dpi=110)
    fig.patch.set_facecolor(BG)
    ax.set_facecolor(BG)
    ax.axvspan(0.0, (end - start) / A.RATE * 1000.0, color=ORANGE, alpha=0.15, lw=0,
               label="the snippet")
    ax.plot(t, x[lo:hi:step], color=COBALT, lw=0.9, label="the hit's own audio")
    tone = smooth[lo:hi:step] / max(float(np.max(np.abs(smooth[lo:hi]))), 1e-9)
    tone *= max(float(np.max(np.abs(x[lo:hi]))), 1e-9)
    ax.plot(t, tone, color=ORANGE, lw=1.6, label="the low tone below 800 Hz")
    ax.axvline((end - start) / A.RATE * 1000.0, color=ORANGE, lw=1.4, ls="--",
               label="the cut: a zero crossing")
    ax.set_xlabel("milliseconds from the onset", color=DIM, fontsize=9)
    ax.set_xticks(np.arange(-5.0, SPAN_S * 1000 + 0.1, 10.0))
    ax.set_xlim(t[0], t[-1])
    ax.set_yticks([])
    for spine in ax.spines.values():
        spine.set_color(DIM)
    ax.tick_params(colors=DIM, labelsize=8)
    leg = ax.legend(loc="upper right", fontsize=9, facecolor=BG, edgecolor=DIM)
    for txt in leg.get_texts():
        txt.set_color(TEXT)
    fig.tight_layout()
    out = ROOT / "docs" / "guide" / "img" / "05-audio-spike.png"
    fig.savefig(out, facecolor=BG)
    print(f"wrote {out} ({out.stat().st_size} bytes); snippet "
          f"{(end - start) / A.RATE * 1000:.1f} ms")

    # the landing-page poster: the same picture, JPEG under 150 KB
    poster = ROOT / "site" / "assets" / "posters" / "explainer-05.jpg"
    fig.savefig(poster, facecolor=BG)
    print(f"wrote {poster} ({poster.stat().st_size} bytes)")
    if poster.stat().st_size >= 150 * 1024:
        raise SystemExit(f"FAIL: the poster is 150 KB or more: {poster.stat().st_size} bytes")


if __name__ == "__main__":
    main()
