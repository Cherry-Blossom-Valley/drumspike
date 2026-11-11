//! The first-motion back-track of versions 3 to 7: the walk from the refined sample back to the foot of the attack,
//! the strict foot (versions 4 and 5) and the lead-in switch (#239, off).

use std::sync::atomic::{AtomicBool, Ordering};

use super::edge::{EdgeTail, edge_foot_counted};
use super::max_abs;
use crate::pyops::py_int;

/// What the first-motion back-track did with a hit (P4-12 stage 2, [`first_motion`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackStep {
    /// The rule of the analysis has no back-track (versions 1 and 2).
    Off,
    /// The hit moved back to the foot of its attack.
    Moved,
    /// The foot is the version 2 time itself (a sharp start), or the peak is under the silence limit.
    Kept,
    /// The walk reached its bound without a gap: bleed or a tail above the threshold (a busy
    /// context). The version 2 time stays.
    Busy,
    /// The walk crossed more than [`FM_LONG_S`] of signal well under the hit: a separate earlier
    /// event, not the hit's own start. The version 2 time stays.
    Separate,
}

/// The peak window of the back-track: the attack's peak lies in `s .. s + FM_PEAK_S` (seconds).
pub(crate) const FM_PEAK_S: f64 = 0.005;
/// The back-track's threshold under that peak (dB). P4-11 table A8 started at 30 dB; the kit
/// measured 33 dB closer to the first motion (`aud`) with the same spread in noise and bleed.
pub const FM_PEAK_DB: f64 = 33.0;
/// The floor before the hit: the median of the [`FM_BLOCK_S`] block maxima in the
/// `FM_FLOOR_S` before the hit, ...
pub(crate) const FM_FLOOR_S: f64 = 0.060;
/// ... without the last `FM_FLOOR_SKIP_S` (the attack's own rise and bleed; the same 60 to 12 ms window as the floor
/// of `lead_in_check`, #273), ...
pub(crate) const FM_FLOOR_SKIP_S: f64 = 0.012;
/// ... in blocks of this length (seconds) ...
pub(crate) const FM_BLOCK_S: f64 = 0.001;
/// ... and the threshold is at least this much (dB) above it: noise and bleed before the hit set
/// the threshold, not only the peak.
pub(crate) const FM_FLOOR_DB: f64 = 6.0;
/// A run of samples under the threshold this long (seconds) ends the attack; a shorter gap (a
/// zero crossing) does not.
pub(crate) const FM_GAP_S: f64 = 0.0005;
/// The walk never goes further back than this (seconds), nor past the midpoint to the previous
/// coarse hit.
pub(crate) const FM_SPAN_S: f64 = 0.010;
/// A walk longer than this (seconds) ...
pub(crate) const FM_LONG_S: f64 = 0.005;
/// ... is taken only when the stretch it crosses (without its last `FM_LONG_END_S`) holds a level
/// within [`FM_LONG_DB`] of the first `FM_LONG_END_S` at the version 2 time: that time sat on the
/// hit's body. Else the stretch is a separate earlier event.
pub(crate) const FM_LONG_END_S: f64 = 0.0005;
/// See [`FM_LONG_END_S`] (dB).
pub(crate) const FM_LONG_DB: f64 = 6.0;
/// The version 4 guard (#101): the strict foot may move at most this far (seconds) before the
/// version 3 foot. Further than that, the version 3 foot stays.
pub(crate) const FM_STRICT_MAX_S: f64 = 0.00025;
/// Version 5 (#127): the strict foot moves at most this many samples before the version 3 foot, at every rate.
pub(crate) const FOOT5_MAX_SAMPLES: i64 = 3;

/// How the strict foot's move before the version 3 foot is capped: the cap and what a move beyond it does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FootTune {
    /// The largest move before the version 3 foot.
    pub cap: FootCap,
    /// What a move beyond the cap does.
    pub overrun: Overrun,
}

/// The cap of [`FootTune`]: a time (version 4) or a count of samples at every rate (version 5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum FootCap {
    /// Seconds, rounded to samples at the rate.
    Seconds(f64),
    /// Samples, at every rate.
    Samples(i64),
    /// Version 10 (#444): a count of samples at 48 kHz, as the count that covers the same time at the rate.
    At48k(i64),
}

