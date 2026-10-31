/* Backend adapter (P3-1a). app.js calls named operations on B and never builds a path.
   B is the IPC backend over the app commands of crates/app (P3-3b): window.__TAURI__ in the app, or the
   test bridge transport (P3-7a, end of file) when the debug binary ds-bridge serves the page. B.app is true.
   P3-12: the HTTP backend of the retired Python server is gone; outside the app and the bridge every call fails.
   Errors: a failed call throws Error(`<status>: <text>`) with .status set. */
/* IPC backend (P3-3b): the named operations over the app commands of crates/app. A command that fails rejects with {code, message};
   fail() turns it into Error(`<code>: <text>`) with .status.
   Bytes: peaks_tile = u32 level, start, n then f32 min[n], max[n], rms[n];
   samples_window = u32 start, sr then f32 samples (little endian). */
/* P4-30: the options of the Export dialog as the commands take them (a missing one is null: the default) */
function exportArgs(o) {
  return { dest: (o && o.dest) || null, midi_mode: (o && o.midi_mode) || null, tracks: (o && o.tracks) || null, formats: (o && o.formats) || null,
    compensate: o && typeof o.compensate === "boolean" ? o.compensate : null };   // P4-7c: null is the default (on)
}
/* #164: the CBV feedback gateway (the owner's design decision on #153: option b, the CBV site
   backend, part 1 in Cherry-Blossom-Valley/Cbvsite: POST .../feedback of the
   make-server-91a5b4f8 edge function). One POST per report, no secret in the app. The default
   of the app setting `feedback_url` (the page passes it, so it can change without a release). */
