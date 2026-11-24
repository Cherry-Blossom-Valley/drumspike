/* Drum Spike - the handles that deliver the files to the DAW (DAW plugin M1c3, #576)
   In a DAW the plugin makes the MIDI and spike files of every DAW track in the background (plugins/ara/src/prepare.rs). This file shows them as
   drag handles: three in the header of a row (MIDI, Spike, Audio spike), four in the toolbar (Drag MIDI, Drag spikes, Drag all, Drag merged MIDI).
   A handle is in one state: ready (count), preparing, error: reason, or "press to prepare" (a spike kind before its first use). Only a ready
   handle drags. The plugin gives the states as the shell command plugin_deliver_state and the page event plugin-deliver.
   Everything here is behind hostMode(): with the mode off nothing runs, no element is made, nothing is focusable (the standalone page is the page of today).
   The design is docs/design/daw-plugin/m1c3-one-drag.md. */
"use strict";

const DLV = {
  st: null,          // the last payload of the plugin: {gen, rows: [{id, name, srcs, midi, spike, audio}], merged, used, hint}
  ready: false,
  poll: null,        // the timer that asks again while a handle is preparing (the page never shows a dead "preparing…")
  menuFor: null,
};
const DLV_KINDS = [["midi", "MIDI", "MIDI"], ["spike", "Spike", "Spike"], ["audio", "Audio spike", "Audio"]];   // key, full label (tooltip, menu, screen reader), small label of the row header (#592)
/* #592: the icons of the three kinds in a row header (inline SVG, no image files): a note, one spike on a line, a waveform. */
const DLV_ICONS = {
  midi: '<path d="M5 11.2V3l6-1.4v8.2"/><ellipse cx="3.6" cy="11.2" rx="1.6" ry="1.2" fill="currentColor"/><ellipse cx="9.6" cy="9.8" rx="1.6" ry="1.2" fill="currentColor"/>',
  spike: '<path d="M1 11.5h5l1-9.5 1 9.5h5"/>',
  audio: '<path d="M1.5 6v2M4 4v6M6.5 2v10M9 4.5v5M11.5 3v8M13.5 6v2"/>',
};
const DLV_HINT = "Drop at the start of the session";
const DLV_TEMPO = "If your DAW asks about the tempo of this MIDI, choose the current tempo map.";

DLV.fault = "";   // the fault run of the check (leak: the handles show without the mode); never set by the page itself
DLV.live = () => hostMode() || DLV.fault === "leak";
DLV.on = () => DLV.live() && DLV.ready;

/* The state of a list of states: error, then preparing, then press to prepare, then ready (the count adds up); none when no row has hits. */
DLV.agg = function (list) {
  const live = list.filter(s => s && s.state !== "none");
  if (!live.length) return { state: "none", n: 0, why: "no hits" };
  const bad = live.find(s => s.state === "error");
  if (bad) return { state: "error", n: 0, why: bad.why };
  if (live.some(s => s.state === "preparing")) return { state: "preparing", n: 0, why: "" };
  if (live.some(s => s.state === "off")) return { state: "off", n: 0, why: "press to prepare" };
  return { state: "ready", n: live.reduce((a, s) => a + s.n, 0), why: "" };
};
DLV.text = function (s) {
  if (s.state === "ready") return `ready ${s.n}`;
  if (s.state === "preparing") return "preparing…";
  if (s.state === "off") return "press to prepare";
  if (s.state === "none") return "no hits";
  const w = `error: ${s.why || "could not be prepared"}`;
  return w.length > 40 ? w.slice(0, 37) + "…" : w;
};
/* The badge of a handle in a row header (#592: the header is 230 px wide, three handles with the whole state text do not fit and lay on the
   role drawing): the count when ready, else one sign. The whole text stays in the tooltip, in the screen reader label and in `.dlvT`.
   A tile has a fixed width, so the badge is 3 signs at most: from 1000 hits it counts in thousands ("1k", "12k"; the tooltip has the number). */
