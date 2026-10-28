"""P4-23: the snare onset diagnosis (`tools/snare_onset.py`). Synthetic signals and events only; no session file and no HOME
content needed (run with HOME pointed at an empty folder)."""
import json
import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import firm_fixes as ff  # noqa: E402
import fix_metric as fm  # noqa: E402
import kick_onset as ko  # noqa: E402
import snare_onset as so  # noqa: E402

SR = 48000


# ---------------------------------------------------------------- synthetic snare tracks

def snare_track(times, dur, amp=0.5, floor=1e-4, seed=1):
    """A snare mic: each hit is a sharp noise burst (1 ms rise) that decays in 80 ms."""
    rng = np.random.default_rng(seed)
    x = rng.normal(0, floor, int(dur * SR))
    for t in times:
        i = int(round(t * SR))
        n = int(0.15 * SR)
        k = np.arange(n) / SR
        env = np.minimum(1.0, k / 0.001) * np.exp(-k / 0.03)
        x[i:i + n] += amp * env * rng.normal(0, 1, n)
    return x


TIMES = [0.5 + 0.35 * k for k in range(30)]


def run_track(shift_ms=0.0, names=("s trigger", "s samp 1", "s samp 2")):
    """An analysis of a 30-hit track: events on the foot, a version 2 time 2 ms later, a trigger file and two sample files at
    a latency; `shift_ms` moves the second sample file (a loose file)."""
    x = snare_track(TIMES, 11.5)
    ev = [{"t": t, "v": 100, "rise": 20.0} for t in TIMES]
    ev[3]["roll"] = True
    ev2 = [{"t": t + 0.002, "v": 100, "rise": 20.0} for t in TIMES]
    files = {names[0]: [t + 0.006 for t in TIMES], names[1]: [t + 0.0072 for t in TIMES],
             names[2]: [t + 0.006 + (shift_ms / 1000.0 if k % 3 == 0 else 0.0) for k, t in enumerate(TIMES)]}
    return so.analyze_one("syn", files[names[0]], ev, ev2, {"s bot": [{"t": t + 0.001} for t in TIMES]}, x, SR, files, None,
                          yard=[names[0]], log=lambda *_: None)


# ---------------------------------------------------------------- sources and constants

def test_every_yardstick_file_is_one_of_the_files_of_the_song():
    for song, v in so.FILES.items():
        assert set(v["yard"]) <= set(v["files"]), song
        assert not set(v.get("extra", [])) & set(v["files"]), song


def test_the_fit_and_held_out_songs_split_the_seven_songs():
    assert set(so.FIT) | set(so.HELD_SONGS) == set(so.SONGS) and not set(so.FIT) & set(so.HELD_SONGS)
    assert len(so.SONGS) == 7 and set(so.FILES) == set(so.SONGS)


def test_the_held_out_sessions_have_three_files_and_two_snare_mics():
    for name, h in so.HELD.items():
        assert len(h["files"]) == 3 and h["top"] and h["bot"], name
    cfg, stems, files, yard = so.session_cfg("Session E")
    assert stems == {"s top": "s top-AtA2", "s bot": "s btm-AtA2"} and yard == files[:1]


def test_files_of_is_what_the_yardstick_reads_for_the_fair_choice(monkeypatch):
    assert fm.ref_files("Session A", "snare") == so.files_of("Session A") == so.FILES["Session A"]["files"]
    assert fm.ref_files("Session A", "kick") == ko.FILES["Session A"]["files"]


# ---------------------------------------------------------------- pure parts

def test_roll_flags_follow_the_output_order_and_skip_flagged_events():
    ev = [{"t": 1.0}, {"t": 2.0, "roll": True}, {"t": 3.0, "roll": True, "bleed": True}, {"t": 4.0, "cut": True}, {"t": 5.0, "roll": True}]
    assert so.roll_flags(ev) == [False, True, True]


def test_same_events_needs_the_same_count_and_the_same_times():
    a = [{"t": 1.0}, {"t": 2.0}, {"t": 3.0}]
    assert so.same_events(a, [{"t": 1.0}, {"t": 2.0000001}, {"t": 3.1}], tol=1e-6) == pytest.approx(2 / 3)
    assert so.same_events(a, a[:2]) == 0.0 and so.same_events([], []) == 0.0
    assert so.same_events(a, a) == 1.0


def test_size_roll_splits_the_moved_hits_by_the_roll_flag_and_skips_the_window():
    rows = [{"err": 0.0015, "roll": True}, {"err": -0.0040, "roll": True}, {"err": 0.0015, "roll": False},
            {"err": 0.0010, "roll": True}, {"err": 0.0005, "roll": False}]
    out = so.size_roll(rows)
    assert out["roll"] == [0.0015, -0.004] and out["normal"] == [0.0015]


