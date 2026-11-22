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

  /* #539: in a DAW (host mode) the manual is one short section on how the page works there, then the chapters that name
     no command that the host owns (the host owns the files, the tracks, the tempo and the playback: the chapters on
     export, adding tracks, tempo, play and Training Mode are left out), then the Shortcuts. The section is made here, not
     in docs/guide: the guide is the manual of the standalone app. */
  const DAW_ID = "daw";
  const DAW_CHAPTERS = ["01-why-sample-exact.html", "03-detection.html", "03b-detector-plugins.html", "04-first-sample.html",
    "08-velocity.html", "10-bleed.html", "11-learn.html"];
  const DAW_HTML = `<h2>Drum Spike in a DAW</h2>
    <p>In a DAW, Drum Spike is the page of a plugin window. The path has five steps:</p>
    <ol id="dawSteps">
      <li>Put the plugin on the drum clips of the DAW.</li>
      <li>Wait for the analysis. The page says when the hits are ready.</li>
      <li>Check the hits and correct them.</li>
      <li>Set the note and the role of each track in the Inspector.</li>
      <li>Drag or export the MIDI and the spikes into the DAW. Drop them at the start of the session.</li>
    </ol>
    <p>The host owns the tracks, the files and the tempo. To add, remove or rename a track, or to change the tempo, do it in the DAW. The host plays the audio: play, stop and locate in the page and in the DAW move together. The page has no mixer, no click and no audition.</p>
    <p>What stays: Analyze All and the analysis of one track, every edit of a hit, the review of doubtful hits, Learn, the zoom and the follow of the playhead, and Undo.</p>`;
  const dawList = () => hostMode() ? manifest.chapters.filter(c => DAW_CHAPTERS.includes(c.file)) : manifest.chapters;

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
    if (hostMode()) nav.appendChild(navEntry(DAW_ID, "In a DAW"));
    for (const c of dawList()) nav.appendChild(navEntry(c.file, c.title));
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
        <p class="hint">${hostMode() ? "The keys of the page, by menu name. The keys without Cmd, Control or Option work when no text field has the focus."
          : "Every command of the app, by menu. The keys without Cmd, Control or Option work when no text field has the focus; the menu does not show them (it would take them from the text fields)."}</p>
        <p class="hint" id="keymapPreset"></p>
        <table class="keymap" id="keymapTable"></table>
      </div>`;
    $("#keymapPreset").textContent = `Key preset: ${DSKeys.presetName()} (change it in Drum Spike > Settings).`;
    DSKeys.renderHelp($("#keymapTable"));
  }

  async function show(id) {
    if (id === "shortcuts") { shortcutsSection(); }
    else if (id === DAW_ID) {
      const div = document.createElement("div");
      div.className = "helpChapterPage";
      div.innerHTML = DAW_HTML;   // a fixed text of this file
      page().replaceChildren(div);
    } else {
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
    if (hostMode() && id !== DAW_ID && id !== "shortcuts" && !DAW_CHAPTERS.includes(id)) id = DAW_ID;   // #539: a chapter of the standalone manual is not in the host mode manual
    if (!id) id = hostMode() ? DAW_ID : manifest.chapters[0].file;   // first chapter (the entry page)
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
    // Help > Drum Spike Help (no id): the last chapter read, else the first one; never the Shortcuts
    // section, which has its own menu item (Help > Keyboard Shortcuts opens it by id)
    showModal("#helpModal");
    await openSection(id || (current && (manifest?.chapters?.some(c => c.file === current) || current === DAW_ID) ? current : null));
  }

  function hooks() {
    $("#helpDone").onclick = () => $("#helpModal").classList.add("hidden");
    $("#helpBack").onclick = () => { if (at > 0) { at--; openSection(history[at], false); } };
    $("#helpFwd").onclick = () => { if (at < history.length - 1) { at++; openSection(history[at], false); } };
    page().addEventListener("click", ev => {
      const a = ev.target.closest("a");
      if (!a) return;
      ev.preventDefault();
      if (a.classList.contains("ext")) Promise.resolve(B.openUrl(a.href)).catch(e => setMsg(`cannot open ${a.href}: ${e && e.message}`, "error"));        // out: the default browser; #559: a refusal shows the address on the status line
      else if (a.classList.contains("chapter")) openSection(a.getAttribute("href"));
      else if (a.getAttribute("href")?.startsWith("#")) openSection(a.getAttribute("href").slice(1));
    });
  }

  window.DSHelp = { open, hooks };
})();
