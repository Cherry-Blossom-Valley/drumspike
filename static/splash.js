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
  var TIMER_MS = 1000;
  var sent = {}, frames = 0, t0 = performance.now();
  function once(id, args) {
    if (sent[id]) return;
    sent[id] = true;
    try { tauri.invoke(id, args); } catch (e) { /* the shell went away: nothing to report */ }
  }
  function painted(source) {
    // #203: what the page saw, for the self-test report (frames run so far, visibility, ms since the script began)
    once("splash_painted", { source: source, frames: frames, visible: document.visibilityState, pageMs: Math.round(performance.now() - t0) });
  }
  // two frames: the first one is certainly presented, so the report is at (not before) the first paint
  function frame(next) { requestAnimationFrame(function () { frames++; next(); }); }
  frame(function () { frame(function () { painted("raf"); }); });
  // #203: a window that is occluded or unattended may run no rAF at all (WKWebView), and the report never came;
  // a timer reports then, so the shell counts the minimum from a known instant instead of from the opening
  setTimeout(function () { painted("timer"); }, TIMER_MS);
  window.addEventListener("pointerdown", function () { once("splash_dismiss"); });
  window.addEventListener("keydown", function () { once("splash_dismiss"); });
})();