/// What a strict foot beyond its cap does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Overrun {
    /// The version 3 foot stays (version 4 and the version 5 default).
    Keep,
    /// The foot moves to the cap, the cap's last sample before the version 3 foot.
    Clamp,
}

impl FootTune {
    /// The version 4 foot: a move of at most [`FM_STRICT_MAX_S`], else the version 3 foot.
    pub const V4: FootTune = FootTune {
        cap: FootCap::Seconds(FM_STRICT_MAX_S),
        overrun: Overrun::Keep,
    };
    /// The version 5 foot: a move of at most [`FOOT5_MAX_SAMPLES`], else the foot moves to the cap.
    pub const V5: FootTune = FootTune {
        cap: FootCap::Samples(FOOT5_MAX_SAMPLES),
        overrun: Overrun::Clamp,
    };
    /// The version 10 foot (#444): [`FootTune::V5`] whose cap covers the same time at every rate.
    pub const V10: FootTune = FootTune {
        cap: FootCap::At48k(FOOT5_MAX_SAMPLES),
        overrun: Overrun::Clamp,
    };
}

/// What the back-track takes beside the signal and its bounds: the peak margin, the strict foot (versions 4 and 5) and
/// the steep edge after the foot (versions 6 and 7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct BackTune {
    /// The threshold under the peak (dB): [`FM_PEAK_DB`], or another margin (P4-31).
    pub(super) peak_db: f64,
    /// The foot taken one step stricter ([`foot_strict`]) with this cap, when `Some`.
    pub(super) tune: Option<FootTune>,
    /// The first steep edge after the foot ([`edge_foot_counted`]) with this tail rule, when `Some`.
    pub(super) edge: Option<EdgeTail>,
    /// Version 8 (#338): the strict foot ([`foot_strict`]) of the walk's foot when no edge counts, with this cap, when
    /// `Some`. The walk reads the 2-tap mean, which halves the first sample of a sharp start and reads it as the line, so
    /// the walk's foot sits one sample into such an attack; the strict foot is the last floor-level sample before it.
    pub(super) fallback: Option<FootTune>,
}

impl BackTune {
    /// The version 3 back-track with the peak margin `peak_db`: no strict foot, no steep edge.
    pub(super) const fn peak(peak_db: f64) -> BackTune {
        BackTune {
            peak_db,
            tune: None,
            edge: None,
            fallback: None,
        }
    }
}

/// Seconds to samples, rounded (the back-track's spans are in time, not samples: P4-11b R4).
pub(super) fn samples(s: f64, srf: f64) -> i64 {
    (s * srf).round() as i64
}

/// The 2-tap mean `|x[i] + x[i - 1]| / 2` (`x[-1]` = 0). It nulls the Nyquist ripple of a
/// band-limited sharp start, so the walk does not read that ripple as motion.
fn two_tap(x: &[f32], i: usize) -> f64 {
    let prev = if i > 0 { f64::from(x[i - 1]) } else { 0.0 };
    (f64::from(x[i]) + prev).abs() / 2.0
}

/// The floor before sample `s`: the median of the [`FM_BLOCK_S`] block maxima of `|x|` in
/// `[from, s - FM_FLOOR_SKIP_S)`, the blocks counted back from the end; 0 with no full block.
pub(super) fn floor_before(x: &[f32], s: i64, from: i64, srf: f64) -> f64 {
    let end = (s - samples(FM_FLOOR_SKIP_S, srf)).min(x.len() as i64);
    let b = samples(FM_BLOCK_S, srf).max(1);
    let from = from.max(0);
    if end - from < b {
        return 0.0;
    }
    let nb = (end - from) / b;
    let mut m: Vec<f64> = (0..nb)
        .map(|k| f64::from(max_abs(x, end - (k + 1) * b, end - k * b)))
        .collect();
    m.sort_by(f64::total_cmp);
    let n = m.len();
    if n % 2 == 1 {
        m[n / 2]
    } else {
        (m[n / 2 - 1] + m[n / 2]) / 2.0
    }
}

