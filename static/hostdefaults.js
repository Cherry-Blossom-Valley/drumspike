/* Drum Spike - user defaults and presets of the plugin (DAW plugin M1d3, #577)
   The design is docs/design/daw-plugin/m1d3-defaults.md. Three levels: built-in values, the defaults file of the plugin, the session.
   The file applies to NEW tracks only (the plugin does that, plugins/ara/src/service.rs); this page does three things in a DAW:
   - Inspector: "Save as default for new <role> tracks" and "Reset to default for new <role> tracks". Both act on the ROLE of the selected row
     and never change a track that exists.
   - Menu (Track): "Load Preset..." and "Save Preset...". A preset changes the tracks of the document only after the user confirmed, as ONE undo step.
   - The page options of the file (`follow`) for a page that has none of its own.
   Every element is made here, in JS, and only in host mode: with the mode off hostDefaultsInit() and the other entry points return at once, so
   the standalone page has no new element, nothing focusable and no new text (owner rule 2026-11-14). The plugin owns the files (B.defaults*,
   B.preset*); the standalone app has no such command and never reads or writes them. */
"use strict";

const HD = { data: null, box: null };
const HD_PARAM_KEYS = ["thresholds", "min_gap_ms", "refine", "vel_lo_db", "vel_hi_db", "vel_cut_db"];
const HD_ROLES = ["kick", "snare", "tom", "hat", "cymbal", "other"];

async function hdLoad() {
  if (!hostMode() || !B.defaultsGet) return null;
  try { HD.data = await B.defaultsGet(); } catch (e) { HD.data = null; }
  return HD.data;
}

/* The word of a role in a text: "snare tracks", and "tracks with no role" for other. */
function hdWord(role) { return role === "other" ? "tracks with no role" : `${role} tracks`; }

/* The rows of the document (one per DAW track) in the order of the DAW: the home source of each row. */
function hdRows() {
  const seen = new Set(), out = [];
  for (const k of S.order || []) {
    const home = HT.homeFor(k);
    if (!S.tracks[home] || seen.has(home)) continue;
    seen.add(home); out.push(home);
  }
  return out;
}

/* The notes of all rows of a role in the order of the DAW, no note twice, at most 8. */
function hdRoleNotes(role) {
  const out = [];
  for (const h of hdRows()) { const t = S.tracks[h]; if (t.role === role && Number.isInteger(t.note) && !out.includes(t.note)) out.push(t.note); }
  return out.slice(0, 8);
}

function hdParamsOf(t) {
  const p = {};
  for (const k of HD_PARAM_KEYS) if (t.params && t.params[k] !== undefined && t.params[k] !== null) p[k] = JSON.parse(JSON.stringify(t.params[k]));
  return p;
}

/* ---------------- the Inspector ---------------- */
function hdInspectorSync() {
  if (!hostMode()) return;
  const name = INS.name, t = name && S.tracks[name];
  if (!HD.box) {
    const anchor = document.getElementById("insRole");
    if (!anchor) return;
    const box = document.createElement("div");
    box.id = "hdInsBox"; box.className = "insRow";
    const save = document.createElement("button"), reset = document.createElement("button");
    for (const b of [save, reset]) { b.type = "button"; b.className = "btn mini"; }
    save.id = "hdSaveBtn"; reset.id = "hdResetBtn";
    save.onclick = () => hdSaveRole(); reset.onclick = () => hdResetRole();
    box.append(save, reset);
    anchor.parentNode.insertBefore(box, anchor.nextSibling);
    HD.box = box;
  }
  const save = document.getElementById("hdSaveBtn"), reset = document.getElementById("hdResetBtn");
  if (!t) { HD.box.classList.add("hidden"); return; }
  HD.box.classList.remove("hidden");
  const role = t.role || "other", word = hdWord(role), notes = hdRoleNotes(role);
  const why = t.follows ? "This track takes its hits from another track." : analysisRunning ? "An analysis is running." : HD.data && HD.data.status === "newer" ? "The defaults file is from a newer version of the plugin." : "";
  save.textContent = `Save as default for new ${word}${notes.length ? ": " + notes.join(", ") : ""}`;
  save.title = why || `New ${word} in every session start with the MIDI notes and the detection settings of the ${role} tracks of this session. Tracks that exist keep theirs.`;
  reset.textContent = `Reset to default for new ${word}`;
  reset.title = why || `New ${word} start with the built-in values again. Tracks that exist keep theirs.`;
  save.disabled = reset.disabled = !!why;
}

async function hdSaveRole() {
  const t = S.tracks[INS.name]; if (!t || !hostMode()) return;
  const role = t.role || "other", word = hdWord(role), notes = hdRoleNotes(role);
  try {
    await B.defaultsSaveRole(role, notes, hdParamsOf(t));
    await hdLoad();
    setMsg(`Saved: new ${word} start with ${notes.length > 1 ? "the notes " + notes.join(", ") : "note " + notes[0] + " (" + nameFromNote(notes[0]) + ")"} and these detection settings.`);
  } catch (e) { setMsg(`error: ${errText(e)}`, "error"); }
  hdInspectorSync();
}

