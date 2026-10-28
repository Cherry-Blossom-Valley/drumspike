"""P4-16: the manual-fixes metric (`tools/fix_metric.py`). Synthetic events only; no session file, no HOME needed."""
import json
import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import fix_metric as fm  # noqa: E402


def ev(*times, **flags):
    return [dict({"t": t}, **flags) for t in times]


def test_match_is_one_to_one_nearest_first():
    # two estimates near one reference: the nearer one owns it, the other stays unmatched
    assert fm.match([1.0], [1.012, 1.004]) == [(0, 1)]
    # one estimate between two references: it goes to the nearer reference
    assert fm.match([1.0, 1.030], [1.021]) == [(1, 0)]
    assert fm.match([1.0], [1.0201]) == []          # beyond 20 ms
    assert fm.match([1.0], [1.0199]) == [(0, 0)]


def test_outputs_leave_out_bleed_and_cut_hits():
    events = ev(1.0) + ev(2.0, bleed=True) + ev(3.0, cut=True) + [{"t": 4.0, "bleed": False, "cut": False}]
    assert fm.outputs(events) == [1.0, 4.0]


def test_a_clean_track_needs_no_fix():
    s = fm.score_track([1.0, 2.0], ev(1.0, 2.0), 0.0)
    assert (s["false"], s["missed"], s["moved"], s["fixes"]) == (0, 0, 0, 0)


def test_false_missed_and_moved_hits_are_one_fix_each():
    ref = [1.0, 2.0, 3.0, 4.0]
    events = ev(1.0, 2.0025, 3.5, 9.0)          # 2.0025 is 2.5 ms off; 3.0 and 4.0 are missed; 3.5 and 9.0 are false
    s = fm.score_track(ref, events, 0.0)
    assert (s["matched"], s["false"], s["missed"], s["moved"], s["fixes"]) == (2, 2, 2, 1, 5)
    assert s["n_ref"] == 4 and s["n_out"] == 4


def test_moved_needs_more_than_one_millisecond():
    near = fm.score_track([1.0], ev(1.0009), 0.0)
    far = fm.score_track([1.0], ev(1.0011), 0.0)
    early = fm.score_track([1.0], ev(0.9989), 0.0)
    assert (near["moved"], far["moved"], early["moved"]) == (0, 1, 1)


def test_the_offset_shifts_the_reference_before_the_onset_check():
    s = fm.score_track([1.0], ev(1.006), 0.005)         # 1 ms after the shifted reference: not moved
    assert (s["moved"], s["fixes"]) == (0, 0)
    s = fm.score_track([1.0], ev(1.006), 0.0)           # 6 ms: moved
    assert (s["moved"], s["fixes"]) == (1, 1)
    assert fm.score_track([1.0], ev(1.006), 0.005)["errs"] == pytest.approx([0.001])


def test_a_flagged_false_hit_costs_nothing_and_a_flagged_real_hit_is_missed():
    ref = [1.0, 2.0]
    events = ev(1.0) + ev(2.0, bleed=True) + ev(5.0, bleed=True) + ev(6.0, cut=True)
    s = fm.score_track(ref, events, 0.0)
    assert s["flagged"] == 3 and s["n_out"] == 1
    assert (s["false"], s["missed"], s["moved"], s["fixes"]) == (0, 1, 0, 1)   # 2.0 flagged: missed; 5.0, 6.0: no fix


def test_a_reference_without_hits_makes_every_output_hit_false():
    s = fm.score_track([], ev(1.0, 2.0, 3.0, bleed=False) + ev(4.0, bleed=True), 0.0)
    assert (s["false"], s["missed"], s["fixes"]) == (3, 0, 3)


def test_offset_is_the_median_of_the_nearest_events_within_50_ms():
    ref = [1.0, 2.0, 3.0, 4.0]
    events = ev(1.004, 2.006, 3.005) + ev(4.2) + ev(2.001, bleed=True)
    # 1.0 -> +0.004; 2.0 -> +0.001 (the flagged event counts, it is the nearer); 3.0 -> +0.005; 4.0 has none within 50 ms
    assert fm.track_offset(ref, events) == pytest.approx(0.004)
    assert fm.track_offset(ref, ev(10.0)) == 0.0
    assert fm.track_offset([1.0], ev(1.08)) == 0.0         # 80 ms away: outside the window
    assert fm.track_offset([1.0], ev(1.049)) == pytest.approx(0.049)
    assert fm.track_offset([], ev(1.0)) == 0.0


