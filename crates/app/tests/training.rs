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
fn a_nudge_by_the_keyboard_path_records_person_and_one_by_the_agent_records_agent() {
    // #389: the same nudge through the two paths of the page: `nudgeSelected` (the keyboard) sends no `by`
    // (person), a `DSAgent.move` sends `by: "agent"`; the record says which.
    let (svc, sid, home) = opened("tr_by");
    svc.training_status(&sid, Some(true)).unwrap();
    svc.events_edit_training(&sid, "k in", &[json!({"op": "move", "index": 0, "t": 0.6})], true, None)
        .unwrap();
    svc.events_edit_training_by(
        &sid,
        "k in",
        &[json!({"op": "move", "index": 0, "t": 0.7})],
        true,
        None,
        Some("agent"),
    )
    .unwrap();
    let recs = records(&home);
    assert_eq!(recs.len(), 2, "{recs:?}");
    assert_eq!(recs[0]["by"], json!("person"), "{}", recs[0]);
    assert_eq!(recs[1]["by"], json!("agent"), "{}", recs[1]);
    // a wrong `by` is refused; the dispatch of `events_edit` validates it
    let e = drumspike_app::call::call(
        &svc,
        "events_edit",
        &json!({"sid": sid, "name": "k in", "ops": [{"op": "move", "index": 0, "t": 0.8}],
                "training": true, "by": "robot"}),
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, 400, "{}", e.message);
    assert!(e.message.contains("by"), "{}", e.message);
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
    let r = svc.events_edit_training(&sid, "k in", &ops, true, None).unwrap();
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
    svc.events_edit_training(
        &sid,
        "k in",
        &[json!({"op": "move", "index": last, "t": 1.7})],
        true,
        None,
    )
    .unwrap();
    let recs = records(&home);
    assert_eq!(recs.last().unwrap()["from"].as_i64(), Some(72_000), "orig 1.5 s");
    assert_eq!(svc.training_status(&sid, None).unwrap()["records"], 4);
}

#[test]
fn a_confirm_op_writes_one_record_and_changes_nothing() {
    let (svc, sid, home) = opened("tr_confirm");
    let before = svc.events_get(&sid, "k in").unwrap()["events"].clone();
    let dirty_before = svc.project_get(&sid).unwrap()["dirty"].clone(); // the fixture wrote hits, so it may be set
    let view = json!({"ms": 5, "px_per_sample": 2});
    let r = svc
        .events_edit_training(
            &sid,
            "k in",
            &[json!({"op": "confirm", "index": 1})],
            true,
            Some(view.clone()),
        )
        .unwrap();
    // (a) the answer is the usual one: the op applied nothing, the log got one record
    assert_eq!(r["training"], json!({"records": 1}), "{r}");
    for k in ["saved", "added", "deleted", "moved", "velocity", "restored", "toggled"] {
        assert_eq!(r[k], 0, "{r}");
    }
    // the document, its `dirty` flag and the undo history (the page's stacks ride on the stored hits) stay as they were
    assert_eq!(
        svc.events_get(&sid, "k in").unwrap()["events"],
        before,
        "the hits are unchanged"
    );
    assert_eq!(
        svc.project_get(&sid).unwrap()["dirty"],
        dirty_before,
        "the dirty flag stays as it was"
    );
    // the record: from = to = the stored position, the event, its confidence, one audio window, the view
    let recs = records(&home);
    assert_eq!(recs.len(), 1, "{recs:?}");
    let rec = &recs[0];
    assert_eq!(rec["op"], "confirm");
    assert_eq!(rec["from"], rec["to"]);
    assert_eq!(rec["from"], 48_000, "1.0 s at 48 kHz");
    assert_eq!(rec["event"]["t"], 1.0);
    assert_eq!(rec["event"]["v"], 90);
    assert!(rec["confidence"]["c"].is_number(), "{rec}");
    assert_eq!(rec["audio"]["at_from"].as_array().unwrap().len(), WINDOW);
    assert!(rec["audio"].get("at_to").is_none(), "{rec}");
    assert_eq!(rec["view"], view, "the view as sent");
    assert_eq!(rec["precision"], "sample", "two pixels per sample");
    assert_eq!(
        (rec["track"].clone(), rec["role"].clone()),
        (json!("k in"), json!("kick"))
    );
    assert_eq!(svc.training_status(&sid, None).unwrap()["records"], 1);
    // a second confirm of the same hit writes one record more, still nothing in the document
    let r = svc
        .events_edit_training(&sid, "k in", &[json!({"op": "confirm", "index": 1})], true, None)
        .unwrap();
    assert_eq!(r["training"], json!({"records": 1}), "{r}");
    assert_eq!(records(&home).len(), 2);
    assert_eq!(svc.events_get(&sid, "k in").unwrap()["events"], before);
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], dirty_before);
    // a bad index refuses the whole call and writes nothing (the ops apply as a whole)
    let e = svc
        .events_edit_training(&sid, "k in", &[json!({"op": "confirm", "index": 99})], true, None)
        .unwrap_err();
    assert_eq!(e.code, 400, "{}", e.message);
    assert_eq!(records(&home).len(), 2);
}

