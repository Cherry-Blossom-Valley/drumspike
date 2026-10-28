use super::*;

const SR: u32 = 48_000;
const W: RefineWindow = RefineWindow::DEFAULT;

/// tests/helpers.kick_hit: pitch sweep 125 -> 55 Hz, attack ramp, exponential decay.
fn kick_hit(n: usize, attack_ms: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(n);
    let mut cum = 0.0;
    for i in 0..n {
        let t = i as f64 / SR as f64;
        let f = 55.0 + 70.0 * (-t / 0.03).exp();
        cum += f;
        let ph = 2.0 * std::f64::consts::PI * cum / SR as f64;
        let env = (t / (attack_ms / 1000.0)).min(1.0) * (-t / 0.12).exp();
        out.push(ph.sin() * env);
    }
    out
}

fn track(onsets: &[(f64, f64)], dur: f64) -> Vec<f32> {
    let mut x = vec![0.0f64; (dur * SR as f64) as usize];
    for &(t, db) in onsets {
        let i = (t * SR as f64).round() as usize;
        let h = kick_hit((0.4 * SR as f64) as usize, 0.5);
        let g = 10f64.powf(db / 20.0);
        for (k, v) in h.iter().enumerate() {
            if i + k < x.len() {
                x[i + k] += v * g;
            }
        }
    }
    x.iter().map(|&v| v as f32).collect()
}

#[test]
fn log_envelope_is_20_log10_of_the_smoothed_envelope() {
    // silence: 20 * log10(0 + 1e-9) = -180 dB in every sample
    assert!(log_envelope(&[0.0; 16], 3).iter().all(|&v| v == 20.0 * 1e-9f64.log10()));
    assert!(log_envelope(&[], 1).is_empty());
    // a cosine of amplitude 0.5 over whole periods: a flat envelope of 0.5 (-6.02 dB)
    let n = 480;
    let seg: Vec<f64> = (0..n)
        .map(|i| 0.5 * (2.0 * std::f64::consts::PI * 10.0 * i as f64 / n as f64).cos())
        .collect();
    let flat = 20.0 * (0.5f64 + 1e-9).log10();
    let e1 = log_envelope(&seg, 1);
    assert_eq!(e1.len(), n);
    assert!(e1.iter().all(|v| (v - flat).abs() < 1e-8));
    // k_sm = 3: np.convolve(mode="same") pads with zeros, so the first and the last
    // value average two of three envelope samples; the rest stay flat
    let e3 = log_envelope(&seg, 3);
    assert_eq!(e3.len(), n);
    let edge = 20.0 * (0.5f64 * 2.0 / 3.0 + 1e-9).log10();
    assert!((e3[0] - edge).abs() < 1e-8 && (e3[n - 1] - edge).abs() < 1e-8);
    assert!(e3[1..n - 1].iter().all(|v| (v - flat).abs() < 1e-8));
    // the same as the steps written out
    let env = hilbert_env(&seg);
    let sm = convolve_same(&env, &[1.0 / 3.0; 3]);
    let want: Vec<f64> = sm.iter().map(|v| 20.0 * (v + 1e-9).log10()).collect();
    assert_eq!(e3, want);
}

#[test]
fn hilbert_env_of_a_cosine_is_flat_and_of_silence_is_zero() {
    let n = 480;
    let seg: Vec<f64> = (0..n)
        .map(|i| (2.0 * std::f64::consts::PI * 10.0 * i as f64 / n as f64).cos())
        .collect();
    let env = hilbert_env(&seg);
    assert_eq!(env.len(), n);
    for v in &env {
        assert!((v - 1.0).abs() < 1e-9, "{v}");
    }
    assert!(hilbert_env(&[0.0; 9]).iter().all(|&v| v == 0.0));
    assert!(hilbert_env(&[]).is_empty());
    // odd length: the h mask covers 1..=(n-1)/2
    let odd: Vec<f64> = (0..101)
        .map(|i| (2.0 * std::f64::consts::PI * 3.0 * i as f64 / 101.0).cos())
        .collect();
    for v in hilbert_env(&odd) {
        assert!((v - 1.0).abs() < 1e-9, "{v}");
    }
}

#[test]
fn convolve_same_matches_numpy_centering() {
    // np.convolve(np.arange(1, 4), np.ones(2) / 2, "same") == [0.5, 1.5, 2.5]
    assert_eq!(convolve_same(&[1.0, 2.0, 3.0], &[0.5, 0.5]), vec![0.5, 1.5, 2.5]);
    // kernel 3: [1*0+... ] np.convolve([1,2,3,4], [1,1,1], "same") == [3, 6, 9, 7]
    assert_eq!(
        convolve_same(&[1.0, 2.0, 3.0, 4.0], &[1.0; 3]),
        vec![3.0, 6.0, 9.0, 7.0]
    );
    // kernel longer than the signal: numpy swaps and returns len(kernel) values
    // np.convolve([1,2], [1,1,1], "same") == [1, 3, 3]
    assert_eq!(convolve_same(&[1.0, 2.0], &[1.0; 3]), vec![1.0, 3.0, 3.0]);
    // 8 samples, kernel 9 (the 48 kHz smoothing on the shortest cut): 9 values, offset 3
    let out = convolve_same(&[1.0; 8], &[1.0; 9]);
    assert_eq!(out, vec![4.0, 5.0, 6.0, 7.0, 8.0, 8.0, 7.0, 6.0, 5.0]);
    assert!(convolve_same(&[], &[1.0]).is_empty());
}

