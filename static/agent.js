/* #371: window.DSAgent, the page-side helpers of an agent that fine-tunes the hits of a song hit by hit with Training Mode
   on (skills/drumspike-finetune, docs/agent-api.md "Page helpers"). Plain functions, no UI, inert unless called. Every move
   and confirm runs through the code the keyboard uses (nudgeSelected, confirmHit), so the training record of an agent is
   byte for byte the record of a person. The helpers that change the document refuse while Training Mode is off. */
(function () {
  const SAMPLE_BEFORE_MS = 0.9, SAMPLE_AFTER_MS = 0.6, WIDE_MS = 10;   // the sample-zoom window around a hit; the 20 ms view is 10 ms each side
  const LINE_LEAD = 12, LINE_MAX_SPREAD = 0.03, DEPART_MIN = 0.03;      // samples of the line, its spread limit and the departure level (shares of the local peak)
  const FRAME_WAIT_MS = 300;                                             // the longest wait for a frame of a hidden page
  const DEPART_BEFORE = 12, DEPART_AFTER = 6;                            // the stroke of the coordinator's routine starts within 12 samples before to 6 after the hit

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
     pre-ripple, bump or dip that leads into the stroke belongs to the stroke. `s` is the window as shares of the local peak. */
  function judge(s, hitAt) {
    const n = Math.min(LINE_LEAD, s.length);
    const lead = s.slice(0, n).sort((a, b) => a - b), level = lead[n >> 1];
    let spread = 0;
    for (let i = 0; i < n; i++) spread = Math.max(spread, Math.abs(s[i] - level));
    const out = { level, spread, first_off: null, suggest: null };
    if (!(spread <= LINE_MAX_SPREAD)) return out;                  // not a line
    const thr = Math.max(3 * spread, DEPART_MIN);
    for (let i = n; i < s.length; i++) {
      if (Math.abs(s[i] - level) > thr) {
        out.first_off = i - hitAt;
        // a departure far from the hit is a pre-bump, bleed or wobble, not the stroke: no suggestion
        if (out.first_off < -DEPART_BEFORE || out.first_off > DEPART_AFTER) out.why = "departure far from the hit";
        else out.suggest = i - 1 - hitAt;
        break;
      }
    }
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
    const shares = Array.from(r.data, v => (peak > 0 ? v / peak : 0));
    return { track, index, sample: hit, start: r.start, hit_at: hitAt, samples: shares, local_peak: peak, amp,
      trainingView: trainingView(track), judge: judge(shares, hitAt) };
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
      ctx.fillStyle = "#8a94a6";   // the wave: the window as shares of the local peak, as the lane draws it
      for (let i = 0; i < v.samples.length; i++) {
        const h = Math.max(1, Math.round(Math.abs(v.samples[i]) * (height / 2 - 8)));
        ctx.fillRect(i * px, mid - h, px, 2 * h);
      }
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
        rows.push({ index: i, id: evs[i].id, t: evs[i].t, sample: l.sample, suggest: j.suggest, spread: j.spread,
          confidence: q ? q.c : null, queue_doubt: qd, queue_why: qd ? [...(q.why || [])] : [],
          line_ok: j.suggest !== null,
          why: j.suggest !== null ? null : j.why || "no clear line before the hit" });
      }
      return rows;
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
