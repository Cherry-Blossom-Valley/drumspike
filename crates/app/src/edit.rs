//! P4-5 step 2: hit and timing edits that lived only in `static/app.js`, as commands an agent can call.
//!
//! * `events_edit {sid, name, ops}`: add, delete, move and set the velocity of single hits (the
//!   page's `addHitAt`, delete, `setHitTime` and the info line's velocity field), with the page's
//!   markers: an added hit has `added: true`, a moved detected hit keeps its time in `orig`, an
//!   edited hit is never cut. P4-14: `restore` takes the tom bleed mark (`"bleed": true`) off one
//!   hit, and an edited hit never carries it. The page still writes the whole list with `events_set`
//!   (its undo model works on its own copy); both end in the same document.
//! * `track_shift_set {sid, name, shift_samples}`: the Inspector's "Hitpoint shift": the hits (and
//!   their `orig`) move by the difference to the stored shift, the new value is stored in the
//!   params for the next analysis.
//! * `export_midi_stored`: `export_midi` with the page's values filled from the document: the
//!   track's note and its stored hits when the call gives none.

use drumspike_core::refine::{ReachStep, RefineRule, RefineWindow, refine_rule_peak};
use serde_json::{Map, Value, json};

use crate::Service;
use crate::error::AppError;
use crate::media::mix_mode;
use crate::registry::lock;

/// The edit commands of this card.
pub const EDIT_COMMANDS: [&str; 2] = ["events_edit", "track_shift_set"];

/// The range of the Inspector's shift field, in samples.
pub const SHIFT_LIMIT: i64 = 100_000;

fn num(op: &Map<String, Value>, key: &str) -> Result<f64, AppError> {
    op.get(key)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite())
        .ok_or_else(|| AppError::bad(format!("`{key}` must be a number")))
}

fn velocity(op: &Map<String, Value>, default: Option<i64>) -> Result<i64, AppError> {
    let v = match op.get("v") {
        None | Some(Value::Null) => default.ok_or_else(|| AppError::bad("`v` is missing"))?,
        Some(v) => v.as_i64().ok_or_else(|| AppError::bad("`v` must be an integer"))?,
    };
    if (1..=127).contains(&v) {
        Ok(v)
    } else {
        Err(AppError::bad(format!("`v` out of range 1 to 127: {v}")))
    }
}

/// The sample grid: `t` to the nearest sample of a track with the rate `sr`.
fn on_grid(t: f64, sr: Option<i64>) -> f64 {
    match sr {
        Some(sr) if sr > 0 => (t * sr as f64).round() / sr as f64,
        _ => t,
    }
}

/// The length of a track in seconds: `duration`, else `n_samples / sr`; `None` when unknown.
#[must_use]
pub fn track_end(tr: &drumspike_core::project::Track) -> Option<f64> {
    tr.duration
        .filter(|d| d.is_finite() && *d > 0.0)
        .or_else(|| match (tr.n_samples, tr.sr) {
            (Some(n), Some(sr)) if n > 0 && sr > 0 => Some(n as f64 / sr as f64),
            _ => None,
        })
}

/// A hand-edited hit: added, or moved away from its detected time.
fn is_manual(e: &Map<String, Value>) -> bool {
    if e.get("added").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    match (
        e.get("orig").and_then(Value::as_f64),
        e.get("t").and_then(Value::as_f64),
    ) {
        (Some(o), Some(t)) => (t - o).abs() > 1e-9,
        _ => false,
    }
}

/// A hit the engineer keeps as it is: a hand-edited hit, or one restored from the tom bleed mark (P4-14 fix round:
/// `edited: true`, the document's reserved edit flag). A re-pick or Re-analyze with `keep_edited` keeps it, and the
/// tom bleed rule never marks it.
pub fn is_kept(e: &Map<String, Value>) -> bool {
    is_manual(e) || e.get("edited").and_then(Value::as_bool) == Some(true)
}

fn hit_index(op: &Map<String, Value>, n: usize) -> Result<usize, AppError> {
    let i = op
        .get("index")
        .and_then(Value::as_u64)
        .ok_or_else(|| AppError::bad("`index` must be a non-negative integer"))?;
    usize::try_from(i)
        .ok()
        .filter(|&i| i < n)
        .ok_or_else(|| AppError::bad(format!("`index` {i} out of range: the track has {n} hits")))
}

