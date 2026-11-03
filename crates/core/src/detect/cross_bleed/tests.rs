//! Tests of the cross-track bleed rule (#281 A6): a planted kick bleed on a tom track is dropped, a
//! real tom hit 10 ms after a kick is kept.

use super::{CrossBleedParams, CrossMic, PairLearned, XKind, cross_bleed, cross_bleed_prior, inherits_pass1_bleed};
use crate::refine::first_motion;

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
fn an_aligned_pair_judges_by_level_alone() {
    // #296: a time-aligned session (Auto-Align 2 and the like): the tom's hits sit at the kick's
    // with a median |lead| of 0.2 ms — the alignment removed the arrival delay, so the lead test
    // cannot fire. 24 coincidences make the pair aligned: the verdict is the level test alone
    // (the bleed is quieter in the kick's band), the margin sweep is counted, and a loud real
    // tom hit away from any kick stays.
    let n = (2.0f64 * f64::from(SR)) as usize;
    let mut kick = vec![0f32; n];
    let mut tom = vec![0f32; n];
    let mut kick_motion = Vec::new();
    let mut tom_motion = Vec::new();
    for i in 0..24 {
        let t = 0.2 + f64::from(i) * 0.05;
        kick_hit(&mut kick, t, -3.0);
        kick_hit(&mut tom, t + 0.0002, -21.0);
        kick_motion.push(t);
        tom_motion.push(t + 0.0002);
    }
    tom_hit(&mut tom, 1.5, 0.0);
    tom_motion.push(1.5);
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: tom_motion,
        },
        CrossMic {
            kind: XKind::Kick,
            signal: &kick,
            motion: kick_motion,
        },
    ];
    let (marks, learned) = cross_bleed(&mics, SR, &CrossBleedParams::default());
    let pl = &learned[0];
    assert!(pl.aligned, "{pl:?}");
    assert_eq!(pl.align_coins, 24);
    assert!(
        pl.align_med_abs_lead_ms.is_some_and(|m| (m - 0.2).abs() < 0.05),
        "{pl:?}"
    );
    // every bleed hit (the 0.2 ms copies, about 18 dB under) is dropped by level alone
    for (j, m) in marks[0].iter().enumerate().take(24) {
        assert!(m.bleed, "hit {j}: {m:?}");
    }
    assert!(!marks[0][24].bleed, "{:?}", marks[0][24]);
    // the margin sweep: all 24 fall under each of -3, 0, +3 and +6 dB
    assert_eq!(pl.aligned_counts, [24, 24, 24, 24], "{pl:?}");
}

#[test]
fn a_pair_with_a_real_lead_keeps_the_two_part_verdict() {
    // the same geometry with the acoustic 1 ms arrival: the pair is NOT aligned (the median
    // |lead| sits over the bound) and a quiet tom hit 2 ms BEFORE the kick stays even though a
    // coinciding kick hit exists (the lead test denies).
    let n = (2.0f64 * f64::from(SR)) as usize;
    let mut kick = vec![0f32; n];
    let mut tom = vec![0f32; n];
    let mut kick_motion = Vec::new();
    let mut tom_motion = Vec::new();
    for i in 0..24 {
        let t = 0.2 + f64::from(i) * 0.05;
        kick_hit(&mut kick, t, -3.0);
        kick_hit(&mut tom, t + 0.001, -21.0);
        kick_motion.push(t);
        tom_motion.push(t + 0.001);
    }
    // a quiet real tom hit 2 ms before the last kick
    tom_hit(&mut tom, 1.348, -20.0);
    tom_motion.push(1.348);
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: tom_motion,
        },
        CrossMic {
            kind: XKind::Kick,
            signal: &kick,
            motion: kick_motion,
        },
    ];
    let (marks, learned) = cross_bleed(&mics, SR, &CrossBleedParams::default());
    let pl = &learned[0];
    assert!(!pl.aligned, "{pl:?}");
    assert_eq!(pl.aligned_counts, [0, 0, 0, 0]);
    // the leading quiet hit stays (it leads the kick), the 1 ms bleed hits are dropped
    assert!(!marks[0][24].bleed, "{:?}", marks[0][24]);
    for m in &marks[0][..24] {
        assert!(m.bleed, "{m:?}");
    }
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

