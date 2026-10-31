/* #255: the in-app manual. The chapters come from static/help/ (prebuilt from docs/guide by
   tools/build_help.py — the one source of the text; a test fails when a guide chapter is missing
   from the help). The Shortcuts section is rendered from static/keymap.js (DSKeys.renderHelp),
   the one source of the keys, for the active preset. Nothing here reaches the network: every
   request stays on the app origin. Links out (class ext) go to the default browser
   (B.openUrl), never inside the app window. */
(function () {
  "use strict";
  const $ = s => document.querySelector(s);
  let manifest = null;            // {chapters: [{file, title, source}]}
  let history = [], at = -1;      // the visited sections and the position in them (back/forward)
  let current = null;             // the section id on screen

  function page() { return $("#helpPage"); }

  function navEntry(id, label) {
    const b = document.createElement("button");
    b.className = "helpChapter";
    b.textContent = label;
    b.dataset.section = id;
    b.onclick = () => openSection(id);
    return b;
  }

  async function ensureManifest() {
    if (manifest) return manifest;
    manifest = await (await fetch("help/index.json")).json();
    const nav = $("#helpChapters");
    nav.innerHTML = "";
    for (const c of manifest.chapters) nav.appendChild(navEntry(c.file, c.title));
    nav.appendChild(navEntry("shortcuts", "Shortcuts"));
    return manifest;
  }

  function markNav(id) {
    for (const b of $("#helpChapters").children) b.classList.toggle("active", b.dataset.section === id);
  }

  function shortcutsSection() {
    // the cheat sheet: one row per keymap row of the active preset, grouped by menu — the
    // same table (ids keymapPreset, keymapTable) that Help > Keyboard Shortcuts always had
    page().innerHTML =
      `<div id="helpShortcuts">
        <h2>Shortcuts</h2>
        <p class="hint">Every command of the app, by menu. The keys without Cmd, Control or Option work when no text
        field has the focus; the menu does not show them (it would take them from the text fields).</p>
        <p class="hint" id="keymapPreset"></p>
        <table class="keymap" id="keymapTable"></table>
      </div>`;
    $("#keymapPreset").textContent = `Key preset: ${DSKeys.presetName()} (change it in Drum Spike > Settings).`;
    DSKeys.renderHelp($("#keymapTable"));
  }

  async function show(id) {
    if (id === "shortcuts") { shortcutsSection(); }
    else {
      const html = await (await fetch(`help/${id}`)).text();
      const div = document.createElement("div");
      div.className = "helpChapterPage";
      div.innerHTML = html;   // prebuilt by tools/build_help.py from docs/guide, escaped there
      page().replaceChildren(div);
    }
    current = id;
    markNav(id);
    page().scrollTop = 0;
  }

  async function openSection(id, push = true) {
    await ensureManifest();
    if (!id) id = manifest.chapters[0].file;   // first chapter (the entry page)
    if (push) {
      history = history.slice(0, at + 1);
      history.push(id);
      at = history.length - 1;
    }
    $("#helpBack").disabled = at <= 0;
    $("#helpFwd").disabled = at >= history.length - 1;
    await show(id);
  }

  async function open(id) {
    showModal("#helpModal");
    await openSection(id || (current && (current === "shortcuts" || manifest?.chapters?.some(c => c.file === current)) ? current : null));
  }

  function hooks() {
    $("#helpDone").onclick = () => $("#helpModal").classList.add("hidden");
    $("#helpBack").onclick = () => { if (at > 0) { at--; openSection(history[at], false); } };
    $("#helpFwd").onclick = () => { if (at < history.length - 1) { at++; openSection(history[at], false); } };
    page().addEventListener("click", ev => {
      const a = ev.target.closest("a");
      if (!a) return;
      ev.preventDefault();
      if (a.classList.contains("ext")) B.openUrl(a.href);        // out: the default browser
      else if (a.classList.contains("chapter")) openSection(a.getAttribute("href"));
      else if (a.getAttribute("href")?.startsWith("#")) openSection(a.getAttribute("href").slice(1));
    });
  }

  window.DSHelp = { open, hooks };
})();
