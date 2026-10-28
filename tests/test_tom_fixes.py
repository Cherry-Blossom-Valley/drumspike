"""P4-20: the tom fixes diagnosis (`tools/tom_fixes.py`). Synthetic signals and events only; no session file and no HOME
content needed (run with HOME pointed at an empty folder)."""
import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import tom_fixes as tf  # noqa: E402
import onset_guard as og  # noqa: E402

SR = 48000


def tone_tail(n_s, start, f=100.0, amp=0.5, decay_s=0.3):
    """A low, slowly decaying tone from `start` (the ring of an earlier tom hit)."""
    x = np.zeros(int(n_s * SR))
    i = int(start * SR)
    t = np.arange(len(x) - i) / SR
    x[i:] = amp * np.exp(-t / decay_s) * np.sin(2 * np.pi * f * t)
    return x


def click(x, t, amp=0.03, f=4000.0, dur=0.004):
    """Add a short high burst (the stick click of a new hit) at `t`."""
    i = int(t * SR)
    n = int(dur * SR)
    tt = np.arange(n) / SR
    x[i:i + n] += amp * np.exp(-tt / 0.001) * np.sin(2 * np.pi * f * tt)
    return x


def noise_burst(n_s, hits, level=0.5, floor=1e-4, seed=1):
    rng = np.random.default_rng(seed)
    x = rng.normal(0, floor, int(n_s * SR))
    for t in hits:
        i = int(t * SR)
        n = int(0.02 * SR)
        x[i:i + n] += level * np.exp(-np.arange(n) / (0.004 * SR)) * rng.choice([-1, 1], n)
    return x


# ---------------------------------------------------------------- the stroke test

def test_stroke_rise_sees_a_click_over_a_ring_that_the_full_band_misses():
    x = click(tone_tail(1.5, 0.5), 0.7)
    y = tf.highpass(x, SR)
    full, hp, best = tf.stroke_rise(x, y, SR, 0.7)
    assert full < tf.RISE_MIN_DB          # the ring of the earlier hit hides the rise on the full band
    assert hp >= 20.0                      # the 2 kHz band shows the click
    assert best == max(full, hp) and best >= tf.RISE_MIN_DB


def test_stroke_rise_stays_low_on_the_ring_alone():
    x = tone_tail(1.5, 0.5)
    y = tf.highpass(x, SR)
    assert tf.stroke_rise(x, y, SR, 0.7)[2] < tf.RISE_MIN_DB


def test_control_pass_rate_is_zero_on_noise_and_digital_silence():
    rng = np.random.default_rng(3)
    x = rng.normal(0, 1e-3, 10 * SR)
    hit, tot = tf.control_pass_rate(x, tf.highpass(x, SR), SR, [], n=100)
    assert tot == 100 and hit <= 3
    z = np.zeros(10 * SR)
    assert tf.control_pass_rate(z, z, SR, [], n=50)[0] == 0


# ---------------------------------------------------------------- the track state and the flags

def test_track_state_leaves_flagged_events_out_of_the_output():
    ref = [1.0, 2.0, 3.0]
    ev = [{"t": 1.002}, {"t": 2.5, "bleed": True}, {"t": 3.001}, {"t": 4.0}, {"t": 5.0, "cut": True}]
    st = tf.track_state(ref, ev)
    assert st["est"] == [1.002, 3.001, 4.0]
    assert st["flagged"] == [(2.5, "bleed"), (5.0, "cut")]
    assert st["missed"] == [1] and st["false"] == [2]        # 4.0 is false; the reference at 2.0 is missed


def test_flag_near_takes_the_nearest_within_20_ms():
    fl = [(1.000, "bleed"), (1.015, "cut"), (2.0, "cut")]
    assert tf.flag_near(fl, 1.012) == "cut"
    assert tf.flag_near(fl, 1.005) == "bleed"
    assert tf.flag_near(fl, 1.040) is None
    assert tf.flag_near(fl, 2.0199) == "cut" and tf.flag_near(fl, 2.0205) is None


# ---------------------------------------------------------------- question 1

def test_miss_class_priority():
    f = tf.miss_class
    assert f(20, None, 9.0, False, False, "threshold", silent=True) == "silent"
    assert f(3, "bleed", 9.0, False, False, "threshold") == "no_stroke"      # the audio contradicts the reference first
    assert f(20, "bleed", 9.0, False, False, "threshold") == "bleed_rule"
    assert f(20, "cut", 9.0, False, False, "threshold") == "loudness_cut"
    assert f(20, None, 0.035, False, False, "threshold") == "outside_20ms"
    assert f(20, None, 0.020, False, False, "threshold") == "threshold"       # inside the match window: not "outside"
    assert f(20, None, 9.0, True, False, "threshold") == "other_tom"
    assert f(20, None, 9.0, False, True, "threshold") == "merged"
    assert f(20, None, 9.0, False, False, "average") == "average"
    assert f(20, None, 9.0, False, False, "mingap") == "other_stage"
    assert f(20, None, 0.035, True, True, "blind") == "outside_20ms"         # the window class before the neighbours


