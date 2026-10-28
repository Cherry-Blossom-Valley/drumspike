/* P3-17: the splash page. The shell (src-tauri/src/splash.rs) sets the version through window.__SPLASH__ and
   sends the status line with window.__splashSet(text) on every tick. No other script. #441: the page reports
   its first painted frame (splash_painted: the 5 s minimum counts from there) and a click or any key
   dismisses the splash early (splash_dismiss). The commands answer nothing and need no data. */
(function () {
  "use strict";
  var info = window.__SPLASH__ || {};
  var ver = document.getElementById("version"), st = document.getElementById("status");
  ver.textContent = info.version ? "Version " + info.version : "";
  window.__splashSet = function (text) { if (typeof text === "string" && st.textContent !== text) st.textContent = text; };
  var tauri = window.__TAURI__ && window.__TAURI__.core;
  if (!tauri || typeof tauri.invoke !== "function") return;   // the test bridge has no shell
  var sent = {};
  function once(id) {
    if (sent[id]) return;
    sent[id] = true;
    try { tauri.invoke(id); } catch (e) { /* the shell went away: nothing to report */ }
  }
  // two frames: the first one is certainly presented, so the report is at (not before) the first paint
  requestAnimationFrame(function () { requestAnimationFrame(function () { once("splash_painted"); }); });
  window.addEventListener("pointerdown", function () { once("splash_dismiss"); });
  window.addEventListener("keydown", function () { once("splash_dismiss"); });
})();
