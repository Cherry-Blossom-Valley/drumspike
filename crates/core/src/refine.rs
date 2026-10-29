//! Onset refinement: the port of `engine._refine_sr` (the v3.16 hybrid method).
//!
//! The method, operation by operation as the Python engine runs it:
//!
//! 1. Cut `[t0 + lo - 20 ms, t0 + hi + 20 ms)`: the search window [`RefineWindow`]
//!    (default `-2..+12` ms, P2-9) plus 20 ms margin on each side.
//! 2. Hilbert envelope of the cut ([`hilbert_env`]: FFT analytic signal, no approximation),
//!    smoothed with a 0.2 ms moving average (`np.convolve(..., mode="same")`), in dB.
//! 3. Rise over 0.5 ms; the anchor `k` is the steepest rise in `[t0 + lo, t0 + hi)`, the
//!    search window clipped at the midpoints towards the neighbouring coarse hits.
//! 4. Walk back from `k` to where the envelope leaves the pre-attack level + 3 dB; that is
//!    the anchor sample. `rise_db` = envelope at `k` minus the pre-attack level.
//!    A window where the envelope never rises (steepest rise <= 0 dB, a decaying tail)
//!    takes the last sample of the search range as the anchor instead (P2-9).
//! 5. The 25 % rule in `[anchor - 1.5 ms, anchor + 10 ms)` on `|x|` in float32; a crossing
//!    at the window start (the previous hit still ringing) resolves to the anchor, or to
//!    `t0` when the window has no rise.
//!
//! Every index is computed with Python `int()` truncation ([`crate::pyops`]) and every
//! early return gives `t0` back unchanged, like the engine. The only place where the two
//! implementations can differ is the floating-point order inside the FFT (rustfft versus
//! numpy's pocketfft) and the smoothing sum; the parity harness
//! (`tools/parity/check_refine.py`) measures that on the corpus and the
//! `bench/refine_bench.py` scenarios.

use std::cell::RefCell;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

