/* The key notice of the editor (M1b12, #561). The plugin runs it after shell.js (wry's initialization
   script), in every page load.

   The rule is native (src/keys.rs): while the plugin's view has the keyboard focus, no key reaches the
   host except a short list of system keys. A key never goes on to the host, so a key that the page does
   not use would be silent. This script makes it say something: when the user presses the key of a command
   that is ABSENT in host mode (#539: the host plays the audio, owns the files, the window, the tracks, no
   agent), the page shows the text of that command's group on its status line (HOST_ABSENT in
   static/state.js, shown by hostAbsent()). A key that has no command at all stays silent, as in the app.

   "Not used" is read from the page itself: no handler took the key (defaultPrevented, read when the
   dispatch has ended), and it is not a letter typed into a text field. It only reads what static/ has:
   DSKeys (keymap.js), HOST_ABSENT, hostOwned and hostAbsent (state.js). It changes nothing in static/.
   A page without them makes it do nothing. */
(function () {
  "use strict";
  function lex(expr) { try { return (0, eval)(expr); } catch (e) { return undefined; } }

  /* What the page makes of a keydown: {kind: page | absent | none, key, id, group}.
     page: a command of the page (host mode table) has the key; absent: the key belongs to a command that
     host mode leaves out (group: the name of its group in HOST_ABSENT, "" when it is in no group); none:
     no command has it. */
  function classify(ev) {
    var out = { kind: "none", key: null, id: null, group: null };
    var K = window.DSKeys;
    if (!K || typeof K.keyOf !== "function" || typeof K.find !== "function" || !K.COMMANDS) return out;
    var key;
    try { key = K.keyOf(ev); } catch (e) { return out; }
    out.key = key;
    if (!key) return out;
    var hit = null;
    try { hit = K.find(ev); } catch (e) { /* none */ }
    if (hit) { out.kind = "page"; out.id = hit.cmd && hit.cmd.id || null; return out; }
    var owned = lex("typeof hostOwned === 'function' ? hostOwned : null");
    var groups = lex("typeof HOST_ABSENT === 'object' ? HOST_ABSENT : null") || [];
    if (typeof owned !== "function") return out;
    for (var i = 0; i < K.COMMANDS.length; i++) {
      var c = K.COMMANDS[i];
      var keys = [];
      try { if (!owned(c.id)) continue; keys = K.keysOf(c, K.preset); } catch (e) { continue; }
      if (keys.indexOf(key) < 0) continue;
      var g = null;
      for (var j = 0; j < groups.length; j++) if (groups[j].menu && groups[j].menu.indexOf(c.id) >= 0) { g = groups[j]; break; }
      out.kind = "absent"; out.id = c.id; out.group = g ? g.group : "";
      return out;
    }
    return out;
  }


  function editable(el) {
    return !!el && (el.tagName === "TEXTAREA" || el.isContentEditable || (el.tagName === "INPUT" && !/^(checkbox|radio|button|range|submit)$/.test(el.type)));
  }
  function typing(ev) {   // a key typed into a text field is the field's, with Cmd or Control it is a command
    return editable(ev.target) && !ev.metaKey && !ev.ctrlKey;
  }

  var shown = [];   // for the check: what was said, and for which key
  var last = null;  // the last keydown: {ev, t}
  /* The plugin asks (src/keys.rs, after a key with Command) what the key meant for a text field of the page: the
     web view runs the action of the menu item that an app would have. Only when a text field has the focus and no handler
     of the page used the key. */
  function edit() {
    var l = last;
    if (!l || Date.now() - l.t > 2000 || l.ev.defaultPrevented) return "";
    var k = String(l.ev.key || "").toLowerCase();
    // selected text that is not in a field (if the page lets anything be selected: it stops selectstart outside fields) copies too
    if (k === "c" && !editable(l.ev.target)) { try { return String(window.getSelection()).length ? "copy" : ""; } catch (e) { return ""; } }
    if (!editable(l.ev.target)) return "";
    return k === "c" ? "copy" : k === "x" ? "cut" : k === "v" ? "paste" : "";
  }
  window.__DS_KEYS__ = { classify: classify, shown: shown, edit: edit };
  window.addEventListener("keydown", function (ev) {
    var test = window.__DS_TEST__;
    // a held space bar repeats key downs: each would play or stop the host. Only the first counts (a text field keeps its repeats).
    if (ev.code === "Space" && ev.repeat && !typing(ev) && !(test && test.fault === "no-repeat-guard")) { ev.preventDefault(); ev.stopImmediatePropagation(); return; }
    last = { ev: ev, t: Date.now() };
    var cls = classify(ev);
    setTimeout(function () {   // the dispatch has ended: a handler of the page that used the key has said so
      if (ev.defaultPrevented) return;
      // the menu items of an app that a text field needs (the host's menu cannot give them any more): undo, redo and select all
      if (editable(ev.target) && ev.metaKey && !ev.ctrlKey && !ev.altKey) {
        var k = String(ev.key || "").toLowerCase();
        try {
          if (test && test.fault === "no-field-undo") { /* the check's fault run */ }
          else if (k === "z") document.execCommand(ev.shiftKey ? "redo" : "undo");
          else if (k === "a") document.execCommand("selectAll");
        } catch (e) { /* none */ }
      }
      if (test && test.fault === "no-key-text") return;   // the check's fault run
      if (cls.kind !== "absent" || ev.repeat || typing(ev)) return;
      var say = lex("typeof hostAbsent === 'function' ? hostAbsent : null");
      if (typeof say !== "function") return;
      say(cls.group || "");
      if (shown.length < 200) shown.push({ at: Date.now(), key: cls.key, id: cls.id, group: cls.group });
    }, 0);
  }, true);
})();
