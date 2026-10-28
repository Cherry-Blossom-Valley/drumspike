"""P4-39b: smooth follow in a view where no glide strip can show (under 1 s, or at the sample level).

Before, every frame there was a full VIEW redraw (a 2 s view: 721 VIEW frames in 6 s on a 120 Hz display), the owner's
stutter and flicker. Now `followKind()` gives "page" there: the view stands still, the compositor moves the playhead
(CP.hist with cp true), and the view flips once when the playhead reaches 90 % of it, to put the playhead at
FOLLOW_FRAC (25 %) of the new view."""
from app_support import open_project
from test_follow_strip import IDLE, play_until_strip, project, stop

ZOOM = 2.0   # seconds: 2 s x 48 kHz = 96,000 samples, under the sample level (262,144) and above SP_MIN_S


def play_zoomed(app, mode, view, start=0.0):
    app.evaluate(f"() => {{ setFollow({mode!r}); S.followHold = false; S.playhead = {start}; setView({start}, {start + view}); }}")
    app.wait_for_function(IDLE)
    app.click("#playBtn")
    app.wait_for_function("() => S.playing && AC.running", timeout=20000)


def test_zoomed_smooth_follow_flips_pages_instead_of_redrawing_every_frame(app, make_project, tmp_path):
    open_project(app, project(make_project, tmp_path))
    play_zoomed(app, "smooth", ZOOM)
    r = app.evaluate("""() => new Promise(done => {
      const v0 = FR.n.view, h0 = CP.hist.length, s0 = SP.n.enter, t0 = performance.now();
      const wait = () => performance.now() - t0 < 3000 ? setTimeout(wait, 100)
        : done({ views: FR.n.view - v0, hist: CP.hist.slice(h0).map(h => !!h[3]), strip: SP.n.enter - s0, kind: followKind() });
      wait(); })""")
    stop(app)
    assert r["kind"] == "page", r
    assert r["views"] <= 4, r   # the flips only (before: one per frame)
    assert r["hist"] and sum(r["hist"]) >= 0.5 * len(r["hist"]), r   # the compositor moves the playhead
    assert r["strip"] == 0, r


def test_zoomed_smooth_follow_flips_at_90_percent_and_the_playhead_sits_at_25_percent(app, make_project, tmp_path):
    open_project(app, project(make_project, tmp_path))
    play_zoomed(app, "smooth", ZOOM)
    r = app.evaluate("""() => new Promise(done => {
      const t00 = performance.now(), seen = []; let last = S.view.t0, before = S.playhead, prevT = t00;
      const loop = () => {
        const nowT = performance.now();
        if (S.view.t0 !== last) { seen.push({ from: last, to: S.view.t0, at: before, now: S.playhead, gap: (nowT - prevT) / 1000 }); last = S.view.t0; }
        before = S.playhead; prevT = nowT;
        if (nowT - t00 < 3500 && seen.length < 1) requestAnimationFrame(loop); else done(seen); };
      requestAnimationFrame(loop); })""")
    stop(app)
    assert r, "the view did not flip in 3.5 s"
    f = r[0]
    # #280: the flip runs in the page's tick and the probe samples it in rAF frames. A loaded host
    # stretches the frame that spans the flip (measured: the flip landed 0.37 s off the 0.15 bound
    # on a quiet host, 1/5 of runs on PR #275), and `at` is the playhead of the frame before that.
    # The allowance widens by the gap the probe itself measured across the flip; a flip at the
    # wrong view fraction (planted: 75 %, 0.2 s off) still fails on the quiet host.
    assert abs((f["at"] - f["from"]) - 0.9 * ZOOM) < 0.15 + f["gap"], f   # flipped when the playhead reached 90 % of the view
    assert abs((f["now"] - f["to"]) - 0.25 * ZOOM) < 0.15 + f["gap"], f   # and it sits at 25 % of the new view


def test_a_view_over_the_sample_level_still_enters_the_strip(app, make_project, tmp_path):
    open_project(app, project(make_project, tmp_path))
    n0 = app.evaluate("() => SP.n.enter")
    play_until_strip(app)
    n1 = app.evaluate("() => [SP.n.enter, followKind()]")
    stop(app)
    assert n1[0] > n0 and n1[1] == "smooth", (n0, n1)


def test_page_mode_centres_and_off_mode_keeps_the_view(app, make_project, tmp_path):
    open_project(app, project(make_project, tmp_path))
    play_zoomed(app, "page", ZOOM)
    # #96: t0 > 0 only proves centring started; the single read caught it mid-lag
    # (off 0.288 vs the 0.2 bound under load). Wait for the centring to converge.
    app.wait_for_function(f"() => Math.abs(S.playhead - S.view.t0 - {0.5 * ZOOM}) < 0.2", timeout=8000)
    c = app.evaluate("() => [S.playhead - S.view.t0, followKind()]")
    stop(app)
    assert abs(c[0] - 0.5 * ZOOM) < 0.2 and c[1] == "page", c   # the page mode centres, as before
    play_zoomed(app, "off", ZOOM)
    app.wait_for_timeout(1500)
    o = app.evaluate("() => [S.view.t0, followKind()]")
    stop(app)
    assert o == [0, "off"], o
