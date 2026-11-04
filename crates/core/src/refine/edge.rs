//! The steep edge after the walk's foot (versions 6 and 7, #293, #294, #321) and how its foot walks back to the
//! line ([`EdgeTail`]).

use super::max_abs;
use super::walk::samples;

/// How the steep edge's foot walks back to the line (#321): [`EdgeTail::Noise`] is version 6, [`EdgeTail::Visible`]
/// version 7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeTail {
    /// Up to [`EDGE_TAIL_SAMPLES`] samples while the level rises by more than [`EDGE_TAIL_NOISE`] times the line noise.
    Noise,
    /// Back while the sample stands more than [`VIS_TAIL_PCT`] of the hit's peak off the line, keeps the edge's sign
    /// and does not grow towards the line by more than [`VIS_TAIL_TOL_PCT`] of the peak. It counts only when it reaches
    /// the line within [`VIS_TAIL_MAX_S`]; otherwise [`EdgeTail::Noise`] decides (a slow wave or an offset on a tom's
    /// line is the line itself).
    Visible,
}

/// Version 6 (#293, #294): the rise window of the steep edge (seconds): the signal must rise by [`EDGE_RISE_DB`] under
/// the peak within it.
pub const EDGE_RISE_S: f64 = 0.0001;
/// The rise that makes a steep edge (dB under the peak of the hit, within [`EDGE_RISE_S`]).
pub const EDGE_RISE_DB: f64 = 18.0;
/// The search stops where the level reaches this much (dB) under the hit's peak: the hit's body began without an
/// edge that counts, and the walk's foot stays.
pub const EDGE_TOP_DB: f64 = 12.0;
/// The guard: the level between the walk's foot and the edge (without the edge's own ramp-up) must stay this much
/// (dB) under the level the edge reaches within [`EDGE_LOCAL_S`]. A ripple inside the body of a soft hit, or a slow
/// attack without a clear jump, is no edge: the search goes on. Fitted on the owner's hand-placed hits (#293, kick
/// 1..332 and snare 1..168 of one session): 9 to 15 dB give the same result; 12 is the middle.
pub const EDGE_GUARD_DB: f64 = 12.0;
/// The window after an edge's foot whose peak the guard compares against (seconds): the level this edge reaches, not
/// a louder swing later in the hit.
pub const EDGE_LOCAL_S: f64 = 0.0005;
/// The tail of the foot's walk-back: up to this many samples more while the level still rises by more than
/// [`EDGE_TAIL_NOISE`] times the line noise, so the hit sits at the foot of the steep rise. Fitted on the owner's
/// hand-placed hits (#293): 4 and 6 give the same result; 4 is the smaller.
pub const EDGE_TAIL_SAMPLES: i64 = 4;
/// The tail's step: a rise of more than this many times the line noise (the median `|x[n] - x[n-1]|` over the
/// [`EDGE_NOISE_S`] before the walk's foot) still belongs to the hit. Fitted on the owner's hand-placed hits (#293):
/// 2 took the slow drift of a kick's line for the hit (1 to 3 samples early); 8 does not.
pub const EDGE_TAIL_NOISE: f64 = 8.0;
/// Version 7 (#321): a sample stands visibly off the line when `|x|` exceeds this share of the hit's peak (percent).
/// Fitted on the owner's hand-placed hits (kick 1..332, snare 1..168 of one session; even hits chose, odd hits
/// checked): 2 and 3 give the same held-out result; 2 is the smaller.
pub const VIS_TAIL_PCT: f64 = 2.0;
/// Version 7: the walk-back to the line goes on while the sample before is no more than this share of the peak
/// (percent) larger than the current one: the small wiggles of a snare's drift do not stop it.
pub const VIS_TAIL_TOL_PCT: f64 = 0.5;
/// Version 7: the longest walk-back to the line (seconds) from the edge's first rising sample. A drift the engineer
/// counts as part of the stroke is short: on the owner's hand-placed hits every limit from 6 samples (0.125 ms at
/// 48 kHz) up keeps the whole gain. A longer stretch off the line is the line itself (a slow wave or an offset of a tom
/// line, Sessions A and B): with a 1 ms limit the walk followed it down to a zero crossing up to 1 ms before the stroke.
pub const VIS_TAIL_MAX_S: f64 = 0.000_15;
/// The stretch before the walk's foot that measures the line noise (seconds).
pub const EDGE_NOISE_S: f64 = 0.002;
/// How far after the walk's foot the edge is looked for (seconds).
pub const EDGE_SPAN_S: f64 = 0.005;

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
pub(super) fn edge_foot(x: &[f32], foot: i64, end: i64, srf: f64, tail_rule: EdgeTail) -> i64 {
    let w = samples(EDGE_RISE_S, srf).max(1);
    let n = x.len() as i64;
    let end = end.min(n - w).min(foot + samples(EDGE_SPAN_S, srf));
    if foot < 0 || foot + w >= end {
        return foot;
    }
    let pk = f64::from(max_abs(x, foot, end));
    if pk <= 0.0 {
        return foot;
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
    // the search starts one rise window before the walk's foot: on a steep attack the first sample over the walk's
    // threshold already lies inside the rise, and from there the next window rises too little
    let lo = (foot - w - EDGE_TAIL_SAMPLES).max(1);
    let mut j = (foot - w).max(1);
    while j < end {
        // an edge raises the level away from the line (a falling slope after a first bump is none)
        if at(j + w).abs() - at(j).abs() < need {
            // the level reached the hit's body without an edge that counts: the walk's foot stays
            if at(j).abs() >= top {
                return foot;
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
        k = match tail_rule {
            EdgeTail::Noise => noise_tail(k),
            // version 7 (#321): the engineer puts the hit on the last sample of the line, judged by eye against the
            // hit's own size, not against the line noise: back over the samples that stand visibly off the line with
            // the edge's sign while they do not grow towards the line (a slow drift before a snare stroke is taken
            // whole, the invisible first steps of a kick on a silent line are not). The walk counts only when it
            // reaches the line within VIS_TAIL_MAX_S: a tom whose line is a slow wave on the edge's side never reaches
            // it, and there version 6's tail stays (the walk would stop at its bound, an arbitrary place)
            EdgeTail::Visible => {
                let vis = pk * VIS_TAIL_PCT / 100.0;
                let tol = pk * VIS_TAIL_TOL_PCT / 100.0;
                let sgn = at(k0 + 1).signum();
                let lo_v = (k0 - samples(VIS_TAIL_MAX_S, srf)).max(1);
                let mut kv = k;
                while kv > lo_v
                    && at(kv).abs() > vis
                    && at(kv).signum() == sgn
                    && at(kv - 1).abs() <= at(kv).abs() + tol
                {
                    kv -= 1;
                }
                if at(kv).abs() <= vis { kv } else { noise_tail(k) }
            }
        };
        let reached = f64::from(max_abs(x, k, (k + local).min(end + w)));
        let before = if k - w > foot {
            f64::from(max_abs(x, foot, k - w))
        } else {
            0.0
        };
        if before <= reached * guard {
            return k;
        }
        j += w;
    }
    foot
}
