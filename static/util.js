/* Drum Spike v2 — util (refactor plan step 5.1, R9 A1)
   The small page helpers with no state: $, fmt, dScale, the note names (NOTE_NAMES,
   nameFromNote, noteFromName) and the in-page confirm askConfirm. Moved verbatim from
   app.js. Plain script (no build step), loaded right after state.js, so every later
   script can use them at load time. The names stay top-level bindings (page globals). */
"use strict";

const $ = s => document.querySelector(s);
const fmt = (t, d=3) => t.toFixed(d);
const dScale = m => m === 1 ? 40 : 1;   // wheel line-mode deltas → px

/* ---------------- note names (octave floor(n/12)-2 → kick 36 = C1) ---------------- */
const NOTE_NAMES = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];
function nameFromNote(n){ return `${NOTE_NAMES[n%12]}${Math.floor(n/12)-2}`; }
function noteFromName(s){
  const m = /^([A-Ga-g])(#|b)?(-?\d+)$/.exec(s.trim());
  if(!m) return null;
  const base = {C:0,D:2,E:4,F:5,G:7,A:9,B:11}[m[1].toUpperCase()];
  return base + (m[2]==="#"?1:m[2]==="b"?-1:0) + 12*(parseInt(m[3])+2);
}
/* ---------------- in-page confirm (replaces window.confirm for project prompts) ---------------- */
function askConfirm(o) {
  return new Promise(resolve => {
    const m = $("#confirmModal"); $("#confirmTitle").textContent = o.title || "Confirm"; $("#confirmMsg").textContent = o.msg || "";
    const acts = $("#confirmActions"); acts.innerHTML = "";
    for (const b of o.buttons) {
      const btn = document.createElement("button"); btn.className = "btn" + (b.cls ? " " + b.cls : ""); btn.textContent = b.label; btn.dataset.act = b.act;
      btn.onclick = () => { m.classList.add("hidden"); resolve(b.act); };
      acts.appendChild(btn);
    }
    m.classList.remove("hidden");
  });
}
