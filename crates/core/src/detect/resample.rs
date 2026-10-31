//! Fixed-ratio resampler to 44.1 kHz for the detector input (P2-2a).
//!
//! The detector chain (`engine.detector_input` today) hands the reference CLI a 48 kHz file, and
//! the CLI resamples it with `librosa.load(sr=44100)`, that is soxr 1.1.0 at `soxr_hq`. No
//! pure-Rust code reproduces soxr bit for bit, so this module is a separate design whose
//! distance to soxr is measured (`tools/parity/check_resample.py`, target SNR >= 80 dB
//! per file) rather than assumed.
//!
//! # Design
//!
//! A windowed-sinc polyphase FIR with a fixed rational ratio: 48000:44100 = 160:147 and
//! 96000:44100 = 320:147. Output sample `k` sits at input time `t_k = k * M / 147` input
//! samples (`M` = 160 or 320) and is the inner product of the input with a continuous kernel
//! `g(tau)` sampled at `tau = (t_k - m) / sr_in` for every input sample `m`:
//!
//! ```text
//! y[k] = sum_m x[m] * g((t_k - m) / sr_in),   x[m] = 0 outside 0..n
//! g = A (*) B                                  (convolution of two Kaiser-windowed sincs)
//! A(s)   = 2 fc_A sinc(2 fc_A s) kaiser(s / T_A, beta_A)
//! B(tau) = 2 fc_B sinc(2 fc_B tau) kaiser(tau / T_B, beta_B)
//! ```
//!
//! Stage B is the anti-alias lowpass: `fc_B` = 21,099.0 Hz (the middle of the transition
//! band from 0.913 of the output Nyquist, soxr_hq's `passband_end`, to 22,050 Hz),
//! `beta_B` = 13.711 (about 133 dB), half length `T_B` = 2.3177 ms. Stage A is a short
//! pre-filter, `fc_A` = 43,419 Hz, `beta_A` = 14.064, `T_A` = 72.93 us (3.5 input samples
//! at 48 kHz); it reproduces the slow passband droop of soxr_hq (-0.0022 dB at 18 kHz,
//! -0.0078 dB at 20 kHz) that a single flat Kaiser design lacks. A single Kaiser stage
//! matched soxr to 73 dB on white noise and failed the 80 dB gate on the hi-hat tracks (69
//! to 76 dB); the cascade matches soxr's measured kernel to 133 dB (white noise) at both
//! rates. The six numbers are a least-squares fit of this kernel family to soxr_hq 1.1.0's
//! measured impulse response (`tools/parity/fit_resample_kernel.py`); the response is
//! -3 dB at 20.96 kHz, -80 dB at 21.92 kHz, below -137 dB from 22.05 kHz on.
//!
//! The kernel is symmetric and centred on `t_k`, so the output has zero group delay: output
//! sample `k` is the signal at `k / 44100` s. Edges are zero padded (soxr does the same).
//!
//! Output length: `ceil(n * 147 / M)`, the rule of `librosa.resample(fix=True)` that
//! `librosa.load(sr=44100)` applies.
//!
//! # Table construction
//!
//! Tap positions are `tau = n * delta` with `delta = 1 / (147 sr_in)`. `B` is evaluated on
//! that grid, `A` on the coarser grid `q * delta` = 1 / (147 * 12,000) s (`q` = 4 at 48 kHz,
//! 8 at 96 kHz, so the quadrature is the same at both rates), and each tap is the trapezoid
//! sum `sum_i A(i q delta) B(tau - i q delta) q delta` over A's support (257 points). The
//! integrand is band limited far below the 1.76 MHz quadrature rate, so the sum is the
//! integral to double precision. Every phase is then normalised to sum exactly 1.0 (DC
//! passes). Building a table takes about 20 ms; [`Resampler::shared`] caches one per rate.
//!
//! # Determinism
//!
//! Each output sample is accumulated in `f64` into four lanes in a fixed tap order and the
//! lanes are folded as `(l0 + l1) + (l2 + l3)`, then rounded to `f32`. The compiler may
//! vectorise the lanes but cannot change the association, so the result is the same on
//! every host and thread count; the parallel split over output blocks does not enter the
//! arithmetic. The table itself uses the platform's `f64::sin` and a Bessel series; a
//! last-bit difference in a tap between two libm implementations can flip the rounding of
//! an output sample in rare cases, far below every parity tolerance of phase 2.

use std::fmt;
use std::sync::OnceLock;

use rayon::prelude::*;

/// The detector's sample rate (the reference pipeline, madmom `SignalProcessor(sample_rate=44100)`).
pub const TARGET_RATE: u32 = 44_100;

/// Phases of the polyphase table: the output rate divided by `gcd(sr_in, 44100)`.
const PHASES: usize = 147;

/// Stage A (pre-filter): cutoff in Hz, Kaiser beta, half length in seconds.
const FC_A: f64 = 43_419.1;
const BETA_A: f64 = 14.0642;
const T_A: f64 = 72.927e-6;

