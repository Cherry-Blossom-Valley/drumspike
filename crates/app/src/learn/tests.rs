use super::*;

fn learn(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn apply_mark_stores_a_mark_with_defaults_and_never_switches_on() {
    // P4-6c decision: apply_mark never turns the filter on (the first Shift+K does: first_not_turns_on)
    let mut l = Map::new();
    let r = apply_mark(&mut l, "not", 1.0, 1.0, &[1.0]).unwrap();
    assert_eq!((r.id.as_deref(), r.added), (Some("m1"), 1));
    assert_eq!(l["on"], false, "a not mark does not switch the filter on");
    let r = apply_mark(&mut l, "this", 2.0, 3.0, &[2.5, 2.25, 2.5]).unwrap();
    assert_eq!((r.id.as_deref(), r.added), (Some("m2"), 2), "a time twice counts once");
    assert_eq!(l["on"], false, "a this mark does not switch the filter on either");
    assert_eq!(l["marks"][1]["at"], json!([2.25, 2.5]));
    assert_eq!(
        (l["method"].clone(), l["threshold"].clone()),
        (json!("patch-2"), json!(0.5))
    );
    assert_eq!(counts(&l), (2, 1));
    assert!(apply_mark(&mut l, "x", 0.0, 0.0, &[1.0]).is_err());
}

#[test]
fn the_last_kind_wins_and_an_emptied_mark_goes() {
    let mut l = Map::new();
    apply_mark(&mut l, "this", 1.0, 1.0, &[1.0]).unwrap();
    apply_mark(&mut l, "this", 2.0, 2.0, &[2.0]).unwrap();
    let r = apply_mark(&mut l, "not", 1.0, 1.0, &[1.001]).unwrap();
    assert_eq!((r.added, r.flipped, r.skipped), (0, 1, 0));
    assert_eq!(counts(&l), (1, 1));
    let ids: Vec<&str> = l["marks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["m2", "m3"], "m1 lost its only example");
    let r = apply_mark(&mut l, "not", 1.0, 1.0, &[1.0]).unwrap();
    assert_eq!((r.id, r.skipped), (None, 1), "the same kind again changes nothing");
}

#[test]
fn a_kind_holds_at_most_64_examples_and_a_refused_mark_changes_nothing() {
    let mut l = Map::new();
    let ats: Vec<f64> = (0..64).map(f64::from).collect();
    apply_mark(&mut l, "this", 0.0, 64.0, &ats).unwrap();
    let before = l.clone();
    let e = apply_mark(&mut l, "this", 100.0, 100.0, &[100.0]).unwrap_err();
    assert_eq!(e.code, 400);
    assert!(e.message.contains("Forget"), "{}", e.message);
    assert_eq!(l, before);
    apply_mark(&mut l, "not", 5.0, 5.0, &[5.0]).unwrap(); // the other kind has room (a flip)
    assert_eq!(counts(&l), (63, 1));
}

#[test]
fn remove_mark_takes_a_mark_or_one_example_and_checks_the_id() {
    let mut l = Map::new();
    apply_mark(&mut l, "this", 1.0, 2.0, &[1.0, 2.0]).unwrap();
    assert_eq!(remove_mark(&mut l, "m1", Some(2.0)).unwrap(), 1);
    assert_eq!(l["marks"][0]["at"], json!([1.0]));
    assert_eq!(remove_mark(&mut l, "m1", Some(9.0)).unwrap_err().code, 400);
    assert_eq!(remove_mark(&mut l, "m9", None).unwrap_err().code, 400);
    assert_eq!(remove_mark(&mut l, "m1", Some(1.0)).unwrap(), 1);
    assert!(marks_of(&l).is_empty(), "the last example takes its mark");
    apply_mark(&mut l, "not", 1.0, 2.0, &[1.0, 2.0]).unwrap();
    assert_eq!(remove_mark(&mut l, "m1", None).unwrap(), 2);
}

#[test]
fn flip_mark_changes_the_kind_and_respects_the_limit() {
    let mut l = Map::new();
    apply_mark(&mut l, "this", 1.0, 1.0, &[1.0]).unwrap();
    assert_eq!(flip_mark(&mut l, "m1").unwrap(), "not");
    assert_eq!(counts(&l), (0, 1));
    assert_eq!(flip_mark(&mut l, "m1").unwrap(), "this");
    assert_eq!(flip_mark(&mut l, "zz").unwrap_err().code, 400);
    let ats: Vec<f64> = (10..74).map(f64::from).collect();
    apply_mark(&mut l, "not", 10.0, 74.0, &ats).unwrap();
    assert_eq!(flip_mark(&mut l, "m2").unwrap_err().code, 400, "65 this examples");
}

#[test]
fn check_learn_accepts_what_apply_mark_writes_and_refuses_the_rest() {
    let mut l = Map::new();
    apply_mark(&mut l, "this", 1.0, 1.0, &[1.0]).unwrap();
    check_learn(&l).unwrap();
    let mut ok = l.clone();
    ok.insert("future".into(), json!(1));
    check_learn(&ok).unwrap();
    for bad in [
        json!({"marks": 3}),
        json!({"on": "yes", "marks": []}),
        json!({"threshold": 2, "marks": []}),
        json!({"marks": [{"id": "m1", "kind": "maybe", "t0": 0, "t1": 0, "at": []}]}),
        json!({"marks": [{"id": "m1", "kind": "this", "t0": 0, "t1": 0, "at": ["x"]}]}),
        json!({"marks": [{"kind": "this", "t0": 0, "t1": 0, "at": []}]}),
        json!({"marks": [{"id": "m1", "kind": "this", "t0": 0, "t1": 0, "at": [1]},
                         {"id": "m1", "kind": "not", "t0": 0, "t1": 0, "at": [2]}]}),
    ] {
        assert!(check_learn(&learn(bad.clone())).is_err(), "{bad}");
    }
}

#[test]
fn counts_and_learn_of_read_the_marks() {
    assert_eq!(counts(&Map::new()), (0, 0));
    let l =
        learn(json!({"marks": [{"kind": "this", "at": [1, 2]}, {"kind": "not", "at": [3]}, {"kind": "x", "at": [4]}]}));
    assert_eq!(counts(&l), (2, 1));
    let extra = learn(json!({"learn": {"marks": []}, "other": 1}));
    assert_eq!(learn_of(&extra), Some(learn(json!({"marks": []}))));
    assert_eq!(learn_of(&learn(json!({"learn": 3}))), None);
}

#[test]
fn the_candidate_rules_pick_the_strongest_and_the_nearest() {
    let c = vec![
        json!([1.0, 0, 0.3, 40]),
        json!([1.2, 0, 0.9, 5]),
        json!([1.4, 1, 0.9, 5]),
        json!([3.0, 0, 0.99, 1]),
    ];
    assert_eq!(
        strongest_candidate(&c, 0.9, 1.5),
        Some(1.2),
        "a tie goes to the earlier one"
    );
    assert_eq!(strongest_candidate(&c, 1.25, 1.5), Some(1.4));
    assert_eq!(strongest_candidate(&c, 1.5, 2.0), None);
    assert_eq!(nearest_candidate(&c, 1.19), Some(1.2));
    assert_eq!(nearest_candidate(&c, 2.0), None);
}

#[test]
fn hits_of_a_range_skip_cut_hits_and_the_shift_comes_off() {
    let ev = vec![
        json!({"t": 1.0, "v": 90}),
        json!({"t": 1.5, "v": 90, "cut": true}),
        json!({"t": 2.0, "v": 90}),
    ];
    assert_eq!(active_hits(&ev, 0.9, 2.5, 0.0).unwrap(), vec![1.0, 2.0]);
    assert_eq!(
        active_hits(&ev, 1.0, 1.0, 0.25).unwrap(),
        vec![0.75],
        "the bounds count"
    );
    assert!(active_hits(&ev, 3.0, 4.0, 0.0).unwrap().is_empty());
    // P4-14: nor does a hit of the tom bleed rule count as active
    let bl = vec![json!({"t": 1.0, "v": 90, "bleed": true}), json!({"t": 2.0, "v": 90})];
    assert_eq!(active_hits(&bl, 0.0, 3.0, 0.0).unwrap(), vec![2.0]);
    assert_eq!(hit_at(&ev, 1.503, 0.0).unwrap(), Some(1.5), "a cut hit can be marked");
    assert_eq!(hit_at(&ev, 1.2, 0.0).unwrap(), None);
    assert!(active_hits(&[json!({"v": 1})], 0.0, 1.0, 0.0).is_err());
}

#[test]
fn rise_example_takes_the_strongest_of_two_rises() {
    let sr = 48_000u32;
    let mut seed = 99u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    };
    let mut x: Vec<f32> = (0..sr as usize * 2)
        .map(|k| 1e-3 * (2.0 * std::f32::consts::PI * 200.0 * k as f32 / sr as f32).sin())
        .collect();
    // a soft burst with a slow attack (8 ms) at 0.6 s, a hard one (instant) at 1.2 s: the steepest rise wins
    for (at, ramp) in [(0.6f32, 0.008f32), (1.2, 0.0)] {
        for (k, v) in x.iter_mut().enumerate().skip((at * sr as f32) as usize).take(4800) {
            let t = (k as f32 - at * sr as f32) / sr as f32;
            *v += 0.5 * (-t / 0.04).exp() * (if ramp > 0.0 { (t / ramp).min(1.0) } else { 1.0 }) * noise();
        }
    }
    let t = rise_example(&x, sr, 0.3, 1.8).expect("two attacks");
    assert!((1.199..1.21).contains(&t), "the strong one, not the first: {t}");
    let t = rise_example(&x, sr, 0.3, 0.9).expect("the weak one alone");
    assert!((0.599..0.61).contains(&t), "{t}");
}

#[test]
fn a_mark_that_changes_kinds_counts_against_the_cap_of_the_new_kind() {
    let mut l = Map::new();
    let nots: Vec<f64> = (0..64).map(f64::from).collect();
    apply_mark(&mut l, "not", 0.0, 64.0, &nots).unwrap();
    apply_mark(&mut l, "this", 100.0, 100.0, &[100.0]).unwrap();
    let before = l.clone();
    // the hit at 100 changes from this to not: 65 not examples
    let e = apply_mark(&mut l, "not", 100.0, 100.0, &[100.0]).unwrap_err();
    assert_eq!(e.code, 400, "{e:?}");
    assert_eq!(l, before, "nothing changed");
}

#[test]
fn rise_example_finds_an_attack_of_6_db_and_nothing_in_a_steady_floor() {
    let sr = 48_000u32;
    // a steady hum at -60 dB, and a noise burst at 0 dB with a 40 ms decay at 1.0 s
    let mut seed = 12345u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    };
    let mut x: Vec<f32> = (0..sr as usize * 2)
        .map(|k| 1e-3 * (2.0 * std::f32::consts::PI * 200.0 * k as f32 / sr as f32).sin())
        .collect();
    for (k, v) in x.iter_mut().enumerate().skip(sr as usize).take(4800) {
        let t = (k - sr as usize) as f32 / sr as f32;
        *v += 0.5 * (-t / 0.04).exp() * noise();
    }
    let t = rise_example(&x, sr, 0.5, 1.5).expect("an attack at 1.0 s");
    assert!((0.999..1.01).contains(&t), "{t}");
    assert_eq!(rise_example(&x, sr, 0.1, 0.9), None, "a range before it");
    assert_eq!(rise_example(&x, sr, 1.6, 1.9), None, "a range after it");
    assert_eq!(rise_example(&vec![0.0; sr as usize], sr, 0.0, 0.9), None, "silence");
}

