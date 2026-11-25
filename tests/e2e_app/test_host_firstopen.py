"""#578 (DAW plugin M1d4): clear states at the first open.

The plugin keeps one state per DAW track row and one for the document (plugins/ara/src/firstopen.rs: the shell command plugin_host_states
and the page event plugin-states). static/hoststate.js only draws them. These tests send the payload the plugin sends and read what the page
shows: the document line, the strip in a track header, the card of a DAW track that has no project track yet, Retry and Retry all, one
announcement per change, one error line. The plugin side has its own stages (plugins/ara/check.py, editor-firstopen-*).
The standalone page has none of it: the last tests."""
import pytest

from test_host_page import FOUR, host_open
from app_support import open_project

K, S_, T, O = (n for n, _, _ in FOUR)


def row(rid, name, state, text, srcs=(), hits=0, retry=False, old=False):
    return {"id": rid, "name": name, "srcs": list(srcs), "state": state, "text": text, "hits": hits, "old_hits": old, "retry": retry}


def doc_of(rows, state, text, attention=0):
    return {"state": state, "text": text, "n": 0, "m": len(rows), "hits": sum(r["hits"] for r in rows), "attention": attention}


def payload(rows, state, text, gen=1, attention=0, doc_id="d1"):
    return {"gen": gen, "doc_id": doc_id, "doc": doc_of(rows, state, text, attention), "rows": rows}


@pytest.fixture
def host(host_app, make_project):
    host_open(host_app, make_project(tracks=FOUR, seconds=6.0))
    host_app.evaluate("""() => { window.__retries = []; B.hostStates = async () => window.__answer || null;
      B.hostRetry = async ids => { window.__retries.push(ids); return { ok: true, started: ids, refused: [] }; }; HST.init(); }""")
    yield host_app
    host_app.evaluate("() => { HST.st = null; HST.gen = -1; HST.fault = ''; HST.error = ''; HST.paint(); }")


def give(page, p):
    page.evaluate("p => HST.take(p)", p)
    page.wait_for_function("() => !FR.raf && FR.flags === 0")


def line(page):
    return page.evaluate("() => { const e = document.getElementById('hstLine'); return e ? { text: e.querySelector('.hstText').textContent, state: e.dataset.state, role: e.getAttribute('role'), live: e.getAttribute('aria-live'), atomic: e.getAttribute('aria-atomic'), quiet: e.classList.contains('hstQuiet') } : null; }")


def strips(page):
    return page.evaluate("() => [...document.querySelectorAll('#lanesScroll .hostState')].map(e => ({ row: e.dataset.row, state: e.dataset.state, text: e.querySelector('.hstRowText').textContent, retry: !!e.querySelector('.hstRetry'), inHeader: !!e.closest('.thead') }))")


def cards(page):
    return page.evaluate("() => [...document.querySelectorAll('#lanesScroll .hostCard')].map(e => ({ row: e.dataset.row, state: e.dataset.state, text: e.querySelector('.hstRowText').textContent, retry: !!e.querySelector('.hstRetry') }))")


def test_the_line_is_a_polite_live_region_and_says_what_the_document_does(host):
    rows = [row("q1", "Kick In", "reading", "Kick In: reading the audio", [K]), row("q2", "Snare Top", "analysing", "Snare Top: analysing", [S_])]
    give(host, payload(rows, "reading", "Reading the audio: 1 of 2 tracks."))
    ln = line(host)
    assert ln["text"] == "Reading the audio: 1 of 2 tracks." and ln["role"] == "status" and ln["live"] == "polite" and ln["atomic"] == "true"
    give(host, payload(rows, "analysing", "Analysing: 0 of 2 tracks.", gen=2))
    assert line(host)["text"] == "Analysing: 0 of 2 tracks."


def test_a_row_that_is_not_done_has_a_strip_in_its_header_with_the_text_of_the_plugin(host):
    rows = [row("q1", "Kick In", "reading", "Kick In: reading the audio", [K]), row("q2", "Snare Top", "waiting", "Snare Top: waiting for the audio from your DAW", [S_]),
            row("q3", "Tom 1", "done", "Tom 1: 12 hits", [T], hits=12), row("q4", "Overhead L", "done", "Overhead L: no hits found: check the role and the threshold", [O])]
    give(host, payload(rows, "reading", "Reading the audio: 2 of 4 tracks."))
    got = {s["row"]: s for s in strips(host)}
    assert got["q1"]["text"] == "Kick In: reading the audio" and got["q1"]["inHeader"]
    assert got["q2"]["text"] == "Snare Top: waiting for the audio from your DAW"
    assert "q3" not in got, "a done row with hits has no strip: the count stays in the header and the info line"
    assert got["q4"]["text"].endswith("no hits found: check the role and the threshold"), "a done row with no hits says so, with a hint"
    assert not any(s["retry"] for s in got.values())


