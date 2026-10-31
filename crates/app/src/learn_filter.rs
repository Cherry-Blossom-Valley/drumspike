//! P4-6c: the Learn filter in the detection pipeline (concept-learn-mode.md 3.3).
//!
//! The filter marks the detected hits of the normal pick (`analyze::detect_hits`: the detector's
//! hits at the Sensitivity with the pick rules that run on them, such as the roll fill), so the
//! hit list with the filter on holds the same hits as without it. A hit stays in the output when
//! the similarity score of its candidate passes the track's threshold. Overrides (concept 2.2 and
//! 3.6): a "this" example is always kept (one under the Sensitivity comes in as a forced hit,
//! [`forced_this`]), an edited hit is never filtered, and the hit nearest to a "not" example
//! always goes. A hit the filter takes out stays a hit marked `"filtered": true` (drawn grey, out
//! of MIDI, spikes, clicks and followers, as the loudness cut and the tom bleed rule do:
//! `export::is_cut`). With the filter off the pipeline is the one of P4-3 (identity:
//! [`super::jobs::pick_kept`]).
//!
//! A Learn change (a mark, the threshold, the filter on or off) does not pick again: it marks the
//! track's current hits ([`refilter_merge`]), so hand deletions, velocity edits and moved or added
//! hits stay as the engineer left them.

use std::path::Path;

use drumspike_core::analyze::{
    Analysis, AnalyzeParams, Detector, analyze_detected, analyze_flags, detect_events, role_note,
};
use drumspike_core::detect::onset::{Candidate, OnsetParams};
use drumspike_core::detect::peaks::FPS;
use drumspike_core::similarity::{self, DEFAULT_THRESHOLD, Recipe};
use drumspike_core::velocity::attack_ms;
use serde_json::{Map, Value};

use crate::edit::{MERGE_WINDOW, is_kept};
use crate::histogram::{analysis_values, dsp_detection_candidates};
use crate::learn::{METHOD, filter_on, learn_of_track, shift_s_of};
use crate::learn_score::{
    Centre, candidate_frames, effective_threshold, example_times, frame_values, learn_key, read_learn, score_all,
    solid_frames, write_learn,
};
use crate::registry::{Shared, lock};
use crate::sensitivity::{scale_of, sensitivity_of};

/// The reach of an example to a hit or a candidate (seconds): the marks resolve to candidates
/// within [`crate::learn::CANDIDATE_REACH`].
pub const REACH: f64 = 0.030;

/// The reach of an edited hit: [`MERGE_WINDOW`], the window in which an edited hit wins over a
/// picked one (`edit::merge_edited`), so a picked hit is either replaced by the edited one or
/// judged by its own score.
pub const EDITED_REACH: f64 = MERGE_WINDOW;

/// The event key of a hit the filter takes out (`"filtered": true`).
pub const FILTERED: &str = "filtered";

fn within(t: f64, times: &[f64], reach: f64) -> bool {
    times.iter().any(|&e| (e - t).abs() <= reach + 1e-9)
}

/// #293 (B8): how far (ms) a hit's attack time ([`attack_ms`]) may differ from a Forget mark's and still go on a
/// Forget-only track. Bleed of the marked drum arrives smeared; a real stroke on its own mic is sharp (the physical
/// separator, not the level: bleed sits at the mark's level, the track's real hits lie above it).
pub const FORGET_ATTACK_MS: f64 = 2.0;

/// #293 (B8): the part of a mark's attack time that the reach adds on top of [`FORGET_ATTACK_MS`] when that is more.
pub const FORGET_ATTACK_FRAC: f64 = 0.5;

/// #293 (B8): the attack reach (ms) around an attack time `a`: `max(FORGET_ATTACK_MS, FORGET_ATTACK_FRAC * a)`.
fn attack_reach(a: f64) -> f64 {
    FORGET_ATTACK_MS.max(FORGET_ATTACK_FRAC * a)
}