/// The first-motion back-track (P4-12 stage 2): from the refined sample `s`, walk back to the
/// foot of the hit's attack. `prev` and `next` are the neighbouring coarse hits (seconds) and
/// `t0` the hit's own coarse time.
///
/// The threshold is the larger of [`FM_PEAK_DB`] under the peak in `s .. s + FM_PEAK_S`
/// (clipped at the midpoint to `next`) and [`FM_FLOOR_DB`] above the floor before the hit
/// ([`floor_before`], from `s - FM_FLOOR_S` or the midpoint to `prev`). The foot is the first
/// sample of the run at or above it ([`two_tap`]) that holds `s`; gaps under [`FM_GAP_S`] do not
/// end the run. The walk stops at `s - FM_SPAN_S` and at the midpoint to `prev`: a walk that
/// reaches that bound without a gap keeps `s` ([`BackStep::Busy`]). A walk of more than
/// [`FM_LONG_S`] is taken only over the hit's own body ([`BackStep::Separate`] else). The result is
/// never later than `s`.
#[must_use]
pub fn first_motion(x: &[f32], s: i64, sr: u32, t0: f64, prev: Option<f64>, next: Option<f64>) -> (i64, BackStep) {
    first_motion_peak(x, s, sr, t0, prev, next, FM_PEAK_DB)
}

/// [`first_motion`] with another peak margin `peak_db` in place of [`FM_PEAK_DB`] (P4-31: kick tracks).
#[must_use]
pub(crate) fn first_motion_peak(
    x: &[f32],
    s: i64,
    sr: u32,
    t0: f64,
    prev: Option<f64>,
    next: Option<f64>,
    peak_db: f64,
) -> (i64, BackStep) {
    back_track(x, s, sr, t0, (prev, next), BackTune::peak(peak_db))
}

/// The back-track of [`first_motion_peak`] with the settings `bt` ([`BackTune`]). `neighbors` are the previous and the
/// next hit time (`None` at the ends).
pub(super) fn back_track(
    x: &[f32],
    s: i64,
    sr: u32,
    t0: f64,
    neighbors: (Option<f64>, Option<f64>),
    bt: BackTune,
) -> (i64, BackStep) {
    let srf = f64::from(sr);
    let (prev, next) = neighbors;
    let prev_mid = prev.map(|p| py_int(((p + t0) / 2.0) * srf));
    let next_mid = next.map(|nx| py_int(((t0 + nx) / 2.0) * srf));
    first_motion_traced(x, s, sr, prev_mid, next_mid, bt).0
}

/// [`first_motion`] with the bounds as samples: `prev_mid` bounds the walk and the floor window (the midpoint to the
/// previous hit), `next_mid` the peak window (the midpoint to the next hit). The flam rule's version 1 places its
/// second stroke with it (bounded by the block after the dip, `prev_mid`), as do the golden corpus and the flam
/// rule's ring comparison (`tools/onset_guard.py` `walk`).
pub(crate) fn first_motion_mid(
    x: &[f32],
    s: i64,
    sr: u32,
    prev_mid: Option<i64>,
    next_mid: Option<i64>,
) -> (i64, BackStep) {
    first_motion_traced(x, s, sr, prev_mid, next_mid, BackTune::peak(FM_PEAK_DB)).0
}

/// The strict foot of version 4 (#101). `d` is the walk's first sample over the threshold. Walking back from
/// it, the floor is the last two samples at or under `floor` (two in a row: a single quiet sample inside a
/// noisy rise is not the floor). The foot is the last floor-level sample before the first sample that leaves
/// that floor after it. No such pair above `lo`: `d` stays. The samples are read raw: the 2-tap mean halves the
/// first sample of a slow rise and reads it as floor.
pub(super) fn foot_strict(x: &[f32], d: i64, lo: i64, floor: f64) -> i64 {
    let quiet = |i: i64| f64::from(x[i as usize].abs()) <= floor;
    let mut i = d;
    while i - 2 >= lo && !(quiet(i - 1) && quiet(i - 2)) {
        i -= 1;
    }
    if i - 2 < lo {
        return d;
    }
    let mut j = i - 1;
    while j < d && quiet(j) {
        j += 1;
    }
    j - 1
}

/// The lead-in of the walk (#239), one switch for the whole process. Off since version 6 (#293, #294: it joined the
/// bleed before snare hits). Its setter went with the before/after measurement `lead_in_check` (removed in the
/// refactor, see git history); no product path turns it on.
pub(super) static LEAD_IN: AtomicBool = AtomicBool::new(false);

