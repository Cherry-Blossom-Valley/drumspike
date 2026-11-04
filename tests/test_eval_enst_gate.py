"""Tests of the ENST evaluation: tools/eval/enst.py (#276 part B: the file name parser incl. the xN
suffix, the label mapping and the scorer with planted hits) and tools/eval/enst_gate.py (#389 C13:
the compare logic, the twelve-number pooling and the planted regression end to end on tiny synthetic
tables). No ENST data, no run files, no tool; mir_eval only for the optional cross-check."""
import json
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools" / "eval"))

import enst  # noqa: E402
import enst_gate  # noqa: E402
from enst import track_name  # noqa: E402

# --- tools/eval/enst.py: parser, labels, scorer, run_item ---


def test_parse_name_plain():
    assert enst.parse_name("036_phrase_disco_simple_slow_sticks") == ("036", "phrase", "disco_simple_slow",
                                                                      "sticks", None)


def test_parse_name_hits_xn():
    # a hits item ends with the repeat count: the tool is the second-to-last field (#262)
    assert enst.parse_name("001_hits_snare-drum_sticks_x6") == ("001", "hits", "snare-drum", "sticks", "6")
    assert enst.parse_name("024_hits_low-tom_mallets_x6") == ("024", "hits", "low-tom", "mallets", "6")


def test_parse_name_minus_one_variants():
    assert enst.parse_name("121_MIDI-minus-one_bigband_brushes")[1] == "MIDI-minus-one"
    assert enst.parse_name("107_minus-one_salsa_sticks")[1] == "minus-one"


def test_parse_name_rejects_garbage():
    with pytest.raises(ValueError):
        enst.parse_name("not-an-item")
    with pytest.raises(ValueError):
        enst.parse_name("036_unknown_type_sticks")


def test_label_mapping():
    assert enst.label_role("bd") == "kick"
    assert enst.label_role("sd") == "snare"
    assert enst.label_role("sd-") == "snare"
    assert enst.label_role("rs") == "snare"
    assert enst.label_role("cs") == "snare"
    assert enst.label_role("chh") == "hat"
    assert enst.label_role("ohh") == "hat"
    for lab in ("mt", "lt", "lmt", "lft", "mtr", "ltr"):
        assert enst.label_role(lab) == "tom"
    for lab in ("rc2", "cr1", "c1", "ch1", "spl2"):
        assert enst.label_role(lab) == "cymbal"
    for lab in ("cb", "sweep", "sticks"):
        assert enst.label_role(lab) is None


def test_match_events_one_to_one_nearest_first():
    ref = [0.0, 0.1, 0.2]
    est = [0.002, 0.098, 0.21, 0.205]
    pairs = dict(enst._match_events(ref, est, 0.050))
    assert pairs == {0: 0, 1: 1, 2: 3}          # 0.21 and 0.205 both near 0.2: the nearer one wins
    # the scorer's offset accumulation reads the DICT items (ref index -> est index); iterating the
    # dict itself yields the ref indices (ints) and crashes (276.result.log rc=21)
    offs = [est[j] - ref[i] for i, j in pairs.items()]
    assert [round(o, 3) for o in offs] == [0.002, -0.002, 0.005]


def test_prf_planted_hits():
    ref = [0.0, 0.1, 0.2, 0.3]
    # two hits inside the window, two extra
    est = [0.004, 0.104, 0.5, 0.6]
    p, r, f = enst.prf(ref, est, 0.050)
    assert (round(p, 3), round(r, 3), round(f, 3)) == (0.5, 0.5, 0.5)
    # perfect hits
    p, r, f = enst.prf(ref, list(ref), 0.050)
    assert (p, r, f) == (1.0, 1.0, 1.0)
    # empty reference: no score (the caller FAILs on a missing role row)
    assert enst.prf([], [0.0], 0.050) is None


def test_prf_matches_mir_eval():
    np = pytest.importorskip("numpy")
    mir_eval = pytest.importorskip("mir_eval")
    ref = np.array([0.0, 0.1, 0.2, 0.4, 0.9])
    est = np.array([0.003, 0.19, 0.21, 0.5, 0.55, 0.905])
    for w in enst.WINDOWS:
        m = mir_eval.onset.f_measure(ref, est, window=w)
        # mir_eval.onset.f_measure returns (F, P, R)
        assert tuple(round(v, 6) for v in enst.prf(list(ref), list(est), w)) == \
            (round(m[1], 6), round(m[2], 6), round(m[0], 6))


