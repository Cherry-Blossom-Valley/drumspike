/* Drum Spike v2 — files (refactor plan step 5.2, R9 A2)
   The project name and the unsaved-changes guard, the native folder and file picks, the File menu (New,
   Open, Save As, Consolidate, Import), the importer of old sessions, relink and find moved audio, Save and
   the stale-write check (#121, #127). Moved verbatim from app.js. Plain script (no build step), loaded
   after commands.js and before app.js; its one load-time statement adds the beforeunload guard. The names
   stay top-level bindings (page globals). */
"use strict";

/* ---------------- the project name and the unsaved-changes guard ---------------- */
function setProjName(n) {
  S.name = n || "";
  setDirtyState(S.dirty);   // also syncs the window title
}
window.addEventListener("beforeunload", ev => {
  if (!S.dirty) return;
  ev.preventDefault(); ev.returnValue = "unsaved changes";   // the browser shows its own prompt
  return ev.returnValue;
});
/* ---------------- folder and file picks: the native dialogs (P3-4b) ----------------
   pickFolder({title, name, copy, file, start, hint}) -> {dir, name, copy, file} or null */
function pickFolder(o) { return DSShell.nativePick(o); }
function joinPath(a, b) { return a.endsWith("/") ? a + b : a + "/" + b; }
/* ---------------- File menu (native, shell.js) ---------------- */
async function fileNew() {   // the current project stays open (and dirty) in the list: no prompt
  const pick = await B.pickSave("New project", "Untitled project");   // the native save dialog (P3-4a)
  if (!pick) return;
  try {
    const r = await B.newProject(pick.name, pick.dir);
    await useProject(r); setMsg(`created ${r.name} in ${r.folder}`);
  } catch(e) { setMsg(`new project failed: ${e.message}`, "error"); }
}
async function fileOpen() {
  const dir = await B.pickProject();   // the native open dialog (P3-3b)
  if (dir) await openProjectPath(dir);
}
/* The app has one native save dialog for the name; it cannot hold a choice, so a second question follows (P3-4d):
   copy the audio into the new project (self-contained) or keep it where it is. Resolves true, false or null (cancel). */