/// The lead-in goes back at most this far (seconds) from the walk's foot: an attack's rise is about a millisecond. A
/// longer rise over the floor is bleed or an earlier stroke, not the hit.
pub(crate) const LEAD_IN_MAX_S: f64 = 0.0015;

/// What the walk of one hit decided (#239): the floor and threshold it used, the foot of the walk and the foot after
/// the lead-in (equal when the lead-in did not join a rise).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeadTrace {
    /// The floor before the hit.
    pub floor: f64,
    /// The walk's threshold.
    pub thr: f64,
    /// The foot of the walk, in samples.
    pub foot: i64,
    /// The foot after the lead-in, in samples.
    pub lead: i64,
}

/// The lead-in of a walk (#239). The threshold `thr` follows the peak, so a small first bump before the main swing
/// (under `thr`, over the floor) ends the walk at the swing. `foot` is the walk's foot: this goes on back from it over
/// the samples at or above [`FM_FLOOR_DB`] over the `floor` (the same gap rule), at most [`LEAD_IN_MAX_S`] and never
/// past `lo`, and returns the first sample of that rise. A rise that does not end within that bound, no floor, or a
/// `thr` that is not above that level: `foot` stays. `judge` is the end of the stretch that decides whether the rise
/// is a first bump (#271): the foot of the walk at the base margin, the same for every peak margin.
fn lead_in(x: &[f32], (foot, judge): (i64, i64), lo: i64, floor: f64, thr: f64, gap: i64, srf: f64) -> i64 {
    if !LEAD_IN.load(Ordering::Relaxed) {
        return foot;
    }
    let low = floor * 10f64.powf(FM_FLOOR_DB / 20.0);
    if low <= 0.0 || low >= thr {
        return foot;
    }
    let lo = lo.max(foot - samples(LEAD_IN_MAX_S, srf));
    let mut lead = foot;
    let mut i = foot;
    while i > lo {
        i -= 1;
        if two_tap(x, i as usize) >= low {
            lead = i;
        } else if lead - i >= gap {
            return if is_first_bump(x, lead, judge.max(lead), gap) {
                lead
            } else {
                foot
            };
        }
    }
    foot
}

/// Whether the stretch `lead .. foot` is a first bump of the hit and not something in front of a hard attack (#271):
/// - it rises toward `foot`: the level in its first half `gap` samples is under 0.8 of the level in the last `gap`
///   samples (a decaying tail of an earlier stroke is as loud at its far end as at its near end);
/// - it is no Nyquist ripple: the sign changes at most 3 of 4 times from one sample to the next (the pre-ringing of a
///   band-limited hard start alternates at nearly every sample; a bump of the hit does not).
fn is_first_bump(x: &[f32], lead: i64, foot: i64, gap: i64) -> bool {
    let w = gap.min(foot - lead);
    if w <= 0 {
        return false;
    }
    let far = f64::from(max_abs(x, lead, lead + (w / 2).max(1)));
    let near = f64::from(max_abs(x, foot - w, foot));
    let pairs = foot - lead - 1;
    let flips = (lead + 1..foot)
        .filter(|&i| x[i as usize] * x[i as usize - 1] < 0.0)
        .count() as i64;
    far < near * 0.8 && flips * 4 <= pairs * 3
}

/// The walk from `s` with its decision (the floor, the threshold, the foot and the foot after the
/// lead-in) for one hit; `None` where the walk does not reach the lead-in (no signal, a busy or separate stretch).
#[must_use]
pub fn first_motion_trace(
    x: &[f32],
    s: i64,
    sr: u32,
    prev_mid: Option<i64>,
    next_mid: Option<i64>,
) -> Option<LeadTrace> {
    first_motion_traced(x, s, sr, prev_mid, next_mid, BackTune::peak(FM_PEAK_DB)).1
}

/// The foot of the walk back from `s` over the samples at or above `thr` (gaps under `gap` do not end the run), and
/// whether the walk ended on a gap before `lo`.
fn walk_foot(x: &[f32], s: i64, lo: i64, thr: f64, gap: i64) -> (i64, bool) {
    let mut i = s;
    let mut foot = s;
    while i > lo {
        i -= 1;
        if two_tap(x, i as usize) >= thr {
            foot = i;
        } else if foot - i >= gap {
            return (foot, true);
        }
    }
    (foot, false)
}