def test_add_up_sums_the_tracks_and_gives_the_rates():
    a = fm.score_track([1.0, 2.0], ev(1.0, 7.0), 0.0)       # 1 false, 1 missed
    b = fm.score_track([1.0, 2.0], ev(1.003, 2.0), 0.0)     # 1 moved
    r = fm.add_up([a, b], minutes=2.0)
    assert (r["n_ref"], r["false"], r["missed"], r["moved"], r["fixes"]) == (4, 1, 1, 1, 3)
    assert r["per_min"] == pytest.approx(1.5)
    assert r["per_100"] == pytest.approx(75.0)
    assert len(r["errs"]) == 3


def test_score_run_adds_the_tom_mics_into_one_role():
    refs = {"Song": {"k in": {"role": "kick", "onsets": [1.0, 2.0]},
                     "s top": {"role": "snare", "onsets": [1.5]},
                     "t1": {"role": "toms", "onsets": [3.0]},
                     "t2": {"role": "toms", "onsets": []}}}
    events = {"Song": {"k in": ev(1.0, 2.0), "s top": ev(1.5, 8.0), "t1": ev(3.0), "t2": ev(4.0, 5.0)}}
    offsets = {"Song": {m: 0.0 for m in refs["Song"]}}
    r = fm.score_run(refs, events, offsets, {"Song": 1.0})["Song"]
    assert [r[x]["fixes"] for x in ("kick", "snare", "toms")] == [0, 1, 2]
    assert r["toms"]["n_ref"] == 1 and r["toms"]["false"] == 2
    assert r["mics"]["t2"]["fixes"] == 2


def test_err_stats_in_milliseconds():
    s = fm.err_stats([0.0, 0.002, -0.002, 0.0005])
    assert s["n"] == 4
    assert s["beyond"] == pytest.approx(0.5)
    assert s["median"] == pytest.approx(0.25)
    assert fm.err_stats([])["n"] == 0


def test_the_tables_need_no_session_files(tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))
    assert fm.track_name("t1 HH_01") == "t1_HH_01"
    assert fm.md_table(["a", "b"], [[1, 2]]) == "| a | b |\n|---|---|\n| 1 | 2 |"


# ---------------------------------------------------------------- P4-16b: confirmed strokes (synthetic signals)

import numpy as np  # noqa: E402

SR = 48000


def burst(hits, n_s=4.0, level=0.5, floor=1e-4, seed=1):
    rng = np.random.default_rng(seed)
    x = rng.normal(0, floor, int(n_s * SR))
    for t in hits:
        i, n = int(t * SR), int(0.02 * SR)
        x[i:i + n] += level * np.exp(-np.arange(n) / (0.004 * SR)) * rng.choice([-1, 1], n)
    return x


def test_rise_db_separates_a_stroke_from_noise_and_a_tail():
    x = burst([1.0])
    assert fm.rise_db(x, SR, 1.0) > 40
    assert abs(fm.rise_db(x, SR, 0.4)) < 12
    assert fm.rise_db(x, SR, 1.015) < 10            # inside the decay of the stroke


def test_layer_offset_is_fixed_on_the_reference_and_needs_a_hit_in_range():
    ref = [1.0, 2.0, 3.0]
    assert fm.layer_offset(ref, [1.004, 2.0045, 3.0035]) == pytest.approx(0.004)
    assert fm.layer_offset(ref, []) == 0.0
    assert fm.layer_offset(ref, [1.2, 2.3]) == 0.0


def test_stroke_flags_layer_window_is_3_ms():
    x = burst([])
    f = fm.stroke_flags([1.0, 2.0, 3.0], [1.0029, 2.0031], x, SR, None)
    assert [a for a, _ in f] == [True, False, False] and not any(b for _, b in f)


