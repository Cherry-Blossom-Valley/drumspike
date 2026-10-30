use super::*;

/// The meter messages a playhead thread sends in one second when it looks every `look`.
fn meters_in_one_second(look: Duration) -> usize {
    let t0 = Instant::now();
    let (mut due, mut sent, mut now) = (None::<Instant>, 0, t0);
    while now < t0 + Duration::from_secs(1) {
        if due.is_none_or(|d| now >= d) {
            due = Some(next_meter_due(due, now));
            sent += 1;
        }
        now += look;
    }
    sent
}

#[test]
fn late_looks_keep_the_meter_rate_and_a_long_stall_makes_no_burst() {
    // on time (20 ms looks) and late (29 ms: about the slack of CI run 38376400763, where the
    // old "a period after the send" rule gave 17 Hz) both give 25 messages per second
    assert_eq!(meters_in_one_second(Duration::from_millis(20)), 25);
    assert_eq!(meters_in_one_second(Duration::from_millis(29)), 25);
    // a stall of 300 ms gives one message, then the rate goes on from there: no catch-up burst
    let t0 = Instant::now();
    let due = next_meter_due(Some(t0), t0 + Duration::from_millis(300));
    assert_eq!(due, t0 + Duration::from_millis(300) + METER_PERIOD);
    // the first message sets the first due time
    assert_eq!(next_meter_due(None, t0), t0 + METER_PERIOD);
}

#[test]
fn hits_are_rounded_sorted_and_deduplicated() {
    // 0.5 s at 48 kHz is sample 24000; two events on the same sample count once; a
    // negative time and NaN are dropped; an event past n_samples is dropped
    let h = hits_from_times([1.0, 0.5, 0.5, -0.1, f64::NAN, 9.0].into_iter(), 48_000, Some(96_000));
    assert_eq!(h, vec![24_000, 48_000]);
    let h = hits_from_times([9.0].into_iter(), 48_000, None);
    assert_eq!(h, vec![432_000], "without n_samples nothing is cut");
}

#[test]
fn times_of_reads_numbers_and_objects() {
    assert_eq!(
        times_of(&json!([0.5, {"t": 1.5, "v": 90}, {"v": 3}, "x", {"t": 2.0, "cut": true}])),
        vec![0.5, 1.5]
    );
    // P4-14: a hit of the tom bleed rule gets no click either
    assert_eq!(
        times_of(&json!([{"t": 1.0, "bleed": true}, {"t": 2.0, "bleed": false}])),
        vec![2.0]
    );
    assert!(times_of(&json!(null)).is_empty());
}

#[test]
fn the_command_list_has_no_duplicates() {
    let mut v = TRANSPORT_COMMANDS.to_vec();
    v.sort_unstable();
    v.dedup();
    assert_eq!(v.len(), TRANSPORT_COMMANDS.len());
}

#[test]
fn clicks_use_the_page_pitch_or_the_role_default() {
    let mut c = ClickState {
        enabled: true,
        volume: 0.22,
        pitches: HashMap::new(),
    };
    assert_eq!(c.pitch("kick"), 1000.0);
    assert_eq!(c.pitch("nonsense"), 1500.0);
    c.pitches.insert("kick".into(), 880.0);
    assert_eq!(c.pitch("kick"), 880.0);
}

#[test]
fn the_master_gain_comes_from_the_environment_only_when_valid() {
    // One test owns DRUMSPIKE_MASTER_GAIN: the process environment is shared.
    unsafe {
        std::env::remove_var("DRUMSPIKE_MASTER_GAIN");
    }
    assert_eq!(master_from_env(), 1.0);
    for (v, want) in [("0.0", 0.0), ("0.25", 0.25), ("-1", 1.0), ("x", 1.0), ("inf", 1.0)] {
        unsafe {
            std::env::set_var("DRUMSPIKE_MASTER_GAIN", v);
        }
        assert_eq!(master_from_env(), want, "{v}");
    }
    unsafe {
        std::env::remove_var("DRUMSPIKE_MASTER_GAIN");
    }
}

#[test]
fn volume_and_pan_are_clamped_to_the_fader_range() {
    assert_eq!(clamp_db(0.0), 0.0);
    assert_eq!(clamp_db(-6.5), -6.5);
    assert_eq!(clamp_db(40.0), MAX_DB as f64);
    assert_eq!(clamp_db(-1e9), MIN_DB as f64);
    assert_eq!(clamp_db(-7.3), -7.3, "no f32 rounding in the document");
    assert_eq!(clamp_pan(0.25), 0.25);
    assert_eq!(clamp_pan(-7.0), -1.0);
    assert_eq!(clamp_pan(7.0), 1.0);
}

#[test]
fn the_meter_message_is_flat_and_rounded() {
    let m = MeterSnapshot {
        tracks: vec![[0.123_456, 0.0], [1.5, 0.000_04]],
        master: [0.999_99, 0.25],
    };
    assert_eq!(
        meter_message("7", &m),
        json!({"sid": "7", "meters": [0.1235, 0.0, 1.5, 0.0], "master": [1.0, 0.25]})
    );
}

