/* Drum Spike v2 — layer (refactor plan step 5.2, R9 A2)
   The one-shot layer of a track (LY), its onset view (LYV), the sampler calibration (SM) and the
   refresh after a track-layer event (LYR). Moved verbatim from app.js. Plain script (no build
   step), loaded after learn.js and before app.js; its one load-time statement registers the
   B.onTrackLayer listener. */
"use strict";

/* ---------------- P4-7a and P4-7b: the one-shot layer of a track ----------------
   The Inspector section Layer (crates/app/src/layer.rs): drop one WAV or AIFF one-shot on the zone (a window drop at the zone's
   position: shell.js passes the drop position) or Choose; level, polarity (normal, invert, auto: the majority of the alignment),
   on/off (playback), the velocity curve (dB with a range, linear, fixed). Align (P4-7b) places each hit's one-shot by its
   cross-correlation with the close mic in the playback and the render; Measure shows the polarity, the flips and the match.
   Render writes Exports/<name>_layer.wav. Each change is one track_layer_set (the service updates a playing transport).
   LY.align[name] keeps the last alignment: the info line shows the match of the selected hit. */
const LY = { align: {}, seq: 0 };
const LY_EXT = /\.(wav|wave|bwf|aif|aiff|aifc)$/i;
function lyLayer(name) { const t = S.tracks[name]; return t && t.layer ? t.layer : null; }
function lyHTML() {
  const info = txt => ` <span class="pinfo" title="${txt}">ⓘ</span>`;
  return `
      <div class="prow">
        <div class="pctl pctlWide lyCol">
          <div class="lyZone" title="Drop one WAV or AIFF one-shot here, or choose one. The app plays it at every hit of this track, with its onset on the hit, and renders it as Exports/&lt;track&gt;_layer.wav.">
            <span class="lyFile">no one-shot</span>
            <span class="lyBtns"><button type="button" class="btn mini lyChoose">Choose…</button><button type="button" class="btn mini lyRemove" title="Remove the layer of this track">Remove</button></span>
          </div>
          <span class="phint lyInfo"></span>
        </div>
      </div>
      <div class="prow lyRow">
        <div class="pctl pctlWide lyCol">
          <div class="lyViewHead">
            <span class="lyViewTitle">Onset</span>
            <span class="lyBtns"><button type="button" class="btn mini lyPlay" title="Hear the one-shot once from its first sample (the close mic sounds underneath, as with the audition key)">Play one-shot</button><button type="button" class="btn mini lyReset" title="Put the onset line back on the measured onset">Reset onset</button></span>
          </div>
          <span class="phint lyViewHint">Drag the line (the handle at its top) to set the moment the one-shot sounds on each hit.</span>
          <span class="phint lyViewInfo"></span>
          <div class="lyViewWrap" title="The first 200 ms of the one-shot. Drag the line to override the onset the layer places on every hit; drag the wave to pan, wheel to zoom. With a sampler calibration the bounce shows with its offset line.">
            <canvas class="lyView"></canvas>
          </div>
        </div>
      </div>
      <div class="prow lyRow">
        <label>Play ${info("Hear the one-shot at every hit while the session plays (also at half speed). Overlapping hits sound together; nothing is cut. The render does not depend on it.")}</label>
        <div class="pctl"><input type="checkbox" class="lyOn"></div>
      </div>
      <div class="prow lyRow">
        <label>Level ${info("The level of the one-shot in dB (-60 to +12), before the velocity curve.")}</label>
        <div class="pctl"><input type="number" class="lyLevel" min="-60" max="12" step="0.5" value="0"><span class="punit">dB</span></div>
      </div>
      <div class="prow lyRow">
        <label>Polarity ${info("Normal: the one-shot as it is. Inverted: upside down. Auto (experimental): the polarity that most hits match in the alignment with the close mic (Measure); on the real sessions it picked inverted for printed samples that were normal, so check it by ear.")}</label>
        <div class="pctl"><select class="lyPol"><option value="normal">normal</option><option value="invert">inverted</option><option value="auto">auto (experimental)</option></select></div>
      </div>
      <div class="prow lyRow">
        <label>Velocity curve ${info("The gain of each hit from its velocity. dB: velocity 127 is the level, lower velocities fall linearly in dB over the range. Linear: the amplitude follows the velocity. Fixed: every hit at the level.")}</label>
        <div class="pctl"><select class="lyCurve"><option value="db">dB</option><option value="linear">linear</option><option value="fixed">fixed</option></select><input type="number" class="lyRange" min="0" max="96" step="1" value="24"><span class="punit lyRangeUnit">dB range</span></div>
      </div>
      <div class="prow lyRow">
        <label>Align ${info("Place each hit's one-shot where its attack matches the close mic best (cross-correlation of the first 10 ms, within 3 ms, sub-sample in the render). The match (0 to 1) of each hit shows in the info line; a low match marks an uncertain hit.")}</label>
        <div class="pctl"><input type="checkbox" class="lyAlign"><button type="button" class="btn mini lyMeasure" title="Measure the alignment of every hit now">Measure</button></div>
      </div>
      <div class="prow lyRow"><label></label><div class="pctl"><span class="phint lyAlignInfo"></span></div></div>
      <div class="pactions lyRow"><button type="button" class="btn lyRender" title="Save the one-shot at every hit as a WAV file (the save dialog starts in the export folder): the rate, length and session position of the track">Render Layer</button></div>
      <div class="prow">
        <div class="pctl pctlWide lyCol">
          <div class="smZone" title="Sampler calibration (P4-7c): write the calibration MIDI, import it at the session start, bounce the sampler's kit piece for its note, then load the bounce here (or drop it on this zone). The MIDI exports then put each note at the aligned attack minus the sampler offset.">
            <span class="smFile">no sampler calibration</span>
            <span class="smBtns"><button type="button" class="btn mini smCal" title="Save the calibration MIDI: one note (this track's note) on a bar line; the save dialog starts in the export folder">Calibration MIDI…</button><button type="button" class="btn mini smLoad" title="Load the bounce of the calibration note (WAV or AIFF, from the session start)">Load bounce…</button></span>
          </div>
          <span class="phint smInfo"></span>
          <span class="smBtns smRow2"><button type="button" class="btn mini smShot" title="Use the loaded bounce as this track's one-shot (with its polarity)">Use as one-shot</button><button type="button" class="btn mini smClear" title="Remove the sampler calibration of this track">Clear</button></span>
        </div>
      </div>`;
}
function lyCurveOf(div) {
  const k = div.querySelector(".lyCurve").value;
  return k === "db" ? { kind: "db", range_db: Math.max(0, Math.min(96, +div.querySelector(".lyRange").value || 0)) } : { kind: k };
}
function lyShow(name, got) {   // the fields from the track's layer; `got` (an answer) adds the file state and the onset
  const div = $("#insParams"); if (!div || INS.name !== name) return;
  const l = lyLayer(name), zone = div.querySelector(".lyZone"); if (!zone) return;
  smShow(name);
  for (const r of div.querySelectorAll(".lyRow")) r.classList.toggle("hidden", !l);
  div.querySelector(".lyRemove").classList.toggle("hidden", !l);
  if (got) LY.got = { name, got };
  const g = got || (LY.got && LY.got.name === name ? LY.got.got : null);
  const file = l ? (g && g.file) || String((l.source && (l.source.path || l.source.relative)) || "").split("/").pop() : "";
  // P4-40e: another one-shot resets the view's window and its data
  if (LYV.file[name] !== file) { delete LYV.win[name]; delete LYV.cache[name]; LYV.file[name] = file; }
  div.querySelector(".lyFile").textContent = l ? file : "no one-shot";
  zone.classList.toggle("set", !!l);
  const st = g && g.status && g.status !== "ok" ? ` · file ${g.status}` : "";
  const eff = l ? (l.onset_set != null ? l.onset_set : l.onset) : 0;
  div.querySelector(".lyInfo").textContent = l ? `onset ${(1000 * eff / l.sr).toFixed(2)} ms, ` +
    (l.onset_set != null ? `set by hand (measured ${(1000 * l.onset / l.sr).toFixed(2)} ms)` : "measured") +
    ` · ${l.sr / 1000} kHz · ${l.channels === 2 ? "stereo" : "mono"}${st}` : "";
  if (!l) { div.querySelector(".lyAlignInfo").textContent = ""; lyViewDraw(name); return; }
  div.querySelector(".lyOn").checked = l.on !== false;
  div.querySelector(".lyLevel").value = String(l.level_db || 0);
  div.querySelector(".lyPol").value = l.polarity || "normal";
  const c = l.curve || { kind: "db", range_db: 24 };
  div.querySelector(".lyCurve").value = c.kind;
  div.querySelector(".lyRange").value = String(c.range_db === undefined ? 24 : c.range_db);
  for (const el of div.querySelectorAll(".lyRange, .lyRangeUnit")) el.classList.toggle("hidden", c.kind !== "db");
  div.querySelector(".lyAlign").checked = !!l.align;
  lyAlignText(name);
  lyViewDraw(name);   // P4-40e: the view follows (the answer's onset, an override, a Reset)
}
function lyAlignText(name) {
  const div = $("#insParams"), a = LY.align[name], el = div && div.querySelector(".lyAlignInfo"); if (!el) return;
  const l = lyLayer(name), auto = !!l && l.polarity === "auto";
  if (!a) { el.textContent = auto ? "auto is experimental: press Measure to see the polarity it picks" : ""; el.classList.remove("lyWarn"); return; }
  let t = `polarity ${a.polarity} (${a.polarity === "inverted" ? a.inverted : a.as_is} of ${a.n}) · ${a.flips.length} flip${a.flips.length === 1 ? "" : "s"} · match ${a.median_match === null ? "-" : a.median_match.toFixed(2)} · ${a.uncertain} uncertain`;
  const warn = auto && a.polarity === "inverted";
  if (auto) t += " · auto is experimental";
  if (warn) t += " · WARNING: auto picked inverted; printed samples were normal in the tests, so check by ear or use normal";
  el.textContent = t;
  el.classList.toggle("lyWarn", warn);
}
async function lySet(name, o) {   // one track_layer_set; the answer's layer goes into the track
  if (!S.sid) return null;
  const seq = ++LY.seq;
  try {
    const r = await B.layerSet(S.sid, name, o);
    const t = S.tracks[name];
    if (t) { if (r.layer) { t.layer = r.layer; t.layer_status = r.status; } else { delete t.layer; delete t.layer_status; } refreshRelinkBar(); }
    if ("path" in o || "polarity" in o || "onset" in o) delete LY.align[name];
    if (seq === LY.seq) lyShow(name, r);
    insSumRefresh(); drawInfo();
    if (r.transport_error) setMsg(`layer of ${name}: saved, but the playing transport did not take it: ${r.transport_error}`);
    return r;
  } catch (e) { setMsg(`layer of ${name}: ${e.message}`); lyShow(name); return null; }
}
async function lySetFile(name, path) {
  const r = await lySet(name, { path });
  if (r && r.layer) setMsg(`${name}: layer ${r.file}, onset ${r.onset_ms} ms`);
  return r;
}
/* P4-7d: the Layer section's dialogs. A load dialog starts in the folder the last load of its kind used (app settings,
   so kept per app, not per project), else the project folder; a save dialog starts in the project's export folder
   (the Export dialog's, else Exports/). Cancel does nothing. */
