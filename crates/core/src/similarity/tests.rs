use super::*;

const SR: u32 = 48_000;

/// A decaying sine burst of `f` Hz at `t` seconds with amplitude `a`.
fn burst(x: &mut [f32], t: f64, f: f64, a: f32, decay_s: f64) {
    let s0 = (t * f64::from(SR)) as usize;
    for i in 0..(0.3 * f64::from(SR)) as usize {
        if s0 + i >= x.len() {
            break;
        }
        let tt = i as f64 / f64::from(SR);
        x[s0 + i] += a * ((2.0 * std::f64::consts::PI * f * tt).sin() * (-tt / decay_s).exp()) as f32;
    }
}

fn noise_burst(x: &mut [f32], t: f64, a: f32) {
    let s0 = (t * f64::from(SR)) as usize;
    let mut seed: u32 = 12345;
    for i in 0..(0.1 * f64::from(SR)) as usize {
        if s0 + i >= x.len() {
            break;
        }
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let r = ((seed >> 16) as f32 / 32768.0) - 1.0;
        x[s0 + i] += a * r * (-(i as f32) / (0.02 * SR as f32)).exp();
    }
}

fn signal() -> (Vec<f32>, Vec<f64>, Vec<f64>) {
    // toms at 110 Hz (the "this" drum) and snare-like noise (the bleed), every 0.5 s
    let mut x = vec![0f32; SR as usize * 6];
    let (mut tom, mut snare) = (vec![], vec![]);
    for i in 0..10 {
        let t = 0.2 + i as f64 * 0.5;
        if i % 2 == 0 {
            burst(&mut x, t, 110.0, 0.2 + 0.05 * i as f32, 0.15);
            tom.push(t);
        } else {
            noise_burst(&mut x, t, 0.05);
            snare.push(t);
        }
    }
    (x, tom, snare)
}

#[test]
fn recipe_names_parse_back_and_the_dimensions_add_up() {
    for r in [Recipe::Patch, Recipe::Mfcc] {
        assert_eq!(Recipe::parse(r.name()), Some(r));
        assert_eq!(r.len(), r.dims().iter().sum::<usize>());
        assert!(!r.is_empty());
    }
    assert_eq!(Recipe::parse("patch-1"), None, "the model recipe is gone");
    assert_eq!(Recipe::parse("nope"), None);
    assert_eq!(Recipe::Patch.dims(), [128, 51, 12]);
    assert_eq!(Recipe::Mfcc.dims(), [52, 19, 12]);
}

#[test]
fn weights_default_and_as_array() {
    assert_eq!(Weights::default(), Weights::DEFAULT);
    assert_eq!(Weights::DEFAULT.as_array(), [1.0, 1.0, 0.5]);
}

#[test]
fn normalise_removes_level_and_mean() {
    let a = normalise(&[0.0, -10.0, -20.0], true);
    let b = normalise(&[30.0, 20.0, 10.0], true);
    assert!(
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-6),
        "a level offset in dB does not count"
    );
    assert!(a.iter().sum::<f32>().abs() < 1e-6);
    assert!((a.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 1e-5);
    assert_eq!(
        normalise(&[5.0, 5.0], true),
        vec![0.0, 0.0],
        "a flat block has no shape"
    );
    assert!(normalise(&[], true).is_empty());
    // the floor: a value 200 dB under the top counts as top - 80
    let f = normalise(&[0.0, -200.0], true);
    let g = normalise(&[0.0, -80.0], true);
    assert_eq!(f, g);
}

#[test]
fn anchor_finds_the_attack_after_the_detector_time() {
    let mut x = vec![0f32; SR as usize];
    burst(&mut x, 0.5, 200.0, 0.5, 0.05);
    let a = anchor(&x, SR, 0.494);
    assert!((a - 0.5).abs() < 0.0015, "{a}");
}

#[test]
fn blocks_are_level_free_and_the_same_drum_is_near() {
    let (x, tom, snare) = signal();
    for r in [Recipe::Patch, Recipe::Mfcc] {
        let ex = Extractor::new(SR, r);
        assert_eq!(ex.recipe(), r);
        let f = |t: f64| weigh(&ex.blocks(&x, t).flat(), r, &Weights::DEFAULT);
        let t0 = f(tom[0]);
        assert_eq!(t0.len(), r.len());
        // a louder hit of the same drum is nearly the same vector
        assert!(cosine(&t0, &f(tom[3])) > 0.9, "{r:?}: {}", cosine(&t0, &f(tom[3])));
        assert!(cosine(&t0, &f(tom[1])) > cosine(&t0, &f(snare[0])) + 0.2, "{r:?}");
    }
    // a window past the end is zero-padded
    let ex = Extractor::new(SR, Recipe::Patch);
    let e = ex.blocks(&x, 5.99).flat();
    assert_eq!(e.len(), Recipe::Patch.len());
    assert!(e.iter().all(|v| v.is_finite()));
}

