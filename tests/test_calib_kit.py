"""P4-11: the calibration kit (`tools/calib_kit.py`). The generator puts every hit start at the known sample (the first
non-zero sample; fractional starts within 0.01 sample), the references are consistent (MIDI against CSV, gaps), and the
comparer reads a planted +3 sample shift as +3.00, a dropped hit as missed and an added hit as false. One small
end-to-end case runs the DSP detector through `service_tool cli run` (skipped without the tool)."""
import csv
import json
import math
import os
import shutil
import subprocess
import sys
from pathlib import Path

import mido
import numpy as np
import pytest
import soundfile as sf

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import calib_kit as ck  # noqa: E402
from core_tools import SERVICE_TOOL, require  # noqa: E402

SR = 48000


# #174: the synthetic kit is deterministic (seeded), so the tests that only read it share one
# session-scoped build instead of rebuilding it per test. The end-to-end test, which writes
# results under the kit root, copies it into its own tmp_path first.
@pytest.fixture(scope="session")
def kit_session(tmp_path_factory):
    out = tmp_path_factory.mktemp("calib-kit")
    ck.generate(out, quick=True, real_per_shell=4, log=lambda *_: None)
    return out


def first_nonzero(y):
    return int(np.argmax(y != 0.0))


@pytest.mark.parametrize("shell", ck.SHELLS)
def test_every_synthetic_start_is_exact_to_the_sample(shell):
    for rise in ck.RISES_MS:
        for lv in ck.LEVELS_DB:
            for pol in ck.POLARITIES:
                s = ck.make_shot(shell, SR, rise, lv, pol, 0.0, seed=5)
                # the nominal start is the first non-zero sample, and the reference says so
                assert first_nonzero(s["y"]) == s["lead"] == int(s["abs_off"]), (shell, rise, lv, pol)
                assert np.sign(s["y"][s["lead"]]) == pol
                assert abs(np.abs(s["y"]).max() - 10 ** (lv / 20)) < 1e-9


def _shift_error(x0, y, d):
    n = len(x0)
    f = np.fft.rfftfreq(n)
    return float(np.sum((np.fft.irfft(np.fft.rfft(x0) * np.exp(-2j * np.pi * f * d), n) - y) ** 2))


@pytest.mark.parametrize("shell", ["kick", "snare", "t2"])
def test_fractional_starts_are_within_a_hundredth_of_a_sample(shell):
    base = ck.make_shot(shell, SR, 0.5, -6.0, 1, 0.0, seed=9)["y"]
    for frac in (0.25, 0.5, 0.75):
        s = ck.make_shot(shell, SR, 0.5, -6.0, 1, frac, seed=9)
        assert s["abs_off"] == s["lead"] + frac
        lo, hi = 0.0, 1.0   # golden section on the delay that best explains the shot
        g = (5 ** 0.5 - 1) / 2
        for _ in range(40):
            a, b = hi - g * (hi - lo), lo + g * (hi - lo)
            if _shift_error(base, s["y"], a) < _shift_error(base, s["y"], b):
                hi = b
            else:
                lo = a
        assert abs((lo + hi) / 2 - frac) < 0.01, (shell, frac, (lo + hi) / 2)


def test_the_reference_of_a_mix_matches_the_audio_and_the_midi(kit_session):
    tmp_path = kit_session
    kinds = ("syn", "real") if ck.LOGIC.is_dir() else ("syn",)   # CI hosts have no Logic content
    for kind in kinds:
        for shell in ck.SHELLS:
            sr = SR if kind == "syn" else ck.SR_REAL
            rows = ck.read_csv(tmp_path / "ref" / kind / "clean" / f"{shell}_{sr}.csv")
            x, fsr = sf.read(str(tmp_path / "audio" / kind / "clean" / f"{shell}_{sr}.wav"))
            assert fsr == sr and len(rows) >= 10
            ts = sorted(r["abs_sample"] for r in rows)
            assert min(np.diff(ts)) / sr >= 0.0199          # pairs are 20 ms or more apart
            main = sorted(r["abs_sample"] for r in rows if r["group"] == "main")
            assert min(np.diff(main)) / sr >= 0.15          # off-pair gap: 150 ms or more
            assert any(r["group"] == "pair" for r in rows)
            mid = mido.MidiFile(str(tmp_path / "ref" / kind / "clean" / f"{shell}_{sr}.mid"))
            assert mid.type == 1 and mid.ticks_per_beat == 24000
            tempos = [m.tempo for t in mid.tracks for m in t if m.type == "set_tempo"]
            assert tempos == [500000]
            got = ck.midi_hits_s(tmp_path / "ref" / kind / "clean" / f"{shell}_{sr}.mid")
            want = sorted(r["abs_s"] for r in rows)
            assert len(got) == len(want)
            assert max(abs(a - b) for a, b in zip(got, want)) <= 0.5 / 48000 + 1e-9   # within half a tick


