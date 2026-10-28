//! `headless::analyze_projects` (P3-11): the analysis of `service_tool analyze` and of the
//! release app's `--analyze` mode.

mod common;

use common::*;
use drumspike_app::headless::analyze_projects;

#[test]
fn analyze_projects_gives_the_events_of_events_get_per_folder() {
    let root = tmp("headless");
    let dir = project(
        &root,
        "Song",
        &[kick_spec("Kick_In", "kick", 36), kick_spec("Snare_Top", "snare", 38)],
    );
    let dir = dir.to_str().unwrap().to_string();
    let svc = service_dsp();
    let out = analyze_projects(&svc, std::slice::from_ref(&dir)).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["project"], dir.as_str());
    assert_eq!(out[0]["job"]["state"], "done");
    let sid = svc.project_open(&dir).unwrap()["sid"].as_str().unwrap().to_string();
    for name in ["Kick_In", "Snare_Top"] {
        let ev = &out[0]["events"][name];
        assert!(!ev.as_array().unwrap().is_empty(), "{name}: no events");
        assert_eq!(ev, &svc.events_get(&sid, name).unwrap()["events"]);
    }
    assert!(analyze_projects(&svc, &[]).unwrap().is_empty());
    let err = analyze_projects(&svc, &[root.join("missing").display().to_string()]).unwrap_err();
    assert!(err.contains("missing"), "{err}");
    std::fs::remove_dir_all(&root).unwrap();
}
