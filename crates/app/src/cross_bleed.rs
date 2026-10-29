//! #281 A6: the cross-track bleed rule for sparse tracks in the analysis
//! ([`drumspike_core::detect::cross_bleed`]).
//!
//! After the per-track analysis of a job (before the tom bleed rule, which judges the re-detected
//! hits), [`run_step`] runs the rule over the tom close mics of the project against every other
//! close mic of the session — the kick, snare and hi-hat close mics ([`source_kind`]) and
//! the other tom close mics ([`crate::tom_bleed::is_tom_close_mic`]). It applies to both detectors (it
//! works on the events, and on the DSP fallback's candidate list):
//!
//! * **The drop**: a tom hit that coincides within 10 ms with a louder peak of a kick, snare or
//!   hi-hat track — the source's **full candidate list** of #284 (every peak, kept or dropped),
//!   not only its confirmed hits (#285: the own method's snare recall on ENST is 0.42-0.59, so
//!   half of the snare bleed has no confirmed snare hit to coincide with), arrives later than it
//!   and is quieter in the band is bleed and is marked `"bleed": true` — out of the output
//!   exactly like a tom bleed rule mark ([`crate::tom_bleed::is_bleed`]). An edited hit is never
//!   marked.
//! * **The template validity** (the DSP fallback only, `DRUMSPIKE_A6_TMPL=1`, default **off** —
//!   the interim ablation of #285: the re-gating costs tom fixes on its own, variant b): the
//!   candidates that coincide with a louder hit on another track take no part in the loud-hit
//!   template; the track is re-detected with the exclusion
//!   ([`drumspike_core::detect::onset::detect_onsets_and_cands_excluding`]) and the resulting
//!   events (refined, velocities and all, via [`drumspike_core::analyze::analyze_detected`])
//!   replace the detected ones — the edited hits of the current events stay
//!   ([`crate::jobs::store_analysis`] with `keep`). The candidate list is written back with the
//!   per-candidate cross-mic lead and level ratio and the dropped candidates marked
//!   ([`drumspike_core::detect::onset::Gate::CrossBleed`]).
//!
//! The outcome goes into the track's `detector` record (`cross_bleed`: `{rule, on, removed,
//! redetected}`) and the job answer (`cross_bleed`). A mic that cannot be read or has another rate
//! is left out; a track without the stored candidate list gets the drop marks only.

use std::path::PathBuf;
use std::sync::Arc;

use drumspike_core::analyze::{analyze_detected, role_note, AnalyzeParams};
use drumspike_core::detect::cross_bleed::{cross_bleed, hit_level, CrossBleedParams, CrossMark, CrossMic, XKind};
use drumspike_core::detect::onset::{self, Gate, OnsetParams, Role};
use drumspike_core::project::Track;
use drumspike_core::refine::first_motion;
use serde_json::{json, Map, Value};

use crate::media::{mix_mode, Decoded};
use crate::registry::{lock, OpenProject, Shared};
use crate::roles::is_sample_name;
use crate::{Service, ServiceConfig};

/// The version a new analysis runs (`detector.cross_bleed.rule`). 2 is #103 step 2: the hat
/// source's own verdict (ratio at or under -32 dB, lead at or over -10 ms) and the
/// nearest-kept-hit evidence match (the stored events carry the refined times, which lost the
/// keys on kept hits).
pub const CROSS_BLEED: u32 = 2;

/// The version of a new analysis: the configuration's pin, else [`CROSS_BLEED`].
pub fn analysis_cross_bleed(cfg: &ServiceConfig) -> u32 {
    cfg.cross_bleed.unwrap_or(CROSS_BLEED)
}

/// A tom close mic of the rule (the same selection as [`crate::tom_bleed::is_tom_close_mic`]).
fn is_tom_close_mic(name: &str, role: Option<&str>) -> bool {
    role == Some("tom") && !is_sample_name(name)
}

