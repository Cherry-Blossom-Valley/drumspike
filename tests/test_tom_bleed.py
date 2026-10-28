"""P4-14 stage 1: the tom bleed measurement (`tools/tom_bleed.py`). Synthetic signals and rows only; the real sessions
are not needed."""
import json
import math
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import tom_bleed as tb  # noqa: E402

SR = 48000


def burst(x, t, amp, freq, decay=0.08):
    a = int(t * SR)
    n = min(len(x) - a, int(0.4 * SR))
    k = np.arange(n) / SR
    x[a:a + n] += (amp * np.sin(2 * np.pi * freq * k) * np.exp(-k / decay)).astype(np.float32)


def test_auc_counts_pairs_with_half_ties():
    assert tb.auc([3, 4], [1, 2]) == 1.0
    assert tb.auc([1, 2], [3, 4]) == 0.0
    assert tb.auc([1, 3], [2, 2]) == 0.5
    assert tb.auc([2], [2]) == 0.5
    assert tb.auc([], [1]) is None
    rows = [{"real": True, "lag_ms": 2.0}, {"real": False, "lag_ms": -2.0}, {"real": None, "lag_ms": -9.0},
            {"real": True, "dom": 1.0}, {"real": False, "dom": 20.0}]
    assert tb.feature_auc(rows[:3], "lag_ms") == 1.0
    assert tb.feature_auc([dict(r, ex_ab=r.get("dom")) for r in rows[3:]], "dom") == 1.0      # higher dom is bleed


def test_tom_band_level_passes_the_tom_and_cuts_the_crack():
    x = np.zeros(SR, dtype=np.float32)
    burst(x, 0.2, 0.5, 150.0)
    burst(x, 0.6, 0.5, 3000.0)
    lv = tb.band_peaks_db(x, SR, [0.2, 0.6], tb.tom_sections(SR), window=(-0.0005, 0.03))
    assert lv[0] > 20 * math.log10(0.5) - 3.0
    assert lv[0] - lv[1] > 25.0
    # the vectorised filter equals a plain one-hit loop
    ref = []
    for t in (0.2, 0.6):
        pre, w0, w1 = int(round(0.05 * SR)), int(round(-0.0005 * SR)), int(round(0.03 * SR))
        a = int(round(t * SR)) + w0 - pre
        y = x[a:a + pre + w1 - w0].astype(np.float64)
        for b0, b1, b2, a1, a2 in tb.tom_sections(SR):
            z0 = z1 = 0.0
            o = np.empty_like(y)
            for k, v in enumerate(y):
                o[k] = b0 * v + z0
                z0 = b1 * v - a1 * o[k] + z1
                z1 = b2 * v - a2 * o[k]
            y = o
        ref.append(20 * math.log10(np.abs(y[pre:]).max() + 1e-9))
    assert np.allclose(lv, ref, atol=1e-9)


def test_strong_hits_are_within_6_db_of_the_90th_percentile():
    x = np.zeros(2 * SR, dtype=np.float32)
    times = [0.1 + 0.15 * i for i in range(10)]
    for i, t in enumerate(times):
        burst(x, t, 0.05 if i == 3 else 0.5, 200.0, decay=0.01)
    m = tb.strong_mask(x, SR, times)
    assert m.tolist() == [i != 3 for i in range(10)]


def test_fundamental_and_ring_find_the_tom_note():
    x = np.zeros(3 * SR, dtype=np.float32)
    snare = np.zeros_like(x)
    tom_times = [0.2, 0.7, 1.2]
    for t in tom_times:
        burst(x, t, 0.5, 110.0, decay=0.15)
    burst(x, 2.0, 0.1, 230.0, decay=0.03)     # snare bleed: another note, short
    burst(snare, 2.0, 0.6, 230.0, decay=0.03)
    f0 = tb.fundamental(x, SR, tom_times)
    assert abs(f0 - 110.0) < 3.0
    own = tb.ring_db(x, SR, tom_times + [2.0], f0)
    oth = tb.ring_db(snare, SR, tom_times + [2.0], f0)
    assert own[0] - own[3] > 20.0
    assert (own - oth)[0] - (own - oth)[3] > 20.0


