//! #343 Training Mode: a correction log of the owner's hit edits, with the audio context of each.
//!
//! The mode is a page setting (off by default). With it on, the page sends `training: true` with every
//! `events_edit` and every Learn mark (`track_learn_mark`, kinds `this` and `not`); the command appends one JSON
//! line per hit affected to `<home>/training/<project id>.jsonl` after the edit applied (`apply_ops` stays pure). The
//! file is never part of the project. A record is self-contained: it carries the samples around the hit, so the
//! measurement kit reads it without the session. #353: with the view of the request the record carries it too, and a
//! `move` or `add` record says its `precision` (`sample` at a pixel per sample or more, `ms` at a coarser zoom).
//! #363: a `move`, `add`, `delete` or `confirm` record carries `placement` `{edge, line_noise, line_slope,
//! onset_spread}`, the measures of `placement_diag` at its `from` (its `to` for an add), computed on demand.
//! #354: a `confirm` record is the editor's "reviewed, right where it is" — the op applies nothing to the document,
//! the record says `from` = `to` = the stored position and carries one audio window.
//!
//! * `training_status {sid, on?}`: `{on, records, path}`; `on` (a bool) sets the state the service reports for the
//!   project (the page sends it when the mode changes), without it the state is read.
//!
//! A failed log never fails the edit: the answer of the command gets `training_error` instead of `training`.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use drumspike_core::refine::placement_diag;

use crate::Service;
use crate::error::AppError;
use crate::media::mix_mode;
use crate::registry::lock;

/// The commands of this module.
pub const TRAINING_COMMANDS: [&str; 1] = ["training_status"];

/// The audio window of a record: this long before the position (milliseconds) ...
pub const BEFORE_MS: f64 = 40.0;
/// ... and this long after it.
pub const AFTER_MS: f64 = 10.0;

/// A Learn example belongs to the stored hit within this many seconds of it.
pub const LEARN_HIT_S: f64 = 0.005;

/// The state per open project: whether the mode is on and how many records the file holds.
#[derive(Default)]
pub(crate) struct State {
    per_sid: Mutex<HashMap<String, Entry>>,
}

#[derive(Default, Clone, Copy)]
struct Entry {
    on: bool,
    records: Option<usize>,
}

/// One affected hit of a command, before the context is added.
pub(crate) struct Example {
    pub op: &'static str,
    /// The event as stored before the op (`None` for an add or a Learn example without a hit).
    pub event: Option<Value>,
    /// Seconds in the coordinates of the stored hits (the Hitpoint shift included).
    pub from: Option<f64>,
    pub to: Option<f64>,
    /// The confidence of the hit as rated at that moment (`None`: not rated).
    pub confidence: Option<Value>,
}

/// The samples of `x` from `n_before` before `pos` to `n_after` after it, as floats rounded to 6 decimals; positions
/// outside the audio read 0, so the length is always `n_before + n_after`.
#[must_use]
pub fn window(x: &[f32], pos: i64, n_before: usize, n_after: usize) -> Vec<Value> {
    let start = pos - n_before as i64;
    (0..(n_before + n_after) as i64)
        .map(|k| {
            let v = usize::try_from(start + k)
                .ok()
                .and_then(|i| x.get(i))
                .copied()
                .unwrap_or(0.0);
            json!((f64::from(v) * 1e6).round() / 1e6)
        })
        .collect()
}

