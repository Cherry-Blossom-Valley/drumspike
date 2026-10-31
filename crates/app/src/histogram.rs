//! P4-3: the data of the two Inspector histograms (Sensitivity and loudness, after Massey DRT)
//! and the two live controls behind them.
//!
//! | Command | What it does |
//! |---|---|
//! | `track_histograms {sid, name, events?}` | the peak level of every hit (dBFS) and the detection candidates from the kept activations ([`crate::activation`]) |
//! | `track_repick {sid, name, params?, store}` | the hits at the params' thresholds picked again from the kept activations, no model run; with `store` they replace the track's events as an analysis does |
//! | `track_velocity_set {sid, name, lo_db, hi_db, cut_db?, events?}` | store the velocity floor and ceiling and the loudness cut, and give the velocities and cut marks of the hits (a follower derives its hits again) |
//! | `track_params_set {sid, name, params}` | store the page's params of a track (the undo and redo of a re-pick give the slider back to the service) |
//!
//! While an analysis or a re-pick store of the project runs, `track_repick` with `store`,
//! `track_velocity_set`, `track_params_set` and `analyze_start` answer 409 ([`crate::jobs::Jobs::begin_store`]).
//! A re-pick writes the backup `Backups/<track>.events.prev.json` only the first time after an
//! analysis, so the backup keeps the hits from before the re-picks.
//!
//! The loudness cut (controller decision 4 after the DRT research, as DRT's Loudness slider filters): `vel_cut_db` in the
//! params, off when absent. A detected hit whose peak level is below it carries `"cut": true`; it stays in the
//! document and on screen (grey) but leaves the output: MIDI, spikes, Export All, the click and a follower
//! (`export::uncut`, [`event_cut`]). An edited hit (added, or moved from `orig`) is never cut. The job and the re-pick
//! mark the hits after each analysis ([`mark_cut_events`]), so the cut stays when the hits change.
//!
//! A re-pick runs the steps of the job after the model run on the same activations
//! ([`crate::jobs::pick_kept`]: the peak picking, the role filter, both min-gap dedups, the
//! refinement, the shift, the velocities, the P3-18 retry) and stores with the job's own store
//! ([`crate::jobs::store_analysis`]), so it stores what a Re-analyze at the same params stores.
//! `crates/app/tests/histogram.rs` checks that (`track_repick_gives_what_a_re_analyze_gives_at_the_same_params`,
//! and over the real sessions the ignored `repick_identity_over_the_golden_sessions`).

use std::path::PathBuf;

use drumspike_core::analyze::role_classes;
use drumspike_core::detect::CLASS_NOTES;
use drumspike_core::detect::N_CLASSES;
use drumspike_core::project::{ANALYSIS_DIR, Event, Track, cache_key};
use drumspike_core::velocity::{peak_db, velocity_of_db};
use serde_json::{Map, Value, json};

use crate::Service;
use crate::candidates::{candidates_key, read_candidates};
use crate::error::AppError;
use crate::export::hit_times;
use crate::follow::velocity_map_of;
use crate::jobs::{self, parse_params};
use crate::learn::learn_of_track;
use crate::media::mix_mode;
use crate::registry::{lock, resolve_shared};
use crate::sensitivity;

/// The loudness cut of a track's params (`vel_cut_db`), `None` when it is off.
pub fn cut_of(params: Option<&Map<String, Value>>) -> Option<f64> {
    params
        .and_then(|m| m.get("vel_cut_db"))
        .and_then(Value::as_f64)
        .filter(|c| c.is_finite())
}

/// True for an event out of the output: the loudness cut marked it (`"cut": true`), the tom
/// bleed rule (`"bleed": true`, [`crate::tom_bleed`]) or the old Learn filter (`"filtered"`, #281 D16).
/// R16 (#40): the one predicate is [`crate::policy::event_out`].
pub fn event_cut(e: &Event) -> bool {
    crate::policy::event_out(e)
}

/// True for an edited hit (added, or moved away from `orig`): the cut leaves it alone.
fn edited(added: bool, orig: Option<f64>, t: f64) -> bool {
    added || orig.is_some_and(|o| (o - t).abs() > 1e-9)
}

/// Mark the detected hits of `events` whose peak level in `x` is below `cut` (`"cut": true`) and clear the mark of
/// the others; with `cut` off every mark goes. Edited hits never carry it.
pub fn mark_cut_events(events: &mut [Event], x: &[f32], sr: u32, cut: Option<f64>) {
    for e in events {
        let on = cut.is_some_and(|c| !edited(e.added == Some(true), e.orig, e.t) && peak_db(x, e.t, sr) < c);
        if on {
            e.extra.insert("cut".into(), Value::Bool(true));
        } else {
            e.extra.shift_remove("cut");
        }
    }
}

