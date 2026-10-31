//! Log-filtered spectrogram, the model input (P2-2b), for both reference chains.
//!
//! Input: a mono `f32` signal at 44.1 kHz ([`super::resample`] gives it). Output: the
//! `(frames, 84)` `f32` matrix in row-major order that the reference detector read.
//! [`Chain`] selects the recipe ([`Chain::Mix`] uses the CLI recipe). The two recipes share the STFT and the filter shape
//! and differ in the frame count and the filterbank grid:
//!
//! | Step | [`Chain::Cli`] (the CLI reference audio script, librosa) | [`Chain::Madmom`] (madmom 27f032e) |
//! |---|---|---|
//! | frame `i` | centred on `i * 441`, 1024 zeros on each side | the same (`int(i * 441.0)`, zero padding) |
//! | frames | `1 + len / 441` (librosa `center=True`) | `ceil(len / 441)` (`end='normal'`) |
//! | window | `np.hanning(2048)` cast to `f32`; frame times window in `f32` | `np.hanning(2048)` in `f64`; frame times window in `f64` |
//! | FFT | `f64` (numpy pocketfft), stored as `complex64` | `f64` (scipy.fftpack), stored as `complex64` |
//! | magnitude | `\|X\|` of the `complex64` value, bins 0 to 1023 | the same |
//! | grid | `20 * 2^(k / 12)` Hz up to 20 kHz (repeated multiplication), nearest bin by `argmin` | `440 * 2^(k / 12)` Hz in [20, 20000], nearest bin by `searchsorted` (`frequencies2bins`) |
//! | filters | 86 unique bins, 84 triangles (start, centre, stop) on consecutive bins, each normalised to sum 1 | the same shape (`TriangularFilter`, `norm_filters=True`) |
//! | compression | `log10(1 + x)` in `f32` | the same |
//!
//! The FFT bin frequencies are `np.fft.fftfreq(2048, 1 / 44100)[:1024]` in both chains.
//! 64 of the 84 centre bins differ between the grids ([`centre_bins`] pins both lists).
//!
//! # Arithmetic
//!
//! The FFT runs in `f64`, like numpy and scipy, and the complex result is rounded to
//! `complex64` before the magnitude, as both references store the STFT as `complex64`. Two
//! real frames share one complex FFT (`Z = A + iB`, `A[k] = (Z[k] + conj Z[N - k]) / 2`).
//! The filterbank product is accumulated in `f64` and rounded to `f32` (the references use
//! an `f32` matrix product; the difference is below 1 ulp of `f32` on the parity corpus).
//! Frames run in parallel (rayon); the parallel split does not enter the arithmetic, so the
//! result does not depend on the thread count.

use std::sync::{Arc, OnceLock};

use rayon::prelude::*;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use super::Chain;
use super::resample::TARGET_RATE;

/// Hop size in samples at 44.1 kHz (100 frames per second).
pub const HOP: usize = 441;
/// Frames per second of the spectrogram.
pub const FPS: f64 = 100.0;
/// Frame (and FFT) size in samples.
pub const FRAME_SIZE: usize = 2048;
/// Magnitude bins kept per frame (`|X|[:1024]`, no Nyquist bin).
pub const N_FFT_BINS: usize = FRAME_SIZE / 2;
/// Bands of the log filterbank (the model's input width).
pub const N_BANDS: usize = 84;
/// Unique filter edge and centre bins: [`N_BANDS`] + 2.
pub const N_EDGE_BINS: usize = N_BANDS + 2;

/// Number of frames for `n` samples at 44.1 kHz: `1 + n / 441` for [`Chain::Cli`] (librosa
/// `center=True`), `ceil(n / 441)` for [`Chain::Madmom`]. They differ by one frame when `n`
/// is a multiple of 441 (and for `n` = 0: 1 against 0).
#[must_use]
pub fn num_frames(n: usize, chain: Chain) -> usize {
    match chain {
        Chain::Cli | Chain::Mix => 1 + n / HOP,
        Chain::Madmom => n.div_ceil(HOP),
    }
}

/// `np.fft.fftfreq(2048, 1 / 44100)[k]`: `k * (1 / (2048 * (1 / 44100)))`.
#[must_use]
pub fn bin_frequency(k: usize) -> f64 {
    let val = 1.0 / (FRAME_SIZE as f64 * (1.0 / f64::from(TARGET_RATE)));
    k as f64 * val
}

fn bin_frequencies() -> Vec<f64> {
    (0..N_FFT_BINS).map(bin_frequency).collect()
}

/// The CLI grid (`AudioProcessor._log_frequencies` and `_frequencies_to_bins`).
fn cli_bins() -> Vec<usize> {
    let bf = bin_frequencies();
    let factor = 2f64.powf(1.0 / 12.0);
    let mut bins: Vec<usize> = Vec::with_capacity(128);
    let mut f = 20.0f64;
    while f <= 20_000.0 * (1.0 + 1e-12) {
        // np.argmin(np.abs(fft_freqs - f)): the first index on a tie.
        let mut best = 0;
        for k in 1..N_FFT_BINS {
            if (bf[k] - f).abs() < (bf[best] - f).abs() {
                best = k;
            }
        }
        if bins.last().is_none_or(|&l| best > l) {
            bins.push(best);
        }
        f *= factor;
    }
    bins
}