/// #293 (B8): the attack gate of a Forget-only track. A hit that the score marked (`out`) stays marked only when its
/// attack time (`attacks`, ms, per hit) lies within [`attack_reach`] of the span of the marks' attack times (`marks_ms`,
/// ms, widened by [`attack_reach`] at each end). A hit the "not" examples marked themselves (`by_not`) always goes.
pub fn forget_attack_gate(out: &mut [bool], attacks: &[f64], marks_ms: &[f64], by_not: &[bool]) {
    let (lo, hi) = marks_ms
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &m| (lo.min(m), hi.max(m)));
    for (i, o) in out.iter_mut().enumerate() {
        let inside = attacks[i] >= lo - attack_reach(lo) && attacks[i] <= hi + attack_reach(hi);
        *o = *o && (by_not[i] || inside);
    }
}

/// #293 (B8): per candidate time, true when the attack gate of a Forget-only track keeps it out of the grey (its attack
/// lies off the marks' span, and it is no "not" example). The score and the threshold are the caller's: this is the
/// gate alone, so the Learn drag preview and the stored filter read one decision. All false unless the track has
/// "not" examples and no "this" example, the only case the gate runs in [`learn_pick`].
pub fn attack_gate_kept(x: &[f32], sr: u32, times: &[f64], this: &[f64], not: &[f64]) -> Vec<bool> {
    if !this.is_empty() || not.is_empty() {
        return vec![false; times.len()];
    }
    let det: Vec<(f64, u8)> = times.iter().map(|&t| (t, 0)).collect();
    let marks_ms: Vec<f64> = not.iter().map(|&n| attack_ms(x, n, sr)).collect();
    let attacks: Vec<f64> = times.iter().map(|&t| attack_ms(x, t, sr)).collect();
    let by_not = mark_out(&det, &[], &[], 0.0, &[], not, &[]);
    let mut out = vec![true; times.len()];
    forget_attack_gate(&mut out, &attacks, &marks_ms, &by_not);
    out.iter().map(|&o| !o).collect()
}

/// #293 (B8) diagnosis: the attack times behind [`attack_gate_kept`]. `attacks` per candidate (ms), `marks_ms` per
/// "not" example (ms), `window` (ms, lo and hi) the span an attack must lie in to pass the gate (the marks' span
/// widened by [`attack_reach`] at each end). Read-only: the decision stays in [`attack_gate_kept`].
#[derive(Debug)]
pub struct AttackProfile {
    pub attacks: Vec<f64>,
    pub marks_ms: Vec<f64>,
    pub window: (f64, f64),
}

/// The [`AttackProfile`] of a Forget-only track (`not` non-empty: the window is the marks' span, see
/// [`forget_attack_gate`]).
pub fn attack_profile(x: &[f32], sr: u32, times: &[f64], not: &[f64]) -> AttackProfile {
    let attacks = times.iter().map(|&t| attack_ms(x, t, sr)).collect();
    let marks_ms: Vec<f64> = not.iter().map(|&n| attack_ms(x, n, sr)).collect();
    let (lo, hi) = marks_ms
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &m| (lo.min(m), hi.max(m)));
    let window = (lo - attack_reach(lo), hi + attack_reach(hi));
    AttackProfile {
        attacks,
        marks_ms,
        window,
    }
}

/// The out mark per detected hit `(time, note)` of the normal pick: true when the score of the
/// hit's candidate (`frames`, `scores`: the arrays of [`crate::learn_score::score_all`]) is under
/// `threshold`, unless a "this" example is within [`REACH`] or an edited hit within
/// [`EDITED_REACH`]; and true for the hit nearest to each "not" example within [`REACH`]. A hit
/// without a candidate (no score) stays in.
pub fn mark_out(
    det: &[(f64, u8)],
    frames: &[u32],
    scores: &[f32],
    threshold: f32,
    this: &[f64],
    not: &[f64],
    edited: &[f64],
) -> Vec<bool> {
    let mut out: Vec<bool> = det
        .iter()
        .map(|&(t, _)| {
            let forced = within(t, this, REACH) || within(t, edited, EDITED_REACH);
            let score = crate::learn_score::nearest_frame(frames, t).and_then(|i| scores.get(i).copied());
            !forced && score.is_some_and(|s| s < threshold)
        })
        .collect();
    for &n in not {
        let near = det
            .iter()
            .enumerate()
            .filter(|(_, (t, _))| (t - n).abs() <= REACH + 1e-9)
            .min_by(|a, b| (a.1.0 - n).abs().total_cmp(&(b.1.0 - n).abs()))
            .map(|(i, _)| i);
        if let Some(i) = near {
            out[i] = true;
        }
    }
    out
}

