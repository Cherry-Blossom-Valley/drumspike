use super::*;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

const SR: u32 = 48_000;

/// A deterministic noise source in [-1, 1).
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

/// A stick stroke: white noise with a 3 ms exponential decay at `amp`, added at `t` (s).
fn stroke(x: &mut [f64], t: f64, amp: f64, n: &mut Noise) {
    stroke_tau(x, t, amp, 0.003, 0.04, n);
}

/// A noise stroke with the decay `tau` (s), `len` seconds long.
fn stroke_tau(x: &mut [f64], t: f64, amp: f64, tau: f64, len: f64, n: &mut Noise) {
    let i0 = (t * f64::from(SR)).round() as usize;
    for k in 0..(len * f64::from(SR)) as usize {
        if i0 + k >= x.len() {
            break;
        }
        x[i0 + k] += amp * n.next() * (-(k as f64) / (tau * f64::from(SR))).exp();
    }
}

fn f32s(x: &[f64]) -> Vec<f32> {
    x.iter().map(|&v| v as f32).collect()
}

/// A snare mic: single strokes every 0.5 s from 0.5 s, and flams (a grace 6 dB under, the main `gap` later) at `flams`.
fn snare(n_hits: usize, flams: &[(f64, f64)], noise_db: f64) -> (Vec<f32>, Vec<f64>) {
    let mut x = vec![0.0f64; (0.5 * (n_hits + 2) as f64 * f64::from(SR)) as usize];
    let mut nz = Noise(0x1234_5678_9abc_def1);
    let mut hits = Vec::new();
    for i in 0..n_hits {
        let t = 0.5 + 0.5 * i as f64;
        if let Some(&(_, gap)) = flams.iter().find(|f| (f.0 - t).abs() < 1e-9) {
            stroke(&mut x, t, 0.25, &mut nz);
            stroke(&mut x, t + gap, 0.5, &mut nz);
        } else {
            stroke(&mut x, t, 0.5, &mut nz);
        }
        hits.push(t);
    }
    let g = 10f64.powf(noise_db / 20.0);
    for v in &mut x {
        *v += g * nz.next();
    }
    (f32s(&x), hits)
}

#[test]
fn the_rule_runs_on_the_snare_from_version_1() {
    assert!(applies(1, "snare"));
    assert!(applies(FLAM_RULE_VERSION, "snare"));
    assert!(!applies(0, "snare"));
    for role in ["kick", "tom", "hat", "cymbal", "other"] {
        assert!(!applies(1, role), "{role}");
    }
    assert_eq!(hop_of(48_000), 6);
    assert_eq!(hop_of(44_100), 6, "5.5125 rounds to 6");
    assert_eq!(hop_of(96_000), 12);
    assert_eq!(hop_of(4_000), 1, "0.5 rounds to the even 0, at least 1");
}

#[test]
fn the_percentile_is_numpys_linear_one() {
    // values from numpy 2: np.percentile([3, 1, 4, 1, 5, 9, 2, 6], q)
    let v = [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0];
    assert_eq!(np_percentile(&v, 10.0), Some(1.0));
    assert_eq!(np_percentile(&v, 50.0), Some(3.5));
    assert!((np_percentile(&v, 99.5).unwrap() - 8.895).abs() < 1e-12);
    assert_eq!(np_percentile(&v, 100.0), Some(9.0));
    assert_eq!(np_percentile(&v, 0.0), Some(1.0));
    assert_eq!(np_percentile(&[], 10.0), None);
    assert_eq!(np_percentile(&[2.5], 99.5), Some(2.5));
    // the upper branch of numpy's lerp: b - (b - a) * (1 - g) for g >= 0.5
    let (a, b) = (0.1f64, 0.7f64);
    let g = 3.0 * 0.9 - 2.0;
    assert_eq!(
        np_percentile(&[a, b, 1.0, 2.0], 90.0),
        Some(2.0 - (2.0 - 1.0) * (1.0 - g))
    );
    assert_eq!(np_percentile(&[a, b], 25.0), Some(a + (b - a) * 0.25));
}

