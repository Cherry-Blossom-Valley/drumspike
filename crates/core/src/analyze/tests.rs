use super::*;
use crate::detect::resample::ResampleError;
use crate::refine::refine_sr;

const SR: u32 = 48_000;

/// tests/helpers.kick_hit: pitch sweep 125 -> 55 Hz, 0.5 ms attack ramp, 0.12 s decay.
fn track(onsets: &[(f64, f64)], dur: f64, sr: u32) -> Vec<f32> {
    let srf = f64::from(sr);
    let mut x = vec![0.0f64; (dur * srf) as usize];
    for &(t0, db) in onsets {
        let i0 = (t0 * srf).round() as usize;
        let g = 10f64.powf(db / 20.0);
        let mut cum = 0.0;
        for k in 0..((0.4 * srf) as usize) {
            if i0 + k >= x.len() {
                break;
            }
            let t = k as f64 / srf;
            cum += 55.0 + 70.0 * (-t / 0.03).exp();
            let env = (t / 0.0005).min(1.0) * (-t / 0.12).exp();
            x[i0 + k] += (2.0 * std::f64::consts::PI * cum / srf).sin() * env * g;
        }
    }
    x.iter().map(|&v| v as f32).collect()
}

const ONSETS: [f64; 6] = [0.25, 0.75, 1.25, 1.75, 2.25, 2.75];

fn kicks(sr: u32) -> Vec<f32> {
    let o: Vec<(f64, f64)> = ONSETS.iter().map(|&t| (t, -3.0)).collect();
    track(&o, 3.5, sr)
}

/// The Python composition of engine.analyze_track, written out with the ported pieces
/// one call at a time (the reference of the unit tests).
fn reference(x: &[f32], sr: u32, role: &str, detected: &[(f64, u8)], p: &AnalyzeParams) -> Vec<Hit> {
    let mut ev: Vec<(f64, u8)> = detected.to_vec();
    if role != "other" {
        ev.retain(|(_, n)| role_classes(role).contains(n));
    }
    ev.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
    let mingap = p.min_gap_ms / 1000.0;
    let (mut dedup, mut last) = (vec![], -9e9);
    for (t, _) in ev {
        if t - last >= mingap {
            dedup.push(t);
            last = t;
        }
    }
    let mut pairs: Vec<(f64, Option<f64>)> = dedup.iter().map(|&t| (t, None)).collect();
    if p.refine {
        pairs.clear();
        for (i, &t) in dedup.iter().enumerate() {
            let mut nb = vec![];
            if i > 0 {
                nb.push(dedup[i - 1]);
            }
            if i + 1 < dedup.len() {
                nb.push(dedup[i + 1]);
            }
            pairs.push(refine_sr(x, t, sr, &nb, p.window));
        }
        pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let (mut fin, mut last) = (vec![], -9e9);
        for (t, r) in pairs {
            if t - last >= mingap {
                fin.push((t, r));
                last = t;
            }
        }
        pairs = fin;
    }
    if p.shift_samples != 0 {
        pairs = pairs
            .iter()
            .map(|&(t, r)| (t + p.shift_samples as f64 / sr as f64, r))
            .collect();
    }
    let v = velocity_map(x, sr, &pairs.iter().map(|p| p.0).collect::<Vec<_>>(), &p.velocity);
    pairs
        .iter()
        .zip(v)
        .map(|(&(t, r), v)| Hit {
            t,
            v,
            rise: r.map(|r| py_round_digits(r, 1)),
        })
        .collect()
}

