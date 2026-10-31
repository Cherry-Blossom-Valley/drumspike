//! The flam rule (P4-13 stage B): the second stroke of a flam or a close pair becomes a hit.
//!
//! The model sees a flam or two strokes closer than about 30 ms as one lobe and keeps the first stroke (P4-10 stage 1).
//! The rule looks for a second stroke inside the lobe of each output hit, on the hit's own mic:
//!
//! 1. The attack envelope ([`attack_env`]): the mic through a zero-phase high-pass at `hp_hz` (1 kHz, the stick
//!    attack), the peak of `|y|` per block of [`HOP_S`], held over `hold_s` (the block and the ones before it).
//! 2. The first stroke's peak P1: the largest block in `[hit, hit + peak_s]`.
//! 3. A re-rise: the first block c after P1, at least `min_gap_s` (12 ms) after the hit and before `max_gap_s` (50 ms)
//!    and `guard_s` (10 ms) before the next output hit, whose level is `rise_db` (9 dB) or more over the loudest block of
//!    the `look_s` (3 ms) before it (the hold and 0.25 ms excluded), within `level_db` of P1, and `floor_db` over the
//!    floor before the hit (the median block 50 to 2 ms before it). One second stroke per hit.
//! 4. Its onset: the version 3 first-motion walk ([`crate::refine::first_motion_mid`]) from c, bounded by the dip before
//!    it (the lowest block of the 2 ms before c).
//! 5. The bleed gate: for every other mic B of the session the transfer gain `G(B->A)` in the attack band
//!    ([`transfer_gains`], the P2-6 calibration). The re-rise is bleed when B's envelope in `[c - 2 ms, c + 1 ms]` plus
//!    `G` plus `margin_db` (10 dB) reaches the stroke's peak P2. A bleed stroke is not added.
//!
//! The parameters are the stage 1 rule `R9_m10` ([`FlamParams::default`]). The Python reference is `tools/flam_pairs.py`
//! (`attack_env`, `transfer_gains`, `second_onsets`) as `tools/flam_split.py` runs it; the parity check is
//! `tests/test_rust_flam.py` and `tools/flam_split.py parity`. The measurements are in
//! `docs/migration/phase4-report.md`, sections "P4-13 stage 1", "P4-13 stage 2, stage A" and "P4-13 stage B".
//!
//! # Determinism
//!
//! Every index is computed as the reference computes it (Python `int()` truncation and `round()` ties to even,
//! [`crate::pyops`]); block counts use [`HOP_S`] as the reference does, also where the block is not exactly that long
//! (44.1 kHz). The reference filters in the frequency domain (|H|^4 of each stage, 0.25 s of zeros at the edges); this
//! module runs each stage twice forward and twice backward in the time domain, the same response (the filter's
//! response dies out long before the 0.25 s), different only in the rounding of the last bits.

use super::attribution::{Biquad, FilterKind};
use crate::pyops::{py_int, py_round};
use crate::refine::first_motion_mid;

/// The version of the rule an analysis records (`detector.flam_rule`); 0 means no rule.
pub const FLAM_RULE_VERSION: u32 = 1;

/// The block length of the attack envelope (seconds); every block count of the rule is a time over this.
pub const HOP_S: f64 = 0.000125;

/// The parameters of the rule (the stage 1 choice `R9_m10`; `tools/flam_pairs.py` `DEFAULTS`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlamParams {
    /// The high-pass corner of the attack envelope (Hz); 0 = no filter.
    pub hp_hz: f64,
    /// The hold of the envelope (s).
    pub hold_s: f64,
    /// The dip before a re-rise is searched in this many seconds before it.
    pub rise_s: f64,
    /// A re-rise is this many dB over the loudest block of the `look_s` before it.
    pub rise_db: f64,
    /// A re-rise is at most this many dB under the first stroke's peak.
    pub level_db: f64,
    /// A re-rise is at least this many dB over the floor before the hit.
    pub floor_db: f64,
    /// The second stroke lies at least this long after the hit (s) ...
    pub min_gap_s: f64,
    /// ... and before this (s) ...
    pub max_gap_s: f64,
    /// ... and this long before the next output hit (s).
    pub guard_s: f64,
    /// The bleed gate: another mic plus its gain plus this many dB reaches the stroke's peak.
    pub margin_db: f64,
    /// The first stroke's peak lies in this window after the hit (s); also the peak window of the second stroke.
    pub peak_s: f64,
    /// The base of the re-rise: the loudest block of this window before it (s).
    pub look_s: f64,
}