#[test]
fn the_time_domain_zero_phase_filter_equals_the_frequency_domain_one() {
    // the reference: |H|^4 of the RBJ high-pass applied to the FFT of the padded signal (`onset_guard.zero_phase`)
    let mut nz = Noise(7);
    let x: Vec<f32> = (0..3000).map(|_| nz.next() as f32 * 0.5).collect();
    let sr = 8000;
    let got = zero_phase(&x, sr, &[(FilterKind::HighPass, 1000.0)]);
    let pad = 2000;
    let nf = (x.len() + 2 * pad).next_power_of_two();
    let mut buf = vec![Complex::new(0.0f64, 0.0); nf];
    for (i, &v) in x.iter().enumerate() {
        buf[pad + i].re = f64::from(v);
    }
    let mut planner = FftPlanner::<f64>::new();
    planner.plan_fft_forward(nf).process(&mut buf);
    let q = Biquad::new(FilterKind::HighPass, 1000.0, sr);
    for (k, c) in buf.iter_mut().enumerate() {
        let w = -2.0 * std::f64::consts::PI * k as f64 / nf as f64;
        let z = Complex::new(w.cos(), w.sin());
        let h = (q.b[0] + q.b[1] * z + q.b[2] * z * z) / (1.0 + q.a[0] * z + q.a[1] * z * z);
        *c *= h.norm_sqr() * h.norm_sqr();
    }
    planner.plan_fft_inverse(nf).process(&mut buf);
    let worst = (0..x.len())
        .map(|i| (got[i] - buf[pad + i].re / nf as f64).abs())
        .fold(0.0f64, f64::max);
    assert!(worst < 1e-12, "{worst}");
    // no stage: the signal as it is
    assert_eq!(
        zero_phase(&x[..4], sr, &[]),
        x[..4].iter().map(|&v| f64::from(v)).collect::<Vec<_>>()
    );
}

#[test]
fn the_attack_envelope_holds_the_block_maxima() {
    let p = FlamParams {
        hp_hz: 0.0,
        ..FlamParams::default()
    };
    let mut x = vec![0.0f32; 6 * 20 + 3];
    x[6 * 2 + 4] = -0.5; // block 2
    x[6 * 12] = 0.25; // block 12
    let (env, hop) = attack_env(&x, SR, &p);
    assert_eq!((hop, env.len()), (6, 20));
    // the hold is 8 blocks (1 ms): the block and the 7 before it
    for (i, &v) in env.iter().enumerate() {
        let want = if (12..=19).contains(&i) {
            0.25
        } else if (2..=9).contains(&i) {
            0.5
        } else {
            0.0
        };
        assert_eq!(v, want, "block {i}");
    }
    // with the high-pass a constant gives no attack
    let (env, _) = attack_env(&vec![0.5f32; 4800], SR, &FlamParams::default());
    assert!(env[100..700].iter().all(|&v| v < 1e-3), "{}", env[400]);
}