/// The project's id in the training folder: the first 12 hex digits of the SHA-256 of its creation time (else of its
/// folder name), so a rename keeps the file.
fn project_key(doc_created: Option<String>, dir_name: &str) -> String {
    let seed = doc_created.unwrap_or_else(|| dir_name.to_string());
    let h = Sha256::digest(seed.as_bytes());
    h.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// The precision of a placement from the view of the request (#353): `sample` when a sample is its own pixel or
/// wider (`px_per_sample >= 1`), else `ms`; null without a numeric `px_per_sample` (the view was not sent).
fn precision_of(view: &Value) -> Value {
    match view.get("px_per_sample").and_then(Value::as_f64) {
        Some(px) if px >= 1.0 => json!("sample"),
        Some(_) => json!("ms"),
        None => Value::Null,
    }
}

fn count_lines(path: &std::path::Path) -> usize {
    std::fs::read_to_string(path).map_or(0, |s| s.lines().count())
}

impl Service {
    /// `<home>/training/<project id>.jsonl` of the open project `sid`, and the id.
    fn training_path(&self, sid: &str) -> Result<(PathBuf, String), AppError> {
        let op = self.inner.registry.get(sid)?;
        let (created, dir_name) = {
            let g = lock(&op);
            let created = g.project.doc.created.as_ref().map(ToString::to_string);
            (
                created,
                g.project
                    .dir
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            )
        };
        let id = project_key(created, &dir_name);
        let home = self
            .home()
            .ok_or_else(|| AppError::internal("no home folder for the training log"))?;
        Ok((home.join("training").join(format!("{id}.jsonl")), id))
    }

    /// `training_status {sid, on?}`: `{on, records, path}` (see the module docs).
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] for an unknown `sid`; an internal error when the service has no home folder.
    pub fn training_status(&self, sid: &str, on: Option<bool>) -> Result<Value, AppError> {
        let (path, _) = self.training_path(sid)?;
        let mut map = lock(&self.inner.training.per_sid);
        let e = map.entry(sid.to_string()).or_default();
        if let Some(on) = on {
            e.on = on;
        }
        let records = *e.records.get_or_insert_with(|| count_lines(&path));
        Ok(json!({"on": e.on, "records": records, "path": path.display().to_string()}))
    }

    /// The records of `examples` on the track `name` (the audio and the track's facts are read here), appended to the
    /// project's file. `view` is the view object of the request (#353, written as sent): a `move` or `add` record gets
    /// the `precision` derived from it, a `delete` record carries the view without a precision. Returns the number
    /// written. `by` (#389) is who made the edit (`"person"` or `"agent"`); a record carries it when `by` is `Some`
    /// (the Learn marks of the sweep carry none, an older log has no field).
    pub(crate) fn training_log(
        &self,
        sid: &str,
        name: &str,
        examples: &[Example],
        view: Option<&Value>,
        by: Option<&str>,
    ) -> Result<usize, AppError> {
        if examples.is_empty() {
            return Ok(0);
        }
        let (path, id) = self.training_path(sid)?;
        let op = self.inner.registry.get(sid)?;
        let (source, mix, role, shift, rule, project) = {
            let g = lock(&op);
            let tr = g.track(name)?;
            let det = tr.extra.get("detector").filter(|d| !d.is_null());
            let rule = crate::jobs::recorded_rule(&self.inner.config, det).version();
            let shift = tr
                .params
                .as_ref()
                .and_then(|p| p.get("shift_samples"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .round();
            let detect = det.and_then(|d| d.get("detector")).cloned().unwrap_or(Value::Null);
            (
                g.source(name)?,
                mix_mode(tr.mix_mode_or_default()),
                tr.role.clone(),
                shift,
                (rule, detect),
                g.project.name(),
            )
        };
        let d = self.inner.media.decode_cached(&source, mix)?;
        let sr = f64::from(d.sr);
        let n_before = (BEFORE_MS / 1000.0 * sr).round() as usize;
        let n_after = (AFTER_MS / 1000.0 * sr).round() as usize;
        let smp = |t: f64| (t * sr).round() as i64;
        let ts = {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64());
            (secs * 1000.0).round() / 1000.0
        };
        let mut buf = String::new();
        for ex in examples {
            let audio = |t: f64| Value::Array(window(&d.samples, smp(t - shift / sr), n_before, n_after));
            let mut a = Map::new();
            if let Some(f) = ex.from {
                a.insert("at_from".into(), audio(f));
            }
            if let Some(t) = ex.to.filter(|t| ex.from.is_none_or(|f| smp(*t) != smp(f))) {
                a.insert("at_to".into(), audio(t));
            }
            let mut rec = json!({
                "ts": ts,
                "app": drumspike_core::VERSION,
                "detect": rule.1,
                "refine": rule.0,
                "project": {"id": id, "name": project},
                "track": name,
                "role": role,
                "sr": d.sr,
                "shift_samples": shift,
                "op": ex.op,
                "from": ex.from.map(smp),
                "to": ex.to.map(smp),
                "event": ex.event,
                "confidence": ex.confidence,
                "window": {"before": n_before, "after": n_after},
                "audio": a,
            });
            // #363: the line measures at the position the record's audio is read at, so a consumer can tell a clear
            // stroke from a wobbly one; a Learn mark has no placement to judge
            if !ex.op.starts_with("learn")
                && let Some(t) = ex.from.or(ex.to)
            {
                let pd = placement_diag(&d.samples, d.sr, smp(t - shift / sr));
                let spread = ex
                    .event
                    .as_ref()
                    .and_then(|e| e.get("timing_confidence"))
                    .and_then(Value::as_f64);
                rec["placement"] = json!({
                    "edge": pd.edge,
                    "line_noise": pd.line_noise,
                    "line_slope": pd.line_slope,
                    "onset_spread": spread,
                });
            }
            // #389: who made the edit, written when the caller says so
            if let Some(b) = by {
                rec["by"] = json!(b);
            }
            // #353: the view of the request as sent; a move or add says how precisely the editor saw the hit
            if let Some(v) = view {
                rec["view"] = v.clone();
                if ex.op != "delete" {
                    rec["precision"] = precision_of(v);
                }
            } else if ex.op != "delete" {
                rec["precision"] = Value::Null;
            }
            buf.push_str(&serde_json::to_string(&rec).map_err(|e| AppError::internal(e.to_string()))?);
            buf.push('\n');
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::internal(format!("{}: {e}", parent.display())))?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| AppError::internal(format!("{}: {e}", path.display())))?;
        f.write_all(buf.as_bytes())
            .map_err(|e| AppError::internal(format!("{}: {e}", path.display())))?;
        let mut map = lock(&self.inner.training.per_sid);
        let e = map.entry(sid.to_string()).or_default();
        let before = *e
            .records
            .get_or_insert_with(|| count_lines(&path).saturating_sub(examples.len()));
        e.records = Some(before + examples.len());
        Ok(examples.len())
    }

    /// The examples of the `add`, `delete` and `move` ops of an `events_edit` call: `current` is the list the ops'
    /// indexes count in (before the call), `sr` the track's rate for the sample grid. A move that leaves the hit where
    /// it is makes none; velocity, restore, toggle and confirm ops make none (the confirm ops have their own
    /// builder, [`Service::training_confirm_examples`]).
    pub(crate) fn training_examples(
        &self,
        sid: &str,
        name: &str,
        current: &[Value],
        ops: &[Value],
        sr: Option<i64>,
    ) -> Vec<Example> {
        struct Pending {
            op: &'static str,
            index: Option<usize>,
            from: Option<f64>,
            to: Option<f64>,
        }
        let t_of = |e: &Value| e.get("t").and_then(Value::as_f64);
        let mut pending = Vec::new();
        let mut added: Vec<Value> = Vec::new();
        for o in ops {
            let kind = o.get("op").and_then(Value::as_str).unwrap_or("");
            let index = o
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|i| usize::try_from(i).ok());
            let at = |i: Option<usize>| i.and_then(|i| current.get(i));
            let was = |e: &Value| e.get("orig").and_then(Value::as_f64).or_else(|| t_of(e));
            match kind {
                "add" => {
                    let Some(t) = o.get("t").and_then(Value::as_f64) else {
                        continue;
                    };
                    let t = crate::edit::on_grid(t, sr);
                    added.push(json!({"t": t, "v": o.get("v").and_then(Value::as_i64).unwrap_or(100)}));
                    pending.push(Pending {
                        op: "add",
                        index: None,
                        from: None,
                        to: Some(t),
                    });
                }
                "delete" => {
                    let Some(e) = at(index) else { continue };
                    pending.push(Pending {
                        op: "delete",
                        index,
                        from: was(e),
                        to: None,
                    });
                }
                "move" => {
                    let (Some(e), Some(t)) = (at(index), o.get("t").and_then(Value::as_f64)) else {
                        continue;
                    };
                    let to = crate::edit::on_grid(t, sr).max(0.0);
                    if t_of(e) == Some(to) {
                        continue;
                    }
                    pending.push(Pending {
                        op: "move",
                        index,
                        from: was(e),
                        to: Some(to),
                    });
                }
                _ => {}
            }
        }
        if pending.is_empty() {
            return Vec::new();
        }
        let all: Vec<Value> = current.iter().cloned().chain(added.iter().cloned()).collect();
        let rates = self.training_rate(sid, name, &all);
        let mut next_added = 0;
        pending
            .into_iter()
            .map(|p| {
                let (event, confidence) = if let Some(i) = p.index {
                    (current.get(i).cloned(), rates.get(i).cloned().flatten())
                } else {
                    next_added += 1;
                    (None, rates.get(current.len() + next_added - 1).cloned().flatten())
                };
                Example {
                    op: p.op,
                    event,
                    from: p.from,
                    to: p.to,
                    confidence,
                }
            })
            .collect()
    }

    /// The examples of the `confirm` ops of an `events_edit` call (#354): one per op, the hit and its confidence as
    /// stored, `from` = `to` = the hit's stored position (the record's one audio window reads there). The indexes
    /// count in `current` (the list before the call); `events_edit_as` has validated them.
    pub(crate) fn training_confirm_examples(
        &self,
        sid: &str,
        name: &str,
        current: &[Value],
        ops: &[Value],
    ) -> Vec<Example> {
        let rates = self.training_rate(sid, name, current);
        ops.iter()
            .filter_map(|o| {
                let i = o
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|i| usize::try_from(i).ok())?;
                let e = current.get(i)?;
                let t = e.get("t").and_then(Value::as_f64);
                Some(Example {
                    op: "confirm",
                    event: Some(e.clone()),
                    from: t,
                    to: t,
                    confidence: rates.get(i).cloned().flatten(),
                })
            })
            .collect()
    }

    /// One example per time of `at` (audio seconds) of a Learn mark of `kind` (`this` or `not`) on `name`: the hit
    /// stored within [`LEARN_HIT_S`] of it (`events` before the mark, `shift_s` the Hitpoint shift), when there is one.
    pub(crate) fn training_learn_examples(
        &self,
        sid: &str,
        name: &str,
        kind: &str,
        at: &[f64],
        events: &[Value],
        shift_s: f64,
    ) -> Vec<Example> {
        let rates = self.training_rate(sid, name, events);
        at.iter()
            .map(|&a| {
                let near = events
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| e.get("t").and_then(Value::as_f64).map(|t| (i, (t - shift_s - a).abs())))
                    .filter(|&(_, d)| d <= LEARN_HIT_S)
                    .min_by(|x, y| x.1.total_cmp(&y.1))
                    .map(|(i, _)| i);
                Example {
                    op: if kind == "this" { "learn_this" } else { "learn_not" },
                    event: near.map(|i| events[i].clone()),
                    from: Some(a + shift_s),
                    to: None,
                    confidence: near.and_then(|i| rates.get(i).cloned().flatten()),
                }
            })
            .collect()
    }

    /// `rate` of one stored or new hit as `track_confidence` gives it, `None` when it cannot be rated.
    pub(crate) fn training_rate(&self, sid: &str, name: &str, events: &[Value]) -> Vec<Option<Value>> {
        match self.track_confidence(sid, name, Some(events), false) {
            Ok(c) => {
                let list = c["confidence"].as_array().cloned().unwrap_or_default();
                (0..events.len()).map(|i| list.get(i).cloned()).collect()
            }
            Err(_) => vec![None; events.len()],
        }
    }

    /// Put the outcome of a log into `out`: `training: {records}` or `training_error`.
    pub(crate) fn training_note(out: &mut Value, r: Result<usize, AppError>) {
        match r {
            Ok(n) => out["training"] = json!({"records": n}),
            Err(e) => out["training_error"] = json!(e.message),
        }
    }
}

/// R3: the handlers of [`TRAINING_COMMANDS`] for [`crate::call::run`].
pub(crate) const HANDLERS: [crate::call::Route; 1] = crate::call::routes(TRAINING_COMMANDS, |s, _, cmd, args| {
    crate::call::json_reply(cmd, dispatch(s, cmd, args))
});

/// Run a command of this module; `None` when `cmd` is not one of them.
#[must_use]
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if !TRAINING_COMMANDS.contains(&cmd) {
        return None;
    }
    Some((|| {
        let sid = args
            .get("sid")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::bad("missing or wrong argument `sid` (string)"))?;
        svc.training_status(sid, args.get("on").and_then(Value::as_bool))
    })())
}

#[cfg(test)]
mod tests;
