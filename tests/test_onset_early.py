"""P4-12 stage 1 and 2: the real-session early-hit tool (`tools/onset_early.py`): the close-mic roles by the owner's names,
the estimator of the P4-11b verifier on planted hits, and the table. The real sessions are not needed."""
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import onset_early as oe  # noqa: E402

SR = 48000


def hit(x, at, amp=0.5, decay=0.05, f=180.0):
    n = len(x) - at
    t = np.arange(n) / SR
    x[at:] += amp * np.sin(2 * np.pi * f * t) * np.exp(-t / decay)


def test_the_close_mics_get_their_roles_and_the_rest_none(tmp_path):
    names = {"k in_01.wav": "kick", "s top_02.wav": "snare", "t1 HH_01.wav": "tom", "t2_02.wav": "tom",
             "t4 ri_02.wav": "tom", "t1 sampl_01.wav": None, "t3 samp_01.wav": None, "k sub_02.wav": None,
             "k out_02.wav": None, "s bot_02.wav": None, "k samp savior_01.wav": None, "hh_01.wav": None}
    assert {n: oe.role_of(n) for n in names} == names
    for sess in ("B song", "A song"):
        (tmp_path / sess / "Audio Files").mkdir(parents=True)
        for n in names:
            (tmp_path / sess / "Audio Files" / n).write_bytes(b"")
    (tmp_path / "loose").mkdir()
    got = oe.tracks(tmp_path, sessions=None)
    assert [g[0] for g in got] == ["A song"] * 5 + ["B song"] * 5
    assert sorted({(g[1], g[3]) for g in got}) == sorted((n, r) for n, r in names.items() if r)


def test_only_the_three_songs_are_measured(tmp_path):
    """P4-12 stage 2: new session folders appear under `samples` (being copied, no references): the default reads
    only Session A, Session B and Session C."""
    assert oe.SESSIONS == ("Session A", "Session B", "Session C")
    for sess in oe.SESSIONS + ("Session G", "Session H"):
        (tmp_path / sess / "Audio Files").mkdir(parents=True)
        (tmp_path / sess / "Audio Files" / "k in_01.wav").write_bytes(b"")
    assert sorted(g[0] for g in oe.tracks(tmp_path)) == sorted(oe.SESSIONS)
    assert len(oe.tracks(tmp_path, sessions=None)) == 5
    assert oe.RULES == (1, 2, 3)


def test_a_hit_10_ms_or_more_before_the_attack_is_early_and_the_next_hit_clips_the_window():
    x = np.zeros(2 * SR, dtype=np.float32)
    x += 1e-4 * np.random.default_rng(3).standard_normal(len(x)).astype(np.float32)   # a floor
    hit(x, SR)                                   # an attack at 1.0 s
    hit(x, SR + int(0.030 * SR))                 # and one 30 ms later
    on = 1.0
    res = oe.early_flags(x, SR, [on - 0.012, on + 0.030])
    (d0, e0), (d2, e2) = res
    assert e0 and 11.5 < d0 < 12.5, res          # 12 ms before the attack
    assert not e2 and d2 < 0.5, res              # the second attack (the first one's tail is above 25 %)
    (d1, e1), = oe.early_flags(x, SR, [on])
    assert not e1 and abs(d1) < 0.5, (d1, e1)    # on the attack
    # 9 ms before is not early; alone, a hit 25 ms before the attack sees it within its 40 ms (early), but with a
    # next hit 20 ms after it the window ends at the midpoint and holds only the floor (not early)
    assert oe.early_flags(x, SR, [on - 0.009])[0][1] is False
    assert oe.early_flags(x, SR, [on - 0.025])[0][1] is True
    assert oe.early_flags(x, SR, [on - 0.025, on - 0.005])[0][1] is False
    assert oe.early_flags(np.zeros(100, dtype=np.float32), SR, [0.0]) == [(0.0, False)]


def test_the_table_counts_early_hits_per_detector_and_role():
    per = [{"detector": "dsp", "role": "kick", "rule": 1, "hits": 100, "early": 4},
           {"detector": "dsp", "role": "snare", "rule": 1, "hits": 50, "early": 1},
           {"detector": "dsp", "role": "kick", "rule": 2, "hits": 100, "early": 0},
           {"detector": "dsp", "role": "snare", "rule": 2, "hits": 50, "early": 1}]
    rows = {(r["detector"], r["role"]): r for r in oe.table(per, (1, 2))}
    assert rows[("dsp", "kick")]["rate_1"] == 4.0 and rows[("dsp", "kick")]["rate_2"] == 0.0
    assert rows[("dsp", "all")]["early_1"] == 5 and rows[("dsp", "all")]["hits_2"] == 150
    per[0]["late"] = 2
    md = oe.markdown(oe.table(per, (1, 2)), {"dsp": {"hits_before": 150, "hits_after": 150, "moved_1ms": 3,
                                                       "max_move_ms": 14.5, "moved_later": 3, "moved_earlier": 0,
                                                       "moved_onto_body": 1, "onto_body_at": ["A / k in 1.000"]}}, (1, 2))
    assert "| dsp | all | 5 / 2 / 150 (3.33 %, 1.33 %) | 1 / 0 / 150 (0.67 %, 0.00 %) |" in md
    assert "| dsp | 150 / 150 | 3 | 3 / 0 | 14.50 | 1 |" in md
    assert "| dsp | onto the body: A / k in 1.000 | | | | |" in md


