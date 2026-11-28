"""#587: a Learn rule acts on the whole DAW track (DAW plugin M1b14).

In a DAW one row of the page is one DAW track, and it can hold clips of several audio files. A Learn mark made in one clip is copied to the other audio
files of the row (the service command track_learn_adopt measures its examples on the audio of the file where it was made), so Learn This and Forget This
act on the whole track, and one action is ONE undo step (a group of the steps of the files). These tests start the page the way the plugin shell does
(GET /?host_mode=1) and send the rows the plugin sends. Every host-mode test has a standalone twin that proves the standalone page did not change, and
a fault run (HT.fault) that breaks the rule it tests. The fixture is synthetic: real hits (short attacks, loud) and bleed-like hits (long attacks,
softer) in two audio files. The design is docs/design/daw-plugin/m1b14-learn-row.md."""
import pytest

from app_support import inspect_track, invoke, open_project
from helpers import synth_track, write_wav
from test_host_page import host_open
from test_subtracks import analyse

REAL = [0.5, 1.5, 2.5, 3.5]
BLEED = [1.0, 2.0, 3.0]
A, B = "Kick In", "Kick Sub"
DUR = 6.0


def build(make_project, tmp_path, tag="Row"):
    src = tmp_path / f"src_{tag}"
    src.mkdir()
    on = sorted(REAL + BLEED)
    specs = []
    for n in (A, B):
        x, _ = synth_track(on, DUR, gains_db=[-4.0 if t in REAL else -9.0 for t in on],
                           attacks_ms=[0.5 if t in REAL else 12.0 for t in on],
                           phases=[0.0 if t in REAL else 0.35 for t in on])
        specs.append((n, write_wav(src / f"{n}.wav", x), "kick", 36, None))
    return make_project.builder.build_project(tmp_path / f"projects_{tag}", tag, specs, "copied")


def rows(both=True, gen=1):
    """One DAW track with both files (the second 10 s later in the song), or two DAW tracks with one file each."""
    clip = lambda n, s: {"src": n, "a": 0.0, "b": DUR, "s": s, "k": 1.0}
    if both:
        return {"gen": gen, "song_end": 16.0, "gone": [], "rows": [{"id": "q1", "name": "Drums", "srcs": [A, B], "clips": [clip(A, 0.0), clip(B, 10.0)]}]}
    return {"gen": gen, "song_end": 16.0, "gone": [], "rows": [{"id": "q1", "name": "Drums", "srcs": [A], "clips": [clip(A, 0.0)]},
                                                               {"id": "q2", "name": "Drums 2", "srcs": [B], "clips": [clip(B, 10.0)]}]}


def sens150(page):
    inspect_track(page, A)
    page.locator("#insParams .pSens").fill("150")
    page.locator("#insParams .pSens").dispatch_event("change")
    page.wait_for_function("() => !hgRepickBusy()")
    page.evaluate("() => { if (document.activeElement && document.activeElement.blur) document.activeElement.blur(); }")


def take(page, p):
    page.evaluate("p => HT.take(p)", p)
    page.wait_for_function("() => !FR.raf && FR.flags === 0")


@pytest.fixture
def host(host_app, make_project, tmp_path):
    host_open(host_app, build(make_project, tmp_path))
    take(host_app, rows())
    analyse(host_app)
    sens150(host_app)
    yield host_app
    host_app.evaluate("() => { HT.rows = []; HT.gen = 0; HT.fault = ''; HT.rebuild(); HT.paint(); }")


@pytest.fixture
def plain(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path, "Twin"))
    analyse(app)
    sens150(app)
    # the sensitivity of the standalone page reaches one track only: bring the second track to the same state
    inspect_track(app, B)
    app.locator("#insParams .pSens").fill("150")
    app.locator("#insParams .pSens").dispatch_event("change")
    app.wait_for_function("() => !hgRepickBusy()")
    app.evaluate("() => { if (document.activeElement && document.activeElement.blur) document.activeElement.blur(); }")
    yield app