#[test]
fn blocks_flat_keeps_the_order() {
    let b = Blocks {
        attack: vec![1.0],
        body: vec![2.0],
        env: vec![3.0],
    };
    assert_eq!(b.flat(), vec![1.0, 2.0, 3.0]);
}

#[test]
fn candidate_features_at_44_1_48_and_96_khz_compare() {
    let mk = |sr: u32| {
        let mut x = vec![0f32; sr as usize];
        let s0 = (0.3 * f64::from(sr)) as usize;
        for i in 0..(0.3 * f64::from(sr)) as usize {
            let tt = i as f64 / f64::from(sr);
            x[s0 + i] = 0.5 * ((2.0 * std::f64::consts::PI * 150.0 * tt).sin() * (-tt / 0.1).exp()) as f32;
        }
        let f = candidate_features(&x, sr, &[0.295], Recipe::Patch);
        weigh(&f[0], Recipe::Patch, &Weights::DEFAULT)
    };
    let (a, b, c) = (mk(44_100), mk(48_000), mk(96_000));
    assert!(cosine(&a, &b) > 0.9, "{}", cosine(&a, &b));
    assert!(cosine(&b, &c) > 0.85, "{}", cosine(&b, &c));
}

#[test]
fn weigh_scales_blocks_and_normalises() {
    let r = Recipe::Patch;
    let mut flat = vec![0f32; r.len()];
    flat[0] = 1.0; // attack
    flat[r.dims()[0]] = 1.0; // body
    let w = weigh(
        &flat,
        r,
        &Weights {
            attack: 3.0,
            body: 4.0,
            env: 1.0,
        },
    );
    assert!((w[0] - 0.6).abs() < 1e-6 && (w[r.dims()[0]] - 0.8).abs() < 1e-6);
    let z = weigh(&vec![0f32; r.len()], r, &Weights::DEFAULT);
    assert!(z.iter().all(|v| *v == 0.0));
}

#[test]
fn track_mean_and_centred_take_off_the_typical_candidate() {
    let r = Recipe::Patch;
    let d = r.dims();
    let mk = |a: f32, b: f32| {
        let mut v = vec![0f32; r.len()];
        v[0] = a; // attack
        v[1] = b;
        v[d[0]] = 1.0; // body: the same in every vector
        v
    };
    let feats: Vec<f32> = [mk(1.0, 0.0), mk(0.0, 1.0), mk(1.0, 1.0)].concat();
    let m = track_mean(&feats, r);
    assert!((m[0] - 2.0 / 3.0).abs() < 1e-6 && (m[1] - 2.0 / 3.0).abs() < 1e-6 && m[d[0]] == 1.0);
    assert!(track_mean(&[], r).iter().all(|v| *v == 0.0));
    let c = centred(&mk(1.0, 0.0), &m, r);
    // attack (1/3, -2/3) normalised; the body block held only the shared colour: zeros
    assert!(
        (c[0] - 1.0 / 5f32.sqrt()).abs() < 1e-5 && (c[1] + 2.0 / 5f32.sqrt()).abs() < 1e-5,
        "{:?}",
        &c[..2]
    );
    assert!(c[d[0]..d[0] + d[1]].iter().all(|v| *v == 0.0));
    // two candidates that share the colour and differ only in the attack point apart
    let (a, b) = (centred(&mk(1.0, 0.0), &m, r), centred(&mk(0.0, 1.0), &m, r));
    let w = Weights::DEFAULT;
    assert!(cosine(&weigh(&a, r, &w), &weigh(&b, r, &w)) < 0.0);
    assert!(
        cosine(&weigh(&mk(1.0, 0.0), r, &w), &weigh(&mk(0.0, 1.0), r, &w)) > 0.4,
        "without centring they look alike"
    );
}

#[test]
fn cosine_of_unit_vectors() {
    assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), 1.0);
    assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
    assert_eq!(cosine(&[1.0, 0.0], &[-1.0, 0.0]), -1.0);
}

