//! C16 (#281 item 20, #352): refinement agreement and polarity per hit.
//!
//! For each hit, three onset estimates are computed and stored on the hit without
//! changing the hit times or the hit set (the app wiring and the export use are a
//! follow-up ticket):
//!
//! 1. the first-motion foot: the first sample in the search window reaching the
//!    window peak minus [`AGREEMENT_FM_PEAK_DB`] (the refinement's peak margin, the
//!    rule of refine version 3). On a clean sample-exact onset this lands within a
//!    few samples of the true onset; the 25 % rule of refinement version 1 was tried
//!    first and sits 1 ms and more after the onset on slow low-frequency attacks
//!    (a 60 Hz kick: +61 samples), which broke the three-estimator agreement;
//! 2. the AIC change point (the classic two-phase AIC minimum) inside the attack
//!    window ([`AGREEMENT_AIC_PRE_S`] before to [`AGREEMENT_AIC_POST_S`] after the
//!    hit). On a transient with a silent pre-phase the two-phase AIC is degenerate:
//!    its minimum slides to the search-window edge (measured on the synthetic kit:
//!    +500 to +641 samples). A change point at the edge of its window is therefore
//!    not an estimate and comes back `None` ([`AGREEMENT_EDGE_GUARD`]);
//! 3. the cross-correlation lag against the track template (the element-wise median of
//!    the hit windows of the track, [`AGREEMENT_TPL_PRE_S`] before to
//!    [`AGREEMENT_TPL_POST_S`] after each hit), searched [`AGREEMENT_XCORR_SPAN_S`]
//!    around the template-aligned position. The lag is measured relative to the
//!    template's own onset (the template's first [`AGREEMENT_TPL_PRE_S`] samples are
//!    the pre-onset audio), not to the window start; a lag at the search edge is
//!    `None`.
//!
//! `timing_confidence` is the spread of the three estimates in samples (max - min);
//! `polarity` is the sign of the best normalized cross-correlation (`+1` normal,
//! `-1` phase-inverted). A hit whose window is silent or too short gets no spread
//! (`None`); the polarity is `0` when no correlation score was computable.
//!
//! The estimates are diagnostic only: no step of the analysis reads them, so the
//! hit times, the velocities and the exports are bit for bit what they were before
//! (checked by the golden files of the frozen references).

/// The template window before each hit (seconds).
pub const AGREEMENT_TPL_PRE_S: f64 = 0.002;
/// The template window after each hit (seconds).
pub const AGREEMENT_TPL_POST_S: f64 = 0.013;
/// The estimate search window before the hit (seconds): the refinement's `lo`.
pub const AGREEMENT_LO_S: f64 = -0.002;
/// The estimate search window after the hit (seconds): the refinement's `hi`.
pub const AGREEMENT_HI_S: f64 = 0.012;
/// The cross-correlation search before and after the template-aligned position (seconds).
pub const AGREEMENT_XCORR_SPAN_S: f64 = 0.005;
/// The AIC window before the hit (seconds).
pub const AGREEMENT_AIC_PRE_S: f64 = 0.004;
/// The AIC window after the hit (seconds): the attack region. A wider window lets the
/// degenerate minimum of the two-phase AIC slide further past the onset (see the module
/// documentation).
pub const AGREEMENT_AIC_POST_S: f64 = 0.002;
/// The first-motion threshold under the window peak in dB: the refinement's peak margin
/// ([`crate::refine::FM_PEAK_DB`]), the rule of refine version 3.
pub const AGREEMENT_FM_PEAK_DB: f64 = 33.0;
/// An estimate within this many samples of its search-window edge is not an estimate:
/// it comes back `None` instead of sitting at the edge.
pub const AGREEMENT_EDGE_GUARD: i64 = 2;
/// A window quieter than this peak is treated as silent: no estimates (the refinement's limit).
pub const AGREEMENT_SILENCE: f32 = 1e-4;

/// One result of [`agreement_track`]: the spread of the three onset estimates in samples
/// (`None` when the hit's window is silent or the track has no template) and the polarity
/// sign (`+1`, `-1`, or `0` when it was not computable).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Agreement {
    /// Spread of the three estimates in samples (max - min).
    pub timing_confidence: Option<f64>,
    /// `+1` normal, `-1` phase-inverted, `0` not computable.
    pub polarity: i32,
}

