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
const DLV_KINDS = [["midi", "MIDI", "MIDI"], ["spike", "Spike", "Spike"], ["audio", "Audio spike", "Audio spike"]];
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
DLV.title = function (label, s, extra) {   // no word "file" or "folder" in what the user reads (the host page test of the visible words)
  const why = s.state === "error" ? `error: ${s.why}` : s.state === "none" ? "no hits" : s.state === "off" ? "press to prepare (a press starts it)" : s.state === "preparing" ? "preparing: made in the background" : `ready (${s.n}). Drag it into your DAW`;
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

/* One handle element. `get()` gives its state; `items()` the files of a drag; `kind` is the first-use kind for a press on "press to prepare". */
DLV.handle = function (label, kind, get, items, extra) {
  const h = document.createElement("div");
  h.className = "dlvH"; h.dataset.kind = kind; h.draggable = false;
  const lb = document.createElement("span"); lb.className = "dlvL"; lb.textContent = label;
  const tx = document.createElement("span"); tx.className = "dlvT";
  h.append(lb, tx);
  h._paint = () => {
    const s = get();
    h.dataset.state = s.state; tx.textContent = DLV.text(s); h.title = DLV.title(label, s, extra);
  };
  h.addEventListener("mousedown", ev => {
    if (ev.button !== 0) return;
    ev.preventDefault(); ev.stopPropagation();
    const s = get(), x0 = ev.clientX, y0 = ev.clientY;
    const off = () => { document.removeEventListener("mousemove", mv, true); document.removeEventListener("mouseup", up, true); };
    const mv = e => { if (Math.hypot(e.clientX - x0, e.clientY - y0) > 4) { off(); if (s.state === "ready") DLV.drag(items()); } };
    const up = () => { off(); if (s.state === "off") DLV.want(kind === "all" ? "spike" : kind); };
    document.addEventListener("mousemove", mv, true); document.addEventListener("mouseup", up, true);
  });
  return h;
};

DLV.want = async function (kind) {
  try { DLV.take(await B.deliverWant(kind)); } catch (e) { setMsg(`could not start the preparation: ${errText(e)}`, "error"); }
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

DLV.rowItems = (row, kinds) => kinds.map(k => ({ row: row.id, kind: k }));
DLV.rowsOf = () => (DLV.st ? DLV.st.rows : []);
DLV.collapsed = function (th) {
  const lanes = document.getElementById("lanesScroll");
  return innerWidth < 520 || (lanes && lanes.dataset.height === "small") || (th && th.classList.contains("tart-hidden"));
};

/* The strip of one row header. */
DLV.strip = function (rowEl, name) {
  const row = DLV.rowFor(name), th = rowEl.querySelector(".thead");
  let old = th.querySelector(".tdel"); if (old) old.remove();
  if (!row) return;
  const strip = document.createElement("div"); strip.className = "tdel";
  const get = k => () => { const r = DLV.rowFor(name); return r ? r[k] : { state: "none", n: 0, why: "" }; };
  const hs = DLV_KINDS.map(([k, label]) => DLV.handle(label, k, get(k), () => DLV.rowItems(DLV.rowFor(name) || row, [k]), k === "midi" ? DLV_TEMPO : ""));
  if (DLV.collapsed(th)) {
    // one handle (the first kind that is ready, MIDI then Spike) and a small menu with the three
    const pick = () => { const r = DLV.rowFor(name) || row; return ["midi", "spike", "audio"].find(k => r[k].state === "ready") || "midi"; };
    const one = DLV.handle("Drag", "one", () => { const r = DLV.rowFor(name) || row; return r[pick()]; }, () => DLV.rowItems(DLV.rowFor(name) || row, [pick()]));
    const more = document.createElement("button"); more.className = "btn mini dlvM"; more.textContent = "…"; more.title = "The three kinds of this DAW track";
    more.onclick = ev => {
      const r = DLV.rowFor(name) || row, b = more.getBoundingClientRect();
      openCtxItems(DLV_KINDS.map(([k, label]) => [k, `${label}: ${DLV.text(r[k])}`, () => { if (r[k].state === "off") DLV.want(k); else if (r[k].state === "ready") DLV.drag(DLV.rowItems(r, [k])); }]), b.left, b.bottom);
      ev.stopPropagation();
    };
    strip.classList.add("collapsed");
    strip.append(one, more); strip._hs = [one];
  } else { strip.append(...hs); strip._hs = hs; }
  th.appendChild(strip);
  strip._hs.forEach(h => h._paint());
};

/* The toolbar. */
DLV.bar = function () {
  let bar = document.getElementById("dlvBar"); if (bar) bar.remove();
  const anchor = document.getElementById("exportAllBtn");
  if (!anchor) return;
  bar = document.createElement("span"); bar.id = "dlvBar"; bar.className = "dlvBar";
  const rows = () => DLV.rowsOf();
  const aggK = k => () => DLV.agg(rows().map(r => r[k]));
  const all = () => { const l = [aggK("midi")(), aggK("spike")()]; if (DLV.st && DLV.st.used.audio) l.push(aggK("audio")()); return DLV.agg(l); };
  const itemsK = ks => () => rows().flatMap(r => ks.filter(k => r[k].state !== "none" && r[k].state !== "off").map(k => ({ row: r.id, kind: k })));
  const merged = () => (DLV.st ? DLV.st.merged : { state: "preparing", n: 0, why: "" });
  const hs = [
    DLV.handle("Drag MIDI", "midi", aggK("midi"), itemsK(["midi"]), DLV_TEMPO),
    DLV.handle("Drag spikes", "spike", aggK("spike"), itemsK(["spike"])),
    DLV.handle("Drag all", "all", all, itemsK(["midi", "spike", ...(DLV.st && DLV.st.used.audio ? ["audio"] : [])])),
    DLV.handle("Drag merged MIDI", "merged", merged, () => [{ row: "", kind: "merged" }], "One MIDI track with all drums. " + DLV_TEMPO),
  ];
  const hint = document.createElement("span"); hint.id = "dlvHint"; hint.className = "dlvHint"; hint.textContent = DLV_HINT;
  bar._hs = hs; bar._hint = hint;
  if (innerWidth < 520) {
    const more = document.createElement("button"); more.className = "btn mini dlvM"; more.textContent = "Drag…"; more.title = "Drag the prepared items into your DAW";
    more.onclick = ev => {
      const b = more.getBoundingClientRect();
      openCtxItems(hs.map(h => [h.dataset.kind, `${h.querySelector(".dlvL").textContent}: ${h.querySelector(".dlvT").textContent}`, () => h.dispatchEvent(new MouseEvent("mousedown", { button: 0, clientX: 0, clientY: 0, bubbles: true }))]), b.left, b.bottom);
      ev.stopPropagation();
    };
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
  document.querySelectorAll("#lanesScroll .track").forEach(el => DLV.strip(el, el.dataset.name));
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
