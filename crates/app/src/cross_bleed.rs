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

use drumspike_core::analyze::{AnalyzeParams, analyze_detected};
use drumspike_core::detect::cross_bleed::{
    ALIGN_COUNT_MARGINS_DB, CrossBleedParams, CrossMark, CrossMic, PairLearned, SessionAlign, XKind,
    cross_bleed_prior_session, cross_bleed_session, hit_level, inherits_pass1_bleed, measure_pair_alignment,
};
use drumspike_core::detect::onset::{self, Gate, OnsetParams, Role};
use drumspike_core::project::Track;
use drumspike_core::refine::first_motion;
use drumspike_core::role::role_note;
use serde_json::{Map, Value, json};

use crate::media::{Decoded, mix_mode};
use crate::policy::{Mark, clear, mark};
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
/// 7 is #309: a pair of a target with a measured-aligned pair (any other pair of the same tom)
/// inherits the session's alignment and judges by level alone — sparse or not, the lead of the
/// other pairs carries no information on an aligned session; the records carry `align_inherited`
/// per pair, through the re-detection prior as well.
/// 8 is #313: the alignment is a PER-SOURCE session property — a source is aligned on the
/// session only when it measures aligned on every tom target with at least `align_min_coins`
/// coincidences and on at least two such targets (the records carry `source_aligned_targets` and
/// `source_targets` per pair), and the #309 inheritance fires only from a session-aligned
/// source's measured-aligned pair. The pass-1 inherit rule takes only the marks an ALIGNED pair
/// made (`aligned_mark`): on a session with no aligned pair the re-detection judges every
/// candidate on its own, exactly as before #296.
/// 9 is #355: the rule judges every hit on the first motion its OWN signal's walk-back
/// derives ([`drumspike_core::detect::cross_bleed`] `canon_motion`), not on the refined list
/// position — a sub-millisecond move of a refined position between placement versions (version
/// 8 to 9 moved a snare hit 0.19 ms earlier) can no longer flip a verdict.
pub const CROSS_BLEED: u32 = 9;

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
    let sw = Switches {
        rule: rule_on,
        tmpl: tmpl_on,
    };
    if !sw.rule && !sw.tmpl {
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
    let p = CrossBleedParams::with_overrides(&svc.inner.config.research);
    // #313: the per-source session test is a SESSION property — decide it ONCE over all tom
    // close mics of the session, before the per-target runs (each run_one sees ONE tom and so
    // could never count two qualifying targets for a source).
    let session = session_alignment(svc, &mics, &p);
    let mut tracks = Map::new();
    for &a in &tgt {
        if let Some(out) = run_one(
            svc,
            op,
            &mics,
            a,
            &p,
            Switches {
                rule: rule_on,
                tmpl: tmpl_on,
            },
            &session,
        ) {
            tracks.insert(mics[a].name.clone(), out);
        }
    }
    if tracks.is_empty() {
        return None;
    }
    Some(json!({"rule": CROSS_BLEED, "tracks": tracks}))
}

/// #313: the per-source session alignment, decided ONCE per session before the per-target
/// runs: every tom close mic and every other close mic of the session is decoded (cached) and
/// every (tom, source) pair's alignment is measured ([`measure_pair_alignment`]). Returns, per
/// index of `mics`, the number of tom targets with at least `align_min_coins` coincidences
/// with the source and how many of those measured aligned. A mic that cannot be decoded (or
/// whose rate excludes the pair) counts no target, so it cannot establish an alignment.
fn session_alignment(svc: &Service, mics: &[Mic], p: &CrossBleedParams) -> Vec<(usize, usize)> {
    let decs: Vec<Option<Arc<Decoded>>> = mics
        .iter()
        .map(|m| svc.inner.media.decode_cached(&m.path, mix_mode(&m.mix)).ok())
        .collect();
    session_alignment_decs(mics, &decs, p)
}

