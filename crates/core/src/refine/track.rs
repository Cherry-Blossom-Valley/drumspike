//! The refinement rule of an analysis ([`RefineRule`], versions 1 to 7), the dispatch to its rule, and the entry
//! points for one hit and for a whole track.

use super::Refined;
use super::edge::EdgeTail;
use super::reach::{ReachStep, refine_reach};
use super::reference::{RefineWindow, refine_sr};
use super::walk::{BackStep, BackTune, FM_PEAK_DB, FootTune, back_track};
use crate::velocity::{VelocityMap, velocity_map};

/// The refinement rule of an analysis (P4-12). The analysis records its version, so a project
/// analysed before keeps its hit times until the engineer analyses it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RefineRule {
    /// Version 1: [`refine_sr`], the retired engine's rule, pinned to the frozen references.
    /// Projects analysed before P4-12 carry no version and run this rule.
    #[default]
    Reference,
    /// Version 2 (P4-12 stage 1): [`refine_reach`](super::reach::refine_reach), the reference with the reach and the tail fix.
    Reach,
    /// Version 3 (P4-12 stage 2): [`refine_first_motion`], version 2 and then the back-track to
    /// the first motion of the hit.
    FirstMotion,
    /// Version 4 (#101): [`RefineRule::FirstMotion`] with the foot taken one step stricter. The
    /// back-track's first sample over its threshold is the first sample that leaves the floor, or
    /// later; the foot is the last floor-level sample before it ([`foot_strict`](super::walk::foot_strict)). The strict foot
    /// moves at most [`FM_STRICT_MAX_S`](super::walk::FM_STRICT_MAX_S) before the version 3 foot, else the version 3 foot stays.
    Foot,
    /// Version 5 (#127): [`RefineRule::Foot`] with the strict foot's move capped at [`FOOT5_MAX_SAMPLES`](super::walk::FOOT5_MAX_SAMPLES)
    /// samples before the version 3 foot (the foot moves to the cap).
    FootCapped,
    /// Version 6 (#293, #294): the version 3 foot (no strict foot, no lead-in), then the first steep edge after it
    /// ([`edge_foot`](super::edge::edge_foot)): a hit that the walk put on the bleed or the soft drift before its attack moves to the foot of
    /// the attack's first steep rise.
    Edge,
    /// Version 7 (#321): [`RefineRule::Edge`] whose last walk-back to the line follows what the engineer sees: back over
    /// the samples that stand visibly off the line ([`VIS_TAIL_PCT`](super::edge::VIS_TAIL_PCT) of the hit's peak) with the edge's sign, when that
    /// walk reaches the line within [`VIS_TAIL_MAX_S`](super::edge::VIS_TAIL_MAX_S) ([`EdgeTail::Visible`]); else version 6's walk. Version 6 judged the tail against the line noise and took at most 4 samples:
    /// on a nearly silent kick line it walked onto invisible steps (2 to 3 samples early), on a snare line with hi-hat
    /// bleed it stopped inside the slow drift (2 to 7 samples late).
    Visible,
    /// Version 8 (#338): [`RefineRule::Visible`] with two corrections. The walk-back to the line measures the distance
    /// from the line (the median of the 2 ms before the walk's foot, [`EdgeTail::Line`]), not from zero: on a kick line
    /// that stood at an offset of about the visibility share, version 7 walked down the line, 4 samples before the
    /// first motion. And when no steep edge counts (a stroke whose attack rises under 18 dB per 0.1 ms), the hit is
    /// version 5's strict foot of the walk's foot, not the walk's foot itself: the walk reads the 2-tap mean, which
    /// halves the first sample of a sharp start, so the foot sat one sample into the attack.
    Line,
    /// Version 9 (#351): [`RefineRule::Line`] with one principle added: the hit is the last sample of the line before
    /// the stroke's FIRST MOTION, whatever its sign. The walk-back from the counted edge's foot goes over every
    /// sample that stands visibly off the line, of either sign, until it meets
    /// [`LINE_RUN_SAMPLES`](super::edge::LINE_RUN_SAMPLES) consecutive samples within the visibility band, within
    /// [`DEPARTURE_MAX_S`](super::edge::DEPARTURE_MAX_S) of the edge's first rising sample ([`EdgeTail::Departure`]);
    /// otherwise version 8's walk-back decides. A stroke often opens with a small swing of the other sign than the
    /// steep edge (a positive bump before a negative kick edge, a dip and a rise before a negative snare edge) that
    /// versions 6 to 8 do not follow past the zero crossing between the swing and the edge.
    Departure,
    /// Version 10 (#362): [`RefineRule::Departure`] with one principle: the line is measured by its own
    /// noise, not by a fixed share of the peak. The visibility band, the run's band and the run length
    /// scale with the spread of the line samples before the stroke (the measure
    /// [`placement_diag`](super::edge::placement_diag) reports as `line_noise`), with the fixed shares only
    /// as a floor, and version 9's own departure as the floor of the answer
    /// ([`EdgeTail::NoiseDeparture`]): a gentle opening swing of either sign counts on a noisy line, where
    /// version 9's fixed band refuses the whole departure, and a shorter run on a noisier line is the rule,
    /// not a tuning. A line that is a slow wave or an offset stays the line, as before.
    NoiseDeparture,
}

