/* Drum Spike v2 — export (refactor plan step 5.2, R9 A2)
   The export messages, the spike-file error detail, the Export dialog (EX), Export Again and
   setupExport. Moved verbatim from app.js. Plain script (no build step), loaded after shell.js
   and before app.js. The names stay top-level bindings (page globals): the e2e tests read EX. */
"use strict";

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
function setMsgDetail(detail) { $("#stMsg").title = trackText(detail || ""); }   // the core's text behind a clear message
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
  const o = { tracks: exTrackSets().names, formats: exFormats(), midi_mode: EX.mode, compensate: hostMode() ? false : EX.comp };
  if (withDest) o.dest = EX.dest;
  return o;
}
/* #579 R1: in a DAW a call to the plugin that does not answer must not leave a tile at "preparing…": after EX.waitMs (30 s) the
   call fails with a reason, and the tile shows the error state. Host mode only; EX.waitMs is a hook for the tests. */
function exWait(p) {
  if (!hostMode()) return p;
  return new Promise((ok, no) => {
    const t = setTimeout(() => no(new Error("no answer from the plugin")), EX.waitMs || 30000);
    p.then(v => { clearTimeout(t); ok(v); }, e => { clearTimeout(t); no(e); });
  });
}
const exPlural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
/* #575: in a DAW the destination row is collapsed: the files go to the cache folder of the document and the user drags them, so the folder is
   rarely the point. One click on the button opens the row (the folder, Choose, Reset). A folder that was chosen, or a note about the folder,
   keeps the row open. Once the user opened the row (the button, Choose or Reset) it stays open until the dialog closes: a Reset
   that makes the folder the default again must not collapse the row under the focused button. With the mode off the button stays hidden and the row shows as before. */
function exDestFold() {
  const sec = $("#exDestSec"), btn = $("#exDestToggle");
  if (!hostMode()) { sec.classList.remove("exFold", "open"); btn.classList.add("hidden"); return; }
  if (EX.destOpen === undefined) EX.destOpen = false;
  const forced = EX.dest !== EX.def || !!EX.destNote;
  const open = EX.destOpen || forced;
  sec.classList.add("exFold"); sec.classList.toggle("open", open);
  btn.classList.toggle("hidden", open); btn.setAttribute("aria-expanded", String(open));
  btn.title = `The files go to ${EX.def || "the folder of the document"}`;   // R2: the default folder is visible without a click
}
function exRender() {   // the controls from EX; the Export button and the tiles follow
  const { names, skipped } = exTrackSets(), all = S.order.length, sel = EX.selected && S.tracks[EX.selected] ? 1 : 0;
  $("#exScopeAll").checked = EX.scope === "all"; $("#exScopeSel").checked = EX.scope === "selected";
  $("#exAllText").textContent = `All tracks (${all})`;
  $("#exSelText").textContent = sel ? `Selected track (1): ${trackLabel(EX.selected)}` : "Selected track (none selected)";
  $("#exScopeSel").disabled = !sel; $("#exScopeSel").closest(".exRow").classList.toggle("off", !sel);
  $("#exSkipped").textContent = skipped.length ? `Skipped: ${skipped.map(k => `${trackLabel(k.name)} (${k.why})`).join(", ")}` : "";
  $("#exMidi").checked = EX.midi; $("#exSpikes").checked = EX.spikes;
  $("#exMarkers").checked = EX.markers;
  $("#exModeTrack").checked = EX.mode === "per_track"; $("#exModeSingle").checked = EX.mode === "single";
  $("#exSpikeSynth").checked = EX.spikeType === "synthetic"; $("#exSpikeAudio").checked = EX.spikeType === "audio"; $("#exSpikeBoth").checked = EX.spikeType === "both";
  for (const id of ["#exSpikeSynth", "#exSpikeAudio", "#exSpikeBoth"]) { $(id).disabled = !EX.spikes; $(id).closest(".exRow").classList.toggle("off", !EX.spikes); }
  const cal = S.order.filter(n => smOf(n));   // P4-7c: the tracks with a sampler calibration
  $("#exComp").checked = EX.comp; $("#exComp").disabled = !EX.midi; $("#exComp").closest(".exRow").classList.toggle("off", !EX.midi);
  $("#exComp").closest(".exRow").title = cal.length ? `The MIDI notes of ${cal.map(trackLabel).join(", ")} move by the sampler offset (the aligned attack minus the offset)` : "No track has a sampler calibration yet (Inspector > Layer > Calibration MIDI…)";
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
    if (hostMode()) {   // #579: a tile that cannot prepare shows the reason; it never stays at "preparing…"
      const why = t.dataset.state === "error" ? `error: ${EX.drag.why || "the files could not be prepared"}` : "";
      if (why) t.querySelector(".exDragN").textContent = why.length > 60 ? why.slice(0, 57) + "…" : why;
      t.querySelector(".exDragN").title = why;
    }
  }
  /* #151: on the default map the Drag MIDI tile shows the visible 120 BPM warning until the project's answer */
  $("#exDragMidi .exDragW").classList.toggle("hidden", !(EX.midi && EX.tempoDefault && !EX.tempoAck));
  exHostNote();
  exDestFold();
}
/* #560: in a DAW the dialog says where the tempo of the MIDI comes from, how many exported tracks have a time-stretched region (the
   files are made all the same) and where to drop the files (#575: the session start). With the mode off nothing here shows. */
