/* Drum Spike v2 — velocity (refactor plan step 5.2, R9 A2)
   The velocity lane (VL): the toggle, the row canvas, the drawing, the stalk gestures, the
   height drag and the group scale. Moved verbatim from app.js. Plain script (no build step),
   loaded after layer.js and before app.js. The names stay top-level bindings (page globals). */
"use strict";

/* ---------------- velocity lane (P4-40f; openDAW O-3, own design from the report's description) ----------------
   View > Velocity Lane: a thin lane (22 px) under the spike lane of every track, one stalk per hit, the height is the
   velocity 1 to 127. Stalks of hits that leave the output (cut, tom bleed, Learn filter; hgIsCut) are grey, the rest in
   the track colour. The two dashed lines are the floor and the ceiling of the loudness settings (P4-3): the velocity
   ramp maps the floor to 20 and the ceiling to 127, so they sit at the 20 and the 127 level of the lane.
   Drag a stalk: the velocity changes. Option-drag draws a straight line over all hits in the range; Cmd-drag draws
   freehand (the stalks under the pointer follow the pointer's height). One gesture is one undo step: one events_edit
   call with one `velocity` op per hit and one step (a single hit: setv; several: setv-many, label
   "Set Velocity (track, n hits)"). Every hand velocity is sent with `edited: true`: the hit keeps the manual colour and
   survives Recompute Velocities and a re-pick (merge_edited, is_kept). The lane is a view setting (app setting
   velocity_lane, default off); it draws with the hits of its row (drawHits), so the edits show live.
   #208: the lane is 80 px by default and its height is the app setting velocity_lane_height (drag its bottom border,
   24 to 400 px). A header "Velocity" with a 127 / 64 / 0 scale, a dot on each stalk top, the selected stalk and the
   hits of a selected range in yellow, a hover label (velocity, time, why a stalk is grey); the
   gesture hint is the Velocity label's tooltip (#245). A drag of a selected stalk scales all
   selected hits by the same ratio. */
const VL_H_DEF = 80, VL_H_MIN = 24, VL_H_MAX = 400;
const VL = { on: false, drag: null, h: VL_H_DEF, hover: null };   // drag: {track, mode, gen, x0, y0, set: Map(id -> [from, to]), anchor, v0, base}; hover: {track, id}
/* #245: the gesture hint lives on the Velocity label as a tooltip (a title on .velLabel), not
   drawn into the lane canvas: text over the stalks read like data. */
const VL_HINT = "Drag a stalk to set velocity. Option-drag: draw a line. Cmd-drag: draw freehand.";
function vlToggle() {
  VL.on = !VL.on;
  if (B.appSettingsSet) B.appSettingsSet("velocity_lane", VL.on).catch(e => setMsg(`settings: ${errText(e)}`));
  vlApply();
  setMsg(`Velocity lane ${VL.on ? "shown" : "hidden"}`);
}
function vlApply() {   // after a toggle: the rows grow or shrink, the canvases resize in the frame
  document.body.classList.toggle("velOn", VL.on);
  for (const row of document.querySelectorAll("#lanesScroll .track")) vlSyncRow(row);   // P4-40f: the vel canvas exists only while the lane is on
  if (typeof spLeave === "function") spLeave("vel");
  invalidate(F.SIZE | F.VIEW | F.GL);
  if (window.DSShell && DSShell.sync) DSShell.sync();
}
/* #245: the velocity lane's canvas lives in a .velWrap with the Velocity label span (the gesture
   hint is the label's tooltip): both the row template and vlSyncRow build the same pair. */
