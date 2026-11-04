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
//! numpy's pocketfft) and the smoothing sum; `tests/test_golden.py` (the refine cases)
//! checks that on the corpus, edge, window and refinement-benchmark scenarios.

use std::cell::RefCell;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::dsp::amp_db_add;
use crate::pyops::py_int;
use crate::velocity::{VelocityMap, velocity_map};

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
#[must_use]
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
#[must_use]
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
#[must_use]
pub fn log_envelope(seg: &[f64], k_sm: usize) -> Vec<f64> {
    let env = hilbert_env(seg);
    let kernel = vec![1.0 / k_sm as f64; k_sm];
    convolve_same(&env, &kernel).into_iter().map(amp_db_add).collect()
}

/// `engine._refine_sr(x, t0, sr, neighbors, lo_ms, hi_ms)`: the refined onset in seconds and
/// `rise_db` (`None` where the engine returns `None`: a cut shorter than 8 samples or an empty
/// search window). `neighbors` are the other coarse hits the caller knows (`analyze_track`
/// passes the previous and the next one); the nearest below and above `t0` clip the search
/// window `win` at the midpoints. This is refinement version 1 ([`RefineRule::Reference`]),
/// pinned to the frozen references; [`refine_reach`] builds on it.
#[must_use]
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
    /// later; the foot is the last floor-level sample before it ([`foot_strict`]). The strict foot
    /// moves at most [`FM_STRICT_MAX_S`] before the version 3 foot, else the version 3 foot stays.
    Foot,
    /// Version 5 (#127): [`RefineRule::Foot`] with the strict foot's move capped at [`FOOT5_MAX_SAMPLES`]
    /// samples before the version 3 foot (the foot moves to the cap).
    FootCapped,
    /// Version 6 (#293, #294): the version 3 foot (no strict foot, no lead-in), then the first steep edge after it
    /// ([`edge_foot`]): a hit that the walk put on the bleed or the soft drift before its attack moves to the foot of
    /// the attack's first steep rise.
    Edge,
    /// Version 7 (#321): [`RefineRule::Edge`] whose last walk-back to the line follows what the engineer sees: back over
    /// the samples that stand visibly off the line ([`VIS_TAIL_PCT`] of the hit's peak) with the edge's sign, when that
    /// walk reaches the line within [`VIS_TAIL_MAX_S`] ([`EdgeTail::Visible`]); else version 6's walk. Version 6 judged the tail against the line noise and took at most 4 samples:
    /// on a nearly silent kick line it walked onto invisible steps (2 to 3 samples early), on a snare line with hi-hat
    /// bleed it stopped inside the slow drift (2 to 7 samples late).
    Visible,
}

/// How the steep edge's foot walks back to the line (#321): [`EdgeTail::Noise`] is version 6, [`EdgeTail::Visible`]
/// version 7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeTail {
    /// Up to [`EDGE_TAIL_SAMPLES`] samples while the level rises by more than [`EDGE_TAIL_NOISE`] times the line noise.
    Noise,
    /// Back while the sample stands more than [`VIS_TAIL_PCT`] of the hit's peak off the line, keeps the edge's sign
    /// and does not grow towards the line by more than [`VIS_TAIL_TOL_PCT`] of the peak. It counts only when it reaches
    /// the line within [`VIS_TAIL_MAX_S`]; otherwise [`EdgeTail::Noise`] decides (a slow wave or an offset on a tom's
    /// line is the line itself).
    Visible,
}

impl RefineRule {
    /// The rule a new analysis runs.
    pub const CURRENT: RefineRule = RefineRule::Visible;

    /// The version number the document stores.
    #[must_use]
    pub fn version(self) -> u32 {
        match self {
            RefineRule::Reference => 1,
            RefineRule::Reach => 2,
            RefineRule::FirstMotion => 3,
            RefineRule::Foot => 4,
            RefineRule::FootCapped => 5,
            RefineRule::Edge => 6,
            RefineRule::Visible => 7,
        }
    }

