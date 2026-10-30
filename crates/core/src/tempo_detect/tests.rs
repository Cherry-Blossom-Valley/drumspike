use super::*;

fn kick(t: f64) -> Hit {
    Hit::new(t, Role::Kick)
}

#[test]
fn role_names_map_to_roles() {
    assert_eq!(Role::from_name("kick"), Role::Kick);
    assert_eq!(Role::from_name("snare"), Role::Snare);
    assert_eq!(Role::from_name("tom"), Role::Tom);
    assert_eq!(Role::from_name("cymbal"), Role::Cymbal);
    assert_eq!(Role::from_name("hat"), Role::Other);
    assert_eq!(Role::from_name(""), Role::Other);
    assert!(Role::Kick.is_ks() && Role::Snare.is_ks() && !Role::Tom.is_ks() && !Role::Cymbal.is_ks());
}

#[test]
fn a_new_hit_has_the_default_velocity_and_the_velocity_weighs_it() {
    let h = Hit::new(1.0, Role::Snare);
    assert_eq!(h.v, DEFAULT_VELOCITY);
    let ghost = Hit { v: 35.0, ..h };
    let accent = Hit { v: 110.0, ..h };
    assert!((ghost.weight() - 0.076).abs() < 0.001, "{}", ghost.weight());
    assert!((accent.weight() - 0.750).abs() < 0.001, "{}", accent.weight());
    assert_eq!(Hit { v: f64::NAN, ..h }.weight(), h.weight());
    assert_eq!(Hit { v: 500.0, ..h }.weight(), 1.0);
}

#[test]
fn params_are_checked() {
    DetectParams::default().validate().unwrap();
    for (bars, pct, range, dur) in [
        (0, 2.0, DEFAULT_BPM_RANGE, None),
        (65, 2.0, DEFAULT_BPM_RANGE, None),
        (4, 0.0, DEFAULT_BPM_RANGE, None),
        (4, 51.0, DEFAULT_BPM_RANGE, None),
        (4, 2.0, (10.0, 200.0), None),
        (4, 2.0, (100.0, 150.0), None),
        (4, 2.0, (100.0, f64::INFINITY), None),
        (4, 2.0, DEFAULT_BPM_RANGE, Some(0.0)),
        (4, 2.0, DEFAULT_BPM_RANGE, Some(f64::NAN)),
    ] {
        let p = DetectParams {
            min_bars: bars,
            min_step_pct: pct,
            bpm_range: range,
            duration_s: dur,
        };
        assert!(p.validate().is_err(), "{p:?}");
    }
}

#[test]
fn prepare_merges_two_mics_of_one_drum_and_drops_bad_late_and_isolated_times() {
    let snare = Hit::new(1.001, Role::Snare);
    let got = prepare(
        &[
            kick(1.0),
            Hit { v: 120.0, ..kick(1.01) },
            kick(f64::NAN),
            kick(-0.5),
            snare,
            kick(0.5),
            kick(1.03),
            kick(30.0),
            kick(5000.0),
        ],
        Some(100.0),
    );
    let loud = Hit { v: 120.0, ..kick(1.0) };
    assert_eq!(got.hits, vec![kick(0.5), loud, snare, kick(1.03)]);
    assert_eq!((got.late, got.isolated), (1, 1), "5000 s is late, 30 s is isolated");
    assert_eq!(prepare(&[kick(1.0), kick(5000.0)], None).isolated, 2);
}

#[test]
fn two_mics_of_one_drum_give_the_detection_of_one() {
    let mut one = Vec::new();
    for i in 0..128 {
        let t = 0.5 + i as f64 * 0.5;
        one.push(if i % 2 == 0 { kick(t) } else { Hit::new(t, Role::Snare) });
    }
    let mut two = one.clone();
    two.extend(one.iter().map(|h| Hit { t: h.t + 0.001, ..*h }));
    let (a, b) = (
        detect(&one, &DetectParams::default()).unwrap(),
        detect(&two, &DetectParams::default()).unwrap(),
    );
    assert_eq!(b.hits, a.hits);
    assert_eq!(a.map, b.map);
}

