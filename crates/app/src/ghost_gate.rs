//! #281 A1: the snare ghost-note gate in the analysis (the soft gate's busy clearance).
//!
//! The soft gate itself ([`drumspike_core::detect::onset`] `OnsetParams::soft_sim`) runs in the
//! per-track detection: a snare candidate below the level gate passes when its spectral shape
//! matches the track template tightly (cosine >= `soft_sim`, default 0.92) down to
//! `soft_gate_db` (default 30 dB) below the reference. The per-track detection cannot know
//! about the other tracks, so the 30 ms clearance against kick and tom hits runs here, after
//! every track of the job is stored (like the A6 cross-track rule):
//!
//! * **The recovery** needs no step: the soft-gated candidates are already events of the
//!   initial detection (`Gate::Soft`).
//! * **The clearance**: for every analysed snare close mic with a stored candidate list, the
//!   candidates that are soft-eligible (shape within the threshold, level within the floor)
//!   AND coincide within 30 ms with a hit of a kick or tom track are denied the soft rescue
//!   ([`drumspike_core::detect::onset::soft_busy_peaks`]); the track is re-detected with the
//!   exclusion ([`drumspike_core::detect::onset::detect_onsets_and_cands_soft_excluding`])
//!   and the resulting events replace the detected ones (the edited hits stay,
//!   [`crate::jobs::store_analysis`] with `keep`). A soft candidate whose shape does not
//!   match the snare template fails the soft gate on the shape alone — no busy information
//!   involved.
//!
//! The outcome goes into the track's `detector` record (`ghost_gate`: `{rule, on, dropped}`)
//! and the job answer (`ghost_gate`). A track without a stored candidate list, with another
//! detector than the DSP fallback, or with no busy soft candidate is left as it is.

use std::sync::Arc;

use drumspike_core::analyze::{AnalyzeParams, analyze_detected, role_note};
use drumspike_core::detect::onset::{self, OverheadSource};
use serde_json::{Map, Value, json};

use crate::media::{Decoded, mix_mode};
use crate::registry::{OpenProject, Shared, lock};
use crate::roles::is_sample_name;
use crate::{Service, ServiceConfig};

/// The version a new analysis runs (`detector.ghost_gate`).
pub const GHOST_GATE: u32 = 1;

/// The version of a new analysis: the configuration's pin, else [`GHOST_GATE`].
#[must_use]
pub fn analysis_ghost_gate(cfg: &ServiceConfig) -> u32 {
    cfg.ghost_gate.unwrap_or(GHOST_GATE)
}

/// The soft gate's busy clearance window (the A1 gate line).
pub const CLEAR_MS: f64 = 30.0;