/// Refuse a time after the end of the track (`end` in seconds; no limit when unknown).
fn check_end(t: f64, end: Option<f64>) -> Result<(), AppError> {
    match end {
        Some(e) if t > e => Err(AppError::bad(format!("`t` {t} is after the end of the track ({e} s)"))),
        _ => Ok(()),
    }
}

/// Apply `ops` to `events` (the list as `events_get` returns it). `index` in every op is a
/// position in that list at the start of the call. An added or moved hit must lie within the
/// track (0 to `end` seconds; P4-8 fix round 2: a hit far after the end made the tempo detection
/// track millions of beats). Returns the new sorted list and the counts
/// `(added, deleted, moved, velocity, restored)`.
/// R05: `was` on an op restores the exact field presence the hit had before the edit, so the page's
/// undo makes the document byte-identical (`undo(edit(doc)) == doc`): a bool key sets the flag
/// (`true`) or takes it off (`false`), `orig: <number>` sets it and `orig: null` takes it off.
/// Unknown keys are ignored; without `was` the op behaves as before (the agent API's default).
fn apply_was(e: &mut serde_json::Map<String, Value>, was: Option<&Value>) {
    let Some(w) = was.and_then(Value::as_object) else {
        return;
    };
    for (k, v) in w {
        match k.as_str() {
            "orig" => {
                if let Some(x) = v.as_f64().filter(|x| x.is_finite()) {
                    e.insert("orig".into(), json!(x));
                } else {
                    e.shift_remove("orig");
                }
            }
            "rise" => {
                if v.as_bool() == Some(true) {
                    e.insert("rise".into(), json!(true));
                } else {
                    e.shift_remove("rise");
                }
            }
            "edited" | "cut" | "bleed" | "filtered" | "learned" | "roll" | "flam" | "rescue" => {
                if v.as_bool() == Some(true) {
                    e.insert(k.clone(), json!(true));
                } else {
                    e.shift_remove(k);
                }
            }
            _ => {}
        }
    }
}

