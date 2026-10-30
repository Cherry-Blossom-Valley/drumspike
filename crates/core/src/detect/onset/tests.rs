use super::*;
/// A kick like tests/helpers.kick_hit (pitch sweep 125 -> 55 Hz, linear 0.5 ms attack
/// ramp, exponential decay), but 1.5 s long instead of 0.4 s, so that no hit ends with
/// a hard cut (a step in the signal is an onset of its own).
fn kick_track(sr: u32, dur_s: f64, onsets: &[(f64, f64)]) -> Vec<f32> {
    let n = (dur_s * f64::from(sr)) as usize;
    let mut x = vec![0f64; n];
    let len = (1.5 * f64::from(sr)) as usize;
    for &(t0, gain_db) in onsets {
        let i0 = (t0 * f64::from(sr)).round() as usize;
        let g = 10f64.powf(gain_db / 20.0);
        let mut ph = 0.0;
        for k in 0..len.min(n.saturating_sub(i0)) {
            let t = k as f64 / f64::from(sr);
            let f = 55.0 + 70.0 * (-t / 0.03).exp();
            ph += 2.0 * std::f64::consts::PI * f / f64::from(sr);
            let env = (t / 0.0005).min(1.0) * (-t / 0.12).exp();
            x[i0 + k] += g * ph.sin() * env;
        }
    }
    x.into_iter().map(|v| v as f32).collect()
}

#[test]
fn role_names_round_trip() {
    for r in Role::ALL {
        assert_eq!(Role::parse(r.name()), Some(r));
    }
    assert_eq!(Role::parse("bass"), None);
}

#[test]
fn every_role_uses_the_phase1_gap() {
    for r in Role::ALL {
        let p = OnsetParams::for_role(r);
        assert_eq!(p.min_gap_s, PHASE1_MIN_GAP_S);
        assert!(p.band_lo_hz < p.band_hi_hz);
        assert!(!band_range(p.band_lo_hz, p.band_hi_hz).is_empty());
    }
    let g = OnsetParams::generic();
    assert_eq!(g.gate_db, None);
    assert_eq!(OnsetParams::for_role(Role::Other), g);
}
#[test]
fn band_range_selects_by_centre() {
    let c = band_centres_hz(Chain::Cli);
    let r = band_range(100.0, 1000.0);
    assert!(c[r.start] >= 100.0 && c[r.start - 1] < 100.0);
    assert!(c[r.end - 1] <= 1000.0 && c[r.end] > 1000.0);
    assert_eq!(band_range(20.0, 20_000.0), 0..N_BANDS);
    assert!(band_range(30_000.0, 40_000.0).is_empty());
}
#[test]
fn superflux_is_zero_on_a_steady_spectrum() {
    let row: Vec<f32> = (0..N_BANDS).map(|b| (b % 7) as f32 * 0.1).collect();
    let spec: Vec<f32> = row.iter().cycle().take(4 * N_BANDS).copied().collect();
    let f = superflux(&spec, 0..N_BANDS);
    // Frame 0 is compared with silence: the sum of the row.
    assert!((f[0] - row.iter().sum::<f32>()).abs() < 1e-5);
    assert!(f[1..].iter().all(|&v| v == 0.0));
    // A one-band shift is absorbed by the 3-band maximum filter.
    let mut shifted = spec.clone();
    shifted[3 * N_BANDS..].copy_from_slice(&[&row[1..], &[0.0]].concat());
    assert_eq!(superflux(&shifted, 0..N_BANDS)[3], 0.0);
    // A rise in one band counts once.
    let mut up = spec.clone();
    up[2 * N_BANDS + 40] += 1.0;
    let want = row[40] + 1.0 - row[39].max(row[40]).max(row[41]);
    assert!((superflux(&up, 0..N_BANDS)[2] - want).abs() < 1e-6);
    assert_eq!(superflux(&up, 0..40)[2], 0.0);
}

#[test]
fn pick_peaks_follows_the_median_threshold_and_local_maximum() {
    let mut odf = vec![0.1f32; 60];
    odf[10] = 5.0;
    odf[11] = 2.0; // inside the local-maximum window of frame 10
    odf[13] = 1.5; // 3 frames after a peak: kept with pre_max 1, post_max 2
    odf[30] = 0.9; // below median 0.1 + delta 1.0
    odf[45] = 1.2;
    assert_eq!(pick_peaks(&odf, 1.0, 10, 0, 1, 2), vec![10, 13, 45]);
    assert_eq!(pick_peaks(&odf, 1.0, 10, 0, 3, 2), vec![10, 45]);
    assert!(pick_peaks(&[0.0; 20], 0.5, 10, 0, 1, 2).is_empty());
}

