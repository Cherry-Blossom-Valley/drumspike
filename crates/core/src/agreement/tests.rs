//! Tests of [`crate::agreement`]: the synthetic-kit checks of #352 (sample-exact ground
//! truth, planted inverted hits, the 24-sample clean-spread bar, the monotone check)
//! plus unit tests of the three estimators.

use super::*;

/// A synthetic one-shot: sharp attack at sample 0, exponential decay, tone `f`.
/// The first non-zero sample is the onset (sample-exact ground truth).
fn shot(n: usize, sr: u32, f: f64, level: f64, polarity: i32) -> Vec<f32> {
    let dt = 1.0 / sr as f64;
    (0..n)
        .map(|i| {
            let t = i as f64 * dt;
            let env = (1.0 - (-t / 0.002).exp()) * (-t / 0.080).exp();
            let v: f64 = (polarity as f64) * level * env * (2.0 * std::f64::consts::PI * f * t).sin();
            v as f32
        })
        .collect()
}

/// A track of planted one-shots at exact sample positions.
fn kit(sr: u32, dur_s: f64, f: f64, events: &[(i64, i32)]) -> (Vec<f32>, Vec<f64>) {
    let n = (dur_s * sr as f64) as usize;
    let mut x = vec![0.0f32; n];
    for &(s, pol) in events {
        let sh = shot((0.4 * sr as f64) as usize, sr, f, 0.5, pol);
        for (i, v) in sh.iter().enumerate() {
            x[s as usize + i] += v;
        }
    }
    let hits = events.iter().map(|&(s, _)| s as f64 / sr as f64).collect();
    (x, hits)
}

/// A clean kit: isolated one-shots far enough apart that no residue crosses the next
/// hit's search window (the shots decay for 80 ms, so the spacing must exceed that).
/// Returns the signal, the hit times and the sample-exact ground truth.
fn clean_kit(sr: u32, f: f64, n: u32, pol_at: Option<usize>) -> (Vec<f32>, Vec<f64>, Vec<i64>) {
    let events: Vec<(i64, i32)> = (0u32..n)
        .map(|i| {
            let s = 4800 + i as i64 * sr as i64 * 2 / 5;
            (s, if Some(i as usize) == pol_at { -1 } else { 1 })
        })
        .collect();
    let (x, hits) = kit(sr, (n as f64) * 0.4 + 1.0, f, &events);
    let trues = events.iter().map(|&(s, _)| s).collect();
    (x, hits, trues)
}

fn sorted_spreads(ag: &[Agreement]) -> Vec<f64> {
    let mut v: Vec<f64> = ag.iter().filter_map(|a| a.timing_confidence).collect();
    v.sort_by(|a, b| a.total_cmp(b));
    v
}

fn median_of(v: &[f64]) -> f64 {
    v[v.len() / 2]
}

#[test]
fn aic_finds_a_step() {
    let x: Vec<f32> = (0..200).map(|i| if i < 100 { 0.0 } else { 0.5 }).collect();
    assert_eq!(aic_change_point(&x, 0, 200), Some(100));
    // too short a window: none
    assert_eq!(aic_change_point(&x, 0, 4), None);
}

#[test]
fn aic_change_point_at_the_window_edge_is_none() {
    // the change sits at the very end of the window: the minimum has nowhere to go and
    // the estimate is not used
    let mut x = vec![0.0f32; 200];
    x[197] = 0.5;
    x[198] = 0.4;
    x[199] = 0.3;
    assert_eq!(aic_change_point(&x, 0, 200), None);
    // the same change with noise on both sides is a real estimate near the truth
    let mut y = vec![0.0f32; 200];
    for (i, v) in y.iter_mut().enumerate() {
        let n = ((i as f64 * 12.9898).sin() * 43758.5453).fract() - 0.5;
        *v = (0.01 * n) as f32;
        if i >= 100 {
            *v += (0.5f64 * (-((i - 100) as f64) / 240.0).exp()) as f32;
        }
    }
    let cp = aic_change_point(&y, 0, 200);
    assert!(cp.is_some());
    assert!((cp.unwrap() - 100).abs() <= 4, "change point {} off", cp.unwrap());
}