def test_side_class3_app_when_no_file_agrees():
    assert so.side_class3({"a": 0.002, "b": -0.003, "c": 0.0015}, ["a"]) == "app"


def test_side_class3_ref_file_when_every_other_file_agrees_and_the_yardstick_has_one_file():
    assert so.side_class3({"a": 0.002, "b": 0.0004, "c": -0.0008}, ["a"]) == "ref_file"
    assert so.side_class3({"a": 0.002, "b": 0.0004, "c": 0.003}, ["a"]) == "mixed"         # only one of two others agrees
    assert so.side_class3({"a": 0.0004, "b": 0.0004}, ["a", "b"]) == "mixed"               # a union yardstick never says ref_file


def test_side_class3_ignores_a_file_without_the_hit_and_knows_the_unknown():
    assert so.side_class3({"a": 0.002, "b": None, "c": 0.0005}, ["a"]) == "ref_file"
    assert so.side_class3({"a": None, "b": None}, ["a"]) == "unknown"
    assert so.side_class3({"a": 0.001, "b": 0.0011}, ["a"]) == "mixed"                      # 1.0 ms agrees, 1.1 ms does not


def test_bot_class_prefers_the_reference_over_the_top_mic():
    assert so.bot_class(0.0005, 0.0003) == "ref"            # both agree: the reference wins
    assert so.bot_class(0.002, 0.0005) == "top"
    assert so.bot_class(0.002, 0.003) == "neither"
    assert so.bot_class(None, 0.0) == "none" and so.bot_class(0.0, None) == "none"


def test_bot_errors_take_off_the_constant_latency_of_the_bottom_mic():
    est = [1.0, 2.0, 3.0]
    shifted = [1.0, 2.0, 3.0]
    bot = [1.0 + 0.0041, 2.0 + 0.0040, 3.0 + 0.0030, 9.0]    # a 4 ms latency; the third hit is 1 ms early
    out = so.bot_errors(est, shifted, [(0, 0), (1, 1), (2, 2)], bot)
    assert out[0][0] == pytest.approx(0.0001) and out[2][0] == pytest.approx(-0.001) and out[2][1] == pytest.approx(-0.001)
    assert so.bot_errors(est, shifted, [(0, 0)], []) == {} and so.bot_errors(est, shifted, [], bot) == {}


def test_bot_errors_leave_out_a_hit_the_bottom_mic_lacks():
    out = so.bot_errors([1.0, 2.0, 3.0], [1.0, 2.0, 3.0], [(0, 0), (1, 1), (2, 2)], [1.0, 3.0])
    assert set(out) == {0, 2}


def test_gate_counts_drop_only_hits_under_the_limit():
    rises = {0: 5.0, 1: 9.9, 2: 10.0, 3: 30.0, 4: 3.0}
    g = so.gate_counts(rises, matched={0, 1, 3}, moved={1}, confirmed={4}, false={2, 4}, min_db=10.0)
    assert g == {"false": 1, "confirmed": 1, "matched": 2, "moved": 1}          # 10.0 stays; 9.9 goes


def test_gate_counts_count_a_confirmed_stroke_inside_the_false_hits_only():
    g = so.gate_counts({0: 1.0, 1: 1.0}, matched={0}, moved=set(), confirmed={1, 7}, false={1}, min_db=10.0)
    assert g == {"false": 1, "confirmed": 1, "matched": 1, "moved": 0}


def test_bin_of_takes_the_lower_edge_and_leaves_the_upper():
    bins = [("a", -1e9, 0.0), ("b", 0.0, 10.0), ("c", 10.0, 1e9)]
    assert [so.bin_of(v, bins) for v in (-5.0, 0.0, 9.99, 10.0, 99.0)] == ["a", "b", "b", "c", "c"]


def test_moved_share_counts_hits_and_moved_hits_by_the_feature():
    rows = [{"err": 0.002, "x": 1.0}, {"err": 0.0001, "x": 2.0}, {"err": -0.003, "x": 11.0}, {"err": 0.0, "x": 12.0}]
    ms = so.moved_share(rows, [("lo", -1e9, 10.0), ("hi", 10.0, 1e9)], lambda r: r["x"])
    assert ms == {"lo": (2, 1), "hi": (2, 1)}


def test_false_class_calls_a_kick_bleed_by_its_own_name():
    f = so.false_class
    assert f(False, False, 0.005, 0.5, 20.0, 0.5) == "kick_ref"
    assert f(True, False, 0.005, 0.5, 20.0, 0.5) == "layer"                      # a layer confirms before the kick test
    assert f(False, True, 0.5, 0.5, 20.0, 0.5) == "witness"
    assert f(False, True, 0.005, 0.5, 20.0, 0.5) == "kick_ref"                    # kick bleed is no witnessed stroke
    assert f(False, False, 0.5, 0.030, 20.0, 0.5) == "double"
    assert f(False, False, 0.5, 0.5, 4.0, 0.5) == "no_stroke"
    assert f(False, False, 0.5, 0.5, 20.0, 0.5) == "ring"
    assert f(False, False, 0.5, 0.5, 20.0, 5.0) == "real_stroke"