async function askSaveAsAudio() {
  const modes = S.order.map(n => S.tracks[n].source_mode);
  const copied = modes.filter(m => m === "copied").length, ref = modes.filter(m => m === "referenced").length;
  const copyFirst = copied > 0 || ref === 0;
  const a = await askConfirm({ title: "Save As: the audio",
    msg: `Copy the audio files into the new project? A project with copied audio holds everything it needs and can be moved or sent. ` +
      `Now: ${ref} track${ref === 1 ? "" : "s"} referenced, ${copied} copied.` +
      (copied && !ref ? " If you keep the audio where it is, the new project points into the old project's Audio Files/." : ""),
    buttons: [{act:"copy", label:"Copy audio into the project", cls: copyFirst ? "primary" : ""},
              {act:"ref", label:"Keep the audio where it is", cls: copyFirst ? "" : "primary"},
              {act:"cancel", label:"Cancel"}] });
  return a === "copy" ? true : a === "ref" ? false : null;
}
async function fileSaveAs() {
  if (!needProject()) return;
  const pick = await B.pickSave("Save As", S.name || "Untitled project");   // the native save dialog, then the audio question
  if (!pick) return;
  const copy = await askSaveAsAudio();
  if (copy === null) { setMsg("save as cancelled"); return; }
  pick.copy = copy;
  try { await saveProjectNow(true); }   // flush pending event writes; the copy must carry every edit
  catch (e) { setMsg(`save as failed: ${errText(e)}`, "error"); return; }   // #249 R02: a refused write stops the copy
  const job = !!pick.copy;   // P3-4d: the copy runs in the background with progress and Cancel; the project stays usable
  try {
    if (job) showProgress(0, "copying the audio…", true);
    const r = await B.saveAs(S.sid, pick.dir, pick.name, pick.copy, job ? copyProgress("copying") : undefined);
    if (job) hideProgress();
    if (r.cancelled) { setMsg("Save As cancelled: nothing was copied and the project is as it was"); return; }
    await initSessions();
    // A newer load (for example Consolidate, clicked while this one still ran) has its own,
    // newer message: a superseded Save As must not overwrite it.
    if (await loadProject())
      setMsg(`saved as ${r.folder}` + (pick.copy ? " (audio copied)" : " (audio referenced)"));
  } catch(e) { if (job) hideProgress(); setMsg(`save as failed: ${e.message}`, "error"); }
}
async function fileConsolidate() {
  if (!needProject()) return;
  try {
    showProgress(0, "copying the audio…", true);
    const r = await B.consolidate(S.sid, copyProgress("copying"));
    hideProgress();
    if (r.cancelled) { setMsg("Consolidate cancelled: no record changed and the copied files are removed"); return; }
    if (r.consolidated.length) await loadProject();   // the link icons change
    setMsg(r.consolidated.length ? `consolidated: ${r.consolidated.join(", ")} copied into Audio Files/` : "nothing to consolidate: every track is copied already");
  } catch(e) { hideProgress(); setMsg(`consolidate failed: ${e.message}`, "error"); }
}
async function fileImport() { return importOldApp(); }
function showImportList(list, emptyText) {
  const box = $("#importList"); box.innerHTML = "";
  if (!list.length) { const e = document.createElement("div"); e.className = "dirEmpty"; e.textContent = emptyText; box.appendChild(e); }
  for (const s of list) {
    const row = document.createElement("div"); row.className = "imRow";
    const main = document.createElement("div");
    const nm = document.createElement("div"); nm.className = "imName"; nm.textContent = s.name;
    const sub = document.createElement("div"); sub.className = "imSub"; sub.textContent = `${s.session_id} · ${s.tracks.length} tracks: ${s.tracks.join(", ")}`;
    main.append(nm, sub);
    const b = document.createElement("button"); b.className = "btn mini primary"; b.textContent = "Import";
    b.onclick = async () => {
      if (!importStart()) return;
      b.disabled = true; setMsg(`importing ${s.name}: copying the audio into a new project folder…`);
      try {
        const r = await B.importLegacy(s.session_id);
        $("#importModal").classList.add("hidden");
        await useProject(r); setMsg(`imported ${r.name} into ${r.folder}`);
      } catch(e) { setMsg(`import failed: ${errText(e)}`, "error"); b.disabled = false; }
      finally { importBusy = false; }
    };
    row.append(main, b); box.appendChild(row);
  }
  $("#importModal").classList.remove("hidden");
}
/* ---------------- the importer of old sessions in the app (P3-4e commands, P3-4b menu item; the zip import and export
   went in P4-30) ----------------
   The app has no sessions/ store: a native dialog gives the folder, the service reads the files. One import at a time
   (no progress channel: the status line says what runs until the answer comes). */