#[test]
fn the_prior_alignment_is_honoured_in_the_re_detection() {
    // #296 verify fix: the alignment is a property of the PAIR, not of one hit list. The same
    // geometry as `a_pair_with_a_real_lead_keeps_the_two_part_verdict` (a real 1 ms arrival, the
    // measured pair NOT aligned) with the PASS-1 pairs handed in: the re-measured alignment
    // (which moved over the bound, as the re-detection's candidate times do) does not win — the
    // verdict is the level test alone, the recorded coins/median stay the pass-1 values.
    let n = (2.0f64 * f64::from(SR)) as usize;
    let mut kick = vec![0f32; n];
    let mut tom = vec![0f32; n];
    let mut kick_motion = Vec::new();
    let mut tom_motion = Vec::new();
    for i in 0..24 {
        let t = 0.2 + f64::from(i) * 0.05;
        kick_hit(&mut kick, t, -3.0);
        kick_hit(&mut tom, t + 0.001, -21.0);
        kick_motion.push(t);
        tom_motion.push(t + 0.001);
    }
    tom_hit(&mut tom, 1.348, -20.0);
    tom_motion.push(1.348);
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: tom_motion,
        },
        CrossMic {
            kind: XKind::Kick,
            signal: &kick,
            motion: kick_motion,
        },
    ];
    let p = CrossBleedParams::default();
    let (marks, learned) = cross_bleed(&mics, SR, &p);
    assert!(!learned[0].aligned, "{learned:?}");
    // the pass-1 decision: the pair aligned with 30 coins (as on the stored hits)
    let prior = [PairLearned {
        tom: 0,
        source: 1,
        lead_ms: None,
        ratio_db: None,
        aligned: true,
        align_coins: 30,
        align_med_abs_lead_ms: Some(0.06),
        aligned_counts: [0; 4],
        align_inherited: false,
    }];
    let (marks2, learned2) = cross_bleed_prior(&mics, SR, &p, Some(&prior));
    assert!(learned2[0].aligned, "{learned2:?}");
    assert_eq!(learned2[0].align_coins, 30);
    assert_eq!(learned2[0].align_med_abs_lead_ms, Some(0.06));
    // level alone: every coinciding hit is bleed, the real hit (quieter than the kick) too
    for (j, m) in marks2[0].iter().enumerate().take(24) {
        assert!(m.bleed, "hit {j}: {m:?}");
    }
    assert!(marks2[0][24].bleed, "{:?}", marks2[0][24]);
    // and the two-part verdict of the measured run is different (the fix changes the outcome)
    assert_ne!(marks[0][24].bleed, marks2[0][24].bleed);
}

/// The pass-2 motion of the app (`refined_motion` in `crates/app/src/cross_bleed.rs`): the bare
/// [`first_motion`] walk, no bounds. The stored times of the kept hits ARE this walk's times —
/// the analysis's own placement (#296 decision: no re-timing of the kept hits).
fn walk_motion(sig: &[f32], ts: &[f64]) -> Vec<f64> {
    ts.iter()
        .map(|&t| {
            let s = (t * f64::from(SR)).round() as i64;
            let (foot, _) = first_motion(sig, s, SR, t, None, None);
            foot as f64 / f64::from(SR)
        })
        .collect()
}

