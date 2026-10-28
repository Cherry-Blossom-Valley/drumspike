"""P3-7a: the project builder and the report checker of the app self-test (apps/desktop/selftest)."""
import importlib.util
import json
import struct

import pytest

from helpers import ROOT, SR, synth_track, write_wav


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "apps/desktop/selftest" / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def riff(fmt_tag, bits, frames, channels=1, sr=SR, extra_chunk=False):
    """A WAV header plus zero samples: PCM (1), float (3) or extensible (0xFFFE)."""
    block = channels * bits // 8
    fmt = struct.pack("<HHIIHH", fmt_tag, channels, sr, sr * block, block, bits)
    chunks = b"fmt " + struct.pack("<I", len(fmt)) + fmt
    if extra_chunk:
        chunks += b"LIST" + struct.pack("<I", 3) + b"abc" + b"\x00"      # odd size: padded to even
    data = bytes(frames * block)
    chunks += b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", 4 + len(chunks)) + b"WAVE" + chunks


@pytest.mark.parametrize("tag,bits", [(1, 16), (1, 24), (3, 32), (0xFFFE, 24)])
def test_wav_info_reads_rate_and_frames_of_every_format(tmp_path, tag, bits):
    mp = load("make_project")
    f = tmp_path / "a.wav"
    f.write_bytes(riff(tag, bits, 123, channels=2, sr=44100, extra_chunk=True))
    assert mp.wav_info(f) == (44100, 123)


def test_wav_info_refuses_a_file_that_is_no_wav(tmp_path):
    mp = load("make_project")
    (tmp_path / "x.wav").write_bytes(b"not a wave file at all")
    with pytest.raises(ValueError):
        mp.wav_info(tmp_path / "x.wav")
    (tmp_path / "nodata.wav").write_bytes(b"RIFF" + struct.pack("<I", 4) + b"WAVE")
    with pytest.raises(ValueError):
        mp.wav_info(tmp_path / "nodata.wav")


@pytest.mark.parametrize("mode", ["referenced", "copied"])
def test_build_project_writes_a_project_the_loader_opens(tmp_path, mode):
    mp = load("make_project")
    x, _ = synth_track([0.25], 1.0)
    wav = write_wav(tmp_path / "Kick In.wav", x)
    folder = mp.build_project(tmp_path / "out", "Song", [("Kick In", wav, "kick", 36)], mode)
    doc = json.loads((tmp_path / "out" / "Song" / "Song.drumspike").read_text())
    assert doc["format"] == 1 and doc["order"] == ["Kick In"] and doc["settings"] == {"mix_mode": "average"}
    for d in ("Audio Files", "Analysis", "Exports", "Backups"):
        assert (tmp_path / "out" / "Song" / d).is_dir()
    assert str(folder) == str(tmp_path / "out" / "Song")
    tr = doc["tracks"]["Kick In"]
    assert (tr["sr"], tr["n_samples"], tr["role"], tr["note"]) == (SR, SR, "kick", 36)
    assert tr["source"]["mode"] == mode
    assert (mode == "copied") == (tr["source"]["relative"].startswith("Audio Files/"))


def test_standard_tracks_picks_the_naming_scheme_and_counts(tmp_path):
    mp = load("make_project")
    audio = tmp_path / "Audio Files"
    audio.mkdir()
    for fn in ("k in_02.wav", "s top_01.wav", "hh_03.wav", "Bass_01.wav"):
        (audio / fn).write_bytes(b"x")
    got = mp.standard_tracks(tmp_path, 3)
    assert [g[0] for g in got] == ["k in", "s top", "hh"] and got[1][2:] == ("snare", 38)
    with pytest.raises(SystemExit):
        mp.standard_tracks(tmp_path, 4)


