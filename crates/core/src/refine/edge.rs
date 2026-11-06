//! The steep edge after the walk's foot (versions 6 and 7, #293, #294, #321) and how its foot walks back to the
//! line ([`EdgeTail`]).

use super::max_abs;
use super::walk::{FM_PEAK_S, samples};

/// How the steep edge's foot walks back to the line (#321): [`EdgeTail::Noise`] is version 6, [`EdgeTail::Visible`]
/// version 7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeTail {
    /// Up to [`EDGE_TAIL_SAMPLES`] samples while the level rises by more than [`EDGE_TAIL_NOISE`] times the line noise.
    Noise,
    /// Back while the sample stands more than [`VIS_TAIL_PCT`] of the hit's peak off the line, keeps the edge's sign
    /// and does not grow towards the line by more than [`VIS_TAIL_TOL_PCT`] of the peak. It counts only when it reaches
    /// the line within [`VIS_TAIL_MAX_S`]; otherwise [`EdgeTail::Noise`] decides (a slow wave or an offset on a tom's
    /// line is the line itself).
    Visible,
    /// Version 8 (#338): [`EdgeTail::Visible`] measured from the line, not from zero. The line's level is the median of
    /// the signal over [`LINE_LEVEL_S`] before the walk's foot: a line that stands at an offset (the slow wave after the
    /// previous stroke, about the visibility share of the peak) is the line itself, not a stretch that stands visibly
    /// off it.
    Line,
    /// Version 9 (#351): the walk-back from the counted edge's foot goes over every sample that stands visibly off the
    /// line, of EITHER sign, until it meets [`LINE_RUN_SAMPLES`] consecutive samples within the visibility band. A stroke
    /// often opens with a small swing of the other sign (a positive bump before a negative kick edge, a dip and a rise
    /// before a negative snare edge) that runs into the edge; versions 6 to 8 follow the edge's sign only and stop at the
    /// zero crossing between the swing and the edge. The walk counts only when it meets the run within
    /// [`DEPARTURE_MAX_S`] of the edge's first rising sample; otherwise version 8's walk-back decides (a slow wave or an
    /// offset on a tom line is the line itself, as before).
    Departure,
}

