//! The analysis pipeline of one track after the decode: `engine.analyze_track` from the
//! detector to the events (P3-2a).
//!
//! The steps, in the order and with the arithmetic of the Python engine:
//!
//! 1. Detection ([`detect_events`]): the DSP detector ([`crate::detect::onset`]) whose hits
//!    carry the note of the track's role ([`role_note`]). Each hit is `(time, note)`.
//!    (P4-19 note: the model path and its roll fill moved to the model branch;
//!    main is model-free, #281 D16.)
//! 2. The role filter ([`role_classes`]): every role except `other` keeps only its notes.
//! 3. Sort of the `(time, note)` pairs, then the min-gap dedup on the times.
//! 4. The refinement ([`crate::refine::refine_track`], the previous and the next coarse hit
//!    as neighbours), then a stable sort by time and a second min-gap dedup. With
//!    `refine` off the coarse times stay and no event has a `rise`. The rule
//!    ([`AnalyzeParams::rule`], P4-12) is the engine's (version 1, the default here and in the
//!    parity checks) or the reach rule the app runs ([`crate::refine::refine_reach`]).
//! 5. The global shift (`shift_samples / sr` seconds) and a stable sort by time.
//! 6. The velocities ([`crate::velocity::velocity_map`]) and `rise` rounded to 1 decimal
//!    with Python's `round(x, 1)` ([`crate::pyops::py_round_digits`]).
//!
//! A source rate other than 44.1, 48 or 96 kHz is an error ([`AnalyzeError::Rate`]): the
//! Python engine decodes such a file with ffmpeg first, the core does not. Parity with
//! `engine.analyze_track` (identical `t`, `v`, `rise` and order) is checked by
//! `tools/parity/check_analyze.py`; [`AnalyzeBranches`] counts which step changed
//! something, so the check can show that every branch ran.

use std::cmp::Ordering;
use std::fmt;

use crate::detect::onset::{self, OnsetError, Role, detect_onsets};
use crate::detect::{DetectError, N_CLASSES};
use crate::pyops::py_round_digits;
use crate::refine::{BackStep, FM_PEAK_DB, Hit, ReachStep, RefineRule, RefineWindow, refine_track_rule_peak};
use crate::velocity::{VelocityMap, velocity_map};

/// The source rates the core analyses (`engine.DETECT_RATES`, the rates of the resampler).
pub const DETECT_RATES: [u32; 3] = [44_100, 48_000, 96_000];

/// The parameters of one analysis (the `params` of a track in the document).
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyzeParams {
    /// Per-class thresholds BD, SD, TT, HH, CY+RD of the model's peak picking (not rounded;
    /// [`detect_events`] rounds them to 3 decimals).
    pub thresholds: [f64; N_CLASSES],
    /// Minimum gap between two hits in ms (`min_gap_ms`, default 20).
    pub min_gap_ms: f64,
    /// Refine each hit to the attack (`refine`, default on).
    pub refine: bool,
    /// The refinement's search window (`refine_lo_ms`, `refine_hi_ms`).
    pub window: RefineWindow,
    /// The refinement rule (P4-12): the engine's ([`RefineRule::Reference`], the default) or
    /// the reach rule. The app sets it per analysis; it is not a document param.
    pub rule: RefineRule,
    /// Global shift of every hit in samples (`shift_samples`, positive = later).
    pub shift_samples: i64,
    /// The dB-to-velocity map (`vel_lo_db`, `vel_hi_db`; velocities 20 to 127).
    pub velocity: VelocityMap,
    /// P4-19: the roll fill version (0 = off). The roll fill moved with the model to the
    /// model branch (#281 A3/D16); the field stays so old documents load.
    pub roll: u32,
    /// P4-31: the kick walk version (0 = the shipped back-track margin everywhere, the default here and in the parity
    /// checks; [`crate::refine::kick_walk_applies`] says for which roles). The app sets it per analysis; it is not a
    /// document param.
    pub kick_walk: u32,
}