DLV.count = n => (n < 1000 ? String(n) : n < 100000 ? `${Math.floor(n / 1000)}k` : "99k");
DLV.badge = s => (s.state === "ready" ? DLV.count(s.n) : s.state === "preparing" ? "…" : s.state === "off" ? "+" : s.state === "error" ? "!" : "–");
DLV.title = function (label, s, extra) {   // no word "file" or "folder" in what the user reads (the host page test of the visible words)
  const why = s.state === "error" ? `error: ${s.why}. Press to try again` : s.state === "none" ? "no hits" : s.state === "off" ? "press to prepare (a press starts it)" : s.state === "preparing" ? "preparing: made in the background" : `ready (${s.n}). Drag it into your DAW with the mouse. Keyboard: use Export… (Cmd+E)`;
  return `${label}: ${why}. ${DLV_HINT}.${extra ? " " + extra : ""}`;
};

/* The rows of the plugin, for the DOM row `name`: the entry of the row that the DOM row draws (its home source), or null. */
DLV.rowFor = function (name) {
  if (!DLV.st) return null;
  if (typeof HT !== "undefined" && HT.on() && HT.merged.has(name)) return null;
  const e = typeof HT !== "undefined" && HT.on() && HT.homeOf.get(name);
  if (e) return DLV.st.rows.find(r => r.id === e.id) || null;
  return DLV.st.rows.find(r => r.srcs[0] === name) || null;
};

/* A press on an element that is a drag source: a press and a move of 4 px on a READY element starts the drag (the plugin checks the hits and gives the
   items); a press with no move on an element that says "press to prepare" or "error" runs `act` (the first use of a spike kind, or a new try).
   The listeners on the document live from the press to the mouse up (or the loss of the window focus) and nowhere else. */
DLV.arm = function (el, get, items, act) {
  el.addEventListener("mousedown", ev => {
    if (ev.button !== 0) return;
    ev.preventDefault(); ev.stopPropagation();
    const s = get(), x0 = ev.clientX, y0 = ev.clientY;
    const off = () => { document.removeEventListener("mousemove", mv, true); document.removeEventListener("mouseup", up, true); window.removeEventListener("blur", off); };
    const mv = e => { if (Math.hypot(e.clientX - x0, e.clientY - y0) > 4) { off(); if (s.state === "ready") { DLV.closeMenu(); DLV.drag(items()); } } };
    const up = () => { off(); if (s.state === "off" || s.state === "error") { DLV.closeMenu(); act(); } };
    document.addEventListener("mousemove", mv, true); document.addEventListener("mouseup", up, true); window.addEventListener("blur", off);
  });
};
/* Keyboard: Enter or Space on an element runs the press (prepare, try again); a drag is mouse only. */
DLV.keys = function (el, get, act) {
  el.addEventListener("keydown", ev => {
    if (ev.key !== "Enter" && ev.key !== " ") return;
    ev.preventDefault(); ev.stopPropagation();
    const s = get();
    if (s.state === "off" || s.state === "error") act();
    else if (s.state === "ready") setMsg("A drag needs the mouse. Keyboard: use Export… (Cmd+E) in the menu");
  });
};
/* `wants`: [{kind, row?}] that a press starts: the first use of the spike kinds, or a new try of what failed. */
DLV.act = wants => () => { wants.forEach(w => DLV.want(w.kind, w.row)); };