/// Version 6 (#293, #294): the rise window of the steep edge (seconds): the signal must rise by [`EDGE_RISE_DB`] under
/// the peak within it.
pub(crate) const EDGE_RISE_S: f64 = 0.0001;
/// The rise that makes a steep edge (dB under the peak of the hit, within [`EDGE_RISE_S`]).
pub(crate) const EDGE_RISE_DB: f64 = 18.0;
/// The search stops where the level reaches this much (dB) under the hit's peak: the hit's body began without an
/// edge that counts, and the walk's foot stays.
pub(crate) const EDGE_TOP_DB: f64 = 12.0;
/// The guard: the level between the walk's foot and the edge (without the edge's own ramp-up) must stay this much
/// (dB) under the level the edge reaches within [`EDGE_LOCAL_S`]. A ripple inside the body of a soft hit, or a slow
/// attack without a clear jump, is no edge: the search goes on. Fitted on the owner's hand-placed hits (#293, kick
/// 1..332 and snare 1..168 of one session): 9 to 15 dB give the same result; 12 is the middle.
pub(crate) const EDGE_GUARD_DB: f64 = 12.0;
/// The window after an edge's foot whose peak the guard compares against (seconds): the level this edge reaches, not
/// a louder swing later in the hit.
pub(crate) const EDGE_LOCAL_S: f64 = 0.0005;
/// The tail of the foot's walk-back: up to this many samples more while the level still rises by more than
/// [`EDGE_TAIL_NOISE`] times the line noise, so the hit sits at the foot of the steep rise. Fitted on the owner's
/// hand-placed hits (#293): 4 and 6 give the same result; 4 is the smaller.
pub(crate) const EDGE_TAIL_SAMPLES: i64 = 4;
/// The tail's step: a rise of more than this many times the line noise (the median `|x[n] - x[n-1]|` over the
/// [`EDGE_NOISE_S`] before the walk's foot) still belongs to the hit. Fitted on the owner's hand-placed hits (#293):
/// 2 took the slow drift of a kick's line for the hit (1 to 3 samples early); 8 does not.
pub(crate) const EDGE_TAIL_NOISE: f64 = 8.0;
/// Version 7 (#321): a sample stands visibly off the line when `|x|` exceeds this share of the hit's peak (percent).
/// Fitted on the owner's hand-placed hits (kick 1..332, snare 1..168 of one session; even hits chose, odd hits
/// checked): 2 and 3 give the same held-out result; 2 is the smaller.
pub(crate) const VIS_TAIL_PCT: f64 = 2.0;
/// Version 7: the walk-back to the line goes on while the sample before is no more than this share of the peak
/// (percent) larger than the current one: the small wiggles of a snare's drift do not stop it.
pub(crate) const VIS_TAIL_TOL_PCT: f64 = 0.5;
/// Version 7: the longest walk-back to the line (seconds) from the edge's first rising sample. A drift the engineer
/// counts as part of the stroke is short: on the owner's hand-placed hits every limit from 6 samples (0.125 ms at
/// 48 kHz) up keeps the whole gain. A longer stretch off the line is the line itself (a slow wave or an offset of a tom
/// line, Sessions A and B): with a 1 ms limit the walk followed it down to a zero crossing up to 1 ms before the stroke.
pub(crate) const VIS_TAIL_MAX_S: f64 = 0.000_15;
/// The stretch before the walk's foot that measures the line noise (seconds).
pub(crate) const EDGE_NOISE_S: f64 = 0.002;
/// Version 8 (#338): the stretch before the walk's foot whose median is the line's level (seconds). Short, so that it
/// follows a line that is a slow wave or holds bleed (over 2 ms the median sat 0.5 % of the peak off the line right
/// before the stroke and the walk went down the wave); longer than the rise window, so that a walk foot that lies two
/// or three samples inside a steep rise does not tilt the median. On the owner's hand-placed hits 8 to 12 samples give
/// the same result; 12 is 0.25 ms at 48 kHz.
pub(crate) const LINE_LEVEL_S: f64 = 0.000_25;
/// Version 9 (#351): the line is reached when this many consecutive samples stand within the visibility band. A
/// single-sample zero crossing between a pre-swing and the edge is not the line, and neither is the short in-band
/// stretch of a bleed's or a ripple's own zero crossing: the run has to be as long as those. Starting point pending
/// the fit on the labelled hits (even hits choose, odd hits check, as [`VIS_TAIL_PCT`] was).
pub(crate) const LINE_RUN_SAMPLES: i64 = 10;
/// Version 9 (#351): the longest walk-back to the line (seconds) from the edge's first rising sample. The swing
/// before a stroke is short; a longer stretch off the line is the line itself (a slow wave or an offset on a tom
/// line), and there version 8's walk-back decides, as before. Starting point pending the fit on the same hits.
pub(crate) const DEPARTURE_MAX_S: f64 = 0.000_5;
/// How far after the walk's foot the edge is looked for (seconds).
pub(crate) const EDGE_SPAN_S: f64 = 0.005;

