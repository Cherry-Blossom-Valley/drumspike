"""#249 R02+R03: save integrity at the page. A refused event write keeps its payload queued, refuses
the flush and stops the step that needed it (save, export, re-analyze, project switch); the status
line holds Retry and Discard. The save answer's `dirty` flag is the page's truth, an edit during a
paused save keeps the page unsaved, and an answer never moves the dirty mark of another project.
Every test here fails on the code before the fix (R02/R03 in #237)."""
from app_support import open_project, wait_until

REFUSED = "503: the write was refused"


def js(app, expr, arg=None):
    return app.evaluate(expr) if arg is None else app.evaluate(expr, arg)


def refuse_writes(app):   # B.setEvents refuses and counts its calls
    js(app, """() => {
      window._ev = 0;
      B.setEvents = () => { window._ev++; return Promise.reject(new Error("%s")); };
    }""" % REFUSED)


def let_writes_through(app):   # the writes go through; the payload of the last one is kept
    js(app, """() => {
      B.setEvents = (sid, name, body) => { window._ev++; window._last = { sid, name, n: (body.events || []).length };
        return Promise.resolve({ followers: [] }); };
    }""")


def queue_an_edit(app):   # one real page edit (a new hit), held by its 400 ms timer until a flush or the timer sends it
    js(app, """() => {
      const evs = S.events["Kick In"] || (S.events["Kick In"] = []);
      evs.push({ t: 3.75, v: 90, id: newId(), added: true });
      evs.sort((a, b) => a.t - b.t);
      markDirty("Kick In");
    }""")


def state(app):
    return js(app, """() => ({ sid: S.sid, dirty: S.dirty, pending: Object.keys(saveTimers).length,
      msg: document.querySelector("#stMsg").textContent,
      dot: document.querySelector("#dirtyDot").classList.contains("hidden"),
      native: !!(S.sid && S.dirty), title: document.title })""")


def test_a_refused_event_write_stops_the_save_and_keeps_the_edit_queued(app, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    refuse_writes(app)
    js(app, '() => { window._sv = 0; B.projectSave = () => { window._sv++; return Promise.resolve({ saved: true, dirty: false }); }; }')
    queue_an_edit(app)
    err = js(app, 'async () => { try { await saveProjectNow(); } catch (e) { return String(e && e.message || e); } return null; }')
    st = state(app)
    assert js(app, '() => window._sv') == 0, "the save ran although the event write was refused"
    assert st["pending"] == 1, "the refused edit is not queued any more"
    assert st["dirty"] is True and st["dot"] is False, "the page shows the project as saved"
    assert "save failed" in st["msg"] and st["msg"].endswith("the write was refused")
    assert err and "the write was refused" in err, "the save did not refuse"


def test_retry_sends_the_queued_write_again(app, make_project):
    sid = open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    refuse_writes(app)
    queue_an_edit(app)
    js(app, 'async () => { try { await saveProjectNow(); } catch (e) {} }')
    assert js(app, "() => Object.keys(saveTimers).length") == 1
    let_writes_through(app)
    app.click("#stEvRetry")
    wait_until(lambda: js(app, "() => Object.keys(saveTimers).length") == 0, msg="the retry sent the queued write")
    assert js(app, "() => window._ev") >= 2, "the refused write was not sent again"
    last = js(app, "() => window._last")
    assert last["sid"] == sid and last["name"] == "Kick In" and last["n"] > 0, "the retry sent another payload"
    assert js(app, """() => document.querySelector("#stEvFail").classList.contains("hidden")""") is True


def test_discard_drops_the_queued_write_and_never_sends_it(app, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    refuse_writes(app)
    queue_an_edit(app)
    js(app, 'async () => { try { await saveProjectNow(); } catch (e) {} }')
    app.click("#stEvDiscard")
    wait_until(lambda: js(app, "() => Object.keys(saveTimers).length") == 0, msg="the edit is dropped")
    n = js(app, "() => window._ev")
    app.wait_for_timeout(500)   # a re-send would need a new call; none may come
    assert js(app, "() => window._ev") == n, "the discarded write went out"
    st = state(app)
    assert st["pending"] == 0 and "dropped" in st["msg"]
    assert js(app, """() => document.querySelector("#stEvFail").classList.contains("hidden")""") is True


def test_a_refused_event_write_stops_the_export(app, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    js(app, """() => {
      window._ex = 0;
      B.setEvents = () => Promise.reject(new Error("%s"));
      B.exportLastGet = () => Promise.resolve({ export_last: { formats: ["midi"], midi_mode: "per_track", compensate: true } });
      B.exportAll = () => { window._ex++; return Promise.resolve({}); };
      EX.tempoAck = true;   // #151: the project has no tempo map; the notice is answered (test_export_again drives it)
      markDirty("Kick In");
    }""" % REFUSED)
    js(app, 'async () => { await exportAgain(); }')   # exportAgain catches its own failure
    st = state(app)
    assert js(app, "() => window._ex") == 0, "the export ran although the event write was refused"
    assert st["pending"] == 1 and st["dirty"] is True
    assert "export failed" in st["msg"]


def test_a_refused_event_write_stops_the_reanalyze(app, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    js(app, """() => {
      window._an = 0;
      B.setEvents = () => Promise.reject(new Error("%s"));
      B.analyze = () => { window._an++; return Promise.resolve({}); };
      markDirty("Kick In");
    }""" % REFUSED)
    js(app, '() => runAnalysis(["Kick In"], false)')   # the chain catches its own failure
    wait_until(lambda: js(app, "() => analysisRunning") is False, msg="the analysis gave up")
    st = state(app)
    assert js(app, "() => window._an") == 0, "the analyze ran although the event write was refused"
    assert st["pending"] == 1 and st["dirty"] is True
    assert "analyze failed" in st["msg"]


def test_a_refused_event_write_stops_the_project_switch(app, make_project):
    sid_a = open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))   # Song 1
    sid_b = open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))   # Song 2: the page is there now
    js(app, 'async sid => { await switchTo(sid); }', sid_a)   # back to Song 1 (clean: the guard does not ask)
    refuse_writes(app)
    r = js(app, """async sidB => {
      markDirty("Kick In");
      const sw = switchTo(sidB);   // the page is dirty: the guard asks
      await new Promise(res => setTimeout(res, 100));
      const b = [...document.querySelectorAll("#confirmActions button")].find(x => x.dataset.act === "skip");
      if (b) b.click();   // Continue without saving
      return { ok: await sw, msg: document.querySelector("#stMsg").textContent };
    }""", sid_b)
    assert r["ok"] is False, "the switch went on although the event write was refused"
    assert "switch failed" in r["msg"]
    st = state(app)
    assert st["sid"] == sid_a, "the page switched projects"
    assert st["pending"] == 1 and st["dirty"] is True


