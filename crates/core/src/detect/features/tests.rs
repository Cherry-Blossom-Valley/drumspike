use super::*;

/// The 84 CLI centre bins (`docs/archive/phase2-report.md`, P2-1c).
fn cli_centres() -> Vec<usize> {
    let mut c: Vec<usize> = (2..=22).collect();
    c.extend([
        24, 25, 26, 28, 30, 31, 33, 35, 37, 40, 42, 45, 47, 50, 53, 56, 59, 63, 67, 71, 75, 79, 84, 89, 94, 100, 106,
        112, 119, 126, 133, 141, 150, 159, 168, 178, 189, 200, 212, 224, 238, 252, 267, 283, 300, 317, 336, 356, 377,
        400, 424, 449, 476, 504, 534, 566, 599, 635, 673, 713, 755, 800, 847,
    ]);
    c
}

/// The 84 madmom centre bins (the same report section).
fn madmom_centres() -> Vec<usize> {
    let mut c: Vec<usize> = (2..=20).collect();
    c.extend([
        22, 23, 24, 26, 27, 29, 31, 32, 34, 36, 39, 41, 43, 46, 49, 51, 55, 58, 61, 65, 69, 73, 77, 82, 87, 92, 97,
        103, 109, 116, 122, 130, 137, 146, 154, 163, 173, 183, 194, 206, 218, 231, 245, 259, 275, 291, 309, 327, 346,
        367, 389, 412, 436, 462, 490, 519, 550, 583, 617, 654, 693, 734, 778, 824, 873,
    ]);
    c
}

#[test]
fn centre_bins_pin_both_grids() {
    let (cli, mad) = (cli_centres(), madmom_centres());
    assert_eq!((cli.len(), mad.len()), (N_BANDS, N_BANDS));
    assert_eq!(centre_bins(Chain::Cli), &cli[..]);
    assert_eq!(centre_bins(Chain::Madmom), &mad[..]);
    // The outer edges of the first and last filter.
    let (bc, bm) = (filterbank_bins(Chain::Cli), filterbank_bins(Chain::Madmom));
    assert_eq!((bc.len(), bc[0], bc[85]), (N_EDGE_BINS, 1, 898));
    assert_eq!((bm.len(), bm[0], bm[85]), (N_EDGE_BINS, 1, 925));
    // 64 of the 84 centres differ (P2-1c).
    let same = cli.iter().zip(&mad).filter(|(a, b)| a == b).count();
    assert_eq!(N_BANDS - same, 64);
}

#[test]
fn bin_frequency_and_band_centres_follow_fftfreq() {
    assert_eq!(bin_frequency(0), 0.0);
    assert!((bin_frequency(1) - 44_100.0 / 2048.0).abs() < 1e-12);
    assert!((bin_frequency(1023) - 1023.0 * 44_100.0 / 2048.0).abs() < 1e-9);
    for chain in Chain::ALL {
        let hz = band_centres_hz(chain);
        assert_eq!(hz.len(), N_BANDS);
        assert_eq!(hz[0], bin_frequency(2));
        assert!(hz.windows(2).all(|w| w[0] < w[1]));
    }
}

#[test]
fn frame_count_rule_of_each_chain() {
    let cli = [
        (0, 1),
        (1, 1),
        (440, 1),
        (441, 2),
        (442, 2),
        (44_100, 101),
        (44_101, 101),
    ];
    let mad = [
        (0, 0),
        (1, 1),
        (440, 1),
        (441, 1),
        (442, 2),
        (44_100, 100),
        (44_101, 101),
    ];
    for (n, want) in cli {
        assert_eq!(num_frames(n, Chain::Cli), want, "cli n={n}");
        assert_eq!(features(&vec![0.1; n], Chain::Cli).len(), want * N_BANDS);
    }
    for (n, want) in mad {
        assert_eq!(num_frames(n, Chain::Madmom), want, "madmom n={n}");
        assert_eq!(features(&vec![0.1; n], Chain::Madmom).len(), want * N_BANDS);
    }
}

#[test]
fn filters_are_normalised_triangles() {
    for chain in Chain::ALL {
        let fs = filters(chain);
        let bins = filterbank_bins(chain);
        assert_eq!(fs.len(), N_BANDS);
        for (b, f) in fs.iter().enumerate() {
            assert_eq!(f.start, bins[b]);
            assert_eq!(f.weights.len(), bins[b + 2] - bins[b]);
            assert_eq!(f.weights[0], 0.0);
            let sum: f64 = f.weights.iter().map(|&w| f64::from(w)).sum();
            assert!((sum - 1.0).abs() < 1e-6, "{chain:?} band {b}: sum {sum}");
            // The peak is at the centre bin.
            let peak = (0..f.weights.len())
                .max_by(|&i, &j| f.weights[i].total_cmp(&f.weights[j]).then(j.cmp(&i)))
                .unwrap();
            assert_eq!(f.start + peak, bins[b + 1], "{chain:?} band {b}");
        }
    }
}

