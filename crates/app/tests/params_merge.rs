//! #442: `track_params_set` and the `params` of `analyze_start` merge into the track's stored
//! params instead of replacing them, through `drumspike_app::Service`, no Tauri.

mod common;

use common::*;
use drumspike_app::Service;
use serde_json::{Value, json};

fn opened(tag: &str) -> (Service, String, std::path::PathBuf) {
    let root = tmp(tag);
    let dir = project(
        &root,
        "Song",
        &[kick_spec("Kick_In", "kick", 36), kick_spec("Snare_Top", "snare", 38)],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    (svc, sid, root)
}

fn hits(svc: &Service, sid: &str, name: &str) -> Vec<Value> {
    svc.events_get(sid, name).unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn seed(svc: &Service, sid: &str, name: &str) {
    let ev = json!([{"t": 0.5, "v": 80}, {"t": 1.0, "v": 90}]);
    svc.events_set(sid, name, &ev).unwrap();
}

fn stored_params(svc: &Service, sid: &str) -> Value {
    svc.tracks_get(sid).unwrap()["tracks"]["Kick_In"]["params"].clone()
}

#[test]
fn track_params_set_keeps_the_stored_shift_and_window() {
    let (svc, sid, root) = opened("pm1");
    seed(&svc, &sid, "Kick_In");
    // the page's way: the whole object, with the shift and a refine window
    svc.track_params_set(
        &sid,
        "Kick_In",
        json!({"shift_samples": 240, "refine_lo_ms": -5.0, "refine_hi_ms": 10.0})
            .as_object()
            .unwrap(),
        false,
    )
    .unwrap();
    // an agent sends one field: the shift and the window survive
    let r = svc
        .track_params_set(&sid, "Kick_In", json!({"sensitivity": 60}).as_object().unwrap(), false)
        .unwrap();
    let p = &r["params"];
    assert_eq!(p["shift_samples"], 240, "the stored shift survives a one-field call");
    assert_eq!(p["refine_lo_ms"], -5.0);
    assert_eq!(p["refine_hi_ms"], 10.0);
    assert_eq!(p["sensitivity"], 60.0);
    assert!(p["thresholds"].is_array(), "the sensitivity gives the thresholds");
    assert_eq!(stored_params(&svc, &sid), *p, "the answer holds the merged object");
    // the stored shift is real: shifting to 0 moves the hits by exactly -240 samples
    let before: Vec<f64> = hits(&svc, &sid, "Kick_In")
        .iter()
        .map(|e| e["t"].as_f64().unwrap())
        .collect();
    let back = svc.track_shift_set(&sid, "Kick_In", 0).unwrap();
    assert_eq!(back["delta"], -240);
    assert_eq!(back["moved"], 2);
    let after: Vec<f64> = hits(&svc, &sid, "Kick_In")
        .iter()
        .map(|e| e["t"].as_f64().unwrap())
        .collect();
    for (a, b) in before.iter().zip(&after) {
        assert!((b - a + 240.0 / 48_000.0).abs() < 1e-9, "{b} - {a} is not -240 samples");
    }
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn analyze_start_params_merge_into_the_stored_params() {
    let (svc, sid, root) = opened("pm2");
    seed(&svc, &sid, "Kick_In");
    svc.track_params_set(
        &sid,
        "Kick_In",
        json!({"shift_samples": 240, "refine_lo_ms": -5.0, "refine_hi_ms": 10.0})
            .as_object()
            .unwrap(),
        false,
    )
    .unwrap();
    svc.analyze_start(
        &sid,
        Some(vec!["Kick_In".into()]),
        json!({"Kick_In": {"sensitivity": 60}}).as_object(),
        None,
    )
    .unwrap();
    let p = stored_params(&svc, &sid);
    assert_eq!(p["shift_samples"], 240, "the stored shift survives the analysis params");
    assert_eq!(p["refine_lo_ms"], -5.0);
    assert_eq!(p["refine_hi_ms"], 10.0);
    assert_eq!(p["sensitivity"], 60.0);
    assert!(p["thresholds"].is_array());
    // the analysis runs in the background: wait for it before the folder goes (a job that still
    // writes made the removal fail on a slow host)
    wait_done(&svc, &sid);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_null_key_removes_the_stored_key() {
    let (svc, sid, root) = opened("pm3");
    seed(&svc, &sid, "Kick_In");
    svc.track_params_set(
        &sid,
        "Kick_In",
        json!({"shift_samples": 240, "min_gap_ms": 30}).as_object().unwrap(),
        false,
    )
    .unwrap();
    let r = svc
        .track_params_set(
            &sid,
            "Kick_In",
            json!({"shift_samples": null}).as_object().unwrap(),
            false,
        )
        .unwrap();
    let p = &r["params"];
    assert!(p.get("shift_samples").is_none(), "null removes the key: {p}");
    assert_eq!(p["min_gap_ms"], 30.0, "the other keys stay");
    assert_eq!(stored_params(&svc, &sid), *p);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_absent_call_keeps_every_stored_key_and_a_fresh_track_gets_the_call() {
    let (svc, sid, root) = opened("pm4");
    seed(&svc, &sid, "Kick_In");
    svc.track_params_set(&sid, "Kick_In", json!({"min_gap_ms": 30}).as_object().unwrap(), false)
        .unwrap();
    // a call without the key keeps it
    svc.track_params_set(&sid, "Kick_In", json!({"refine": false}).as_object().unwrap(), false)
        .unwrap();
    let p = stored_params(&svc, &sid);
    assert_eq!(p["min_gap_ms"], 30.0);
    assert_eq!(p["refine"], false);
    // a track with no stored params gets the call's object as it is
    svc.track_params_set(&sid, "Snare_Top", json!({"min_gap_ms": 25}).as_object().unwrap(), false)
        .unwrap();
    assert_eq!(
        svc.tracks_get(&sid).unwrap()["tracks"]["Snare_Top"]["params"],
        json!({"min_gap_ms": 25})
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn track_params_set_replace_stores_exactly_the_sent_object() {
    let (svc, sid, root) = opened("pm5");
    seed(&svc, &sid, "Kick_In");
    // a stored object with a sensitivity and a shift
    svc.track_params_set(
        &sid,
        "Kick_In",
        json!({"thresholds": [0.1, 0.2, 0.3, 0.4, 0.5], "sensitivity": 60, "shift_samples": 240})
            .as_object()
            .unwrap(),
        false,
    )
    .unwrap();
    // the page's undo of a re-pick: the whole before object, without the sensitivity
    let sent = json!({"thresholds": [0.2, 0.3, 0.4, 0.5, 0.6], "shift_samples": 480});
    let r = svc
        .track_params_set(&sid, "Kick_In", sent.as_object().unwrap(), true)
        .unwrap();
    assert_eq!(r["params"], sent, "replace: true stores exactly the sent object");
    assert_eq!(
        stored_params(&svc, &sid),
        sent,
        "no sensitivity leaks through a replace"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn analyze_start_replace_stores_exactly_the_sent_params() {
    let (svc, sid, root) = opened("pm6");
    seed(&svc, &sid, "Kick_In");
    svc.track_params_set(
        &sid,
        "Kick_In",
        json!({"thresholds": [0.1, 0.2, 0.3, 0.4, 0.5], "sensitivity": 60, "shift_samples": 240})
            .as_object()
            .unwrap(),
        false,
    )
    .unwrap();
    let sent = json!({"Kick_In": {"thresholds": [0.3, 0.4, 0.5, 0.6, 0.7], "shift_samples": 120}});
    svc.analyze_start_keep(
        &sid,
        Some(vec!["Kick_In".into()]),
        sent.as_object(),
        None,
        None,
        false,
        true,
    )
    .unwrap();
    let p = stored_params(&svc, &sid);
    assert_eq!(
        p,
        json!({"thresholds": [0.3, 0.4, 0.5, 0.6, 0.7], "shift_samples": 120}),
        "replace: true stores exactly the sent object: {p}"
    );
    assert!(p.get("sensitivity").is_none());
    // the analysis runs in the background: wait for it before the folder goes
    wait_done(&svc, &sid);
    std::fs::remove_dir_all(&root).unwrap();
}
