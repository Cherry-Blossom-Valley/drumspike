//! The unit tests of `role.rs` (#281 item 10, the ticket's gate (a)): synthetic tracks.

use super::{role_features, role_from_audio, role_from_audio_confidence, role_threshold, Role};
use std::f64::consts::PI;

const SR: u32 = 44100;

fn secs(s: f64) -> usize {
    (s * f64::from(SR)) as usize
}

/// A sine at `hz` with an exponential decay of `tau` seconds, hit every `period` s.
fn decaying_sine(hz: f64, tau: f64, period: f64, dur: f64, amp: f32) -> Vec<f32> {
    let n = secs(dur);
    let mut x = vec![0.0_f32; n];
    let mut t0 = 0.1; // the first hit needs a before-level to rise from
    while t0 < dur {
        let start = secs(t0);
        for (i, slot) in x.iter_mut().enumerate().take(n).skip(start) {
            let t = (i - start) as f64 / f64::from(SR);
            *slot += amp * (2.0 * PI * hz * t).sin() as f32 * (-t / tau).exp() as f32;
        }
        t0 += period;
    }
    x
}

/// A band-limited noise burst with a decay of `tau` seconds through a one-pole high-pass
/// at `cut_hz` and a one-pole low-pass at 6 kHz, plus an optional body sine, hit every
/// `period` s.
fn noise_hit(cut_hz: f64, body_hz: Option<f64>, tau: f64, period: f64, dur: f64, amp: f32) -> Vec<f32> {
    let n = secs(dur);
    let mut x = vec![0.0_f32; n];
    let mut lp = 0.0_f64;
    let mut lp2 = 0.0_f64;
    let a = (-2.0 * PI * cut_hz / f64::from(SR)).exp();
    let b = (-2.0 * PI * 6000.0 / f64::from(SR)).exp();
    let mut t0 = 0.1; // the first hit needs a before-level to rise from
    let mut seed = 0x2545_F491_4F6C_DD1D_u64;
    while t0 < dur {
        let start = secs(t0);
        for (i, slot) in x.iter_mut().enumerate().take(n).skip(start) {
            let t = (i - start) as f64 / f64::from(SR);
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let noise = (seed >> 33) as i64 as f64 / (1u64 << 31) as f64 - 1.0;
            lp = a * lp + (1.0 - a) * noise;
            let hp = noise - lp; // the high-passed part
            lp2 = b * lp2 + (1.0 - b) * hp;
            let env = (-t / tau).exp();
            let mut s = (hp - lp2) * env; // band-limited to cut_hz..6 kHz
            if let Some(bh) = body_hz {
                // The body dominates a real snare mic; the wire noise rides on top.
                s += 2.5 * (2.0 * PI * bh * t).sin() * env;
            }
            *slot += (amp as f64 * s) as f32;
        }
        t0 += period;
    }
    x
}

/// Steady pink-ish noise (1/f: white noise through a heavy low-pass, no onsets).
fn pink_noise(dur: f64, amp: f32) -> Vec<f32> {
    let n = secs(dur);
    let mut x = Vec::with_capacity(n);
    let mut lp = 0.0_f64;
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    for _ in 0..n {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let noise = (seed >> 33) as i64 as f64 / (1u64 << 31) as f64 - 1.0;
        lp = 0.995 * lp + 0.005 * noise;
        x.push((amp as f64 * lp * 30.0) as f32);
    }
    x
}

fn assert_role(x: &[f32], want: Role) {
    let f = role_features(x, SR);
    assert_eq!(role_from_audio(x, SR), Some(want), "features {f:?}");
}

/// The per-role bars (`role_threshold`, ticket #393): kick and tom pass, the roles that
/// never reach 0.90 precision on ENST have no bar and are never suggested.
#[test]
fn per_role_thresholds_match_the_decision() {
    assert_eq!(role_threshold(Role::Kick), Some(0.05));
    assert_eq!(role_threshold(Role::Tom), Some(0.60));
    assert_eq!(role_threshold(Role::Snare), None);
    assert_eq!(role_threshold(Role::Hat), None);
    assert_eq!(role_threshold(Role::Cymbal), None);
}