def test_label_hits_shifts_the_reference_and_matches_one_to_one():
    ref = [1.0, 2.0, 3.0]
    times = [1.025, 1.032, 2.025, 3.025, 5.0]               # 25 ms late: outside +-20 ms without the offset
    off, real = tb.label_hits(ref, times)
    assert abs(off - 0.025) < 1e-9
    assert real == [True, False, True, True, False]


def test_pick_examples_takes_reference_hits_after_10_s_and_the_strongest_free_candidates():
    cands = [[5.0, 2, 0.9, 100], [11.0, 2, 0.5, 80], [12.01, 2, 0.4, 70], [13.0, 2, 0.8, 90], [20.0, 2, 0.3, 60],
             [30.0, 2, 0.95, 120]]
    this, not_ = tb.pick_examples([5.0, 11.0, 12.0, 20.0], cands, n_this=2, n_not=2)
    assert this == [11.0, 12.01]
    assert not_ == [30.0, 13.0]
    assert tb.nearest(np.array([1.0, 2.0]), [10, 20], 2.02) == 20
    assert tb.nearest(np.array([1.0, 2.0]), [10, 20], 2.05) is None


def rows_of(session, mic, spec):
    """rows from (real, ex_ab, ring_db)"""
    return [{"session": session, "mic": mic, "real": real, "excluded": False, "ex_ab": e, "ring_db": g, "p26": False,
             "dom": 0.0, "lag_ms": 0.0, "level_db": 0.0, "ring_own_db": 0.0, "learn": None}
            for real, e, g in spec]


def test_rule_removes_only_on_the_bleed_side_of_every_feature():
    r = {"ex_ab": 1.0, "ring_db": -10.0, "dom": 30.0}
    assert tb.removes([("ex_ab", 2.0)], r)
    assert not tb.removes([("ex_ab", 1.0)], r)                       # at the threshold: kept
    assert tb.removes([("ex_ab", 2.0), ("ring_db", -5.0)], r)
    assert not tb.removes([("ex_ab", 2.0), ("ring_db", -12.0)], r)
    assert tb.removes([("dom", 20.0)], r) and not tb.removes([("dom", 40.0)], r)
    assert not tb.removes([("learn", 0.5)], {"learn": None})


def test_choose_rule_keeps_the_real_hits_and_scores_held_out():
    # real hits: ex_ab 10..29, ring 0; false: ex_ab 0..9 and ring -20, one real hit sits among the false ones
    spec = [(True, 10.0 + i, 0.0) for i in range(99)] + [(True, 3.0, -20.0)] + [(False, float(i % 10), -20.0)
                                                                               for i in range(50)]
    rows = rows_of("Session A", "t1", spec) + rows_of("Session B", "t1", spec)
    refs = {"Session A": {"t1": [0.0] * 100}, "Session B": {"t1": [0.0] * 100}}
    ch = tb.choose_rule(rows, refs, feats=["ex_ab", "ring_db"], n=40)
    assert ch["n_real"] == 100 and ch["lost"] <= 1
    kept = [r for r in rows if not tb.removes(ch["rule"], r)]
    assert sum(1 for r in kept if r["real"] is False) == 0
    per, ses, pool = tb.evaluate(rows, refs, lambda r: not tb.removes(ch["rule"], r), ["Session B"])
    assert ses["Session B"]["fp"] == 0 and ses["Session B"]["tp"] == 99
    assert ses["Session B"]["fn"] == 1 and abs(ses["Session B"]["f"] - 2 * 99 / 199) < 1e-9
    # three real hits among the false ones: removing all false hits would lose 3 %, more than the cap allows
    spec3 = [(True, 10.0 + i, 0.0) for i in range(97)] + [(True, 3.0, -20.0)] * 3 + [(False, float(i % 10), -20.0)
                                                                                    for i in range(50)]
    ch3 = tb.choose_rule(rows_of("Session A", "t1", spec3), {"Session A": {"t1": [0.0] * 100}}, feats=["ex_ab"], n=40)
    assert ch3["lost"] == 0 and ch3["rule"] == [("ex_ab", 3.0)]


