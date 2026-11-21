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

  function typing(ev) {   // a key typed into a text field is the field's, with Cmd or Control it is a command
    var el = ev.target;
    var editable = !!el && (el.tagName === "TEXTAREA" || el.isContentEditable || (el.tagName === "INPUT" && !/^(checkbox|radio|button|range|submit)$/.test(el.type)));
    return editable && !ev.metaKey && !ev.ctrlKey;
  }

  var shown = [];   // for the check: what was said, and for which key
  window.__DS_KEYS__ = { classify: classify, shown: shown };
  window.addEventListener("keydown", function (ev) {
    var test = window.__DS_TEST__;
    if (test && test.fault === "no-key-text") return;   // the check's fault run
    var cls = classify(ev);
    if (cls.kind !== "absent" || ev.repeat) return;
    setTimeout(function () {   // the dispatch has ended: a handler of the page that used the key has said so
      if (ev.defaultPrevented || typing(ev)) return;
      var say = lex("typeof hostAbsent === 'function' ? hostAbsent : null");
      if (typeof say !== "function") return;
      say(cls.group || "");
      if (shown.length < 200) shown.push({ at: Date.now(), key: cls.key, id: cls.id, group: cls.group });
    }, 0);
  }, true);
})();
