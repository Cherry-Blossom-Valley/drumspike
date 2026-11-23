"""#555: in host mode the page shows song time (DAW plugin M1b8).

One row of the page is one DAW track; the plugin gives its clips as {src, a, b, s, k} (plugins/ara/src/rows.rs, the shell command
plugin_host_regions and the event plugin-regions). static/hosttime.js holds the rows; the lanes, the hits, the velocity lane, the overview and
the ruler draw song time, and the keys and the commands treat a DAW track as ONE row. These tests start the page the way the plugin shell
does (GET /?host_mode=1) and send the payload the plugin sends. The plugin side has its own stages (plugins/ara/check.py, editor-song-*).
The standalone app has none of it: the last tests.

Each test track has hits at 0.5 s, 1.5 s and 2.5 s of its own audio (6 s long)."""
import pytest

from app_support import open_project
from app_support import inspect_track
from test_histograms import build as hist_build
from test_host_page import FOUR, host_open
from test_subtracks import analyse

NAMES = [n for n, *_ in FOUR]
HITS = [0.5, 1.5, 2.5]
TRACKS = tuple((n, r, k, [{"t": t, "v": 100} for t in HITS]) for n, r, k in FOUR)   # every track has stored hits at 0.5 s, 1.5 s and 2.5 s of its audio


def rows_payload(gen=1):
    """DAW track 1 holds two sources: the first as a whole at song 0, the second CUT (its audio from 1.0 s to 3.0 s) at song 10; DAW track 2 holds
    the third source at song 4; the fourth source has no clip (it has no row of its own: the plugin makes one with an identity clip)."""
    a, b, c, d = NAMES
    return {"gen": gen, "song_end": 12.0, "gone": [], "rows": [
        {"id": "q1", "name": "Drums", "srcs": [a, b], "clips": [
            {"src": a, "a": 0.0, "b": 3.0, "s": 0.0, "k": 1.0}, {"src": b, "a": 1.0, "b": 3.0, "s": 10.0, "k": 1.0}]},
        {"id": "q2", "name": "Toms", "srcs": [c], "clips": [{"src": c, "a": 0.0, "b": 3.0, "s": 4.0, "k": 1.0}]},
        {"id": "s3", "name": d, "srcs": [d], "clips": [{"src": d, "a": 0.0, "b": 6.0, "s": 0.0, "k": 1.0}]}]}


@pytest.fixture
def host(host_app, make_project):
    host_open(host_app, make_project(tracks=TRACKS, seconds=6.0))
    host_app.evaluate("() => { setTrackHeight('large'); }")
    yield host_app
    host_app.evaluate("() => { HT.rows = []; HT.gen = 0; HT.fault = ''; HT.rebuild(); HT.paint(); }")


def take(page, p):
    changed = page.evaluate("p => HT.take(p)", p)
    page.wait_for_function("() => !FR.raf && FR.flags === 0")
    return changed


def line_at(page, row, song_t, lane="spike"):
    """True when the lane of the row has a hit line at the song time (the alpha of the pixel column in the middle of the lane)."""
    return page.evaluate("""([n, t, lane]) => {
      const c = document.querySelector(`.track[data-name="${CSS.escape(n)}"] canvas.${lane}`), dpr = GEO.dpr, W = laneWidth();
      const x = Math.round(tToX(t, W)) + 0.5, d = c.getContext('2d').getImageData(Math.floor(x * dpr), Math.floor(c.height / 2), 1, 1).data;
      return d[3] > 100; }""", [row, song_t, lane])   # the lane background has an alpha of 13; a hit line has more than 100


def test_the_round_trip_between_song_time_and_source_time_is_within_one_sample(host):
    sr = 48000.0
    out = host.evaluate("""sr => {
      const cases = {
        whole: [{ src: 'x', a: 0, b: 10, s: 0, k: 1 }],
        cut: [{ src: 'x', a: 1.25, b: 4, s: 7.5, k: 1 }],
        two: [{ src: 'x', a: 0, b: 2, s: 0, k: 1 }, { src: 'x', a: 0, b: 2, s: 5, k: 1 }],
        stretch: [{ src: 'x', a: 0.5, b: 2.5, s: 3, k: 1.5 }],
      };
      const worst = {};
      for (const [k, clips] of Object.entries(cases)) {
        let w = 0;
        for (const c of clips) for (let i = 0; i <= 40; i++) {
          const t = c.a + (c.b - c.a) * i / 40 * 0.999, x = HT.toSong(c, t), back = HT.toSource(clips, x);
          if (!back) { w = Infinity; continue; }
          w = Math.max(w, Math.abs(Math.round(back.t * sr) - Math.round(t * sr)));
        }
        worst[k] = w;
      }
      const gap = HT.toSource(cases.two, 3.5), last = HT.toSource(cases.two, 5.5);
      return { worst, gap, lastSong: last && HT.toSong(last.clip, last.t), lastClipS: last && last.clip.s };
    }""", sr)
    assert all(v <= 1 for v in out["worst"].values()), out
    assert out["gap"] is None, "a point between two clips is in no clip"
    assert out["lastClipS"] == 5.0, "the later clip is the one that holds a point"