async function hdResetRole() {
  const t = S.tracks[INS.name]; if (!t || !hostMode()) return;
  const role = t.role || "other", word = hdWord(role);
  try {
    await B.defaultsResetRole(role);
    await hdLoad();
    setMsg(`New ${word} start with the built-in values.`);
  } catch (e) { setMsg(`error: ${errText(e)}`, "error"); }
  hdInspectorSync();
}

/* ---------------- the preset dialogs ---------------- */
function hdModal() {
  let m = document.getElementById("hdModal");
  if (m) return m;
  m = document.createElement("div"); m.id = "hdModal"; m.className = "modal hidden";
  m.innerHTML = `<div class="modal-box"><h3 id="hdTitle"></h3><p id="hdHint" class="tdim"></p><div id="hdBody"></div>
    <div class="modal-actions"><button id="hdCancel" class="btn" type="button">Cancel</button><button id="hdOk" class="btn primary" type="button">OK</button></div></div>`;
  document.body.appendChild(m);
  m.querySelector("#hdCancel").onclick = () => hdClose();
  return m;
}
function hdClose() { const m = document.getElementById("hdModal"); if (m) { m.classList.add("hidden"); document.removeEventListener("keydown", m._esc, true); } }

/* The plan of a load: for each row whose role the preset has, the new note (the i-th row of a role takes the i-th note of the list, the last
   one when the list is used up) and the keys of the detection settings that differ. Nothing is written. */
function hdPlan(preset) {
  const items = [], seen = {};
  for (const h of hdRows()) {
    const t = S.tracks[h], role = t.role || "other", e = preset.roles && preset.roles[role];
    if (!e) continue;
    const i = seen[role] = (seen[role] || 0);
    seen[role]++;
    const to = e.notes && e.notes.length ? e.notes[Math.min(i, e.notes.length - 1)] : null;
    const item = { home: h, srcs: HT.srcsOf(h).filter(s => S.tracks[s]), role, note: null, params: null };
    if (to !== null && t.note !== to) item.note = { from: t.note, to };
    if (e.params) {
      const keys = Object.keys(e.params).filter(k => JSON.stringify((t.params || {})[k]) !== JSON.stringify(e.params[k]));
      if (keys.length) item.params = { keys, set: e.params };
    }
    if (item.note || item.params) items.push(item);
  }
  return items;
}

function hdPlural(n, one, many) { return `${n} ${n === 1 ? one : many}`; }

async function hdLoadPreset(preset) {
  const items = hdPlan(preset);
  if (!items.length) { setMsg(`Preset "${preset.name}" changes no track.`); return; }
  const nNote = items.filter(i => i.note).length, nPar = items.filter(i => i.params).length, parts = [];
  if (nNote) parts.push(`the MIDI note of ${hdPlural(nNote, "track", "tracks")}`);
  if (nPar) parts.push(`the detection settings of ${hdPlural(nPar, "track", "tracks")}`);
  if (HD.fault !== "no-confirm") {
    const act = await askConfirm({ title: "Load Preset", msg: `Load preset "${preset.name}"? ${hdPlural(items.length, "track changes", "tracks change")}: ${parts.join(", ")}. The hits stay as they are; Re-analyze uses the new settings. You can undo this in one step.`,
      buttons: [{ label: "Cancel", act: "cancel" }, { label: "Load", act: "load", cls: "primary" }] });
    if (act !== "load") return;
  }
  const sid = S.sid, steps = [];
  const undoDone = async () => {   // all or nothing: a refused step takes the steps before it back
    for (const s of steps.reverse()) { try { await applyOp(s, true); } catch (e) { /* the message below says that the load stopped */ } }
  };
  for (const it of items) {
    for (const src of it.srcs) {
      const t = S.tracks[src];
      try {
        if (it.note) {
          await B.setNote(sid, src, it.note.to);
          steps.push({ type: "note", sid, track: src, before: t.note, after: it.note.to });
          t.note = it.note.to;
        }
        if (it.params) {
          const before = JSON.parse(JSON.stringify(t.params || {})), after = Object.assign({}, before, JSON.parse(JSON.stringify(it.params.set)));
          await B.paramsSet(sid, src, after);
          steps.push({ type: "params", label: "Detection setting", sid, track: src, before: { params: before }, after: { params: after } });
          t.params = after;
        }
      } catch (e) {
        await undoDone();
        setMsg(`error: the preset was not loaded (${errText(e)}); the tracks are as they were`, "error");
        return;
      }
    }
  }
  if (HD.fault === "two-steps") for (const s of steps) pushUndo(s); else pushGroup(`Load Preset (${preset.name})`, steps);
  for (const it of items) for (const src of it.srcs) refreshHeadNote(src);
  if (INS.name) syncInspector(true);
  setDirtyState(true);
  setMsg(`Loaded preset "${preset.name}": ${hdPlural(items.length, "track", "tracks")} changed. You can undo this in one step.`);
}

