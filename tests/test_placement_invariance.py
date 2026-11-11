"""#443: tools/placement_invariance.py measures the rule's invariance on generated strokes, no internal data.

The planted defect of the Gate: a placer that reads |x| from zero (no line subtraction) must fail
the DC-offset line. The delay line: a hit must move by exactly the delay; an analyzer that quantizes
positions shows as 1 off. A line with 0 hits fails the report."""
import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))
import placement_invariance as inv  # noqa: E402

SR = 48000


def slow_rise_hit(at=200):
    """A 0.2 s excerpt: a 0.004 line, then a slow rise (0.004 per sample) to the stroke peak 0.4
    after `at` - low enough that a +6 dB gain does not clip. A slow rise makes a 1 % DC offset
    move a zero-level crossing by a whole sample."""
    x = np.full(9600, 0.004, dtype=np.float32)
    x[at:at + 100] = np.float32(0.004) + np.linspace(0, 0.2, 100, dtype=np.float32)
    x[at + 100:at + 300] = np.linspace(0.204, 0.4, 200, dtype=np.float32)
    x[at + 300:] = np.linspace(0.4, 0.12, len(x) - at - 300, dtype=np.float32)
    return {"role": "kick", "x": x, "at": at, "sr": SR}


def hits(n=4):
    return [slow_rise_hit() for _ in range(n)]


def placer_from_offset(offset_of):
    """A fake placer: records are (x, sr, t0); the policy places from the audio alone and returns
    the refined position in buffer samples (like parity's t * sr)."""

    def place(records):
        return np.array([float(offset_of(rec[0], round(rec[2] * rec[1]))) for rec in records])

    return place


def raw_crossing(x, at=None):
    """The DEFECT: the first sample at half the peak's |x|, read from zero - the DC rides along."""
    del at
    pk = np.abs(x[96:]).max()
    return 96 + int(np.argmax(np.abs(x[96:]) >= 0.5 * pk))


def line_crossing(x, at=None):
    """The correct policy: the line (the median of the excerpt's head) is the zero of the signal."""
    del at
    line = np.median(x[:96])
    pk = np.abs((x - line)[96:]).max()
    return 96 + int(np.argmax(np.abs((x - line)[96:]) >= 0.5 * pk))


def test_a_placer_that_reads_from_zero_fails_the_dc_line():
    """The DC offset of 1 % of the peak moves the zero-level crossing by a sample; a line-subtracting
    placer does not move."""
    hs = hits()
    rng = np.random.default_rng(7)
    res = inv.measure_window_lines(hs, placer_from_offset(raw_crossing), rng)
    dc = res["lines"]["DC offset 1% of peak"]
    assert dc["kick"]["n"] == len(hs) and dc["kick"]["moved1"] == 100.0
    assert dc["kick"]["moved5"] == 0.0
    good = inv.measure_window_lines(hs, placer_from_offset(line_crossing), rng)
    assert good["lines"]["DC offset 1% of peak"]["kick"]["moved1"] == 0.0


def test_gain_and_polarity_leave_the_answer_alone():
    hs = hits()
    res = inv.measure_window_lines(hs, placer_from_offset(line_crossing), np.random.default_rng(7))
    for line in ("gain -12 dB", "gain +6 dB", "polarity flip"):
        assert res["lines"][line]["kick"]["moved1"] == 0.0, line


def test_the_coarse_time_line_moves_the_coarse_time_not_the_answer():
    """The coarse-time records keep the audio and shift t0; the deltas still come out per role."""
    hs = hits()
    res = inv.measure_window_lines(hs, placer_from_offset(line_crossing), np.random.default_rng(7))
    for ms in (-3, -1, 1, 3):
        m = res["lines"][f"coarse time {ms:+d} ms"]["kick"]
        assert m["n"] == len(hs) and m["moved1"] == 0.0


def test_shares_count_half_sample_up_and_the_delay_line_counts_exact_moves():
    d = np.array([0.4, 0.6, 1.6, 4.6])
    roles = np.array(["kick"] * 4)
    s = inv.shares(d, roles)
    assert (s["kick"]["moved1"], s["kick"]["moved2"], s["kick"]["moved5"]) == (75.0, 50.0, 25.0)

    # the delay line: a shift-invariant analyzer puts every hit exactly `delay` later
    def analyzer(records):
        return [np.flatnonzero(np.abs(x) > 0.5).astype(float) for x, _, _ in records]

    ex = [{"role": "kick", "x": np.concatenate([np.zeros(inv.HEAD_GUARD), _impulse_track()]),
           "sr": 1000, "len": 2000}]
    res = inv.measure_delay_line(ex, analyzer, [1, 7])
    assert res["delay 1 sample"]["kick"]["exact"] == 100.0
    assert res["delay 7 samples"]["kick"]["exact"] == 100.0

    # the planted defect: an analyzer that quantizes to even samples shows 1 off, never exact
    def quantizer(records):
        return [np.floor(np.flatnonzero(np.abs(x) > 0.5) / 2) * 2 for x, _, _ in records]

    res = inv.measure_delay_line(ex, quantizer, [1])
    assert res["delay 1 sample"]["kick"]["exact"] == 0.0 and res["delay 1 sample"]["kick"]["off1"] == 100.0