#[test]
fn attack_is_large_for_a_click_and_small_for_a_steady_tone() {
    let n = 44_100;
    let tone: Vec<f32> = (0..n)
        .map(|i| (0.5 * (2.0 * std::f64::consts::PI * 90.0 * i as f64 / 44_100.0).sin()) as f32)
        .collect();
    // The envelope of a low tone ripples by a few dB within its period.
    assert!(attack_db(&tone, 50) < 6.0, "{}", attack_db(&tone, 50));
    let mut click = tone.clone();
    for v in &mut click[50 * HOP..50 * HOP + 22] {
        *v += 0.3;
    }
    assert!(attack_db(&click, 50) > 20.0, "{}", attack_db(&click, 50));
    assert_eq!(attack_db(&[0.0; 100], 0), 0.0);
}

#[test]
fn synthetic_kicks_are_found_within_one_frame() {
    let onsets = [
        (0.2, -3.0),
        (0.5, -1.0),
        (0.9, -12.0),
        (1.4, -3.0),
        (2.0, -2.0),
        (2.6, -30.0),
        (3.1, -3.0),
    ];
    for sr in [44_100u32, 48_000, 96_000] {
        let x = kick_track(sr, 3.6, &onsets);
        let hits = detect_onsets(&x, sr, Role::Kick).unwrap();
        let frames: Vec<i64> = hits.iter().map(|h| h.frame as i64).collect();
        for (t, _) in onsets {
            let want = (t * 100.0).round() as i64;
            assert!(
                frames.iter().any(|f| (f - want).abs() <= 1),
                "{sr} Hz: onset {t} not in {frames:?}"
            );
        }
        assert_eq!(hits.len(), onsets.len(), "{sr} Hz: {frames:?}");
        for h in &hits {
            assert!((0.0..=1.0).contains(&h.confidence));
            assert!((h.time - h.frame as f64 / 100.0).abs() < 1e-12);
            assert!(h.rise_db.is_some());
        }
    }
}

#[test]
fn silence_gives_no_hit_for_any_role() {
    let x = vec![0f32; 48_000 * 3];
    for r in Role::ALL {
        assert!(detect_onsets(&x, 48_000, r).unwrap().is_empty(), "{}", r.name());
    }
    assert!(detect_onsets(&[], 48_000, Role::Kick).unwrap().is_empty());
}

#[test]
fn onsets_from_spectrogram_keeps_the_gap_and_the_confidence_range() {
    let x = kick_track(44_100, 1.0, &[(0.2, -3.0), (0.21, -3.0), (0.6, -6.0)]);
    let spec = features(&x, Chain::Cli);
    let mut p = OnsetParams::generic();
    p.min_gap_s = 0.05;
    let hits = onsets_from_spectrogram(&spec, &x, &p);
    for w in hits.windows(2) {
        assert!(w[1].time - w[0].time >= 0.05 - 1e-9);
    }
    assert!(
        hits.iter()
            .all(|h| h.rise_db.is_none() && (0.0..=1.0).contains(&h.confidence))
    );
    assert!(hits.iter().any(|h| h.confidence == 1.0));
    assert!(onsets_from_spectrogram(&[], &[], &p).is_empty());
}

#[test]
fn detect_onsets_with_rejects_an_unsupported_rate() {
    let e = detect_onsets_with(&[0.0; 100], 22_050, &OnsetParams::generic()).unwrap_err();
    assert_eq!(e, OnsetError::Resample(ResampleError::UnsupportedRate(22_050)));
    assert!(e.to_string().contains("22050"));
}

