//! Invariance tests for detection and placement (#345): transformations of the input
//! with a known effect on the output, checked sample for sample, without ground truth.
//! They run on synthetic kits (no audio files) through the public analysis path,
//! `analyze_track` with [`Detector::Dsp`] and the `CURRENT` refine rule, at 48 kHz,
//! for the kick, snare, tom and hat cases. A case the current code violates is a
//! finding, not a tuning job: such a test is marked `#[ignore = "finding: ..."]` and
//! the measured numbers are reported on the ticket.
//!
//! Each case prints one table row (case, hits, max move in samples, verdict).
//!
//! The synthetic renderer is shared with `robustness.rs` through `tests/common`.
//!
//! Run: `cargo test -p drumspike-core --test invariance -j 8 -- --nocapture`

use std::sync::OnceLock;

use drumspike_core::analyze::{Analysis, AnalyzeParams, Detector, analyze_track};

mod common;

use common::{Voice, render_hits};

const SR: u32 = 48_000;
/// Baseline peak level, low enough that +6 dB stays clear of full scale.
const PEAK_DB: f64 = -12.0;
/// The phrase length; the last hit at 2.0 s plus its decay fits inside.
const TOTAL_S: f64 = 3.2;
/// The roles and their phrase: hit times (s) and voice. Hits far above the 20 ms
/// min gap, all four voices well detectable.
const ROLES: [(&str, &[f64], Voice); 4] = [
    ("kick", &[0.2, 0.8, 1.4, 2.0], Voice::AcousticKick),
    ("snare", &[0.2, 0.8, 1.4, 2.0], Voice::AcousticSnare),
    ("tom", &[0.2, 0.8, 1.4, 2.0], Voice::AcousticTom),
    ("hat", &[0.2, 0.8, 1.4, 2.0], Voice::Hat),
];

/// The role's index in `ROLES`.
fn role_index(role: &str) -> usize {
    ROLES.iter().position(|(r, _, _)| *r == role).unwrap()
}

/// The role's untransformed phrase signal, rendered once and reused across the cases.
fn base_signal(role: &str) -> &'static Vec<f32> {
    static BASES: OnceLock<Vec<Vec<f32>>> = OnceLock::new();
    let bases = BASES.get_or_init(|| {
        ROLES
            .iter()
            .map(|&(_, times, voice)| {
                let hits: Vec<(f64, Voice)> = times.iter().map(|&t| (t, voice)).collect();
                render_hits(SR, &hits, TOTAL_S, PEAK_DB)
            })
            .collect()
    });
    &bases[role_index(role)]
}

/// Run the analysis path for one signal.
fn analyze(role: &str, x: &[f32]) -> Analysis {
    analyze_track(x, SR, role, &AnalyzeParams::default(), Detector::Dsp).expect("analyze_track must not fail")
}

/// The hit times of an analysis, in whole samples.
fn hit_samples(a: &Analysis) -> Vec<f64> {
    a.events.iter().map(|h| (h.t * f64::from(SR)).round()).collect()
}

/// Pair two hit lists by time order and return the largest move in samples.
/// Fails with the case name when the hit count differs.
fn max_move(case: &str, base: &[f64], got: &[f64]) -> f64 {
    assert_eq!(
        base.len(),
        got.len(),
        "{case}: hit count changed: {} found where {} hits exist",
        got.len(),
        base.len()
    );
    base.iter().zip(got).map(|(&b, &g)| (g - b).abs()).fold(0.0, f64::max)
}

/// The standing check of one case: same hit count, every hit moved at most
/// `max_move_allowed` samples, one printed table row, and the measured max move
/// returned.
fn check_stable(case: &str, base: &[f64], got: &[f64], max_move_allowed: f64) -> f64 {
    let m = max_move(case, base, got);
    assert!(
        m <= max_move_allowed,
        "{case}: a hit moved {m} samples, over the {max_move_allowed} sample allowance"
    );
    println!("| {case} | {} | {m:.0} | PASS |", base.len());
    m
}

/// The velocity order of an analysis, in time order.
fn velocity_order(a: &Analysis) -> Vec<i64> {
    a.events.iter().map(|h| h.v).collect()
}

/// Two velocity sequences keep the same order (the ranks in time order agree).
fn assert_velocity_order(case: &str, base: &[i64], got: &[i64]) {
    assert_eq!(base.len(), got.len(), "{case}: velocity count changed");
    let rank = |v: &[i64]| -> Vec<usize> {
        let mut idx: Vec<usize> = (0..v.len()).collect();
        idx.sort_by_key(|&i| v[i]);
        let mut r = vec![0usize; v.len()];
        for (pos, i) in idx.iter().enumerate() {
            r[*i] = pos;
        }
        r
    };
    assert_eq!(rank(base), rank(got), "{case}: the velocity order changed");
}

// --- the cases (one helper per case, called by one test per role) ----------

