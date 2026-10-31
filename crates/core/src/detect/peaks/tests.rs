use super::*;

fn frames(x: &[f32], thr: f64, chain: Chain, b: &mut Branches) -> Vec<usize> {
    pick_column(x, thr, chain, b)
}

/// A column of `n` frames at `base` with the given (frame, value) bumps.
fn col(n: usize, base: f32, bumps: &[(usize, f32)]) -> Vec<f32> {
    let mut x = vec![base; n];
    for &(f, v) in bumps {
        x[f] = v;
    }
    x
}

/// The inputs of the rule tests below (also used by the branch counter test).
fn combine_input() -> Vec<f32> {
    col(80, 0.0, &[(30, 0.8), (32, 0.9), (50, 0.5), (52, 0.6), (54, 0.75)])
}
fn plateau_input() -> Vec<f32> {
    col(40, 0.0, &[(20, 0.7), (21, 0.7)])
}
fn frame0_only_input() -> Vec<f32> {
    col(40, 0.0, &[(0, 3.0)])
}
fn frame0_and_more_input() -> Vec<f32> {
    col(40, 0.0, &[(0, 0.9), (20, 0.9)])
}
fn ends_input() -> Vec<f32> {
    col(30, 0.0, &[(1, 0.8), (29, 0.8)])
}

#[test]
fn moving_average_padding_of_each_chain() {
    let x = col(30, 0.0, &[(0, 1.2), (29, 2.4)]);
    let cli = moving_average(&x, Chain::Cli);
    let mad = moving_average(&x, Chain::Madmom);
    // Frame 0: the CLI edge padding repeats x[0] ten times (11 of 12 values are 1.2).
    assert!((cli[0] - 1.1).abs() < 1e-6, "{}", cli[0]);
    // madmom zero padding: only x[0] in the window.
    assert!((mad[0] - 0.1).abs() < 1e-6, "{}", mad[0]);
    // Last frame: edge padding repeats x[29] once, zero padding adds a zero.
    assert!((cli[29] - 0.4).abs() < 1e-6, "{}", cli[29]);
    assert!((mad[29] - 0.2).abs() < 1e-6, "{}", mad[29]);
    // Inside: both are the plain mean over [t - 10, t + 1].
    assert_eq!((cli[15], mad[15]), (0.0, 0.0));
    assert!((cli[28] - 0.2).abs() < 1e-6 && (mad[28] - 0.2).abs() < 1e-6);
    assert!(moving_average(&[], Chain::Cli).is_empty());
    assert!(moving_average(&[], Chain::Madmom).is_empty());
}

#[test]
fn candidates_are_the_picks_at_the_floor_with_their_margin() {
    for chain in Chain::ALL {
        let x = col(60, 0.01, &[(20, 0.9), (40, 0.3)]);
        let c = candidates(&x, 0.01, chain);
        assert_eq!(c.iter().map(|p| p.0).collect::<Vec<_>>(), vec![20, 40], "{chain:?}");
        let avg = moving_average(&x, chain);
        assert_eq!(c[1].1, 0.3 - avg[40]);
        // the margin decides the threshold: 40 passes 0.22 and not 0.32 (single_peaks_and_threshold)
        assert!(c[1].1 >= 0.22 && c[1].1 < 0.32, "{chain:?} {c:?}");
        assert!(c[0].1 > 0.8, "{c:?}");
        assert!(candidates(&x, 0.95, chain).is_empty());
        assert!(candidates(&[], 0.01, chain).is_empty());
    }
}

#[test]
fn single_peaks_and_threshold() {
    for chain in Chain::ALL {
        let mut b = Branches::default();
        let x = col(60, 0.01, &[(20, 0.9), (40, 0.3)]);
        // At 40: 0.3 against an average of 0.41 / 12 passes 0.22 but not 0.32.
        assert_eq!(frames(&x, 0.32, chain, &mut b), vec![20], "{chain:?}");
        assert_eq!(frames(&x, 0.22, chain, &mut b), vec![20, 40], "{chain:?}");
        assert!(frames(&[0.0; 50], 0.22, chain, &mut b).is_empty());
        assert!(frames(&[], 0.22, chain, &mut b).is_empty());
    }
}