def test_two_sources_on_one_daw_track_are_one_row_with_both_clips_at_their_song_positions(host):
    a, b, c, d = NAMES
    assert host.evaluate("() => HT.on()") is False, "no rows yet: the page is as it was"
    assert take(host, rows_payload()) is True
    assert host.evaluate("() => HT.on()") is True
    assert take(host, rows_payload()) is False, "the same rows twice change nothing"
    host.evaluate("() => setView(0, 12)")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    shown = host.evaluate("() => [...document.querySelectorAll('.track')].filter(r => getComputedStyle(r).display !== 'none').map(r => [r.dataset.name, r.querySelector('.tnm').textContent])")
    assert shown == [[a, "Drums"], [c, "Toms"], [d, d]], "one row for the two sources of the DAW track; its header shows the name of the DAW track"
    assert host.evaluate("() => [...document.querySelectorAll('.track.hostMerged')].map(r => r.dataset.name)") == [b]
    assert host.evaluate("() => HT.merged.size + HT.dropped") == 1
    # the hits of both sources are drawn: the first source at its source time, the second (cut from 1.0 s) at 10 + (t - 1.0)
    host.evaluate("() => setView(0, 12)")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    for t in (0.5, 1.5, 2.5, 10.5, 11.5):
        assert line_at(host, a, t), f"a hit line at song {t} s in the row of the DAW track"
    for t in (10.0, 11.0, 7.0):
        assert not line_at(host, a, t), f"no hit line at song {t} s"
    # the hit at 0.5 s of the second source lies before the start of its clip: in no clip, so it is not drawn (and not in the project's song)
    assert host.evaluate("([a]) => HT.hitRefs(HT.clips(a), S.events, 0, 12).map(r => [r.src, r.i, Math.round(r.t * 1000) / 1000])", [a]) == [
        [a, 0, 0.5], [a, 1, 1.5], [a, 2, 2.5], [NAMES[1], 1, 10.5], [NAMES[1], 2, 11.5]]
    # the toms are 4 s later in the song: their hits are at 4.5, 5.5, 6.5 and not at their source time
    assert line_at(host, c, 4.5) and line_at(host, c, 6.5) and not line_at(host, c, 1.5)


def test_the_stored_hits_stay_in_source_time_whatever_the_page_shows(host):
    a, b = NAMES[0], NAMES[1]
    before = host.evaluate("() => JSON.stringify(S.events)")
    take(host, rows_payload())
    host.evaluate("() => setView(0, 12)")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    moved = rows_payload(2)
    moved["rows"][0]["clips"][1]["s"] = 3.0   # the clip moves in the DAW
    take(host, moved)
    assert host.evaluate("() => JSON.stringify(S.events)") == before, "a display change writes no hit time"
    assert host.evaluate("([b]) => HT.hitRefs(HT.clips(S.order[0]), S.events, 0, 12).filter(r => r.src === b).map(r => Math.round(r.t * 1000) / 1000)", [b]) == [3.5, 4.5]


def test_the_timeline_is_the_song_the_end_of_the_last_clip(host):
    take(host, rows_payload())
    assert host.evaluate("() => maxDur()") == 12.0
    assert host.evaluate("() => HT.songEnd()") == 12.0
    # the host position is the playhead as it is: no mapping through a track
    host.evaluate("() => { appOnPlayhead({ sid: S.sid, epoch: 1, t_ns: performance.now() * 1e6, pos: 11.0, playing: false }); }")
    assert host.evaluate("() => PH.v") == 11.0
    host.evaluate("() => { S.selTrack = S.order[2]; }")
    host.evaluate("() => { appOnPlayhead({ sid: S.sid, epoch: 2, t_ns: performance.now() * 1e6, pos: 11.0, playing: false }); }")
    assert host.evaluate("() => PH.v") == 11.0, "the selected track does not matter"


def test_the_standalone_app_shows_source_time_and_has_no_rows(app, make_project):
    open_project(app, make_project(tracks=TRACKS, seconds=6.0))
    assert app.evaluate("() => HT.on()") is False
    # even with rows given (a test sends them), the mode is off without the switch: nothing changes
    app.evaluate("p => HT.take(p)", rows_payload())
    app.wait_for_function("() => !FR.raf && FR.flags === 0")
    assert app.evaluate("() => HT.on()") is False
    assert app.evaluate("() => [HT.clips(S.order[0]), maxDur(), document.querySelectorAll('.track.hostMerged').length, trackLabel(S.order[0])]") == [None, 10, 0, NAMES[0]]
    assert app.evaluate("() => HT.hitCount(S.order[0], S.events)") == 0
    app.evaluate("() => { HT.rows = []; HT.gen = 0; HT.rebuild(); }")


# ---------------------------------------------------------------- input: a hit is edited in the source of the clip under the pointer

def spike_event(page, row, kind, song_t, dx=0):
    """A mouse event of the spike lane of the row at a song time (a pointer in the page; the drag listens on the window)."""
    page.evaluate("""([n, kind, t, dx]) => {
      const c = document.querySelector(`.track[data-name="${CSS.escape(n)}"] canvas.spike`), x = GEO.laneX + tToX(t, laneWidth()) + dx, r = c.getBoundingClientRect();
      const ev = new MouseEvent(kind, { bubbles: true, cancelable: true, button: 0, clientX: x, clientY: r.top + r.height / 2 });
      (kind === 'mouseup' ? window : c).dispatchEvent(ev); }   // the drag listens for the move on the lane and for the release on the window""", [row, kind, song_t, dx])