#[test]
fn nearest_mean_averages_the_k_best() {
    let c = [1.0f32, 0.0];
    let e1 = [1.0f32, 0.0];
    let e2 = [0.0f32, 1.0];
    let e3 = [-1.0f32, 0.0];
    let e4 = [0.6f32, 0.8];
    assert_eq!(nearest_mean(&c, &[]), 0.0);
    assert_eq!(nearest_mean(&c, &[&e1]), 1.0);
    // k = 3 of 4: 1.0, 0.6, 0.0 (not -1.0)
    assert!((nearest_mean(&c, &[&e1, &e2, &e3, &e4]) - 1.6 / 3.0).abs() < 1e-6);
}

#[test]
fn score_follows_section_3_1() {
    let c = [1.0f32, 0.0];
    let p = [1.0f32, 0.0];
    let n_far = [0.0f32, 1.0];
    let n_near = [0.8f32, 0.6];
    let anti = [-1.0f32, 0.0];
    assert_eq!(score(&c, &[&p], &[]), 1.0);
    assert_eq!(score(&c, &[&anti], &[]), 0.0);
    // a negative that looks like nothing in c does not move the score
    assert_eq!(score(&c, &[&p], &[&n_far]), 1.0);
    // one that looks like c pulls it down
    assert!((score(&c, &[&p], &[&n_near]) - 0.6).abs() < 1e-6);
    // a negative with a negative similarity counts as 0
    let p6 = [0.6f32, 0.8];
    assert!((score(&c, &[&p6], &[&anti]) - 0.8).abs() < 1e-6);
}

#[test]
fn example_scores_leave_each_example_out() {
    let a = [1.0f32, 0.0];
    let b = [0.6f32, 0.8];
    let n = [0.0f32, 1.0];
    let (this, not) = example_scores(&[&a, &b], &[&n]);
    // a against b only (0.6), minus its similarity to n (0)
    assert!((this[0] - 0.8).abs() < 1e-6, "{this:?}");
    // b against a (0.6), minus n (0.8)
    assert!((this[1] - 0.4).abs() < 1e-6, "{this:?}");
    // n against the positives: mean of 0.0 and 0.8, no other negative
    assert!((not[0] - 0.7).abs() < 1e-6, "{not:?}");
    let (one, _) = example_scores(&[&a], &[]);
    assert_eq!(one, vec![0.5], "one example alone has nothing to compare with");
}

#[test]
fn auto_threshold_sits_in_the_gap() {
    // bleed around 0.55, hits around 0.9, nothing between 0.65 and 0.82
    let mut c: Vec<f32> = (0..200).map(|i| 0.5 + (i % 10) as f32 * 0.015).collect();
    c.extend((0..100).map(|i| 0.83 + (i % 10) as f32 * 0.015));
    let t = auto_threshold(&c, &[0.85, 0.92], &[0.55]);
    assert!(t > 0.66 && t < 0.82, "{t}");
    // the negatives do not move it, even one that scores like the hits
    assert_eq!(auto_threshold(&c, &[0.85, 0.92], &[0.9]), t);
    assert_eq!(auto_threshold(&c, &[0.85, 0.92], &[]), t);
    // overlapping groups: the emptiest bins near the boundary of the two
    let mut d: Vec<f32> = (0..200).map(|i| 0.4 + (i % 10) as f32 * 0.02).collect(); // 0.40 to 0.58
    d.extend((0..6).map(|i| 0.6 + (i % 2) as f32 * 0.02)); // a thin stretch at 0.60 and 0.62
    d.extend((0..80).map(|i| 0.64 + (i % 10) as f32 * 0.02)); // 0.64 to 0.82
    let t = auto_threshold(&d, &[0.7, 0.72], &[]);
    assert!(t > 0.6 && t < 0.64, "{t}");
    // nothing under the examples: every candidate stays
    let e: Vec<f32> = (0..100).map(|i| 0.6 + (i % 10) as f32 * 0.01).collect();
    assert_eq!(auto_threshold(&e, &[0.65], &[]), AUTO_LO);
    // too few candidates or no positive: the default
    assert_eq!(auto_threshold(&[0.1, 0.2], &[0.9], &[]), DEFAULT_THRESHOLD);
    assert_eq!(auto_threshold(&c, &[], &[0.2]), DEFAULT_THRESHOLD);
    assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
    assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
}

#[test]
fn auto_threshold_is_the_middle_of_the_empty_run_exactly() {
    // bleed at 0.42 to 0.48 (mean 0.45), hits at 0.72 to 0.78 (mean 0.75): boundary 0.60, the empty
    // bins 0.54 to 0.66 of the window: the middle is 0.60
    let mut c: Vec<f32> = Vec::new();
    for v in [0.42f32, 0.44, 0.46, 0.48, 0.72, 0.74, 0.76, 0.78] {
        c.extend(std::iter::repeat_n(v + 0.001, 25));
    }
    let t = auto_threshold(&c, &[0.75, 0.76], &[]);
    assert!((t - 0.60).abs() < 1e-6, "{t}");
}

