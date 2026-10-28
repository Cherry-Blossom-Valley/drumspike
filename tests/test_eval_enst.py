"""Tests for tools/eval/enst.py (#276 part B): the file name parser (incl. the xN suffix), the label mapping and
the scorer with planted hits. No ENST data and no mir_eval needed."""
import json
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools" / "eval"))

import enst  # noqa: E402
from enst import track_name  # noqa: E402


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
