"""P4-15 stage 1: the onset guard tool (`tools/onset_guard.py`) on planted signals: the version 3 walk port, the zero
phase bands, the P2-6 gain calibration, candidate A (cross-mic lead stop) and B (band walk), the extension rule and
the counts. The real sessions and the kit outputs are not needed."""
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import onset_guard as og  # noqa: E402

SR = 48000


def hat(rng, n, amp, decay=0.004):
    """A hat-like burst: high-band noise with a fast decay, peak `amp`."""
    y = og.band(rng.standard_normal(n + 2000), SR, "high")[1000:1000 + n]
    y *= np.exp(-np.arange(n) / (decay * SR))
    return amp * y / np.abs(y[:200]).max()


def snare(rng, n, amp):
    """A snare-like hit: a 200 Hz body plus wire noise under 2.5 kHz, sharp start, peak `amp`."""
    t = np.arange(n) / SR
    y = np.sin(2 * np.pi * 200 * t) * np.exp(-t / 0.08) + 0.5 * og.lowpass(rng.standard_normal(n), SR, 2500.0) * np.exp(-t / 0.05)
    return amp * y / np.abs(y).max()


def session(lead="hat", seed=1):
    """Own mic A and hat mic B, 3 s. Hat bursts every 100 ms (B at 0.2, A hears them 12 dB down, 0.5 ms later) until
    0.9 s for the calibration; a snare hit at exactly 1.0 s. lead="hat": one more hat burst 2 ms before the snare;
    lead="own": the snare's own quiet start 2 ms before it (200 Hz, 18 dB down), no hat burst near it."""
    rng = np.random.default_rng(seed)
    n = 3 * SR
    a, b = np.zeros(n), np.zeros(n)
    lag = int(0.0005 * SR)
    for k in range(1, 9):
        at = int(k * 0.1 * SR)
        h = hat(rng, int(0.03 * SR), 0.2)
        b[at:at + len(h)] += h
        a[at + lag:at + lag + len(h)] += 0.25 * h
    on = SR
    s = snare(rng, int(0.2 * SR), 0.5)
    a[on:on + len(s)] += s
    if lead == "hat":
        at = on - int(0.002 * SR) - lag
        h = hat(rng, int(0.03 * SR), 0.2)
        b[at:at + len(h)] += h
        a[at + lag:at + lag + len(h)] += 0.25 * h
    else:
        m = int(0.002 * SR)
        a[on - m:on] += 0.06 * np.sin(2 * np.pi * 200 * np.arange(m) / SR + 0.35)
    a += 1e-5 * rng.standard_normal(n)
    b += 1e-5 * rng.standard_normal(n)
    return a, b, on


def track(a, b, on, t2_late_s=0.0006):
    """The Track of mic A with B as its other mic; version 2 a little after the start, version 3 from the port."""
    times = [k * 0.1 + 0.0005 for k in range(1, 9)]   # A's own events: none (the hat bursts are not A's hits)
    t2 = (on / SR) + t2_late_s
    s2 = int(round(t2 * SR))
    pm = int((0.8 + t2) / 2 * SR)
    f3, step = og.walk(a, s2, SR, pm, None)
    assert step == "moved"
    src = [(og.envelope(og.band(b, SR, "high"), SR), "hh")]
    tr = og.Track("planted", "a", "snare", [0.8, t2], [0.8, f3 / SR], x=a, sr=SR, sources=src)
    del times
    return tr, f3


