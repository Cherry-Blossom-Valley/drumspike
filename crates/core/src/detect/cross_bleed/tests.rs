//! Tests of the cross-track bleed rule (#281 A6): a planted kick bleed on a tom track is dropped, a
//! real tom hit 10 ms after a kick is kept.

use super::{CrossBleedParams, CrossMic, XKind, cross_bleed};

const SR: u32 = 48_000;

/// A kick-like hit (125 -> 55 Hz sweep, 0.5 ms attack, 0.12 s decay) at `t0` s, `db` dBFS.
fn kick_hit(x: &mut [f32], t0: f64, db: f64) {
    let i0 = (t0 * f64::from(SR)).round() as usize;
    let amp = 10f64.powf(db / 20.0);
    let mut ph = 0.0f64;
    for k in 0..((0.3 * f64::from(SR)) as usize).min(x.len().saturating_sub(i0)) {
        let t = k as f64 / f64::from(SR);
        ph += 2.0 * std::f64::consts::PI * (55.0 + 70.0 * (-t / 0.03).exp()) / f64::from(SR);
        let env = (t / 0.0005).min(1.0) * (-t / 0.12).exp();
        x[i0 + k] += (ph.sin() * env * amp) as f32;
    }
}

/// A tom-like hit (200 -> 90 Hz sweep, 0.5 ms attack, 0.25 s decay) at `t0` s, `db` dBFS.
fn tom_hit(x: &mut [f32], t0: f64, db: f64) {
    let i0 = (t0 * f64::from(SR)).round() as usize;
    let amp = 10f64.powf(db / 20.0);
    let mut ph = 0.0f64;
    for k in 0..((0.4 * f64::from(SR)) as usize).min(x.len().saturating_sub(i0)) {
        let t = k as f64 / f64::from(SR);
        ph += 2.0 * std::f64::consts::PI * (90.0 + 110.0 * (-t / 0.05).exp()) / f64::from(SR);
        let env = (t / 0.0005).min(1.0) * (-t / 0.25).exp();
        x[i0 + k] += (ph.sin() * env * amp) as f32;
    }
}

#[test]
fn kick_bleed_is_dropped_and_a_real_tom_hit_10ms_after_a_kick_is_kept() {
    // The kick mic: a kick alone at 0.2 s (a clear case) and the kick at 1.0 s. The tom mic hears the
    // kicks 1 ms later and 18 dB under (the bleed), plus a real tom hit 10 ms after the second kick.
    let dur = 2.0f64;
    let n = (dur * f64::from(SR)) as usize;
    let mut kick = vec![0f32; n];
    let mut tom = vec![0f32; n];
    kick_hit(&mut kick, 0.2, -3.0);
    kick_hit(&mut kick, 0.3, -3.0);
    kick_hit(&mut kick, 0.4, -3.0);
    kick_hit(&mut kick, 0.6, -3.0);
    kick_hit(&mut kick, 0.7, -3.0);
    kick_hit(&mut kick, 0.8, -3.0);
    kick_hit(&mut kick, 1.0, -3.0);
    // the clear cases: the tom mic hears the alone kicks 1 ms later and 18 dB under (the bleed)
    for t in [0.2, 0.3, 0.4, 0.6, 0.7, 0.8] {
        kick_hit(&mut tom, t + 0.001, -21.0);
    }
    // the coinciding case: bleed of the 1.0 s kick
    kick_hit(&mut tom, 1.0 + 0.001, -21.0);
    // the real tom hit 10 ms after the kick: loud on the tom mic (louder in the tom band than the
    // kick's band level on the kick mic), motion starts 10 ms after the kick's
    tom_hit(&mut tom, 1.010, 0.0);
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: vec![0.201, 1.001, 1.010],
        },
        CrossMic {
            kind: XKind::Kick,
            signal: &kick,
            motion: vec![0.200, 0.300, 0.400, 0.600, 0.700, 0.800, 1.000],
        },
    ];
    let (marks, learned) = cross_bleed(&mics, SR, &CrossBleedParams::default());
    // the pair learned the clear case: the bleed arrives about 1 ms late at about -18 dB
    let pl = &learned[0];
    assert_eq!(pl.tom, 0);
    assert_eq!(pl.source, 1);
    let lead = pl.lead_ms.expect("the clear case teaches the lead");
    let ratio = pl.ratio_db.expect("the clear case teaches the ratio");
    assert!((0.5..=3.0).contains(&lead), "lead {lead}");
    assert!((-25.0..=-10.0).contains(&ratio), "ratio {ratio}");
    // hit 1: the bleed of the 1.0 s kick (later and much quieter) is dropped
    assert!(marks[0][1].bleed, "{:?}", marks[0][1]);
    // hit 2: the real tom hit is kept (it is loud on the tom mic, so no level test fires)
    assert!(!marks[0][2].bleed, "{:?}", marks[0][2]);
    // the evidence is stored per hit
    assert!(marks[0][1].lead_ms.is_some_and(|l| l > 0.3));
    assert!(marks[0][2].lead_ms.is_some_and(|l| (l - 10.0).abs() < 0.5));
}