#[test]
fn refine_finds_the_attack_of_an_isolated_hit() {
    let x = track(&[(0.5, -3.0)], 1.5);
    let (t, rise) = refine_sr(&x, 0.5 + 0.003, SR, &[], W);
    let i = (t * SR as f64).round() as i64;
    // the 25 % rule lands inside the 0.5 ms attack ramp of the sine sweep
    assert!((24_000..=24_048).contains(&i), "refined to {i}");
    assert!(rise.unwrap() > 6.0, "{rise:?}");
    // the result is a sample index over sr
    assert_eq!(t, i as f64 / SR as f64);
}

#[test]
fn refine_returns_t0_for_silence_and_short_cuts() {
    let x = vec![0.0f32; SR as usize];
    let (t, rise) = refine_sr(&x, 0.5, SR, &[], W);
    assert_eq!(t, 0.5);
    assert!(rise.is_some(), "silence has an envelope, the 25 % rule bails out");
    assert_eq!(
        refine_sr(&x[..5], 0.0, SR, &[], W),
        (0.0, None),
        "cut shorter than 8 samples"
    );
    assert_eq!(refine_sr(&x, -5.0, SR, &[], W), (-5.0, None), "t0 before the file");
    assert_eq!(refine_sr(&x, 5.0, SR, &[], W), (5.0, None), "t0 after the file");
}

#[test]
fn neighbours_clip_the_search_window() {
    // two hits 25 ms apart: the second refines to its own attack, not the first one
    let x = track(&[(0.5, -3.0), (0.525, -3.0)], 1.5);
    let (t2, _) = refine_sr(&x, 0.525 + 0.002, SR, &[0.5 + 0.002], W);
    assert!((t2 - 0.525).abs() < 0.0005, "{t2}");
    // an empty window (neighbours at the same time on both sides) returns t0
    assert_eq!(
        refine_sr(&x, 0.5250001, SR, &[0.52500005, 0.52500015], W),
        (0.5250001, None)
    );
    // refine_track passes the previous and the next coarse hit
    let r = refine_track(&x, SR, &[0.502, 0.527], W);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0], refine_sr(&x, 0.502, SR, &[0.527], W));
    assert_eq!(r[1], refine_sr(&x, 0.527, SR, &[0.502], W));
}

#[test]
fn the_window_reaches_an_attack_12_ms_after_t0() {
    // the detector is early: an attack 12 ms after t0 is outside -7..+7 (and outside the
    // 10 ms reach of the 25 % rule behind the anchor) but at the end of -2..+12
    let x = track(&[(0.5, -3.0)], 1.5);
    let t0 = 0.488;
    let (t, _) = refine_sr(&x, t0, SR, &[], RefineWindow::DEFAULT);
    let i = (t * SR as f64).round() as i64;
    assert!((24_000..=24_048).contains(&i), "default window: {i}");
    let (t7, _) = refine_sr(&x, t0, SR, &[], RefineWindow::SYMMETRIC_7);
    assert_eq!(t7, t0, "the phase 1 window finds nothing and gives t0 back");
    // an attack 4 ms before t0 is inside -7..+7 only
    let (t_b, _) = refine_sr(&x, 0.504, SR, &[], RefineWindow::SYMMETRIC_7);
    assert!((t_b - 0.5).abs() < 0.0005, "{t_b}");
    // the window is a parameter: lo and hi move the cut and the search range
    let w = RefineWindow {
        lo_ms: -12.0,
        hi_ms: 2.0,
    };
    let (t_w, _) = refine_sr(&x, 0.509, SR, &[], w);
    assert!((t_w - 0.5).abs() < 0.0005, "{t_w}");
    assert_eq!(RefineWindow::default(), RefineWindow::DEFAULT);
    assert_eq!((RefineWindow::DEFAULT.lo_ms, RefineWindow::DEFAULT.hi_ms), (-2.0, 12.0));
    // refine_track passes the window on
    assert_eq!(
        refine_track(&x, SR, &[t0], RefineWindow::SYMMETRIC_7)[0],
        (t7, refine_sr(&x, t0, SR, &[], RefineWindow::SYMMETRIC_7).1)
    );
}

#[test]
fn a_window_without_a_rise_looks_after_its_end_or_keeps_t0() {
    // a decaying tail with no attack in reach: the envelope never rises, t0 stays
    let x = track(&[(0.5, -3.0)], 1.5);
    let (t, rise) = refine_sr(&x, 0.65, SR, &[], RefineWindow::DEFAULT);
    assert_eq!(t, 0.65);
    assert!(rise.is_some_and(|r| r <= 0.0), "{rise:?}");
    // the same tail with an attack 4 ms after the window end: the 25 % rule from the
    // last sample of the search range finds it
    let y = track(&[(0.5, -3.0), (0.666, -3.0)], 1.5);
    let (t2, _) = refine_sr(&y, 0.65, SR, &[], RefineWindow::DEFAULT);
    assert!((t2 - 0.666).abs() < 0.0005, "{t2}");
}

/// `x` plus white noise at `db` dBFS peak (a fixed seed): a real floor, above the 1e-4
/// silence limit of the 25 % rule.
fn noisy(x: &[f32], db: f64) -> Vec<f32> {
    let g = 10f64.powf(db / 20.0);
    let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
    x.iter()
        .map(|&v| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let u = (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
            (v as f64 + g * u) as f32
        })
        .collect()
}

fn sample(t: f64) -> i64 {
    (t * SR as f64).round() as i64
}

