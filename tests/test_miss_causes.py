"""P4-10 stage 1: the miss classifier (`tools/miss_causes.py`). The replay of the peak rules gives the frames that the Rust
tests of `peaks.rs` pin; every cause is made by a planted condition in a synthetic activation; the retry of a track
without hits, the dry-run variants, the waveform second-onset search and the tables are checked on small inputs. One end to
end case runs the real app chain on a short file and compares the replay with the app's events (skipped without
`service_tool`)."""
import csv
import json
import struct
import sys
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import miss_causes as mc  # noqa: E402
from core_tools import SERVICE_TOOL, require  # noqa: E402


def col(n, base, bumps):
    x = np.full(n, base, dtype=np.float32)
    for f, v in bumps:
        x[f] = v
    return x


def act_of(n, **cols):
    """A (n, 5) activation: `bd`, `sd`, `tt`, `hh`, `cy` are the columns (arrays or None for zeros)."""
    a = np.zeros((n, 5), dtype=np.float32)
    for i, name in enumerate(["bd", "sd", "tt", "hh", "cy"]):
        if name in cols:
            a[:, i] = cols[name]
    return a


# ---- the replay of the peak rules: the inputs and results of the Rust tests in peaks.rs

def test_replay_gives_the_frames_of_the_rust_tests():
    combine = col(80, 0.0, [(30, 0.8), (32, 0.9), (50, 0.5), (52, 0.6), (54, 0.75)])
    assert mc.stages(combine, 0.22)["kept"] == [30, 50, 54]
    assert mc.stages(combine, 0.22)["peaks"] == [30, 32, 50, 52, 54]
    assert mc.stages(combine, 0.22)["merged"] == {32: 30, 52: 50}
    assert mc.stages(col(40, 0.0, [(20, 0.7), (21, 0.7)]), 0.22)["kept"] == [20]
    assert mc.stages(col(40, 0.0, [(0, 3.0)]), 0.22)["kept"] == []                  # frame 0 alone: dropped
    assert mc.stages(col(40, 0.0, [(0, 0.9), (20, 0.9)]), 0.22)["kept"] == [0, 20]
    assert mc.stages(col(30, 0.0, [(1, 0.8), (29, 0.8)]), 0.22)["kept"] == [1, 29]
    y = np.zeros(30, dtype=np.float32)
    y[0], y[1] = 0.5, 0.6
    assert mc.stages(y, 0.22)["kept"] == [1]                                        # madmom zero padding
    x = col(60, 0.01, [(20, 0.9), (40, 0.3)])
    assert mc.stages(x, 0.32)["kept"] == [20] and mc.stages(x, 0.22)["kept"] == [20, 40]


def test_moving_average_is_the_mean_over_t_minus_10_to_t_plus_1_with_zero_padding():
    x = col(30, 0.0, [(0, 1.2), (29, 2.4)])
    avg = mc.moving_average(x)
    assert abs(avg[0] - 0.1) < 1e-6 and abs(avg[29] - 0.2) < 1e-6 and avg[15] == 0.0 and abs(avg[28] - 0.2) < 1e-6
    brute = [sum(x[j] for j in range(t - 10, t + 2) if 0 <= j < 30) / 12 for t in range(30)]
    assert np.allclose(avg, brute)


def test_changed_rules_change_the_picks():
    x = col(60, 0.0, [(20, 0.9), (26, 0.3)])
    assert mc.stages(x, 0.22)["kept"] == [20]                                       # the 26 sits under avg + thr
    assert mc.stages(x, 0.22, {"pre_avg": 3})["kept"] == [20, 26]
    assert mc.stages(x, 0.22, {"avg_off": True})["kept"] == [20, 26]
    two = col(60, 0.0, [(20, 0.8), (22, 0.9)])
    assert mc.stages(two, 0.22)["kept"] == [20] and mc.stages(two, 0.22, {"combine_s": 0.01})["kept"] == [20, 22]
    assert mc.stages(col(60, 0.0, [(20, 0.5), (21, 0.9)]), 0.22, {"pre_max": 0, "post_max": 0})["kept"] == [20]