def test_main_prints_the_project_folder(tmp_path, capsys):
    mp = load("make_project")
    audio = tmp_path / "sess" / "Audio Files"
    audio.mkdir(parents=True)
    (audio / "k in_01.wav").write_bytes(riff(1, 16, 480))
    mp.main(["--out", str(tmp_path / "o"), "--name", "P", "--from", str(tmp_path / "sess"), "--tracks", "1"])
    assert capsys.readouterr().out.strip().endswith("/o/P")


def test_wav_folder_tracks_and_the_wavs_option(tmp_path, capsys):
    """P3-6a: the 30-lane session of make_session.py (t01.wav ...) becomes a project with --wavs."""
    mp = load("make_project")
    folder = tmp_path / "wavs"
    folder.mkdir()
    for k in range(4):
        (folder / f"t{k + 1:02d}.wav").write_bytes(riff(1, 24, 960, sr=96000))
    got = mp.wav_folder_tracks(folder, 4)
    assert [t[0] for t in got] == ["t01", "t02", "t03", "t04"] and [t[2] for t in got] == ["kick", "snare", "tom", "kick"]
    with pytest.raises(SystemExit):
        mp.wav_folder_tracks(folder, 5)
    mp.main(["--out", str(tmp_path / "o"), "--name", "P", "--wavs", str(folder), "--tracks", "3"])
    assert capsys.readouterr().out.strip().endswith("/o/P")
    with pytest.raises(SystemExit):
        mp.main(["--out", str(tmp_path / "o2"), "--tracks", "3"])      # neither --from nor --wavs


def test_summary_prints_the_render_numbers():
    cr = load("check_report")
    play = {"fps_median": 60, "fps_mean": 59.9, "js_p50_ms": 1, "js_p95_ms": 2, "js_max_ms": 3, "lane_redraws": 10, "frames": 100,
            "frame": {"over_20ms": 0}}
    rep = report(True)
    rep["metrics"]["render"] = {"lanes": 30, "gl": {"renderer": "Apple GPU"}, "warm_ms": 5, "playback_app": play, "playback_every": play, "playback_all_app": play, "playback_all_every": play,
                                "frames_zoom": {"n": 1}, "agreement_2d": [{"min": 1}]}
    text = cr.summary(rep)
    assert "render: 30 lanes" in text and "playback_every: fps median 60" in text and "agreement_2d" in text
    assert "self-test clock" in text
    rep["metrics"]["render"]["playback_app"] = dict(play, transport=True, start_ms=812.5, tracks=30, underruns=0, master=0.0)
    text = cr.summary(rep)
    assert "playback_app: fps median 60" in text and "transport: start 812.5 ms, 30 tracks, underruns 0, master 0.0" in text


def test_summary_prints_the_pan_mode():
    """P4-2: the pan mode of `render` (also alone, `pan_only`): per segment the frames, the missed frames, the JS p95 and
    the draws; the zoom steps."""
    cr = load("check_report")
    seg = {"n": 240, "fps": 60.0, "over_16_7": 130, "missed": 1, "max_ms": 33, "js_p95_ms": 2, "view_draws": 240, "gl_posts": 240}
    rep = report(True)
    rep["metrics"]["render"] = {"lanes": 30, "gl": {"renderer": "Apple GPU"}, "warm_ms": 5,
                                "pan": {"steady_right": seg, "scroll": dict(seg, missed=0), "zoom_steps": [{"step": "in", "frames": 2}]}}
    text = cr.summary(rep)
    assert "pan steady_right: 240 frames, fps 60.0, over 16.7 ms 130, missed 1, max 33 ms, JS p95 2 ms, view draws 240, GL posts 240" in text
    assert "pan scroll:" in text and "missed 0" in text and '"step": "in"' in text
    assert "agreement_2d" not in text and "playback_app" not in text