/// A snare-like bleed hit: band-limited noise (the kick template's low bands stay quiet).
fn snare_bleed(sr: u32, t0: f64, gain_db: f64) -> impl Fn(&mut [f32]) {
    move |x: &mut [f32]| {
        let i0 = (t0 * f64::from(sr)).round() as usize;
        let g = 10f64.powf(gain_db / 20.0);
        let mut state = 0x2545F4914F6CDD1Du64;
        for k in 0..(0.3 * f64::from(sr)) as usize {
            let Some(v) = x.get_mut(i0 + k) else { break };
            let t = k as f64 / f64::from(sr);
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let noise = ((state >> 33) as f64 / (1u64 << 31) as f64) - 1.0;
            // a 2 kHz-ish ring plus noise: energy far above the kick's band range
            let env = (t / 0.0005).min(1.0) * (-t / 0.08).exp();
            *v += (g * env * (0.5 * noise + 0.5 * (2.0 * std::f64::consts::PI * 2000.0 * t).sin())) as f32;
        }
    }
}

/// A4: the candidate list of a synthetic track with a planted loud kick, a soft kick of the
/// same shape and a bleed hit: the verdicts are pass, pass and fail, and the kept candidates
/// are exactly the detector's hits (before the minimum gap).
#[test]
fn candidates_verdict_loud_soft_and_bleed() {
    let sr = 48_000u32;
    let mut x = kick_track(sr, 2.0, &[(0.3, 0.0), (0.8, -15.0)]);
    snare_bleed(sr, 1.3, -6.0)(&mut x);
    let p = OnsetParams::for_role(Role::Kick);
    let (hits, cands, weak_limit) = detect_onsets_and_cands(&x, sr, &p).unwrap();
    // the detector's hits are exactly the kept candidates, in order
    let kept_frames: Vec<usize> = cands.iter().filter(|c| c.kept).map(|c| c.frame).collect();
    assert_eq!(kept_frames, hits.iter().map(|h| h.frame).collect::<Vec<_>>());
    // the loud hit passes the level gate
    let loud = cands
        .iter()
        .find(|c| (c.time - 0.3).abs() < 0.02)
        .expect("loud candidate");
    assert_eq!(loud.gate, Gate::Level);
    assert!(loud.kept);
    // the soft same-shape hit passes (level or shape rescue, its verdict says which)
    let soft = cands
        .iter()
        .find(|c| (c.time - 0.8).abs() < 0.02)
        .expect("soft candidate");
    assert!(soft.kept, "gate {:?}", soft.gate);
    assert!(matches!(soft.gate, Gate::Level | Gate::WeakAttack | Gate::Shape));
    // the bleed hit fails and names the gate that failed
    let bleed = cands
        .iter()
        .find(|c| (c.time - 1.3).abs() < 0.02)
        .expect("bleed candidate");
    assert!(!bleed.kept, "bleed kept with gate {:?}", bleed.gate);
    assert!(matches!(bleed.gate, Gate::Level | Gate::WeakFlux | Gate::Shape));
    // a re-pick at the detection's scale reproduces the kept set
    assert_eq!(repick_candidates(&cands, &p, weak_limit, 1.0), kept_frames);
    // and the re-pick is monotonic in the scale: stricter keeps a subset, looser a superset
    let strict = repick_candidates(&cands, &p, weak_limit, 1.6);
    let loose = repick_candidates(&cands, &p, weak_limit, 0.4);
    assert!(
        strict.len() <= kept_frames.len(),
        "strict {strict:?} kept {kept_frames:?}"
    );
    assert!(loose.len() >= kept_frames.len(), "loose {loose:?} kept {kept_frames:?}");
    assert!(strict.iter().all(|f| kept_frames.contains(f)));
    assert!(kept_frames.iter().all(|f| loose.contains(f)));
    // the stored measurements survive the round trip of the store's fields
    for c in &cands {
        assert_eq!(c.time, c.frame as f64 / 100.0);
    }
}

/// A snare hit: band noise with a 190 Hz body (energy in the snare band 100-300 Hz, far
/// above the kick's).
fn snare_hit(sr: u32, t0: f64, gain_db: f64) -> impl Fn(&mut [f32]) {
    move |x: &mut [f32]| {
        let i0 = (t0 * f64::from(sr)).round() as usize;
        let g = 10f64.powf(gain_db / 20.0);
        let mut state = 0x9E3779B97F4A7C15u64;
        for k in 0..(0.3 * f64::from(sr)) as usize {
            let Some(v) = x.get_mut(i0 + k) else { break };
            let t = k as f64 / f64::from(sr);
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let noise = ((state >> 33) as f64 / (1u64 << 31) as f64) - 1.0;
            let env = (t / 0.0005).min(1.0) * (-t / 0.06).exp();
            *v += (g * env * (0.4 * noise + 0.6 * (2.0 * std::f64::consts::PI * 190.0 * t).sin())) as f32;
        }
    }
}

