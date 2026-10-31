use super::*;

fn sine(sr: u32, f: f64, amp: f64, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (amp * (2.0 * std::f64::consts::PI * f * i as f64 / f64::from(sr)).sin()) as f32)
        .collect()
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() / x.len() as f64).sqrt()
}

/// Interior of the output: skip the kernel half length at both ends.
fn interior(y: &[f32]) -> &[f32] {
    let edge = (HALF_LENGTH_S * 44_100.0).ceil() as usize + 2;
    &y[edge..y.len() - edge]
}

fn expected_taps(sr: u32) -> usize {
    2 * ((HALF_LENGTH_S * f64::from(sr)).ceil() as usize + 1)
}

#[test]
fn passes_44100_through_unchanged() {
    let x = sine(44_100, 440.0, 0.5, 10_000);
    let y = resample_to_44100(&x, 44_100).unwrap();
    assert_eq!(x, y);
    assert_eq!(Resampler::new(44_100).unwrap().taps_per_phase(), 0);
    assert_eq!(Resampler::shared(44_100).unwrap().source_rate(), 44_100);
}

#[test]
fn output_length_is_ceil_n_times_147_over_m() {
    let r48 = Resampler::new(48_000).unwrap();
    let r96 = Resampler::new(96_000).unwrap();
    for n in [0usize, 1, 100, 101, 147, 160, 161, 1000, 4800, 48_000, 48_001] {
        let want48 = (n * 147).div_ceil(160);
        let want96 = (n * 147).div_ceil(320);
        assert_eq!(r48.output_len(n), want48, "n={n}");
        assert_eq!(r96.output_len(n), want96, "n={n}");
        assert_eq!(r48.process(&vec![0.1; n]).len(), want48, "n={n}");
        assert_eq!(r96.process(&vec![0.1; n]).len(), want96, "n={n}");
    }
    // librosa.resample(fix=True) values measured with soxr 1.1.0: 100 -> 92, 1000 -> 919.
    assert_eq!(r48.output_len(100), 92);
    assert_eq!(r48.output_len(1000), 919);
    assert_eq!(r48.output_len(48_001), 44_101);
}

/// R14: an extreme public-helper input saturates instead of overflowing
/// (debug panic / release wrap).
#[test]
fn output_len_saturates_for_extreme_inputs() {
    for sr in [44_100u32, 48_000, 96_000] {
        let r = Resampler::new(sr).unwrap();
        assert_eq!(r.output_len(usize::MAX), usize::MAX.div_ceil(r.m), "sr={sr}");
        assert_eq!(
            r.output_len(usize::MAX / PHASES),
            (usize::MAX / PHASES * PHASES).div_ceil(r.m),
            "sr={sr}"
        );
    }
}

#[test]
fn unsupported_rates_error() {
    for sr in [0u32, 22_050, 32_000, 88_200, 176_400, 192_000] {
        assert_eq!(Resampler::new(sr).unwrap_err(), ResampleError::UnsupportedRate(sr));
        assert_eq!(Resampler::shared(sr).unwrap_err(), ResampleError::UnsupportedRate(sr));
        assert!(resample_to_44100(&[0.0; 10], sr).is_err());
    }
    assert_eq!(
        ResampleError::UnsupportedRate(32_000).to_string(),
        "resample: unsupported source rate 32000 Hz (44100, 48000 or 96000)"
    );
}

#[test]
fn table_rows_sum_to_one_and_are_symmetric_in_phase_zero() {
    let r = Resampler::new(48_000).unwrap();
    assert_eq!(r.taps_per_phase(), expected_taps(48_000));
    assert_eq!(r.taps_per_phase(), 232);
    for p in 0..PHASES {
        let row = &r.table[p * r.taps..(p + 1) * r.taps];
        let s: f64 = row.iter().sum();
        assert!((s - 1.0).abs() < 1e-14, "phase {p} sums to {s}");
    }
    // Phase 0 is g(j - half), symmetric about j = half.
    let row = &r.table[..r.taps];
    for j in 1..r.half {
        assert!((row[r.half + j] - row[r.half - j]).abs() < 1e-15);
    }
    // The two rates sample the same continuous kernel: phase 0 of 96 kHz at even
    // offsets equals phase 0 of 48 kHz, up to the per-phase normalisation (about 1e-8).
    let r96 = Resampler::new(96_000).unwrap();
    assert_eq!(r96.taps_per_phase(), expected_taps(96_000));
    assert_eq!(r96.taps_per_phase(), 462);
    let row96 = &r96.table[..r96.taps];
    for j in 0..r.half {
        let g48 = row[r.half + j];
        let g96 = 2.0 * row96[r96.half + 2 * j];
        assert!((g48 - g96).abs() < 1e-7 * g48.abs().max(1e-3), "j {j}: {g48} vs {g96}");
    }
}

