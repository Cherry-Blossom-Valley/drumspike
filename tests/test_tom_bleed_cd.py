"""P4-14 stage 1c: the candidates C and D (`tools/tom_bleed_cd.py`). Synthetic signals and rows only."""
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import tom_bleed_cd as cd  # noqa: E402

SR = 48000


def burst(x, t, amp, freq, decay=0.05):
    a = int(t * SR)
    n = min(len(x) - a, int(0.3 * SR))
    k = np.arange(n) / SR
    x[a:a + n] += (amp * np.sin(2 * np.pi * freq * k) * np.exp(-k / decay)).astype(np.float32)


def test_played_needs_a_level_near_the_other_toms():
    p = cd.played({"t1": -15.0, "t2": -18.0, "t3": -16.0, "t4": -27.0})
    assert p == {"t1": True, "t2": True, "t3": True, "t4": False}
    assert cd.played({"t1": -15.0, "t2": -21.5}, gap=6.0) == {"t1": True, "t2": False}
    assert cd.played({"t1": -15.0}) == {"t1": True}


def test_transfer_gain_uses_the_quiet_hits_when_there_are_enough():
    la = np.array([-30.0, -31, -32, -33, -34, -10, -11])
    lb = np.zeros(7)
    quiet = [True] * 5 + [False] * 2
    assert cd.transfer_gain(la, lb, quiet, pct=0) == -34.0
    assert cd.transfer_gain(la, lb, quiet, pct=100) == -30.0                            # the quiet hits only
    assert cd.transfer_gain(la, lb, [True] * 4 + [False] * 3, pct=100) == -10.0       # too few quiet: all hits
    assert cd.transfer_gain(la[:4], lb[:4], [True] * 4) is None


def test_tom_rel_and_excess_over_the_chosen_mics():
    lv = {"t1": {"t1": np.array([-20.0, -30.0]), "t2": np.array([-40.0, -25.0]), "t3": np.array([-35.0, -35.0])}}
    ref = {"t1": -20.0, "t2": -20.0, "t3": -40.0}
    assert cd.tom_rel_over(lv, ref, "t1", ["t2"]).tolist() == [20.0, -5.0]
    assert cd.tom_rel_over(lv, ref, "t1", ["t2", "t3"]).tolist() == [-5.0, -15.0]       # t3 at +5 over its level
    assert cd.tom_rel_over(lv, ref, "t1", []).tolist() == [200.0, 190.0]
    gain = {("t2", "t1"): -10.0, ("t3", "t1"): None}
    assert cd.excess_over(lv, gain, "t1", ["t2", "t3"]).tolist() == [30.0, 5.0]       # t3 has no gain: no source
    assert cd.excess_over(lv, {}, "t1", ["t2"]).tolist() == [200.0, 200.0]
    two = {("t2", "t1"): -10.0, ("t3", "t1"): 0.0}
    assert cd.excess_over(lv, two, "t1", ["t2", "t3"]).tolist() == [15.0, 5.0]   # the lowest excess


def test_session_measures_leave_out_a_tom_mic_without_its_own_drum():
    n = 8 * SR
    sig = {m: np.zeros(n, dtype=np.float32) for m in ("t1", "t2", "t3")}
    loud = [0.2 + 0.25 * i for i in range(8)]
    soft = [2.5 + 0.25 * i for i in range(6)]
    t2_hits = [4.5 + 0.25 * i for i in range(8)]
    for t in loud + soft:
        a = 0.6 if t in loud else 0.1                       # soft t1 hits: 15 dB under the loud ones
        burst(sig["t1"], t, a, 110.0)
        burst(sig["t2"], t + 0.002, a * 0.1, 110.0)
        burst(sig["t3"], t + 0.002, a * 0.05, 110.0)        # t3 hears only t1: its level is t1 bleed
    for t in t2_hits:
        burst(sig["t2"], t, 0.6, 90.0)
        burst(sig["t1"], t + 0.002, 0.06, 90.0)            # t2 in the t1 mic: listed as t1 hits too
        burst(sig["t3"], t + 0.002, 0.03, 90.0)
    times = {"t1": np.array(loud + soft + t2_hits), "t2": np.array(t2_hits), "t3": np.array(loud)}
    meas, ref, play, gain = cd.session_measures(sig, times, SR)
    assert play == {"t1": True, "t2": True, "t3": False}
    assert ref["t3"] < ref["t1"] - 20
    exc = meas["t1"]["exc_cd"]
    own = exc[:len(loud) + len(soft)]
    bleed = exc[len(loud) + len(soft):]
    assert own.min() > bleed.max() + 10                      # soft own hits stay far above t2's bleed
    rel_c = meas["t1"]["tom_rel_c"]
    assert rel_c[len(loud):len(loud) + len(soft)].min() > 10  # t3 does not count: soft t1 hits are t1's
    assert gain[("t2", "t1")] is not None and set(gain) == {(b, a) for b in sig for a in sig if a != b}


