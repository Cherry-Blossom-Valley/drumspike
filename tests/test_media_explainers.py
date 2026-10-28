"""#246: the explainers' numbers, timing gate and scene lists, without Manim or a render."""
import math
import re
import shutil
import subprocess
import sys
import wave
from pathlib import Path
from types import SimpleNamespace

import pytest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools" / "media" / "explainers"))
import physics  # noqa: E402
import timing  # noqa: E402

SCRIPTS = ROOT / "docs" / "product" / "scripts"
NEEDS_FFMPEG = pytest.mark.skipif(shutil.which("ffmpeg") is None or shutil.which("ffprobe") is None,
                                  reason="needs ffmpeg and ffprobe")


def _wav(path, seconds, rate=24000):
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b"\x10\x00" * int(seconds * rate))
    return path


def test_one_ms_is_48_samples_and_34_cm():
    assert physics.samples_per_ms(48000) == pytest.approx(48.0)
    assert physics.cm_per_ms() == pytest.approx(34.3)


def test_one_ms_notches_at_500_1500_2500():
    assert physics.notches(1.0, max_hz=3000) == pytest.approx([500.0, 1500.0, 2500.0])


def test_a_tenth_of_a_ms_puts_the_first_notch_at_5_khz():
    assert physics.notches(0.1, max_hz=6000)[0] == pytest.approx(5000.0)


def test_no_delay_has_no_notch_and_full_addition():
    assert physics.notches(0.0) == []
    assert physics.comb_gain(500.0, 0.0) == pytest.approx(2.0)


def test_comb_gain_is_zero_at_each_notch():
    for f in physics.notches(1.0, max_hz=4000):
        assert physics.comb_gain(f, 1.0) == pytest.approx(0.0, abs=1e-9)


def test_delayed_shifts_by_whole_samples():
    values = [1.0, 2.0, 3.0, 4.0]
    assert physics.delayed(values, 1000.0 / 48000 * 2, rate=48000) == [0.0, 0.0, 1.0, 2.0]


def test_first_motion_lands_on_the_attack_not_the_coarse_time():
    full = [0.0] * 240 + physics.snare_hit(2400)
    start = physics.first_motion(full, 120)
    assert 200 <= start <= 260  # the hit starts at sample 240; the walk stops at its first motion


def test_velocity_of_db_is_the_default_map():
    # engine.velocity_of_db: -40 dBFS is 20, -6 is 127, linear in dB and clamped outside (#325)
    assert physics.velocity_of_db(-40.0) == 20
    assert physics.velocity_of_db(-6.0) == 127
    assert physics.velocity_of_db(-120.0) == 20
    assert physics.velocity_of_db(0.0) == 127
    assert physics.velocity_of_db(-23.0) == 74  # the midpoint 53.5 ties to even


def test_peak_db_reads_the_window_from_5_ms_before_to_60_ms_after():
    x = [0.0] * 48000
    x[4800] = 0.5  # a peak at 0.1 s
    x[4800 - 288] = 1.0  # 6 ms before the hit: outside the window
    assert physics.peak_db(x, 0.1) == pytest.approx(20 * math.log10(0.5))


def test_explainer_8_follows_a_ghost_a_normal_hit_and_an_accent():
    import demo_levels
    rows = demo_levels.snare_hits()
    picks = demo_levels.picked(rows)
    assert picks["ghost"]["ghost"] and not picks["normal"]["ghost"] and not picks["accent"]["ghost"]
    assert picks["ghost"]["level_db"] < picks["normal"]["level_db"] < picks["accent"]["level_db"]
    # the velocities s04 draws are the map's velocities of the levels it plots
    for name, row in picks.items():
        assert row["velocity"] == physics.velocity_of_db(row["level_db"]), name


def _boxes_meet(a, b):
    return (a.get_left()[0] < b.get_right()[0] and b.get_left()[0] < a.get_right()[0]
            and a.get_bottom()[1] < b.get_top()[1] and b.get_bottom()[1] < a.get_top()[1])


def _assert_no_overlap(labels):
    for i, a in enumerate(labels):
        for b in labels[i + 1:]:
            assert not _boxes_meet(a, b), f"{a.text!r} overlaps {b.text!r}"


def _min_separation_px(a, b):
    """The distance between two boxes in px at 1080p (the frame's 8 units are 1080 px); <= 0 if they meet."""
    sep_x = max(a.get_left()[0] - b.get_right()[0], b.get_left()[0] - a.get_right()[0])
    sep_y = max(a.get_bottom()[1] - b.get_top()[1], b.get_bottom()[1] - a.get_top()[1])
    return max(sep_x, sep_y) * 135.0