#[test]
fn segment_keeps_the_minimum_length_and_finds_a_step() {
    let mut u = vec![0.0; 10];
    u.extend(vec![0.1; 10]);
    // the penalty of `pieces`: half the minimum length times the smallest step (0.1 here)
    assert_eq!(segment(&u, 4, 0.2), vec![(0, 10), (10, 20)]);
    // a run of 3 is shorter than 4: one piece; with a minimum of 3 it is a piece of its own
    let mut v = vec![0.0; 10];
    v.extend(vec![0.1; 3]);
    v.extend(vec![0.0; 10]);
    assert_eq!(segment(&v, 4, 0.2), vec![(0, 23)]);
    assert_eq!(segment(&v, 3, 0.1), vec![(0, 10), (10, 13), (13, 23)]);
}

#[test]
fn the_grid_fit_recovers_two_periods_and_is_continuous_at_the_break() {
    let mut on = Vec::new();
    for n in 0..40i64 {
        let t = if n < 20 {
            1.0 + n as f64 * 0.5
        } else {
            11.0 + (n - 20) as f64 * 0.4
        };
        on.push(OnBeat { n, t, role: Role::Kick });
    }
    let (g, kept) = fit(&on, &[20]).unwrap();
    assert_eq!(kept.len(), 40);
    assert!((g.t0 - 1.0).abs() < 1e-9 && (g.periods[0] - 0.5).abs() < 1e-9 && (g.periods[1] - 0.4).abs() < 1e-9);
    assert!((g.time(20) - 11.0).abs() < 1e-9 && (g.time(-2) - 0.0).abs() < 1e-9);
    assert!(sse_of(&on, &[20]) < 1e-12 && sse_of(&on, &[24]) > 1e-3);
    let mut b = vec![23];
    place_breaks(&on, &mut b, 0, 4);
    assert_eq!(b, vec![20], "the bar line within 2 bars that fits best");
    assert!(fit(&on[..1], &[]).is_none(), "one beat cannot give a period");
}

#[test]
fn the_fit_leaves_out_an_outlier_and_a_small_group_of_beats_far_out() {
    // 64 beats at 0.5 s with a small alternating jitter, one hit 30 ms late, a pair far out
    let mut on: Vec<OnBeat> = (0..64i64)
        .map(|n| OnBeat {
            n,
            t: 1.0 + n as f64 * 0.5 + if n % 2 == 0 { 0.001 } else { -0.001 },
            role: Role::Kick,
        })
        .collect();
    on[40].t += 0.03;
    for n in [100, 101] {
        on.push(OnBeat {
            n,
            t: 1.0 + n as f64 * 0.5 + 0.05,
            role: Role::Snare,
        });
    }
    let kept = drop_isolated(&on);
    assert_eq!(
        kept.len(),
        64,
        "beats 100 and 101: a group of 2, 37 beats from the song"
    );
    let (g, used) = fit(&kept, &[]).unwrap();
    assert_eq!(used.len(), 63, "the late hit is out");
    assert!((g.periods[0] - 0.5).abs() < 1e-5 && (g.t0 - 1.0).abs() < 1e-3, "{g:?}");
    // only small groups: nothing to compare with, all stay
    assert_eq!(drop_isolated(&on[64..]).len(), 2);
}

#[test]
fn the_error_bar_reads_two_standard_errors_or_the_end_jitter() {
    let on: Vec<OnBeat> = (0..200i64)
        .map(|n| OnBeat {
            n,
            t: n as f64 * 0.5 + if n % 2 == 0 { 0.002 } else { -0.002 },
            role: Role::Kick,
        })
        .collect();
    let (g, kept) = fit(&on, &[]).unwrap();
    let e = error_bar(&g, 0, &kept, (i64::MIN, i64::MAX), 200.0);
    assert!((e - 120.0 * 0.002 / 200.0 / 0.25).abs() < 1e-4, "{e}");
}