async function hdOpenLoad() {
  if (!hostMode() || !S.sid) return;
  const d = await hdLoad();
  const m = hdModal();
  m.querySelector("#hdTitle").textContent = "Load Preset";
  m.querySelector("#hdHint").textContent = d && d.status === "nofolder" ? "The plugin has no folder for presets here." : "Choose a preset. Nothing changes until you confirm.";
  const body = m.querySelector("#hdBody"); body.textContent = "";
  m.querySelector("#hdOk").classList.add("hidden");
  for (const p of (d && d.presets) || []) {
    const row = document.createElement("div"); row.className = "insRow"; row.dataset.preset = p.name;
    const load = document.createElement("button"); load.type = "button"; load.className = "btn"; load.textContent = p.name; load.dataset.act = "load";
    load.onclick = () => { hdClose(); hdLoadPreset(p); };
    row.appendChild(load);
    if (!p.builtin) {
      const del = document.createElement("button"); del.type = "button"; del.className = "btn mini"; del.textContent = "Delete"; del.dataset.act = "delete";
      del.title = `Delete the preset "${p.name}"`;
      del.onclick = async () => {
        hdClose();
        const act = await askConfirm({ title: "Delete Preset", msg: `Delete preset "${p.name}"? The file is removed.`, buttons: [{ label: "Cancel", act: "cancel" }, { label: "Delete", act: "delete", cls: "primary" }] });
        if (act === "delete") { try { await B.presetDelete(p.name); setMsg(`Deleted preset "${p.name}".`); } catch (e) { setMsg(`error: ${errText(e)}`, "error"); } }
        hdOpenLoad();
      };
      row.appendChild(del);
    }
    body.appendChild(row);
  }
  showModal("#hdModal");
  m._onclose = () => {};
  const first = body.querySelector("button"); if (first) first.focus();
}

function hdOpenSave() {
  if (!hostMode() || !S.sid) return;
  const m = hdModal();
  m.querySelector("#hdTitle").textContent = "Save Preset";
  m.querySelector("#hdHint").textContent = "Saves the MIDI note and the detection settings of the first track of each role. Type a name.";
  const body = m.querySelector("#hdBody"); body.textContent = "";
  const field = document.createElement("input"); field.type = "text"; field.id = "hdName"; field.maxLength = 80; field.autocomplete = "off"; field.spellcheck = false;
  field.setAttribute("aria-label", "Preset name");
  const err = document.createElement("div"); err.id = "hdErr"; err.className = "tdim";
  body.append(field, err);
  const ok = m.querySelector("#hdOk"); ok.classList.remove("hidden"); ok.textContent = "Save";
  const go = async () => {
    const roles = {};
    for (const h of hdRows()) {
      const t = S.tracks[h], role = t.role || "other";
      if (roles[role]) continue;
      roles[role] = { notes: Number.isInteger(t.note) ? [t.note] : [], params: hdParamsOf(t) };
    }
    try { await B.presetSave(field.value, roles); hdClose(); setMsg(`Saved preset "${field.value.trim()}".`); }
    catch (e) { err.textContent = errText(e); }
  };
  ok.onclick = go;
  field.onkeydown = ev => { if (ev.key === "Enter") { ev.preventDefault(); go(); } ev.stopPropagation(); };
  showModal("#hdModal");
  field.focus();
}

/* ---------------- the page options of the file ---------------- */
function hdApplyFollow(d) {
  const f = d && d.follow; if (!f) return;
  let have = false;
  try { have = localStorage.getItem("dsg_settings") !== null; } catch (e) { /* no storage */ }
  if (f.mode && S.followMode !== f.mode && !have) setFollow(f.mode);
  if (typeof f.suspend_on_edit === "boolean" && !have) SETTINGS().ui.suspendFollowOnEdit = f.suspend_on_edit;
  let sel = null;
  try { sel = localStorage.getItem("dsHostFollowSel"); } catch (e) { /* no storage */ }
  if (typeof f.host_selection === "boolean" && sel === null && typeof DSHostSel !== "undefined" && DSHostSel.follows() !== f.host_selection) DSHostSel.setFollow(f.host_selection);
}

async function hostDefaultsInit() {
  if (!hostMode() || !B.defaultsGet) return;
  const d = await hdLoad();
  hdApplyFollow(d);
  hdInspectorSync();
}

window.DSHostDefaults = { init: hostDefaultsInit, load: hdLoad, inspector: hdInspectorSync, openLoad: hdOpenLoad, openSave: hdOpenSave, plan: hdPlan, word: hdWord, data: () => HD.data,
  setFault: f => { HD.fault = f; } };
