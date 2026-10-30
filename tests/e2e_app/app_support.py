"""Helpers of the app e2e tests (imported by the test modules)."""
import hashlib
import json
import os
import time
from pathlib import Path

import frozen

ROOT = Path(__file__).resolve().parents[2]

MIDI = frozen.store("midi")
SPIKES = frozen.store("spike")
VELOCITIES = frozen.store("velocities")


def shot_dir(name):
    """#243: the coordinator's screenshots go outside test-results (which holds the Playwright
    traces): <repo>/test-shots, or $DRUMSPIKE_SHOT_DIR. Returns the file path, dir created."""
    d = Path(os.environ.get("DRUMSPIKE_SHOT_DIR") or (ROOT / "test-shots"))
    d.mkdir(parents=True, exist_ok=True)
    return d / name


def file_sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def midi_reference(key, events, note, ppq=24000, tempo=120.0):
    """The bytes of the MIDI file the retired server (engine._py_export_midi, pretty_midi) wrote for these events,
    frozen in tests/golden/frozen/midi.json (P3-12)."""
    return bytes.fromhex(MIDI.ref(f"e2e_app/{key}", {"events": events, "note": note, "ppq": ppq, "tempo": tempo})["hex"])


def spike_reference(key, source, times, fmt="pcm16"):
    """frozen.wav_summary of the spike file drumspike_core.write_spike_wav wrote for this source and these times,
    frozen in tests/golden/frozen/spike.json (P3-12)."""
    return SPIKES.ref(f"e2e_app/{key}", {"source_sha256": file_sha(source), "times": times, "format": fmt})


def velocities_reference(key, source, events, lo_db=-40.0, hi_db=-6.0):
    """The answer of the retired server's route 34 (engine.recompute_velocities after engine.load_f32), frozen in
    tests/golden/frozen/velocities.json (P3-12)."""
    return VELOCITIES.ref(f"e2e_app/{key}", {"source_sha256": file_sha(source), "events": events, "lo_db": lo_db,
                                             "hi_db": hi_db}, canonical_host=True)


def menu(page, id, wait=True, **extra):
    """A click on a native menu item (P3-13a): the bridge puts the event `menu` {id, path?, sid?} into its
    event log (command selftest_menu, as the app's debug command of the same name emits it), and the page's
    own listener (shell.js) runs the item. wait=True: until the item's handler has ended, and returns its
    verdict (True: it ran; False: an unknown id, or a dialog of the page was open). wait=False: until the
    handler has started (a handler that opens a dialog ends only when the dialog is answered)."""
    page.wait_for_function("() => window.DSShell && DSShell.listening()")
    seq = page.evaluate("() => DSShell.log.seen") + 1
    sent = invoke(page, "selftest_menu", id=id, **extra)
    assert sent.get("id") == id, sent
    page.wait_for_function("n => DSShell.log.seen >= n", arg=seq)
    if not wait:
        return None
    page.wait_for_function("n => n in DSShell.log.results", arg=seq, timeout=60000)
    return page.evaluate("n => DSShell.log.results[n]", seq)


def save_to(page, path):
    """P4-7d: the next native save dialog (Render Layer, Calibration MIDI...) answers `path`; its folder is made."""
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    page.evaluate("p => { window.__DS_SAVE__ = p; }", str(path))


def render_layer(page, pdir, track="Kick In"):
    """P4-7d: Render Layer with the save dialog answered by the track's file in Exports/."""
    save_to(page, Path(pdir) / "Exports" / f"{track}_layer.wav")
    page.locator("#inspector .lyRender").click()


