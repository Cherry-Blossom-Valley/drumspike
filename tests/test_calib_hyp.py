"""P4-11b: the hypothesis sweep (`tools/calib_hyp.py`). Every generator is tested by a measurement in its audio (the planted
condition is there), and the verdict code by planted rows (a known effect reads as `held`, no effect as `did not hold`).
One small end-to-end case runs the DSP detector (skipped without `service_tool`) and one the refinement alone (skipped
without `parity`)."""
import math
import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import calib_hyp as ch  # noqa: E402
import calib_kit as ck  # noqa: E402
from core_tools import PARITY, SERVICE_TOOL, require  # noqa: E402

SR = 48000


def cases(hid):
    return ch.GENERATORS[hid](quick=True)


def first_nonzero(y):
    return int(np.argmax(y != 0.0))


# ---------------------------------------------------------------- generators: the planted condition is in the audio

def test_the_lowpass_darkens_without_moving_the_first_motion():
    base = ck.make_shot("snare", SR, 0.5, -6.0, 1, 0.0, seed=3)["y"]
    dark = ch.lowpass(base, SR, 2000.0)
    assert first_nonzero(dark) == first_nonzero(base)
    def hf(y):
        sp = np.abs(np.fft.rfft(y)) ** 2
        f = np.fft.rfftfreq(len(y), 1 / SR)
        return sp[f > 6000].sum() / sp.sum()
    assert hf(dark) < 0.05 * hf(base)


def test_the_clipped_hit_has_a_flat_top_at_minus_6_dbfs_and_a_faster_attack():
    clean = ch.mk("kick", SR, 5, rise=5.0)["y"]
    clipped = ch.clip_shot("kick", SR, 5, 5.0, 18.0)["y"]
    assert abs(np.abs(clipped).max() - 0.5) < 1e-9
    assert np.mean(np.abs(clipped) >= 0.5 - 1e-9) > 0.01 and np.mean(np.abs(clean) >= np.abs(clean).max() - 1e-9) < 1e-3
    def rise(y):
        a = np.abs(y)
        return int(np.argmax(a >= 0.9 * a.max())) - int(np.argmax(a >= 0.1 * a.max()))
    assert rise(clipped) < 0.6 * rise(clean)


def test_the_gate_cuts_everything_before_its_opening_and_opens_at_the_level():
    y = ck.make_shot("snare", SR, 2.0, -6.0, 1, 0.0, seed=2)["y"]
    for thr in (-30.0, -12.0):
        yg, gi = ch.gate_shot(y, SR, thr)
        assert np.all(yg[:gi] == 0.0) and abs(yg[gi]) >= np.abs(y).max() * 10 ** (thr / 20) - 1e-12
        assert gi > 0 and abs(y[gi - 1]) < np.abs(y).max() * 10 ** (thr / 20)


def test_the_hat_bleed_has_the_asked_peak_and_sits_above_6_khz():
    x = ch.hat_bleed(SR * 2, SR, np.random.default_rng(1), -30.0)
    assert abs(20 * math.log10(np.abs(x).max()) + 30.0) < 0.01
    sp = np.abs(np.fft.rfft(x)) ** 2
    f = np.fft.rfftfreq(len(x), 1 / SR)
    assert sp[f > 5500].sum() / sp.sum() > 0.95


def test_the_dc_offset_and_the_rumble_are_in_the_r1_files():
    by = {c["tag"]: c for c in cases("R1") if c["shell"] == "kick"}
    assert abs(np.mean(by["dc10"]["buf"]) - 0.10) < 0.01 and abs(np.mean(by["clean"]["buf"])) < 0.01
    x = by["rumble20"]["buf"]
    amp = 2 * np.abs(np.fft.rfft(x))[int(round(20.0 * len(x) / SR))] / len(x)
    assert abs(20 * math.log10(amp) + 20.0) < 1.5            # -20 dBFS sine (leakage of the hits allowed)