async function dialogFolder(key) {
  try { const st = await B.appSettingsGet(); if (st && typeof st[key] === "string" && st[key]) return st[key]; } catch (e) { /* the project folder */ }
  try { const s = (await B.sessions()).find(x => x.sid === S.sid); return (s && s.folder) || null; } catch (e) { return null; }
}
async function rememberDialogFolder(key, path) {   // the folder of a file the user chose, kept for the next load of its kind
  const i = path.lastIndexOf("/");
  if (i > 0) { try { await B.appSettingsSet(key, path.slice(0, i)); } catch (e) { /* the dialog starts where it did */ } }
}
async function exportFolder() {   // the folder a save starts in: the one kept by the Export dialog, else Exports/ of the project
  try { return (await B.exportExisting(S.sid, {})).dest || null; } catch (e) { return null; }
}
const joinFile = (folder, file) => folder ? `${folder}/${file}` : file;
async function lyChoose(name) {
  const p = await B.pickOneShot(await dialogFolder("layer_one_shot_dir"));
  if (!p) return;
  await rememberDialogFolder("layer_one_shot_dir", p);
  await lySetFile(name, p);
}
/* ---------------- P4-40e: the onset view of the Layer section ----------------
   The first 200 ms of the one-shot (crates/app/src/layer.rs track_layer_wave) with a line at the onset (the lanes'
   tToX on the view's own window). Drag the line: one track_layer_set {onset} on release overrides the measured onset
   for the render, the playback and the Align; Reset sends onset null. Drag the wave pans, the wheel zooms. With a
   sampler calibration (P4-7c) the bounce (track_sampler_wave) shows with a line at its offset. Play runs the
   transport audition with `shot`: the one-shot once at the window start. */
