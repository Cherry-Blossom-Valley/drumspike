/* Drum Spike v2 — tracks (refactor plan step 5.2, R9 A2)
   The track rows: the DOM of the lanes and headers (buildTrackDOM, the selected track), the header drag that
   reorders the tracks, the sensitivity of soft tracks and the follow of another track (P3-18) with the role icons,
   the pointer handlers of the spike and wave lanes and the header context menu (P3-14e), the role of a track
   (0.9.1, setTrackRole, renameTrack) and Track > Remove Track (P4-39a). Moved verbatim from app.js. Plain script
   (no build step), loaded after inspector.js and before app.js. The names stay top-level bindings (page globals). */
"use strict";

/* ---------------- track DOM ---------------- */
/* #539: the empty state of host mode. No button works there (the host owns the files and the track set), so the text says
   what to do in the DAW; while the first analysis runs it says that it runs. */
const HOST_EMPTY = "No drum tracks yet. In your DAW, put the Drum Spike plugin on a drum track.";
const HOST_EMPTY_BUSY = "Analyzing the drum tracks. The hits appear here when it is done.";
function hostEmptyText() {
  if (typeof HST !== "undefined" && HST.ready && !HST.absent) return HST.error || HST.docText() || "Checking the tracks of your DAW…";   // #578: the plugin says what happens and what to do
  return analysisRunning ? HOST_EMPTY_BUSY : HOST_EMPTY;
}
function hostEmptyRefresh() {   // the text follows the analysis state (setAnalysisButtons)
  const el = document.querySelector(".empty-state.hostEmpty");
  if (el && el.textContent !== hostEmptyText()) el.textContent = hostEmptyText();
}
function renderHostEmpty(scroll) {
  if (typeof HST !== "undefined" && HST.ready && HST.hasRows()) { HST.paint(); return; }   // #578: the cards of the DAW tracks say what happens
  const box = document.createElement("div"); box.className = "empty-state hostEmpty";
  box.textContent = hostEmptyText();
  scroll.appendChild(box);
}
function renderNoProject(scroll) {
  if (hostMode()) { renderHostEmpty(scroll); return; }   // #539
  const box = document.createElement("div"); box.className = "empty-state noProject";
  const p = document.createElement("div"); p.textContent = "No project is open."; box.appendChild(p);
  const acts = document.createElement("div"); acts.className = "esActs";
  const bNew = document.createElement("button"); bNew.id = "esNew"; bNew.className = "btn primary"; bNew.textContent = "New project…"; bNew.onclick = fileNew;
  const bOpen = document.createElement("button"); bOpen.id = "esOpen"; bOpen.className = "btn"; bOpen.textContent = "Open…"; bOpen.onclick = fileOpen;
  acts.append(bNew, bOpen); box.appendChild(acts);
  const title = document.createElement("div"); title.className = "ptitle"; title.textContent = "Open Recent"; box.appendChild(title);
  const list = document.createElement("div"); list.id = "esRecentList"; box.appendChild(list);
  const clear = document.createElement("button");   // #441: Clear Recent next to the list
  clear.id = "esRecentClear"; clear.className = "btn esClear"; clear.textContent = "Clear Recent";
  clear.onclick = clearRecentProjects;
  box.appendChild(clear);
  scroll.appendChild(box);
  renderRecentList();   // fills #esRecentList as well
}
/* P3-6b: a removed canvas keeps its backing store until the garbage collector finds it, and the collector does not see
   that memory: three re-opens of the 30-track project raised the footprint of the WebContent process by 380 MB.
   Size 0 gives the store back at once. */
function releaseCanvases(root) { root.querySelectorAll("canvas").forEach(c => { c.width = 0; c.height = 0; }); }
function buildTrackDOM() {
  const scroll = $("#lanesScroll"); releaseCanvases(scroll); scroll.innerHTML = "";
  MT.tracks = {};   // P3-16: the meters of the old rows are gone
  if (!S.sid) { renderNoProject(scroll); return; }
  if (!S.order.length && hostMode()) { renderHostEmpty(scroll); return; }   // #539
  if (!S.order.length) {
    scroll.innerHTML = `<div class="empty-state">No tracks yet — drop audio files or a folder here, or click <b>⬆ Add tracks</b> to add kick, snare, toms and overheads.<br>Then hit <b>⚙ Analyze all</b>.</div>`;
    return;
  }
  for (const name of S.order) {
    const t = S.tracks[name];
    const row = document.createElement("div");
    row.className = "track" + (t.offline ? " offline" : ""); row.dataset.name = name;
    t.params = t.params || defaultParams(t.role);   // the Inspector edits these (P3-14a)
    row.innerHTML = `
      <div class="track-row">
        <div class="thead" title="${hostMode() ? "Click: select the track (the Inspector shows it). Right-click: Re-analyze" : "Click: select the track (the Inspector shows it). Drag: reorder. Right-click: Export…, Re-analyze, Reset Volume and Pan"}">
          <span class="tstrip" style="background:${t.color}"></span>
          <div class="tname"></div>
          <div class="tnote">${t.role} · <span class="nnote">${nameFromNote(t.note)}</span> (${t.note})<span class="tfollow"></span></div>
          <div class="tctl">
            <button class="btn mini bMute" title="Mute (Track > Mute Selected Track, Shift+M)">M</button>
            <button class="btn mini bSolo" title="Solo (Track > Solo Selected Track, Shift+S)">S</button>
            <div class="mslider vol" data-ctl="vol"><div class="mfill"></div><div class="mknob"></div></div>
            <div class="mslider pan" data-ctl="pan"><div class="mcenter"></div><div class="mknob"></div></div>
            <span class="mval" data-for="vol"></span>
            <span class="mval mvalPan" data-for="pan"></span>
          </div>
          <div class="meter" title="Level after volume and pan. The red light: clipped (click to reset)">
            <div class="mclip"></div>
            <div class="mbars"></div>
          </div>
          <div class="rowResize" title="Drag: the height of this track's wave lane. Double-click: back to the Track Height"></div>
        </div>
        <div class="tlanes">
          <canvas class="lane spike" height="34"></canvas>
          <canvas class="lane wave"  height="120"></canvas>
          ${VL.on ? velWrapHtml() : ""}
          <div class="rowResize" title="Drag: the height of this track's wave lane. Double-click: back to the Track Height"></div>
        </div>
      </div>`;
    scroll.appendChild(row);
    if (t.follows) { row.querySelector(".tfollow").textContent = ` · follows ${trackLabel(t.follows)}`; row.classList.add("follower"); }   // P3-18: names are data
    const tn = row.querySelector(".tname");   // C1: name as text node — no HTML injection
    const nm = document.createElement("span"); nm.className = "tnm"; nm.textContent = trackLabel(name); nm.title = trackLabel(name);   // #558: a host shows the name of its track; the key stays the name
    tn.appendChild(nm);
    setHeadIcon(row, t.role);   // 0.9.3: the role icon left of the name
    // P3-14c: the header is compact; the 🔗 badge and the hit counts are in the Inspector (source state, Detected, Edited)
    if (t.offline) {
      const b = document.createElement("button"); b.className = "btn mini tOffline"; b.textContent = "offline";
      if (hostMode()) { b.title = `source ${t.status}: the host provides the audio`; b.disabled = true; }   // #539: no relink in host mode
      else { b.title = `source ${t.status}: click to relink the audio file`; b.onclick = () => relinkTrack(name); }
      tn.appendChild(b);
    }
    row.querySelector(".thead").addEventListener("click", ev => {
      if (headerDragged) { headerDragged = false; return; }   // drag-release, not a click
      if (ev.target.closest("button") || ev.target.tagName === "INPUT" || ev.target.tagName === "SELECT") return;
      if (ev.target.closest(".mslider, .meter, .rowResize")) return;   // P3-16: the mixer controls; #439: the height handle
      setSelTrack(name);   // P4-39c: a second click keeps the track selected
    });
    row.querySelector(".thead").addEventListener("contextmenu", ev => { ev.preventDefault(); openTrackMenu(name, ev.clientX, ev.clientY); });   // P3-14e
    setupTrackDrag(row);   // Pro Tools style: drag the header up/down to reorder
    attachTrackHandlers(row, name);
    vlBind(row, name);   // P4-40f: the velocity lane's gestures
    bindRowResize(row, name);   // #167: drag the lower border of the row, double-click resets it
    bindRowResize(row, name, row.querySelector(".thead .rowResize"));   // #439: the header's bottom edge drags the same height
    bindTrackMix(row, name);   // P3-16
  }
  refreshCounts();
  document.querySelectorAll(".track").forEach(r => r.classList.toggle("selTrack", r.dataset.name === S.selTrack));   // P4-39c: the first track is selected at open
  if (typeof HST !== "undefined") HST.paint();   // #578: the states of the first open again for the new DOM rows (nothing with the mode off)
  HT.rebuild(); HT.paint();   // #555: the rows of the DAW tracks again for the new DOM rows (nothing with the mode off)
  hostSelReapply();   // #562: the rows are new: the marks and the hidden tracks of the DAW selection again (nothing with the mode off)
  syncInspector(true);   // P3-14a: the rows are new; the Inspector shows the selected track or its empty state
  sizeCanvases();   // P3-14d: the new canvases get their CSS size now (from GEO: no layout read); the frame draws them
  invalidate(F.VIEW | F.NEAR);
}
/* P4-39c: a project with tracks always has a selected track (Inspector, key commands, Learn, follow menus). Clicks on
   empty lane space, Escape and lnSetSel(null) clear the hit selection only. A selected track that leaves (removed, hidden)
   passes the selection to the next track in the order it had, else the previous one. */