/// The detected hits of the track's role (and their roll flags): the role filter of
/// `analyze::analyze_flags`, applied before the examples look at the hits.
pub fn role_hits(det: &[(f64, u8)], roll: &[bool], role: &str) -> (Vec<(f64, u8)>, Vec<bool>) {
    let classes = drumspike_core::analyze::role_classes(role);
    let keep = |n: u8| role == "other" || classes.contains(&n);
    let mut d = Vec::with_capacity(det.len());
    let mut r = Vec::new();
    for (i, &(t, n)) in det.iter().enumerate() {
        if keep(n) {
            d.push((t, n));
            if !roll.is_empty() {
                r.push(roll.get(i).copied().unwrap_or(false));
            }
        }
    }
    (d, r)
}

/// The two analyses of a Learn pick ([`learn_pick`]): the normal pick with the filter's marks and the Add
/// rule's hits, and the forced "this" hits.
#[derive(Debug, Clone)]
pub struct LearnPick {
    /// The normal pick.
    pub base: Analysis,
    /// The "this" examples the Sensitivity did not give (forced hits).
    pub forced: Analysis,
}

/// #293: the Learn pick of a track on the DSP candidates. `detection` is the kept candidate list with its
/// detection parameters and weak-flux limit; `rows` its candidate rows ([`dsp_detection_candidates`]).
/// The detector's hits come at the Sensitivity `scale` ([`Detector::Candidates`]), the filter marks them
/// when it is on ([`mark_out`]), the Add rule adds the candidates below the Sensitivity that look like a
/// "this" example, and the forced "this" hits come in on their own analysis. `dir`/`key` enable the
/// feature cache. No roll flags: the roll fill ran on the model's activations, which are gone (#281 D16).
///
/// # Errors
///
/// The text of the analysis error when the detection or either of the combined analyses is refused for the role or
/// signal. An unknown `learn.method` falls back to the default recipe (`learn: unknown method` only if that were
/// unknown too).
#[allow(clippy::too_many_arguments)]
pub fn learn_pick(
    x: &[f32],
    sr: u32,
    role: &str,
    detection: (&[Candidate], &OnsetParams, f32),
    rows: &[Value],
    learn: &Map<String, Value>,
    edited: &[f64],
    params: &AnalyzeParams,
    sens: f64,
    scale: f32,
    dir: Option<&Path>,
    key: Option<&Value>,
) -> Result<LearnPick, String> {
    let method = learn
        .get("method")
        .and_then(Value::as_str)
        .filter(|m| Recipe::parse(m).is_some())
        .unwrap_or(METHOD);
    let recipe = Recipe::parse(method).ok_or("learn: unknown method")?;
    let (cands, cparams, weak_limit) = detection;
    let frames = candidate_frames(rows);
    let dim = recipe.len();
    let compute = || -> Vec<f32> {
        let times: Vec<f64> = frames.iter().map(|&f| f64::from(f) / FPS).collect();
        similarity::candidate_features(x, sr, &times, recipe)
            .into_iter()
            .flatten()
            .collect()
    };
    let feats = match (dir, key) {
        (Some(dir), Some(key)) => {
            let k = learn_key(key, &frames, method);
            match read_learn(dir, &k, dim) {
                Some((f, feats)) if f == frames => feats,
                _ => {
                    let flat = compute();
                    let _ = write_learn(dir, &frames, &flat, dim, &k);
                    flat
                }
            }
        }
        _ => compute(),
    };
    let (this, not) = example_times(learn);
    let solid = solid_frames(&frame_values(rows, &frames), sens);
    let extra = |t: f64| similarity::candidate_features(x, sr, &[t], recipe).into_iter().next();
    let scored = score_all(
        &frames,
        &feats,
        recipe,
        &similarity::Weights::DEFAULT,
        Centre::APP,
        &solid,
        &this,
        &not,
        &extra,
    );
    let detector = Detector::Candidates {
        cands,
        params: cparams,
        weak_limit,
        scale,
    };
    let detected = detect_events(x, sr, role, &params.thresholds, detector).map_err(|e| e.to_string())?;
    // the examples see the hits of the track's role only (an other-class detection near an example is
    // dropped by the role filter later and must neither stand in for a "this" example nor take a "not")
    let (mut det, _) = role_hits(&detected, &[], role);
    // the filter cuts only when it is on; with it off the pick runs for the Add rule alone
    let mut out = if filter_on(learn) {
        let threshold = effective_threshold(learn, &scored);
        let mut out = mark_out(&det, &frames, &scored.scores, threshold, &this, &not, edited);
        if this.is_empty() && !not.is_empty() {
            // #293 (B8): Forget marks alone: a score-marked hit goes only at an attack time near the marks' attack
            let marks_ms: Vec<f64> = not.iter().map(|&n| attack_ms(x, n, sr)).collect();
            let attacks: Vec<f64> = det.iter().map(|&(t, _)| attack_ms(x, t, sr)).collect();
            let by_not = mark_out(&det, &frames, &[], threshold, &[], &not, &[]);
            forget_attack_gate(&mut out, &attacks, &marks_ms, &by_not);
        }
        out
    } else {
        vec![false; det.len()]
    };
    // the Add rule: candidates below the Sensitivity that look like a Learn This example (and no Forget
    // example near) become hits, flagged learned. They join `det` before the combined analysis so the
    // min gap applies to them like to any hit.
    let note = role_note(role);
    let adds = add_times(rows, &frames, &scored.scores, sens, add_threshold(learn), &det, &not);
    // the output events before the Add rule: an added hit that the analysis moves is matched to the nearest
    // output event that is new, so an existing hit next to it stays unflagged
    let before: Vec<f64> = if adds.is_empty() {
        Vec::new()
    } else {
        let roll = vec![false; det.len()];
        analyze_flags(x, sr, role, &det, &roll, &out, params)
            .map_err(|e| e.to_string())?
            .events
            .iter()
            .map(|h| h.t)
            .collect()
    };
    let mut added: Vec<f64> = Vec::new();
    for t in adds {
        det.push((t, note));
        out.push(false);
        added.push(t);
    }
    let roll = vec![false; det.len()];
    let mut base = analyze_flags(x, sr, role, &det, &roll, &out, params).map_err(|e| e.to_string())?;
    // the learned flags: for each added time the nearest new output event within REACH (a rule inside
    // analyze_flags that drops or moves an entry cannot shift the flags)
    if !added.is_empty() {
        let mut learned = vec![false; base.events.len()];
        for &t in &added {
            let nearest = base
                .events
                .iter()
                .enumerate()
                .filter(|(_, h)| (h.t - t).abs() <= REACH + 1e-9)
                .filter(|(_, h)| !before.iter().any(|&b| (b - h.t).abs() <= 0.001))
                .min_by(|(_, a), (_, b)| (a.t - t).abs().total_cmp(&(b.t - t).abs()));
            if let Some((i, _)) = nearest {
                learned[i] = true;
            }
        }
        if learned.iter().any(|&b| b) {
            base.learned = learned;
        }
    }
    let forced: Vec<(f64, u8)> = forced_this(&det, rows, &this).into_iter().map(|t| (t, note)).collect();
    let forced = analyze_detected(x, sr, role, &forced, params).map_err(|e| e.to_string())?;
    Ok(LearnPick { base, forced })
}