def test_manifest_lists_every_mic_with_its_hits_in_samples(tmp_path, monkeypatch):
    monkeypatch.setattr(tb, "SESSIONS", {"S": {"dir": "S dir", "excluded": set(), "mics": {
        "k in": ("k in_01", "kick", "k in"), "t1": ("t1_01", "tom", "t1")}}})
    monkeypatch.setattr(tb, "SAMPLES", tmp_path / "samples")
    for m, ev in (("k in", [{"t": 1.0, "v": 100, "rise": 20.0}]), ("t1", [{"t": 0.49999, "v": 90, "rise": None}])):
        wd = tmp_path / "work" / tb.tid("S", m) / "model"
        wd.mkdir(parents=True)
        (wd / "events.json").write_text(json.dumps({"sr": 48000, "events": ev}))
    man = tb.manifest(tmp_path, "S", tb.P26)
    assert man["params"]["bleed_roles"] == ["tom"] and man["params"]["resonance_enabled"] is False
    assert [(t["id"], t["role"], t["instrument"]) for t in man["tracks"]] == [("k in", "kick", "k in"), ("t1", "tom", "t1")]
    assert man["tracks"][1]["hits"] == [{"sample": 24000, "rise": None}]
    assert man["tracks"][0]["path"].endswith("S dir/Audio Files/k in_01.wav")


def test_relative_to_other_toms_takes_the_loudest_other_tom():
    own = tb.relative_to_other_toms([-10.0, -2.0], -20.0, [([-30.0, -40.0], -30.0), ([-15.0, -50.0], -20.0)])
    assert own.tolist() == [5.0, 28.0]                     # hit 1: 10 dB against 5 dB; hit 2: 18 dB against -10 dB
    assert tb.relative_to_other_toms([-10.0], -20.0, []).tolist() == [210.0]


def test_leave_one_track_out_scores_each_track_with_a_rule_from_the_others():
    reals = [(True, 10.0 + i, 0.0) for i in range(100)]
    rows = (rows_of("Session A", "t1", reals + [(False, float(i % 5), -20.0) for i in range(40)])
            + rows_of("Session A", "t2", reals + [(False, 5.0 + i % 5, -20.0) for i in range(40)]))
    refs = {"Session A": {"t1": [0.0] * 100, "t2": [0.0] * 100}}
    loo = {m: (rule, base, sc, lost) for m, rule, base, sc, lost in tb.leave_one_track_out(rows, refs, ["ex_ab"])}
    assert sorted(loo) == ["t1", "t2"]
    # t1 gets the rule of t2 (false hits up to 9 dB): all its false hits go; t2 gets t1's (below 10 dB, fit to 0 to 4 dB): some stay
    assert loo["t1"][2]["fp"] == 0 and loo["t2"][2]["fp"] > 0
    assert all(v[3] == 0 and v[1]["fp"] == 40 and v[2]["tp"] == 100 for v in loo.values())
    assert tb.rule_text([("level_db", -5.2), ("dom", 3.0)]) == "level_db < -5.2 and dom > 3"


def test_false_owner_names_the_nearest_other_reference():
    refs = {"S": {"k in": [1.0], "s top": [2.0], "t1": [3.0], "t2": [2.01], "toms_combined": [5.0]}}
    row = {"session": "S", "mic": "t1"}
    assert tb.false_owner(dict(row, t=1.010), refs) == "k in"
    assert tb.false_owner(dict(row, t=2.012), refs) == "tom"          # t2 (2.014) is nearer than the snare (2.004)
    assert tb.false_owner(dict(row, t=3.004), refs) is None           # its own reference does not count
    assert tb.false_owner(dict(row, t=5.004), refs) == "tom"          # the combined tom reference
    assert tb.false_owner(dict(row, t=1.040), refs) is None