def test_a_clean_mix_holds_exact_first_samples_at_the_reference(kit_session):
    tmp_path = kit_session
    rows = ck.read_csv(tmp_path / "ref" / "syn" / "clean" / "kick_48000.csv")
    x, _ = sf.read(str(tmp_path / "audio" / "syn" / "clean" / "kick_48000.wav"))
    n = 0
    for r in rows:
        if r["frac"] == 0.0 and r["group"] == "main":
            i = int(r["abs_sample"])
            assert x[i] != 0.0, r["id"]
            n += 1
    assert n >= 4


def test_real_references_carry_both_starts_and_the_features(kit_session):
    if not ck.LOGIC.is_dir():
        pytest.skip("no Logic content on this host")
    tmp_path = kit_session
    rows = ck.read_csv(tmp_path / "ref" / "real" / "clean" / "snare_44100.csv")
    x, _ = sf.read(str(tmp_path / "audio" / "real" / "clean" / "snare_44100.wav"))
    for r in rows:
        assert r["aud_sample"] >= r["abs_sample"] and r["gap_samples"] >= 0
        for k in ("rise_ms", "centroid_hz", "decay_ms", "level_db", "presilence_ms"):
            assert k in r
        a = int(r["abs_sample"])
        assert abs(x[a]) > 10 ** (-90 / 20) * 0.5     # the reference sample is at or above -90 dBFS


def synthetic_run(shift=0.0, drop=None, add=None):
    refs = []
    for i in range(10):
        s = 0.5 + 0.4 * i
        refs.append({"id": i, "abs_s": s, "abs_sample": s * SR, "group": "main"})
    det = [r["abs_s"] + shift / SR for r in refs]
    if drop is not None:
        det.pop(drop)
    if add is not None:
        det.append(add)
    return ck.compare_pair(refs, sorted(det), SR)


def test_a_planted_shift_of_three_samples_reads_as_plus_three():
    rows, counts = synthetic_run(shift=3.0)
    st = ck.stats([r["err_abs"] for r in rows if r["found"]])
    assert (counts["found"], counts["missed"], counts["false"]) == (10, 0, 0)
    assert all(r["found"] for r in rows)
    assert abs(st["mean"] - 3.0) < 1e-6 and abs(st["median"] - 3.0) < 1e-6 and st["sd"] < 1e-6
    assert abs(st["min"] - 3.0) < 1e-6 and abs(st["p95"] - 3.0) < 1e-6


def test_a_dropped_hit_is_missed_and_an_added_hit_is_false():
    rows, counts = synthetic_run(drop=4)
    assert (counts["found"], counts["missed"], counts["false"]) == (9, 1, 0)
    assert [r["id"] for r in rows if not r["found"]] == [4]
    rows, counts = synthetic_run(add=2.0 + 0.2)
    assert (counts["found"], counts["missed"], counts["false"]) == (10, 0, 1)
    # a detection 30 ms away does not match, and is counted as a far-off detection of the missed hit
    rows, counts = synthetic_run(drop=2, add=0.5 + 0.8 + 0.030)
    assert (counts["found"], counts["missed"], counts["false"], counts["far"]) == (9, 1, 1, 1)
    assert abs(rows[2]["err_far"] - 0.030 * SR) < 1e-6


def test_matching_is_one_to_one_and_nearest_first():
    pairs, missed, false = ck.match([1.0, 1.02], [1.012])
    assert pairs == [(1, 0)] and missed == [0] and false == []


