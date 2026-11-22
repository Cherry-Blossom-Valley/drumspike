/* Drum Spike - the page in a DAW shows song time (DAW plugin M1b8, #555)
   In host mode the timeline of the page is the song time of the DAW. One row of the page is one DAW track (one region sequence of ARA):
   the plugin gives the rows as {id, name, srcs, clips: [{src, a, b, s, k}]} (the shell command plugin_host_regions and the page event
   plugin-regions; plugins/ara/src/rows.rs). A clip says: the slice [a, b) of the source `src` (seconds) lies in the song from `s`,
   stretched by `k`. A hit with source time t has song time s + (t - a) * k (the formula of to_song of the export); a point at song time
   x in a clip has source time a + (x - s) / k. The design is docs/design/daw-plugin/m1b8-song-time.md. In short:
   - Display only. The project, the events, the undo steps and every call of the backend keep SOURCE time; only the drawing and the
     input convert, through this file. Nothing here writes a hit time.
   - A row is drawn by the DOM row of its home source (the first source of the row that is not the home of an earlier row). The DOM rows
     of the other sources of the row are hidden (.hostMerged) and skipped by every key that moves the selection, as the rows that the
     DAW hides are (hostHidden, #562).
   - With the mode off, or with no rows (a host with no regions), HT.on() is false and every guarded path of the page is the path of today.
   The functions are pure (they read S only for the tracks that exist); the test driver of the plugin (src/editor/driver.js) replaces
   some of them to make the fault runs of the check (DS_ARA_SONG_FAULT). */
"use strict";

const HT = {
  rows: [],          // the rows as the plugin gave them
  gen: 0,            // the change counter of the plugin
  fault: "",         // the fault runs of the check (DS_ARA_SONG_FAULT, set by the test driver of the plugin, never in a delivery build): leak (the mode is on without the host-mode switch), source-x (hits at source time), no-cut-start (the mapping ignores a, where a clip starts in its source), home-only (a row draws and edits its first source only), store-song (an added hit is stored with its song time), selected-track (the follow maps through the first clip of the selected row)
  view: [],          // [{row, home, srcs, clips}] for the rows that have a home source in this project
  homeOf: new Map(), // source key -> its entry (the row it is the home of)
  inRow: new Map(),  // source key -> the first entry that holds it (home or not)
  merged: new Set(), // sources that are in a row and the home of none: their DOM rows are hidden
  dropped: 0,        // rows that no DOM row can show (every source of the row is the home of an earlier row)
};

HT.on = () => (hostMode() || HT.fault === "leak") && HT.view.length > 0;

/* Take the answer of plugin_host_regions or the payload of plugin-regions. Returns true when the rows changed. */
HT.apply = function (p) {
  if (!p || typeof p !== "object" || !Array.isArray(p.rows)) return false;
  if (Number.isFinite(p.gen) && p.gen < HT.gen) return false;   // an older message
  const fin = Number.isFinite;
  const rows = [];
  for (const r of p.rows) {
    if (!r || typeof r.id !== "string" || !Array.isArray(r.srcs) || !Array.isArray(r.clips)) continue;
    const clips = r.clips.filter(c => c && typeof c.src === "string" && fin(c.a) && fin(c.b) && fin(c.s) && fin(c.k) && c.b > c.a && c.k > 0)
      .map(c => ({ src: c.src, a: c.a, b: c.b, s: c.s, k: c.k }));
    rows.push({ id: r.id, name: typeof r.name === "string" ? r.name : r.id, srcs: r.srcs.filter(x => typeof x === "string"), clips });
  }
  if (Number.isFinite(p.gen)) HT.gen = p.gen;
  const same = JSON.stringify(rows) === JSON.stringify(HT.rows);
  HT.rows = rows;
  HT.rebuild();
  return !same;
};

/* The rows for the tracks that this project has. A source is the home of one row only. */
HT.rebuild = function () {
  const have = n => typeof S === "undefined" || !S.tracks || !Object.keys(S.tracks).length || !!S.tracks[n];
  HT.view = []; HT.homeOf = new Map(); HT.inRow = new Map(); HT.merged = new Set(); HT.dropped = 0;
  for (const row of HT.rows) {
    const srcs = row.srcs.filter(have), clips = row.clips.filter(c => have(c.src));
    if (!srcs.length) continue;
    const home = srcs.find(n => !HT.homeOf.has(n));
    if (home === undefined) { HT.dropped++; continue; }
    const e = { row, id: row.id, name: row.name, home, srcs, clips };
    HT.view.push(e);
    HT.homeOf.set(home, e);
    for (const n of srcs) if (!HT.inRow.has(n)) HT.inRow.set(n, e);
  }
  for (const n of HT.inRow.keys()) if (!HT.homeOf.has(n)) HT.merged.add(n);
};

/* The clips that the lane of the DOM row `name` draws. A source that no row holds (a round has not made its row yet) is one clip with the
   identity mapping. A hidden (merged) source has none. null when the mode is off: the caller keeps the old code. */