#[test]
fn env_first_motion_finds_the_foot() {
    // a linear rise from 0: the foot is the first sample at the peak margin (33 dB under
    // the peak), well inside the window
    let x: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
    assert_eq!(env_first_motion(&x, 0, 100), Some(3));
    // silent window: none
    let z = vec![0.0f32; 100];
    assert_eq!(env_first_motion(&z, 0, 100), None);
    // motion at the very start of the window: the onset lies before the window
    let r: Vec<f32> = (0..100).map(|i| 0.5 + i as f32 / 100.0).collect();
    assert_eq!(env_first_motion(&r, 0, 100), None);
}

#[test]
fn xcorr_lag_at_the_search_edge_is_not_an_estimate() {
    let sr = 48_000u32;
    // the template: a shot with its onset 2 ms in (the pre-window)
    let tpl: Vec<f64> = shot(720, sr, 170.0, 0.5, 1).iter().map(|&v| v as f64).collect();
    let span = 240i64;
    let sh = shot(9600, sr, 170.0, 0.5, 1);
    // the signal holds the same shot with its onset exactly at the far edge of the
    // search: the best lag is the edge and is flagged as unusable
    let mut x = vec![0.0f32; 48000];
    let onset = (96 + span) as usize;
    for (i, v) in sh.iter().enumerate() {
        x[onset + i] = *v;
    }
    let base = 96i64;
    let (d, score, at_edge) = xcorr_lag(&x, base, span, &tpl).expect("no lag at all");
    assert!(at_edge, "edge lag not flagged: d {}", d);
    assert!(score.abs() > 0.9);
    // the same shot aligned at the base: lag 0, not at the edge
    let mut y = vec![0.0f32; 48000];
    for (i, v) in sh.iter().enumerate() {
        y[96 + i] = *v;
    }
    let (d, _, at_edge) = xcorr_lag(&y, base, span, &tpl).expect("no lag at all");
    assert!(!at_edge);
    assert_eq!(d, 0);
}

#[test]
fn clean_hits_agree_within_half_millisecond() {
    // on clean, sample-exact synthetic hits the three estimators agree within 24 samples
    // (0.5 ms at 48 kHz) for every hit
    let sr = 48_000u32;
    for (role, f) in [("kick", 60.0), ("snare", 200.0), ("tom", 170.0)] {
        let (x, hits, _) = clean_kit(sr, f, 8, None);
        let ag = agreement_track(&x, sr, &hits);
        let sp = sorted_spreads(&ag);
        assert_eq!(sp.len(), hits.len(), "FAIL: missing spreads on {}", role);
        let worst = sp[sp.len() - 1];
        println!(
            "{}: clean spread median {:.0}, worst {:.0}",
            role,
            median_of(&sp),
            worst
        );
        assert!(worst <= 24.0, "FAIL: {} spread {} > 24", role, worst);
    }
}

#[test]
fn noise_and_smeared_attack_widen_the_spread() {
    // the monotone check: planted noise, and a smeared attack, each widen the spread
    // over the clean hit
    let sr = 48_000u32;
    let f = 170.0;
    let (x, hits, _) = clean_kit(sr, f, 8, None);
    let clean = median_of(&sorted_spreads(&agreement_track(&x, sr, &hits)));
    // planted noise at -14 dB under the hit level
    let mut noisy = x.clone();
    for (i, v) in noisy.iter_mut().enumerate() {
        let n = ((i as f64 * 12.9898).sin() * 43758.5453).fract() - 0.5;
        *v += (0.1 * n) as f32;
    }
    let noisy_med = median_of(&sorted_spreads(&agreement_track(&noisy, sr, &hits)));
    println!("clean {:.0}, noisy {:.0}", clean, noisy_med);
    assert!(noisy_med > clean, "FAIL: noise did not widen the spread");
    // a smeared attack: the same shots behind a 2 ms box smear
    let smeared: Vec<f32> = {
        let mut s = vec![0.0f32; x.len()];
        let k = 96usize;
        for i in 0..x.len() {
            let a = i.saturating_sub(k);
            let win = &x[a..=i];
            s[i] = win.iter().sum::<f32>() / win.len() as f32;
        }
        s
    };
    let smear_med = median_of(&sorted_spreads(&agreement_track(&smeared, sr, &hits)));
    println!("smeared {:.0}", smear_med);
    assert!(smear_med > clean, "FAIL: smear did not widen the spread");
}