def idle(page):
    page.evaluate("() => DSLearn.idle()")


def hits(page, n):
    return page.evaluate("n => S.events[n].map(e => [e.t, !!e.filtered])", n)


def grey(page, n):
    return [t for t, f in hits(page, n) if f]


def near(ts, t, tol=0.03):
    return any(abs(x - t) <= tol for x in ts)


def mark(page, n, t, key):
    """Select the hit nearest to t of the file n (as a click does) and press K or Shift+K."""
    i = page.evaluate("([n, t]) => S.events[n].findIndex(e => Math.abs(e.t - t) < 0.03)", [n, t])
    assert i >= 0, f"no hit near {t} in {n}"
    page.evaluate("([n, i]) => { lnSetSel(null); S.sel = { track: n, index: i }; S.playhead = S.events[n][i].t; drawInfo(); }", [n, i])
    page.keyboard.press(key)
    idle(page)


def learn_of(page, sid, n):
    return invoke(page, "tracks_get", sid=sid)["tracks"][n].get("learn")


def top(page):
    return page.evaluate("() => { const o = S.undo[S.undo.length - 1]; return o ? [o.type, o.steps ? o.steps.map(x => x.track) : null, S.undo.length] : null; }")


def sid_of(page):
    return page.evaluate("() => S.sid")


def test_forget_this_on_one_clip_takes_the_same_kind_of_hit_out_of_the_other_audio_file_in_one_undo_step(host):
    sid = sid_of(host)
    before = {n: hits(host, n) for n in (A, B)}
    assert not grey(host, A) and not grey(host, B)
    n0 = host.evaluate("() => S.undo.length")
    mark(host, A, 1.0, "Shift+K")
    assert near(grey(host, A), 1.0), grey(host, A)
    assert near(grey(host, B), 1.0), "the other audio file of the DAW track follows the rule: " + str(grey(host, B))
    assert not any(near(grey(host, B), t, 0.005) for t in REAL), "no real hit goes"
    row = learn_of(host, sid, B)["row"]
    assert [(e["from"], e["kind"]) for e in row] == [(A, "not")] and learn_of(host, sid, B)["on"] is True
    assert top(host) == ["group", [A, B], n0 + 1], "one undo step that holds the step of each audio file"
    text = host.locator("#stMsg").inner_text()
    assert "audio file 1:" in text and "audio file 2:" in text and "hits out of Drums" in text, text
    assert A not in text and B not in text, "the key of a file shows nowhere: " + text
    host.evaluate("() => undo()")
    host.wait_for_function("(n) => S.undo.length === n", arg=n0)
    idle(host)
    assert {n: hits(host, n) for n in (A, B)} == before, "one undo brings back the hits of both files"
    assert not (learn_of(host, sid, A) or {}).get("marks") and not (learn_of(host, sid, B) or {}).get("row")
    host.evaluate("() => redo()")
    try:
        host.wait_for_function("(n) => S.undo.length === n + 1", arg=n0, timeout=5000)
    except Exception:
        raise AssertionError("redo: " + host.locator("#stMsg").inner_text())
    idle(host)
    assert near(grey(host, B), 1.0) and near(grey(host, A), 1.0), "redo does both again"


def test_learn_this_acts_on_both_audio_files_in_one_undo_step(host):
    sid = sid_of(host)
    n0 = host.evaluate("() => S.undo.length")
    mark(host, A, 0.5, "K")
    for n in (A, B):
        l = learn_of(host, sid, n)
        assert l and (l["marks"] if n == A else l["row"]), n
    assert [(e["from"], e["kind"]) for e in learn_of(host, sid, B)["row"]] == [(A, "this")]
    assert top(host) == ["group", [A, B], n0 + 1]
    host.evaluate("() => undo()")
    host.wait_for_function("(n) => S.undo.length === n", arg=n0)
    idle(host)
    assert not (learn_of(host, sid, B) or {}).get("row")


