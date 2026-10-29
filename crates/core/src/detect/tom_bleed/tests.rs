use super::*;

const SR: u32 = 48_000;
/// The amplitude of tom 1's hits together with the kick (see the unison test).
const UNISON: f32 = 0.05;

/// A decaying tone burst with a sharp attack at `at` (samples).
fn burst(x: &mut [f32], at: usize, amp: f32, freq: f64) {
    for i in 0..(SR as usize / 5) {
        let k = at + i;
        if k >= x.len() {
            break;
        }
        let t = i as f64 / SR as f64;
        x[k] += amp * ((2.0 * std::f64::consts::PI * freq * t).sin() * (-t / 0.08).exp()) as f32;
    }
}

/// A deterministic noise floor (LCG), so no mic is digital silence.
fn floor(x: &mut [f32], amp: f32, seed: u64) {
    let mut s = seed;
    for v in x.iter_mut() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *v += amp * (((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0) as f32;
    }
}

/// A session of tom close mics. `plays[d]`: the hit times (s) and amplitudes of drum d (mic d is its close mic);
/// a hit of drum d also reaches mic m at `leak(d, m)` of its amplitude 1 ms later. `gains[m]` scales mic m (a hot
/// preamp). Holds the signals and, per mic, the hits a detector lists: its own and every hit of another drum whose
/// amplitude on the mic is at least `listed`.
struct Kit {
    x: Vec<Vec<f32>>,
    hits: Vec<Vec<i64>>,
}

fn kit(plays: &[Vec<(f64, f32)>], leak: &dyn Fn(usize, usize) -> f32, gains: &[f32], listed: f32) -> Kit {
    kit_f(plays, leak, gains, listed, &[140.0, 110.0, 85.0, 70.0])
}

/// [`kit`] with the pitch of each drum (`freqs[d % len]`).
fn kit_f(
    plays: &[Vec<(f64, f32)>],
    leak: &dyn Fn(usize, usize) -> f32,
    gains: &[f32],
    listed: f32,
    freqs: &[f64],
) -> Kit {
    let n = SR as usize * 20;
    let k = plays.len();
    let mut x = vec![vec![0.0f32; n]; k];
    let mut hits = vec![Vec::new(); k];
    for (d, play) in plays.iter().enumerate() {
        for &(t, a) in play {
            let at = (t * SR as f64) as usize;
            for m in 0..k {
                let (amp, s) = if m == d {
                    (a * gains[m], at)
                } else {
                    (a * leak(d, m) * gains[m], at + SR as usize / 1000)
                };
                burst(&mut x[m], s, amp, freqs[d % freqs.len()]);
                if m == d || amp >= listed {
                    hits[m].push(s as i64);
                }
            }
        }
    }
    for (m, v) in x.iter_mut().enumerate() {
        floor(v, 1e-4, 11 + m as u64);
    }
    Kit { x, hits }
}

fn mics<'a>(k: &'a Kit, drums: &[&str]) -> Vec<TomMic<'a>> {
    k.x.iter()
        .zip(&k.hits)
        .zip(drums)
        .map(|((x, h), d)| TomMic {
            drum: d.to_string(),
            signal: x,
            hits: h.clone(),
        })
        .collect()
}

/// Drum d plays `n` hits from `t0` every `step` s, loud ones (0.5) and every third one soft (0.1).
fn part(t0: f64, step: f64, n: usize) -> Vec<(f64, f32)> {
    (0..n)
        .map(|i| (t0 + i as f64 * step, if i % 3 == 2 { 0.1 } else { 0.5 }))
        .collect()
}

/// Three toms (drums 0 to 2), a snare (3, 200 Hz) and a kick (4, 55 Hz): every drum reaches every mic (the parts
/// lie 300 ms apart, so one drum's decay does not reach the next drum's window). The
/// snare is loud on the tom mics (0.3), the kick and the other toms at 0.1; the toms reach the snare and kick
/// mics at 0.05.
fn sk_kit(plays: &[Vec<(f64, f32)>]) -> Kit {
    sk_kit_listed(plays, 0.009)
}

/// [`sk_kit`] where a mic lists another drum's hit only at `listed` or more of its amplitude.
fn sk_kit_listed(plays: &[Vec<(f64, f32)>], listed: f32) -> Kit {
    let leak = |d: usize, m: usize| match (d, m) {
        (3, 0..=2) => 0.3,
        (0..=2, 3 | 4) => 0.05,
        _ => 0.1,
    };
    kit_f(plays, &leak, &[1.0; 5], listed, &[140.0, 110.0, 85.0, 200.0, 55.0])
}

fn sources<'a>(k: &'a Kit, which: &[(usize, SourceKind)]) -> Vec<SourceMic<'a>> {
    which
        .iter()
        .map(|&(m, kind)| SourceMic {
            kind,
            signal: &k.x[m],
            hits: k.hits[m].clone(),
        })
        .collect()
}

/// The hits of mic `m` that lie 1 ms after a hit of drum `d` (its bleed).
fn bleed_of(k: &Kit, play: &[(f64, f32)], m: usize) -> Vec<usize> {
    let at: Vec<i64> = play.iter().map(|&(t, _)| (t * SR as f64) as i64 + 48).collect();
    (0..k.hits[m].len()).filter(|&j| at.contains(&k.hits[m][j])).collect()
}

#[test]
fn version_2_marks_the_snare_and_kick_bleed_and_keeps_the_own_hits() {
    let plays = [
        part(0.5, 1.5, 12),
        part(0.8, 1.5, 12),
        part(1.1, 1.5, 12),
        part(1.4, 1.5, 12),
        part(1.7, 1.5, 12),
    ];
    let k = sk_kit(&plays);
    let toms = &mics(&k, &["tom1", "tom2", "tom3", "x", "y"])[..3];
    let src = sources(&k, &[(3, SourceKind::Snare), (4, SourceKind::Kick)]);
    let p = TomBleedParams::default();
    let v1 = tom_bleed(toms, SR, &p).unwrap();
    let v2 = tom_bleed_with(toms, &src, SR, &p).unwrap();
    assert_eq!(v2.off, None);
    assert!(v2.sources.iter().all(|u| u.unused.is_none()), "{:?}", v2.sources);
    assert_eq!(v2.sources.len(), 6);
    for m in 0..3 {
        let own: Vec<i64> = plays[m].iter().map(|&(t, _)| (t * SR as f64) as i64).collect();
        for (j, &h) in k.hits[m].iter().enumerate() {
            if own.contains(&h) {
                assert!(
                    !v2.bleed[m][j],
                    "mic {m}: own hit {h} kept: {:?}",
                    v2.source_excess[m][j]
                );
            }
        }
        let snare = bleed_of(&k, &plays[3], m);
        let kick = bleed_of(&k, &plays[4], m);
        assert!(snare.len() >= 8 && kick.len() >= 8, "mic {m}: the listed bleed");
        assert!(
            snare.iter().chain(&kick).all(|&j| v2.bleed[m][j]),
            "mic {m}: snare and kick bleed marked"
        );
        assert!(
            snare.iter().all(|&j| !v1.bleed[m][j]),
            "mic {m}: rule CD alone keeps the snare bleed"
        );
        // rule CD's marks stay
        assert!(v1.bleed[m].iter().zip(&v2.bleed[m]).all(|(a, b)| !a || *b));
    }
    // the sources take no part in rule CD: same measures, gains and merges with and without them
    assert_eq!(
        (v1.mics.clone(), v1.transfers.clone()),
        (v2.mics.clone(), v2.transfers.clone())
    );
    // nothing is marked when rule CD is off (one played tom)
    let one = tom_bleed_with(&toms[..1], &src, SR, &p).unwrap();
    assert!(one.off.is_some() && one.n_bleed() == 0);
}