const LYV = { win: {}, file: {}, cache: {}, err: {}, drag: null, pan: null, seq: 0, draws: 0 };
function lyView(name) { const div = $("#insParams"); return div && INS.name === name ? div.querySelector(".lyView") : null; }
function lyViewWin(name) {   // the visible window of the track's one-shot, in its own seconds
  const l = lyLayer(name), dur = l ? l.frames / l.sr : 0.2, key = l ? l.source.path : "";
  let w = LYV.win[name];
  if (!w || w.key !== key) {   // a new one-shot opens at its onset, so a sound after a long lead-in shows
    const t0 = l ? Math.max(0, Math.min(lyOnsetS(name) - 0.01, dur - 0.2)) : 0;
    w = LYV.win[name] = { key, t0, t1: Math.min(dur, t0 + 0.2) };
  }
  const span = Math.max(0.002, Math.min(dur, w.t1 - w.t0));
  w.t0 = Math.max(0, Math.min(w.t0, Math.max(0, dur - span)));
  w.t1 = Math.min(dur, w.t0 + span);
  return w;
}
function lyOnsetS(name) {   // the effective onset in seconds (the drag preview wins)
  const l = lyLayer(name); if (!l) return null;
  if (LYV.drag && LYV.drag.name === name) return LYV.drag.s / l.sr;
  return (l.onset_set != null ? l.onset_set : l.onset) / l.sr;
}
function lyViewX(name, t, W) { const w = lyViewWin(name); return (t - w.t0) / (w.t1 - w.t0) * W; }
function lyViewInfo(name) {
  const div = $("#insParams"); if (!div || INS.name !== name) return;
  const l = lyLayer(name), el = div.querySelector(".lyViewInfo"); if (!el) return;
  if (!l) { el.textContent = ""; return; }
  if (LYV.err[name]) { el.textContent = `the wave could not be read: ${LYV.err[name]}`; return; }
  const w = lyViewWin(name), on = lyOnsetS(name);
  const from = w.t0 > 0.0005 ? `from ${w.t0.toFixed(2)} s` : "first";
  const off = on < w.t0 || on > w.t1 ? ", the line is off the view" : "";
  el.textContent = `${from} ${(w.t1 - w.t0).toFixed(2)} s of ${(l.frames / l.sr).toFixed(2)} s${off}`;
}
function lyViewCache(name) {   // the cached wave of this one-shot (a cache of another file is not drawn)
  const c = LYV.cache[name], l = lyLayer(name);
  return c && l && c.key === l.source.path ? c : null;
}
async function lyViewFetch(name, s0, s1) {   // the visible window of the one-shot (and of the bounce)
  const l = lyLayer(name);
  if (!S.sid || !l) return;
  const seq = ++LYV.seq;
  const c = { key: l.source.path };
  try {
    c.shot = await B.layerWave(S.sid, name, s0, s1);
    if (seq !== LYV.seq) return;
    const s = S.tracks[name] && S.tracks[name].sampler;
    if (s && s.file) { try { c.bounce = await B.samplerWave(S.sid, name, s0, s1); } catch (e) { /* no bounce this time */ } }
    if (seq !== LYV.seq) return;
    LYV.cache[name] = c;
    delete LYV.err[name];
  } catch (e) {
    if (seq !== LYV.seq) return;   // the message shows in the info line, not as a black view
    LYV.err[name] = String(e && e.message !== undefined ? e.message : e);
    lyViewInfo(name);
    return;
  }
  if (INS.name === name) lyViewDraw(name);
}
function lyViewColumns(d, off, sr, w, W) {   // per pixel column the min and max of the samples it covers
  const min = new Float32Array(W + 1), max = new Float32Array(W + 1), seen = new Uint8Array(W + 1);
  for (let i = 0; i < d.length; i++) {
    const t = (off + i) / sr;
    if (t < w.t0 || t > w.t1) continue;
    const x = Math.round((t - w.t0) / (w.t1 - w.t0) * W);
    if (d[i] < min[x] || !seen[x]) min[x] = d[i];
    if (d[i] > max[x] || !seen[x]) max[x] = d[i];
    seen[x] = 1;
  }
  return { min, max, seen };
}
function lyCanvas(c) {   // size the backing store to the element; returns [ctx, W, H] in CSS px
  const W = c.clientWidth, H = c.clientHeight, dpr = window.devicePixelRatio || 1;
  if (c.width !== Math.round(W * dpr) || c.height !== Math.round(H * dpr)) { c.width = Math.round(W * dpr); c.height = Math.round(H * dpr); }
  const ctx = c.getContext("2d"); ctx.setTransform(dpr, 0, 0, dpr, 0, 0); ctx.clearRect(0, 0, W, H);
  LYV.draws++;   // the tests read it: nothing draws without a change
  return [ctx, W, H];
}
function lyViewDraw(name) {
  const c = lyView(name); if (!c) return;
  const [ctx, W, H] = lyCanvas(c);
  const l = lyLayer(name); if (!l) return;
  const w = lyViewWin(name), sr = l.sr, cache = lyViewCache(name);
  const acc = LN_CSS.getPropertyValue("--accent").trim() || "#4da3ff";
  const mid = H / 2, amp = H / 2 - 4;
  const trace = (buf, color) => {
    if (!buf) return;
    const { min, max, seen } = lyViewColumns(buf.data, buf.start, buf.sr, w, W);
    ctx.strokeStyle = color; ctx.lineWidth = 1; ctx.beginPath();
    for (let x = 0; x <= W; x++) {
      if (!seen[x]) continue;
      ctx.moveTo(x + 0.5, mid - max[x] * amp);
      ctx.lineTo(x + 0.5, mid - min[x] * amp);
    }
    ctx.stroke();
  };
  trace(cache && cache.shot, "#88909c");
  trace(cache && cache.bounce, "rgba(136, 144, 156, 0.55)");
  // the lines: the onset (drag it), the measured one when overridden, the bounce's sampler offset
  const line = (t, color, width) => {
    const x = Math.round(lyViewX(name, t, W)) + 0.5;
    ctx.strokeStyle = color; ctx.lineWidth = width; ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, H); ctx.stroke();
  };
  line(lyOnsetS(name), acc, 1.5);
  const hx = Math.round(lyViewX(name, lyOnsetS(name), W));   // the drag handle: a tab on the top of the line
  ctx.fillStyle = acc; ctx.fillRect(hx - 4, 0, 9, 7);
  if (l.onset_set != null && !(LYV.drag && LYV.drag.name === name)) line(l.onset / l.sr, "#6e7681", 1);
  const s = S.tracks[name] && S.tracks[name].sampler;
  if (s) line(s.offset_samples / s.sr, "#ffb84d", 1);
  lyViewInfo(name);
  // the data for the window (a little wider, so a small pan does not refetch)
  const s0 = Math.floor(w.t0 * sr), s1 = Math.ceil(w.t1 * sr) + 1;
  const got = cache && cache.shot && cache.shot.sr === sr && cache.shot.start <= s0 &&
    cache.shot.start + cache.shot.data.length >= Math.min(s1, cache.shot.start + cache.shot.data.length);
  if (!got) lyViewFetch(name, Math.max(0, s0 - 64), s1 + 64);
}
function lyViewAttach(div, name) {
  const c = div.querySelector(".lyView");
  div.querySelector(".lyPlay").onclick = () => audition(false, null, name);
  div.querySelector(".lyReset").onclick = () => lySet(name, { onset: null });
  c.onpointerdown = ev => {
    const l = lyLayer(name); if (!l) return;
    const r = c.getBoundingClientRect(), x = ev.clientX - r.left;
    c.setPointerCapture(ev.pointerId);
    if (Math.abs(x - lyViewX(name, lyOnsetS(name), r.width)) <= 5) {
      LYV.drag = { name, s: (l.onset_set != null ? l.onset_set : l.onset) };
    } else {
      const w = lyViewWin(name);
      LYV.pan = { name, x0: x, t0: w.t0, t1: w.t1 };
    }
    lyViewDraw(name);
  };
  c.onpointermove = ev => {
    const l = lyLayer(name); if (!l) return;
    const r = c.getBoundingClientRect(), x = Math.max(0, Math.min(r.width, ev.clientX - r.left));
    const w = lyViewWin(name), dur = l.frames / l.sr;
    if (LYV.drag && LYV.drag.name === name) {
      LYV.drag.s = Math.max(0, Math.min(l.frames - 1, (w.t0 + x / r.width * (w.t1 - w.t0)) * l.sr));
      lyViewDraw(name);
    } else if (LYV.pan && LYV.pan.name === name) {
      const dt = (LYV.pan.x0 - x) / r.width * (LYV.pan.t1 - LYV.pan.t0), span = LYV.pan.t1 - LYV.pan.t0;
      const t0 = Math.max(0, Math.min(LYV.pan.t0 + dt, dur - span));
      Object.assign(w, { t0, t1: Math.min(dur, t0 + span) });
      lyViewDraw(name);
    } else {   // the cursor says what a press does: the line moves, the wave pans
      c.style.cursor = Math.abs(x - lyViewX(name, lyOnsetS(name), r.width)) <= 5 ? "ew-resize" : "grab";
    }
  };
  const up = () => {
    if (LYV.drag && LYV.drag.name === name) {
      const s = Math.round(LYV.drag.s);
      LYV.drag = null;
      lySet(name, { onset: s });   // the answer redraws the view and the info
    }
    LYV.pan = null;
  };
  c.onpointerup = up;
  c.onpointercancel = up;
  c.onwheel = ev => {
    const l = lyLayer(name); if (!l) return;
    ev.preventDefault();
    const r = c.getBoundingClientRect(), x = Math.max(0, Math.min(r.width, ev.clientX - r.left));
    const w = lyViewWin(name), dur = l.frames / l.sr, span0 = w.t1 - w.t0;
    const at = w.t0 + x / r.width * span0;
    const span = Math.max(0.002, Math.min(dur, span0 * (ev.deltaY > 0 ? 1.25 : 0.8)));
    const t0 = Math.max(0, Math.min(at - (at - w.t0) * (span / span0), dur - span));
    Object.assign(w, { t0, t1: Math.min(dur, t0 + span) });
    lyViewDraw(name);
  };
}
/* A window drop (DSShell.hooks.otherPaths): one audio file dropped on the Layer zone of the Inspector sets the layer of its
   track; anything else goes on to Add Tracks. */
