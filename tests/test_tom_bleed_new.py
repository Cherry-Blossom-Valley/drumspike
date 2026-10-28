"""P4-14 stage 1b: the split of a combined tom reference and the checks of `tools/tom_bleed_new.py`. Synthetic signals
only; the sessions are not needed."""
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import tom_bleed as tb  # noqa: E402
import tom_bleed_new as tn  # noqa: E402

SR = 48000


def tom(freq, decay, n=int(0.3 * SR), click=0.0):
    k = np.arange(n) / SR
    y = np.sin(2 * np.pi * freq * k * (1 + 0.15 * np.exp(-k / 0.02))) * np.exp(-k / decay)
    y[:48] += click * np.hanning(96)[48:]
    return y


def combined(times_kinds, samples, length=8.0, pan=None):
    """A stereo sample track: sample `kind` at each time, gain 0.5 to 1 (velocity), each kind with its own pan."""
    rng = np.random.default_rng(7)
    L = np.zeros(int(length * SR))
    R = np.zeros(int(length * SR))
    for t, kind in times_kinds:
        a = int(t * SR)
        y = samples[kind] * rng.uniform(0.5, 1.0)
        n = min(len(y), len(L) - a)
        pl, pr = (pan or {}).get(kind, (1.0, 1.0))
        L[a:a + n] += pl * y[:n]
        R[a:a + n] += pr * y[:n]
    return [L.astype(np.float32), R.astype(np.float32)]


def test_waveform_similarity_is_high_for_one_sample_at_any_gain():
    s = {"hi": tom(130, 0.08, click=0.5), "lo": tom(70, 0.2, click=0.2)}
    chans = combined([(0.5, "hi"), (1.5, "hi"), (2.5, "lo"), (3.5, "lo")], s)
    W, n = tn.waveforms(chans, SR, [0.5, 1.5, 2.5, 3.5])
    sim = tn.similarity(W, n, 2, 48)
    assert sim[0, 1] > 0.99 and sim[2, 3] > 0.99
    assert sim[0, 2] < 0.5
    # a 0.5 ms late onset still finds the same sample (the +-1 ms shift)
    W2, _ = tn.waveforms(chans, SR, [0.5, 1.5005])
    assert tn.similarity(W2, n, 2, 48)[0, 1] > 0.9
    assert (W2 @ W2.T)[0, 1] < tn.similarity(W2, n, 2, 48)[0, 1]


def test_average_linkage_keeps_blocks_apart():
    sim = np.full((6, 6), 0.1)
    sim[:3, :3] = 0.9
    sim[3:5, 3:5] = 0.8
    np.fill_diagonal(sim, 1.0)
    assert tn.average_linkage(sim, 0.5) == [[0, 1, 2], [3, 4], [5]]
    assert tn.average_linkage(sim, 0.05) == [[0, 1, 2, 3, 4, 5]]
    assert tn.average_linkage(np.zeros((0, 0))) == []
    # after a merge the mean to the new cluster counts both members: 0.9 to one, -0.5 to the other is 0.2, too low
    sim = np.array([[1.0, 0.95, 0.9], [0.95, 1.0, -0.5], [0.9, -0.5, 1.0]])
    assert tn.average_linkage(sim, 0.4) == [[0, 1], [2]]


def test_major_groups_take_in_the_small_clusters_they_resemble():
    sim = np.full((12, 12), 0.1)
    sim[:6, :6] = 0.9
    sim[6:11, 6:11] = 0.9
    sim[11, :6] = sim[:6, 11] = 0.6            # hit 11: a small cluster like the first group
    np.fill_diagonal(sim, 1.0)
    clusters = [[0, 1, 2, 3, 4, 5], [6, 7, 8, 9, 10], [11]]
    assert tn.major_groups(clusters, sim, share=0.05, min_hits=5) == [[0, 1, 2, 3, 4, 5, 11], [6, 7, 8, 9, 10]]
    assert tn.major_groups(clusters, sim, share=0.5, min_hits=5) == [list(range(12))]


def test_vote_names_a_whole_group_by_its_majority():
    loudest = ["t1", "t1", "t2", "t3", "t3", "t3", "t2"]
    assert tn.vote_names([[0, 1, 2], [3, 4, 5, 6]], loudest) == [("t1", 2 / 3), ("t3", 0.75)]
    assert tn.vote_names([[2, 6]], loudest) == [("t2", 1.0)]


