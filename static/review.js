/* Drum Spike v2 — hit review (refactor plan step 5.2, R9 A2)
   Tab event navigation, the review queue of doubtful hits (#150, RQ) and the other mics under the selected
   hit (#157, MX). Moved verbatim from app.js. Plain script (no build step), loaded after files.js and
   before app.js. The names stay top-level bindings (page globals). */
"use strict";

/* ---------------- event navigation (Tab) ---------------- */
function allEventsSorted(trackFilter) {
  const out = [];
  for (const name of S.order) {
    if (trackFilter && name !== trackFilter) continue;
    const evs = S.events[name] || [];
    for (let i=0;i<evs.length;i++) out.push({t: evs[i].t, track: name, index: i});
  }
  out.sort((a,b)=>a.t-b.t);
  return out;
}
function gotoEvent(dir, forceGlobal) {
  const trackFilter = (!forceGlobal && S.selTrack) ? S.selTrack : null;
  const all = allEventsSorted(trackFilter);
  if (!all.length) { setMsg(trackFilter ? `no events on ${trackFilter}` : "no events"); return; }
  // jump anchor is ALWAYS the playhead; the selection only tie-breaks simultaneous
  // hits when it sits exactly at the playhead (the normal state right after a jump).
  // Ties are ordered by POSITION IN THE SORTED LIST (D1) — matching the sort order,
  // not an alphabetical key that disagrees with it.
  let refT = S.playhead, refPos = -1;
  if (S.sel && (!trackFilter || S.sel.track === trackFilter)) {
    const e = (S.events[S.sel.track]||[])[S.sel.index];
    if (e && Math.abs(e.t - S.playhead) < 1e-9)
      refPos = all.findIndex(p => p.track === S.sel.track && p.index === S.sel.index);
  }
  // #175: the "Tab moves to" setting. "all found hits" visits the deactivated hits too; the default
  // ("active hits only") skips them and the grey hits of the Learn filter (P4-6c).
  const allHits = SETTINGS().ui.tabMode === "all";
  const grey = p => !allHits && (lnGreyHit(p.track, S.events[p.track][p.index]) || hgIsCut(S.events[p.track][p.index]));
  let target = null;
  if (dir > 0) {
    for (let i=0; i<all.length; i++) {
      const p = all[i];
      if (p.t > refT + 1e-9 || (p.t === refT && i > refPos)) { if (!grey(p)) { target = p; break; } }
    }
    if (!target) { setMsg(trackFilter ? `end of ${trackFilter} events` : "no next event"); return; }
  } else {
    for (let i=all.length-1; i>=0; i--) {
      const p = all[i];
      if (p.t < refT - 1e-9 || (p.t === refT && i < refPos)) { if (!grey(p)) { target = p; break; } }
    }
    if (!target) { setMsg(trackFilter ? `start of ${trackFilter} events` : "no previous event"); return; }
  }
  S.sel = {track: target.track, index: target.index};
  S.playhead = snapSample(target.t, target.track);
  centerOn(S.playhead);
  updateStatus();   // the status bar: "track: hit n/N"; the info line has the rest
  if (SETTINGS().ui.auditionTab && !S.playing) audition(false, { track: target.track, t: target.t });   // P4-40d: Audition on Tab
}

/* ---------------- review queue (#150) ----------------
   D steps through the doubtful hits of all tracks, lowest confidence first; Shift+D steps back. The service rates the
   page's hits (track_confidence, crates/app/src/confidence.rs: a hit under 0.5 is doubtful); the page asks again only
   for a track whose hits changed since its last answer. The info line shows the reasons of a selected doubtful hit. */
var RQ = { sig: {}, rate: {}, at: new WeakMap() };   // per track: the signature and the ratings of the last answer; hit -> {key, r}
function rqKey(e) { return `${e.t}|${e.added ? 1 : 0}|${e.orig}|${e.cut ? 1 : 0}|${e.bleed ? 1 : 0}|${e.filtered ? 1 : 0}`; }
async function rqRatings(n) {
  const evs = S.events[n] || [], sig = evs.map(rqKey).join(",");
  if (RQ.sig[n] !== sig) {
    const r = await B.confidence(S.sid, n, evs);
    RQ.sig[n] = sig; RQ.rate[n] = r.confidence;
  }
  evs.forEach((e, i) => RQ.at.set(e, { key: rqKey(e), r: RQ.rate[n][i] }));
  return RQ.rate[n];
}
async function rqQueue() {   // [{track, e, c}], lowest confidence first; ties by time, then by track order
  const q = [];
  for (const n of S.order) {
    if (!S.tracks[n] || S.tracks[n].follows || !(S.events[n] || []).length) continue;   // a follower's hits are its source's
    const rs = await rqRatings(n);
    S.events[n].forEach((e, i) => { if (rs[i] && rs[i].doubt) q.push({ track: n, e, c: rs[i].c }); });
  }
  return q.sort((a, b) => a.c - b.c || a.e.t - b.e.t || S.order.indexOf(a.track) - S.order.indexOf(b.track));
}
async function reviewStep(dir) {   // Hits > Next Doubtful Hit (dir 1), Previous Doubtful Hit (dir -1)
  if (!S.sid) return;
  let q;
  try { q = await rqQueue(); } catch (err) { setMsg(`review queue: ${errText(err)}`, "error"); return; }
  if (!q.length) { setMsg("no doubtful hits"); return; }
  const cur = S.sel && (S.events[S.sel.track] || [])[S.sel.index];
  const at = cur ? q.findIndex(p => p.e === cur) : -1;   // the selected hit's place; a hit out of the queue starts it again
  const i = at < 0 ? (dir > 0 ? 0 : q.length - 1) : at + dir;
  if (i >= q.length) { setMsg("no more doubtful hits"); return; }
  if (i < 0) { setMsg("no earlier doubtful hits"); return; }
  const p = q[i];
  S.sel = { track: p.track, index: S.events[p.track].indexOf(p.e) };
  S.playhead = snapSample(p.e.t, p.track);
  centerOn(S.playhead);
  updateStatus();
  setMsg(`doubtful hit ${i + 1} of ${q.length}`);
}
function rqWhy(e) {   // the info line text of a doubtful hit whose rating is current, else ""
  const a = e && RQ.at.get(e);
  return a && a.key === rqKey(e) && a.r && a.r.doubt ? `${a.r.c.toFixed(2)}: ${a.r.why.join("; ")}` : "";
}