var selMem = { sid: null, order: [] };   // the project and track order the selection was last checked against
function pickSelTrack(oldOrder, sel) {
  if (sel && S.tracks[sel]) return sel;
  const i = oldOrder.indexOf(sel);
  if (i >= 0) {
    for (let j = i + 1; j < oldOrder.length; j++) if (S.tracks[oldOrder[j]]) return oldOrder[j];
    for (let j = i - 1; j >= 0; j--) if (S.tracks[oldOrder[j]]) return oldOrder[j];
  }
  return S.order.find(n => S.tracks[n]) || null;
}
function ensureSelTrack() {   // in updateStatus: a lost selection moves on; the order is remembered
  if (S.sid !== selMem.sid) selMem = { sid: S.sid, order: [] };
  if (!S.selTrack || !S.tracks[S.selTrack]) {
    const n = pickSelTrack(selMem.order, S.selTrack);
    if (n) { setSelTrack(n); return; }
  }
  const o = S.order;
  if (o.length !== selMem.order.length || o.some((n, i) => n !== selMem.order[i])) selMem.order = o.slice();
}
function setSelTrack(name) {   // the header click and the keys P and ; (P3-19)
  if (!name || !S.tracks[name]) name = pickSelTrack(selMem.order, S.selTrack);   // P4-39c: never none while tracks exist
  if (name) name = HT.homeFor(name);   // #555: a source that is merged into a DAW track selects the row that shows it, never a hidden row
  S.selTrack = name;
  document.querySelectorAll(".track").forEach(r => r.classList.toggle("selTrack", r.dataset.name === S.selTrack));
  invalidate(F.HEAD);   // P3-14b: the sub counter uses the rate of the selected track
  updateStatus();
}
/* ---------------- track reordering (drag header, live reorder like Pro Tools) ---------------- */
let headerDragged = false;
function applyOrder(newOrder) {
  if (hostAbsent("tracks")) return;   // #539: the host owns the order of the tracks
  S.order = newOrder;
  const scroll = $("#lanesScroll");
  for (const n of newOrder) {
    const r = document.querySelector(`.track[data-name="${n}"]`);
    if (r) scroll.appendChild(r);   // moves existing DOM nodes — canvas pixels survive
  }
  B.reorder(S.sid, newOrder)
    .then(() => { setMsg("track order saved"); setDirtyState(true); })
    .catch(e => setMsg(`reorder failed: ${e.message}`, "error"));
  invalidate(F.VIEW);   // the overview shows the tracks in this order; the rows' places come from the observer
}
function setupTrackDrag(row) {
  const thead = row.querySelector(".thead");
  thead.addEventListener("mousedown", ev => {
    if (ev.button !== 0 || hostMode()) return;   // #539: no reorder in host mode
    if (ev.target.closest("button") || ["INPUT","SELECT"].includes(ev.target.tagName)) return;
    ev.preventDefault();   // no text selection while grabbing the header
    const startY = ev.clientY;
    let dragging = false;
    const scroll = $("#lanesScroll");
    const mv = ev2 => {
      if (!dragging && Math.abs(ev2.clientY - startY) > 5) {
        dragging = true; headerDragged = true;
        row.classList.add("dragging");
        document.body.classList.add("draggingTrack");
      }
      if (!dragging) return;
      // auto-scroll when dragging near the list edges
      const sr = scroll.getBoundingClientRect();
      if (ev2.clientY < sr.top + 48) scroll.scrollTop -= 14;
      else if (ev2.clientY > sr.bottom - 48) scroll.scrollTop += 14;
      // live reorder: the dragged row slips between neighbors as it crosses their midpoints
      const rows = [...scroll.querySelectorAll(".track")].filter(r => r !== row);
      for (let i=0; i<rows.length; i++) {
        const rc = rows[i].getBoundingClientRect();
        if (ev2.clientY < rc.top + rc.height/2) { scroll.insertBefore(row, rows[i]); break; }
        if (i === rows.length-1 && ev2.clientY > rc.bottom - rc.height/2) scroll.appendChild(row);
      }
    };
    const up = () => {
      window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up);
      row.classList.remove("dragging");
      document.body.classList.remove("draggingTrack");
      if (dragging) {
        const newOrder = [...scroll.querySelectorAll(".track")].map(r => r.dataset.name);
        if (newOrder.join("\u0000") !== S.order.join("\u0000")) applyOrder(newOrder);
      }
    };
    window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
  });
}
function refreshCounts(track) {   // the header counts of one track, or of every track
  for (const name of track ? [track] : S.order) {
    const row = rowOf(name);
    if (!row) continue;
  }
  if (INS.name && (!track || track === INS.name)) { insCounts(); hgScheduleRefresh(track); }   // P3-14a; P4-3: the histograms follow an edit
}
function defaultParams(role) {
  return { thresholds: [0.22,0.24,0.32,0.22,0.30], min_gap_ms: 20, refine: true,
           vel_lo_db: -40, vel_hi_db: -6, click_freq: ROLE_DEFAULT_CLICK[role] || 1500 };
}