#[test]
fn rule_versions_and_the_reference_rule() {
    assert_eq!(RefineRule::default(), RefineRule::Reference);
    assert_eq!(
        (
            RefineRule::Reference.version(),
            RefineRule::Reach.version(),
            RefineRule::FirstMotion.version()
        ),
        (1, 2, 3)
    );
    for r in [RefineRule::Reference, RefineRule::Reach, RefineRule::FirstMotion] {
        assert_eq!(RefineRule::from_version(u64::from(r.version())), Some(r));
    }
    assert_eq!(RefineRule::from_version(0), None);
    assert_eq!(RefineRule::from_version(4), None);
    assert_eq!(RefineRule::CURRENT, RefineRule::FirstMotion);
    // the reference rule is refine_sr, step Kept, on every kind of window
    let x = noisy(&track(&[(0.5, -3.0)], 1.5), -70.0);
    for t0 in [0.486, 0.497, 0.503, 0.65, 2.0] {
        let (t, rise) = refine_sr(&x, t0, SR, &[], W);
        let r = refine_rule(&x, t0, SR, &[], W, RefineRule::Reference);
        assert_eq!(
            (r.t, r.rise, r.step, r.back),
            (t, rise, ReachStep::Kept, BackStep::Off),
            "{t0}"
        );
        // version 2 has no back-track; version 3 is version 2 with it
        let two = refine_rule(&x, t0, SR, &[], W, RefineRule::Reach);
        let three = refine_rule(&x, t0, SR, &[], W, RefineRule::FirstMotion);
        assert_eq!(two.back, BackStep::Off);
        assert_eq!((three.rise, three.step), (two.rise, two.step), "{t0}");
        assert!(three.t <= two.t && three.back != BackStep::Off, "{t0}: {three:?}");
    }
}

#[test]
fn reach_finds_the_attack_the_reference_window_misses() {
    // M3: the DSP frame 17 ms before the attack; the rule window ends 10 ms after an anchor on
    // the floor, so the reference reports a floor sample, 10 ms or more early
    let x = noisy(&track(&[(0.5, -3.0)], 1.5), -70.0);
    let t0 = 0.483;
    let (t1, _) = refine_sr(&x, t0, SR, &[], W);
    assert!(
        sample(t1) < 24_000 - 480,
        "the reference is on the floor: {}",
        sample(t1)
    );
    let r = refine_reach(&x, t0, SR, &[], W);
    assert_eq!(r.step, ReachStep::Reached);
    assert!((24_000..=24_048).contains(&sample(r.t)), "{}", sample(r.t));
    assert!(r.rise.unwrap() > 6.0, "{:?}", r.rise);
    // an attack 19 ms after t0 is still in reach; the same hit refined from near its attack is the same
    let r18 = refine_reach(&x, 0.481, SR, &[], W);
    assert_eq!(r18.step, ReachStep::Reached);
    assert_eq!(r18.t, refine_reach(&x, 0.498, SR, &[], W).t);
    // digital silence: the reference gives t0 back, the reach finds the attack
    let y = track(&[(0.5, -3.0)], 1.5);
    assert_eq!(refine_sr(&y, t0, SR, &[], W).0, t0);
    let ry = refine_reach(&y, t0, SR, &[], W);
    assert_eq!(ry.step, ReachStep::Reached);
    assert!((24_000..=24_048).contains(&sample(ry.t)), "{}", sample(ry.t));
    // a hit the reference window holds is the reference's, bit for bit
    for t0 in [0.497, 0.503, 0.488] {
        let r = refine_reach(&x, t0, SR, &[], W);
        assert_eq!((r.t, r.rise, r.step), {
            let (t, rise) = refine_sr(&x, t0, SR, &[], W);
            (t, rise, ReachStep::Kept)
        });
    }
}

#[test]
fn reach_never_passes_the_midpoint_to_the_next_hit() {
    // the next coarse hit 20 ms after t0: the reach ends at the midpoint (t0 + 10 ms), before
    // the attack at t0 + 17 ms, so the reference stands
    let x = noisy(&track(&[(0.5, -3.0)], 1.5), -70.0);
    let t0 = 0.483;
    let r = refine_reach(&x, t0, SR, &[t0 + 0.020], W);
    assert_eq!(r.step, ReachStep::Kept);
    assert_eq!(r.t, refine_sr(&x, t0, SR, &[t0 + 0.020], W).0);
    // with the next hit 40 ms away the attack is in reach again
    assert_eq!(refine_reach(&x, t0, SR, &[t0 + 0.040], W).step, ReachStep::Reached);
    // a decaying tail without a rise: the reference gives t0 back and nothing follows within
    // 20 ms: t0 stays, flagged
    let r2 = refine_reach(&x, 0.65, SR, &[], W);
    assert_eq!((r2.t, r2.rise, r2.step), {
        let (t, rise) = refine_sr(&x, 0.65, SR, &[], W);
        (t, rise, ReachStep::Unresolved)
    });
    assert_eq!(r2.t, 0.65);
}

#[test]
fn a_window_that_ends_inside_the_attack_keeps_the_reference() {
    // the verifier's case: a floor ripple 9 ms before the onset takes the anchor, so the rule
    // window ends 1 ms into the attack, which starts with a soft click 3 ms before the body.
    // The reference crosses at the click (right); the reach sees only the floor before the
    // window's last millisecond and the body after it. The attack's foot is the click, at the
    // reference's crossing: the reference stands (not the body, 3 ms later)
    let on = 24_000usize;
    let mut buf = vec![0.0f64; SR as usize];
    let tone = |k: usize| (2.0 * std::f64::consts::PI * 2000.0 * k as f64 / SR as f64).sin();
    for k in 0..48 {
        buf[on - 510 + k] += 0.0028 * tone(k); // the ripple, -51 dB, 10.6 ms before the onset
    }
    for k in 0..144 {
        buf[on + k] += 0.02 * tone(k); // the click, -34 dB, 3 ms
    }
    let body = kick_hit((0.4 * SR as f64) as usize, 0.3);
    for (k, v) in body.iter().enumerate() {
        buf[on + 144 + k] += 0.7 * v;
    }
    let x = noisy(&buf.iter().map(|&v| v as f32).collect::<Vec<_>>(), -60.0);
    let t0 = (on as f64 - 600.0) / SR as f64;
    let (t1, rise) = refine_sr(&x, t0, SR, &[], W);
    assert!((sample(t1) - on as i64).abs() <= 12, "the reference: {}", sample(t1));
    let r = refine_reach(&x, t0, SR, &[], W);
    assert_eq!((r.t, r.rise, r.step), (t1, rise, ReachStep::Kept));
}

