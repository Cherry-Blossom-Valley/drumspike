//! `to_bits` pins of the shared primitives: a merged helper gives the bits of every former copy, so a
//! merge cannot move a detection time, a mark or a velocity.

use super::{
    Biquad, band_level_db, band_peak_db, band_sections, broadband_peak_db, median, ms_to_samples, np_percentile,
    percentile,
};
use crate::detect::attribution::Role;

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
fn bands() -> Vec<&'static [super::BandStage]> {
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
fn the_padded_band_level_keeps_the_bits_of_the_two_merged_copies() {
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
                            assert_eq!(band_level_db(&x, &sec, c, w, pre).to_bits(), want, "{at:?}");
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
fn the_clipped_broadband_peak_keeps_the_bits_of_the_two_merged_copies() {
    let mut checked = 0;
    for x in signals() {
        for c in CENTRES {
            for w in WINDOWS {
                let want = broadband_ref(&x, c, w).to_bits();
                let at = (x.len(), c, w);
                assert_eq!(broadband_peak_db(&x, c, w).to_bits(), want, "{at:?}");
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 3 * 7 * 4);
}

#[test]
fn the_option_median_keeps_the_bits_of_the_two_merged_copies() {
    let all = lists();
    for v in &all {
        let want = median_ref(&mut v.clone()).map(f64::to_bits);
        assert_eq!(median(&mut v.clone()).map(f64::to_bits), want, "{v:?}");
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
            let a = percentile(&v, q).map(f64::to_bits);
            let b = np_percentile(&v, q).map(f64::to_bits);
            assert_eq!(a.is_some(), b.is_some(), "{v:?} {q}");
            differ += usize::from(a != b);
            checked += 1;
        }
    }
    assert_eq!(checked, 47 * 6);
    assert!(differ > 0, "no case tells the two percentile rules apart");
}

#[test]
fn percentile_follows_numpy_linear() {
    let v = [3.0, 1.0, 2.0, 10.0];
    assert_eq!(percentile(&v, 0.0), Some(1.0));
    assert_eq!(percentile(&v, 100.0), Some(10.0));
    assert_eq!(percentile(&v, 50.0), Some(2.5));
    // numpy.percentile([1, 2, 3, 10], 90) = 7.9
    assert!((percentile(&v, 90.0).unwrap() - 7.9).abs() < 1e-12);
    assert_eq!(percentile(&[], 50.0), None);
}

#[test]
fn ms_to_samples_rounds_half_up() {
    assert_eq!(ms_to_samples(5.0, 44_100), 221);
    assert_eq!(ms_to_samples(0.5, 48_000), 24);
    assert_eq!(ms_to_samples(-0.5, 48_000), -24);
    assert_eq!(ms_to_samples(0.0, 48_000), 0);
}

#[test]
fn the_percentile_is_numpys_linear_one() {
    // values from numpy 2: np.percentile([3, 1, 4, 1, 5, 9, 2, 6], q)
    let v = [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0];
    assert_eq!(np_percentile(&v, 10.0), Some(1.0));
    assert_eq!(np_percentile(&v, 50.0), Some(3.5));
    assert!((np_percentile(&v, 99.5).unwrap() - 8.895).abs() < 1e-12);
    assert_eq!(np_percentile(&v, 100.0), Some(9.0));
    assert_eq!(np_percentile(&v, 0.0), Some(1.0));
    assert_eq!(np_percentile(&[], 10.0), None);
    assert_eq!(np_percentile(&[2.5], 99.5), Some(2.5));
    // the upper branch of numpy's lerp: b - (b - a) * (1 - g) for g >= 0.5
    let (a, b) = (0.1f64, 0.7f64);
    let g = 3.0 * 0.9 - 2.0;
    assert_eq!(
        np_percentile(&[a, b, 1.0, 2.0], 90.0),
        Some(2.0 - (2.0 - 1.0) * (1.0 - g))
    );
    assert_eq!(np_percentile(&[a, b], 25.0), Some(a + (b - a) * 0.25));
}