/// The element-wise median of the hit windows: the track template. Hits whose window
/// would leave the signal are left out; with no full window the template is empty.
fn template(x: &[f32], sr: u32, hits: &[f64]) -> Vec<f64> {
    let srf = sr as f64;
    let n = x.len() as i64;
    let pre = (AGREEMENT_TPL_PRE_S * srf).round() as i64;
    let post = (AGREEMENT_TPL_POST_S * srf).round() as i64;
    let len = (pre + post) as usize;
    let mut cuts: Vec<Vec<f64>> = Vec::new();
    for &h in hits {
        let s = (h * srf).round() as i64;
        if s - pre >= 0 && s + post <= n {
            cuts.push(
                x[(s - pre) as usize..(s + post) as usize]
                    .iter()
                    .map(|&v| v as f64)
                    .collect(),
            );
        }
    }
    if cuts.is_empty() {
        return Vec::new();
    }
    (0..len)
        .map(|i| {
            let mut col: Vec<f64> = cuts.iter().map(|c| c[i]).collect();
            col.sort_by(f64::total_cmp);
            let m = col.len();
            if m % 2 == 1 {
                col[m / 2]
            } else {
                (col[m / 2 - 1] + col[m / 2]) / 2.0
            }
        })
        .collect()
}

/// The first sample in `[from, to)` reaching the window peak minus
/// [`AGREEMENT_FM_PEAK_DB`] (the first-motion foot, the rule of refine version 3),
/// or `None` when the window is empty, silent, or the foot is at the window's start
/// edge ([`AGREEMENT_EDGE_GUARD`]): a foot at the very start means the motion began
/// before the window.
fn env_first_motion(x: &[f32], from: i64, to: i64) -> Option<i64> {
    let (f, t) = (from.max(0) as usize, (to.max(0) as usize).min(x.len()));
    if t <= f {
        return None;
    }
    let seg = &x[f..t];
    let mx = seg.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if mx < AGREEMENT_SILENCE {
        return None;
    }
    let thr = (mx as f64 * 10f64.powf(-AGREEMENT_FM_PEAK_DB / 20.0)) as f32;
    seg.iter()
        .position(|v| v.abs() >= thr)
        .map(|i| f as i64 + i as i64)
        .filter(|v| v - f as i64 >= AGREEMENT_EDGE_GUARD)
}