#[test]
fn a_tom_hit_that_leads_the_kick_stays_even_when_quieter() {
    let n = (1.0 * f64::from(SR)) as usize;
    let mut kick = vec![0f32; n];
    let mut tom = vec![0f32; n];
    // a quiet tom hit 2 ms before a loud kick (a flam-like unison): the tom leads, so it stays
    kick_hit(&mut kick, 0.5, -3.0);
    kick_hit(&mut tom, 0.5 + 0.001, -21.0); // the bleed of the kick, for the clear-case-free pair
    tom_hit(&mut tom, 0.498, -20.0);
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: vec![0.498, 0.501],
        },
        CrossMic {
            kind: XKind::Kick,
            signal: &kick,
            motion: vec![0.500],
        },
    ];
    let (marks, learned) = cross_bleed(&mics, SR, &CrossBleedParams::default());
    // no clear case (the tom lists a hit at every kick): the fixed lead bound and no level test
    assert!(learned[0].lead_ms.is_none());
    // the leading tom hit stays
    assert!(!marks[0][0].bleed, "{:?}", marks[0][0]);
}

#[test]
fn a_tom_mics_bleed_on_another_tom_mic_is_dropped() {
    // tom_1 plays alone three times (the clear cases) and once together with a real tom_2 hit;
    // tom_2's mic hears every tom_1 hit 1 ms later and 15 dB under, and its detector fires on the
    // bleed of the unison hit: the rule drops it (tom close mics are sources for each other).
    let n = (2.0f64 * f64::from(SR)) as usize;
    let mut t1 = vec![0f32; n];
    let mut t2 = vec![0f32; n];
    for t in [0.2, 0.4, 0.6] {
        tom_hit(&mut t1, t, -3.0);
        tom_hit(&mut t2, t + 0.001, -18.0);
    }
    tom_hit(&mut t1, 1.0, -3.0);
    // the real tom_2 hit 2 ms after the tom_1 hit: loud on tom_2's own mic
    tom_hit(&mut t2, 1.0 + 0.001, 0.0);
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &t2,
            motion: vec![0.201, 0.401, 0.601, 1.001],
        },
        CrossMic {
            kind: XKind::Tom,
            signal: &t1,
            motion: vec![0.200, 0.400, 0.600, 1.000],
        },
    ];
    let (marks, learned) = cross_bleed(&mics, SR, &CrossBleedParams::default());
    let pl = &learned[0];
    assert_eq!(pl.tom, 0);
    assert_eq!(pl.source, 1);
    // the three clear-case bleeds are dropped; the real tom_2 hit stays
    assert!(marks[0][0].bleed, "{:?}", marks[0][0]);
    assert!(marks[0][1].bleed, "{:?}", marks[0][1]);
    assert!(marks[0][2].bleed, "{:?}", marks[0][2]);
    assert!(!marks[0][3].bleed, "{:?}", marks[0][3]);
}

/// A hi-hat-like hit (a 7 kHz ping, 0.1 ms attack, 0.05 s decay) at `t0` s, `db` dBFS.
fn hat_hit(x: &mut [f32], t0: f64, db: f64) {
    let i0 = (t0 * f64::from(SR)).round() as usize;
    let amp = 10f64.powf(db / 20.0);
    for k in 0..((0.05 * f64::from(SR)) as usize).min(x.len().saturating_sub(i0)) {
        let t = k as f64 / f64::from(SR);
        let env = (t / 0.0001).min(1.0) * (-t / 0.05).exp();
        x[i0 + k] += ((2.0 * std::f64::consts::PI * 7000.0 * t).sin() * env * amp) as f32;
    }
}