function lyDrop(paths, pos) {
  if (!INS.name || !pos) return false;
  const hit = z => { if (!z || !z.offsetParent) return false; const b = z.getBoundingClientRect(); return pos.x >= b.left && pos.x <= b.right && pos.y >= b.top && pos.y <= b.bottom; };
  const sm = hit(document.querySelector("#insParams .smZone")), ly = !sm && hit(document.querySelector("#insParams .lyZone"));
  if (!sm && !ly) return false;
  const audio = (paths || []).filter(p => LY_EXT.test(p));
  if (audio.length !== 1 || paths.length !== 1) { setMsg(sm ? "drop one WAV or AIFF bounce on the sampler calibration" : "drop one WAV or AIFF one-shot on the Layer section"); return true; }
  if (sm) smLoad(INS.name, audio[0], false); else lySetFile(INS.name, audio[0]);
  return true;
}
async function lyMeasure(name) {
  if (!S.sid || !lyLayer(name)) return null;
  try {
    const a = await B.layerAlign(S.sid, name);
    LY.align[name] = a; lyAlignText(name); drawInfo();
    return a;
  } catch (e) { setMsg(`alignment of ${name}: ${e.message}`); return null; }
}
async function lyRender(name) {
  if (!S.sid) return null;
  const path = await B.pickSaveFile("Render the layer", joinFile(await exportFolder(), `${name} layer.wav`), "wav", "WAV audio");
  if (!path) { setMsg(`${name}: render cancelled`); return null; }
  try {
    const r = await B.layerExport(S.sid, name, path);
    setMsg(`${name}: layer rendered to ${r.path}${r.outside ? ` (${r.outside} hits outside the file)` : ""}`);
    return r;
  } catch (e) { setMsg(`render of ${name}: ${e.message}`); return null; }
}
function lyMatchOf(track, t) {   // the match of the hit at t in the last alignment of the track (null: none)
  const a = LY.align[track]; if (!a || !a.hits) return null;
  let lo = 0, hi = a.hits.length;
  while (lo < hi) { const m = (lo + hi) >> 1; if (a.hits[m].t < t - 1e-7) lo = m + 1; else hi = m; }
  const h = a.hits[lo];
  return h && Math.abs(h.t - t) <= 1e-7 ? h : null;
}
/* P4-7c: the sampler calibration of the track (crates/app/src/sampler.rs). Calibration MIDI… writes one note on a bar line into
   the export folder; Load bounce… (or a drop on the zone) measures the delay of the sampler's transient and its polarity
   against the close mic and keeps them in the track (`sampler`); the MIDI exports then put each note at the aligned attack
   minus that offset (the Export dialog's "Compensate sampler offset"). Use as one-shot sets the bounce as the layer. */
