"""#589: the export dialog in a DAW makes one file per DAW track (DAW plugin M1c4).

The plugin gives the states of the prepared files of every DAW track (the payload of plugin_deliver_state, static/deliver.js). In a DAW the dialog lists those DAW
tracks, shows them on its tiles, and Export asks the plugin to copy the prepared files (plugin_deliver_export). These tests send the payload the plugin sends and
stand in for the three commands; the plugin side has its own stage (plugins/ara/check.py, export-rows). The last test is the standalone twin: with the mode off the
dialog lists audio tracks and calls the old commands."""
import pytest

from test_host_page import FOUR, host_open
from test_host_song_time import TRACKS, rows_payload
from test_host_deliver import S_READY, S_PREP, S_OFF, S_NONE, payload, err

R = {"midi": S_READY, "spike": S_READY, "audio": S_READY}


@pytest.fixture
def host(host_app, make_project):
    host_open(host_app, make_project(tracks=TRACKS, seconds=6.0))
    host_app.evaluate("p => { HT.take(p); }", rows_payload())
    host_app.evaluate("""() => {
      window.__calls = [];
      B.deliverTake = async items => { window.__calls.push(['take', items]); return { files: items.map((it, i) => '/x/' + it.row + '_' + it.kind + i) }; };
      B.deliverWant = async kind => { window.__calls.push(['want', kind]); return null; };
      B.deliverPlan = async (items, dest) => { window.__calls.push(['plan', items, dest]); return { files: [], replace: 2, dest }; };
      B.deliverProgress = async () => ({ busy: false, done: 0, total: 0, track: '' });
      B.deliverExport = async (items, dest, start) => { window.__calls.push(['export', items, dest, start]); return { files: [], dest, midi: items.filter(i => i.kind === 'midi').length,
        spikes: items.filter(i => i.kind === 'spike').length, audio: items.filter(i => i.kind === 'audio').length, merged: items.filter(i => i.kind === 'merged').length,
        replaced: 0, outside_song: { no_region: 1, before_start: 2 } }; };
      DLV.init(); }""")
    yield host_app
    host_app.evaluate("() => { if (EX.open) { EX.busy = false; exClose(); } DLV.st = null; DLV.paint(); HT.rows = []; HT.gen = 0; HT.rebuild(); HT.paint(); }")


def give(page, p):
    page.evaluate("p => DLV.take(p)", p)


def open_dialog(page, scope="all", track=None):
    page.evaluate("([s, t]) => openExport(s, t)", [scope, track])
    page.wait_for_function("() => !document.querySelector('#exportModal').classList.contains('hidden')")


def calls(page, kind):
    return page.evaluate("k => window.__calls.filter(c => c[0] === k)", kind)


def test_the_dialog_lists_daw_tracks_and_exports_one_item_per_track_and_kind(host):
    give(host, payload(spike=S_READY))
    open_dialog(host)
    assert host.locator("#exAllText").inner_text() == "All tracks (3)"
    host.wait_for_function("() => document.querySelector('#exDragMidi').dataset.state === 'ready'")
    assert host.evaluate("() => [document.querySelector('#exDragMidi .exDragN').textContent, document.querySelector('#exDragSpikes .exDragN').textContent]") == ["3 files", "3 files"]
    assert "source" not in host.evaluate("() => document.querySelector('#exportModal').innerText").lower()
    host.wait_for_function("() => window.__calls.some(c => c[0] === 'plan') && document.querySelector('#exReplace').textContent === '2 files will be replaced'")
    want = [{"row": r, "kind": k} for k in ("midi", "spike") for r in ("q1", "q2", "s3")]
    assert calls(host, "plan")[-1][1] == want
    assert host.locator("#exReplace").inner_text() == "2 files will be replaced"
    host.evaluate("() => { EX.tempoAck = true; }")
    host.locator("#exGo").click()
    host.wait_for_function("() => EX.done > 0 && !EX.busy")
    items = calls(host, "export")[-1][1]
    assert items == want
    msg = host.locator("#stMsg").inner_text()
    assert "3 MIDI and 3 spike files" in msg and "1 in no playback region, 2 before the song start" in msg, msg