#[test]
fn a_pass_two_candidate_within_1ms_of_a_pass1_bleed_hit_is_dropped_by_inheritance() {
    // #296 test 1: the pass-2 verdict — no coinciding source at the first-motion time — would
    // KEEP the candidate, but a pass-1 bleed-marked hit sits within 1 ms of its verdict time,
    // so the inherit rule drops it. The pass-1 mark list is handed in as the app hands it: the
    // times of the hits pass 1 judged bleed over the FULL source set (pass 1 may have seen a
    // source that pass 2's walked time no longer coincides with — the walked time sits on the
    // bleed's slow rise, more than coincide_ms before the source's attack).
    let n = (2.0f64 * f64::from(SR)) as usize;
    let mut snare = vec![0f32; n];
    let mut tom = vec![0f32; n];
    // a snare hit at 1.5 s (the pair's clear case) and the weak event at 0.5 s (the re-detected
    // candidate of a hit pass 1 marked bleed)
    kick_hit(&mut snare, 1.5, -3.0);
    kick_hit(&mut tom, 1.5 + 0.001, -20.0);
    tom_hit(&mut tom, 0.5, -20.0);
    // pass 2: the candidate's verdict time is the bare walk; the only source hit is far away,
    // so the verdict has nothing to compare against and keeps the candidate on its own
    let cands = [0.5];
    let walked = walk_motion(&tom, &cands);
    let mics_pass2 = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: walked.clone(),
        },
        CrossMic {
            kind: XKind::Snare,
            signal: &snare,
            motion: vec![1.5],
        },
    ];
    let (marks_t, _) = cross_bleed_prior(&mics_pass2, SR, &CrossBleedParams::default(), None);
    assert!(
        (walked[0] - 1.5).abs() > 0.010 + 1e-9,
        "no snare hit within coincide_ms of the walked time {}",
        walked[0]
    );
    assert!(!marks_t[0][0].bleed, "the own verdict keeps it: {:?}", marks_t[0][0]);
    // the inherit rule drops it: the pass-1 bleed hit sits within 1 ms of the verdict time
    let pass1_bleed = [walked[0] + 0.0005];
    assert!(
        inherits_pass1_bleed(walked[0], &pass1_bleed),
        "the pass-1 bleed hit is within 1 ms of the verdict time {}",
        walked[0]
    );
}

#[test]
fn a_pass_two_candidate_far_from_every_pass1_bleed_hit_keeps_its_own_verdict() {
    // #296 test 2: no pass-1-marked hit within 1 ms — the rule does not fire and the candidate
    // is judged exactly as before the rule (the band is tight: 1.5 ms away is not inherited).
    let mut x = vec![0f32; SR as usize];
    tom_hit(&mut x, 0.5, -6.0);
    let walked = walk_motion(&x, &[0.5]);
    let near = [walked[0] + 0.0005];
    let far = [walked[0] + 0.0015];
    assert!(inherits_pass1_bleed(walked[0], &near), "0.5 ms is inside the band");
    assert!(!inherits_pass1_bleed(walked[0], &far), "1.5 ms is outside the band");
    assert!(!inherits_pass1_bleed(walked[0], &[]), "no pass-1 marks: no inheritance");
}

#[test]
fn the_kept_hits_stored_times_are_the_analysis_placement_on_the_two_part_geometry() {
    // #296 test 3: on the two-part-verdict geometry the bare walk does NOT move a loud real hit
    // onto the snare hit (the round-3 regression came from extra refine bounds; there are none
    // now) — the stored time is the analysis's own placement, 2.5 to 5.6 ms from the snare.
    let n = (9.0f64 * f64::from(SR)) as usize;
    let mut snare = vec![0f32; n];
    let mut tom = vec![0f32; n];
    let mut snare_motion = Vec::new();
    let mut cands = Vec::new();
    // 24 snare strokes 0.3 s apart; the tom mic hears each 1 ms later and 14 dB under; the grid
    // candidates sit 11 ms early
    for i in 0..24 {
        let t = 0.2 + f64::from(i) * 0.3;
        kick_hit(&mut snare, t, -6.0);
        kick_hit(&mut tom, t + 0.001, -20.0);
        snare_motion.push(t);
        cands.push(t - 0.011);
    }
    // the loud real tom hit 4 ms after the last snare hit (the tom mic 2 dB louder than the
    // snare mic's level at its hit — the level test keeps it); the grid candidate 1 ms early.
    let tl = snare_motion[23] + 0.004;
    tom_hit(&mut tom, tl, -4.0);
    tom_hit(&mut snare, tl + 0.001, -14.0);
    cands.push(tl - 0.001);
    // the stored placement of the kept hit is the bare walk of the tom's OWN material — it takes
    // no source signal and no source hit time at all (the round-3 regression moved the stored
    // time onto the snare hit with extra refine bounds; the bare walk has none and cannot
    // collapse onto the source)
    let walked = walk_motion(&tom, &cands);
    let last = walked.len() - 1;
    // and it does not sit ON the snare hit (the round-3 collapse put it 0.1 ms from the source)
    assert!(
        (walked[last] - snare_motion[23]).abs() > 0.0005,
        "the stored time is not moved onto the snare hit ({} vs {})",
        walked[last],
        snare_motion[23]
    );
    // and the verdict on that placement keeps the loud hit (the two-part verdict)
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: walked,
        },
        CrossMic {
            kind: XKind::Snare,
            signal: &snare,
            motion: snare_motion,
        },
    ];
    let (marks, _) = cross_bleed_prior(&mics, SR, &CrossBleedParams::default(), None);
    assert!(!marks[0][last].bleed, "the loud hit survives: {:?}", marks[0][last]);
}