def test_read_activation_checks_the_header_and_the_size(tmp_path):
    a = np.arange(30, dtype="<f4").reshape(6, 5) / 30
    p = tmp_path / "act.bin"
    p.write_bytes(b"DSACT\0\0\0" + struct.pack("<IIQ", 1, 5, 6) + a.tobytes())
    assert np.array_equal(mc.read_activation(p), a)
    p.write_bytes(b"DSACT\0\0\0" + struct.pack("<IIQ", 1, 5, 7) + a.tobytes())
    with pytest.raises(ValueError):
        mc.read_activation(p)
    for version, classes in ((2, 5), (1, 4)):
        p.write_bytes(b"DSACT\0\0\0" + struct.pack("<IIQ", version, classes, 6) + a.tobytes())
        with pytest.raises(ValueError):
            mc.read_activation(p)
    p.write_bytes(b"XSACT\0\0\0" + struct.pack("<IIQ", 1, 5, 6) + a.tobytes())
    with pytest.raises(ValueError):
        mc.read_activation(p)


def test_the_retry_of_a_track_without_hits_runs_the_steps_of_the_app():
    sd = col(100, 0.0, [(30, 0.15), (60, 0.15)])
    act = act_of(100, sd=sd)
    picks, _, th, retry = mc.effective_picks(act, "snare")
    assert retry == 95.0 and picks == [(30, 1), (60, 1)] and th[1] < 0.24
    picks, _, th, retry = mc.effective_picks(act_of(100, sd=col(100, 0.0, [(30, 0.5)])), "snare")
    assert retry is None and picks == [(30, 1)] and th == mc.THRESHOLDS
    picks, _, _, retry = mc.effective_picks(act_of(100), "snare")
    assert retry is None and picks == []                                              # nothing at any step
    assert abs(mc.scale_of(100.0) - 0.4) < 1e-12 and abs(mc.thresholds_of(150.0)[0] - 0.01) < 1e-12


def test_role_dedup_keeps_the_role_class_and_the_minimum_gap():
    picks = [(10, 1), (11, 0), (12, 1), (15, 1), (16, 3)]
    kept, dropped = mc.role_dedup(picks, "snare")
    assert kept == [10 / 100, 15 / 100] and dropped == [12 / 100]
    assert mc.role_dedup(picks, "snare", min_gap=0.001)[0] == [10 / 100, 12 / 100, 15 / 100]
    kept, dropped = mc.role_dedup(picks, "snare", classes=[0, 1, 3])
    assert kept == [10 / 100, 15 / 100] and dropped == [11 / 100, 12 / 100, 16 / 100]
    assert mc.role_dedup(picks, "kick")[0] == [11 / 100]


# ---- one cause per miss, each by a planted condition

def classify(act, role, t, events=(), claimed=(), thresholds=None):
    picks, st, _, _ = mc.effective_picks(act, role, thresholds)
    cause, detail = mc.classify_miss(t, role, act, st, picks, list(events), claimed)
    return cause, detail


def test_blind_when_the_column_never_rises_and_threshold_when_it_rises_a_little():
    n = 100
    assert classify(act_of(n), "snare", 0.5)[0] == "blind"
    assert classify(act_of(n, sd=col(n, 0.0, [(48, 0.01)])), "snare", 0.5)[0] == "blind"
    c, d = classify(act_of(n, sd=col(n, 0.0, [(48, 0.12), (80, 0.9)])), "snare", 0.5)
    assert c == "threshold" and abs(d["peak"] - 0.12) < 1e-6


def test_average_when_a_loud_neighbour_lifted_the_moving_average():
    n = 120
    sd = col(n, 0.0, [(40, 0.95), (46, 0.35), (47, 0.30)])        # 46 passes 0.24 but not avg + 0.24
    act = act_of(n, sd=sd)
    st = mc.stages(sd, 0.24)
    assert st["pass_thr"][46] and st["det"][46] == 0.0 and st["kept"] == [40]
    c, _ = classify(act, "snare", 0.47, claimed=[40])
    assert c == "average"