/* ---------------- P3-18: sensitivity and soft tracks ----------------
   The Sensitivity slider maps a value 0 to 150 to the five class thresholds; crates/app/src/sensitivity.rs has the same
   formulas (the service's 0-hit retry uses them). 0 to 100 is the mapping of P3-14a (1.6 - 1.2 v/100, so an old project
   gives the same thresholds back; its end 0.088 to 0.128), 100 to 150 goes on geometrically down to 0.01 for the kick.
   The readout under the slider shows the lowest and the highest threshold. A track whose analysis gave no hit got its
   sensitivity raised by the service (params.weak_signal {from, to}); the Inspector and the status line say so.
   A track can follow another track (S.tracks[n].follows): its hits are the source's, derived by the service, read-only
   here (followerLocked); the header and the Inspector show "follows <name>". */
const SENS = { BASE: [0.22,0.24,0.32,0.22,0.30], DEFAULT: 50, OLD_MAX: 100, MAX: 150, FLOOR: 0.01, CEIL: 0.85 };
function sensScale(v) {
  v = Number.isFinite(v) ? Math.min(SENS.MAX, Math.max(0, v)) : SENS.DEFAULT;
  if (v <= SENS.OLD_MAX) return 1.6 - 1.2*(v/100);   // the operation order of P3-14a: the same doubles
  return 0.4 * Math.pow((SENS.FLOOR/0.22)/0.4, (v - SENS.OLD_MAX)/(SENS.MAX - SENS.OLD_MAX));
}
function sensThresholds(v) { const s = sensScale(v); return SENS.BASE.map(b => Math.min(SENS.CEIL, Math.max(SENS.FLOOR, b*s))); }
function sensOf(th) {   // the slider value of a track's thresholds (the mean against the mean 0.26 of the base)
  const mean = th.reduce((a,b)=>a+b,0)/th.length, s = mean/0.26;
  const v = s >= 0.4 ? (1.6 - s)/1.2*100 : SENS.OLD_MAX + (SENS.MAX - SENS.OLD_MAX)*Math.log(0.4/s)/Math.log(0.4/(SENS.FLOOR/0.22));
  return Number.isFinite(v) ? Math.min(SENS.MAX, Math.max(0, v)) : SENS.MAX;
}
function sensReadout(v) { const th = sensThresholds(v); return `thresholds ≈ ${Math.min(...th).toFixed(2)} … ${Math.max(...th).toFixed(2)}`; }
function weakSignalText(w) {   // params.weak_signal or a job entry's weak_signal
  if (!w) return "";
  return w.to === null || w.to === undefined ? `no hits found (sensitivity up to ${SENS.MAX} tried)` : `weak signal: sensitivity raised to ${w.to} (from ${w.from})`;
}
function weakSignals(tracks) {   // the job's tracks: [[name, text], ...] for the status line
  return Object.entries(tracks || {}).filter(([, t]) => t.state === "done" && t.weak_signal).map(([n, t]) => [n, `${n}: ${weakSignalText(t.weak_signal)}`]);
}
function followersOf(name) { return S.order.filter(n => S.tracks[n] && S.tracks[n].follows === name); }
/* 0.9.3: the role icon of the track header (inline SVG, no image files, no external fonts).
   #9: the instrument drawings of the header, 48x48. */