def test_the_hum_has_the_asked_peak_level_and_its_fundamental():
    by = {c["tag"]: c for c in cases("R2") if c["shell"] == "snare"}
    hum = by["hum50_40"]["buf"] - by["clean"]["buf"][:len(by["hum50_40"]["buf"])]       # same hits: what is left is the hum
    assert abs(20 * math.log10(np.abs(hum).max()) + 40.0) < 0.1
    k = int(round(50.0 * len(hum) / SR))
    assert np.abs(np.fft.rfft(hum))[k - 1:k + 2].max() > 0.9 * np.abs(np.fft.rfft(hum)).max()
    assert {e["hum_hz"] for e in by["hum50_40"]["events"]} == {50.0}


def test_the_sample_rates_keep_the_rise_time_in_milliseconds():
    rises = {}
    for c in cases("R4"):
        if c["shell"] != "kick":
            continue
        e = [e for e in c["events"] if e["rise_ms"] == 2.0][0]
        a = int(e["abs_sample"])
        y = np.abs(c["buf"][a:a + int(0.02 * c["sr"])])
        rises[c["sr"]] = (int(np.argmax(y >= 0.9 * y.max())) - int(np.argmax(y >= 0.1 * y.max()))) / c["sr"] * 1000.0
    assert set(rises) == {44100, 96000} and abs(rises[44100] - rises[96000]) < 0.4 and rises[44100] > 1.0


def test_the_two_mics_are_summed_with_the_asked_delay_and_inverted_polarity():
    c = [c for c in cases("R5") if c["shell"] == "snare" and c["variant"] == "earlier_louder"][0]
    e = [e for e in c["events"] if e["delay_ms"] == 3.0][0]
    ds = int(round(0.003 * c["sr"]))
    assert e["second_sample"] - e["abs_sample"] == ds
    a = int(e["abs_sample"]) - ck.SINC_HALF                              # the shots carry the kit's lead of 32 samples
    first = ck.make_shot("snare", SR, 0.0, -6.0, 1, 0.0, seed=15000 + e["id"])["y"][:2000].copy()
    second = ck.make_shot("snare", SR, 0.0, -9.0, -1, 0.0, seed=15000 + e["id"])["y"]
    want = first.copy()
    want[ds:] += second[:2000 - ds]
    assert np.allclose(c["buf"][a:a + 2000], want, atol=1e-9)           # first mic alone, then the inverted, quieter, delayed copy
    assert np.sign(second[ck.SINC_HALF]) == -1 and any(e2["delay_ms"] == 0.0 for e2 in c["events"])


def test_the_edge_hits_sit_at_the_asked_distance_from_the_start_and_the_end():
    by = {c["tag"]: c for c in cases("R6") if c["shell"] == "kick"}
    c = by["edge25"]
    first = [e for e in c["events"] if e["group"] == "first"][0]
    last = [e for e in c["events"] if e["group"] == "last"][0]
    assert abs(first["abs_sample"] / SR * 1000 - 25.0) < 0.1
    assert abs((len(c["buf"]) - last["abs_sample"]) / SR * 1000 - 25.0) < 0.1
    assert len(c["buf"]) == 4 * SR and np.abs(c["buf"][-20:]).max() > 0          # the last hit is cut by the end of the file


def test_the_roll_has_the_asked_sixteenth_and_a_flam_the_asked_delay_and_grace_level():
    c = [c for c in cases("C1") if c["shell"] == "kick"][0]
    roll = [e for e in c["events"] if e["spacing_ms"] > 100 and e["k"] >= 1]
    prev = [e for e in c["events"] if e["spacing_ms"] == roll[0]["spacing_ms"] and e["k"] == roll[0]["k"] - 1][0]
    assert abs((roll[0]["abs_sample"] - prev["abs_sample"]) / SR * 1000 - roll[0]["spacing_ms"]) < 0.5
    f = cases("C3")[0]
    main = [e for e in f["events"] if e["group"] == "main" and e["delay_ms"] == 30.0][0]
    grace = [e for e in f["events"] if e["group"] == "grace" and e["delay_ms"] == 30.0 and e["id"] == main["id"] + 1][0]
    assert abs((main["abs_sample"] - grace["abs_sample"]) / SR * 1000 - 30.0) < 0.1
    pk = lambda e, n: np.abs(f["buf"][int(e["abs_sample"]):int(e["abs_sample"]) + n]).max()
    assert abs(20 * math.log10(pk(main, int(0.02 * SR))) - (20 * math.log10(pk(grace, int(0.02 * SR))) + 8.0)) < 2.0   # grace -14, main -6


