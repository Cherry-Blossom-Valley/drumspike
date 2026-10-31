//! Velocities: `engine.peak_in` and `engine.velocity_map`, ported operation by operation.

use crate::pyops::{py_round, window};

/// `engine.peak_in(x, t, sr)`: max |x| in `[t - 5 ms, t + 60 ms)`, 0.0 for an empty window.
pub fn peak_in(x: &[f32], t: f64, sr: u32) -> f64 {
    let (i0, i1) = window(x.len(), t, sr, 0.005, 0.060);
    let seg = &x[i0..i1];
    if seg.is_empty() {
        return 0.0;
    }
    seg.iter().fold(0.0f32, |m, v| m.max(v.abs())) as f64
}

/// The dB-to-velocity map of `engine.velocity_map`: `lo_db` maps to `lo_v`, `hi_db` to
/// `hi_v`, linear in between, clamped outside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VelocityMap {
    /// Peak level that maps to `lo_v` (default -40 dBFS).
    pub lo_db: f64,
    /// Peak level that maps to `hi_v` (default -6 dBFS).
    pub hi_db: f64,
    /// Lowest velocity (default 20).
    pub lo_v: i64,
    /// Highest velocity (default 127).
    pub hi_v: i64,
}

impl Default for VelocityMap {
    fn default() -> Self {
        VelocityMap {
            lo_db: -40.0,
            hi_db: -6.0,
            lo_v: 20,
            hi_v: 127,
        }
    }
}

/// The peak level of the hit at `t` in dBFS: [`peak_in`] as `20 log10`, at least -180 dB
/// (P4-3: the loudness histogram; [`velocity_map`] uses the same value).
pub fn peak_db(x: &[f32], t: f64, sr: u32) -> f64 {
    20.0 * peak_in(x, t, sr).max(1e-9).log10()
}

/// #293 (B8): the attack time (ms) of the hit at `t`, in the window of [`peak_in`]: from the first sample that reaches
/// 10 % of the window's peak to the peak. A stroke on its own mic rises in well under a millisecond; bleed of another
/// drum arrives smeared over several. 0.0 for an empty or silent window.
pub fn attack_ms(x: &[f32], t: f64, sr: u32) -> f64 {
    let (i0, i1) = window(x.len(), t, sr, 0.005, 0.060);
    let seg = &x[i0..i1];
    let (peak_i, peak) = seg
        .iter()
        .map(|v| v.abs())
        .enumerate()
        .fold((0, 0.0f32), |(bi, bv), (i, v)| if v > bv { (i, v) } else { (bi, bv) });
    if peak <= 0.0 {
        return 0.0;
    }
    let start = seg[..=peak_i]
        .iter()
        .position(|v| v.abs() >= 0.1 * peak)
        .unwrap_or(peak_i);
    (peak_i - start) as f64 * 1000.0 / f64::from(sr)
}

/// The velocity of a peak level `d` (dBFS) with the map `m` (P4-3: the page computes the same
/// numbers while the engineer drags a handle of the loudness histogram).
pub fn velocity_of_db(d: f64, m: &VelocityMap) -> i64 {
    let denom = (m.hi_db - m.lo_db).max(1e-6);
    let span = (m.hi_v - m.lo_v) as f64;
    let vv = m.lo_v + py_round((d - m.lo_db) / denom * span);
    vv.max(m.lo_v).min(m.hi_v)
}

