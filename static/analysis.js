/* Drum Spike v2 — analysis (refactor plan step 5.2, R9 A2)
   The events reload after an analysis, the progress indicator (also of the copy jobs), Re-analyze Selection
   (#265), the analysis run and its job poll, and the detector and error report of a run. Moved verbatim
   from app.js. Plain script (no build step), loaded after review.js and before app.js. The names stay
   top-level bindings (page globals). */
"use strict";

/* ---------------- events reload (the analysis wrote new events into the open project) ---------------- */
async function loadEventsOnly() {
  S.undo.length = 0; S.redo.length = 0;   // B2: stacks reference pre-analysis state
  for (const n of S.order) S.events[n] = normEvents(await B.events(S.sid, n));
  S.sel = null; setDirtyState(true);   // the analysis wrote new events into the open project
  invalidateHits(); updateStatus(); refreshCounts();
}

/* ---------------- progress indicator ---------------- */
function flashBtn(el) {   // brief highlight on the button a keyboard shortcut triggered
  if (!el) return;
  el.classList.remove("flash");
  void el.offsetWidth;          // restart the animation on rapid presses
  el.classList.add("flash");
  clearTimeout(el._flashT);
  el._flashT = setTimeout(() => el.classList.remove("flash"), 500);
}
function showProgress(pct, text, cancelable) {   // pct 0..100, or -1 = indeterminate; cancelable: the button stops the running job (P3-4d)
  const w = $("#progressWrap"); w.classList.remove("hidden");
  w.classList.toggle("cancelable", !!cancelable);
  const cb = $("#progressCancel"); cb.classList.toggle("hidden", !cancelable);
  if (!cancelable) cb.disabled = false;
  const bar = $("#progressBar");
  // 0% determinate is invisible — animate until there is real progress to show
  bar.classList.toggle("indeterminate", pct <= 0);
  if (pct > 0) bar.style.width = pct + "%";
  NOTE.base = text || "";
  $("#progressText").textContent = NOTE.base + (NOTE.until > Date.now() ? ` · ${NOTE.text}` : "");
}
const NOTE = { base: "", text: "", until: 0 };   // a refusal while a job runs: the status message is hidden then, so the progress text carries it for a few seconds
function jobNote(text) {
  setMsg(text);
  NOTE.text = text; NOTE.until = Date.now() + 5000;
  if (!$("#progressWrap").classList.contains("hidden")) $("#progressText").textContent = `${NOTE.base} · ${text}`;
}
function hideProgress() { $("#progressWrap").classList.add("hidden"); $("#progressWrap").classList.remove("cancelable"); $("#progressCancel").classList.add("hidden"); }
/* P3-4d: the progress message of a copy job {state, done, total, bytes_done, bytes_total, file} on the bar, with Cancel */
function copyProgress(label) {
  return p => {
    const pct = p.bytes_total ? (100 * p.bytes_done) / p.bytes_total : (100 * p.done) / Math.max(1, p.total);
    const n = Math.min(p.done + (p.state === "running" ? 1 : 0), p.total);
    showProgress(Math.max(1, Math.round(pct)), `${label} ${n} of ${p.total}${p.file ? " - " + p.file : ""}`, true);
  };
}
function setAnalysisButtons(disabled) {
  $("#analyzeAllBtn").disabled = disabled;
  $("#analyzeAllBtn").classList.toggle("running", disabled);   // 0.9.3: the accent colour while an analysis runs
  document.querySelectorAll(".bReanalyze").forEach(b => b.disabled = disabled);
  const rm = document.querySelector("#ctxMenu [data-act=reanalyze]"); if (rm) rm.disabled = disabled;
}

/* ---------------- Re-analyze Selection (#265) ----------------
   Hits > Re-analyze Selection, Shift+R and the range context menu entry: the hits of the selected
   range (LN.sel) only, on its track. The service runs the detector with a 100 ms pad, takes only
   the hits inside [t0, t1], leaves the hits outside bit-identical and refreshes the followers; the
   answer carries the whole event list and the info line text. Edited hits inside the range follow
   the Keep dialog (its text names the range); the run is one undo step "Re-analyze Selection"
   (before/after, like a re-pick), the backup of the previous events is the service's. */