def test_stroke_flags_witness_needs_both_mics():
    own, wit = burst([1.0, 2.0]), burst([1.0, 3.0], seed=2)
    f = fm.stroke_flags([1.0, 2.0, 3.0], [], own, SR, wit)
    assert [b for _, b in f] == [True, False, False]      # 2.0: own mic only; 3.0: witness only
    assert not any(b for _, b in fm.stroke_flags([1.0], [], own, SR, None))     # no witness mic: never


def test_stroke_flags_witness_ignores_bleed_of_the_other_role():
    x = burst([1.0, 2.0])
    f = fm.stroke_flags([1.0, 2.0], [], x, SR, x, other=[1.011, 2.013])
    assert [b for _, b in f] == [False, True]               # 11 ms: bleed; 13 ms: not
    f = fm.stroke_flags([1.0], [1.0], x, SR, x, other=[1.0])
    assert f == [(True, False)]                             # a layer still confirms


def test_score_track_counts_only_false_hits_as_confirmed():
    ref = [1.0, 2.0]
    events = ev(1.0, 2.0, 5.0, 6.0, 7.0)                    # 5.0, 6.0, 7.0 are false
    flags = [(True, True), (False, True), (True, False), (False, True), (False, False)]
    s = fm.score_track(ref, events, 0.0, flags=flags)
    assert s["false"] == 3 and s["confirmed"] == 2          # the flags of the two matched hits do not count
    assert (s["confirmed_layer"], s["confirmed_witness"]) == (1, 1)
    assert s["fixes"] == 3 and s["fixes_ex"] == 1


def test_score_track_without_flags_confirms_nothing():
    s = fm.score_track([1.0], ev(1.0, 5.0), 0.0)
    assert s["confirmed"] == 0 and s["fixes_ex"] == s["fixes"] == 1


def test_a_hit_with_both_flags_is_one_confirmed_stroke():
    s = fm.score_track([1.0], ev(1.0, 5.0), 0.0, flags=[(False, False), (True, True)])
    assert s["confirmed"] == 1 and (s["confirmed_layer"], s["confirmed_witness"]) == (1, 1) and s["fixes_ex"] == 0


def test_add_up_sums_the_confirmed_columns_and_fixes_ex():
    a = fm.score_track([1.0], ev(1.0, 5.0), 0.0, flags=[(False, False), (True, False)])
    b = fm.score_track([1.0, 2.0], ev(1.0), 0.0)
    t = fm.add_up([a, b], 1.0)
    assert t["confirmed"] == 1 and t["fixes"] == 2 and t["fixes_ex"] == 1


# ---------------------------------------------------------------- P4-16f: strokes the owner heard as real

def test_listened_flags_window_is_2_ms():
    assert fm.listened_flags([1.0, 2.0, 3.0], [1.0019, 2.0021]) == [True, False, False]
    assert fm.listened_flags([1.0], []) == [False]


def test_a_listened_false_hit_is_a_confirmed_stroke():
    s = fm.score_track([1.0], ev(1.0, 5.0, 6.0), 0.0,
                       flags=[(False, False, True), (False, False, True), (False, False, False)])
    assert s["false"] == 2 and s["confirmed"] == 1 and s["confirmed_listened"] == 1 and s["fixes_ex"] == 1
    assert (s["confirmed_layer"], s["confirmed_witness"]) == (0, 0)
    assert fm.add_up([s], 1.0)["confirmed_listened"] == 1


def test_listened_holds_only_the_8_real_verdicts():
    assert sum(len(v) for v in fm.LISTENED.values()) == 8
    assert all(role == "snare" for _, role in fm.LISTENED)


def test_control_rates_the_rules_fire_on_strokes_and_not_on_silence():
    hits = [0.5 + 0.4 * i for i in range(12)]
    x = burst(hits, n_s=8.0)
    c = fm.control_rates(hits, hits, x, SR, x, hits)
    assert c["matched"][:2] == (1.0, 1.0)
    assert c["moved40"][:2] == (0.0, 0.0)
    assert c["random"][2] > 50 and c["random"][:2] == (0.0, 0.0)


