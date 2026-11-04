/* Drum Spike v2 — DAW-style navigation
   Concepts: playhead-anchored zoom, Tab event navigation, event lines through
   both lanes (spike + waveform), follow-playhead, persistent position readout. */
"use strict";

const OWN_ZOOM_NAME_MAX = 40;   // #169: the longest name of an own zoom preset
const ROLE_DEFAULT_NOTE = { kick:36, snare:38, tom:48, hat:42, cymbal:49, other:36 };
const ROLE_DEFAULT_CLICK = { kick:1000, snare:2500, tom:700, hat:4000, cymbal:3000, other:1500 };

/* ---------------- backend ---------------- */
/* Every call to the server goes through B (static/backend.js): named operations, no paths here. */
function normEvents(d){
  const evs = Array.isArray(d) ? d : (d && d.events) || [];
  return evs.map((e,i) => { const o = {t: e.t, v: e.v, id: "l"+(i+1)};   // stable identity (B1)
    if (e.added) o.added = true; if (e.orig !== undefined) o.orig = e.orig;   // manual-hit markers
    if (e.rise !== undefined) o.rise = e.rise;   // transient rise dB (R1 confidence) — carried, not used
    if (e.cut) o.cut = true;   // P4-3: the loudness cut took it out of the output
    if (e.bleed) o.bleed = true;   // P4-14: the tom bleed rule took it out of the output (shown as a cut hit for now)
    if (e.roll) o.roll = true;   // P4-19: the roll fill added it (a normal hit; kept so a page edit does not drop the tag)
    if (e.flam) o.flam = true;   // P4-13 stage B: the flam rule added it (the second stroke of a flam; a normal hit)
    if (e.rescue) o.rescue = true;   // P4-10 stage 5: the kick rescue added it (a quiet kick the model missed; a normal hit)
    if (e.learned) o.learned = true;   // P4-38b: a K added it (kept so undo and redo snapshots keep the tag)
    if (e.filtered) o.filtered = true;   // P4-6c: the Learn filter took it out of the output (a grey hit)
    if (e.edited) o.edited = true;   // P4-14 fix round: restored from the bleed mark, kept by a re-pick or Re-analyze
    if (e.keep) o.keep = true;   // #242: the user keeps the hit in the output (Toggle Hit Active)
    if (e.deactivated) o.deactivated = true;   // #242: the user took the hit out of the output (Toggle Hit Active)
    return o; }).sort((a, b) => a.t - b.t);   // P3-14d: hitsIn searches them by time (the service sends them sorted)
}
let _eid = 0;
function newId(){ return "n" + (++_eid); }

/* manual/edited hits: added by hand, or moved away from their detected position */
const MANUAL_COLOR = "#ff4fd8";
function isManual(e) { return !!e.added || (e.orig !== undefined && Math.abs(e.t - e.orig) > 1e-9); }
/* a kept hit: manual, a hand velocity or a restored hit (`edited: true`), or one of the user's
   per-hit overrides of #242 (`keep`/`deactivated`) — the page's copy of edit::is_kept
   (crates/app/src/edit.rs). One predicate decides the Keep/Discard dialog of a re-analysis and
   the service's merge (keep_edited), so both see the same edits. */
function isKept(e) { return isManual(e) || e.edited === true || e.keep === true || e.deactivated === true; }