/// `engine.velocity_map(x, sr, times)`: dB-mapped velocities, Python `round` = ties to even.
pub fn velocity_map(x: &[f32], sr: u32, times: &[f64], m: &VelocityMap) -> Vec<i64> {
    times.iter().map(|&t| velocity_of_db(peak_db(x, t, sr), m)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    /// Mono signal with a single-sample peak of exactly `db` dBFS at time `t` (`tests/test_engine.py`).
    fn spike_at(db: f64, t: f64) -> Vec<f32> {
        let mut x = vec![0.0f32; SR as usize];
        x[(t * SR as f64) as usize] = 10f64.powf(db / 20.0) as f32;
        x
    }

    #[test]
    fn peak_in_finds_the_peak_and_handles_empty_windows() {
        let x = spike_at(-6.0, 0.5);
        assert_eq!(peak_in(&x, 0.5, SR), 10f64.powf(-0.3) as f32 as f64);
        assert_eq!(peak_in(&x, 0.5 + 0.0051, SR), 0.0, "peak just before the window");
        assert_eq!(peak_in(&x, 5.0, SR), 0.0, "window past the end is empty");
    }

    /// Mono signal that rises linearly from 0 to 0.5 over `rise_ms` after `t` and holds (#293 fixtures).
    fn rise_at(rise_ms: f64, t: f64) -> Vec<f32> {
        let a = (t * f64::from(SR)) as usize;
        let n = (rise_ms / 1000.0 * f64::from(SR)) as usize;
        (0..SR as usize)
            .map(|i| {
                if i < a {
                    0.0
                } else {
                    0.5 * (i - a).min(n) as f32 / n as f32
                }
            })
            .collect()
    }

    #[test]
    fn attack_ms_is_short_for_a_sharp_stroke_and_long_for_a_smeared_bleed() {
        // a stroke rising in 0.5 ms: 90 % of it between the 10 % crossing and the peak
        let sharp = attack_ms(&rise_at(0.5, 0.5), 0.5, SR);
        assert!(sharp > 0.3 && sharp < 0.6, "sharp {sharp}");
        // bleed rising over 12 ms
        let smeared = attack_ms(&rise_at(12.0, 0.5), 0.5, SR);
        assert!(smeared > 10.0 && smeared < 12.5, "smeared {smeared}");
        assert_eq!(attack_ms(&vec![0.0; 4800], 0.05, SR), 0.0, "silence has no attack");
        assert_eq!(
            attack_ms(&spike_at(-6.0, 0.5), 0.5, SR),
            0.0,
            "a single sample peak rises in no time"
        );
    }

    #[test]
    fn velocity_maps_lo_db_to_lo_v_and_hi_db_to_hi_v() {
        let m = VelocityMap::default();
        assert_eq!(velocity_map(&spike_at(-40.0, 0.1), SR, &[0.1], &m), vec![20]);
        assert_eq!(velocity_map(&spike_at(-6.0, 0.2), SR, &[0.2], &m), vec![127]);
    }

    #[test]
    fn velocity_clamps_outside_range() {
        let m = VelocityMap::default();
        assert_eq!(velocity_map(&spike_at(-120.0, 0.1), SR, &[0.1], &m), vec![20]);
        assert_eq!(velocity_map(&spike_at(0.0, 0.1), SR, &[0.1], &m), vec![127]);
        assert_eq!(velocity_map(&vec![0.0f32; 100], SR, &[0.001], &m), vec![20], "silence");
    }

    #[test]
    fn peak_db_and_velocity_of_db_are_the_two_halves_of_velocity_map() {
        let m = VelocityMap::default();
        let x = spike_at(-23.0, 0.1);
        let d = peak_db(&x, 0.1, SR);
        assert!((d + 23.0).abs() < 1e-5, "{d}");
        assert_eq!(velocity_map(&x, SR, &[0.1], &m), vec![velocity_of_db(d, &m)]);
        assert_eq!(peak_db(&x, 3.0, SR), -180.0, "silence is -180 dBFS");
        assert_eq!(velocity_of_db(-40.0, &m), 20);
        assert_eq!(velocity_of_db(-6.0, &m), 127);
        assert_eq!(velocity_of_db(5.0, &m), 127);
        // ties to even: (d + 40) / 34 * 107 = 53.5 rounds to 54, 52.5 to 52
        let tie = |k: f64| k * 34.0 / 107.0 - 40.0;
        assert_eq!(velocity_of_db(tie(53.5), &m) - 20, 54);
        assert_eq!(velocity_of_db(tie(52.5), &m) - 20, 52);
        // a ceiling at or below the floor: the denominator is 1e-6
        let flat = VelocityMap {
            lo_db: -10.0,
            hi_db: -10.0,
            ..m
        };
        assert_eq!(velocity_of_db(-10.5, &flat), 20);
        assert_eq!(velocity_of_db(-9.5, &flat), 127);
    }

    #[test]
    fn velocity_midpoint_interpolates() {
        let m = VelocityMap::default();
        let v = velocity_map(&spike_at(-23.0, 0.1), SR, &[0.1], &m);
        assert!((72..=75).contains(&v[0]), "{v:?}");
    }
}