/// [`session_alignment`] over already-decoded signals (`None` when a mic could not be decoded).
fn session_alignment_decs(mics: &[Mic], decs: &[Option<Arc<Decoded>>], p: &CrossBleedParams) -> Vec<(usize, usize)> {
    let mut out = vec![(0usize, 0usize); mics.len()];
    let is_tom = |m: &Mic| m.kind.is_none();
    for (a, ma) in mics.iter().enumerate() {
        if !is_tom(ma) {
            continue;
        }
        let Some(dx) = decs[a].as_ref() else { continue };
        // the CrossMic set at THIS tom's rate: the other toms and the eligible sources; the
        // target itself is included (measure_pair_alignment skips the (a, a) pair)
        let mut pos: Vec<usize> = Vec::new();
        for (j, mj) in mics.iter().enumerate() {
            let eligible = is_tom(mj) || mj.kind.is_some();
            if !eligible {
                continue;
            }
            if let Some(d) = decs[j].as_ref()
                && d.sr == dx.sr
            {
                pos.push(j);
            }
        }
        let cross: Vec<CrossMic> = pos
            .iter()
            .map(|&j| {
                let m = &mics[j];
                let d = decs[j].as_ref().expect("checked above");
                let kind = if is_tom(m) {
                    XKind::Tom
                } else {
                    m.kind.expect("checked above")
                };
                let motion = if is_tom(m) {
                    m.times.clone()
                } else {
                    source_motion(m, &d.samples, d.sr)
                };
                CrossMic {
                    kind,
                    signal: &d.samples,
                    motion,
                }
            })
            .collect();
        for (ta, sa, aligned, coins, _) in measure_pair_alignment(&cross, dx.sr, p) {
            // count each pair once, from its own target tom's pass (the set repeats per tom)
            if pos[ta] != a {
                continue;
            }
            if coins >= p.align_min_coins {
                out[pos[sa]].1 += 1;
                if aligned {
                    out[pos[sa]].0 += 1;
                }
            }
        }
    }
    out
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
/// #313: the pass-1 bleed times that feed the inherit rule — only the marks an ALIGNED pair
/// made ([`CrossMark::aligned_mark`]), never an edited hit. On a session with no aligned pair
/// the list is empty and the re-detection judges every candidate on its own, exactly as before
/// #296 (a two-part mark does not survive the re-detection there).
fn aligned_pass1_bleed(marks: &[CrossMark], times: &[f64], edited: &[bool]) -> Vec<f64> {
    (0..marks.len())
        .filter(|&j| marks[j].bleed && marks[j].aligned_mark && !edited[j])
        .map(|j| times[j])
        .collect()
}

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
                    "align_inherited": l.align_inherited,
                    // #313: the source's per-session test (see [`PairLearned::source_aligned_targets`])
                    "source_aligned_targets": l.source_aligned_targets,
                    "source_targets": l.source_targets,
                    // #309: the pair's clear-case bleed ratio distribution — a coinciding hit
                    // louder than the max is the tom's own motion, never bleed (aligned pairs)
                    "ratio_med_db": l.ratio_db,
                    "ratio_p95_db": l.ratio_p95_db,
                    "ratio_max_db": l.ratio_max_db,
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

/// The ablation switches of #285 (see [`switch`]): the drop and the template exclusion.
#[derive(Debug, Clone, Copy)]
struct Switches {
    /// `DRUMSPIKE_A6_RULE`: the drop marks on the events and the re-detected candidates.
    rule: bool,
    /// `DRUMSPIKE_A6_TMPL`: the template exclusion in the re-detection.
    tmpl: bool,
}

fn run_one(
    svc: &Service,
    op: &Shared,
    mics: &[Mic],
    a: usize,
    p: &CrossBleedParams,
    sw: Switches,
    session: &[(usize, usize)],
) -> Option<Value> {
    let x_mic = &mics[a];
    // decode the tom mic and the sources (a source that cannot be read or has another rate is out)
    let (dec_x, srcs) = decode_target(svc, mics, a)?;
    // #313: the session-wide per-source verdict, handed to both passes (see [`session_alignment`]).
    // The CrossMic vec is [the target, srcs...]; the target is never queried as a source.
    let sess = SessionAlign {
        source_aligned_targets: std::iter::once(0)
            .chain(srcs.iter().map(|(i, _)| session[*i].0))
            .collect(),
        source_targets: std::iter::once(0)
            .chain(srcs.iter().map(|(i, _)| session[*i].1))
            .collect(),
    };
    let t = Target {
        svc,
        op,
        mics,
        x_mic,
        dec_x,
        srcs,
        p,
        sw,
        sess,
    };
    // 1. the drop marks on the events
    let pass1 = mark_pass1(&t)?;
    // 2. the template validity: re-detect with the exclusion (the DSP fallback only)
    let redetected = match redetect(&t, &pass1) {
        Ok(redetected) => redetected,
        Err(early) => return Some(early),
    };
    crate::follow::refresh_followers(svc, op, &x_mic.name);
    Some(json!({"removed": pass1.removed, "redetected": redetected, "pairs": pass1.pairs}))
}

/// The decoded sources of a target: per source its index in the mics and its signal.
type Sources = Vec<(usize, Arc<Decoded>)>;

/// The rule run over one tom close mic target (`x_mic`): its decode, the decoded sources at its rate and the
/// session-wide per-source verdict.
struct Target<'a> {
    svc: &'a Service,
    op: &'a Shared,
    mics: &'a [Mic],
    x_mic: &'a Mic,
    dec_x: Arc<Decoded>,
    srcs: Sources,
    p: &'a CrossBleedParams,
    sw: Switches,
    sess: SessionAlign,
}

