//! #265: `reanalyze_selection` — re-analyze only the hits of a selected range on one track
//! (concept `docs/concepts/reanalyze-selection.md`, option B plus the pre-run line of option D).
//!
//! The shared rules 1 to 7 of the concept, as implemented here:
//!
//! 1. The detector runs on the track's audio with a pad of 100 ms on both sides of the range, so
//!    the spectral frames and the refine-to-attack walk see their context; only hits whose
//!    refined time lies inside `[t0, t1]` are taken.
//! 2. Hits of the track inside the range are replaced, hits outside are not touched at all
//!    (same times, ids, velocities — the stored values are carried over unchanged).
//! 3. A new hit is dropped when it is closer than the minimum gap to the nearest untouched hit
//!    outside the range; a hit outside is never moved.
//! 4. Edited and manual hits inside the range follow the Keep dialog as today (the page counts
//!    only the hits of the range); with `keep_edited` they stay and win within 20 ms over a
//!    picked one ([`crate::edit::merge_edited`]), without it they are replaced.
//! 5. The Learn marks of the track stay and are applied to the new hits inside the range
//!    (geometrically: a hit at a This example is `learned`, a hit at a Forget example is
//!    `filtered` when the filter is on; the score-based filter of a whole-track run is a
//!    full-file computation and applies on the next whole-track analysis). The followers of the
//!    track are refreshed (their hits outside the range are unchanged, so the derivation is the
//!    same as a range-limited one).
//! 6. The previous events go into the backup file (`Backups/<track>.events.prev.json`, as for a
//!    whole-track run); the page's undo puts the range back as one step "Re-analyze Selection".
//! 7. The answer carries the info line text: `Kick In: re-analyzed 0.900 to 2.100 s, 3 hits
//!    replaced by 4`.

use drumspike_core::project::Event;
use serde_json::{Value, json};

use crate::Service;
use crate::error::AppError;
use crate::media::mix_mode;
use crate::registry::lock;

/// The pad on both sides of the range (concept rule 1).
pub const PAD_S: f64 = 0.1;

/// How far (seconds) a Learn example reaches to a hit ([`crate::learn_filter::REACH`]).
const LEARN_REACH: f64 = 0.030;

