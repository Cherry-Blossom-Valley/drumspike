//! Filter and statistics primitives shared by the detection rules and the exports (refactor R7).
//!
//! Every function here keeps the exact float operations of the copy it replaces: the `to_bits`
//! tests in `dsp/tests.rs` compare each one with the former copies bit for bit.

/// The kind of one filter stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    /// Second-order lowpass (RBJ cookbook, Q = 1/sqrt(2)).
    LowPass,
    /// Second-order highpass (RBJ cookbook, Q = 1/sqrt(2)).
    HighPass,
}

/// One stage of a band: the kind and the corner frequency in Hz. Each stage runs twice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandStage {
    /// Lowpass or highpass.
    pub kind: FilterKind,
    /// Corner frequency in Hz.
    pub freq_hz: f64,
}

/// Normalised biquad coefficients (`a0` = 1), in the row order of a scipy `sos` section.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Biquad {
    /// Feed-forward coefficients.
    pub b: [f64; 3],
    /// Feedback coefficients `a1`, `a2`.
    pub a: [f64; 2],
}

impl Biquad {
    /// RBJ cookbook lowpass or highpass at `freq_hz`, Q = 1/sqrt(2), for rate `sr`.
    #[must_use]
    pub fn new(kind: FilterKind, freq_hz: f64, sr: u32) -> Biquad {
        let w = 2.0 * std::f64::consts::PI * freq_hz / f64::from(sr);
        let (cw, sw) = (w.cos(), w.sin());
        let alpha = sw / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let b = match kind {
            FilterKind::LowPass => [(1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0],
            FilterKind::HighPass => [(1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0],
        };
        let a0 = 1.0 + alpha;
        Biquad {
            b: [b[0] / a0, b[1] / a0, b[2] / a0],
            a: [-2.0 * cw / a0, (1.0 - alpha) / a0],
        }
    }
}

/// The sections of a band at rate `sr`: every stage twice, in order.
#[must_use]
pub(crate) fn band_sections(band: &[BandStage], sr: u32) -> Vec<Biquad> {
    band.iter()
        .flat_map(|s| {
            let q = Biquad::new(s.kind, s.freq_hz, sr);
            [q, q]
        })
        .collect()
}

/// Filter `x[from..to]` through `sections` (direct form II transposed, zero state at
/// `from`, the recursion of `scipy.signal.sosfilt`) and return
/// `20 log10(max |y| + 1e-9)` over the samples `y[w0..to]`. `from <= w0 <= to <= x.len()`;
/// an empty window gives -180 dB.
#[must_use]
pub(crate) fn band_peak_db(x: &[f32], sections: &[Biquad], from: usize, w0: usize, to: usize) -> f64 {
    if w0 >= to {
        return -180.0;
    }
    let mut z = vec![[0.0f64; 2]; sections.len()];
    let mut peak = 0.0f64;
    for (i, &v) in x[from..to].iter().enumerate() {
        let mut y = f64::from(v);
        for (s, zs) in sections.iter().zip(z.iter_mut()) {
            let out = s.b[0] * y + zs[0];
            zs[0] = s.b[1] * y - s.a[0] * out + zs[1];
            zs[1] = s.b[2] * y - s.a[1] * out;
            y = out;
        }
        if from + i >= w0 {
            peak = peak.max(y.abs());
        }
    }
    amp_db_add(peak)
}

/// Amplitude to dB with an additive floor: `20 log10(a + 1e-9)`; 0 gives -180 dB. The floor policy
/// of the band levels ([`band_peak_db`]), the refine log envelope and the flam lobe levels.
#[must_use]
pub(crate) fn amp_db_add(a: f64) -> f64 {
    20.0 * (a + 1e-9).log10()
}

/// Amplitude to dB with a clamp floor: `20 log10(max(a, floor))`. The floor policy of the hit peak
/// levels (`floor` 1e-9, at least -180 dB) and the role envelope (`floor` 1e-12, at least -240 dB).
/// Not the same value as [`amp_db_add`] near the floor: the two policies stay apart.
#[must_use]
pub(crate) fn amp_db_max(a: f64, floor: f64) -> f64 {
    20.0 * a.max(floor).log10()
}

/// The band level (dBFS) of `x` in `[c + w.0, c + w.1)` with the filter run in over `pre` samples
/// before the window: [`band_peak_db`] over a zero-padded copy where the run-in or the window leaves
/// the signal (the Python reference pads the same way). The tom bleed and the cross bleed rule
/// measure with it.
#[must_use]
pub(crate) fn band_level_db(x: &[f32], sec: &[Biquad], c: i64, w: (i64, i64), pre: i64) -> f64 {
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

/// The broadband peak (dBFS) of `x` in `[c + w.0, c + w.1)`, clipped to the signal (-180 dB when
/// empty): [`band_peak_db`] without sections and without run-in.
#[must_use]
pub(crate) fn broadband_peak_db(x: &[f32], c: i64, w: (i64, i64)) -> f64 {
    let n = x.len() as i64;
    let a = (c + w.0).clamp(0, n);
    let b = (c + w.1).clamp(0, n);
    band_peak_db(x, &[], a as usize, a as usize, b as usize)
}

/// The `q`-th percentile (0 to 100) of `v` with numpy's default linear interpolation;
/// `None` for an empty slice. NaN values are not expected.
pub(crate) fn percentile(v: &[f64], q: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let pos = q / 100.0 * (s.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(s.len() - 1);
    let frac = pos - lo as f64;
    Some(s[lo] + (s[hi] - s[lo]) * frac)
}

/// Milliseconds to samples: `floor(ms * sr / 1000 + 0.5)` (the same rule in the Python
/// reference; Python's `round` rounds half to even and would differ).
#[must_use]
pub(crate) fn ms_to_samples(ms: f64, sr: u32) -> i64 {
    (ms * f64::from(sr) / 1000.0 + 0.5).floor() as i64
}

/// `np.percentile(v, q)` with numpy's default linear method, bit for bit: the virtual index `(n - 1) * q / 100` and
/// numpy's `_lerp` (from the upper value when the fraction is 0.5 or more). `None` for an empty slice.
pub(crate) fn np_percentile(v: &[f64], q: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    let vi = (n - 1) as f64 * (q / 100.0);
    let lo = (vi.floor().max(0.0) as usize).min(n - 1);
    let hi = (lo + 1).min(n - 1);
    let g = vi - lo as f64;
    let (a, b) = (s[lo], s[hi]);
    let d = b - a;
    Some(if g >= 0.5 { b - d * (1.0 - g) } else { a + d * g })
}

/// The median of `v` (the mean of the two middle values for an even count), sorting `v` in place;
/// `None` for an empty slice.
pub fn median(v: &mut [f64]) -> Option<f64> {
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

#[cfg(test)]
mod tests;