/// The foot of the first steep edge after the walk's foot (version 6, #293, #294). The walk stops at the first sample
/// over its threshold, which lies on the bleed or the soft drift that often precedes a snare or kick attack; the
/// engineer puts the hit where the waveform visibly starts. An edge is a sample `j` from one rise window before
/// `foot` (a steep attack's walk foot lies inside its rise) to `end` (at most
/// [`EDGE_SPAN_S`] after the foot: the peak and the search stay on this hit, not on a louder later swing) where
/// `|x[j + w] - x[j]|` (w = [`EDGE_RISE_S`]) reaches [`EDGE_RISE_DB`] under the peak of `[foot, end)`. Its foot is the
/// first sample in that window whose step to the next sample is half the edge's per-sample rise, walked back while
/// the step before it stays as large (a quarter let the floor noise through), then up to [`EDGE_TAIL_SAMPLES`] more
/// while the level rises by more than [`EDGE_TAIL_NOISE`] times the line noise (at least a sixteenth of the edge's
/// per-sample rise: a digitally silent line has no noise). The edge counts when the level between `foot` and its foot (without the last `w` samples, the edge's own
/// ramp-up) lies [`EDGE_GUARD_DB`] under the peak of the [`EDGE_LOCAL_S`] after its foot; else the search goes on after
/// the window. An edge raises `|x|` (a falling slope is none), and the search stops where the level itself reaches
/// [`EDGE_TOP_DB`] under the peak of `[foot, end)`. The first edge that counts is the hit; none: `foot` stays.
#[cfg(test)]
pub(super) fn edge_foot(x: &[f32], foot: i64, end: i64, srf: f64, tail_rule: EdgeTail) -> i64 {
    edge_foot_counted(x, foot, end, srf, tail_rule).unwrap_or(foot)
}