def wait_idle(page):
    page.wait_for_function("() => !FR.raf && FR.flags === 0")
    page.wait_for_function("() => typeof SH === 'undefined' || !SH.pending")


def test_a_double_click_in_the_second_clip_adds_a_hit_in_the_source_time_of_that_source(host):
    a, b = NAMES[0], NAMES[1]
    take(host, rows_payload())
    host.evaluate("() => setView(0, 12)")
    wait_idle(host)
    n_a, n_b = host.evaluate("([a, b]) => [S.events[a].length, S.events[b].length]", [a, b])
    spike_event(host, a, "dblclick", 11.2)   # song 11.2 s lies in the second clip (the audio of the second source from 1.0 s, at song 10 s)
    host.wait_for_function("([b, n]) => S.events[b].length === n", arg=[b, n_b + 1])
    got = host.evaluate("([a, b]) => ({ a: S.events[a].length, t: S.events[b].map(e => e.t) })", [a, b])
    assert got["a"] == n_a, "the first source is not touched"
    assert any(abs(t - 2.2) <= 0.02 for t in got["t"]), f"stored in source time 1.0 + (11.2 - 10.0), to the pixel: {got['t']}"
    assert not any(abs(t - 11.2) < 0.01 for t in got["t"]), "the song time was not stored"
    near = host.evaluate("([a, b]) => HT.hitRefs(HT.clips(a), S.events, 11.0, 11.4).map(r => [r.src === b, r.t])", [a, b])
    assert len(near) == 1 and near[0][0] is True and abs(near[0][1] - 11.2) <= 0.02, near
    # the same point in a gap (song 7 s: no clip of this DAW track holds it): no hit, and the message says why
    spike_event(host, a, "dblclick", 7.0)
    wait_idle(host)
    assert host.evaluate("([a, b]) => [S.events[a].length, S.events[b].length]", [a, b]) == [n_a, n_b + 1]
    assert "outside every region" in host.evaluate("() => document.getElementById('stMsg').textContent")


def test_a_hit_drag_moves_the_hit_in_its_source_and_stays_in_its_clip(host):
    a, b = NAMES[0], NAMES[1]
    take(host, rows_payload())
    host.evaluate("() => setView(0, 12)")
    wait_idle(host)
    # the second source has a hit at 2.5 s of its audio: song 11.5 s
    px_per_s = host.evaluate("() => laneWidth() / viewDur()")
    spike_event(host, a, "mousedown", 11.5)
    spike_event(host, a, "mousemove", 11.5, dx=-0.25 * px_per_s)
    spike_event(host, a, "mouseup", 11.5, dx=-0.25 * px_per_s)
    host.wait_for_function("([b]) => S.events[b].some(e => Math.abs(e.t - 2.25) < 0.02)", arg=[b])
    # a drag far to the left stops at the start of the clip: the source time never goes below 1.0 s
    spike_event(host, a, "mousedown", 10.5)   # the hit at 1.5 s of the second source
    spike_event(host, a, "mousemove", 10.5, dx=-3 * px_per_s)
    spike_event(host, a, "mouseup", 10.5, dx=-3 * px_per_s)
    wait_idle(host)
    assert host.evaluate("([b]) => Math.min(...S.events[b].filter(e => e.t > 0.9).map(e => e.t))", [b]) >= 1.0 - 1e-9, "the hit stops at the start of its clip"


def test_tab_and_the_track_keys_treat_a_merged_row_as_one_row_and_never_select_a_hidden_one(host):
    a, b, c, d = NAMES
    take(host, rows_payload())
    host.evaluate("([a]) => { setSelTrack(a); S.playhead = 0; }", [a])
    seen = []
    for _ in range(6):
        host.evaluate("() => gotoEvent(1)")
        seen.append(host.evaluate("() => [S.sel.track, S.sel.index, Math.round(S.playhead * 1000) / 1000]"))
    assert seen[:5] == [[a, 0, 0.5], [a, 1, 1.5], [a, 2, 2.5], [b, 1, 10.5], [b, 2, 11.5]], "Tab walks the hits of both sources of the DAW track in song order"
    assert host.evaluate("() => S.selTrack") == a, "the selection is the row, not the hidden source"
    host.evaluate("() => { S.playhead = 12; }")
    host.evaluate("() => gotoEvent(-1)")
    assert host.evaluate("() => [S.sel.track, S.sel.index]") == [b, 2]
    host.evaluate("([a]) => setSelTrack(a)", [a])
    order = []
    for _ in range(3):
        host.evaluate("() => selectTrackStep(1)")
        order.append(host.evaluate("() => S.selTrack"))
    assert order == [c, d, d], "; steps over the merged row (the second source of the first DAW track) and stops at the last row"
    order = []
    for _ in range(3):
        host.evaluate("() => selectTrackStep(-1)")
        order.append(host.evaluate("() => S.selTrack"))
    assert order == [c, a, a], "P steps back over it and stops at the first row"
    host.evaluate("([b]) => setSelTrack(b)", [b])
    assert host.evaluate("() => S.selTrack") == a, "a source that is merged into a row selects that row"
    # the Tab over all tracks walks one list too
    host.evaluate("() => { S.selTrack = null; S.playhead = 0; }")
    assert host.evaluate("() => allEventsSorted(null).filter(p => p.track === S.order[1]).length") == 2, "only the hits of the second source inside its clip"


