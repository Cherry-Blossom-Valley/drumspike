/* Drum Spike - the states of the page at the first open (DAW plugin M1d4, #578)
   The design is docs/design/daw-plugin/m1d4-first-open.md. The plugin keeps ONE state per DAW track row and one for the document
   (plugins/ara/src/firstopen.rs: the shell command plugin_host_states and the page event plugin-states). This page only DRAWS them:
   - a line over the lanes (the document: no clip, waiting, reading n of m, analysing n of m, n tracks need attention, all ready), a live
     region that the screen reader says once per change;
   - a strip in the header of a track that is not done, or that has no hits; failed and offline rows have the control Retry;
   - a card in the lanes for a DAW track that has no project track yet (reading, analysing, failed before the first round);
   - Retry all on the line when more than one row needs attention.
   Nothing here guesses a state from a timer: the page asks the plugin again every second while something is not done, and draws what it
   gets. Every element is made here, in JS, and only in host mode: with the mode off hostStateInit() and every other entry point return
   at once, so the standalone page has no new element, nothing focusable and no new text (owner rule 2026-11-14). */
"use strict";

const HST = { ready: false, absent: false, st: null, gen: -1, doc: "", line: null, cards: null, poll: null, announced: "", doneAt: 0, error: "", fault: "", pulls: 0, writes: 0, lineWrites: 0 };
const HST_POLL_MS = 1000;
const HST_FADE_MS = 8000;   // the line of a finished document goes quiet after this long (its state stays in the data)

HST.on = () => hostMode() && HST.ready;
HST.rows = () => (HST.st && HST.st.rows) || [];
HST.docText = () => (HST.st && HST.st.doc && HST.st.doc.text) || "";
HST.hasRows = () => HST.rows().length > 0;
HST.attention = () => HST.rows().filter(r => r.retry);

/* A new answer of the plugin (the command or the event). An older one (a lower number of the same document) is dropped. An answer with
   `error` keeps the last list and shows one line. */
HST.take = function (p) {
  if (!hostMode() || !p) return false;
  if (p.error) { HST.error = String(p.error); HST.paint(); return false; }
  if (!p.doc || !Array.isArray(p.rows)) return false;
  if (p.doc_id === HST.doc && typeof p.gen === "number" && p.gen < HST.gen) return false;
  HST.absent = false;
  HST.doc = p.doc_id; HST.gen = typeof p.gen === "number" ? p.gen : HST.gen;
  const before = HST.st && HST.st.doc ? HST.st.doc.state : "";
  HST.st = p; HST.error = "";
  if (p.doc.state === "done" && before !== "done") HST.doneAt = performance.now();
  if (p.doc.state !== "done") HST.doneAt = 0;
  HST.paint(); HST.schedule();
  return true;
};

HST.pull = function () {
  if (!hostMode() || typeof B.hostStates !== "function") return Promise.resolve(false);
  HST.pulls++;
  // a host that does not answer the command (an older plugin, a bridge without it): the page keeps its own texts and shows no line
  return B.hostStates().then(p => HST.take(p), () => { if (!HST.st) { HST.absent = true; HST.paint(); } return false; });
};

/* Asked again every second while a row is not done (the plugin decides; the page only asks). A finished document is not asked again:
   a change comes as the event. */
HST.schedule = function () {
  clearTimeout(HST.poll); HST.poll = null;
  const busy = HST.rows().some(r => r.state !== "done") || HST.rows().length === 0;
  if (HST.ready && busy) HST.poll = setTimeout(() => HST.pull(), HST_POLL_MS);
};

HST.ensureLine = function () {
  if (HST.line && HST.line.isConnected) return HST.line;
  const lanes = document.getElementById("lanesScroll");
  if (!lanes || !lanes.parentNode) return null;
  const el = document.createElement("div");
  el.id = "hstLine"; el.className = "hstLine";
  el.setAttribute("role", "status"); el.setAttribute("aria-live", "polite"); el.setAttribute("aria-atomic", "true");
  const text = document.createElement("span"); text.className = "hstText"; el.appendChild(text);
  lanes.parentNode.insertBefore(el, lanes);
  HST.line = el;
  return el;
};

HST.retry = function (ids) {
  if (!hostMode() || typeof B.hostRetry !== "function" || !ids.length) return Promise.resolve(null);
  return B.hostRetry(ids).then(r => {
    const refused = (r && r.refused) || [];
    if (refused.length) setMsg(refused[0].reason || "The plugin is still working on this track.");
    return HST.pull().then(() => r);
  }, e => { setMsg(`Retry did not reach the plugin: ${errText(e)}`, "error"); return null; });
};

/* The strip or the button text is replaced only when it changed, so a screen reader hears a change once. */
function hstSet(el, text) {
  if (el.textContent !== text) { el.textContent = text; HST.writes++; }
}