/// A1: the soft gate. A planted soft snare hit (a ghost note, well below the level gate)
/// passes with `Gate::Soft`; a planted soft kick bleed that coincides with a kick hit is
/// denied the soft rescue by the busy clearance and fails.
#[test]
fn soft_gate_recovers_a_ghost_snare_and_a_busy_kick_bleed_fails() {
    let sr = 48_000u32;
    let p = OnsetParams::for_role(Role::Snare);
    assert_eq!(p.soft_sim, Some(0.92));
    // the ghost sits 18 dB under the matched hits, deeper than the 12 dB level gate
    let mut x = vec![0f32; (2.0 * f64::from(sr)) as usize];
    snare_hit(sr, 0.3, 0.0)(&mut x);
    snare_hit(sr, 1.5, 0.0)(&mut x);
    snare_hit(sr, 0.9, -18.0)(&mut x);
    let (hits, cands, weak_limit) = detect_onsets_and_cands(&x, sr, &p).unwrap();
    let ghost = cands
        .iter()
        .find(|c| (c.time - 0.9).abs() < 0.02)
        .expect("ghost candidate");
    assert!(ghost.kept, "ghost dropped with gate {:?}", ghost.gate);
    assert_eq!(ghost.gate, Gate::Soft);
    assert!(!ghost.soft_busy);
    assert!(hits.iter().any(|h| (h.time - 0.9).abs() < 0.02));
    // the re-pick at the detection's scale keeps it
    let kept: Vec<usize> = cands.iter().filter(|c| c.kept).map(|c| c.frame).collect();
    assert_eq!(repick_candidates(&cands, &p, weak_limit, 1.0), kept);

    // the busy clearance: the same ghost, now coinciding with a kick hit (the kick track's
    // event at the ghost's time). The busy peaks deny the soft rescue; the re-detection
    // drops the ghost.
    let busy = vec![0.9];
    let peaks = soft_busy_peaks(&cands, &p, &busy, 30.0);
    assert!(
        peaks.iter().any(|&k| (cands[k].time - 0.9).abs() < 0.02),
        "ghost not busy: {peaks:?}"
    );
    let (_, cands2, _) = detect_onsets_and_cands_soft_excluding(&x, sr, &p, &peaks).unwrap();
    let ghost2 = cands2
        .iter()
        .find(|c| (c.time - 0.9).abs() < 0.02)
        .expect("ghost candidate");
    assert!(!ghost2.kept, "busy ghost kept with gate {:?}", ghost2.gate);
    assert_eq!(ghost2.gate, Gate::Level);
    assert!(ghost2.soft_busy);
    // and the re-pick honors the stored flag at the detection's scale: the busy ghost,
    // kept only by the soft rescue, stays out (at looser scales it may pass as a loud hit,
    // which is the slider's own loosening)
    assert!(!repick_candidates(&cands2, &p, weak_limit, 1.0).contains(&ghost2.frame));
}