#[test]
fn a_kick_and_tom_unison_keeps_the_tom_hit() {
    // tom 1 also plays soft hits together with every other loud kick hit: the kick excess of those lies between
    // the kick threshold (4.0 dB) and the tom threshold (9.2 dB)
    let kick = part(1.7, 1.5, 12);
    let mut t1 = part(0.5, 1.5, 12);
    let unison: Vec<(f64, f32)> = kick
        .iter()
        .step_by(2)
        .filter(|&&(_, a)| a > 0.2)
        .map(|&(t, _)| (t, UNISON))
        .collect();
    t1.extend(&unison);
    let plays = [t1, part(0.8, 1.5, 12), part(1.1, 1.5, 12), Vec::new(), kick];
    let k = sk_kit(&plays);
    let toms = &mics(&k, &["tom1", "tom2", "tom3"])[..3];
    let src = sources(&k, &[(4, SourceKind::Kick)]);
    let mut p = TomBleedParams::default();
    let r = tom_bleed_with(toms, &src, SR, &p).unwrap();
    let at: Vec<usize> = unison
        .iter()
        .map(|&(t, _)| k.hits[0].iter().position(|&h| h == (t * SR as f64) as i64).unwrap())
        .collect();
    for &j in &at {
        let e = r.source_excess[0][j][1];
        assert!((4.0..9.2).contains(&e), "unison hit {j}: kick excess {e}");
        assert!(!r.bleed[0][j], "unison hit {j} kept");
    }
    // the same hits at the tom threshold would be lost
    p.kick_threshold_db = p.snare_threshold_db;
    let r = tom_bleed_with(toms, &src, SR, &p).unwrap();
    assert!(at.iter().all(|&j| r.bleed[0][j]));
}

#[test]
fn the_source_gain_is_calibrated_where_the_tom_has_no_hit() {
    // the snare plays 12 loud hits; tom 1 strikes together with 5 of them. The tom mics list only their own hits
    // (the snare bleed, 0.15, stays under the listing level), so 7 snare hits find tom 1 quiet. G(S->A) must come
    // from those (the snare alone, about -10 dB); from the 5 unison hits it would be about the tom's own level,
    // and the unison hits would be lost
    let snare: Vec<(f64, f32)> = (0..12).map(|i| (1.4 + 1.5 * i as f64, 0.5)).collect();
    let mut t1 = part(0.5, 1.5, 12);
    let unison: Vec<(f64, f32)> = snare.iter().step_by(2).take(5).map(|&(t, _)| (t, 0.5)).collect();
    t1.extend(&unison);
    let plays = [t1, part(0.8, 1.5, 12), part(1.1, 1.5, 12), snare.clone()];
    let k = sk_kit_listed(&plays, 0.2);
    let toms = &mics(&k, &["tom1", "tom2", "tom3"])[..3];
    let p = TomBleedParams::default();
    let r = tom_bleed_with(toms, &sources(&k, &[(3, SourceKind::Snare)]), SR, &p).unwrap();
    // the reference: the same snare without the unison hits
    let plain = [part(0.5, 1.5, 12), part(0.8, 1.5, 12), part(1.1, 1.5, 12), snare];
    let k0 = sk_kit_listed(&plain, 0.2);
    let r0 = tom_bleed_with(
        &mics(&k0, &["tom1", "tom2", "tom3"])[..3],
        &sources(&k0, &[(3, SourceKind::Snare)]),
        SR,
        &p,
    )
    .unwrap();
    let (g, g0) = (r.sources[0].gain_db.unwrap(), r0.sources[0].gain_db.unwrap());
    assert!(
        (g - g0).abs() < 0.5,
        "G(S->A) {g} dB against {g0} dB without the unison hits"
    );
    assert_eq!(r.sources[0].unused, None);
    assert_eq!(r.off, None, "rule CD is on, so the marks count");
    let at: Vec<usize> = unison
        .iter()
        .map(|&(t, _)| k.hits[0].iter().position(|&h| h == (t * SR as f64) as i64).unwrap())
        .collect();
    assert_eq!(at.len(), 5);
    for &j in &at {
        assert!(
            !r.bleed[0][j],
            "unison hit {j} kept: snare excess {}",
            r.source_excess[0][j][0]
        );
    }
}

#[test]
fn a_source_is_not_used_without_a_gain_when_the_tom_is_silent_or_in_a_shared_pair() {
    let plays = [
        part(0.5, 1.5, 12),
        part(0.8, 1.5, 12),
        part(1.1, 1.5, 12),
        part(1.4, 1.5, 12),
    ];
    let k = sk_kit(&plays);
    let toms = &mics(&k, &["tom1", "tom2", "tom3"])[..3];
    let p = TomBleedParams::default();
    let use_of = |src: &[SourceMic]| tom_bleed_with(toms, src, SR, &p).unwrap().sources[0].unused;
    // no gain: a source with four hits
    let few = SourceMic {
        kind: SourceKind::Snare,
        signal: &k.x[3],
        hits: k.hits[3][..4].to_vec(),
    };
    assert_eq!(use_of(&[few]), Some(Unused::NoGain));
    // shared: the snare mic is tom 1's own signal (the pair hears one drum as loud)
    let same = SourceMic {
        kind: SourceKind::Snare,
        signal: &k.x[0],
        hits: k.hits[0].clone(),
    };
    assert_eq!(use_of(&[same]), Some(Unused::Shared));
    // silent: tom 1 gated to silence at the snare's hits
    let mut gated = k.x[0].clone();
    for &(t, _) in &plays[3] {
        let a = (t * SR as f64) as usize;
        gated[a - 2400..a + 2400].iter_mut().for_each(|v| *v = 0.0);
    }
    let toms_g = [
        TomMic {
            drum: "tom1".into(),
            signal: &gated,
            hits: k.hits[0].clone(),
        },
        toms[1].clone(),
        toms[2].clone(),
    ];
    let src = sources(&k, &[(3, SourceKind::Snare)]);
    let r = tom_bleed_with(&toms_g, &src, SR, &p).unwrap();
    assert_eq!(r.sources[0].unused, Some(Unused::Silent), "{:?}", r.sources[0]);
}

#[test]
fn bleed_of_other_toms_is_marked_and_own_hits_stay() {
    let plays = [part(0.5, 0.9, 20), part(0.8, 0.9, 20), part(1.1, 0.9, 20)];
    let k = kit(&plays, &|_, _| 0.1, &[1.0, 1.0, 1.0], 0.009);
    let r = tom_bleed(&mics(&k, &["tom1", "tom2", "tom3"]), SR, &TomBleedParams::default()).unwrap();
    assert_eq!(r.off, None);
    assert!(r.mics.iter().all(|m| m.played && !m.silent), "{:?}", r.mics);
    for (m, play) in plays.iter().enumerate() {
        let own: Vec<i64> = play.iter().map(|&(t, _)| (t * SR as f64) as i64).collect();
        for (j, &h) in k.hits[m].iter().enumerate() {
            assert_eq!(
                r.bleed[m][j],
                !own.contains(&h),
                "mic {m} hit {h}: excess {}",
                r.excess[m][j]
            );
        }
    }
    assert_eq!(r.n_bleed(), r.bleed.iter().flatten().filter(|&&b| b).count());
    assert_eq!(r.transfers.len(), 6, "every ordered pair of drums has a gain");
    assert!(r.merged.is_empty(), "played pairs stay two drums");
}