const ROLE_ICONS = {
  // #9, review 2: 3/4-view instrument drawings, 48x48, stroke currentColor with a light
  // same-colour fill; the header element gets the track colour (setHeadIcon).
  // kick: the round front head with tension rods, the spurs and the pedal.
  kick: '<g fill="none"><circle cx="24" cy="16" r="12"/><circle cx="24" cy="16" r="12" fill="currentColor" fill-opacity=".12"/><path d="M24 1.5v3M24 27.5v3M9 16h3M36 16h3M12.7 4.7l2.1 2.1M33.2 4.7l-2.1 2.1M12.7 27.3l2.1-2.1M33.2 27.3l-2.1-2.1"/><path d="M13 25.5 8.5 33M35 25.5l4.5 7.5"/><path d="M24 28v7M24 35h12M28 35v4"/><circle cx="24" cy="30.5" r="1.8" fill="currentColor" stroke="none"/></g>',
  // snare: short and wide shell, the top head as an ellipse, lugs, hoop, tripod.
  snare: '<g fill="none"><path d="M8 15 10.5 26M40 15 37.5 26"/><path d="M10.5 26A14 4.5 0 0 0 37.5 26" /><ellipse cx="24" cy="15" rx="16" ry="5" fill="currentColor" fill-opacity=".12"/><ellipse cx="24" cy="15" rx="13.5" ry="4"/><path d="M11 21h4M33 21h4M17 22.5h3M28 22.5h3"/><path d="M24 29v4M24 33l-8 8M24 33l8 8"/></g>',
  // tom: the deeper floor tom on three straight legs.
  tom: '<g fill="none"><path d="M9 10 11.5 32M39 10 36.5 32"/><path d="M11.5 32A13 4.5 0 0 0 36.5 32"/><ellipse cx="24" cy="10" rx="15" ry="4.5" fill="currentColor" fill-opacity=".12"/><ellipse cx="24" cy="10" rx="12.5" ry="3.5"/><path d="M12 17h4M32 17h4M13.5 24h3.5M31 24h3.5"/><path d="M12 33l-2.5 9M24 33v9M36 33l2.5 9"/></g>',
  // hat: two cymbals on a stand, the footboard of the pedal.
  hat: '<g fill="none"><ellipse cx="24" cy="14" rx="14" ry="4" fill="currentColor" fill-opacity=".12"/><ellipse cx="24" cy="20" rx="11.5" ry="3.2"/><path d="M24 10.5V6M24 23v10M24 33l-9 9M24 33l9 9"/><path d="M15 42h9"/></g>',
  // cymbal: a tilted ellipse with a bell on a stand (the crash/ride of the overheads).
  cymbal: '<g fill="none"><g transform="rotate(-10 24 15)"><ellipse cx="24" cy="15" rx="17" ry="4.5" fill="currentColor" fill-opacity=".12"/><ellipse cx="24" cy="15" rx="17" ry="4.5"/><circle cx="24" cy="13" r="2.6"/></g><path d="M24 20v13M24 33l-9 9M24 33l9 9"/></g>',
  // other: the microphone (room, clap and every role without its own drum).
  other: '<g fill="none"><rect x="18" y="4" width="12" height="17" rx="6" fill="currentColor" fill-opacity=".12"/><rect x="18" y="4" width="12" height="17" rx="6"/><path d="M20 9h8M20 13h8M20 17h8M14 15v3a10 10 0 0 0 20 0v-3M24 28v9M15 40h18"/></g>',
};
function roleIconEl(role) {   // a role name is a fixed key, never data: the markup is ours
  const s = document.createElement("span"); s.className = "tart";
  s.innerHTML = `<svg viewBox="0 0 48 48" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${ROLE_ICONS[role] || ROLE_ICONS.other}</svg>`;
  s.title = `role: ${role}`;
  return s;
}
function setHeadIcon(row, role) {   // the drawing sits right-aligned in the header's free area, in the track colour
  const th = row.querySelector(".thead"); if (!th) return;
  const cur = th.querySelector(".tart"); if (cur) cur.remove();
  const s = roleIconEl(role);
  const t = S.tracks[row.dataset.name];   // #9 review 2: the drawing takes the track colour
  if (t && t.color) s.style.color = t.color;
  th.appendChild(s);
}
function roleLineText(t) { return `${t.role} · ${nameFromNote(t.note)} (${t.note})` + (t.follows ? ` · follows ${trackLabel(t.follows)}` : ""); }
function followerLocked(name) {   // true (with the message) when the track's hits are derived from another track
  const t = S.tracks[name];
  if (!t || !t.follows) return false;
  setMsg(`${trackLabel(name)} follows ${trackLabel(t.follows)}: its hits are derived, edit the hits of ${trackLabel(t.follows)}`);
  return true;
}
function refreshHeadRole(name) {   // the role line of the header, and the Inspector's when it shows the track
  const row = rowOf(name), t = S.tracks[name];
  if (row && t) {
    setHeadIcon(row, t.role);   // 0.9.3: a role change swaps the icon at once
    row.querySelector(".nnote").textContent = nameFromNote(t.note);
    row.querySelector(".tfollow").textContent = t.follows ? ` · follows ${trackLabel(t.follows)}` : "";   // textContent: a track name is data
    row.classList.toggle("follower", !!t.follows);
  }
  if (name === INS.name) insRoleLine();
}
async function reloadFollowers(names) {   // the service derived the followers' events again (an edit or an analysis of the source)
  for (const f of names || []) {
    if (!S.tracks[f] || !S.sid) continue;
    S.events[f] = normEvents(await B.events(S.sid, f));
    if (S.sel && S.sel.track === f && S.sel.index >= S.events[f].length) S.sel = null;
    invalidateHits(f); refreshCounts(f);
  }
}
async function refreshTrackMeta() {   // after an analysis: the params the retry changed and the follows, without a DOM rebuild
  if (!S.sid) return;
  const gen = loadGen();
  try {
    const r = await B.tracks(S.sid);
    if (gen !== loadGen()) return;
    for (const [n, t] of Object.entries(r.tracks || {})) {
      if (!S.tracks[n]) continue;
      if (t.params) S.tracks[n].params = t.params;
      S.tracks[n].follows = t.follows || null; S.tracks[n].n_events = t.n_events;
      if (t.layer) { S.tracks[n].layer = t.layer; S.tracks[n].layer_status = t.layer_status; } else { delete S.tracks[n].layer; delete S.tracks[n].layer_status; }   // P4-7c: agent commands change them too
      if (t.sampler) S.tracks[n].sampler = t.sampler; else delete S.tracks[n].sampler;
      refreshHeadRole(n);
    }
  } catch (e) { /* the next project load shows them */ }
  if (INS.name) syncInspector(true);
}
/* Inspector "Hits" and Track > Follow Hits Of / Own Detection: the track takes the hits of `src`, or detects itself */
async function setTrackFollow(name, src) {
  if (!S.tracks[name]) return false;
  if (analysisRunning) { setMsg("analysis running: change the follow after it"); return false; }
  src = src || null;
  if (src && !S.tracks[src]) { setMsg(`no track named ${src}`); return false; }
  try { await flushEvents(); }
  catch (e) { setMsg(`follow failed: ${errText(e)}`, "error"); return false; }   // #249 R02: the follow must not take the service's old hits
  try {
    await B.setFollow(S.sid, name, src);
    S.tracks[name].follows = src;
    S.events[name] = normEvents(await B.events(S.sid, name));
  } catch (e) { setMsg(`follow failed: ${errText(e)}`, "error"); if (name === INS.name) syncInspector(true); return false; }
  if (S.sel && S.sel.track === name) S.sel = null;
  setDirtyState(true); refreshHeadRole(name); invalidateHits(name); refreshCounts(name); updateStatus();
  if (name === INS.name) syncInspector(true);
  DSShell.sync();   // the check marks of Track > Follow Hits Of
  if (src) {   // #411: one hit reads "1 hit", not "1 hits"
    const n = S.events[name].length;
    setMsg(`${trackLabel(name)} follows ${trackLabel(src)}: ${n} ${n === 1 ? "hit" : "hits"}, velocities from its own signal`);
  } else {
    setMsg(`${trackLabel(name)}: own detection (re-analyze to detect its hits)`);
  }
  return true;
}
function followSelected(src) { const n = needTrack(); if (n) return setTrackFollow(n, src); }

/* ---------------- track handlers ---------------- */
/* P3-14d: the x of a pointer in a lane (the ruler, the overview) from clientX and GEO: MouseEvent.offsetX lays the
   page out when it is dirty, and a window listener's offsetX is relative to whatever element is under the pointer */
const laneXOf = ev => ev.clientX - GEO.laneX;
const rulerXOf = ev => ev.clientX - GEO.rulerX;
const ovXOf = ev => ev.clientX - GEO.ovX;

