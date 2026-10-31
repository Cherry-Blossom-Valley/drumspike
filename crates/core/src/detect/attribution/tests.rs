use super::*;

const SR: u32 = 48_000;

/// (snare signal, tom signal, snare hit samples, tom hit samples, bleed samples)
type Session = (Vec<f32>, Vec<f32>, Vec<i64>, Vec<i64>, Vec<i64>);

/// A decaying tone burst with a sharp attack at `at` (samples), amplitude `amp`.
fn burst(x: &mut [f32], at: usize, amp: f32, freq: f64, decay_s: f64) {
    for i in 0..(SR as usize / 5) {
        let k = at + i;
        if k >= x.len() {
            break;
        }
        let t = i as f64 / f64::from(SR);
        x[k] += amp * ((2.0 * std::f64::consts::PI * freq * t).sin() * (-t / decay_s).exp()) as f32;
    }
}

#[test]
fn calibrate_gain_takes_the_quiet_values_when_there_are_enough() {
    let all = [-30.0, -20.0, -10.0, 0.0, 10.0, 20.0];
    let quiet = [-30.0, -20.0, -10.0, 0.0, 10.0];
    let (g, n, q) = calibrate_gain(&all, &quiet, 10.0, 5).unwrap();
    assert_eq!((n, q), (5, true));
    assert!((g - percentile(&quiet, 10.0).unwrap()).abs() < 1e-12);
    let (g, n, q) = calibrate_gain(&all, &quiet[..4], 10.0, 5).unwrap();
    assert_eq!((n, q), (6, false), "too few quiet values: all of them");
    assert!((g - percentile(&all, 10.0).unwrap()).abs() < 1e-12);
    assert_eq!(calibrate_gain(&all[..4], &[], 10.0, 5), None);
    assert_eq!(calibrate_gain(&[], &[], 10.0, 0), None, "at least one value");
    assert_eq!(calibrate_gain(&[1.0], &[], 50.0, 0), Some((1.0, 1, false)));
}