#[test]
fn the_suggestion_is_always_a_whole_number_of_bpm_and_the_grid_follows() {
    // A fit at 125.9991 BPM with a tight error bar: 126, and the snapped grid stays on the hits.
    let p = 60.0 / 125.9991;
    let on: Vec<OnBeat> = (0..500i64)
        .map(|n| OnBeat {
            n,
            t: 1.0 + n as f64 * p,
            role: Role::Kick,
        })
        .collect();
    let g0 = Grid {
        t0: 1.0,
        breaks: vec![],
        periods: vec![p],
        se: vec![1e-6],
    };
    let (g, errs) = snap(&g0, &on, 499);
    assert_eq!(60.0 / g.periods[0], 126.0);
    assert_eq!(errs.len(), 1);
    assert!(
        (g.time(499) - (1.0 + 499.0 * p)).abs() <= p / 4.0,
        "the snapped grid left the hits: {} vs {}",
        g.time(499),
        1.0 + 499.0 * p
    );
    // A fit at 125.37 BPM whose error bar holds no integer: still 125, never a fraction.
    let p = 60.0 / 125.37;
    let on: Vec<OnBeat> = (0..500i64)
        .map(|n| OnBeat {
            n,
            t: 1.0 + n as f64 * p,
            role: Role::Kick,
        })
        .collect();
    let g0 = Grid {
        t0: 1.0,
        breaks: vec![],
        periods: vec![p],
        se: vec![1e-6],
    };
    let (g, _) = snap(&g0, &on, 499);
    assert_eq!(60.0 / g.periods[0], 125.0);
}

/// #472 gate (owner 2026-10-27): every suggestion on every fixture is a whole number of BPM:
/// the start tempo, every step and every alternative.
#[test]
fn every_suggestion_on_every_fixture_is_a_whole_number_of_bpm() {
    let checks: Vec<(&str, Detection)> = vec![
        ("session_e kick and snare", rr_detect(false, 0.0)),
        ("session_e all tracks", rr_detect(true, 0.0)),
        (
            "the other side kick only",
            detect(&session_d(), &DetectParams::default()).unwrap(),
        ),
    ];
    for (name, d) in checks {
        assert_eq!(
            d.map.bpm,
            d.map.bpm.round(),
            "{name}: the start tempo {} is no whole number",
            d.map.bpm
        );
        for s in &d.map.steps {
            assert_eq!(
                s.bpm,
                s.bpm.round(),
                "{name}: the step at {} s suggests {} BPM",
                s.t,
                s.bpm
            );
        }
        for a in &d.alternatives {
            assert_eq!(
                a.bpm,
                a.bpm.round(),
                "{name}: the alternative {} BPM is no whole number",
                a.bpm
            );
        }
    }
}

#[test]
fn the_sign_test_and_the_correlation_read_counts_and_patterns() {
    assert_eq!(sign_test(5.0, 5.0), 0.0);
    assert!((sign_test(3.0, 1.0) - 0.420).abs() < 0.001 && (sign_test(9.0, 1.0) - 0.836).abs() < 0.001);
    assert!(sign_test(105.0, 100.0) < 0.4);
    let rock: Vec<f64> = (0..64).map(|i| [1.0, -1.0, 1.0, -1.0][i % 4]).collect();
    let off: Vec<f64> = (0..64).map(|i| [0.0, 0.0, 1.0, 0.0][i % 4]).collect();
    let fs = [rock.clone(), off];
    assert!((corr(&fs, 4) - 1.0).abs() < 1e-9 && corr(&fs, 2) < 0.9 && corr(&fs, 3) < -0.8);
    assert!(
        (corr(std::slice::from_ref(&rock), 2) - 1.0).abs() < 1e-9,
        "a two-beat pattern alone"
    );
}

/// P4-39e: the hits of the owner's project "Session E" (6 tracks, 280 s, no velocities).
fn session_e() -> (Vec<Hit>, f64) {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("../../../../tests/fixtures/tempo/session_e_hits.json")).unwrap();
    let mut hits = Vec::new();
    let mut dur: f64 = 0.0;
    for tr in v["tracks"].as_object().unwrap().values() {
        let role = Role::from_name(tr["role"].as_str().unwrap());
        dur = dur.max(tr["duration"].as_f64().unwrap());
        hits.extend(
            tr["hits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| Hit::new(t.as_f64().unwrap(), role)),
        );
    }
    (hits, dur)
}

