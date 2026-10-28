/* Self-test driver of the scenario `tracks` (P3-4c). Like shell_driver.js it runs in the real WKWebView of the
   debug .app with the real commands and the real page code, and sends one JSON report to `selftest_report`.
   It counts the user actions from the drop of a folder of real drum tracks to Export all (exit E14): the drop
   (the paths reach the page like a Finder or window drop: `selftest_open_paths`, the queue, the event
   `open-paths`, DSShell.hooks.otherPaths), one click on "Add n tracks", one click on Analyze all, one click on
   Export all. No key is pressed and no field is filled: a listener counts keydown and input events.
   A native drag cannot be driven without the GUI; the drop is the same code path from the queue on. */
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
      await sleep(25);
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
  const actions = [];
  let keys = 0;
  document.addEventListener("keydown", () => { keys++; }, true);
  document.addEventListener("input", () => { keys++; }, true);
  const act = name => actions.push(name);
  const shown = id => { const e = $q(id); return !!e && !e.classList.contains("hidden"); };

  try {
    await step("cold_start", async () => {
      need(B.app === true, "B.app is not true");
      need(typeof DSShell === "object" && typeof DSShell.hooks.otherPaths === "function", "the drop hook is not set");
      for (const s of await T.invoke("project_list", {})) await T.invoke("project_close", { sid: s.sid, force: true });
      S.sid = null; setDirtyState(false); await initSessions(); await loadProject();
      need(!S.sid, "a project is open");
      return { drop: sc.drop };
    });

    let t0;
    await step("drop_to_list", async () => {
      t0 = now();
      act("drop");
      await T.invoke("selftest_open_paths", { paths: [sc.drop] });
      await until(() => shown("#tracksModal") && $q("#tracksRows").children.length > 0, "the confirm list", 20000);
      const rows = [...document.querySelectorAll("#tracksRows .urow")].map(r => ({ stem: r.dataset.stem,
        role: r.querySelector(".urole").value, note: r.querySelector(".unote").value, noguess: r.classList.contains("noguess") }));
      report.metrics.drop_to_list_ms = r2(now() - t0);
      report.metrics.roles = Object.fromEntries(rows.map(r => [r.stem, `${r.role} ${r.note}`]));
      need(rows.length === sc.expect.tracks, `the list has ${rows.length} rows, expected ${sc.expect.tracks}`);
      const wrong = rows.filter(r => sc.expect.roles[r.stem] !== r.role).map(r => `${r.stem}: ${r.role}, expected ${sc.expect.roles[r.stem]}`);
      need(wrong.length === 0, `wrong roles: ${wrong.join("; ")}`);
      need(rows.every(r => !r.noguess), "a row has no guessed role");
      need(!$q("#tracksCopy").checked, "the box Copy is checked by default");
      return { rows: rows.length, title: $q("#tracksTitle").textContent, add_button: $q("#tracksAdd").textContent };
    });

    await step("accept", async () => {
      const t = now();
      act("accept");
      $q("#tracksAdd").click();
      await until(() => S.sid && S.loaded > 0 && S.order.length === sc.expect.tracks && S.order.every(n => n in S.peaks), "the tracks and their lanes", 60000);
      report.metrics.accept_to_lanes_ms = r2(now() - t);
      const p = await T.invoke("project_get", { sid: S.sid });
      const modes = Object.values(p.tracks).map(x => x.source.mode);
      need(modes.every(m => m === "referenced"), `source modes: ${modes.join(",")}`);
      need(Object.values(p.tracks).every(x => x.status === "ok"), "a track is offline");
      for (const [stem, want] of Object.entries(sc.expect.roles)) {
        const tr = Object.entries(p.tracks).find(([, v]) => v.source.path.endsWith(`/${stem}.wav`));
        need(tr && tr[1].role === want, `track of ${stem}: ${tr && tr[1].role}`);
      }
      report.metrics.project_name = p.name;
      return { tracks: S.order.length, name: p.name };
    });

    await step("analyze", async () => {
      const t = now();
      act("analyze");
      $q("#analyzeAllBtn").click();
      await until(() => $q("#progressText").textContent === "analysis done", "the analysis", 240000);
      report.metrics.analyze_ms = r2(now() - t);
      // the page loads the new events after the message "analysis done": wait until it holds what the service holds
      const service = async () => { let n = 0; for (const name of S.order) n += (await T.invoke("events_get", { sid: S.sid, name })).events.length; return n; };
      const page = () => S.order.reduce((a, n) => a + (S.events[n] || []).length, 0);
      await until(async () => page() > 0 && page() === await service(), "the page to hold the detected hits", 30000);
      report.metrics.hits_total = page();
      need(report.metrics.hits_total >= sc.expect.min_hits_total, `only ${report.metrics.hits_total} hits`);
      return { hits: report.metrics.hits_total };
    });

    await step("export", async () => {
      const t = now();
      act("export");
      $q("#exportAllBtn").click();   // P4-30: the arrow opens the Export dialog (everything chosen), Export runs it
      await until(() => !$q("#exportModal").classList.contains("hidden"), "the Export dialog", 15000);
      $q("#exGo").click();
      await until(() => /exported \d+ MIDI and \d+ spike files/.test($q("#stMsg").textContent), "Export", 120000);
      report.metrics.export_ms = r2(now() - t);
      const ex = await T.invoke("export_existing", { sid: S.sid });
      need(ex.count === 2 * sc.expect.tracks, `${ex.count} export files, expected ${2 * sc.expect.tracks}`);
      return { files: ex.count };
    });

    await step("actions", async () => {
      report.metrics.tracks_flow = { user_actions: actions.length, actions, keys_typed: keys };
      need(actions.length === 4, `${actions.length} user actions: ${actions.join(", ")}`);
      need(keys === 0, `${keys} key or input events: the engineer typed`);
      return report.metrics.tracks_flow;
    });

    report.pass = report.steps.every(s => s.ok);
  } catch (e) {
    report.errors.push(String((e && e.message) || e));
    report.pass = false;
  }
  await T.invoke("selftest_report", { report, via_quit: true });
})();