impl Default for FlamParams {
    fn default() -> Self {
        FlamParams {
            hp_hz: 1000.0,
            hold_s: 0.001,
            rise_s: 0.002,
            rise_db: 9.0,
            level_db: 30.0,
            floor_db: 12.0,
            min_gap_s: 0.012,
            max_gap_s: 0.050,
            guard_s: 0.010,
            margin_db: 10.0,
            peak_s: 0.012,
            look_s: 0.003,
        }
    }
}

/// The roles the rule of `version` runs on. Version 1: the snare (P4-13 stage A measured kick and toms: no stroke
/// test for them).
pub fn applies(version: u32, role: &str) -> bool {
    version >= 1 && role == "snare"
}

/// The late-peak gate (P4-13 stage B fix): the rule runs on a track only when its [`late_peak_db`] is under this (dB).
/// A snare top mic decays inside the first 12 ms (-6.1 to -8.8 dB on 9 sessions), a bottom mic's wires peak after it
/// (+0.2 to +6.4 dB): the rule would add the wires' second peak as a stroke there.
pub const LATE_PEAK_MAX_DB: f64 = -3.0;

/// The window of the late peak: from `peak_s` (the first stroke's peak window) to this many seconds after the hit.
pub const LATE_PEAK_END_S: f64 = 0.030;

/// A second stroke the rule found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Second {
    /// The stroke's onset (s).
    pub t: f64,
    /// The hit whose lobe holds it (s).
    pub first: f64,
    /// `t - first` in ms.
    pub gap_ms: f64,
    /// The stroke's peak over the dip before it (dB).
    pub rise_db: f64,
    /// The stroke's peak against the first stroke's peak (dB).
    pub level_db: f64,
    /// The bleed gate: another mic explains it (the rule does not add it).
    pub bleed: bool,
    /// The block of the dip before the stroke's rise (the walk's bound is the block after it).
    pub dip: i64,
}

/// A time in blocks: `int(round(s / HOP_S))`.
fn blocks(s: f64) -> i64 {
    py_round(s / HOP_S)
}

/// The block length in samples at rate `sr`: `max(1, int(round(HOP_S * sr)))`.
pub fn hop_of(sr: u32) -> usize {
    py_round(HOP_S * sr as f64).max(1) as usize
}

/// `x` through the biquad `stages` [(kind, corner Hz)], each twice forward and twice backward: zero phase, the power
/// response `|H|^4` per stage (`tools/onset_guard.py` `zero_phase`). The backward passes start 0.25 s after the end.
pub fn zero_phase(x: &[f32], sr: u32, stages: &[(FilterKind, f64)]) -> Vec<f64> {
    let n = x.len();
    if stages.is_empty() {
        return x.iter().map(|&v| v as f64).collect();
    }
    let pad = py_int(0.25 * sr as f64).max(0) as usize;
    let mut y: Vec<f64> = Vec::with_capacity(n + pad);
    y.extend(x.iter().map(|&v| v as f64));
    y.resize(n + pad, 0.0);
    let run = |y: &mut [f64], q: &Biquad, back: bool| {
        let (mut z0, mut z1) = (0.0f64, 0.0f64);
        let len = y.len();
        for k in 0..len {
            let i = if back { len - 1 - k } else { k };
            let v = y[i];
            let out = q.b[0] * v + z0;
            z0 = q.b[1] * v - q.a[0] * out + z1;
            z1 = q.b[2] * v - q.a[1] * out;
            y[i] = out;
        }
    };
    for &(kind, f) in stages {
        let q = Biquad::new(kind, f, sr);
        run(&mut y, &q, false);
        run(&mut y, &q, false);
        run(&mut y, &q, true);
        run(&mut y, &q, true);
    }
    y.truncate(n);
    y
}

/// The attack envelope of `x` on the block grid: the maximum of `|high-pass(x)|` per block of [`hop_of`] samples, held
/// over `hold_s` (the block and the ones before it). Returns the envelope and the block length in samples.
pub fn attack_env(x: &[f32], sr: u32, p: &FlamParams) -> (Vec<f64>, usize) {
    let y = if p.hp_hz != 0.0 {
        zero_phase(x, sr, &[(FilterKind::HighPass, p.hp_hz)])
    } else {
        x.iter().map(|&v| v as f64).collect()
    };
    let hop = hop_of(sr);
    let nb = y.len() / hop;
    let blk: Vec<f64> = (0..nb)
        .map(|i| y[i * hop..(i + 1) * hop].iter().fold(0.0f64, |m, v| m.max(v.abs())))
        .collect();
    let k = blocks(p.hold_s).max(1) as usize;
    let env = (0..nb)
        .map(|i| blk[(i + 1).saturating_sub(k)..=i].iter().fold(0.0f64, |m, &v| m.max(v)))
        .collect();
    (env, hop)
}