def test_the_fault_run_no_fan_out_leaves_the_other_audio_file_alone(host):
    sid = sid_of(host)
    host.evaluate("() => { HT.fault = 'no-fan-out'; }")
    mark(host, A, 1.0, "Shift+K")
    assert near(grey(host, A), 1.0)
    assert not grey(host, B), "no-fan-out: only the file of the clip takes the rule"
    assert not (learn_of(host, sid, B) or {}).get("row")
    assert top(host)[0] == "learn", "a plain step"


def test_the_standalone_twin_marks_one_track_with_one_plain_undo_step_and_never_calls_adopt(plain):
    sid = sid_of(plain)
    calls = []
    plain.on("request", lambda r: calls.append(r.url.rsplit("/invoke/", 1)[1]) if "/invoke/" in r.url else None)
    n0 = plain.evaluate("() => S.undo.length")
    mark(plain, A, 1.0, "Shift+K")
    assert near(grey(plain, A), 1.0) and not grey(plain, B), "only the marked track changes"
    assert not (learn_of(plain, sid, B) or {}).get("row")
    assert top(plain) == ["learn", None, n0 + 1], "a plain step, no group"
    assert "track_learn_adopt" not in calls
    assert "audio file" not in plain.locator("#stMsg").inner_text()
    assert plain.evaluate("() => document.getElementById('hostLearnNote')") is None
    # the undo and redo of a plain Learn step: the result of lnApplyOp is nothing, as before the ticket
    assert plain.evaluate("() => lnApplyOp(S.undo[S.undo.length - 1], true) === undefined")
    idle(plain)
    assert not grey(plain, A)


def test_a_file_that_joins_the_track_later_does_not_follow_until_the_user_applies(host_app, make_project, tmp_path):
    host_open(host_app, build(make_project, tmp_path, "Join"))
    take(host_app, rows(both=False))
    analyse(host_app)
    sens150(host_app)
    sid = sid_of(host_app)
    mark(host_app, A, 1.0, "Shift+K")
    assert not grey(host_app, B) and top(host_app)[0] == "learn", "two DAW tracks: the rule is for the file of the clip"
    assert host_app.evaluate("() => document.getElementById('hostLearnNote')") is None
    # the clip of the second file moves to the same DAW track
    take(host_app, rows(both=True, gen=2))
    host_app.wait_for_function("() => document.getElementById('hostLearnNote')")
    note = host_app.locator("#hostLearnNote").inner_text()
    assert "1 audio file of this track does not follow its Learn marks yet" in note and "Apply" in note, note
    assert not grey(host_app, B), "nothing changes by itself"
    assert not (learn_of(host_app, sid, B) or {}).get("row")
    n0 = host_app.evaluate("() => S.undo.length")
    host_app.locator("#hostLearnNote a.hostLearnApply").click()
    idle(host_app)
    assert near(grey(host_app, B), 1.0), "Apply: the rule reaches the file"
    assert top(host_app) == ["learn", None, n0 + 1], top(host_app)   # one file to catch up: the plain step
    host_app.wait_for_function("() => !document.getElementById('hostLearnNote')")
    host_app.evaluate("() => undo()")
    idle(host_app)
    assert not grey(host_app, B)
    host_app.evaluate("() => { HT.rows = []; HT.gen = 0; HT.fault = ''; HT.rebuild(); HT.paint(); }")


