"""P4-14 stage 2: the tom bleed rule of the app (`crates/core/src/detect/tom_bleed.rs`, through `parity tom-bleed`)
against the Python reference `tools/tom_bleed_cd.py` `rule_cd` on synthetic sessions, and the reference against stage
1c's CD measures. P4-17 stage 2: rule version 2 (the kick and snare close mics as sources) against
`tools/tom_bleed_sk.py` `rule_v2`."""
import json
import subprocess
import sys
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

from core_tools import PARITY, require

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import tom_bleed_cd as cd  # noqa: E402
import tom_bleed_sk as sk  # noqa: E402

SR = 48000
FREQS = (140.0, 110.0, 85.0, 70.0)


def burst(x, at, amp, freq):
    n = min(len(x) - at, SR // 5)
    k = np.arange(n) / SR
    x[at:at + n] += (amp * np.sin(2 * np.pi * freq * k) * np.exp(-k / 0.08)).astype(np.float32)


def kit(plays, leak, gains, listed, seconds=20, seed=3, freqs=FREQS):
    """Tom close mics: `plays[d]` the (time, amplitude) hits of drum d, heard on mic m at `leak(d, m)` 1 ms later and
    scaled by `gains[m]`; each mic lists its own hits and the other drums' hits of at least `listed` amplitude."""
    n = SR * seconds
    k = len(plays)
    x = [np.zeros(n, np.float32) for _ in range(k)]
    hits = [[] for _ in range(k)]
    for d, play in enumerate(plays):
        for t, a in play:
            at = int(t * SR)
            for m in range(k):
                amp, s = (a * gains[m], at) if m == d else (a * leak(d, m) * gains[m], at + SR // 1000)
                burst(x[m], s, amp, freqs[d % len(freqs)])
                if m == d or amp >= listed:
                    hits[m].append(s)
    rng = np.random.default_rng(seed)
    for v in x:
        v += (rng.uniform(-1, 1, n) * 1e-4).astype(np.float32)
    return x, [sorted(h) for h in hits]


def part(t0, step, n):
    return [(t0 + i * step, 0.1 if i % 3 == 2 else 0.5) for i in range(n)]


def run_both(tmp_path, x, hits, drums=None, threshold=None):
    """(Rust answer, Python reference) for one session."""
    ids = [f"t{i + 1}" for i in range(len(x))]
    drums = drums or ids
    mics = []
    for i, (sig, h) in enumerate(zip(x, hits)):
        p = tmp_path / f"{ids[i]}.wav"
        sf.write(str(p), sig, SR, subtype="FLOAT")
        mics.append({"id": ids[i], "drum": drums[i], "path": str(p), "hits": [int(v) for v in h]})
    job = {"mics": mics}
    if threshold is not None:
        job["threshold_db"] = threshold
    jf, out = tmp_path / "job.json", tmp_path / "out.json"
    jf.write_text(json.dumps(job))
    r = subprocess.run([str(require(PARITY)), "tom-bleed", str(jf), str(out)], capture_output=True, text=True)
    assert r.returncode == 0, r.stderr[-800:]
    rust = json.loads(out.read_text())
    py = cd.rule_cd({i: s for i, s in zip(ids, x)}, {i: np.asarray(h) / SR for i, h in zip(ids, hits)}, SR,
                    drums=dict(zip(ids, drums)), **({"threshold": threshold} if threshold is not None else {}))
    return rust, py


def same(rust, py):
    """The Rust answer equals the reference: measures to 1e-9 dB, shares, flags, gains and the off state."""
    for m in rust["mics"]:
        ref = py["mics"][m["id"]]
        for k in ("strong", "level_ok", "played", "silent"):
            assert m[k] == ref[k], (m["id"], k, m[k], ref[k])
        for k in ("level_db", "louder_share", "silent_share"):
            assert (m[k] is None) == (ref[k] is None), (m["id"], k)
            if m[k] is not None:
                assert abs(m[k] - ref[k]) < 1e-9, (m["id"], k, m[k], ref[k])
        assert np.allclose(m["excess"], py["excess"][m["id"]], atol=1e-9, rtol=0), m["id"]
        assert m["bleed"] == [bool(b) for b in py["bleed"][m["id"]]], m["id"]
    got = {(g["source"], g["target"]): (g["gain_db"], g["hits"], g["quiet_only"]) for g in rust["transfers"]}
    assert set(got) == set(py["gain"])
    for k, (g, n, q) in py["gain"].items():
        assert abs(got[k][0] - g) < 1e-9 and got[k][1:] == (n, q), (k, got[k], (g, n, q))
    assert [tuple(x) for x in rust["merged"]] == [tuple(x) for x in py["merged"]]
    off = None if rust["off"] is None else (rust["off"]["kind"], rust["off"]["mics"])
    assert off == (None if py["off"] is None else (py["off"][0], list(py["off"][1])))


def test_three_played_toms_match_the_reference(tmp_path):
    x, hits = kit([part(0.5, 0.9, 20), part(0.8, 0.9, 20), part(1.1, 0.9, 20)], lambda d, m: 0.1, [1, 1, 1], 0.009)
    rust, py = run_both(tmp_path, x, hits)
    same(rust, py)
    assert rust["off"] is None and sum(sum(m["bleed"]) for m in rust["mics"]) == 120
    assert rust["merged"] == [], "played pairs stay two drums"   # every listed bleed hit (3 drums x 20 hits x 2 other mics)
    # the threshold decides: a high one marks own hits too, and both sides agree on it
    rust, py = run_both(tmp_path, x, hits, threshold=45.0)
    same(rust, py)
    assert sum(sum(m["bleed"]) for m in rust["mics"]) > 120


@pytest.mark.parametrize("gain_db", [0.0, 15.6, 29.0, 35.0])
def test_a_bleed_only_mic_matches_the_reference_and_takes_no_real_hit(tmp_path, gain_db):
    """Mic 3 hears only drum 1 (at -20 dB): as recorded it fails the level test; with a hot preamp (+15.6 dB, and
    drum 1 louder on it than on its own mic by 9 and 15 dB) the shared pair counts as one drum (fix round 3)."""
    leak = lambda d, m: 0.001 if (m == 2 and d == 1) else 0.1   # noqa: E731
    x, hits = kit([part(0.5, 0.9, 20), part(0.8, 0.9, 20), []], leak, [1, 1, 10 ** (gain_db / 20)], 0.009)
    rust, py = run_both(tmp_path, x, hits)
    same(rust, py)
    assert rust["off"] is None
    for m in (0, 1):
        own = {int(t * SR) for t, _ in part(0.5 + 0.3 * m, 0.9, 20)}
        assert not any(b for h, b in zip(hits[m], rust["mics"][m]["bleed"]) if h in own), (gain_db, m)
    of_drum1 = [b for h, b in zip(hits[2], rust["mics"][2]["bleed"])
                if any(abs(h - int(t * SR)) < 100 for t, _ in part(0.5, 0.9, 20))]
    assert of_drum1
    if gain_db == 0.0:
        assert rust["merged"] == [] and not rust["mics"][2]["played"] and all(of_drum1)
    else:
        assert rust["merged"] == [["t1", "t3"]] and not any(of_drum1)


def test_the_off_cases_match_the_reference(tmp_path):
    # one played tom
    x, hits = kit([part(0.5, 0.9, 20), []], lambda d, m: 0.1, [1, 1], 0.009)
    rust, py = run_both(tmp_path, x, hits)
    same(rust, py)
    assert rust["off"] == {"kind": "played", "mics": ["t1", "t2"]} and rust["merged"] == [["t1", "t2"]]
    # a gated mic
    plays = [part(0.5, 0.9, 20), part(0.8, 0.9, 20), part(1.1, 0.9, 20)]
    x, hits = kit(plays, lambda d, m: 0.1, [1, 1, 1], 0.009)
    own = [int(t * SR) for t, _ in plays[2]]
    keep = np.zeros(len(x[2]), bool)
    for s in own:
        keep[max(0, s - SR // 50):s + SR // 5] = True
    x[2][~keep] = 0.0
    hits[2] = own
    rust, py = run_both(tmp_path, x, hits)
    same(rust, py)
    assert rust["off"] == {"kind": "silence", "mics": ["t3"]}
    # two mics of one drum
    x, hits = kit([part(0.5, 0.9, 20), part(0.5, 0.9, 20)], lambda d, m: 0.0, [1, 1], 1.0)
    rust, py = run_both(tmp_path, x, hits, drums=["tom1", "tom1"])
    same(rust, py)
    assert rust["off"]["kind"] == "played" and rust["transfers"] == []


def test_hits_at_the_ends_and_the_quiet_window_match_the_reference(tmp_path):
    # hits 1 ms from the start and 2 ms from the end: the windows run over the signal, both sides pad zeros
    p0 = [(0.001, 0.5)] + [(0.5 + i * 0.9, 0.5) for i in range(12)]
    p1 = [(0.505 + i * 0.9, 0.5) for i in range(0, 12, 2)] + [(11.998, 0.5)]
    x, hits = kit([p0, p1], lambda d, m: 0.1, [1, 1], 1.0, seconds=12)
    rust, py = run_both(tmp_path, x, hits)
    same(rust, py)
    g = {(t["source"], t["target"]): t for t in rust["transfers"]}
    assert (g[("t1", "t2")]["hits"], g[("t1", "t2")]["quiet_only"]) == (7, True)
    assert g[("t2", "t1")]["quiet_only"] is False


def test_rule_cd_equals_the_stage_1c_measures_when_every_tom_is_played():
    x, hits = kit([part(0.5, 0.9, 20), part(0.8, 0.9, 20), part(1.1, 0.9, 20)], lambda d, m: 0.1, [1, 1, 1], 0.009)
    sig = {f"t{i + 1}": s for i, s in enumerate(x)}
    times = {f"t{i + 1}": np.asarray(h) / SR for i, h in enumerate(hits)}
    meas, ref, play, gain = cd.session_measures(sig, times, SR)
    r = cd.rule_cd(sig, times, SR)
    assert all(play.values()) and all(v["played"] for v in r["mics"].values())
    for m in sig:
        assert np.allclose(r["excess"][m], meas[m]["exc_cd"], atol=1e-12)
        assert abs(r["mics"][m]["level_db"] - ref[m]) < 1e-12
    assert {k: v[0] for k, v in r["gain"].items()} == {k: v for k, v in gain.items() if v is not None}



def sk_session(unison=0.05):
    """Three toms, a snare (200 Hz) and a kick (55 Hz), 300 ms apart; the snare reaches the tom mics at 0.3, the kick and
    the other toms at 0.1, the toms the snare and kick mics at 0.05. Tom 1 also plays soft hits with every other loud
    kick hit (a unison)."""
    kick = [(1.7 + 1.5 * i, 0.1 if i % 3 == 2 else 0.5) for i in range(12)]
    t1 = [(0.5 + 1.5 * i, 0.1 if i % 3 == 2 else 0.5) for i in range(12)]
    t1 += [(t, unison) for i, (t, a) in enumerate(kick) if i % 2 == 0 and a > 0.2]
    plays = [t1] + [[(t0 + 1.5 * i, 0.1 if i % 3 == 2 else 0.5) for i in range(12)] for t0 in (0.8, 1.1, 1.4)] + [kick]

    def leak(d, m):
        if d == 3 and m <= 2:
            return 0.3
        if d <= 2 and m >= 3:
            return 0.05
        return 0.1
    return kit(plays, leak, [1] * 5, 0.009, freqs=(140.0, 110.0, 85.0, 200.0, 55.0))


def test_version_2_matches_the_reference_and_keeps_the_kick_unison(tmp_path):
    x, hits = sk_session()
    ids, kinds = ["t1", "t2", "t3"], {"s": "snare", "k": "kick"}
    paths = {}
    for i, sig in zip(ids + ["s", "k"], x):
        paths[i] = tmp_path / f"{i}.wav"
        sf.write(str(paths[i]), sig, SR, subtype="FLOAT")
    job = {"mics": [{"id": i, "path": str(paths[i]), "hits": [int(v) for v in hits[n]]} for n, i in enumerate(ids)],
           "sources": [{"id": i, "kind": kinds[i], "path": str(paths[i]), "hits": [int(v) for v in hits[n]]}
                       for n, i in ((3, "s"), (4, "k"))]}
    jf, out = tmp_path / "job.json", tmp_path / "out.json"
    jf.write_text(json.dumps(job))
    r = subprocess.run([str(require(PARITY)), "tom-bleed", str(jf), str(out)], capture_output=True, text=True)
    assert r.returncode == 0, r.stderr[-800:]
    rust = json.loads(out.read_text())
    sources = {"snare": (x[3], np.asarray(hits[3]) / SR), "kick": (x[4], np.asarray(hits[4]) / SR)}
    py, exc, notes, marks = sk.rule_v2({i: x[n] for n, i in enumerate(ids)},
                                        {i: np.asarray(hits[n]) / SR for n, i in enumerate(ids)}, sources, SR)
    assert rust["off"] is None and py["off"] is None
    for n, m in enumerate(rust["mics"]):
        i = m["id"]
        assert m["bleed"] == [bool(b) for b in marks[i]], i
        assert m["bleed"] != [bool(b) for b in py["bleed"][i]], f"{i}: the sources mark more than rule CD"
        se = np.asarray(m["source_excess"], float)
        for k, kind in enumerate(("snare", "kick")):
            want = exc[i][kind] if kind in exc[i] else np.full(len(hits[n]), cd.NO_OTHER)
            assert np.allclose(se[:, k], want, atol=1e-9, rtol=0), (i, kind)
    use = {(u["tom"], u["source"]): u for u in rust["sources"]}
    for i in ids:
        for sid, kind in kinds.items():
            u, nt = use[(i, sid)], notes[i][kind]
            for a, b in ((u["gain_db"], nt["g_sa"]), (u["back_db"], nt["g_as"]), (u["silent_share"], nt["silent"])):
                assert (a is None) == (b is None) and (a is None or abs(a - b) < 1e-9), (i, kind, a, b)
            assert u["unused"] == (None if nt["used"] else nt["why"]), (i, kind)
    # the unison hits of tom 1 stay: their kick excess lies between 4.0 and 9.2 dB
    t1 = rust["mics"][0]
    uni_t = {int((1.7 + 1.5 * i) * SR) for i in range(0, 12, 2) if i % 3 != 2}   # the loud kick hits of the unison
    uni = [j for j, h in enumerate(hits[0]) if h in uni_t]
    assert len(uni) == 4
    for j in uni:
        assert sk.KICK_DB <= t1["source_excess"][j][1] < sk.SNARE_DB and not t1["bleed"][j], j