/// The madmom grid (`log_frequencies(12, 20, 20000, fref=440)` and
/// `frequencies2bins(..., unique_bins=True)`).
fn madmom_bins() -> Vec<usize> {
    let bf = bin_frequencies();
    let left = ((20.0f64 / 440.0).log2() * 12.0).floor();
    let right = ((20_000.0f64 / 440.0).log2() * 12.0).ceil();
    let mut bins: Vec<usize> = Vec::with_capacity(128);
    let mut k = left;
    while k < right {
        let f = 440.0 * 2f64.powf(k / 12.0);
        k += 1.0;
        if !(20.0..=20_000.0).contains(&f) {
            continue;
        }
        // searchsorted (left side), clipped to [1, len - 1], then the nearer neighbour
        // (the right one on a tie).
        let mut idx = bf.partition_point(|&b| b < f).clamp(1, N_FFT_BINS - 1);
        if f - bf[idx - 1] < bf[idx] - f {
            idx -= 1;
        }
        // np.unique: the indices are already non-decreasing.
        if bins.last().is_none_or(|&l| idx > l) {
            bins.push(idx);
        }
    }
    bins
}

/// The [`N_EDGE_BINS`] unique filter edge and centre bins of the chain's grid, ascending.
/// Filter `b` spans bins `[bins[b], bins[b + 2])` with its peak at `bins[b + 1]`.
pub fn filterbank_bins(chain: Chain) -> &'static [usize] {
    static CLI: OnceLock<Vec<usize>> = OnceLock::new();
    static MADMOM: OnceLock<Vec<usize>> = OnceLock::new();
    match chain {
        Chain::Cli | Chain::Mix => CLI.get_or_init(cli_bins),
        Chain::Madmom => MADMOM.get_or_init(madmom_bins),
    }
}

/// The [`N_BANDS`] filter centre bins of the chain's grid (`filterbank_bins(chain)[1..85]`).
#[must_use]
pub fn centre_bins(chain: Chain) -> &'static [usize] {
    let b = filterbank_bins(chain);
    &b[1..b.len() - 1]
}

/// One triangular filter: the first bin and the normalised weights.
struct Filter {
    start: usize,
    weights: Vec<f32>,
}

fn build_filters(chain: Chain) -> Vec<Filter> {
    let bins = filterbank_bins(chain);
    bins.windows(3)
        .map(|w| {
            let (start, centre, stop) = (w[0], w[1], w[2]);
            // Unique ascending bins give stop - start >= 2, so neither reference's
            // single-bin special case applies.
            debug_assert!(start < centre && centre < stop);
            let c = centre - start;
            let s = stop - start;
            let weights = match chain {
                Chain::Cli | Chain::Mix => {
                    // _create_madmom_filterbank: Python floats stored in an f32 matrix, then
                    // divided by the f32 row sum.
                    let mut w: Vec<f32> = (0..s)
                        .map(|j| {
                            if j < c {
                                (j as f64 / c as f64) as f32
                            } else if j == c {
                                1.0
                            } else {
                                ((s - j) as f64 / (s - c) as f64) as f32
                            }
                        })
                        .collect();
                    let sum: f32 = w.iter().sum();
                    w.iter_mut().for_each(|v| *v /= sum);
                    w
                }
                Chain::Madmom => {
                    // TriangularFilter: np.linspace(0, 1, c, endpoint=False) and
                    // np.linspace(1, 0, s - c, endpoint=False) in f64, divided by the f64 sum,
                    // stored in the f32 filterbank.
                    let up = 1.0 / c as f64;
                    let down = -1.0 / (s - c) as f64;
                    let w: Vec<f64> = (0..s)
                        .map(|j| {
                            if j < c {
                                j as f64 * up
                            } else {
                                (j - c) as f64 * down + 1.0
                            }
                        })
                        .collect();
                    let sum: f64 = w.iter().sum();
                    w.iter().map(|v| (v / sum) as f32).collect()
                }
            };
            Filter { start, weights }
        })
        .collect()
}

fn filters(chain: Chain) -> &'static [Filter] {
    static CLI: OnceLock<Vec<Filter>> = OnceLock::new();
    static MADMOM: OnceLock<Vec<Filter>> = OnceLock::new();
    match chain {
        Chain::Cli | Chain::Mix => CLI.get_or_init(|| build_filters(Chain::Cli)),
        Chain::Madmom => MADMOM.get_or_init(|| build_filters(Chain::Madmom)),
    }
}

/// Centre frequency in Hz (`fftfreq` of the centre bin) of each of the [`N_BANDS`] bands.
pub fn band_centres_hz(chain: Chain) -> &'static [f64] {
    static CLI: OnceLock<Vec<f64>> = OnceLock::new();
    static MADMOM: OnceLock<Vec<f64>> = OnceLock::new();
    let make = || centre_bins(chain).iter().map(|&k| bin_frequency(k)).collect();
    match chain {
        Chain::Cli | Chain::Mix => CLI.get_or_init(make),
        Chain::Madmom => MADMOM.get_or_init(make),
    }
}

