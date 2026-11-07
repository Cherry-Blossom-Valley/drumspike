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
    /// Version 10 (#362): the departure of version 9 with the line measured by its own noise, not by a fixed
    /// share of the peak. The visibility band, the run's band and the run length scale with the spread of the
    /// line samples before the stroke (the measure [`placement_diag`] reports as `line_noise`, the standard
    /// deviation of the [`EDGE_NOISE_S`] before the walk's foot over the peak), with the fixed shares only as a
    /// floor. Version 9's departure is the FLOOR: when version 9 accepts a departure, version 10 returns
    /// exactly version 9's answer, and the noise-scaled walk only adds departures version 9 refused — a
    /// gentle opening swing of a noisy line counts where version 9's fixed band refuses the whole departure
    /// (the whole stroke falls back to the version 8 walk), and the run it must meet afterwards shortens
    /// with the noise (floored at [`SWING_MIN_SAMPLES`], the swing guard). A line that is a slow wave or an
    /// offset stays the line, as before.
    NoiseDeparture,
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
/// Version 9 (#351): the line is reached when this many consecutive samples stand within the
/// run's band. A single-sample zero crossing between a pre-swing and the edge is not the
/// line, and neither is the short in-band stretch of a bleed's or a ripple's own zero crossing:
/// the run has to be as long as those. Fitted on the labelled hits the coordinator staged for
/// #351 (Sessions C and D of the 2026-11-06 labels and the owner's hand-placed Session E; even
/// hits chose, odd hits checked): 6 and 8 lose the run inside the wider swings; 10 held on the
/// check half.
pub(crate) const LINE_RUN_SAMPLES: i64 = 10;
/// Version 9 (#351): the longest walk-back to the line (seconds) from the edge's first rising
/// sample. The swing before a stroke is short; a longer stretch off the line is the line itself
/// (a slow wave or an offset on a tom line), and there version 8's walk-back decides, as before.
/// Fitted on the same hits: 0.6 ms cut the walk short of the run behind a 13-sample swing; from
/// 0.8 ms up the result held to the 1 ms tried, so the shorter bound is kept.
pub(crate) const DEPARTURE_MAX_S: f64 = 0.000_8;
/// Version 9 (#351): the line level for the departure walk is the median over this stretch
/// (seconds) before the walk's foot, longer than [`LINE_LEVEL_S`]: the pre-swing occupies most
/// of the short window before the foot and tilts its median onto the swing itself (on the
/// labelled hits the short window put the line half a swing high and broke the run). Fitted on
/// the same hits: 12 samples failed, 24 held and 48 gave the same held-out result; 24 is the
/// smaller.
pub(crate) const DEPARTURE_LEVEL_S: f64 = 0.000_5;
/// Version 9 (#351): a sample of the departure walk stands off the line when `|x - line|`
/// exceeds this share of the hit's peak (percent); the run of line samples is counted in the
/// tighter [`DEPARTURE_RUN_PCT`] band instead. Fitted on the same hits: at the version 7
/// [`VIS_TAIL_PCT`] of 2 the swing of a session whose line carries bleed at about that level
/// never stood clear of it; 2.5 separated the two and 3 began to miss the small dips; 5 and
/// over lost the swings whole.
pub(crate) const DEPARTURE_VIS_PCT: f64 = 2.5;
/// Version 9 (#351), the run's band: once the walk has departed, only samples within this share
/// of the peak (the version 7 [`VIS_TAIL_PCT`]) count toward the run of line samples. The
/// looser [`DEPARTURE_VIS_PCT`] decides the departure alone: on the labelled hits the stroke's
/// first motion sometimes opens at 2 to 2.5 % of the peak, and in the loose band that sample
/// reads as line — the run then ends one sample late, on the motion's own first sample.
pub(crate) const DEPARTURE_RUN_PCT: f64 = VIS_TAIL_PCT;
/// Version 9 (#351), the stands-out fit: the departure counts as the stroke's first motion when
/// its largest excursion exceeds this multiple of the largest off-line excursion over
/// [`EDGE_NOISE_S`] before the run of line samples. Bleed at 3 % of the peak before a 3 % swing
/// does not stand out; the fitted swings stand 2 to 15 times over a line under 1 %. Fitted on
/// the same hits: 1.5 moved hand-placed hits of Session E that stand, 2.5 began to refuse real
/// swings; 2 held on the check half.
pub(crate) const DEPARTURE_STANDS_OUT: f64 = 2.0;
/// Version 9 (#351), the swing fit: the stretch between the run of line samples and the edge
/// must contain at least one excursion this many samples wide (a swing, not a flicker). At
/// 48 kHz 3 is the data's floor: the sinc lobes of #271 are 1 sample wide, every one of them,
/// so the walk stops there and version 8 decides. The 1-sample dip before a Session D snare's
/// wide swing passes because the swing after it is in the same stretch. Fitted on the same
/// hits: 2 and 3 give the same held-out result; 3 is the data's floor.
pub(crate) const SWING_MIN_SAMPLES: i64 = 3;
/// Version 9 (#351), the bridge fit: between the run of line samples and the edge, at most
/// this many consecutive samples may stand within the departure band. The stroke's first
/// motion runs into the edge; a run that is separated from it by a longer stretch of
/// sub-departure content is the line before an earlier flicker, not the line before the
/// motion (the flicker itself does not run into the edge). Measured in the loose departure
/// band, not the run's tighter one: the line's own level wobbles by about half a percent of
/// the peak between coarse feet, and a flicker that sits at the run band's edge would
/// otherwise flip with it. Fitted on the same hits: 3 refused the 1-to-2-sample dip before a
/// Session D snare's wide swing, which the owner counts as the first motion; 5 kept a
/// 2-sample flicker at 2.6 % of the peak that sits 5 samples before the swing (Session E
/// snare 93 of the staged excerpts, where the tool's stretch holds 5 such samples); 4 held
/// on the check half and moved no Session E hit.
pub(crate) const DEPARTURE_BRIDGE_SAMPLES: i64 = 4;
/// Version 10 (#362), the noise-scaled departure: the line is measured by its own noise, not by a fixed
/// share of the peak — one principle, one rule for all kits, no tuning to named hits. The visibility band
/// (`[DEPARTURE_VIS_PCT]` floored), the run's band (`[VIS_TAIL_PCT]` floored) and the run length
/// (`[SWING_MIN_SAMPLES]` floored) scale with the spread of the line samples before the stroke: the
/// standard deviation of the [`EDGE_NOISE_S`] before the walk's foot over the hit's peak, the measure
/// [`placement_diag`] reports as `line_noise`. Version 9's departure is the floor of the answer: the
/// noise-scaled walk runs only where version 9 refused. Refitted on the 2026-11-07 Training Mode labels
/// of Sessions A to J (7327 windows of 960 samples; even rows chose, odd rows checked; the Python port of
/// this rule in the #362 fit, aggregates only) over the rows where version 9 declines: the band multiple
/// 0.5 held; 1.0 and above took the wobble of the wobbliest snares for the stroke's own motion.
pub(crate) const DEPARTURE10_VIS_MULT: f64 = 0.5;
/// Version 10 (#362), the run's band multiple over the line noise, with [`VIS_TAIL_PCT`] as the floor.
/// Refitted on the same rows: 0.5 held; 2.0 let the wobble inside the run's band end the run early, on
/// the wobbliest lines.
pub(crate) const DEPARTURE10_RUN_MULT: f64 = 0.5;
/// Version 10 (#362), the run-length fit: the run of line samples the walk must meet is
/// `max([SWING_MIN_SAMPLES], round(K / sigma))` with K this constant — a shorter run on a noisier line is
/// the rule, not a tuning (a 3-sample floor is the swing guard: a run shorter than a swing is a flicker).
/// Refitted on the rows where version 9 declines (the sequential rule: version 9's arm is the
/// floor, the noise-scaled arm runs only there): 0.2 held (a 0.5 % line walks a 40-sample run
/// back, a 1 % line a 20-sample one); every 0.1-and-below grid point either refused the gentle
/// swings of the noisiest lines or left a session worse than version 9.
pub(crate) const DEPARTURE10_RUN_K: f64 = 0.2;
/// Version 10 (#362), the longest walk-back to the line (seconds), anchored at the walked-back rise
/// start `k0`, as version 9 anchors. The noise-scaled arm runs only where version 9 refused, so its
/// window may not be narrower than version 9's. Fitted on the labels of Sessions A to J: 1.2 ms held
/// and 1.6 ms and 2 ms gave the same held-out result, so the shorter bound is kept.
pub(crate) const DEPARTURE10_MAX_S: f64 = 0.001_2;
/// Version 10 (#362), the ramp refusal: the departure must leave the line as a motion, not as a
/// ramp — a slow drift (the owner's #294 shape: a drift to 10 % of the peak over 1 ms before the
/// swing) crosses the band with a per-sample step under the line's own noise, while a swing's
/// first step clears it. The threshold is the line noise with this share of the peak as the
/// floor, because a digitally silent line has no noise of its own.
pub(crate) const DEPARTURE10_RAMP_PCT: f64 = 0.5;
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
        EdgeTail::Line | EdgeTail::Departure | EdgeTail::NoiseDeparture => {
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
            // LINE_RUN_SAMPLES consecutive samples within the departure band: the last sample of that run is the
            // last line sample before the stroke's first motion. A single-sample zero crossing between a pre-swing
            // and the edge is not the line (the count resets on a sample off the line), and the walk counts only
            // once it has passed a sample that stands visibly off the line: without a departure there is no first
            // motion to walk before (the tail of a bleed or of band-limited pre-ringing is not the stroke's). The
            // walk counts only when it meets the run within DEPARTURE_MAX_S of the edge's first rising sample;
            // otherwise version 8's walk-back decides (a slow wave or an offset on a tom line is the line itself,
            // as before). Two fits guard what the walk may take for the first motion: it must STAND OUT from the
            // line before it (its largest excursion a multiple of the largest off-line excursion over
            // EDGE_NOISE_S before the run) and be a SWING, not a flicker (the stretch between the run and the
            // edge contains an excursion SWING_MIN_SAMPLES wide). The line level is measured over the longer
            // DEPARTURE_LEVEL_S window: the pre-swing occupies most of the LINE_LEVEL_S stretch before the foot
            // and tilts its median onto the swing itself
            // version 10 (#362): version 9's departure is the FLOOR, not an alternative. The arm runs
            // version 9's own walk first — version 9's fixed shares of the peak, its anchor at the
            // walked-back rise start `k0`, its run of [`LINE_RUN_SAMPLES`]: when version 9 accepts a
            // departure, version 10 returns exactly version 9's answer, so for every synthetic case of
            // the version 9 suite the two decide alike and version 10 may only ADD departures that
            // version 9 refused. Only when version 9's arm refuses does the noise-scaled walk get a
            // chance: the visibility band, the run's band and the run length scale with sigma (the
            // standard deviation of the [`EDGE_NOISE_S`] before the walk's foot over the peak, the
            // measure `placement_diag` reports as `line_noise`), the fixed shares only as a floor. The
            // ramp refusal (the departure must leave the line as a motion, not as a ramp — the owner's
            // #294 shape) guards only the noise-scaled arm: it never touches a departure version 9
            // would have found, because version 9's answer is returned before it is reached.
            EdgeTail::Departure | EdgeTail::NoiseDeparture => {
                let line9 = {
                    let from = (foot - samples(DEPARTURE_LEVEL_S, srf)).max(0);
                    let mut v: Vec<f64> = (from..foot).map(at).collect();
                    if v.is_empty() {
                        0.0
                    } else {
                        let mid = v.len() / 2;
                        *v.select_nth_unstable_by(mid, f64::total_cmp).1
                    }
                };
                let off9 = |i: i64| at(i) - line9;
                // one departure walk, parameterised: walk back over every sample that stands off the
                // line in the loose band `vis`, then meet `run` consecutive samples within the
                // tighter run's band `vis_run` no later than `lo_d`; the three filters of version 9
                // (stands out, swing not flicker, runs into the edge) follow
                let arm = |vis: f64, vis_run: f64, run: i64, lo_d: i64| -> Option<i64> {
                    let mut i = k;
                    let mut in_band = 0i64;
                    let mut departed = false;
                    let mut hit = None;
                    while i >= lo_d {
                        // the departure is judged in the loose band; once departed, the run counts
                        // only samples that stand within the tighter run's band (the stroke's first
                        // motion can open just under the loose band)
                        if off9(i).abs() > vis || (departed && off9(i).abs() > vis_run) {
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
                    hit.filter(|&h| {
                        // the first motion stands out from the line before it: its largest excursion over
                        // the largest off-line excursion of the EDGE_NOISE_S before the run of line
                        // samples, floored at the departure band itself — a ripple on a very quiet line
                        // stands over the line's own excursions yet is no motion (#351 verify: the
                        // ripples stood 2x over a line under 1 % and passed the unfloored test)
                        let run_start = h - run + 1;
                        let ref_lo = (run_start - samples(EDGE_NOISE_S, srf)).max(0);
                        let reference = (ref_lo..run_start)
                            .map(|m| off9(m).abs())
                            .fold(0.0_f64, f64::max)
                            .max(vis);
                        let dep_max = (h..=k0).map(|m| off9(m).abs()).fold(0.0_f64, f64::max);
                        dep_max > DEPARTURE_STANDS_OUT * reference
                    })
                    .filter(|&h| {
                        // the first motion is a swing, not a flicker: the stretch between the run and the edge
                        // contains at least one excursion SWING_MIN_SAMPLES wide
                        let mut best = 0i64;
                        let mut cur = 0i64;
                        let mut prev = 0i64;
                        for m in h + 1..=k0 {
                            let s = off9(m).signum() as i64;
                            if s != 0 && s == prev {
                                cur += 1;
                            } else {
                                cur = 1;
                                prev = s;
                            }
                            best = best.max(cur);
                        }
                        best >= SWING_MIN_SAMPLES
                    })
                    .filter(|&h| {
                        // the first motion runs into the edge: between the run and the edge at
                        // most DEPARTURE_BRIDGE_SAMPLES consecutive samples may stand within the
                        // departure band. A longer stretch means the run sits before an earlier
                        // flicker (one that does not run into the edge: sub-departure content
                        // separates it from the swing): the walk has not reached the line before
                        // the stroke's motion yet, and version 8's walk-back decides
                        let mut cur = 0i64;
                        let mut best = 0i64;
                        for m in h + 1..=k0 {
                            if off9(m).abs() <= vis {
                                cur += 1;
                                best = best.max(cur);
                            } else {
                                cur = 0;
                            }
                        }
                        best <= DEPARTURE_BRIDGE_SAMPLES
                    })
                };
                // version 9's arm, unchanged: the floor every version 10 answer rests on
                let v9 = arm(
                    pk * DEPARTURE_VIS_PCT / 100.0,
                    pk * DEPARTURE_RUN_PCT / 100.0,
                    LINE_RUN_SAMPLES.max(1),
                    (k0 - samples(DEPARTURE_MAX_S, srf)).max(1),
                );
                if tail_rule == EdgeTail::Departure {
                    v9
                } else {
                    let sigma = {
                        let from = (foot - samples(EDGE_NOISE_S, srf)).max(0);
                        let w: Vec<f64> = (from..foot).map(|i| at(i) - line).collect();
                        if w.is_empty() || pk <= 0.0 {
                            0.0
                        } else {
                            let m = w.len() as f64;
                            let mean = w.iter().sum::<f64>() / m;
                            (w.iter().map(|d| (d - mean) * (d - mean)).sum::<f64>() / m).sqrt() / pk
                        }
                    };
                    // the noise-scaled arm runs only where version 9 refused; a shorter run on a
                    // noisier line is the rule, not a tuning (the run is floored at the swing guard),
                    // and the ramp refusal guards the departures it adds
                    v9.or_else(|| {
                        let vis = (DEPARTURE_VIS_PCT / 100.0).max(DEPARTURE10_VIS_MULT * sigma) * pk;
                        let vis_run = (VIS_TAIL_PCT / 100.0).max(DEPARTURE10_RUN_MULT * sigma) * pk;
                        let run = if sigma > 0.0 {
                            (DEPARTURE10_RUN_K / sigma).round() as i64
                        } else {
                            0
                        }
                        .max(SWING_MIN_SAMPLES);
                        arm(vis, vis_run, run, (k0 - samples(DEPARTURE10_MAX_S, srf)).max(1)).filter(|&h| {
                            // the departure leaves the line as a MOTION, not as a ramp (the
                            // owner's #294 shape): a slow drift crosses the band with a
                            // per-sample step under the line's own noise, while a swing's
                            // first step clears it. The threshold is the line noise with a
                            // fixed share of the peak as the floor (a digitally silent line
                            // has none); the line's level cancels in the difference
                            h + 1 > k0 || (at(h + 1) - at(h)).abs() > sigma.max(DEPARTURE10_RAMP_PCT / 100.0 * pk)
                        })
                    })
                }
                .unwrap_or_else(|| line_walk(k, k0))
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