#[test]
fn a_sparse_pair_inherits_the_sessions_alignment() {
    // #309: an aligned session (the tool applied to every track). The snare pair has 24
    // coincidences at a 0.2 ms median |lead| — measured aligned. The kick pair has only 2 (the
    // kick rarely coincides with the tom), and its coins are the bleed events themselves, whose
    // residual lead looks real: the pair cannot measure its own alignment and inherits the
    // session's — the verdict is the level test alone, so the kick bleed 3 ms after the kick
    // (which the lead test let through) is dropped, and a loud real tom hit stays.
    let n = (8.0f64 * f64::from(SR)) as usize;
    let mut snare = vec![0f32; n];
    let mut kick = vec![0f32; n];
    let mut tom = vec![0f32; n];
    let mut tom_motion = Vec::new();
    let mut snare_motion = Vec::new();
    // 24 snare strokes; the tom mic hears each at the snare's time (aligned) and 18 dB under
    for i in 0..24 {
        let t = 0.2 + f64::from(i) * 0.05;
        kick_hit(&mut snare, t, -3.0);
        kick_hit(&mut tom, t + 0.0002, -21.0);
        snare_motion.push(t);
        tom_motion.push(t + 0.0002);
    }
    // the kick, played three times away from the snare; the tom mic hears the first two 3 ms
    // later and 18 dB under (the residual lead of the aligned session's kick bleed)
    for t in [5.0, 7.0, 9.0] {
        kick_hit(&mut kick, t, -3.0);
    }
    for t in [5.0, 7.0] {
        kick_hit(&mut tom, t + 0.003, -21.0);
    }
    tom_motion.extend([5.003, 7.003]);
    // the real tom hit: loud, 4 ms after the third kick (clear of the other events)
    tom_hit(&mut tom, 9.004, 0.0);
    tom_motion.push(9.004);
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: tom_motion,
        },
        CrossMic {
            kind: XKind::Snare,
            signal: &snare,
            motion: snare_motion,
        },
        CrossMic {
            kind: XKind::Kick,
            signal: &kick,
            motion: vec![5.0, 7.0, 9.0],
        },
    ];
    let (marks, learned) = cross_bleed(&mics, SR, &CrossBleedParams::default());
    // the snare pair measured its own alignment; the kick pair is sparse and inherits it
    assert!(learned[0].aligned && !learned[0].align_inherited, "{learned:?}");
    assert!(learned[1].aligned, "{learned:?}");
    assert!(learned[1].align_inherited, "{learned:?}");
    assert_eq!(learned[1].align_coins, 3, "{learned:?}");
    // the kick bleed 3 ms after the kick is dropped by level alone (the lead test cannot deny it)
    assert!(marks[0][24].bleed, "{:?}", marks[0][24]);
    assert!(marks[0][25].bleed, "{:?}", marks[0][25]);
    // the loud real hit 4 ms after the kick stays (louder in the tom band than the kick's band)
    assert!(!marks[0][26].bleed, "{:?}", marks[0][26]);
}

