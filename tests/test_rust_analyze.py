"""P3-2a: the analysis pipeline after the detector in the Rust core (`service_tool core-analyze`:
detect_events and analyze_detected of the app's job, P3-12).

Reference: engine.analyze_track of the retired server in the Rust mode as it ran before P3-2a,
that is engine._rust_detect plus the Python steps engine._events_after_detection with the Rust
refinement and velocities, frozen in tests/golden/frozen/rust_analyze.json (key: file, detector,
role, params; inputs: the source SHA-256, the params and the model SHA-256). Events must be
identical: t, v, rise and order (json.dumps keeps the sign of zero and every float bit). The
corpus-wide check with real tracks is tools/parity/check_analyze.py.
"""
import hashlib
import json
import os

import pytest

import frozen
import golden_corpus
from core_tools import core_analyze
from helpers import SR, synth_track, write_wav

STORE = frozen.store("rust_analyze")

# The frozen references predate the A1 snare soft gate (P3-12 recorded the retired server,
# which has no soft rescue). Pin the soft gate off: DRUMSPIKE_A1_SOFT_SIM=0. The A1 behavior
# itself is covered by the soft-gate unit tests and the a1-gate (enst, fix_metric).
# An autouse fixture, not a module-level os.environ set: a permanent set leaks into the
# other core_analyze callers of the same pytest process and changed what their DSP detector
# finds (the demo-session truth test read 46 snares instead of 50 with the gate pinned off).
@pytest.fixture(autouse=True)
def _pin_soft_gate_off(monkeypatch):
    monkeypatch.setenv("DRUMSPIKE_A1_SOFT_SIM", "0")

ONSETS = [0.25, 0.75, 1.25, 1.75, 2.25, 2.75]
PARAMS = [
    ({}, "kick"),
    ({"refine": False}, "other"),
    ({"shift_samples": 37, "min_gap_ms": 45, "refine_lo_ms": -7, "refine_hi_ms": 7,
      "vel_lo_db": -50, "vel_hi_db": -3}, "kick"),
    ({"shift_samples": -480, "thresholds": [0.3, 0.3, 0.3, 0.3, 0.3]}, "bass"),
    ({"min_gap_ms": 5, "shift_samples": 12.7}, "snare"),
]
DETECTOR = {"dsp": "rust-dsp"}


def sha_of(path):
    return hashlib.sha256(open(path, "rb").read()).hexdigest()


@pytest.fixture(params=["dsp"])
def detector(request):
    """(name, None): the DSP detector is the only one on main (#281 D16)."""
    return request.param, None


@pytest.fixture(scope="module")
def corpus(tmp_path_factory):
    out = tmp_path_factory.mktemp("analyze_corpus")
    golden_corpus.write_corpus(out)
    return out


def reference(path, role, params, d, model):
    """(events, detector) of the retired server for one file, role, params and detector."""
    inputs = {"source_sha256": sha_of(path), "role": role, "params": params, "detector": d,
              "model_sha256": None}
    r = STORE.ref(f"{os.path.basename(path)}|{d}|{role}|{json.dumps(params, sort_keys=True)}", inputs)
    return r["events"], r["detector"]


def jobs_for(path, model):
    return [{"wav": str(path), "role": role, "params": params, "model": None} for params, role in PARAMS]


def same(a, b):
    return json.dumps(a) == json.dumps(b)


def test_the_core_analysis_equals_the_python_steps(detector, kick_wav):
    d, model = detector
    src, idx = kick_wav
    for (params, role), res in zip(PARAMS, core_analyze(jobs_for(src, model))):
        ref, det = reference(src, role, params, d, model)
        assert "error" not in res, res
        assert same(res["events"], ref), (params, role, res["events"], ref)
        assert res["detector"] == det == DETECTOR[d]
    res = core_analyze([{"wav": str(src), "role": "kick", "params": {}, "model": None}])[0]
    got = [round(e["t"] * SR) for e in res["events"]]
    assert len(got) == len(idx) and all(abs(g - i) <= 24 for g, i in zip(got, idx)), (got, idx)
    assert all(e["rise"] == round(e["rise"], 1) for e in res["events"])


@pytest.mark.parametrize("name", ["pcm16_mono.wav", "float32_mono_96k.wav", "pcm16_mono_44k.wav",
                                  "pcm24_stereo_lr.wav", "pcm16_partial5000.wav", "pcm16_short100.wav"])
