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

use drumspike_core::analyze::{AnalyzeParams, analyze_detected, role_note};
use drumspike_core::detect::cross_bleed::{
    ALIGN_COUNT_MARGINS_DB, CrossBleedParams, CrossMark, CrossMic, PairLearned, XKind, cross_bleed, cross_bleed_prior,
    hit_level, inherits_pass1_bleed,
};
use drumspike_core::detect::onset::{self, Gate, OnsetParams, Role};
use drumspike_core::project::Track;
use drumspike_core::refine::first_motion;
use serde_json::{Map, Value, json};

use crate::media::{Decoded, mix_mode};
use crate::registry::{OpenProject, Shared, lock};
use crate::roles::is_sample_name;
use crate::{Service, ServiceConfig};

/// The version a new analysis runs (`detector.cross_bleed.rule`). 1 is #103 step 2: the hat-pair
/// keys and the nearest-kept-hit evidence match are reporting only — they change no flag, time or
/// count. 2 is #296: aligned-session pairs (Auto-Align 2 and the like) judge by level alone.
/// 3 is the verify fix on #296: the alignment is decided once on the stored hits (pass 1) and
/// handed to the re-detection pass (`cross_bleed_prior`), and every pair (aligned or not) is
/// recorded as `cross_bleed.pairs` in the job and detector records.
/// 4 is the #296 decision after the round 2-4 regressions: the pass-2 timing is back to the
/// first-motion walk of 3 (no re-timing of the kept hits), and a pass-2 candidate within 1 ms
/// of a pass-1 bleed-marked hit inherits the mark and is dropped; the records carry
/// `t_verdict` and the kept list.
/// 5 is the #296 verify fix on 4: the inherit rule is applied on the analysis's FINAL placement
/// (after `analyze_detected`, before `store_analysis`) instead of the pass-2 verdict time — a
/// candidate whose verdict time sits 10 ms clear of a pass-1 bleed hit can still be placed onto
/// it; the dropped events are recorded as `inherited`.
/// 6 is the coordinator's refinement of 5: the inherit rule is per CANDIDATE, not per placed
/// event — only the placement of a candidate whose own verdict found NO coinciding source
/// (`lead_ms` None) is dropped; a candidate with a coinciding source keeps pass 2's verdict and
/// an event the analysis added without a candidate is never touched.
pub const CROSS_BLEED: u32 = 6;

/// The version of a new analysis: the configuration's pin, else [`CROSS_BLEED`].
#[must_use]
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
                            || crate::policy::event_user_kept(e) // #242: the user's override wins
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

/// #296: the pairs of a rule run over one target, per source of the combined mic vec (the
/// target is index 0): the job record's and the detector record's `cross_bleed.pairs`. EVERY
/// pair is recorded (aligned or not), so the reason is visible: the source's track name, the
/// alignment verdict, the coincidence count, the median |lead| and the count of coinciding
/// target hits under each of [`ALIGN_COUNT_MARGINS_DB`] (the sweep counts on an aligned pair).
fn pairs_json(mics: &[Mic], srcs: &[(usize, Arc<Decoded>)], learned: &[PairLearned]) -> Value {
    Value::Array(
        learned
            .iter()
            .map(|l| {
                let counts: Map<String, Value> = ALIGN_COUNT_MARGINS_DB
                    .iter()
                    .zip(l.aligned_counts)
                    .map(|(m, c)| (m.to_string(), json!(c)))
                    .collect();
                json!({
                    "source": mics[srcs[l.source - 1].0].name,
                    "aligned": l.aligned,
                    "coins": l.align_coins,
                    "med_abs_lead_ms": l.align_med_abs_lead_ms,
                    "counts_db": counts,
                })
            })
            .collect(),
    )
}

