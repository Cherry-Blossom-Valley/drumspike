//! The key sets of the commands against the responses of the retired Python server, recorded in
//! `tests/fixtures/app-shapes/` (P3-3a; frozen since P3-12, when `tests/test_app_shapes.py` went with the
//! server). Nine JSON commands, ten files (`project_list` has two shapes: the sessions rows of route 19
//! and the id rows of route 1).

mod common;

use common::*;
use drumspike_app::ListShape;
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let p = repo_root()
        .join("tests/fixtures/app-shapes")
        .join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}

fn same_keys(command: &str, got: &Value, fixture_name: &str) {
    assert_eq!(
        shape(got),
        shape(&fixture(fixture_name)),
        "{command}: key set differs from {fixture_name}.json"
    );
}

#[test]
fn the_nine_json_commands_have_the_keys_of_the_server_responses() {
    let root = tmp("shapes");
    let dir = project(
        &root,
        "Shapes",
        &[kick_spec("Kick_In", "kick", 36), kick_spec("Snare_Top", "snare", 38)],
    );
    let svc = service_dsp();
    let opened = svc.project_open(dir.to_str().unwrap()).unwrap();
    let sid = opened["sid"].as_str().unwrap().to_string();
    same_keys("project_open", &opened, "project_open");

    let started = svc
        .analyze_start(&sid, Some(vec!["Kick_In".into(), "Snare_Top".into()]), None, None)
        .unwrap();
    same_keys("analyze_start", &started, "analyze_start");
    let mut status = wait_done(&svc, &sid);
    // P4-13 stage B: the flam rule's outcome on the snare track (`flam` of the job and of the track) is the app's
    // own key; it is taken out, every other key is compared as before
    assert!(
        status.as_object_mut().unwrap().shift_remove("flam").is_some(),
        "{status}"
    );
    assert!(status["tracks"]["Snare_Top"]
        .as_object_mut()
        .unwrap()
        .shift_remove("flam")
        .is_some());
    // #281 A1: the snare ghost-note gate's outcome (`ghost_gate` of the job and of the track) is the app's
    // own key; it is taken out, every other key is compared as before
    assert!(
        status.as_object_mut().unwrap().shift_remove("ghost_gate").is_some(),
        "{status}"
    );
    assert!(status["tracks"]["Snare_Top"]
        .as_object_mut()
        .unwrap()
        .shift_remove("ghost_gate")
        .is_some());
    same_keys("analyze_status", &status, "analyze_status");

    same_keys("project_get", &svc.project_get(&sid).unwrap(), "project_get");
    same_keys("project_list", &svc.project_list(ListShape::Sessions), "project_list");
    same_keys("project_list", &svc.project_list(ListShape::Ids), "project_list_ids");
    // P4-12: the app's detector record names the refinement rule (`refine`); the server's had no
    // version. The keys the app adds are taken out; every other key is compared as before.
    let mut tracks = svc.tracks_get(&sid).unwrap();
    for t in tracks["tracks"].as_object_mut().unwrap().values_mut() {
        let det = t["detector"].as_object_mut().unwrap();
        assert!(det.shift_remove("refine").is_some(), "{det:?}");
        // P4-13 stage B: and the flam rule's version and outcome on a snare track
        det.shift_remove("flam_rule");
        det.shift_remove("flam");
        // #281 A1: and the ghost-note gate's record on a snare track
        det.shift_remove("ghost_gate");
    }
    same_keys("tracks_get", &tracks, "tracks_get");

    let ev = svc.events_get(&sid, "Kick_In").unwrap();
    same_keys("events_get", &ev, "events_get");
    same_keys(
        "events_set",
        &svc.events_set(&sid, "Kick_In", &ev["events"]).unwrap(),
        "events_set",
    );

    // The server's peaks.json holds the level arrays; the app sends them as tiles, so the
    // header answer has the same top-level keys and the same level names.
    let info = svc.peaks_info(&sid, "Kick_In").unwrap();
    let want = fixture("peaks_info");
    let top = |v: &Value| {
        let mut k = v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        k.sort();
        k
    };
    assert_eq!(top(&info), top(&want));
    assert_eq!(top(&info["levels"]), top(&want["levels"]));
    assert!(info["levels"]["256"]["n"].as_u64().unwrap() > 0);
    assert_eq!(
        info["levels"]["256"]["n"].as_u64().unwrap(),
        info["n_samples"].as_u64().unwrap() / 256
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn shape_can_fail() {
    // proof that the comparison fires: a planted extra key and a renamed key differ
    let a = serde_json::json!({"x": 1, "y": {"z": [1]}});
    assert_ne!(
        shape(&a),
        shape(&serde_json::json!({"x": 1, "y": {"z": [1]}, "extra": 0}))
    );
    assert_ne!(shape(&a), shape(&serde_json::json!({"x": 1, "y": {"w": [1]}})));
    assert_eq!(
        shape(&serde_json::json!([{"b": 1, "a": 2}])),
        serde_json::json!([{"a": null, "b": null}])
    );
    assert_eq!(shape(&serde_json::json!([])), serde_json::json!("list"));
}