impl Default for AnalyzeParams {
    fn default() -> Self {
        AnalyzeParams {
            thresholds: crate::detect::peaks::THRESHOLDS,
            min_gap_ms: 20.0,
            refine: true,
            window: RefineWindow::DEFAULT,
            rule: RefineRule::Reference,
            shift_samples: 0,
            velocity: VelocityMap::default(),
            roll: 0,
            kick_walk: 0,
        }
    }
}

/// The detector of an analysis: the DSP detector.
pub enum Detector<'a> {
    /// The DSP onset detector ([`crate::detect::onset::detect_onsets`]) with the role's
    /// parameters.
    Dsp,
    /// A detector backend (#363): the trait object of `crates/core/src/backend.rs` (the
    /// built-in [`crate::backend::DspBackend`] or an [`crate::backend::ExternalDetector`]).
    Backend(&'a dyn crate::backend::DetectorBackend),
    /// The kept candidate list of a DSP detection ([`crate::detect::onset::Candidate`], A4),
    /// re-judged at `scale` (1.0 = the detection's gate verdicts, the default slider value;
    /// larger is stricter, smaller looser). The times of the kept candidates, role-filtered
    /// and min-gapped like the other detectors' picks; the rest of the analysis
    /// ([`analyze_flagged`]) runs as always, so a re-pick stores what an analysis stores.
    Candidates {
        /// The candidate list of the detection.
        cands: &'a [crate::detect::onset::Candidate],
        /// The detection's parameters (`onset::OnsetParams`), for the gates.
        params: &'a crate::detect::onset::OnsetParams,
        /// The detection's weak-flux limit.
        weak_limit: f32,
        /// The slider's gate scale.
        scale: f32,
    },
}

/// Errors of [`analyze_track`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalyzeError {
    /// The source rate is not 44.1, 48 or 96 kHz.
    Rate(u32),
    /// The model chain failed.
    Detect(DetectError),
    /// The DSP fallback failed.
    Onset(OnsetError),
    /// A detector backend failed (#363).
    Backend(crate::backend::BackendError),
}

impl fmt::Display for AnalyzeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnalyzeError::Rate(sr) => write!(
                f,
                "analyze: unsupported sample rate {sr} Hz (44100, 48000 or 96000; the Python engine decodes other rates with ffmpeg)"
            ),
            AnalyzeError::Detect(e) => e.fmt(f),
            AnalyzeError::Onset(e) => e.fmt(f),
            AnalyzeError::Backend(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for AnalyzeError {}

impl From<DetectError> for AnalyzeError {
    fn from(e: DetectError) -> Self {
        AnalyzeError::Detect(e)
    }
}

impl From<OnsetError> for AnalyzeError {
    fn from(e: OnsetError) -> Self {
        AnalyzeError::Onset(e)
    }
}

impl From<crate::backend::BackendError> for AnalyzeError {
    fn from(e: crate::backend::BackendError) -> Self {
        AnalyzeError::Backend(e)
    }
}

/// How often each step of the pipeline changed something (for tests and the parity check).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnalyzeBranches {
    /// Analyses (tracks).
    pub tracks: usize,
    /// Hits the detector gave.
    pub detected: usize,
    /// Analyses where the role filter ran (every role except `other`).
    pub filter_runs: usize,
    /// Hits the role filter removed.
    pub filtered_out: usize,
    /// Hits the first min-gap dedup removed.
    pub dedup_dropped: usize,
    /// Analyses with `refine` off.
    pub refine_off: usize,
    /// Hits the refinement moved.
    pub refine_moved: usize,
    /// Refined hits without a `rise` (the refinement returned `None`).
    pub rise_none: usize,
    /// Hits the second min-gap dedup (after the refinement) removed.
    pub dedup2_dropped: usize,
    /// P4-12: hits the reach rule refined at a later attack (the reference's window held none).
    pub reach_moved: usize,
    /// P4-12: hits the reach rule measured above the previous hit's tail.
    pub tail_moved: usize,
    /// P4-12: hits whose window held no attack and whose reach found none (the coarse time stays).
    pub reach_unresolved: usize,
    /// P4-12 stage 2: hits the back-track moved to the foot of their attack.
    pub back_moved: usize,
    /// P4-12 stage 2: hits whose back-track reached its bound in a busy context (the time stays).
    pub back_busy: usize,
    /// P4-12 stage 2: hits whose long back-track crossed a separate earlier event (the time stays).
    pub back_separate: usize,
    /// Analyses with a shift other than 0.
    pub shifted: usize,
    /// Events in the result.
    pub events: usize,
}