def test_item_refs_routes_by_label():
    tm = {"mt": "tom_1", "lt": "tom_2"}
    evs = [(0.0, "bd"), (0.1, "sd"), (0.2, "chh"), (0.3, "mt"), (0.4, "lt"), (0.5, "cb")]
    refs = enst.item_refs(evs, tm)
    assert [t for t, _ in refs["kick"]] == [0.0]
    assert [t for t, _ in refs["snare"]] == [0.1]
    assert [t for t, _ in refs["hi-hat"]] == [0.2]
    assert [t for t, _ in refs["tom_1"]] == [0.3]
    assert [t for t, _ in refs["tom_2"]] == [0.4]


def test_run_item_removes_stale_scratch(monkeypatch, tmp_path):
    # A killed run leaves a partial scratch project behind; the resume must remove it
    # before project_new instead of failing with 409 "project already exists" (#276).
    wav = tmp_path / "audio.wav"
    wav.write_bytes(b"")
    item = {"wav": {"kick": wav, "snare": wav}}
    scratch = tmp_path / "scratch"
    (scratch / "p" / "enst").mkdir(parents=True)          # the stale partial project
    (scratch / "steps.json").write_text("stale")

    def fake_run(cmd, capture_output, text, env, stdin):
        # project_new must not see the stale project any more
        assert not (scratch / "p").exists()
        present = list(item["wav"])
        steps = 5 + len(present)
        rows = [{"ok": True}] * steps
        rows[4] = {"ok": True, "result": {"tracks": {track_name(m): {"detector": {"name": "d"}} for m in present}}}
        for i in range(steps - 5):
            rows[5 + i] = {"ok": True, "result": {"events": []}}
        return subprocess.CompletedProcess(cmd, 0, stdout="\n".join(json.dumps(r) for r in rows), stderr="")

    monkeypatch.setattr(enst.subprocess, "run", fake_run)
    out = tmp_path / "out"
    res = enst.run_item(Path("tool"), out, "drummer_1", "064_stem", item, "default", scratch)
    assert res["stem"] == "064_stem"
    assert (out / "run" / "default" / "drummer_1" / "064_stem.json").is_file()
    # a rerun with the result file present returns it and does not touch a fresh scratch dir
    scratch.mkdir()
    (scratch / "p" / "enst").mkdir(parents=True)
    res2 = enst.run_item(Path("tool"), out, "drummer_1", "064_stem", item, "default", scratch)
    assert res2 == res
    assert (scratch / "p" / "enst").exists()              # not removed: the result file exists


# --- tools/eval/enst_gate.py: compare, pooling, planted regression, stale run files ---


def test_compare_passes_on_equal_and_small_noise():
    base = {"d1|kick": 0.8, "d1|hat": 0.9}
    rows, changed, improved, missing = enst_gate.compare_numbers(base, dict(base))
    assert (rows, changed, improved, missing) == (2, [], [], [])
    # a 1 % dip is inside the 2 % tolerance and is not an improvement either
    rows, changed, improved, missing = enst_gate.compare_numbers(base, {"d1|kick": 0.792, "d1|hat": 0.9})
    assert (rows, changed, missing) == (2, [], [])


def test_compare_fails_on_a_two_percent_regression():
    base = {"d1|kick": 0.8, "d2|snare": 0.5}
    rows, changed, improved, missing = enst_gate.compare_numbers(base, {"d1|kick": 0.8, "d2|snare": 0.489})
    assert rows == 2
    assert changed == [("d2|snare", 0.5, 0.489)]      # 0.489 < 0.5 * 0.98
    assert improved == []
    ok, lines = enst_gate.verdict(rows, changed, missing)
    assert not ok
    assert any("REGRESSED d2|snare" in line for line in lines)


def test_compare_missing_row_and_zero_rows():
    rows, changed, improved, missing = enst_gate.compare_numbers({"d1|kick": 0.8}, {})
    assert (rows, missing) == (0, ["d1|kick"])
    ok, lines = enst_gate.verdict(rows, changed, missing)
    assert not ok
    assert any("MISSING d1|kick" in line for line in lines)
    # a baseline without any compared row must FAIL too
    ok, lines = enst_gate.verdict(0, [], [])
    assert not ok
    assert any("no row could be compared" in line for line in lines)


def test_verdict_passes_clean():
    assert enst_gate.verdict(12, [], []) == (True, [])


def test_numbers_from_per_pools_tom_mics():
    # the tom mics pool into one number per drummer, role rows without reference hits are omitted
    per = {
        ("dsp", "d1", "kick"): {"ref": [0.0, 0.1], "est": [0.0, 0.1]},
        ("dsp", "d1", "snare"): {"ref": [0.0], "est": [0.0]},
        ("dsp", "d1", "hi-hat"): {"ref": [0.0], "est": []},
        ("dsp", "d1", "tom_1"): {"ref": [0.0], "est": [0.0]},
        ("dsp", "d1", "tom_2"): {"ref": [0.5], "est": []},
        ("dsp", "d2", "kick"): {"ref": [1.0], "est": [1.0]},
    }
    numbers = enst_gate.numbers_from_per(per)
    assert set(numbers) == {"d1|kick", "d1|snare", "d1|hat", "d1|tom", "d2|kick"}
    assert numbers["d1|kick"] == 1.0
    assert numbers["d1|hat"] == 0.0                      # output but no match
    # the pooled tom: 2 refs, 1 match -> F1 = 2 * 0.5 * 1 / 1.5
    assert abs(numbers["d1|tom"] - 2 / 3) < 1e-9