function attachTrackHandlers(row, name) {
  const spike = row.querySelector(".spike"), wave = row.querySelector(".wave");
  const evs = () => S.events[name] || [];
  /* #555: in a DAW this row is a DAW track and shows the clips of its sources in song time. The hit under the pointer is {src, i, clip}: an edit goes
     to the source of the clip, in source time (the stored hits never see song time). Standalone: the hit of this track, src = name, no clip. */
  function hitRef(px) {   // W6: the hits within 4 px (binary search)
    const clips = HT.clips(name), W = laneWidth(), t = xToT(px, W), d = 4 * viewDur() / W;
    if (clips) {
      let best = null, bd = 4;
      for (const r of HT.hitRefs(clips, S.events, t - d, t + d, hitDrag)) { const dx = Math.abs(tToX(r.t, W) - px); if (dx < bd) { bd = dx; best = r; } }
      return best;
    }
    const evsL = evs();
    for (const i of hitsIn(evsL, t - d, t + d, dragSkip(name))) {
      const x = tToX(evsL[i].t, W);
      if (Math.abs(x-px) < 4) return { src: name, i, t: evsL[i].t, clip: null };
    }
    return null;
  }
  const hitEvent = px => { const r = hitRef(px); return r ? r.i : -1; };
  /* the source and the source time of a point of the lane: in a DAW the clip under the pointer decides (the later clip if they overlap); a point in no
     clip is null. Standalone: this track, snapped to the sample. */
  function pointOf(px) {
    const clips = HT.clips(name), W = laneWidth();
    if (!clips) return { src: name, t: snapSample(xToT(px, W), name) };
    const w = HT.toSource(clips, xToT(px, W));
    return w ? { src: w.clip.src, t: snapSample(w.t, w.clip.src), clip: w.clip } : null;
  }
  const addAt = (px, snap) => {   // Option-click and double click add a hit; in a DAW only inside a clip
    const p = pointOf(px);
    if (!p) { setMsg("outside every region of this track: no hit is added"); return; }
    if (HT.fault === "store-song" && p.clip) { addHitAt(p.src, snapSample(xToT(px, laneWidth()), p.src), snap); return; }   // the fault run: the song time is stored
    addHitAt(p.src, p.t, snap, p.clip);
  };
  let dragState = null;
  function startPanGesture(ev) {
    S.followHold = true;   // F1: user view change pauses follow until next play
    const startX = laneXOf(ev), startCX = ev.clientX, t0 = S.view.t0;
    let moved = false;
    const mv = ev2 => {
      const dx = ev2.clientX - startCX;
      if (Math.abs(dx) > 3) moved = true;
      if (moved) {
        const dt = dx / laneWidth() * viewDur();
        setView(t0 - dt, t0 - dt + viewDur());
      }
    };
    const up = () => {
      window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up);
      if (!moved) {
        S.playhead = snapPlayhead(xToT(startX, laneWidth()));
        lnClearSel();   // P4-6a: a click clears the range selection
        invalidate(F.HEAD); updateStatus();
      }
    };
    window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
  }
  /* W7: the end of a hit drag listens on the window only while the drag runs (before P3-14d each track added one
     window listener per project open and never removed it) */
  function endHitDrag() {   // P4-9 stage 2: the drag was a preview on the page's copy; one events_edit move on release
    window.removeEventListener("mouseup", endHitDrag);
    const ds = dragState; dragState = null;
    if (!ds) return;
    const name = ds.src, e = (S.events[name] || [])[ds.i];   // #555: the source of the clip that held the hit
    if (!ds.moved || !e || ds.gen !== (EDGEN[name] || 0) || !hitDrag) { hitDrag = null; invalidateHits(name); if (ds.moved) { updateStatus(); setMsg("the hits changed during the drag: the move was dropped"); } return; }   // void: the list was replaced meanwhile
    const id = e.id, to = e.t, from = ds.origT, i = ds.i;
    edCall(name, evs2 => {   // the preview keeps the hit at the pointer until the stored hits arrive; its index is the stored one
      if ((EDGEN[name] || 0) !== ds.gen || !evs2[i] || evs2[i].id !== id) { if (to !== from) setMsg("the hits changed during the drag: the move was dropped"); return null; }
      if (to === from) return null;
      return { ops: [{ op: "move", index: i, t: to }], drop: [id], make: [[to, id]], sel: id, drag: true, step: { type: "move", track: name, id, from, to, was: hitWas(evs2[i]) } };
    }, () => { e.t = from; }).then(ok => { if (!ok) return edReload(S.sid, name); }).finally(() => { hitDrag = null; invalidateHits(name); updateStatus(); });
  }
  spike.addEventListener("mousedown", ev => {
    if (ev.button === 2) return;
    ev.preventDefault();   // no text selection during event drags / pans
    const px = laneXOf(ev);
    if (ev.altKey && ev.shiftKey) {   // P4-40b: Option+Shift-click on a hit deletes it (one undo step, as Delete); off a hit it stays the Shift click
      const hr = hitRef(px);
      if (hr) { if (!followerLocked(hr.src)) edDelete(hr.src, S.events[hr.src][hr.i].id); return; }
    }
    if (ev.shiftKey) { lnClearSel(); S.playhead = snapPlayhead(xToT(px, laneWidth())); invalidate(F.HEAD); updateStatus(); return; }
    if (ev.altKey) { addAt(px, true); return; }   // P3-13b: Option-click adds a hit (Cubase); #144: on the attack
    const hr = hitRef(px), i = hr ? hr.i : -1, src = hr ? hr.src : name;   // #555: the source of the clip that holds the hit
    if (i < 0) {   // P4-6a: a click on a Learn mark bar selects the mark (Delete removes it)
      const lm = lnMarkAt(name, px, ev.offsetY);
      if (lm) { lnClearSel(); S.sel = null; lnSetMark(lm.src || name, lm.id); updateStatus(); setMsg(`${trackLabel(lm.src || name)}: mark ${lm.id} (${lm.kind === "not" ? "not this" : "this"}, ${(lm.at || []).length} ${(lm.at || []).length === 1 ? "example" : "examples"}); Delete removes it, right-click for more`); return; }
    }
    lnClearSel(); if (LN.mark) lnSetMark(null);
    if (i >= 0 && followerLocked(src)) { S.sel = {track:src, index:i}; invalidateHits(src); updateStatus(); return; }   // P3-18: select, no drag
    if (i >= 0) {
      S.sel = {track:src, index:i};
      if (SH.pending || hgRepickBusy()) { invalidateHits(src); updateStatus(); return; }   // an edit is on its way: select, no drag (the hits on the page are not the stored ones yet)
      if (hasPendingWrite(src)) pushEvents(src).catch(() => {});   // a write of another step goes out before the drag moves the page's copy; #249 R02: a refusal shows on the status line
      dragState = {i, src, clip: hr.clip, startX: px, origT: S.events[src][i].t, moved:false, sr: S.tracks[src].sr, gen: EDGEN[src] || 0};
      hitDrag = { track: src, index: i };
      window.addEventListener("mouseup", endHitDrag);
      invalidateHits(src); updateStatus();
      return;
    }
    startPanGesture(ev);   // empty spike area: click = playhead, drag = pan
  });
  spike.addEventListener("mousemove", ev => {
    const px = laneXOf(ev);
    if (!dragState) {   // P4-40b: the cursor comes from CSS classes (.onHit; the modifier classes of #lanesScroll)
      const on = hitEvent(px) >= 0;
      spike.classList.toggle("onHit", on); curOver(on && !followerLocked(name), ev); return;
    }
    const cl = dragState.clip, dsrc = dragState.src;
    const dt = (xToT(px, laneWidth()) - xToT(dragState.startX, laneWidth())) / (cl ? cl.k : 1);   // #555: a distance in song time is 1/k in the source
    if (Math.abs(px-dragState.startX) > 2) dragState.moved = true;
    const i = dragState.i, evsL = S.events[dsrc] || [];
    if (!evsL[i] || dragState.gen !== (EDGEN[dsrc] || 0)) {   // the list was replaced: the drag is void (and says so, as at release)
      const moved = dragState.moved; dragState = null; hitDrag = null; invalidateHits(dsrc);
      if (moved) setMsg("the hits changed during the drag: the move was dropped");
      return;
    }
    let nt = snapSample(dragState.origT + dt, dsrc);
    if (cl) nt = Math.max(cl.a, Math.min(nt, cl.b - 1 / dragState.sr));   // #555: the hit stays inside its clip
    evsL[i].t = nt;
    invalidateHits(dsrc);
    const dSmp = Math.round((evsL[i].t - dragState.origT)*dragState.sr);
    $("#stSel").textContent = `${trackLabel(name)}: moving ${dSmp>0?"+":""}${dSmp} smp`;
    drawInfo();
  });
  spike.addEventListener("mouseleave", () => { spike.classList.remove("onHit"); curOver(false); });
  spike.addEventListener("dblclick", ev => addAt(laneXOf(ev)));
  spike.addEventListener("contextmenu", ev => {
    const hr = hitRef(laneXOf(ev));
    if (!hr) { lnContext(ev, name, "spike"); ev.preventDefault(); return; }   // P4-6a: the menus of a mark bar and of the selection; no browser menu on an empty lane
    ev.preventDefault();
    if (followerLocked(hr.src)) return;   // P3-18: a follower's hits are not deleted here
    edDelete(hr.src, S.events[hr.src][hr.i].id);   // P4-9 stage 2: events_edit delete
  });
  wave.addEventListener("mousedown", ev => {
    if (ev.button!==0) return;
    ev.preventDefault();
    const px = laneXOf(ev);
    if (ev.shiftKey) { lnClearSel(); S.playhead = snapPlayhead(xToT(px, laneWidth())); invalidate(F.HEAD); updateStatus(); return; }
    if (ev.altKey) { lnClearSel(); addAt(px, true); return; }     // P3-13b: Option-click adds a hit; #144: on the attack
    lnRangeGesture(ev, name);   // P4-6a: click = playhead, drag = select a range (the pan stays on the spike lane, the wheel and the overview)
  });
  wave.addEventListener("mousemove", ev => curOver(false, ev));   // P4-40b
  wave.addEventListener("contextmenu", ev => { lnContext(ev, name, "wave"); });   // P4-6a: inside the selection
  for (const c of [spike, wave]) c.addEventListener("wheel", ev => {
    ev.preventDefault();
    const dY = ev.deltaY * dScale(ev.deltaMode), dX = ev.deltaX * dScale(ev.deltaMode);
    const secPerPx = viewDur() / laneWidth();
    // P4 #145: Cmd+wheel and the trackpad pinch (a ctrl+wheel) zoom around the pointer; one wheel notch (100 px)
    // is a factor of about 1.6, a pinch moves in small steps. nativeFeel stops the page zoom of both.
    if (ev.metaKey || ev.ctrlKey) { zoomAtX(laneXOf(ev), laneWidth(), 2 ** Math.max(-1, Math.min(1, dY / 150))); return; }
    // horizontal swipe or shift+wheel pans the timeline; vertical wheel scrolls the track list
    if (Math.abs(dX) > Math.max(2, Math.abs(dY)) || ev.shiftKey) panView((dX || dY) * secPerPx);
    else $("#lanesScroll").scrollBy({ top: dY });
  }, {passive:false});

  row.querySelector(".bMute").onclick = ev => { S.muted[name] = !S.muted[name]; row.classList.toggle("muted", S.muted[name]); ev.target.classList.toggle("on", S.muted[name]); applyMixer(); };
  row.querySelector(".bSolo").onclick = ev => { S.solo[name] = !S.solo[name]; row.classList.toggle("solo", S.solo[name]); ev.target.classList.toggle("on", S.solo[name]); applyMixer(); };
}