#[test]
fn a_confirm_without_training_is_refused_and_writes_nothing() {
    let (svc, sid, home) = opened("tr_confirm_off");
    let dirty_before = svc.project_get(&sid).unwrap()["dirty"].clone(); // the fixture wrote hits, so it may be set
    // (b) the op without the mode is refused with an error that names the mode
    let e = svc
        .events_edit_training(&sid, "k in", &[json!({"op": "confirm", "index": 0})], false, None)
        .unwrap_err();
    assert_eq!(e.code, 400, "{}", e.message);
    assert!(e.message.contains("Training Mode"), "{}", e.message);
    // the agent surface behaves the same (the page never sends the op with the mode off)
    let e = drumspike_app::call::call(
        &svc,
        "events_edit",
        &json!({"sid": sid, "name": "k in", "ops": [{"op": "confirm", "index": 0}]}),
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, 400, "{}", e.message);
    assert!(e.message.contains("Training Mode"), "{}", e.message);
    // a mixed call is refused whole: no record, no edit
    let e = svc
        .events_edit_training(
            &sid,
            "k in",
            &[
                json!({"op": "confirm", "index": 0}),
                json!({"op": "move", "index": 0, "t": 0.6}),
            ],
            false,
            None,
        )
        .unwrap_err();
    assert_eq!(e.code, 400, "{}", e.message);
    assert!(e.message.contains("Training Mode"), "{}", e.message);
    let ev = svc.events_get(&sid, "k in").unwrap()["events"].clone();
    assert_eq!(ev[0]["t"], 0.5, "the move did not apply");
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], dirty_before);
    assert!(records(&home).is_empty(), "{:?}", records(&home));
    // a good index and the mode on: the same ops log and apply
    let r = svc
        .events_edit_training(
            &sid,
            "k in",
            &[
                json!({"op": "confirm", "index": 0}),
                json!({"op": "move", "index": 0, "t": 0.6}),
            ],
            true,
            None,
        )
        .unwrap();
    assert_eq!(r["training"], json!({"records": 2}), "{r}");
    assert_eq!(r["moved"], 1, "{r}");
    let recs = records(&home);
    assert_eq!(recs.len(), 2, "{recs:?}");
    assert_eq!(
        (recs[0]["op"].clone(), recs[1]["op"].clone()),
        (json!("move"), json!("confirm"))
    );
    assert_eq!(recs[1]["from"], 24_000, "the stored position before the move");
    assert_eq!(recs[1]["to"], 24_000);
}

