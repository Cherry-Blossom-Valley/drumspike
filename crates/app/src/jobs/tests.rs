use super::*;

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn begin_store_running_needs_backup_repick_stored_and_analysed_keep_the_rules() {
    let j = Jobs::default();
    assert!(!j.storing("s") && !j.running("s"));
    let g = j.begin_store("s").expect("free");
    assert!(j.storing("s") && j.begin_store("s").is_none(), "one store at a time");
    assert!(j.begin_store("t").is_some(), "per project");
    drop(g);
    assert!(!j.storing("s"));
    j.queue("s", &["a".into()]);
    assert!(j.running("s"));
    assert!(
        j.needs_backup("s", "a") && j.needs_backup("s", "a"),
        "asking does not mark"
    );
    j.repick_stored("s", "a");
    assert!(!j.needs_backup("s", "a") && j.needs_backup("s", "b"));
    j.analysed("s", "a");
    assert!(
        j.needs_backup("s", "a"),
        "after an analysis the backup is written again"
    );
}

#[test]
fn jobs_start_idle_and_queue_every_target() {
    let j = Jobs::default();
    assert_eq!(j.status("s1"), json!({"state": "idle", "tracks": {}}));
    let t = j.queue("s1", &["a".into(), "b".into()]);
    assert_eq!(
        j.status("s1"),
        json!({"state": "queued", "tracks": {"a": {"state": "queued"}, "b": {"state": "queued"}}})
    );
    let v = j.update("s1", &t, |v| v["state"] = Value::from("running")).unwrap();
    assert_eq!(v["state"], "running");
    assert_eq!(j.status("s2")["state"], "idle");
}

#[test]
fn fail_unfinished_turns_open_tracks_into_errors_and_ends_the_job() {
    let j = Jobs::default();
    let t = j.queue("s1", &["a".into(), "b".into(), "c".into()]);
    j.update("s1", &t, |v| {
        v["tracks"]["a"] = json!({"state": "done", "n_events": 3});
        v["tracks"]["b"] = json!({"state": "running", "stage": "decoding"});
    });
    let v = j.fail_unfinished("s1", &t, "boom").unwrap();
    assert_eq!(v["state"], "done");
    assert_eq!(v["tracks"]["a"]["state"], "done");
    assert_eq!(v["tracks"]["b"], json!({"state": "error", "error": "boom"}));
    assert_eq!(v["tracks"]["c"], json!({"state": "error", "error": "boom"}));
    assert_eq!(j.status("s1"), v);
}

#[test]
fn cancel_and_clear_set_the_flag_and_finish_cancelled_marks_the_rest() {
    let j = Jobs::default();
    assert!(!j.cancel("s1"), "no job: nothing to cancel");
    assert!(
        j.cancelled("s1"),
        "no flag counts as cancelled: a cleared job never runs"
    );
    let t = j.queue("s1", &["a".into(), "b".into(), "c".into()]);
    assert!(!j.cancelled("s1"));
    assert!(j.cancel("s1"));
    assert!(j.cancelled("s1"));
    assert!(
        j.cancel("s1"),
        "a second cancel is still a cancel while the job is queued"
    );
    let v = j.finish_cancelled("s1", &t, &["b".into(), "c".into()]).unwrap();
    assert_eq!(v["state"], "done");
    assert_eq!(
        v["tracks"]["a"],
        json!({"state": "queued"}),
        "the caller marks the consumed tracks"
    );
    assert_eq!(v["tracks"]["b"], json!({"state": "error", "error": CANCELLED_ERROR}));
    assert_eq!(v["tracks"]["c"], json!({"state": "error", "error": CANCELLED_ERROR}));
    assert_eq!(j.status("s1"), v);
    assert!(!j.cancel("s1"), "a done job is not cancelled");
    // a new job starts with a cleared flag; the close of the project sets it and drops the job
    let t2 = j.queue("s1", &["a".into()]);
    assert!(!j.cancelled("s1"));
    j.clear("s1");
    assert!(j.cancelled("s1"));
    assert!(
        j.finish_cancelled("s1", &t2, &["a".into()]).is_none(),
        "nothing is written for a closed project"
    );
    assert_eq!(j.status("s1")["state"], "idle");
}