def test_stage_causes_reads_the_activation_of_a_missed_hit():
    act = np.zeros((600, 5), np.float32)
    act[100:103, 2] = [0.1, 0.2, 0.1]            # a weak tom lobe at 1.0 s: under 0.32
    act[300:303, 2] = [0.2, 0.9, 0.2]            # a strong one at 3.0 s: found
    st = {"est": [3.02], "shifted": [1.02, 3.0], "pairs": [(1, 0)], "missed": [0], "false": [], "flagged": [], "off": 0.0}
    cause, peak = tf.stage_causes(st, act)[0]
    assert cause == "threshold" and peak == pytest.approx(0.2, abs=1e-6)
    act2 = np.zeros((600, 5), np.float32)
    act2[300:303, 2] = [0.2, 0.9, 0.2]
    assert tf.stage_causes(st, act2)[0][0] == "blind"


# ---------------------------------------------------------------- question 2

def test_false_class_priority_and_windows():
    def f(**k):
        a = dict(no_ref_mic=False, other_tom_s=9.0, snare_s=9.0, kick_s=9.0, after_s=9.0, sample_s=9.0,
                 snare_bleed=False, kick_bleed=False, rise_own=20.0, rise_cym=None, decay=False)
        a.update(k)
        return tf.false_class(**a)
    assert f() == "real_stroke"
    assert f(no_ref_mic=True, other_tom_s=0.0) == "no_reference"
    assert f(other_tom_s=0.012) == "other_tom"                # 12 ms is inside
    assert f(other_tom_s=0.013) == "real_stroke"              # 13 ms is outside
    assert f(snare_s=-0.005, other_tom_s=0.010) == "snare_ref"  # the nearest names the source
    assert f(kick_s=0.002) == "kick_ref"
    assert f(after_s=0.012) == "double" and f(after_s=0.060) == "double" and f(after_s=0.061) == "real_stroke"
    assert f(after_s=0.011) == "real_stroke"
    assert f(sample_s=0.006) == "sample_track" and f(sample_s=0.0061) == "real_stroke"
    assert f(snare_bleed=True) == "snare_audio" and f(kick_bleed=True) == "kick_audio"
    assert f(rise_own=3.0, rise_cym=20.0) == "cymbal"
    assert f(rise_own=26.0, rise_full=6.0, rise_cym=20.0) == "cymbal"      # only the highs rise: a cymbal or hat stroke
    assert f(rise_own=26.0, rise_full=16.0, rise_cym=20.0) == "real_stroke"  # the tom's own body rises too
    assert f(rise_own=26.0, rise_full=6.0, rise_cym=10.0) == "real_stroke"   # no cymbal stroke on its mic
    assert f(rise_own=3.0, decay=True) == "ring" and f(rise_own=3.0) == "weak"
    assert f(other_tom_s=0.0, after_s=0.03) == "other_tom"    # a source before the double test


def test_dominance_separates_a_leak_from_a_source():
    # a snare ghost note leaking into a tom mic: the snare mic is near its typical hit, the tom mic far under its own
    assert tf.is_source_bleed(25.0, tf.dominance_db(-3.0, -15.0))
    # a tom stroke leaking into the snare mic: the snare mic is far under its typical hit, the tom mic at its own
    assert not tf.is_source_bleed(25.0, tf.dominance_db(-25.0, -2.0))
    assert not tf.is_source_bleed(5.0, tf.dominance_db(0.0, -20.0))          # no stroke on the source mic
    assert not tf.is_source_bleed(None, 20.0)


def test_in_decay_and_after_matched():
    assert tf.in_decay(1.5, [1.0]) and not tf.in_decay(1.03, [1.0]) and not tf.in_decay(2.5, [1.0])
    assert tf.after_matched_s(1.03, [0.5, 1.0, 1.2]) == pytest.approx(0.03)
    assert tf.after_matched_s(0.1, [0.5]) == 9.0


# ---------------------------------------------------------------- question 3

