/* Drum Spike - the menu of the page in host mode (DAW plugin M1b11, #559)
   A plugin window has no menu bar, so the settings and the view options of the macOS menu were out of reach. In host
   mode the page adds one button to the top bar (#hostMenuBtn) that opens a panel with the commands of the app menu.
   The panel is made from the command table of static/keymap.js (DSKeys.COMMANDS), the table that the macOS menu is
   compiled from (apps/desktop/src-tauri/src/menu/keymap.rs). Two lists remove commands: hostOwned() (HOST_OWNED and
   the menu ids of HOST_ABSENT, static/state.js). A command of the table that is in neither the panel nor one of the
   lists would be a hole: hostMenuAudit() names it and the e2e test fails on it. A new command appears here with no
   second list.
   An item runs the same handler as the macOS menu item (DSShell.menu({id}), static/shell.js), so a menu item, its
   key and its button do the same thing. The check marks and the greyed items follow the same state that the page
   reports to the macOS menu (DSShell.menuState(), the rules of menu/sync.rs).
   With the mode off nothing here runs: hostMenuInit() returns at once, the page has no button and no panel
   (owner rule 2026-11-14: the standalone page is unchanged).
   Keys: the button opens with Enter, Space or Down. In the panel Up and Down move, Home and End jump, Right or
   Enter opens a menu or runs an item, Left goes back, Escape goes back and then closes (the focus returns to the
   button). A panel key never reaches the page's own key table (it would nudge or play). In a low window the panel
   scrolls inside the window. */
"use strict";

/* The check mark of every check item of the table, from the state that DSShell.menuState() gives. The e2e test fails
   for a check row of the table without an entry here, and for an entry without a row. */
const HM_CHECKS = {
  snap: s => s.snap, click: s => s.click, info_line: s => s.infoLine, inspector: s => s.inspector, overview: s => s.overview,
  ruler_tempo: s => s.tempoRuler, ruler_meter: s => s.meterRuler, ruler_beats: s => s.beatsRuler, ruler_markers: s => s.markersRuler,
  grid: s => s.grid, velocity_lane: s => s.velocityLane, other_mics: s => s.otherMics, training_mode: s => s.trainingMode,
  suspend_follow: s => s.suspendFollow, stop_return_mode: s => s.stopReturnMode, audition_tab: s => s.auditionTab,
  learn_filter: s => s.learnFilter,
  counter_seconds: s => s.counter === "seconds", counter_samples: s => s.counter === "samples",
  counter_barsbeats: s => s.counter === "barsbeats", counter_timecode: s => s.counter === "timecode",
  tab_active: s => s.tabMode === "active", tab_all: s => s.tabMode === "all",
  height_small: s => s.trackHeight === "small", height_medium: s => s.trackHeight === "medium", height_large: s => s.trackHeight === "large",
  nudge_val_1smp: s => s.nudgeValue === "1smp", nudge_val_10smp: s => s.nudgeValue === "10smp", nudge_val_100smp: s => s.nudgeValue === "100smp",
  nudge_val_1ms: s => s.nudgeValue === "1ms", nudge_val_10ms: s => s.nudgeValue === "10ms",
  follow_smooth: s => s.follow === "smooth", follow_page: s => s.follow === "page", follow_off: s => s.follow === "off",
};
/* The items that need more than a project (menu/sync.rs sync_checks): the state key that must be true. */
const HM_NEEDS = { undo: "undo", redo: "redo", delete_hit: "selHit", nudge_earlier: "selHit", nudge_later: "selHit",
  audition: "selHit", toggle_hit_active: "selHit", reanalyze_selection: "selRange", follow_own: "selected" };

const HM = { open: false, group: null, level: 0, saved: null };

/* The entries of the panel: [{name, items: [{id, title, keys, check, checked, enabled, sub, arg}]}], one group per menu of the
   app (the first part of the path), the items in the order of the table, a submenu as the heading `sub` on its items. */
function hostMenuModel() {
  const st = DSShell.menuState(), groups = [], byName = new Map();
  const grp = name => { if (!byName.has(name)) { const g = { name, items: [] }; byName.set(name, g); groups.push(g); } return byName.get(name); };
  const have = !!S.sid;
  for (const c of DSKeys.COMMANDS) {
    if (!c.menu || !c.path || hostOwned(c.id)) continue;
    const parts = c.path.split(" > "), sub = c.id === "follow_own" ? "Follow Hits Of" : parts.slice(1).join(" > ") || null;
    let enabled = !c.needs_project || have;
    if (HM_NEEDS[c.id]) enabled = enabled && !!st[HM_NEEDS[c.id]];
    let title = c.title;
    if (c.id === "undo" && st.undoTitle) title = st.undoTitle;
    if (c.id === "redo" && st.redoTitle) title = st.redoTitle;
    const check = c.kind === "check", key = DSKeys.shownKeyOf(c, DSKeys.preset);
    grp(parts[0]).items.push({ id: c.id, title, keys: key ? DSKeys.display(key) : "", check, checked: check && !!(HM_CHECKS[c.id] && HM_CHECKS[c.id](st)), enabled, sub });
    if (c.id === "follow_own") for (const n of st.tracks || []) if (n !== st.selected) grp(parts[0]).items.push({ id: "follow", title: n, keys: "", check: true, checked: n === st.follows, enabled: !!st.selected, sub, arg: { name: n } });
    if (c.id === "zoom_organize") (st.zoomOwn || []).forEach((n, i) => grp(parts[0]).items.push({ id: "zoom_own", title: n, keys: "", check: false, checked: false, enabled: have, sub, arg: { n: i } }));
  }
  return groups;
}

