/* Drum Spike v2 — state (R17 step 1, #55)
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
    s.ui = Object.assign({ infoLine: true, counter: "seconds", auditionTab: false, suspendFollowOnEdit: false, stopReturnToStart: false, tabMode: "active", training: false }, s.ui || {});   // P3-14b: the info line is shown; the main counter shows seconds; P4-wf10: Transport > Suspend Follow when Editing; #436: Transport > Stop Returns to Start; #175: Tab moves to active hits only
    if (!["samples", "barsbeats", "timecode"].includes(s.ui.counter)) s.ui.counter = "seconds";   // #149: Bars|Beats and Timecode counters too
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
    if (st && typeof st.velocity_lane === "boolean") { VL.on = st.velocity_lane; vlApply(); }   // P4-40f
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

/* ---------------- host mode (DAW plugin P2, #509) ----------------
   In a DAW the host owns the files, the transport and the tempo. The plugin shell sets
   window.__DS_HOST_MODE__ before the page loads; backend.js turns it into the one flag
   B.hostMode on the transport backend, and hostMode() is the one place that reads it. The
   desktop app never sets the flag, so the mode is off and the standalone page is unchanged
   (owner rule 2026-11-14). While the mode is on, the commands the host owns are absent from
   the menus (shell.js) and the keys (keymap.js), and the play, stop and locate of the page
   go to the host transport (B.hostTransport, static/transport.js). */
const HOST_OWNED = ["new", "open", "open_recent", "recent_clear", "close", "save", "save_as", "add_tracks", "import_old", "consolidate", "import_tempo_map", "detect_tempo"];

/* #539: the command groups that are absent in host mode: the host plays the audio, owns the window,
   the files and the track set, and there is no agent. THE one list: the menu items and the keys
   (menu: the ids of static/keymap.js, so hostOwned() hides them in the menu, the key table and the
   shortcut list), the page functions that call the service (commands: the names of the service
   commands that no page call may reach in host mode; the e2e test counts them through a spy on the one
   invoke), the controls (controls: CSS selectors, hidden by hostApply(); the ids stay in the page) and
   the short text that names the host (say: what the status line shows when a key or a flow of the
   group is used). With the mode off nothing here is read: the standalone page is the page of today.
   Not in a group, because they work through the plugin: Analyze All, Analyze Selected Track,
   Re-analyze Selection, the role of a track, every hit edit, Learn, tempo reads, zoom, follow, undo, and the
   MIDI file drag (static/export.js exDragStart, which M1c wires). */
const HOST_ABSENT = [
  { group: "audio", say: "the host plays the audio",
    menu: ["audition", "audition_tab", "click", "mute_track", "solo_track", "reset_mix", "half_speed"],   // #559: the host has no half speed (a play request carries no speed)
    commands: ["transport_play", "transport_stop", "transport_seek", "transport_status", "transport_mix", "transport_clicks",
      "transport_master", "track_set_mix", "outputs_list", "output_select"],
    controls: ["#clickBtn", "#auditionTabBtn", ".master", "#insMixBox", ".tctl", ".meter", "#setCat-audio", "#setCat-audition",
      "#setCat-pitches", "#setAudio", "#setAudition", "#setPitchCat"] },
  { group: "process", say: "the host owns the window",
    menu: ["quit", "about"],
    commands: ["quit_app", "open_paths_take", "about_info", "about_third_party", "open_link"],
    controls: ["#aboutModal"] },
  { group: "files", say: "the host owns the files",
    menu: ["export", "export_again", "export_selected", "rename"],
    commands: ["export_all", "export_existing", "export_dest_set", "export_markers_set", "export_compensate_set", "export_last_get",
      "export_last_set", "export_midi", "export_spikes", "export_layer", "sampler_calibration_midi", "reveal_path", "project_rename"],
    controls: ["#exportAllBtn", "#exportModal", ".lyRender", ".smCal", "#insFileRow", "#insStateRow"] },
  { group: "agent", say: "the host runs no agent or training",
    menu: ["training_mode", "confirm_hit"],
    commands: ["window_call_next", "window_call_result", "agent_skill_status", "agent_skill_install", "feedback_send",
      "training_status", "training_hand_placed"],
    controls: ["#feedbackBtn", "#fbModal", "#setCat-agent", "#setAgentSkill", "#stTraining"] },
  { group: "tracks", say: "the host owns the tracks",
    menu: ["tracks_remove"],
    commands: ["tracks_add", "tracks_remove", "tracks_restore", "tracks_reorder", "track_rename", "tracks_scan"],
    controls: ["#addTracksBtn", "#tracksModal", "#insRelink"] },
];
function hostMode() { return typeof B !== "undefined" && !!B.hostMode; }   // B is a top-level const: no window.B
function hostOwned(id) { return hostMode() && (HOST_OWNED.includes(id) || HOST_ABSENT.some(g => g.menu.includes(id))); }
/* The few texts of the page that name an absent command in the middle of a sentence (a tooltip of the Inspector). In host
   mode they say it without the absent name; with the mode off the text is the text of today. */
