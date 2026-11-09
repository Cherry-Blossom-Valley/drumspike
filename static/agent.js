/* #371: window.DSAgent, the page-side helpers of an agent that fine-tunes the hits of a song hit by hit with Training Mode
   on (skills/drumspike-finetune, docs/agent-api.md "Page helpers"). Plain functions, no UI, inert unless called. Every move
   and confirm runs through the code the keyboard uses (nudgeSelected, confirmHit), so the training record of an agent is
   byte for byte the record of a person. The helpers that change the document refuse while Training Mode is off. */
(function () {
  const SAMPLE_BEFORE_MS = 0.9, SAMPLE_AFTER_MS = 0.6, WIDE_MS = 10;   // the sample-zoom window around a hit; the 20 ms view is 10 ms each side
  const LINE_LEAD = 12, LINE_MIN = 4, DEPART_MIN = 0.03;                 // samples of the line before the foot, the fewest that make a line, the band without a readable hit's peak (share of the local peak)
  const FOOT_STEP_SHARE = 0.25, QUIET_MIN = 3, QUIET_CAP = 1000;                          // #408: the foot walk stops under this share of the steepest step; a line needs steepest / largest quiet step of at least this
  const DEPARTURE_VIS_PCT = 0.025;                                       // #394 guard: the detector's own band, a share of the hit's peak (edge.rs DEPARTURE_VIS_PCT)
  const FRAME_WAIT_MS = 300;                                             // the longest wait for a frame of a hidden page
  const DEPART_BEFORE = 12, DEPART_AFTER = 6;                            // the stroke of the coordinator's routine starts within 12 samples before to 6 after the hit
  const EDGE_SPAN_MS = 5;                                                // #394: the detector's own peak span after the hit (edge.rs EDGE_SPAN_S)

  function need(track) {
    if (!S.sid) throw new Error("no project is open");
    if (!S.tracks[track]) throw new Error(`no track "${track}"`);
  }
  function needHit(track, index) {
    need(track);
    const e = (S.events[track] || [])[index];
    if (!e) throw new Error(`track "${track}" has no hit ${index}`);
    return e;
  }
  function needTraining() {
    if (!trainingOn()) throw new Error("Training Mode is off: turn it on first with setTraining(true) in the page (in the window: View > Training Mode)");
  }
  /* a page in the background gets no frame: resolve after the frames or after FRAME_WAIT_MS, whichever comes first */
  const draw = () => new Promise(r => {
    const t = setTimeout(r, FRAME_WAIT_MS);
    requestAnimationFrame(() => requestAnimationFrame(() => { clearTimeout(t); r(); }));
  });
  const sampleOf = (track, e) => Math.round(e.t * S.tracks[track].sr);

  /* #409: the name of a pre-save backup in the page, the way `project.backup_name` (crates/core/src/project/save.rs)
     writes it: `<name> YYYY-MM-DD HH-MM-SS.drumspike` in local time, `/`, `\`, `:` and NUL as `-`. `secs` is Unix
     seconds, as the document's `modified` field carries it. */
  function backupName(name, secs) {
    const d = new Date(secs * 1000), p = n => String(n).padStart(2, "0");
    const safe = Array.from(name, c => (c === "/" || c === "\\" || c === ":" || c === "\0") ? "-" : c).join("");
    return `${safe} ${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}-${p(d.getMinutes())}-${p(d.getSeconds())}.drumspike`;
  }
  /* the name of the backup that holds the state before this session; set by this session's first save */
  let _beforeBackup = null;

  function select(track, index) {
    if (S.selTrack !== track) setSelTrack(track);
    S.sel = { track, index };
    S.playhead = snapSample(S.events[track][index].t, track);
    updateStatus();
  }

  /* the owner's convention (#351): the hit is the last line sample before the stroke's first motion of either sign; a
     pre-ripple, bump or dip that leads into the stroke belongs to the stroke. `s` is the window as shares of the local
     peak; `ref` (#394) is the local peak over the hit's peak (edge.rs EDGE_SPAN_S): it turns the detector's band
     (DEPARTURE_VIS_PCT of the hit's peak) into a share of the local peak. Without a readable hit's peak the band is
     DEPART_MIN.
     #408: the judge measures the line directly before the stroke, not the far left of the view. It finds the steepest step
     near the hit, walks back to its foot (stopping where the step falls under FOOT_STEP_SHARE of the steepest, so a slow
     wave is line) and over the samples above the band that lead into it, and takes the line from the LINE_LEAD quietest
     samples ending nearest the foot. The answer is a shape verdict: `foot_off` (the foot against the hit), `rise` (the
     hit sample's distance from the line), `quiet_ratio` (steepest step / largest step of the line) and `verdict`:
     `late` (the hit stands up the rise, its own sample over the band), `at_foot`, or `no_quiet_line` (no quiet stretch
     exists: nothing to measure from, `suggest` null). A hit whose own sample is within the band stands (suggest 0), and so
     does a hit in a view whose level stays within the band near the hit. The band is a level: a slow stroke passes it
     while every single step stays under it, and a hit in a dip after the stroke passed the band is still late. */
  function judge(s, hitAt, ref) {
    const out = { level: null, spread: null, first_off: null, suggest: null, foot_off: null, rise: null, quiet_ratio: null, verdict: "no_quiet_line" };
    const band = typeof ref === "number" && Number.isFinite(ref) && ref > 0 ? DEPARTURE_VIS_PCT / ref : DEPART_MIN;   // the hit's peak as a share of the local peak
    // the steepest step near the hit: the stroke's edge (a step j is s[j] to s[j+1])
    let p = -1, steep = 0;
    for (let j = Math.max(0, hitAt - DEPART_BEFORE); j <= Math.min(s.length - 2, hitAt + DEPART_AFTER); j++) {
      const d = Math.abs(s[j + 1] - s[j]);
      if (d > steep) { steep = d; p = j; }
    }
    if (p < 0 || !(steep > 0)) return out;
    // the view holds no motion the detector would read: nothing to move to, the hit stands. The band is a level, so the
    // test is the level's range near the hit: a slow stroke passes the band while every single step stays under it.
    const near = s.slice(Math.max(0, hitAt - DEPART_BEFORE), Math.min(s.length, hitAt + DEPART_AFTER + 1));
    if (Math.max(...near) - Math.min(...near) <= band) { out.suggest = 0; out.verdict = "at_foot"; out.why = "no motion above the band near the hit"; return out; }
    // the foot: walk back from the edge while the steps are still a share of the steepest one; a slow wave under that share is line
    let f = p;
    while (f > 0 && Math.abs(s[f] - s[f - 1]) >= FOOT_STEP_SHARE * steep) f--;
    // The line is the LINE_LEAD samples that end nearest the foot with the smallest largest step: directly before the foot
    // when that stretch is quiet, else the quiet stretch further back. The samples above the band between it and the foot are
    // the stroke's first motion, so the foot moves back over them (owner rule A: a dip or ripple under the band is line).
    let end = -1, quiet = Infinity;
    for (let e = f; e >= Math.max(LINE_MIN, Math.min(LINE_LEAD, f)); e--) {
      let q = 0;
      for (let i = Math.max(1, e - LINE_LEAD + 1); i < e; i++) q = Math.max(q, Math.abs(s[i] - s[i - 1]));
      if (q < quiet) { quiet = q; end = e; }
    }
    if (end < 0) return out;
    const from = Math.max(0, end - LINE_LEAD), lead = s.slice(from, end).sort((a, b) => a - b), level = lead[lead.length >> 1];
    while (f >= end && Math.abs(s[f] - level) > band) f--;           // the line ends at end - 1
    let spread = 0;
    for (let i = from; i < end; i++) spread = Math.max(spread, Math.abs(s[i] - level));
    out.level = level; out.spread = spread;
    out.quiet_ratio = Math.min(QUIET_CAP, quiet > 0 ? steep / quiet : QUIET_CAP);   // finite, so the answer stays JSON
    out.foot_off = f - hitAt;
    out.rise = Math.abs(s[Math.min(Math.max(hitAt, 0), s.length - 1)] - level);
    if (out.quiet_ratio < QUIET_MIN) return out;                   // a ring or an uneven stretch: no line to measure from
    out.first_off = f + 1 - hitAt;
    // owner rule A: the hit stands when its own sample (the guard includes it) is within the detector's band of the line;
    // else a hit up the rise is late. The verdict does not depend on the suggestion's range, so a late hit far from its foot
    // is still listed by `audit`.
    let reach = out.rise;                                          // a stroke that passed the band before the hit: a hit in a later dip is still late
    for (let i = Math.max(0, f + 1); i < Math.min(hitAt, s.length); i++) reach = Math.max(reach, Math.abs(s[i] - level));
    const standing = reach <= band;
    out.verdict = out.foot_off < 0 && !standing ? "late" : "at_foot";
    // a foot far from the hit is a pre-bump, bleed or wobble, not the stroke: no suggestion
    if (out.foot_off < -DEPART_BEFORE || out.foot_off > DEPART_AFTER) { out.why = "departure far from the hit"; return out; }
    out.suggest = standing ? 0 : out.foot_off;                     // a hit within the band stands, also before the foot: measured on the owner rows, a later move there costs 55 exact
    if (standing && out.foot_off !== 0) out.why = "under the detector's band";
    return out;
  }

  async function view(track, index, beforeMs, afterMs, opts) {
    const e = needHit(track, index), sr = S.tracks[track].sr;
    select(track, index);
    S.followHold = true;
    setView(e.t - beforeMs / 1000, e.t + afterMs / 1000);
    const a = S.view.t0 * sr, b = S.view.t1 * sr;
    const r = await B.samples(S.sid, track, Math.max(0, Math.floor(a)), Math.ceil(b));
    let peak = 0;
    for (let i = 0; i < r.data.length; i++) peak = Math.max(peak, Math.abs(r.data[i]));
    const base = S.peakAmp[track] || 1;                             // the amplitude zoom to the local peak: the largest step that does not clip
    let amp = (opts && opts.amp) || AMP_STEPS.filter(v => !(peak > 0) || peak / base * v <= 1).pop() || 1;
    if (!AMP_STEPS.includes(amp)) amp = 1;
    setAmpZoom(amp, false);
    await draw();
    const hit = sampleOf(track, e), hitAt = hit - r.start;
    // #394: the judge gets the hit's peak as its reference: the largest sample from the hit over the detector's own
    // span (edge.rs EDGE_SPAN_S, 5 ms after the hit). `samples` stay shares of the local peak; the judge gets the two
    // peaks' ratio for the guard and `look` answers `stroke_peak` in raw samples.
    const spanEnd = Math.ceil(hit + EDGE_SPAN_MS * sr / 1000);
    let strokePeak = null;
    if (spanEnd <= r.start + r.data.length) {
      strokePeak = Math.max(0, ...r.data.slice(Math.max(0, hitAt)).map(v => Math.abs(v)));
    } else {
      const rs = await B.samples(S.sid, track, r.start, spanEnd);
      if (rs && rs.data && rs.data.length) strokePeak = Math.max(0, ...rs.data.slice(Math.max(0, hit - rs.start)).map(v => Math.abs(v)));
    }
    const ref = peak > 0 && strokePeak > 0 ? peak / strokePeak : null;
    const shares = Array.from(r.data, v => (peak > 0 ? v / peak : 0));
    // #408: the band the judge reads against, stated in the unit of the detector: 2.5 % of the hit's peak over 5 ms
    const band = { pct: DEPARTURE_VIS_PCT, of: "the hit's peak", span_ms: EDGE_SPAN_MS, share_of_local_peak: ref ? DEPARTURE_VIS_PCT / ref : DEPART_MIN };
    return { track, index, sample: hit, start: r.start, hit_at: hitAt, samples: shares, local_peak: peak, stroke_peak: strokePeak, amp, band,
      trainingView: trainingView(track), judge: judge(shares, hitAt, ref) };
  }

  async function ratings(track) {
    const r = await B.confidence(S.sid, track, S.events[track] || [], true);
    return r.confidence || [];
  }

  /* #389: the edits of the helpers say by: agent; the keyboard path stays person. A depth counter, not a flag:
     two overlapping helper calls must both log as agent, so the last one to end resets the author only when the
     depth is back to 0. */
  let _agentDepth = 0;
  async function asAgent(fn) {
    const first = _agentDepth === 0;
    _agentDepth++;
    if (first) setEditBy("agent");
    try { return await fn(); } finally { _agentDepth--; if (_agentDepth === 0) setEditBy("person"); }
  }

  const DSAgent = {
    /* the hits of a track; with Training Mode on, the confidence and the doubt reasons of the review queue */
    async hits(track) {
      need(track);
      const evs = S.events[track] || [], rate = trainingOn() ? await ratings(track) : [];
      return evs.map((e, index) => {
        const h = { index, t: e.t, sample: sampleOf(track, e), v: e.v, added: !!e.added, edited: !!e.edited, flam: !!e.flam, bleed: !!e.bleed };
        const q = rate[index];
        if (q) { h.confidence = q.c; h.doubt = !!q.doubt; h.why = q.why || []; }
        return h;
      });
    },
    /* the sample-zoom view on a hit (defaults 0.9 ms before, 0.6 ms after, amplitude to the local peak) */
    look(track, index, opts) {
      const o = opts || {};
      /* #388: the user watches the window, so each helper names its step in the status line (the skill asks for it) */
      if (S.events[track] && S.events[track][index]) setMsg(`agent: looking at ${track} hit ${index + 1}/${S.events[track].length}`);
      return view(track, index, o.before_ms === undefined ? SAMPLE_BEFORE_MS : o.before_ms, o.after_ms === undefined ? SAMPLE_AFTER_MS : o.after_ms, o);
    },
    /* the 20 ms view, for deletes and bleed questions */
    wide(track, index) { return view(track, index, WIDE_MS, WIDE_MS, {}); },
    /* the nudge of the keyboard: samples < 0 is earlier. The view goes to sample zoom first, so the record says precision sample. */
    async move(track, index, samples) {
      needTraining(); needHit(track, index);
      if (!Number.isInteger(samples)) throw new Error("move: samples must be a whole number");
      return asAgent(async () => {
        const from = sampleOf(track, S.events[track][index]);
        if (!trainingView(track) || trainingView(track).px_per_sample < 1) await DSAgent.look(track, index);
        else select(track, index);
        const ok = await nudgeSelected(samples);
        const at = S.sel && S.sel.track === track ? S.sel.index : index;
        if (ok) setMsg(`agent: moved ${track} hit ${at + 1}/${S.events[track].length} by ${samples} samples`);
        return { moved: !!ok, index: at, from, to: sampleOf(track, S.events[track][at]) };
      });
    },
    /* Hits > Confirm Hit: the hit is reviewed and right where it is */
    async confirm(track, index) {
      needTraining(); needHit(track, index);
      return asAgent(async () => {
        if (!trainingView(track) || trainingView(track).px_per_sample < 1) await DSAgent.look(track, index);
        else select(track, index);
        return { confirmed: !!(await confirmHit()), index };
      });
    },
    /* #409: a picture of the view as the user sees it: the sample-zoom window of the hit, drawn from the same
       samples and amplitude zoom the page draws, with the hit marked. Answers {track, index, sample, width,
       height, format, image}; `image` is a PNG data URL. */
    async render(track, index, opts) {
      const o = opts || {};
      if (S.events[track] && S.events[track][index]) setMsg(`agent: rendering ${track} hit ${index + 1}/${S.events[track].length}`);
      const v = await view(track, index, o.before_ms === undefined ? SAMPLE_BEFORE_MS : o.before_ms,
        o.after_ms === undefined ? SAMPLE_AFTER_MS : o.after_ms, o);
      const px = Math.max(1, Math.min(32, Math.round((v.trainingView && v.trainingView.px_per_sample) || 1)));
      const width = Math.min(4096, v.samples.length * px), height = 320;
      const c = document.createElement("canvas"); c.width = width; c.height = height;
      const ctx = c.getContext("2d"), mid = height / 2;
      ctx.fillStyle = "#14171c"; ctx.fillRect(0, 0, width, height);
      ctx.strokeStyle = "#2a2f38"; ctx.beginPath(); ctx.moveTo(0, mid + 0.5); ctx.lineTo(width, mid + 0.5); ctx.stroke();
      ctx.fillStyle = "#8a94a6";   // the wave: the signed window as shares of the local peak, as the lane draws it: a line at sample zoom
      ctx.strokeStyle = "#8a94a6"; ctx.beginPath();
      for (let i = 0; i < v.samples.length; i++) {
        const y = mid - Math.round(v.samples[i] * (height / 2 - 8));
        if (i === 0) ctx.moveTo(i * px, y); else ctx.lineTo(i * px, y);
      }
      ctx.stroke();
      ctx.fillStyle = "#e5484d";   // the hit: the line the page draws at the hit's own sample
      ctx.fillRect(v.hit_at * px, 0, Math.max(1, px), height);
      return { track, index, sample: v.sample, width, height, format: "image/png", image: c.toDataURL("image/png") };
    },
    async undo() {
      needTraining();
      return { undone: !!(await undoRedo(true)) };
    },
    /* look over a range of hit indices (from to, inclusive) without moving: one row per hit */
    async plan(track, from, to) {
      need(track);
      const evs = S.events[track] || [], rate = trainingOn() ? await ratings(track) : [], rows = [];
      for (let i = Math.max(0, from); i <= Math.min(to, evs.length - 1); i++) {
        const l = await DSAgent.look(track, i), q = rate[i], j = l.judge, qd = !!(q && q.doubt);
        // queue_doubt: the review queue's doubt (a reason to look); line_ok: the judge found a line and a suggestible departure
        rows.push({ index: i, id: evs[i].id, t: evs[i].t, sample: l.sample, suggest: j.suggest, spread: j.spread, verdict: j.verdict,
          confidence: q ? q.c : null, queue_doubt: qd, queue_why: qd ? [...(q.why || [])] : [],
          line_ok: j.suggest !== null,
          why: j.suggest !== null ? null : j.why || "no clear line before the hit" });
      }
      return rows;
    },
    /* #408: every hit of the track whose shape verdict is `late`: the hit stands up the rise, over the band. It reads the shape
       measures of `look` and never `judge.suggest` or a plan row, so a wrong suggestion cannot hide a late hit. */
    async audit(track) {
      need(track);
      const evs = S.events[track] || [], late = [];
      for (let i = 0; i < evs.length; i++) {
        const j = (await DSAgent.look(track, i)).judge;
        if (j.verdict === "late") late.push({ index: i, id: evs[i].id, t: evs[i].t, foot_off: j.foot_off, rise: j.rise, quiet_ratio: j.quiet_ratio });
      }
      return late;
    },
    /* the moves of a plan: [{index, samples}] (or a row of plan: its suggest); a zero or null move is skipped */
    async apply(track, list) {
      needTraining(); need(track);
      const ids = list.map(m => (S.events[track][m.index] || {}).id), out = [];
      for (let k = 0; k < list.length; k++) {
        const m = list[k], n = m.samples !== undefined ? m.samples : m.suggest;
        const index = S.events[track].findIndex(e => e.id === ids[k]);
        if (!Number.isInteger(n) || n === 0 || index < 0) { out.push({ index: m.index, moved: false }); continue; }
        out.push(await DSAgent.move(track, index, n));
      }
      return out;
    },
    /* #409: the save writes a pre-save backup whose NAME is the time of the save but whose file holds the state
       BEFORE that save. This session's first save therefore writes the backup that holds the state before the
       session: its name is the save's own stamp, which the document's `modified` carries (stamped in the same
       save call, just before the backup is written). */
    async save() {
      const first = _beforeBackup === null;
      await saveProjectNow();
      if (first) {
        try {
          const s = await B.projectGet(S.sid);
          if (s && s.modified) _beforeBackup = backupName(s.name, s.modified);
        } catch (e) { /* backups() answers before: null and says so honestly */ }
      }
      return { saved: !S.dirty };
    },
    /* #409: the backups of the project, named the way the save names them: a backup named <time> holds the state
       as it was just BEFORE the save at <time> (the time of the save that replaced it). `before` names the backup
       that holds the state before this session - the one this session's first save wrote; null until then. */
    async backups() {
      if (!S.sid) throw new Error("no project is open");
      return { rule: "a backup named <time> holds the state as it was just before the save at <time>",
        before: _beforeBackup };
    },
    /* #388: the project the window has open (its summary and track names), so an agent that calls the window knows what it edits */
    async project() {
      if (!S.sid) throw new Error("no project is open");
      return { ...(await B.projectGet(S.sid)), tracks: Object.keys(S.tracks) };
    },
    /* #388: what the window shows in its status line and its selection field, and the Training Mode switch of the window */
    statusLine() { return { message: $("#stMsg").textContent, selection: $("#stSel").textContent, training: $("#stTraining").classList.contains("hidden") ? null : $("#stTraining").textContent }; },
    setTraining(on) { setTraining(!!on); return { training: trainingOn() }; },
    async status() {
      const on = trainingOn(), st = S.sid && on ? await B.trainingStatus(S.sid) : null;
      return { training: on, records: st ? st.records : 0, path: st ? st.path : null };
    },
  };
  window.DSAgent = DSAgent;

  /* #388: the window endpoint (crates/app/src/window_relay.rs). The page long-polls window_call_next, runs the named helper of
     DSAgent and answers with window_call_result. Only DSAgent's own functions run (never a name the object lacks), so the
     endpoint reaches nothing else in the page. The loop ends when the service says the endpoint is closed, and starts again
     after a failed poll. */
  async function relayLoop() {
    const poller = Math.random().toString(36).slice(2);   // a poll of this page ends the hanging polls of an earlier page
    for (;;) {
      let r;
      try { r = await B.windowCallNext(25000, poller); } catch (e) { await new Promise(t => setTimeout(t, 2000)); continue; }
      if (!r.enabled) return;
      const c = r.call;
      if (!c) continue;
      try {
        const f = Object.prototype.hasOwnProperty.call(DSAgent, c.fn) ? DSAgent[c.fn] : null;
        if (typeof f !== "function") throw new Error(`no helper ${c.fn}`);
        const v = await f.apply(DSAgent, c.args || []);
        await B.windowCallResult(c.id, true, v);
      } catch (e) {
        await B.windowCallResult(c.id, false, undefined, String(e && e.message !== undefined ? e.message : e)).catch(() => {});
      }
    }
  }
  window.DSAgentRelay = { start: relayLoop };
  if (typeof B !== "undefined" && B.windowCallNext) relayLoop();
})();

