/* Self-test driver of the scenario `keys` (#206). Like shell_driver.js it runs in the real WKWebView
   of the debug .app and sends one JSON report to the command `selftest_report`. It loops over the
   whole key command table (static/keymap.js, DSKeys.COMMANDS — no hand-written list) and sends
   every key of every row in every preset (DSKeys.PRESETS: Pro Tools (default) and Cubase) through
   the path that the row says: a `menu_key` through the real native menu (a synthetic keyDown
   NSEvent goes to the app's main menu via `NSMenu performKeyEquivalent:`, command
   `selftest_menu_key` — the same route a physical key press takes), a `page_keys` key as a keydown
   event on the page (the code path of app.js). "The command ran" is checked through a recorder:
   the driver wraps the entry points that a key reaches (app.js `runKey` reads KEY_RUN[id] or
   DSShell.handlers[id] at call time; the menu event of shell.js reads DSShell.handlers[id]) and
   waits for a record of the command.

   After setup and every preset change the driver reads the NSMenu of macOS and asserts the AppKit
   Shift convention that menu.rs `fix_shift_equivalents` applies (a Shift+letter accelerator has
   the UPPERCASE letter and no Shift in the mask, a Cmd+letter stays lowercase) — the
   deterministic proof that the fix took effect, per preset: "shift items converted N, wrong 0".

   Three commands are recorded but not executed (their handler call is skipped so that the run
   survives them): quit (ends the app) and close (closes the open project). Everything else runs
   for real; the page dialogs (confirm modal, About, Settings) are answered with cancel/done by
   the driver between the keys, and the native file dialogs are stubbed to null (a cancel).
   A valid selection (a hit) is seeded before every key: the items that are disabled without
   state (redo, nudge, audition) must dispatch; "not dispatched" counts as failed.

   The track height keys (height_taller, height_shorter) additionally assert the px value: the
   wave lane of every track changes to the next height (TRACK_HEIGHTS in app.js).

   The report holds, per preset: keys tested N, passed P, failed F (metrics.keys.<preset>) with
   one row per key; `pass` is false when N is 0 or F > 0. */