def test_a_daw_track_with_no_project_track_gets_a_card_and_the_empty_page_never_shows(host):
    rows = [row("q9", "Crash", "reading", "Crash: reading the audio", ["missing-key"])]
    give(host, payload(rows, "reading", "Reading the audio: 0 of 1 track."))
    c = cards(host)
    assert c == [{"row": "q9", "state": "reading", "text": "Crash: reading the audio", "retry": False}]
    assert host.evaluate("() => document.querySelectorAll('.hostCard').length") == 1
    assert "put the Drum Spike plugin on a drum track" not in host.inner_text("#lanesScroll"), "the old empty text is gone when the plugin gives a state"


def test_the_empty_state_says_what_to_do_in_the_daw_and_never_the_old_text(host):
    give(host, payload([], "nocip", "No audio yet. In your DAW, add an audio clip on a track that has the Drum Spike plugin."))
    text = host.evaluate("() => { const d = document.createElement('div'); renderHostEmpty(d); return d.textContent; }")
    assert text == "No audio yet. In your DAW, add an audio clip on a track that has the Drum Spike plugin."
    assert line(host)["state"] == "nocip"
    before = host.evaluate("() => { HST.st = null; const d = document.createElement('div'); renderHostEmpty(d); return d.textContent; }")
    assert before == "Checking the tracks of your DAW…", "before the first answer the page says that it checks, never an empty page"


def test_a_failed_row_shows_the_reason_and_retry_and_retry_all_needs_more_than_one(host):
    one = [row("q1", "Kick In", "failed", "Kick In: the DAW did not hand over the audio. Press Retry. If it fails again, check the clip in the DAW.", [K], retry=True),
           row("q2", "Snare Top", "done", "Snare Top: 4 hits", [S_], hits=4)]
    give(host, payload(one, "attention", "1 of 2 tracks needs attention.", attention=1))
    s = strips(host)
    assert len(s) == 1 and s[0]["retry"] and "did not hand over the audio" in s[0]["text"]
    assert host.locator("#hstLine .hstRetryAll").count() == 0, "one failed row: only its own Retry"
    host.click("#lanesScroll .hostState .hstRetry")
    assert host.evaluate("() => window.__retries") == [["q1"]]
    two = one[:1] + [row("q2", "Snare Top", "offline", "Snare Top: the DAW gives no audio for this track.", [S_], retry=True)]
    give(host, payload(two, "attention", "2 of 2 tracks need attention.", gen=2, attention=2))
    host.click("#hstLine .hstRetryAll")
    assert host.evaluate("() => window.__retries")[-1] == ["q1", "q2"]
    assert host.locator("#hstLine .hstRetryAll").get_attribute("class") and host.locator("#lanesScroll .hstRetry").count() == 2


def test_a_track_on_two_daw_rows_has_one_strip_for_each_row_with_its_own_name_and_retry(host):
    """#593: the page has ONE lane for a track but the plugin gives one state row for each DAW track row that holds a clip of it: each row gets its own strip."""
    two = [row("q1", "Kick In", "failed", "Kick In: the DAW did not hand over the audio. Press Retry.", [K], retry=True),
           row("q5", "Bus", "failed", "Bus: the DAW did not hand over the audio. Press Retry.", [K], retry=True)]
    give(host, payload(two, "attention", "2 of 2 tracks need attention.", attention=2))
    s = strips(host)
    assert [x["row"] for x in s] == ["q1", "q5"], s
    assert s[0]["text"].startswith("Kick In:") and s[1]["text"].startswith("Bus:"), "the second row keeps its own name"
    assert all(x["inHeader"] and x["retry"] for x in s)
    host.locator("#lanesScroll .hostState[data-row='q5'] .hstRetry").click()
    assert host.evaluate("() => window.__retries")[-1] == ["q5"], "the Retry of the second strip asks for the second row"
    one = [two[0], row("q5", "Bus", "done", "Bus: 4 hits", [K], hits=4)]
    give(host, payload(one, "attention", "1 of 2 tracks needs attention.", gen=2, attention=1))
    assert [x["row"] for x in strips(host)] == ["q1"], "a strip that its row no longer needs goes, the other stays"