impl Service {
    /// `reanalyze_selection {sid, name, t0, t1, keep_edited}`: detect again in the range only.
    /// The runs use the track's current settings (the Inspector values), like a whole-track run.
    /// A follower is refused (its hits are the source's), as in `events_set`.
    ///
    /// # Errors
    ///
    /// A 400 [`AppError`] for a reversed or negative range, bad params or a failed detection; a
    /// 404 for an unknown `sid` or track; a 409 when the project is open read-only, an analysis
    /// is queued or running, or another write holds the project.
    pub fn reanalyze_selection(
        &self,
        sid: &str,
        name: &str,
        t0: f64,
        t1: f64,
        keep_edited: bool,
    ) -> Result<Value, AppError> {
        if !(t0.is_finite() && t1.is_finite()) || t0 < 0.0 || t1 <= t0 {
            return Err(AppError::bad(format!(
                "the range must be 0 <= t0 < t1 in seconds (got {t0} to {t1})"
            )));
        }
        let op = self.inner.registry.get_writable(sid)?;
        // one writer at a time, like an analysis start: the store guard is held from the read of
        // the events to the commit (R07)
        let _guard = self.begin_write(sid)?;
        let (role, params, follows) = {
            let g = lock(&op);
            crate::follow::check_own(&g, name)?;
            let tr = g.track(name)?;
            (
                tr.role.clone().unwrap_or_else(|| "other".into()),
                tr.params.clone(),
                tr.follows.clone(),
            )
        };
        if follows.is_some() {
            return Err(AppError::conflict(format!(
                "{name} is a follower: its hits are the source's (Track > Own Detection first)"
            )));
        }
        let current = self.track_events(sid, name)?;
        let (inside_old, outside): (Vec<Value>, Vec<Value>) =
            current.iter().cloned().partition(|e| in_range(e, t0, t1));
        let edited_inside: Vec<Value> = inside_old
            .iter()
            .filter(|e| e.as_object().is_some_and(crate::edit::is_kept))
            .cloned()
            .collect();

        // the detection on the padded slice of the track's audio (the project lock is free for
        // the decode, as in the snap of an edit)
        crate::registry::resolve_shared(&op);
        let (source, mix, mut parsed, det) = {
            let g = lock(&op);
            let tr = g.track(name)?;
            if g.offline(name) {
                return Err(AppError::bad(format!(
                    "track is offline ({}): relink it first",
                    g.status_str(name)
                )));
            }
            let det = tr.extra.get("detector").filter(|d| !d.is_null()).cloned();
            (
                g.source(name)?,
                mix_mode(tr.mix_mode_or_default()),
                crate::jobs::parse_params(params.as_ref()).map_err(AppError::bad)?,
                det,
            )
        };
        let d = self.inner.media.decode_cached(&source, mix)?;
        let sr = f64::from(d.sr);
        let a = ((t0 - PAD_S).max(0.0) * sr).floor().max(0.0) as usize;
        let b = (((t1 + PAD_S) * sr).ceil() as usize).min(d.samples.len());
        let offset_s = a as f64 / sr;
        // the rule of the run: the recorded one of the track's analysis (a re-pick rule), else
        // the one a new analysis runs (P4-12)
        parsed.params.rule = crate::jobs::recorded_rule(&self.inner.config, det.as_ref());
        let (analysis, _record, _kept) = self.inner.detector.analyze_prepared_cands(
            &self.inner.config,
            &d.samples[a..b],
            d.sr,
            &role,
            &parsed,
            None,
        )?;
        let mut picked: Vec<Value> = analysis
            .events
            .iter()
            .filter(|h| {
                let t = h.t + offset_s;
                t >= t0 - 1e-9 && t <= t1 + 1e-9
            })
            .map(|h| {
                let mut e = json!({"t": h.t + offset_s, "v": h.v});
                if let Some(rise) = h.rise {
                    e["rise"] = json!(rise);
                }
                e
            })
            .collect();

        // rule 5: the Learn marks of the track apply to the new hits (times in audio time, the
        // marks are stored before the Hitpoint shift)
        let learn = {
            let g = lock(&op);
            g.track(name).ok().and_then(|tr| crate::learn::learn_of(&tr.extra))
        };
        if let Some(learn) = learn {
            let (shift_s, filter) = {
                let g = lock(&op);
                let shift_s = g.track(name).map_or(0.0, crate::learn::shift_s_of);
                (shift_s, crate::learn::filter_on(&learn))
            };
            let (this, not) = crate::learn_score::example_times(&learn);
            for e in picked.iter_mut().filter_map(Value::as_object_mut) {
                let at = e.get("t").and_then(Value::as_f64).unwrap_or(0.0) - shift_s;
                let hit_this = this.iter().any(|&m| (m - at).abs() <= LEARN_REACH + 1e-9);
                let hit_not = not.iter().any(|&m| (m - at).abs() <= LEARN_REACH + 1e-9);
                if filter && hit_not {
                    e.insert("filtered".into(), Value::Bool(true));
                } else if hit_this {
                    e.insert("learned".into(), Value::Bool(true));
                }
            }
        }

        // rule 3: a new hit closer than the minimum gap to an untouched hit outside goes
        let min_gap = parsed.params.min_gap_ms / 1000.0;
        let out_times: Vec<f64> = outside
            .iter()
            .filter_map(|e| e.get("t").and_then(Value::as_f64))
            .collect();
        picked.retain(|e| {
            let t = e.get("t").and_then(Value::as_f64).unwrap_or(0.0);
            !out_times.iter().any(|&o| (o - t).abs() < min_gap - 1e-9)
        });

        // rule 4: the edited hits of the range stay when the Keep dialog said so
        if keep_edited && !edited_inside.is_empty() {
            picked = crate::edit::merge_edited(std::mem::take(&mut picked), &edited_inside);
        }
        let n_old = inside_old.len();
        let n_new = picked.len();

        // the loudness cut of the track marks the new hits like a whole-track run (P4-3)
        let cut = lock(&op)
            .track(name)
            .ok()
            .and_then(|t| crate::histogram::cut_of(t.params.as_ref()));
        let mut events: Vec<Event> = {
            let mut all = outside.clone();
            all.extend(picked.iter().cloned());
            all.sort_by(|x, y| {
                let t = |e: &Value| e.get("t").and_then(Value::as_f64).unwrap_or(0.0);
                t(x).total_cmp(&t(y))
            });
            all.iter()
                .filter_map(|e| serde_json::from_value::<Event>(e.clone()).ok())
                .collect()
        };
        crate::histogram::mark_cut_events(&mut events, &d.samples, d.sr, cut);

        // rule 6: the backup of the previous events, then the store (the rate, length and
        // `detector` record of the track stay; a range run changes no settings)
        let stored = {
            let mut g = lock(&op);
            crate::jobs::backup_events(&g, name)?;
            let tr = g.track_mut(name)?;
            tr.events = Some(events.clone());
            tr.extra.shift_remove("events");
            tr.sr.get_or_insert(i64::from(d.sr));
            g.mark_dirty();
            let vals: Vec<Value> = events.iter().filter_map(|e| serde_json::to_value(e).ok()).collect();
            vals
        };
        let rows = crate::follow::refresh_followers(self, &op, name);
        let mut out = json!({
            "track": name,
            "t0": t0,
            "t1": t1,
            "replaced": n_old,
            "n_new": n_new,
            "events": stored,
            "info": format!("{name}: re-analyzed {} to {} s, {n_old} hits replaced by {n_new}",
                fmt_s(t0), fmt_s(t1)),
        });
        if !rows.is_empty() {
            out["followers"] = Value::Array(crate::follow::refreshed_names(&rows));
        }
        Ok(out)
    }
}

/// Whether the hit `e` lies inside the range (its refined time is its `t`).
fn in_range(e: &Value, t0: f64, t1: f64) -> bool {
    match e.get("t").and_then(Value::as_f64) {
        Some(t) => t >= t0 - 1e-9 && t <= t1 + 1e-9,
        None => false,
    }
}

/// The info line format of rule 7: three decimals, like the lane (`0.900 to 2.100 s`).
fn fmt_s(t: f64) -> String {
    format!("{t:.3}")
}

/// Run a command of this card for the bridge; `None` when `cmd` is not one of them.
#[must_use]
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if cmd != "reanalyze_selection" {
        return None;
    }
    let run = || -> Result<Value, AppError> {
        let s = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{k}` (string)")))
        };
        let t = |k: &str| {
            args.get(k)
                .and_then(Value::as_f64)
                .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{k}` (number)")))
        };
        svc.reanalyze_selection(
            s("sid")?,
            s("name")?,
            t("t0")?,
            t("t1")?,
            args.get("keep_edited")
                .and_then(Value::as_bool)
                .unwrap_or(crate::command_policy::PAGE_KEEP_EDITED),
        )
    };
    Some(run())
}