def test_a_hit_3_ms_or_more_after_its_clear_attack_is_late_and_the_previous_hit_clips_the_window():
    x = np.zeros(2 * SR, dtype=np.float32)
    x += 1e-4 * np.random.default_rng(5).standard_normal(len(x)).astype(np.float32)   # a floor
    hit(x, SR, f=1000.0)                         # an attack at 1.0 s
    (d0, l0), = oe.late_flags(x, SR, [1.0])
    assert not l0 and d0 < 0.3, (d0, l0)         # on the attack
    (d5, l5), = oe.late_flags(x, SR, [1.005])
    assert l5 and 4.7 < d5 < 5.3, (d5, l5)       # 5 ms into the body
    (d2, l2), = oe.late_flags(x, SR, [1.002])
    assert not l2 and 1.7 < d2 < 2.3, (d2, l2)   # 2 ms is not late
    assert oe.late_flags(x, SR, [0.99]) == [(0.0, False)]          # before the attack: the floor, not late
    # with a hit 1 ms before, the window starts at the midpoint: the run back to the attack is cut there
    (_, la), (db, lb) = oe.late_flags(x, SR, [1.001, 1.005])
    assert not la and not lb and 1.7 < db < 2.3, (la, lb, db)
    assert oe.late_flags(np.zeros(100, dtype=np.float32), SR, [0.0]) == [(0.0, False)]


def test_a_hit_moved_onto_the_body_is_counted_only_from_a_time_that_was_not_early():
    key = ("A song", "k in", "dsp")
    rows = [{"session": key[0], "name": key[1], "detector": key[2]}]
    times_of = {key + (1,): [1.0, 2.0, 3.0, 4.0], key + (2,): [1.0015, 2.012, 3.0005, 4.0008]}
    flags_of = {key + (1,): ([False, True, False, False], [False] * 4),
                key + (2,): ([False] * 4, [True, True, True, True])}
    o = oe.side_effects(rows, times_of, (1, 2), flags_of)["dsp"]
    # 1.0: moved 1.5 ms onto a late place (counted); 2.0 was early before (a fix); 3.0 and 4.0 moved less than 1 ms
    assert (o["moved_onto_body"], o["onto_body_at"]) == (1, ["A song / k in 1.000"])
    assert (o["moved_1ms"], o["moved_later"], o["moved_earlier"]) == (2, 4, 0)


def test_the_side_effects_count_moves_both_ways_swaps_and_doubles():
    """P4-12 stage 2: rule 2 against rule 3: moves earlier and later by more than 1 ms, moves beyond the 10 ms span, a hit
    that crosses the previous hit's old time (a swap), a new pair under the 20 ms min gap, a changed count."""
    key = ("A song", "s top", "dsp")
    other = ("A song", "t1", "dsp")
    rows = [{"session": key[0], "name": key[1], "detector": key[2]}, {"session": other[0], "name": other[1], "detector": other[2]}]
    times_of = {key + (2,): [1.0, 1.030, 2.0, 3.0, 4.0], key + (3,): [0.9985, 1.0175, 2.0005, 2.988, 4.0],
                other + (2,): [1.0, 2.0], other + (3,): [1.0]}
    o = oe.side_effects(rows, times_of, (2, 3))["dsp"]
    # 1.0 -1.5 ms, 1.030 -12.5 ms (beyond the span, and 19 ms after the previous hit's new time: a new close pair),
    # 2.0 +0.5 ms, 3.0 -12 ms, 4.0 unchanged
    assert (o["earlier_1ms"], o["later_1ms"], o["beyond_span"]) == (3, 0, 2)
    assert (o["moved_earlier"], o["moved_later"]) == (3, 1)
    assert o["new_close"] == 1 and o["crossed"] == 0
    times_of[key + (3,)] = [1.0, 1.030, 2.0, 1.99, 4.0]          # 3.0 moved before 2.0: a swap
    assert oe.side_effects(rows, times_of, (2, 3))["dsp"]["crossed"] == 1
    assert o["count_changed"] == ["A song / t1: 2 -> 1"]
    md = oe.report(oe.table([], (1, 2, 3)), {"2->3": {"dsp": o}}, (1, 2, 3))
    assert "Rule 2->3:" in md and "| dsp | 7 / 6 | 3 | 1 / 3 |" in md and "| 3 / 0 | 2 | 0 | 1 |" in md