def test_a_press_of_export_inside_the_150_ms_runs_the_waiting_plan_call_first_and_shows_its_count(host):
    """#597: the plan call of an option change waits 150 ms; an Export inside that time used to run with the OLD replace count on the screen."""
    give(host, payload(spike=S_READY, audio=S_READY, used={"spike": True, "audio": True}))
    open_dialog(host)
    host.wait_for_function("() => window.__calls.some(c => c[0] === 'plan') && document.querySelector('#exReplace').textContent === '2 files will be replaced'")
    host.evaluate("""() => { EX.tempoAck = true; window.__calls = [];
      B.deliverPlan = async (items, dest) => { window.__calls.push(['plan', items.length]); await new Promise(r => setTimeout(r, 50)); window.__calls.push(['planned']); return { files: [], replace: 7, dest }; };   // a real delay: the export must wait for the answer
      const o = B.deliverExport;
      B.deliverExport = async (items, dest, start) => { window.__calls.push(['press', document.querySelector('#exReplace').textContent]); return o(items, dest, start); };
      document.querySelector('#exSpikeBoth').click();   // an option change: its plan call is waiting
      document.querySelector('#exGo').click(); }""")   # Export in the same task: inside the 150 ms
    host.wait_for_function("() => EX.done > 0 && !EX.busy")
    seen = host.evaluate("() => window.__calls.map(c => c[0])")
    assert seen == ["plan", "planned", "press", "export"], f"the waiting plan call was answered before Export ran: {seen}"
    assert calls(host, "press")[0][1] == "7 files will be replaced", "the count on the screen when the files are written is the new one"
    # the dialog in the quiet state: no plan call is pending, Export does not call the plan again
    host.wait_for_function("() => window.__calls.some(c => c[0] === 'plan')")   # the plan call that the finished export asked for has run: nothing is pending
    host.wait_for_timeout(300)
    host.evaluate("() => { window.__calls = []; EX.done = 0; }")
    host.locator("#exGo").click()
    host.wait_for_function("() => EX.done > 0 && !EX.busy")
    assert host.evaluate("() => window.__calls.map(c => c[0])") == ["press", "export"]


def test_a_press_of_export_while_the_plan_call_is_in_flight_waits_for_it_and_shows_its_count(host):
    """#599: the 150 ms timer already fired and the plan call is on its way (it answers after 400 ms); Export used to run at once with the OLD count on the screen."""
    give(host, payload(spike=S_READY, audio=S_READY, used={"spike": True, "audio": True}))
    open_dialog(host)
    host.wait_for_function("() => window.__calls.some(c => c[0] === 'plan') && document.querySelector('#exReplace').textContent === '2 files will be replaced'")
    host.evaluate("""() => { EX.tempoAck = true; window.__calls = [];
      B.deliverPlan = async (items, dest) => { window.__calls.push(['plan', items.length]); await new Promise(r => setTimeout(r, 400)); window.__calls.push(['planned']); return { files: [], replace: 7, dest }; };   // a real delay
      const o = B.deliverExport;
      B.deliverExport = async (items, dest, start) => { window.__calls.push(['press', document.querySelector('#exReplace').textContent]); return o(items, dest, start); };
      document.querySelector('#exSpikeBoth').click(); }""")   # an option change: its plan call starts after 150 ms
    host.wait_for_function("() => window.__calls.some(c => c[0] === 'plan')")   # the timer fired: the call is in flight and the answer is 400 ms away
    assert host.evaluate("() => EX.infoFlush") is None, "the timer fired: no flush is waiting any more, the call is in flight"
    assert host.evaluate("() => window.__calls.map(c => c[0])") == ["plan"]
    host.evaluate("() => { document.querySelector('#exGo').click(); }")
    host.wait_for_function("() => EX.done > 0 && !EX.busy")
    seen = host.evaluate("() => window.__calls.map(c => c[0])")
    assert seen == ["plan", "planned", "press", "export"], f"Export waited for the plan call that was in flight: {seen}"
    assert calls(host, "press")[0][1] == "7 files will be replaced", "the count on the screen when the files are written is the new one"