/// #81: a soft attack (about 4 dB over a hum) is no [`rise_example`], so the K-add of an empty range took the range
/// start, up to a range length before the hit. [`weak_rise`] finds it; a steady floor and silence keep `None` (the
/// range start). Planted fault: the old path (`rise_example` only) answers `None` for the soft attack.
#[test]
fn weak_rise_finds_a_soft_attack_and_nothing_in_a_steady_floor() {
    let sr = 48_000u32;
    let mut seed = 777u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    };
    let mut x: Vec<f32> = (0..sr as usize * 2)
        .map(|k| 1e-2 * (2.0 * std::f32::consts::PI * 200.0 * k as f32 / sr as f32).sin())
        .collect();
    for (k, v) in x.iter_mut().enumerate().skip(sr as usize).take(4800) {
        let t = (k - sr as usize) as f32 / sr as f32;
        *v += 1.2e-2 * (-t / 0.04).exp() * noise();
    }
    let (t0, t1) = (0.98, 1.02);
    assert_eq!(
        rise_example(&x, sr, t0, t1),
        None,
        "under 6 dB: the old path adds at the range start"
    );
    let t = weak_rise(&x, sr, t0, t1).expect("the soft attack at 1.0 s");
    assert!((0.999..1.005).contains(&t), "{t}");
    assert_eq!(weak_rise(&x, sr, 0.3, 0.9), None, "a steady hum");
    assert_eq!(weak_rise(&vec![0.0; sr as usize], sr, 0.0, 0.9), None, "silence");
}