def test_role_and_note_of_a_row_reach_every_source_and_the_inspector_says_so(host):
    a, b = NAMES[0], NAMES[1]
    take(host, rows_payload())
    host.evaluate("([a]) => { setSelTrack(a); syncInspector(true); }", [a])
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    text = host.evaluate("() => document.getElementById('insRowNote') && document.getElementById('insRowNote').textContent")
    assert text and "2 sources" in text and "role, note, Sensitivity, Loudness and Shift apply to all of them" in text and "one undo step" in text
    assert host.evaluate("([a]) => document.getElementById('insRowNote').textContent", [a])
    ok = host.evaluate("async ([a]) => await setTrackRole(a, 'tom')", [a])
    assert ok is True
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    roles = host.evaluate("([a, b]) => [S.tracks[a].role, S.tracks[b].role, S.tracks[a].note === S.tracks[b].note]", [a, b])
    assert roles == ["tom", "tom", True], "the second source of the DAW track got the role and the note of the row"
    assert host.evaluate("() => HT.on()") is True, "the rows survive the reload of the project"


def test_a_row_of_one_source_has_no_note_line(host):
    take(host, rows_payload())
    host.evaluate("([c]) => { setSelTrack(c); syncInspector(true); }", [NAMES[2]])
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    assert host.evaluate("() => !document.getElementById('insRowNote')")


def test_the_fault_flags_change_what_the_page_does(host):
    """The fault runs of the check (DS_ARA_SONG_FAULT): each flag breaks one rule of this file, so the tests above can fail."""
    a, b = NAMES[0], NAMES[1]
    take(host, rows_payload())
    host.evaluate("() => setView(0, 12)")
    wait_idle(host)
    host.evaluate("() => { HT.fault = 'source-x'; invalidateHits(); }")
    wait_idle(host)
    assert not line_at(host, a, 10.5) and line_at(host, a, 1.5), "source-x: the hit of the second source is drawn at its source time"
    host.evaluate("() => { HT.fault = 'no-cut-start'; invalidateHits(); }")
    wait_idle(host)
    assert not line_at(host, a, 10.5), "no-cut-start: the cut of the clip is ignored (the hit is at 11.5, not 10.5)"
    host.evaluate("() => { HT.fault = 'home-only'; invalidateHits(); }")
    wait_idle(host)
    assert not line_at(host, a, 10.5) and line_at(host, a, 1.5), "home-only: only the first source of the DAW track is drawn"


def test_the_store_song_fault_stores_the_song_time_of_a_hit(host):
    """The fault flag store-song: an added hit is stored with its song time. The clip lies at song 3.5 s, so the song time is inside the audio
    of the source and the service takes it (a song time past the end of the audio is refused, which the plugin stage shows)."""
    a, b = NAMES[0], NAMES[1]
    moved = rows_payload()
    moved["rows"][0]["clips"][1]["s"] = 3.5
    take(host, moved)
    host.evaluate("() => setView(0, 12)")
    wait_idle(host)
    n_b = host.evaluate("([b]) => S.events[b].length", [b])
    host.evaluate("() => { HT.fault = 'store-song'; }")
    spike_event(host, a, "dblclick", 4.0)   # inside the second clip: its audio from 1.0 s, at song 3.5 s: the source time is 1.5 s
    host.wait_for_function("([b, n]) => S.events[b].length === n", arg=[b, n_b + 1])
    host.evaluate("() => { HT.fault = ''; }")
    times = host.evaluate("([b]) => S.events[b].map(e => e.t)", [b])
    assert any(abs(t - 4.0) < 0.02 for t in times), f"store-song: the song time 4.0 was stored: {times}"


def test_the_leak_fault_turns_the_mode_on_in_the_standalone_page_and_the_standalone_checks_see_it(app, make_project):
    open_project(app, make_project(tracks=TRACKS, seconds=6.0))
    app.evaluate("p => HT.take(p)", rows_payload())
    app.evaluate("() => { HT.fault = 'leak'; HT.rebuild(); }")
    assert app.evaluate("() => [HT.on(), HT.clips(S.order[0]) !== null, maxDur()]") == [True, True, 12.0], "the values the standalone test refuses"
    app.evaluate("() => { HT.fault = ''; HT.rows = []; HT.gen = 0; HT.rebuild(); }")


# ---------------------------------------------------------------- the settings of a DAW track reach all its sources, as one undo step

A2, B2 = "Kick In", "Kick Sub"


def two_source_rows(gen=1):
    """One DAW track with the two sources of the histogram project: the first at song 0 s, the second at song 10 s (both 6 s of audio)."""
    return {"gen": gen, "song_end": 16.0, "gone": [], "rows": [
        {"id": "q1", "name": "Drums", "srcs": [A2, B2], "clips": [
            {"src": A2, "a": 0.0, "b": 6.0, "s": 0.0, "k": 1.0}, {"src": B2, "a": 0.0, "b": 6.0, "s": 10.0, "k": 1.0}]}]}


@pytest.fixture
def host2(host_app, make_project, tmp_path):
    host_open(host_app, hist_build(make_project, tmp_path, "Row"))
    analyse(host_app)
    host_app.evaluate("p => HT.take(p)", two_source_rows())
    host_app.wait_for_function("() => !FR.raf && FR.flags === 0")
    inspect_track(host_app, A2)
    yield host_app
    host_app.evaluate("() => { HT.rows = []; HT.gen = 0; HT.fault = ''; HT.rebuild(); HT.paint(); }")