#[test]
fn strong_peaks_and_the_gain_follow_the_reference() {
    // B: 10 strong blocks in 10 blocks of 50 ms at 8000 blocks per second; A hears B 20 dB lower
    let rate = 8000.0;
    let mut b = vec![1e-4f64; 4000];
    for i in 0..10 {
        b[i * 400 + 100] = if i == 3 { 0.05 } else { 1.0 };
    }
    let peaks = strong_peaks(&b, rate);
    assert_eq!(
        peaks,
        (0..10).filter(|&i| i != 3).map(|i| i * 400 + 100).collect::<Vec<_>>(),
        "0.05 is 26 dB under"
    );
    let a: Vec<f64> = b.iter().map(|v| v * 0.1).collect();
    let g = calibrate(&a, &b, &peaks, &[], rate).unwrap();
    assert!((g + 20.0).abs() < 1e-6, "{g}");
    // A has its own hit at 5 of the 9 peaks: the 4 quiet values are too few, every value counts
    let mut a2 = a.clone();
    let own: Vec<f64> = peaks.iter().take(5).map(|&p| p as f64 / rate).collect();
    for &p in peaks.iter().take(5) {
        a2[p + 10] = 1.0;
    }
    let g2 = calibrate(&a2, &b, &peaks, &own, rate).unwrap();
    assert!(
        (g2 + 20.0).abs() < 1e-6,
        "the 10th percentile of 5 x 0 dB and 4 x -20 dB: {g2}"
    );
    // fewer than 5 strong hits: no gain
    assert_eq!(calibrate(&a, &b, &peaks[..4], &[], rate), None);
    // a silent source (under -100 dB) gives no values
    assert_eq!(calibrate(&a, &vec![0.0; 4000], &peaks, &[], rate), None);
    let gains = transfer_gains(&a, &[&b, &b[..300]], &[], 48_000, 6);
    assert!(
        (gains[0].unwrap() + 20.0).abs() < 1e-6 && gains[1].is_none(),
        "{gains:?}"
    );
}

#[test]
fn a_flam_gets_its_second_stroke_and_single_strokes_get_none() {
    let flams = [(2.0, 0.025), (4.0, 0.040)];
    let (x, hits) = snare(16, &flams, -70.0);
    let (found, gains) = flam_split(&x, SR, &hits, &[], &FlamParams::default());
    assert!(gains.is_empty());
    let add = added(&found);
    assert_eq!(add.len(), 2, "{found:?}");
    for (s, &(t, gap)) in found.iter().zip(&flams) {
        assert_eq!(s.first, t);
        assert!((s.t - (t + gap)).abs() < 0.0005, "{s:?}");
        assert!(s.rise_db >= 9.0 && !s.bleed, "{s:?}");
        assert!((s.gap_ms - gap * 1000.0).abs() < 0.5);
    }
}

#[test]
fn the_window_limits_hold() {
    let p = FlamParams::default();
    // a main stroke 8 ms after the grace (under the 12 ms minimum) and one 60 ms after (past 50 ms) are no second stroke
    let (x, hits) = snare(10, &[(2.0, 0.008), (3.0, 0.060)], -70.0);
    let (env, _) = attack_env(&x, SR, &p);
    assert!(second_onsets(&x, SR, &hits, &p, &env, &[]).is_empty());
    // the rise: with rise_db 80 (the main stroke lies about 65 dB over the grace's decayed tail) the flam is not found
    let (x, hits) = snare(10, &[(2.0, 0.025)], -70.0);
    let (env, _) = attack_env(&x, SR, &p);
    assert_eq!(second_onsets(&x, SR, &hits, &p, &env, &[]).len(), 1);
    let strict = FlamParams { rise_db: 80.0, ..p };
    assert!(second_onsets(&x, SR, &hits, &strict, &env, &[]).is_empty());
    // the guard: a next output hit 20 ms after the grace ends the search before the main stroke
    let mut close = hits.clone();
    close.push(2.03);
    assert!(second_onsets(&x, SR, &close, &p, &env, &[]).is_empty());
    // the floor: a re-rise that is not 12 dB over the noise before the hit is none
    let (x, hits) = snare(10, &[(2.0, 0.025)], -6.0);
    let (env, _) = attack_env(&x, SR, &p);
    assert!(second_onsets(&x, SR, &hits, &p, &env, &[]).is_empty());
}