#[test]
fn role_classes_and_notes_follow_the_engine() {
    assert_eq!(role_classes("kick"), &[35]);
    assert_eq!(role_classes("snare"), &[38]);
    assert_eq!(role_classes("tom"), &[41, 43, 45, 47, 48, 50]);
    assert_eq!(role_classes("hat"), &[42, 44, 46]);
    assert_eq!(role_classes("cymbal"), &[49, 51, 52, 53, 55]);
    let other: Vec<u8> = (28..60).collect();
    assert_eq!(role_classes("other"), other.as_slice());
    assert_eq!(role_classes("bass"), other.as_slice());
    let notes: Vec<u8> = ["kick", "snare", "tom", "hat", "cymbal", "other", "bass"]
        .iter()
        .map(|r| role_note(r))
        .collect();
    assert_eq!(notes, [35, 38, 47, 42, 49, 35, 35]);
    // every role keeps its own DSP note
    for r in ["kick", "snare", "tom", "hat", "cymbal", "other", "bass"] {
        assert!(role_classes(r).contains(&role_note(r)), "{r}");
    }
}

#[test]
fn thresholds_round_to_3_decimals_like_the_cli_format() {
    // float(f"{t:.3f}") in Python 3.12
    let t = round_thresholds(&[0.2225, 0.0005, 0.2345, 0.9995, 0.30000000000000004]);
    assert_eq!(t, [0.223, 0.001, 0.234, 1.0, 0.3]);
    assert_eq!(
        round_thresholds(&[0.125, 0.1225, 0.2215, 0.0015, 0.0025]),
        [0.125, 0.122, 0.222, 0.002, 0.003]
    );
    assert_eq!(
        round_thresholds(&crate::detect::peaks::THRESHOLDS),
        crate::detect::peaks::THRESHOLDS
    );
}

#[test]
fn a_rate_outside_the_resampler_is_an_error() {
    let x = vec![0.0f32; 22_050];
    let p = AnalyzeParams::default();
    assert_eq!(
        analyze_detected(&x, 22_050, "kick", &[], &p),
        Err(AnalyzeError::Rate(22_050))
    );
    assert_eq!(
        analyze_track(&x, 22_050, "kick", &p, Detector::Dsp),
        Err(AnalyzeError::Rate(22_050))
    );
    let e = detect_events(&x, 32_000, "kick", &p.thresholds, Detector::Dsp).unwrap_err();
    assert!(e.to_string().contains("32000"), "{e}");
    let e: AnalyzeError = DetectError::Resample(ResampleError::UnsupportedRate(1)).into();
    assert!(matches!(e, AnalyzeError::Detect(_)));
    let e: AnalyzeError = OnsetError::Resample(ResampleError::UnsupportedRate(1)).into();
    assert!(e.to_string().starts_with("onset:"), "{e}");
}