impl RefineRule {
    /// The rule a new analysis runs.
    pub const CURRENT: RefineRule = RefineRule::NoiseDeparture;

    /// The version number the document stores.
    #[must_use]
    pub fn version(self) -> u32 {
        match self {
            RefineRule::Reference => 1,
            RefineRule::Reach => 2,
            RefineRule::FirstMotion => 3,
            RefineRule::Foot => 4,
            RefineRule::FootCapped => 5,
            RefineRule::Edge => 6,
            RefineRule::Visible => 7,
            RefineRule::Line => 8,
            RefineRule::Departure => 9,
            RefineRule::NoiseDeparture => 10,
        }
    }

    /// The rule of a stored version number; `None` for a version this build does not know.
    #[must_use]
    pub fn from_version(v: u64) -> Option<RefineRule> {
        match v {
            1 => Some(RefineRule::Reference),
            2 => Some(RefineRule::Reach),
            3 => Some(RefineRule::FirstMotion),
            4 => Some(RefineRule::Foot),
            5 => Some(RefineRule::FootCapped),
            6 => Some(RefineRule::Edge),
            7 => Some(RefineRule::Visible),
            8 => Some(RefineRule::Line),
            9 => Some(RefineRule::Departure),
            10 => Some(RefineRule::NoiseDeparture),
            _ => None,
        }
    }
}

/// Refinement version 3 (P4-12 stage 2, [`RefineRule::FirstMotion`]): [`refine_reach`](super::reach::refine_reach), then the
/// back-track [`first_motion`](super::first_motion) from its time. `rise` and the reach step are version 2's; a hit
/// moves only earlier, and only where the back-track finds the foot of its attack.
#[must_use]
pub fn refine_first_motion(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow) -> Refined {
    refine_first_motion_peak(x, t0, sr, neighbors, win, FM_PEAK_DB)
}

/// [`refine_first_motion`] with another back-track peak margin (P4-31).
#[must_use]
pub(crate) fn refine_first_motion_peak(
    x: &[f32],
    t0: f64,
    sr: u32,
    neighbors: &[f64],
    win: RefineWindow,
    peak_db: f64,
) -> Refined {
    refine_back_peak(x, t0, sr, neighbors, win, BackTune::peak(peak_db))
}

/// [`refine_first_motion_peak`] with the back-track settings `bt`: the strict foot of its `tune` ([`RefineRule::Foot`],
/// [`RefineRule::FootCapped`]) when it is `Some`, and the steep edge after the foot ([`RefineRule::Edge`],
/// [`RefineRule::Visible`]) when its `edge` is set.
fn refine_back_peak(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow, bt: BackTune) -> Refined {
    let r = refine_reach(x, t0, sr, neighbors, win);
    let srf = f64::from(sr);
    let (mut prev, mut next): (Option<f64>, Option<f64>) = (None, None);
    for &nt in neighbors {
        if nt < t0 && prev.is_none_or(|p| nt > p) {
            prev = Some(nt);
        } else if nt > t0 && next.is_none_or(|q| nt < q) {
            next = Some(nt);
        }
    }
    let s = (r.t * srf).round() as i64;
    let (foot, back) = back_track(x, s, sr, t0, (prev, next), bt);
    Refined {
        t: if back == BackStep::Moved {
            foot as f64 / srf
        } else {
            r.t
        },
        back,
        ..r
    }
}

/// [`refine_reach`](super::reach::refine_reach) or [`refine_sr`], as `rule` says.
#[must_use]
pub fn refine_rule(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow, rule: RefineRule) -> Refined {
    refine_rule_peak(x, t0, sr, neighbors, win, rule, FM_PEAK_DB)
}