/// [`edge_foot`] that says when no edge counts: `None` then (version 8 takes the strict foot of the walk's foot in that
/// case, [`super::walk::first_motion_traced`]).
pub(super) fn edge_foot_counted(x: &[f32], foot: i64, end: i64, srf: f64, tail_rule: EdgeTail) -> Option<i64> {
    let w = samples(EDGE_RISE_S, srf).max(1);
    let n = x.len() as i64;
    let end = end.min(n - w).min(foot + samples(EDGE_SPAN_S, srf));
    if foot < 0 || foot + w >= end {
        return None;
    }
    let pk = f64::from(max_abs(x, foot, end));
    if pk <= 0.0 {
        return None;
    }
    let need = pk * 10f64.powf(-EDGE_RISE_DB / 20.0);
    let guard = 10f64.powf(-EDGE_GUARD_DB / 20.0);
    let top = pk * 10f64.powf(-EDGE_TOP_DB / 20.0);
    let local = samples(EDGE_LOCAL_S, srf).max(1);
    let step = need / w as f64 / 2.0;
    let at = |i: i64| f64::from(x[i as usize]);
    let noise = {
        let from = (foot - samples(EDGE_NOISE_S, srf)).max(1);
        let mut d: Vec<f64> = (from..foot).map(|i| (at(i) - at(i - 1)).abs()).collect();
        if d.is_empty() {
            0.0
        } else {
            let mid = d.len() / 2;
            *d.select_nth_unstable_by(mid, f64::total_cmp).1
        }
    };
    // the line's level, the median of the short stretch before the walk's foot (versions 8 and 9 measure from it; 0
    // for the older tails: they read the distance from zero)
    let line = match tail_rule {
        EdgeTail::Line | EdgeTail::Departure => {
            let from = (foot - samples(LINE_LEVEL_S, srf)).max(0);
            let mut v: Vec<f64> = (from..foot).map(at).collect();
            if v.is_empty() {
                0.0
            } else {
                let mid = v.len() / 2;
                *v.select_nth_unstable_by(mid, f64::total_cmp).1
            }
        }
        EdgeTail::Noise | EdgeTail::Visible => 0.0,
    };
    let off = |i: i64| at(i) - line;
    // the search starts one rise window before the walk's foot: on a steep attack the first sample over the walk's
    // threshold already lies inside the rise, and from there the next window rises too little
    let lo = (foot - w - EDGE_TAIL_SAMPLES).max(1);
    let mut j = (foot - w).max(1);
    while j < end {
        // an edge raises the level away from the line (a falling slope after a first bump is none)
        if at(j + w).abs() - at(j).abs() < need {
            // the level reached the hit's body without an edge that counts: the walk's foot stays
            if at(j).abs() >= top {
                return None;
            }
            j += 1;
            continue;
        }
        // the rise window starts at `j`; the edge's first rising sample lies inside it. The walk-back follows the
        // rising level `|x|`: it stops at a zero crossing or a bump's top, never climbs a falling slope
        let rise = |m: i64| at(m + 1).abs() - at(m).abs();
        let mut k = (j..j + w).find(|&m| rise(m) >= step).unwrap_or(j);
        let k0 = k;
        while k > lo && rise(k - 1) >= step {
            k -= 1;
        }
        // version 6's tail: up to EDGE_TAIL_SAMPLES more while the level rises by more than EDGE_TAIL_NOISE times the
        // line noise. It may pass the walk's foot (the walk stops at the first sample over its threshold, the foot of
        // the rise lies up to a few samples before it); it takes only samples of the edge's sign (band-limited ringing
        // before a hard start changes sign at every sample, #271) and the last sample on the line, so that the hit sits
        // on the sample before the first rise, as version 5's strict foot did
        let noise_tail = |mut k: i64| {
            let mut tail = 0;
            while k > lo
                && tail < EDGE_TAIL_SAMPLES
                && rise(k - 1) > (EDGE_TAIL_NOISE * noise).max(step / 16.0)
                && (at(k - 1) * at(k) > 0.0 || at(k - 1).abs() <= EDGE_TAIL_NOISE * noise)
            {
                k -= 1;
                tail += 1;
            }
            k
        };
        // version 7 (#321): the engineer puts the hit on the last sample of the line, judged by eye against the
        // hit's own size, not against the line noise: back over the samples that stand visibly off the line with
        // the edge's sign while they do not grow towards the line (a slow drift before a snare stroke is taken
        // whole, the invisible first steps of a kick on a silent line are not). The walk counts only when it
        // reaches the line within VIS_TAIL_MAX_S: a tom whose line is a slow wave on the edge's side never reaches
        // it, and there version 6's tail stays (the walk would stop at its bound, an arbitrary place)
        // Version 8 (#338) reads every level from the line, not from zero: a line at an offset of about the
        // visibility share is the line, not a drift the walk goes down
        let line_walk = |k: i64, k0: i64| {
            let vis = pk * VIS_TAIL_PCT / 100.0;
            let tol = pk * VIS_TAIL_TOL_PCT / 100.0;
            let sgn = off(k0 + 1).signum();
            let lo_v = (k0 - samples(VIS_TAIL_MAX_S, srf)).max(1);
            let mut kv = k;
            while kv > lo_v
                && off(kv).abs() > vis
                && off(kv).signum() == sgn
                && off(kv - 1).abs() <= off(kv).abs() + tol
            {
                kv -= 1;
            }
            if off(kv).abs() <= vis { kv } else { noise_tail(k) }
        };
        k = match tail_rule {
            EdgeTail::Noise => noise_tail(k),
            EdgeTail::Visible | EdgeTail::Line => line_walk(k, k0),
            // version 9 (#351): back over every sample that stands visibly off the line, of either sign, until
            // LINE_RUN_SAMPLES consecutive samples within the visibility band: the last sample of that run is the
            // last line sample before the stroke's first motion. A single-sample zero crossing between a pre-swing
            // and the edge is not the line (the count resets on a sample off the line), and the walk counts only
            // once it has passed a sample that stands visibly off the line: without a departure there is no first
            // motion to walk before (the tail of a bleed or of band-limited pre-ringing is not the stroke's). The
            // walk counts only when it meets the run within DEPARTURE_MAX_S of the edge's first rising sample;
            // otherwise version 8's walk-back decides (a slow wave or an offset on a tom line is the line itself,
            // as before)
            EdgeTail::Departure => {
                let vis = pk * VIS_TAIL_PCT / 100.0;
                let run = LINE_RUN_SAMPLES.max(1);
                let lo_d = (k0 - samples(DEPARTURE_MAX_S, srf)).max(1);
                let mut i = k;
                let mut in_band = 0i64;
                let mut departed = false;
                let mut hit = None;
                while i >= lo_d {
                    if off(i).abs() > vis {
                        in_band = 0;
                        departed = true;
                    } else if departed {
                        in_band += 1;
                        if in_band >= run {
                            hit = Some(i + run - 1);
                            break;
                        }
                    }
                    i -= 1;
                }
                hit.unwrap_or_else(|| line_walk(k, k0))
            }
        };
        let reached = f64::from(max_abs(x, k, (k + local).min(end + w)));
        let before = if k - w > foot {
            f64::from(max_abs(x, foot, k - w))
        } else {
            0.0
        };
        if before <= reached * guard {
            return Some(k);
        }
        j += w;
    }
    None
}