#[test]
fn a_bleed_only_mic_takes_no_real_hit_at_any_gain() {
    // mic 2 is a tom nobody plays, next to tom 1: it hears drum 0 at -20 dB and drum 1 at -60 dB. Its gain:
    // as recorded (1.0: 20 dB under the played mics, it fails the level test and is not played), a hot preamp
    // (6.0: 4.4 dB under), and drum 0's hits 9 and 15 dB louder on it than on drum 0's own mic
    let plays = [part(0.5, 0.9, 20), part(0.8, 0.9, 20), Vec::new()];
    let leak = |d: usize, m: usize| if m == 2 && d == 1 { 0.001 } else { 0.1 };
    let own: Vec<Vec<i64>> = plays
        .iter()
        .map(|p| p.iter().map(|&(t, _)| (t * SR as f64) as i64).collect())
        .collect();
    for g in [1.0f32, 10f32.powf(29.0 / 20.0), 10f32.powf(35.0 / 20.0), 6.0] {
        let k = kit(&plays, &leak, &[1.0, 1.0, g], 0.009);
        let r = tom_bleed(&mics(&k, &["tom1", "tom2", "tom3"]), SR, &TomBleedParams::default()).unwrap();
        assert_eq!(r.off, None, "gain {g}: {:?} {:?}", r.merged, r.mics);
        for (m, own) in own.iter().enumerate().take(2) {
            for (j, &h) in k.hits[m].iter().enumerate() {
                if own.contains(&h) {
                    assert!(
                        !r.bleed[m][j],
                        "gain {g}: mic {m} lost own hit {h}, excess {}",
                        r.excess[m][j]
                    );
                }
            }
        }
        // mic 2's hits of drum 0: marked while mic 2 fails the level test, kept (false hits) once the pair counts
        // as one drum
        let of_drum0: Vec<bool> = k.hits[2]
            .iter()
            .zip(&r.bleed[2])
            .filter(|(&h, _)| own[0].iter().any(|&o| (h - o).abs() < 100))
            .map(|(_, &b)| b)
            .collect();
        assert!(!of_drum0.is_empty());
        if g == 1.0 {
            assert!(
                r.merged.is_empty() && !r.mics[2].level_ok && !r.mics[2].played,
                "{:?}",
                r.mics[2]
            );
            assert!(of_drum0.iter().all(|&b| b), "gain 1: mic 2's bleed of drum 0 is marked");
        } else {
            assert_eq!(r.merged, vec![(0, 2)], "gain {g}");
            assert!(
                of_drum0.iter().all(|&b| !b),
                "gain {g}: one drum, mic 2 keeps its false hits"
            );
        }
    }
}

#[test]
fn a_quiet_main_tom_with_a_hot_bleed_only_mic_loses_no_real_hit() {
    // fix round 5 (the verifier's case): drum 2's mic is 6.2 dB under the other toms, mic 3 hears only drum 2
    // and lies 9 or 15 dB above drum 2's own mic. The weaker mic of the shared pair is the main tom, 6.2 dB under
    // the others: less than 8 dB, so the pair counts as one drum
    let plays = [part(0.5, 0.9, 20), part(0.8, 0.9, 20), part(1.1, 0.9, 20), Vec::new()];
    let leak = |d: usize, m: usize| if m == 3 && d != 2 { 0.001 } else { 0.1 };
    // -3.7 dB on the mic: drum 2's lower pitch loses 2.5 dB more in the tom band, so its level lies 6.2 dB under
    let main = 10f32.powf(-3.7 / 20.0);
    for db in [9.0f32, 15.0] {
        let gains = [1.0, 1.0, main, main * 10f32.powf((20.0 + db) / 20.0)];
        let k = kit(&plays, &leak, &gains, 0.009);
        let r = tom_bleed(
            &mics(&k, &["tom1", "tom2", "tom3", "tom4"]),
            SR,
            &TomBleedParams::default(),
        )
        .unwrap();
        assert_eq!(r.off, None, "{db} dB");
        let lv = |m: usize| r.mics[m].level_db.unwrap();
        let gap = (lv(0) + lv(1)) / 2.0 - lv(2);
        assert!((5.7..6.7).contains(&gap), "the main tom lies {gap} dB under the others");
        for (m, play) in plays.iter().enumerate().take(3) {
            let own: Vec<i64> = play.iter().map(|&(t, _)| (t * SR as f64) as i64).collect();
            for (j, &h) in k.hits[m].iter().enumerate() {
                if own.contains(&h) {
                    assert!(
                        !r.bleed[m][j],
                        "{db} dB: mic {m} lost own hit {h}, excess {}",
                        r.excess[m][j]
                    );
                }
            }
        }
        assert_eq!(r.merged, vec![(2, 3)], "{db} dB");
    }
}

#[test]
fn a_bleed_only_mic_8_4_db_or_more_under_keeps_the_direction() {
    let plays = [part(0.5, 0.9, 20), part(0.8, 0.9, 20), part(1.1, 0.9, 20), Vec::new()];
    for db in [8.4f32, 13.3] {
        // mic 3 hears drum 2 so that its level lies `db` under the other drums (drum 2 itself lies 2.5 dB under them)
        let l = 10f32.powf(-(db - 2.5) / 20.0);
        let leak = move |d: usize, m: usize| match (d, m) {
            (2, 3) => l,
            (_, 3) => 0.001,
            _ => 0.1,
        };
        let k = kit(&plays, &leak, &[1.0; 4], 0.009);
        let r = tom_bleed(
            &mics(&k, &["tom1", "tom2", "tom3", "tom4"]),
            SR,
            &TomBleedParams::default(),
        )
        .unwrap();
        let lv = |m: usize| r.mics[m].level_db.unwrap();
        let gap = (lv(0) + lv(1)) / 2.0 - lv(3);
        assert!((gap - db as f64).abs() < 0.5, "mic 3 lies {gap} dB under the others");
        assert!(
            r.merged.is_empty() && !r.mics[3].played,
            "{db} dB: {:?} {:?}",
            r.merged,
            r.mics[3]
        );
        let own2: Vec<i64> = plays[2].iter().map(|&(t, _)| (t * SR as f64) as i64 + 48).collect();
        let of_drum2: Vec<bool> = k.hits[3]
            .iter()
            .zip(&r.bleed[3])
            .filter(|(h, _)| own2.contains(h))
            .map(|(_, &b)| b)
            .collect();
        assert!(
            !of_drum2.is_empty() && of_drum2.iter().all(|&b| b),
            "{db} dB: its bleed of drum 2 is marked"
        );
    }
}

