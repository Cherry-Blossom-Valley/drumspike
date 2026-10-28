/* Drum Spike v2 — state (R17 step 1, #55 per docs/review/R17-plan.md)
   The state object S, the playhead accessor PH (P4-1), the global settings and the
   dirty-state flag. Plain script (no build step), loaded first. The e2e tests and shell.js
   read these as globals; S is on window from day one. The settings functions still call
   back into app.js globals (the shared global scope; the explicit init(deps) wiring comes
   with the later steps of the plan). */
"use strict";

/* ---------------- state ---------------- */
const S = {
  sid: localStorage.getItem("dsg_sid") || null,
  tracks: {},
  order: [],
  view: { t0: 0, t1: 10 },
  events: {},          // name -> [{t, v}]
  peaks: {}, peakErr: {},
  samplesCache: {},
  peakAmp: {},
  snap: true,          // playhead snaps to the nearest hit when clicked close (10px/30ms)
  selTrack: null,      // track context: header click → Tab navigates this track
  followMode: "smooth",// smooth | page | off
  followHold: false,   // user pan/zoom during playback pauses following until next play
  followResumeAt: null,// P4-wf10: an edit during playback suspended the follow; the song time at which it resumes
  sel: null,           // {track, index}
  loaded: 0,           // generation of the last finished loadProject (0 while one runs)
  playing: false,
  playStartPos: 0,
  playReturn: null,    // #207: the play-and-return key that started this play (the next Space or the same key stops and returns)
  muted: {}, solo: {}, clicks: {},
  follow: true,
  undo: [], redo: [],
  drag: null,
  theadW: 230,
  name: "", dirty: false,   // the open project's name and unsaved-changes flag (P0-8)
  exportAgain: false,   // P4-31: the project keeps an export shape (File > Export Again; greyed before the first)
  inspector: true,     // P3-14a: View > Show Inspector; an app setting (<home>/settings.json), read at boot
  ampZoom: 1,   // P3-20a: View > Waveform Zoom (1x to 64x), a display gain of the wave lanes; a view setting
  nudgeValue: "1smp",   // P3-20a: Edit > Nudge Value (1smp, 10smp, 100smp, 1ms, 10ms); a view setting
  rowH: {}, zoomTall: false,   // P4-27: wave lane heights of single rows (the Zoom Toggle), and the app setting zoom_toggle_tall
  trackHeight: "medium",   // P3-14c: View > Track Height (small, medium, large) of every track; an app setting, read at boot
  overviewOn: true,   // #169: View > Show Overview; the app setting overview, read at boot
  zoomOwn: [],   // #169: the engineer's zoom presets [{name, span}] (span in seconds); the app setting zoom_presets, read at boot
};
/* P4-1: S.playhead. While the compositor moves the playhead (block "compositor playhead"), it reads the position that
   the animation shows now, so every reader (follow, counters, tests, the self-test) sees the drawn playhead between the
   page's ticks. A write is a move by the user (ruler, lane click, Tab, Enter, a test): while playing, the next tick
   seeks the transport to it (P3-5b), and the write asks for that tick.
   P4-1b: while playing, a write of the value the playhead has (PH.v, or the last value the getter gave, PH.r) is no
   move: `S.playhead = Math.min(S.playhead, maxDur())` (loadProject) and the like must not seek. */
const PH = { v: 0, r: NaN, user: false };
Object.defineProperty(S, "playhead", { enumerable: true,
  get() { if ((CP.anim || SP.on) && !PH.user) return (PH.r = cpPos(performance.now())); PH.r = NaN; return PH.v; },   // P4-1b fix: PH.r only while the compositor moves it
  set(x) { if (S.playing && (x === PH.v || x === PH.r)) return; PH.v = x; PH.user = true; if (S.playing) requestFrame(); } });