/// P4-6c: the times of the track's edited hits (added or moved: `edit::is_kept`); the pipeline never
/// filters their candidates (concept 3.6).
pub(crate) fn edited_times(events: &[Value]) -> Vec<f64> {
    events
        .iter()
        .filter(|e| e.as_object().is_some_and(is_kept))
        .filter_map(|e| e.get("t").and_then(Value::as_f64))
        .collect()
}

/// #293 (B8): the hits a stored analysis of a track with a Learn keeps, the hits a Learn change gives: the Learn
/// pick on the DSP candidates (the filter's marks, the Add rule's hits) and the forced "this" hits, without the
/// hits the engineer deleted ([`dropped_times`]). `edited`: the times of the track's edited hits, which the
/// filter does not take out. `None` when the track has no Learn. The job owns the track, so no idle check.
#[allow(clippy::too_many_arguments)]
pub(crate) fn stored_learn_events(
    op: &Shared,
    name: &str,
    x: &[f32],
    sr: u32,
    role: &str,
    detection: (&[Candidate], &OnsetParams, f32),
    params: &AnalyzeParams,
    edited: &[f64],
    dir: &Path,
    key: &Value,
) -> Result<Option<Vec<Value>>, String> {
    let Some(learn) = learn_of_track(op, name) else {
        return Ok(None);
    };
    let (cands, cparams, weak_limit) = detection;
    let rows = dsp_detection_candidates(cands, cparams, weak_limit, role);
    let shift_s = lock(op).track(name).map_or(0.0, shift_s_of);
    let dropped: Vec<f64> = dropped_times(&learn).into_iter().map(|d| d + shift_s).collect();
    let sens = sensitivity_of(&params.thresholds);
    let scale = scale_of(sens) as f32;
    let pick = learn_pick(
        x,
        sr,
        role,
        detection,
        &rows,
        &learn,
        edited,
        params,
        sens.round(),
        scale,
        Some(dir),
        Some(key),
    )?;
    let t = |e: &Value| e.get("t").and_then(Value::as_f64).unwrap_or(0.0);
    let base: Vec<Value> = analysis_values(&pick.base)
        .into_iter()
        .filter(|b| !within(t(b), &dropped, MERGE_WINDOW))
        .collect();
    let forced = analysis_values(&pick.forced)
        .into_iter()
        .filter(|f| !within(t(f), &dropped, MERGE_WINDOW) && !base.iter().any(|b| (t(b) - t(f)).abs() < MERGE_WINDOW));
    let mut out: Vec<Value> = base.iter().cloned().chain(forced).collect();
    out.sort_by(|a, b| t(a).total_cmp(&t(b)));
    Ok(Some(out))
}

