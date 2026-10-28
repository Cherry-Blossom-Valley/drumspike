"""P4-67b (#73): wait_until sees the lists that page events fill (app.invokes).

The Playwright sync API delivers a request event to its Python listener only inside a Playwright call. The page
sends the request after the evaluate returned (a timer), so only the pause of wait_until can let the event in."""
import pytest

import app_support
from app_support import wait_until

LATE = """() => { setTimeout(() => fetch('/invoke/app_settings_get', {method: 'POST',
    headers: {'Content-Type': 'application/json'}, body: '{}'}), 200); }"""


def test_a_condition_on_app_invokes_starves_with_time_sleep_and_passes_with_the_pump(app, monkeypatch):
    n = app.invokes.count("app_settings_get")
    app.evaluate(LATE)
    monkeypatch.setattr(app_support, "PUMP", None)          # the old wait_until: time.sleep only
    with pytest.raises(AssertionError, match="the late request"):
        wait_until(lambda: app.invokes.count("app_settings_get") > n, timeout=2.0, msg="the late request")
    monkeypatch.undo()                                       # the fixture's pump again
    assert app_support.PUMP is not None
    app.wait_for_timeout(500)                                # lets the first request in, so it cannot count below
    n = app.invokes.count("app_settings_get")
    app.evaluate(LATE)
    wait_until(lambda: app.invokes.count("app_settings_get") > n, timeout=5.0, msg="the late request with the pump")