def open_project(page, folder):
    """File > Open with the native dialog answered by window.__DS_PICK__; waits until the lanes of THAT
    folder are loaded (the page may have opened an earlier project of the bridge at start-up)."""
    want = os.path.realpath(folder)
    # P4-55: the previous project can be genuinely unsaved; guardDirty then shows its real
    # "unsaved changes" prompt inside the open and nothing answers it. The page does NOT fake
    # the clean state: the helper asks the service (project_clean_check, the truth after the
    # pending writes settle) and, on unsaved, answers the explicit "Continue without saving"
    # button (no disk write) once the modal is up; then the open goes through as below.
    # A project the test closed can still be S.sid for a moment: its 404 counts as clean.
    dirty = page.evaluate("""async () => { if (!S.sid) return false; try {
        return (await B.projectCleanCheck(S.sid)).dirty; } catch (e) { return false; } }""")
    page.evaluate("f => { window.__DS_PICK__ = f; }", str(folder))
    menu(page, "open", wait=False)
    if dirty:
        end = time.time() + 10
        while time.time() < end:
            clicked = page.evaluate("""() => { const b = [...document.querySelectorAll('#confirmActions button')]
                .find(b => b.dataset.act === 'skip'); if (b) { b.click(); return true; } return false; }""")
            if clicked:
                break
            # no modal: the page saw the project clean (the late write was not yet its truth),
            # so guardDirty passed on its own; the open is already on its way
            got = page.evaluate("async () => S.sid ? (await B.projectGet(S.sid).catch(() => null))?.folder ?? null : null")
            if got and os.path.realpath(got) == want:
                break
            time.sleep(0.1)
    # P4-42: 60 s under load (a loaded host starts the project late; the wait is the ready condition, not a sleep)
    end = time.time() + 60
    while time.time() < end:
        # a project the test closed in the service (project_close) can still be S.sid for a moment: its
        # 404 means "not open yet", as null does (CI 38414427460 and 38377478514, x86-mac)
        got = page.evaluate("async () => S.sid ? (await B.projectGet(S.sid).catch(() => null))?.folder ?? null : null")
        if got and os.path.realpath(got) == want:
            break
        time.sleep(0.1)
    else:
        diag = page.evaluate("() => ({sid: S.sid, seen: DSShell.log.seen, results: DSShell.log.results, "
                             "modal: [...document.querySelectorAll('.modal:not(.hidden)')].map(m => m.id), msg: document.querySelector('#stMsg')?.textContent})")
        raise AssertionError(f"the page did not open {want} in 60 s: {diag}")
    page.wait_for_function("() => S.loaded > 0 && S.order.length > 0 && S.order.every(n => n in S.peaks)")
    return page.evaluate("() => S.sid")


def invoke(page, cmd, **args):
    """Call a command through the bridge from the page (as B does), returns the JSON answer."""
    return page.evaluate("async ([c, a]) => (await fetch('/invoke/' + c, {method: 'POST', "
                         "headers: {'Content-Type': 'application/json'}, body: JSON.stringify(a)})).json()", [cmd, args])


# P4-67b: the pause of wait_until between two checks. The Playwright sync API delivers page events (request,
# console, dialog) to their Python listeners only inside a Playwright call, never in a time.sleep: a condition
# on a list that such a listener fills (app.invokes) saw it frozen for the whole timeout (CI 39073137512).
# The app fixture sets PUMP to a short page.wait_for_timeout while its page lives; None: time.sleep.
PUMP = None


def wait_until(fn, timeout=10.0, msg="condition"):
    """Poll fn() until it is truthy; returns its value, or fails with msg. Between the checks it runs PUMP
    (the page delivers its pending events), else it sleeps."""
    end = time.time() + timeout
    while time.time() < end:
        v = fn()
        if v:
            return v
        if PUMP is not None:
            PUMP()
        else:
            time.sleep(0.1)
    raise AssertionError(f"timeout waiting for {msg}")


# ---------- project folders (format 1, docs/migration/project-format.md) without the retired server's layer ----------

class Project:
    """A project folder read as plain JSON: .dir, .path (the document), .doc, .name."""

    def __init__(self, folder, path):
        self.dir, self.path = str(Path(folder).resolve()), str(path)
        self.doc = json.loads(Path(path).read_text())

    @property
    def name(self):
        return self.doc.get("name") or Path(self.dir).name


def document_path(folder):
    """The folder's own document (<folder>/<folder name>.drumspike), else its only .drumspike file."""
    folder = Path(folder)
    own = folder / f"{folder.resolve().name}.drumspike"
    if own.is_file():
        return own
    docs = sorted(folder.glob("*.drumspike"))
    if len(docs) != 1:
        raise FileNotFoundError(f"no .drumspike document in {folder}")
    return docs[0]


def load_project(folder):
    return Project(folder, document_path(folder))


def head_sha1(path):
    with open(path, "rb") as f:
        return hashlib.sha1(f.read(1 << 20)).hexdigest()


def resolve_source(record, project_dir):
    """(path, "ok" | "changed" | "missing"): the absolute path, the path relative to the project, then
    Audio Files/<file name>; a candidate is "ok" when its size and the SHA-1 of its first MiB match the record."""
    if not record:
        return None, "missing"
    cands = []
    if record.get("path"):
        cands.append(record["path"])
    if record.get("relative"):
        cands.append(os.path.normpath(os.path.join(project_dir, record["relative"])))
    base = os.path.basename(record.get("path") or record.get("relative") or "")
    if base:
        cands.append(os.path.join(project_dir, "Audio Files", base))
    existing = []
    for c in dict.fromkeys(cands):
        if os.path.isfile(c):
            if os.path.getsize(c) == record.get("size") and head_sha1(c) == record.get("head_sha1"):
                return c, "ok"
            existing.append(c)
    return (existing[0], "changed") if existing else (None, "missing")


