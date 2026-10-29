/* Drum Spike v2 — session (R17 step 6, #55 per docs/review/R17-plan.md)
   Project open/switch/close: _gen and loadProject (with loadPeaks and prepareLanes),
   initSessions, renderRecentList/clearRecentProjects, useProject, guardDirty, switchTo,
   openProjectPath and closeProject. Plain script (no build step), loaded after render.js
   and before backend.js. The functions stay top-level bindings: the e2e tests, shell.js
   and the empty page call them as page globals.
   The module state a load resets lives with its owners (GLQ.loadedSeq in lanes-gl.js,
   lnReset/tmLoad/selMem in app.js, auReset in transport.js): it arrives as injected hooks —
   boot wires sessionInit({ GLQ, lnReset, auReset, tmLoad, selMem, setSelMem }); a js_unit
   test wires its own. Nothing here reads the DOM or the backend at load time. */
"use strict";

/* ---------------- the injected hooks ---------------- */
const SESSION_HOOKS = {};
function sessionInit(deps) { if (deps) Object.assign(SESSION_HOOKS, deps); }

/* ---------------- sessions, projects ---------------- */
async function initSessions(keepNone) {   // keepNone: a refresh of the list (dirty event) must not open a project when none is open
  const sid0 = S.sid, list = await B.sessions();
  let gone = false;
  // an open or a close moved S.sid while the list was on its way (a dirty event's refresh runs 100 ms late): the list is
  // older than that change and must not pick the project, else it resets S.sid to null or to another project
  if (S.sid !== sid0) { /* keep */ }
  else if (S.sid && list.some(s => s.sid === S.sid)) { /* keep */ }
  else if (!S.sid && keepNone) { /* nothing is open on purpose: only the list changes */ }
  else if (S.sid && keepNone) { gone = true; }   // another client (an agent command) closed the open project: no silent switch to another one
  else if (list.length) { S.sid = list[0].sid; localStorage.setItem("dsg_sid", S.sid); }
  else gone = true;   // S3-3: no silent 'Untitled project' folder
  if (gone) { S.sid = null; localStorage.removeItem("dsg_sid"); setProjName(""); setDirtyState(false); }
  const cur = list.find(s => s.sid === S.sid);
  if (cur) { setProjName(cur.name); setDirtyState(cur.dirty); }
  if (gone && keepNone && !S.sid) loadProject();   // the lanes of the closed project go; the empty page shows
  renderRecentList();
  DSShell.sync();   // P3-4b: greys the native menu items while no project is open, refreshes Open Recent; P3-13a: the Window menu
}
async function renderRecentList() {   // the Open Recent list of the empty page (no project open); the menu has its own
  let recent = [];
  try { recent = await B.recentProjects(); } catch(e) {}
  const box = $("#esRecentList");
  if (!box) return;
  box.innerHTML = "";
  const clear = $("#esRecentClear");   // #441: the control only makes sense while the list has entries
  if (clear) clear.hidden = !recent.length;
  if (!recent.length) { const e = document.createElement("div"); e.className = "phint"; e.textContent = "no recent projects"; box.appendChild(e); return; }
  for (const r of recent) {
    const row = document.createElement("div");
    row.className = "sbRecent";
    row.title = r.folder;
    const nm = document.createElement("div"); nm.className = "sbProjName"; nm.textContent = r.name;
    const sub = document.createElement("div"); sub.className = "sbProjSub"; sub.textContent = r.folder;
    row.append(nm, sub);
    row.onclick = () => openProjectPath(r.folder);
    box.appendChild(row);
  }
}
/* #441 Clear Recent: empties the stored recent list (recent.json) and the File > Open Recent menu
   (DSShell.sync rebuilds it); the start screen list re-renders to the "no recent projects" state and
   the status line confirms. No undo; a project opened afterwards starts a new list. */