impl AnalyzeBranches {
    /// The counters as `(name, count)` pairs in a fixed order.
    #[must_use]
    pub fn pairs(&self) -> [(&'static str, usize); 17] {
        [
            ("tracks", self.tracks),
            ("detected", self.detected),
            ("filter_runs", self.filter_runs),
            ("filtered_out", self.filtered_out),
            ("dedup_dropped", self.dedup_dropped),
            ("refine_off", self.refine_off),
            ("refine_moved", self.refine_moved),
            ("rise_none", self.rise_none),
            ("dedup2_dropped", self.dedup2_dropped),
            ("reach_moved", self.reach_moved),
            ("tail_moved", self.tail_moved),
            ("reach_unresolved", self.reach_unresolved),
            ("back_moved", self.back_moved),
            ("back_busy", self.back_busy),
            ("back_separate", self.back_separate),
            ("shifted", self.shifted),
            ("events", self.events),
        ]
    }

    /// Add the counts of `other` to these.
    pub fn add(&mut self, other: &AnalyzeBranches) {
        self.tracks += other.tracks;
        self.detected += other.detected;
        self.filter_runs += other.filter_runs;
        self.filtered_out += other.filtered_out;
        self.dedup_dropped += other.dedup_dropped;
        self.refine_off += other.refine_off;
        self.refine_moved += other.refine_moved;
        self.rise_none += other.rise_none;
        self.dedup2_dropped += other.dedup2_dropped;
        self.reach_moved += other.reach_moved;
        self.tail_moved += other.tail_moved;
        self.reach_unresolved += other.reach_unresolved;
        self.back_moved += other.back_moved;
        self.back_busy += other.back_busy;
        self.back_separate += other.back_separate;
        self.shifted += other.shifted;
        self.events += other.events;
    }
}

/// The result of [`analyze_track`]: the events (`t`, `v`, `rise` rounded to 1 decimal) in
/// time order and the branch counts.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    /// The events as the document stores them.
    pub events: Vec<Hit>,
    /// What each step changed.
    pub branches: AnalyzeBranches,
    /// P4-19: per event, true when the roll fill added it (empty: none did).
    pub roll: Vec<bool>,
    /// P4-6c: per event, true when the Learn filter takes it out of the output (empty: none).
    pub learn_out: Vec<bool>,
    /// P4-38b: per event, true when the Learn Add rule added it (a candidate below the Sensitivity
    /// that looks like a Learn This example; empty: none did).
    pub learned: Vec<bool>,
}

impl Analysis {
    /// True when the roll fill added event `i`.
    #[must_use]
    pub fn is_roll(&self, i: usize) -> bool {
        self.roll.get(i).copied().unwrap_or(false)
    }

    /// P4-6c: true when the Learn filter takes event `i` out of the output.
    #[must_use]
    pub fn is_learn_out(&self, i: usize) -> bool {
        self.learn_out.get(i).copied().unwrap_or(false)
    }

    /// P4-38b: true when the Learn Add rule added event `i`.
    #[must_use]
    pub fn is_learned(&self, i: usize) -> bool {
        self.learned.get(i).copied().unwrap_or(false)
    }
}

/// The MIDI notes a role keeps (`engine.ROLE_CLASSES`); an unknown role gets the set of
/// `other` (every note from 28 to 59).
#[must_use]
pub fn role_classes(role: &str) -> &'static [u8] {
    const OTHER: [u8; 32] = {
        let mut a = [0u8; 32];
        let mut i = 0;
        while i < 32 {
            a[i] = 28 + i as u8;
            i += 1;
        }
        a
    };
    match role {
        "kick" => &[35],
        "snare" => &[38],
        "tom" => &[41, 43, 45, 47, 48, 50],
        "hat" => &[42, 44, 46],
        "cymbal" => &[49, 51, 52, 53, 55],
        _ => &OTHER,
    }
}