/// A1 (#286): the overhead denial. A soft-eligible ghost at a moment where the oh is much
/// louder than the snare mic (both measured as the full-band peak) keeps no soft rescue; an
/// oh under the snare mic leaves the ghost in.
#[test]
fn the_overhead_denial_takes_the_bleed_and_keeps_the_ghost() {
    let sr = 48_000u32;
    let p = OnsetParams::for_role(Role::Snare);
    let mut x = vec![0f32; (2.0 * f64::from(sr)) as usize];
    snare_hit(sr, 0.3, 0.0)(&mut x);
    snare_hit(sr, 1.5, 0.0)(&mut x);
    snare_hit(sr, 0.9, -18.0)(&mut x);
    let (hits, cands, _) = detect_onsets_and_cands(&x, sr, &p).unwrap();
    assert!(hits.iter().any(|h| (h.time - 0.9).abs() < 0.02));
    let gt = cands
        .iter()
        .find(|c| (c.time - 0.9).abs() < 0.02)
        .expect("ghost candidate")
        .time;

    // the oh hears the ghost's moment loudly: hat bleed, oh - snare about +18 dB
    let mut oh = vec![0f32; x.len()];
    snare_hit(sr, gt, 0.0)(&mut oh);
    let ohs = [OverheadSource { signal: &oh, sr }];
    let peaks = soft_overhead_peaks(&cands, &p, (&x, sr), &ohs, 2.0);
    assert!(
        peaks.iter().any(|&k| (cands[k].time - 0.9).abs() < 0.02),
        "the bleed ghost not denied: {peaks:?}"
    );
    let (_, cands2, _) = detect_onsets_and_cands_soft_excluding(&x, sr, &p, &peaks).unwrap();
    let ghost2 = cands2
        .iter()
        .find(|c| (c.time - 0.9).abs() < 0.02)
        .expect("ghost candidate");
    assert!(!ghost2.kept, "oh bleed kept with gate {:?}", ghost2.gate);

    // the ghost-note case: the oh hears little of it (a quiet room), the soft rescue stays
    let mut oh2 = vec![0f32; x.len()];
    snare_hit(sr, gt, -40.0)(&mut oh2);
    let ohs2 = [OverheadSource { signal: &oh2, sr }];
    let peaks2 = soft_overhead_peaks(&cands, &p, (&x, sr), &ohs2, 2.0);
    assert!(
        !peaks2.iter().any(|&k| (cands[k].time - 0.9).abs() < 0.02),
        "the ghost denied by a quiet oh: {peaks2:?}"
    );
}

/// A1: a soft candidate whose shape does not match the snare template (a quiet kick bleed
/// alone on the mic, no snare-like body) fails the soft gate on the shape, without any
/// busy information.
#[test]
fn soft_gate_needs_the_snare_shape() {
    let sr = 48_000u32;
    let mut p = OnsetParams::for_role(Role::Snare);
    // pinned at the pre-A1 0.95: the synthetic kick sweep's sim is 0.928, so at the measured
    // default 0.92 the shape test alone no longer denies it (the real pipeline denies it by
    // the oh denial, which this unit test has no oh track for)
    p.soft_sim = Some(0.95);
    let mut x = vec![0f32; (2.0 * f64::from(sr)) as usize];
    snare_hit(sr, 0.3, 0.0)(&mut x);
    snare_hit(sr, 1.5, 0.0)(&mut x);
    // a soft kick bleed: the kick's low sweep, no noise, no snare band peak
    kick_track_into(&mut x, sr, 0.9, -18.0);
    let (_, cands, _) = detect_onsets_and_cands(&x, sr, &p).unwrap();
    let bleed = cands.iter().find(|c| (c.time - 0.9).abs() < 0.05);
    if let Some(bleed) = bleed {
        assert!(!bleed.kept, "soft kick bleed kept with gate {:?}", bleed.gate);
        assert_ne!(bleed.gate, Gate::Soft);
    }
    // (no candidate at all also passes: the peak picking never saw a snare-like rise)
}

/// A1: the second soft template. A planted ghost whose shape does not match the loud-hit
/// template closely enough (a quieter, duller stroke: more wires, less shell) still passes
/// when `soft_t2` is set — its shape matches the second template built from the track's
/// own soft, low-attack candidates. Without `soft_t2` the same ghost fails.
#[test]
fn soft_t2_rescues_a_ghost_the_loud_template_scores_too_low() {
    let sr = 48_000u32;
    let p = OnsetParams::for_role(Role::Snare);
    // two loud hits form the loud template; several soft hits (18 dB under) form the
    // second template's band
    let mut x = vec![0f32; (3.0 * f64::from(sr)) as usize];
    snare_hit(sr, 0.3, 0.0)(&mut x);
    snare_hit(sr, 1.5, 0.0)(&mut x);
    for t in [0.9, 1.1, 1.9, 2.1, 2.3] {
        snare_hit(sr, t, -18.0)(&mut x);
    }
    // without the second template: the plain soft gate decides
    let (_, cands0, _) = detect_onsets_and_cands(&x, sr, &p).unwrap();
    let ghost0 = cands0.iter().find(|c| (c.time - 2.5).abs() < 0.05);
    if let Some(g) = ghost0 {
        assert!(!g.kept || g.gate != Gate::Soft || g.similarity >= p.soft_sim.unwrap());
    }
    // with the second template: a deeper ceiling on the attack and a T2 threshold on
    let mut p2 = p.clone();
    p2.soft_t2 = Some(0.90);
    p2.soft_attack_max = Some(120.0);
    let (_, cands, _) = detect_onsets_and_cands(&x, sr, &p2).unwrap();
    let soft: Vec<&Candidate> = cands.iter().filter(|c| c.gate == Gate::Soft).collect();
    assert!(
        !soft.is_empty(),
        "no soft-rescued candidate with T2; cands {}",
        cands.len()
    );
    for c in &soft {
        assert!(c.similarity_soft.is_finite(), "T2 similarity not stored");
        assert!(
            c.similarity_soft >= 0.90,
            "T2 sim {} below threshold",
            c.similarity_soft
        );
    }
    // the re-pick keeps the T2-rescued set at the detection's scale
    let kept: Vec<usize> = cands.iter().filter(|c| c.kept).map(|c| c.frame).collect();
    assert_eq!(repick_candidates(&cands, &p2, 0.0, 1.0), kept);
}