def test_plant_drops_every_tenth_kick_hit():
    est = [float(i) for i in range(25)]
    assert enst_gate.plant_est("kick", est, "drop-kick-tenth") == [e for i, e in enumerate(est) if i % 10 != 9]
    assert enst_gate.plant_est("snare", est, "drop-kick-tenth") == est   # only the kick rows
    assert enst_gate.plant_est("kick", est, None) == est


def _synthetic_per(config, plant, tool_fp=None):
    role_of = {"kick": "kick", "snare": "snare", "hi-hat": "hat"}
    per = {}
    for d in ("drummer_1", "drummer_2"):
        for mic in ("kick", "snare", "hi-hat"):
            per[(config, d, mic)] = {"ref": [float(i) for i in range(12)],
                                     "est": enst_gate.plant_est(role_of[mic],
                                                                [float(i) for i in range(12)], plant),
                                     "tool_fp": tool_fp}
    return per


def _patch_gate(monkeypatch, collect):
    """The gate on a synthetic corpus: no dataset, no run files, no tool. `collect` stands in for
    `collect_per` (it must accept the `tool_fp` keyword of the stale check)."""
    monkeypatch.setattr(enst_gate.enst, "index_items", lambda root: {})
    monkeypatch.setattr(enst_gate, "collect_per", collect)


def test_do_gate_planted_regression_fails(tmp_path, monkeypatch, capsys):
    # the (b) of the ticket end to end: the planted regression prints the changed kick rows and
    # FAILs; the same numbers unplanted PASS. No dataset, no run files, no tool.
    out, baseline = tmp_path / "out", tmp_path / "baseline.json"
    (tmp_path / "root").mkdir()
    baseline.write_text(json.dumps({"window": 0.05, "config": "dsp", "compared": 6,
                                    "numbers": {"drummer_1|kick": 1.0, "drummer_1|snare": 1.0,
                                                "drummer_1|hat": 1.0, "drummer_2|kick": 1.0,
                                                "drummer_2|snare": 1.0, "drummer_2|hat": 1.0}}))
    _patch_gate(monkeypatch, lambda out, root, config, plant=None, tool_fp=None, log=print:
                _synthetic_per(config, plant))
    rc = enst_gate.do_gate(str(tmp_path / "root"), out, baseline, tool=str(tmp_path / "absent"),
                           plant="drop-kick-tenth", log=print)
    text = capsys.readouterr().out
    assert rc == 1
    assert "REGRESSED drummer_1|kick" in text and "REGRESSED drummer_2|kick" in text
    assert "rows compared: 6" in text
    assert text.rstrip().endswith("FAIL")
    # the baseline was not touched by the planted run
    assert json.loads(baseline.read_text())["numbers"]["drummer_1|kick"] == 1.0


def test_do_gate_passes_on_the_baseline_numbers(tmp_path, monkeypatch, capsys):
    out, baseline = tmp_path / "out", tmp_path / "baseline.json"
    (tmp_path / "root").mkdir()
    # the baseline holds only the roles the synthetic corpus has (no tom mics)
    base = {f"{d}|{r}": 1.0 for d in ("drummer_1", "drummer_2") for r in ("kick", "snare", "hat")}
    baseline.write_text(json.dumps({"window": 0.05, "config": "dsp", "compared": 6, "numbers": base}))
    _patch_gate(monkeypatch, lambda out, root, config, plant=None, tool_fp=None, log=print:
                _synthetic_per(config, plant))
    rc = enst_gate.do_gate(str(tmp_path / "root"), out, baseline, tool=str(tmp_path / "absent"),
                           log=print)
    text = capsys.readouterr().out
    assert rc == 0
    assert "rows compared: 6" in text
    assert text.rstrip().endswith("PASS")


def test_do_gate_plant_never_refreshes_the_baseline(tmp_path):
    baseline = tmp_path / "baseline.json"
    (tmp_path / "root").mkdir()
    try:
        enst_gate.do_gate(str(tmp_path), tmp_path / "out", baseline, plant="drop-kick-tenth",
                          refresh=True, log=print)
        raise AssertionError("the planted refresh must be refused")
    except SystemExit as e:
        assert "must not refresh" in str(e)
    assert not baseline.exists()


