"""P4-17 stage 1: tom bleed from the snare and kick mics (`tools/tom_bleed_sk.py`). Synthetic signals and events only."""
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import fix_metric as fm  # noqa: E402
import tom_bleed_sk as sk  # noqa: E402

SR = 48000


def burst(x, t, amp, freq, decay=0.05):
    a = int(t * SR)
    n = min(len(x) - a, int(0.3 * SR))
    k = np.arange(n) / SR
    x[a:a + n] += (amp * np.sin(2 * np.pi * freq * k) * np.exp(-k / decay)).astype(np.float32)


# ---------------------------------------------------------------- causes

def test_classify_takes_the_nearest_source_then_own_ring_unknown():
    others = [("snare", [1.010]), ("kick", [1.004]), ("other tom", [5.0])]
    assert sk.classify(1.0, [], others) == "kick"                       # 4 ms beats 10 ms
    assert sk.classify(1.0, [], [("snare", [1.010]), ("kick", [1.025])]) == "snare"
    assert sk.classify(5.02, [], others) == "other tom"
    assert sk.classify(3.0, [2.98], others) == "own"                    # its own tom 20 ms before: a second detection
    assert sk.classify(3.0, [2.5], others) == "ring"                    # 500 ms after its own tom's hit
    assert sk.classify(3.0, [1.5], others) == "unknown"                 # 1.5 s after: beyond the decay
    assert sk.classify(3.0, [3.5], others) == "unknown"                 # an own hit after it is no decay
    assert sk.classify(1.0, [], [("snare", [1.031])]) == "unknown"      # beyond 30 ms
    assert sk.in_decay(3.0, [2.5]) and not sk.in_decay(3.0, [2.99]) and not sk.in_decay(3.0, [])


def test_match_equals_the_p4_16_match():
    rng = np.random.default_rng(7)
    ref = np.sort(rng.uniform(0, 30, 200)).tolist()
    est = (np.sort(rng.uniform(0, 30, 120)).tolist() + [r + rng.normal(0, 0.008) for r in ref[::2]]
           + [r + 0.015 for r in ref[1::7]])
    assert sk.match(ref, est) == fm.match(ref, est)
    assert sk.match([1.0], [1.012, 1.004]) == [(0, 1)]
    assert sk.match([1.0, 1.030], [1.021]) == [(1, 0)]


def test_score_track_equals_the_p4_16_score():
    ref = [1.0, 2.0, 3.0, 4.0, 5.0]
    events = [{"t": 1.0}, {"t": 2.0025}, {"t": 2.01, "bleed": True}, {"t": 3.5}, {"t": 5.0005, "cut": True}, {"t": 9.0}]
    a, b = sk.score_track(ref, events, 0.001), fm.score_track(ref, events, 0.001)
    assert a == {k: b[k] for k in a}
    # P4-16b adds the confirmed-stroke fields; without flags they change nothing.
    assert b["fixes_ex"] == b["fixes"] and b["confirmed"] == 0


def test_false_and_real_hits_leave_out_the_flagged_ones():
    events = [{"t": 1.0}, {"t": 1.5}, {"t": 2.0, "bleed": True}, {"t": 3.004}]
    assert sk.false_hits([1.0, 2.0, 3.0], events, 0.0) == [1.5]
    assert sk.real_hits([1.0, 2.0, 3.0], events, 0.0) == [1.0, 3.004]
    assert sk.false_hits([1.0, 2.0, 3.0], events, 0.004) == [1.5]       # the offset moves the reference


# ---------------------------------------------------------------- candidate SK on signals

def snare_session(gated=False, shared=False):
    """A tom mic t1 and a snare mic: loud and soft tom hits, snare hits that bleed into t1 at -14 dB and are listed as
    t1 hits too. `gated`: t1 is digital silence outside its own hits; `shared`: the snare mic hears t1 as loud."""
    n = 10 * SR
    t1, sn = np.zeros(n, np.float32), np.zeros(n, np.float32)
    loud = [0.2 + 0.3 * i for i in range(8)]
    soft = [2.8 + 0.3 * i for i in range(6)]
    snare = [5.0 + 0.3 * i for i in range(12)]
    for t in loud + soft:
        a = 0.6 if t in loud else 0.1                    # soft tom hits 15.6 dB under the loud ones
        burst(t1, t, a, 110.0)
        burst(sn, t + 0.002, a * (0.9 if shared else 0.05), 110.0)
    for t in snare:
        burst(sn, t, 0.6, 200.0)
        if not gated:
            burst(t1, t + 0.002, 0.6 * 0.2, 200.0)       # -14 dB in the tom mic
    times = np.asarray(loud + soft + snare)
    return t1, sn, times, np.asarray(snare), len(loud) + len(soft)