/// The Add threshold of the learn object: `add`, or the default 0.85 (P4-38b; the card's strict
/// fixed value - the engineer drags the Add line in the histogram).
pub fn add_threshold(learn: &Map<String, Value>) -> f32 {
    learn
        .get("add")
        .and_then(Value::as_f64)
        .map_or(0.85, |v| v as f32)
        .clamp(0.0, 1.0)
}

/// P4-38b: the times the Add rule adds: candidates BELOW the Sensitivity (`v > sens`, a row of
/// `[t, class, strength, v]`) whose frame scores at or over `th`, with no detection hit of the
/// track within [`REACH`] and no Forget example within 50 ms. One time per candidate, sorted.
pub fn add_times(
    cands: &[Value],
    frames: &[u32],
    scores: &[f32],
    sens: f64,
    th: f32,
    det: &[(f64, u8)],
    not: &[f64],
) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for c in cands {
        let (Some(t), Some(v)) = (c.get(0).and_then(Value::as_f64), c.get(3).and_then(Value::as_f64)) else {
            continue;
        };
        if v <= sens {
            continue;
        }
        let f = crate::learn_score::nearest_frame(frames, t);
        let Some(i) = f else { continue };
        let Some(&s) = scores.get(i) else { continue };
        if f64::from(s) < f64::from(th) {
            continue;
        }
        if det.iter().any(|&(d, _)| (d - t).abs() <= REACH + 1e-9) {
            continue;
        }
        if not.iter().any(|&n| (n - t).abs() <= 0.050 + 1e-9) {
            continue;
        }
        if out.iter().any(|&o| (o - t).abs() < 1e-9) {
            continue;
        }
        out.push(t);
    }
    out.sort_by(f64::total_cmp);
    out
}

