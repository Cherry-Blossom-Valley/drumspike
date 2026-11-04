//! Refinement version 2, the reach rule ([`RefineRule::Reach`](super::RefineRule::Reach)): the reference with the reach (M3) and the tail
//! fix (C1).

use super::reference::{Detail, RISE_S, RULE_POST_S, RefineWindow, RuleState, refine_detail};
use super::walk::BackStep;
use super::{Refined, max_abs};
use crate::pyops::py_int;

/// How far after the coarse time the reach rule looks for the attack (seconds). The DSP
/// detector gives the frame before the attack, 6 to 15 ms early (P4-11b verifier notes), and
/// the reference's rule window ends 10 ms after its anchor.
pub(crate) const REACH_S: f64 = 0.020;
/// The stretch before the rule window whose maximum is the floor of the reach test (seconds).
pub(crate) const FLOOR_S: f64 = 0.010;
/// The rule window holds no attack when the reach after it is at least this many times louder
/// than the window's maximum ...
pub(crate) const REACH_GAIN: f32 = 2.0;
/// ... and the window's maximum (without its last [`EDGE_S`]) is at most this many times the
/// floor before it (about 10 dB: a real floor varies by 6 to 7 dB between two 10 ms
/// stretches), or the reference gave the coarse time back, ...
pub(crate) const FLOOR_RATIO: f32 = 3.0;
/// ... or the window's maximum is this many times (about 30 dB) under the attack that follows:
/// no first motion of that hit. A quieter event 12 to 20 dB under the hit (a grace note,
/// bleed) stays the hit's onset.
pub(crate) const DEEP_RATIO: f32 = 30.0;
/// The end of the rule window that the floor test leaves out (seconds): an attack that starts
/// there is cut by the window, and its anchor was a ripple of the floor.
pub(crate) const EDGE_S: f64 = 0.001;
/// The tail test: the stretch before the rule window that must already hold the 25 % level
/// for a crossing before the anchor to count as the previous hit's tail (seconds), ...
pub const TAIL_S: f64 = 0.005;
/// ... and the previous coarse hit lies at most this far (seconds) before the rule window: the
/// tail belongs to an earlier hit (sixteenths at 100 BPM), not to the hit's own body ...
pub(crate) const TAIL_PREV_S: f64 = 0.150;
/// ... and does not rise there: the envelope's mean over its second half is at most this much
/// (dB) above the first half's (a snare tail decays by under 1 dB in 5 ms, a slow attack rises
/// by several dB), ...
pub(crate) const TAIL_RISE_DB: f64 = 3.0;
/// ... and the envelope falls by at least this much (dB) from the crossing to its lowest
/// point before the steepest rise (the trough between the tail and the attack).
pub(crate) const TROUGH_DB: f64 = 0.5;
/// A reference crossing less than this before the foot of the attack the reach found (seconds)
/// is that attack's start, cut by the rule window's end: the reference stands.
pub(crate) const REACH_GAP_S: f64 = 0.002;
/// The foot of that attack: walking back from its start while the maximum of the last this
/// many seconds stays above the floor's maximum.
pub(crate) const FOOT_S: f64 = 0.00025;

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

/// Refinement version 2 (P4-12 stage 1, [`RefineRule::Reach`](super::RefineRule::Reach)): the reference [`refine_sr`](super::refine_sr),
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
pub(crate) fn refine_reach(x: &[f32], t0: f64, sr: u32, neighbors: &[f64], win: RefineWindow) -> Refined {
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