/// The note of a DSP fallback hit (`engine.ROLE_NOTE`; an unknown role gets 35 like
/// `other`).
#[must_use]
pub fn role_note(role: &str) -> u8 {
    match role {
        "snare" => 38,
        "tom" => 47,
        "hat" => 42,
        "cymbal" => 49,
        _ => 35,
    }
}

/// The thresholds as the CLI receives them: `float(f"{t:.3f}")` for each.
#[must_use]
pub fn round_thresholds(t: &[f64; N_CLASSES]) -> [f64; N_CLASSES] {
    t.map(|v| py_round_digits(v, 3))
}

fn check_rate(sr: u32) -> Result<(), AnalyzeError> {
    if DETECT_RATES.contains(&sr) {
        Ok(())
    } else {
        Err(AnalyzeError::Rate(sr))
    }
}

/// Step 1: the detector's hits as `(time, note)` in the detector's order. The model gets
/// the thresholds rounded to 3 decimals; a DSP hit carries [`role_note`], and an unknown role
/// runs the DSP fallback with the parameters of `other`.
///
/// # Errors
///
/// [`AnalyzeError::Rate`] when `sr` is not 44.1, 48 or 96 kHz; [`AnalyzeError::Onset`] when
/// the DSP detector fails; [`AnalyzeError::Backend`] when the detector backend fails.
pub fn detect_events(
    x: &[f32],
    sr: u32,
    role: &str,
    _thresholds: &[f64; N_CLASSES],
    detector: Detector<'_>,
) -> Result<Vec<(f64, u8)>, AnalyzeError> {
    check_rate(sr)?;
    match detector {
        Detector::Dsp => {
            let hits = detect_onsets(x, sr, Role::parse(role).unwrap_or(Role::Other))?;
            let note = role_note(role);
            Ok(hits.iter().map(|h| (h.time, note)).collect())
        }
        Detector::Backend(backend) => {
            let (hits, _) = backend.detect(x, sr, role)?;
            Ok(hits
                .iter()
                .map(|h| (h.time, crate::backend::class_note(h.class, role)))
                .collect())
        }
        Detector::Candidates {
            cands,
            params,
            weak_limit,
            scale,
        } => {
            let frames = onset::repick_candidates(cands, params, weak_limit, scale);
            let note = role_note(role);
            Ok(frames.iter().map(|&f| (f as f64 / onset::FPS, note)).collect())
        }
    }
}

