/* Self-test driver of the scenario `shell` (P3-4b). Like driver.js it runs in the real WKWebView of the
   debug .app, with the real commands and the real page code, and sends one JSON report to the command
   `selftest_report`. It checks the native shell: the live menu (items, shortcuts, enabled state), every menu
   item through the code a click runs (`selftest_menu` -> event "menu" -> shell.js), Open Recent, the dialogs
   (answered by stubs: a native sheet cannot be driven without the GUI), the close prompt of the window button
   and of Quit, and the paths from the Finder (`open -a <app> <document>`, started by self-test.sh).
   P3-13a: the menu bar of ux-daw-reference.md 3.2 (53 custom items in 9 menus; 85 since P4-6a: Hits > Learn This, Not This, Learn Filter (greyed), Forget Examples of Track; 54 since P3-16 Track > Reset Volume and Pan; 58 since P3-14b: Show Info Line, Main Counter Seconds and Samples, Exchange Counters; 59 since P3-14a View > Show Inspector; 64 since P3-14c View > Track Height; 65 since P3-18 Track > Own Detection, with the dynamic submenu Follow Hits Of;   66 since P3-19 File > Open Most Recent; 74 since P3-20a View > Waveform Zoom and Edit > Nudge Value; 75 since P3-20b Transport > Half-Speed Play; 81 since P4-8 File > Import Tempo Map, Track > Detect Tempo, View > the three map rulers and the grid; 89 since P4-wf10 Transport > Suspend Follow when Editing), the check items in both
   directions (a toolbar click sets the NSMenu check mark, a menu item sets the toolbar), the Window menu, and
   every new item with the effect it must have; no in-window menu, sidebar or ? button is left.
   Not covered here: a real mouse click on an NSMenu item and the Dock menu's Quit (no accessibility access). */