/// Peaks at 30 and 32, and a rising run at 50, 52, 54 (each a local maximum). The CLI
/// chains 50, 52 and 54 into one group and keeps the largest `proc` (54); madmom keeps
/// 50, merges 52 into it and keeps 54 (0.04 s from the kept 50).
#[test]
fn combine_rules_differ() {
    let x = combine_input();
    let mut b = Branches::default();
    assert_eq!(frames(&x, 0.22, Chain::Cli, &mut b), vec![32, 54]);
    assert_eq!((b.peaks, b.cli_joined, b.cli_chain), (5, 3, 1), "{b:?}");
    let mut b = Branches::default();
    assert_eq!(frames(&x, 0.22, Chain::Madmom, &mut b), vec![30, 50, 54]);
    assert_eq!((b.peaks, b.madmom_merged, b.madmom_left_anchor), (5, 2, 1), "{b:?}");
}

/// Two equal frames: both windows of the moving average hold both values, so `proc` is
/// equal and the CLI keeps the first; madmom finds two peaks and merges the second.
#[test]
fn ties_keep_the_first_frame() {
    let x = plateau_input();
    let mut b = Branches::default();
    assert_eq!(frames(&x, 0.22, Chain::Cli, &mut b), vec![20]);
    assert_eq!((b.peaks, b.cli_tie), (2, 1));
    let mut b = Branches::default();
    assert_eq!(frames(&x, 0.22, Chain::Madmom, &mut b), vec![20]);
    assert_eq!((b.peaks, b.madmom_plateau, b.madmom_merged), (2, 1, 1));
}

/// madmom drops a class whose only peak is at frame 0 and keeps frame 0 next to other
/// peaks; the CLI has no such rule. (With edge padding a CLI peak at frame 0 needs
/// `x[0] - x[1] >= 12 * threshold`, so the test uses 3.0, beyond a sigmoid.)
#[test]
fn madmom_frame0_rule() {
    let only0 = frame0_only_input();
    let mut b = Branches::default();
    assert!(frames(&only0, 0.22, Chain::Madmom, &mut b).is_empty());
    assert_eq!((b.peaks, b.madmom_frame0_dropped), (1, 1));
    let mut b = Branches::default();
    assert_eq!(frames(&only0, 0.22, Chain::Cli, &mut b), vec![0]);
    assert_eq!(b.cli_frame0, 1);
    let both = frame0_and_more_input();
    let mut b = Branches::default();
    assert_eq!(frames(&both, 0.22, Chain::Madmom, &mut b), vec![0, 20]);
    assert_eq!(b.madmom_frame0_kept, 1);
    // The CLI average at frame 0 is mostly x[0] itself: no peak there.
    let mut b = Branches::default();
    assert_eq!(frames(&both, 0.22, Chain::Cli, &mut b), vec![20]);
}

/// Peaks next to both ends: the padding enters the average and the maximum.
#[test]
fn peaks_at_both_ends() {
    let x = ends_input();
    for chain in Chain::ALL {
        let mut b = Branches::default();
        assert_eq!(frames(&x, 0.22, chain, &mut b), vec![1, 29], "{chain:?}");
        assert_eq!((b.pad_start, b.pad_end), (1, 1), "{chain:?}");
    }
    // The padding decides: a high start. With edge padding the CLI average at frame 1
    // is 5.6 / 12 (proc 0.13, no peak); madmom's zero padding gives 1.1 / 12 and a peak
    // at frame 1 (0.6, the maximum of [-1, 2]).
    let mut y = vec![0f32; 30];
    y[0] = 0.5;
    y[1] = 0.6;
    let mut b = Branches::default();
    assert!(frames(&y, 0.22, Chain::Cli, &mut b).is_empty());
    let mut b = Branches::default();
    assert_eq!(frames(&y, 0.22, Chain::Madmom, &mut b), vec![1]);
}

/// The CLI compares `proc >= threshold` in f32: a `proc` equal to `0.22f32`, which is
/// below 0.22 in f64, is a peak.
#[test]
fn cli_threshold_compares_in_f32() {
    let thr = 0.22f32;
    assert!(f64::from(thr) < 0.22);
    let mut x = vec![0f32; 40];
    let mut v = 0.2395f32;
    let mut found = false;
    for _ in 0..100_000 {
        x[20] = v;
        let proc = (v - moving_average(&x, Chain::Cli)[20]).max(0.0);
        if proc == thr {
            found = true;
            break;
        }
        v = v.next_up();
    }
    assert!(found, "no activation gives proc == 0.22f32");
    let mut b = Branches::default();
    assert_eq!(frames(&x, 0.22, Chain::Cli, &mut b), vec![20]);
    // One ulp lower is below the threshold.
    x[20] = v.next_down();
    let proc = x[20] - moving_average(&x, Chain::Cli)[20];
    assert!(proc < thr);
    assert!(frames(&x, 0.22, Chain::Cli, &mut b).is_empty());
}