/* ---------------- track DOM ---------------- */
function renderNoProject(scroll) {
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
        <div class="thead" title="Click: select the track (the Inspector shows it). Drag: reorder. Right-click: Export…, Re-analyze, Reset Volume and Pan">
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
    if (t.follows) { row.querySelector(".tfollow").textContent = ` · follows ${t.follows}`; row.classList.add("follower"); }   // P3-18: names are data
    const tn = row.querySelector(".tname");   // C1: name as text node — no HTML injection
    const nm = document.createElement("span"); nm.className = "tnm"; nm.textContent = name; nm.title = name;
    tn.appendChild(nm);
    setHeadIcon(row, t.role);   // 0.9.3: the role icon left of the name
    // P3-14c: the header is compact; the 🔗 badge and the hit counts are in the Inspector (source state, Detected, Edited)
    if (t.offline) {
      const b = document.createElement("button"); b.className = "btn mini tOffline"; b.textContent = "offline";
      b.title = `source ${t.status}: click to relink the audio file`;
      b.onclick = () => relinkTrack(name);
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
  S.selTrack = name;
  document.querySelectorAll(".track").forEach(r => r.classList.toggle("selTrack", r.dataset.name === S.selTrack));
  invalidate(F.HEAD);   // P3-14b: the sub counter uses the rate of the selected track
  updateStatus();
}
/* ---------------- track reordering (drag header, live reorder like Pro Tools) ---------------- */
let headerDragged = false;
function applyOrder(newOrder) {
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
    if (ev.button !== 0) return;
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
function roleLineText(t) { return `${t.role} · ${nameFromNote(t.note)} (${t.note})` + (t.follows ? ` · follows ${t.follows}` : ""); }
function followerLocked(name) {   // true (with the message) when the track's hits are derived from another track
  const t = S.tracks[name];
  if (!t || !t.follows) return false;
  setMsg(`${name} follows ${t.follows}: its hits are derived, edit the hits of ${t.follows}`);
  return true;
}
function refreshHeadRole(name) {   // the role line of the header, and the Inspector's when it shows the track
  const row = rowOf(name), t = S.tracks[name];
  if (row && t) {
    setHeadIcon(row, t.role);   // 0.9.3: a role change swaps the icon at once
    row.querySelector(".nnote").textContent = nameFromNote(t.note);
    row.querySelector(".tfollow").textContent = t.follows ? ` · follows ${t.follows}` : "";   // textContent: a track name is data
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
    setMsg(`${name} follows ${src}: ${n} ${n === 1 ? "hit" : "hits"}, velocities from its own signal`);
  } else {
    setMsg(`${name}: own detection (re-analyze to detect its hits)`);
  }
  return true;
}
function followSelected(src) { const n = needTrack(); if (n) return setTrackFollow(n, src); }

/* ---------------- playhead overlay (global, Pro Tools style) ---------------- */
function positionPlayhead() {   // the overlay lives in #editor; it shows only inside the lane area (never over the header column or the Inspector)
  const W = laneWidth();
  const x = Math.round((S.theadW + tToX(S.playhead, W)) * 64) / 64;   // P4-1: float noise writes no new style
  const el = $("#playheadOverlay");
  const display = x >= S.theadW - 0.5 && x <= S.theadW + W + 0.5 ? "block" : "none";
  if (FR.head[0] !== display) el.style.display = FR.head[0] = display;
  if (CP.anim) return;   // P4-1: the compositor moves it
  const tf = `translateX(${x}px)`;
  if (FR.head[1] !== tf) el.style.transform = FR.head[1] = tf;
}
function drawHead() {   // the playhead and the time display; a text is written only when it changed
  positionPlayhead();
  const [main, sub] = counterTexts();
  if (FR.head[2] !== main) $("#timeDisplay").textContent = FR.head[2] = main;
  if (FR.head[3] !== sub) $("#sampleDisplay").textContent = FR.head[3] = sub;
}

/* P3-14b: counters. Main counter: seconds with 6 decimals, samples, Bars|Beats (bar.beat of the
   tempo map, #149) or Timecode (HH:MM:SS:FF at 30 frames per second from the sample position,
   #149); the sub counter shows the other format of the pair (seconds<->samples,
   Bars|Beats<->Timecode). The sample rate is the one of the selected track, else of the first
   track. The Timecode frame rate is 30, non drop: frames = floor(t * 30) over the whole project. */
const COUNTER_PAIRS = { seconds: "samples", samples: "seconds", barsbeats: "timecode", timecode: "barsbeats" };
function counterRate() {
  const t = (S.selTrack && S.tracks[S.selTrack]) || (S.order.length && S.tracks[S.order[0]]);
  return t && t.sr ? t.sr : 48000;
}
function counterBBText(bar, beat, ticks) {   // #149: the Pro Tools Bars|Beats shape, 960 ticks per beat
  return `${bar}| ${String(beat).padStart(2, " ")}| ${String(ticks).padStart(4, " ")}`;
}
function counterBarsBeats() {
  const m = TM.map;
  if (!m) {   // no map: the default one the export uses, 120 BPM 4/4, bar 1 at 0 s
    // quarter-note beats at 120 BPM; the same microsecond snap to the next beat as below, and the
    // ticks are floored (#208: rounding showed 960 -> 0 of the old beat, a beat too early)
    const total = S.playhead * 2 + 2e-6;
    return counterBBText(Math.floor(total / 4) + 1, Math.floor(total % 4) + 1, Math.min(959, Math.floor((total % 1) * 960 + 1e-6)));
  }
  const g = tmGrid();
  if (!g || !g.t.length) return counterBBText(1, 1, 0);
  let i = tmLower(g.t, S.playhead) - 1;   // the beat at or before the playhead (#149)
  if (i < 0) return counterBBText(1, 1, 0);   // before bar 1: clamped to 1| 1| 000
  // the grid times accumulate (tmGrid), so a playhead on a beat can sit a hair below its
  // grid time: treat anything within a microsecond of the next beat as that beat (#149)
  if (i + 1 < g.t.length && g.t[i + 1] - S.playhead * 1 <= 1e-6) i++;
  const dt = (i + 1 < g.t.length ? g.t[i + 1] - g.t[i] : 0.5) || 0.5;
  const ticks = Math.floor((S.playhead - g.t[i]) / dt * 960 + 1e-6);   // floored, never a beat too early (#208)
  return counterBBText(g.bar[i], g.beat[i], Math.max(0, Math.min(959, ticks)));
}
function counterTimecode() {
  const fps = 30, sr = counterRate();
  const t = Math.round(S.playhead * sr) / sr;   // the sample position of the playhead (#149)
  let frames = Math.floor(t * fps);
  const ff = frames % fps; frames = Math.floor(frames / fps);
  const ss = frames % 60; frames = Math.floor(frames / 60);
  const mm = frames % 60, hh = Math.floor(frames / 60);
  const two = n => String(n).padStart(2, "0");
  return `${two(hh)}:${two(mm)}:${two(ss)}:${two(ff)} tc`;
}
function counterTexts() {
  const secs = `${fmt(S.playhead, 6)} s`, smp = `${Math.round(S.playhead * counterRate())} smp`;
  const bb = counterBarsBeats(), tc = counterTimecode();
  const texts = { seconds: secs, samples: smp, barsbeats: bb, timecode: tc };
  const main = texts[SETTINGS().ui.counter] || secs;
  return [main, texts[COUNTER_PAIRS[SETTINGS().ui.counter] || "samples"]];
}
function setCounter(mode) {   // View > Main Counter: "seconds", "samples", "barsbeats" or "timecode" (#149)
  if (!(mode in COUNTER_PAIRS)) return;
  SETTINGS().ui.counter = mode; saveSettings();
  FR.head[2] = FR.head[3] = null; invalidate(F.HEAD);
  $("#timeDisplay").title = `Main counter (${mode}${mode === "timecode" ? ", 30 fps" : ""}). Click: exchange the counters (View > Main Counter > Exchange Counters)`;
  DSShell.sync();
}
function exchangeCounters() { setCounter(COUNTER_PAIRS[SETTINGS().ui.counter] || "seconds"); }

/* ---------------- canvas sizing ---------------- */
function laneWidth() { return GEO.laneW; }
/* P3-6b: the canvases of a lane keep a backing store only while the lane is in view or half a screen from it: each
   2D canvas of the 30-lane project cost 0.8 to 2.6 MB of graphics memory in the WebContent process (about 100 MB
   together). Other lanes hold a 1 x 1 canvas with the same CSS size and are drawn again when they come near.
   P3-14d: "near" comes from GEO (no layout read); a row that the observer has not measured yet is not near. */
function lanesNear() {
  const m = GEO.scH / 2, near = new Set();
  document.querySelectorAll("#lanesScroll .track").forEach(row => {
    const g = GEO.rows[row.dataset.name];
    if (g && g.bottom - GEO.top >= -m && g.top - GEO.top <= GEO.scH + m) near.add(row);
  });
  return near;
}
function sizeLaneCanvas(c, live) {
  const dpr = GEO.dpr, W = laneWidth(), hh = laneHeights(), row = c.closest(".track"), own = row && S.rowH && S.rowH[row.dataset.name];
  const vel = c.classList.contains("vel");   // #208: the velocity lane has its own height (VL.h), not the wave height
  const h = c.classList.contains("spike") ? hh.spike : vel ? VL.h : (own ? Math.round(hh.wave * own) : hh.wave);   // P3-14c; P4-27/#167: a row with its own factor
  c.style.width = W + "px";   // the CSS height comes from the variables of setTrackHeight (an inline height would win over them)
  c.style.transform = ""; c._sptf = "";   // #463: a lane sized for a draw is not on a strip (the rAF path's cache resets too)
  if (vel) c.style.height = h + "px";
  if (live) { c.width = W * dpr; c.height = h * dpr; c.getContext("2d").setTransform(dpr, 0, 0, dpr, 0, 0); }
  else { c.width = 1; c.height = 1; }
}
function syncLaneBuffers() {   // in the frame: a lane came near the view, or left it; true when one came near
  const near = lanesNear();
  let woke = false;
  document.querySelectorAll("#lanesScroll .track").forEach(row => {
    const live = near.has(row);
    if (live === (row._live !== false)) return;
    row._live = live; woke = woke || live;
    row.querySelectorAll("canvas.lane").forEach(c => sizeLaneCanvas(c, live));
  });
  return woke;
}
function sizeCanvases() {   // in the frame: every canvas to the sizes of GEO (writes only)
  const dpr = GEO.dpr, near = lanesNear();
  document.querySelectorAll("#lanesScroll .track").forEach(row => {
    row._live = near.has(row);
    row.querySelectorAll("canvas.lane").forEach(c => sizeLaneCanvas(c, row._live));
  });
  const ov = $("#overview"), rul = $("#ruler"), rw = GEO.rulerW;
  ov.width = GEO.ovW * dpr; ov.height = GEO.ovH * dpr;
  ov.getContext("2d").setTransform(dpr,0,0,dpr,0,0);
  rul.style.width = rw + "px"; rul.style.height = "22px";
  rul.style.transform = ""; rul._sptf = "";   // #150: the time ruler drops its strip transform with its size, like the lane canvases (sizeLaneCanvas)
  rul.width = rw * dpr; rul.height = 22 * dpr; rul.getContext("2d").setTransform(dpr,0,0,dpr,0,0);
  sizeMapCanvases();   // P4-8
  FR.sized = sizeKey();
}
function resizeCanvases() { invalidate(F.SIZE); }   // the next frame sizes and draws every canvas

/* ---------------- coordinates ---------------- */
/* P4-1b fix: the span to the nanosecond. A view is stored as {t0, t1}, and t1 - t0 of a view set as {t0, t0 + d} is d plus
   or minus an ulp for many t0; the follow step carried that noise on (8 became 7.999999999999999). Two draws of the same
   span then put a peak bin at a column edge on different sides (the strip against a direct draw). */
const viewDur = () => Math.round((S.view.t1 - S.view.t0) * 1e9) / 1e9;
const spanOf = v => Math.round((v.t1 - v.t0) * 1e9) / 1e9;   // the same for any view
function tToX(t, w) { return (t - S.view.t0) / viewDur() * w; }
function xToT(x, w) { return S.view.t0 + x / w * viewDur(); }
function maxDur() { return Math.max(10, ...S.order.map(n => S.tracks[n].duration)); }
function snapSample(t, name) { const sr = S.tracks[name].sr; return Math.round(t*sr)/sr; }
/* playhead snapping: within 10px on screen AND <= 30 ms of the nearest hit -> land exactly on it */
function snapPlayhead(t) {
  if (!S.snap) return t;
  const pxTime = viewDur() / laneWidth();          // seconds per screen pixel
  const thresh = Math.min(10 * pxTime, 0.030);
  let best = null, bestDt = thresh;
  for (const n of S.order) {   // W6: only the hits within the threshold (binary search)
    const evs = S.events[n] || [];
    for (const i of hitsIn(evs, t - thresh, t + thresh, dragSkip(n))) {
      const dt = Math.abs(evs[i].t - t);
      if (dt < bestDt) { bestDt = dt; best = evs[i].t; }
    }
  }
  return best !== null ? best : t;
}
/* per-track display gain: normalize so the track's own peak fills ~90% of half-height */
function peakAmpOf(p) {
  if (!p || !p.levels) return 1;
  const L = p.levels[65536] || p.levels[4096] || p.levels[256];
  if (!L || !L.max || !L.max.length) return 1;
  const mx = Math.max(...L.max), mn = Math.min(...L.min);   // E: negative peaks count too
  return Math.max(1e-5, mx, -mn);
}

/* ---------------- view navigation ---------------- */
function viewClamp(t0, t1) {
  const maxD = maxDur();
  const d = Math.min(maxD, Math.max(t1 - t0, 0.002));   // keep the requested span (zoom presets, pans
  t0 = Math.max(0, Math.min(t0, maxD - d));             // and follow near 0 used to shrink it) and
  return { t0, t1: t0 + d };                            // slide it inside [0, maxD]
}
function setView(t0, t1) {
  S.view = viewClamp(t0, t1); invalidate(F.VIEW); updateStatus();
}
function centerOn(t) {
  const d = viewDur();
  setView(t - d/2, t + d/2);
}
function zoomAround(anchorT, factor) {
  S.followHold = true;   // user interaction pauses follow until next play
  const d = viewDur();
  const nd = Math.min(maxDur(), Math.max(0.002, d*factor));
  setView(anchorT - nd/2, anchorT + nd/2);
}
function zoomAtX(x, w, factor) {   // P4 #145: Cmd+wheel and pinch; the time under lane x stays under x
  S.followHold = true;
  const a = xToT(x, w), f = x / w;
  const nd = Math.min(maxDur(), Math.max(0.002, viewDur()*factor));
  setView(a - f*nd, a - f*nd + nd);
}
function zoomPlayhead(factor) {
  if (S.playhead < S.view.t0 || S.playhead > S.view.t1) centerOn(S.playhead);
  zoomAround(S.playhead, factor);
}
function panView(dt) {
  S.followHold = true;   // user interaction pauses follow until next play
  const maxD = maxDur();
  const t0 = Math.max(0, Math.min(S.view.t0 + dt, maxD - viewDur()));
  setView(t0, t0 + viewDur());
}

/* ---------------- drawing ---------------- */
function redrawAll() {   // everything, now (tests and the self-test; the app invalidates and draws in the frame)
  FR.flags &= ~(F.VIEW | F.HEAD | F.PH | F.GL); FR.tracks.clear(); FR.sel = selKey();
  drawAll();
}
function drawAll() {
  drawRuler(); drawOverview();
  for (const name of S.order) {
    const row = rowOf(name);
    if (!row) continue;
    if (row._live === false && !GLQ.all) continue;   // far from the view: no canvas store (the GL lane is culled by its position)
    drawSpike(row.querySelector(".spike"), name);
    drawWave(row.querySelector(".wave"), name);
    if (VL.on) drawVelLane(row.querySelector(".vel"), name);   // P4-40f: the vel lane draws at open too, not only after edits
  }
  glFlush();   // P3-6a: the wave lanes queued by drawWave
  drawHead();
}
/* P3-14d: the hit lines of one track after an edit. With the GL engine the wave lane's 2D canvas holds only the hit
   lines, so the GL lane is not drawn again; the Canvas 2D path draws its wave lane again. False: no canvas store. */
function drawHits(name) {
  const row = rowOf(name);
  if (!row || row._live === false || !S.tracks[name]) return false;   // far from the view: drawn when it comes near
  const wave = row.querySelector(".wave");
  drawSpike(row.querySelector(".spike"), name);
  if (VL.on) drawVelLane(row.querySelector(".vel"), name);   // P4-40f: the velocity lane draws with the hits
  if (glEngine() && S.peaks[name]) {
    const [W, H] = laneSize(wave);
    wave.getContext("2d").clearRect(0, 0, W, H);
    drawEventLines(wave, name);
  } else drawWave(wave, name);
  return true;
}

function drawRuler() {
  const c = $("#ruler"), ctx = c.getContext("2d"), W = GEO.rulerW, H = 22;
  ctx.clearRect(0,0,W,H);
  ctx.fillStyle = "#1c1f26"; ctx.fillRect(0,0,W,H);
  const dur = viewDur();
  const steps = [0.001,0.002,0.005,0.01,0.02,0.05,0.1,0.2,0.5,1,2,5,10,30,60];
  const step = steps.find(s => s*W/dur > 60) || 60;
  // Pro Tools style: the major tick runs the full height, its label sits right of it with room above and beside
  ctx.strokeStyle = "#3a404c"; ctx.fillStyle = "#8a93a3"; ctx.font = "10px -apple-system, sans-serif"; ctx.textBaseline = "top";
  for (let t = Math.ceil(S.view.t0/step)*step; t <= S.view.t1; t += step) {
    const x = Math.round(tToX(t, W)) + 0.5;
    ctx.beginPath(); ctx.moveTo(x, 3); ctx.lineTo(x, H); ctx.stroke();
    ctx.fillText(step < 1 ? fmt(t,3) : fmt(t,1), x+5, 5);
    for (let k=1;k<5;k++){ const xx = Math.round(tToX(t + step*k/5, W)) + 0.5; ctx.beginPath(); ctx.moveTo(xx,H-5); ctx.lineTo(xx,H); ctx.stroke(); }
  }
  if (AU.win) {   // P4-40d: the loop of the audition
    const x0 = Math.max(0, tToX(AU.win.t0, W)), x1 = Math.min(W, tToX(AU.win.t1, W));
    if (x1 > x0) { ctx.fillStyle = "rgba(255, 184, 77, 0.45)"; ctx.fillRect(x0, 0, x1 - x0, H); ctx.fillStyle = "#ffb84d"; ctx.fillRect(x0, H - 3, x1 - x0, 3); }
  }
  if (LN.sel) {   // P4-6a: two small edge marks of the range selection
    ctx.fillStyle = "#4da3ff";
    for (const t of [LN.sel.t0, LN.sel.t1]) { const x = Math.round(tToX(t, W)); if (x >= -4 && x <= W + 4) { ctx.fillRect(x - 1, 0, 2, H); ctx.beginPath(); ctx.moveTo(x - 4, 0); ctx.lineTo(x + 4, 0); ctx.lineTo(x, 5); ctx.closePath(); ctx.fill(); } }
  }
  drawMapRulers();   // P4-8: the tempo, meter and Bars|Beats rulers share the x scale (and the strip)
}

/* P4-40b: the lane cursor says what a click does. #lanesScroll carries one class: curAdd (Option held: a click adds a hit)
   or curDelete (Option+Shift held over a hit that can be deleted: a click deletes it). The classes follow the keys
   (keydown and keyup of Option and Shift, also the modifiers of every mouse move) and the pointer; the CSS draws the cursors. */
const CUR = { alt: false, shift: false, del: false };
function curApply() {
  const sc = document.getElementById("lanesScroll"); if (!sc) return;
  sc.classList.toggle("curAdd", CUR.alt && !CUR.shift);
  sc.classList.toggle("curDelete", CUR.alt && CUR.shift && CUR.del);
}
function curMods(ev) { CUR.alt = !!ev.altKey; CUR.shift = !!ev.shiftKey; curApply(); }
function curOver(del, ev) { CUR.del = del; if (ev) { CUR.alt = !!ev.altKey; CUR.shift = !!ev.shiftKey; } curApply(); }   // del: the pointer is over a hit that Delete can remove
for (const t of ["keydown", "keyup", "mousemove"]) window.addEventListener(t, ev => { if (ev.target !== window) curMods(ev); }, { capture: true, passive: true });
window.addEventListener("blur", () => { CUR.alt = CUR.shift = false; curApply(); });

function drawOverviewTracks(ctx, W, H) {
  ctx.fillStyle = "#161920"; ctx.fillRect(0,0,W,H);
  const rowH = H / Math.max(1, S.order.length);
  S.order.forEach((name, idx) => {
    const p = S.peaks[name]; if (!p) return;
    const L = p.levels[65536] || p.levels[4096] || p.levels[256]; if (!L) return;
    const y0 = idx*rowH;
    ctx.fillStyle = "rgba(255,255,255,0.04)"; ctx.fillRect(0, y0, W, rowH-1);
    const amp = S.peakAmp[name] || 1;
    const scale = ((rowH-4)/2) / amp;
    if (Array.isArray(L.rms)) {
      ctx.fillStyle = S.tracks[name].color;
      for (let px=0; px<W; px++) {
        const b0 = Math.floor(px/W*L.n), b1 = Math.max(b0+1, Math.floor((px+1)/W*L.n));
        let r = 0; for (let b=b0;b<Math.min(b1,L.n);b++) r = Math.max(r, L.rms[b]);
        const hh = Math.min((rowH-4)/2, r*scale);
        if (hh>0.3) ctx.fillRect(px, y0+rowH/2-hh, 1, hh*2);
      }
    }
    ctx.fillStyle = S.tracks[name].color + "66";
    for (let px=0; px<W; px++) {
      const b0 = Math.floor(px/W*L.n), b1 = Math.max(b0+1, Math.floor((px+1)/W*L.n));
      let mn=1, mx=-1;
      for (let b=b0;b<Math.min(b1,L.n);b++) { mn=Math.min(mn,L.min[b]); mx=Math.max(mx,L.max[b]); }
      const yy0 = y0+rowH/2 - mx*scale, yy1 = y0+rowH/2 - mn*scale;
      ctx.fillRect(px, yy0, 1, Math.max(1, yy1-yy0));
    }
  });
}

/* P3-6a: the tracks of the overview do not depend on the view or the playhead: they are drawn once into
   an offscreen canvas (OV) and blitted; only the view box and the playhead are drawn per redraw. The
   uncached drawing cost 7.9 ms per redraw with 30 tracks in WKWebView. */
const OV = { key: "", peaks: [], amps: [], canvas: null };
function overviewTracks(c, W, H) {
  const peaks = S.order.map(n => S.peaks[n]), amps = S.order.map(n => S.peakAmp[n]);
  const key = `${c.width}x${c.height}|${W}x${H}|${S.order.map(n => n + S.tracks[n].color).join("|")}`;
  if (OV.canvas && key === OV.key && peaks.every((p, i) => p === OV.peaks[i]) && amps.every((a, i) => a === OV.amps[i])) return OV.canvas;
  OV.key = key; OV.peaks = peaks; OV.amps = amps;
  if (OV.canvas) { OV.canvas.width = 0; OV.canvas.height = 0; }
  OV.canvas = document.createElement("canvas"); OV.canvas.width = c.width; OV.canvas.height = c.height;
  const ctx = OV.canvas.getContext("2d");
  ctx.setTransform(c.width / W, 0, 0, c.height / H, 0, 0);
  drawOverviewTracks(ctx, W, H);
  return OV.canvas;
}
function drawOverview() {
  const c = $("#overview"), ctx = c.getContext("2d"), W = GEO.ovW, H = GEO.ovH;
  ctx.clearRect(0,0,W,H); ctx.fillStyle = "#161920"; ctx.fillRect(0,0,W,H);
  const maxD = maxDur();
  if (c.width && c.height) { ctx.save(); ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.drawImage(overviewTracks(c, W, H), 0, 0); ctx.restore(); }
  const x0 = S.view.t0/maxD*W, x1 = S.view.t1/maxD*W;
  ctx.fillStyle = "rgba(77,163,255,0.10)"; ctx.fillRect(x0, 0, x1-x0, H);
  ctx.strokeStyle = "#4da3ff"; ctx.strokeRect(x0+0.5, 0.5, x1-x0-1, H-1);
  ctx.fillStyle = "#4da3ff"; ctx.fillRect(x0-1, 0, 3, H); ctx.fillRect(x1-2, 0, 3, H);
  const px = S.playhead/maxD*W;
  ctx.strokeStyle = "#ff9f43"; ctx.lineWidth = 1.5;
  ctx.beginPath(); ctx.moveTo(px,0); ctx.lineTo(px,H); ctx.stroke();
  ctx.lineWidth = 1;
  ctx.fillStyle = "#ff9f43";
  ctx.beginPath(); ctx.moveTo(px-5,0); ctx.lineTo(px+5,0); ctx.lineTo(px,7); ctx.closePath(); ctx.fill();
}

/* APP_V stays in app.js (refactor step 5.2): it reads the ?v= of this script's own tag at load; wave.js
   loads before app.js, where the tag is not in the DOM yet. glEngine (wave.js) reads it at run time. */
const APP_V = (Array.from(document.scripts).map(x => /app\.js\?v=(\w+)/.exec(x.src)).find(Boolean) || [0, "0"])[1];

function openUndoHistory() {
  if (!needProject()) return;
  const box = $("#undoHistList"); box.textContent = "";
  const rows = undoHistoryRows();
  $("#undoHistEmpty").classList.toggle("hidden", rows.length > 0);
  for (const r of rows) {
    const b = document.createElement("button");
    b.className = "uhRow" + (r.redo ? " uhRedo" : "");
    b.textContent = stepText(r.op, !r.redo);
    b.title = r.redo ? `Redo ${r.count} step${r.count === 1 ? "" : "s"}` : `Undo ${r.count} step${r.count === 1 ? "" : "s"}`;
    b.dataset.count = r.count; b.dataset.dir = r.redo ? "redo" : "undo";
    b.onclick = async () => { $("#undoHistModal").classList.add("hidden"); await undoSteps(!r.redo, r.count); };
    box.appendChild(b);
  }
  showModal("#undoHistModal");
}
function needProject() {   // S3-3: the page can show no project at all
  if (S.sid) return true;
  setMsg("no project is open: use New, Open or Open Recent"); return false;
}

/* ---------------- playback: the native transport (P3-5b) ---------------- */
function applyMixer() { appMix(); }   // live mute/solo during playback
function play(returnKey) { return appPlay(undefined, returnKey); }   // #207: returnKey is the play-and-return key that starts the play
function showSpeed(speed) {   // the status bar says "half speed" while the transport plays at half speed
  const el = $("#stSpeed"), half = speed === 0.5;
  if (el.classList.contains("hidden") === half) el.classList.toggle("hidden", !half);
}
function pause() { return appPause(); }

/* ---------------- follow modes (Pro Tools/Logic style) ----------------
   smooth: timeline glides continuously, playhead pinned at FOLLOW_FRAC of the view
   page:   flip one page when the playhead crosses the right edge
   off:    playhead may leave the view                                             */
const FOLLOW_FRAC = 0.25;
function followStep() {
  if (S.followMode === "off" || S.followHold) return;
  const d = viewDur();
  if (followKind() === "page") {   // P4-39b: also the smooth follow where no glide can show; it flips to FOLLOW_FRAC, the page mode centres
    if (S.playhead > S.view.t1 - d*0.1 || S.playhead < S.view.t0) {
      if (S.followMode !== "smooth") { centerOn(S.playhead); return; }
      const v = viewClamp(S.playhead - FOLLOW_FRAC * d, S.playhead - FOLLOW_FRAC * d + d);
      if (Math.abs(v.t0 - S.view.t0) > 1e-12 || Math.abs(v.t1 - S.view.t1) > 1e-9) setView(v.t0, v.t1);
    }
    return;
  }
  // smooth
  if (S.playhead < S.view.t0 || S.playhead > S.view.t1) { centerOn(S.playhead); return; }  // re-enter after Tab/jump
  const anchorX = FOLLOW_FRAC * d;
  const curX = S.playhead - S.view.t0;
  // P3-14d (W4): the view moves in every frame in which the playhead moved half a pixel or more (before: about 33 Hz
  // and 2 px steps, so the pinned playhead juddered against the 60 Hz frames)
  // P4-1: only when the clamped view changes. Left of the anchor at the start of the song (and at its end) the view
  // cannot move; before, every frame drew all lanes again for nothing (the whole `play` scenario has a 300 s view).
  if (Math.abs(curX - anchorX) / d * laneWidth() < 0.5) return;
  const v = viewClamp(S.playhead - anchorX, S.playhead - anchorX + d);
  if (Math.abs(v.t0 - S.view.t0) > 1e-12 || Math.abs(v.t1 - S.view.t1) > 1e-9) setView(v.t0, v.t1);   // the span is (t0 + d) - t0: an ulp apart
}
function tick() {   // one playback frame (the frame scheduler calls it while playing; play() calls it once)
  if (!S.playing) { cpStop(); spLeave("pause"); flushFrame(); return; }
  PH.v = appTickPos();
  followResume();   // P4-wf10: the 2 s of playback after the edit are over: follow again
  if (PH.v >= maxDur()) { pause(); S.playhead = 0; invalidate(F.HEAD); flushFrame(); return; }
  const now = AC.tickAt;
  let strip = SP.on && spTick(now);   // P4-1: a strip of the smooth follow moves by itself
  if (!SP.on && SP.req && spEnter(now, true)) strip = spTick(now);   // P4-1b: a strip asked for ahead shows now
  if (SP.on) { spWrite(now); requestFrame(); }   // #463: the strip's 2D transforms are written per frame (rAF), so the loop keeps running under a strip
  if (!strip) followStep();
  const cp = !strip && cpArm(now);   // P4-1: the compositor moves the playhead while the view stands still
  if (cp) spAhead(now);
  FR.flags |= F.PH;   // cheap: the overlay and the time; a follow step asked for the whole view
  const ox = Math.round(PH.v / maxDur() * GEO.ovW * 2);   // the overview playhead, in half pixels: a VIEW frame draws it too
  if (ox !== FR.ovx) { FR.ovx = ox; FR.flags |= F.HEAD; }
  flushFrame();
  if (!strip && !cp) spEnter(now);   // the view glides in the frame path: the first strip
  /* #463: the flag says this tick did NOT draw the playhead (an animation did). cp is also true when cpStill said
     show:false and no animation exists (the playhead left the view in off mode): the frame path drew it, so the
     tick must stay readable for drawnPlayheadAt. */
  CP.hist.push([now, AC.drawn, performance.now(), (cp && !!CP.anim) || SP.on]);
  if (CP.hist.length > CP_KEEP) CP.hist.splice(0, CP.hist.length - CP_KEEP);
  if (!cp && !SP.on) { requestFrame(); return; }
  clearTimeout(CP.slow); CP.slow = setTimeout(() => { if (S.playing) requestFrame(); }, CP_SLOW_MS);   // the counters without meter frames
}

/* ---------------- track handlers ---------------- */
/* P3-14d: the x of a pointer in a lane (the ruler, the overview) from clientX and GEO: MouseEvent.offsetX lays the
   page out when it is dirty, and a window listener's offsetX is relative to whatever element is under the pointer */
const laneXOf = ev => ev.clientX - GEO.laneX;
const rulerXOf = ev => ev.clientX - GEO.rulerX;
const ovXOf = ev => ev.clientX - GEO.ovX;

function attachTrackHandlers(row, name) {
  const spike = row.querySelector(".spike"), wave = row.querySelector(".wave");
  const evs = () => S.events[name] || [];
  function hitEvent(px) {   // W6: the hits within 4 px (binary search)
    const evsL = evs(), W = laneWidth(), t = xToT(px, W), d = 4 * viewDur() / W;
    for (const i of hitsIn(evsL, t - d, t + d, dragSkip(name))) {
      const x = tToX(evsL[i].t, W);
      if (Math.abs(x-px) < 4) return i;
    }
    return -1;
  }
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
    const e = evs()[ds.i];
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
      const hi = hitEvent(px);
      if (hi >= 0) { if (!followerLocked(name)) edDelete(name, evs()[hi].id); return; }
    }
    if (ev.shiftKey) { lnClearSel(); S.playhead = snapPlayhead(xToT(px, laneWidth())); invalidate(F.HEAD); updateStatus(); return; }
    if (ev.altKey) { addHitAt(name, snapSample(xToT(px, laneWidth()), name), true); return; }   // P3-13b: Option-click adds a hit (Cubase); #144: on the attack
    const i = hitEvent(px);
    if (i < 0) {   // P4-6a: a click on a Learn mark bar selects the mark (Delete removes it)
      const lm = lnMarkAt(name, px, ev.offsetY);
      if (lm) { lnClearSel(); S.sel = null; lnSetMark(name, lm.id); updateStatus(); setMsg(`${name}: mark ${lm.id} (${lm.kind === "not" ? "not this" : "this"}, ${(lm.at || []).length} ${(lm.at || []).length === 1 ? "example" : "examples"}); Delete removes it, right-click for more`); return; }
    }
    lnClearSel(); if (LN.mark) lnSetMark(null);
    if (i >= 0 && followerLocked(name)) { S.sel = {track:name, index:i}; invalidateHits(name); updateStatus(); return; }   // P3-18: select, no drag
    if (i >= 0) {
      S.sel = {track:name, index:i};
      if (SH.pending || hgRepickBusy()) { invalidateHits(name); updateStatus(); return; }   // an edit is on its way: select, no drag (the hits on the page are not the stored ones yet)
      if (hasPendingWrite(name)) pushEvents(name).catch(() => {});   // a write of another step goes out before the drag moves the page's copy; #249 R02: a refusal shows on the status line
      dragState = {i, startX: px, origT: evs()[i].t, moved:false, sr: S.tracks[name].sr, gen: EDGEN[name] || 0};
      hitDrag = { track: name, index: i };
      window.addEventListener("mouseup", endHitDrag);
      invalidateHits(name); updateStatus();
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
    const dt = xToT(px, laneWidth()) - xToT(dragState.startX, laneWidth());
    if (Math.abs(px-dragState.startX) > 2) dragState.moved = true;
    const i = dragState.i, evsL = evs();
    if (!evsL[i] || dragState.gen !== (EDGEN[name] || 0)) {   // the list was replaced: the drag is void (and says so, as at release)
      const moved = dragState.moved; dragState = null; hitDrag = null; invalidateHits(name);
      if (moved) setMsg("the hits changed during the drag: the move was dropped");
      return;
    }
    evsL[i].t = snapSample(dragState.origT + dt, name);
    invalidateHits(name);
    const dSmp = Math.round((evsL[i].t - dragState.origT)*dragState.sr);
    $("#stSel").textContent = `${name}: moving ${dSmp>0?"+":""}${dSmp} smp`;
    drawInfo();
  });
  spike.addEventListener("mouseleave", () => { spike.classList.remove("onHit"); curOver(false); });
  spike.addEventListener("dblclick", ev => addHitAt(name, snapSample(xToT(laneXOf(ev), laneWidth()), name)));
  spike.addEventListener("contextmenu", ev => {
    const i = hitEvent(laneXOf(ev));
    if (i < 0) { lnContext(ev, name, "spike"); ev.preventDefault(); return; }   // P4-6a: the menus of a mark bar and of the selection; no browser menu on an empty lane
    ev.preventDefault();
    if (followerLocked(name)) return;   // P3-18: a follower's hits are not deleted here
    edDelete(name, evs()[i].id);   // P4-9 stage 2: events_edit delete
  });
  wave.addEventListener("mousedown", ev => {
    if (ev.button!==0) return;
    ev.preventDefault();
    const px = laneXOf(ev);
    if (ev.shiftKey) { lnClearSel(); S.playhead = snapPlayhead(xToT(px, laneWidth())); invalidate(F.HEAD); updateStatus(); return; }
    if (ev.altKey) { lnClearSel(); addHitAt(name, snapSample(xToT(px, laneWidth()), name), true); return; }     // P3-13b: Option-click adds a hit; #144: on the attack
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
  ];
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

/* P3-13c: the key preset ("default" = Pro Tools, or "cubase"); the app setting "key_preset". The page keys follow at once
   (DSKeys.preset); the menu accelerators follow through DSShell.sync (shell_sync carries the preset). */
function setKeyPreset(name, save) {
  if (!DSKeys.setPreset(name)) return false;
  const sel = $("#setKeyPreset");
  if (sel) sel.value = DSKeys.preset;
  if (save !== false && B.appSettingsSet) B.appSettingsSet("key_preset", DSKeys.preset).catch(e => setMsg(`settings: ${errText(e)}`));
  DSShell.sync();
  if (save !== false) setMsg(`key preset: ${DSKeys.presetName()}`);
  return true;
}
/* P4-29: native app feel. The page must not behave like a web page in the webview: no text selection (CSS user-select in
   style.css; here the events that CSS cannot stop: Select All from the native Edit menu, the browser context menu, drags of
   images and text, page zoom by pinch or Cmd+wheel or Cmd+plus). Text fields, text areas, contenteditable and `.selectable`
   text (About, error details) keep their selection, their menu and their drags. The app's own context menus (track header,
   spike lane, wave lane) call preventDefault themselves and stay. */
function nativeFeel() {
  const SEL = "input:not([type=checkbox]):not([type=radio]):not([type=button]):not([type=range]), textarea, [contenteditable=''], [contenteditable='true'], .selectable";
  const inText = t => !!(t && t.closest && t.closest(SEL));
  const node = n => n && (n.nodeType === 1 ? n : n.parentElement);
  document.addEventListener("contextmenu", ev => { if (!inText(ev.target)) ev.preventDefault(); });
  document.addEventListener("dragstart", ev => { if (!inText(ev.target)) ev.preventDefault(); });   // the app's own drags use mouse events, not HTML drag and drop
  document.addEventListener("selectstart", ev => { if (!inText(node(ev.target))) ev.preventDefault(); });
  document.addEventListener("mousedown", ev => {   // P4-29 fix: a double-click selects a word; the stray selection made the
    // selectionchange handler call removeAllRanges, which forced a layout inside the event dispatch (the frames test)
    if (ev.detail > 1 && !inText(ev.target)) ev.preventDefault();
  });
  /* No selectionchange handler here: a removeAllRanges in it forces a layout inside the event
     dispatch (the frames test). The stray selection cannot happen: the mousedown guard above
     stops the multi-click word selection and selectstart stops the drags. */
  for (const t of ["gesturestart", "gesturechange", "gestureend"]) document.addEventListener(t, ev => ev.preventDefault());
  window.addEventListener("wheel", ev => { if (ev.ctrlKey || ev.metaKey) ev.preventDefault(); }, { passive: false });   // pinch arrives as ctrl+wheel; the lanes zoom the timeline with it (P4 #145)
  window.addEventListener("keydown", ev => {
    const mod = ev.metaKey || ev.ctrlKey;
    if (!mod || ev.altKey) return;
    const k = ev.key.toLowerCase();
    if (k === "a" && !ev.shiftKey && !inText(ev.target)) ev.preventDefault();   // Select All outside a field: nothing to select (the app has no Select All of hits)
    else if (["+", "=", "-", "_", "0"].includes(k) && !(window.DSKeys && DSKeys.find(ev))) ev.preventDefault();   // page zoom; the app's own zoom keys stay
  }, true);
}
nativeFeel();


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
function insRoleControls(offer) {   // the role select, the source hint and the note offer of the selected track
  const t = S.tracks[INS.name], sel = $("#insRoleSel");
  if (!t || !sel) return;
  if (!sel.options.length) for (const r of INS_ROLES) { const o = document.createElement("option"); o.value = r; o.textContent = r; sel.appendChild(o); }
  sel.value = INS_ROLES.includes(t.role) ? t.role : "other";
  $("#insRoleSrcRow").classList.remove("hidden");
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
  S.tracks[name].note = r.note;
  S.tracks[name].role_source = r.role_source;
  S.tracks[name].role_match = apply.source === "user" || apply.source === "audio" ? null : (apply.role_match ?? S.tracks[name].role_match ?? null);
  await trReload(!!apply.op, name);   // the lanes, histograms and the panel follow the new role
  refreshHeadRole(name);
  if (!apply.op) pushUndo({ type: "role", sid, track: name, before: r.previous, after: { role, note: r.note, role_source: r.role_source, role_match: S.tracks[name].role_match } });
  insRoleControls(r);
  setMsg(apply.op ? `${name} is a ${role} track again` : `${name} is now a ${role} track${r.reanalyzed ? " (the hits are re-picked)" : ""}`);
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
/* ---------------- global handlers ---------------- */
function setupGlobal() {
  $("#playBtn").onclick = () => playStop();
  $("#stopBtn").onclick = () => stopAndReturn();
  $("#analyzeAllBtn").onclick = () => startAnalysis(null);
  $("#exportAllBtn").onclick = () => openExport("all");
  setupExport();
  $("#progressCancel").onclick = async () => { $("#progressCancel").disabled = true; try { await B.jobCancel(S.sid); setMsg("cancelling…"); } catch(e) { setMsg(`cancel failed: ${e.message}`, "error"); } };   // P3-4d
  $("#relinkFind").onclick = findMovedAudio;
  $("#relinkDismiss").onclick = () => { relinkHidden = S.sid; refreshRelinkBar(); };
  $("#relinkClose").onclick = () => $("#relinkModal").classList.add("hidden");
  $("#relinkAgain").onclick = findMovedAudio;
  // P3-13a: the sidebar, the File menu and the ? button are gone; the native menu (shell.js) runs their actions
  $("#settingsDone").onclick = () => $("#settingsModal").classList.add("hidden");
  plHooks();   // #387: the licence and missing-plugin dialogs of the detector plugins
  pmHooks();
  $("#setOutput").onchange = ev => chooseAudioOutput(ev.target.value);
  const kp = $("#setKeyPreset");   // P3-13c
  for (const [value, text] of Object.entries(DSKeys.PRESETS)) { const o = document.createElement("option"); o.value = value; o.textContent = text; kp.appendChild(o); }
  kp.value = DSKeys.preset;
  kp.onchange = ev => setKeyPreset(ev.target.value);
  $("#setZoomTall").onchange = ev => setZoomTall(ev.target.checked);   // P4-27
  $("#setTabMode").value = SETTINGS().ui.tabMode === "all" ? "all" : "active";   // #175
  $("#setTabMode").onchange = ev => setTabMode(ev.target.value);
  $("#setAuditionTab").checked = !!SETTINGS().ui.auditionTab;   // P4-40d
  $("#setAuditionTab").onchange = ev => setAuditionTab(ev.target.checked);   // #240: one flip for button, checkbox, menu and settings
  setupMaster();   // P3-16
  $("#tempoDetectBtn").onclick = () => DSShell.menu({id: "detect_tempo"});   // owner 2026-10-14: the same as Track > Detect Tempo
  setupInspector();   // P3-14a
  $("#helpDone").onclick = () => $("#helpModal").classList.add("hidden");   // kept for the tests; the manual wires its own buttons in DSHelp.hooks
  DSHelp.hooks();   // #255: the manual (chapters, back/forward, links out)
  $("#undoHistDone").onclick = () => $("#undoHistModal").classList.add("hidden");   // P4-40a
  $("#importCancel").onclick = () => $("#importModal").classList.add("hidden");
  // global settings bindings
  const st = SETTINGS();
  $("#setClickOn").checked = st.click.enabled;
  $("#setClickVol").value = Math.round(st.click.volume*100);
  $("#setClickOn").onchange = ev => setClick(ev.target.checked);
  $("#setClickVol").oninput = ev => { st.click.volume = ev.target.value/100; saveSettings(); };
  const pitchBox = $("#setPitches");
  for (const role of ["kick","snare","tom","hat","cymbal","other"]) {
    const row = document.createElement("div"); row.className = "prow";
    const lab = document.createElement("label"); lab.textContent = role[0].toUpperCase()+role.slice(1);
    const ctl = document.createElement("div"); ctl.className = "pctl";
    const inp = document.createElement("input"); inp.type = "number"; inp.value = st.click.pitches[role] || 1500; inp.step = 100; inp.style.width = "90px";
    inp.onchange = () => { st.click.pitches[role] = Math.max(50, Math.min(8000, +inp.value||1500)); saveSettings(); };
    const unit = document.createElement("span"); unit.className = "punit"; unit.textContent = "Hz";
    ctl.append(inp, unit); row.append(lab, ctl); pitchBox.appendChild(row);
  }
  $("#clickBtn").classList.toggle("on", !!st.click.enabled);
  $("#clickBtn").onclick = () => toggleClick();
  $("#auditionTabBtn").classList.toggle("on", !!st.ui.auditionTab);   // #240
  $("#auditionTabBtn").setAttribute("aria-pressed", st.ui.auditionTab ? "true" : "false");
  $("#auditionTabBtn").onclick = () => toggleAuditionTab();
  $("#followBtn").onclick = () => { const m = ["smooth", "page", "off"]; setFollow(m[(m.indexOf(S.followMode) + 1) % 3]); setMsg(`follow playhead: ${S.followMode}`); };
  $("#snapChk").onchange = ev => setSnap(ev.target.checked);
  $("#raKeep").onclick = () => { if (pendingRA) { const l = pendingRA; pendingRA = null; runAnalysis(l, true); } if (pendingRASel) { const s = pendingRASel; pendingRASel = null; runReanalyzeSelection(s, true); } };
  $("#raDiscard").onclick = () => { if (pendingRA) { const l = pendingRA; pendingRA = null; runAnalysis(l, false); } if (pendingRASel) { const s = pendingRASel; pendingRASel = null; runReanalyzeSelection(s, false); } };
  $("#raCancel").onclick = () => { $("#reanalyzeModal").classList.add("hidden"); pendingRA = null; pendingRASel = null; setMsg("re-analyze cancelled"); };
  document.querySelectorAll(".zBtn").forEach(b => b.onclick = () => zoomPlayhead(b.dataset.z === "in" ? 0.5 : 2));
  $("#zoomSel").onchange = ev => { const v = ev.target.value; ev.target.value = ""; if (v && needProject()) applyZoomSpan(v); };
  $("#ruler").addEventListener("mousedown", rulerDown);
  const ov = $("#overview");
  function ovMode(px) {
    const maxD = maxDur(), W = GEO.ovW;
    const x0 = S.view.t0/maxD*W, x1 = S.view.t1/maxD*W;
    const phX = S.playhead/maxD*W;
    if (Math.abs(px-phX) <= 6) return "scrub";
    if (Math.abs(px-x0) <= 5) return "resizeL";
    if (Math.abs(px-x1) <= 5) return "resizeR";
    return (px > x0 && px < x1) ? "move" : "jump";
  }
  function ovCursor(px) {
    const m = ovMode(px);
    return m==="scrub" ? "col-resize" : (m==="resizeL"||m==="resizeR") ? "ew-resize" : "crosshair";
  }
  ov.addEventListener("mousemove", ev => { ov.style.cursor = ovCursor(ovXOf(ev)); });
  ov.addEventListener("mousedown", ev => {
    if (ev.button!==0) return;
    S.followHold = true;   // F1: any overview interaction is a user view change
    const maxD = maxDur(), W = GEO.ovW;
    const mode = ovMode(ovXOf(ev));
    const startX = ovXOf(ev);
    const t0 = S.view.t0, t1 = S.view.t1;
    if (mode === "scrub") {
      const mv = e2 => { S.playhead = Math.max(0, Math.min(ovXOf(e2)/W*maxD, maxD)); invalidate(F.HEAD); updateStatus(); };
      mv(ev);
      const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
      window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
      return;
    }
    if (mode === "resizeL" || mode === "resizeR") {
      const mv = e2 => {
        const t = Math.max(0, Math.min(ovXOf(e2)/W*maxD, maxD));
        if (mode === "resizeL") setView(Math.min(t, t1-0.002), t1);
        else setView(t0, Math.max(t, t0+0.002));
      };
      const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
      window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
      return;
    }
    if (mode === "jump") centerOn(startX/W*maxD);   // jump, then continue dragging the window
    const anchorT0 = S.view.t0;
    const mv = e2 => {
      const dt = (ovXOf(e2) - startX)/W*maxD;
      // direct manipulation: the rectangle follows the pointer (drag right → view moves right)
      const n0 = Math.max(0, Math.min(anchorT0 + dt, maxD - viewDur()));
      setView(n0, n0 + viewDur());
    };
    const up = () => { window.removeEventListener("mousemove", mv); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", mv); window.addEventListener("mouseup", up);
  });
  ov.addEventListener("wheel", ev => {
    ev.preventDefault();
    const dY = ev.deltaY * dScale(ev.deltaMode), dX = ev.deltaX * dScale(ev.deltaMode);
    if (Math.abs(dX) > Math.max(2, Math.abs(dY)) || ev.shiftKey) {
      const secPerPx = maxDur() / GEO.ovW;
      panView((dX || dY) * secPerPx);
    } else $("#lanesScroll").scrollBy({ top: dY });
  }, {passive:false});

  window.addEventListener("keydown", ev => {
    if (DSShell.menuOwnsKey(ev)) return;   // P3-4b: the native menu runs this key (app only), so it runs once
    // a field keeps its keys (P3-13a: a check box does not, so J still works after a click on the snap box).
    // #436: a slider gives its keys to the app too (Space plays after a click on a slider); only a text
    // field keeps them, and the status line says when a transport key went into a text field.
    const tg = ev.target;
    const keeps = tg.tagName === "SELECT" || tg.tagName === "TEXTAREA" || tg.isContentEditable
        || (tg.tagName === "INPUT" && !["checkbox", "radio", "button", "range"].includes(tg.type));
    if (keeps) {
      const swallowed = DSKeys.find(ev);   // the key has a command: the field takes it, say so
      if (swallowed && (swallowed.cmd.id === "play_stop" || swallowed.cmd.id === "half_speed")) {
        const what = tg.title || tg.id ? `the ${tg.title || tg.id} field` : "the text field";
        setMsg(`"${DSKeys.display(swallowed.key)}" went to ${what}: press Escape and click the lanes to play`);
      }
      return;
    }
    // P3-13b: the key table (static/keymap.js) says which command the key runs
    const hit = DSKeys.find(ev);
    if (!hit) return;
    if (hit.cmd.id !== "deselect") ev.preventDefault();
    if (hit.cmd.menu && DSKeys.parse(hit.key).key === "Tab") DSKeys.notePageRun(hit.cmd.menu);   // #317: see DSKeys.menuOwns
    runKey(hit.cmd.id, ev);
  });
}

/* P3-13b: what a key of the table does in the page. The menu items (shell.js handlers) and these functions
   call the same editor commands; a key differs only where the old key did: Delete without a selected hit
   removes the selected track after a confirm (P4-39a), the nudge keys pan the view when no hit is selected, and Escape deselects. A command that is
   not here (a menu key in the web page, where no native menu runs it) runs its menu handler. */
function nudgeOrPan(step) {   // step in units of the nudge value (P3-20a); a range selection nudges its hits (#151)
  if (LN.sel || S.sel) nudgeUnits(step);
  else panView(Math.sign(step) * viewDur() * 0.2);
}
const KEY_RUN = {
  next_hit: () => gotoEvent(1, false),
  prev_hit: () => gotoEvent(-1, false),
  next_hit_all: () => gotoEvent(1, true),
  prev_hit_all: () => gotoEvent(-1, true),
  review_next: () => reviewStep(1),
  review_prev: () => reviewStep(-1),
  other_mics: () => mxToggle(),   // #157: View > Other Mics Under Hit (O)
  zoom_in: () => zoomStep(true),
  zoom_out: () => zoomStep(false),
  zoom_full: () => zoomPreset(0),
  zoom_30s: () => zoomPreset(1),
  zoom_5s: () => zoomPreset(2),
  zoom_500ms: () => zoomPreset(3),
  zoom_50ms: () => zoomPreset(4),
  zoom_5ms: () => zoomPreset(5),
  play_stop: ({ key }) => playStop(key),
  half_speed: ({ key }) => halfSpeedPlayStop(key),   // P3-20b
  go_start: () => goTo(0),
  go_end: () => goTo(projectEnd()),
  go_play_start: () => goTo(S.playStartPos),
  click: () => toggleClick(),
  snap: () => toggleSnap(),
  follow_toggle: () => toggleFollow(),
  mute_track: () => trackButton(".bMute"),
  solo_track: () => trackButton(".bSolo"),
  delete_hit: () => { if (!tmDeleteSel() && !lnDeleteMark()) { if (LN.sel || S.sel) deleteSelectedHit(); else if (S.selTrack && S.tracks[S.selTrack]) removeTrack(S.selTrack); } },   // P4-39a: no hit, no marker: the selected track (after the confirm)   // P4-6a: a selected Learn mark bar too   // P4-8 and P4-40g: a selected tempo, meter or review marker first   // #151: a range selection deletes its hits
  markers_set: () => markerAtSelection(),   // P4-40g: M sets a marker at the selected hit or the playhead
  marker_next: () => markerJump(1),   // P4-40g: Option+M
  marker_prev: () => markerJump(-1),   // P4-40g: Option+Shift+M
  nudge_earlier: () => nudgeOrPan(-1),
  nudge_later: () => nudgeOrPan(1),
  nudge_earlier_10: () => nudgeOrPan(-10),
  nudge_later_10: () => nudgeOrPan(10),
  deselect: () => { if (auStop()) return; lnClearSel(); if (LN.mark) lnSetMark(null); S.sel = null; if (TM.sel) { TM.sel = null; invalidate(F.VIEW); } requestFrame(); updateStatus(); },   // the frame redraws the track of the old selection
  undo: () => undo(),
  redo: () => redo(),
  prev_track: () => selectTrackStep(-1),   // P3-19
  next_track: () => selectTrackStep(1),
  zoom_toggle: () => zoomToggle(false),
  zoom_keep: () => zoomToggle(true),
  nudge_value_up: () => stepNudgeValue(1),   // P3-20a
  nudge_value_down: () => stepNudgeValue(-1),
  save: () => saveProjectNow().catch(() => {}),   // #249 R02: the refusal is already on the status line
  audition: ({ shift }) => audition(shift),   // P4-40d
};
function runKey(id, ev) {
  const f = KEY_RUN[id] || DSShell.handlers[id];
  if (!f) return false;
  try {
    const r = f({ id, shift: !!(ev && ev.shiftKey), key: ev ? DSKeys.keyOf(ev) : null });
    if (r && typeof r.catch === "function") r.catch(e => setMsg(`${id} failed: ${e.message}`, "error"));
  } catch (e) { setMsg(`${id} failed: ${e.message}`, "error"); }
  return true;
}

function rulerDown(ev) {   // the time ruler and the Bars|Beats ruler (P4-8): a click puts the playhead
  if (ev.button!==0) return;
  ev.preventDefault();
  S.followHold = true;   // F1: repositioning the playhead is a user intent
  const move = e2 => { S.playhead = Math.max(0, snapPlayhead(Math.min(xToT(rulerXOf(e2), GEO.rulerW), maxDur()))); invalidate(F.HEAD); updateStatus(); };
  move(ev);
  const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
  window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
}

/* ---------------- boot ---------------- */
(async function boot() {
  persistInit({ edIdle });   // R17 step 2: persist waits for the edit queue through the injected hook (edit.js owns edIdle since step 3)
  editInit({ pushEvents, voidHitDrag: name => { if (hitDrag && hitDrag.track === name) hitDrag = null; } });   // R17 step 3: edit.js flushes the writes of other steps through the injected hook; voidHitDrag is the input layer's (a replaced hit list voids an in-flight drag)
  sessionInit({ GLQ, lnReset, auReset, tmLoad, selMem: () => selMem, setSelMem: v => { selMem = v; }, exTempoReset: () => { EX.tempoAck = false; } });   // R17 step 6: session.js resets the module state of the owners a load replaces (the GL tile queue, LN/AU/TM, the selection memory) through the injected hooks; #151: the default-tempo answer is per project
  tmInit(); measureGeo(); geoObserve();   // P4-8: the map rulers; P3-14d: the geometry once now, then from the ResizeObserver
  setupUpload(); setupGlobal(); setupInfoLine(); staleSetup();
  if (B.onDirty) {   // P3-4a: the service tells when a project becomes dirty or clean
    let listTimer = null;
    B.onDirty(p => {
      if (p.sid === S.sid) setDirtyState(p.dirty);
      clearTimeout(listTimer);   // one refresh of the project list (its dots) for a burst of events
      listTimer = setTimeout(() => initSessions(true).catch(() => {}), 100);   // P3-4d: keepNone, else a late event reopens a project the user closed
    });
  }
  try {
    await loadViewSettings();   // P3-14a: the Inspector's state, before the first layout the user sees
    await initSessions();
    await loadProject();
    showBackend();
  } catch(e) {
    setMsg(`init failed: ${e.message}`, "error");
  }
  await DSShell.boot();   // P3-4b: menu events, Finder paths (app only)
  B.appReady().catch(() => {});   // P3-17: the first layout is done; the shell shows the window and closes the splash
})();
