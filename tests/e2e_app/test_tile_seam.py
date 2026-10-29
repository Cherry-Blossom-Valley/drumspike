"""#435: the tile seam of a follow strip while the worker's tile answers are late.

A strip of the smooth follow is two tiles of one view each. The page sized the tiles to the
scroll's clientWidth but placed them a lane width apart: whatever the clientWidth lacks against
the lane width (a classic scrollbar, the 2 px inset) opened a dark gap column between the tiles
that slides through every lane while the strip moves. The velocity lane kept its one-view draw
under the strip: its stalks and its right-edge scale ended at the seam and nothing showed right
of it, so the scale text floated at the seam instead of at the lane's left edge.

The test plays a project with the velocity lane on, delays the worker's tile answers
(GLQ.eng.stall, the test hook) once per strip, and reads the lane box while the strip's new tile
column is in view and the next strip is still pending. The lanes scroll is narrowed by 15 px so
the geometry is the owner's (the clientWidth smaller than the lane width) whatever scrollbars
the test browser draws. Asserts, per track, at the delayed frames: no column of the wave lane is
empty background, the velocity stalks are present in the new column, and no velocity label text
floats in the middle of the lane; at the strip's entry the velocity label sits at the lane's
left edge. On main the stalk and the label checks fail (the output names the frame).

A track whose row element is not fully inside the strip's painted area is skipped with a note:
the strip draws only the lanes that intersect the lanes viewport, and an element screenshot of a
row that pokes out of it (a short test window: three 234 px rows do not fit into a 492 px area)
reads the dead pixels below the scroll clip - background with scattered event lines - which the
strip must and does not paint. On a window that shows every row (the mac) all three are checked.
"""
import time
from collections import Counter
from pathlib import Path

import pytest

from app_support import invoke, open_project
from helpers import SR, synth_track, write_wav
from test_follow_strip import lane_shot, pixels

VIEW = 8.0
STALL_MS = 800
IDLE = "() => !FR.raf && FR.flags === 0 && (!GLQ.eng || GLQ.eng.stats.drawn >= GLQ.eng.stats.posted)"
TRACKS = (("Kick In", "kick", 36), ("Snare Top", "snare", 38), ("Hat L", "hat", 42))
HITS = {"Kick In": [round(0.37 * k + 0.05, 6) for k in range(108)],
        "Snare Top": [round(0.53 * k + 0.21, 6) for k in range(75)],
        "Hat L": [round(0.11 * k + 0.13, 6) for k in range(362)]}

STATE = """() => {
  const now = performance.now();
  const row = document.querySelector('.track[data-name="Kick In"]');
  const tl = row && row.querySelector('.tlanes'), vel = row && row.querySelector('.vel'), wv = row && row.querySelector('.wave');
  const tr = tl && tl.getBoundingClientRect();
  const rel = c => { const r = c.getBoundingClientRect(); return { top: Math.round(r.top - tr.top), h: Math.round(r.height) }; };
  const box = document.querySelector('#lanesGLStrip');
  const br = box && box.getBoundingClientRect();
  const rows = {};
  for (const n of ['Kick In', 'Snare Top', 'Hat L']) {
    const el = document.querySelector(`.track[data-name="${n}"] .tlanes`);
    if (el) { const r = el.getBoundingClientRect(); rows[n] = { top: Math.round(r.top), h: Math.round(r.height) }; }
  }
  return { ph: cpPos(now), playing: S.playing, dpr: GEO.dpr, laneW: GEO.laneW, scW: GEO.scW,
    box: br ? { top: Math.round(br.top), h: Math.round(br.height) } : null, rows,
    sp: { on: SP.on, s: SP.s, d: SP.d, req: !!SP.req, done: SP.req ? !!SP.req.done : null },
    tl: tr ? { w: Math.round(tr.width), h: Math.round(tr.height) } : null,
    wave: wv && tr ? rel(wv) : null, vel: vel && tr ? rel(vel) : null };
}"""

