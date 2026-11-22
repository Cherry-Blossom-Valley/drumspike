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
    host_app.evaluate("() => { HT.rows = []; HT.rebuild(); HT.paint(); }")


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
    app.evaluate("() => { HT.rows = []; HT.rebuild(); }")