def test_explainer_8_hit_names_and_map_labels_do_not_overlap():
    # #325 review: "normal" and "ghost" ran together in s01; the top-right labels of s04 stacked on each other.
    # Needs Manim for the text boxes: `uv run --with manim pytest` (build.py installs it for the render).
    pytest.importorskip("manim")
    import demo_levels
    import explainer_08
    samples, _ = demo_levels.session()
    picks = demo_levels.picked(demo_levels.snare_hits())
    _assert_no_overlap(list(explainer_08.hit_labels(picks, len(samples) / explainer_08.RATE)))
    velocities = {n: physics.velocity_of_db(r["level_db"]) for n, r in picks.items()}
    _assert_no_overlap(list(explainer_08.map_labels(picks, velocities, explainer_08.map_xy)))


def test_explainer_8_s03_bar_labels_have_a_visible_gap():
    # #371: the s03 bar labels "normal" and "accent" touched ("normalaccent"); every pair of labels
    # keeps a visible gap of 8 px at 1080p, and the check fails on the old 0.9 spacing.
    pytest.importorskip("manim")
    import demo_levels
    import explainer_08
    picks = [(n, r["t"], r["level_db"]) for n, r in demo_levels.picked(demo_levels.snare_hits()).items()]
    labels = list(explainer_08.bar_labels(picks))
    assert len(labels) == 6
    for i, a in enumerate(labels):
        for b in labels[i + 1:]:
            assert _min_separation_px(a, b) >= 8.0, f"{a.text!r} and {b.text!r} are closer than 8 px"
    old = list(explainer_08.bar_labels(picks, spacing=0.9))  # the old layout must fail this check
    assert any(_min_separation_px(a, b) < 8.0 for i, a in enumerate(old) for b in old[i + 1:])


def test_explainer_8_dot_columns_stay_under_the_title():
    # #325 review: the backbeat level's pile ran through the title; a pile taller than DOT_ROWS starts a column
    pytest.importorskip("manim")
    import demo_levels
    import explainer_08
    levels = [r["level_db"] for r in demo_levels.snare_hits()]
    piled = explainer_08.stack(levels, 0.0)
    assert max(-2.4 + y for _, _, y in piled) <= 0.0
    assert len(piled) == len(levels)


def test_timing_gate_passes_when_picture_matches_narration():
    narration = {"s01-a": 10.0, "s02-b": 12.0}
    picture = {"s01-a": 10.05, "s02-b": 11.95}
    assert timing.timing_errors(["s01-a", "s02-b"], narration, picture) == []


def test_timing_gate_fails_drift_over_100_ms():
    errors = timing.timing_errors(["s01-a"], {"s01-a": 10.0}, {"s01-a": 10.2})
    assert len(errors) == 1 and "drift" in errors[0]


def test_timing_gate_fails_a_scene_without_narration_or_picture():
    errors = timing.timing_errors(["s01-a", "s02-b"], {"s01-a": 1.0}, {"s01-a": 1.0, "s03-c": 1.0})
    text = "\n".join(errors)
    assert "s02-b: no narration" in text
    assert "s02-b: no picture" in text
    assert "s03-c: picture scene is not a scene of the script" in text


def test_scene_frames_cut_each_scene_at_its_middle(tmp_path, monkeypatch):
    import build
    calls = []
    monkeypatch.setattr(build.subprocess, "run", lambda cmd, **kw: calls.append(cmd) or SimpleNamespace(returncode=0))
    paths = build.scene_frames(Path("x.mp4"), {"s01-a": 4.0, "s02-b": 2.0}, ["s01-a", "s02-b"], tmp_path)
    assert [cmd[cmd.index("-ss") + 1] for cmd in calls] == ["2.00", "5.00"]
    assert [p.name for p in paths] == ["s01-a.png", "s02-b.png"]


def test_track_lays_each_scene_at_its_picture_start(tmp_path):
    import build
    wavs = {"s01-a": _wav(tmp_path / "a.wav", 0.4), "s02-b": _wav(tmp_path / "b.wav", 0.2)}
    placed = build.lay_track(wavs, {"s01-a": 0.5, "s02-b": 0.3}, ["s01-a", "s02-b"], tmp_path / "track.wav")
    assert placed["s01-a"] == pytest.approx((0.0, 0.0))
    assert placed["s02-b"] == pytest.approx((0.5, 0.5))  # silence after the short first WAV
    with wave.open(str(tmp_path / "track.wav"), "rb") as w:
        assert w.getnframes() == round(0.8 * 24000)  # the picture's length, padded with silence