def test_miss_class_takes_silence_before_everything():
    assert so.miss_class(2.0, 0.5, True, "threshold", silent=True) == "silent"
    assert so.miss_class(2.0, 0.5, True, "threshold") == "no_stroke"
    assert so.miss_class(20.0, 0.030, True, "threshold") == "outside_20ms"
    assert so.miss_class(20.0, 0.5, True, "threshold") == "merged"
    assert so.miss_class(20.0, 0.5, False, "threshold") == "threshold" and so.miss_class(20.0, 0.5, False, "offset") == "other_stage"


# ---------------------------------------------------------------- rules (pure)

def fake_result(errs, replay):
    rows = [{"err": e, "file_err": {"f": e}, "rel_db": 0.0, "jump_db": 40.0, "roll": e > 0.0025,
             "replay": dict({n: 0.0 for n in ko.WALKS}, **r)} for e, r in zip(errs, replay)]
    return {"rows": rows, "fair": {"file": "f", "how": "trigger"}}


def test_subset_walks_keep_the_chosen_rows_and_count_both_sides():
    # hit 0 moved by +2 ms: the variant brings it back (right); hit 1 right: the variant moves it out (wrong)
    res = {"A": fake_result([0.002, 0.0, 0.003], [{"peak 25": -0.002}, {"peak 25": 0.002}, {"peak 25": -0.003}])}
    allrows = so.subset_walks(res, lambda r: True, ["A"], [], [])["walk peak 25"]
    assert allrows["all"] == (2, 1, 1, 2)                    # moved 2 -> 1; one right hit made wrong, two wrong made right
    roll = so.subset_walks(res, lambda r: r["roll"], ["A"], [], [])["walk peak 25"]
    assert roll["all"] == (1, 0, 0, 1)                        # only the 3 ms hit is a roll row


def test_subset_walks_leave_out_a_song_without_such_a_row():
    res = {"B": fake_result([0.0], [{}]), "A": fake_result([0.002], [{}])}      # the song without such a row comes first
    out = so.subset_walks(res, lambda r: r["err"] > 0.001, ["B"], ["A"], [])
    assert out["walk peak 25"]["fit"] == (0, 0, 0, 0) and out["walk peak 25"]["held"][0] == 1


def rolled(errs, flags):
    r = fake_result(errs, [{}] * len(errs))
    for row, f in zip(r["rows"], flags):
        row["roll"] = f
    return r


def test_group_shift_is_fitted_on_the_fit_songs_and_counted_on_the_group_only():
    res = {"A": rolled([0.0020, 0.0021, 0.0, 0.0], [True, True, False, False]), "B": rolled([0.0020, 0.0, 0.0], [True, False, False]),
           "C": rolled([0.0020, 0.0], [True, False])}
    sh, g = so.group_shift(res, lambda r: r["roll"], ["A"], ["B"], ["C"])
    assert sh == pytest.approx(-1.25)                                       # the smallest shift that brings both roll hits of A back
    assert g["fit"] == (2, 0, 0, 2) and g["held"] == (1, 0, 0, 1) and g["sessions"] == (1, 0, 0, 1) and g["seven"] == (3, 0, 0, 3)


def test_group_shift_counts_a_right_hit_it_moves_out_of_the_window():
    res = {"A": rolled([0.0020, 0.0020], [True, True]), "B": rolled([0.0020, -0.0005], [True, True])}
    sh, g = so.group_shift(res, lambda r: True, ["A"], ["B"], [])
    assert sh == pytest.approx(-1.0)
    assert g["held"] == (1, 1, 1, 1)                       # the 2 ms hit comes back (right); the -0.5 ms hit goes out (wrong)


def test_fill_variants_score_each_variant_against_the_shipped_fill(monkeypatch):
    calls = []

    def chain(act, role="snare", lo=0.05, tol=0.08, nmin=8):
        calls.append((lo, tol, nmin))
        return [1.0, 2.0] if (lo, tol, nmin) == (0.05, 0.08, 8) else [1.0, 2.0, 3.0, 4.0]
    monkeypatch.setattr(ff, "chain_picks", chain)
    shifted = [1.01, 2.01, 3.01]                           # the third reference hit: a variant pick at 3.0 finds it
    out = so.fill_variants(None, shifted, [{"t": 1.01}, {"t": 2.01}], {"v": (0.03, 0.12, 6)})
    assert out == {"v": (1, 1)} and calls == [(0.05, 0.08, 8), (0.03, 0.12, 6)]


# ---------------------------------------------------------------- the analysis of one track

