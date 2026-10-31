//! Peak pyramid: `engine.compute_peaks` and the tile API for the waveform view.
//!
//! Definition (engine.py): for each bucket size `bs` in [`PEAK_LEVELS`], the first
//! `(n // bs) * bs` samples are cut into buckets of `bs` samples; per bucket min, max and
//! RMS are rounded to 5 decimals (numpy `np.round(a, 5)` on float32 input, so the
//! rounding happens in float32). The trailing partial block is dropped. When fewer than
//! `bs` samples exist the level is a single zero bucket (`n = 1`).

use serde::Serialize;

/// One level of the pyramid: `n` buckets of min, max and RMS.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PeakLevel {
    /// Minimum sample per bucket, rounded to 5 decimals.
    pub min: Vec<f64>,
    /// Maximum sample per bucket, rounded to 5 decimals.
    pub max: Vec<f64>,
    /// RMS per bucket, rounded to 5 decimals.
    pub rms: Vec<f64>,
    /// Bucket count.
    pub n: usize,
}

/// Bucket sizes of `engine.compute_peaks`'s default levels.
pub const PEAK_LEVELS: [usize; 3] = [256, 4096, 65536];

/// numpy `np.round(a, 5)` on a float32 array: `rint(a * 1e5) / 1e5` in float32.
/// Returned as f64 so JSON carries the same value Python's `tolist()` produces.
#[inline]
fn round5_f32(v: f32) -> f64 {
    f64::from((v * 1e5f32).round_ties_even() / 1e5f32)
}

/// Raw (unrounded) statistics of one bucket.
#[derive(Clone, Copy)]
struct Raw {
    mn: f32,
    mx: f32,
    ss: f64,
}

fn raw_level(x: &[f32], bs: usize) -> Vec<Raw> {
    let m = (x.len() / bs) * bs;
    x[..m]
        .chunks_exact(bs)
        .map(|b| {
            let mut mn = f32::INFINITY;
            let mut mx = f32::NEG_INFINITY;
            // 4 independent accumulators: lets the CPU overlap the f64 adds.
            let mut ss = [0.0f64; 4];
            let (quads, rest) = b.as_chunks::<4>();
            for q in quads {
                for (k, &v) in q.iter().enumerate() {
                    mn = mn.min(v);
                    mx = mx.max(v);
                    let d = f64::from(v);
                    ss[k] += d * d;
                }
            }
            for &v in rest {
                mn = mn.min(v);
                mx = mx.max(v);
                let d = f64::from(v);
                ss[0] += d * d;
            }
            Raw {
                mn,
                mx,
                ss: ss[0] + ss[1] + ss[2] + ss[3],
            }
        })
        .collect()
}

fn finish_level(raw: &[Raw], bs: usize) -> PeakLevel {
    if raw.is_empty() {
        return PeakLevel {
            min: vec![0.0],
            max: vec![0.0],
            rms: vec![0.0],
            n: 1,
        };
    }
    PeakLevel {
        min: raw.iter().map(|r| round5_f32(r.mn)).collect(),
        max: raw.iter().map(|r| round5_f32(r.mx)).collect(),
        rms: raw
            .iter()
            .map(|r| round5_f32((r.ss / bs as f64).sqrt() as f32))
            .collect(),
        n: raw.len(),
    }
}

/// One level of the pyramid: `(n // bs) * bs` samples in buckets of `bs`.
pub fn peaks_level(x: &[f32], bs: usize) -> PeakLevel {
    finish_level(&raw_level(x, bs), bs)
}

/// `engine.compute_peaks(x)` for the default levels. Keys are the bucket sizes.
///
/// One pass over the samples: the 4096 and 65536 levels are aggregates of the 256-level
/// buckets (min of mins, max of maxs, sum of sums of squares). Min/max are exact; the RMS
/// differs from a direct computation only by f64 summation order (far below 1e-5).
pub fn compute_peaks(x: &[f32]) -> Vec<(usize, PeakLevel)> {
    compute_peaks_levels(x, &PEAK_LEVELS)
}