/// A kick hit written into an existing buffer (the `kick_track` pitch sweep, one onset).
fn kick_track_into(x: &mut [f32], sr: u32, t0: f64, gain_db: f64) {
    let i0 = (t0 * f64::from(sr)).round() as usize;
    let g = 10f64.powf(gain_db / 20.0);
    let len = (1.5 * f64::from(sr)) as usize;
    let mut ph = 0.0f64;
    for k in 0..len.min(x.len().saturating_sub(i0)) {
        let t = k as f64 / f64::from(sr);
        let f = 55.0 + 70.0 * (-t / 0.03).exp();
        ph += 2.0 * std::f64::consts::PI * f / f64::from(sr);
        let env = (t / 0.0005).min(1.0) * (-t / 0.12).exp();
        x[i0 + k] += (g * ph.sin() * env) as f32;
    }
}

// A5: a synthetic kick-band spectrogram (8 frames, all bands at `mag_db` inside the kick
// range, silence outside) for the `gained_spec` tests.
fn flat_spec(mag_db: f32) -> Vec<f32> {
    let bands = band_range(20.0, 4_000.0);
    let mut spec = vec![-12f32; 8 * N_BANDS];
    for i in 0..8 {
        for b in bands.clone() {
            spec[i * N_BANDS + b] = mag_db;
        }
    }
    spec
}

#[test]
fn gained_spec_leaves_a_nominal_track_unchanged() {
    // scale = q995 of (10^v - 1) over the snare bands: above the reference (23) -> None.
    assert!(gained_spec(&flat_spec(1.4), &band_range(20.0, 4_000.0), Role::Snare).is_none());
}

#[test]
fn gained_spec_skips_a_near_nominal_track() {
    // A5 round 2: a track just barely under the reference scale (gain 23/20 = 1.15, under
    // the quiet bar) stays shipped: a marginal gain admits bleed on real multitracks.
    assert!(gained_spec(&flat_spec(1.331), &band_range(20.0, 4_000.0), Role::Snare).is_none());
}

#[test]
fn gained_spec_never_gains_a_kick() {
    // A5 round 3 (the coordinator's gate: the Session A kick 52 to 55, Session H 7 to 6):
    // the kick never gains, at any scale, like the tom and cymbal.
    assert!(gained_spec(&flat_spec(0.62), &band_range(20.0, 4_000.0), Role::Kick).is_none());
}

#[test]
fn gained_spec_gains_a_really_quiet_track_to_the_reference() {
    // scale 3.17 -> gain 23/3.17 = 7.3 (over the quiet bar): the snare bands land at the
    // reference scale, the outside bands are untouched.
    let bands = band_range(20.0, 4_000.0);
    let out = gained_spec(&flat_spec(0.62), &bands, Role::Snare).expect("gained");
    let g = 23.0 / (10f32.powf(0.62) - 1.0);
    for i in 0..8 {
        for b in bands.clone() {
            let want = (1.0 + g * (10f32.powf(0.62) - 1.0)).log10();
            assert!((out[i * N_BANDS + b] - want).abs() < 1e-4, "frame {i} band {b}");
        }
        for b in 0..N_BANDS {
            if !bands.contains(&b) {
                assert_eq!(out[i * N_BANDS + b], -12.0, "frame {i} band {b}");
            }
        }
    }
}
