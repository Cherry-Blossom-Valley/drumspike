use super::*;

#[test]
fn cut_of_event_cut_and_the_marks_follow_the_level_and_spare_edited_hits() {
    assert_eq!(cut_of(None), None);
    assert_eq!(cut_of(json!({"vel_cut_db": -30}).as_object()), Some(-30.0));
    assert_eq!(cut_of(json!({"vel_cut_db": null}).as_object()), None);
    // a hit of -6 dBFS at 0.1 s and one of -40 dBFS at 0.5 s
    let sr = 48_000;
    let mut x = vec![0.0f32; sr as usize];
    x[4_800] = 10f32.powf(-6.0 / 20.0);
    x[24_000] = 10f32.powf(-40.0 / 20.0);
    let mut ev = vec![Event::new(0.1, 100), Event::new(0.5, 30), Event::new(0.5, 30)];
    ev[2].added = Some(true);
    mark_cut_events(&mut ev, &x, sr, Some(-30.0));
    assert_eq!(
        ev.iter().map(event_cut).collect::<Vec<_>>(),
        [false, true, false],
        "the added hit is spared"
    );
    mark_cut_events(&mut ev, &x, sr, None);
    assert!(ev.iter().all(|e| !event_cut(e)), "off: every mark goes");
    // P4-14: a hit of the tom bleed rule is out of the output too, and the loudness cut leaves its mark alone
    ev[0].extra.insert("bleed".into(), Value::Bool(true));
    assert!(event_cut(&ev[0]));
    mark_cut_events(&mut ev, &x, sr, Some(-30.0));
    assert_eq!(ev[0].extra.get("bleed"), Some(&Value::Bool(true)));
    // P4-6c: a hit the Learn filter takes out is out of the output too
    ev[1].extra.insert("filtered".into(), Value::Bool(true));
    mark_cut_events(&mut ev, &x, sr, None);
    assert!(event_cut(&ev[1]) && ev[1].extra.get("filtered") == Some(&Value::Bool(true)));
    ev[1].extra.insert("filtered".into(), Value::Bool(false));
    assert!(
        !event_cut(&ev[1]),
        "planted fault: filtered false keeps the hit in the output"
    );
    // a hit restored from the bleed mark (`edited: true`) is no hand edit: the loudness cut still takes it
    let mut quiet = Event::new(0.5, 30);
    quiet.edited = Some(true);
    let mut one = [quiet];
    mark_cut_events(&mut one, &x, sr, Some(-30.0));
    assert!(event_cut(&one[0]), "a restored quiet hit is cut");
    let mut vals = [json!({"t": 0.5, "edited": true})];
    mark_cut_values(&mut vals, &x, sr, Some(-30.0)).unwrap();
    assert_eq!(vals[0]["cut"], true);
    let mut vals = vec![
        json!({"t": 0.1}),
        json!({"t": 0.5}),
        json!({"t": 0.5, "orig": 0.45}),
        json!({"t": 0.5, "cut": true}),
    ];
    mark_cut_values(&mut vals, &x, sr, Some(-30.0)).unwrap();
    assert_eq!(
        vals.iter().map(|e| e.get("cut").is_some()).collect::<Vec<_>>(),
        [false, true, false, true]
    );
    mark_cut_values(&mut vals, &x, sr, None).unwrap();
    assert!(vals.iter().all(|e| e.get("cut").is_none()));
    assert_eq!(mark_cut_values(&mut [json!(1)], &x, sr, None).unwrap_err().code, 400);
    assert_eq!(
        mark_cut_values(&mut [json!({"v": 1})], &x, sr, None).unwrap_err().code,
        400
    );
}

#[test]
fn begin_write_holds_the_store_lock_and_refuses_while_another_writes() {
    let svc = Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    let g = svc.begin_write("s").unwrap();
    assert!(svc.inner.jobs.storing("s"), "held while the write runs");
    assert_eq!(
        svc.begin_write("s").map(|_| ()).unwrap_err().message,
        Service::busy().message
    );
    drop(g);
    assert!(!svc.inner.jobs.storing("s"));
    svc.inner.jobs.queue("s", &["a".into()]);
    let e = svc.begin_write("s").map(|_| ()).unwrap_err();
    assert!(e.code == 409 && e.message.contains("analysis"), "{e:?}");
    assert!(!svc.inner.jobs.storing("s"), "a refused write leaves no lock");
}

#[test]
fn role_class_indices_follow_the_role_filter() {
    assert_eq!(role_class_indices("kick"), vec![0]);
    assert_eq!(role_class_indices("snare"), vec![1]);
    assert_eq!(role_class_indices("tom"), vec![2]);
    assert_eq!(role_class_indices("hat"), vec![3]);
    assert_eq!(role_class_indices("cymbal"), vec![4]);
    assert_eq!(role_class_indices("other"), vec![0, 1, 2, 3, 4]);
    assert_eq!(role_class_indices("bass"), vec![0, 1, 2, 3, 4]);
}

#[test]
fn largest_gap_is_the_widest_inner_run_without_a_candidate() {
    assert_eq!(
        largest_gap(&[10, 12, 40, 41, 90, 95]).unwrap(),
        json!({"from": 42, "to": 89, "mid": 65})
    );
    assert_eq!(largest_gap(&[5, 5, 6]), None, "no inner gap");
    assert_eq!(largest_gap(&[]), None);
    assert_eq!(largest_gap(&[3, 1]).unwrap(), json!({"from": 2, "to": 2, "mid": 2}));
    // a tie keeps the first (the stricter) gap
    assert_eq!(largest_gap(&[0, 10, 20]).unwrap()["from"], 1);
}

#[test]
fn dispatch_ignores_other_commands_and_checks_arguments() {
    let svc = Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    assert!(dispatch(&svc, "project_get", &json!({})).is_none());
    for c in HISTOGRAM_COMMANDS {
        let e = dispatch(&svc, c, &json!({})).unwrap().unwrap_err();
        assert_eq!(e.code, 400, "{c}: {e:?}");
    }
    let e = dispatch(
        &svc,
        "track_velocity_set",
        &json!({"sid": "x", "name": "n", "lo_db": -40}),
    )
    .unwrap()
    .unwrap_err();
    assert!(e.message.contains("hi_db"), "{e:?}");
    let e = dispatch(&svc, "track_repick", &json!({"sid": "nope", "name": "n"}))
        .unwrap()
        .unwrap_err();
    assert_eq!(e.code, 404, "{e:?}");
}