#[test]
fn analyze_detected_equals_the_step_by_step_reference_in_every_branch() {
    let x = kicks(SR);
    // a kick close to each onset, a snare (filtered for kick), a near duplicate
    // (first dedup) and two hits that refine onto one attack (second dedup)
    let mut det: Vec<(f64, u8)> = vec![];
    for &t in &ONSETS {
        if t == 1.25 {
            // two coarse hits 20 ms apart around one attack: the first refines forward
            // onto it, so the second dedup drops the other one
            det.push((t - 0.0115, 35));
            det.push((t + 0.0085, 35));
        } else {
            det.push((t - 0.006, 35));
        }
        det.push((t + 0.4, 38));
    }
    det.push((0.75 - 0.006 + 0.005, 35)); // 5 ms after a kick: dropped by the first dedup
    det.reverse(); // the detector's order does not matter
    let cases = [
        ("kick", AnalyzeParams::default()),
        ("other", AnalyzeParams::default()),
        ("bass", AnalyzeParams::default()),
        (
            "kick",
            AnalyzeParams {
                refine: false,
                ..AnalyzeParams::default()
            },
        ),
        (
            "kick",
            AnalyzeParams {
                shift_samples: -37,
                min_gap_ms: 12.0,
                window: RefineWindow::SYMMETRIC_7,
                velocity: VelocityMap {
                    lo_db: -50.0,
                    hi_db: -3.0,
                    ..VelocityMap::default()
                },
                ..AnalyzeParams::default()
            },
        ),
        (
            "snare",
            AnalyzeParams {
                shift_samples: 480,
                ..AnalyzeParams::default()
            },
        ),
    ];
    let mut total = AnalyzeBranches::default();
    for (role, p) in &cases {
        let a = analyze_detected(&x, SR, role, &det, p).unwrap();
        assert_eq!(a.events, reference(&x, SR, role, &det, p), "{role} {p:?}");
        assert!(a.events.windows(2).all(|w| w[0].t <= w[1].t));
        total.add(&a.branches);
    }
    let get = |name: &str| total.pairs().iter().find(|(n, _)| *n == name).unwrap().1;
    assert_eq!(get("tracks"), cases.len());
    for name in [
        "filter_runs",
        "filtered_out",
        "dedup_dropped",
        "refine_off",
        "refine_moved",
        "dedup2_dropped",
        "shifted",
    ] {
        assert!(get(name) > 0, "{name}: {total:?}");
    }
    // the kick role: six kicks on their attacks, rise rounded to 1 decimal
    let a = analyze_detected(&x, SR, "kick", &det, &AnalyzeParams::default()).unwrap();
    assert_eq!(a.events.len(), ONSETS.len());
    for (e, t) in a.events.iter().zip(ONSETS) {
        assert!((e.t - t).abs() <= 0.0005, "{e:?} {t}");
        let r = e.rise.unwrap();
        assert_eq!(r, (r * 10.0).round() / 10.0);
        assert!((20..=127).contains(&e.v));
    }
    assert_eq!(a.branches.filtered_out, ONSETS.len());
    assert_eq!(a.branches.dedup_dropped, 1);
    assert_eq!(a.branches.dedup2_dropped, 1);
    // refine off: the coarse times stay and no event has a rise
    let p = AnalyzeParams {
        refine: false,
        ..AnalyzeParams::default()
    };
    let a = analyze_detected(&x, SR, "kick", &det, &p).unwrap();
    assert!(a.events.iter().all(|e| e.rise.is_none()));
    assert_eq!(a.events[0].t, 0.25 - 0.006);
    // nothing detected: no events
    let a = analyze_detected(&x, SR, "kick", &[], &AnalyzeParams::default()).unwrap();
    assert!(a.events.is_empty() && a.branches.events == 0);
}

