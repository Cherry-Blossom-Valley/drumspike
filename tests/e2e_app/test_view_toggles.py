"""#439: page keys for the View toggles that had none, and the menu check item follows.

Shift+B (Bars|Beats Ruler), Shift+R (Markers Ruler) and Option+Cmd+1/2/3 (Track Height
small/medium/large) run the same handlers as the menu items; `__DS_MENU_STATE__` is the check
state the native menu greys and checks from (DSShell.sync), so a flip here is the menu check
item following. Velocity Lane (V) and Other Mics Under Hit (O) had keys already.
"""
import pytest

from app_support import open_project


def state(app):
    return app.evaluate("() => window.__DS_MENU_STATE__")


@pytest.fixture
def song(app, make_project):
    yield open_project(app, make_project())


def test_shift_b_toggles_the_bars_beats_ruler_and_the_check_item_follows(app, song):
    s0 = state(app)["beatsRuler"]
    app.keyboard.press("Shift+B")
    assert state(app)["beatsRuler"] is (not s0)
    app.keyboard.press("Shift+B")
    assert state(app)["beatsRuler"] is s0


def test_shift_r_toggles_the_markers_ruler_and_the_check_item_follows(app, song):
    s0 = state(app)["markersRuler"]
    app.keyboard.press("Shift+R")
    assert state(app)["markersRuler"] is (not s0)
    app.keyboard.press("Shift+R")
    assert state(app)["markersRuler"] is s0


def test_option_cmd_1_2_3_set_the_track_heights_and_the_check_item_follows(app, song):
    app.keyboard.press("Alt+Meta+1")
    assert state(app)["trackHeight"] == "small"
    app.keyboard.press("Alt+Meta+3")
    assert state(app)["trackHeight"] == "large"
    app.keyboard.press("Alt+Meta+2")
    assert state(app)["trackHeight"] == "medium"