#[test]
fn a_slow_attack_is_not_a_tail() {
    // a tom 100 ms after a quiet hit: its attack swells for 6 ms, sags 1.6 dB and then jumps to
    // the body. The reference's window starts in the swell, its crossing lies before the
    // anchor and falls into a trough: the shape of a tail, but the 5 ms before the window rise
    // (TAIL_RISE_DB). The swell is the tom's own: no coarse time moves the hit later
    let ms = |v: f64| v * 1e-3 * SR as f64;
    let env = |k: f64| {
        if k < ms(6.0) {
            0.32 * k / ms(6.0)
        } else if k < ms(8.5) {
            0.32 - 0.1 * (k - ms(6.0)) / ms(2.5)
        } else if k < ms(9.0) {
            0.22 + 0.68 * (k - ms(8.5)) / ms(0.5)
        } else {
            0.9 * (-(k - ms(9.0)) / ms(150.0)).exp()
        }
    };
    let mut buf = vec![0.0f64; SR as usize];
    for (on, g) in [(14_400usize, 0.05), (19_200, 1.0)] {
        for k in 0..(0.3 * SR as f64) as usize {
            let ph = 2.0 * std::f64::consts::PI * 1000.0 * k as f64 / SR as f64;
            buf[on + k] += g * env(k as f64) * ph.sin();
        }
    }
    let x = noisy(&buf.iter().map(|&v| v as f32).collect::<Vec<_>>(), -70.0);
    let nb = [0.3];
    // the coarse times whose window holds the attack (earlier ones miss it: the reach's case)
    for k in 0..23 {
        let t0 = 0.390 + k as f64 * 0.001;
        let r = refine_reach(&x, t0, SR, &nb, W);
        assert_eq!(
            (r.t, r.step),
            (refine_sr(&x, t0, SR, &nb, W).0, ReachStep::Kept),
            "{t0}"
        );
    }
}

#[test]
fn a_window_past_the_attack_is_no_tail() {
    // a coarse time 8 to 12 ms late: the reference's window starts in the hit's decay, before a
    // dip and a second stroke. The 5 ms before the window are louder than the window (the hit's
    // own attack): no tail, the reference stands (the kit's SoCal Toms#276)
    let ms = |v: f64| v * 1e-3 * SR as f64;
    let env = |k: f64| {
        if k < ms(0.5) {
            0.9 * k / ms(0.5)
        } else if k < ms(8.0) {
            0.1 + 0.8 * (-(k - ms(0.5)) / ms(3.0)).exp()
        } else if k < ms(9.0) {
            0.1 + 0.4 * (k - ms(8.0)) / ms(1.0)
        } else {
            0.5 * (-(k - ms(9.0)) / ms(60.0)).exp()
        }
    };
    let on = 24_000usize;
    let mut buf = vec![0.0f64; SR as usize];
    for k in 0..(0.3 * SR as f64) as usize {
        buf[on + k] += env(k as f64) * (2.0 * std::f64::consts::PI * 1000.0 * k as f64 / SR as f64).sin();
    }
    let x = noisy(&buf.iter().map(|&v| v as f32).collect::<Vec<_>>(), -70.0);
    let nb = [0.4];
    for k in 0..5 {
        let t0 = 0.508 + k as f64 * 0.001;
        let r = refine_reach(&x, t0, SR, &nb, W);
        assert_eq!(
            (r.t, r.step),
            (refine_sr(&x, t0, SR, &nb, W).0, ReachStep::Kept),
            "{t0}"
        );
    }
}

#[test]
fn a_quieter_event_before_the_hit_stays_its_onset() {
    // a grace note 15 dB under the hit, 12 ms before it: the window holds a rise, so the
    // reference's onset (the grace) stays (a flam is a detection matter, not the reach's)
    let x = noisy(&track(&[(0.488, -18.0), (0.5, -3.0)], 1.5), -70.0);
    let t0 = 0.487;
    let (t, rise) = refine_sr(&x, t0, SR, &[], W);
    assert!((23_424..=23_472).contains(&sample(t)), "the grace: {}", sample(t));
    let r = refine_reach(&x, t0, SR, &[], W);
    assert_eq!((r.t, r.rise, r.step), (t, rise, ReachStep::Kept));
}

