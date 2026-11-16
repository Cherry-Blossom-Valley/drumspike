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
  var listeners = {};
  var polling = false;
  async function pollEvents() {
    polling = true;
    var after = "now";
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