/// A deterministic white-noise burst (LCG) with an exponential decay.
fn noise_burst(x: &mut [f32], at: usize, amp: f32, decay_s: f64, seed: u64) {
    let mut s = seed;
    for i in 0..(SR as usize / 5) {
        let k = at + i;
        if k >= x.len() {
            break;
        }
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let u = ((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0;
        let t = i as f64 / f64::from(SR);
        x[k] += amp * (u * (-t / decay_s).exp()) as f32;
    }
}

/// A session: snare (noise bursts) and tom (short low tone bursts). Every snare hit
/// bleeds into the tom mic `offset` samples later at -20 dB; the tom also has its own
/// hits, which bleed into the snare mic 1.5 ms later at -20 dB.
/// Returns (snare signal, tom signal, snare hit samples, tom hit samples, bleed samples).
fn session_with(offset: i64) -> Session {
    let n = SR as usize * 12;
    let mut snare = vec![0.0f32; n];
    let mut tom = vec![0.0f32; n];
    let delay = ms_to_samples(1.5, SR) as usize;
    let mut s_hits = Vec::new();
    let mut t_hits = Vec::new();
    let mut bleed_at = Vec::new();
    for k in 0..20 {
        let at = SR as usize / 4 + k * SR as usize / 2;
        noise_burst(&mut snare, at, 0.5, 0.05, 7 + k as u64);
        // bleed: the same burst, shifted and 20 dB down
        let b = (at as i64 + offset) as usize;
        noise_burst(&mut tom, b, 0.05, 0.05, 7 + k as u64);
        s_hits.push(at as i64);
        bleed_at.push(b as i64);
    }
    for k in 0..10 {
        let at = SR as usize / 2 + k * SR as usize + SR as usize / 8;
        burst(&mut tom, at, 0.6, 110.0, 0.02);
        burst(&mut snare, at + delay, 0.06, 110.0, 0.02);
        t_hits.push(at as i64);
    }
    (snare, tom, s_hits, t_hits, bleed_at)
}

fn session() -> Session {
    session_with(ms_to_samples(1.5, SR))
}

fn inputs<'a>(snare: &'a [f32], tom: &'a [f32], s_hits: &[i64], t_all: &[i64]) -> Vec<TrackInput<'a>> {
    let hits = |v: &[i64]| {
        v.iter()
            .map(|&s| HitInput {
                sample: s,
                rise: Some(20.0),
            })
            .collect()
    };
    vec![
        TrackInput {
            id: "s".into(),
            role: Role::Snare,
            instrument: "snare".into(),
            signal: snare,
            hits: hits(s_hits),
        },
        TrackInput {
            id: "t".into(),
            role: Role::Tom,
            instrument: "t1".into(),
            signal: tom,
            hits: hits(t_all),
        },
    ]
}

#[test]
fn role_names_round_trip_and_bands_are_fixed() {
    for r in [Role::Kick, Role::Snare, Role::Tom, Role::Hat, Role::Cymbal, Role::Other] {
        assert_eq!(Role::from_name(r.name()), Some(r));
    }
    assert_eq!(Role::from_name("drums"), None);
    assert_eq!(Role::Tom.band(), &[hp(50.0), lp(400.0)]);
    assert_eq!(Role::Hat.band(), Role::Cymbal.band());
    assert!(Role::Other.band().is_empty());
}

#[test]
fn biquad_lowpass_passes_dc_and_highpass_blocks_it() {
    let l = Biquad::new(FilterKind::LowPass, 400.0, SR);
    let h = Biquad::new(FilterKind::HighPass, 400.0, SR);
    let dc = |q: &Biquad| (q.b[0] + q.b[1] + q.b[2]) / (1.0 + q.a[0] + q.a[1]);
    assert!((dc(&l) - 1.0).abs() < 1e-12);
    assert!(dc(&h).abs() < 1e-12);
    assert_eq!(band_sections(Role::Tom.band(), SR).len(), 4);
    assert!(band_sections(&[], SR).is_empty());
}

#[test]
fn band_peak_db_measures_in_band_and_skips_the_preroll() {
    let mut x = vec![0.0f32; SR as usize];
    burst(&mut x, 1000, 0.5, 100.0, 1.0);
    let low = band_sections(Role::Kick.band(), SR);
    let high = band_sections(Role::Hat.band(), SR);
    let w0 = 1000 + 2400;
    let l = band_peak_db(&x, &low, 1000, w0, w0 + 480);
    let h = band_peak_db(&x, &high, 1000, w0, w0 + 480);
    assert!((l - 20.0 * 0.5f64.log10()).abs() < 3.0, "{l}");
    assert!(h < l - 60.0, "{h} {l}");
    // broadband: exact peak
    let b = band_peak_db(&x, &[], w0, w0, w0 + 480);
    let m = f64::from(x[w0..w0 + 480].iter().fold(0.0f32, |m, v| m.max(v.abs())));
    assert!((b - 20.0 * (m + 1e-9).log10()).abs() < 1e-12);
    assert_eq!(band_peak_db(&x, &low, 10, 20, 20), -180.0);
}

#[test]
fn percentile_follows_numpy_linear() {
    let v = [3.0, 1.0, 2.0, 10.0];
    assert_eq!(percentile(&v, 0.0), Some(1.0));
    assert_eq!(percentile(&v, 100.0), Some(10.0));
    assert_eq!(percentile(&v, 50.0), Some(2.5));
    // numpy.percentile([1, 2, 3, 10], 90) = 7.9
    assert!((percentile(&v, 90.0).unwrap() - 7.9).abs() < 1e-12);
    assert_eq!(percentile(&[], 50.0), None);
}

#[test]
fn ms_to_samples_rounds_half_up() {
    assert_eq!(ms_to_samples(5.0, 44_100), 221);
    assert_eq!(ms_to_samples(0.5, SR), 24);
    assert_eq!(ms_to_samples(-0.5, SR), -24);
    assert_eq!(ms_to_samples(0.0, SR), 0);
}

#[test]
fn hit_flags_report_names_and_confidence() {
    let mut f = HitFlags::default();
    assert!(!f.is_flagged());
    assert_eq!(f.confidence(), 0.0);
    assert!(f.names().is_empty());
    f.resonance = Some(ResonanceFlag {
        rise_db: 1.0,
        median_db: 15.0,
        confidence: 0.6,
    });
    f.bleed = Some(BleedFlag {
        source: 0,
        lag_samples: -72,
        excess_db: -3.0,
        dominance_db: 20.0,
        confidence: 0.875,
    });
    assert!(f.is_flagged());
    assert_eq!(f.confidence(), 0.875);
    assert_eq!(f.names(), vec!["bleed", "resonance"]);
}

#[test]
fn defaults_are_off_and_enabled_turns_both_on() {
    let d = AttributionParams::default();
    assert!(!d.bleed_enabled && !d.resonance_enabled);
    let e = AttributionParams::enabled();
    assert!(e.bleed_enabled && e.resonance_enabled);
    assert_eq!(e.window_ms, d.window_ms);
    let (snare, tom, s_hits, t_hits, _b) = session();
    let tr = inputs(&snare, &tom, &s_hits, &t_hits);
    let out = attribute(&tr, SR, &d).unwrap();
    assert!(out.flags.iter().flatten().all(|f| !f.is_flagged()));
    assert!(out.transfers.is_empty());
    assert_eq!(out.flags[0].len(), s_hits.len());
    assert_eq!(attribute(&tr, 0, &e), Err(AttributionError::ZeroRate));
    assert_eq!(AttributionError::ZeroRate.to_string(), "sample rate is 0");
}

#[test]
fn snare_bleed_on_the_tom_mic_is_flagged_and_real_hits_are_not() {
    let (snare, tom, s_hits, t_hits, bleed_at) = session();
    let mut t_all: Vec<i64> = t_hits.iter().chain(&bleed_at).copied().collect();
    t_all.sort_unstable();
    let tr = inputs(&snare, &tom, &s_hits, &t_all);
    let out = attribute(&tr, SR, &AttributionParams::enabled()).unwrap();
    for (h, f) in tr[1].hits.iter().zip(&out.flags[1]) {
        let is_bleed = bleed_at.contains(&h.sample);
        assert_eq!(f.bleed.is_some(), is_bleed, "tom hit at {}", h.sample);
        if let Some(b) = f.bleed {
            assert_eq!(b.source, 0);
            assert!(b.lag_samples < 0 && b.lag_samples >= -ms_to_samples(5.0, SR));
            assert!(b.confidence >= 0.5 && b.confidence <= 1.0);
        }
    }
    // the snare hits are the source, never bleed of the tom
    assert!(out.flags[0].iter().all(|f| f.bleed.is_none()));
    // both directions calibrated; the tom-to-snare gain is -20 dB, measured at tom hits
    // with a quiet snare mic
    assert_eq!(out.transfers.len(), 2);
    let g = out.transfers.iter().find(|t| t.source == 1 && t.target == 0).unwrap();
    assert!(g.quiet_only);
    assert!((g.gain_db + 20.0).abs() < 3.0, "{}", g.gain_db);
}

#[test]
fn same_instrument_never_flags_and_unlisted_roles_are_skipped() {
    let (snare, tom, s_hits, t_hits, bleed_at) = session();
    let mut t_all: Vec<i64> = t_hits.iter().chain(&bleed_at).copied().collect();
    t_all.sort_unstable();
    let mut tr = inputs(&snare, &tom, &s_hits, &t_all);
    tr[1].instrument = "snare".into();
    let out = attribute(&tr, SR, &AttributionParams::enabled()).unwrap();
    assert!(out.flags[1].iter().all(|f| f.bleed.is_none()));
    assert!(out.transfers.is_empty());
    tr[1].instrument = "t1".into();
    let p = AttributionParams {
        bleed_roles: vec![Role::Kick],
        ..AttributionParams::enabled()
    };
    let out = attribute(&tr, SR, &p).unwrap();
    assert!(out.flags.iter().flatten().all(|f| f.bleed.is_none()));
}

#[test]
fn a_silent_target_gets_no_transfer_and_no_flag() {
    // a gated tom mic: digital silence except its own hits; no transfer into it
    let (snare, _tom, s_hits, _t_hits, _b) = session();
    let mut gated = vec![0.0f32; snare.len()];
    let mut g_hits = Vec::new();
    for k in 0..8 {
        let at = SR as usize / 2 + k * SR as usize + SR as usize / 8;
        burst(&mut gated, at, 0.6, 110.0, 0.02);
        // the gate closes 20 ms after the hit
        gated[at + 960..at + SR as usize / 5].fill(0.0);
        g_hits.push(at as i64);
    }
    let tr = inputs(&snare, &gated, &s_hits, &g_hits);
    let out = attribute(&tr, SR, &AttributionParams::enabled()).unwrap();
    assert!(out.transfers.iter().all(|t| t.target != 1), "{:?}", out.transfers);
    assert!(out.flags[1].iter().all(|f| f.bleed.is_none()));
}

#[test]
fn a_source_after_the_hit_does_not_count() {
    // the tom mic hears each snare burst 1.5 ms BEFORE the snare mic: the snare attack
    // comes after the tom hit, so the snare is no source (arrival order)
    let delay = ms_to_samples(1.5, SR);
    let (snare, tom, s_hits, t_hits, early) = session_with(-delay);
    let mut t_all: Vec<i64> = t_hits.iter().chain(&early).copied().collect();
    t_all.sort_unstable();
    let tr = inputs(&snare, &tom, &s_hits, &t_all);
    let out = attribute(&tr, SR, &AttributionParams::enabled()).unwrap();
    assert!(out.flags[1].iter().all(|f| f.bleed.is_none()));
    // with a lag tolerance beyond the delay the early hits are flagged, with a positive lag
    let p = AttributionParams {
        lag_tol_ms: 2.0,
        ..AttributionParams::enabled()
    };
    let out = attribute(&tr, SR, &p).unwrap();
    let mut flagged = 0;
    for (h, f) in tr[1].hits.iter().zip(&out.flags[1]) {
        if let Some(b) = f.bleed {
            assert!(early.contains(&h.sample), "real tom hit at {} flagged", h.sample);
            assert!(b.lag_samples > 0, "{}", b.lag_samples);
            flagged += 1;
        }
    }
    assert!(flagged >= 15, "{flagged} of {}", early.len());
}

#[test]
fn resonance_flags_a_low_rise_on_tom_tracks_only() {
    let sig = vec![0.0f32; 1000];
    let mk = |role: Role, rises: &[f64]| TrackInput {
        id: "x".into(),
        role,
        instrument: role.name().into(),
        signal: &sig,
        hits: rises
            .iter()
            .enumerate()
            .map(|(i, &r)| HitInput {
                sample: i as i64 * 10,
                rise: Some(r),
            })
            .collect(),
    };
    let rises = [20.0, 21.0, 19.0, 22.0, 18.0, 3.0, 9.5];
    let tr = vec![mk(Role::Tom, &rises), mk(Role::Kick, &rises)];
    let p = AttributionParams {
        bleed_enabled: false,
        ..AttributionParams::enabled()
    };
    let out = attribute(&tr, SR, &p).unwrap();
    let flagged: Vec<bool> = out.flags[0].iter().map(|f| f.resonance.is_some()).collect();
    // median 19; threshold 9: 3.0 is flagged, 9.5 is not
    assert_eq!(flagged, vec![false, false, false, false, false, true, false]);
    let r = out.flags[0][5].resonance.unwrap();
    assert_eq!(r.median_db, 19.0);
    assert!((r.confidence - (0.5 + 0.5 * 6.0 / 12.0)).abs() < 1e-12);
    assert!(out.flags[1].iter().all(|f| f.resonance.is_none()));
    // too few hits with a rise: no median, no flag
    let few = vec![mk(Role::Tom, &[20.0, 1.0])];
    let out = attribute(&few, SR, &p).unwrap();
    assert!(out.flags[0].iter().all(|f| f.resonance.is_none()));
}

#[test]
fn has_near_and_broadband_peak_behave_at_the_edges() {
    let v = [10, 20, 30];
    assert!(has_near(&v, 22, 2));
    assert!(!has_near(&v, 25, 4));
    assert!(!has_near(&[], 0, 100));
    let x = [0.5f32, -1.0, 0.25];
    assert!((broadband_peak_db(&x, 0, (0, 3))).abs() < 1e-6);
    assert_eq!(broadband_peak_db(&x, 10, (0, 3)), -180.0);
}