def test_the_corpus_gives_identical_events(detector, corpus, name):
    d, model = detector
    for (params, role), res in zip(PARAMS, core_analyze(jobs_for(corpus / name, model))):
        ref, _ = reference(corpus / name, role, params, d, model)
        assert "error" not in res, res
        assert same(res["events"], ref), (name, params, role)


def test_analyze_detected_counts_every_branch(kick_wav):
    src, _ = kick_wav
    det = []
    for t in ONSETS:
        det += [(t - 0.006, 35), (t + 0.4, 38)]
    det += [(0.749, 35), (1.2385, 35), (1.2585, 35)]
    det = [list(d) for d in det if d[0] != 1.25 - 0.006]
    total = {}
    jobs = [{"wav": str(src), "role": role, "params": params, "model": None, "detected": det} for params, role in PARAMS]
    for (params, role), res in zip(PARAMS, core_analyze(jobs)):
        ref = STORE.ref(f"detected|{role}|{json.dumps(params, sort_keys=True)}",
                        {"source_sha256": sha_of(src), "detected": [tuple(d) for d in det], "role": role, "params": params})
        assert "error" not in res, res
        assert same(res["events"], ref), (params, role)
        for k, v in res["branches"].items():
            total[k] = total.get(k, 0) + v
    assert set(total) == {"tracks", "detected", "filter_runs", "filtered_out", "dedup_dropped", "refine_off",
                          "refine_moved", "rise_none", "dedup2_dropped", "shifted", "events",
                          "reach_moved", "tail_moved", "reach_unresolved",   # P4-12: the reach rule's steps
                          "back_moved", "back_busy", "back_separate"}        # P4-12 stage 2: the back-track's
    assert total["reach_moved"] == total["tail_moved"] == total["reach_unresolved"] == 0   # the engine's rule (1)
    assert total["back_moved"] == total["back_busy"] == total["back_separate"] == 0
    for k in ("filter_runs", "filtered_out", "dedup_dropped", "refine_off", "refine_moved", "dedup2_dropped",
              "shifted"):
        assert total[k] > 0, (k, total)


def test_the_refinement_rule_is_a_job_option(kick_wav):
    """P4-12: `rule` 2 runs the reach rule after the detector; no `rule` (or 1) is the engine's, frozen above. Coarse
    times 17 ms before each attack (the DSP frame before it): rule 1 gives them back, rule 2 finds the attacks."""
    src, idx = kick_wav
    det = [[t - 0.017, 35] for t in ONSETS]
    old, one, new, three, bad = core_analyze([{"wav": str(src), "role": "kick", "model": None, "detected": det, "rule": r}
                                              for r in (None, 1, 2, 3, 7)])
    assert same(old["events"], one["events"]) and old["branches"]["reach_moved"] == 0
    assert [round(e["t"] * SR) for e in old["events"]] == [round((t - 0.017) * SR) for t in ONSETS]
    assert new["branches"]["reach_moved"] == len(ONSETS), new["branches"]
    got = [round(e["t"] * SR) for e in new["events"]]
    assert all(abs(g - i) <= 24 for g, i in zip(got, idx)) and len(got) == len(idx), (got, idx)
    # P4-12 stage 2: rule 3 is rule 2 and then the back-track: each hit moves back to the first motion of its attack
    assert new["branches"]["back_moved"] == 0 and three["branches"]["back_moved"] == len(ONSETS), three["branches"]
    first = [round(e["t"] * SR) for e in three["events"]]
    assert all(f < g and 0 <= f - i <= 8 for f, g, i in zip(first, got, idx)) and len(first) == len(idx), (first, got, idx)
    assert "unknown rule" in bad.get("error", ""), bad


def test_an_unsupported_rate_is_an_error(tmp_path):
    x, _ = synth_track(ONSETS, 3.5, sr=22050)
    src = write_wav(tmp_path / "kick22k.wav", x, sr=22050)
    a, b = core_analyze([{"wav": str(src), "role": "kick", "model": None},
                         {"wav": str(src), "role": "kick", "model": None, "detected": [[0.25, 35]]}])
    assert "22050" in a.get("error", ""), a
    assert "22050" in b.get("error", ""), b