/// What pass 1 decided on the stored hits.
struct Pass1 {
    /// The hits pass 1 marked bleed.
    removed: usize,
    /// The pass-1 bleed marks: the times of the hits pass 1 judged bleed. They are REMOVED from the track, so the
    /// re-detection cannot see them and judges their candidates on their own — the pass-1 verdict is handed to pass
    /// 2 as the inherit rule's band list (#296).
    bleed_pass1: Vec<f64>,
    /// The pass-1 pairs: the alignment decided here is handed to the re-detection (#296).
    learned: Vec<PairLearned>,
    /// [`pairs_json`] of `learned`.
    pairs: Value,
}

/// The tom mic `a` and every other close mic at its rate as a source. `None` when the tom mic cannot be decoded or no
/// source is left.
fn decode_target(svc: &Service, mics: &[Mic], a: usize) -> Option<(Arc<Decoded>, Sources)> {
    let x_mic = &mics[a];
    let dec_x: Arc<Decoded> = svc.inner.media.decode_cached(&x_mic.path, mix_mode(&x_mic.mix)).ok()?;
    let mut srcs: Sources = Vec::new();
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
    Some((dec_x, srcs))
}

/// Pass 1 under the project lock: the rule on the stored hits, the drop marks (with the switch on), the #96 reporting
/// keys and the `cross_bleed` record. `None` when the track or its hits are gone or the hits changed while the rule ran.
fn mark_pass1(t: &Target) -> Option<Pass1> {
    let (x_mic, mics, srcs, sr) = (t.x_mic, t.mics, &t.srcs, t.dec_x.sr);
    let mut removed = 0usize;
    let mut g = lock(t.op);
    let tr = g.track_mut(&x_mic.name).ok()?;
    let evs = tr.events.as_mut()?;
    if evs.len() != x_mic.times.len() {
        return None; // the hits changed while the rule ran
    }
    let (marks_t, learned) = cross_bleed_session(
        &cross_mics(&t.dec_x, srcs, mics, x_mic.times.clone()),
        sr,
        t.p,
        Some(&t.sess),
    );
    let marks = &marks_t[0];
    let bleed_pass1 = aligned_pass1_bleed(marks, &x_mic.times, &x_mic.edited);
    let pairs = pairs_json(mics, srcs, &learned);
    // #309: pass 1's verdict, visible: per track, the hits pass 1 judged bleed with their
    // time, lead and ratio (the re-detection record below replaces the step-1 marks, but
    // this list stays on the detector record).
    let pass1_removed: Vec<Value> = (0..marks.len())
        .filter(|&j| marks[j].bleed && !x_mic.edited[j])
        .map(|j| json!({"t": x_mic.times[j], "lead_ms": marks[j].lead_ms, "ratio_db": marks[j].ratio_db}))
        .collect();
    if t.sw.rule {
        for (j, e) in evs.iter_mut().enumerate() {
            if marks[j].bleed && !x_mic.edited[j] {
                mark(e, Mark::Bleed);
                removed += 1;
            } else {
                clear(e, Mark::Bleed);
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
            json!({"rule": CROSS_BLEED, "on": true, "removed": removed, "survive": removed, "pass1_removed": pass1_removed, "pairs": pairs.clone()}),
        );
    }
    g.mark_dirty();
    Some(Pass1 {
        removed,
        bleed_pass1,
        learned,
        pairs,
    })
}

/// Pass 2 with a stored candidate list (the DSP fallback): re-detect with the template exclusion, drop the bleed
/// candidates, store the analysis, put the #96 keys on the kept hits and write the annotated candidate list. `Ok`: the
/// re-detected count (0 without a stored list); `Err`: the track's outcome of a run that stopped early (its followers
/// are not refreshed).
fn redetect(t: &Target, pass1: &Pass1) -> Result<usize, Value> {
    let (svc, op, x_mic, mics, srcs, p) = (t.svc, t.op, t.x_mic, t.mics, &t.srcs, t.p);
    let dec_x = &t.dec_x;
    let sr = dec_x.sr;
    let removed = pass1.removed;
    let mut redetected = 0usize;
    let dir = lock(op)
        .project
        .subdir(drumspike_core::project::ANALYSIS_DIR)
        .join(&x_mic.name);
    if let Some((params, _weak_limit, key)) = crate::candidates::read_candidates_meta(&dir) {
        let cfg = &svc.inner.config;
        let p_on = OnsetParams::for_role_with(Role::Tom, &cfg.research);
        // the exclusion: candidates that coincide with a louder hit of another close mic
        let Ok((_, cands_a, _)) = onset::detect_onsets_and_cands(&dec_x.samples, sr, &p_on, &cfg.rules, &cfg.research)
        else {
            return Err(json!({"removed": removed, "redetected": redetected}));
        };
        let exclude: Vec<usize> = if t.sw.tmpl {
            template_exclude(t, &cands_a)
        } else {
            Vec::new()
        };
        let Ok((hits, mut cands, weak2)) =
            onset::detect_onsets_and_cands_excluding(&dec_x.samples, sr, &p_on, &exclude, &cfg.rules, &cfg.research)
        else {
            return Err(json!({"removed": removed, "redetected": redetected}));
        };
        // the drop on the re-detected candidates, with the evidence stored per candidate; the
        // candidate times are walked back to first motion as well (the same refinement) — the
        // pass-2 timing is the analysis's own placement and is NOT changed here (the #296
        // coordinator decision: rounds 2 to 4 re-timed the kept hits and shifted every
        // re-detected hit of the unaligned sessions 6 to 10 ms late). The pass-1 pairs come in
        // as `prior` (#296): the alignment is decided on the stored hits, not re-measured on
        // the candidate times (whose |lead| medians move).
        let cand_motion = refined_motion(&dec_x.samples, sr, &cands.iter().map(|c| c.time).collect::<Vec<_>>());
        let (marks_t, learned2) = cross_bleed_prior_session(
            &cross_mics(dec_x, srcs, mics, cand_motion.clone()),
            sr,
            p,
            Some(&pass1.learned),
            Some(&t.sess),
        );
        // the pass-1 inherit rule (#296, coordinator refinement): applied per CANDIDATE on the
        // FINAL placement, after `analyze_detected` (see `inherit_on_placement`).
        let marks = &marks_t[0];
        let pairs2 = pairs_json(mics, srcs, &learned2);
        let dropped = drop_candidates(&mut cands, marks, t.sw.rule);
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
            return Err(json!({"removed": removed, "redetected": redetected, "pairs": pairs2}));
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
        let inherited = inherit_on_placement(&mut analysis, &pass1.bleed_pass1, &kept_cands);
        let mut det = lock(op)
            .track(&x_mic.name)
            .ok()
            .and_then(|t| t.extra.get("detector").cloned())
            .unwrap_or_else(|| json!({"detector": "dsp"}));
        // #96: the dropped candidates with the lead and level the rule saw; the re-detection replaces the step-1 marks.
        if let Some(c) = det.get_mut("cross_bleed") {
            record_pass2(c, &cands, &cand_motion, marks, &dropped, &inherited, &pairs2);
        }
        if crate::jobs::store_analysis(svc, op, &x_mic.name, (analysis, None), &det, dec_x, false, true, None).is_err()
        {
            return Err(json!({"removed": removed, "redetected": redetected}));
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
        set_kept_a6_keys(op, &x_mic.name, &kept_keys);
        // the annotated candidate list back into the analysis dir (the same key)
        if let Err(e) = crate::candidates::write_candidates(&dir, &cands, &params, weak2, &key) {
            eprintln!("candidates not kept for {}: {e}", x_mic.name);
        }
    }
    Ok(redetected)
}

/// The template exclusion: the target's candidates that coincide with a louder hit of another close mic.
fn template_exclude(t: &Target, cands_a: &[onset::Candidate]) -> Vec<usize> {
    let (mics, srcs, p, dec_x) = (t.mics, &t.srcs, t.p, &t.dec_x);
    let sr = dec_x.sr;
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
}

/// The pass-2 evidence on every re-detected candidate, and with the switch on the drop of the kept bleed candidates.
/// Returns the dropped candidates' indexes.
fn drop_candidates(cands: &mut [onset::Candidate], marks: &[CrossMark], rule: bool) -> Vec<usize> {
    let mut dropped: Vec<usize> = Vec::new();
    for (j, c) in cands.iter_mut().enumerate() {
        c.cross_lead_ms = marks[j].lead_ms.map(|l| l as f32);
        c.cross_ratio_db = marks[j].ratio_db.map(|r| r as f32);
        if rule && marks[j].bleed && c.kept {
            c.kept = false;
            c.gate = Gate::CrossBleed;
            dropped.push(j);
        }
    }
    dropped
}

/// Pass 2's part of the `cross_bleed` record `c`. #296: the records carry the candidate's verdict time (`t_verdict`,
/// the first-motion walk of pass 2) next to the grid time, and the KEPT candidates are recorded too.
fn record_pass2(
    c: &mut Value,
    cands: &[onset::Candidate],
    cand_motion: &[f64],
    marks: &[CrossMark],
    dropped: &[usize],
    inherited: &[f64],
    pairs2: &Value,
) {
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

/// Under the project lock: the #96 keys of the hits A6 kept on the re-stored events of `name`.
fn set_kept_a6_keys(op: &Shared, name: &str, kept_keys: &[(f64, CrossMark)]) {
    let mut g = lock(op);
    if let Ok(tr) = g.track_mut(name) {
        // #103 step 1: the stored events carry the refined times of the analysis, not the
        // re-detection hit time `h.time`, so the exact-time match missed and the A6
        // evidence was lost on kept hits (1616 hat FPs with a hat hit inside the A6
        // window and no keys; package 8 of the detector plan). Nearest kept hit within
        // 25 ms instead: the refinement shift is a few ms and the detector's grid is
        // 10 ms, so 25 ms cannot reach the next hit of a dense track.
        const KEY_MATCH_MS: f64 = 0.025;
        for e in tr.events.iter_mut().flatten() {
            let mut best: Option<(f64, CrossMark)> = None;
            for &(t, m) in kept_keys {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use drumspike_core::analyze::Analysis;
    use drumspike_core::detect::cross_bleed::{
        CrossBleedParams, CrossMark, CrossMic, SessionAlign, XKind, cross_bleed, cross_bleed_session,
    };
    use drumspike_core::refine::Hit;

    use super::{Mic, aligned_pass1_bleed, inherit_on_placement, session_alignment_decs};

    const SR: u32 = 48_000;

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

    fn mark(bleed: bool, aligned_mark: bool) -> CrossMark {
        CrossMark {
            bleed,
            lead_ms: Some(1.0),
            ratio_db: Some(-20.0),
            hat_lead_ms: None,
            hat_ratio_db: None,
            aligned_mark,
        }
    }

    /// #313 gate test 5, the mechanism-2 regression (the flip): the pass-1 marks that feed the
    /// inherit rule are only the ALIGNED pairs' ([`CrossMark::aligned_mark`]). A two-part mark —
    /// an unaligned pair's verdict — is not handed over, so the re-detection judges its
    /// candidate on its own and a candidate placed on the marked time is KEPT, exactly as
    /// before #296: on a session with no aligned pair the inherit rule is a no-op.
    #[test]
    fn a_two_part_mark_does_not_feed_the_inherit_rule() {
        let marked = 1.000_000;
        let times = vec![marked];
        let edited = vec![false];
        // the two-part mark stays out of the inherit list ...
        assert!(aligned_pass1_bleed(&[mark(true, false)], &times, &edited).is_empty());
        // ... the aligned mark is handed over ...
        assert_eq!(aligned_pass1_bleed(&[mark(true, true)], &times, &edited), vec![marked]);
        // ... and an edited hit is never handed over
        let edited_hit = vec![true];
        assert!(aligned_pass1_bleed(&[mark(true, true)], &times, &edited_hit).is_empty());
        // the flip geometry end to end: with no aligned mark the candidate placed on the
        // pass-1-marked time is kept (before #296, rule 1's mark died with the re-detection)
        let mut a = analysis(&[marked]);
        let inherited = inherit_on_placement(&mut a, &[], &[(marked - 0.004, None)]);
        assert!(inherited.is_empty(), "the event stays: {inherited:?}");
        assert_eq!(a.events.len(), 1);
        assert_eq!(a.events[0].t, marked);
    }

    // ---- #313: the session pre-pass and the handed-in verdict (fix tests 2 and 3) ----

    /// A kick-like hit at `t0` s (as the core's kick hit): a 55->125 Hz sweep.
    fn kick_hit(x: &mut [f32], t0: f64, db: f64) {
        let i0 = (t0 * f64::from(SR)).round() as usize;
        let amp = 10f64.powf(db / 20.0);
        let mut ph = 0.0f64;
        for k in 0..((0.3 * f64::from(SR)) as usize).min(x.len().saturating_sub(i0)) {
            let t = k as f64 / f64::from(SR);
            ph += 2.0 * std::f64::consts::PI * (55.0 + 70.0 * (-t / 0.03).exp()) / f64::from(SR);
            let env = (t / 0.0005).min(1.0) * (-t / 0.12).exp();
            x[i0 + k] += (ph.sin() * env * amp) as f32;
        }
    }

    fn mic(name: &str, kind: Option<XKind>, times: Vec<f64>) -> Mic {
        Mic {
            name: name.into(),
            kind,
            path: std::path::PathBuf::new(),
            mix: "average".into(),
            edited: vec![false; times.len()],
            times,
            cands: None,
        }
    }

    fn dec(sig: &[f32]) -> Option<Arc<crate::media::Decoded>> {
        Some(Arc::new(crate::media::Decoded {
            sr: SR,
            samples: sig.to_vec(),
        }))
    }

    /// The per-target [`CrossMic`] vec of `run_one` (the target, then the sources) with its handed-in
    /// session verdict, and the verdict run: the app path on one tom.
    fn run_target(
        toms: &[(&str, &[f32], &[f64])],
        srcs: &[(&str, XKind, &[f32], &[f64])],
        target: usize,
        session: &[(usize, usize)],
        p: &CrossBleedParams,
    ) -> (Vec<CrossMark>, Vec<drumspike_core::detect::cross_bleed::PairLearned>) {
        let mut mics_vec = vec![CrossMic {
            kind: XKind::Tom,
            signal: toms[target].1,
            motion: toms[target].2.to_vec(),
        }];
        let mut sess = vec![(0usize, 0usize)];
        for (name, kind, sig, times) in srcs {
            mics_vec.push(CrossMic {
                kind: *kind,
                signal: sig,
                motion: times.to_vec(),
            });
            let idx = toms.iter().position(|(n, _, _)| n == name);
            match idx {
                Some(i) => sess.push(session[i]),
                None => sess.push(session[toms.len() + srcs.iter().position(|(n, _, _, _)| n == name).expect("src")]),
            }
        }
        let (marks_t, learned) = cross_bleed_session(
            &mics_vec,
            SR,
            p,
            Some(&SessionAlign {
                source_aligned_targets: sess.iter().map(|s| s.0).collect(),
                source_targets: sess.iter().map(|s| s.1).collect(),
            }),
        );
        (marks_t[0].clone(), learned)
    }

    /// #313 fix test 2: two tom tracks of one session analysed as SEPARATE targets get the same
    /// session verdict (and records) as one call over both. The snare measures aligned on both
    /// toms (its copies at the snare's times), so the pre-pass reads the snare 2 of 2, and each
    /// per-target run judges by the level test alone with the session-wide record.
    #[test]
    fn two_tom_targets_get_the_same_session_verdict_as_one_call_over_both() {
        let n = (2.0f64 * f64::from(SR)) as usize;
        let mut snare = vec![0f32; n];
        let mut t1 = vec![0f32; n];
        let mut t2 = vec![0f32; n];
        let mut snare_motion = Vec::new();
        let mut t1_motion = Vec::new();
        let mut t2_motion = Vec::new();
        for i in 0..24 {
            let t = 0.2 + f64::from(i) * 0.05;
            kick_hit(&mut snare, t, -3.0);
            kick_hit(&mut t1, t + 0.0002, -21.0);
            kick_hit(&mut t2, t, -21.0);
            snare_motion.push(t);
            t1_motion.push(t + 0.0002);
            t2_motion.push(t);
        }
        // the mics of the session, as run_step builds them (the toms first, then the snare)
        let mics = [
            mic("t1", None, t1_motion.clone()),
            mic("t2", None, t2_motion.clone()),
            mic("snare", Some(XKind::Snare), snare_motion.clone()),
        ];
        let decs = [dec(&t1), dec(&t2), dec(&snare)];
        let p = CrossBleedParams::default();
        let session = session_alignment_decs(&mics, &decs, &p);
        // the pre-pass verdict: the snare aligned on 2 of 2 qualifying tom targets
        assert_eq!(session[2], (2, 2), "{session:?}");
        let toms: [(&str, &[f32], &[f64]); 2] = [("t1", &t1, &t1_motion), ("t2", &t2, &t2_motion)];
        let srcs: [(&str, XKind, &[f32], &[f64]); 1] = [("snare", XKind::Snare, &snare, &snare_motion)];
        let (marks1, learned1) = run_target(&toms, &srcs, 0, &session, &p);
        let (marks2, learned2) = run_target(&toms, &srcs, 1, &session, &p);
        // one call over both toms (the core's own full-session run) for comparison
        let mics_all = [
            CrossMic {
                kind: XKind::Tom,
                signal: &t1,
                motion: t1_motion.clone(),
            },
            CrossMic {
                kind: XKind::Tom,
                signal: &t2,
                motion: t2_motion.clone(),
            },
            CrossMic {
                kind: XKind::Snare,
                signal: &snare,
                motion: snare_motion.clone(),
            },
        ];
        let (marks_all, learned_all) = cross_bleed(&mics_all, SR, &p);
        // learned_all order: t1's (t2, snare) pairs, then t2's (t1, snare) pairs
        for (lp, la, mp, ma_all) in [
            (&learned1[0], &learned_all[1], &marks1, &marks_all[0]),
            (&learned2[0], &learned_all[3], &marks2, &marks_all[1]),
        ] {
            // the same session-wide record ...
            assert_eq!(lp.source_targets, la.source_targets, "{lp:?} vs {la:?}");
            assert_eq!(lp.source_aligned_targets, la.source_aligned_targets, "{lp:?} vs {la:?}");
            assert_eq!(lp.aligned, la.aligned, "{lp:?} vs {la:?}");
            assert_eq!(lp.align_coins, la.align_coins, "{lp:?} vs {la:?}");
            assert_eq!(lp.align_med_abs_lead_ms, la.align_med_abs_lead_ms, "{lp:?} vs {la:?}");
            // ... and the same verdict: the level test alone, the copies bleed with the mark
            for (m, ma) in mp.iter().zip(ma_all.iter()) {
                assert_eq!(m.bleed, ma.bleed, "{m:?} vs {ma:?}");
                assert!(m.bleed, "{m:?}");
            }
        }
    }

    /// #313 fix test 3, the ENST-like case through the app path: the snare measures aligned on
    /// tom 1 (0.2 ms copies) but not on tom 2 (1 ms copies) — the pre-pass reads 1 of 2, and
    /// NEITHER per-target run judges by the level test alone: the pairs keep the two-part
    /// verdict, so the 1 ms copies are bleed WITHOUT the aligned mark and the 0.2 ms copies of
    /// tom 1 stay.
    #[test]
    fn the_enst_like_case_stays_unaligned_through_the_app_path() {
        let n = (8.0f64 * f64::from(SR)) as usize;
        let mut snare = vec![0f32; n];
        let mut t1 = vec![0f32; n];
        let mut t2 = vec![0f32; n];
        let mut snare_motion = Vec::new();
        let mut t1_motion = Vec::new();
        let mut t2_motion = Vec::new();
        for i in 0..24 {
            let t = 0.2 + f64::from(i) * 0.05;
            kick_hit(&mut snare, t, -3.0);
            kick_hit(&mut t1, t + 0.0002, -21.0);
            kick_hit(&mut t2, t + 0.001, -21.0);
            snare_motion.push(t);
            t1_motion.push(t + 0.0002);
            t2_motion.push(t + 0.001);
        }
        let mics = [
            mic("t1", None, t1_motion.clone()),
            mic("t2", None, t2_motion.clone()),
            mic("snare", Some(XKind::Snare), snare_motion.clone()),
        ];
        let decs = [dec(&t1), dec(&t2), dec(&snare)];
        let p = CrossBleedParams::default();
        let session = session_alignment_decs(&mics, &decs, &p);
        // the pre-pass verdict: the snare aligned on only 1 of its 2 qualifying targets
        assert_eq!(session[2], (1, 2), "{session:?}");
        let toms: [(&str, &[f32], &[f64]); 2] = [("t1", &t1, &t1_motion), ("t2", &t2, &t2_motion)];
        let srcs: [(&str, XKind, &[f32], &[f64]); 1] = [("snare", XKind::Snare, &snare, &snare_motion)];
        let (marks1, learned1) = run_target(&toms, &srcs, 0, &session, &p);
        let (marks2, learned2) = run_target(&toms, &srcs, 1, &session, &p);
        for learned in [&learned1, &learned2] {
            assert!(!learned[0].aligned, "{learned:?}");
            assert_eq!(learned[0].source_targets, 2, "{learned:?}");
            assert_eq!(learned[0].source_aligned_targets, 1, "{learned:?}");
        }
        // two-part verdict on both toms: the copies are bleed without the aligned mark
        for m in &marks1 {
            assert!(!m.bleed && !m.aligned_mark, "{m:?}");
        }
        for m in &marks2 {
            assert!(m.bleed && !m.aligned_mark, "{m:?}");
        }
    }
}
