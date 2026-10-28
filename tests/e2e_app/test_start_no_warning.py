"""#281 D16b: the app starts and shows no warning.

The release main (#294) ships without the old detector, so the status line must not show an
error or a fallback/reduced-mode warning at start. The check is `no_warning(app)`:
`#stDet` carries no warn/err class and no warning text.

`test_planted_warning_is_detected` proves the check is not vacuous: with
DRUMSPIKE_PLANT_START_WARNING=1 it plants the old fallback warning in `#stDet` and asserts
that the same check FAILS. Without the env var the test skips (the planted run is evidence
for the #308 report, run explicitly).
"""
import os

import pytest

FALLBACK_NOTE = "No usable model: fallback detector, check toms and cymbals"

WARN_WORDS = ("no usable model", "fallback detector", "model error", "adtof", "model:")


def no_warning(page) -> bool:
    """True while the detector status line shows no warning (no warn/err class, no warning text)."""
    state = page.evaluate("""() => { const el = document.querySelector('#stDet');
        return { text: el ? el.textContent : null, warn: el ? el.classList.contains('warn') : null,
                 err: el ? el.classList.contains('err') : null }; }""")
    assert state["text"] is not None, "no #stDet in the page"
    if state["warn"] or state["err"]:
        return False
    low = state["text"].lower()
    return not any(w in low for w in WARN_WORDS)


def test_start_shows_no_warning(app):
    """The start of the release app: no warning anywhere."""
    assert no_warning(app), app.evaluate(
        "() => document.querySelector('#stDet')?.textContent")


def test_planted_warning_is_detected(app):
    """With DRUMSPIKE_PLANT_START_WARNING=1: a planted fallback warning makes the check fail."""
    if not os.environ.get("DRUMSPIKE_PLANT_START_WARNING"):
        pytest.skip("set DRUMSPIKE_PLANT_START_WARNING=1 to run the planted-warning proof")
    app.evaluate(f"() => {{ const el = document.querySelector('#stDet'); el.textContent = {FALLBACK_NOTE!r};"
                 " el.classList.add('warn'); }")
    assert not no_warning(app), "the planted warning was NOT detected"
    app.evaluate("() => { const el = document.querySelector('#stDet'); el.textContent = '';"
                 " el.classList.remove('warn'); }")