const FEEDBACK_URL = "https://pubtzkqxycnqqhmrfnhn.supabase.co/functions/v1/make-server-91a5b4f8/feedback";
function ipcBackend(T) {
  const invoke = (cmd, args) => T.core.invoke(cmd, args).catch(e => { throw fail(e); });
  const UNSUPPORTED = "This file is not supported. Drum Spike reads WAV files with 44.1, 48 or 96 kHz. " +
    "Convert the file in your DAW (WAV, 48 kHz) and add it again.";
  /* P3-3c: only the clear text goes to the lane and the status line. The core text stays in err.detail
     (a tooltip in the page) and in the console log. */
  const isUnsupported = m => /unsupported sample rate|wav format|unsupported format|not a RIFF|not a wav/i.test(m);
  const friendly = m => isUnsupported(m) ? UNSUPPORTED : m;
  function fail(e) {
    const code = e && typeof e === "object" && typeof e.code === "number" ? e.code : 500;
    const raw = String(e && e.message !== undefined ? e.message : e);
    const err = new Error(`${code}: ${friendly(raw)}`);
    err.status = code;
    if (isUnsupported(raw)) { err.detail = raw.slice(0, 400); console.warn(`drumspike: ${err.detail}`); }
    return err;
  }
  const later = what => () => Promise.reject(Object.assign(new Error(`501: ${what} is not in the app yet (it comes with a later milestone)`), { status: 501 }));
  async function peaks(sid, name) {
    const info = await invoke("peaks_info", { sid, name });
    const levels = {};
    await Promise.all(Object.keys(info.levels).map(async key => {
      const n = info.levels[key].n;
      const buf = await invoke("peaks_tile", { sid, name, level: +key, from: 0, to: n });
      const got = new Uint32Array(buf, 0, 3)[2];   // the tile may hold fewer bins than asked
      const f = k => Array.from(new Float32Array(buf, 12 + 4 * k * got, got));
      levels[key] = { n: got, min: f(0), max: f(1), rms: f(2) };
    }));
    return { sr: info.sr, n_samples: info.n_samples, levels };
  }
  /* P3-6b: the tiles as bytes for the tile worker: raw = { level: ArrayBuffer of a peaks_tile answer }; levels holds the
     counts, and the arrays of the coarsest level only (the overview and the display gain use it). The page keeps no more. */
  async function peaksRaw(sid, name) {
    const info = await invoke("peaks_info", { sid, name });
    const raw = {}, levels = {}, keys = Object.keys(info.levels);
    await Promise.all(keys.map(async key => {
      const n = info.levels[key].n;
      raw[key] = await invoke("peaks_tile", { sid, name, level: +key, from: 0, to: n });
      levels[key] = { n: new Uint32Array(raw[key], 0, 3)[2] };   // the tile may hold fewer bins than asked
    }));
    const top = keys.reduce((a, k) => (+k > +a ? k : a), keys[0]), got = levels[top].n;
    const f = k => Array.from(new Float32Array(raw[top], 12 + 4 * k * got, got));
    levels[top] = { n: got, min: f(0), max: f(1), rms: f(2) };
    return { sr: info.sr, n_samples: info.n_samples, levels, raw };
  }
  async function samples(sid, name, start, end) {
    const buf = await invoke("samples_window", { sid, name, start, end });
    const h = new Uint32Array(buf, 0, 2);
    return { start: h[0], sr: h[1], data: new Float32Array(buf, 8) };
  }
  async function job(sid) {
    const st = await invoke("analyze_status", { sid });
    for (const t of Object.values(st.tracks || {})) if (t.error && isUnsupported(t.error)) { t.error_detail = t.error.slice(0, 400); t.error = UNSUPPORTED; }
    return st;
  }
  const b = {
    app: true,
    sessions: () => invoke("project_list", {}),
    recentProjects: () => Promise.resolve([]),
    recentClear: async () => ({ cleared: 0 }),   // #441: no stored list without a home folder
    legacyList: () => Promise.resolve([]),
    openProject: path => invoke("project_open", { path }),
    projectGet: sid => invoke("project_get", { sid }),
    backendInfo: async () => {
      const i = await invoke("app_info", {});
      return { backend: "rust", core: i.core_version, detector: i.detector, chain: i.chain };
    },
    tracks: sid => invoke("tracks_get", { sid }),
    setEvents: (sid, name, body) => invoke("events_set", { sid, name, events: body.events }),
    events: (sid, name) => invoke("events_get", { sid, name }),
    eventsEdit: (sid, name, ops) => invoke("events_edit", { sid, name, ops }),   // P4-9 stage 2: add, delete, move, velocity of single hits
    peaks, peaksRaw, samples, job,
    analyze: (sid, tracks, params, keepEdited) => invoke("analyze_start", { sid, tracks, params, keep_edited: !!keepEdited, progress: new T.core.Channel() }),   // P4-9: keep_edited merges the edited hits in the service
    /* writes Exports/<name>_spike.mid in the project folder; answers {path, size, n_events} */
    exportMidi: (sid, name, note, events, ppq) => invoke("export_midi", { sid, name, note, events, ppq }),   // P4-8: the tempo map decides the tempo
    /* P3-3c: Exports/<name>_spike.wav at the rate, length, format and bext time reference of the source;
       answers {path, size, n_events, sr, n_samples, format, time_reference} */
    exportSpikes: (sid, name, events) => invoke("export_spikes", { sid, name, events }),
    /* P3-3c: MIDI and spikes of every track; onProgress gets {state, done, total, track}; answers
       {files, midi, spikes, errors, elapsed_ms} */
    exportAll: (sid, onProgress, o) => { const ch = new T.core.Channel();   // P4-30: o = {dest, midi_mode, tracks, formats} (all optional)
      ch.onmessage = m => { if (onProgress && m && m.export) onProgress(m.export); };
      return invoke("export_all", { sid, ...exportArgs(o), progress: ch }).then(r => {   // the same clear text as for the lane
        for (const e of r.errors || []) if (isUnsupported(e.message)) { e.detail = e.message.slice(0, 400); e.message = UNSUPPORTED; }
        return r; }); },
    /* P3-3c: route 34, the times stay and v is computed again; answers {events} */
    recomputeVelocities: (sid, name, events) => invoke("velocities_recompute", { sid, name, events }),
    /* P4-3: the Inspector histograms. histograms: {loudness: {levels}, detection: {state, candidates, gap}};
       repick: the hits at the params' thresholds from the kept activations (store: as an analysis stores);
       velocitySet: stores the floor, ceiling and loudness cut (null: off), answers {events} with the new v and the cut
       marks (a follower: stored, {events, follows}) */
    histograms: (sid, name, events) => invoke("track_histograms", { sid, name, events: events || null }),
    /* #150: the confidence of each hit of `events` for the review queue: {confidence: [{c, doubt, why}], source} */
    confidence: (sid, name, events) => invoke("track_confidence", { sid, name, events }),
    paramsSet: (sid, name, params) => invoke("track_params_set", { sid, name, params }),   // fix round: undo and redo of a re-pick
    pluginsList: (sid) => invoke("plugins_list", sid ? { sid } : {}),   // #387: the installed detector plugins
    pluginAccept: (sid, id) => invoke("plugin_accept", { sid, id }),   // #387: record the licence acceptance in the project
    repick: (sid, name, params, store) => invoke("track_repick", { sid, name, params, store: !!store, keep_edited: true }),   // P4-9: the edited hits stay (Rust merge_edited)
    shiftSet: (sid, name, shift) => invoke("track_shift_set", { sid, name, shift_samples: shift }),   // P4-9: the Hitpoint shift
    velocitySet: (sid, name, loDb, hiDb, cutDb, events) => invoke("track_velocity_set", { sid, name, lo_db: loDb, hi_db: hiDb,
      cut_db: cutDb === null || cutDb === undefined ? null : cutDb, events: events || null }),
    /* P4-6a: Learn marks (crates/app/src/learn.rs). learnMark: {sid, name, kind: this|not, t0, t1 | hit} answers {marked, mark, learn,
       track (the track that holds the marks: the source of a follower), routed, counts, reason?}; the rest answer {learn, ...};
       learnSet stores the whole object (null: none), the undo and redo of a step */
    learnMark: (sid, name, kind, t0, t1, hit) => invoke("track_learn_mark", { sid, name, kind, t0: t0 === undefined ? null : t0,
      t1: t1 === undefined ? null : t1, hit: hit === undefined ? null : hit }),
    learnRemove: (sid, name, id, at) => invoke("track_learn_remove", { sid, name, id, at: at === undefined ? null : at }),
    learnForgetAt: (sid, name, t0, t1, hit) => invoke("track_learn_forget_at", { sid, name, t0: t0 === undefined ? null : t0,   // P4-36 Forget This: {removed, learn, track, routed}
      t1: t1 === undefined ? null : t1, hit: hit === undefined ? null : hit }),
    learnFlip: (sid, name, id) => invoke("track_learn_flip", { sid, name, id }),
    learnForget: (sid, name) => invoke("track_learn_forget", { sid, name }),
    learnSet: (sid, name, learn, events) => invoke("track_learn_set", { sid, name, learn, events: events || null }),   // P4-6c: events = the snapshot of an undo step
    /* P4-6c: the scores of the candidates and the examples (crates/app/src/learn_score.rs):
       {state, track, routed, method, candidates: [[t, class, score, v]], examples: {this, not}, auto, threshold, cache, ms} */
    learnScores: (sid, name) => invoke("track_learn_scores", { sid, name }),
    /* P4-8: the tempo and meter map (crates/app/src/tempo.rs). tempoMapGet: {map, default}; tempoMapSet (the whole map, null or {} is
       the default): {map, previous, changed}, previous null when the stored value was not valid; tempoMapDetect: a suggestion
       (read only; the page accepts it with tempoMapSet); tempoMapImportMidi: {map, previous, changed, tempo_events, meter_events,
       snapped}, a 400 with the reason for a file without tempo or meter events; pickTempoFile: the native open dialog for a MIDI file */
    tempoMapGet: sid => invoke("tempo_map_get", { sid }),
    tempoMapSet: (sid, map) => invoke("tempo_map_set", { sid, map }),
    tempoMapDetect: sid => invoke("tempo_map_detect", { sid }),
    tempoMapRefitBar1: (sid, bpm) => invoke("tempo_map_refit_bar1", { sid, bpm }),   // #468: the supported bar 1 of a typed start tempo
    tempoMapImportMidi: (sid, path) => invoke("tempo_map_import_midi", { sid, path }),
    /* P4-40g: the review markers (crates/app/src/markers.rs). markersGet: {markers, n}; markersSet
       (the whole list, [] clears it): {markers, previous, changed, n} — `previous` is the undo state. */
    markersGet: sid => invoke("markers_get", { sid }),
    markersSet: (sid, markers) => invoke("markers_set", { sid, markers }),

    /* #167: the per-track lane heights (crates/app/src/rowheights.rs). The map is
       {track: factor}; the wave lane of the row is the global wave height times the factor. */
    rowHeightsGet: sid => invoke("row_heights_get", { sid }),
    rowHeightsSet: (sid, heights) => invoke("row_heights_set", { sid, heights }),
    pickTempoFile: async () => {
      const r = await T.dialog.open({ directory: false, multiple: false, title: "Import a tempo map from a MIDI file",
        filters: [{ name: "MIDI file", extensions: ["mid", "midi"] }] });
      return typeof r === "string" && r ? r : null;
    },
    /* P4-7a and P4-7b: the one-shot layer of a track (crates/app/src/layer.rs). layerGet: {layer, status, path, file, onset_ms};
       layerSet(sid, name, {path?, level_db?, polarity?, on?, curve?, align?, onset?}) answers the same ("" as path removes the layer; `onset` (samples, P4-40e) overrides the measured one, null resets; `transport_error` when the open transport did not take the layers);
       layerAlign: per hit {t, lag, match, sign, uncertain} and the track's polarity; layerExport writes Exports/<name>_layer.wav;
       layerWave/samplerWave (P4-40e): the one-shot's / the calibration bounce's mono samples (u32 start, u32 rate, then f32, like samples_window) for the Layer view;
       pickOneShot: the native open dialog for a WAV or AIFF file */
    layerGet: (sid, name) => invoke("track_layer_get", { sid, name }),
    layerSet: (sid, name, o) => invoke("track_layer_set", { sid, name, ...o }),
    layerAlign: (sid, name) => invoke("track_layer_align", { sid, name }),
    layerExport: (sid, name, path) => invoke("export_layer", { sid, name, path: path || null }),   // P4-7d: path = the save dialog's file
    layerWave: async (sid, name, start, end) => {
      const buf = await invoke("track_layer_wave", { sid, name, start, end });
      const h = new Uint32Array(buf, 0, 2);
      return { start: h[0], sr: h[1], data: new Float32Array(buf, 8) };
    },
    /* P4-7c: the sampler calibration (crates/app/src/sampler.rs): samplerCalibrationMidi writes <name>_calibration.mid {path, t, tick, text};
       samplerSet(sid, name, {path, one_shot}) measures the bounce ("" removes it) and answers {sampler, offset_ms, polarity, file, layer}; samplerGet the same without a layer */
    samplerCalibrationMidi: (sid, name, dest, path) => invoke("sampler_calibration_midi", { sid, name, dest: dest || null, path: path || null }),
    samplerGet: (sid, name) => invoke("track_sampler_get", { sid, name }),
    samplerWave: async (sid, name, start, end) => {
      const buf = await invoke("track_sampler_wave", { sid, name, start, end });
      const h = new Uint32Array(buf, 0, 2);
      return { start: h[0], sr: h[1], data: new Float32Array(buf, 8) };
    },
    samplerSet: (sid, name, o) => invoke("track_sampler_set", { sid, name, path: (o && o.path) || "", one_shot: !!(o && o.one_shot) }),
    /* P4-7d: the Layer section's load and save dialogs. dir (optional) is the folder the dialog starts in */
    pickBounce: async dir => {
      const r = await T.dialog.open({ directory: false, multiple: false, title: "Choose the bounce of the calibration note (WAV or AIFF)",
        defaultPath: dir || undefined, filters: [{ name: "Audio", extensions: ["wav", "wave", "bwf", "aif", "aiff", "aifc"] }] });
      return typeof r === "string" && r ? r : null;
    },
    pickOneShot: async dir => {
      const r = await T.dialog.open({ directory: false, multiple: false, title: "Choose a one-shot (WAV or AIFF)",
        defaultPath: dir || undefined, filters: [{ name: "Audio", extensions: ["wav", "wave", "bwf", "aif", "aiff", "aifc"] }] });
      return typeof r === "string" && r ? r : null;
    },
    /* the native save dialog for one file: defaultPath is a full path (folder and name); the answer is the path or null */
    pickSaveFile: async (title, defaultPath, ext, label) => {
      const r = await T.dialog.save({ title, defaultPath, filters: [{ name: label, extensions: [ext] }] });
      return typeof r === "string" && r ? r : null;
    },
    /* the native open dialog; the path of a project folder, or null */
    pickProject: async () => {
      const r = await T.dialog.open({ directory: true, multiple: false, title: "Open a Drum Spike project folder" });
      return typeof r === "string" ? r : null;
    },
  };
  Object.assign(b, transportMethods(invoke, T));
  const names = { newProject: "New project", projectSave: "Save", saveAs: "Save As", consolidate: "Consolidate",
    closeProject: "Close project", relink: "Relink", importLegacy: "Import",
    renameSession: "Rename", reorder: "Reordering tracks", setNote: "Saving the MIDI note" };
  for (const k of Object.keys(names)) b[k] = later(names[k]);
  Object.assign(b, shellMethods(invoke, T));   // P3-4b: menu events, quit, Finder paths, native pickers
  Object.assign(b, lifecycleMethods(invoke, T));   // P3-4a: replaces the stubs of New, Save, Save As, Close, Rename, Reorder, Note
  Object.assign(b, importMethods(invoke, T));      // P3-4e: replaces the stubs of the importer
  Object.assign(b, tracksMethods(invoke, T));      // P3-4c: Add tracks by path, kit presets, the native file dialog
  Object.assign(b, jobMethods(invoke, T));         // P3-4d: Save As with copied audio, Consolidate, job_cancel, Relink, the folder search
  return b;
}