#[test]
fn the_null_output_resolves_the_choice_against_the_test_list() {
    let host = TransportHost::new();
    let mut st = lock(&host.st);
    st.output = Output::Null { sample_rate: None };
    assert_eq!(output_for(&st), (Output::Null { sample_rate: None }, None));
    st.choice = Some("AirPods".into());
    let (o, note) = output_for(&st);
    assert_eq!(o, Output::Null { sample_rate: None });
    assert!(note.unwrap().contains("AirPods"), "no test list: not connected");
    st.device_list = Some(vec![DeviceKey {
        name: "AirPods".into(),
        rate: 44_100,
        channels: 2,
    }]);
    assert_eq!(output_for(&st).1, None);
    st.probe_override = st.device_list.as_ref().map(|l| l[0].clone());
    assert_eq!(
        output_for(&st).0,
        Output::Null {
            sample_rate: Some(44_100)
        }
    );
    // the watcher of a null output sees the override only
    let k = probe(probe_input(&st)).unwrap();
    assert_eq!((k.name.as_str(), k.rate), ("AirPods", 44_100));
    assert_eq!(
        DeviceKey::of(&DeviceInfo {
            name: "x".into(),
            sample_rate: 1,
            channels: 2,
            buffer_frames: None
        })
        .json(),
        json!({"name": "x", "rate": 1, "channels": 2})
    );
}

#[test]
fn dev_err_maps_argument_errors_to_400_and_the_rest_to_500() {
    assert_eq!(dev_err(TransportError::Arg("x".into())).code, 400);
    assert_eq!(dev_err(TransportError::Device("x".into())).code, 500);
    assert_eq!(dev_err(TransportError::Track("x".into())).code, 500);
    assert_eq!(dev_err(TransportError::NoAnswer("x".into())).code, 500);
}

/// A service with an open (silent) transport on one tiny track, and the same plan as a lost one.
fn service_with_open_plan(tag: &str) -> (Service, Vec<String>) {
    use drumspike_core::io::{WavFormat, WavSpec, write_wav};
    let dir = std::env::temp_dir().join(format!("ds_unit_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.wav");
    let spec = WavSpec {
        sample_rate: 48_000,
        channels: 1,
        format: WavFormat::Float32,
        extensible: false,
        channel_mask: 0,
    };
    write_wav(&path, &spec, &vec![0.0f32; 48_000], None).unwrap();
    let svc = Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    let names = vec!["t".to_string()];
    let plan = Plan {
        sid: "s1".into(),
        names: names.clone(),
        roles: vec!["kick".into()],
        specs: vec![TrackSpec {
            hits: vec![100],
            path,
            mode: mix_mode("average"),
            click_pitch: 60.0,
        }],
        vols: vec![(0.0, 0.0)],
        layers: vec![None],
    };
    let mut st = lock(&svc.host().st);
    st.output = Output::Null { sample_rate: None };
    let open = start(&mut st, plan.clone(), Speed::Normal).unwrap();
    st.open = Some(open);
    st.lost = Some(Lost {
        plan,
        position_s: 0.0,
        playing: false,
        speed: Speed::Normal,
        error: String::new(),
    });
    drop(st);
    (svc, names)
}

#[test]
fn a_layer_refresh_builds_the_plans_once_and_gives_both_plans_the_same_layers() {
    // P4-7a fix 5: the plans are built outside the host lock, once; the lost plan gets the same ones
    let (svc, names) = service_with_open_plan("refresh_once");
    let before = LAYER_PLAN_CALLS.with(std::cell::Cell::get);
    svc.transport_refresh_layers("s1").unwrap();
    assert_eq!(LAYER_PLAN_CALLS.with(std::cell::Cell::get) - before, 1);
    // the lost plan is kept equal to the open one by apply_layers (names and layers)
    svc.apply_layers("s1", &names, vec![None]).unwrap();
    let st = lock(&svc.host().st);
    assert_eq!(
        st.lost.as_ref().unwrap().plan.layers,
        st.open.as_ref().unwrap().plan.layers
    );
}

#[test]
fn a_failed_hand_over_of_the_layers_is_an_error_not_silence() {
    // P4-7a fix 5: a bank the transport refuses (wrong track count) is reported
    let (svc, names) = service_with_open_plan("refresh_err");
    let e = svc.apply_layers("s1", &names, vec![]).unwrap_err();
    assert!(e.to_string().contains("layers for"), "{e}");
    // another project's refresh touches nothing
    svc.apply_layers("other", &names, vec![]).unwrap();
}

#[test]
fn the_template_cache_does_not_hand_a_freed_one_shots_template_to_a_new_one() {
    // P4-7a: the key is an address; a one-shot freed and a new one of the same length may share it
    let mk = |v: f32| {
        Arc::new(OneShot {
            sr: 48_000,
            chans: vec![vec![v; 64]],
        })
    };
    let plan = |shot: Arc<OneShot>| Plan {
        sid: "s".into(),
        names: vec!["t".into()],
        roles: vec!["kick".into()],
        specs: vec![],
        vols: vec![],
        layers: vec![Some(LayerPlan {
            path: PathBuf::from("a.wav"),
            shot,
            starts: vec![(0.0, 1.0)],
        })],
    };
    let mut tpls = LayerTpls::new();
    let first = mk(0.25);
    layer_bank(&mut tpls, &plan(first.clone()), 48_000);
    let (_, old_tpl, _) = tpls.values().next().unwrap().clone();
    assert_eq!(old_tpl[0], 0.25);
    // force the stale-address case: the cached entry sits under the key of a different one-shot
    let second = mk(0.75);
    let key = (Arc::as_ptr(&second) as *const () as usize, 48_000u32);
    let entry = tpls.values().next().unwrap().clone();
    tpls.clear();
    tpls.insert(key, entry);
    layer_bank(&mut tpls, &plan(second.clone()), 48_000);
    assert_eq!(
        tpls[&key].1[0], 0.75,
        "the template of the new one-shot, not the cached one"
    );
}
