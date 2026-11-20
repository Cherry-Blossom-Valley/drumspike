/* The shell script of the editor page (M1b2, #538). The plugin runs it before every page
   script (wry's initialization script). It gives the page what the Tauri shell of the desktop
   app gives it: window.__TAURI__ with core.invoke, core.Channel, dialog.open, dialog.save and
   event.listen, and it sets window.__DS_HOST_MODE__ (the page then leaves the files to the
   host, as static/backend.js and static/state.js read it).

   The transport is the page's own scheme. invoke is a fetch POST to /__ds/invoke/<command>;
   the plugin runs drumspike_app::call::run on the service of the ARA document and answers
   JSON, or bytes for the four binary commands (an ArrayBuffer). A failed command rejects with
   {code, message}, as the Tauri shell does. A Channel is fetched by long poll on
   /__ds/channel/<id> (the way of the test bridge of the app); the events of the service come
   by long poll on /__ds/events. No thread of the plugin pushes into the web view. The page's
   Content-Security-Policy allows only this scheme. */
(function () {
  "use strict";
  var API = "/__ds/";
  window.__DS_HOST_MODE__ = true;
  function sleep(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
  var seq = 0;
  function Channel() {
    this.id = "c" + Date.now().toString(36) + (++seq).toString(36) + Math.random().toString(36).slice(2, 8);
    this.onmessage = null;
    this.closed = false;
    this.live = false;
  }
  Channel.prototype.toJSON = function () { return { __channel: this.id }; };
  Channel.prototype.poll = async function () {
    for (var i = 0; (this.live || i < 600) && !this.closed; i++) {
      var msgs = [];
      try { msgs = await (await fetch(API + "channel/" + this.id + "?timeout_ms=2000", { cache: "no-store" })).json(); }
      catch (e) { await sleep(300); }
      for (var k = 0; k < msgs.length; k++) {
        var m = msgs[k];
        if (this.onmessage) this.onmessage(m);
        if (m && ((m.job && m.job.state === "done") || (m.export && m.export.state === "done") ||
          (m.copy && ["done", "cancelled", "error"].indexOf(m.copy.state) >= 0) || (this.live && m.end === true && !m.error))) this.closed = true;
      }
    }
  };
  async function invoke(cmd, args) {
    args = args || {};
    var chans = Object.values(args).filter(function (v) { return v instanceof Channel; });
    chans.forEach(function (v) { v.poll(); });
    var r;
    try {
      r = await fetch(API + "invoke/" + encodeURIComponent(cmd), { method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify(args), cache: "no-store" });
    } catch (e) {
      chans.forEach(function (v) { v.closed = true; });
      throw { code: 500, message: "the plugin did not answer: " + e };
    }
    if (!r.ok) {
      chans.forEach(function (v) { v.closed = true; });
      throw await r.json().catch(function () { return { code: r.status, message: "plugin error" }; });
    }
    return (r.headers.get("content-type") || "").indexOf("application/json") === 0 ? r.json() : r.arrayBuffer();
  }
  var polling = false;
  async function pollEvents() {
    polling = true;
    var after = "open";   // from the moment this editor opened: an event announced before the first poll is not lost
    for (;;) {
      var r = null;
      try { r = await (await fetch(API + "events?after=" + after + "&timeout_ms=2000", { cache: "no-store" })).json(); }
      catch (e) { await sleep(300); continue; }
      after = r.seq;
      for (var i = 0; i < r.events.length; i++) {
        var m = r.events[i];
        (listeners[m.event] || []).slice().forEach(function (f) { f({ event: m.event, payload: m.payload }); });
      }
    }
  }
  // The plugin tells the page when it changed the data behind the page's back (a round of the
  // plugin ended, an archive was restored): the page loads its project again. The page's pending
  // writes are DROPPED, not flushed: they hold the page's older list and would be written over
  // the data of the round (S3). The page's edit queue is NOT waited for either (R5): a queued
  // operation is index-based and would be applied to the new list. A burst of events is one
  // reload. The view (zoom and scroll of the lanes) is kept across the reload; the undo history
  // is not (open point of #539).
  // What this script uses of the page: functions, two globals and one element. A rename in
  // static/ must not make the reload do less in silence (S4): a missing one is reported at once
  // (console error, shell command plugin_report), and a stage checks the bundled page for them.
  var PAGE_FUNCTIONS = ["cancelEvents", "initSessions", "loadProject", "invalidate"];
  var PAGE_GLOBALS = ["S", "F"];
  var PAGE_IDS = ["lanesScroll"];
  function missingPageParts() {
    var names = PAGE_FUNCTIONS.slice();
    if (window.__DS_TEST__ && window.__DS_TEST__.fault === "missing-function") names.push("renamedLoadProject");   // the check's fault run
    var out = names.filter(function (n) {
      try { return typeof (0, eval)(n) !== "function"; } catch (e) { return true; }
    });
    PAGE_GLOBALS.forEach(function (n) {
      try { if (typeof (0, eval)(n) !== "object") out.push(n); } catch (e) { out.push(n); }
    });
    PAGE_IDS.forEach(function (n) { if (!document.getElementById(n)) out.push("#" + n); });
    return out;
  }
  var reloading = false, again = false;
  async function reloadProject() {
    if (reloading) { again = true; return; }
    reloading = true;
    try {
      var missing = missingPageParts();
      if (missing.length) {
        console.error("drumspike plugin: the page has no " + missing.join(", ") + "; the reload does less");
        try { await invoke("plugin_report", { missing_page_functions: missing }); } catch (e) { /* no plugin */ }
      }
      do {
        again = false;
        var view = null, scroll = 0;
        try { view = { t0: S.view.t0, t1: S.view.t1 }; scroll = document.getElementById("lanesScroll").scrollTop; } catch (e) { /* no view yet */ }
        try { if (typeof cancelEvents === "function" && S.sid) cancelEvents(S.sid); } catch (e) { /* none */ }
        if (typeof initSessions === "function") await initSessions();
        if (typeof loadProject === "function") await loadProject();
        try {
          if (view && S.order && S.order.length && view.t1 > view.t0) {
            S.view = { t0: view.t0, t1: view.t1 };
            if (typeof invalidate === "function" && typeof F !== "undefined") invalidate(F.VIEW);
            document.getElementById("lanesScroll").scrollTop = scroll;
          }
        } catch (e) { /* the page keeps its full view */ }
      } while (again);
    } catch (e) { /* the page shows its own error */ }
    reloading = false;
  }
  var listeners = {};
  listeners["plugin-reload"] = [function () { reloadProject(); }];
  window.__TAURI__ = {
    core: { invoke: invoke, Channel: Channel },
    dialog: {
      open: async function (opts) { window.__dialogOpts = opts; return null; },
      save: async function (opts) { window.__saveOpts = opts; return null; }
    },
    event: {
      listen: async function (name, cb) {
        (listeners[name] = listeners[name] || []).push(cb);
        if (!polling) pollEvents();
        return function () { listeners[name] = (listeners[name] || []).filter(function (f) { return f !== cb; }); };
      }
    }
  };
  pollEvents();   // the plugin's own listener starts at once
  try { window.localStorage.setItem("dsg_probe", "1"); window.localStorage.removeItem("dsg_probe"); }
  catch (e) {
    var mem = {};
    try {
      Object.defineProperty(window, "localStorage", { value: {
        getItem: function (k) { return k in mem ? mem[k] : null; },
        setItem: function (k, v) { mem[k] = String(v); },
        removeItem: function (k) { delete mem[k]; }, clear: function () { mem = {}; },
        key: function (i) { return Object.keys(mem)[i] || null; },
        get length() { return Object.keys(mem).length; } } });
    } catch (e2) { /* no storage: the page runs without it */ }
  }
})();