#[test]
fn a_sparse_pair_on_an_unaligned_session_is_not_inherited() {
    // #309: with NO pair of the target measured aligned (the snare pair's coins carry the
    // acoustic 1 ms arrival), the session is not aligned and the sparse kick pair keeps the
    // two-part verdict: the quiet bleed 3 ms after the kick is dropped by lead AND level, and a
    // quiet tom hit 2 ms BEFORE the kick stays (it leads).
    let n = (8.0f64 * f64::from(SR)) as usize;
    let mut snare = vec![0f32; n];
    let mut kick = vec![0f32; n];
    let mut tom = vec![0f32; n];
    let mut tom_motion = Vec::new();
    let mut snare_motion = Vec::new();
    for i in 0..24 {
        let t = 0.2 + f64::from(i) * 0.05;
        kick_hit(&mut snare, t, -3.0);
        kick_hit(&mut tom, t + 0.001, -21.0);
        snare_motion.push(t);
        tom_motion.push(t + 0.001);
    }
    kick_hit(&mut kick, 5.0, -3.0);
    kick_hit(&mut tom, 5.003, -21.0); // the kick bleed
    kick_hit(&mut kick, 7.0, -3.0);
    tom_hit(&mut tom, 6.998, -20.0); // the quiet hit 2 ms before the kick
    tom_motion.extend([5.003, 6.998]);
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: tom_motion,
        },
        CrossMic {
            kind: XKind::Snare,
            signal: &snare,
            motion: snare_motion,
        },
        CrossMic {
            kind: XKind::Kick,
            signal: &kick,
            motion: vec![5.0, 7.0],
        },
    ];
    let (marks, learned) = cross_bleed(&mics, SR, &CrossBleedParams::default());
    assert!(!learned[0].aligned, "{learned:?}");
    assert!(!learned[1].aligned, "{learned:?}");
    assert!(!learned[1].align_inherited, "{learned:?}");
    // the two-part verdict still fires on the late quiet bleed ...
    assert!(marks[0][24].bleed, "{:?}", marks[0][24]);
    // ... and denies the leading quiet hit
    assert!(!marks[0][25].bleed, "{:?}", marks[0][25]);
}

#[test]
fn a_non_sparse_pair_of_an_aligned_session_judges_by_level_alone() {
    // #309 verify fix: alignment is a property of the SESSION. The kick pair has 25 coincidences
    // of its own (non-sparse) with a 4 ms median |lead| — it cannot measure its own alignment,
    // but the snare pair of the same tom is measured aligned, so the kick pair judges by the
    // level test alone and the quiet bleed 4 ms BEFORE the kick hit (whose negative lead the
    // two-part verdict would keep — the detector's placement lag, not acoustics) is dropped.
    let n = (4.0f64 * f64::from(SR)) as usize;
    let mut snare = vec![0f32; n];
    let mut kick = vec![0f32; n];
    let mut tom = vec![0f32; n];
    let mut tom_motion = Vec::new();
    let mut snare_motion = Vec::new();
    // 24 snare strokes; the tom mic hears each at the snare's time (aligned) and 18 dB under
    for i in 0..24 {
        let t = 0.2 + f64::from(i) * 0.05;
        kick_hit(&mut snare, t, -3.0);
        kick_hit(&mut tom, t + 0.0002, -21.0);
        snare_motion.push(t);
        tom_motion.push(t + 0.0002);
    }
    // 25 kick strokes away from the snare; the tom mic hears each 4 ms BEFORE the kick hit
    // (the placement lag of the measured session) and 22 dB under
    for i in 0..25 {
        let t = 2.0 + f64::from(i) * 0.05;
        kick_hit(&mut kick, t, -3.0);
        kick_hit(&mut tom, t - 0.004, -25.0);
        tom_motion.push(t - 0.004);
    }
    // the real tom hit: loud, clear of every other event
    tom_hit(&mut tom, 3.5, 0.0);
    tom_motion.push(3.5);
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: tom_motion,
        },
        CrossMic {
            kind: XKind::Snare,
            signal: &snare,
            motion: snare_motion,
        },
        CrossMic {
            kind: XKind::Kick,
            signal: &kick,
            motion: (0..25).map(|i| 2.0 + f64::from(i) * 0.05).collect(),
        },
    ];
    let (marks, learned) = cross_bleed(&mics, SR, &CrossBleedParams::default());
    // the snare pair measured its own alignment; the non-sparse kick pair inherited it
    assert!(learned[0].aligned && !learned[0].align_inherited, "{learned:?}");
    assert!(learned[1].aligned, "{learned:?}");
    assert!(learned[1].align_inherited, "{learned:?}");
    assert_eq!(learned[1].align_coins, 25, "{learned:?}");
    assert!(learned[1].align_med_abs_lead_ms.is_some_and(|m| m > 1.0), "{learned:?}");
    // level alone: every kick bleed is dropped (the lead test cannot keep them)
    for j in 24..49 {
        assert!(marks[0][j].bleed, "hit {j}: {:?}", marks[0][j]);
    }
    // the loud real hit stays
    assert!(!marks[0][49].bleed, "{:?}", marks[0][49]);
}

