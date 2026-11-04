/* Drum Spike v2 — addtracks (refactor plan step 5.2, R9 A2)
   Add tracks by drop or dialog: setupUpload, the confirm list askTracks, addTracksFlow and the
   follows it sets. Moved verbatim from app.js. Plain script (no build step), loaded after
   shell.js and before app.js; its one load-time statement sets DSShell.hooks.otherPaths. */
"use strict";

/* ---------------- Add tracks: the native dialog (P3-4c) ---------------- */
function setupUpload() {
  $("#addTracksBtn").onclick = () => { if (!needProject()) return; addTracksDialog(); };
}

/* ---------------- add tracks by drop or dialog (P3-4c) ---------------- */
/* The app gets paths, not bytes. A drop on the window (shell.js, DSShell.hooks.otherPaths), File > Add Tracks and
   the toolbar button end in addTracksFlow(paths). B.tracksScan lists the files with the guessed role and note
   (names of the owner's scheme: k in, s top, t1, oh ...) and the files the app cannot read, with the reason.
   #225: a file whose name gives no role gets the role the audio suggests; the row is marked "from audio" and the
   select is preset to it, the user can change it before the add.
   One click on "Add n tracks" adds them, referenced; the box copies them into the project instead. The engineer
   types nothing. With no project open the drop makes one, named after the folder.
   X-1 (P3-9b): a Pro Tools "Audio Files" folder holds guitars, vocals and rooms too. When the list has files with
   a drum role and files without one (role "other"), only the drum files are checked; "Drums only" and "All"
   switch, "n of m checked" shows the count, and a role picked by hand checks the row. */