/* Project lifecycle (P3-4a): New, Save, Save As (referenced), Close, Rename, Reorder, Note, Open Recent,
   the dirty event, the native save dialog, and the lane build with progress. Commands:
   project_new, project_save, project_save_as, project_close, project_rename, tracks_reorder,
   track_set_note, project_recent, project_recent_clear, peaks_build_start, peaks_build_status
   (crates/app/src/lifecycle.rs, lanes.rs).
   Save As with copyAudio is a job since P3-4d: jobMethods below replaces saveAs. */
function lifecycleMethods(invoke, T) {
  return {
    recentProjects: () => invoke("project_recent", {}),
    recentClear: () => invoke("project_recent_clear", {}),   // #441 Clear Recent: empties recent.json
    newProject: (name, dir, audioMode) => invoke("project_new", { name: name || null, dir: dir || null, audio_mode: audioMode || null }),
    projectSave: (sid, force) => invoke("project_save", { sid, force: !!force }),   // #121: force = Save anyway (409 when the file changed on disk)
    projectCleanCheck: sid => invoke("project_clean_check", { sid }),   // P4-40a: an undo back to the saved state clears the dirty flag; {dirty}
    saveAs: (sid, dir, name, copyAudio) => invoke("project_save_as", { sid, dir, name, copy_audio: !!copyAudio }),
    closeProject: (sid, force) => invoke("project_close", { sid, force: !!force }),
    renameSession: (sid, name) => invoke("project_rename", { sid, name }),
    reorder: (sid, order) => invoke("tracks_reorder", { sid, order }),
    setNote: (sid, name, note) => invoke("track_set_note", { sid, name, note }),
    renameTrack: (sid, name, newName) => invoke("track_rename", { sid, name, new_name: newName }),   // P3-14a
    /* P3-18: the track takes the hits of `source` (null: its own detection); answers {follows, n_events} */
    setFollow: (sid, name, source) => invoke("track_set_follow", { sid, name, source: source || null }),
    appSettingsGet: () => invoke("app_settings_get", {}),   // P3-14a: <home>/settings.json
    appSettingsSet: (key, value) => invoke("app_settings_set", { key, value }),
    /* the native save dialog: the user types the project name; answers {dir, name} or null */
    pickSave: async (title, defaultName) => {
      const r = await T.dialog.save({ title, defaultPath: defaultName });
      if (typeof r !== "string" || !r) return null;
      const path = r.replace(/\/+$/, "");
      const i = path.lastIndexOf("/");
      return { dir: i > 0 ? path.slice(0, i) : "/", name: path.slice(i + 1) };
    },
    /* cb({sid, name}) after the layer of a track changed (event track-layer; P4-33: by any client) */
    onTrackLayer: cb => T.event.listen("track-layer", e => cb(e.payload)),
    /* cb({sid, dirty}) on every change of a project's unsaved flag (event project-dirty) */
    onDirty: cb => T.event.listen("project-dirty", e => cb(e.payload)),
    /* builds Analysis/<track>/peaks.bin for the tracks that lack it; onProgress(job) gets the job after
       every change ({state, total, tracks: {name: {state, stage?, error?}}}); resolves when the job is done */
    prepareLanes: (sid, onProgress) => new Promise((resolve, reject) => {
      const ch = new T.core.Channel();
      let over = false, timer = null;
      const finish = () => { if (!over) { over = true; clearInterval(timer); resolve(); } };
      ch.onmessage = m => {
        if (!m || !m.job) return;
        try { if (onProgress) onProgress(m.job); } catch (e) { /* a broken callback must not stop the build */ }
        if (m.job.state === "done") finish();
      };
      invoke("peaks_build_start", { sid, tracks: null, progress: ch }).then(r => {
        if (!r.started.length) { finish(); return; }
        timer = setInterval(async () => {   // the status ends the wait when a message got lost
          try { if ((await invoke("peaks_build_status", { sid })).state === "done") finish(); } catch (e) { finish(); }
        }, 300);
      }).catch(e => { over = true; reject(e); });
    }),
  };
}

