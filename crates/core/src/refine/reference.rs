//! Refinement version 1, the reference rule ([`RefineRule::Reference`](super::RefineRule::Reference)): the port of `engine._refine_sr` with its
//! search window, the Hilbert envelope and the 25 % rule.

use std::cell::RefCell;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::dsp::amp_db_add;
use crate::pyops::py_int;

/// The steepest-rise search window around the coarse time `t0`, in ms (`lo_ms`, `hi_ms` of
/// `engine._refine_sr`). The detector reports hits about 6 ms before the attack, so the
/// default reaches further after `t0` than before it (P2-9,
/// `docs/archive/phase2-report.md`, section "P2-9 Refinement window").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefineWindow {
    /// Start of the window relative to `t0` (ms, negative = before `t0`).
    pub lo_ms: f64,
    /// End of the window relative to `t0` (ms, exclusive).
    pub hi_ms: f64,
}

impl RefineWindow {
    /// The app's window (`engine.REFINE_LO_MS`, `engine.REFINE_HI_MS`): `-2..+12` ms.
    pub const DEFAULT: RefineWindow = RefineWindow {
        lo_ms: -2.0,
        hi_ms: 12.0,
    };
    /// The phase 1 window `-7..+7` ms (`engine.REFINE_SYMMETRIC_MS`), for a caller whose
    /// `t0` is an attack and not a detector time (the P2-6 bleed source search).
    pub const SYMMETRIC_7: RefineWindow = RefineWindow {
        lo_ms: -7.0,
        hi_ms: 7.0,
    };
}