/// Shift: the signal delayed by n samples (n = 1, 7, 480, zeros in front) gives the
/// same hit count and every hit exactly n samples later.
fn case_shift(role: &str) {
    let base_hits = hit_samples(&analyze(role, base_signal(role)));
    for &n in &[1usize, 7, 480] {
        let mut x = vec![0.0f32; n];
        x.extend_from_slice(base_signal(role));
        let got = hit_samples(&analyze(role, &x));
        let expected: Vec<f64> = base_hits.iter().map(|&s| s + n as f64).collect();
        check_stable(&format!("shift {n}/{role}"), &expected, &got, 0.0);
    }
}

/// Gain: the signal at -6 dB and at +6 dB (no clipping) gives the same hits sample
/// for sample; velocities do not change order.
fn case_gain(role: &str) {
    let base = analyze(role, base_signal(role));
    let base_hits = hit_samples(&base);
    let base_vels = velocity_order(&base);
    for (name, g) in [("-6 dB", 0.5f32), ("+6 dB", 2.0f32)] {
        let x: Vec<f32> = base_signal(role).iter().map(|&v| v * g).collect();
        let got = analyze(role, &x);
        check_stable(&format!("gain {name}/{role}"), &base_hits, &hit_samples(&got), 0.0);
        assert_velocity_order(&format!("gain {name}/{role}"), &base_vels, &velocity_order(&got));
    }
}

/// Noise floor: white noise at 40 dB under the softest hit's peak added to the signal
/// gives the same hit count and no hit moved by more than 1 sample.
fn case_noise_floor(role: &str) {
    let base = analyze(role, base_signal(role));
    let base_hits = hit_samples(&base);
    let x = base_signal(role);
    // The softest hit's peak: the largest sample in a 0.4 s window at each hit.
    let softest = ROLES[role_index(role)]
        .1
        .iter()
        .map(|&t| {
            let i0 = (t * f64::from(SR)) as usize;
            let i1 = ((t + 0.4) * f64::from(SR)) as usize;
            x[i0..i1.min(x.len())].iter().fold(0.0f32, |m, &v| m.max(v.abs()))
        })
        .fold(f32::INFINITY, f32::min);
    let noise_peak = softest * 10f32.powf(-40.0 / 20.0);
    let mut rng = common::Rng(0xfeed_beef);
    let mut noisy = x.clone();
    for v in &mut noisy {
        *v += noise_peak * (2.0 * rng.next_f32() - 1.0) as f32;
    }
    let got = analyze(role, &noisy);
    check_stable(&format!("noise floor/{role}"), &base_hits, &hit_samples(&got), 1.0);
}

/// Gap: 2 s of silence inserted between two hits gives the hits before unchanged and
/// the hits after exactly 96 000 samples later.
fn case_gap(role: &str) {
    const SPLIT_S: f64 = 1.1; // between the second hit (0.8 s) and the third (1.4 s)
    const INSERT: usize = 2 * SR as usize; // 96 000 samples at 48 kHz
    let times = ROLES[role_index(role)].1;
    let voice = ROLES[role_index(role)].2;
    let part1: Vec<(f64, Voice)> = times.iter().filter(|&&t| t < SPLIT_S).map(|&t| (t, voice)).collect();
    let part2: Vec<(f64, Voice)> = times
        .iter()
        .filter(|&&t| t >= SPLIT_S)
        .map(|&t| (t - SPLIT_S, voice))
        .collect();
    let p1 = render_hits(SR, &part1, SPLIT_S, PEAK_DB);
    let p2 = render_hits(SR, &part2, TOTAL_S - SPLIT_S, PEAK_DB);
    let mut base = p1.clone();
    base.extend_from_slice(&p2);
    let mut gapped = p1;
    gapped.extend(std::iter::repeat_n(0.0f32, INSERT));
    gapped.extend_from_slice(&p2);
    let base_hits = hit_samples(&analyze(role, &base));
    let got = hit_samples(&analyze(role, &gapped));
    let split_samples = SPLIT_S * f64::from(SR);
    let (before, after): (Vec<f64>, Vec<f64>) = base_hits.iter().copied().partition(|&s| s < split_samples);
    let m_before = max_move(&format!("gap/{role} before"), &before, &got[..before.len()]);
    assert!(
        m_before <= 0.0,
        "gap/{role}: a hit before the gap moved {m_before} samples"
    );
    let expected_after: Vec<f64> = after.iter().map(|&s| s + INSERT as f64).collect();
    let m_after = max_move(&format!("gap/{role} after"), &expected_after, &got[before.len()..]);
    assert!(
        m_after <= 0.0,
        "gap/{role}: a hit after the gap moved {m_after} samples instead of exactly {INSERT}"
    );
    println!("| gap/{role} | {} | 0 | PASS |", base_hits.len());
}

/// Polarity: the inverted signal gives the same hit count and no hit moved by more
/// than 1 sample.
fn case_polarity(role: &str) {
    let base_hits = hit_samples(&analyze(role, base_signal(role)));
    let x: Vec<f32> = base_signal(role).iter().map(|&v| -v).collect();
    let got = hit_samples(&analyze(role, &x));
    check_stable(&format!("polarity/{role}"), &base_hits, &got, 1.0);
}