/* Native shell (P3-4b): the event "menu" and "open-paths" from the Rust shell, the quit command, the paths that
   came from the Finder, the sync of the menu state, the native pickers, and export_existing (Export all asks
   before it replaces files). The commands shell_sync, open_paths_take and quit_app exist in the Tauri app only
   (apps/desktop/src-tauri/src/shell.rs); the test bridge answers them locally: nothing to sync (the last menu
   state stays in window.__DS_MENU_STATE__ for the tests), no paths, and quitApp counts in window.__DS_QUIT__.
   P3-13a: shellSync sends the state of the check items and the current project with the project flag. */
function shellMethods(invoke, T) {
  const bridge = () => !!window.__DS_BRIDGE__ && !(window.__TAURI__ && window.__TAURI__.core);
  return {
    listen: (name, cb) => T.event.listen(name, e => cb(e.payload)),
    shellSync: (hasProject, state) => {
      if (bridge()) { window.__DS_MENU_STATE__ = state || null; return Promise.resolve(0); }
      return invoke("shell_sync", { has_project: !!hasProject, state: state || null });
    },
    /* P3-17: the page says that its first layout is finished (app.js, end of boot). The main window starts hidden; the
       shell shows it and closes the splash (command app_ready). One frame is awaited first; a hidden window paints none,
       so the wait ends after 60 ms. The test bridge counts the calls in window.__DS_READY__ and calls window.__DS_ON_READY__. */
    appReady: async () => {
      await new Promise(res => { const t = setTimeout(res, 60); requestAnimationFrame(() => { clearTimeout(t); res(); }); });
      if (bridge()) {
        window.__DS_READY__ = (window.__DS_READY__ || 0) + 1;
        if (typeof window.__DS_ON_READY__ === "function") window.__DS_ON_READY__(window.__DS_READY__);
        return { ok: true };
      }
      return invoke("app_ready", {});
    },
    takeOpenPaths: () => (bridge() ? Promise.resolve({ projects: [], others: [] }) : invoke("open_paths_take", {})),
    quitApp: () => { if (bridge()) { window.__DS_QUIT__ = (window.__DS_QUIT__ || 0) + 1; return Promise.resolve(); } return invoke("quit_app", {}); },
    /* P4-30: {count, files, dest, default_dest, remembered} for the options of the Export dialog (the same keys as exportAll) */
    exportExisting: (sid, o) => invoke("export_existing", { sid, ...exportArgs(o) }),
    exportDestSet: (sid, dest) => invoke("export_dest_set", { sid, dest: dest || null }),   // the folder the project keeps; null: back to Exports/
    exportMarkersSet: (sid, on) => invoke("export_markers_set", { sid, on }),   // P4-40g: the remembered "Markers in MIDI" checkbox
    exportLastGet: sid => invoke("export_last_get", { sid }),   // P4-31: {export_last} = the export shape of the project, or none
    exportLastSet: (sid, last) => invoke("export_last_set", { sid, ...last }),   // P4-31: the dialog's choices at once, and what an export ran with
    /* pre-renders the files of the drag tiles into Exports/.drag (hidden; never the chosen folder): {dir, midi: [paths], spikes: [paths], errors}; {clear: true} removes them */
    exportDragPrepare: (sid, o) => invoke("export_drag_prepare", { sid, ...(({ dest, ...rest }) => rest)(exportArgs(o)), clear: !!(o && o.clear) }),
    exportDragTake: (sid, paths) => invoke("export_drag_take", { sid, paths }),   // #433: a drag starts: the files move into Exports/ under their export names; {files}
    /* the native file drag out of the window (tauri-plugin-drag): the files must exist. The test bridge has no native drag: it records
       the paths in window.__DS_DRAG__ */
    dragFiles: async paths => {
      if (bridge()) { (window.__DS_DRAG__ = window.__DS_DRAG__ || []).push(paths.slice()); return { result: "Dropped" }; }
      const c = document.createElement("canvas"); c.width = c.height = 64;
      const g = c.getContext("2d"); g.fillStyle = "#4da3ff"; g.beginPath(); if (g.roundRect) g.roundRect(4, 4, 56, 56, 10); else g.rect(4, 4, 56, 56); g.fill();
      g.fillStyle = "#fff"; g.font = "bold 30px sans-serif"; g.textAlign = "center"; g.textBaseline = "middle"; g.fillText(String(paths.length), 32, 34);
      const on_event = new T.core.Channel(); on_event.onmessage = () => {};
      return invoke("plugin:drag|start_drag", { item: paths, image: c.toDataURL("image/png"), onEvent: on_event });
    },
    /* Show in Finder; the test bridge records the path in window.__DS_REVEAL__ */
    revealPath: async path => {
      if (bridge()) { (window.__DS_REVEAL__ = window.__DS_REVEAL__ || []).push(path); return {}; }
      return invoke("reveal_path", { path });
    },
    /* P3-8a: {name, version, core_version, notice} and the third-party list (src-tauri/src/about.rs); the test
       bridge has no such commands and answers from app_info */
    aboutInfo: async () => {
      if (!bridge()) return invoke("about_info", {});
      const i = await invoke("app_info", {});
      return { name: "Drum Spike", version: "test bridge", build_sha: "test-bridge", core_version: i.core_version, notice: i.notice };
    },
    aboutThirdParty: () => (bridge() ? Promise.resolve("The third-party list is part of the app bundle.") : invoke("about_third_party", {})),
    /* #18: open one of the fixed About addresses by id (src-tauri/src/about.rs, open_link); the page
       never passes an address. The test bridge has no browser: it records the id in window.__DS_OPEN_LINK__ */
    openLink: async id => {
      if (bridge()) { (window.__DS_OPEN_LINK__ = window.__DS_OPEN_LINK__ || []).push(id); return {}; }
      return invoke("open_link", { id });
    },
    /* #255: open an http(s) link of the help in the default browser (src-tauri/src/about.rs, open_url);
       the page only passes links the prebuilt help ships (class ext). The test bridge has no browser:
       it records the address in window.__DS_OPEN_URL__ */
    openUrl: async url => {
      if (bridge()) { (window.__DS_OPEN_URL__ = window.__DS_OPEN_URL__ || []).push(url); return {}; }
      return invoke("open_url", { url });
    },
    /* #164: the CBV feedback gateway (part 2 of #153; the route is part 1, Cbvsite). The page previews
       the exact payload; the bridge answers the command (records it in window.__DS_FEEDBACK__, or fails
       once with the text of window.__DS_FEEDBACK_FAIL__), the app posts it (crates/app/src/feedback.rs). */
    feedbackSend: async (payload, url) => {
      if (bridge()) {
        (window.__DS_FEEDBACK__ = window.__DS_FEEDBACK__ || []).push(JSON.parse(JSON.stringify(payload)));
        if (window.__DS_FEEDBACK_FAIL__) {
          const text = String(window.__DS_FEEDBACK_FAIL__);
          delete window.__DS_FEEDBACK_FAIL__;
          const err = new Error(text.replace(/^\d+:\s*/, ""));
          err.status = parseInt(text, 10) || 500;
          throw err;
        }
        return { ok: true, issue_url: "https://github.com/Cherry-Blossom-Valley/drumspike/issues/1000" };
      }
      return invoke("feedback_send", { payload, url: url || FEEDBACK_URL });
    },
    /* native pickers: the path, or null */
    pickDir: async (title, start) => {
      const r = await T.dialog.open({ directory: true, multiple: false, title, defaultPath: start || undefined });
      return typeof r === "string" ? r : null;
    },
    pickFile: async (title, start) => {
      const r = await T.dialog.open({ directory: false, multiple: false, title, defaultPath: start || undefined,
        filters: [{ name: "Audio", extensions: ["wav", "wave", "bwf", "aif", "aiff"] }] });
      return typeof r === "string" ? r : null;
    },
  };
}