#[test]
fn polarity_right_on_every_planted_inverted_hit() {
    let sr = 48_000u32;
    // hits every 0.25 s from 0.5 s; the hits at 1.5 s and 2.5 s are planted phase-inverted
    let events: Vec<(i64, i32)> = (0u32..8)
        .map(|i| {
            let s = (i as i64 + 2) * sr as i64 / 4;
            (s, if i == 4 || i == 6 { -1 } else { 1 })
        })
        .collect();
    let (x, hits) = kit(sr, 3.0, 170.0, &events);
    let ag = agreement_track(&x, sr, &hits);
    let mut checked = 0;
    let mut misses = 0;
    for (i, a) in ag.iter().enumerate() {
        checked += 1;
        let want = events[i].1;
        if a.polarity != want {
            misses += 1;
            eprintln!(
                "MISS hit {} at {}: polarity {} want {}",
                i, events[i].0, a.polarity, want
            );
        }
    }
    println!("polarity checked {} hits, {} misses", checked, misses);
    assert!(checked > 0, "FAIL: 0 checked hits");
    assert_eq!(misses, 0, "FAIL: polarity misses");
}

#[test]
fn median_onset_error_stays_zero_and_spread_is_stored() {
    let sr = 48_000u32;
    let (x, hits, trues) = clean_kit(sr, 170.0, 6, None);
    let before = hits.clone();
    let ag = agreement_track(&x, sr, &hits);
    // the hit times are never touched: the median onset error against the sample-exact
    // ground truth stays 0 samples
    let mut errs: Vec<i64> = hits
        .iter()
        .zip(trues.iter())
        .map(|(h, t)| (h * sr as f64).round() as i64 - t)
        .collect();
    errs.sort();
    let median = errs[errs.len() / 2];
    println!("median onset error {} samples over {} hits", median, errs.len());
    assert_eq!(median, 0);
    assert_eq!(hits, before);
    // every checked hit has a spread
    let with_spread = ag.iter().filter(|a| a.timing_confidence.is_some()).count();
    println!("spread stored on {} of {} hits", with_spread, ag.len());
    assert_eq!(with_spread, ag.len());
    assert!(ag.iter().all(|a| a.timing_confidence.unwrap() < 0.025 * sr as f64));
    // silence: no estimates
    let mut quiet = vec![0.0f32; sr as usize];
    assert_eq!(
        agreement_track(&quiet, sr, &[1000.0 / sr as f64])[0],
        Agreement::default()
    );
    // a window far under the silence limit: no spread
    quiet[1000] = 1e-6f32;
    assert_eq!(
        agreement_track(&quiet, sr, &[1000.0 / sr as f64])[0].timing_confidence,
        None
    );
}

#[test]
fn timing_confidence_distribution_per_role() {
    // the counted distribution the report prints: median and p95 of the spread in
    // samples per role, on the clean synthetic kit (one shell per role, isolated
    // planted hits, one planted inverted hit per role that must come out -1)
    let sr = 48_000u32;
    let roles: [(&str, f64); 3] = [("kick", 60.0), ("snare", 200.0), ("tom", 170.0)];
    for (role, f) in roles {
        let (x, hits, _) = clean_kit(sr, f, 12, Some(5));
        let ag = agreement_track(&x, sr, &hits);
        let sp = sorted_spreads(&ag);
        assert_eq!(sp.len(), hits.len(), "FAIL: 0 checked hits for {}", role);
        let n = sp.len();
        let median = median_of(&sp);
        let p95 = sp[(0.95 * (n - 1) as f64).round() as usize];
        let inv_ok = ag
            .iter()
            .enumerate()
            .filter(|(i, a)| if *i == 5 { a.polarity == -1 } else { a.polarity == 1 })
            .count();
        println!(
            "{}: checked {} hits, spread median {:.0} samples, p95 {:.0} samples, polarity on target {} of {}",
            role, n, median, p95, inv_ok, n
        );
        assert_eq!(inv_ok, n, "FAIL: polarity misses on {}", role);
        assert!(p95 <= 24.0, "FAIL: {} p95 spread {} > 24", role, p95);
    }
}

#[test]
fn inverted_template_majority_gives_negative_score() {
    // a track whose template comes from normal hits (the majority) reads a planted
    // inverted hit as -1 even against a loud neighbour
    let sr = 48_000u32;
    let events: Vec<(i64, i32)> = (0u32..7)
        .map(|i| (i as i64 * sr as i64 / 4, if i == 3 { -1 } else { 1 }))
        .collect();
    let (x, hits) = kit(sr, 2.25, 170.0, &events);
    let ag = agreement_track(&x, sr, &hits);
    assert_eq!(ag[3].polarity, -1);
    assert!(ag.iter().filter(|a| a.polarity == 1).count() >= 5);
}