VEL_LABELS = """() => { const c = document.querySelector('.track[data-name="Kick In"] .vel');
  const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data, cols = [];
  for (let x = 0; x < c.width; x++) { let n = 0;
    for (let y = 0; y < c.height; y += 2) { const i = (y * c.width + x) * 4, r = d[i], g = d[i + 1], b = d[i + 2];
      if (d[i + 3] > 200 && r >= 100 && r <= 190 && g >= 100 && g <= 200 && b >= 100 && b <= 210 && Math.max(r, g, b) - Math.min(r, g, b) <= 30) n++; }
    if (n >= 2) cols.push(x); }
  return { w: c.width, cols }; }"""


def project(make_project, tmp_path):
    """Three tracks with hits all over 40 s, so every part of the strip shows stalks and lines."""
    src = tmp_path / "src_seam"
    src.mkdir()
    specs = []
    for (name, role, note), ts in zip(TRACKS, HITS.values()):
        x, _ = synth_track(ts, 40.0)
        specs.append((name, write_wav(src / f"{name}.wav", x), role, note,
                      [{"t": round(t * SR) / SR, "v": 100} for t in ts]))
    return Path(make_project.builder.build_project(tmp_path / "projects_seam", "Seam", specs, "copied"))


def label_columns(region):
    """Columns (device px) of label text: bright low-saturation grey-blue pixels."""
    out = []
    for x in range(len(region[0])):
        n = 0
        for y in range(0, len(region), 2):
            r, g, b = region[y][x]
            if 100 <= r <= 190 and 100 <= g <= 200 and 100 <= b <= 210 and max(r, g, b) - min(r, g, b) <= 30:
                n += 1
        if n >= 2:
            out.append(x)
    return out


def empty_columns(region, tol=4):
    """Columns where every sampled pixel sits within tol of the region's modal colour."""
    mode = Counter()
    for y in range(0, len(region), 3):
        for x in range(0, len(region[0]), 3):
            mode[region[y][x]] += 1
    if not mode:
        return []
    bg = mode.most_common(1)[0][0]
    out = []
    for x in range(len(region[0])):
        for y in range(0, len(region), 2):
            if max(abs(a - b) for a, b in zip(region[y][x], bg)) > tol:
                break
        else:
            out.append(x)
    return out


def stalk_columns(region, color):
    """Columns holding a pixel near the track colour (a velocity stalk or its dot)."""
    r0, g0, b0 = color
    out = []
    for x in range(len(region[0])):
        for y in range(0, len(region), 2):
            r, g, b = region[y][x]
            if abs(r - r0) <= 40 and abs(g - g0) <= 40 and abs(b - b0) <= 40:
                out.append(x)
                break
    return out


@pytest.fixture
def vel_setting_back(app):
    """The test's vlToggle() persists the velocity_lane app setting in the session's bridge home
    (one ds-bridge server serves the whole pytest session, conftest `bridge` scope=session). Every
    later boot in the same folder run restores it: body.velOn puts each row's 80 px vel canvas in
    the flow and GEO_H 'header or row height' fails (this poisoned test_trackheight in the full
    folder runs; quiet reruns without this test were green). Leave the setting as the test found it."""
    was = app.evaluate("() => VL.on")
    yield
    if not was:
        try:
            invoke(app, "app_settings_set", key="velocity_lane", value=False)
        except Exception:
            pass   # the page may already be gone; the next session's bridge home starts fresh


