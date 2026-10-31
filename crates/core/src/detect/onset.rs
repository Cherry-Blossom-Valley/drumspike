//! DSP onset fallback (P2-5).
//!
//! This module
//! still gives a hit list for a close-mic drum track. It is a SuperFlux-style detector
//! (Boeck and Widmer, `DAFx` 2013) on the log-filtered spectrogram of P2-2, with peak picking
//! on one adaptive threshold, gates against bleed and the phase 1 minimum gap. The
//! parameters per role are in [`OnsetParams::for_role`]; the report
//! `docs/archive/phase2-report.md` (section P2-5) has the measurements behind them.
//!
//! # Chain
//!
//! 1. Resample to 44.1 kHz with [`super::resample`] (48 and 96 kHz; 44.1 kHz passes).
//! 2. Log-filtered spectrogram: [`super::features::features`] with [`Chain::Cli`], the
//!    feature recipe of the reference detector (frame `i` centred on `i * 441` with 1024 zeros on
//!    each side, `1 + len / 441` frames at 100 fps, `np.hanning(2048)`, `|X|[:1024]`, the
//!    fmin-anchored 12-band triangular filterbank, `log10(1 + x)` in `f32`). The bands of a
//!    role are selected on the CLI grid's centre frequencies.
//! 3. `SuperFlux`: `flux[i] = sum_b max(0, S[i, b] - maxfilt3(S[i - 1])[b])` over the role's
//!    bands, with a 3-band maximum filter along frequency on the previous frame (1-frame
//!    lag). The frame before frame 0 is silence (the framing already pads with zeros).
//! 4. Peak picking (madmom `peak_picking` structure): a frame is a candidate when
//!    `flux[i] >= median(flux[i - pre_avg ..= i + post_avg]) + delta` and it is the maximum
//!    of the thresholded flux over `[i - pre_max, i + post_max]` (zeros outside the track).
//!    The moving median replaces madmom's moving mean: one loud hit lifts the mean of the
//!    next 10 frames by a tenth of its flux, and a second hit 20 ms later (a flam) falls
//!    below it.
//! 5. Gates. The level of a candidate is the band energy (dB) of the peak frame or the
//!    next one, relative to the track's reference level (a high quantile of the candidate
//!    levels). A candidate within `gate_db` of the reference is a loud hit. A loud hit with
//!    a weak flux (below `weak_ratio` times the template median, for example a flam on the
//!    decay of the first hit) must also show a time-domain attack of `attack_db`
//!    ([`attack_db`]). A quieter candidate is kept when its shape ([`Shape`]) has at least
//!    `sim_min` cosine similarity with the template of the track's loud hits (ghost notes
//!    and soft hits of the own drum pass, bleed of another drum does not), down to
//!    `sim_gate_db` below the reference.
//! 6. Minimum gap: a hit closer than `min_gap_s` (20 ms, the phase 1 `min_gap_ms`) to the
//!    previous kept hit is dropped, the rule of `engine.analyze_track`.
//! 7. Per hit: `time` = frame / 100 (the frame convention of the reference onsets; the phase 1
//!    refinement moves it to the attack later in the app), `rise_db` from
//!    [`crate::refine::refine_track`] at that time, and a confidence in [0, 1] (the flux
//!    over the 90th percentile of the kept hits' flux).

use std::fmt;

use rayon::prelude::*;

use super::Chain;
use super::cross_bleed::{CrossBleedParams, hit_level_full_band};
use super::features::{band_centres_hz, features};
use super::resample::{ResampleError, resample_to_44100};
use crate::refine::{RefineWindow, refine_track};

pub use super::features::{FPS, FRAME_SIZE, HOP, N_BANDS, N_FFT_BINS};

/// The role of a track, as the app names it (`engine.ROLE_CLASSES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum Role {
    /// Kick drum (reference class BD).
    Kick,
    /// Snare drum (SD).
    Snare,
    /// Toms (TT).
    Tom,
    /// Hi-hat (HH).
    Hat,
    /// Cymbals and ride (CY+RD).
    Cymbal,
    /// Anything else: every hit is kept.
    #[default]
    Other,
}

impl Role {
    /// Every role, in the app's order.
    pub const ALL: [Role; 6] = [Role::Kick, Role::Snare, Role::Tom, Role::Hat, Role::Cymbal, Role::Other];

    /// Parse the app's role name (`kick`, `snare`, `tom`, `hat`, `cymbal`, `other`).
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "kick" => Some(Role::Kick),
            "snare" => Some(Role::Snare),
            "tom" => Some(Role::Tom),
            "hat" => Some(Role::Hat),
            "cymbal" => Some(Role::Cymbal),
            "other" => Some(Role::Other),
            _ => None,
        }
    }

    /// The app's name of the role.
    pub fn name(self) -> &'static str {
        match self {
            Role::Kick => "kick",
            Role::Snare => "snare",
            Role::Tom => "tom",
            Role::Hat => "hat",
            Role::Cymbal => "cymbal",
            Role::Other => "other",
        }
    }
}

/// What the shape template compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Shape {
    /// The positive flux differences at the peak (where the energy rises).
    Flux,
    /// The log-filtered spectrum of the frame after the peak (what sounds).
    Spectrum,
}