/// The detection of the Session E hits (kick and snare, as `tempo_map_detect` reads them
/// by default, or all six tracks), each shifted by `shift` seconds.
fn rr_detect(all: bool, shift: f64) -> Detection {
    let (hits, dur) = session_e();
    let hits: Vec<Hit> = hits
        .into_iter()
        .filter(|h| all || h.role.is_ks())
        .map(|h| Hit { t: h.t + shift, ..h })
        .collect();
    detect(
        &hits,
        &DetectParams {
            duration_s: Some(dur + shift),
            ..DetectParams::default()
        },
    )
    .unwrap()
}

/// Distance of `t` from the nearest bar line of the grid with bar 1 at `bar1` and `bpm` in 4/4.
fn off_bar_line(t: f64, bar1: f64, bpm: f64) -> f64 {
    let bar = 4.0 * 60.0 / bpm;
    let x = (t - bar1).rem_euclid(bar);
    x.min(bar - x)
}

#[test]
fn session_e_gives_118_bpm_with_bar_1_on_the_backbeat_grid() {
    // P4-39e: the hits sit on a 118.0 BPM grid with the snare on beats 2 and 4 and bar 1 at
    // 2.025 s (one bar earlier is -0.009 s: the detection's bar 1 is the first downbeat at or
    // after 0 s, within a quarter beat). A sparse section (77 to 87 s, one kick pair per bar)
    // made the local comb read 88.5 BPM (4/3 beats); the tracker took it for 6 beats, the fit
    // dropped that short piece but kept the beat numbers, and the grid bent to 117.48 BPM.
    for all in [false, true] {
        let d = rr_detect(all, 0.0);
        let what = if all { "all tracks" } else { "kick and snare" };
        assert!((d.map.bpm - 118.0).abs() <= 0.05, "{what}: {} BPM", d.map.bpm);
        assert!(
            d.map.steps.is_empty() && d.excursions.is_empty(),
            "{what}: {:?} {:?}",
            d.steps,
            d.excursions
        );
        assert_eq!(d.map.meter, (4, 4), "{what}");
        assert!(
            off_bar_line(d.map.bar1_s, 2.025, 118.0) <= 0.03 && d.map.bar1_s <= 2.025 + 0.03,
            "{what}: bar 1 at {} s",
            d.map.bar1_s
        );
        assert!(d.on_beat >= 0.6, "{what}: on the beat {}", d.on_beat);
    }
}

#[test]
fn session_e_shifted_by_each_sixteenth_keeps_bar_1_on_the_backbeat() {
    // The four sixteenth phases: the same hits moved by 0 to 3 sixteenths (and one bar, so bar 1
    // stays after 0 s) give the same tempo and bar 1 moved by the same time.
    let s16 = 60.0 / 118.0 / 4.0;
    for k in 0..4 {
        let shift = 4.0 * 4.0 * s16 + k as f64 * s16;
        let d = rr_detect(false, shift);
        assert!((d.map.bpm - 118.0).abs() <= 0.05, "shift {k}/16: {} BPM", d.map.bpm);
        assert!(
            off_bar_line(d.map.bar1_s, 2.025 + shift, 118.0) <= 0.03,
            "shift {k}/16: bar 1 at {} s, expected {} s or a bar before or after",
            d.map.bar1_s,
            2.025 + shift
        );
    }
}

// ---------------------------------------------------------------- the #317 step guard

#[test]
fn the_simple_ratio_test_knows_the_guarded_ratios_and_their_tolerance() {
    for r in [4.0 / 3.0, 1.5, 2.0, 3.0 / 4.0, 2.0 / 3.0, 0.5] {
        assert!(is_simple_ratio(r, 1.0), "{r}");
        assert!(is_simple_ratio(1.0, r), "1/{r}");
    }
    // 1 % off a simple ratio is still inside the 2 % tolerance, 3 % is not.
    assert!(is_simple_ratio(4.0 / 3.0 * 1.01, 1.0));
    assert!(!is_simple_ratio(4.0 / 3.0 * 1.03, 1.0));
    // The step ratios of the synthetic trains and of real songs are no simple ratios.
    for r in [100.0 / 120.0, 126.0 / 100.0, 140.0 / 120.0, 123.0 / 120.0, 1.0] {
        assert!(!is_simple_ratio(r, 1.0), "{r}");
    }
}