pub fn apply_ops(
    events: Vec<Value>,
    ops: &[Value],
    sr: Option<i64>,
    end: Option<f64>,
) -> Result<(Vec<Value>, [usize; 5]), AppError> {
    let n = events.len();
    let mut list: Vec<Option<Value>> = events.into_iter().map(Some).collect();
    let mut added = Vec::new();
    let mut counts = [0usize; 5];
    for op in ops {
        let o = op
            .as_object()
            .ok_or_else(|| AppError::bad("every op must be an object"))?;
        let kind = o.get("op").and_then(Value::as_str).unwrap_or("");
        match kind {
            "add" => {
                let t = on_grid(num(o, "t")?, sr);
                if t < 0.0 {
                    return Err(AppError::bad("`t` must not be negative"));
                }
                check_end(t, end)?;
                let mut hit = json!({"t": t, "v": velocity(o, Some(100))?});
                // P4-38b: K on a range or the playhead with no hit adds one flagged learned
                if o.get("learned").and_then(Value::as_bool) == Some(true) {
                    hit["learned"] = json!(true);
                }
                // `added: false` puts a hit back as a detected one (the page's undo of a delete), with its `orig`
                if o.get("added").and_then(Value::as_bool) != Some(false) {
                    hit["added"] = json!(true);
                } else if let Some(orig) = o.get("orig").and_then(Value::as_f64).filter(|x| x.is_finite()) {
                    hit["orig"] = json!(orig);
                }
                // P4-40a: `keep` {..} gives back the other fields the deleted hit had (`rise`, `rise_db`, ...): the undo of a
                // delete restores the hit as it was, so the project can be the saved one again. #154: the stored `id`
                // comes back too — the undo of a delete used to lose it, so the state was not the one before the step.
                if let Some(keep) = o.get("keep").and_then(Value::as_object) {
                    for (k, v) in keep {
                        if !["t", "v", "added", "orig"].contains(&k.as_str()) && hit.get(k).is_none() {
                            hit[k] = v.clone();
                        }
                    }
                }
                added.push(hit);
                counts[0] += 1;
            }
            "delete" | "move" | "velocity" | "restore" => {
                let i = hit_index(o, n)?;
                let slot = list[i]
                    .as_mut()
                    .ok_or_else(|| AppError::bad(format!("hit {i} was deleted by an earlier op")))?;
                if kind == "delete" {
                    list[i] = None;
                    counts[1] += 1;
                    continue;
                }
                let e = slot
                    .as_object_mut()
                    .ok_or_else(|| AppError::bad("every event must be an object"))?;
                if kind == "restore" {
                    // P4-14: the hit goes back into the output (the tom bleed rule had marked it); `edited` keeps it
                    // there through a re-pick or Re-analyze with keep_edited
                    let bleed = e.shift_remove("bleed").is_some();
                    if e.shift_remove("filtered").is_some() || bleed {
                        counts[4] += 1;
                    }
                    e.insert("edited".into(), Value::Bool(true));
                    apply_was(e, o.get("was")); // R05: the undo of a restore brings the marks and the edited flag back as they were
                } else if kind == "move" {
                    let to = on_grid(num(o, "t")?, sr).max(0.0);
                    check_end(to, end)?;
                    let from = e.get("t").and_then(Value::as_f64).unwrap_or(to);
                    if to != from {
                        if e.get("added").and_then(Value::as_bool) != Some(true) && !e.contains_key("orig") {
                            e.insert("orig".into(), json!(from));
                        }
                        e.insert("t".into(), json!(to));
                        apply_was(e, o.get("was"));
                        counts[2] += 1;
                    }
                } else {
                    e.insert("v".into(), json!(velocity(o, None)?));
                    // P4-40f: a hand velocity is an edit (`edited: true`, the document's reserved flag): it keeps the manual
                    // colour and survives Recompute Velocities (velocities_recompute keeps its `v`) and a re-pick
                    // (merge_edited) through `is_kept`. Without the flag the op stays as it was (the agent API's default).
                    if o.get("edited").and_then(Value::as_bool) == Some(true) {
                        e.insert("edited".into(), Value::Bool(true));
                    }
                    apply_was(e, o.get("was"));
                    counts[3] += 1;
                }
            }
            other => {
                return Err(AppError::bad(format!(
                    "unknown op `{other}`: use add, delete, move, velocity or restore"
                )));
            }
        }
    }
    let mut out: Vec<Value> = list.into_iter().flatten().chain(added).collect();
    out.sort_by(|a, b| {
        let t = |e: &Value| e.get("t").and_then(Value::as_f64).unwrap_or(0.0);
        t(a).total_cmp(&t(b))
    });
    for e in out.iter_mut().filter_map(Value::as_object_mut) {
        if is_manual(e) {
            e.remove("cut"); // an edited hit is never cut (the page's eventsBody)
        }
        if is_kept(e) {
            e.remove("bleed"); // P4-14: an edited or restored hit is never marked by the tom bleed rule
            e.remove("filtered"); // P4-6c: a hit the old Learn filter had taken out (its mark leaves)
        }
    }
    Ok((out, counts))
}

/// #144: the snap of an added hit looks for an attack this far around the click (seconds) ...
pub const SNAP_S: f64 = 0.010;
/// ... and takes it only when the envelope rises at least this much there (dB): a decaying tail or noise does not snap.
pub const SNAP_RISE_DB: f64 = 6.0;

/// #144: the onset of the attack near `click` (seconds) in the track's audio `x` at `sr`, found as the analysis finds
/// it: the refinement `rule` (back-track margin `peak_db`) with `click` as the coarse time, its search window
/// +-[`SNAP_S`], clipped at the midpoints to `neighbors` (the track's other hits). `None` when no attack lies within
/// [`SNAP_S`] of the click.
#[must_use]
pub fn snap_onset(x: &[f32], sr: u32, click: f64, neighbors: &[f64], rule: RefineRule, peak_db: f64) -> Option<f64> {
    let ms = SNAP_S * 1000.0;
    let win = RefineWindow { lo_ms: -ms, hi_ms: ms };
    let r = refine_rule_peak(x, click, sr, neighbors, win, rule, peak_db);
    // the refinement gives the coarse time back where it finds no attack (a silent or flat window)
    let found = r.t != click
        && r.step != ReachStep::Unresolved
        && r.rise.is_some_and(|d| d >= SNAP_RISE_DB)
        && (r.t - click).abs() <= SNAP_S + 0.5 / f64::from(sr.max(1));
    found.then_some(r.t)
}

/// Two hits closer than this (seconds) are the same hit: an edited hit wins over a detected one.
pub const MERGE_WINDOW: f64 = 0.020;