function velWrapHtml() {
  return `<div class="velWrap"><canvas class="lane vel" height="${VL.h}"></canvas>` +
         `<span class="velLabel" title="${VL_HINT}">Velocity</span></div>`;
}
function velLabelNode() {
  const label = document.createElement("span");   // the tooltip of the Velocity label: the gesture hint (#245)
  label.className = "velLabel";
  label.title = VL_HINT;
  label.textContent = "Velocity";
  return label;
}
function vlSyncRow(row) {   // the row gets or loses its vel canvas (releaseCanvases gives the store back at once)
  let c = row.querySelector(".vel");
  if (VL.on && !c) {
    c = document.createElement("canvas");
    c.className = "lane vel"; c.height = VL.h;
    const wrap = document.createElement("div");   // #245: wraps the canvas so the label sits on it, not in it
    wrap.className = "velWrap";
    wrap.appendChild(c);
    wrap.appendChild(velLabelNode());
    row.querySelector(".wave").after(wrap);
    vlBind(row, row.dataset.name);
  } else if (!VL.on && c) {
    if (c.width) { c.width = 0; c.height = 0; }
    const wrap = c.closest(".velWrap");
    if (wrap) wrap.remove(); else c.remove();
  }
  for (const hd of row.querySelectorAll(".rowResize")) {   // #437: the handles' text follows the lane it drags (the row's border and the header's edge, #439)
    hd.title = VL.on ? "Drag: the height of the velocity lane (the handle is at its bottom edge). Double-click: the wave lane back to the Track Height"
                     : "Drag: the height of this track's wave lane. Double-click: back to the Track Height";
  }
}
function vlYOf(v, H) { return Math.round(H - (v - 1) / 126 * (H - 4) - 2); }   // the y of a velocity (2 px padding)
function vlVOf(y, H) { return Math.max(1, Math.min(127, Math.round(1 + (H - 2 - y) / (H - 4) * 126))); }
function drawVelLane(canvas, name) {
  const ctx = canvas.getContext("2d"), [W, H] = laneSize(canvas);
  ctx.clearRect(0, 0, W, H);
  ctx.fillStyle = "rgba(0,0,0,0.22)"; ctx.fillRect(0, 0, W, H);
  const color = trackColor(name);
  // the floor and the ceiling of the loudness settings (P4-3): the ramp maps them to the velocities 20 and 127
  ctx.strokeStyle = "rgba(138,147,163,0.4)"; ctx.setLineDash([3, 3]); ctx.lineWidth = 1;
  for (const v of [20, 127]) { const y = vlYOf(v, H) + 0.5; ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(W, y); ctx.stroke(); }
  ctx.setLineDash([]);
  // #208: the scale at the right edge (127 top, 64 middle, 0 bottom) and the name at the left
  ctx.fillStyle = "rgba(138,147,163,0.9)"; ctx.font = "10px sans-serif"; ctx.textBaseline = "middle";
  // the header (127, Velocity) hangs below the 127 line, so its full glyph height is inside the lane (#208)
  const headY = vlYOf(127, H) + 4;
  ctx.textAlign = "right"; ctx.fillText("127", W - 3, headY);
  ctx.textBaseline = "middle";
  ctx.textAlign = "right"; ctx.fillText("64", W - 3, vlYOf(64, H)); ctx.fillText("0", W - 3, H - 6);
  const evs = S.events[name] || [];
  const clips = HT.clips(name);   // #555: the clips of the DAW track: the hits of every source of the row at their song time
  const pad = 8 * viewDur() / W;
  const refs = clips ? HT.hitRefs(clips, S.events, S.view.t0 - pad, S.view.t1 + pad, hitDrag)
    : hitsIn(evs, S.view.t0 - pad, S.view.t1 + pad, dragSkip(name)).map(i => ({ src: name, i, t: evs[i].t }));
  for (const h of refs) {
    const e = (S.events[h.src] || evs)[h.i], i = h.i, x = Math.round(tToX(h.t, W));
    if (x < -2 || x > W + 2) continue;
    const range = LN.sel && LN.sel.track === h.src ? LN.sel : null;
    const out = hgIsCut(e), sel = S.sel && S.sel.track === h.src && S.sel.index === i;
    const inRange = !!range && e.t >= range.t0 && e.t <= range.t1;
    ctx.fillStyle = out ? HG_GREY : e.edited || isManual(e) ? MANUAL_COLOR : color;
    const y = vlYOf(e.v, H);
    ctx.fillRect(x - 1, y, 2, H - y);
    ctx.beginPath(); ctx.arc(x, y, 2.2, 0, 2 * Math.PI); ctx.fill();   // the dot at the stalk top
    if (inRange) { ctx.fillStyle = "#ffd24d"; ctx.beginPath(); ctx.arc(x, y, 2.6, 0, 2 * Math.PI); ctx.fill(); }   // a hit of the selected range
    if (sel) { ctx.strokeStyle = "#ffd24d"; ctx.lineWidth = 1.5; ctx.strokeRect(x - 3, y - 3, 6, H - y + 3); ctx.lineWidth = 1; }
  }
  if (VL.drag && VL.drag.track === name && VL.drag.mode === "line") {   // the line the Option drag draws
    const d = VL.drag, x0 = tToX(d.t0, W), x1 = d.curX;
    ctx.strokeStyle = "rgba(255,255,255,0.55)"; ctx.beginPath(); ctx.moveTo(x0, d.y0); ctx.lineTo(x1, d.curY); ctx.stroke();
  }
  const hvSrc = VL.hover && (clips ? clips.some(c => c.src === VL.hover.track) : VL.hover.track === name) ? VL.hover.track : null;
  const hv = hvSrc ? (S.events[hvSrc] || []).find(x => x.id === VL.hover.id) : null;
  if (hv) {   // the hover label: the velocity, the time, and why a grey stalk is grey
    const hc = clips ? clips.find(c => c.src === hvSrc && hv.t >= c.a && hv.t < c.b) : null;
    const x = Math.round(tToX(hc ? HT.toSong(hc, hv.t) : hv.t, W));
    const txt = `velocity ${hv.v} · ${fmt(hv.t)} s${hgIsCut(hv) ? " · grey: leaves the output (cut)" : ""}`;
    const tw = ctx.measureText(txt).width + 8, bx = Math.max(2, Math.min(x + 6, W - tw - 2));
    ctx.fillStyle = "rgba(16,20,28,0.92)"; ctx.fillRect(bx, H - 15, tw, 13);
    ctx.fillStyle = "#e6e9ef"; ctx.textAlign = "left"; ctx.fillText(txt, bx + 4, H - 8);
  }
  ctx.textAlign = "left";
}
/* The gestures of one row's velocity lane. A gesture previews on the page's copy and commits once: one events_edit call,
   one undo step. `gen` guards the commit like a hit drag does: the hits changed meanwhile, the gesture is dropped. */