#[test]
fn the_reach_rule_moves_only_the_hits_whose_window_missed_the_attack() {
    // P4-12: the kicks over a -70 dB floor; every other coarse time 17 ms before its attack
    // (the DSP frame before it), the others 6 ms before
    let mut s: u64 = 7;
    let x: Vec<f32> = kicks(SR)
        .iter()
        .map(|&v| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            v + (3e-4 * ((s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0)) as f32
        })
        .collect();
    let det: Vec<(f64, u8)> = ONSETS
        .iter()
        .enumerate()
        .map(|(i, &t)| (if i % 2 == 0 { t - 0.017 } else { t - 0.006 }, 35))
        .collect();
    let old = analyze_detected(&x, SR, "kick", &det, &AnalyzeParams::default()).unwrap();
    let reach = AnalyzeParams {
        rule: RefineRule::Reach,
        ..AnalyzeParams::default()
    };
    let new = analyze_detected(&x, SR, "kick", &det, &reach).unwrap();
    let moved = |b: &AnalyzeBranches| (b.reach_moved, b.tail_moved, b.reach_unresolved);
    assert_eq!(moved(&old.branches), (0, 0, 0), "the engine's rule");
    assert_eq!(moved(&new.branches), (3, 0, 0));
    assert_eq!((old.events.len(), new.events.len()), (6, 6));
    for (i, ((a, b), t)) in old.events.iter().zip(&new.events).zip(ONSETS).enumerate() {
        assert!((b.t - t).abs() <= 0.0005, "{i}: {b:?} {t}");
        if i % 2 == 0 {
            assert!(a.t < t - 0.01, "the engine's rule lands on the floor: {i} {a:?}");
        } else {
            assert_eq!(a, b, "{i}: a window that holds the attack is the engine's");
        }
    }
    let names: Vec<&str> = new.branches.pairs().iter().map(|(n, _)| *n).collect();
    assert!(names.contains(&"reach_moved") && names.contains(&"tail_moved") && names.contains(&"reach_unresolved"));
    // P4-12 stage 2: version 3 moves every hit back from version 2's 25 % point to its first
    // motion, never later, and keeps the velocities' hits and the count
    let first = AnalyzeParams {
        rule: RefineRule::FirstMotion,
        ..AnalyzeParams::default()
    };
    let three = analyze_detected(&x, SR, "kick", &det, &first).unwrap();
    let backs = |b: &AnalyzeBranches| (b.back_moved, b.back_busy, b.back_separate);
    assert_eq!(backs(&new.branches), (0, 0, 0), "version 2 has no back-track");
    assert_eq!(backs(&three.branches), (6, 0, 0));
    assert_eq!(moved(&three.branches), (3, 0, 0), "the reach runs first");
    assert_eq!(three.events.len(), 6);
    for (i, ((b, c), t)) in new.events.iter().zip(&three.events).zip(ONSETS).enumerate() {
        let (on, two, now) = (
            (t * 48_000.0).round(),
            (b.t * 48_000.0).round(),
            (c.t * 48_000.0).round(),
        );
        assert!(
            now < two && now >= on && now - on <= 8.0,
            "{i}: {now} against {on} (version 2 {two})"
        );
        assert_eq!(c.rise, b.rise, "{i}");
    }
    assert!(names.contains(&"back_moved") && names.contains(&"back_busy") && names.contains(&"back_separate"));
    let mut sum = AnalyzeBranches::default();
    sum.add(&three.branches);
    sum.add(&three.branches);
    assert_eq!(backs(&sum), (12, 0, 0));
}

#[test]
fn equal_times_sort_by_note_and_the_dedup_keeps_the_first() {
    let x = kicks(SR);
    let det = [(0.244, 38), (0.244, 35)];
    let p = AnalyzeParams {
        refine: false,
        ..AnalyzeParams::default()
    };
    let a = analyze_detected(&x, SR, "other", &det, &p).unwrap();
    assert_eq!(a.events.len(), 1);
    assert_eq!(a.branches.dedup_dropped, 1);
    assert_eq!(a.branches.filter_runs, 0);
}

#[test]
fn the_dsp_fallback_finds_every_kick_and_carries_the_role_note() {
    for sr in DETECT_RATES {
        let x = kicks(sr);
        let det = detect_events(&x, sr, "snare", &crate::detect::peaks::THRESHOLDS, Detector::Dsp).unwrap();
        assert!(det.iter().all(|&(_, n)| n == 38), "{det:?}");
        let a = analyze_track(&x, sr, "kick", &AnalyzeParams::default(), Detector::Dsp).unwrap();
        assert_eq!(a.events.len(), ONSETS.len(), "{sr}: {:?}", a.events);
        for (e, t) in a.events.iter().zip(ONSETS) {
            assert!((e.t - t).abs() <= 0.0005, "{sr}: {e:?} {t}");
        }
        assert_eq!(a.branches.filtered_out, 0);
        // an unknown role runs the DSP fallback with the parameters of `other`
        let u = detect_events(&x, sr, "bass", &crate::detect::peaks::THRESHOLDS, Detector::Dsp).unwrap();
        let o = detect_events(&x, sr, "other", &crate::detect::peaks::THRESHOLDS, Detector::Dsp).unwrap();
        assert_eq!(u, o);
    }
}