def track_menu(page, name, act):
    """P3-14e: a right-click on the header of the track and a click on one item of the context menu
    (act: export, reanalyze, mix; Params went into the Inspector with P3-14a). P4-30: act midi or spikes
    exports that format of the track through the Export dialog (the context menu has the one item Export...)."""
    if act in ("midi", "spikes"):
        return export_dialog(page, formats=(act,), scope="selected", track=name)
    page.locator(f'.track[data-name="{name}"] .thead').click(button="right")
    page.locator(f'#ctxMenu [data-act="{act}"]').click()


def open_export(page, scope="all", track=None, via="button"):
    """P4-30: open the Export dialog: the toolbar button (via=button), File > Export... (menu), Track > Export Selected
    Tracks... (selected, with the track selected first) or the context menu of the track header (scope=selected, via=header)."""
    if scope == "selected" and via == "menu":
        page.evaluate("n => setSelTrack(n)", track)
        menu(page, "export_selected")
    elif scope == "selected":
        page.locator(f'.track[data-name="{track}"] .thead').click(button="right")
        page.locator('#ctxMenu [data-act="export"]').click()
    elif via == "menu":
        menu(page, "export")
    else:
        page.click("#exportAllBtn")
    page.wait_for_function("() => !document.querySelector('#exportModal').classList.contains('hidden')")


def ex_confirm_tempo(page):
    """#151: the default-tempo notice: the first MIDI export on a project without a tempo map asks
    (Export anyway / Cancel). The helper answers Export anyway; a test that wants the notice itself
    drives the confirm on its own."""
    btn = page.locator('#confirmActions [data-act="anyway"]')
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:   # the notice asks after tempo_map_get answers: either it shows or the export starts
        if btn.is_visible():
            btn.click()
            return
        if page.evaluate("() => EX.ran"):
            return   # no notice: the map is set, MIDI is off or the session already answered
        page.wait_for_timeout(50)


def ex_run(page, timeout=60000):
    """Click Export and wait for the export-finished event (#172). Race: a click's handler runs a task later, so a
    bare wait for !EX.busy passed before exRun had set the flag and the dialog closed mid-export (the files were
    checked before the service had written them). Now the test waits for EX.done, the count of the exports the page
    has finished (static/app.js)."""
    done0 = page.evaluate("() => EX.done")
    page.click("#exGo")
    ex_confirm_tempo(page)   # #151: the default-tempo notice, when it asks
    page.wait_for_function("d0 => !EX.busy && EX.done > d0", arg=done0, timeout=timeout)


def export_dialog(page, formats=("midi", "spikes"), scope="all", track=None, mode=None, via=None, go=True, close=True):
    """P4-30: open the Export dialog, set the formats (and the MIDI mode: per_track or single), click Export, wait until it
    has run and close the dialog. go=False leaves the dialog open and unrun."""
    open_export(page, scope, track, via or ("header" if scope == "selected" else "button"))
    page.locator("#exMidi").set_checked("midi" in formats)
    page.locator("#exSpikes").set_checked("spikes" in formats)
    if mode == "single":
        page.locator("#exModeSingle").check()
    elif mode == "per_track":
        page.locator("#exModeTrack").check()
    if not go:
        return
    ex_run(page)
    if close:
        page.click("#exCancel")
        page.wait_for_function("() => document.querySelector('#exportModal').classList.contains('hidden')")


def inspect_track(page, name):
    """P3-14a: shows the Inspector and selects the track (a click on its header, on the name line, as a user
    does); returns the Inspector locator once it shows that track."""
    page.evaluate("() => { if (!S.inspector) setInspector(true); }")
    if page.evaluate("n => S.selTrack !== n", name):
        page.locator(f'.track[data-name="{name}"] .thead .tnote').click()   # the role line: never a button (P3-14c: the offline badge follows a short name)
    page.wait_for_function("n => S.selTrack === n && document.querySelector('#insName').value === n", arg=name)
    # P4-25: the controls of a closed section are hidden; tests that drive them see every section open (the stored state stays)
    page.evaluate("() => { for (const k of INS_SEC_ORDER) INSSEC.open[k] = true; insSecApply(); }")
    return page.locator("#inspector")


def edits_done(page):
    """P4-9 stage 2: the hit edits, shifts and undo steps on their way to the service are done (the page's SH queue is empty)."""
    page.wait_for_function("() => SH.pending === 0")