/// [`mark_cut_events`] for the JSON events of the page (`t`, `added`, `orig`); every event must be an object.
pub fn mark_cut_values(events: &mut [Value], x: &[f32], sr: u32, cut: Option<f64>) -> Result<(), AppError> {
    for e in events {
        let m = e
            .as_object_mut()
            .ok_or_else(|| AppError::bad("every event must be an object"))?;
        let t = m
            .get("t")
            .and_then(Value::as_f64)
            .ok_or_else(|| AppError::bad("every event needs a numeric t"))?;
        let added = m.get("added") == Some(&Value::Bool(true));
        let orig = m.get("orig").and_then(Value::as_f64);
        if cut.is_some_and(|c| !edited(added, orig, t) && peak_db(x, t, sr) < c) {
            m.insert("cut".into(), Value::Bool(true));
        } else {
            m.shift_remove("cut");
        }
    }
    Ok(())
}

/// The commands of this card, as the route map names them.
pub const HISTOGRAM_COMMANDS: [&str; 4] = [
    "track_histograms",
    "track_repick",
    "track_velocity_set",
    "track_params_set",
];

/// The model classes (0 to 4) a role keeps: the classes whose note is in [`role_classes`].
pub fn role_class_indices(role: &str) -> Vec<usize> {
    let notes = role_classes(role);
    (0..N_CLASSES).filter(|&c| notes.contains(&CLASS_NOTES[c])).collect()
}

/// The first whole slider value at which a candidate of class `c` with the margin `strength`
/// passes its threshold (the thresholds fall as the slider rises); [`sensitivity::MAX`] when
/// none does (a margin below the floor by the `f32` rounding).
pub fn slider_value_of(strength: f32, c: usize, table: &[[f32; N_CLASSES]]) -> usize {
    table.partition_point(|th| th[c] > strength).min(table.len() - 1)
}

/// The widest run of whole slider values without a candidate between the lowest and the
/// highest value that has one (DRT: "mind the gap"): `{from, to, mid}` (the empty values
/// `from..=to`, `mid` the whole value in the middle), or `None` without an inner gap.
pub fn largest_gap(values: &[usize]) -> Option<Value> {
    let mut v: Vec<usize> = values.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut best: Option<(usize, usize)> = None;
    for w in v.windows(2) {
        let (a, b) = (w[0] + 1, w[1] - 1);
        if w[1] > w[0] + 1 && best.is_none_or(|(x, y)| b - a > y - x) {
            best = Some((a, b));
        }
    }
    best.map(|(a, b)| json!({"from": a, "to": b, "mid": (a + b) / 2}))
}
pub fn dsp_detection_candidates(
    cands: &[drumspike_core::detect::onset::Candidate],
    params: &drumspike_core::detect::onset::OnsetParams,
    weak_limit: f32,
    role: &str,
) -> Vec<Value> {
    let class = role_class_indices(role).first().copied().unwrap_or(0);
    cands
        .iter()
        .map(|c| {
            // The largest gate scale that still keeps the candidate, inverted to the slider
            // value (the kept set is `loud && weak_ok || rescue`: the paths are
            // alternatives, each true up to its own scale, so the bounds take the max). A
            // candidate kept by the detection passes at scale 1.0 (v <= 50 shows it
            // already); the constraints give the loosest scale that still keeps it.
            let attack_ok = c.attack >= params.attack_db;
            let weak_bound = if c.flux >= weak_limit || attack_ok {
                f64::INFINITY
            } else if c.flux > 0.0 {
                f64::from(weak_limit / c.flux)
            } else {
                0.0 // never passes the weak check: the loud path keeps it out
            };
            let loud_bound = match params.gate_db {
                None => weak_bound, // no level gate: only the weak check bounds the loud path
                Some(g) if c.rel_db < 0.0 => f64::from(-g / c.rel_db).min(weak_bound),
                Some(_) => weak_bound, // at or above the reference: only the weak check bounds
            };
            let rescue_bound = params.sim_min.map_or(0.0, |smin| {
                let by_sim = if c.similarity >= 1.0 {
                    f64::INFINITY
                } else {
                    f64::from((1.0 - smin) / (1.0 - c.similarity))
                };
                let by_gate = if c.rel_db < 0.0 {
                    f64::from(-params.sim_gate_db / c.rel_db)
                } else {
                    f64::INFINITY
                };
                by_sim.min(by_gate)
            });
            let s_needed = loud_bound.max(rescue_bound);
            let v = if s_needed.is_infinite() {
                0 // kept at every scale: on from the left end of the slider
            } else if s_needed <= 0.0 {
                sensitivity::MAX as usize
            } else {
                sensitivity::slider_of_scale(s_needed).round() as usize
            };
            json!([c.time, class, f64::from(c.similarity), v])
        })
        .collect()
}