def undo_top(page):
    return page.evaluate("() => { const o = S.undo[S.undo.length - 1]; return o ? [o.type, o.steps ? o.steps.map(x => x.track) : null, S.undo.length] : null; }")


def shift_of(page):
    return page.evaluate("([a, b]) => [S.tracks[a].params.shift_samples || 0, S.tracks[b].params.shift_samples || 0]", [A2, B2])


def set_shift(page, v):
    page.evaluate("v => { const f = document.querySelector('#insParams .pShift'); f.value = v; f.dispatchEvent(new Event('change')); }", v)


def test_shift_of_a_row_moves_all_its_sources_and_one_undo_restores_both(host2):
    n0 = host2.evaluate("() => S.undo.length")
    set_shift(host2, 12)
    host2.wait_for_function("([a, b]) => S.tracks[b].params && S.tracks[b].params.shift_samples === 12 && S.tracks[a].params.shift_samples === 12", arg=[A2, B2])
    assert undo_top(host2) == ["group", [A2, B2], n0 + 1], "one undo step that holds the step of each source"
    host2.evaluate("() => undo()")
    host2.wait_for_function("() => S.undo.length === %d" % n0)
    host2.wait_for_function("([a, b]) => !S.tracks[a].params.shift_samples && !S.tracks[b].params.shift_samples", arg=[A2, B2])
    host2.evaluate("() => redo()")
    host2.wait_for_function("([a, b]) => S.tracks[a].params.shift_samples === 12 && S.tracks[b].params.shift_samples === 12", arg=[A2, B2])


def test_loudness_of_a_row_reaches_all_its_sources_in_one_undo_step(host2):
    n0 = host2.evaluate("() => S.undo.length")
    host2.evaluate("async ([a]) => { await hgVelCommit(a, S.tracks[a].params, -50, -10, null); }", [A2])
    got = host2.evaluate("([a, b]) => [S.tracks[a].params.vel_lo_db, S.tracks[b].params.vel_lo_db, S.tracks[a].params.vel_hi_db, S.tracks[b].params.vel_hi_db]", [A2, B2])
    assert got == [-50, -50, -10, -10]
    assert undo_top(host2) == ["group", [A2, B2], n0 + 1]
    host2.evaluate("() => undo()")
    host2.wait_for_function("([a, b]) => S.tracks[a].params.vel_lo_db !== -50 && S.tracks[b].params.vel_lo_db !== -50", arg=[A2, B2])
    assert host2.evaluate("() => S.undo.length") == n0


def test_sensitivity_of_a_row_repicks_all_its_sources_in_one_undo_step(host2):
    host2.wait_for_function("() => DSHist.state().detection === 'ok'")
    n0 = host2.evaluate("() => S.undo.length")
    before = host2.evaluate("([a, b]) => [S.events[a].length, S.events[b].length]", [A2, B2])
    host2.evaluate("() => { const s = document.querySelector('#insParams .pSens'); s.value = 150; s.dispatchEvent(new Event('input')); }")
    host2.evaluate("() => document.querySelector('#insParams .pSens').dispatchEvent(new Event('change'))")
    host2.wait_for_function("() => !HG.sens.base && !DSHist.state().busy")
    host2.wait_for_function("(n) => S.undo.length === n + 1", arg=n0)
    top = undo_top(host2)
    assert top[0] == "group" and top[1] == [A2, B2], top
    sens = host2.evaluate("([a, b]) => [S.tracks[a].params.sensitivity, S.tracks[b].params.sensitivity]", [A2, B2])
    assert sens[0] == sens[1] == 150, "both sources have the sensitivity of the row"
    host2.evaluate("() => undo()")
    host2.wait_for_function("(n) => S.undo.length === n", arg=n0)
    assert host2.evaluate("([a, b]) => [S.events[a].length, S.events[b].length]", [A2, B2]) == before, "one undo restores the hits of both sources"
    assert host2.evaluate("([a, b]) => [S.tracks[a].params.sensitivity, S.tracks[b].params.sensitivity]", [A2, B2])[0] != 150


def test_the_fault_flags_of_the_settings_break_the_rule(host2):
    n0 = host2.evaluate("() => S.undo.length")
    host2.evaluate("() => { HT.fault = 'no-fan-out'; }")
    set_shift(host2, 7)
    host2.wait_for_function("([a]) => S.tracks[a].params.shift_samples === 7", arg=[A2])
    assert shift_of(host2) == [7, 0], "no-fan-out: the second source is not shifted"
    host2.evaluate("() => { undo(); }")
    host2.wait_for_function("() => S.undo.length === %d" % n0)
    host2.evaluate("() => { HT.fault = 'no-group'; }")
    set_shift(host2, 9)
    host2.wait_for_function("([a, b]) => S.tracks[a].params.shift_samples === 9 && S.tracks[b].params.shift_samples === 9", arg=[A2, B2])
    assert host2.evaluate("() => S.undo.length") == n0 + 2, "no-group: one undo entry for each source"


