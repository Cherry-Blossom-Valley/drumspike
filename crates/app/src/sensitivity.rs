//! P3-18: the Sensitivity slider's mapping to the five class thresholds, and the retry steps
//! for a track that gives no hit (a soft track: a kick sub mic, a resonant head).
//!
//! The page (`static/app.js`, `buildParamsPanel`) has the same formulas; the retry in
//! `jobs.rs` uses these. The slider value `v` runs from 0 to [`MAX`]:
//!
//! | `v` | scale | thresholds (kick, snare, tom, hat, cymbal) |
//! |---|---|---|
//! | 0 | 1.6 | 0.35, 0.38, 0.51, 0.35, 0.48 |
//! | 50 (the default) | 1.0 | 0.22, 0.24, 0.32, 0.22, 0.30 |
//! | 100 | 0.4 | 0.088, 0.096, 0.128, 0.088, 0.12 (the end of the slider before P3-18) |
//! | 100 to 150 | 0.4 down to 0.01 / 0.22, geometric | the lowest threshold goes from 0.088 to 0.01 |
//!
//! From 0 to 100 the formula is the one of the slider before P3-18 (`1.6 - 1.2 v / 100`), so an
//! old project's thresholds map to the same slider value and the same thresholds come back. The
//! floor of the clamp went from 0.05 to [`FLOOR`]; in the old range the lowest value is 0.088, so
//! the old floor never applied.

use drumspike_core::detect::N_CLASSES;

/// The thresholds of the default sensitivity (`AnalyzeParams::default().thresholds`).
pub const BASE: [f64; N_CLASSES] = [0.22, 0.24, 0.32, 0.22, 0.30];
/// The slider value of the defaults.
pub const DEFAULT: f64 = 50.0;
/// The last slider value of the range before P3-18 (scale 0.4).
pub const OLD_MAX: f64 = 100.0;
/// The last slider value.
pub const MAX: f64 = 150.0;
/// The lowest threshold a slider value gives (at [`MAX`], for the classes with the base 0.22).
pub const FLOOR: f64 = 0.01;
/// The highest threshold.
pub const CEIL: f64 = 0.85;
/// The scale at [`OLD_MAX`].
const SCALE_AT_OLD_MAX: f64 = 0.4;
/// The slider values the retry of a track without hits tries, in order: the lowest threshold
/// about 0.10, 0.05, 0.02 and 0.01.
pub const RETRY_STEPS: [f64; 4] = [95.0, 113.0, 134.0, 150.0];

/// The lowest base threshold (0.22): the class that reaches [`FLOOR`] at [`MAX`].
fn base_min() -> f64 {
    BASE.iter().copied().fold(f64::INFINITY, f64::min)
}

/// The mean of the base thresholds (0.26): the inverse mapping reads the mean of a track's thresholds.
fn base_mean() -> f64 {
    BASE.iter().sum::<f64>() / N_CLASSES as f64
}

/// The scale at the end of the slider: `FLOOR / 0.22`.
fn scale_at_max() -> f64 {
    FLOOR / base_min()
}

/// The factor every threshold is multiplied with at the slider value `v` (clamped to 0 to [`MAX`]).
pub fn scale_of(v: f64) -> f64 {
    let v = if v.is_finite() { v.clamp(0.0, MAX) } else { DEFAULT };
    if v <= OLD_MAX {
        // the operation order of the page before P3-18: the same doubles
        1.6 - 1.2 * (v / OLD_MAX)
    } else {
        // geometric from 0.4 at 100 to FLOOR / 0.22 at 150
        let frac = (v - OLD_MAX) / (MAX - OLD_MAX);
        SCALE_AT_OLD_MAX * (scale_at_max() / SCALE_AT_OLD_MAX).powf(frac)
    }
}

/// The five thresholds at the slider value `v`.
pub fn thresholds_of(v: f64) -> [f64; N_CLASSES] {
    let s = scale_of(v);
    BASE.map(|b| (b * s).clamp(FLOOR, CEIL))
}

/// The slider value of a track's thresholds: the mean of the five against the mean of the base
/// (the inverse of [`thresholds_of`] for every value of the slider; a hand-written list gives the
/// value of its mean). Not rounded; the page rounds for the slider.
pub fn sensitivity_of(th: &[f64; N_CLASSES]) -> f64 {
    let mean = th.iter().sum::<f64>() / N_CLASSES as f64;
    let s = mean / base_mean();
    let v = if s >= SCALE_AT_OLD_MAX {
        (1.6 - s) / 1.2 * OLD_MAX
    } else {
        OLD_MAX + (MAX - OLD_MAX) * (SCALE_AT_OLD_MAX / s).ln() / (SCALE_AT_OLD_MAX / scale_at_max()).ln()
    };
    if v.is_finite() { v.clamp(0.0, MAX) } else { MAX }
}

