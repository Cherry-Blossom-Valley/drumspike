"""#444: a 10 fps capture of the lane canvas under a background cargo build.

The owner's report (0.9.3): waveforms flicker and jump occasionally during playback.
#435 fixed the tile seam of the follow strip; this is the loaded capture that hunts what
is left of it. The demo session (model-free, tools/media/demo_session.py, 13 tracks)
plays for 10 s with the smooth follow on while a fresh `cargo build` of the workspace
compiles in a scratch target dir (every core busy: the owner's background build). The
lane canvas is captured at 10 fps wall clock; a frame is a finding when

- a visible lane's wave centre row goes mostly background (the content went empty: a
  cleared canvas, a released placeholder, a tile gap; every tile draws the centre line
  over its whole width, lanes-gl.js). A lane the source culls - its wave part fully
  outside the canvas (glDraw's position check, spLanes' y check) - has no content on
  screen to empty and is not counted, or
- the shown content sits more than one tile (one view, GEO.laneW px) away from where
  the playhead puts it: the strip's animated transform against cpPos - FOLLOW_FRAC * d,
  else the newest GL draw the worker has answered (the same measure as JUMP_LOOP in
  test_follow_strip.py, against the playhead instead of the stale S.view).

A frame with no source to read at all (the GL canvas hidden and no strip box) is itself
a finding: nothing is on screen. A GL frame whose worker answer is merely late is not:
the canvas still shows, the rows are read from it, and the app glides until the answer
lands (P4-42) - only the shift stays unattributed for that frame. The summary records
the late answers, so a lag shows in the log without failing the capture.

On main the capture is expected to pass; the gate ran it five times under the load and
the test stays as the regression guard. A red names the frame: index, wall time,
playhead, source, the lane's lit fraction or the shift in px.
"""
import importlib.util
import json
import os
import shutil
import subprocess
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path

import pytest

from app_support import open_project
from helpers import ROOT

VIEW = 8.0            # seconds, the follow view of the strip tests
CAPTURE_S = 10.0      # the owner's capture: 10 s of play
FPS = 10              # the capture rate
SHIFT_TILES = 1.0     # a frame fails when the content sits more than one tile off the playhead
LIT_MIN = 0.9         # a drawn lane's centre row is lit over the lane's width (the FRAME_LOOP bar)
IDLE = "() => !FR.raf && FR.flags === 0 && (!GLQ.eng || GLQ.eng.stats.drawn >= GLQ.eng.stats.posted)"

ROLE_NOTE = {"kick": 36, "snare": 38, "tom": 43, "hat": 42, "cymbal": 49, "other": 40}