def test_the_standalone_twin_shifts_one_track_with_one_plain_undo_step(app, make_project, tmp_path):
    open_project(app, hist_build(make_project, tmp_path, "Twin"))
    analyse(app)
    inspect_track(app, A2)
    n0 = app.evaluate("() => S.undo.length")
    set_shift(app, 12)
    app.wait_for_function("([a]) => S.tracks[a].params.shift_samples === 12", arg=[A2])
    assert shift_of(app) == [12, 0], "only the selected track shifts"
    top = app.evaluate("() => { const o = S.undo[S.undo.length - 1]; return [o.type, S.undo.length]; }")
    assert top == ["shift", n0 + 1], "a plain step, no group"
    app.evaluate("async ([a]) => { await hgVelCommit(a, S.tracks[a].params, -50, -10, null); }", [A2])
    assert app.evaluate("() => S.undo[S.undo.length - 1].type") == "loud"
    assert app.evaluate("([b]) => S.tracks[b].params.vel_lo_db !== -50", [B2])


def test_hits_outside_every_clip_are_counted_in_the_status_line(host):
    take(host, rows_payload())
    note = lambda: host.evaluate("() => { const e = document.getElementById('hostOutsideNote'); return e ? e.textContent.trim() : null; }")
    # the second source has a clip from 1.0 s: its hit at 0.5 s is in no clip of the row
    assert note() == "· 1 hit outside every clip"
    full = rows_payload(2)
    full["rows"][0]["clips"][1]["a"] = 0.0
    take(host, full)
    host.evaluate("() => updateStatus()")
    assert note() is None, "zero hits outside: no text"


def test_the_standalone_status_line_has_no_outside_note(app, make_project):
    open_project(app, make_project(tracks=TRACKS, seconds=6.0))
    app.evaluate("() => updateStatus()")
    assert app.evaluate("() => document.getElementById('hostOutsideNote')") is None


# ---------------------------------------------------------------- review round: the playhead, Tab, role undo, clamps, readers of the playhead

def test_add_hit_at_the_playhead_goes_to_the_source_of_the_clip_under_it_in_source_time(host):
    a, b = NAMES[0], NAMES[1]
    p = rows_payload()
    p["rows"][0]["clips"][1].update({"a": 0.0, "b": 3.0, "s": 10.0})   # the second source at song 10 s, from the start of its audio
    take(host, p)
    n_a, n_b = host.evaluate("([a, b]) => [S.events[a].length, S.events[b].length]", [a, b])
    host.evaluate("([a]) => { setSelTrack(a); S.playhead = 11.2; addHitAtPlayhead(); }", [a])
    host.wait_for_function("([b, n]) => S.events[b].length === n", arg=[b, n_b + 1])
    got = host.evaluate("([a, b]) => ({ a: S.events[a].length, t: S.events[b].map(e => e.t) })", [a, b])
    assert got["a"] == n_a
    assert any(abs(t - 1.2) <= 1 / 48000 + 1e-9 for t in got["t"]), f"stored in source time 11.2 - 10.0: {got['t']}"
    assert not any(abs(t - 11.2) < 0.01 for t in got["t"])
    host.evaluate("() => { S.playhead = 7.0; addHitAtPlayhead(); }")   # no clip of the row holds song 7 s
    wait_idle(host)
    assert host.evaluate("([a, b]) => [S.events[a].length, S.events[b].length]", [a, b]) == [n_a, n_b + 1]
    assert "outside every region" in host.evaluate("() => document.getElementById('stMsg').textContent")


def test_the_store_song_fault_stores_the_song_time_of_the_playhead(host):
    a, b = NAMES[0], NAMES[1]
    moved = rows_payload()
    moved["rows"][0]["clips"][1].update({"a": 0.0, "b": 3.0, "s": 3.5})
    take(host, moved)
    n_b = host.evaluate("([b]) => S.events[b].length", [b])
    host.evaluate("([a]) => { HT.fault = 'store-song'; setSelTrack(a); S.playhead = 4.0; addHitAtPlayhead(); }", [a])
    host.wait_for_function("([b, n]) => S.events[b].length === n", arg=[b, n_b + 1])
    host.evaluate("() => { HT.fault = ''; }")
    assert host.evaluate("([b]) => S.events[b].some(e => Math.abs(e.t - 4.0) < 0.01)", [b]), "store-song: the song time was stored as source time"


def tab_walk(page, a, steps):
    out = []
    for d in steps:
        page.evaluate("d => gotoEvent(d)", d)
        out.append(page.evaluate("() => [S.sel.track, S.sel.index]"))
    return out


def tab_rows(s_a, s_b):
    a, b = NAMES[0], NAMES[1]
    return {"gen": 1, "song_end": 12.0, "gone": [], "rows": [{"id": "q1", "name": "Drums", "srcs": [a, b], "clips": [
        {"src": a, "a": 0.0, "b": 2.0, "s": s_a, "k": 1.0}, {"src": b, "a": 0.0, "b": 1.0, "s": s_b, "k": 1.0}]}]}


def test_tab_and_shift_tab_walk_the_hits_of_two_clips_at_an_offset_that_is_not_on_the_sample_grid(host):
    a, b = NAMES[0], NAMES[1]
    take(host, tab_rows(0.1234567, 5.1234))   # the song times 0.6234567, 1.6234567 and 5.6234 are not on the grid of 48 kHz
    host.evaluate("([a]) => { setSelTrack(a); S.playhead = 0; }", [a])
    fwd = tab_walk(host, a, [1, 1, 1])
    assert fwd == [[a, 0], [a, 1], [b, 0]], "Tab visits each hit once, in song order"
    back = tab_walk(host, a, [-1, -1])
    assert back == [[a, 1], [a, 0]], "Shift-Tab walks back over the same hits"
    assert host.evaluate("() => S.playhead") == pytest.approx(0.6234567, abs=1e-12), "the playhead is the song time, not snapped"