#[test]
fn merged_pairs_need_a_shared_pair_and_a_weaker_mic_that_passes_the_level_test() {
    let stats = |level: f64| MicStats {
        level_db: Some(level),
        ..Default::default()
    };
    let t = |source, target, gain_db| Transfer {
        source,
        target,
        gain_db,
        hits: 10,
        quiet_only: false,
    };
    let p = TomBleedParams::default();
    let g = [0, 1, 2];
    let mut r = TomBleed {
        mics: vec![stats(-10.0), stats(-12.0), stats(-30.0)],
        transfers: vec![t(0, 1, -15.0), t(1, 0, -15.0), t(0, 2, -20.0), t(2, 0, 20.0)],
        ..Default::default()
    };
    assert!(
        merged_pairs(&r, &g, &p).is_empty(),
        "a played pair (-30 dB) and a weak bleed-only mic (18 dB under)"
    );
    r.mics[2].level_db = Some(-15.0);
    assert_eq!(merged_pairs(&r, &g, &p), vec![(0, 2)], "a hot bleed-only mic: one drum");
    r.mics[2].level_db = Some(5.0);
    assert_eq!(
        merged_pairs(&r, &g, &p),
        vec![(0, 2)],
        "hotter than the played mic: the played one is weaker, it passes"
    );
    r.mics[2].level_db = Some(-15.0);
    r.transfers[0].gain_db = -5.0;
    r.transfers[1].gain_db = -5.0;
    assert_eq!(
        merged_pairs(&r, &g, &p),
        vec![(0, 1), (0, 2)],
        "sum -10 dB: shared, the weaker mic 1 passes"
    );
    r.mics[1].level_db = Some(-22.0);
    assert_eq!(
        merged_pairs(&r, &g, &p),
        vec![(0, 1), (0, 2)],
        "7 dB under: one drum (a quiet main tom passes the 8 dB margin)"
    );
    r.mics[1].level_db = Some(-23.4);
    assert_eq!(
        merged_pairs(&r, &g, &p),
        vec![(0, 2)],
        "8.4 dB under: today's direction"
    );
    r.mics[1].level_db = Some(-25.0);
    r.transfers[0].gain_db = -21.0;
    assert!(
        !merged_pairs(&r, &g, &p).contains(&(0, 1)),
        "sum exactly -26 dB is not shared"
    );
    assert_eq!(
        merged_pairs(&r, &[0, 1, 1], &p),
        vec![(0, 2)],
        "a drum group counts once in the level test"
    );
}

#[test]
fn the_quiet_window_picks_the_values_of_the_gain() {
    // drum 1 is struck 5 ms after every second hit of drum 0: there mic 1 is not quiet (its own hit within 30
    // ms), and the gain of 0 -> 1 uses only the other half of drum 0's strong hits
    let p0: Vec<(f64, f32)> = (0..12).map(|i| (0.5 + i as f64 * 0.9, 0.5)).collect();
    let p1: Vec<(f64, f32)> = (0..12).step_by(2).map(|i| (0.505 + i as f64 * 0.9, 0.5)).collect();
    let k = kit(&[p0, p1], &|_, _| 0.1, &[1.0, 1.0], 1.0);
    // half the hits are struck together: keep the two drums apart (no shared pair) to see the quiet window
    let p = TomBleedParams {
        shared_sum_db: f64::INFINITY,
        ..Default::default()
    };
    let r = tom_bleed(&mics(&k, &["tom1", "tom2"]), SR, &p).unwrap();
    let g = r.transfers.iter().find(|t| t.source == 0 && t.target == 1).unwrap();
    assert_eq!((g.hits, g.quiet_only), (6, true), "{g:?}");
    let g = r.transfers.iter().find(|t| t.source == 1 && t.target == 0).unwrap();
    assert_eq!(
        (g.hits, g.quiet_only),
        (6, false),
        "mic 0 has a hit within 30 ms of each: {g:?}"
    );
}

#[test]
fn one_played_tom_or_a_silent_mic_turns_the_rule_off() {
    let plays = [part(0.5, 0.9, 20), Vec::new()];
    let k = kit(&plays, &|_, _| 0.1, &[1.0, 1.0], 0.009);
    let r = tom_bleed(&mics(&k, &["tom1", "tom2"]), SR, &TomBleedParams::default()).unwrap();
    // with no third drum the bleed-only mic cannot fail a level test: the shared pair counts as one drum, still one
    assert_eq!(r.merged, vec![(0, 1)]);
    assert_eq!(r.off, Some(Off::FewerThanTwoPlayed { played: vec![0, 1] }));
    assert_eq!(r.n_bleed(), 0);
    // two mics of one drum count once
    let plays = [part(0.5, 0.9, 20), part(0.5, 0.9, 20)];
    let k = kit(&plays, &|_, _| 0.0, &[1.0, 1.0], 1.0);
    let r = tom_bleed(&mics(&k, &["tom1", "tom1"]), SR, &TomBleedParams::default()).unwrap();
    assert!(matches!(r.off, Some(Off::FewerThanTwoPlayed { .. })), "{:?}", r.off);
    assert!(r.transfers.is_empty(), "one drum: no transfer");
    // a gated mic: digital silence between its own hits
    let plays = [part(0.5, 0.9, 20), part(0.8, 0.9, 20), part(1.1, 0.9, 20)];
    let mut k = kit(&plays, &|_, _| 0.1, &[1.0, 1.0, 1.0], 0.009);
    let own: Vec<usize> = plays[2].iter().map(|&(t, _)| (t * SR as f64) as usize).collect();
    for (i, v) in k.x[2].iter_mut().enumerate() {
        if !own
            .iter()
            .any(|&s| i >= s.saturating_sub(SR as usize / 50) && i < s + SR as usize / 5)
        {
            *v = 0.0;
        }
    }
    k.hits[2] = own.iter().map(|&s| s as i64).collect();
    let r = tom_bleed(&mics(&k, &["tom1", "tom2", "tom3"]), SR, &TomBleedParams::default()).unwrap();
    assert_eq!(r.off, Some(Off::DigitalSilence { mics: vec![2] }));
    assert!(r.mics[2].silent_share.unwrap() > 0.9 && r.mics[0].silent_share == Some(0.0));
    assert_eq!(r.n_bleed(), 0);
    assert_eq!(
        tom_bleed(&[], 0, &TomBleedParams::default()),
        Err(AttributionError::ZeroRate)
    );
    let r = tom_bleed(&[], SR, &TomBleedParams::default()).unwrap();
    assert_eq!(r.off, Some(Off::FewerThanTwoPlayed { played: vec![] }));
}

#[test]
fn level_db_pads_with_zeros_and_strong_hits_follow_the_peak() {
    let sec = band_sections(Role::Tom.band(), SR);
    let mut x = vec![0.0f32; 4800];
    burst(&mut x, 100, 0.5, 110.0);
    // a window that starts before the signal equals the same window on a zero-padded copy
    let mut padded = vec![0.0f32; 2400];
    padded.extend_from_slice(&x);
    let a = level_db(&x, &sec, 100, (-24, 384), 2400);
    let b = level_db(&padded, &sec, 2500, (-24, 384), 2400);
    assert!((a - b).abs() < 1e-12 && a > -20.0, "{a} {b}");
    // past the end: zeros, not a shorter window
    let tail = level_db(&x, &sec, 4700, (-24, 384), 2400);
    assert!(tail.is_finite());
    assert!((level_db(&[], &sec, 0, (-24, 384), 2400) + 180.0).abs() < 1e-9);
    let mut y = vec![0.0f32; 48_000];
    burst(&mut y, 100, 0.5, 110.0);
    burst(&mut y, 20_000, 0.4, 110.0);
    burst(&mut y, 40_000, 0.1, 110.0);
    assert_eq!(strong_hits(&y, &[100, 20_000, 40_000], (-24, 240), 6.0), vec![0, 1]);
    assert!(strong_hits(&x, &[], (-24, 240), 6.0).is_empty());
    assert!(near(&[10, 20], 25, 5) && !near(&[10, 20], 26, 5) && !near(&[], 0, 5));
    assert!((broad_db(&x, 100, (-24, 240)) - broad_db(&x, 100, (-24, 240))).abs() < 1e-12);
}
// ---- version 3: the tom pair rule (P4-21)

/// Two mics (drums `a`, `b`, or the same drum) with the hits `(t, level dB)`; every hit is output and a target.
/// The own-mic attack rises default to 0 dB (the guard keeps nothing); set `rises` and `guarded` for the guard.
struct Pairs {
    drums: Vec<&'static str>,
    times: Vec<Vec<f64>>,
    levels: Vec<Vec<f64>>,
    output: Vec<Vec<bool>>,
    target: Vec<Vec<bool>>,
    rises: Vec<Vec<f64>>,
    guarded: Vec<bool>,
}

