//! P3-4e: the importer of old sessions through `drumspike_app::Service`, without Tauri. Test
//! names start with the command name; the last tests check the route map rows 3 and 13 (P4-30 removed the zip import and export).
//! (The comparison with the Python server, `tools/parity/check_import.py`, went with the zip import in P4-30.)

mod common;

use std::path::{Path, PathBuf};

use common::*;
use drumspike_app::{IMPORT_COMMANDS, Service};
use drumspike_core::project::load_project;
use serde_json::{Value, json};

fn service(root: &Path) -> Service {
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    svc.set_projects_dir(root.join("projects"));
    svc
}

/// A session folder of the old server: one track `Kick_In`, events, a previous events file.
fn session(root: &Path, name: &str) -> PathBuf {
    let sdir = root.join("sessions").join("ab12cd34");
    let td = sdir.join("tracks").join("Kick_In");
    write_wav(&td.join("source.wav"), 48_000, &kick_signal(&ONSETS, 3.5, 48_000));
    std::fs::write(
        td.join("events.json"),
        json!({"sr": 48000, "duration": 3.5, "n_samples": 168000, "events": [{"t": 0.25, "v": 100}, {"t": 0.75, "v": 90, "added": true}]})
            .to_string(),
    )
    .unwrap();
    std::fs::write(td.join("events.json.prev.json"), r#"{"events": [{"t": 0.5, "v": 1}]}"#).unwrap();
    let meta = json!({"tracks": {"Kick_In": {"role": "kick", "note": 36, "sr": 48000, "n_samples": 168000,
        "duration": 3.5, "color": "#e05555", "params": {"min_gap_ms": 20}}},
        "name": name, "created": 1759950000.5, "order": ["Kick_In"]});
    std::fs::write(sdir.join("meta.json"), meta.to_string()).unwrap();
    sdir
}

fn listing(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(d: &Path, base: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            out.push(p.strip_prefix(base).unwrap().display().to_string());
            if p.is_dir() {
                walk(&p, base, out);
            }
        }
    }
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn check_imported(svc: &Service, sum: &Value, sdir: &Path) {
    assert_eq!(sum["name"], "Song: Take 2");
    assert_eq!(sum["dirty"], false);
    assert_eq!(sum["tracks"]["Kick_In"]["status"], "ok");
    let folder = PathBuf::from(sum["folder"].as_str().unwrap());
    assert!(folder.starts_with(svc.projects_dir()), "{folder:?}");
    assert!(
        folder
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("Song- Take 2")
    );
    let p = load_project(&folder).unwrap();
    assert_eq!(p.doc.format, 1);
    assert_eq!(p.doc.created.as_ref().unwrap().as_f64(), Some(1759950000.5));
    assert_eq!(p.doc.settings.as_ref().unwrap().mix_mode.as_deref(), Some("ffmpeg"));
    let k = p.doc.track("Kick_In").unwrap();
    assert_eq!(k.events().len(), 2);
    assert_eq!(k.mix_mode.as_deref(), Some("ffmpeg"));
    let rec = k.source_ref().unwrap();
    assert_eq!(rec.mode.as_deref(), Some("copied"));
    assert_eq!(rec.relative.as_deref(), Some("Audio Files/Kick_In.wav"));
    assert_eq!(
        std::fs::read(folder.join("Audio Files/Kick_In.wav")).unwrap(),
        std::fs::read(sdir.join("tracks/Kick_In/source.wav")).unwrap()
    );
    let prev: Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join("Backups/Kick_In.events.prev.json")).unwrap())
            .unwrap();
    assert_eq!(prev, json!({"events": [{"t": 0.5, "v": 1}]}));
}

// ---------- legacy_list ----------

