"""#175: the setting "Tab moves to: active hits only | all found hits" and the visibility of a selected
deactivated hit. Default: Tab (and Shift+Tab) skip the deactivated hit (3 hits, 1 deactivated: Tab visits
2); with "all found hits" Tab visits 3; a selected deactivated hit draws a dashed grey line (pixel check
against a selected regular hit on the same lane) and the counter and the info line say "deactivated";
the setting persists across a reload. The status line names the mode when it changes. A cropped screenshot
of the deactivated state is written to $DRUMSPIKE_175_SHOT when that names a file."""
import os

import pytest

from app_support import menu, open_project
from test_menus import with_hits

SR = 48000


def sel(page):
    return page.evaluate("() => S.sel")


def tab_mode(page):
    return page.evaluate("() => SETTINGS().ui.tabMode")


def playhead(page):
    return page.evaluate("() => S.playhead")


def cut_middle(page, sid, track="Kick In"):
    """Mark the middle of the three hits deactivated (cut), like a ghost note under the threshold."""
    return page.evaluate("""async ([sid, track]) => {
      const evs = (S.events[track] || []).map(e => ({ t: e.t, v: e.v, id: e.id }));
      if (evs.length !== 3) return false;
      evs[1].cut = true;
      await B.setEvents(sid, track, { events: evs });
      await loadProject();
      return (S.events[track] || []).length === 3;
    }""", [sid, track])


@pytest.fixture
def song(app, make_project):
    sid = open_project(app, make_project(seconds=6.0))
    with_hits(app, sid, {"Kick In": [1.0, 2.0, 3.0]})
    assert cut_middle(app, sid)
    app.evaluate("() => { S.playhead = 0.5; S.sel = null; S.selTrack = null; setView(0, 6); }")
    return app


@pytest.fixture(autouse=True)
def canvas_2d_lanes(app):
    """The pixel check reads the Canvas 2D lane; the GL lanes restore themselves."""
    assert app.evaluate("() => setLanesGL(false)") is False
    yield
    app.evaluate("() => setLanesGL(true)")


def test_tab_skips_the_deactivated_hit_by_default(song):
    assert tab_mode(song) == "active"
    song.keyboard.press("Tab")
    assert abs(playhead(song) - 1.0) < 1e-6   # the first hit
    song.keyboard.press("Tab")
    assert abs(playhead(song) - 3.0) < 1e-6   # the deactivated 2.0 is skipped
    song.keyboard.press("Shift+Tab")
    assert abs(playhead(song) - 1.0) < 1e-6   # Shift+Tab follows the same rule
    assert sel(song) == {"track": "Kick In", "index": 0}


def test_all_found_hits_visits_the_deactivated_hit_too(song):
    assert menu(song, "tab_all") is True
    assert tab_mode(song) == "all"
    song.keyboard.press("Tab")
    assert abs(playhead(song) - 1.0) < 1e-6
    song.keyboard.press("Tab")
    assert abs(playhead(song) - 2.0) < 1e-6   # the deactivated hit is visited
    song.keyboard.press("Tab")
    assert abs(playhead(song) - 3.0) < 1e-6
    assert menu(song, "tab_active") is True
    assert tab_mode(song) == "active"
    song.keyboard.press("Shift+Tab")
    assert abs(playhead(song) - 1.0) < 1e-6   # active again: 2.0 is skipped backwards


def test_the_status_line_names_the_mode_on_a_toggle(song):
    song.evaluate("() => setMsg('')")
    menu(song, "tab_all")
    assert "all found hits" in song.locator("#stMsg").text_content()
    song.evaluate("() => setMsg('')")
    menu(song, "tab_active")
    assert "active hits only" in song.locator("#stMsg").text_content()


def test_the_setting_persists_across_a_reload(song):
    menu(song, "tab_all")
    assert tab_mode(song) == "all"
    song.reload()
    song.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0")
    assert tab_mode(song) == "all"
    menu(song, "tab_active")
    song.reload()
    song.wait_for_function("() => typeof S !== 'undefined' && S.loaded > 0")
    assert tab_mode(song) == "active"


def line_columns(page, track):
    """Every column of the track's spike lane that carries lit pixels, with its colour classes:
    [{x, lit, yellow, grey}] (lit rows, whether a pixel is the selection yellow, whether a pixel
    is a medium grey). The hit lines run the full lane height; the markers and the ring are a few
    rows at the top."""
    return page.evaluate("""n => {
      const c = document.querySelector(`.track[data-name="${n}"] canvas.spike`);
      if (!c || !c.width || !c.height) return null;
      const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
      const out = [];
      for (let x = 0; x < c.width; x++) {
        let lit = 0, yellow = 0, grey = 0;
        for (let y = 0; y < c.height; y++) {
          const i = (y * c.width + x) * 4;
          if (d[i + 3] <= 40) continue;
          lit++;
          const r = d[i], g = d[i + 1], b = d[i + 2];
          if (r > 200 && g > 180 && b < 160) yellow++;
          else if (Math.abs(r - g) < 24 && Math.abs(g - b) < 24 && 90 < r && r < 190) grey++;
        }
        if (lit) out.push({ x, lit, yellow, grey });
      }
      return { h: c.height, cols: out };
    }""", track)


def select_hit(page, index):
    page.evaluate("i => { S.sel = { track: 'Kick In', index: i }; updateStatus(); invalidate(F.VIEW | F.NEAR); }", index)
    page.wait_for_timeout(150)


def shot_path(tmp_path):
    dest = os.environ.get("DRUMSPIKE_175_SHOT")
    return dest if dest else str(tmp_path / "selected-deactivated.png")


def test_a_selected_deactivated_hit_stays_visibly_deactivated(song, tmp_path):
    def yellow_line(scan):
        return [c for c in scan["cols"] if c["yellow"] > scan["h"] * 0.7]

    def grey_line(scan):
        return [c for c in scan["cols"] if c["lit"] > scan["h"] * 0.55 and c["grey"] >= c["lit"] * 0.8]

    # the selected regular hit: its line is the selection yellow over the lane height
    select_hit(song, 0)
    regular = line_columns(song, "Kick In")
    assert regular, "the spike lane is readable"
    assert yellow_line(regular), \
        f"the selected regular hit line is the selection yellow; cols {regular['cols'][:12]}"

    # the selected deactivated hit: a grey line, never the full yellow of a regular hit
    select_hit(song, 1)
    deactivated = line_columns(song, "Kick In")
    assert deactivated, "the spike lane is readable"
    assert grey_line(deactivated), \
        f"the selected deactivated hit line is grey; cols {deactivated['cols'][:12]}"
    assert not yellow_line(deactivated), \
        f"the selected deactivated hit line is not the full yellow of a regular hit; cols {deactivated['cols'][:12]}"

    # the counter and the info line say "deactivated"; a regular hit does not
    assert "deactivated" in song.locator("#stSel").text_content()
    assert "deactivated" in song.evaluate("() => $('#ilIndex').textContent")
    select_hit(song, 0)
    assert "deactivated" not in song.locator("#stSel").text_content()

    # one cropped screenshot of the deactivated state (the spike lane of the track)
    out = shot_path(tmp_path)
    song.locator(".track[data-name='Kick In'] canvas.spike").screenshot(path=out)
    assert os.path.getsize(out) > 0
    select_hit(song, 1)   # back to the deactivated state for any later look
    print(f"\n[cropped screenshot] {out}")