/// R08: a start that replaces a job (the close and reopen path; the command refuses a
/// duplicate start) makes the older generation stale: the old worker writes and emits
/// nothing, and its cancel and crash paths write nothing either.
#[test]
fn a_second_start_replaces_the_job_and_the_stale_generation_writes_nothing() {
    let j = Jobs::default();
    let first = j.queue("s1", &["a".into(), "b".into()]);
    j.update("s1", &first, |v| v["state"] = Value::from("running")).unwrap();
    assert!(j.running("s1"));
    let second = j.queue("s1", &["a".into()]);
    assert_ne!(first.gen(), second.gen(), "a new job is a new generation");
    assert!(!j.active("s1", &first), "the replaced generation is stale");
    assert!(j.active("s1", &second));
    assert_eq!(
        j.update("s1", &first, |v| v["state"] = Value::from("done")),
        None,
        "the old worker resurrects no state"
    );
    assert_eq!(
        j.update("s1", &first, |v| v["tracks"]["a"] =
            json!({"state": "done", "n_events": 1})),
        None,
        "the old worker writes no track entry"
    );
    assert_eq!(
        j.status("s1")["state"],
        "queued",
        "the new job is untouched by the old worker"
    );
    assert_eq!(j.finish_cancelled("s1", &first, &["b".into()]), None);
    assert_eq!(j.fail_unfinished("s1", &first, "boom"), None);
    assert_eq!(j.status("s1")["state"], "queued");
    // the new generation keeps working
    assert_eq!(
        j.update("s1", &second, |v| v["state"] = Value::from("running"))
            .unwrap()["state"],
        "running"
    );
}

/// R08: a cancel reaches only the active generation: the flags are per job, and a stale
/// generation stops because it is not active, not because its flag was reused.
#[test]
fn a_cancel_reaches_only_the_active_generation() {
    let j = Jobs::default();
    // the cancel during the queued to running transition
    let first = j.queue("s1", &["a".into()]);
    assert!(!first.is_cancelled());
    j.update("s1", &first, |v| v["state"] = Value::from("running")).unwrap();
    let second = j.queue("s1", &["a".into()]);
    assert!(j.cancel("s1"), "the active job is asked to stop");
    assert!(second.is_cancelled());
    assert!(!first.is_cancelled(), "the old flag is never reused");
    assert!(!j.active("s1", &second), "the asked job stops at its next check");
    assert!(!j.active("s1", &first), "the stale worker stops anyway");
    // and a cancel while the job is still queued works the same
    let j2 = Jobs::default();
    let t = j2.queue("s2", &["a".into()]);
    assert_eq!(j2.status("s2")["state"], "queued");
    assert!(j2.cancel("s2"));
    assert!(t.is_cancelled());
    assert!(!j2.active("s2", &t));
    assert!(j2.cancelled("s2"));
}

/// R08: a closed project ignores the late callbacks of its worker (no resurrected state,
/// queued or running), and the reopen gets a fresh generation the old token cannot touch.
#[test]
fn a_closed_project_ignores_late_callbacks_and_a_reopen_gets_a_fresh_generation() {
    let j = Jobs::default();
    // close while queued: the late update of the worker resurrects nothing
    let queued = j.queue("s1", &["a".into()]);
    j.clear("s1");
    assert_eq!(j.status("s1"), json!({"state": "idle", "tracks": {}}));
    assert_eq!(j.update("s1", &queued, |v| v["state"] = Value::from("running")), None);
    assert_eq!(j.status("s1")["state"], "idle");
    // close while running: the same, and the cancel and crash paths write nothing
    let running = j.queue("s1", &["a".into()]);
    j.update("s1", &running, |v| v["state"] = Value::from("running"))
        .unwrap();
    j.clear("s1");
    assert_eq!(
        j.update("s1", &running, |v| v["tracks"]["a"] =
            json!({"state": "done", "n_events": 1})),
        None
    );
    assert_eq!(j.finish_cancelled("s1", &running, &["a".into()]), None);
    assert_eq!(j.fail_unfinished("s1", &running, "boom"), None);
    assert_eq!(j.status("s1")["state"], "idle");
    // the reopen: a fresh generation, and the late progress of the closed one stays out
    let reopened = j.queue("s1", &["a".into()]);
    assert_ne!(reopened.gen(), queued.gen());
    assert_ne!(reopened.gen(), running.gen());
    assert!(j.active("s1", &reopened));
    assert!(!j.active("s1", &running));
    assert_eq!(
        j.update("s1", &running, |v| v["tracks"]["a"] =
            json!({"state": "running", "stage": "decoding"})),
        None,
        "the late progress writes nothing into the reopened job"
    );
    assert_eq!(j.status("s1")["tracks"]["a"], json!({"state": "queued"}));
}