/// The largest `|x|` in `[from, to)`, clipped to the signal; 0 for an empty range.
fn max_abs(x: &[f32], from: i64, to: i64) -> f32 {
    let f = from.max(0) as usize;
    let t = (to.max(0) as usize).min(x.len());
    if t <= f {
        return 0.0;
    }
    x[f..t].iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// The AIC change point of the two-phase model in `[from, to)`: the split `k` minimising
/// `k * ln(var_pre) + (n - k) * ln(var_post)` (the +4 parameter term is constant and
/// dropped). Returns the absolute sample of the change point, or `None` when the window
/// is too short or a side has zero variance.
fn aic_change_point(x: &[f32], from: i64, to: i64) -> Option<i64> {
    let (f, t) = (from.max(0), to.max(0).min(x.len() as i64));
    if t <= f {
        return None;
    }
    // a silent window has no change point (the refinement's silence limit)
    if max_abs(x, f, t) < AGREEMENT_SILENCE {
        return None;
    }
    let n = t - f;
    if n < 8 {
        return None;
    }
    // prefix sums of x and x^2 for O(1) segment moments
    let mut s1 = vec![0.0f64; (t - f + 1) as usize];
    let mut s2 = vec![0.0f64; (t - f + 1) as usize];
    for i in f..t {
        let v = x[i as usize] as f64;
        s1[(i - f + 1) as usize] = s1[(i - f) as usize] + v;
        s2[(i - f + 1) as usize] = s2[(i - f) as usize] + v * v;
    }
    let at = |s: &Vec<f64>, i: usize| s[i] - s[0];
    let var = |s1v: f64, s2v: f64, m: i64| {
        if m <= 1 {
            return f64::INFINITY;
        }
        let mean = s1v / m as f64;
        (s2v - m as f64 * mean * mean) / (m - 1) as f64
    };
    let mut best = (f64::INFINITY, 0i64);
    for k in 2..=(n - 2) {
        let v0 = var(at(&s1, k as usize), at(&s2, k as usize), k);
        let v1 = var(
            at(&s1, n as usize) - at(&s1, k as usize),
            at(&s2, n as usize) - at(&s2, k as usize),
            n - k,
        );
        if v0 < 0.0 || v1 < 0.0 {
            continue;
        }
        // a constant side (zero variance) is a legitimate phase: floor the log
        let aic = k as f64 * v0.max(1e-30).ln() + (n - k) as f64 * v1.max(1e-30).ln();
        if aic < best.0 {
            best = (aic, f + k);
        }
    }
    if best.0.is_infinite() {
        return None;
    }
    // a change point at the edge of its window is the degenerate minimum of the
    // two-phase AIC on a transient, not an estimate
    if best.1 - (f + 2) < AGREEMENT_EDGE_GUARD || (t - 2) - best.1 < AGREEMENT_EDGE_GUARD {
        return None;
    }
    Some(best.1)
}

/// The normalized cross-correlation score of the signal segment starting at `from`
/// (length `tpl.len()`) against the template, or `None` when the segment leaves the
/// signal or either side is silent.
fn xcorr_score(x: &[f32], from: i64, tpl: &[f64]) -> Option<f64> {
    let to = from + tpl.len() as i64;
    if from < 0 || to > x.len() as i64 {
        return None;
    }
    let seg = &x[from as usize..to as usize];
    let mut num = 0.0;
    let mut eseg = 0.0;
    let mut etpl = 0.0;
    for i in 0..tpl.len() {
        let s = seg[i] as f64;
        num += s * tpl[i];
        eseg += s * s;
        etpl += tpl[i] * tpl[i];
    }
    if eseg <= 0.0 || etpl <= 0.0 {
        return None;
    }
    Some(num / (eseg * etpl).sqrt())
}

/// The best normalized cross-correlation of the template in the signal: the lag `d`
/// (relative to the template-aligned position `base`) with the largest `|score|`, and
/// whether that lag sits at the edge of the search span (then it is not an estimate).
/// `None` when nothing was computable.
fn xcorr_lag(x: &[f32], base: i64, span: i64, tpl: &[f64]) -> Option<(i64, f64, bool)> {
    let mut best: Option<(i64, f64)> = None;
    for d in -span..=span {
        if let Some(score) = xcorr_score(x, base + d, tpl)
            && best.is_none_or(|(_, s)| score.abs() > s.abs())
        {
            best = Some((d, score));
        }
    }
    let (d, score) = best?;
    Some((d, score, span - d.abs() < AGREEMENT_EDGE_GUARD))
}

/// The agreement of every hit of a track: the estimates of `hits` (seconds, the final
/// refined times, in the order given) against the signal `x` at `sr`. The hit times and
/// the hit set are never touched.
pub fn agreement_track(x: &[f32], sr: u32, hits: &[f64]) -> Vec<Agreement> {
    let srf = sr as f64;
    let tpl = template(x, sr, hits);
    let lo = (AGREEMENT_LO_S * srf).round() as i64;
    let hi = (AGREEMENT_HI_S * srf).round() as i64;
    let span = (AGREEMENT_XCORR_SPAN_S * srf).round() as i64;
    let aic_lo = -(AGREEMENT_AIC_PRE_S * srf).round() as i64;
    let aic_hi = (AGREEMENT_AIC_POST_S * srf).round() as i64;
    hits.iter()
        .map(|&h| {
            let s = (h * srf).round() as i64;
            let mut out = Agreement::default();
            // 1. the first-motion foot (the refinement's peak margin)
            let e1 = env_first_motion(x, s + lo, s + hi);
            // 2. the AIC change point
            let e2 = aic_change_point(x, s + aic_lo, s + aic_hi);
            // 3. the cross-correlation lag against the track template, measured relative
            // to the template's own onset (its first pre-window holds the pre-onset audio)
            let tpl_pre = (AGREEMENT_TPL_PRE_S * srf).round() as i64;
            let mut e3: Option<i64> = None;
            if !tpl.is_empty()
                && s + lo - span >= 0
                && s + hi + span <= x.len() as i64
                && let Some((d, score, at_edge)) = xcorr_lag(x, s + lo, span, &tpl)
            {
                // the polarity comes from the best score even when the lag itself
                // sits at the search edge and is not an estimate
                out.polarity = if score >= 0.0 { 1 } else { -1 };
                if !at_edge {
                    e3 = Some(s + lo + d + tpl_pre);
                }
            }
            let ests: Vec<i64> = [e1, e2, e3].into_iter().flatten().collect();
            if ests.len() >= 2 {
                let mn = *ests.iter().min().unwrap();
                let mx = *ests.iter().max().unwrap();
                out.timing_confidence = Some((mx - mn) as f64);
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests;