# One record per captured frame: the playhead, the follow source that shows (the strip's
# box, else #lanesGL), the lit fraction of every visible lane's wave centre row, and the
# content's offset against the playhead in px of the lane (positive: the content ahead).
PROBE = """() => {
  const now = performance.now(), W = Math.round(GEO.laneW * GEO.dpr), x0 = Math.round(GEO.waveX * GEO.dpr);
  const ph = cpPos(now), gl = GLQ.eng && GLQ.eng.canvas.style.visibility !== 'hidden' && GLQ.eng.canvas.style.display !== 'none';
  const box = !!SP.box && SP.box.style.display === 'block';
  const read = c => { const t = document.createElement('canvas');
    t.width = Math.max(Math.round(GEO.scW * GEO.dpr), 1); t.height = Math.max(Math.round(GEO.scH * GEO.dpr), 1);
    const x = t.getContext('2d', { willReadFrequently: true }); x.drawImage(c, 0, 0); return x; };
  const srcs = gl ? [[GLQ.eng.canvas.width === W ? 0 : x0, read(GLQ.eng.canvas)]]
            : box ? [...SP.box.firstChild.children].map(c => [0, read(c)]) : [];
  const rows = {};
  for (const { name } of spRows()) {
    const g = GEO.rows[name]; if (!g) { continue; }
    /* the lane the source paints: glDraw culls a wave lane that lies fully outside the
       scroll box (!GLQ.all: the draw-everything hook paints it anyway), spLanes culls by
       y only. A culled lane has no content on the canvas, so the floor does not count it. */
    const yy = g.wave - GEO.top, H = g.waveH || GEO.waveH;
    if (gl ? (!GLQ.all && (yy + H < 0 || yy > GEO.scH || GEO.waveX + GEO.laneW < 0 || GEO.waveX > GEO.scW))
           : (yy + H < 0 || yy > GEO.scH)) { continue; }
    const y = Math.round((g.wave - GEO.top + GEO.waveH / 2) * GEO.dpr);
    let best = 0;
    for (const [sx, x] of srcs) { for (const dy of [-1, 0, 1]) {
      if (y + dy < 0 || y + dy >= x.canvas.height) { continue; }
      const w = Math.min(W, x.canvas.width - sx); if (w <= 0) { continue; }
      const px = x.getImageData(sx, y + dy, w, 1).data;
      let lit = 0; for (let i = 3; i < px.length; i += 4) { if (px[i]) { lit++; } }
      best = Math.max(best, lit / w); } }
    rows[name] = Math.round(best * 1000) / 1000;
  }
  let shift = null, how = null;
  if (box && SP.on) { const s = SP.s, d = SP.d;
    shift = new DOMMatrix(getComputedStyle(SP.box.firstChild).transform).m41 + (ph - FOLLOW_FRAC * d - s) / d * GEO.laneW;
    how = 'strip';
  } else if (gl) { const p = window.__posts.filter(q => q[0] <= GLQ.eng.stats.drawn).pop();
    if (p) { shift = (ph - FOLLOW_FRAC * viewDur() - p[1]) / viewDur() * GEO.laneW; how = 'gl'; } }
  return { t: Math.round(now), ph: Math.round(ph * 1000) / 1000, playing: S.playing, sp: SP.on,
           gl: !!gl, box, how, W: GEO.laneW, rows,
           shift: shift === null ? null : Math.round(shift * 100) / 100 };
}"""

# The frame-path draws, with the view each was made with (as JUMP_LOOP's wrapper).
DRAW_WATCH = """() => { window.__posts = [];
  if (GLQ.eng) { const f = GLQ.eng.draw;
    GLQ.eng.draw = lanes => { const r = f(lanes);
      window.__posts.push([GLQ.eng.stats.posted, S.view.t0, performance.now()]); return r; }; } }"""


def _module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture(scope="module")
def demo_project(tmp_path_factory):
    """The demo session (#242) as a project: the generator's 13 WAVs with their roles and
    the truth hits as events. Model-free: nothing is analysed, the peaks come from the WAVs."""
    ds = _module(ROOT / "tools" / "media" / "demo_session.py", "ds_demo_session_444")
    out = tmp_path_factory.mktemp("demo444")
    ds.generate(out)
    truth = json.loads((out / "truth.json").read_text())
    own_mic = dict(ds.OWN_MIC)
    own_mic["crash"] = "oh l"   # the crash's bleed reference is its overhead (demo_session.py)
    specs = []
    for name in truth["tracks"]:
        role = ds.ROLE[name]
        events = [{"t": h["t"] / truth["sr"], "v": max(1, round(h["v"] * 127))}
                  for h in truth["hits"] if own_mic.get(h["drum"]) == name]
        specs.append((name, out / f"{name}.wav", role, ROLE_NOTE[role], events))
    mk = _module(ROOT / "apps" / "desktop" / "selftest" / "make_project.py", "ds_make_project_444")
    return Path(mk.build_project(tmp_path_factory.mktemp("proj444"), "Demo 444", specs, "copied"))