def test_not_max_when_a_larger_neighbour_just_outside_the_window_wins():
    n = 120
    t = 0.497                                                        # the window is frames 46 to 51
    sd = col(n, 0.0, [(51, 0.5), (52, 0.9)])
    act = act_of(n, sd=sd)
    st = mc.stages(sd, 0.24)
    assert st["det"][51] != 0 and 51 not in st["peaks"] and 52 in st["peaks"]
    assert mc.window_frames(t, n) == (46, 51)
    assert classify(act, "snare", t)[0] == "not_max"


def test_combine_when_the_peak_was_merged_into_a_found_neighbour():
    n = 120
    sd = col(n, 0.0, [(40, 0.8), (42, 0.9)])
    act = act_of(n, sd=sd)
    assert mc.stages(sd, 0.24)["merged"] == {42: 40}
    assert classify(act, "snare", 0.447, claimed=[40])[0] == "combine"


def test_role_when_another_class_fired_and_the_own_one_did_not():
    n = 100
    act = act_of(n, hh=col(n, 0.0, [(48, 0.8)]), sd=col(n, 0.0, [(48, 0.1), (80, 0.9)]))
    c, d = classify(act, "snare", 0.5)
    assert c == "role" and d["own"] == "threshold"
    assert classify(act, "kick", 0.5)[0] == "role"


def test_lobe_when_the_hit_sits_in_a_found_neighbours_lobe_and_has_no_maximum_of_its_own():
    n = 120
    sd = np.zeros(n, np.float32)
    sd[39:46] = [0.3, 0.8, 0.75, 0.2, 0.1, 0.05, 0.01]             # one lobe, the second hit has no bump
    act = act_of(n, sd=sd)
    assert mc.stages(sd, 0.24)["kept"] == [40]
    assert classify(act, "snare", 0.45, claimed=[40])[0] == "lobe"
    # the same decay with a second maximum of its own, under the threshold: not a lobe
    sd2 = sd.copy()
    sd2[43:47] = [0.05, 0.10, 0.14, 0.04]
    c, _ = classify(act_of(n, sd=sd2), "snare", 0.45, claimed=[40])
    assert c == "threshold"
    assert classify(act, "snare", 0.45)[0] != "lobe"                  # no found neighbour: no lobe


def test_later_stages_offset_refine_and_the_exclusion_of_a_found_neighbour():
    n = 100
    act = act_of(n, sd=col(n, 0.0, [(40, 0.9)]))
    t = 0.42
    assert classify(act, "snare", t)[0] == "refine"                    # picked at frame 40, no event
    c, d = classify(act, "snare", t, events=[0.46])
    assert c == "offset" and abs(d["event_off_ms"] - 40.0) < 1e-6
    # an event 160 ms away is no evidence
    assert classify(act, "snare", t, events=[0.58])[0] == "refine"
    # the found neighbour's own pick is not evidence for this hit: with it claimed, nothing of its own is left
    assert classify(act, "snare", t, claimed=[40])[0] == "lobe"


def test_classify_item_marks_found_and_missed_and_counts_false_and_bleed():
    n = 300
    sd = col(n, 0.0, [(90, 0.9), (190, 0.9), (250, 0.9)])          # hits at 1.0 s and 2.0 s, a false one at 2.6 s
    act = act_of(n, sd=sd)
    item = {"id": "x/y", "set": "drt", "file": "y", "role": "snare", "gt": [
        {"t": 1.0, "group": "plain", "level": -6.0, "gap_ms": None, "kind": "target"},
        {"t": 1.5, "group": "ghost", "level": -30.0, "gap_ms": None, "kind": "target_ghost"},
        {"t": 2.0, "group": "plain", "level": -6.0, "gap_ms": None, "kind": "target"},
        {"t": 1.0, "group": "bleed", "level": -20.0, "gap_ms": None, "kind": "bleed_hat"},
        {"t": 2.6, "group": "bleed", "level": -20.0, "gap_ms": None, "kind": "bleed_hat"},
        {"t": 2.9, "group": "bleed", "level": -20.0, "gap_ms": None, "kind": "bleed_hat"}]}
    events = [1.0, 2.0, 2.6]
    rows, info = mc.classify_item(item, act, events, dsp_events=[1.0, 1.5])
    assert [r["found"] for r in rows] == [True, False, True]
    assert rows[1]["cause"] == "blind" and rows[1]["dsp_found"] is True and rows[0]["err_ms"] == 0.0
    # the event at 1.0 s belongs to a target; only the free event at 2.6 s is a bleed trigger
    assert info == {"retry": None, "false": 1, "n_events": 3, "bleed_total": 3, "bleed_hit": 1}