def test_track_scene_longer_than_its_picture_pushes_the_next_scene(tmp_path):
    import build
    wavs = {"s01-a": _wav(tmp_path / "a.wav", 0.7), "s02-b": _wav(tmp_path / "b.wav", 0.2)}
    placed = build.lay_track(wavs, {"s01-a": 0.5, "s02-b": 0.5}, ["s01-a", "s02-b"], tmp_path / "track.wav")
    assert placed["s02-b"] == pytest.approx((0.5, 0.7))
    errors = timing.track_errors(placed, ["audio", "video"], {"audio": 1.0, "video": 1.0})
    assert errors == ["s02-b: audio drift +200 ms in the track (limit 100 ms)"]


def test_track_gate_wants_one_video_and_one_audio_of_equal_length():
    ok = {"audio": 10.02, "video": 10.0}
    assert timing.track_errors({}, ["audio", "video"], ok) == []
    assert "streams are ['video']" in timing.track_errors({}, ["video"], {"video": 10.0})[0]
    assert "differ by +80 ms" in timing.track_errors({}, ["audio", "video"], {"audio": 10.08, "video": 10.0})[0]
    assert "no duration" in timing.track_errors({}, ["audio", "video"], {"video": 10.0})[0]


@NEEDS_FFMPEG
def test_mux_puts_the_track_on_a_stub_picture_and_passes_the_gate(tmp_path):
    import build
    wavs = {"s01-a": _wav(tmp_path / "a.wav", 0.5), "s02-b": _wav(tmp_path / "b.wav", 0.5)}
    picture = {"s01-a": 0.5, "s02-b": 0.5}
    placed = build.lay_track(wavs, picture, ["s01-a", "s02-b"], tmp_path / "track.wav")
    stub = tmp_path / "stub.mp4"
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=64x64:r=30:d=1",
                    "-c:v", "libx264", "-pix_fmt", "yuv420p", str(stub)], check=True)
    muxed = tmp_path / "muxed.mp4"
    build.mux(stub, tmp_path / "track.wav", muxed, 1.0)
    kinds, durations = build.probe(muxed)
    assert sorted(kinds) == ["audio", "video"]
    assert timing.track_errors(placed, kinds, durations) == []


@NEEDS_FFMPEG
def test_mux_holds_a_picture_that_is_short_of_the_track(tmp_path):
    # Manim's frame count can run short of the planned length: the picture is held to the track's length
    import build
    wavs = {"s01-a": _wav(tmp_path / "a.wav", 1.0)}
    placed = build.lay_track(wavs, {"s01-a": 1.0}, ["s01-a"], tmp_path / "track.wav")
    stub = tmp_path / "stub.mp4"
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=64x64:r=30:d=0.95",
                    "-c:v", "libx264", "-pix_fmt", "yuv420p", str(stub)], check=True)
    muxed = tmp_path / "muxed.mp4"
    build.mux(stub, tmp_path / "track.wav", muxed, 1.0)
    kinds, durations = build.probe(muxed)
    assert durations["video"] == pytest.approx(1.0, abs=0.05)
    assert timing.track_errors(placed, kinds, durations) == []


def test_velocity_of_db_is_the_app_default_map():
    # velocity.rs: -40 dBFS -> 20, -6 dBFS -> 127, linear in dB, clamped outside
    assert physics.velocity_of_db(-40.0) == 20
    assert physics.velocity_of_db(-6.0) == 127
    assert physics.velocity_of_db(5.0) == 127
    assert physics.velocity_of_db(-120.0) == 20
    assert physics.velocity_of_db(-19.6) == 84
    assert physics.velocity_of_db(-6.43) == 126


def test_map_points_draw_the_velocity_of_each_level():
    levels = [-40.0, -19.6, -6.43, -1.0]
    assert physics.map_points(levels) == [(d, physics.velocity_of_db(d)) for d in levels]
    assert [v for _, v in physics.map_points(levels)] == [20, 84, 126, 127]