/* ---------------- other mics under the selected hit (#157) ----------------
   View > Other Mics Under Hit (key O): a panel under the lanes with one small waveform lane per other track, for the
   window of the selected hit (the hit time +-50 ms); the hit time is the centre line. The window is one bridge read per
   track (B.samples); the page keeps no PCM of its own. The panel shows only with a selected hit, and it follows every
   selection, because updateStatus calls mxRefresh. A generation counter drops the reads that a newer selection passed. */
const MX = { on: false, gen: 0 };
const MX_HALF = 0.05, MX_W = 240, MX_H = 40;
function mxToggle() {
  MX.on = !MX.on;
  if (B.appSettingsSet) B.appSettingsSet("other_mics", MX.on).catch(e => setMsg(`settings: ${errText(e)}`));
  mxApply();
  setMsg(`Other mics ${MX.on ? "shown" : "hidden"}`);
}
function mxApply() {   // after a toggle or the settings load: the panel shows or hides, the menu state follows
  document.body.classList.toggle("mxOn", MX.on);
  mxRefresh();
  if (window.DSShell && DSShell.sync) DSShell.sync();
}
function mxRefresh() {   // the panel: one row per other track, filled from the window of the selected hit
  const panel = $("#micsPanel"), sel = S.sel;
  const e = MX.on && sel && (S.events[sel.track] || [])[sel.index];
  panel.classList.toggle("hidden", !e);
  const gen = ++MX.gen;
  if (!e) { panel.replaceChildren(); return; }
  const names = S.order.filter(n => n !== sel.track && S.tracks[n]);
  panel.replaceChildren(...names.map(n => {
    const row = document.createElement("div"), head = document.createElement("span"), label = document.createElement("span"), gain = document.createElement("span"), c = document.createElement("canvas");
    row.className = "mxRow"; row.dataset.name = n;
    head.className = "mxHead";
    label.className = "mxName"; label.textContent = n;
    gain.className = "mxGain";   // the normalisation gain of the lane, set by mxDraw
    head.append(label, gain);
    c.className = "mxCan"; c.width = MX_W; c.height = MX_H;
    row.append(head, c);
    return row;
  }));
  const sid = S.sid;
  for (const n of names) {
    const sr = S.peaks[n] && S.peaks[n].sr;
    if (!sr) continue;   // no peaks: no sample rate, the row stays empty
    const a = Math.max(0, Math.round((e.t - MX_HALF) * sr)), b = Math.round((e.t + MX_HALF) * sr);
    B.samples(sid, n, a, b).then(r => {
      if (MX.gen !== gen || S.sid !== sid) return;   // a newer selection, or another project: drop the read
      const c = panel.querySelector(`.mxRow[data-name="${CSS.escape(n)}"] canvas`);
      if (c) {
        let peak = 0;   // the lane scales to its own peak: a bleed-level signal still reads
        for (let i = 0; i < r.data.length; i++) { const v = Math.abs(r.data[i]); if (v > peak) peak = v; }
        const db = peak > 1e-6 ? `+${(20 * Math.log10(1 / peak)).toFixed(0)} dB` : "−∞ dB";
        const g = panel.querySelector(`.mxRow[data-name="${CSS.escape(n)}"] .mxGain`);
        if (g) g.textContent = db;
        mxDraw(c, r, (e.t * sr - r.start) / r.data.length, S.tracks[n].color, peak);
      }
    }, () => {});
  }
}
function mxDraw(canvas, r, hitAt, color, peak) {   // r: {start, data} of the window; hitAt: the hit's place in the window (0 to 1); peak: the normalisation peak (1 = full scale)
  const ctx = canvas.getContext("2d"), W = canvas.width, H = canvas.height, d = r.data, n = d.length;
  const k = peak > 0 ? 1 / peak : 0;   // samples of a quiet lane reach the full lane height
  ctx.clearRect(0, 0, W, H);
  ctx.fillStyle = "rgba(0,0,0,0.22)"; ctx.fillRect(0, 0, W, H);
  ctx.fillStyle = color;
  for (let x = 0; x < W; x++) {   // the min and the max of the samples of one pixel column
    const i0 = Math.floor(x * n / W), i1 = Math.max(i0 + 1, Math.floor((x + 1) * n / W));
    let lo = 1, hi = -1;
    for (let i = i0; i < i1 && i < n; i++) { if (d[i] < lo) lo = d[i]; if (d[i] > hi) hi = d[i]; }
    if (hi < lo) continue;
    const y0 = (1 - hi * k) * H / 2, y1 = (1 - lo * k) * H / 2;
    ctx.fillRect(x, y0, 1, Math.max(1, y1 - y0));
  }
  ctx.fillStyle = "#ffd24d";   // the hit time
  ctx.fillRect(Math.round(hitAt * W), 0, 1, H);
}