def test_a_request_of_the_state_that_never_answers_ends_and_the_poll_goes_on(host):
    """#593: a hostStates call that never returns used to end the poll of the page for ever."""
    rows = [row("q1", "Kick In", "analysing", "Kick In: analysing", [K])]
    give(host, payload(rows, "analysing", "Analysing: 0 of 1 track.", gen=3))
    res = host.evaluate("""async () => { clearTimeout(HST.poll); HST.reqMs = 150; const n0 = HST.pulls;
      B.hostStates = () => new Promise(() => {});
      const r = await HST.pull();
      return { r, polling: HST.poll !== null, absent: HST.absent, pulls: HST.pulls - n0, kept: HST.st && HST.st.gen }; }""")
    assert res == {"r": False, "polling": True, "absent": False, "pulls": 1, "kept": 3}, res
    host.evaluate("() => { HST.reqMs = 5000; clearTimeout(HST.poll); }")


def test_an_answer_that_comes_after_the_limit_is_still_used_when_it_is_newer(host):
    """#593 round 2 (N9): a slow plugin must still update the page."""
    rows = [row("q1", "Kick In", "analysing", "Kick In: analysing", [K])]
    give(host, payload(rows, "analysing", "Analysing: 0 of 1 track.", gen=3))
    newer = payload(rows, "analysing", "Analysing: 0 of 1 track.", gen=4)
    host.evaluate("""async p => { clearTimeout(HST.poll); HST.reqMs = 80; B.hostStates = () => new Promise(ok => setTimeout(() => ok(p), 300)); await HST.pull(); }""", newer)
    host.wait_for_function("() => HST.st.gen === 4")
    host.evaluate("() => { HST.reqMs = 5000; clearTimeout(HST.poll); }")


def test_a_state_change_is_written_once_not_at_every_poll(host):
    rows = [row("q1", "Kick In", "analysing", "Kick In: analysing", [K])]
    give(host, payload(rows, "analysing", "Analysing: 0 of 1 track."))
    n = host.evaluate("() => HST.writes")
    for g in range(2, 8):
        give(host, payload(rows, "analysing", "Analysing: 0 of 1 track.", gen=g))
    assert host.evaluate("() => HST.writes") == n, "the same state again writes nothing, so a screen reader says nothing"
    give(host, payload([row("q1", "Kick In", "analysing", "Kick In: analysing", [K])], "analysing", "Analysing: 1 of 2 tracks.", gen=9))
    assert host.evaluate("() => HST.writes") == n + 1


def test_the_fault_run_that_writes_at_every_poll_is_caught_by_the_same_count(host):
    host.evaluate("() => { HST.fault = 'first-announce'; }")
    rows = [row("q1", "Kick In", "analysing", "Kick In: analysing", [K])]
    give(host, payload(rows, "analysing", "Analysing: 0 of 1 track."))
    n = host.evaluate("() => HST.writes")
    for g in range(2, 6):
        give(host, payload(rows, "analysing", "Analysing: 0 of 1 track.", gen=g))
    assert host.evaluate("() => HST.writes") > n + 3, "with the fault the line is written every time (the check above would fail)"


def test_an_older_answer_is_dropped_and_an_error_keeps_the_list_and_shows_one_line(host):
    rows = [row("q1", "Kick In", "analysing", "Kick In: analysing", [K])]
    give(host, payload(rows, "analysing", "Analysing: 0 of 1 track.", gen=5))
    give(host, payload(rows, "reading", "Reading the audio: 0 of 1 track.", gen=3))
    assert line(host)["text"] == "Analysing: 0 of 1 track."
    host.evaluate("() => HST.take({ error: 'The plugin could not read the state of the tracks. Close and open the window again.' })")
    assert line(host)["text"].startswith("The plugin could not read the state") and line(host)["state"] == "error"
    assert strips(host)[0]["text"] == "Kick In: analysing", "the last good list stays"
    give(host, payload(rows, "analysing", "Analysing: 0 of 1 track.", gen=6))
    assert line(host)["state"] == "analysing"


def test_a_finished_document_goes_quiet_after_a_while_and_keeps_its_state(host):
    rows = [row("q1", "Kick In", "done", "Kick In: 9 hits", [K], hits=9)]
    give(host, payload(rows, "done", "The track is ready: 9 hits."))
    assert line(host)["quiet"] is False
    host.evaluate("() => { HST.doneAt = performance.now() - 9000; HST.paint(); }")
    assert line(host)["quiet"] is True and line(host)["text"] == "The track is ready: 9 hits."


def test_a_strip_survives_a_rebuild_of_the_rows(host):
    rows = [row("q1", "Kick In", "failed", "Kick In: the analysis did not finish. Press Retry.", [K], retry=True)]
    give(host, payload(rows, "attention", "1 of 1 track needs attention.", attention=1))
    host.evaluate("() => { buildTrackDOM(); }")
    host.wait_for_function("() => !FR.raf && FR.flags === 0")
    assert [s["row"] for s in strips(host)] == ["q1"]