const SM = { last: {}, seq: 0 };
const smOf = name => { const t = S.tracks[name]; return t && t.sampler ? t.sampler : null; };
const smSigned = ms => `${ms < 0 ? "-" : "+"}${Math.abs(ms).toFixed(2)}`;
function smText(s) {   // "sampler offset +5.51 ms, polarity inverted"
  return `sampler offset ${smSigned(1000 * s.offset_samples / s.sr)} ms, polarity ${s.polarity || "unknown"}`;
}
function smShow(name) {
  const div = $("#insParams"); if (!div || INS.name !== name || !div.querySelector(".smZone")) return;
  const s = smOf(name), zone = div.querySelector(".smZone");
  zone.classList.toggle("set", !!s);
  div.querySelector(".smFile").textContent = s ? String(s.file || "").split("/").pop() || "bounce" : "no sampler calibration";
  const cal = SM.last[name];
  div.querySelector(".smInfo").textContent = s ? smText(s) + (s.polarity === "inverted" ? " (the bounce is upside down against the close mic)" : "") :
    cal ? cal.text : "Calibration MIDI…: import it at the session start, bounce the sampler's kit piece for that note, then load the bounce here.";
  div.querySelector(".smRow2").classList.toggle("hidden", !s);
}
async function smCalibrate(name) {
  if (!S.sid) return null;
  const path = await B.pickSaveFile("Write the calibration MIDI", joinFile(await exportFolder(), `${name} calibration.mid`), "mid", "MIDI file");
  if (!path) { setMsg(`${name}: calibration MIDI cancelled`); return null; }
  try {
    const r = await B.samplerCalibrationMidi(S.sid, name, null, path);
    SM.last[name] = r; smShow(name);
    setMsg(`${name}: calibration MIDI written to ${r.path}. ${r.text}`);
    return r;
  } catch (e) { setMsg(`calibration MIDI of ${name}: ${errText(e)}`); return null; }
}
function smAdopt(name, r) {   // an answer of track_sampler_set or track_sampler_get goes into the track
  const t = S.tracks[name]; if (!t) return;
  if (r.sampler) t.sampler = r.sampler; else delete t.sampler;
  if (r.layer && r.layer.layer) { t.layer = r.layer.layer; t.layer_status = r.layer.status; delete LY.align[name]; refreshRelinkBar(); }
}
async function smLoad(name, path, oneShot) {
  if (!S.sid) return null;
  const seq = ++SM.seq;
  try {
    const r = await B.samplerSet(S.sid, name, { path, one_shot: !!oneShot });
    smAdopt(name, r);
    if (seq === SM.seq) { delete SM.last[name]; smShow(name); if (r.layer) lyShow(name, r.layer); }
    insSumRefresh(); setDirtyState(true);
    setMsg(`${name}: ${r.sampler ? smText(r.sampler) : "sampler calibration removed"}${r.layer ? "; the bounce is the layer one-shot" : ""}`);
    return r;
  } catch (e) { setMsg(`sampler bounce of ${name}: ${errText(e)}`); smShow(name); return null; }
}
async function smChoose(name) {
  const p = await B.pickBounce(await dialogFolder("sampler_bounce_dir"));
  if (!p) return;
  await rememberDialogFolder("sampler_bounce_dir", p);
  await smLoad(name, p, false);
}
/* The page asks the service for the layer and the sampler of the inspected track while the Layer section is on screen, so
   a change that came through the agent commands (track_layer_set, track_sampler_set) shows without a reload (the P4-9 way:
   the service holds the truth and the page shows what is stored). */