def test_summary_prints_the_frame_scheduler_counters():
    """P3-14d: the layout reads inside the frames and the GL draws of the 10 hit edits."""
    cr = load("check_report")
    rep = report(True)
    rep["metrics"].update(frames_layout_reads={"n": 0, "where": []}, edit_gl_draws=0, edit_frames={"tracks": 10, "view": 0})
    rep["metrics"]["render"] = {"lanes": 30, "gl": {"renderer": "Apple GPU"}, "warm_ms": 5, "frames_zoom": {"n": 1}, "agreement_2d": [{"min": 1}],
                                "layout_reads_in_frames": {"n": 0, "where": []}, "frames_drawn": {"view": 120}, "frames_idle": {"n": 120, "p99_ms": 17}}
    for k in ("playback_app", "playback_every", "playback_all_app", "playback_all_every"):
        rep["metrics"]["render"][k] = {"fps_median": 60, "fps_mean": 60, "js_p50_ms": 1, "js_p95_ms": 2, "js_max_ms": 3, "lane_redraws": 10, "frames": 100, "frame": {"over_20ms": 0}}
    text = cr.summary(rep)
    assert 'frames_layout_reads = {"n": 0, "where": []}' in text and "edit_gl_draws = 0" in text and '"tracks": 10' in text
    assert 'render layout reads in the frames = {"n": 0, "where": []}, frames drawn = {"view": 120}' in text
    assert 'render frames_idle (no change) = {"n": 120, "p99_ms": 17}' in text


def test_summary_prints_the_memory_numbers():
    cr = load("check_report")
    rep = report(True)
    row = {"t_s": 12.5, "tag": "play_5s", "web_content_mb": 210.5, "gpu_mb": 300.0, "textures": {"peaks": 90, "samples": 0}}
    rep["metrics"]["memory"] = {"engine": {"remote": True, "renderer": "Apple GPU"}, "process_count": 1, "web_content_peak_mb": 210.5,
                                "web_content_first_mb": 150.0, "web_content_last_mb": 200.0, "web_content_rss_peak_mb": 250.0, "gpu_peak_mb": 300.0,
                                "page_peak_numbers_max": 2640, "open_to_drawn_ms": [800, 700, 750], "open_to_drawn_median_ms": 750,
                                "open_to_drawn_max_ms": 800, "samples": [row], "target": {"web_content_mb": 400, "met": False, "met_at_rest": True}, "play": {"underruns": 0},
                                "zoom_sweeps_before_play": [{"p99_ms": 22}], "zoom_sweeps_after_play": [{"p99_ms": 23, "over_30ms": 1}]}
    text = cr.summary(rep)
    assert "engine worker (Apple GPU), 1 new WebContent" in text and "footprint peak 210.5 MB" in text and "NOT MET" in text
    assert "median 750" in text and "play_5s" in text and "zoom_sweeps_after_play[0]" in text and "frames over 30 ms 1" in text
    assert cr.memory_lines(None) == []


def report(ok=True):
    return {"scenario": "slice", "pass": ok, "errors": [] if ok else ["boom"],
            "steps": [{"step": "open", "ok": ok, "ms": 12.5, **({} if ok else {"error": "x"})}],
            "metrics": {"open_ms": 12.5, "frames_scroll": {"n": 3}}, "tracks": {}}


def test_summary_lists_steps_metrics_and_errors():
    cr = load("check_report")
    text = cr.summary(report(False))
    assert "FAIL" in text and "open" in text and "open_ms = 12.5" in text and "frames_scroll" in text and "error: boom" in text
    assert "PASS" in cr.summary(report(True))