def _stale_fixture(tmp_path, fp):
    """A run file from another build: one item, wrong fingerprint, correct shape otherwise."""
    d = tmp_path / "out" / "run" / "dsp" / "drummer_1"
    d.mkdir(parents=True)
    (d / "003_phrase_A_tak1.txt.json").write_text(json.dumps(
        {"drummer": "drummer_1", "stem": "003_phrase_A_tak1.txt", "config": "dsp", "tool_fp": fp,
         "detector": {"kick": "rust-dsp"}, "events": {"kick": []}}))
    return d / "003_phrase_A_tak1.txt.json"


def test_stale_fingerprint_triggers_the_rerun(tmp_path, monkeypatch, capsys):
    # the fix of the coordinator: a run file from another detector build is measured again, the
    # stale file is deleted first and never scored
    root = tmp_path / "root"
    root.mkdir()
    ann = root / "ann.txt"
    ann.write_text("")
    idx = {"drummer_1": {"003_phrase_A_tak1.txt": {"ann": ann}}}
    monkeypatch.setattr(enst_gate.enst, "index_items", lambda r: idx)
    stale_p = _stale_fixture(tmp_path, "0000000000000000")
    tool = tmp_path / "service_tool"
    tool.write_bytes(b"\x00tool")                      # the current build (any bytes)
    ran = []
    monkeypatch.setattr(enst_gate.enst, "do_run",
                        lambda root, out, t, configs, log=print: ran.append((t, configs)))
    monkeypatch.setattr(enst_gate, "collect_per",
                        lambda out, root, config, plant=None, tool_fp=None, log=print:
                        _synthetic_per(config, None))
    baseline = tmp_path / "baseline.json"
    base = {f"{d}|{r}": 1.0 for d in ("drummer_1", "drummer_2") for r in ("kick", "snare", "hat")}
    baseline.write_text(json.dumps({"window": 0.05, "config": "dsp", "compared": 6, "numbers": base}))
    rc = enst_gate.do_gate(str(root), tmp_path / "out", baseline, tool=str(tool), log=print)
    text = capsys.readouterr().out
    assert rc == 0
    assert not stale_p.is_file()                       # the stale file was removed and rerun
    assert ran and ran[0][1] == ("dsp",)               # the rerun went through do_run
    assert "another detector build" in text
    assert "stale or missing" not in text              # the tool exists, so no FAIL


def test_stale_run_files_never_scored_even_fresh(tmp_path, monkeypatch, capsys):
    # defence in depth: collect_per refuses a stale file outright, so a rerun that did not
    # happen (e.g. do_run skipped the item) cannot produce a score
    root = tmp_path / "root"
    root.mkdir()
    ann = root / "ann.txt"
    ann.write_text("")
    idx = {"drummer_1": {"003_phrase_A_tak1.txt": {"ann": ann}}}
    monkeypatch.setattr(enst_gate.enst, "index_items", lambda r: idx)
    monkeypatch.setattr(enst_gate.enst, "DRUMMERS", ("drummer_1",))   # the synthetic corpus
    monkeypatch.setattr(enst_gate.enst, "tom_mapping", lambda root, drummer: {})
    stale_p = _stale_fixture(tmp_path, "0000000000000000")
    try:
        enst_gate.collect_per(tmp_path / "out", root, "dsp", tool_fp="cafecafe", log=print)
        raise AssertionError("a stale run file must not be scored")
    except SystemExit as e:
        assert "stale" in str(e)
    # a file with the current build's fingerprint passes the check
    stale_p.write_text(json.dumps({"drummer": "drummer_1", "stem": "003_phrase_A_tak1.txt",
                                   "config": "dsp", "tool_fp": "cafecafe",
                                   "detector": {"kick": "rust-dsp"}, "events": {"kick": []}}))
    per = enst_gate.collect_per(tmp_path / "out", root, "dsp", tool_fp="cafecafe", log=print)
    assert per[("dsp", "drummer_1", "kick")]["est"] == []


def test_stale_files_without_a_built_tool_fail(tmp_path, monkeypatch, capsys):
    root = tmp_path / "root"
    root.mkdir()
    ann = root / "ann.txt"
    ann.write_text("")
    monkeypatch.setattr(enst_gate.enst, "index_items",
                        lambda r: {"drummer_1": {"003_phrase_A_tak1.txt": {"ann": ann}}})
    _stale_fixture(tmp_path, "0000000000000000")
    try:
        enst_gate.do_gate(str(root), tmp_path / "out", tmp_path / "baseline.json",
                          tool=str(tmp_path / "absent"), log=print)
        raise AssertionError("stale files without a tool must FAIL")
    except SystemExit as e:
        assert "not built" in str(e)