/// The slider value of a gate scale `s` (the inverse of [`scale_of`], A4): the value whose
/// thresholds are `base * s`. Not rounded; the page rounds for the slider.
pub fn slider_of_scale(s: f64) -> f64 {
    let v = if s >= SCALE_AT_OLD_MAX {
        (1.6 - s) / 1.2 * OLD_MAX
    } else {
        OLD_MAX + (MAX - OLD_MAX) * (SCALE_AT_OLD_MAX / s).ln() / (SCALE_AT_OLD_MAX / scale_at_max()).ln()
    };
    if v.is_finite() { v.clamp(0.0, MAX) } else { MAX }
}

/// The retry steps above the sensitivity of `current`: the values of [`RETRY_STEPS`] that lower
/// the thresholds, each with its thresholds. Empty at the end of the slider.
pub fn retry_steps(current: &[f64; N_CLASSES]) -> Vec<(f64, [f64; N_CLASSES])> {
    let now = sensitivity_of(current);
    let lowest = current.iter().copied().fold(f64::INFINITY, f64::min);
    RETRY_STEPS
        .iter()
        .filter(|&&v| v > now + 0.5)
        .map(|&v| (v, thresholds_of(v)))
        .filter(|(_, th)| th.iter().copied().fold(f64::INFINITY, f64::min) < lowest - 1e-9)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slider of P3-14a (static/app.js before P3-18): 0 to 100, clamp 0.05 to 0.85.
    fn old(v: f64) -> [f64; N_CLASSES] {
        let scale = 1.6 - 1.2 * (v / 100.0);
        BASE.map(|b| (b * scale).clamp(0.05, 0.85))
    }

    #[test]
    fn the_old_range_gives_the_same_thresholds_as_before() {
        for v in 0..=100 {
            assert_eq!(thresholds_of(v as f64), old(v as f64), "v = {v}");
        }
        assert_eq!(thresholds_of(DEFAULT), BASE);
        assert_eq!(
            thresholds_of(DEFAULT),
            drumspike_core::analyze::AnalyzeParams::default().thresholds
        );
    }

    #[test]
    fn the_new_range_reaches_the_floor_and_falls_without_a_step() {
        let end = thresholds_of(MAX);
        assert!(
            (end[0] - FLOOR).abs() < 1e-12 && (end[3] - FLOOR).abs() < 1e-12,
            "{end:?}"
        );
        assert!(end.iter().all(|t| (FLOOR..0.02).contains(t)), "{end:?}");
        let mut last = thresholds_of(0.0);
        for v in 1..=150 {
            let th = thresholds_of(v as f64);
            for k in 0..N_CLASSES {
                assert!(th[k] < last[k], "v = {v}: {th:?} is not below {last:?}");
            }
            last = th;
        }
        // continuous at the old end
        let a = thresholds_of(100.0);
        let b = thresholds_of(100.001);
        assert!((a[0] - b[0]).abs() < 1e-4, "{a:?} {b:?}");
        assert_eq!(thresholds_of(f64::NAN), BASE, "a bad value is the default");
        assert_eq!(thresholds_of(999.0), thresholds_of(MAX));
        assert_eq!(thresholds_of(-5.0), thresholds_of(0.0));
    }

    #[test]
    fn the_inverse_gives_the_slider_value_back() {
        for v in 0..=150 {
            let back = sensitivity_of(&thresholds_of(v as f64));
            assert!((back - v as f64).abs() < 1e-6, "v = {v}: {back}");
        }
        assert!((sensitivity_of(&BASE) - DEFAULT).abs() < 1e-12);
        // the readout of P3-14a divided the mean by 0.32 and showed 66 for the defaults; now 50
        assert_eq!(sensitivity_of(&[0.3; 5]).round(), 37.0);
        assert_eq!(
            sensitivity_of(&[0.0; 5]),
            MAX,
            "zero thresholds end at the slider's end"
        );
        assert_eq!(sensitivity_of(&[2.0; 5]), 0.0);
    }

    #[test]
    fn the_retry_steps_lower_the_thresholds_from_the_current_value_on() {
        let steps = retry_steps(&BASE);
        assert_eq!(steps.iter().map(|s| s.0).collect::<Vec<_>>(), RETRY_STEPS);
        let lows: Vec<f64> = steps.iter().map(|s| (s.1[0] * 1000.0).round() / 1000.0).collect();
        assert_eq!(lows, [0.101, 0.05, 0.02, 0.01]);
        assert_eq!(retry_steps(&thresholds_of(113.0)).len(), 2, "134 and 150 are left");
        assert_eq!(retry_steps(&thresholds_of(149.8)), vec![], "150 is within half a step");
        assert!(retry_steps(&thresholds_of(MAX)).is_empty());
        assert_eq!(retry_steps(&thresholds_of(0.0)).len(), 4);
    }
}