def test_a_failed_roll_back_step_of_a_group_undo_is_reported(host):
    msg = host.evaluate("""async () => {
      const mk = (t) => ({ type: "x", track: t });
      // the first step is applied, the second is refused, and the first cannot be taken back
      let n = 0;
      const apply2 = (s, rev) => { n++; if (n === 2) return false; if (n === 3) return false; return true; };
      const ok = await groupApplyOp({ steps: [mk("Kick In"), mk("Kick Sub")] }, false, apply2);
      return [ok, document.querySelector("#stMsg").textContent];
    }""")
    assert msg[0] is False
    assert "a step before it could not be taken back (Drums" in msg[1] or "could not be taken back" in msg[1], msg[1]
    # when the roll-back works the old text stays
    msg = host.evaluate("""async () => {
      let n = 0;
      const ok = await groupApplyOp({ steps: [{ type: "x", track: "Kick In" }, { type: "x", track: "Kick Sub" }] }, false, () => (++n === 2 ? false : true));
      return [ok, document.querySelector("#stMsg").textContent];
    }""")
    assert msg[0] is False and "the steps before it were taken back" in msg[1] and "could not" not in msg[1], msg[1]


def test_the_standalone_group_undo_message_is_the_one_of_before(plain):
    msg = plain.evaluate("""async () => {
      let n = 0;
      const ok = await groupApplyOp({ steps: [{ type: "x", track: "Kick In" }, { type: "x", track: "Kick Sub" }] }, true, () => (++n === 2 ? false : true));
      return [ok, document.querySelector("#stMsg").textContent];
    }""")
    assert msg[0] is False and msg[1] == "undo: a step of Kick In was refused; the steps before it were taken back", msg[1]


def refine(page, lo):
    inspect_track(page, A)
    page.evaluate("v => { const f = document.querySelector('#insParams .pRefineLo'); f.value = v; f.dispatchEvent(new Event('change')); }", lo)


def test_the_refine_window_follows_the_row_in_one_undo_step(host):
    n0 = host.evaluate("() => S.undo.length")
    refine(host, -5)
    host.wait_for_function("([a, b]) => S.tracks[a].params.refine_lo_ms === -5 && S.tracks[b].params.refine_lo_ms === -5", arg=[A, B])
    assert top(host) == ["group", [A, B], n0 + 1]
    host.evaluate("() => undo()")
    host.wait_for_function("([a, b]) => S.tracks[a].params.refine_lo_ms !== -5 && S.tracks[b].params.refine_lo_ms !== -5", arg=[A, B])
    host.evaluate("() => { HT.fault = 'no-fan-out'; }")
    refine(host, -7)
    host.wait_for_function("([a]) => S.tracks[a].params.refine_lo_ms === -7", arg=[A])
    assert host.evaluate("([b]) => S.tracks[b].params.refine_lo_ms !== -7", [B]), "no-fan-out: the second file keeps its window"


def test_the_standalone_twin_sets_the_refine_window_of_one_track_with_one_plain_step(plain):
    n0 = plain.evaluate("() => S.undo.length")
    refine(plain, -5)
    plain.wait_for_function("([a]) => S.tracks[a].params.refine_lo_ms === -5", arg=[A])
    assert plain.evaluate("([b]) => S.tracks[b].params.refine_lo_ms !== -5", [B])
    assert top(plain) == ["params", None, n0 + 1]


# ---------------------------------------------------------------- the nudge of a range stays in its clip

def touching():
    """One audio file in two clips that touch in the source (3.0 s) and in the song: the DAW cut it there."""
    return {"gen": 5, "song_end": 6.0, "gone": [], "rows": [{"id": "q1", "name": "Drums", "srcs": [A], "clips": [
        {"src": A, "a": 0.0, "b": 3.0, "s": 0.0, "k": 1.0}, {"src": A, "a": 3.0, "b": 6.0, "s": 3.0, "k": 1.0}]}]}


def range_nudge(page, t0, t1, units):
    page.evaluate("([n, a, b]) => { lnSetSel(n, a, b); }", [A, t0, t1])
    page.evaluate("u => nudgeUnits(u)", units)
    idle(page)