let pendingRASel = null;   // the range of a Re-analyze Selection waiting for the Keep answer
function reanalyzeSelected() {
  if (!needProject()) return;
  const s = LN.sel && LN.sel.t1 > LN.sel.t0 ? LN.sel : null;
  if (!s) { setMsg("no range selected: drag a wave lane first"); return; }
  if (analysisRunning) { setMsg("analysis already running — wait for it to finish"); return; }
  if (followerLocked(s.track)) return;
  shSerial(async () => {
    if (hasPendingWrite(s.track)) await settleEvents(s.track);
    let inside = 0, kept = 0;
    try {
      const r = await B.events(S.sid, s.track);
      const stored = Array.isArray(r) ? r : (r && r.events) || [];
      inside = stored.filter(e => e.t >= s.t0 - 1e-9 && e.t <= s.t1 + 1e-9).length;
      kept = stored.filter(e => e.t >= s.t0 - 1e-9 && e.t <= s.t1 + 1e-9 && isKept(e)).length;
    } catch (e) { setMsg(`re-analyze: ${errText(e)}`, "error"); return; }
    if (!inside) { setMsg(`no hits in the range (${fmt(s.t0, 3)} to ${fmt(s.t1, 3)} s)`); return; }
    if (kept) {   // rule 4: the Keep dialog opens only when edited hits are in the range, and names the range
      $("#reanalyzeMsg").innerHTML = "";
      const p = $("#reanalyzeMsg");
      p.appendChild(document.createTextNode(
        `Re-analyzing ${s.track} from ${fmt(s.t0, 3)} to ${fmt(s.t1, 3)} s: ` +
        `${inside} ${inside === 1 ? "hit" : "hits"} in the range, ${kept} edited.`));
      $("#reanalyzeModal").classList.remove("hidden");
      pendingRASel = { track: s.track, t0: s.t0, t1: s.t1 };
      return;
    }
    return runReanalyzeSelection(s, false);
  });
}
async function runReanalyzeSelection(s, keep) {
  $("#reanalyzeModal").classList.add("hidden");
  const name = s.track;
  try {
    const before = (S.events[name] || []).map(e => ({ ...e }));   // the page's copy with ids: the undo step's before
    setMsg("re-analyzing the range…");
    const r = await B.reanalyzeSelection(S.sid, name, s.t0, s.t1, !!keep);
    const old = S.events[name] || [];
    S.events[name] = adoptHits(r.events, old, [], []);
    EDGEN[name] = (EDGEN[name] || 0) + 1;
    if (S.sel && S.sel.track === name) S.sel = null;
    pushUndo({ type: "reanalyze", track: name, t0: s.t0, t1: s.t1,
      before: { events: before }, after: { events: (S.events[name] || []).map(e => ({ ...e })) } });
    setDirtyState(true);   // the service stored the merged hits
    invalidateHits(name); refreshCounts(name); updateStatus();
    setMsg(r.info);   // rule 7: the track, the range and "N hits replaced by M"
    reloadFollowers(Array.isArray(r.followers) ? r.followers : followersOf(name));
    if (INS.name === name) syncInspector(true);
  } catch (e) { setMsg(`re-analyze failed: ${errText(e)}`, "error"); }
}
function raApplyOp(op, reverse) {   // applyOp: the undo and redo of a Re-analyze Selection (before/after, like a re-pick)
  const st = reverse ? op.before : op.after;
  S.events[op.track] = st.events.map(e => ({ ...e }));
  if (S.sel && S.sel.track === op.track) S.sel = null;
  markDirty(op.track); invalidateHits(op.track); refreshCounts(op.track); updateStatus();
  if (INS.name === op.track) syncInspector(true);
}

