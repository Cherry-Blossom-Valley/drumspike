"""P4-26: lane polish. (1) Tab held: no coarse-envelope blocks between a jump and its sample window (GLQ.hold) and no
stale async draw in the Canvas 2D path (a generation per canvas). (2) The filled waveform from a visible span of 500 ms
(WAVE_FILL_S). (3) The key preset is the default (Pro Tools) when no settings file exists, and the Pro Tools keys are in
the default table. (4) The window title and the edited flag travel in the shell_sync state. Each guard has a
planted-fault check: the test switches the guard off (GLQ.noHold, GLQ.noGen, WAVE_FILL_S, a settings file with cubase)
and sees the fault."""
import json

from app_support import open_project
from test_keypreset import preset_home, reload  # noqa: F401  (the fixture, the reload helper)
from test_menus import empty_page, with_hits

JUMPS = """async () => { const m0 = GLQ.eng.stats.modes.slice();
  for (let i = 0; i < 3; i++) { gotoEvent(1); await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))); }
  await new Promise(r => setTimeout(r, 400));
  return GLQ.eng.stats.modes.map((x, k) => x - m0[k]); }"""


def tab_setup(app, make_project):
    open_project(app, make_project(seconds=8.0))
    sid = app.evaluate("() => S.sid")
    with_hits(app, sid, {"Kick In": [0.5, 2.0, 3.5, 5.0, 6.5], "Snare Top": [1.0]})
    app.wait_for_function("() => GLQ.eng.stats.drawn >= 1")
    app.evaluate("() => { S.selTrack = 'Kick In'; S.playhead = 0.5; setView(0.475, 0.525); }")   # 50 ms: the sample window shows
    app.wait_for_function("() => GLQ.eng.textures().samples >= 1")
    app.evaluate("() => new Promise(r => setTimeout(r, 300))")


def test_tab_jumps_out_of_the_window_draw_no_coarse_envelope(app, make_project):
    tab_setup(app, make_project)
    modes = app.evaluate(JUMPS)
    assert modes[0] == 0 and modes[1] > 0, modes                  # only exact (sample window) draws reached the worker
    assert app.evaluate("() => GLQ.held") >= 1


FAULT_JUMPS = """async () => { const real = ensureSamples, gates = [];   // the fault shows only while the window fetch is pending
  window.ensureSamples = (n, s, e) => new Promise(res => gates.push(() => real(n, s, e).then(res)));
  GLQ.noHold = true;
  const m0 = GLQ.eng.stats.modes.slice();
  for (let i = 0; i < 3; i++) { gotoEvent(1); await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))); }
  await new Promise(r => setTimeout(r, 400));
  const modes = GLQ.eng.stats.modes.map((x, k) => x - m0[k]);
  window.ensureSamples = real; gates.forEach(g => g());          // the answers come after the read: they cannot mask the fault
  return modes; }"""


def test_planted_fault_without_the_hold_the_envelope_blocks_show(app, make_project):
    tab_setup(app, make_project)
    # #220 gate run 3: on the loaded host the sample-window answer beat the draw pass, every draw
    # was exact ([0, 2, 0]) and the planted fault never showed. Hold the answers back (the GATED
    # idiom) so the draw pass runs with the fetch pending: without the hold, the envelope draws.
    modes = app.evaluate(FAULT_JUMPS)
    assert modes[0] > 0, modes                                    # the old behaviour: a coarse envelope draw per jump


GATED = """async (noGen) => {
  GLQ.noGen = noGen;
  await setLanesGL(false);
  const name = S.order[0], canvas = document.querySelector(`.track[data-name="${name}"] canvas.wave`), real = ensureSamples, gates = [];
  window.ensureSamples = (n, s, e) => new Promise(res => gates.push(() => real(n, s, e).then(res)));
  let strokes = 0; const ctx = canvas.getContext("2d"), st = ctx.stroke.bind(ctx); ctx.stroke = () => { if (ctx.strokeStyle === S.tracks[name].color.toLowerCase()) strokes++; st(); };
  setView(0.40, 0.45); drawWave(canvas, name);                  // draw A waits for its samples
  setView(0.45, 0.50); drawWave(canvas, name);                  // draw B waits too; B is the newer one
  await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));   // the frame draw that setView schedules waits too
  gates.reverse().forEach(g => g());                            // the answers come late and in the other order
  await new Promise(r => setTimeout(r, 500));
  window.ensureSamples = real; ctx.stroke = st; GLQ.noGen = false;
  return strokes; }"""


def test_the_2d_path_drops_a_stale_draw(app, make_project):
    open_project(app, make_project(seconds=8.0))
    app.wait_for_function("() => GLQ.eng.stats.drawn >= 1")
    assert app.evaluate(GATED, False) == 1                        # one pass drew the wave line
    app.evaluate("() => setLanesGL(true)")


def test_planted_fault_without_the_generation_the_stale_draw_lands(app, make_project):
    open_project(app, make_project(seconds=8.0))
    app.wait_for_function("() => GLQ.eng.stats.drawn >= 1")
    assert app.evaluate(GATED, True) >= 2                         # the stale passes drew over each other
    app.evaluate("() => setLanesGL(true)")