#[test]
fn shared_table_equals_a_fresh_one() {
    let fresh = Resampler::new(96_000).unwrap();
    let shared = Resampler::shared(96_000).unwrap();
    assert_eq!(fresh.table, shared.table);
    assert_eq!(fresh.taps, shared.taps);
}

#[test]
fn dc_passes() {
    for sr in [48_000u32, 96_000] {
        let x = vec![0.25f32; sr as usize / 10];
        let y = resample_to_44100(&x, sr).unwrap();
        for (k, &v) in interior(&y).iter().enumerate() {
            assert!((f64::from(v) - 0.25).abs() < 1e-6, "sr {sr} k {k} v {v}");
        }
    }
}

/// A 1 kHz and a 10 kHz sine keep amplitude and frequency: compare the interior with the
/// ideal sine at 44.1 kHz (zero group delay). The design follows `soxr_hq`'s passband
/// droop (about -0.00002 dB at 10 kHz), so the bound is 1e-5 absolute at amplitude 0.5;
/// the interpolated zero crossings give the frequency within 0.01 % and the projection on
/// the ideal sine the amplitude within 1e-5.
#[test]
fn sine_keeps_frequency_and_amplitude() {
    for sr in [48_000u32, 96_000] {
        for f in [1_000.0, 10_000.0] {
            let n = sr as usize; // 1 s
            let x = sine(sr, f, 0.5, n);
            let y = resample_to_44100(&x, sr).unwrap();
            let ideal = sine(44_100, f, 0.5, y.len());
            let edge = (HALF_LENGTH_S * 44_100.0).ceil() as usize + 2;
            let mut max_err = 0.0f64;
            for k in edge..y.len() - edge {
                max_err = max_err.max((f64::from(y[k]) - f64::from(ideal[k])).abs());
            }
            assert!(max_err < 1e-5, "sr {sr} f {f}: max abs error {max_err}");
            let yi = interior(&y);
            // Upward zero crossings with linear interpolation; cycles / span = frequency.
            let ups: Vec<f64> = yi
                .windows(2)
                .enumerate()
                .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
                .map(|(i, w)| i as f64 + f64::from(-w[0]) / f64::from(w[1] - w[0]))
                .collect();
            let f_est = (ups.len() - 1) as f64 * 44_100.0 / (ups[ups.len() - 1] - ups[0]);
            assert!((f_est - f).abs() / f < 1e-4, "sr {sr} f {f}: estimated {f_est}");
            // Amplitude by projection on the ideal sine (a whole-cycle RMS is biased).
            let (mut num, mut den) = (0.0f64, 0.0f64);
            for k in edge..y.len() - edge {
                num += f64::from(y[k]) * f64::from(ideal[k]);
                den += f64::from(ideal[k]) * f64::from(ideal[k]);
            }
            let a = 0.5 * num / den;
            assert!((a - 0.5).abs() < 1e-5, "sr {sr} f {f}: amplitude {a}");
        }
    }
}

/// Tones above the output Nyquist are attenuated by at least 80 dB (the design is below
/// -137 dB from 22.05 kHz on).
#[test]
fn tones_above_22050_are_attenuated_80_db() {
    for (sr, f) in [
        (48_000u32, 22_500.0),
        (48_000, 23_000.0),
        (96_000, 22_500.0),
        (96_000, 30_000.0),
        (96_000, 47_000.0),
    ] {
        let x = sine(sr, f, 0.5, sr as usize);
        let y = resample_to_44100(&x, sr).unwrap();
        let r_in = 0.5 / std::f64::consts::SQRT_2;
        let r_out = rms(interior(&y));
        let db = 20.0 * (r_out / r_in).log10();
        assert!(db <= -80.0, "sr {sr} f {f}: {db:.1} dB");
    }
}

#[test]
fn zero_padding_at_the_edges_and_empty_input() {
    let r = Resampler::new(48_000).unwrap();
    assert!(r.process(&[]).is_empty());
    let y = r.process(&[1.0]);
    assert_eq!(y.len(), 1);
    // The single input sample at t = 0 lands on output 0 with the kernel's centre weight.
    assert!((f64::from(y[0]) - r.table[r.half]).abs() < 1e-7);
    let y = r.process(&[0.0; 5]);
    assert!(y.iter().all(|&v| v == 0.0));
}

#[test]
fn same_result_for_any_split_and_thread_count() {
    // The parallel blocks do not change the bits: compare with a serial evaluation.
    let r = Resampler::new(48_000).unwrap();
    let x = sine(48_000, 3_000.0, 0.7, 20_000);
    let y = r.process(&x);
    for (k, &v) in y.iter().enumerate() {
        let km = k * r.m;
        assert_eq!(v, r.sample(&x, km / PHASES, km % PHASES));
    }
}