/* One handle element. `get()` gives its state; `items()` the items of a drag; `wants` what a press starts. */
DLV.handle = function (label, kind, get, items, extra, wants, small) {
  const h = document.createElement("div");
  h.className = "dlvH"; h.dataset.kind = kind; h.draggable = false;
  h.setAttribute("role", "button"); h.tabIndex = 0;
  const lb = document.createElement("span"); lb.className = "dlvL"; lb.textContent = small || label;
  const tx = document.createElement("span"); tx.className = "dlvT";
  const bd = document.createElement("span"); bd.className = "dlvB"; bd.setAttribute("aria-hidden", "true");
  h.append(lb, tx, bd);
  if (small && DLV_ICONS[kind]) {   // a row header: the icon and the badge over a small label
    const ic = document.createElement("span"); ic.className = "dlvI"; ic.setAttribute("aria-hidden", "true");
    ic.innerHTML = `<svg viewBox="0 0 15 14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round">${DLV_ICONS[kind]}</svg>`;
    h.prepend(ic); h.classList.add("dlvTile");
  }
  h._paint = () => {
    const s = get();
    h.dataset.state = s.state; tx.textContent = DLV.text(s); bd.textContent = DLV.badge(s); h.title = DLV.title(label, s, extra);
    h.setAttribute("aria-label", `${label}, ${s.state === "error" ? "error, press to retry" : DLV.text(s)}`);
  };
  const act = DLV.act(wants || [{ kind }]);
  DLV.arm(h, get, items, act);
  DLV.keys(h, get, act);
  return h;
};

DLV.want = async function (kind, row) {
  try { DLV.take(await B.deliverWant(kind, row)); } catch (e) { setMsg(`could not start the preparation: ${errText(e)}`, "error"); }
};

/* The drag: the plugin checks that every item is ready for the CURRENT hits; a refused take is not an error of the user (the state says
   why and the page asks again by itself). The files exist already: nothing is made at the press. */
DLV.drag = async function (items) {
  if (!items.length) return;
  let files;
  try { files = (await B.deliverTake(items)).files; }
  catch (e) { setMsg(`drag refused: ${errText(e)}`, "error"); DLV.pull(); return; }
  B.dragFiles(files).then(() => setMsg(`dragged ${files.length} item${files.length === 1 ? "" : "s"}: ${DLV_HINT.toLowerCase()}`))
    .catch(e => setMsg(`drag failed: ${errText(e)}`, "error"));
};

/* A small menu of handles (the collapsed header, the narrow toolbar). An entry is a drag source itself: a press and a move on a ready entry starts the
   drag, a press on "press to prepare" or "error" starts the work. Its listeners on the document are removed when the menu closes. */
DLV.closeMenu = function () {
  const m = document.getElementById("dlvMenu");
  if (DLV.menuOff) { DLV.menuOff(); DLV.menuOff = null; }
  if (m) m.remove();
};
DLV.menu = function (anchor, entries) {
  DLV.closeMenu();
  const m = document.createElement("div"); m.id = "dlvMenu"; m.className = "dlvMenu"; m.setAttribute("role", "menu");
  for (const en of entries) {
    const it = document.createElement("div"); it.className = "dlvItem"; it.setAttribute("role", "menuitem"); it.tabIndex = 0;
    const s = en.get();
    it.dataset.state = s.state; it.dataset.kind = en.kind;
    it.textContent = `${en.label}: ${DLV.text(s)}`;
    it.setAttribute("aria-label", `${en.label}, ${s.state === "error" ? "error, press to retry" : DLV.text(s)}`);
    it.title = DLV.title(en.label, s, "");
    const act = DLV.act(en.wants);
    DLV.arm(it, en.get, en.items, act);
    DLV.keys(it, en.get, () => { DLV.closeMenu(); act(); });
    m.appendChild(it);
  }
  document.body.appendChild(m);
  const b = anchor.getBoundingClientRect();
  m.style.left = Math.max(0, Math.min(b.left, innerWidth - m.offsetWidth)) + "px";
  m.style.top = Math.max(0, Math.min(b.bottom, innerHeight - m.offsetHeight)) + "px";
  const away = ev => { if (!m.contains(ev.target) && ev.target !== anchor) DLV.closeMenu(); };
  const esc = ev => { if (ev.key === "Escape") DLV.closeMenu(); };
  document.addEventListener("mousedown", away, true); document.addEventListener("keydown", esc, true);
  DLV.menuOff = () => { document.removeEventListener("mousedown", away, true); document.removeEventListener("keydown", esc, true); };
  const first = m.querySelector(".dlvItem"); if (first) first.focus();
};