/// The placement evidence of one hit (#344): whether a steep edge counts at the stored position,
/// and how the line before it reads. `line_noise` and `line_slope` are shares of the hit's peak
/// (the slope per ms, signed: positive rises). The review queue's Training Mode reads them per
/// hit, computed on demand and never stored; no rule of the analysis reads them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacementDiag {
    /// A steep edge counts for the hit's peak ([`edge_foot_counted`], the version 8 tail rule):
    /// `false` when the placement fell back to the walk's foot.
    pub edge: bool,
    /// The standard deviation of the 2 ms before the foot, over the peak.
    pub line_noise: f64,
    /// The trend of the same 2 ms, per ms, over the peak.
    pub line_slope: f64,
}

/// The placement evidence of the hit at `t` samples (the stored position, #344). The edge decision
/// is the version 8 one for the hit's peak: [`edge_foot_counted`] from `t` over the
/// [`FM_PEAK_S`] window with the [`EdgeTail::Line`] rule. The line is measured as version 8
/// measures it: the line level is the median over [`LINE_LEVEL_S`] before the foot, the noise is
/// the standard deviation of the [`EDGE_NOISE_S`] before the foot over the hit's peak, and the
/// slope the trend of the same 2 ms per ms over the peak.
#[must_use]
pub fn placement_diag(x: &[f32], sr: u32, t: i64) -> PlacementDiag {
    let srf = f64::from(sr);
    let n = x.len() as i64;
    let t = t.clamp(0, n);
    let end = (t + samples(FM_PEAK_S, srf)).min(n);
    let peak = f64::from(max_abs(x, t, end));
    let edge = edge_foot_counted(x, t, end, srf, EdgeTail::Line).is_some();
    // the line level: the version 8 median over LINE_LEVEL_S before the foot
    let line = {
        let from = (t - samples(LINE_LEVEL_S, srf)).max(0);
        let mut v: Vec<f64> = (from..t).map(|i| f64::from(x[i as usize])).collect();
        if v.is_empty() {
            0.0
        } else {
            let mid = v.len() / 2;
            *v.select_nth_unstable_by(mid, f64::total_cmp).1
        }
    };
    // the 2 ms before the foot, read off the line
    let from = (t - samples(EDGE_NOISE_S, srf)).max(0);
    let w: Vec<f64> = (from..t).map(|i| f64::from(x[i as usize]) - line).collect();
    let (noise, slope) = if w.is_empty() || peak <= 0.0 {
        (0.0, 0.0)
    } else {
        let m = w.len() as f64;
        let mean = w.iter().sum::<f64>() / m;
        let noise = (w.iter().map(|d| (d - mean) * (d - mean)).sum::<f64>() / m).sqrt() / peak;
        // the trend: the least squares slope over the window, per ms over the peak
        let si: f64 = (0..w.len()).map(|i| i as f64).sum();
        let sii: f64 = (0..w.len()).map(|i| (i * i) as f64).sum();
        let sd: f64 = w.iter().sum();
        let sid: f64 = w.iter().enumerate().map(|(i, d)| i as f64 * d).sum();
        let den = m * sii - si * si;
        let per_sample = if den == 0.0 { 0.0 } else { (m * sid - si * sd) / den };
        (noise, per_sample * srf / 1000.0 / peak)
    };
    PlacementDiag {
        edge,
        line_noise: noise,
        line_slope: slope,
    }
}