def test_the_snap_song_fault_makes_tab_find_the_same_hit_again(host):
    a, b = NAMES[0], NAMES[1]
    take(host, tab_rows(0.1234567, 5.1234))
    host.evaluate("([a]) => { HT.fault = 'snap-song'; setSelTrack(a); S.playhead = 0; }", [a])
    fwd = tab_walk(host, a, [1, 1, 1])
    back = tab_walk(host, a, [-1, -1])
    host.evaluate("() => { HT.fault = ''; }")
    assert fwd != [[a, 0], [a, 1], [b, 0]] or back != [[a, 1], [a, 0]], f"the old snap makes the walk stop or repeat: {fwd} {back}"


def test_a_role_change_of_a_row_is_one_undo_step_that_brings_back_the_hits_of_every_source(host2):
    a, b = A2, B2
    host2.evaluate("([b]) => { addHitAt(b, 5.55, false); }", [b])
    host2.wait_for_function("([b]) => S.events[b].some(e => Math.abs(e.t - 5.55) < 0.002)", arg=[b])
    wait_idle(host2)
    before = host2.evaluate("([a, b]) => [S.events[a].map(e => e.t.toFixed(6)).join(), S.events[b].map(e => e.t.toFixed(6)).join(), S.tracks[a].role, S.tracks[b].role]", [a, b])
    assert host2.evaluate("async ([a]) => await setTrackRole(a, 'tom')", [a]) is True
    # the role change loads the project again (the stacks start empty): the group is the one entry on them
    assert host2.evaluate("() => { const o = S.undo[S.undo.length - 1]; return [o.type, o.steps.map(x => x.track), S.undo.length]; }") == ["group", [a, b], 1]
    assert host2.evaluate("([a, b]) => [S.tracks[a].role, S.tracks[b].role]", [a, b]) == ["tom", "tom"]
    assert host2.evaluate("async () => await undoRedo(true)") is True
    wait_idle(host2)
    after = host2.evaluate("([a, b]) => [S.events[a].map(e => e.t.toFixed(6)).join(), S.events[b].map(e => e.t.toFixed(6)).join(), S.tracks[a].role, S.tracks[b].role]", [a, b])
    assert after == before, "one undo: the roles and the hits of both sources, the hand edit of the second included"
    assert host2.evaluate("() => S.undo.length") == 0


def test_a_group_step_that_is_refused_takes_the_applied_steps_back(host):
    out = host.evaluate("""async () => {
      const calls = [], apply = (s, rev) => { calls.push([s.id, rev]); return s.id === 'first' ? false : true; };
      const r = await groupApplyOp({ type: 'group', steps: [{ id: 'first' }, { id: 'second' }] }, true, apply);   # an undo takes the last step first
      return { r, calls }; }""".replace("#", "//"))
    assert out["r"] is False
    assert out["calls"] == [["second", True], ["first", True], ["second", False]], "the refused step takes the one before it back"


def test_the_nudge_and_the_add_stay_inside_the_clip(host):
    a, b = NAMES[0], NAMES[1]
    p = rows_payload()
    p["rows"][0]["clips"][1].update({"a": 1.0, "b": 2.6, "s": 10.0})
    take(host, p)
    # the hit at 2.5 s of the second source: a nudge of +10000 samples stops at the last sample of the clip (2.6 s - 1 sample)
    idx = host.evaluate("([b]) => S.events[b].findIndex(e => Math.abs(e.t - 2.5) < 0.01)", [b])
    host.evaluate("([b, i]) => { setSelTrack(S.order[0]); S.sel = { track: b, index: i }; nudgeSelected(10000); }", [b, idx])
    host.wait_for_function("([b]) => S.events[b].some(e => Math.abs(e.t - (2.6 - 1 / 48000)) < 1e-6)", arg=[b])
    n_b = host.evaluate("([b]) => S.events[b].length", [b])
    host.evaluate("([b]) => { addHitAt(b, 99, false, HT.clips(S.order[0]).find(c => c.src === b)); }", [b])
    host.wait_for_function("([b, n]) => S.events[b].length === n", arg=[b, n_b + 1])
    assert host.evaluate("([b]) => Math.max(...S.events[b].map(e => e.t)) < 2.6", [b]), "an add past the end of the clip lands at its last sample"


def test_the_readers_of_the_playhead_in_a_daw_use_song_time_or_are_inert(host):
    a, b = NAMES[0], NAMES[1]
    take(host, rows_payload())
    assert host.evaluate("() => projectEnd()") == 12.0, "Go to End is the end of the song"
    host.evaluate("() => goTo(projectEnd())")
    assert host.evaluate("() => S.playhead") == 12.0
    # the zoom toggle on a selected hit of the second source centres on its song time (10.5 s), not on its source time (1.5 s)
    idx = host.evaluate("([b]) => S.events[b].findIndex(e => Math.abs(e.t - 1.5) < 0.01)", [b])
    host.evaluate("([a, b, i]) => { setSelTrack(a); S.sel = { track: b, index: i }; S.zoomBack = null; zoomToggle(); }", [a, b, idx])
    mid = host.evaluate("() => (S.view.t0 + S.view.t1) / 2")
    assert abs(mid - 10.5) < 0.01, mid
    # the tempo map and the markers are in the time of each source: inert in a DAW, with one text
    assert host.evaluate("() => setBar1AtPlayhead()") is None
    assert "the DAW owns" in host.evaluate("() => document.getElementById('stMsg').textContent")
    assert host.evaluate("async () => await markerAtSelection()") is False
    host.evaluate("() => { MK.list = [{ t: 1.0 }]; markerJump(1); }")
    assert "the DAW owns the markers" in host.evaluate("() => document.getElementById('stMsg').textContent")
    host.evaluate("() => { MK.list = []; }")