@pytest.mark.parametrize("module", ["explainer_01.py", "explainer_04.py", "explainer_08.py", "explainer_10.py"])
def test_explainer_captions_go_through_say(module):
    # one caption at a time: a scene adds its line with say(), never a bare caption() that stacks
    source = (ROOT / "tools" / "media" / "explainers" / module).read_text(encoding="utf-8")
    assert "caption(" not in source
    assert "self.say(" in source


@pytest.mark.parametrize("script,module,cls", [
    ("explainer-01-why-one-ms.md", "explainer_01.py", "Explainer01"),
    ("explainer-04-first-sample.md", "explainer_04.py", "Explainer04"),
    ("explainer-08-velocity.md", "explainer_08.py", "Explainer08"),
    ("explainer-10-bleed.md", "explainer_10.py", "Explainer10"),
])
def test_explainer_has_one_method_per_script_scene(script, module, cls):
    ids = timing.script_scene_ids(SCRIPTS / script)
    source = (ROOT / "tools" / "media" / "explainers" / module).read_text(encoding="utf-8")
    assert f"class {cls}(Explainer)" in source
    tuple_block = re.search(r"script_ids = \((.*?)\)", source, re.S).group(1)
    assert re.findall(r'"([^"]+)"', tuple_block) == ids
    for sid in ids:
        assert re.search(rf"def {sid.replace('-', '_')}\(self\)", source), sid


def test_explainer_08_s04_draws_the_velocities_of_map_points():
    # the dots and labels of s04 come from map_points(), the app's velocity_of_db for each plotted level
    source = (ROOT / "tools" / "media" / "explainers" / "explainer_08.py").read_text(encoding="utf-8")
    body = source.split("def s04_map", 1)[1].split("def s05_histogram", 1)[0]
    assert "map_points(" in body
    assert "velocity_of_db(" not in body.replace("map_points", "")


def test_build_knows_explainer_10():
    import build
    assert build.VIDEOS["10"] == ("explainer-10-bleed", "explainer_10.py", "Explainer10")


# #336 review: no two text boxes of a scene intersect, and no text box intersects a plotted
# line or bar (bounding boxes, every scene). Needs Manim and the demo session.
EXPLAINER_10_SCENES = timing.script_scene_ids(SCRIPTS / "explainer-10-bleed.md")


def _run_scene(sid):
    """Play one scene of Explainer10 into a bare Scene: stub play/wait, keep the mobjects."""
    import explainer_10
    from manim import FadeOut
    scene = explainer_10.Explainer10.__new__(explainer_10.Explainer10)
    scene.mobjects = []
    scene.cap = None
    scene.elapsed = 0.0
    scene.sid = sid
    scene.plan = {}
    scene.narration = {}

    def play(*anims, run_time=0.0):
        for a in anims:
            m = getattr(a, "mobject", None)
            if m is None:
                continue
            if isinstance(a, FadeOut):
                scene.mobjects = [x for x in scene.mobjects if x is not m]
            elif not any(x is m for x in scene.mobjects):
                scene.mobjects.append(m)

    scene.play = play
    scene.wait = lambda t: None
    getattr(scene, sid.replace("-", "_"))()
    return scene


def _scene_members(scene):
    seen, texts, plotted = set(), [], []
    from manim import Dot, Text, VMobject

    def walk(m):
        if id(m) in seen:
            return
        seen.add(id(m))
        if isinstance(m, Text):  # a text box is one unit; its SVG-path children stay inside it
            texts.append(m)
            return
        if isinstance(m, VMobject) and not isinstance(m, Dot) and m.get_num_points() > 0:
            plotted.append(m)
        for sub in m.submobjects:
            walk(sub)

    for top in scene.mobjects:
        walk(top)
    return texts, plotted


@pytest.mark.parametrize("sid", EXPLAINER_10_SCENES)
def test_explainer_10_scene_texts_do_not_overlap(sid):
    pytest.importorskip("manim")
    texts, _ = _scene_members(_run_scene(sid))
    assert len(texts) >= 1
    for i, a in enumerate(texts):
        for b in texts[i + 1:]:
            assert not _boxes_meet(a, b), f"{sid}: {a.text!r} overlaps {b.text!r}"


@pytest.mark.parametrize("sid", EXPLAINER_10_SCENES)
def test_explainer_10_scene_text_stays_off_the_plot(sid):
    pytest.importorskip("manim")
    texts, plotted = _scene_members(_run_scene(sid))
    assert len(plotted) >= 1
    for t in texts:
        for p in plotted:
            assert not _boxes_meet(t, p), f"{sid}: {t.text!r} overlaps a plotted line/bar"