/* Every command of the table that has a menu item must be in the panel or in one of the two lists, and never in both.
   `shown` is the list of ids the panel shows (default: the model). Returns the holes. */
function hostMenuAudit(shown) {
  const ids = shown || hostMenuModel().flatMap(g => g.items.map(i => i.id));
  const inPanel = new Set(ids), out = { missing: [], both: [], stale: [], unhandled: [], noCheck: [] };
  for (const c of DSKeys.COMMANDS) {
    if (!c.menu) continue;   // a key without a menu item (nudge by 10, Deselect): the app menu has none either
    const owned = HOST_OWNED.includes(c.id) || HOST_ABSENT.some(g => g.menu.includes(c.id));
    if (!owned && !inPanel.has(c.id)) out.missing.push(c.id);
    if (owned && inPanel.has(c.id)) out.both.push(c.id);
    if (!owned && !DSShell.handlers[c.menu]) out.unhandled.push(c.id);
    if (!owned && c.kind === "check" && !HM_CHECKS[c.id]) out.noCheck.push(c.id);
  }
  const known = new Set(DSKeys.COMMANDS.map(c => c.id).concat(["recent_clear"]));   // Clear Recent is an item of the native Open Recent menu that the table has no row for
  for (const id of [...HOST_OWNED, ...HOST_ABSENT.flatMap(g => g.menu)]) if (!known.has(id)) out.stale.push(id);   // a name in a list that the table does not have
  return out;
}

const hmEl = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; };
const hmItems = () => [...HM.panel.querySelectorAll('[role^="menuitem"]:not([aria-disabled="true"])')];
function hmFocus(el) { if (el) { for (const x of HM.panel.querySelectorAll('[tabindex="0"]')) x.tabIndex = -1; el.tabIndex = 0; el.focus(); } }

function hmRender(groupName) {   // level 0: the menus; level 1: the items of one menu
  const p = HM.panel, groups = hostMenuModel();
  p.textContent = "";
  HM.group = groupName || null; HM.level = groupName ? 1 : 0;
  p.dataset.level = String(HM.level);
  if (!groupName) {
    for (const g of groups) {
      const b = hmEl("button", "hmItem hmGroup", g.name); b.type = "button"; b.setAttribute("role", "menuitem"); b.setAttribute("aria-haspopup", "menu"); b.dataset.group = g.name; b.tabIndex = -1;
      b.appendChild(hmEl("span", "hmKeys", "›"));
      b.onclick = () => { hmRender(g.name); hmFocus(hmItems()[0]); };
      p.appendChild(b);
    }
    return;
  }
  const g = groups.find(x => x.name === groupName);
  const back = hmEl("button", "hmItem hmBack", `‹ ${groupName}`); back.type = "button"; back.setAttribute("role", "menuitem"); back.tabIndex = -1; back.dataset.back = "1";
  back.onclick = () => { hmRender(null); hmFocus(p.querySelector(`[data-group="${CSS.escape(groupName)}"]`)); };
  p.appendChild(back);
  let sub;
  for (const it of g ? g.items : []) {
    if (it.sub !== sub) { sub = it.sub; if (sub) p.appendChild(hmEl("div", "hmHead", sub)); }
    const b = hmEl("button", "hmItem" + (it.sub ? " hmSub" : ""), ""); b.type = "button"; b.tabIndex = -1;
    b.setAttribute("role", it.check ? "menuitemcheckbox" : "menuitem"); b.dataset.cmd = it.id;
    if (it.arg) b.dataset.arg = JSON.stringify(it.arg);
    if (it.check) b.setAttribute("aria-checked", String(it.checked));
    if (!it.enabled) b.setAttribute("aria-disabled", "true");
    b.appendChild(hmEl("span", "hmMark", it.check && it.checked ? "✓" : ""));
    b.appendChild(hmEl("span", "hmTitle", it.title));
    b.appendChild(hmEl("span", "hmKeys", it.keys));
    b.onclick = () => { if (it.enabled) hmRun(it); };
    p.appendChild(b);
  }
}

function hmRun(it) {   // the same handler as the macOS menu; the panel closes first, with the focus on the button: a dialog (Settings) stores it as its opener, so Escape gives it back
  hostMenuClose(true);
  Promise.resolve(DSShell.menu({ id: it.id, ...(it.arg || {}) })).catch(e => setMsg(`${it.id} failed: ${e && e.message}`, "error"));
}

/* The panel shows the state of the moment it opened; while it is open the marks and the greyed items follow the page (a shortcut key
   or the host changes a setting under it). The state has no change event that the page sends to a panel, so a short timer reads it. */