def test_analyze_one_labels_the_app_flags_the_roll_hit_and_checks_the_second_mic():
    res = run_track()
    assert res["counts"]["matched"] == 30 and res["counts"]["moved"] == 0
    assert "s top" in res["hat"] and "k in" not in res["hat"]
    assert res["n_roll"] == 1 and [r["roll"] for r in res["rows"]].count(True) == 1
    assert len(res["rise"]) == 30 and all(v > 10.0 for v in res["rise"])        # a real stroke: far over the peak before it
    assert res["fair"]["file"] == "s trigger" and res["fair"]["how"] == "trigger"


def test_analyze_one_names_the_loose_file_and_the_whose_error_class():
    res = run_track(shift_ms=1.8)
    moved = [r for r in res["rows"] if "moved_class" in r]
    assert res["files"]["s samp 2"]["moved"] == 10 and len(moved) == 0           # the app follows the trigger, not the loose file
    assert res["hat"]["s samp 2"] > res["hat"]["s trigger"] + 0.3
    assert all(r["side3"] == "mixed" or r["side3"] == "ref_file" for r in res["rows"])


def test_analyze_one_gives_the_moved_hit_a_second_mic_class():
    x = snare_track(TIMES, 11.5)
    ev = [{"t": t + (0.002 if k == 5 else 0.0), "v": 100} for k, t in enumerate(TIMES)]   # the app is 2 ms late at hit 5
    files = {"s trigger": [t + 0.006 for t in TIMES], "s samp 1": [t + 0.006 for t in TIMES], "s samp 2": [t + 0.006 for t in TIMES]}
    bot = [{"t": t + 0.003} for t in TIMES]                                          # the bottom mic is on time (3 ms latency)
    res = so.analyze_one("syn", files["s trigger"], ev, [], {"s bot": bot}, x, SR, files, None, yard=["s trigger"], log=lambda *_: None)
    moved = [r for r in res["rows"] if abs(r["err"]) > 0.001]
    assert len(moved) == 1 and moved[0]["j"] == 5 and moved[0]["bot"] == "ref" and moved[0]["side3"] == "app"


# ---------------------------------------------------------------- the report

def test_the_report_sections_run_on_a_track_result():
    res = run_track(shift_ms=1.8)
    res["events_equal_yardstick"] = 1.0
    res["extra"] = {}
    res["fill"] = {"shipped (0.05, 8 %, 8)": (0, 0)}
    res["miss_false"] = {"missed": [], "false": []}
    data = {"songs": {"Session A": res}, "held": {}}
    for sec in (so.section_files, so.section_fair, so.section_moved, so.section_hits):
        text = sec(data)
        assert "| " in text
    assert "s samp 2" in so.section_files(data)


# ---------------------------------------------------------------- P4-31 candidate B: the rise gate on a run folder

def test_the_gate_drops_a_hit_without_a_stroke_and_leaves_flam_hits_alone(tmp_path, monkeypatch):
    """`do_gate`: a hit whose own-mic rise is under the limit goes, a flam hit under the limit stays, a hit on a loud stroke
    stays; kick and tom events and the `before` runs are copied unchanged."""
    x = snare_track([0.5, 1.0, 1.5], 3.0)
    x[int(2.0 * SR):int(2.1 * SR)] += 0.02          # a steady 100 ms floor, no rise at 2.1 s or 2.2 s
    cfg = {"dir": "none", "mics": {"s top": ("s top", "snare"), "k in": ("k in", "kick")}}
    monkeypatch.setattr(fm, "songs", lambda: {"Song": cfg})
    monkeypatch.setattr(fm, "read_stem", lambda c, stem: (x, SR))
    src, dst = tmp_path / "src", tmp_path / "dst"
    (src / "run").mkdir(parents=True)
    (src / "refs.json").write_text("{}")
    ev = [{"t": 0.5, "v": 90}, {"t": 1.0, "v": 90}, {"t": 2.05, "v": 30}, {"t": 2.08, "v": 30, "flam": True}]
    kick = [{"t": 2.05, "v": 30}]
    (src / "run" / "Song_default.json").write_text(json.dumps({"events": {"s top": ev, "k in": kick}}))
    (src / "run" / "Song_before.json").write_text("{}")
    info = so.do_gate(src, dst, 10.0, log=lambda *_: None)
    out = json.loads((dst / "run" / "Song_default.json").read_text())
    assert [e["t"] for e in out["events"]["s top"]] == [0.5, 1.0, 2.08], "the hit at 2.05 s has no stroke"
    assert out["events"]["k in"] == kick
    assert info["Song"]["s top"] == {"hits": 4, "dropped": 1, "flam_kept": 1, "flam_under": 1}
    assert (dst / "run" / "Song_before.json").read_text() == "{}" and (dst / "refs.json").is_file()
