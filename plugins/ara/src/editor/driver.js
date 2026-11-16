/* The test driver of the editor page (M1b2). The plugin adds it after shell.js ONLY when the
   test switch DS_ARA_EDITOR_MODE is set (a real host sets none). It runs on the real page and
   reports to the plugin through window.ipc (rows "page_report" of the plugin's report).
   window.__DS_TEST__ = {mode, arg, fault} comes from the plugin.
   Modes: ready (boot report only), measure, keys, focus, resize, edit, shell, analyse, net. */
(function () {
  "use strict";
  var T = window.__DS_TEST__ || { mode: "ready", arg: "", fault: "" };
  function post(o) { try { window.ipc.postMessage(JSON.stringify(o)); } catch (e) { /* no ipc */ } }
  var violations = [];
  document.addEventListener("securitypolicyviolation", function (ev) {
    violations.push({ directive: ev.violatedDirective, blocked: ev.blockedURI });
  });
  var KEY_NAMES = { Space: "Space", ArrowRight: "Right", Tab: "Tab", Escape: "Esc" };
  var keySeen = [], focusKeys = [];
  document.addEventListener("keydown", function (ev) {
    var name = KEY_NAMES[ev.code] || (ev.code === "KeyZ" ? "Z" : null);
    if (!name) return;
    if (T.mode === "focus") focusKeys.push({ t: Math.round(performance.now()), key: name });
    var row = { key: name, meta: !!ev.metaKey };
    try { var f = typeof DSKeys !== "undefined" && DSKeys.find ? DSKeys.find(ev) : null; if (f) row.page = f.cmd; } catch (e) { /* ignore */ }
    keySeen.push(row);
  }, true);

  async function raw(cmd, args) {   // the bytes of the service's JSON answer, as text
    var r = await fetch("/__ds/invoke/" + cmd, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(args || {}) });
    return { status: r.status, text: await r.text() };
  }
  function whenBooted(fn, deadlineMs) {
    var t0 = performance.now();
    (function poll() {
      var loaded = false;
      try { loaded = typeof S !== "undefined" && S.loaded > 0; } catch (e) { /* not yet */ }
      if (loaded || performance.now() - t0 > (deadlineMs || 15000)) fn(loaded);
      else setTimeout(poll, 100);
    })();
  }
  function resources() {
    return performance.getEntriesByType("resource").map(function (e) { return e.name; });
  }
  function p95(a) { if (!a.length) return null; var s = a.slice().sort(function (x, y) { return x - y; }); return s[Math.min(s.length - 1, Math.floor(0.95 * (s.length - 1)))]; }

  if (T.mode === "net") {   // the fault run of the no-network line: the page asks for a foreign host
    try { var im = new Image(); im.src = "http://127.0.0.1:9/x.png"; } catch (e) { /* refused */ }
    try { fetch("http://127.0.0.1:9/x").catch(function () {}); } catch (e) { /* refused */ }
  }

  whenBooted(async function (loaded) {
    var names = [], rawEvents = {}, n = 0;
    try { names = S.order.slice(); } catch (e) { /* no state */ }
    for (var i = 0; i < names.length; i++) {
      var r = await raw("events_get", { sid: S.sid, name: names[i] });
      rawEvents[names[i]] = r.text;
      try { n += JSON.parse(r.text).events.length; } catch (e) { /* counted 0 */ }
    }
    post({ kind: "boot", loaded: loaded, sid: S.sid, tracks: names, n_events: n, raw_events: rawEvents,
      hostMode: typeof B !== "undefined" && !!B.hostMode, tauri: !!(window.__TAURI__ && window.__TAURI__.core),
      resources: resources(), violations: violations, innerW: window.innerWidth, innerH: window.innerHeight });
    if (T.mode === "measure") measure(loaded);
    if (T.mode === "keys") setTimeout(function () { post({ kind: "keys", seen: keySeen, hasFocus: document.hasFocus() }); }, 7000);
    if (T.mode === "focus") focus();
    if (T.mode === "resize") sizes();
    if (T.mode === "edit") await edit(names);
    if (T.mode === "shell") await shell(names);
    if (T.mode === "analyse") await analyse(names);
    if (T.mode === "bigpost") await bigpost(names);
  });

  function measure() {
    var cs = Array.prototype.slice.call(document.querySelectorAll("#lanesScroll canvas"));
    var t0 = performance.now(), last = t0, deltas = [], n = 0, up = false, c = cs[0];
    var iv = setInterval(function () {   // scroll 10x/s, zoom 2x/s (the human drive of the gate)
      if (!c) return;
      n++;
      var opts = { deltaY: up ? -240 : 240, deltaX: 0, deltaMode: 0, bubbles: true, cancelable: true, ctrlKey: n % 5 === 0 };
      c.dispatchEvent(new WheelEvent("wheel", opts));
      if (n % 25 === 0) { up = !up; c = cs[(n / 25) % cs.length | 0] || c; }
    }, 100);
    (function loop(t) {
      deltas.push(t - last); last = t;
      if (t - t0 < 8000) { requestAnimationFrame(loop); return; }
      clearInterval(iv);
      deltas = deltas.slice(20);
      var mx = 0; deltas.forEach(function (d) { if (d > mx) mx = d; });
      post({ kind: "measure", frames: deltas.length, p95_ms: p95(deltas), max_ms: mx, canvases: cs.length,
        innerW: window.innerWidth, innerH: window.innerHeight });
    })(t0);
  }
  function focus() {
    var s = [], t0 = performance.now();
    var iv = setInterval(function () {
      s.push({ t: Math.round(performance.now() - t0), hasFocus: document.hasFocus() });
      if (performance.now() - t0 > 7000) { clearInterval(iv); post({ kind: "focus", samples: s, keys: focusKeys }); }
    }, 300);
  }
  function sizes() {
    var s = [], t0 = performance.now();
    var iv = setInterval(function () {
      s.push({ t: Math.round(performance.now() - t0), innerW: window.innerWidth, innerH: window.innerHeight });
      if (performance.now() - t0 > 6000) { clearInterval(iv); post({ kind: "resize", samples: s }); }
    }, 200);
  }
  async function edit(names) {   // move one hit through the page's own backend call
    var name = names[0], before = await raw("events_get", { sid: S.sid, name: name });
    var evs = JSON.parse(before.text).events, idx = Math.min(1, evs.length - 1), from = evs[idx].t, to = from + 0.01;
    var res = null, err = null;
    try { res = await B.eventsEdit(S.sid, name, [{ op: "move", index: idx, t: to }]); } catch (e) { err = String(e && e.message || e); }
    var after = await raw("events_get", { sid: S.sid, name: name });
    post({ kind: "edit", name: name, index: idx, from: from, to: to, error: err, before_raw: before.text, after_raw: after.text });
  }
  async function shell(names) {   // every shell command and the four binary commands
    var out = {}, sid = S.sid, name = names[0];
    var cmds = ["shell_sync", "shell_typing", "app_ready", "open_paths_take", "quit_app", "reveal_path", "about_info",
      "about_third_party", "open_link", "open_url", "startup_mark", "splash_painted", "splash_dismiss", "splash_report",
      "selftest_clock", "plugin:drag|start_drag"];
    var args = { shell_sync: { has_project: true, state: null }, shell_typing: { on: true }, reveal_path: { path: T.arg || "/tmp" },
      open_link: { id: "website" }, open_url: { url: "https://example.com/" } };
    for (var i = 0; i < cmds.length; i++) {
      try { var v = await window.__TAURI__.core.invoke(cmds[i], args[cmds[i]] || {}); out[cmds[i]] = { ok: true, v: v === undefined ? null : v }; }
      catch (e) { out[cmds[i]] = { ok: false, code: e && e.code, message: e && e.message }; }
    }
    var bin = {};
    async function bytes(label, cmd, a) {
      try { var b = await window.__TAURI__.core.invoke(cmd, a); bin[label] = { ok: b instanceof ArrayBuffer, len: b.byteLength || 0 }; }
      catch (e) { bin[label] = { ok: false, code: e && e.code, message: e && e.message }; }
    }
    var info = await window.__TAURI__.core.invoke("peaks_info", { sid: sid, name: name }).catch(function () { return null; });
    var lv = info ? Object.keys(info.levels)[0] : "0";
    await bytes("peaks_tile", "peaks_tile", { sid: sid, name: name, level: +lv, from: 0, to: info ? info.levels[lv].n : 1 });
    await bytes("samples_window", "samples_window", { sid: sid, name: name, start: 0, end: 2000 });
    if (T.arg) {
      await window.__TAURI__.core.invoke("track_layer_set", { sid: sid, name: name, path: T.arg }).catch(function () {});
      await bytes("track_layer_wave", "track_layer_wave", { sid: sid, name: name, start: 0, end: 2000 });
      await window.__TAURI__.core.invoke("track_sampler_set", { sid: sid, name: name, path: T.arg }).catch(function () {});
      await bytes("track_sampler_wave", "track_sampler_wave", { sid: sid, name: name, start: 0, end: 2000 });
    }
    var errc = null;
    try { await window.__TAURI__.core.invoke("project_get", { sid: "no-such-sid" }); } catch (e) { errc = { code: e.code, message: e.message }; }
    var nocmd = null;
    try { await window.__TAURI__.core.invoke("no_such_command", {}); } catch (e) { nocmd = { code: e.code, message: e.message }; }
    var panicked = null;
    try { await window.__TAURI__.core.invoke("tracks_get", { sid: sid }); panicked = "answered"; } catch (e) { panicked = { code: e.code, message: e.message }; }
    // a channel message: a read-only command with a progress channel is not needed; the event
    // listener is checked by registering one
    var listened = false;
    try { var un = await window.__TAURI__.event.listen("project-dirty", function () {}); listened = typeof un === "function"; } catch (e) { /* no */ }
    post({ kind: "shell", commands: out, binary: bin, error_shape: errc, unknown: nocmd, tracks_get: panicked, listen: listened,
      keys: Object.keys(window.__TAURI__).sort(), core: Object.keys(window.__TAURI__.core).sort(), hostMode: window.__DS_HOST_MODE__ });
  }
  async function bigpost(names) {   // a 300 KB body through the custom scheme (events_set of a dense track)
    var evs = [], i;
    for (i = 0; i < 16000; i++) evs.push({ t: Math.round(i * 0.00625 * 1e6) / 1e6, v: 100 });
    var body = JSON.stringify({ sid: S.sid, name: names[0], events: evs });
    var err = null;
    try { await window.__TAURI__.core.invoke("events_set", { sid: S.sid, name: names[0], events: evs }); } catch (e) { err = String(e && e.message || e); }
    var back = await raw("events_get", { sid: S.sid, name: names[0] }), n = -1;
    try { n = JSON.parse(back.text).events.length; } catch (e) { /* no */ }
    post({ kind: "bigpost", bytes: body.length, sent: evs.length, stored: n, error: err });
  }
  async function analyse(names) {   // an analysis started from the page
    var ch = new window.__TAURI__.core.Channel(), msgs = 0;
    ch.onmessage = function () { msgs++; };
    var t0 = performance.now(), st = null, err = null;
    try {
      await window.__TAURI__.core.invoke("analyze_start", { sid: S.sid, tracks: names, params: null, replace: true, keep_edited: false, progress: ch });
      for (;;) {
        st = await window.__TAURI__.core.invoke("analyze_status", { sid: S.sid });
        if (st.state === "done" || performance.now() - t0 > 60000) break;
        await new Promise(function (r) { setTimeout(r, 100); });
      }
    } catch (e) { err = String(e && e.message || e); }
    post({ kind: "analyse", state: st && st.state, ms: Math.round(performance.now() - t0), channel_messages: msgs, error: err });
  }
  /* a page-side post that the host can ask for at any time */
  window.__DS_REPORT_NOW__ = function () { post({ kind: "now", resources: resources(), violations: violations }); };
  if (T.mode === "net") setTimeout(window.__DS_REPORT_NOW__, 6000);
})();