def test_a_range_nudge_stops_at_the_edge_of_its_clip_when_two_clips_of_one_file_touch(host_app, make_project, tmp_path):
    host_open(host_app, build(make_project, tmp_path, "Nudge"))
    take(host_app, touching())
    analyse(host_app)
    take(host_app, touching())
    host_app.evaluate("() => { const x = S.events['Kick In'].find(e => Math.abs(e.t - 2.5) < 0.03); S.nudgeValue = '10ms'; }")
    t_before = host_app.evaluate("() => S.events['Kick In'].filter(e => e.t > 2.4 && e.t < 3.1).map(e => e.t)")
    # a hit 0.5 s before the join, nudged by far more than 0.5 s: 60 x 10 ms
    host_app.evaluate("() => { lnSetSel('Kick In', 2.4, 2.6); nudgeUnits(60); }")
    idle(host_app)
    host_app.wait_for_function("() => !S.events['Kick In'].some(e => Math.abs(e.t - 2.5) < 0.03)")
    inside = host_app.evaluate("() => S.events['Kick In'].filter(e => e.t > 2.4 && e.t < 3.0).map(e => e.t)")
    after = host_app.evaluate("() => S.events['Kick In'].filter(e => e.t >= 3.0 && e.t < 3.5).map(e => e.t)")
    assert len(inside) == 1 and inside[0] > 2.99, ("the hit stops at the end of its clip", inside)
    assert after == [t for t in t_before if t >= 3.0], ("the hit of the second clip did not move and nothing came over", after)
    host_app.evaluate("() => { HT.rows = []; HT.gen = 0; HT.fault = ''; HT.rebuild(); HT.paint(); }")


def test_the_standalone_range_nudge_is_not_clamped(plain):
    plain.evaluate("() => { S.nudgeValue = '10ms'; lnSetSel('Kick In', 2.4, 2.6); nudgeUnits(60); }")
    idle(plain)
    plain.wait_for_function("() => !S.events['Kick In'].some(e => Math.abs(e.t - 2.5) < 0.03)")
    moved = plain.evaluate("() => S.events['Kick In'].filter(e => e.t > 2.9).map(e => e.t)")
    assert any(abs(t - 3.1) < 0.02 for t in moved), moved


# ---------------------------------------------------------------- review round 1

def test_a_refused_file_stays_in_the_message_of_the_action(host):
    sid = sid_of(host)
    host.route("**/invoke/track_learn_adopt", lambda route: route.fulfill(status=400, content_type="application/json", body='{"code":400,"message":"planted refusal"}'))
    mark(host, A, 1.0, "Shift+K")
    host.unroute("**/invoke/track_learn_adopt")
    assert near(grey(host, A), 1.0) and not grey(host, B)
    assert not (learn_of(host, sid, B) or {}).get("row")
    text = host.locator("#stMsg").inner_text()
    assert "audio file 2 of 2 of Drums: not applied" in text and "planted refusal" in text, text
    assert host.evaluate("() => _msgLevel") == "error", "the error style"


def test_the_standalone_message_of_a_mark_is_the_old_one(plain):
    mark(plain, A, 1.0, "Shift+K")
    text = plain.locator("#stMsg").inner_text()
    assert "not applied" not in text and "audio file" not in text, text
    assert text.startswith(f"{A}: Forget This on 1 hit"), text


def test_after_remove_example_on_the_origin_a_hint_shows_and_apply_takes_the_copy_back(host):
    sid = sid_of(host)
    mark(host, A, 1.0, "Shift+K")
    assert near(grey(host, B), 1.0) and host.evaluate("() => document.getElementById('hostLearnNote')") is None
    mid = learn_of(host, sid, A)["marks"][0]["id"]
    host.evaluate("([n, id]) => learnRemove(n, id)", [A, mid])
    idle(host)
    host.wait_for_function("() => document.getElementById('hostLearnNote')")
    assert "does not follow its Learn marks yet" in host.locator("#hostLearnNote").inner_text()
    host.locator("#hostLearnNote a.hostLearnApply").click()
    idle(host)
    host.wait_for_function("() => !document.getElementById('hostLearnNote')")
    assert not (learn_of(host, sid, B) or {}).get("row"), "the copy of the removed mark is gone"
    assert not grey(host, B), "and file b is no longer filtered by it"