impl Pairs {
    fn new(drums: &[&'static str], hits: &[&[(f64, f64)]]) -> Pairs {
        Pairs {
            drums: drums.to_vec(),
            times: hits.iter().map(|h| h.iter().map(|x| x.0).collect()).collect(),
            levels: hits.iter().map(|h| h.iter().map(|x| x.1).collect()).collect(),
            output: hits.iter().map(|h| vec![true; h.len()]).collect(),
            target: hits.iter().map(|h| vec![true; h.len()]).collect(),
            rises: hits.iter().map(|h| vec![0.0; h.len()]).collect(),
            guarded: vec![true; hits.len()],
        }
    }

    fn marks(&self) -> Vec<Vec<bool>> {
        let mics: Vec<PairMic> = (0..self.drums.len())
            .map(|i| PairMic {
                drum: self.drums[i],
                signal: &[],
                times: &self.times[i],
                output: &self.output[i],
                target: &self.target[i],
                guarded: self.guarded[i],
            })
            .collect();
        let rises: Vec<Option<Vec<f64>>> = (0..self.drums.len())
            .map(|i| self.guarded[i].then(|| self.rises[i].clone()))
            .collect();
        pair_marks(&mics, &self.levels, &rises, &TomBleedParams::default())
    }
}

/// Three hits per mic at 1, 2 and 3 s, level 0 dB everywhere (the median is 0 dB), except the hit under test.
fn flat(a_t: f64, a_lvl: f64, b_t: f64, b_lvl: f64) -> Pairs {
    Pairs::new(
        &["a", "b"],
        &[
            &[(1.0, 0.0), (2.0, 0.0), (a_t, a_lvl)],
            &[(1.5, 0.0), (2.5, 0.0), (b_t, b_lvl)],
        ],
    )
}

#[test]
fn rule_version_5_and_the_pair_parameters() {
    assert_eq!(RULE_VERSION, 5);
    let p = TomBleedParams::default();
    assert!((p.winner_window_ms - 12.0).abs() < 1e-12 && (p.winner_margin_db - 6.0).abs() < 1e-12);
    assert!(
        (p.pair_keep_rise_db - PAIR_KEEP_RISE_DB).abs() < 1e-12 && (PAIR_KEEP_RISE_DB - 10.0).abs() < 1e-12,
        "the guard keeps a 10 dB or more own-mic rise (#177)"
    );
    assert!((PAIR_HP_HZ - 2000.0).abs() < 1e-12);
    assert!(
        (p.kick_shared_threshold_db - KICK_SHARED_THRESHOLD_DB).abs() < 1e-12
            && (KICK_SHARED_THRESHOLD_DB - 8.0).abs() < 1e-12,
        "the guarded shared kick marks under 8 dB (#362)"
    );
    assert!(
        (p.kick_shared_rise_db - KICK_SHARED_RISE_DB).abs() < 1e-12 && (KICK_SHARED_RISE_DB - 15.0).abs() < 1e-12,
        "the guarded shared kick keeps a 15 dB or more 2 kHz rise (#362)"
    );
}

#[test]
fn a_pair_needs_the_12_ms_window() {
    // 12 ms apart (and a float rounding of it): a pair; 12.2 ms apart: none
    let m = flat(3.0, 0.0, 3.012, -10.0).marks();
    assert_eq!((m[0][2], m[1][2]), (false, true), "{m:?}");
    let m = flat(3.0, 0.0, 3.0 - 0.012, -10.0).marks();
    assert_eq!((m[0][2], m[1][2]), (false, true), "before the winner too: {m:?}");
    let m = flat(3.0, 0.0, 3.0122, -10.0).marks();
    assert_eq!((m[0][2], m[1][2]), (false, false), "{m:?}");
    let m = flat(3.0, 0.0, 3.0 + 0.0125, -10.0).marks();
    assert!(!m[1][2], "{m:?}");
}

#[test]
fn the_lower_hit_goes_at_6_db_and_not_below() {
    let m = flat(3.0, 0.0, 3.001, -6.0).marks();
    assert_eq!((m[0][2], m[1][2]), (false, true), "6.0 dB: the lower goes: {m:?}");
    let m = flat(3.0, 0.0, 3.001, -5.99).marks();
    assert_eq!((m[0][2], m[1][2]), (false, false), "5.99 dB: both stay: {m:?}");
    let m = flat(3.0, -6.0, 3.001, 0.0).marks();
    assert_eq!((m[0][2], m[1][2]), (true, false), "the other way round: {m:?}");
    let m = flat(3.0, 0.0, 3.001, 0.0).marks();
    assert_eq!(
        (m[0][2], m[1][2]),
        (false, false),
        "equal levels (a unison stroke): {m:?}"
    );
}

/// Mic a has a +30 dB hit at 1 s (median 0 dB); mic b's hit at 1.001 s has the relative level `rel_b` (its other
/// hits are at 0 dB). Returns whether b's hit is marked.
fn floor_case(rel_b: f64) -> bool {
    // b's other hits are at 0 dB, so its median is 0 dB and the hit under test has the relative level `rel_b`
    let p = Pairs::new(
        &["a", "b"],
        &[
            &[(1.0, 30.0), (2.0, 0.0), (3.0, 0.0)],
            &[(1.001, rel_b), (5.0, 0.0), (6.0, 0.0)],
        ],
    );
    p.marks()[1][0]
}

#[test]
fn a_hit_is_marked_only_when_it_is_3_db_under_its_own_median() {
    assert!((PAIR_FLOOR_DB + 3.0).abs() < 1e-12 && (TomBleedParams::default().winner_floor_db + 3.0).abs() < 1e-12);
    assert!(floor_case(-3.1), "3.1 dB under: bleed");
    assert!(floor_case(-3.0), "3.0 dB under: bleed");
    assert!(
        !floor_case(-2.9),
        "2.9 dB under: stays, though the winner is 30 dB louder"
    );
    assert!(!floor_case(0.0) && !floor_case(5.0), "a loud hit stays");
    // a loud unison: both toms at their typical level, one hit 7 dB over its partner's relative level: both stay
    let p = Pairs::new(
        &["a", "b"],
        &[
            &[(1.0, 7.0), (2.0, 0.0), (3.0, 0.0)],
            &[(1.001, 0.0), (5.0, 0.0), (6.0, 0.0)],
        ],
    );
    let m = p.marks();
    assert!(m.iter().flatten().all(|&b| !b), "{m:?}");
    // a hit that stays still wins against a hit of a third mic
    let p = Pairs::new(
        &["a", "b", "c"],
        &[
            &[(1.0, 12.0), (2.0, 0.0), (3.0, 0.0)],
            &[(1.001, -1.0), (5.0, 0.0), (6.0, 0.0)],
            &[(1.002, -8.0), (7.0, 0.0), (8.0, 0.0)],
        ],
    );
    let m = p.marks();
    assert_eq!((m[0][0], m[1][0], m[2][0]), (false, false, true), "{m:?}");
}

#[test]
fn the_level_is_against_the_median_of_the_mics_own_output_hits() {
    // mic b is 20 dB louder in all its hits: raw levels differ by 20 dB at the pair, the relative levels by 0
    let p = Pairs::new(
        &["a", "b"],
        &[
            &[(1.0, -30.0), (2.0, -30.0), (3.0, -30.0)],
            &[(1.0, -10.0), (2.0, -10.0), (3.0, -10.0)],
        ],
    );
    let m = p.marks();
    assert!(
        m.iter().flatten().all(|&b| !b),
        "two unison strokes at their typical levels: {m:?}"
    );
    // the median of an even count is the mean of the two middle values: 0 and -8 give -4
    let p = Pairs::new(&["a", "b"], &[&[(1.0, 0.0), (2.0, -8.0)], &[(1.0, -10.0), (5.0, 0.0)]]);
    let m = p.marks();
    // a: rel 4 and -4; b: rel -10 and 5 -> the hit of b at 1.0 s (rel -10) loses against a's (rel +4): gap 14
    assert_eq!((m[0][0], m[1][0]), (false, true), "{m:?}");
    // a hit that is not in the output does not count for the median: a's output median is -5 dB (hits at 0 and -10),
    // with the loud hit out of the output (+30 dB) counted it would be 0 dB; b's hit at 1.001 s (rel -4 dB) is 9 dB under a's
    let mut p = Pairs::new(
        &["a", "b"],
        &[&[(1.0, 0.0), (2.0, -10.0), (3.0, 30.0)], &[(1.001, -8.0), (5.0, 0.0)]],
    );
    p.output[0][2] = false;
    let m = p.marks();
    assert!(m[1][0] && !m[0][0], "{m:?}");
    p.output[0][2] = true;
    assert!(
        !p.marks()[1][0],
        "with the loud hit in the output a's median is 0 dB and the gap only 4 dB"
    );
}

#[test]
fn three_toms_and_a_marked_hit_protects_no_other() {
    // tom a struck loud at 1.000 s; b hears it at -9 dB at 1.002 s and c at -14 dB at 1.010 s: both go
    let hits_a: &[(f64, f64)] = &[(1.0, 0.0), (2.0, 0.0), (3.0, 0.0)];
    let hits_b: &[(f64, f64)] = &[(1.002, -9.0), (2.5, 0.0), (3.5, 0.0)];
    let hits_c: &[(f64, f64)] = &[(1.010, -14.0), (2.7, 0.0), (3.7, 0.0)];
    let m = Pairs::new(&["a", "b", "c"], &[hits_a, hits_b, hits_c]).marks();
    assert_eq!((m[0][0], m[1][0], m[2][0]), (false, true, true), "{m:?}");
    // c at 1.020 s is 20 ms from a (outside the window) and 18 ms from b: a marked hit protects nobody,
    // and there is no winner in c's window
    // (c is 7 dB under b: a marked b would take it out, if a marked hit protected others)
    let hits_c: &[(f64, f64)] = &[(1.020, -16.0), (2.7, 0.0), (3.7, 0.0)];
    let hits_b: &[(f64, f64)] = &[(1.010, -9.0), (2.5, 0.0), (3.5, 0.0)];
    let m = Pairs::new(&["a", "b", "c"], &[hits_a, hits_b, hits_c]).marks();
    assert_eq!(
        (m[1][0], m[2][0]),
        (true, false),
        "b goes (10 ms from a); c keeps: {m:?}"
    );
    // a kept hit of the middle level: b (-5 dB: kept against a) wins against c (-11 dB) 5 ms later
    let hits_a: &[(f64, f64)] = &[(1.000, 0.0), (2.0, 0.0), (3.0, 0.0)];
    let hits_b: &[(f64, f64)] = &[(1.002, -5.0), (2.5, 0.0), (3.5, 0.0)];
    let hits_c: &[(f64, f64)] = &[(1.007, -11.0), (2.7, 0.0), (3.7, 0.0)];
    let m = Pairs::new(&["a", "b", "c"], &[hits_a, hits_b, hits_c]).marks();
    assert_eq!((m[0][0], m[1][0], m[2][0]), (false, false, true), "{m:?}");
}

#[test]
fn two_hits_of_one_mic_or_one_drum_never_pair() {
    // two hits of the same mic 5 ms apart, 20 dB apart
    let p = Pairs::new(&["a"], &[&[(1.0, 0.0), (1.005, -20.0), (2.0, 0.0)]]);
    assert!(p.marks()[0].iter().all(|&b| !b));
    // two mics of one drum (top and bottom) hear one stroke: never a pair
    let p = Pairs::new(
        &["tom1", "tom1"],
        &[
            &[(1.0, 0.0), (2.0, 0.0), (3.0, 0.0)],
            &[(1.001, -20.0), (2.0, 0.0), (3.0, 0.0)],
        ],
    );
    assert!(p.marks().iter().flatten().all(|&b| !b));
    // the same hits on two drums: the lower goes
    let p = Pairs::new(
        &["tom1", "tom2"],
        &[
            &[(1.0, 0.0), (2.0, 0.0), (3.0, 0.0)],
            &[(1.001, -20.0), (2.5, 0.0), (3.5, 0.0)],
        ],
    );
    assert!(p.marks()[1][0]);
}

#[test]
fn hits_out_of_the_output_and_edited_hits_are_not_marked_and_do_not_win() {
    let hits: [&[(f64, f64)]; 2] = [
        &[(1.0, 0.0), (2.0, 0.0), (3.0, 0.0)],
        &[(1.001, -20.0), (2.5, 0.0), (3.5, 0.0)],
    ];
    // the winner is cut or marked by version 2 (not in the output): the lower hit stays
    let mut p = Pairs::new(&["a", "b"], &hits);
    p.output[0][0] = false;
    assert!(!p.marks()[1][0] && !p.marks()[0][0]);
    // the lower hit was edited: it stays, and it is kept as it is
    let mut p = Pairs::new(&["a", "b"], &hits);
    p.target[1][0] = false;
    assert!(!p.marks()[1][0]);
    // an edited hit still wins against a hit of another drum
    let mut p = Pairs::new(&["a", "b"], &hits);
    p.target[0][0] = false;
    assert!(p.marks()[1][0]);
    // a mic without output hits has no median and takes no part
    let mut p = Pairs::new(&["a", "b"], &hits);
    p.output[1] = vec![false; 3];
    assert_eq!(p.marks()[1], vec![false; 3]);
}

#[test]
fn the_hit_level_is_the_peak_in_5_ms_before_to_30_ms_after() {
    let mut x = vec![0.0f32; 48_000];
    x[1000] = 0.5;
    x[2000] = 0.25;
    // hit at 1000 samples: the window runs from 760 to 2440 samples
    let t = 1000.0 / 48_000.0;
    assert!((pair_level_db(&x, 48_000, t) - 20.0 * 0.5f64.log10()).abs() < 1e-9);
    // a peak 4 ms before counts, one 6 ms before does not
    let early = (1000.0 + 0.004 * 48_000.0) / 48_000.0;
    assert!((pair_level_db(&x, 48_000, early) - 20.0 * 0.5f64.log10()).abs() < 1e-9);
    let later = (1000.0 + 0.006 * 48_000.0) / 48_000.0;
    assert!((pair_level_db(&x, 48_000, later) - 20.0 * 0.25f64.log10()).abs() < 1e-9);
    // the 30 ms edge: a peak 29 ms after the hit counts, one 32 ms after does not
    let inside = (2000.0 - 0.029 * 48_000.0) / 48_000.0;
    let mut y = vec![0.0f32; 48_000];
    y[2000] = 0.25;
    assert!(
        (pair_level_db(&y, 48_000, inside) - 20.0 * 0.25f64.log10()).abs() < 1e-9,
        "29 ms after: counts"
    );
    let outside = (2000.0 - 0.032 * 48_000.0) / 48_000.0;
    assert!(
        (pair_level_db(&y, 48_000, outside) + 180.0).abs() < 1e-9,
        "32 ms after: does not count"
    );
    // 31 ms after the first peak the window of a hit at 1000 samples has ended for it
    let far = (2000.0 + 0.040 * 48_000.0) / 48_000.0;
    assert!(
        (pair_level_db(&x, 48_000, far) + 180.0).abs() < 1e-9,
        "silence is -180 dB"
    );
    assert!((pair_level_db(&x, 48_000, 100.0) + 180.0).abs() < 1e-9, "past the end");
    assert!((pair_level_db(&[], 48_000, 0.0) + 180.0).abs() < 1e-9);
    assert!((median(&mut [3.0, 1.0, 2.0]).unwrap() - 2.0).abs() < 1e-12);
    assert!((median(&mut [4.0, 1.0, 2.0, 3.0]).unwrap() - 2.5).abs() < 1e-12);
    assert!(median(&mut []).is_none());
}

#[test]
fn tom_pairs_reads_the_levels_from_the_signals() {
    // tom a struck at 1 s (amp 0.5) is heard on mic b at 0.05; both mics also have a solo hit each
    let n = 5 * SR as usize;
    let mut a = vec![0.0f32; n];
    let mut b = vec![0.0f32; n];
    let at = |s: f64| (s * f64::from(SR)) as usize;
    burst(&mut a, at(1.0), 0.5, 110.0);
    burst(&mut b, at(1.001), 0.05, 110.0);
    burst(&mut a, at(2.0), 0.5, 110.0);
    burst(&mut b, at(3.0), 0.5, 140.0);
    burst(&mut a, at(3.5), 0.05, 110.0);
    let ta = [1.0, 2.0];
    let tb = [1.001, 3.0];
    let ones = [true; 2];
    // unguarded: the test checks the level path (version 3 rule); a guarded run would keep the
    // planted hit (a burst over silence has a huge attack rise) and read no levels
    let mics = [
        PairMic {
            drum: "tom1",
            signal: &a,
            times: &ta,
            output: &ones,
            target: &ones,
            guarded: false,
        },
        PairMic {
            drum: "tom2",
            signal: &b,
            times: &tb,
            output: &ones,
            target: &ones,
            guarded: false,
        },
    ];
    let m = tom_pairs(&mics, SR, &TomBleedParams::default());
    assert_eq!(m, vec![vec![false, false], vec![true, false]]);
}

/// A steady tone from `t0` to `t1` (no attack: the envelope is flat).
fn tone(x: &mut [f32], sr: u32, t0: f64, t1: f64, amp: f32, freq: f64) {
    let sr = f64::from(sr);
    for i in (t0 * sr) as usize..((t1 * sr) as usize).min(x.len()) {
        x[i] += amp * (2.0 * std::f64::consts::PI * freq * (i as f64 / sr)).sin() as f32;
    }
}

/// The #177 scenario on two close-tuned toms: tom a (the partner) is struck at 1 s (amp 0.5); tom b's hit at
/// 1.001 s is 10 dB under its own median (its other hits are 0.5) and 20 dB under the partner's stroke. The
/// own-mic content before the hit is `under_db` under the attack, in both bands (steady noise, the ring of an
/// earlier stroke). Returns (marks, rise).
fn rise_case(under_db: f64, guarded: bool) -> (Vec<Vec<bool>>, f64) {
    let n = 5 * SR as usize;
    let at = |s: f64| (s * f64::from(SR)) as usize;
    let mut a = vec![0.0f32; n];
    let mut b = vec![0.0f32; n];
    burst(&mut a, at(1.0), 0.5, 110.0);
    burst(&mut a, at(2.0), 0.5, 110.0);
    burst(&mut a, at(3.0), 0.5, 110.0);
    floor(&mut b, 0.05f32 * 10f32.powf((-under_db / 20.0) as f32), 7);
    burst(&mut b, at(1.001), 0.05, 110.0);
    burst(&mut b, at(3.0), 0.5, 140.0);
    let ta = [1.0, 2.0, 3.0];
    let tb = [1.001, 3.0];
    let ones_a = [true; 3];
    let ones_b = [true; 2];
    let mics = [
        PairMic {
            drum: "tom1",
            signal: &a,
            times: &ta,
            output: &ones_a,
            target: &ones_a,
            guarded: true,
        },
        PairMic {
            drum: "tom2",
            signal: &b,
            times: &tb,
            output: &ones_b,
            target: &ones_b,
            guarded,
        },
    ];
    let hp = zero_phase(&b, SR, &[(FilterKind::HighPass, PAIR_HP_HZ)]);
    let rise = stroke_rise_db(&b, &hp, SR, 1.001);
    (tom_pairs(&mics, SR, &TomBleedParams::default()), rise)
}

#[test]
fn a_12_db_own_mic_rise_stays_a_hit_and_a_bleed_rise_stays_marked() {
    // 12 dB of attack over the ring (the #177 real strokes): the hit stays, though it is 20 dB under
    // the partner's stroke and 10 dB under its own median
    let (m, rise) = rise_case(12.0, true);
    assert!(rise >= 11.9, "the planted 12 dB rise measures {rise}");
    assert_eq!(
        m,
        vec![vec![false; 3], vec![false, false]],
        "the real stroke stays: {m:?}"
    );
    // no attack on the own mic (the partner's arrival over the ring): the bleed stays marked
    let (m, rise) = rise_case(0.0, true);
    assert!(rise < 10.0, "no attack: the rise measures {rise}");
    assert_eq!(m, vec![vec![false; 3], vec![true, false]], "the bleed is marked: {m:?}");
    // a recorded version 3 re-pick next to a version 4 target: no guard, the version 3 rule
    let (m, _) = rise_case(12.0, false);
    assert_eq!(
        m,
        vec![vec![false; 3], vec![true, false]],
        "unguarded: the version 3 rule: {m:?}"
    );
}

#[test]
fn the_keep_guard_boundary_is_10_db() {
    let mut p = Pairs::new(
        &["a", "b"],
        &[
            &[(1.0, 0.0), (2.0, 0.0), (3.0, 0.0)],
            &[(1.001, -10.0), (5.0, 0.0), (6.0, 0.0)],
        ],
    );
    p.rises[1][0] = 9.9;
    assert!(p.marks()[1][0], "9.9 dB under what came before: bleed");
    p.rises[1][0] = 10.0;
    assert!(!p.marks()[1][0], "10 dB or more: the hit stays");
    p.rises[1][0] = 12.0;
    assert!(!p.marks()[1][0]);
    // the guard is per mic: an unguarded mic keeps the version 3 rule at the same rise
    p.guarded[1] = false;
    assert!(p.marks()[1][0], "version 3 on mic b: no guard");
    // an unguarded mic's hit is still beaten by a guarded mic's kept hit
    let mut p = Pairs::new(
        &["a", "b", "c"],
        &[
            &[(1.0, 0.0), (2.0, 0.0), (3.0, 0.0)],
            &[(1.002, -9.0), (2.5, 0.0), (3.5, 0.0)],
            &[(1.010, -14.0), (2.7, 0.0), (3.7, 0.0)],
        ],
    );
    p.rises[1][0] = 30.0;
    assert_eq!(
        (p.marks()[1][0], p.marks()[2][0]),
        (false, true),
        "b's attack keeps it and it still wins against c"
    );
}

#[test]
fn the_attack_rise_is_the_peak_over_what_came_before_in_both_bands() {
    // a 3 kHz stroke: the high-pass passes it, so both bands measure the planted 12 dB
    let n = SR as usize;
    let at = |s: f64| (s * f64::from(SR)) as usize;
    let mut x = vec![0.0f32; n];
    let amp = 0.2f32;
    tone(&mut x, SR, 0.0, 0.5, amp * 10f32.powf((-12.0 / 20.0) as f32), 3000.0);
    burst(&mut x, at(0.5), amp, 3000.0);
    let hp = zero_phase(&x, SR, &[(FilterKind::HighPass, PAIR_HP_HZ)]);
    let rise = stroke_rise_db(&x, &hp, SR, 0.5);
    assert!((rise - 12.0).abs() < 1.5, "a 12 dB attack measures {rise}");
    // silence before: the -180 dB floor gives a huge rise
    let mut y = vec![0.0f32; n];
    burst(&mut y, at(0.5), amp, 3000.0);
    let hp = zero_phase(&y, SR, &[(FilterKind::HighPass, PAIR_HP_HZ)]);
    assert!(stroke_rise_db(&y, &hp, SR, 0.5) > 100.0);
    // an empty window is -180: the rise of a hit at the very start is the peak over the floor
    let mut z = vec![0.0f32; n];
    burst(&mut z, at(0.004), amp, 3000.0);
    let hp = zero_phase(&z, SR, &[(FilterKind::HighPass, PAIR_HP_HZ)]);
    assert!(stroke_rise_db(&z, &hp, SR, 0.004) > 100.0);
}

// ------------------------------------------------------------- version 5: the guarded shared kick (#362)

/// A floor tom next to the kick: the kick and tom 3 hear each other at 0.3 of the amplitude (a shared pair, the
/// gain sum -21 dB over `shared_sum_db` -26), the other toms at 0.05 (sums at -52 dB, no merge). The kick plays
/// from 1.4 s, tom 3 from 1.1 s, so every kick hit lists a bleed hit on tom 3 (+1 ms) and tom 3's own hits stay
/// far from the kick's.
fn shared_kick_kit() -> (Kit, Vec<Vec<(f64, f32)>>) {
    let plays = vec![
        part(0.5, 1.5, 12),
        part(0.8, 1.5, 12),
        part(1.1, 1.5, 12),
        Vec::new(),
        part(1.4, 1.5, 12),
    ];
    let leak = |d: usize, m: usize| match (d, m) {
        (4, 0..=2) | (0..=2, 4) => 0.3,
        _ => 0.05,
    };
    (
        kit_f(&plays, &leak, &[1.0; 5], 0.009, &[140.0, 110.0, 85.0, 200.0, 55.0]),
        plays,
    )
}

/// The indices of mic `m`'s hits that are its own drum's strokes (not the +1 ms leaks).
fn own_hits(k: &Kit, plays: &[Vec<(f64, f32)>], d: usize, m: usize) -> Vec<usize> {
    let at: Vec<i64> = plays[d].iter().map(|&(t, _)| (t * SR as f64) as i64).collect();
    (0..k.hits[m].len()).filter(|&j| at.contains(&k.hits[m][j])).collect()
}

fn guarded(p_rise: f64) -> TomBleedParams {
    TomBleedParams {
        kick_shared_rise_db: p_rise,
        ..TomBleedParams::default()
    }
}

#[test]
fn a_shared_kick_source_marks_the_kick_bleed_on_the_floor_tom() {
    let (k, plays) = shared_kick_kit();
    let toms = mics(&k, &["tom1", "tom2", "tom3"]);
    let src = sources(&k, &[(4, SourceKind::Kick)]);
    // the guard off: the synthetic bursts all have a sharp attack
    let p = guarded(1e9);
    let r = tom_bleed_with(&toms, &src, SR, &p).unwrap();
    assert_eq!(r.sources[2].unused, Some(Unused::Shared), "the kick pair is shared");
    assert!(r.off.is_none(), "rule CD is on: {:?}", r.off);
    let kick_bleed = bleed_of(&k, &plays[4], 2);
    assert!(kick_bleed.len() >= 8, "the listed kick bleed on tom 3");
    assert!(
        kick_bleed.iter().all(|&j| r.bleed[2][j] && r.shared_kick_bleed[2][j]),
        "the kick bleed on tom 3 is marked"
    );
    let own = own_hits(&k, &plays, 2, 2);
    assert!(own.len() >= 8);
    assert!(
        own.iter().all(|&j| !r.bleed[2][j] && !r.shared_kick_bleed[2][j]),
        "tom 3's own strokes stay"
    );
    // the names say which rule marked each hit (#96): the guarded kick is named, and `bleed` is the OR of the names
    assert!(
        kick_bleed.iter().all(|&j| r.by[2][j].contains(&"v5-kick")),
        "the kick bleed names v5-kick"
    );
    assert!(
        own.iter().all(|&j| r.by[2][j].is_empty()),
        "tom 3's own strokes name no rule"
    );
    for (m, hits) in r.bleed.iter().enumerate() {
        for (j, &b) in hits.iter().enumerate() {
            assert_eq!(b, !r.by[m][j].is_empty(), "mic {m} hit {j}: bleed and by agree");
        }
    }
}

#[test]
fn the_shared_kick_keeps_a_hit_with_an_own_2k_attack() {
    let (mut k, plays) = shared_kick_kit();
    // the default guard keeps a hit with an own 2 kHz attack (a stick click); give every tom mic's
    // hit one, so the kick bleed on the tom mics is not marked
    for m in 0..3 {
        for &h in &k.hits[m] {
            burst(&mut k.x[m], h as usize, 0.02, 4000.0);
        }
    }
    let toms = mics(&k, &["tom1", "tom2", "tom3"]);
    let src = sources(&k, &[(4, SourceKind::Kick)]);
    let r = tom_bleed_with(&toms, &src, SR, &TomBleedParams::default()).unwrap();
    // the guarded shared kick marks nothing; the pair rule's own marks are not this test's concern
    assert!(r.shared_kick_bleed.iter().all(|m| m.iter().all(|&b| !b)));
    let _ = plays;
}

#[test]
fn shared_kick_marks_survive_rule_cd_off_for_fewer_than_two_played() {
    let (k, plays) = shared_kick_kit();
    // every tom mic the same drum: one group, rule CD off with all three mics played
    let toms = mics(&k, &["t", "t", "t"]);
    let src = sources(&k, &[(4, SourceKind::Kick)]);
    let p = guarded(1e9);
    let r = tom_bleed_with(&toms, &src, SR, &p).unwrap();
    match &r.off {
        Some(Off::FewerThanTwoPlayed { played }) => assert_eq!(played.len(), 3, "{off:?}", off = r.off),
        other => panic!("expected fewer than two played, got {other:?}"),
    }
    assert!(
        r.bleed.iter().all(|m| m.iter().all(|&b| !b)),
        "no marks in bleed while rule CD is off"
    );
    let kick_bleed = bleed_of(&k, &plays[4], 2);
    assert!(
        kick_bleed.iter().all(|&j| r.shared_kick_bleed[2][j]),
        "the guarded marks are there for the caller"
    );
}

#[test]
fn a_digitally_silent_mic_takes_no_shared_kick_marks() {
    let (k, plays) = shared_kick_kit();
    // tom 3 gated to digital silence at the kick's hits
    let mut gated = k.x[2].clone();
    for &(t, _) in &plays[4] {
        let a = (t * SR as f64) as usize;
        gated[a - 2400..a + 2400].iter_mut().for_each(|v| *v = 0.0);
    }
    let mut toms = mics(&k, &["tom1", "tom2", "tom3"]);
    toms[2] = TomMic {
        drum: "tom3".into(),
        signal: &gated,
        hits: k.hits[2].clone(),
    };
    let src = sources(&k, &[(4, SourceKind::Kick)]);
    let p = guarded(1e9);
    let r = tom_bleed_with(&toms, &src, SR, &p).unwrap();
    assert_eq!(r.sources[2].unused, Some(Unused::Silent), "{:?}", r.sources[2]);
    assert!(r.shared_kick_bleed[2].iter().all(|&b| !b));
}