async function smRefreshAll() {   // the sampler of every track from the service
  if (!S.sid) return;
  const sid = S.sid;
  await Promise.all(S.order.map(async n => {
    try { const r = await B.samplerGet(sid, n); if (sid === S.sid && S.tracks[n]) { if (r.sampler) S.tracks[n].sampler = r.sampler; else delete S.tracks[n].sampler; } } catch (e) { /* the page keeps what it has */ }
  }));
}
const LYR = { busy: false, again: new Set() };
async function lyRefresh(name) {
  if (!S.sid || !S.tracks[name]) return false;
  if (LYR.busy) { LYR.again.add(name); return false; }   // an event while one refresh runs: ask again after it
  LYR.busy = true;
  const sid = S.sid, seq = LY.seq, sseq = SM.seq;
  try {
    const [l, s] = await Promise.all([B.layerGet(sid, name), B.samplerGet(sid, name)]);
    if (sid !== S.sid || !S.tracks[name] || seq !== LY.seq || sseq !== SM.seq) return false;
    const t = S.tracks[name];
    const same = JSON.stringify([t.layer || null, t.layer_status || null, t.sampler || null]) === JSON.stringify([l.layer || null, l.layer ? l.status : null, s.sampler || null]);
    if (!same) {
      if (l.layer) { t.layer = l.layer; t.layer_status = l.status; } else { delete t.layer; delete t.layer_status; }
      if (s.sampler) t.sampler = s.sampler; else delete t.sampler;
      if (JSON.stringify(t.layer || null) !== JSON.stringify(l.layer || null)) delete LY.align[name];
      refreshRelinkBar(); insSumRefresh();
    }
    if (INS.name === name) lyShow(name, l.layer ? l : undefined);
    return !same;
  } catch (e) { return false; } finally { LYR.busy = false; for (const n of [...LYR.again]) { LYR.again.delete(n); lyRefresh(n); } }
}
/* P4-33: the service sends track-layer {sid, name} after every layer change, by the page or by an agent command (CLI, MCP).
   The page asks for the stored layer of that track and shows it: Inspector, summary, relink bar. No poll. */