/// The kept detection of a track (A4): the DSP candidate list with the detection's
/// parameters and weak-flux limit (#281 D16: main is model-free).
pub(crate) enum Kept {
    /// The DSP candidates, the detection's role parameters and its weak-flux limit.
    Dsp(
        Vec<drumspike_core::detect::onset::Candidate>,
        drumspike_core::detect::onset::OnsetParams,
        f32,
    ),
}

/// What one track needs, taken under the project lock.
/// The events of an analysis as the document stores them: `t`, `v`, `rise`, and the marks `roll` (P4-19) and
/// `filtered` (P4-6c).
pub(crate) fn analysis_values(analysis: &drumspike_core::analyze::Analysis) -> Vec<Value> {
    analysis
        .events
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let mut e = json!({"t": h.t, "v": h.v});
            if let Some(r) = h.rise {
                e["rise"] = json!(r);
            }
            if analysis.is_learn_out(i) {
                e[crate::learn_filter::FILTERED] = json!(true);
            }
            if analysis.is_learned(i) {
                e["learned"] = json!(true); // P4-38b: the Learn Add rule added it
            }
            e
        })
        .collect()
}

/// The inputs of a pick ([`Service::pick_inputs`]).
pub(crate) struct PickInputs {
    pub(crate) op: crate::registry::Shared,
    pub(crate) st: TrackState,
    pub(crate) params: Option<Map<String, Value>>,
    pub(crate) parsed: jobs::ParsedParams,
    pub(crate) kept: Kept,
    pub(crate) dec: std::sync::Arc<crate::media::Decoded>,
}

pub(crate) struct TrackState {
    pub(crate) source: Result<PathBuf, AppError>,
    pub(crate) mix: String,
    pub(crate) role: String,
    pub(crate) follows: Option<String>,
    pub(crate) dir: PathBuf,
    pub(crate) key: drumspike_core::project::CacheKey,
    pub(crate) params: Option<Map<String, Value>>,
}

impl Service {
    pub(crate) fn track_state(&self, op: &crate::registry::Shared, name: &str) -> Result<TrackState, AppError> {
        let g = lock(op);
        let tr = g.track(name)?;
        Ok(TrackState {
            source: g.source(name),
            mix: tr.mix_mode_or_default().to_string(),
            role: tr.role.clone().unwrap_or_else(|| "other".into()),
            follows: tr.follows.clone(),
            dir: g.project.subdir(ANALYSIS_DIR).join(name),
            key: cache_key(tr),
            params: tr.params.clone(),
        })
    }

