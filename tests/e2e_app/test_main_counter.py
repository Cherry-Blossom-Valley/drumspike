"""#149: the main counter formats like the Pro Tools main counter. Bars|Beats shows bar.beat of
the tempo map at the playhead (static/app.js counterBarsBeats, the tmGrid beat times); Timecode
shows HH:MM:SS:FF computed from the sample position of the playhead at 30 frames per second, non
drop. The sub counter shows the other format of the pair (Bars|Beats<->Timecode). A project with
a known tempo map (one tempo step at 2.0 s to 90 BPM, one meter change to 7/8 at bar 3, 120 BPM
before) is checked at three playhead positions that sit exactly on beats of the map.

Counter of the map (quarter notes): 120 BPM before 2.0 s (0.5 s per beat: bar 1 = 0.0, 0.5, 1.0,
1.5; bar 2 = 2.0, 2.667, 3.333, 4.0), 90 BPM from 2.0 s (0.667 s per beat), 7/8 from bar 3 (bar 3
starts at 4.667 s, its eighth beats are 0.333 s apart, bar 4 at 7.0 s)."""
from app_support import invoke, open_project
from test_editor import build

MAP = {
    "bpm": 120.0,
    "steps": [{"t": 2.0, "bpm": 90.0}],
    "meter": [4, 4],
    "changes": [{"bar": 3, "meter": [7, 8]}],
    "bar1_s": 0.0,
}
# (playhead seconds, bar, beat) — every position exactly on a beat of the map
POSITIONS = [(1.5, 1, 4), (2.0, 2, 1), (5.0, 3, 2)]
SR = 48_000
FPS = 30


def frames_of(samples):
    t = samples / SR
    frames = int(t * FPS)
    return f"{frames // (FPS * 3600) % 100:02d}:{frames // (FPS * 60) % 60:02d}:{frames // FPS % 60:02d}:{frames % FPS:02d} tc"


def set_playhead(app, t):
    app.evaluate("t => { S.playhead = t; drawHead(); }", t)


def test_bars_beats_and_timecode_at_three_positions(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path, name="Counter"))
    invoke(app, "tempo_map_set", sid=sid, map=MAP)
    app.evaluate("async () => { await tmLoad(); }")
    for mode in ("barsbeats", "timecode"):
        app.evaluate(f'm => setCounter("{mode}")')
        for t, bar, beat in POSITIONS:
            set_playhead(app, t)
            samples = round(t * SR)
            main = app.evaluate("() => document.querySelector('#timeDisplay').textContent")
            sub = app.evaluate("() => document.querySelector('#sampleDisplay').textContent")
            if mode == "barsbeats":
                assert main == f"{bar}.{beat}", f"Bars|Beats at {t} s: {main}"
                assert sub == frames_of(samples), f"sub Timecode at {t} s: {sub}"
            else:
                assert main == frames_of(samples), f"Timecode at {t} s: {main}"
                assert sub == f"{bar}.{beat}", f"sub Bars|Beats at {t} s: {sub}"