def test_a_host_that_does_not_answer_the_command_keeps_the_old_texts_and_shows_no_line(host_app, make_project):
    host_open(host_app, make_project(tracks=(), seconds=1.0))
    host_app.evaluate("""() => { B.hostStates = async () => { throw new Error('unknown command'); }; HST.ready = false; HST.st = null; HST.absent = false; HST.init(); }""")
    host_app.wait_for_function("() => HST.absent === true")
    assert host_app.evaluate("() => document.getElementById('hstLine').classList.contains('hstQuiet')") is True
    assert "put the Drum Spike plugin on a drum track" in host_app.evaluate("() => { const d = document.createElement('div'); renderHostEmpty(d); return d.textContent; }")
    host_app.evaluate("() => { HST.take({ gen: 1, doc_id: 'd', doc: { state: 'nocip', text: 'No audio yet.', n: 0, m: 0, hits: 0, attention: 0 }, rows: [] }); }")
    assert host_app.evaluate("() => [HST.absent, document.getElementById('hstLine').classList.contains('hstQuiet')]") == [False, False], "a later answer brings the line back"


def test_one_error_does_not_end_the_poll_and_a_later_answer_brings_the_list_back(host):
    rows = [row("q1", "Kick In", "analysing", "Kick In: analysing", [K])]
    give(host, payload(rows, "analysing", "Analysing: 0 of 1 track."))
    host.evaluate("() => HST.take({ error: 'The plugin could not read the state of the tracks.' })")
    assert host.evaluate("() => HST.poll !== null"), "S2: the poll goes on after an error answer"
    host.evaluate("""() => { clearTimeout(HST.poll); HST.poll = null; B.hostStates = async () => { throw new Error('one rejected answer'); }; return HST.pull(); }""")
    assert host.evaluate("() => HST.poll !== null"), "S2: and after a rejected call"
    host.evaluate("""p => { clearTimeout(HST.poll); B.hostStates = async () => p; return HST.pull(); }""", payload(rows, "analysing", "Analysing: 1 of 1 track.", gen=9))
    host.wait_for_function("() => document.querySelector('#hstLine .hstText').textContent === 'Analysing: 1 of 1 track.'")
    assert host.evaluate("() => HST.error") == ""


def test_the_poll_slows_down_only_when_nothing_is_in_progress_for_a_while(host):
    failed = [row("q1", "Kick In", "failed", "Kick In: the analysis did not finish. Press Retry.", [K], retry=True)]
    give(host, payload(failed, "attention", "1 of 1 track needs attention.", attention=1))
    delay = """() => { let d = 0; const real = window.setTimeout; window.setTimeout = (f, ms) => { d = ms; return real(f, 100000); }; HST.schedule(); window.setTimeout = real; return d; }"""
    assert host.evaluate(delay) == 1000
    host.evaluate("() => { HST.since = performance.now() - 31000; }")
    assert host.evaluate(delay) == 5000, "N1: offline or failed for more than 30 s: 5 s"
    busy = [row("q1", "Kick In", "reading", "Kick In: reading the audio", [K])]
    give(host, payload(busy, "reading", "Reading the audio: 0 of 1 track.", gen=5))
    host.evaluate("() => { HST.since = performance.now() - 99000; }")
    assert host.evaluate(delay) == 1000, "something in progress is always asked every second"


def test_the_focus_stays_in_the_row_strip_when_the_retry_button_goes(host):
    failed = [row("q1", "Kick In", "failed", "Kick In: the analysis did not finish. Press Retry.", [K], retry=True)]
    give(host, payload(failed, "attention", "1 of 1 track needs attention.", attention=1))
    host.evaluate("() => document.querySelector('#lanesScroll .hostState .hstRetry').focus()")
    assert host.evaluate("() => document.activeElement.className.includes('hstRetry')")
    give(host, payload([row("q1", "Kick In", "analysing", "Kick In: analysing", [K])], "analysing", "Analysing: 0 of 1 track.", gen=2))
    assert host.evaluate("() => document.activeElement !== document.body && document.activeElement.classList.contains('hostState')"), "N2: the focus never falls to the body"


def test_the_standalone_page_has_no_element_and_nothing_focusable_of_it(app, make_project):
    open_project(app, make_project(tracks=FOUR, seconds=6.0))
    assert app.evaluate("() => { HST.init(); HST.take({ gen: 1, doc_id: 'd', doc: { state: 'done', text: 'x' }, rows: [] }); return HST.ready; }") is False
    assert app.evaluate("() => document.querySelectorAll('#hstLine, .hostState, .hostCard, .hstRetry, .hstRetryAll').length") == 0
    assert app.evaluate("() => typeof B.hostStates === 'function'") is True, "the call exists but nothing calls it"
    assert app.evaluate("() => HST.pulls") == 0