/// `engine.compute_peaks(x, levels)` for arbitrary bucket sizes. A level that is a
/// multiple of the previous one is aggregated from it, every other level is computed
/// from the samples.
pub fn compute_peaks_levels(x: &[f32], levels: &[usize]) -> Vec<(usize, PeakLevel)> {
    let mut out = Vec::with_capacity(levels.len());
    let mut prev: Option<(usize, Vec<Raw>)> = None;
    for &bs in levels {
        let raw = match &prev {
            Some((pbs, praw)) if bs % pbs == 0 => {
                let k = bs / pbs;
                praw.chunks_exact(k)
                    .map(|g| {
                        g.iter().fold(
                            Raw {
                                mn: f32::INFINITY,
                                mx: f32::NEG_INFINITY,
                                ss: 0.0,
                            },
                            |a, r| Raw {
                                mn: a.mn.min(r.mn),
                                mx: a.mx.max(r.mx),
                                ss: a.ss + r.ss,
                            },
                        )
                    })
                    .collect()
            }
            _ => raw_level(x, bs),
        };
        out.push((bs, finish_level(&raw, bs)));
        prev = Some((bs, raw));
    }
    out
}

// ---------------------------------------------------------------------------
// Tile API: what the waveform view asks for
// ---------------------------------------------------------------------------

/// The clamp of the UI's `ensureSamples`: `s = min(max(0, from), len)`,
/// `e = max(s, min(len, to))`. A window past the end is empty, never an error.
#[inline]
pub fn clamp_window(len: usize, from: i64, to: i64) -> (usize, usize) {
    let s = (from.max(0) as usize).min(len);
    let e = (to.max(0) as usize).min(len).max(s);
    (s, e)
}

/// A slice of one pyramid level: buckets `[start, start + min.len())`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PeakTile<'a> {
    /// Bucket size of the level.
    pub level: usize,
    /// First bucket index of the tile.
    pub start: usize,
    /// Minimum per bucket.
    pub min: &'a [f64],
    /// Maximum per bucket.
    pub max: &'a [f64],
    /// RMS per bucket.
    pub rms: &'a [f64],
}

/// A window of samples `[start, start + data.len())`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SampleWindow<'a> {
    /// First sample index of the window.
    pub start: usize,
    /// The samples.
    pub data: &'a [f32],
}

/// A decoded mono track with its pyramid: the unit the tile API serves.
#[derive(Debug, Clone)]
pub struct Track {
    /// Native sample rate.
    pub sr: u32,
    samples: Vec<f32>,
    levels: Vec<(usize, PeakLevel)>,
}

impl Track {
    /// Build the default pyramid ([`PEAK_LEVELS`]) over `samples`.
    pub fn new(sr: u32, samples: Vec<f32>) -> Track {
        Track::with_levels(sr, samples, &PEAK_LEVELS)
    }

    /// Build a pyramid with the given bucket sizes.
    pub fn with_levels(sr: u32, samples: Vec<f32>, levels: &[usize]) -> Track {
        let levels = compute_peaks_levels(&samples, levels);
        Track { sr, samples, levels }
    }

    /// Sample count.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// True when the track has no samples.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// All samples.
    pub fn all_samples(&self) -> &[f32] {
        &self.samples
    }

    /// All levels, `(bucket size, level)` in the order they were built.
    pub fn levels(&self) -> &[(usize, PeakLevel)] {
        &self.levels
    }

    /// The level with bucket size `bs`.
    pub fn level(&self, bs: usize) -> Option<&PeakLevel> {
        self.levels.iter().find(|(b, _)| *b == bs).map(|(_, l)| l)
    }