/// The "this" examples that the normal pick has no hit of the track's role for (`det` after
/// [`role_hits`]; none within [`REACH`]): the time of the nearest detection candidate within
/// [`REACH`] of each (`cands`: `[t, class, strength, slider]` rows of the role), else the
/// example's own time (a rise example), once each. So a "this" example always yields one hit.
pub fn forced_this(det: &[(f64, u8)], cands: &[Value], this: &[f64]) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for &t in this {
        if det.iter().any(|&(d, _)| (d - t).abs() <= REACH + 1e-9) {
            continue;
        }
        let c = crate::learn::nearest_candidate(cands, t).unwrap_or(t);
        if !out.iter().any(|&o| (o - c).abs() < 1e-9) {
            out.push(c);
        }
    }
    out.sort_by(f64::total_cmp);
    out
}

/// The hits after a Learn change: the track's `current` hits marked again from `base` (the normal
/// pick with the filter's marks, as events) plus the `forced` "this" hits.
///
/// * an edited hit (`edit::is_kept`) stays as it is and is never filtered;
/// * a detected hit with a `base` hit at the same time (within 1 microsecond) keeps every field
///   (its velocity, the cut and bleed marks) and takes the base hit's `filtered` mark;
/// * a detected hit with no base hit is a forced hit of an earlier step (or a hit of another
///   pick): it goes, and comes back from `forced` when its example is still there;
/// * a hit the engineer deleted stays deleted (a base hit with no current hit is not added);
/// * a forced hit with no current hit within [`MERGE_WINDOW`] comes in;
/// * P4-38d: so does a base hit of the Add rule (`"learned": true`, not filtered): K adds the look-alikes at
///   once;
/// * P4-38e: a forced or Add-rule hit within [`MERGE_WINDOW`] of a `dropped` time (the hand deletions of
///   [`note_dropped`], times after the Hitpoint shift) stays out: a deleted Learn hit stays deleted;
/// * a flam hit (P4-13 stage B, `"flam": true`) stays with every field and is filtered when its first stroke is.
pub fn refilter_merge(current: &[Value], base: &[Value], forced: &[Value], dropped: &[f64]) -> Vec<Value> {
    let t = |e: &Value| e.get("t").and_then(Value::as_f64).unwrap_or(0.0);
    let filtered = |e: &Value| e.get(FILTERED) == Some(&Value::Bool(true));
    let mut out: Vec<Value> = Vec::new();
    for e in current {
        let Some(m) = e.as_object() else { continue };
        let mut e = e.clone();
        if is_kept(m) {
            if let Some(o) = e.as_object_mut() {
                o.shift_remove(FILTERED);
            }
            out.push(e);
            continue;
        }
        // P4-13 stage B: a flam hit has no base hit (the pick has no second stroke): it stays, and follows its first
        // stroke below
        if e.get("flam") == Some(&Value::Bool(true)) {
            out.push(e);
            continue;
        }
        let Some(b) = base.iter().find(|b| (t(b) - t(&e)).abs() < 1e-6) else {
            continue;
        };
        if let Some(o) = e.as_object_mut() {
            if filtered(b) {
                o.insert(FILTERED.into(), Value::Bool(true));
            } else {
                o.shift_remove(FILTERED);
            }
        }
        out.push(e);
    }
    let learned = base
        .iter()
        .filter(|b| b.get("learned") == Some(&Value::Bool(true)) && !filtered(b));
    for f in forced.iter().chain(learned) {
        if within(t(f), dropped, MERGE_WINDOW) {
            continue;
        }
        if !out.iter().any(|e| (t(e) - t(f)).abs() < MERGE_WINDOW) {
            out.push(f.clone());
        }
    }
    out.sort_by(|a, b| t(a).total_cmp(&t(b)));
    follow_first_stroke(&mut out);
    out
}

/// P4-38e: the key of the learn object with the times of the Learn hits the engineer deleted (seconds, before the
/// Hitpoint shift, as the example times).
pub const DROPPED: &str = "dropped";

/// The `dropped` times of a learn object (none when the key is missing).
pub fn dropped_times(learn: &Map<String, Value>) -> Vec<f64> {
    learn
        .get(DROPPED)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect()
}