/// Parameters of the detector. [`OnsetParams::for_role`] gives the tuned values; the
/// report explains each one.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OnsetParams {
    /// Lowest band centre in Hz that enters the flux, the level and the shape.
    pub band_lo_hz: f64,
    /// Highest band centre in Hz that enters the flux, the level and the shape.
    pub band_hi_hz: f64,
    /// Threshold above the moving median of the flux (log10 units summed over bands).
    pub delta: f32,
    /// Frames before the current one in the moving median.
    pub pre_avg: usize,
    /// Frames after the current one in the moving median.
    pub post_avg: usize,
    /// Frames before the current one in the local-maximum test.
    pub pre_max: usize,
    /// Frames after the current one in the local-maximum test.
    pub post_max: usize,
    /// Quantile of the candidate levels that is the track's reference level.
    pub ref_quantile: f32,
    /// Level gate: a candidate more than this many dB below the reference level is not a
    /// loud hit. `None`: every candidate is loud.
    pub gate_db: Option<f32>,
    /// Candidates within this many dB of the reference form the shape template.
    pub template_db: f32,
    /// What the shape template compares.
    pub shape: Shape,
    /// Shape rescue: a candidate below the level gate is kept when its spectral shape has
    /// at least this cosine similarity with the template. `None`: no rescue.
    pub sim_min: Option<f32>,
    /// The shape rescue only applies down to this many dB below the reference level.
    pub sim_gate_db: f32,
    /// A loud candidate whose flux is below `weak_ratio` times the median flux of the
    /// template candidates must show a time-domain attack (`attack_db`). 0 disables it.
    pub weak_ratio: f32,
    /// Minimum attack in dB of a weak candidate (rise of the second-difference envelope over
    /// its median in the 10 ms before).
    pub attack_db: f32,
    /// #281 A1: the soft gate (the ghost-note rescue). A candidate below the level gate is
    /// kept when its spectral shape has at least this (tighter) cosine similarity with the
    /// template of the loud hits, down to `soft_gate_db` below the reference — and no kick
    /// or tom hit sits within 30 ms of it (the engine denies the soft rescue to those
    /// candidates via `soft_busy_peaks`, see [`Candidate::soft_busy`]). `None`: no soft gate.
    #[serde(default)]
    pub soft_sim: Option<f32>,
    /// The soft gate's level floor in dB below the reference level.
    #[serde(default)]
    pub soft_gate_db: f32,
    /// A1: the soft gate's second template (from the track's own soft candidates: below the
    /// level gate down to `soft_gate_db`, low attack, not busy). A candidate below the level
    /// gate is also kept when its shape has at least this cosine similarity with the second
    /// template. The loud-hit template is the wrong reference for wire-buzz ghost notes
    /// (the a1-probe2 measurement: the false hits match it at least as well as the ghosts).
    /// `None` or 0: off.
    #[serde(default)]
    pub soft_t2: Option<f32>,
    /// A1: the maximum time-domain attack (dB) of a soft rescue. `None`: no ceiling. The
    /// measurement (a1-probe2): the ghost notes' attack p95 is 19.5 dB, the false hits'
    /// median 21 dB — a ceiling alone does not separate, but it builds the second template
    /// from the low-attack (wire-buzz-like) candidates and keeps sharp bleed out of the
    /// rescue.
    #[serde(default)]
    pub soft_attack_max: Option<f32>,
    /// Minimum distance between two kept hits in seconds.
    pub min_gap_s: f64,
    /// The track's role (A5: the relative delta's reference scale is per role). Stored, so a
    /// candidates file reads back the role it was written with; a file written before it
    /// has no `role` key and reads as `Role::Other`.
    #[serde(default)]
    pub role: Role,
}

/// Minimum gap of phase 1 (`min_gap_ms` = 20 in `defaultParams` of the app and in
/// `engine.analyze_track`); the same for every role.
pub const PHASE1_MIN_GAP_S: f64 = 0.020;

fn env_f32(name: &str) -> Option<f32> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

impl OnsetParams {
    /// The generic parameters (role `other`): every band, no level gate.
    pub fn generic() -> OnsetParams {
        OnsetParams {
            band_lo_hz: 20.0,
            band_hi_hz: 20_000.0,
            delta: 1.0,
            pre_avg: 10,
            post_avg: 0,
            pre_max: 1,
            post_max: 2,
            ref_quantile: 0.95,
            gate_db: None,
            template_db: 6.0,
            shape: Shape::Spectrum,
            sim_min: None,
            sim_gate_db: 60.0,
            weak_ratio: 0.0,
            attack_db: 0.0,
            soft_sim: None,
            soft_gate_db: 0.0,
            soft_t2: None,
            soft_attack_max: None,
            min_gap_s: PHASE1_MIN_GAP_S,
            role: Role::Other,
        }
    }

    /// Tuned parameters for a role (see `docs/archive/phase2-report.md`, P2-5): tuned
    /// for the F-measure within 1 frame against the R3 onsets of the role's class on the
    /// close-mic tracks of three sessions; the kick values also pass the synthetic corpus.
    pub fn for_role(role: Role) -> OnsetParams {
        let g = OnsetParams::generic();
        let mut p = match role {
            Role::Kick => OnsetParams {
                band_lo_hz: 20.0,
                band_hi_hz: 4_000.0,
                delta: 0.3,
                ref_quantile: 0.99,
                gate_db: Some(6.0),
                shape: Shape::Flux,
                // Detector #8 package 3 (#76): the shape rescue stays on the kick. Removing it
                // cut the three real sessions' 51 sub-level false positives but failed the ENST
                // gate (drummer_1|kick 0.849 -> 0.617, drummer_2 0.984 -> 0.903, drummer_3
                // 0.982 -> 0.785) and lost 4 refs on Session B at the working Sensitivity: the
                // rescue's true and false keeps overlap in rel_db, similarity and attack on
                // both corpora (2871 true / 222 false keeps on the ENST kick mics, 4 true /
                // 52 false on the sessions; counted distributions in #76), so no
                // per-candidate threshold separates them.
                sim_min: Some(0.85),
                sim_gate_db: 45.0,
                weak_ratio: 0.25,
                attack_db: 15.0,
                ..g
            },
            Role::Snare => OnsetParams {
                band_lo_hz: 100.0,
                band_hi_hz: 300.0,
                // A1: 1.0, not the tuned 2.0 — the ghost notes never cross the absolute
                // flux threshold (the a1-probe: not one of the 109 ghost notes has a
                // candidate within 60 ms at 2.0); the gates, the soft rescue included,
                // do the filtering instead.
                delta: env_f32("DRUMSPIKE_A1_DELTA").unwrap_or(1.0),
                ref_quantile: 0.99,
                gate_db: Some(12.0),
                // A1: the soft gate recovers the ghost notes (peak about 1/4 of the matched
                // hits, i.e. -12 dB — at the level gate's edge) and reaches deeper; the
                // clearance against kick and tom hits guards the false positives. The env
                // tunables follow the A6 pattern (the measurement drives the defaults).
                // `DRUMSPIKE_A1_SOFT_SIM=0` pins the soft gate off (the frozen references
                // have no soft gate, P3-12). The default 0.92 is from the a1-probe2 table
                // (ghost best-candidate sim p5 0.918, p10 0.923: 94 of the 99 candidates
                // with a candidate within 60 ms keep it; the model-absent false hits all
                // match at 0.950+, so the cut costs nothing there — the overhead denial
                // `oh_minus_s >= 2`, the 123/142 cut, is what separates them).
                soft_sim: {
                    let s = env_f32("DRUMSPIKE_A1_SOFT_SIM").unwrap_or(0.92);
                    (s > 0.0).then_some(s)
                },
                soft_gate_db: env_f32("DRUMSPIKE_A1_SOFT_DB").unwrap_or(30.0),
                // The second template and the attack ceiling: off until the a1-probe2
                // measurement picks their defaults (the wake measures first, then decides
                // once).
                soft_t2: env_f32("DRUMSPIKE_A1_SOFT_T2").filter(|s| *s > 0.0),
                soft_attack_max: env_f32("DRUMSPIKE_A1_SOFT_ATTACK"),
                ..g
            },
            Role::Tom => OnsetParams {
                band_lo_hz: 50.0,
                band_hi_hz: 600.0,
                delta: 1.0,
                pre_avg: 5,
                post_max: 3,
                ref_quantile: 0.99,
                gate_db: Some(3.0),
                sim_min: Some(0.9),
                sim_gate_db: 20.0,
                ..g
            },
            Role::Hat => OnsetParams {
                band_lo_hz: 2_000.0,
                band_hi_hz: 20_000.0,
                delta: 0.5,
                pre_avg: 5,
                ref_quantile: 0.99,
                gate_db: Some(3.0),
                template_db: 3.0,
                sim_min: Some(0.95),
                sim_gate_db: 30.0,
                ..g
            },
            Role::Cymbal => OnsetParams {
                band_lo_hz: 500.0,
                band_hi_hz: 20_000.0,
                delta: 0.5,
                pre_avg: 5,
                pre_max: 2,
                post_max: 1,
                gate_db: Some(30.0),
                template_db: 3.0,
                ..g
            },
            Role::Other => g,
        };
        p.role = role;
        p
    }
}