    /// Buckets `[from, to)` of the level with bucket size `level`, clamped to the level
    /// like [`clamp_window`]; `None` when the pyramid has no such level.
    pub fn peaks(&self, level: usize, from: i64, to: i64) -> Option<PeakTile<'_>> {
        let l = self.level(level)?;
        let (s, e) = clamp_window(l.n, from, to);
        Some(PeakTile {
            level,
            start: s,
            min: &l.min[s..e],
            max: &l.max[s..e],
            rms: &l.rms[s..e],
        })
    }

    /// Samples `[from, to)` clamped like the UI's `ensureSamples` (see [`clamp_window`]).
    pub fn samples(&self, from: i64, to: i64) -> SampleWindow<'_> {
        let (s, e) = clamp_window(self.samples.len(), from, to);
        SampleWindow {
            start: s,
            data: &self.samples[s..e],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_window_matches_ensure_samples() {
        assert_eq!(clamp_window(1000, -5, 10), (0, 10));
        assert_eq!(clamp_window(1000, 990, 2000), (990, 1000));
        assert_eq!(clamp_window(1000, 2000, 3000), (1000, 1000), "past the end: empty");
        assert_eq!(clamp_window(1000, 50, 40), (50, 50), "reversed: empty at the start");
        assert_eq!(clamp_window(0, 0, 10), (0, 0));
    }

    #[test]
    fn track_tiles_and_windows() {
        let x: Vec<f32> = (0..1000).map(|i| (i as f32) / 1000.0).collect();
        let t = Track::new(48_000, x.clone());
        assert_eq!(t.len(), 1000);
        assert!(!t.is_empty());
        assert_eq!(t.all_samples(), &x[..]);
        assert_eq!(t.levels().len(), 3);
        assert_eq!(t.level(256).unwrap().n, 3);
        assert_eq!(t.level(4096).unwrap().n, 1, "short track: one zero bucket");
        assert_eq!(t.level(999), None);
        let tile = t.peaks(256, 1, 10).unwrap();
        assert_eq!((tile.level, tile.start, tile.min.len()), (256, 1, 2));
        assert_eq!(tile.max, &t.level(256).unwrap().max[1..3]);
        assert_eq!(tile.rms, &t.level(256).unwrap().rms[1..3]);
        assert_eq!(t.peaks(256, 7, 9).unwrap().min, &[] as &[f64]);
        assert_eq!(t.peaks(4096, 0, 1).unwrap().max, &[0.0]);
        assert!(t.peaks(128, 0, 1).is_none());
        let w = t.samples(-10, 5);
        assert_eq!((w.start, w.data), (0, &x[..5]));
        let w = t.samples(995, 5000);
        assert_eq!((w.start, w.data), (995, &x[995..]));
        let w = t.samples(5000, 6000);
        assert_eq!((w.start, w.data.len()), (1000, 0));
        let odd = Track::with_levels(96_000, x, &[100]);
        assert_eq!(odd.sr, 96_000);
        assert_eq!(odd.level(100).unwrap().n, 10);
        assert!(Track::new(48_000, Vec::new()).is_empty());
    }

    #[test]
    fn peaks_short_input_fallback() {
        let p = compute_peaks(&[0.5, -0.5]);
        for (_, lvl) in &p {
            assert_eq!(
                *lvl,
                PeakLevel {
                    min: vec![0.0],
                    max: vec![0.0],
                    rms: vec![0.0],
                    n: 1
                }
            );
        }
    }

    #[test]
    fn peaks_level_drops_the_partial_block_and_rounds_to_5_decimals() {
        let mut x = vec![0.0f32; 256 * 2 + 100];
        x[3] = 0.123_456;
        x[300] = -0.5;
        x[600] = 1.0; // in the dropped partial block
        let l = peaks_level(&x, 256);
        assert_eq!(l.n, 2);
        // the value is the float32 0.12346 widened to f64, exactly what numpy's tolist() gives
        assert_eq!(l.max, vec![f64::from(0.12346f32), 0.0]);
        assert_eq!(l.min, vec![0.0, -0.5]);
        let rms0 = ((0.123_456_f64 * 0.123_456) / 256.0).sqrt() as f32;
        assert_eq!(l.rms[0], f64::from((rms0 * 1e5).round_ties_even() / 1e5));
        assert_eq!(
            l.rms[1],
            f64::from(((0.5f32 * 0.5 / 256.0).sqrt() * 1e5f32).round_ties_even()) / 1e5
        );
    }

    #[test]
    fn aggregated_levels_equal_direct_levels() {
        let x: Vec<f32> = (0..70_000).map(|i| ((i as f32) * 0.01).sin() * 0.9).collect();
        let p = compute_peaks(&x);
        assert_eq!(p[0].0, 256);
        assert_eq!(p[2].0, 65536);
        for (bs, lvl) in &p {
            let direct = peaks_level(&x, *bs);
            assert_eq!(lvl.n, direct.n);
            assert_eq!(lvl.min, direct.min);
            assert_eq!(lvl.max, direct.max);
            for (a, b) in lvl.rms.iter().zip(&direct.rms) {
                assert!((a - b).abs() <= 1e-5, "rms {a} vs {b}");
            }
        }
        let odd = compute_peaks_levels(&x, &[100, 300, 700]);
        assert_eq!(odd[1].1, peaks_level(&x, 300));
        assert_eq!(odd[2].1, peaks_level(&x, 700));
    }
}