def test_the_spike_types_the_single_file_and_the_selected_track(host):
    give(host, payload(spike=S_READY, audio=S_READY, used={"spike": True, "audio": True}))
    open_dialog(host)
    host.locator("#exSpikeBoth").check()
    assert len(host.evaluate("() => exHostItems('all')")) == 9
    host.locator("#exSpikeAudio").check()
    assert {i["kind"] for i in host.evaluate("() => exHostItems('all')")} == {"midi", "audio"}
    host.locator("#exModeSingle").check()
    assert host.evaluate("() => exHostItems('midi')") == [{"row": "", "kind": "merged"}]
    label = host.locator("#exModeSingle").locator("xpath=..").inner_text()
    assert "One MIDI track with all drums" in label and "multi-track" not in label.lower()
    host.evaluate("f => { EX.selected = f; }", FOUR[0][0])
    host.locator("#exScopeSel").check()
    assert host.locator("#exModeSingle").is_disabled()
    assert "needs All tracks" in host.locator("#exModeSingle").locator("xpath=..").get_attribute("title")
    assert all(i["kind"] != "merged" for i in host.evaluate("() => exHostItems('all')"))
    assert len([i for i in host.evaluate("() => exHostItems('all')") if i["kind"] == "midi"]) == 1


def test_a_set_that_is_not_ready_makes_the_dialog_wait_and_never_write_an_old_file(host):
    give(host, payload(midi=S_PREP))
    open_dialog(host)
    host.evaluate("""() => { EX.tempoAck = true; window.__n = 0; const o = B.deliverExport;
      B.deliverExport = async (items, dest, start) => { window.__n++; window.__starts = (window.__starts || []).concat([start]); if (window.__n < 3) { const e = new Error('409: Drum A: preparing'); e.status = 409; throw e; } return o(items, dest, start); }; }""")
    host.locator("#exGo").click()
    host.wait_for_function("() => EX.done > 0 && !EX.busy", timeout=20000)
    assert host.evaluate("() => window.__n") == 3
    starts = host.evaluate("() => window.__starts")
    assert starts[0] is True and all(x is False for x in starts[1:]), starts


def test_on_the_limit_the_dialog_names_the_daw_track_and_writes_nothing_and_an_error_stops_it(host):
    give(host, payload(midi=S_PREP))
    open_dialog(host)
    host.evaluate("""() => { EX.tempoAck = true; EX.waitMs = 700;
      B.deliverExport = async () => { const e = new Error('409: Drum A: preparing'); e.status = 409; throw e; }; }""")
    host.locator("#exGo").click()
    host.wait_for_function("() => EX.done > 0 && !EX.busy", timeout=20000)
    msg = host.locator("#stMsg").inner_text()
    assert "Drum A is not ready after" in msg and "nothing was written" in msg, msg
    host.evaluate("""() => { EX.done = 0; B.deliverExport = async () => { window.__n2 = (window.__n2 || 0) + 1; const e = new Error('409: Drum A: error: no answer from the preparation (trying again)'); e.status = 409; throw e; }; }""")
    host.locator("#exGo").click()
    host.wait_for_function("() => EX.done > 0 && !EX.busy", timeout=20000)
    assert host.evaluate("() => window.__n2") == 1, "an error is not waited for"
    assert "no answer from the preparation" in host.locator("#stMsg").inner_text()


def test_cancel_while_the_export_runs_asks_the_plugin_to_stop(host):
    give(host, payload())
    open_dialog(host)
    host.evaluate("""() => { EX.tempoAck = true; window.__cancel = 0;
      B.deliverProgress = async c => { if (c) window.__cancel++; return { busy: true, done: 1, total: 3, track: 'Drum A' }; };
      B.deliverExport = () => new Promise((ok, no) => { window.__rej = no; }); }""")
    host.locator("#exGo").click()
    host.wait_for_function("() => EX.busy")
    host.locator("#exCancel").click()
    assert host.evaluate("() => window.__cancel") == 1 and host.evaluate("() => EX.busy") is True
    host.evaluate("() => { const e = new Error('409: cancelled: nothing was written'); e.status = 409; window.__rej(e); }")
    host.wait_for_function("() => !EX.busy")
    assert "cancelled" in host.locator("#stMsg").inner_text()