DLV.rowItems = (row, kinds) => kinds.map(k => ({ row: row.id, kind: k }));
DLV.rowsOf = () => (DLV.st ? DLV.st.rows : []);
DLV.collapsed = function (th) {
  const lanes = document.getElementById("lanesScroll");
  return innerWidth < 520 || (lanes && lanes.dataset.height === "small") || (th && th.classList.contains("tart-hidden"));
};

/* The strip of one row header. */
DLV.strip = function (rowEl, name, narrow) {
  const row = DLV.rowFor(name), th = rowEl.querySelector(".thead");
  let old = th.querySelector(".tdel"); if (old) old.remove();
  if (!row) return;
  const strip = document.createElement("div"); strip.className = "tdel";
  const cur = () => DLV.rowFor(name) || row;
  const get = k => () => { const r = DLV.rowFor(name); return r ? r[k] : { state: "none", n: 0, why: "" }; };
  const hs = DLV_KINDS.map(([k, label, small]) => DLV.handle(label, k, get(k), () => DLV.rowItems(cur(), [k]), k === "midi" ? DLV_TEMPO : "", [{ kind: k, row: row.id }], small));
  if (narrow || DLV.collapsed(th)) {
    // one handle (the first kind that is ready, MIDI then Spike) and a small menu with the three
    const pick = () => ["midi", "spike", "audio"].find(k => cur()[k].state === "ready") || "midi";
    const one = DLV.handle("Drag", "one", () => cur()[pick()], () => DLV.rowItems(cur(), [pick()]), "", [{ kind: "midi", row: row.id }]);
    const more = document.createElement("button"); more.className = "btn mini dlvM"; more.textContent = "…"; more.title = "The three kinds of this DAW track";
    more.onclick = ev => {
      const r = cur();
      DLV.menu(more, DLV_KINDS.map(([k, label]) => ({ kind: k, label, get: () => cur()[k], items: () => DLV.rowItems(cur(), [k]), wants: [{ kind: k, row: r.id }] })));
      ev.stopPropagation();
    };
    strip.classList.add("collapsed");
    strip.append(one, more); strip._hs = [one];
  } else { strip.append(...hs); strip._hs = hs; }
  th.appendChild(strip);
  strip._hs.forEach(h => h._paint());
  return strip;
};

/* The toolbar. */
DLV.bar = function () {
  let bar = document.getElementById("dlvBar"); if (bar) bar.remove();
  DLV.closeMenu();
  const anchor = document.getElementById("exportAllBtn");
  if (!anchor) return;
  bar = document.createElement("span"); bar.id = "dlvBar"; bar.className = "dlvBar";
  const rows = () => DLV.rowsOf();
  const aggK = k => () => DLV.agg(rows().map(r => r[k]));
  const all = () => { const l = [aggK("midi")(), aggK("spike")()]; if (DLV.st && DLV.st.used.audio) l.push(aggK("audio")()); return DLV.agg(l); };
  const allKinds = () => ["midi", "spike", ...(DLV.st && DLV.st.used.audio ? ["audio"] : [])];
  const itemsK = ks => () => rows().flatMap(r => ks.filter(k => r[k].state !== "none" && r[k].state !== "off").map(k => ({ row: r.id, kind: k })));
  const merged = () => (DLV.st ? DLV.st.merged : { state: "preparing", n: 0, why: "" });
  const defs = [
    { kind: "midi", label: "Drag MIDI", get: aggK("midi"), items: itemsK(["midi"]), extra: DLV_TEMPO, wants: [{ kind: "midi" }] },
    { kind: "spike", label: "Drag spikes", get: aggK("spike"), items: itemsK(["spike"]), extra: "", wants: [{ kind: "spike" }] },
    { kind: "all", label: "Drag all", get: all, items: () => itemsK(allKinds())(), extra: "A press prepares the spike and the audio spike items.", wants: [{ kind: "spike" }, { kind: "audio" }, { kind: "midi" }] },
    { kind: "merged", label: "Drag merged MIDI", get: merged, items: () => [{ row: "", kind: "merged" }], extra: "One MIDI track with all drums. " + DLV_TEMPO, wants: [{ kind: "merged" }] },
  ];
  const hs = defs.map(d => DLV.handle(d.label, d.kind, d.get, d.items, d.extra, d.wants));
  const hint = document.createElement("span"); hint.id = "dlvHint"; hint.className = "dlvHint"; hint.textContent = DLV_HINT;
  bar._hs = hs; bar._hint = hint;
  if (innerWidth < 520) {
    const more = document.createElement("button"); more.className = "btn mini dlvM"; more.textContent = "Drag…"; more.title = "Drag the prepared items into your DAW";
    more.onclick = ev => { DLV.menu(more, defs.map(d => ({ kind: d.kind, label: d.label, get: d.get, items: d.items, wants: d.wants }))); ev.stopPropagation(); };
    bar.append(more, hint);
  } else bar.append(...hs, hint);
  anchor.after(bar);
};