#[test]
fn the_view_of_the_request_rides_in_the_record_and_derives_the_precision() {
    let (svc, sid, home) = opened("tr_view");
    // #353 (a): the view as sent, the precision from its px_per_sample, a delete carries the view without a precision
    let wide = json!({"ms": 2, "px_per_sample": 8});
    let ops = [
        json!({"op": "move", "index": 0, "t": 0.6}),
        json!({"op": "add", "t": 0.25}),
        json!({"op": "delete", "index": 1}),
    ];
    let r = svc
        .events_edit_training(&sid, "k in", &ops, true, Some(wide.clone()))
        .unwrap();
    assert_eq!(r["training"], json!({"records": 3}), "{r}");
    let recs = records(&home);
    assert_eq!(recs.len(), 3, "{recs:?}");
    let by = |op: &str| {
        recs.iter()
            .find(|r| r["op"] == op)
            .unwrap_or_else(|| panic!("no {op}: {recs:?}"))
            .clone()
    };
    let (mv, add, del) = (by("move"), by("add"), by("delete"));
    assert_eq!(mv["view"], wide, "the view as sent");
    assert_eq!(mv["precision"], json!("sample"));
    assert_eq!(add["precision"], json!("sample"));
    assert_eq!(del["view"], wide, "a delete carries the view too");
    assert!(del.get("precision").is_none(), "{del}");
    // a coarser zoom is `ms`
    let coarse = json!({"ms": 2000, "px_per_sample": 0.2});
    svc.events_edit_training(
        &sid,
        "k in",
        &[json!({"op": "move", "index": 0, "t": 0.7})],
        true,
        Some(coarse.clone()),
    )
    .unwrap();
    let last = records(&home).pop().unwrap();
    assert_eq!(last["precision"], json!("ms"), "{last}");
    assert_eq!(last["view"], coarse);
    // without a view: `precision: null`, no view (also over the agent surface, which reads the parameter)
    svc.events_edit_training(&sid, "k in", &[json!({"op": "move", "index": 0, "t": 0.8})], true, None)
        .unwrap();
    let last = records(&home).pop().unwrap();
    assert!(last["precision"].is_null() && last.get("view").is_none(), "{last}");
    let a = drumspike_app::call::call(
        &svc,
        "events_edit",
        &json!({"sid": sid, "name": "k in", "ops": [{"op": "move", "index": 0, "t": 0.9}], "training": true, "view": wide}),
        None,
    )
    .unwrap();
    let drumspike_app::reply::Reply::Json(a) = a else {
        panic!("not JSON")
    };
    assert_eq!(a["training"], json!({"records": 1}), "{a}");
    let last = records(&home).pop().unwrap();
    assert_eq!(last["precision"], json!("sample"), "{last}");
    assert_eq!(last["view"], wide);
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
    let b = plain.events_edit_training(&sid2, "k in", &ops, false, None).unwrap();
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
    svc.events_edit_training(&sid, "k in", &[json!({"op": "add", "t": 0.25})], true, None)
        .unwrap();
    assert_eq!(svc.training_status(&sid, None).unwrap()["records"], 1);
    assert_eq!(svc.training_status("nope", None).unwrap_err().code, 404);
    // the agent surface lists it
    assert!(drumspike_app::api::find("training_status").is_some());
    let c = drumspike_app::call::call(&svc, "training_status", &json!({"sid": sid}), None);
    assert!(c.is_ok());
}

/// A kick track with a sharp onset at 0.5 s and a line before it: silent, or white noise whose standard deviation is
/// `noise` of the peak (#363). The hit's peak is 0.5.
fn line_spec(name: &'static str, noise: f64) -> Spec {
    let mut x = vec![0.0f32; 48_000 * 2];
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let at = 24_000;
    for v in &mut x[..at] {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        // uniform noise of amplitude a has the standard deviation a / sqrt(3)
        let a = noise * 0.5 * 3.0f64.sqrt();
        *v = (a * ((seed as f64 / u64::MAX as f64) * 2.0 - 1.0)) as f32;
    }
    for k in 0..14_400 {
        let t = k as f64 / 48_000.0;
        let env = ((k + 1) as f64 / 2.0).min(1.0) * (-t / 0.06).exp();
        x[at + k] += (0.5 * env * (2.0 * std::f64::consts::PI * 180.0 * t + 0.7).sin()) as f32;
    }
    Spec {
        name,
        role: "kick",
        note: 36,
        x,
        sr: 48_000,
    }
}