/// The late-peak gate's second test: the share `W` of the hits whose late peak beats their first peak must be this or more
/// for a skip. A monotone decay (a ringy top mic) has a late peak under its first one on every hit; wires peak after it.
/// Real top mics measured W 0.00 to 0.23, bottom mics 0.52 to 0.94, the DRT single-snare files 0.06 to 0.12.
pub const LATE_SHARE_MIN: f64 = 0.4;

/// Per hit (s) with a full 30 ms window and a first peak: the largest attack-envelope block in `[hit + peak_s, hit +
/// 30 ms)` over the largest in `[hit, hit + peak_s)` (a ratio). `env`: the [`attack_env`]; `sr`, `hop`: rate and block.
fn late_ratios(env: &[f64], hits: &[f64], sr: u32, hop: usize, p: &FlamParams) -> Vec<f64> {
    let r = sr as f64 / hop as f64;
    let (f, e) = (py_int(p.peak_s * r).max(1), py_int(LATE_PEAK_END_S * r));
    hits.iter()
        .filter_map(|&t| {
            let b = py_int(t * sr as f64).div_euclid(hop as i64);
            if b < 0 || b + e >= env.len() as i64 {
                return None;
            }
            let p1 = max_of(&env[b as usize..(b + f) as usize]);
            if p1 <= 0.0 {
                return None;
            }
            Some(max_of(&env[(b + f) as usize..(b + e) as usize]) / p1)
        })
        .collect()
}

/// The late peak `A` of a mic: the median over `hits` of the [`late_ratios`] in dB (`tools/flam_pairs.py`
/// `late_peak_db`); `None` without a value.
pub fn late_peak_db(env: &[f64], hits: &[f64], sr: u32, hop: usize, p: &FlamParams) -> Option<f64> {
    let v: Vec<f64> = late_ratios(env, hits, sr, hop, p)
        .iter()
        .map(|r| 20.0 * r.log10())
        .collect();
    (!v.is_empty()).then(|| median(&v))
}

/// The late share `W` of a mic: the share of the same hits whose late peak beats the first (ratio over 1);
/// `None` without a value (`tools/flam_pairs.py` `late_share`).
pub fn late_share(env: &[f64], hits: &[f64], sr: u32, hop: usize, p: &FlamParams) -> Option<f64> {
    let v = late_ratios(env, hits, sr, hop, p);
    (!v.is_empty()).then(|| v.iter().filter(|&&r| r > 1.0).count() as f64 / v.len() as f64)
}

/// True for a late-peaking mic (a snare bottom): the late peak `A` is [`LATE_PEAK_MAX_DB`] or more AND the share `W`
/// is [`LATE_SHARE_MIN`] or more. `A` alone measures the decay: a ringy undamped top mic has `A` over -3 dB and `W` near 0.
pub fn late_peaking(late_db: Option<f64>, share: Option<f64>) -> bool {
    late_db.is_some_and(|v| v >= LATE_PEAK_MAX_DB) && share.is_some_and(|w| w >= LATE_SHARE_MIN)
}