def test_size_bin_boundaries_and_sign_free():
    assert tf.size_bin(1.2) == "1 to 2" and tf.size_bin(-2.0) == "1 to 2"
    assert tf.size_bin(2.01) == "2 to 5" and tf.size_bin(-5.0) == "2 to 5"
    assert tf.size_bin(5.01) == "over 5" and tf.size_bin(-9.0) == "over 5"


def test_moved_class_by_sign_floor_and_gap():
    f = tf.moved_class
    assert f(0.004, -30.0, -40.0, 0.0) == "app_late"           # rises without a gap from the reference time
    assert f(0.004, -30.0, -40.0, 12.0) == "ref_precursor"      # a dip between the two times
    assert f(0.004, -37.0, -40.0, 0.0) == "ref_early"           # under the floor plus 6 dB at the reference time
    assert f(0.004, -34.0, -40.0, 0.0) == "app_late"            # exactly 6 dB over the floor moves
    assert f(-0.004, -30.0, -40.0, 0.0) == "ref_late"
    assert f(-0.004, -30.0, -40.0, 12.0) == "app_early"
    assert f(-0.004, -39.0, -40.0, 0.0) == "app_early"


def test_envelope_dip_separates_a_gap_from_a_slope():
    x = np.zeros(int(0.1 * SR))
    x[int(0.010 * SR):int(0.011 * SR)] = 0.1        # an early blip
    x[int(0.020 * SR):int(0.030 * SR)] = np.linspace(0.001, 0.5, int(0.010 * SR))
    gap = tf.max_dip_db(tf.envelope_blocks(x, SR, 0.010, 0.025))
    assert gap > 30.0                                # the blip, then silence, then the attack
    ramp = np.zeros(int(0.1 * SR))
    ramp[int(0.020 * SR):int(0.030 * SR)] = np.linspace(0.001, 0.5, int(0.010 * SR))
    assert tf.max_dip_db(tf.envelope_blocks(ramp, SR, 0.0205, 0.0295)) < 3.0
    assert tf.max_dip_db([-30.0, -20.0, -10.0]) == 0.0 and tf.max_dip_db([-10.0, -25.0, -20.0]) == 15.0


def test_shift_score_counts_the_hits_it_breaks():
    errs = [0.0, 0.0005, 0.002, -0.0015]                  # right, right, late 2 ms, early 1.5 ms
    assert tf.shift_score(errs, 0.0) == (2, 0, 0)
    after, wrong, right = tf.shift_score(errs, -0.002)    # fixes the 2 ms hit, breaks the two exact ones, worsens one
    assert (after, wrong, right) == (3, 2, 1)
    assert tf.shift_score([0.001, -0.001], 0.0) == (0, 0, 0)  # exactly 1 ms is right


def test_shift_fit_prefers_the_smaller_shift_on_a_tie():
    errs = {"a": [0.0, 0.0, 0.0], "b": [0.0, 0.0]}
    assert tf.shift_fit(errs, ["a", "b"], [-1.0, 0.0, 1.0]) == 0.0
    late = {"a": [0.003] * 5}
    assert tf.shift_fit(late, ["a"], [-4.0, -3.0, -2.0, 0.0]) in (-3.0, -2.0)   # -2 ms leaves 1 ms: right, -3 is exact
    assert tf.shift_fit(late, ["a"], [0.0, -3.0, -2.0]) == -2.0                 # the smaller shift wins the tie


def test_rewalk_goes_further_back_with_a_lower_floor_and_restores_the_constants():
    x = np.zeros(int(0.3 * SR))
    a0 = int(0.100 * SR)
    ramp = np.linspace(0.0, 0.5, int(0.010 * SR))
    x[a0:a0 + len(ramp)] = ramp
    x[a0 + len(ramp):a0 + len(ramp) + 200] = 0.5
    t = 0.1025                                         # an event 2.5 ms into the ramp
    keep = (og.FM_PEAK_DB, og.FM_SPAN_S, og.FM_FLOOR_DB)
    ship = tf.rewalk(x, SR, t, None, None)
    deep = tf.rewalk(x, SR, t, None, None, peak_db=60.0, floor_margin_db=0.0)
    assert deep < ship <= t and (ship - deep) * SR >= 5
    assert (og.FM_PEAK_DB, og.FM_SPAN_S, og.FM_FLOOR_DB) == keep          # the module constants are back
    assert tf.rewalk(x, SR, ship, None, None) == pytest.approx(ship, abs=2.0 / SR)   # the shipped walk is idempotent