def test_the_ghost_follows_the_loud_hit_at_the_asked_gap_and_level():
    c = cases("C4")[0]
    main = [e for e in c["events"] if e["group"] == "main" and e["gap_ms"] == 60.0 and e["ghost_db"] == -12.0][0]
    ghost = [e for e in c["events"] if e["group"] == "ghost" and e["id"] == main["id"] + 1][0]
    assert abs((ghost["abs_sample"] - main["abs_sample"]) / SR * 1000 - 60.0) < 0.1
    assert ghost["ghost_db"] == -12.0 and ghost["gap_ms"] == 60.0


def test_the_tom_ring_is_in_the_c5_files_at_the_asked_level():
    by = {c["tag"]: c for c in cases("C5")}
    ring, none = by["ring22"], by["ringnone"]
    diff = ring["buf"] - none["buf"][:len(ring["buf"])]                 # same kicks and toms: what is left is the ring
    assert abs(np.abs(diff).max() - 10 ** (-22 / 20)) < 0.1 * 10 ** (-22 / 20)
    k = int(round(170.0 * len(diff) / SR))
    assert np.abs(np.fft.rfft(diff))[k - 2:k + 3].max() > 0.5 * np.abs(np.fft.rfft(diff)).max()
    assert any(e["group"] == "kick" and e["abs_s"] < -1e5 for e in ring["events"])


def test_the_attack_phase_of_the_frame_sweep_is_exact_and_the_tune_sets_the_half_cycle():
    for c in cases("M3e"):
        for e in c["events"]:
            assert int(round(e["abs_sample"])) % 480 == int(round(e["phase_ms"] * 48))
    c = [c for c in cases("S1") if c["shell"] == "t4"][0]
    e = [e for e in c["events"] if e["tune_hz"] == 40.0][0]
    a = int(e["abs_sample"])
    y = c["buf"][a:a + 3000]
    zc = int(np.argmax(np.signbit(y[1:]) != np.signbit(y[:-1])))              # the first half cycle
    assert 0.4 * SR / (2 * 40.0 * 1.25) < zc < 1.2 * SR / (2 * 40.0 * 1.25)
    assert abs(e["period_ms"] - 25.0) < 1e-9


def test_the_polarity_pairs_are_the_same_hit_inverted():
    c = cases("S2")[0]
    by = {}
    for e in c["events"]:
        by.setdefault(e["pair"], []).append(e)
    pair = next(v for v in by.values() if len(v) == 2 and v[0]["rise_ms"] == 0.0)
    ya = c["buf"][int(pair[0]["abs_sample"]):int(pair[0]["abs_sample"]) + 2000]
    yb = c["buf"][int(pair[1]["abs_sample"]):int(pair[1]["abs_sample"]) + 2000]
    assert pair[0]["pol"] == -pair[1]["pol"] and np.allclose(ya, -yb, atol=1e-6)


# ---------------------------------------------------------------- verdicts: planted rows

def rows_for(hid, det="dsp", shell="kick", **kw):
    return {"hyp": hid, "detector": det, "shell": shell, "found": True, "sr": SR, **kw}


def pair_rows(hid, n, delta, key, a, b, **kw):
    out = []
    for i in range(n):
        base = 5.0 + (i % 3)
        out.append(rows_for(hid, err_abs=base, pair=float(i), rise_ms=2.0, **{key: a}, **kw))
        out.append(rows_for(hid, err_abs=base + delta, pair=float(i), rise_ms=2.0, **{key: b}, **kw))
    return out