/* ---------------- analysis ---------------- */
let pollTimer = null;
let analysisRunning = false;
let pendingKeep = false;   // B4, P4-9: the analysis keeps the edited hits (the service merges them); only for the message
function startAnalysisGo(tracks) {
  if (!S.order.length) { setMsg("no tracks"); return; }
  if (analysisRunning) { setMsg("analysis already running — wait for it to finish"); return; }
  const list = tracks || S.order;
  const withEvents = list.filter(n => (S.events[n]||[]).length > 0);
  if (withEvents.length) {
    const keptTotal = withEvents.reduce((a,n) => a + (S.events[n]||[]).filter(isKept).length, 0);
    if (keptTotal === 0) {
      if (!confirm(`Re-analyzing ${withEvents.length} track(s) overwrites their events (a backup is kept). Continue?`)) return;
      runAnalysis(list, false);
    } else {
      $("#reanalyzeMsg").innerHTML = "";
      const p = $("#reanalyzeMsg");
      p.appendChild(document.createTextNode(
        `Re-analyzing ${withEvents.length} track(s): ` +
        withEvents.map(n => `${n} (${(S.events[n]||[]).filter(isKept).length} edited)`).join(", ") + "."));
      $("#reanalyzeModal").classList.remove("hidden");
      pendingRA = list;   // runAnalysis is called by the modal buttons
    }
  } else runAnalysis(list, false);
}
let pendingRA = null;
function runAnalysis(list, keepManual) {
  $("#reanalyzeModal").classList.add("hidden");
  pendingKeep = !!keepManual && list.some(n => (S.events[n]||[]).some(isKept));
  const params = {};
  for (const n of list) params[n] = S.tracks[n].params || defaultParams(S.tracks[n].role);
  setMsg("analysis queued…");
  analysisRunning = true; setAnalysisButtons(true);
  showProgress(0, "queued…");
  flushEvents().then(() => B.analyze(S.sid, list, params, !!keepManual))   // P4-9: the service holds the edits, keep_edited merges them
    .catch(e => { throw e.status ? new Error(`analyze failed: ${e.message}`) : e; })   // C3; P4-3 round 2: the service's text (409: a re-pick is being stored)
    .then(() => pollJob())
    .catch(e => { analysisRunning = false; setAnalysisButtons(false); hideProgress(); setMsg(/^analyze failed/.test(e.message || "") ? e.message : `analyze failed: ${errText(e)}`, "error"); });   // #249 R02: a refused write is named like the rest
}
function pollJob() {
  clearTimeout(pollTimer);
  B.job(S.sid).then(st => {
    if (st.state === "running" || st.state === "queued") {
      const names = Object.keys(st.tracks || {});
      const done = names.filter(n => (st.tracks[n].state||"") === "done").length;
      const failed = names.filter(n => (st.tracks[n].state||"") === "error").length;
      const total = Math.max(1, names.length);
      const cur = names.find(n => (st.tracks[n].state||"") === "running");
      const stage = cur ? (st.tracks[cur].stage || "analyzing") : st.state;
      showProgress(Math.round(100*(done+failed)/total),
        `Analyzing ${done+failed}/${total}` + (cur ? ` — ${cur}: ${stage}` : "") + (failed ? ` (${failed} failed)` : ""));
      pollTimer = setTimeout(pollJob, 800);
      return;
    }
    analysisRunning = false; setAnalysisButtons(false);
    if (st.state === "done") {
      showDetector(st.tracks, st.bleed);
      const bad = failedTracks(st.tracks), total = Object.keys(st.tracks || {}).length;
      const kept = pendingKeep; pendingKeep = false;
      showProgress(100, bad.length ? `analysis done: ${bad.length} of ${total} failed` : "analysis done");
      setTimeout(hideProgress, bad.length ? 8000 : 1500);
      loadEventsOnly().then(() => refreshTrackMeta()).then(() => {   // P3-18: the params the retry changed, the follows
        if (kept) setMsgBg("analysis done — edited hits kept");
        markTrackErrors(st.tracks);
        const weak = weakSignals(st.tracks);   // P3-18: "weak signal: sensitivity raised to ..." or "no hits found"
        if (bad.length) { setMsgBg(`analysis: ${bad.length} of ${total} tracks failed (${bad[0][0]}: ${bad[0][1]})`, "error"); setMsgDetail(bad[0][2]); }
        else if (weak.length) { setMsgBg(weak.map(w => w[1]).join("; ")); setMsgDetail(weak.map(w => w[1]).join("\n")); }
      });
    } else hideProgress();   // idle: nothing to show
  }).catch(()=>{ pollTimer = setTimeout(pollJob, 1500); });
}