impl Default for RefineWindow {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Margin on each side of the search window for the envelope (seconds).
pub(crate) const MARGIN_S: f64 = 0.02;
/// Envelope smoothing length (seconds).
pub(crate) const SMOOTH_S: f64 = 0.0002;
/// The rise is measured over this span (seconds).
pub(crate) const RISE_S: f64 = 0.0005;
/// The walk-back stops where the envelope is within this many dB of the pre-attack level.
pub(crate) const RISE_DB: f64 = 3.0;
/// Pre-attack window length before `k - 2 * rise` (seconds).
pub(crate) const PRE_S: f64 = 0.001;
/// 25 % rule window before the anchor (seconds).
pub(crate) const RULE_PRE_S: f64 = 0.0015;
/// 25 % rule window after the anchor (seconds).
pub(crate) const RULE_POST_S: f64 = 0.010;

thread_local! {
    static PLANNER: RefCell<FftPlanner<f64>> = RefCell::new(FftPlanner::new());
}

fn plan(n: usize, inverse: bool) -> Arc<dyn Fft<f64>> {
    PLANNER.with(|p| {
        let mut p = p.borrow_mut();
        if inverse {
            p.plan_fft_inverse(n)
        } else {
            p.plan_fft_forward(n)
        }
    })
}

/// `engine._hilbert_env(seg)`: `|ifft(fft(seg) * h)|` with `h` = 1 at DC (and at Nyquist for
/// even `n`), 2 for the positive frequencies, 0 for the negative ones. The inverse transform
/// is scaled by `1/n` like `np.fft.ifft`; the magnitude is `hypot(re, im)` like `np.abs`.
/// An empty input gives an empty envelope.
#[must_use]
pub(crate) fn hilbert_env(seg: &[f64]) -> Vec<f64> {
    let n = seg.len();
    if n == 0 {
        return Vec::new();
    }
    let mut buf: Vec<Complex<f64>> = seg.iter().map(|&v| Complex::new(v, 0.0)).collect();
    plan(n, false).process(&mut buf);
    // X * h: numpy multiplies complex by the real h cast to complex, which is an exact
    // scaling of both parts for h in {0, 1, 2}.
    let half = n / 2;
    let even = n.is_multiple_of(2);
    for (i, v) in buf.iter_mut().enumerate() {
        let h = if i == 0 || (even && i == half) {
            1.0
        } else if (even && i < half) || (!even && i <= half) {
            2.0
        } else {
            0.0
        };
        *v = Complex::new(v.re * h, v.im * h);
    }
    plan(n, true).process(&mut buf);
    let fct = 1.0 / n as f64;
    buf.iter().map(|v| (v.re * fct).hypot(v.im * fct)).collect()
}

/// `np.convolve(a, v, mode="same")` for real vectors: the centre `max(len(a), len(v))`
/// values of the full convolution, offset `(min(len) - 1) / 2` into it (numpy swaps the
/// operands when `v` is longer). Products are summed in ascending index of the longer
/// operand.
#[must_use]
pub(crate) fn convolve_same(a: &[f64], v: &[f64]) -> Vec<f64> {
    let (long, short) = if a.len() >= v.len() { (a, v) } else { (v, a) };
    let (n, k) = (long.len(), short.len());
    if n == 0 || k == 0 {
        return Vec::new();
    }
    let off = (k - 1) / 2;
    (0..n)
        .map(|i| {
            let f = i + off; // index into the full convolution
            let j0 = f.saturating_sub(k - 1);
            let j1 = f.min(n - 1);
            let mut s = 0.0;
            for j in j0..=j1 {
                s += long[j] * short[f - j];
            }
            s
        })
        .collect()
}

/// The smoothed log envelope of `engine._refine_sr` for one cut: `20 * log10(env + 1e-9)`
/// after the `k_sm`-point moving average. Exposed for the parity harness and tests.
#[must_use]
pub(crate) fn log_envelope(seg: &[f64], k_sm: usize) -> Vec<f64> {
    let env = hilbert_env(seg);
    let kernel = vec![1.0 / k_sm as f64; k_sm];
    convolve_same(&env, &kernel).into_iter().map(amp_db_add).collect()
}

/// `engine._refine_sr(x, t0, sr, neighbors, lo_ms, hi_ms)`: the refined onset in seconds and
/// `rise_db` (`None` where the engine returns `None`: a cut shorter than 8 samples or an empty
/// search window). `neighbors` are the other coarse hits the caller knows (`analyze_track`
/// passes the previous and the next one); the nearest below and above `t0` clip the search
/// window `win` at the midpoints. This is refinement version 1 ([`RefineRule::Reference`](super::RefineRule::Reference)),
/// pinned to the frozen references; [`refine_reach`](super::reach::refine_reach) builds on it.
#[must_use]
pub fn refine_sr(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow) -> (f64, Option<f64>) {
    let d = refine_detail(x, t0, sr, neighbors, win);
    (d.t, d.rise)
}

/// One run of the reference rule with the state the reach rule reads (P4-12 stage 1).
pub(super) struct Detail {
    /// The reference result (seconds) and `rise_db`.
    pub(super) t: f64,
    pub(super) rise: Option<f64>,
    /// The midpoint towards the next coarse hit (absolute sample), when there is one.
    pub(super) next_mid: Option<i64>,
    /// The previous coarse hit (seconds), when there is one.
    pub(super) prev: Option<f64>,
    /// The 25 % rule, when the reference reached it.
    pub(super) rule: Option<RuleState>,
}

/// Where the 25 % rule of the reference looked and what it found (absolute samples).
pub(super) struct RuleState {
    /// The smoothed log envelope of the cut, and the cut's first sample.
    pub(super) e: Vec<f64>,
    pub(super) a: usize,
    /// The anchor, the steepest rise, and whether the search window had no rise (the anchor is
    /// then its last sample).
    pub(super) anchor: i64,
    pub(super) k: i64,
    pub(super) flat: bool,
    /// The rule window `[i0, i1)` and its maximum `|x|`.
    pub(super) i0: i64,
    pub(super) i1: i64,
    pub(super) mx: f32,
    /// The first sample at 25 % of `mx`; `None` when the window is silent (`mx` under 1e-4).
    pub(super) cross: Option<i64>,
}

impl RuleState {
    /// The reference gave `t0` back from the rule: a silent window, or no rise and a crossing at
    /// the window start.
    pub(super) fn gave_t0(&self) -> bool {
        self.cross.is_none() || (self.flat && self.cross == Some(self.i0))
    }
}

/// The body of [`refine_sr`] (every operation as before), keeping the state of the 25 % rule.
pub(super) fn refine_detail(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow) -> Detail {
    let mut prv: Option<f64> = None;
    let mut nxt: Option<f64> = None;
    for &nt in neighbors {
        if nt < t0 && prv.is_none_or(|p| nt > p) {
            prv = Some(nt);
        } else if nt > t0 && nxt.is_none_or(|n| nt < n) {
            nxt = Some(nt);
        }
    }
    let srf = f64::from(sr);
    let next_mid = nxt.map(|n| py_int(((t0 + n) / 2.0) * srf));
    let bail = |rise: Option<f64>| Detail {
        t: t0,
        rise,
        next_mid,
        prev: prv,
        rule: None,
    };
    let len = x.len() as i64;
    let (lo_s, hi_s) = (win.lo_ms / 1000.0, win.hi_ms / 1000.0);
    let a = py_int((t0 + lo_s - MARGIN_S) * srf).max(0);
    let b = py_int((t0 + hi_s + MARGIN_S) * srf).min(len);
    if b - a < 8 {
        return bail(None);
    }
    let (a, b) = (a as usize, b as usize);
    let cut: Vec<f64> = x[a..b].iter().map(|&v| f64::from(v)).collect();
    let k_sm = py_int(SMOOTH_S * srf).max(1) as usize;
    let e = log_envelope(&cut, k_sm);
    let l = py_int(RISE_S * srf).max(0) as usize;
    let elen = e.len() as i64;
    let c = py_int(t0 * srf) - a as i64;
    let mut lo = (l as i64).max(c + py_int(lo_s * srf));
    let mut hi = elen.min(c + py_int(hi_s * srf));
    if let Some(m) = next_mid {
        hi = hi.min(m - a as i64);
    }
    if let Some(p) = prv {
        lo = lo.max(py_int(((p + t0) / 2.0) * srf) - a as i64);
    }
    if hi <= lo {
        return bail(None);
    }
    let (lo, hi) = (lo as usize, hi as usize);
    // d[i] = e[i] - e[i - l] for i >= l (d is -1e9 below l, never inside [lo, hi) since lo >= l);
    // np.argmax returns the first maximum.
    let mut k = lo;
    let mut best = f64::NEG_INFINITY;
    for i in lo..hi {
        let d = if i >= l { e[i] - e[i - l] } else { -1e9 };
        if d > best {
            best = d;
            k = i;
        }
    }
    let k2 = k as i64 - 2 * l as i64;
    let p0 = (k2 - py_int(PRE_S * srf)).max(0) as usize;
    let p1 = k2.max(1) as usize;
    let pre = e[p0..p1].iter().copied().fold(f64::INFINITY, f64::min);
    let mut n = k;
    while n > 0 && e[n - 1] > pre + RISE_DB {
        n -= 1;
    }
    let rise_db = e[k] - pre;
    // No rise (steepest 0.5 ms rise <= 0 dB): the envelope only falls in the window (a
    // decaying tail). The steepest "rise" there is a near-tie that a 1-ulp decode difference
    // moves, so the anchor is the last sample of the search range instead, and the 25 % rule
    // looks for an attack after the window; with no crossing after it, t0 stays (P2-9).
    let flat = best <= 0.0;
    let anchor = if flat { (a + hi - 1) as i64 } else { (a + n) as i64 };
    let i0 = (anchor - py_int(RULE_PRE_S * srf)).max(0);
    let i1 = (anchor + py_int(RULE_POST_S * srf)).min(len);
    if i1 <= i0 {
        return bail(Some(rise_db));
    }
    let seg = &x[i0 as usize..i1 as usize];
    let mx = seg.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let state = |cross: Option<i64>, e: Vec<f64>| RuleState {
        e,
        a,
        anchor,
        k: (a + k) as i64,
        flat,
        i0,
        i1,
        mx,
        cross,
    };
    if mx < 1e-4f32 {
        return Detail {
            t: t0,
            rise: Some(rise_db),
            next_mid,
            prev: prv,
            rule: Some(state(None, e)),
        };
    }
    let thr = 0.25f32 * mx;
    let idx = seg.iter().position(|v| v.abs() >= thr).unwrap_or(0);
    let t = if idx == 0 {
        if flat { t0 } else { anchor as f64 / srf }
    } else {
        (i0 + idx as i64) as f64 / srf
    };
    Detail {
        t,
        rise: Some(rise_db),
        next_mid,
        prev: prv,
        rule: Some(state(Some(i0 + idx as i64), e)),
    }
}