SOLID = """async () => { const name = S.order[0], c = S.tracks[name].color, want = [1, 3, 5].map(i => parseInt(c.slice(i, i + 2), 16));
  const d = (await laneImage(name)).data; let solid = 0, blend = 0;   // blend: neither the lane background (16, 19, 24) nor the track colour
  const near = (i, w) => Math.abs(d[i] - w[0]) < 3 && Math.abs(d[i + 1] - w[1]) < 3 && Math.abs(d[i + 2] - w[2]) < 3;
  for (let i = 0; i < d.length; i += 4) { if (near(i, want)) solid++; else if (!near(i, [16, 19, 24])) blend++; }
  return [solid, blend]; }"""


def solid_stable(app):
    # #96: a redraw can be posted between the drawn>=posted wait and the read; the read
    # then catches a half-drawn frame (CI 39114398125: filled saw 3108 blend pixels with
    # the envelope still in). The transport is stopped here, so redraws come only from
    # our own posts: read, pump one frame, read again, return the first pair that
    # agrees (5 tries; each try is one frame plus a redraw).
    b = []
    for _ in range(5):
        app.wait_for_function("() => GLQ.eng.stats.drawn >= GLQ.eng.stats.posted")
        a = app.evaluate(SOLID)
        app.evaluate("() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))")
        app.wait_for_function("() => GLQ.eng.stats.drawn >= GLQ.eng.stats.posted")
        b = app.evaluate(SOLID)
        if a == b:
            return b
    return b


def test_the_waveform_is_filled_from_500_ms_and_not_below(app, make_project):
    open_project(app, make_project(seconds=8.0))
    app.wait_for_function("() => GLQ.eng.stats.drawn >= 1")
    flags = app.evaluate("""() => { const name = S.order[0], f = d => { setView(1, 1 + d); return glLane(document.querySelector(`.track[data-name="${name}"] canvas.wave`), name, S.view).fill; };
      return [f(0.4), f(0.499), f(0.5), f(0.6), f(20)]; }""")
    assert flags == [False, False, True, True, True]
    app.evaluate("() => setView(0, 8)")                            # 8 s: the tiles (no sample window)
    app.evaluate("() => { TM.ui.grid = false; tmApplyUi(true); }") # P4-29: the grid lines are drawn in the lane and would count as blend in both measurements
    app.wait_for_function("() => GLQ.eng.stats.drawn >= GLQ.eng.stats.posted")
    filled = solid_stable(app)
    app.evaluate("() => { WAVE_FILL_S = 100; redrawAll(); }")      # planted fault: the threshold above the view: the old look
    app.wait_for_function("() => GLQ.eng.stats.drawn >= GLQ.eng.stats.posted")
    assert app.evaluate("() => glLane(document.querySelector('.track canvas.wave'), S.order[0], S.view).fill") is False
    old = solid_stable(app)
    app.evaluate("() => { WAVE_FILL_S = 0.5; redrawAll(); }")
    assert filled[0] > old[0] and filled[0] > 500, (filled, old)
    assert filled[1] * 1.5 < old[1], (filled, old)                  # no translucent envelope pixels left: one solid shape (the centre line is the rest)


PRO_TOOLS = {"next_hit": "Tab", "prev_hit": "Option+Tab", "zoom_out": "R", "zoom_in": "T", "nudge_earlier": "NumpadMinus",
             "nudge_later": "NumpadPlus", "go_start": "Return", "play_stop": "Space"}


def test_no_settings_file_means_the_pro_tools_preset(preset_home, make_project):
    app = preset_home
    settings = app.home / "settings.json"
    settings.unlink(missing_ok=True)
    reload(app)
    assert app.evaluate("() => DSKeys.preset") == "default"
    assert app.evaluate("() => DSShell.menuState().keyPreset") == "default"
    keys = app.evaluate("() => Object.fromEntries(DSKeys.COMMANDS.map(c => [c.id, DSKeys.keysOf(c, DSKeys.preset)]))")
    for cmd, key in PRO_TOOLS.items():
        assert key in keys[cmd], (cmd, keys[cmd])
    assert "Cmd+[" in keys["zoom_out"] and "Cmd+]" in keys["zoom_in"]
    # planted fault: the same check with a settings file that says cubase fails, so it can fire
    settings.write_text(json.dumps({"key_preset": "cubase"}))
    reload(app)
    assert app.evaluate("() => DSKeys.preset") == "cubase"
    assert app.evaluate("() => DSKeys.keysOf(DSKeys.COMMANDS.find(c => c.id === 'next_hit'), DSKeys.preset)")[0] != "Tab"


def test_the_shell_sync_state_carries_the_window_title(app, make_project):
    open_project(app, make_project())
    st = app.evaluate("() => DSShell.menuState()")
    assert st["projectName"] == app.evaluate("() => S.name") and st["projectName"] and st["dirty"] is False
    app.evaluate("() => { window.__syncs = 0; const s = DSShell.sync; DSShell.sync = () => { window.__syncs++; return s(); }; setDirtyState(true); }")
    assert app.evaluate("() => window.__syncs") >= 1               # a dirty change asks for a sync
    assert app.evaluate("() => DSShell.menuState().dirty") is True
    app.evaluate("() => setDirtyState(false)")
    empty_page(app)
    st = app.evaluate("() => DSShell.menuState()")
    assert st["projectName"] == "" and st["dirty"] is False