/// `np.hanning(2048)`: `0.5 + 0.5 * cos(pi * n / (M - 1))` for `n = 1 - M, 3 - M, ..., M - 1`.
fn hanning() -> &'static [f64] {
    static W: OnceLock<Vec<f64>> = OnceLock::new();
    W.get_or_init(|| {
        let m = FRAME_SIZE as f64;
        (0..FRAME_SIZE)
            .map(|i| {
                let n = 1.0 - m + 2.0 * i as f64;
                0.5 + 0.5 * (std::f64::consts::PI * n / (m - 1.0)).cos()
            })
            .collect()
    })
}

/// The window as `f32` (`np.hanning(2048).astype(np.float32)`, the CLI's window).
fn hanning_f32() -> &'static [f32] {
    static W: OnceLock<Vec<f32>> = OnceLock::new();
    W.get_or_init(|| hanning().iter().map(|&v| v as f32).collect())
}

fn fft_plan() -> Arc<dyn Fft<f64>> {
    static PLAN: OnceLock<Arc<dyn Fft<f64>>> = OnceLock::new();
    PLAN.get_or_init(|| FftPlanner::new().plan_fft_forward(FRAME_SIZE))
        .clone()
}

/// Windowed frame `i` of `x` in `f64`, by the chain's rule (see the module docs).
fn fill_frame(x: &[f32], i: usize, chain: Chain, out: &mut [f64]) {
    let start = (i * HOP) as i64 - (FRAME_SIZE / 2) as i64;
    let (w64, w32) = (hanning(), hanning_f32());
    for (j, o) in out.iter_mut().enumerate() {
        let k = start + j as i64;
        *o = if k >= 0 && (k as usize) < x.len() {
            let v = x[k as usize];
            match chain {
                Chain::Cli | Chain::Mix => f64::from(v * w32[j]),
                Chain::Madmom => f64::from(v) * w64[j],
            }
        } else {
            0.0
        };
    }
}

/// `|X|` of a `complex64` value: the `f64` FFT output rounded to `f32` parts first.
fn mag_c64(z: Complex<f64>) -> f32 {
    (z.re as f32).hypot(z.im as f32)
}

/// Apply the filterbank and `log10(1 + x)` to one magnitude frame.
fn bands(filters: &[Filter], mag: &[f32], row: &mut [f32]) {
    for (r, f) in row.iter_mut().zip(filters) {
        let s: f64 = f
            .weights
            .iter()
            .zip(&mag[f.start..])
            .map(|(&w, &m)| f64::from(w) * f64::from(m))
            .sum();
        *r = (1.0f32 + s as f32).log10();
    }
}

/// Log-filtered spectrogram of a 44.1 kHz mono signal by the chain's recipe (module docs):
/// row-major `(num_frames(len, chain), 84)` `f32`.
#[must_use]
pub fn features(signal_44k: &[f32], chain: Chain) -> Vec<f32> {
    let frames = num_frames(signal_44k.len(), chain);
    let mut out = vec![0f32; frames * N_BANDS];
    let fft = fft_plan();
    let filters = filters(chain);
    out.par_chunks_mut(2 * N_BANDS).enumerate().for_each_init(
        || {
            (
                vec![Complex::new(0f64, 0f64); FRAME_SIZE],
                vec![Complex::new(0f64, 0f64); fft.get_inplace_scratch_len()],
                vec![0f64; FRAME_SIZE],
                vec![0f64; FRAME_SIZE],
                vec![0f32; N_FFT_BINS],
                vec![0f32; N_FFT_BINS],
            )
        },
        |(buf, scratch, a, b, ma, mb), (pair, rows)| {
            let i0 = 2 * pair;
            fill_frame(signal_44k, i0, chain, a);
            let second = rows.len() > N_BANDS;
            if second {
                fill_frame(signal_44k, i0 + 1, chain, b);
            } else {
                b.iter_mut().for_each(|v| *v = 0.0);
            }
            for j in 0..FRAME_SIZE {
                buf[j] = Complex::new(a[j], b[j]);
            }
            fft.process_with_scratch(buf, scratch);
            // Z = A + iB: A[k] = (Z[k] + conj Z[N-k]) / 2, B[k] = (Z[k] - conj Z[N-k]) / 2i.
            for k in 0..N_FFT_BINS {
                let z = buf[k];
                let zc = buf[(FRAME_SIZE - k) % FRAME_SIZE].conj();
                let sa = (z + zc) * 0.5;
                let d = (z - zc) * 0.5;
                ma[k] = mag_c64(sa);
                mb[k] = mag_c64(Complex::new(d.im, -d.re));
            }
            bands(filters, ma, &mut rows[..N_BANDS]);
            if second {
                bands(filters, mb, &mut rows[N_BANDS..]);
            }
        },
    );
    out
}

#[cfg(test)]
mod tests;