/* P2-7: the detector that ran (job status: tracks[name].detector = "rust-dsp", .chain, .note)
   in the status line; #294 made the DSP detector the only one, so
   it gets no warning. A note gets a visible warning. A track that failed shows its error in red
   (status line and track header), also when every track failed. */
function failedTracks(tracks) {   // [[name, error message], ...]
  return Object.entries(tracks || {}).filter(([, t]) => t.state === "error").map(([n, t]) => [n, t.error || "failed", t.error_detail || ""]);
}
function detectorLabel(id) {   // #411: the status bar names the built-in detector like the Inspector does ("Built-in"); a plug-in keeps its id
  return id === "rust-dsp" ? "Built-in" : id;
}
function showDetector(tracks, bleed) {
  const el = $("#stDet");
  const runs = Object.values(tracks || {}).filter(t => t.state === "done" && t.detector);
  const bad = failedTracks(tracks);
  const parts = [];
  if (runs.length) {
    const names = [...new Set(runs.map(t => detectorLabel(t.detector) + (t.chain ? ` (${t.chain})` : "")))];
    parts.push(`detector: ${names.join(", ")}`);
  }
  const notes = [...new Set(runs.map(t => t.note).filter(Boolean))];
  parts.push(...notes);
  if (bad.length) parts.push(`${bad.length} of ${Object.keys(tracks).length} tracks failed: ${bad[0][1]}`);
  if (bleed && bleed.rule) {   // #411: one hit reads "1 hit"
    const nb = Object.values(bleed.tracks || {}).reduce((a, b) => a + b, 0);
    parts.push(bleed.on ? `tom bleed: ${nb} ${nb === 1 ? "hit" : "hits"} out of the output`
                        : `tom bleed rule off: ${bleed.reason}`);   // P4-14: the analysis says why
  }
  if (!parts.length) return;
  el.textContent = parts.join(" · ");
  el.classList.toggle("warn", notes.length > 0);
  el.classList.toggle("err", bad.length > 0);
  el.title = bad.length ? bad.map(([n, m, d]) => `${n}: ${m}` + (d ? `\n  (${d})` : "")).join("\n") : notes.join("\n");
}
function markTrackErrors(tracks) {   // a red mark and the message in the header of each failed track
  document.querySelectorAll(".track").forEach(row => {
    const t = (tracks || {})[row.dataset.name];
    const old = row.querySelector(".terrmark"); if (old) old.remove();
    row.classList.toggle("terr", !!t && t.state === "error");
    if (t && t.state === "error") {
      const m = document.createElement("div"); m.className = "terrmark";
      m.textContent = `analysis failed: ${t.error || "failed"}`; m.title = m.textContent + (t.error_detail ? `\n(${t.error_detail})` : "");
      row.querySelector(".thead").appendChild(m);
    }
  });
}
/* P2 fix M1: a detector backend that cannot load shows its error at once (B.backendInfo), not only after a run. */
async function showBackend() {
  try {
    const b = await B.backendInfo();
    const el = $("#stDet");
    if (b.note) {
      el.textContent = b.note; el.classList.add("warn"); el.classList.remove("err"); el.title = "";
    }
  } catch (e) { /* the status line stays empty */ }
}
