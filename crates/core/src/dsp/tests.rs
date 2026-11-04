//! `to_bits` pins of the shared primitives: a merged helper gives the bits of every former copy, so a
//! merge cannot move a detection time, a mark or a velocity.

use crate::detect::attribution::{self, Biquad, Role, band_peak_db, band_sections};
use crate::detect::{cross_bleed, tom_bleed};

/// A deterministic noise source in [-1, 1) (xorshift).
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

/// Noise at -40 dB plus a decaying 110 Hz burst at sample 400: `n` samples.
fn signal(n: usize, seed: u64) -> Vec<f32> {
    let mut nz = Noise(seed);
    (0..n)
        .map(|i| {
            let burst = if i >= 400 {
                let t = (i - 400) as f64 / 48_000.0;
                0.6 * (2.0 * std::f64::consts::PI * 110.0 * t).sin() * (-t / 0.02).exp()
            } else {
                0.0
            };
            (0.01 * nz.next() + burst) as f32
        })
        .collect()
}

/// The signals of the grid: empty, shorter than the windows, and long enough for every window.
fn signals() -> Vec<Vec<f32>> {
    vec![Vec::new(), signal(50, 3), signal(3000, 7)]
}

/// The window centres: before the signal, at its start, inside, at its end and past it.
const CENTRES: [i64; 7] = [-500, 0, 10, 420, 1500, 2990, 3500];
/// The windows: the rules' typical one, empty, inverted and symmetric.
const WINDOWS: [(i64, i64); 4] = [(-24, 384), (0, 0), (10, -5), (-100, 100)];
/// The pre-rolls: none, 5 ms and 50 ms at 48 kHz.
const PREROLLS: [i64; 3] = [0, 240, 2400];

/// The band levels in the grid: broadband and every role band.
fn bands() -> Vec<&'static [attribution::BandStage]> {
    vec![
        &[],
        Role::Kick.band(),
        Role::Tom.band(),
        Role::Snare.band(),
        Role::Hat.band(),
    ]
}

/// The padded band level as of the refactor base (`cross_bleed::level_at` and `tom_bleed::level_db` were this
/// text twice), the reference.
fn level_ref(x: &[f32], sec: &[Biquad], c: i64, w: (i64, i64), pre: i64) -> f64 {
    let start = c + w.0 - pre;
    let len = (pre + w.1 - w.0).max(0) as usize;
    if start >= 0 && start as usize + len <= x.len() {
        let s = start as usize;
        return band_peak_db(x, sec, s, s + pre as usize, s + len);
    }
    let seg: Vec<f32> = (0..len)
        .map(|i| {
            let k = start + i as i64;
            if k >= 0 && (k as usize) < x.len() {
                x[k as usize]
            } else {
                0.0
            }
        })
        .collect();
    band_peak_db(&seg, sec, 0, pre as usize, len)
}

/// The clipped broadband peak as of the refactor base (`tom_bleed::broad_db` and
/// `attribution::broadband_peak_db`), the reference.
fn broadband_ref(x: &[f32], c: i64, w: (i64, i64)) -> f64 {
    let n = x.len() as i64;
    let a = (c + w.0).clamp(0, n);
    let b = (c + w.1).clamp(0, n);
    band_peak_db(x, &[], a as usize, a as usize, b as usize)
}

/// The `Option` median as of the refactor base (`cross_bleed::median` and `tom_bleed::median`), the reference.
fn median_ref(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// The value lists of the median and percentile grids: empty, one value, odd and even counts, signed zeros,
/// duplicates and 40 noise lists of 1 to 40 values.
fn lists() -> Vec<Vec<f64>> {
    let mut out = vec![
        vec![],
        vec![2.5],
        vec![3.0, 1.0, 2.0],
        vec![3.0, 1.0, 2.0, 10.0],
        vec![-0.0, 0.0],
        vec![0.1, 0.2, 0.3, 0.4],
        vec![-12.5, -12.5, -3.0, -40.25, -3.0],
    ];
    let mut nz = Noise(11);
    for n in 1..=40 {
        out.push((0..n).map(|_| 60.0 * nz.next()).collect());
    }
    out
}

#[test]
fn the_padded_band_level_keeps_the_bits_of_both_former_copies() {
    let mut checked = 0;
    for sr in [44_100, 48_000] {
        for band in bands() {
            let sec = band_sections(band, sr);
            for x in signals() {
                for c in CENTRES {
                    for w in WINDOWS {
                        for pre in PREROLLS {
                            let want = level_ref(&x, &sec, c, w, pre).to_bits();
                            let at = (x.len(), sr, band.len(), c, w, pre);
                            assert_eq!(cross_bleed::level_at(&x, &sec, c, w, pre).to_bits(), want, "{at:?}");
                            assert_eq!(tom_bleed::level_db(&x, &sec, c, w, pre).to_bits(), want, "{at:?}");
                            checked += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(checked, 2 * 5 * 3 * 7 * 4 * 3);
}

#[test]
fn the_clipped_broadband_peak_keeps_the_bits_of_both_former_copies() {
    let mut checked = 0;
    for x in signals() {
        for c in CENTRES {
            for w in WINDOWS {
                let want = broadband_ref(&x, c, w).to_bits();
                let at = (x.len(), c, w);
                assert_eq!(tom_bleed::broad_db(&x, c, w).to_bits(), want, "{at:?}");
                assert_eq!(attribution::broadband_peak_db(&x, c, w).to_bits(), want, "{at:?}");
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 3 * 7 * 4);
}

#[test]
fn the_option_median_keeps_the_bits_of_both_former_copies() {
    let all = lists();
    for v in &all {
        let want = median_ref(&mut v.clone()).map(f64::to_bits);
        assert_eq!(cross_bleed::median(&mut v.clone()).map(f64::to_bits), want, "{v:?}");
        assert_eq!(tom_bleed::median(&mut v.clone()).map(f64::to_bits), want, "{v:?}");
    }
    assert_eq!(all.len(), 47);
}

#[test]
fn percentile_and_np_percentile_are_two_rules_and_stay_apart() {
    // `percentile` interpolates from the lower value, `np_percentile` from the upper one at a fraction of 0.5 or
    // more (numpy's `_lerp`): the last bit differs, so the two are never merged.
    let mut differ = 0;
    let mut checked = 0;
    for v in lists() {
        for q in [10.0, 25.0, 50.0, 75.0, 90.0, 99.5] {
            let a = attribution::percentile(&v, q).map(f64::to_bits);
            let b = crate::detect::flam::np_percentile(&v, q).map(f64::to_bits);
            assert_eq!(a.is_some(), b.is_some(), "{v:?} {q}");
            differ += usize::from(a != b);
            checked += 1;
        }
    }
    assert_eq!(checked, 47 * 6);
    assert!(differ > 0, "no case tells the two percentile rules apart");
}
