//! #265: Re-analyze Selection — the gate of the ticket (the shared rules of
//! `docs/concepts/reanalyze-selection.md`): a range run replaces only the hits inside the
//! range, leaves the hits outside bit-identical, drops a new hit closer than the minimum gap
//! to an outside hit, and the undo (the previous events written back) restores the range.

mod common;

use common::{kick_spec, project, service_dsp, wait_done};
use drumspike_app::api::find;
use serde_json::Value;

fn times(events: &[Value]) -> Vec<f64> {
    let mut t: Vec<f64> = events
        .iter()
        .filter_map(|e| e.get("t").and_then(Value::as_f64))
        .collect();
    t.sort_by(f64::total_cmp);
    t
}

/// The stored events of the track (as `events_get` answers them).
fn events_of(svc: &drumspike_app::Service, sid: &str) -> Result<Vec<Value>, drumspike_app::error::AppError> {
    Ok(svc.events_get(sid, "k in")?["events"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

/// Open the project, analyze the whole track once, and answer the events after. `tag` keeps
/// the temporary folders of the tests apart (they run in parallel threads).
fn analyzed(tag: &str) -> (drumspike_app::Service, String, String, Vec<Value>) {
    let root = common::tmp(tag);
    let dir = project(&root, "Sel", &[kick_spec("k in", "kick", 36)]);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    svc.set_projects_dir(root.join("projects"));
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    let events = events_of(&svc, &sid).unwrap();
    assert!(!events.is_empty(), "the synthetic kick track must detect its onsets");
    (svc, sid, dir.display().to_string(), events)
}

#[test]
fn a_range_run_replaces_only_the_hits_inside_and_leaves_the_outside_bit_identical() {
    let (svc, sid, _dir, before) = analyzed("reanalyze_1");
    let out_times_before: Vec<Value> = before
        .iter()
        .filter(|e| !matches!(e.get("t").and_then(Value::as_f64), Some(t) if (0.9..=2.1).contains(&t)))
        .cloned()
        .collect();
    assert!(
        !out_times_before.is_empty(),
        "the range must not hold every hit of the track"
    );

    let r = svc
        .reanalyze_selection(&sid, "k in", 0.9, 2.1, false)
        .unwrap_or_else(|e| panic!("the range run failed: {e}"));
    let after = events_of(&svc, &sid).unwrap();

    // the hits outside the range are untouched (bit-identical values)
    let out_after: Vec<Value> = after
        .iter()
        .filter(|e| !matches!(e.get("t").and_then(Value::as_f64), Some(t) if (0.9..=2.1).contains(&t)))
        .cloned()
        .collect();
    assert_eq!(
        out_after, out_times_before,
        "the hits outside the range must be untouched"
    );

    // the hits of the range were replaced: the answer names it and the info line names track and range
    let replaced = r["replaced"].as_u64().unwrap();
    let n_new = r["n_new"].as_u64().unwrap();
    assert!(replaced >= 1, "the range holds onsets of the corpus");
    assert_eq!(r["track"], "k in");
    let info = r["info"].as_str().unwrap();
    assert!(info.contains("re-analyzed 0.900 to 2.100 s"), "{info}");
    assert!(info.contains(&format!("{replaced} hits replaced by {n_new}")), "{info}");
    // the answer events are the stored ones
    assert_eq!(times(r["events"].as_array().unwrap()), times(&after));
    assert_eq!(after.len(), out_after.len() + n_new as usize);
}

#[test]
fn a_new_hit_closer_than_the_minimum_gap_to_an_outside_hit_is_dropped() {
    let (svc, sid, _dir, before) = analyzed("reanalyze_2");
    // the range starts a hair before an outside hit: a detection at the edge lands within the
    // 20 ms default minimum gap of the untouched hit and must be dropped
    let edge = before
        .iter()
        .filter_map(|e| e.get("t").and_then(Value::as_f64))
        .find(|t| *t > 0.5)
        .unwrap();
    let t0 = edge - 0.002;
    let t1 = edge + 0.05;
    let r = svc.reanalyze_selection(&sid, "k in", t0, t1, false).unwrap();
    let after = events_of(&svc, &sid).unwrap();
    let inside_new: Vec<f64> = r["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e.get("t").and_then(Value::as_f64))
        .filter(|t| (t0..=t1).contains(t))
        .collect();
    for t in &inside_new {
        let nearest = before
            .iter()
            .filter_map(|e| e.get("t").and_then(Value::as_f64))
            .filter(|o| !(t0..=t1).contains(o))
            .map(|o| (o - t).abs())
            .fold(f64::INFINITY, f64::min);
        assert!(
            nearest >= 0.02 - 1e-6,
            "a new hit at {t} is {nearest} s from an untouched hit"
        );
    }
    assert!(!after.is_empty());
}

#[test]
fn the_previous_events_written_back_restore_the_range_and_the_backup_holds_them() {
    let (svc, sid, dir, before) = analyzed("reanalyze_3");
    let _ = svc.reanalyze_selection(&sid, "k in", 0.9, 2.1, false).unwrap();
    // the undo of the page: the whole list written back, byte for byte
    svc.events_set(&sid, "k in", &Value::Array(before.clone())).unwrap();
    let restored = events_of(&svc, &sid).unwrap();
    assert_eq!(restored, before, "the undo restores the hits of the range");

    // rule 6: the backup of the previous events (the same folder shape as a whole-track run)
    let backup = std::path::Path::new(&dir).join("Backups").join("k in.events.prev.json");
    let body: Value = serde_json::from_str(&std::fs::read_to_string(&backup).unwrap()).unwrap();
    assert_eq!(times(body["events"].as_array().unwrap()), times(&before));
}

#[test]
fn a_refused_range_and_a_refused_follower_and_the_registry_row() {
    let root = common::tmp("reanalyze_bad");
    let dir = project(
        &root,
        "Sel2",
        &[kick_spec("k in", "kick", 36), kick_spec("s top", "snare", 38)],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    svc.set_projects_dir(root.join("projects"));
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    // a reversed range is a 400
    let e = svc.reanalyze_selection(&sid, "k in", 2.0, 1.0, false).unwrap_err();
    assert_eq!(e.code, 400, "{e}");
    // a follower is a 409, like events_set
    svc.track_set_follow(&sid, "s top", Some("k in")).unwrap();
    let e = svc.reanalyze_selection(&sid, "s top", 0.0, 1.0, false).unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    // the agent surface knows the command: sid, name, t0, t1 and the keep choice
    let row = find("reanalyze_selection").expect("the row");
    let names: Vec<&str> = row.args.iter().map(|a| a.name).collect();
    assert_eq!(names, ["sid", "name", "t0", "t1", "keep_edited"]);
}