def test_the_polarity_verdict_reads_a_planted_shift():
    same = pair_rows("S2", 8, 0.0, "pol", 1.0, -1.0)
    shifted = pair_rows("S2", 8, 3.0, "pol", 1.0, -1.0)
    assert ch.v_s2(same)["verdict"] == "held" and ch.v_s2(shifted)["verdict"] == "did not hold"


def test_the_clip_and_lowpass_verdicts_read_a_planted_shift_and_its_sign():
    early = pair_rows("S3", 8, -10.0, "drive_db", 0.0, 12.0)
    late = pair_rows("S3", 8, +10.0, "drive_db", 0.0, 12.0)
    assert ch.v_s3(early)["verdict"] == "held" and ch.v_s3(late)["verdict"] != "held"
    dark = pair_rows("S4", 8, +6.0, "cutoff_hz", 0.0, 2000.0)
    bright = pair_rows("S4", 8, -6.0, "cutoff_hz", 0.0, 2000.0)
    assert ch.v_s4(dark)["verdict"] == "held" and ch.v_s4(bright)["verdict"] != "held"


def test_the_sample_rate_verdict_tells_constant_milliseconds_from_constant_samples():
    def rows(fn):
        return [rows_for("R4", shell=sh, sr=float(sr), rise_ms=2.0, err_abs=fn(sr)) for sh in ("kick",) for sr in (44100, 48000, 96000)
                for _ in range(4)]
    ms = ch.v_r4(rows(lambda sr: 0.5e-3 * sr), Path("/nonexistent"))
    smp = ch.v_r4(rows(lambda sr: 24.0), Path("/nonexistent"))
    assert ms["verdict"] == "held" and smp["verdict"] == "did not hold"


def test_the_roll_verdict_distinguishes_late_from_early_shifts():
    def rows(shift):
        r = [rows_for("C1", spacing_ms=0.0, k=0.0, err_abs=0.0) for _ in range(6)]
        for sp in (125.0, 68.0):
            r += [rows_for("C1", spacing_ms=sp, k=float(k), err_abs=(shift if sp < 80 else 0.0)) for k in range(1, 10)]
        return r
    assert ch.v_c1(rows(+9.0))["verdict"] == "held"
    late = ch.v_c1(rows(-9.0))
    assert late["verdict"] == "did not hold" and "early" in late["headline"]


def test_the_flam_verdict_needs_the_main_hit_found():
    def rows(main_found):
        r = []
        for i in range(8):
            r.append(rows_for("C3", group="main", id=float(2 * i), delay_ms=30.0, grace_db=-14.0, found=main_found, err_abs=0.0))
            r.append(rows_for("C3", group="grace", id=float(2 * i + 1), delay_ms=30.0, grace_db=-14.0, err_abs=0.0))
        return r
    assert ch.v_c3(rows(True))["verdict"] == "held" and ch.v_c3(rows(False))["verdict"] == "did not hold"


def test_the_gate_verdict_compares_the_two_references():
    def rows(follows_gate):
        return [rows_for("R3", gate_db=-12.0, rise_ms=2.0, abs_sample=1000.0, gate_sample=1050.0,
                         det_sample=1050.0 if follows_gate else 1000.0, err_abs=50.0 if follows_gate else 0.0) for _ in range(6)]
    assert ch.v_r3(rows(True))["verdict"] == "held" and ch.v_r3(rows(False))["verdict"] != "held"


def test_the_mic_verdict_compares_the_two_mics():
    def rows(early):
        return [rows_for("R5", shell="snare", case="snare_mics_earlier_louder", delay_ms=d, abs_sample=1000.0, second_sample=1000.0 + 48 * d,
                         det_sample=1005.0 if early else 1000.0 + 48 * d, err_abs=5.0 if early else 48 * d) for d in (0.0, 1.0, 2.0, 3.0)
                for _ in range(4)]
    assert ch.v_r5(rows(True))["verdict"] == "held" and ch.v_r5(rows(False))["verdict"] != "held"