def test_the_midi_of_a_detection_is_read_through_its_tempo(tmp_path):
    mid = mido.MidiFile(type=1, ticks_per_beat=24000)
    t0, t1 = mido.MidiTrack(), mido.MidiTrack()
    t0.append(mido.MetaMessage("set_tempo", tempo=500000, time=0))
    t1.append(mido.Message("note_on", channel=9, note=38, velocity=90, time=96000))   # 2.0 s
    t1.append(mido.Message("note_off", channel=9, note=38, velocity=0, time=2400))
    mid.tracks.extend([t0, t1])
    mid.save(tmp_path / "d.mid")
    assert ck.midi_hits_s(tmp_path / "d.mid") == [2.0]


def test_tables_and_compare_end_to_end_with_the_dsp_detector(tmp_path, kit_session):
    # writes results under the kit root, so it works on its own copy of the shared kit (#174)
    for sub in ("audio", "ref"):
        shutil.copytree(kit_session / sub, tmp_path / sub)
    tool = require(SERVICE_TOOL)
    env_before = os.environ.get("DRUMSPIKE_CALIB_TOOL")
    os.environ["DRUMSPIKE_CALIB_TOOL"] = str(tool)
    try:
        n = ck.run_all(tmp_path, jobs=2, detectors=["dsp"], only=r"syn/clean/(kick|snare)_", log=lambda *_: None)
        assert n == 2
        res = ck.compare(tmp_path, log=lambda *_: None)
    finally:
        if env_before is None:
            os.environ.pop("DRUMSPIKE_CALIB_TOOL")
        else:
            os.environ["DRUMSPIKE_CALIB_TOOL"] = env_before
    assert len(res["runs"]) == 2
    for run in res["runs"]:
        assert run["found"] >= 4 and run["detector"] == "dsp"
    errs = [r["err_abs"] for r in res["rows"] if r.get("found") and r["group"] == "main" and r["level_db"] == -6.0
            and r["rise_ms"] == 0.0]
    assert errs and max(abs(e) for e in errs) < 5.0           # a hard attack at -6 dBFS: within a few samples
    # the MIDI D agrees with the events to the tick
    d = [r["midi_minus_events"] for r in res["rows"] if "midi_minus_events" in r]
    assert d and max(abs(x) for x in d) <= 0.5 + 1e-6
    assert (tmp_path / "tables.md").read_text().startswith("### Synthetic")


# ---- the spectral fingerprint and the sweeps (owner addition, 2026-10-13)

def fp_of(shell, **kw):
    s = ck.make_shot(shell, SR, 0.0, -6.0, 1, 0.0, seed=3, **kw)
    return s["fp"]


def test_the_fingerprint_separates_mids_from_highs_and_strong_from_weak_attacks():
    mids = fp_of("snare", band=0.0, click=0.0)
    highs = fp_of("snare", band=1.0, click=0.0)
    assert highs["hi_mid_db"] > mids["hi_mid_db"] + 15.0
    assert highs["rolloff85_hz"] > mids["rolloff85_hz"] * 1.5 and highs["centroid10_hz"] > mids["centroid10_hz"]
    assert abs(mids["e_low"] + mids["e_mid"] + mids["e_high"] - 1.0) < 1e-9
    strong = fp_of("snare", band=0.5, click=2.0)
    none = fp_of("snare", band=0.5, click=0.0)
    assert strong["attack_db"] > none["attack_db"] + 3.0


def test_the_fingerprint_finds_the_fundamental_where_it_is_clear():
    for shell, f0 in (("t3", 125.0), ("t1", 230.0)):
        fp = ck.make_shot(shell, SR, 0.0, -6.0, 1, 0.0, seed=3, click=0.0)["fp"]
        assert abs(fp["f0_hz"] - f0) < 0.2 * f0, (shell, fp["f0_hz"])
    # the rise time and the decay time are in the fingerprint
    slow = ck.make_shot("kick", SR, 5.0, -6.0, 1, 0.0, seed=3)["fp"]
    fast = ck.make_shot("kick", SR, 0.0, -6.0, 1, 0.0, seed=3)["fp"]
    assert slow["rise1090_ms"] > fast["rise1090_ms"] + 1.5 and fast["decay_ms"] > 20.0