def test_snare_bleed_has_a_low_excess_and_own_hits_a_high_one():
    t1, sn, times, st, n_own = snare_session()
    exc, notes = sk.sk_excess(t1, times, {"snare": (sn, st)}, SR)
    assert notes["snare"]["used"] and abs(notes["snare"]["g_sa"] + 14.0) < 1.5
    own, bleed = exc["snare"][:n_own], exc["snare"][n_own:]
    assert bleed.max() < 3.0                              # the gain explains the bleed
    assert own.min() > 30.0                               # soft own hits stay far above: the excess is a ratio
    assert own.min() > sk.cd.THRESHOLD_DB + 20


def test_a_gated_or_shared_source_is_not_used():
    t1, sn, times, st, n_own = snare_session(gated=True)
    exc, notes = sk.sk_excess(t1, times[:n_own], {"snare": (sn, st)}, SR)
    assert notes["snare"]["why"] == "silent" and notes["snare"]["silent"] >= 0.25 and exc == {}
    t1, sn, times, st, n_own = snare_session(shared=True)
    exc, notes = sk.sk_excess(t1, times, {"snare": (sn, st)}, SR)
    assert notes["snare"]["g_sa"] + notes["snare"]["g_as"] > -26.0
    assert notes["snare"]["why"] == "shared" and exc == {}
    exc, notes = sk.sk_excess(t1, times, {"snare": (sn, st[:3])}, SR)
    assert notes["snare"]["why"] == "no gain" and exc == {}


def test_rule_sk_adds_the_sources_to_rule_cd_and_stays_off_with_it():
    cdr = {"bleed": {"t1": [False, True, False, False, False]}, "off": None}
    exc = {"t1": {"snare": np.array([20.0, 20.0, 5.0, 20.0, 20.0]), "kick": np.array([20.0, 20.0, 20.0, 3.0, 9.0])}}
    assert sk.rule_sk(cdr, exc, 9.2)["t1"].tolist() == [False, True, True, True, True]
    assert sk.rule_sk(cdr, exc, 9.2, 4.0)["t1"].tolist() == [False, True, True, True, False]
    assert sk.rule_sk(cdr, exc, 4.0, 2.0)["t1"].tolist() == [False, True, False, False, False]
    assert sk.rule_sk(cdr, exc, None)["t1"].tolist() == [False, True, False, False, False]       # rule CD alone
    off = dict(cdr, off=("silence", ["t1"]))
    assert sk.rule_sk(off, exc, 9.2)["t1"].tolist() == [False, True, False, False, False]
    assert sk.rule_sk(cdr, {"t1": {}}, 9.2)["t1"].tolist() == [False, True, False, False, False]  # no source


def test_ring_guard_marks_a_flat_hit_in_the_decay_of_a_kept_hit():
    times = np.array([1.0, 1.2, 1.4, 2.0, 3.5, 3.52])
    rise = np.array([30.0, 1.0, 1.0, 20.0, 1.0, 1.0])
    keep = np.ones(6, bool)
    m = sk.ring_mask(times, keep, rise, 3.0)
    # 1.2 and 1.4: flat, inside 1.0's decay (1.4 counts from 1.0: a marked hit is not kept); 2.0 rises;
    # 3.5: 1.5 s after the last kept hit; 3.52: 20 ms after 3.5, inside the 30 ms of one stroke
    assert m.tolist() == [False, True, True, False, False, False]
    # a hit the guard marks is no kept hit: 2.5 is 1.5 s after the kept 1.0, not 0.6 s after the marked 1.9
    assert sk.ring_mask(np.array([1.0, 1.9, 2.5]), np.ones(3, bool), np.array([30.0, 1.0, 1.0]), 3.0).tolist() == [
        False, True, False]
    keep[0] = False                               # rule CD marked the first hit: 1.2 is the first kept hit
    assert sk.ring_mask(times, keep, rise, 3.0).tolist() == [False, False, True, False, False, False]
    cdr = {"bleed": {"t1": [False] * 6}, "off": None}
    out = sk.rule_sk(cdr, {"t1": {}}, None, ring={"t1": (times, rise)}, rise_db=3.0)
    assert out["t1"].tolist() == [False, True, True, False, False, False]


def test_ring_rise_is_high_at_an_attack_and_low_in_a_decay():
    x = np.zeros(2 * SR, np.float32)
    burst(x, 0.5, 0.6, 110.0, decay=0.5)
    r = sk.ring_rise(x, np.array([0.5, 0.9]), SR)
    assert r[0] > 40.0 and abs(r[1]) < 3.0


