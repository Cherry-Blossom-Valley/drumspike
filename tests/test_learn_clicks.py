"""P4-34: the click simulation of tools/learn_clicks.py on a synthetic track, with a planted fault. No app, no audio."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))
import learn_clicks as lc  # noqa: E402


def make(n_real=20, n_fp=5):
    ref = [1.0 + i for i in range(n_real)]
    ev = [{"t": t, "v": 100} for t in ref]
    fps = [{"t": 0.5 + 3.0 * j + 0.001 * j, "v": 60 + j} for j in range(n_fp)]   # far from every reference hit
    return ref, ev + fps


def engine_for(events, fault=False):
    """The app's filter as measured in P4-6c: a "not" click greys the marked hit only; a "this" click changes nothing.
    fault=True plants a bug: a "not" click also greys the real hit next to it."""
    state = [dict(e) for e in events]

    def click(kind, t):
        if kind == "not":
            for e in state:
                if abs(e["t"] - t) < 0.002 or (fault and 0 < e["t"] - t < 0.6):
                    e["filtered"] = True
        return [dict(e) for e in state]
    return click


def test_each_not_click_saves_one_fix_until_no_false_hit_is_left():
    ref, ev = make()
    r = lc.simulate_track(engine_for(ev), ref, ev, n_clicks=8)
    fixes = [x["fixes"] for x in r["rows"]]
    assert fixes[:6] == [5, 4, 3, 2, 1, 0]
    assert [x["click"][0] for x in r["rows"][1:6]] == ["not"] * 5
    assert fixes[6:] == [0, 0, 0]
    assert [x["click"][0] for x in r["rows"][6:]] == ["this"] * 3          # then K on a matching hit, no change
    assert max(x["missed"] for x in r["rows"]) == 0
    assert lc.at(r["rows"], 99) == r["rows"][-1]


def test_the_strongest_false_hit_is_clicked_first_and_a_marked_hit_is_skipped():
    ref, ev = make()
    off = 0.0
    assert lc.choose_click(ev, ref) == ("not", max((e for e in ev if e["v"] < 100), key=lambda e: e["v"])["t"])
    ev2 = [e for e in ev if e["v"] >= 100]                                  # no false hit: K on the weakest real hit
    ev2[3]["v"] = 10
    assert lc.choose_click(ev2, ref) == ("this", ev2[3]["t"])
    assert lc.choose_click(ev2, ref, marked=[ev2[3]["t"]])[1] != ev2[3]["t"]
    assert lc.choose_click([], ref) is None and off == 0.0


def test_planted_fault_real_hits_lost_is_seen():
    ref, ev = make()
    good = lc.summarize([dict(song="s", mic="t1", role="toms", n_ref=20, **lc.simulate_track(engine_for(ev), ref, ev, 5))])
    bad = lc.summarize([dict(song="s", mic="t1", role="toms", n_ref=20,
                             **lc.simulate_track(engine_for(ev, fault=True), ref, ev, 5))])
    assert good["roles"]["toms"]["lost_total"] == 0
    assert bad["roles"]["toms"]["lost_total"] > 0                           # the planted fault greys real hits
    assert bad["roles"]["toms"]["fixes"][5] > good["roles"]["toms"]["fixes"][5]


def test_a_filtered_hit_is_not_in_the_output_and_fp_only_stops_at_the_first_k():
    ref, ev = make()
    r = lc.simulate_track(engine_for(ev), ref, ev, n_clicks=8)
    t = dict(song="s", mic="t1", role="toms", n_ref=20, **r)
    assert len(lc.fp_only(t)["rows"]) == 6                                  # k = 0 .. 5
    assert lc.row_of(ref, [dict(e, filtered=True) for e in ev], 0.0, 0, None)["n_out"] == 0
    kinds = lc.click_kinds([t])
    assert kinds[("toms", "not")]["eq1"] == 5 and kinds[("toms", "this")]["zero"] == 3


def test_not_only_clicks_shift_k_until_no_false_hit_is_left_and_never_k():
    ref, ev = make()
    r = lc.simulate_track(engine_for(ev), ref, ev, n_clicks=8, not_only=True)
    assert [x["click"][0] for x in r["rows"][1:]] == ["not"] * 5          # P4-37: Shift+K only, then the track stops
    assert [x["fixes"] for x in r["rows"]] == [5, 4, 3, 2, 1, 0]
    assert lc.choose_click([e for e in ev if e["v"] >= 100], ref, not_only=True) is None


def test_added_false_names_the_source_mic_and_skips_real_hits():
    """#81: a hit Learn added with no reference hit within the metric's 20 ms is false; its source is the nearest hit
    of another mic within 12 ms. A K-added hit 21 ms before its reference hit is false (the P4-38d data), a learned hit
    at a reference hit is not. Planted fault: with the offset not applied, the real Add-rule hit counts as false."""
    t = {"offset": 0.010, "ref": [1.0, 2.0, 3.0],
         "events": [{"t": 1.010, "v": 90},
                    {"t": 2.011, "v": 30, "learned": True},                  # real (ref 2.0 + offset)
                    {"t": 2.989, "v": 20, "learned": True, "added": True},   # K-add 21 ms early: false
                    {"t": 4.000, "v": 20, "learned": True},                  # bleed of s top, kept
                    {"t": 5.000, "v": 20, "learned": True, "bleed": True},   # bleed of t2, taken out
                    {"t": 6.000, "v": 20, "learned": True}],                 # no source
         "others": {"s top": [3.996], "t2": [4.020, 5.003], "k in": []}}
    rows = lc.added_false(t)
    assert [(r["t"], r["kind"], r["out"], r["source"], r["dt"]) for r in rows] == [
        (2.989, "K", True, None, None), (4.0, "Add", True, "s top", -4.0), (5.0, "Add", False, "t2", 3.0),
        (6.0, "Add", True, None, None)]
    faulty = lc.added_false(dict(t, offset=0.0, ref=[1.0, 1.95]))           # the planted fault: wrong yardstick
    assert any(r["t"] == 2.011 for r in faulty)


def test_planted_fault_add_run_without_project_open_is_refused():
    """#69: the Add run starts with `project_open` (each `Song.run` is a fresh cli process). The fault: no open."""
    names = {"kick": "k__kick"}
    good = lc.add_run_steps(names, ["kick"], 0.9, "/tmp/p")
    assert good[0] == {"command": "project_open", "args": {"path": "/tmp/p"}}
    assert good[1]["command"] == "track_learn_set" and good[1]["args"]["learn"]["add"] == 0.9
    faulty = [s for s in good if s["command"] != "project_open"]         # the planted fault: open missing
    assert faulty[0]["command"] == "track_learn_set"