def test_the_level_verdict_of_the_25_percent_rule():
    rows = [{"hyp": "M", "shell": "kick", "shape": sh, "rise_ms": 2.0, "level_under_peak_db": lv, "err_abs": 20.0, "rise_db": 15.0,
             "ideal25": 24.0} for sh in ch.SHAPES for lv in (-11.0, -12.0, -10.0)]
    assert ch.v_m1(rows)["verdict"] == "held"
    off = [dict(r, level_under_peak_db=-4.0) for r in rows]
    assert ch.v_m1(off)["verdict"] == "did not hold"


# ---------------------------------------------------------------- end to end

def test_the_refinement_sweep_and_one_detector_run_end_to_end(tmp_path):
    parity = require(PARITY)
    tool = require(SERVICE_TOOL)
    import os
    os.environ["CARGO_TARGET_DIR"] = str(parity.parents[1])
    rule = ch.RULE
    try:
        ch.RULE = 2   # M1 is about the 25 % rule: version 2 (P4-12 stage 1) still ends there
        rows = ch.refine_all(tmp_path, quick=True)
    finally:
        ch.RULE = rule
    m1 = ch.v_m1([r for r in rows if r["hyp"] == "M"])
    assert m1["verdict"] == "held" and "dB under the local peak" in m1["headline"]
    # P4-12 stage 2: version 3 (the app's) walks back to the foot: the detected point is far under the peak, and
    # each hit is earlier than version 2 put it and closer to the first motion
    try:
        ch.RULE = 3
        first = ch.refine_all(tmp_path, quick=True)
    finally:
        ch.RULE = rule
    m1_3 = ch.v_m1([r for r in first if r["hyp"] == "M"])
    lv = float(m1_3["headline"].split("detected point ")[1].split(" dB")[0])
    assert m1_3["verdict"] == "did not hold" and lv >= 20.0, m1_3["headline"]
    two = [r for r in rows if r["hyp"] == "M"]
    three = [r for r in first if r["hyp"] == "M"]
    assert len(two) == len(three) and all(c["err_abs"] <= b["err_abs"] for b, c in zip(two, three))
    assert np.median([abs(c["err_abs"]) for c in three]) < np.median([abs(b["err_abs"]) for b in two])
    # M3 with the engine's refinement (rule 1, P4-11b): an attack 13 ms or more after the coarse time is out of reach
    m3 = ch.gen_m3_refine(tmp_path, quick=True, rule=1)
    inside = [r for r in m3 if 0.0 <= r["attack_vs_t0_ms"] <= 10.0]
    outside = [r for r in m3 if r["attack_vs_t0_ms"] >= 13.0]
    assert inside and max(abs(r["err_abs"]) for r in inside) < 24 and max(abs(r["err_abs"]) for r in outside) > 100
    # P4-12: the reach rule (rule 2, the app's) reaches an attack up to 18 ms after the coarse time as exactly as inside
    reach = [r for r in ch.gen_m3_refine(tmp_path, quick=True, rule=2) if 0.0 <= r["attack_vs_t0_ms"] <= 18.0]
    assert len(reach) == 2 * 19 and max(abs(r["err_abs"]) for r in reach) < 24   # kick and snare, 0 to 18 ms
    old = os.environ.get("DRUMSPIKE_CALIB_TOOL")
    os.environ["DRUMSPIKE_CALIB_TOOL"] = str(tool)
    try:
        ch.generate(tmp_path, quick=True, only={"S2"}, log=lambda *_: None)
        assert ch.run_cases(tmp_path, jobs=2, detectors=["dsp"], only={"S2"}, log=lambda *_: None) >= 2
        v = ch.compute_verdicts(tmp_path)
    finally:
        if old is None:
            os.environ.pop("DRUMSPIKE_CALIB_TOOL")
        else:
            os.environ["DRUMSPIKE_CALIB_TOOL"] = old
    assert v["S2"]["verdict"] in ("held", "did not hold") and "S2" in ch.verdict_text(v)
