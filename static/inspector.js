/* Drum Spike v2 — Inspector (refactor plan step 5.2, R9 A2)
   The params panel (the Detection, Velocity, Timing, Learn, Output and Layer sections: buildParamsPanel, the
   steppers), the Inspector itself (INS, setInspector, syncInspector, the counts, flags and source lines), the track
   height (P3-14c) and the collapsible sections with the Inspector width (P4-25, setupInspector). Moved verbatim
   from app.js. Plain script (no build step), loaded after status.js and before app.js. The names stay top-level
   bindings (page globals). */
"use strict";

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
      ${p.refine !== false ? `<div class="prow">
        <label>Refine window low ${info("The search window around the coarse detection time, in ms: the refine looks for the attack from this far before it. Default -2. Stored at once; the next re-analyze uses it.")}</label>
        <div class="pctl"><input type="number" class="pRefineLo" step="1" value="${Number.isFinite(p.refine_lo_ms) ? p.refine_lo_ms : -2}"><span class="punit">ms</span></div>
      </div>
      <div class="prow">
        <label>Refine window high ${info("The search window around the coarse detection time, in ms: the refine looks for the attack up to this far after it. Default 12. Stored at once; the next re-analyze uses it.")}</label>
        <div class="pctl"><input type="number" class="pRefineHi" step="1" value="${Number.isFinite(p.refine_hi_ms) ? p.refine_hi_ms : 12}"><span class="punit">ms</span></div>
      </div>` : ""}
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
    for (const sel of [".pSens", ".pGap", ".pRefine", ".pRefineLo", ".pRefineHi", ".pFlam"]) { const el = div.querySelector(sel); if (el) el.disabled = true; }
    noteRow.classList.remove("hidden"); note.textContent = `follows ${t.follows}: the hits are derived from it, the detection settings are not used`;
    div.querySelector(".pactions .phint").textContent = "re-analyze derives the hits from the followed track again";
  }
  div.querySelector(".pGap").onchange = ev => p.min_gap_ms = +ev.target.value;
  div.querySelector(".pRefine").onchange = ev => p.refine = ev.target.checked;
  // #232: the refine window is stored at once (track_params_set, one undo step), so an agent and
  // the app see the same number (tracks_get), and Undo brings the old window back
  // the fields exist only while Refine to attack is on (a missing field threw here and cut the panel's setup short)
  const refLo = div.querySelector(".pRefineLo"), refHi = div.querySelector(".pRefineHi");
  if (refLo) refLo.onchange = ev => paramsFieldCommit(name, "refine_lo_ms", Math.round(+ev.target.value || 0));
  if (refHi) refHi.onchange = ev => paramsFieldCommit(name, "refine_hi_ms", Math.round(+ev.target.value || 0));
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
