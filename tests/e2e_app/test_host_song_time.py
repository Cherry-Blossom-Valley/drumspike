"""#555: in host mode the page shows song time (DAW plugin M1b8).

One row of the page is one DAW track; the plugin gives its clips as {src, a, b, s, k} (plugins/ara/src/rows.rs, the shell command
plugin_host_regions and the event plugin-regions). static/hosttime.js holds the rows; the lanes, the hits, the velocity lane, the overview and
the ruler draw song time, and the keys and the commands treat a DAW track as ONE row. These tests start the page the way the plugin shell
does (GET /?host_mode=1) and send the payload the plugin sends. The plugin side has its own stages (plugins/ara/check.py, editor-song-*).
The standalone app has none of it: the last tests.

Each test track has hits at 0.5 s, 1.5 s and 2.5 s of its own audio (6 s long)."""
import pytest

from app_support import open_project
from test_host_page import FOUR, host_open

NAMES = [n for n, *_ in FOUR]


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
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
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
    open_project(app, make_project(tracks=FOUR, seconds=6.0))
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
      (kind === 'mousemove' || kind === 'mouseup' ? window : c).dispatchEvent(ev); }""", [row, kind, song_t, dx])


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
    host.wait_for_function("([b, n]) => S.events[b].length === n", [b, n_b + 1])
    got = host.evaluate("([a, b]) => ({ a: S.events[a].length, t: S.events[b].map(e => e.t) })", [a, b])
    assert got["a"] == n_a, "the first source is not touched"
    assert any(abs(t - 2.2) <= 1 / 48000 for t in got["t"]), f"stored in source time 1.0 + (11.2 - 10.0): {got['t']}"
    assert not any(abs(t - 11.2) < 0.01 for t in got["t"]), "the song time was not stored"
    assert host.evaluate("([a, b]) => HT.hitRefs(HT.clips(a), S.events, 11.0, 11.4).map(r => [r.src === b, Math.round(r.t * 1000) / 1000])", [a, b]) == [[True, 11.2]]
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
    host.wait_for_function("([b]) => S.events[b].some(e => Math.abs(e.t - 2.25) < 0.002)", [b])
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
    assert text and "2 sources" in text and "role and note apply to all of them" in text
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
    host.evaluate("() => { HT.fault = 'store-song'; }")
    n_b = host.evaluate("([b]) => S.events[b].length", [b])
    host.evaluate("() => { HT.fault = ''; }")
    n_a = host.evaluate("([a]) => S.events[a].length", [a])
    host.evaluate("() => { HT.fault = 'store-song'; }")
    spike_event(host, a, "dblclick", 11.2)
    host.wait_for_function("([b, n]) => S.events[b].length === n", [b, n_b + 1])
    assert host.evaluate("([b]) => S.events[b].some(e => Math.abs(e.t - 11.2) < 0.01)", [b]), "store-song: the song time was stored"
    host.evaluate("() => { HT.fault = ''; }")


def test_the_leak_fault_turns_the_mode_on_in_the_standalone_page_and_the_standalone_checks_see_it(app, make_project):
    open_project(app, make_project(tracks=FOUR, seconds=6.0))
    app.evaluate("p => HT.take(p)", rows_payload())
    app.evaluate("() => { HT.fault = 'leak'; HT.rebuild(); }")
    assert app.evaluate("() => [HT.on(), HT.clips(S.order[0]) !== null, maxDur()]") == [True, True, 12.0], "the values the standalone test refuses"
    app.evaluate("() => { HT.fault = ''; HT.rows = []; HT.gen = 0; HT.rebuild(); }")