/// Trailing content: appending 5 s of a different synthetic phrase changes no hit of
/// the first part.
fn case_trailing(role: &str, other_role: &str) {
    let (times, voice) = {
        let r = ROLES[role_index(role)];
        (r.1, r.2)
    };
    let other_voice = ROLES[role_index(other_role)].2;
    // One render for both: the prefix of the combined render is the untouched
    // baseline, so the appended phrase cannot change the first part's samples.
    let mut hits: Vec<(f64, Voice)> = times.iter().map(|&t| (t, voice)).collect();
    for &t in times {
        hits.push((t + 5.0, other_voice));
    }
    let combined = render_hits(SR, &hits, TOTAL_S + 5.0 + 1.0, PEAK_DB);
    let base_len = (TOTAL_S * f64::from(SR)) as usize;
    let base_hits = hit_samples(&analyze(role, &combined[..base_len]));
    let got_all = hit_samples(&analyze(role, &combined));
    // Only the first part's hits are under test; hits in the appended phrase are the
    // trailing content itself.
    let got: Vec<f64> = got_all.iter().copied().take_while(|&s| s < base_len as f64).collect();
    check_stable(&format!("trailing {other_role}/{role}"), &base_hits, &got, 1.0);
}

// --- one test per case and role --------------------------------------------

#[test]
fn shift_kick() {
    case_shift("kick");
}

#[test]
fn shift_snare() {
    case_shift("snare");
}

#[test]
fn shift_tom() {
    case_shift("tom");
}

#[test]
fn shift_hat() {
    case_shift("hat");
}

#[test]
fn gain_kick() {
    case_gain("kick");
}

#[test]
fn gain_snare() {
    case_gain("snare");
}

/// Finding (measured 2026-11-05, main 8d29b420): at +6 dB the tom yields 8 hits where
/// 4 exist — the detector adds an onset about 24 000 samples (0.5 s) after each hit
/// (base 9615, 38418, 67216, 96015; got adds 33595, 62396, 91195, 119996). At -6 dB
/// the same signal is exact. No clipping (peak -6 dBFS).
#[test]
#[ignore = "finding: at +6 dB the tom yields 8 hits where 4 exist (an extra onset ~24 000 samples after each hit)"]
fn gain_tom() {
    case_gain("tom");
}

#[test]
fn gain_hat() {
    case_gain("hat");
}

#[test]
fn noise_floor_kick() {
    case_noise_floor("kick");
}

#[test]
fn noise_floor_snare() {
    case_noise_floor("snare");
}

#[test]
fn noise_floor_tom() {
    case_noise_floor("tom");
}

#[test]
fn noise_floor_hat() {
    case_noise_floor("hat");
}

#[test]
fn gap_kick() {
    case_gap("kick");
}

#[test]
fn gap_snare() {
    case_gap("snare");
}

#[test]
fn gap_tom() {
    case_gap("tom");
}

#[test]
fn gap_hat() {
    case_gap("hat");
}

#[test]
fn polarity_kick() {
    case_polarity("kick");
}

#[test]
fn polarity_snare() {
    case_polarity("snare");
}

#[test]
fn polarity_tom() {
    case_polarity("tom");
}

#[test]
fn polarity_hat() {
    case_polarity("hat");
}

#[test]
fn trailing_kick() {
    case_trailing("kick", "snare");
}

/// Finding (measured 2026-11-05, main 8d29b420): appending 5 s of tom changes the
/// snare's first part — the hit at sample 9612 disappears (base 9612, 38416, 67214,
/// 96015; got 38416, 67214, 96015). The two prefixes are sample-identical, so the
/// detector's thresholds depend on the whole signal, not the local content.
#[test]
#[ignore = "finding: appending 5 s of tom content removes the snare's first hit (sample 9612) from an unchanged prefix"]
fn trailing_snare() {
    case_trailing("snare", "tom");
}

#[test]
fn trailing_tom() {
    case_trailing("tom", "hat");
}

#[test]
fn trailing_hat() {
    case_trailing("hat", "kick");
}

// --- the planted-failure tests ---------------------------------------------
// Every assertion helper has its own unit test that proves it can fail: a planted
// wrong expectation fails.

#[test]
#[should_panic(expected = "hit count changed")]
fn max_move_fails_on_a_planted_count_change() {
    let base = [100.0, 500.0, 900.0];
    let got = [100.0, 900.0];
    let _ = max_move("planted", &base, &got);
}

#[test]
#[should_panic(expected = "moved 5 samples, over the 1")]
fn check_stable_fails_on_a_planted_move() {
    let base = [100.0, 500.0, 900.0];
    let mut got = base;
    got[1] += 5.0;
    check_stable("planted", &base, &got, 1.0);
}

#[test]
#[should_panic(expected = "the velocity order changed")]
fn assert_velocity_order_fails_on_a_planted_order_change() {
    assert_velocity_order("planted", &[80, 100, 60], &[80, 60, 100]);
}