let importBusy = false;
const errText = e => String((e && e.message) || e).replace(/^\d+: /, "");   // the clear text without the status code
function importStart() {
  if (importBusy) { setMsg("an import runs already: wait for it to finish"); return false; }
  importBusy = true; return true;
}
async function importOldApp() {   // File > Import Old Session: the folder of old sessions, then the list
  const folder = await B.pickImportFolder();
  if (!folder) { setMsg("import cancelled"); return null; }
  setMsg(`looking for old sessions in ${folder}…`);
  let list;
  try { list = await B.legacyList(folder); }
  catch(e) { setMsg(`import failed: ${errText(e)}`, "error"); return null; }
  showImportList(list, `no old Drum Spike sessions in ${folder} (an old session is a folder with a meta.json)`);
  setMsg(`${list.length} old session${list.length === 1 ? "" : "s"} in ${folder}`);
  return list;
}
/* ---------------- relink (offline tracks, P0-4) ---------------- */
async function relinkTrack(name) {
  const t = S.tracks[name];
  let start = "";   // begin where the file was last seen
  try { const src = (await B.projectGet(S.sid)).tracks[name].source; if (src && src.path) start = src.path.replace(/\/[^/]*$/, ""); } catch(e) {}
  const pick = await pickFolder({ title: `Relink ${name} (${t.status})`, file: true, start,
    hint: "Browse to the folder that holds the audio file and pick it. Size and head hash must match the original; a different file needs 'Relink anyway'." });
  if (!pick) return;
  const path = joinPath(pick.dir, pick.file);
  const post = force => B.relink(S.sid, name, path, force);
  try {
    let r;
    try { r = await post(false); }
    catch(e) {
      if (!/^409/.test(e.message)) throw e;
      const a = await askConfirm({ title: "File does not match", msg: `${path} does not match the original source of ${name} (size or head hash differ). Relink anyway? The track's sample rate and length are refreshed from this file; its hit times stay as they are.`,
        buttons: [{act:"force", label:"Relink anyway", cls:"primary"}, {act:"cancel", label:"Cancel"}] });
      if (a !== "force") { setMsg("relink cancelled"); return; }
      r = await post(true);
    }
    setMsg(`relinked ${name} -> ${path}` + (r.matched ? "" : " (different file)"));
    await loadProject();
  } catch(e) { setMsg(`relink failed: ${e.message}`, "error"); }
}

/* ---------------- find moved audio (P3-4d) ---------------- */
/* A project with offline tracks shows a bar: "Find moved audio...". One click opens the native folder dialog; the
   service walks the folder, matches by file name, then size and head hash, and relinks every track with exactly one
   match (relink_search). The result lists what it found, the tracks with more than one match (the user picks) and the
   tracks it did not find (a file picker per track, as before). */