def test_the_sweeps_keep_the_exact_start_and_name_their_axis(kit_session):
    for shell in ck.SHELLS:
        axes = ck.sweep_axes(shell, quick=False)
        assert {a for a, _, _ in axes} == ({"tune", "click", "band"} if shell == "snare" else {"tune", "click"})
        for _, _, kw in axes:
            assert ("band" in kw) == (shell == "snare") and kw["click"] is not None
            s = ck.make_shot(shell, SR, 0.0, ck.SWEEP_LEVEL_DB, 1, 0.0, seed=7, **kw)
            assert first_nonzero(s["y"]) == s["lead"] == int(s["abs_off"])
    tmp_path = kit_session
    rows = ck.read_csv(tmp_path / "ref" / "sweep" / "clean" / "snare_48000.csv")
    main = [r for r in rows if r["group"] == "main"]
    assert {r["axis"] for r in main} == {"tune", "click", "band"}
    assert all(not math.isnan(r["hi_mid_db"]) and not math.isnan(r["attack_db"]) for r in rows)
    assert min(np.diff(sorted(r["abs_sample"] for r in main))) / SR >= 0.15


def test_the_fingerprint_tables_group_error_and_rate_by_feature():
    rows = []
    for i in range(60):
        hi = i % 3
        rows.append({"kind": "real", "cond": "clean", "sr": 44100, "shell": "snare", "detector": "dsp", "group": "main",
                     "found": i % 5 != 0, "err_abs": 4.0 * hi, "err_aud": 0.0, "gain_db": 0.0,
                     **{f: float(hi) + 0.01 * i for f in ck.FEATURES}, "attack_db": float(hi), "hi_mid_db": float(hi)})
    text = ck.fingerprint_tables({"rows": rows, "runs": []})
    assert "hi_mid_db" in text and "Pearson" in text
    # the error rises with the feature by construction: the ranking names it with r near +1
    line = [ln for ln in text.splitlines() if ln.startswith("| real | snare | dsp |")][0]
    assert "(+0.9" in line or "(+1.0" in line
    assert ck._corr([1, 2, 3, 4, 5, 6], [2, 4, 6, 8, 10, 12]) > 0.999


# ---- the acoustic slicer (consolidated files)

def test_the_slicer_returns_the_exact_start_of_every_planted_slice():
    sr = 44100
    x = np.zeros(sr * 6)
    rng = np.random.default_rng(1)
    plan = [(1000, 6000), (30000, 36000), (80000, 90000), (150000, 152000), (200000, 230000)]
    for a, b in plan:
        n = b - a
        x[a:b] = rng.uniform(0.5, 1.0, n) * np.exp(-np.arange(n) / (n / 5)) * rng.choice([-1, 1], n)
    x[2500] = 0.0                                   # a single silent sample inside a slice: no gap
    # two bursts 10 ms apart (shorter than the 20 ms gap) are one slice
    x[100000:100500] = 0.2
    x[100500 + 441:101000 + 441] = 0.2
    # two bursts 30 ms apart (longer than the gap) are two slices
    x[120000:120500] = 0.2
    x[120500 + 1323:121000 + 1323] = 0.2
    # a start just above -90 dBFS after a gap counts; a tail just below does not extend the slice
    x[260000] = 10 ** (-89.0 / 20)
    x[260001:260100] = 10 ** (-91.0 / 20) * 0.5
    got = ck.slice_by_gaps(np.abs(x), sr, min_s=0.0)
    starts = [a for a, _ in got]
    assert starts == [a for a, _ in plan[:3]] + [100000, 120000, 120000 + 500 + 1323] + [150000, 200000, 260000], starts
    assert got[3] == (100000, 101441), got[3]
    assert got[-1] == (260000, 260001) and got[4] == (120000, 120500)
    # a short blip is dropped with the default minimum length (50 ms)
    assert [a for a, _ in ck.slice_by_gaps(np.abs(x), sr)] == [1000, 30000, 80000, 200000]