def test_take_lock_writes_the_owner_file_in_the_host_lock_format(tmp_path, monkeypatch):
    """#69: the host lock reads <dir>/owner (key=value). Without it the lock is stale (empty-after-grace)."""
    import shutil
    import subprocess

    heavy = tmp_path / "heavy.lock.d"
    monkeypatch.setattr(lc, "HEAVY", heavy)
    monkeypatch.setenv("HOST_LOCK_HOST", "arm64-mac")
    monkeypatch.setenv("CREWBOARD_AGENT", "agent@host")
    lc.take_lock("Some Song")
    text = (heavy / "owner").read_text()
    fields = dict(line.split("=", 1) for line in text.splitlines())
    assert fields["agent"] == "agent@host" and fields["host"] == "arm64-mac"
    assert fields["pid"] == str(__import__("os").getpid())
    assert int(fields["started_epoch"]) > 0 and fields["reason"] == "p434 learn_clicks Some Song"
    assert not list(heavy.glob("owner.tmp*"))                                # written by rename
    if shutil.which("crewboard"):                                            # the real reader agrees: held, not stale
        out = subprocess.run(["crewboard", "lock", "status", str(heavy)], capture_output=True, text=True)
        assert out.returncode == 0 and "held" in out.stdout


def test_crewboard_host_lock_caller_lock_is_honored(tmp_path, monkeypatch):
    """#69 option (a): with CREWBOARD_HOST_LOCK set to heavy.lock.d the caller holds the
    lock. learn_clicks must not take, wait for or remove it (crewboard 1d4a51e)."""
    import shutil
    heavy = tmp_path / "heavy.lock.d"
    heavy.mkdir()
    (heavy / "owner").write_text("agent=other@host\npid=999999\n")     # a foreign live holder
    out = tmp_path / "out"
    monkeypatch.setattr(lc, "HEAVY", heavy)
    monkeypatch.setenv("CREWBOARD_HOST_LOCK", str(heavy))
    monkeypatch.setattr(lc, "references",
                        lambda out: {"S": {"mics": {"k": {"role": "kick", "onsets": []}}, "duration_s": 60.0}})
    monkeypatch.setattr(lc.fm, "songs", lambda: {"S": {"mics": {"k": 0}}})

    class FakeSong:
        names = {"k": "k__kick"}
        folder = str(tmp_path / "p")

        def __init__(self, *a):
            self.calls = a

        def setup(self):
            return {"k": []}

        def all_events(self):
            return {"k": []}

        def run(self, steps, tag):
            return [{"result": {}} for _ in steps]

    monkeypatch.setattr(lc, "Song", FakeSong)
    monkeypatch.setattr(lc.time, "sleep",
                        lambda s: (_ for _ in ()).throw(AssertionError("waited for the caller's lock")))
    lc.do_run(out, tool=None, clicks=1, log=lambda *_: None)
    assert (heavy / "owner").is_file()          # not removed: the lock is not ours
    # and without the env, the same call takes the lock and removes it again
    shutil.rmtree(out / "tracks")           # the first run wrote the track; without it the song is skipped
    shutil.rmtree(heavy)                    # the foreign holder is gone; otherwise do_run waits for it
    monkeypatch.delenv("CREWBOARD_HOST_LOCK")
    monkeypatch.setattr(lc.time, "sleep", lambda s: None)
    lc.do_run(out, tool=None, clicks=1, log=lambda *_: None)
    assert not heavy.exists()