function hmRefresh() {
  if (!HM.open || HM.level !== 1) return;
  const model = hostMenuModel().find(g => g.name === HM.group);
  if (!model) return;
  for (const it of model.items) {
    const sel = `[data-cmd="${it.id}"]` + (it.arg ? `[data-arg='${JSON.stringify(it.arg)}']` : "");
    const b = HM.panel.querySelector(sel);
    if (!b) continue;
    if (it.check) b.setAttribute("aria-checked", String(it.checked));
    if (it.enabled) b.removeAttribute("aria-disabled"); else b.setAttribute("aria-disabled", "true");
    const mark = b.querySelector(".hmMark"); if (mark) mark.textContent = it.check && it.checked ? "✓" : "";
  }
}
function hostMenuOpen() {
  if (!hostMode() || HM.open) return;
  HM.open = true;
  const r = HM.btn.getBoundingClientRect(), p = HM.panel;
  p.style.left = Math.max(4, Math.round(r.left)) + "px"; p.style.top = Math.round(r.bottom + 4) + "px";
  p.style.maxHeight = Math.max(120, innerHeight - Math.round(r.bottom) - 12) + "px";   // a low window: the panel scrolls inside it
  hmRender(null);
  p.classList.remove("hidden"); HM.btn.setAttribute("aria-expanded", "true");
  hmFocus(hmItems()[0]);
  HM.timer = setInterval(hmRefresh, 250);
}
function hostMenuClose(focusBtn) {
  if (!HM.open) return;
  HM.open = false; clearInterval(HM.timer); HM.panel.classList.add("hidden"); HM.btn.setAttribute("aria-expanded", "false");
  if (focusBtn !== false) HM.btn.focus();
}

function hmKey(ev) {   // the keys of the panel; none of them goes on to the page's key table
  const items = hmItems(), i = items.indexOf(document.activeElement), k = ev.key;
  const stop = () => { ev.preventDefault(); ev.stopPropagation(); };
  if (k === "ArrowDown") { stop(); hmFocus(items[(i + 1) % items.length]); }
  else if (k === "ArrowUp") { stop(); hmFocus(items[(i - 1 + items.length) % items.length]); }
  else if (k === "Home") { stop(); hmFocus(items[0]); }
  else if (k === "End") { stop(); hmFocus(items[items.length - 1]); }
  else if (k === "Enter" || k === " " || (k === "ArrowRight" && document.activeElement && document.activeElement.dataset.group)) { stop(); if (document.activeElement && document.activeElement.click) document.activeElement.click(); }
  else if (k === "ArrowLeft" || k === "Escape") {
    stop();
    if (HM.level === 1) { const g = HM.group; hmRender(null); hmFocus(HM.panel.querySelector(`[data-group="${CSS.escape(g)}"]`)); }
    else if (k === "Escape") hostMenuClose(true);
  } else if (k === "Tab") { hostMenuClose(true); stop(); }
  else ev.stopPropagation();   // a letter or a number: neither a key command nor a typed search
}

function hostMenuInit() {
  if (!hostMode() || HM.btn) return;
  const btn = hmEl("button", "btn icon", "☰"); btn.id = "hostMenuBtn"; btn.type = "button";
  btn.setAttribute("aria-haspopup", "menu"); btn.setAttribute("aria-expanded", "false");
  btn.title = "Menu: the settings, the view options and the commands of the Drum Spike menu that the host does not own";
  const panel = hmEl("div", "hostMenu hidden"); panel.id = "hostMenu"; panel.setAttribute("role", "menu"); panel.setAttribute("aria-label", "Drum Spike menu");
  HM.btn = btn; HM.panel = panel;
  const top = document.getElementById("topbar");
  top.insertBefore(btn, top.firstChild);
  document.body.appendChild(panel);
  btn.onclick = () => (HM.open ? hostMenuClose(true) : hostMenuOpen());
  btn.addEventListener("keydown", ev => {
    if (["Enter", " ", "ArrowDown"].includes(ev.key)) { ev.preventDefault(); ev.stopPropagation(); if (!HM.open) hostMenuOpen(); }
    else if (ev.key === "Escape" && HM.open) { ev.preventDefault(); ev.stopPropagation(); hostMenuClose(true); }
  });
  panel.tabIndex = -1;
  panel.addEventListener("keydown", hmKey);
  panel.addEventListener("mousedown", ev => { if (!ev.target.closest('[role^="menuitem"]')) ev.preventDefault(); });   // a click on a heading or the padding keeps the focus on the item: Escape still closes, keys stay out of the page
  document.addEventListener("mousedown", ev => { if (HM.open && !panel.contains(ev.target) && ev.target !== btn) hostMenuClose(false); }, true);
  window.addEventListener("blur", () => hostMenuClose(false));
  window.addEventListener("resize", () => { if (HM.open) { hostMenuClose(false); } });
}

window.DSHostMenu = { model: hostMenuModel, audit: hostMenuAudit, open: hostMenuOpen, close: hostMenuClose, init: hostMenuInit, isOpen: () => HM.open, refresh: hmRefresh, CHECKS: HM_CHECKS };
