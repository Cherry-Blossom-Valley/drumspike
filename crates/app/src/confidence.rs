//! P4 #150: the confidence of each hit for the review queue (workflow polish rank 2). The page steps
//! through the doubtful hits, lowest confidence first, and the info line says why a hit is doubtful.
//! The value is computed when the page asks; it is never stored in the events, and the detection
//! results do not change.
//!
//! * `track_confidence {sid, name, events?}`: `{confidence: [{c, doubt, why}], source, doubt_below}`,
//!   one entry per hit of `events` (the page's list, else the stored one), in their order.
//!
//! The confidence `c` (0 to 1) of a hit:
//!
//! | Hit | `c` |
//! |---|---|
//! | added or moved by hand | 1 (the user decided) |
//! | out of the output (`cut`, `bleed`, `filtered`) | 1, not doubtful (it is not in the output) |
//! | with kept activations (a model analysis) | the tag cap ([`tag_cap`]; 1 for an untagged hit) x margin |
//! | without (the DSP fallback, no analysis since P4-3) | the tag cap x level |
//!
//! On a model track the level is only a reason, never a factor: ghost notes are real hits 20 to 30 dB under the
//! median, and the model is sure of them (coordinator, #150).
//!
//! * margin ([`margin_factor`]): only with kept activations (a model analysis, [`crate::activation`]). The strongest
//!   activation of the track's role classes from [`WIN_BEFORE`] frames before the hit to [`WIN_AFTER`] frames after it
//!   (the refinement moves a hit earlier than its activation peak), against the track's threshold of that class:
//!   0 at the threshold, 1 at [`MARGIN_FULL`] or more over it.
//! * level ([`level_factor`]): the peak level of the hit (`velocity::peak_db`) against the median of the track's hits
//!   in the output: 1 at the median or louder, 0 at [`LEVEL_SPAN_DB`] under it.
//!
//! A hit is doubtful when `c` is under [`DOUBT_BELOW`]. `why` names the tag, a margin under [`DOUBT_BELOW`] and a
//! level under [`DOUBT_BELOW`] of a doubtful hit.
//!
//! Detector #8 package 4: the detector's own gate verdict names itself too. The stored
//! candidates of the analysis ([`crate::candidates`]) hold the gate that kept each hit's
//! candidate ([`Gate`], A4); a hit the detector kept only through a rescue says which one —
//! the soft gate, the shape rescue, a weak flux saved by the time-domain attack — whether or
//! not it is doubtful. A hit at the flux threshold (the level gate) adds no line: its only
//! evidence is the level, which `why` names already when it is doubtful. A track without a
//! current candidate list (no analysis yet, stale after a relink) rates by the level alone.
//!
//! # Time coordinates (R13 #237)
//!
//! A hit lives in four time coordinates:
//!
//! * audio: seconds in the decoded source. The samples ([`peak_db`]) and the model input live here.
//! * detector: the activation frame grid, `t * peaks::FPS` frames per second. The same timeline as
//!   audio, counted in frames ([`hit_strength`]).
//! * shifted event: the stored hit `t` (`track_shift_set` moved the hits, `orig` with them), audio
//!   plus the recorded shift.
//! * sampler-compensated: the export coordinate, the aligned attack minus the sampler offset
//!   (`sampler::midi_events_shifted`). Not read here.
//!
//! `track_confidence` reads the loudness of the audio: the peak level of every hit at its attack
//! on the audio timeline (#281 D16: the activation margin of the model path moved to
//! the model branch; the review queue orders by the level evidence alone).

use drumspike_core::detect::N_CLASSES;
use drumspike_core::detect::onset::{Candidate, Gate};
use drumspike_core::detect::peaks;
use drumspike_core::velocity::peak_db;
use serde_json::{Value, json};

use crate::Service;
use crate::error::AppError;
use crate::export::hit_times;
use crate::media::mix_mode;
use crate::registry::resolve_shared;

/// The commands of this card.
pub const CONFIDENCE_COMMANDS: [&str; 1] = ["track_confidence"];