HT.clips = function (name) {
  if (!HT.on()) return null;
  const e = HT.homeOf.get(name);
  if (e) return HT.fault === "home-only" ? e.clips.filter(c => c.src === name) : e.clips;
  if (HT.merged.has(name)) return [];
  const t = typeof S !== "undefined" && S.tracks && S.tracks[name];
  return t ? [{ src: name, a: 0, b: t.duration || 0, s: 0, k: 1 }] : [];
};
HT.isMerged = name => HT.on() && HT.merged.has(name);
/* The sources of the row of the DOM row `name`: where a setting of the row goes (the row owns role, note, parameters and mix). */
HT.srcsOf = name => { const e = HT.on() && HT.homeOf.get(name); return e ? e.srcs : [name]; };
/* The DOM row (the home source) that draws the source `src`, for a click that names a source (a hit, the DAW selection). */
HT.homeFor = src => { const e = HT.on() && HT.inRow.get(src); return e ? e.home : src; };
/* The name of the DAW track of a source (the row it is in), for the texts of the page; null with the mode off or for no row. */
HT.label = src => { const e = HT.on() && HT.inRow.get(src); return e ? e.name : null; };

HT.toSong = (c, t) => c.s + (t - (HT.fault === "no-cut-start" ? 0 : c.a)) * c.k;
HT.songLen = c => (c.b - c.a) * c.k;
/* A point at song time x in the clips: {t, clip} of the LAST clip that holds it (a later clip lies over an earlier one), or null. */
HT.toSource = function (clips, x) {
  for (let i = clips.length - 1; i >= 0; i--) {
    const c = clips[i];
    if (x >= c.s && x < c.s + HT.songLen(c)) return { t: c.a + (x - c.s) / c.k, clip: c };
  }
  return null;
};
/* The end of the last clip in the song, 0 with no rows. */
HT.songEnd = () => HT.view.reduce((m, e) => e.clips.reduce((m2, c) => Math.max(m2, c.s + HT.songLen(c)), m), 0);

/* The hits of the clips of a DOM row inside the song window [t0, t1]: [{src, i, t, clip}] with t the song time. `skip` is the dragged hit
   {track, index} of the page (hitDrag): hitsIn() keeps it in order, so it is passed on for its own source. */
HT.hitRefs = function (clips, events, t0, t1, skip) {
  const out = [];
  for (const c of clips) {
    const evs = events[c.src];
    if (!evs || !evs.length) continue;
    const a = Math.max(c.a, c.a + (t0 - c.s) / c.k), b = Math.min(c.b, c.a + (t1 - c.s) / c.k);
    if (!(b >= a)) continue;
    const sk = skip && skip.track === c.src ? skip.index : -1;
    for (const i of hitsIn(evs, a, b, sk)) {
      const t = evs[i].t;
      if (t < c.a || t >= c.b) continue;   // [a, b) of the clip
      out.push({ src: c.src, i, t: HT.fault === "source-x" ? t : HT.toSong(c, t), clip: c });
    }
  }
  return out;
};
/* The song time of a hit of a source: through the first clip (in the order of the song) of any row that holds it; null when no clip does. */
HT.songTime = function (src, t) {
  for (const e of HT.view) for (const c of e.clips) if (c.src === src && t >= c.a && t < c.b) return HT.toSong(c, t);
  return null;
};
/* Times of one source each ({src: [t, ...]}) in the song: the ghosts of the Sensitivity drag. */
HT.mapTimes = function (clips, bySrc) {
  const out = [];
  for (const c of clips) for (const t of bySrc[c.src] || []) if (t >= c.a && t < c.b) out.push(HT.toSong(c, t));
  return out;
};
/* The number of hits that the clips of the row hold. */
HT.hitCount = function (name, events) {
  const clips = HT.clips(name);
  let n = 0;
  if (clips) for (const c of clips) for (const e of events[c.src] || []) if (e.t >= c.a && e.t < c.b) n++;
  return n;
};
/* The hits of a source that lie in no clip of any row that holds it (they stay stored; the page counts them). */
HT.outside = function (src, events) {
  let n = 0;
  const clips = HT.view.flatMap(e => e.clips).filter(c => c.src === src);
  for (const e of events[src] || []) if (!clips.some(c => e.t >= c.a && e.t < c.b)) n++;
  return n;
};

/* The DOM: the rows of merged sources are hidden, the header of a row shows the name of the DAW track. Called after the rows changed and
   after the page built its rows again. */
HT.paint = function () {
  if (typeof document === "undefined") return;
  const on = HT.on();
  for (const row of document.querySelectorAll(".track")) {
    const n = row.dataset.name;
    row.classList.toggle("hostMerged", on && HT.merged.has(n));
    const nm = row.querySelector(".tname .tnm");
    if (nm) { const l = typeof trackLabel === "function" ? trackLabel(n) : n; if (nm.textContent !== l) { nm.textContent = l; nm.title = l; } }
    const e = on && HT.homeOf.get(n);
    row.dataset.hostRow = e ? e.id : "";
    row.classList.toggle("hostMany", !!e && e.srcs.length > 1);
  }
};

/* The page learns the rows: from the answer of the plugin after a load, and from the events. With the mode off nothing runs. */
HT.pull = function () {
  if (!hostMode() || typeof B === "undefined" || typeof B.hostRegions !== "function") return Promise.resolve(false);
  return B.hostRegions().then(p => HT.take(p), () => false);
};
HT.take = function (p) {
  const changed = HT.apply(p);
  HT.paint();
  if (changed) { invalidate(F.SIZE | F.VIEW | F.GL); if (typeof updateStatus === "function") updateStatus(); }
  return changed;
};
HT.init = function () {
  if (!hostMode() || HT.ready) return;
  HT.ready = true;
  if (B.listen) B.listen("plugin-regions", p => HT.take(p)).catch(() => {});
  HT.pull();
};