if (B.onTrackLayer) B.onTrackLayer(p => { if (p && p.sid === S.sid && S.tracks[p.name]) lyRefresh(p.name); });
function lyAttach(div, name) {
  div.querySelector(".smCal").onclick = () => smCalibrate(name);
  div.querySelector(".smLoad").onclick = () => smChoose(name);
  div.querySelector(".smShot").onclick = () => { const s = smOf(name); if (s && s.file) smLoad(name, s.file, true); };
  div.querySelector(".smClear").onclick = () => smLoad(name, "", false);
  div.querySelector(".lyChoose").onclick = () => lyChoose(name);
  div.querySelector(".lyRemove").onclick = () => lySet(name, { path: "" });
  div.querySelector(".lyOn").onchange = ev => lySet(name, { on: ev.target.checked });
  div.querySelector(".lyLevel").onchange = ev => { const v = Math.max(-60, Math.min(12, +ev.target.value || 0)); ev.target.value = String(v); lySet(name, { level_db: v }); };
  div.querySelector(".lyPol").onchange = ev => lySet(name, { polarity: ev.target.value });
  div.querySelector(".lyCurve").onchange = () => lySet(name, { curve: lyCurveOf(div) });
  div.querySelector(".lyRange").onchange = () => lySet(name, { curve: lyCurveOf(div) });
  div.querySelector(".lyAlign").onchange = async ev => { const r = await lySet(name, { align: ev.target.checked }); if (r && ev.target.checked) lyMeasure(name); };
  div.querySelector(".lyMeasure").onclick = () => lyMeasure(name);
  div.querySelector(".lyRender").onclick = () => lyRender(name);
  lyViewAttach(div, name);   // P4-40e: the onset view (drag, pan, zoom, Play, Reset)
  lyShow(name);
  if (S.sid) lyRefresh(name);   // the stored layer and sampler (an agent command may have changed them since the project loaded)
}