function exHostNote() {
  const el = $("#exHostNote"), warn = $("#exHostWarn");
  if (!hostMode()) { el.classList.add("hidden"); warn.classList.add("hidden"); return; }
  /* the tempo of the host changed while it played and the file has one tempo: a clear line, no block */
  const varies = EX.midi && EX.hostTempo && EX.hostTempo.source === "transport-varies";
  warn.textContent = varies ? "The host's tempo changes, but the MIDI file has one tempo: the bar positions of the MIDI file can be wrong. The times in seconds are right." : "";
  warn.classList.toggle("hidden", !varies);
  const h = EX.hostTempo || { source: "none" }, parts = [];
  if (EX.midi) parts.push(h.source === "ara" ? `MIDI tempo: the tempo map of the host (${h.steps + 1} tempo${h.steps ? "s" : ""})` :
    h.source === "transport" ? `MIDI tempo: one tempo from the host, ${Math.round(h.bpm * 100) / 100} BPM, no tempo map` :
    h.source === "transport-varies" ? `MIDI tempo: the host's tempo changes, the file has one tempo, ${Math.round(h.bpm * 100) / 100} BPM` :
    "MIDI tempo: none from the host, the file has 120 BPM");
  const names = exTrackSets().names, st = (h.stretched || []).filter(n => names.includes(n));
  if (st.length) parts.push(`${st.length} exported track${st.length === 1 ? " has" : "s have"} a region that the host stretches in time: the hits follow the stretch as one linear factor; warp markers inside a clip are not followed`);
  parts.push("Files start at song position 0: drop them at the start of the session in your DAW");   // #575: where to drop them; no development note
  el.textContent = parts.join(". ") + ".";
  el.classList.remove("hidden");
}
async function exInfo() {   // the replace count of the current options (the newest answer wins)
  const my = ++EX.infoSeq;
  if (!exFormats().length || !exTrackSets().names.length) { EX.replace = 0; exRender(); return; }
  try {
    const r = await exWait(B.exportExisting(S.sid, exOpts(true)));
    if (my !== EX.infoSeq || !EX.open) return;
    EX.replace = r.count; EX.destOk = true; EX.destNote = EX.destNote.startsWith("The folder") ? EX.destNote : "";
    if (hostMode() && EX.drag.byDest) { exRender(); exDragSchedule(true); return; }   // #579: the folder works now: the tiles prepare again
  } catch (e) {
    if (my !== EX.infoSeq || !EX.open) return;
    EX.destOk = false; EX.destNote = `The folder cannot be used: ${errText(e)}`; EX.replace = 0;
    if (hostMode()) { clearTimeout(EX.timer); EX.dragSeq++; EX.drag = { midi: null, spikes: null, failed: true, why: EX.destNote, byDest: true }; }   // #579: the tiles say why, they do not wait
  }
  exRender();
}
function exDragSchedule(now) {   // the tiles show "preparing…" until the files of the new options are there
  clearTimeout(EX.timer);
  const my = ++EX.dragSeq;
  EX.drag = { midi: null, spikes: null, failed: false };
  exRender();
  if (!exFormats().length || !exTrackSets().names.length) return;
  if (!EX.destOk) {
    if (hostMode()) { EX.drag = { midi: null, spikes: null, failed: true, why: EX.destNote || "the folder cannot be used", byDest: true }; exRender(); }   // #579
    return;
  }
  EX.timer = setTimeout(() => {
    EX.dragChain = EX.dragChain.then(async () => {   // one render at a time: two would clear each other's files
      if (my !== EX.dragSeq || !EX.open) return;
      try {
        await flushEvents();
        const r = await exWait(B.exportDragPrepare(S.sid, exOpts(false)));
        exportedFollowers(r);   // P4-4: the render derives stale followers like the export it previews; the page takes the new hits even when the export then finds everything fresh
        if (my !== EX.dragSeq || !EX.open) return;
        EX.drag = { midi: r.midi, spikes: r.spikes, failed: false };
        /* #579: in a DAW a kind with no file and an error from the render is an error, not "ready" with no file */
        const bad = hostMode() ? (r.errors || []).map(x => x && x.message || String(x)).join("; ") : "";
        if (bad && ((EX.midi && !(r.midi || []).length) || (EX.spikes && !(r.spikes || []).length))) EX.drag = { midi: null, spikes: null, failed: true, why: bad };
      } catch (e) { if (my === EX.dragSeq) EX.drag = { midi: null, spikes: null, failed: true, why: errText(e) }; }
      if (EX.open) exRender();
    });
  }, now ? 0 : 400);
}
function exChanged() { $("#exResult").classList.add("hidden"); exRender(); exInfo(); exDragSchedule(); exTempoDefault(); }
/* #151: the default-ness of the map drives the visible 120 BPM warning of the Drag MIDI tile; the newest answer wins */
async function exTempoDefault() {
  const my = ++EX.tempoSeq, sid = S.sid;
  try {
    if (hostMode()) {   // #560: the tempo comes from the host (its timeline, or one tempo of its process call)
      const h = await B.hostTempo();
      if (my === EX.tempoSeq) { EX.hostTempo = h; EX.tempoDefault = h.source === "none"; exRender(); }
      return;
    }
    const d = !!(await B.tempoMapGet(sid)).default; if (my === EX.tempoSeq) { EX.tempoDefault = d; exRender(); }
  }
  catch (e) { if (my === EX.tempoSeq) EX.tempoDefault = false; }
}
async function openExport(scope, track) {
  if (!needProject()) return;   // #560: in a DAW the dialog works (files in song time, a folder sheet, the drag tiles)
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
  if (typeof info.export_compensate === "boolean") EX.comp = info.export_compensate;   // #232: a kept "Compensate" choice seeds the dialog; null: the calibration default above
  const last = info.export_last;   // P4-31: the project keeps what its last export wrote with (and every choice, recorded at once)
  if (last) {
    EX.midi = last.formats.includes("midi"); EX.spikes = last.formats.includes("spikes") || last.formats.includes("audio");
    EX.spikeType = last.formats.includes("spikes") && last.formats.includes("audio") ? "both" : last.formats.includes("audio") ? "audio" : "synthetic";   // #218: the spike type the project keeps
    EX.mode = last.midi_mode === "single" ? "single" : "per_track";   // P4-31: the choices the project keeps seed the dialog; compensate keeps its default (a recorded off was the default of an earlier export, not a choice)
  }
  if (hostMode()) { EX.comp = false; EX.markers = false; }   // #560: the sampler offset and markers are absent in a DAW (#579: the audio spike types are there, in song time)
  EX.dest = EX.def;
  if (EX.remembered) {   // the folder of the last choice, when it can still be used
    try { await exWait(B.exportExisting(S.sid, { dest: EX.remembered })); EX.dest = EX.remembered; }
    catch (e) {
      if (hostMode()) {   // #579: a remembered folder that is gone (or not allowed any more): the default folder and one short text, no error
        EX.destNote = "The folder chosen before is not available: Exports/ is used.";
        EX.remembered = null;
        B.exportDestSet(S.sid, null).catch(() => {});
      } else EX.destNote = `The folder kept for this project cannot be used (${errText(e)}): Exports/ is used.`;
    }
  }
  EX.open = true; EX.destOpen = false;   // #575: a DAW starts with the destination row collapsed
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
  EX.destOpen = true;   // #575: the user is in the row
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
  EX.destOpen = true;   // #575: the row stays open under the focused Reset button
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
  /* #441: the stated bound of the MIDI files, when the tick grid is coarser than a sample */
  const bound = typeof r.worst_error_samples === "number" && r.worst_error_samples > 0.5
    ? `; MIDI is within ${Math.ceil(r.worst_error_samples * 100) / 100} samples at this tempo and rate; the spike file is sample exact`
    : "";
  return `exported ${what} to ${where} (${(r.elapsed_ms / 1000).toFixed(1)} s)` + bound;
}
/* #151: the default-tempo notice, shared by the Export dialog and File > Export Again: a MIDI export of a
   project whose tempo map is the default map would use 120 BPM. "anyway" lets the export write; "set_tempo"
   opens Track > Detect Tempo without writing; "cancel" writes nothing. A tempo_map_get that fails never asks. */