/// The kick fixture clears its own bar (0.05).
#[test]
fn a_kick_fixture_clears_its_bar() {
    let x = decaying_sine(60.0, 0.25, 0.5, 4.0, 0.9);
    let (raw, c) = role_from_audio_confidence(&x, SR);
    assert_eq!(raw, Some(Role::Kick));
    assert!(c >= 0.05, "confidence {c} under the kick bar");
    assert_eq!(role_from_audio(&x, SR), Some(Role::Kick));
}

/// The tom fixture clears its own bar (0.60).
#[test]
fn a_tom_fixture_clears_its_bar() {
    let x = decaying_sine(180.0, 0.5, 0.5, 4.0, 0.8);
    let (raw, c) = role_from_audio_confidence(&x, SR);
    assert_eq!(raw, Some(Role::Tom));
    assert!(c >= 0.60, "confidence {c} under the tom bar");
    assert_eq!(role_from_audio(&x, SR), Some(Role::Tom));
}

/// A snare fixture is never suggested: snare has no bar on the ENST run (the snare and
/// tom votes trade places), the raw vote still names it.
#[test]
fn a_snare_fixture_is_never_suggested() {
    let x = noise_hit(60.0, Some(200.0), 0.12, 0.5, 4.0, 0.7);
    let (raw, _) = role_from_audio_confidence(&x, SR);
    assert_eq!(raw, Some(Role::Snare));
    assert_eq!(role_from_audio(&x, SR), None);
}

/// A hat fixture is never suggested either (hat has no bar).
#[test]
fn a_hat_fixture_is_never_suggested() {
    let x = noise_hit(2000.0, None, 0.08, 0.25, 4.0, 0.6);
    let (raw, _) = role_from_audio_confidence(&x, SR);
    assert_eq!(raw, Some(Role::Hat));
    assert_eq!(role_from_audio(&x, SR), None);
}

/// A role below its bar is not suggested: half tom hits and half snare-ish hits put the
/// vote margin under the tom bar of 0.60, so the tom suggestion is withheld.
#[test]
fn a_role_below_its_bar_returns_none() {
    let mut x = decaying_sine(180.0, 0.5, 1.0, 4.0, 0.8);
    // Snare-ish bursts between them (a 200 Hz body with wire noise): their votes drop
    // the tom share and margin under the bar.
    let snare = noise_hit(60.0, Some(200.0), 0.12, 2.0, 4.0, 0.7);
    for (i, s) in x.iter_mut().enumerate() {
        *s += snare[i];
    }
    let (raw, c) = role_from_audio_confidence(&x, SR);
    // The votes split between tom and snare: whoever wins, the confidence is under the
    // winner's bar (tom 0.60; snare has no bar at all), so nothing is suggested.
    assert!(raw == Some(Role::Tom) || raw == Some(Role::Snare));
    assert!(c < 0.60, "expected the confidence under the tom bar, got {c}");
    assert_eq!(role_from_audio(&x, SR), None);
}

#[test]
fn a_low_decaying_sine_is_a_kick() {
    let x = decaying_sine(60.0, 0.25, 0.5, 4.0, 0.9);
    assert_role(&x, Role::Kick);
}

#[test]
fn a_noise_burst_with_a_200_hz_body_is_a_snare() {
    // The high-pass at 60 Hz keeps the burst broadband (a real snare is not bright:
    // its wire noise is well under the 200 Hz body and the room).
    let x = noise_hit(60.0, Some(200.0), 0.12, 0.5, 4.0, 0.7);
    let (raw, _) = role_from_audio_confidence(&x, SR);
    assert_eq!(raw, Some(Role::Snare));
    // Snare has no bar on the ENST run: it is never suggested.
    assert_eq!(role_from_audio(&x, SR), None);
}

#[test]
fn bright_short_noise_is_a_hat() {
    let x = noise_hit(2000.0, None, 0.08, 0.25, 4.0, 0.6);
    let (raw, _) = role_from_audio_confidence(&x, SR);
    assert_eq!(raw, Some(Role::Hat));
    // Hat has no bar on the ENST run: it is never suggested.
    assert_eq!(role_from_audio(&x, SR), None);
}