#[test]
fn hanning_matches_numpy() {
    let w = hanning();
    assert_eq!(w[0], 0.0);
    assert_eq!(w[FRAME_SIZE - 1], 0.0);
    // Symmetric, the maximum between the two middle samples.
    for i in 0..FRAME_SIZE {
        assert!((w[i] - w[FRAME_SIZE - 1 - i]).abs() < 1e-15);
    }
    let want = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * 1023.0 / 2047.0).cos();
    assert!((w[1023] - want).abs() < 1e-15);
}

/// A sine at the centre frequency of a band peaks in that band, with the value the
/// windowed DFT predicts, in both chains.
#[test]
fn sine_at_a_known_bin() {
    for chain in Chain::ALL {
        // Band 60: a centre bin above the region where the grids give one band per bin.
        let k = centre_bins(chain)[60];
        let f = bin_frequency(k);
        let n = 44_100;
        let amp = 0.5f64;
        let x: Vec<f32> = (0..n)
            .map(|i| (amp * (2.0 * std::f64::consts::PI * f * i as f64 / 44_100.0).sin()) as f32)
            .collect();
        let spec = features(&x, chain);
        let frame = 50; // fully inside the signal
        let row = &spec[frame * N_BANDS..(frame + 1) * N_BANDS];
        let best = (0..N_BANDS).max_by(|&a, &b| row[a].total_cmp(&row[b])).unwrap();
        assert_eq!(best, 60, "{chain:?}");
        // |X[k]| of a sine on bin k under a Hann window is amp * sum(w) / 2; the filter
        // weight there is the peak weight of the triangle.
        let fs = &filters(chain)[60];
        let peak_w = f64::from(fs.weights[k - fs.start]);
        let wsum: f64 = hanning().iter().sum();
        let side: f64 = [k - 1, k + 1]
            .iter()
            .map(|&j| f64::from(fs.weights[j - fs.start]) * amp * wsum / 4.0)
            .sum();
        let want = (1.0 + peak_w * amp * wsum / 2.0 + side).log10();
        let got = f64::from(row[60]);
        assert!((got - want).abs() < 2e-3, "{chain:?}: {got} vs {want}");
    }
}

/// The paired FFT gives the same rows as a direct per-frame f64 FFT, and silence gives
/// zeros.
#[test]
fn paired_fft_matches_a_direct_fft() {
    let n = 441 * 37 + 100;
    let x: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f64 / 44_100.0;
            ((2.0 * std::f64::consts::PI * 997.0 * t).sin() * 0.5 + (2.0 * std::f64::consts::PI * 80.0 * t).sin() * 0.3)
                as f32
        })
        .collect();
    let fft = FftPlanner::<f64>::new().plan_fft_forward(FRAME_SIZE);
    for chain in Chain::ALL {
        let spec = features(&x, chain);
        assert_eq!(spec.len(), num_frames(n, chain) * N_BANDS);
        let mut frame = vec![0f64; FRAME_SIZE];
        for i in [0usize, 5, 18, num_frames(n, chain) - 1] {
            fill_frame(&x, i, chain, &mut frame);
            let mut buf: Vec<Complex<f64>> = frame.iter().map(|&v| Complex::new(v, 0.0)).collect();
            fft.process(&mut buf);
            let mag: Vec<f32> = buf[..N_FFT_BINS].iter().map(|&z| mag_c64(z)).collect();
            let mut row = vec![0f32; N_BANDS];
            bands(filters(chain), &mag, &mut row);
            for (b, &want) in row.iter().enumerate() {
                let got = spec[i * N_BANDS + b];
                assert!(
                    (got - want).abs() <= 1e-6,
                    "{chain:?} frame {i} band {b}: {got} vs {want}"
                );
            }
        }
        assert!(features(&vec![0f32; 5000], chain).iter().all(|&v| v == 0.0));
    }
}

/// [`Chain::Mix`] computes the CLI features, bit for bit.
#[test]
fn mix_uses_the_cli_recipe() {
    let x: Vec<f32> = (0..20_000).map(|i| ((i as f32) * 0.013).sin() * 0.4).collect();
    assert_eq!(features(&x, Chain::Mix), features(&x, Chain::Cli));
    assert_eq!(centre_bins(Chain::Mix), centre_bins(Chain::Cli));
    assert_eq!(num_frames(441, Chain::Mix), num_frames(441, Chain::Cli));
}

#[test]
fn empty_signal() {
    assert_eq!(features(&[], Chain::Cli), vec![0f32; N_BANDS]);
    assert!(features(&[], Chain::Madmom).is_empty());
}