async function clearRecentProjects() {
  try { await B.recentClear(); } catch (e) { return; }
  await renderRecentList();
  if (typeof DSShell !== "undefined" && DSShell.sync) DSShell.sync();
  setMsg("recent projects cleared");
}
async function useProject(summary) {   // a project the server returned (new/open/import): make it the current one
  await flushEvents();   // the project left behind stays open: its pending edits reach it first. #249 R02: a refused write rejects — the switch is aborted before S.sid moves
  S.sid = summary.sid; localStorage.setItem("dsg_sid", S.sid);
  await initSessions(); await loadProject();
  return summary;
}
/* leaving a dirty project (opening another, closing): in-page prompt instead of a silent loss */
async function guardDirty(action) {
  if (!S.dirty) return true;
  const a = await askConfirm({ title: "Unsaved changes",
    msg: `"${S.name}" has unsaved changes. Save before you ${action}? (Unsaved edits stay in memory and in Backups/ autosaves while the project is open.)`,
    buttons: [{act:"save", label:"Save", cls:"primary"}, {act:"skip", label:"Continue without saving"}, {act:"cancel", label:"Cancel"}] });
  if (a === "cancel" || a === null) return false;
  if (a === "save") { try { await saveProjectNow(); } catch (e) { /* #249 R02: the refusal is on the status line */ } if (S.dirty) return false; }
  return true;
}
async function switchTo(sid) {
  if (sid === S.sid) { await loadProject(); return true; }
  if (!await guardDirty("switch projects")) return false;
  try { await flushEvents(); }   // "continue without saving" keeps the edits in the server's memory
  catch (e) { setMsg(`switch failed: ${errText(e)}`, "error"); return false; }   // #249 R02: the switch does not run on a refused write
  S.sid = sid; localStorage.setItem("dsg_sid", sid);
  await loadProject(); await initSessions();
  return true;
}
async function openProjectPath(path) {
  if (!await guardDirty("open another project")) return false;
  try {
    const r = await B.openProject(path);
    await useProject(r); setMsg(`opened ${r.name}`); return true;
  } catch(e) { setMsg(`open failed: ${e.message}`, "error"); return false; }
}
async function closeProject(sid, name) {
  const post = body => B.closeProject(sid, !!body.force);
  let dirty = sid === S.sid ? S.dirty : false;
  if (!dirty && sid !== S.sid) { try { dirty = (await B.projectGet(sid)).dirty; } catch(e) {} }
  const prompt = async () => {
    let a = await askConfirm({ title: "Close project",
      msg: `"${name}" has unsaved changes. Save before closing? "Discard" closes it and drops the unsaved edits (the last autosave stays in Backups/).`,
      buttons: [{act:"save", label:"Save and close", cls:"primary"}, {act:"discard", label:"Discard changes"}, {act:"cancel", label:"Cancel"}] });
    if (a === "save") {   // #127: a file changed on disk asks Save anyway / Reload from disk / Close without saving
      const s = await saveOrChoose(sid, name, "Close without saving");
      if (s === "saved") return post({});
      if (s === "reloaded") return "reloaded";
      if (s !== "leave") return null;
      a = "discard";
    }
    if (a === "discard") { cancelEvents(sid); return post({force: true}); }
    return null;
  };
  try {
    let r;
    if (dirty) r = await prompt();
    else {
      try { r = await post({}); }
      catch(e) { if (!/^409/.test(e.message)) throw e; r = await prompt(); }   // the server knows better
    }
    if (r === "reloaded") return false;   // the project stays open with the file from disk
    if (!r) { setMsg("close cancelled"); return false; }
  } catch(e) { setMsg(`close failed: ${e.message}`, "error"); return false; }
  setMsg(`closed ${name}`); cancelEvents(sid);
  if (sid === S.sid) { S.sid = null; localStorage.removeItem("dsg_sid"); setDirtyState(false); await initSessions(); await loadProject(); }
  else await initSessions();
  return true;
}

