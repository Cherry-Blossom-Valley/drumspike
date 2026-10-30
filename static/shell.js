/* Native shell glue (P3-4b, P3-13a). Loaded between backend.js and app.js. In the desktop app the native menu
   bar (apps/desktop/src-tauri/src/menu.rs) sends the event "menu" {id, path?, sid?}; this file turns it into
   the page's own functions (fileNew, saveProjectNow, zoomStep, ...). P3-13a: the menu bar is the only menu;
   the window has no sidebar, no File menu and no ? button. P3-13b: the keys come from static/keymap.js. The page tells the shell the state of the check
   items (snap, follow mode, click, Inspector) and the current project (the Window menu) with every sync.
   It also holds the pieces that app.js calls by name: the quit flow, the project rename dialog, Recompute Velocities, the native replacement of the folder browser, and the handling
   of paths from the Finder. Every function runs at call time, so the globals of app.js (S, B, setMsg,
   askConfirm, ...) exist then. The test bridge sends the same event "menu" (command selftest_menu). */
(function () {
  "use strict";
  const $q = s => document.querySelector(s);

  /* ---------------- helpers ---------------- */
  /* a dialog that waits for an answer; the sheets (class "sheet": Settings, Keyboard Shortcuts) wait for nothing:
     a menu item closes them and runs */
  const modalOpen = () => !!document.querySelector(".modal:not(.hidden):not(.sheet)");
  const closeSheets = () => document.querySelectorAll(".modal.sheet:not(.hidden)").forEach(m => m.classList.add("hidden"));
  const textFocus = () => {
    const e = document.activeElement;
    return !!e && (e.tagName === "INPUT" || e.tagName === "TEXTAREA" || e.isContentEditable);
  };
  /* The accelerators of the native menu come from the key table (static/keymap.js, P3-13b); menu.rs reads the
     same table. The menu runs these keys, so app.js must not run them again. Redo with Cmd-Y is a page key (the
     menu has Shift-Cmd-Z only), so app.js keeps Cmd-Y. Keys without Cmd, Control or Option (Space, Tab, Return,
     letters) are page keys: a text field keeps them (P3-13a). */
  const menuOwnsKey = ev => DSKeys.menuOwns(ev);

  /* ---------------- quit flow (window close button, Cmd-Q, Dock Quit) ---------------- */
  let quitting = false;
  /* Asks Save, Discard or Cancel once for each project with unsaved changes. Cancel ends the flow and
     the app stays open. After the last answer the app ends (B.quitApp). Resolves true when it quit. */
  async function quitFlow() {
    if (quitting) return false;
    quitting = true;
    try {
      await flushEvents();   // the pending edits of the page reach the service first
      const dirty = (await B.sessions()).filter(s => s.dirty);
      for (const s of dirty) {
        const a = await askConfirm({ title: "Unsaved changes",
          msg: `"${s.name}" has unsaved changes. Save before you quit? "Discard" drops the unsaved edits (the last autosave stays in Backups/).`,
          buttons: [{ act: "save", label: "Save", cls: "primary" }, { act: "discard", label: "Discard" }, { act: "cancel", label: "Cancel" }] });
        if (a === "save") {   // #127: a file changed on disk asks Save anyway / Reload from disk / Quit without saving
          let r;
          try { r = await saveOrChoose(s.sid, s.name, "Quit without saving"); }
          catch (e) { setMsg(`save failed: ${e.message}`, "error"); return false; }
          if (r === "reloaded") return false;   // the quit stops; the project shows the file from disk
          if (!r) { setMsg("quit cancelled"); return false; }
        } else if (a !== "discard") { setMsg("quit cancelled"); return false; }
      }
      await B.quitApp();
      return true;
    } catch (e) { setMsg(`quit failed: ${e.message}`, "error"); return false; }
    finally { quitting = false; }
  }

  /* ---------------- rename (File > Rename Project; replaces window.prompt) ---------------- */
  async function renameProject(s, name) {
    try { await B.renameSession(s.sid, name); }
    catch (e) { setMsg(`rename failed: ${e.message}`, "error"); return false; }
    if (s.sid === S.sid) { setProjName(name); setDirtyState(true); }
    await initSessions();
    return true;
  }
  /* P4-28: File > Rename Project opens a small dialog (#renameModal). The toolbar shows no project name; the name is in
     the window title. Enter = Rename, Escape = Cancel; keydown stops here so the page's shortcuts do not run. A
     refresh of the project list does not touch the dialog (it is static markup, nothing rebuilds it). */
  function renameCurrent() {
    if (!needProject()) return;
    const modal = $q("#renameModal"), input = $q("#renameField");
    if (!modal.classList.contains("hidden")) { input.focus(); return; }
    const s = { sid: S.sid, name: S.name };
    input.value = s.name || "";
    modal.classList.remove("hidden");
    const close = () => { modal.classList.add("hidden"); input.onkeydown = null; $q("#renameOk").onclick = null; $q("#renameCancel").onclick = null; };
    const ok = async () => {
      const nm = input.value.trim();
      close();
      if (nm && nm !== s.name) await renameProject(s, nm);
    };
    $q("#renameOk").onclick = ok;
    $q("#renameCancel").onclick = close;
    input.onkeydown = ev => {
      ev.stopPropagation();   // the page's shortcuts must not run while the user types
      if (ev.key === "Enter") { ev.preventDefault(); ok(); }
      else if (ev.key === "Escape") { ev.preventDefault(); close(); }
    };
    input.focus(); input.select();
  }

  /* ---------------- Recompute Velocities (Edit menu) ---------------- */
  /* One undo step for all tracks: op {type: "vel", tracks: {name: {before: {id: v}, after: {id: v}}}} */
  function applyVel(op, reverse) {
    for (const [name, t] of Object.entries(op.tracks)) {
      const to = reverse ? t.before : t.after;
      for (const e of S.events[name] || []) if (e.id in to) e.v = to[e.id];
      markDirty(name);
    }
    invalidateHits(); refreshCounts();   // P3-14d: the hit lines in the next frame, no GL draw
  }
  async function recomputeVelocities() {
    if (!needProject()) return;
    await flushEvents();
    const names = S.order.filter(n => (S.events[n] || []).length && !(S.tracks[n] && S.tracks[n].offline));
    if (!names.length) { setMsg("no hits to recompute: run Analyze first"); return; }
    const op = { type: "vel", tracks: {} };
    let changed = 0;
    const failed = [];
    for (const n of names) {
      try {
        const sent = S.events[n];
        const r = await B.recomputeVelocities(S.sid, n, sent);
        const before = {}, after = {};
        (r.events || []).forEach((e, i) => {
          const old = sent[i];
          if (old && e.id !== undefined && old.id === e.id && old.v !== e.v) { before[e.id] = old.v; after[e.id] = e.v; }
        });
        const k = Object.keys(after).length;
        if (k) { op.tracks[n] = { before, after }; changed += k; }
      } catch (e) { failed.push(`${n}: ${e.message}`); }
    }
    if (changed) { applyVel(op, false); pushUndo(op); }
    setMsg(`velocities recomputed for ${names.length - failed.length} track(s): ${changed} hit(s) changed` + (failed.length ? `; failed: ${failed.join("; ")}` : ""));
  }

  /* ---------------- About (P3-8a, #294: model-free) ---------------- */
  /* Help > About: the version, what the app does and a button for the third-party list. The version and the
     list come from the shell (B.aboutInfo, B.aboutThirdParty); the plain line is here. Page text goes in with textContent. */
  const ABOUT_SUMMARY = "Drum Spike finds the hits in your close-mic drum tracks and writes MIDI and spike files for them.";
  /* #18: under the summary line: the CBV logo, the licence line and the two links (their short addresses).
     A click on a link or on the logo asks the shell to open one fixed address by id (open_link); the
     page never passes an address. The test bridge records the id in window.__DS_OPEN_LINK__. */
  const ABOUT_LICENCE = "Drum Spike is free and open source (Apache-2.0).";
  const ABOUT_LINKS = [
    ["website", "aboutLinkWebsite", "drumspike.cherry-blossom-valley.com"],
    ["source", "aboutLinkSource", "github.com/Cherry-Blossom-Valley/drumspike"],
  ];
  function aboutBox() {
    let m = $q("#aboutModal");
    if (m) return m;
    m = document.createElement("div"); m.id = "aboutModal"; m.className = "modal hidden";
    const box = document.createElement("div"); box.className = "modal-box";
    const el = (tag, id, cls) => { const e = document.createElement(tag); if (id) e.id = id; if (cls) e.className = cls; box.appendChild(e); return e; };
    const art = document.createElement("img"); art.id = "aboutArt"; art.src = "/brand/drumspike-icon.svg"; art.alt = "Drum Spike icon"; art.draggable = false;   // P3-17: the splash art
    art.width = 96; art.height = 96; art.style.cssText = "display:block;margin:0 auto 6px"; box.appendChild(art);
    el("h3", "aboutTitle").textContent = "About Drum Spike";
    el("p", "aboutVersion", "hint");
    el("p", "aboutSummary").textContent = ABOUT_SUMMARY;
    const logo = document.createElement("img");
    logo.id = "aboutLogo"; logo.src = "/brand/cbv-logo-white.png"; logo.alt = "Cherry Blossom Valley"; logo.draggable = false;
    logo.style.cssText = "display:block;margin:10px auto 4px;height:32px;width:auto;cursor:pointer";
    logo.onclick = () => { B.openLink("cbv"); };
    box.appendChild(logo);
    el("p", "aboutLicence", "hint").textContent = ABOUT_LICENCE;
    const links = el("p", "aboutLinks");
    for (const [id, elId, label] of ABOUT_LINKS) {
      const a = document.createElement("span");
      a.id = elId; a.textContent = label;
      a.style.cssText = "cursor:pointer;text-decoration:underline;color:#7ab8ff;margin:0 10px";
      a.onclick = () => { B.openLink(id); };
      links.appendChild(a);
    }
    const third = el("pre", "aboutThird", "hidden selectable"); third.style.cssText = "white-space:pre-wrap;max-height:240px;overflow:auto;font-size:11px";
    const acts = el("div", "aboutActions", "modal-actions");
    const mk = (act, label, cls) => { const b = document.createElement("button"); b.className = "btn" + (cls ? " " + cls : ""); b.textContent = label; b.dataset.act = act; acts.appendChild(b); return b; };
    mk("third", "Third-party software"); mk("ok", "OK", "primary");
    document.body.appendChild(m); m.appendChild(box);
    return m;
  }
  async function about() {
    const m = aboutBox();
    let info = null;
    try { info = await B.aboutInfo(); } catch (e) { info = null; }
    info = info || {};
    $q("#aboutVersion").textContent = `Version ${info.version || "unknown"}` + (info.core_version ? ` (core ${info.core_version})` : "");
    const third = $q("#aboutThird"); third.classList.add("hidden"); third.textContent = "";
    m.classList.remove("hidden");
    await new Promise(resolve => {
      const done = () => { m.classList.add("hidden"); document.removeEventListener("keydown", esc, true); resolve(); };
      const esc = ev => { if (ev.key === "Escape") { ev.stopPropagation(); done(); } };
      document.addEventListener("keydown", esc, true);
      m.querySelector('button[data-act="ok"]').onclick = done;
      m.querySelector('button[data-act="third"]').onclick = async () => {
        if (!third.classList.contains("hidden")) { third.classList.add("hidden"); return; }
        let t = "";
        try { t = await B.aboutThirdParty(); } catch (e) { t = `The third-party list is not available: ${e.message}`; }
        third.textContent = t; third.classList.remove("hidden"); third.scrollTop = 0;
      };
    });
  }

  /* ---------------- Feedback (part 2 of #153, #164) ---------------- */
  /* The Feedback button at the top right opens a form (kind, title, description, optional steps) with
     a live preview of the exact JSON the app posts to the CBV feedback gateway (B.feedbackSend; the
     test bridge answers the command, so e2e never needs the network). The meta holds counts only:
     never a file name, a folder path, a project name, audio, a user or a host name. The same
     private-word guard as the docs (tools/media/check_paths.py BANNED_BASE) runs on the free text
     before anything is sent, and again in crates/app/src/feedback.rs before the POST. */
  const FB_KINDS = [["bug", "Bug"], ["improvement", "Improvement"], ["other", "Other"]];
  const FB_PRIVATE = ["/Users", "/var/folders", "/home", "/private/tmp", "x86-mac", "admin@", "arm64-mac", "linux-arm64"];  // private-list-definition
  function fbHits(text) {
    const out = new Set(), low = text.toLowerCase();
    for (const w of FB_PRIVATE) if (low.includes(w)) out.add(w);
    for (const tok of text.split(/\s+/)) {
      const t = tok.replace(/^[^\w]+|[^\w]+$/g, "");
      if ((t.startsWith("/") && t.includes("/")) || t.startsWith("~/") || /^[A-Za-z]:[\\/]/.test(t)) out.add(t);
    }
    return [...out];
  }
  async function feedbackMeta() {
    let info = {}, back = {};
    try { info = await B.aboutInfo(); } catch (e) { /* the counts stay */ }
    try { back = await B.backendInfo(); } catch (e) { /* the defaults stay */ }
    let dur = 0, sr = null;
    for (const n of S.order) {
      const p = S.peaks[n];
      if (p && p.sr) { sr = p.sr; dur = Math.max(dur, (p.n_samples || 0) / p.sr); }
    }
    let hits = 0;
    for (const n of Object.keys(S.events)) hits += (S.events[n] || []).length;
    const ua = navigator.userAgent;
    return {
      app_version: info.version || "unknown",
      build_sha: info.build_sha || "unknown",
      core_version: info.core_version || back.core || "unknown",
      os: (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "unknown",
      arch: /arm|aarch/i.test(ua) ? "arm64" : (/x86_64|x64|Win64|amd64/i.test(ua) ? "x86_64" : "unknown"),
      language: navigator.language || "unknown",
      detector: back.detector || "rust",
      counts: { tracks: S.order.length, hits, sample_rate: sr, session_length_s: Math.round(dur) },   // counts only, never a name or a path
    };
  }
  function feedbackBox() {
    let m = $q("#fbModal");
    if (m) return m;
    m = document.createElement("div"); m.id = "fbModal"; m.className = "modal hidden";
    const box = document.createElement("div"); box.className = "modal-box fbBox";
    const el = (tag, id, cls) => { const e = document.createElement(tag); if (id) e.id = id; if (cls) e.className = cls; box.appendChild(e); return e; };
    el("h3", "fbTitle").textContent = "Send Feedback";
    el("p", "fbIntro", "phint").textContent = "Goes to the Cherry Blossom Valley feedback gateway and opens an issue in the Drum Spike repo. The preview below is the exact text that is sent.";
    const row = el("div", "fbKindRow", "prow");
    const lab = document.createElement("label"); lab.textContent = "Kind "; row.appendChild(lab);
    const sel = document.createElement("select"); sel.id = "fbKind";
    for (const [v, t] of FB_KINDS) { const o = document.createElement("option"); o.value = v; o.textContent = t; sel.appendChild(o); }
    row.appendChild(sel);
    const mk = (id, label, rows) => {
      const wrap = el("div", id + "Wrap", "fbField");
      const l = document.createElement("label"); l.htmlFor = id; l.textContent = label; wrap.appendChild(l);
      const f = document.createElement(rows ? "textarea" : "input"); f.id = id;
      if (rows) f.rows = rows;
      wrap.appendChild(f);
      return f;
    };
    mk("fbTitleIn", "Title (one line)").type = "text";
    mk("fbDesc", "What happened?", 4);
    mk("fbSteps", "Steps to reproduce (optional)", 3);
    el("div", "fbPreviewLabel", "ptitle").textContent = "Preview (exactly what is sent)";
    el("pre", "fbPreview", "fbPreview");
    el("div", "fbMsg", "phint");
    const acts = el("div", "fbActions", "modal-actions");
    const cancel = document.createElement("button"); cancel.id = "fbCancel"; cancel.className = "btn"; cancel.textContent = "Cancel"; acts.appendChild(cancel);
    const send = document.createElement("button"); send.id = "fbSend"; send.className = "btn primary"; send.textContent = "Send"; acts.appendChild(send);
    document.body.appendChild(m); m.appendChild(box);
    return m;
  }
  async function feedback() {
    const m = feedbackBox();
    const meta = await feedbackMeta();
    /* The gateway URL is the app setting `feedback_url`, the built-in default when it is not set
       (so it can change without a release; the owner's answer on #164). */
    let settings = {};
    try { settings = await B.appSettingsGet() || {}; } catch (e) { /* the default stays */ }
    const gateway = typeof settings.feedback_url === "string" && settings.feedback_url ? settings.feedback_url : null;
    const fields = [$q("#fbKind"), $q("#fbTitleIn"), $q("#fbDesc"), $q("#fbSteps")];
    const payload = () => {
      const p = { kind: $q("#fbKind").value, title: $q("#fbTitleIn").value.trim(), description: $q("#fbDesc").value };
      if ($q("#fbSteps").value.trim()) p.steps = $q("#fbSteps").value.trim();
      p.meta = meta;
      return p;
    };
    const refresh = () => {
      $q("#fbPreview").textContent = JSON.stringify(payload(), null, 2);
      const hits = fbHits([$q("#fbTitleIn").value, $q("#fbDesc").value, $q("#fbSteps").value].join("\n"));
      const bad = hits.length || !$q("#fbTitleIn").value.trim();
      $q("#fbSend").disabled = bad;
      const msg = $q("#fbMsg");
      if (hits.length) msg.textContent = `Not sent: the text holds a private name or path (${hits.join(", ")}). Remove it and send again.`;
      else if (!msg.textContent.startsWith("Sent")) msg.textContent = "";
    };
    for (const f of fields.slice(1)) f.oninput = refresh;
    refresh();
    m.classList.remove("hidden");
    await new Promise(resolve => {
      const done = () => { m.classList.add("hidden"); document.removeEventListener("keydown", esc, true); resolve(); };
      const esc = ev => { if (ev.key === "Escape") { ev.stopPropagation(); done(); } };
      document.addEventListener("keydown", esc, true);
      $q("#fbCancel").onclick = done;
      $q("#fbSend").onclick = async () => {
        const btn = $q("#fbSend");
        btn.disabled = true;
        try {
          const r = await B.feedbackSend(JSON.parse($q("#fbPreview").textContent), gateway);   // exactly the previewed JSON
          $q("#fbMsg").textContent = "Sent. Thank you" + (r && r.issue_url ? `: ${r.issue_url}` : "") + ".";
          $q("#fbTitleIn").value = ""; $q("#fbDesc").value = ""; $q("#fbSteps").value = "";
          refresh();
        } catch (e) {
          $q("#fbMsg").textContent = `Not sent: ${e.message} The text is kept; try again.`;
        } finally {
          btn.disabled = false;
        }
      };
    });
  }

  /* ---------------- native dialogs instead of the in-page folder browser (route 12) ---------------- */
  /* Answers like pickFolder(): {dir, name?, copy?, file?} or null (the native dialogs; the page has no folder browser). */
  async function nativePick(o) {
    const split = path => { const p = path.replace(/\/+$/, ""), i = p.lastIndexOf("/"); return { dir: i > 0 ? p.slice(0, i) : "/", base: p.slice(i + 1) }; };
    if (o.file) {
      const path = await B.pickFile(o.title || "Choose a file", o.start || "");
      if (!path) return null;
      const p = split(path);
      return { dir: p.dir, file: p.base };
    }
    if (o.name) {
      const r = await B.pickSave(o.title || "Save", typeof o.name === "string" ? o.name : "");
      return r ? { dir: r.dir, name: r.name, copy: false } : null;
    }
    const dir = await B.pickDir(o.title || "Choose a folder", o.start || "");
    return dir ? { dir } : null;
  }

  /* ---------------- paths from the Finder, the Dock and drops ---------------- */
  const hooks = { otherPaths: paths => setMsg(`${paths.length} dropped item(s) are no Drum Spike project`) };   // app.js sets it to addTracksFlow (P3-4c)
  /* The queue of the shell holds the paths; the event "open-paths" is only a nudge. */
  async function takeOpenPaths() {
    const r = await B.takeOpenPaths();
    const projects = (r && r.projects) || [], others = (r && r.others) || [];
    for (const p of projects) await openProjectPath(p);
    if (others.length) hooks.otherPaths(others, (r && r.pos) || null);   // P4-7a: the drop position (null: no drop)
    return { projects, others };
  }

  /* ---------------- menu events ---------------- */
  const handlers = {
    new: () => fileNew(),
    open: () => fileOpen(),
    recent: p => (p.path ? openProjectPath(p.path) : undefined),
    recent_clear: async () => {   // #441: File > Open Recent > Clear Recent empties the list and the menu
      await B.recentClear();
      await renderRecentList();
      setMsg("recent projects cleared");
    },
    open_recent: async () => {   // P3-19: File > Open Most Recent (the first entry of Open Recent)
      let list = [];
      try { list = await B.recentProjects(); } catch (e) { list = []; }
      if (list && list.length) return openProjectPath(list[0].folder);
      setMsg("no recent projects");
    },
    close: () => { if (needProject()) return closeProject(S.sid, S.name); },
    save: () => { if (needProject()) return saveProjectNow(); },
    save_as: () => fileSaveAs(),
    export: () => openExport("all"),   // P4-30: the Export dialog (app.js)
    export_again: () => exportAgain(),   // P4-31: the last export again: the same files to the same place, no dialog (app.js)
    export_selected: () => exportSelectedTracks(),
    tracks_remove: () => removeSelectedTrack(),   // P4-39a: Track > Remove Track (app.js)
    prev_track: () => { if (needProject()) selectTrackStep(-1); },   // #158 (rank 8): Track > Select Previous Track (app.js)
    next_track: () => { if (needProject()) selectTrackStep(1); },
    undo: () => (textFocus() ? document.execCommand("undo") : undo()),
    redo: () => (textFocus() ? document.execCommand("redo") : redo()),
    undo_history: () => openUndoHistory(),   // P4-40a: Edit > Undo History (app.js)
    recompute_velocities: () => recomputeVelocities(),
    about: () => about(),
    quit: () => quitFlow(),
    add_tracks: () => { if (needProject()) return addTracksDialog(); },   // P3-4c: the native file dialog, then the confirm list (app.js)
    consolidate: () => { if (needProject()) return fileConsolidate(); },   // P3-4d: a job with progress and cancel (app.js)
    import_old: () => fileImport(),   // P3-4e: the folder dialog, then the list of old sessions (app.js)
    // P3-13a: the items of the removed sidebar, File menu and ? button, and the menu bar of ux-daw-reference.md 3.2
    settings: () => openSettings(),
    rename: () => renameCurrent(),
    delete_hit: () => { if (needProject() && !tmDeleteSel() && !lnDeleteMark()) deleteSelectedHit(); },   // P4-6a: a selected Learn mark bar first   // P4-8: a selected tempo or meter marker first
    markers_set: () => { if (needProject()) markerAtSelection(); },   // #158 (rank 8): Edit > Set Marker at Selection (app.js)
    marker_next: () => { if (needProject()) markerJump(1); },   // #158 (rank 8): Transport > Next Marker
    marker_prev: () => { if (needProject()) markerJump(-1); },
    audition: () => { if (needProject()) audition(false); },   // #158 (rank 8): Hits > Audition Hit (the Shift loop stays the page's Shift+A)
    add_hit: () => addHitAtPlayhead(),
    nudge_earlier: () => { if (needProject()) nudgeUnits(-1); },
    nudge_later: () => { if (needProject()) nudgeUnits(1); },
    nudge_val_1smp: () => chooseNudgeValue("1smp"),   // P3-20a: Edit > Nudge Value (radio items)
    nudge_val_10smp: () => chooseNudgeValue("10smp"),
    nudge_val_100smp: () => chooseNudgeValue("100smp"),
    nudge_val_1ms: () => chooseNudgeValue("1ms"),
    nudge_val_10ms: () => chooseNudgeValue("10ms"),
    amp_in: () => stepAmpZoom(1),   // P3-20a: View > Waveform Zoom
    amp_out: () => stepAmpZoom(-1),
    amp_reset: () => { setAmpZoom(1); setMsg("amplitude zoom: 1x"); },
    zoom_in: () => { if (needProject()) zoomStep(true); },
    zoom_out: () => { if (needProject()) zoomStep(false); },
    zoom_project: () => { if (needProject()) zoomToProject(); },
    zoom_full: () => { if (needProject()) zoomPreset(0); },
    zoom_toggle: () => { if (needProject()) zoomToggle(false); },   // #158 (rank 8): View > Zoom Toggle (app.js)
    zoom_30s: () => { if (needProject()) zoomPreset(1); },
    zoom_5s: () => { if (needProject()) zoomPreset(2); },
    zoom_500ms: () => { if (needProject()) zoomPreset(3); },
    zoom_50ms: () => { if (needProject()) zoomPreset(4); },
    zoom_5ms: () => { if (needProject()) zoomPreset(5); },
    zoom_add: () => { if (needProject()) zoomPresetAdd(); },   // #169: View > Zoom Presets > Add Current
    zoom_organize: () => { if (needProject()) zoomPresetOrganize(); },   // #169: Organize renames and removes the own presets
    zoom_own: p => { if (needProject()) applyOwnZoom(p.n); },   // #169: an own preset (Option+1 to Option+5 for the first five)
    info_line: () => setInfoLine(!SETTINGS().ui.infoLine),   // P3-14b
    overview: () => setOverview(!S.overviewOn),   // #169: View > Show Overview
    counter_seconds: () => setCounter("seconds"),
    counter_samples: () => setCounter("samples"),
    counter_exchange: () => exchangeCounters(),
    play_stop: () => playStop(),
    half_speed: () => halfSpeedPlayStop(),   // P3-20b: Transport > Half-Speed Play
    stop_return: () => { if (needProject()) stopAndReturn(); },
    go_start: () => { if (needProject()) goTo(0); },
    go_end: () => { if (needProject()) goTo(projectEnd()); },
    go_play_start: () => { if (needProject()) goTo(S.playStartPos); },
    follow_smooth: () => setFollow("smooth"),
    follow_page: () => setFollow("page"),
    follow_off: () => setFollow("off"),
    follow_toggle: () => toggleFollow(),
    click: () => toggleClick(),
    next_hit: () => { if (needProject()) gotoEvent(1, false); },
    prev_hit: () => { if (needProject()) gotoEvent(-1, false); },
    next_hit_all: () => { if (needProject()) gotoEvent(1, true); },
    prev_hit_all: () => { if (needProject()) gotoEvent(-1, true); },
    review_next: () => { if (needProject()) reviewStep(1); },   // #150: the review queue
    review_prev: () => { if (needProject()) reviewStep(-1); },
    snap: () => toggleSnap(),
    analyze_track: () => analyzeSelected(),
    analyze_all: () => { if (needProject()) startAnalysis(null); },
    learn_this: () => learnMark("this"),   // P4-6a: Hits > Learn This (K), Forget This (Shift+K), Learn Filter (greyed until P4-6b), Forget Examples of Track
    learn_not: () => learnMark("not"),
    learn_forget_this: () => learnForgetThis(),   // P4-36: Hits > Remove Example (Option+K)
    learn_filter: () => learnFilter(),
    learn_forget: () => learnForget(),
    mute_track: () => trackButton(".bMute"),
    solo_track: () => trackButton(".bSolo"),
    reset_mix: () => resetSelectedMix(),   // P3-16
    follow_own: () => followSelected(null),   // P3-18: Track > Own Detection
    follow: p => (p.name ? followSelected(p.name) : undefined),   // P3-18: Track > Follow Hits Of > <track> (one entry per other track)
    inspector: () => toggleInspector(),   // P3-14a: View > Show Inspector (check item)
    height_taller: () => stepTrackHeight(1),   // P3-14c: View > Track Height (Cmd+Down, Cmd+Up; the sizes are radio items)
    height_shorter: () => stepTrackHeight(-1),
    height_small: () => chooseTrackHeight("small"),
    height_medium: () => chooseTrackHeight("medium"),
    height_large: () => chooseTrackHeight("large"),
    import_tempo_map: () => { if (needProject()) return importTempoMap(); },   // P4-8: File > Import Tempo Map (app.js, block "tempo and meter map")
    detect_tempo: () => { if (needProject()) return detectTempo(); },          // P4-8: Track > Detect Tempo
    set_bar1: () => setBar1AtPlayhead(),                                       // P4-39d: Track > Set Bar 1 at Playhead
    set_bar1_audio: () => setBar1AtAudioStart(),                               // #468: Track > Set Bar 1 at Audio Start
    ruler_tempo: () => tmToggleUi("tempo"),                                    // P4-8: View > Tempo Ruler, Meter Ruler, Bars|Beats Ruler, Show Grid
    ruler_meter: () => tmToggleUi("meter"),
    ruler_beats: () => tmToggleUi("beats"),
    ruler_markers: () => tmToggleUi("markers"),                                // P4-40g: View > Markers Ruler
    grid: () => tmToggleUi("grid"),
    velocity_lane: () => vlToggle(),   // P4-40f: View > Velocity Lane
    other_mics: () => mxToggle(),   // #157: View > Other Mics Under Hit
    suspend_follow: () => toggleSuspendFollow(),   // P4-wf10: Transport > Suspend Follow when Editing
    stop_return_mode: () => toggleStopReturnMode(),   // #436: Transport > Stop Returns to Start
    project: p => (p.sid ? switchTo(p.sid) : undefined),   // Window menu: one entry per open project
    shortcuts: () => openShortcuts(),
  };
  /* Runs the item. While a dialog of the page is open the menu waits: the user answers the dialog first
     (a second dialog would take the place of the first one). After the item the menu gets the state again:
     a check item that the user clicked shows the page's state even when the page refused the item.
     `log` counts the events for the tests (seen: arrived, done: the handler ended). */
  const log = { seen: 0, done: 0, last: null, results: {} };
  async function menu(p) {
    const me = ++log.seen;
    let ok = false;
    try {
      const h = handlers[p.id];
      if (!h) return false;
      if (modalOpen()) { setMsg("answer the open dialog first"); return false; }
      ok = true;
      closeSheets();
      // P4-1b: while a strip of the smooth follow shows, S.view is set in the ticks only (about every 40 ms); a menu
      // event is no DOM event, so the capture listeners of app.js do not run for it: take the view of this moment
      if (typeof spSyncView === "function") spSyncView();
      try { await h(p); }
      catch (e) { setMsg(`${p.id} failed: ${e.message}`, "error"); }
      return true;
    } finally {
      log.done++; log.last = { id: p.id, ok }; log.results[me] = ok;
      sync();
    }
  }

  /* greys the menu items that need a project; refreshes Open Recent and the Window menu; sets the check items */
  let syncing = Promise.resolve(), booted = false;
  function menuState() {
    const sel = S.selTrack && S.tracks[S.selTrack] ? S.selTrack : null;   // P3-18: Track > Follow Hits Of lists the other tracks, the source checked
    return { current: S.sid || null, snap: !!S.snap, follow: S.followMode || "smooth", click: !!SETTINGS().click.enabled,
      infoLine: !!SETTINGS().ui.infoLine, counter: SETTINGS().ui.counter, inspector: !!S.inspector, trackHeight: S.trackHeight, nudgeValue: S.nudgeValue, ampZoom: S.ampZoom,
      overview: !!S.overviewOn, zoomOwn: (S.zoomOwn || []).map(p => p.name),   // #169: Show Overview; the own zoom presets (names, in order)
      tempoRuler: tmShown("tempo"), meterRuler: tmShown("meter"), beatsRuler: tmShown("beats"), markersRuler: tmShown("markers"), grid: tmShown("grid"), velocityLane: VL.on, otherMics: MX.on, keyPreset: DSKeys.preset, projectName: S.sid ? S.name : "", dirty: !!(S.sid && S.dirty),   // P4-26: the window title; P4-22 markersRuler; P4-40f: velocityLane
      suspendFollow: !!SETTINGS().ui.suspendFollowOnEdit,   // P4-wf10: Transport > Suspend Follow when Editing
      stopReturnMode: !!SETTINGS().ui.stopReturnToStart,   // #436: Transport > Stop Returns to Start
      exportAgain: !!S.exportAgain,   // P4-31: File > Export Again is greyed before the project keeps an export shape
      undo: S.undo.length > 0, redo: S.redo.length > 0,   // #158 (rank 9): Edit > Undo and Redo are greyed while their stack is empty
      selHit: !!(S.sel || LN.sel),   // #158 (rank 9): Delete, the nudge items and Audition Hit need a hit or range selection
      tracks: S.sid ? S.order.slice() : [], selected: sel, follows: (sel && S.tracks[sel].follows) || null,
      ...(typeof undoTitles === "function" ? undoTitles() : {}),   // P4-40a: Edit > Undo and Redo name their step
      learnFilter: !!(sel && typeof lnFilterOn === "function" && lnFilterOn(lnTarget(sel))) };   // P4-6c: Hits > Learn Filter
  }
  function sync() {
    // not before boot() has registered the listeners: the first sync tells the shell that the page listens
    if (!B.shellSync || !booted) return Promise.resolve();
    syncing = syncing.then(() => B.shellSync(!!S.sid, menuState())).catch(() => {});
    return syncing;
  }

  async function boot() {
    if (!B.listen) return;
    const swallow = () => {};
    try { await B.listen("menu", p => { menu(p || {}); }); } catch (e) { swallow(e); }
    try { await B.listen("open-paths", () => { takeOpenPaths().catch(swallow); }); } catch (e) { swallow(e); }
    booted = true;
    await sync();
    try { await takeOpenPaths(); } catch (e) { swallow(e); }   // a cold start with `open -a <app> <document>`
  }

  window.DSShell = { menuOwnsKey, menu, log, quitFlow, renameCurrent, renameProject,
    recomputeVelocities, applyVel, about, feedback, nativePick, takeOpenPaths, menuState, sync, boot, hooks, handlers,
    listening: () => booted };

  /* The Feedback button (top right, index.html). Bound here, not in app.js: it belongs to the shell
     like About. #164 */
  const fbBind = () => {
    const b = $q("#feedbackBtn");
    if (b && !b.dataset.fb) { b.dataset.fb = "1"; b.onclick = () => { feedback(); }; }
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", fbBind); else fbBind();
})();
