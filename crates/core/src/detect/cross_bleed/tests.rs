//! Tests of the cross-track bleed rule (#281 A6): a planted kick bleed on a tom track is dropped, a
//! real tom hit 10 ms after a kick is kept.

use super::{CrossBleedParams, CrossMic, PairLearned, XKind, candidate_motion, cross_bleed, cross_bleed_prior};
use crate::refine::{RefineRule, RefineWindow, first_motion};

const SR: u32 = 48_000;

/// A stick click (2 ms of noise, fixed sequence) — the broadband attack the version 6 edge
/// search refines onto, as in the app's own test bursts.
fn click(x: &mut [f32], at: usize, amp: f32) {
    let mut s = 99u64;
    for k in 0..((0.002 * f64::from(SR)) as usize).min(x.len().saturating_sub(at)) {
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let noise = ((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0;
        x[at + k] += (noise * (k as f64 / f64::from(SR) / 0.002).exp()) as f32 * amp;
    }
}

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

/// The pass-2 motion the app used before the verify fix round 2: the bare [`first_motion`] walk
/// of the candidate times (app `refined_motion`, no `prev` bound).
fn walk_motion(x: &[f32], sr: u32, ts: &[f64]) -> Vec<f64> {
    ts.iter()
        .map(|&t| {
            let s = (t * f64::from(sr)).round() as i64;
            let (foot, _) = first_motion(x, s, sr, t, None, None);
            foot as f64 / f64::from(sr)
        })
        .collect()
}

#[test]
fn the_pass_two_candidates_are_refined_with_the_stored_rule_not_the_bare_walk() {
    // #296 verify round 2: on an aligned session the re-detection's candidate times sit on the
    // detector's grid, up to a grid step from the true onset. The bare first-motion walk (the
    // old pass-2 motion) runs back over the slow rise / the previous stroke's decay that merges
    // into the bleed response — on a bleed it ends at the candidate (BackStep::Busy) or on the
    // rise — and lands more than `coincide_ms` before the snare hit, so the pass-2 coincidence
    // lookup finds nothing and the bleed candidate STAYS (the coordinator's record: kept
    // candidates with no coinciding source hit at all, `dropped` leads of 0.7 to 4.5 ms where
    // pass 1's median |lead| is 0.06 ms). The stored rule (version 6) refines onto the attack,
    // so both passes see the same times and the bleed is dropped.
    let n = (9.0f64 * f64::from(SR)) as usize;
    let mut snare = vec![0f32; n];
    let mut tom = vec![0f32; n];
    let mut snare_motion = Vec::new();
    // 24 snare strokes 0.3 s apart; the tom mic hears each one 18 dB under at the SAME time
    // (aligned), after a much louder stroke 0.1 s before whose decay merges into the response
    let mut cands = Vec::new();
    for i in 0..24 {
        let t = 0.2 + f64::from(i) * 0.3;
        kick_hit(&mut snare, t, -3.0);
        snare_motion.push(t);
        kick_hit(&mut tom, t - 0.1, -3.0);
        let on = (t * f64::from(SR)).round() as usize;
        kick_hit(&mut tom, t, -21.0);
        click(&mut tom, on, 0.045);
        // the detector's grid: the candidate sits 11 ms before the response's onset
        cands.push(t - 0.011);
    }
    // one real tom stroke, loud, away from every snare hit
    tom_hit(&mut tom, 7.5, 0.0);
    cands.push(7.5 - 0.011);
    let mics_move = |motion: Vec<f64>| {
        [
            CrossMic {
                kind: XKind::Tom,
                signal: &tom,
                motion,
            },
            CrossMic {
                kind: XKind::Snare,
                signal: &snare,
                motion: snare_motion.clone(),
            },
        ]
    };
    // the pass-1 decision: the pair aligned with 24 coins (as on the stored hits)
    let prior = [PairLearned {
        tom: 0,
        source: 1,
        lead_ms: None,
        ratio_db: None,
        aligned: true,
        align_coins: 24,
        align_med_abs_lead_ms: Some(0.06),
        aligned_counts: [0; 4],
    }];
    // the OLD pass-2 motion (the bare walk): the bleed candidates land about 11 ms before the
    // snare hits — outside `coincide_ms` — so no source hit coincides and NOTHING is dropped
    let walked = walk_motion(&tom, SR, &cands);
    for (w, &c) in walked.iter().zip(&cands).take(24) {
        assert!(
            (w - c).abs() < 1e-9,
            "the walk keeps the grid candidate (busy): {w} vs {c}"
        );
        assert!(
            snare_motion.iter().all(|&s| (s - w).abs() > 0.010 + 1e-9),
            "the walked time {w} is more than coincide_ms from every snare hit"
        );
    }
    let (marks_walk, _) = cross_bleed_prior(&mics_move(walked), SR, &CrossBleedParams::default(), Some(&prior));
    assert!(
        marks_walk[0].iter().all(|m| !m.bleed),
        "the old pass-2 motion keeps every bleed candidate: {:?}",
        marks_walk[0]
    );
    // the NEW pass-2 motion (the stored rule): the candidates refine onto the attacks, every
    // bleed candidate coincides with its snare hit and is dropped by level alone; the real
    // stroke stays. The refine bounds (round 3): the source hits within coincide_ms of each
    // candidate — here none (the candidates sit 11 ms before the snare hits).
    let bounds: Vec<Vec<f64>> = cands
        .iter()
        .map(|&c| {
            snare_motion
                .iter()
                .filter(|&&s| (s - c).abs() <= 0.010 + 1e-9)
                .copied()
                .collect()
        })
        .collect();
    let ruled = candidate_motion(&tom, SR, &cands, RefineWindow::DEFAULT, RefineRule::Edge, true, &bounds);
    for (r, &t) in ruled.iter().zip(&snare_motion).take(24) {
        assert!(
            (r - t).abs() <= 0.010 + 1e-9,
            "the ruled time {r} coincides with the snare hit {t}"
        );
    }
    let (marks, _) = cross_bleed_prior(&mics_move(ruled), SR, &CrossBleedParams::default(), Some(&prior));
    for (j, m) in marks[0].iter().enumerate() {
        assert_eq!(m.bleed, j < 24, "candidate {j}: {m:?}");
    }
    // the fix changes the outcome
    assert_ne!(
        marks_walk[0].iter().filter(|m| m.bleed).count(),
        marks[0].iter().filter(|m| m.bleed).count()
    );
}

#[test]
fn a_loud_tom_hit_near_a_snare_hit_survives_the_candidate_refine() {
    // #296 verify round 3: on an UNALIGNED pair (the two-part-verdict geometry) a loud real
    // tom hit a few ms after a snare hit vanished in the re-detection: the candidate's refine
    // walk ran back over the snare bleed on the tom mic (louder than the floor) onto the
    // bleed's edge — the snare's time — so the candidate landed on the snare hit and collapsed
    // with, or was judged as, the bleed. The stored hits' refine has a `prev` bound (the
    // midpoint to the previous hit); the candidates get the same, including a coinciding
    // source hit's time: the walk never crosses the snare hit and the loud stroke survives the
    // re-detection at its own time, as it did under rule 3. And two candidates that refine to
    // one time keep the louder one: the loud candidate does not take the bleed candidate's
    // refined time.
    let n = (9.0f64 * f64::from(SR)) as usize;
    let mut snare = vec![0f32; n];
    let mut tom = vec![0f32; n];
    let mut snare_motion = Vec::new();
    let mut tom_motion = Vec::new();
    let mut cands = Vec::new();
    // 24 snare strokes 0.3 s apart; the tom mic hears each one 14 dB under, 1 ms later (the
    // acoustic arrival — the pair is NOT aligned); the grid candidates sit 11 ms early, as in
    // `the_pass_two_candidates_are_refined_with_the_stored_rule_not_the_bare_walk`
    for i in 0..24 {
        let t = 0.2 + f64::from(i) * 0.3;
        kick_hit(&mut snare, t, -6.0);
        kick_hit(&mut tom, t + 0.001, -20.0);
        // the broadband attack of the bleed response (as in the round-2 test): a sharp edge the
        // version-6 edge search can take
        click(&mut tom, ((t + 0.001) * f64::from(SR)).round() as usize, 0.09);
        snare_motion.push(t);
        tom_motion.push(t + 0.001);
        cands.push(t - 0.011);
    }
    // the loud real tom hit 4 ms after the last snare hit (the tom mic 2 dB louder than the
    // snare mic's level at its hit — the level test keeps it); the snare mic hears it 10 dB
    // under. The grid candidate sits 1 ms early.
    let ts = snare_motion[23];
    let tl = ts + 0.004;
    tom_hit(&mut tom, tl, -4.0);
    tom_hit(&mut snare, tl + 0.001, -14.0);
    tom_motion.push(tl);
    cands.push(tl - 0.001);
    let p = CrossBleedParams::default();
    let mics = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: tom_motion.clone(),
        },
        CrossMic {
            kind: XKind::Snare,
            signal: &snare,
            motion: snare_motion.clone(),
        },
    ];
    // pass 1 (the stored hits): the loud hit stays — the two-part verdict, the tom mic louder
    let (marks1, learned) = cross_bleed(&mics, SR, &p);
    assert!(!learned[0].aligned, "{:?}", learned[0]);
    assert!(!marks1[0][24].bleed, "pass 1: {:?}", marks1[0][24]);
    // the refine bounds: every source hit within coincide_ms of the candidate (as the app
    // hands them in)
    let bounds: Vec<Vec<f64>> = cands
        .iter()
        .map(|&c| {
            snare_motion
                .iter()
                .filter(|&&s| (s - c).abs() <= p.coincide_ms / 1000.0 + 1e-9)
                .copied()
                .collect()
        })
        .collect();
    let ruled = candidate_motion(&tom, SR, &cands, RefineWindow::DEFAULT, RefineRule::Edge, true, &bounds);
    let last = ruled.len() - 1;
    // the loud candidate keeps its own attack: not on the snare hit, not on the bleed
    // candidate's refined time (no collapse — the louder one keeps its time)
    assert!(
        ruled[last] > ruled[23] + 0.002,
        "the loud candidate collapsed onto the bleed candidate's time: loud {} vs bleed {}",
        ruled[last],
        ruled[23]
    );
    assert!(
        (ruled[last] - tl).abs() <= 0.0025,
        "the loud candidate left its own attack: {} vs {tl}",
        ruled[last]
    );
    // and it survives the re-detection verdict, as in pass 1; the bleed candidate is dropped
    let mics2 = [
        CrossMic {
            kind: XKind::Tom,
            signal: &tom,
            motion: ruled,
        },
        CrossMic {
            kind: XKind::Snare,
            signal: &snare,
            motion: snare_motion,
        },
    ];
    let (marks2, _) = cross_bleed_prior(&mics2, SR, &p, Some(&learned));
    assert!(
        !marks2[0][last].bleed,
        "pass 2 drops the loud hit: {:?}",
        marks2[0][last]
    );
}