# ---------------------------------------------------------------- P4-16c: the fair kick reference

NAN = float("nan")


def test_pick_fair_takes_the_trigger_file_first_even_with_a_worse_jitter():
    assert fm.pick_fair(["k samp 1", "k trigger"], {"k samp 1": 0.1, "k trigger": 0.9}) == ("k trigger", "trigger")
    assert fm.pick_fair(["k samp 1", "k TRIG_01", "k trigger"], {}) == ("k TRIG_01", "trigger")   # the first trigger file


def test_pick_fair_takes_the_lowest_jitter_without_a_trigger_and_the_first_on_a_tie():
    files = ["k samp 1", "k samp 2", "k samp 3"]
    assert fm.pick_fair(files, {"k samp 1": 0.9, "k samp 2": 0.3, "k samp 3": 0.2}) == ("k samp 3", "hat")
    assert fm.pick_fair(files, {"k samp 1": 0.4, "k samp 2": 0.4, "k samp 3": 0.4}) == ("k samp 1", "hat")
    assert fm.pick_fair(files, {"k samp 1": NAN, "k samp 2": 0.5, "k samp 3": NAN}) == ("k samp 2", "hat")


def test_pick_fair_with_one_file_or_no_jitter_takes_the_first_file():
    assert fm.pick_fair(["k samp 1"], {"k samp 1": 0.7}) == ("k samp 1", "hat")
    assert fm.pick_fair(["k samp 1"], {}) == ("k samp 1", "first")
    assert fm.pick_fair(["a", "b"], {"a": NAN, "b": NAN}) == ("a", "first")


def _refs():
    return {"Song": {"duration_s": 60.0, "mics": {
        "k in": {"role": "kick", "stem": "k in", "onsets": [1.0, 2.0], "source": "old"},
        "s top": {"role": "snare", "stem": "s top", "onsets": [1.5], "source": "snare"},
        "t1": {"role": "toms", "stem": "t1", "onsets": [3.0], "source": "tom"}}}}


def test_apply_kick_ref_sets_only_the_kick_reference_and_union_keeps_everything(monkeypatch):
    monkeypatch.setattr(fm, "kick_files", lambda song: ["k samp 1", "k samp 2"])
    monkeypatch.setattr(fm, "songs", lambda: {"Song": {"dir": "x"}})
    monkeypatch.setattr(fm, "read_reference", lambda cfg, stems: ([1.001, 2.002] if stems == ["k samp 2"] else [9.0], {}))
    hat = {"Song": {"k samp 1": 0.9, "k samp 2": 0.2}}
    refs = _refs()
    assert fm.apply_kick_ref(refs, "union", hat) == _refs()
    out = fm.apply_kick_ref(refs, "fair", hat, log=lambda *_: None)
    mics = out["Song"]["mics"]
    assert mics["k in"]["onsets"] == [1.001, 2.002] and mics["k in"]["ref_file"] == "k samp 2"
    assert mics["s top"] == _refs()["Song"]["mics"]["s top"] and mics["t1"] == _refs()["Song"]["mics"]["t1"]
    assert refs == _refs()          # the input is not changed


def test_apply_kick_ref_takes_the_trigger_file_of_a_session_that_has_one(monkeypatch):
    monkeypatch.setattr(fm, "kick_files", lambda song: ["k samp 2", "k trigger"])
    monkeypatch.setattr(fm, "songs", lambda: {"Song": {"dir": "x"}})
    monkeypatch.setattr(fm, "read_reference", lambda cfg, stems: ([5.0] if stems == ["k trigger"] else [9.0], {}))
    out = fm.apply_kick_ref(_refs(), "fair", {"Song": {"k samp 2": 0.0, "k trigger": 1.0}}, log=lambda *_: None)
    assert out["Song"]["mics"]["k in"]["onsets"] == [5.0] and out["Song"]["mics"]["k in"]["ref_file"] == "k trigger"