/* ---------------- project ---------------- */
/* ---------------- project ---------------- */
let _gen = 0;   // C5: generation counter — session switches invalidate in-flight loads
async function loadProject() {
  if (S.playing) pause();   // the transport plays the project that is leaving
  const gen = ++_gen;
  S.loaded = 0; SESSION_HOOKS.GLQ.loadedSeq = 0;
  staleSync();   // #121: the choices belong to the project whose file changed
  if (!S.sid) {   // S3-3: nothing is open; the lanes area offers New / Open / Open Recent
    S.tracks = {}; S.order = []; S.events = {}; S.peaks = {}; S.samplesCache = {}; S.peakAmp = {};
    S.sel = null; S.selTrack = null; SESSION_HOOKS.setSelMem({ sid: null, order: [] }); S.undo = []; S.redo = []; S.view = { t0: 0, t1: 10 }; S.zoomBack = null; S.rowH = {}; SESSION_HOOKS.lnReset(); SESSION_HOOKS.auReset();
    S.exportAgain = false;   // P4-31: no project, no export shape
    SESSION_HOOKS.tmLoad();   // P4-8: no map without a project
    setProjName(""); setDirtyState(false);
    buildTrackDOM(); invalidate(F.VIEW); updateStatus(); refreshCounts();
    S.loaded = gen; refreshRelinkBar();
    return true;
  }
  const r = await B.tracks(S.sid);
  if (gen !== _gen) return;
  S.exportAgain = false;
  try { S.exportAgain = !!(await B.exportLastGet(S.sid)).export_last; } catch (e) {}   // P4-31: File > Export Again follows the export shape of the project
  if (gen !== _gen) return;
  S.tracks = r.tracks; S.order = r.order || Object.keys(r.tracks);
  if (r.name !== undefined) setProjName(r.name);
  if (r.dirty !== undefined) setDirtyState(r.dirty);
  S.events = {}; S.peaks = {}; S.peakErr = {}; S.samplesCache = {}; S.peakAmp = {};
  S.sel = null; S.undo = []; S.redo = []; S.zoomBack = null; S.rowH = {}; SESSION_HOOKS.lnReset(); SESSION_HOOKS.auReset();
  // P4-39c: a project with tracks always has a selected track: the same project keeps it (or the neighbour of a lost
  // one), a project that opens takes the first track
  S.selTrack = SESSION_HOOKS.selMem().sid === S.sid ? pickSelTrack(SESSION_HOOKS.selMem().order, S.selTrack) : (S.order[0] || null);
  SESSION_HOOKS.setSelMem({ sid: S.sid, order: S.order.slice() });
  const t1max = Math.max(10, ...S.order.map(n => S.tracks[n].duration));
  S.view = { t0: 0, t1: t1max };   // open projects at Full zoom level
  buildTrackDOM();
  if (B.prepareLanes) { await prepareLanes(S.sid); if (gen !== _gen) return; }   // P3-4a: progress while peaks.bin is built
  const eng = B.peaksRaw ? glEngine() : null;   // P3-6b: in the app the tile bytes go to the engine, the page keeps no arrays
  for (let i = 0; i < S.order.length; i += 8) {   // 8 tracks at a time (P3-6b: 30 tracks one after the other took 2 s)
    await Promise.all(S.order.slice(i, i + 8).map(async n => {
      S.events[n] = normEvents(await B.events(S.sid, n));
      if (gen !== _gen) return;   // C5
      try { S.peaks[n] = await loadPeaks(n, eng); S.peakErr[n] = null; } catch(e) { S.peaks[n] = null; S.peakErr[n] = e.message.replace(/^\d+: /, ""); markLaneDetail(n, e.detail); }
      S.peakAmp[n] = peakAmpOf(S.peaks[n]);
    }));
    if (gen !== _gen) return;   // C5
  }
  if (gen !== _gen) return;   // C5
  try {   // #167: the saved lane-height factors of the rows (the tracks that left are skipped)
    const rh = await B.rowHeightsGet ? await B.rowHeightsGet(S.sid) : null;
    if (gen !== _gen) return;
    if (rh && rh.heights && typeof rh.heights === "object") setRowHeights(Object.fromEntries(Object.entries(rh.heights).filter(([n, f]) => S.tracks[n] && typeof f === "number" && isFinite(f) && f > 0)));
  } catch (e) { /* no row heights */ }
  await SESSION_HOOKS.tmLoad();   // P4-8: the tempo and meter map (the rulers and the grid)
  if (gen !== _gen) return;
  S.playhead = Math.min(S.playhead, maxDur());
  SESSION_HOOKS.GLQ.loadedSeq = SESSION_HOOKS.GLQ.eng ? -1 : 0;   // P3-6b: the self-test waits for the engine to answer the first draw with lanes (open to drawn; glDraw sets the number)
  invalidate(F.VIEW); flushFrame(); updateStatus(); refreshCounts();   // the loaded project is drawn now, not one frame later
  S.loaded = gen;   // the status line is final: the e2e tests wait for this before they type
  refreshRelinkBar();   // P3-4d: offline tracks offer "Find moved audio..."
  return true;   // false-y when a newer loadProject superseded this one (the early returns above)
}
async function loadPeaks(n, eng) {   // P3-6b: the tiles of a track to the engine, the page keeps the counts and the coarsest level
  if (!eng || !B.peaksRaw || !glEngine()) return B.peaks(S.sid, n);
  const r = await B.peaksRaw(S.sid, n), cur = glEngine();
  if (!cur) return B.peaks(S.sid, n);
  cur.addTiles(n, r.raw);
  return { sr: r.sr, n_samples: r.n_samples, levels: r.levels, tiles: true };
}
/* The first open of a project builds peaks.bin for every track (about 1 s per track of 350 s). The
   service builds them as a job; the progress bar shows track n of m while it runs (P3-4a). */
async function prepareLanes(sid) {
  let live = true, shown = false;
  try {
    await B.prepareLanes(sid, job => {
      if (!live || job.state === "done") return;
      const ts = Object.entries(job.tracks || {});
      const finished = ts.filter(([, t]) => t.state === "done" || t.state === "error").length;
      const cur = ts.find(([, t]) => t.state === "running");
      shown = true;
      showProgress(Math.round(100 * finished / Math.max(1, ts.length)),
        `building waveforms: ${Math.min(finished + 1, ts.length)} of ${ts.length}${cur ? ` (${cur[0]})` : ""}`);
    });
  } catch (e) { /* the lanes below show the error of each track */ }
  finally { live = false; if (shown) hideProgress(); }
}