/* ---------------- global settings ---------------- */
let _settings = null;
function SETTINGS() {
  if (!_settings) {
    let s = {};
    try { s = JSON.parse(localStorage.getItem("dsg_settings") || "{}"); } catch(e) {}
    const def = { click: { enabled: true, volume: 0.22,
      pitches: Object.assign({}, ROLE_DEFAULT_CLICK) } };
    s.click = Object.assign(def.click, s.click || {});
    s.click.pitches = Object.assign({}, def.click.pitches, s.click.pitches || {});
    s.ui = Object.assign({ infoLine: true, counter: "seconds", auditionTab: false, suspendFollowOnEdit: false, stopReturnToStart: false }, s.ui || {});   // P3-14b: the info line is shown; the main counter shows seconds; P4-wf10: Transport > Suspend Follow when Editing; #436: Transport > Stop Returns to Start
    if (s.ui.counter !== "samples") s.ui.counter = "seconds";
    // one-time migration: the per-track 🔈 click became a global one — default it ON
    if (!localStorage.getItem("dsg_settings_v2")) {
      s.click.enabled = true;
      localStorage.setItem("dsg_settings_v2", "1");
    }
    _settings = s;
  }
  return _settings;
}
function saveSettings() { localStorage.setItem("dsg_settings", JSON.stringify(SETTINGS())); appClicksSoon(); }

/* dirty state (P0-8): a dot in the window title and next to the project name while the open
   project has unsaved changes; the server's flag is the truth (B.projectGet) */
function setDirtyState(dirty) {
  S.dirty = !!dirty;
  $("#dirtyDot").classList.toggle("hidden", !S.dirty);
  document.title = (S.dirty ? "● " : "") + (S.name || "Drum Spike") + (S.name ? " - Drum Spike" : "");
  if (typeof DSShell !== "undefined") DSShell.sync();   // P4-26: the native window title follows the name and the unsaved flag
}

async function loadViewSettings() {   // at boot: the saved state of the Inspector, the key preset and the track height (the defaults stay when there are none)
  try {
    const st = await B.appSettingsGet();
    if (st && typeof st.inspector === "boolean") setInspector(st.inspector, false);
    if (st && typeof st.key_preset === "string") setKeyPreset(st.key_preset, false);   // P3-13c
    if (st && TRACK_HEIGHTS[st.track_height]) setTrackHeight(st.track_height, false);   // P3-14c
    if (st && AMP_STEPS.includes(st.amp_zoom)) setAmpZoom(st.amp_zoom, false);   // P3-20a
    if (st && typeof st.zoom_toggle_tall === "boolean") setZoomTall(st.zoom_toggle_tall, false);   // P4-27
    if (st && NUDGE_VALUES[st.nudge_value]) setNudgeValue(st.nudge_value, false);   // P3-20a
    for (const k of Object.keys(TM_SETTING)) if (st && typeof st[TM_SETTING[k]] === "boolean") TM.ui[k] = st[TM_SETTING[k]];   // P4-8
    if (st && typeof st.velocity_lane === "boolean") { VL.on = st.velocity_lane; VL.hint = VL.on; vlApply(); }   // P4-40f; #208: the hint shows on an open lane
    if (st && typeof st.velocity_lane_height === "number" && st.velocity_lane_height >= VL_H_MIN && st.velocity_lane_height <= VL_H_MAX) VL.h = Math.round(st.velocity_lane_height);   // #208
    if (st && typeof st.other_mics === "boolean") { MX.on = st.other_mics; mxApply(); }   // #157
    if (st && typeof st.overview === "boolean") setOverview(st.overview, false);   // #169
    if (st && Array.isArray(st.zoom_presets)) {   // #169: a damaged entry is left out
      S.zoomOwn = st.zoom_presets.filter(p => p && typeof p.name === "string" && p.name.trim() && Number(p.span) > 0)
        .map(p => ({ name: p.name.trim().slice(0, OWN_ZOOM_NAME_MAX), span: Number(p.span) }));
      renderZoomOwn();
      if (window.DSShell && DSShell.sync) DSShell.sync();
    }
    tmColors();
  } catch (e) { /* no settings */ }
}

/* ---------------- the public names on window ----------------
   The e2e tests (page.evaluate) and shell.js read S, PH and the settings functions by name;
   S is on window from day one (R17 plan). The function declarations above are already
   properties of window; the two consts are not, so they go on explicitly. */
window.S = S;
window.PH = PH;