def test_a_new_document_of_the_plugin_is_accepted_and_an_older_message_of_the_same_document_is_not(host):
    p = rows_payload(5)
    p["doc"] = "d1"
    assert take(host, p) is True
    older = rows_payload(3)
    older["doc"] = "d1"
    older["rows"][0]["name"] = "Old"
    assert take(host, older) is False, "an older counter of the same document is dropped"
    fresh = rows_payload(1)
    fresh["doc"] = "d2"
    fresh["rows"][0]["name"] = "New document"
    assert take(host, fresh) is True, "a new document counts from 0 again"
    assert host.evaluate("() => HT.rows[0].name") == "New document"


def test_the_range_of_the_daw_stands_at_the_song_position_of_an_offset_clip(host):
    a = NAMES[0]
    p = rows_payload()
    p["rows"][0]["clips"][0].update({"a": 0.0, "b": 3.0, "s": 4.0})   # the first source lies at song 4 s
    take(host, p)
    host.evaluate("() => { setView(0, 12); DSHostSel.setFollow(true); }")
    sel = {"n": 50, "seen": True, "tracks": [], "none": False, "ranges": [{"t0": 5.0, "t1": 6.0}], "range_why": "ok", "hidden": []}
    host.evaluate("p => DSHostSel.event(p)", sel)
    wait_idle(host)
    got = host.evaluate("""() => { const b = document.querySelector('#hostRange .hostBand'), r = b.getBoundingClientRect(), e = document.getElementById('editor').getBoundingClientRect();
      return { t0: DSHostSel.state().range.t0, left: r.left - e.left, want: S.theadW + tToX(5.0, laneWidth()) }; }""")
    assert got["t0"] == 5.0 and abs(got["left"] - got["want"]) <= 1.5, got
    host.evaluate("() => { HT.fault = 'range-source'; }")
    host.evaluate("p => DSHostSel.event({ ...p, n: 51, ranges: [{ t0: 5.1, t1: 6.0 }] })", sel)
    wait_idle(host)
    fault = host.evaluate("() => DSHostSel.state().range.t0")
    host.evaluate("() => { HT.fault = ''; }")
    assert abs(fault - 5.1) > 1.0, f"range-source: the range was shown in the source time of the clip: {fault}"


def test_a_track_that_has_no_row_of_its_own_is_counted_in_the_status_line(host):
    a, b = NAMES[0], NAMES[1]
    p = rows_payload()
    p["rows"].insert(1, {"id": "q9", "name": "Second", "srcs": [a], "clips": [{"src": a, "a": 0.0, "b": 3.0, "s": 20.0, "k": 1.0}]})   # the first source sits on a second DAW track as well
    take(host, p)
    assert host.evaluate("() => HT.dropped") == 1
    host.evaluate("() => updateStatus()")
    assert "1 DAW track not shown" in host.evaluate("() => document.getElementById('hostOutsideNote').textContent")


def test_the_standalone_page_has_no_row_attribute(app, make_project):
    open_project(app, make_project(tracks=TRACKS, seconds=6.0))
    app.evaluate("p => HT.take(p)", rows_payload())
    app.wait_for_function("() => !FR.raf && FR.flags === 0")
    assert app.evaluate("() => document.querySelectorAll('.track[data-host-row]').length") == 0
    app.evaluate("() => { HT.rows = []; HT.gen = 0; HT.rebuild(); }")


def test_an_add_whose_snap_lands_past_the_clip_is_one_undo_entry(host):
    a, b = NAMES[0], NAMES[1]
    p = rows_payload()
    p["rows"][0]["clips"][1].update({"a": 1.0, "b": 2.495, "s": 10.0})   # the hit at 2.5 s of the second source lies 5 ms past the end of the clip
    take(host, p)
    wait_idle(host)
    n_b = host.evaluate("([b]) => S.events[b].length", [b])
    host.evaluate("([a, b]) => { addHitAt(b, 2.49, true, HT.clips(a).find(c => c.src === b)); }", [a, b])
    host.wait_for_function("([b, n]) => S.events[b].length === n", arg=[b, n_b + 1])
    host.wait_for_function("() => S.undo.length > 0 && S.undo[S.undo.length - 1].type === 'group'")   # the add and the move into the clip are joined
    wait_idle(host)
    top = host.evaluate("() => { const o = S.undo[S.undo.length - 1]; return [o.type, o.steps.map(s => s.type), S.undo.length]; }")
    assert top == ["group", ["add", "move"], 1], top
    assert host.evaluate("([b]) => S.events[b].filter(e => e.added || e.edited).every(e => e.t < 2.495)", [b]), "the new hit is inside its clip"
    host.evaluate("() => undo()")
    host.wait_for_function("([b, n]) => S.events[b].length === n", arg=[b, n_b])
    assert host.evaluate("() => S.undo.length") == 0, "one undo took the add and the move back: no hit is left outside the clip"
