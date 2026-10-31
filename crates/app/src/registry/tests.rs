use super::*;
use drumspike_core::project::{add_track, new_project};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ds_app_registry_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn project_with_offline_track(dir: &std::path::Path) -> Project {
    let mut p = new_project(dir, "Reg").unwrap();
    add_track(&mut p, "kick", None, "kick", 36, None).unwrap();
    p
}

#[test]
fn register_keeps_one_entry_per_folder_and_get_finds_it() {
    let dir = tmp("reg");
    let reg = Registry::new();
    let project = project_with_offline_track(&dir);
    let a = reg.register(project.clone());
    let sid = lock(&a).sid.clone();
    assert_eq!(sid.len(), 8);
    let again = reg.register(project);
    assert!(Arc::ptr_eq(&a, &again), "the same folder keeps its entry");
    assert_eq!(reg.all().len(), 1);
    assert!(Arc::ptr_eq(&reg.get(&sid).unwrap(), &a));
    assert_eq!(reg.get("nope").unwrap_err().code, 404);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn is_open_is_true_only_while_the_entry_is_registered() {
    let dir = tmp("is_open");
    let reg = Registry::new();
    let project = project_with_offline_track(&dir);
    let a = reg.register(project.clone());
    let sid = lock(&a).sid.clone();
    assert!(reg.is_open(&a));
    reg.remove(&sid).unwrap();
    assert!(!reg.is_open(&a), "a closed project is not open");
    // a new entry of the same folder is another session: the old handle stays closed
    let b = reg.register(project);
    assert!(!Arc::ptr_eq(&a, &b));
    assert!(reg.is_open(&b));
    assert!(!reg.is_open(&a));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_track_without_a_source_is_offline_and_conflicts() {
    let dir = tmp("off");
    let op = OpenProject::new("abcd1234".into(), project_with_offline_track(&dir));
    assert!(op.offline("kick"));
    assert_eq!(op.status_str("kick"), "missing");
    assert_eq!(op.source("kick").unwrap_err().code, 409);
    assert!(op.source("kick").unwrap_err().message.contains("offline (missing)"));
    assert_eq!(op.source("nope").unwrap_err(), AppError::bad("unknown track"));
    assert_eq!(op.status_str("nope"), "missing");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn summary_and_public_track_have_the_server_keys() {
    let dir = tmp("sum");
    let mut op = OpenProject::new("abcd1234".into(), project_with_offline_track(&dir));
    op.mark_dirty();
    let s = op.summary();
    let keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "sid", "name", "folder", "document", "dirty", "format", "created", "modified", "settings", "order",
            "tracks"
        ]
    );
    assert_eq!(s["dirty"], true);
    assert_eq!(s["order"], json!(["kick"]));
    assert_eq!(s["tracks"]["kick"]["status"], "missing");
    assert_eq!(s["tracks"]["kick"]["source"], Value::Null);
    let p = op.public_track("kick").unwrap();
    assert!(p.get("events").is_none() && p.get("source").is_none());
    assert_eq!(p["n_events"], 0);
    assert_eq!(p["offline"], true);
    assert_eq!(p["source_mode"], Value::Null);
    assert!(op.mtime() > 0.0, "the created time counts before the first save");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dirty_changes_send_one_event_each_and_start_the_autosave_clock() {
    let dir = tmp("dirtyev");
    let reg = Registry::new();
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let f: Emitter = Arc::new(move |name, v| {
        assert_eq!(name, "project-dirty");
        lock(&sink).push(v);
    });
    reg.emitter.set(f);
    let op = reg.register(project_with_offline_track(&dir));
    let sid = lock(&op).sid.clone();
    {
        let mut g = lock(&op);
        assert!(g.dirty_since.is_none());
        g.mark_dirty();
        g.mark_dirty();
        assert!(g.dirty && g.dirty_since.is_some());
        g.mark_clean();
        g.mark_clean();
        assert!(!g.dirty && g.dirty_since.is_none() && g.last_autosave.is_none());
    }
    assert_eq!(
        *lock(&seen),
        [json!({"sid": sid, "dirty": true}), json!({"sid": sid, "dirty": false})]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn remove_takes_a_project_out_and_unknown_ids_are_404() {
    let dir = tmp("remove");
    let reg = Registry::new();
    let op = reg.register(project_with_offline_track(&dir));
    let sid = lock(&op).sid.clone();
    assert!(Arc::ptr_eq(&reg.remove(&sid).unwrap(), &op));
    assert_eq!(reg.all().len(), 0);
    assert_eq!(reg.remove(&sid).unwrap_err().code, 404);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// P3-15c: the light track object and document head equal the full serialisation minus the
/// keys they leave out, in the same key order.
#[test]
fn light_track_object_and_doc_head_equal_the_full_serialisation() {
    let dir = tmp("light");
    let mut op = OpenProject::new("abcd1234".into(), project_with_offline_track(&dir));
    {
        let tr = op.track_mut("kick").unwrap();
        tr.events = Some(vec![
            drumspike_core::Event::new(0.5, 90),
            drumspike_core::Event::new(1.0, 100),
        ]);
        tr.extra.insert("zz_unknown".into(), json!({"a": 1}));
        tr.params = Some(serde_json::from_value(json!({"min_gap_ms": 30})).unwrap());
    }
    op.project.doc.extra.insert("future_key".into(), json!([1, 2]));
    let full = |tr: &Track| {
        let Value::Object(mut m) = serde_json::to_value(tr).unwrap() else {
            unreachable!()
        };
        m.shift_remove("events");
        m.shift_remove("source");
        m
    };
    let tr = op.track("kick").unwrap().clone();
    let light = track_object_light(&tr).unwrap();
    assert_eq!(light, full(&tr));
    assert_eq!(
        light.keys().collect::<Vec<_>>(),
        full(&tr).keys().collect::<Vec<_>>(),
        "same key order"
    );
    // an untyped event list in `extra` is left out too
    let mut untyped = tr.clone();
    untyped.events = None;
    untyped.extra.insert("events".into(), json!([{"x": 1}]));
    assert_eq!(track_object_light(&untyped).unwrap(), full(&untyped));
    let dv = serde_json::to_value(&op.project.doc).unwrap();
    let head = doc_head_value(&op.project.doc);
    for k in ["format", "name", "created", "modified", "settings", "future_key"] {
        assert_eq!(head.get(k), dv.get(k), "{k}");
    }
    assert!(head.get("tracks").is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn resolve_shared_finds_the_sources_like_resolve_and_records_lists_them() {
    let dir = tmp("shared");
    let mut p = new_project(&dir, "Reg").unwrap();
    let wav = dir.join("k.wav");
    std::fs::write(&wav, b"RIFF0000WAVEdata").unwrap();
    let rec = drumspike_core::project::build_source(&wav, &p.dir, "referenced").unwrap();
    add_track(&mut p, "kick", Some(rec.clone()), "kick", 36, None).unwrap();
    add_track(&mut p, "snare", None, "snare", 38, None).unwrap();
    let reg = Registry::new();
    let op = reg.register(p);
    let (d, recs) = lock(&op).records();
    assert_eq!(d, lock(&op).project.dir);
    assert_eq!(recs, vec![("kick".to_string(), Some(rec)), ("snare".to_string(), None)]);
    resolve_shared(&op);
    assert_eq!(lock(&op).status_str("kick"), "ok");
    assert_eq!(lock(&op).status_str("snare"), "missing");
    assert_eq!(lock(&op).paths["kick"], Some(wav.clone()));
    std::fs::remove_file(&wav).unwrap();
    resolve_shared(&op);
    let mut g = lock(&op);
    assert_eq!(g.status_str("kick"), "missing");
    let (paths, status) = (g.paths.clone(), g.status.clone());
    g.resolve();
    assert_eq!(
        (g.paths.clone(), g.status.clone()),
        (paths, status),
        "the same as resolve"
    );
    drop(g);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn n_events_counts_an_untyped_list_too() {
    let dir = tmp("untyped");
    let mut op = OpenProject::new("abcd1234".into(), project_with_offline_track(&dir));
    let tr = op.track_mut("kick").unwrap();
    tr.events = None;
    tr.extra.insert("events".into(), json!([{"x": 1}, {"x": 2}]));
    assert_eq!(op.n_events("kick"), 2);
    assert_eq!(op.n_events("nope"), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}
