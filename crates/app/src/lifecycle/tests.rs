use super::*;

#[test]
fn the_lifecycle_command_list_has_no_duplicates() {
    let mut v = LIFECYCLE_COMMANDS.to_vec();
    v.sort_unstable();
    v.dedup();
    assert_eq!(v.len(), LIFECYCLE_COMMANDS.len());
}

#[test]
fn unique_folder_counts_up() {
    let d = std::env::temp_dir().join(format!("ds_life_unique_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("Untitled project")).unwrap();
    assert_eq!(unique_folder(&d, UNTITLED), "Untitled project 2");
    std::fs::create_dir_all(d.join("Untitled project 2")).unwrap();
    assert_eq!(unique_folder(&d, UNTITLED), "Untitled project 3");
    assert_eq!(unique_folder(&d, "Other"), "Other");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn read_recent_tolerates_a_missing_or_damaged_file() {
    let d = std::env::temp_dir().join(format!("ds_life_recent_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    assert!(read_recent(&d.join("none.json")).is_empty());
    std::fs::write(d.join("bad.json"), "{not json").unwrap();
    assert!(read_recent(&d.join("bad.json")).is_empty());
    std::fs::write(d.join("obj.json"), "{}").unwrap();
    assert!(read_recent(&d.join("obj.json")).is_empty());
    std::fs::write(d.join("mixed.json"), r#"[1, {"folder": "/x"}, "s"]"#).unwrap();
    assert_eq!(read_recent(&d.join("mixed.json")).len(), 1);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn state_from_env_has_an_interval_and_no_open_job() {
    let st = State::from_env();
    assert!(st.autosave_ms.load(Ordering::Relaxed) > 0);
    assert_eq!(st.lanes.status("x")["state"], "idle");
}

#[test]
fn dispatch_ignores_other_commands() {
    let svc = Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    let ch = Arc::new(Channels::default());
    assert!(dispatch(&svc, &ch, "project_open", &json!({})).is_none());
    assert!(dispatch(&svc, &ch, "project_recent", &json!({})).is_some());
    assert!(dispatch(&svc, &ch, "project_recent_clear", &json!({})).is_some());
}

#[test]
fn default_projects_dir_is_documents_drumspike_projects() {
    assert!(default_projects_dir().ends_with("Drumspike Projects"));
}

#[test]
fn json_writer_is_atomic_and_indented() {
    let d = std::env::temp_dir().join(format!("ds_life_json_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    write_json_atomic(&d.join("sub/x.json"), &json!([{"a": 1}])).unwrap();
    assert_eq!(
        std::fs::read_to_string(d.join("sub/x.json")).unwrap(),
        "[\n {\n  \"a\": 1\n }\n]"
    );
    assert!(!d.join("sub/x.json.tmp").exists());
    std::fs::remove_dir_all(&d).unwrap();
}

fn service_with_home(root: &Path) -> Service {
    let svc = Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    svc.set_home(root.join("home"));
    svc
}

/// P3-14a: the rename changes the record's key (at the same place), the order and the
/// `Analysis/` folder; the page's name field reaches this through the Inspector.
#[test]
fn track_rename_rekeys_the_record_the_order_and_the_analysis_folder() {
    use drumspike_core::project::document::Track;
    let root = std::env::temp_dir().join(format!("ds_life_rename_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = new_project(&root, "Song").unwrap().dir;
    let svc = service_with_home(&root);
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let op = svc.inner.registry.get(&sid).unwrap();
    {
        let mut g = lock(&op);
        for n in ["Kick", "Snare", "Tom"] {
            let tr = Track {
                role: Some("kick".into()),
                note: Some(36),
                ..Track::default()
            };
            g.project.doc.tracks_mut().insert(n.to_string(), tr);
        }
        g.project.doc.order = Some(vec!["Tom".into(), "Snare".into(), "Kick".into()]);
        g.mark_clean();
    }
    let adir = dir.join(ANALYSIS_DIR);
    std::fs::create_dir_all(adir.join("Snare")).unwrap();
    std::fs::write(adir.join("Snare/peaks.bin"), b"peaks").unwrap();
    std::fs::create_dir_all(adir.join("Snare Top")).unwrap(); // a stale folder under the new name goes
    let r = svc.track_rename(&sid, "Snare", " Snare Top ").unwrap();
    assert_eq!(r, json!({"name": "Snare Top"}));
    {
        let g = lock(&op);
        let keys: Vec<&String> = g.project.doc.tracks().map(|(n, _)| n).collect();
        assert_eq!(keys, ["Kick", "Snare Top", "Tom"]);
        assert_eq!(g.project.doc.order.as_deref().unwrap(), ["Tom", "Snare Top", "Kick"]);
        assert_eq!(g.track("Snare Top").unwrap().note, Some(36));
        assert!(g.dirty);
        assert_eq!(g.status_str("Snare Top"), g.status_str("Kick"));
    }
    assert_eq!(std::fs::read(adir.join("Snare Top/peaks.bin")).unwrap(), b"peaks");
    assert!(!adir.join("Snare").exists());
    // the same name again: nothing changes, no error
    assert_eq!(svc.track_rename(&sid, "Kick", "Kick").unwrap()["name"], "Kick");
    // a taken name, a bad name, an unknown track, an unknown project
    assert_eq!(svc.track_rename(&sid, "Kick", "Tom").unwrap_err().code, 409);
    assert_eq!(svc.track_rename(&sid, "Kick", "a/b").unwrap_err().code, 400);
    assert_eq!(svc.track_rename(&sid, "Kick", "..").unwrap_err().code, 400);
    assert_eq!(svc.track_rename(&sid, "Kick", "   ").unwrap_err().code, 400);
    assert_eq!(svc.track_rename(&sid, "Kick", &"x".repeat(81)).unwrap_err().code, 400);
    assert_eq!(svc.track_rename(&sid, "Nope", "Kick 2").unwrap_err().code, 400);
    assert!(svc.track_rename("00000000", "Kick", "Kick 2").is_err());
    // a running or queued analysis of the project refuses the rename (it writes under the old name)
    svc.inner.jobs.queue(&sid, &["Kick".to_string()]);
    let e = svc.track_rename(&sid, "Kick", "Kick 2").unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.contains("analysis"), "{}", e.message);
    svc.inner.jobs.clear(&sid);
    assert_eq!(svc.track_rename(&sid, "Kick", "Kick 2").unwrap()["name"], "Kick 2");
    svc.project_close(&sid, true).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
}

/// P3-14a: the app settings commands keep the other keys of `<home>/settings.json` (the
/// audio keys of P3-16) and refuse to write those keys themselves.
#[test]
fn app_settings_set_keeps_the_other_keys_and_refuses_the_audio_keys() {
    let root = std::env::temp_dir().join(format!("ds_life_settings_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let svc = service_with_home(&root);
    assert_eq!(svc.app_settings_get(), json!({}));
    std::fs::create_dir_all(root.join("home")).unwrap();
    std::fs::write(
        root.join("home/settings.json"),
        r#"{"audio_output": "AirPods", "master_db": -6.0}"#,
    )
    .unwrap();
    assert_eq!(
        svc.app_settings_set("inspector", &json!(false)).unwrap(),
        json!({"key": "inspector", "value": false})
    );
    let v = svc.app_settings_get();
    assert_eq!(v["inspector"], false);
    assert_eq!(v["audio_output"], "AirPods");
    assert_eq!(v["master_db"], -6.0);
    svc.app_settings_set("inspector", &json!(true)).unwrap();
    assert_eq!(svc.app_settings_get()["inspector"], true);
    for bad in [
        "audio_output",
        "master_db",
        "",
        "Inspector",
        "a-b",
        "has space",
        &"k".repeat(33),
    ] {
        assert_eq!(svc.app_settings_set(bad, &json!(1)).unwrap_err().code, 400, "{bad}");
    }
    assert_eq!(svc.app_settings_get()["audio_output"], "AirPods");
    // a damaged file reads as empty and is replaced by the next write
    std::fs::write(root.join("home/settings.json"), "{not json").unwrap();
    assert_eq!(svc.app_settings_get(), json!({}));
    svc.app_settings_set("inspector", &json!(true)).unwrap();
    assert_eq!(svc.app_settings_get(), json!({"inspector": true}));
    // without a home folder nothing is written and the answer is the same
    let bare = Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    assert_eq!(bare.app_settings_set("inspector", &json!(true)).unwrap()["value"], true);
    assert_eq!(bare.app_settings_get(), json!({}));
    std::fs::remove_dir_all(&root).unwrap();
}

/// P3-15e: an open and a close of the same project in parallel (both run on the blocking pool
/// since P3-15c). The open finds the open session, then the close takes it out before the open
/// answers. The test holds the lock of the recent list, so the open waits in `touch_recent`,
/// which comes after `register` and before the answer.
#[test]
fn project_open_racing_a_close_of_the_same_project_answers_an_open_session() {
    let root = std::env::temp_dir().join(format!("ds_life_open_race_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = new_project(&root, "Song").unwrap().dir;
    let svc = Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    svc.set_home(root.join("home"));
    let path = dir.display().to_string();
    let first = svc.project_open(&path).unwrap()["sid"].as_str().unwrap().to_string();
    let hold = lock(&svc.inner.life.recent);
    let opener = {
        let (svc, path) = (svc.clone(), path.clone());
        std::thread::spawn(move || svc.project_open(&path))
    };
    std::thread::sleep(Duration::from_millis(300)); // the open registers and waits for the lock
    svc.project_close(&first, true).unwrap();
    drop(hold);
    let r = opener.join().unwrap().unwrap();
    let sid = r["sid"].as_str().unwrap();
    assert!(
        svc.project_get(sid).is_ok(),
        "the open answered the closed session {sid}"
    );
    assert_ne!(sid, first);
    assert_eq!(svc.project_list(crate::ListShape::Ids).as_array().unwrap().len(), 1);
    std::fs::remove_dir_all(&root).unwrap();
}

/// #441 Clear Recent: the stored list is emptied on disk, a second clear is a no-op and a
/// project opened afterwards starts a new list.
#[test]
fn project_recent_clear_empties_the_stored_list_and_a_new_open_starts_it_again() {
    let root = std::env::temp_dir().join(format!("ds_life_recentclear_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = new_project(&root, "Song").unwrap().dir;
    let dir2 = new_project(&root, "Song 2").unwrap().dir;
    let svc = service_with_home(&root);
    svc.project_open(&dir.display().to_string()).unwrap();
    svc.project_open(&dir2.display().to_string()).unwrap();
    assert_eq!(svc.project_recent().as_array().unwrap().len(), 2);
    assert_eq!(svc.project_recent_clear()["cleared"], 2);
    assert!(svc.project_recent().as_array().unwrap().is_empty());
    assert_eq!(read_recent(&root.join("home/recent.json")).len(), 0);
    assert_eq!(svc.project_recent_clear()["cleared"], 0); // a second clear is a no-op
    let dir3 = new_project(&root, "Song 3").unwrap().dir; // an open afterwards starts a new list
    svc.project_open(&dir3.display().to_string()).unwrap();
    let recent = svc.project_recent().as_array().unwrap().clone();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0]["name"], "Song 3");
    std::fs::remove_dir_all(&root).unwrap();
}