#[test]
fn analyze_flags_carries_the_out_flag_through_the_dedup_and_the_refinement() {
    let x = kicks(SR);
    // the kicks a few ms after each onset (detector times), one duplicate inside the min gap, the
    // second and the fifth taken out by the Learn filter
    let det: Vec<(f64, u8)> = ONSETS.iter().map(|&t| (t + 0.006, 35)).chain([(0.256, 35)]).collect();
    let out = [false, true, false, false, true, false, true];
    let p = AnalyzeParams::default();
    let a = analyze_flags(&x, SR, "kick", &det, &[], &out, &p).unwrap();
    let plain = analyze_detected(&x, SR, "kick", &det, &p).unwrap();
    assert_eq!(a.events, plain.events, "the flags never change the hits");
    assert_eq!(a.events.len(), ONSETS.len());
    let flagged: Vec<usize> = (0..a.events.len()).filter(|&i| a.is_learn_out(i)).collect();
    assert_eq!(
        flagged,
        vec![1, 4],
        "the flag follows its hit; the dropped duplicate's flag goes with it"
    );
    assert!(a.roll.is_empty() && plain.learn_out.is_empty() && !plain.is_learn_out(1));
    // planted fault: no flag at all gives an empty vector, not a list of false
    let none = analyze_flags(&x, SR, "kick", &det, &[], &[false; 7], &p).unwrap();
    assert!(none.learn_out.is_empty());
}

#[test]
fn detect_hits_then_analyze_flags_is_analyze_track_and_keeps_the_roll_flags() {
    let x = kicks(SR);
    let p = AnalyzeParams::default();
    let (det, roll) = detect_hits(&x, SR, "kick", &p, Detector::Dsp).unwrap();
    assert!(!det.is_empty() && roll.is_empty());
    let a = analyze_flags(&x, SR, "kick", &det, &roll, &[], &p).unwrap();
    assert_eq!(a, analyze_track(&x, SR, "kick", &p, Detector::Dsp).unwrap());
    // the roll flags ride along with the out flags (planted fault: a roll flag that went missing)
    let roll = vec![true; det.len()];
    let out = vec![false; det.len()];
    let b = analyze_flags(&x, SR, "kick", &det, &roll, &out, &p).unwrap();
    assert!((0..b.events.len()).all(|i| b.is_roll(i) && !b.is_learn_out(i)));
}

#[test]
fn the_kick_walk_version_no_longer_changes_anything() {
    // #281 D16: the kick walk moved with the model to the model branch; every version runs
    // the shipped back-track margin, on the kick like on every other role.
    let on = 24_000usize;
    let mut x = vec![0.0f32; 48_000];
    for k in 0..144 {
        x[on - 144 + k] += (0.028 * (2.0 * std::f64::consts::PI * 900.0 * k as f64 / 48_000.0).sin()) as f32;
    }
    for k in 0..14_400 {
        let t = k as f64 / 48_000.0;
        x[on + k] += ((-t / 0.06).exp() * (2.0 * std::f64::consts::PI * 180.0 * t + 0.7).sin()) as f32;
    }
    let p = |kick_walk| AnalyzeParams {
        rule: RefineRule::FirstMotion,
        kick_walk,
        ..AnalyzeParams::default()
    };
    let run =
        |role: &str, note: u8, kick_walk| analyze_detected(&x, 48_000, role, &[(0.5, note)], &p(kick_walk)).unwrap();
    let (k0, k1) = (run("kick", 35, 0), run("kick", 35, 1));
    assert_eq!((k0.events.len(), k1.events.len()), (1, 1));
    assert_eq!(k0.events, k1.events, "the kick walk version is inert");
    let (s0, s1) = (run("snare", 38, 0), run("snare", 38, 1));
    assert_eq!(s0.events, s1.events);
    assert_eq!(s0.events[0].t, k0.events[0].t, "the same shipped walk everywhere");
    assert_eq!(AnalyzeParams::default().kick_walk, 0);
}