const layerOffline = n => !!(S.tracks[n] && S.tracks[n].layer_status && S.tracks[n].layer_status !== "ok");   // P4-7a: the track's one-shot file
const offlineNames = () => S.order.filter(n => S.tracks[n] && (S.tracks[n].offline || layerOffline(n)));
let relinkHidden = null;   // the project whose bar the user hid with "Not now"
function refreshRelinkBar() {
  const bar = $("#relinkBar"), off = S.sid ? offlineNames() : [];
  if (!off.length || relinkHidden === S.sid) { bar.classList.add("hidden"); return; }
  const n = off.length, t = S.order.length;
  $("#relinkText").textContent = n === 1 ? `1 of ${t} tracks is offline: its audio was moved, renamed or changed.`
    : `${n} of ${t} tracks are offline: their audio was moved, renamed or changed.`;
  bar.classList.remove("hidden");
}
async function findMovedAudio() {
  if (!S.sid) return;
  const off = offlineNames();
  if (!off.length) { setMsg("no track is offline"); return; }
  let start = "";   // begin where the first missing file was last seen
  try { const src = (await B.projectGet(S.sid)).tracks[off[0]].source; if (src && src.path) start = src.path.replace(/\/[^/]*$/, ""); } catch(e) {}
  const folder = await B.pickDir("Find moved audio: pick the folder that holds the audio now", start);
  if (!folder) { setMsg("search cancelled"); return; }
  $("#relinkModal").classList.add("hidden");
  setMsg(`searching ${folder} for ${off.length} offline track${off.length === 1 ? "" : "s"}…`);
  showProgress(0, `searching ${folder}`, true);
  let r;
  try { r = await B.relinkSearch(S.sid, folder); }
  catch(e) { hideProgress(); setMsg(`search failed: ${errText(e)}`, "error"); return; }
  hideProgress();
  if (r.cancelled) { setMsg("search cancelled: nothing was changed"); return; }
  if (r.relinked) await loadProject();   // the lanes get their peaks; the bar and the badges update
  showRelinkResult(r, folder);
  setMsg(`found ${r.relinked} of ${r.offline} offline track${r.offline === 1 ? "" : "s"}` +
    (r.ambiguous.length ? `, ${r.ambiguous.length} ambiguous` : "") + (r.missing.length ? `, ${r.missing.length} still missing` : "") +
    ` (${r.files_seen} files looked at in ${(r.elapsed_ms / 1000).toFixed(1)} s)`);
}
function showRelinkResult(r, folder) {
  const n = r.ambiguous.length, m = r.missing.length;
  $("#relinkSummary").textContent = `Found ${r.relinked} of ${r.offline} offline track${r.offline === 1 ? "" : "s"} in ${folder}. ` +
    (n || m ? `${n} ambiguous (more than one matching file), ${m} still missing.` : "Every offline track is linked again.") + (r.truncated ? " The folder is very large: the search stopped early." : "");
  const amb = $("#relinkAmbiguous"), mis = $("#relinkMissing"); amb.innerHTML = ""; mis.innerHTML = "";
  for (const a of r.ambiguous) {
    const row = document.createElement("div"); row.className = "rlRow";
    const nm = document.createElement("div"); nm.className = "rlName"; nm.textContent = `${a.track}${a.layer ? " (layer one-shot)" : ""}: more than one file matches. Pick the right one.`; row.appendChild(nm);
    for (const path of a.candidates) {
      const b = document.createElement("button"); b.className = "btn mini rlPick"; b.textContent = path; b.title = `Link ${a.track} to this file`;   // textContent: paths are data
      b.onclick = async () => {
        try { if (a.layer) await B.layerSet(S.sid, a.track, { path }); else await B.relink(S.sid, a.track, path, false); row.remove(); await loadProject(); setMsg(`relinked ${a.track}${a.layer ? " (layer)" : ""} -> ${path}`); }
        catch(e) { setMsg(`relink failed: ${errText(e)}`, "error"); }
      };
      row.appendChild(b);
    }
    amb.appendChild(row);
  }
  for (const x of r.missing) {
    const row = document.createElement("div"); row.className = "rlRow";
    const nm = document.createElement("div"); nm.className = "rlName"; nm.textContent = `${x.track}${x.layer ? " (layer one-shot)" : ""}: not found in this folder.`;
    const b = document.createElement("button"); b.className = "btn mini"; b.textContent = "Pick the file…"; b.style.marginLeft = "8px";
    b.onclick = () => x.layer ? lyChoose(x.track) : relinkTrack(x.track);
    nm.appendChild(b); row.appendChild(nm);
    for (const d of x.differs || []) { const t = document.createElement("div"); t.className = "rlDiff"; t.textContent = `a file with this name but other content: ${d}`; row.appendChild(t); }
    mis.appendChild(row);
  }
  $("#relinkModal").classList.remove("hidden");
}

async function saveProjectNow(flushOnly, force) {   // Cmd/Ctrl+S: flush pending event writes, then write the document
  try { await flushEvents(); }
  catch (e) { setMsg(`save failed: ${errText(e)}`, "error"); throw e; }   // #249 R02: a refused write stops the save (its message is already on the status line)
  if (flushOnly || !needProject()) return;
  const sid = S.sid, seq = editSeq();   // #249 R03: the answer belongs to this project and to the edits up to now
  try { const r = await B.projectSave(sid, force); staleSet(null);
    if (S.sid === sid) setDirtyState(!!(r && r.dirty) || editSeq() !== seq);   // #249 R03: r.dirty is the truth (the project may have changed during the write); the mark only moves while this project is the open one
    setMsg(force ? "project saved over the file on disk" : "project saved");
    /* R12: the save is committed; a failed backup cleanup comes back as a warning on the status line */
    if (r && r.warning) setMsgBg(`project saved, but the backup cleanup failed: ${r.warning}`, "error"); }
  catch(e) {
    if (isStale(e)) staleSet(S.sid);
    setMsg(`save failed: ${e.message}`, "error");
  }
}
/* #121: the stale-write check. A save of a document that changed on disk since Drum Spike read it is refused (409,
   "the project file changed on disk ..."); the autosave sends project-stale for it. The status line then holds the
   error and two choices: Reload from disk (the open edits go; the autosaves stay in Backups/) or Save anyway. */