/// `np.percentile(v, q)` with numpy's default linear method, bit for bit: the virtual index `(n - 1) * q / 100` and
/// numpy's `_lerp` (from the upper value when the fraction is 0.5 or more). `None` for an empty slice.
pub fn np_percentile(v: &[f64], q: f64) -> Option<f64> {
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

/// `np.median` of a non-empty slice.
fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// The index of the first maximum (`np.argmax`) of a non-empty slice.
fn argmax(v: &[f64]) -> usize {
    let mut best = 0;
    for (i, &x) in v.iter().enumerate() {
        if x > v[best] {
            best = i;
        }
    }
    best
}

/// The index of the first minimum (`np.argmin`) of a non-empty slice.
fn argmin(v: &[f64]) -> usize {
    let mut best = 0;
    for (i, &x) in v.iter().enumerate() {
        if x < v[best] {
            best = i;
        }
    }
    best
}

fn max_of(v: &[f64]) -> f64 {
    v.iter().fold(f64::NEG_INFINITY, |m, &x| m.max(x))
}

/// The strong hits of a mic B (`tools/onset_guard.py` `strong_peaks`): per block of 50 ms of its envelope (`rate`
/// blocks per second) the largest block, kept when within 15 dB of the 99.5th percentile of those maxima.
pub fn strong_peaks(env: &[f64], rate: f64) -> Vec<usize> {
    let b = py_int(0.050 * rate).max(1) as usize;
    let nb = env.len() / b;
    if nb == 0 {
        return Vec::new();
    }
    let (arg, val): (Vec<usize>, Vec<f64>) = (0..nb)
        .map(|i| {
            let blk = &env[i * b..(i + 1) * b];
            let a = argmax(blk);
            (i * b + a, blk[a])
        })
        .unzip();
    let r = np_percentile(&val, 99.5).unwrap_or(0.0);
    let lim = r * 10f64.powf(-15.0 / 20.0);
    arg.into_iter()
        .zip(val)
        .filter(|&(_, v)| v >= lim)
        .map(|(a, _)| a)
        .collect()
}

/// `G(B->A)` in dB (`tools/onset_guard.py` `calibrate_gain`, P2-6): at B's strong blocks `peaks` the level of A's
/// envelope minus B's in `[-0.5, +8)` ms; the 10th percentile over those where A has no own hit (`own`, sorted
/// seconds) within 30 ms when there are 5 of them, else over all; `None` with fewer than 5.
pub fn calibrate(env_a: &[f64], env_b: &[f64], peaks: &[usize], own: &[f64], rate: f64) -> Option<f64> {
    const MIN_HITS: usize = 5;
    if peaks.len() < MIN_HITS {
        return None;
    }
    let (w0, w1) = (py_int(-0.0005 * rate), py_int(0.008 * rate));
    let mut vals = Vec::new();
    let mut quiet = Vec::new();
    for &p in peaks {
        let p = p as i64;
        let a = (p + w0).max(0) as usize;
        let b = ((p + w1).max(0) as usize).min(env_a.len());
        if b <= a {
            continue;
        }
        let la = 20.0 * (max_of(&env_a[a..b]) + 1e-9).log10();
        let lb = 20.0 * (max_of(&env_b[a..b]) + 1e-9).log10();
        if lb < -100.0 {
            continue;
        }
        vals.push(la - lb);
        let t = p as f64 / rate;
        let k = own.partition_point(|&o| o < t);
        let near = [k.wrapping_sub(1), k]
            .iter()
            .any(|&j| j < own.len() && (own[j] - t).abs() <= 0.030);
        if !near {
            quiet.push(la - lb);
        }
    }
    let used = if quiet.len() >= MIN_HITS { &quiet } else { &vals };
    if used.len() >= MIN_HITS {
        np_percentile(used, 10.0)
    } else {
        None
    }
}

/// The transfer gains of the other mics' envelopes `sources` into the own envelope `env_a` (`tools/flam_pairs.py`
/// `transfer_gains`): per source its gain, `None` without one (fewer than 5 strong hits). `own`: the own output hits
/// (s); `sr`, `hop`: the rate and the block length. A source is compared over the common length.
pub fn transfer_gains(env_a: &[f64], sources: &[&[f64]], own: &[f64], sr: u32, hop: usize) -> Vec<Option<f64>> {
    let rate = sr as f64 / hop as f64;
    let mut own: Vec<f64> = own.to_vec();
    own.sort_by(f64::total_cmp);
    sources
        .iter()
        .map(|env_b| {
            let n = env_a.len().min(env_b.len());
            let peaks = strong_peaks(&env_b[..n], rate);
            calibrate(&env_a[..n], &env_b[..n], &peaks, &own, rate)
        })
        .collect()
}

/// The second strokes inside the lobes of the output hits `hits` (s) of the own mic `x` (`tools/flam_pairs.py`
/// `second_onsets`). `env`: the own [`attack_env`]; `gains`: the sources' envelopes with their gains (sources without
/// a gain left out). One [`Second`] per re-rise, the bleed ones included.
pub fn second_onsets(
    x: &[f32],
    sr: u32,
    hits: &[f64],
    p: &FlamParams,
    env: &[f64],
    gains: &[(&[f64], f64)],
) -> Vec<Second> {
    let hop = hop_of(sr) as i64;
    let srf = sr as f64;
    let mut times: Vec<f64> = hits.to_vec();
    times.sort_by(f64::total_cmp);
    let nb = env.len() as i64;
    let (rise_b, peak_b, min_b, max_b, guard_b) = (
        blocks(p.rise_s),
        blocks(p.peak_s),
        blocks(p.min_gap_s),
        blocks(p.max_gap_s),
        blocks(p.guard_s),
    );
    let (g_b, l_b) = (blocks(p.hold_s + 0.00025), blocks(p.look_s));
    let (f50, f2, b2, b1) = (blocks(0.050), blocks(0.002), blocks(0.002), blocks(0.001));
    let rise_k = 10f64.powf(p.rise_db / 20.0);
    let level_k = 10f64.powf(-p.level_db / 20.0);
    let floor_k = 10f64.powf(p.floor_db / 20.0);
    let gate_k: Vec<f64> = gains
        .iter()
        .map(|&(_, g)| 10f64.powf((g + p.margin_db) / 20.0))
        .collect();
    let mut out = Vec::new();
    for (k, &t) in times.iter().enumerate() {
        let b0 = py_int(t * srf).div_euclid(hop);
        let mut end = (b0 + max_b).min(nb);
        if let Some(&nx) = times.get(k + 1) {
            end = end.min(py_int(nx * srf).div_euclid(hop) - guard_b);
        }
        if end - b0 <= min_b.max(2) || b0 < 0 {
            continue;
        }
        let pk_end = ((b0 + peak_b).min(end) + 1).min(nb);
        if pk_end <= b0 {
            continue;
        }
        let ipk = b0 + argmax(&env[b0 as usize..pk_end as usize]) as i64;
        let p1 = env[ipk as usize];
        if p1 <= 0.0 {
            continue;
        }
        let (f0, f1) = ((b0 - f50).max(0), (b0 - f2).max(0));
        let floor = if f1 - f0 >= 8 {
            median(&env[f0 as usize..f1 as usize])
        } else {
            0.0
        };
        let start = (ipk + 1).max(b0 + min_b);
        if start >= end {
            continue;
        }
        let found = (start..end).find(|&i| {
            let lo = env[(i - g_b - l_b).max(0) as usize..(i - g_b + 1).max(0) as usize]
                .iter()
                .fold(f64::NEG_INFINITY, |m, &v| m.max(v));
            let v = env[i as usize];
            v >= lo * rise_k && v >= p1 * level_k && v >= floor * floor_k
        });
        let Some(c) = found else { continue };
        let d0 = ipk.max(c - rise_b);
        let d = d0 + argmin(&env[d0 as usize..c as usize]) as i64;
        let p2 = max_of(&env[c as usize..(c + peak_b).min(nb) as usize]);
        let sc = c * hop;
        let (foot, _) = first_motion_mid(x, sc, sr, Some((d + 1) * hop), None);
        // the walk never passes its bound, the block after the dip (an old clamp to one block before it was dead)
        debug_assert!(foot >= (d + 1) * hop, "the walk stops at the dip");
        let bleed = gains.iter().zip(&gate_k).any(|(&(env_b, _), &gk)| {
            let lo_b = (c - b2).max(0);
            let hi_b = (c + b1 + 1).min(env_b.len() as i64);
            hi_b > lo_b && max_of(&env_b[lo_b as usize..hi_b as usize]) * gk >= p2
        });
        let ts = foot as f64 / srf;
        out.push(Second {
            t: ts,
            first: t,
            gap_ms: (ts - t) * 1000.0,
            rise_db: 20.0 * (p2 / env[d as usize].max(1e-12)).log10(),
            level_db: 20.0 * (p2 / p1).log10(),
            bleed,
            dip: d,
        });
    }
    out
}

/// The rule on one mic: [`attack_env`] of `x` and of every source (a source of another length is compared over the
/// common length), [`transfer_gains`], [`second_onsets`]. Returns the strokes (bleed ones included) and per source
/// its gain.
pub fn flam_split(
    x: &[f32],
    sr: u32,
    hits: &[f64],
    sources: &[&[f32]],
    p: &FlamParams,
) -> (Vec<Second>, Vec<Option<f64>>) {
    let (env, hop) = attack_env(x, sr, p);
    let envs: Vec<Vec<f64>> = sources.iter().map(|s| attack_env(s, sr, p).0).collect();
    let refs: Vec<&[f64]> = envs.iter().map(Vec::as_slice).collect();
    let gains = transfer_gains(&env, &refs, hits, sr, hop);
    let used: Vec<(&[f64], f64)> = refs
        .iter()
        .zip(&gains)
        .filter_map(|(e, g)| g.map(|g| (&e[..e.len().min(env.len())], g)))
        .collect();
    (second_onsets(x, sr, hits, p, &env, &used), gains)
}

/// The times of the strokes the rule adds: those the bleed gate leaves.
pub fn added(found: &[Second]) -> Vec<f64> {
    found.iter().filter(|s| !s.bleed).map(|s| s.t).collect()
}

#[cfg(test)]
mod tests;