/// A hit under this confidence is doubtful.
pub const DOUBT_BELOW: f64 = 0.5;
/// The activation over the threshold that gives a margin of 1.
pub const MARGIN_FULL: f64 = 0.3;
/// The level under the track median (dB) that gives a level of 0.
pub const LEVEL_SPAN_DB: f64 = 24.0;
/// Activation frames before the hit frame that the margin reads.
pub const WIN_BEFORE: usize = 2;
/// Activation frames after the hit frame that the margin reads.
pub const WIN_AFTER: usize = 5;

/// The margin of an activation over the threshold of its class: 0 at or under the threshold, 1 at
/// [`MARGIN_FULL`] or more over it, linear between.
pub fn margin_factor(strength: f64, threshold: f64) -> f64 {
    ((strength - threshold) / MARGIN_FULL).clamp(0.0, 1.0)
}

/// The level of a hit against the track median: 1 at the median or louder, 0 at [`LEVEL_SPAN_DB`]
/// or more under it, linear between.
pub fn level_factor(level_db: f64, median_db: f64) -> f64 {
    (1.0 + (level_db - median_db) / LEVEL_SPAN_DB).clamp(0.0, 1.0)
}

/// The median of `v` (the mean of the two middle values for an even count), `None` when empty.
pub fn median(v: &[f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let m = s.len() / 2;
    Some(if s.len() % 2 == 1 {
        s[m]
    } else {
        (s[m - 1] + s[m]) / 2.0
    })
}

/// The cap and the reason of a hit that a rule added under the model threshold, `None` for an
/// untagged hit. The lowest cap wins when a hit has more than one tag.
pub fn tag_cap(e: &Value) -> Option<(f64, &'static str)> {
    const TAGS: [(&str, f64, &str); 5] = [
        ("rescue", 0.3, "kick rescue (no model hit)"),
        ("learned", 0.4, "Learn Add (under the Sensitivity)"),
        ("roll", 0.4, "roll fill"),
        ("flam", 0.5, "flam second stroke"),
        ("stage8", 0.3, "stage 8: real tom stroke the reference lacks"),
    ];
    TAGS.iter()
        .filter(|(k, _, _)| e.get(*k) == Some(&Value::Bool(true)))
        .map(|&(_, cap, why)| (cap, why))
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// True for a hit the user added or moved from its detected time.
pub fn is_edited(e: &Value) -> bool {
    if e.get("added") == Some(&Value::Bool(true)) {
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

/// True for a hit that is not in the output (the loudness cut, the bleed rule, the Learn filter).
/// R16 (#40): the one predicate is [`crate::policy::is_out`].
pub fn is_out(e: &Value) -> bool {
    crate::policy::is_out(e)
}

/// The strongest activation of `classes` near time `t` and its class: the frames from
/// [`WIN_BEFORE`] before to [`WIN_AFTER`] after the hit frame. `act` is (frames, [`N_CLASSES`])
/// row-major. `t` is a detector time on the audio timeline (see the module coordinates): the
/// frames of the activations sit there, so a shifted event time must be converted first.
/// `None` when the window holds no frame or `classes` is empty.
///
/// # Panics
///
/// Panics (slice indexing of `act`) when a class in `classes` is not below [`N_CLASSES`].
pub fn hit_strength(act: &[f32], classes: &[usize], t: f64) -> Option<(f64, usize)> {
    let frames = act.len() / N_CLASSES;
    let f = (t * peaks::FPS).round().max(0.0) as usize;
    let lo = f.saturating_sub(WIN_BEFORE);
    let hi = (f + WIN_AFTER).min(frames.checked_sub(1)?);
    let mut best: Option<(f64, usize)> = None;
    for fr in lo..=hi {
        for &c in classes {
            let s = f64::from(act[fr * N_CLASSES + c]);
            if best.is_none_or(|b| s > b.0) {
                best = Some((s, c));
            }
        }
    }
    best
}

/// The model margin of one hit, as [`rate`] takes it: the margin factor and the strength and
/// threshold for the reason.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Margin {
    /// [`margin_factor`] of the strength.
    pub factor: f64,
    /// The strongest activation near the hit.
    pub strength: f64,
    /// The threshold of its class.
    pub threshold: f64,
}

/// The rating of one hit: `{c, doubt, why}` (see the module table). `level_db` is the hit's peak
/// level, `median_db` the track median (`None`: no level factor), `margin` the model margin
/// (`None`: no kept activations), `gate` the verdict of the hit's candidate (`None`: no current
/// candidate list, or no candidate of the hit in it).
pub fn rate(e: &Value, level_db: f64, median_db: Option<f64>, margin: Option<Margin>, gate: Option<Gate>) -> Value {
    let r2 = |x: f64| (x * 1000.0).round() / 1000.0;
    if is_edited(e) {
        return json!({"c": 1.0, "doubt": false, "why": ["edited by hand"]});
    }
    if is_out(e) {
        return json!({"c": 1.0, "doubt": false, "why": ["not in the output"]});
    }
    let level = median_db.map_or(1.0, |m| level_factor(level_db, m));
    let mut why: Vec<String> = Vec::new();
    let cap = tag_cap(e).map_or(1.0, |(cap, tag)| {
        why.push(tag.to_string());
        cap
    });
    // the gate that kept the hit's candidate names itself (Detector #8 package 4): a hit kept
    // only by a rescue is doubtful for the rescue's reason, not (only) the level's
    match gate {
        Some(Gate::Soft) => why.push("soft gate: ghost note below the level gate".into()),
        Some(Gate::Shape) => why.push("shape rescue below the level gate".into()),
        Some(Gate::WeakAttack) => why.push("weak flux, kept by the time-domain attack".into()),
        _ => {}
    }
    // the margin when the model rated the hit; the level only without one
    let c = cap * margin.map_or(level, |m| m.factor);
    let doubt = c < DOUBT_BELOW;
    if let Some(mg) = margin.filter(|m| m.factor < DOUBT_BELOW) {
        why.push(format!(
            "weak model peak: {:.2} for a threshold of {:.2}",
            mg.strength, mg.threshold
        ));
    }
    if let Some(m) = median_db.filter(|_| doubt && level < DOUBT_BELOW) {
        why.push(format!("{:.0} dB under the track median", m - level_db));
    }
    if doubt && why.is_empty() {
        why.push("low confidence".into());
    }
    json!({"c": r2(c), "doubt": doubt, "why": why})
}

impl Service {
    /// `track_confidence {sid, name, events?}`: `{confidence, source, doubt_below}`. `confidence`
    /// has one [`rate`] entry per hit of `events` (the page's list, else the stored one), in their
    /// order. `source` is `model` (kept activations: margin and level), `level` (no kept
    /// activations: the level only) or `follows` (a follower: its hits are the source's, none is
    /// doubtful). 409 for an offline track.
    ///
    /// The `events` times are the shifted-event coordinate (`track_shift_set` moved the stored
    /// hits, the audio did not move). Every read below converts the time to detector time first:
    /// the recorded shift comes off before the activations (`hit_strength`, the frame grid) and
    /// before the peak level (`peak_db`, the audio samples) — see the module coordinates. The
    /// gate verdicts come from the stored candidates of the current analysis ([`hit_gates`]).
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] for an unknown `sid` or track; a 409 for an offline track (its source cannot be
    /// resolved); a 400 when an event has no numeric `t`; the decode error when the audio cannot be read.
    pub fn track_confidence(&self, sid: &str, name: &str, events: Option<&[Value]>) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        resolve_shared(&op);
        let st = self.track_state(&op, name)?;
        let events = match events {
            Some(e) => e.to_vec(),
            None => self.events_of(sid, name)?,
        };
        if st.follows.is_some() {
            let one = json!({"c": 1.0, "doubt": false, "why": ["follows another track"]});
            return Ok(json!({"confidence": vec![one; events.len()], "source": "follows", "doubt_below": DOUBT_BELOW}));
        }
        let times = hit_times(&events)?;
        let source = st.source.clone()?;
        let d = self.inner.media.decode_cached(&source, mix_mode(&st.mix))?;
        // the hits carry the Hitpoint shift; the audio does not (module coordinates): the
        // recorded shift comes off before the peak level
        let dt = st
            .params
            .as_ref()
            .and_then(|p| p.get("shift_samples"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            / f64::from(d.sr);
        let levels: Vec<f64> = times.iter().map(|&t| peak_db(&d.samples, t - dt, d.sr)).collect();
        let kept: Vec<f64> = events
            .iter()
            .zip(&levels)
            .filter(|(e, _)| !is_out(e))
            .map(|(_, &l)| l)
            .collect();
        let med = median(&kept);
        let gates = hit_gates(&st, &times, dt);
        let confidence: Vec<Value> = events
            .iter()
            .zip(times.iter().zip(&levels))
            .zip(&gates)
            .map(|((e, (_, &l)), g)| rate(e, l, med, None, *g))
            .collect();
        let source = "level";
        Ok(json!({"confidence": confidence, "source": source, "doubt_below": DOUBT_BELOW}))
    }
}

/// The gate verdict of each hit's candidate (Detector #8 package 4), `None` where the stored
/// candidate list has none for it: no analysis yet, a stale list, or the hit is not in it.
/// A hit's candidate is the kept one within the refinement window of its detector time (`t`
/// minus the shift): the refinement moves a hit at most 12 ms after (2 ms before) its frame
/// ([`drumspike_core::refine::RefineWindow::DEFAULT`]) and kept candidates sit at least 20 ms
/// apart (the minimum gap), so the window holds the hit's own candidate only.
fn hit_gates(st: &crate::histogram::TrackState, times: &[f64], shift_s: f64) -> Vec<Option<Gate>> {
    let mut out = vec![None; times.len()];
    let Some((cands, _, _)) = crate::candidates::read_candidates(
        &st.dir,
        &crate::candidates::candidates_key(&st.key, &st.mix, &Value::Null),
    ) else {
        return out;
    };
    for (i, &t) in times.iter().enumerate() {
        let td = t - shift_s;
        let mut best: Option<(&Candidate, f64)> = None;
        for c in &cands {
            if !c.kept {
                continue;
            }
            let d = td - c.time;
            if !(-0.002..=0.012).contains(&d) {
                continue;
            }
            if best.is_none_or(|(_, bd)| d.abs() < bd) {
                best = Some((c, d.abs()));
            }
        }
        out[i] = best.map(|(c, _)| c.gate);
    }
    out
}

/// The commands of this module for the one dispatcher ([`crate::call`]); `None` for another command.
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if !CONFIDENCE_COMMANDS.contains(&cmd) {
        return None;
    }
    let s = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{k}` (string)")))
    };
    let run = || {
        svc.track_confidence(
            s("sid")?,
            s("name")?,
            args.get("events").and_then(Value::as_array).map(Vec::as_slice),
        )
    };
    Some(run())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_factor_is_0_at_the_threshold_and_1_at_the_full_margin() {
        assert_eq!(margin_factor(0.3, 0.3), 0.0);
        assert_eq!(margin_factor(0.1, 0.3), 0.0);
        assert!((margin_factor(0.45, 0.3) - 0.5).abs() < 1e-9);
        assert_eq!(margin_factor(0.6, 0.3), 1.0);
        assert_eq!(margin_factor(0.99, 0.3), 1.0);
    }

    #[test]
    fn level_factor_is_1_at_the_median_and_0_at_the_span_under_it() {
        assert_eq!(level_factor(-10.0, -10.0), 1.0);
        assert_eq!(level_factor(-3.0, -10.0), 1.0);
        assert!((level_factor(-22.0, -10.0) - 0.5).abs() < 1e-9);
        assert_eq!(level_factor(-34.0, -10.0), 0.0);
        assert_eq!(level_factor(-60.0, -10.0), 0.0);
    }

    #[test]
    fn median_of_odd_even_and_empty_lists() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn tag_cap_takes_the_lowest_cap_of_the_tags() {
        assert_eq!(tag_cap(&json!({"t": 1.0})), None);
        assert_eq!(tag_cap(&json!({"t": 1.0, "rescue": false})), None);
        assert_eq!(
            tag_cap(&json!({"t": 1.0, "flam": true})),
            Some((0.5, "flam second stroke"))
        );
        assert_eq!(tag_cap(&json!({"t": 1.0, "roll": true})), Some((0.4, "roll fill")));
        assert_eq!(
            tag_cap(&json!({"t": 1.0, "stage8": true})),
            Some((0.3, "stage 8: real tom stroke the reference lacks"))
        );
        assert_eq!(tag_cap(&json!({"t": 1.0, "learned": true})).unwrap().0, 0.4);
        assert_eq!(
            tag_cap(&json!({"t": 1.0, "flam": true, "rescue": true})),
            Some((0.3, "kick rescue (no model hit)"))
        );
    }

    #[test]
    fn is_edited_and_is_out_read_the_marks() {
        assert!(is_edited(&json!({"t": 1.0, "added": true})));
        assert!(is_edited(&json!({"t": 1.0, "orig": 0.9})));
        assert!(!is_edited(&json!({"t": 1.0, "orig": 1.0})));
        assert!(!is_edited(&json!({"t": 1.0})));
        for k in ["cut", "bleed", "filtered"] {
            assert!(is_out(&json!({"t": 1.0, k: true})), "{k}");
            assert!(!is_out(&json!({"t": 1.0, k: false})), "{k}");
        }
        assert!(!is_out(&json!({"t": 1.0})));
    }

    #[test]
    fn hit_strength_reads_the_window_around_the_hit_frame_of_the_classes() {
        let mut act = vec![0.0f32; 100 * N_CLASSES];
        act[52 * N_CLASSES] = 0.7; // class 0, 2 frames after frame 50
        act[50 * N_CLASSES + 1] = 0.9; // class 1, not asked
        act[60 * N_CLASSES] = 0.95; // class 0, outside the window
        assert_eq!(hit_strength(&act, &[0], 0.5), Some((f64::from(0.7f32), 0)));
        assert_eq!(hit_strength(&act, &[0, 1], 0.5), Some((f64::from(0.9f32), 1)));
        assert_eq!(hit_strength(&act, &[0], 0.56).unwrap().1, 0); // frame 56: window 54..61 holds frame 60
        assert_eq!(hit_strength(&act, &[0], 0.56).unwrap().0, f64::from(0.95f32));
        assert_eq!(hit_strength(&act, &[], 0.5), None);
        assert_eq!(hit_strength(&[], &[0], 0.5), None);
        // a hit near the end reads the frames it has; a hit after the activations has no margin
        assert_eq!(hit_strength(&act, &[0], 0.98), Some((0.0, 0)));
        assert_eq!(hit_strength(&act, &[0], 5.0), None);
    }

    #[test]
    fn rate_follows_the_module_table() {
        let strong = Some(Margin {
            factor: 1.0,
            strength: 0.9,
            threshold: 0.3,
        });
        let weak = Some(Margin {
            factor: 0.2,
            strength: 0.36,
            threshold: 0.3,
        });
        // untagged at the median with a strong peak: confident
        let r = rate(&json!({"t": 1.0}), -10.0, Some(-10.0), strong, None);
        assert_eq!((r["c"].as_f64(), r["doubt"].as_bool()), (Some(1.0), Some(false)));
        assert_eq!(r["why"], json!([]));
        // a weak model peak: doubtful, and the reason names it
        let r = rate(&json!({"t": 1.0}), -10.0, Some(-10.0), weak, None);
        assert_eq!((r["c"].as_f64(), r["doubt"].as_bool()), (Some(0.2), Some(true)));
        assert_eq!(r["why"], json!(["weak model peak: 0.36 for a threshold of 0.30"]));
        // 18 dB under the median, no activations: level 0.25
        let r = rate(&json!({"t": 1.0}), -28.0, Some(-10.0), None, None);
        assert_eq!(r["c"].as_f64(), Some(0.25));
        assert_eq!(r["why"], json!(["18 dB under the track median"]));
        // a quiet hit (30 dB under the median, a ghost note) with a strong model peak is not doubtful
        let r = rate(&json!({"t": 1.0}), -40.0, Some(-10.0), strong, None);
        assert_eq!((r["c"].as_f64(), r["doubt"].as_bool()), (Some(1.0), Some(false)));
        assert_eq!(r["why"], json!([]));
        // a weak peak on a quiet hit: the margin rates it, the level is a reason
        let r = rate(&json!({"t": 1.0}), -28.0, Some(-10.0), weak, None);
        assert_eq!(r["c"].as_f64(), Some(0.2));
        assert_eq!(
            r["why"],
            json!([
                "weak model peak: 0.36 for a threshold of 0.30",
                "18 dB under the track median"
            ])
        );
        // a rescue hit without activations: the cap times the level
        let r = rate(&json!({"t": 1.0, "rescue": true}), -16.0, Some(-10.0), None, None);
        assert_eq!(r["c"].as_f64(), Some(0.225));
        assert_eq!(r["why"], json!(["kick rescue (no model hit)"]));
        // and with activations: the cap times the margin
        let r = rate(&json!({"t": 1.0, "flam": true}), -10.0, Some(-10.0), weak, None);
        assert_eq!(r["c"].as_f64(), Some(0.1));
        assert_eq!(
            r["why"],
            json!(["flam second stroke", "weak model peak: 0.36 for a threshold of 0.30"])
        );
        let r = rate(&json!({"t": 1.0, "flam": true}), -10.0, Some(-10.0), strong, None);
        assert_eq!((r["c"].as_f64(), r["doubt"].as_bool()), (Some(0.5), Some(false)));
        // an edited hit and a hit out of the output are never doubtful
        let r = rate(
            &json!({"t": 1.0, "added": true, "rescue": true}),
            -60.0,
            Some(-10.0),
            weak,
            None,
        );
        assert_eq!((r["c"].as_f64(), r["doubt"].as_bool()), (Some(1.0), Some(false)));
        let r = rate(&json!({"t": 1.0, "cut": true}), -60.0, Some(-10.0), weak, None);
        assert_eq!(r["why"], json!(["not in the output"]));
        assert_eq!(r["doubt"], json!(false));
        // no median (no hit in the output): the level factor is 1
        let r = rate(&json!({"t": 1.0}), -60.0, None, None, None);
        assert_eq!(r["c"].as_f64(), Some(1.0));
    }

    #[test]
    fn the_candidate_gate_names_itself_in_why() {
        // a soft-rescued ghost note: the why names the soft gate, then the level evidence
        let r = rate(&json!({"t": 1.0}), -28.0, Some(-10.0), None, Some(Gate::Soft));
        assert_eq!(r["c"].as_f64(), Some(0.25));
        assert_eq!(r["doubt"], json!(true));
        assert_eq!(
            r["why"],
            json!([
                "soft gate: ghost note below the level gate",
                "18 dB under the track median"
            ])
        );
        // a shape rescue names itself too
        let r = rate(&json!({"t": 1.0}), -10.0, Some(-10.0), None, Some(Gate::Shape));
        assert_eq!(r["why"], json!(["shape rescue below the level gate"]));
        assert_eq!(r["doubt"], json!(false));
        // a weak loud hit kept by its attack
        let r = rate(&json!({"t": 1.0}), -10.0, Some(-10.0), None, Some(Gate::WeakAttack));
        assert_eq!(r["why"], json!(["weak flux, kept by the time-domain attack"]));
        // a loud hit at the flux threshold (the level gate) and no verdict add no line
        let r = rate(&json!({"t": 1.0}), -10.0, Some(-10.0), None, Some(Gate::Level));
        assert_eq!((r["c"].as_f64(), r["doubt"].as_bool()), (Some(1.0), Some(false)));
        assert_eq!(r["why"], json!([]));
        let r = rate(&json!({"t": 1.0}), -28.0, Some(-10.0), None, None);
        assert_eq!(r["why"], json!(["18 dB under the track median"]));
    }

    #[test]
    fn dispatch_answers_only_its_command() {
        assert_eq!(CONFIDENCE_COMMANDS, ["track_confidence"]);
        let svc = Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        assert!(dispatch(&svc, "markers_get", &json!({})).is_none());
        assert!(dispatch(&svc, "track_confidence", &json!({})).unwrap().is_err());
        assert!(
            dispatch(&svc, "track_confidence", &json!({"sid": "x", "name": "y"}))
                .unwrap()
                .is_err()
        );
    }
}
