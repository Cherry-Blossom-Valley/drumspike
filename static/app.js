/* Drum Spike v2 — DAW-style navigation
   Concepts: playhead-anchored zoom, Tab event navigation, event lines through
   both lanes (spike + waveform), follow-playhead, persistent position readout. */
"use strict";

const OWN_ZOOM_NAME_MAX = 40;   // #169: the longest name of an own zoom preset
const ROLE_DEFAULT_NOTE = { kick:36, snare:38, tom:48, hat:42, cymbal:49, other:36 };
const ROLE_DEFAULT_CLICK = { kick:1000, snare:2500, tom:700, hat:4000, cymbal:3000, other:1500 };
const $ = s => document.querySelector(s);
const fmt = (t, d=3) => t.toFixed(d);
const dScale = m => m === 1 ? 40 : 1;   // wheel line-mode deltas → px

/* ---------------- note names (octave floor(n/12)-2 → kick 36 = C1) ---------------- */
const NOTE_NAMES = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];
function nameFromNote(n){ return `${NOTE_NAMES[n%12]}${Math.floor(n/12)-2}`; }
function noteFromName(s){
  const m = /^([A-Ga-g])(#|b)?(-?\d+)$/.exec(s.trim());
  if(!m) return null;
  const base = {C:0,D:2,E:4,F:5,G:7,A:9,B:11}[m[1].toUpperCase()];
  return base + (m[2]==="#"?1:m[2]==="b"?-1:0) + 12*(parseInt(m[3])+2);
}

/* ---------------- backend ---------------- */
/* Every call to the server goes through B (static/backend.js): named operations, no paths here. */
function normEvents(d){
  const evs = Array.isArray(d) ? d : (d && d.events) || [];
  return evs.map((e,i) => { const o = {t: e.t, v: e.v, id: "l"+(i+1)};   // stable identity (B1)
    if (e.added) o.added = true; if (e.orig !== undefined) o.orig = e.orig;   // manual-hit markers
    if (e.rise !== undefined) o.rise = e.rise;   // transient rise dB (R1 confidence) — carried, not used
    if (e.cut) o.cut = true;   // P4-3: the loudness cut took it out of the output
    if (e.bleed) o.bleed = true;   // P4-14: the tom bleed rule took it out of the output (shown as a cut hit for now)
    if (e.roll) o.roll = true;   // P4-19: the roll fill added it (a normal hit; kept so a page edit does not drop the tag)
    if (e.flam) o.flam = true;   // P4-13 stage B: the flam rule added it (the second stroke of a flam; a normal hit)
    if (e.rescue) o.rescue = true;   // P4-10 stage 5: the kick rescue added it (a quiet kick the model missed; a normal hit)
    if (e.learned) o.learned = true;   // P4-38b: a K added it (kept so undo and redo snapshots keep the tag)
    if (e.filtered) o.filtered = true;   // P4-6c: the Learn filter took it out of the output (a grey hit)
    if (e.edited) o.edited = true;   // P4-14 fix round: restored from the bleed mark, kept by a re-pick or Re-analyze
    return o; }).sort((a, b) => a.t - b.t);   // P3-14d: hitsIn searches them by time (the service sends them sorted)
}
let _eid = 0;
function newId(){ return "n" + (++_eid); }

/* manual/edited hits: added by hand, or moved away from their detected position */
const MANUAL_COLOR = "#ff4fd8";
function isManual(e) { return !!e.added || (e.orig !== undefined && Math.abs(e.t - e.orig) > 1e-9); }
/* a kept hit: manual, or a hand velocity or a restored hit (`edited: true`) — the page's copy of
   edit::is_kept (crates/app/src/edit.rs). One predicate decides the Keep/Discard dialog of a
   re-analysis and the service's merge (keep_edited), so both see the same edits. */
function isKept(e) { return isManual(e) || e.edited === true; }

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
/* ---------------- in-page confirm (replaces window.confirm for project prompts) ---------------- */
function askConfirm(o) {
  return new Promise(resolve => {
    const m = $("#confirmModal"); $("#confirmTitle").textContent = o.title || "Confirm"; $("#confirmMsg").textContent = o.msg || "";
    const acts = $("#confirmActions"); acts.innerHTML = "";
    for (const b of o.buttons) {
      const btn = document.createElement("button"); btn.className = "btn" + (b.cls ? " " + b.cls : ""); btn.textContent = b.label; btn.dataset.act = b.act;
      btn.onclick = () => { m.classList.add("hidden"); resolve(b.act); };
      acts.appendChild(btn);
    }
    m.classList.remove("hidden");
  });
}
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

/* ---------------- events reload (the analysis wrote new events into the open project) ---------------- */
async function loadEventsOnly() {
  S.undo.length = 0; S.redo.length = 0;   // B2: stacks reference pre-analysis state
  for (const n of S.order) S.events[n] = normEvents(await B.events(S.sid, n));
  S.sel = null; setDirtyState(true);   // the analysis wrote new events into the open project
  invalidateHits(); updateStatus(); refreshCounts();
}

/* ---------------- track DOM ---------------- */
function renderNoProject(scroll) {
  const box = document.createElement("div"); box.className = "empty-state noProject";
  const p = document.createElement("div"); p.textContent = "No project is open."; box.appendChild(p);
  const acts = document.createElement("div"); acts.className = "esActs";
  const bNew = document.createElement("button"); bNew.id = "esNew"; bNew.className = "btn primary"; bNew.textContent = "New project…"; bNew.onclick = fileNew;
  const bOpen = document.createElement("button"); bOpen.id = "esOpen"; bOpen.className = "btn"; bOpen.textContent = "Open…"; bOpen.onclick = fileOpen;
  acts.append(bNew, bOpen); box.appendChild(acts);
  const title = document.createElement("div"); title.className = "ptitle"; title.textContent = "Open Recent"; box.appendChild(title);
  const list = document.createElement("div"); list.id = "esRecentList"; box.appendChild(list);
  const clear = document.createElement("button");   // #441: Clear Recent next to the list
  clear.id = "esRecentClear"; clear.className = "btn esClear"; clear.textContent = "Clear Recent";
  clear.onclick = clearRecentProjects;
  box.appendChild(clear);
  scroll.appendChild(box);
  renderRecentList();   // fills #esRecentList as well
}
/* P3-6b: a removed canvas keeps its backing store until the garbage collector finds it, and the collector does not see
   that memory: three re-opens of the 30-track project raised the footprint of the WebContent process by 380 MB.
   Size 0 gives the store back at once. */
function releaseCanvases(root) { root.querySelectorAll("canvas").forEach(c => { c.width = 0; c.height = 0; }); }
function buildTrackDOM() {
  const scroll = $("#lanesScroll"); releaseCanvases(scroll); scroll.innerHTML = "";
  MT.tracks = {};   // P3-16: the meters of the old rows are gone
  if (!S.sid) { renderNoProject(scroll); return; }
  if (!S.order.length) {
    scroll.innerHTML = `<div class="empty-state">No tracks yet — drop audio files or a folder here, or click <b>⬆ Add tracks</b> to add kick, snare, toms and overheads.<br>Then hit <b>⚙ Analyze all</b>.</div>`;
    return;
  }
  for (const name of S.order) {
    const t = S.tracks[name];
    const row = document.createElement("div");
    row.className = "track" + (t.offline ? " offline" : ""); row.dataset.name = name;
    t.params = t.params || defaultParams(t.role);   // the Inspector edits these (P3-14a)
    row.innerHTML = `
      <div class="track-row">
        <div class="thead" title="Click: select the track (the Inspector shows it). Drag: reorder. Right-click: Export…, Re-analyze, Reset Volume and Pan">
          <span class="tstrip" style="background:${t.color}"></span>
          <div class="tname"></div>
          <div class="tnote">${t.role} · <span class="nnote">${nameFromNote(t.note)}</span> (${t.note})<span class="tfollow"></span></div>
          <div class="tctl">
            <button class="btn mini bMute" title="Mute (Track > Mute Selected Track, Shift+M)">M</button>
            <button class="btn mini bSolo" title="Solo (Track > Solo Selected Track, Shift+S)">S</button>
            <div class="mslider vol" data-ctl="vol"><div class="mfill"></div><div class="mknob"></div></div>
            <div class="mslider pan" data-ctl="pan"><div class="mcenter"></div><div class="mknob"></div></div>
            <span class="mval" data-for="vol"></span>
            <span class="mval mvalPan" data-for="pan"></span>
          </div>
          <div class="meter" title="Level after volume and pan. The red light: clipped (click to reset)">
            <div class="mclip"></div>
            <div class="mbars"></div>
          </div>
          <div class="rowResize" title="Drag: the height of this track's wave lane. Double-click: back to the Track Height"></div>
        </div>
        <div class="tlanes">
          <canvas class="lane spike" height="34"></canvas>
          <canvas class="lane wave"  height="120"></canvas>
          ${VL.on ? `<canvas class="lane vel"   height="${VL.h}"></canvas>` : ""}
          <div class="rowResize" title="Drag: the height of this track's wave lane. Double-click: back to the Track Height"></div>
        </div>
      </div>`;
    scroll.appendChild(row);
    if (t.follows) { row.querySelector(".tfollow").textContent = ` · follows ${t.follows}`; row.classList.add("follower"); }   // P3-18: names are data
    const tn = row.querySelector(".tname");   // C1: name as text node — no HTML injection
    const nm = document.createElement("span"); nm.className = "tnm"; nm.textContent = name; nm.title = name;
    tn.appendChild(nm);
    setHeadIcon(row, t.role);   // 0.9.3: the role icon left of the name
    // P3-14c: the header is compact; the 🔗 badge and the hit counts are in the Inspector (source state, Detected, Edited)
    if (t.offline) {
      const b = document.createElement("button"); b.className = "btn mini tOffline"; b.textContent = "offline";
      b.title = `source ${t.status}: click to relink the audio file`;
      b.onclick = () => relinkTrack(name);
      tn.appendChild(b);
    }
    row.querySelector(".thead").addEventListener("click", ev => {
      if (headerDragged) { headerDragged = false; return; }   // drag-release, not a click
      if (ev.target.closest("button") || ev.target.tagName === "INPUT" || ev.target.tagName === "SELECT") return;
      if (ev.target.closest(".mslider, .meter, .rowResize")) return;   // P3-16: the mixer controls; #439: the height handle
      setSelTrack(name);   // P4-39c: a second click keeps the track selected
    });
    row.querySelector(".thead").addEventListener("contextmenu", ev => { ev.preventDefault(); openTrackMenu(name, ev.clientX, ev.clientY); });   // P3-14e
    setupTrackDrag(row);   // Pro Tools style: drag the header up/down to reorder
    attachTrackHandlers(row, name);
    vlBind(row, name);   // P4-40f: the velocity lane's gestures
    bindRowResize(row, name);   // #167: drag the lower border of the row, double-click resets it
    bindRowResize(row, name, row.querySelector(".thead .rowResize"));   // #439: the header's bottom edge drags the same height
    bindTrackMix(row, name);   // P3-16
  }
  refreshCounts();
  document.querySelectorAll(".track").forEach(r => r.classList.toggle("selTrack", r.dataset.name === S.selTrack));   // P4-39c: the first track is selected at open
  syncInspector(true);   // P3-14a: the rows are new; the Inspector shows the selected track or its empty state
  sizeCanvases();   // P3-14d: the new canvases get their CSS size now (from GEO: no layout read); the frame draws them
  invalidate(F.VIEW | F.NEAR);
}
/* P4-39c: a project with tracks always has a selected track (Inspector, key commands, Learn, follow menus). Clicks on
   empty lane space, Escape and lnSetSel(null) clear the hit selection only. A selected track that leaves (removed, hidden)
   passes the selection to the next track in the order it had, else the previous one. */
var selMem = { sid: null, order: [] };   // the project and track order the selection was last checked against
function pickSelTrack(oldOrder, sel) {
  if (sel && S.tracks[sel]) return sel;
  const i = oldOrder.indexOf(sel);
  if (i >= 0) {
    for (let j = i + 1; j < oldOrder.length; j++) if (S.tracks[oldOrder[j]]) return oldOrder[j];
    for (let j = i - 1; j >= 0; j--) if (S.tracks[oldOrder[j]]) return oldOrder[j];
  }
  return S.order.find(n => S.tracks[n]) || null;
}
function ensureSelTrack() {   // in updateStatus: a lost selection moves on; the order is remembered
  if (S.sid !== selMem.sid) selMem = { sid: S.sid, order: [] };
  if (!S.selTrack || !S.tracks[S.selTrack]) {
    const n = pickSelTrack(selMem.order, S.selTrack);
    if (n) { setSelTrack(n); return; }
  }
  const o = S.order;
  if (o.length !== selMem.order.length || o.some((n, i) => n !== selMem.order[i])) selMem.order = o.slice();
}
function setSelTrack(name) {   // the header click and the keys P and ; (P3-19)
  if (!name || !S.tracks[name]) name = pickSelTrack(selMem.order, S.selTrack);   // P4-39c: never none while tracks exist
  S.selTrack = name;
  document.querySelectorAll(".track").forEach(r => r.classList.toggle("selTrack", r.dataset.name === S.selTrack));
  invalidate(F.HEAD);   // P3-14b: the sub counter uses the rate of the selected track
  updateStatus();
}
/* ---------------- track reordering (drag header, live reorder like Pro Tools) ---------------- */
let headerDragged = false;
function applyOrder(newOrder) {
  S.order = newOrder;
  const scroll = $("#lanesScroll");
  for (const n of newOrder) {
    const r = document.querySelector(`.track[data-name="${n}"]`);
    if (r) scroll.appendChild(r);   // moves existing DOM nodes — canvas pixels survive
  }
  B.reorder(S.sid, newOrder)
    .then(() => { setMsg("track order saved"); setDirtyState(true); })
    .catch(e => setMsg(`reorder failed: ${e.message}`, "error"));
  invalidate(F.VIEW);   // the overview shows the tracks in this order; the rows' places come from the observer
}
function setupTrackDrag(row) {
  const thead = row.querySelector(".thead");
  thead.addEventListener("mousedown", ev => {
    if (ev.button !== 0) return;
    if (ev.target.closest("button") || ["INPUT","SELECT"].includes(ev.target.tagName)) return;
    ev.preventDefault();   // no text selection while grabbing the header
    const startY = ev.clientY;
    let dragging = false;
    const scroll = $("#lanesScroll");
    const mv = ev2 => {
      if (!dragging && Math.abs(ev2.clientY - startY) > 5) {
        dragging = true; headerDragged = true;
        row.classList.add("dragging");
        document.body.classList.add("draggingTrack");
      }
      if (!dragging) return;
      // auto-scroll when dragging near the list edges
      const sr = scroll.getBoundingClientRect();
      if (ev2.clientY < sr.top + 48) scroll.scrollTop -= 14;
      else if (ev2.clientY > sr.bottom - 48) scroll.scrollTop += 14;
      // live reorder: the dragged row slips between neighbors as it crosses their midpoints
      const rows = [...scroll.querySelectorAll(".track")].filter(r => r !== row);
      for (let i=0; i<rows.length; i++) {
        const rc = rows[i].getBoundingClientRect();
        if (ev2.clientY < rc.top + rc.height/2) { scroll.insertBefore(row, rows[i]); break; }
        if (i === rows.length-1 && ev2.clientY > rc.bottom - rc.height/2) scroll.appendChild(row);
      }
    };
    const up = () => {
      window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up);
      row.classList.remove("dragging");
      document.body.classList.remove("draggingTrack");
      if (dragging) {
        const newOrder = [...scroll.querySelectorAll(".track")].map(r => r.dataset.name);
        if (newOrder.join("\u0000") !== S.order.join("\u0000")) applyOrder(newOrder);
      }
    };
    window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
  });
}
function refreshCounts(track) {   // the header counts of one track, or of every track
  for (const name of track ? [track] : S.order) {
    const row = rowOf(name);
    if (!row) continue;
  }
  if (INS.name && (!track || track === INS.name)) { insCounts(); hgScheduleRefresh(track); }   // P3-14a; P4-3: the histograms follow an edit
}
function defaultParams(role) {
  return { thresholds: [0.22,0.24,0.32,0.22,0.30], min_gap_ms: 20, refine: true,
           vel_lo_db: -40, vel_hi_db: -6, click_freq: ROLE_DEFAULT_CLICK[role] || 1500 };
}

/* ---------------- P3-18: sensitivity and soft tracks ----------------
   The Sensitivity slider maps a value 0 to 150 to the five class thresholds; crates/app/src/sensitivity.rs has the same
   formulas (the service's 0-hit retry uses them). 0 to 100 is the mapping of P3-14a (1.6 - 1.2 v/100, so an old project
   gives the same thresholds back; its end 0.088 to 0.128), 100 to 150 goes on geometrically down to 0.01 for the kick.
   The readout under the slider shows the lowest and the highest threshold. A track whose analysis gave no hit got its
   sensitivity raised by the service (params.weak_signal {from, to}); the Inspector and the status line say so.
   A track can follow another track (S.tracks[n].follows): its hits are the source's, derived by the service, read-only
   here (followerLocked); the header and the Inspector show "follows <name>". */
const SENS = { BASE: [0.22,0.24,0.32,0.22,0.30], DEFAULT: 50, OLD_MAX: 100, MAX: 150, FLOOR: 0.01, CEIL: 0.85 };
function sensScale(v) {
  v = Number.isFinite(v) ? Math.min(SENS.MAX, Math.max(0, v)) : SENS.DEFAULT;
  if (v <= SENS.OLD_MAX) return 1.6 - 1.2*(v/100);   // the operation order of P3-14a: the same doubles
  return 0.4 * Math.pow((SENS.FLOOR/0.22)/0.4, (v - SENS.OLD_MAX)/(SENS.MAX - SENS.OLD_MAX));
}
function sensThresholds(v) { const s = sensScale(v); return SENS.BASE.map(b => Math.min(SENS.CEIL, Math.max(SENS.FLOOR, b*s))); }
function sensOf(th) {   // the slider value of a track's thresholds (the mean against the mean 0.26 of the base)
  const mean = th.reduce((a,b)=>a+b,0)/th.length, s = mean/0.26;
  const v = s >= 0.4 ? (1.6 - s)/1.2*100 : SENS.OLD_MAX + (SENS.MAX - SENS.OLD_MAX)*Math.log(0.4/s)/Math.log(0.4/(SENS.FLOOR/0.22));
  return Number.isFinite(v) ? Math.min(SENS.MAX, Math.max(0, v)) : SENS.MAX;
}
function sensReadout(v) { const th = sensThresholds(v); return `thresholds ≈ ${Math.min(...th).toFixed(2)} … ${Math.max(...th).toFixed(2)}`; }
function weakSignalText(w) {   // params.weak_signal or a job entry's weak_signal
  if (!w) return "";
  return w.to === null || w.to === undefined ? `no hits found (sensitivity up to ${SENS.MAX} tried)` : `weak signal: sensitivity raised to ${w.to} (from ${w.from})`;
}
function weakSignals(tracks) {   // the job's tracks: [[name, text], ...] for the status line
  return Object.entries(tracks || {}).filter(([, t]) => t.state === "done" && t.weak_signal).map(([n, t]) => [n, `${n}: ${weakSignalText(t.weak_signal)}`]);
}
function followersOf(name) { return S.order.filter(n => S.tracks[n] && S.tracks[n].follows === name); }
/* 0.9.3: the role icon of the track header (inline SVG, no image files, no external fonts).
   #9: the instrument drawings of the header, 48x48. */
const ROLE_ICONS = {
  // #9, review 2: 3/4-view instrument drawings, 48x48, stroke currentColor with a light
  // same-colour fill; the header element gets the track colour (setHeadIcon).
  // kick: the round front head with tension rods, the spurs and the pedal.
  kick: '<g fill="none"><circle cx="24" cy="16" r="12"/><circle cx="24" cy="16" r="12" fill="currentColor" fill-opacity=".12"/><path d="M24 1.5v3M24 27.5v3M9 16h3M36 16h3M12.7 4.7l2.1 2.1M33.2 4.7l-2.1 2.1M12.7 27.3l2.1-2.1M33.2 27.3l-2.1-2.1"/><path d="M13 25.5 8.5 33M35 25.5l4.5 7.5"/><path d="M24 28v7M24 35h12M28 35v4"/><circle cx="24" cy="30.5" r="1.8" fill="currentColor" stroke="none"/></g>',
  // snare: short and wide shell, the top head as an ellipse, lugs, hoop, tripod.
  snare: '<g fill="none"><path d="M8 15 10.5 26M40 15 37.5 26"/><path d="M10.5 26A14 4.5 0 0 0 37.5 26" /><ellipse cx="24" cy="15" rx="16" ry="5" fill="currentColor" fill-opacity=".12"/><ellipse cx="24" cy="15" rx="13.5" ry="4"/><path d="M11 21h4M33 21h4M17 22.5h3M28 22.5h3"/><path d="M24 29v4M24 33l-8 8M24 33l8 8"/></g>',
  // tom: the deeper floor tom on three straight legs.
  tom: '<g fill="none"><path d="M9 10 11.5 32M39 10 36.5 32"/><path d="M11.5 32A13 4.5 0 0 0 36.5 32"/><ellipse cx="24" cy="10" rx="15" ry="4.5" fill="currentColor" fill-opacity=".12"/><ellipse cx="24" cy="10" rx="12.5" ry="3.5"/><path d="M12 17h4M32 17h4M13.5 24h3.5M31 24h3.5"/><path d="M12 33l-2.5 9M24 33v9M36 33l2.5 9"/></g>',
  // hat: two cymbals on a stand, the footboard of the pedal.
  hat: '<g fill="none"><ellipse cx="24" cy="14" rx="14" ry="4" fill="currentColor" fill-opacity=".12"/><ellipse cx="24" cy="20" rx="11.5" ry="3.2"/><path d="M24 10.5V6M24 23v10M24 33l-9 9M24 33l9 9"/><path d="M15 42h9"/></g>',
  // cymbal: a tilted ellipse with a bell on a stand (the crash/ride of the overheads).
  cymbal: '<g fill="none"><g transform="rotate(-10 24 15)"><ellipse cx="24" cy="15" rx="17" ry="4.5" fill="currentColor" fill-opacity=".12"/><ellipse cx="24" cy="15" rx="17" ry="4.5"/><circle cx="24" cy="13" r="2.6"/></g><path d="M24 20v13M24 33l-9 9M24 33l9 9"/></g>',
  // other: the microphone (room, clap and every role without its own drum).
  other: '<g fill="none"><rect x="18" y="4" width="12" height="17" rx="6" fill="currentColor" fill-opacity=".12"/><rect x="18" y="4" width="12" height="17" rx="6"/><path d="M20 9h8M20 13h8M20 17h8M14 15v3a10 10 0 0 0 20 0v-3M24 28v9M15 40h18"/></g>',
};
function roleIconEl(role) {   // a role name is a fixed key, never data: the markup is ours
  const s = document.createElement("span"); s.className = "tart";
  s.innerHTML = `<svg viewBox="0 0 48 48" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${ROLE_ICONS[role] || ROLE_ICONS.other}</svg>`;
  s.title = `role: ${role}`;
  return s;
}
function setHeadIcon(row, role) {   // the drawing sits right-aligned in the header's free area, in the track colour
  const th = row.querySelector(".thead"); if (!th) return;
  const cur = th.querySelector(".tart"); if (cur) cur.remove();
  const s = roleIconEl(role);
  const t = S.tracks[row.dataset.name];   // #9 review 2: the drawing takes the track colour
  if (t && t.color) s.style.color = t.color;
  th.appendChild(s);
}
function roleLineText(t) { return `${t.role} · ${nameFromNote(t.note)} (${t.note})` + (t.follows ? ` · follows ${t.follows}` : ""); }
function followerLocked(name) {   // true (with the message) when the track's hits are derived from another track
  const t = S.tracks[name];
  if (!t || !t.follows) return false;
  setMsg(`${name} follows ${t.follows}: its hits are derived, edit the hits of ${t.follows}`);
  return true;
}
function refreshHeadRole(name) {   // the role line of the header, and the Inspector's when it shows the track
  const row = rowOf(name), t = S.tracks[name];
  if (row && t) {
    setHeadIcon(row, t.role);   // 0.9.3: a role change swaps the icon at once
    row.querySelector(".nnote").textContent = nameFromNote(t.note);
    row.querySelector(".tfollow").textContent = t.follows ? ` · follows ${t.follows}` : "";   // textContent: a track name is data
    row.classList.toggle("follower", !!t.follows);
  }
  if (name === INS.name) insRoleLine();
}
async function reloadFollowers(names) {   // the service derived the followers' events again (an edit or an analysis of the source)
  for (const f of names || []) {
    if (!S.tracks[f] || !S.sid) continue;
    S.events[f] = normEvents(await B.events(S.sid, f));
    if (S.sel && S.sel.track === f && S.sel.index >= S.events[f].length) S.sel = null;
    invalidateHits(f); refreshCounts(f);
  }
}
async function refreshTrackMeta() {   // after an analysis: the params the retry changed and the follows, without a DOM rebuild
  if (!S.sid) return;
  const gen = loadGen();
  try {
    const r = await B.tracks(S.sid);
    if (gen !== loadGen()) return;
    for (const [n, t] of Object.entries(r.tracks || {})) {
      if (!S.tracks[n]) continue;
      if (t.params) S.tracks[n].params = t.params;
      S.tracks[n].follows = t.follows || null; S.tracks[n].n_events = t.n_events;
      if (t.layer) { S.tracks[n].layer = t.layer; S.tracks[n].layer_status = t.layer_status; } else { delete S.tracks[n].layer; delete S.tracks[n].layer_status; }   // P4-7c: agent commands change them too
      if (t.sampler) S.tracks[n].sampler = t.sampler; else delete S.tracks[n].sampler;
      refreshHeadRole(n);
    }
  } catch (e) { /* the next project load shows them */ }
  if (INS.name) syncInspector(true);
}
/* Inspector "Hits" and Track > Follow Hits Of / Own Detection: the track takes the hits of `src`, or detects itself */
async function setTrackFollow(name, src) {
  if (!S.tracks[name]) return false;
  if (analysisRunning) { setMsg("analysis running: change the follow after it"); return false; }
  src = src || null;
  if (src && !S.tracks[src]) { setMsg(`no track named ${src}`); return false; }
  try { await flushEvents(); }
  catch (e) { setMsg(`follow failed: ${errText(e)}`, "error"); return false; }   // #249 R02: the follow must not take the service's old hits
  try {
    await B.setFollow(S.sid, name, src);
    S.tracks[name].follows = src;
    S.events[name] = normEvents(await B.events(S.sid, name));
  } catch (e) { setMsg(`follow failed: ${errText(e)}`, "error"); if (name === INS.name) syncInspector(true); return false; }
  if (S.sel && S.sel.track === name) S.sel = null;
  setDirtyState(true); refreshHeadRole(name); invalidateHits(name); refreshCounts(name); updateStatus();
  if (name === INS.name) syncInspector(true);
  DSShell.sync();   // the check marks of Track > Follow Hits Of
  if (src) {   // #411: one hit reads "1 hit", not "1 hits"
    const n = S.events[name].length;
    setMsg(`${name} follows ${src}: ${n} ${n === 1 ? "hit" : "hits"}, velocities from its own signal`);
  } else {
    setMsg(`${name}: own detection (re-analyze to detect its hits)`);
  }
  return true;
}
function followSelected(src) { const n = needTrack(); if (n) return setTrackFollow(n, src); }

/* ---------------- playhead overlay (global, Pro Tools style) ---------------- */
function positionPlayhead() {   // the overlay lives in #editor; it shows only inside the lane area (never over the header column or the Inspector)
  const W = laneWidth();
  const x = Math.round((S.theadW + tToX(S.playhead, W)) * 64) / 64;   // P4-1: float noise writes no new style
  const el = $("#playheadOverlay");
  const display = x >= S.theadW - 0.5 && x <= S.theadW + W + 0.5 ? "block" : "none";
  if (FR.head[0] !== display) el.style.display = FR.head[0] = display;
  if (CP.anim) return;   // P4-1: the compositor moves it
  const tf = `translateX(${x}px)`;
  if (FR.head[1] !== tf) el.style.transform = FR.head[1] = tf;
}
function drawHead() {   // the playhead and the time display; a text is written only when it changed
  positionPlayhead();
  const [main, sub] = counterTexts();
  if (FR.head[2] !== main) $("#timeDisplay").textContent = FR.head[2] = main;
  if (FR.head[3] !== sub) $("#sampleDisplay").textContent = FR.head[3] = sub;
}

/* ---------------- follow strip (P4-1, review Q7) ----------------
   Smooth follow moves the view with the playhead. Before, every frame drew the ruler, the spike lanes, the hit lines
   and the GL wave lanes again (60 draws per second since P3-14d). Now the page draws a strip two views wide once
   (S.view [s, s + 2d] on twice the lane width, with the same draw functions, so a hit line sits on the same pixel
   as in a direct draw) and a compositor animation moves it (translateX, linear, from the transport model). The GL
   wave lanes of the strip come from the tile worker as two ImageBitmaps (one per view, op "strip"); the page shows
   them in two bitmaprenderer canvases (#lanesGLStrip) in the same task as the 2D canvases and the animations, so the
   wave and the hit lines move together (before, the GL wave was a frame behind the 2D lines: W4). The strips lie on
   a grid of half a view: strip k starts at s0 + k d/2, so the next one is asked for while the current one shows and
   is shown when the view's start reaches it (it would be good until s + d). Only the visible rows are in a strip.
   Fallback to the frame path (spLeave): a pause, a seek, a zoom or a pan (a VIEW frame), an edit or a new selection
   (a track redraw), a scroll, a resize, a mute (a GL repaint), a view under SP_MIN_S or at the sample level (the
   sample windows are not in a strip), the end of the song, a late or failed strip. After a fallback the GL canvas is
   drawn again; the strip stays (frozen) until that draw is shown. Not in a strip: the engine in the page (no
   worker), a page without the ImageBitmap transfer. Canvas 2D (no GL): the page draws the wave lanes into the strip. */
const SP = { on: false, s: 0, d: 0, key: "", req: null, els: [], anims: [], box: null, leaving: 0, leaveFrames: 0, timer: 0, retryAt: 0,
  n: { enter: 0, swap: 0, leave: 0, late: 0, failed: 0, ahead: 0 }, why: "" };
const SP_MIN_S = 1, SP_SAMPLES = 262144, SP_LEAD_S = 0.5, SP_AHEAD_S = 1, SP_SHOW_FRAMES = 60;   // P4-1b: lead and ahead in seconds of wall time
function spRows() {   // the rows that show in the lanes area now (a strip draws only these)
  const out = [];
  for (const n of S.order) {
    const row = rowOf(n), g = GEO.rows[n];
    if (row && g && row._live !== false && g.bottom - GEO.top >= 0 && g.top - GEO.top <= GEO.scH) out.push({ row, name: n });
  }
  return out;
}
function spKey() {   // what a strip depends on besides its time: a change makes it stale
  // P4-1b: spTick sets S.view = {t0, t0 + d}, and (t0 + d) - t0 can be d plus or minus an ulp; with that in the key the
  // strip left for "layout" at random and came back (a fallback draw and new stores). viewDur() gives the span without it.
  return [viewDur(), GEO.laneW, GEO.rulerW, GEO.dpr, GEO.top, GEO.scW, GEO.scH, GEO.waveX, GEO.waveH, rowHKey(), GEO.n, S.ampZoom, S.order.join("|"),
          [...document.querySelectorAll("#lanesScroll .track.muted")].map(r => r.dataset.name).join("|"), !!glEngine()].join(",");
}
/* P4-39b: can a glide strip show in this view at all? The static conditions only (the transient ones stay in spCan):
   the view is at least SP_MIN_S, no track is at the sample level, the engine has a strip (GL) or the page has the peak
   arrays (Canvas 2D). */
function glideStatic() {
  const d = viewDur();
  if (d < SP_MIN_S) return false;
  for (const n of S.order) { const p = S.peaks[n]; if (p && p.sr && d * p.sr <= SP_SAMPLES) return false; }   // the sample level
  const eng = glEngine();
  if (eng) return !!eng.strip;
  return S.order.every(n => !S.peaks[n] || !S.peaks[n].tiles);   // Canvas 2D: the page needs the peak arrays
}
/* P4-39b: what the follow does in this view: "smooth", "page" or "off". The chosen mode "smooth" behaves like "page"
   (the view stands still, the compositor moves the playhead, one flip to FOLLOW_FRAC) where no glide can show; before,
   every frame was a full VIEW redraw there (a 2 s view: the owner's stutter and flicker). */
function followKind() {
  if (S.followMode === "off" || S.followHold) return "off";
  if (S.followMode === "page") return "page";
  return glideStatic() ? "smooth" : "page";
}
function spCan(pos, ahead) {   // the strip may show at playhead pos (ahead: pos is the anchor where the glide starts)
  if (window.__DS_NO_STRIP__ || followKind() !== "smooth" || !GEO.ok || hitDrag) return false;
  if (!AC.running || AC.pending !== null || AC.seeking) return false;
  const v = S.view, d = viewDur();
  if (pos < v.t0 || pos > v.t1 || (!ahead && cpStill(pos))) return false;
  if (pos - FOLLOW_FRAC * d + d >= maxDur()) return false;   // the view reaches the end of the song and stops there
  if (!glideStatic()) return false;   // P4-39b: under SP_MIN_S, the sample level, no strip in the engine
  const eng = glEngine();
  return eng ? !!(eng.ready && !eng.lost) : true;
}
function spLanes(view) {   // the GL lanes of the visible rows for one view (glDraw's places and culling)
  const dpr = GEO.dpr, out = [];
  for (const n of S.order) {
    const row = rowOf(n), g = GEO.rows[n];
    if (!row || !g || !S.peaks[n] || !S.tracks[n]) continue;
    const y = g.wave - GEO.top, H = g.waveH || GEO.waveH;
    if (y + H < 0 || y > GEO.scH) continue;
    const l = glLane(row.querySelector(".wave"), n, view);
    l.y = Math.round(y * dpr);   // #435: the strip draws each view on its own laneW-wide tile, so the lane sits at x 0 (glDraw's shared canvas places it at the scroll's waveX)
    out.push(l);
  }
  return out;
}
function spDrop() {   // P4-1b fix: drop the strip request; its bitmaps (if they came) are closed, an answer still on its way too
  const r = SP.req;
  SP.req = null;
  if (r && r.bitmaps) { r.bitmaps.forEach(b => b.close()); r.bitmaps = null; }
}
function spRequest(s) {   // ask the worker for the GL part of the strip from s (Canvas 2D: nothing to ask)
  spDrop();
  const d = viewDur(), eng = glEngine(), req = SP.req = { s, d, key: spKey(), bitmaps: null, done: !eng, failed: false };
  if (!eng) return;
  eng.size(Math.max(1, Math.round(GEO.laneW * GEO.dpr)), Math.max(1, Math.round(GEO.scH * GEO.dpr)));   // #435: the strip's tiles are the lane's own canvas, laneW wide, not the scroll's — tiles sized to the scroll's clientWidth left a laneW - scW dark gap column between them (the clientWidth is smaller by the scrollbar)
  // keep: no strip shows, so #lanesGL shows now or will show before this strip does (after a fallback spLeftCheck shows
  // it once the fallback draw is there, maybe before this answer); the worker draws its lanes again after taking the parts
  // off its canvas. Under a strip it lets the canvas go (1 x 1) until the strip ends. P4-1b fix: was "#lanesGL visible
  // now", which was false in a re-entry before spLeftCheck: the worker released the canvas and spLeftCheck showed it blank.
  const keep = !SP.on;
  eng.strip([spLanes({ t0: s, t1: s + d }), spLanes({ t0: s + d, t1: s + 2 * d })], keep).then(r => {
    if (SP.req !== req) { if (r.bitmaps) r.bitmaps.forEach(b => b.close()); return; }
    if (r.bitmaps) { req.bitmaps = r.bitmaps; req.done = true; } else { req.failed = true; SP.n.failed++; SP.why = r.error; }
    if (S.playing) requestFrame();
  });
}
function spBox() {
  if (SP.box) return SP.box;
  const box = document.createElement("div"), inner = document.createElement("div");
  box.id = "lanesGLStrip"; box.appendChild(inner);
  for (let i = 0; i < 2; i++) inner.appendChild(document.createElement("canvas"));
  $("#editor").insertBefore(box, $("#lanesScroll"));
  return (SP.box = box);
}
/* Show strip req: draw its 2D part (two views on twice the width), show its bitmaps, arm the animations. In a tick. */
function spApply(req, now) {
  const W = GEO.laneW, RW = GEO.rulerW, d = req.d, s = req.s, dpr = GEO.dpr, rows = spRows();
  const keep = [S.view, GEO.laneW, GEO.rulerW];
  cpStop();   // P4-1b fix round: an entry from spAhead comes straight from the compositor playhead (no frame path stopped it)
  for (const a of SP.anims) a.cancel();   // spApply redraws in this task: no paint sees the canceled state
  SP.anims = [];
  try {
    SP.drawing = true;
    S.view = { t0: s, t1: s + 2 * d }; GEO.laneW = 2 * W; GEO.rulerW = 2 * RW;   // the draw functions draw the strip
    for (const { row, name } of rows) {
      const sp = row.querySelector(".spike"), wv = row.querySelector(".wave");
      sizeLaneCanvas(sp, true); sizeLaneCanvas(wv, true);
      drawSpike(sp, name);
      const vl = VL.on ? row.querySelector(".vel") : null;   // #435: the vel lane is a canvas.lane too: it rides the strip's animation, so it must cover both views (before, it kept its one-view draw: the stalks ended at the seam with the scale text there and nothing right of it)
      if (vl && S.tracks[name]) { sizeLaneCanvas(vl, true); drawVelLane(vl, name); }
      if (req.bitmaps) { wv.getContext("2d").clearRect(0, 0, 2 * W, laneSize(wv)[1]); drawEventLines(wv, name); }
      else drawWave(wv, name);
    }
    const rul = $("#ruler");
    rul.style.width = 2 * RW + "px"; rul.width = 2 * RW * dpr; rul.height = 22 * dpr; rul.getContext("2d").setTransform(dpr, 0, 0, dpr, 0, 0);
    sizeMapCanvases();   // P4-8: the map rulers are strips too (drawRuler draws them)
    drawRuler();
  } finally { [S.view, GEO.laneW, GEO.rulerW] = keep; SP.drawing = false; }
  FR.sized = "";   // the canvases are not at the sizes of GEO now: the fallback sizes them again
  SP.els = []; SP.clips = [];
  const clip = (el, w) => { const v = `polygon(0 0, ${w}px 0, ${w}px 100%, 0 100%)`; if (el.style.clipPath !== v) el.style.clipPath = v; SP.clips.push(el); };
  for (const { row } of rows) {   // the lane box is a little wider than the lane: show the strip on the lane's width only
    for (const c of row.querySelectorAll("canvas.lane")) SP.els.push({ el: c, scale: W });
    clip(row.querySelector(".tlanes"), W);
  }
  SP.els.push({ el: $("#ruler"), scale: RW });
  clip($("#rulerClip"), RW);
  for (const k of TM_KINDS) if (TM.cans && TM.vis[k]) { SP.els.push({ el: TM.cans[k], scale: RW }); clip(TM.cans[k].parentNode, RW); }   // P4-8
  if (req.bitmaps) {
    const box = spBox(), inner = box.firstChild;
    Object.assign(box.style, { left: GEO.scL + GEO.waveX + "px", top: GEO.scT + "px", width: W + "px", height: GEO.scH + "px", display: "block" });
    [...inner.children].forEach((c, i) => {
      Object.assign(c.style, { left: i * W + "px", width: W + "px", height: GEO.scH + "px" });   // #435: each tile is the lane's own W-wide canvas, so the tiles sit W apart with no gap (they were scW wide at W apart: a laneW - scW dark column slid through the lane)
      c.getContext("bitmaprenderer").transferFromImageBitmap(req.bitmaps[i]);
    });
    req.bitmaps = null;
    if (GLQ.eng && GLQ.eng.canvas.style.visibility !== "hidden") {   // under the strip; drawn again when the strip ends
      GLQ.eng.canvas.style.visibility = "hidden";
      if (GLQ.eng.release) GLQ.eng.release();   // P4-1b: its buffers go while it is hidden (the footprint, item 2)
    }
    SP.leaving = 0;
    SP.els.push({ el: inner, scale: W, gl: true });
  }
  SP.s = s; SP.d = d; SP.key = req.key; SP.req = null; SP.geo = spGeo();
  spArm(now);   // P4-1b: the next strip of the grid is asked for SP_LEAD_S before it is due (spTick), not now: two strips
}               // of bitmaps were held for half a view each time
/* The strip from the transport model (the same model as the compositor playhead). The 2D els are written per frame
   (spWrite, from the rAF loop); only the GL strip keeps a compositor animation. */
function spArm(now) {
  const d = SP.d, s = SP.s, pos = AC.drawn, rate = AC.speed, t0 = pos - FOLLOW_FRAC * d;
  const dur = Math.max(0, (s + d - t0) / rate * 1000);   // the strip is good until the view's start reaches s + d
  for (const a of SP.anims) a.cancel();
  SP.anims = SP.els.filter(({ gl }) => gl).map(({ el, scale, gl }) => {
    const a = el.animate([{ transform: `translateX(${-(t0 - s) / d * scale}px)` }, { transform: `translateX(${-scale}px)` }], { duration: dur, easing: "linear", fill: "forwards" });
    a.startTime = now; a._gl = !!gl;
    /* #463: the model time the keyframe 0 shows (the playhead's position at arm) and the model span the keyframes
       cover. spFreeze maps a model time to currentTime through them: CP.at0 moves on when cpRate rebases the clock,
       the animation's axis does not. */
    a._pos0 = pos; a._span = s + d - t0;
    return a;
  });
  /* #463: the swap must not step the drawn playhead. spArm re-anchored the shared clock from AC.drawn; after a
     device position correction (the stall of an underrun catch-up) the model and the clock are apart (measured
     276 ms in the probe), and the re-anchor stepped the line by exactly that (gate10: nv jumps of 81-133 ms
     tagged 'compositor' at the swap). While the playhead animation runs at an unchanged speed, rebase the live
     seg in place instead (cpRate): the position stays continuous and the pr slews the clock to the model. A
     fresh anchor only without an animation (the frame path drew until now) or at a speed change. The seg covers
     the strip's span, so cpPos cannot clamp before the next swap. */
  if (CP.anim && CP.rate === rate) {
    const e = pos - cpPos(now);
    if (Math.abs(e) > CP_TOL_S) cpRate(e);
    const g = CP.segs[CP.segs.length - 1];
    if (g && g.end === Infinity) { g.dur = Math.max(g.dur, Math.max(0, (SP.s + d - cpPos(now)) / rate * 1000)); CP.dur = g.dur; }
  } else {
    cpEndSeg(now);
    Object.assign(CP, { at0: now, pos0: pos, rate, pr: 1, dur });
    CP.segs.push({ at0: now, pos0: pos, rate, pr: 1, dur, end: Infinity });
  }
  if (CP.segs.length > CP_KEEP) CP.segs.splice(0, CP.segs.length - CP_KEEP);
  spWrite(now);   // the 2D els show the strip in this task, before the redraw is painted
  spTimer(now);
}
function spLead(d) { return Math.min(d / 4, SP_LEAD_S * AC.speed); }   // song time before a swap at which the next strip is asked for
function spTimer(now) {   // the tick that asks for the next strip, or the one that swaps it in (the view's start reaches it)
  const d = SP.d, t0 = cpPos(now) - FOLLOW_FRAC * d, due = SP.s + d / 2 - (SP.req ? 0 : spLead(d));
  clearTimeout(SP.timer);
  SP.timer = setTimeout(requestFrame, Math.max(0, (due - t0) / AC.speed * 1000) + 1);
}
/* In the tick while a strip shows: the view of the strip now, the next strip, or the fallback. False: it fell back. */
function spTick(now) {
  const pos = AC.drawn;
  if (SP.frozen) return true;   // spFreeze (tests)
  if (!spCan(pos)) { spLeave("state"); return false; }
  if (spKey() !== SP.key) { spLeave("layout"); return false; }
  const d = SP.d, t0 = pos - FOLLOW_FRAC * d, next = SP.req;
  S.view = { t0, t1: t0 + d };   // the view the strip shows now; nothing is drawn (input, the overview and the counters use it)
  if (next && next.failed) { spLeave("failed"); return false; }
  if (t0 >= SP.s + d / 2 && next && next.done && next.s === SP.s + d / 2 && next.key === SP.key) { spApply(next, now); SP.n.swap++; }
  else if (t0 >= SP.s + d * 0.9) { SP.n.late++; spLeave("late"); return false; }
  else {
    if (!next && t0 >= SP.s + d / 2 - spLead(d)) { spRequest(SP.s + d / 2); spTimer(now); }
    const e = pos - cpPos(now);   // #463: slew the strip animations through their playback rate; re-arm only beyond CP_HARD_S
    if (CP.rate !== AC.speed || Math.abs(e) > CP_REANCHOR_S) spArm(now);   // the bound: a real discontinuity re-anchors
    else if (Math.abs(e) > CP_TOL_S) cpRate(e);
  }
  return true;
}
/* After a frame of the frame path (the view glides): ask for the first strip, or show it when it is here.
   only: show a strip that is here and fits (no request, no fallback); the tick calls it before the follow step, so a
   strip asked for ahead (spAhead) shows at the first tick of the glide, without a draw of the frame path. */
function spEnter(now, only) {
  const pos = AC.drawn;
  if (!spCan(pos) || now < SP.retryAt) { if (!only && !SP.on) spDrop(); return false; }
  const r = SP.req, d = viewDur(), t0 = S.view.t0;
  const fits = r && !r.failed && Math.abs(r.d - d) <= 1e-9 * d && r.key === spKey() && t0 >= r.s && t0 < r.s + d * 0.9;
  if (only) { if (!fits || !r.done) return false; }
  else if (r && r.failed) { spDrop(); SP.retryAt = now + 2000; return false; }
  else if (!fits) { spRequest(t0); return false; }
  else if (!r.done) return false;
  SP.on = true; SP.n.enter++; if (r.ahead) SP.n.ahead++;
  spApply(r, now);
  return true;
}
/* P4-1b: while the view stands still at the start of the song (smooth follow, the playhead left of its anchor), ask
   for the first strip SP_AHEAD_S before the glide starts. The glide then starts in the strip: no frames of the frame
   path (each drew every lane and kept the GL canvas's surfaces busy just before the strip took two more). */
function spAhead(now) {
  if (SP.on || S.followMode !== "smooth" || S.followHold || now < SP.retryAt || window.__DS_NO_AHEAD__) return;   // the flag: tests (the P4-1 entry)
  const pos = AC.drawn, v = S.view, d = viewDur(), still = cpStill(pos), anchor = v.t0 + FOLLOW_FRAC * d;
  if (!still || !still.show || v.t0 > 0 || still.until !== anchor || (anchor - pos) / AC.speed > SP_AHEAD_S) return;
  if (!spCan(anchor, true)) return;
  const r = SP.req;
  if (r && (r.failed || (Math.abs(r.d - d) <= 1e-9 * d && r.key === spKey() && r.s === v.t0))) return;
  spRequest(v.t0);
  SP.req.ahead = true;
}
function spGlFreeze() {   // P4-2: the GL strip that moved on after a leave stops where it is (the view stopped or jumped)
  if (!SP.glAnim) return;
  try { SP.glAnim.commitStyles(); } catch (e) { /* not in every engine */ }
  SP.glAnim.cancel(); SP.glAnim = null;
}
function spLeave(why) {
  spDrop();
  if (why === "seek" || why === "pause") spGlFreeze();   // P4-2 fix round: also after the strip left (a moving fallback)
  if (!SP.on) return;
  // P4-2 (P4-1b verifier minor 1): when the glide goes on over the same lanes (a mute or a solo, a late strip; not a seek, a
  // pause, a zoom, a scroll or a resize), the GL strip keeps moving with the transport model until #lanesGL shows the frame
  // path's draw. Frozen, it stood 2 to 4 frames while the hit lines moved on, and the wave then jumped 12 to 17 px.
  const glides = S.playing && AC.running && !AC.seeking && S.followMode === "smooth" && !S.followHold && !SP.frozen && why !== "seek" && why !== "pause" && SP.geo === spGeo()
    && AC.drawn - FOLLOW_FRAC * viewDur() + viewDur() < maxDur();   // P4-2 fix round: not where the view stops at the song end (followStep's clamp)
  SP.on = false; SP.frozen = false; SP.n.leave++; SP.why = why; PH.r = NaN;   // P4-1b fix round: PH.r as in cpStop
  clearTimeout(SP.timer);
  if (SP.glAnim) { SP.glAnim.cancel(); SP.glAnim = null; }
  for (const a of SP.anims) {   // #463: every animation commits its shown transform inline before the cancel, else the
    // canvas shows the stale inline transform of a page ago for a frame until the fallback draw lands (the lanes flash);
    // sizeLaneCanvas clears it again when a canvas is sized for a draw
    try { a.commitStyles(); } catch (e) { /* not in every engine, and not for a finished animation without fill */ }
    if (a._gl && glides) { SP.glAnim = a; continue; }
    a.cancel();
  }
  SP.anims = []; SP.els = [];
  for (const el of SP.clips || []) el.style.clipPath = "";
  SP.clips = [];
  cpEndSeg(performance.now());
  const eng = GLQ.eng;
  if (SP.box && SP.box.style.display === "block") { SP.leaving = eng && !eng.lost ? eng.stats.posted : -1; SP.leaveFrames = 0; SP.posted = null; }
  invalidate(F.SIZE | F.VIEW);   // the canvases back to one view, and everything drawn again (the GL canvas too)
}
function spGeo() { return [GEO.n, GEO.top, GEO.laneW, GEO.scW, GEO.scH, GEO.waveH, rowHKey(), GEO.dpr, viewDur()].join(","); }   // a glide over the same lanes
function glSized() {   // #lanesGL has the size of the lanes area again (where the placeholder takes the worker's frame size)
  const eng = GLQ.eng;
  return !eng || eng.lost || !GLQ.dims || !eng.size_ || (eng.canvas.width === eng.size_[0] && eng.canvas.height === eng.size_[1]);
}
function spLeftCheck() {   // in the frame after a fallback: hide the frozen GL strip once the GL canvas showed a new draw
  const eng = GLQ.eng, posted = SP.posted || (SP.posted = []);
  posted.push(eng ? eng.stats.posted : 0);   // P4-2: the draws posted until each frame of the fallback (below)
  if (SP.glAnim && S.view.t1 >= maxDur() - 1e-9) spGlFreeze();   // P4-2 fix round: the view reached the song end and stops
  if (SP.leaving > 0 && eng && !eng.lost && eng.stats.drawn <= SP.leaving) return;
  // P4-1b fix round: under the strip the worker's canvas was 1 x 1 (released). The placeholder (#lanesGL) shows a frame of
  // the worker some frames after its drawn answer when the worker is behind (Chromium: up to 9 frames, measured), so it
  // could still show the 1 x 1 frame: blank lanes. Where the placeholder takes the size of the frames it shows (GLQ.dims),
  // wait until it has the size of the lanes area again (glSized).
  // P4-2 (P4-1b verifier minor 2): no limit any more. After SP_SHOW_FRAMES frames the page showed #lanesGL even at 1 x 1
  // (a worker more than 1 s behind: blank lanes); now it asks for the draw again every SP_SHOW_FRAMES frames and waits on
  // (the frozen strip shows meanwhile).
  if (!glSized()) { if (++SP.leaveFrames % SP_SHOW_FRAMES === 0) glRepaint(); requestFrame(); return; }
  // P4-2: while the GL strip moves on (a glide), #lanesGL takes over only with a draw at most two frames old: the first
  // draw after the leave can be many frames old when the worker was behind (measured in Chromium: 19 px behind the view)
  const anim = SP.glAnim && SP.glAnim.playState === "running";
  if (anim && eng && !eng.lost && posted.length > 2 && eng.stats.drawn < posted[posted.length - 3]) { requestFrame(); return; }
  if (++SP.leaveFrames < 2) { requestFrame(); return; }   // the drawn answer comes before the canvas shows it
  SP.leaving = 0; SP.posted = null;
  const box = SP.box;
  if (SP.glAnim) { SP.glAnim.cancel(); SP.glAnim = null; }
  box.style.display = "none"; box.firstChild.style.transform = "";
  for (const c of box.firstChild.children) c.getContext("bitmaprenderer").transferFromImageBitmap(null);
  if (GLQ.eng) GLQ.eng.canvas.style.visibility = "";
}
/* Test hook: hold the strip as it is at page time t (the tests compare its pixels with a direct draw of that view);
   null goes on from the model. */
function spFreeze(t) {
  if (!SP.on) return false;
  if (t === null) { SP.frozen = false; spArm(performance.now()); return true; }
  SP.frozen = true; clearTimeout(SP.timer);
  /* #463: the GL strip's compositor animation keeps its own time axis (startTime at arm); CP.at0 moves on with every
     cpRate rebase, so t - CP.at0 is wrong. The keyframes map the strip's model span linearly: keyframe 0 at the arm's
     playhead position (a._pos0), keyframe 1 one view d later - the fraction follows the model position, not the clock. */
  for (const a of SP.anims) {
    const dur = a.effect.getTiming().duration;
    a.pause();
    a.currentTime = Math.max(0, Math.min(dur, (cpPos(t) - a._pos0) / a._span * dur));
  }
  spWrite(t, true);   // the 2D els hold their place at t from now on (no frame writes while frozen)
  return true;
}
function spSyncView() {   // input while a strip shows: the view of this moment (the ticks set it only about every 40 ms)
  if (!SP.on) return;
  const d = SP.d, t0 = cpPos(performance.now()) - FOLLOW_FRAC * d;
  S.view = { t0, t1: t0 + d };
}
for (const t of ["pointerdown", "mousedown", "mousemove", "wheel", "click", "dblclick", "contextmenu", "keydown"])
  window.addEventListener(t, spSyncView, { capture: true, passive: true });
/* P3-14b: counters. Main counter: seconds with 6 decimals, samples, Bars|Beats (bar.beat of the
   tempo map, #149) or Timecode (HH:MM:SS:FF at 30 frames per second from the sample position,
   #149); the sub counter shows the other format of the pair (seconds<->samples,
   Bars|Beats<->Timecode). The sample rate is the one of the selected track, else of the first
   track. The Timecode frame rate is 30, non drop: frames = floor(t * 30) over the whole project. */
const COUNTER_PAIRS = { seconds: "samples", samples: "seconds", barsbeats: "timecode", timecode: "barsbeats" };
function counterRate() {
  const t = (S.selTrack && S.tracks[S.selTrack]) || (S.order.length && S.tracks[S.order[0]]);
  return t && t.sr ? t.sr : 48000;
}
function counterBBText(bar, beat, ticks) {   // #149: the Pro Tools Bars|Beats shape, 960 ticks per beat
  return `${bar}| ${String(beat).padStart(2, " ")}| ${String(ticks).padStart(4, " ")}`;
}
function counterBarsBeats() {
  const m = TM.map;
  if (!m) {   // no map: the default one the export uses, 120 BPM 4/4, bar 1 at 0 s
    const total = S.playhead * 2;   // quarter-note beats at 120 BPM
    return counterBBText(Math.floor(total / 4) + 1, Math.floor(total % 4) + 1, Math.round((total % 1) * 960) % 960);
  }
  const g = tmGrid();
  if (!g || !g.t.length) return counterBBText(1, 1, 0);
  let i = tmLower(g.t, S.playhead) - 1;   // the beat at or before the playhead (#149)
  if (i < 0) return counterBBText(1, 1, 0);   // before bar 1: clamped to 1| 1| 000
  // the grid times accumulate (tmGrid), so a playhead on a beat can sit a hair below its
  // grid time: treat anything within a microsecond of the next beat as that beat (#149)
  if (i + 1 < g.t.length && g.t[i + 1] - S.playhead * 1 <= 1e-6) i++;
  const dt = (i + 1 < g.t.length ? g.t[i + 1] - g.t[i] : 0.5) || 0.5;
  const ticks = ((Math.round((S.playhead - g.t[i]) / dt * 960) % 960) + 960) % 960;
  return counterBBText(g.bar[i], g.beat[i], Math.max(0, ticks));
}
function counterTimecode() {
  const fps = 30, sr = counterRate();
  const t = Math.round(S.playhead * sr) / sr;   // the sample position of the playhead (#149)
  let frames = Math.floor(t * fps);
  const ff = frames % fps; frames = Math.floor(frames / fps);
  const ss = frames % 60; frames = Math.floor(frames / 60);
  const mm = frames % 60, hh = Math.floor(frames / 60);
  const two = n => String(n).padStart(2, "0");
  return `${two(hh)}:${two(mm)}:${two(ss)}:${two(ff)} tc`;
}
function counterTexts() {
  const secs = `${fmt(S.playhead, 6)} s`, smp = `${Math.round(S.playhead * counterRate())} smp`;
  const bb = counterBarsBeats(), tc = counterTimecode();
  const texts = { seconds: secs, samples: smp, barsbeats: bb, timecode: tc };
  const main = texts[SETTINGS().ui.counter] || secs;
  return [main, texts[COUNTER_PAIRS[SETTINGS().ui.counter] || "samples"]];
}
function setCounter(mode) {   // View > Main Counter: "seconds", "samples", "barsbeats" or "timecode" (#149)
  if (!(mode in COUNTER_PAIRS)) return;
  SETTINGS().ui.counter = mode; saveSettings();
  FR.head[2] = FR.head[3] = null; invalidate(F.HEAD);
  $("#timeDisplay").title = `Main counter (${mode}${mode === "timecode" ? ", 30 fps" : ""}). Click: exchange the counters (View > Main Counter > Exchange Counters)`;
  DSShell.sync();
}
function exchangeCounters() { setCounter(COUNTER_PAIRS[SETTINGS().ui.counter] || "seconds"); }

/* ---------------- canvas sizing ---------------- */
function laneWidth() { return GEO.laneW; }
/* P3-6b: the canvases of a lane keep a backing store only while the lane is in view or half a screen from it: each
   2D canvas of the 30-lane project cost 0.8 to 2.6 MB of graphics memory in the WebContent process (about 100 MB
   together). Other lanes hold a 1 x 1 canvas with the same CSS size and are drawn again when they come near.
   P3-14d: "near" comes from GEO (no layout read); a row that the observer has not measured yet is not near. */
function lanesNear() {
  const m = GEO.scH / 2, near = new Set();
  document.querySelectorAll("#lanesScroll .track").forEach(row => {
    const g = GEO.rows[row.dataset.name];
    if (g && g.bottom - GEO.top >= -m && g.top - GEO.top <= GEO.scH + m) near.add(row);
  });
  return near;
}
function sizeLaneCanvas(c, live) {
  const dpr = GEO.dpr, W = laneWidth(), hh = laneHeights(), row = c.closest(".track"), own = row && S.rowH && S.rowH[row.dataset.name];
  const vel = c.classList.contains("vel");   // #208: the velocity lane has its own height (VL.h), not the wave height
  const h = c.classList.contains("spike") ? hh.spike : vel ? VL.h : (own ? Math.round(hh.wave * own) : hh.wave);   // P3-14c; P4-27/#167: a row with its own factor
  c.style.width = W + "px";   // the CSS height comes from the variables of setTrackHeight (an inline height would win over them)
  c.style.transform = ""; c._sptf = "";   // #463: a lane sized for a draw is not on a strip (the rAF path's cache resets too)
  if (vel) c.style.height = h + "px";
  if (live) { c.width = W * dpr; c.height = h * dpr; c.getContext("2d").setTransform(dpr, 0, 0, dpr, 0, 0); }
  else { c.width = 1; c.height = 1; }
}
function syncLaneBuffers() {   // in the frame: a lane came near the view, or left it; true when one came near
  const near = lanesNear();
  let woke = false;
  document.querySelectorAll("#lanesScroll .track").forEach(row => {
    const live = near.has(row);
    if (live === (row._live !== false)) return;
    row._live = live; woke = woke || live;
    row.querySelectorAll("canvas.lane").forEach(c => sizeLaneCanvas(c, live));
  });
  return woke;
}
function sizeCanvases() {   // in the frame: every canvas to the sizes of GEO (writes only)
  const dpr = GEO.dpr, near = lanesNear();
  document.querySelectorAll("#lanesScroll .track").forEach(row => {
    row._live = near.has(row);
    row.querySelectorAll("canvas.lane").forEach(c => sizeLaneCanvas(c, row._live));
  });
  const ov = $("#overview"), rul = $("#ruler"), rw = GEO.rulerW;
  ov.width = GEO.ovW * dpr; ov.height = GEO.ovH * dpr;
  ov.getContext("2d").setTransform(dpr,0,0,dpr,0,0);
  rul.style.width = rw + "px"; rul.style.height = "22px";
  rul.style.transform = ""; rul._sptf = "";   // #150: the time ruler drops its strip transform with its size, like the lane canvases (sizeLaneCanvas)
  rul.width = rw * dpr; rul.height = 22 * dpr; rul.getContext("2d").setTransform(dpr,0,0,dpr,0,0);
  sizeMapCanvases();   // P4-8
  FR.sized = sizeKey();
}
function resizeCanvases() { invalidate(F.SIZE); }   // the next frame sizes and draws every canvas

/* ---------------- coordinates ---------------- */
/* P4-1b fix: the span to the nanosecond. A view is stored as {t0, t1}, and t1 - t0 of a view set as {t0, t0 + d} is d plus
   or minus an ulp for many t0; the follow step carried that noise on (8 became 7.999999999999999). Two draws of the same
   span then put a peak bin at a column edge on different sides (the strip against a direct draw). */
const viewDur = () => Math.round((S.view.t1 - S.view.t0) * 1e9) / 1e9;
const spanOf = v => Math.round((v.t1 - v.t0) * 1e9) / 1e9;   // the same for any view
function tToX(t, w) { return (t - S.view.t0) / viewDur() * w; }
function xToT(x, w) { return S.view.t0 + x / w * viewDur(); }
function maxDur() { return Math.max(10, ...S.order.map(n => S.tracks[n].duration)); }
function snapSample(t, name) { const sr = S.tracks[name].sr; return Math.round(t*sr)/sr; }
/* playhead snapping: within 10px on screen AND <= 30 ms of the nearest hit -> land exactly on it */
function snapPlayhead(t) {
  if (!S.snap) return t;
  const pxTime = viewDur() / laneWidth();          // seconds per screen pixel
  const thresh = Math.min(10 * pxTime, 0.030);
  let best = null, bestDt = thresh;
  for (const n of S.order) {   // W6: only the hits within the threshold (binary search)
    const evs = S.events[n] || [];
    for (const i of hitsIn(evs, t - thresh, t + thresh, dragSkip(n))) {
      const dt = Math.abs(evs[i].t - t);
      if (dt < bestDt) { bestDt = dt; best = evs[i].t; }
    }
  }
  return best !== null ? best : t;
}
/* per-track display gain: normalize so the track's own peak fills ~90% of half-height */
function peakAmpOf(p) {
  if (!p || !p.levels) return 1;
  const L = p.levels[65536] || p.levels[4096] || p.levels[256];
  if (!L || !L.max || !L.max.length) return 1;
  const mx = Math.max(...L.max), mn = Math.min(...L.min);   // E: negative peaks count too
  return Math.max(1e-5, mx, -mn);
}

/* ---------------- view navigation ---------------- */
function viewClamp(t0, t1) {
  const maxD = maxDur();
  const d = Math.min(maxD, Math.max(t1 - t0, 0.002));   // keep the requested span (zoom presets, pans
  t0 = Math.max(0, Math.min(t0, maxD - d));             // and follow near 0 used to shrink it) and
  return { t0, t1: t0 + d };                            // slide it inside [0, maxD]
}
function setView(t0, t1) {
  S.view = viewClamp(t0, t1); invalidate(F.VIEW); updateStatus();
}
function centerOn(t) {
  const d = viewDur();
  setView(t - d/2, t + d/2);
}
function zoomAround(anchorT, factor) {
  S.followHold = true;   // user interaction pauses follow until next play
  const d = viewDur();
  const nd = Math.min(maxDur(), Math.max(0.002, d*factor));
  setView(anchorT - nd/2, anchorT + nd/2);
}
function zoomAtX(x, w, factor) {   // P4 #145: Cmd+wheel and pinch; the time under lane x stays under x
  S.followHold = true;
  const a = xToT(x, w), f = x / w;
  const nd = Math.min(maxDur(), Math.max(0.002, viewDur()*factor));
  setView(a - f*nd, a - f*nd + nd);
}
function zoomPlayhead(factor) {
  if (S.playhead < S.view.t0 || S.playhead > S.view.t1) centerOn(S.playhead);
  zoomAround(S.playhead, factor);
}
function panView(dt) {
  S.followHold = true;   // user interaction pauses follow until next play
  const maxD = maxDur();
  const t0 = Math.max(0, Math.min(S.view.t0 + dt, maxD - viewDur()));
  setView(t0, t0 + viewDur());
}

/* ---------------- drawing ---------------- */
function redrawAll() {   // everything, now (tests and the self-test; the app invalidates and draws in the frame)
  FR.flags &= ~(F.VIEW | F.HEAD | F.PH | F.GL); FR.tracks.clear(); FR.sel = selKey();
  drawAll();
}
function drawAll() {
  drawRuler(); drawOverview();
  for (const name of S.order) {
    const row = rowOf(name);
    if (!row) continue;
    if (row._live === false && !GLQ.all) continue;   // far from the view: no canvas store (the GL lane is culled by its position)
    drawSpike(row.querySelector(".spike"), name);
    drawWave(row.querySelector(".wave"), name);
    if (VL.on) drawVelLane(row.querySelector(".vel"), name);   // P4-40f: the vel lane draws at open too, not only after edits
  }
  glFlush();   // P3-6a: the wave lanes queued by drawWave
  drawHead();
}
/* P3-14d: the hit lines of one track after an edit. With the GL engine the wave lane's 2D canvas holds only the hit
   lines, so the GL lane is not drawn again; the Canvas 2D path draws its wave lane again. False: no canvas store. */
function drawHits(name) {
  const row = rowOf(name);
  if (!row || row._live === false || !S.tracks[name]) return false;   // far from the view: drawn when it comes near
  const wave = row.querySelector(".wave");
  drawSpike(row.querySelector(".spike"), name);
  if (VL.on) drawVelLane(row.querySelector(".vel"), name);   // P4-40f: the velocity lane draws with the hits
  if (glEngine() && S.peaks[name]) {
    const [W, H] = laneSize(wave);
    wave.getContext("2d").clearRect(0, 0, W, H);
    drawEventLines(wave, name);
  } else drawWave(wave, name);
  return true;
}

function drawRuler() {
  const c = $("#ruler"), ctx = c.getContext("2d"), W = GEO.rulerW, H = 22;
  ctx.clearRect(0,0,W,H);
  ctx.fillStyle = "#1c1f26"; ctx.fillRect(0,0,W,H);
  const dur = viewDur();
  const steps = [0.001,0.002,0.005,0.01,0.02,0.05,0.1,0.2,0.5,1,2,5,10,30,60];
  const step = steps.find(s => s*W/dur > 60) || 60;
  // Pro Tools style: the major tick runs the full height, its label sits right of it with room above and beside
  ctx.strokeStyle = "#3a404c"; ctx.fillStyle = "#8a93a3"; ctx.font = "10px -apple-system, sans-serif"; ctx.textBaseline = "top";
  for (let t = Math.ceil(S.view.t0/step)*step; t <= S.view.t1; t += step) {
    const x = Math.round(tToX(t, W)) + 0.5;
    ctx.beginPath(); ctx.moveTo(x, 3); ctx.lineTo(x, H); ctx.stroke();
    ctx.fillText(step < 1 ? fmt(t,3) : fmt(t,1), x+5, 5);
    for (let k=1;k<5;k++){ const xx = Math.round(tToX(t + step*k/5, W)) + 0.5; ctx.beginPath(); ctx.moveTo(xx,H-5); ctx.lineTo(xx,H); ctx.stroke(); }
  }
  if (AU.win) {   // P4-40d: the loop of the audition
    const x0 = Math.max(0, tToX(AU.win.t0, W)), x1 = Math.min(W, tToX(AU.win.t1, W));
    if (x1 > x0) { ctx.fillStyle = "rgba(255, 184, 77, 0.45)"; ctx.fillRect(x0, 0, x1 - x0, H); ctx.fillStyle = "#ffb84d"; ctx.fillRect(x0, H - 3, x1 - x0, 3); }
  }
  if (LN.sel) {   // P4-6a: two small edge marks of the range selection
    ctx.fillStyle = "#4da3ff";
    for (const t of [LN.sel.t0, LN.sel.t1]) { const x = Math.round(tToX(t, W)); if (x >= -4 && x <= W + 4) { ctx.fillRect(x - 1, 0, 2, H); ctx.beginPath(); ctx.moveTo(x - 4, 0); ctx.lineTo(x + 4, 0); ctx.lineTo(x, 5); ctx.closePath(); ctx.fill(); } }
  }
  drawMapRulers();   // P4-8: the tempo, meter and Bars|Beats rulers share the x scale (and the strip)
}

/* ---------------- tempo and meter map (P4-8 stage C) ----------------
   The map lives in the service (tempo_map_get, tempo_map_set, tempo_map_detect, tempo_map_import_midi): the page holds a copy
   (TM.map) only to draw it. The page does no tempo logic beyond drawing: it walks the map to place the bar and beat lines
   (tmGrid) and sends every change as the whole new map. One change is one undo step: {type: "tempo", before, after}, as the
   re-pick does (tempo_map_set answers `previous`). Three rulers above the time ruler (Tempo, Meter, Bars|Beats; View menu)
   and the grid lines in the lanes (drawEventLines draws them first, so the hit lines lie over them). Each of the four
   states is an app setting (ruler_tempo, ruler_meter, ruler_beats, grid_lines); no value means "on" (P4-29; before: only once a map is set, not
   the default map). The rulers are 2D canvases drawn by drawRuler (the same x scale as the time ruler) and move with the
   follow strip like it. Double-click a ruler: add a step or a change (at the nearest beat, or bar), or edit the value in
   place; click a value and press Delete: remove it. */
const TM = { map: null, isDefault: true, grid: null, gridOn: false, vis: { tempo: false, meter: false, beats: false, markers: false }, cans: null,
  ui: { tempo: null, meter: null, beats: null, markers: null, grid: null }, sel: null, edit: null, chain: Promise.resolve(), pending: 0, mk: {}, widths: new Map(), mctx: null,
  col: { bar: "rgba(138,147,163,0.34)", beat: "rgba(138,147,163,0.14)", sub: "rgba(138,147,163,0.07)", text: "#cdd3de", dim: "#7d8593", accent: "#4da3ff", line: "#2c313c", orange: "#ff9f43" } };
const TM_KINDS = ["tempo", "meter", "beats", "markers"], TM_H = 18;
const TM_SETTING = { tempo: "ruler_tempo", meter: "ruler_meter", beats: "ruler_beats", markers: "ruler_markers", grid: "grid_lines" };
const TM_NAME = { tempo: "Tempo ruler", meter: "Meter ruler", beats: "Bars|Beats ruler", markers: "Markers ruler", grid: "Bar and beat grid" };
function tmShown(k) {   // P4-29: no saved choice = shown (the map rulers also with the default map; P4-40g: the markers ruler when the project has markers)
  const v = TM.ui[k];
  return v === null ? (k === "markers" ? MK.list.length > 0 : !!TM.map) : v;
}
function tmBpmText(b) { return String(Math.round(b * 100) / 100); }   // 94.99999208 shows as 95; the map keeps the exact value
const tmMeterText = m => `${m[0]}/${m[1]}`;
function tmColors() {   // outside the frame: getComputedStyle is a layout read
  const cs = getComputedStyle(document.documentElement), get = (n, d) => cs.getPropertyValue(n).trim() || d;
  TM.col = { bar: get("--grid-bar", TM.col.bar), beat: get("--grid-beat", TM.col.beat), sub: get("--grid-sub", TM.col.sub), text: get("--text", TM.col.text), dim: get("--dim", TM.col.dim),
    accent: get("--accent", TM.col.accent), line: get("--line", TM.col.line), orange: get("--orange", TM.col.orange) };
}
function tmInit() {   // at boot: the canvases and the events of the rulers
  if (TM.cans) return;
  TM.cans = {};
  const box = $("#mapRulers");
  for (const row of box.querySelectorAll(".mrow")) {
    const kind = row.dataset.ruler, c = row.querySelector("canvas.mcan");
    TM.cans[kind] = c;
    if (kind === "beats") {   // a click puts the playhead, as on the time ruler; P4-39d: the bar-1 marker is dragged or double-clicked
      c.addEventListener("mousedown", tmBeatsDown); c.addEventListener("dblclick", tmBeatsDouble);
      c.addEventListener("mousemove", ev => c.classList.toggle("onMarker", !!(TM.map && S.sid && tmBar1Hit(rulerXOf(ev)))));   // P4-40b: the resize cursor on the bar-1 marker
      c.addEventListener("mouseleave", () => c.classList.remove("onMarker"));
    } else {
      c.addEventListener("mousedown", ev => tmDown(kind, ev)); c.addEventListener("dblclick", ev => tmDouble(kind, ev));
      if (kind === "tempo") {   // #173: the drag cursor on a tempo step marker (the start marker is not draggable)
        c.addEventListener("mousemove", ev => c.classList.toggle("onMarker", !!(TM.map && S.sid && tmTempoHit(ev))));
        c.addEventListener("mouseleave", () => c.classList.remove("onMarker"));
      }
    }
  }
  const inp = $("#mapEdit");
  inp.addEventListener("keydown", ev => {
    if (ev.key === "Enter") { ev.preventDefault(); tmEditEnd(true); }
    else if (ev.key === "Escape") { ev.preventDefault(); ev.stopPropagation(); tmEditEnd(false); }
  });
  inp.addEventListener("blur", () => tmEditEnd(true));
  window.addEventListener("mousedown", ev => {   // a click outside the field applies it (a lane or the ruler calls preventDefault: no blur); outside the rulers it drops the selection
    if (TM.edit && ev.target !== inp) tmEditEnd(true);
    if (TM.sel && !(ev.target.closest && ev.target.closest("#mapRulers"))) { TM.sel = null; invalidate(F.VIEW); }
  }, true);
  tmColors();
}
async function tmLoad() {   // with the project: the copy of its map, then the rulers and the grid as the settings say
  tmEditEnd(false); TM.sel = null; TM.grid = null; TM.mk = {};
  TM.map = null; TM.isDefault = true;
  if (S.sid && B.tempoMapGet) {
    try { const r = await B.tempoMapGet(S.sid); TM.map = r.map; TM.isDefault = !!r.default; } catch (e) { /* no map: nothing is drawn */ }
  }
  await mkLoad();   // P4-40g: the marker list (also without a map)
  tmApplyUi(true);
}
function tmApplyUi(resize) {   // the rulers and the grid as tmShown says; draws again
  if (!TM.cans) return;
  for (const k of TM_KINDS) TM.vis[k] = tmShown(k);
  TM.gridOn = tmShown("grid");
  $("#mapRulers").classList.toggle("hidden", !TM_KINDS.some(k => TM.vis[k]));
  for (const row of document.querySelectorAll("#mapRulers .mrow")) row.classList.toggle("hidden", !TM.vis[row.dataset.ruler]);
  spLeave("map");
  invalidate((resize ? F.SIZE : 0) | F.VIEW);
  DSShell.sync();
}
function tmToggleUi(k) {   // View > Tempo Ruler, Meter Ruler, Bars|Beats Ruler, Show Bar and Beat Grid
  TM.ui[k] = !tmShown(k);
  if (B.appSettingsSet) B.appSettingsSet(TM_SETTING[k], TM.ui[k]).catch(e => setMsg(`settings: ${errText(e)}`));
  tmApplyUi(true);
  setMsg(`${TM_NAME[k]} ${TM.ui[k] ? "shown" : "hidden"}`);
}

/* The bar and beat times from the map: beat i is at g.t[i], in bar g.bar[i], beat number g.beat[i] (1 is the bar line);
   g.bars[b - 1] is the time of bar b. A beat is 4/den quarter notes; a tempo step inside a beat is handled exactly. The walk
   starts at bar 1 (bar1_s) and ends a second after the longest track. Cached until the map or the project changes. */
function tmGrid() {
  const m = TM.map;
  if (!m) return null;
  if (TM.grid) return TM.grid;
  const end = maxDur() + 1, steps = m.steps, changes = m.changes, g = { t: [], bar: [], beat: [], bars: [] };
  let t = m.bar1_s, bar = 1, si = 0, ci = 0, meter = m.meter;
  while (si < steps.length && steps[si].t <= t) si++;
  let bpm = si > 0 ? steps[si - 1].bpm : m.bpm;
  while (t <= end && g.t.length < 400000) {
    while (ci < changes.length && changes[ci].bar <= bar) meter = changes[ci++].meter;
    g.bars.push(t);
    for (let b = 1; b <= meter[0] && t <= end; b++) {
      g.t.push(t); g.bar.push(bar); g.beat.push(b);
      let rem = 4 / meter[1];   // quarter notes to the next beat
      while (rem > 1e-12) {
        const stepT = si < steps.length ? steps[si].t : Infinity, dt = rem * 60 / bpm;
        if (t + dt <= stepT) { t += dt; rem = 0; }
        else { rem -= (stepT - t) * bpm / 60; t = stepT; bpm = steps[si++].bpm; }
      }
    }
    bar++;
  }
  return (TM.grid = g);
}
function tmLower(a, x) { let lo = 0, hi = a.length; while (lo < hi) { const mid = (lo + hi) >> 1; if (a[mid] < x) lo = mid + 1; else hi = mid; } return lo; }
function tmNearest(a, x) { const i = tmLower(a, x); return i > 0 && (i >= a.length || x - a[i - 1] <= a[i] - x) ? i - 1 : Math.min(i, a.length - 1); }
function tmBpmAt(t) { const m = TM.map; let b = m.bpm; for (const s of m.steps) { if (s.t <= t) b = s.bpm; else break; } return b; }
function tmMeterAt(bar) { const m = TM.map; let v = m.meter; for (const c of m.changes) { if (c.bar <= bar) v = c.meter; else break; } return v; }

/* The grid lines of one lane, drawn before the hit lines (drawEventLines). Beat lines are dropped under 6 px apart, bar lines under 4.
   A beat wider than 24 px gets its 8th line, wider than 48 px its 8th and 16th lines (TM.col.sub, fainter than the beat lines). */
function drawGridLines(ctx, W, H) {
  if (!TM.gridOn || !TM.map) return;
  const g = tmGrid();
  if (!g || !g.t.length) return;
  const d = viewDur(), pad = 2 * d / W, a = S.view.t0 - pad, b = S.view.t1 + pad;
  const i0 = tmLower(g.t, a), i1 = tmLower(g.t, b), nb = i1 - i0;
  const bars = tmLower(g.bars, b) - tmLower(g.bars, a);
  if (nb <= 0) return;
  const beats = W / nb >= 6, barLines = !bars || W / bars >= 4;
  if (!beats && !barLines) return;
  ctx.save(); ctx.lineWidth = 1; ctx.globalAlpha = 1;
  if (beats) {
    ctx.strokeStyle = TM.col.beat; ctx.beginPath();
    for (let i = i0; i < i1; i++) if (g.beat[i] !== 1) { const x = Math.round(tToX(g.t[i], W)) + 0.5; ctx.moveTo(x, 0); ctx.lineTo(x, H); }
    ctx.stroke();
    ctx.strokeStyle = TM.col.sub; ctx.beginPath();   // the subdivisions of each beat by its own width on the lane (a step inside a beat changes it)
    for (let i = i0; i < i1 && i + 1 < g.t.length; i++) {
      const x0 = tToX(g.t[i], W), x1 = tToX(g.t[i + 1], W), n = x1 - x0 > 48 ? 4 : x1 - x0 > 24 ? 2 : 1;
      for (let k = 1; k < n; k++) { const x = Math.round(x0 + (x1 - x0) * k / n) + 0.5; ctx.moveTo(x, 0); ctx.lineTo(x, H); }
    }
    ctx.stroke();
  }
  if (barLines) {
    ctx.strokeStyle = TM.col.bar; ctx.beginPath();
    for (let i = i0; i < i1; i++) if (g.beat[i] === 1) { const x = Math.round(tToX(g.t[i], W)) + 0.5; ctx.moveTo(x, 0); ctx.lineTo(x, H); }
    ctx.stroke();
  }
  ctx.restore();
}

/* The rulers. Markers (a tempo step, a meter change) are lines with the value to their right. */
function tmMarkers(kind) {   // cached with the grid (tmLoad clears both); P4-40g: the review markers come from the service
  if (kind === "markers") return MK.items;
  if (!TM.map) return [];
  return TM.mk[kind] || (TM.mk[kind] = tmMarkersBuild(kind));
}
function tmMarkersBuild(kind) {
  const m = TM.map, out = [];
  if (kind === "tempo") {
    out.push({ id: "start", i: -1, t: 0, text: tmBpmText(m.bpm), del: false });
    m.steps.forEach((s, i) => out.push({ id: "s" + i, i, t: s.t, text: tmBpmText(s.bpm), del: true }));
  } else {
    const g = tmGrid();
    out.push({ id: "start", i: -1, t: g && g.bars.length ? g.bars[0] : m.bar1_s, text: tmMeterText(m.meter), del: false });
    m.changes.forEach((c, i) => { const t = g && g.bars[c.bar - 1]; if (t !== undefined) out.push({ id: "c" + i, i, bar: c.bar, t, text: tmMeterText(c.meter), del: true }); });
  }
  return out;
}
function tmTextW(text) {
  let w = TM.widths.get(text);
  if (w === undefined) {
    if (!TM.mctx) { TM.mctx = document.createElement("canvas").getContext("2d"); TM.mctx.font = "10px -apple-system, sans-serif"; }
    TM.widths.set(text, w = Math.ceil(TM.mctx.measureText(text).width));
    if (TM.widths.size > 500) TM.widths.clear();
  }
  return w;
}
/* P4-40b: the lane cursor says what a click does. #lanesScroll carries one class: curAdd (Option held: a click adds a hit)
   or curDelete (Option+Shift held over a hit that can be deleted: a click deletes it). The classes follow the keys
   (keydown and keyup of Option and Shift, also the modifiers of every mouse move) and the pointer; the CSS draws the cursors. */
const CUR = { alt: false, shift: false, del: false };
function curApply() {
  const sc = document.getElementById("lanesScroll"); if (!sc) return;
  sc.classList.toggle("curAdd", CUR.alt && !CUR.shift);
  sc.classList.toggle("curDelete", CUR.alt && CUR.shift && CUR.del);
}
function curMods(ev) { CUR.alt = !!ev.altKey; CUR.shift = !!ev.shiftKey; curApply(); }
function curOver(del, ev) { CUR.del = del; if (ev) { CUR.alt = !!ev.altKey; CUR.shift = !!ev.shiftKey; } curApply(); }   // del: the pointer is over a hit that Delete can remove
for (const t of ["keydown", "keyup", "mousemove"]) window.addEventListener(t, ev => { if (ev.target !== window) curMods(ev); }, { capture: true, passive: true });
window.addEventListener("blur", () => { CUR.alt = CUR.shift = false; curApply(); });
function tmHit(kind, px) {   // the marker under x (ruler coordinates), or null
  for (const it of tmMarkers(kind)) {
    const x = tToX(it.t, GEO.rulerW);
    if (px >= x - 4 && px <= x + tmTextW(it.text) + 8) return it;
  }
  return null;
}
function tmTempoHit(ev) {   // #173: the draggable tempo step under the pointer, or null (the start marker is not a step)
  if (!TM.map || !S.sid) return null;
  const it = tmHit("tempo", rulerXOf(ev));
  return it && it.del ? it : null;
}
function sizeMapCanvases() {   // in the frame (and for a strip): the visible canvases to GEO.rulerW; writes only
  if (!TM.cans) return;
  const dpr = GEO.dpr, rw = GEO.rulerW;
  for (const k of TM_KINDS) {
    if (!TM.vis[k]) continue;
    const c = TM.cans[k];
    c.style.width = rw + "px"; c.style.height = TM_H + "px";
    c.style.transform = ""; c._sptf = "";   // #150: a map ruler sized for a draw is not on a strip (the stale strip transform would shift the draw off the clip)
    c.width = rw * dpr; c.height = TM_H * dpr; c.getContext("2d").setTransform(dpr, 0, 0, dpr, 0, 0);
  }
}
function drawMapRulers() {
  if (!TM.cans) return;
  for (const k of TM_KINDS) if (TM.vis[k]) { if (k === "beats") drawBeatsRuler(TM.cans[k]); else drawMarkerRuler(k, TM.cans[k]); }
}
function drawMarkerRuler(kind, c) {
  const ctx = c.getContext("2d"), W = GEO.rulerW, H = TM_H, col = TM.col;
  ctx.clearRect(0, 0, W, H);
  if (kind === "markers") { if (!MK.items.length) return; }   // P4-40g: the review markers, independent of the map
  else if (!TM.map) return;
  ctx.font = "10px -apple-system, sans-serif"; ctx.textBaseline = "middle"; ctx.lineWidth = 1;
  const mk = tmMarkers(kind), selId = TM.sel && TM.sel.kind === kind ? TM.sel.id : null;
  for (const it of mk) {
    const x = Math.round(tToX(it.t, W)) + 0.5;
    if (x > W) break;
    const on = it.id === selId;
    ctx.strokeStyle = on ? col.accent : (kind === "markers" ? col.orange : col.dim);
    ctx.fillStyle = on ? col.accent : col.text;
    ctx.beginPath(); ctx.moveTo(x, 2); ctx.lineTo(x, H); ctx.stroke();
    if (kind === "markers") {   // a flag: the marker's note, or its number
      ctx.beginPath(); ctx.moveTo(x, 2); ctx.lineTo(x + 5, 2); ctx.stroke();
    }
    ctx.fillText(it.text, x + 4, H / 2);
  }
  /* #150: the value in force at the left edge of each view the canvas draws. One view: the label at x 3. Under the
     follow strip the canvas holds two views (spApply, SP.drawing): a label only at x 3 would glide off-screen with
     the first view, so the second view's left edge gets its own carry label too (the last marker left of that edge). */
  if (kind !== "markers") {
    const half = SP.drawing ? W / 2 : W;
    for (let vx = 0; vx < W; vx += half) {
      let c2 = null, f2 = null;
      for (const it of mk) {
        const x = Math.round(tToX(it.t, W)) + 0.5;
        if (x >= vx + half) break;
        if (x < vx) { c2 = it; continue; }
        if (f2 === null) f2 = x;
      }
      if (c2 && (f2 === null || f2 - vx > tmTextW(c2.text) + 12)) { ctx.fillStyle = col.dim; ctx.fillText(c2.text, vx + 3, H / 2); }
    }
  }
}
function drawBeatsRuler(c) {
  const ctx = c.getContext("2d"), W = GEO.rulerW, H = TM_H, col = TM.col;
  ctx.clearRect(0, 0, W, H);
  const g = TM.map && tmGrid();
  if (!g || !g.t.length) return;
  const d = viewDur(), pad = 40 * d / W, a = S.view.t0 - pad, b = S.view.t1 + pad;
  const i0 = tmLower(g.t, a), i1 = tmLower(g.t, b), nb = i1 - i0;
  if (nb <= 0) return;
  const bars = Math.max(1, tmLower(g.bars, b) - tmLower(g.bars, a)), pxBar = W / bars, pxBeat = W / nb;
  let k = 1;
  for (const s of [1, 2, 4, 5, 8, 10, 16, 20, 25, 50, 100, 200, 500, 1000]) { k = s; if (pxBar * s >= 44) break; }
  ctx.font = "10px -apple-system, sans-serif"; ctx.textBaseline = "middle"; ctx.lineWidth = 1;
  const small = [], bigs = [], labels = [];
  for (let i = i0; i < i1; i++) {
    const x = Math.round(tToX(g.t[i], W)) + 0.5;
    if (g.beat[i] === 1) {
      if (pxBar >= 4 || (g.bar[i] - 1) % k === 0) bigs.push(x);
      if ((g.bar[i] - 1) % k === 0) labels.push([x, String(g.bar[i])]);
    } else if (pxBeat >= 5) {
      small.push(x);
      if (pxBeat >= 44) labels.push([x, `${g.bar[i]}|${g.beat[i]}`, true]);
    }
  }
  ctx.strokeStyle = col.line; ctx.beginPath();
  for (const x of small) { ctx.moveTo(x, H - 5); ctx.lineTo(x, H); }
  ctx.stroke();
  ctx.strokeStyle = col.dim; ctx.beginPath();
  for (const x of bigs) { ctx.moveTo(x, 2); ctx.lineTo(x, H); }
  ctx.stroke();
  for (const [x, text, dim] of labels) { ctx.fillStyle = dim ? col.dim : col.text; ctx.fillText(text, x + 4, H / 2); }
  const x1 = Math.round(tToX(g.bars[0], W)) + 0.5;   // P4-39d: the bar-1 marker (drag it, or double-click it for its time)
  if (x1 >= 0 && x1 <= W) { ctx.strokeStyle = col.accent; ctx.beginPath(); ctx.moveTo(x1, 0); ctx.lineTo(x1, H); ctx.stroke(); }
}

/* Selecting, adding, editing and deleting. */
function tmDown(kind, ev) {
  if (ev.button !== 0) return;
  const it = tmHit(kind, ev.clientX - GEO.rulerX);
  ev.preventDefault();
  if (kind === "markers" && !it) return rulerDown(ev);   // P4-40g: a click on the ruler puts the playhead
  if (TM.edit) tmEditEnd(true);
  TM.sel = it ? { kind, id: it.id, i: it.i, del: it.del } : null;
  if (it && S.sel) { S.sel = null; updateStatus(); }
  if (it && kind === "tempo" && it.del && TM.map && S.sid) tmTempoDrag(it, ev);   // #173: a step marker drags
  invalidate(F.VIEW);
}
/* #173: a drag of a tempo step marker moves that step by the pointer delta (no snap: a drag of 200 ms
   moves the step 200 ms), clamped between the neighbours with the map's 1 ms gap (MIN_STEP_GAP_S of the
   service; the first step stays after 0). The preview map redraws the ruler, the bars and the grid at
   once (TM.grid and TM.mk cleared, like the bar-1 drag); the drop is one tmCommit, so one undo step.
   The start marker is the tempo in force, not a change: not draggable (as Delete refuses it). */
function tmTempoDrag(it, ev) {
  const orig = TM.map, i = it.i, cx0 = ev.clientX, px0 = rulerXOf(ev), gap = 0.001;
  let t = it.t, moved = false, copy = null;
  const lo = i > 0 ? orig.steps[i - 1].t + gap : gap, hi = i + 1 < orig.steps.length ? orig.steps[i + 1].t - gap : Infinity;
  const show = tt => {
    if (tt === orig.steps[i].t) { if (TM.map !== orig) { TM.map = orig; TM.grid = null; TM.mk = {}; invalidate(F.VIEW); } return; }
    if (!copy) copy = JSON.parse(JSON.stringify(orig));
    copy.steps[i].t = tt;
    TM.map = copy; TM.grid = null; TM.mk = {}; invalidate(F.VIEW);
  };
  const move = e2 => {
    if (!moved && Math.abs(e2.clientX - cx0) < 3) return;
    moved = true;
    t = Math.min(hi, Math.max(lo, it.t + xToT(rulerXOf(e2), GEO.rulerW) - xToT(px0, GEO.rulerW)));
    show(t);
  };
  const up = () => {
    window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up);
    show(orig.steps[i].t);   // the stored map until the service has the new one
    if (moved && t !== orig.steps[i].t) {
      const next = JSON.parse(JSON.stringify(orig));
      next.steps[i].t = t;
      tmCommit(next, `tempo step ${tmBpmText(orig.steps[i].bpm)} BPM at ${fmt(t, 3)} s`);
    }
  };
  window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
}
function tmDouble(kind, ev) {
  if (!S.sid) return;
  if (kind !== "markers" && !TM.map) return;
  ev.preventDefault();
  const px = ev.clientX - GEO.rulerX, t = xToT(px, GEO.rulerW), g = tmGrid();
  let it = tmHit(kind, px);
  if (!it && kind === "markers") {   // P4-40g: a double-click adds a marker (the field takes its optional note)
    it = { id: "new", i: -1, t: Math.max(0, t), text: "", add: true };
  } else if (!it && kind === "tempo") {
    const bt = g && g.t.length ? g.t[tmNearest(g.t, t)] : Math.max(0, t), at = Math.round(bt * 1e6) / 1e6;
    it = tmMarkers("tempo").find(x => Math.abs(x.t - at) < 0.0005);   // a step is there already: edit it
    if (!it) it = { id: "new", i: -1, t: at, text: tmBpmText(tmBpmAt(at)), add: true };
  } else if (!it && kind === "meter") {
    if (!g || !g.bars.length) return;
    const n = tmNearest(g.bars, t) + 1;
    it = tmMarkers("meter").find(x => x.bar === n || (n === 1 && x.id === "start"));
    if (!it) it = { id: "new", i: -1, bar: n, t: g.bars[n - 1], text: tmMeterText(tmMeterAt(n)), add: true };
  }
  if (it) tmEditStart(kind, it);
}
function tmEditStart(kind, it) {
  tmEditEnd(false);
  spLeave("map");   // a strip would move the markers away from the field
  const inp = $("#mapEdit"), row = document.querySelector(`#mapRulers .mrow[data-ruler="${kind}"]`);
  const x = tToX(it.t, GEO.rulerW), box = $("#mapRulers").getBoundingClientRect().left;
  inp.style.left = Math.round(GEO.rulerX - box + Math.min(Math.max(2, x), GEO.rulerW - 70)) + "px";
  inp.style.top = row.offsetTop + 1 + "px";
  inp.value = it.raw !== undefined ? it.raw : it.text; inp.classList.remove("hidden");
  TM.edit = { kind, it, done: false };
  inp.focus(); inp.select();
}
function tmEditEnd(commit) {
  const e = TM.edit;
  if (!e || e.done) return;
  e.done = true; TM.edit = null;
  const inp = $("#mapEdit"), v = inp.value.trim();
  inp.classList.add("hidden");
  if (!commit || (e.kind !== "markers" && !TM.map)) return;
  if (v === e.it.text && !e.it.add && e.it.raw === undefined) return;   // unchanged: the exact imported value stays
  if (e.kind === "beats") {   // P4-39d: the time of bar 1 in seconds
    const s = Number(v.replace(/\s*s$/, ""));
    if (!v || !Number.isFinite(s) || s < 0) { setMsg("bar 1: type a time in seconds, for example 2.025"); return; }
    return void tmSetBar1(s);
  }
  if (e.kind === "markers") {   // P4-40g: the note of a marker; an empty text is no note
    const list = MK.list.slice(), it = e.it;
    if (it.add) {
      const entry = { t: it.t };
      if (v) entry.note = v;
      list.push(entry);
      return void mkCommit(list, `marker at ${fmt(it.t, 3)} s${v ? `: ${v}` : ""}`, it.t);
    }
    const cur = list[it.i];
    if (!cur) return;
    if (v) cur.note = v; else delete cur.note;
    return void mkCommit(list, `marker at ${fmt(cur.t, 3)} s${v ? `: ${v}` : ": no note"}`, cur.t);
  }
  const next = JSON.parse(JSON.stringify(TM.map)), it = e.it;
  if (e.kind === "tempo") {
    const b = Number(v);
    if (!v || !Number.isFinite(b) || b <= 0) { setMsg("tempo: type a number of BPM, for example 95"); return; }
    if (it.add && tmBpmText(b) === it.text) { setMsg("tempo: the same as the tempo there, no step added"); return; }   // a step equal to the value in force changes nothing
    if (it.id === "start") {
      next.bpm = b;
      if (next.steps.length) return void tmCommit(next, `tempo ${tmBpmText(b)} BPM, bar 1 stays: the map has steps`);
      // #468: a typed start tempo on a map without steps re-fits bar 1 to the hits (the phase of
      // the kick and snare hits at the new tempo, tempo_map_refit_bar1): the grid sits on them
      // again. The commit is one step, so one undo restores the tempo and bar 1 together.
      const sid0 = S.sid, was = TM.map.bar1_s;
      return void (async () => {
        try {
          const r = await B.tempoMapRefitBar1(sid0, b);
          if (sid0 !== S.sid) return;
          if (Math.abs(r.bar1_s - was) < 1e-9) return void tmCommit(next, `tempo ${tmBpmText(b)} BPM, bar 1 stays at ${fmt(was, 3)} s`);
          next.bar1_s = r.bar1_s;
          return void tmCommit(next, `tempo ${tmBpmText(b)} BPM, bar 1 moved from ${fmt(was, 3)} s to ${fmt(r.bar1_s, 3)} s`);
        } catch (e) { return void tmCommit(next, `tempo ${tmBpmText(b)} BPM, bar 1 stays: ${errText(e)}`); }
      })();
    }
    if (it.add) { next.steps.push({ t: it.t, bpm: b }); next.steps.sort((p, q) => p.t - q.t); }
    else next.steps[it.i].bpm = b;
    return void tmCommit(next, it.add ? `tempo step ${tmBpmText(b)} BPM at ${fmt(it.t, 2)} s` : `tempo ${tmBpmText(b)} BPM`);
  }
  const mm = /^(\d{1,3})\s*\/\s*(\d{1,3})$/.exec(v);
  if (!mm) { setMsg("meter: type beats and note value, for example 7/8"); return; }
  const meter = [Number(mm[1]), Number(mm[2])];
  if (it.add && tmMeterText(meter) === it.text) { setMsg("meter: the same as the meter there, no change added"); return; }
  if (it.id === "start") next.meter = meter;
  else if (it.add) { next.changes.push({ bar: it.bar, meter }); next.changes.sort((p, q) => p.bar - q.bar); }
  else next.changes[it.i].meter = meter;
  tmCommit(next, it.add ? `meter ${v} from bar ${it.bar}` : `meter ${v}`);
}
function tmDeleteSel() {   // Delete (or Edit > Delete Hit) with a marker selected: true when it took the key
  const s = TM.sel;
  if (s && s.kind === "markers") {   // P4-40g: a selected review marker
    const m = MK.list[s.i];
    if (!m) return false;
    const list = MK.list.slice();
    list.splice(s.i, 1);
    TM.sel = null;
    mkCommit(list, `marker at ${fmt(m.t, 3)} s deleted`);
    return true;
  }
  if (!s || !TM.map) return false;
  if (!s.del) { setMsg(`the start ${s.kind === "tempo" ? "tempo" : "meter"} stays: edit its value`); return true; }
  const next = JSON.parse(JSON.stringify(TM.map));
  if (s.kind === "tempo") next.steps.splice(s.i, 1); else next.changes.splice(s.i, 1);
  TM.sel = null;
  tmCommit(next, s.kind === "tempo" ? "tempo step deleted" : "meter change deleted");
  return true;
}
/* P4-39d: bar 1 by hand. Track > Set Bar 1 at Playhead (or at the selected hit), a double-click on the bar-1 marker of the
   Bars|Beats ruler (a field with the time in seconds) and a drag of that marker move bar 1 only: the tempo, the steps, the meter
   and the meter changes stay. One undo step through tmCommit, as every tempo map edit. */
function tmSetBar1(t) {
  if (!TM.map || !S.sid) { setMsg("bar 1: open a project first"); return Promise.resolve(false); }
  const next = JSON.parse(JSON.stringify(TM.map));
  next.bar1_s = Math.max(0, t);
  return tmCommit(next, `bar 1 at ${fmt(next.bar1_s, 3)} s`);
}
function setBar1AtPlayhead() {   // Track > Set Bar 1 at Playhead: the selected hit's time when a hit is selected, else the playhead
  if (!needProject()) return null;
  const e = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  return tmSetBar1(e ? e.t : S.playhead);
}
function setBar1AtAudioStart() {   // #468: Track > Set Bar 1 at Audio Start: bar 1 at 0 s, the DAW convention
  if (!needProject()) return null;
  return tmSetBar1(0);
}
function tmBar1Hit(px) {   // true when x (ruler coordinates) is on the bar-1 marker: its line and its label "1"
  if (!TM.map) return false;
  const x = tToX(TM.map.bar1_s, GEO.rulerW);
  return px >= x - 4 && px <= x + tmTextW("1") + 8;
}

/* ---------------- review markers (P4-40g) ----------------
   The markers live in the project document (the `markers` pass-through key) and come from the service
   (markers_get, markers_set): the page holds a copy (MK.list) to draw them, like the map (TM.map).
   markers_set replaces the whole list and answers `previous`, so one more call undoes it
   (docs/agent-api.md). M sets a marker at the selected hit or the playhead; the Markers ruler shows
   them (a click puts the playhead, a click on a flag selects it, Delete removes it, a double-click
   edits its note); Option+M and Option+Shift+M jump to the next and the previous marker. */
const MK = { list: [], items: [], chain: Promise.resolve(), pending: 0 };
function mkItems() {   // the ruler items: {id, i, t, text (the note, or the number), raw (the note), del}
  return MK.list.map((m, i) => { const note = typeof m.note === "string" ? m.note : ""; return { id: "m" + i, i, t: m.t, text: note || `M${i + 1}`, raw: note, del: true }; });
}
function mkQueue(fn) {   // every write runs in order (the tempo chain pattern)
  MK.pending++;
  return (MK.chain = MK.chain.then(fn).finally(() => { MK.pending--; }));
}
async function mkLoad() {   // with the project, and after every write: the copy of the marker list
  MK.list = [];
  if (S.sid && B.markersGet) {
    try { const r = await B.markersGet(S.sid); MK.list = Array.isArray(r.markers) ? r.markers : []; } catch (e) { /* no markers */ }
  }
  MK.items = mkItems();
  if (TM.sel && TM.sel.kind === "markers" && TM.sel.i >= MK.list.length) TM.sel = null;
  tmApplyUi(false);   // the ruler shows when the project has markers, until a toggle stores the choice
  invalidate(F.VIEW);
}
function mkCommit(list, what, selT) {   // the whole new list goes to the service; selT: the marker to select after
  const sid = S.sid;
  return mkQueue(async () => {
    if (sid !== S.sid) return false;
    try {
      const r = await B.markersSet(sid, list);
      if (sid !== S.sid) return false;
      await mkLoad();
      if (r.changed) { setDirtyState(true); setMsg(what); } else setMsg(`${what}: no change`);
      if (selT !== undefined) {   // the marker at selT stays selected (its index moved with the sort)
        const i = MK.list.findIndex(m => Math.abs(m.t - selT) < 1e-9);
        if (i >= 0) { TM.sel = { kind: "markers", id: "m" + i, i, del: true }; invalidate(F.VIEW); }
      }
      return true;
    } catch (e) { setMsg(`${what} refused: ${errText(e)}`); return false; }
  });
}
function markerAdd(t, note) {   // one more marker at t (the service stores the list sorted)
  if (!needProject()) return Promise.resolve(false);
  const entry = { t: Math.max(0, t) };
  if (note) entry.note = note;
  const what = `marker at ${fmt(t, 3)} s${note ? `: ${note}` : ""}`;
  return mkCommit(MK.list.concat([entry]), what, t);
}
function markerAtSelection() {   // M: the selected hit's time, else the playhead (as Set Bar 1 at Playhead)
  if (!needProject()) return Promise.resolve(false);
  const e = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  return markerAdd(e ? e.t : S.playhead);
}
function markerJump(dir) {   // Option+M and Option+Shift+M: the next or the previous marker from the playhead
  if (!MK.list.length) { setMsg(dir > 0 ? "no next marker" : "no previous marker"); return; }
  const at = S.playhead;
  const m = dir > 0 ? MK.list.find(x => x.t > at + 1e-9) : MK.list.slice().reverse().find(x => x.t < at - 1e-9);
  if (!m) { setMsg(dir > 0 ? "no next marker" : "no previous marker"); return; }
  S.playhead = Math.min(m.t, maxDur());
  centerOn(S.playhead);
  updateStatus();
  setMsg(`marker at ${fmt(m.t, 3)} s${typeof m.note === "string" && m.note ? `: ${m.note}` : ""}`);
}
function tmBeatsDown(ev) {
  if (ev.button !== 0 || !S.sid || !tmBar1Hit(rulerXOf(ev))) return rulerDown(ev);
  ev.preventDefault();
  if (TM.edit) tmEditEnd(true);
  const orig = TM.map, x0 = ev.clientX;
  let t = orig.bar1_s, moved = false;
  const show = bar1 => { TM.map = bar1 === orig.bar1_s ? orig : { ...orig, bar1_s: bar1 }; TM.grid = null; TM.mk = {}; invalidate(F.VIEW); };
  const move = e2 => {
    if (!moved && Math.abs(e2.clientX - x0) < 3) return;
    moved = true;
    t = Math.max(0, snapPlayhead(Math.min(xToT(rulerXOf(e2), GEO.rulerW), maxDur())));
    show(t);
  };
  const up = () => {
    window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up);
    show(orig.bar1_s);   // the stored map until the service has the new one
    if (moved && t !== orig.bar1_s) tmSetBar1(t);
  };
  window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
}
function tmBeatsDouble(ev) {
  if (!TM.map || !S.sid || !tmBar1Hit(rulerXOf(ev))) return;
  ev.preventDefault();
  tmEditStart("beats", { id: "bar1", i: -1, t: TM.map.bar1_s, text: fmt(TM.map.bar1_s, 3) });
}
/* One change: the whole new map goes to the service, the answer's `previous` is the undo state. Changes are done in order. */
function tmQueue(fn) {   // every write of the map runs in order; undo and redo wait while one is on its way (tmUndoWaits)
  TM.pending++;
  return (TM.chain = TM.chain.then(fn).finally(() => { TM.pending--; }));
}
function tmUndoWaits() {   // like hgUndoWaits: an undo while a change is stored would take the wrong step
  if (!TM.pending) return false;
  setMsg("a tempo map change is being stored: undo and redo wait for it");
  return true;
}
function tmCommit(next, what) {
  const sid = S.sid;
  return tmQueue(async () => {
    if (sid !== S.sid) return false;
    try {
      const r = await B.tempoMapSet(sid, next);
      if (sid !== S.sid) return false;
      if (!r.changed) { setMsg(`${what}: no change`); return false; }
      pushUndo({ type: "tempo", sid, before: r.previous || {}, after: r.map, what });
      await tmLoad(); setDirtyState(true); setMsg(what);
      return true;
    } catch (e) { setMsg(`${what} refused: ${errText(e)}`); return false; }
  });
}
function tmApplyOp(op, reverse) {   // undo and redo: the stored map goes back to the service
  const sid = op.sid, map = reverse ? op.before : op.after;
  tmQueue(async () => {
    if (sid !== S.sid) return;
    try { await B.tempoMapSet(sid, map); await tmLoad(); setDirtyState(true); setMsg(`${reverse ? "undo" : "redo"}: ${op.what}`); }
    catch (e) { setMsg(`${reverse ? "undo" : "redo"} failed: ${errText(e)}`, "error"); }
  });
}

/* File > Import Tempo Map: the tempo and time-signature events of a MIDI file from the DAW replace the map (one undo step). */
async function importTempoMap() {
  if (!needProject()) return null;
  const path = await B.pickTempoFile();
  if (!path) { setMsg("import cancelled"); return null; }
  const name = path.split("/").pop(), sid = S.sid;
  return tmQueue(async () => {
    try {
      const r = await B.tempoMapImportMidi(sid, path);
      if (sid !== S.sid) return r;
      if (!r.changed) { setMsg(`${name}: the tempo map is the same as before`); return r; }
      pushUndo({ type: "tempo", sid, before: r.previous || {}, after: r.map, what: "import tempo map" });
      await tmLoad(); setDirtyState(true);
      setMsg(`imported ${r.tempo_events} tempo and ${r.meter_events} meter events from ${name}`);
      return r;
    } catch (e) { setMsg(`tempo map import refused: ${errText(e)}`); return null; }
  });
}

/* Track > Detect Tempo: the suggestion of tempo_map_detect with its confidence and warnings; Accept is one undo step. */
let tmDetecting = false;
async function detectTempo() {
  if (!needProject() || tmDetecting) return null;
  if (!$("#tempoModal").classList.contains("hidden")) { setMsg("answer the Detect Tempo dialog first"); return null; }
  tmDetecting = true; setMsg("detecting the tempo from the kick and snare hits…");
  let d;
  try { d = await B.tempoMapDetect(S.sid); }
  catch (e) { setMsg(`detect tempo failed: ${errText(e)}`, "error"); return null; }
  finally { tmDetecting = false; }
  setMsg("tempo detected: accept or cancel");
  return tmDetectDialog(d);
}
function tmDetectDialog(d) {
  return new Promise(resolve => {
    const m = $("#tempoModal"), body = $("#tempoBody"), b1 = $("#tempoBar1"), pct = x => `${Math.round(x * 100)} %`;
    const whole = b => String(Math.round(b));   // whole-number tempos only in this dialog (owner rule 2026-10-27); the map keeps the exact value
    const low = d.bar1.confidence < 0.4, m44 = (d.suggestion.meter || [4, 4]).join("/") === "4/4";
    const other = m44 ? "two beats later" : "at the other candidate";   // the candidate is two beats later in 4/4 only
    let later = false;
    const line = (text, cls) => { const p = document.createElement("p"); p.textContent = text; if (cls) p.className = cls; body.appendChild(p); return p; };
    const render = () => {
      body.textContent = "";
      line(`Start tempo: ${whole(d.suggestion.bpm)} BPM (confidence ${pct(d.start.confidence)})`);
      const alt = (d.start.alternatives || []).filter(a => !a.gated).slice(0, 3).map(a => whole(a.bpm));
      if (alt.length) line(`The beat could also be: ${alt.join(", ")} BPM`, "tdim");
      line(`Bar 1 at ${fmt(later ? d.bar1.alternative : d.bar1.t, 3)} s (confidence ${pct(d.bar1.confidence)})` + (later ? `, ${other} than the first guess` : ""));
      if (low) line("The bar 1 confidence is low: the first bar line is a coin flip. Try the other candidate.", "twarn");
      const mv = d.meter && d.meter.value || d.suggestion.meter;
      line(`Meter: ${mv[0]}/${mv[1]}` + (d.meter && d.meter.feel === "two-beat" ? " (2/4, or 6/8 in dotted quarters, fit as well)" : ""));
      if (d.steps.length) {
        line(`Tempo steps (${d.steps.length}):`);
        const tb = document.createElement("table");
        for (const s of d.steps) {
          const tr = document.createElement("tr");
          for (const x of [`bar ${s.bar}`, `${fmt(s.t, 2)} s`, `${whole(s.bpm)} BPM`, `${s.bars} bars`, `confidence ${pct(s.confidence)}`]) { const td = document.createElement("td"); td.textContent = x; tr.appendChild(td); }
          tb.appendChild(tr);
        }
        body.appendChild(tb);
      } else line("No tempo steps: the tempo is steady.");
      for (const w of d.warnings || []) line(w, "twarn");
      line(`${d.fit.hits} hits, ${d.fit.beats} beats; ${pct(d.fit.on_beat)} on the beat, jitter ${tmBpmText(d.fit.jitter_ms)} ms`, "tdim");
    };
    const close = answer => {
      m.classList.add("hidden"); document.removeEventListener("keydown", esc, true);
      resolve(answer);
    };
    const esc = ev => { if (ev.key === "Escape") { ev.stopPropagation(); setMsg("detect tempo cancelled"); close(false); } };
    document.addEventListener("keydown", esc, true);
    b1.classList.toggle("hidden", !low);
    const b1Text = () => (later ? "Bar 1 as first guessed" : m44 ? "Bar 1 two beats later" : "Bar 1 at the other candidate");
    b1.textContent = b1Text();
    b1.onclick = () => { later = !later; b1.textContent = b1Text(); render(); };
    $("#tempoCancel").onclick = () => { setMsg("detect tempo cancelled"); close(false); };
    $("#tempoAccept").onclick = async () => {
      const map = JSON.parse(JSON.stringify(d.suggestion));
      if (later) map.bar1_s = d.bar1.alternative;
      m.classList.add("hidden"); document.removeEventListener("keydown", esc, true);
      resolve(await tmCommit(map, "tempo map detected"));
    };
    render();
    m.classList.remove("hidden");
  });
}

function drawOverviewTracks(ctx, W, H) {
  ctx.fillStyle = "#161920"; ctx.fillRect(0,0,W,H);
  const rowH = H / Math.max(1, S.order.length);
  S.order.forEach((name, idx) => {
    const p = S.peaks[name]; if (!p) return;
    const L = p.levels[65536] || p.levels[4096] || p.levels[256]; if (!L) return;
    const y0 = idx*rowH;
    ctx.fillStyle = "rgba(255,255,255,0.04)"; ctx.fillRect(0, y0, W, rowH-1);
    const amp = S.peakAmp[name] || 1;
    const scale = ((rowH-4)/2) / amp;
    if (Array.isArray(L.rms)) {
      ctx.fillStyle = S.tracks[name].color;
      for (let px=0; px<W; px++) {
        const b0 = Math.floor(px/W*L.n), b1 = Math.max(b0+1, Math.floor((px+1)/W*L.n));
        let r = 0; for (let b=b0;b<Math.min(b1,L.n);b++) r = Math.max(r, L.rms[b]);
        const hh = Math.min((rowH-4)/2, r*scale);
        if (hh>0.3) ctx.fillRect(px, y0+rowH/2-hh, 1, hh*2);
      }
    }
    ctx.fillStyle = S.tracks[name].color + "66";
    for (let px=0; px<W; px++) {
      const b0 = Math.floor(px/W*L.n), b1 = Math.max(b0+1, Math.floor((px+1)/W*L.n));
      let mn=1, mx=-1;
      for (let b=b0;b<Math.min(b1,L.n);b++) { mn=Math.min(mn,L.min[b]); mx=Math.max(mx,L.max[b]); }
      const yy0 = y0+rowH/2 - mx*scale, yy1 = y0+rowH/2 - mn*scale;
      ctx.fillRect(px, yy0, 1, Math.max(1, yy1-yy0));
    }
  });
}

/* P3-6a: the tracks of the overview do not depend on the view or the playhead: they are drawn once into
   an offscreen canvas (OV) and blitted; only the view box and the playhead are drawn per redraw. The
   uncached drawing cost 7.9 ms per redraw with 30 tracks in WKWebView. */
const OV = { key: "", peaks: [], amps: [], canvas: null };
function overviewTracks(c, W, H) {
  const peaks = S.order.map(n => S.peaks[n]), amps = S.order.map(n => S.peakAmp[n]);
  const key = `${c.width}x${c.height}|${W}x${H}|${S.order.map(n => n + S.tracks[n].color).join("|")}`;
  if (OV.canvas && key === OV.key && peaks.every((p, i) => p === OV.peaks[i]) && amps.every((a, i) => a === OV.amps[i])) return OV.canvas;
  OV.key = key; OV.peaks = peaks; OV.amps = amps;
  if (OV.canvas) { OV.canvas.width = 0; OV.canvas.height = 0; }
  OV.canvas = document.createElement("canvas"); OV.canvas.width = c.width; OV.canvas.height = c.height;
  const ctx = OV.canvas.getContext("2d");
  ctx.setTransform(c.width / W, 0, 0, c.height / H, 0, 0);
  drawOverviewTracks(ctx, W, H);
  return OV.canvas;
}
function drawOverview() {
  const c = $("#overview"), ctx = c.getContext("2d"), W = GEO.ovW, H = GEO.ovH;
  ctx.clearRect(0,0,W,H); ctx.fillStyle = "#161920"; ctx.fillRect(0,0,W,H);
  const maxD = maxDur();
  if (c.width && c.height) { ctx.save(); ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.drawImage(overviewTracks(c, W, H), 0, 0); ctx.restore(); }
  const x0 = S.view.t0/maxD*W, x1 = S.view.t1/maxD*W;
  ctx.fillStyle = "rgba(77,163,255,0.10)"; ctx.fillRect(x0, 0, x1-x0, H);
  ctx.strokeStyle = "#4da3ff"; ctx.strokeRect(x0+0.5, 0.5, x1-x0-1, H-1);
  ctx.fillStyle = "#4da3ff"; ctx.fillRect(x0-1, 0, 3, H); ctx.fillRect(x1-2, 0, 3, H);
  const px = S.playhead/maxD*W;
  ctx.strokeStyle = "#ff9f43"; ctx.lineWidth = 1.5;
  ctx.beginPath(); ctx.moveTo(px,0); ctx.lineTo(px,H); ctx.stroke();
  ctx.lineWidth = 1;
  ctx.fillStyle = "#ff9f43";
  ctx.beginPath(); ctx.moveTo(px-5,0); ctx.lineTo(px+5,0); ctx.lineTo(px,7); ctx.closePath(); ctx.fill();
}

function pickLevel(p, spp) {
  /* finest pyramid level whose bucket is not coarser than one pixel */
  const levels = Object.keys(p.levels).map(Number).sort((a,b)=>a-b);
  let best = null;
  for (const L of levels) if (L <= spp) best = L;
  return best;
}

/* ---------------- WebGL2 lanes (P3-6a, static/lanes-gl.js) ----------------
   One WebGL2 canvas (#lanesGL) lies under #lanesScroll and draws every visible wave lane at its place, one
   draw call per lane. The wave canvases above it are transparent: drawWave queues a lane (glPush),
   redrawAll draws the queue in one pass (glFlush), and the 2D side keeps the event lines. A scroll, a
   layout change (rows moved, resized, muted) repaints the GL canvas only (glRepaint). No WebGL2, a lost
   context or window.__DS_NO_GL__ / ?gl=0 leave the Canvas 2D path in drawWave. */
/* P4-26: the filled waveform (Pro Tools look): from a visible span of WAVE_FILL_S the peaks are drawn as one solid min/max
   shape in the track colour (the same peaks, no extra pass: the RMS body is not drawn); below it the look stays as it was
   (the sample window as bars or a line, the translucent envelope with the RMS body where no window is here yet). */
let WAVE_FILL_S = 0.5;
function waveFill(dur) { return dur >= WAVE_FILL_S; }
/* P4-26: GLQ.pend counts the sample window fetches on their way per lane, GLQ.exact says a lane showed its sample window in the
   last draw. A lane that showed the exact wave and waits for the window of a new view keeps the frame on screen
   (the draw is skipped) instead of showing the coarse envelope (bins of 256 samples, visible as blocks) for the frames
   between the jump and the window: with Tab held, every jump out of the cached window flashed those blocks.
   GLQ.noHold = true restores the old behaviour (the planted-fault test). */
const GLQ = { pend: {}, exact: {}, held: 0, noHold: false, noGen: false, eng: null, tried: false, off: false, all: false, q: [], warmed: new WeakSet(), box: "", fetched: {}, loadedSeq: 0, dims: false };
/* P4-1b fix round: does the placeholder (#lanesGL) take the size of the worker's frames (Chromium does)? Then spLeftCheck can
   see when it shows a full frame again. Seen once is enough; an engine that keeps the placeholder's own size never sets it. */
function glDims() {
  const e = GLQ.eng;
  if (!GLQ.dims && e && e.size_ && e.size_[0] > 1 && e.size_[0] !== 300 && e.canvas.width === e.size_[0] && e.canvas.height === e.size_[1]) GLQ.dims = true;
}
/* P3-6b: in the app the engine runs in a worker (tiles-worker.js) and the page holds no peak arrays (only the
   coarsest level for the overview) and no sample windows: tiles and windows move to the engine as bytes.
   Without a worker with WebGL2 the same engine runs in the page (LanesGL.create), still with raw tiles. The server
   keeps the arrays and the PCM of the page. */
function glMount(eng) {   // P3-14d: a scroll or a layout change repaints through the frame (GEO, invalidate(F.GL))
  const cv = eng.canvas, sc = $("#lanesScroll");
  if (GLQ.eng && GLQ.eng.canvas.parentNode) GLQ.eng.canvas.remove();
  cv.id = "lanesGL"; $("#editor").insertBefore(cv, sc);
  GLQ.eng = eng; GLQ.box = "";
}
function glEvict(key) { delete S.samplesCache[key]; invalidate(F.GL); }   // the engine dropped an old window: fetch it again when the lane needs it
function glEngine() {
  if (!GLQ.tried) {
    GLQ.tried = true;
    const no = window.__DS_NO_GL__ || /[?&]gl=0(&|$)/.test(location.search);
    const worker = !window.__DS_NO_WORKER__ && !/[?&]worker=0(&|$)/.test(location.search);
    const eng = no || !window.LanesGL ? null
      : (worker && LanesGL.remote({ url: "/tiles-worker.js?v=" + APP_V, onLost: why => glRemoteLost(why), onEvict: glEvict, onDrawn: () => { glDims(); if (SP.leaving) requestFrame(); } }))
        || LanesGL.create({ onLost: () => glDrop(), onEvict: glEvict });
    if (eng) glMount(eng);
  }
  return GLQ.off ? null : GLQ.eng;
}
const APP_V = (Array.from(document.scripts).map(x => /app\.js\?v=(\w+)/.exec(x.src)).find(Boolean) || [0, "0"])[1];
function glRemoteLost(why) {   // the worker engine failed: before its first answer, try the engine in the page; later, Canvas 2D
  const eng = GLQ.eng;
  if (eng && eng.ready) { glDrop(); return; }
  const local = window.LanesGL && LanesGL.create({ onLost: () => glDrop(), onEvict: glEvict });
  if (!local) { glDrop(); return; }
  glMount(local);
  glFeed(local);
  invalidate(F.GL);
}
function glDrop() {   // context lost or a draw failed: back to Canvas 2D for good
  if (GLQ.eng) { GLQ.eng.canvas.remove(); GLQ.eng.close(); }
  GLQ.eng = null; GLQ.q = [];
  invalidate(F.VIEW);
}
/* the arrays of a track as the bytes of peaks_tile answers (u32 level, start, n, then f32 min, max, rms) */
function packTiles(p) {
  const raw = {};
  for (const [lv, L] of Object.entries(p.levels)) {
    const n = L.n, buf = new ArrayBuffer(12 + 12 * n), f = new Float32Array(buf, 12);
    new Uint32Array(buf, 0, 3).set([+lv, 0, n]);
    f.set(L.min, 0); f.set(L.max, n); f.set(L.rms || new Float32Array(n), 2 * n);
    raw[lv] = buf;
  }
  return raw;
}
/* the tiles of every track to the engine (again): after the engine in the page took over from a failed worker */
function glFeed(eng) {
  const sid = S.sid;
  for (const n of S.order) {
    if (!S.peaks[n] || !S.peaks[n].tiles || !B.peaksRaw) continue;
    B.peaksRaw(sid, n).then(raw => { if (S.sid === sid && GLQ.eng === eng) eng.addTiles(n, raw.raw); }).catch(() => null);
  }
}
/* P3-6b: the arrays of a track for the Canvas 2D path (a track in app mode keeps only the coarsest level) */
function ensureArrays(name) {
  const p = S.peaks[name];
  if (!p || !p.tiles) return Promise.resolve();
  if (!p.loading) {
    const sid = S.sid;
    p.loading = B.peaks(sid, name).then(full => { if (S.sid === sid && S.peaks[name] === p) { S.peaks[name] = full; invalidate(F.VIEW); } }, () => { p.loading = null; });
  }
  return p.loading;
}
async function setLanesGL(on) {   // test hook: switch the renderer
  GLQ.off = !on; GLQ.fetched = {};
  if (GLQ.eng) GLQ.eng.canvas.style.display = on ? "" : "none";
  if (!on) await Promise.all(S.order.map(ensureArrays));
  redrawAll();
  return !!glEngine();
}
function glPush(canvas, name) {
  if (!glEngine() || !S.peaks[name] || !S.tracks[name]) return false;
  GLQ.q.push({ canvas, name });
  return true;
}
function glLane(canvas, name, view) {
  // P4-2 (P4-1b verifier minor 3): the span from spanOf (to the nanosecond, as viewDur), not t1 * sr - span0, which kept the
  // float noise of the view
  const p = S.peaks[name], d = spanOf(view), span0 = view.t0 * p.sr, spanS = d * p.sr, c = S.samplesCache[name];
  const have = spanS <= 262144 && c && c.start <= span0 && c.end >= span0 + spanS && !!c.data === !B.peaksRaw;   // the app keeps the window in the engine, not in the page
  const row = canvas.closest(".track");
  const [W, H] = laneSize(canvas), dpr = GEO.dpr;   // not canvas.width: a far lane holds a 1 x 1 canvas; P3-14d: no layout read
  return { key: name, W, H, dw: Math.floor(W * dpr), dh: Math.floor(H * dpr), x: 0, y: 0,
           fade: row && row.classList.contains("muted") ? 0.35 : 1,   // the .tlanes opacity of a muted track
           fill: waveFill(d),   // P4-26: the solid min/max shape from a visible span of WAVE_FILL_S
           color: S.tracks[name].color, peaks: B.peaksRaw ? undefined : p, amp: S.peakAmp[name] || 1, gain: S.ampZoom, span0, spanS,
           samples: have ? (c.data ? { start: c.start, data: c.data } : { start: c.start, end: c.end }) : null, want: spanS <= 262144 && !have };
}
/* draw the lanes of items [{canvas, name}] at their places; false when WebGL2 is not available */
function glDraw(items) {   // P3-14d: places from GEO; the GL canvas style is written only when its box changed
  const eng = glEngine();
  if (!eng) return null;
  const cv = eng.canvas, dpr = GEO.dpr, view = S.view, cw = GEO.scW, chh = GEO.scH, box = `${GEO.scL},${GEO.scT},${cw},${chh}`;
  if (GLQ.box !== box) {
    GLQ.box = box;
    cv.style.left = GEO.scL + "px"; cv.style.top = GEO.scT + "px"; cv.style.width = cw + "px"; cv.style.height = chh + "px";
  }
  eng.size(Math.max(1, Math.round(cw * dpr)), Math.max(1, Math.round(chh * dpr)));
  const lanes = [], x = GEO.waveX, W = GEO.laneW;
  for (const it of items) {
    const g = GEO.rows[it.name];
    if (!g) continue;   // not measured yet: the observer repaints when it is
    const y = g.wave - GEO.top, H = g.waveH || GEO.waveH;
    if (!GLQ.all && (y + H < 0 || y > chh || x + W < 0 || x > cw)) continue;   // scrolled out of view
    const l = glLane(it.canvas, it.name, view);
    l.x = Math.round(x * dpr); l.y = Math.round(y * dpr);
    l.item = it; lanes.push(l);
  }
  eng.prune(S.order);
  if (B.peaksRaw) for (const it of items) { const p = S.peaks[it.name]; if (p && !p.tiles && !eng.hasTiles(it.name)) eng.addTiles(it.name, packTiles(p)); }   // arrays of the Canvas 2D period go back to the engine
  for (const l of lanes) {
    if (!l.want || GLQ.fetched[l.key] === view) continue;   // the sample window is not here yet
    GLQ.fetched[l.key] = view;
    GLQ.pend[l.key] = (GLQ.pend[l.key] || 0) + 1;
    ensureSamples(l.key, Math.max(0, Math.floor(l.span0)), Math.ceil(l.span0 + l.spanS)).catch(() => null).then(() => {
      GLQ.pend[l.key]--;
      if (S.view === view && glEngine()) invalidate(F.GL);   // the window is here: draw the exact wave
    });
  }
  if (!GLQ.noHold && lanes.some(l => l.want && GLQ.pend[l.key] > 0 && GLQ.exact[l.key])) { GLQ.held++; return lanes; }   // P4-26: keep the exact frame until the window is here
  if (!eng.draw(lanes)) return null;
  for (const l of lanes) GLQ.exact[l.key] = !!l.samples;
  if (GLQ.loadedSeq < 0 && lanes.length) GLQ.loadedSeq = eng.stats.posted;   // the first draw with lanes after a load: the self-test waits for its answer
  return lanes;
}
function glItems() {   // every lane that WebGL2 draws, from the DOM
  const out = [];
  for (const name of S.order) {
    const row = rowOf(name);
    if (row && S.peaks[name] && S.tracks[name]) out.push({ canvas: row.querySelector(".wave"), name });
  }
  return out;
}
function glRepaint() {   // the lanes moved or changed on screen, not the data
  if (!glEngine()) return;
  try { glDraw(glItems()); } catch (e) { glDrop(); }
}
function glFlush() {   // the end of drawAll: draw the queued lanes, then the 2D parts
  const items = GLQ.q; GLQ.q = [];
  if (!glEngine()) return;
  let lanes;
  try { lanes = glDraw(items); } catch (e) { lanes = null; }
  if (!lanes) { glDrop(); return; }   // the lanes fall back to Canvas 2D
  for (const it of items) {
    if (it.canvas.closest(".track")._live === false) continue;   // a lane without a canvas store has no event lines to draw
    const c2 = it.canvas.getContext("2d"), [W, H] = laneSize(it.canvas);
    c2.clearRect(0, 0, W, H);
    drawEventLines(it.canvas, it.name);
    const p = S.peaks[it.name];
    if (!B.peaksRaw && !GLQ.warmed.has(p)) { GLQ.warmed.add(p); GLQ.eng.warm([{ key: it.name, peaks: p }]); }
  }
}
/* The pixels of a lane as the user sees it (the GL lane under the 2D canvas); tests and the self-test read this,
   because the browser clears a WebGL canvas after it showed it. Returns a promise of ImageData. */
async function laneImage(name) {
  const c = document.querySelector(`.track[data-name="${name}"] canvas.wave`), tmp = document.createElement("canvas");
  const dpr0 = window.devicePixelRatio || 1;
  tmp.width = Math.floor(c.clientWidth * dpr0); tmp.height = Math.floor(c.clientHeight * dpr0);
  const t = tmp.getContext("2d"), eng = glEngine();
  if (eng && S.peaks[name]) {
    const l = glLane(c, name, S.view), sc = $("#lanesScroll"), dpr = window.devicePixelRatio || 1;
    eng.size(Math.max(1, Math.round(sc.clientWidth * dpr)), Math.max(1, Math.round(sc.clientHeight * dpr)));
    const im = await eng.readLane(l);
    if (im) t.putImageData(new ImageData(im.data, im.width, im.height), 0, 0);
    GLQ.box = ""; glRepaint();   // the read drew the lane at the corner: put the canvas back
  }
  if (c.width > 1) t.drawImage(c, 0, 0);
  const out = t.getImageData(0, 0, tmp.width, tmp.height);
  tmp.width = 0; tmp.height = 0;
  return out;
}

function drawWave(canvas, name) {
  if (glPush(canvas, name)) return;   // P3-6a: WebGL2 draws this lane
  const ctx = canvas.getContext("2d"), [W, H] = laneSize(canvas);
  const t = S.tracks[name], p = S.peaks[name];
  const gen = canvas._gen = (canvas._gen || 0) + 1;   // P4-26: a draw that waits for its samples is dropped when a newer draw of this lane came meanwhile
  ctx.clearRect(0,0,W,H);
  ctx.fillStyle = "#101318"; ctx.fillRect(0,0,W,H);
  if (!p) { ctx.fillStyle = "#7d8593"; ctx.fillText(S.peakErr[name] ? `cannot draw this track: ${S.peakErr[name]}` : "no peaks — re-analyze", 10, 20); return; }
  if (p.tiles) { ensureArrays(name); return; }   // P3-6b: the page holds the tiles in the engine only; the 2D path loads the arrays
  const sr = p.sr, span0 = S.view.t0*sr, spanS = viewDur()*sr, span1 = span0+spanS;   // P4-1b fix: the span of viewDur
  const mid = H/2, color = t.color;
  const amp = S.peakAmp[name] || 1;
  const scale = (H*0.45) / amp * S.ampZoom;   // auto-gain: track peak → 90% of half-height; P3-20a: times the amplitude zoom (the canvas clips)
  const fill = waveFill(viewDur());   // the tile path below; the sample window path (up to 262144 samples) draws the exact wave, which the GL lane shows as solid columns too
  if (spanS <= 262144) {
    ensureSamples(name, Math.max(0,Math.floor(span0)), Math.ceil(span1)).catch(() => null).then(() => {
      if (!GLQ.noGen && canvas._gen !== gen) return;   // P4-26: stale: a newer draw has cleared and drawn this lane
      const cache = S.samplesCache[name];
      if (!cache || !cache.data || cache.start > span0 || cache.end < span1) return;   // P3-14d: the GL engine took the window over meanwhile
      const i0 = Math.max(cache.start, Math.floor(span0)), i1 = Math.min(cache.end, Math.ceil(span1));
      const pxPer = W/spanS;
      if (pxPer > 3) {
        ctx.fillStyle = color;
        for (let i=i0; i<i1; i++) {
          const x = tToX(i/sr, W), y = mid - cache.data[i-cache.start]*scale;
          ctx.beginPath(); ctx.arc(x,y,1.4,0,7); ctx.fill();
        }
      }
      ctx.strokeStyle = color; ctx.lineWidth = 1; ctx.beginPath();
      for (let i=i0; i<i1; i++) {
        const x = tToX(i/sr, W), y = mid - cache.data[i-cache.start]*scale;
        i===i0 ? ctx.moveTo(x,y) : ctx.lineTo(x,y);
      }
      ctx.stroke();
      ctx.strokeStyle = "rgba(255,255,255,0.15)";
      ctx.beginPath(); ctx.moveTo(0,mid); ctx.lineTo(W,mid); ctx.stroke();
      drawEventLines(canvas, name);
    });
    return;
  }
  const level = pickLevel(p, spanS/W) || 256;
  const L = p.levels[level];
  const hasRms = Array.isArray(L.rms);
  // pass 1: peak envelope (min/max), translucent
  ctx.fillStyle = fill ? color : color + "55";   // P4-26: from WAVE_FILL_S solid, no RMS body
  for (let px=0; px<W; px++) {
    const s0 = span0 + px/W*spanS, s1 = span0 + (px+1)/W*spanS;
    let b0 = Math.max(0, Math.floor(s0/level)), b1 = Math.min(L.n, Math.ceil(s1/level));
    if (b1 <= b0) { b0 = Math.min(L.n-1, b0); b1 = b0+1; }
    let mn = 1, mx = -1;
    for (let b=b0; b<b1; b++) { mn = Math.min(mn, L.min[b]); mx = Math.max(mx, L.max[b]); }
    const y0 = mid - mx*scale, y1 = mid - mn*scale;
    ctx.fillRect(px, y0, 1, Math.max(1, y1-y0));
  }
  // pass 2: RMS body — the solid "flesh" of the waveform (Pro Tools / Logic look)
  if (hasRms && !fill) {
    ctx.fillStyle = color;
    for (let px=0; px<W; px++) {
      const s0 = span0 + px/W*spanS, s1 = span0 + (px+1)/W*spanS;
      let b0 = Math.max(0, Math.floor(s0/level)), b1 = Math.min(L.n, Math.ceil(s1/level));
      if (b1 <= b0) { b0 = Math.min(L.n-1, b0); b1 = b0+1; }
      let rms = 0;
      for (let b=b0; b<b1; b++) rms = Math.max(rms, L.rms[b]);
      const hh = rms*scale;
      if (hh > 0.3) ctx.fillRect(px, mid-hh, 1, hh*2);
    }
  }
  ctx.strokeStyle = "rgba(255,255,255,0.12)"; ctx.beginPath(); ctx.moveTo(0,mid); ctx.lineTo(W,mid); ctx.stroke();
  drawEventLines(canvas, name);
}

function drawSpike(canvas, name) {
  const ctx = canvas.getContext("2d"), [W, H] = laneSize(canvas);
  ctx.clearRect(0,0,W,H);
  ctx.fillStyle = S.tracks[name].color + "0d"; ctx.fillRect(0,0,W,H);
  ctx.strokeStyle = "#2c313c"; ctx.beginPath(); ctx.moveTo(0,H-0.5); ctx.lineTo(W,H-0.5); ctx.stroke();
  drawEventLines(canvas, name);
}

/* P3-14d (W6): the hits of a track are sorted by time, except the one a drag moves (hitDrag) until the drag ends. The
   hits in [t0, t1] are found by a binary search: the dragged hit counts as its left neighbour's time in the search,
   so the rest stays sorted, and it is tested on its own. Returns the indices in order. */
let hitDrag = null;   // {track, index} while a hit of that track is dragged
function hitsIn(evs, t0, t1, skip) {
  const key = i => i === skip ? (i > 0 ? evs[i-1].t : -Infinity) : evs[i].t;
  let lo = 0, hi = evs.length;
  while (lo < hi) { const mid = (lo + hi) >> 1; if (key(mid) < t0) lo = mid + 1; else hi = mid; }
  const out = [];
  for (let i = lo; i < evs.length && key(i) <= t1; i++) if (i !== skip) out.push(i);
  if (skip >= 0 && skip < evs.length && evs[skip].t >= t0 && evs[skip].t <= t1) {
    let k = 0; while (k < out.length && out[k] < skip) k++;
    out.splice(k, 0, skip);
  }
  return out;
}
function dragSkip(name) { return hitDrag && hitDrag.track === name ? hitDrag.index : -1; }
/* Event lines drawn through BOTH lanes: handle in spike lane, continuation in wave lane. */
function drawEventLines(canvas, name) {
  const ctx = canvas.getContext("2d"), [W, H] = laneSize(canvas);
  const isSpike = canvas.classList.contains("spike");
  drawGridLines(ctx, W, H);   // P4-8: the bar and beat lines lie under the hit lines
  lnDrawBand(ctx, W, H, name);   // P4-6a: the range selection
  const color = S.tracks[name].color, events = S.events[name] || [];
  const lnNot = lnLearn(name) ? lnTimes(name).not : null;   // P4-6a: a hit marked "not"; P4-6c: grey when the filter took it out, else a red tint (it is exported, Tab stops on it)
  const pad = 5 * viewDur() / W;   // the hits up to 5 px outside the lane: their markers (8 px wide) reach into it (P4-1: a strip shows them too)
  // Owner request 2026-10-11: a hit line in the track colour vanished over the wave of that colour (the old 2D wave
  // filled its body at 33 %, the GL wave is solid). In the wave lane the lines are the UI text grey at 60 %, with a
  // soft dark halo, so the position reads on any colour without glare (white at 85 % was too bright for the owner).
  // Manual hits stay magenta, the selected hit yellow. Three passes: the halos in one path, then the lines per colour.
  const groups = new Map();   // colour -> [x, ...]
  let sel = null;
  for (const i of hitsIn(events, S.view.t0 - pad, S.view.t1 + pad, dragSkip(name))) {
    const x = Math.round(tToX(events[i].t, W)) + 0.5;
    if (x < -5 || x > W+5) continue;
    if (S.sel && S.sel.track===name && S.sel.index===i) { sel = { x, i }; continue; }
    const t0 = events[i].t;
    const pv = lnPreview(name, events[i]);   // P4-6c: a threshold drag shows the grey of the live threshold
    const grey = pv === null ? hgIsCut(events[i]) : pv || hgCutOnly(events[i]);
    const notTint = !grey && lnNot && lnNot.length && lnNear(lnNot, t0);   // marked "not", still in the output (filter off, or an edited hit)
    const c = grey ? HG_GREY : notTint ? LN_NOT_TINT : (isManual(events[i]) || events[i].edited) ? MANUAL_COLOR : (isSpike ? color : "#cdd3de");   // P4-3: a cut hit is grey; P4-6c: so is a hit the Learn filter took out; a "not" example still in the output is tinted; P4-40f: a hand velocity keeps the manual colour
    (groups.get(c) || groups.set(c, []).get(c)).push(x);
  }
  const gone = HG.ghosts[name];   // P4-3: while the Sensitivity moves, the hits that go stay as grey lines
  if (gone && gone.length) {
    ctx.strokeStyle = "#7a818d"; ctx.lineWidth = 1.25; ctx.globalAlpha = 1; ctx.setLineDash([4, 3]); ctx.beginPath();
    for (const t of gone) { const x = Math.round(tToX(t, W)) + 0.5; if (x >= -5 && x <= W + 5) { ctx.moveTo(x, 0); ctx.lineTo(x, H); } }
    ctx.stroke(); ctx.setLineDash([]);
  }
  ctx.lineWidth = 3; ctx.strokeStyle = "#000"; ctx.globalAlpha = isSpike ? 0.5 : 0.4;
  ctx.beginPath();
  for (const xs of groups.values()) for (const x of xs) { ctx.moveTo(x, 0); ctx.lineTo(x, H); }
  if (sel) { ctx.moveTo(sel.x, 0); ctx.lineTo(sel.x, H); }
  ctx.stroke();
  ctx.lineWidth = 1; ctx.globalAlpha = isSpike ? 1 : 0.6;
  for (const [c, xs] of groups) {
    ctx.strokeStyle = c; ctx.beginPath();
    for (const x of xs) { ctx.moveTo(x, 0); ctx.lineTo(x, H); }
    ctx.stroke();
    if (isSpike) { ctx.fillStyle = c; for (const x of xs) { ctx.beginPath(); ctx.moveTo(x-4, 0); ctx.lineTo(x+4, 0); ctx.lineTo(x, 7); ctx.closePath(); ctx.fill(); } }
  }
  if (sel) {
    ctx.globalAlpha = 1; ctx.strokeStyle = "#ffe066"; ctx.lineWidth = 2;
    ctx.beginPath(); ctx.moveTo(sel.x, 0); ctx.lineTo(sel.x, H); ctx.stroke();
    if (isSpike) {
      ctx.fillStyle = "#ffe066";
      ctx.beginPath(); ctx.moveTo(sel.x-4, 0); ctx.lineTo(sel.x+4, 0); ctx.lineTo(sel.x, 7); ctx.closePath(); ctx.fill();
      ctx.font = "9px sans-serif"; ctx.fillText(`${sel.i+1}/${events.length}`, sel.x+5, 9);
    }
  }
  ctx.globalAlpha = 1; ctx.lineWidth = 1;
  if (isSpike) lnDrawMarks(ctx, W, H, name);   // P4-6a: the mark bars and the example ticks over the hit lines
}

/* ---------------- samples cache ---------------- */
/* P3-6b: one window fetch per track at a time. Without this a zoom gesture started a new 1 MB fetch for each of the
   visible lanes in every frame while the earlier ones were still on their way (23 to 30 fetches of 0.8 MB with a median
   of 300 to 460 ms in one 2 s sweep, frames up to 120 ms); now a call only records the range it wants, and the running
   fetch is followed by one more for the newest range. GLQ.noCoalesce restores the old behaviour (measurements). */
const SF = {};
function windowCovers(name, start, end) {
  const cache = S.samplesCache[name], eng = glEngine();
  return !!(cache && cache.start <= start && cache.end >= end && !!cache.data === !eng);   // data in the page: Canvas 2D; none: in the engine
}
function ensureSamples(name, start, end) {
  if (windowCovers(name, start, end)) return Promise.resolve();
  if (GLQ.noCoalesce) return fetchWindow(name, start, end);
  const f = SF[name] || (SF[name] = { run: null, want: null });
  f.want = { start, end };
  if (!f.run) f.run = (async () => {
    try {
      while (f.want) {
        const w = f.want; f.want = null;
        if (!windowCovers(name, w.start, w.end)) await fetchWindow(name, w.start, w.end);
      }
    } finally { f.run = null; }
  })();
  return f.run;
}
async function fetchWindow(name, start, end) {
  const eng = glEngine(), sid = S.sid;
  const s = Math.max(0, start - 48000), e = end + 48000;
  const r = await B.samples(sid, name, s, e);
  if (S.sid !== sid) return;   // another project is open now
  if (eng && !eng.lost) {   // P3-6b: the window moves to the engine; the page keeps its range only
    S.samplesCache[name] = { start: r.start, end: r.start + r.data.length };
    eng.setWindow(name, r.start, r.data);
    return;
  }
  S.samplesCache[name] = { start: r.start, end: r.start + r.data.length, data: r.data };
  if (S.samplesCache[name].data.length > 900000) {
    const c2 = S.samplesCache[name];
    const keep0 = Math.max(c2.start, Math.floor(start) - 96000), keep1 = Math.min(c2.end, end + 96000);
    c2.data = c2.data.slice(keep0 - c2.start, keep1 - c2.start); c2.start = keep0; c2.end = keep1;
  }
}

/* ---------------- event navigation (Tab) ---------------- */
function allEventsSorted(trackFilter) {
  const out = [];
  for (const name of S.order) {
    if (trackFilter && name !== trackFilter) continue;
    const evs = S.events[name] || [];
    for (let i=0;i<evs.length;i++) out.push({t: evs[i].t, track: name, index: i});
  }
  out.sort((a,b)=>a.t-b.t);
  return out;
}
function gotoEvent(dir, forceGlobal) {
  const trackFilter = (!forceGlobal && S.selTrack) ? S.selTrack : null;
  const all = allEventsSorted(trackFilter);
  if (!all.length) { setMsg(trackFilter ? `no events on ${trackFilter}` : "no events"); return; }
  // jump anchor is ALWAYS the playhead; the selection only tie-breaks simultaneous
  // hits when it sits exactly at the playhead (the normal state right after a jump).
  // Ties are ordered by POSITION IN THE SORTED LIST (D1) — matching the sort order,
  // not an alphabetical key that disagrees with it.
  let refT = S.playhead, refPos = -1;
  if (S.sel && (!trackFilter || S.sel.track === trackFilter)) {
    const e = (S.events[S.sel.track]||[])[S.sel.index];
    if (e && Math.abs(e.t - S.playhead) < 1e-9)
      refPos = all.findIndex(p => p.track === S.sel.track && p.index === S.sel.index);
  }
  const grey = p => lnGreyHit(p.track, S.events[p.track][p.index]);   // P4-6c: Tab skips the grey hits of the Learn filter
  let target = null;
  if (dir > 0) {
    for (let i=0; i<all.length; i++) {
      const p = all[i];
      if (p.t > refT + 1e-9 || (p.t === refT && i > refPos)) { if (!grey(p)) { target = p; break; } }
    }
    if (!target) { setMsg(trackFilter ? `end of ${trackFilter} events` : "no next event"); return; }
  } else {
    for (let i=all.length-1; i>=0; i--) {
      const p = all[i];
      if (p.t < refT - 1e-9 || (p.t === refT && i < refPos)) { if (!grey(p)) { target = p; break; } }
    }
    if (!target) { setMsg(trackFilter ? `start of ${trackFilter} events` : "no previous event"); return; }
  }
  S.sel = {track: target.track, index: target.index};
  S.playhead = snapSample(target.t, target.track);
  centerOn(S.playhead);
  updateStatus();   // the status bar: "track: hit n/N"; the info line has the rest
  if (SETTINGS().ui.auditionTab && !S.playing) audition(false, { track: target.track, t: target.t });   // P4-40d: Audition on Tab
}

/* ---------------- review queue (#150) ----------------
   D steps through the doubtful hits of all tracks, lowest confidence first; Shift+D steps back. The service rates the
   page's hits (track_confidence, crates/app/src/confidence.rs: a hit under 0.5 is doubtful); the page asks again only
   for a track whose hits changed since its last answer. The info line shows the reasons of a selected doubtful hit. */
var RQ = { sig: {}, rate: {}, at: new WeakMap() };   // per track: the signature and the ratings of the last answer; hit -> {key, r}
function rqKey(e) { return `${e.t}|${e.added ? 1 : 0}|${e.orig}|${e.cut ? 1 : 0}|${e.bleed ? 1 : 0}|${e.filtered ? 1 : 0}`; }
async function rqRatings(n) {
  const evs = S.events[n] || [], sig = evs.map(rqKey).join(",");
  if (RQ.sig[n] !== sig) {
    const r = await B.confidence(S.sid, n, evs);
    RQ.sig[n] = sig; RQ.rate[n] = r.confidence;
  }
  evs.forEach((e, i) => RQ.at.set(e, { key: rqKey(e), r: RQ.rate[n][i] }));
  return RQ.rate[n];
}
async function rqQueue() {   // [{track, e, c}], lowest confidence first; ties by time, then by track order
  const q = [];
  for (const n of S.order) {
    if (!S.tracks[n] || S.tracks[n].follows || !(S.events[n] || []).length) continue;   // a follower's hits are its source's
    const rs = await rqRatings(n);
    S.events[n].forEach((e, i) => { if (rs[i] && rs[i].doubt) q.push({ track: n, e, c: rs[i].c }); });
  }
  return q.sort((a, b) => a.c - b.c || a.e.t - b.e.t || S.order.indexOf(a.track) - S.order.indexOf(b.track));
}
async function reviewStep(dir) {   // Hits > Next Doubtful Hit (dir 1), Previous Doubtful Hit (dir -1)
  if (!S.sid) return;
  let q;
  try { q = await rqQueue(); } catch (err) { setMsg(`review queue: ${errText(err)}`, "error"); return; }
  if (!q.length) { setMsg("no doubtful hits"); return; }
  const cur = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  const at = cur ? q.findIndex(p => p.e === cur) : -1;   // the selected hit's place; a hit out of the queue starts it again
  const i = at < 0 ? (dir > 0 ? 0 : q.length - 1) : at + dir;
  if (i >= q.length) { setMsg("no more doubtful hits"); return; }
  if (i < 0) { setMsg("no earlier doubtful hits"); return; }
  const p = q[i];
  S.sel = { track: p.track, index: S.events[p.track].indexOf(p.e) };
  S.playhead = snapSample(p.e.t, p.track);
  centerOn(S.playhead);
  updateStatus();
  setMsg(`doubtful hit ${i + 1} of ${q.length}`);
}
function rqWhy(e) {   // the info line text of a doubtful hit whose rating is current, else ""
  const a = e && RQ.at.get(e);
  return a && a.key === rqKey(e) && a.r && a.r.doubt ? `${a.r.c.toFixed(2)}: ${a.r.why.join("; ")}` : "";
}

/* ---------------- other mics under the selected hit (#157) ----------------
   View > Other Mics Under Hit (key O): a panel under the lanes with one small waveform lane per other track, for the
   window of the selected hit (the hit time +-50 ms); the hit time is the centre line. The window is one bridge read per
   track (B.samples); the page keeps no PCM of its own. The panel shows only with a selected hit, and it follows every
   selection, because updateStatus calls mxRefresh. A generation counter drops the reads that a newer selection passed. */
const MX = { on: false, gen: 0 };
const MX_HALF = 0.05, MX_W = 240, MX_H = 40;
function mxToggle() {
  MX.on = !MX.on;
  if (B.appSettingsSet) B.appSettingsSet("other_mics", MX.on).catch(e => setMsg(`settings: ${errText(e)}`));
  mxApply();
  setMsg(`Other mics ${MX.on ? "shown" : "hidden"}`);
}
function mxApply() {   // after a toggle or the settings load: the panel shows or hides, the menu state follows
  document.body.classList.toggle("mxOn", MX.on);
  mxRefresh();
  if (window.DSShell && DSShell.sync) DSShell.sync();
}
function mxRefresh() {   // the panel: one row per other track, filled from the window of the selected hit
  const panel = $("#micsPanel"), sel = S.sel;
  const e = MX.on && sel && (S.events[sel.track] || [])[sel.index];
  panel.classList.toggle("hidden", !e);
  const gen = ++MX.gen;
  if (!e) { panel.replaceChildren(); return; }
  const names = S.order.filter(n => n !== sel.track && S.tracks[n]);
  panel.replaceChildren(...names.map(n => {
    const row = document.createElement("div"), head = document.createElement("span"), label = document.createElement("span"), gain = document.createElement("span"), c = document.createElement("canvas");
    row.className = "mxRow"; row.dataset.name = n;
    head.className = "mxHead";
    label.className = "mxName"; label.textContent = n;
    gain.className = "mxGain";   // the normalisation gain of the lane, set by mxDraw
    head.append(label, gain);
    c.className = "mxCan"; c.width = MX_W; c.height = MX_H;
    row.append(head, c);
    return row;
  }));
  const sid = S.sid;
  for (const n of names) {
    const sr = S.peaks[n] && S.peaks[n].sr;
    if (!sr) continue;   // no peaks: no sample rate, the row stays empty
    const a = Math.max(0, Math.round((e.t - MX_HALF) * sr)), b = Math.round((e.t + MX_HALF) * sr);
    B.samples(sid, n, a, b).then(r => {
      if (MX.gen !== gen || S.sid !== sid) return;   // a newer selection, or another project: drop the read
      const c = panel.querySelector(`.mxRow[data-name="${CSS.escape(n)}"] canvas`);
      if (c) {
        let peak = 0;   // the lane scales to its own peak: a bleed-level signal still reads
        for (let i = 0; i < r.data.length; i++) { const v = Math.abs(r.data[i]); if (v > peak) peak = v; }
        const db = peak > 1e-6 ? `+${(20 * Math.log10(1 / peak)).toFixed(0)} dB` : "−∞ dB";
        const g = panel.querySelector(`.mxRow[data-name="${CSS.escape(n)}"] .mxGain`);
        if (g) g.textContent = db;
        mxDraw(c, r, (e.t * sr - r.start) / r.data.length, S.tracks[n].color, peak);
      }
    }, () => {});
  }
}
function mxDraw(canvas, r, hitAt, color, peak) {   // r: {start, data} of the window; hitAt: the hit's place in the window (0 to 1); peak: the normalisation peak (1 = full scale)
  const ctx = canvas.getContext("2d"), W = canvas.width, H = canvas.height, d = r.data, n = d.length;
  const k = peak > 0 ? 1 / peak : 0;   // samples of a quiet lane reach the full lane height
  ctx.clearRect(0, 0, W, H);
  ctx.fillStyle = "rgba(0,0,0,0.22)"; ctx.fillRect(0, 0, W, H);
  ctx.fillStyle = color;
  for (let x = 0; x < W; x++) {   // the min and the max of the samples of one pixel column
    const i0 = Math.floor(x * n / W), i1 = Math.max(i0 + 1, Math.floor((x + 1) * n / W));
    let lo = 1, hi = -1;
    for (let i = i0; i < i1 && i < n; i++) { if (d[i] < lo) lo = d[i]; if (d[i] > hi) hi = d[i]; }
    if (hi < lo) continue;
    const y0 = (1 - hi * k) * H / 2, y1 = (1 - lo * k) * H / 2;
    ctx.fillRect(x, y0, 1, Math.max(1, y1 - y0));
  }
  ctx.fillStyle = "#ffd24d";   // the hit time
  ctx.fillRect(Math.round(hitAt * W), 0, 1, H);
}

function openUndoHistory() {
  if (!needProject()) return;
  const box = $("#undoHistList"); box.textContent = "";
  const rows = undoHistoryRows();
  $("#undoHistEmpty").classList.toggle("hidden", rows.length > 0);
  for (const r of rows) {
    const b = document.createElement("button");
    b.className = "uhRow" + (r.redo ? " uhRedo" : "");
    b.textContent = stepText(r.op, !r.redo);
    b.title = r.redo ? `Redo ${r.count} step${r.count === 1 ? "" : "s"}` : `Undo ${r.count} step${r.count === 1 ? "" : "s"}`;
    b.dataset.count = r.count; b.dataset.dir = r.redo ? "redo" : "undo";
    b.onclick = async () => { $("#undoHistModal").classList.add("hidden"); await undoSteps(!r.redo, r.count); };
    box.appendChild(b);
  }
  showModal("#undoHistModal");
}
function needProject() {   // S3-3: the page can show no project at all
  if (S.sid) return true;
  setMsg("no project is open: use New, Open or Open Recent"); return false;
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
}

/* ---------------- playback: the native transport (P3-5b) ---------------- */
function applyMixer() { appMix(); }   // live mute/solo during playback
function play(returnKey) { return appPlay(undefined, returnKey); }   // #207: returnKey is the play-and-return key that starts the play
/* P3-20b: Shift+Space (Pro Tools "Half-speed playback"): play from the playhead at half speed, one octave down;
   Space or Shift+Space stops. The transport resamples to twice the device rate; the playhead, the follow, the
   meters and the clicks run at the played speed (the messages carry speed 0.5). */
function halfSpeedPlayStop() { return auStop() ? undefined : S.playing ? pause() : appPlay(0.5); }   // P4-40d: Space and Shift+Space stop an audition first
function showSpeed(speed) {   // the status bar says "half speed" while the transport plays at half speed
  const el = $("#stSpeed"), half = speed === 0.5;
  if (el.classList.contains("hidden") === half) el.classList.toggle("hidden", !half);
}
function pause() { return appPause(); }
/* ---------------- mixer: volume, pan and meters per track; master fader and meter (P3-16) ----------------
   Pro Tools style. Volume -inf to +12 dB (the fader position follows VOL_TAPER: 0 dB at 3/4), pan -1 (left) to
   +1 (right); both are kept in the project (track_set_mix, marks it dirty). The master volume is kept in the app
   settings (transport_master). A drag moves a control; with Option the drag is ten times finer; a double-click
   or an Option-click resets it (0 dB, centre). The value shows in a tooltip while the mouse is on the control.
   Monitor settings only: no export reads them.
   Meters: the transport sends one message for all tracks about 25 times a second while it plays ({meters:
   [l0, r0, ...], master: [l, r]}, linear peaks, post-fader; a muted track still shows its level). A level falls
   at METER_FALL dB/s; the peak-hold line stays HOLD_MS and then falls at HOLD_FALL dB/s; a peak at or above
   0 dBFS lights the red clip light until a click on it. A message or a fall step changes the model and calls
   invalidate(F.METER); the frame (flushFrame) draws the meters with transforms only (no layout read); after the
   last message the meters fall on a 40 ms timer, which stops when every meter is at the floor. */
const VOL_TAPER = [[0, -120], [0.03, -60], [0.25, -24], [0.5, -9], [0.75, 0], [1, 12]];
const METER_FLOOR = -60, METER_FALL = 24, HOLD_MS = 1500, HOLD_FALL = 20, METER_STEP_MS = 40;
function taper(x, a, b) {   // piecewise linear over VOL_TAPER: column a in, column b out
  const T = VOL_TAPER;
  if (x <= T[0][a]) return T[0][b];
  for (let i = 1; i < T.length; i++) {
    if (x <= T[i][a]) { const p = T[i - 1], q = T[i]; return p[b] + (q[b] - p[b]) * (x - p[a]) / (q[a] - p[a]); }
  }
  return T[T.length - 1][b];
}
const volPos = db => taper(db, 1, 0), posVol = p => taper(p, 0, 1);
const clamp01 = v => Math.max(0, Math.min(1, v));
function fmtDb(db) { return db <= -119.95 ? "-inf" : (db > 0.04 ? "+" : "") + db.toFixed(1); }
function fmtPan(p) { const v = Math.round(p * 100); return v === 0 ? "C" : v < 0 ? `L${-v}` : `R${v}`; }   // P3-14c: C, L50, R50 (Pro Tools)
function mixTip(el, text) {
  let t = $("#mixTip");
  if (!t) { t = document.createElement("div"); t.id = "mixTip"; document.body.appendChild(t); }
  t.textContent = text; t.classList.remove("hidden");
  const r = el.getBoundingClientRect();
  t.style.left = Math.max(0, Math.min(r.left, innerWidth - t.offsetWidth - 4)) + "px";
  t.style.top = Math.max(0, r.top - t.offsetHeight - 4) + "px";
}
function hideMixTip() { const t = $("#mixTip"); if (t) t.classList.add("hidden"); }
/* One control: a horizontal slider (div). o = {get, set(v), reset, toPos, fromPos, fmt, label, commit}. */
function bindMixSlider(el, o) {
  let dragging = false;
  const show = () => {
    const v = o.get(), p = clamp01(o.toPos(v));
    el.querySelector(".mknob").style.left = `${p * 100}%`;
    const f = el.querySelector(".mfill"); if (f) f.style.width = `${p * 100}%`;
    el.dataset.value = o.fmt(v);
    el.title = `${o.label} ${o.fmt(v)}. Drag (Option: fine); double-click or Option-click: reset`;
    const out = el.parentElement && el.parentElement.querySelector(`[data-for="${el.dataset.ctl}"]`);
    if (out) out.textContent = o.fmt(v);
  };
  el._show = show;
  el.addEventListener("mousedown", ev => {
    if (ev.button !== 0) return;
    ev.preventDefault(); ev.stopPropagation();
    const w = el.clientWidth || 1;
    let x0 = ev.clientX, pos = clamp01(o.toPos(o.get())), moved = false;
    dragging = true;
    const mv = e2 => {
      const dx = e2.clientX - x0; x0 = e2.clientX;
      if (!dx) return;
      moved = true;
      pos = clamp01(pos + dx / w * (e2.altKey ? 0.1 : 1));
      o.set(o.fromPos(pos)); show(); mixTip(el, `${o.label} ${o.fmt(o.get())}`);
    };
    const up = e2 => {
      document.removeEventListener("mousemove", mv, true); document.removeEventListener("mouseup", up, true);
      dragging = false; hideMixTip();
      if (!moved && e2.altKey) { o.reset(); show(); }
    };
    document.addEventListener("mousemove", mv, true); document.addEventListener("mouseup", up, true);
    mixTip(el, `${o.label} ${o.fmt(o.get())}`);
  });
  el.addEventListener("click", ev => ev.stopPropagation());   // not a click on the header (track selection)
  el.addEventListener("dblclick", ev => { ev.preventDefault(); ev.stopPropagation(); o.reset(); show(); });
  el.addEventListener("mouseenter", () => mixTip(el, `${o.label} ${o.fmt(o.get())}`));
  el.addEventListener("mouseleave", () => { if (!dragging) hideMixTip(); });
  show();
}
/* Sends: one call in flight per target; a change during the call sends the latest value after it. */
const _mixQ = {};
function mixSend(key, call, onDone) {
  const q = _mixQ[key] || (_mixQ[key] = { busy: false, again: false });
  if (q.busy) { q.again = true; return; }
  q.busy = true; q.again = false;
  call().then(r => { if (onDone) onDone(r); })
    .catch(e => setMsg(`volume/pan failed: ${e.message.replace(/^\d+: /, "")}`, "error"))
    .finally(() => { q.busy = false; if (q.again) mixSend(key, call, onDone); });
}
function trackVol(name) { const t = S.tracks[name] || {}; return typeof t.volume_db === "number" ? t.volume_db : 0; }
function trackPan(name) { const t = S.tracks[name] || {}; return typeof t.pan === "number" ? t.pan : 0; }
function sendTrackMix(name) {
  const sid = S.sid;
  mixSend(`t:${sid}:${name}`, () => B.trackSetMix(sid, name, trackVol(name), trackPan(name)),
    () => { if (S.sid === sid) setDirtyState(true); });
}
function setTrackMix(name, volumeDb, pan) {   // the model, the controls, the service
  const t = S.tracks[name]; if (!t) return;
  if (volumeDb !== null) t.volume_db = Math.round(Math.max(-120, Math.min(12, volumeDb)) * 10) / 10;
  if (pan !== null) t.pan = Math.round(Math.max(-1, Math.min(1, pan)) * 100) / 100;
  const row = rowOf(name);
  if (row) for (const el of row.querySelectorAll(".mslider")) el._show();
  if (name === INS.name) for (const el of document.querySelectorAll("#inspector .mslider")) el._show();   // P3-14a
  sendTrackMix(name);
}
function bindTrackMix(row, name) {   // P3-14a: set goes through setTrackMix too, so the Inspector's controls follow a header drag
  bindMixSlider(row.querySelector(".mslider.vol"), { label: "Volume", get: () => trackVol(name),
    set: v => setTrackMix(name, v, null), reset: () => setTrackMix(name, 0, null),
    toPos: volPos, fromPos: posVol, fmt: fmtDb });
  bindMixSlider(row.querySelector(".mslider.pan"), { get label() { return (S.tracks[name] || {}).play_channels === 2 ? "Balance" : "Pan"; }, get: () => trackPan(name),
    set: v => setTrackMix(name, null, v), reset: () => setTrackMix(name, null, 0),
    toPos: p => (p + 1) / 2, fromPos: x => x * 2 - 1, fmt: fmtPan });
  const meter = row.querySelector(".meter");
  MT.tracks[name] = newMeter(meter);
  meter.querySelector(".mclip").addEventListener("click", ev => { ev.stopPropagation(); clearClip(MT.tracks[name]); });
}
/* A stereo file plays as stereo (P3-16): its meter has two bars (L, R) and its pan is a balance. A mono track's meter
   is one bar (its level after the volume, before the pan). The channels come with the play answer. */
function markTrackChannels(names, channels) {
  names.forEach((n, i) => {
    const row = rowOf(n), t = S.tracks[n]; if (!row || !t) return;
    t.play_channels = channels[i];
    row.querySelector(".meter").classList.toggle("mono", channels[i] !== 2);
    invalidate(F.METER);   // P4-40h: the canvas paints one bar for mono, two for stereo
    const pan = row.querySelector(".mslider.pan"); if (pan && pan._show) pan._show();
  });
}
function resetSelectedMix() {   // Track > Reset Volume and Pan
  const n = needTrack(); if (!n) return;
  setTrackMix(n, 0, 0); setMsg(`${n}: volume 0 dB, pan centre`);
}
function setMasterDb(db, send) {
  S.masterDb = Math.round(Math.max(-120, Math.min(12, db)) * 10) / 10;
  const el = $("#masterVol"); if (el && el._show) el._show();
  if (send) mixSend("master", () => B.transportMaster(S.masterDb));
}
function setupMaster() {
  S.masterDb = 0;
  bindMixSlider($("#masterVol"), { label: "Master", get: () => S.masterDb, set: v => setMasterDb(v, true),
    reset: () => setMasterDb(0, true), toPos: volPos, fromPos: posVol, fmt: fmtDb });
  MT.master = newMeter($("#masterMeter"), true);
  $("#masterMeter .mclip").addEventListener("click", () => clearClip(MT.master));
  B.transportStatus().then(st => { if (st && typeof st.master_db === "number") setMasterDb(st.master_db, false); }).catch(() => {});
}
/* meters. P4-40h: the bars and the hold lines are drawn on ONE canvas per meter (a style write of any meter
   element triggers the Blink style/prepaint lifecycle of the whole 30-track page in every frame, measured at
   19 % of one core at 25 msg/s; a canvas write skips that lifecycle, 4 % with all rows drawn). The model (lvl,
   hold, clip) and `drawn` (the last painted 1/200 fractions, the tests read them) are unchanged. The clip light
   stays a DOM element: the click on it must keep working, and it changes rarely. Only rows inside the scroll
   viewport are painted (their model still updates); a scroll or a geometry change repaints once. */
const MT = { tracks: {}, master: null, timer: null, at: 0 };
function meterBox(el, horizontal) { return el.querySelector(horizontal ? ".mhbars" : ".mbars"); }
function newMeter(el, horizontal) {
  const box = meterBox(el, horizontal);
  const c = document.createElement("canvas");
  box.textContent = "";
  box.appendChild(c);
  const m = { el, horizontal: !!horizontal, lvl: [-Infinity, -Infinity], hold: [-Infinity, -Infinity], holdAt: [0, 0], clip: false,
    light: el.querySelector(".mclip"), canvas: c, ctx: null, grad: null, w: 0, h: 0, dpr: 0, mono: null,
    drawn: [-1, -1, -1, -1], clipDrawn: false, t: 0 };
  m.ctx = c.getContext("2d");
  meterSize(m);
  drawMeter(m);   // the empty slots at once: the look before the first message is the old one
  new ResizeObserver(() => { meterSize(m); invalidate(F.METER); }).observe(box);
  return m;
}
function meterSize(m) {   // the backing store to the box size and the device pixel ratio; writes only
  const box = m.canvas.parentElement, dpr = window.devicePixelRatio || 1;
  if (!box.isConnected) return;   /* #110: the box left the DOM (the project closed). The final ResizeObserver
     delivery of a removed element lands after buildTrackDOM, and a write here would size the detached
     canvas again (1 x 1) and take its backing store back. A draw sizes canvases that are in the DOM only. */
  const w = Math.max(1, Math.round(box.clientWidth * dpr)), h = Math.max(1, Math.round(box.clientHeight * dpr));
  if (m.w === w && m.h === h && m.dpr === dpr) return;
  m.w = w; m.h = h; m.dpr = dpr;
  m.canvas.width = w; m.canvas.height = h;
  m.grad = meterGrad(m);
  m.drawn = [-1, -1, -1, -1];
}
function meterGrad(m) {   // the level gradient of the CSS look, as an offscreen strip at the bar size
  const off = document.createElement("canvas"), ctx = off.getContext("2d");
  const w = m.w, h = m.horizontal ? Math.max(1, Math.floor(m.h / 2)) : m.h;
  off.width = Math.max(1, w); off.height = Math.max(1, h);
  const g = m.horizontal ? ctx.createLinearGradient(0, 0, w, 0) : ctx.createLinearGradient(0, h, 0, 0);
  g.addColorStop(0, "#2fbf5b"); g.addColorStop(0.7, "#2fbf5b"); g.addColorStop(0.88, "#e8c547"); g.addColorStop(1, "#ff4b4b");
  ctx.fillStyle = g; ctx.fillRect(0, 0, off.width, off.height);
  return off;
}
function clearClip(m) { if (!m) return; m.clip = false; invalidate(F.METER); }
function meterIn(m, l, r, now) {   // linear peaks of one message (or 0 for the fall)
  const dt = m.t ? Math.min(0.5, (now - m.t) / 1000) : 0; m.t = now;
  [l, r].forEach((v, c) => {
    v = +v || 0;
    if (v >= 1) m.clip = true;
    const db = v > 0 ? 20 * Math.log10(v) : -Infinity;
    m.lvl[c] = Math.max(db, m.lvl[c] - METER_FALL * dt);
    if (db >= m.hold[c]) { m.hold[c] = db; m.holdAt[c] = now; }
    else if (now - m.holdAt[c] > HOLD_MS) m.hold[c] = Math.max(m.lvl[c], m.hold[c] - HOLD_FALL * dt);
  });
}
const meterFrac = db => clamp01((db - METER_FLOOR) / -METER_FLOOR);
function drawMeter(m) {   // in the frame (F.METER): paints the one canvas; no layout read
  if (window.devicePixelRatio !== m.dpr) meterSize(m);   // a display move must not leave a stale backing store
  const f0 = Math.round(meterFrac(m.lvl[0]) * 200) / 200, f1 = Math.round(meterFrac(m.lvl[1]) * 200) / 200;
  const h0 = Math.round(meterFrac(m.hold[0]) * 200) / 200, h1 = Math.round(meterFrac(m.hold[1]) * 200) / 200;
  const mono = !m.horizontal && m.el.classList.contains("mono");
  if (f0 === m.drawn[0] && f1 === m.drawn[1] && h0 === m.drawn[2] && h1 === m.drawn[3] && mono === m.mono && m.clip === m.clipDrawn) return;
  if (!m.w || !m.h) { m.mono = mono; return; }   // sized by the observer; nothing to paint into yet
  m.mono = mono;
  m.drawn[0] = f0; m.drawn[1] = f1; m.drawn[2] = h0; m.drawn[3] = h1;
  if (m.clip !== m.clipDrawn) { m.clipDrawn = m.clip; m.light.classList.toggle("on", m.clip); }
  const ctx = m.ctx, W = m.w, H = m.h, grad = m.grad;
  ctx.clearRect(0, 0, W, H);
  if (m.horizontal) {   // the master: two bars of H/2 (the CSS gap of 2 px between), the gradient along the width
    const bh = Math.floor(H / 2), gap = H - 2 * bh, lw = Math.max(1, m.dpr);
    for (let c = 0; c < 2; c++) {
      const y = c * (bh + gap), f = c ? f1 : f0, h = c ? h1 : h0;
      ctx.fillStyle = "#0d0f13"; ctx.fillRect(0, y, W, bh);
      const fw = Math.round(f * W);
      if (fw > 0) ctx.drawImage(grad, 0, 0, W, bh, 0, y, fw, bh);   // the full gradient compressed: the old scaleX look
      if (h > 0) { ctx.fillStyle = "#ffe08a"; ctx.fillRect(Math.max(0, Math.round(h * W) - lw), y, lw, bh); }
    }
    return;
  }
  const gap = Math.round(m.dpr), bw = mono ? W : Math.floor((W - gap) / 2);   // the bars, 1 CSS px apart
  for (let c = 0; c < 2; c++) {
    if (mono && c === 1) break;
    const x = mono ? 0 : c * (W - bw), f = c ? f1 : f0, h = c ? h1 : h0;
    ctx.fillStyle = "#0d0f13"; ctx.fillRect(x, 0, bw, H);
    const fh = Math.round(f * H);
    if (fh > 0) ctx.drawImage(grad, 0, 0, bw, H, x, H - fh, bw, fh);   // the full gradient compressed: the old scaleY look
    if (h > 0) { ctx.fillStyle = "#ffe08a"; ctx.fillRect(x, Math.max(0, H - Math.round(h * H)), bw, Math.max(1, m.dpr)); }
  }
}
function allMeters() { const v = Object.values(MT.tracks); if (MT.master) v.push(MT.master); return v; }
function drawMeters() {   // in the frame (F.METER): only the rows inside the scroll viewport (GEO, cached)
  const top = GEO.top, bot = GEO.top + GEO.scH;
  for (const name of S.order) {
    const m = MT.tracks[name];
    if (!m) continue;
    const g = GEO.ok && GEO.rows[name];
    if (g && (g.bottom <= top || g.top >= bot)) continue;
    drawMeter(m);
  }
  if (MT.master) drawMeter(MT.master);
}
function onMeters(m) {
  AC.meters = m; AC.meterN = (AC.meterN || 0) + 1;
  const now = performance.now(), names = AC.names || S.order;
  for (let i = 0; i < names.length; i++) { const t = MT.tracks[names[i]]; if (t) meterIn(t, m.meters[2 * i], m.meters[2 * i + 1], now); }
  if (MT.master && m.master) meterIn(MT.master, m.master[0], m.master[1], now);
  MT.at = now;
  invalidate(F.METER);
  meterFallSoon();
}
/* When no message came for 3 steps (pause, end, an error), the meters fall to the floor on a 40 ms timer. */
function meterFallSoon() {
  if (MT.timer) return;
  MT.timer = setTimeout(function step() {
    MT.timer = null;
    const now = performance.now();
    if (now - MT.at < 3 * METER_STEP_MS) { meterFallSoon(); return; }
    let up = false;
    for (const x of allMeters()) {
      meterIn(x, 0, 0, now);
      if ([...x.lvl, ...x.hold].some(db => db > METER_FLOOR)) up = true;
    }
    invalidate(F.METER);
    if (up) MT.timer = setTimeout(step, METER_STEP_MS);
  }, METER_STEP_MS);
}
let _clicksTimer = null;
function appClicksSoon() {   // settings sliders fire often: one call per 100 ms
  clearTimeout(_clicksTimer);
  _clicksTimer = setTimeout(() => { const c = SETTINGS().click; appCmd(() => B.transportClicks(c.enabled, c.volume, c.pitches)).catch(e => setMsg(`clicks failed: ${e.message}`, "error")); }, 100);
}

/* ---------------- follow modes (Pro Tools/Logic style) ----------------
   smooth: timeline glides continuously, playhead pinned at FOLLOW_FRAC of the view
   page:   flip one page when the playhead crosses the right edge
   off:    playhead may leave the view                                             */
const FOLLOW_FRAC = 0.25;
function followStep() {
  if (S.followMode === "off" || S.followHold) return;
  const d = viewDur();
  if (followKind() === "page") {   // P4-39b: also the smooth follow where no glide can show; it flips to FOLLOW_FRAC, the page mode centres
    if (S.playhead > S.view.t1 - d*0.1 || S.playhead < S.view.t0) {
      if (S.followMode !== "smooth") { centerOn(S.playhead); return; }
      const v = viewClamp(S.playhead - FOLLOW_FRAC * d, S.playhead - FOLLOW_FRAC * d + d);
      if (Math.abs(v.t0 - S.view.t0) > 1e-12 || Math.abs(v.t1 - S.view.t1) > 1e-9) setView(v.t0, v.t1);
    }
    return;
  }
  // smooth
  if (S.playhead < S.view.t0 || S.playhead > S.view.t1) { centerOn(S.playhead); return; }  // re-enter after Tab/jump
  const anchorX = FOLLOW_FRAC * d;
  const curX = S.playhead - S.view.t0;
  // P3-14d (W4): the view moves in every frame in which the playhead moved half a pixel or more (before: about 33 Hz
  // and 2 px steps, so the pinned playhead juddered against the 60 Hz frames)
  // P4-1: only when the clamped view changes. Left of the anchor at the start of the song (and at its end) the view
  // cannot move; before, every frame drew all lanes again for nothing (the whole `play` scenario has a 300 s view).
  if (Math.abs(curX - anchorX) / d * laneWidth() < 0.5) return;
  const v = viewClamp(S.playhead - anchorX, S.playhead - anchorX + d);
  if (Math.abs(v.t0 - S.view.t0) > 1e-12 || Math.abs(v.t1 - S.view.t1) > 1e-9) setView(v.t0, v.t1);   // the span is (t0 + d) - t0: an ulp apart
}
function tick() {   // one playback frame (the frame scheduler calls it while playing; play() calls it once)
  if (!S.playing) { cpStop(); spLeave("pause"); flushFrame(); return; }
  PH.v = appTickPos();
  followResume();   // P4-wf10: the 2 s of playback after the edit are over: follow again
  if (PH.v >= maxDur()) { pause(); S.playhead = 0; invalidate(F.HEAD); flushFrame(); return; }
  const now = AC.tickAt;
  let strip = SP.on && spTick(now);   // P4-1: a strip of the smooth follow moves by itself
  if (!SP.on && SP.req && spEnter(now, true)) strip = spTick(now);   // P4-1b: a strip asked for ahead shows now
  if (SP.on) { spWrite(now); requestFrame(); }   // #463: the strip's 2D transforms are written per frame (rAF), so the loop keeps running under a strip
  if (!strip) followStep();
  const cp = !strip && cpArm(now);   // P4-1: the compositor moves the playhead while the view stands still
  if (cp) spAhead(now);
  FR.flags |= F.PH;   // cheap: the overlay and the time; a follow step asked for the whole view
  const ox = Math.round(PH.v / maxDur() * GEO.ovW * 2);   // the overview playhead, in half pixels: a VIEW frame draws it too
  if (ox !== FR.ovx) { FR.ovx = ox; FR.flags |= F.HEAD; }
  flushFrame();
  if (!strip && !cp) spEnter(now);   // the view glides in the frame path: the first strip
  /* #463: the flag says this tick did NOT draw the playhead (an animation did). cp is also true when cpStill said
     show:false and no animation exists (the playhead left the view in off mode): the frame path drew it, so the
     tick must stay readable for drawnPlayheadAt. */
  CP.hist.push([now, AC.drawn, performance.now(), (cp && !!CP.anim) || SP.on]);
  if (CP.hist.length > CP_KEEP) CP.hist.splice(0, CP.hist.length - CP_KEEP);
  if (!cp && !SP.on) { requestFrame(); return; }
  clearTimeout(CP.slow); CP.slow = setTimeout(() => { if (S.playing) requestFrame(); }, CP_SLOW_MS);   // the counters without meter frames
}

/* ---------------- track handlers ---------------- */
/* P3-14d: the x of a pointer in a lane (the ruler, the overview) from clientX and GEO: MouseEvent.offsetX lays the
   page out when it is dirty, and a window listener's offsetX is relative to whatever element is under the pointer */
const laneXOf = ev => ev.clientX - GEO.laneX;
const rulerXOf = ev => ev.clientX - GEO.rulerX;
const ovXOf = ev => ev.clientX - GEO.ovX;
/* ---------------- velocity lane (P4-40f; openDAW O-3, own design from the report's description) ----------------
   View > Velocity Lane: a thin lane (22 px) under the spike lane of every track, one stalk per hit, the height is the
   velocity 1 to 127. Stalks of hits that leave the output (cut, tom bleed, Learn filter; hgIsCut) are grey, the rest in
   the track colour. The two dashed lines are the floor and the ceiling of the loudness settings (P4-3): the velocity
   ramp maps the floor to 20 and the ceiling to 127, so they sit at the 20 and the 127 level of the lane.
   Drag a stalk: the velocity changes. Option-drag draws a straight line over all hits in the range; Cmd-drag draws
   freehand (the stalks under the pointer follow the pointer's height). One gesture is one undo step: one events_edit
   call with one `velocity` op per hit and one step (a single hit: setv; several: setv-many, label
   "Set Velocity (track, n hits)"). Every hand velocity is sent with `edited: true`: the hit keeps the manual colour and
   survives Recompute Velocities and a re-pick (merge_edited, is_kept). The lane is a view setting (app setting
   velocity_lane, default off); it draws with the hits of its row (drawHits), so the edits show live.
   #208: the lane is 80 px by default and its height is the app setting velocity_lane_height (drag its bottom border,
   24 to 400 px). A header "Velocity" with a 127 / 64 / 0 scale, a dot on each stalk top, the selected stalk and the
   hits of a selected range in yellow, a hover label (velocity, time, why a stalk is grey), and a one-line hint
   until the first gesture. A drag of a selected stalk scales all selected hits by the same ratio. */
const VL_H_DEF = 80, VL_H_MIN = 24, VL_H_MAX = 400;
const VL = { on: false, drag: null, h: VL_H_DEF, hover: null, hint: false };   // drag: {track, mode, gen, x0, y0, set: Map(id -> [from, to]), anchor, v0, base}; hover: {track, id}; hint: the one-line hint shows
function vlToggle() {
  VL.on = !VL.on;
  VL.hint = VL.on;
  if (B.appSettingsSet) B.appSettingsSet("velocity_lane", VL.on).catch(e => setMsg(`settings: ${errText(e)}`));
  vlApply();
  setMsg(`Velocity lane ${VL.on ? "shown" : "hidden"}`);
}
function vlApply() {   // after a toggle: the rows grow or shrink, the canvases resize in the frame
  document.body.classList.toggle("velOn", VL.on);
  for (const row of document.querySelectorAll("#lanesScroll .track")) vlSyncRow(row);   // P4-40f: the vel canvas exists only while the lane is on
  if (typeof spLeave === "function") spLeave("vel");
  invalidate(F.SIZE | F.VIEW | F.GL);
  if (window.DSShell && DSShell.sync) DSShell.sync();
}
function vlSyncRow(row) {   // the row gets or loses its vel canvas (releaseCanvases gives the store back at once)
  let c = row.querySelector(".vel");
  if (VL.on && !c) {
    c = document.createElement("canvas");
    c.className = "lane vel"; c.height = VL.h;
    row.querySelector(".wave").after(c);
    vlBind(row, row.dataset.name);
  } else if (!VL.on && c) {
    if (c.width) { c.width = 0; c.height = 0; }
    c.remove();
  }
  for (const hd of row.querySelectorAll(".rowResize")) {   // #437: the handles' text follows the lane it drags (the row's border and the header's edge, #439)
    hd.title = VL.on ? "Drag: the height of the velocity lane (the handle is at its bottom edge). Double-click: the wave lane back to the Track Height"
                     : "Drag: the height of this track's wave lane. Double-click: back to the Track Height";
  }
}
function vlYOf(v, H) { return Math.round(H - (v - 1) / 126 * (H - 4) - 2); }   // the y of a velocity (2 px padding)
function vlVOf(y, H) { return Math.max(1, Math.min(127, Math.round(1 + (H - 2 - y) / (H - 4) * 126))); }
function drawVelLane(canvas, name) {
  const ctx = canvas.getContext("2d"), [W, H] = laneSize(canvas);
  ctx.clearRect(0, 0, W, H);
  ctx.fillStyle = "rgba(0,0,0,0.22)"; ctx.fillRect(0, 0, W, H);
  const color = S.tracks[name].color;
  // the floor and the ceiling of the loudness settings (P4-3): the ramp maps them to the velocities 20 and 127
  ctx.strokeStyle = "rgba(138,147,163,0.4)"; ctx.setLineDash([3, 3]); ctx.lineWidth = 1;
  for (const v of [20, 127]) { const y = vlYOf(v, H) + 0.5; ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(W, y); ctx.stroke(); }
  ctx.setLineDash([]);
  // #208: the scale at the right edge (127 top, 64 middle, 0 bottom) and the name at the left
  ctx.fillStyle = "rgba(138,147,163,0.9)"; ctx.font = "10px sans-serif"; ctx.textBaseline = "middle";
  // the header (127, Velocity, the hint) hangs below the 127 line, so its full glyph height is inside the lane (#208)
  const headY = vlYOf(127, H) + 4;
  ctx.textBaseline = "top";
  ctx.textAlign = "right"; ctx.fillText("127", W - 3, headY);
  ctx.textAlign = "left"; ctx.fillText("Velocity", 4, headY);
  ctx.textBaseline = "middle";
  ctx.textAlign = "right"; ctx.fillText("64", W - 3, vlYOf(64, H)); ctx.fillText("0", W - 3, H - 6);
  const evs = S.events[name] || [];
  const range = LN.sel && LN.sel.track === name ? LN.sel : null;
  for (const i of hitsIn(evs, S.view.t0 - 8 * viewDur() / W, S.view.t1 + 8 * viewDur() / W, dragSkip(name))) {
    const e = evs[i], x = Math.round(tToX(e.t, W));
    if (x < -2 || x > W + 2) continue;
    const out = hgIsCut(e), sel = S.sel && S.sel.track === name && S.sel.index === i;
    const inRange = !!range && e.t >= range.t0 && e.t <= range.t1;
    ctx.fillStyle = out ? HG_GREY : e.edited || isManual(e) ? MANUAL_COLOR : color;
    const y = vlYOf(e.v, H);
    ctx.fillRect(x - 1, y, 2, H - y);
    ctx.beginPath(); ctx.arc(x, y, 2.2, 0, 2 * Math.PI); ctx.fill();   // the dot at the stalk top
    if (inRange) { ctx.fillStyle = "#ffd24d"; ctx.beginPath(); ctx.arc(x, y, 2.6, 0, 2 * Math.PI); ctx.fill(); }   // a hit of the selected range
    if (sel) { ctx.strokeStyle = "#ffd24d"; ctx.lineWidth = 1.5; ctx.strokeRect(x - 3, y - 3, 6, H - y + 3); ctx.lineWidth = 1; }
  }
  if (VL.drag && VL.drag.track === name && VL.drag.mode === "line") {   // the line the Option drag draws
    const d = VL.drag, x0 = tToX(d.t0, W), x1 = d.curX;
    ctx.strokeStyle = "rgba(255,255,255,0.55)"; ctx.beginPath(); ctx.moveTo(x0, d.y0); ctx.lineTo(x1, d.curY); ctx.stroke();
  }
  const hv = VL.hover && VL.hover.track === name ? evs.find(x => x.id === VL.hover.id) : null;
  if (hv) {   // the hover label: the velocity, the time, and why a grey stalk is grey
    const x = Math.round(tToX(hv.t, W));
    const txt = `velocity ${hv.v} · ${fmt(hv.t)} s${hgIsCut(hv) ? " · grey: leaves the output (cut)" : ""}`;
    const tw = ctx.measureText(txt).width + 8, bx = Math.max(2, Math.min(x + 6, W - tw - 2));
    ctx.fillStyle = "rgba(16,20,28,0.92)"; ctx.fillRect(bx, H - 15, tw, 13);
    ctx.fillStyle = "#e6e9ef"; ctx.textAlign = "left"; ctx.fillText(txt, bx + 4, H - 8);
  }
  if (VL.hint && W > 420) {   // the one-line hint until the first gesture
    ctx.fillStyle = "rgba(230,233,239,0.75)"; ctx.textAlign = "left";
    ctx.textBaseline = "top";
    ctx.fillText("Drag a stalk to set velocity. Option-drag: draw a line. Cmd-drag: draw freehand.", 64, headY);
    ctx.textBaseline = "middle";
  }
  ctx.textAlign = "left";
}
/* The gestures of one row's velocity lane. A gesture previews on the page's copy and commits once: one events_edit call,
   one undo step. `gen` guards the commit like a hit drag does: the hits changed meanwhile, the gesture is dropped. */
function vlBind(row, name) {
  const c = row.querySelector(".vel");
  if (!c) return;   // the lane is off: the row has no vel canvas (vlSyncRow binds when it appears)
  const evs = () => S.events[name] || [];
  const hitAt = px => {
    const W = laneWidth(), t = xToT(px, W), d = 4 * viewDur() / W;
    for (const i of hitsIn(evs(), t - d, t + d, dragSkip(name))) if (Math.abs(tToX(evs()[i].t, W) - px) < 4) return i;
    return -1;
  };
  const preview = (e, v) => { e.v = v; };
  const commit = () => {
    const d = VL.drag; VL.drag = null;
    if (!d) return;
    VL.hint = false;
    const list = [...d.set.entries()].map(([id, [from, to, was]]) => ({ id, from, to, ...(was ? { was } : {}) })).filter(s => s.to !== s.from);
    if (!list.length || d.gen !== (EDGEN[name] || 0)) { invalidateHits(name); if (list.length) setMsg("the hits changed during the drag: the velocities were dropped"); return; }
    const step = list.length === 1
      ? { type: "setv", track: name, id: list[0].id, from: list[0].from, to: list[0].to, ...(list[0].was ? { was: list[0].was } : {}) }
      : { type: "setv-many", track: name, list };
    edCall(name, evs2 => {
      const ops = [];
      for (const s of list) { const j = edIndex(evs2, s.id); if (j < 0) { setMsg("edit: the hit is not there any more"); return null; } ops.push({ op: "velocity", index: j, v: s.to, edited: true }); }
      return { ops, step };   // P4-40f: one gesture, one undo step
    }).then(ok => { if (!ok) return edReload(S.sid, name); invalidateHits(name); updateStatus(); });
  };
  const setRange = (x0, v0, x1, v1) => {   // the Option drag: a straight line from (x0, v0) to (x1, v1) over all hits between
    const W = laneWidth(), evsL = evs();
    const fOf = x => (x1 === x0 ? 1 : (x - x0) / (x1 - x0));
    const ta = Math.min(xToT(x0, W), xToT(x1, W)), tb = Math.max(xToT(x0, W), xToT(x1, W));
    for (const i of hitsIn(evsL, ta, tb, -1)) {
      const v = Math.round(v0 + (v1 - v0) * fOf(tToX(evsL[i].t, W)));
      const id = evsL[i].id;
      if (!VL.drag.set.has(id)) VL.drag.set.set(id, [evsL[i].v, evsL[i].v, hitWas(evsL[i])]);   // R05: the field presence before the gesture
      preview(evsL[i], v); VL.drag.set.get(id)[1] = v;
    }
  };
  c.addEventListener("mousedown", ev => {
    if (ev.button === 2 || !VL.on || !S.sid || followerLocked(name)) return;
    ev.preventDefault();
    const px = laneXOf(ev), [W, H] = laneSize(c), hi = hitAt(px);
    const mode = ev.metaKey || ev.ctrlKey ? "free" : ev.altKey ? "line" : "stalk";
    if (mode === "stalk" && hi < 0) return;
    const t0 = xToT(px, W);
    const rect = c.getBoundingClientRect();   // cached: the row does not move during a gesture (offsetY lies once the pointer leaves the canvas)
    VL.drag = { track: name, mode, gen: EDGEN[name] || 0, x0: px, y0: ev.offsetY, curX: px, curY: ev.offsetY, t0, set: new Map(), rect };
    if (hasPendingWrite(name)) pushEvents(name).catch(() => {});   // a write of another step goes out before the drag moves the page's copy; #249 R02: a refusal shows on the status line
    if (mode === "stalk") {
      const e = evs()[hi], grp = vlGroup(name, e);   // #208: a selected stalk moves the selection, scaled
      VL.drag.anchor = e.id; VL.drag.v0 = e.v; VL.drag.base = grp.length > 1 ? new Map(grp.map(g => [g.id, g.v])) : null;
      for (const g of grp) VL.drag.set.set(g.id, [g.v, g.v, hitWas(g)]);
      preview(e, vlVOf(ev.offsetY, H)); VL.drag.set.get(e.id)[1] = e.v;
      if (VL.drag.base) vlScale(VL.drag, evs(), VL.drag.set.get(e.id)[1]);
    }
    invalidateHits(name);
    const move = e2 => {
      const d = VL.drag; if (!d || d.track !== name) return;
      d.curX = e2.clientX - d.rect.left; d.curY = e2.clientY - d.rect.top;
      const evsL = evs(), W2 = laneWidth();
      if (d.mode === "line") {
        setRange(tToX(d.t0, W2), vlVOf(d.y0, H), d.curX, vlVOf(d.curY, H));
      } else if (d.mode === "free") {
        const v = vlVOf(d.curY, H);
        for (const i of hitsIn(evsL, S.view.t0 - 1e-6, S.view.t1 + 1e-6, -1)) {   // the hits crossed since the last move
          const x = tToX(evsL[i].t, W2);
          if (Math.abs(x - d.curX) < 4) {
            const id = evsL[i].id;
            if (!d.set.has(id)) d.set.set(id, [evsL[i].v, evsL[i].v]);
            preview(evsL[i], v); d.set.get(id)[1] = v;
          }
        }
      } else if (d.base) {   // #208: the selected hits scale by the ratio of the dragged stalk
        const i = evsL.findIndex(x => x.id === d.anchor);
        if (i >= 0) { preview(evsL[i], vlVOf(d.curY, H)); d.set.get(d.anchor)[1] = evsL[i].v; vlScale(d, evsL, evsL[i].v); }
      } else {
        const i = evsL.findIndex(x => x.id === d.anchor);
        if (i >= 0) { preview(evsL[i], vlVOf(d.curY, H)); d.set.get(d.anchor)[1] = evsL[i].v; }
      }
      invalidateHits(name);
      if (d.mode === "stalk") {   // the value while the stalk moves
        const n = [...d.set.values()].filter(s => s[1] !== s[0]).length;
        $("#stSel").textContent = d.base
          ? `${name}: velocity ${d.set.get(d.anchor)[1]}, ${d.base.size} hits scaled by ${(d.set.get(d.anchor)[1] / d.v0).toFixed(2)}`
          : `${name}: velocity ${d.set.get(d.anchor)[1]}`;
        return;
      }
      const n = [...d.set.values()].filter(s => s[1] !== s[0]).length;
      $("#stSel").textContent = `${name}: Set Velocity (${n} ${n === 1 ? "hit" : "hits"})`;
    };
    const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); commit(); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  });
  c.addEventListener("mousemove", ev => {
    const hi = VL.on ? hitAt(laneXOf(ev)) : -1;
    c.classList.toggle("onStalk", hi >= 0);
    const id = hi >= 0 ? evs()[hi].id : null;
    const was = VL.hover;
    if (id !== (was && was.track === name ? was.id : null)) {   // the hover label follows the stalk under the pointer
      VL.hover = id ? { track: name, id } : null;
      if (was || id) drawVelLane(c, name);
    }
    curOver(false, ev);
  });
  c.addEventListener("mouseleave", () => {
    c.classList.remove("onStalk", "onTop");
    if (VL.hover && VL.hover.track === name) { VL.hover = null; drawVelLane(c, name); }
  });
}
/* #437: the lane's resize handle is the row's bottom border (the .rowResize div sits on the velocity
   lane's bottom edge while the lane is on); it drags the lane's height, like the wave lane's row drag. */
function vlResize(ev) {   // the drag of the bottom border: the lane grows when the pointer goes down; saved when let go
  const y0 = ev.clientY, h0 = VL.h;
  const move = e2 => {
    VL.h = Math.max(VL_H_MIN, Math.min(VL_H_MAX, h0 + (e2.clientY - y0)));
    invalidate(F.SIZE | F.VIEW | F.GL);
  };
  const up = () => {
    window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up);
    if (B.appSettingsSet) B.appSettingsSet("velocity_lane_height", VL.h).catch(e => setMsg(`settings: ${errText(e)}`));
  };
  window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
}
/* #208: the hits a stalk drag moves: the hits of the selected range (or the one selected hit) when the stalk is one of
   them; else the stalk alone. */
function vlGroup(name, e) {
  const evsL = S.events[name] || [];
  let list;
  if (LN.sel && LN.sel.track === name) list = evsL.filter(x => x.t >= LN.sel.t0 && x.t <= LN.sel.t1);
  else if (S.sel && S.sel.track === name && evsL[S.sel.index]) list = [evsL[S.sel.index]];
  else list = [];
  return list.includes(e) && list.length > 1 ? list : [e];
}
/* #208: the scale of a stalk drag: every selected hit gets its velocity times the ratio of the dragged stalk, rounded
   and kept in 1..127; the dragged hit gets the value of the pointer. */
function vlScale(d, evsL, newV) {
  const ratio = newV / d.v0;
  for (const [id, base] of d.base) {
    if (id === d.anchor) continue;
    const i = evsL.findIndex(x => x.id === id);
    if (i < 0) continue;
    const v = Math.max(1, Math.min(127, Math.round(base * ratio)));
    evsL[i].v = v; d.set.get(id)[1] = v;
  }
}

function attachTrackHandlers(row, name) {
  const spike = row.querySelector(".spike"), wave = row.querySelector(".wave");
  const evs = () => S.events[name] || [];
  function hitEvent(px) {   // W6: the hits within 4 px (binary search)
    const evsL = evs(), W = laneWidth(), t = xToT(px, W), d = 4 * viewDur() / W;
    for (const i of hitsIn(evsL, t - d, t + d, dragSkip(name))) {
      const x = tToX(evsL[i].t, W);
      if (Math.abs(x-px) < 4) return i;
    }
    return -1;
  }
  let dragState = null;
  function startPanGesture(ev) {
    S.followHold = true;   // F1: user view change pauses follow until next play
    const startX = laneXOf(ev), startCX = ev.clientX, t0 = S.view.t0;
    let moved = false;
    const mv = ev2 => {
      const dx = ev2.clientX - startCX;
      if (Math.abs(dx) > 3) moved = true;
      if (moved) {
        const dt = dx / laneWidth() * viewDur();
        setView(t0 - dt, t0 - dt + viewDur());
      }
    };
    const up = () => {
      window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up);
      if (!moved) {
        S.playhead = snapPlayhead(xToT(startX, laneWidth()));
        lnClearSel();   // P4-6a: a click clears the range selection
        invalidate(F.HEAD); updateStatus();
      }
    };
    window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
  }
  /* W7: the end of a hit drag listens on the window only while the drag runs (before P3-14d each track added one
     window listener per project open and never removed it) */
  function endHitDrag() {   // P4-9 stage 2: the drag was a preview on the page's copy; one events_edit move on release
    window.removeEventListener("mouseup", endHitDrag);
    const ds = dragState; dragState = null;
    if (!ds) return;
    const e = evs()[ds.i];
    if (!ds.moved || !e || ds.gen !== (EDGEN[name] || 0) || !hitDrag) { hitDrag = null; invalidateHits(name); if (ds.moved) { updateStatus(); setMsg("the hits changed during the drag: the move was dropped"); } return; }   // void: the list was replaced meanwhile
    const id = e.id, to = e.t, from = ds.origT, i = ds.i;
    edCall(name, evs2 => {   // the preview keeps the hit at the pointer until the stored hits arrive; its index is the stored one
      if ((EDGEN[name] || 0) !== ds.gen || !evs2[i] || evs2[i].id !== id) { if (to !== from) setMsg("the hits changed during the drag: the move was dropped"); return null; }
      if (to === from) return null;
      return { ops: [{ op: "move", index: i, t: to }], drop: [id], make: [[to, id]], sel: id, drag: true, step: { type: "move", track: name, id, from, to, was: hitWas(evs2[i]) } };
    }, () => { e.t = from; }).then(ok => { if (!ok) return edReload(S.sid, name); }).finally(() => { hitDrag = null; invalidateHits(name); updateStatus(); });
  }
  spike.addEventListener("mousedown", ev => {
    if (ev.button === 2) return;
    ev.preventDefault();   // no text selection during event drags / pans
    const px = laneXOf(ev);
    if (ev.altKey && ev.shiftKey) {   // P4-40b: Option+Shift-click on a hit deletes it (one undo step, as Delete); off a hit it stays the Shift click
      const hi = hitEvent(px);
      if (hi >= 0) { if (!followerLocked(name)) edDelete(name, evs()[hi].id); return; }
    }
    if (ev.shiftKey) { lnClearSel(); S.playhead = snapPlayhead(xToT(px, laneWidth())); invalidate(F.HEAD); updateStatus(); return; }
    if (ev.altKey) { addHitAt(name, snapSample(xToT(px, laneWidth()), name), true); return; }   // P3-13b: Option-click adds a hit (Cubase); #144: on the attack
    const i = hitEvent(px);
    if (i < 0) {   // P4-6a: a click on a Learn mark bar selects the mark (Delete removes it)
      const lm = lnMarkAt(name, px, ev.offsetY);
      if (lm) { lnClearSel(); S.sel = null; lnSetMark(name, lm.id); updateStatus(); setMsg(`${name}: mark ${lm.id} (${lm.kind === "not" ? "not this" : "this"}, ${(lm.at || []).length} ${(lm.at || []).length === 1 ? "example" : "examples"}); Delete removes it, right-click for more`); return; }
    }
    lnClearSel(); if (LN.mark) lnSetMark(null);
    if (i >= 0 && followerLocked(name)) { S.sel = {track:name, index:i}; invalidateHits(name); updateStatus(); return; }   // P3-18: select, no drag
    if (i >= 0) {
      S.sel = {track:name, index:i};
      if (SH.pending || hgRepickBusy()) { invalidateHits(name); updateStatus(); return; }   // an edit is on its way: select, no drag (the hits on the page are not the stored ones yet)
      if (hasPendingWrite(name)) pushEvents(name).catch(() => {});   // a write of another step goes out before the drag moves the page's copy; #249 R02: a refusal shows on the status line
      dragState = {i, startX: px, origT: evs()[i].t, moved:false, sr: S.tracks[name].sr, gen: EDGEN[name] || 0};
      hitDrag = { track: name, index: i };
      window.addEventListener("mouseup", endHitDrag);
      invalidateHits(name); updateStatus();
      return;
    }
    startPanGesture(ev);   // empty spike area: click = playhead, drag = pan
  });
  spike.addEventListener("mousemove", ev => {
    const px = laneXOf(ev);
    if (!dragState) {   // P4-40b: the cursor comes from CSS classes (.onHit; the modifier classes of #lanesScroll)
      const on = hitEvent(px) >= 0;
      spike.classList.toggle("onHit", on); curOver(on && !followerLocked(name), ev); return;
    }
    const dt = xToT(px, laneWidth()) - xToT(dragState.startX, laneWidth());
    if (Math.abs(px-dragState.startX) > 2) dragState.moved = true;
    const i = dragState.i, evsL = evs();
    if (!evsL[i] || dragState.gen !== (EDGEN[name] || 0)) {   // the list was replaced: the drag is void (and says so, as at release)
      const moved = dragState.moved; dragState = null; hitDrag = null; invalidateHits(name);
      if (moved) setMsg("the hits changed during the drag: the move was dropped");
      return;
    }
    evsL[i].t = snapSample(dragState.origT + dt, name);
    invalidateHits(name);
    const dSmp = Math.round((evsL[i].t - dragState.origT)*dragState.sr);
    $("#stSel").textContent = `${name}: moving ${dSmp>0?"+":""}${dSmp} smp`;
    drawInfo();
  });
  spike.addEventListener("mouseleave", () => { spike.classList.remove("onHit"); curOver(false); });
  spike.addEventListener("dblclick", ev => addHitAt(name, snapSample(xToT(laneXOf(ev), laneWidth()), name)));
  spike.addEventListener("contextmenu", ev => {
    const i = hitEvent(laneXOf(ev));
    if (i < 0) { lnContext(ev, name, "spike"); ev.preventDefault(); return; }   // P4-6a: the menus of a mark bar and of the selection; no browser menu on an empty lane
    ev.preventDefault();
    if (followerLocked(name)) return;   // P3-18: a follower's hits are not deleted here
    edDelete(name, evs()[i].id);   // P4-9 stage 2: events_edit delete
  });
  wave.addEventListener("mousedown", ev => {
    if (ev.button!==0) return;
    ev.preventDefault();
    const px = laneXOf(ev);
    if (ev.shiftKey) { lnClearSel(); S.playhead = snapPlayhead(xToT(px, laneWidth())); invalidate(F.HEAD); updateStatus(); return; }
    if (ev.altKey) { lnClearSel(); addHitAt(name, snapSample(xToT(px, laneWidth()), name), true); return; }     // P3-13b: Option-click adds a hit; #144: on the attack
    lnRangeGesture(ev, name);   // P4-6a: click = playhead, drag = select a range (the pan stays on the spike lane, the wheel and the overview)
  });
  wave.addEventListener("mousemove", ev => curOver(false, ev));   // P4-40b
  wave.addEventListener("contextmenu", ev => { lnContext(ev, name, "wave"); });   // P4-6a: inside the selection
  for (const c of [spike, wave]) c.addEventListener("wheel", ev => {
    ev.preventDefault();
    const dY = ev.deltaY * dScale(ev.deltaMode), dX = ev.deltaX * dScale(ev.deltaMode);
    const secPerPx = viewDur() / laneWidth();
    // P4 #145: Cmd+wheel and the trackpad pinch (a ctrl+wheel) zoom around the pointer; one wheel notch (100 px)
    // is a factor of about 1.6, a pinch moves in small steps. nativeFeel stops the page zoom of both.
    if (ev.metaKey || ev.ctrlKey) { zoomAtX(laneXOf(ev), laneWidth(), 2 ** Math.max(-1, Math.min(1, dY / 150))); return; }
    // horizontal swipe or shift+wheel pans the timeline; vertical wheel scrolls the track list
    if (Math.abs(dX) > Math.max(2, Math.abs(dY)) || ev.shiftKey) panView((dX || dY) * secPerPx);
    else $("#lanesScroll").scrollBy({ top: dY });
  }, {passive:false});

  row.querySelector(".bMute").onclick = ev => { S.muted[name] = !S.muted[name]; row.classList.toggle("muted", S.muted[name]); ev.target.classList.toggle("on", S.muted[name]); applyMixer(); };
  row.querySelector(".bSolo").onclick = ev => { S.solo[name] = !S.solo[name]; row.classList.toggle("solo", S.solo[name]); ev.target.classList.toggle("on", S.solo[name]); applyMixer(); };
}

/* P3-14e: the track header context menu (Export…, Re-analyze, Reset Volume and Pan); the Track and
   Hits menus run the same actions for the selected track. P3-14a: Params is gone, the Inspector holds the parameters. */
let ctxAway = null;
function closeTrackMenu() {
  const m = document.getElementById("ctxMenu"); if (m) m.remove();
  if (ctxAway) { document.removeEventListener("mousedown", ctxAway, true); ctxAway = null; }
}
function openTrackMenu(name, x, y) {
  const items = [
    ["export", "Export…", () => openExport("selected", name)],   // P4-30: the one Export dialog with this track
    ["reanalyze", "Re-analyze", () => startAnalysis([name])],
    ["mix", "Reset Volume and Pan", () => { setTrackMix(name, 0, 0); setMsg(`${name}: volume 0 dB, pan centre`); }],   // P3-16
  ];
  openCtxItems(items, x, y);
}
function openCtxItems(items, x, y) {   // [act, label, run] rows at a point (P4-6a: the Learn menus use it too)
  closeTrackMenu();
  const m = document.createElement("div"); m.id = "ctxMenu";
  for (const [act, label, run] of items) {
    const b = document.createElement("button"); b.dataset.act = act; b.textContent = label;
    if (act === "reanalyze") b.disabled = !!$("#analyzeAllBtn").disabled;
    b.onclick = () => { closeTrackMenu(); run(); };
    m.appendChild(b);
  }
  document.body.appendChild(m);
  m.style.left = Math.max(0, Math.min(x, innerWidth - m.offsetWidth)) + "px";
  m.style.top = Math.max(0, Math.min(y, innerHeight - m.offsetHeight)) + "px";
  ctxAway = ev => { if (!m.contains(ev.target)) closeTrackMenu(); };
  document.addEventListener("mousedown", ctxAway, true);
}
document.addEventListener("keydown", ev => { if (ev.key === "Escape") closeTrackMenu(); }, true);

/* ---------------- params panel: the Detection, Timing, Velocity and Output sections of the Inspector (P3-14a;
   before that a drawer under the lane) ---------------- */
function insSecHTML(id, title, body) {   // P4-25: a collapsible section of the Inspector (state: INSSEC, insSecApply)
  return `<section class="insSec" data-sec="${id}"><button type="button" class="insSecHead" aria-expanded="false"><span class="insTri" aria-hidden="true"></span><span class="insSecTitle">${title}</span><span class="insSum"></span></button><div class="insSecBody">${body}</div></section>`;
}
/* P4-25 (owner): the native spin buttons of a number field did not work and were hard to hit and to read on the dark panel.
   Every number field of the panel gets a minus and a plus button (20 px wide) and Up/Down keys (Shift: x10), with the
   field's own step, min and max; each step ends in a "change" event, so the field's handler runs as for a typed value. */
function insStep(inp, dir, mult) {
  const step = (parseFloat(inp.step) || 1) * (mult || 1), min = inp.min === "" ? -Infinity : +inp.min, max = inp.max === "" ? Infinity : +inp.max;
  const cur = parseFloat(inp.value), base = Number.isFinite(cur) ? cur : (max < 0 ? max : Math.min(Math.max(0, min), max));
  let v = Number.isFinite(cur) ? base + dir * step : base;   // an empty field (no cut) starts at its first sensible value
  v = Math.max(min, Math.min(max, Math.round(v * 1e6) / 1e6));
  if (inp.disabled || String(v) === inp.value) return;
  inp.value = String(v);
  inp.dispatchEvent(new Event("input", { bubbles: true })); inp.dispatchEvent(new Event("change", { bubbles: true }));
}
function insSteppers(div) {
  for (const inp of div.querySelectorAll("input[type=number]")) {
    const w = document.createElement("span"); w.className = "stepper";
    inp.parentNode.insertBefore(w, inp);
    const mk = (dir, glyph, label) => { const b = document.createElement("button"); b.type = "button"; b.className = dir > 0 ? "stepBtn stepUp" : "stepBtn stepDn"; b.textContent = glyph; b.tabIndex = -1; b.setAttribute("aria-label", label);
      b.onclick = ev => insStep(inp, dir, ev.shiftKey ? 10 : 1); return b; };
    w.append(mk(-1, "\u2212", "Decrease"), inp, mk(1, "+", "Increase"));
    inp.addEventListener("keydown", ev => {
      if (ev.key !== "ArrowUp" && ev.key !== "ArrowDown") return;
      ev.preventDefault(); ev.stopPropagation(); insStep(inp, ev.key === "ArrowUp" ? 1 : -1, ev.shiftKey ? 10 : 1);
    });
  }
}
function buildParamsPanel(div, name, p) {
  const t = S.tracks[name];
  const info = txt => ` <span class="pinfo" title="${txt}">ⓘ</span>`;
  const sec = insSecHTML;
  div.innerHTML = `
    <div class="pbody">
      ${sec("detection", "Detection", `
      <div class="prow">
        <label>Hits ${info("Own detection: the onset detector finds the hits of this track. Follow: the track takes the hits of another track, sample-exact, with velocities from its own signal (a kick sub mic follows the kick in). A follower is read-only in the lanes; its exports and its click use the followed hits.")}</label>
        <div class="pctl"><select class="pFollow"></select></div>
      </div>
      <div class="prow">
        <label>Detector ${info("Which detector runs on this track: Built-in (the DSP onset detector) or one of the installed plug-ins that support this track's role and rate. A plug-in's licence is shown and accepted before its first use. Applies on the next re-analyze.")}</label>
        <div class="pctl"><select class="pDetector" title="The detector of this track"><option value="">Built-in</option></select></div>
      </div>
      <div class="prow pDetectorNoteRow hidden"><label></label><div class="pctl"><span class="phint pDetectorNote"></span></div></div>
      <div class="prow">
        <label class="hgHead">Sensitivity ${info("Scales the CRNN neural-network onset thresholds (default 0.22–0.32). Higher sensitivity lowers the thresholds: quieter, softer hits are detected — but bleed and noise produce more false hits. Lower sensitivity only catches clear attacks. From 100 on the thresholds go down to 0.01 for soft tracks (a kick sub mic). A track with no hit at its sensitivity is tried again at 95, 113, 134 and 150 (on while the hit count grows by more than 10 %); the value kept is stored here.")}<span class="phint hgSensCount"></span></label>
        <div class="pctl pctlWide hgCol">
          <div class="hgWrap hgSensWrap" title="Every candidate hit of this track by its strength.">
            <canvas class="hgCanvas hgSensCanvas"></canvas>
          </div>
          <div class="hgMarkRow"><button type="button" class="hgGapBtn hidden" aria-label="Snap into the gap"></button></div>
          <div class="hgSliderRow"><input type="range" class="pSens hgRange" min="0" max="150" value="50" title="Sensitivity: the threshold line of the histogram"></div>
          <span class="phint pSensVal hidden"></span>
        </div>
      </div>
      <div class="prow pSensNoteRow hidden"><label></label><div class="pctl"><span class="phint pSensNote"></span></div></div>
      <div class="prow">
        <label>Minimum gap ${info("Two detections closer together than this are merged into one (the earlier one wins). Raise it if one hit is detected twice (e.g. snare bleed into the kick mic); lower it for very fast playing such as rolls. Default 20 ms.")}</label>
        <div class="pctl"><input type="number" class="pGap" min="1" max="500" value="${p.min_gap_ms}"><span class="punit">ms</span></div>
      </div>
      <div class="prow">
        <label>Refine to attack ${info("After detection, each hit is moved to the first sample crossing 25% of the local peak — the physical stick impact, sample-exact. Leave this on; disable only to compare raw detector output.")}</label>
        <div class="pctl"><input type="checkbox" class="pRefine" ${p.refine?"checked":""}></div>
      </div>
      ${t.role === "snare" ? `<div class="prow">
        <label>Split flams ${info("A flam or two strokes closer than about 30 ms look like one hit to the detector. On (the default for snare tracks): the second stroke inside a hit becomes a hit of its own, unless another mic of the project explains it as bleed. Off: one hit per flam. Applies on the next re-analyze.")}</label>
        <div class="pctl"><input type="checkbox" class="pFlam" ${p.flam_split !== false ? "checked" : ""}></div>
      </div>` : ""}
      <div class="pactions">
        <button class="btn primary bReanalyze" title="Re-run detection on this track. Edited hits (magenta) are offered to be kept.">↻ Re-analyze</button>
        <span class="phint">detection changes apply on the next re-analyze</span>
      </div>
      `)}
      ${sec("velocity", "Velocity", `
      <div class="prow hgLoudRow">
        <div class="pctl pctlWide hgCol">
          <div class="hgWrap hgLoudWrap" title="The peak level of every hit of this track (dBFS). Drag the two handles: the floor (velocity 20) and the ceiling (velocity 127); the line is the velocity ramp between them. The velocities follow live; they are stored when you let go (one undo step).">
            <canvas class="hgCanvas hgLoudCanvas"></canvas>
          </div>
          <span class="phint hgLoudInfo"></span>
        </div>
      </div>
      <div class="prow">
        <label>Velocity floor ${info("Hits with a peak at or below this dBFS level get velocity 20 (softest). Between floor and ceiling the velocity interpolates linearly with the peak level in dB. Default -40.")}</label>
        <div class="pctl"><input type="number" class="pVLo" value="${p.vel_lo_db}" step="1"><span class="punit">dBFS → vel 20</span></div>
      </div>
      <div class="prow">
        <label>Velocity ceiling ${info("Hits with a peak at or above this dBFS level get velocity 127 (full). Default -6. The floor…ceiling range is stretched to MIDI 20…127.")}</label>
        <div class="pctl"><input type="number" class="pVHi" value="${p.vel_hi_db}" step="1"><span class="punit">dBFS → vel 127</span></div>
      </div>
      <div class="prow">
        <label>Loudness cut ${info("Hits with a peak below this dBFS level are cut: grey in the lane and the histogram, and left out of MIDI, spikes, Export All, the click and a follower (quiet bleed). They stay in the project; clear the field to bring them back. Edited hits are never cut. Empty = no cut (the default). The orange handle of the histogram sets the same.")}</label>
        <div class="pctl"><input type="number" class="pVCut" value="${Number.isFinite(p.vel_cut_db) ? p.vel_cut_db : ""}" step="1" max="0" placeholder="off"><span class="punit">dBFS</span></div>
      </div>
      `)}
      ${sec("timing", "Timing", `
      <div class="prow">
        <label>Hitpoint shift ${info("Shifts every hitpoint of this track by a fixed number of samples — positive = later, negative = earlier. Changing it shifts the current hits immediately by the difference and is stored for the next re-analysis. Use it to compensate a fixed latency offset of this track.")}</label>
        <div class="pctl"><input type="number" class="pShift" value="${Math.round(p.shift_samples||0)}" step="1" min="-100000" max="100000"><span class="punit">samples</span></div>
      </div>
      `)}
      ${sec("learn", "Learn", `
      <div class="prow lnRow">
        <label>Examples ${info("Mark good and bad hits anywhere on the track: drag over a stretch of the wave lane (or click a hit), then Learn This (K) or Forget This (Shift+K). A range takes its hits; a ghost note under the Sensitivity threshold is found too. The marks are kept in the project. With the Filter on, hits unlike your examples go grey at once: they stay in the lane, Tab skips them, and they leave MIDI, spikes and the click.")}</label>
        <div class="pctl pctlWide lnCol">
          <span class="phint lnCount"></span>
          <span class="phint lnHintMsg hidden"></span>
          <div class="lnBtns"><button class="btn mini lnThis" title="Learn This (K): the selected range or hit is a good example">Learn This</button><button class="btn mini lnNot" title="Forget This (Shift+K): bleed, or a ghost note to drop">Forget This</button><button class="btn mini lnForgetThis" title="Remove Example (Option+K): removes the Learn example at the selected hit or range, or the selected mark">Remove Example</button><button class="btn mini lnForget" title="Forget Examples of Track: removes every example of the track (Undo brings them back)">Forget All</button><label class="lnFilter" title="Learn Filter: hits unlike the examples go grey at once. They stay in the lane, Tab skips them, and they leave MIDI, spikes and the click. The first Forget This (Shift+K) turns it on. Undo brings the old state back."><input type="checkbox" class="lnFilterBox"> Filter</label></div>
          <canvas class="lnHist" height="60" title="The similarity scores of the detection candidates (50 bins). Solid: they pass the Sensitivity. Outlined: under it. Green and red ticks: the examples. Drag the red line: the threshold (one undo step on release)."></canvas>
          <div class="lnHistRow"><span class="lnHistInfo"></span><a href="#" class="lnAuto" title="The automatic threshold from the scores (the stage A rule)">Auto</a></div>
          <details class="lnList"><summary>Marks</summary><div class="lnRows"></div></details>
        </div>
      </div>
      `)}
      ${sec("output", "Output", `
      <div class="prow">
        <label>MIDI note ${info("General MIDI note of every hit on this track: kick C1 (36), snare D1 (38), toms C2–G2 (41–50), closed hat D#1 (42), crash C#2 (49). Name and number stay in sync.")}</label>
        <div class="pctl"><input type="text" class="pNote" value="${nameFromNote(t.note)}" style="width:70px"><input type="number" class="pNoteNum" min="0" max="127" value="${t.note}" style="width:70px"></div>
      </div>
      <div class="insMixSlot"></div>`)}
      ${sec("layer", "Layer", lyHTML())}
    </div>`;
  const sens = div.querySelector(".pSens");
  const cur = Array.isArray(p.thresholds) && p.thresholds.length === 5 ? p.thresholds : SENS.BASE;
  const sensVal = div.querySelector(".pSensVal");
  const showSens = v => { sensVal.textContent = sensReadout(v); };
  // P3-18: the slider stands at the stored sensitivity, else at the value of the stored thresholds (the readout of
  // P3-14a divided by 0.32 and showed 66 with the default thresholds; the defaults are 50)
  sens.value = Number.isFinite(p.sensitivity) ? Math.round(p.sensitivity) : Math.round(sensOf(cur));
  showSens(+sens.value);
  const noteRow = div.querySelector(".pSensNoteRow"), note = div.querySelector(".pSensNote");
  const showNote = () => { const txt = weakSignalText(p.weak_signal); note.textContent = txt; noteRow.classList.toggle("hidden", !txt); };
  showNote();
  sens.oninput = () => {
    p.thresholds = sensThresholds(+sens.value);
    p.sensitivity = +sens.value;
    delete p.weak_signal;   // the note of the last analysis is stale once the user sets the sensitivity
    showSens(+sens.value); showNote();
    hgSensInput(name, p);   // P4-3: the histogram colours and the live re-pick
  };
  sens.onchange = () => hgSensCommit(name, p);   // P4-3: let go: the re-pick is stored (one undo step)
  // P3-18: the follow of this track: own detection or one of the other tracks
  const follow = div.querySelector(".pFollow");
  const own = document.createElement("option"); own.value = ""; own.textContent = "own detection"; follow.appendChild(own);
  for (const n of S.order) {
    if (n === name) continue;
    const o = document.createElement("option"); o.value = n; o.textContent = `follow ${n}`; follow.appendChild(o);   // names are data
  }
  follow.value = t.follows && S.tracks[t.follows] ? t.follows : "";
  follow.onchange = () => setTrackFollow(name, follow.value || null);
  // #387: the Detector select (a follower has no own detection: no select, no note)
  const detSel = div.querySelector(".pDetector");
  if (t.follows) { detSel.closest(".prow").classList.add("hidden"); div.querySelector(".pDetectorNoteRow").classList.add("hidden"); }
  else {
    detSel.onchange = async () => {
      const id = detSel.value;
      if (!id) { p.detector = "builtin"; setDetectorNote(name, ""); return; }
      const list = await pluginsInstalled();
      const pl = pluginFor(list, id);
      if (!pl) { detSel.value = trackDetectorParam(name) === id ? id : ""; return; }
      if (pl.accepted !== true) { plAskLicence(name, id, detSel); return; }   // the licence dialog before the first use
      p.detector = id;
      setDetectorNote(name, id);
    };
    setDetectorFill(name);   // async: the installed plugins that support role and rate
  }
  if (t.follows) {   // the detection settings are not used for a follower
    for (const sel of [".pSens", ".pGap", ".pRefine", ".pFlam"]) { const el = div.querySelector(sel); if (el) el.disabled = true; }
    noteRow.classList.remove("hidden"); note.textContent = `follows ${t.follows}: the hits are derived from it, the detection settings are not used`;
    div.querySelector(".pactions .phint").textContent = "re-analyze derives the hits from the followed track again";
  }
  div.querySelector(".pGap").onchange = ev => p.min_gap_ms = +ev.target.value;
  div.querySelector(".pRefine").onchange = ev => p.refine = ev.target.checked;
  const flam = div.querySelector(".pFlam");   // P4-13 stage B: snare tracks only; the track param flam_split (absent = on)
  if (flam) { flam.title = "Applies on the next re-analyze"; flam.onchange = ev => p.flam_split = ev.target.checked; }
  div.querySelector(".pShift").onchange = ev => {   // P4-9: the service shifts the hits (track_shift_set); the page shows the result
    const nv = Math.max(-100000, Math.min(100000, Math.round(+ev.target.value||0)));
    ev.target.value = nv;
    if (followerLocked(name)) { ev.target.value = shiftNow(name); return; }   // P3-18
    shSerial(() => hgSerial(name, async () => {   // one after the other: the old value is the one the last call stored
      try {
        if (nv === shiftNow(name)) return;
        const ov = shiftNow(name);
        if (!await shiftApply(name, nv)) return;
        pushUndo({ type: "shift", track: name, from: ov, to: nv });   // one undo step, none for a refused shift
        setMsg(`shifted all hits of ${name} by ${nv-ov} samples`);
      } finally {
        const f = document.querySelector("#insParams .pShift");
        if (SH.pending <= 1 && f && INS.name === name) f.value = shiftNow(name);   // 1: this call itself
        insSumRefresh();
      }
    }));
  };
  // P4-3: the number fields and the handles of the loudness histogram set the same floor and ceiling
  div.querySelector(".pVLo").onchange = ev => hgVelCommit(name, p, +ev.target.value, p.vel_hi_db, hgCutOf(p));
  div.querySelector(".pVHi").onchange = ev => hgVelCommit(name, p, p.vel_lo_db, +ev.target.value, hgCutOf(p));
  div.querySelector(".pVCut").onchange = ev => {   // empty: no cut
    const txt = ev.target.value.trim();
    hgVelCommit(name, p, p.vel_lo_db, p.vel_hi_db, txt === "" ? null : Math.min(0, +txt));
  };
  const noteName = div.querySelector(".pNote"), noteNum = div.querySelector(".pNoteNum");
  noteName.onchange = () => { const n = noteFromName(noteName.value); if (n!==null) { t.note = n; noteNum.value = n; saveMetaNote(name); refreshHeadNote(name);} else noteName.value = nameFromNote(t.note); };
  noteNum.onchange = () => { const n = Math.max(0, Math.min(127, +noteNum.value)); t.note = n; noteName.value = nameFromNote(n); saveMetaNote(name); refreshHeadNote(name); };
  div.querySelector(".bReanalyze").onclick = () => startAnalysis([name]);
  insSteppers(div);   // P4-25
  hgAttach(div, name, p);   // P4-3
  lnBuildRow(div, name);   // P4-6a
  lyAttach(div, name);   // P4-7a
}
/* ---------------- P4-7a and P4-7b: the one-shot layer of a track ----------------
   The Inspector section Layer (crates/app/src/layer.rs): drop one WAV or AIFF one-shot on the zone (a window drop at the zone's
   position: shell.js passes the drop position) or Choose; level, polarity (normal, invert, auto: the majority of the alignment),
   on/off (playback), the velocity curve (dB with a range, linear, fixed). Align (P4-7b) places each hit's one-shot by its
   cross-correlation with the close mic in the playback and the render; Measure shows the polarity, the flips and the match.
   Render writes Exports/<name>_layer.wav. Each change is one track_layer_set (the service updates a playing transport).
   LY.align[name] keeps the last alignment: the info line shows the match of the selected hit. */
const LY = { align: {}, seq: 0 };
const LY_EXT = /\.(wav|wave|bwf|aif|aiff|aifc)$/i;
function lyLayer(name) { const t = S.tracks[name]; return t && t.layer ? t.layer : null; }
function lyHTML() {
  const info = txt => ` <span class="pinfo" title="${txt}">ⓘ</span>`;
  return `
      <div class="prow">
        <div class="pctl pctlWide lyCol">
          <div class="lyZone" title="Drop one WAV or AIFF one-shot here, or choose one. The app plays it at every hit of this track, with its onset on the hit, and renders it as Exports/&lt;track&gt;_layer.wav.">
            <span class="lyFile">no one-shot</span>
            <span class="lyBtns"><button type="button" class="btn mini lyChoose">Choose…</button><button type="button" class="btn mini lyRemove" title="Remove the layer of this track">Remove</button></span>
          </div>
          <span class="phint lyInfo"></span>
        </div>
      </div>
      <div class="prow lyRow">
        <div class="pctl pctlWide lyCol">
          <div class="lyViewHead">
            <span class="lyViewTitle">Onset</span>
            <span class="lyBtns"><button type="button" class="btn mini lyPlay" title="Hear the one-shot once from its first sample (the close mic sounds underneath, as with the audition key)">Play one-shot</button><button type="button" class="btn mini lyReset" title="Put the onset line back on the measured onset">Reset onset</button></span>
          </div>
          <span class="phint lyViewHint">Drag the line (the handle at its top) to set the moment the one-shot sounds on each hit.</span>
          <span class="phint lyViewInfo"></span>
          <div class="lyViewWrap" title="The first 200 ms of the one-shot. Drag the line to override the onset the layer places on every hit; drag the wave to pan, wheel to zoom. With a sampler calibration the bounce shows with its offset line.">
            <canvas class="lyView"></canvas>
          </div>
        </div>
      </div>
      <div class="prow lyRow">
        <label>Play ${info("Hear the one-shot at every hit while the session plays (also at half speed). Overlapping hits sound together; nothing is cut. The render does not depend on it.")}</label>
        <div class="pctl"><input type="checkbox" class="lyOn"></div>
      </div>
      <div class="prow lyRow">
        <label>Level ${info("The level of the one-shot in dB (-60 to +12), before the velocity curve.")}</label>
        <div class="pctl"><input type="number" class="lyLevel" min="-60" max="12" step="0.5" value="0"><span class="punit">dB</span></div>
      </div>
      <div class="prow lyRow">
        <label>Polarity ${info("Normal: the one-shot as it is. Inverted: upside down. Auto (experimental): the polarity that most hits match in the alignment with the close mic (Measure); on the real sessions it picked inverted for printed samples that were normal, so check it by ear.")}</label>
        <div class="pctl"><select class="lyPol"><option value="normal">normal</option><option value="invert">inverted</option><option value="auto">auto (experimental)</option></select></div>
      </div>
      <div class="prow lyRow">
        <label>Velocity curve ${info("The gain of each hit from its velocity. dB: velocity 127 is the level, lower velocities fall linearly in dB over the range. Linear: the amplitude follows the velocity. Fixed: every hit at the level.")}</label>
        <div class="pctl"><select class="lyCurve"><option value="db">dB</option><option value="linear">linear</option><option value="fixed">fixed</option></select><input type="number" class="lyRange" min="0" max="96" step="1" value="24"><span class="punit lyRangeUnit">dB range</span></div>
      </div>
      <div class="prow lyRow">
        <label>Align ${info("Place each hit's one-shot where its attack matches the close mic best (cross-correlation of the first 10 ms, within 3 ms, sub-sample in the render). The match (0 to 1) of each hit shows in the info line; a low match marks an uncertain hit.")}</label>
        <div class="pctl"><input type="checkbox" class="lyAlign"><button type="button" class="btn mini lyMeasure" title="Measure the alignment of every hit now">Measure</button></div>
      </div>
      <div class="prow lyRow"><label></label><div class="pctl"><span class="phint lyAlignInfo"></span></div></div>
      <div class="pactions lyRow"><button type="button" class="btn lyRender" title="Save the one-shot at every hit as a WAV file (the save dialog starts in the export folder): the rate, length and session position of the track">Render Layer</button></div>
      <div class="prow">
        <div class="pctl pctlWide lyCol">
          <div class="smZone" title="Sampler calibration (P4-7c): write the calibration MIDI, import it at the session start, bounce the sampler's kit piece for its note, then load the bounce here (or drop it on this zone). The MIDI exports then put each note at the aligned attack minus the sampler offset.">
            <span class="smFile">no sampler calibration</span>
            <span class="smBtns"><button type="button" class="btn mini smCal" title="Save the calibration MIDI: one note (this track's note) on a bar line; the save dialog starts in the export folder">Calibration MIDI…</button><button type="button" class="btn mini smLoad" title="Load the bounce of the calibration note (WAV or AIFF, from the session start)">Load bounce…</button></span>
          </div>
          <span class="phint smInfo"></span>
          <span class="smBtns smRow2"><button type="button" class="btn mini smShot" title="Use the loaded bounce as this track's one-shot (with its polarity)">Use as one-shot</button><button type="button" class="btn mini smClear" title="Remove the sampler calibration of this track">Clear</button></span>
        </div>
      </div>`;
}
function lyCurveOf(div) {
  const k = div.querySelector(".lyCurve").value;
  return k === "db" ? { kind: "db", range_db: Math.max(0, Math.min(96, +div.querySelector(".lyRange").value || 0)) } : { kind: k };
}
function lyShow(name, got) {   // the fields from the track's layer; `got` (an answer) adds the file state and the onset
  const div = $("#insParams"); if (!div || INS.name !== name) return;
  const l = lyLayer(name), zone = div.querySelector(".lyZone"); if (!zone) return;
  smShow(name);
  for (const r of div.querySelectorAll(".lyRow")) r.classList.toggle("hidden", !l);
  div.querySelector(".lyRemove").classList.toggle("hidden", !l);
  if (got) LY.got = { name, got };
  const g = got || (LY.got && LY.got.name === name ? LY.got.got : null);
  const file = l ? (g && g.file) || String((l.source && (l.source.path || l.source.relative)) || "").split("/").pop() : "";
  // P4-40e: another one-shot resets the view's window and its data
  if (LYV.file[name] !== file) { delete LYV.win[name]; delete LYV.cache[name]; LYV.file[name] = file; }
  div.querySelector(".lyFile").textContent = l ? file : "no one-shot";
  zone.classList.toggle("set", !!l);
  const st = g && g.status && g.status !== "ok" ? ` · file ${g.status}` : "";
  const eff = l ? (l.onset_set != null ? l.onset_set : l.onset) : 0;
  div.querySelector(".lyInfo").textContent = l ? `onset ${(1000 * eff / l.sr).toFixed(2)} ms, ` +
    (l.onset_set != null ? `set by hand (measured ${(1000 * l.onset / l.sr).toFixed(2)} ms)` : "measured") +
    ` · ${l.sr / 1000} kHz · ${l.channels === 2 ? "stereo" : "mono"}${st}` : "";
  if (!l) { div.querySelector(".lyAlignInfo").textContent = ""; lyViewDraw(name); return; }
  div.querySelector(".lyOn").checked = l.on !== false;
  div.querySelector(".lyLevel").value = String(l.level_db || 0);
  div.querySelector(".lyPol").value = l.polarity || "normal";
  const c = l.curve || { kind: "db", range_db: 24 };
  div.querySelector(".lyCurve").value = c.kind;
  div.querySelector(".lyRange").value = String(c.range_db === undefined ? 24 : c.range_db);
  for (const el of div.querySelectorAll(".lyRange, .lyRangeUnit")) el.classList.toggle("hidden", c.kind !== "db");
  div.querySelector(".lyAlign").checked = !!l.align;
  lyAlignText(name);
  lyViewDraw(name);   // P4-40e: the view follows (the answer's onset, an override, a Reset)
}
function lyAlignText(name) {
  const div = $("#insParams"), a = LY.align[name], el = div && div.querySelector(".lyAlignInfo"); if (!el) return;
  const l = lyLayer(name), auto = !!l && l.polarity === "auto";
  if (!a) { el.textContent = auto ? "auto is experimental: press Measure to see the polarity it picks" : ""; el.classList.remove("lyWarn"); return; }
  let t = `polarity ${a.polarity} (${a.polarity === "inverted" ? a.inverted : a.as_is} of ${a.n}) · ${a.flips.length} flip${a.flips.length === 1 ? "" : "s"} · match ${a.median_match === null ? "-" : a.median_match.toFixed(2)} · ${a.uncertain} uncertain`;
  const warn = auto && a.polarity === "inverted";
  if (auto) t += " · auto is experimental";
  if (warn) t += " · WARNING: auto picked inverted; printed samples were normal in the tests, so check by ear or use normal";
  el.textContent = t;
  el.classList.toggle("lyWarn", warn);
}
async function lySet(name, o) {   // one track_layer_set; the answer's layer goes into the track
  if (!S.sid) return null;
  const seq = ++LY.seq;
  try {
    const r = await B.layerSet(S.sid, name, o);
    const t = S.tracks[name];
    if (t) { if (r.layer) { t.layer = r.layer; t.layer_status = r.status; } else { delete t.layer; delete t.layer_status; } refreshRelinkBar(); }
    if ("path" in o || "polarity" in o || "onset" in o) delete LY.align[name];
    if (seq === LY.seq) lyShow(name, r);
    insSumRefresh(); drawInfo();
    if (r.transport_error) setMsg(`layer of ${name}: saved, but the playing transport did not take it: ${r.transport_error}`);
    return r;
  } catch (e) { setMsg(`layer of ${name}: ${e.message}`); lyShow(name); return null; }
}
async function lySetFile(name, path) {
  const r = await lySet(name, { path });
  if (r && r.layer) setMsg(`${name}: layer ${r.file}, onset ${r.onset_ms} ms`);
  return r;
}
/* P4-7d: the Layer section's dialogs. A load dialog starts in the folder the last load of its kind used (app settings,
   so kept per app, not per project), else the project folder; a save dialog starts in the project's export folder
   (the Export dialog's, else Exports/). Cancel does nothing. */
async function dialogFolder(key) {
  try { const st = await B.appSettingsGet(); if (st && typeof st[key] === "string" && st[key]) return st[key]; } catch (e) { /* the project folder */ }
  try { const s = (await B.sessions()).find(x => x.sid === S.sid); return (s && s.folder) || null; } catch (e) { return null; }
}
async function rememberDialogFolder(key, path) {   // the folder of a file the user chose, kept for the next load of its kind
  const i = path.lastIndexOf("/");
  if (i > 0) { try { await B.appSettingsSet(key, path.slice(0, i)); } catch (e) { /* the dialog starts where it did */ } }
}
async function exportFolder() {   // the folder a save starts in: the one kept by the Export dialog, else Exports/ of the project
  try { return (await B.exportExisting(S.sid, {})).dest || null; } catch (e) { return null; }
}
const joinFile = (folder, file) => folder ? `${folder}/${file}` : file;
async function lyChoose(name) {
  const p = await B.pickOneShot(await dialogFolder("layer_one_shot_dir"));
  if (!p) return;
  await rememberDialogFolder("layer_one_shot_dir", p);
  await lySetFile(name, p);
}
/* ---------------- P4-40e: the onset view of the Layer section ----------------
   The first 200 ms of the one-shot (crates/app/src/layer.rs track_layer_wave) with a line at the onset (the lanes'
   tToX on the view's own window). Drag the line: one track_layer_set {onset} on release overrides the measured onset
   for the render, the playback and the Align; Reset sends onset null. Drag the wave pans, the wheel zooms. With a
   sampler calibration (P4-7c) the bounce (track_sampler_wave) shows with a line at its offset. Play runs the
   transport audition with `shot`: the one-shot once at the window start. */
const LYV = { win: {}, file: {}, cache: {}, err: {}, drag: null, pan: null, seq: 0, draws: 0 };
function lyView(name) { const div = $("#insParams"); return div && INS.name === name ? div.querySelector(".lyView") : null; }
function lyViewWin(name) {   // the visible window of the track's one-shot, in its own seconds
  const l = lyLayer(name), dur = l ? l.frames / l.sr : 0.2, key = l ? l.source.path : "";
  let w = LYV.win[name];
  if (!w || w.key !== key) {   // a new one-shot opens at its onset, so a sound after a long lead-in shows
    const t0 = l ? Math.max(0, Math.min(lyOnsetS(name) - 0.01, dur - 0.2)) : 0;
    w = LYV.win[name] = { key, t0, t1: Math.min(dur, t0 + 0.2) };
  }
  const span = Math.max(0.002, Math.min(dur, w.t1 - w.t0));
  w.t0 = Math.max(0, Math.min(w.t0, Math.max(0, dur - span)));
  w.t1 = Math.min(dur, w.t0 + span);
  return w;
}
function lyOnsetS(name) {   // the effective onset in seconds (the drag preview wins)
  const l = lyLayer(name); if (!l) return null;
  if (LYV.drag && LYV.drag.name === name) return LYV.drag.s / l.sr;
  return (l.onset_set != null ? l.onset_set : l.onset) / l.sr;
}
function lyViewX(name, t, W) { const w = lyViewWin(name); return (t - w.t0) / (w.t1 - w.t0) * W; }
function lyViewInfo(name) {
  const div = $("#insParams"); if (!div || INS.name !== name) return;
  const l = lyLayer(name), el = div.querySelector(".lyViewInfo"); if (!el) return;
  if (!l) { el.textContent = ""; return; }
  if (LYV.err[name]) { el.textContent = `the wave could not be read: ${LYV.err[name]}`; return; }
  const w = lyViewWin(name), on = lyOnsetS(name);
  const from = w.t0 > 0.0005 ? `from ${w.t0.toFixed(2)} s` : "first";
  const off = on < w.t0 || on > w.t1 ? ", the line is off the view" : "";
  el.textContent = `${from} ${(w.t1 - w.t0).toFixed(2)} s of ${(l.frames / l.sr).toFixed(2)} s${off}`;
}
function lyViewCache(name) {   // the cached wave of this one-shot (a cache of another file is not drawn)
  const c = LYV.cache[name], l = lyLayer(name);
  return c && l && c.key === l.source.path ? c : null;
}
async function lyViewFetch(name, s0, s1) {   // the visible window of the one-shot (and of the bounce)
  const l = lyLayer(name);
  if (!S.sid || !l) return;
  const seq = ++LYV.seq;
  const c = { key: l.source.path };
  try {
    c.shot = await B.layerWave(S.sid, name, s0, s1);
    if (seq !== LYV.seq) return;
    const s = S.tracks[name] && S.tracks[name].sampler;
    if (s && s.file) { try { c.bounce = await B.samplerWave(S.sid, name, s0, s1); } catch (e) { /* no bounce this time */ } }
    if (seq !== LYV.seq) return;
    LYV.cache[name] = c;
    delete LYV.err[name];
  } catch (e) {
    if (seq !== LYV.seq) return;   // the message shows in the info line, not as a black view
    LYV.err[name] = String(e && e.message !== undefined ? e.message : e);
    lyViewInfo(name);
    return;
  }
  if (INS.name === name) lyViewDraw(name);
}
function lyViewColumns(d, off, sr, w, W) {   // per pixel column the min and max of the samples it covers
  const min = new Float32Array(W + 1), max = new Float32Array(W + 1), seen = new Uint8Array(W + 1);
  for (let i = 0; i < d.length; i++) {
    const t = (off + i) / sr;
    if (t < w.t0 || t > w.t1) continue;
    const x = Math.round((t - w.t0) / (w.t1 - w.t0) * W);
    if (d[i] < min[x] || !seen[x]) min[x] = d[i];
    if (d[i] > max[x] || !seen[x]) max[x] = d[i];
    seen[x] = 1;
  }
  return { min, max, seen };
}
function lyCanvas(c) {   // size the backing store to the element; returns [ctx, W, H] in CSS px
  const W = c.clientWidth, H = c.clientHeight, dpr = window.devicePixelRatio || 1;
  if (c.width !== Math.round(W * dpr) || c.height !== Math.round(H * dpr)) { c.width = Math.round(W * dpr); c.height = Math.round(H * dpr); }
  const ctx = c.getContext("2d"); ctx.setTransform(dpr, 0, 0, dpr, 0, 0); ctx.clearRect(0, 0, W, H);
  LYV.draws++;   // the tests read it: nothing draws without a change
  return [ctx, W, H];
}
function lyViewDraw(name) {
  const c = lyView(name); if (!c) return;
  const [ctx, W, H] = lyCanvas(c);
  const l = lyLayer(name); if (!l) return;
  const w = lyViewWin(name), sr = l.sr, cache = lyViewCache(name);
  const acc = LN_CSS.getPropertyValue("--accent").trim() || "#4da3ff";
  const mid = H / 2, amp = H / 2 - 4;
  const trace = (buf, color) => {
    if (!buf) return;
    const { min, max, seen } = lyViewColumns(buf.data, buf.start, buf.sr, w, W);
    ctx.strokeStyle = color; ctx.lineWidth = 1; ctx.beginPath();
    for (let x = 0; x <= W; x++) {
      if (!seen[x]) continue;
      ctx.moveTo(x + 0.5, mid - max[x] * amp);
      ctx.lineTo(x + 0.5, mid - min[x] * amp);
    }
    ctx.stroke();
  };
  trace(cache && cache.shot, "#88909c");
  trace(cache && cache.bounce, "rgba(136, 144, 156, 0.55)");
  // the lines: the onset (drag it), the measured one when overridden, the bounce's sampler offset
  const line = (t, color, width) => {
    const x = Math.round(lyViewX(name, t, W)) + 0.5;
    ctx.strokeStyle = color; ctx.lineWidth = width; ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, H); ctx.stroke();
  };
  line(lyOnsetS(name), acc, 1.5);
  const hx = Math.round(lyViewX(name, lyOnsetS(name), W));   // the drag handle: a tab on the top of the line
  ctx.fillStyle = acc; ctx.fillRect(hx - 4, 0, 9, 7);
  if (l.onset_set != null && !(LYV.drag && LYV.drag.name === name)) line(l.onset / l.sr, "#6e7681", 1);
  const s = S.tracks[name] && S.tracks[name].sampler;
  if (s) line(s.offset_samples / s.sr, "#ffb84d", 1);
  lyViewInfo(name);
  // the data for the window (a little wider, so a small pan does not refetch)
  const s0 = Math.floor(w.t0 * sr), s1 = Math.ceil(w.t1 * sr) + 1;
  const got = cache && cache.shot && cache.shot.sr === sr && cache.shot.start <= s0 &&
    cache.shot.start + cache.shot.data.length >= Math.min(s1, cache.shot.start + cache.shot.data.length);
  if (!got) lyViewFetch(name, Math.max(0, s0 - 64), s1 + 64);
}
function lyViewAttach(div, name) {
  const c = div.querySelector(".lyView");
  div.querySelector(".lyPlay").onclick = () => audition(false, null, name);
  div.querySelector(".lyReset").onclick = () => lySet(name, { onset: null });
  c.onpointerdown = ev => {
    const l = lyLayer(name); if (!l) return;
    const r = c.getBoundingClientRect(), x = ev.clientX - r.left;
    c.setPointerCapture(ev.pointerId);
    if (Math.abs(x - lyViewX(name, lyOnsetS(name), r.width)) <= 5) {
      LYV.drag = { name, s: (l.onset_set != null ? l.onset_set : l.onset) };
    } else {
      const w = lyViewWin(name);
      LYV.pan = { name, x0: x, t0: w.t0, t1: w.t1 };
    }
    lyViewDraw(name);
  };
  c.onpointermove = ev => {
    const l = lyLayer(name); if (!l) return;
    const r = c.getBoundingClientRect(), x = Math.max(0, Math.min(r.width, ev.clientX - r.left));
    const w = lyViewWin(name), dur = l.frames / l.sr;
    if (LYV.drag && LYV.drag.name === name) {
      LYV.drag.s = Math.max(0, Math.min(l.frames - 1, (w.t0 + x / r.width * (w.t1 - w.t0)) * l.sr));
      lyViewDraw(name);
    } else if (LYV.pan && LYV.pan.name === name) {
      const dt = (LYV.pan.x0 - x) / r.width * (LYV.pan.t1 - LYV.pan.t0), span = LYV.pan.t1 - LYV.pan.t0;
      const t0 = Math.max(0, Math.min(LYV.pan.t0 + dt, dur - span));
      Object.assign(w, { t0, t1: Math.min(dur, t0 + span) });
      lyViewDraw(name);
    } else {   // the cursor says what a press does: the line moves, the wave pans
      c.style.cursor = Math.abs(x - lyViewX(name, lyOnsetS(name), r.width)) <= 5 ? "ew-resize" : "grab";
    }
  };
  const up = () => {
    if (LYV.drag && LYV.drag.name === name) {
      const s = Math.round(LYV.drag.s);
      LYV.drag = null;
      lySet(name, { onset: s });   // the answer redraws the view and the info
    }
    LYV.pan = null;
  };
  c.onpointerup = up;
  c.onpointercancel = up;
  c.onwheel = ev => {
    const l = lyLayer(name); if (!l) return;
    ev.preventDefault();
    const r = c.getBoundingClientRect(), x = Math.max(0, Math.min(r.width, ev.clientX - r.left));
    const w = lyViewWin(name), dur = l.frames / l.sr, span0 = w.t1 - w.t0;
    const at = w.t0 + x / r.width * span0;
    const span = Math.max(0.002, Math.min(dur, span0 * (ev.deltaY > 0 ? 1.25 : 0.8)));
    const t0 = Math.max(0, Math.min(at - (at - w.t0) * (span / span0), dur - span));
    Object.assign(w, { t0, t1: Math.min(dur, t0 + span) });
    lyViewDraw(name);
  };
}
/* A window drop (DSShell.hooks.otherPaths): one audio file dropped on the Layer zone of the Inspector sets the layer of its
   track; anything else goes on to Add Tracks. */
function lyDrop(paths, pos) {
  if (!INS.name || !pos) return false;
  const hit = z => { if (!z || !z.offsetParent) return false; const b = z.getBoundingClientRect(); return pos.x >= b.left && pos.x <= b.right && pos.y >= b.top && pos.y <= b.bottom; };
  const sm = hit(document.querySelector("#insParams .smZone")), ly = !sm && hit(document.querySelector("#insParams .lyZone"));
  if (!sm && !ly) return false;
  const audio = (paths || []).filter(p => LY_EXT.test(p));
  if (audio.length !== 1 || paths.length !== 1) { setMsg(sm ? "drop one WAV or AIFF bounce on the sampler calibration" : "drop one WAV or AIFF one-shot on the Layer section"); return true; }
  if (sm) smLoad(INS.name, audio[0], false); else lySetFile(INS.name, audio[0]);
  return true;
}
async function lyMeasure(name) {
  if (!S.sid || !lyLayer(name)) return null;
  try {
    const a = await B.layerAlign(S.sid, name);
    LY.align[name] = a; lyAlignText(name); drawInfo();
    return a;
  } catch (e) { setMsg(`alignment of ${name}: ${e.message}`); return null; }
}
async function lyRender(name) {
  if (!S.sid) return null;
  const path = await B.pickSaveFile("Render the layer", joinFile(await exportFolder(), `${name} layer.wav`), "wav", "WAV audio");
  if (!path) { setMsg(`${name}: render cancelled`); return null; }
  try {
    const r = await B.layerExport(S.sid, name, path);
    setMsg(`${name}: layer rendered to ${r.path}${r.outside ? ` (${r.outside} hits outside the file)` : ""}`);
    return r;
  } catch (e) { setMsg(`render of ${name}: ${e.message}`); return null; }
}
function lyMatchOf(track, t) {   // the match of the hit at t in the last alignment of the track (null: none)
  const a = LY.align[track]; if (!a || !a.hits) return null;
  let lo = 0, hi = a.hits.length;
  while (lo < hi) { const m = (lo + hi) >> 1; if (a.hits[m].t < t - 1e-7) lo = m + 1; else hi = m; }
  const h = a.hits[lo];
  return h && Math.abs(h.t - t) <= 1e-7 ? h : null;
}
/* P4-7c: the sampler calibration of the track (crates/app/src/sampler.rs). Calibration MIDI… writes one note on a bar line into
   the export folder; Load bounce… (or a drop on the zone) measures the delay of the sampler's transient and its polarity
   against the close mic and keeps them in the track (`sampler`); the MIDI exports then put each note at the aligned attack
   minus that offset (the Export dialog's "Compensate sampler offset"). Use as one-shot sets the bounce as the layer. */
const SM = { last: {}, seq: 0 };
const smOf = name => { const t = S.tracks[name]; return t && t.sampler ? t.sampler : null; };
const smSigned = ms => `${ms < 0 ? "-" : "+"}${Math.abs(ms).toFixed(2)}`;
function smText(s) {   // "sampler offset +5.51 ms, polarity inverted"
  return `sampler offset ${smSigned(1000 * s.offset_samples / s.sr)} ms, polarity ${s.polarity || "unknown"}`;
}
function smShow(name) {
  const div = $("#insParams"); if (!div || INS.name !== name || !div.querySelector(".smZone")) return;
  const s = smOf(name), zone = div.querySelector(".smZone");
  zone.classList.toggle("set", !!s);
  div.querySelector(".smFile").textContent = s ? String(s.file || "").split("/").pop() || "bounce" : "no sampler calibration";
  const cal = SM.last[name];
  div.querySelector(".smInfo").textContent = s ? smText(s) + (s.polarity === "inverted" ? " (the bounce is upside down against the close mic)" : "") :
    cal ? cal.text : "Calibration MIDI…: import it at the session start, bounce the sampler's kit piece for that note, then load the bounce here.";
  div.querySelector(".smRow2").classList.toggle("hidden", !s);
}
async function smCalibrate(name) {
  if (!S.sid) return null;
  const path = await B.pickSaveFile("Write the calibration MIDI", joinFile(await exportFolder(), `${name} calibration.mid`), "mid", "MIDI file");
  if (!path) { setMsg(`${name}: calibration MIDI cancelled`); return null; }
  try {
    const r = await B.samplerCalibrationMidi(S.sid, name, null, path);
    SM.last[name] = r; smShow(name);
    setMsg(`${name}: calibration MIDI written to ${r.path}. ${r.text}`);
    return r;
  } catch (e) { setMsg(`calibration MIDI of ${name}: ${errText(e)}`); return null; }
}
function smAdopt(name, r) {   // an answer of track_sampler_set or track_sampler_get goes into the track
  const t = S.tracks[name]; if (!t) return;
  if (r.sampler) t.sampler = r.sampler; else delete t.sampler;
  if (r.layer && r.layer.layer) { t.layer = r.layer.layer; t.layer_status = r.layer.status; delete LY.align[name]; refreshRelinkBar(); }
}
async function smLoad(name, path, oneShot) {
  if (!S.sid) return null;
  const seq = ++SM.seq;
  try {
    const r = await B.samplerSet(S.sid, name, { path, one_shot: !!oneShot });
    smAdopt(name, r);
    if (seq === SM.seq) { delete SM.last[name]; smShow(name); if (r.layer) lyShow(name, r.layer); }
    insSumRefresh(); setDirtyState(true);
    setMsg(`${name}: ${r.sampler ? smText(r.sampler) : "sampler calibration removed"}${r.layer ? "; the bounce is the layer one-shot" : ""}`);
    return r;
  } catch (e) { setMsg(`sampler bounce of ${name}: ${errText(e)}`); smShow(name); return null; }
}
async function smChoose(name) {
  const p = await B.pickBounce(await dialogFolder("sampler_bounce_dir"));
  if (!p) return;
  await rememberDialogFolder("sampler_bounce_dir", p);
  await smLoad(name, p, false);
}
/* The page asks the service for the layer and the sampler of the inspected track while the Layer section is on screen, so
   a change that came through the agent commands (track_layer_set, track_sampler_set) shows without a reload (the P4-9 way:
   the service holds the truth and the page shows what is stored). */
async function smRefreshAll() {   // the sampler of every track from the service
  if (!S.sid) return;
  const sid = S.sid;
  await Promise.all(S.order.map(async n => {
    try { const r = await B.samplerGet(sid, n); if (sid === S.sid && S.tracks[n]) { if (r.sampler) S.tracks[n].sampler = r.sampler; else delete S.tracks[n].sampler; } } catch (e) { /* the page keeps what it has */ }
  }));
}
const LYR = { busy: false, again: new Set() };
async function lyRefresh(name) {
  if (!S.sid || !S.tracks[name]) return false;
  if (LYR.busy) { LYR.again.add(name); return false; }   // an event while one refresh runs: ask again after it
  LYR.busy = true;
  const sid = S.sid, seq = LY.seq, sseq = SM.seq;
  try {
    const [l, s] = await Promise.all([B.layerGet(sid, name), B.samplerGet(sid, name)]);
    if (sid !== S.sid || !S.tracks[name] || seq !== LY.seq || sseq !== SM.seq) return false;
    const t = S.tracks[name];
    const same = JSON.stringify([t.layer || null, t.layer_status || null, t.sampler || null]) === JSON.stringify([l.layer || null, l.layer ? l.status : null, s.sampler || null]);
    if (!same) {
      if (l.layer) { t.layer = l.layer; t.layer_status = l.status; } else { delete t.layer; delete t.layer_status; }
      if (s.sampler) t.sampler = s.sampler; else delete t.sampler;
      if (JSON.stringify(t.layer || null) !== JSON.stringify(l.layer || null)) delete LY.align[name];
      refreshRelinkBar(); insSumRefresh();
    }
    if (INS.name === name) lyShow(name, l.layer ? l : undefined);
    return !same;
  } catch (e) { return false; } finally { LYR.busy = false; for (const n of [...LYR.again]) { LYR.again.delete(n); lyRefresh(n); } }
}
/* P4-33: the service sends track-layer {sid, name} after every layer change, by the page or by an agent command (CLI, MCP).
   The page asks for the stored layer of that track and shows it: Inspector, summary, relink bar. No poll. */
if (B.onTrackLayer) B.onTrackLayer(p => { if (p && p.sid === S.sid && S.tracks[p.name]) lyRefresh(p.name); });
function lyAttach(div, name) {
  div.querySelector(".smCal").onclick = () => smCalibrate(name);
  div.querySelector(".smLoad").onclick = () => smChoose(name);
  div.querySelector(".smShot").onclick = () => { const s = smOf(name); if (s && s.file) smLoad(name, s.file, true); };
  div.querySelector(".smClear").onclick = () => smLoad(name, "", false);
  div.querySelector(".lyChoose").onclick = () => lyChoose(name);
  div.querySelector(".lyRemove").onclick = () => lySet(name, { path: "" });
  div.querySelector(".lyOn").onchange = ev => lySet(name, { on: ev.target.checked });
  div.querySelector(".lyLevel").onchange = ev => { const v = Math.max(-60, Math.min(12, +ev.target.value || 0)); ev.target.value = String(v); lySet(name, { level_db: v }); };
  div.querySelector(".lyPol").onchange = ev => lySet(name, { polarity: ev.target.value });
  div.querySelector(".lyCurve").onchange = () => lySet(name, { curve: lyCurveOf(div) });
  div.querySelector(".lyRange").onchange = () => lySet(name, { curve: lyCurveOf(div) });
  div.querySelector(".lyAlign").onchange = async ev => { const r = await lySet(name, { align: ev.target.checked }); if (r && ev.target.checked) lyMeasure(name); };
  div.querySelector(".lyMeasure").onclick = () => lyMeasure(name);
  div.querySelector(".lyRender").onclick = () => lyRender(name);
  lyViewAttach(div, name);   // P4-40e: the onset view (drag, pan, zoom, Play, Reset)
  lyShow(name);
  if (S.sid) lyRefresh(name);   // the stored layer and sampler (an agent command may have changed them since the project loaded)
}
/* ---------------- P4-6a: range selection and Learn marks (owner request 2026-10-12, docs/migration/concept-learn-mode.md) ----------------
   A drag in the wave lane selects a time range on that track (LN.sel; one at a time, a click, K or Escape clears it). K
   (Hits > Learn This) and Shift+K (Forget This) turn the selection, or the selected hit, into a mark: the service resolves the
   examples (track_learn_mark: the active hits, else the strongest candidate, else the strongest rise of 6 dB) and keeps them in
   the track's `learn` object. The page draws the selection band in both lanes, a 3 px bar per mark at the top of the spike
   lane (green: this, red: not) with a dot or a cross on each example hit, and the Inspector has a Learn row. Every step
   (mark, remove, flip, forget) is one undo step {type: "learn", track, before, after}: the whole learn object goes back with
   track_learn_set. A follower marks its source (the service routes). There is no filter yet (P4-6b): a "not" hit is drawn grey. */
const LN_CSS = getComputedStyle(document.documentElement);   // the theme tokens, read once (never in a frame)
const LN_NOT_TINT = "#b8706c";   // P4-6c: a "not" example that is still in the output (not the filtered grey)
const LN_THIS = LN_CSS.getPropertyValue("--learn-this").trim() || "#4cc38a", LN_NOT = LN_CSS.getPropertyValue("--learn-not").trim() || "#e5534b", LN_SAME = 0.002, LN_BAR = 3, LN_PULSE_MS = 450;
const LN = { sel: null, mark: null, pending: 0, busy: 0, chain: Promise.resolve(), cache: {}, sig: "" };
function lnReset() { LN.sel = null; LN.mark = null; LN.sig = ""; }
function lnLearn(name) { const t = S.tracks[name]; return t && t.learn && Array.isArray(t.learn.marks) && t.learn.marks.length ? t.learn : null; }
function lnTarget(name) { const t = S.tracks[name]; return t && t.follows && S.tracks[t.follows] ? t.follows : name; }   // a follower marks its source
function lnCounts(learn) {
  const c = { this: 0, not: 0 };
  for (const m of (learn && learn.marks) || []) c[m.kind === "not" ? "not" : "this"] += (m.at || []).length;
  return c;
}
function lnTimes(name) {   // the example times of a track as the lanes draw them (the Hitpoint shift is back on), sorted, cached per learn object
  const learn = lnLearn(name), t = S.tracks[name];
  const shift = t && t.sr && t.params ? (t.params.shift_samples || 0) / t.sr : 0;
  const c = LN.cache[name];
  if (c && c.learn === learn && c.shift === shift) return c;
  const out = { learn, shift, this: [], not: [] };
  for (const m of (learn && learn.marks) || []) for (const a of m.at || []) out[m.kind === "not" ? "not" : "this"].push(a + shift);
  out.this.sort((a, b) => a - b); out.not.sort((a, b) => a - b);
  return (LN.cache[name] = out);
}
function lnNear(sorted, t) {   // an example within LN_SAME of t (binary search)
  let lo = 0, hi = sorted.length;
  while (lo < hi) { const mid = (lo + hi) >> 1; if (sorted[mid] < t - LN_SAME) lo = mid + 1; else hi = mid; }
  return lo < sorted.length && sorted[lo] <= t + LN_SAME;
}
function lnKindOf(name, t) { const c = lnTimes(name); return c.not.length && lnNear(c.not, t) ? "not" : c.this.length && lnNear(c.this, t) ? "this" : null; }
function lnHitsIn(name, t0, t1) { return (S.events[name] || []).filter(e => e.t >= t0 && e.t <= t1 && !hgIsCut(e)).length; }
function lnRedraw(name) { if (name) invalidateHits(name); }
function lnSetSel(track, t0, t1) {
  const was = LN.sel ? LN.sel.track : null;
  LN.sel = track === null ? null : { track, t0: Math.min(t0, t1), t1: Math.max(t0, t1) };
  if (LN.sel) LN.mark = null;
  lnRedraw(was); lnRedraw(track); drawRuler(); drawInfo(); lnInspector();
  selShellSync();   // #158 (rank 9): a range selection enables Delete and the nudge items
}
function lnClearSel() { if (LN.sel) lnSetSel(null); }
function lnSetMark(track, id) {
  const was = LN.mark ? LN.mark.track : null;
  LN.mark = track ? { track, id } : null;
  LN.pulse = track ? { until: performance.now() + LN_PULSE_MS } : null;   // the ticks of the selected mark pulse once
  if (track) setTimeout(() => { if (LN.pulse && performance.now() >= LN.pulse.until) { LN.pulse = null; lnRedraw(track); } }, LN_PULSE_MS + 20);
  lnRedraw(was); lnRedraw(track); lnInspector();
}
function lnSelText() {
  const s = LN.sel; if (!s) return "";
  const n = lnHitsIn(s.track, s.t0, s.t1);
  return `selection ${fmt(s.t0, 3)} to ${fmt(s.t1, 3)} s (${fmt(s.t1 - s.t0, 3)} s), ${n} ${n === 1 ? "hit" : "hits"}`;
}
/* the drawing: the band (both lanes, under the hit lines) and the marks (spike lane, over them) */
function lnDrawBand(ctx, W, H, name) {
  const s = LN.sel; if (!s || s.track !== name) return;
  const x0 = Math.max(0, tToX(s.t0, W)), x1 = Math.min(W, tToX(s.t1, W));
  if (x1 < 0 || x0 > W) return;
  ctx.globalAlpha = 0.2; ctx.fillStyle = "#4da3ff"; ctx.fillRect(x0, 0, Math.max(1, x1 - x0), H);
  ctx.globalAlpha = 0.75; ctx.fillRect(Math.round(x0), 0, 1, H); ctx.fillRect(Math.round(x1) - (x1 - x0 < 1 ? 0 : 1), 0, 1, H);
  ctx.globalAlpha = 1;
}
function lnBarSpan(m, W) {   // the x range of a mark bar: at least 6 px (one hit)
  const a = tToX(m.t0, W), b = tToX(m.t1, W), mid = (a + b) / 2, w = Math.max(6, b - a);
  return [mid - w / 2, mid + w / 2];
}
function lnDrawMarks(ctx, W, H, name) {
  const learn = lnLearn(name); if (!learn) return;
  const dim = learn.on === false, shift = lnTimes(name).shift;
  for (const m of learn.marks) {
    const [x0, x1] = lnBarSpan(m, W);
    if (x1 < -6 || x0 > W + 6) continue;
    const sel = LN.mark && LN.mark.track === name && LN.mark.id === m.id, col = m.kind === "not" ? LN_NOT : LN_THIS;
    const pulse = sel && LN.pulse && performance.now() < LN.pulse.until, r = pulse ? 3.5 : 2;   // the ticks of a selected mark pulse once
    ctx.globalAlpha = dim ? 0.4 : 1; ctx.fillStyle = col;
    ctx.fillRect(x0, 0, x1 - x0, sel ? LN_BAR + 1 : LN_BAR);   // the bar and the ticks stay in the top 4 px (concept 2.4)
    if (sel) { ctx.strokeStyle = "#fff"; ctx.lineWidth = 1; ctx.strokeRect(x0 + 0.5, 0.5, x1 - x0 - 1, LN_BAR); }
    for (const a of m.at || []) {
      const x = Math.round(tToX(a + shift, W)) + 0.5;
      if (x < -4 || x > W + 4) continue;
      if (m.kind === "not") { ctx.strokeStyle = col; ctx.lineWidth = 1.5; ctx.beginPath(); ctx.moveTo(x - r, 2 - r); ctx.lineTo(x + r, 2 + r); ctx.moveTo(x + r, 2 - r); ctx.lineTo(x - r, 2 + r); ctx.stroke(); }
      else { ctx.beginPath(); ctx.arc(x, 2, r, 0, 6.2832); ctx.fill(); }
    }
  }
  ctx.globalAlpha = 1; ctx.lineWidth = 1;
}
function lnMarkAt(name, px, py) {   // the mark bar under a point of the spike lane (its top 6 px), the last one drawn
  const learn = lnLearn(name); if (!learn || py > 6) return null;
  const W = laneWidth(); let hit = null;
  for (const m of learn.marks) { const [x0, x1] = lnBarSpan(m, W); if (px >= x0 - 1 && px <= x1 + 1) hit = m; }
  return hit;
}
/* the gesture: a drag in the wave lane (more than 3 px) selects; a click is the playhead, as before, and clears the selection */
function lnClampT(name, t) { const d = (S.tracks[name] && S.tracks[name].duration) || maxDur(); return Math.max(0, Math.min(t, d)); }
function lnRangeGesture(ev, name) {
  S.followHold = true;
  const W = laneWidth(), startCX = ev.clientX, startX = laneXOf(ev), t0 = lnClampT(name, xToT(startX, W));
  let moved = false;
  const mv = ev2 => {
    if (!moved && Math.abs(ev2.clientX - startCX) <= 3) return;
    moved = true;
    lnSetSel(name, t0, lnClampT(name, xToT(laneXOf(ev2), W)));
  };
  const up = () => {
    window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up);
    if (moved) {
      if (S.selTrack !== name) setSelTrack(name);
      setMsg(`${name}: ${lnSelText()} (K: Learn This, Shift+K: Forget This)`);
      return;
    }
    S.playhead = snapPlayhead(xToT(startX, W));
    lnClearSel(); invalidate(F.HEAD); updateStatus();
  };
  window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
}
/* the commands: one function per menu item; the key and the buttons call them too */
function lnQueue(fn) {   // the calls run one after the other; an undo waits while one is on its way
  LN.pending++;
  // P4-6c: the hit edits on their way first: a Learn change with the filter on re-picks in the service, keeping them
  const p = LN.chain.then(async () => { await flushEvents(); LN.busy++; try { return await fn(); } finally { LN.busy--; } }).catch(e => setMsg(`learn failed: ${errText(e)}`, "error")).finally(() => { LN.pending--; lnInspector(); });
  LN.chain = p;
  return p;
}
function lnUndoWaits() { return LN.pending > 0; }
function lnSnap(track) { return (S.events[track] || []).map(({ id, ...e }) => ({ ...e })); }   // P4-6c: the hits as the service stores them
function lnApply(track, learn, r) {   // the new learn object of a track from the service; returns the one before.
  // P4-6c: `r` is the command's answer; with the filter on (or just turned off) the service marked the track's hits
  // again (r.filter.events: the same hits, the grey ones "filtered": true; hand deletions and velocities stay). The
  // hits before and after go into LN.snap: the undo step restores them exactly (track_learn_set with events)
  const t = S.tracks[track], before = t.learn || null;
  if (learn) t.learn = learn; else delete t.learn;   // an object without marks stays (Forget keeps the settings)
  LN.snap = null;
  if (r && r.filter) { const evBefore = lnSnap(track); lnAdopt(track, r.filter); LN.snap = { evBefore, evAfter: lnSnap(track) }; }
  lnScoreDrop(track);
  lnRedraw(track); lnInspector(); drawInfo(); setDirtyState(true);
  if (window.DSShell && DSShell.sync) DSShell.sync();   // Hits > Learn Filter shows the filter of the selected track
  return before;
}
function lnPush(r, before, label, t) {   // P4-6c: one undo step of a Learn change: the learn object and, when the hits changed, their snapshot; P4-40a: its label and place
  pushUndo({ type: "learn", track: r.track, before, after: r.learn || null, ...(LN.snap || {}),
    ...(label ? { label, where: { track: r.track, t: Number.isFinite(t) ? t : null } } : {}) });
}
function lnAdopt(track, f) {   // P4-6c: the service's hits after a Learn change; the page ids and the selected hit stay
  const old = S.events[track] || [], selT = S.sel && S.sel.track === track && old[S.sel.index] ? old[S.sel.index].t : null;
  S.events[track] = adoptHits(f.events, old, [], []);
  EDGEN[track] = (EDGEN[track] || 0) + 1;   // the list was replaced (an edit plan made on the old one is void)
  if (selT !== null) {
    const i = S.events[track].findIndex(e => Math.abs(e.t - selT) < 1e-6);
    S.sel = i >= 0 ? { track, index: i } : null;
  }
  const e = LN_SC[lnTarget(track)]; if (e) e.drag = false;   // a threshold drag's preview ends with the stored hits
  invalidateHits(track); refreshCounts(track); updateStatus();
  const fol = Array.isArray(f.followers) ? f.followers : [];
  if (fol.length) reloadFollowers(fol);
  if (INS.name === track) hgLoad();
}
const LN_CANDIDATE_REACH = 0.030;   // the same reach as the service's sweep (crates/app/src/learn.rs CANDIDATE_REACH)
function lnSweepLocal(track, learn) {   // P4-38b: the learned+added hits the service's sweep dropped leave the page at once (not stale until the poll)
  const at = (((learn || {}).marks || []).filter(m => m.kind === "this")).flatMap(m => m.at || []);
  const old = S.events[track] || [];
  const kept = old.filter(e => !(e.learned && e.added && !at.some(x => Math.abs(x - e.t) <= LN_CANDIDATE_REACH)));
  if (kept.length === old.length) return false;
  S.events[track] = kept;
  EDGEN[track] = (EDGEN[track] || 0) + 1;   // the list was replaced (an edit plan made on the old one is void)
  invalidateHits(track); refreshCounts(track); updateStatus();
  return true;
}
const LN_HINT_NOT = "Forget This alone takes only this hit and near copies: add a Learn This example (K) so similar bleed goes too.";
const LN_HINT_OFF = "Learn filter off: Shift+K on bleed or tick Filter to use it";
function lnHint(learn) {   // P4-6c: the hint of the Learn section: Forget This alone, with the threshold following the examples
  const c = lnCounts(learn);
  return learn && learn.on === true && learn.auto === true && c.this === 0 && c.not > 0 ? LN_HINT_NOT : "";
}
function lnResult(r) {   // P4-6c: the hits and the grey hits after a Learn change, for the status line
  if (r && r.filter_error) return ` (the filter applies on the next Re-analyze: ${r.filter_error})`;
  if (!r || !r.filter) return "";
  const c = lnOutCounts(r.track), hint = lnHint(r.learn);
  return `. ${c.live} hits, Learn takes ${c.learnOut} out${hint ? `. ${hint}` : ""}`;   // the same counts as the Learn row
}
/* P4-35: Learn guard. One Learn step (mark, remove, flip, forget, filter on or off) that takes out many more hits than
   before gets a dialog: Keep, or Undo (the normal undo step of that change). The counts come from the service's answer
   (learn_out_before, learn_out_after, hits, hits_before); a threshold drag is an explicit choice and never asks. */
function lnGuardLimit(liveBefore) { return Math.max(10, 0.25 * liveBefore); }   // P4-35: the hits one filter step may take out (10, or 25 % of the output hits before)
function lnGuardCuts(r) {   // the number of hits the step takes out beyond the limit lnGuardLimit, else 0
  if (!r || !r.filter || !Number.isFinite(r.learn_out_after) || !Number.isFinite(r.learn_out_before)) return 0;
  const more = r.learn_out_after - r.learn_out_before, liveBefore = (r.hits_before || 0) - r.learn_out_before;
  return more > lnGuardLimit(liveBefore) ? more : 0;
}
function lnGuardAdds(r) {   // P4-38b: the hits the step ADDS beyond max(10, 25 % of the live hits before), else 0
  if (!r || !Number.isFinite(r.hits) || !Number.isFinite(r.hits_before)) return 0;
  const more = r.hits - r.hits_before;
  return more > Math.max(10, 0.25 * r.hits_before) ? more : 0;
}
function lnGuard(r) {   // not awaited by the Learn queue: the Undo button joins that queue
  const adds = lnGuardAdds(r);
  if (!lnGuardCuts(r)) {
    if (!adds) return;
    askConfirm({ title: `Learn adds many hits to ${r.track}`,
      msg: `Learn now adds ${adds} hits (before: ${r.hits_before}). The examples may be mixed up.`,
      buttons: [{ label: "Keep", act: "keep" }, { label: "Undo", act: "undo", cls: "primary" }] }).then(async a => {
      if (a !== "undo") return;
      await LN.chain;   // the step is on top of the undo stack once the queue is empty
      undo();
    });
    return;
  }
  askConfirm({ title: `Learn takes many hits out of ${r.track}`,
    msg: `Learn now takes ${r.learn_out_after} of ${r.hits} hits out (before: ${r.learn_out_before}). The examples may be mixed up.`,
    buttons: [{ label: "Keep", act: "keep" }, { label: "Undo", act: "undo", cls: "primary" }] }).then(async a => {
    if (a !== "undo") return;
    await LN.chain;   // the step is on top of the undo stack once the queue is empty
    undo();
  });
}
function lnOutCounts(target) {   // owner check 2026-10-15: count only what Learn changes; hits the bleed rules or the loudness cut already took out are not "filtered"
  const evs = S.events[target] || [], other = x => (!!x.cut || !!x.bleed) && !isManual(x);
  return { live: evs.filter(x => !hgIsCut(x)).length, learnOut: evs.filter(x => lnGreyHit(target, x) && !other(x)).length };
}
function lnWhat(r) {
  const c = r.counts || lnCounts(r.learn), n = r.mark ? r.mark.at.length : 0;
  const via = r.routed ? ` on ${r.track}` : "";
  return { n, c, via };
}
function learnMark(kind) {   // Hits > Learn This (K), Forget This (Shift+K)
  if (!needProject()) return false;
  let name, a, hit = null, hitId = null;
  if (LN.sel) { name = LN.sel.track; a = { t0: LN.sel.t0, t1: LN.sel.t1 }; }
  else if (S.sel && (S.events[S.sel.track] || [])[S.sel.index]) { name = S.sel.track; hitId = S.events[name][S.sel.index].id; hit = S.events[name][S.sel.index].t; a = { hit }; }
  else { setMsg("select a range or a hit first (drag in a wave lane, or click a hit)"); return false; }
  const track = S.tracks[name], routed = track && track.follows && S.tracks[track.follows] ? track.follows : null;
  return lnQueue(async () => {
    await flushEvents();   // the hit edits on their way first (P4-9 stage 2)
    if (hitId !== null) {   // the time of the hit when the call runs: a nudge asked before it has moved the hit
      const e = (S.events[name] || []).find(x => x.id === hitId);
      if (!e) { setMsg(`${name}: the hit is not there any more`); return; }
      a = { hit: e.t };
    }
    const r = await B.learnMark(S.sid, name, kind, a.t0, a.t1, a.hit);
    if (!r.marked) { setMsg(r.reason === "already marked" ? `${name}: ${hit === null ? "those hits are" : "that hit is"} already ${kind === "this" ? "learned" : "marked not"}` : `${name}: no hit in the selection`); return; }
    // P4-6c (controller 2026-10-13): the first Shift+K of a track whose filter is off turns the filter on in the
    // service (r.filter_on), with the threshold following the examples; K alone never does. One undo step
    // (track_learn_set with the learn before re-picks again)
    const before = lnApply(r.track, r.learn, r);
    lnPush(r, before, kind === "this" ? "Learn This" : "Forget This", a.hit !== undefined ? a.hit : a.t0); lnGuard(r);
    lnSetSel(null);
    const w = lnWhat(r), off = !r.learn || r.learn.on !== true;
    const matchHint = kind === "not" && r.learn && r.learn.on && !lnHint(r.learn) ?   // P4-37b: how strict the current Match is (not next to the "Forget This alone" hint)
      `. The current Match takes ${lnOutCounts(r.track).learnOut} similar ${lnOutCounts(r.track).learnOut === 1 ? "hit" : "hits"} out - drag the Match line in the histogram for more or fewer` : "";
    const pre = routed ? `learned on ${r.track} (${name} follows it): ` : `${name}: `;
    if (r.added_hit != null && r.learn) {   // P4-38b: K on a missed hit added it: the status says what the Add rule does from here (P4-38d: with the filter off too, which a K-add never turns on)
      const tail = `. Learn adds ${lnAddCount(r.track, lnAddOf(r.track))} similar hits (Add ${lnAddOf(r.track).toFixed(2)}) - drag the Add line for more or fewer`;
      setMsg(`${pre}added 1 hit at ${r.added_hit.toFixed(3)} s and learned it (${w.c.this} learn, ${w.c.not} forget)${r.filter_error ? ` (the filter applies on the next Re-analyze: ${r.filter_error})` : ""}${tail}`);
      return;
    }
    setMsg(`${pre}${kind === "this" ? "learned" : "Forget This on"} ${w.n} ${w.n === 1 ? "hit" : "hits"}${r.flipped ? `, ${r.flipped} changed kind` : ""} (${w.c.this} learn, ${w.c.not} forget)${r.filter_on ? ", Learn filter on" : ""}${lnResult(r)}${off && kind === "this" ? `. ${LN_HINT_OFF}` : ""}${matchHint}`);
  });
}
function lnTrackFor() { return S.selTrack || (LN.sel && LN.sel.track) || (LN.mark && LN.mark.track) || (S.sel && S.sel.track) || null; }
function learnForget() {   // Hits > Forget Examples of Track
  if (!needProject()) return false;
  const name = lnTrackFor();
  if (!name) { setMsg("no track selected: click a track header first"); return false; }
  const target = lnTarget(name);
  if (!lnLearn(target)) { setMsg(`${target}: no Learn examples to forget`); return false; }
  return lnQueue(async () => {
    const r = await B.learnForget(S.sid, name);
    const before = lnApply(r.track, r.learn, r);
    lnPush(r, before, "Forget Examples"); lnGuard(r);
    if (LN.mark && LN.mark.track === r.track) lnSetMark(null);
    setMsg(`${r.track}: forgot ${r.examples} ${r.examples === 1 ? "example (Undo brings it back)" : "examples (Undo brings them back)"}${lnResult(r)}`);
  });
}
function learnForgetThis() {   // P4-36: Hits > Remove Example (Option+K): the example at the selected range or hit, else the selected mark bar
  if (!needProject()) return false;
  let name, a, hitId = null;
  if (LN.sel) { name = LN.sel.track; a = { t0: LN.sel.t0, t1: LN.sel.t1 }; }
  else if (S.sel && (S.events[S.sel.track] || [])[S.sel.index]) { name = S.sel.track; hitId = S.events[name][S.sel.index].id; a = {}; }
  else if (LN.mark) return learnRemove(LN.mark.track, LN.mark.id);
  else { setMsg("Remove Example: select a hit, a range or a mark first"); return false; }
  if (!lnLearn(lnTarget(name))) { setMsg(`${lnTarget(name)}: no Learn example to forget`); return false; }
  return lnQueue(async () => {
    await flushEvents();   // the time of the hit when the call runs, as in learnMark
    if (hitId !== null) {
      const e = (S.events[name] || []).find(x => x.id === hitId);
      if (!e) { setMsg(`${name}: the hit is not there any more`); return; }
      a = { hit: e.t };
    }
    const r = await B.learnForgetAt(S.sid, name, a.t0, a.t1, a.hit);
    if (!r.removed) { setMsg(`${r.track}: no Learn example ${a.hit === undefined ? "in the selection" : "at that hit"}`); return; }   // nothing changed: no undo step
    const evBefore = r.swept ? lnSnap(r.track) : null;   // the sweep drops hits: snapshot them for the undo step
    const before = lnApply(r.track, r.learn, r);
    if (evBefore) { lnSweepLocal(r.track, r.learn); LN.snap = { evBefore, evAfter: lnSnap(r.track) }; }
    lnPush(r, before, "Remove Example", a.hit !== undefined ? a.hit : a.t0); lnGuard(r);
    if (LN.mark && LN.mark.track === r.track && !(r.learn && r.learn.marks.some(m => m.id === LN.mark.id))) lnSetMark(null);
    if (a.hit === undefined) lnSetSel(null);
    setMsg(`${r.track}: forgot ${r.removed} ${r.removed === 1 ? "example (Undo brings it back)" : "examples (Undo brings them back)"}${lnResult(r)}`);
  });
}
function learnRemove(track, id) {   // a mark bar: Remove Example, or Delete with the bar selected
  const m = lnLearn(track) && lnLearn(track).marks.find(x => x.id === id);
  if (!m) return false;
  return lnQueue(async () => {
    const r = await B.learnRemove(S.sid, track, id);
    const before = lnApply(r.track, r.learn, r);
    lnPush(r, before, "Remove Example"); lnGuard(r);
    if (LN.mark && LN.mark.id === id) lnSetMark(null);
    setMsg(`${r.track}: removed ${r.removed} ${r.removed === 1 ? "example" : "examples"}${lnResult(r)}`);
  });
}
function learnFlip(track, id) {
  if (!lnLearn(track)) return false;
  return lnQueue(async () => {
    const r = await B.learnFlip(S.sid, track, id);
    const before = lnApply(r.track, r.learn, r);
    lnPush(r, before, "Flip Example"); lnGuard(r);
    setMsg(`${r.track}: mark ${id} is now ${r.kind === "this" ? "Learn This" : "Forget This"}${lnResult(r)}`);
  });
}
function lnDeleteMark() {   // Delete with a mark bar selected (and no hit): true when it took the key
  if (!LN.mark || S.sel) return false;
  learnRemove(LN.mark.track, LN.mark.id);
  return true;
}
/* ---------------- P4-6c: the similarity scores in the page (the filter, the histogram, the grey hits) ----------------
   The filter runs in the service: a Learn change with the filter on re-picks the track there (lnApply adopts the
   stored hits), and a hit the filter takes out stays a hit marked "filtered": true, drawn grey (hgIsCut), skipped
   by Tab, out of MIDI, spikes, clicks and followers. The scores of a track (track_learn_scores,
   crates/app/src/learn_score.rs) feed the histogram, the info line and the live grey of a threshold drag; they
   are cached per (sid, target track, learn object), and a Learn change drops them (lnScoreDrop). */
const LN_SC = {};
function lnFilterOn(name) {   // the filter draws grey only when the track's learn says on
  const t = S.tracks[name], learn = t && t.learn;
  return !!(learn && learn.on === true && Array.isArray(learn.marks) && learn.marks.length);
}
function lnScoreSig(name) {
  return JSON.stringify([S.sid, lnTarget(name), S.tracks[lnTarget(name)] && S.tracks[lnTarget(name)].learn || null]);
}
function lnScoreDrop(name) {   // the learn changed: drop the cache and refetch (the grey hits and the histogram follow)
  const target = lnTarget(name);
  delete LN_SC[target];
  lnScores(target).catch(() => {});
}
function lnScores(name) {   // the data of the target track (async; the sync map follows when it lands)
  const target = lnTarget(name), sig = lnScoreSig(name);
  let e = LN_SC[target];
  if (!e || e.sig !== sig) {
    e = LN_SC[target] = { sig, data: null, byT: new Map(), byGate: new Map(), threshold: 0.5 };
    e.p = B.learnScores(S.sid, target).then(r => {
      if (r && r.state === "ok") {
        e.data = r;
        e.threshold = r.threshold;
        e.byT = new Map(r.candidates.map(row => [Math.round(row[0] * 1000) / 1000, row[2]]));
        e.byGate = new Map(r.candidates.map(row => [Math.round(row[0] * 1000) / 1000, row[4] === true]));   // #293 B8: the attack gate keeps it out
      }
      lnRedraw(target);
      const row = document.querySelector("#insParams .lnRow"); if (row) { lnHistDraw(target); invalidateHits(target); }
      return e;
    }).catch(() => e);
  }
  return e.p;
}
function lnScoreSync(name) {   // the sync view of the data: {on, threshold, byT, byGate} or null (the first draw after a mark)
  const target = lnTarget(name), e = LN_SC[target];
  return e && e.data && e.data.state === "ok" ? { on: lnFilterOn(name), threshold: e.threshold, byT: e.byT, byGate: e.byGate } : null;
}
function lnNearestCand(map, t) {   // the value of the candidate nearest to t (unshifted times) in a map keyed by time, or null
  let best = null, bd = 0.051;
  for (const [ct, v] of map) { const d = Math.abs(ct - t); if (d <= bd) { bd = d; best = v; } }
  return best;
}
function lnScoreAt(name, t) {   // the score of the candidate nearest to t (unshifted times), or null
  const sc = lnScoreSync(name); return sc ? lnNearestCand(sc.byT, t) : null;
}
function lnShift(name) { const t = S.tracks[name]; return t && t.sr && t.params ? (t.params.shift_samples || 0) / t.sr : 0; }
function lnPreview(name, e) {   // P4-6c: during a threshold drag the grey of a hit follows the live threshold (null: no drag)
  const x = LN_SC[lnTarget(name)]; if (!x || !x.drag || !lnFilterOn(name)) return null;
  if (isManual(e)) return false;   // an edited hit is never filtered
  const lnT = lnTimes(name);
  if (lnT.this.length && lnNear(lnT.this, e.t)) return false;   // a "this" example stays
  if (lnT.not.length && lnNear(lnT.not, e.t)) return true;
  const s = lnScoreAt(name, e.t - lnShift(name));
  const sc = lnScoreSync(name), gated = sc !== null && lnNearestCand(sc.byGate, e.t - lnShift(name)) === true;   // #293 B8: the attack gate keeps it out, as the service stores it
  return s !== null && s < x.threshold && !gated;
}
function lnGreyHit(name, e) {   // P4-6c: a grey hit of the Learn filter (the stored mark, or the drag's preview)
  const pv = lnPreview(name, e);
  return pv === null ? !!e.filtered && !isManual(e) : pv;
}
function lnHistDraw(name) {   // the Similarity histogram of the Learn row (concept 3.2)
  const cv = document.querySelector("#inspector .lnHist"); if (!cv) return;
  const target = lnTarget(name), e = LN_SC[target], ctx = cv.getContext("2d");
  const dpr = devicePixelRatio || 1, W = cv.width = cv.clientWidth * dpr, H = cv.height = 60 * dpr;
  ctx.clearRect(0, 0, W, H);
  const info = document.querySelector("#inspector .lnHistInfo");
  const sc = lnScoreSync(name); if (!sc || !e || !e.data || e.data.state !== "ok") { ctx.fillStyle = "#88909c"; ctx.textBaseline = "alphabetic"; ctx.font = `${11 * dpr}px sans-serif`; ctx.fillText("no scores yet", 8, H / 2); if (info) info.textContent = ""; return; }
  const rows = e.data.candidates, sens = Number.isFinite(e.data.sens) ? e.data.sens : 150;
  const NB = 50, bins = new Array(NB).fill(0), binsOut = new Array(NB).fill(0);
  for (const row of rows) { const b = Math.min(NB - 1, Math.max(0, Math.floor(row[2] * NB))); if (row[3] <= sens) bins[b]++; else binsOut[b]++; }
  if (info) {   // the counts under the histogram (concept 2.7): the hits, the grey ones, the Match strictness
    const { live, learnOut } = lnOutCounts(target);
    const addPart = lnHasThis(name) ? ` · Add ${lnAddOf(name).toFixed(2)} (adds ${lnAddCount(name, lnAddOf(name))})` : "";
    info.textContent = `${live} hits · Learn takes ${learnOut} out · Match ${sc.threshold.toFixed(2)}${e.drag ? "" : lnAutoOn(target) ? " auto" : ""}${addPart}`;
  }
  // #210: the labels live above the bars. Lay them out first (font and margins scale with dpr), so the
  // bar area can shrink by the label band and no glyph is clipped at the top of the canvas.
  const fontPx = 10 * dpr, topPad = 2 * dpr, lineH = Math.round(fontPx * 1.1);
  ctx.font = `${fontPx}px sans-serif`; ctx.textBaseline = "top";
  const thX = sc.threshold * W;
  const lbl = `Match ${sc.threshold.toFixed(2)}`, lw = ctx.measureText(lbl).width;
  const mx = Math.max(2, Math.min(W - lw - 2, thX + 4));
  let add = null, albl = "", aw = 0, axx = 0, ay = topPad;
  if (lnHasThis(name)) {   // P4-38b: the Add line (only with a Learn This example) and its live count
    add = e.dragAdd ?? lnAddOf(name);
    albl = `Add ${add.toFixed(2)} · adds ${lnAddCount(name, add)}`; aw = ctx.measureText(albl).width;
    axx = Math.max(2, Math.min(W - aw - 2, add * W - aw - 4));   // left of the Add line
    const clash = x2 => x2 < mx + lw + 2 && x2 + aw > mx - 2;
    if (clash(axx)) {   // would overlap the Match label: try the other side of the Add line, else a second line
      const rx = Math.max(2, Math.min(W - aw - 2, add * W + 4));
      if (clash(rx)) { ay = topPad + lineH; axx = Math.max(2, Math.min(W - aw - 2, add * W - aw - 4)); }
      else axx = rx;
    }
  }
  const labelBand = ay > topPad ? topPad + 2 * lineH : topPad + lineH;   // the rows the labels occupy
  const max = Math.max(1, ...bins, ...binsOut), bw = W / NB, barBot = H - 12 * dpr;
  for (let b = 0; b < NB; b++) {
    const h = bins[b] / max * (barBot - labelBand), ho = binsOut[b] / max * (barBot - labelBand);
    if (bins[b]) { ctx.fillStyle = "#4da3ff"; ctx.fillRect(b * bw + 1, barBot - h, bw - 2, h); }
    if (binsOut[b]) { ctx.strokeStyle = "#4da3ff"; ctx.strokeRect(b * bw + 1.5, barBot - ho, bw - 3, ho); }
  }
  ctx.strokeStyle = "#e5534b"; ctx.beginPath(); ctx.moveTo(thX, 0); ctx.lineTo(thX, barBot); ctx.stroke();
  ctx.fillStyle = "#e5534b";   // P4-37b: the strictness label
  ctx.fillText(lbl, mx, topPad);
  if (add !== null) {
    ctx.strokeStyle = "#4cc38a"; ctx.beginPath(); ctx.moveTo(add * W, 0); ctx.lineTo(add * W, barBot); ctx.stroke();
    ctx.fillStyle = "#4cc38a";
    ctx.fillText(albl, axx, ay);
  }
  ctx.fillStyle = "#4cc38a";   // the this ticks
  for (const x of (e.data.examples.this || []).map(x2 => x2[1] * W)) { ctx.fillRect(x - 1.5, H - 10 * dpr, 3, 4 * dpr); }
  ctx.fillStyle = "#e5534b";   // the not ticks
  for (const x of (e.data.examples.not || []).map(x2 => x2[1] * W)) { ctx.fillRect(x - 1.5, H - 6 * dpr, 3, 4 * dpr); }
}
function lnAutoOn(name) { const l = lnLearn(name); return !!(l && l.auto === true); }
function lnAddOf(name) { const l = lnLearn(name); return l && typeof l.add === "number" ? l.add : 0.85; }   // P4-38b
function lnHasThis(name) { const t = lnTimes(name); return t.this.length > 0; }
function lnAddCount(name, add) {   // P4-38b: how many below-Sens candidates the Add rule would add
  const target = lnTarget(name), e = LN_SC[target];
  if (!e || !e.data || e.data.state !== "ok") return 0;
  const sens = Number.isFinite(e.data.sens) ? e.data.sens : 150;
  let n = 0;
  for (const row of e.data.candidates) {
    if (row[3] <= sens) continue;
    const s = lnScoreAt(name, row[0]);
    if (s !== null && s >= add) n++;
  }
  return n;
}
function lnSetLearn(name, change, msg) {   // P4-6c: one change of the learn object (filter on/off, threshold, Auto): one undo step
  const target = lnTarget(name), cur = S.tracks[target] && S.tracks[target].learn; if (!cur) return false;
  const next = Object.assign({}, cur, change);
  return lnQueue(async () => {
    const r = await B.learnSet(S.sid, target, next);
    const before = lnApply(r.track, r.learn, r);
    lnPush(r, before, "on" in change ? `Learn Filter ${change.on ? "On" : "Off"}` : "Learn Settings"); if ("on" in change) lnGuard(r);   // filter on or off asks; a threshold or Auto change is an explicit choice
    setMsg(`${r.track}: ${msg}${lnResult(r)}`);
  });
}
function lnHistSetThreshold(name, th) {   // the release of a threshold drag: the threshold is the engineer's now (auto off)
  const v = Math.round(th * 1000) / 1000;
  return lnSetLearn(name, { threshold: v, auto: false }, `similarity threshold ${v.toFixed(2)}`);
}

function lnApplyOp(op, reverse) {   // applyOp: undo and redo of a Learn step
  const learn = (reverse ? op.before : op.after) || null, events = reverse ? op.evBefore : op.evAfter;   // P4-6c: the snapshot, exactly
  lnQueue(async () => {
    const r = await B.learnSet(S.sid, op.track, learn, events);
    lnApply(r.track, r.learn, r);
    if (LN.mark && !lnLearn(op.track)) lnSetMark(null);
    setMsg(`${reverse ? "undo" : "redo"}: Learn marks of ${op.track}`);
  });
}
function learnFilter() {   // Hits > Learn Filter (a check item; the shell syncs it): the check box of the Learn row
  const name = lnTrackFor(), learn = name && lnLearn(lnTarget(name));
  if (!learn || !(learn.marks || []).length) { setMsg("no Learn examples: mark hits with K or Shift+K first"); return false; }
  return lnSetLearn(name, { on: learn.on !== true }, `Learn filter ${learn.on !== true ? "on" : "off"}`);
}
/* the context menus: right-click inside the selection (Learn This, Forget This, Clear Selection) or on a mark bar */
function lnContext(ev, name, lane) {
  const px = laneXOf(ev), W = laneWidth();
  let items = null;
  const m = lane === "spike" ? lnMarkAt(name, px, ev.offsetY) : null;
  if (m) {
    lnSetMark(name, m.id);
    items = [["remove", "Remove Example", () => learnRemove(name, m.id)],
      ["flip", m.kind === "this" ? "Change to Forget This" : "Change to Learn This", () => learnFlip(name, m.id)]];
  } else if (LN.sel && LN.sel.track === name) {
    const t = xToT(px, W);
    if (t >= LN.sel.t0 && t <= LN.sel.t1) items = [["learn", "Learn This", () => learnMark("this")], ["not", "Forget This", () => learnMark("not")], ["clear", "Clear Selection", () => lnClearSel()]];
  }
  if (!items) return false;
  ev.preventDefault(); openCtxItems(items, ev.clientX, ev.clientY);
  return true;
}
/* the Inspector Learn row: counts or the hint, the buttons, the collapsed example list */
function lnInspector() {
  const row = document.querySelector("#insParams .lnRow"); if (!row || !INS.name) return;
  const name = INS.name, tgt = lnTarget(name), learn = lnLearn(tgt), c = lnCounts(learn);
  const can = !!LN.sel || !!(S.sel && (S.events[S.sel.track] || [])[S.sel.index]);
  const sig = JSON.stringify([name, tgt, learn, can, LN.mark, LN.pending]);
  if (sig === LN.sig) return;
  LN.sig = sig; insSumRefresh();
  const follows = tgt !== name;
  row.querySelector(".lnCount").textContent = learn
    ? `${follows ? `on ${tgt}: ` : ""}✓ ${c.this}  ✗ ${c.not}${learn.on === false ? "  (filter off)" : ""}${c.this + c.not === 1 ? "  · 1 example: add 2 more for a stable result" : ""}`
    : follows ? `follows ${tgt}: no examples there` : "Select hits you want and press K";
  const hintEl = row.querySelector(".lnHintMsg"), hint = lnHint(learn);   // P4-6c: Forget This alone
  if (hintEl) { hintEl.textContent = hint; hintEl.classList.toggle("hidden", !hint); }
  row.querySelector(".lnThis").disabled = !can; row.querySelector(".lnNot").disabled = !can;
  row.querySelector(".lnForgetThis").disabled = !learn || !(can || LN.mark);
  row.querySelector(".lnForget").disabled = !learn;
  const box = row.querySelector(".lnFilterBox");   // P4-6c: the checkbox follows the stored state
  if (box) { box.disabled = !learn || !Array.isArray(learn.marks) || !learn.marks.length; box.checked = !!(learn && learn.on === true); }
  const hist = row.querySelector(".lnHist"), on = lnFilterOn(tgt);   // concept 2.4: filter off, the histogram is hidden
  if (hist) { hist.classList.toggle("hidden", !on); row.querySelector(".lnHistRow").classList.toggle("hidden", !on); }
  if (hist && on && !hist.dataset.dragging) lnHistDraw(name);
  const rows = row.querySelector(".lnRows"); rows.textContent = "";
  for (const m of (learn && learn.marks) || []) {
    const d = document.createElement("div"); d.className = "lnMk" + (LN.mark && LN.mark.id === m.id && LN.mark.track === tgt ? " sel" : ""); d.dataset.id = m.id; d.dataset.kind = m.kind;
    const dot = document.createElement("span"); dot.className = `lnDot ${m.kind}`; dot.textContent = m.kind === "not" ? "✗" : "✓";
    const txt = document.createElement("span"); txt.className = "lnTxt";
    txt.textContent = `${m.id}  ${fmt(m.t0, 3)}${m.t1 !== m.t0 ? ` to ${fmt(m.t1, 3)}` : ""} s  ·  ${(m.at || []).length} ${(m.at || []).length === 1 ? "hit" : "hits"}`;
    const x = document.createElement("button"); x.className = "btn mini lnX"; x.textContent = "×"; x.title = "Remove this mark";
    x.onclick = ev => { ev.stopPropagation(); learnRemove(tgt, m.id); };
    d.onclick = () => {   // a click on a row selects the mark and moves the playhead to its first example (concept 2.7)
      lnSetMark(tgt, m.id);
      const t = ((m.at || [])[0] ?? m.t0) + lnTimes(tgt).shift;
      S.followHold = true; S.playhead = Math.max(0, t);
      if (t < S.view.t0 || t > S.view.t1) centerOn(t);
      invalidate(F.HEAD); updateStatus();
    };
    d.append(dot, txt, x); rows.appendChild(d);
  }
}
function lnBuildRow(div, name) {   // buildParamsPanel: the button handlers of the Learn row
  const row = div.querySelector(".lnRow"); if (!row) return;
  row.querySelector(".lnThis").onclick = () => learnMark("this");
  row.querySelector(".lnNot").onclick = () => learnMark("not");
  row.querySelector(".lnForgetThis").onclick = () => learnForgetThis();
  row.querySelector(".lnForget").onclick = () => learnForget();
  const box = row.querySelector(".lnFilterBox"), learn = S.tracks[name] && S.tracks[name].learn;
  box.disabled = !learn || !Array.isArray(learn.marks) || !learn.marks.length;
  box.checked = !!(learn && learn.on === true);
  box.onchange = () => lnSetLearn(name, { on: box.checked }, `Learn filter ${box.checked ? "on" : "off"}`);   // P4-6c: one undo step (the examples stay)
  const auto = row.querySelector(".lnAuto");
  auto.onclick = ev => {   // the threshold follows the examples again (one undo step)
    ev.preventDefault();
    if (!lnAutoOn(lnTarget(name))) lnSetLearn(name, { auto: true }, "similarity threshold: automatic");
    return false;
  };
  const hist = row.querySelector(".lnHist");
  hist.onmousemove = ev => {   // P4-40b: the resize cursor within 6 px of the Match line or the Add line
    const e = LN_SC[lnTarget(name)], r = hist.getBoundingClientRect(), x = ev.clientX - r.left;
    const ready = !!(e && e.data && e.data.state === "ok");
    hist.classList.toggle("onLine", ready && (Math.abs(x - e.threshold * r.width) <= 6 || (lnHasThis(name) && Math.abs(x - lnAddOf(name) * r.width) <= 6)));
  };
  hist.onmouseleave = () => hist.classList.remove("onLine");
  hist.onmousedown = ev => {   // the threshold drag: the grey hits follow live (lnPreview), one undo step on release.
    // P4-38b: a mousedown near the Add line (green) drags the Add threshold instead
    const target = lnTarget(name), e = LN_SC[target];
    if (!e || !e.data || e.data.state !== "ok" || ev.button !== 0) return;
    ev.preventDefault();
    const r0 = hist.getBoundingClientRect();
    const addShown = lnHasThis(name);
    const addX = lnAddOf(name) * r0.width;
    const dragAdd = addShown && Math.abs(ev.clientX - r0.left - addX) <= 6;
    hist.dataset.dragging = "1";
    if (dragAdd) { e.dragAdd = lnAddOf(name); } else { e.drag = true; }
    const start = e.threshold, startAdd = lnAddOf(name);
    const move = ev2 => {
      const r = hist.getBoundingClientRect();
      const v = Math.round(Math.min(1, Math.max(0, (ev2.clientX - r.left) / r.width)) * 1000) / 1000;
      if (dragAdd) { e.dragAdd = v; } else { e.threshold = v; }
      lnHistDraw(name); invalidateHits(target);
    }, up = () => {
      delete hist.dataset.dragging;
      document.removeEventListener("mousemove", move); document.removeEventListener("mouseup", up);
      if (dragAdd) {
        const v = e.dragAdd; delete e.dragAdd;
        if (Math.abs(v - startAdd) > 1e-9) lnSetLearn(name, { add: v }, `Add threshold ${v.toFixed(2)}`);
        else lnHistDraw(name);
      } else if (Math.abs(e.threshold - start) > 1e-9) lnHistSetThreshold(target, e.threshold);
      else { e.drag = false; lnHistDraw(name); invalidateHits(target); }
    };
    document.addEventListener("mousemove", move); document.addEventListener("mouseup", up);
    move(ev);
  };
  LN.sig = ""; lnInspector();
  lnScores(name).then(() => { lnHistDraw(name); invalidateHits(name); });   // the scores arrive: the grey hits and the histogram
}
window.DSLearn = {   // the e2e tests read the state through this
  state: () => ({ sel: LN.sel && { ...LN.sel }, mark: LN.mark && { ...LN.mark }, pending: LN.pending, text: lnSelText(),
    learn: Object.fromEntries(S.order.filter(n => S.tracks[n].learn).map(n => [n, S.tracks[n].learn])) }),
  times: name => { const c = lnTimes(name); return { this: c.this.slice(), not: c.not.slice(), shift: c.shift }; },
  barX: (name, id) => { const m = lnLearn(name).marks.find(x => x.id === id); return lnBarSpan(m, laneWidth()).map(x => x + GEO.laneX); },
  hitKind: (name, t) => lnKindOf(name, t),
  pulsing: () => !!(LN.pulse && performance.now() < LN.pulse.until),
  colors: () => ({ this: LN_THIS, not: LN_NOT }),
  /* P4-6c: the scores of a track (the histogram and the grey-hit checks read this) */
  scores: name => { const e = LN_SC[lnTarget(name)]; return e && e.data; },
  scoresReady: name => lnScores(name).then(e => e && e.data),   // the scores of the current learn object (a promise)
  idle: () => LN.chain.then(() => edIdle()),   // every queued Learn change and its re-pick is applied
  filterOn: name => lnFilterOn(name),
  greyHit: (name, t) => { const e = (S.events[name] || []).find(x => Math.abs(x.t - t) < 0.03); return !!e && lnGreyHit(name, e); },
};

/* ---------------- P4-3: the Sensitivity and loudness histograms of the Inspector (owner request 2026-10-12) ----------------
   As in Massey DRT (docs/migration/research-massey-drt.md): each histogram is drawn in its control's own track.
   Sensitivity: every candidate hit of the track's activation (B.histograms, from the activations the analysis keeps in
   Analysis/<track>/activation.bin) at the slider value where it turns on: strong hits at the left, weak ones and bleed
   at the right. Coloured = detected at the slider value, grey = filtered. While the slider moves, the service picks the
   hits again from the kept activations (B.repick, no model run; the same hits as a Re-analyze at that value) and the
   lane shows them live, with the hits that go as grey lines; letting go stores the re-pick (one undo step; edited hits
   are kept as Re-analyze > Keep keeps them). The largest gap between the candidates is marked; "Set in gap" puts the
   slider there (a start value, never set on its own). Loudness: the peak level of every hit (dBFS) with the floor and
   ceiling handles of the velocity map and the 20 to 127 ramp over it; the velocities follow live (velOfDb, the core's
   velocity_of_db) and are stored when the handle is let go (one undo step, as Recompute Velocities). A follower shows
   its loudness histogram only. Both canvases draw only on a change: new data, a control, a resize; nothing per frame. */
const HG = { name: null, p: null, el: null, d: null, lv: new Map(), gen: 0, ghosts: {}, refresh: 0, ro: null, kept: null, draws: 0,
  sens: { base: null, want: null, busy: false, n: null, weak: null },   // base: the events and params at the drag start
  vel: { base: null, drag: null } };
const HG_THUMB = 12;   // px: the slider thumb (style.css .hgRange); the bars sit on the thumb's travel
const HG_GREY = "#4a505c";
function pyRound(x) { return Math.abs(x % 1) === 0.5 ? 2 * Math.round(x / 2) : Math.round(x); }   // Python round: ties to even
function velOfDb(d, lo, hi) {   // crates/core/src/velocity.rs velocity_of_db, the same doubles
  const denom = Math.max(hi - lo, 1e-6), vv = 20 + pyRound((d - lo) / denom * 107);
  return Math.max(20, Math.min(127, vv));
}
function hgSensX(v, W, max) { return HG_THUMB / 2 + (v / max) * (W - HG_THUMB); }
function hgCanvas(c) {   // size the backing store to the element; returns [ctx, W, H] in CSS px
  const W = c.clientWidth, H = c.clientHeight, dpr = window.devicePixelRatio || 1;
  if (c.width !== Math.round(W * dpr) || c.height !== Math.round(H * dpr)) { c.width = Math.round(W * dpr); c.height = Math.round(H * dpr); }
  const ctx = c.getContext("2d"); ctx.setTransform(dpr, 0, 0, dpr, 0, 0); ctx.clearRect(0, 0, W, H);
  HG.draws++;   // the tests check that nothing draws without a change
  return [ctx, W, H];
}
function hgAttach(div, name, p) {   // buildParamsPanel: the controls of this track
  HG.name = name; HG.p = p; HG.d = null; HG.lv = new Map(); HG.kept = null;
  HG.sens = { base: null, want: null, busy: false, n: null, weak: null }; HG.vel = { base: null, drag: null };
  HG.el = { sens: div.querySelector(".pSens"), sensCanvas: div.querySelector(".hgSensCanvas"), sensWrap: div.querySelector(".hgSensWrap"),
    count: div.querySelector(".hgSensCount"), gapBtn: div.querySelector(".hgGapBtn"), loud: div.querySelector(".hgLoudCanvas"),
    loudInfo: div.querySelector(".hgLoudInfo"), vlo: div.querySelector(".pVLo"), vhi: div.querySelector(".pVHi"), vcut: div.querySelector(".pVCut"),
    hint: div.querySelector(".pactions .phint") };
  hgRememberParams(p);
  HG.el.gapBtn.onclick = () => hgSetInGap();
  HG.el.sensWrap.onpointermove = hgSensHover; HG.tip = "";
  div.querySelector(".pGap").title = div.querySelector(".pRefine").title = "Applies on the next re-analyze";
  hgLoudPointer(HG.el.loud);
  if (HG.ro) HG.ro.disconnect();
  HG.ro = new ResizeObserver(() => hgDraw());   // a new Inspector width: draw again (only then)
  HG.ro.observe(HG.el.sensWrap); HG.ro.observe(HG.el.loud);
  hgLoad();
}
function hgApiEvents(name) { return (S.events[name] || []).map(e => ({ t: e.t })); }
async function hgLoad() {   // the data of the shown track: on a track change, after an analysis, an edit or a re-pick
  const name = HG.name, gen = ++HG.gen;
  if (!name || !S.sid || !S.inspector || !S.tracks[name] || S.tracks[name].offline) { HG.d = null; hgDraw(); return; }
  const evs = S.events[name] || [];
  try {
    const d = await B.histograms(S.sid, name, hgApiEvents(name));
    if (gen !== HG.gen || HG.name !== name) return;
    HG.d = d; HG.lv = new Map();
    const st = d.detection.state; HG.kept = st === "ok" ? true : st === "missing" || st === "no_model" ? false : HG.kept;
    evs.forEach((e, i) => HG.lv.set(e.id, d.loudness.levels[i]));
  } catch (e) { if (gen === HG.gen) { HG.d = null; if (!S.peakErr[name]) setMsgBg(`histograms: ${errText(e)}`); } }   // P4-39c: the first track is selected at open; a file the lane already calls unsupported has no second message
  hgDraw();
}
function hgScheduleRefresh(track) {   // refreshCounts: the hits of the shown track changed (not by a drag of this card)
  if (track && track !== HG.name) return;
  if (HG.sens.base || HG.vel.drag) return;
  clearTimeout(HG.refresh); HG.refresh = setTimeout(hgLoad, 250);
}
function hgDetection() { const d = HG.d && HG.d.detection; return d && d.state === "ok" ? d : null; }
function hgDraw() { if (HG.el && HG.el.sens && HG.el.sens.isConnected) { hgDrawSens(); hgDrawLoud(); insSumRefresh(); } }
/* P4-25 (owner): the Sensitivity histogram shows every candidate on the axis of its strength (log scale, strong at the left,
   HG_NB bins), not on the slider value, so a strong group spreads into its own cluster and the gap is a natural empty
   stretch. On = the track colour, off = grey (a bin with both is stacked). Heights: log(1 + count) against the tallest bin.
   The threshold is a line between the weakest candidate that is on and the strongest that is off; the gap marker sits
   between the weakest strong candidate and the strongest weak one (the largest gap of the slider values). */
const HG_NB = 80;
function hgBarHeights(counts, H) {   // log(1 + count) against the tallest bin, 2 px at least, never above the canvas
  const full = Math.max(2, H - 3), top = Math.max(1, ...counts);
  return { heights: counts.map(n => n ? Math.min(full, Math.max(2, Math.round(Math.log1p(n) / Math.log1p(top) * full))) : 0) };
}
function hgSensState() {   // the counts the bars show, per slider value and per strength bin; the line and marker as fractions of the axis
  const d = hgDetection(); if (!d) return null;
  const cur = +HG.el.sens.value, bins = new Array(d.max + 1).fill(0), cands = d.candidates;
  for (const c of cands) bins[c[3]]++;
  let active = 0; for (let v = 0; v <= Math.min(cur, d.max); v++) active += bins[v];
  const ls = cands.map(c => Math.log(Math.max(c[2], 1e-6)));
  const hi0 = ls.length ? Math.max(...ls) : 0, lo0 = ls.length ? Math.min(...ls) : 0, pad = (hi0 - lo0) * 0.03;   // the axis runs from the actual extremes, 3 % margin each side
  const hi = hi0 + pad, lo = lo0 - pad, span = hi - lo;
  const f = l => span > 0 ? (hi - l) / span : 0.5;   // 0 = the strongest, 1 = the weakest
  const sb = new Array(HG_NB).fill(0), son = new Array(HG_NB).fill(0);
  cands.forEach((c, i) => { const b = Math.round(f(ls[i]) * (HG_NB - 1)); sb[b]++; if (c[3] <= cur) son[b]++; });
  const edge = (arr, pick) => arr.length ? pick(...arr) : null;
  const onL = cands.flatMap((c, i) => c[3] <= cur ? [ls[i]] : []), offL = cands.flatMap((c, i) => c[3] > cur ? [ls[i]] : []);
  const a = edge(onL, Math.min), b = edge(offL, Math.max);   // the weakest on, the strongest off
  const lineF = a === null && b === null ? null : a === null ? 0 : b === null ? 1 : (f(a) + f(b)) / 2;
  let gapF = null;
  if (d.gap) {
    const strong = cands.flatMap((c, i) => c[3] < d.gap.from ? [ls[i]] : []), weak = cands.flatMap((c, i) => c[3] > d.gap.to ? [ls[i]] : []);
    if (strong.length && weak.length) gapF = (f(Math.min(...strong)) + f(Math.max(...weak))) / 2;
  }
  return { cur, bins, active, total: cands.length, gap: d.gap, max: d.max, nb: HG_NB, sb, son, lineF, gapF, hi, lo,
    ...hgBarHeights(sb, HG.el.sensCanvas.clientHeight) };
}
function hgStrX(f, W, nb) { return (f * (nb - 1) + 0.5) * (W / nb); }   // the x of a fraction of the strength axis (the bin centres)
const HG_SENS_TIP = "Every candidate hit of this track by its strength (log scale): the strongest at the left, weak ones and bleed at the right. Coloured = detected at this setting, grey = filtered out. The line is the Sensitivity threshold: drag the slider below. The triangle marks the largest gap between two clusters (Massey DRT: mind the gap); click it to snap the slider into the gap.";
function hgSensTitle(bar) {   // the tooltip of the canvas: the explanation, the counts and the thresholds; over a bar that bar's count
  const w = HG.el && HG.el.sensWrap; if (!w) return;
  w.title = (bar || "") + (bar ? "\n" : "") + HG_SENS_TIP + (HG.tip ? `\n${HG.tip}` : "");
}
function hgSensHover(ev) {
  const s = hgSensState(); if (!s) return;
  const r = HG.el.sensCanvas.getBoundingClientRect();
  const b = Math.max(0, Math.min(s.nb - 1, Math.floor((ev.clientX - r.left) / Math.max(1, r.width) * s.nb)));
  const n = s.sb[b], on = s.son[b];
  hgSensTitle(n ? `${n} ${n === 1 ? "candidate" : "candidates"} here: ${on} on, ${n - on} off` : "no candidate here");
}
function hgDrawSens() {
  const el = HG.el, d = hgDetection(), t = S.tracks[HG.name];
  const st = HG.d && HG.d.detection ? HG.d.detection.state : null;
  el.sensWrap.classList.toggle("hgEmpty", !d);
  el.gapBtn.classList.toggle("hidden", !d || !d.gap);
  const [ctx, W, H] = hgCanvas(el.sensCanvas);
  if (!d) {
    if (el.hint && st !== "follows" && !(t && t.follows)) el.hint.textContent = "detection changes apply on the next re-analyze";
    el.count.textContent = st === "follows" ? "" : st === "no_model" ? "the slider applies on the next re-analyze"
      : st === "missing" ? "re-analyze once to see the candidates; until then the slider applies on the next re-analyze" : "";
    return;
  }
  const s = hgSensState(), bw = W / s.nb, color = (t && t.color) || "#4da3ff";
  for (let b = 0; b < s.nb; b++) {
    const h = s.heights[b]; if (!h) continue;
    const n = s.sb[b], onH = s.son[b] ? Math.max(1, Math.round(h * s.son[b] / n)) : 0, x = b * bw + 0.5, w = Math.max(1, bw - 1);
    ctx.fillStyle = HG_GREY; ctx.fillRect(x, H - h, w, h - onH);
    ctx.fillStyle = color; ctx.fillRect(x, H - onH, w, onH);
  }
  if (s.lineF !== null) {   // the threshold: between the weakest candidate that is on and the strongest that is off
    const x = s.lineF === 0 ? 0.5 : s.lineF === 1 ? W - 0.5 : Math.round(hgStrX(s.lineF, W, s.nb)) + 0.5;   // all on: at the left edge; none on: at the right
    ctx.strokeStyle = LN_CSS.getPropertyValue("--accent").trim() || "#4da3ff"; ctx.lineWidth = 1;
    ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, H); ctx.stroke();
  }
  // the marker under the canvas: in the middle of the largest gap; a click snaps the slider into it
  const g = s.gap;
  el.gapBtn.classList.toggle("hidden", s.gapF === null);
  if (g && s.gapF !== null) {
    el.gapBtn.style.left = `${hgStrX(s.gapF, W, s.nb) - 5}px`;
    el.gapBtn.title = `Snap into the gap: ${s.bins.slice(0, g.from).reduce((a, b) => a + b, 0)} hits`;
  }
  const hits = HG.sens.n !== null ? HG.sens.n : (S.events[HG.name] || []).filter(e => !isManual(e)).length;
  let weak = 0; if (g) for (let v = g.from + 1; v <= Math.min(s.cur, s.max); v++) weak += s.bins[v];   // active candidates beyond the gap
  el.count.textContent = `${hits} hits` + (weak ? ` · ${weak} weak` : "");
  HG.tip = `${s.active} of ${s.total} candidates on` + (HG.sens.weak && HG.sens.weak.to != null ? `, retry at ${HG.sens.weak.to}` : "") + `\n${sensReadout(s.cur)}`;
  hgSensTitle();
}
function hgCanRepick() {   // HG.kept: true (the service keeps the activations), false (none: the slider applies on the next
  // re-analyze, as before P4-3), null (not known yet: the first re-pick tells)
  return HG.kept !== false && !analysisRunning && !SH.pending && !(S.tracks[HG.name] || {}).follows;
}
function hgSensInput(name, p) {
  if (name !== HG.name) return;
  if (!hgCanRepick() || HG.gapSet) { hgDrawSens(); return; }   // HG.gapSet: Set in gap stores once, with no preview round
  const s = HG.sens;
  if (!s.base) s.base = hgBase(name);
  if (!s.want || !s.want.store) s.want = { params: { ...p }, store: false };
  hgDrawSens(); hgPump(name);
}
function hgSensCommit(name, p) {
  if (name !== HG.name || !hgCanRepick()) { hgRememberParams(p); return; }
  if (!HG.sens.base) HG.sens.base = hgBase(name);
  HG.sens.want = { params: { ...p }, store: true };
  hgPump(name);
}
function hgBase(name) {   // the hits and the slider of the drag start (the undo step's before)
  return { events: (S.events[name] || []).map(e => ({ ...e })), thresholds: HG.baseParams.thresholds, sensitivity: HG.baseParams.sensitivity };
}
function hgRememberParams(p) { HG.baseParams = { thresholds: (p.thresholds || SENS.BASE).slice(), sensitivity: p.sensitivity }; }
async function hgPump(name) {   // one re-pick at a time; the newest value wins, a store is never dropped
  const s = HG.sens;
  if (s.busy || !s.want) return;
  const w = s.want; s.want = null; s.busy = true;
  try {
    await edIdle(); await settleEvents(name);   // P4-9: the service merges the edited hits it holds
    const r = await B.repick(S.sid, name, w.params, w.store);
    if (HG.name === name && s.base) {
      if (w.store) hgStored(name, r, w.params); else hgPreview(name, r);
    }
  } catch (e) {
    if (e && e.status === 409) {   // no kept activations (or a missing detection, an analysis running): the slider applies on the next re-analyze
      if (/re-analyze it once/.test(e.message)) HG.kept = false; else setMsg(`re-pick: ${errText(e)}`);
      hgRestore(name, true); s.want = null;
    } else { setMsg(`re-pick: ${errText(e)}`); hgRestore(name, false); }
  } finally {
    s.busy = false;
    if (s.want) hgPump(name);
  }
}
function hgTakeLevels(events, lv) {   // the levels of a re-pick for the loudness histogram (aligned with the events)
  if (!HG.d) return;
  HG.lv = lv;
  HG.d.loudness.levels = events.map(e => lv.has(e.id) ? lv.get(e.id) : -Infinity);
}
function hgGone(base, picked) {   // the detected hits of the base that the pick does not have: grey lines in the lane
  const ts = picked.map(e => e.t), out = [];
  let j = 0;
  for (const e of base) {
    if (isManual(e)) continue;
    while (j < ts.length && ts[j] < e.t - 5e-4) j++;
    if (!(j < ts.length && Math.abs(ts[j] - e.t) <= 5e-4)) out.push(e.t);
  }
  return out;
}
function hgPreview(name, r) {
  const s = HG.sens;
  const shown = adoptEvents(r.events, s.base.events), lv = new Map();   // the service merged the edited hits (keep_edited)
  shown.forEach((e, i) => { if (r.levels && Number.isFinite(r.levels[i])) lv.set(e.id, r.levels[i]); });
  S.events[name] = shown;   // shown, not written: the store follows on release
  hgTakeLevels(shown, lv);   // the loudness histogram follows the re-picked hits
  HG.ghosts[name] = hgGone(s.base.events, shown.filter(e => !isManual(e)));
  s.n = r.n_events; s.weak = r.weak_signal;
  if (S.sel && S.sel.track === name) S.sel = null;
  invalidateHits(name); insCounts(); hgDrawSens(); hgDrawLoud();
}
function hgStored(name, r, params) {
  const s = HG.sens, base = s.base, p = HG.p, t = S.tracks[name];
  const after = adoptEvents(r.events, base.events);   // stored by the service, edited hits kept
  S.events[name] = after; delete HG.ghosts[name];
  if (r.weak_signal && r.weak_signal.to != null) {   // the retry of a 0-hit pick: the params the service stored
    p.thresholds = r.weak_signal.thresholds; p.sensitivity = r.weak_signal.to; p.weak_signal = r.weak_signal;
  } else if (r.weak_signal) p.weak_signal = r.weak_signal;
  pushUndo({ type: "repick", track: name,
    before: { events: base.events, thresholds: base.thresholds, sensitivity: base.sensitivity },
    after: { events: after.map(e => ({ ...e })), thresholds: (p.thresholds || []).slice(), sensitivity: p.sensitivity } });
  s.base = null; s.n = null; s.weak = null;
  if (S.sel && S.sel.track === name) S.sel = null;
  setDirtyState(true);   // the service stored the merged hits
  invalidateHits(name); refreshCounts(name); updateStatus();
  const fol = r.entry && Array.isArray(r.entry.followers) ? r.entry.followers : [];
  if (fol.length) reloadFollowers(fol);
  setMsg(`${name}: ${r.n_events} ${r.n_events === 1 ? "hit" : "hits"} at sensitivity ${p.sensitivity}` + (r.weak_signal ? ` (${weakSignalText(r.weak_signal)})` : ""));   // #411: singular for one hit
  if (t === S.tracks[name] && INS.name === name) { hgRememberParams(p); insSensRefresh(p); insCounts(); insSumRefresh(); hgDrawSens(); }   // P4-25: in place, no rebuild of the panel (it flickered)
  hgLoad();
}
function insSensRefresh(p) {   // the slider, its readout and the note after a stored re-pick (the same as buildParamsPanel sets at build)
  const div = $("#insParams"), sens = div.querySelector(".pSens"); if (!sens) return;
  const cur = Array.isArray(p.thresholds) && p.thresholds.length === 5 ? p.thresholds : SENS.BASE;
  sens.value = Number.isFinite(p.sensitivity) ? Math.round(p.sensitivity) : Math.round(sensOf(cur));
  div.querySelector(".pSensVal").textContent = sensReadout(+sens.value);
  const txt = weakSignalText(p.weak_signal), note = div.querySelector(".pSensNote");
  note.textContent = txt; div.querySelector(".pSensNoteRow").classList.toggle("hidden", !txt);
}
function hgRestore(name, keepParams) {   // a failed re-pick: back to the hits (and unless kept, the slider) of the drag start
  const s = HG.sens, base = s.base; if (!base) return;
  S.events[name] = base.events; delete HG.ghosts[name];
  s.base = null; s.n = null; s.weak = null;
  invalidateHits(name);
  if (keepParams) { hgRememberParams(HG.p); hgDrawSens(); return; }
  HG.p.thresholds = base.thresholds; HG.p.sensitivity = base.sensitivity;
  if (INS.name === name) syncInspector(true);
}
function hgSetInGap() {
  const d = hgDetection(); if (!d || !d.gap) return;
  HG.el.sens.value = d.gap.mid;
  HG.gapSet = true;
  try { HG.el.sens.dispatchEvent(new Event("input")); } finally { HG.gapSet = false; }   // the readout and p; no preview round
  HG.el.sens.dispatchEvent(new Event("change"));   // one re-pick, stored
}
/* round 2: the params and maps that undo and redo send go out one after the other per track, so a fast undo and redo
   reach the service in the order they were made */
function hgSerial(track, call) {
  const q = HG.queue || (HG.queue = {});
  const next = (q[track] || Promise.resolve()).then(call).catch(e => setMsg(`${track}: ${errText(e)}`));
  q[track] = next;
  return next;
}
function hgRepickBusy() { return HG.sens.busy || !!HG.sens.want || !!HG.sens.base; }   // a re-pick (or its store) is on its way
function hgUndoWaits(op) {   // round 2: undo and redo of a re-pick step wait while a re-pick is on its way
  if (!op || (op.type !== "repick" && op.type !== "shift") || !hgRepickBusy()) return false;
  setMsg("a re-pick is being stored: undo and redo wait for it");
  return true;
}
/* P4-9: the Hitpoint shift and the hand-edit merge run in the service (track_shift_set; keep_edited of track_repick and
   analyze_start, Rust merge_edited). The page sends its pending edits first, asks, and shows the stored hits. */
function adoptEvents(raw, old) {   // the service's hits with page ids; an edited hit keeps the id of the page's copy (its undo steps)
  const man = (old || []).filter(isManual);
  return normEvents(raw).map(e => { const m = isManual(e) ? man.find(x => Math.abs(x.t - e.t) < 1e-9) : null; e.id = m ? m.id : newId(); return e; });
}
function shiftNow(name) { const t = S.tracks[name]; return Math.round((t && t.params && t.params.shift_samples) || 0); }
async function shiftApply(name, nv) {   // true when the service shifted the track to nv samples; the hits shown are the stored ones
  const sid = S.sid;
  if (hgRepickBusy()) { setMsg("a re-pick is being stored: try the shift again"); return false; }
  try {
    await settleEvents(name);
    const r = await B.shiftSet(sid, name, nv);
    if (sid !== S.sid || !S.tracks[name]) return false;
    const t = S.tracks[name], old = S.events[name] || [], fresh = normEvents(await B.events(sid, name));
    if (fresh.length === old.length) fresh.forEach((e, i) => { e.id = old[i].id; });   // the same hits: the ids (undo, selection) stay
    S.events[name] = fresh;
    t.params = t.params || defaultParams(t.role); t.params.shift_samples = r.shift_samples;
    if (S.sel && S.sel.track === name && S.sel.index >= fresh.length) S.sel = null;
    setDirtyState(true); invalidateHits(name); refreshCounts(name); updateStatus();
    reloadFollowers(followersOf(name));
    if (INS.name === name) {   // P4-25: in place, no panel rebuild (a rebuild took the focus of the stepper keys and flickered)
      const f = document.querySelector("#insParams .pShift");
      if (f && SH.pending <= 1) f.value = shiftNow(name);   // undo and redo show the value; a newer step in the queue keeps its own
      insSumRefresh();
    }
    return true;
  } catch (e) { setMsg(`shift: ${errText(e)}`); return false; }
}
async function shiftApplyOp(op, reverse) {   // applyOp: undo and redo of a shift (runs in the queue)
  const to = reverse ? op.from : op.to;
  const ok = await hgSerial(op.track, () => shiftApply(op.track, to));
  if (ok) setMsg(`${reverse ? "undo" : "redo"}: shift of ${op.track} to ${to} samples`);
  return !!ok;
}
function hgApplyRepickOp(op, reverse) {   // applyOp: undo and redo of a stored re-pick
  const st = reverse ? op.before : op.after, t = S.tracks[op.track];
  S.events[op.track] = st.events.map(e => ({ ...e }));
  if (t) {
    t.params = t.params || defaultParams(t.role); t.params.thresholds = st.thresholds;
    if (st.sensitivity === undefined) delete t.params.sensitivity; else t.params.sensitivity = st.sensitivity;
    // fix round: the service's params follow, so a Save writes the slider of the hits it shows; round 2: in order
    const sid = S.sid, params = JSON.parse(JSON.stringify(t.params));
    hgSerial(op.track, () => B.paramsSet(sid, op.track, params));
  }
  if (S.sel && S.sel.track === op.track) S.sel = null;
  markDirty(op.track); invalidateHits(op.track); refreshCounts(op.track); updateStatus();
  if (INS.name === op.track) syncInspector(true);
}

/* the loudness histogram: dBFS on x, the floor and ceiling handles, the velocity ramp, and the loudness cut (controller
   decision 4, DRT's Loudness slider filters): a third handle, off at the left edge. A detected hit below it is grey in
   the histogram and in the lane and leaves the output (MIDI, spikes, Export All, the click, a follower); it stays in the
   document (e.cut). An edited hit is never cut. */
function hgCutOf(p) { return Number.isFinite(p.vel_cut_db) ? p.vel_cut_db : null; }
function hgIsCut(e) { return (!!e.cut || !!e.bleed || !!e.filtered) && !isManual(e); }   // P4-14: a hit of the tom bleed rule shows as a cut hit (owner: tom-bleed-show); P4-6c: so does one the Learn filter took out
function hgCutOnly(e) { return (!!e.cut || !!e.bleed) && !isManual(e); }   // the cut and bleed marks without the Learn filter's (its drag preview decides that one)
function hgLoudRange() {   // P4-25: the actual dBFS range of the hits plus 2 dB, widened to hold the floor, the ceiling and the cut handles (at least 12 dB)
  if (HG.vel.drag && HG.vel.drag.range) return HG.vel.drag.range;   // a drag keeps its axis: the handle under the pointer must not move away
  const lv = (HG.d ? HG.d.loudness.levels : []).filter(Number.isFinite), c = hgCutOf(HG.p);
  const pts = [...lv, HG.p.vel_lo_db, HG.p.vel_hi_db, ...(c === null ? [] : [c])].filter(Number.isFinite);
  let lo = Math.max(-96, Math.floor(Math.min(...pts)) - 2), hi = Math.min(0, Math.ceil(Math.max(...pts)) + 2);
  if (hi - lo < 12) { lo = Math.max(-96, hi - 12); hi = Math.min(0, lo + 12); }
  const r = { lo, hi };
  if (HG.vel.drag) HG.vel.drag.range = r;
  return r;
}
function hgLoudX(db, W, r) { return 6 + (db - r.lo) / (r.hi - r.lo) * (W - 12); }
function hgLoudDb(x, W, r) { return r.lo + (x - 6) / (W - 12) * (r.hi - r.lo); }
function hgLoudState() {
  if (!HG.d) return null;
  const r = hgLoudRange(), bins = new Array(r.hi - r.lo + 1).fill(0), cutBins = new Array(r.hi - r.lo + 1).fill(0);
  const cut = hgCutOf(HG.p), evs = S.events[HG.name] || [];
  let nCut = 0;
  HG.d.loudness.levels.forEach((l, i) => {
    const b = Math.max(0, Math.min(bins.length - 1, Math.floor(l) - r.lo));
    bins[b]++;
    const e = evs[i];
    if (cut !== null && l < cut && !(e && isManual(e))) { cutBins[b]++; nCut++; }
  });
  return { range: r, bins, cutBins, n: HG.d.loudness.levels.length, nCut, floor: HG.p.vel_lo_db, ceiling: HG.p.vel_hi_db, cut };
}
function hgDrawLoud() {
  const c = HG.el.loud, st = hgLoudState(), t = S.tracks[HG.name];
  const [ctx, W, H] = hgCanvas(c);
  if (!st) { HG.el.loudInfo.textContent = ""; return; }
  const { range: r, bins, cutBins } = st, top = Math.max(1, ...bins), bw = Math.max(1, (W - 12) / bins.length);
  const color = (t && t.color) || "#4da3ff";
  for (let i = 0; i < bins.length; i++) {
    if (!bins[i]) continue;
    const h = Math.max(2, Math.round(Math.sqrt(bins[i] / top) * (H - 12))), x = hgLoudX(r.lo + i, W, r);
    const hc = cutBins[i] ? Math.round(h * cutBins[i] / bins[i]) : 0;   // the cut part of the bar is grey
    ctx.fillStyle = color; ctx.fillRect(x, H - h, bw, h - hc);
    if (hc) { ctx.fillStyle = HG_GREY; ctx.fillRect(x, H - hc, bw, hc); }
  }
  const yv = v => H - 2 - (v / 127) * (H - 12);
  const xf = hgLoudX(st.floor, W, r), xc = hgLoudX(st.ceiling, W, r);
  ctx.strokeStyle = "#cdd3de"; ctx.lineWidth = 1.5; ctx.beginPath();   // the velocity ramp 20 .. 127
  ctx.moveTo(6, yv(20)); ctx.lineTo(xf, yv(20)); ctx.lineTo(xc, yv(127)); ctx.lineTo(W - 6, yv(127)); ctx.stroke();
  const handle = (x, lab, col) => {
    ctx.strokeStyle = col; ctx.lineWidth = 2; ctx.beginPath(); ctx.moveTo(x, 8); ctx.lineTo(x, H); ctx.stroke();
    ctx.fillStyle = col; ctx.beginPath(); ctx.moveTo(x - 5, 0); ctx.lineTo(x + 5, 0); ctx.lineTo(x, 8); ctx.closePath(); ctx.fill();
    ctx.fillStyle = "#7d8593"; ctx.font = "9px sans-serif"; ctx.fillText(lab, Math.min(W - 18, x + 4), 9);
  };
  handle(xf, "20", "#4da3ff"); handle(xc, "127", "#4da3ff");
  const xcut = hgLoudX(st.cut === null ? r.lo : st.cut, W, r);
  if (st.cut !== null) { ctx.fillStyle = "rgba(0,0,0,0.35)"; ctx.fillRect(6, 0, xcut - 6, H); }   // the cut region
  handle(xcut, st.cut === null ? "cut" : "", "#ff9f43");
  HG.el.loudInfo.textContent = `${st.n} hits · vel 20 at ${st.floor} dB · 127 at ${st.ceiling} dB · ` +
    (st.cut === null ? "no cut" : `${st.nCut} cut below ${st.cut} dB`) + ` · axis ${r.lo} to 0 dBFS`;
}
function hgLoudPointer(c) {
  c.addEventListener("pointerdown", ev => {
    const st = hgLoudState(); if (!st || analysisRunning) return;
    const rect = c.getBoundingClientRect(), W = rect.width, x = ev.clientX - rect.left;
    const near = [["lo", hgLoudX(st.floor, W, st.range)], ["hi", hgLoudX(st.ceiling, W, st.range)],
      ["cut", hgLoudX(st.cut === null ? st.range.lo : st.cut, W, st.range)]].sort((a, b) => Math.abs(x - a[1]) - Math.abs(x - b[1]));
    HG.vel.drag = { which: near[0][0], range: st.range, W, left: rect.left };
    HG.vel.base = hgLoudBase(HG.name);
    c.setPointerCapture(ev.pointerId);
    hgVelMove(ev.clientX);
    ev.preventDefault();
  });
  c.addEventListener("pointermove", ev => { if (HG.vel.drag) hgVelMove(ev.clientX); });
  const up = () => { if (!HG.vel.drag) return; HG.vel.drag = null; hgVelCommit(HG.name, HG.p, HG.p.vel_lo_db, HG.p.vel_hi_db, hgCutOf(HG.p), true); };
  c.addEventListener("pointerup", up); c.addEventListener("pointercancel", up);
}
function hgLoudBase(name) {   // the floor, ceiling, cut, velocities and cut marks before a change (the undo step's before)
  const p = HG.name === name ? HG.p : S.tracks[name].params;
  return { lo: p.vel_lo_db, hi: p.vel_hi_db, cut: hgCutOf(p), v: new Map((S.events[name] || []).map(e => [e.id, [e.v, !!e.cut]])) };
}
function hgVelMove(clientX) {   // a handle follows the pointer in whole dB; the velocities and cut marks follow live, not written
  const dr = HG.vel.drag, p = HG.p;
  const db = Math.round(hgLoudDb(clientX - dr.left, dr.W, dr.range));
  if (dr.which === "lo") p.vel_lo_db = Math.max(dr.range.lo, Math.min(p.vel_hi_db - 1, db));
  else if (dr.which === "hi") p.vel_hi_db = Math.min(0, Math.max(p.vel_lo_db + 1, db));
  else p.vel_cut_db = db <= dr.range.lo ? null : Math.min(0, db);   // at the left edge: off
  hgShowLoudFields();
  hgLiveVelocities();
  hgDrawLoud();
}
function hgShowLoudFields() {
  const p = HG.p;
  HG.el.vlo.value = p.vel_lo_db; HG.el.vhi.value = p.vel_hi_db;
  HG.el.vcut.value = hgCutOf(p) === null ? "" : hgCutOf(p);
}
function hgLiveVelocities() {
  const name = HG.name, p = HG.p, cut = hgCutOf(p);
  for (const e of S.events[name] || []) {
    const d = HG.lv.get(e.id);
    if (d === undefined) continue;
    e.v = velOfDb(d, p.vel_lo_db, p.vel_hi_db);
    if (cut !== null && d < cut && !isManual(e)) e.cut = true; else delete e.cut;
  }
  invalidateHits(name); updateStatus();
}
async function hgVelCommit(name, p, lo, hi, cut, fromDrag) {   // a handle let go, or a number field: store the map, the cut and the velocities
  if (!Number.isFinite(lo) || !Number.isFinite(hi) || (cut !== null && !Number.isFinite(cut))) { if (HG.name === name) hgShowLoudFields(); return; }
  if (analysisRunning) {   // fix round: as the handles, the number fields wait for the analysis (the service answers 409 too)
    setMsg("velocity mapping: wait for the analysis to finish");
    if (HG.name === name) hgShowLoudFields();
    return;
  }
  const base = fromDrag && HG.vel.base ? HG.vel.base : hgLoudBase(name);
  HG.vel.base = null;
  p.vel_lo_db = lo; p.vel_hi_db = hi;
  if (cut === null) delete p.vel_cut_db; else p.vel_cut_db = cut;
  if (HG.el && HG.name === name) hgShowLoudFields();
  try {
    await edIdle();   // P4-9 stage 2: a hit edit on its way is stored first
    const sent = S.events[name] || [];
    const r = await B.velocitySet(S.sid, name, lo, hi, cut, sent.map(e => ({ t: e.t, id: e.id, ...(e.added ? { added: true } : {}), ...(e.orig !== undefined ? { orig: e.orig } : {}) })));
    if (r.follows) { S.events[name] = normEvents(r.events); invalidateHits(name); refreshCounts(name); }
    else {
      const op = { type: "loud", track: name, before: { lo: base.lo, hi: base.hi, cut: base.cut, hits: {} }, after: { lo, hi, cut, hits: {} } };
      r.events.forEach((e, i) => {
        const o = sent[i];
        if (!o || o.id !== e.id) return;
        const b = base.v.get(o.id) || [o.v, !!o.cut];
        op.before.hits[o.id] = b; op.after.hits[o.id] = [e.v, !!e.cut];
      });
      hgApplyLoudOp(op, false, true); pushUndo(op);
    }
    setMsg(`${name}: velocity 20 at ${lo} dB, 127 at ${hi} dB` + (cut === null ? "" : `, hits below ${cut} dB cut`));
  } catch (e) {
    setMsg(`velocities: ${errText(e)}`);
    // round 2: nothing was stored: the floor, ceiling and cut, the fields and the live velocities go back
    p.vel_lo_db = base.lo; p.vel_hi_db = base.hi;
    if (base.cut === null) delete p.vel_cut_db; else p.vel_cut_db = base.cut;
    for (const ev of S.events[name] || []) {
      const b = base.v.get(ev.id);
      if (!b) continue;
      ev.v = b[0]; if (b[1]) ev.cut = true; else delete ev.cut;
    }
    if (HG.el && HG.name === name) hgShowLoudFields();
    invalidateHits(name); updateStatus();
  }
  if (HG.name === name) hgDrawLoud();
}
function hgApplyLoudOp(op, reverse, initial) {   // applyOp of a "loud" step: the floor, ceiling and cut, the velocities and cut marks
  const st = reverse ? op.before : op.after, t = S.tracks[op.track];
  if (!t) return;
  t.params = t.params || defaultParams(t.role);
  t.params.vel_lo_db = st.lo; t.params.vel_hi_db = st.hi;
  if (st.cut === null) delete t.params.vel_cut_db; else t.params.vel_cut_db = st.cut;
  for (const e of S.events[op.track] || []) {
    const h = st.hits[e.id];
    if (!h) continue;
    e.v = h[0];
    if (h[1]) e.cut = true; else delete e.cut;
  }
  if (!initial && B.velocitySet) { const sid = S.sid; hgSerial(op.track, () => B.velocitySet(sid, op.track, st.lo, st.hi, st.cut, [])); }   // undo, redo: the stored map follows, in order
  markDirty(op.track); invalidateHits(op.track); refreshCounts(op.track); updateStatus();
  if (HG.name === op.track && HG.el) { hgShowLoudFields(); hgDrawLoud(); }
}
/* the test hook: what the histograms show (the e2e tests read the counts, the threshold and the handle positions) */
window.DSHist = {
  state() {
    const s = HG.el && HG.el.sens ? hgSensState() : null, l = HG.el ? hgLoudState() : null;
    const sw = HG.el ? HG.el.sensCanvas.clientWidth : 0, lw = HG.el ? HG.el.loud.clientWidth : 0;
    return { name: HG.name, detection: HG.d ? HG.d.detection.state : null, sens: s && { ...s, x: hgSensX(s.cur, sw, s.max), lineX: s.lineF === null ? null : hgStrX(s.lineF, sw, s.nb), gapX: s.gapF === null ? null : hgStrX(s.gapF, sw, s.nb), width: sw },
      hits: HG.sens.n, ghosts: (HG.ghosts[HG.name] || []).length, busy: HG.sens.busy || !!HG.sens.want,
      loud: l && { ...l, xFloor: hgLoudX(l.floor, lw, l.range), xCeiling: hgLoudX(l.ceiling, lw, l.range),
        xCut: hgLoudX(l.cut === null ? l.range.lo : l.cut, lw, l.range), width: lw },
      text: HG.el ? HG.el.count.textContent : "", draws: HG.draws, kept: HG.kept };
  },
  velOfDb, pyRound, sensX: hgSensX, loudX: hgLoudX, barHeights: hgBarHeights, strX: hgStrX,
};
async function saveMetaNote(name) {
  try { await B.setNote(S.sid, name, S.tracks[name].note); setDirtyState(true); }
  catch(e) { setMsg(`note failed: ${e.message}`, "error"); }
}
function refreshHeadNote(name) { refreshHeadRole(name); }   // P3-14a; P3-18: the follow text is on the same line

/* ---------------- Inspector (P3-14a; ux-daw-reference.md 3.4: the left zone, Cubase model) ----------------
   The zone shows the selected track (S.selTrack): name (editable), colour, role and note, source file and state (with
   Relink when offline), hit counts, the detection parameters (buildParamsPanel: the controls of the old params drawer)
   and the volume and pan of P3-16 (the same setTrackMix as the header). View > Show Inspector (Option+I) toggles it;
   the state is an app setting (<home>/settings.json, key "inspector"). Everything here writes the DOM from event
   handlers; no frame reads it, and the ResizeObserver of P3-14d gives the lanes their new width. */
const INS = { name: null };
function setInspector(on, save) {
  S.inspector = !!on;
  $("#inspector").classList.toggle("hidden", !S.inspector);
  const ib = $("#inspectorBtn"); if (ib) { ib.classList.toggle("on", S.inspector); ib.setAttribute("aria-pressed", S.inspector ? "true" : "false"); }   // P4-25: the toolbar button follows (menu, key, button all end here)
  if (S.inspector && HG.name) hgLoad();   // P4-3: the histograms load only while the Inspector shows
  if (save !== false && B.appSettingsSet) B.appSettingsSet("inspector", S.inspector).catch(e => setMsg(`settings: ${errText(e)}`));
  DSShell.sync();
}
function toggleInspector() { setInspector(!S.inspector); setMsg(S.inspector ? "Inspector shown" : "Inspector hidden"); }
/* P3-13c: the key preset ("default" = Pro Tools, or "cubase"); the app setting "key_preset". The page keys follow at once
   (DSKeys.preset); the menu accelerators follow through DSShell.sync (shell_sync carries the preset). */
function setKeyPreset(name, save) {
  if (!DSKeys.setPreset(name)) return false;
  const sel = $("#setKeyPreset");
  if (sel) sel.value = DSKeys.preset;
  if (save !== false && B.appSettingsSet) B.appSettingsSet("key_preset", DSKeys.preset).catch(e => setMsg(`settings: ${errText(e)}`));
  DSShell.sync();
  if (save !== false) setMsg(`key preset: ${DSKeys.presetName()}`);
  return true;
}
/* P4-29: native app feel. The page must not behave like a web page in the webview: no text selection (CSS user-select in
   style.css; here the events that CSS cannot stop: Select All from the native Edit menu, the browser context menu, drags of
   images and text, page zoom by pinch or Cmd+wheel or Cmd+plus). Text fields, text areas, contenteditable and `.selectable`
   text (About, error details) keep their selection, their menu and their drags. The app's own context menus (track header,
   spike lane, wave lane) call preventDefault themselves and stay. */
function nativeFeel() {
  const SEL = "input:not([type=checkbox]):not([type=radio]):not([type=button]):not([type=range]), textarea, [contenteditable=''], [contenteditable='true'], .selectable";
  const inText = t => !!(t && t.closest && t.closest(SEL));
  const node = n => n && (n.nodeType === 1 ? n : n.parentElement);
  document.addEventListener("contextmenu", ev => { if (!inText(ev.target)) ev.preventDefault(); });
  document.addEventListener("dragstart", ev => { if (!inText(ev.target)) ev.preventDefault(); });   // the app's own drags use mouse events, not HTML drag and drop
  document.addEventListener("selectstart", ev => { if (!inText(node(ev.target))) ev.preventDefault(); });
  document.addEventListener("mousedown", ev => {   // P4-29 fix: a double-click selects a word; the stray selection made the
    // selectionchange handler call removeAllRanges, which forced a layout inside the event dispatch (the frames test)
    if (ev.detail > 1 && !inText(ev.target)) ev.preventDefault();
  });
  /* No selectionchange handler here: a removeAllRanges in it forces a layout inside the event
     dispatch (the frames test). The stray selection cannot happen: the mousedown guard above
     stops the multi-click word selection and selectstart stops the drags. */
  for (const t of ["gesturestart", "gesturechange", "gestureend"]) document.addEventListener(t, ev => ev.preventDefault());
  window.addEventListener("wheel", ev => { if (ev.ctrlKey || ev.metaKey) ev.preventDefault(); }, { passive: false });   // pinch arrives as ctrl+wheel; the lanes zoom the timeline with it (P4 #145)
  window.addEventListener("keydown", ev => {
    const mod = ev.metaKey || ev.ctrlKey;
    if (!mod || ev.altKey) return;
    const k = ev.key.toLowerCase();
    if (k === "a" && !ev.shiftKey && !inText(ev.target)) ev.preventDefault();   // Select All outside a field: nothing to select (the app has no Select All of hits)
    else if (["+", "=", "-", "_", "0"].includes(k) && !(window.DSKeys && DSKeys.find(ev))) ev.preventDefault();   // page zoom; the app's own zoom keys stay
  }, true);
}
nativeFeel();


/* ---------------- track height (P3-14c; ux-daw-reference.md 3.4 row "Track height", 3.2 View > Track Height) ----------------
   Three heights for every track: the wave lane takes the extra height, the spike lane stays. The choice is an app setting
   (<home>/settings.json, key "track_height"). setTrackHeight writes two CSS variables; the lanes and the header follow by
   CSS, the ResizeObserver of P3-14d measures the new heights (GEO.waveH, GEO.spikeH are in sizeKey), and the next frame
   sizes the canvases and draws. No frame reads layout. Taller and Shorter (Cmd+Down, Cmd+Up; Cubase Zoom In/Out Tracks)
   step through the list; Small, Medium and Large are radio items. */
const TRACK_HEIGHTS = { small: { spike: 34, wave: 56 }, medium: { spike: 34, wave: 120 }, large: { spike: 34, wave: 240 } };
const HEIGHT_ORDER = ["small", "medium", "large"];
function laneHeights() { return TRACK_HEIGHTS[S.trackHeight] || TRACK_HEIGHTS.medium; }
function setTrackHeight(name, save) {
  if (!TRACK_HEIGHTS[name]) return false;
  S.trackHeight = name;
  const h = TRACK_HEIGHTS[name], st = document.documentElement.style;
  st.setProperty("--spike-h", `${h.spike}px`); st.setProperty("--wave-h", `${h.wave}px`);
  st.setProperty("--tart-size", name === "large" ? "65px" : "54px");   // #9: the header drawing scales with the height
  if (Object.keys(S.rowH).length) setRowHeights(S.rowH);   // #167: the factors follow the new global height (the zoom scales every lane)
  $("#lanesScroll").dataset.height = name;
  if (save !== false && B.appSettingsSet) B.appSettingsSet("track_height", name).catch(e => setMsg(`settings: ${errText(e)}`));
  DSShell.sync();
  return true;
}
function chooseTrackHeight(name) { if (setTrackHeight(name)) setMsg(`track height: ${name}`); }   // View > Track Height > Small, Medium, Large
function stepTrackHeight(dir) {   // Taller (+1) and Shorter (-1)
  const i = HEIGHT_ORDER.indexOf(S.trackHeight) + dir;
  if (i < 0 || i >= HEIGHT_ORDER.length) { setMsg(`track height: ${S.trackHeight} is the ${dir > 0 ? "largest" : "smallest"}`); return; }
  chooseTrackHeight(HEIGHT_ORDER[i]);
}
function syncInspector(force) {   // after a change of the selected track (updateStatus) or new rows (buildTrackDOM)
  const name = S.selTrack && S.tracks[S.selTrack] ? S.selTrack : null;
  if (!force && name === INS.name) return;
  INS.name = name;
  $("#insEmpty").classList.toggle("hidden", !!name);
  $("#insBody").classList.toggle("hidden", !name);
  if (!name) { insMixHome(); $("#insParams").textContent = ""; return; }
  const t = S.tracks[name];
  const nm = $("#insName"); nm.value = name; nm.dataset.name = name;
  $("#insColor").style.background = t.color || "#888";
  insRoleLine(); insCounts(); insSource(); insRoleControls();
  t.params = t.params || {};
  for (const [k, v] of Object.entries(defaultParams(t.role))) if (t.params[k] === undefined) t.params[k] = v;   // a track that holds only a shift (track_shift_set) has no floor or ceiling
  insMixHome();   // the mix box leaves the panel before the panel is rebuilt
  buildParamsPanel($("#insParams"), name, t.params);
  $("#insParams .insMixSlot").appendChild($("#insMixBox"));
  $("#insParams .bReanalyze").disabled = analysisRunning;
  insFlags();
  insSecApply();   // P4-25: open state per section, summaries
  for (const el of document.querySelectorAll("#inspector .mslider")) el._show();
}
function insRoleLine() { const t = S.tracks[INS.name]; if (t) $("#insRole").textContent = roleLineText(t); }   // P3-18: "· follows k in" when the track follows
function insCounts() {
  const evs = S.events[INS.name] || [], man = evs.filter(isManual).length;
  $("#insDetected").textContent = String(evs.length - man);
  $("#insEdited").textContent = String(man);
  insFlags(evs);
  insSumRefresh();
}
/* P4-24: what the automatic rules did to this track: bleed hits out of the output (an edited or restored one is back in);
   P4-13 stage B: flam hits added; P4-10 stage 5: quiet kick hits added. Only the non-zero parts;
   nothing when all are zero. */
function insFlags(evs) {
  const row = document.querySelector("#inspector .insFlagRow"); if (!row) return;
  evs = evs || S.events[INS.name] || [];
  const out = evs.filter(e => e.bleed && !isManual(e) && !e.edited).length,
    flams = evs.filter(e => e.flam && !e.cut && !e.filtered).length,   // only flam hits in the output
    kicks = evs.filter(e => e.rescue && !e.cut && !e.filtered).length,   // only rescue hits in the output
    quietRole = S.tracks[INS.name] && S.tracks[INS.name].role === "snare" ? "Snare" : "Kick";   // P4-10 stage 7
  const parts = [];
  if (out) parts.push(`Bleed rules: ${out} ${out === 1 ? "hit" : "hits"} out of the output`);
  if (flams) parts.push(`Flams: ${flams} ${flams === 1 ? "hit" : "hits"} added`);
  if (kicks) parts.push(`${quietRole}: ${kicks} quiet ${kicks === 1 ? "hit" : "hits"} added`);
  const el = row.querySelector(".insFlags");
  el.textContent = parts.join(" · ");
  el.title = "Bleed hits are grey in the lane. They leave MIDI, spikes and exports. A hand velocity edit brings one back: select the hit and set the velocity in the info line. Flam hits are the second strokes of flams on a snare track. Quiet kick and snare hits are hits the onset detector found on a kick or snare track.";
  row.classList.toggle("hidden", !parts.length);
}
function insSource() {   // the state now; the file after one round trip (the public track record carries no path)
  const name = INS.name, t = S.tracks[name], st = $("#insState"), f = $("#insFile");
  st.textContent = t.offline ? `offline (${t.status})` : (t.source_mode || "copied");
  st.classList.toggle("offline", !!t.offline); $("#insRelink").classList.toggle("hidden", !t.offline); insSumRefresh();
  f.textContent = "…"; f.title = "";
  B.projectGet(S.sid).then(p => {
    if (INS.name !== name) return;
    const src = p && p.tracks && p.tracks[name] ? p.tracks[name].source : null, path = src ? (src.path || src.relative || "") : "";
    f.textContent = path ? path.replace(/^.*\//, "") : "(no file)"; f.title = path; insSumRefresh();
  }).catch(() => { if (INS.name === name) f.textContent = "?"; });
}
/* ---------------- 0.9.1: the role of a track (Inspector Track section) ----------------
   The select sets the role through track_set_role: the service records the role and its source
   (user, guess, audio), re-analyzes the track (a stored re-pick at the stored params) and
   answers the previous state (role, note, hits) as the undo state. One undo step; the note offer
   takes the new role's default note when the track played on the old role's default. */
const INS_ROLES = ["kick", "snare", "tom", "hat", "cymbal", "other"];
function insRoleSourceText(t) {   // where the role comes from; absent: an older project (the role came from the name)
  const s = t.role_source, m = t.role_match;
  if (s === "user") return "set by you";
  if (s === "audio") return "suggested from the audio";
  if (s === "preset") return m ? `from kit preset (${m} = ${t.role})` : "from kit preset";
  if (s === "scheme") return m ? `from the name (${m} = ${t.role})` : "from the name";
  if (s === "none" || s === "guess") return "not recognised from the name: choose the role";
  return t.role === "other" ? "not recognised from the name: choose the role" : "from the name";
}
function insRoleControls(offer) {   // the role select, the source hint and the note offer of the selected track
  const t = S.tracks[INS.name], sel = $("#insRoleSel");
  if (!t || !sel) return;
  if (!sel.options.length) for (const r of INS_ROLES) { const o = document.createElement("option"); o.value = r; o.textContent = r; sel.appendChild(o); }
  sel.value = INS_ROLES.includes(t.role) ? t.role : "other";
  $("#insRoleSrcRow").classList.remove("hidden");
  const srcTxt = insRoleSourceText(t);
  $("#insRoleSrc").textContent = srcTxt;
  $("#insRoleSrc").classList.toggle("lyWarn", srcTxt.startsWith("not recognised"));
  const orow = $("#insNoteOfferRow");
  INS.offer = offer && offer.suggest_note != null && offer.note !== offer.suggest_note
    ? { name: INS.name, role: offer.role, note: offer.note, suggest_note: offer.suggest_note, role_source: offer.role_source } : null;
  if (INS.offer) {
    $("#insNoteOffer").textContent = `${INS.offer.role}: default note ${nameFromNote(INS.offer.suggest_note)} (${INS.offer.suggest_note})`;
    orow.classList.remove("hidden");
  } else orow.classList.add("hidden");
}
async function setTrackRole(name, role, apply = {}) {   // apply: {note?, source?, events?, op?}; true when the service did it
  if (!S.tracks[name] || !INS_ROLES.includes(role)) return false;
  if (analysisRunning) { setMsg("analysis running: change the role after it"); return false; }
  const sid = S.sid;
  try { await trFlush(!apply.op); }
  catch (e) { setMsg(`role change failed: ${errText(e)}`, "error"); return false; }   // #249 R02: the role change must not run while its edits are refused
  let r;
  try { r = await B.trackSetRole(sid, name, role, apply.note ?? null, apply.source ?? null, apply.role_match ?? null, apply.events ?? null); }
  catch (e) { setMsg(`role change failed: ${errText(e)}`, "error"); return false; }
  if (sid !== S.sid) return false;
  S.tracks[name].role = role;
  S.tracks[name].note = r.note;
  S.tracks[name].role_source = r.role_source;
  S.tracks[name].role_match = apply.source === "user" || apply.source === "audio" ? null : (apply.role_match ?? S.tracks[name].role_match ?? null);
  await trReload(!!apply.op, name);   // the lanes, histograms and the panel follow the new role
  refreshHeadRole(name);
  if (!apply.op) pushUndo({ type: "role", sid, track: name, before: r.previous, after: { role, note: r.note, role_source: r.role_source, role_match: S.tracks[name].role_match } });
  insRoleControls(r);
  setMsg(apply.op ? `${name} is a ${role} track again` : `${name} is now a ${role} track${r.reanalyzed ? " (the hits are re-picked)" : ""}`);
  return true;
}
async function roleApplyOp(op, reverse) {   // the undo and the redo of a role change
  if (op.sid !== S.sid) { setMsg("that project is not the open one"); return false; }
  const st = reverse ? op.before : op.after;
  return setTrackRole(op.track, st.role, { note: st.note, source: st.role_source, role_match: st.role_match, events: reverse ? op.before.events : null, op: true });
}
/* The name field: the service renames the record, its place in the order and the Analysis/ folder (track_rename). Every
   key of the page (events, peaks, meters, the engine's tiles) carries the name, so the project is loaded again; the view,
   the playhead and the selection stay. */
async function renameTrack(name, to) {
  to = String(to || "").trim();
  if (!to || to === name || !S.tracks[name]) return false;
  if (S.tracks[to]) { setMsg(`a track named "${to}" exists`); return false; }
  if (analysisRunning) { setMsg("analysis running: rename the track after it"); return false; }
  try { await flushEvents(); await B.renameTrack(S.sid, name, to); }
  catch (e) { setMsg(`rename failed: ${errText(e)}`, "error"); return false; }
  if (S.playing) pause();   // P4-1b fix round: loadProject pauses; the playhead to keep is the one of the pause, not one read before it
  const view = { ...S.view }, ph = S.playhead;
  await loadProject();
  S.view = view; S.playhead = ph; S.selTrack = to;
  document.querySelectorAll(".track").forEach(r => r.classList.toggle("selTrack", r.dataset.name === to));
  invalidate(F.VIEW); updateStatus();
  setMsg(`renamed ${name} to ${to}`);
  return true;
}
/* ---------------- P4-39a: Track > Remove Track ----------------
   The service removes the track (record, hits, Learn examples, layer, Analysis/<name>/; a follower becomes a normal track) and
   answers a snapshot of the record. One undo step {type: "trkdel"} keeps that snapshot and the page's mute and solo of the
   track: undo puts the track back through tracks_restore, redo removes it again. Every key of the page carries the track name,
   so the project is loaded again (the view and the playhead stay). The first removal ends with the one step on the stacks
   (loadProject empties them); undo and redo keep the stacks they are working on. */
function trModalOpen() { return !!document.querySelector(".modal:not(.hidden):not(.sheet)"); }
async function trReload(keepStacks, pick) {
  if (S.playing) pause();   // loadProject pauses; the playhead to keep is the one of the pause
  const view = { ...S.view }, ph = S.playhead, undo = S.undo, redo = S.redo;
  await loadProject();
  if (keepStacks) { S.undo = undo; S.redo = redo; }
  S.view = view; S.playhead = Math.min(ph, maxDur());
  setSelTrack(pick && S.tracks[pick] ? pick : null);
  invalidate(F.VIEW); DSShell.sync(); updateStatus();
}
async function trFlush(inQueue) {   // the hit edits on their way, then the page's own writes; a step of the SH queue must not wait for the queue (edIdle)
  if (!inQueue) await edIdle();
  await Promise.all(pendingWriteTracks().map(pushEvents));
}
async function trRemove(name, op) {   // true when the service removed the track; op: the step that a redo runs again (it runs in the SH queue)
  const sid = S.sid;
  try { await trFlush(!!op); }
  catch (e) { setMsg(`remove failed: ${errText(e)}`, "error"); return false; }   // #249 R02: the removal must not run while its edits are refused
  let r;
  try { r = await B.tracksRemove(sid, [name]); }
  catch (e) { setMsg(`remove failed: ${errText(e)}`, "error"); return false; }
  if (sid !== S.sid) return false;
  const rest = S.order.filter(n => n !== name), idx = S.order.indexOf(name);
  const step = op || { type: "trkdel", sid, name, snapshot: null, muted: !!S.muted[name], solo: !!S.solo[name] };
  step.snapshot = r.snapshot; step.muted = !!S.muted[name]; step.solo = !!S.solo[name];
  delete S.muted[name]; delete S.solo[name]; dropQueuedWrite(name);
  await trReload(!!op, rest[idx] || rest[idx - 1] || null);   // the next track in order, else the previous one
  if (!op) pushUndo(step);
  setMsg(op ? `${name} removed` : `${name} removed (undo: Cmd+Z)`);
  return true;
}
async function removeTrack(name) {   // Track > Remove Track, the Delete key: the confirm first
  if (!needProject() || !S.tracks[name]) return false;
  if (trModalOpen()) return false;
  if (analysisRunning) { setMsg("analysis running: remove the track after it"); return false; }
  const n = (S.events[name] || []).length, fol = S.order.filter(f => S.tracks[f] && S.tracks[f].follows === name);
  const a = await askConfirm({ title: `Remove track ${name} and its ${n} hit${n === 1 ? "" : "s"}?`,
    msg: "The audio file stays where it is. Undo brings the track back with its hits." + (fol.length ? ` ${fol.join(", ")} will detect on its own.` : ""),
    buttons: [{ label: "Cancel", act: "cancel" }, { label: "Remove", act: "remove", cls: "primary" }] });
  if (a !== "remove" || !S.tracks[name]) { setMsg("remove track cancelled"); return false; }
  return trRemove(name, null);
}
function removeSelectedTrack() { const n = needTrack(); return n ? removeTrack(n) : false; }
async function trApplyOp(op, reverse) {   // the undo and the redo of a removal; true when the service did it
  if (op.sid !== S.sid) { setMsg("that project is not the open one"); return false; }
  if (!reverse) return trRemove(op.name, op);
  try { await trFlush(true); }
  catch (e) { setMsg(`undo failed: ${errText(e)}`, "error"); return false; }   // #249 R02: the restore must not run while its edits are refused
  try { await B.tracksRestore(op.sid, op.snapshot); }
  catch (e) { setMsg(`undo failed: ${errText(e)}`, "error"); return false; }
  if (op.muted) S.muted[op.name] = true;
  if (op.solo) S.solo[op.name] = true;
  await trReload(true, op.name);
  for (const n of [op.name]) { const row = rowOf(n); if (row) { row.classList.toggle("muted", !!S.muted[n]); row.classList.toggle("solo", !!S.solo[n]); row.querySelector(".bMute").classList.toggle("on", !!S.muted[n]); row.querySelector(".bSolo").classList.toggle("on", !!S.solo[n]); } }
  if (op.muted || op.solo) applyMixer();
  setMsg(`${op.name} restored`);
  return true;
}
/* ---------------- P4-25: collapsible sections and the width of the Inspector ----------------
   The open state is kept per section (not per track) and the width in localStorage (keys dsg_ins_sec, dsg_ins_w), each read
   and written inside try/catch: without storage the defaults stand. A closed header shows a short muted summary. */
const INS_SEC_ORDER = ["track", "hits", "detection", "velocity", "timing", "learn", "output", "layer"];
const INS_SEC_DEFAULT = { track: false, hits: true, detection: true, velocity: true, timing: false, learn: false, output: false, layer: false };
const INS_W = { def: 320, min: 260, max: 480 };
const INSSEC = { open: { ...INS_SEC_DEFAULT } };
function insStoreGet(key) { try { return JSON.parse(localStorage.getItem(key)); } catch (e) { return null; } }
function insStoreSet(key, v) { try { localStorage.setItem(key, JSON.stringify(v)); } catch (e) { /* no storage: the state lasts until the next start */ } }
function insSecLoad() {
  const o = insStoreGet("dsg_ins_sec");
  if (o && typeof o === "object") for (const k of INS_SEC_ORDER) if (typeof o[k] === "boolean") INSSEC.open[k] = o[k];
}
function insMixHome() { const m = $("#insMixBox"); if (m && m.parentNode !== $("#insBody")) $("#insBody").appendChild(m); }
function insSums() {
  const n = INS.name, t = S.tracks[n]; if (!t) return {};
  const p = t.params || {}, evs = S.events[n] || [], el = $("#insFile");
  const out = evs.filter(e => hgIsCut(e)).length, st = $("#insState");
  const c = lnCounts(lnLearn(lnTarget(n))), sh = Math.round(p.shift_samples || 0);
  const num = v => Number.isFinite(+v) ? String(Math.round(+v * 10) / 10) : "?";
  const sens = Number.isFinite(p.sensitivity) ? Math.round(p.sensitivity) : Math.round(sensOf(Array.isArray(p.thresholds) && p.thresholds.length === 5 ? p.thresholds : SENS.BASE));
  const f = el && el.textContent && el.textContent !== "…" ? el.textContent : "";
  return {
    track: [f, st ? st.textContent : ""].filter(Boolean).join(" · "),
    hits: `${evs.length}${out ? ` · ${out} out` : ""}`,
    detection: t.follows ? `follows ${t.follows}` : `sens ${sens} · gap ${num(p.min_gap_ms)} ms`,
    velocity: `${num(p.vel_lo_db)} to ${num(p.vel_hi_db)} dB${Number.isFinite(p.vel_cut_db) ? ` · cut ${num(p.vel_cut_db)}` : ""}`,
    timing: sh ? `shift ${sh > 0 ? "+" : ""}${sh} samples` : "no shift",
    learn: c.this || c.not ? `${c.this} this · ${c.not} not` : "no marks",
    output: `note ${nameFromNote(t.note)} (${t.note})`,
    layer: t.layer ? `${String((t.layer.source && (t.layer.source.path || t.layer.source.relative)) || "").split("/").pop()}${t.layer.on === false ? " · off" : ""}${t.layer.level_db ? ` · ${num(t.layer.level_db)} dB` : ""}` : "none",   // P4-7a
  };
}
function insSumRefresh() {
  if (!INS.name) return;
  const sums = insSums();
  for (const sec of document.querySelectorAll("#inspector .insSec")) {
    const el = sec.querySelector(".insSum"); if (el) el.textContent = sums[sec.dataset.sec] || "";
  }
}
function insSecApply() {   // after a rebuild or a change of the state: aria-expanded, the class, the summaries
  for (const sec of document.querySelectorAll("#inspector .insSec")) {
    const on = !!INSSEC.open[sec.dataset.sec];
    sec.classList.toggle("open", on);
    sec.querySelector(".insSecHead").setAttribute("aria-expanded", on ? "true" : "false");
  }
  insSumRefresh();
}
function insSecToggle(id, all) {
  const on = !INSSEC.open[id];
  if (all) for (const k of INS_SEC_ORDER) INSSEC.open[k] = on; else INSSEC.open[id] = on;
  insStoreSet("dsg_ins_sec", INSSEC.open);
  insSecApply();
  if (on) { hgDraw(); if (INS.name && INSSEC.open.learn) lnHistDraw(INS.name); for (const el of document.querySelectorAll("#inspector .mslider")) el._show(); }   // a canvas or slider in a closed section had no size
}
function insWidthSet(w, save) {
  w = Math.max(INS_W.min, Math.min(INS_W.max, Math.round(+w) || INS_W.def));
  $("#inspector").style.width = w + "px";
  if (save) insStoreSet("dsg_ins_w", w);
  return w;
}
function setupInspectorLayout() {   // once, at boot
  insSecLoad();
  const ins = $("#inspector");
  ins.addEventListener("click", ev => {
    const h = ev.target.closest(".insSecHead"); if (!h || !ins.contains(h)) return;
    insSecToggle(h.parentNode.dataset.sec, ev.altKey);
  });
  ins.addEventListener("keydown", ev => { if ((ev.key === "Enter" || ev.key === " ") && ev.target.closest(".insSecHead")) ev.stopPropagation(); });   // the button keeps Enter and Space (not play)
  ins.addEventListener("input", insSumRefresh);
  ins.addEventListener("change", () => setTimeout(insSumRefresh, 0));
  const w0 = insStoreGet("dsg_ins_w");
  insWidthSet(Number.isFinite(w0) ? w0 : INS_W.def, false);
  const hd = $("#insHandle");
  let drag = null;
  hd.addEventListener("pointerdown", ev => { drag = { x: ev.clientX, w: ins.getBoundingClientRect().width }; hd.setPointerCapture(ev.pointerId); hd.classList.add("drag"); ev.preventDefault(); });
  hd.addEventListener("pointermove", ev => { if (drag) insWidthSet(drag.w + ev.clientX - drag.x, false); });
  const end = ev => { if (!drag) return; drag = null; hd.classList.remove("drag"); insWidthSet(ins.getBoundingClientRect().width, true); };
  hd.addEventListener("pointerup", end); hd.addEventListener("pointercancel", end);
  hd.addEventListener("dblclick", () => insWidthSet(INS_W.def, true));
}
function setupInspector() {   // once, at boot: the name field, the Relink button and the mix controls
  setupInspectorLayout();
  const nm = $("#insName");
  nm.addEventListener("keydown", ev => {
    ev.stopPropagation();   // the page's shortcuts must not run while the user types
    if (ev.key === "Enter") { ev.preventDefault(); nm.blur(); }
    else if (ev.key === "Escape") { ev.preventDefault(); nm.value = nm.dataset.name || ""; nm.blur(); }
  });
  nm.addEventListener("blur", async () => {
    const from = nm.dataset.name;
    if (from && nm.value.trim() !== from && !(await renameTrack(from, nm.value)) && INS.name === from) nm.value = from;
  });
  $("#insRelink").onclick = () => { if (INS.name) relinkTrack(INS.name); };
  $("#insRoleSel").onchange = ev => {   // 0.9.1: the role of the selected track
    const name = INS.name, role = ev.target.value;
    if (!name || S.tracks[name] && S.tracks[name].role === role) return;
    setTrackRole(name, role).then(ok => { if (!ok && INS.name === name) ev.target.value = S.tracks[name] ? S.tracks[name].role : ev.target.value; });
  };
  $("#insNoteOfferBtn").onclick = () => {   // 0.9.1: the note offer of a role change
    const o = INS.offer; if (!o) return;
    setTrackRole(o.name, o.role, { note: o.suggest_note, source: o.role_source });
  };
  $("#inspectorBtn").onclick = () => toggleInspector();   // P4-25: the toolbar button, the same as View > Show Inspector
  $("#insClose").onclick = () => toggleInspector();   // the × in the title bar: the same as View > Show Inspector (the check item follows)
  bindMixSlider($("#inspector .mslider.vol"), { label: "Volume", get: () => trackVol(INS.name),
    set: v => { if (INS.name) setTrackMix(INS.name, v, null); }, reset: () => { if (INS.name) setTrackMix(INS.name, 0, null); },
    toPos: volPos, fromPos: posVol, fmt: fmtDb });
  bindMixSlider($("#inspector .mslider.pan"), { get label() { return (S.tracks[INS.name] || {}).play_channels === 2 ? "Balance" : "Pan"; },
    get: () => trackPan(INS.name), set: v => { if (INS.name) setTrackMix(INS.name, null, v); }, reset: () => { if (INS.name) setTrackMix(INS.name, null, 0); },
    toPos: p => (p + 1) / 2, fromPos: x => x * 2 - 1, fmt: fmtPan });
}

/* ---------------- progress indicator ---------------- */
function flashBtn(el) {   // brief highlight on the button a keyboard shortcut triggered
  if (!el) return;
  el.classList.remove("flash");
  void el.offsetWidth;          // restart the animation on rapid presses
  el.classList.add("flash");
  clearTimeout(el._flashT);
  el._flashT = setTimeout(() => el.classList.remove("flash"), 500);
}
function showProgress(pct, text, cancelable) {   // pct 0..100, or -1 = indeterminate; cancelable: the button stops the running job (P3-4d)
  const w = $("#progressWrap"); w.classList.remove("hidden");
  w.classList.toggle("cancelable", !!cancelable);
  const cb = $("#progressCancel"); cb.classList.toggle("hidden", !cancelable);
  if (!cancelable) cb.disabled = false;
  const bar = $("#progressBar");
  // 0% determinate is invisible — animate until there is real progress to show
  bar.classList.toggle("indeterminate", pct <= 0);
  if (pct > 0) bar.style.width = pct + "%";
  NOTE.base = text || "";
  $("#progressText").textContent = NOTE.base + (NOTE.until > Date.now() ? ` · ${NOTE.text}` : "");
}
const NOTE = { base: "", text: "", until: 0 };   // a refusal while a job runs: the status message is hidden then, so the progress text carries it for a few seconds
function jobNote(text) {
  setMsg(text);
  NOTE.text = text; NOTE.until = Date.now() + 5000;
  if (!$("#progressWrap").classList.contains("hidden")) $("#progressText").textContent = `${NOTE.base} · ${text}`;
}
function hideProgress() { $("#progressWrap").classList.add("hidden"); $("#progressWrap").classList.remove("cancelable"); $("#progressCancel").classList.add("hidden"); }
/* P3-4d: the progress message of a copy job {state, done, total, bytes_done, bytes_total, file} on the bar, with Cancel */
function copyProgress(label) {
  return p => {
    const pct = p.bytes_total ? (100 * p.bytes_done) / p.bytes_total : (100 * p.done) / Math.max(1, p.total);
    const n = Math.min(p.done + (p.state === "running" ? 1 : 0), p.total);
    showProgress(Math.max(1, Math.round(pct)), `${label} ${n} of ${p.total}${p.file ? " - " + p.file : ""}`, true);
  };
}
function setAnalysisButtons(disabled) {
  $("#analyzeAllBtn").disabled = disabled;
  $("#analyzeAllBtn").classList.toggle("running", disabled);   // 0.9.3: the accent colour while an analysis runs
  document.querySelectorAll(".bReanalyze").forEach(b => b.disabled = disabled);
  const rm = document.querySelector("#ctxMenu [data-act=reanalyze]"); if (rm) rm.disabled = disabled;
}

/* ---------------- analysis ---------------- */
let pollTimer = null;
let analysisRunning = false;
let pendingKeep = false;   // B4, P4-9: the analysis keeps the edited hits (the service merges them); only for the message
function startAnalysisGo(tracks) {
  if (!S.order.length) { setMsg("no tracks"); return; }
  if (analysisRunning) { setMsg("analysis already running — wait for it to finish"); return; }
  const list = tracks || S.order;
  const withEvents = list.filter(n => (S.events[n]||[]).length > 0);
  if (withEvents.length) {
    const keptTotal = withEvents.reduce((a,n) => a + (S.events[n]||[]).filter(isKept).length, 0);
    if (keptTotal === 0) {
      if (!confirm(`Re-analyzing ${withEvents.length} track(s) overwrites their events (a backup is kept). Continue?`)) return;
      runAnalysis(list, false);
    } else {
      $("#reanalyzeMsg").innerHTML = "";
      const p = $("#reanalyzeMsg");
      p.appendChild(document.createTextNode(
        `Re-analyzing ${withEvents.length} track(s): ` +
        withEvents.map(n => `${n} (${(S.events[n]||[]).filter(isKept).length} edited)`).join(", ") + "."));
      $("#reanalyzeModal").classList.remove("hidden");
      pendingRA = list;   // runAnalysis is called by the modal buttons
    }
  } else runAnalysis(list, false);
}
let pendingRA = null;
function runAnalysis(list, keepManual) {
  $("#reanalyzeModal").classList.add("hidden");
  pendingKeep = !!keepManual && list.some(n => (S.events[n]||[]).some(isKept));
  const params = {};
  for (const n of list) params[n] = S.tracks[n].params || defaultParams(S.tracks[n].role);
  setMsg("analysis queued…");
  analysisRunning = true; setAnalysisButtons(true);
  showProgress(0, "queued…");
  flushEvents().then(() => B.analyze(S.sid, list, params, !!keepManual))   // P4-9: the service holds the edits, keep_edited merges them
    .catch(e => { throw e.status ? new Error(`analyze failed: ${e.message}`) : e; })   // C3; P4-3 round 2: the service's text (409: a re-pick is being stored)
    .then(() => pollJob())
    .catch(e => { analysisRunning = false; setAnalysisButtons(false); hideProgress(); setMsg(/^analyze failed/.test(e.message || "") ? e.message : `analyze failed: ${errText(e)}`, "error"); });   // #249 R02: a refused write is named like the rest
}
function pollJob() {
  clearTimeout(pollTimer);
  B.job(S.sid).then(st => {
    if (st.state === "running" || st.state === "queued") {
      const names = Object.keys(st.tracks || {});
      const done = names.filter(n => (st.tracks[n].state||"") === "done").length;
      const failed = names.filter(n => (st.tracks[n].state||"") === "error").length;
      const total = Math.max(1, names.length);
      const cur = names.find(n => (st.tracks[n].state||"") === "running");
      const stage = cur ? (st.tracks[cur].stage || "analyzing") : st.state;
      showProgress(Math.round(100*(done+failed)/total),
        `Analyzing ${done+failed}/${total}` + (cur ? ` — ${cur}: ${stage}` : "") + (failed ? ` (${failed} failed)` : ""));
      pollTimer = setTimeout(pollJob, 800);
      return;
    }
    analysisRunning = false; setAnalysisButtons(false);
    if (st.state === "done") {
      showDetector(st.tracks, st.bleed);
      const bad = failedTracks(st.tracks), total = Object.keys(st.tracks || {}).length;
      const kept = pendingKeep; pendingKeep = false;
      showProgress(100, bad.length ? `analysis done: ${bad.length} of ${total} failed` : "analysis done");
      setTimeout(hideProgress, bad.length ? 8000 : 1500);
      loadEventsOnly().then(() => refreshTrackMeta()).then(() => {   // P3-18: the params the retry changed, the follows
        if (kept) setMsgBg("analysis done — edited hits kept");
        markTrackErrors(st.tracks);
        const weak = weakSignals(st.tracks);   // P3-18: "weak signal: sensitivity raised to ..." or "no hits found"
        if (bad.length) { setMsgBg(`analysis: ${bad.length} of ${total} tracks failed (${bad[0][0]}: ${bad[0][1]})`, "error"); setMsgDetail(bad[0][2]); }
        else if (weak.length) { setMsgBg(weak.map(w => w[1]).join("; ")); setMsgDetail(weak.map(w => w[1]).join("\n")); }
      });
    } else hideProgress();   // idle: nothing to show
  }).catch(()=>{ pollTimer = setTimeout(pollJob, 1500); });
}

/* P2-7: the detector that ran (job status: tracks[name].detector = "rust-dsp", .chain, .note)
   in the status line; #294 made the DSP detector the only one, so
   it gets no warning. A note gets a visible warning. A track that failed shows its error in red
   (status line and track header), also when every track failed. */
function failedTracks(tracks) {   // [[name, error message], ...]
  return Object.entries(tracks || {}).filter(([, t]) => t.state === "error").map(([n, t]) => [n, t.error || "failed", t.error_detail || ""]);
}
function detectorLabel(id) {   // #411: the status bar names the built-in detector like the Inspector does ("Built-in"); a plug-in keeps its id
  return id === "rust-dsp" ? "Built-in" : id;
}
function showDetector(tracks, bleed) {
  const el = $("#stDet");
  const runs = Object.values(tracks || {}).filter(t => t.state === "done" && t.detector);
  const bad = failedTracks(tracks);
  const parts = [];
  if (runs.length) {
    const names = [...new Set(runs.map(t => detectorLabel(t.detector) + (t.chain ? ` (${t.chain})` : "")))];
    parts.push(`detector: ${names.join(", ")}`);
  }
  const notes = [...new Set(runs.map(t => t.note).filter(Boolean))];
  parts.push(...notes);
  if (bad.length) parts.push(`${bad.length} of ${Object.keys(tracks).length} tracks failed: ${bad[0][1]}`);
  if (bleed && bleed.rule) {   // #411: one hit reads "1 hit"
    const nb = Object.values(bleed.tracks || {}).reduce((a, b) => a + b, 0);
    parts.push(bleed.on ? `tom bleed: ${nb} ${nb === 1 ? "hit" : "hits"} out of the output`
                        : `tom bleed rule off: ${bleed.reason}`);   // P4-14: the analysis says why
  }
  if (!parts.length) return;
  el.textContent = parts.join(" · ");
  el.classList.toggle("warn", notes.length > 0);
  el.classList.toggle("err", bad.length > 0);
  el.title = bad.length ? bad.map(([n, m, d]) => `${n}: ${m}` + (d ? `\n  (${d})` : "")).join("\n") : notes.join("\n");
}
function markTrackErrors(tracks) {   // a red mark and the message in the header of each failed track
  document.querySelectorAll(".track").forEach(row => {
    const t = (tracks || {})[row.dataset.name];
    const old = row.querySelector(".terrmark"); if (old) old.remove();
    row.classList.toggle("terr", !!t && t.state === "error");
    if (t && t.state === "error") {
      const m = document.createElement("div"); m.className = "terrmark";
      m.textContent = `analysis failed: ${t.error || "failed"}`; m.title = m.textContent + (t.error_detail ? `\n(${t.error_detail})` : "");
      row.querySelector(".thead").appendChild(m);
    }
  });
}
/* P2 fix M1: a detector backend that cannot load shows its error at once (B.backendInfo), not only after a run. */
async function showBackend() {
  try {
    const b = await B.backendInfo();
    const el = $("#stDet");
    if (b.note) {
      el.textContent = b.note; el.classList.add("warn"); el.classList.remove("err"); el.title = "";
    }
  } catch (e) { /* the status line stays empty */ }
}

/* ---------------- export: the messages (the dialog is the block "P4-30: the Export dialog" below) ---------------- */
/* P4-4: hits with no sample in the audio file (before its start or after its end) are not in the spike file; the
   MIDI file keeps them. The answers count them, and the message says so (they were skipped without a word). */
function outsideNote(n, what) { return n ? `; ${n} hit${n === 1 ? "" : "s"} outside the audio file: ${what}` : ""; }
function midiOutsideNote(n, before) {   // the MIDI file keeps a hit after the end; one before the start sits at 0 s
  const b = before || 0, a = (n || 0) - b, s = k => (k === 1 ? "" : "s");
  return (a > 0 ? `; ${a} hit${s(a)} after the end of the audio file: the MIDI file keeps them` : "") +
    (b > 0 ? `; ${b} hit${s(b)} before its start: at 0 s in the MIDI file` : "");
}
function exportedFollowers(r) { if (r && Array.isArray(r.followers) && r.followers.length) reloadFollowers(r.followers); }   // P4-4: a stale follower was derived again

/* ---------------- P3-3c: spike files, Export all (app only), error detail ---------------- */
function setMsgDetail(detail) { $("#stMsg").title = detail || ""; }   // the core's text behind a clear message
function markLaneDetail(name, detail) {   // tooltip of a lane that cannot draw
  const c = document.querySelector(`.track[data-name="${name}"] canvas.wave`);
  if (c) c.title = detail || "";
}
/* ---------------- P4-30: the Export dialog ----------------
   One Export entry (File > Export…, Cmd+E, the toolbar arrow; Track > Export Selected Tracks… and the track header menu
   open it with the selected track). The user picks the tracks (all, or the selected one), the formats (MIDI per track
   or one multi-track file, spike WAV) and the destination folder (Choose… opens the native folder dialog; the choice
   is kept in the project, `export_dest`; Reset goes back to Exports/). The dialog says how many files the export
   replaces (export_existing), Enter exports, Escape cancels and no page key runs while it is open. A track that is
   offline or has no hits is listed as skipped. The drag tiles hold the files of the current options, pre-rendered into
   the hidden Exports/.drag folder (#433; export_drag_prepare, 400 ms after the last change); a drag of a tile moves its
   files into Exports/ under the export names (export_drag_take) and starts the native file drag (B.dragFiles) with them.
   Cancel, close and a new option change remove the pre-rendered files; Exports/ gets a file only by a drag or an export. */
const EX = { open: false, scope: "all", selected: null, midi: true, spikes: true, spikeType: "synthetic", mode: "per_track", comp: false, markers: false, dest: "", def: "", remembered: null,
  /* #172: the export-finished event for the e2e tests: the count of the exports this page has finished */
  destOk: true, replace: 0, busy: false, done: 0, drag: { midi: null, spikes: null }, timer: null, dragSeq: 0, dragChain: Promise.resolve(),
  infoSeq: 0, lastFile: null, ran: false, tempoSeq: 0, tempoDefault: false,
  /* #151: the default-tempo answer holds per project (loadProject resets it when a project opens) */
  tempoAck: false };
function exTrackSets() {   // the tracks the options name: the ones that export, and the skipped ones with the reason
  const pool = EX.scope === "selected" ? (EX.selected && S.tracks[EX.selected] ? [EX.selected] : []) : S.order.slice();
  const ok = n => S.tracks[n] && !S.tracks[n].offline && (S.events[n] || []).length > 0;
  return { names: pool.filter(ok), skipped: pool.filter(n => !ok(n)).map(n => ({ name: n, why: S.tracks[n] && S.tracks[n].offline ? "offline" : "no hits" })) };
}
function exFormats() {   // #218: the spike type decides which WAV formats run: synthetic = spikes, audio = audio, both = both
  const spike = !EX.spikes ? [] : EX.spikeType === "audio" ? ["audio"] : EX.spikeType === "both" ? ["spikes", "audio"] : ["spikes"];
  return [EX.midi && "midi", ...spike].filter(Boolean);
}
function exOpts(withDest) {
  const o = { tracks: exTrackSets().names, formats: exFormats(), midi_mode: EX.mode, compensate: EX.comp };
  if (withDest) o.dest = EX.dest;
  return o;
}
const exPlural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
function exRender() {   // the controls from EX; the Export button and the tiles follow
  const { names, skipped } = exTrackSets(), all = S.order.length, sel = EX.selected && S.tracks[EX.selected] ? 1 : 0;
  $("#exScopeAll").checked = EX.scope === "all"; $("#exScopeSel").checked = EX.scope === "selected";
  $("#exAllText").textContent = `All tracks (${all})`;
  $("#exSelText").textContent = sel ? `Selected track (1): ${EX.selected}` : "Selected track (none selected)";
  $("#exScopeSel").disabled = !sel; $("#exScopeSel").closest(".exRow").classList.toggle("off", !sel);
  $("#exSkipped").textContent = skipped.length ? `Skipped: ${skipped.map(k => `${k.name} (${k.why})`).join(", ")}` : "";
  $("#exMidi").checked = EX.midi; $("#exSpikes").checked = EX.spikes;
  $("#exMarkers").checked = EX.markers;
  $("#exModeTrack").checked = EX.mode === "per_track"; $("#exModeSingle").checked = EX.mode === "single";
  $("#exSpikeSynth").checked = EX.spikeType === "synthetic"; $("#exSpikeAudio").checked = EX.spikeType === "audio"; $("#exSpikeBoth").checked = EX.spikeType === "both";
  for (const id of ["#exSpikeSynth", "#exSpikeAudio", "#exSpikeBoth"]) { $(id).disabled = !EX.spikes; $(id).closest(".exRow").classList.toggle("off", !EX.spikes); }
  const cal = S.order.filter(n => smOf(n));   // P4-7c: the tracks with a sampler calibration
  $("#exComp").checked = EX.comp; $("#exComp").disabled = !EX.midi; $("#exComp").closest(".exRow").classList.toggle("off", !EX.midi);
  $("#exComp").closest(".exRow").title = cal.length ? `The MIDI notes of ${cal.join(", ")} move by the sampler offset (the aligned attack minus the offset)` : "No track has a sampler calibration yet (Inspector > Layer > Calibration MIDI…)";
  for (const id of ["#exModeTrack", "#exModeSingle"]) { $(id).disabled = !EX.midi; $(id).closest(".exRow").classList.toggle("off", !EX.midi); }
  $("#exDest").value = EX.dest; $("#exDest").title = EX.dest;
  $("#exReset").disabled = EX.dest === EX.def;
  let problem = EX.destNote;
  if (!EX.midi && !EX.spikes) problem = "Choose at least one format.";
  else if (!names.length) problem = problem || "No track to export: every track in this choice is offline or has no hits.";
  $("#exProblem").textContent = problem;
  $("#exReplace").textContent = !names.length || !exFormats().length || !EX.destOk ? "" :
    EX.replace ? `${exPlural(EX.replace, "file", "files")} will be replaced` : "No file will be replaced.";
  $("#exGo").disabled = EX.busy || !names.length || !exFormats().length || !EX.destOk;
  for (const k of ["midi", "spikes"]) {
    const t = $(k === "midi" ? "#exDragMidi" : "#exDragSpikes");
    t.classList.toggle("hidden", !EX[k]);
    if (!EX[k]) continue;
    const files = EX.drag[k];
    t.dataset.state = !names.length || !EX.midi && !EX.spikes ? "disabled" : EX.drag.failed ? "error" : files ? "ready" : "preparing";
    t.querySelector(".exDragN").textContent = t.dataset.state === "preparing" ? "preparing…" : files ? exPlural(files.length, "file", "files") : "";
  }
  /* #151: on the default map the Drag MIDI tile shows the visible 120 BPM warning until the project's answer */
  $("#exDragMidi .exDragW").classList.toggle("hidden", !(EX.midi && EX.tempoDefault && !EX.tempoAck));
}
async function exInfo() {   // the replace count of the current options (the newest answer wins)
  const my = ++EX.infoSeq;
  if (!exFormats().length || !exTrackSets().names.length) { EX.replace = 0; exRender(); return; }
  try {
    const r = await B.exportExisting(S.sid, exOpts(true));
    if (my !== EX.infoSeq || !EX.open) return;
    EX.replace = r.count; EX.destOk = true; EX.destNote = EX.destNote.startsWith("The folder") ? EX.destNote : "";
  } catch (e) {
    if (my !== EX.infoSeq || !EX.open) return;
    EX.destOk = false; EX.destNote = `The folder cannot be used: ${errText(e)}`; EX.replace = 0;
  }
  exRender();
}
function exDragSchedule(now) {   // the tiles show "preparing…" until the files of the new options are there
  clearTimeout(EX.timer);
  const my = ++EX.dragSeq;
  EX.drag = { midi: null, spikes: null, failed: false };
  exRender();
  if (!exFormats().length || !exTrackSets().names.length || !EX.destOk) return;
  EX.timer = setTimeout(() => {
    EX.dragChain = EX.dragChain.then(async () => {   // one render at a time: two would clear each other's files
      if (my !== EX.dragSeq || !EX.open) return;
      try {
        await flushEvents();
        const r = await B.exportDragPrepare(S.sid, exOpts(false));
        exportedFollowers(r);   // P4-4: the render derives stale followers like the export it previews; the page takes the new hits even when the export then finds everything fresh
        if (my !== EX.dragSeq || !EX.open) return;
        EX.drag = { midi: r.midi, spikes: r.spikes, failed: false };
      } catch (e) { if (my === EX.dragSeq) EX.drag = { midi: null, spikes: null, failed: true }; }
      if (EX.open) exRender();
    });
  }, now ? 0 : 400);
}
function exChanged() { $("#exResult").classList.add("hidden"); exRender(); exInfo(); exDragSchedule(); exTempoDefault(); }
/* #151: the default-ness of the map drives the visible 120 BPM warning of the Drag MIDI tile; the newest answer wins */
async function exTempoDefault() {
  const my = ++EX.tempoSeq, sid = S.sid;
  try { const d = !!(await B.tempoMapGet(sid)).default; if (my === EX.tempoSeq) { EX.tempoDefault = d; exRender(); } }
  catch (e) { if (my === EX.tempoSeq) EX.tempoDefault = false; }
}
async function openExport(scope, track) {
  if (!needProject()) return;
  if (EX.open) { $("#exGo").focus(); return; }
  if (EX.busy) { setMsg("the export is still running"); return; }   // P4-31: Export again owns the export; the dialog does not race it
  try { await flushEvents(); }
  catch (e) { setMsg(`export failed: ${errText(e)}`, "error"); return; }   // #249 R02: the dialog reads the service's hits; a refused write stops it
  EX.selected = track || (S.selTrack && S.tracks[S.selTrack] ? S.selTrack : null);
  EX.scope = scope === "selected" && EX.selected ? "selected" : "all";
  EX.busy = false; EX.destNote = ""; EX.destOk = true; EX.replace = 0; EX.lastFile = null; EX.ran = false;
  $("#exResult").classList.add("hidden");
  await smRefreshAll();   // P4-7c: a calibration set by an agent command counts (reads only: tracks_get would derive a stale follower)
  EX.comp = S.order.some(n => smOf(n));   // the default: on when any track has a calibration
  EX.midi = true; EX.spikes = true; EX.mode = "per_track";   // P4-31: the defaults, until the project's export shape seeds them
  let info = null;
  try { info = await B.exportExisting(S.sid, {}); } catch (e) { setMsg(`export failed: ${errText(e)}`, "error"); return; }
  EX.def = info.default_dest; EX.remembered = info.remembered;
  EX.markers = !!info.export_markers;   // P4-40g: the remembered "Markers in MIDI" checkbox
  const last = info.export_last;   // P4-31: the project keeps what its last export wrote with (and every choice, recorded at once)
  if (last) {
    EX.midi = last.formats.includes("midi"); EX.spikes = last.formats.includes("spikes") || last.formats.includes("audio");
    EX.spikeType = last.formats.includes("spikes") && last.formats.includes("audio") ? "both" : last.formats.includes("audio") ? "audio" : "synthetic";   // #218: the spike type the project keeps
    EX.mode = last.midi_mode === "single" ? "single" : "per_track";   // P4-31: the choices the project keeps seed the dialog; compensate keeps its default (a recorded off was the default of an earlier export, not a choice)
  }
  EX.dest = EX.def;
  if (EX.remembered) {   // the folder of the last choice, when it can still be used
    try { await B.exportExisting(S.sid, { dest: EX.remembered }); EX.dest = EX.remembered; }
    catch (e) { EX.destNote = `The folder kept for this project cannot be used (${errText(e)}): Exports/ is used.`; }
  }
  EX.open = true;
  $("#exportModal").classList.remove("hidden");
  document.addEventListener("keydown", exKey, true);
  exChanged();
  $("#exGo").focus();
}
function exClose() {
  if (EX.busy) return;
  EX.open = false; clearTimeout(EX.timer); EX.dragSeq++;
  $("#exportModal").classList.add("hidden");
  document.removeEventListener("keydown", exKey, true);
  const sid = S.sid;   // the files of the tiles go: they can be big (a spike file has the length of its audio)
  EX.dragChain = EX.dragChain.then(() => B.exportDragPrepare(sid, { clear: true }).catch(() => {}));
}
function exCancel() { exClose(); $("#exCancel").textContent = "Cancel"; if (!EX.ran) setMsg("export cancelled"); }   // after an export (even a failed one) the message keeps its result
function exKey(ev) {   // the dialog owns every key: no page key (Space plays, J snaps) runs while it is open
  if (ev.key === "Escape") { ev.preventDefault(); ev.stopPropagation(); exCancel(); return; }
  if (ev.key === "Enter" && ev.target.tagName !== "BUTTON") { ev.preventDefault(); ev.stopPropagation(); exRun(); return; }
  ev.stopPropagation();
}
async function exChoose() {
  const pick = await pickFolder({ title: "Choose the export folder", start: EX.dest });
  if (!pick || !pick.dir) return;
  try {
    await B.exportExisting(S.sid, { dest: pick.dir });   // absolute, a folder, writable: the service checks
    await B.exportDestSet(S.sid, pick.dir);
  } catch (e) { EX.destNote = `The folder cannot be used: ${errText(e)}`; exRender(); return; }
  EX.dest = pick.dir; EX.remembered = pick.dir; EX.destNote = ""; EX.destOk = true;
  setDirtyState(true);
  exChanged(); exRecord();   // P4-31: the folder is part of the export shape
}
async function exReset() {
  try { await B.exportDestSet(S.sid, null); } catch (e) { EX.destNote = errText(e); exRender(); return; }
  EX.dest = EX.def; EX.remembered = null; EX.destNote = ""; EX.destOk = true;
  setDirtyState(true);
  exChanged(); exRecord();   // P4-31: back to Exports/ in the record
}
function exSamplerNote(r) {   // P4-7c: which tracks' notes moved by the sampler offset, and the notes that fell before the start
  const t = Object.entries(r.sampler || {});
  if (!t.length) return "";
  return `; sampler offset applied to ${t.map(([k, v]) => `${k} (${smSigned(v.offset_ms)} ms)`).join(", ")}` + ((r.warnings || []).length ? `; ${r.warnings.join("; ")}` : "");
}
function exSummary(r) {   // "exported 2 MIDI and 2 spike files to Exports/ in the project folder (0.1 s)"; #218 counts the audio spike files
  const parts = [];
  if (r.midi_file) parts.push("1 multi-track MIDI");
  else if (r.midi) parts.push(`${r.midi} MIDI`);
  if (r.spikes) parts.push(`${r.spikes} spike`);
  if (r.audio) parts.push(`${r.audio} audio spike`);
  const last = r.audio ? r.audio : r.spikes ? r.spikes : r.midi_file ? 1 : (r.midi || 0);   // the noun agrees with the last part: "1 MIDI and 1 spike file", "2 MIDI and 2 audio spike files"
  const what = parts.length ? parts.join(" and ") + (last === 1 ? " file" : " files") : "0 files";
  const where = EX.def && r.dest === EX.def ? "Exports/ in the project folder" : r.dest;   // P4-31: export again runs with the dialog closed (EX.def may be empty)
  return `exported ${what} to ${where} (${(r.elapsed_ms / 1000).toFixed(1)} s)`;
}
/* #151: the default-tempo notice, shared by the Export dialog and File > Export Again: a MIDI export of a
   project whose tempo map is the default map would use 120 BPM. "anyway" lets the export write; "set_tempo"
   opens Track > Detect Tempo without writing; "cancel" writes nothing. A tempo_map_get that fails never asks. */
async function askDefaultTempo() {
  let def = false;
  try { def = !!(await B.tempoMapGet(S.sid)).default; } catch (e) { return "anyway"; }
  EX.tempoDefault = def;   // the same answer drives the visible warning of the Drag MIDI tile
  if (!def) return "anyway";
  return await askConfirm({ title: "No tempo set",
    msg: "The export uses the default 120 BPM. Set the tempo first: Track > Detect Tempo, File > Import Tempo Map, or the Tempo ruler. Or export anyway.",
    buttons: [{ label: "Cancel", act: "cancel" }, { label: "Set Tempo…", act: "set_tempo" }, { label: "Export anyway", act: "anyway", cls: "primary" }] });
}
async function exRun() {
  if (EX.busy || $("#exGo").disabled) return;
  EX.busy = true;   // set before the notice: while it asks, exClose (Escape) is refused
  /* #151: a MIDI export with no tempo set uses the default 120 BPM: ask before the first export writes */
  if (EX.midi && !EX.tempoAck) {
    const a = await askDefaultTempo();
    if (a !== "anyway") { EX.busy = false; setMsg("export cancelled"); if (a === "set_tempo") detectTempo(); return; }   // no file is written
    EX.tempoAck = true;   // the answer holds for this project: the next export does not ask again
  }
  const o = exOpts(true), n = o.tracks.length;
  EX.ran = true; $("#exCancel").textContent = "Close"; exRender();
  try {
    await flushEvents();   // the service writes from its own copy of the events
    showProgress(0, `Exporting 0/${n}`);
    let live = true;   // a message that arrives after the answer (the test bridge polls) must not override the result
    const r = await B.exportAll(S.sid, p => { if (live && p.state === "running") showProgress(Math.round(100 * p.done / Math.max(1, p.total)), `Exporting ${p.done}/${p.total} — ${p.track}`); }, o);
    live = false;
    const bad = r.errors || [];
    showProgress(100, bad.length ? `export done: ${bad.length} problems` : "export done");
    setTimeout(hideProgress, bad.length ? 8000 : 1500);
    const out = Object.entries(r.outside || {}).map(([k, c]) => `${k} ${c}`);   // P4-4
    const msg = exSummary(r) + (bad.length ? `; ${bad.length} failed (${bad[0].name}: ${bad[0].message})` : "") +
      (out.length ? `; hits outside the audio file, not in the spike files: ${out.join(", ")} (the MIDI files keep them; a hit before the start sits at 0 s there)` : "") +
      exSamplerNote(r);
    setMsg(msg);
    exportedFollowers(r);
    setMsgDetail(bad.map(e => `${e.name}: ${e.message}` + (e.detail ? `\n  (${e.detail})` : "")).join("\n"));
    EX.lastFile = (r.files || [])[0] || null;
    exRecord(true);   // P4-31: the project keeps what this export ran with (even with per-file errors)
    $("#exResultText").textContent = msg.replace(/^exported/, "Exported");
    $("#exResult").classList.toggle("hidden", !EX.lastFile);
    exInfo();
  } catch (e) { hideProgress(); setMsg(`export failed: ${e.message}`, "error"); setMsgDetail(e.detail); EX.destNote = errText(e); }
  finally { EX.busy = false; EX.done++; if (EX.open) exRender(); }   // #172: done is the export-finished event
}
async function exDragStart(kind) {
  const files = EX.drag[kind];
  if (!files || !files.length) return;
  /* #151: a drag of the MIDI tile on the default map writes 120 BPM MIDI: the press asks the same notice
     first; Cancel starts no drag and no file moves, Set Tempo… opens Track > Detect Tempo, and only the
     project's "Export anyway" (the per-project answer) lets the tile drag. A set map never asks. */
  if (kind === "midi" && !EX.tempoAck) {
    const a = await askDefaultTempo();
    if (a !== "anyway") { if (a === "set_tempo") detectTempo(); return; }
    EX.tempoAck = true;
    exRender();
  }
  let out;
  try { out = (await B.exportDragTake(S.sid, files)).files; }   // #433: the dragged files move into Exports/ (the export names): they stay after the drag and the close
  catch (e) { setMsg(`drag failed: ${errText(e)}`, "error"); return; }
  exDragSchedule(true);   // the tiles render again into the hidden folder: the next drag of a tile finds its files
  B.dragFiles(out).then(() => setMsg(`dragged ${exPlural(out.length, "file", "files")}: drop ${out.length === 1 ? "it" : "them"} into your DAW`))
    .catch(e => setMsg(`drag failed: ${errText(e)}`, "error"));
}
/* P4-31: the project keeps its export shape: the Export dialog records every choice at once (like the folder and
   the markers checkbox), and an export records what it ran with. The dialog shows the same choices after a reload,
   and File > Export Again (Option+Cmd+E) repeats the last export: the same files to the same place, no dialog.
   `ran`: an export just ran (the item is on from now on). */
function exRecord(ran) {
  if (!S.sid) return;
  const last = { formats: exFormats(), midi_mode: EX.mode, compensate: EX.comp, scope: EX.scope,
    track: EX.scope === "selected" ? EX.selected : null, dest: EX.dest && EX.dest !== EX.def ? EX.dest : null };
  B.exportLastSet(S.sid, last).then(r => {
    S.exportAgain = true;
    if (r.changed) setDirtyState(true);   // the record is a document field, like the folder and the markers checkbox
    if (typeof DSShell !== "undefined") DSShell.sync();   // the Export Again item follows at once
  }).catch(() => {});
}
let exAgainBusy = false;
async function exportAgain() {   // P4-31: File > Export Again: the last export of the project, no dialog
  if (!needProject() || exAgainBusy || EX.busy) return;
  let last = null;
  try { last = (await B.exportLastGet(S.sid)).export_last; } catch (e) { setMsg(`export again failed: ${errText(e)}`, "error"); return; }
  if (!last) { setMsg("nothing exported yet: Export again repeats the last export of the project"); return; }
  const o = { formats: last.formats, midi_mode: last.midi_mode === "single" ? "single" : "per_track", compensate: last.compensate !== false };
  if (last.scope === "selected" && last.track) o.tracks = [last.track];   // the track of the last export, not the selection of now
  if (last.dest) o.dest = last.dest;
  const n = (o.tracks || S.order).length;
  /* #151: Export Again asks the same notice before it writes (the dialog path sets EX.tempoAck the same way) */
  if ((o.formats || []).includes("midi") && !EX.tempoAck) {
    const a = await askDefaultTempo();
    if (a !== "anyway") { setMsg("export cancelled"); if (a === "set_tempo") detectTempo(); return; }
    EX.tempoAck = true;
  }
  exAgainBusy = true;
  try {
    await flushEvents();   // the service writes from its own copy of the events
    showProgress(0, `Exporting 0/${n}`);
    let live = true;   // a message that arrives after the answer (the test bridge polls) must not override the result
    const r = await B.exportAll(S.sid, p => { if (live && p.state === "running") showProgress(Math.round(100 * p.done / Math.max(1, p.total)), `Exporting ${p.done}/${p.total} — ${p.track}`); }, o);
    live = false;
    const bad = r.errors || [];
    showProgress(100, bad.length ? `export done: ${bad.length} problems` : "export done");
    setTimeout(hideProgress, bad.length ? 8000 : 1500);
    setMsg(exSummary(r));
    exportedFollowers(r);
  } catch (e) { hideProgress(); setMsg(`export failed: ${e.message}`, "error"); }
  finally { exAgainBusy = false; EX.done++; }   // #172: done is the export-finished event
}
function setupExport() {
  $("#exScopeAll").onchange = () => { EX.scope = "all"; exChanged(); exRecord(); };   // P4-31: every choice is recorded at once
  $("#exScopeSel").onchange = () => { EX.scope = "selected"; exChanged(); exRecord(); };
  $("#exMidi").onchange = ev => { EX.midi = ev.target.checked; exChanged(); exRecord(); };
  $("#exSpikes").onchange = ev => { EX.spikes = ev.target.checked; exChanged(); exRecord(); };
  $("#exComp").onchange = ev => { EX.comp = ev.target.checked; exChanged(); exRecord(); };
  $("#exMarkers").onchange = async ev => {   // P4-40g: the checkbox is remembered per project
    const on = ev.target.checked;
    EX.markers = on;
    try { await B.exportMarkersSet(S.sid, on); setDirtyState(true); }
    catch (e) { EX.markers = !on; setMsg(`the Markers in MIDI choice failed: ${errText(e)}`, "error"); }
    exChanged();
  };
  $("#exModeTrack").onchange = () => { EX.mode = "per_track"; exChanged(); exRecord(); };
  $("#exModeSingle").onchange = () => { EX.mode = "single"; exChanged(); exRecord(); };
  $("#exSpikeSynth").onchange = () => { EX.spikeType = "synthetic"; exChanged(); exRecord(); };   // #218: the spike type is part of the export shape
  $("#exSpikeAudio").onchange = () => { EX.spikeType = "audio"; exChanged(); exRecord(); };
  $("#exSpikeBoth").onchange = () => { EX.spikeType = "both"; exChanged(); exRecord(); };
  $("#exChoose").onclick = () => exChoose();
  $("#exReset").onclick = () => exReset();
  $("#exGo").onclick = () => exRun();
  $("#exCancel").onclick = () => exCancel();
  $("#exReveal").onclick = () => { if (EX.lastFile) B.revealPath(EX.lastFile).catch(e => setMsg(`show in Finder failed: ${errText(e)}`, "error")); };
  for (const t of document.querySelectorAll("#exportModal .exDrag")) {
    t.addEventListener("mousedown", ev => {   // a press and a move of 4 px starts the native drag; the files are there already
      if (ev.button !== 0 || t.dataset.state !== "ready") return;
      ev.preventDefault();
      const x0 = ev.clientX, y0 = ev.clientY;
      const off = () => { document.removeEventListener("mousemove", mv, true); document.removeEventListener("mouseup", off, true); };
      const mv = e => { if (Math.hypot(e.clientX - x0, e.clientY - y0) > 4) { off(); exDragStart(t.dataset.kind); } };
      document.addEventListener("mousemove", mv, true); document.addEventListener("mouseup", off, true);
    });
  }
}

/* ---------------- Add tracks: the native dialog (P3-4c) ---------------- */
function setupUpload() {
  $("#addTracksBtn").onclick = () => { if (!needProject()) return; addTracksDialog(); };
}

/* ---------------- add tracks by drop or dialog (P3-4c) ---------------- */
/* The app gets paths, not bytes. A drop on the window (shell.js, DSShell.hooks.otherPaths), File > Add Tracks and
   the toolbar button end in addTracksFlow(paths). B.tracksScan lists the files with the guessed role and note
   (names of the owner's scheme: k in, s top, t1, oh ...) and the files the app cannot read, with the reason.
   #225: a file whose name gives no role gets the role the audio suggests; the row is marked "from audio" and the
   select is preset to it, the user can change it before the add.
   One click on "Add n tracks" adds them, referenced; the box copies them into the project instead. The engineer
   types nothing. With no project open the drop makes one, named after the folder.
   X-1 (P3-9b): a Pro Tools "Audio Files" folder holds guitars, vocals and rooms too. When the list has files with
   a drum role and files without one (role "other"), only the drum files are checked; "Drums only" and "All"
   switch, "n of m checked" shows the count, and a role picked by hand checks the row. */
let addingTracks = false;
function dropProjectName(paths) {
  const p = String(paths[0] || "").replace(/\/+$/, "").split("/");
  let n = p.pop();
  if (/\.(wav|wave|bwf|aiff?|flac|mp3)$/i.test(n || "")) n = p.pop();
  if (/^audio files$/i.test(n || "")) n = p.pop();
  return (n || "").trim();
}
async function newDropProject(name, copy) {   // a free project name: "<name>", "<name> 2" ...
  for (let k = 1; k <= 50; k++) {
    try { return await B.newProject(name ? (k === 1 ? name : `${name} ${k}`) : null, null, copy ? "copied" : "referenced"); }
    catch (e) { if (e.status !== 409 || !name) throw e; }
  }
  throw new Error("no free project name");
}
/* The confirm list. Resolves {paths, roles, notes, copy} for the checked files, or null for Cancel. */
function askTracks(paths, scan0) {
  const modal = $("#tracksModal"), rows = $("#tracksRows"), addBtn = $("#tracksAdd"), sel = $("#tracksPreset");
  let scan = scan0;
  let scanMixed = false;   // #243: the scan's mixed roles decide the "not checked" note until the next scan
  const noteOf = row => { const n = noteFromName(row.querySelector(".unote").value); return n === null ? ROLE_DEFAULT_NOTE[row.querySelector(".urole").value] : n; };
  const checked = () => [...rows.querySelectorAll(".urow")].filter(r => r.querySelector(".uuse").checked);
  const isDrum = role => role !== "other";
  /* P3-18: a kick sub mic follows the kick in. The source: a kick track of the project that follows nothing, else
     the first checked kick row of the list whose name has no "sub". A row whose name has "sub" and whose role is
     kick gets the box "follow <source>", checked; the user can uncheck it. */
  const isSub = r => /sub/i.test(r.dataset.stem) && r.querySelector(".urole").value === "kick";
  function kickSource() {
    const own = S.sid ? S.order.find(n => S.tracks[n].role === "kick" && !S.tracks[n].follows) : null;
    if (own) return { name: own, row: null };
    const r = checked().find(r => r.querySelector(".urole").value === "kick" && !/sub/i.test(r.dataset.stem));
    return r ? { name: null, row: r } : null;
  }
  function refreshFollow() {
    const src = kickSource();
    for (const r of rows.querySelectorAll(".urow")) {
      const wrap = r.querySelector(".ufollowWrap"), show = !!src && isSub(r) && src.row !== r;
      wrap.classList.toggle("hidden", !show);
      wrap.dataset.src = show ? (src.name || "") : ""; wrap.dataset.srcStem = show && src.row ? src.row.dataset.stem : "";
      if (show) r.querySelector(".ufollowTxt").textContent = `follow ${src.name || src.row.dataset.stem}`;   // names are data
    }
  }
  /* #243: the summary is recounted from the rows on every change: a role picked by hand leaves its
     scan bucket (the names, the audio) and counts as "set by you". */
  function guessSummary() {
    const rs = [...rows.querySelectorAll(".urow")];
    const hand = rs.filter(r => r.dataset.hand).length;
    const names = rs.filter(r => r.dataset.guess !== "audio" && r.dataset.guess !== "none" && !r.dataset.hand).length;
    const audioN = rs.filter(r => r.dataset.guess === "audio" && !r.dataset.hand).length;
    $("#tracksGuess").textContent = rs.length ? `${names} of ${rs.length} roles come from the names.` +
      (audioN ? ` ${audioN} from the audio.` : "") +
      (hand ? ` ${hand} set by you.` : "") +
      (scanMixed ? " Files without a drum role are not checked." : "") + " Check the list and press Add." : "";
  }
  function count() {
    const n = checked().length, m = rows.querySelectorAll(".urow").length;
    addBtn.textContent = `Add ${n} track${n === 1 ? "" : "s"}`; addBtn.disabled = n === 0;
    if (m) $("#tracksTitle").textContent = addBtn.textContent;
    $("#tracksCount").textContent = m ? `${n} of ${m} checked` : "";
    guessSummary();
    refreshFollow();
  }
  function checkWhere(want) {   // "Drums only" and "All"
    for (const r of rows.querySelectorAll(".urow")) r.querySelector(".uuse").checked = want(r.querySelector(".urole").value);
    count();
  }
  function render() {
    rows.innerHTML = "";
    scanMixed = scan.files.some(f => isDrum(f.role)) && scan.files.some(f => !isDrum(f.role));
    for (const f of scan.files) {
      const audio = f.guess === "audio";   // #225: the role comes from the audio, the name gave none
      const row = document.createElement("div"); row.className = "urow" + (f.guess === "none" ? " noguess" : "");
      row.dataset.path = f.path; row.dataset.stem = f.stem; row.dataset.guess = f.guess;   // #243: where the scan took the role from
      row.innerHTML = `<input type="checkbox" class="uuse" ${!scanMixed || isDrum(f.role) ? "checked" : ""} title="add this file">
        <span class="fname"></span>
        <select class="urole">${["kick","snare","tom","hat","cymbal","other"].map(r => `<option ${r === f.role ? "selected" : ""}>${r}</option>`).join("")}</select>
        <input class="unote" type="text" value="${nameFromNote(f.note)}" title="MIDI note name">
        ${audio ? '<span class="ufromaudio" title="the role the audio of the file suggests">from audio</span>' : ""}
        <label class="ufollowWrap hidden" title="P3-18: a sub mic takes the hits of the kick track (sample-exact, velocities from its own signal) instead of its own detection"><input type="checkbox" class="ufollow" checked> <span class="ufollowTxt"></span></label>`;
      const fn = row.querySelector(".fname");
      fn.textContent = `${f.name} (${(f.size / 1e6).toFixed(1)} MB, ${f.sr / 1000} kHz)`;   // textContent: file names are data
      fn.title = f.guess === "none" ? `${f.path}\nno role recognised from the name: it stays "other" unless you pick one`
        : audio ? `${f.path}\nthe role is the audio's suggestion: change it if it is wrong` : f.path;
      row.querySelector(".urole").onchange = ev => {
        row.querySelector(".unote").value = nameFromNote(ROLE_DEFAULT_NOTE[ev.target.value]); row.classList.remove("noguess");
        row.dataset.hand = "1";   // #243: the role is picked by hand now, whatever the scan guessed
        const mark = row.querySelector(".ufromaudio"); if (mark) mark.remove();   // #225: a role picked by hand is no audio suggestion
        if (isDrum(ev.target.value)) row.querySelector(".uuse").checked = true;   // a role picked by hand: add the file
        count();
      };
      row.querySelector(".uuse").onchange = count;
      rows.appendChild(row);
    }
    const rej = $("#tracksRejected"); rej.innerHTML = "";
    for (const r of scan.rejected) {
      const d = document.createElement("div"); d.className = "rejrow";
      d.textContent = `not added: ${r.name} - ${r.reason}`; d.title = r.path; rej.appendChild(d);
    }
    $("#tracksTitle").textContent = scan.files.length ? `Add ${scan.files.length} track${scan.files.length === 1 ? "" : "s"}` : "No track can be added";
    $("#tracksPick").classList.toggle("hidden", !scan.files.length);
    addBtn.classList.toggle("hidden", !scan.files.length);
    $("#tracksCancel").textContent = scan.files.length ? "Cancel" : "Close";
    count();
  }
  async function presets(select) {
    let list = [];
    try { list = await B.kitPresets(); } catch (e) { /* no preset folder: the list stays empty */ }
    sel.innerHTML = `<option value="">Kit preset: names of the scheme</option>` + list.map(p => `<option></option>`).join("");
    [...sel.options].slice(1).forEach((o, i) => { o.value = o.textContent = list[i].name; });
    sel.value = select || "";
    if (sel.value !== (select || "")) sel.value = "";
  }
  return new Promise(resolve => {
    const done = v => { modal.classList.add("hidden"); $("#tracksCancel").onclick = $("#tracksAdd").onclick = sel.onchange = $("#tracksPresetSave").onclick = $("#tracksDrums").onclick = $("#tracksAll").onclick = null; resolve(v); };
    $("#tracksDrums").onclick = () => checkWhere(isDrum);
    $("#tracksAll").onclick = () => checkWhere(() => true);
    $("#tracksCancel").onclick = () => done(null);
    addBtn.onclick = () => { const r = checked(); done({ paths: r.map(x => x.dataset.path), roles: r.map(x => x.querySelector(".urole").value), notes: r.map(noteOf), copy: $("#tracksCopy").checked,
      stems: r.map(x => x.dataset.stem),   // P3-18: the follow of each row: {src: an existing track} or {srcStem: a row of this list}, or null
      follows: r.map(x => { const w = x.querySelector(".ufollowWrap"); return !w.classList.contains("hidden") && x.querySelector(".ufollow").checked ? { src: w.dataset.src || null, srcStem: w.dataset.srcStem || null } : null; }) }); };
    sel.onchange = async () => {
      try { scan = await B.tracksScan(paths, sel.value); render(); } catch (e) { setMsg(`kit preset failed: ${e.message}`, "error"); }
    };
    $("#tracksPresetSave").onclick = async () => {
      const name = dropProjectName(paths) || S.name || "My kit";
      const rules = [...rows.querySelectorAll(".urow")].map(r => ({ pattern: r.dataset.stem, role: r.querySelector(".urole").value, note: noteOf(r) }));
      try { const r = await B.kitPresetSave(name, rules); await presets(r.name); setMsg(`saved the kit preset "${r.name}" (${r.rules} names)`); }
      catch (e) { setMsg(`kit preset failed: ${e.message}`, "error"); }
    };
    $("#tracksCopy").checked = false;
    render();
    presets("").then(() => {});
    modal.classList.remove("hidden");
    (scan.files.length ? addBtn : $("#tracksCancel")).focus();
  });
}
async function addTracksFlow(paths) {
  if (!paths || !paths.length) return;
  if (addingTracks || !$("#tracksModal").classList.contains("hidden")) { setMsg("answer the open dialog first"); return; }
  addingTracks = true;
  try {
    setMsg("reading the files…");
    const scan = await B.tracksScan(paths, "");
    if (!scan.files.length && !scan.rejected.length) { setMsg("no audio files in the dropped items"); return; }
    const pick = await askTracks(paths, scan);
    if (!pick) { setMsg("adding tracks cancelled"); return; }
    if (!S.sid) await useProject(await newDropProject(dropProjectName(paths), pick.copy));
    await flushEvents();
    showProgress(0, `${pick.copy ? "copying" : "adding"} 0/${pick.paths.length}`, pick.copy);   // P3-4d: a copy has Cancel
    let live = true;   // a late message (the test bridge polls) must not override the result
    const r = await B.tracksAdd(S.sid, pick.paths, pick.roles, pick.notes, pick.copy,
      p => {
        if (!live) return;
        if (p.state === "running") showProgress(Math.round(100 * p.done / Math.max(1, p.total)), `${pick.copy ? "copying" : "adding"} ${p.done}/${p.total} - ${p.file}`, pick.copy);
        else if (p.state === "peaks") showProgress(Math.round(100 * p.done / Math.max(1, p.total)), `building waveforms: ${p.done} of ${p.total}`);
      });
    live = false; hideProgress();
    if (r.cancelled) { setMsg("adding tracks cancelled: no track was added and the copied files are removed"); return; }
    const followed = await applyAddFollows(pick, r);   // P3-18
    await initSessions();
    if (await loadProject()) {
      const bad = [...scan.rejected, ...(r.rejected || [])];
      setMsg(`added ${r.added.length} track${r.added.length === 1 ? "" : "s"} (${pick.copy ? "audio copied into the project" : "audio referenced where it is"})` +
        (followed.length ? `; ${followed.map(f => `${f[0]} follows ${f[1]}`).join(", ")}` : "") +
        (bad.length ? `; ${bad.length} not added (${bad[0].name}: ${bad[0].reason})` : "") + " - hit Analyze all");
      setMsgDetail(bad.map(b => `${b.name}: ${b.reason}`).join("\n"));
    }
  } catch (e) { hideProgress(); setMsg(`add tracks failed: ${e.message}`, "error"); setMsgDetail(e.detail); }
  finally { addingTracks = false; }
}
/* P3-18: the follows the list suggested, set after the add (the names come from the service): [[follower, source], ...] */
async function applyAddFollows(pick, r) {
  const byPath = {};
  for (const t of r.tracks || []) byPath[t.path] = t.name;
  const out = [];
  for (let i = 0; i < pick.paths.length; i++) {
    const f = pick.follows && pick.follows[i], name = byPath[pick.paths[i]];
    if (!f || !name) continue;
    const src = f.src || (f.srcStem ? byPath[pick.paths[pick.stems.indexOf(f.srcStem)]] : null);
    if (!src || src === name) continue;
    try { await B.setFollow(S.sid, name, src); out.push([name, src]); }
    catch (e) { setMsg(`${name}: follow failed: ${errText(e)}`, "error"); }
  }
  return out;
}
async function addTracksDialog() {   // File > Add Tracks and the toolbar button: the native file dialog
  const files = await B.pickAudioFiles();
  if (files && files.length) await addTracksFlow(files);
}
if (window.DSShell) DSShell.hooks.otherPaths = (paths, pos) => (lyDrop(paths, pos) ? undefined : addTracksFlow(paths));   // a drop on the window (open-paths, not an HTML5 drop); P4-7a: on the Layer zone it sets the layer

/* ---------------- status ---------------- */
var _msgSeq = 0;   // var, not let: setMsg may run before this line; counts status messages: the background PCM prefetch never overwrites a newer one
/* #112: messages have levels (error, warning, info). A background info does not replace a held error
   until the error's hold time ends (MSG_HOLD) or the user acts (a direct setMsg from a user action). */
var MSG_HOLD = 5000;   // ms an error keeps the status line
var _msgLevel = "info";   // the level of the message now shown
var _msgHoldUntil = 0;   // performance.now() until which a background info must wait
function setMsgLvl(m, lvl, bg) {
  const now = performance.now();
  if (bg && _msgLevel === "error" && now < _msgHoldUntil) return;   // a held error stays readable
  _msgSeq++;
  $("#stMsg").textContent = m;
  _msgLevel = lvl;
  _msgHoldUntil = lvl === "error" ? now + MSG_HOLD : 0;
}
function setMsg(m, lvl) { setMsgLvl(m, lvl || "info", false); }   // direct (user action): replaces everything
function setMsgBg(m, lvl) { setMsgLvl(m, lvl || "info", true); }   // background writer: yields to a held error
let selSynced = null;   // the last selHit that the menu saw (sync only when it flips, not on every drag mousemove)
function selShellSync() {   // #158 (rank 9): Delete, the nudge items and Audition Hit are greyed without a hit or range selection
  const sel = !!(S.sel || LN.sel);
  if (sel !== selSynced) { selSynced = sel; if (window.DSShell) DSShell.sync(); }
}
function updateStatus() {   // P3-14b: the status bar is short; the info line has the details
  const e = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  $("#stSel").textContent = e ? `${S.sel.track}: hit ${S.sel.index + 1}/${S.events[S.sel.track].length}` : "no selection";
  ensureSelTrack();   // P4-39c
  drawInfo();
  syncInspector();   // P3-14a: the Inspector follows the selected track
  lnInspector();   // P4-6a: the Learn row (buttons follow the selection)
  mxRefresh();   // #157: the other mics of the selected hit
  selShellSync();   // #158 (rank 9): the selection-dependent menu items follow the selection
}

/* ---------------- info line (P3-14b, Cubase model) ----------------
   Under the toolbar, a fixed height (it does not move the lanes when a hit is selected). The selected hit: track, index of
   total, time, sample (field), velocity (field), origin (detected, moved, added) and the offset from the detected position
   in samples. Return applies a field, Escape cancels; each edit is one undo step (move or setv). Texts are written only
   when they changed (no layout read). */
var IL = {};   // the last texts written, key -> text
function ilSet(id, text, prop) {
  const k = id + (prop || "");
  if (IL[k] === text) return;
  IL[k] = text;
  const el = $("#" + id);
  if (prop === "value") el.value = text; else el.textContent = text;
}
function hitOrigin(e, sr) {   // {origin, offset}: "detected" 0, "moved" and the offset in samples, "added" and null
  if (e.added) return { origin: "added", offset: null };
  if (isManual(e)) return { origin: "moved", offset: Math.round((e.t - e.orig) * sr) };
  return { origin: "detected", offset: 0 };
}
function drawInfo() {
  const box = $("#infoLine");
  const shown = !!SETTINGS().ui.infoLine;
  if (box.classList.contains("hidden") === shown) box.classList.toggle("hidden", !shown);
  if (!shown) return;
  const selText = lnSelText(), selEl = $("#ilSel");   // P4-6a: the range selection and its hit count
  ilSet("ilSel", selText); if (selEl.classList.contains("hidden") === !!selText) selEl.classList.toggle("hidden", !selText);
  const e = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  const f = $("#ilFields"), empty = $("#ilEmpty");
  if (f.classList.contains("hidden") !== !e) f.classList.toggle("hidden", !e);
  if (empty.classList.contains("hidden") !== !!e) empty.classList.toggle("hidden", !!e);
  if (!e) return;
  const tr = S.tracks[S.sel.track], sr = tr.sr, o = hitOrigin(e, sr);
  ilSet("ilTrack", S.sel.track);
  ilSet("ilIndex", `${S.sel.index + 1}/${S.events[S.sel.track].length}`);
  ilSet("ilTime", `${fmt(e.t, 6)} s`);
  if (document.activeElement !== $("#ilSample")) ilSet("ilSample", String(Math.round(e.t * sr)), "value");
  if (document.activeElement !== $("#ilVel")) ilSet("ilVel", String(e.v), "value");
  ilSet("ilOrigin", o.origin);
  ilSet("ilOffset", o.offset === null ? "-" : `${o.offset > 0 ? "+" : ""}${o.offset} smp`);
  const lk = lnLearn(S.sel.track) ? lnKindOf(S.sel.track, e.t) : null, lf = $("#ilLearnF");   // P4-6a: a hit that is a Learn example
  // P4-6c: the hit's similarity score; a grey hit of the filter says why
  const sim = lnLearn(S.sel.track) ? lnScoreAt(S.sel.track, e.t - lnShift(S.sel.track)) : null, th = (LN_SC[lnTarget(S.sel.track)] || {}).threshold;
  const simTxt = sim === null ? "" : lnGreyHit(S.sel.track, e) && th !== undefined ? `filtered: sim ${sim.toFixed(2)} < ${th.toFixed(2)}` : `sim ${sim.toFixed(2)}`;
  ilSet("ilLearn", lk === "not" ? "not this" : lk === "this" ? "this" : simTxt);
  const show = !!lk || !!simTxt;
  if (lf.classList.contains("hidden") === show) lf.classList.toggle("hidden", !show);
  const box2 = $("#ilOrigin"); if (box2.dataset.o !== o.origin) box2.dataset.o = o.origin;
  const why = rqWhy(e), df = $("#ilDoubtF");   // #150: why the review queue counts the hit as doubtful
  ilSet("ilDoubt", why);
  if (df.classList.contains("hidden") === !!why) df.classList.toggle("hidden", !why);
  const lm = lyLayer(S.sel.track) ? lyMatchOf(S.sel.track, e.t) : null, mf = $("#ilMatchF");   // P4-7b: the layer match of the hit
  ilSet("ilMatch", lm ? `${lm.match === null ? "-" : lm.match.toFixed(2)}${lm.uncertain ? " uncertain" : ""}${lm.sign !== null && LY.align[S.sel.track] && lm.sign !== (LY.align[S.sel.track].polarity === "inverted" ? -1 : 1) ? " flip" : ""}` : "");
  if (mf.classList.contains("hidden") === !!lm) mf.classList.toggle("hidden", !lm);
}
function setInfoLine(on) {   // View > Show Info Line
  SETTINGS().ui.infoLine = !!on; saveSettings();
  drawInfo(); DSShell.sync();
}
function infoFieldApply(which) {   // Return in a field: the value is applied as one undo step; a bad value is refused with a message
  const inp = $(which === "sample" ? "#ilSample" : "#ilVel");
  const e = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  const text = inp.value.trim();
  const back = () => { IL[inp.id + "value"] = null; drawInfo(); };
  if (!e) { back(); return false; }
  if (!/^[0-9]+$/.test(text)) { setMsg(`${which === "sample" ? "sample" : "velocity"}: type a whole number`); back(); return false; }
  const n = parseInt(text, 10), trk = S.sel.track, t = S.tracks[trk];
  if (which === "sample") {
    const last = (t.n_samples || Infinity) - 1, smp = Math.min(n, last);
    if (smp !== n) setMsg(`sample ${n} is after the end of ${trk}: set to ${smp}`);
    setHitTime(trk, S.sel.index, smp / t.sr);
  } else {
    const v = Math.max(1, Math.min(127, n));
    if (v !== n) setMsg(`velocity ${n} is outside 1 to 127: set to ${v}`);
    if (followerLocked(trk)) { back(); updateStatus(); return true; }   // P3-18
    const id = e.id;
    edCall(trk, evs => {   // events_edit velocity, one undo step
      const j = edIndex(evs, id);
      if (j < 0 || evs[j].v === v) return null;
      return { ops: [{ op: "velocity", index: j, v, edited: true }], step: { type: "setv", track: trk, id, from: evs[j].v, to: v, was: hitWas(evs[j]) } };   // setv; P4-40f: a hand velocity is an edit; R05: was for the undo
    });
  }
  back(); updateStatus();
  return true;
}
function setupInfoLine() {
  for (const [id, which] of [["ilSample", "sample"], ["ilVel", "vel"]]) {
    const inp = $("#" + id);
    inp.addEventListener("keydown", ev => {
      if (ev.key === "Enter") { ev.preventDefault(); ev.stopPropagation(); infoFieldApply(which); inp.blur(); }
      else if (ev.key === "Escape") { ev.preventDefault(); ev.stopPropagation(); IL[id + "value"] = null; drawInfo(); inp.blur(); }
    });
    inp.addEventListener("blur", () => { IL[id + "value"] = null; drawInfo(); });   // a field that loses the focus is cancelled
  }
  $("#timeDisplay").addEventListener("click", () => exchangeCounters());
  $("#timeDisplay").style.cursor = "pointer";
  const ns = $("#nudgeSel");   // P3-20a: the list of nudge values; focus goes back to the page so the keys work
  for (const k of NUDGE_ORDER) ns.add(new Option(`Nudge ${NUDGE_VALUES[k].label}`, k));
  ns.value = S.nudgeValue;
  ns.addEventListener("change", () => { chooseNudgeValue(ns.value); ns.blur(); });
  DSKeys.live.nudge_earlier = DSKeys.live.nudge_later = () => `now ${NUDGE_VALUES[S.nudgeValue].label}`;
  $("#timeDisplay").title = `Main counter (${SETTINGS().ui.counter}). Click: exchange the counters (View > Main Counter > Exchange Counters)`;
  drawInfo();
}

/* ---------------- editor commands (P3-13a) ----------------
   One function per command of the native menu (shell.js `handlers`); the keys and the toolbar call the same
   function, so a menu item, its key and its button do the same thing. */
function addHitAt(name, t, snap) {   // hand-added → manual (magenta): events_edit add, one undo step; #144: snap puts it on the attack near t
  if (followerLocked(name)) return;   // P3-18
  const id = newId();
  return edCall(name, () => {
    const pl = { ops: [{ op: "add", t, ...(snap ? { snap: true } : {}) }], make: [[t, id]], sel: id, step: { type: "add", track: name, id, t, v: 100 } };
    if (snap) pl.done = r => {   // the step and the page id follow the hit to where it landed; the redo adds it there, without a new snap
      const s = r && r.snaps && r.snaps[0];
      if (!s) return;
      pl.make = [[s.t, id]]; pl.step.t = s.t; pl.step.v = s.v;
      const sr = S.tracks[name].sr || 48000, d = Math.round((s.t - s.click) * sr);
      pl.msg = s.snapped ? `added hit on the attack (${d >= 0 ? "+" : ""}${d} samples from the click), velocity ${s.v}`
        : "no attack within 10 ms: the hit stays at the click, velocity 100";
    };
    return pl;
  });
}
function needTrack() {   // the selected track (header click), or a message
  if (!needProject()) return null;
  if (!S.selTrack || !S.tracks[S.selTrack]) { setMsg("no track selected: click a track header first"); return null; }
  return S.selTrack;
}
function addHitAtPlayhead() { const n = needTrack(); if (n) addHitAt(n, snapSample(S.playhead, n)); }
function deleteSelectedHit() {
  if (LN.sel) return rangeDelete();   // #151: a range selection takes the key: every hit of the range, one undo step
  if (!S.sel) { setMsg("no hit selected"); return; }
  const trk = S.sel.track, e = (S.events[trk] || [])[S.sel.index];
  if (followerLocked(trk)) return;   // P3-18
  if (e) return edDelete(trk, e.id);
}
/* #151 (workflow polish rank 3): Delete and the nudge keys act on every hit of the range selection, so a bleed
   burst or a whole fill is one action, not one per hit. One events_edit call with one op per hit, one undo step
   (del-many, move-many — the shape of setv-many): the step holds every hit (id, t, v, added, orig, keep), so one
   undo puts the document back byte-identical (the keep fields of the stored hits, as in edDelete). Every hit of
   the range counts, a cut or bleed one too: the range is aimed at what the lane shows. No hit in the range: a
   message, no step. A follower is refused (its hits are derived, P3-18). The range stays after the edit. */
async function rangeDelete() {
  const s = LN.sel, trk = s.track;
  if (followerLocked(trk)) return;
  let stored = [];   // the service's hits: the step keeps the fields of each hit as they are stored
  try {
    if (S.sid && (S.events[trk] || []).length) { if (hasPendingWrite(trk)) await settleEvents(trk); const r = await B.events(S.sid, trk); stored = Array.isArray(r) ? r : (r && r.events) || []; }
  } catch (e) { stored = []; }
  return edCall(trk, evs => {
    const pool = stored.slice(), list = [];
    for (const e of evs) {
      if (e.t < s.t0 || e.t > s.t1) continue;
      const j = edIndex(evs, e.id);
      if (j < 0) continue;
      const raw = pool.find(x => Math.abs(x.t - e.t) < 1e-9);
      if (raw) pool.splice(pool.indexOf(raw), 1);
      list.push({ id: e.id, t: e.t, v: e.v, added: !!e.added, ...(e.orig !== undefined ? { orig: e.orig } : {}), keep: delKeep(raw) });
    }
    if (!list.length) { setMsg(`no hits in the range (${fmt(s.t0, 3)} to ${fmt(s.t1, 3)} s)`); return null; }
    return { ops: list.map(x => ({ op: "delete", index: edIndex(evs, x.id) })), drop: list.map(x => x.id), sel: null,
      msg: `deleted ${list.length} ${list.length === 1 ? "hit" : "hits"} of the range`,
      step: { type: "del-many", track: trk, list } };
  });
}
/* P3-20a (A1 p. 28 "Change Nudge value"): the nudge value of the arrow, comma and period keys. 1, 10, 100 samples, 1 ms, 10 ms
   (ms at the rate of the hit's track, rounded to a sample, at least one). A view setting (app setting "nudge_value"). */
const NUDGE_VALUES = { "1smp": { label: "1 smp", smp: () => 1 }, "10smp": { label: "10 smp", smp: () => 10 }, "100smp": { label: "100 smp", smp: () => 100 },
  "1ms": { label: "1 ms", smp: sr => Math.max(1, Math.round(sr * 0.001)) }, "10ms": { label: "10 ms", smp: sr => Math.max(1, Math.round(sr * 0.01)) } };
const NUDGE_ORDER = Object.keys(NUDGE_VALUES);
function nudgeSamples(sr) { return NUDGE_VALUES[S.nudgeValue].smp(sr || 48000); }
function setNudgeValue(key, save) {
  if (!NUDGE_VALUES[key]) return false;
  S.nudgeValue = key;
  const sel = $("#nudgeSel"); if (sel) sel.value = key;
  if (save !== false && B.appSettingsSet) B.appSettingsSet("nudge_value", key).catch(e => setMsg(`settings: ${errText(e)}`));
  DSShell.sync();
  return true;
}
function chooseNudgeValue(key) { if (setNudgeValue(key)) setMsg(`nudge value: ${NUDGE_VALUES[key].label}`); }   // Edit > Nudge Value
function stepNudgeValue(dir) {   // Option+Cmd+numpad plus (+1) and minus (-1)
  const i = NUDGE_ORDER.indexOf(S.nudgeValue) + dir;
  if (i < 0 || i >= NUDGE_ORDER.length) { setMsg(`nudge value: ${NUDGE_VALUES[S.nudgeValue].label} is the ${dir > 0 ? "largest" : "smallest"}`); return; }
  chooseNudgeValue(NUDGE_ORDER[i]);
}
function nudgeUnits(units) {   // the selected hit by units x the nudge value (negative = earlier); a range selection moves all its hits (#151)
  if (LN.sel) return rangeNudge(units);
  if (!S.sel) { setMsg("no hit selected"); return; }
  nudgeSelected(units * nudgeSamples(S.tracks[S.sel.track].sr));
}
function rangeNudge(units) {   // #151: every hit of the range by units x the nudge value, one call, one undo step (move-many)
  const s = LN.sel, trk = s.track, sr = S.tracks[trk] && S.tracks[trk].sr;
  if (followerLocked(trk)) return;
  if (!sr) { setMsg(`${trk}: the track has no sample rate`); return; }
  return edCall(trk, evs => {
    const list = [];
    let inRange = 0;
    for (const e of evs) {
      if (e.t < s.t0 || e.t > s.t1) continue;
      inRange++;
      const to = Math.max(0, Math.round((e.t + units * nudgeSamples(sr) / sr) * sr) / sr);
      if (to === e.t) continue;
      const j = edIndex(evs, e.id);
      if (j < 0) continue;
      list.push({ id: e.id, from: e.t, to, was: hitWas(e) });
    }
    if (!inRange) { setMsg(`no hits in the range (${fmt(s.t0, 3)} to ${fmt(s.t1, 3)} s)`); return null; }
    if (!list.length) { setMsg("the hits of the range did not move"); return null; }
    return { ops: list.map(x => ({ op: "move", index: edIndex(evs, x.id), t: x.to })), drop: list.map(x => x.id), make: list.map(x => [x.to, x.id]),
      msg: `nudged ${list.length} ${list.length === 1 ? "hit" : "hits"} of the range by ${NUDGE_VALUES[S.nudgeValue].label}`,
      step: { type: "move-many", track: trk, list } };
  });
}
/* P3-20a (A1 p. 33, 34 "Waveform zoom"): the amplitude zoom of every wave lane, 1x to 64x in steps of two, a display gain only
   (peaks, hits and exports stay). A clipped peak draws at the lane edge (the lane clips). A view setting ("amp_zoom"). */
const AMP_STEPS = [1, 2, 4, 8, 16, 32, 64];
function setAmpZoom(v, save) {
  if (!AMP_STEPS.includes(v)) return false;
  const changed = S.ampZoom !== v;
  S.ampZoom = v;
  const el = $("#ampLbl"); if (el) { el.textContent = `Amp ${v}x`; el.classList.toggle("hidden", v === 1); }
  if (changed) { GLQ.box = ""; invalidate(F.VIEW | F.GL); }
  if (changed && save !== false && B.appSettingsSet) B.appSettingsSet("amp_zoom", v).catch(e => setMsg(`settings: ${errText(e)}`));
  DSShell.sync();
  return true;
}
function stepAmpZoom(dir) {   // In (+1) doubles, Out (-1) halves
  const i = AMP_STEPS.indexOf(S.ampZoom) + dir;
  if (i < 0 || i >= AMP_STEPS.length) { setMsg(`amplitude zoom: ${S.ampZoom}x is the ${dir > 0 ? "largest" : "smallest"}`); return; }
  setAmpZoom(AMP_STEPS[i]); setMsg(`amplitude zoom: ${S.ampZoom}x`);
}
function nudgeSelected(step) {   // step in samples, negative = earlier; the time is read when the call runs (two quick nudges add up)
  if (!S.sel) { setMsg("no hit selected"); return; }
  const trk = S.sel.track, i = S.sel.index, sr = S.tracks[trk].sr;
  return setHitTime(trk, i, from => from + step/sr, "Nudge Hit");
}
function setHitTime(trk, i, t, label) {   // move hit i of the track to the sample nearest to t (a number, or a function of the hit's time): events_edit move, one undo step (nudge, info line)
  if (followerLocked(trk)) return;   // P3-18
  const e = (S.events[trk] || [])[i];
  if (!e) return;
  const id = e.id, sr = S.tracks[trk].sr;
  return edCall(trk, evs => {
    const j = edIndex(evs, id);
    if (j < 0) return null;
    const from = evs[j].t, to = Math.max(0, Math.round((typeof t === "function" ? t(from) : t) * sr) / sr);
    if (to === from) return null;
    return { ops: [{ op: "move", index: j, t: to }], drop: [id], make: [[to, id]], sel: S.sel && S.sel.track === trk ? id : undefined,
      step: { type: "move", track: trk, id, from, to, ...(label ? { label } : {}), was: hitWas(evs[j]) } };
  });
}
function zoomStep(inward) { flashBtn(document.querySelector(`.zBtn[data-z="${inward ? "in" : "out"}"]`)); zoomPlayhead(inward ? 0.5 : 2); }
function zoomPreset(i) {   // 0 = Full, 1 = 30 s, 2 = 5 s, 3 = 500 ms, 4 = 50 ms, 5 = 5 ms (the toolbar buttons, largest span first)
  const opts = [...document.querySelectorAll("#zoomSel option[value]:not([value=''])")]   // P3-14e: the dropdown, largest span first
    .map(o => o.value);
  const span = opts[i];
  if (span === undefined) return false;
  flashBtn($("#zoomSel")); applyZoomSpan(span);
  return true;
}
function applyZoomSpan(v) {   // a preset span ("full" or seconds), centred on the playhead
  const span = v === "full" ? maxDur() : parseFloat(v);
  S.followHold = true;   // F1: user view change
  setView(S.playhead - span/2, S.playhead + span/2);
}
/* #169: View > Show Overview: the overview strip is shown or hidden; the lanes take its height (the layout, then the
   ResizeObserver of P3-14d). The choice is the app setting overview. */
function setOverview(on, save) {
  S.overviewOn = !!on;
  $("#overviewWrap").classList.toggle("hidden", !S.overviewOn);
  invalidate(F.SIZE | F.VIEW);
  if (save !== false) {
    if (B.appSettingsSet) B.appSettingsSet("overview", S.overviewOn).catch(e => setMsg(`settings: ${errText(e)}`));
    setMsg(`overview ${S.overviewOn ? "shown" : "hidden"}`);
  }
  if (window.DSShell && DSShell.sync) DSShell.sync();
}
/* #169: View > Zoom Presets > Add Current and Organize: the engineer's own spans, the app setting zoom_presets (a list
   of {name, span}, span in seconds). Each one is an entry of the zoom dropdown after the six presets, and of the menu:
   the first five with Option+1 to Option+5 (sync_zoom_own in menu.rs reads the names of the page state). */
function renderZoomOwn() {   // the dropdown entries of the own presets
  const sel = $("#zoomSel");
  for (const o of sel.querySelectorAll("option[data-own]")) o.remove();
  for (const p of S.zoomOwn) {
    const o = document.createElement("option");
    o.value = String(p.span); o.dataset.own = "1"; o.textContent = p.name;
    sel.appendChild(o);
  }
}
function saveZoomOwn() {   // the list, the dropdown and the menu follow
  renderZoomOwn();
  if (B.appSettingsSet) B.appSettingsSet("zoom_presets", S.zoomOwn).catch(e => setMsg(`settings: ${errText(e)}`));
  if (window.DSShell && DSShell.sync) DSShell.sync();
}
function applyOwnZoom(n) {   // an own preset: the menu item (Option+1 to Option+5 for the first five) or the dropdown
  const p = S.zoomOwn[n];
  if (!p) return false;
  flashBtn($("#zoomSel")); applyZoomSpan(String(p.span));
  return true;
}
/* the zoom preset dialogs: one modal for Add Current and Organize. fill(body) builds the body, accept() returns true
   when the dialog may close; Cancel and Escape close it too (cancel: false hides the Cancel button). */
function presetDialog(title, hint, fill, accept, cancel) {
  const m = $("#presetModal"), body = $("#presetBody"), ok = $("#presetAccept"), no = $("#presetCancel");
  $("#presetTitle").textContent = title; $("#presetHint").textContent = hint || "";
  body.textContent = ""; fill(body);
  no.classList.toggle("hidden", !cancel);
  const close = () => { m.classList.add("hidden"); document.removeEventListener("keydown", esc, true); };
  const esc = ev => { if (ev.key === "Escape") { ev.preventDefault(); close(); } };
  ok.onclick = () => { if (accept()) close(); };
  no.onclick = close;
  document.addEventListener("keydown", esc, true);
  m.classList.remove("hidden");
  const first = body.querySelector("input") || ok;
  first.focus();
}
function zoomPresetAdd() {   // File: View > Zoom Presets > Add Current: the span of the view under a name
  const span = S.view.t1 - S.view.t0;
  presetDialog("Add Current Span", `The span is ${fmt(span, 3)} s. The name shows in the zoom dropdown and in View > Zoom Presets.`, body => {
    const inp = document.createElement("input");
    inp.type = "text"; inp.id = "presetName"; inp.maxLength = OWN_ZOOM_NAME_MAX; inp.value = `Span ${S.zoomOwn.length + 1}`;
    inp.onkeydown = ev => { if (ev.key === "Enter") { ev.preventDefault(); $("#presetAccept").click(); } };
    body.appendChild(inp);
  }, () => {
    const name = $("#presetName").value.trim().slice(0, OWN_ZOOM_NAME_MAX);
    if (!name) { $("#presetName").focus(); return false; }
    S.zoomOwn = [...S.zoomOwn, { name, span }];
    saveZoomOwn(); setMsg(`zoom preset "${name}" saved`);
    return true;
  }, true);
}
function zoomPresetOrganize() {   // View > Zoom Presets > Organize: rename and remove (each change is saved at once)
  presetDialog("Organize Zoom Presets", S.zoomOwn.length ? "Rename a preset here or remove it. The first five keep Option+1 to Option+5." : "No own presets yet: Add Current saves the span of the view.",
    body => renderPresetRows(body), () => true, false);
}
function renderPresetRows(body) {   // one row per own preset: the name (renamed on change) and Remove
  body.textContent = "";
  S.zoomOwn.forEach((p, n) => {
    const row = document.createElement("div"); row.className = "presetRow";
    const inp = document.createElement("input"); inp.type = "text"; inp.value = p.name; inp.maxLength = OWN_ZOOM_NAME_MAX;
    inp.onchange = () => {
      const name = inp.value.trim().slice(0, OWN_ZOOM_NAME_MAX);
      if (!name) { inp.value = p.name; return; }
      S.zoomOwn = S.zoomOwn.map((q, i) => i === n ? { ...q, name } : q);
      saveZoomOwn();
    };
    const rm = document.createElement("button"); rm.className = "btn"; rm.textContent = "Remove";
    rm.onclick = () => { S.zoomOwn = S.zoomOwn.filter((_, i) => i !== n); saveZoomOwn(); renderPresetRows(body); };
    row.append(inp, rm);
    body.appendChild(row);
  });
}
/* P3-19 (A1 p. 5 "Zoom Toggle"): E zooms to 5 ms around the selected hit (the playhead when no hit is selected) and
   stores the view; E again puts the stored view back. Option+Shift+E keeps the view and forgets the stored one.
   P4-27: with a range selected (P4-6a) E zooms so the range fills the lane width exactly. With the app setting "Zoom Toggle
   expands track height" (`zoom_toggle_tall`) E also doubles the wave lane of the track that holds the range (else the selected
   hit's track) and scrolls it into view; E again puts the view, the scroll position and the heights back. The height is a
   local override of the row (S.rowH, a factor of the global wave height; a CSS variable of the row); the app
   setting track_height is not touched. */
const ZOOM_TOGGLE_SPAN = 0.005;
function rowHKey() { return S.rowH ? Object.entries(S.rowH).map(([n, h]) => n + "=" + h).join("|") : ""; }
function setRowHeights(map) {   // map: track name -> factor of the global wave height (an override), {} = every row as the Track Height says; #167
  S.rowH = {};
  for (const row of document.querySelectorAll("#lanesScroll .track")) {
    const f = map[row.dataset.name];
    if (f) { S.rowH[row.dataset.name] = f; row.style.setProperty("--wave-h", `${Math.round(laneHeights().wave * f)}px`); }
    else row.style.removeProperty("--wave-h");
    setHeadTartFit(row);   // #172: a row dragged below the header drawing's height hides the drawing
  }
  invalidate(F.SIZE | F.GL | F.NEAR);
}
/* #172: the role drawing (.tart) sits bottom-right in the header (bottom: 8px); when the row is
   dragged short it would overlap the .tctl controls (the M/S buttons and the pan slider). The
   drawing shows only while the header is tall enough for the drawing and its 8 px margin above
   the top of the controls. The size is the authored --tart-size, not the live box: a hidden
   drawing measures 0 px and would unhide itself (an oscillation). Measured here (outside the
   frame loop), not per frame. */
function setHeadTartFit(row) {
  const th = row.querySelector(".thead");
  const tctl = th && th.querySelector(".tctl");
  const tart = th && th.querySelector(".tart");
  if (!th || !tctl || !tart) return;
  const size = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--tart-size")) || 54;
  const needed = tctl.offsetTop + 8 + size;
  th.classList.toggle("tart-hidden", row.offsetHeight < needed);
}
function rowHeightsSave() {   // #167: the factor map into the project document (the drag and the double-click land here)
  if (!S.sid || !B.rowHeightsSet) return;
  B.rowHeightsSet(S.sid, S.rowH).catch(e => setMsg(`row heights: ${errText(e)}`, "error"));
}
function rowHeightOf(name) { return S.rowH[name] || 1; }   // the factor of a row (1 = as the Track height says)
function rowHeightPx(name) { return Math.round(laneHeights().wave * rowHeightOf(name)); }
/* #167: the lower border of a row is a drag handle for that row's wave lane. The drag stores a
   factor of the global wave height (the Track Height zoom scales every lane, the dragged one
   included); a double-click puts the row back to the global height. The map lands in the project
   (row_heights) when the drag ends or the double-click resets. */
function bindRowResize(row, name, hd) {   // hd: the handle; the row's lower border (default) or the header's bottom edge (#439: the same drag)
  hd = hd || row.querySelector(".tlanes .rowResize");
  let drag = null;
  hd.addEventListener("pointerdown", ev => {
    if (ev.button !== 0) return;
    if (VL.on) { vlResize(ev); return; }   // #437: with the velocity lane on, the handle is at its bottom edge and drags the lane's height
    drag = { y: ev.clientY, f0: rowHeightOf(name), px0: rowHeightPx(name), f: rowHeightOf(name) };
    hd.setPointerCapture(ev.pointerId);
    ev.preventDefault(); ev.stopPropagation();
  });
  hd.addEventListener("pointermove", ev => {
    if (!drag) return;
    const px = Math.min(1200, Math.max(24, drag.px0 + ev.clientY - drag.y));
    drag.f = px / laneHeights().wave;
    setRowHeights({ ...S.rowH, [name]: drag.f });
  });
  const end = () => {
    if (!drag) return;
    drag = null;
    rowHeightsSave();
  };
  hd.addEventListener("pointerup", end);
  hd.addEventListener("pointercancel", end);
  hd.addEventListener("dblclick", ev => {   // the row back to the global height
    ev.preventDefault(); ev.stopPropagation();
    const map = { ...S.rowH };
    delete map[name];
    setRowHeights(map);
    rowHeightsSave();
    setMsg(`track height of ${name}: ${S.trackHeight}`);
  });
}
function setZoomTall(on, save) {
  S.zoomTall = !!on;
  const c = $("#setZoomTall"); if (c) c.checked = S.zoomTall;
  if (save !== false && B.appSettingsSet) B.appSettingsSet("zoom_toggle_tall", S.zoomTall).catch(e => setMsg(`settings: ${errText(e)}`));
}
function zoomToggle(keep) {
  if (!needProject()) return;
  if (keep) { S.zoomBack = null; setMsg("zoom kept: E zooms in again"); return; }
  S.followHold = true;
  const sc = $("#lanesScroll");
  if (S.zoomBack) {
    const z = S.zoomBack; S.zoomBack = null;
    setView(z.t0, z.t1);
    if (z.rowH) { setRowHeights(Object.fromEntries(Object.entries(z.rowH).filter(([n]) => S.tracks[n]))); sc.scrollTop = z.top; }
    return;
  }
  const rg = LN.sel && S.tracks[LN.sel.track] && LN.sel.t1 > LN.sel.t0 ? LN.sel : null;
  const e = S.sel ? (S.events[S.sel.track] || [])[S.sel.index] : null;
  S.zoomBack = { t0: S.view.t0, t1: S.view.t1, rowH: null, top: sc.scrollTop };
  if (rg) setView(rg.t0, rg.t1);   // the range fills the lane width
  else { const t = e ? e.t : S.playhead; setView(t - ZOOM_TOGGLE_SPAN / 2, t + ZOOM_TOGGLE_SPAN / 2); }
  const trk = rg ? rg.track : e ? S.sel.track : null, row = trk && rowOf(trk);
  if (S.zoomTall && row) {
    S.zoomBack.rowH = { ...S.rowH };
    setRowHeights({ ...S.rowH, [trk]: rowHeightOf(trk) * 2 });
    row.scrollIntoView({ block: "nearest" });   // forces the layout once; this is a key command, not a frame
    GEO.top = sc.scrollTop;
  }
}
/* P3-19 (A1 p. 6): P and ; select the previous and the next track; nothing happens at the first and the last track.
   No track selected: ; takes the first, P the last. */
function selectTrackStep(dir) {
  if (!needProject() || !S.order.length) return;
  const i = S.order.indexOf(S.selTrack);
  const j = i < 0 ? (dir > 0 ? 0 : S.order.length - 1) : i + dir;
  if (j < 0 || j >= S.order.length) return;
  setSelTrack(S.order[j]);
  const row = document.querySelector(`.track[data-name="${CSS.escape(S.order[j])}"]`);
  if (row && row.scrollIntoView) row.scrollIntoView({ block: "nearest" });
}
function zoomToProject() { S.followHold = true; setAmpZoom(1); setView(0, maxDur()); }   // P3-20a: also resets the amplitude zoom (A1 p. 34)
function projectEnd() { return S.order.length ? Math.max(...S.order.map(n => S.tracks[n].duration || 0)) : 0; }
function goTo(t) {   // Transport > Go to ...: the playhead moves (a seek while playing), the view follows it
  S.playhead = Math.max(0, t);
  if (S.playhead < S.view.t0 || S.playhead > S.view.t1) centerOn(S.playhead);
  invalidate(F.HEAD); updateStatus();
}
function playStop(key) {   // key: the pressed key in the table's notation (a menu item or the play button: undefined)
  return auStop() ? undefined : S.playing
    ? ((S.playReturn && (key === "Space" || key === S.playReturn))   // #207 play-and-return: the next Space or the same key returns
       || SETTINGS().ui.stopReturnToStart ? stopAndReturn()   // #436: the mode returns every Stop of Play/Stop (Space, Numpad0, the ■ button)
      : pause())
    : play(key === "Option+Space" ? key : null);
}
function halfSpeedPlayStop(key) {   // P4-40d: Space and Shift+Space stop an audition first; key as in playStop (#207)
  return auStop() ? undefined : S.playing
    ? (S.playReturn && (key === "Space" || key === S.playReturn) ? stopAndReturn() : pause())
    : appPlay(0.5, key === "Option+Shift+Space" ? key : null);
}
function stopAndReturn() { const p = S.playStartPos; auStop(true); pause(); S.playhead = p; invalidate(F.HEAD); updateStatus(); }
const FOLLOW_ICON = { smooth: "⇢", page: "⇥", off: "⊘" };   // P3-14e: the follow button shows the mode
let lastFollow = "smooth";   // the mode that Follow Playhead On/Off (F) turns back on
function setFollow(mode) {
  if (!["smooth", "page", "off"].includes(mode)) return;
  if (mode !== "off") lastFollow = mode;
  S.followMode = mode; S.followHold = false;
  const fb = $("#followBtn"); fb.dataset.mode = mode; fb.textContent = FOLLOW_ICON[mode];
  fb.classList.toggle("on", mode !== "off"); fb.setAttribute("aria-label", `follow playhead: ${mode}`);
  DSShell.sync();
}
function toggleFollow() { setFollow(S.followMode === "off" ? lastFollow : "off"); setMsg(`follow playhead: ${S.followMode}`); }
/* P4-wf10: Transport > Suspend Follow when Editing (a user setting: it persists across a restart).
   While it is checked and the transport plays, an add, move or delete of a hit suspends the follow: the
   view start does not change for 2 s of playback, then followResume turns the view to the page of the playhead. */
function toggleSuspendFollow() {
  const st = SETTINGS();
  st.ui.suspendFollowOnEdit = !st.ui.suspendFollowOnEdit; saveSettings();
  if (!st.ui.suspendFollowOnEdit) S.followResumeAt = null;   // unchecked while suspended: follow again at once
  setMsg(`suspend follow when editing: ${st.ui.suspendFollowOnEdit ? "on" : "off"}`);
  DSShell.sync();
}
/* #436: Transport > Stop Returns to Start (a user setting: it persists across a restart). While it is
   checked, a Stop (Space while playing, or the ■ button) puts the playhead back where the play started,
   exactly like Option+Space. Unchecked, the stop keeps the playhead where it is. */
function toggleStopReturnMode() {
  const st = SETTINGS();
  st.ui.stopReturnToStart = !st.ui.stopReturnToStart; saveSettings();
  setMsg(`stop returns to start: ${st.ui.stopReturnToStart ? "on" : "off"}`);
  DSShell.sync();
}
function suspendFollowForEdit() {   // an edit during playback: hold the view, resume 2 s of playback later
  if (!SETTINGS().ui.suspendFollowOnEdit || !S.playing || S.followMode === "off") return;
  S.followHold = true;
  S.followResumeAt = AC.drawn + 2;   // song seconds: 2 s of playback (a second edit extends it)
}
function followResume() {   // in the tick: 2 s of playback after the edit the follow resumes at the next page
  if (S.followResumeAt === null || !S.playing || PH.v < S.followResumeAt) return;
  S.followResumeAt = null;
  if (!S.followHold) return;
  S.followHold = false;
  const d = viewDur(), v = viewClamp(PH.v - FOLLOW_FRAC * d, PH.v - FOLLOW_FRAC * d + d);
  if (Math.abs(v.t0 - S.view.t0) > 1e-12 || Math.abs(v.t1 - S.view.t1) > 1e-9) setView(v.t0, v.t1);   // the next page: only a turn is a setView
}
function setSnap(on) { S.snap = !!on; $("#snapChk").checked = S.snap; DSShell.sync(); }
function toggleSnap() { setSnap(!S.snap); setMsg(S.snap ? "snap to hits on" : "snap to hits off"); }
function setClick(on) {
  const st = SETTINGS();
  st.click.enabled = !!on; saveSettings();
  $("#setClickOn").checked = st.click.enabled; $("#clickBtn").classList.toggle("on", st.click.enabled);
  DSShell.sync();
}
function toggleClick() {
  setClick(!SETTINGS().click.enabled);
  setMsg(SETTINGS().click.enabled ? "metronome click on — beeps on every hit of audible tracks" : "metronome click off");
}
function trackButton(cls) {   // Track > Mute or Solo Selected Track: the header button of the selected track
  const n = needTrack();
  const row = n && document.querySelector(`.track[data-name="${CSS.escape(n)}"]`);
  if (row) row.querySelector(cls).click();
}
function analyzeSelected() { const n = needTrack(); if (n) startAnalysis([n]); }
function exportSelectedTracks() { const n = needTrack(); if (n) return openExport("selected", n); }   // P4-30: Track > Export Selected Tracks…
function showModal(id) {   // Settings and Keyboard Shortcuts: Escape or the button closes them
  const m = $(id);
  m.classList.remove("hidden");
  const esc = ev => { if (ev.key === "Escape" && !m.classList.contains("hidden")) { ev.stopPropagation(); m.classList.add("hidden"); } };
  document.removeEventListener("keydown", m._esc, true);
  m._esc = esc; document.addEventListener("keydown", esc, true);
}
function openSettings() { showModal("#settingsModal"); refreshAudioOutput(); setPluginsFill(); }
/* ---------------- #387: detector plug-ins ----------------
   The installed plugins (plugins_list), the Detector select in the Inspector (buildParamsPanel:
   first entry Built-in, then one entry per plugin that supports the track's role and rate),
   the licence dialog before the first use and the missing-plugin ask on a re-analyze. A
   plugin runs on the next re-analyze, like the other detection settings (params.detector). */
async function pluginsInstalled() {
  try { return (await B.pluginsList(S.sid)).plugins || []; }
  catch (e) { return []; }
}
function pluginFor(list, id) { return list.find(p => p.id === id) || null; }
/* The current detector of the track: the params win (they apply on the next re-analyze),
   else the plugin the track's stored detector record names. */
function trackDetectorParam(name) {
  const t = S.tracks[name], p = (t && t.params) || {};
  if (typeof p.detector === "string") return p.detector === "builtin" ? "" : p.detector;
  const rec = t && t.extra && t.extra.detector;
  return rec && rec.plugin && rec.plugin.id ? rec.plugin.id : "";
}
async function setDetectorFill(name) {   // buildParamsPanel: the options of the Detector select
  const div = $("#insParams"); if (!div || INS.name !== name) return;
  const sel = div.querySelector(".pDetector"); if (!sel) return;
  const t = S.tracks[name], list = await pluginsInstalled();
  if (!t || INS.name !== name) return;
  const sr = t.sr;
  for (const o of [...sel.querySelectorAll("option.plugin")]) o.remove();
  for (const pl of list) {
    if (!pl.roles.includes(t.role)) continue;
    if (sr && !pl.sample_rates.includes(sr)) continue;   // the app passes the file's rate
    const o = document.createElement("option");
    o.value = pl.id; o.textContent = pl.name; o.classList.add("plugin");
    o.title = `plugin ${pl.id} ${pl.version} · ${pl.licence}`;
    sel.appendChild(o);
  }
  const cur = trackDetectorParam(name);
  sel.value = list.some(pl => pl.id === cur) ? cur : "";
  setDetectorNote(name, sel.value);
}
function setDetectorNote(name, id) {   // the sensitivity slider only judges the built-in detector's candidates
  const div = $("#insParams"); if (!div) return;
  const sens = div.querySelector(".pSens"), row = div.querySelector(".pDetectorNoteRow"), note = div.querySelector(".pDetectorNote");
  if (!sens || !row) return;
  const on = !!id;
  sens.disabled = on;
  row.classList.toggle("hidden", !on);
  if (on) note.textContent = `${id}: the hits come from the plug-in; the Sensitivity slider applies to the built-in detector`;
}
let plPending = null;   // the plugin waiting for its licence answer: {name, id, sel}
async function plAskLicence(name, id, sel) {   // the licence dialog before the first use
  const list = await pluginsInstalled();
  const pl = pluginFor(list, id);
  if (!pl) return;
  plPending = { name, id, sel };
  $("#plAccept").classList.remove("hidden");
  $("#plIntro").innerHTML = `<b>${pl.name}</b> (plugin ${pl.id} ${pl.version}) runs <code>${pl.executable}</code>. Licence: ${pl.licence}.`;
  $("#plText").textContent = pl.licence_text || "(no licence text)";
  showModal("#pluginLicenceModal");
}
function plHooks() {
  $("#plAccept").onclick = async () => {
    const pend = plPending; plPending = null;
    $("#pluginLicenceModal").classList.add("hidden");
    if (!pend) return;
    try { await B.pluginAccept(S.sid, pend.id); } catch (e) { setMsg(`plug-in: ${errText(e)}`, "error"); return; }
    pend.sel.value = pend.id;
    const p = S.tracks[pend.name].params || (S.tracks[pend.name].params = {});
    p.detector = pend.id;
    setDetectorNote(pend.name, pend.id);
    setMsg(`${pend.id}: licence accepted, the plug-in runs on the next re-analyze`);
  };
  $("#plCancel").onclick = () => {
    const pend = plPending; plPending = null;
    $("#pluginLicenceModal").classList.add("hidden");
    if (pend) pend.sel.value = trackDetectorParam(pend.name);
  };
}
let pmTargets = null;   // the tracks of a re-analyze waiting for the missing-plugin answer
async function startAnalysis(tracks) {
  if (!S.order.length) { setMsg("no tracks"); return; }
  if (analysisRunning) { setMsg("analysis already running — wait for it to finish"); return; }
  const list = tracks || S.order;
  // #387: a track whose Detector names a plugin that is not installed asks first (the design
  // section 6): install it again, or analyze with the built-in detector.
  const list2 = await pluginsInstalled();
  const missing = list.filter(n => {
    const id = trackDetectorParam(n);
    return id && !pluginFor(list2, id);
  });
  if (missing.length) {
    pmTargets = list;
    $("#pmMsg").innerHTML = missing.map(n => {
      const id = trackDetectorParam(n);
      return `<b>${n}</b>: the plug-in <b>${id}</b> is not installed.`;
    }).join("<br>");
    $("#pluginMissingModal").classList.remove("hidden");
    return;
  }
  startAnalysisGo(list);
}
function pmHooks() {
  $("#pmCancel").onclick = () => { $("#pluginMissingModal").classList.add("hidden"); pmTargets = null; };
  $("#pmBuiltin").onclick = () => {
    const list = pmTargets; pmTargets = null;
    $("#pluginMissingModal").classList.add("hidden");
    if (!list) return;
    for (const n of list) {
      const id = trackDetectorParam(n);
      if (!id) continue;
      const p = S.tracks[n].params || (S.tracks[n].params = {});
      if (p.detector === id) p.detector = "builtin";   // the record keeps naming it
    }
    startAnalysisGo(list);
  };
}
function setPluginsFill() {   // Settings: the installed plugins with id, name, version, roles, licence
  pluginsInstalled().then(list => {
    const box = $("#setPlugins"); if (!box) return;
    if (!list.length) { box.innerHTML = `<span class="phint">No detector plug-ins installed.</span>`; return; }
    box.innerHTML = "";
    for (const pl of list) {
      const d = document.createElement("div");
      d.className = "prow";
      d.innerHTML = `<label title="${pl.executable}">${pl.name} <span class="phint">${pl.id} ${pl.version} · roles ${pl.roles.join(", ")} · ${pl.licence}</span></label>`;
      const view = document.createElement("button");
      view.type = "button"; view.className = "btn mini"; view.textContent = "Licence";
      view.onclick = () => {
        $("#plIntro").innerHTML = `<b>${pl.name}</b> (plugin ${pl.id} ${pl.version}) runs <code>${pl.executable}</code>. Licence: ${pl.licence}.`;
        $("#plText").textContent = pl.licence_text || "(no licence text)";
        plPending = null;
        $("#plAccept").classList.add("hidden");   // a look at the licence is not an acceptance
        showModal("#pluginLicenceModal");
      };
      d.appendChild(view);
      box.appendChild(d);
    }
  });
}
/* ---------------- Settings > Audio output (P3-16) ----------------
   The list of the output devices and "System default" (the default choice); the device in use with its rate and
   channels; the message when the saved device is missing or does not answer (the transport then plays on the
   system default). The choice is kept in the app settings by the service (output_select). */
function fmtDevice(d) { return `${d.name}, ${+(d.rate / 1000).toFixed(1)} kHz, ${d.channels} ch`; }
function renderAudioOutput(a) {
  const sel = $("#setOutput"); sel.innerHTML = "";
  const def = (a.devices || []).find(d => d.default);
  const opt = (value, text) => { const o = document.createElement("option"); o.value = value; o.textContent = text; sel.appendChild(o); };
  opt("", def ? `System default (${def.name})` : "System default");
  for (const d of a.devices || []) opt(d.name, fmtDevice(d));
  if (a.choice && !(a.devices || []).some(d => d.name === a.choice)) opt(a.choice, `${a.choice} (not connected)`);
  sel.value = a.choice || "";
  $("#setOutputNow").textContent = a.current ? `Plays on: ${fmtDevice(a.current)}` :
    `Plays on: ${a.choice && (a.devices || []).some(d => d.name === a.choice) ? a.choice : (def ? def.name : "the system default")} (opens at the next play)`;
  const note = $("#setOutputNote"); note.textContent = a.note || ""; note.classList.toggle("hidden", !a.note);
}
function refreshAudioOutput() {
  return B.audioDevices().then(renderAudioOutput).catch(e => setMsg(`audio devices: ${e.message}`));
}
function chooseAudioOutput(name) {
  return appCmd(() => B.audioOutput(name || null)).then(a => {
    renderAudioOutput(a);
    setMsg(a.note || `audio output: ${name || "system default"}`);
  }).catch(e => setMsg(`audio output failed: ${e.message.replace(/^\d+: /, "")}`, "error"));
}
function openShortcuts() {
  $("#keymapPreset").textContent = `Key preset: ${DSKeys.presetName()} (change it in Drum Spike > Settings).`;
  DSKeys.renderHelp($("#keymapTable"));
  showModal("#helpModal");
}   // P3-13b: made from static/keymap.js

/* ---------------- global handlers ---------------- */
function setupGlobal() {
  $("#playBtn").onclick = () => playStop();
  $("#stopBtn").onclick = () => stopAndReturn();
  $("#analyzeAllBtn").onclick = () => startAnalysis(null);
  $("#exportAllBtn").onclick = () => openExport("all");
  setupExport();
  $("#progressCancel").onclick = async () => { $("#progressCancel").disabled = true; try { await B.jobCancel(S.sid); setMsg("cancelling…"); } catch(e) { setMsg(`cancel failed: ${e.message}`, "error"); } };   // P3-4d
  $("#relinkFind").onclick = findMovedAudio;
  $("#relinkDismiss").onclick = () => { relinkHidden = S.sid; refreshRelinkBar(); };
  $("#relinkClose").onclick = () => $("#relinkModal").classList.add("hidden");
  $("#relinkAgain").onclick = findMovedAudio;
  // P3-13a: the sidebar, the File menu and the ? button are gone; the native menu (shell.js) runs their actions
  $("#settingsDone").onclick = () => $("#settingsModal").classList.add("hidden");
  plHooks();   // #387: the licence and missing-plugin dialogs of the detector plugins
  pmHooks();
  $("#setOutput").onchange = ev => chooseAudioOutput(ev.target.value);
  const kp = $("#setKeyPreset");   // P3-13c
  for (const [value, text] of Object.entries(DSKeys.PRESETS)) { const o = document.createElement("option"); o.value = value; o.textContent = text; kp.appendChild(o); }
  kp.value = DSKeys.preset;
  kp.onchange = ev => setKeyPreset(ev.target.value);
  $("#setZoomTall").onchange = ev => setZoomTall(ev.target.checked);   // P4-27
  $("#setAuditionTab").checked = !!SETTINGS().ui.auditionTab;   // P4-40d
  $("#setAuditionTab").onchange = ev => { SETTINGS().ui.auditionTab = ev.target.checked; saveSettings(); };
  setupMaster();   // P3-16
  $("#tempoDetectBtn").onclick = () => DSShell.menu({id: "detect_tempo"});   // owner 2026-10-14: the same as Track > Detect Tempo
  setupInspector();   // P3-14a
  $("#helpDone").onclick = () => $("#helpModal").classList.add("hidden");
  $("#undoHistDone").onclick = () => $("#undoHistModal").classList.add("hidden");   // P4-40a
  $("#importCancel").onclick = () => $("#importModal").classList.add("hidden");
  // global settings bindings
  const st = SETTINGS();
  $("#setClickOn").checked = st.click.enabled;
  $("#setClickVol").value = Math.round(st.click.volume*100);
  $("#setClickOn").onchange = ev => setClick(ev.target.checked);
  $("#setClickVol").oninput = ev => { st.click.volume = ev.target.value/100; saveSettings(); };
  const pitchBox = $("#setPitches");
  for (const role of ["kick","snare","tom","hat","cymbal","other"]) {
    const row = document.createElement("div"); row.className = "prow";
    const lab = document.createElement("label"); lab.textContent = role[0].toUpperCase()+role.slice(1);
    const ctl = document.createElement("div"); ctl.className = "pctl";
    const inp = document.createElement("input"); inp.type = "number"; inp.value = st.click.pitches[role] || 1500; inp.step = 100; inp.style.width = "90px";
    inp.onchange = () => { st.click.pitches[role] = Math.max(50, Math.min(8000, +inp.value||1500)); saveSettings(); };
    const unit = document.createElement("span"); unit.className = "punit"; unit.textContent = "Hz";
    ctl.append(inp, unit); row.append(lab, ctl); pitchBox.appendChild(row);
  }
  $("#clickBtn").classList.toggle("on", !!st.click.enabled);
  $("#clickBtn").onclick = () => toggleClick();
  $("#followBtn").onclick = () => { const m = ["smooth", "page", "off"]; setFollow(m[(m.indexOf(S.followMode) + 1) % 3]); setMsg(`follow playhead: ${S.followMode}`); };
  $("#snapChk").onchange = ev => setSnap(ev.target.checked);
  $("#raKeep").onclick = () => { if (pendingRA) { const l = pendingRA; pendingRA = null; runAnalysis(l, true); } };
  $("#raDiscard").onclick = () => { if (pendingRA) { const l = pendingRA; pendingRA = null; runAnalysis(l, false); } };
  $("#raCancel").onclick = () => { $("#reanalyzeModal").classList.add("hidden"); pendingRA = null; setMsg("re-analyze cancelled"); };
  document.querySelectorAll(".zBtn").forEach(b => b.onclick = () => zoomPlayhead(b.dataset.z === "in" ? 0.5 : 2));
  $("#zoomSel").onchange = ev => { const v = ev.target.value; ev.target.value = ""; if (v && needProject()) applyZoomSpan(v); };
  $("#ruler").addEventListener("mousedown", rulerDown);
  const ov = $("#overview");
  function ovMode(px) {
    const maxD = maxDur(), W = GEO.ovW;
    const x0 = S.view.t0/maxD*W, x1 = S.view.t1/maxD*W;
    const phX = S.playhead/maxD*W;
    if (Math.abs(px-phX) <= 6) return "scrub";
    if (Math.abs(px-x0) <= 5) return "resizeL";
    if (Math.abs(px-x1) <= 5) return "resizeR";
    return (px > x0 && px < x1) ? "move" : "jump";
  }
  function ovCursor(px) {
    const m = ovMode(px);
    return m==="scrub" ? "col-resize" : (m==="resizeL"||m==="resizeR") ? "ew-resize" : "crosshair";
  }
  ov.addEventListener("mousemove", ev => { ov.style.cursor = ovCursor(ovXOf(ev)); });
  ov.addEventListener("mousedown", ev => {
    if (ev.button!==0) return;
    S.followHold = true;   // F1: any overview interaction is a user view change
    const maxD = maxDur(), W = GEO.ovW;
    const mode = ovMode(ovXOf(ev));
    const startX = ovXOf(ev);
    const t0 = S.view.t0, t1 = S.view.t1;
    if (mode === "scrub") {
      const mv = e2 => { S.playhead = Math.max(0, Math.min(ovXOf(e2)/W*maxD, maxD)); invalidate(F.HEAD); updateStatus(); };
      mv(ev);
      const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
      window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
      return;
    }
    if (mode === "resizeL" || mode === "resizeR") {
      const mv = e2 => {
        const t = Math.max(0, Math.min(ovXOf(e2)/W*maxD, maxD));
        if (mode === "resizeL") setView(Math.min(t, t1-0.002), t1);
        else setView(t0, Math.max(t, t0+0.002));
      };
      const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
      window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
      return;
    }
    if (mode === "jump") centerOn(startX/W*maxD);   // jump, then continue dragging the window
    const anchorT0 = S.view.t0;
    const mv = e2 => {
      const dt = (ovXOf(e2) - startX)/W*maxD;
      // direct manipulation: the rectangle follows the pointer (drag right → view moves right)
      const n0 = Math.max(0, Math.min(anchorT0 + dt, maxD - viewDur()));
      setView(n0, n0 + viewDur());
    };
    const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
  });
  ov.addEventListener("wheel", ev => {
    ev.preventDefault();
    const dY = ev.deltaY * dScale(ev.deltaMode), dX = ev.deltaX * dScale(ev.deltaMode);
    if (Math.abs(dX) > Math.max(2, Math.abs(dY)) || ev.shiftKey) {
      const secPerPx = maxDur() / GEO.ovW;
      panView((dX || dY) * secPerPx);
    } else $("#lanesScroll").scrollBy({ top: dY });
  }, {passive:false});

  window.addEventListener("keydown", ev => {
    if (DSShell.menuOwnsKey(ev)) return;   // P3-4b: the native menu runs this key (app only), so it runs once
    // a field keeps its keys (P3-13a: a check box does not, so J still works after a click on the snap box).
    // #436: a slider gives its keys to the app too (Space plays after a click on a slider); only a text
    // field keeps them, and the status line says when a transport key went into a text field.
    const tg = ev.target;
    const keeps = tg.tagName === "SELECT" || tg.tagName === "TEXTAREA" || tg.isContentEditable
        || (tg.tagName === "INPUT" && !["checkbox", "radio", "button", "range"].includes(tg.type));
    if (keeps) {
      const swallowed = DSKeys.find(ev);   // the key has a command: the field takes it, say so
      if (swallowed && (swallowed.cmd.id === "play_stop" || swallowed.cmd.id === "half_speed")) {
        const what = tg.title || tg.id ? `the ${tg.title || tg.id} field` : "the text field";
        setMsg(`"${DSKeys.display(swallowed.key)}" went to ${what}: press Escape and click the lanes to play`);
      }
      return;
    }
    // P3-13b: the key table (static/keymap.js) says which command the key runs
    const hit = DSKeys.find(ev);
    if (!hit) return;
    if (hit.cmd.id !== "deselect") ev.preventDefault();
    runKey(hit.cmd.id, ev);
  });
}

/* P3-13b: what a key of the table does in the page. The menu items (shell.js handlers) and these functions
   call the same editor commands; a key differs only where the old key did: Delete without a selected hit
   removes the selected track after a confirm (P4-39a), the nudge keys pan the view when no hit is selected, and Escape deselects. A command that is
   not here (a menu key in the web page, where no native menu runs it) runs its menu handler. */
function nudgeOrPan(step) {   // step in units of the nudge value (P3-20a); a range selection nudges its hits (#151)
  if (LN.sel || S.sel) nudgeUnits(step);
  else panView(Math.sign(step) * viewDur() * 0.2);
}
const KEY_RUN = {
  next_hit: () => gotoEvent(1, false),
  prev_hit: () => gotoEvent(-1, false),
  next_hit_all: () => gotoEvent(1, true),
  prev_hit_all: () => gotoEvent(-1, true),
  review_next: () => reviewStep(1),
  review_prev: () => reviewStep(-1),
  other_mics: () => mxToggle(),   // #157: View > Other Mics Under Hit (O)
  zoom_in: () => zoomStep(true),
  zoom_out: () => zoomStep(false),
  zoom_full: () => zoomPreset(0),
  zoom_30s: () => zoomPreset(1),
  zoom_5s: () => zoomPreset(2),
  zoom_500ms: () => zoomPreset(3),
  zoom_50ms: () => zoomPreset(4),
  zoom_5ms: () => zoomPreset(5),
  play_stop: ({ key }) => playStop(key),
  half_speed: ({ key }) => halfSpeedPlayStop(key),   // P3-20b
  go_start: () => goTo(0),
  go_end: () => goTo(projectEnd()),
  go_play_start: () => goTo(S.playStartPos),
  click: () => toggleClick(),
  snap: () => toggleSnap(),
  follow_toggle: () => toggleFollow(),
  mute_track: () => trackButton(".bMute"),
  solo_track: () => trackButton(".bSolo"),
  delete_hit: () => { if (!tmDeleteSel() && !lnDeleteMark()) { if (LN.sel || S.sel) deleteSelectedHit(); else if (S.selTrack && S.tracks[S.selTrack]) removeTrack(S.selTrack); } },   // P4-39a: no hit, no marker: the selected track (after the confirm)   // P4-6a: a selected Learn mark bar too   // P4-8 and P4-40g: a selected tempo, meter or review marker first   // #151: a range selection deletes its hits
  markers_set: () => markerAtSelection(),   // P4-40g: M sets a marker at the selected hit or the playhead
  marker_next: () => markerJump(1),   // P4-40g: Option+M
  marker_prev: () => markerJump(-1),   // P4-40g: Option+Shift+M
  nudge_earlier: () => nudgeOrPan(-1),
  nudge_later: () => nudgeOrPan(1),
  nudge_earlier_10: () => nudgeOrPan(-10),
  nudge_later_10: () => nudgeOrPan(10),
  deselect: () => { if (auStop()) return; lnClearSel(); if (LN.mark) lnSetMark(null); S.sel = null; if (TM.sel) { TM.sel = null; invalidate(F.VIEW); } requestFrame(); updateStatus(); },   // the frame redraws the track of the old selection
  undo: () => undo(),
  redo: () => redo(),
  prev_track: () => selectTrackStep(-1),   // P3-19
  next_track: () => selectTrackStep(1),
  zoom_toggle: () => zoomToggle(false),
  zoom_keep: () => zoomToggle(true),
  nudge_value_up: () => stepNudgeValue(1),   // P3-20a
  nudge_value_down: () => stepNudgeValue(-1),
  save: () => saveProjectNow().catch(() => {}),   // #249 R02: the refusal is already on the status line
  audition: ({ shift }) => audition(shift),   // P4-40d
};
function runKey(id, ev) {
  const f = KEY_RUN[id] || DSShell.handlers[id];
  if (!f) return false;
  try {
    const r = f({ id, shift: !!(ev && ev.shiftKey), key: ev ? DSKeys.keyOf(ev) : null });
    if (r && typeof r.catch === "function") r.catch(e => setMsg(`${id} failed: ${e.message}`, "error"));
  } catch (e) { setMsg(`${id} failed: ${e.message}`, "error"); }
  return true;
}

function rulerDown(ev) {   // the time ruler and the Bars|Beats ruler (P4-8): a click puts the playhead
  if (ev.button!==0) return;
  ev.preventDefault();
  S.followHold = true;   // F1: repositioning the playhead is a user intent
  const move = e2 => { S.playhead = Math.max(0, snapPlayhead(Math.min(xToT(rulerXOf(e2), GEO.rulerW), maxDur()))); invalidate(F.HEAD); updateStatus(); };
  move(ev);
  const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
  window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
}

/* ---------------- boot ---------------- */
(async function boot() {
  persistInit({ edIdle });   // R17 step 2: persist waits for the edit queue through the injected hook (edit.js owns edIdle since step 3)
  editInit({ pushEvents, voidHitDrag: name => { if (hitDrag && hitDrag.track === name) hitDrag = null; } });   // R17 step 3: edit.js flushes the writes of other steps through the injected hook; voidHitDrag is the input layer's (a replaced hit list voids an in-flight drag)
  sessionInit({ GLQ, lnReset, auReset, tmLoad, selMem: () => selMem, setSelMem: v => { selMem = v; }, exTempoReset: () => { EX.tempoAck = false; } });   // R17 step 6: session.js resets the module state of the owners a load replaces (the GL tile queue, LN/AU/TM, the selection memory) through the injected hooks; #151: the default-tempo answer is per project
  tmInit(); measureGeo(); geoObserve();   // P4-8: the map rulers; P3-14d: the geometry once now, then from the ResizeObserver
  setupUpload(); setupGlobal(); setupInfoLine(); staleSetup();
  if (B.onDirty) {   // P3-4a: the service tells when a project becomes dirty or clean
    let listTimer = null;
    B.onDirty(p => {
      if (p.sid === S.sid) setDirtyState(p.dirty);
      clearTimeout(listTimer);   // one refresh of the project list (its dots) for a burst of events
      listTimer = setTimeout(() => initSessions(true).catch(() => {}), 100);   // P3-4d: keepNone, else a late event reopens a project the user closed
    });
  }
  try {
    await loadViewSettings();   // P3-14a: the Inspector's state, before the first layout the user sees
    await initSessions();
    await loadProject();
    showBackend();
  } catch(e) {
    setMsg(`init failed: ${e.message}`, "error");
  }
  await DSShell.boot();   // P3-4b: menu events, Finder paths (app only)
  B.appReady().catch(() => {});   // P3-17: the first layout is done; the shell shows the window and closes the splash
})();