@contextmanager
def loaded_build():
    """The owner's background cargo build: the workspace compiles into a fresh scratch
    target dir (so it really compiles) with every core busy while the capture runs.
    Killed and deleted when the capture ends; the test asserts it never exited early."""
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo" / "bin" / "cargo")
    target = tempfile.mkdtemp(prefix="ds444-load-")
    env = dict(os.environ, CARGO_TARGET_DIR=target)
    proc = subprocess.Popen([cargo, "build", "--offline"], cwd=str(ROOT), env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        yield proc
    finally:
        if proc.poll() is None:
            proc.kill()
        proc.wait(60)
        shutil.rmtree(target, ignore_errors=True)


def test_the_loaded_play_shows_no_empty_or_shifted_lane_frame(app, demo_project):
    open_project(app, demo_project)
    app.evaluate(f"() => {{ setFollow('smooth'); S.followHold = false; S.playhead = 0; setView(0, {VIEW}); }}")
    app.wait_for_function(IDLE, timeout=60000)
    assert app.evaluate("() => GEO.dpr") == 1, "the pixel checks assume a device scale of 1 (the test browser)"
    app.evaluate(DRAW_WATCH)
    with loaded_build() as storm:
        app.click("#playBtn")
        app.wait_for_function("() => S.playing && AC.running", timeout=30000)
        frames, t0, tick = [], time.monotonic(), 0.0
        while time.monotonic() - t0 < CAPTURE_S:
            frames.append(app.evaluate(PROBE))
            tick += 1.0 / FPS
            delay = t0 + tick - time.monotonic()
            if delay > 0:
                app.wait_for_timeout(int(delay * 1000))
        assert storm.poll() is None, (f"the background cargo build exited early (rc={storm.returncode}): "
                                      f"the capture ran on a quiet host, which the gate does not count")
        app.evaluate("() => { if (S.playing) pause(); }")

    quiet = [f for f in frames if not f["playing"]]
    assert not quiet, f"the transport did not hold the loaded play: {len(quiet)} frames without S.playing, first {quiet[0]}"
    assert len(frames) >= 20, (f"only {len(frames)} frames captured in {CAPTURE_S} s at {FPS} fps "
                               f"(the capture itself starved): {frames[:2]}")
    span = frames[-1]["t"] - frames[0]["t"]
    assert span >= 6000, f"the capture spanned only {span} ms of wall time"
    hows = {f["how"] for f in frames}
    blind = [(i, f) for i, f in enumerate(frames) if not f["gl"] and not f["box"]]
    assert not blind, (f"frames with no lane source visible (nothing to read: the GL canvas "
                       f"hidden and no strip box): "
                       f"{[(i, f['t'], f['sp'], f['playing']) for i, f in blind[:5]]} "
                       f"of {len(frames)} (span {span} ms)")
    for i, f in enumerate(frames):
        empty = sorted(n for n, lit in f["rows"].items() if lit < LIT_MIN)
        assert not empty, (f"frame {i} at {f['t']} ms (playhead {f['ph']} s, source {f['how'] or 'none'}): "
                           f"the wave lane's centre row went empty on {empty}: "
                           f"{ {n: f['rows'][n] for n in empty} }")
        if f["shift"] is not None:
            assert abs(f["shift"]) <= SHIFT_TILES * f["W"], (
                f"frame {i} at {f['t']} ms (playhead {f['ph']} s, source {f['how']}): the lane content sits "
                f"{f['shift']} px from where the playhead puts it (more than one tile = {SHIFT_TILES * f['W']} px)")
    worst = max((abs(f["shift"]) for f in frames if f["shift"] is not None), default=0.0)
    floor = min((min(f["rows"].values()) for f in frames if f["rows"]), default=1.0)
    sp_stats = app.evaluate("() => ({ n: SP.n, why: SP.why })")
    late = [i for i, f in enumerate(frames) if f["gl"] and not f["how"]]
    n_gl = sum(1 for f in frames if f["gl"])
    srcs = sorted(h or "none" for h in hows)  # "none": a GL frame whose answer is late
    print(f"loaded capture: {len(frames)} frames in {span} ms ({len(frames) * 1000.0 / max(span, 1):.1f} fps), "
          f"sources {srcs}, worst |shift| {worst:.1f} px of {SHIFT_TILES * frames[0]['W']:.0f}, "
          f"lowest centre-row lit {floor}, strip {json.dumps(sp_stats['n'])} why {sp_stats['why']!r}, "
          f"GL late answers {len(late)} of {n_gl}: {late[:5]}")
