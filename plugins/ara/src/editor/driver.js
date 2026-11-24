/* The test driver of the editor page (M1b2). The plugin adds it after shell.js ONLY when the
   test switch DS_ARA_EDITOR_MODE is set (a real host sets none). It runs on the real page and
   reports to the plugin through window.ipc (rows "page_report" of the plugin's report).
   window.__DS_TEST__ = {mode, arg, fault} comes from the plugin.
   Modes: ready (boot report only), measure, keys, focus, resize, edit, shell, analyse, net, and (M1b11) hit, menu, geometry, (M1b13) selection, (M1d1) leftovers, (M1d4) firstopen. */
(function () {
  "use strict";
  var T = window.__DS_TEST__ || { mode: "ready", arg: "", fault: "" };
  function post(o) { try { window.ipc.postMessage(JSON.stringify(o)); } catch (e) { /* no ipc */ } }
  var violations = [];
  var reloads = 0;
  try { window.__TAURI__.event.listen("plugin-reload", function () { reloads++; }); } catch (e) { /* no events */ }
  var selEvents = 0, selLast = null;   // M1b13: the events of the host's selection that the page got
  try { window.__TAURI__.event.listen("plugin-selection", function (e) { selEvents++; selLast = e.payload; }); } catch (e) { /* no events */ }
  var regEvents = 0, regLast = null;   // M1b8: the events with the rows of the page in song time
  try { window.__TAURI__.event.listen("plugin-regions", function (e) { regEvents++; regLast = e.payload; }); } catch (e) { /* no events */ }
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
      if (loaded || performance.now() - t0 > (T.mode === "open-first" || T.mode === "firstopen" ? 2500 : (deadlineMs || 15000))) fn(loaded);
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

  window.addEventListener("error", function (ev) { post({ kind: "js_error", message: String(ev.message), line: ev.lineno }); });
  whenBooted(async function (loaded) {   // (open-first: the editor opens before any project exists)
    var names = [], rawEvents = {}, n = 0;
    try { names = S.order.slice(); } catch (e) { /* no state */ }
    for (var i = 0; i < names.length; i++) {
      var r = await raw("events_get", { sid: S.sid, name: names[i] });
      rawEvents[names[i]] = r.text;
      try { n += JSON.parse(r.text).events.length; } catch (e) { /* counted 0 */ }
    }
    post({ kind: "boot", text: String(document.body && document.body.innerText || "").slice(0, 400), loaded: loaded, sid: S.sid, tracks: names, n_events: n, raw_events: rawEvents,
      hostMode: typeof B !== "undefined" && !!B.hostMode, tauri: !!(window.__TAURI__ && window.__TAURI__.core),
      resources: resources(), violations: violations, innerW: window.innerWidth, innerH: window.innerHeight });
    try { if (T.mode === "measure") measure(loaded); } catch (e) { post({ kind: "js_error", message: String(e), stack: String(e && e.stack) }); }
    if (T.mode === "keys") setTimeout(function () { post({ kind: "keys", seen: keySeen, hasFocus: document.hasFocus() }); }, 7000);
    if (T.mode === "focus") focus();
    if (T.mode === "resize") sizes();
    if (T.mode === "edit" || T.mode === "edit-destroy") await edit(names);
    if (T.mode === "edit-round") { await edit(names); await follow(+T.arg || names.length + 1); }
    if (T.mode === "edit-region") { var r0 = reloads; await edit(names, true); await follow(names.length, true, r0); }
    if (T.mode === "open-first") await follow(+T.arg || 4);
    if (T.mode === "names") { try { await namesRun(); } catch (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); } }
    if (T.mode === "refuse") await refuse(names);
    if (T.mode === "shell") { try { await shell(names); } catch (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); } }
    if (T.mode === "analyse" || T.mode === "analyse-race") await analyse(names);
    if (T.mode === "bigpost") await bigpost(names);
    if (T.mode === "transport") transportRun();
    if (T.mode === "hit") hitRun(names).catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "menu") menuRun().catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "geometry") geometryRun().catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "hostkeys") hostkeysRun().catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "exportdialog") exportDialogRun(names).catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "exportrows") exportRowsRun().catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "delivery") deliveryRun(names).catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "deliver") deliverRun().catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "firstopen") firstopenRun().catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "leftovers") leftoversRun().catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "selection") selectionRun().catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "song") songRun().catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
    if (T.mode === "playback") playbackRun().catch(function (e) { post({ kind: "js_error", message: String(e && e.message || e), stack: String(e && e.stack) }); });
  });

  /* Mode selection (M1b13, #562): the host's view sends a list of selections and hide lists (the steps of `--sel-steps`, which the plugin passes
     in T.arg too). Each step is `[pre@]host step`. The pre action is the page's: `zoom` (the view song 3.0 to 4.2 s), `pointer` (the button of the pointer
     goes down and stays), `user` (the same, and the user selects the third track), `key` (a key goes down and stays), `off` and `on` (the
     page's menu item Follow the DAW Selection, clicked through the DOM). After the pre action the page reports `sel-armed`; the host sends its step;
     the page waits for the event of the plugin (or 1.2 s when none comes), lets the page settle, and reads what it shows (`snap1`); it then ends
     the pointer or key action, waits for the page to settle again and reads again (`snap2`), and reports `sel-snap`. The last report `selection`
     carries every step. */
  function selClick(el) { el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true })); }
  async function selMenuFollow() {   // View > Follow the DAW Selection through the panel of the page (clicks on the real elements)
    selClick(document.getElementById("hostMenuBtn"));
    await sleepM(150);
    selClick(document.querySelector('#hostMenu [data-group="View"]'));
    await sleepM(150);
    var it = document.querySelector('#hostMenu [data-host="host_follow_selection"]');
    var was = it && it.getAttribute("aria-checked");
    if (it) selClick(it);
    await sleepM(150);
    return was;
  }
  function selSnap(names) {
    var sc = document.getElementById("lanesScroll"), c = sc.getBoundingClientRect();
    var row = function (n) { return document.querySelector('.track[data-name="' + CSS.escape(n) + '"]'); };
    var rowIn = function (n) { var r = row(n); if (!r) return null; var b = r.getBoundingClientRect(); return b.height > 0 && b.top >= c.top - 1 && b.bottom <= c.bottom + 1; };
    var st = DSHostSel.state(), b = document.getElementById("hostRange"), bb = b.querySelector(".hostBand"), br = (bb || b).getBoundingClientRect(), ed = document.getElementById("editor").getBoundingClientRect();
    return { sel: S.selTrack, marked: st.marked, hidden: st.hidden, note: st.note, range: st.range, follow: st.follow, pending: st.pending, n: st.n,
      band: { shown: getComputedStyle(b).display !== "none" && !!bb && getComputedStyle(bb).display !== "none", n: b.querySelectorAll(".hostBand").length, left: br.left - ed.left, width: br.width,
        want_left: st.range ? S.theadW + Math.max(0, tToX(st.range.t0, laneWidth())) : null,   // the band is cut at the edges of the lane area
        want_width: st.range ? Math.min(laneWidth(), tToX(st.range.t1, laneWidth())) - Math.max(0, tToX(st.range.t0, laneWidth())) : null, top: br.top - ed.top, bottom: br.bottom - ed.top, h: ed.height },
      view: [S.view.t0, S.view.t1], scroll_top: sc.scrollTop, sel_in_view: S.selTrack ? rowIn(S.selTrack) : null,
      rows_in_view: (function () { var o = {}; names.forEach(function (n) { o[n] = rowIn(n); }); return o; })(),
      rows_shown: names.filter(function (n) { var r = row(n); return r && r.getBoundingClientRect().height > 0; }),
      own_range: !!LN.sel, hit: S.sel ? true : false, events: selEvents, last_event: selLast };
  }
  async function selectionRun() {
    var names = S.order.slice();
    var steps = (T.arg || "").split(";").filter(function (x) { return x; });
    if (T.fault === "sel-no-guard") window.hostSelBusy = function () { return false; };   // the fault run: the page overwrites a selection while an action runs
    if (T.fault === "sel-own-range") window.hostSelRange = function (list) { if (list && list.length) lnSetSel(S.selTrack, list[0].t0, list[0].t1); else lnClearSel(); };   // the page takes the range of the host as its own hit selection
    if (T.fault === "sel-no-scroll") { window.hostSelReveal = function () { }; window.hostSelScrollTo = function () { }; }   // nothing scrolls to the track or the range
    if (T.fault === "sel-user-loses") window.hostSelSig = function () { return ""; };   // the page cannot tell that the user changed the selection during an action
    if (T.fault === "sel-no-switch") window.hostSelFollowOn = function () { return true; };   // the switch of the menu does not stop the follow
    if (T.fault === "sel-no-note") { var h0 = window.hostSelHidden; window.hostSelHidden = function (n) { h0(n); if (HS.note) { HS.note.textContent = ""; HS.note.classList.add("hidden"); } }; }   // the rows hide, with no text
    setTrackHeight("large"); setInspector(false, false);
    for (var w = 0; w < 50 && !DSHostSel.state().payload; w++) await sleepM(100);   // the pull of the state the host gave before the editor opened
    await sleepM(600);
    var initial = selSnap(names);
    var results = [], held = null;
    post({ kind: "sel-ready", initial: initial, names: names, steps: steps, rows: names.map(function (n) { var r = document.querySelector('.track[data-name="' + CSS.escape(n) + '"]').getBoundingClientRect(); return Math.round(r.height); }),
      lanes_h: Math.round(document.getElementById("lanesScroll").getBoundingClientRect().height), follow: DSHostSel.follows() });
    for (var k = 0; k < steps.length; k++) {
      var pre = steps[k].indexOf("@") >= 0 ? steps[k].split("@")[0] : "";
      var base = selEvents, doc = document;
      if (pre === "zoom") setView(3.0, 4.2);
      if (pre === "pointer" || pre === "user") { doc.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })); doc.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true })); held = "pointer"; }
      if (pre === "user") setSelTrack(names[2]);   // the user selects a track while the button is down
      if (pre === "key") { doc.dispatchEvent(new KeyboardEvent("keydown", { key: "Shift", bubbles: true })); held = "key"; }
      var menuWas = null;
      if (pre === "off" || pre === "on") menuWas = await selMenuFollow();
      await sleepM(250);
      var before = selSnap(names);
      post({ kind: "sel-armed", k: k, step: steps[k] });
      var t0 = performance.now();
      while (selEvents === base && performance.now() - t0 < 1200) await sleepM(50);
      var gotEvent = selEvents > base;
      await sleepM(900);
      var snap1 = selSnap(names);
      if (held === "pointer") { doc.dispatchEvent(new MouseEvent("mouseup", { bubbles: true })); doc.dispatchEvent(new PointerEvent("pointerup", { bubbles: true })); }
      if (held === "key") doc.dispatchEvent(new KeyboardEvent("keyup", { key: "Shift", bubbles: true }));
      held = null;
      await sleepM(900);
      var snap2 = selSnap(names);
      results.push({ k: k, step: steps[k], pre: pre, got_event: gotEvent, menu_was: menuWas, before: before, snap1: snap1, snap2: snap2 });
      post({ kind: "sel-snap", k: k });
    }
    post({ kind: "selection", visibility: document.visibilityState, names: names, steps: results, host_mode: !!B.hostMode, display: (function () { var d = {}; names.forEach(function (n) { d[n] = S.tracks[n].display_name; }); return d; })() });
  }

  /* Mode hostkeys (M1b12, #561): the test host posts real key events to its window, one every 900 ms after the
     "hostkeys-ready" report, and records every one that reaches the host. This side records what the PAGE saw of
     each key: its notation, how the page classes it (page, absent, none: __DS_KEYS__.classify), whether a page
     handler took it (defaultPrevented), and the text of the status line 600 ms later. The status line is cleared
     before each key, so a text belongs to its key. The keys F13 and F14 are markers: they put the focus into a text
     field that this driver adds to the page (a stand-in for a field of the page) and take it out; F12 ends the run. */
  async function hostkeysRun() {
    var K = window.__DS_KEYS__ || { classify: function () { return { kind: "none" }; }, shown: [] };
    var tr = window.__DS_TRANSPORT__ || { delivered: 0, calls: [] };
    function sleepMs(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
    function msgText() { var el = document.getElementById("stMsg"); return el ? String(el.textContent || "") : null; }
    var field = document.createElement("input");
    field.type = "text"; field.id = "dsTestField";
    field.spellcheck = false; field.setAttribute("autocorrect", "off"); field.setAttribute("autocapitalize", "off"); field.autocomplete = "off";   // the system must not change what the stage types
    field.style.cssText = "position:fixed;left:4px;top:4px;width:120px;height:22px;z-index:99999";
    document.body.appendChild(field);
    var keys = [];
    document.addEventListener("keydown", function (ev) {
      if (/^(Shift|Meta|Alt|Control)/.test(ev.code)) return;   // the modifier keys that the host presses before a key are no key of the stage
      try { setMsg(""); } catch (e) { /* no page */ }
      var rec = { n: keys.length, code: ev.code, key: ev.key, meta: !!ev.metaKey, alt: !!ev.altKey, shift: !!ev.shiftKey, ctrl: !!ev.ctrlKey,
        field: document.activeElement === field, at: Date.now(), cls: K.classify(ev), prevented: null, msg: null,
        repeat: !!ev.repeat, undo0: S.undo.length, redo0: S.redo.length, undo: null, redo: null, value0: field.value, value: null };
      keys.push(rec);
      setTimeout(function () { rec.prevented = ev.defaultPrevented; }, 0);
      setTimeout(function () { rec.msg = msgText(); rec.undo = S.undo.length; rec.redo = S.redo.length; rec.value = field.value; }, 600);
      if (ev.code === "F13") field.focus();
      if (ev.code === "F14") field.blur();
      if (ev.code === "F12") setTimeout(report, 900);
    }, true);
    function report() {
      post({ kind: "hostkeys", visibility: document.visibilityState, hasFocus: document.hasFocus(), keys: keys, shown: K.shown,
        calls: tr.calls, field_value: field.value, host_mode: !!B.hostMode,
        groups: (function () { try { return HOST_ABSENT.map(function (g) { return { group: g.group, say: g.say }; }); } catch (e) { return []; } })() });
    }
    for (var i = 0; i < 150 && !(tr.delivered >= 1 && tr.offer); i++) await sleepMs(100);
    try { S.selTrack = S.order[0]; } catch (e) { /* no track: the key H adds no hit */ }   // the key H of the page adds a hit on the selected track
    post({ kind: "hostkeys-ready", delivered: tr.delivered, offer: tr.offer || null });
  }

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
    setTimeout(function () {   // a hidden page (locked screen) draws no frame: say so
      if (deltas.length < 30) { clearInterval(iv); post({ kind: "measure", visibility: document.visibilityState, frames: deltas.length, p95_ms: null, max_ms: null, canvases: cs.length, innerW: window.innerWidth, innerH: window.innerHeight }); }
    }, 14000);
    (function loop(t) {
      if (T.fault === "slow-frames") { var b = performance.now(); while (performance.now() - b < 50) { /* the fault run: a slow frame */ } }
      deltas.push(t - last); last = t;
      if (t - t0 < 8000) { requestAnimationFrame(loop); return; }
      clearInterval(iv);
      deltas = deltas.slice(20);
      var mx = 0; deltas.forEach(function (d) { if (d > mx) mx = d; });
      post({ kind: "measure", visibility: document.visibilityState, frames: deltas.length, p95_ms: p95(deltas), max_ms: mx, canvases: cs.length,
        innerW: window.innerWidth, innerH: window.innerHeight,
        host_playing: !!S.playing, host_pos: PH.v, transport_msgs: (window.__DS_TRANSPORT__ || {}).messages || 0 });   // M1b5: the page followed the host during the measure
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
  /* Mode transport (M1b5): the page's drawn position, a sample a frame, for T.arg seconds (default 11.5).
     The test host plays a script (play, stop, locate) and writes its own truth; the check compares
     the two on the wall clock (epoch ms). The position is what the frame loop would draw now:
     the extrapolated position while the page plays, else the playhead. */
  function transportRun() {
    var dur = (+T.arg || 11.5) * 1000, t0 = performance.now(), rows = [], sent = false;
    var tr = window.__DS_TRANSPORT__ || { messages: 0, delivered: 0, log: [] };
    function done(why) {
      if (sent) return;
      sent = true;
      var maxd = null; try { maxd = maxDur(); } catch (e) { /* none */ }
      var wired = false; try { wired = typeof B.hostTransport.onPosition === "function"; } catch (e) { /* none */ }
      var asked = null;   // the page asks the host to play: the plugin refuses (the host plays, not the page)
      try { asked = Promise.resolve(B.hostTransport.play(0)).then(function () { return "resolved"; }, function (e) { return "rejected: " + (e && e.message); }); } catch (e) { asked = Promise.resolve("threw"); }
      asked.then(function (play) {
        post({ kind: "transport", why: why, visibility: document.visibilityState, samples: rows, messages: tr.messages, delivered: tr.delivered,
          log: tr.log, first_at: tr.first_at, max_dur: maxd, wired: wired, host_mode: !!B.hostMode, playing: !!S.playing, pos: PH.v, play: play });
      });
    }
    setTimeout(function () { done("timer"); }, dur + 4000);   // a hidden page draws no frame
    if (T.fault === "page-transport") {   // the fault run of the no-own-audio line: the page uses the app's transport
      setTimeout(function () { window.__TAURI__.core.invoke("transport_status", {}).catch(function () { /* refused */ }); }, 1500);
    }
    (function frame(t) {
      var pos = null;
      try { pos = S.playing ? appPos(t) : PH.v; } catch (e) { /* no state */ }
      rows.push([Math.round((performance.timeOrigin + t) * 10) / 10, pos === null ? null : Math.round(pos * 1e4) / 1e4, S.playing ? 1 : 0]);
      if (t - t0 < dur) requestAnimationFrame(frame); else done("frames");
    })(performance.now());
  }
  /* Mode playback (M1b7): the page asks the host to move. The driver does what a user does: a click in
     the ruler (S.playhead = t, as rulerDown does), the Play button (play()), the Stop button (pause()).
     The test host has a playback controller (--playback full|ignore|partial, or none) and a transport
     that follows it (--transport follow, --snap 0.5: a position request lands on the grid, so the
     page's own click and the host's answer differ and the check can tell them apart). The page's
     position is sampled once a frame, with the wall clock (epoch ms). The report carries the samples,
     the steps (what the driver did, when, and what the page showed after) and the calls and messages
     that the shell script noted. The check compares them with the record of the host. */
  async function playbackRun() {
    var scenario = T.arg || "full";
    var wait = scenario === "ignore" ? 1800 : 750;
    var rows = [], steps = [], stopped = false;
    function sleepMs(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
    var tr = window.__DS_TRANSPORT__ || { messages: 0, delivered: 0, log: [], seen: [], calls: [] };
    if (T.fault === "page-transport") {   // the fault run of the no-own-audio line: the page uses the app's transport
      setTimeout(function () { window.__TAURI__.core.invoke("transport_status", {}).catch(function () { /* refused */ }); }, 1500);
    }
    (function frame(t) {
      var pos = null;
      try { pos = S.playing ? appPos(t) : PH.v; } catch (e) { /* no state */ }
      rows.push([Math.round((performance.timeOrigin + t) * 10) / 10, pos === null ? null : Math.round(pos * 1e4) / 1e4, S.playing ? 1 : 0]);
      if (!stopped) requestAnimationFrame(frame);
    })(performance.now());
    function text(id) { var el = document.getElementById(id); return el ? String(el.textContent || "") : null; }
    async function step(name, fn, ms) {
      var st = { name: name, at: Date.now(), playing_before: !!S.playing, calls_before: tr.calls.length };
      try { await fn(); } catch (e) { st.threw = String(e && e.message || e); }
      await sleepMs(ms || wait);
      var btn = document.getElementById("playBtn"), ruler = document.getElementById("ruler");
      st.msg = text("stMsg"); st.playing_after = !!S.playing; st.pos_after = PH.v;
      st.play_disabled = !!(btn && btn.disabled); st.play_title = btn ? btn.title : null; st.ruler_title = ruler ? ruler.title : null;
      st.end = Date.now();
      steps.push(st);
    }
    // the first message of the host and the answer to the offer: the page is ready for a request
    for (var i = 0; i < 100 && !(tr.delivered >= 1 && tr.offer); i++) await sleepMs(100);
    var first = tr.delivered >= 1;
    if (scenario === "tracks") {   // M1b8: ONE DAW track holds two sources, clip 2 later in the song than clip 1; a second DAW track holds the third
      HT.fault = T.song || "";   // selected-track: the fault run maps the follow through the selected row (static/transport.js)
      var rowA = HT.rows[0], rowB = HT.rows[1], homeA = rowA && rowA.srcs[0], homeB = rowB && rowB.srcs[0];
      var inClip2 = rowA.clips[1].s + 1.0;   // inside the second clip of the first DAW track
      await step("select-b", function () { S.selTrack = homeB; }, 500);
      await step("locate-clip2", function () { S.playhead = inClip2; }, 600);
      await step("select-a", function () { S.selTrack = homeA; }, 700);
      await step("play", function () { return play(); }, 900);
      await step("select-b-playing", function () { S.selTrack = homeB; }, 500);
      await step("stop", function () { return pause(); }, 700);
      stopped = true;
      post({ kind: "playback", scenario: scenario, visibility: document.visibilityState, samples: rows, steps: steps, calls: tr.calls, seen: tr.seen,
        offer: tr.offer, first_message: first, tracks: [homeA, homeB], rows: HT.rows.map(function (x) { return { id: x.id, srcs: x.srcs, clips: x.clips }; }), in_clip2: inClip2,
        host_mode: !!B.hostMode, wired: true });
      return;
    }
    if (scenario === "drag") {   // M1b7 X3: a drag in the ruler with a host that does not answer: the fallback must not jump under the pointer
      var ruler = document.getElementById("ruler"), rc = ruler.getBoundingClientRect(), during = [];
      function ev(type, f) { return new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: rc.left + rc.width * f, clientY: rc.top + rc.height / 2 }); }
      ruler.dispatchEvent(ev("mousedown", 0.5));
      for (var k = 0; k < 15; k++) {   // 3 s of drag, a move every 200 ms (the fallback of the first locate is due at 1 s)
        window.dispatchEvent(ev("mousemove", 0.4 + 0.2 * ((k % 5) / 5)));
        for (var q = 0; q < 8; q++) { await sleepMs(25); during.push([Date.now(), PH.v]); }   // a sample every 25 ms: a jump back between two moves shows
      }
      var held = PH.v;
      window.dispatchEvent(ev("mouseup", 0.5));
      await sleepMs(1800);
      stopped = true;
      post({ kind: "playback", scenario: scenario, visibility: document.visibilityState, samples: rows, steps: [{ name: "drag", during: during, held: held, pos_after: PH.v, calls: tr.calls.length }], calls: tr.calls, seen: tr.seen,
        offer: tr.offer, first_message: first, host_mode: !!B.hostMode, wired: true });
      return;
    }
    if (scenario === "late") {   // M1b7 X4: the host starts 2.5 s after the request, after the page gave up
      await step("locate-stopped", function () { S.playhead = 1.2; }, 600);
      await step("play", function () { play(); }, 2300);   // not awaited: the snapshot is taken 2.3 s after the call
      var gaveUp = { msg: text("stMsg"), playing: !!S.playing };
      await sleepMs(3000);
      stopped = true;
      post({ kind: "playback", scenario: scenario, visibility: document.visibilityState, samples: rows, steps: steps, gave_up: gaveUp, final_playing: !!S.playing, final_msg: text("stMsg"),
        calls: tr.calls, seen: tr.seen, offer: tr.offer, first_message: first, host_mode: !!B.hostMode, wired: true });
      return;
    }
    await step("locate-stopped", function () { S.playhead = 1.2; }, wait);
    await step("play", function () { return play(); }, wait + 200);
    await step("locate-playing", function () { S.playhead = 0.6; }, wait + 200);
    await step("stop", function () { return pause(); }, wait);
    await step("locate-stopped-2", function () { S.playhead = 2.6; }, wait);
    if (scenario === "none") await step("stop-direct", function () { return B.hostTransport.stop(0); }, 400);   // the shell's stop, called as a script: no host control, no error
    var maxd = null; try { maxd = maxDur(); } catch (e) { /* none */ }
    stopped = true;
    post({ kind: "playback", scenario: scenario, visibility: document.visibilityState, first_message: first, samples: rows, steps: steps, calls: tr.calls, seen: tr.seen,
      offer: tr.offer, max_dur: maxd, host_mode: !!B.hostMode, wired: typeof B.hostTransport.onPosition === "function" });
  }
  async function edit(names, withRole) {   // move one hit through the page's own backend call
    var dirty = [];
    try { await window.__TAURI__.event.listen("project-dirty", function (e) { dirty.push(e.payload); }); } catch (e) { /* no events */ }
    var name = names[0], before = await raw("events_get", { sid: S.sid, name: name });
    var evs = JSON.parse(before.text).events, idx = Math.min(1, evs.length - 1), from = evs[idx].t, to = from + 0.01;
    var res = null, err = null, role = null, follower = null;
    try { res = await B.eventsEdit(S.sid, name, [{ op: "move", index: idx, t: to }]); } catch (e) { err = String(e && e.message || e); }
    if (withRole) { try { await window.__TAURI__.core.invoke("track_set_role", { sid: S.sid, name: name, role: "cymbal" }); role = "cymbal"; } catch (e) { err = String(e && e.message || e); } }
    if (withRole && names.length > 1) {   // R1: the second track follows the first one
      try { await window.__TAURI__.core.invoke("track_set_follow", { sid: S.sid, name: names[1], source: name }); follower = names[1]; } catch (e) { err = String(e && e.message || e); }
    }
    var after = await raw("events_get", { sid: S.sid, name: name });
    // the report goes at once: the host may store the archive the moment it sees it
    post({ kind: "edit", name: name, index: idx, from: from, to: to, error: err, role: role, follower: follower, before_raw: before.text, after_raw: after.text });
    await new Promise(function (r) { setTimeout(r, 1500); });   // the event of the service comes by long poll
    post({ kind: "edit-events", dirty_events: dirty });
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
    try { await window.__TAURI__.core.invoke("export_existing", { sid: sid }); panicked = "answered"; } catch (e) { panicked = { code: e.code, message: e.message }; }
    // a channel message: a read-only command with a progress channel is not needed; the event
    // listener is checked by registering one
    var listened = false;
    try { var un = await window.__TAURI__.event.listen("project-dirty", function () {}); listened = typeof un === "function"; } catch (e) { /* no */ }
    post({ kind: "shell", commands: out, binary: bin, error_shape: errc, unknown: nocmd, export_existing: panicked, listen: listened,
      keys: Object.keys(window.__TAURI__).sort(), core: Object.keys(window.__TAURI__.core).sort(), hostMode: window.__DS_HOST_MODE__ });
  }
  async function follow(expect, needReload, reloadsFrom) {   // wait for the plugin's reload event: the page holds the new tracks
    var t0 = performance.now(), ok = false, reloads0 = reloadsFrom === undefined ? reloads : reloadsFrom;
    while (performance.now() - t0 < 30000) {
      try { if (S.order.length >= expect && S.loaded > 0 && S.sid && (!needReload || reloads > reloads0)) { ok = true; break; } } catch (e) { /* not yet */ }
      await new Promise(function (r) { setTimeout(r, 200); });
    }
    await new Promise(function (r) { setTimeout(r, 500); });
    var names = [], page = {}, rawEv = {};
    try { names = S.order.slice(); names.forEach(function (n) { page[n] = (S.events[n] || []).map(function (e) { return { t: e.t, v: e.v }; }); }); } catch (e) { /* none */ }
    for (var i = 0; i < names.length; i++) rawEv[names[i]] = (await raw("events_get", { sid: S.sid, name: names[i] })).text;
    var roles = {}, follows = {};
    try { names.forEach(function (n) { roles[n] = S.tracks[n] && S.tracks[n].role; follows[n] = S.tracks[n] && S.tracks[n].follows; }); } catch (e) { /* none */ }
    post({ kind: "follow", ok: ok, roles: roles, follows: follows, reloads: reloads, tracks: names, page_events: page, raw_events: rawEv, sid: S.sid, ms: Math.round(performance.now() - t0) });
  }
  /* M1b10: what the page gets as the names of the tracks. `names`: the answer of tracks_get (the key
     and display_name of every track) and the raw events; the host then renames a DAW track and moves
     a region; `names-after`: the same after the plugin's reload event, with no new analysis round. */
  async function namesSnap() {
    var o = JSON.parse((await raw("tracks_get", { sid: S.sid })).text);
    var keys = Object.keys(o.tracks || {}), display = {}, rawEv = {};
    var roles = {};
    keys.forEach(function (k) { display[k] = o.tracks[k].display_name; roles[k] = { role: o.tracks[k].role, note: o.tracks[k].note, role_source: o.tracks[k].role_source }; });
    for (var i = 0; i < keys.length; i++) rawEv[keys[i]] = (await raw("events_get", { sid: S.sid, name: keys[i] })).text;
    var dom = Array.prototype.slice.call(document.querySelectorAll("#lanesScroll .tnm")).map(function (e) { return e.textContent; });
    return { order: o.order, display: display, roles: roles, raw_events: rawEv, dom_names: dom };
  }
  async function namesRun() {
    function sleepMs(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
    var mode = T.arg || "";   // roles: the roles of the tracks follow the DAW names; roles-hand: the user sets the role of the first track first
    var first = await namesSnap();
    var auto = first.roles;
    if (mode === "roles-hand") {
      await window.__TAURI__.core.invoke("track_set_role", { sid: S.sid, name: first.order[0], role: "cymbal" });
      first = await namesSnap();
    }
    if (mode === "settings") {   // N10: the user sets a role, a note and a volume by hand
      var inv = window.__TAURI__.core.invoke;
      await inv("track_set_role", { sid: S.sid, name: first.order[0], role: "cymbal", note: 60 });
      await inv("track_set_mix", { sid: S.sid, name: first.order[0], volume_db: -3 });
      first = await namesSnap();
    }
    first.roles_auto = auto;
    var base = reloads, t0 = performance.now();
    post(Object.assign({ kind: "names", reloads: reloads }, first));
    var limit = (T.arg === "base" || T.arg === "settings") ? 1500 : 15000;   // the baseline run renames nothing: no reload to wait for
    while (reloads === base && performance.now() - t0 < limit) await sleepMs(100);
    var reloaded = reloads > base;
    if (mode.indexOf("roles") === 0) {   // a new role is a new round: its end is a second reload
      var t1 = performance.now();
      while (reloads < base + 2 && performance.now() - t1 < 12000) await sleepMs(100);
      await sleepMs(1000);
    }
    await sleepMs(800);   // the rename and the move are two calls of the host: the last state
    var after = await namesSnap();
    post(Object.assign({ kind: "names-after", reloads: reloads, reloaded: reloaded, waited_ms: Math.round(performance.now() - t0) }, after));
  }
  /* Mode song (M1b8, #555): the rows of the page in song time. The first report `song` carries what the plugin answers to `plugin_host_regions` and what
     the page DRAWS for those rows (the pixels of the hit lines in the lanes, the rows shown, the headers); the host then renames a DAW track, moves a
     region to another one and shifts and trims others (`--rename-seq`, `--move-region`, `--shift-region`, `--trim-region`); the second report `song-after`
     carries the answer again, the events `plugin-regions` the page got and what the page draws then. Fault runs (DS_ARA_SONG_FAULT, made here in the page):
     `source-x`, `no-cut-start`, `home-only`: the driver sets the flag HT.fault, which the page code (static/hosttime.js) reads. */
  function songFaults() { HT.fault = T.song || ""; }   // the fault runs are in the page (static/hosttime.js reads the flag); the driver only sets it
  async function songDraw(answer, upto) {   // what the page draws for the rows the plugin gave: one entry for each hit that the clips hold, and each one outside them
    setView(0, upto); invalidate(F.VIEW); flushFrame();
    for (var w0 = 0; w0 < 50 && (FR.raf || FR.flags !== 0); w0++) await new Promise(function (r) { setTimeout(r, 100); });   // the frame that draws the view is done
    var W = laneWidth(), dpr = GEO.dpr, rows = {};
    answer.rows.forEach(function (r) {
      var home = r.srcs[0], cv = document.querySelector('.track[data-name="' + CSS.escape(home) + '"] canvas.spike'), cx = cv.getContext("2d"), hits = [], outside = [];
      function drawn(song) { var x = Math.round(tToX(song, W)) + 0.5, d = cx.getImageData(Math.floor(x * dpr), Math.floor(cv.height / 2), 1, 1).data; return d[3] > 100; }
      var seen = {};
      r.clips.forEach(function (c) {
        (S.events[c.src] || []).forEach(function (e) {
          var song = c.s + (e.t - c.a) * c.k;   // the formula of the export, written here and not taken from the page
          if (e.t >= c.a && e.t < c.b) hits.push({ src: c.src, t: e.t, song: song, drawn: drawn(song) });
          else outside.push({ src: c.src, t: e.t, song: song, drawn: drawn(song) });
        });
      });
      rows[r.id] = { home: home, hits: hits, outside: outside };
    });
    var shown = [].slice.call(document.querySelectorAll(".track")).filter(function (r) { return getComputedStyle(r).display !== "none"; });
    return { rows: rows, shown: shown.map(function (r) { return r.dataset.name; }), headers: shown.map(function (r) { return r.querySelector(".tnm").textContent; }),
      merged: [].slice.call(document.querySelectorAll(".track.hostMerged")).map(function (r) { return r.dataset.name; }), max_dur: maxDur() };
  }
  async function songSnap(upto) {
    var rows = await window.__TAURI__.core.invoke("plugin_host_regions", {});
    return { rows: rows, events: regEvents, last_event: regLast, order: S.order.slice(), sel: S.selTrack, page: await songDraw(rows, upto) };
  }
  async function songRun() {
    function sleepM(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
    songFaults();
    await sleepM(600);
    var first = await songSnap(8);
    post(Object.assign({ kind: "song" }, first));
    if (T.arg === "edit" || T.arg === "base") {   // the edit run: a double click in the second clip of the first DAW track, then one in the gap; `base`: no edit (the notes to compare with)
      var row0 = first.rows.rows[0], home = row0.srcs[0], c1 = row0.clips[1], gapT = (row0.clips[0].s + (row0.clips[0].b - row0.clips[0].a) + c1.s) / 2, out = {};
      function dbl(t) {
        var cv = document.querySelector('.track[data-name="' + CSS.escape(home) + '"] canvas.spike');
        cv.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0, clientX: GEO.laneX + tToX(t, laneWidth()), clientY: cv.getBoundingClientRect().top + 8 }));
      }
      var n0 = (S.events[c1.src] || []).length, nh = (S.events[home] || []).length;
      if (T.arg === "edit") {
        dbl(c1.s + 0.6);   // 0.6 s into the second clip: source time c1.a + 0.6
        for (var w = 0; w < 50 && (S.events[c1.src] || []).length === n0; w++) await sleepM(100);
        out.added_at = (S.events[c1.src] || []).map(function (e) { return e.t; });
        out.want_src_t = c1.a + 0.6; out.want_song_t = c1.s + 0.6;
        for (var w1 = 0; w1 < 30 && (FR.raf || FR.flags !== 0); w1++) await sleepM(100);
        dbl(gapT);
        for (var w2 = 0; w2 < 30 && !/outside every region/.test((document.getElementById("stMsg") || {}).textContent || ""); w2++) await sleepM(100);
        out.gap_msg = (document.getElementById("stMsg") || {}).textContent;
      }
      out.n_before = n0; out.n_after = (S.events[c1.src] || []).length; out.home_before = nh; out.home_after = (S.events[home] || []).length; out.src = c1.src;
      await sleepM(4500);   // the plugin takes the edit into its notes: no condition that the page can read (the host waits for the row page_sync as well)
      post(Object.assign({ kind: "song-after", edit: out }, await songSnap(8)));
      return;
    }
    var base = regEvents, t0 = performance.now();
    while (regEvents === base && performance.now() - t0 < 15000) await sleepM(100);
    await sleepM(1000);   // the host makes its changes in several calls: the last state
    var after = await songSnap(22);
    post(Object.assign({ kind: "song-after", waited_ms: Math.round(performance.now() - t0) }, after));
  }
  /* Mode deliver (M1c3, #576): the files that the plugin prepares by itself, and the handles of the page. The first report `deliver` holds: the rows of
     the plugin (plugin_host_regions) and the stored hits of each source, the states after the first round (MIDI is made at once, the wav kinds say
     "press to prepare"), the handles in the DOM, the paths of a take of the MIDI and the merged file, the refused take of a wav kind that is off,
     the first use of both wav kinds (time to ready), one take of everything and the drag of it through the page (a private pasteboard in a gate run),
     then an edit of one hit (the take at once: stale, a 409; the wait; the new set) and the counters of the worker. Fault runs: DS_ARA_PREPARE_FAULT. */
  async function deliverRun() {
    var out = { steps: [] };
    function inv(c, a) { return window.__TAURI__.core.invoke(c, a || {}).then(function (v) { return { ok: true, v: v }; }, function (e) { return { ok: false, code: e && e.status, msg: String(e && (e.message || e)) }; }); }
    function sleepD(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
    async function until(fn, ms) { var t0 = performance.now(); for (;;) { var v = false; try { v = await fn(); } catch (e) { /* not yet */ } if (v) return Math.round(performance.now() - t0); if (performance.now() - t0 > ms) return -1; await sleepD(100); } }
    async function state() { var r = await inv("plugin_deliver_state"); return r.ok ? r.v : null; }
    function allIn(st, kind, want) { return st && st.rows.length > 0 && st.rows.every(function (r) { return r[kind].state === want; }); }
    function dom() {
      function hs(sel) { return Array.prototype.map.call(document.querySelectorAll(sel + " .dlvH"), function (h) { return { kind: h.dataset.kind, state: h.dataset.state, text: h.querySelector(".dlvT").textContent }; }); }
      var rows = {};
      Array.prototype.forEach.call(document.querySelectorAll("#lanesScroll .track"), function (el) { var s = el.querySelector(".tdel"); if (s) rows[el.dataset.name] = hs('#lanesScroll .track[data-name="' + CSS.escape(el.dataset.name) + '"] .tdel'); });
      return { rows: rows, bar: hs("#dlvBar"), hint: (document.getElementById("dlvHint") || {}).textContent };
    }
    await until(function () { return S.order.length > 0 && S.order.every(function (n) { return (S.events[n] || []).length > 0; }); }, 40000);
    await settle();
    var regions = await inv("plugin_host_regions"); out.regions = regions.v;
    out.events = {};
    for (var i = 0; i < S.order.length; i++) { var g = await inv("events_get", { sid: S.sid, name: S.order[i] }); out.events[S.order[i]] = g.v ? g.v.events : null; }
    out.t_midi = await until(async function () { return allIn(await state(), "midi", "ready"); }, 40000);
    var st = await state(); out.state1 = st;
    await sleepD(300); out.dom1 = dom();
    if (out.t_midi < 0) { post(Object.assign({ kind: "deliver" }, out)); return; }   // the watchdog run: nothing else to do
    var mt = await inv("plugin_deliver_take", { items: st.rows.map(function (r) { return { row: r.id, kind: "midi" }; }) }); out.take_midi = mt;
    var mg = await inv("plugin_deliver_take", { items: [{ row: "", kind: "merged" }] }); out.take_merged = mg;
    if (T.arg === "short") { post(Object.assign({ kind: "deliver" }, out)); return; }   // the hostile-name run: the MIDI files are enough
    if (T.arg === "late") {   // round 3 (P1): a row "Snare_audio" shows up after the first take (scenario late-row); its spike file has the name of the audio file of "Snare"
      var first = st.rows[0].id;
      await inv("plugin_deliver_want", { kind: "spike" }); await inv("plugin_deliver_want", { kind: "audio" });
      out.t_late = await until(async function () { var s1 = await state(); return s1 && s1.rows.length === 1 && s1.rows[0].spike.state === "ready" && s1.rows[0].audio.state === "ready"; }, 60000);
      out.late1 = await inv("plugin_deliver_take", { items: [{ row: first, kind: "audio" }] });
      out.t_late2 = await until(async function () { var s2 = await state(); return s2 && s2.rows.length === 2 && s2.rows.every(function (r) { return ["midi", "spike", "audio"].every(function (k) { return r[k].state === "ready"; }); }); }, 60000);
      var s3 = await state(); var second = s3.rows.filter(function (r) { return r.id !== first; })[0];
      out.late_rows = s3.rows.map(function (r) { return r.name; });
      out.late2 = await inv("plugin_deliver_take", { items: [{ row: first, kind: "audio" }, { row: second ? second.id : "none", kind: "spike" }] });
      post(Object.assign({ kind: "deliver" }, out)); return;
    }
    out.take_spike_off = await inv("plugin_deliver_take", { items: [{ row: st.rows[0].id, kind: "spike" }] });
    out.want_spike = await inv("plugin_deliver_want", { kind: "spike" });
    out.t_spike = await until(async function () { return allIn(await state(), "spike", "ready"); }, 60000);
    out.want_audio = await inv("plugin_deliver_want", { kind: "audio" });
    out.t_audio = await until(async function () { return allIn(await state(), "audio", "ready"); }, 90000);
    var st2 = await state(); out.state2 = st2; await sleepD(300); out.dom2 = dom();
    var items = []; st2.rows.forEach(function (r) { ["midi", "spike", "audio"].forEach(function (k) { items.push({ row: r.id, kind: k }); }); });
    var all = await inv("plugin_deliver_take", { items: items }); out.take_all = all;
    if (all.ok) { try { await B.dragFiles(all.v.files); out.drag = "ok"; } catch (e) { out.drag = String(e && e.message || e); } }
    out.count_before = (await state()).counters;
    if (T.arg === "noedit") { out.dom3 = dom(); post(Object.assign({ kind: "deliver" }, out)); return; }   // the run that reads the files as they were made
    // an edit of one hit of the first source of the first row, taken at once (the set is stale) and later (the set is new)
    var home = st2.rows[0].srcs[0], clip = regions.v.rows[0].clips.filter(function (c) { return c.src === home; })[0];
    // three edits 400 ms apart: the preparation waits for the quiet time, so they make ONE set of files (a pass for each would be the fault no-debounce)
    var adds = [0.9, 1.0, 1.1], eds = [];
    for (var ai = 0; ai < adds.length; ai++) {
      var ed1 = await inv("events_edit", { sid: S.sid, name: home, ops: [{ op: "add", t: clip.a + adds[ai], v: 100 }] });
      eds.push(ed1.ok);
      if (ai < adds.length - 1) await sleepD(400);
    }
    out.edit = { ok: eds.every(Boolean), src: home, songs: adds.map(function (x) { return clip.s + x; }) };
    out.take_stale = await inv("plugin_deliver_take", { items: [{ row: st2.rows[0].id, kind: "midi" }] });
    out.t_after = await until(async function () { var s = await state(); return allIn(s, "midi", "ready") && allIn(s, "spike", "ready") && allIn(s, "audio", "ready") && s.merged.state === "ready" && s.counters.jobs > out.count_before.jobs; }, 60000);
    await sleepD(500);
    out.count_after = (await state()).counters;
    var st3 = await state(); out.state3 = st3;
    out.take_after = await inv("plugin_deliver_take", { items: items });
    out.take_merged_after = await inv("plugin_deliver_take", { items: [{ row: "", kind: "merged" }] });
    out.dom3 = dom();
    post(Object.assign({ kind: "deliver" }, out));
  }
  /* Mode firstopen (M1d4, #578): the page at the first open. The driver samples what the page shows (the line over the lanes, the strips and cards of the
     rows, the text of the lanes) and what the plugin answers to plugin_host_states every 100 ms and reports the timeline in ONE report `firstopen`.
     The argument (T.arg) names the scenario: `flow` (until the document is done), `limit` (a read with no progress turns failed after the limit; Retry is refused while the read still runs; the read ends and the real state comes back), `stale` (a failed row that keeps old hits; the user edits it, Retry brings the new audio), `offline` (the first row that is offline gets Retry twice, fast; the
     host's late access then brings the real state back) and `failed` (the same for a row that failed; the events of every track are read before and
     after the press, and one track is edited before it). Fault runs: DS_ARA_EDITOR_FAULT=first-announce (the line is written at every poll). */
  async function firstopenRun() {
    if (T.fault === "first-announce") HST.fault = "first-announce";
    var inv = window.__TAURI__.core.invoke, mode = T.arg || "flow", t0 = performance.now(), timeline = [], pressed = null, answers = null;
    var before = null, after = null, edited = null, seenAttention = false, pressAt = 0, attentionAt = 0, answers2 = null;
    function dom() {
      var line = document.getElementById("hstLine"), rows = [].map.call(document.querySelectorAll("#lanesScroll .hostState, #lanesScroll .hostCard"), function (e) {
        return { row: e.dataset.row, state: e.dataset.state, text: e.querySelector(".hstRowText").textContent, retry: !!e.querySelector(".hstRetry") };
      });
      return { line: line ? line.querySelector(".hstText").textContent : null, lstate: line ? line.dataset.state : null, retry_all: !!(line && line.querySelector(".hstRetryAll")),
        rows: rows, lanes: String(document.getElementById("lanesScroll").innerText || "").slice(0, 400), tracks: (S.order || []).length, writes: HST.writes,
        line_writes: HST.lineWrites, hits: (S.order || []).reduce(function (a, n) { return a + (S.events[n] || []).length; }, 0),
        events_n: (S.order || []).reduce(function (o, n) { o[n] = (S.events[n] || []).length; return o; }, {}),
        handles: DLV && DLV.st ? DLV.st.rows.map(function (r) { return { id: r.id, midi: r.midi.state }; }) : null };
    }
    async function eventsOf() {
      var out = {};
      for (var i = 0; i < (S.order || []).length; i++) { try { out[S.order[i]] = (await raw("events_get", { sid: S.sid, name: S.order[i] })).text; } catch (e) { out[S.order[i]] = null; } }
      return out;
    }
    var limit = mode === "flow" ? 60000 : 45000, finished = false;
    while (performance.now() - t0 < limit && !finished) {
      var st = null;
      try { st = await inv("plugin_host_states", {}); } catch (e) { st = { error: String(e && e.message || e) }; }
      var d = dom();
      timeline.push(Object.assign({ t: Math.round(performance.now() - t0), plugin: st && st.doc ? { state: st.doc.state, text: st.doc.text, rows: st.rows.map(function (r) { return { id: r.id, state: r.state, name: r.name, retry: r.retry, hits: r.hits }; }) } : st }, d));
      // the press waits until the PAGE shows the row with its Retry button: the plugin may be ahead of the page by a poll
      var need = st && st.rows ? st.rows.filter(function (r) { return r.retry && d.rows.some(function (x) { return x.row === r.id && x.retry; }); }) : [];
      if (need.length && !attentionAt) attentionAt = performance.now();
      // `stale`: the row stays failed for a while before the press, so the files of the drag handles are made (they must stay not ready)
      if (mode !== "flow" && need.length && !pressed && (mode !== "stale" || performance.now() - attentionAt > 7000)) {
        seenAttention = true;
        if (mode === "failed" || mode === "stale") {
          try { before = await eventsOf(); } catch (e) { before = null; }
          // one track is edited by the user before the press: the edit must survive a failure and the retry
          try {
            var done0 = S.order.find(function (n) { return (mode === "stale" ? n === (need[0].srcs || [])[0] : n !== (need[0].srcs || [])[0]) && (S.events[n] || []).length > 1; });
            if (done0) {
              var evs = JSON.parse((await raw("events_get", { sid: S.sid, name: done0 })).text).events;
              await B.eventsEdit(S.sid, done0, [{ op: "move", index: 1, t: evs[1].t + 0.01 }]);
              edited = { name: done0, from: evs[1].t, to: evs[1].t + 0.01 };
              before = await eventsOf();
            }
          } catch (e) { edited = { error: String(e && e.message || e) }; }
        }
        pressAt = performance.now() - t0;
        // two presses in one go: the second must start no second job
        var a = inv("plugin_host_retry", { rows: [need[0].id] }), b = inv("plugin_host_retry", { rows: [need[0].id] });
        answers = [await a, await b]; pressed = need[0].id;
      }
      // the document is done AND the page has loaded the project (its tracks and their hits): the plugin is ahead of the page by a reload
      // `offline`: after the wait limit the row is offline again with Retry: a second press (M1) must start a job again
      if (mode === "offline" && pressed && !answers2 && performance.now() - t0 - pressAt > 3600 && need.some(function (r) { return r.id === pressed; })) {
        answers2 = [await inv("plugin_host_retry", { rows: [pressed] })];
      }
      if (mode === "flow" && st && st.doc && st.doc.state === "done" && d.tracks >= st.rows.length && d.hits > 0 && d.line === st.doc.text) finished = true;
      if ((mode === "offline" || mode === "stale" || mode === "limit") && pressed && st && st.doc && st.doc.state === "done") finished = true;
      if (mode === "failed" && pressed && performance.now() - t0 - pressAt > 9000) finished = true;
      if (!finished) await new Promise(function (r) { setTimeout(r, 100); });
    }
    if (mode === "failed" || mode === "stale") { try { after = await eventsOf(); } catch (e) { after = null; } }
    post({ kind: "firstopen", mode: mode, timeline: timeline, pressed: pressed, press_at: Math.round(pressAt), answers: answers, answers2: answers2, seen_attention: seenAttention,
      before: before, after: after, edited: edited, order: S.order || [], writes: HST.writes, finished: finished });
  }
  /* Mode leftovers (M1d1, #575): the page shows the name of the DAW track where it names a track (progress, info line, status bar, undo label,
     follower text, export dialog, Follow menu), and after the host changed the audio of one source it says what happened. The first report
     `leftovers` carries the texts; the host then replaces the audio of a source (`--update-content-after`); the plugin analyses it again and
     tells the page; the second report `leftovers-after` carries the status line that the page shows. Fault runs (DS_ARA_EDITOR_FAULT):
     `left-key` (the label helper returns the key), `left-dot` (the unsaved dot shows), `left-dest` (the destination row is not collapsed). The plugin fault `DS_ARA_LEFTOVER_FAULT=no-audio-reason` sends no audio reason. */
  async function leftoversRun() {
    if (T.fault === "left-key") window.trackLabel = function (k) { return k; };   // the fault run: the page shows the key of a track
    if (T.fault === "left-dot") window.setDirtyState = function (d) { S.dirty = !!d; document.getElementById("dirtyDot").classList.toggle("hidden", !S.dirty); };   // the fault run: the standalone dot shows in a DAW
    if (T.fault === "left-dest") window.exDestFold = function () {};   // the fault run: the destination row is never collapsed
    var order = S.order.slice(), text = function (id) { return document.querySelector(id).textContent; };
    await sleepM(600);
    var a = order[0], b = order[2], out = { order: order, a: a, b: b };
    var real = B.job, jobs = {};
    jobs[a] = { state: "done" }; jobs[b] = { state: "running", stage: "detecting" };
    B.job = async function () { return { state: "running", tracks: jobs }; };
    pollJob(); await sleepM(400); clearTimeout(pollTimer); B.job = real;
    out.progress = text("#progressText"); hideProgress();
    S.selTrack = b; S.sel = { track: b, index: 0 }; updateStatus(); drawInfo();
    out.info = text("#ilTrack"); out.sel = text("#stSel");
    S.sel = null; updateStatus();
    pushUndo({ type: "del", track: b, t: 1.5 }); out.undo = undoTitles().undoTitle; S.undo.pop();
    S.tracks[a].follows = b; followerLocked(a); out.follower = text("#stMsg"); delete S.tracks[a].follows;
    setMsg(b + ": 12 hits"); out.status = text("#stMsg");
    openExport("selected", b);
    for (var w = 0; w < 50 && document.querySelector("#exportModal").classList.contains("hidden"); w++) await sleepM(100);
    out.export_sel = text("#exSelText"); out.export_note = text("#exHostNote");
    out.dest_visible = !!document.querySelector("#exDest").offsetParent;
    exCancel();
    out.follow_menu = DSHostMenu.model().flatMap(function (g) { return g.items; }).filter(function (i) { return i.id === "follow"; }).map(function (i) { return i.title; });
    out.header = Array.prototype.slice.call(document.querySelectorAll("#lanesScroll .tnm")).map(function (e) { return e.textContent; });
    out.dirty_dot_hidden = (setDirtyState(true), document.getElementById("dirtyDot").classList.contains("hidden")); setDirtyState(false);
    setMsg("");
    post(Object.assign({ kind: "leftovers", reloads: reloads }, out));
    var base = reloads, t0 = performance.now(), msg = "";
    while (performance.now() - t0 < 30000) {
      msg = text("#stMsg");
      if (/^The DAW changed the audio/.test(msg)) break;
      await sleepM(200);
    }
    post({ kind: "leftovers-after", reloads: reloads, reloaded: reloads > base, waited_ms: Math.round(performance.now() - t0), msg: msg,
      header: Array.prototype.slice.call(document.querySelectorAll("#lanesScroll .tnm")).map(function (e) { return e.textContent; }) });
  }
  /* Mode delivery (M1c1, #560): the export in song time, the folder rule and the file drag, through the page's own invoke.
     DS_ARA_PICK_DIR answers the folder sheet; DS_ARA_DRAG_PRESSED stands in for the mouse button; a gate run starts NO system drag session (the plugin writes the items to a private pasteboard). The report carries what each
     call answered and the hits of every track, so the check reads the files and compares them with the hits. */
  async function deliveryRun(names) {
    var inv = window.__TAURI__.core.invoke, sid = S.sid, out = { names: names };
    function err(e) { return { code: e && e.code, message: e && e.message }; }
    var png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    // the host may move a region in playback time now (no new round): the export must follow it
    out.events_before = {};
    for (var b = 0; b < names.length; b++) out.events_before[names[b]] = JSON.parse((await raw("events_get", { sid: sid, name: names[b] })).text).events;
    post({ kind: "delivery-ready" });
    await new Promise(function (r) { setTimeout(r, 2500); });
    out.events = {};    for (var i = 0; i < names.length; i++) out.events[names[i]] = JSON.parse((await raw("events_get", { sid: sid, name: names[i] })).text).events;
    out.tempo = await inv("plugin_host_tempo", {});
    out.display = {};
    var tg = JSON.parse((await raw("tracks_get", { sid: sid })).text);
    names.forEach(function (n) { out.display[n] = tg.tracks[n].display_name; });
    // before any choice a dest outside the document folder is refused
    try { await inv("export_all", { sid: sid, dest: T.arg || "/tmp" }); out.before_choice = { ok: true }; } catch (e) { out.before_choice = err(e); }
    out.folder = await window.__TAURI__.dialog.open({ directory: true, title: "Choose the export folder" });
    if (typeof out.folder === "string") {
      try { out.exported = await inv("export_all", { sid: sid, dest: out.folder, formats: ["midi", "spikes"], compensate: false }); } catch (e) { out.exported = { error: err(e) }; }
      try { out.exported_single = await inv("export_all", { sid: sid, dest: out.folder, formats: ["midi"], midi_mode: "single", compensate: false }); } catch (e) { out.exported_single = { error: err(e) }; }
      // M1c2 (#579): the audio spikes, in song time like the synthetic ones
      try { out.exported_audio = await inv("export_all", { sid: sid, dest: out.folder, formats: ["audio"], compensate: false }); } catch (e) { out.exported_audio = { error: err(e) }; }
      try { await inv("export_all", { sid: sid, dest: out.folder + "/sub", formats: ["midi"] }); out.subfolder = { ok: true }; } catch (e) { out.subfolder = err(e); }
      try { await inv("export_all", { sid: sid, dest: T.arg || "/tmp", formats: ["midi"] }); out.other = { ok: true }; } catch (e) { out.other = err(e); }
    }
    try {
      var prep = await inv("export_drag_prepare", { sid: sid, formats: ["midi", "spikes"], compensate: false });
      out.prepared = prep;
      out.taken = await inv("export_drag_take", { sid: sid, paths: prep.midi.concat(prep.spikes) });
      try { out.drag = await inv("plugin:drag|start_drag", { item: out.taken.files, image: png, onEvent: null }); out.drag_ok = true; } catch (e) { out.drag_ok = false; out.drag_error = err(e); }
      try { await inv("plugin:drag|start_drag", { item: ["/etc/hosts"], image: png, onEvent: null }); out.drag_outside = { ok: true }; } catch (e) { out.drag_outside = err(e); }
    } catch (e) { out.prepare_error = err(e); }
    post(Object.assign({ kind: "delivery" }, out));
  }
  /* Mode exportdialog (M1c2, #579): the REAL Export dialog in the REAL page of the plugin, opened from the host-mode menu
     (File > Export...) by DOM clicks, as a user does it. The report holds what the dialog shows at each step (the folder text,
     the replace line, the tiles and their count, the result line); the check reads the files that the Export button wrote.
     Steps: open, drag a tile, Export, close, open again, Export, Choose a folder (DS_ARA_PICK_DIR answers the sheet; T.arg is
     that folder), Export, close. `--cycles 2` runs it in a second editor of the same document: the folder of the choice
     must then be the one the dialog opens with. DS_ARA_DRAG_PRESSED stands in for the mouse button of the drag; no system drag session starts in a gate run. */
  async function exportDialogRun(names) {
    var out = { names: names, def: null, steps: [] };
    function q(sel) { return document.querySelector(sel); }
    function txt(sel) { var e = q(sel); return e ? String(e.textContent || "") : null; }
    function shown(sel) { var e = q(sel); return !!e && !e.classList.contains("hidden"); }
    function mouse(el, type, dx) { var r = el.getBoundingClientRect(); el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: r.left + r.width / 2 + (dx || 0), clientY: r.top + r.height / 2, detail: 1 })); }
    async function until(fn, ms) { var t0 = performance.now(); for (;;) { var v = false; try { v = fn(); } catch (e) { /* not yet */ } if (v) return true; if (performance.now() - t0 > ms) return false; await sleepM(80); } }
    function tile(id) { var t = q(id); return t ? { state: t.dataset.state, text: String(t.querySelector(".exDragN").textContent || ""), hidden: t.classList.contains("hidden") } : null; }
    function snap(label) {
      var r = { label: label, open: shown("#exportModal") === true && !q("#exportModal").classList.contains("hidden"), dest: q("#exDest") ? q("#exDest").value : null,
        problem: txt("#exProblem"), replace: txt("#exReplace"), go_disabled: q("#exGo") ? q("#exGo").disabled : null, note: txt("#exHostNote"),
        result: shown("#exResult") ? txt("#exResultText") : null, midi: tile("#exDragMidi"), spikes: tile("#exDragSpikes"),
        dest_ok: EX.destOk, def: EX.def, msg: txt("#stMsg"), spike_audio_visible: !!q("#exSpikeAudio") && q("#exSpikeAudio").closest("label").checkVisibility({ visibilityProperty: true }),
        spike_both_visible: !!q("#exSpikeBoth") && q("#exSpikeBoth").closest("label").checkVisibility({ visibilityProperty: true }) };
      out.steps.push(r);
      return r;
    }
    function tilesDone() { var a = tile("#exDragMidi"), b = tile("#exDragSpikes"); return a && b && a.state !== "preparing" && b.state !== "preparing"; }
    var btn = document.getElementById("hostMenuBtn"), panel = document.getElementById("hostMenu");
    if (!btn || !panel) { out.error = "no menu button"; post(Object.assign({ kind: "exportdialog" }, out)); return; }
    async function openFromMenu() {   // File > Export... with clicks, as a user does
      mouse(btn, "mousedown"); mouse(btn, "mouseup"); btn.click();
      if (!(await until(function () { return DSHostMenu.isOpen(); }, 3000))) return "the click did not open the menu";
      var g = panel.querySelector('[data-group="File"]'); if (!g) return "no File group";
      g.click(); await sleepM(100);
      var it = panel.querySelector('[data-cmd="export"]'); if (!it) return "no item export";
      mouse(it, "mousedown"); mouse(it, "mouseup"); it.click();
      if (!(await until(function () { return !q("#exportModal").classList.contains("hidden"); }, 10000))) return "the dialog did not open";
      return "ok";
    }
    async function answerNotice() {   // the host gave no tempo: the dialog asks once, per document
      var go = await until(function () { return !q("#confirmModal").classList.contains("hidden"); }, 1500);
      if (!go) return false;
      var b = q('#confirmActions button[data-act="anyway"]'); if (b) b.click();
      return true;
    }
    async function exportNow(label) {
      var n0 = EX.done;
      if (q("#exGo").disabled) { var r0 = snap(label); r0.export_finished = false; r0.disabled = true; return r0; }   // the dialog refuses the export: nothing to wait for
      q("#exGo").click();
      out["notice_" + label] = await answerNotice();
      var ok = await until(function () { return EX.done > n0 && !EX.busy; }, 60000);
      await sleepM(400);
      var r = snap(label); r.export_finished = ok; return r;
    }
    async function close() {
      q("#exCancel").click();
      await until(function () { return q("#exportModal").classList.contains("hidden"); }, 5000);
      await sleepM(300);
    }
    // the tracks have their hits
    await until(function () { return names.every(function (n) { return (S.events[n] || []).length > 0; }); }, 30000);
    await settle();
    out.menu = await openFromMenu();
    await until(tilesDone, 40000);
    out.def = EX.def; out.remembered = EX.remembered;
    snap("open");
    // the drag of a tile: a press and a move on the tile, as a user does (the files move into Exports/, the native drag starts)
    var mt = q("#exDragMidi");
    if (mt && mt.dataset.state === "ready") {
      mouse(mt, "mousedown"); mouse(mt, "mousemove", 30); mouse(mt, "mouseup", 30);
      await until(function () { return /^dragged|^drag failed/.test(txt("#stMsg") || ""); }, 15000);
      await until(tilesDone, 40000);
      snap("drag");
    }
    await exportNow("export1");
    await close();
    out.menu2 = await openFromMenu();
    await until(tilesDone, 40000);
    snap("reopen");
    await exportNow("export2");
    // Choose... : the sheet is answered by DS_ARA_PICK_DIR; the folder becomes the destination
    var before = EX.dest;
    if (T.arg && EX.dest !== T.arg) {
      q("#exChoose").click();
      await until(function () { return EX.dest !== before; }, 15000);
      await until(tilesDone, 40000);
      snap("chosen");
      await exportNow("export3");
      // #579: the spike type Both in the dialog: the tiles prepare the synthetic and the audio files, Export writes both
      q("#exSpikeBoth").click();
      await sleepM(200);
      await until(tilesDone, 40000);
      snap("both");
      await exportNow("export4");
    }
    await close();
    post(Object.assign({ kind: "exportdialog" }, out));
  }
  /* Mode exportrows (M1c4, #589): the REAL Export dialog in the REAL page makes ONE file per DAW track and kind. The host has a DAW track with clips of three audio files
     and one with one file (the arguments of the stage deliver). Steps: open (the tiles), Export into Exports/ (the files are compared with the files that a take of the
     handles gives), the merged MIDI (single mode), the scope "Selected track" (the single mode is off, with its reason), Choose a folder and Export there, an edit
     followed AT ONCE by Export (the dialog waits for the new set: the new hit is in the file, never the old file), the forged calls. T.arg names the run: "" (all of it),
     "stuck" (a job that never ends: the dialog says which DAW track is not ready, nothing is written; the folder is that of DS_ARA_PICK_DIR). */
  async function exportRowsRun() {
    var out = { steps: [] }, targ = String(T.arg || "").split(";"), OTHER = targ[0], PICK = targ[1] || "";
    function q(sel) { return document.querySelector(sel); }
    function txt(sel) { var e = q(sel); return e ? String(e.textContent || "") : null; }
    function sleepE(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
    function mouse(el, type) { var r = el.getBoundingClientRect(); el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, detail: 1 })); }
    async function until(fn, ms) { var t0 = performance.now(); for (;;) { var v = false; try { v = await fn(); } catch (e) { /* not yet */ } if (v) return Math.round(performance.now() - t0); if (performance.now() - t0 > ms) return -1; await sleepE(80); } }
    function inv(c, a) { return window.__TAURI__.core.invoke(c, a || {}).then(function (v) { return { ok: true, v: v }; }, function (e) { return { ok: false, code: e && (e.code || e.status), msg: String(e && (e.message || e)) }; }); }
    function tile(id) { var t = q(id); return t ? { state: t.dataset.state, text: String(t.querySelector(".exDragN").textContent || "") } : null; }
    function snap(label) {
      var r = { label: label, dest: q("#exDest") ? q("#exDest").value : null, problem: txt("#exProblem"), replace: txt("#exReplace"), go_disabled: q("#exGo").disabled,
        all_text: txt("#exAllText"), sel_text: txt("#exSelText"), skipped: txt("#exSkipped"), single_label: q("#exModeSingle").closest("label").textContent.trim(),
        single_title: q("#exModeSingle").closest("label").title, single_disabled: q("#exModeSingle").disabled, single_checked: q("#exModeSingle").checked,
        result: q("#exResult").classList.contains("hidden") ? null : txt("#exResultText"), msg: txt("#stMsg"), midi: tile("#exDragMidi"), spikes: tile("#exDragSpikes"), body: document.body.innerText.toLowerCase().indexOf("source") >= 0 };
      out.steps.push(r); return r;
    }
    var btn = document.getElementById("hostMenuBtn"), panel = document.getElementById("hostMenu");
    async function openFromMenu() {
      mouse(btn, "mousedown"); mouse(btn, "mouseup"); btn.click();
      if ((await until(function () { return DSHostMenu.isOpen(); }, 3000)) < 0) return "the click did not open the menu";
      var g = panel.querySelector('[data-group="File"]'); if (!g) return "no File group";
      g.click(); await sleepE(100);
      var it = panel.querySelector('[data-cmd="export"]'); if (!it) return "no item export";
      mouse(it, "mousedown"); mouse(it, "mouseup"); it.click();
      if ((await until(function () { return !q("#exportModal").classList.contains("hidden"); }, 10000)) < 0) return "the dialog did not open";
      return "ok";
    }
    async function answerNotice() {
      var go = await until(function () { return !q("#confirmModal").classList.contains("hidden"); }, 1500);
      if (go < 0) return false;
      var b = q('#confirmActions button[data-act="anyway"]'); if (b) b.click();
      return true;
    }
    async function exportNow(label, ms) {
      var n0 = EX.done;
      if (q("#exGo").disabled) { var r0 = snap(label); r0.export_finished = false; r0.disabled = true; return r0; }
      q("#exGo").click();
      out["notice_" + label] = await answerNotice();
      var ok = await until(function () { return EX.done > n0 && !EX.busy; }, ms || 90000);
      await sleepE(300);
      var r = snap(label); r.export_finished = ok >= 0; return r;
    }
    async function ready(kind) { return until(function () { var t = tile(kind); return t && t.state === "ready"; }, 90000); }
    await until(function () { return S.order.length > 0 && S.order.every(function (n) { return (S.events[n] || []).length > 0; }); }, 40000);
    await settle();
    out.regions = (await inv("plugin_host_regions")).v;
    out.events = {};
    for (var i = 0; i < S.order.length; i++) { var g = await inv("events_get", { sid: S.sid, name: S.order[i] }); out.events[S.order[i]] = g.v ? g.v.events : null; }
    out.menu = await openFromMenu();
    var st0 = await inv("plugin_deliver_state"); out.rows = st0.v ? st0.v.rows.map(function (r) { return { id: r.id, name: r.name }; }) : [];
    if (OTHER === "stuck") {
      EX.waitMs = 2500;
      snap("open");
      await exportNow("stuck", 40000);
      out.default = EX.def; post(Object.assign({ kind: "exportrows" }, out)); return;
    }
    await ready("#exDragMidi");
    snap("open");
    var plan = await inv("plugin_deliver_plan", { items: out.rows.map(function (r) { return { row: r.id, kind: "midi" }; }) }); out.plan = plan;
    out.default = EX.def;
    await exportNow("export1", 120000);
    // the files of the handles for the same items: the bytes must be equal
    var items = []; out.rows.forEach(function (r) { items.push({ row: r.id, kind: "midi" }); items.push({ row: r.id, kind: "spike" }); });
    out.take = await inv("plugin_deliver_take", { items: items });
    // the single mode: the merged MIDI file
    q("#exModeSingle").click(); await sleepE(200);
    snap("single");
    out.take_merged = await inv("plugin_deliver_take", { items: [{ row: "", kind: "merged" }] });
    await exportNow("export_single", 60000);
    q("#exModeTrack").click(); await sleepE(100);
    // the scope "Selected track": the single mode is off with its reason, the export holds that track only
    q("#exModeSingle").click(); await sleepE(100);
    var sel = out.regions && out.regions.rows ? out.regions.rows[0] : null;
    var lane = document.querySelector("#lanesScroll .track .tdel"); EX.selected = lane ? lane.closest(".track").dataset.name : (sel ? sel.srcs[0] : S.order[0]);
    q("#exScopeSel").click(); await sleepE(300);
    snap("selected");
    q("#exScopeAll").click(); await sleepE(200);
    q("#exModeTrack").click(); await sleepE(100);
    // Choose...: the sheet is answered by DS_ARA_PICK_DIR; Export writes there
    var before = EX.dest;
    if (PICK && EX.dest !== PICK) { q("#exChoose").click(); await until(function () { return EX.dest !== before; }, 15000); }
    snap("chosen");
    await exportNow("export_chosen", 120000);
    // the type Both: the audio spike files
    q("#exSpikeBoth").click(); await sleepE(200);
    await ready("#exDragSpikes");
    snap("both");
    await exportNow("export_both", 120000);
    // an edit and AT ONCE Export: the new hit is in the file the dialog writes (it waits for the new set)
    var home = out.rows.length ? out.regions.rows[0].srcs[0] : S.order[0], clip = out.regions.rows[0].clips.filter(function (c) { return c.src === home; })[0];
    var ed = await inv("events_edit", { sid: S.sid, name: home, ops: [{ op: "add", t: clip.a + 1.05, v: 100 }] });
    out.edit = { ok: ed.ok, song: clip.s + 1.05 };
    q("#exSpikeSynth").click(); await sleepE(50);
    await exportNow("export_after_edit", 120000);
    // the forged calls: a row that is no row, a name and a path in an item, a folder that is not allowed (the folder of the fault runs is T.arg)
    out.forged = {};
    out.forged.row = await inv("plugin_deliver_export", { items: [{ row: "../etc", kind: "midi" }], start: false });
    out.forged.plan_row = await inv("plugin_deliver_plan", { items: [{ row: "no-such-row", kind: "midi" }] });
    out.forged.name = await inv("plugin_deliver_export", { items: [{ row: out.rows[0].id, kind: "midi", name: "../escape.mid" }], dest: PICK, start: false });
    out.forged.dest = await inv("plugin_deliver_export", { items: [{ row: out.rows[0].id, kind: "midi" }], dest: OTHER || "/tmp", start: false });
    out.forged.sub = await inv("plugin_deliver_export", { items: [{ row: out.rows[0].id, kind: "midi" }], dest: (PICK || "/tmp") + "/sub", start: false });
    out.forged.path = await inv("plugin_deliver_export", { items: [{ row: out.rows[0].id, kind: "midi" }], path: "/tmp/x.mid", start: false });
    out.state_end = (await inv("plugin_deliver_state")).v;
    q("#exCancel").click();
    post(Object.assign({ kind: "exportrows" }, out));
  }
  async function refuse(names) {   // F3: the page cannot change the track set
    var sid = S.sid, name = names[0], out = {};
    var tr0 = JSON.stringify(await window.__TAURI__.core.invoke("tracks_get", { sid: sid }));
    var calls = { tracks_add: { sid: sid, paths: ["/tmp/x.wav"] }, tracks_remove: { sid: sid, names: [name] },
      tracks_restore: { sid: sid, rows: [] }, tracks_reorder: { sid: sid, order: names.slice().reverse() },
      track_rename: { sid: sid, name: name, new_name: "renamed" }, tracks_scan: { paths: ["/tmp"] },
      project_close: { sid: sid }, project_open: { path: "/tmp/nothing" }, project_new: { name: "x", dir: "/tmp" },
      project_save_as: { sid: sid, dir: "/tmp", name: "x" }, project_rename: { sid: sid, name: "x" },
      export_all: { sid: sid, dest: "/tmp/outside" }, transport_play: { sid: sid }, feedback_send: {} };
    for (var c in calls) {
      try { await window.__TAURI__.core.invoke(c, calls[c]); out[c] = { ok: true }; }
      catch (e) { out[c] = { ok: false, code: e && e.code, message: e && e.message }; }
    }
    var tr1 = JSON.stringify(await window.__TAURI__.core.invoke("tracks_get", { sid: sid }));
    post({ kind: "refuse", calls: out, tracks_same: tr0 === tr1, tracks: names });
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
      post({ kind: "analyse-started" });
      for (;;) {
        st = await window.__TAURI__.core.invoke("analyze_status", { sid: S.sid });
        if (st.state === "done" || performance.now() - t0 > 60000) break;
        await new Promise(function (r) { setTimeout(r, 100); });
      }
    } catch (e) { err = String(e && e.message || e); }
    post({ kind: "analyse", state: st && st.state, ms: Math.round(performance.now() - t0), channel_messages: msgs, error: err });
  }

  /* ---- M1b11: the lanes, the menu and the geometry, read from what the page DREW ---- */
  function sleepM(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
  async function settle() {   // the page has drawn everything it was asked to
    for (var i = 0; i < 60; i++) { await sleepM(100); try { if (FR.flags === 0 && !FR.raf) break; } catch (e) { /* none */ } }
    await sleepM(300);
  }
  function laneStats(c) {   // the pixels of one lane canvas: the rows that carry a visible pixel, and the one colour that fills the lane
    var r = c.getBoundingClientRect(), out = { kind: c.classList.contains("spike") ? "spike" : "wave", cssH: Math.round(r.height * 10) / 10, cssW: Math.round(r.width) };
    if (c.width < 2) { out.tiny = true; return out; }
    var d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data, dpr = c.width / r.width, minY = 1e9, maxY = -1, opaque = 0, n = 0, cnt = {};
    for (var y = 0; y < c.height; y++) for (var x = 0; x < c.width; x++) {
      var i = (y * c.width + x) * 4, a = d[i + 3];
      n++;
      if (a > 100) { opaque++; if (y < minY) minY = y; if (y > maxY) maxY = y; }
      if (x % 3 === 0) { var k = (d[i] >> 5) + "," + (d[i + 1] >> 5) + "," + (d[i + 2] >> 5) + "," + (a >> 6); cnt[k] = (cnt[k] || 0) + 1; }
    }
    var top = Object.keys(cnt).sort(function (a, b) { return cnt[b] - cnt[a]; })[0], tot = 0; Object.keys(cnt).forEach(function (k) { tot += cnt[k]; });
    out.storeH = Math.round(c.height / dpr * 10) / 10; out.dpr = Math.round(dpr * 100) / 100;
    out.minY = minY === 1e9 ? null : Math.round(minY / dpr * 10) / 10; out.maxY = maxY < 0 ? null : Math.round(maxY / dpr * 10) / 10;
    out.opaque = Math.round(opaque / n * 1000) / 1000; out.top = top; out.share = Math.round(cnt[top] / tot * 1000) / 1000;
    return out;
  }
  function lanesOf(name) {
    var row = document.querySelector('.track[data-name="' + CSS.escape(name) + '"]'), o = {};
    if (!row) return o;
    row.querySelectorAll("canvas.lane").forEach(function (c) { var st = laneStats(c); o[st.kind] = st; });
    return o;
  }
  function hostAsk(kind, extra) { post(Object.assign({ kind: kind }, extra || {})); }
  async function pointerClick(el, fx, fy, count) {   // a real click of the pointer: the host sends the NSEvents
    el.scrollIntoView({ block: "nearest" }); await sleepM(200);
    var r = el.getBoundingClientRect();
    hostAsk("host-click", { x: r.left + r.width * fx, y: r.top + r.height * fy, count: count });
  }
  /* Mode hit: a hit made with the pointer in the lane of a track of the role "other" and of a drum role; the report
     carries the selection, the colour of the track and the pixels of both lanes before and after. */
  var pointerLog = [];
  ["mousedown", "mouseup", "click", "dblclick", "pointerdown", "pointerup"].forEach(function (t) {
    window.addEventListener(t, function (ev) { if (T.mode === "hit" || T.mode === "menu") pointerLog.push([t, Math.round(ev.clientX), Math.round(ev.clientY), ev.detail, ev.target && (ev.target.className || ev.target.id || ev.target.tagName)]); }, true);
  });
  async function hitRun(names) {
    var tracks = [];
    if (T.fault === "no-color") {   // the fault run: the service gives no colour and the page has no repair
      window.trackColors = function () { /* the repair is off */ };
      window.trackColor = function () { return undefined; };
      names.forEach(function (n) { S.tracks[n].color = undefined; });
      invalidate(F.VIEW);
    }
    await settle();
    var roleOther = null;
    try { roleOther = await setTrackRole(names[0], "other"); } catch (e) { roleOther = "threw " + e; }
    if (T.fault === "no-color") names.forEach(function (n) { S.tracks[n].color = undefined; });
    await settle();
    var pair = [names[0], names[1]];
    for (var i = 0; i < pair.length; i++) {
      var name = pair[i], n0 = (S.events[name] || []).length, before = lanesOf(name);
      var spike = document.querySelector('.track[data-name="' + CSS.escape(name) + '"] canvas.spike');
      var evs = S.events[name] || [], dur = S.tracks[name].duration || 1;
      var t = evs.length >= 2 ? (evs[0].t + evs[1].t) / 2 : dur / 2;   // between two hits, inside the audio of the track
      await pointerClick(spike, (t - S.view.t0) / (S.view.t1 - S.view.t0), 0.5, 2);
      var added = false;
      for (var k = 0; k < 60 && !added; k++) { await sleepM(100); added = (S.events[name] || []).length > n0; }
      await settle();
      var lanes = lanesOf(name);
      tracks.push({ name: name, role: S.tracks[name].role, note: S.tracks[name].note, color: S.tracks[name].color === undefined ? null : S.tracks[name].color,
        pointer_added: added, n0: n0, n1: (S.events[name] || []).length, sel: S.sel, ln_sel: LN.sel,
        added_flags: (S.events[name] || []).filter(function (e) { return e.added; }).length, before: before, after: lanes });
    }
    post({ kind: "hit", visibility: document.visibilityState, role_other: roleOther, tracks: tracks, host_mode: !!B.hostMode, pointer_log: pointerLog.slice(0, 40) });
  }
  /* Mode geometry: the lines of the hits and of the grid must span the lane after every change of the zoom, of the track height
     and of the window (also across the 520 px of the low window). One row per step: the size of each lane box, of its store and the
     rows that carry a line. */
  async function geometryRun() {
    var names = S.order.slice(), steps = [];
    if (T.fault === "stale-geometry") {   // the fault run: the page as it was before #559, the key of the sizes without the measured height of the rows
      window.sizeKey = function () { return [GEO.laneW, GEO.rulerW, GEO.ovW, GEO.ovH, GEO.waveH, GEO.spikeH, GEO.dpr, rowHKey(), VL.on ? VL.h : 0].join(","); };
    }
    function centre() {   // a view of 500 ms with a hit of every track in it
      var t = null;
      names.forEach(function (n) { if (t === null && (S.events[n] || []).length) t = S.events[n][0].t; });
      S.playhead = t === null ? 0 : t; zoomPreset(3);
    }
    async function rec(name) {
      await settle();
      var rows = {};
      names.forEach(function (n) { rows[n] = lanesOf(n); });
      steps.push({ name: name, th: S.trackHeight, win: [innerWidth, innerHeight], dpr: window.devicePixelRatio, low: document.body.classList.contains("hostLow"),
        view: [S.view.t0, S.view.t1], geo: { waveH: GEO.waveH, spikeH: GEO.spikeH, dpr: GEO.dpr }, lanes: rows });
    }
    async function until(cond, ms) { var t0 = performance.now(); while (performance.now() - t0 < ms && !cond()) await sleepM(100); return cond(); }
    await settle(); centre(); await rec("base");
    setTrackHeight("large"); await rec("track-large");
    zoomPreset(0); await rec("zoom-full");
    setTrackHeight("small"); await rec("track-small");
    centre(); await rec("zoom-in");
    var own = {}; own[names[0]] = 1.5; setRowHeights(own); await rec("row-taller");   // the Zoom Toggle and the drag of a row: one row with its own height
    setRowHeights({}); await rec("row-reset");
    hostAsk("host-low"); var low = await until(function () { return innerHeight < 520; }, 30000);
    await rec("window-low");
    setTrackHeight("large"); await rec("low-track-large");
    zoomPreset(0); await rec("low-zoom-full");
    centre(); await rec("low-zoom-in");
    hostAsk("host-high"); var high = await until(function () { return innerHeight >= 520; }, 30000);
    await rec("window-high");
    setTrackHeight("medium"); centre(); await rec("high-track-medium");
    post({ kind: "geometry", visibility: document.visibilityState, went_low: low, went_high: high, steps: steps });
  }
  /* Mode menu: the menu button of the page in host mode: the commands, the settings, a view option, the keyboard and the low window. */
  async function menuRun() {
    await settle();
    var rep = { visibility: document.visibilityState, host_mode: !!B.hostMode };
    if (T.fault === "no-menu") { var b0 = document.getElementById("hostMenuBtn"); if (b0) b0.remove(); }
    var btn = document.getElementById("hostMenuBtn"), panel = document.getElementById("hostMenu");
    rep.button = !!btn; rep.panel = !!panel;
    if (!btn) { post(Object.assign({ kind: "menu" }, rep)); return; }
    function key(el, k) { el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true })); }
    var model = DSHostMenu.model(); rep.groups = model.map(function (g) { return [g.name, g.items.length]; });
    rep.audit = DSHostMenu.audit();
    rep.first_title = btn.title;
    // the keyboard: Enter opens, Down moves, Escape closes and the focus is back on the button
    btn.focus(); key(btn, "Enter"); await sleepM(100);
    var kb = { opened: DSHostMenu.isOpen(), focus: document.activeElement && document.activeElement.dataset && document.activeElement.dataset.group };
    key(document.activeElement, "ArrowDown"); kb.moved = document.activeElement.dataset.group;
    key(document.activeElement, "Escape"); await sleepM(100);
    kb.closed = !DSHostMenu.isOpen(); kb.focus_back = document.activeElement && document.activeElement.id;
    rep.keyboard = kb;
    // a view option with the pointer: View > Track Height > Large
    function mouse(el, type) { var r = el.getBoundingClientRect(); el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, detail: 1 })); }
    async function pick(group, cmd) {   // the clicks are DOM events: the web view of this host does not hand a single click of the host's NSEvents to the page (the double click of mode hit comes through)
      mouse(btn, "mousedown"); mouse(btn, "mouseup"); mouse(btn, "click");
      for (var i = 0; i < 20 && !DSHostMenu.isOpen(); i++) await sleepM(50);
      if (!DSHostMenu.isOpen()) return "the click did not open the menu";
      var g = panel.querySelector('[data-group="' + group + '"]'); mouse(g, "click"); await sleepM(100);
      var it = panel.querySelector('[data-cmd="' + cmd + '"]'); if (!it) return "no item " + cmd;
      mouse(it, "mousedown"); mouse(it, "mouseup"); mouse(it, "click");
      for (var j = 0; j < 20 && DSHostMenu.isOpen(); j++) await sleepM(50);
      return DSHostMenu.isOpen() ? "the click did not run the item" : "ok";
    }
    rep.pick_height = await pick("View", "height_large"); await settle();
    rep.track_height = S.trackHeight;
    // the settings
    rep.pick_settings = await pick("Drum Spike", "settings"); await sleepM(300);
    var modal = document.getElementById("settingsModal");
    rep.settings = { visible: !!modal && !modal.classList.contains("hidden"),
      cats: Array.prototype.filter.call(document.querySelectorAll("#setCats .setCatBtn"), function (b) { return b.checkVisibility({ visibilityProperty: true }); }).map(function (b) { return b.dataset.cat; }) };
    var done = document.getElementById("settingsDone"); if (done) done.click(); await sleepM(300);
    // the low window
    hostAsk("host-low"); await (async function () { var t0 = performance.now(); while (performance.now() - t0 < 30000 && innerHeight >= 520) await sleepM(100); })();
    await settle();
    btn.click(); await sleepM(200);
    panel.querySelector('[data-group="View"]').click(); await sleepM(200);
    var pr = panel.getBoundingClientRect(), items = panel.querySelectorAll("[data-cmd]"), last = items[items.length - 1];
    last.scrollIntoView({ block: "nearest" }); await sleepM(100);
    var lr = last.getBoundingClientRect(), pr2 = panel.getBoundingClientRect();
    rep.low = { innerH: innerHeight, innerW: innerWidth, top: pr2.top, bottom: pr2.bottom, left: pr2.left, right: pr2.right, scrolls: panel.scrollHeight > panel.clientHeight,
      last_visible: lr.top >= pr2.top - 1 && lr.bottom <= pr2.bottom + 1, last_in_window: lr.bottom <= innerHeight };
    key(document.activeElement, "Escape"); key(document.activeElement, "Escape"); await sleepM(100);
    rep.low.closed = !DSHostMenu.isOpen();
    rep.pointer_log = pointerLog.slice(0, 40);
    post(Object.assign({ kind: "menu" }, rep));
  }
  /* a page-side post that the host can ask for at any time */
  window.__DS_REPORT_NOW__ = function () { post({ kind: "now", resources: resources(), violations: violations }); };
  if (T.mode === "net") setTimeout(window.__DS_REPORT_NOW__, 6000);
})();