def test_a_dsp_event_that_belongs_to_another_hit_does_not_count_as_found():
    n = 300
    act = act_of(n, sd=col(n, 0.0, [(90, 0.9)]))
    item = {"id": "x/z", "set": "drt", "file": "z", "role": "snare", "gt": [
        {"t": 1.0, "group": "pair_first", "level": -6.0, "gap_ms": None, "kind": "target_first"},
        {"t": 1.006, "group": "pair_second", "level": -6.0, "gap_ms": 6, "kind": "target_second_6ms"}]}
    rows, _ = mc.classify_item(item, act, [1.0], dsp_events=[1.0])
    assert rows[0]["found"] and not rows[1]["found"] and rows[1]["dsp_found"] is False


# ---- the items from the kits

def test_drt_items_read_the_ground_truth_columns(tmp_path):
    with open(tmp_path / "ground_truth.csv", "w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["file", "onset_sample", "onset_s", "kind", "level_dbfs", "role"])
        w.writerows([["03_flams.wav", 48000, 1.0, "snare", -12, "target_grace"],
                     ["03_flams.wav", 48240, 1.005, "snare", -6, "target_main"],
                     ["05_snare_hat_bleed.wav", 48000, 1.0, "snare", -6, "target"],
                     ["05_snare_hat_bleed.wav", 49000, 1.02, "hat", -20, "bleed_hat"],
                     ["07_kick_velocity_ramp.wav", 48000, 1.0, "kick", -10, "target"],
                     ["09_close_hits_min_separation.wav", 48000, 1.0, "snare", -12, "target_first"],
                     ["09_close_hits_min_separation.wav", 48192, 1.004, "snare", -12, "target_second_4ms"]])
    items = {i["file"]: i for i in mc.drt_items(tmp_path)}
    assert [g["group"] for g in items["03_flams"]["gt"]] == ["flam_grace", "flam_main"]
    assert [g["group"] for g in items["05_snare_hat_bleed"]["gt"]] == ["plain", "bleed"]
    assert items["07_kick_velocity_ramp"]["role"] == "kick" and items["07_kick_velocity_ramp"]["note"] == 36
    assert items["03_flams"]["role"] == "snare" and items["03_flams"]["note"] == 38
    assert [g["gap_ms"] for g in items["09_close_hits_min_separation"]["gt"]] == [None, 4]
    assert items["09_close_hits_min_separation"]["gt"][1]["group"] == "pair_second"


def test_calib_items_mark_the_second_hit_of_a_pair(tmp_path):
    (tmp_path / "audio" / "syn" / "clean").mkdir(parents=True)
    (tmp_path / "ref" / "syn" / "clean").mkdir(parents=True)
    (tmp_path / "audio" / "syn" / "clean" / "snare_48000.wav").write_bytes(b"")
    (tmp_path / "audio" / "syn" / "clean" / "snare_44100.wav").write_bytes(b"")
    with open(tmp_path / "ref" / "syn" / "clean" / "snare_48000.csv", "w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["abs_s", "group", "level_db", "pair"])
        w.writerows([[0.5, "main", -20.0, ""], [1.0, "pair", -6.0, 0], [1.03, "pair", -14.0, 0]])
    items = mc.calib_items(tmp_path)
    assert len(items) == 1 and items[0]["id"] == "syn/clean/snare_48000" and items[0]["role"] == "snare"
    assert [(g["group"], g["gap_ms"]) for g in items[0]["gt"]] == [("main", None), ("pair_first", None), ("pair_second", 30)]


# ---- the dry run

def one_item(role, gt):
    return {"id": "d/e", "set": "drt", "file": "e", "role": role, "gt": [
        {"t": t, "group": g, "level": -6.0, "gap_ms": None, "kind": g} for t, g in gt]}


def test_score_events_counts_found_by_group_false_and_bleed_triggers():
    item = one_item("snare", [(1.0, "plain"), (2.0, "ghost"), (3.0, "bleed")])
    found, false, trig = mc.score_events(item, [1.002, 3.001, 4.0])
    assert found == {"plain": 1} and false == 2 and trig == 1


def test_variants_change_what_is_found_and_the_dsp_rescue_adds_events_far_from_picks():
    n = 300
    # a loud hit at 1.0 s and a quiet one at 2.0 s that the threshold stops; the hat class fires on a ghost at 2.5 s
    act = act_of(n, sd=col(n, 0.0, [(99, 0.9), (199, 0.15), (249, 0.08)]), hh=col(n, 0.0, [(249, 0.8)]))
    item = one_item("snare", [(1.0, "plain"), (2.0, "plain"), (2.5, "ghost")])
    base = mc.variant_events(item, act, [], mc.VARIANTS["base"])
    assert len(base) == 1 and abs(base[0] - 1.0) < 1e-9
    assert len(mc.variant_events(item, act, [], mc.VARIANTS["thr50"])) == 2
    cross = mc.variant_events(item, act, [], {"cross_min": 0.05})
    assert len(cross) == 2 and abs(cross[1] - 2.5) < 1e-9
    assert len(mc.variant_events(item, act, [], mc.VARIANTS["role_any"])) == 2
    assert len(mc.variant_events(item, act, [], {"cross_min": 0.5})) == 1       # the own column is too low there
    with_dsp = mc.variant_events(item, act, [1.0, 2.003], {"dsp_far": 0.03})
    assert len(with_dsp) == 2 and abs(with_dsp[1] - 2.003) < 1e-9               # the DSP hit at 1.0 s is a duplicate
    assert len(mc.variant_events(item, act, [2.003], {"dsp_far": 0.03, "dsp_roles": ["kick"]})) == 1


def test_dry_run_sums_found_and_false_over_files():
    n = 300
    act = act_of(n, sd=col(n, 0.0, [(99, 0.9), (199, 0.15)]))
    item = one_item("snare", [(1.0, "plain"), (2.0, "plain")])
    data = [(item, act, [1.0], [])]
    res = mc.dry_run(data, {"base": {}, "thr50": {"thr_scale": 0.5}})
    assert res["base"]["found"] == {"drt/plain": 1} and res["thr50"]["found"] == {"drt/plain": 2}
    assert res["base"]["targets"] == 2 and res["base"]["false"] == {"drt/clean": 0}


def test_file_lead_is_the_median_distance_from_the_pick_to_its_event():
    n = 300
    act = act_of(n, sd=col(n, 0.0, [(99, 0.9), (199, 0.9)]))
    item = one_item("snare", [(1.0, "plain"), (2.0, "plain")])
    assert abs(mc.file_lead(item, act, [0.99 + 0.006, 1.99 + 0.010]) - 0.008) < 1e-9
    assert mc.file_lead(item, act, [5.0]) == mc.LEAD_S


# ---- the waveform second-onset search

def burst(n, sr, start, length_s, level, seed):
    rng = np.random.default_rng(seed)
    x = np.zeros(n)
    k = int(length_s * sr)
    env = np.exp(-np.arange(k) / (0.02 * sr))
    x[int(start * sr):int(start * sr) + k] = rng.standard_normal(k) * env * level
    return x


def test_step_onsets_finds_a_second_burst_and_ignores_a_plain_decay():
    sr = 48000
    n = int(0.5 * sr)
    one = burst(n, sr, 0.1, 0.2, 0.3, 1)
    two = one + burst(n, sr, 0.130, 0.2, 0.6, 2)
    assert mc.step_onsets(one, sr, [0.1]) == []
    extra = mc.step_onsets(two, sr, [0.1])
    assert len(extra) == 1 and abs(extra[0] - 0.130) < 0.004
    # a second event in the way stops the search before it
    assert mc.step_onsets(two, sr, [0.1, 0.128]) == []
    # a step under the rise threshold is not an onset
    weak = one + burst(n, sr, 0.130, 0.2, 0.05, 3)
    assert mc.step_onsets(weak, sr, [0.1]) == []


# ---- the tables

def test_level_bin_cause_cell_and_the_cause_table_add_up():
    assert [mc.level_bin(x) for x in (-6.0, -20.0, -35.0, 0.0, -0.5)] == [
        "-10 to 0", "-30 to -20", "-40 to -30", "-10 to 0", "-10 to 0"]
    assert mc.cause_cell(0, 0) == "-" and mc.cause_cell(3, 0) == "3" and mc.cause_cell(3, 2) == "3 (2)"
    rows = [{"file": "a", "found": True, "cause": "", "dsp_found": False},
            {"file": "a", "found": False, "cause": "blind", "dsp_found": True},
            {"file": "a", "found": False, "cause": "lobe", "dsp_found": False},
            {"file": "b", "found": False, "cause": "blind", "dsp_found": False}]
    t = mc.cause_table(rows, lambda r: r["file"], "file").splitlines()
    head = [c.strip() for c in t[0].strip("|").split("|")]
    last = [c.strip() for c in t[-1].strip("|").split("|")]
    assert last[0] == "**all**" and last[1] == "4" and last[2] == "1"
    assert last[head.index("blind")] == "2 (1)" and last[head.index("lobe")] == "1"
    assert sum(int(c.split()[0]) for c in last[3:] if c != "-") == 3          # the misses add up


def test_the_cause_texts_cover_every_cause():
    assert set(mc.CAUSES) == set(mc.CAUSE_TEXT)


def test_gained_item_writes_a_float_copy_with_the_gain_and_keeps_the_ground_truth(tmp_path):
    sr = 48000
    x = np.zeros(sr)
    x[1000] = 0.1
    src = tmp_path / "in" / "snare_48000.wav"
    src.parent.mkdir()
    sf.write(src, x, sr, subtype="PCM_24")
    item = {"id": "syn/clean/snare_48000", "audio": str(src), "gt": [1], "role": "snare"}
    g = mc.gained_item(tmp_path, item, 20)
    y, sr2 = sf.read(g["audio"], dtype="float32")
    assert sr2 == sr and abs(float(y[1000]) - 1.0) < 1e-4 and g["id"] == "gain20/syn/clean/snare_48000" and g["gt"] == [1]
    assert Path(g["audio"]).name == "snare_48000.wav"                       # the track name the app derives from it


def test_tables_md_has_the_sections_and_the_dry_run_blocks(tmp_path):
    n = 300
    act = act_of(n, sd=col(n, 0.0, [(99, 0.9)]))
    item = {"id": "drt/x", "set": "drt", "file": "x", "role": "snare", "gt": [
        {"t": 1.0, "group": "plain", "level": -6.0, "gap_ms": None, "kind": "target"},
        {"t": 2.0, "group": "plain", "level": -6.0, "gap_ms": None, "kind": "target"}]}
    data = [(item, act, [1.0], [])]
    rows, _ = mc.classify_all(data)
    dry = {"peak": mc.dry_run(data, {"base": {}}), "gain": {}, "split": {}}
    md = mc.tables_md(rows, data, dry)
    assert "## Owner's DRT kit, per file" in md and "## Replay check" in md and "Dry run on the kept activations" in md
    assert "in 1 of 1 files" in md and "| x | 2 | 1 |" in md
    assert "Gain ahead of the model" not in md                               # an empty block is left out
    mc.write_rows(tmp_path, rows)
    assert json.loads((tmp_path / "causes.json").read_text())[1]["cause"] == "blind"
    assert (tmp_path / "causes.csv").read_text().splitlines()[0].startswith("id,set,file")
