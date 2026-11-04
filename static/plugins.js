/* Drum Spike v2 — detector plug-ins and audio output (refactor plan step 5.2, R9 A2)
   The installed detector plug-ins (#387): the Detector select of the Inspector, the licence dialog, the
   missing-plug-in ask of startAnalysis and the Settings list; and Settings > Audio output (P3-16). Moved
   verbatim from app.js. Plain script (no build step), loaded after analysis.js and before app.js. The
   names stay top-level bindings (page globals). */
"use strict";

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