def test_hit_pitch_and_the_group_pitch_of_isolated_hits():
    x = np.zeros(4 * SR)
    for t, f in ((0.5, 120.0), (1.5, 80.0), (2.5, 80.0), (2.53, 200.0)):
        a = int(t * SR)
        x[a:a + int(0.3 * SR)] += tom(f, 0.2)
    on = np.array([0.5, 1.5, 2.5, 2.53])
    p = tn.hit_pitch(x, SR, on)
    assert abs(p[0] - 120) < 3 and abs(p[1] - 80) < 3
    assert tn.isolated(on).tolist() == [True, True, False, True]
    hz, _ = tn.group_pitch([1, 2], p, tn.isolated(on))
    assert abs(hz - 80) < 3                                   # hit 2 is cut short by hit 3: not used
    hz, p1090 = tn.group_pitch([0, 1, 2], np.array([80.0, 200.0, 200.0]), np.array([True, False, False]))
    assert hz == 80.0 and p1090[1] == 200.0


def test_split_hits_groups_by_sample_and_names_by_the_vote():
    rng = np.random.default_rng(3)
    s = {"t1": tom(140, 0.06, click=0.6), "t2": tom(100, 0.1, click=0.4), "t3": tom(70, 0.2, click=0.2)}
    kinds = list(rng.choice(["t1", "t2", "t3"], size=36))
    times = [0.3 + 0.21 * i for i in range(36)]
    chans = combined(list(zip(times, kinds)), s, pan={"t1": (1.0, 0.4), "t2": (0.8, 0.8), "t3": (0.4, 1.0)})
    # a noisy vote: one hit in six names a wrong mic; whole groups still get the right name
    loudest = [k if i % 6 else {"t1": "t2", "t2": "t3", "t3": "t1"}[k] for i, k in enumerate(kinds)]
    split, det = tn.split_hits(chans, SR, np.array(times), loudest)
    for m in ("t1", "t2", "t3"):
        assert split[m] == [t for t, k in zip(times, kinds) if k == m]
    assert split["t4"] == []
    assert sorted(g["mic"] for g in det["groups"]) == ["t1", "t2", "t3"]
    hz = {g["mic"]: g["pitch_hz"] for g in det["groups"]}
    assert hz["t1"] > hz["t2"] > hz["t3"]


def test_split_accuracy_counts_each_true_tom_by_its_name():
    truth = {"t1": [1.0, 2.0], "t2": [3.0]}
    split = {"t1": [1.001], "t2": [2.0, 3.004]}
    assert tn.split_accuracy(truth, split) == {"t1": {"t1": 1, "t2": 1}, "t2": {"t2": 1}}
    assert tn.split_accuracy({"t1": [5.0]}, split) == {"t1": {None: 1}}


def test_union_check_counts_a_hit_lost_on_every_mic():
    def row(mic, t, level):
        return {"session": "S", "mic": mic, "t": t, "offset_ms": 2.0, "level_db": level, "tom_rel_db": -10.0}

    rows = [row("t1", 1.002, 0.0), row("t2", 1.004, -20.0),        # kept on t1
            row("t2", 2.002, -20.0), row("t1", 2.003, -20.0),      # gone from both
            row("t1", 3.002, -20.0)]
    refs = {"S": {"toms_combined": [1.0, 2.0, 3.0, 4.0]}}
    u = tn.union_check(rows, refs, tn.RULE_B)
    assert u == {"S": {"refs": 4, "found_before": 3, "kept_after": 1, "lost": 2}}


def test_sessions_use_tom_close_mics_and_the_stage_1_rule():
    for s, cfg in tn.SESSIONS.items():
        toms = [v[0] for v in cfg["mics"].values() if v[1] == "tom"]
        assert len(toms) == 4 and all(tb.is_close_tom(t) for t in toms), s
        assert not any(tb.is_close_tom(st) for st in sum(cfg["toms_ref"].get("per_tom", {}).values(), [])
                       + cfg["toms_ref"].get("combined", []))
    assert tn.RULE_B == [("level_db", -5.267), ("tom_rel_db", 2.146)]


def test_cluster_purity_counts_the_majority_tom_of_each_cluster():
    truth = {"t1": [1.0, 2.0, 3.0], "t2": [4.0, 5.0]}
    on = [1.001, 2.0, 3.0, 4.002, 5.0, 9.0]
    assert tn.cluster_purity([[0, 1, 3], [2, 4], [5]], on, truth) == (3, 5)
    assert tn.cluster_purity([[0, 1, 2], [3, 4, 5]], on, truth) == (5, 5)