/// madmom compares `act >= avg + threshold` with the sum in f32.
#[test]
fn madmom_threshold_sum_is_f32() {
    let mut x = vec![0f32; 40];
    let mut v = 0.2395f32;
    // The smallest activation that passes.
    loop {
        x[20] = v;
        let avg = moving_average(&x, Chain::Madmom)[20];
        if v >= avg + 0.22f32 {
            break;
        }
        v = v.next_up();
    }
    let mut b = Branches::default();
    assert_eq!(frames(&x, 0.22, Chain::Madmom, &mut b), vec![20]);
    x[20] = v.next_down();
    assert!(frames(&x, 0.22, Chain::Madmom, &mut b).is_empty());
}

#[test]
fn pick_sorts_classes_and_sets_time_and_activation() {
    let n = 50;
    let mut a = vec![0f32; n * N_CLASSES];
    a[10 * N_CLASSES + 3] = 0.9; // HH at 10
    a[10 * N_CLASSES] = 0.8; // BD at 10
    a[30 * N_CLASSES + 2] = 0.95; // TT at 30
    for chain in Chain::ALL {
        let o = pick(&a, chain);
        let got: Vec<(usize, usize)> = o.iter().map(|o| (o.frame, o.class)).collect();
        assert_eq!(got, vec![(10, 0), (10, 3), (30, 2)], "{chain:?}");
        assert_eq!(o[2].time, 0.3);
        assert_eq!(o[1].activation, 0.9);
        let (o2, b) = pick_counted(&a, chain);
        assert_eq!(o2, o);
        assert_eq!(b.peaks, 3);
    }
    assert!(pick(&[], Chain::Cli).is_empty());
}

/// [`Chain::Mix`] picks with the madmom rules.
#[test]
fn pick_with_takes_the_thresholds_per_class() {
    // Two frames of class 1 (SD) and class 2 (TT) at 0.3: above SD's 0.24, below TT's 0.32.
    let mut act = vec![0f32; 60 * N_CLASSES];
    act[20 * N_CLASSES + 1] = 0.3;
    act[40 * N_CLASSES + 2] = 0.3;
    for chain in Chain::ALL {
        assert_eq!(pick_with(&act, chain, &THRESHOLDS), pick(&act, chain), "{chain:?}");
        let classes = |th: &[f64; N_CLASSES]| -> Vec<(usize, usize)> {
            pick_with(&act, chain, th).iter().map(|o| (o.frame, o.class)).collect()
        };
        assert_eq!(classes(&THRESHOLDS), vec![(20, 1)], "{chain:?}");
        assert_eq!(
            classes(&[0.22, 0.24, 0.2, 0.22, 0.30]),
            vec![(20, 1), (40, 2)],
            "{chain:?}"
        );
        assert!(classes(&[0.5; N_CLASSES]).is_empty(), "{chain:?}");
    }
}

#[test]
fn mix_uses_the_madmom_picker() {
    let x = combine_input();
    let mut b = Branches::default();
    assert_eq!(frames(&x, 0.22, Chain::Mix, &mut b), vec![30, 50, 54]);
    assert_eq!(moving_average(&x, Chain::Mix), moving_average(&x, Chain::Madmom));
    let mut a = vec![0f32; 40 * N_CLASSES];
    a[0] = 0.9;
    assert!(pick(&a, Chain::Mix).is_empty());
    assert_eq!(pick(&a, Chain::Cli).len(), 0);
}

#[test]
#[should_panic(expected = "is not (frames, 5)")]
fn pick_rejects_a_ragged_input() {
    let _ = pick(&[0.0; 7], Chain::Cli);
}

/// Every counted branch of both pickers is hit by the inputs of the rule tests above.
#[test]
fn branch_counter_covers_every_rule() {
    let inputs = [
        combine_input(),
        plateau_input(),
        frame0_only_input(),
        frame0_and_more_input(),
        ends_input(),
    ];
    let mut all = Branches::default();
    for x in &inputs {
        for chain in Chain::ALL {
            let mut b = Branches::default();
            pick_column(x, 0.22, chain, &mut b);
            all.add(&b);
        }
    }
    for (name, v) in all.pairs() {
        assert!(v > 0, "branch {name} was never hit: {all:?}");
    }
    let mut twice = all.clone();
    twice.add(&all);
    for ((_, a), (_, t)) in all.pairs().iter().zip(twice.pairs()) {
        assert_eq!(t, 2 * a);
    }
}