#[test]
fn a_re_rise_another_mic_explains_is_bleed() {
    // own: single strokes; a source B (the hat) strokes 25 ms after four of them and at 12 other times, and the own mic
    // hears B 20 dB lower: those re-rises are bleed. Without the source they are added.
    let n = 24;
    let mut own = vec![0.0f64; (0.5 * f64::from(n + 2) * f64::from(SR)) as usize];
    let mut b = own.clone();
    let mut nz = Noise(99);
    let hits: Vec<f64> = (0..n).map(|i| 0.5 + 0.5 * f64::from(i)).collect();
    for &t in &hits {
        stroke(&mut own, t, 0.5, &mut nz);
    }
    for (i, &t) in hits.iter().enumerate() {
        let tb = if i % 6 == 0 { t + 0.025 } else { t + 0.25 };
        stroke(&mut b, tb, 1.0, &mut nz);
    }
    for (o, v) in own.iter_mut().zip(&b) {
        *o += 0.1 * v + 1e-5 * nz.next();
    }
    let (x, src) = (f32s(&own), f32s(&b));
    let (found, gains) = flam_split(&x, SR, &hits, &[&src], &FlamParams::default());
    let g = gains[0].expect("a gain");
    assert!((g + 20.0).abs() < 3.0, "{g}");
    assert_eq!(found.len(), 4, "{found:?}");
    assert!(found.iter().all(|s| s.bleed), "{found:?}");
    assert!(added(&found).is_empty());
    let (alone, _) = flam_split(&x, SR, &hits, &[], &FlamParams::default());
    assert_eq!(added(&alone).len(), 4);
}

#[test]
fn a_second_stroke_on_the_first_strokes_ring_never_starts_before_the_dips_next_block() {
    // the first stroke rings 20 ms (decay), the main stroke 25 ms after it: the ring is above the walk's threshold (33 dB
    // under the main stroke's peak) all the way back to the walk's bound, the block after the dip. The walk does not end
    // (busy) and the stroke stays at the rise block c, which is not before the bound: no clamp is needed (the old one
    // was dead; a `debug_assert` in `second_onsets` guards it). Exact: the block grid, not before the bound.
    let p = FlamParams::default();
    let mut x = vec![0.0f64; (0.5 * 12.0 * f64::from(SR)) as usize];
    let mut nz = Noise(0xfeed_beef_0123_4567);
    let hits: Vec<f64> = (0..10).map(|i| 0.5 + 0.5 * f64::from(i)).collect();
    for &t in &hits {
        stroke_tau(&mut x, t, 1.0, 0.02, 0.1, &mut nz);
        if (t - 2.0).abs() < 1e-9 || (t - 4.0).abs() < 1e-9 {
            stroke_tau(&mut x, t + 0.025, 1.5, 0.003, 0.04, &mut nz);
        }
    }
    for v in &mut x {
        *v += 1e-5 * nz.next();
    }
    let x = f32s(&x);
    let (env, hop) = attack_env(&x, SR, &p);
    let found = second_onsets(&x, SR, &hits, &p, &env, &[]);
    assert_eq!(found.len(), 2, "{found:?}");
    for (s, main) in found.iter().zip([2.025, 4.025]) {
        let at = (s.t * f64::from(SR)).round() as i64;
        assert!(at >= (s.dip + 1) * hop as i64, "not before the bound: {s:?}");
        assert_eq!(at % hop as i64, 0, "a block start: {s:?}");
        assert_eq!(
            at,
            (main * f64::from(SR)).round() as i64,
            "the main stroke's onset: {s:?}"
        );
    }
}