def test_fair_kick_reads_the_jitter_of_the_song_only(monkeypatch):
    monkeypatch.setattr(fm, "kick_files", lambda song: ["a", "b"])
    assert fm.fair_kick("Song", {"Other": {"a": 0.9, "b": 0.1}, "Song": {"a": 0.1, "b": 0.9}}) == ("a", "hat")
    assert fm.fair_kick("Song", {}) == ("a", "first")


# ---------------------------------------------------------------- P4-23: one fair choice for kick and snare (`--ref`)

def _fake_files(monkeypatch):
    monkeypatch.setattr(fm, "kick_files", lambda song: ["k samp 1", "k samp 2"])
    monkeypatch.setattr(fm, "snare_files", lambda song: ["s trigger", "s samp 1", "s samp 2"])
    monkeypatch.setattr(fm, "songs", lambda: {"Song": {"dir": "x"}})
    onsets = {"k samp 2": [1.001, 2.002], "s trigger": [1.504], "s samp 2": [7.0], "s samp 1": [8.0]}
    monkeypatch.setattr(fm, "read_reference", lambda cfg, stems: (onsets.get(stems[0], [9.9]), {}))


def test_fair_ref_dispatches_on_the_role(monkeypatch):
    _fake_files(monkeypatch)
    hat = {"Song": {"k samp 1": 0.9, "k samp 2": 0.2, "s samp 1": 0.1, "s samp 2": 0.2}}
    assert fm.fair_ref("Song", "kick", hat) == ("k samp 2", "hat")
    assert fm.fair_ref("Song", "snare", hat) == ("s trigger", "trigger")          # the trigger file before any jitter


def test_apply_ref_sets_the_roles_that_are_fair_and_leaves_the_others(monkeypatch):
    _fake_files(monkeypatch)
    hats = {"kick": {"Song": {"k samp 1": 0.9, "k samp 2": 0.2}}, "snare": {"Song": {}}}
    refs = _refs()
    both = fm.apply_ref(refs, {"kick": "fair", "snare": "fair"}, hats, log=lambda *_: None)["Song"]["mics"]
    assert both["k in"]["onsets"] == [1.001, 2.002] and both["s top"]["onsets"] == [1.504] and both["s top"]["ref_file"] == "s trigger"
    assert both["t1"] == _refs()["Song"]["mics"]["t1"]                              # toms keep their reference
    snare = fm.apply_ref(refs, {"kick": "union", "snare": "fair"}, hats, log=lambda *_: None)["Song"]["mics"]
    assert snare["k in"] == _refs()["Song"]["mics"]["k in"] and snare["s top"]["onsets"] == [1.504]
    kick = fm.apply_ref(refs, {"kick": "fair"}, hats, log=lambda *_: None)["Song"]["mics"]
    assert kick["s top"] == _refs()["Song"]["mics"]["s top"]                         # a role not named stays union
    assert refs == _refs()                                                          # the input is not changed


def test_apply_ref_union_everywhere_changes_nothing(monkeypatch):
    _fake_files(monkeypatch)
    assert fm.apply_ref(_refs(), {"kick": "union", "snare": "union"}, {}, log=lambda *_: None) == _refs()


def test_apply_ref_snare_without_a_trigger_takes_the_lowest_hat_jitter(monkeypatch):
    _fake_files(monkeypatch)
    monkeypatch.setattr(fm, "snare_files", lambda song: ["s samp 1", "s samp 2"])
    out = fm.apply_ref(_refs(), {"snare": "fair"}, {"snare": {"Song": {"s samp 1": 0.3, "s samp 2": 0.1}}}, log=lambda *_: None)
    assert out["Song"]["mics"]["s top"]["onsets"] == [7.0] and out["Song"]["mics"]["s top"]["ref_file"] == "s samp 2"
    assert "snare" in out["Song"]["mics"]["s top"]["source"] and "hat" in out["Song"]["mics"]["s top"]["source"]


def test_read_hat_takes_the_hat_of_each_song_from_a_tracks_file(tmp_path):
    p = tmp_path / "tracks.json"
    p.write_text('{"songs": {"A": {"hat": {"f": 0.2}, "x": 1}}, "held": {}}')
    assert fm.read_hat(p) == {"A": {"f": 0.2}}