const HOST_WORDS = [["spikes, Export All, the click and a follower", "spikes and a follower"]];
function hostWords(text) { return hostMode() ? HOST_WORDS.reduce((t, [from, to]) => t.split(from).join(to), text) : text; }
/* A flow of a group was used (a key, a call by name, a control that is hidden): true when the group is absent, with the
   text that names the host on the status line. The caller returns at once. */
function hostAbsent(group) {
  if (!hostMode()) return false;
  const g = HOST_ABSENT.find(x => x.group === group);
  setMsg(g ? g.say : "the host owns this");
  return true;
}
/* The page of the host mode: one class on <body> (the layout of the low and wide window, style.css) and one rule that
   hides the controls of the list. Idempotent; the mode off removes both again. */
function hostApply() {
  const on = hostMode();
  document.body.classList.toggle("host", on);
  let st = document.getElementById("hostStyle");
  if (!on) { if (st) st.remove(); return; }
  if (!st) { st = document.createElement("style"); st.id = "hostStyle"; document.head.appendChild(st); }
  st.textContent = HOST_ABSENT.map(g => g.controls).flat().map(c => `body.host ${c}`).join(",\n") + " { display: none !important; }";
}

/* #539: a low and wide window. In LUNA the editor is a strip below the timeline, 300 to 400 px high and the full width,
   so under HOST_LOW_H the page gives the lanes the room: the Inspector and the Overview step back and the tracks take the
   small height. These are the page's own switches without saving, so the saved settings stay as the user made them;
   when the window grows again the page puts back what it changed, unless the user has changed it since. The class
   hostLow on <body> compacts the empty state (style.css, host mode only). */
const HOST_LOW_H = 520;
const HOST_FIT = { low: false, mem: null, busy: false, touched: {} };
function hostFitTouch(what) { if (hostMode() && HOST_FIT.low && !HOST_FIT.busy) HOST_FIT.touched[what] = true; }   // the user changed it while the window was low
function hostFit() {
  if (!hostMode()) return;
  const low = innerHeight < HOST_LOW_H;
  document.body.classList.toggle("hostLow", low);
  if (low === HOST_FIT.low) return;
  HOST_FIT.low = low;
  if (low) {
    HOST_FIT.mem = { inspector: S.inspector, trackHeight: S.trackHeight, overview: S.overviewOn }; HOST_FIT.touched = {};
    HOST_FIT.busy = true;
    if (S.inspector) setInspector(false, false);
    if (S.trackHeight !== "small") setTrackHeight("small", false);
    if (S.overviewOn) setOverview(false, false);
    HOST_FIT.busy = false;
  } else if (HOST_FIT.mem) {
    const m = HOST_FIT.mem; HOST_FIT.mem = null;
    const t = HOST_FIT.touched; HOST_FIT.touched = {};   // what the user changed while the window was low stays as the user left it
    if (m.inspector && !S.inspector && !t.inspector) setInspector(true, false);
    if (m.trackHeight !== "small" && S.trackHeight === "small" && !t.trackHeight) setTrackHeight(m.trackHeight, false);
    if (m.overview && !S.overviewOn && !t.overview) setOverview(true, false);
  }
  invalidate(F.SIZE | F.VIEW);
}

/* ---------------- the public names on window ----------------
   The e2e tests (page.evaluate) and shell.js read S, PH and the settings functions by name;
   S is on window from day one (R17 plan). The function declarations above are already
   properties of window; the two consts are not, so they go on explicitly. */
window.S = S;
window.PH = PH;