    /// The rule of a stored version number; `None` for a version this build does not know.
    #[must_use]
    pub fn from_version(v: u64) -> Option<RefineRule> {
        match v {
            1 => Some(RefineRule::Reference),
            2 => Some(RefineRule::Reach),
            3 => Some(RefineRule::FirstMotion),
            4 => Some(RefineRule::Foot),
            5 => Some(RefineRule::FootCapped),
            6 => Some(RefineRule::Edge),
            7 => Some(RefineRule::Visible),
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
#[must_use]
pub fn refine_reach(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow) -> Refined {
    let srf = f64::from(sr);
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
pub const FM_FLOOR_S: f64 = 0.060;
/// ... without the last `FM_FLOOR_SKIP_S` (the attack's own rise and bleed; the same 60 to 12 ms window as the floor
/// of `lead_in_check`, #273), ...
pub const FM_FLOOR_SKIP_S: f64 = 0.012;
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
/// The version 4 guard (#101): the strict foot may move at most this far (seconds) before the
/// version 3 foot. Further than that, the version 3 foot stays.
pub const FM_STRICT_MAX_S: f64 = 0.00025;
/// Version 5 (#127): the strict foot moves at most this many samples before the version 3 foot, at every rate.
pub const FOOT5_MAX_SAMPLES: i64 = 3;

/// How the strict foot's move before the version 3 foot is capped: the cap and what a move beyond it does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FootTune {
    /// The largest move before the version 3 foot.
    pub cap: FootCap,
    /// What a move beyond the cap does.
    pub overrun: Overrun,
}

/// The cap of [`FootTune`]: a time (version 4) or a count of samples at every rate (version 5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FootCap {
    /// Seconds, rounded to samples at the rate.
    Seconds(f64),
    /// Samples, at every rate.
    Samples(i64),
}

/// What a strict foot beyond its cap does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overrun {
    /// The version 3 foot stays (version 4 and the version 5 default).
    Keep,
    /// The foot moves to the cap, the cap's last sample before the version 3 foot.
    Clamp,
}

impl FootTune {
    /// The version 4 foot: a move of at most [`FM_STRICT_MAX_S`], else the version 3 foot.
    pub const V4: FootTune = FootTune {
        cap: FootCap::Seconds(FM_STRICT_MAX_S),
        overrun: Overrun::Keep,
    };
    /// The version 5 foot: a move of at most [`FOOT5_MAX_SAMPLES`], else the foot moves to the cap.
    pub const V5: FootTune = FootTune {
        cap: FootCap::Samples(FOOT5_MAX_SAMPLES),
        overrun: Overrun::Clamp,
    };
}

/// What the back-track takes beside the signal and its bounds: the peak margin, the strict foot (versions 4 and 5) and
/// the steep edge after the foot (versions 6 and 7).
#[derive(Debug, Clone, Copy, PartialEq)]
struct BackTune {
    /// The threshold under the peak (dB): [`FM_PEAK_DB`], or another margin (P4-31).
    peak_db: f64,
    /// The foot taken one step stricter ([`foot_strict`]) with this cap, when `Some`.
    tune: Option<FootTune>,
    /// The first steep edge after the foot ([`edge_foot`]) with this tail rule, when `Some`.
    edge: Option<EdgeTail>,
}

impl BackTune {
    /// The version 3 back-track with the peak margin `peak_db`: no strict foot, no steep edge.
    const fn peak(peak_db: f64) -> BackTune {
        BackTune {
            peak_db,
            tune: None,
            edge: None,
        }
    }
}

/// Seconds to samples, rounded (the back-track's spans are in time, not samples: P4-11b R4).
fn samples(s: f64, srf: f64) -> i64 {
    (s * srf).round() as i64
}

/// The 2-tap mean `|x[i] + x[i - 1]| / 2` (`x[-1]` = 0). It nulls the Nyquist ripple of a
/// band-limited sharp start, so the walk does not read that ripple as motion.
fn two_tap(x: &[f32], i: usize) -> f64 {
    let prev = if i > 0 { f64::from(x[i - 1]) } else { 0.0 };
    (f64::from(x[i]) + prev).abs() / 2.0
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
        .map(|k| f64::from(max_abs(x, end - (k + 1) * b, end - k * b)))
        .collect();
    m.sort_by(f64::total_cmp);
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
#[must_use]
pub fn first_motion(x: &[f32], s: i64, sr: u32, t0: f64, prev: Option<f64>, next: Option<f64>) -> (i64, BackStep) {
    first_motion_peak(x, s, sr, t0, prev, next, FM_PEAK_DB)
}

/// [`first_motion`] with another peak margin `peak_db` in place of [`FM_PEAK_DB`] (P4-31: kick tracks).
#[must_use]
pub fn first_motion_peak(
    x: &[f32],
    s: i64,
    sr: u32,
    t0: f64,
    prev: Option<f64>,
    next: Option<f64>,
    peak_db: f64,
) -> (i64, BackStep) {
    back_track(x, s, sr, t0, (prev, next), BackTune::peak(peak_db))
}

/// The back-track of [`first_motion_peak`] with the settings `bt` ([`BackTune`]). `neighbors` are the previous and the
/// next hit time (`None` at the ends).
fn back_track(
    x: &[f32],
    s: i64,
    sr: u32,
    t0: f64,
    neighbors: (Option<f64>, Option<f64>),
    bt: BackTune,
) -> (i64, BackStep) {
    let srf = f64::from(sr);
    let (prev, next) = neighbors;
    let prev_mid = prev.map(|p| py_int(((p + t0) / 2.0) * srf));
    let next_mid = next.map(|nx| py_int(((t0 + nx) / 2.0) * srf));
    first_motion_traced(x, s, sr, prev_mid, next_mid, bt).0
}

/// [`first_motion`] with the bounds as samples: `prev_mid` bounds the walk and the floor window (the midpoint to the
/// previous hit), `next_mid` the peak window (the midpoint to the next hit). P4-13 stage B: the flam rule walks from a
/// second stroke's re-rise with `prev_mid` at the dip after the first stroke (`tools/onset_guard.py` `walk`).
#[must_use]
pub fn first_motion_mid(x: &[f32], s: i64, sr: u32, prev_mid: Option<i64>, next_mid: Option<i64>) -> (i64, BackStep) {
    first_motion_traced(x, s, sr, prev_mid, next_mid, BackTune::peak(FM_PEAK_DB)).0
}

/// The strict foot of version 4 (#101). `d` is the walk's first sample over the threshold. Walking back from
/// it, the floor is the last two samples at or under `floor` (two in a row: a single quiet sample inside a
/// noisy rise is not the floor). The foot is the last floor-level sample before the first sample that leaves
/// that floor after it. No such pair above `lo`: `d` stays. The samples are read raw: the 2-tap mean halves the
/// first sample of a slow rise and reads it as floor.
fn foot_strict(x: &[f32], d: i64, lo: i64, floor: f64) -> i64 {
    let quiet = |i: i64| f64::from(x[i as usize].abs()) <= floor;
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

/// The lead-in of the walk (#239), one switch for the whole process. Off since version 6 (#293, #294: it joined the
/// bleed before snare hits). Its setter went with the before/after measurement `lead_in_check` (removed in the
/// refactor, see git history); no product path turns it on.
static LEAD_IN: AtomicBool = AtomicBool::new(false);

/// The lead-in goes back at most this far (seconds) from the walk's foot: an attack's rise is about a millisecond. A
/// longer rise over the floor is bleed or an earlier stroke, not the hit.
pub const LEAD_IN_MAX_S: f64 = 0.0015;

/// What the walk of one hit decided (#239): the floor and threshold it used, the foot of the walk and the foot after
/// the lead-in (equal when the lead-in did not join a rise).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeadTrace {
    /// The floor before the hit.
    pub floor: f64,
    /// The walk's threshold.
    pub thr: f64,
    /// The foot of the walk, in samples.
    pub foot: i64,
    /// The foot after the lead-in, in samples.
    pub lead: i64,
}

/// The lead-in of a walk (#239). The threshold `thr` follows the peak, so a small first bump before the main swing
/// (under `thr`, over the floor) ends the walk at the swing. `foot` is the walk's foot: this goes on back from it over
/// the samples at or above [`FM_FLOOR_DB`] over the `floor` (the same gap rule), at most [`LEAD_IN_MAX_S`] and never
/// past `lo`, and returns the first sample of that rise. A rise that does not end within that bound, no floor, or a
/// `thr` that is not above that level: `foot` stays. `judge` is the end of the stretch that decides whether the rise
/// is a first bump (#271): the foot of the walk at the base margin, the same for every peak margin.
fn lead_in(x: &[f32], (foot, judge): (i64, i64), lo: i64, floor: f64, thr: f64, gap: i64, srf: f64) -> i64 {
    if !LEAD_IN.load(Ordering::Relaxed) {
        return foot;
    }
    let low = floor * 10f64.powf(FM_FLOOR_DB / 20.0);
    if low <= 0.0 || low >= thr {
        return foot;
    }
    let lo = lo.max(foot - samples(LEAD_IN_MAX_S, srf));
    let mut lead = foot;
    let mut i = foot;
    while i > lo {
        i -= 1;
        if two_tap(x, i as usize) >= low {
            lead = i;
        } else if lead - i >= gap {
            return if is_first_bump(x, lead, judge.max(lead), gap) {
                lead
            } else {
                foot
            };
        }
    }
    foot
}

/// Whether the stretch `lead .. foot` is a first bump of the hit and not something in front of a hard attack (#271):
/// - it rises toward `foot`: the level in its first half `gap` samples is under 0.8 of the level in the last `gap`
///   samples (a decaying tail of an earlier stroke is as loud at its far end as at its near end);
/// - it is no Nyquist ripple: the sign changes at most 3 of 4 times from one sample to the next (the pre-ringing of a
///   band-limited hard start alternates at nearly every sample; a bump of the hit does not).
fn is_first_bump(x: &[f32], lead: i64, foot: i64, gap: i64) -> bool {
    let w = gap.min(foot - lead);
    if w <= 0 {
        return false;
    }
    let far = f64::from(max_abs(x, lead, lead + (w / 2).max(1)));
    let near = f64::from(max_abs(x, foot - w, foot));
    let pairs = foot - lead - 1;
    let flips = (lead + 1..foot)
        .filter(|&i| x[i as usize] * x[i as usize - 1] < 0.0)
        .count() as i64;
    far < near * 0.8 && flips * 4 <= pairs * 3
}

/// Version 6 (#293, #294): the rise window of the steep edge (seconds): the signal must rise by [`EDGE_RISE_DB`] under
/// the peak within it.
pub const EDGE_RISE_S: f64 = 0.0001;
/// The rise that makes a steep edge (dB under the peak of the hit, within [`EDGE_RISE_S`]).
pub const EDGE_RISE_DB: f64 = 18.0;
/// The search stops where the level reaches this much (dB) under the hit's peak: the hit's body began without an
/// edge that counts, and the walk's foot stays.
pub const EDGE_TOP_DB: f64 = 12.0;
/// The guard: the level between the walk's foot and the edge (without the edge's own ramp-up) must stay this much
/// (dB) under the level the edge reaches within [`EDGE_LOCAL_S`]. A ripple inside the body of a soft hit, or a slow
/// attack without a clear jump, is no edge: the search goes on. Fitted on the owner's hand-placed hits (#293, kick
/// 1..332 and snare 1..168 of one session): 9 to 15 dB give the same result; 12 is the middle.
pub const EDGE_GUARD_DB: f64 = 12.0;
/// The window after an edge's foot whose peak the guard compares against (seconds): the level this edge reaches, not
/// a louder swing later in the hit.
pub const EDGE_LOCAL_S: f64 = 0.0005;
/// The tail of the foot's walk-back: up to this many samples more while the level still rises by more than
/// [`EDGE_TAIL_NOISE`] times the line noise, so the hit sits at the foot of the steep rise. Fitted on the owner's
/// hand-placed hits (#293): 4 and 6 give the same result; 4 is the smaller.
pub const EDGE_TAIL_SAMPLES: i64 = 4;
/// The tail's step: a rise of more than this many times the line noise (the median `|x[n] - x[n-1]|` over the
/// [`EDGE_NOISE_S`] before the walk's foot) still belongs to the hit. Fitted on the owner's hand-placed hits (#293):
/// 2 took the slow drift of a kick's line for the hit (1 to 3 samples early); 8 does not.
pub const EDGE_TAIL_NOISE: f64 = 8.0;
/// Version 7 (#321): a sample stands visibly off the line when `|x|` exceeds this share of the hit's peak (percent).
/// Fitted on the owner's hand-placed hits (kick 1..332, snare 1..168 of one session; even hits chose, odd hits
/// checked): 2 and 3 give the same held-out result; 2 is the smaller.
pub const VIS_TAIL_PCT: f64 = 2.0;
/// Version 7: the walk-back to the line goes on while the sample before is no more than this share of the peak
/// (percent) larger than the current one: the small wiggles of a snare's drift do not stop it.
pub const VIS_TAIL_TOL_PCT: f64 = 0.5;
/// Version 7: the longest walk-back to the line (seconds) from the edge's first rising sample. A drift the engineer
/// counts as part of the stroke is short: on the owner's hand-placed hits every limit from 6 samples (0.125 ms at
/// 48 kHz) up keeps the whole gain. A longer stretch off the line is the line itself (a slow wave or an offset of a tom
/// line, Sessions A and B): with a 1 ms limit the walk followed it down to a zero crossing up to 1 ms before the stroke.
pub const VIS_TAIL_MAX_S: f64 = 0.000_15;
/// The stretch before the walk's foot that measures the line noise (seconds).
pub const EDGE_NOISE_S: f64 = 0.002;
/// How far after the walk's foot the edge is looked for (seconds).
pub const EDGE_SPAN_S: f64 = 0.005;

/// The foot of the first steep edge after the walk's foot (version 6, #293, #294). The walk stops at the first sample
/// over its threshold, which lies on the bleed or the soft drift that often precedes a snare or kick attack; the
/// engineer puts the hit where the waveform visibly starts. An edge is a sample `j` from one rise window before
/// `foot` (a steep attack's walk foot lies inside its rise) to `end` (at most
/// [`EDGE_SPAN_S`] after the foot: the peak and the search stay on this hit, not on a louder later swing) where
/// `|x[j + w] - x[j]|` (w = [`EDGE_RISE_S`]) reaches [`EDGE_RISE_DB`] under the peak of `[foot, end)`. Its foot is the
/// first sample in that window whose step to the next sample is half the edge's per-sample rise, walked back while
/// the step before it stays as large (a quarter let the floor noise through), then up to [`EDGE_TAIL_SAMPLES`] more
/// while the level rises by more than [`EDGE_TAIL_NOISE`] times the line noise (at least a sixteenth of the edge's
/// per-sample rise: a digitally silent line has no noise). The edge counts when the level between `foot` and its foot (without the last `w` samples, the edge's own
/// ramp-up) lies [`EDGE_GUARD_DB`] under the peak of the [`EDGE_LOCAL_S`] after its foot; else the search goes on after
/// the window. An edge raises `|x|` (a falling slope is none), and the search stops where the level itself reaches
/// [`EDGE_TOP_DB`] under the peak of `[foot, end)`. The first edge that counts is the hit; none: `foot` stays.
fn edge_foot(x: &[f32], foot: i64, end: i64, srf: f64, tail_rule: EdgeTail) -> i64 {
    let w = samples(EDGE_RISE_S, srf).max(1);
    let n = x.len() as i64;
    let end = end.min(n - w).min(foot + samples(EDGE_SPAN_S, srf));
    if foot < 0 || foot + w >= end {
        return foot;
    }
    let pk = f64::from(max_abs(x, foot, end));
    if pk <= 0.0 {
        return foot;
    }
    let need = pk * 10f64.powf(-EDGE_RISE_DB / 20.0);
    let guard = 10f64.powf(-EDGE_GUARD_DB / 20.0);
    let top = pk * 10f64.powf(-EDGE_TOP_DB / 20.0);
    let local = samples(EDGE_LOCAL_S, srf).max(1);
    let step = need / w as f64 / 2.0;
    let at = |i: i64| f64::from(x[i as usize]);
    let noise = {
        let from = (foot - samples(EDGE_NOISE_S, srf)).max(1);
        let mut d: Vec<f64> = (from..foot).map(|i| (at(i) - at(i - 1)).abs()).collect();
        if d.is_empty() {
            0.0
        } else {
            let mid = d.len() / 2;
            *d.select_nth_unstable_by(mid, f64::total_cmp).1
        }
    };
    // the search starts one rise window before the walk's foot: on a steep attack the first sample over the walk's
    // threshold already lies inside the rise, and from there the next window rises too little
    let lo = (foot - w - EDGE_TAIL_SAMPLES).max(1);
    let mut j = (foot - w).max(1);
    while j < end {
        // an edge raises the level away from the line (a falling slope after a first bump is none)
        if at(j + w).abs() - at(j).abs() < need {
            // the level reached the hit's body without an edge that counts: the walk's foot stays
            if at(j).abs() >= top {
                return foot;
            }
            j += 1;
            continue;
        }
        // the rise window starts at `j`; the edge's first rising sample lies inside it. The walk-back follows the
        // rising level `|x|`: it stops at a zero crossing or a bump's top, never climbs a falling slope
        let rise = |m: i64| at(m + 1).abs() - at(m).abs();
        let mut k = (j..j + w).find(|&m| rise(m) >= step).unwrap_or(j);
        let k0 = k;
        while k > lo && rise(k - 1) >= step {
            k -= 1;
        }
        // version 6's tail: up to EDGE_TAIL_SAMPLES more while the level rises by more than EDGE_TAIL_NOISE times the
        // line noise. It may pass the walk's foot (the walk stops at the first sample over its threshold, the foot of
        // the rise lies up to a few samples before it); it takes only samples of the edge's sign (band-limited ringing
        // before a hard start changes sign at every sample, #271) and the last sample on the line, so that the hit sits
        // on the sample before the first rise, as version 5's strict foot did
        let noise_tail = |mut k: i64| {
            let mut tail = 0;
            while k > lo
                && tail < EDGE_TAIL_SAMPLES
                && rise(k - 1) > (EDGE_TAIL_NOISE * noise).max(step / 16.0)
                && (at(k - 1) * at(k) > 0.0 || at(k - 1).abs() <= EDGE_TAIL_NOISE * noise)
            {
                k -= 1;
                tail += 1;
            }
            k
        };
        k = match tail_rule {
            EdgeTail::Noise => noise_tail(k),
            // version 7 (#321): the engineer puts the hit on the last sample of the line, judged by eye against the
            // hit's own size, not against the line noise: back over the samples that stand visibly off the line with
            // the edge's sign while they do not grow towards the line (a slow drift before a snare stroke is taken
            // whole, the invisible first steps of a kick on a silent line are not). The walk counts only when it
            // reaches the line within VIS_TAIL_MAX_S: a tom whose line is a slow wave on the edge's side never reaches
            // it, and there version 6's tail stays (the walk would stop at its bound, an arbitrary place)
            EdgeTail::Visible => {
                let vis = pk * VIS_TAIL_PCT / 100.0;
                let tol = pk * VIS_TAIL_TOL_PCT / 100.0;
                let sgn = at(k0 + 1).signum();
                let lo_v = (k0 - samples(VIS_TAIL_MAX_S, srf)).max(1);
                let mut kv = k;
                while kv > lo_v
                    && at(kv).abs() > vis
                    && at(kv).signum() == sgn
                    && at(kv - 1).abs() <= at(kv).abs() + tol
                {
                    kv -= 1;
                }
                if at(kv).abs() <= vis { kv } else { noise_tail(k) }
            }
        };
        let reached = f64::from(max_abs(x, k, (k + local).min(end + w)));
        let before = if k - w > foot {
            f64::from(max_abs(x, foot, k - w))
        } else {
            0.0
        };
        if before <= reached * guard {
            return k;
        }
        j += w;
    }
    foot
}

/// The walk of [`first_motion_mid`] with its decision (the floor, the threshold, the foot and the foot after the
/// lead-in) for one hit; `None` where the walk does not reach the lead-in (no signal, a busy or separate stretch).
#[must_use]
pub fn first_motion_trace(
    x: &[f32],
    s: i64,
    sr: u32,
    prev_mid: Option<i64>,
    next_mid: Option<i64>,
) -> Option<LeadTrace> {
    first_motion_traced(x, s, sr, prev_mid, next_mid, BackTune::peak(FM_PEAK_DB)).1
}

/// The foot of the walk back from `s` over the samples at or above `thr` (gaps under `gap` do not end the run), and
/// whether the walk ended on a gap before `lo`.
fn walk_foot(x: &[f32], s: i64, lo: i64, thr: f64, gap: i64) -> (i64, bool) {
    let mut i = s;
    let mut foot = s;
    while i > lo {
        i -= 1;
        if two_tap(x, i as usize) >= thr {
            foot = i;
        } else if foot - i >= gap {
            return (foot, true);
        }
    }
    (foot, false)
}

fn first_motion_traced(
    x: &[f32],
    s: i64,
    sr: u32,
    prev_mid: Option<i64>,
    next_mid: Option<i64>,
    bt: BackTune,
) -> ((i64, BackStep), Option<LeadTrace>) {
    let BackTune { peak_db, tune, edge } = bt;
    let srf = f64::from(sr);
    let n = x.len() as i64;
    if s <= 0 || s >= n {
        return ((s, BackStep::Kept), None);
    }
    let mut pe = (s + samples(FM_PEAK_S, srf)).min(n);
    if let Some(m) = next_mid {
        pe = pe.min(m);
    }
    let pk = f64::from(max_abs(x, s, pe));
    if pk < 1e-4 {
        return ((s, BackStep::Kept), None);
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
    let (foot, ended) = walk_foot(x, s, lo, thr, gap);
    if !ended {
        return ((s, BackStep::Busy), None);
    }
    if s - foot > samples(FM_LONG_S, srf) {
        let e = samples(FM_LONG_END_S, srf).max(1);
        let held = f64::from(max_abs(x, foot, (s - e).max(foot + 1)));
        let at = f64::from(max_abs(x, s, s + e));
        if held < at * 10f64.powf(-FM_LONG_DB / 20.0) {
            return ((s, BackStep::Separate), None);
        }
    }
    let walked = foot;
    // The bump decision is judged on the stretch up to the foot of the walk at the base margin, so that every peak
    // margin accepts or rejects the same lead-in (#271).
    let judge = if peak_db == FM_PEAK_DB {
        foot
    } else {
        let thr0 = (pk * 10f64.powf(-FM_PEAK_DB / 20.0)).max(floor * 10f64.powf(FM_FLOOR_DB / 20.0));
        match walk_foot(x, s, lo, thr0, gap) {
            (f0, true) => f0,
            _ => foot,
        }
    };
    let foot = lead_in(x, (foot, judge), lo, floor, thr, gap, srf);
    let trace = LeadTrace {
        floor,
        thr,
        foot: walked,
        lead: foot,
    };
    let foot = match tune {
        None => foot,
        Some(tune) => {
            let end = (s - samples(FM_FLOOR_SKIP_S, srf)).min(n);
            let strict_foot = foot_strict(x, foot, lo, f64::from(max_abs(x, from, end)));
            let cap = match tune.cap {
                FootCap::Seconds(t) => samples(t, srf),
                FootCap::Samples(k) => k,
            };
            if foot - strict_foot <= cap {
                strict_foot
            } else {
                match tune.overrun {
                    Overrun::Keep => foot,
                    Overrun::Clamp => foot - cap,
                }
            }
        }
    };
    // version 6: the first steep edge after the foot; it may lie after `s` (the hit moves later)
    let foot = match edge {
        Some(tail) => edge_foot(x, foot, pe, srf, tail),
        None => foot,
    };
    let step = if foot < s || (edge.is_some() && foot != s) {
        (foot, BackStep::Moved)
    } else {
        (s, BackStep::Kept)
    };
    (step, Some(trace))
}

/// Refinement version 3 (P4-12 stage 2, [`RefineRule::FirstMotion`]): [`refine_reach`], then the
/// back-track [`first_motion`] from its time. `rise` and the reach step are version 2's; a hit
/// moves only earlier, and only where the back-track finds the foot of its attack.
#[must_use]
pub fn refine_first_motion(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow) -> Refined {
    refine_first_motion_peak(x, t0, sr, neighbors, win, FM_PEAK_DB)
}

/// [`refine_first_motion`] with another back-track peak margin (P4-31).
#[must_use]
pub fn refine_first_motion_peak(
    x: &[f32],
    t0: f64,
    sr: u32,
    neighbors: &[f64],
    win: RefineWindow,
    peak_db: f64,
) -> Refined {
    refine_back_peak(x, t0, sr, neighbors, win, BackTune::peak(peak_db))
}

/// [`refine_first_motion_peak`] with the back-track settings `bt`: the strict foot of its `tune` ([`RefineRule::Foot`],
/// [`RefineRule::FootCapped`]) when it is `Some`, and the steep edge after the foot ([`RefineRule::Edge`],
/// [`RefineRule::Visible`]) when its `edge` is set.
fn refine_back_peak(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow, bt: BackTune) -> Refined {
    let r = refine_reach(x, t0, sr, neighbors, win);
    let srf = f64::from(sr);
    let (mut prev, mut next): (Option<f64>, Option<f64>) = (None, None);
    for &nt in neighbors {
        if nt < t0 && prev.is_none_or(|p| nt > p) {
            prev = Some(nt);
        } else if nt > t0 && next.is_none_or(|q| nt < q) {
            next = Some(nt);
        }
    }
    let s = (r.t * srf).round() as i64;
    let (foot, back) = back_track(x, s, sr, t0, (prev, next), bt);
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
#[must_use]
pub fn refine_rule(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow, rule: RefineRule) -> Refined {
    refine_rule_peak(x, t0, sr, neighbors, win, rule, FM_PEAK_DB)
}

/// [`refine_rule`] with the back-track peak margin `peak_db` (it matters for versions 3 to 7, the rules with a back-track).
#[must_use]
pub fn refine_rule_peak(
    x: &[f32],
    t0: f64,
    sr: u32,
    neighbors: &[f64],
    win: RefineWindow,
    rule: RefineRule,
    peak_db: f64,
) -> Refined {
    // the back-track of versions 4 to 7: the strict foot or the steep edge after it
    let back = |tune, edge| refine_back_peak(x, t0, sr, neighbors, win, BackTune { peak_db, tune, edge });
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
        RefineRule::Foot => back(Some(FootTune::V4), None),
        RefineRule::FootCapped => back(Some(FootTune::V5), None),
        RefineRule::Edge => back(None, Some(EdgeTail::Noise)),
        RefineRule::Visible => back(None, Some(EdgeTail::Visible)),
    }
}

/// [`refine_track`] with the rule of the analysis: the previous and the next coarse hit are the
/// neighbours of each one.
#[must_use]
pub fn refine_track_rule(x: &[f32], sr: u32, coarse: &[f64], win: RefineWindow, rule: RefineRule) -> Vec<Refined> {
    refine_track_rule_peak(x, sr, coarse, win, rule, FM_PEAK_DB)
}

/// [`refine_track_rule`] with the back-track peak margin `peak_db` (P4-31).
#[must_use]
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
#[must_use]
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
mod golden_tests;
#[cfg(test)]
mod tests;