/* Add tracks (P3-4c). Commands: tracks_scan {paths, preset}, tracks_add {sid, paths, roles, notes, copy, progress},
   kit_presets_list, kit_preset_save {name, rules}, kit_preset_load {name} (crates/app/src/tracks.rs, roles.rs).
   tracksScan answers {files: [{path, name, stem, size, sr, role, note, guess}], rejected: [{path, name, reason}],
   guessed}; tracksAdd(sid, paths, roles, notes, copy, onProgress) answers {added, tracks, rejected, elapsed_ms};
   onProgress gets {state, done, total, file} after each file. A file stays where it is unless copy is true. */
function tracksMethods(invoke, T) {
  return {
    tracksScan: (paths, preset) => invoke("tracks_scan", { paths, preset: preset || null }),
    tracksAdd: (sid, paths, roles, notes, copy, onProgress) => {
      const ch = new T.core.Channel();
      ch.onmessage = m => { if (onProgress && m && m.add) onProgress(m.add); };
      return invoke("tracks_add", { sid, paths, roles, notes, copy: !!copy, progress: ch });
    },
    /* P4-39a: Track > Remove Track: answers {removed, unfollowed, order, tracks, snapshot}; the snapshot feeds tracksRestore (the undo) */
    tracksRemove: (sid, names) => invoke("tracks_remove", { sid, names, snapshot: true }),
    tracksRestore: (sid, tracks) => invoke("tracks_restore", { sid, tracks }),
    /* 0.9.1: the role of a track (Inspector); answers {role, note, role_source, suggest_note, reanalyzed, previous} */
    trackSetRole: (sid, name, role, note = null, source = null, roleMatch = null, events = null) => invoke("track_set_role", { sid, name, role, note, source, role_match: roleMatch, events }),
    kitPresets: () => invoke("kit_presets_list", {}),
    kitPresetSave: (name, rules) => invoke("kit_preset_save", { name, rules }),
    kitPresetLoad: name => invoke("kit_preset_load", { name }),
    /* the native open dialog for audio files (several); the list of paths, or null */
    pickAudioFiles: async () => {
      const r = await T.dialog.open({ multiple: true, directory: false, title: "Add tracks",
        filters: [{ name: "Audio", extensions: ["wav", "wave", "bwf", "aif", "aiff"] }] });
      return Array.isArray(r) ? r : (typeof r === "string" && r ? [r] : null);
    },
  };
}