    /// The kept detection of `st` (A4): the DSP candidate list with its detection parameters
    /// and weak-flux limit. `Err("missing")` means the candidates are stale (Re-analyze once).
    pub(crate) fn kept_detection(&self, st: &TrackState) -> Result<Result<Kept, &'static str>, AppError> {
        if let Some((cands, params, weak_limit)) =
            read_candidates(&st.dir, &candidates_key(&st.key, &st.mix, &Value::Null))
        {
            return Ok(Ok(Kept::Dsp(cands, params, weak_limit)));
        }
        Ok(Err("missing"))
    }

    /// The candidate rows of a track (`[t, class, similarity, v]`, [`dsp_detection_candidates`]) from its
    /// kept detection: `None` for a follower (its source has them), no kept detection or an offline track.
    /// #293: the Learn commands read the candidates here (the model's activations are gone).
    pub(crate) fn track_candidates(&self, sid: &str, name: &str) -> Result<Option<Vec<Value>>, AppError> {
        let op = self.inner.registry.get(sid)?;
        resolve_shared(&op);
        let st = self.track_state(&op, name)?;
        if st.follows.is_some() {
            return Ok(None);
        }
        Ok(match self.kept_detection(&st)? {
            Ok(Kept::Dsp(cands, cparams, weak_limit)) => {
                Some(dsp_detection_candidates(&cands, &cparams, weak_limit, &st.role))
            }
            Err(_) => None,
        })
    }

    /// `track_histograms {sid, name, events?}`: `{loudness: {levels}, detection}`. `levels` is
    /// the peak level in dBFS of each hit of `events` (the page's list, else the stored one),
    /// in their order (`velocity::peak_db`, the value the velocity maps). `detection` is
    /// `{state: "ok", classes, candidates, gap}` ([`detection_candidates`], [`largest_gap`]), or
    /// `{state}` with `follows` (a follower: its hits come from its source), `no_model` or
    /// `missing`. 409 for an offline track.
    pub fn track_histograms(&self, sid: &str, name: &str, events: Option<&[Value]>) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        resolve_shared(&op);
        let st = self.track_state(&op, name)?;
        let events = match events {
            Some(e) => e.to_vec(),
            None => self.events_of(sid, name)?,
        };
        let times = hit_times(&events)?;
        let source = st.source.clone()?;
        let d = self.inner.media.decode_cached(&source, mix_mode(&st.mix))?;
        let levels: Vec<f64> = times.iter().map(|&t| peak_db(&d.samples, t, d.sr)).collect();
        let detection = if let Some(src) = &st.follows {
            json!({"state": "follows", "source": src})
        } else {
            match self.kept_detection(&st)? {
                Ok(Kept::Dsp(cands, cparams, weak_limit)) => {
                    let candidates = dsp_detection_candidates(&cands, &cparams, weak_limit, &st.role);
                    let vs: Vec<usize> = candidates
                        .iter()
                        .filter_map(|c| c[3].as_u64())
                        .map(|v| v as usize)
                        .collect();
                    json!({"state": "ok", "classes": role_class_indices(&st.role), "candidates": candidates,
                           "gap": largest_gap(&vs), "max": sensitivity::MAX})
                }
                Err(state) => json!({"state": state}),
            }
        };
        Ok(json!({"loudness": {"levels": levels, "sr": d.sr}, "detection": detection}))
    }

    /// The stored events of a track as JSON values.
    pub(crate) fn events_of(&self, sid: &str, name: &str) -> Result<Vec<Value>, AppError> {
        let op = self.inner.registry.get(sid)?;
        let g = lock(&op);
        let m = g.track_object(name)?;
        Ok(m.get("events").and_then(Value::as_array).cloned().unwrap_or_default())
    }

    /// `track_repick {sid, name, params?, store}`: the hits at the thresholds of `params` (the
    /// page's params of the track, as `analyze_start` takes them; else the stored ones) picked
    /// again from the kept activations: `{events, n_events, weak_signal}`. Without `store`
    /// nothing changes (the live preview while a control moves). With `store` the params and the
    /// events are stored as the analysis job stores them (backup, detector record, the retry's
    /// sensitivity, the followers), and the answer carries the job's entry as `entry`.
    /// 409: a follower, no model, no current activations (Re-analyze once), an analysis running,
    /// an offline track; 400: wrong params.
    pub fn track_repick(
        &self,
        sid: &str,
        name: &str,
        params: Option<&Map<String, Value>>,
        store: bool,
    ) -> Result<Value, AppError> {
        self.track_repick_keep(sid, name, params, store, false)
    }

    /// The inputs of a pick from the kept activations of `name` (the re-pick and, P4-6c, a Learn
    /// change): the project, the track state, the params (`params`, else the stored ones) parsed with
    /// the recorded refinement and roll fill versions, the activations, their key data and the
    /// decoded source. 409 for a follower, an analysis running, no kept activations.
    pub(crate) fn pick_inputs(
        &self,
        sid: &str,
        name: &str,
        params: Option<&Map<String, Value>>,
    ) -> Result<PickInputs, AppError> {
        let op = self.inner.registry.get(sid)?;
        resolve_shared(&op);
        let st = self.track_state(&op, name)?;
        if let Some(src) = &st.follows {
            return Err(AppError::conflict(format!(
                "track {name} follows {src}: it has no detection of its own"
            )));
        }
        self.check_idle(sid)?;
        let params = params.cloned().or(st.params.clone());
        let mut parsed = parse_params(params.as_ref()).map_err(AppError::bad)?;
        if let Some(e) = &parsed.thresholds_error {
            return Err(AppError::bad(e.clone()));
        }
        // P4-12: a re-pick keeps the refinement rule of the track's analysis
        parsed.params.rule = {
            let g = lock(&op);
            jobs::recorded_rule(&self.inner.config, g.track(name)?.extra.get("detector"))
        };
        let Ok(kept) = self.kept_detection(&st)? else {
            return Err(AppError::conflict(format!(
                "no kept detection for {name}: re-analyze it once"
            )));
        };
        let source = st.source.clone()?;
        let dec = self.inner.media.decode_cached(&source, mix_mode(&st.mix))?;
        Ok(PickInputs {
            op,
            st,
            params,
            parsed,
            kept,
            dec,
        })
    }

    /// P4-6c: the hits of `name` after a Learn change ([`crate::learn_filter::refilter_merge`]): the normal
    /// pick from the kept candidates, marked by the filter when it runs, laid over the track's current hits
    /// (hand deletions, velocity edits and edited hits stay), plus the forced "this" hits and the Add-rule
    /// hits; stored as `events_set` stores the page's hits. No new detection is stored and no pick rule runs
    /// again. #293: the DSP candidates replace the kept activations. Answers `{events, followers?}`.
    pub(crate) fn learn_refilter_track(&self, sid: &str, name: &str) -> Result<Value, AppError> {
        let PickInputs {
            op,
            st,
            params,
            parsed,
            kept,
            dec,
        } = self.pick_inputs(sid, name, None)?;
        let Kept::Dsp(cands, cparams, weak_limit) = kept;
        let rows = dsp_detection_candidates(&cands, &cparams, weak_limit, &st.role);
        let scale = sensitivity::scale_of(sensitivity::sensitivity_of(&parsed.params.thresholds)) as f32;
        let mut dropped: Vec<f64> = Vec::new();
        let (base, forced) = if let Some(learn) = learn_of_track(&op, name) {
            // P4-38e: the Learn hits the engineer deleted (times before the Hitpoint shift) stay out
            let shift_s = crate::learn::shift_s_of(lock(&op).track(name)?);
            dropped = crate::learn_filter::dropped_times(&learn)
                .into_iter()
                .map(|d| d + shift_s)
                .collect();
            let edited = crate::learn_filter::edited_times(&self.events_of(sid, name)?);
            let pick = crate::learn_filter::learn_pick(
                &dec.samples,
                dec.sr,
                &st.role,
                (&cands, &cparams, weak_limit),
                &rows,
                &learn,
                &edited,
                &parsed.params,
                sensitivity::sensitivity_of(&parsed.params.thresholds).round(),
                scale,
                Some(&st.dir),
                Some(&json!({"source": st.key, "mix": st.mix})),
            )
            .map_err(AppError::bad)?;
            (analysis_values(&pick.base), analysis_values(&pick.forced))
        } else {
            let detector = drumspike_core::analyze::Detector::Candidates {
                cands: &cands,
                params: &cparams,
                weak_limit,
                scale,
            };
            let a = drumspike_core::analyze::analyze_track(&dec.samples, dec.sr, &st.role, &parsed.params, detector)
                .map_err(|e| AppError::bad(e.to_string()))?;
            (analysis_values(&a), Vec::new())
        };
        let (mut base, mut forced) = (base, forced);
        mark_cut_values(&mut forced, &dec.samples, dec.sr, cut_of(params.as_ref()))?;
        // P4-38d: the Add-rule hits of the base come in like the forced ones (the loudness cut marks them too)
        mark_cut_values(&mut base, &dec.samples, dec.sr, cut_of(params.as_ref()))?;
        let merged = crate::learn_filter::refilter_merge(&self.events_of(sid, name)?, &base, &forced, &dropped);
        let r = self.events_set(sid, name, &Value::Array(merged))?;
        // P4-38d (#81): on a tom close mic the Add-rule hits are new picked hits: the tom bleed rule judges them
        // as a re-pick does
        let role = lock(&op).track(name)?.role.clone();
        if crate::tom_bleed::is_tom_close_mic(name, role.as_deref()) {
            let v =
                crate::tom_bleed::recorded_bleed_rule(&self.inner.config, lock(&op).track(name)?.extra.get("detector"));
            let targets = crate::tom_bleed::repick_targets(self, &op, name, v);
            crate::tom_bleed::run_step(self, &op, &targets);
        }
        let mut out = json!({"events": self.events_of(sid, name)?});
        if let Some(f) = r.get("followers") {
            out["followers"] = f.clone();
        }
        Ok(out)
    }

    /// [`Service::track_repick`] with `keep_edited` (P4-5 step 2b): a stored re-pick keeps the
    /// edited hits of the track ([`crate::edit::merge_edited`]), and the answer holds the stored
    /// events. Without `store` the answer is the pick merged the same way (not stored), with
    /// `levels` aligned with the merged events (P4-9: the page's live preview).
    pub fn track_repick_keep(
        &self,
        sid: &str,
        name: &str,
        params: Option<&Map<String, Value>>,
        store: bool,
        keep_edited: bool,
    ) -> Result<Value, AppError> {
        // R06: the stored re-pick changes the document (the preview without `store` is a read)
        if store {
            self.inner.registry.get_writable(sid)?;
        }
        let PickInputs {
            op,
            st,
            params,
            parsed,
            kept,
            dec,
        } = self.pick_inputs(sid, name, params)?;
        // A4: a re-pick from the kept candidates; the gate scale is the slider's (the default
        // value 50 is scale 1.0 and reproduces the detection).
        let (analysis, weak) = match &kept {
            Kept::Dsp(cands, cparams, weak_limit) => {
                let scale = sensitivity::scale_of(sensitivity::sensitivity_of(&parsed.params.thresholds)) as f32;
                let detector = drumspike_core::analyze::Detector::Candidates {
                    cands,
                    params: cparams,
                    weak_limit: *weak_limit,
                    scale,
                };
                let a =
                    drumspike_core::analyze::analyze_track(&dec.samples, dec.sr, &st.role, &parsed.params, detector)
                        .map_err(|e| AppError::bad(e.to_string()))?;
                (a, None)
            }
        };
        let mut events = analysis_values(&analysis);
        mark_cut_values(&mut events, &dec.samples, dec.sr, cut_of(params.as_ref()))?;
        // the peak level of each hit: the loudness histogram follows the preview
        let levels: Vec<f64> = analysis
            .events
            .iter()
            .map(|h| peak_db(&dec.samples, h.t, dec.sr))
            .collect();
        let mut out = json!({"events": events, "n_events": analysis.events.len(), "levels": levels,
                             "weak_signal": weak.clone().unwrap_or(Value::Null)});
        if keep_edited && !store {
            // P4-9: the preview shows the edited hits as the stored re-pick will keep them; the levels follow the
            // merged list (the loudness histogram), `n_events` stays the number of picked hits
            let merged = crate::edit::merge_edited(events, &self.events_of(sid, name)?);
            let lv: Vec<f64> = merged
                .iter()
                .map(|e| peak_db(&dec.samples, e.get("t").and_then(Value::as_f64).unwrap_or(0.0), dec.sr))
                .collect();
            out["events"] = Value::Array(merged);
            out["levels"] = json!(lv);
        }
        if store {
            // one writer per project, and no analysis queued since the check above
            let _guard = self.begin_write(sid)?;
            let backup = self.inner.jobs.needs_backup(sid, name);
            if let Some(p) = params {
                let mut g = lock(&op);
                g.track_mut(name)?.params = Some(p);
                g.mark_dirty();
            }
            // A4: the record names the detector the pick came from.
            let det = jobs::detector_record(jobs::DETECTOR_DSP, None, Value::Null, parsed.params.rule);
            // P4-14: the re-pick runs the bleed rule version its analysis recorded (none before P4-14)
            let bleed_rule = {
                let g = lock(&op);
                crate::tom_bleed::recorded_bleed_rule(&self.inner.config, g.track(name)?.extra.get("detector"))
            };
            // P4-13 stage B: and the flam rule version it recorded (none before stage B)
            let flam_rule = {
                let g = lock(&op);
                crate::flam::recorded_flam_rule(&self.inner.config, g.track(name)?.extra.get("detector"))
            };
            // #293: a track with a Learn keeps its Learn marks through the re-pick (the same hits as a Learn change)
            let learned = match &kept {
                Kept::Dsp(cands, cparams, weak_limit) => {
                    let edited = if keep_edited {
                        crate::learn_filter::edited_times(&jobs::track_events(&op, name))
                    } else {
                        Vec::new()
                    };
                    let key = json!({"source": st.key, "mix": st.mix});
                    crate::learn_filter::stored_learn_events(
                        &op,
                        name,
                        &dec.samples,
                        dec.sr,
                        &st.role,
                        (cands, cparams, *weak_limit),
                        &parsed.params,
                        &edited,
                        &st.dir,
                        &key,
                    )
                    .map_err(AppError::bad)?
                }
            };
            out["entry"] = jobs::store_analysis(
                self,
                &op,
                name,
                (analysis, weak),
                &det,
                &dec,
                backup,
                keep_edited,
                learned,
            )
            .map_err(AppError::bad)?;
            // fix round: every tom of the session, as the analysis job runs it (the pair rule judges the toms together)
            let targets = crate::tom_bleed::repick_targets(self, &op, name, bleed_rule);
            if let Some(bleed) = crate::tom_bleed::run_step(self, &op, &targets) {
                out["bleed"] = bleed;
            }
            if let Some(flam) = crate::flam::run_step(self, &op, &[(name.to_string(), flam_rule)]) {
                out["flam"] = flam;
            }
            // round 2: marked only after the store, so a failed backup is written by the next re-pick
            self.inner.jobs.repick_stored(sid, name);
            out["backup"] = Value::Bool(backup);
            if keep_edited {
                let stored = self.events_of(sid, name)?;
                out["n_events"] = stored.len().into();
                out["events"] = Value::Array(stored);
                out.as_object_mut().map(|m| m.shift_remove("levels"));
            }
        }
        Ok(out)
    }

    /// `track_velocity_set {sid, name, lo_db, hi_db, cut_db?, events?}`: store the velocity floor
    /// and ceiling (`vel_lo_db`, `vel_hi_db` in the params) and the loudness cut (`vel_cut_db`;
    /// `None` removes it) and answer `{events}`: `events` (the page's list, else the stored one)
    /// with `v` mapped from each hit's peak level and the cut marks, not stored (the page applies
    /// them as one undo step, as Recompute Velocities). A follower derives its hits again with the
    /// new map and cut and stores them; its answer carries `follows`.
    pub fn track_velocity_set(
        &self,
        sid: &str,
        name: &str,
        lo_db: f64,
        hi_db: f64,
        cut_db: Option<f64>,
        events: Option<&[Value]>,
    ) -> Result<Value, AppError> {
        self.track_velocity_set_apply(sid, name, lo_db, hi_db, cut_db, events, false)
    }

    /// [`Service::track_velocity_set`] with `apply` (P4-5, M2): the new `v` and the cut marks are also written to the
    /// track's stored events (as `events_set` does with the answer), so a later export has them. The Service default is
    /// false (the page applies the answer itself as one undo step); `call::call` defaults it to true. The answer gets
    /// `applied: true`. A follower derives and stores its hits in both cases.
    #[allow(clippy::too_many_arguments)]
    pub fn track_velocity_set_apply(
        &self,
        sid: &str,
        name: &str,
        lo_db: f64,
        hi_db: f64,
        cut_db: Option<f64>,
        events: Option<&[Value]>,
        apply: bool,
    ) -> Result<Value, AppError> {
        if !lo_db.is_finite() || !hi_db.is_finite() || cut_db.is_some_and(|c| !c.is_finite()) {
            return Err(AppError::bad("lo_db, hi_db and cut_db must be numbers"));
        }
        let op = self.inner.registry.get_writable(sid)?;
        // round 2: the write holds the project's store lock, so a re-pick store cannot come in between
        let _guard = self.begin_write(sid)?;
        resolve_shared(&op);
        // R09: nothing is stored until the events validate and the source decodes; a rejected
        // command leaves the document (and the dirty flag) as it was.
        let (follows, old_params) = {
            let g = lock(&op);
            let tr = g.track(name)?;
            (tr.follows.clone(), tr.params.clone())
        };
        if let Some(src) = follows {
            // the derive reads the new map, so the params go in first; a failing derive (the
            // follower's own audio is gone) puts them back, so the rejection changes nothing
            {
                let mut g = lock(&op);
                set_velocity_params(g.track_mut(name)?, lo_db, hi_db, cut_db);
                g.mark_dirty();
            }
            if let Err(e) = crate::follow::derive_one(self, &op, name) {
                let mut g = lock(&op);
                g.track_mut(name)?.params = old_params;
                g.clean_if_saved();
                return Err(e);
            }
            return Ok(json!({"events": self.events_of(sid, name)?, "follows": src}));
        }
        let st = self.track_state(&op, name)?;
        let mut events = match events {
            Some(e) => e.to_vec(),
            None => self.events_of(sid, name)?,
        };
        let times = hit_times(&events)?;
        let d = self.inner.media.decode_cached(&st.source.clone()?, mix_mode(&st.mix))?;
        let mut new_map = Map::new();
        new_map.insert("vel_lo_db".into(), json!(lo_db));
        new_map.insert("vel_hi_db".into(), json!(hi_db));
        let map = velocity_map_of(Some(&new_map));
        for (e, t) in events.iter_mut().zip(times) {
            let m = e
                .as_object_mut()
                .ok_or_else(|| AppError::bad("every event must be an object"))?;
            m.insert("v".into(), json!(velocity_of_db(peak_db(&d.samples, t, d.sr), &map)));
        }
        mark_cut_values(&mut events, &d.samples, d.sr, cut_db)?;
        // commit: the params, the hits (`apply`) and the dirty flag, now that nothing can fail
        {
            let mut g = lock(&op);
            set_velocity_params(g.track_mut(name)?, lo_db, hi_db, cut_db);
            g.mark_dirty();
        }
        if apply {
            self.events_set_with(&op, name, &Value::Array(events.clone()))?;
            return Ok(json!({"events": events, "applied": true}));
        }
        Ok(json!({"events": events}))
    }
}