def test_the_acoustic_set_gets_its_slices_and_references(kit_session):
    if not ck.DKD.is_dir():
        pytest.skip("no Drum Kit Designer content on this host")
    tmp_path = kit_session
    rows = ck.read_csv(tmp_path / "ref" / "acoustic" / "clean" / "kick_44100.csv")
    x, _ = sf.read(str(tmp_path / "audio" / "acoustic" / "clean" / "kick_44100.wav"))
    assert len(rows) >= 8
    for r in rows:
        assert abs(x[int(r["abs_sample"])]) > 10 ** (-90 / 20) * 0.5       # the start is above -90 dBFS
        assert r["aud_sample"] >= r["abs_sample"]


def test_the_backtrack_what_if_goes_to_the_foot_of_a_ramp():
    sr = 44100
    x = np.zeros(sr // 2)
    ramp = np.linspace(0.0, 1.0, 220)[1:]                  # 5 ms ramp from sample 1000
    x[1000:1000 + len(ramp)] = ramp
    x[1000 + len(ramp):1500] = 1.0
    foot = ck.backtrack(x, 1000 + 120, sr, 30.0)
    assert abs(foot - (1000 + 7)) <= 2 and foot < 1000 + 120      # the -30 dB point of a linear ramp is 3 % up


# ---- the audible-start (`aud`) reference

def test_the_audible_start_is_the_first_sample_within_40_db_of_the_peak():
    for shell in ("kick", "snare"):
        for rise in (0.5, 2.0, 5.0):
            s = ck.make_shot(shell, SR, rise, -6.0, -1, 0.0, seed=4)
            y, a = np.abs(s["y"]), s["aud_off"]
            peak = y.max()
            assert y[a] >= peak * 0.01 and np.all(y[:a] < peak * 0.01), (shell, rise)
    # a plain ramp: 1 % of the peak is reached at 1 % of the ramp
    x = np.zeros(1000)
    x[100:300] = np.linspace(0.0, 1.0, 200)
    assert ck.features(x, SR)["aud_start"] == 100 + 2 and ck.features(x, SR)["abs_start"] == 101


def test_the_aud_error_is_the_detection_minus_the_aud_sample():
    refs = [{"id": 0, "abs_s": 1.0, "abs_sample": SR * 1.0, "aud_sample": SR * 1.0 + 7.0, "group": "main"}]
    rows, _ = ck.compare_pair(refs, [1.0 + 10.0 / SR], SR)
    assert abs(rows[0]["err_abs"] - 10.0) < 1e-6 and abs(rows[0]["err_aud"] - 3.0) < 1e-6   # later than aud: positive
    rows, _ = ck.compare_pair(refs, [1.0 + 4.0 / SR], SR)
    assert abs(rows[0]["err_aud"] + 3.0) < 1e-6                                              # before aud: negative


def test_the_acoustic_pool_drops_fragments_and_features_come_from_the_cut_shot(tmp_path):
    if not ck.DKD.is_dir():
        pytest.skip("no Drum Kit Designer content on this host")
    pool = ck.acoustic_pool()
    for cls, items in pool.items():
        assert items
        top = max(i["peak_db"] for i in items)
        assert all(i["peak_db"] >= top - 25.0 for i in items), cls      # no fragment 20 dB under the file's peak
        for i in items:
            assert len(i["y"]) <= int(0.8 * SR)                          # features are of the cut shot
    cut = ck.cut_shot(np.ones(SR), SR)
    assert len(cut) == int(0.8 * SR) and cut[0] == 1.0 and cut[-1] < 0.01   # cut at 0.8 s with a fade


def test_the_acoustic_pool_drops_slices_whose_audible_start_is_far_after_the_first_sample():
    if not ck.DKD.is_dir():
        pytest.skip("no Drum Kit Designer content on this host")
    pool = ck.acoustic_pool()
    assert all(i["gap_samples"] <= ck.ACOUSTIC_MAX_GAP_S * ck.SR_REAL for items in pool.values() for i in items)
    assert not any(i["name"] == "SoCal Kit Snare#38" for i in pool["snare"])      # the slice with a long run-in