/// #81 (Session H t3): in a K range the steepest soft rise can be the bleed of a kick that is louder in its own mic; the
/// tom's own soft attack is less steep. [`weak_rise_clear`] skips a rise with a louder hit of another close mic within
/// [`BLEED_REACH`] and takes the tom attack; a quieter hit there does not rule the rise out, and with every rise ruled
/// out the answer is `None` (the range start). Planted fault: [`weak_rise`] alone takes the kick bleed.
#[test]
fn weak_rise_clear_skips_a_rise_under_a_louder_hit_of_another_close_mic() {
    let sr = 48_000u32;
    let mut seed = 4242u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    };
    let mut x: Vec<f32> = (0..sr as usize * 2)
        .map(|k| 2e-3 * (2.0 * std::f32::consts::PI * 200.0 * k as f32 / sr as f32).sin())
        .collect();
    // kick bleed at 1.00 s (the steepest rise), the tom's own soft attack at 1.04 s (less steep)
    for (at, amp, decay) in [(1.00, 8e-2f32, 0.01f32), (1.04, 2e-2, 0.02)] {
        let s = (at * f64::from(sr)) as usize;
        for (i, v) in x.iter_mut().skip(s).take(4800).enumerate() {
            let t = i as f32 / sr as f32;
            *v += amp * (-t / decay).exp() * noise();
        }
    }
    let (t0, t1) = (0.98, 1.08);
    let rises = rises_of(&x, sr, t0, t1, WEAK_RISE_DB);
    let bleed = weak_rise(&x, sr, t0, t1).expect("a rise");
    assert!(
        (0.999..1.005).contains(&bleed),
        "the steepest rise is the kick bleed: {bleed} of {rises:?}"
    );
    let kick = 10.0 * peak_after(&x, sr, bleed);
    let t = weak_rise_clear(&x, sr, t0, t1, &[(0.999, kick)]).unwrap_or_else(|| panic!("the tom attack: {rises:?}"));
    assert!((1.039..1.045).contains(&t), "{t} of {rises:?}");
    assert_eq!(
        weak_rise_clear(&x, sr, t0, t1, &[(0.999, 1e-4)]),
        Some(bleed),
        "a quieter hit is no source"
    );
    let louder: Vec<(f64, f32)> = (0..=5).map(|i| (0.98 + 0.02 * f64::from(i), 1.0)).collect();
    assert_eq!(weak_rise_clear(&x, sr, t0, t1, &louder), None, "every rise is bleed");
}