#[test]
fn the_late_peak_is_the_median_of_the_12_to_30_ms_peak_over_the_first_12_ms_and_gates_the_rule() {
    let p = FlamParams::default();
    let (sr, hop) = (48_000u32, 6usize); // 8000 blocks per second: 96 blocks = 12 ms, 240 = 30 ms
    let mut env = vec![1e-3f64; 8000];
    // hits at 0.1, 0.2, 0.3 s (blocks 800, 1600, 2400): the late peak is -6 dB, -3 dB and +6 dB under/over the first
    for (b, late) in [(800usize, 0.5f64), (1600, 10f64.powf(-3.0 / 20.0)), (2400, 2.0)] {
        env[b + 10] = 1.0;
        env[b + 150] = late;
    }
    let v = late_peak_db(&env, &[0.1, 0.2, 0.3], sr, hop, &p).unwrap();
    assert!((v + 3.0).abs() < 1e-9, "the median of -6.02, -3 and +6.02: {v}");
    let two = late_peak_db(&env, &[0.1, 0.3], sr, hop, &p).unwrap();
    assert!(
        (two - ((20.0 * 0.5f64.log10() + 20.0 * 2f64.log10()) / 2.0)).abs() < 1e-9,
        "{two}"
    );
    // a hit whose 30 ms window passes the end of the envelope or has no first peak counts for nothing
    assert_eq!(late_peak_db(&env, &[0.999, 5.0], sr, hop, &p), None);
    assert_eq!(late_peak_db(&vec![0.0; 8000], &[0.1], sr, hop, &p), None);
    assert_eq!(late_peak_db(&env, &[], sr, hop, &p), None);
    // the share W: hits whose late peak beats the first: 1 of 3
    assert!((late_share(&env, &[0.1, 0.2, 0.3], sr, hop, &p).unwrap() - 1.0 / 3.0).abs() < 1e-12);
    assert_eq!(late_share(&env, &[], sr, hop, &p), None);
    // the gate: A at -3 dB or more AND W at 0.4 or more; no value is not late-peaking
    assert!(
        late_peaking(Some(-3.0), Some(0.4))
            && late_peaking(Some(0.2), Some(0.94))
            && late_peaking(Some(6.4), Some(0.52))
    );
    assert!(
        !late_peaking(Some(-3.01), Some(0.9)) && !late_peaking(Some(-8.8), Some(0.0)) && !late_peaking(None, Some(1.0))
    );
    assert!(
        !late_peaking(Some(0.0), Some(0.39)) && !late_peaking(Some(0.0), None),
        "A alone is not enough"
    );
}

#[test]
fn a_top_like_mic_decays_early_and_a_wire_like_mic_peaks_late() {
    // 12 single strokes: top = a 3 ms decay (late peak far under the first); bottom = a weak first stroke and the wires'
    // louder rise 14 ms later (the late peak over the first)
    let p = FlamParams::default();
    let mut top = vec![0.0f64; (0.5 * 14.0 * f64::from(SR)) as usize];
    let mut bot = top.clone();
    let mut nz = Noise(77);
    let hits: Vec<f64> = (0..12).map(|i| 0.5 + 0.5 * f64::from(i)).collect();
    for &t in &hits {
        stroke(&mut top, t, 0.5, &mut nz);
        stroke(&mut bot, t, 0.2, &mut nz);
        stroke_tau(&mut bot, t + 0.014, 0.8, 0.006, 0.04, &mut nz);
    }
    for v in top.iter_mut().chain(bot.iter_mut()) {
        *v += 1e-5 * nz.next();
    }
    let late = |x: &[f64]| {
        let (env, hop) = attack_env(&f32s(x), SR, &p);
        late_peak_db(&env, &hits, SR, hop, &p)
    };
    let (t, b) = (late(&top).unwrap(), late(&bot).unwrap());
    let share = |x: &[f64]| {
        let (env, hop) = attack_env(&f32s(x), SR, &p);
        late_share(&env, &hits, SR, hop, &p)
    };
    let (ts, bs) = (share(&top).unwrap(), share(&bot).unwrap());
    assert!(t < -10.0 && !late_peaking(Some(t), Some(ts)), "top {t} {ts}");
    assert!(
        b > 3.0 && bs > 0.9 && late_peaking(Some(b), Some(bs)),
        "bottom {b} {bs}"
    );
}

#[test]
fn a_ringy_top_mic_with_a_slow_decay_is_not_late_peaking_although_its_a_is_over_the_gate() {
    // a 60 ms decay: on a monotone decay A is the decay at 12 ms (about -1.7 dB, over -3), but no hit peaks late (W 0)
    let p = FlamParams::default();
    let mut x = vec![0.0f64; (0.5 * 14.0 * f64::from(SR)) as usize];
    let mut nz = Noise(5);
    let hits: Vec<f64> = (0..12).map(|i| 0.5 + 0.5 * f64::from(i)).collect();
    for &t in &hits {
        stroke_tau(&mut x, t, 0.5, 0.06, 0.2, &mut nz);
    }
    let (env, hop) = attack_env(&f32s(&x), SR, &p);
    let a = late_peak_db(&env, &hits, SR, hop, &p).unwrap();
    let w = late_share(&env, &hits, SR, hop, &p).unwrap();
    assert!(a >= LATE_PEAK_MAX_DB, "planted: A alone would gate it: {a}");
    assert!(w < LATE_SHARE_MIN && !late_peaking(Some(a), Some(w)), "A {a} W {w}");
}