#[test]
fn legacy_list_names_the_sessions_of_a_folder() {
    let root = tmp("legacy_list");
    let svc = service(&root);
    let sdir = session(&root, "Song: Take 2");
    std::fs::create_dir_all(root.join("sessions/no_meta")).unwrap();
    std::fs::create_dir_all(root.join("sessions/bad")).unwrap();
    std::fs::write(root.join("sessions/bad/meta.json"), "{nope").unwrap();
    let rows = svc.legacy_list(root.join("sessions").to_str().unwrap()).unwrap();
    assert_eq!(
        rows,
        json!([{"session_id": "ab12cd34", "name": "Song: Take 2", "tracks": ["Kick_In"], "dir": sdir.display().to_string()}])
    );
    // the session folder itself is the one session
    assert_eq!(
        svc.legacy_list(sdir.to_str().unwrap())
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        svc.legacy_list(root.join("none").to_str().unwrap()).unwrap_err().code,
        404
    );
    assert_eq!(svc.legacy_list("").unwrap_err().code, 404);
}

// ---------- project_import ----------

#[test]
fn project_import_of_a_session_folder_copies_audio_and_leaves_the_session_alone() {
    let root = tmp("imp_folder");
    let svc = service(&root);
    let sdir = session(&root, "Song: Take 2");
    let before = listing(&sdir);
    let sum = svc.project_import(sdir.to_str().unwrap()).unwrap();
    check_imported(&svc, &sum, &sdir);
    assert_eq!(listing(&sdir), before, "the session is only read");
    // the project is open and in the recent list
    assert_eq!(
        svc.project_get(sum["sid"].as_str().unwrap()).unwrap()["name"],
        "Song: Take 2"
    );
    assert_eq!(svc.project_recent().as_array().unwrap().len(), 1);
    // a second import makes a second folder
    let again = svc.project_import(sdir.to_str().unwrap()).unwrap();
    assert!(again["folder"].as_str().unwrap().ends_with("Song- Take 2 2"));
    assert_ne!(again["sid"], sum["sid"]);
}

#[test]
fn project_import_checks_its_arguments_and_the_documents() {
    let root = tmp("imp_args");
    let svc = service(&root);
    assert_eq!(svc.project_import("").unwrap_err().code, 400);
    assert_eq!(svc.project_import("/nonexistent/session").unwrap_err().code, 404);
    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let e = svc.project_import(empty.to_str().unwrap()).unwrap_err();
    assert!(e.code == 400 && e.message.contains("no meta.json"), "{e}");
    // a crafted track name in meta.json is refused before any path is built from it
    let sdir = session(&root, "Crafted");
    let mut meta: Value = serde_json::from_str(&std::fs::read_to_string(sdir.join("meta.json")).unwrap()).unwrap();
    meta["tracks"]["../../x"] = meta["tracks"]["Kick_In"].clone();
    std::fs::write(sdir.join("meta.json"), meta.to_string()).unwrap();
    let e = svc.project_import(sdir.to_str().unwrap()).unwrap_err();
    assert!(e.code == 400 && e.message.to_lowercase().contains("track name"), "{e}");
    assert!(!root.join("x").exists());
}

// ---------- every command has a test ----------

#[test]
fn every_import_command_has_a_test_in_import_rs() {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/import.rs")).unwrap();
    for c in IMPORT_COMMANDS {
        assert!(
            src.contains(&format!("fn {c}_")),
            "no test named {c}_... in tests/import.rs"
        );
    }
}

#[test]
fn the_bridge_runs_the_import_commands() {
    use drumspike_app::bridge::Bridge;
    use drumspike_app::reply::Reply;
    let root = tmp("bridge");
    let svc = service(&root);
    let sdir = session(&root, "Song: Take 2");
    let b = Bridge::new(svc, repo_root().join("static"));
    let Reply::Json(rows) = b
        .invoke(
            "legacy_list",
            &json!({"folder": root.join("sessions").to_str().unwrap()}),
        )
        .unwrap()
    else {
        panic!("json expected")
    };
    assert_eq!(rows[0]["session_id"], "ab12cd34");
    let Reply::Json(sum) = b
        .invoke("project_import", &json!({"session_dir": sdir.to_str().unwrap()}))
        .unwrap()
    else {
        panic!("json expected")
    };
    assert_eq!(sum["name"], "Song: Take 2");
    let e = b.invoke("project_import", &json!({})).err().unwrap();
    assert_eq!(e.code, 400);
}
