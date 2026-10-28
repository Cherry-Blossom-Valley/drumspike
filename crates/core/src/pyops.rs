//! Python numeric semantics used by the ports of `engine.py`.
//!
//! The Python engine mixes `int()` truncation, `round()` with ties to even and slice
//! clamping. Every port goes through these helpers so the Rust code makes the same
//! choices at the same places.

/// Python `int(f)` for a finite float: truncation toward zero.
#[inline]
pub fn py_int(f: f64) -> i64 {
    f as i64
}

/// Python `round(f)` on a float: ties go to the even neighbour.
#[inline]
pub fn py_round(f: f64) -> i64 {
    f.round_ties_even() as i64
}

/// Python `round(f, ndigits)` (and `float(f"{f:.{ndigits}f}")`): the exact binary value
/// rounded to `ndigits` decimals with ties to even, read back as the nearest float. Rust's
/// fixed-precision formatting rounds the exact value the same way, so `round(0.25, 1)` is
/// 0.2, `round(0.35, 1)` is 0.3 (0.35 is stored below the tie) and `round(-0.04, 1)` is -0.0.
/// A non-finite value comes back unchanged.
pub fn py_round_digits(f: f64, ndigits: usize) -> f64 {
    if !f.is_finite() {
        return f;
    }
    format!("{f:.ndigits$}").parse().unwrap_or(f)
}

/// The sample window `[max(0, int((t - pre) * sr)), min(len, int((t + post) * sr)))` the
/// engine slices around `t`, as `(start, end)` with `start <= end <= len`.
///
/// Python slice semantics are kept exactly: a start past the end gives an empty window and
/// a negative upper bound (`t < -post`, never produced by the detector) counts from the end
/// like `x[0:-n]`, so `engine.peak_in` and this helper agree for every `t`.
#[inline]
pub fn window(len: usize, t: f64, sr: u32, pre: f64, post: f64) -> (usize, usize) {
    let srf = sr as f64;
    let i0 = (py_int((t - pre) * srf).max(0) as usize).min(len);
    let mut i1 = py_int((t + post) * srf).min(len as i64);
    if i1 < 0 {
        i1 += len as i64;
    }
    let i1 = i1.max(0) as usize;
    (i0, i1.max(i0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn py_int_truncates_toward_zero() {
        assert_eq!(py_int(2.9), 2);
        assert_eq!(py_int(-2.9), -2);
        assert_eq!(py_int(0.999_999_9), 0);
    }

    #[test]
    fn py_round_ties_to_even() {
        assert_eq!(py_round(0.5), 0);
        assert_eq!(py_round(1.5), 2);
        assert_eq!(py_round(2.5), 2);
        assert_eq!(py_round(-0.5), 0);
        assert_eq!(py_round(73.5), 74);
    }

    #[test]
    fn py_round_digits_matches_python_round_on_ties() {
        // (x, round(x, 1)) from Python 3.12
        let table: [(f64, f64); 22] = [
            (0.05, 0.1),
            (0.15, 0.1),
            (0.25, 0.2),
            (0.35, 0.3),
            (0.45, 0.5),
            (0.55, 0.6),
            (0.65, 0.7),
            (0.75, 0.8),
            (0.85, 0.8),
            (0.95, 0.9),
            (1.25, 1.2),
            (2.25, 2.2),
            (-0.25, -0.2),
            (-0.75, -0.8),
            (12.25, 12.2),
            (0.04999999, 0.0),
            (99.95, 100.0),
            (3.45, 3.5),
            (62.75, 62.8),
            (47.35, 47.4),
            (1e-20, 0.0),
            (0.0, 0.0),
        ];
        for (x, want) in table {
            let got = py_round_digits(x, 1);
            assert_eq!(got.to_bits(), want.to_bits(), "round({x}, 1) = {want}, got {got}");
        }
        // the sign of zero survives, as in Python
        for x in [-0.04, -1e-20, -0.0] {
            let got = py_round_digits(x, 1);
            assert!(got == 0.0 && got.is_sign_negative(), "round({x}, 1) = -0.0, got {got}");
        }
        assert_eq!(py_round_digits(0.2225, 3), 0.223);
        assert_eq!(py_round_digits(0.2345, 3), 0.234);
        assert!(py_round_digits(f64::NAN, 1).is_nan());
        assert_eq!(py_round_digits(f64::INFINITY, 1), f64::INFINITY);
    }

    #[test]
    fn window_matches_engine_slicing() {
        // engine.peak_in: pre 5 ms, post 60 ms at 48 kHz
        assert_eq!(window(100_000, 1.0, 48_000, 0.005, 0.060), (47_760, 50_880));
        // start clamped at 0, end clamped at len
        assert_eq!(window(1_000, 0.0, 48_000, 0.005, 0.060), (0, 1_000));
        // start past the end: empty window at len
        assert_eq!(window(1_000, 5.0, 48_000, 0.005, 0.060), (1_000, 1_000));
        // t far before 0: a negative stop counts from the end like a Python slice
        assert_eq!(window(1_000, -1.0, 48_000, 0.005, 0.060), (0, 0));
        assert_eq!(window(96_000, -0.5, 48_000, 0.005, 0.060), (0, 74_880));
    }
}