def test_the_score_command_takes_one_ref_for_both_roles_and_a_per_role_override(monkeypatch):
    seen = []
    monkeypatch.setattr(fm, "do_score", lambda out, **kw: seen.append(kw))
    fm.main(["score"])
    fm.main(["score", "--ref", "union", "--snare-ref", "fair", "--hat-snare", "h.json"])
    assert seen[0]["ref"] == "fair" and seen[0]["kick_ref"] is None and seen[0]["snare_ref"] is None      # the default is fair
    assert seen[1]["ref"] == "union" and seen[1]["snare_ref"] == "fair" and seen[1]["hat_snare"] == "h.json"


def test_ref_modes_give_each_role_its_own_option_over_the_common_one():
    assert fm.ref_modes("fair") == {"kick": "fair", "snare": "fair"}
    assert fm.ref_modes("union", snare_ref="fair") == {"kick": "union", "snare": "fair"}
    assert fm.ref_modes("fair", kick_ref="union") == {"kick": "union", "snare": "fair"}


def test_apply_ref_replaces_the_reference_of_the_main_mic_only(monkeypatch):
    _fake_files(monkeypatch)
    refs = _refs()
    refs["Song"]["mics"]["k sub"] = {"role": "kick", "stem": "k sub", "onsets": [4.0], "source": "sub"}
    out = fm.apply_ref(refs, {"kick": "fair"}, {"kick": {"Song": {"k samp 1": 0.9, "k samp 2": 0.2}}}, log=lambda *_: None)
    assert out["Song"]["mics"]["k in"]["onsets"] == [1.001, 2.002] and out["Song"]["mics"]["k sub"]["onsets"] == [4.0]


# ---- P4-43: the DRT side (`score --midi`); MIDI files written here with mido, no audio

def _drt_refs(tmp_path):
    """A refs.json of two songs with kick, snare and two tom mics; 40 hits per mic, irregular spacing."""
    rng = np.random.default_rng(7)
    refs = {}
    for k, song in enumerate(["Song A", "Song B"]):
        mics = {}
        for i, (m, role) in enumerate([("k in", "kick"), ("s top", "snare"), ("t1", "toms"), ("t2", "toms")]):
            t = 1.0 + 0.1 * i + k * 0.03 + np.cumsum(rng.uniform(0.15, 0.6, 40))
            mics[m] = {"role": role, "stem": m, "onsets": [float(x) for x in t], "source": "test"}
        refs[song] = {"duration_s": 30.0, "mics": mics}
    src = tmp_path / "src"
    src.mkdir()
    (src / "refs.json").write_text(json.dumps(refs))
    return src, refs


def _score_midi(tmp_path, src, midi):
    return fm.do_score(tmp_path / "out", run=src, ref="union", midi=midi, log=lambda *_: None)["drt"]


def test_midi_hits_follow_the_tempo_map_and_take_note_ons_only(tmp_path):
    import mido
    mf = mido.MidiFile(ticks_per_beat=96)
    tr = mido.MidiTrack()
    mf.tracks.append(tr)
    tr += [mido.MetaMessage("set_tempo", tempo=600000, time=0),          # 100 BPM: one beat 0.6 s
           mido.Message("note_on", note=36, velocity=90, time=96),       # 0.6 s
           mido.Message("note_on", note=36, velocity=0, time=10),        # a note-off as velocity 0: no hit
           mido.MetaMessage("set_tempo", tempo=300000, time=86),         # at 1.2 s, 200 BPM
           mido.Message("note_on", note=38, velocity=40, time=96)]       # 1.5 s
    mf.save(str(tmp_path / "x.mid"))
    assert fm.midi_hits(tmp_path / "x.mid") == pytest.approx([0.6, 1.5])