def rows(session, spec):
    return [{"session": session, "mic": "t1", "real": real, "excluded": False, "p26": p26, "exc_cd": v,
             "level_db": 0.0, "t": float(i), "offset_ms": 0.0} for i, (real, v, p26) in enumerate(spec)]


def test_fit_keeps_the_cap_on_each_fit_session():
    a = rows("Session A", [(True, 20.0 + i, False) for i in range(100)] + [(False, float(i % 5), False) for i in range(30)])
    b = rows("Session F", [(True, 20.0 + i, False) for i in range(98)] + [(True, 3.0, False)] * 2
             + [(False, float(i % 5), False) for i in range(30)])
    refs = {"Session A": {"t1": [0.0] * 100}, "Session F": {"t1": [0.0] * 100}}
    rule = cd.fit(a + b, refs, "exc_cd", False, n=40)
    lost = cd.lost_per_session(a + b, cd.remover(rule, False), cd.FIT)
    assert lost["Session A"][0] <= 1 and lost["Session F"][0] <= 1   # 2 real hits at 3 dB: 3 stays
    assert rule[-1][0] == "exc_cd" and rule[-1][1] <= 3.0
    assert cd.lost_per_session(a, cd.remover([], True), ["Session A"]) == {"Session A": (0, 100)}


def test_remover_joins_the_rule_and_the_p26_flag():
    r = {"p26": True, "exc_cd": 50.0}
    assert cd.remover([("exc_cd", 10.0)], True)(r) and not cd.remover([("exc_cd", 10.0)], False)(r)
    assert cd.remover([("exc_cd", 60.0)], False)(r) and not cd.remover([], False)(r)


def test_stroke_cells_count_removed_hits_by_kind():
    refs = {"S": {"t1": [1.0, 2.0], "t2": [3.0]}}
    rs = [{"session": "S", "mic": "t1", "t": t, "offset_ms": 0.0, "real": real, "excluded": False, "p26": True}
          for t, real in ((2.2, True), (3.0, False), (1.0, True), (7.0, None))]
    c = cd.stroke_cells(rs, refs, lambda r: r["p26"])
    assert c == {"real": "0 / 0 / 1 / 1", "false": "1 / 0 / 0 / 0"}


# ---------------------------------------------------------------- stage 2: the app's events

def test_track_name_follows_the_app():
    assert cd.track_name("t1 hh_02") == "t1_hh_02"
    assert cd.track_name("t1 hh-AtA2") == "t1_hh-AtA2"
    assert cd.track_name(" t 1.L ") == "t_1_L"


def test_app_mics_are_the_kick_snare_and_tom_mics():
    table = {"S": {"mics": {"k in": ("k in_01", "kick", "k in"), "hh": ("hh_01", "hat", "hh"),
                            "t1": ("t 1_01", "tom", "t1"), "s top": ("s top_01", "snare", "s top")}}}
    assert cd.app_mics(table, "S") == [("k in", "k in_01", "kick"), ("t1", "t 1_01", "tom"),
                                       ("s top", "s top_01", "snare")]


def test_wait_for_room_waits_for_the_lock_and_the_load(monkeypatch, tmp_path):
    lock = tmp_path / "measure.lock"
    lock.write_text("another agent")
    loads = [9.0, 9.0, 9.0, 1.0]
    slept = []
    monkeypatch.setattr(cd, "LOCK", lock)
    monkeypatch.setattr("os.getloadavg", lambda: (loads.pop(0) if len(loads) > 1 else loads[0], 0.0, 0.0))

    def sleep(s):
        slept.append(s)
        lock.unlink(missing_ok=True)
    monkeypatch.setattr("time.sleep", sleep)
    cd.wait_for_room(log=lambda m: None)
    assert len(slept) == 2          # once for the lock, once for the load
