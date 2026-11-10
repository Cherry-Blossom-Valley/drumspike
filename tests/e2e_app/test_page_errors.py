"""#429: three defects where the page hid an error from the user: a rejected promise that no handler caught, a lost
peaks_build_status call that ended the lane build as a success, and the native confirm() of Re-analyze."""
import json
from pathlib import Path

from playwright.sync_api import expect

from app_support import menu, open_project, wait_until
from helpers import SR, synth_track, write_wav

ONSETS = [0.5, 1.5, 2.5]


def project(make_project, tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    x, _ = synth_track(ONSETS, 4.0)
    wav = write_wav(src / "Kick In.wav", x)
    events = [{"t": int(round(t * SR)) / SR, "v": 100} for t in ONSETS]
    return Path(make_project.builder.build_project(tmp_path / "projects", "Song", [("Kick In", wav, "kick", 36, events)], "copied"))


def test_a_menu_handler_that_rejects_puts_a_message_on_the_status_line(app, make_project, tmp_path):
    open_project(app, project(make_project, tmp_path))
    # a handler that starts work and does not await it: its rejection has no catch of the menu
    app.evaluate("""() => { DSShell.handlers.boom = () => { (async () => { throw new Error("menu boom"); })(); };
      setMsg(""); window.__seq = _msgSeq; }""")
    expect(app.locator("#stMsg")).to_have_text("")
    menu(app, "boom")
    expect(app.locator("#stMsg")).to_have_text("unexpected error: menu boom")
    assert app.evaluate("() => _msgSeq - window.__seq") == 1, "one message per error"
    # an error thrown in a timer reaches the same line
    app.evaluate("() => { setTimeout(() => { throw new Error('timer boom'); }, 0); }")
    expect(app.locator("#stMsg")).to_have_text("unexpected error: timer boom")
    assert any("menu boom" in e for e in app.errors), "the full error is in the console"
    app.errors.clear()   # the fixture fails a test on a console error; these two are the point of this one


def stub_build(app, status):
    """peaks_build_start answers one track started; peaks_build_status answers by `status(n)` (n = call number)."""
    calls = []

    def start(route):
        route.fulfill(status=200, content_type="application/json", body=json.dumps({"started": ["Kick In"]}))

    def poll(route):
        calls.append(1)
        answer = status(len(calls))
        if answer is None:
            route.fulfill(status=409, content_type="application/json", body=json.dumps({"code": 409, "message": "status lost"}))   # a 4xx is no console error of the fixture
        else:
            route.fulfill(status=200, content_type="application/json", body=json.dumps({"state": answer}))
    app.route("**/invoke/peaks_build_start", start)
    app.route("**/invoke/peaks_build_status", poll)
    return calls


def test_a_status_call_that_always_fails_rejects_the_build_and_shows_an_error(app, make_project, tmp_path):
    open_project(app, project(make_project, tmp_path))
    calls = stub_build(app, lambda n: None)
    app.evaluate("() => { setMsg(''); window.__r = 'pending'; B.prepareLanes(S.sid, () => {}).then(() => { window.__r = 'done'; }, e => { window.__r = 'rejected: ' + e.message; }); }")
    app.wait_for_function("() => window.__r !== 'pending'", timeout=10000)
    assert app.evaluate("() => window.__r") == "rejected: 409: status lost"
    assert len(calls) == 3, "a small fixed number of tries"
    # the page: the lane build reports the error, not the done state
    app.evaluate("() => { setMsg(''); prepareLanes(S.sid); }")
    expect(app.locator("#stMsg")).to_contain_text("waveform build failed: status lost")
    assert app.evaluate("() => document.querySelector('#progressWrap').classList.contains('hidden')") is True


def test_a_status_call_that_fails_once_and_then_answers_ends_as_done(app, make_project, tmp_path):
    open_project(app, project(make_project, tmp_path))
    calls = stub_build(app, lambda n: None if n == 1 else "done")
    app.evaluate("() => { window.__r = 'pending'; B.prepareLanes(S.sid, () => {}).then(() => { window.__r = 'done'; }, e => { window.__r = 'rejected: ' + e.message; }); }")
    app.wait_for_function("() => window.__r !== 'pending'", timeout=10000)
    assert app.evaluate("() => window.__r") == "done"
    assert len(calls) == 2, "the lost call was tried again"


def test_reanalyze_asks_in_the_page_and_cancel_keeps_the_events(app, make_project, tmp_path):
    open_project(app, project(make_project, tmp_path))
    app.on("dialog", lambda d: (_ for _ in ()).throw(AssertionError("a native dialog opened: " + d.message)))
    before = app.evaluate("() => S.events['Kick In'].length")
    assert before > 0
    app.click("#analyzeAllBtn")
    expect(app.locator("#confirmModal")).to_be_visible()
    expect(app.locator("#confirmMsg")).to_contain_text("overwrites their events")
    app.click("#confirmActions button[data-act=cancel]")
    expect(app.locator("#confirmModal")).to_be_hidden()
    expect(app.locator("#stMsg")).to_have_text("re-analyze cancelled")
    assert app.evaluate("() => S.events['Kick In'].length") == before
    assert "analyze_start" not in app.invokes
    # OK starts the analysis
    app.click("#analyzeAllBtn")
    app.click("#confirmActions button[data-act=go]")
    wait_until(lambda: "analyze_start" in app.invokes, timeout=60, msg="OK started the analysis")
    app.wait_for_function("() => !analysisRunning", timeout=60000)