use crate::pyops::py_int;
use crate::velocity::{velocity_map, VelocityMap};

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
pub const MARGIN_S: f64 = 0.02;
/// Envelope smoothing length (seconds).
pub const SMOOTH_S: f64 = 0.0002;
/// The rise is measured over this span (seconds).
pub const RISE_S: f64 = 0.0005;
/// The walk-back stops where the envelope is within this many dB of the pre-attack level.
pub const RISE_DB: f64 = 3.0;
/// Pre-attack window length before `k - 2 * rise` (seconds).
pub const PRE_S: f64 = 0.001;
/// 25 % rule window before the anchor (seconds).
pub const RULE_PRE_S: f64 = 0.0015;
/// 25 % rule window after the anchor (seconds).
pub const RULE_POST_S: f64 = 0.010;

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
pub fn hilbert_env(seg: &[f64]) -> Vec<f64> {
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
pub fn convolve_same(a: &[f64], v: &[f64]) -> Vec<f64> {
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
pub fn log_envelope(seg: &[f64], k_sm: usize) -> Vec<f64> {
    let env = hilbert_env(seg);
    let kernel = vec![1.0 / k_sm as f64; k_sm];
    convolve_same(&env, &kernel)
        .into_iter()
        .map(|v| 20.0 * (v + 1e-9).log10())
        .collect()
}

/// `engine._refine_sr(x, t0, sr, neighbors, lo_ms, hi_ms)`: the refined onset in seconds and
/// `rise_db` (`None` where the engine returns `None`: a cut shorter than 8 samples or an empty
/// search window). `neighbors` are the other coarse hits the caller knows (`analyze_track`
/// passes the previous and the next one); the nearest below and above `t0` clip the search
/// window `win` at the midpoints. This is refinement version 1 ([`RefineRule::Reference`]),
/// pinned to the frozen references; [`refine_reach`] builds on it.
pub fn refine_sr(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow) -> (f64, Option<f64>) {
    let d = refine_detail(x, t0, sr, neighbors, win);
    (d.t, d.rise)
}

/// One run of the reference rule with the state the reach rule reads (P4-12 stage 1).
struct Detail {
    /// The reference result (seconds) and `rise_db`.
    t: f64,
    rise: Option<f64>,
    /// The midpoint towards the next coarse hit (absolute sample), when there is one.
    next_mid: Option<i64>,
    /// The previous coarse hit (seconds), when there is one.
    prev: Option<f64>,
    /// The 25 % rule, when the reference reached it.
    rule: Option<RuleState>,
}

/// Where the 25 % rule of the reference looked and what it found (absolute samples).
struct RuleState {
    /// The smoothed log envelope of the cut, and the cut's first sample.
    e: Vec<f64>,
    a: usize,
    /// The anchor, the steepest rise, and whether the search window had no rise (the anchor is
    /// then its last sample).
    anchor: i64,
    k: i64,
    flat: bool,
    /// The rule window `[i0, i1)` and its maximum `|x|`.
    i0: i64,
    i1: i64,
    mx: f32,
    /// The first sample at 25 % of `mx`; `None` when the window is silent (`mx` under 1e-4).
    cross: Option<i64>,
}

impl RuleState {
    /// The reference gave `t0` back from the rule: a silent window, or no rise and a crossing at
    /// the window start.
    fn gave_t0(&self) -> bool {
        self.cross.is_none() || (self.flat && self.cross == Some(self.i0))
    }
}

/// The body of [`refine_sr`] (every operation as before), keeping the state of the 25 % rule.
fn refine_detail(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow) -> Detail {
    let mut prv: Option<f64> = None;
    let mut nxt: Option<f64> = None;
    for &nt in neighbors {
        if nt < t0 && prv.is_none_or(|p| nt > p) {
            prv = Some(nt);
        } else if nt > t0 && nxt.is_none_or(|n| nt < n) {
            nxt = Some(nt);
        }
    }
    let srf = sr as f64;
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
    let cut: Vec<f64> = x[a..b].iter().map(|&v| v as f64).collect();
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
        if flat {
            t0
        } else {
            anchor as f64 / srf
        }
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

/// The refinement rule of an analysis (P4-12). The analysis records its version, so a project
/// analysed before keeps its hit times until the engineer analyses it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RefineRule {
    /// Version 1: [`refine_sr`], the retired engine's rule, pinned to the frozen references.
    /// Projects analysed before P4-12 carry no version and run this rule.
    #[default]
    Reference,
    /// Version 2 (P4-12 stage 1): [`refine_reach`], the reference with the reach and the tail fix.
    Reach,
    /// Version 3 (P4-12 stage 2): [`refine_first_motion`], version 2 and then the back-track to
    /// the first motion of the hit.
    FirstMotion,
    /// Version 4 (#101): [`RefineRule::FirstMotion`] with the foot taken one step stricter. The
    /// back-track's first sample over its threshold is the first sample that leaves the floor, or
    /// later; the foot is the last floor-level sample before it ([`foot_strict`]).
    Foot,
}

impl RefineRule {
    /// The rule a new analysis runs.
    pub const CURRENT: RefineRule = RefineRule::FirstMotion;

    /// The version number the document stores.
    pub fn version(self) -> u32 {
        match self {
            RefineRule::Reference => 1,
            RefineRule::Reach => 2,
            RefineRule::FirstMotion => 3,
            RefineRule::Foot => 4,
        }
    }

    /// The rule of a stored version number; `None` for a version this build does not know.
    pub fn from_version(v: u64) -> Option<RefineRule> {
        match v {
            1 => Some(RefineRule::Reference),
            2 => Some(RefineRule::Reach),
            3 => Some(RefineRule::FirstMotion),
            4 => Some(RefineRule::Foot),
            _ => None,
        }
    }
}

/// How far after the coarse time the reach rule looks for the attack (seconds). The DSP
/// detector gives the frame before the attack, 6 to 15 ms early (P4-11b verifier notes), and
/// the reference's rule window ends 10 ms after its anchor.
pub const REACH_S: f64 = 0.020;
/// The stretch before the rule window whose maximum is the floor of the reach test (seconds).
pub const FLOOR_S: f64 = 0.010;
/// The rule window holds no attack when the reach after it is at least this many times louder
/// than the window's maximum ...
pub const REACH_GAIN: f32 = 2.0;
/// ... and the window's maximum (without its last [`EDGE_S`]) is at most this many times the
/// floor before it (about 10 dB: a real floor varies by 6 to 7 dB between two 10 ms
/// stretches), or the reference gave the coarse time back, ...
pub const FLOOR_RATIO: f32 = 3.0;
/// ... or the window's maximum is this many times (about 30 dB) under the attack that follows:
/// no first motion of that hit. A quieter event 12 to 20 dB under the hit (a grace note,
/// bleed) stays the hit's onset.
pub const DEEP_RATIO: f32 = 30.0;
/// The end of the rule window that the floor test leaves out (seconds): an attack that starts
/// there is cut by the window, and its anchor was a ripple of the floor.
pub const EDGE_S: f64 = 0.001;
/// The tail test: the stretch before the rule window that must already hold the 25 % level
/// for a crossing before the anchor to count as the previous hit's tail (seconds), ...
pub const TAIL_S: f64 = 0.005;
/// ... and the previous coarse hit lies at most this far (seconds) before the rule window: the
/// tail belongs to an earlier hit (sixteenths at 100 BPM), not to the hit's own body ...
pub const TAIL_PREV_S: f64 = 0.150;
/// ... and does not rise there: the envelope's mean over its second half is at most this much
/// (dB) above the first half's (a snare tail decays by under 1 dB in 5 ms, a slow attack rises
/// by several dB), ...
pub const TAIL_RISE_DB: f64 = 3.0;
/// ... and the envelope falls by at least this much (dB) from the crossing to its lowest
/// point before the steepest rise (the trough between the tail and the attack).
pub const TROUGH_DB: f64 = 0.5;
/// A reference crossing less than this before the foot of the attack the reach found (seconds)
/// is that attack's start, cut by the rule window's end: the reference stands.
pub const REACH_GAP_S: f64 = 0.002;
/// The foot of that attack: walking back from its start while the maximum of the last this
/// many seconds stays above the floor's maximum.
pub const FOOT_S: f64 = 0.00025;

/// What the reach rule did with a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReachStep {
    /// The reference result stands.
    Kept,
    /// The rule window held no attack; the 25 % rule ran from the foot of the attack later in
    /// the reach.
    Reached,
    /// The 25 % crossing lay on the previous hit's tail; the rule starts at the trough after it.
    Tail,
    /// The reference gave the coarse time back (no attack in its window) and the reach found
    /// none either: the coarse time stays (flagged).
    Unresolved,
}

/// What the first-motion back-track did with a hit (P4-12 stage 2, [`first_motion`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackStep {
    /// The rule of the analysis has no back-track (versions 1 and 2).
    Off,
    /// The hit moved back to the foot of its attack.
    Moved,
    /// The foot is the version 2 time itself (a sharp start), or the peak is under the silence limit.
    Kept,
    /// The walk reached its bound without a gap: bleed or a tail above the threshold (a busy
    /// context). The version 2 time stays.
    Busy,
    /// The walk crossed more than [`FM_LONG_S`] of signal well under the hit: a separate earlier
    /// event, not the hit's own start. The version 2 time stays.
    Separate,
}