async function askDefaultTempo() {
  let def = false;
  try { def = hostMode() ? (await B.hostTempo()).source === "none" : !!(await B.tempoMapGet(S.sid)).default; } catch (e) { return "anyway"; }
  EX.tempoDefault = def;   // the same answer drives the visible warning of the Drag MIDI tile
  if (!def) return "anyway";
  if (hostMode()) return await askConfirm({ title: "No tempo from the host",   // #560: the host gave no tempo; the tempo cannot be set here
    msg: "The host gave no tempo, so the MIDI file has the default 120 BPM. Export anyway, or set the tempo in the host first.",
    buttons: [{ label: "Cancel", act: "cancel" }, { label: "Export anyway", act: "anyway", cls: "primary" }] });
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
    if (a !== "anyway") { EX.busy = false; setMsg("export cancelled"); if (a === "set_tempo") detectTempo(); if (EX.open) exRender(); return; }   // no file is written; #478: the render takes the Export button back — a render that landed while the notice held busy disabled it (line 79) and only a render takes that back
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
    const song = Object.values(r.outside_song || {}).reduce((a, c) => [a[0] + c.no_region, a[1] + c.before_start], [0, 0]);   // #560: the hits the song time left out
    const msg = exSummary(r) + (bad.length ? `; ${bad.length} failed (${trackLabel(bad[0].name)}: ${bad[0].message})` : "") +
      (out.length ? `; hits outside the audio file, not in the spike files: ${out.join(", ")} (the MIDI files keep them; a hit before the start sits at 0 s there)` : "") +
      (song[0] + song[1] ? `; hits left out of the files: ${song[0]} in no playback region, ${song[1]} before the song start` : "") +
      exSamplerNote(r) +
      ((r.also_updated || []).length ? `. Also updated: ${r.also_updated.length} older spike file(s) in this folder` : "");   // #274
    setMsg(msg);
    exportedFollowers(r);
    setMsgDetail(bad.map(e => `${trackLabel(e.name)}: ${e.message}` + (e.detail ? `\n  (${e.detail})` : "")).join("\n"));
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
  if (kind === "midi" && !EX.tempoAck && !hostMode()) {   // #560: in a DAW the tile shows the warning; no question between the press and the drag
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
  if (hostAbsent("files")) return;
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
  $("#exComp").onchange = async ev => {   // #232: the checkbox is remembered per project (like the markers one)
    if (hostAbsent("files")) return;   // #560: the row is hidden in a DAW (no sampler offset there)
    const on = ev.target.checked;
    EX.comp = on;
    try { await B.exportCompensateSet(S.sid, on); setDirtyState(true); }
    catch (e) { setMsg(`the Compensate choice failed: ${errText(e)}`, "error"); }
    exChanged(); exRecord();
  };
  $("#exMarkers").onchange = async ev => {   // P4-40g: the checkbox is remembered per project
    if (hostAbsent("files")) return;   // #560: the row is hidden in a DAW (no markers in a song-time file)
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
  $("#exDestToggle").onclick = () => { EX.destOpen = true; exDestFold(); $("#exChoose").focus(); };   // #575
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