/* P3-14e: the track header context menu (Export…, Re-analyze, Reset Volume and Pan); the Track and
   Hits menus run the same actions for the selected track. P3-14a: Params is gone, the Inspector holds the parameters. */
let ctxAway = null;
function closeTrackMenu() {
  const m = document.getElementById("ctxMenu"); if (m) m.remove();
  if (ctxAway) { document.removeEventListener("mousedown", ctxAway, true); ctxAway = null; }
}
function openTrackMenu(name, x, y) {
  const items = [
    ["export", "Export…", () => openExport("selected", name)],   // P4-30: the one Export dialog with this track
    ["reanalyze", "Re-analyze", () => startAnalysis([name])],
    ["mix", "Reset Volume and Pan", () => { setTrackMix(name, 0, 0); setMsg(`${name}: volume 0 dB, pan centre`); }],   // P3-16
  ].filter(it => !hostMode() || it[0] === "reanalyze");   // #539: in host mode only Re-analyze (the host owns the files and the mix)
  openCtxItems(items, x, y);
}
function openCtxItems(items, x, y) {   // [act, label, run] rows at a point (P4-6a: the Learn menus use it too)
  closeTrackMenu();
  const m = document.createElement("div"); m.id = "ctxMenu";
  for (const [act, label, run] of items) {
    const b = document.createElement("button"); b.dataset.act = act; b.textContent = label;
    if (act === "reanalyze") b.disabled = !!$("#analyzeAllBtn").disabled;
    b.onclick = () => { closeTrackMenu(); run(); };
    m.appendChild(b);
  }
  document.body.appendChild(m);
  m.style.left = Math.max(0, Math.min(x, innerWidth - m.offsetWidth)) + "px";
  m.style.top = Math.max(0, Math.min(y, innerHeight - m.offsetHeight)) + "px";
  ctxAway = ev => { if (!m.contains(ev.target)) closeTrackMenu(); };
  document.addEventListener("mousedown", ctxAway, true);
}
document.addEventListener("keydown", ev => { if (ev.key === "Escape") closeTrackMenu(); }, true);