/// P4-5 step 2b: the picked hits plus the edited hits of `current` (the page's `hgMerge` after a
/// re-pick and `mergeKept` after a Re-analyze): a picked hit within [`MERGE_WINDOW`] of an edited
/// hit is dropped, the edited hits stay as they are, the list is sorted by time (the picks first
/// on a tie). An edited hit is one added by hand, moved away from its detected time, or (P4-14) restored from the
/// tom bleed mark ([`is_kept`]).
#[must_use]
pub fn merge_edited(picked: Vec<Value>, current: &[Value]) -> Vec<Value> {
    let t = |e: &Value| e.get("t").and_then(Value::as_f64).unwrap_or(0.0);
    let edited: Vec<&Value> = current.iter().filter(|e| e.as_object().is_some_and(is_kept)).collect();
    let mut out: Vec<Value> = picked
        .into_iter()
        .filter(|p| !edited.iter().any(|m| (t(m) - t(p)).abs() < MERGE_WINDOW))
        .collect();
    out.extend(edited.into_iter().cloned());
    out.sort_by(|a, b| t(a).total_cmp(&t(b)));
    out
}

impl Service {
    /// `events_edit {sid, name, ops}`: change single hits. An op is one of
    /// `{op:"add", t, v?, added?, orig?, snap?}` (velocity 100; `added: false` with `orig` puts back a detected or moved hit; `keep` {..} gives back its other fields;
    /// #144: `snap: true` puts the hit on the attack within [`SNAP_S`] of `t` with the velocity of the audio there), `{op:"delete", index}`, `{op:"move", index, t}` and
    /// `{op:"velocity", index, v, edited?: true}` (a hand velocity marks the hit `edited`, P4-40f) and `{op:"restore", index}` (P4-14: the hit loses its tom bleed mark and goes back
    /// into the output). R05: `move`, `velocity` and `restore` ops may carry `was` {..}, the field
    /// presence the hit had before the edit (bool flags, `orig` as a number or null); the page's
    /// undo sends it, so the document comes back byte-identical; `index` is the position in the list `events_get` returned
    /// before the call, `t` seconds (put on the sample grid). The ops apply as a whole: a bad op
    /// changes nothing. A follower is refused (409), as in `events_set`. Returns
    /// `{saved, added, deleted, moved, velocity, restored, followers?, snaps?}`; `snaps` holds one
    /// `{click, t, v, snapped}` per add with `snap: true`.
    pub fn events_edit(&self, sid: &str, name: &str, ops: &[Value]) -> Result<Value, AppError> {
        self.events_edit_as(sid, name, ops, true)
    }

    /// [`Service::events_edit`]; `by_hand: false` for an internal edit (P4-38e: the Learn sweep), whose deletions do
    /// not go in `learn.dropped`.
    pub(crate) fn events_edit_as(
        &self,
        sid: &str,
        name: &str,
        ops: &[Value],
        by_hand: bool,
    ) -> Result<Value, AppError> {
        if ops.is_empty() {
            return Err(AppError::bad("`ops` must not be empty"));
        }
        let op = self.inner.registry.get_writable(sid)?;
        // R07: one store guard from the read of the list to the commit. An edit that overlaps
        // another write of the project (a second edit, a re-pick store, a shift) is refused with
        // a retryable 409 instead of silently committing over it. The guard is not the project
        // lock: the decode of a snap (`snap_ops`) still runs with the project lock free.
        let _guard = self.begin_write(sid)?;
        let (sr, end) = {
            let g = lock(&op);
            crate::follow::check_own(&g, name)?;
            let tr = g.track(name)?;
            (tr.sr, track_end(tr))
        };
        let current = self.track_events(sid, name)?;
        let (ops, snaps) = self.snap_ops(sid, name, ops, &current)?;
        let (events, c) = apply_ops(current, &ops, sr, end)?;
        let events = Value::Array(events);
        // P4-38e: a hand edit notes the Learn hits it deleted (`learn.dropped`); the Learn sweep does not
        let mut out = if by_hand {
            self.events_set_by_hand_with(&op, name, &events)?
        } else {
            self.events_set_with(&op, name, &events)?
        };
        out["added"] = c[0].into();
        out["deleted"] = c[1].into();
        out["moved"] = c[2].into();
        out["velocity"] = c[3].into();
        out["restored"] = c[4].into();
        if !snaps.is_empty() {
            out["snaps"] = Value::Array(snaps);
        }
        Ok(out)
    }