/* Copy jobs and relink (P3-4d). Commands: project_save_as {sid, name, dir, copy_audio, progress}, project_consolidate
   {sid, progress}, job_status {sid}, job_cancel {sid}, project_relink {sid, track, path, force}, relink_search
   {sid, folder} (crates/app/src/copyjob.rs, relink.rs).
   A copy job answers {job} at once and sends {sid, copy: {id, kind, state, done, total, bytes_done, bytes_total, file}}
   on the channel until state is done (result), cancelled or error (error). saveAs(sid, dir, name, copyAudio, onProgress)
   resolves the summary of the new project (a Save As without copy answers at once, like before) or {cancelled: true};
   consolidate(sid, onProgress) resolves {consolidated, dirty} or {cancelled: true}. A lost channel message does not hang
   the page: job_status is asked every 500 ms as well. jobCancel(sid) stops the running job of the project (also a
   copy-mode tracksAdd and a relink search). relinkSearch(sid, folder) answers {offline, relinked, found, ambiguous,
   missing, ...}; relink keeps the file picker flow (409 for a file that does not match, unless force). */
function jobMethods(invoke, T) {
  const FINAL = ["done", "cancelled", "error"];
  function runJob(sid, start, onProgress) {
    return new Promise((resolve, reject) => {
      const ch = new T.core.Channel();
      let over = false, timer = null;
      const finish = st => {
        if (over) return; over = true; clearInterval(timer);
        if (st.state === "done") resolve(st.result);
        else if (st.state === "cancelled") resolve({ cancelled: true });
        else reject(Object.assign(new Error(`500: ${st.error || "the job failed"}`), { status: 500 }));
      };
      ch.onmessage = m => {
        if (!m || !m.copy || over) return;
        try { if (onProgress) onProgress(m.copy); } catch (e) { /* a broken callback must not stop the job */ }
        if (FINAL.includes(m.copy.state)) finish(m.copy);
      };
      start(ch).then(r => {
        const st = (r && r.job) || r;
        if (st && FINAL.includes(st.state)) { finish(st); return; }
        timer = setInterval(async () => {
          try { const st2 = await invoke("job_status", { sid }); if (FINAL.includes(st2.state)) finish(st2); } catch (e) { /* the channel still ends it */ }
        }, 500);
      }).catch(e => { over = true; clearInterval(timer); reject(e); });
    });
  }
  return {
    saveAs: (sid, dir, name, copyAudio, onProgress) => copyAudio
      ? runJob(sid, ch => invoke("project_save_as", { sid, dir, name, copy_audio: true, progress: ch }), onProgress)
      : (() => { const ch = new T.core.Channel(); ch.closed = true; /* the command takes a channel; this one stays unused */
          return invoke("project_save_as", { sid, dir, name, copy_audio: false, progress: ch }); })(),
    consolidate: (sid, onProgress) => runJob(sid, ch => invoke("project_consolidate", { sid, progress: ch }), onProgress),
    jobCancel: sid => invoke("job_cancel", { sid }),
    jobStatus: sid => invoke("job_status", { sid }),
    relink: (sid, track, path, force) => invoke("project_relink", { sid, track, path, force: !!force }),
    relinkSearch: (sid, folder) => invoke("relink_search", { sid, folder }),
  };
}