def test_analyze_matched_error_sign_is_event_minus_reference():
    x = noise_burst(2.0, [0.5, 1.0])
    st = {"est": [0.503, 1.0], "shifted": [0.5, 1.0], "pairs": [(0, 0), (1, 1)]}
    rows = tf.analyze_matched(st, [{"v": 100, "rise": 20.0}, {"v": 90, "rise": 21.0}], x, tf.highpass(x, SR), SR, -10.0)
    assert rows[0]["err"] == pytest.approx(0.003) and rows[1]["err"] == pytest.approx(0.0)
    assert rows[0]["gap_prev_ms"] == 9e3 and rows[1]["gap_prev_ms"] == pytest.approx(497.0)
    assert rows[0]["v"] == 100 and rows[0]["rise_own"] >= 10.0


# ---------------------------------------------------------------- question 4

def test_cut_eval_counts_false_removed_and_matched_lost():
    rows = [{"status": "false", "rel_db": -20.0}, {"status": "false", "rel_db": -5.0},
            {"status": "matched", "rel_db": -16.0}, {"status": "matched", "rel_db": -2.0}]
    assert tf.cut_eval(rows, -15.0) == (1, 1)
    assert tf.cut_eval(rows, -20.0) == (1, 0)             # the cut is inclusive
    assert tf.cut_eval(rows, -25.0) == (0, 0)


def test_wta_drops_the_lower_of_two_mics_within_the_window():
    ev = [{"mic": "t1", "t": 1.000, "rel_db": -12.0, "status": "false"},
          {"mic": "t2", "t": 1.004, "rel_db": 0.0, "status": "matched"},
          {"mic": "t1", "t": 2.000, "rel_db": -3.0, "status": "matched"},
          {"mic": "t2", "t": 2.100, "rel_db": 0.0, "status": "matched"},      # 100 ms apart: two hits
          {"mic": "t1", "t": 3.000, "rel_db": -1.0, "status": "matched"},
          {"mic": "t1", "t": 3.005, "rel_db": -20.0, "status": "false"}]      # the same mic: never dropped
    assert tf.wta_drop(ev) == {0}
    assert tf.wta_eval(ev) == (1, 0)
    assert tf.wta_drop(ev, margin_db=13.0) == set()                           # the 12 dB gap is under the margin
    assert tf.wta_drop(ev, window_s=0.101, margin_db=0.0) == {0, 2}         # 101 ms window: the t1 hit at 2.0 goes
    chain = [{"mic": "a", "t": 1.0, "rel_db": -9.0, "status": "false"}, {"mic": "b", "t": 1.005, "rel_db": 0.0, "status": "matched"},
             {"mic": "c", "t": 1.010, "rel_db": -5.0, "status": "false"}]
    assert tf.wta_drop(chain) == {0, 2}                                      # the loudest of a chain stays


def test_variant_events_keep_old_event_times_and_lead_new_picks():
    base_p, base_e = [1.0, 2.0, 3.0], [1.010, 2.012, 3.008]                   # lead 10 ms median
    out = tf.variant_events(base_p, base_e, [1.0, 1.5, 3.0], tol=1e-6)
    assert out == [1.010, 1.510, 3.008]
    assert tf.variant_events([], [], [1.0]) == [pytest.approx(1.0 + tf.mc.LEAD_S)]


def test_pick_variant_changes_the_tom_threshold_and_the_average():
    act = np.zeros((400, 5), np.float32)
    act[200:203, 2] = [0.1, 0.25, 0.1]                                       # under 0.32, over 0.20
    act[100:103, 2] = [0.2, 0.9, 0.2]
    assert len(tf.pick_variant(act)) == 1
    assert len(tf.pick_variant(act, thr=0.20)) == 2
    assert len(tf.pick_variant(act, thr=0.40)) == 1 and len(tf.pick_variant(act, thr=0.99)) == 0


# ---------------------------------------------------------------- the report

def test_class_table_counts_per_song_and_in_total():
    rows = [{"song": "Session A", "class": "other_tom"}, {"song": "Session A", "class": "kick_ref"},
            {"song": "Session H", "class": "other_tom"}]
    t = tf.class_table(rows, tf.FALSE_CLASSES)
    lines = t.strip().splitlines()
    assert lines[0] == "| song | other_tom | kick_ref | all |"
    assert "| Session A | 1 | 1 | 2 |" in lines and "| Session H | 1 | 0 | 1 |" in lines
    assert lines[-1] == "| **all** | **2** | **1** | **3** |"


def test_modules_read_no_home_content_at_import():
    # the tool's paths are constants; nothing is read until a command runs
    assert tf.SRC.name == "fix-metric-p419" and tf.OUT.name == "tom-fixes"
    assert tf.FIT and not set(tf.FIT) & set(tf.HELD) and set(tf.FIT) | set(tf.HELD) == set(tf.SONGS)