/* ---------------- 0.9.1: the role of a track (Inspector Track section) ----------------
   The select sets the role through track_set_role: the service records the role and its source
   (user, guess, audio), re-analyzes the track (a stored re-pick at the stored params) and
   answers the previous state (role, note, hits) as the undo state. One undo step; the note offer
   takes the new role's default note when the track played on the old role's default. */
const INS_ROLES = ["kick", "snare", "tom", "hat", "cymbal", "other"];
function insRoleSourceText(t) {   // where the role comes from; absent: an older project (the role came from the name)
  const s = t.role_source, m = t.role_match;
  if (s === "user") return "set by you";
  if (s === "audio") return "suggested from the audio";
  if (s === "preset") return m ? `from kit preset (${m} = ${t.role})` : "from kit preset";
  if (s === "scheme") return m ? `from the name (${m} = ${t.role})` : "from the name";
  if (s === "fuzzy") return m ? `guessed from the name (${m} ≈ ${t.role}): check it` : "guessed from the name: check it";   // #285
  if (s === "none" || s === "guess") return "not recognised from the name: choose the role";
  return t.role === "other" ? "not recognised from the name: choose the role" : "from the name";
}
/* #555: a DAW track with several sources: one short line says what the settings of the row reach. The row owns role and note (every source gets them);
   Sensitivity, Loudness, Learn and the hits stay with one source: the first one (the Inspector shows it) or the source of the hit or clip you work in. */
function insRowNote(name) {
  let n = document.getElementById("insRowNote");
  const k = HT.srcsOf(name).length;
  if (k <= 1) { if (n) n.remove(); return; }
  if (!n) { n = document.createElement("div"); n.id = "insRowNote"; n.style.cssText = "font-size:11px;color:#8a93a3;margin:4px 0;line-height:1.35"; $("#insRoleSrcRow").after(n); }
  n.textContent = `This DAW track has ${k} sources: role, note, Sensitivity, Loudness and Shift apply to all of them (one undo step). Learn marks and hit edits belong to the source of the clip you work in.`;
}
function insRoleControls(offer) {   // the role select, the source hint and the note offer of the selected track
  const t = S.tracks[INS.name], sel = $("#insRoleSel");
  if (!t || !sel) return;
  if (!sel.options.length) for (const r of INS_ROLES) { const o = document.createElement("option"); o.value = r; o.textContent = r; sel.appendChild(o); }
  sel.value = INS_ROLES.includes(t.role) ? t.role : "other";
  $("#insRoleSrcRow").classList.remove("hidden");
  insRowNote(INS.name);
  const srcTxt = insRoleSourceText(t);
  $("#insRoleSrc").textContent = srcTxt;
  $("#insRoleSrc").classList.toggle("lyWarn", srcTxt.startsWith("not recognised"));
  // #285: the Name pattern field of #232 is gone; the hint above names the matched words.
  const orow = $("#insNoteOfferRow");
  INS.offer = offer && offer.suggest_note != null && offer.note !== offer.suggest_note
    ? { name: INS.name, role: offer.role, note: offer.note, suggest_note: offer.suggest_note, role_source: offer.role_source } : null;
  if (INS.offer) {
    $("#insNoteOffer").textContent = `${INS.offer.role}: default note ${nameFromNote(INS.offer.suggest_note)} (${INS.offer.suggest_note})`;
    orow.classList.remove("hidden");
  } else orow.classList.add("hidden");
}
async function setTrackRole(name, role, apply = {}) {   // apply: {note?, source?, events?, op?}; true when the service did it
  if (!S.tracks[name] || !INS_ROLES.includes(role)) return false;
  if (analysisRunning) { setMsg("analysis running: change the role after it"); return false; }
  const sid = S.sid;
  try { await trFlush(!apply.op); }
  catch (e) { setMsg(`role change failed: ${errText(e)}`, "error"); return false; }   // #249 R02: the role change must not run while its edits are refused
  let r;
  try { r = await B.trackSetRole(sid, name, role, apply.note ?? null, apply.source ?? null, apply.role_match ?? null, apply.events ?? null); }
  catch (e) { setMsg(`role change failed: ${errText(e)}`, "error"); return false; }
  if (sid !== S.sid) return false;
  S.tracks[name].role = role;
  if (S.tracks[name].colorFromRole) delete S.tracks[name].color;   // the colour of the old role was only a default
  trackColors(S.tracks);
  S.tracks[name].note = r.note;
  S.tracks[name].role_source = r.role_source;
  S.tracks[name].role_match = apply.source === "user" || apply.source === "audio" ? null : (apply.role_match ?? S.tracks[name].role_match ?? null);
  const steps = [{ type: "role", sid, track: name, before: r.previous, after: { role, note: r.note, role_source: r.role_source, role_match: S.tracks[name].role_match } }];
  let refused = null;
  if (!apply.op) for (const o of HT.srcsOf(name)) {   // #555: the row owns role and note: the other sources of the DAW track get the same (their hits are re-picked at their stored settings); an undo step applies its own source only (op)
    if (o === name || !S.tracks[o]) continue;
    try {
      const r2 = await B.trackSetRole(sid, o, role, r.note, apply.source ?? null, apply.role_match ?? null, null);
      S.tracks[o].role = role; S.tracks[o].note = r2.note; S.tracks[o].role_source = r2.role_source;
      steps.push({ type: "role", sid, track: o, before: r2.previous, after: { role, note: r2.note, role_source: r2.role_source, role_match: apply.source === "user" || apply.source === "audio" ? null : (apply.role_match ?? S.tracks[o].role_match ?? null) } });   // each source keeps its own previous state (its hand edits come back with the undo)
    } catch (e) { refused = o; setMsg(`role change of ${trackLabel(o)} failed: ${errText(e)}; the sources before it have the new role (one undo step takes them back)`, "error"); break; }   // a refused source stops the row: the entry holds exactly the sources that changed
  }
  await trReload(!!apply.op, name);   // the lanes, histograms and the panel follow the new role
  refreshHeadRole(name);
  if (!apply.op) pushGroup(`Set Role (${trackLabel(name)})`, steps);
  insRoleControls(r);
  if (!refused) setMsg(apply.op ? `${name} is a ${role} track again` : `${name} is now a ${role} track${r.reanalyzed ? " (the hits are re-picked)" : ""}`);
  return true;
}
async function roleApplyOp(op, reverse) {   // the undo and the redo of a role change
  if (op.sid !== S.sid) { setMsg("that project is not the open one"); return false; }
  const st = reverse ? op.before : op.after;
  return setTrackRole(op.track, st.role, { note: st.note, source: st.role_source, role_match: st.role_match, events: reverse ? op.before.events : null, op: true });
}
/* The name field: the service renames the record, its place in the order and the Analysis/ folder (track_rename). Every
   key of the page (events, peaks, meters, the engine's tiles) carries the name, so the project is loaded again; the view,
   the playhead and the selection stay. */