/* The importer of old sessions (P3-4e; the zip import and export went in P4-30). Commands: legacy_list {folder},
   project_import {session_dir} (crates/app/src/import.rs). The app has no sessions/ store: the user picks the folder
   of old sessions (pickImportFolder). legacyList(folder) rows are {session_id, name, tracks, dir}; importLegacy(session_id)
   imports the row of the last listing. */
function importMethods(invoke, T) {
  const dirs = new Map();
  const one = r => (typeof r === "string" && r ? r : null);
  return {
    legacyList: async folder => {
      if (!folder) return [];
      const rows = await invoke("legacy_list", { folder });
      for (const r of rows) dirs.set(r.session_id, r.dir);
      return rows;
    },
    importLegacy: id => invoke("project_import", { session_dir: dirs.get(id) || id }),
    pickImportFolder: async () => one(await T.dialog.open({ directory: true, multiple: false, title: "Folder with old Drum Spike sessions" })),
  };
}

/* Transport (P3-5b): play, pause, seek, mute, solo and clicks on the native transport of the core
   (crates/app/src/transport.rs). The page never gets audio data: it sends the state and gets the playhead.
   transportPlay(sid, position, events, onPlayhead, speed): events = {track: [t, ...]} (the hits the clicks follow);
   speed (P3-20b) 0.5 plays at half speed, one octave down; without it (or 1) at normal speed;
   onPlayhead gets {sid, epoch, t_ns, frame, rate, pos, playing, underruns, reopens, end} about every 33 ms. */