#[test]
fn the_tail_fix_starts_the_rule_after_the_previous_tail() {
    // C1: a snare roll at 75 ms (the P4-11b kit's snare: a 190 Hz tone decaying in 70 ms and
    // noise decaying in 110 ms, sharp onsets); the tail of the first hit is above 25 % of the
    // second when the rule window opens
    let mut buf = vec![0.0f64; SR as usize];
    let mut s: u64 = 12345;
    for onset in [0.300, 0.375] {
        let i0 = (onset * SR as f64) as usize;
        for k in 0..(0.4 * SR as f64) as usize {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let u = (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
            let t = k as f64 / SR as f64;
            let tone = (0.7 + 2.0 * std::f64::consts::PI * 190.0 * t).sin() * (-t / 0.07).exp();
            buf[i0 + k] += 0.4 * tone + 0.5 * u * (-t / 0.11).exp();
        }
    }
    let x: Vec<f32> = buf.iter().map(|&v| v as f32).collect();
    let onset = 18_000; // 0.375 s
    let t0 = 0.369;
    let (t1, _) = refine_sr(&x, t0, SR, &[0.296], W);
    assert!(
        sample(t1) < onset - 20,
        "the reference is on the tail: {}",
        sample(t1) - onset
    );
    let r = refine_reach(&x, t0, SR, &[0.296], W);
    assert_eq!(r.step, ReachStep::Tail);
    assert!(r.t > t1 && (sample(r.t) - onset).abs() <= 24, "{}", sample(r.t) - onset);
    // the first hit (silence before it) is the reference's
    let r0 = refine_reach(&x, 0.296, SR, &[t0], W);
    assert_eq!((r0.t, r0.step), (refine_sr(&x, 0.296, SR, &[t0], W).0, ReachStep::Kept));
    // the tail must belong to an earlier coarse hit within TAIL_PREV_S: without one (the first
    // burst taken as the hit's own body) or with it 400 ms away, the reference stands
    for nb in [vec![], vec![t0 - 0.4]] {
        let k = refine_reach(&x, t0, SR, &nb, W);
        assert_eq!(
            (k.t, k.step),
            (refine_sr(&x, t0, SR, &nb, W).0, ReachStep::Kept),
            "{nb:?}"
        );
    }
    // refine_track_rule passes the neighbours: the same as one call per hit
    let rr = refine_track_rule(&x, SR, &[0.296, t0], W, RefineRule::Reach);
    assert_eq!(rr, vec![r0, r]);
    let rf = refine_track_rule(&x, SR, &[0.296, t0], W, RefineRule::Reference);
    assert_eq!(
        rf.iter().map(|r| (r.t, r.rise)).collect::<Vec<_>>(),
        refine_track(&x, SR, &[0.296, t0], W)
    );
}

#[test]
fn the_reach_rule_stays_in_the_signal_everywhere() {
    // every coarse time on a 1 ms grid over a busy track (hits 30 to 70 ms apart over a loud
    // floor), at the file's edges and outside it: no panic, a time inside the file, and the
    // reference's result wherever the reach does not act
    let mut on = vec![];
    let mut t: f64 = 0.004;
    while t < 0.9 {
        on.push((t, -3.0 - 20.0 * ((t * 37.0).sin().abs())));
        t += 0.03 + 0.04 * ((t * 11.0).cos().abs());
    }
    let x = noisy(&track(&on, 1.0), -30.0);
    let mut steps = std::collections::HashMap::new();
    for i in -5..1010 {
        let t0 = i as f64 / 1000.0;
        for nb in [vec![], vec![t0 - 0.03, t0 + 0.03]] {
            let r = refine_reach(&x, t0, SR, &nb, W);
            let (t1, rise1) = refine_sr(&x, t0, SR, &nb, W);
            *steps.entry(format!("{:?}", r.step)).or_insert(0) += 1;
            assert!(r.t.is_finite(), "{t0}");
            if (0.0..1.0).contains(&t0) {
                assert!((0.0..1.0).contains(&r.t), "{t0}: {r:?}");
            }
            match r.step {
                ReachStep::Kept => assert_eq!((r.t, r.rise), (t1, rise1), "{t0}"),
                // no attack found where the window missed one: the coarse time stays
                ReachStep::Unresolved => assert_eq!((r.t, r.rise), (t0, rise1), "{t0}"),
                // a fix only moves a hit later, and not beyond the reach and its rule window
                _ => assert!(r.t > t1 && r.t < t0 + 0.035, "{t0}: {r:?} against {t1}"),
            }
        }
    }
    // the track exercises every step
    for k in ["Kept", "Reached", "Tail", "Unresolved"] {
        assert!(steps.get(k).is_some_and(|&n| n > 0), "{k}: {steps:?}");
    }
}

// ---------------------------------------------------------------- P4-12 stage 2: the back-track

/// A hit at `at` (samples): a 180 Hz sine with a linear attack of `ramp` samples (0: a sharp
/// start at full level, phase 0.7 rad) and a 60 ms decay, peak `amp`.
fn shot(x: &mut [f32], at: usize, ramp: usize, amp: f64) {
    for k in 0..(0.3 * SR as f64) as usize {
        if at + k >= x.len() {
            break;
        }
        let t = k as f64 / SR as f64;
        let env = if ramp == 0 {
            1.0
        } else {
            ((k + 1) as f64 / ramp as f64).min(1.0)
        };
        let v = amp * env * (-t / 0.06).exp() * (2.0 * std::f64::consts::PI * 180.0 * t + 0.7).sin();
        x[at + k] += v as f32;
    }
}

/// A sine of `amp` and `f` Hz in `[from, to)`.
fn tone(x: &mut [f32], from: usize, to: usize, f: f64, amp: f64) {
    for (k, v) in x[from..to].iter_mut().enumerate() {
        *v += (amp * (2.0 * std::f64::consts::PI * f * k as f64 / SR as f64).sin()) as f32;
    }
}

#[test]
fn the_back_track_walks_a_slow_attack_back_to_its_foot() {
    // a 3 ms linear attack: version 2 puts the hit near its 25 % point, about 0.75 ms late
    let on = 24_000;
    let mut x = vec![0.0f32; 48_000];
    shot(&mut x, on, 144, 0.8);
    let x = noisy(&x, -90.0);
    let two = refine_reach(&x, 0.497, SR, &[], W);
    assert!(sample(two.t) - on as i64 > 20, "version 2 sits in the ramp: {two:?}");
    let (foot, step) = first_motion(&x, sample(two.t), SR, 0.497, None, None);
    assert_eq!(step, BackStep::Moved);
    // -33 dB of the peak on a linear ramp is 2 % of it: 3 samples after the start
    assert!((0..=6).contains(&(foot - on as i64)), "{foot} against {on}");
    let three = refine_first_motion(&x, 0.497, SR, &[], W);
    assert_eq!(
        (sample(three.t), three.back, three.rise),
        (foot, BackStep::Moved, two.rise)
    );
    // the spans are in time: at 96 kHz the same hit lands as close in milliseconds
    let mut y = vec![0.0f32; 96_000];
    let mut v = vec![0.0f32; 48_000];
    shot(&mut v, on, 144, 0.8);
    for (k, s) in v.iter().enumerate() {
        y[2 * k] = *s;
        y[2 * k + 1] = *s;
    }
    let (f96, st96) = first_motion(&y, sample(two.t) * 2, 96_000, 0.497, None, None);
    assert_eq!(st96, BackStep::Moved);
    assert!((0..=12).contains(&(f96 - 2 * on as i64)), "{f96}");
}

#[test]
fn a_sharp_start_stays_also_when_band_limited() {
    // a sharp start at an integer sample: the foot is that sample
    let on = 24_000usize;
    let mut x = vec![0.0f32; 48_000];
    shot(&mut x, on, 0, 0.5);
    assert_eq!(
        first_motion(&x, on as i64, SR, 0.497, None, None),
        (on as i64, BackStep::Kept)
    );
    // half a sample later through a windowed sinc (the band-limited ripple before the start is
    // -21 dB one sample before it and above -33 dB for 3 samples): the foot stays within one
    // sample of the start, the 2-tap mean nulls the ripple
    let half = 32i64;
    let h: Vec<f64> = (-half..=half)
        .map(|k| {
            let u = k as f64 - 0.5;
            let sinc = (std::f64::consts::PI * u).sin() / (std::f64::consts::PI * u);
            let w = 0.5 + 0.5 * (std::f64::consts::PI * k as f64 / (half + 1) as f64).cos();
            sinc * w
        })
        .collect();
    let sum: f64 = h.iter().sum();
    let y: Vec<f32> = (0..x.len() as i64)
        .map(|i| {
            let mut a = 0.0;
            for (j, hv) in h.iter().enumerate() {
                let k = i - (j as i64 - half);
                if (0..x.len() as i64).contains(&k) {
                    a += x[k as usize] as f64 * hv / sum;
                }
            }
            a as f32
        })
        .collect();
    assert!(y[on - 1].abs() > 0.5 * 0.05, "the ripple is there: {}", y[on - 1]);
    let (foot, _) = first_motion(&y, on as i64 + 1, SR, 0.497, None, None);
    assert!((on as i64 - 1..=on as i64 + 1).contains(&foot), "{foot}");
}

#[test]
fn a_noise_floor_raises_the_threshold() {
    // a 3 ms attack under a floor 30 dB below the peak (above the 33 dB threshold): the floor
    // term stops the walk in the attack instead of letting it run through the noise
    let on = 24_000usize;
    let mut x = vec![0.0f32; 48_000];
    shot(&mut x, on, 144, 1.0);
    let x = noisy(&x, -30.0);
    let s = on as i64 + 60;
    let (foot, step) = first_motion(&x, s, SR, 0.497, None, None);
    assert_eq!(step, BackStep::Moved, "{foot}");
    // the floor's median block maximum is about -31 dB: 6 dB over it is about 6 % of the peak
    assert!((0..=24).contains(&(foot - on as i64)), "{foot} against {on}");
    assert!(foot < s - 30, "it moved: {foot}");
    assert!(floor_before(&x, s, s - 2400, SR as f64) > 0.02);
    assert_eq!(floor_before(&x, s, s - 90, SR as f64), 0.0, "no full block");
    // the median of the block maxima, the blocks counted back from 2 ms before the hit: one loud
    // block (a bleed burst) does not move it; with an even count the two middle ones average
    let mut b = vec![0.0f32; 48 * 8];
    for (k, lv) in [0.1f32, 0.9, 0.3, 0.2, 0.5].iter().enumerate() {
        b[48 * k + 7] = *lv;
    }
    let end = 48 * 5 + 96;
    assert!((floor_before(&b, end as i64, 0, SR as f64) - 0.3).abs() < 1e-6);
    assert!(
        (floor_before(&b, end as i64, 48, SR as f64) - 0.4).abs() < 1e-6,
        "0.9, 0.3, 0.2, 0.5"
    );
}

#[test]
fn a_walk_that_reaches_its_bound_keeps_the_time() {
    // a bleed tone 15 dB under the hit for the 12 ms before it, the floor before that silent: the
    // run never ends within 10 ms, so the version 2 time stays (a busy context)
    let on = 24_000usize;
    let mut x = vec![0.0f32; 48_000];
    tone(&mut x, on - 576, on, 900.0, 0.18);
    shot(&mut x, on, 0, 1.0);
    assert_eq!(
        first_motion(&x, on as i64, SR, 0.497, None, None),
        (on as i64, BackStep::Busy)
    );
    // the midpoint to the previous coarse hit bounds the walk as well: a tone of 3 ms before the
    // hit ends inside 10 ms, but a previous hit 4 ms before cuts the walk at 2 ms
    let mut y = vec![0.0f32; 48_000];
    tone(&mut y, on - 144, on, 900.0, 0.18);
    shot(&mut y, on, 0, 1.0);
    let (foot, step) = first_motion(&y, on as i64, SR, 0.5, None, None);
    assert_eq!(step, BackStep::Moved);
    assert!((on as i64 - 150..on as i64 - 130).contains(&foot), "{foot}");
    assert_eq!(
        first_motion(&y, on as i64, SR, 0.5, Some(0.496), None),
        (on as i64, BackStep::Busy)
    );
    // the next coarse hit clips the peak window: a peak right after that midpoint is not this hit's
    let mut z = vec![0.0f32; 48_000];
    shot(&mut z, on, 144, 0.01);
    shot(&mut z, on + 100, 0, 1.0);
    let (fz, sz) = first_motion(&z, on as i64 + 72, SR, 0.5, None, Some(0.5 + 2.0 * 80.0 / 48_000.0));
    assert_eq!(sz, BackStep::Moved);
    assert!((on as i64..on as i64 + 8).contains(&fz), "{fz}");
}

#[test]
fn the_walk_span_is_in_time_at_every_rate() {
    // 96 kHz: a tone 3 dB under the hit joined to it for 7 ms (672 samples, more than a span
    // counted in 48 kHz samples) is crossed; for 12 ms (1152 samples) the walk reaches its 10 ms
    // bound (960 samples) and the time stays
    let sr = 96_000u32;
    let on = 48_000usize;
    for (len, want) in [(672usize, BackStep::Moved), (1152, BackStep::Busy)] {
        let mut x = vec![0.0f32; 96_000];
        for (k, v) in x[on - len..on].iter_mut().enumerate() {
            *v = (0.7 * (2.0 * std::f64::consts::PI * 900.0 * k as f64 / sr as f64).sin()) as f32;
        }
        for k in 0..(0.3 * sr as f64) as usize {
            let t = k as f64 / sr as f64;
            x[on + k] += ((-t / 0.06).exp() * (2.0 * std::f64::consts::PI * 180.0 * t + 0.7).sin()) as f32;
        }
        let (foot, step) = first_motion(&x, on as i64, sr, 0.497, None, None);
        assert_eq!(step, want, "{len}");
        if want == BackStep::Moved {
            assert!(
                (on as i64 - len as i64..on as i64 - len as i64 + 8).contains(&foot),
                "{foot}"
            );
        } else {
            assert_eq!(foot, on as i64);
        }
    }
}

#[test]
fn the_floor_window_ends_at_the_midpoint_to_the_previous_hit() {
    // a loud earlier event (a 25 ms burst, 20 dB under the hit) that ends 25 ms before the hit; the
    // previous coarse hit 25 ms before it puts the midpoint after the burst: the floor before the hit
    // is silent and the walk reaches the foot of a 3 ms attack; without that bound the burst is
    // half the floor window and the threshold stops the walk high in the attack
    let on = 24_000usize;
    let mut x = vec![0.0f32; 48_000];
    tone(&mut x, on - 2400, on - 1200, 900.0, 0.1);
    shot(&mut x, on, 144, 1.0);
    let t0 = on as f64 / SR as f64;
    let (foot, step) = first_motion(&x, on as i64 + 60, SR, t0, Some(t0 - 0.025), None);
    assert_eq!(step, BackStep::Moved);
    assert!((0..=6).contains(&(foot - on as i64)), "{foot}");
    let (high, _) = first_motion(&x, on as i64 + 60, SR, t0, None, None);
    assert!(high - on as i64 > 15, "the burst raises the floor: {high}");
}

#[test]
fn a_long_walk_crosses_only_the_hits_own_body() {
    // 7 ms of a tone joined to the hit: 20 dB under it is a separate earlier event (the time
    // stays); 3 dB under it is the hit's own body, version 2 sat late on it (the walk is taken)
    let on = 24_000usize;
    for (amp, want) in [(0.1, BackStep::Separate), (0.7, BackStep::Moved)] {
        let mut x = vec![0.0f32; 48_000];
        tone(&mut x, on - 336, on, 900.0, amp);
        shot(&mut x, on, 0, 1.0);
        let (foot, step) = first_motion(&x, on as i64, SR, 0.497, None, None);
        assert_eq!(step, want, "{amp}");
        if want == BackStep::Moved {
            assert!((on as i64 - 340..on as i64 - 330).contains(&foot), "{foot}");
        } else {
            assert_eq!(foot, on as i64);
        }
    }
}

#[test]
fn the_back_track_keeps_silence_and_the_edges() {
    let x = vec![0.0f32; 4800];
    assert_eq!(first_motion(&x, 2400, SR, 0.05, None, None), (2400, BackStep::Kept));
    let mut y = vec![0.0f32; 4800];
    shot(&mut y, 0, 0, 0.5);
    assert_eq!(first_motion(&y, 0, SR, 0.0, None, None), (0, BackStep::Kept));
    assert_eq!(first_motion(&y, 4800, SR, 0.1, None, None), (4800, BackStep::Kept));
    assert_eq!(first_motion(&y, 9000, SR, 0.2, None, None), (9000, BackStep::Kept));
    // a hit 2 samples into the file: the walk ends at the file's start
    let mut z = vec![0.0f32; 4800];
    shot(&mut z, 2, 48, 0.5);
    let (f, st) = first_motion(&z, 30, SR, 0.0, None, None);
    assert!((0..=30).contains(&f) && st != BackStep::Separate, "{f} {st:?}");
}

#[test]
fn the_first_motion_rule_stays_in_the_signal_everywhere() {
    // every coarse time on a 1 ms grid over a busy track: no panic, never later than version 2,
    // never more than the span earlier, rise and reach step as version 2, and every step taken
    let mut on = vec![];
    let mut t: f64 = 0.004;
    while t < 0.9 {
        on.push((t, -3.0 - 20.0 * ((t * 37.0).sin().abs())));
        t += 0.03 + 0.04 * ((t * 11.0).cos().abs());
    }
    // and a 12 ms bleed tone before every third hit
    let mut x = noisy(&track(&on, 1.0), -45.0);
    for &(t, _) in on.iter().step_by(3) {
        let at = sample(t) as usize;
        if at > 576 {
            tone(&mut x, at - 576, at, 900.0, 0.4);
        }
    }
    let mut steps = std::collections::HashMap::new();
    for i in -5..1010 {
        let t0 = i as f64 / 1000.0;
        for nb in [vec![], vec![t0 - 0.03, t0 + 0.03]] {
            let two = refine_reach(&x, t0, SR, &nb, W);
            let r = refine_first_motion(&x, t0, SR, &nb, W);
            *steps.entry(format!("{:?}", r.back)).or_insert(0) += 1;
            assert!(r.t.is_finite(), "{t0}");
            assert_eq!((r.rise, r.step), (two.rise, two.step), "{t0}");
            assert!(
                r.t <= two.t && r.t >= two.t - FM_SPAN_S - 1e-9,
                "{t0}: {r:?} against {two:?}"
            );
            if r.back != BackStep::Moved {
                assert_eq!(r.t, two.t, "{t0}");
            }
        }
    }
    // (a busy context and a separate event have their own tests above)
    for k in ["Moved", "Kept"] {
        assert!(steps.get(k).is_some_and(|&n| n > 0), "{k}: {steps:?}");
    }
}

#[test]
fn recompute_velocities_keeps_times_and_rise() {
    let x = track(&[(0.5, -6.0), (1.0, -40.0)], 1.5);
    let mut hits = vec![
        Hit {
            t: 0.5,
            v: 1,
            rise: Some(12.5),
        },
        Hit {
            t: 1.0,
            v: 1,
            rise: None,
        },
    ];
    recompute_velocities(&x, SR, &mut hits, &VelocityMap::default());
    assert_eq!(hits[0].t, 0.5);
    assert_eq!(hits[0].rise, Some(12.5));
    assert_eq!(hits[1].rise, None);
    assert_eq!(hits[0].v, velocity_map(&x, SR, &[0.5], &VelocityMap::default())[0]);
    assert!(hits[0].v > 100 && hits[1].v <= 25, "{hits:?}");
}

#[test]
fn the_back_track_with_sample_bounds_equals_the_one_with_hit_times() {
    // P4-13 stage B: `first_motion_mid` takes the midpoints as samples; `first_motion` computes them from the times
    let on = 24_000;
    let mut x = vec![0.0f32; 48_000];
    shot(&mut x, on, 144, 0.8);
    let x = noisy(&x, -90.0);
    let s = on as i64 + 40;
    for (prev, next) in [
        (None, None),
        (Some(0.49), None),
        (Some(0.4995), Some(0.5004)),
        (None, Some(0.5001)),
    ] {
        let pm = prev.map(|p: f64| ((p + 0.5) / 2.0 * SR as f64) as i64);
        let nm = next.map(|n: f64| ((0.5 + n) / 2.0 * SR as f64) as i64);
        assert_eq!(
            first_motion(&x, s, SR, 0.5, prev, next),
            first_motion_mid(&x, s, SR, pm, nm)
        );
    }
    // a bound at the dip keeps the walk out of an earlier stroke
    let (foot, _) = first_motion_mid(&x, s, SR, Some(on as i64 + 20), None);
    assert!(foot >= on as i64 + 20, "{foot}");
}

/// P4-31: a hit with a lead-in 31 dB under its peak for the 3 ms before it: the walk at 33 dB under the peak (shipped)
/// crosses the lead-in, the walk at 29 dB stops at the hit.
fn lead_in_hit() -> (Vec<f32>, usize) {
    let on = 24_000usize;
    let mut x = vec![0.0f32; 48_000];
    tone(&mut x, on - 144, on, 900.0, 0.028);
    shot(&mut x, on, 0, 1.0);
    (x, on)
}

#[test]
fn a_lower_peak_margin_stops_the_walk_later() {
    let (x, on) = lead_in_hit();
    let s = on as i64;
    let (foot, step) = first_motion(&x, s, SR, 0.5, None, None);
    assert_eq!(step, BackStep::Moved);
    assert!(foot < on as i64 - 100, "the shipped walk crosses the lead-in: {foot}");
    assert_eq!(first_motion_peak(&x, s, SR, 0.5, None, None, FM_PEAK_DB), (foot, step));
    // the track entry points pass the margin on; the default margin is the shipped rule
    let w = RefineWindow::DEFAULT;
    for rule in [RefineRule::Reference, RefineRule::Reach, RefineRule::FirstMotion] {
        assert_eq!(
            refine_rule_peak(&x, 0.5, SR, &[], w, rule, FM_PEAK_DB),
            refine_rule(&x, 0.5, SR, &[], w, rule),
            "{rule:?}"
        );
    }
    let shipped = refine_first_motion(&x, 0.5, SR, &[], w);
    let kick = refine_first_motion_peak(&x, 0.5, SR, &[], w, FM_PEAK_DB);
    assert_eq!(kick, shipped);
    assert_eq!(
        (kick.rise, kick.step),
        (shipped.rise, shipped.step),
        "only the back-track differs"
    );
    let coarse = [0.5, 0.9];
    let a = refine_track_rule(&x, SR, &coarse, w, RefineRule::FirstMotion);
    assert_eq!(
        refine_track_rule_peak(&x, SR, &coarse, w, RefineRule::FirstMotion, FM_PEAK_DB),
        a
    );
    let b = refine_track_rule_peak(&x, SR, &coarse, w, RefineRule::FirstMotion, FM_PEAK_DB);
    assert_eq!(b[0].t, kick.t);
    assert_eq!(b.len(), a.len());
}