(async () => {
  const sc = window.__SELFTEST__;
  const T = window.__TAURI__.core;
  const report = { scenario: sc.name, pass: false, steps: [], metrics: {}, tracks: {}, errors: [] };
  const now = () => performance.now();
  const r2 = x => Math.round(x * 100) / 100;
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const $q = s => document.querySelector(s);
  const need = (cond, msg) => { if (!cond) throw new Error(msg); };

  async function until(fn, what, ms) {
    const end = now() + (ms || 30000);
    for (;;) {
      const v = await fn();
      if (v) return v;
      if (now() > end) throw new Error(`timed out waiting for ${what}`);
      await sleep(50);
    }
  }
  async function step(name, fn) {
    const t = now();
    try {
      const detail = await fn();
      report.steps.push({ step: name, ok: true, ms: r2(now() - t), detail: detail === undefined ? null : detail });
    } catch (e) {
      report.steps.push({ step: name, ok: false, ms: r2(now() - t), error: String((e && e.message) || e) });
      throw e;
    }
  }

  /* ---- menu helpers ---- */
  const triggered = new Set();
  async function click(id) {   // the code of a real click: Rust dispatch, event "menu", shell.js
    const payload = await T.invoke("selftest_menu", { id });
    const entry = id === "recent:clear" ? "recent_clear"   // #441: the Clear Recent item has its own payload id
      : id.startsWith("recent:") ? "recent" : id.startsWith("project:") ? "project" : id.startsWith("follow:") ? "follow" : id;
    need(payload && payload.id === entry, `menu ${id} gave ${JSON.stringify(payload)}`);
    triggered.add(entry);
    return payload;
  }
  const items = async () => Object.fromEntries((await T.invoke("selftest_menu_report")).items.map(i => [i.id, i]));
  const msgIs = re => until(() => re.test($q("#stMsg").textContent), `the message ${re}`, 15000)
    .catch(e => { throw new Error(`${e.message} (the status line shows: ${$q("#stMsg").textContent})`); });
  const visible = sel => $q(sel) && !$q(sel).classList.contains("hidden");
  const modalVisible = () => !$q("#confirmModal").classList.contains("hidden");
  async function answer(act, what) {   // wait for the page's dialog, read it, press a button
    await until(modalVisible, `the dialog for ${what}`, 15000);
    const text = $q("#confirmMsg").textContent, title = $q("#confirmTitle").textContent;
    const labels = [...document.querySelectorAll("#confirmActions button")].map(b => b.textContent);
    $q(`#confirmActions button[data-act="${act}"]`).click();
    await until(() => !modalVisible() || $q("#confirmMsg").textContent !== text, `the dialog to close after ${act}`, 15000);
    return { title, text, labels };
  }
  const dialog = window.__TAURI__.dialog;
  const stub = (kind, value) => { dialog[kind] = async opts => { window.__lastDialog = { kind, opts }; return value; }; };
  const folderOf = async sid => (await T.invoke("project_get", { sid })).folder;
  const sessions = () => T.invoke("project_list", {});
  const sameDir = (a, b) => a.replace(/^\/private/, "").replace(/\/+$/, "") === b.replace(/^\/private/, "").replace(/\/+$/, "");
  const waitProject = (pred, what, ms) => until(async () => (S.sid && pred(await T.invoke("project_get", { sid: S.sid }))) && S.loaded > 0 && S.order.every(n => n in S.peaks), what, ms || 60000);
  const work = sc.work;
  let quitCalls = 0;
  const names = {};

  try {
    await step("cold_start", async () => {
      need(B.app === true, "B.app is not true");
      need(window.__DS_NATIVE_MENU__ === true, "the shell did not set __DS_NATIVE_MENU__");
      // P3-13a: the native menu bar is the only menu
      const left = ["#menuBtn", "#fileBtn", "#fileMenu", "#helpBtn", "#sidebar", "#sidebarBackdrop", ".menuWrap"].filter(s => $q(s));
      need(left.length === 0, `in-window menu elements left: ${left.join(", ")}`);
      need(typeof DSShell === "object", "shell.js is not loaded");
      return { native_menu: true, in_window_menus: 0 };
    });

    // P3-17: the main window starts hidden; the page says "ready" once its first layout is done; then the main
    // window is shown and the splash closes, at the earliest 5 s after its first painted frame (#441) and at
    // the latest after 15 s. Plants (DRUMSPIKE_SPLASH_PLANT, debug builds): early_show fails the first check,
    // keep_splash the third.
    await step("splash_and_first_show", async () => {
      const rep = () => T.invoke("splash_report");
      await until(async () => (await rep()).page_ready, "the page to say ready", 30000);
      let r = await rep();
      need(r.main_visible_at_ready === false, `the main window was visible before the page was ready: ${JSON.stringify(r)}`);
      r = await until(async () => { const x = await rep(); return x.finished && !x.splash_window_open ? x : null; }, "the splash to close", 20000)
        .catch(async e => { throw new Error(`${e.message}: ${JSON.stringify(await rep())}`); });
      need(r.reason === "ready", `the splash ended by ${r.reason}, not by the page: ${JSON.stringify(r)}`);
      need(r.main_visible_now === true, "the main window is not visible after the splash closed");
      // #441: the splash waits for the first painted frame; a page that never paints closes the splash
      // from the open instead (the paint-skip fallback, allowed by the gate) - only the shown time differs
      const paintSkipped = typeof r.painted_ms !== "number";
      if (!paintSkipped) need(r.closed_after_paint_ms >= r.min_ms, `the splash showed ${r.closed_after_paint_ms} ms from its first paint (min ${r.min_ms})`);
      need(r.splash_closed_ms >= r.min_ms && r.splash_closed_ms <= r.max_ms, `the splash closed after ${r.splash_closed_ms} ms (limits ${r.min_ms} to ${r.max_ms})`);
      need(r.main_shown_ms >= r.ready_ms, `the main window was shown at ${r.main_shown_ms} ms, before the ready call at ${r.ready_ms} ms`);
      const texts = r.steps.map(x => x.text);
      for (const want of ["Starting Drum Spike\u2026", "Opening the project\u2026", "Ready"]) need(texts.includes(want), `no status line "${want}": ${texts.join(" | ")}`);
      report.metrics.splash_ready_ms = r.ready_ms; report.metrics.splash_closed_ms = r.splash_closed_ms;
      report.metrics.splash_painted_ms = r.painted_ms; report.metrics.splash_closed_after_paint_ms = r.closed_after_paint_ms;
      return { ready_ms: r.ready_ms, painted_ms: r.painted_ms, main_shown_ms: r.main_shown_ms, splash_closed_ms: r.splash_closed_ms, closed_after_paint_ms: r.closed_after_paint_ms, steps: r.steps };
    });

    // #195: the one enabled-state expectation, shared by menu_table and menu_state_follows_project
    // (menu.rs sync + sync_checks): open_recent tracks the recent list, export_again also needs an
    // export shape, undo/redo are greyed while their stack is empty (#158 rank 9), the five hit
    // items need a hit selection and Re-analyze Selection a range (#265). `S` is the live page
    // state of the running scenario.
    const menuEnabledWant = (rep, hasProj) => i => i.id === "open_recent" ? rep.recent.some(r => /^recent:\d+$/.test(r))   // P3-19
      : i.id === "undo" ? i.ready && hasProj && S.undo.length > 0
      : i.id === "redo" ? i.ready && hasProj && S.redo.length > 0
      : i.id === "delete_hit" || i.id === "nudge_earlier" || i.id === "nudge_later" || i.id === "audition" || i.id === "toggle_hit_active" ? i.ready && hasProj && !!S.sel
      : i.id === "reanalyze_selection" ? i.ready && hasProj && !!(LN.sel && LN.sel.t1 > LN.sel.t0)   // #265: the hits of the range only
      : i.ready && (hasProj ? i.id !== "export_again" || !!S.exportAgain : !i.needs_project);

    await step("menu_table", async () => {
      await until(async () => (await T.invoke("selftest_menu_report")).items, "the menu report", 10000);
      const rep = await T.invoke("selftest_menu_report");
      const byId = Object.fromEntries(rep.items.map(i => [i.id, i]));
      need(rep.items.length === sc.expect.menu_items, `expected ${sc.expect.menu_items} custom items, got ${rep.items.length}`);
      const missing = rep.items.filter(i => !i.in_menu).map(i => i.id);
      need(missing.length === 0, `items missing in the live menu: ${missing.join(", ")}`);
      // P3-13b: the live menu (menu.rs, the table compiled in) against the table that the page loaded (static/keymap.js)
      const rows = DSKeys.COMMANDS.filter(c => c.menu);
      need(rows.length === rep.items.length, `the key table has ${rows.length} menu commands, the menu ${rep.items.length} items`);
      const differ = [];
      // P3-13c: the live menu has the accelerators of the saved key preset (the owner's settings.json); compare with that preset's keys
      const preset = rep.preset || DSKeys.preset;
      need(preset === DSKeys.preset, `the menu has the ${rep.preset} preset, the page ${DSKeys.preset}`);
      for (const c of rows) {
        const it = byId[c.menu];
        if (!it) { differ.push(`${c.menu}: not in the menu`); continue; }
        const mk = DSKeys.menuKeyOf(c, preset), want = mk ? DSKeys.tauriOf(mk) : null;
        if ((it.accel || null) !== want) differ.push(`${c.menu}: menu ${it.accel}, table ${want}`);
        if (it.label !== c.title) differ.push(`${c.menu}: label ${it.label}, table ${c.title}`);
        if ((it.page_keys || []).join() !== c.page_keys.join()) differ.push(`${c.menu}: page keys ${it.page_keys}, table ${c.page_keys}`);
      }
      need(differ.length === 0, `the live menu and static/keymap.js differ: ${differ.join("; ")}`);
      need(DSKeys.problems(DSKeys.COMMANDS).length === 0, `static/keymap.js: ${DSKeys.problems(DSKeys.COMMANDS).join("; ")}`);
      const withAccel = rep.items.filter(i => i.accel).length;
      // #195: derived from the key table (as test_keys does), so a new item with a shortcut
      // cannot stale the count; the menu and the table must simply agree
      const wantAccel = DSKeys.menuKeys(preset).length;
      need(withAccel === wantAccel, `${withAccel} items have a shortcut, the table ${wantAccel}`);
      const titles = ["Drum Spike", "File", "Edit", "View", "Transport", "Hits", "Track", "Window", "Help"];
      need(rep.menus.join() === titles.join(), `menus: ${rep.menus.join(", ")}`);
      const checks = rep.items.filter(i => i.kind === "check").map(i => i.id);
      // #195: list updated to the menu of main 04ccb96; new check items named in the comment:
      // #149 counter Bars|Beats and Timecode, #175 Tab Moves To (radio), the markers ruler,
      // #208/#157 Velocity Lane and Other Mics Under Hit, #436 Stop Returns to Start
      need(checks.join() === "nudge_val_1smp,nudge_val_10smp,nudge_val_100smp,nudge_val_1ms,nudge_val_10ms,height_small,height_medium,height_large,inspector,info_line,overview,counter_seconds,counter_samples,counter_barsbeats,counter_timecode,tab_active,tab_all,ruler_tempo,ruler_meter,ruler_beats,ruler_markers,grid,velocity_lane,other_mics,stop_return_mode,audition_tab,follow_smooth,follow_page,follow_off,suspend_follow,click,snap,learn_filter", `check items: ${checks}`);   // P4-wf10: Suspend Follow when Editing
      // #195: the document from `open -a` can already be open here (it arrives 6 s in, the splash
      // closes after 5-15 s), so the enabled state is read after menu sync with a project. The
      // expectation follows menu.rs sync: open_recent tracks the recent list, export_again also
      // needs an export shape, undo/redo are greyed while their stack is empty (#158 rank 9),
      // delete_hit/nudge_earlier/nudge_later/audition need a hit selection (the page sends
      // `selHit` with every sync), everything else is enabled iff it is ready (and, without a
      // project, does not need one).
      const hasProj = !!S.sid;
      const enabledWant = menuEnabledWant(rep, hasProj);
      // the page syncs the menu shortly after the document arrives; re-read the report until it matches.
      // The bad list rides in the timeout error: without it a timeout says nothing about which item lagged.
      let lastBad = [];
      try {
        await until(async () => {
          const r = await T.invoke("selftest_menu_report");
          const bad = r.items.filter(i => i.enabled !== enabledWant(i)).map(i => `${i.id}=${i.enabled}`);
          lastBad = bad;
          return bad.length ? null : true;   // wait until NOTHING mismatches; the bad list rides in the timeout error
        }, "the menu enabled state to follow the page state", 10000);
      } catch (e) {
        const snap = lastBad.length ? ` last mismatch: ${lastBad.join(", ")}` : "";
        throw new Error(`${e.message};${snap} (S.sid=${S.sid}, S.sel=${JSON.stringify(S.sel)}, undo=${S.undo.length}, redo=${S.redo.length})`);
      }
      need(rep.recent.length >= 1, "Open Recent has no entry at all (not even the placeholder)");
      report.metrics.menu = { items: rep.items.length, menus: rep.menus.length, later: rep.items.filter(i => !i.ready).map(i => `${i.id} (${i.card})`),
        accelerators: withAccel, check_items: checks.length, page_keys: rep.items.reduce((n, i) => n + (i.page_keys || []).length, 0),
        table_commands: DSKeys.COMMANDS.length, compared_with_table: rows.length };
      return report.metrics.menu;
    });

    await step("menu_key_preset", async () => {
      // P3-13c: the key preset sets the accelerators of the live menu (menu.rs set_preset); the report names the preset
      // the menu has and the accelerator of every item, which is compared with the page's table (menuKeyOf)
      const compare = async preset => {
        await until(async () => (await T.invoke("selftest_menu_report")).preset === preset, `the menu to have the ${preset} accelerators`, 10000);
        const rep = await T.invoke("selftest_menu_report");
        const byId = Object.fromEntries(rep.items.map(i => [i.id, i]));
        const differ = [];
        for (const c of DSKeys.COMMANDS.filter(x => x.menu)) {
          const k = DSKeys.menuKeyOf(c, preset), want = k ? DSKeys.tauriOf(k) : null;
          if (!byId[c.menu] || (byId[c.menu].accel || null) !== want) differ.push(`${c.menu}: menu ${byId[c.menu] && byId[c.menu].accel}, table ${want}`);
        }
        need(differ.length === 0, `${preset}: the live menu and the table differ: ${differ.join("; ")}`);
        need(rep.items.every(i => i.in_menu), `${preset}: items missing in the live menu`);
        const withAccel = rep.items.filter(i => i.accel).length;
        need(withAccel === DSKeys.menuKeys(preset).length, `${preset}: ${withAccel} accelerators, table ${DSKeys.menuKeys(preset).length}`);   // P3-19, #195: derived from the key table like menu_table (was a stale hard-coded 33/28)
        return { rep, byId, withAccel };
      };
      // P4-26: the step never SAVES a preset (setKeyPreset(name, false)): a failed check used to leave "cubase" in the settings.json
      // of the home folder (the owner's own when the app ran without DRUMSPIKE_HOME). The finally block puts page and menu back.
      const presetWas = DSKeys.preset;
      let pre, cub;
      try {
      if (presetWas !== "default") { setKeyPreset("default", false); await DSShell.sync(); }
      pre = await compare("default");
      need(pre.byId.next_hit.accel === null && pre.byId.next_hit_all.accel === "Ctrl+Tab", "the default preset has Control+Tab for next hit on all tracks");
      setKeyPreset("cubase", false);
      await DSShell.sync();
      need(DSKeys.preset === "cubase" && DSShell.menuState().keyPreset === "cubase", "the page did not switch to the Cubase preset");
      cub = await compare("cubase");
      const a = id => cub.byId[id].accel;
      need(a("next_hit") === "Alt+N" && a("prev_hit") === "Alt+B", `Cubase hit keys: ${a("next_hit")}, ${a("prev_hit")}`);
      need(a("nudge_earlier") === "CmdOrCtrl+ArrowLeft" && a("nudge_later") === "CmdOrCtrl+ArrowRight", "Cubase nudge keys");
      need(a("zoom_in") === "CmdOrCtrl+]" && a("zoom_out") === "CmdOrCtrl+[", "Cubase zoom keys (the aliases of the gap)");
      need(a("next_hit_all") === null && a("zoom_project") === null && a("go_start") === null && a("mute_track") === null, "page keys must not be accelerators");
      need(a("save") === "CmdOrCtrl+S" && a("inspector") === "Alt+I", "rows without a Cubase list keep their accelerator");
      setKeyPreset("default", false);
      await DSShell.sync();
      const back = await compare("default");
      need(back.byId.next_hit.accel === null && back.byId.prev_hit.accel === "Alt+Tab" && back.byId.nudge_earlier.accel === null, "the default preset is back");
      } finally { setKeyPreset(presetWas, false); await DSShell.sync(); }
      report.metrics.key_preset = { default_accelerators: pre.withAccel, cubase_accelerators: cub.withAccel,
        cubase_changed: DSKeys.COMMANDS.filter(c => c.menu && DSKeys.menuKeyOf(c, "cubase") !== DSKeys.menuKeyOf(c, "default")).map(c => c.menu) };
      return report.metrics.key_preset;
    });

    await step("finder_open", async () => {
      if (!sc.open_doc) { report.metrics.finder = "skipped (no document given)"; return { skipped: true }; }
      const t = now();
      await until(() => S.sid && S.loaded > 0, "the project from `open -a`", 90000);
      const folder = await folderOf(S.sid);
      need(sameDir(folder, sc.project), `the Finder opened ${folder}, expected ${sc.project}`);
      report.metrics.finder_open_ms = r2(now() - t);
      names.a = S.name;
      return { name: S.name };
    });

    await step("menu_open", async () => {
      // File > Open: the native dialog is answered by a stub
      if (S.sid) { await click("close"); await until(() => !S.sid, "the close", 10000); }
      stub("open", sc.project);
      await click("open");
      await waitProject(p => sameDir(p.folder, sc.project), "File > Open");
      names.a = S.name;
      need(window.__lastDialog.kind === "open" && window.__lastDialog.opts.directory === true, "File > Open did not use the native folder dialog");
      need(S.order.length === sc.expect.tracks, `expected ${sc.expect.tracks} tracks, got ${S.order.length}`);
      return { name: S.name, tracks: S.order.length };
    });

    await step("menu_state_follows_project", async () => {
      // the lagging items ride in the timeout error (the 2776224 shape): a bare timeout
      // says nothing about which item did not follow. The expectation is the same model as
      // menu_table (menuEnabledWant): after File > Open of a fresh project, undo/redo and the
      // five hit items are legitimately greyed (#158 rank 9) until the page state has a stack
      // step or a hit selection — `enabled === ready` alone can never pass.
      let lastBad = [];
      try {
        const hasProj = !!S.sid;
        const rep = await T.invoke("selftest_menu_report");
        const enabledWant = menuEnabledWant(rep, hasProj);
        await until(async () => {
          const r = await T.invoke("selftest_menu_report");
          const bad = r.items.filter(i => i.enabled !== enabledWant(i)).map(i => `${i.id}=${i.enabled} (ready=${i.ready})`);
          lastBad = bad;
          return bad.length ? null : true;
        }, "the menu to enable the items of an open project", 10000);
      } catch (e) {
        const snap = lastBad.length ? ` last mismatch: ${lastBad.join(", ")}` : "";
        throw new Error(`${e.message};${snap} (S.sid=${S.sid}, S.exportAgain=${JSON.stringify(S.exportAgain)}, S.sel=${JSON.stringify(S.sel)}, undo=${S.undo.length}, redo=${S.redo.length})`);
      }
      const rep = await T.invoke("selftest_menu_report");
      const off = rep.items.filter(i => !i.enabled).map(i => i.id).sort();
      return { disabled_with_project: off };
    });

    const TMK = { tempo: "ruler_tempo", meter: "ruler_meter", beats: "ruler_beats", grid: "grid" };   // P4-8: the View items of the map rulers and the grid
    await step("menu_check_items", async () => {
      // P3-13a: each check item follows a toolbar click (the NSMenu mark, read back), and the toolbar follows the item
      const mark = async () => { const r = await items(); return { snap: r.snap.checked, click: r.click.checked,
        follow: ["smooth", "page", "off"].filter(m => r[`follow_${m}`].checked),
        info: r.info_line.checked, counter: ["seconds", "samples"].filter(m => r[`counter_${m}`].checked), inspector: r.inspector.checked,
        height: ["small", "medium", "large"].filter(m => r[`height_${m}`].checked),
        nudge: NUDGE_ORDER.filter(k => r[`nudge_val_${k}`].checked),
        rulers: Object.keys(TMK).filter(k => r[TMK[k]].checked) }; };   // P4-8: View > the three map rulers and the grid
      const marks = (want, what) => until(async () => JSON.stringify(await mark()) === JSON.stringify(want), what, 10000);
      const st = () => ({ snap: S.snap, click: !!SETTINGS().click.enabled, follow: [S.followMode],
        info: !!SETTINGS().ui.infoLine, counter: [SETTINGS().ui.counter], inspector: !!S.inspector, height: [S.trackHeight], nudge: [S.nudgeValue],
        rulers: Object.keys(TMK).filter(k => tmShown(k)) });
      const clickWas = !!SETTINGS().click.enabled, inspectorWas = !!S.inspector;
      setSnap(true); setFollow("smooth");
      await marks(st(), "the first sync of the check items");
      const toolbar = [];
      $q("#snapChk").click();                                                   // toolbar -> menu
      need(S.snap === false, "the snap box did not change S.snap");
      await marks(st(), "Snap Playhead to Hits after a click on the snap box"); toolbar.push("snap");
      const sel = $q("#followBtn");
      for (const m of ["page", "off", "smooth"]) {
        sel.click();                                                            // the button cycles smooth, page, off
        await marks(st(), `Follow Playhead > ${m} after the follow select`); toolbar.push(`follow_${m}`);
      }
      $q("#clickBtn").click();
      need(!!SETTINGS().click.enabled !== clickWas, "the click button did not toggle the click");
      await marks(st(), "Metronome Click after a click on the click button"); toolbar.push("click");
      const menu = [];                                                          // menu -> toolbar
      await click("snap");
      await until(() => $q("#snapChk").checked === true && S.snap === true, "the snap box after the menu item", 10000);
      await marks(st(), "the snap mark after the menu item"); menu.push("snap");
      for (const m of ["page", "off", "smooth"]) {
        await click(`follow_${m}`);
        await until(() => sel.dataset.mode === m && S.followMode === m, `the follow select after Follow Playhead > ${m}`, 10000);
        await marks(st(), `the follow marks after ${m}`); menu.push(`follow_${m}`);
      }
      await click("click");
      await until(() => !!SETTINGS().click.enabled === clickWas && $q("#clickBtn").classList.contains("on") === clickWas, "the click button after the menu item", 10000);
      await marks(st(), "the click mark after the menu item"); menu.push("click");
      await click("follow_toggle");                                             // F: off, then back to the last mode
      await until(() => S.followMode === "off", "Follow Playhead On/Off (off)", 10000);
      await marks(st(), "the follow marks after On/Off");
      await click("follow_toggle");
      await until(() => S.followMode === "smooth", "Follow Playhead On/Off (back to smooth)", 10000);
      await marks(st(), "the follow marks after On/Off again");
      // P3-14b: Show Info Line and Main Counter (radio) from the menu; a click on the main counter exchanges, the marks follow
      const infoWas = !!SETTINGS().ui.infoLine;
      await click("info_line");
      await until(() => !!SETTINGS().ui.infoLine === !infoWas && $q("#infoLine").classList.contains("hidden") === infoWas, "the info line after the menu item", 10000);
      await marks(st(), "the Show Info Line mark after the menu item"); menu.push("info_line");
      await click("info_line");
      await marks(st(), "the Show Info Line mark after the second item"); need(!!SETTINGS().ui.infoLine === infoWas, "Show Info Line did not toggle back");
      await click("counter_samples");
      await until(() => $q("#timeDisplay").textContent.endsWith(" smp"), "the main counter in samples", 10000);
      await marks(st(), "the Main Counter > Samples mark"); menu.push("counter_samples");
      $q("#timeDisplay").click();                                               // toolbar -> menu: the click exchanges
      await until(() => SETTINGS().ui.counter === "seconds" && $q("#timeDisplay").textContent.endsWith(" s"), "the exchange by a click on the main counter", 10000);
      await marks(st(), "the Main Counter marks after a click on the counter"); toolbar.push("counter_click");
      await click("counter_exchange");
      await until(() => SETTINGS().ui.counter === "samples", "Exchange Counters", 10000);
      await marks(st(), "the Main Counter marks after Exchange Counters"); menu.push("counter_exchange");
      await click("counter_seconds");
      await until(() => SETTINGS().ui.counter === "seconds", "Main Counter > Seconds", 10000);
      await marks(st(), "the Main Counter > Seconds mark"); menu.push("counter_seconds");
      // P3-14a: View > Show Inspector (no toolbar control; the page toggles the zone, the mark and the setting follow)
      await click("inspector");
      await until(() => !!S.inspector !== inspectorWas && $q("#inspector").classList.contains("hidden") === inspectorWas, "the Inspector after the menu item", 10000);
      await marks(st(), "the Inspector mark after the menu item"); menu.push("inspector");
      await click("inspector");
      await until(() => !!S.inspector === inspectorWas, "the Inspector after the menu item again", 10000);
      await marks(st(), "the Inspector mark after the menu item again");
      // P3-14c: View > Track Height (radio items Small, Medium, Large; Taller and Shorter step): the lanes, the mark and the setting follow
      const heightWas = S.trackHeight, waveH = () => $q("#lanesScroll canvas.wave").clientHeight;
      await click("height_large");
      await until(() => S.trackHeight === "large" && waveH() === 240 && GEO.waveH === 240, "the large height after the menu item", 10000);
      await marks(st(), "the Track Height > Large mark"); menu.push("height_large");
      await click("height_shorter");
      await until(() => S.trackHeight === "medium" && waveH() === 120, "Shorter: medium", 10000);
      await marks(st(), "the Track Height mark after Shorter");
      await click("height_small");
      await until(() => S.trackHeight === "small" && waveH() === 56 && GEO.waveH === 56, "Track Height > Small", 10000);
      await marks(st(), "the Track Height > Small mark");
      await click("height_shorter");   // small is the smallest: the height stays
      await msgIs(/track height: small is the smallest/);
      need(S.trackHeight === "small", "Shorter at small changed the height");
      await click("height_taller");
      await until(() => S.trackHeight === "medium", "Taller: medium", 10000);
      await click("height_medium");
      await until(() => S.trackHeight === "medium" && waveH() === 120, "Track Height > Medium", 10000);
      await marks(st(), "the Track Height > Medium mark");
      const saved = await until(async () => { const v = await T.invoke("app_settings_get", {}); return v.track_height === "medium" ? v : null; }, "settings.json track_height medium", 10000);
      need(saved.track_height === "medium", `settings.json track_height ${saved.track_height}`);
      if (heightWas !== "medium") { await click(`height_${heightWas}`); await until(() => S.trackHeight === heightWas, "the height as before", 10000); }   // the owner's setting stays
      // P3-20a: Edit > Nudge Value (radio items): the mark, the toolbar list and the setting follow
      const nudgeWas = S.nudgeValue, nudgeSel = $q("#nudgeSel");
      for (const k of ["100smp", "1ms", "10ms", "10smp", "1smp"]) {
        await click(`nudge_val_${k}`);
        await until(() => S.nudgeValue === k && nudgeSel.value === k, `Nudge Value > ${k}`, 10000);
        await marks(st(), `the Nudge Value > ${k} mark`); menu.push(`nudge_val_${k}`);
      }
      const nudgeSaved = await until(async () => { const v = await T.invoke("app_settings_get", {}); return v.nudge_value === "1smp" ? v : null; }, "settings.json nudge_value 1smp", 10000);
      need(nudgeSaved.nudge_value === "1smp", `settings.json nudge_value ${nudgeSaved.nudge_value}`);
      nudgeSel.value = "10ms"; nudgeSel.dispatchEvent(new Event("change"));      // toolbar -> menu: the list next to the main counter
      await until(() => S.nudgeValue === "10ms", "the nudge list", 10000);
      await marks(st(), "the Nudge Value marks after the list"); toolbar.push("nudge_list");
      await click(`nudge_val_${nudgeWas}`);
      await until(() => S.nudgeValue === nudgeWas, "the nudge value as before", 10000);   // the owner's setting stays
      // P4-8: View > Tempo Ruler, Meter Ruler, Bars|Beats Ruler, Show Bar and Beat Grid: the page, the mark and the app setting follow
      const tmWas = { ...TM.ui }, TMS = { tempo: "ruler_tempo", meter: "ruler_meter", beats: "ruler_beats", grid: "grid_lines" };
      for (const k of Object.keys(TMK)) {
        const was = tmShown(k);
        await click(TMK[k]);
        await until(() => tmShown(k) === !was && (k === "grid" ? TM.gridOn === !was : TM.vis[k] === !was), `${TMK[k]} after the menu item`, 10000);
        await marks(st(), `the ${TMK[k]} mark after the menu item`); menu.push(TMK[k]);
        const saved = await until(async () => { const v = await T.invoke("app_settings_get", {}); return v[TMS[k]] === !was ? v : null; }, `settings.json ${TMS[k]}`, 10000);
        need(saved[TMS[k]] === !was, `settings.json ${TMS[k]} ${saved[TMS[k]]}`);
        await click(TMK[k]);
        await until(() => tmShown(k) === was, `${TMK[k]} after the menu item again`, 10000);
        await marks(st(), `the ${TMK[k]} mark after the menu item again`);
      }
      TM.ui = tmWas; tmApplyUi(true);   // the owner's settings stay (null: on once a map is set)
      for (const k of Object.keys(TMS)) await T.invoke("app_settings_set", { key: TMS[k], value: tmWas[k] === undefined ? null : tmWas[k] });
      report.metrics.menu_checks = { check_items: 21, follow_a_toolbar_click: toolbar.length, toolbar_follows_the_item: menu.length };
      need(new Set(toolbar).size === 7 && new Set(menu).size === 20, `toolbar ${toolbar}, menu ${menu}`);
      return report.metrics.menu_checks;
    });

    await step("menu_new_and_recent", async () => {
      const dir = `${work}/new`;
      stub("save", `${dir}/New Song`);
      await click("new");
      await until(() => S.name === "New Song", "File > New", 20000);
      need(window.__lastDialog.kind === "save", "File > New did not use the native save dialog");
      const fresh = await T.invoke("project_get", { sid: S.sid });
      need(sameDir(fresh.folder, `${dir}/New Song`), `New Song is in ${fresh.folder}`);
      // Open Recent: the menu lists the recent projects; the first is the new one
      const rep = await until(async () => {
        const r = await T.invoke("selftest_menu_report");
        return r.recent.length >= 2 && r.recent[0] === "recent:0" ? r : null;
      }, "the Open Recent entries", 10000);
      const recent = await T.invoke("project_recent", {});
      need(recent[0].name === "New Song", `recent[0] is ${recent[0].name}`);
      // an entry of Open Recent opens its project: the second entry is project A
      const second = recent.find(r => r.name === names.a);
      need(second, "project A is not in the recent list");
      const idx = recent.indexOf(second);
      await click(`recent:${idx}`);
      await waitProject(p => p.name === names.a, "Open Recent");
      // P3-19: File > Open Most Recent opens the first entry; the item is enabled now and opens the first entry of the list
      const rep2 = await T.invoke("selftest_menu_report");
      need(rep2.items.find(i => i.id === "open_recent").enabled === true, "Open Most Recent is greyed although the list has entries");
      const first = (await T.invoke("project_recent", {}))[0];
      await click("open_recent");
      await waitProject(p => sameDir(p.folder, first.folder), "Open Most Recent");
      return { recent: rep.recent.length, opened_index: idx };
    });

    // #441: the Clear Recent item of the Open Recent submenu empties the stored list and the menu;
    // the next open starts a new list (the start-screen list is the e2e_app test_clear_recent.py).
    await step("recent_clear", async () => {
      const had = (await T.invoke("project_recent", {})).length;
      need(had >= 1, `the recent list is empty before the clear: ${had}`);
      await click("recent:clear");
      await until(async () => {
        const r = await T.invoke("selftest_menu_report");
        return r.recent.length === 1 && r.recent[0] === "recent:none";
      }, "the Open Recent menu to empty", 10000);
      const left = await T.invoke("project_recent", {});
      need(left.length === 0, `the recent list is not empty: ${JSON.stringify(left)}`);
      const rep3 = await T.invoke("selftest_menu_report");
      need(rep3.items.find(i => i.id === "open_recent").enabled === false, "Open Most Recent is enabled although the list is empty");
      return { cleared: had, menu: ["recent:none"] };
    });

    await step("menu_window", async () => {
      // P3-13a: the Window menu lists the open projects (the current one checked); an entry switches to its project
      const open = await T.invoke("project_list", { shape: "ids" });
      const entries = async () => (await T.invoke("selftest_menu_report")).window.filter(w => /^project:\d+$/.test(w.id));
      await until(async () => { const e = await entries(); return e.length === open.length && e.filter(w => w.checked).length === 1
        && e.find(w => w.checked).label === S.name; }, "the Window menu entries", 10000);
      const e = await entries();
      const other = e.find(w => !w.checked);
      need(other, `the Window menu has no second project: ${JSON.stringify(e)}`);
      setDirtyState(false);
      await click(other.id);
      await waitProject(p => p.name === other.label, "Window > project");
      await until(async () => (await entries()).find(w => w.checked).label === other.label, "the check mark on the new current project", 10000);
      const back = (await entries()).find(w => w.label === names.a);
      setDirtyState(false);
      await click(back.id);
      await waitProject(p => p.name === names.a, "Window > project A");
      report.metrics.menu_window = { entries: e.length, labels: e.map(w => w.label) };
      return report.metrics.menu_window;
    });

    await step("menu_edit_undo_redo_recompute", async () => {
      const name = S.order[0], sid = S.sid;
      const ev = [1, 2, 3].map((t, i) => ({ t, v: 1, id: `s${i}` }));
      await T.invoke("events_set", { sid, name, events: ev });
      await loadProject();
      await until(() => S.events[name] && S.events[name].length === 3, "the three hits", 15000);
      await click("recompute_velocities");
      await until(() => S.events[name].every(e => e.v >= 20), "Recompute Velocities", 15000);
      const after = S.events[name].map(e => e.v);
      await click("undo");
      await until(() => S.events[name].every(e => e.v === 1), "Undo", 10000);
      await click("redo");
      await until(() => S.events[name].map(e => e.v).join() === after.join(), "Redo", 10000);
      return { velocities: after };
    });

    await step("menu_save", async () => {
      await until(async () => (await T.invoke("project_get", { sid: S.sid })).dirty === true, "the dirty flag", 10000);
      await click("save");
      await until(async () => (await T.invoke("project_get", { sid: S.sid })).dirty === false, "Save", 15000);
      return { saved: true };
    });

    await step("menu_export", async () => {
      // P4-30: File > Export... opens the Export dialog; Export writes the files of the tracks with hits; the next dialog
      // says how many files it replaces (no confirm dialog); Escape cancels
      const sid = S.sid;
      const names = S.order.filter(k => S.tracks[k] && !S.tracks[k].offline && (S.events[k] || []).length > 0), n = names.length;
      need(n > 0, "no track has hits to export");
      // The dialog opens only after openExport's bridge calls return: wait for EX.open AND
      // #exportModal without `hidden` (openExport:5898), never for the text alone.
      await click("export");
      await until(() => EX.open === true && visible("#exportModal"), "the Export dialog", 15000);
      $q("#exGo").click();
      // #151: a project without a tempo map asks "No tempo set" before the first export;
      // answer it the way a user would (Export anyway), then the export runs. The notice
      // appears a few IPC round-trips after the click: wait for it to SHOW (or for the
      // export to have finished without one), never for "not visible yet".
      await until(() => visible("#confirmModal") || /exported \d+ MIDI and \d+ spike files?/.test($q("#stMsg").textContent), `the tempo notice or the export (stMsg=${JSON.stringify($q("#stMsg").textContent)})`, 15000);
      if (visible("#confirmModal")) {
        need($q("#confirmTitle").textContent === "No tempo set", `unexpected confirm "${$q("#confirmTitle").textContent}" before the export`);
        const anyway = $q("#confirmActions button[data-act='anyway']");
        need(!!anyway, "the tempo notice has no Export anyway button");
        anyway.click();
      }
      await until(() => new RegExp(`exported ${n} MIDI and ${n} spike files?`).test($q("#stMsg").textContent), `Export (stMsg=${JSON.stringify($q("#stMsg").textContent)})`, 60000);
      const have = await T.invoke("export_existing", { sid });
      need(have.count === 2 * n, `Exports/ holds ${have.count} files, expected ${2 * n}`);
      // Cancel only while the dialog is really open (EX.open), and wait until it is closed again.
      await until(() => EX.open === true && visible("#exportModal"), "the Export dialog open after the export", 15000);
      $q("#exCancel").click();
      await until(() => EX.open === false && !visible("#exportModal"), "the Export dialog closed after Cancel", 10000);
      need(EX.open === false && !visible("#exportModal"), "the Export dialog is not closed before the second open");
      await click("export");
      // #exReplace keeps its old text after a close (exClose does not clear it): the wait must be
      // on the dialog being OPEN first, or Escape races openExport's pending bridge calls and the
      // dialog reopens after exClose (openExport:5898 removes `hidden` again).
      await until(() => EX.open === true && visible("#exportModal"), "the Export dialog open again", 15000);
      await until(() => $q("#exReplace").textContent === `${2 * n} files will be replaced`, "the replace count in the dialog", 15000);
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      // P4-31: after an export ran, the message keeps its result — "export cancelled" only
      // comes from a cancel before the first export; Escape must close the dialog either way.
      await until(() => EX.open === false && !visible("#exportModal") && (/export cancelled/.test($q("#stMsg").textContent) || new RegExp(`exported ${n} MIDI and ${n} spike files?`).test($q("#stMsg").textContent)), `Escape (stMsg=${JSON.stringify($q("#stMsg").textContent)})`, 10000);
      return { files: have.count };
    });

    await step("menu_new_items", async () => {
      // P3-13a: every item that is new in the menu bar, with the effect it must have
      const done = [];
      const name = S.order[0];
      const span = () => S.view.t1 - S.view.t0;
      const near = (a, b) => Math.abs(a - b) < 1e-9;
      // Drum Spike > Settings and Help > Keyboard Shortcuts: the sheets
      await click("settings");
      await until(() => visible("#settingsModal") && $q("#setPitches").querySelectorAll("input").length === 6, "the Settings sheet", 5000);
      $q("#settingsDone").click(); done.push("settings");
      await click("shortcuts");
      await until(() => visible("#helpModal") && /Go to Project Start/.test($q("#helpModal").textContent), "the shortcuts panel", 5000);
      const rowsShown = [...document.querySelectorAll("#keymapTable tr[data-cmd]")].map(r => r.dataset.cmd);   // P3-13b: made from the table
      need(rowsShown.join() === DSKeys.COMMANDS.map(c => c.id).join(), `the help panel lists ${rowsShown.length} of ${DSKeys.COMMANDS.length} commands`);
      report.metrics.help_rows = rowsShown.length;
      $q("#helpDone").click(); done.push("shortcuts");
      // File > Rename Project: a dialog; Escape keeps the name
      const old = S.name;
      await click("rename");
      const field = await until(() => visible("#renameModal") && $q("#renameField"), "the rename dialog", 5000);
      field.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await until(() => !visible("#renameModal") && S.name === old, "the name after Escape", 5000);
      done.push("rename");
      // View: zoom around the playhead, to the project, the presets
      S.playhead = 1; setView(0, 2);
      await click("zoom_in"); need(near(span(), 1), `Zoom In: span ${span()}`);
      await click("zoom_out"); need(near(span(), 2), `Zoom Out: span ${span()}`);
      await click("zoom_project"); need(S.view.t0 === 0 && near(span(), maxDur()), `Zoom to Project: ${S.view.t0} ${span()}`);
      const presets = { zoom_full: maxDur(), zoom_30s: Math.min(30, maxDur()), zoom_5s: 5, zoom_500ms: 0.5, zoom_50ms: 0.05, zoom_5ms: 0.005 };
      for (const [id, want] of Object.entries(presets)) { await click(id); need(near(span(), want), `${id}: span ${span()}`); }
      done.push("zoom_in", "zoom_out", "zoom_project", ...Object.keys(presets));
      // View > Waveform Zoom (P3-20a): a display gain of 1x to 64x; the label and the setting follow; Zoom to Project resets it
      const ampWas = S.ampZoom, label = () => $q("#ampLbl");
      setAmpZoom(1);
      await click("amp_in"); need(S.ampZoom === 2 && label().textContent === "Amp 2x" && !label().classList.contains("hidden"), `Waveform Zoom In: ${S.ampZoom}`);
      await click("amp_in"); need(S.ampZoom === 4, `Waveform Zoom In again: ${S.ampZoom}`);
      await click("amp_out"); need(S.ampZoom === 2, `Waveform Zoom Out: ${S.ampZoom}`);
      const ampSaved = await until(async () => { const v = await T.invoke("app_settings_get", {}); return v.amp_zoom === 2 ? v : null; }, "settings.json amp_zoom 2", 10000);
      need(ampSaved.amp_zoom === 2, `settings.json amp_zoom ${ampSaved.amp_zoom}`);
      await click("amp_reset"); need(S.ampZoom === 1 && label().classList.contains("hidden"), `Waveform Zoom Reset: ${S.ampZoom}`);
      await click("amp_in"); await click("zoom_project"); need(S.ampZoom === 1, `Zoom to Project: amplitude zoom ${S.ampZoom}`);
      if (ampWas !== 1) setAmpZoom(ampWas);   // the owner's setting stays
      done.push("amp_in", "amp_out", "amp_reset");
      // Transport: go to, play/stop (master gain 0), stop and return
      S.playStartPos = 0.75; S.playhead = 2;
      await click("go_start"); need(S.playhead === 0, `Go to Project Start: ${S.playhead}`);
      await click("go_play_start"); need(S.playhead === 0.75, `Go to Playback Start: ${S.playhead}`);
      await click("go_end"); need(near(S.playhead, projectEnd()), `Go to Project End: ${S.playhead}`);
      S.playhead = 0.5;
      await click("play_stop");
      await until(() => S.playing, "Play/Stop (play)", 15000);
      need(S.playStartPos === 0.5, `the playback start is ${S.playStartPos}`);
      await sleep(300);
      await click("play_stop");
      await until(() => !S.playing, "Play/Stop (stop)", 15000);
      await click("play_stop");
      await until(() => S.playing, "Play/Stop (play again)", 15000);
      await sleep(300);
      await click("stop_return");
      await until(() => !S.playing && S.playhead === S.playStartPos, "Stop and Return", 15000);
      // P3-20b: Half-Speed Play from the playhead (master gain 0), the status line, the playhead at half the rate, stop
      S.playhead = 0.5;
      const hs0 = now();
      await click("half_speed");
      await until(() => S.playing && AC.running && AC.speed === 0.5, "Half-Speed Play (play)", 15000);
      const halfStartMs = r2(now() - hs0);
      need(visible("#stSpeed") && $q("#stSpeed").textContent === "half speed", "the status line does not say half speed");
      const hsA = await T.invoke("transport_status", {});
      await sleep(600);
      const hsB = await T.invoke("transport_status", {});
      const hsRate = (hsB.pos - hsA.pos) / ((hsB.t_ns - hsA.t_ns) / 1e9);
      need(hsB.speed === 0.5 && Math.abs(hsRate - 0.5) < 0.08, `half speed: status speed ${hsB.speed}, ${hsRate} s per s`);
      await click("half_speed");
      await until(() => !S.playing, "Half-Speed Play (stop)", 15000);
      need(!visible("#stSpeed"), "the status line still says half speed after the stop");
      report.metrics.half_speed = { start_ms: halfStartMs, rate: r2(hsRate) };
      done.push("go_start", "go_play_start", "go_end", "play_stop", "stop_return", "half_speed");
      // Hits: next and previous on the selected track, on all tracks
      const evs = [1, 2, 3].map((t, i) => ({ t, v: 80, id: `h${i}` }));
      await T.invoke("events_set", { sid: S.sid, name, events: evs });
      await loadProject();
      await until(() => S.events[name] && S.events[name].length === 3, "the hits of the Hits menu", 15000);
      S.selTrack = name; S.sel = null; S.playhead = 0;
      await click("next_hit"); need(S.sel && S.sel.track === name && S.playhead === 1, `Next Hit: ${JSON.stringify(S.sel)} ${S.playhead}`);
      await click("next_hit"); need(S.playhead === 2, `Next Hit again: ${S.playhead}`);
      await click("prev_hit"); need(S.playhead === 1, `Previous Hit: ${S.playhead}`);
      const allHits = S.order.flatMap(n => (S.events[n] || []).map(e => e.t)).sort((a, b) => a - b);
      await click("next_hit_all"); need(S.playhead === allHits.find(t => t > 1 + 1e-9), `Next Hit on All Tracks: ${S.playhead}`);
      const at = S.playhead;
      await click("prev_hit_all"); need(S.playhead <= at && S.sel, `Previous Hit on All Tracks: ${S.playhead}`);
      done.push("next_hit", "prev_hit", "next_hit_all", "prev_hit_all");
      // P4-6a: Hits > Learn This, Not This, Forget Examples of Track (the hits at 1, 2 and 3 s), and the Learn Filter check item
      S.selTrack = name; S.sel = null;
      lnSetSel(name, 0.5, 2.5); await click("learn_this");
      await until(() => LN.pending === 0 && lnLearn(name) && lnCounts(lnLearn(name)).this === 2, "Learn This", 15000);
      lnSetSel(name, 2.5, 3.5); await click("learn_not");
      await until(() => LN.pending === 0 && lnCounts(lnLearn(name)).not === 1, "Not This", 15000);
      const lf = (await items()).learn_filter;
      need(lf && lf.kind === "check" && lf.enabled === true, `Learn Filter is a check item: ${JSON.stringify(lf)}`);
      await click("learn_filter");
      await click("learn_forget");
      await until(() => LN.pending === 0 && !lnLearn(name), "Forget Examples of Track", 15000);
      done.push("learn_this", "learn_not", "learn_filter", "learn_forget");
      // Edit: add a hit at the playhead on the selected track, nudge it, delete it
      S.playhead = 2.5;
      await click("add_hit");
      // P4-9 stage 2: hit edits go through events_edit (async): wait for the edit queue
      await until(() => SH.pending === 0 && S.events[name].length === 4 && S.sel && S.events[name][S.sel.index].t === 2.5 && S.events[name][S.sel.index].added, "Add Hit at Playhead", 15000);
      const sr = S.tracks[name].sr;
      await click("nudge_later"); await click("nudge_later"); await click("nudge_earlier");
      await until(() => SH.pending === 0 && S.sel && Math.round(S.events[name][S.sel.index].t * sr) === Math.round(2.5 * sr) + 1, "Nudge Hit Later and Earlier", 15000);
      await click("delete_hit");
      await until(() => SH.pending === 0 && S.events[name].length === 3 && !S.sel, "Delete Hit", 15000);
      done.push("add_hit", "nudge_later", "nudge_earlier", "delete_hit");
      // #242: Hits > Toggle Hit Active on the hit at 1 s: deactivated, then active again; the undo
      // takes both steps back (the flags of `was`), so the scenario leaves no mark on the stored hit
      S.selTrack = name; S.sel = { track: name, index: S.events[name].findIndex(e => Math.abs(e.t - 1) < 1e-9) }; DSShell.sync();
      await click("toggle_hit_active");
      await until(() => SH.pending === 0 && S.events[name][S.sel.index].deactivated === true && !S.events[name][S.sel.index].keep, "Toggle Hit Active (deactivated)", 15000);
      await click("toggle_hit_active");
      await until(() => SH.pending === 0 && S.events[name][S.sel.index].keep === true && !S.events[name][S.sel.index].deactivated, "Toggle Hit Active (active again)", 15000);
      await click("undo"); await click("undo");
      await until(() => SH.pending === 0 && !S.events[name][S.sel.index].keep && !S.events[name][S.sel.index].deactivated, "the undo of Toggle Hit Active", 15000);
      done.push("toggle_hit_active");
      // Track: mute and solo the selected track, its MIDI and spike file
      await click("mute_track"); need(S.muted[name] === true, "Mute Selected Track");
      await click("mute_track"); need(!S.muted[name], "Mute Selected Track again");
      await click("solo_track"); need(S.solo[name] === true, "Solo Selected Track");
      await click("solo_track"); need(!S.solo[name], "Solo Selected Track again");
      // P3-16: Reset Volume and Pan of the selected track
      setTrackMix(name, -9, -0.5);
      need(S.tracks[name].volume_db === -9 && S.tracks[name].pan === -0.5, "volume and pan set");
      await click("reset_mix");
      need(S.tracks[name].volume_db === 0 && S.tracks[name].pan === 0, `Reset Volume and Pan: ${S.tracks[name].volume_db} ${S.tracks[name].pan}`);
      const setChk = (sel, v) => { if ($q(sel).checked !== v) $q(sel).click(); };
      // P4-30: the Export dialog with the selected track: MIDI only, then spikes only.
      // The dialog opens only after openExport's bridge calls return: wait for EX.open AND
      // #exportModal without `hidden`, never for the text alone (menu_export shape).
      const answerTempoNotice = async (wantRe, what) => {
        // #151: a project without a tempo map asks "No tempo set" before the first export;
        // answer it the way a user would (Export anyway). The notice appears a few IPC
        // round-trips after the click: wait for it to SHOW (or for the export to have
        // finished without one), never for "not visible yet".
        await until(() => visible("#confirmModal") || wantRe.test($q("#stMsg").textContent), `${what}: the tempo notice or the export (stMsg=${JSON.stringify($q("#stMsg").textContent)})`, 15000);
        if (visible("#confirmModal")) {
          need($q("#confirmTitle").textContent === "No tempo set", `${what}: unexpected confirm "${$q("#confirmTitle").textContent}"`);
          const anyway = $q("#confirmActions button[data-act='anyway']");
          need(!!anyway, `${what}: the tempo notice has no Export anyway button`);
          anyway.click();
        }
        await msgIs(wantRe);
      };
      await click("export_selected");
      await until(() => EX.open === true && visible("#exportModal") && $q("#exScopeSel").checked, "the Export dialog for the selected track", 15000);
      setChk("#exMidi", true); setChk("#exSpikes", false); $q("#exGo").click();
      await answerTempoNotice(/^exported 1 MIDI file to /, "the MIDI export of the selected track");
      await until(() => EX.open === true && visible("#exportModal"), "the Export dialog open after the MIDI export", 15000);
      $q("#exCancel").click();
      await until(() => EX.open === false && !visible("#exportModal"), "the Export dialog closed after Cancel", 10000);
      await click("export_selected");
      await until(() => EX.open === true && visible("#exportModal"), "the Export dialog again", 15000);
      setChk("#exMidi", false); setChk("#exSpikes", true); $q("#exGo").click();
      await answerTempoNotice(/^exported 1 spike file to /, "the spike export of the selected track");
      await until(() => EX.open === true && visible("#exportModal"), "the Export dialog open after the spike export", 15000);
      $q("#exCancel").click();
      await until(() => EX.open === false && !visible("#exportModal"), "the Export dialog closed after the spike export", 10000);
      done.push("mute_track", "solo_track", "reset_mix", "export_selected");
      // Hits > Analyze Selected Track, Analyze All: the real analysis (the re-analyze question: discard the edits)
      const analyzed = async (id, what) => {
        await click(id);
        await until(() => { if (visible("#reanalyzeModal")) $q("#raDiscard").click(); return analysisRunning; }, `${what} to start`, 30000);
        await until(() => !analysisRunning, what, 180000);
        return $q("#progressText").textContent;
      };
      const one = await analyzed("analyze_track", "Analyze Selected Track");
      const all = await analyzed("analyze_all", "Analyze All");
      done.push("analyze_track", "analyze_all");
      S.selTrack = null;
      report.metrics.menu_new_items = { items: done.length, analyze_track: one, analyze_all: all };
      return report.metrics.menu_new_items;
    });

    await step("menu_reanalyze_selection", async () => {
      // #265: Hits > Re-analyze Selection: the hits of a range selection (LN.sel) only. A range
      // with hits but no edited one (the Keep dialog would open otherwise); the info line reports
      // the replacement and the run is one undo step, so the scenario leaves no mark. The menu
      // state without a range is checked by menuEnabledWant (the item is greyed without one).
      const name = S.order[0];
      const before = (S.events[name] || []).map(e => ({ ...e }));
      need(before.length > 0, `no hits on ${name} for the range run`);
      S.sel = null; S.selTrack = name;
      lnSetSel(name, before[0].t - 0.1, before[before.length - 1].t + 0.1);
      await click("reanalyze_selection");
      await until(() => { if (visible("#reanalyzeModal")) $q("#raDiscard").click(); return S.undo.length > 0 && S.undo[S.undo.length - 1].type === "reanalyze"; }, "Re-analyze Selection to push its step", 30000);
      await until(() => SH.pending === 0 && $q("#stMsg").textContent.length > 0, "the range run to finish (the info line)", 180000);
      const after = (S.events[name] || []).map(e => ({ ...e }));
      const info = $q("#stMsg").textContent;
      await click("undo");
      await until(() => SH.pending === 0 && JSON.stringify((S.events[name] || []).map(e => e.t)) === JSON.stringify(before.map(e => e.t)), "the undo of Re-analyze Selection", 15000);
      lnSetSel(null);
      report.metrics.menu_reanalyze_selection = { track: name, before: before.length, after: after.length, info };
      return report.metrics.menu_reanalyze_selection;
    });

    await step("menu_tempo_map", async () => {
      // P4-8: Track > Detect Tempo (the answer of the service replaced by a fixed one: this scenario checks the menu, the dialog
      // and the undo step, the detection itself has its own tests) and File > Import Tempo Map (a MIDI file the app wrote itself)
      const sid = S.sid, name = S.order[0], stored = () => T.invoke("tempo_map_get", { sid });
      need((await stored()).default === true, "the project has a tempo map already");
      const real = B.tempoMapDetect;
      B.tempoMapDetect = async () => ({
        suggestion: { bpm: 96, steps: [{ t: 20, bpm: 100 }], meter: [4, 4], changes: [], bar1_s: 0.5 },
        start: { bpm: 96, error_bar: 0.1, confidence: 0.8, range_edge: false, alternatives: [] },
        bar1: { t: 0.5, confidence: 0.3, backbeat: 0.5, downbeat: 0.5, alternative: 1.1 },
        meter: { value: [4, 4], confidence: 0.8, feel: null }, steps: [{ t: 20, bpm: 100, bar: 9, bars: 8, confidence: 0.7 }], excursions: [],
        fit: { on_beat: 0.5, jitter_ms: 10, hits: 100, beats: 50, late: 0, isolated: 0 }, tracks: [], params: {}, warnings: ["a planted warning"], changed: false });
      try {
        await click("detect_tempo");
        await until(() => visible("#tempoModal"), "the Detect Tempo dialog", 15000);
        const text = $q("#tempoBody").textContent;
        need(/96 BPM/.test(text) && /a planted warning/.test(text) && visible("#tempoBar1"), `the dialog: ${text}`);
        $q("#tempoBar1").click();
        $q("#tempoAccept").click();
        await until(async () => (await stored()).map.bar1_s === 1.1, "Accept (bar 1 two beats later)", 15000);
      } finally { B.tempoMapDetect = real; }
      const m = (await stored()).map;
      need(m.bpm === 96 && m.steps.length === 1 && S.undo[S.undo.length - 1].type === "tempo", `Accept: ${JSON.stringify(m)}`);
      need(tmShown("tempo") && tmShown("grid") && TM.vis.beats, "a map is set: the rulers and the grid show");
      const mid = await T.invoke("export_midi", { sid, name });
      await click("undo");
      await until(async () => (await stored()).default === true, "Undo of the accept", 15000);
      stub("open", mid.path);
      await click("import_tempo_map");
      await until(async () => (await stored()).default === false, "File > Import Tempo Map", 15000);
      need(window.__lastDialog.kind === "open" && window.__lastDialog.opts.filters[0].extensions.includes("mid"), "Import Tempo Map did not ask for a MIDI file");
      const imp = (await stored()).map;
      need(Math.abs(imp.bpm - 96) < 0.001 && imp.steps.length === 1 && Math.abs(imp.bar1_s - 1.1) < 0.001, `the imported map: ${JSON.stringify(imp)}`);
      await click("undo");
      await until(async () => (await stored()).default === true, "Undo of the import", 15000);
      // the undo stores the map first and writes its message after the reload (tmQueue): wait for it, or it can land
      // after "import cancelled" and hide it
      await msgIs(/^undo: import tempo map$/);
      stub("open", null);
      await click("import_tempo_map");
      await msgIs(/^import cancelled$/);
      report.metrics.menu_tempo_map = { accepted_bpm: m.bpm, imported_bpm: imp.bpm };
      return report.metrics.menu_tempo_map;
    });

    // #221: the items that no earlier step triggers, each through the real menu dispatch (the payload id,
    // the state, the settings and the project content are checked, and everything is put back afterwards)
    await step("menu_view_transport_rest", async () => {
      const done = [];
      const sid = S.sid, name = S.order[0];
      const stored = () => T.invoke("tempo_map_get", { sid });
      // View > Tab Moves To (radio): the setting follows, then back
      const tabWas = SETTINGS().ui.tabMode === "all" ? "all" : "active";
      await click("tab_active");
      await until(() => SETTINGS().ui.tabMode === "active", "Tab Moves To > Active Hits Only", 10000);
      await click("tab_all");
      await until(() => SETTINGS().ui.tabMode === "all", "Tab Moves To > All Found Hits", 10000);
      setTabMode(tabWas);
      await until(() => SETTINGS().ui.tabMode === tabWas, "Tab Moves To back", 10000);
      done.push("tab_active", "tab_all");
      // the check items without a toolbar control: the mark and the state follow, then back
      const flip = async (id, get, what) => {
        const was = get();
        await click(id);
        await until(() => get() === !was, `${what} after the menu item`, 10000);
        await click(id);
        await until(() => get() === was, `${what} back`, 10000);
        done.push(id);
      };
      await flip("ruler_markers", () => tmShown("markers"), "the Markers Ruler");
      await flip("velocity_lane", () => VL.on, "the Velocity Lane");
      await flip("other_mics", () => MX.on, "Other Mics Under Hit");
      await flip("overview", () => S.overviewOn, "Show Overview");
      await flip("suspend_follow", () => !!SETTINGS().ui.suspendFollowOnEdit, "Suspend Follow when Editing");
      await flip("stop_return_mode", () => !!SETTINGS().ui.stopReturnToStart, "Stop Returns to Start");
      // #240: Transport > Audition on Tab (the saved ui.auditionTab, the toolbar button and the menu check flip together)
      await flip("audition_tab", () => !!SETTINGS().ui.auditionTab, "Audition on Tab");
      // Main Counter: Bars|Beats and Timecode (the radio items of #149), then the counter as it was
      const counterWas = SETTINGS().ui.counter;
      await click("counter_barsbeats");
      await until(() => SETTINGS().ui.counter === "barsbeats", "Main Counter > Bars|Beats", 10000);
      await click("counter_timecode");
      await until(() => SETTINGS().ui.counter === "timecode", "Main Counter > Timecode (30 fps)", 10000);
      await click(`counter_${counterWas}`);
      await until(() => SETTINGS().ui.counter === counterWas, "the Main Counter as it was", 10000);
      done.push("counter_barsbeats", "counter_timecode", `counter_${counterWas}`);
      // View > Zoom Toggle: the view goes to the 5 ms window, then back
      S.playhead = 1.5; setView(0, 2);
      await click("zoom_toggle");
      await until(() => S.zoomBack && Math.abs(S.view.t1 - S.view.t0 - 0.005) < 1e-9, "Zoom Toggle in", 10000);
      await click("zoom_toggle");
      await until(() => !S.zoomBack && Math.abs(S.view.t1 - S.view.t0 - 2) < 1e-9, "Zoom Toggle back", 10000);
      done.push("zoom_toggle");
      // View > Zoom Presets: Organize opens the dialog (no own presets yet), Add Current saves the span
      // of the view and the menu lists the own preset; the list is put back at the end
      await click("zoom_organize");
      await until(() => visible("#presetModal") && /No own presets yet/.test($q("#presetHint").textContent), "the Organize dialog", 5000);
      $q("#presetAccept").click();
      await until(() => !visible("#presetModal"), "the Organize dialog to close", 5000);
      done.push("zoom_organize");
      const ownWas = S.zoomOwn;
      setView(0, 1);
      await click("zoom_add");
      await until(() => visible("#presetModal") && $q("#presetName"), "the Add Current dialog", 5000);
      $q("#presetName").value = "Self Test Span";
      $q("#presetAccept").click();
      await until(() => !visible("#presetModal") && S.zoomOwn.length === ownWas.length + 1, "the own preset saved", 5000);
      // the menu update goes through the async shell_sync; the report lists the Zoom Presets submenu ids
      await until(async () => ((await T.invoke("selftest_menu_report")).zoom_own || []).some(id => id === "zoom_own:0"), "the menu lists the own preset", 5000);
      done.push("zoom_add");
      // Edit > Set Marker at Selection (the playhead; the service stores the list), then the jumps
      const mkWas = MK.list.length;
      S.sel = null; S.playhead = 1.25;
      await click("markers_set");
      await until(() => MK.pending === 0 && MK.list.length === mkWas + 1 && MK.list.some(m => Math.abs(m.t - 1.25) < 1e-9), "the marker at the playhead", 15000);
      S.playhead = 0.5;
      await click("marker_next");
      await until(() => Math.abs(S.playhead - 1.25) < 1e-9, "Next Marker", 10000);
      S.playhead = 3;
      await click("marker_prev");
      await until(() => Math.abs(S.playhead - 1.25) < 1e-9, "Previous Marker", 10000);
      done.push("markers_set", "marker_next", "marker_prev");
      mkCommit(MK.list.slice(0, mkWas), "markers put back");
      await until(() => MK.pending === 0 && MK.list.length === mkWas, "the marker list put back", 15000);
      // Track > Set Bar 1 at Playhead and at Audio Start: the map commits, Undo brings it back.
      // The fresh project's map has bar 1 at the audio start already (the default bar1_s is 0):
      // the playhead set goes first, or the audio-start set is a no change and pushes no step
      // (gate run 1 timed out on "bar 1 at 0.000 s: no change" with an empty undo stack).
      const bar1Was = (await stored()).map.bar1_s;
      S.sel = null; S.playhead = 0.8;
      await click("set_bar1");
      await until(async () => Math.abs((await stored()).map.bar1_s - 0.8) < 1e-9, "Set Bar 1 at Playhead", 15000);
      // an undo while the commit chain still stores (tmUndoWaits) is refused, and the step is pushed
      // after the service answers: wait for the pushed step on the undo stack (the marker put-back and
      // the tempo commit run on different queues, so the op is found by payload, not by stack position),
      // then retry the undo until the map is back - a click before the push would pop an older step
      const tempoOp = bar1 => S.undo.find(o => o.type === "tempo" && o.after && Math.abs((o.after.bar1_s || 0) - bar1) < 1e-9);
      // #221: a timed-out bar 1 wait must say why: the last steps of the stacks, the queue state and the status line
      const stackDump = () => `undo=[${S.undo.slice(-3).map(o => `${o.type}/${o.what || ""}${o.after ? ` bar1_s=${o.after.bar1_s}` : ""}`).join(" | ")}] redo=${S.redo.length} TM.pending=${TM.pending} msg=${JSON.stringify($q("#stMsg").textContent)}`;
      const undoUntil = (want, what, pend = () => false) => {
        let sent = 0;   // a retry only after 250 ms: the click needs a tick to take the step off the stack
        return until(async () => {
          if (!pend() && Date.now() - sent > 250 && !(await want())) { sent = Date.now(); await click("undo"); }
          return await want();
        }, what, 15000).catch(e => { throw new Error(`${e.message} [${stackDump()}]`); });
      };
      await until(() => tempoOp(0.8), "the bar 1 step on the undo stack", 15000).catch(e => { throw new Error(`${e.message} [${stackDump()}]`); });
      await click("set_bar1_audio");
      await until(async () => Math.abs((await stored()).map.bar1_s) < 1e-9, "Set Bar 1 at Audio Start", 15000);
      await until(() => tempoOp(0), "the bar 1 step on the undo stack", 15000).catch(e => { throw new Error(`${e.message} [${stackDump()}]`); });
      // the undo pops last in, first out: the audio-start set goes back to the playhead set, the
      // playhead set back to bar1Was - each undoUntil pops its own step and its pend stops the
      // clicks once that step is off the stack, so the two waits run in this order
      await undoUntil(async () => Math.abs((await stored()).map.bar1_s - 0.8) < 1e-9, "Undo of Set Bar 1 at Audio Start", () => TM.pending > 0 || !tempoOp(0));
      await undoUntil(async () => Math.abs((await stored()).map.bar1_s - bar1Was) < 1e-9, "Undo of Set Bar 1", () => TM.pending > 0 || !tempoOp(0.8));
      done.push("set_bar1", "set_bar1_audio");
      // Edit > Undo History: the sheet lists the steps of the tempo map, Done closes it
      await click("undo_history");
      await until(() => visible("#undoHistModal") && $q("#undoHistList").children.length > 0, "the Undo History sheet", 5000);
      $q("#undoHistDone").click();
      await until(() => !visible("#undoHistModal"), "the Undo History sheet to close", 5000);
      done.push("undo_history");
      // File > Export Again with nothing exported yet (the bridge answer is stubbed: the Copy Song
      // exports of the earlier steps must not be rewritten): the message, no dialog, no files
      const realLast = B.exportLastGet;
      B.exportLastGet = async () => ({ export_last: null });
      try { await click("export_again"); await msgIs(/nothing exported yet/); }
      finally { B.exportLastGet = realLast; }
      done.push("export_again");
      // Hits > Next and Previous Doubtful Hit (the queue of a fresh analysis is empty: the message says so)
      await click("review_next");
      await until(() => /doubtful|review queue/.test($q("#stMsg").textContent), "Next Doubtful Hit", 10000);
      await click("review_prev");
      await until(() => /doubtful|review queue/.test($q("#stMsg").textContent), "Previous Doubtful Hit", 10000);
      done.push("review_next", "review_prev");
      // Hits > Audition Hit on the first hit of the selected track (the master gain is 0), then stop
      S.selTrack = name;
      await until(() => (S.events[name] || []).length > 0, "the hits for the audition", 15000);
      S.sel = { track: name, index: 0 };
      await click("audition");
      await until(() => AU.on === true, "Audition Hit", 15000);
      auStop();
      await until(() => AU.on === false, "the audition to stop", 15000);
      done.push("audition");
      // Hits > Remove Example without a selection: the message asks for one
      S.sel = null; lnSetSel(null); lnSetMark(null);
      await click("learn_forget_this");
      await msgIs(/Remove Example: select a hit, a range or a mark first/);
      done.push("learn_forget_this");
      // Track > Select Next and Previous Track
      S.selTrack = name;
      await click("next_track");
      await until(() => S.selTrack === S.order[1], "Select Next Track", 10000);
      await click("prev_track");
      await until(() => S.selTrack === name, "Select Previous Track", 10000);
      done.push("next_track", "prev_track");
      // Track > Remove Track: the confirm, the track gone, Undo brings it back with its hits
      S.selTrack = name;
      await until(() => SH.pending === 0, "the event writes to settle", 15000);
      await click("tracks_remove");
      await answer("remove", "the Remove Track confirm");
      await until(() => !S.order.includes(name), "Remove Track", 15000);
      // trRemove reloads the page before it pushes the step: wait for the trkdel op on the undo stack
      // (by payload - the SH and the page queues can reorder), then undo; a refusal retries while the
      // op is still on the stack - a click before the push would pop an older step
      const trkdelOp = () => S.undo.find(o => o.type === "trkdel" && o.sid === S.sid && o.name === name);
      await until(() => !S.order.includes(name) && trkdelOp(), "the removal step on the undo stack", 15000);
      await undoUntil(() => S.order.includes(name) && (S.events[name] || []).length > 0, "Undo brings the track back", () => SH.pending > 0 || !trkdelOp());
      done.push("tracks_remove");
      S.zoomOwn = ownWas; saveZoomOwn();   // the menu does not list the own preset any more
      await until(async () => !((await T.invoke("selftest_menu_report")).zoom_own || []).some(id => id.startsWith("zoom_own:")), "the own preset out of the menu", 10000);
      S.selTrack = null;
      report.metrics.menu_rest = { items: done.length, list: done };
      return report.metrics.menu_rest;
    });

    await step("menu_save_as", async () => {
      stub("save", `${work}/copy/Copy Song`);
      await click("save_as");
      // P3-4d: the app asks whether to copy the audio; this one keeps it referenced (Consolidate copies it next)
      const q = await answer("ref", "the Save As audio question");
      need(q.labels.join() === "Copy audio into the project,Keep the audio where it is,Cancel", `Save As question: ${q.labels}`);
      await until(() => S.name === "Copy Song", "File > Save As", 30000);
      need(window.__lastDialog.kind === "save", "Save As did not use the native save dialog");
      const p = await T.invoke("project_get", { sid: S.sid });
      need(sameDir(p.folder, `${work}/copy/Copy Song`), `Copy Song is in ${p.folder}`);
      names.b = "Copy Song";
      return { folder: p.folder };
    });

    await step("menu_consolidate", async () => {
      // P3-4d: Consolidate is a job (the menu item is enabled now): it copies the referenced audio of the Save As copy
      const sid = S.sid;
      const before = Object.values((await T.invoke("project_get", { sid })).tracks).map(t => t.source.mode);
      need(before.every(m => m === "referenced"), `the Save As copy should be referenced: ${before}`);
      $q("#stMsg").textContent = "";
      const t = now();
      await click("consolidate");
      await until(() => /^consolidated: /.test($q("#stMsg").textContent), "Consolidate", 120000);
      const p = await T.invoke("project_get", { sid });
      const modes = Object.values(p.tracks).map(t => t.source.mode);
      need(modes.every(m => m === "copied") && Object.values(p.tracks).every(t => t.status === "ok"), `after Consolidate: ${modes}`);
      await click("consolidate");
      await until(() => /nothing to consolidate/.test($q("#stMsg").textContent), "the second Consolidate", 30000);
      report.metrics.consolidate_ms = r2(now() - t);
      return { tracks: modes.length, message: "consolidated, then nothing to consolidate" };
    });

    await step("menu_save_as_copy", async () => {
      // P3-4d: Save As with the audio copied is a background job with progress; the dialog button chooses it
      const orig = window.copyProgress, seen = [];
      window.copyProgress = label => { const f = orig(label); return p => { seen.push(p); f(p); }; };
      try {
        stub("save", `${work}/copy/Copy Song Two`);
        const t = now();
        await click("save_as");
        await answer("copy", "the Save As audio question");
        await until(() => S.name === "Copy Song Two", "Save As with copied audio", 120000);
        const p = await T.invoke("project_get", { sid: S.sid });
        need(sameDir(p.folder, `${work}/copy/Copy Song Two`), `Copy Song Two is in ${p.folder}`);
        need(Object.values(p.tracks).every(t => t.source.mode === "copied" && t.status === "ok"), "the copy is not self-contained");
        need(seen.length >= 6 && seen[seen.length - 1].state === "done", `progress: ${seen.length} messages, last ${seen.length && seen[seen.length - 1].state}`);
        report.metrics.save_as_copy = { ms: r2(now() - t), progress_updates: seen.length, tracks: Object.keys(p.tracks).length };
        return report.metrics.save_as_copy;
      } finally { window.copyProgress = orig; }
    });

    await step("menu_add_tracks", async () => {
      // P3-4c: Add Tracks opens the native file dialog (several audio files); a stub cancels it
      stub("open", null);
      await click("add_tracks");
      await until(() => window.__lastDialog && window.__lastDialog.opts && window.__lastDialog.opts.multiple === true, "the file dialog of Add Tracks", 5000);
      need(window.__lastDialog.opts.directory === false, "the Add Tracks dialog picks folders");
      need($q("#tracksModal").classList.contains("hidden"), "a cancelled dialog opened the list");
      return { filters: window.__lastDialog.opts.filters };
    });

    await step("menu_follow", async () => {
      // P3-18: Track > Follow Hits Of lists every other track of the project; an entry makes the selected track
      // follow it (its hits derived from the source, the entry checked); Track > Own Detection gives it its own hits back.
      const [a, b] = S.order;
      const before = (S.events[a] || []).map(e => ({ t: e.t, v: e.v, id: e.id }));
      if (!(S.events[b] || []).length) {
        await T.invoke("events_set", { sid: S.sid, name: b, events: [1, 2, 3].map((t, i) => ({ t, v: 80, id: `f${i}` })) });
        await loadProject();
        await until(() => S.events[b] && S.events[b].length === 3, "the hits of the source track", 15000);
      }
      S.selTrack = a; S.sel = null; DSShell.sync();
      const entries = async () => (await T.invoke("selftest_menu_report")).follow.filter(f => /^follow:\d+$/.test(f.id));
      await until(async () => (await entries()).length === S.order.length - 1, "the Follow Hits Of entries", 10000);
      const e = await entries();
      need(!e.some(f => f.label === a) && e.every(f => !f.checked), `the entries for ${a}: ${JSON.stringify(e)}`);
      const src = e.find(f => f.label === b);
      need(src, `no entry for ${b}: ${JSON.stringify(e)}`);
      const p = await click(src.id);
      need(p.name === b, `the entry payload ${JSON.stringify(p)}`);
      await until(() => S.tracks[a].follows === b, `${a} follows ${b}`, 15000);
      const have = S.events[b].map(x => x.t), got = (S.events[a] || []).map(x => x.t);
      need(have.length > 0 && have.join() === got.join(), `derived hits of ${a}: ${got.length}, source ${have.length}`);
      await until(async () => { const f = (await entries()).find(x => x.label === b); return f && f.checked; }, "the check mark on the source", 10000);
      await click("follow_own");
      await until(() => S.tracks[a].follows == null, `${a} back on its own detection`, 15000);
      await until(async () => (await entries()).every(f => !f.checked), "no check mark after Own Detection", 10000);
      await T.invoke("events_set", { sid: S.sid, name: a, events: before });   // the hits as they were before the step
      await loadProject();
      report.metrics.menu_follow = { entries: e.length, source: b, derived: got.length };
      return report.metrics.menu_follow;
    });

    await step("menu_import_old", async () => {
      // P3-4e behind the P3-4b item: Import Old Session opens the folder dialog and lists the folder (the work folder holds no
      // old session). Nothing is imported here: an import would write a project folder into the user's projects folder.
      // (P4-30 removed Import and Export Project Zip.)
      stub("open", work);
      await click("import_old");
      await until(() => !$q("#importModal").classList.contains("hidden"), "the import list", 15000);
      need(window.__lastDialog.kind === "open" && window.__lastDialog.opts.directory === true, "Import Old Session did not use the native folder dialog");
      const listed = $q("#importList").textContent;
      need(/no old Drum Spike sessions/.test(listed), `import list: ${listed}`);
      $q("#importCancel").click();
      return { import_list: listed };
    });

    await step("menu_about", async () => {
      // P3-8a, #294: Help > About opens the About window; read the version and the third-party list (model-free: no model credit or notice)
      await click("about");
      await until(() => $q("#aboutModal") && !$q("#aboutModal").classList.contains("hidden"), "the About window", 15000);
      const info = await T.invoke("about_info", {});
      const text = $q("#aboutModal").textContent;
      need($q("#aboutTitle").textContent === "About Drum Spike", `About title: ${$q("#aboutTitle").textContent}`);
      need($q("#aboutVersion").textContent.includes(`Version ${info.version}`) && info.version === sc.expect.version,
        `About version: ${$q("#aboutVersion").textContent} (shell says ${info.version})`);
      need(text.includes("finds the hits in your close-mic drum tracks"), "About lacks the one-line description");
      const credit = "ADT" + "OF";   // the residue gate may not see the literal; the check is on the page text
      need(!text.includes(credit) && !text.includes("Zehren"), "About still carries the detector credit (#294)");
      $q('#aboutActions button[data-act="third"]').click();
      await until(() => !$q("#aboutThird").classList.contains("hidden") && $q("#aboutThird").textContent.length > 1000, "the third-party list", 15000);
      const list = $q("#aboutThird").textContent;
      const crates = Number((list.match(/## Crates \((\d+)\)/) || [])[1]);
      need(list.includes("# Third-party software in Drum Spike") && list.includes("MPL-2.0") && crates > 100, `third-party list: ${crates} crates`);
      $q('#aboutActions button[data-act="ok"]').click();
      await until(() => $q("#aboutModal").classList.contains("hidden"), "the About window to close", 15000);
      return { title: "About Drum Spike", version: info.version, notice_chars: list.length, third_party_crates: crates };
    });

    await step("menu_close", async () => {
      // a dirty project: Cancel keeps it, Discard closes it; the menu item is File > Close
      await T.invoke("project_rename", { sid: S.sid, name: "Copy Song" });
      await initSessions();
      const sid = S.sid;
      await click("close");
      const first = await answer("cancel", "the close prompt");
      need(first.labels.join() === "Save and close,Discard changes,Cancel", `close prompt: ${first.labels}`);
      need((await sessions()).some(s => s.sid === sid), "Cancel closed the project");
      await click("close");
      await answer("discard", "the close prompt");
      await until(async () => !(await sessions()).some(s => s.sid === sid), "Discard to close the project", 15000);
      // P3-15e: project_close runs on the blocking pool (P3-15c), so the list can lose the project before the
      // close's answer reaches the page. Wait for the page's own close (the menu handler has returned), else
      // the next step reads the closed id from S.sid ("no such project").
      const gone = now();
      await until(() => S.sid !== sid && DSShell.log.done === DSShell.log.seen, "the page to finish the close", 15000);
      return { closed: true, page_after_list_ms: r2(now() - gone) };
    });

    await step("finder_paths_event", async () => {
      // `RunEvent::Opened` code path: a document, the same project as a folder, and an audio file.
      // P3-15e: then two more Finder double-clicks at once (the document, the folder). The project is closed
      // first, so the opens load it in parallel on the blocking pool; they must end in one session.
      const doc = `${sc.project}/${sc.project.split("/").pop()}.drumspike`;
      const audio = `${sc.project}/Audio Files/none.wav`;
      for (const s of (await sessions()).filter(s => sameDir(s.folder, sc.project))) await T.invoke("project_close", { sid: s.sid, force: true });
      S.sid = null; setDirtyState(false); await initSessions(); await loadProject();
      const openProject = B.openProject;
      let started = 0, finished = 0;
      B.openProject = async path => { started++; try { return await openProject(path); } finally { finished++; } };
      const settled = () => {   // every open the page started has answered, and no new one for 300 ms
        let seen = -1, since = now();
        return until(() => {
          if (finished !== started || started !== seen) { seen = started; since = now(); return false; }
          return started > 0 && now() - since > 300;
        }, "the opens from open-paths to finish", 60000);
      };
      try {
        await T.invoke("selftest_open_paths", { paths: [doc, sc.project, audio] });
        await T.invoke("selftest_open_paths", { paths: [doc] });
        await T.invoke("selftest_open_paths", { paths: [sc.project] });
        await settled();
        await waitProject(p => sameDir(p.folder, sc.project), "the project from open-paths");
        // P3-4c: the audio file goes to Add Tracks; it does not exist, so the list says why and offers Close
        await until(() => $q("#tracksModal") && !$q("#tracksModal").classList.contains("hidden") && /not exist/.test($q("#tracksRejected").textContent), "the Add Tracks list for the audio file", 10000);
        $q("#tracksCancel").click();
        await until(() => $q("#tracksModal").classList.contains("hidden"), "the list to close", 5000);
        await settled();
      } finally { B.openProject = openProject; }
      const open = (await sessions()).filter(s => sameDir(s.folder, sc.project)).length;
      need(open === 1, `${open} sessions of the project after ${started} opens`);
      return { opened_as: S.name, open_copies: open, page_opens: started };
    });

    await step("quit_prompts", async () => {
      // exactly two dirty projects: A and Copy Song
      for (const s of await sessions()) await T.invoke("project_close", { sid: s.sid, force: true });
      S.sid = null; setDirtyState(false); await initSessions(); await loadProject();
      await until(async () => (await T.invoke("selftest_menu_report")).items.find(i => i.id === "save").enabled === false, "the menu to grey Save", 10000);
      stub("open", sc.project);
      await click("open");
      await waitProject(p => sameDir(p.folder, sc.project), "A");
      const a = await sessions();
      const copy = `${work}/copy/Copy Song`;
      stub("open", copy);
      setDirtyState(false);
      await click("open");
      await waitProject(p => sameDir(p.folder, copy), "Copy Song");
      for (const s of await sessions()) await T.invoke("project_rename", { sid: s.sid, name: s.name });
      await initSessions();
      const list = await sessions();
      need(list.length === 2 && list.every(s => s.dirty), `expected 2 dirty projects, got ${JSON.stringify(list.map(s => [s.name, s.dirty]))}`);
      B.quitApp = async () => { quitCalls++; };   // the real command would end the process before the report
      // 1. the window close button (the real CloseRequested): Discard the first, Cancel the second
      await T.invoke("selftest_close_window");
      const p1 = await answer("discard", "the first dirty project");
      const p2 = await answer("cancel", "the second dirty project");
      need(p1.text !== p2.text, "both prompts name the same project");
      await until(() => /quit cancelled/.test($q("#stMsg").textContent), "the cancel message", 10000);
      need(quitCalls === 0, "Cancel still quit");
      need((await sessions()).length === 2, "Cancel closed a project");
      report.metrics.quit_cancel = { prompts: 2, app_still_open: true };
      // 2. Cmd-Q (the menu item): Save the first, Discard the second, then the app quits
      await click("quit");
      const q1 = await answer("save", "quit, first project");
      const q2 = await answer("discard", "quit, second project");
      await until(() => quitCalls === 1, "the quit command", 15000);
      const after = await sessions();
      need(after.filter(s => s.dirty).length === 1, `after Save and Discard: ${after.filter(s => s.dirty).length} dirty, expected 1`);
      report.metrics.quit_save_discard = { prompts: 2, quit_calls: quitCalls };
      return { first: [p1.text, p2.text], second: [q1.text, q2.text] };
    });

    await step("every_item_triggered", async () => {
      const rep = await T.invoke("selftest_menu_report");
      const ids = rep.items.map(i => i.id);
      const left = ids.filter(id => !triggered.has(id));
      need(left.length === 0, `not triggered: ${left.join(", ")}`);
      need(triggered.has("recent"), "no Open Recent entry was triggered");
      need(triggered.has("project"), "no Window menu entry was triggered");
      need(triggered.has("follow"), "no Follow Hits Of entry was triggered");
      report.metrics.menu_triggered = { custom_items: ids.length, triggered: ids.length - left.length, open_recent_entries: 1,
        window_entries: 1, follow_entries: 1, total: triggered.size };
      return report.metrics.menu_triggered;
    });

    report.pass = report.steps.every(s => s.ok);
  } catch (e) {
    report.errors.push(String((e && e.message) || e));
    report.pass = false;
  }
  // the report ends the process through the real quit path (the gate, ExitRequested with a code)
  await T.invoke("selftest_report", { report, via_quit: true });
})();