def test_the_strip_shows_no_tile_seam_while_the_next_answer_is_late(app, make_project, tmp_path, vel_setting_back):
    open_project(app, project(make_project, tmp_path))
    app.evaluate("() => { TM.ui.grid = false; tmApplyUi(true); if (!VL.on) vlToggle(); }")
    # the hint text lives inside the vel lane and would trip the mid-lane label check: draw without it
    app.evaluate("() => { VL.hint = false; document.querySelectorAll('#lanesScroll .track').forEach(row => "
                 "{ const c = row.querySelector('.vel'); if (c && S.tracks[row.dataset.name]) drawVelLane(c, row.dataset.name); }); }")
    # the owner's scrollbar: the scroll's client width falls short of the lane's right edge
    # (waveX + laneW), so the pre-fix tile image (scW wide) cuts the lane there
    app.add_style_tag(content="#lanesScroll { width: calc(100% - 15px) !important; }")
    app.evaluate(f"() => {{ setFollow('smooth'); S.followHold = false; S.playhead = 0; setView(0, {VIEW}); }}")
    app.wait_for_function(IDLE, timeout=60000)
    lw, scw, wx, dpr = app.evaluate("() => [GEO.laneW, GEO.scW, GEO.waveX, GEO.dpr]")
    assert dpr == 1, f"the pixel checks assume a device scale of 1 (the test browser), got {dpr}"
    assert wx + lw - scw >= 10, (f"the test needs the scroll's client width short of the lane's right edge "
                                 f"(the owner's scrollbar): waveX {wx} + laneW {lw} = {wx + lw} vs scW {scw}")
    app.click("#playBtn")
    app.wait_for_function("() => S.playing && AC.running", timeout=30000)
    app.wait_for_function("() => SP.on", timeout=30000)
    # the entry: the velocity label sits at the lane's left edge of its canvas
    lab = app.evaluate(VEL_LABELS)
    assert lab["cols"] and min(lab["cols"]) < 70 * dpr, f"no velocity label at the lane's left edge at the entry: {lab}"

    colors = app.evaluate(
        "() => Object.fromEntries(['Kick In', 'Snare Top', 'Hat L'].map(n => [n, S.tracks[n].color]))")
    colors = {n: tuple(int(c[i:i + 2], 16) for i in (1, 3, 5)) for n, c in colors.items()}

    stalled, shots, guards = None, [], []
    t0 = time.monotonic()
    while time.monotonic() - t0 < 120 and len(shots) < 6:
        st = app.evaluate(STATE)
        if not st["playing"] or not st["sp"]["on"]:
            break
        s, d, ph = st["sp"]["s"], st["sp"]["d"], st["ph"]
        app.evaluate(f"() => {{ if (GLQ.eng) GLQ.eng.stall({STALL_MS}); }}")   # keep the worker behind: the next tiles come late
        if st["sp"]["req"] and not st["sp"]["done"] and stalled != s:
            stalled = s   # this strip's next tiles are the late ones
        # past the swap point (ph >= s + 0.75 d) with the answer still pending: the new tile column is in view
        if st["sp"]["req"] and not st["sp"]["done"] and stalled == s and ph >= s + d * 0.75 + 0.05 \
                and 0.45 <= (ph - 0.25 * d - s) / d <= 0.62:   # the new column safely in view (the checked band)
            guards.append(st)
            shots.append({n: pixels(lane_shot(app, n)) for n in HITS})
        if ph > s + d * 1.02:
            break   # the strip left late; its window is over
    assert shots, "no delayed frame captured: the stall did not hold the strip's swap past its swap point"

    for st, shot in zip(guards, shots):
        s, d, W = st["sp"]["s"], st["sp"]["d"], st["laneW"]
        o = (st["ph"] - 0.25 * d - s) / d * W   # the strip's slide at the frame
        assert 0.45 * W <= o <= 0.62 * W, f"the new column is not safely in view (slide {o} of {W}): {st['sp']}"
        assert st["box"], "no strip box at the delayed frame"
        checked = []
        for n, img in shot.items():
            r = st["rows"].get(n)
            # a row that pokes out of the strip's painted area: its element screenshot reads dead
            # pixels below the scroll clip, which the strip must not paint (a short test window)
            if not r or r["top"] < st["box"]["top"] or r["top"] + r["h"] > st["box"]["top"] + st["box"]["h"]:
                print(f"skip {n}: its row ({r}) is not inside the strip's area {st['box']}")
                continue
            checked.append(n)
            wave, vel = st["wave"], st["vel"]
            empty = empty_columns(img[wave["top"]:wave["top"] + wave["h"]])
            assert not empty, f"{n}: empty wave columns at the delayed frame (the tile gap), first {empty[:8]} (slide {o:.0f} of {W})"
            stalks = stalk_columns(img[vel["top"]:vel["top"] + vel["h"]], colors[n])
            assert [x for x in stalks if x > 0.7 * W], (
                f"{n}: no velocity stalks in the new tile column (x > 0.7 W), slide {o:.0f}")
            mid = [x for x in label_columns(img[vel["top"]:vel["top"] + vel["h"]]) if 0.25 * W < x < 0.75 * W]
            assert not mid, f"{n}: velocity label text in the middle of the lane (the stale one-view draw at the seam), columns {mid[:8]}"
        assert checked, f"no track fully inside the strip's area {st['box']}: rows {st['rows']}"
    app.evaluate("() => { if (S.playing) pause(); }")