/// One detected hit, with the fields the detector gives the app (frame, time, `rise_db`,
/// confidence) and the detector's own measurements.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OnsetHit {
    /// Frame index at 100 fps.
    pub frame: usize,
    /// Frame time in seconds (`frame / 100`).
    pub time: f64,
    /// Envelope rise at the refined anchor in dB (phase 1 `rise_db`), when the refinement
    /// produced one.
    pub rise_db: Option<f64>,
    /// Confidence in [0, 1]: the flux of the hit relative to the 90th percentile of the
    /// kept hits' flux.
    pub confidence: f32,
    /// The flux value at the peak.
    pub flux: f32,
    /// Band level at the peak in dB relative to the track's reference level.
    pub rel_db: f32,
    /// Cosine similarity of the hit's shape ([`Shape`]) with the track's template.
    pub similarity: f32,
}

/// The gate of the gate stage that decided a candidate ([`Candidate`]): the gate it passed
/// (a kept candidate) or the one it failed (a dropped candidate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Gate {
    /// The level gate: `rel_db >= -gate_db` (a loud hit). A candidate that fails here is
    /// bleed or noise; the shape rescue was its only way in and did not apply.
    Level,
    /// The weak-flux check of a loud candidate (`flux >= weak_ratio * template median`);
    /// a candidate that fails it needed the time-domain attack (`attack_db`) and had none.
    WeakFlux,
    /// The time-domain attack of a weak loud candidate (`attack >= attack_db`), passed.
    WeakAttack,
    /// The shape rescue: cosine similarity with the loud-hit template at least `sim_min`
    /// down to `sim_gate_db` below the reference.
    Shape,
    /// The minimum gap: a kept gate verdict within `min_gap_s` of the previous kept hit.
    Gap,
    /// A6: the cross-track bleed rule dropped the candidate (a kick or snare hit's bleed on
    /// this mic; the evidence is in the candidate's `cross_lead_ms` and `cross_ratio_db`).
    CrossBleed,
    /// A1: the soft gate kept the candidate (a ghost note or soft hit: below the level
    /// gate, shape within `soft_sim` of the template down to `soft_gate_db`, no kick or tom
    /// hit within 30 ms).
    Soft,
}

/// One peak-picked candidate of the model-free detector with its measurements and its gate
/// verdict (A4). The detector keeps every peak, so the Sensitivity slider and
/// `track_repick` can pick again at other gates without a new detection; the list has the
/// same lifetime as the model's activations (reset on re-analysis). The verdicts are the
/// ones of the detection's own parameters; a re-pick at other parameters re-judges the
/// measurements ([`repick_candidates`]).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    /// Frame index at 100 fps.
    pub frame: usize,
    /// Frame time in seconds (`frame / 100`).
    pub time: f64,
    /// The flux value at the peak.
    pub flux: f32,
    /// Band level at the peak in dB relative to the track's reference level.
    pub rel_db: f32,
    /// Cosine similarity of the hit's shape ([`Shape`]) with the track's template.
    pub similarity: f32,
    /// The time-domain attack ([`attack_db`]) at the frame, measured at the detection
    /// (the re-pick has no signal resampling and reuses it).
    pub attack: f32,
    /// The verdict of the gate stage at the detection's parameters.
    pub kept: bool,
    /// The gate that decided (`passed` when `kept`, else the one that failed).
    pub gate: Gate,
    /// A6: the cross-mic lead over the loudest coinciding hit of a kick or snare track (ms,
    /// `motion - motion_other`; negative when the candidate leads). `None` without a
    /// coinciding hit (the cross-track rule ran only on tracks with one).
    #[serde(default)]
    pub cross_lead_ms: Option<f32>,
    /// A6: the band level ratio against it (dB, `level - level_other`); `None` as above.
    #[serde(default)]
    pub cross_ratio_db: Option<f32>,
    /// A1: the soft rescue was denied to this candidate (a kick or tom hit sits within 30
    /// ms of it); the re-pick honors the flag because the busy times are not part of the
    /// stored measurements. Only meaningful when `soft_sim` is `Some`.
    #[serde(default)]
    pub soft_busy: bool,
    /// A1: the cosine similarity of the candidate's shape with the second (soft) template
    /// (`NaN` when the detection had no second template). The re-pick needs it to re-judge
    /// the soft rescue at another scale.
    #[serde(default)]
    pub similarity_soft: f32,
}

/// Errors of the onset detector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnsetError {
    /// The source rate cannot be resampled to 44.1 kHz.
    Resample(ResampleError),
}

impl fmt::Display for OnsetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OnsetError::Resample(e) => write!(f, "onset: {e}"),
        }
    }
}

impl std::error::Error for OnsetError {}

impl From<ResampleError> for OnsetError {
    fn from(e: ResampleError) -> Self {
        OnsetError::Resample(e)
    }
}

/// Indices of the bands whose centre lies in `[lo_hz, hi_hz]`.
pub fn band_range(lo_hz: f64, hi_hz: f64) -> std::ops::Range<usize> {
    let c = band_centres_hz(Chain::Cli);
    let lo = c.iter().position(|&f| f >= lo_hz).unwrap_or(N_BANDS);
    let hi = c.iter().rposition(|&f| f <= hi_hz).map_or(lo, |h| h + 1);
    lo..hi.max(lo)
}

/// `SuperFlux` onset function of a `(frames, 84)` log spectrogram over the bands `bands`:
/// the positive differences to the 3-band maximum filter of the previous frame, summed.
/// The frame before frame 0 is silence.
pub fn superflux(spec: &[f32], bands: std::ops::Range<usize>) -> Vec<f32> {
    let frames = spec.len() / N_BANDS;
    let mut flux = vec![0f32; frames];
    let zero = [0f32; N_BANDS];
    flux.par_iter_mut().enumerate().for_each(|(i, f)| {
        let cur = &spec[i * N_BANDS..(i + 1) * N_BANDS];
        let prev = if i == 0 {
            &zero[..]
        } else {
            &spec[(i - 1) * N_BANDS..i * N_BANDS]
        };
        let mut s = 0f32;
        for b in bands.clone() {
            let lo = b.saturating_sub(1);
            let hi = (b + 1).min(N_BANDS - 1);
            let m = prev[lo].max(prev[b]).max(prev[hi]);
            let d = cur[b] - m;
            if d > 0.0 {
                s += d;
            }
        }
        *f = s;
    });
    flux
}