def _impulse_track():
    """2000 samples with stroke peaks every 100 samples; the ramps stay under the 0.5 gate."""
    x = np.zeros(2000, dtype=np.float32)
    for p in range(100, 2000, 100):
        x[p] = 1.0
        x[p + 1:p + 6] = np.linspace(0.4, 0.1, 5, dtype=np.float32)
    return x


def test_bounds_and_a_line_with_0_hits_fail(tmp_path, capsys):
    window = {"DC offset 1% of peak": {"kick": {"n": 5, "moved1": 100.0, "moved2": 0.0, "moved5": 0.0}}}
    bounds = {"window": {"DC offset 1% of peak": {"kick": {"moved1": 0.0}}}}
    assert inv.over_bounds(window, {}, bounds) == [("DC offset 1% of peak kick", "moved1", 100.0, 0.0)]
    assert inv.over_bounds(window, {}, {"window": {}}) == []
    # 'exact' is a floor: a run under it is over the bound, a run above it is fine
    delay = {"delay 1 sample": {"kick": {"n": 3, "exact": 90.0, "off1": 0.0, "off2": 0.0, "off5": 0.0, "lost": 0.0}}}
    assert inv.over_bounds(window, delay, {"delay": {"delay 1 sample": {"kick": {"exact": 98.0}}}}) == [("delay 1 sample kick", "exact", 90.0, 98.0)]
    assert inv.over_bounds(window, delay, {"delay": {"delay 1 sample": {"kick": {"exact": 90.0}}}}) == []
    assert inv.print_report({}, {}, out=sys.stderr) == 1
    assert inv.print_report(window, {"delay 1 sample": {"kick": {"n": 3, "exact": 100.0, "off1": 0.0,
                                                                 "off2": 0.0, "off5": 0.0, "lost": 0.0}}},
                            out=sys.stderr) == 0


def test_load_hits_reads_the_layout_from_the_arguments(tmp_path):
    """A tiny measure dir + samples root in the argument layout: one kick track, hits cut around t."""
    import soundfile as sf
    sr = 48000
    x = np.full(sr, 0.01, dtype=np.float32)
    x[sr // 2:sr // 2 + 100] = np.linspace(0.01, 1.0, 100, dtype=np.float32)
    folder = tmp_path / "Session A" / "Audio Files"
    folder.mkdir(parents=True)
    sf.write(folder / "01 Kick.wav", x, sr, subtype="FLOAT")
    before = tmp_path / "Session A_before.json"
    before.write_text(json.dumps({"01_Kick": {"role": "kick",
                                              "events": [{"t": 0.5 + i * 0.001} for i in range(6)]}}))
    hs, _ = inv.load_hits(tmp_path, tmp_path, per=40, seed=7)
    assert len(hs) == 6 and all(h["role"] == "kick" for h in hs)
    assert all(h["at"] == int(inv.PRE * SR) for h in hs)
    # a track of another role and short tracks are skipped loudly
    before.write_text(json.dumps({"01_Kick": {"role": "hat", "events": [{"t": 0.5}] * 6},
                                  "02_X": {"role": "kick", "events": [{"t": 0.5}] * 3}}))
    hs2, _ = inv.load_hits(tmp_path, tmp_path, per=40, seed=7)
    assert hs2 == []


def test_the_delay_line_names_the_rule_in_its_job(tmp_path):
    """#444: `service_tool core-analyze` runs rule version 1 when a job names no rule, so the delay
    line must name the rule it measures. A stand-in tool records the job it was given."""
    seen = tmp_path / "seen.json"
    tool = tmp_path / "service_tool"
    tool.write_text(
        "#!/bin/sh\n"
        f"cp \"$2\" '{seen}'\n"
        "echo '{\"events\": [{\"t\": 0.5}]}'\n"
    )
    tool.chmod(0o755)
    x = np.zeros(SR, dtype=np.float32)
    out = inv.analyze_with_service_tool([(x, SR, "kick")], tool, tmp_path, 10)
    assert [round(float(v)) for v in out[0]] == [SR // 2]
    job = json.loads(seen.read_text())
    assert job[0]["rule"] == 10 and job[0]["role"] == "kick", job
    inv.analyze_with_service_tool([(x, SR, "snare")], tool, tmp_path)
    assert json.loads(seen.read_text())[0]["rule"] == inv.RULE == 10