/// [`refine_rule`] with the back-track peak margin `peak_db` (it matters for versions 3 to 8, the rules with a back-track).
#[must_use]
pub fn refine_rule_peak(
    x: &[f32],
    t0: f64,
    sr: u32,
    neighbors: &[f64],
    win: RefineWindow,
    rule: RefineRule,
    peak_db: f64,
) -> Refined {
    // the back-track of versions 4 to 8: the strict foot, or the steep edge after it (and, version 8, the strict foot
    // when no edge counts)
    let back = |tune, edge, fallback| {
        refine_back_peak(
            x,
            t0,
            sr,
            neighbors,
            win,
            BackTune {
                peak_db,
                tune,
                edge,
                fallback,
            },
        )
    };
    match rule {
        RefineRule::Reference => {
            let (t, rise) = refine_sr(x, t0, sr, neighbors, win);
            Refined {
                t,
                rise,
                step: ReachStep::Kept,
                back: BackStep::Off,
            }
        }
        RefineRule::Reach => refine_reach(x, t0, sr, neighbors, win),
        RefineRule::FirstMotion => refine_first_motion_peak(x, t0, sr, neighbors, win, peak_db),
        RefineRule::Foot => back(Some(FootTune::V4), None, None),
        RefineRule::FootCapped => back(Some(FootTune::V5), None, None),
        RefineRule::Edge => back(None, Some(EdgeTail::Noise), None),
        RefineRule::Visible => back(None, Some(EdgeTail::Visible), None),
        RefineRule::Line => back(None, Some(EdgeTail::Line), Some(FootTune::V5)),
        RefineRule::Departure => back(None, Some(EdgeTail::Departure), Some(FootTune::V5)),
        RefineRule::NoiseDeparture => back(None, Some(EdgeTail::NoiseDeparture), Some(FootTune::V5)),
    }
}

/// [`refine_track`] with the rule of the analysis: the previous and the next coarse hit are the
/// neighbours of each one.
#[must_use]
pub fn refine_track_rule(x: &[f32], sr: u32, coarse: &[f64], win: RefineWindow, rule: RefineRule) -> Vec<Refined> {
    refine_track_rule_peak(x, sr, coarse, win, rule, FM_PEAK_DB)
}

/// [`refine_track_rule`] with the back-track peak margin `peak_db` (P4-31).
#[must_use]
pub(crate) fn refine_track_rule_peak(
    x: &[f32],
    sr: u32,
    coarse: &[f64],
    win: RefineWindow,
    rule: RefineRule,
    peak_db: f64,
) -> Vec<Refined> {
    coarse
        .iter()
        .enumerate()
        .map(|(i, &t)| refine_rule_peak(x, t, sr, &neighbours(coarse, i), win, rule, peak_db))
        .collect()
}

/// The previous and the next coarse hit of hit `i`.
fn neighbours(coarse: &[f64], i: usize) -> Vec<f64> {
    let mut nb = Vec::with_capacity(2);
    if i > 0 {
        nb.push(coarse[i - 1]);
    }
    if i + 1 < coarse.len() {
        nb.push(coarse[i + 1]);
    }
    nb
}

/// Refine every coarse hit of a track the way `engine.analyze_track` does: the previous and
/// the next coarse hit are the neighbours of each one. `coarse` is the min-gap deduplicated,
/// ascending detector output; the result is in the same order (not re-sorted, not re-deduped).
#[must_use]
pub fn refine_track(x: &[f32], sr: u32, coarse: &[f64], win: RefineWindow) -> Vec<(f64, Option<f64>)> {
    coarse
        .iter()
        .enumerate()
        .map(|(i, &t)| refine_sr(x, t, sr, &neighbours(coarse, i), win))
        .collect()
}

/// A hit as the document stores it: time in seconds, velocity, optional `rise` (dB).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Onset in seconds.
    pub t: f64,
    /// MIDI velocity.
    pub v: i64,
    /// Envelope rise at the anchor in dB (`rise_db`), when the refinement produced one.
    pub rise: Option<f64>,
}

/// `engine.recompute_velocities`: keep every hit time and every other field, replace only
/// the velocity with `velocity_map` at the hit times.
pub fn recompute_velocities(x: &[f32], sr: u32, hits: &mut [Hit], m: &VelocityMap) {
    let times: Vec<f64> = hits.iter().map(|h| h.t).collect();
    for (h, v) in hits.iter_mut().zip(velocity_map(x, sr, &times, m)) {
        h.v = v;
    }
}