def test_the_tiles_drag_the_prepared_sets_and_say_why_when_a_set_failed(host):
    give(host, payload(spike=err("disk full")))
    open_dialog(host)
    host.wait_for_function("() => document.querySelector('#exDragMidi').dataset.state === 'ready'")
    assert host.evaluate("() => document.querySelector('#exDragSpikes').dataset.state") == "error"
    assert "disk full" in host.locator("#exDragSpikes .exDragN").inner_text()
    host.evaluate("() => exClose()")
    give(host, payload(spike=S_OFF))
    open_dialog(host)
    host.wait_for_function("() => window.__calls.some(c => c[0] === 'want' && c[1] === 'spike')")
    give(host, payload(spike=S_READY))
    host.wait_for_function("() => document.querySelector('#exDragSpikes').dataset.state === 'ready'")
    box = host.locator("#exDragMidi").bounding_box()
    host.mouse.move(box["x"] + 5, box["y"] + 5)
    host.mouse.down()
    host.mouse.move(box["x"] + 60, box["y"] + 20, steps=4)
    host.mouse.up()
    host.wait_for_function("() => window.__calls.some(c => c[0] === 'take')")
    assert calls(host, "take")[-1][1] == [{"row": r, "kind": "midi"} for r in ("q1", "q2", "s3")]
    assert all(c != "export_drag_prepare" for c in host.invokes)


def test_a_row_with_no_hits_is_skipped_and_named(host):
    give(host, payload(midi=S_READY))
    p = payload()
    p["rows"][1]["midi"] = S_NONE
    give(host, p)
    open_dialog(host)
    assert "Toms (no hits)" in host.locator("#exSkipped").inner_text()
    assert [i["row"] for i in host.evaluate("() => exHostItems('midi')")] == ["q1", "s3"]


def test_the_standalone_dialog_lists_audio_tracks_and_calls_the_old_commands(app, make_project):
    from test_host_page import analyzed
    analyzed(app, make_project(tracks=FOUR, seconds=6.0))
    app.evaluate("() => openExport('all')")
    app.wait_for_function("() => !document.querySelector('#exportModal').classList.contains('hidden')")
    assert app.locator("#exAllText").inner_text() == "All tracks (4)"
    app.wait_for_function("() => document.querySelector('#exDragMidi').dataset.state === 'ready'", timeout=60000)
    assert app.evaluate("() => exTrackSets().names") == ["Kick In", "Snare Top", "Tom 1", "Overhead L"]
    assert "multi-track" in app.locator("#exModeSingle").locator("xpath=..").inner_text()
    assert "export_drag_prepare" in app.invokes and "export_existing" in app.invokes
    assert not any(c.startswith("plugin_deliver") for c in app.invokes)
    app.evaluate("() => { EX.tempoAck = true; }")
    app.locator("#exGo").click()
    app.wait_for_function("() => EX.done > 0 && !EX.busy", timeout=60000)
    assert "export_all" in app.invokes
    assert not any(c.startswith("plugin_deliver") for c in app.invokes)
    app.evaluate("() => exClose()")


def test_measure_the_page_cost_of_forty_rows_with_the_dialog_open(host):
    """#589 measure (page side): 40 DAW tracks, the dialog open: the time of one payload of the plugin (`DLV.take`, which repaints the handles and the dialog)
    and the size of the payload. Printed with `-s`; the bound is wide (the numbers are in the report)."""
    r = host.evaluate("""async () => {
      const R = { state: 'ready', n: 1, why: '' };
      const mk = g => ({ gen: g, merged: R, used: { spike: true, audio: true }, hint: '', rows: Array.from({ length: 40 }, (_, i) => ({ id: 'r' + i, name: 'Track ' + i, srcs: ['s' + i], midi: R, spike: R, audio: R })) });
      DLV.take(mk(1)); await openExport('all'); if (!EX.open) return { ms: -1, bytes: 0 };
      const t = []; for (let g = 2; g < 22; g++) { const p = mk(g); p.rows[g % 40].midi = { state: 'preparing', n: 0, why: '' }; const t0 = performance.now(); DLV.take(p); t.push(performance.now() - t0); }
      return { ms: t.reduce((a, b) => a + b, 0) / t.length, bytes: JSON.stringify(mk(1)).length }; }""")
    print(f"MEASURE page, 40 rows, dialog open: one payload {r['ms']:.2f} ms to take and repaint, {r['bytes']} bytes")
    assert 0 <= r["ms"] < 250


HINT = "Drop at the start of the session, not at the clip"


