//! #343 Training Mode: the correction log of `events_edit` and the Learn marks (`crates/app/src/training.rs`),
//! through `drumspike_app::Service`, no Tauri.

mod common;

use common::*;
use drumspike_app::Service;
use serde_json::{Value, json};
use std::path::PathBuf;

/// 40 ms before and 10 ms after at 48 kHz.
const WINDOW: usize = 1920 + 480;

fn opened(tag: &str) -> (Service, String, PathBuf) {
    let root = tmp(tag);
    let dir = project(&root, "Training", &[kick_spec("k in", "kick", 36)]);
    let svc = service_dsp();
    let home = root.join("home");
    svc.set_home(&home);
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let ev = json!([{"t": 0.5, "v": 80}, {"t": 1.0, "v": 90}, {"t": 1.5, "v": 100}]);
    svc.events_set(&sid, "k in", &ev).unwrap();
    (svc, sid, home)
}

fn records(home: &std::path::Path) -> Vec<Value> {
    let Ok(dir) = std::fs::read_dir(home.join("training")) else {
        return Vec::new();
    };
    dir.flatten()
        .flat_map(|f| {
            std::fs::read_to_string(f.path())
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .map(|l| serde_json::from_str(&l).unwrap())
        .collect()
}

#[test]
fn events_edit_with_training_writes_one_record_per_move_add_and_delete() {
    let (svc, sid, home) = opened("tr_a");
    // a hit moved before: its `orig` is the `from` of the next move
    svc.events_edit(&sid, "k in", &[json!({"op": "move", "index": 2, "t": 1.6})])
        .unwrap();
    let ops = [
        json!({"op": "move", "index": 0, "t": 0.6}),
        json!({"op": "add", "t": 0.25}),
        json!({"op": "delete", "index": 1}),
        json!({"op": "velocity", "index": 2, "v": 64}),
    ];
    let r = svc.events_edit_training(&sid, "k in", &ops, true).unwrap();
    assert_eq!(r["training"], json!({"records": 3}), "{r}");
    let recs = records(&home);
    assert_eq!(recs.len(), 3, "{recs:?}");
    let by = |op: &str| {
        recs.iter()
            .find(|r| r["op"] == op)
            .unwrap_or_else(|| panic!("no {op}: {recs:?}"))
    };
    let (mv, add, del) = (by("move"), by("add"), by("delete"));
    assert_eq!((mv["from"].as_i64(), mv["to"].as_i64()), (Some(24_000), Some(28_800)));
    assert_eq!(mv["event"]["v"], 80);
    assert!(
        mv["confidence"]["c"].is_number() && mv["confidence"]["doubt"].is_boolean(),
        "{mv}"
    );
    assert_eq!(mv["audio"]["at_from"].as_array().unwrap().len(), WINDOW);
    assert_eq!(mv["audio"]["at_to"].as_array().unwrap().len(), WINDOW);
    assert_eq!((add["from"].clone(), add["to"].as_i64()), (Value::Null, Some(12_000)));
    assert!(add["audio"].get("at_from").is_none());
    assert_eq!(add["audio"]["at_to"].as_array().unwrap().len(), WINDOW);
    assert!(add["confidence"]["c"].is_number(), "{add}");
    assert_eq!((del["from"].as_i64(), del["to"].clone()), (Some(48_000), Value::Null));
    assert_eq!(del["event"]["t"], 1.0);
    assert!(del["audio"].get("at_to").is_none());
    assert_eq!(del["audio"]["at_from"].as_array().unwrap().len(), WINDOW);
    for r in &recs {
        assert_eq!(
            (r["sr"].as_i64(), r["track"].as_str(), r["role"].as_str()),
            (Some(48_000), Some("k in"), Some("kick"))
        );
        assert_eq!(r["app"], drumspike_core::VERSION);
        assert!(r["project"]["id"].as_str().is_some_and(|s| s.len() == 12) && r["project"]["name"] == "Training");
        assert!(r["refine"].is_number() && r["ts"].is_number());
    }
    // a second move of the moved hit starts at its `orig`
    let ev = svc.events_get(&sid, "k in").unwrap()["events"].clone();
    let last = ev.as_array().unwrap().len() - 1;
    svc.events_edit_training(&sid, "k in", &[json!({"op": "move", "index": last, "t": 1.7})], true)
        .unwrap();
    let recs = records(&home);
    assert_eq!(recs.last().unwrap()["from"].as_i64(), Some(72_000), "orig 1.5 s");
    assert_eq!(svc.training_status(&sid, None).unwrap()["records"], 4);
}

#[test]
fn the_same_ops_without_training_write_no_file_and_answer_as_before() {
    let (svc, sid, home) = opened("tr_b");
    let (plain, sid2, home2) = opened("tr_b2");
    let ops = [
        json!({"op": "move", "index": 0, "t": 0.6}),
        json!({"op": "add", "t": 0.25}),
        json!({"op": "delete", "index": 1}),
    ];
    let a = svc.events_edit(&sid, "k in", &ops).unwrap();
    let b = plain.events_edit_training(&sid2, "k in", &ops, false).unwrap();
    assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
    assert!(a.get("training").is_none() && a.get("training_error").is_none(), "{a}");
    assert!(!home.join("training").exists() && !home2.join("training").exists());
    // the agent call without the argument behaves the same
    let c = drumspike_app::call::call(
        &svc,
        "events_edit",
        &json!({"sid": sid, "name": "k in", "ops": [{"op": "add", "t": 2.0}]}),
        None,
    );
    assert!(c.is_ok());
    assert!(!home.join("training").exists());
}

#[test]
fn learn_this_and_learn_not_write_one_record_per_example() {
    let (svc, sid, home) = opened("tr_c");
    let r = svc
        .track_learn_mark_training(&sid, "k in", "this", None, Some(0.5), true)
        .unwrap();
    assert_eq!(r["training"], json!({"records": 1}), "{r}");
    svc.track_learn_mark_training(&sid, "k in", "not", None, Some(1.0), true)
        .unwrap();
    let recs = records(&home);
    assert_eq!(recs.len(), 2, "{recs:?}");
    assert_eq!(
        (recs[0]["op"].as_str(), recs[1]["op"].as_str()),
        (Some("learn_this"), Some("learn_not"))
    );
    assert_eq!(
        (recs[0]["from"].as_i64(), recs[1]["from"].as_i64()),
        (Some(24_000), Some(48_000))
    );
    assert!(recs[0]["to"].is_null());
    assert_eq!(recs[0]["event"]["v"], 80);
    assert!(recs[0]["confidence"]["c"].is_number());
    assert_eq!(recs[1]["audio"]["at_from"].as_array().unwrap().len(), WINDOW);
    // without the mode: no record, no new field
    let r = svc.track_learn_mark(&sid, "k in", "this", None, Some(1.5)).unwrap();
    assert!(r.get("training").is_none());
    assert_eq!(records(&home).len(), 2);
}

#[test]
fn training_status_reports_the_state_the_count_and_the_path() {
    let (svc, sid, home) = opened("tr_d");
    let s = svc.training_status(&sid, None).unwrap();
    assert_eq!((s["on"].clone(), s["records"].clone()), (json!(false), json!(0)));
    assert!(
        s["path"]
            .as_str()
            .unwrap()
            .starts_with(home.join("training").to_str().unwrap())
    );
    let s = svc.training_status(&sid, Some(true)).unwrap();
    assert_eq!(s["on"], true);
    svc.events_edit_training(&sid, "k in", &[json!({"op": "add", "t": 0.25})], true)
        .unwrap();
    assert_eq!(svc.training_status(&sid, None).unwrap()["records"], 1);
    assert_eq!(svc.training_status("nope", None).unwrap_err().code, 404);
    // the agent surface lists it
    assert!(drumspike_app::api::find("training_status").is_some());
    let c = drumspike_app::call::call(&svc, "training_status", &json!({"sid": sid}), None);
    assert!(c.is_ok());
}