#[test]
fn long_bright_noise_is_a_cymbal() {
    // One single bright hit with a long tail (steady noise would have no onsets).
    let x = noise_hit(2000.0, None, 2.0, 10.0, 4.0, 0.6);
    let (raw, _) = role_from_audio_confidence(&x, SR);
    assert_eq!(raw, Some(Role::Cymbal));
    // Cymbal has no bar on the ENST run: it is never suggested.
    assert_eq!(role_from_audio(&x, SR), None);
}

#[test]
fn pink_noise_is_none() {
    let x = pink_noise(4.0, 0.4);
    let f = role_features(&x, SR);
    assert_eq!(role_from_audio(&x, SR), None, "features {f:?}");
}

#[test]
fn empty_and_silent_input_is_none() {
    assert_eq!(role_from_audio(&[], SR), None);
    assert_eq!(role_from_audio(&vec![0.0_f32; secs(1.0)], SR), None);
}

#[test]
fn role_strs_are_the_app_names() {
    assert_eq!(Role::Kick.as_str(), "kick");
    assert_eq!(Role::Snare.as_str(), "snare");
    assert_eq!(Role::Tom.as_str(), "tom");
    assert_eq!(Role::Hat.as_str(), "hat");
    assert_eq!(Role::Cymbal.as_str(), "cymbal");
}

#[test]
fn a_mid_hit_with_a_long_body_is_a_tom() {
    // A 180 Hz body with a slow decay and little noise: mid band, longer than a snare.
    let x = decaying_sine(180.0, 0.5, 0.5, 4.0, 0.8);
    assert_role(&x, Role::Tom);
}

#[test]
fn features_work_at_48k() {
    // A resampled (48 kHz) version of the kick: the bands are fixed in Hz, so the
    // decision must not move with the rate.
    let x44 = decaying_sine(60.0, 0.25, 0.5, 4.0, 0.9);
    let mut x48 = Vec::with_capacity(secs(4.0));
    let mut t = 0.0_f64;
    while t * f64::from(SR) + 1.0 < x44.len() as f64 {
        let i = (t * f64::from(SR)) as usize;
        let frac = t * f64::from(SR) - i as f64;
        x48.push(x44[i] * ((1.0 - frac) as f32) + x44[i + 1] * (frac as f32));
        t += 1.0 / 48000.0;
    }
    let f = role_features(&x48, 48000);
    assert!(f.w_low > 0.4, "features {f:?}");
    assert_eq!(role_from_audio(&x48, 48000), Some(Role::Kick));
}

/// The fixture of the Add Tracks e2e (`tests/e2e_app/test_tracks.py::test_an_unnamed_file_gets_the_audio_suggestion_and_the_mark`,
/// helpers.py `kick_hit`: the pitch sweep 125 -> 55 Hz, a 0.12 s decay, a 0.5 ms attack, a hit
/// every 0.5 s, 48 kHz, the gain -3 dB) is a kick the model-free core suggests (#406: the e2e
/// runs without the old model, so its fixture must clear the core's own kick bar).
#[test]
fn the_add_tracks_e2e_fixture_is_a_kick() {
    let sr: u32 = 48000;
    let n = (4.0 * f64::from(sr)) as usize;
    let mut x = vec![0.0_f32; n];
    for &t0 in &[0.5, 1.0, 1.5, 2.0, 2.5, 3.0] {
        let start = (t0 * f64::from(sr)) as usize;
        for (i, slot) in x.iter_mut().enumerate().take(n).skip(start) {
            let t = (i - start) as f64 / f64::from(sr);
            let f = 55.0 + 70.0 * (-t / 0.03).exp();
            let env = (t / 0.0005).min(1.0) as f32 * (-t / 0.12).exp() as f32;
            let g = 10.0_f64.powf(-3.0 / 20.0) as f32;
            *slot += g * (2.0 * std::f64::consts::PI * f * t).sin() as f32 * env;
        }
    }
    let (raw, c) = role_from_audio_confidence(&x, sr);
    assert_eq!(raw, Some(Role::Kick));
    assert!(
        c >= role_threshold(Role::Kick).unwrap(),
        "confidence {c} under the kick bar"
    );
    assert_eq!(role_from_audio(&x, sr), Some(Role::Kick));
}