#[test]
fn grid_share_reads_on_grid_hits_and_rejects_off_grid_ones() {
    let on: Vec<Hit> = (0..16).map(|k| Hit::new(10.0 + k as f64 * 0.5, Role::Kick)).collect();
    assert!((grid_share(&on, 10.0, 20.0, 0.5) - 1.0).abs() < 1e-9);
    // The same hits against a 3/4 grid: 0.5/0.375 is 4/3 beats; every third hit still lands
    // on it by arithmetic, but the share stays far under the own grid's 1.0.
    assert!(grid_share(&on, 10.0, 20.0, 0.375) < 0.4);
    // An empty window is 0.
    assert_eq!(grid_share(&on, 30.0, 40.0, 0.5), 0.0);
}

/// #465: the kick-only hits of the owner's project "Session D" (Pro Tools 92 BPM, bar 1
/// at 0 s, 377 kick hits, audio 48 kHz).
fn session_d() -> Vec<Hit> {
    let v: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/tempo/session_d_kick_hits.json"
    ))
    .unwrap();
    v["tracks"]["kick"]["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| Hit::new(t.as_f64().unwrap(), Role::Kick))
        .collect()
}

#[test]
fn session_d_kick_only_session_suggests_92_and_bar_1_on_the_beat() {
    let d = detect(&session_d(), &DetectParams::default()).unwrap();
    println!(
        "Session D: suggested {} BPM (fitted {:.4}, error bar {:.5}), confidence {}, bar 1 at {:.4} s (backbeat {}, downbeat {}, alternative {:.4}), alternatives {:?}, meter {:?} feel {:?}, on beat {}, jitter {} ms",
        d.map.bpm,
        d.bpm,
        d.bpm_error_bar,
        d.confidence,
        d.bar1_s,
        d.bar1_backbeat,
        d.bar1_downbeat,
        d.bar1_alternative_s,
        d.alternatives,
        d.meter,
        d.meter_feel,
        d.on_beat,
        d.jitter_ms
    );
    // #472: the kick-only session is played at 92 BPM; the half tempo (46) must not win.
    assert!(
        (d.map.bpm - 92.0).abs() <= 0.1,
        "suggested {} BPM, alternatives {:?} (want 92)",
        d.map.bpm,
        d.alternatives
    );
    // The hits sit on a 92.000 BPM grid from 0 s: bar 1 must lie on a beat of that grid.
    let beat = 60.0 / 92.0;
    let phase = d.bar1_s.rem_euclid(beat);
    let off = phase.min(beat - phase);
    assert!(
        off <= 0.005,
        "bar 1 at {} s is {:.1} ms off a beat of the 92 BPM grid from 0 s",
        d.bar1_s,
        off * 1000.0
    );
    // The half tempo stays visible as an alternative.
    assert!(
        d.alternatives.iter().any(|a| (a.bpm - 46.0).abs() < 0.01),
        "46 BPM is no longer an alternative: {:?}",
        d.alternatives
    );
}

/// #468: a typed start tempo re-fits bar 1 to the hits. The owner typed 92 over the 46
/// suggestion with bar 1 at 2.555 s (53 ms off a beat); the refit moves bar 1 onto a beat of
/// the 92 BPM grid from 0 s, which the fixture's hits sit on.
#[test]
fn the_refit_of_a_typed_start_tempo_puts_bar_1_on_a_beat_the_hits_support() {
    let r = refit_bar1(&session_d(), 92.0, 2.555).unwrap();
    let beat = 60.0 / 92.0;
    let phase = r.bar1_s.rem_euclid(beat);
    let off = phase.min(beat - phase);
    assert!(
        off <= 0.005,
        "bar 1 at {} s is {:.1} ms off a beat of the 92 BPM grid from 0 s",
        r.bar1_s,
        off * 1000.0
    );
    // The bar keeps its number: the refit moves bar 1 by at most half a beat.
    assert!(
        (r.bar1_s - 2.555).abs() <= 0.5 * beat,
        "bar 1 moved from 2.555 s to {} s, more than half a beat",
        r.bar1_s
    );
    assert!(r.support > 0.0, "the phase has no hit support");
}

