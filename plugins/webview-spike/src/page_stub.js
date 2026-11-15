/* The stub transport and the test drivers of the S2 spike (ticket #505). This file is part of
   the plugin, not of the page: the plugin injects it at document start through wry's
   initialization script (before static/backend.js runs), and no file of static/ is changed.

   What it does:
   - sets window.__DS_HOST_MODE__ before the page boots (P2 host mode),
   - provides window.__TAURI__ = a stub transport for static/backend.js that answers every
     command the page asks at boot from one recorded fixture project (8 tracks, 5 minutes,
     48 kHz, synthetic audio only; one fixed seed, so the content is recorded, not random),
   - after the page boots, sets B.hostTransport (play/stop/locate/onPosition/tempoMap, as
     transport.js documents it) and calls the page's appHostFollow() itself,
   - drives the per-mode test logic and reports to the native side over window.ipc.

   The mode comes in as window.__DS_SPIKE_MODE__ (the plugin prepends it to this script):
     boot      report ready when the page's B.appReady() ran (the 50 open/close cycles)
     measure   report frame times while the page scrolls and zooms the fixture (8 s)
     keys      record which keys reach the page, report at +10 s
     focus     report document.hasFocus() snapshots for 8 s (the host clicks in and out)
     resize    report window.innerWidth/Height for 6 s (the host resizes the window)
     no-ready  the fault run: boot normally but never report ready
*/
(function () {
  "use strict";
  var MODE = window.__DS_SPIKE_MODE__ || "boot";

  /* ---------------- the recorded fixture project ---------------- */
  /* one fixed PRNG: the same content every boot ("recorded"), synthetic only */
  function mulberry32(a) {
    return function () {
      a |= 0; a = (a + 0x6D2B79F5) | 0;
      var t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }
  var FIX = {
    sid: "fixture-s2",
    name: "S2 Fixture",
    sr: 48000,
    dur: 300,            // 5 minutes
    tracks: [
      { name: "Kick",  role: "kick",   note: 36, color: "#c25b4e", seed: 101 },
      { name: "Snare", role: "snare",  note: 38, color: "#4e79c2", seed: 202 },
      { name: "Hat",   role: "hat",    note: 42, color: "#4ec27a", seed: 303 },
      { name: "Tom 1", role: "tom",    note: 48, color: "#c2a44e", seed: 404 },
      { name: "Tom 2", role: "tom",    note: 45, color: "#a44ec2", seed: 505 },
      { name: "Ride",  role: "cymbal", note: 51, color: "#4ec2c2", seed: 606 },
      { name: "Clap",  role: "other",  note: 39, color: "#c24e8a", seed: 707 },
      { name: "Room",  role: "other",  note: 55, color: "#8a8a4e", seed: 808 }
    ],
    levels: { "65536": null, "4096": null, "256": null }   // bin size -> bin count (filled below)
  };
  Object.keys(FIX.levels).forEach(function (k) { FIX.levels[k] = Math.ceil((FIX.dur * FIX.sr) / +k); });
  var tileCache = {};   // "name/level" -> the full tile buffer of that level

  function peakValue(rnd, i, n) {   // a stable pseudo-waveform: slow envelope + noise
    var env = 0.35 + 0.3 * Math.sin(i / 200 + rnd() * 1e-9);
    return Math.min(1, Math.max(0.02, env * (0.55 + 0.45 * rnd())));
  }
  function trackTile(name, levelStr) {
    var key = name + "/" + levelStr;
    if (tileCache[key]) return tileCache[key];
    var n = FIX.levels[levelStr], level = +levelStr;
    var seed = 0, i;
    for (i = 0; i < FIX.tracks.length; i++) if (FIX.tracks[i].name === name) seed = FIX.tracks[i].seed;
    var rnd = mulberry32(seed * 7919 + level);
    var buf = new ArrayBuffer(12 + 12 * n);
    var u = new Uint32Array(buf, 0, 3), f = new Float32Array(buf, 12);
    u[0] = level; u[1] = 0; u[2] = n;
    for (i = 0; i < n; i++) {
      var mid = peakValue(rnd, i, n);
      f[i] = -mid;                 // min
      f[n + i] = mid;              // max
      f[2 * n + i] = mid * 0.4;    // rms
    }
    tileCache[key] = buf;
    return buf;
  }
  function tileRange(name, levelStr, from, to) {   // a sub range of the level's tile as one peaks_tile answer
    var n = FIX.levels[levelStr], level = +levelStr;
    var a = Math.max(0, from | 0), b = Math.min(n, to | 0), cnt = b - a;
    var full = trackTile(name, levelStr);
    if (a === 0 && cnt === n) return full;
    var src = new Float32Array(full, 12);
    var out = new ArrayBuffer(12 + 12 * cnt);
    var u = new Uint32Array(out, 0, 3), f = new Float32Array(out, 12);
    u[0] = level; u[1] = a; u[2] = cnt;
    f.set(src.subarray(a, a + cnt), 0);
    return out;
  }
  function trackEvents(seed) {   // about one hit per second, at 0.5 s grid + jitter
    var rnd = mulberry32(seed), evs = [], t = 0.25;
    while (t < FIX.dur) { evs.push({ t: +t.toFixed(3), v: 40 + Math.floor(rnd() * 87) }); t += 0.5 + rnd() * 1.5; }
    return evs;
  }
  function sampleWindow(start, end) {   // u32 start, u32 sr, f32 samples
    var a = Math.max(0, start | 0), b = Math.min(FIX.dur * FIX.sr, end | 0), n = Math.max(0, b - a);
    var out = new ArrayBuffer(8 + 4 * n), u = new Uint32Array(out, 0, 2), f = new Float32Array(out, 8);
    u[0] = a; u[1] = FIX.sr;
    var rnd = mulberry32(9001 + a);
    for (var i = 0; i < n; i++) f[i] = 0.5 * Math.sin((a + i) / 40) * (0.6 + 0.4 * rnd());
    return out;
  }

  /* ---------------- the report channel to native ---------------- */
  var logLines = [];
  function spikePost(obj) {
    try { window.ipc.postMessage(JSON.stringify(obj)); } catch (e) { /* native gone (destroy) */ }
  }
  function log(s) { if (logLines.length < 60) logLines.push(String(s).slice(0, 400)); }
  try {
    var oerr = console.error, owarn = console.warn;
    console.error = function () { log("E " + Array.prototype.map.call(arguments, String).join(" ")); oerr.apply(console, arguments); };
    console.warn = function () { log("W " + Array.prototype.map.call(arguments, String).join(" ")); owarn.apply(console, arguments); };
  } catch (e) { /* no console */ }
  window.addEventListener("error", function (ev) { log("X " + ev.message + " @" + (ev.filename || "") + ":" + (ev.lineno || 0)); });

  /* ---------------- the stub transport (window.__TAURI__) ---------------- */
  var hostCalls = [];   // play/stop/locate calls the page made on the host transport
  function stubChannel() {
    return { onmessage: null, closed: false, live: false, toJSON: function () { return { __channel: "stub" }; } };
  }
  function stubInvoke(cmd, args) {
    args = args || {};
    var sid = args.sid;
    function ok(v) { return Promise.resolve(v); }
    switch (cmd) {
      case "project_list": return ok([{ sid: FIX.sid, name: FIX.name, dirty: false }]);
      case "project_get": return ok({ sid: sid, name: FIX.name, dirty: false });
      case "project_recent": return ok([]);
      case "project_recent_clear": return ok({ cleared: 0 });
      case "tracks_get": {
        var tracks = {}, order = [];
        FIX.tracks.forEach(function (t) {
          tracks[t.name] = { role: t.role, note: t.note, color: t.color, duration: FIX.dur, follows: null,
            params: null };   // the page fills its defaults (defaultParams)
          order.push(t.name);
        });
        return ok({ name: FIX.name, dirty: false, tracks: tracks, order: order });
      }
      case "events_get": {
        var i; for (i = 0; i < FIX.tracks.length; i++) if (FIX.tracks[i].name === args.name) return ok(trackEvents(FIX.tracks[i].seed));
        return ok([]);
      }
      case "events_set": case "events_edit": case "track_params_set": case "velocities_recompute":
        return ok({ events: [] });
      case "peaks_info": return ok({ sr: FIX.sr, n_samples: FIX.dur * FIX.sr,
        levels: { "65536": { n: FIX.levels["65536"] }, "4096": { n: FIX.levels["4096"] }, "256": { n: FIX.levels["256"] } } });
      case "peaks_tile": return ok(tileRange(args.name, String(args.level), args.from, args.to));
      case "samples_window": return ok(sampleWindow(args.start, args.end));
      case "analyze_status": return ok({ tracks: {} });
      case "job_status": return ok({ state: "done" });
      case "peaks_build_start": return ok({ started: [] });   // prepareLanes finishes at once
      case "peaks_build_status": return ok({ state: "done" });
      case "app_info": return ok({ core_version: "spike-s2-fixture", detector: "fixture", chain: "stub" });
      case "app_settings_get": return ok({});
      case "app_settings_set": return ok({});
      case "shell_sync": return ok(0);
      case "shell_typing": return ok({});
      case "export_last_get": return ok({ export_last: 0 });
      case "row_heights_get": return ok({ heights: {} });
      case "row_heights_set": return ok({ heights: {} });
      case "tempo_map_get": return ok({ map: null, default: null });
      case "markers_get": return ok({ markers: [], n: 0 });
      case "transport_status": return ok({ t_ns: 0, pos: 0, playing: false, rate: FIX.sr, underruns: 0, reopens: 0 });
      case "outputs_list": return ok({ devices: [{ name: "fixture", rate: FIX.sr, channels: 2, default: true }],
        choice: null, current: "fixture", note: "", null_output: true, master_db: 0 });
      case "plugins_list": return ok({ plugins: [] });
      case "agent_skill_status": return ok({ installed: false, files: {} });
      case "about_info": return ok({ name: "Drum Spike", version: "spike-s2", core_version: "spike-s2-fixture", notice: "" });
      case "app_ready": {   // P3-17: the page's first layout is done -> the native side learns it
        if (MODE !== "no-ready") {
          spikePost({ kind: "ready", mode: MODE, sid: typeof S !== "undefined" ? S.sid : null,
            tracks: typeof S !== "undefined" && S.order ? S.order.length : 0,
            innerW: window.innerWidth, innerH: window.innerHeight, logs: logLines.slice(0, 12) });
        } else {
          spikePost({ kind: "log-no-ready", logs: logLines.slice(0, 12) });   // the fault run still talks
        }
        return ok({ ok: true });
      }
      default:
        log("stub: unhandled command " + cmd);
        return ok(null);
    }
  }
  window.__DS_HOST_MODE__ = true;
  window.__TAURI__ = {
    core: { invoke: stubInvoke, Channel: stubChannel },
    dialog: { open: function () { return Promise.resolve(null); }, save: function () { return Promise.resolve(null); } },
    event: { listen: function () { return Promise.resolve(function () {}); } }
  };
  /* localStorage is per-origin in WKWebView; a throw at boot must not kill initSessions */
  try { window.localStorage.setItem("dsg_probe", "1"); window.localStorage.removeItem("dsg_probe"); }
  catch (e) {
    var mem = {};
    try { Object.defineProperty(window, "localStorage", { value: {
      getItem: function (k) { return k in mem ? mem[k] : null; },
      setItem: function (k, v) { mem[k] = String(v); },
      removeItem: function (k) { delete mem[k]; }, clear: function () { mem = {}; }, key: function (i) { return Object.keys(mem)[i] || null; },
      get length() { return Object.keys(mem).length; } } }); } catch (e2) { log("no localStorage"); }
  }

  /* ---------------- the key recorder (every mode: the evidence of the key report) ---------------- */
  var KEY_NAMES = { Space: "Space", ArrowRight: "Right", ArrowLeft: "Left", ArrowUp: "Up", ArrowDown: "Down", Tab: "Tab", Escape: "Esc" };
  var keySeen = [];
  var focusKeys = [];   // focus mode: the keys with their time; the input-path evidence of the focus
  document.addEventListener("keydown", function (ev) {
    var name = KEY_NAMES[ev.code] || (ev.code === "KeyZ" ? "Z" : null);
    if (!name) return;
    if (MODE === "focus") focusKeys.push({ t: Math.round(performance.now()), key: name });
    var row = { key: name, meta: !!ev.metaKey, ctrl: !!ev.ctrlKey, shift: !!ev.shiftKey };
    try { var f = typeof DSKeys !== "undefined" && DSKeys.find ? DSKeys.find(ev) : null; if (f) row.page = f.cmd; } catch (e) { /* the finder must not see a half page */ }
    if (ev.defaultPrevented) row.preventDefault = true;
    keySeen.push(row);
  }, true);

  /* ---------------- the host transport + the late injection ---------------- */
  var hostTransport = {
    _calls: hostCalls,
    play: function (pos, speed) { hostCalls.push({ call: "play", pos: pos, speed: speed || 1 }); return Promise.resolve(); },
    stop: function (pos) { hostCalls.push({ call: "stop", pos: pos }); },
    locate: function (pos) { hostCalls.push({ call: "locate", pos: pos }); },
    tempoMap: function () { return Promise.resolve(null); },
    onPosition: null
  };
  function injectHostTransport() {   // backend.js has run (B exists) and app.js may already have booted:
    try { B.hostTransport = hostTransport; } catch (e) { log("no B: " + e); }
    try { if (typeof appHostFollow === "function") appHostFollow(); } catch (e) { log("no appHostFollow: " + e); }
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", injectHostTransport);
  else injectHostTransport();

  /* ---------------- the per-mode drivers ---------------- */
  function laneCanvases() {   // the spike/wave canvases of the track rows (tracks.js binds wheel on them)
    return Array.prototype.slice.call(document.querySelectorAll("#lanesScroll canvas"));
  }
  function driveWheel() {   // alternate scroll and zoom over the lanes, the page's own handlers do the rest
    var cs = laneCanvases();
    if (!cs.length) return false;
    var c = cs[0], up = false, n = 0;
    var iv = setInterval(function () {
      n++;
      var opts = { deltaY: up ? -240 : 240, deltaX: 0, deltaMode: 0, bubbles: true, cancelable: true };
      if (n % 5 === 0) opts.ctrlKey = true;   // every fifth wheel is a zoom (the page zooms the timeline with it)
      else opts.ctrlKey = false;
      var ev;
      try { ev = new WheelEvent("wheel", opts); } catch (e) { return; }
      c.dispatchEvent(ev);
      if (n % 25 === 0) { up = !up; var other = cs[(n / 25) % cs.length | 0]; if (other) c = other; }
    }, 100);
    return iv;
  }
  function p95(a) { if (!a.length) return null; var s = a.slice().sort(function (x, y) { return x - y; }); return s[Math.min(s.length - 1, Math.floor(0.95 * (s.length - 1)))]; }

  function whenBooted(fn, deadlineMs) {   // run fn when the page is through loadProject (S.loaded set) or the deadline came
    var t0 = performance.now();
    (function poll() {
      var loaded = false;
      try { loaded = typeof S !== "undefined" && S.loaded > 0; } catch (e) { /* not yet */ }
      if (loaded || performance.now() - t0 > (deadlineMs || 12000)) fn(loaded);
      else setTimeout(poll, 100);
    })();
  }

  if (MODE === "measure") {
    whenBooted(function (loaded) {
      var t0 = performance.now(), last = t0, deltas = [];
      var wheel = driveWheel();
      function loop(t) {
        deltas.push(t - last); last = t;
        if (t - t0 < 8000) requestAnimationFrame(loop);
        else {
          if (wheel) clearInterval(wheel);
          deltas = deltas.slice(20);   // the warmup frames of the first draw stay out
          var p = p95(deltas), mx = 0;
          deltas.forEach(function (d) { if (d > mx) mx = d; });
          spikePost({ kind: "measure", frames: deltas.length, p95_ms: p === null ? null : +p.toFixed(2), max_ms: +mx.toFixed(2), booted: loaded,
            innerW: window.innerWidth, innerH: window.innerHeight });
        }
      }
      requestAnimationFrame(loop);
    });
  }
  if (MODE === "keys") {   // the host sends the keys at 2.5..7.5 s; the report goes at 10 s
    setTimeout(function () {
      spikePost({ kind: "keys", seen: keySeen, hostCalls: hostCalls, hasFocus: document.hasFocus() });
    }, 10000);
  }
  if (MODE === "focus") {   // the host takes the focus at ~2.5 s, gives it back at ~4.5 s
    var fSamples = [], t0 = performance.now();
    var iv = setInterval(function () {
      fSamples.push({ t: Math.round(performance.now() - t0), hasFocus: document.hasFocus(),
        active: (document.activeElement && document.activeElement.tagName) || null });
      if (performance.now() - t0 > 8000) {
        clearInterval(iv);
        spikePost({ kind: "focus", samples: fSamples, keys: focusKeys, hostCalls: hostCalls });
      }
    }, 300);
  }
  if (MODE === "resize") {   // the host resizes the window at ~2 s and ~4 s
    var rSamples = [], r0 = performance.now();
    var riv = setInterval(function () {
      rSamples.push({ t: Math.round(performance.now() - r0), innerW: window.innerWidth, innerH: window.innerHeight });
      if (performance.now() - r0 > 6000) {
        clearInterval(riv);
        spikePost({ kind: "resize", samples: rSamples });
      }
    }, 200);
  }
  if (MODE === "boot") {   // a late console line still reaches the log file of the cycle
    setTimeout(function () { spikePost({ kind: "log", logs: logLines }); }, 6000);
  }
})();