(async () => {
  const sc = window.__SELFTEST__;
  const T = window.__TAURI__.core;
  const report = { scenario: sc.name, pass: false, steps: [], metrics: {}, tracks: {}, errors: [] };
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const $q = s => document.querySelector(s);
  const need = (cond, msg) => { if (!cond) throw new Error(msg); };

  async function until(fn, what, ms) {
    const end = performance.now() + (ms || 30000);
    for (;;) {
      const v = await fn();
      if (v) return v;
      if (performance.now() > end) throw new Error(`timed out waiting for ${what}`);
      await sleep(25);
    }
  }
  async function step(name, fn) {
    const t = performance.now();
    try {
      const detail = await fn();
      report.steps.push({ step: name, ok: true, ms: Math.round((performance.now() - t) * 100) / 100, detail: detail === undefined ? null : detail });
    } catch (e) {
      report.steps.push({ step: name, ok: false, ms: Math.round((performance.now() - t) * 100) / 100, error: String((e && e.message) || e) });
      throw e;
    }
  }

  /* ---- the recorder ---- */
  let via = null;                       // the path the current key is sent through ("menu" | "page")
  const ran = [];                       // {cmd, via}
  const RECORD_ONLY = new Set(["quit", "close"]);   // the run must survive these
  function wrapAll() {
    const wrap = (obj, id) => {
      const orig = obj[id];
      obj[id] = function (...a) {
        ran.push({ cmd: id, via });
        if (RECORD_ONLY.has(id)) return;
        return orig.apply(this, a);
      };
    };
    for (const c of DSKeys.COMMANDS) {
      if (DSShell.handlers[c.id]) wrap(DSShell.handlers, c.id);
      if (typeof KEY_RUN !== "undefined" && KEY_RUN[c.id]) wrap(KEY_RUN, c.id);
    }
  }
  const ranSince = (n, cmd, wantVia) => ran.slice(n).some(e => e.cmd === cmd && e.via === wantVia);

  /* ---- dialogs: the native ones answer null (a cancel), the page modals are drained ---- */
  /* A refused dispatch is retried once (#207): shell.js menu() drops a menu event while a page
     modal is open, and a modal of an earlier row can open late (openExport awaits first) — the
     Rust side logged the dispatch, the row still timed out. drainSettled closes the modal before
     the press; if the record is still missing, the key is pressed again after a settled drain and
     the row notes `retried`. A handler that then still never runs stays a failed row. */
  const dialog = window.__TAURI__.dialog;
  for (const kind of ["open", "save", "message", "confirm", "ask"]) {
    if (dialog && typeof dialog[kind] === "function") dialog[kind] = async () => null;
  }
  const CLOSE_ID = /cancel|done|close/i;
  const CLOSE_TEXT = /^(cancel|discard|close|no|got it|done)$/i;
  async function drain() {
    for (let i = 0; i < 30; i++) {
      const vis = [...document.querySelectorAll(".modal")].filter(m => !m.classList.contains("hidden"));
      if (!vis.length) return;
      for (const m of vis) {
        const btns = [...m.querySelectorAll("button")];
        const btn = btns.find(b => CLOSE_ID.test(b.id)) || btns.find(b => CLOSE_TEXT.test(b.textContent.trim()));
        if (btn) btn.click(); else m.classList.add("hidden");
      }
      await sleep(20);
    }
  }
  /* A modal of an earlier row can open LATE: openExport awaits (flushEvents, smRefreshAll,
     exportExisting) before it shows #exportModal, so the drain right after the export row sees
     nothing and the modal appears during the next row. shell.js menu() refuses a dispatch while a
     modal is open ("answer the open dialog first"), the Rust log still shows the id, and the row
     times out: #207 lost exactly cubase/undo (the row after export) this way, 2 of 3 runs on
     arm64-mac. Drain until a round passes with no modal, so the refusal window stays shut. */
  async function drainSettled(rounds) {
    for (let i = 0; i < (rounds || 3); i++) { await drain(); await sleep(150); }
    await drain();
  }
  const openModals = () => [...document.querySelectorAll(".modal:not(.hidden)")].map(m => m.id || m.classList[0]).join(",") || "none";
  const evidence = id => `modals open: ${openModals()}; stMsg: ${JSON.stringify($q("#stMsg") && $q("#stMsg").textContent)}; ` +
    `DSShell.log: seen ${DSShell.log.seen}, done ${DSShell.log.done}, last ${JSON.stringify(DSShell.log.last)}; ` +
    `undo ${S.undo.length}, redo ${S.redo.length} (waiting for ${id})`;

  /* ---- a key of the table as a keydown event (the mapping of DSKeys.keyOf backwards) ---- */
  const NAMED = { Space: " ", Return: "Enter", Delete: "Delete", Backspace: "Backspace", Escape: "Escape",
    Left: "ArrowLeft", Right: "ArrowRight", Up: "ArrowUp", Down: "ArrowDown" };
  const NUMPAD = { NumpadMinus: ["-", "NumpadSubtract"], NumpadPlus: ["+", "NumpadAdd"], NumpadPeriod: [".", "NumpadDecimal"],
    NumpadEnter: ["Enter", "NumpadEnter"] };
  function synthEvent(k) {
    const p = DSKeys.parse(k);
    need(p, `${k} is not in the table's notation`);
    let key = null, code = "";
    const k0 = p.key;
    if (/^[A-Z0-9]$/.test(k0)) { key = k0.toLowerCase(); if (/[A-Z]/.test(k0)) code = "Key" + k0; else code = "Digit" + k0; }
    else if (NAMED[k0]) key = NAMED[k0];
    else if (NUMPAD[k0]) { [key, code] = NUMPAD[k0]; }
    else if (/^Numpad[0-9]$/.test(k0)) { key = k0.slice(6); code = k0; }
    else if (k0 === "Plus") key = "+";
    else if (k0 === "Minus") key = "-";
    else { key = k0; if (k0 === "[") code = "BracketLeft"; if (k0 === "]") code = "BracketRight"; }
    return new KeyboardEvent("keydown", { key, code: code || undefined,
      ctrlKey: p.mods.includes("Control"), metaKey: p.mods.includes("Cmd"),
      altKey: p.mods.includes("Option"), shiftKey: p.mods.includes("Shift"), bubbles: true });
  }

  /* ---- the state that disabled-by-state menu items need (redo, nudge, audition): a valid
     selection. A delete key removes the selected hit: re-seed one (a real edit) when it is gone. ---- */
  async function ensureSel() {
    if (S.sel && S.events[S.sel.track] && S.sel.index < (S.events[S.sel.track] || []).length) return;
    let tr = S.order.find(n => (S.events[n] || []).length > 0);
    if (!tr) {
      tr = S.order[0];
      need(tr, "no track to seed a hit into");
      const spike = $q(`.track[data-name="${CSS.escape(tr)}"] .spike`);
      need(spike, `no spike element of ${tr}`);
      const r = spike.getBoundingClientRect();
      const n0 = (S.events[tr] || []).length;
      // a dblclick can land on a frame where the lane does not take it (a just-scrolled view after
      // an undo/redo reveal, a transient overlay): three tries, a new hit each time it worked
      for (let tryN = 0; (S.events[tr] || []).length === n0 && tryN < 3; tryN++) {
        spike.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, clientX: r.left + 40, clientY: r.top + 8 }));
        await until(() => (S.events[tr] || []).length > n0, "the seed hit to be added", 5000)
          .catch(() => {});
      }
      need((S.events[tr] || []).length > n0, "the seed hit to be added (3 tries)");
    }
    S.sel = { track: tr, index: 0 };
    await DSShell.sync();
  }

  /* ---- track height (px assertion) ---- */
  const wavePx = () => {
    const lanes = [...document.querySelectorAll("#lanesScroll canvas.wave")];
    return { n: lanes.length, px: lanes.length ? Math.round(lanes[0].getBoundingClientRect().height) : null,
      var: parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--wave-h")) };
  };
  // `from` is the height the reset after each key restores (the state before the key: taller runs
  // first and leaves medium behind, so shorter resets to medium, not to its pre-cycle large)
  const HEIGHT_EXPECT = { height_taller: { from: "medium", to: "large" }, height_shorter: { from: "medium", to: "small" } };
  try {
    await step("cold_start", async () => {
      need(B.app === true, "B.app is not true");
      need(window.__DS_NATIVE_MENU__ === true, "the shell did not set __DS_NATIVE_MENU__");
      need(typeof DSKeys === "object" && DSKeys.COMMANDS.length > 50, "the key table did not load");
      need(typeof DSShell === "object" && DSShell.handlers, "shell.js is not loaded");
      need(DSKeys.problems(DSKeys.COMMANDS).length === 0, `static/keymap.js: ${DSKeys.problems(DSKeys.COMMANDS).join("; ")}`);
      wrapAll();
      return { commands: DSKeys.COMMANDS.length };
    });

    // the project the scenario names (DRUMSPIKE_SELFTEST_PROJECT): File > Open with the native
    // dialog stubbed to it (the same way shell_driver opens a project); the lanes give the px
    // assertion its canvases
    await step("project_open", async () => {
      // a sid from localStorage of an earlier run of this .app is stale (the registry of the new
      // instance does not know it): ignore a close failure for it
      if (S.sid) { try { await T.invoke("project_close", { sid: S.sid, force: true }); } catch (e) {} S.sid = null; localStorage.removeItem("dsg_sid"); setDirtyState(false); await initSessions(); await loadProject(); }
      dialog.open = async () => sc.project;
      await DSShell.handlers.open({ id: "open" });
      await until(() => S.sid && S.loaded > 0 && S.order.every(n => n in S.peaks), "the project to load", 60000);
      await drain();
      return { tracks: S.order.length };
    }).catch(() => {
      throw new Error(`the project to load (sid=${JSON.stringify(S.sid)}, loaded=${JSON.stringify(S.loaded)}, order=${S.order.length}, tracks=${Object.keys(S.tracks || {}).length}, msg=${$q("#stMsg") && $q("#stMsg").textContent}, peaks=${Object.keys(S.peaks || {}).length})`);
    });

    /* undo, redo and export_again are greyed on a fresh project: seed the state they need (a real
       nudge edit gives the undo stack an entry, the undo after it gives redo its entry, and the
       export shape below turns Export Again on) */
    await step("seed_state", async () => {
      await ensureSel();   // a selected hit: redo, the nudge items and audition need state
      let tr = S.sel.track;
      if ((S.events[tr] || []).length === 0) throw new Error("ensureSel left no hit on " + tr);
      if (typeof KEY_RUN.nudge_later !== "function") throw new Error("KEY_RUN.nudge_later is missing");
      const n = ran.length; via = "page";
      KEY_RUN.nudge_later();
      await until(() => ranSince(n, "nudge_later", "page"), "the seed nudge to run", 5000);
      await until(() => S.undo.length > 0, "the seed nudge to make an undo step", 5000);
      if (typeof KEY_RUN.undo !== "function") throw new Error("KEY_RUN.undo is missing");
      const u = ran.length;
      KEY_RUN.undo();
      await until(() => ranSince(u, "undo", "page"), "the seed undo to run", 5000);
      await until(() => S.redo.length > 0, "the seed undo to make a redo step", 5000);
      // the export dialog of the keys loop is answered with cancel (drain), so no export runs:
      // seed the export shape through the same backend call the page's exRecord uses, so that
      // File > Export Again is on when the loop reaches it (it goes last)
      await B.exportLastSet(S.sid, { formats: ["midi"], midi_mode: "single", compensate: true, scope: "all", track: null, dest: null });
      S.exportAgain = true;
      await DSShell.sync();   // the menu items enabled/disabled must match the seeded stacks
      return { undo: S.undo.length, redo: S.redo.length, export_again: S.exportAgain };
    });

    /* every key of every row of every preset; a row without its own Cubase list is tested in both
       passes (the gate asks for every key of every row in every preset); export_again goes last
       (its item is greyed before the first export, which the keys loop itself performs) */
    await step("keys", async () => {
      const commands = [...DSKeys.COMMANDS].sort((a, b) => (a.id === "export_again") - (b.id === "export_again"));
      const results = [];                 // one row per (preset, command, key)
      const wasPreset = DSKeys.preset;
      try {
        for (const preset of Object.keys(DSKeys.PRESETS)) {
          need(setKeyPreset(preset, false), `setKeyPreset(${preset}) refused`);
          await DSShell.sync();
          await until(async () => (await T.invoke("selftest_menu_report")).preset === preset,
            `the menu to have the ${preset} accelerators`, 10000);
          const byId = Object.fromEntries((await T.invoke("selftest_menu_report")).items.map(i => [i.id, i]));
          // the menu is rebuilt for the preset: read the NSMenu of macOS after the sync
          const nsRows = await T.invoke("selftest_ns_menu");
          let nsLive = nsRows;   // re-read for export_again (its item enables at the first export)
          need(Array.isArray(nsRows) && nsRows.length > 50, `the NSMenu report is empty: ${JSON.stringify(nsRows).slice(0, 200)}`);
          report.metrics["ns_menu_" + preset] = nsRows;   // evidence: what macOS has registered (label, key, mask)
          // the deterministic check of the AppKit Shift convention (no GUI event needed): after
          // setup and every preset change, every NSMenu item whose accelerator has Shift+letter
          // must carry the UPPERCASE letter as the key equivalent and NO Shift in its mask
          // (fix_shift_equivalents); a Cmd+letter item stays lowercase. FAIL on wrong > 0 or N = 0.
          let convN = 0; const convWrong = [];
          for (const r of nsRows) {
            if (r.key && r.key.length === 1 && /[A-Za-z]/.test(r.key)) {
              if (r.mask & (1 << 17)) convWrong.push(`${r.menu}: ${r.label} key ${JSON.stringify(r.key)} mask ${r.mask} (Shift letter not converted)`);
              else if (/[A-Z]/.test(r.key)) convN++;
            }
          }
          report.metrics["shift_conv_" + preset] = { converted: convN, wrong: convWrong };
          need(convWrong.length === 0, `${preset}: shift items converted ${convN}, wrong ${convWrong.length}: ${convWrong.slice(0, 4).join(" | ")}`);
          need(convN > 0, `${preset}: shift items converted 0`);
          const nsOf = label => nsLive.filter(r => r.label === label);
          const stat = { tested: 0, passed: 0, failed: [] };
          for (const c of commands) {
            const mk = DSKeys.menuKeyOf(c, preset);
            for (const k of DSKeys.keysOf(c, preset)) {
              const row = { preset, cmd: c.id, key: k, path: k === mk ? "menu" : "page", pass: false, why: null };
              results.push(row);
              stat.tested++;
              try {
                await ensureSel();                 // a valid selection: redo, nudge, audition stay enabled
                if (c.id === "undo" || c.id === "redo") {
                  // keys tested earlier (nudge, delete, undo) drain or clear the stacks that
                  // seed_state built: re-seed right before the key runs. A step on the page is not
                  // enough: AppKit SWALLOWS a key equivalent that matches a disabled item and
                  // performKeyEquivalent still returns YES (the #206 probe), so the seed also runs
                  // when the live menu item is disabled while the page stack is not empty.
                  // undo needs an edit on the stack, redo needs an undone one.
                  const want = c.id;
                  let off = S[want].length === 0;
                  if (!off && row.path === "menu") {
                    const rep = await T.invoke("selftest_menu_report");
                    off = !((rep.items.find(i => i.id === want) || {}).enabled);
                  }
                  if (off) {
                    if (typeof KEY_RUN.nudge_later !== "function" || typeof KEY_RUN.undo !== "function")
                      throw new Error("KEY_RUN.nudge_later/undo is missing");
                    const edit = want === "redo" && S.undo.length === 0;   // a new edit clears redo: nudge first, then undo
                    if (edit || (want === "undo" && S.undo.length === 0)) {
                      const n = ran.length; via = "page"; KEY_RUN.nudge_later();
                      await until(() => ranSince(n, "nudge_later", "page"), `the ${want} seed nudge to run`, 5000);
                    }
                    if (want === "redo") {
                      const u = ran.length; via = "page"; KEY_RUN.undo();
                      await until(() => ranSince(u, "undo", "page"), "the redo seed undo to run", 5000);
                      await until(() => S.redo.length > 0, "the redo seed undo to make a redo step", 5000);
                    }
                    await DSShell.sync();
                    if (row.path === "menu") {
                      // the enabled state must be live in the NSMenu before the perform: wait for
                      // muda to report the item on (set_enabled runs inside the sync)
                      await until(async () => {
                        const rep = await T.invoke("selftest_menu_report");
                        return !!((rep.items.find(i => i.id === want) || {}).enabled);
                      }, `the ${want} item to enable after the seed`, 5000);
                    }
                  }
                }
                await drainSettled();              // no page dialog may block the menu event, even one that opens late
                via = row.path;
                const mark = ran.length;
                const dispatchKey = async () => {  // the perform, once: the menu path or the page path
                  if (row.path === "menu") {
                    const r = await T.invoke("selftest_menu_key", { key: k });
                    need(r.matched === 1, `${c.id}: NSMenu performKeyEquivalent did not match (${JSON.stringify(r)})`);
                  } else {
                    window.dispatchEvent(synthEvent(k));
                  }
                };
                if (row.path === "menu") {
                  if (c.id === "export_again") nsLive = await T.invoke("selftest_ns_menu");
                  // 1) the live menu item carries the accelerator of the table
                  const it = byId[c.menu];
                  need(it, `${c.id}: no menu item`);
                  const want = DSKeys.tauriOf(k);
                  need((it.accel || null) === want, `${c.id}: the live menu has ${it.accel}, the table ${want}`);
                  // the NSMenu of macOS has the key equivalent (muda's mapping) and the item is enabled.
                  // The label grows with the state ("Undo Add Hit (k in, 14.701 s)"): match it as a prefix.
                  // Undo/redo enable and disable with every edit: their enabled state in the preset
                  // snapshot (nsLive) is stale by the time the key runs — re-read the NSMenu first.
                  if (c.id === "undo" || c.id === "redo") nsLive = await T.invoke("selftest_ns_menu");
                  const pk = DSKeys.parse(k).key;
                  const chars = { Up: "\u{F700}", Down: "\u{F701}", Left: "\u{F702}", Right: "\u{F703}",
                    Return: "\u{000d}", Tab: "\u{0009}" }[pk];
                  // #206: the AppKit convention that menu.rs applies: a Shift+Cmd accelerator has the
                  // UPPERCASE letter as the key equivalent and no Shift in the mask
                  const eq = chars ?? (pk.length === 1 ? (k.includes("Shift") ? pk.toUpperCase() : pk.toLowerCase()) : pk);
                  const ns = () => nsLive.filter(r => r.key === eq && (r.label === c.title || r.label.startsWith(c.title)));
                  let hit = ns()[0];
                  need(hit, `${c.id}: "${c.title}" is not in the NSMenu with the key equivalent ${JSON.stringify(eq)} (rows: ${nsLive.filter(r => r.key === eq).map(r => JSON.stringify(r.label)).join(", ")})`);
                  if (!hit.enabled && c.id === "export_again") {
                    // the first export enables the item asynchronously (exRecord -> DSShell.sync):
                    // poll until the item is on, else the item is disabled for real
                    await until(async () => {
                      nsLive = await T.invoke("selftest_ns_menu");
                      hit = ns()[0];
                      return hit && hit.enabled;
                    }, `export_again to enable at the first export`, 20000);
                  }
                  need(hit.enabled, `${c.id}: the NSMenu item is disabled`);
                  // 3) the real native path: NSMenu performKeyEquivalent with a synthetic keyDown
                  await dispatchKey();
                } else {
                  await dispatchKey();
                }
                const pk2 = DSKeys.parse(k).key;
                const eq2 = { Up: "\u{F700}", Down: "\u{F701}", Left: "\u{F702}", Right: "\u{F703}",
                  Return: "\u{000d}", Tab: "\u{0009}" }[pk2] ?? (pk2.length === 1 ? pk2.toLowerCase() : pk2);
                await until(() => ranSince(mark, c.id, row.path), `${c.id} (${k}, ${row.path}) to run`, 10000)
                  .catch(async e => {
                    // one honest retry (#207): the dispatch can be refused by a modal of an earlier
                    // row that opened late (menu() answers the open dialog first; the Rust log still
                    // shows the id) or lost while the page is busy. Drain and press again. A key
                    // whose handler still never runs stays a failure, now with the page evidence.
                    const before = evidence(c.id);
                    await drainSettled();
                    const mark2 = ran.length;
                    await dispatchKey();
                    await until(() => ranSince(mark2, c.id, row.path), `${c.id} (${k}, ${row.path}) to run`, 10000)
                      .catch(async e2 => {
                        const log = await T.invoke("selftest_menu_log").catch(() => null);
                        throw new Error(`${e2.message} [before the retry: ${before}] [rust menu log tail: ${JSON.stringify((log || []).slice(-8))}]`);
                      });
                    row.retried = true;
                  });
                if (HEIGHT_EXPECT[c.id]) {
                  // the lane height of every track changed one step (assert the px value)
                  const want = HEIGHT_EXPECT[c.id];
                  need(S.trackHeight === want.to, `${c.id}: track height is ${S.trackHeight}, not ${want.to}`);
                  const px = wavePx();
                  need(px.n > 0, `${c.id}: no wave lanes to measure`);
                  need(px.px === ({ small: 56, medium: 120, large: 240 })[want.to],
                    `${c.id}: wave lane ${px.px} px, not the ${want.to} value`);
                  setTrackHeight(want.from, false);   // back for the next key
                }
                row.pass = true;
              } catch (e) { row.why = String((e && e.message) || e); }
              if (!row.pass) {
                // #206 rework (c-6328710125): "not dispatched" counts as failed. The state that
                // disabled-by-state items need (an edit to undo, a selected hit) is seeded before
                // every key, so a not-dispatched key is a real failure now.
                stat.failed.push(`${row.preset}/${c.id}/${k}: ${row.why}`);
              }
              else stat.passed++;
              await drainSettled(2);             // close a modal this row opened (possibly late) before the next row
            }
          }
          report.metrics.keys = report.metrics.keys || {};
          report.metrics.keys[preset] = { ...stat, rows: results.filter(r => r.preset === preset) };
          need(stat.tested > 0, `${preset}: no keys tested`);
          // the run continues into the next preset even with failures (the coordinator's totals
          // per preset); the gate fails at the end with every preset's tally
          stat.failed.length && stat.failed.forEach(f => report.errors.push(f));
        }
      } finally {
        setKeyPreset(wasPreset, false);
        await DSShell.sync();
      }
      const presets = Object.keys(DSKeys.PRESETS);
      const tally = Object.fromEntries(presets.map(p => {
        const s = report.metrics.keys[p];
        const cv = report.metrics["shift_conv_" + p];
        const conv = cv ? `, shift items converted ${cv.converted}, wrong ${cv.wrong.length}` : "";
        return [p, s ? `${DSKeys.presetName(p)}: keys tested ${s.tested}, passed ${s.passed}, failed ${s.failed.length}${conv}` : "NOT REACHED"];
      }));
      const allFailed = presets.flatMap(p => (report.metrics.keys[p] || {}).failed || []);
      need(allFailed.length === 0, Object.entries(tally).map(([p, t]) => `${p}: ${t}`).join("; ") + " | " + allFailed.slice(0, 8).join(" | "));
      return tally;
    });

    report.pass = true;
  } catch (e) {
    report.errors.push(String((e && e.message) || e));
  }
  await T.invoke("selftest_report", { report });
})();