def test_a_save_answer_with_dirty_true_keeps_the_page_unsaved(app, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    js(app, '() => { B.projectSave = () => Promise.resolve({ saved: true, dirty: true }); }')
    js(app, 'async () => { await saveProjectNow(); }')
    st = state(app)
    assert st["dirty"] is True, "the page cleared the flag the service kept"
    assert st["dot"] is False and st["native"] is True and st["title"].startswith("● "), \
        "the page and the native close/quit gate do not show unsaved"


def test_an_edit_during_a_paused_save_keeps_the_page_unsaved(app, make_project):
    open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))
    js(app, """() => {
      B.setEvents = () => Promise.resolve({ followers: [] });
      window.__dsSaveRes = null;
      B.projectSave = () => new Promise(res => { window.__dsSaveRes = res; });
    }""")
    js(app, '() => { window._sv = saveProjectNow(); }')   # the save hangs on the stub
    queue_an_edit(app)   # the engineer edits while the save is paused
    js(app, '() => window.__dsSaveRes({ saved: true, dirty: false })')   # the service did not see the edit
    js(app, 'async () => { await window._sv; }')
    st = state(app)
    assert st["dirty"] is True, "the save answer cleared the flag of the edit that slipped in"
    assert st["dot"] is False and st["native"] is True and st["title"].startswith("● ")


def test_a_save_answer_after_a_switch_leaves_the_open_project_alone(app, make_project):
    sid_a = open_project(app, make_project(tracks=(("Kick In", "kick", 36),)))   # Song 1
    folder_b = make_project(tracks=(("Kick In", "kick", 36),))                   # Song 2
    sid_b = open_project(app, folder_b)                                          # the page is on Song 2
    js(app, 'async sid => { await switchTo(sid); }', sid_a)                      # back to Song 1
    js(app, """() => {
      B.setEvents = () => Promise.resolve({ followers: [] });
      window.__dsSaveRes = null;
      B.projectSave = () => new Promise(res => { window.__dsSaveRes = res; });
    }""")
    js(app, 'sid => { window._sv = saveOrChoose(sid, "Song 1", "leave"); }', sid_a)   # the save of Song 1 hangs
    open_project(app, folder_b)   # the switch to Song 2 happens before the answer arrives
    queue_an_edit(app)            # Song 2 gets an edit of its own: its mark shows
    js(app, '() => window.__dsSaveRes({ saved: true, dirty: false })')   # Song 1's answer arrives late
    assert js(app, 'async () => { return await window._sv; }') == "saved"
    st = state(app)
    assert st["sid"] == sid_b, "the answer moved the page back"
    assert st["dirty"] is True and st["dot"] is False and st["native"] is True, \
        "Song 1's answer cleared Song 2's dirty mark"
