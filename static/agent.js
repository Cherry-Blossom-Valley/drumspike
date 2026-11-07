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
    if (!trainingOn()) throw new Error("Training Mode is off: turn it on first with the menu command training_mode (View > Training Mode)");
  }
  /* a page in the background gets no frame: resolve after the frames or after FRAME_WAIT_MS, whichever comes first */
  const draw = () => new Promise(r => {
    const t = setTimeout(r, FRAME_WAIT_MS);
    requestAnimationFrame(() => requestAnimationFrame(() => { clearTimeout(t); r(); }));
  });
  const sampleOf = (track, e) => Math.round(e.t * S.tracks[track].sr);

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
      return view(track, index, o.before_ms === undefined ? SAMPLE_BEFORE_MS : o.before_ms, o.after_ms === undefined ? SAMPLE_AFTER_MS : o.after_ms, o);
    },
    /* the 20 ms view, for deletes and bleed questions */
    wide(track, index) { return view(track, index, WIDE_MS, WIDE_MS, {}); },
    /* the nudge of the keyboard: samples < 0 is earlier. The view goes to sample zoom first, so the record says precision sample. */
    async move(track, index, samples) {
      needTraining(); needHit(track, index);
      if (!Number.isInteger(samples)) throw new Error("move: samples must be a whole number");
      const from = sampleOf(track, S.events[track][index]);
      if (!trainingView(track) || trainingView(track).px_per_sample < 1) await DSAgent.look(track, index);
      else select(track, index);
      const ok = await nudgeSelected(samples);
      const at = S.sel && S.sel.track === track ? S.sel.index : index;
      return { moved: !!ok, index: at, from, to: sampleOf(track, S.events[track][at]) };
    },
    /* Hits > Confirm Hit: the hit is reviewed and right where it is */
    async confirm(track, index) {
      needTraining(); needHit(track, index);
      if (!trainingView(track) || trainingView(track).px_per_sample < 1) await DSAgent.look(track, index);
      else select(track, index);
      return { confirmed: !!(await confirmHit()), index };
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
    async save() { await saveProjectNow(); return { saved: !S.dirty }; },
    async status() {
      const on = trainingOn(), st = S.sid && on ? await B.trainingStatus(S.sid) : null;
      return { training: on, records: st ? st.records : 0, path: st ? st.path : null };
    },
  };
  window.DSAgent = DSAgent;
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