/// The velocity keys of a track's params (`vel_lo_db`, `vel_hi_db`, `vel_cut_db`); R09 writes
/// them once, at the commit of `track_velocity_set`.
fn set_velocity_params(tr: &mut Track, lo_db: f64, hi_db: f64, cut_db: Option<f64>) {
    let p = tr.params.get_or_insert_with(Map::new);
    p.insert("vel_lo_db".into(), json!(lo_db));
    p.insert("vel_hi_db".into(), json!(hi_db));
    match cut_db {
        Some(c) => p.insert("vel_cut_db".into(), json!(c)),
        None => p.shift_remove("vel_cut_db"),
    };
}

impl Service {
    /// 409 while an analysis of `sid` is queued or running, or a re-pick of it is stored.
    pub(crate) fn check_idle(&self, sid: &str) -> Result<(), AppError> {
        if self.inner.jobs.running(sid) {
            return Err(AppError::conflict("an analysis is running: wait for it to finish"));
        }
        if self.inner.jobs.storing(sid) {
            return Err(Self::busy());
        }
        Ok(())
    }

    /// Round 2: the project's store lock for a write (a re-pick store, the velocity map, the
    /// params): 409 while another write or an analysis of `sid` runs. Held until it is dropped.
    pub(crate) fn begin_write(&self, sid: &str) -> Result<crate::jobs::StoreGuard<'_>, AppError> {
        let guard = self.inner.jobs.begin_store(sid).ok_or_else(Self::busy)?;
        if self.inner.jobs.running(sid) {
            return Err(AppError::conflict("an analysis is running: wait for it to finish"));
        }
        Ok(guard)
    }

    /// R07: an edit, a re-pick store and an analysis of a project are one writer at a time; the
    /// loser of the guard gets this retryable conflict.
    fn busy() -> AppError {
        AppError::conflict("another write of the project is in progress: try again")
    }

    /// `track_params_set {sid, name, params}`: store the page's params of a track as they are
    /// (the object `analyze_start` takes). The undo and redo of a stored re-pick send the
    /// slider's params back, so a Save writes the params of the hits it shows. 409 while an
    /// analysis or a re-pick store runs.
    pub fn track_params_set(&self, sid: &str, name: &str, params: &Map<String, Value>) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let _guard = self.begin_write(sid)?;
        let mut g = lock(&op);
        // P4-5: a `sensitivity` without `thresholds` (an agent) stores the thresholds the page's slider would
        let mut params = params.clone();
        if !params.contains_key("thresholds")
            && let Some(v) = params.get("sensitivity").and_then(Value::as_f64)
        {
            params.insert("thresholds".into(), json!(sensitivity::thresholds_of(v)));
        }
        g.track_mut(name)?.params = Some(params.clone());
        g.mark_dirty();
        Ok(json!({"params": params}))
    }
}