/// #81 (the Gate rerun on 29a542f: every K-add of Session H t3 at its range start): refine gives its input back when it
/// finds no attack, for example in a floor under 1e-4 (its 25 % rule window is "silent"), with the floor's dB as the
/// rise. That piece start is no rise: [`weak_rise`] answers `None`, the K-add takes the [`range_centre`], and
/// [`rise_log`] says `no attack` for each piece. Planted fault: [`rises_of`] without the piece start check answers the
/// range start.
#[test]
fn a_piece_start_from_refine_is_no_rise_and_the_log_says_why() {
    let sr = 48_000u32;
    let mut seed = 99u32;
    let x: Vec<f32> = (0..sr as usize)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            2e-5 * ((seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0)
        })
        .collect();
    let (t0, t1) = (0.5, 0.54);
    let raw = pieces_of(&x, sr, t0, t1);
    assert!(
        raw.iter()
            .any(|&(a, t, r)| t == a && r.is_some_and(|r| r >= WEAK_RISE_DB)),
        "the floor gives a piece start with dB: {raw:?}"
    );
    assert_eq!(weak_rise(&x, sr, t0, t1), None, "{raw:?}");
    assert_eq!(range_centre(t0, t1), 0.52);
    let log = rise_log(&x, sr, t0, t1, &[]);
    assert_eq!(log.len(), raw.len());
    assert!(log.iter().all(|r| r["why"] == "no attack"), "{log:?}");
}