/// R08: a stale worker commits no events: the project was closed while its track ran in
/// the model step, so the store is refused (`finish_one` answers `analysis cancelled`).
/// The DSP fallback runs on silence, so the fresh generation commits and the track has
/// its (empty) events and detector record afterwards.
#[test]
fn a_stale_worker_commits_no_events() {
    use drumspike_core::project::{add_track, new_project};
    let dir = std::env::temp_dir().join(format!("ds_jobs_stale_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let svc = Service::new(ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    let mut project = new_project(&dir, "Stale").unwrap();
    add_track(&mut project, "kick", None, "kick", 36, None).unwrap();
    let op = svc.inner.registry.register(project);
    let sid = lock(&op).sid.clone();
    let prepared = || Prepared {
        dec: crate::media::Decoded {
            sr: 48_000,
            samples: vec![0.0; 4800],
        },
        role: "kick".into(),
        parsed: parse_params(None).unwrap(),
        follows: None,
        dir: dir.join("Analysis/kick"),
        source_key: CacheKey {
            size: None,
            mtime: None,
            head_sha1: None,
        },
        mix: "ffmpeg".into(),
    };
    // the project is closed while the track of the old worker runs in its model step
    let token = svc.inner.jobs.queue(&sid, &["kick".to_string()]);
    svc.inner.jobs.clear(&sid);
    let e = finish_one(&svc, &op, "kick", prepared(), false, &sid, &token).unwrap_err();
    assert_eq!(e, CANCELLED_ERROR);
    {
        let g = lock(&op);
        let tr = g.project.doc.tracks().find(|(n, _)| n.as_str() == "kick").unwrap().1;
        assert!(tr.extra.get("detector").is_none(), "no commit from the stale worker");
        assert!(tr.sr.is_none(), "the stale worker stored no rate either");
    }
    // the fresh generation commits: the store runs
    let token2 = svc.inner.jobs.queue(&sid, &["kick".to_string()]);
    let entry = finish_one(&svc, &op, "kick", prepared(), false, &sid, &token2).unwrap();
    assert_eq!(entry["state"], "done");
    {
        let g = lock(&op);
        let tr = g.project.doc.tracks().find(|(n, _)| n.as_str() == "kick").unwrap().1;
        assert_eq!(tr.events.as_ref().unwrap().len(), 0, "the silence has no hits");
        assert!(tr.extra.get("detector").is_some(), "the store wrote the record");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// R08: a duplicate start is refused with a 409 while a job is queued or running, and a
/// start works again after the close of the project.
#[test]
fn a_second_analyze_start_is_refused_until_the_project_reopens() {
    use drumspike_core::project::{add_track, new_project};
    let dir = std::env::temp_dir().join(format!("ds_jobs_start2_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let svc = Service::new(ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    let mut project = new_project(&dir, "Twice").unwrap();
    add_track(&mut project, "kick", None, "kick", 36, None).unwrap();
    let op = svc.inner.registry.register(project);
    let sid = lock(&op).sid.clone();
    // the first job is queued (as `start` does before the worker spawns)
    svc.inner.jobs.queue(&sid, &["kick".to_string()]);
    let e = svc
        .analyze_start_keep(&sid, Some(vec!["kick".into()]), None, None, None, false)
        .unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.contains("already queued or running"), "{}", e.message);
    // after the close (and a reopen) a start works again: the real path, a fresh generation
    svc.inner.jobs.clear(&sid);
    assert_eq!(
        svc.analyze_start_keep(&sid, Some(vec!["kick".into()]), None, None, None, false)
            .unwrap(),
        json!({"started": ["kick"]})
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn kind_names_the_dsp_fallback_which_ignores_prepared_features() {
    let slot = DetectorSlot;
    let cfg = ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    };
    assert_eq!(slot.kind(&cfg).unwrap(), Kind::Dsp);
    let x = vec![0.0f32; 48_000];
    let parsed = parse_params(None).unwrap();
    let plain = slot.analyze_prepared(&cfg, &x, 48_000, "kick", &parsed, None).unwrap();
    let prepared = slot.analyze_prepared(&cfg, &x, 48_000, "kick", &parsed, None).unwrap();
    assert_eq!(plain, prepared);
    assert_eq!(prepared.1["detector"], DETECTOR_DSP);
    // #281 D16: no model meta on the model-free main
    let kept = slot.analyze_kept(&cfg, &x, 48_000, "kick", &parsed, None).unwrap();
    assert_eq!((kept.0, kept.1, kept.2), (prepared.0.clone(), prepared.1.clone(), None));
    assert_eq!(slot.model_meta(&cfg).unwrap(), None);
}

#[test]
fn parse_params_maps_a_sensitivity_without_thresholds() {
    let p = parse_params(Some(&obj(json!({"sensitivity": 100})))).unwrap();
    assert_eq!(p.params.thresholds, crate::sensitivity::thresholds_of(100.0));
    let both = parse_params(Some(&obj(
        json!({"sensitivity": 100, "thresholds": [0.3, 0.3, 0.3, 0.3, 0.3]}),
    )))
    .unwrap();
    assert_eq!(both.params.thresholds, [0.3; 5], "thresholds win");
    let none = parse_params(Some(&obj(json!({"sensitivity": 50})))).unwrap();
    assert_eq!(none.params.thresholds, AnalyzeParams::default().thresholds);
}

#[test]
fn parse_params_reads_the_defaults_and_the_engine_keys() {
    let d = parse_params(None).unwrap();
    assert_eq!(d.params, AnalyzeParams::default());
    assert!(d.thresholds_error.is_none());
    let p = parse_params(Some(&obj(json!({
        "min_gap_ms": 45, "refine": true, "shift_samples": -480, "vel_lo_db": -50, "vel_hi_db": -3,
        "refine_lo_ms": -7, "refine_hi_ms": 7, "thresholds": [0.3, 0.3, 0.3, 0.3, 0.3]
    }))))
    .unwrap();
    assert_eq!(p.params.min_gap_ms, 45.0);
    assert_eq!(p.params.shift_samples, -480);
    assert_eq!((p.params.velocity.lo_db, p.params.velocity.hi_db), (-50.0, -3.0));
    assert_eq!((p.params.window.lo_ms, p.params.window.hi_ms), (-7.0, 7.0));
    assert_eq!(p.params.thresholds, [0.3; 5]);
}

#[test]
fn parse_params_follows_python_truth_and_reports_bad_values() {
    let off = parse_params(Some(&obj(json!({"refine": false, "refine_lo_ms": -9})))).unwrap();
    assert!(!off.params.refine);
    assert_eq!(
        off.params.window,
        RefineWindow::DEFAULT,
        "the window counts only with refine"
    );
    let none = parse_params(Some(&obj(json!({"shift_samples": null})))).unwrap();
    assert_eq!(none.params.shift_samples, 0, "int(x or 0)");
    assert!(parse_params(Some(&obj(json!({"min_gap_ms": "fast"})))).is_err());
    let short = parse_params(Some(&obj(json!({"thresholds": [0.1, 0.2]})))).unwrap();
    assert_eq!(
        short.thresholds_error.as_deref(),
        Some("thresholds: 2 values, expected 5")
    );
    let nolist = parse_params(Some(&obj(json!({"thresholds": 3})))).unwrap();
    assert!(nolist.thresholds_error.is_some());
    for (v, t) in [
        (json!(0), false),
        (json!("x"), true),
        (json!([]), false),
        (json!({"a": 1}), true),
    ] {
        assert_eq!(truthy(&v), t, "{v}");
    }
}

/// A model step that gives `n` hits for the kick threshold `th[0]` (the counts by step).
fn counts(table: &[(f64, usize)], tried: &mut Vec<f64>, th: &[f64; N_CLASSES]) -> Result<Analysis, String> {
    tried.push(th[0]);
    let n = table
        .iter()
        .find(|(t, _)| (th[0] - t).abs() < 0.002)
        .map_or(0, |(_, n)| *n);
    let events = (0..n)
        .map(|i| drumspike_core::Hit {
            t: 0.5 + i as f64,
            v: 90,
            rise: None,
        })
        .collect();
    Ok(Analysis {
        events,
        branches: Default::default(),
        roll: Vec::new(),
        learn_out: Vec::new(),
        learned: Vec::new(),
    })
}

/// P3-18: the steps run in order on the model step; the first step with a hit is kept, the next
/// one replaces it while the count grows by more than 10 %; the DSP path and a track at the end
/// of the slider run none.
#[test]
fn the_retry_lowers_the_thresholds_step_by_step_until_a_hit_appears() {
    // hits from the second step on, then flat: the second step is kept, the third ends the retry
    let mut tried: Vec<f64> = Vec::new();
    let mut run = |th: &[f64; N_CLASSES]| counts(&[(0.05, 1), (0.02, 1), (0.01, 1)], &mut tried, th);
    let found = retry_without_hits(&mut run, &sensitivity::BASE)
        .unwrap()
        .expect("a step with a hit");
    assert_eq!(found.0, 113.0, "the second step (about 0.05)");
    assert_eq!(found.2.events.len(), 1);
    assert_eq!(tried.len(), 3, "the third step did not grow: {tried:?}");
    assert!(
        (tried[0] - 0.1).abs() < 0.002 && (tried[1] - 0.05).abs() < 0.001,
        "{tried:?}"
    );
    // the kick sub of "Session C" (controller decision 2026-10-12): 42, 336, 386, 386 -> 134 kept
    let mut tried: Vec<f64> = Vec::new();
    let mut sub = |th: &[f64; N_CLASSES]| counts(&[(0.101, 42), (0.05, 336), (0.02, 386), (0.01, 386)], &mut tried, th);
    let found = retry_without_hits(&mut sub, &sensitivity::BASE).unwrap().unwrap();
    assert_eq!((found.0, found.2.events.len()), (134.0, 386));
    assert_eq!(
        tried.len(),
        4,
        "150 was tried and did not grow by more than 10 %: {tried:?}"
    );
    // a growth of exactly 10 % does not count; 11 % does
    let mut tried: Vec<f64> = Vec::new();
    let mut flat = |th: &[f64; N_CLASSES]| counts(&[(0.101, 100), (0.05, 110), (0.02, 400)], &mut tried, th);
    assert_eq!(
        retry_without_hits(&mut flat, &sensitivity::BASE).unwrap().unwrap().0,
        95.0
    );
    let mut tried: Vec<f64> = Vec::new();
    let mut grow = |th: &[f64; N_CLASSES]| counts(&[(0.101, 100), (0.05, 111), (0.02, 111)], &mut tried, th);
    assert_eq!(
        retry_without_hits(&mut grow, &sensitivity::BASE).unwrap().unwrap().0,
        113.0
    );
    let mut none = |_: &[f64; N_CLASSES]| -> Result<Analysis, String> {
        Ok(Analysis {
            events: vec![],
            branches: Default::default(),
            roll: Vec::new(),
            learn_out: Vec::new(),
            learned: Vec::new(),
        })
    };
    assert!(retry_without_hits(&mut none, &sensitivity::BASE).unwrap().is_none());
    assert!(retry_without_hits(&mut none, &sensitivity::thresholds_of(150.0))
        .unwrap()
        .is_none());
    let mut fail = |_: &[f64; N_CLASSES]| -> Result<Analysis, String> { Err("boom".into()) };
    assert_eq!(retry_without_hits(&mut fail, &sensitivity::BASE).unwrap_err(), "boom");
}

#[test]
fn the_rule_of_an_analysis_and_of_a_repick() {
    // P4-12: the pin of DRUMSPIKE_REFINE_RULE
    assert_eq!(parse_rule("1"), Some(RefineRule::Reference));
    assert_eq!(parse_rule(" 2 "), Some(RefineRule::Reach));
    assert_eq!(parse_rule("Reference"), Some(RefineRule::Reference));
    assert_eq!(parse_rule("reach"), Some(RefineRule::Reach));
    assert_eq!(parse_rule("3"), Some(RefineRule::FirstMotion));
    assert_eq!(parse_rule("First-Motion"), Some(RefineRule::FirstMotion));
    assert_eq!(parse_rule("foot"), Some(RefineRule::Foot));
    assert_eq!(parse_rule("4"), Some(RefineRule::Foot));
    assert_eq!(parse_rule("5"), None);
    assert_eq!(parse_rule(""), None);
    let free = ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    };
    let pinned = ServiceConfig {
        refine_rule: Some(RefineRule::Reference),
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    };
    // a new analysis runs the current rule unless pinned
    assert_eq!(analysis_rule(&free), RefineRule::CURRENT);
    assert_eq!(RefineRule::CURRENT, RefineRule::FirstMotion);
    assert_eq!(analysis_rule(&pinned), RefineRule::Reference);
    // a re-pick keeps the rule its analysis recorded; no record or no version: before P4-12 (1)
    let rec = |v: Value| {
        detector_record(DETECTOR_DSP, Some("mix"), Value::Null, RefineRule::Reach)
            .as_object()
            .map(|m| {
                let mut m = m.clone();
                m.insert("refine".into(), v);
                Value::Object(m)
            })
    };
    assert_eq!(recorded_rule(&free, None), RefineRule::Reference);
    let old = json!({"detector": "rust-dsp", "chain": "mix", "model": null});
    assert_eq!(recorded_rule(&free, Some(&old)), RefineRule::Reference);
    assert_eq!(recorded_rule(&free, rec(json!(null)).as_ref()), RefineRule::Reference);
    assert_eq!(recorded_rule(&free, rec(json!(1)).as_ref()), RefineRule::Reference);
    assert_eq!(recorded_rule(&free, rec(json!(2)).as_ref()), RefineRule::Reach);
    assert_eq!(recorded_rule(&free, rec(json!(3)).as_ref()), RefineRule::FirstMotion);
    assert_eq!(
        recorded_rule(&free, rec(json!(9)).as_ref()),
        RefineRule::CURRENT,
        "a newer version"
    );
    assert_eq!(
        recorded_rule(&pinned, rec(json!(2)).as_ref()),
        RefineRule::Reference,
        "the pin wins"
    );
    assert_eq!(
        detector_record(DETECTOR_DSP, None, Value::Null, RefineRule::Reach),
        json!({"detector": "rust-dsp", "chain": null, "model": null, "refine": 2})
    );
}

#[test]
fn json_indent_one_matches_python_dump() {
    let s = to_json_indent1(&json!({"a": [1, 2], "b": {}}));
    assert_eq!(s, "{\n \"a\": [\n  1,\n  2\n ],\n \"b\": {}\n}");
}

#[test]
fn the_dsp_fallback_runs_without_a_model_file() {
    let slot = DetectorSlot;
    let cfg = ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    };
    let x = vec![0.0f32; 48_000];
    let parsed = parse_params(None).unwrap();
    let (a, det) = slot.analyze_prepared(&cfg, &x, 48_000, "kick", &parsed, None).unwrap();
    assert!(a.events.is_empty());
    // P4-12: the record names the refinement rule; parse_params alone keeps the engine's (1)
    assert_eq!(
        det,
        json!({"detector": "rust-dsp", "chain": null, "model": null, "refine": 1})
    );
    let mut reach = parse_params(None).unwrap();
    reach.params.rule = RefineRule::Reach;
    assert_eq!(
        slot.analyze_prepared(&cfg, &x, 48_000, "kick", &reach, None).unwrap().1["refine"],
        2
    );
    let e = slot
        .analyze_prepared(&cfg, &x, 22_050, "kick", &parsed, None)
        .map(|_| ())
        .unwrap_err();
    assert_eq!(e.code, 400);
    assert!(e.message.contains("unsupported sample rate"));
}