#[test]
fn the_gain_counts_the_quiet_hits_only_when_there_are_five() {
    // 10 strong blocks of B in 10 blocks of 50 ms at 8000 blocks per second. A hears B 20 dB lower at 6 of them (quiet:
    // no own hit within 30 ms). At the other 4, own hits lie 20 ms after the block (outside [-0.5, 8) ms of the window,
    // inside the 30 ms of "near"), and A's level there is 40 dB under B's. The gain is the 10th percentile of the
    // quiet 6: -20; over all 10 it would be -40.
    let rate = 8000.0;
    let mut b = vec![1e-4f64; 4000];
    for i in 0..10 {
        b[i * 400 + 100] = 1.0;
    }
    let peaks = strong_peaks(&b, rate);
    assert_eq!(peaks.len(), 10);
    let mut a: Vec<f64> = b.iter().map(|v| v * 0.1).collect();
    let mut own = Vec::new();
    for &p in &peaks[6..] {
        a[p] = 0.01;
        own.push(p as f64 / rate + 0.020);
    }
    let g = calibrate(&a, &b, &peaks, &own, rate).unwrap();
    assert!((g + 20.0).abs() < 1e-6, "the quiet hits only: {g}");
    // planted fault: with the own hits left out every hit is quiet and the 10th percentile is the near ones'
    let all = calibrate(&a, &b, &peaks, &[], rate).unwrap();
    assert!(all < -35.0, "{all}");
}

#[test]
fn the_walk_does_not_move_a_second_stroke_back_over_a_ring_before_the_dip() {
    // a first stroke at 0.5 s (3 kHz tone, 3 ms decay, 10 ms long); the second stroke 30 ms after it; a ring above the walk
    // threshold (33 dB under the stroke's peak) but under the rise test's level line (30 dB), from 4.5 ms before the stroke
    // up to it, 14.5 ms of silence before it. The walk with its bound (the block after the dip) stays at the rise; without
    // the bound it would end at the silence, 4.5 ms early. Plant: `prev_mid` None in `second_onsets` fails this.
    let p = FlamParams::default();
    let mut x = vec![0.0f64; SR as usize];
    let tone = |x: &mut [f64], t: f64, amp: f64, tau: f64, len: f64| {
        let i0 = (t * f64::from(SR)).round() as usize;
        for k in 0..(len * f64::from(SR)) as usize {
            let s = k as f64 / f64::from(SR);
            x[i0 + k] += amp * (2.0 * std::f64::consts::PI * 3000.0 * s).sin() * (-s / tau).exp();
        }
    };
    let c = 0.530;
    tone(&mut x, 0.5, 1.0, 0.003, 0.010);
    tone(&mut x, c - 0.0045, 0.027, 1e9, 0.0045);
    tone(&mut x, c, 1.0, 0.003, 0.02);
    let x = f32s(&x);
    let (env, hop) = attack_env(&x, SR, &p);
    let found = second_onsets(&x, SR, &[0.5], &p, &env, &[]);
    assert_eq!(found.len(), 1, "{found:?}");
    let s = &found[0];
    let at = (s.t * f64::from(SR)).round() as i64;
    let bound = (s.dip + 1) * hop as i64;
    let (free, _) = crate::refine::first_motion_mid(&x, (c * f64::from(SR)).round() as i64, SR, None, None);
    assert!(free < bound, "the unbounded walk ends before the bound: {free} {bound}");
    assert!(at >= bound, "the bounded walk does not: {at} {bound} {s:?}");
}