/// #81: [`rise_log`] names the taken rise and the bleed rise with the louder hit (the scene of
/// `weak_rise_clear_skips_a_rise_under_a_louder_hit_of_another_close_mic`).
#[test]
fn the_rise_log_names_the_taken_rise_and_the_bleed() {
    let sr = 48_000u32;
    let mut seed = 4242u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    };
    let mut x: Vec<f32> = (0..sr as usize * 2)
        .map(|k| 2e-3 * (2.0 * std::f32::consts::PI * 200.0 * k as f32 / sr as f32).sin())
        .collect();
    for (at, amp, decay) in [(1.00, 8e-2f32, 0.01f32), (1.04, 2e-2, 0.02)] {
        let s = (at * f64::from(sr)) as usize;
        for (i, v) in x.iter_mut().skip(s).take(4800).enumerate() {
            let t = i as f32 / sr as f32;
            *v += amp * (-t / decay).exp() * noise();
        }
    }
    let (t0, t1) = (0.98, 1.08);
    let bleed = weak_rise(&x, sr, t0, t1).expect("a rise");
    let kick = 10.0 * peak_after(&x, sr, bleed);
    let taken = weak_rise_clear(&x, sr, t0, t1, &[(0.999, kick)]).expect("the tom attack");
    let log = rise_log(&x, sr, t0, t1, &[(0.999, kick)]);
    let why = |t: f64| {
        log.iter()
            .find(|r| r["t"].as_f64() == Some(t) && r["why"] != "same time")
            .map(|r| r["why"].clone())
    };
    assert_eq!(why(taken), Some(json!("taken")), "{log:?}");
    assert_eq!(why(bleed), Some(json!("bleed")), "{log:?}");
    let b = log.iter().find(|r| r["why"] == "bleed").unwrap();
    assert_eq!(b["bleed_t"], json!(0.999));
}

#[test]
fn the_first_not_example_turns_the_filter_on_with_the_threshold_following() {
    // K: never
    let mut l = Map::new();
    apply_mark(&mut l, "this", 1.0, 1.0, &[1.0]).unwrap();
    assert!(!first_not_turns_on(&mut l, "this", 0));
    assert_eq!((l["on"].clone(), l["auto"].clone()), (json!(false), json!(true)));
    // the first Shift+K: on, auto
    let before = counts(&l).1;
    apply_mark(&mut l, "not", 2.0, 2.0, &[2.0]).unwrap();
    assert!(first_not_turns_on(&mut l, "not", before));
    assert_eq!((l["on"].clone(), l["auto"].clone()), (json!(true), json!(true)));
    // the engineer turns it off: a second Shift+K leaves it off
    l.insert("on".into(), json!(false));
    let before = counts(&l).1;
    apply_mark(&mut l, "not", 3.0, 3.0, &[3.0]).unwrap();
    assert!(
        !first_not_turns_on(&mut l, "not", before),
        "only the first not example decides"
    );
    assert_eq!(l["on"], false);
    // planted fault: a Shift+K that marked nothing (no not example now) turns nothing on
    let mut empty = Map::new();
    assert!(!first_not_turns_on(&mut empty, "not", 0));
    assert!(!empty.contains_key("on"));
}

#[test]
fn a_0_4_0_learn_object_with_no_not_example_loads_with_the_filter_off() {
    let mut l = Map::new();
    apply_mark(&mut l, "this", 1.0, 1.0, &[1.0]).unwrap();
    l.insert("on".into(), json!(true));
    l.insert("auto".into(), json!(false));
    let mut extra = Map::new();
    extra.insert("learn".into(), Value::Object(l.clone()));
    assert!(migrate_learn(&mut extra));
    assert_eq!(extra["learn"]["on"], false);
    assert!(!migrate_learn(&mut extra), "once");
    // planted fault: a learn object with a not example keeps the filter on
    apply_mark(&mut l, "not", 2.0, 2.0, &[2.0]).unwrap();
    let mut extra = Map::new();
    extra.insert("learn".into(), Value::Object(l));
    assert!(!migrate_learn(&mut extra));
    assert_eq!(extra["learn"]["on"], true);
    assert!(!migrate_learn(&mut Map::new()));
}