pub(super) fn first_motion_traced(
    x: &[f32],
    s: i64,
    sr: u32,
    prev_mid: Option<i64>,
    next_mid: Option<i64>,
    bt: BackTune,
) -> ((i64, BackStep), Option<LeadTrace>) {
    let BackTune {
        peak_db,
        tune,
        edge,
        fallback,
    } = bt;
    let srf = f64::from(sr);
    let n = x.len() as i64;
    if s <= 0 || s >= n {
        return ((s, BackStep::Kept), None);
    }
    let mut pe = (s + samples(FM_PEAK_S, srf)).min(n);
    if let Some(m) = next_mid {
        pe = pe.min(m);
    }
    let pk = f64::from(max_abs(x, s, pe));
    if pk < 1e-4 {
        return ((s, BackStep::Kept), None);
    }
    let mut lo = (s - samples(FM_SPAN_S, srf)).max(0);
    let mut from = s - samples(FM_FLOOR_S, srf);
    if let Some(m) = prev_mid {
        lo = lo.max(m);
        from = from.max(m);
    }
    let floor = floor_before(x, s, from, srf);
    let thr = (pk * 10f64.powf(-peak_db / 20.0)).max(floor * 10f64.powf(FM_FLOOR_DB / 20.0));
    let gap = samples(FM_GAP_S, srf).max(1);
    let (foot, ended) = walk_foot(x, s, lo, thr, gap);
    if !ended {
        return ((s, BackStep::Busy), None);
    }
    if s - foot > samples(FM_LONG_S, srf) {
        let e = samples(FM_LONG_END_S, srf).max(1);
        let held = f64::from(max_abs(x, foot, (s - e).max(foot + 1)));
        let at = f64::from(max_abs(x, s, s + e));
        if held < at * 10f64.powf(-FM_LONG_DB / 20.0) {
            return ((s, BackStep::Separate), None);
        }
    }
    let walked = foot;
    // The bump decision is judged on the stretch up to the foot of the walk at the base margin, so that every peak
    // margin accepts or rejects the same lead-in (#271).
    let judge = if peak_db == FM_PEAK_DB {
        foot
    } else {
        let thr0 = (pk * 10f64.powf(-FM_PEAK_DB / 20.0)).max(floor * 10f64.powf(FM_FLOOR_DB / 20.0));
        match walk_foot(x, s, lo, thr0, gap) {
            (f0, true) => f0,
            _ => foot,
        }
    };
    let foot = lead_in(x, (foot, judge), lo, floor, thr, gap, srf);
    let trace = LeadTrace {
        floor,
        thr,
        foot: walked,
        lead: foot,
    };
    // the strict foot of `foot` with its cap (versions 4 and 5 always, version 8 when no edge counts)
    let strict = |foot: i64, tune: FootTune| {
        let end = (s - samples(FM_FLOOR_SKIP_S, srf)).min(n);
        let strict_foot = foot_strict(x, foot, lo, f64::from(max_abs(x, from, end)));
        let cap = match tune.cap {
            FootCap::Seconds(t) => samples(t, srf),
            FootCap::Samples(k) => k,
            FootCap::At48k(k) => super::edge::at_rate(k, srf),
        };
        if foot - strict_foot <= cap {
            strict_foot
        } else {
            match tune.overrun {
                Overrun::Keep => foot,
                Overrun::Clamp => foot - cap,
            }
        }
    };
    let foot = match tune {
        None => foot,
        Some(tune) => strict(foot, tune),
    };
    // versions 6 to 8: the first steep edge after the foot; it may lie after `s` (the hit moves later). No edge that
    // counts: the walk's foot stays, or (version 8) its strict foot
    let foot = match edge {
        Some(tail) => match edge_foot_counted(x, foot, pe, srf, tail) {
            Some(k) => k,
            None => match fallback {
                Some(tune) => strict(foot, tune),
                None => foot,
            },
        },
        None => foot,
    };
    let step = if foot < s || (edge.is_some() && foot != s) {
        (foot, BackStep::Moved)
    } else {
        (s, BackStep::Kept)
    };
    (step, Some(trace))
}