/// #281 A5 delta-rule version (env `DRUMSPIKE_DELTA_RULE`; unset counts as 1, the A5
/// product default; `0` pins the shipped absolute `delta` for the frozen references).
/// Version 1: [`gained_spec`].
pub fn delta_rule() -> u32 {
    match std::env::var("DRUMSPIKE_DELTA_RULE") {
        Ok(v) => v.parse().unwrap_or(1),
        Err(_) => 1,
    }
}

/// A5: the minimum time-domain attack (dB) a gain-dependent admit must show (env
/// `DRUMSPIKE_A5_GAIN_ATTACK`; 0 pins the check off). A decay tail that only crosses
/// `delta` on the gained spectrogram has almost none (the false-hit probe: 2.7 to 3.7 dB
/// against 116+ for the real strokes); a real stroke keeps its attack at any level.
fn gain_attack_db() -> f32 {
    std::env::var("DRUMSPIKE_A5_GAIN_ATTACK")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10.0)
}

/// A5 round 2: the minimum gain factor for the rule to fire (env
/// `DRUMSPIKE_A5_QUIET_GAIN` overrides all roles). A track just barely under its
/// reference scale (the coordinator's gate on the owner's real multitracks: a loud
/// snare top a touch under `s_ref`) gets a marginal gain that admits bleed and
/// ghost-level peaks; the rule now fires only when the track is *really* quiet.
/// Snare takes the strong bar (2.0, 6 dB): the real-session failures of round 1 are
/// on the snare, and a near-nominal snare must stay exactly shipped. The other
/// gainable roles take 1.25: the ENST gain of round 1 lives on the quieter hat mics,
/// in the 1.25 to 4 factor band (the round-2 probe `quiet_gain_probe`), and a
/// sub-1.25 hat gain is within the noise of the shipped tuning. The real-session bar
/// is measured by the coordinator per round.
fn quiet_gain(role: Role) -> f32 {
    let default = match role {
        Role::Snare => 2.0,
        _ => 1.25,
    };
    std::env::var("DRUMSPIKE_A5_QUIET_GAIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The A5 reference contrast scale per role: the q90 contrast of a nominal track (the
/// ENST-Drums phrase items, see `docs/migration/reports/a5-self-calibration.md`). The
/// relative delta keeps the shipped `delta` for a track at this scale; a track with half
/// the scale gets half the delta. Env `DRUMSPIKE_DELTA_REF_<ROLE>` overrides for a
/// measurement round.
fn s_ref(role: Role) -> f32 {
    let (default, name) = match role {
        Role::Kick => (23.0, "KICK"),
        Role::Snare => (23.0, "SNARE"),
        Role::Hat => (1.1, "HAT"),
        Role::Tom => (5.0, "TOM"),
        Role::Cymbal => (1.0, "CYMBAL"),
        Role::Other => (1.0, "OTHER"),
    };
    std::env::var(format!("DRUMSPIKE_DELTA_REF_{name}"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A5 delta rule 1: the track's own level instead of an absolute flux threshold. A track
/// quieter than the role's reference scale [`s_ref`] has its spectrogram *gained up* to
/// the reference (boost-only: a track at or above the reference is untouched, so
/// nominal behaviour and everything tuned on it is kept) and the shipped absolute
/// `delta` then applies to the gained spectrogram. A quiet track's gained spectrogram
/// is independent of how quiet it was, so the detection is exactly level-invariant
/// `None` (for kick, tom and cymbal tracks: kick joined them in round 3 - the
/// coordinator's gate on the owner's real multitracks read the Session A kick 52 to 55
/// with 3 more false hits and the Session H kick 7 to 6, and a real kick runs at the
/// shipped scale so the gain bought nothing there; tom and cymbal: their scale tracks
/// the bleed, not the drum - the a5 round 1 measurement: lowering the tom delta admits
/// the bleed peaks and costs 35 to 86 milli-F1) means: run the shipped detector
/// unchanged.
pub fn gained_spec(spec: &[f32], bands: &std::ops::Range<usize>, role: Role) -> Option<Vec<f32>> {
    if matches!(role, Role::Kick | Role::Tom | Role::Cymbal) {
        return None;
    }
    let frames = spec.len() / N_BANDS;
    if bands.is_empty() || frames == 0 {
        return None;
    }
    let mut mags: Vec<f32> = Vec::with_capacity(frames * bands.len());
    for i in 0..frames {
        for b in bands.clone() {
            mags.push(10f32.powf(spec[i * N_BANDS + b]) - 1.0);
        }
    }
    let scale = quantile(&mags, 0.995)?;
    if scale <= 0.0 {
        return None;
    }
    let reference = s_ref(role);
    if scale >= reference {
        return None;
    }
    let g = reference / scale;
    // A5 round 2: a near-nominal track (a small gain) stays shipped - a marginal gain
    // admits bleed and ghost-level peaks on real multitracks (the coordinator's gate);
    // see [`quiet_gain`].
    if g < quiet_gain(role) {
        return None;
    }
    let mut out = spec.to_vec();
    for i in 0..frames {
        for b in bands.clone() {
            let v = spec[i * N_BANDS + b];
            out[i * N_BANDS + b] = (1.0 + g * (10f32.powf(v) - 1.0)).log10();
        }
    }
    Some(out)
}

/// Band level in dB of each frame over `bands` (linear sum of the filtered magnitudes).
fn band_levels(spec: &[f32], bands: &std::ops::Range<usize>) -> Vec<f32> {
    spec.as_chunks::<N_BANDS>()
        .0
        .iter()
        .map(|row| {
            let s: f32 = row[bands.clone()].iter().map(|&v| 10f32.powf(v) - 1.0).sum();
            20.0 * s.max(1e-9).log10()
        })
        .collect()
}

/// Quantile `q` in [0, 1] of `v` with linear interpolation (numpy's default);
/// `None` when empty.
fn quantile(v: &[f32], q: f32) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    let pos = f64::from(q.clamp(0.0, 1.0)) * (s.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(s.len() - 1);
    let frac = (pos - lo as f64) as f32;
    Some(s[lo] + (s[hi] - s[lo]) * frac)
}

/// Attack of a 44.1 kHz signal at frame `i` in dB: the largest rise of the 0.5 ms
/// second-difference envelope over its median in the 10 ms before (excluding the last
/// 1 ms), searched from 10 ms before to 20 ms after the frame centre. The second
/// difference weights a sharp transient far above a low tone or a slow swell.
pub fn attack_db(x44: &[f32], i: usize) -> f32 {
    const WIN: usize = 22;
    let n = x44.len();
    let c = i * HOP;
    let a = c.saturating_sub(HOP + HOP + WIN);
    let z = (c + 2 * HOP).min(n);
    if z <= a + 2 * HOP {
        return 0.0;
    }
    // Envelope in dB over [a, z): mean square of the second difference over WIN samples.
    let d2 = |k: usize| -> f64 {
        let x0 = f64::from(x44[k]);
        let x1 = if k >= 1 { f64::from(x44[k - 1]) } else { 0.0 };
        let x2 = if k >= 2 { f64::from(x44[k - 2]) } else { 0.0 };
        x0 - 2.0 * x1 + x2
    };
    let sq: Vec<f64> = (a..z).map(|k| d2(k).powi(2)).collect();
    let mut env = vec![-120f32; sq.len()];
    let mut acc = 0f64;
    for k in 0..sq.len() {
        acc += sq[k];
        if k >= WIN {
            acc -= sq[k - WIN];
        }
        if k + 1 >= WIN {
            env[k] = (10.0 * (acc / WIN as f64).max(1e-12).log10()) as f32;
        }
    }
    let t0 = (c.saturating_sub(HOP)).max(a + HOP);
    let mut best = f32::MIN;
    let mut buf: Vec<f32> = Vec::with_capacity(HOP);
    let mut t = t0;
    while t < z {
        let l0 = t - HOP - a;
        let l1 = t - 44 - a;
        buf.clear();
        buf.extend_from_slice(&env[l0..l1]);
        let m = median(&mut buf);
        best = best.max(env[t - a] - m);
        t += 11;
    }
    best
}

/// Median of `buf` (the mean of the two middle values for an even length, as numpy);
/// reorders `buf`. 0 when empty.
fn median(buf: &mut [f32]) -> f32 {
    let n = buf.len();
    if n == 0 {
        return 0.0;
    }
    let (_, hi, _) = buf.select_nth_unstable_by(n / 2, f32::total_cmp);
    let hi = *hi;
    if n % 2 == 1 {
        return hi;
    }
    let lo = buf[..n / 2].iter().copied().fold(f32::MIN, f32::max);
    0.5 * (lo + hi)
}

/// Moving median of `v` over `[i - pre, i + post]`, zeros outside.
fn moving_median(v: &[f32], pre: usize, post: usize) -> Vec<f32> {
    let n = v.len();
    let w = pre + post + 1;
    let mut buf = vec![0f32; w];
    (0..n)
        .map(|i| {
            for (j, b) in buf.iter_mut().enumerate() {
                let k = i as i64 - pre as i64 + j as i64;
                *b = if k >= 0 && (k as usize) < n { v[k as usize] } else { 0.0 };
            }
            median(&mut buf)
        })
        .collect()
}

/// Peak picking on an onset function: the frames `i` with
/// `odf[i] >= median(odf[i - pre_avg ..= i + post_avg]) + delta` that are also the maximum
/// of the thresholded function over `[i - pre_max, i + post_max]` (zeros outside the
/// track, scipy `mode='constant'`). Returns ascending frame indices.
pub fn pick_peaks(
    odf: &[f32],
    delta: f32,
    pre_avg: usize,
    post_avg: usize,
    pre_max: usize,
    post_max: usize,
) -> Vec<usize> {
    let n = odf.len();
    let med = moving_median(odf, pre_avg, post_avg);
    let det: Vec<f32> = (0..n)
        .map(|i| if odf[i] >= med[i] + delta { odf[i] } else { 0.0 })
        .collect();
    (0..n)
        .filter(|&i| {
            if det[i] <= 0.0 {
                return false;
            }
            let lo = i.saturating_sub(pre_max);
            let hi = (i + post_max + 1).min(n);
            det[lo..hi].iter().all(|&v| v <= det[i])
        })
        .collect()
}

/// The flux shape of frame `i` over `bands`: the positive differences to the 3-band maximum
/// of the previous frame, L2-normalised.
fn flux_shape(spec: &[f32], i: usize, bands: &std::ops::Range<usize>) -> Vec<f32> {
    let cur = &spec[i * N_BANDS..(i + 1) * N_BANDS];
    let zero = [0f32; N_BANDS];
    let prev = if i == 0 {
        &zero[..]
    } else {
        &spec[(i - 1) * N_BANDS..i * N_BANDS]
    };
    let mut v: Vec<f32> = bands
        .clone()
        .map(|b| {
            let m = prev[b.saturating_sub(1)]
                .max(prev[b])
                .max(prev[(b + 1).min(N_BANDS - 1)]);
            (cur[b] - m).max(0.0)
        })
        .collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt() + 1e-12;
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

/// The spectral shape of frame `i` over `bands`: the log-filtered row, L2-normalised. The
/// template compares the frame after each flux peak, where the attack has entered the
/// window.
fn spectral_shape(spec: &[f32], i: usize, bands: &std::ops::Range<usize>) -> Vec<f32> {
    let row = &spec[i * N_BANDS..(i + 1) * N_BANDS];
    let mut v: Vec<f32> = row[bands.clone()].to_vec();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt() + 1e-12;
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

/// [`onsets_with_candidates`] with a template exclusion (A6 template validity): `exclude` holds the
/// peak indices whose candidates coincide with a louder confirmed hit on another track. They take no
/// part in the loud-hit template — a tom track whose loudest candidates are bleed must not learn the
/// bleed as its template — but are judged like every other candidate.
pub fn onsets_with_candidates_excluding(
    spec: &[f32],
    x44: &[f32],
    p: &OnsetParams,
    exclude: &[usize],
) -> (Vec<OnsetHit>, Vec<Candidate>, f32) {
    onsets_with_candidates_inner(spec, x44, p, exclude, &[])
}

/// [`onsets_with_candidates`] with a soft-rescue exclusion (A1): `soft_exclude` holds the peak
/// indices whose candidates coincide with a kick or tom hit within 30 ms (the busy clearance).
/// Those candidates cannot pass the soft gate ([`Gate::Soft`]); they are judged like every
/// other candidate otherwise and carry [`Candidate::soft_busy`].
pub fn onsets_with_candidates_soft_excluding(
    spec: &[f32],
    x44: &[f32],
    p: &OnsetParams,
    soft_exclude: &[usize],
) -> (Vec<OnsetHit>, Vec<Candidate>, f32) {
    onsets_with_candidates_inner(spec, x44, p, &[], soft_exclude)
}

/// Onsets of a 44.1 kHz signal and its log spectrogram with explicit parameters: peak
/// picking, the level gate with the shape rescue, the attack check of weak candidates, the
/// minimum gap and the confidence. `rise_db` stays `None` (it needs the native-rate signal).
/// Also returns every peak-picked candidate with its gate verdict (A4), before the minimum
/// gap; the hits are exactly the candidates that [`Candidate::kept`] at the scale 1.0 of
/// [`repick_candidates`], min-gapped. The third return is the weak-flux limit
/// (`p.weak_ratio * template median`), which the store keeps for the re-pick.
pub fn onsets_with_candidates(spec: &[f32], x44: &[f32], p: &OnsetParams) -> (Vec<OnsetHit>, Vec<Candidate>, f32) {
    onsets_with_candidates_inner(spec, x44, p, &[], &[])
}

fn onsets_with_candidates_inner(
    spec: &[f32],
    x44: &[f32],
    p: &OnsetParams,
    exclude: &[usize],
    soft_exclude: &[usize],
) -> (Vec<OnsetHit>, Vec<Candidate>, f32) {
    let bands = band_range(p.band_lo_hz, p.band_hi_hz);
    let frames = spec.len() / N_BANDS;
    if bands.is_empty() || frames == 0 {
        return (Vec::new(), Vec::new(), 0.0);
    }
    let spec_gained = if delta_rule() >= 1 {
        gained_spec(spec, &bands, p.role)
    } else {
        None
    };
    // A5: the gain applies only where an absolute constant acts - the flux `delta` of the
    // peak picking (and the weak-flux limit derived from it). Levels, shapes and the
    // relative gates stay on the track's own spectrogram: they are differences and are
    // level-invariant already, and the false-hit probe (ring re-peaks kept by the Level
    // gate on an over-gained track) showed that gating on the gained spectrogram admits
    // decay tails the shipped gates reject.
    let flux_raw = superflux(spec, bands.clone());
    let flux = match &spec_gained {
        Some(s) => superflux(s, bands.clone()),
        None => flux_raw.clone(),
    };
    let peaks = pick_peaks(&flux, p.delta, p.pre_avg, p.post_avg, p.pre_max, p.post_max);
    // A5: the frames the raw (ungained) flux would not have picked at `delta` - the
    // gain-dependent admits, judged below.
    let peaks_raw = if spec_gained.is_some() {
        pick_peaks(&flux_raw, p.delta, p.pre_avg, p.post_avg, p.pre_max, p.post_max)
    } else {
        peaks.clone()
    };
    if peaks.is_empty() {
        return (Vec::new(), Vec::new(), 0.0);
    }
    let lv = band_levels(spec, &bands);
    // The level of a candidate: the frame after the flux peak holds more of the attack.
    let level: Vec<f32> = peaks.iter().map(|&i| lv[i].max(lv[(i + 1).min(frames - 1)])).collect();
    let reference = quantile(&level, p.ref_quantile).unwrap_or(0.0);
    let rel: Vec<f32> = level.iter().map(|l| l - reference).collect();
    let shapes: Vec<Vec<f32>> = peaks
        .iter()
        .map(|&i| match p.shape {
            Shape::Flux => flux_shape(spec, i, &bands),
            Shape::Spectrum => spectral_shape(spec, (i + 1).min(frames - 1), &bands),
        })
        .collect();
    let attacks: Vec<f32> = peaks.iter().map(|&i| attack_db(x44, i)).collect();
    let attack_ceiling_ok = |k: usize| p.soft_attack_max.is_none_or(|a| attacks[k] <= a);
    // A1: the second template, from the track's own soft candidates: below the level gate,
    // down to `soft_gate_db`, with an attack at or under `soft_attack_max`, and not denied
    // by the busy clearance. The median of their shapes, normalised (like the loud
    // template). Needs at least 5 members; else no second template.
    let soft_t2_band = |k: usize| {
        rel[k] <= -p.gate_db.unwrap_or(f32::INFINITY)
            && rel[k] >= -p.soft_gate_db
            && attack_ceiling_ok(k)
            && !soft_exclude.contains(&k)
    };
    let t2_members: Vec<usize> = (0..peaks.len()).filter(|&k| soft_t2_band(k)).collect();
    let mut template2 = vec![0f32; bands.len()];
    let mut has_t2 = false;
    if p.soft_t2.is_some() && t2_members.len() >= 5 {
        has_t2 = true;
        let mut col = Vec::with_capacity(t2_members.len());
        for (b, t) in template2.iter_mut().enumerate() {
            col.clear();
            col.extend(t2_members.iter().map(|&k| shapes[k][b]));
            *t = quantile(&col, 0.5).unwrap_or(0.0);
        }
        let norm = template2.iter().map(|x| x * x).sum::<f32>().sqrt() + 1e-12;
        template2.iter_mut().for_each(|x| *x /= norm);
    }
    let sim2: Vec<f32> = shapes
        .iter()
        .map(|s| {
            if has_t2 {
                s.iter().zip(&template2).map(|(a, b)| a * b).sum()
            } else {
                f32::NAN
            }
        })
        .collect();
    let strong: Vec<usize> = (0..peaks.len())
        .filter(|&k| rel[k] >= -p.template_db && !exclude.contains(&k))
        .collect();
    // Template: the band-wise median of the strong candidates' shapes, normalised.
    let mut template = vec![0f32; bands.len()];
    if !strong.is_empty() {
        let mut col = Vec::with_capacity(strong.len());
        for (b, t) in template.iter_mut().enumerate() {
            col.clear();
            col.extend(strong.iter().map(|&k| shapes[k][b]));
            *t = quantile(&col, 0.5).unwrap_or(0.0);
        }
        let norm = template.iter().map(|x| x * x).sum::<f32>().sqrt() + 1e-12;
        template.iter_mut().for_each(|x| *x /= norm);
    }
    let sim: Vec<f32> = shapes
        .iter()
        .map(|s| s.iter().zip(&template).map(|(a, b)| a * b).sum())
        .collect();
    let strong_flux: Vec<f32> = strong.iter().map(|&k| flux[peaks[k]]).collect();
    let weak_limit = p.weak_ratio * quantile(&strong_flux, 0.5).unwrap_or(0.0);
    let mut hits: Vec<OnsetHit> = Vec::with_capacity(peaks.len());
    let mut candidates: Vec<Candidate> = Vec::with_capacity(peaks.len());
    let mut last: Option<usize> = None;
    let gap_frames = p.min_gap_s * FPS - 1e-9;
    for (k, &i) in peaks.iter().enumerate() {
        let loud = p.gate_db.is_none_or(|g| rel[k] >= -g);
        let attack = attacks[k];
        let weak_ok = flux[i] >= weak_limit;
        let attack_ok = attack >= p.attack_db;
        let rescued = p.sim_min.is_some_and(|s| sim[k] >= s && rel[k] >= -p.sim_gate_db);
        // A1: the soft gate. A candidate whose shape matches the template tightly passes
        // below the level gate — unless a kick or tom hit sits within 30 ms of it (the
        // busy clearance; `soft_exclude` holds those peaks, the engine computes them from
        // the kick and tom tracks' hits via `soft_busy_peaks`). The second template (from
        // the track's own soft, low-attack candidates) rescues the wire-buzz ghost notes
        // that the loud-hit template scores too low, and its ceiling keeps the sharp
        // bleed transients out.
        let soft_busy = soft_exclude.contains(&k);
        let soft_rescued = !soft_busy
            && rel[k] >= -p.soft_gate_db
            && attack_ceiling_ok(k)
            && (p.soft_sim.is_some_and(|s| sim[k] >= s) || p.soft_t2.is_some_and(|s| sim2[k] >= s));
        // A4: the gate that decided, from the measurements (the same verdicts a re-pick
        // computes at the detection's scale).
        // A5: a candidate whose flux only passes `delta` on the gained spectrogram is a
        // gain-dependent admit; it must also show the time-domain attack, which a decay
        // tail of the previous hit (the false hits of the probe) does not. A rejected one
        // is not a candidate at all: the store record (the app's binary format) has no
        // A5 field, and a re-pick must not resurrect a peak the detection measured away.
        let gain_dep = spec_gained.is_some() && !peaks_raw.contains(&i);
        let attack_floor = p.attack_db.max(gain_attack_db());
        if loud && gain_dep && attack < attack_floor {
            continue;
        }
        let (kept, gate) = if loud && gain_dep {
            (true, Gate::WeakAttack)
        } else if loud {
            if weak_ok {
                (true, Gate::Level)
            } else if attack_ok {
                (true, Gate::WeakAttack)
            } else if rescued {
                (true, Gate::Shape)
            } else if soft_rescued {
                (true, Gate::Soft)
            } else {
                (false, Gate::WeakFlux)
            }
        } else if rescued {
            (true, Gate::Shape)
        } else if soft_rescued {
            (true, Gate::Soft)
        } else {
            (false, Gate::Level)
        };
        candidates.push(Candidate {
            frame: i,
            time: i as f64 / FPS,
            flux: flux[i],
            rel_db: rel[k],
            similarity: sim[k],
            attack,
            kept,
            gate,
            cross_lead_ms: None,
            cross_ratio_db: None,
            soft_busy,
            similarity_soft: sim2[k],
        });
        if !kept {
            continue;
        }
        if last.is_some_and(|l| ((i - l) as f64) < gap_frames) {
            if let Some(c) = candidates.last_mut() {
                c.kept = false;
                c.gate = Gate::Gap;
            }
            continue;
        }
        last = Some(i);
        hits.push(OnsetHit {
            frame: i,
            time: i as f64 / FPS,
            rise_db: None,
            confidence: 0.0,
            flux: flux[i],
            rel_db: rel[k],
            similarity: sim[k],
        });
    }
    let fluxes: Vec<f32> = hits.iter().map(|h| h.flux).collect();
    let fref = quantile(&fluxes, 0.9).unwrap_or(1.0).max(1e-6);
    for h in &mut hits {
        h.confidence = (h.flux / fref).clamp(0.0, 1.0);
    }
    (hits, candidates, weak_limit)
}

/// A1: the peak indices of `cands` whose soft rescue is denied — a soft-eligible candidate
/// (its shape within `p.soft_sim` of the template, its level within `p.soft_gate_db` of the
/// reference) that coincides within `clear_ms` with a kick or tom hit. `busy` holds those
/// hits' times in seconds (the kick and tom tracks' events). The engine re-detects the
/// snare with these peaks excluded ([`detect_onsets_and_cands_soft_excluding`]); the
/// re-pick honors the stored [`Candidate::soft_busy`] flag.
pub fn soft_busy_peaks(cands: &[Candidate], p: &OnsetParams, busy: &[f64], clear_ms: f64) -> Vec<usize> {
    if p.soft_sim.is_none() && p.soft_t2.is_none() {
        return Vec::new();
    }
    let attack_max_ok = |c: &Candidate| p.soft_attack_max.is_none_or(|a| c.attack <= a);
    cands
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let t1 = p.soft_sim.is_some_and(|s| c.similarity >= s) && c.rel_db >= -p.soft_gate_db && attack_max_ok(c);
            let t2 =
                p.soft_t2.is_some_and(|s| c.similarity_soft >= s) && c.rel_db >= -p.soft_gate_db && attack_max_ok(c);
            (t1 || t2) && busy.iter().any(|&t| (t - c.time).abs() <= clear_ms / 1000.0 + 1e-9)
        })
        .map(|(k, _)| k)
        .collect()
}

/// The source for [`soft_overhead_peaks`]: the decoded oh signal and its rate.
pub struct OverheadSource<'a> {
    /// The oh track's mono signal at `sr`.
    pub signal: &'a [f32],
    /// The signal's rate.
    pub sr: u32,
}

/// A1 (#286): the peak indices of `cands` whose soft rescue is denied by an overhead — a
/// soft-eligible candidate (the same tests as [`soft_busy_peaks`]) where the oh track is
/// louder than the snare mic by more than `margin_db` at the candidate's time, both mics
/// measured as the FULL-BAND peak (the #286 A1 measurement: the A6 own-band form does not
/// separate; the common-band form cuts 123 of 142 model-absent false hits and denies none
/// of the ghosts). No oh hit is required at the time: the hat and cymbal bleed moments are
/// often below the cymbal detector's gate, and the level test is a property of the audio.
pub fn soft_overhead_peaks(
    cands: &[Candidate],
    p: &OnsetParams,
    snare: (&[f32], u32),
    ohs: &[OverheadSource],
    margin_db: f64,
) -> Vec<usize> {
    if p.soft_sim.is_none() && p.soft_t2.is_none() || ohs.is_empty() {
        return Vec::new();
    }
    let attack_max_ok = |c: &Candidate| p.soft_attack_max.is_none_or(|a| c.attack <= a);
    // the level window tolerates the detector's grid offset (a candidate time can sit one hop
    // before its transient): the A6 window plus a hop each way
    let p_x = CrossBleedParams {
        window_ms: (-12.0, 30.0),
        ..CrossBleedParams::default()
    };
    cands
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let t1 = p.soft_sim.is_some_and(|s| c.similarity >= s) && c.rel_db >= -p.soft_gate_db && attack_max_ok(c);
            let t2 =
                p.soft_t2.is_some_and(|s| c.similarity_soft >= s) && c.rel_db >= -p.soft_gate_db && attack_max_ok(c);
            if !(t1 || t2) {
                return false;
            }
            // both mics in the COMMON band — the full-band peak: `the overhead is a mix of
            // everything; only a common band compares like with like` (the #286 A1
            // measurement: the full-band form cuts 123 of 142 model-absent false hits and
            // denies none of the ghosts)
            let s_level = hit_level_full_band(snare.0, snare.1, c.time, &p_x);
            ohs.iter().any(|o| {
                let oh_level = hit_level_full_band(o.signal, o.sr, c.time, &p_x);
                oh_level - s_level > margin_db
            })
        })
        .map(|(k, _)| k)
        .collect()
}

/// The re-pick of the candidate list at another gate scale (A4). `scale` is the slider's
/// gate scale: 1.0 keeps exactly the detection's gate verdicts (the default slider value
/// reproduces the detection); a larger scale is stricter (fewer candidates), a smaller one
/// looser (more). The level gate, the shape rescue and the weak-flux limit scale with it;
/// the attack is a physical measurement and stays. Every candidate is re-judged from its
/// stored measurements; the minimum gap is not applied here (the analysis applies it).
/// `weak_limit` is the detection's weak-flux limit (`p.weak_ratio * template median`,
/// as [`onsets_with_candidates`] computed it).
pub fn repick_candidates(cands: &[Candidate], p: &OnsetParams, weak_limit: f32, scale: f32) -> Vec<usize> {
    let gate_db = p.gate_db.map(|g| g / scale);
    let sim_gate_db = p.sim_gate_db / scale;
    let sim_min = p.sim_min.map(|s| 1.0 - (1.0 - s) / scale);
    let soft_sim = p.soft_sim.map(|s| 1.0 - (1.0 - s) / scale);
    let soft_t2 = p.soft_t2.map(|s| 1.0 - (1.0 - s) / scale);
    let soft_gate_db = p.soft_gate_db / scale;
    let weak_limit = weak_limit / scale;
    cands
        .iter()
        .filter(|c| {
            let loud = gate_db.is_none_or(|g| c.rel_db >= -g);
            let weak_ok = c.flux >= weak_limit || c.attack >= p.attack_db;
            let rescued = sim_min.is_some_and(|s| c.similarity >= s && c.rel_db >= -sim_gate_db);
            // A1: the soft rescue scales like the shape rescue; a busy candidate (a kick or
            // tom hit within 30 ms) never passes it, at any scale. The second template's
            // similarity scales the same way; the attack ceiling is a physical bound and
            // stays.
            let soft_attack_ok = p.soft_attack_max.is_none_or(|a| c.attack <= a);
            let soft = soft_attack_ok
                && c.rel_db >= -soft_gate_db
                && !c.soft_busy
                && (soft_sim.is_some_and(|s| c.similarity >= s) || soft_t2.is_some_and(|s| c.similarity_soft >= s));
            (loud && weak_ok) || rescued || soft
        })
        .map(|c| c.frame)
        .collect()
}

/// Onsets of a 44.1 kHz signal and its log spectrogram with explicit parameters.
pub fn onsets_from_spectrogram(spec: &[f32], x44: &[f32], p: &OnsetParams) -> Vec<OnsetHit> {
    onsets_with_candidates(spec, x44, p).0
}

/// Detect the hits of a mono track at `sr` (44.1, 48 or 96 kHz) for `role` with the tuned
/// parameters of [`OnsetParams::for_role`].
pub fn detect_onsets(x: &[f32], sr: u32, role: Role) -> Result<Vec<OnsetHit>, OnsetError> {
    detect_onsets_with(x, sr, &OnsetParams::for_role(role))
}

/// [`detect_onsets`] with explicit parameters. `rise_db` comes from the phase 1
/// refinement on the native-rate signal at each frame time; the time itself stays the
/// frame time.
pub fn detect_onsets_with(x: &[f32], sr: u32, p: &OnsetParams) -> Result<Vec<OnsetHit>, OnsetError> {
    Ok(detect_onsets_and_cands(x, sr, p)?.0)
}

/// [`detect_onsets_and_cands`] with a template exclusion (A6): `exclude` holds the peak indices whose
/// candidates coincide with a louder confirmed hit on another track; they take no part in the loud-hit
/// template (see [`onsets_with_candidates_excluding`]). The peak picking is deterministic, so the
/// indices are the same in the re-run.
pub fn detect_onsets_and_cands_excluding(
    x: &[f32],
    sr: u32,
    p: &OnsetParams,
    exclude: &[usize],
) -> Result<(Vec<OnsetHit>, Vec<Candidate>, f32), OnsetError> {
    let y = resample_to_44100(x, sr)?;
    let spec = features(&y, Chain::Cli);
    let (mut hits, cands, weak_limit) = onsets_with_candidates_excluding(&spec, &y, p, exclude);
    let times: Vec<f64> = hits.iter().map(|h| h.time).collect();
    for (h, (_, r)) in hits.iter_mut().zip(refine_track(x, sr, &times, RefineWindow::DEFAULT)) {
        h.rise_db = r;
    }
    Ok((hits, cands, weak_limit))
}

/// [`detect_onsets_and_cands`] with a soft-rescue exclusion (A1): `soft_exclude` holds the peak
/// indices whose candidates coincide with a kick or tom hit within 30 ms; they cannot pass the
/// soft gate (see [`onsets_with_candidates_soft_excluding`]). The peak picking is deterministic,
/// so the indices are the same in the re-run.
pub fn detect_onsets_and_cands_soft_excluding(
    x: &[f32],
    sr: u32,
    p: &OnsetParams,
    soft_exclude: &[usize],
) -> Result<(Vec<OnsetHit>, Vec<Candidate>, f32), OnsetError> {
    let y = resample_to_44100(x, sr)?;
    let spec = features(&y, Chain::Cli);
    let (mut hits, cands, weak_limit) = onsets_with_candidates_soft_excluding(&spec, &y, p, soft_exclude);
    let times: Vec<f64> = hits.iter().map(|h| h.time).collect();
    for (h, (_, r)) in hits.iter_mut().zip(refine_track(x, sr, &times, RefineWindow::DEFAULT)) {
        h.rise_db = r;
    }
    Ok((hits, cands, weak_limit))
}

/// [`detect_onsets_with`] with the peak-picked candidate list and the weak-flux limit of
/// the detection (A4): the model-free detector keeps every candidate with its gate verdict,
/// so the Sensitivity slider and `track_repick` pick again without a new detection.
pub fn detect_onsets_and_cands(
    x: &[f32],
    sr: u32,
    p: &OnsetParams,
) -> Result<(Vec<OnsetHit>, Vec<Candidate>, f32), OnsetError> {
    let y = resample_to_44100(x, sr)?;
    let spec = features(&y, Chain::Cli);
    let (mut hits, cands, weak_limit) = onsets_with_candidates(&spec, &y, p);
    let times: Vec<f64> = hits.iter().map(|h| h.time).collect();
    for (h, (_, r)) in hits.iter_mut().zip(refine_track(x, sr, &times, RefineWindow::DEFAULT)) {
        h.rise_db = r;
    }
    Ok((hits, cands, weak_limit))
}

#[cfg(test)]
mod tests;