/// A kick, snare or hi-hat close mic of the rule (a source only; no sample or trigger track).
/// The hi-hat (#285): the ENST toms sit next to the hi-hat, so its bleed escapes the kick/snare
/// coincidence; the same rule runs over it.
fn source_kind(name: &str, role: Option<&str>) -> Option<XKind> {
    if is_sample_name(name) {
        return None;
    }
    match role {
        Some("kick") => Some(XKind::Kick),
        Some("snare") => Some(XKind::Snare),
        Some("hat") => Some(XKind::Hat),
        // #286 A1: the overhead carries the hat and cymbal bleed; its source kind is measured
        // in the target's band (the common-band test in `cross_bleed`)
        Some("cymbal") => Some(XKind::Overhead),
        _ => None,
    }
}

struct Mic {
    name: String,
    kind: Option<XKind>,
    /// The track's source path.
    path: PathBuf,
    mix: String,
    /// The event times (the refine back-track: the first motion of every hit).
    times: Vec<f64>,
    /// The track's full candidate list (#284, every peak kept or dropped) when one is stored and
    /// current (#285): the rule coincides against every peak of a source track, not only its
    /// confirmed hits — a dropped soft snare candidate is still a snare stroke louder than its
    /// bleed on the tom. `None` without a stored list (the events decide).
    cands: Option<Vec<f64>>,
    /// Per event, an edited hit (the rule never marks it).
    edited: Vec<bool>,
}

/// An ablation switch for #285 (env): `DRUMSPIKE_A6_RULE` gates the drop (the marks applied to
/// the events and the re-detected candidates, `1` default); `DRUMSPIKE_A6_TMPL` gates the template
/// exclusion in the re-detection (`0` default — the coordinator's decision on the interim
/// ablation: the re-gating costs tom fixes on its own, variant b, and stays behind the switch).
/// `DRUMSPIKE_A6_RULE=0`: the drop off (the `main` variant).
fn switch(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => v != "0",
        Err(_) => name != "DRUMSPIKE_A6_TMPL",
    }
}

/// The times of a track's stored candidate list (#284, every peak kept or dropped), `None` when
/// the list is missing, stale or from another detector than the DSP fallback (the model path
/// keeps no candidate list).
pub(crate) fn candidate_times(g: &OpenProject, name: &str, tr: &Track) -> Option<Vec<f64>> {
    let det = tr.extra.get("detector")?;
    if (det.get("detector").and_then(|d| d.as_str())).unwrap_or_default() != "dsp" {
        return None;
    }
    let dir = g.project.subdir(drumspike_core::project::ANALYSIS_DIR).join(name);
    let key = crate::candidates::candidates_key(
        &drumspike_core::project::cache_key(tr),
        tr.mix_mode_or_default(),
        det.get("model").unwrap_or(&Value::Null),
    );
    crate::candidates::read_candidates(&dir, &key).map(|(cs, _, _)| cs.iter().map(|c| c.time).collect())
}

/// The coincidence times of a source mic: the full candidate list when one is stored, else the
/// confirmed hits (#285).
/// The source's motion: the stored candidate times walked back to their first motion (#285: the
/// candidate times sit on the detector's grid (10 ms on ENST), while the tom's own times are the
/// refined first motion — the lead test compared grid against sample-exact and could not
/// separate. The walk-back is the same [`first_motion`] the refine stage gives every event).
fn refined_motion(sig: &[f32], sr: u32, ts: &[f64]) -> Vec<f64> {
    ts.iter()
        .map(|&t| {
            let s = (t * f64::from(sr)).round() as i64;
            let (foot, _) = first_motion(sig, s, sr, t, None, None);
            foot as f64 / f64::from(sr)
        })
        .collect()
}

fn source_motion(m: &Mic, sig: &[f32], sr: u32) -> Vec<f64> {
    m.cands
        .as_ref()
        .map_or_else(|| m.times.clone(), |cs| refined_motion(sig, sr, cs))
}