async function renameTrack(name, to) {
  if (hostAbsent("tracks")) return false;   // #539: the host names the tracks
  to = String(to || "").trim();
  if (!to || to === name || !S.tracks[name]) return false;
  if (S.tracks[to]) { setMsg(`a track named "${to}" exists`); return false; }
  if (analysisRunning) { setMsg("analysis running: rename the track after it"); return false; }
  try { await flushEvents(); await B.renameTrack(S.sid, name, to); }
  catch (e) { setMsg(`rename failed: ${errText(e)}`, "error"); return false; }
  if (S.playing) pause();   // P4-1b fix round: loadProject pauses; the playhead to keep is the one of the pause, not one read before it
  const view = { ...S.view }, ph = S.playhead;
  await loadProject();
  S.view = view; S.playhead = ph; S.selTrack = to;
  document.querySelectorAll(".track").forEach(r => r.classList.toggle("selTrack", r.dataset.name === to));
  invalidate(F.VIEW); updateStatus();
  setMsg(`renamed ${name} to ${to}`);
  return true;
}
/* ---------------- P4-39a: Track > Remove Track ----------------
   The service removes the track (record, hits, Learn examples, layer, Analysis/<name>/; a follower becomes a normal track) and
   answers a snapshot of the record. One undo step {type: "trkdel"} keeps that snapshot and the page's mute and solo of the
   track: undo puts the track back through tracks_restore, redo removes it again. Every key of the page carries the track name,
   so the project is loaded again (the view and the playhead stay). The first removal ends with the one step on the stacks
   (loadProject empties them); undo and redo keep the stacks they are working on. */
function trModalOpen() { return !!document.querySelector(".modal:not(.hidden):not(.sheet)"); }
async function trReload(keepStacks, pick) {
  if (S.playing) pause();   // loadProject pauses; the playhead to keep is the one of the pause
  const view = { ...S.view }, ph = S.playhead, undo = S.undo, redo = S.redo;
  await loadProject();
  if (keepStacks) { S.undo = undo; S.redo = redo; }
  S.view = view; S.playhead = Math.min(ph, maxDur());
  setSelTrack(pick && S.tracks[pick] ? pick : null);
  invalidate(F.VIEW); DSShell.sync(); updateStatus();
}
async function trFlush(inQueue) {   // the hit edits on their way, then the page's own writes; a step of the SH queue must not wait for the queue (edIdle)
  if (!inQueue) await edIdle();
  await Promise.all(pendingWriteTracks().map(pushEvents));
}
async function trRemove(name, op) {   // true when the service removed the track; op: the step that a redo runs again (it runs in the SH queue)
  if (hostAbsent("tracks")) return false;   // #539: the host owns the track set
  const sid = S.sid;
  try { await trFlush(!!op); }
  catch (e) { setMsg(`remove failed: ${errText(e)}`, "error"); return false; }   // #249 R02: the removal must not run while its edits are refused
  let r;
  try { r = await B.tracksRemove(sid, [name]); }
  catch (e) { setMsg(`remove failed: ${errText(e)}`, "error"); return false; }
  if (sid !== S.sid) return false;
  const rest = S.order.filter(n => n !== name), idx = S.order.indexOf(name);
  const step = op || { type: "trkdel", sid, name, snapshot: null, muted: !!S.muted[name], solo: !!S.solo[name] };
  step.snapshot = r.snapshot; step.muted = !!S.muted[name]; step.solo = !!S.solo[name];
  delete S.muted[name]; delete S.solo[name]; dropQueuedWrite(name);
  await trReload(!!op, rest[idx] || rest[idx - 1] || null);   // the next track in order, else the previous one
  if (!op) pushUndo(step);
  setMsg(op ? `${name} removed` : `${name} removed (undo: Cmd+Z)`);
  return true;
}
async function removeTrack(name) {   // Track > Remove Track, the Delete key: the confirm first
  if (hostAbsent("tracks")) return false;
  if (!needProject() || !S.tracks[name]) return false;
  if (trModalOpen()) return false;
  if (analysisRunning) { setMsg("analysis running: remove the track after it"); return false; }
  const n = (S.events[name] || []).length, fol = S.order.filter(f => S.tracks[f] && S.tracks[f].follows === name);
  const a = await askConfirm({ title: `Remove track ${name} and its ${n} hit${n === 1 ? "" : "s"}?`,
    msg: "The audio file stays where it is. Undo brings the track back with its hits." + (fol.length ? ` ${fol.join(", ")} will detect on its own.` : ""),
    buttons: [{ label: "Cancel", act: "cancel" }, { label: "Remove", act: "remove", cls: "primary" }] });
  if (a !== "remove" || !S.tracks[name]) { setMsg("remove track cancelled"); return false; }
  return trRemove(name, null);
}
function removeSelectedTrack() { const n = needTrack(); return n ? removeTrack(n) : false; }
async function trApplyOp(op, reverse) {   // the undo and the redo of a removal; true when the service did it
  if (op.sid !== S.sid) { setMsg("that project is not the open one"); return false; }
  if (!reverse) return trRemove(op.name, op);
  try { await trFlush(true); }
  catch (e) { setMsg(`undo failed: ${errText(e)}`, "error"); return false; }   // #249 R02: the restore must not run while its edits are refused
  try { await B.tracksRestore(op.sid, op.snapshot); }
  catch (e) { setMsg(`undo failed: ${errText(e)}`, "error"); return false; }
  if (op.muted) S.muted[op.name] = true;
  if (op.solo) S.solo[op.name] = true;
  await trReload(true, op.name);
  for (const n of [op.name]) { const row = rowOf(n); if (row) { row.classList.toggle("muted", !!S.muted[n]); row.classList.toggle("solo", !!S.solo[n]); row.querySelector(".bMute").classList.toggle("on", !!S.muted[n]); row.querySelector(".bSolo").classList.toggle("on", !!S.solo[n]); } }
  if (op.muted || op.solo) applyMixer();
  setMsg(`${op.name} restored`);
  return true;
}