/// #296 verify fix, coordinator refinement: inherit per CANDIDATE, not per placed event. A
/// placed event is dropped only when (a) it is the placement of a pass-2 candidate whose own
/// verdict found NO coinciding source (`lead_ms` None — the loophole that kept the bleed on
/// Session E) and (b) that placement lies within [`PASS1_INHERIT_MS`] of a pass-1 bleed-marked
/// time (`bleed_pass1`). A candidate that pass 2 judged WITH a coinciding source keeps pass 2's
/// verdict; an event the analysis added without a candidate is never touched. `kept_cands` are
/// the kept candidates as `(grid time, lead_ms)`; each maps to its placed event — the nearest
/// event within the analysis's refine window (-2..+12 ms) of the grid time. No time changes;
/// the parallel per-event vectors (`roll`, `learn_out`, `learned`) are filtered with the
/// events. Returns the dropped events' times.
fn inherit_on_placement(
    analysis: &mut drumspike_core::analyze::Analysis,
    bleed_pass1: &[f64],
    kept_cands: &[(f64, Option<f64>)],
) -> Vec<f64> {
    const CAND_PLACE_MS: f64 = 14.0 / 1000.0 + 1e-9; // the -2..+12 ms refine window
    let mut with_source = vec![false; analysis.events.len()];
    let mut mapped = vec![false; analysis.events.len()];
    for &(tc, lead) in kept_cands {
        let mut best: Option<usize> = None;
        for (i, e) in analysis.events.iter().enumerate() {
            let d = (e.t - tc).abs();
            if d <= CAND_PLACE_MS && best.is_none_or(|b| d < (analysis.events[b].t - tc).abs()) {
                best = Some(i);
            }
        }
        if let Some(i) = best {
            if lead.is_some() {
                with_source[i] = true;
            }
            mapped[i] = true;
        }
    }
    let mut inherited = Vec::new();
    let mut keep: Vec<usize> = Vec::new();
    for (i, h) in analysis.events.iter().enumerate() {
        if mapped[i] && !with_source[i] && inherits_pass1_bleed(h.t, bleed_pass1) {
            inherited.push(h.t);
        } else {
            keep.push(i);
        }
    }
    if inherited.len() < analysis.events.len() {
        let roll = keep
            .iter()
            .map(|&i| analysis.roll.get(i).copied().unwrap_or(false))
            .collect();
        let learn_out = keep
            .iter()
            .map(|&i| analysis.learn_out.get(i).copied().unwrap_or(false))
            .collect();
        let learned = keep
            .iter()
            .map(|&i| analysis.learned.get(i).copied().unwrap_or(false))
            .collect();
        analysis.events = keep.iter().map(|&i| analysis.events[i]).collect();
        analysis.roll = roll;
        analysis.learn_out = learn_out;
        analysis.learned = learned;
    }
    inherited
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
        if let Ok(d) = svc.inner.media.decode_cached(&m.path, mix_mode(&m.mix))
            && d.sr == dec_x.sr
        {
            srcs.push((i, d));
        }
    }
    if srcs.is_empty() {
        return None;
    }
    let sr = dec_x.sr;
    // 1. the drop marks on the events
    let mut removed = 0usize;
    // the pass-1 bleed marks: the times of the hits pass 1 judged bleed (assigned in the block
    // below). They are REMOVED from the track, so the re-detection cannot see them and judges
    // their candidates on their own — the pass-1 verdict is handed to pass 2 as the inherit
    // rule's band list (#296).
    let bleed_pass1: Vec<f64>;
    // the pass-1 pairs: the alignment decided here is handed to the re-detection (#296)
    let learned: Vec<PairLearned>;
    let pairs;
    {
        let mut g = lock(op);
        let tr = g.track_mut(&x_mic.name).ok()?;
        let evs = tr.events.as_mut()?;
        if evs.len() != x_mic.times.len() {
            return None; // the hits changed while the rule ran
        }
        let (marks_t, learned1) = cross_bleed(&cross_mics(&dec_x, &srcs, mics, x_mic.times.clone()), sr, p);
        let marks = &marks_t[0];
        bleed_pass1 = (0..marks.len())
            .filter(|&j| marks[j].bleed && !x_mic.edited[j])
            .map(|j| x_mic.times[j])
            .collect();
        learned = learned1;
        pairs = pairs_json(mics, &srcs, &learned);
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
                json!({"rule": CROSS_BLEED, "on": true, "removed": removed, "survive": removed, "pairs": pairs.clone()}),
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
        // candidate times are walked back to first motion as well (the same refinement) — the
        // pass-2 timing is the analysis's own placement and is NOT changed here (the #296
        // coordinator decision: rounds 2 to 4 re-timed the kept hits and shifted every
        // re-detected hit of the unaligned sessions 6 to 10 ms late). The pass-1 pairs come in
        // as `prior` (#296): the alignment is decided on the stored hits, not re-measured on
        // the candidate times (whose |lead| medians move).
        let cand_motion = refined_motion(&dec_x.samples, sr, &cands.iter().map(|c| c.time).collect::<Vec<_>>());
        let (marks_t, learned2) = cross_bleed_prior(
            &cross_mics(&dec_x, &srcs, mics, cand_motion.clone()),
            sr,
            p,
            Some(&learned),
        );
        // the pass-1 inherit rule (#296, coordinator refinement): applied per CANDIDATE on the
        // FINAL placement, after `analyze_detected` (see `inherit_on_placement`).
        let marks = &marks_t[0];
        let pairs2 = pairs_json(mics, &srcs, &learned2);
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
        let Ok(mut analysis) = analyze_detected(&dec_x.samples, sr, "tom", &kept, &params_a) else {
            return Some(json!({"removed": removed, "redetected": redetected, "pairs": pairs2}));
        };
        // the #296 verify fix, coordinator refinement: the inherit rule applies per CANDIDATE on
        // the FINAL placement. Each kept candidate maps to the placed event nearest its grid
        // time within the refine window; an event is dropped only when it is such a placement of
        // a candidate whose own verdict found NO coinciding source (`lead_ms` None) and the
        // placement lies within [`PASS1_INHERIT_MS`] of a pass-1 bleed-marked time. A candidate
        // with a coinciding source keeps pass 2's verdict; an event the analysis added without
        // a candidate is never touched. No time changes.
        let kept_cands: Vec<(f64, Option<f64>)> = cands
            .iter()
            .enumerate()
            .filter(|&(_, c)| c.kept)
            .map(|(j, c)| (c.time, marks[j].lead_ms))
            .collect();
        let inherited = inherit_on_placement(&mut analysis, &bleed_pass1, &kept_cands);
        let mut det = lock(op)
            .track(&x_mic.name)
            .ok()
            .and_then(|t| t.extra.get("detector").cloned())
            .unwrap_or_else(|| json!({"detector": "dsp"}));
        // #96: the dropped candidates with the lead and level the rule saw; the re-detection replaces the step-1 marks.
        // #296: the records carry the candidate's verdict time (`t_verdict`, the first-motion
        // walk of pass 2) next to the grid time, and the KEPT candidates are recorded too.
        if let Some(c) = det.get_mut("cross_bleed") {
            let dropped_rec: Vec<Value> = dropped
                .iter()
                .map(|&j| {
                    json!({"t": cands[j].time, "t_verdict": cand_motion[j], "lead_ms": marks[j].lead_ms, "ratio_db": marks[j].ratio_db})
                })
                .collect();
            let kept_rec: Vec<Value> = cands
                .iter()
                .enumerate()
                .filter(|(_, c)| c.kept)
                .map(|(j, c)| {
                    json!({"t": c.time, "t_verdict": cand_motion[j], "lead_ms": marks[j].lead_ms, "ratio_db": marks[j].ratio_db})
                })
                .collect();
            c["survive"] = json!(0);
            c["dropped"] = json!(dropped_rec);
            c["kept"] = json!(kept_rec);
            // #296: the events the inherit rule dropped on the final placement, their stored times.
            c["inherited"] = json!(inherited);
            c["pairs"] = pairs2.clone();
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
                        if d <= KEY_MATCH_MS && best.is_none_or(|(bd, _)| d < bd) {
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
    Some(json!({"removed": removed, "redetected": redetected, "pairs": pairs}))
}

#[cfg(test)]
mod tests {
    use drumspike_core::analyze::Analysis;
    use drumspike_core::refine::Hit;

    use super::inherit_on_placement;

    fn analysis(ts: &[f64]) -> Analysis {
        Analysis {
            events: ts.iter().map(|&t| Hit { t, v: 100, rise: None }).collect(),
            branches: Default::default(),
            roll: vec![false; ts.len()],
            learn_out: vec![false; ts.len()],
            learned: vec![false; ts.len()],
        }
    }

    /// #296, coordinator refinement, test 1: a kept candidate with NO coinciding source
    /// (`lead_ms` None) whose FINAL placement lands on a pass-1 bleed time is dropped, even
    /// though its verdict time sat 10 ms clear of the bleed hit.
    #[test]
    fn a_lead_none_candidate_placed_on_a_pass1_bleed_time_is_dropped() {
        let pass1_bleed: Vec<f64> = vec![1.000_000];
        // placed ON the pass-1 bleed hit; its grid and verdict times sat 10 ms before it
        let on_bleed = 1.000_400;
        assert!((on_bleed - pass1_bleed[0]).abs() <= 1e-3);
        let verdict_time = pass1_bleed[0] - 0.010;
        assert!(
            (verdict_time - pass1_bleed[0]).abs() > 1e-3,
            "10 ms is outside the verdict band"
        );
        // the candidate's grid time sits within the refine window of its placement
        let grid = on_bleed - 0.004;
        let mut a = analysis(&[on_bleed, verdict_time]);
        a.roll[1] = true;
        let inherited = inherit_on_placement(&mut a, &pass1_bleed, &[(grid, None)]);
        assert_eq!(inherited, vec![on_bleed]);
        assert_eq!(a.events.len(), 1);
        assert_eq!(a.events[0].t, verdict_time);
        assert!(a.roll[0], "the parallel vectors follow the kept events");
        assert_eq!(
            a.events.iter().position(|h| h.t == on_bleed),
            None,
            "absent from the stored events"
        );
    }

    /// #296, coordinator refinement, test 2: a loud tom hit 5 ms after a snare hit on an
    /// unaligned pair — its pass-2 candidate HAS a coinciding source (lead about -1.6 ms, any
    /// ratio) — is KEPT although pass 1 marked its position.
    #[test]
    fn a_candidate_with_a_coinciding_source_keeps_pass2s_verdict() {
        let pass1_bleed: Vec<f64> = vec![1.000_000];
        let loud = 1.005_000; // 5 ms after the snare hit
        let grid = loud - 0.003;
        let mut a = analysis(&[loud]);
        a.learn_out[0] = true;
        let inherited = inherit_on_placement(&mut a, &pass1_bleed, &[(grid, Some(-1.6))]);
        assert!(inherited.is_empty(), "the loud real hit stays: {inherited:?}");
        assert_eq!(a.events.len(), 1);
        assert_eq!(a.events[0].t, loud);
        assert!(a.learn_out[0], "the parallel vectors follow");
    }

    /// #296, coordinator refinement, test 3: an event the analysis added WITHOUT a candidate, on
    /// a pass-1 bleed time, is kept — the rule never touches it.
    #[test]
    fn an_event_without_a_candidate_is_never_touched() {
        let pass1_bleed: Vec<f64> = vec![1.000_000];
        let own = 1.000_400; // on the bleed time, but no candidate maps to it
        let mut a = analysis(&[own, 0.5]);
        a.roll[0] = true;
        a.learn_out[0] = true;
        let inherited = inherit_on_placement(&mut a, &pass1_bleed, &[(0.5, None)]);
        assert!(inherited.is_empty());
        assert_eq!(a.events.len(), 2);
        assert_eq!(a.events[0].t, own);
        assert!(a.roll[0]);
        assert!(a.learn_out[0]);
    }

    /// #296 verify fix: the inherit rule on the FINAL placement. A candidate whose grid time is
    /// clear of a pass-1 bleed hit by more than the refine window maps to no placed event — a
    /// real hit 4 ms from the snare hit, outside the band, stays. No time moves.
    #[test]
    fn the_inherit_rule_drops_on_the_final_placement_not_the_verdict_time() {
        let pass1_bleed: Vec<f64> = vec![1.000_000];
        let on_bleed = 1.000_400;
        let verdict_time = pass1_bleed[0] - 0.010;
        // a candidate whose grid time is 20 ms clear of the placement maps to nothing
        let far_grid = on_bleed - 0.020;
        let mut a = analysis(&[verdict_time, on_bleed, pass1_bleed[0] + 0.004]);
        a.roll[0] = true;
        let inherited = inherit_on_placement(&mut a, &pass1_bleed, &[(far_grid, None)]);
        assert!(inherited.is_empty(), "no candidate maps within the window");
        assert_eq!(a.events.len(), 3);
        assert!(a.roll[0], "the parallel vectors follow the kept events");
    }

    /// Nothing marked: the events pass through unchanged.
    #[test]
    fn no_pass1_marks_keeps_every_event() {
        let mut a = analysis(&[0.5, 0.501_5, 0.502]);
        let inherited = inherit_on_placement(&mut a, &[], &[(0.5, None)]);
        assert!(inherited.is_empty());
        assert_eq!(a.events.len(), 3);
    }
}