/// Stage B (anti-alias lowpass): cutoff in Hz, Kaiser beta, half length in seconds.
const FC_B: f64 = 21_099.02;
const BETA_B: f64 = 13.7113;
const T_B: f64 = 2.31772e-3;

/// Quadrature step for stage A: `1 / (147 * 12,000)` s, a whole number of tap grid steps at
/// both rates.
const QUAD_RATE: f64 = 147.0 * 12_000.0;

/// Errors of the resampler: only the rate can be wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResampleError {
    /// The source rate is not 44,100, 48,000 or 96,000 Hz.
    UnsupportedRate(u32),
}

impl fmt::Display for ResampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResampleError::UnsupportedRate(sr) => {
                write!(f, "resample: unsupported source rate {sr} Hz (44100, 48000 or 96000)")
            }
        }
    }
}

impl std::error::Error for ResampleError {}

/// Modified Bessel function of the first kind, order 0, by its power series. For
/// `x <= 16` the series converges in about 40 terms to full `f64` precision.
fn bessel_i0(x: f64) -> f64 {
    let q = x * x / 4.0;
    let mut term = 1.0;
    let mut sum = 1.0;
    let mut k = 1.0;
    while term > sum * 1e-18 {
        term *= q / (k * k);
        sum += term;
        k += 1.0;
    }
    sum
}

/// Kaiser window at `r = tau / T_h` in `(-1, 1)`; zero outside.
fn kaiser(r: f64, beta: f64) -> f64 {
    if r.abs() >= 1.0 {
        return 0.0;
    }
    bessel_i0(beta * (1.0 - r * r).sqrt()) / bessel_i0(beta)
}

/// `sin(pi x) / (pi x)` with the limit at 0.
fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

/// One Kaiser-windowed sinc stage, `2 fc sinc(2 fc tau) kaiser(tau / t_h)`.
fn stage(tau: f64, fc: f64, beta: f64, t_h: f64) -> f64 {
    2.0 * fc * sinc(2.0 * fc * tau) * kaiser(tau / t_h, beta)
}

/// Half length of the composite kernel in seconds.
const HALF_LENGTH_S: f64 = T_A + T_B;

/// A fixed-ratio resampler from 48 or 96 kHz to 44.1 kHz; 44.1 kHz passes through.
#[derive(Debug, Clone)]
pub struct Resampler {
    sr_in: u32,
    /// Input samples per 147 output samples: 160 (48 kHz), 320 (96 kHz) or 147 (pass-through).
    m: usize,
    /// Taps per phase (even); tap `j` of phase `p` is `g((j - half + p / 147) / sr_in)`.
    taps: usize,
    half: usize,
    /// `PHASES * taps` coefficients, phase-major.
    table: Vec<f64>,
}

impl Resampler {
    /// Build the table for `sr_in`; `Err` for any rate other than 44,100, 48,000 or 96,000.
    ///
    /// # Errors
    ///
    /// [`ResampleError::UnsupportedRate`] when `sr_in` is not 44,100, 48,000 or 96,000 Hz.
    pub fn new(sr_in: u32) -> Result<Resampler, ResampleError> {
        let (m, q) = match sr_in {
            44_100 => (147, 0usize),
            48_000 => (160, 4),
            96_000 => (320, 8),
            other => return Err(ResampleError::UnsupportedRate(other)),
        };
        if m == PHASES {
            return Ok(Resampler {
                sr_in,
                m,
                taps: 0,
                half: 0,
                table: Vec::new(),
            });
        }
        let sr = f64::from(sr_in);
        let delta = 1.0 / (PHASES as f64 * sr);
        debug_assert!(((q as f64 * delta) * QUAD_RATE - 1.0).abs() < 1e-12);
        // Stage A on the quadrature grid, with the trapezoid weight (A is zero at +-T_A, so
        // the end weights need no halving).
        let n_a = (T_A * QUAD_RATE).floor() as i64;
        let a: Vec<f64> = (-n_a..=n_a)
            .map(|i| stage(i as f64 / QUAD_RATE, FC_A, BETA_A, T_A) / QUAD_RATE)
            .collect();
        // Stage B on the tap grid.
        let n_b = (T_B / delta).ceil() as i64;
        let b: Vec<f64> = (-n_b..=n_b)
            .map(|i| stage(i as f64 * delta, FC_B, BETA_B, T_B))
            .collect();
        let b_at = |n: i64| -> f64 { if n.abs() > n_b { 0.0 } else { b[(n + n_b) as usize] } };
        // The composite kernel is zero at |tau| >= T_A + T_B.
        let half = (HALF_LENGTH_S * sr).ceil() as usize + 1;
        let taps = 2 * half;
        let mut table = vec![0.0f64; PHASES * taps];
        for p in 0..PHASES {
            let row = &mut table[p * taps..(p + 1) * taps];
            for (j, c) in row.iter_mut().enumerate() {
                // tau = (j - half + p / 147) / sr = n * delta
                let n = (j as i64 - half as i64) * PHASES as i64 + p as i64;
                let mut g = 0.0f64;
                for (i, &ai) in a.iter().enumerate() {
                    g += ai * b_at(n - (i as i64 - n_a) * q as i64);
                }
                *c = g;
            }
            // Normalise the phase to unit DC gain (fixed order sum).
            let s: f64 = row.iter().fold(0.0, |acc, &c| acc + c);
            for c in row.iter_mut() {
                *c /= s;
            }
        }
        Ok(Resampler {
            sr_in,
            m,
            taps,
            half,
            table,
        })
    }