let addingTracks = false;
function dropProjectName(paths) {
  const p = String(paths[0] || "").replace(/\/+$/, "").split("/");
  let n = p.pop();
  if (/\.(wav|wave|bwf|aiff?|flac|mp3)$/i.test(n || "")) n = p.pop();
  if (/^audio files$/i.test(n || "")) n = p.pop();
  return (n || "").trim();
}
async function newDropProject(name, copy) {   // a free project name: "<name>", "<name> 2" ...
  for (let k = 1; k <= 50; k++) {
    try { return await B.newProject(name ? (k === 1 ? name : `${name} ${k}`) : null, null, copy ? "copied" : "referenced"); }
    catch (e) { if (e.status !== 409 || !name) throw e; }
  }
  throw new Error("no free project name");
}
/* The confirm list. Resolves {paths, roles, notes, copy} for the checked files, or null for Cancel. */
function askTracks(paths, scan0) {
  const modal = $("#tracksModal"), rows = $("#tracksRows"), addBtn = $("#tracksAdd"), sel = $("#tracksPreset");
  let scan = scan0;
  let scanMixed = false;   // #243: the scan's mixed roles decide the "not checked" note until the next scan
  const noteOf = row => { const n = noteFromName(row.querySelector(".unote").value); return n === null ? ROLE_DEFAULT_NOTE[row.querySelector(".urole").value] : n; };
  const checked = () => [...rows.querySelectorAll(".urow")].filter(r => r.querySelector(".uuse").checked);
  const isDrum = role => role !== "other";
  /* P3-18: a kick sub mic follows the kick in. The source: a kick track of the project that follows nothing, else
     the first checked kick row of the list whose name has no "sub". A row whose name has "sub" and whose role is
     kick gets the box "follow <source>", checked; the user can uncheck it. */
  const isSub = r => /sub/i.test(r.dataset.stem) && r.querySelector(".urole").value === "kick";
  function kickSource() {
    const own = S.sid ? S.order.find(n => S.tracks[n].role === "kick" && !S.tracks[n].follows) : null;
    if (own) return { name: own, row: null };
    const r = checked().find(r => r.querySelector(".urole").value === "kick" && !/sub/i.test(r.dataset.stem));
    return r ? { name: null, row: r } : null;
  }
  function refreshFollow() {
    const src = kickSource();
    for (const r of rows.querySelectorAll(".urow")) {
      const wrap = r.querySelector(".ufollowWrap"), show = !!src && isSub(r) && src.row !== r;
      wrap.classList.toggle("hidden", !show);
      wrap.dataset.src = show ? (src.name || "") : ""; wrap.dataset.srcStem = show && src.row ? src.row.dataset.stem : "";
      if (show) r.querySelector(".ufollowTxt").textContent = `follow ${src.name || src.row.dataset.stem}`;   // names are data
    }
  }
  /* #243: the summary is recounted from the rows on every change: a role picked by hand leaves its
     scan bucket (the names, the audio) and counts as "set by you". */
  function guessSummary() {
    const rs = [...rows.querySelectorAll(".urow")];
    const hand = rs.filter(r => r.dataset.hand).length;
    const names = rs.filter(r => r.dataset.guess !== "audio" && r.dataset.guess !== "none" && !r.dataset.hand).length;
    const audioN = rs.filter(r => r.dataset.guess === "audio" && !r.dataset.hand).length;
    $("#tracksGuess").textContent = rs.length ? `${names} of ${rs.length} roles come from the names.` +
      (audioN ? ` ${audioN} from the audio.` : "") +
      (hand ? ` ${hand} set by you.` : "") +
      (scanMixed ? " Files without a drum role are not checked." : "") + " Check the list and press Add." : "";
  }
  function count() {
    const n = checked().length, m = rows.querySelectorAll(".urow").length;
    addBtn.textContent = `Add ${n} track${n === 1 ? "" : "s"}`; addBtn.disabled = n === 0;
    if (m) $("#tracksTitle").textContent = addBtn.textContent;
    $("#tracksCount").textContent = m ? `${n} of ${m} checked` : "";
    guessSummary();
    refreshFollow();
  }
  function checkWhere(want) {   // "Drums only" and "All"
    for (const r of rows.querySelectorAll(".urow")) r.querySelector(".uuse").checked = want(r.querySelector(".urole").value);
    count();
  }
  function render() {
    rows.innerHTML = "";
    scanMixed = scan.files.some(f => isDrum(f.role)) && scan.files.some(f => !isDrum(f.role));
    for (const f of scan.files) {
      const audio = f.guess === "audio";   // #225: the role comes from the audio, the name gave none
      const fuzzy = f.guess === "fuzzy";   // #285: a typo guess from the name (one edit from a term): marked, check it
      const row = document.createElement("div"); row.className = "urow" + (f.guess === "none" ? " noguess" : "");
      row.dataset.path = f.path; row.dataset.stem = f.stem; row.dataset.guess = f.guess;   // #243: where the scan took the role from
      row.innerHTML = `<input type="checkbox" class="uuse" ${!scanMixed || isDrum(f.role) ? "checked" : ""} title="add this file">
        <span class="fname"></span>
        <select class="urole">${["kick","snare","tom","hat","cymbal","other"].map(r => `<option ${r === f.role ? "selected" : ""}>${r}</option>`).join("")}</select>
        <input class="unote" type="text" value="${nameFromNote(f.note)}" title="MIDI note name">
        ${audio ? '<span class="ufromaudio" title="the role the audio of the file suggests">from audio</span>' : ""}
        ${fuzzy ? '<span class="ufromaudio" title="the name looks like a typo of a drum name: the role is a guess, check it">guessed</span>' : ""}
        <label class="ufollowWrap hidden" title="P3-18: a sub mic takes the hits of the kick track (sample-exact, velocities from its own signal) instead of its own detection"><input type="checkbox" class="ufollow" checked> <span class="ufollowTxt"></span></label>`;
      const fn = row.querySelector(".fname");
      fn.textContent = `${f.name} (${(f.size / 1e6).toFixed(1)} MB, ${f.sr / 1000} kHz)`;   // textContent: file names are data
      fn.title = f.guess === "none" ? `${f.path}\nno role recognised from the name: it stays "other" unless you pick one`
        : audio ? `${f.path}\nthe role is the audio's suggestion: change it if it is wrong`
        : fuzzy ? `${f.path}\nthe role is a guess from a name that looks like a typo: change it if it is wrong` : f.path;
      row.querySelector(".urole").onchange = ev => {
        row.querySelector(".unote").value = nameFromNote(ROLE_DEFAULT_NOTE[ev.target.value]); row.classList.remove("noguess");
        row.dataset.hand = "1";   // #243: the role is picked by hand now, whatever the scan guessed
        const mark = row.querySelector(".ufromaudio"); if (mark) mark.remove();   // #225: a role picked by hand is no audio suggestion
        if (isDrum(ev.target.value)) row.querySelector(".uuse").checked = true;   // a role picked by hand: add the file
        count();
      };
      row.querySelector(".uuse").onchange = count;
      rows.appendChild(row);
    }
    const rej = $("#tracksRejected"); rej.innerHTML = "";
    for (const r of scan.rejected) {
      const d = document.createElement("div"); d.className = "rejrow";
      d.textContent = `not added: ${r.name} - ${r.reason}`; d.title = r.path; rej.appendChild(d);
    }
    $("#tracksTitle").textContent = scan.files.length ? `Add ${scan.files.length} track${scan.files.length === 1 ? "" : "s"}` : "No track can be added";
    $("#tracksPick").classList.toggle("hidden", !scan.files.length);
    addBtn.classList.toggle("hidden", !scan.files.length);
    $("#tracksCancel").textContent = scan.files.length ? "Cancel" : "Close";
    count();
  }
  async function presets(select) {
    let list = [];
    try { list = await B.kitPresets(); } catch (e) { /* no preset folder: the list stays empty */ }
    sel.innerHTML = `<option value="">Kit preset: names of the scheme</option>` + list.map(p => `<option></option>`).join("");
    [...sel.options].slice(1).forEach((o, i) => { o.value = o.textContent = list[i].name; });
    sel.value = select || "";
    if (sel.value !== (select || "")) sel.value = "";
  }
  return new Promise(resolve => {
    const done = v => { modal.classList.add("hidden"); $("#tracksCancel").onclick = $("#tracksAdd").onclick = sel.onchange = $("#tracksPresetSave").onclick = $("#tracksDrums").onclick = $("#tracksAll").onclick = null; resolve(v); };
    $("#tracksDrums").onclick = () => checkWhere(isDrum);
    $("#tracksAll").onclick = () => checkWhere(() => true);
    $("#tracksCancel").onclick = () => done(null);
    addBtn.onclick = () => { const r = checked(); done({ paths: r.map(x => x.dataset.path), roles: r.map(x => x.querySelector(".urole").value), notes: r.map(noteOf), copy: $("#tracksCopy").checked,
      stems: r.map(x => x.dataset.stem),   // P3-18: the follow of each row: {src: an existing track} or {srcStem: a row of this list}, or null
      follows: r.map(x => { const w = x.querySelector(".ufollowWrap"); return !w.classList.contains("hidden") && x.querySelector(".ufollow").checked ? { src: w.dataset.src || null, srcStem: w.dataset.srcStem || null } : null; }) }); };
    sel.onchange = async () => {
      try { scan = await B.tracksScan(paths, sel.value); render(); } catch (e) { setMsg(`kit preset failed: ${e.message}`, "error"); }
    };
    $("#tracksPresetSave").onclick = async () => {
      const name = dropProjectName(paths) || S.name || "My kit";
      const rules = [...rows.querySelectorAll(".urow")].map(r => ({ pattern: r.dataset.stem, role: r.querySelector(".urole").value, note: noteOf(r) }));
      try { const r = await B.kitPresetSave(name, rules); await presets(r.name); setMsg(`saved the kit preset "${r.name}" (${r.rules} names)`); }
      catch (e) { setMsg(`kit preset failed: ${e.message}`, "error"); }
    };
    $("#tracksCopy").checked = false;
    render();
    presets("").then(() => {});
    modal.classList.remove("hidden");
    (scan.files.length ? addBtn : $("#tracksCancel")).focus();
  });
}
async function addTracksFlow(paths) {
  if (!paths || !paths.length) return;
  if (addingTracks || !$("#tracksModal").classList.contains("hidden")) { setMsg("answer the open dialog first"); return; }
  addingTracks = true;
  try {
    setMsg("reading the files…");
    const scan = await B.tracksScan(paths, "");
    if (!scan.files.length && !scan.rejected.length) { setMsg("no audio files in the dropped items"); return; }
    const pick = await askTracks(paths, scan);
    if (!pick) { setMsg("adding tracks cancelled"); return; }
    if (!S.sid) await useProject(await newDropProject(dropProjectName(paths), pick.copy));
    await flushEvents();
    showProgress(0, `${pick.copy ? "copying" : "adding"} 0/${pick.paths.length}`, pick.copy);   // P3-4d: a copy has Cancel
    let live = true;   // a late message (the test bridge polls) must not override the result
    const r = await B.tracksAdd(S.sid, pick.paths, pick.roles, pick.notes, pick.copy,
      p => {
        if (!live) return;
        if (p.state === "running") showProgress(Math.round(100 * p.done / Math.max(1, p.total)), `${pick.copy ? "copying" : "adding"} ${p.done}/${p.total} - ${p.file}`, pick.copy);
        else if (p.state === "peaks") showProgress(Math.round(100 * p.done / Math.max(1, p.total)), `building waveforms: ${p.done} of ${p.total}`);
      });
    live = false; hideProgress();
    if (r.cancelled) { setMsg("adding tracks cancelled: no track was added and the copied files are removed"); return; }
    const followed = await applyAddFollows(pick, r);   // P3-18
    await initSessions();
    if (await loadProject()) {
      const bad = [...scan.rejected, ...(r.rejected || [])];
      setMsg(`added ${r.added.length} track${r.added.length === 1 ? "" : "s"} (${pick.copy ? "audio copied into the project" : "audio referenced where it is"})` +
        (followed.length ? `; ${followed.map(f => `${f[0]} follows ${f[1]}`).join(", ")}` : "") +
        (bad.length ? `; ${bad.length} not added (${bad[0].name}: ${bad[0].reason})` : "") + " - hit Analyze all");
      setMsgDetail(bad.map(b => `${b.name}: ${b.reason}`).join("\n"));
    }
  } catch (e) { hideProgress(); setMsg(`add tracks failed: ${e.message}`, "error"); setMsgDetail(e.detail); }
  finally { addingTracks = false; }
}
/* P3-18: the follows the list suggested, set after the add (the names come from the service): [[follower, source], ...] */
async function applyAddFollows(pick, r) {
  const byPath = {};
  for (const t of r.tracks || []) byPath[t.path] = t.name;
  const out = [];
  for (let i = 0; i < pick.paths.length; i++) {
    const f = pick.follows && pick.follows[i], name = byPath[pick.paths[i]];
    if (!f || !name) continue;
    const src = f.src || (f.srcStem ? byPath[pick.paths[pick.stems.indexOf(f.srcStem)]] : null);
    if (!src || src === name) continue;
    try { await B.setFollow(S.sid, name, src); out.push([name, src]); }
    catch (e) { setMsg(`${name}: follow failed: ${errText(e)}`, "error"); }
  }
  return out;
}
async function addTracksDialog() {   // File > Add Tracks and the toolbar button: the native file dialog
  const files = await B.pickAudioFiles();
  if (files && files.length) await addTracksFlow(files);
}
if (window.DSShell) DSShell.hooks.otherPaths = (paths, pos) => (lyDrop(paths, pos) ? undefined : addTracksFlow(paths));   // a drop on the window (open-paths, not an HTML5 drop); P4-7a: on the Layer zone it sets the layer