/// Python's `<` on floats as an ordering (equal for NaN, which the detector never gives).
fn by_time(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

/// Steps 2 to 6 on the detector's hits `detected` (`(time, note)`): the role filter, the
/// sort and min-gap dedup, the refinement with the second sort and dedup, the shift, the
/// velocities and the rounded `rise`.
///
/// # Errors
///
/// [`AnalyzeError::Rate`] when `sr` is not 44.1, 48 or 96 kHz.
pub fn analyze_detected(
    x: &[f32],
    sr: u32,
    role: &str,
    detected: &[(f64, u8)],
    params: &AnalyzeParams,
) -> Result<Analysis, AnalyzeError> {
    analyze_flagged(x, sr, role, detected, &[], &[], params)
}

/// P4-6c: [`analyze_detected`] with the two flags per detected hit that [`detect_hits`] and the Learn filter give
/// (`roll`: added by the roll fill; `out`: the Learn filter takes it out of the output; empty: none), carried through
/// the dedups and the refinement into [`Analysis::roll`] and [`Analysis::learn_out`].
///
/// # Errors
///
/// [`AnalyzeError::Rate`] when `sr` is not 44.1, 48 or 96 kHz.
pub fn analyze_flags(
    x: &[f32],
    sr: u32,
    role: &str,
    detected: &[(f64, u8)],
    roll: &[bool],
    out: &[bool],
    params: &AnalyzeParams,
) -> Result<Analysis, AnalyzeError> {
    analyze_flagged(x, sr, role, detected, roll, out, params)
}

/// [`analyze_detected`] with two flags per detected hit (`roll`: added by the roll fill; `out`: taken out by the
/// Learn filter; empty: none) that the events carry through the dedups and the refinement into [`Analysis::roll`]
/// and [`Analysis::learn_out`].
fn analyze_flagged(
    x: &[f32],
    sr: u32,
    role: &str,
    detected: &[(f64, u8)],
    roll: &[bool],
    out: &[bool],
    params: &AnalyzeParams,
) -> Result<Analysis, AnalyzeError> {
    check_rate(sr)?;
    let mut br = AnalyzeBranches {
        tracks: 1,
        detected: detected.len(),
        ..AnalyzeBranches::default()
    };
    // the flags as bits: 1 = roll, 2 = Learn out
    let bit = |v: &[bool], i: usize, b: u8| if v.get(i).copied().unwrap_or(false) { b } else { 0 };
    let mut ev: Vec<(f64, u8, u8)> = detected
        .iter()
        .enumerate()
        .map(|(i, &(t, n))| (t, n, bit(roll, i, 1) | bit(out, i, 2)))
        .collect();
    if role != "other" {
        let classes = role_classes(role);
        ev.retain(|(_, n, _)| classes.contains(n));
        br.filter_runs = 1;
        br.filtered_out = detected.len() - ev.len();
    }
    let mingap = params.min_gap_ms / 1000.0;
    // ev.sort(): the (t, note) tuples
    ev.sort_by(|a, b| by_time(a.0, b.0).then(a.1.cmp(&b.1)));
    let mut dedup: Vec<f64> = Vec::with_capacity(ev.len());
    let mut flags: Vec<u8> = Vec::with_capacity(ev.len());
    let mut last = -9e9;
    for &(t, _, r) in &ev {
        if t - last >= mingap {
            dedup.push(t);
            flags.push(r);
            last = t;
        }
    }
    br.dedup_dropped = ev.len() - dedup.len();
    let mut pairs: Vec<(f64, Option<f64>, u8)>;
    if params.refine {
        let peak_db = FM_PEAK_DB;
        let refined = refine_track_rule_peak(x, sr, &dedup, params.window, params.rule, peak_db);
        br.refine_moved = refined.iter().zip(&dedup).filter(|(r, t)| r.t != **t).count();
        br.rise_none = refined.iter().filter(|r| r.rise.is_none()).count();
        let steps = |s: ReachStep| refined.iter().filter(|r| r.step == s).count();
        br.reach_moved = steps(ReachStep::Reached);
        br.tail_moved = steps(ReachStep::Tail);
        br.reach_unresolved = steps(ReachStep::Unresolved);
        let backs = |s: BackStep| refined.iter().filter(|r| r.back == s).count();
        br.back_moved = backs(BackStep::Moved);
        br.back_busy = backs(BackStep::Busy);
        br.back_separate = backs(BackStep::Separate);
        pairs = refined.iter().zip(&flags).map(|(r, &f)| (r.t, r.rise, f)).collect();
        pairs.sort_by(|a, b| by_time(a.0, b.0)); // stable, as list.sort(key=...)
        let n = pairs.len();
        let mut last = -9e9;
        pairs.retain(|&(t, _, _)| {
            if t - last >= mingap {
                last = t;
                true
            } else {
                false
            }
        });
        br.dedup2_dropped = n - pairs.len();
    } else {
        br.refine_off = 1;
        pairs = dedup.iter().zip(&flags).map(|(&t, &f)| (t, None, f)).collect();
    }
    if params.shift_samples != 0 {
        br.shifted = 1;
        let dt = params.shift_samples as f64 / f64::from(sr);
        for p in &mut pairs {
            p.0 += dt;
        }
        pairs.sort_by(|a, b| by_time(a.0, b.0));
    }
    let times: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let vels = velocity_map(x, sr, &times, &params.velocity);
    let events: Vec<Hit> = pairs
        .iter()
        .zip(vels)
        .map(|(&(t, r, _), v)| Hit {
            t,
            v,
            rise: r.map(|r| py_round_digits(r, 1)),
        })
        .collect();
    br.events = events.len();
    let flag = |b: u8| -> Vec<bool> {
        if pairs.iter().any(|p| p.2 & b != 0) {
            pairs.iter().map(|p| p.2 & b != 0).collect()
        } else {
            Vec::new()
        }
    };
    Ok(Analysis {
        events,
        branches: br,
        roll: flag(1),
        learn_out: flag(2),
        // P4-38b: the Learn Add rule's flags are set by the caller (learn_pick), which knows
        // which detection entries it added; the analysis steps themselves never add.
        learned: Vec::new(),
    })
}

/// `engine.analyze_track` after the decode: [`detect_events`], then [`analyze_detected`].
/// `x` is the native-rate mono signal (the track's downmix), `sr` its rate (44.1, 48 or
/// 96 kHz), `role` the track's role (`kick`, `snare`, `tom`, `hat`, `cymbal`, `other`; any
/// other name filters like `other` and runs the DSP fallback with its parameters).
///
/// # Errors
///
/// [`AnalyzeError::Rate`] when `sr` is not 44.1, 48 or 96 kHz; [`AnalyzeError::Onset`] or
/// [`AnalyzeError::Backend`] when the detector fails.
pub fn analyze_track(
    x: &[f32],
    sr: u32,
    role: &str,
    params: &AnalyzeParams,
    detector: Detector<'_>,
) -> Result<Analysis, AnalyzeError> {
    let (detected, roll) = detect_hits(x, sr, role, params, detector)?;
    analyze_flagged(x, sr, role, &detected, &roll, &[], params)
}

/// The DSP fallback with its per-track candidate list (A4): the same analysis as
/// [`analyze_track`] with [`Detector::Dsp`] (identical events), plus the peak-picked
/// candidates with their gate verdicts, the detection's parameters and its weak-flux limit
/// for the store. One resample and one spectrogram for both.
///
/// # Errors
///
/// [`AnalyzeError::Rate`] when `sr` is not 44.1, 48 or 96 kHz; [`AnalyzeError::Onset`] when
/// the DSP detector fails.
pub fn analyze_track_dsp_cands(
    x: &[f32],
    sr: u32,
    role: &str,
    params: &AnalyzeParams,
) -> Result<(Analysis, Vec<onset::Candidate>, onset::OnsetParams, f32), AnalyzeError> {
    check_rate(sr)?;
    let p = onset::OnsetParams::for_role(Role::parse(role).unwrap_or(Role::Other));
    let (hits, cands, weak_limit) = onset::detect_onsets_and_cands(x, sr, &p)?;
    let note = role_note(role);
    let detected: Vec<(f64, u8)> = hits.iter().map(|h| (h.time, note)).collect();
    let a = analyze_flagged(x, sr, role, &detected, &[], &[], params)?;
    Ok((a, cands, p, weak_limit))
}

/// P4-6c: the detector's hits `(time, note)` and the roll flag per hit ([`detect_hits`]).
pub type DetectedHits = (Vec<(f64, u8)>, Vec<bool>);

/// P4-6c: step 1 of [`analyze_track`], the detector's hits with the pick rules that run on them (P4-19: the roll
/// fill; its flag per hit, empty when it added none or does not apply). The Learn filter marks these hits and then
/// runs [`analyze_flags`], so a hit list with the filter on holds the same hits as without it.
///
/// # Errors
///
/// [`AnalyzeError::Rate`] when `sr` is not 44.1, 48 or 96 kHz; [`AnalyzeError::Onset`] or
/// [`AnalyzeError::Backend`] when the detector fails.
pub fn detect_hits(
    x: &[f32],
    sr: u32,
    role: &str,
    params: &AnalyzeParams,
    detector: Detector<'_>,
) -> Result<DetectedHits, AnalyzeError> {
    Ok((detect_events(x, sr, role, &params.thresholds, detector)?, Vec::new()))
}

#[cfg(test)]
mod tests;