# ---------------------------------------------------------------- scoring

def song(exc, cd_marks=None, off=None):
    d = {"refs": {"k in": {"role": "kick", "onsets": [1.0]}, "t1": {"role": "toms", "onsets": [2.0, 3.0]}},
         "events": {"k in": [{"t": 1.0}], "t1": [{"t": 2.0}, {"t": 2.5}, {"t": 3.0}]},
         "off": {"k in": 0.0, "t1": 0.0}, "minutes": 1.0}
    m = {"toms": ["t1"], "cd": {"t1": cd_marks or [False] * 3}, "off": off, "exc": {"t1": exc},
         "rise": {"t1": [30.0] * 3}}
    return d, m


def test_evaluate_counts_fixes_and_lost_real_hits():
    d, m = song({"snare": [20.0, 3.0, 8.0]})
    base = sk.evaluate({"S": d}, {"S": m}, None)["S"]
    assert (base["scores"]["toms"]["false"], base["scores"]["toms"]["fixes"], base["lost"], base["real"]) == (1, 1, 0, 2)
    r = sk.evaluate({"S": d}, {"S": m}, 5.0)["S"]
    assert (r["scores"]["toms"]["fixes"], r["lost"], r["marked"]) == (0, 0, 1)
    r = sk.evaluate({"S": d}, {"S": m}, 9.2)["S"]
    assert (r["scores"]["toms"]["missed"], r["scores"]["toms"]["fixes"], r["lost"]) == (1, 1, 1)
    assert r["scores"]["kick"]["fixes"] == 0
    d, m = song({"snare": [20.0, 3.0, 8.0]}, off=["silence", ["t1"]])
    assert sk.evaluate({"S": d}, {"S": m}, 9.2)["S"]["marked"] == 0


def test_accepted_caps_each_fit_song_and_the_held_out_total():
    def res(lost):
        return {s: {"lost": lost.get(s, 0), "real": 100} for s in sk.FIT + sk.HELD}
    assert sk.accepted(res({"Session A": 1, "Session F": 1, "Session H": 4}))
    assert not sk.accepted(res({"Session A": 2}))
    assert not sk.accepted(res({"Session F": 2}))
    assert not sk.accepted(res({h: 2 for h in sk.HELD} | {"Session H": 0}))    # 8 of 500 held out
    assert not sk.accepted(res({h: 1 for h in sk.HELD} | {"Session H": 2}))    # 6 of 500 held out


def test_fit_keeps_the_cap_on_each_fit_song():
    d1, m1 = song({"snare": [20.0, 3.0, 8.0]})
    data, meas = {s: d1 for s in sk.FIT}, {s: m1 for s in sk.FIT}
    assert sk.fit(data, meas, [2.0, 5.0, 9.0]) == (5.0, None, None)          # 9 dB loses 3.0 on each song
    assert sk.fit(data, meas, [9.0, 12.0]) is None


def test_second_strokes_count_unowned_hits_10_to_50_ms_after_a_tom_hit():
    d = {"refs": {"t1": {"role": "toms", "onsets": [2.0]}, "t2": {"role": "toms", "onsets": [5.0]},
                  "s top": {"role": "snare", "onsets": [7.0]}},
         "off": {"t1": 0.0, "t2": 0.001, "s top": 0.0}, "real": {"t1": [2.0], "t2": []}}
    newly = {"t1": [2.0, 2.02, 5.03, 5.06, 7.02], "t4": [2.02]}
    # 2.0 is real, 2.02 after t1's 2.0, 5.03 after t2's 5.001, 5.06 too late, 7.02 after the snare only, t4 has no refs
    assert sk.second_strokes(d, newly) == 2


def test_marked_events_set_and_clear_the_flag():
    ev = [{"t": 1.0, "bleed": True}, {"t": 2.0, "v": 5}]
    assert sk.marked_events(ev, [False, True]) == [{"t": 1.0}, {"t": 2.0, "v": 5, "bleed": True}]


def test_wait_for_room_waits_for_the_lock_and_the_load(monkeypatch, tmp_path):
    lock = tmp_path / "measure.lock"
    lock.write_text("another agent")
    loads = [9.0, 9.0, 9.0, 1.0]
    slept = []
    monkeypatch.setattr(sk, "LOCK", lock)
    monkeypatch.setattr("os.getloadavg", lambda: (loads.pop(0) if len(loads) > 1 else loads[0], 0.0, 0.0))

    def sleep(s):
        slept.append(s)
        lock.unlink(missing_ok=True)
    monkeypatch.setattr("time.sleep", sleep)
    sk.wait_for_room(log=lambda m: None)
    assert len(slept) == 2