def test_tom_rel_measures_each_other_tom_against_its_own_strong_hits():
    a = np.zeros(4 * SR, dtype=np.float32)
    b = np.zeros(4 * SR, dtype=np.float32)
    ta = [0.2 + 0.3 * i for i in range(6)]
    tb_ = [2.2 + 0.3 * i for i in range(6)]
    for t in ta:
        burst(a, t, 0.8, 110.0, decay=0.05)
        burst(b, t + 0.002, 0.024, 110.0, decay=0.05)     # A in B's mic: 10 dB under B's own hits
    for t in tb_:
        burst(b, t, 0.08, 90.0, decay=0.05)                # B is a quiet mic: its own hits are at 0.08
        burst(a, t + 0.002, 0.2, 90.0, decay=0.05)         # B in A's mic: louder than B's own mic, 12 dB under A's hits
    rel = tb.tom_rel_all({"t1": a, "t2": b}, {"t1": ta, "t2": tb_}, SR)
    assert rel["t1"][0] - rel["t2"][0] > 15.0               # each mic keeps its own reference level
    assert (rel["t2"][1] > 6.0).all()                       # B's hits are B's, though A's mic is louder there
    assert (rel["t1"][1] > 6.0).all()


def test_evaluate_leaves_out_excluded_tracks():
    rows = rows_of("Session A", "t1", [(True, 0.0, 0.0), (False, 0.0, 0.0)])
    rows += [dict(r, mic="t4", excluded=True, real=None) for r in rows_of("Session A", "t4", [(None, 0, 0)] * 3)]
    refs = {"Session A": {"t1": [0.0, 1.0], "t4": [0.0] * 5}}
    per, ses, pool = tb.evaluate(rows, refs, lambda r: True, ["Session A"])
    assert list(per) == [("Session A", "t1")]
    assert (pool["tp"], pool["fp"], pool["fn"]) == (1, 1, 1)


def test_sample_and_trigger_tracks_are_not_tom_close_mics():
    for stem in ("t1 sampl_01", "t samp_01.L", "t trigger_01.R", "t sample sd3_03.L", "T2 SAMP"):
        assert not tb.is_close_tom(stem)
    for stem in ("t1 hh_02", "t 3_03", "t4Ri_01", "t2-AtA2", "Tom4Ri.09_01"):
        assert tb.is_close_tom(stem)


def test_stroke_kind_names_unisons_flams_and_doubles():
    refs = {"S": {"t1": [1.0, 2.0], "t2": [1.005, 3.0], "s top": [4.0], "toms_combined": [9.0]}}

    def kind(mic, t):
        return tb.stroke_kind({"session": "S", "mic": mic, "t": t + 0.004, "offset_ms": 4.0}, refs)

    assert kind("t2", 1.005) == "unison"                    # t1 has a hit 5 ms away
    assert kind("t1", 2.03) == "flam"                       # its own hit 30 ms before
    assert kind("t1", 3.02) == "flam"                       # t2's hit 20 ms before
    assert kind("t1", 2.2) == "double"
    assert kind("t1", 2.3) is None and kind("t1", 4.0) is None      # 300 ms on; a snare hit is no tom
    assert kind("t1", 3.2) is None                                    # a double is on its own tom only
    rows = [dict(r, session="S", mic="t1", t=t + 0.004, offset_ms=4.0) for r, t in
            zip(rows_of("S", "t1", [(True, 0.0, 0.0), (False, 0.0, 0.0), (True, 9.0, 0.0)]), [2.2, 2.03, 5.0])]
    c = tb.stroke_counts(rows, refs, [("ex_ab", 5.0)])
    assert c == {"real": {"double": 1}, "false": {"flam": 1}}