#[test]
fn auto_threshold_seeds_the_rest_0_1_under_the_examples() {
    // the rest sits 0.14 to 0.18 under the examples: a seed 0.2 under them finds nothing
    let mut c: Vec<f32> = Vec::new();
    for v in [0.52f32, 0.54, 0.56] {
        c.extend(std::iter::repeat_n(v + 0.001, 30));
    }
    for v in [0.66f32, 0.68, 0.70, 0.72, 0.74] {
        c.extend(std::iter::repeat_n(v + 0.001, 10));
    }
    let t = auto_threshold(&c, &[0.70], &[]);
    assert!((t - 0.62).abs() < 1e-6, "{t}");
}

#[test]
fn filter_detected_forced_not_takes_the_nearest_hit_and_a_time_in_both_lists_adds_nothing() {
    let det = vec![(0.10, 47), (0.12, 47)];
    assert_eq!(
        filter_detected(&det, &[], 0.5, &[], &[0.105], 47, 0.03),
        vec![(0.12, 47)]
    );
    assert_eq!(
        filter_detected(&det, &[], 0.5, &[], &[0.118], 47, 0.03),
        vec![(0.10, 47)]
    );
    // a time that is "this" and "not" at once is not brought back
    assert_eq!(filter_detected(&[], &[], 0.5, &[0.2], &[0.2], 47, 0.03), vec![]);
    assert_eq!(filter_detected(&[], &[], 0.5, &[0.2], &[], 47, 0.03), vec![(0.2, 47)]);
}

#[test]
fn filter_detected_keeps_scores_at_the_threshold_and_applies_forced_examples() {
    let det = vec![(0.10, 47), (0.20, 47), (0.30, 47), (0.40, 38)];
    let scores = vec![(10, 0.9), (20, 0.4), (30, 0.6), (40, 0.1)];
    let out = filter_detected(&det, &scores, 0.6, &[], &[], 47, 0.03);
    assert_eq!(out, vec![(0.10, 47), (0.30, 47)], "0.6 passes 0.6");
    // a frame without a score stays
    let out = filter_detected(&[(0.55, 47)], &scores, 0.6, &[], &[], 47, 0.03);
    assert_eq!(out, vec![(0.55, 47)]);
    // forced: "not" drops the nearest hit, "this" brings one back
    let out = filter_detected(&det, &scores, 0.6, &[0.20], &[0.105], 47, 0.03);
    assert_eq!(out, vec![(0.20, 47), (0.30, 47)]);
    // a "this" time with a hit near it adds nothing
    let out = filter_detected(&det, &scores, 0.0, &[0.31], &[], 47, 0.03);
    assert_eq!(out.len(), 4);
}

#[test]
fn suggest_threshold_follows_the_kinds_of_examples() {
    let c = [0.31, 0.33, 0.35, 0.36, 0.8, 0.82, 0.85, 0.9];
    assert_eq!(suggest_threshold(&c, &[], &[]), DEFAULT_THRESHOLD);
    assert_eq!(suggest_threshold(&c, &[], &[0.2]), SUGGEST_NOT_ONLY);
    assert_eq!(
        suggest_threshold(&c, &[0.85, 0.92], &[]),
        auto_threshold(&c, &[0.85, 0.92], &[])
    );
    assert_eq!(suggest_threshold(&c, &[0.85, 0.92], &[0.3]), SUGGEST_BOTH);
    // planted fault: the "not only" cut keeps a candidate that is only somewhat like the "not" example
    // (cosine 0.6: score 0.2) and drops a near copy (cosine 0.95: score 0.025)
    let score_of = |sn: f32| score(&[1.0, 0.0], &[], &[&[sn, (1.0 - sn * sn).sqrt()]]);
    assert!(score_of(0.6) >= SUGGEST_NOT_ONLY && score_of(0.95) < SUGGEST_NOT_ONLY);
    // with both kinds: a candidate as like the "not" as the "this" example stays, one clearly more like the "not" goes
    let both = |sp: f32, sn: f32| ((1.0 + sp - sn.max(0.0)) / 2.0).clamp(0.0, 1.0);
    assert!(both(0.5, 0.5) >= SUGGEST_BOTH && both(0.5, 0.75) < SUGGEST_BOTH);
}