HST.paintLine = function () {
  const el = HST.ensureLine();
  if (!el) return;
  const t = el.querySelector(".hstText");
  const d = HST.st && HST.st.doc;
  const text = HST.error ? HST.error : d ? d.text : "Checking the tracks of your DAW…";
  if (HST.fault === "first-announce") { t.textContent = text; HST.writes++; HST.lineWrites++; }   // the fault run: written at every poll
  else if (t.textContent !== text) { hstSet(t, text); HST.lineWrites++; }
  el.dataset.state = HST.error ? "error" : d ? d.state : "unknown";
  const quiet = d && d.state === "done" && HST.doneAt && performance.now() - HST.doneAt > HST_FADE_MS;
  el.classList.toggle("hstQuiet", !!quiet || (HST.absent && !HST.st));
  let all = el.querySelector(".hstRetryAll");
  const need = HST.attention();
  if (need.length > 1 && !HST.error) {
    if (!all) {
      all = document.createElement("button"); all.type = "button"; all.className = "btn mini hstRetryAll"; all.textContent = "Retry all";
      all.onclick = () => HST.retry(HST.attention().map(r => r.id));
      el.appendChild(all);
    }
  } else if (all) all.remove();
};

/* The DOM row of a state row: the first source with a project track that is not merged into another row. */
HST.trackRow = function (r) {
  for (const k of r.srcs || []) {
    const el = document.querySelector(`#lanesScroll .track[data-name="${CSS.escape(k)}"]`);
    if (el && !el.classList.contains("hostMerged")) return el;
  }
  return null;
};

HST.stripText = function (r) { return r.text; };

HST.makeRetry = function (r) {
  const b = document.createElement("button");
  b.type = "button"; b.className = "btn mini hstRetry"; b.textContent = "Retry";
  b.setAttribute("aria-label", `Retry ${r.name}`);
  b.onclick = ev => { ev.stopPropagation(); HST.retry([r.id]); };
  return b;
};

/* One state element (a strip or a card) for a row: the text, and Retry for a failed or offline row. */
HST.fill = function (el, r) {
  el.dataset.state = r.state; el.dataset.row = r.id;
  let t = el.querySelector(".hstRowText");
  if (!t) { t = document.createElement("span"); t.className = "hstRowText"; el.prepend(t); }
  hstSet(t, HST.stripText(r));
  let b = el.querySelector(".hstRetry");
  if (r.retry && !b) el.appendChild(HST.makeRetry(r));
  else if (!r.retry && b) b.remove();
  el.title = r.text;
};

HST.paintRows = function () {
  const scroll = document.getElementById("lanesScroll");
  if (!scroll) return;
  const keep = new Set();
  let cards = scroll.querySelector(":scope > .hstCards");
  for (const r of HST.rows()) {
    const row = HST.trackRow(r);
    if (row) {
      // R5: a strip only for a row that is not done, or that has no hits
      const want = r.state !== "done" || r.hits === 0;
      let s = row.querySelector(".thead .hostState");
      if (want) {
        if (!s) { s = document.createElement("div"); s.className = "hostState"; row.querySelector(".thead").appendChild(s); }
        HST.fill(s, r); keep.add(s);
      }
      continue;
    }
    // no project track yet: a card in the lanes
    if (!cards) { cards = document.createElement("div"); cards.className = "hstCards"; scroll.appendChild(cards); }
    let c = cards.querySelector(`.hostCard[data-row="${CSS.escape(r.id)}"]`);
    if (!c) { c = document.createElement("div"); c.className = "hostCard"; cards.appendChild(c); }
    HST.fill(c, r); keep.add(c);
  }
  document.querySelectorAll("#lanesScroll .hostState, #lanesScroll .hostCard").forEach(e => { if (!keep.has(e)) e.remove(); });
  if (cards && !cards.children.length) cards.remove();
  // the cards come in the order of the DAW
  if (cards) { const order = HST.rows().map(r => r.id); [...cards.children].sort((a, b) => order.indexOf(a.dataset.row) - order.indexOf(b.dataset.row)).forEach(c => cards.appendChild(c)); }
};

HST.paint = function () {
  if (!hostMode() || !HST.ready) return;
  HST.paintLine(); HST.paintRows();
  const e = document.querySelector("#lanesScroll .empty-state.hostEmpty");
  if (e) hostEmptyRefresh();
};

HST.init = function () {
  if (!hostMode() || HST.ready) return;
  HST.ready = true;
  if (B.listen) B.listen("plugin-states", p => HST.take(p)).catch(() => {});
  HST.paintLine();
  HST.pull();
};

function hostStateInit() { HST.init(); }