/* #371: Settings > Agent fine-tune skill. The status text (not installed / installed <version> / older copy), the Install
   button (Update when the copy is older) and the one `claude mcp add` line with a Copy button. The app installs the skill
   through agent_skill_install (crates/app/src/agent_skill.rs); the line carries the path of this app's binary. */
function agentSkillShow(st, note) {
  const text = !st.installed ? "not installed"
    : st.current ? `installed ${st.version}` : `older copy installed (${st.version || "unknown version"}), this app has ${st.bundled_version}`;
  $("#setSkillStatus").textContent = note ? `${text}. ${note}` : text;
  $("#setSkillInstall").textContent = st.installed && !st.current ? "Update" : "Install";
  $("#setSkillCmd").textContent = st.mcp_command;
}
async function agentSkillFill() {
  const el = $("#setSkillStatus");
  if (!el || !B.agentSkillStatus) return;
  try { agentSkillShow(await B.agentSkillStatus()); }
  catch (e) { el.textContent = `agent skill: ${errText(e)}`; }
}
async function agentSkillInstall() {
  try {
    const r = await B.agentSkillInstall(), st = await B.agentSkillStatus();
    agentSkillShow(st, `${r.files} files in ${r.installed}`);
  } catch (e) { $("#setSkillStatus").textContent = `install failed: ${errText(e)}`; }
}
async function agentSkillCopy() {
  const text = $("#setSkillCmd").textContent;
  try { await navigator.clipboard.writeText(text); setMsg("copied the claude mcp add line"); }
  catch (e) { setMsg(`copy failed: ${errText(e)}`, "error"); }
}
(function () {
  const install = $("#setSkillInstall"), copy = $("#setSkillCopy");
  if (install) install.onclick = agentSkillInstall;
  if (copy) copy.onclick = agentSkillCopy;
})();