/// A refined hit: the onset (seconds), `rise_db`, what the reach rule did and what the back-track did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Refined {
    /// Onset in seconds.
    pub t: f64,
    /// `rise_db` of the refinement that gave `t`.
    pub rise: Option<f64>,
    /// The step of [`refine_reach`] ([`ReachStep::Kept`] for the reference rule).
    pub step: ReachStep,
    /// The step of the back-track ([`BackStep::Off`] for versions 1 and 2).
    pub back: BackStep,
}

/// The largest `|x|` in `[from, to)`, clipped to the signal; 0 for an empty range.
fn max_abs(x: &[f32], from: i64, to: i64) -> f32 {
    let f = from.max(0) as usize;
    let t = (to.max(0) as usize).min(x.len());
    if t <= f {
        return 0.0;
    }
    x[f..t].iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// The tail fix (C1): the reference's crossing lies before the anchor, the previous coarse hit is
/// at most [`TAIL_PREV_S`] before the rule window, the 5 ms before the rule window already hold
/// the 25 % level but not the window's maximum (louder there is the hit's own attack: the window
/// is past it) and do not rise there ([`TAIL_RISE_DB`]: a slow attack of the hit itself rises
/// there), and the envelope falls from
/// the crossing into a trough before the steepest rise: the crossing is on the previous hit's
/// tail. The result is the first sample of the steepest 0.5 ms rise at 25 % whose envelope is
/// above the tail's highest level between the crossing and that rise. `None` when the crossing
/// is not on a tail or the attack never rises above the tail. The caller takes the result only
/// when it is later than the reference's (a tail makes a hit early, never late).
fn tail_fix(x: &[f32], rs: &RuleState, prev: Option<f64>, srf: f64) -> Option<f64> {
    let cross = rs.cross?;
    if rs.flat || !prev.is_some_and(|p| rs.i0 as f64 / srf - p <= TAIL_PREV_S) {
        return None;
    }
    let thr = 0.25f32 * rs.mx;
    let span = py_int(TAIL_S * srf);
    // the envelope covers the cut from `a` on; the walk-back can put the anchor near its start
    let base = rs.a as i64;
    // the 5 ms before the window hold the 25 % level of a tail, but not the window's maximum: louder
    // there is the hit's own attack, and the window is past it (a late reference, no tail)
    let pre = max_abs(x, rs.i0 - span, rs.i0);
    if cross >= rs.anchor || rs.i0 - span < base || pre < thr || pre >= rs.mx {
        return None;
    }
    // a tail does not rise before the window (a slow attack of the hit itself does)
    let mean = |from: i64, to: i64| {
        let (f, t) = ((from - base) as usize, (to - base) as usize);
        rs.e[f..t].iter().sum::<f64>() / (t - f).max(1) as f64
    };
    if mean(rs.i0 - span / 2, rs.i0) > mean(rs.i0 - span, rs.i0 - span / 2) + TAIL_RISE_DB {
        return None;
    }
    // the stretch from the crossing to the start of the steepest 0.5 ms rise: the tail
    let q = (rs.k - py_int(RISE_S * srf)).max(cross);
    let (c, q) = ((cross - base) as usize, (q - base) as usize);
    let m = (c..=q).fold(c, |m, i| if rs.e[i] < rs.e[m] { i } else { m });
    if rs.e[m] > rs.e[c] - TROUGH_DB {
        return None;
    }
    let tail_db = rs.e[c..=q].iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (base + q as i64..=rs.k)
        .find(|&i| x[i as usize].abs() >= thr && rs.e[(i - base) as usize] >= tail_db)
        .map(|i| i as f64 / srf)
}

/// Refinement version 2 (P4-12 stage 1, [`RefineRule::Reach`]): the reference [`refine_sr`],
/// with two fixes that change a hit only where the reference is wrong, and only to a later time.
///
/// 1. Reach (M3): when the reference's 25 % rule window holds no attack and the reach after it
///    does ([`missed_attack`]), the 25 % rule runs from that attack's foot
///    ([`ReachStep::Reached`]).
/// 2. Tail (C1): a 25 % crossing on the previous hit's tail ([`tail_fix`]) is searched again
///    within the steepest rise, above the tail's level ([`ReachStep::Tail`]).
///
/// When the reference gave `t0` back (no attack in its window) and the reach finds none either,
/// `t0` stays and the hit is flagged [`ReachStep::Unresolved`]. Everywhere else the result is
/// the reference's, bit for bit; `rise` is always the reference's.
pub fn refine_reach(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow) -> Refined {
    let srf = sr as f64;
    let d = refine_detail(x, t0, sr, neighbors, win);
    let kept = Refined {
        t: d.t,
        rise: d.rise,
        step: ReachStep::Kept,
        back: BackStep::Off,
    };
    let Some(rs) = &d.rule else {
        return kept;
    };
    let fixed = |t: Option<f64>, step: ReachStep| match t.filter(|&t| t > d.t) {
        Some(t) => Refined {
            t,
            rise: d.rise,
            step,
            back: BackStep::Off,
        },
        None => kept,
    };
    if let Some(foot) = missed_attack(x, t0, &d, rs, srf) {
        // the 25 % rule from the attack's foot: the first sample at 25 % of the maximum of the
        // rule window's length after it
        let i1 = (foot + py_int(RULE_POST_S * srf)).min(x.len() as i64);
        let thr = 0.25f32 * max_abs(x, foot, i1);
        let t = (foot..i1).find(|&i| x[i as usize].abs() >= thr).map(|i| i as f64 / srf);
        return fixed(t, ReachStep::Reached);
    }
    if let Some(t) = tail_fix(x, rs, d.prev, srf).filter(|&t| t > d.t) {
        return fixed(Some(t), ReachStep::Tail);
    }
    if rs.gave_t0() {
        // the window held no attack and the reach finds none: the coarse time stays, flagged
        return Refined {
            step: ReachStep::Unresolved,
            ..kept
        };
    }
    kept
}

/// The foot of the attack the reference's rule window missed (M3), or `None` when the window
/// holds the hit's attack. The window holds none when the reach after it (to [`REACH_S`] after
/// `t0`, never past the midpoint to the next coarse hit) is [`REACH_GAIN`] times louder than the
/// window and the window is silent (the reference gave `t0` back), a floor or a decaying tail
/// (its maximum without the last [`EDGE_S`] at most [`FLOOR_RATIO`] times the 10 ms before it),
/// or [`DEEP_RATIO`] times under the reach. The attack starts at the first sample of the reach
/// at [`REACH_GAIN`] times the window's maximum (at least the silence limit 1e-4); its foot is
/// back from there while the last [`FOOT_S`] stays above the floor's maximum (into the window
/// when the window's end cut the attack, or when a soft start of the hit is in it). A reference
/// crossing less than [`REACH_GAP_S`] before the foot is the attack's start: `None`.
fn missed_attack(x: &[f32], t0: f64, d: &Detail, rs: &RuleState, srf: f64) -> Option<i64> {
    let mut end = py_int((t0 + REACH_S) * srf).min(x.len() as i64);
    if let Some(m) = d.next_mid {
        end = end.min(m);
    }
    let after = max_abs(x, rs.i1, end);
    let floor = max_abs(x, rs.i0 - py_int(FLOOR_S * srf), rs.i0);
    // the window's maximum without its last millisecond: an attack that only starts there is not in it
    let held = max_abs(x, rs.i0, rs.i1 - py_int(EDGE_S * srf));
    let missed = after >= 1e-4f32
        && after >= REACH_GAIN * rs.mx
        && (rs.gave_t0() || held <= FLOOR_RATIO * floor || DEEP_RATIO * rs.mx <= after);
    if !missed {
        return None;
    }
    // it exists: after >= REACH_GAIN * mx and after >= 1e-4 (a silent window's maximum can be 0)
    let start = (REACH_GAIN * rs.mx).max(1e-4f32);
    let c = (rs.i1..end).find(|&i| x[i as usize].abs() >= start)?;
    let w = py_int(FOOT_S * srf).max(1);
    let mut foot = c;
    while foot > rs.i0 && max_abs(x, foot - w, foot) > floor {
        foot -= 1;
    }
    match rs.cross {
        Some(cross) if foot - cross < py_int(REACH_GAP_S * srf) => None,
        _ => Some(foot),
    }
}

/// The peak window of the back-track: the attack's peak lies in `s .. s + FM_PEAK_S` (seconds).
pub const FM_PEAK_S: f64 = 0.005;
/// The back-track's threshold under that peak (dB). P4-11 table A8 started at 30 dB; the kit
/// measured 33 dB closer to the first motion (`aud`) with the same spread in noise and bleed.
pub const FM_PEAK_DB: f64 = 33.0;
/// The floor before the hit: the median of the [`FM_BLOCK_S`] block maxima in the
/// `FM_FLOOR_S` before the hit, ...
pub const FM_FLOOR_S: f64 = 0.050;
/// ... without the last `FM_FLOOR_SKIP_S` (the attack's own soft start), ...
pub const FM_FLOOR_SKIP_S: f64 = 0.002;
/// ... in blocks of this length (seconds) ...
pub const FM_BLOCK_S: f64 = 0.001;
/// ... and the threshold is at least this much (dB) above it: noise and bleed before the hit set
/// the threshold, not only the peak.
pub const FM_FLOOR_DB: f64 = 6.0;
/// A run of samples under the threshold this long (seconds) ends the attack; a shorter gap (a
/// zero crossing) does not.
pub const FM_GAP_S: f64 = 0.0005;
/// The walk never goes further back than this (seconds), nor past the midpoint to the previous
/// coarse hit.
pub const FM_SPAN_S: f64 = 0.010;
/// A walk longer than this (seconds) ...
pub const FM_LONG_S: f64 = 0.005;
/// ... is taken only when the stretch it crosses (without its last `FM_LONG_END_S`) holds a level
/// within [`FM_LONG_DB`] of the first `FM_LONG_END_S` at the version 2 time: that time sat on the
/// hit's body. Else the stretch is a separate earlier event.
pub const FM_LONG_END_S: f64 = 0.0005;
/// See [`FM_LONG_END_S`] (dB).
pub const FM_LONG_DB: f64 = 6.0;

/// P4-31 (candidate A): the version of the kick walk, recorded as `detector.kick_walk`; 0 means the shipped margin.
pub const KICK_WALK_VERSION: u32 = 1;
/// Version 1 of the kick walk: the back-track's peak margin on kick tracks (dB). A kick's peak comes 6 to 11 ms after
/// its foot; at 29 dB the walk stops later than at [`FM_PEAK_DB`] (P4-22: 88 to 78 moved kicks on the seven songs, 37 to 30
/// on the two held-out sessions).
/// Seconds to samples, rounded (the back-track's spans are in time, not samples: P4-11b R4).
fn samples(s: f64, srf: f64) -> i64 {
    (s * srf).round() as i64
}

/// The 2-tap mean `|x[i] + x[i - 1]| / 2` (`x[-1]` = 0). It nulls the Nyquist ripple of a
/// band-limited sharp start, so the walk does not read that ripple as motion.
fn two_tap(x: &[f32], i: usize) -> f64 {
    let prev = if i > 0 { x[i - 1] as f64 } else { 0.0 };
    (x[i] as f64 + prev).abs() / 2.0
}

/// The floor before sample `s`: the median of the [`FM_BLOCK_S`] block maxima of `|x|` in
/// `[from, s - FM_FLOOR_SKIP_S)`, the blocks counted back from the end; 0 with no full block.
fn floor_before(x: &[f32], s: i64, from: i64, srf: f64) -> f64 {
    let end = (s - samples(FM_FLOOR_SKIP_S, srf)).min(x.len() as i64);
    let b = samples(FM_BLOCK_S, srf).max(1);
    let from = from.max(0);
    if end - from < b {
        return 0.0;
    }
    let nb = (end - from) / b;
    let mut m: Vec<f64> = (0..nb)
        .map(|k| max_abs(x, end - (k + 1) * b, end - k * b) as f64)
        .collect();
    m.sort_by(|a, c| a.total_cmp(c));
    let n = m.len();
    if n % 2 == 1 {
        m[n / 2]
    } else {
        (m[n / 2 - 1] + m[n / 2]) / 2.0
    }
}

/// The first-motion back-track (P4-12 stage 2): from the refined sample `s`, walk back to the
/// foot of the hit's attack. `prev` and `next` are the neighbouring coarse hits (seconds) and
/// `t0` the hit's own coarse time.
///
/// The threshold is the larger of [`FM_PEAK_DB`] under the peak in `s .. s + FM_PEAK_S`
/// (clipped at the midpoint to `next`) and [`FM_FLOOR_DB`] above the floor before the hit
/// ([`floor_before`], from `s - FM_FLOOR_S` or the midpoint to `prev`). The foot is the first
/// sample of the run at or above it ([`two_tap`]) that holds `s`; gaps under [`FM_GAP_S`] do not
/// end the run. The walk stops at `s - FM_SPAN_S` and at the midpoint to `prev`: a walk that
/// reaches that bound without a gap keeps `s` ([`BackStep::Busy`]). A walk of more than
/// [`FM_LONG_S`] is taken only over the hit's own body ([`BackStep::Separate`] else). The result is
/// never later than `s`.
pub fn first_motion(x: &[f32], s: i64, sr: u32, t0: f64, prev: Option<f64>, next: Option<f64>) -> (i64, BackStep) {
    first_motion_peak(x, s, sr, t0, prev, next, FM_PEAK_DB)
}

/// [`first_motion`] with another peak margin `peak_db` in place of [`FM_PEAK_DB`] (P4-31: kick tracks).
pub fn first_motion_peak(
    x: &[f32],
    s: i64,
    sr: u32,
    t0: f64,
    prev: Option<f64>,
    next: Option<f64>,
    peak_db: f64,
) -> (i64, BackStep) {
    back_track(x, s, sr, t0, prev, next, peak_db, false)
}

/// The back-track of [`first_motion_peak`]; `strict` takes the foot one step stricter ([`foot_strict`], version 4).
fn back_track(
    x: &[f32],
    s: i64,
    sr: u32,
    t0: f64,
    prev: Option<f64>,
    next: Option<f64>,
    peak_db: f64,
    strict: bool,
) -> (i64, BackStep) {
    let srf = sr as f64;
    let prev_mid = prev.map(|p| py_int(((p + t0) / 2.0) * srf));
    let next_mid = next.map(|nx| py_int(((t0 + nx) / 2.0) * srf));
    first_motion_mid_rule(x, s, sr, prev_mid, next_mid, peak_db, strict)
}

/// [`first_motion`] with the bounds as samples: `prev_mid` bounds the walk and the floor window (the midpoint to the
/// previous hit), `next_mid` the peak window (the midpoint to the next hit). P4-13 stage B: the flam rule walks from a
/// second stroke's re-rise with `prev_mid` at the dip after the first stroke (`tools/onset_guard.py` `walk`).
pub fn first_motion_mid(x: &[f32], s: i64, sr: u32, prev_mid: Option<i64>, next_mid: Option<i64>) -> (i64, BackStep) {
    first_motion_mid_rule(x, s, sr, prev_mid, next_mid, FM_PEAK_DB, false)
}

/// The strict foot of version 4 (#101). `d` is the walk's first sample over the threshold. Walking back from
/// it, the floor is the last two samples at or under `floor` (two in a row: a single quiet sample inside a
/// noisy rise is not the floor). The foot is the last floor-level sample before the first sample that leaves
/// that floor after it. No such pair above `lo`: `d` stays. The samples are read raw: the 2-tap mean halves
/// the first sample of a slow rise and reads it as floor.
fn foot_strict(x: &[f32], d: i64, lo: i64, floor: f64) -> i64 {
    let quiet = |i: i64| (x[i as usize].abs() as f64) <= floor;
    let mut i = d;
    while i - 2 >= lo && !(quiet(i - 1) && quiet(i - 2)) {
        i -= 1;
    }
    if i - 2 < lo {
        return d;
    }
    let mut j = i - 1;
    while j < d && quiet(j) {
        j += 1;
    }
    j - 1
}

/// [`first_motion_mid`] with another peak margin `peak_db` in place of [`FM_PEAK_DB`], and the foot taken
/// strict ([`foot_strict`]) when `strict`.
fn first_motion_mid_rule(
    x: &[f32],
    s: i64,
    sr: u32,
    prev_mid: Option<i64>,
    next_mid: Option<i64>,
    peak_db: f64,
    strict: bool,
) -> (i64, BackStep) {
    let srf = sr as f64;
    let n = x.len() as i64;
    if s <= 0 || s >= n {
        return (s, BackStep::Kept);
    }
    let mut pe = (s + samples(FM_PEAK_S, srf)).min(n);
    if let Some(m) = next_mid {
        pe = pe.min(m);
    }
    let pk = max_abs(x, s, pe) as f64;
    if pk < 1e-4 {
        return (s, BackStep::Kept);
    }
    let mut lo = (s - samples(FM_SPAN_S, srf)).max(0);
    let mut from = s - samples(FM_FLOOR_S, srf);
    if let Some(m) = prev_mid {
        lo = lo.max(m);
        from = from.max(m);
    }
    let floor = floor_before(x, s, from, srf);
    let thr = (pk * 10f64.powf(-peak_db / 20.0)).max(floor * 10f64.powf(FM_FLOOR_DB / 20.0));
    let gap = samples(FM_GAP_S, srf).max(1);
    let mut i = s;
    let mut foot = s;
    let mut ended = false;
    while i > lo {
        i -= 1;
        if two_tap(x, i as usize) >= thr {
            foot = i;
        } else if foot - i >= gap {
            ended = true;
            break;
        }
    }
    if !ended {
        return (s, BackStep::Busy);
    }
    if s - foot > samples(FM_LONG_S, srf) {
        let e = samples(FM_LONG_END_S, srf).max(1);
        let held = max_abs(x, foot, (s - e).max(foot + 1)) as f64;
        let at = max_abs(x, s, s + e) as f64;
        if held < at * 10f64.powf(-FM_LONG_DB / 20.0) {
            return (s, BackStep::Separate);
        }
    }
    let foot = if strict {
        let end = (s - samples(FM_FLOOR_SKIP_S, srf)).min(n);
        foot_strict(x, foot, lo, max_abs(x, from, end) as f64)
    } else {
        foot
    };
    if foot < s {
        (foot, BackStep::Moved)
    } else {
        (s, BackStep::Kept)
    }
}

/// Refinement version 3 (P4-12 stage 2, [`RefineRule::FirstMotion`]): [`refine_reach`], then the
/// back-track [`first_motion`] from its time. `rise` and the reach step are version 2's; a hit
/// moves only earlier, and only where the back-track finds the foot of its attack.
pub fn refine_first_motion(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow) -> Refined {
    refine_first_motion_peak(x, t0, sr, neighbors, win, FM_PEAK_DB)
}

/// [`refine_first_motion`] with another back-track peak margin (P4-31).
pub fn refine_first_motion_peak(
    x: &[f32],
    t0: f64,
    sr: u32,
    neighbors: &[f64],
    win: RefineWindow,
    peak_db: f64,
) -> Refined {
    refine_back_peak(x, t0, sr, neighbors, win, peak_db, false)
}

/// [`refine_first_motion_peak`] with the strict foot of version 4 ([`RefineRule::Foot`]) when `strict`.
fn refine_back_peak(
    x: &[f32],
    t0: f64,
    sr: u32,
    neighbors: &[f64],
    win: RefineWindow,
    peak_db: f64,
    strict: bool,
) -> Refined {
    let r = refine_reach(x, t0, sr, neighbors, win);
    let srf = sr as f64;
    let (mut prev, mut next): (Option<f64>, Option<f64>) = (None, None);
    for &nt in neighbors {
        if nt < t0 && prev.is_none_or(|p| nt > p) {
            prev = Some(nt);
        } else if nt > t0 && next.is_none_or(|q| nt < q) {
            next = Some(nt);
        }
    }
    let s = (r.t * srf).round() as i64;
    let (foot, back) = back_track(x, s, sr, t0, prev, next, peak_db, strict);
    Refined {
        t: if back == BackStep::Moved {
            foot as f64 / srf
        } else {
            r.t
        },
        back,
        ..r
    }
}

/// [`refine_reach`] or [`refine_sr`], as `rule` says.
pub fn refine_rule(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow, rule: RefineRule) -> Refined {
    refine_rule_peak(x, t0, sr, neighbors, win, rule, FM_PEAK_DB)
}

/// [`refine_rule`] with the back-track peak margin `peak_db` (it matters for [`RefineRule::FirstMotion`] only).
pub fn refine_rule_peak(
    x: &[f32],
    t0: f64,
    sr: u32,
    neighbors: &[f64],
    win: RefineWindow,
    rule: RefineRule,
    peak_db: f64,
) -> Refined {
    match rule {
        RefineRule::Reference => {
            let (t, rise) = refine_sr(x, t0, sr, neighbors, win);
            Refined {
                t,
                rise,
                step: ReachStep::Kept,
                back: BackStep::Off,
            }
        }
        RefineRule::Reach => refine_reach(x, t0, sr, neighbors, win),
        RefineRule::FirstMotion => refine_first_motion_peak(x, t0, sr, neighbors, win, peak_db),
        RefineRule::Foot => refine_back_peak(x, t0, sr, neighbors, win, peak_db, true),
    }
}

/// [`refine_track`] with the rule of the analysis: the previous and the next coarse hit are the
/// neighbours of each one.
pub fn refine_track_rule(x: &[f32], sr: u32, coarse: &[f64], win: RefineWindow, rule: RefineRule) -> Vec<Refined> {
    refine_track_rule_peak(x, sr, coarse, win, rule, FM_PEAK_DB)
}

/// [`refine_track_rule`] with the back-track peak margin `peak_db` (P4-31).
pub fn refine_track_rule_peak(
    x: &[f32],
    sr: u32,
    coarse: &[f64],
    win: RefineWindow,
    rule: RefineRule,
    peak_db: f64,
) -> Vec<Refined> {
    coarse
        .iter()
        .enumerate()
        .map(|(i, &t)| refine_rule_peak(x, t, sr, &neighbours(coarse, i), win, rule, peak_db))
        .collect()
}

/// The previous and the next coarse hit of hit `i`.
fn neighbours(coarse: &[f64], i: usize) -> Vec<f64> {
    let mut nb = Vec::with_capacity(2);
    if i > 0 {
        nb.push(coarse[i - 1]);
    }
    if i + 1 < coarse.len() {
        nb.push(coarse[i + 1]);
    }
    nb
}

/// Refine every coarse hit of a track the way `engine.analyze_track` does: the previous and
/// the next coarse hit are the neighbours of each one. `coarse` is the min-gap deduplicated,
/// ascending detector output; the result is in the same order (not re-sorted, not re-deduped).
pub fn refine_track(x: &[f32], sr: u32, coarse: &[f64], win: RefineWindow) -> Vec<(f64, Option<f64>)> {
    coarse
        .iter()
        .enumerate()
        .map(|(i, &t)| refine_sr(x, t, sr, &neighbours(coarse, i), win))
        .collect()
}

/// A hit as the document stores it: time in seconds, velocity, optional `rise` (dB).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Onset in seconds.
    pub t: f64,
    /// MIDI velocity.
    pub v: i64,
    /// Envelope rise at the anchor in dB (`rise_db`), when the refinement produced one.
    pub rise: Option<f64>,
}

/// `engine.recompute_velocities`: keep every hit time and every other field, replace only
/// the velocity with `velocity_map` at the hit times.
pub fn recompute_velocities(x: &[f32], sr: u32, hits: &mut [Hit], m: &VelocityMap) {
    let times: Vec<f64> = hits.iter().map(|h| h.t).collect();
    for (h, v) in hits.iter_mut().zip(velocity_map(x, sr, &times, m)) {
        h.v = v;
    }
}

#[cfg(test)]
mod tests;