/// The refit refuses a tempo that is not a positive number and too few kick and snare hits.
#[test]
fn the_refit_refuses_a_bad_tempo_and_too_few_hits() {
    assert!(refit_bar1(&session_d(), 0.0, 2.555).is_err());
    assert!(refit_bar1(&session_d(), -92.0, 2.555).is_err());
    let few: Vec<Hit> = (0..4).map(|k| Hit::new(k as f64 * 0.5, Role::Kick)).collect();
    let e = refit_bar1(&few, 92.0, 0.0).unwrap_err();
    assert!(e.contains("at least 8"), "{e}");
}

/// The #317 situation: a sparse or noisy passage whose hits fit the old tempo's grid about as
/// well as the 4/3 grid. Its hits sit on the old grid here; the piece must go.
#[test]
fn the_guard_drops_a_simple_ratio_piece_that_the_old_grid_explains_as_well() {
    let mut hits: Vec<Hit> = (0..20)
        .map(|k| Hit::new(10.0 + k as f64 * 0.5 + 0.01, Role::Kick))
        .collect();
    hits.extend((0..8).map(|k| Hit::new(10.5 + k as f64, Role::Snare)));
    let pcs = vec![
        Piece {
            start: 0.0,
            period: 0.5,
        },
        Piece {
            start: 10.0,
            period: 0.375,
        },
    ];
    let kept = guard_simple_ratio_pieces(&hits, &pcs);
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!((kept[0].period - 0.5).abs() < 1e-9);
}

/// A real step at a simple ratio: its hits are on its own grid and off the old one.
#[test]
fn the_guard_keeps_a_simple_ratio_piece_that_its_own_grid_explains() {
    let mut hits: Vec<Hit> = (0..20).map(|k| Hit::new(0.0 + k as f64 * 0.5, Role::Kick)).collect();
    hits.extend((0..24).map(|k| Hit::new(10.0 + k as f64 * 0.375, Role::Snare)));
    let pcs = vec![
        Piece {
            start: 0.0,
            period: 0.5,
        },
        Piece {
            start: 10.0,
            period: 0.375,
        },
    ];
    let kept = guard_simple_ratio_pieces(&hits, &pcs);
    assert_eq!(kept.len(), 2, "{kept:?}");
}

/// A piece at a tempo that is no simple ratio of the one before it is never scrutinized, and
/// a piece with fewer kick and snare hits than the comb needs is left to the fit's rules.
#[test]
fn the_guard_leaves_other_ratios_and_very_thin_pieces_alone() {
    let hits: Vec<Hit> = (0..20).map(|k| Hit::new(10.0 + k as f64 * 0.5, Role::Kick)).collect();
    // 126 over 100: no simple ratio, the piece stays even though the old grid fits better.
    let pcs = vec![
        Piece {
            start: 0.0,
            period: 0.5,
        },
        Piece {
            start: 10.0,
            period: 60.0 / 126.0,
        },
    ];
    assert_eq!(guard_simple_ratio_pieces(&hits, &pcs).len(), 2);
    // A 4/3 piece with 3 hits in its window: below the guard's evidence floor.
    let thin: Vec<Hit> = (0..3).map(|k| Hit::new(10.0 + k as f64 * 0.5, Role::Kick)).collect();
    let pcs = vec![
        Piece {
            start: 0.0,
            period: 0.5,
        },
        Piece {
            start: 10.0,
            period: 0.375,
        },
    ];
    assert_eq!(guard_simple_ratio_pieces(&thin, &pcs).len(), 2);
}