var _staleSid = null;   // the project whose file changed on disk; the choices show while it is the open one
function isStale(e) { return !!e && /the project file changed on disk/.test(e.message || ""); }
function staleSet(sid) { _staleSid = sid; staleSync(); }
function staleSync() {
  const el = $("#stStale"), on = !!S.sid && _staleSid === S.sid;
  if (el.classList.contains("hidden") === on) el.classList.toggle("hidden", !on);
}
async function staleReload(sid) {   // #127: also a project that is not the open one; it stays in the background
  sid = sid || S.sid;
  if (!sid) return;
  try {
    const cur = sid === S.sid, folder = (await B.projectGet(sid)).folder;
    cancelEvents(sid);
    await B.closeProject(sid, true);
    if (_staleSid === sid) staleSet(null);
    const r = await B.openProject(folder);
    if (cur) await useProject(r); else await initSessions();
    setMsg(`reloaded ${r.name} from disk`);
  } catch(e) { setMsg(`reload failed: ${e.message}`, "error"); }
}
/* #127: the save of the quit and close flows. When the file changed on disk, the engineer chooses: Save anyway, Reload
   from disk (the quit or close stops), leave without saving (`leave` is the label) or Cancel. Answers "saved",
   "leave", "reloaded" or null (cancel). Other save errors throw. */
async function saveOrChoose(sid, name, leave) {
  const cur = sid === S.sid;
  if (cur) { try { await flushEvents(); } catch (e) { setMsg(`save failed: ${errText(e)}`, "error"); return null; } }   // #249 R02: the close/quit save does not run on a refused write
  const seq = editSeq();   // #249 R03
  let r;
  try { r = await B.projectSave(sid); }
  catch(e) {
    if (!isStale(e)) throw e;
    if (cur) staleSet(sid);
    const a = await askConfirm({ title: "Project file changed on disk",
      msg: `"${name}": ${e.message.replace(/^409:\s*/, "")}. "Save anyway" writes the open project over it (the file on disk goes to Backups/). "Reload from disk" opens the file on disk and drops the unsaved edits. "${leave}" leaves the file on disk as it is.`,
      buttons: [{act:"force", label:"Save anyway", cls:"primary"}, {act:"reload", label:"Reload from disk"}, {act:"leave", label:leave}, {act:"cancel", label:"Cancel"}] });
    if (a === "reload") { await staleReload(sid); return "reloaded"; }
    if (a === "leave") return "leave";
    if (a !== "force") return null;
    r = await B.projectSave(sid, true);
    if (_staleSid === sid) staleSet(null);
  }
  if (sid === S.sid) setDirtyState(!!(r && r.dirty) || editSeq() !== seq);   // #249 R03: the answer clears the mark of its own project only, and only while it is the open one; r.dirty and an edit during the write keep it
  return "saved";
}
function staleSetup() {
  $("#stReload").onclick = () => staleReload();
  $("#stSaveAnyway").onclick = () => saveProjectNow(false, true).catch(() => {});   // #249 R02: the refusal is already on the status line
  $("#stEvRetry").onclick = () => { evFailSet(false); flushEvents().catch(() => {}); };   // #249 R02: the queued writes go out again; a new refusal re-arms the banner
  $("#stEvDiscard").onclick = () => { evFailSet(false); cancelEvents(S.sid); setMsg("the queued hit edits are dropped"); };
  if (B.listen) B.listen("project-stale", p => {   // the autosave found the change before the user saved
    if (!p || p.sid !== S.sid) return;
    staleSet(p.sid); setMsgBg(p.message, "error");
  }).catch(() => {});
  if (B.listen) B.listen("autosave-failed", p => {   // #228: the autosave could not write Backups/; once per failure streak
    if (!p || p.sid !== S.sid) return;
    setMsgBg(`Autosave failed: ${p.message}. Your edits are not backed up; save the project.`, "error");
  }).catch(() => {});
}
