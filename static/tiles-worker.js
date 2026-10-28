/* Tile worker (P3-6b). Owns the WebGL2 canvas of the wave lanes (an OffscreenCanvas the page hands over):
   the peak tiles as textures, the sample windows (a small LRU), and the lane drawing. The page sends raw
   tile bytes, sample windows and lane boxes; it keeps no PCM and no peak arrays. All the logic is in
   lanes-gl.js (LanesGL.host), which tests also run in a page. */
(function (scope) {
  "use strict";
  function boot(S) {
    S.importScripts("lanes-gl.js" + S.location.search);   // the page version in the query string (cache)
    const handle = S.LanesGL.host((msg, transfer) => S.postMessage(msg, transfer || []));
    S.onmessage = ev => handle(ev.data);
    return handle;
  }
  if (typeof WorkerGlobalScope !== "undefined" && scope instanceof WorkerGlobalScope) boot(scope);
  else scope.__tilesWorkerBoot = boot;   // a test page loads this file and calls it with a stand-in scope
})(self);