/// Run the rule after an analysis: `targets` are the analysed tracks. Marks the bleed hits of the
/// tom close mic targets, re-detects them with the template exclusion (the DSP fallback), writes
/// the annotated candidate list and records the outcome. Returns the session's outcome
/// (`{rule, tracks: {name: {removed, redetected}}}`), or `None` when no target is a tom close mic.
pub(crate) fn run_step(svc: &Service, op: &Shared, targets: &[String]) -> Option<Value> {
    let rule_on = switch("DRUMSPIKE_A6_RULE");
    let tmpl_on = switch("DRUMSPIKE_A6_TMPL");
    if !rule_on && !tmpl_on {
        return None; // the ablation's `main` variant: the feature off
    }
    let (mics, tgt): (Vec<Mic>, Vec<usize>) = {
        let g = lock(op);
        let mut mics = Vec::new();
        for name in g.project.doc.track_order() {
            let Ok(tr) = g.track(&name) else { continue };
            let role = tr.role.as_deref();
            let kind = source_kind(&name, role);
            if !(is_tom_close_mic(&name, role) || kind.is_some())
                || tr.follows.is_some()
                || tr.events.is_none()
                || g.offline(&name)
            {
                continue;
            }
            let Ok(path) = g.source(&name) else { continue };
            let evs = tr.events();
            let cands = candidate_times(&g, &name, tr);
            mics.push(Mic {
                name,
                kind,
                path,
                mix: tr.mix_mode_or_default().to_string(),
                times: evs.iter().map(|e| e.t).collect(),
                cands,
                edited: evs
                    .iter()
                    .map(|e| {
                        e.added == Some(true)
                            || e.orig.is_some_and(|o| (o - e.t).abs() > 1e-9)
                            || e.edited == Some(true)
                    })
                    .collect(),
            });
        }
        let tgt: Vec<usize> = targets
            .iter()
            .filter_map(|n| mics.iter().position(|m| m.kind.is_none() && m.name == *n))
            .collect();
        (mics, tgt)
    };
    if tgt.is_empty() {
        return None;
    }
    let p = CrossBleedParams::default();
    let mut tracks = Map::new();
    for &a in &tgt {
        if let Some(out) = run_one(svc, op, &mics, a, &p, rule_on, tmpl_on) {
            tracks.insert(mics[a].name.clone(), out);
        }
    }
    if tracks.is_empty() {
        return None;
    }
    Some(json!({"rule": CROSS_BLEED, "tracks": tracks}))
}

/// The rule's mics: the tom mic under test with `motion` as its hit times, then the sources.
fn cross_mics<'a>(
    x: &'a Decoded,
    srcs: &'a [(usize, Arc<Decoded>)],
    mics: &'a [Mic],
    motion: Vec<f64>,
) -> Vec<CrossMic<'a>> {
    let mut v = vec![CrossMic {
        kind: XKind::Tom,
        signal: &x.samples,
        motion,
    }];
    for (i, d) in srcs {
        v.push(CrossMic {
            kind: mics[*i].kind.expect("checked above"),
            signal: &d.samples,
            motion: source_motion(&mics[*i], &d.samples, d.sr),
        });
    }
    v
}

/// The rule on one tom close mic: the marks on its events and (with a stored candidate list) the
/// re-detection with the template exclusion. Returns the track's outcome, `None` when nothing ran.
/// #96 reporting keys of a hit: the lead (ms) and the level ratio (dB) the rule saw on it, `a6_lead_ms` and
/// `a6_level_db`. A key is left out when the rule had no coinciding hit. They change no flag, time or count.
fn set_a6_keys(e: &mut drumspike_core::project::Event, m: CrossMark) {
    if let Some(lead) = m.lead_ms {
        e.extra.insert("a6_lead_ms".into(), json!(lead));
    } else {
        e.extra.shift_remove("a6_lead_ms");
    }
    if let Some(ratio) = m.ratio_db {
        e.extra.insert("a6_level_db".into(), json!(ratio));
    } else {
        e.extra.shift_remove("a6_level_db");
    }
    // #103 step 2 calibration: the hat pair's own lead and level (see `CrossMark::hat_lead_ms`).
    if let Some(lead) = m.hat_lead_ms {
        e.extra.insert("a6_hat_lead_ms".into(), json!(lead));
    } else {
        e.extra.shift_remove("a6_hat_lead_ms");
    }
    if let Some(ratio) = m.hat_ratio_db {
        e.extra.insert("a6_hat_level_db".into(), json!(ratio));
    } else {
        e.extra.shift_remove("a6_hat_level_db");
    }
}