function vlBind(row, name) {
  const c = row.querySelector(".vel");
  if (!c) return;   // the lane is off: the row has no vel canvas (vlSyncRow binds when it appears)
  let hitSrc = name;   // the source of the stalk that hitAt found
  let cur = name;   // #555: the source that a gesture edits: in a DAW the source of the clip that holds the hit (standalone: this track)
  const evs = () => S.events[cur] || [];
  const hitsOf = (t0, t1, skip) => {   // the hits of the current source in [t0, t1] of the page time: [{i, t}] (t on the page); skip is the hit that the old call left out (here -1: none)
    const clips = HT.clips(name);
    if (clips) return HT.hitRefs(clips, S.events, t0, t1, hitDrag).filter(r => r.src === cur);
    return hitsIn(evs(), t0, t1, skip).map(i => ({ i, t: evs()[i].t }));
  };
  const hitAt = px => {
    const W = laneWidth(), t = xToT(px, W), d = 4 * viewDur() / W, clips = HT.clips(name);
    hitSrc = name;
    if (clips) {   // #555: the nearest stalk of any source of the DAW track; the gesture edits that source
      let best = -1, bd = 4;
      for (const r of HT.hitRefs(clips, S.events, t - d, t + d, hitDrag)) { const dx = Math.abs(tToX(r.t, W) - px); if (dx < bd) { bd = dx; best = r.i; hitSrc = r.src; } }
      return best;
    }
    for (const i of hitsIn(evs(), t - d, t + d, dragSkip(name))) if (Math.abs(tToX(evs()[i].t, W) - px) < 4) return i;
    return -1;
  };
  const preview = (e, v) => { e.v = v; };
  const commit = () => {
    const d = VL.drag; VL.drag = null;
    if (!d) return;
    const list = [...d.set.entries()].map(([id, [from, to, was]]) => ({ id, from, to, ...(was ? { was } : {}) })).filter(s => s.to !== s.from);
    const name = d.track;   // #555: the source the gesture edited
    if (!list.length || d.gen !== (EDGEN[name] || 0)) { invalidateHits(name); if (list.length) setMsg("the hits changed during the drag: the velocities were dropped"); return; }
    const step = list.length === 1
      ? { type: "setv", track: name, id: list[0].id, from: list[0].from, to: list[0].to, ...(list[0].was ? { was: list[0].was } : {}) }
      : { type: "setv-many", track: name, list };
    edCall(name, evs2 => {
      const ops = [];
      for (const s of list) { const j = edIndex(evs2, s.id); if (j < 0) { setMsg("edit: the hit is not there any more"); return null; } ops.push({ op: "velocity", index: j, v: s.to, edited: true }); }
      return { ops, step };   // P4-40f: one gesture, one undo step
    }).then(ok => { if (!ok) return edReload(S.sid, name); invalidateHits(name); updateStatus(); });
  };
  const setRange = (x0, v0, x1, v1) => {   // the Option drag: a straight line from (x0, v0) to (x1, v1) over all hits between
    const W = laneWidth(), evsL = evs();
    const fOf = x => (x1 === x0 ? 1 : (x - x0) / (x1 - x0));
    const ta = Math.min(xToT(x0, W), xToT(x1, W)), tb = Math.max(xToT(x0, W), xToT(x1, W));
    for (const { i, t } of hitsOf(ta, tb, -1)) {
      const v = Math.round(v0 + (v1 - v0) * fOf(tToX(t, W)));
      const id = evsL[i].id;
      if (!VL.drag.set.has(id)) VL.drag.set.set(id, [evsL[i].v, evsL[i].v, hitWas(evsL[i])]);   // R05: the field presence before the gesture
      preview(evsL[i], v); VL.drag.set.get(id)[1] = v;
    }
  };
  c.addEventListener("mousedown", ev => {
    if (ev.button === 2 || !VL.on || !S.sid || followerLocked(name)) return;
    ev.preventDefault();
    cur = name;
    const px = laneXOf(ev), [W, H] = laneSize(c), hi = hitAt(px);
    if (hi >= 0) cur = hitSrc;
    else if (HT.clips(name)) { const w0 = HT.toSource(HT.clips(name), xToT(px, W)); if (w0) cur = w0.clip.src; }   // a line or a free gesture starting off a stalk edits the source of the clip under the pointer
    const src = cur;
    const mode = ev.metaKey || ev.ctrlKey ? "free" : ev.altKey ? "line" : "stalk";
    if (mode === "stalk" && hi < 0) return;
    const t0 = xToT(px, W);
    const rect = c.getBoundingClientRect();   // cached: the row does not move during a gesture (offsetY lies once the pointer leaves the canvas)
    VL.drag = { track: src, mode, gen: EDGEN[src] || 0, x0: px, y0: ev.offsetY, curX: px, curY: ev.offsetY, t0, set: new Map(), rect };
    if (hasPendingWrite(src)) pushEvents(src).catch(() => {});   // a write of another step goes out before the drag moves the page's copy; #249 R02: a refusal shows on the status line
    if (mode === "stalk") {
      const e = evs()[hi], grp = vlGroup(src, e);   // #208: a selected stalk moves the selection, scaled
      VL.drag.anchor = e.id; VL.drag.v0 = e.v; VL.drag.base = grp.length > 1 ? new Map(grp.map(g => [g.id, g.v])) : null;
      for (const g of grp) VL.drag.set.set(g.id, [g.v, g.v, hitWas(g)]);
      preview(e, vlVOf(ev.offsetY, H)); VL.drag.set.get(e.id)[1] = e.v;
      if (VL.drag.base) vlScale(VL.drag, evs(), VL.drag.set.get(e.id)[1]);
    }
    invalidateHits(src);
    const move = e2 => {
      const d = VL.drag; if (!d || d.track !== src) return;
      d.curX = e2.clientX - d.rect.left; d.curY = e2.clientY - d.rect.top;
      const evsL = evs(), W2 = laneWidth();
      if (d.mode === "line") {
        setRange(tToX(d.t0, W2), vlVOf(d.y0, H), d.curX, vlVOf(d.curY, H));
      } else if (d.mode === "free") {
        const v = vlVOf(d.curY, H);
        for (const { i, t } of hitsOf(S.view.t0 - 1e-6, S.view.t1 + 1e-6, -1)) {   // the hits crossed since the last move
          const x = tToX(t, W2);
          if (Math.abs(x - d.curX) < 4) {
            const id = evsL[i].id;
            if (!d.set.has(id)) d.set.set(id, [evsL[i].v, evsL[i].v]);
            preview(evsL[i], v); d.set.get(id)[1] = v;
          }
        }
      } else if (d.base) {   // #208: the selected hits scale by the ratio of the dragged stalk
        const i = evsL.findIndex(x => x.id === d.anchor);
        if (i >= 0) { preview(evsL[i], vlVOf(d.curY, H)); d.set.get(d.anchor)[1] = evsL[i].v; vlScale(d, evsL, evsL[i].v); }
      } else {
        const i = evsL.findIndex(x => x.id === d.anchor);
        if (i >= 0) { preview(evsL[i], vlVOf(d.curY, H)); d.set.get(d.anchor)[1] = evsL[i].v; }
      }
      invalidateHits(src);
      if (d.mode === "stalk") {   // the value while the stalk moves
        const n = [...d.set.values()].filter(s => s[1] !== s[0]).length;
        $("#stSel").textContent = d.base
          ? `${trackLabel(src)}: velocity ${d.set.get(d.anchor)[1]}, ${d.base.size} hits scaled by ${(d.set.get(d.anchor)[1] / d.v0).toFixed(2)}`
          : `${trackLabel(src)}: velocity ${d.set.get(d.anchor)[1]}`;
        return;
      }
      const n = [...d.set.values()].filter(s => s[1] !== s[0]).length;
      $("#stSel").textContent = `${trackLabel(src)}: Set Velocity (${n} ${n === 1 ? "hit" : "hits"})`;
    };
    const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); commit(); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  });
  c.addEventListener("mousemove", ev => {
    const hi = VL.on ? hitAt(laneXOf(ev)) : -1, hsrc = hitSrc;
    c.classList.toggle("onStalk", hi >= 0);
    const id = hi >= 0 ? (S.events[hsrc] || [])[hi].id : null;
    const was = VL.hover;
    if (id !== (was && was.track === hsrc ? was.id : null)) {   // the hover label follows the stalk under the pointer
      VL.hover = id ? { track: hsrc, id } : null;
      if (was || id) drawVelLane(c, name);
    }
    curOver(false, ev);
  });
  c.addEventListener("mouseleave", () => {
    c.classList.remove("onStalk", "onTop");
    const hc = HT.clips(name);
    if (VL.hover && (hc ? hc.some(x => x.src === VL.hover.track) : VL.hover.track === name)) { VL.hover = null; drawVelLane(c, name); }
  });
}
/* #437: the lane's resize handle is the row's bottom border (the .rowResize div sits on the velocity
   lane's bottom edge while the lane is on); it drags the lane's height, like the wave lane's row drag. */