DLV.hintText = function () {
  const dropped = typeof HT !== "undefined" && HT.on() ? HT.dropped : 0;
  return dropped ? `${DLV_HINT}. ${dropped} DAW track${dropped === 1 ? " has" : "s have"} no handle in the header: use the toolbar handles` : DLV_HINT;
};

DLV.paint = function () {
  if (!DLV.live() || !DLV.ready) return;
  const made = [];
  document.querySelectorAll("#lanesScroll .track").forEach(el => { const s = DLV.strip(el, el.dataset.name); if (s && !s.classList.contains("collapsed")) made.push([el, s]); });
  // #592: the tiles have a fixed width, so only the width of the header decides the fit. A strip that is wider than its place (a narrow header)
  // becomes the one handle with the menu. All strips are made first and measured in one pass (one layout, not one per row).
  made.filter(([, s]) => s.scrollWidth > s.clientWidth + 1).forEach(([el]) => DLV.strip(el, el.dataset.name, true));
  DLV.bar();
  DLV.repaint();
};
DLV.repaint = function () {
  document.querySelectorAll("#lanesScroll .tdel").forEach(s => (s._hs || []).forEach(h => h._paint()));
  const bar = document.getElementById("dlvBar");
  if (bar) { (bar._hs || []).forEach(h => h._paint()); bar._hint.textContent = DLV.hintText(); }
};

DLV.take = function (p) {
  if (!p || typeof p !== "object" || !Array.isArray(p.rows)) return false;
  if (DLV.st && Number.isFinite(p.gen) && p.gen < DLV.st.gen && p.gen !== 0) return false;
  const first = !DLV.st;
  const sig = r => r.map(x => x.id).join("|");
  const shape = !first && sig(DLV.st.rows) !== sig(p.rows);
  DLV.st = p;
  if (first || shape) DLV.paint(); else DLV.repaint();
  DLV.schedule();
  return true;
};
/* A handle that is preparing is asked about again every 1.5 s: the page never shows a dead "preparing…" when an event got lost. */
DLV.schedule = function () {
  clearTimeout(DLV.poll); DLV.poll = null;
  const busy = DLV.st && (DLV.st.rows.some(r => ["midi", "spike", "audio"].some(k => r[k].state === "preparing")) || DLV.st.merged.state === "preparing");
  if (busy) DLV.poll = setTimeout(() => DLV.pull(), 1500);
};
DLV.pull = function () {
  if (!hostMode() || typeof B.deliverState !== "function") return Promise.resolve(false);
  return B.deliverState().then(p => DLV.take(p), () => false);
};
DLV.init = function () {
  if (!hostMode() || DLV.ready) return;
  DLV.ready = true;
  if (B.listen) B.listen("plugin-deliver", p => DLV.take(p)).catch(() => {});
  const lanes = document.getElementById("lanesScroll");
  if (lanes) new MutationObserver(() => { if (!DLV.busyPaint) { DLV.busyPaint = true; queueMicrotask(() => { DLV.busyPaint = false; DLV.paint(); }); } }).observe(lanes, { childList: true });
  window.addEventListener("resize", () => DLV.paint());
  DLV.pull();
};