    /// The process-wide table for `sr_in`, built on first use.
    ///
    /// # Errors
    ///
    /// [`ResampleError::UnsupportedRate`] when `sr_in` is not 44,100, 48,000 or 96,000 Hz.
    ///
    /// # Panics
    ///
    /// Never in practice: the `expect("supported rate")` on [`Resampler::new`] runs only for
    /// a rate that the `match` above has already accepted.
    pub fn shared(sr_in: u32) -> Result<&'static Resampler, ResampleError> {
        static R44: OnceLock<Resampler> = OnceLock::new();
        static R48: OnceLock<Resampler> = OnceLock::new();
        static R96: OnceLock<Resampler> = OnceLock::new();
        let cell = match sr_in {
            44_100 => &R44,
            48_000 => &R48,
            96_000 => &R96,
            other => return Err(ResampleError::UnsupportedRate(other)),
        };
        Ok(cell.get_or_init(|| Resampler::new(sr_in).expect("supported rate")))
    }

    /// The source rate this table was built for.
    pub fn source_rate(&self) -> u32 {
        self.sr_in
    }

    /// Taps per phase (0 for the 44.1 kHz pass-through).
    pub fn taps_per_phase(&self) -> usize {
        self.taps
    }

    /// Output length for `n` input samples: `ceil(n * 147 / M)`, saturating at
    /// `usize::MAX` for inputs where the multiplication would overflow (R14).
    pub fn output_len(&self, n: usize) -> usize {
        n.saturating_mul(PHASES).div_ceil(self.m)
    }

    /// One output sample: input index `n_k`, phase `p`, four-lane `f64` accumulation.
    #[inline]
    fn sample(&self, x: &[f32], n_k: usize, p: usize) -> f32 {
        let row = &self.table[p * self.taps..(p + 1) * self.taps];
        // Tap j multiplies x[n_k + half - j]; the j range that stays inside 0..x.len().
        let hi = n_k + self.half; // index for j = 0
        let j_start = hi.saturating_sub(x.len() - 1);
        let j_end = (hi + 1).min(self.taps);
        let mut acc = [0.0f64; 4];
        let mut j = j_start;
        while j + 4 <= j_end {
            acc[0] += row[j] * f64::from(x[hi - j]);
            acc[1] += row[j + 1] * f64::from(x[hi - j - 1]);
            acc[2] += row[j + 2] * f64::from(x[hi - j - 2]);
            acc[3] += row[j + 3] * f64::from(x[hi - j - 3]);
            j += 4;
        }
        let mut lane = 0;
        while j < j_end {
            acc[lane] += row[j] * f64::from(x[hi - j]);
            lane += 1;
            j += 1;
        }
        ((acc[0] + acc[1]) + (acc[2] + acc[3])) as f32
    }

    /// Resample `x` (mono, the source rate of this table) to 44.1 kHz.
    pub fn process(&self, x: &[f32]) -> Vec<f32> {
        if self.m == PHASES {
            return x.to_vec();
        }
        let n_out = self.output_len(x.len());
        let mut y = vec![0.0f32; n_out];
        if x.is_empty() {
            return y;
        }
        // Blocks of outputs are independent; each output is computed the same way on
        // every thread, so the parallel split does not change a single bit.
        const BLOCK: usize = 4096;
        y.par_chunks_mut(BLOCK).enumerate().for_each(|(b, chunk)| {
            let k0 = b * BLOCK;
            for (i, out) in chunk.iter_mut().enumerate() {
                let km = (k0 + i) * self.m;
                *out = self.sample(x, km / PHASES, km % PHASES);
            }
        });
        y
    }
}

/// Resample a mono signal at `sr_in` to 44.1 kHz with the shared table (see the module
/// docs). A 44.1 kHz signal is returned unchanged; any other rate than 48 or 96 kHz is an
/// error.
///
/// # Errors
///
/// [`ResampleError::UnsupportedRate`] when `sr_in` is not 44,100, 48,000 or 96,000 Hz.
pub fn resample_to_44100(x: &[f32], sr_in: u32) -> Result<Vec<f32>, ResampleError> {
    Ok(Resampler::shared(sr_in)?.process(x))
}

#[cfg(test)]
mod tests;