fn run_one(
    svc: &Service,
    op: &Shared,
    mics: &[Mic],
    a: usize,
    p: &CrossBleedParams,
    rule_on: bool,
    tmpl_on: bool,
) -> Option<Value> {
    let x_mic = &mics[a];
    // decode the tom mic and the sources (a source that cannot be read or has another rate is out)
    let dec_x: Arc<Decoded> = svc.inner.media.decode_cached(&x_mic.path, mix_mode(&x_mic.mix)).ok()?;
    let mut srcs: Vec<(usize, Arc<Decoded>)> = Vec::new();
    for (i, m) in mics.iter().enumerate() {
        // every other close mic is a source: kick, snare and the other tom mics
        if !m.kind.is_some() || i == a {
            continue;
        }
        if let Ok(d) = svc.inner.media.decode_cached(&m.path, mix_mode(&m.mix)) {
            if d.sr == dec_x.sr {
                srcs.push((i, d));
            }
        }
    }
    if srcs.is_empty() {
        return None;
    }
    let sr = dec_x.sr;
    // 1. the drop marks on the events
    let mut removed = 0usize;
    {
        let mut g = lock(op);
        let tr = g.track_mut(&x_mic.name).ok()?;
        let evs = tr.events.as_mut()?;
        if evs.len() != x_mic.times.len() {
            return None; // the hits changed while the rule ran
        }
        let (marks_t, _) = cross_bleed(&cross_mics(&dec_x, &srcs, mics, x_mic.times.clone()), sr, p);
        let marks = &marks_t[0];
        if rule_on {
            for (j, e) in evs.iter_mut().enumerate() {
                if marks[j].bleed && !x_mic.edited[j] {
                    e.extra.insert("bleed".into(), Value::Bool(true));
                    removed += 1;
                } else {
                    e.extra.shift_remove("bleed");
                }
            }
        }
        // #96 reporting keys: the lead and level the rule saw on every hit it judged (no flag changes)
        for (j, e) in evs.iter_mut().enumerate() {
            set_a6_keys(e, marks[j]);
        }
        if let Some(Value::Object(det)) = tr.extra.get_mut("detector") {
            // `survive`: the step-1 marks that reach the CD step unless the re-detection below replaces the hits
            det.insert(
                "cross_bleed".into(),
                json!({"rule": CROSS_BLEED, "on": true, "removed": removed, "survive": removed}),
            );
        }
        g.mark_dirty();
    }
    // 2. the template validity: re-detect with the exclusion (the DSP fallback only)
    let mut redetected = 0usize;
    let dir = lock(op)
        .project
        .subdir(drumspike_core::project::ANALYSIS_DIR)
        .join(&x_mic.name);
    if let Some((params, _weak_limit, key)) = crate::candidates::read_candidates_meta(&dir) {
        let p_on = OnsetParams::for_role(Role::Tom);
        // the exclusion: candidates that coincide with a louder hit of another close mic
        let Ok((_, cands_a, _)) = onset::detect_onsets_and_cands(&dec_x.samples, sr, &p_on) else {
            return Some(json!({"removed": removed, "redetected": redetected}));
        };
        let exclude: Vec<usize> = if tmpl_on {
            (0..cands_a.len())
                .filter(|&k| {
                    let tc = cands_a[k].time;
                    srcs.iter().any(|(i, d)| {
                        source_motion(&mics[*i], &d.samples, sr).iter().any(|&ts| {
                            (ts - tc).abs() <= p.coincide_ms / 1000.0 + 1e-9
                                && hit_level(&d.samples, sr, mics[*i].kind.expect("checked above"), ts, p)
                                    > hit_level(&dec_x.samples, sr, XKind::Tom, tc, p)
                        })
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let Ok((hits, mut cands, weak2)) =
            onset::detect_onsets_and_cands_excluding(&dec_x.samples, sr, &p_on, &exclude)
        else {
            return Some(json!({"removed": removed, "redetected": redetected}));
        };
        // the drop on the re-detected candidates, with the evidence stored per candidate; the
        // candidate times are walked back to first motion as well (the same refinement)
        let cand_motion = refined_motion(&dec_x.samples, sr, &cands.iter().map(|c| c.time).collect::<Vec<_>>());
        let (marks_t, _) = cross_bleed(&cross_mics(&dec_x, &srcs, mics, cand_motion), sr, p);
        let marks = &marks_t[0];
        let mut dropped: Vec<usize> = Vec::new();
        for (j, c) in cands.iter_mut().enumerate() {
            c.cross_lead_ms = marks[j].lead_ms.map(|l| l as f32);
            c.cross_ratio_db = marks[j].ratio_db.map(|r| r as f32);
            if rule_on && marks[j].bleed && c.kept {
                c.kept = false;
                c.gate = Gate::CrossBleed;
                dropped.push(j);
            }
        }
        let drop_frames: Vec<usize> = dropped.iter().map(|&j| cands[j].frame).collect();
        redetected = drop_frames.len();
        let kept: Vec<(f64, u8)> = hits
            .iter()
            .filter(|h| !drop_frames.contains(&h.frame))
            .map(|h| (h.time, role_note("tom")))
            .collect();
        let params_a = AnalyzeParams {
            rule: crate::jobs::analysis_rule(&svc.inner.config),
            ..AnalyzeParams::default()
        };
        let Ok(analysis) = analyze_detected(&dec_x.samples, sr, "tom", &kept, &params_a) else {
            return Some(json!({"removed": removed, "redetected": redetected}));
        };
        let mut det = lock(op)
            .track(&x_mic.name)
            .ok()
            .and_then(|t| t.extra.get("detector").cloned())
            .unwrap_or_else(|| json!({"detector": "dsp"}));
        // #96: the dropped candidates with the lead and level the rule saw; the re-detection replaces the step-1 marks
        if let Some(c) = det.get_mut("cross_bleed") {
            let dropped_rec: Vec<Value> = dropped
                .iter()
                .map(|&j| json!({"t": cands[j].time, "lead_ms": marks[j].lead_ms, "ratio_db": marks[j].ratio_db}))
                .collect();
            c["survive"] = json!(0);
            c["dropped"] = json!(dropped_rec);
        }
        if crate::jobs::store_analysis(svc, op, &x_mic.name, (analysis, None), &det, &dec_x, false, true, None).is_err()
        {
            return Some(json!({"removed": removed, "redetected": redetected}));
        }
        // #96: the lead and level of the hits A6 kept, on their re-stored events (matched by the candidate's frame)
        let kept_keys: Vec<(f64, CrossMark)> = hits
            .iter()
            .filter(|h| !drop_frames.contains(&h.frame))
            .filter_map(|h| {
                cands
                    .iter()
                    .position(|c| c.frame == h.frame)
                    .map(|j| (h.time, marks[j]))
            })
            .collect();
        {
            let mut g = lock(op);
            if let Ok(tr) = g.track_mut(&x_mic.name) {
                // #103 step 1: the stored events carry the refined times of the analysis, not the
                // re-detection hit time `h.time`, so the exact-time match missed and the A6
                // evidence was lost on kept hits (1616 hat FPs with a hat hit inside the A6
                // window and no keys; package 8 of the detector plan). Nearest kept hit within
                // 25 ms instead: the refinement shift is a few ms and the detector's grid is
                // 10 ms, so 25 ms cannot reach the next hit of a dense track.
                const KEY_MATCH_MS: f64 = 0.025;
                for e in tr.events.iter_mut().flatten() {
                    let mut best: Option<(f64, CrossMark)> = None;
                    for &(t, m) in &kept_keys {
                        let d = (t - e.t).abs();
                        if d <= KEY_MATCH_MS && best.map_or(true, |(bd, _)| d < bd) {
                            best = Some((d, m));
                        }
                    }
                    if let Some((_, m)) = best {
                        set_a6_keys(e, m);
                    }
                }
            }
            g.mark_dirty();
        }
        // the annotated candidate list back into the analysis dir (the same key)
        if let Err(e) = crate::candidates::write_candidates(&dir, &cands, &params, weak2, &key) {
            eprintln!("candidates not kept for {}: {e}", x_mic.name);
        }
    }
    crate::follow::refresh_followers(svc, op, &x_mic.name);
    Some(json!({"removed": removed, "redetected": redetected}))
}