#[test]
fn a_hat_pairs_evidence_is_recorded_and_the_generic_verdict_fires() {
    // #285: the hi-hat close mic is a source kind. The hat plays alone three times (the clear
    // cases) and once with a real tom hit; the tom mic hears the hat 1 ms later and 18 dB under.
    // #103 step 2, the measured verdict (docs/review/detector-plan.md package 8): the hat-pair
    // distributions overlap, so no NEW hat threshold is added — the pair's own evidence is
    // recorded on every hit. The GENERIC #285 verdict still fires on hat pairs exactly as on
    // kick and snare pairs: the hat-cause arrivals (later than the hat by more than the pair's
    // lead bound, quieter) are dropped. Bypassing the generic verdict for hat sources kept 521
    // hat-bleed tom events that main drops and regressed the tom F1 rows (103-tomdiff).
    let n = (2.0f64 * f64::from(SR)) as usize;
    let mut hat = vec![0f32; n];
    let mut tom = vec![0f32; n];
    for t in [0.2, 0.4, 0.6] {
        hat_hit(&mut hat, t, -3.0);
        hat_hit(&mut tom, t + 0.001, -18.0);
    }
    hat_hit(&mut hat, 1.0, -3.0);
    // the real tom hit 2 ms after the hat: loud on the tom mic's own band
    tom_hit(&mut tom, 1.0 + 0.001, 0.0);
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            // the detector's refine back-track: the bleed arrivals 1 ms after the hat, the real
            // hit at 1.001
            motion: vec![0.201, 0.401, 0.601, 1.001],
        },
        CrossMic {
            kind: XKind::Hat,
            signal: &hat,
            motion: vec![0.200, 0.400, 0.600, 1.000],
        },
    ];
    let (marks, learned) = cross_bleed(&mics, SR, &CrossBleedParams::default());
    assert_eq!(learned[0].source, 1);
    // the hat pair's own lead/ratio is recorded on every hit, and the generic verdict fires:
    // the three bleed arrivals (1 ms after the hat, 15 dB under) are dropped, the real tom
    // hit (louder than the hat in the tom band, ratio +4.9 dB) stays.
    for m in &marks[0][..3] {
        assert!(m.hat_lead_ms.is_some() && m.hat_ratio_db.is_some(), "{m:?}");
        assert!(m.bleed, "{m:?}");
    }
    assert!(!marks[0][3].bleed, "{:?}", marks[0][3]);
    assert!(marks[0][3].hat_lead_ms.is_some(), "{:?}", marks[0][3]);
}

#[test]
fn an_overhead_mic_is_not_a_tom_pair_source() {
    // #286 A1, measured: the common-band (full-band) no-lead oh test is a SNARE-target form
    // (the soft gate's `soft_overhead_peaks`); in a tom pair the tom's own-band level against
    // the oh's full-band peak is not like with like and the form dropped about two thirds of
    // the tom hits (Session A t1 66 -> 19). So the oh is no source for the tom rule at all: a
    // loud oh marks nothing, a quiet oh neither.
    let n = (2.0f64 * f64::from(SR)) as usize;
    let mut oh = vec![0f32; n];
    let mut tom = vec![0f32; n];
    // two clear cases (the oh alone; the tom mic hears a weak response 1 ms later)
    for t in [0.2, 0.4] {
        hat_hit(&mut oh, t, -6.0);
        hat_hit(&mut tom, t + 0.001, -30.0);
    }
    // the would-be bleed case: the oh 6 dB over the tom mic's response
    hat_hit(&mut oh, 0.8, -6.0);
    hat_hit(&mut tom, 0.8 + 0.001, -12.0);
    // the ghost-note case: a real quiet tom stroke, the oh under it in the tom's band
    hat_hit(&mut oh, 1.2, -20.0);
    tom_hit(&mut tom, 1.2 + 0.001, -10.0);
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: vec![0.201, 0.401, 0.801, 1.201],
        },
        CrossMic {
            kind: XKind::Overhead,
            signal: &oh,
            motion: vec![0.200, 0.400, 0.800, 1.200],
        },
    ];
    let (marks, learned) = cross_bleed(&mics, SR, &CrossBleedParams::default());
    assert!(learned.is_empty(), "{learned:?}");
    for (j, m) in marks[0].iter().enumerate() {
        assert!(!m.bleed, "hit {j} marked bleed: {m:?}");
    }
}