    /// #144: `ops` with every `{op:"add", snap: true}` put on the attack near its `t` ([`snap_onset`] with the track's
    /// refinement rule, Hitpoint shift and `current` hits as neighbours); a snapped add without `v` gets the velocity
    /// that `velocities_recompute` gives at its time. An add with no attack within [`SNAP_S`] keeps its `t` and `v`.
    /// Also returns one `{click, t, v, snapped}` per snap op, in op order (empty when no op asks for a snap).
    fn snap_ops(
        &self,
        sid: &str,
        name: &str,
        ops: &[Value],
        current: &[Value],
    ) -> Result<(Vec<Value>, Vec<Value>), AppError> {
        let wants = |o: &Value| {
            o.get("op").and_then(Value::as_str) == Some("add") && o.get("snap").and_then(Value::as_bool) == Some(true)
        };
        if !ops.iter().any(wants) {
            return Ok((ops.to_vec(), Vec::new()));
        }
        let op = self.inner.registry.get(sid)?;
        crate::registry::resolve_shared(&op);
        let (source, mix, sr, rule, peak_db, shift) = {
            let g = lock(&op);
            let tr = g.track(name)?;
            let det = tr.extra.get("detector").filter(|d| !d.is_null());
            let cfg = &self.inner.config;
            // an analysed track snaps with the rule of its analysis, a track never analysed with the rule a new one runs
            let rule = match det {
                Some(_) => crate::jobs::recorded_rule(cfg, det),
                None => crate::jobs::analysis_rule(cfg),
            };
            let shift = tr
                .params
                .as_ref()
                .and_then(|p| p.get("shift_samples"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .round();
            let peak_db = drumspike_core::refine::FM_PEAK_DB;
            (
                g.source(name)?,
                mix_mode(tr.mix_mode_or_default()),
                tr.sr,
                rule,
                peak_db,
                shift,
            )
        };
        let d = self.inner.media.decode_cached(&source, mix)?;
        // the hits carry the Hitpoint shift; the audio does not
        let dt = shift / f64::from(d.sr);
        let neighbors: Vec<f64> = current
            .iter()
            .filter_map(|e| e.get("t").and_then(Value::as_f64))
            .map(|t| t - dt)
            .collect();
        let mut out = Vec::with_capacity(ops.len());
        let mut snaps = Vec::new();
        for o in ops {
            if !wants(o) {
                out.push(o.clone());
                continue;
            }
            let mut o = o.clone();
            let click = on_grid(num(o.as_object().expect("an add op is an object"), "t")?, sr);
            let snapped =
                snap_onset(&d.samples, d.sr, click - dt, &neighbors, rule, peak_db).map(|t| on_grid(t + dt, sr));
            if let Some(t) = snapped {
                o["t"] = json!(t);
                if o.get("v").is_none_or(Value::is_null) {
                    let r = self.velocities_recompute(sid, name, Some(&[json!({"t": t})]))?;
                    o["v"] = r["events"][0]["v"].clone();
                }
            }
            o.as_object_mut().map(|m| m.remove("snap"));
            snaps.push(json!({
                "click": click,
                "t": o["t"],
                "v": o.get("v").cloned().filter(|v| !v.is_null()).unwrap_or(json!(100)),
                "snapped": snapped.is_some(),
            }));
            out.push(o);
        }
        Ok((out, snaps))
    }

    /// `track_shift_set {sid, name, shift_samples}`: the Inspector's Hitpoint shift (positive is
    /// later). Every hit and every `orig` moves by the difference to the stored shift
    /// (`params.shift_samples`, 0 when none) and the new value is stored for the next analysis.
    /// 400 outside +-[`SHIFT_LIMIT`] or for a track without a sample rate; 409 for a follower and
    /// while an analysis or a re-pick store runs. Returns `{shift_samples, delta, moved}`.
    pub fn track_shift_set(&self, sid: &str, name: &str, shift_samples: i64) -> Result<Value, AppError> {
        if shift_samples.unsigned_abs() > SHIFT_LIMIT as u64 {
            return Err(AppError::bad(format!("shift_samples out of range +-{SHIFT_LIMIT}")));
        }
        let op = self.inner.registry.get_writable(sid)?;
        let _guard = self.begin_write(sid)?;
        let (sr, old) = {
            let g = lock(&op);
            crate::follow::check_own(&g, name)?;
            let tr = g.track(name)?;
            let sr = tr
                .sr
                .filter(|&s| s > 0)
                .ok_or_else(|| AppError::bad("the track has no sample rate"))?;
            let old = tr
                .params
                .as_ref()
                .and_then(|p| p.get("shift_samples"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .round() as i64;
            (sr as f64, old)
        };
        let delta = shift_samples - old;
        let d = delta as f64 / sr;
        let mut events = self.track_events(sid, name)?;
        for e in events.iter_mut().filter_map(Value::as_object_mut) {
            for key in ["t", "orig"] {
                if let Some(t) = e.get(key).and_then(Value::as_f64) {
                    e.insert(key.into(), json!(((t + d) * sr).round() / sr));
                }
            }
        }
        let moved = if delta == 0 { 0 } else { events.len() };
        {
            let mut g = lock(&op);
            let tr = g.track_mut(name)?;
            tr.params
                .get_or_insert_with(Map::new)
                .insert("shift_samples".into(), json!(shift_samples));
            g.mark_dirty();
        }
        if delta != 0 {
            self.events_set_with(&op, name, &Value::Array(events))?;
        }
        Ok(json!({"shift_samples": shift_samples, "delta": delta, "moved": moved}))
    }

    /// `export_midi` with the values the page passes taken from the document when the call has
    /// none: `note` is the track's note (36 without one), `events` the track's stored hits,
    /// `ppq` 24000, `tempo` 120. The Tauri command and the test bridge call this.
    pub fn export_midi_stored(
        &self,
        sid: &str,
        name: &str,
        note: Option<i64>,
        events: Option<&[Value]>,
        ppq: Option<i64>,
        tempo: Option<f64>,
    ) -> Result<Value, AppError> {
        self.export_midi_stored_to(sid, name, note, events, ppq, tempo, None)
    }

    /// P4-30: [`Service::export_midi_stored`] into the folder `dest`; `None` is `Exports/`.
    #[allow(clippy::too_many_arguments)]
    pub fn export_midi_stored_to(
        &self,
        sid: &str,
        name: &str,
        note: Option<i64>,
        events: Option<&[Value]>,
        ppq: Option<i64>,
        tempo: Option<f64>,
        dest: Option<&str>,
    ) -> Result<Value, AppError> {
        self.export_midi_stored_with(sid, name, note, events, ppq, tempo, dest, true)
    }

    /// P4-7c: [`Service::export_midi_stored_to`] with the choice to compensate the sampler offset.
    #[allow(clippy::too_many_arguments)]
    pub fn export_midi_stored_with(
        &self,
        sid: &str,
        name: &str,
        note: Option<i64>,
        events: Option<&[Value]>,
        ppq: Option<i64>,
        tempo: Option<f64>,
        dest: Option<&str>,
        compensate: bool,
    ) -> Result<Value, AppError> {
        let note = if let Some(n) = note {
            n
        } else {
            let op = self.inner.registry.get_writable(sid)?;
            let g = lock(&op);
            g.track(name)?.note.unwrap_or(36)
        };
        let stored;
        let events = if let Some(e) = events {
            e
        } else {
            stored = self.track_events(sid, name)?;
            &stored[..]
        };
        self.export_midi_with(
            sid,
            name,
            note,
            events,
            ppq.unwrap_or(24000),
            tempo.unwrap_or(120.0),
            dest,
            compensate,
        )
    }
}

/// Run a command of this card for the bridge; `None` when `cmd` is not one of them.
#[must_use]
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if !EDIT_COMMANDS.contains(&cmd) {
        return None;
    }
    let s = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{k}` (string)")))
    };
    Some((|| {
        if cmd == "events_edit" {
            let ops = args
                .get("ops")
                .and_then(Value::as_array)
                .ok_or_else(|| AppError::bad("missing or wrong argument `ops` (list)"))?;
            svc.events_edit(s("sid")?, s("name")?, ops)
        } else {
            let n = args
                .get("shift_samples")
                .and_then(Value::as_i64)
                .ok_or_else(|| AppError::bad("missing or wrong argument `shift_samples` (integer)"))?;
            svc.track_shift_set(s("sid")?, s("name")?, n)
        }
    })())
}

#[cfg(test)]
mod tests;