/// A string argument.
fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (string)")))
}

/// A number argument.
fn arg_f64(args: &Value, key: &str) -> Result<f64, AppError> {
    args.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (number)")))
}

/// Run a command of this card for the bridge; `None` when `cmd` is not one of them. The
/// argument names are those of the Tauri wrappers.
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    let events = || args.get("events").and_then(Value::as_array).map(Vec::as_slice);
    let run = || -> Result<Value, AppError> {
        match cmd {
            "track_histograms" => svc.track_histograms(arg_str(args, "sid")?, arg_str(args, "name")?, events()),
            "track_repick" => svc.track_repick_keep(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                args.get("params").and_then(Value::as_object),
                args.get("store").and_then(Value::as_bool).unwrap_or(false),
                args.get("keep_edited").and_then(Value::as_bool).unwrap_or(false),
            ),
            "track_velocity_set" => svc.track_velocity_set_apply(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                arg_f64(args, "lo_db")?,
                arg_f64(args, "hi_db")?,
                match args.get("cut_db") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(
                        v.as_f64()
                            .ok_or_else(|| AppError::bad("wrong argument `cut_db` (number)"))?,
                    ),
                },
                events(),
                args.get("apply").and_then(Value::as_bool).unwrap_or(false),
            ),
            "track_params_set" => svc.track_params_set(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                args.get("params")
                    .and_then(Value::as_object)
                    .ok_or_else(|| AppError::bad("missing or wrong argument `params` (object)"))?,
            ),
            _ => Err(AppError::not_found(format!("unknown command {cmd}"))),
        }
    };
    HISTOGRAM_COMMANDS.contains(&cmd).then(run)
}

#[cfg(test)]
mod tests;