def test_the_hint_shows_before_the_drag_starts_and_after_it_says_what_to_do_when_notes_are_late(host):
    """R12 (#589): a MIDI file dropped at the clip start puts every note late. The message is set BEFORE the system drag is asked for, and after the drop
    it says what to do; the toolbar hint is a visible note (the accent colour) and the dialog shows the same sentence next to its tiles."""
    give(host, payload())
    host.evaluate("""() => { window.__order = []; B.dragFiles = async files => { window.__order.push(['drag', document.querySelector('#stMsg').textContent]); return true; }; }""")
    host.evaluate("() => DLV.drag([{ row: 'q1', kind: 'midi' }, { row: 'q2', kind: 'midi' }])")
    host.wait_for_function("() => window.__order.length === 1 && /^dragged/.test(document.querySelector('#stMsg').textContent)")
    during = host.evaluate("() => window.__order[0][1]")
    assert during == "Dragging 2 items: drop at the start of the session, not at the clip", during
    assert host.locator("#stMsg").inner_text() == "dragged 2 items. If the notes are late, move the clip to the start of the session"
    assert host.inner_text("#dlvHint") == HINT
    assert host.evaluate("() => getComputedStyle(document.querySelector('#dlvHint')).color") == "rgb(77, 163, 255)"
    open_dialog(host)
    host.wait_for_function("() => document.querySelector('#exDragMidi').dataset.state === 'ready'")
    assert host.inner_text("#exDragHint") == HINT and host.locator("#exDragHint").is_visible()
    host.evaluate("() => { window.__order = []; }")
    box = host.locator("#exDragMidi").bounding_box()
    host.mouse.move(box["x"] + 5, box["y"] + 5)
    host.mouse.down()
    host.mouse.move(box["x"] + 60, box["y"] + 20, steps=4)
    host.mouse.up()
    host.wait_for_function("() => window.__order.length === 1")
    assert host.evaluate("() => window.__order[0][1]") == "Dragging 3 files: drop at the start of the session, not at the clip"


def test_the_standalone_page_has_no_drop_hint(app, make_project):
    from test_host_page import analyzed
    analyzed(app, make_project(tracks=FOUR, seconds=6.0))
    app.evaluate("() => openExport('all')")
    app.wait_for_function("() => !document.querySelector('#exportModal').classList.contains('hidden')")
    assert app.evaluate("() => [document.getElementById('dlvHint'), document.getElementById('exDragHint')]") == [None, None]
    app.evaluate("() => exClose()")


def test_a_burst_of_changes_makes_one_plan_call_in_host_mode(host):
    """#589 round 2 (S4): the replace count is asked once for a burst of changes (150 ms, the newest wins)."""
    give(host, payload(spike=S_READY))
    open_dialog(host)
    host.wait_for_function("() => window.__calls.some(c => c[0] === 'plan')")
    host.wait_for_timeout(400)
    host.evaluate("() => { window.__calls = []; exInfo(); exInfo(); exInfo(); EX.midi = false; exChanged(); EX.midi = true; exChanged(); }")
    host.wait_for_timeout(600)
    assert len(calls(host, "plan")) == 1, calls(host, "plan")


def test_a_cancel_during_the_wait_says_cancelled_and_a_partial_write_is_told_in_plain_words(host):
    give(host, payload(midi=S_PREP))
    open_dialog(host)
    host.evaluate("""() => { EX.tempoAck = true; EX.waitMs = 20000;
      B.deliverExport = async () => { const e = new Error('409: Drum A: preparing'); e.status = 409; throw e; }; }""")
    host.locator("#exGo").click()
    host.wait_for_function("() => EX.busy")
    host.locator("#exCancel").click()
    host.wait_for_function("() => !EX.busy", timeout=10000)
    assert host.locator("#stMsg").inner_text() == "export cancelled: nothing was written"
    host.evaluate("""() => { EX.done = 0; B.deliverExport = async () => { const e = new Error('500: 2 of 5 files were written: a.mid, b.mid. Not written: C, D, E. Press Export again. (C: could not write the file: x)'); e.status = 500; throw e; }; }""")
    host.locator("#exGo").click()
    host.wait_for_function("() => EX.done > 0 && !EX.busy")
    msg = host.locator("#stMsg").inner_text()
    assert msg.startswith("export failed: 2 of 5 files were written: a.mid, b.mid. Not written: C, D, E. Press Export again."), msg