function vlResize(ev) {   // the drag of the bottom border: the lane grows when the pointer goes down; saved when let go
  const y0 = ev.clientY, h0 = VL.h;
  const move = e2 => {
    VL.h = Math.max(VL_H_MIN, Math.min(VL_H_MAX, h0 + (e2.clientY - y0)));
    invalidate(F.SIZE | F.VIEW | F.GL);
  };
  const up = () => {
    window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up);
    if (B.appSettingsSet) B.appSettingsSet("velocity_lane_height", VL.h).catch(e => setMsg(`settings: ${errText(e)}`));
  };
  window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
}
/* #208: the hits a stalk drag moves: the hits of the selected range (or the one selected hit) when the stalk is one of
   them; else the stalk alone. */
function vlGroup(name, e) {
  const evsL = S.events[name] || [];
  let list;
  if (LN.sel && LN.sel.track === name) list = evsL.filter(x => x.t >= LN.sel.t0 && x.t <= LN.sel.t1);
  else if (S.sel && S.sel.track === name && evsL[S.sel.index]) list = [evsL[S.sel.index]];
  else list = [];
  return list.includes(e) && list.length > 1 ? list : [e];
}
/* #208: the scale of a stalk drag: every selected hit gets its velocity times the ratio of the dragged stalk, rounded
   and kept in 1..127; the dragged hit gets the value of the pointer. */
function vlScale(d, evsL, newV) {
  const ratio = newV / d.v0;
  for (const [id, base] of d.base) {
    if (id === d.anchor) continue;
    const i = evsL.findIndex(x => x.id === id);
    if (i < 0) continue;
    const v = Math.max(1, Math.min(127, Math.round(base * ratio)));
    evsL[i].v = v; d.set.get(id)[1] = v;
  }
}