def test_explain_click_counts_each_filter_stage_on_a_synthetic_track():
    """#293 diagnosis: the candidates of one Forget click, each with the stage it passed (synthetic, planted cases)."""
    scores = {"state": "ok", "threshold": 0.6, "attack": {"marks": [2.0], "window": [1.0, 5.0]},
              "candidates": [[1.0, "k", 0.9, 80, False, 2.0],    # past the score: the rule keeps it
                             [2.0, "k", 0.2, 80, True, 2.5],     # under the threshold, in the window: greyed, ref-matched
                             [3.0, "k", 0.1, 80, True, 9.0],     # attack off the window: the gate keeps it out of the grey
                             [4.0, "k", 0.3, 80, False, 3.0]]}   # under the threshold, in the window, no hit there yet
    # the shape pairs [template cos, mark cos]: the first candidate is decided by the template (0.5, its cos is higher)
    scores["shape"] = [[0.7, 0.1], [None, 0.4], [0.2, 0.1], [None, -0.2]]
    events = [{"t": 1.0, "v": 80}, {"t": 2.005, "v": 80, "filtered": True}]
    r = lc.explain_click(scores, events, [2.0], 1, 5.0)
    s = r["summary"]
    # #405: candidates 3 and 4 have no detected hit within the tolerance (below the threshold): not counted
    assert s == {"n": 2, "candidates": 4, "past_shape": 1, "via_template": 1, "in_window": 2, "would_grey": 1,
                 "greyed": 1, "ref": 1, "would_grey_ref": 1, "greyed_ref": 1}, s
    assert [c["in_output"] for c in r["candidates"]] == [True, True, False, False]
    assert r["candidates"][1]["cos"] == 0.4 and r["candidates"][1]["via_template"] is False
    assert r["candidates"][0]["via_template"] is True and r["candidates"][3]["cos"] == -0.2
    assert r["window"] == [1.0, 5.0] and r["mark_ms"] == [2.0]
    assert "past shape" in lc.explain_table([dict(r, song="Session A", mic="k in", role="kick")])
    # a track the scores answer "missing" for: no candidate, no threshold, nothing to count
    m = lc.explain_click({"state": "missing"}, events, [2.0], 1, 5.0)
    assert m["threshold"] is None and m["summary"]["n"] == 0 and m["summary"]["would_grey"] == 0