def test_main_exit_status_follows_pass_and_the_midi_check(tmp_path, capsys):
    cr = load("check_report")
    rp = tmp_path / "r.json"
    rp.write_text(json.dumps(report(False)))
    assert cr.main([str(rp)]) == 1
    rp.write_text(json.dumps(report(True)))
    assert cr.main([str(rp)]) == 0
    events = [[0.5, 100], [1.0, 80]]
    (tmp_path / "Exports").mkdir()
    (tmp_path / "Exports" / "k_spike.mid").write_bytes(cr.midi_reference("unit", "k", [{"t": a, "v": b} for a, b in events], 36)[0])
    rep = report(True)
    rep["tracks"] = {"k": {"events": events, "note": 36, "midi": {"path": "Exports/k_spike.mid"}}}
    rp.write_text(json.dumps(rep))
    assert cr.main([str(rp), "--project", str(tmp_path)]) == 0
    assert "1 / 1" in capsys.readouterr().out
    rep["tracks"]["k"]["events"] = [[0.5, 99], [1.0, 80]]              # a changed velocity must be caught
    rp.write_text(json.dumps(rep))
    assert cr.main([str(rp), "--project", str(tmp_path)]) == 1
    rep["tracks"]["k"]["midi"]["path"] = "Exports/missing.mid"
    rp.write_text(json.dumps(rep))
    assert cr.main([str(rp), "--project", str(tmp_path)]) == 1
    del rep["tracks"]["k"]["events"]
    rp.write_text(json.dumps(rep))
    assert cr.main([str(rp), "--project", str(tmp_path)]) == 1


def test_synthetic_session_becomes_a_project_with_events(tmp_path):
    """P3-5b: the 30-track session of make_session.py (here 3 short tracks) is built into a project with the known hits."""
    mp = load("make_project")
    sess = tmp_path / "sess"
    sess.mkdir()
    tracks = []
    for k in range(3):
        x, _ = synth_track([0.25, 0.75], 1.5, sr=48000)
        write_wav(sess / f"t{k + 1:02d}.wav", x, sr=48000)
        tracks.append({"file": f"t{k + 1:02d}.wav", "hit_samples": [12000, 36000], "hit_times_s": [0.25, 0.75]})
    (sess / "session.json").write_text(json.dumps({"sr": 48000, "seconds": 1.5, "tracks": tracks}))
    out = tmp_path / "proj"
    out.mkdir()
    mp.main(["--out", str(out), "--name", "Synth", "--synthetic", str(sess)])
    doc = json.loads((out / "Synth" / "Synth.drumspike").read_text())
    assert list(doc["tracks"]) == ["t01", "t02", "t03"]
    assert [doc["tracks"][n]["role"] for n in doc["tracks"]] == ["kick", "snare", "tom"]
    ev = doc["tracks"]["t02"]["events"]
    assert [e["t"] for e in ev] == [0.25, 0.75] and all(e["v"] == 100 and "id" in e for e in ev)
    assert doc["tracks"]["t01"]["sr"] == 48000
    with pytest.raises(SystemExit):
        mp.main(["--out", str(out), "--name", "None"])      # neither --from nor --synthetic


def test_play_numbers_are_printed_by_the_checker():
    cr = load("check_report")
    play = {"seconds": 60, "callbacks": 11250, "underruns": 0, "underrun_frames": 0, "stream_errors": 0, "reopens": 0,
            "pre_gain_peak": 3.84, "master_after": 0,
            "playhead_error_ms": {"n": 200, "median": 1.2, "p95": 3.4, "max": 6.0, "status_rtt_median": 0.9, "status_rtt_max": 3.0},
            "mute": {"n": 8, "latency_ms_median": 3.1, "latency_ms_max": 5.2, "rtt_ms_max": 4.4, "total_ms_max": 9.0},
            "seek_settle_ms": [120.5]}
    report = {"scenario": "play", "pass": True, "steps": [], "errors": [], "metrics": {"play": play, "play_start_ms": 180.2}}
    text = cr.summary(report)
    assert "underruns 0 (0 frames)" in text and "median 1.2 ms, p95 3.4 ms" in text and "max 5.2 ms" in text
    assert "play_start_ms = 180.2" in text and "[120.5]" in text
    play["playhead_error_ms"] = {"n": 0, "note": "no animation frames (window hidden?)"}
    assert "not measured: no animation frames" in cr.summary(report)
    assert cr.play_lines(None) == []