/// The overhead denial margin (#286 A1): a soft-eligible snare candidate within the clearance
/// of an oh hit that is louder than the snare mic by more than this (dB, both mics measured
/// as the full-band peak — the common-band form) keeps no soft rescue. The env
/// `DRUMSPIKE_A1_OH_MARGIN_DB` overrides the default.
fn oh_margin_db() -> f64 {
    std::env::var("DRUMSPIKE_A1_OH_MARGIN_DB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.0)
}

/// A kick or tom close mic of the clearance (the busy sources; no sample or trigger track).
fn busy_role(name: &str, role: Option<&str>) -> bool {
    !is_sample_name(name) && matches!(role, Some("kick") | Some("tom"))
}

/// An overhead or cymbal track of the clearance (the #286 A1 oh denial; no sample or trigger
/// track).
fn oh_role(name: &str, role: Option<&str>) -> bool {
    !is_sample_name(name) && matches!(role, Some("cymbal"))
}

/// The busy times: the hit times of every kick and tom track of the project (their stored
/// events; an offline track's hits still sit in the recording).
fn busy_times(g: &OpenProject) -> Vec<f64> {
    let mut v = Vec::new();
    for name in g.project.doc.track_order() {
        let Ok(tr) = g.track(&name) else { continue };
        if !busy_role(&name, tr.role.as_deref()) {
            continue;
        }
        if let Some(evs) = tr.events.as_ref() {
            v.extend(evs.iter().map(|e| e.t));
        }
    }
    v.sort_by(f64::total_cmp);
    v
}

/// Run the clearance after an analysis: re-detects every analysed snare close mic whose
/// stored candidate list has a soft-eligible candidate within 30 ms of a kick or tom hit.
/// Returns the session's outcome (`{rule, tracks: {name: {dropped}}}`), or `None` when
/// nothing ran.
/// A track name with its mix mode (the plan's entries).
type NamedMix = (String, String);

pub(crate) fn run_step(svc: &Service, op: &Shared, targets: &[String]) -> Option<Value> {
    // the plan (the busy times, the oh tracks and the snare targets with their mix modes), one lock
    let (busy, ohs, snares): (Vec<f64>, Vec<NamedMix>, Vec<NamedMix>) = {
        let g = lock(op);
        let busy = busy_times(&g);
        let ohs: Vec<(String, String)> = g
            .project
            .doc
            .track_order()
            .into_iter()
            .filter(|name| {
                let Ok(tr) = g.track(name) else { return false };
                oh_role(name, tr.role.as_deref()) && tr.events.is_some() && !g.offline(name)
            })
            .filter_map(|name| {
                g.track(&name)
                    .ok()
                    .map(|tr| (name, tr.mix_mode_or_default().to_string()))
            })
            .collect();
        if busy.is_empty() && ohs.is_empty() {
            return None;
        }
        let snares = g
            .project
            .doc
            .track_order()
            .into_iter()
            .filter(|name| {
                let Ok(tr) = g.track(name) else { return false };
                tr.role.as_deref() == Some("snare")
                    && !is_sample_name(name)
                    && tr.follows.is_none()
                    && tr.events.is_some()
                    && !g.offline(name)
                    && targets.contains(name)
            })
            .filter_map(|name| {
                g.track(&name)
                    .ok()
                    .map(|tr| (name, tr.mix_mode_or_default().to_string()))
            })
            .collect();
        (busy, ohs, snares)
    };
    let mut tracks = Map::new();
    for (name, mix) in &snares {
        if let Some(out) = run_one(svc, op, name, mix, &busy, &ohs) {
            tracks.insert(name.clone(), out);
        }
    }
    if tracks.is_empty() {
        return None;
    }
    Some(json!({"rule": GHOST_GATE, "tracks": tracks}))
}

/// The clearance on one snare close mic. Returns the outcome, `None` when the track keeps
/// its events as they are.
fn run_one(svc: &Service, op: &Shared, name: &str, mix: &str, busy: &[f64], ohs: &[(String, String)]) -> Option<Value> {
    let (det, dir, params, key, cands) = {
        let g = lock(op);
        let tr = g.track(name).ok()?;
        let det = tr.extra.get("detector")?.clone();
        if det.get("detector").and_then(|d| d.as_str()) != Some(crate::jobs::DETECTOR_DSP) {
            return None;
        }
        let dir = g.project.subdir(drumspike_core::project::ANALYSIS_DIR).join(name);
        let (params, _weak, key) = crate::candidates::read_candidates_meta(&dir)?;
        params.soft_sim?; // another detector build: no soft gate to clear
        let cands = crate::candidates::read_candidates(&dir, &key)?.0;
        (det, dir, params, key, cands)
    };
    let path = lock(op).source(name).ok()?;
    let dec: Arc<Decoded> = svc.inner.media.decode_cached(&path, mix_mode(mix)).ok()?;
    // the oh sources (#286 A1): decoded signals; the denial is a property of the audio (the
    // oh level against the snare mic at the candidate's time), no oh hits are needed
    let mut keep_alive: Vec<Arc<Decoded>> = Vec::new();
    for (oname, omix) in ohs {
        let Some(opath) = lock(op).source(oname).ok() else {
            continue;
        };
        let Some(odec) = svc.inner.media.decode_cached(&opath, mix_mode(omix)).ok() else {
            continue;
        };
        if odec.sr != dec.sr {
            continue;
        }
        keep_alive.push(odec);
    }
    // the Arcs live in keep_alive; build the borrowed sources after all pushes
    let oh_sources: Vec<OverheadSource> = keep_alive
        .iter()
        .map(|d| OverheadSource {
            signal: &d.samples,
            sr: d.sr,
        })
        .collect();
    let mut peaks = onset::soft_busy_peaks(&cands, &params, busy, CLEAR_MS);
    if !oh_sources.is_empty() {
        peaks.extend(onset::soft_overhead_peaks(
            &cands,
            &params,
            (&dec.samples, dec.sr),
            &oh_sources,
            oh_margin_db(),
        ));
        peaks.sort_unstable();
        peaks.dedup();
    }
    if peaks.is_empty() {
        return None;
    }
    let Ok((hits, mut cands, weak)) =
        onset::detect_onsets_and_cands_soft_excluding(&dec.samples, dec.sr, &params, &peaks)
    else {
        return None;
    };
    let mut dropped = 0usize;
    for (j, c) in cands.iter_mut().enumerate() {
        if peaks.contains(&j) && c.kept {
            c.kept = false;
            c.gate = onset::Gate::Level;
            dropped += 1;
        }
        c.soft_busy = peaks.contains(&j);
    }
    let kept: Vec<(f64, u8)> = hits.iter().map(|h| (h.time, role_note("snare"))).collect();
    let params_a = AnalyzeParams {
        rule: crate::jobs::analysis_rule(&svc.inner.config),
        ..AnalyzeParams::default()
    };
    let analysis = analyze_detected(&dec.samples, dec.sr, "snare", &kept, &params_a).ok()?;
    crate::jobs::store_analysis(svc, op, name, (analysis, None), &det, &dec, false, true, None).ok()?;
    if let Err(e) = crate::candidates::write_candidates(&dir, &cands, &params, weak, &key) {
        eprintln!("candidates not kept for {name}: {e}");
    }
    {
        let mut g = lock(op);
        if let Some(Value::Object(d)) = g.track_mut(name).ok()?.extra.get_mut("detector") {
            d.insert(
                "ghost_gate".into(),
                json!({"rule": GHOST_GATE, "on": true, "dropped": dropped}),
            );
        }
        g.mark_dirty();
    }
    crate::follow::refresh_followers(svc, op, name);
    Some(json!({"dropped": dropped}))
}