fn opened_line(tag: &str, noise: f64) -> (Service, String, PathBuf, PathBuf) {
    let root = tmp(tag);
    let dir = project(&root, "Training", &[line_spec("k line", noise)]);
    let svc = service_dsp();
    let home = root.join("home");
    svc.set_home(&home);
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.events_set(&sid, "k line", &json!([{"t": 0.5, "v": 90}])).unwrap();
    (svc, sid, home, dir)
}

#[test]
fn a_move_record_carries_the_line_measures_of_a_clean_hit_and_of_a_noisy_one() {
    let (svc, sid, home, _) = opened_line("tr_pl_clean", 0.0);
    svc.events_edit_training(
        &sid,
        "k line",
        &[json!({"op": "move", "index": 0, "t": 0.6})],
        true,
        None,
    )
    .unwrap();
    let p = &records(&home)[0]["placement"];
    assert_eq!(p["edge"], true, "{p}");
    assert!(p["line_noise"].as_f64().unwrap() < 0.01, "{p}");
    assert!(p["line_slope"].is_number() && p["onset_spread"].is_null(), "{p}");

    let (svc, sid, home, _) = opened_line("tr_pl_noisy", 0.02);
    svc.events_edit_training(
        &sid,
        "k line",
        &[json!({"op": "move", "index": 0, "t": 0.6})],
        true,
        None,
    )
    .unwrap();
    let p = &records(&home)[0]["placement"];
    assert!(p["line_noise"].as_f64().unwrap() > 0.01, "{p}");
}

#[test]
fn every_edit_record_and_a_confirm_record_carry_placement() {
    let (svc, sid, home, _) = opened_line("tr_pl_all", 0.0);
    let ops = [
        json!({"op": "confirm", "index": 0}),
        json!({"op": "add", "t": 1.0}),
        json!({"op": "delete", "index": 0}),
    ];
    svc.events_edit_training(&sid, "k line", &ops, true, None).unwrap();
    let recs = records(&home);
    assert_eq!(recs.len(), 3, "{recs:?}");
    for r in &recs {
        let p = &r["placement"];
        assert!(
            p["edge"].is_boolean() && p["line_noise"].is_number() && p["line_slope"].is_number(),
            "{r}"
        );
        assert!(p.get("onset_spread").is_some(), "{r}");
    }
    let confirm = recs.iter().find(|r| r["op"] == "confirm").unwrap();
    assert_eq!(confirm["placement"]["edge"], true, "{confirm}");
    // the add reads at its `to`: silence there, so no edge
    let add = recs.iter().find(|r| r["op"] == "add").unwrap();
    assert_eq!(add["placement"]["edge"], false, "{add}");
}

#[test]
fn the_mode_off_leaves_the_answer_and_the_project_file_as_they_were() {
    let (a, sa, ha, da) = opened_line("tr_pl_off_a", 0.0);
    let (b, sb, hb, db) = opened_line("tr_pl_off_b", 0.0);
    let ops = [json!({"op": "move", "index": 0, "t": 0.6})];
    let ra = a.events_edit(&sa, "k line", &ops).unwrap();
    let rb = b.events_edit_training(&sb, "k line", &ops, false, None).unwrap();
    assert_eq!(serde_json::to_string(&ra).unwrap(), serde_json::to_string(&rb).unwrap());
    assert!(!ha.join("training").exists() && !hb.join("training").exists());
    a.project_save(&sa).unwrap();
    b.project_save(&sb).unwrap();
    let text = |d: &PathBuf| {
        let f = std::fs::read_dir(d)
            .unwrap()
            .flatten()
            .map(|f| f.path())
            .find(|p| p.extension().is_some_and(|e| e == "drumspike"))
            .unwrap();
        std::fs::read_to_string(f).unwrap()
    };
    // the log is never part of the project, with the mode on or off
    let (ta, tb) = (text(&da), text(&db));
    assert!(!ta.contains("placement") && !tb.contains("placement"));
}