def test_the_references_as_midi_score_zero_fixes_for_every_song_and_role(tmp_path):
    src, refs = _drt_refs(tmp_path)
    fm.main(["refs-midi", "--run", str(src), "--midi", str(tmp_path / "midi")])
    assert (tmp_path / "midi" / "Song A" / "s top.mid").is_file()
    drt = _score_midi(tmp_path, src, tmp_path / "midi")
    assert set(drt) == {"Song A", "Song B"}
    for song in drt:
        for role in fm.ROLES:
            r = drt[song][role]
            assert (r["false"], r["missed"], r["moved"], r["fixes"]) == (0, 0, 0, 0), (song, role)
            assert r["n_out"] == r["n_ref"] > 0
    assert "DRT: fixes per song and role" in (tmp_path / "out" / "drt-report.md").read_text()


def test_one_moved_one_deleted_and_one_added_hit_are_one_fix_each(tmp_path):
    src, refs = _drt_refs(tmp_path)
    fm.do_refs_midi(src, tmp_path / "midi", log=lambda *_: None)
    on = list(refs["Song A"]["mics"]["s top"]["onsets"])
    on[10] += 0.002                                    # moved by 2 ms
    del on[20]                                         # deleted: one missed hit
    on.append(on[-1] + 0.5)                            # added, far from every reference hit: one false hit
    fm.write_midi(tmp_path / "midi" / "Song A" / "s top.mid", on)
    drt = _score_midi(tmp_path, src, tmp_path / "midi")
    t = drt["Song A"]["mics"]["s top"]
    assert (t["moved"], t["missed"], t["false"], t["fixes"]) == (1, 1, 1, 3)
    assert drt["Song A"]["snare"]["fixes"] == 3
    others = [drt[s]["mics"][m]["fixes"] for s in drt for m in drt[s]["mics"] if (s, m) != ("Song A", "s top")]
    assert others == [0] * 7
    report = (tmp_path / "out" / "drt-report.md").read_text()
    assert "| Song A | s top | snare | 40 | 40 | 1 / 1 / 1 |" in report


def test_the_app_safe_names_count_and_a_song_without_a_folder_is_left_out(tmp_path):
    src, refs = _drt_refs(tmp_path)
    for m, v in refs["Song B"]["mics"].items():
        fm.write_midi(tmp_path / "midi" / "Song_B" / f"{fm.track_name(m)}.mid", v["onsets"])
    drt = _score_midi(tmp_path, src, tmp_path / "midi")
    assert list(drt) == ["Song B"] and drt["Song B"]["kick"]["fixes"] == 0


def test_a_song_folder_without_a_file_for_each_mic_stops_the_score(tmp_path):
    src, refs = _drt_refs(tmp_path)
    fm.do_refs_midi(src, tmp_path / "midi", log=lambda *_: None)
    (tmp_path / "midi" / "Song B" / "t2.mid").unlink()
    with pytest.raises(SystemExit, match="Song B/t2.mid"):
        _score_midi(tmp_path, src, tmp_path / "midi")


def test_drt_against_drum_spike_and_the_exit_rule():
    def role(f):
        return {"n_ref": 10, "n_out": 10, "flagged": 0, "matched": 10, "false": f, "missed": 0, "moved": 0, "fixes": f,
                "errs": [], "per_min": 0.0, "per_100": 0.0}

    def run(fixes):
        return {s: dict({r: role(f) for r, f in zip(fm.ROLES, v)}, mics={}) for s, v in fixes.items()}

    refs, minutes, off = {"A": {}, "B": {}}, {"A": 1.0, "B": 1.0}, {"A": {}, "B": {}}
    drt = run({"A": (3, 3, 3), "B": (1, 1, 1)})
    text = "\n".join(fm.drt_tables(drt, off, refs, minutes, ds=run({"A": (1, 1, 1), "B": (1, 1, 1)})))
    assert "| A | **all roles** | 30 | 9 / 0 / 0 = **9** | 3 / 0 / 0 = **3** | **6** |" in text
    assert "**FAIL** (A yes, B no)" in text                    # B: 3 fixes each, Drum Spike is not lower
    text = "\n".join(fm.drt_tables(drt, off, refs, minutes, ds=run({"A": (1, 1, 1), "B": (0, 1, 1)})))
    assert "**PASS** (A yes, B yes)" in text