def test_the_walk_port_takes_a_slow_attack_to_its_foot_and_keeps_a_sharp_start():
    x = np.zeros(SR)
    on = SR // 2
    m = int(0.003 * SR)
    x[on:on + m] = np.linspace(0, 1, m) * 0.5                   # a 3 ms ramp
    x[on + m:on + m + 2000] = 0.5 * np.exp(-np.arange(2000) / 400)
    foot, step = og.walk(x, on + m // 2, SR)
    assert step == "moved" and on <= foot <= on + 5, (foot, step)   # 33 dB under 0.5 is reached in the first samples
    y = np.zeros(SR)
    y[on:on + 2000] = 0.5 * np.exp(-np.arange(2000) / 400)       # a sharp start: the walk from it stays
    assert og.walk(y, on, SR) == (on, "kept")
    # the 2-tap mean: a Nyquist ripple before the start (pre-ringing) is not read as motion
    y[on - 20:on] = 0.02 * (-1.0) ** np.arange(20)
    assert og.walk(y, on, SR)[0] == on


def test_a_long_walk_over_a_quiet_stretch_is_a_separate_event_and_a_busy_one_keeps_s():
    x = np.zeros(SR)
    on = SR // 2
    x[on - int(0.008 * SR):on] = 0.02                            # 8 ms of a quiet event, 28 dB under the hit
    x[on:on + 2000] = 0.5
    assert og.walk(x, on, SR) == (on, "separate")
    z = np.full(SR, 0.02)                                        # bleed above the threshold to the 10 ms bound
    z[on:on + 2000] = 0.5
    z[:on - int(0.012 * SR)] = 0.0                               # 12 ms of it: the floor median stays 0
    assert og.walk(z, on, SR) == (on, "busy")


def test_zero_phase_bands_pass_their_band_and_keep_the_time():
    t = np.arange(SR) / SR
    lo, hi = np.sin(2 * np.pi * 100 * t), np.sin(2 * np.pi * 15000 * t)
    assert np.abs(og.lowpass(lo + hi, SR, 2000.0)[2000:-2000] - lo[2000:-2000]).max() < 0.01
    assert np.abs(og.band(lo + hi, SR, "high")[2000:-2000] - hi[2000:-2000]).max() < 0.05
    imp = np.zeros(4001)
    imp[2000] = 1.0
    y = og.lowpass(imp, SR, 2000.0)
    assert int(np.argmax(y)) == 2000                             # no delay
    assert np.allclose(y[1950:2000], y[2001:2051][::-1], atol=1e-9)   # symmetric: zero phase


def test_the_gain_is_the_10th_percentile_over_the_quiet_hits_else_over_all():
    env_b = np.zeros(SR)
    env_a = np.zeros(SR)
    peaks = []
    for k in range(10):
        p = 2000 + k * 4000
        env_b[p] = 1.0
        env_a[p + 10] = 10 ** ((-20 - k) / 20)                  # A hears B at -20 to -29 dB
        peaks.append(p)
    g = og.calibrate_gain(env_a, env_b, peaks, [], SR, pct=10.0)
    assert abs(g - np.percentile([-20 - k for k in range(10)], 10)) < 0.01
    own = [(2000 + k * 4000) / SR for k in range(5)]             # A plays at the first five: the quiet five are used
    g = og.calibrate_gain(env_a, env_b, peaks, own, SR, pct=10.0)
    assert abs(g - np.percentile([-25 - k for k in range(5)], 10)) < 0.01
    assert og.calibrate_gain(env_a, env_b, peaks[:4], [], SR) is None


def test_candidate_a_stops_the_walk_at_a_hat_burst_the_hat_mic_explains():
    a, b, on = session("hat")
    tr, f3 = track(a, b, on)
    assert f3 < on - int(0.0015 * SR), (f3 - on)                # version 3 joins the hat burst 2 ms early
    foot, step = tr.cand_a(1, **og.A_PARAMS)
    assert step == "stopped" and abs(foot - on) <= int(0.0003 * SR), (foot - on, step)
    g = tr.gains(og.A_PARAMS["pct"])[0]
    assert -14.0 < g < -10.0, g                                  # the planted 12 dB


def test_candidate_a_keeps_the_own_quiet_lead_without_a_burst_in_another_mic():
    a, b, on = session("own")
    tr, f3 = track(a, b, on)
    assert f3 < on - int(0.001 * SR), (f3 - on)                  # version 3 walks onto the own quiet start
    foot, _ = tr.cand_a(1, **og.A_PARAMS)
    assert foot == f3, (foot - f3)


def test_candidate_b_walks_the_low_pass_and_stays_between_versions_3_and_2():
    a, b, on = session("hat")
    tr, f3 = track(a, b, on)
    s2 = tr.samples(1)[0]
    foot, _ = tr.cand_b(1, 2000.0)
    assert f3 <= foot <= s2
    assert abs(foot - on) <= int(0.0003 * SR), (foot - on)       # the hat burst is above 5 kHz: not in the low-pass
    a2, b2, on2 = session("own")
    tr2, f32 = track(a2, b2, on2)
    assert tr2.cand_b(1, 2000.0)[0] - f32 <= int(0.0005 * SR)    # the own 200 Hz start is in the low-pass


def test_the_rule_labels_a_quiet_lead_with_a_rise_in_another_mic_as_bleed():
    a, b, on = session("hat")
    tr, f3 = track(a, b, on)
    s2 = tr.samples(1)[0]
    assert og.rule_label(tr.env, tr.sources(), s2, f3, SR) == "bleed"
    a2, b2, on2 = session("own")
    tr2, f32 = track(a2, b2, on2)
    assert og.rule_label(tr2.env, tr2.sources(), tr2.samples(1)[0], f32, SR) == "own"


def test_the_scores_count_bleed_on_the_own_start_and_moved_own_hits():
    rows = [{"src": "hand", "label": "bleed", "own_ms": 1.0, "v2_ms": 1.5, "A": 1.2, "B": 0.0},
            {"src": "hand", "label": "bleed", "own_ms": 2.0, "v2_ms": 2.2, "A": 2.31, "B": 2.0},
            {"src": "hand", "label": "own_quiet", "own_ms": 0.0, "v2_ms": 1.0, "A": 0.0, "B": 0.6},
            {"src": "hand", "label": "own_loud", "own_ms": 0.0, "v2_ms": 3.0, "A": 0.04, "B": 0.0},
            {"src": "rule", "label": "bleed", "own_ms": None, "v2_ms": 2.0, "A": 1.0, "B": 0.0}]
    s = og.score(rows, cands=("A", "B"))
    assert s["A"]["hand"]["bleed_on_own"] == 1 and s["B"]["hand"]["bleed_on_own"] == 1
    assert s["v2"]["hand"]["bleed_on_own"] == 1
    assert s["A"]["hand"]["own_moved"] == 0 and s["B"]["hand"]["own_moved"] == 1
    assert s["B"]["hand"]["own_quiet_moved"] == 1
    assert s["A"]["rule"]["bleed_moved"] == 1 and s["B"]["rule"]["bleed_moved"] == 0


def test_the_kit_summary_counts_main_hits_and_moves_of_sharp_attacks():
    rows = [{"kind": "syn", "cond": "clean", "shell": "snare", "group": "main", "rise_ms": 0.0, "v3": 0.0, "A": 0.0,
             "A'": 0.0, "B": 3.0},
            {"kind": "syn", "cond": "clean", "shell": "snare", "group": "main", "rise_ms": 0.0, "v3": -4.0, "A": -0.5,
             "A'": -4.0, "B": -4.0},
            {"kind": "syn", "cond": "clean", "shell": "snare", "group": "pair", "rise_ms": 0.0, "v3": 0.0, "A": 9.0,
             "A'": 9.0, "B": 9.0}]
    s = og.kit_summary(rows)
    assert s["sharp"]["B"] == {"n": 2, "outside": 2, "clean_outside": 2, "moved": 1, "away": 1}
    assert s["sharp"]["A"]["moved"] == 1 and s["sharp"]["A"]["away"] == 0
    st = s["sets"][0]
    assert st["n"] == 2 and st["A"]["closer"] == 1 and st["B"]["later"] == 1 and st["B"]["away"] == 1