/// P4-38e: keep `learn.dropped` up to date after a hand edit of the hits from `old` to `new` (`shift_s`: the Hitpoint
/// shift in seconds). A Learn hit of `old` (the Add rule's `"learned": true`, or a hit within [`REACH`] of a "this"
/// example: a forced hit, or the hit a K added) that has no hit of `new` within [`MERGE_WINDOW`] is a deletion: its time
/// goes in. A dropped time with a hit of `new` within [`MERGE_WINDOW`] goes out (the undo of the delete). The key is
/// removed when the list is empty. True when the list changed.
pub fn note_dropped(learn: &mut Map<String, Value>, old: &[Value], new: &[Value], shift_s: f64) -> bool {
    let t = |e: &Value| e.get("t").and_then(Value::as_f64).map(|t| t - shift_s);
    let new_times: Vec<f64> = new.iter().filter_map(t).collect();
    let (this, _) = crate::learn_score::example_times(learn);
    let before = dropped_times(learn);
    let mut list: Vec<f64> = before
        .iter()
        .copied()
        .filter(|&d| !within(d, &new_times, MERGE_WINDOW))
        .collect();
    for e in old {
        let (Some(m), Some(x)) = (e.as_object(), t(e)) else {
            continue;
        };
        let learn_hit = m.get("learned") == Some(&Value::Bool(true)) || within(x, &this, REACH);
        if !learn_hit || within(x, &new_times, MERGE_WINDOW) || within(x, &list, 1e-6) {
            continue;
        }
        list.push(x);
    }
    list.sort_by(f64::total_cmp);
    if list == before {
        return false;
    }
    if list.is_empty() {
        learn.shift_remove(DROPPED);
    } else {
        learn.insert(DROPPED.into(), Value::from(list));
    }
    true
}

/// P4-38e: take the `dropped` times within [`REACH`] of `this` (Learn This example times, before the Hitpoint shift)
/// out: K on a spot of a deleted Learn hit says the engineer wants it again. The key is removed when the list is
/// empty. True when the list changed.
pub fn undrop_at(learn: &mut Map<String, Value>, this: &[f64]) -> bool {
    let before = dropped_times(learn);
    let list: Vec<f64> = before.iter().copied().filter(|&d| !within(d, this, REACH)).collect();
    if list.len() == before.len() {
        return false;
    }
    if list.is_empty() {
        learn.shift_remove(DROPPED);
    } else {
        learn.insert(DROPPED.into(), Value::from(list));
    }
    true
}

/// How far after its first stroke a flam hit lies at most (the rule's `max_gap_s`, 50 ms).
const FLAM_REACH: f64 = 0.050;

/// A flam hit (the second stroke of a hit, `"flam": true`, not edited) is out of the output exactly when its first
/// stroke, the hit before it within [`FLAM_REACH`], is filtered; a flam hit with no first stroke stays in.
fn follow_first_stroke(hits: &mut [Value]) {
    let t = |e: &Value| e.get("t").and_then(Value::as_f64).unwrap_or(0.0);
    let on = |e: &Value, k: &str| e.get(k) == Some(&Value::Bool(true));
    for i in 0..hits.len() {
        if !on(&hits[i], "flam") || hits[i].as_object().is_some_and(is_kept) {
            continue;
        }
        let first_out = hits[..i]
            .iter()
            .rev()
            .find(|h| !on(h, "flam"))
            .filter(|h| t(&hits[i]) - t(h) <= FLAM_REACH)
            .is_some_and(|h| on(h, FILTERED));
        if let Some(o) = hits[i].as_object_mut() {
            if first_out {
                o.insert(FILTERED.into(), Value::Bool(true));
            } else {
                o.shift_remove(FILTERED);
            }
        }
    }
}

/// The threshold of a `learn` object: the stored one, or [`DEFAULT_THRESHOLD`] when it has
/// none (`learn.threshold` missing).
pub fn threshold_of(learn: &serde_json::Map<String, Value>) -> f32 {
    learn
        .get("threshold")
        .and_then(Value::as_f64)
        .map_or(DEFAULT_THRESHOLD, |v| v as f32)
}

#[cfg(test)]
mod tests;