#[test]
fn a_non_sparse_pair_on_an_unaligned_session_keeps_the_two_part_verdict() {
    // #309: the same non-sparse kick pair (25 coins, 4 ms median |lead|) on a session with NO
    // measured-aligned pair (the snare pair's coins carry a real 1 ms arrival): the two-part
    // verdict stays, so the quiet bleed that leads the kick hit is kept.
    let n = (4.0f64 * f64::from(SR)) as usize;
    let mut snare = vec![0f32; n];
    let mut kick = vec![0f32; n];
    let mut tom = vec![0f32; n];
    let mut tom_motion = Vec::new();
    let mut snare_motion = Vec::new();
    for i in 0..24 {
        let t = 0.2 + f64::from(i) * 0.05;
        kick_hit(&mut snare, t, -3.0);
        kick_hit(&mut tom, t + 0.001, -21.0);
        snare_motion.push(t);
        tom_motion.push(t + 0.001);
    }
    for i in 0..25 {
        let t = 2.0 + f64::from(i) * 0.05;
        kick_hit(&mut kick, t, -3.0);
        kick_hit(&mut tom, t - 0.004, -25.0);
        tom_motion.push(t - 0.004);
    }
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: tom_motion,
        },
        CrossMic {
            kind: XKind::Snare,
            signal: &snare,
            motion: snare_motion,
        },
        CrossMic {
            kind: XKind::Kick,
            signal: &kick,
            motion: (0..25).map(|i| 2.0 + f64::from(i) * 0.05).collect(),
        },
    ];
    let (marks, learned) = cross_bleed(&mics, SR, &CrossBleedParams::default());
    assert!(!learned[0].aligned, "{learned:?}");
    // non-sparse (25 coins) and NOT aligned, inherited or measured
    assert!(!learned[1].aligned, "{learned:?}");
    assert!(!learned[1].align_inherited, "{learned:?}");
    assert_eq!(learned[1].align_coins, 25, "{learned:?}");
    // the two-part verdict: the bleed leads the source (negative lead) and is kept
    for j in 24..49 {
        assert!(!marks[0][j].bleed, "hit {j}: {:?}", marks[0][j]);
    }
}

#[test]
fn the_inherited_alignment_survives_the_prior() {
    // #309: the pass-1 align_inherited travels with the alignment into the re-detection pass —
    // the record does not lie about the inheritance (the same geometry as
    // `the_prior_alignment_is_honoured_in_the_re_detection`, with the pass-1 pair inherited).
    let n = (2.0f64 * f64::from(SR)) as usize;
    let mut kick = vec![0f32; n];
    let mut tom = vec![0f32; n];
    let mut kick_motion = Vec::new();
    let mut tom_motion = Vec::new();
    for i in 0..24 {
        let t = 0.2 + f64::from(i) * 0.05;
        kick_hit(&mut kick, t, -3.0);
        kick_hit(&mut tom, t + 0.001, -21.0);
        kick_motion.push(t);
        tom_motion.push(t + 0.001);
    }
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: tom_motion,
        },
        CrossMic {
            kind: XKind::Kick,
            signal: &kick,
            motion: kick_motion,
        },
    ];
    let p = CrossBleedParams::default();
    let prior = [PairLearned {
        tom: 0,
        source: 1,
        lead_ms: None,
        ratio_db: None,
        aligned: true,
        align_coins: 24,
        align_med_abs_lead_ms: Some(1.0),
        aligned_counts: [0; 4],
        align_inherited: true,
    }];
    let (marks2, learned2) = cross_bleed_prior(&mics, SR, &p, Some(&prior));
    assert!(learned2[0].aligned, "{learned2:?}");
    assert!(learned2[0].align_inherited, "{learned2:?}");
    assert_eq!(learned2[0].align_coins, 24);
    // level alone: every coinciding hit is bleed
    for (j, m) in marks2[0].iter().enumerate() {
        assert!(m.bleed, "hit {j}: {m:?}");
    }
}