function transportMethods(invoke, T) {
  return {
    transportPlay: (sid, position, events, onPlayhead, speed) => {
      const progress = new T.core.Channel();
      progress.live = true;   // the test bridge polls a live channel until a message has end: true
      progress.onmessage = onPlayhead;
      return invoke("transport_play", { sid, position, events, progress, speed: speed || null });
    },
    /* P4-40d: an audition: o = {track, start, end, loop, speed}. The host solos `track` for this play only and plays the
       window; no playhead messages come (the channel is only there because the app's command takes one).
       P4-40e: o.shot (a track name) plays that track's layer one-shot once at the window start instead of the hits. */
    transportAudition: (sid, o) => {
      const progress = new T.core.Channel();
      const win = { sid, start: o.start, end: o.end, loop: !!o.loop, tracks: [o.track], speed: o.speed === 0.5 ? 0.5 : null, progress };
      if (o.shot) win.shot = o.shot;
      return invoke("transport_play", win).finally(() => { progress.closed = true; });
    },
    transportStop: close => invoke("transport_stop", { close: !!close }),
    transportSeek: position => invoke("transport_seek", { position }),
    transportMix: (mute, solo) => invoke("transport_mix", { mute, solo }),
    transportClicks: (enabled, volume, pitches) => invoke("transport_clicks", { enabled, volume, pitches }),
    transportStatus: () => invoke("transport_status", {}),
    /* P3-16: volume (dB, -120 is off, up to +12) and pan (-1 to +1) of a track, kept in the project; answers
       {volume_db, pan, applied}. The master volume in dB, kept in the app settings. The output devices and the
       choice: {devices: [{name, rate, channels, default}], choice, current, note, null_output, master_db}. */
    trackSetMix: (sid, name, volumeDb, pan) => invoke("track_set_mix", { sid, name, volume_db: volumeDb, pan }),
    transportMaster: volumeDb => invoke("transport_master", { volume_db: volumeDb }),
    audioDevices: () => invoke("outputs_list", {}),
    audioOutput: device => invoke("output_select", { device: device || null }),
  };
}

/* Bridge transport (P3-7a, test builds only): the same shape as window.__TAURI__ (core.invoke,
   core.Channel, dialog.open, dialog.save, event.listen), over POST /invoke/<command> of the debug binary ds-bridge
   (crates/app/src/bridge.rs). ds-bridge puts window.__DS_BRIDGE__ into index.html; the real
   app has no such flag. A Channel is fetched by long poll: GET /channel/<id>. The dialog
   answers window.__DS_PICK__ (a test sets it). Errors are {code, message}, as in the app. */
function bridgeTransport() {
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  let seq = 0;
  class Channel {
    constructor() { this.id = `c${Date.now().toString(36)}${(++seq).toString(36)}${Math.random().toString(36).slice(2, 8)}`; this.onmessage = null; this.closed = false; }
    toJSON() { return { __channel: this.id }; }
    async poll() {   // started after the command that got the channel; ends with the job
      for (let i = 0; (this.live || i < 600) && !this.closed; i++) {   // a live channel (playhead) runs until its end message
        let msgs = [];
        try { msgs = await (await fetch(`/channel/${this.id}?timeout_ms=2000`, { cache: "no-store" })).json(); }
        catch (e) { await sleep(300); }
        for (const m of msgs) {
          if (this.onmessage) this.onmessage(m);
          if (m && ((m.job && m.job.state === "done") || (m.export && m.export.state === "done") || (m.copy && ["done", "cancelled", "error"].includes(m.copy.state)) || (this.live && m.end === true && !m.error))) this.closed = true;
        }
      }
    }
  }
  async function invoke(cmd, args) {
    args = args || {};
    // P3-4d: the poll starts before the command, so a command that runs long (tracks_add with the copy) shows its
    // progress while it runs; the Tauri Channel delivers live as well. Messages wait in the queue until taken.
    const chans = Object.values(args).filter(v => v instanceof Channel);
    for (const v of chans) v.poll();
    let r;
    try {
      r = await fetch(`/invoke/${cmd}`, { method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify(args), cache: "no-store" });
    } catch (e) { for (const v of chans) v.closed = true; throw e; }
    if (!r.ok) { for (const v of chans) v.closed = true; throw await r.json().catch(() => ({ code: r.status, message: "bridge error" })); }
    const out = (r.headers.get("content-type") || "").startsWith("application/json") ? await r.json() : await r.arrayBuffer();
    return out;
  }
  /* events (project-dirty): one long poll of GET /events, started by the first listen(). The bridge keeps a
     numbered log and a read takes nothing away; the first request (after=now) starts at the end of the log. */
  const listeners = {};
  let polling = false;
  async function pollEvents() {
    polling = true;
    let after = "now";
    for (;;) {
      let r = null;
      try { r = await (await fetch(`/events?after=${after}&timeout_ms=2000`, { cache: "no-store" })).json(); }
      catch (e) { await sleep(300); continue; }
      after = r.seq;
      for (const m of r.events) for (const f of listeners[m.event] || []) f({ event: m.event, payload: m.payload });
    }
  }
  return {
    core: { invoke, Channel },
    dialog: { open: async opts => { window.__dialogOpts = opts; return window.__DS_PICK__ === undefined ? null : window.__DS_PICK__; },
      save: async opts => { window.__saveOpts = opts; return window.__DS_SAVE__ === undefined ? null : window.__DS_SAVE__; } },
    event: { listen: async (name, cb) => { (listeners[name] = listeners[name] || []).push(cb); if (!polling) pollEvents(); return () => { listeners[name] = listeners[name].filter(f => f !== cb); }; } },
  };
}

const B = ipcBackend(window.__TAURI__ && window.__TAURI__.core ? window.__TAURI__ : bridgeTransport());
