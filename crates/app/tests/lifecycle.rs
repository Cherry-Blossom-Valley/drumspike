//! P3-4a: one or more tests per lifecycle command, the dirty event, the autosave and the lane
//! preparation, through `drumspike_app::Service` without Tauri. Test names start with the command
//! name; `command_tests.rs` checks that.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::file_sha256;
use common::*;
use drumspike_app::bridge::Bridge;
use drumspike_app::reply::Reply;
use drumspike_app::{Progress, Service};
use drumspike_core::project::{BACKUPS_DIR, add_track, build_source, load_project, save_project};
use serde_json::{Value, json};

/// Open a two-track project; returns (service, sid, folder, root). The service has a home folder.
fn opened(tag: &str) -> (Service, String, PathBuf, PathBuf) {
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
    (svc, sid, dir, root)
}

fn keys(v: &Value) -> Vec<&str> {
    let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    k.sort_unstable();
    k
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir.join(BACKUPS_DIR))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "drumspike"))
        .collect();
    v.sort();
    v
}

fn is_dirty(svc: &Service, sid: &str) -> bool {
    svc.project_get(sid).unwrap()["dirty"].as_bool().unwrap()
}

// ---------- project_new ----------

#[test]
fn project_new_creates_the_layout_and_opens_the_project() {
    let root = tmp("new_layout");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let v = svc
        .project_new(Some("Drum Song 03"), Some(root.to_str().unwrap()), None)
        .unwrap();
    let folder = root.join("Drum Song 03");
    for d in ["Audio Files", "Analysis", "Exports", "Backups"] {
        assert!(folder.join(d).is_dir(), "{d}");
    }
    assert!(folder.join("Drum Song 03.drumspike").is_file());
    assert_eq!(v["name"], "Drum Song 03");
    assert_eq!(v["dirty"], false);
    assert_eq!(v["order"], json!([]));
    assert_eq!(v["folder"], folder.display().to_string());
    assert_eq!(
        svc.project_get(v["sid"].as_str().unwrap()).unwrap()["name"],
        "Drum Song 03"
    );
    // the same key set as project_open
    let opened = svc.project_open(folder.to_str().unwrap()).unwrap();
    assert_eq!(keys(&v), keys(&opened));
    assert_eq!(opened["sid"], v["sid"], "the folder keeps its id");
    // a new project is clean, so a closing prompt would not show; the document has no backup yet
    assert_eq!(backups(&folder).len(), 0);
}

#[test]
fn project_new_keeps_the_audio_mode_in_the_settings() {
    let root = tmp("new_mode");
    let svc = service_dsp();
    let dir = root.to_str().unwrap();
    svc.project_new(Some("A"), Some(dir), None).unwrap();
    svc.project_new(Some("B"), Some(dir), Some("copied")).unwrap();
    let mode = |n: &str| {
        let p = load_project(&root.join(n)).unwrap();
        let s = serde_json::to_value(&p.doc).unwrap();
        (s["settings"]["audio_mode"].clone(), s["settings"]["mix_mode"].clone())
    };
    assert_eq!(mode("A"), (json!("referenced"), json!("average")));
    assert_eq!(mode("B"), (json!("copied"), json!("average")));
    let e = svc.project_new(Some("C"), Some(dir), Some("link")).unwrap_err();
    assert_eq!(e.code, 400);
    assert!(!root.join("C").exists(), "a refused call creates nothing");
}

#[test]
fn project_new_without_a_name_makes_untitled_project_in_the_default_folder() {
    let root = tmp("new_untitled");
    let svc = service_dsp();
    svc.set_projects_dir(&root);
    let a = svc.project_new(None, None, None).unwrap();
    let b = svc.project_new(Some("  "), None, None).unwrap();
    assert_eq!(a["name"], "Untitled project");
    assert_eq!(b["name"], "Untitled project 2");
    assert!(root.join("Untitled project/Untitled project.drumspike").is_file());
    assert!(root.join("Untitled project 2/Untitled project 2.drumspike").is_file());
    assert_ne!(a["sid"], b["sid"]);
}

#[test]
fn project_new_refuses_a_bad_or_taken_name() {
    let root = tmp("new_bad");
    let svc = service_dsp();
    let dir = root.to_str().unwrap();
    for bad in ["a/b", "a\\b", "Song: Take 2", ".."] {
        let e = svc.project_new(Some(bad), Some(dir), None).unwrap_err();
        assert_eq!(e.code, 400, "{bad}");
        assert!(e.message.contains("bad project name"), "{e}");
    }
    svc.project_new(Some("Taken"), Some(dir), None).unwrap();
    let e = svc.project_new(Some("Taken"), Some(dir), None).unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.contains("already exists"), "{e}");
}

// ---------- project_clean_check (P4-40a) ----------

#[test]
fn project_clean_check_clears_the_flag_only_when_the_document_is_the_saved_one() {
    let (svc, sid, _dir, _root) = opened("clean_check");
    // a clean project stays clean
    assert_eq!(svc.project_clean_check(&sid).unwrap(), json!({"dirty": false}));
    // an edit and its reverse: the document is the opened one again
    let was = svc.events_get(&sid, "Kick_In").unwrap();
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.5, "v": 90}])).unwrap();
    assert!(is_dirty(&svc, &sid));
    assert_eq!(
        svc.project_clean_check(&sid).unwrap(),
        json!({"dirty": true}),
        "a changed document stays dirty"
    );
    assert!(is_dirty(&svc, &sid));
    svc.events_set(&sid, "Kick_In", &was["events"]).unwrap();
    assert!(
        is_dirty(&svc, &sid),
        "an edit marks the project dirty, the check clears it"
    );
    assert_eq!(svc.project_clean_check(&sid).unwrap(), json!({"dirty": false}));
    assert!(!is_dirty(&svc, &sid));
    // after a save the saved document is the baseline
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.5, "v": 90}])).unwrap();
    svc.project_save(&sid).unwrap();
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.7, "v": 90}])).unwrap();
    assert_eq!(svc.project_clean_check(&sid).unwrap()["dirty"], true);
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.5, "v": 90}])).unwrap();
    assert_eq!(
        svc.project_clean_check(&sid).unwrap()["dirty"],
        false,
        "back to the saved state"
    );
    assert_eq!(svc.project_clean_check("nope").unwrap_err().code, 404);
}

// ---------- project_save ----------

#[test]
fn project_save_writes_the_document_clears_dirty_and_keeps_a_backup() {
    let (svc, sid, dir, _root) = opened("save_basic");
    let doc = dir.join("Song.drumspike");
    let before = file_sha256(&doc).unwrap();
    let n_backups = backups(&dir).len();
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.5, "v": 90}])).unwrap();
    assert!(is_dirty(&svc, &sid));
    let r = svc.project_save(&sid).unwrap();
    assert_eq!(keys(&r), ["dirty", "saved", "warning"]);
    assert_eq!(r["dirty"], false);
    assert_eq!(r["saved"], doc.display().to_string());
    assert!(!is_dirty(&svc, &sid));
    assert_ne!(file_sha256(&doc).unwrap(), before);
    let reread = load_project(&dir).unwrap();
    assert_eq!(reread.doc.track("Kick_In").unwrap().events().len(), 1);
    assert_eq!(
        backups(&dir).len(),
        n_backups + 1,
        "the previous document went to Backups/"
    );
    assert_eq!(svc.project_save("nope").unwrap_err().code, 404);
}

/// R12: a backup-rotation failure after the document is renamed into place is a warning in the
/// save answer, not a failed save: the baseline, dirty flag and disk stamp move, and the next
/// save is not classified as a stale-write conflict. The injection: a `.drumspike` directory
/// older than ten real backups — the rotation counts it as a document candidate and
/// `remove_file` on a directory fails on every platform.
#[test]
fn project_save_answers_a_failed_backup_rotation_as_a_warning_and_moves_the_baseline() {
    let (svc, sid, dir, _root) = opened("save_rot_fail");
    let doc = dir.join("Song.drumspike");
    let bdir = dir.join(BACKUPS_DIR);
    let stuck = bdir.join("aaa-stuck.drumspike");
    std::fs::create_dir(&stuck).unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    std::fs::File::open(&stuck)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
    for i in 0..10 {
        std::fs::write(bdir.join(format!("old {i} 2000-01-01 00-00-00.drumspike")), b"{}").unwrap();
    }
    let before = file_sha256(&doc).unwrap();
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.5, "v": 90}])).unwrap();
    let r = svc.project_save(&sid).unwrap();
    assert_eq!(keys(&r), ["dirty", "saved", "warning"]);
    let warn = r["warning"].as_str().unwrap();
    assert!(warn.contains("aaa-stuck.drumspike"), "{warn}");
    assert_eq!(r["saved"], doc.display().to_string());
    assert!(!is_dirty(&svc, &sid), "the committed save moved the baseline");
    assert_ne!(file_sha256(&doc).unwrap(), before, "the document is on disk");
    let reread = load_project(&dir).unwrap();
    assert_eq!(reread.doc.track("Kick_In").unwrap().events().len(), 1);
    // the next save is this process's own write, not a stale-write conflict
    let r2 = svc.project_save(&sid).unwrap();
    assert_eq!(r2["warning"].as_str().unwrap(), warn, "the injection is still there");
    assert_eq!(r2["dirty"], false);
    assert!(stuck.is_dir(), "the injection is untouched by the saves");
}

/// P3-15c: the save writes from a copy of the project without the project lock. An edit that comes
/// while the files are written must not be lost: the file then holds the events of the save's copy,
/// and the project stays dirty with the new events; without an edit the project is clean and the
/// file holds what the project holds.
///
/// P4-43: the edit can also come after the save cleared the flag (the file then misses it too). The
/// answer is judged by the order of the `project-dirty` events, which are sent under the project lock:
/// an answer `false` needs the save's `dirty:false` before any `dirty:true` of the edit; an answer `true`
/// means the save did not clear the flag, so no `dirty:false` comes.
#[test]
fn project_save_keeps_the_project_dirty_when_an_edit_comes_during_the_write() {
    let (svc, sid, dir, _root) = opened("save_race");
    let seen: Arc<Mutex<Vec<bool>>> = Arc::default();
    let sink = Arc::clone(&seen);
    svc.set_emitter(Arc::new(move |name, v| {
        if name == "project-dirty" {
            sink.lock().unwrap().push(v["dirty"].as_bool().unwrap());
        }
    }));
    let mut raced = 0;
    for round in 0..40 {
        let want = json!([{"t": 0.25 + round as f64 * 0.01, "v": 90}]);
        svc.events_set(&sid, "Kick_In", &json!([{"t": 0.1, "v": 80}])).unwrap();
        assert!(is_dirty(&svc, &sid));
        seen.lock().unwrap().clear();
        let s2 = svc.clone();
        let sid2 = sid.clone();
        let w = want.clone();
        let edit = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_micros(200 * (round % 8)));
            s2.events_set(&sid2, "Kick_In", &w).unwrap();
        });
        let r = svc.project_save(&sid).unwrap();
        edit.join().unwrap();
        let mem = svc.events_get(&sid, "Kick_In").unwrap()["events"].clone();
        let file = load_project(&dir).unwrap();
        let on_disk: Vec<f64> = file
            .doc
            .track("Kick_In")
            .unwrap()
            .events()
            .iter()
            .map(|e| e.t)
            .collect();
        let in_mem: Vec<f64> = mem
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["t"].as_f64().unwrap())
            .collect();
        assert_eq!(in_mem, [want[0]["t"].as_f64().unwrap()], "the edit is kept in memory");
        let events = std::mem::take(&mut *seen.lock().unwrap());
        if r["dirty"] == false {
            assert_eq!(
                events.first(),
                Some(&false),
                "round {round}: an answer `dirty: false` needs the save's dirty:false before the edit's dirty:true"
            );
        } else {
            assert_eq!(r["dirty"], true);
            assert!(
                !events.contains(&false),
                "round {round}: an answer `dirty: true` means the save did not clear the flag"
            );
        }
        if on_disk != in_mem {
            raced += 1;
            assert!(
                is_dirty(&svc, &sid),
                "round {round}: the file misses the edit, so the project must stay dirty"
            );
        }
        // a save without a concurrent edit makes file and memory equal and the project clean
        let r = svc.project_save(&sid).unwrap();
        assert_eq!(r["dirty"], false);
        assert!(!is_dirty(&svc, &sid));
        let again = load_project(&dir).unwrap();
        let on_disk: Vec<f64> = again
            .doc
            .track("Kick_In")
            .unwrap()
            .events()
            .iter()
            .map(|e| e.t)
            .collect();
        assert_eq!(on_disk, in_mem);
    }
    eprintln!("rounds with an edit during the write: {raced} of 40");
}

#[test]
fn project_save_keeps_ten_backups() {
    let (svc, sid, dir, _root) = opened("save_rotate");
    for k in 0..14 {
        svc.track_set_note(&sid, "Kick_In", 30 + k).unwrap();
        svc.project_save(&sid).unwrap();
    }
    assert_eq!(backups(&dir).len(), 10);
}

/// #121: another writer (a second instance, a sync tool, a restored backup) puts its own document in place: the
/// bytes differ and so does the size.
fn write_outside(doc: &Path) -> Vec<u8> {
    let mut v: Value = serde_json::from_slice(&std::fs::read(doc).unwrap()).unwrap();
    v["outside_writer"] = json!("restored from a backup by hand");
    let bytes = serde_json::to_vec_pretty(&v).unwrap();
    std::fs::write(doc, &bytes).unwrap();
    bytes
}

#[test]
fn project_save_refuses_a_document_that_changed_on_disk_and_leaves_the_file_unchanged() {
    let (svc, sid, dir, _root) = opened("save_stale");
    let doc = dir.join("Song.drumspike");
    svc.track_set_note(&sid, "Kick_In", 41).unwrap();
    let outside = write_outside(&doc);
    let n_backups = backups(&dir).len();
    let e = svc.project_save(&sid).unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    assert!(e.message.starts_with(drumspike_app::registry::CHANGED_ON_DISK), "{e}");
    assert!(
        e.message.contains("Song.drumspike") && e.message.contains("Save anyway"),
        "{e}"
    );
    assert_eq!(std::fs::read(&doc).unwrap(), outside, "the outside content stays");
    assert_eq!(
        backups(&dir).len(),
        n_backups,
        "nothing is written, not even the pre-save copy"
    );
    assert!(is_dirty(&svc, &sid), "the edit is still unsaved");
    // it holds until the user chooses: a second Save is refused too
    assert_eq!(svc.project_save(&sid).unwrap_err().code, 409);
}

#[test]
fn project_save_with_force_writes_over_a_document_that_changed_on_disk() {
    let (svc, sid, dir, _root) = opened("save_force");
    let doc = dir.join("Song.drumspike");
    svc.track_set_note(&sid, "Kick_In", 42).unwrap();
    write_outside(&doc);
    assert_eq!(svc.project_save(&sid).unwrap_err().code, 409);
    let before = backups(&dir);
    let r = svc.project_save_with(&sid, true).unwrap();
    assert_eq!(r["dirty"], false);
    let v: Value = serde_json::from_slice(&std::fs::read(&doc).unwrap()).unwrap();
    assert_eq!(v["tracks"]["Kick_In"]["note"], 42);
    assert!(
        v.get("outside_writer").is_none(),
        "Save anyway writes the open document"
    );
    // the new backup, not the last by name: in the same second "Song <stamp> (2)" sorts before "Song <stamp>"
    let added: Vec<PathBuf> = backups(&dir).into_iter().filter(|p| !before.contains(p)).collect();
    assert_eq!(added.len(), 1, "one pre-save copy: {added:?}");
    let newest: Value = serde_json::from_slice(&std::fs::read(&added[0]).unwrap()).unwrap();
    assert!(
        newest.get("outside_writer").is_some(),
        "the outside document is the pre-save copy in Backups/"
    );
    // the file is ours again: the next Save needs no force
    svc.track_set_note(&sid, "Kick_In", 43).unwrap();
    svc.project_save(&sid).unwrap();
}

#[test]
fn project_save_of_an_unchanged_document_saves_as_before() {
    let (svc, sid, dir, _root) = opened("save_unchanged");
    let doc = dir.join("Song.drumspike");
    svc.project_save(&sid).unwrap(); // clean
    for k in 0..3 {
        svc.track_set_note(&sid, "Kick_In", 50 + k).unwrap();
        let r = svc.project_save(&sid).unwrap();
        assert_eq!(r["dirty"], false, "round {k}");
    }
    let v: Value = serde_json::from_slice(&std::fs::read(&doc).unwrap()).unwrap();
    assert_eq!(v["tracks"]["Kick_In"]["note"], 52);
    // a document that is gone is no change: the save writes it again
    std::fs::remove_file(&doc).unwrap();
    svc.track_set_note(&sid, "Kick_In", 60).unwrap();
    svc.project_save(&sid).unwrap();
    assert!(doc.is_file());
}

#[test]
fn project_save_through_the_dispatcher_takes_force() {
    let (svc, sid, dir, _root) = opened("save_dispatch");
    write_outside(&dir.join("Song.drumspike"));
    let ch = Arc::new(drumspike_app::reply::Channels::default());
    let run = |args: Value| drumspike_app::lifecycle::dispatch(&svc, &ch, "project_save", &args).unwrap();
    assert_eq!(run(json!({"sid": sid})).unwrap_err().code, 409);
    assert_eq!(run(json!({"sid": sid, "force": false})).unwrap_err().code, 409);
    assert_eq!(run(json!({"sid": sid, "force": true})).unwrap()["dirty"], false);
}

#[test]
fn project_save_as_takes_the_new_document_as_its_own() {
    let (svc, sid, dir, root) = opened("save_as_stale");
    write_outside(&dir.join("Song.drumspike"));
    svc.project_save_as(&sid, Some("Song Copy"), Some(root.to_str().unwrap()), false, None)
        .unwrap();
    svc.track_set_note(&sid, "Kick_In", 44).unwrap();
    svc.project_save(&sid).unwrap();
}

// ---------- project_save_as ----------

#[test]
fn project_save_as_writes_a_new_project_with_referenced_audio_under_the_same_id() {
    let (svc, sid, dir, root) = opened("saveas_ok");
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.25, "v": 80}])).unwrap();
    let out = root.join("elsewhere");
    let v = svc
        .project_save_as(&sid, Some("Song B"), Some(out.to_str().unwrap()), false, None)
        .unwrap();
    let new_dir = out.join("Song B");
    assert_eq!(v["sid"], sid.as_str());
    assert_eq!(v["name"], "Song B");
    assert_eq!(v["folder"], new_dir.display().to_string());
    assert_eq!(v["dirty"], false);
    assert!(new_dir.join("Song B.drumspike").is_file());
    let p = load_project(&new_dir).unwrap();
    assert_eq!(
        p.doc.track("Kick_In").unwrap().events().len(),
        1,
        "the unsaved edit is in the copy"
    );
    for (_, tr) in p.doc.tracks() {
        assert_eq!(tr.source_ref().unwrap().mode.as_deref(), Some("referenced"));
    }
    assert_eq!(v["tracks"]["Kick_In"]["status"], "ok");
    // the old folder keeps what it had on disk
    assert_eq!(
        load_project(&dir).unwrap().doc.track("Kick_In").unwrap().events().len(),
        0
    );
    assert_eq!(svc.project_get(&sid).unwrap()["folder"], new_dir.display().to_string());
}

#[test]
fn project_save_as_refuses_a_taken_folder_a_bad_name_and_an_unknown_project_for_copied_audio_too() {
    let (svc, sid, _dir, root) = opened("saveas_bad");
    let out = root.join("out");
    let d = out.to_str().unwrap();
    // P3-4d: copied audio is a job now; the checks that come before the job answer at once
    assert_eq!(
        svc.project_save_as(&sid, None, Some(d), true, None).unwrap_err().code,
        400
    );
    assert_eq!(
        svc.project_save_as(&sid, Some("a/b"), Some(d), true, None)
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        svc.project_save_as("nope", Some("X"), Some(d), true, None)
            .unwrap_err()
            .code,
        404
    );
    assert!(!out.exists(), "nothing is written");
    assert_eq!(
        svc.project_save_as(&sid, None, Some(d), false, None).unwrap_err().code,
        400
    );
    assert_eq!(
        svc.project_save_as(&sid, Some("a/b"), Some(d), false, None)
            .unwrap_err()
            .code,
        400
    );
    svc.project_save_as(&sid, Some("Twice"), Some(d), false, None).unwrap();
    let e = svc
        .project_save_as(&sid, Some("Twice"), Some(d), false, None)
        .unwrap_err();
    assert_eq!(e.code, 409);
    assert_eq!(
        svc.project_save_as("nope", Some("Y"), Some(d), false, None)
            .unwrap_err()
            .code,
        404
    );
}

#[test]
fn project_save_as_defaults_to_the_projects_folder() {
    let (svc, sid, _dir, root) = opened("saveas_default");
    svc.set_projects_dir(root.join("projects"));
    let v = svc.project_save_as(&sid, Some("Fresh"), None, false, None).unwrap();
    assert_eq!(v["folder"], root.join("projects/Fresh").display().to_string());
}

// ---------- project_close ----------

#[test]
fn project_close_is_409_while_dirty_unless_forced() {
    let (svc, sid, dir, _root) = opened("close_dirty");
    svc.track_set_note(&sid, "Kick_In", 40).unwrap();
    let e = svc.project_close(&sid, false).unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.contains("unsaved changes"), "{e}");
    assert!(svc.project_get(&sid).is_ok(), "still open");
    let r = svc.project_close(&sid, true).unwrap();
    assert_eq!(keys(&r), ["closed", "folder"]);
    assert_eq!(r["closed"], sid.as_str());
    assert_eq!(r["folder"], dir.display().to_string());
    assert_eq!(svc.project_get(&sid).unwrap_err().code, 404);
    // the unsaved edit is gone from the disk too: the document was not written
    assert_eq!(load_project(&dir).unwrap().doc.track("Kick_In").unwrap().note, Some(36));
}

#[test]
fn project_close_of_a_clean_project_works_and_clears_the_jobs() {
    let (svc, sid, _dir, _root) = opened("close_clean");
    svc.project_save(&sid).unwrap();
    svc.peaks_build_start(&sid, None, None).unwrap();
    wait_lanes(&svc, &sid);
    let r = svc.project_close(&sid, false).unwrap();
    assert_eq!(r["closed"], sid.as_str());
    assert_eq!(svc.project_list(drumspike_app::ListShape::Ids), json!([]));
    assert_eq!(svc.analyze_status(&sid)["state"], "idle");
    assert_eq!(svc.peaks_build_status(&sid)["state"], "idle");
    assert_eq!(svc.project_close(&sid, true).unwrap_err().code, 404);
}

// ---------- project_rename ----------

#[test]
fn project_rename_changes_the_display_name_and_marks_dirty() {
    let (svc, sid, dir, _root) = opened("rename_ok");
    let r = svc.project_rename(&sid, "  New Name  ").unwrap();
    assert_eq!(r, json!({"name": "New Name"}));
    assert!(is_dirty(&svc, &sid));
    assert_eq!(svc.project_get(&sid).unwrap()["name"], "New Name");
    assert_eq!(svc.project_get(&sid).unwrap()["folder"], dir.display().to_string());
    svc.project_save(&sid).unwrap();
    let p = load_project(&dir).unwrap();
    assert_eq!(p.name(), "New Name");
    assert_eq!(
        p.path(),
        dir.join("Song.drumspike"),
        "the folder and the document keep their names"
    );
}

#[test]
fn project_rename_cuts_to_80_characters_and_refuses_bad_names() {
    let (svc, sid, _dir, _root) = opened("rename_bad");
    let long = "é".repeat(100);
    let r = svc.project_rename(&sid, &long).unwrap();
    assert_eq!(r["name"].as_str().unwrap().chars().count(), 80);
    for bad in ["", "   ", "a/b", "x:y", ".."] {
        assert_eq!(svc.project_rename(&sid, bad).unwrap_err().code, 400, "{bad:?}");
    }
    assert_eq!(svc.project_rename("nope", "x").unwrap_err().code, 404);
}

// ---------- tracks_reorder ----------

#[test]
fn tracks_reorder_stores_the_order_and_marks_dirty() {
    let (svc, sid, dir, _root) = opened("reorder_ok");
    let r = svc.tracks_reorder(&sid, &json!(["Snare_Top", "Kick_In"])).unwrap();
    assert_eq!(r, json!({"order": ["Snare_Top", "Kick_In"]}));
    assert!(is_dirty(&svc, &sid));
    assert_eq!(svc.tracks_get(&sid).unwrap()["order"], json!(["Snare_Top", "Kick_In"]));
    svc.project_save(&sid).unwrap();
    assert_eq!(load_project(&dir).unwrap().doc.track_order(), ["Snare_Top", "Kick_In"]);
}

#[test]
fn tracks_reorder_needs_a_permutation_of_the_track_names() {
    let (svc, sid, _dir, _root) = opened("reorder_bad");
    for bad in [
        json!(["Kick_In"]),
        json!(["Kick_In", "Kick_In"]),
        json!(["Kick_In", "Snare_Top", "Extra"]),
        json!([1, 2]),
        json!("Kick_In"),
        Value::Null,
    ] {
        assert_eq!(svc.tracks_reorder(&sid, &bad).unwrap_err().code, 400, "{bad}");
    }
    assert!(!is_dirty(&svc, &sid), "a refused call changes nothing");
}

// ---------- track_set_note ----------

#[test]
fn track_set_note_sets_the_note_and_marks_dirty() {
    let (svc, sid, dir, _root) = opened("note_ok");
    assert_eq!(svc.track_set_note(&sid, "Kick_In", 35).unwrap(), json!({"note": 35}));
    assert!(is_dirty(&svc, &sid));
    assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"]["Kick_In"]["note"], 35);
    svc.project_save(&sid).unwrap();
    assert_eq!(load_project(&dir).unwrap().doc.track("Kick_In").unwrap().note, Some(35));
}

#[test]
fn track_set_note_refuses_an_unknown_track_and_a_note_out_of_range() {
    let (svc, sid, _dir, _root) = opened("note_bad");
    assert_eq!(svc.track_set_note(&sid, "Nope", 36).unwrap_err().code, 400);
    assert_eq!(svc.track_set_note(&sid, "Kick_In", 128).unwrap_err().code, 400);
    assert_eq!(svc.track_set_note(&sid, "Kick_In", -1).unwrap_err().code, 400);
    assert_eq!(svc.track_set_note("nope", "Kick_In", 36).unwrap_err().code, 404);
    assert!(!is_dirty(&svc, &sid));
}

// ---------- track_set_follow (P3-18; the derivation itself is in tests/follow.rs) ----------

#[test]
fn track_set_follow_sets_and_clears_the_follow_and_marks_dirty() {
    let (svc, sid, dir, _root) = opened("follow_set");
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.25, "v": 100}]))
        .unwrap();
    svc.project_save(&sid).unwrap();
    let r = svc.track_set_follow(&sid, "Snare_Top", Some("Kick_In")).unwrap();
    assert_eq!(r, json!({"follows": "Kick_In", "n_events": 1}));
    assert!(is_dirty(&svc, &sid));
    assert_eq!(
        svc.tracks_get(&sid).unwrap()["tracks"]["Snare_Top"]["follows"],
        "Kick_In"
    );
    svc.project_save(&sid).unwrap();
    assert_eq!(
        load_project(&dir)
            .unwrap()
            .doc
            .track("Snare_Top")
            .unwrap()
            .follows
            .as_deref(),
        Some("Kick_In")
    );
    assert_eq!(
        svc.track_set_follow(&sid, "Snare_Top", None).unwrap(),
        json!({"follows": null, "n_events": 1})
    );
    assert!(is_dirty(&svc, &sid));
    assert_eq!(
        svc.track_set_follow(&sid, "Nope", Some("Kick_In")).unwrap_err().code,
        400
    );
    assert_eq!(
        svc.track_set_follow("nope", "Snare_Top", Some("Kick_In"))
            .unwrap_err()
            .code,
        404
    );
}

// ---------- track_rename, app_settings_get, app_settings_set (P3-14a Inspector) ----------

#[test]
fn track_rename_rekeys_the_track_its_order_and_its_analysis_folder_and_marks_dirty() {
    let (svc, sid, dir, _root) = opened("track_rename_ok");
    let adir = dir.join("Analysis");
    std::fs::create_dir_all(adir.join("Kick_In")).unwrap();
    std::fs::write(adir.join("Kick_In/peaks.bin"), b"peaks").unwrap();
    assert_eq!(
        svc.track_rename(&sid, "Kick_In", "Kick Inside").unwrap(),
        json!({"name": "Kick Inside"})
    );
    assert!(is_dirty(&svc, &sid));
    let t = svc.tracks_get(&sid).unwrap();
    assert_eq!(t["order"], json!(["Kick Inside", "Snare_Top"]));
    assert_eq!(t["tracks"]["Kick Inside"]["role"], "kick");
    assert!(t["tracks"].get("Kick_In").is_none());
    assert_eq!(std::fs::read(adir.join("Kick Inside/peaks.bin")).unwrap(), b"peaks");
    assert!(!adir.join("Kick_In").exists());
    svc.project_save(&sid).unwrap();
    let doc = load_project(&dir).unwrap().doc;
    assert!(doc.track("Kick Inside").is_some() && doc.track("Kick_In").is_none());
    assert_eq!(doc.track_order(), ["Kick Inside", "Snare_Top"]);
}

#[test]
fn track_rename_refuses_a_taken_name_a_bad_name_an_unknown_track_and_an_unknown_project() {
    let (svc, sid, _dir, _root) = opened("track_rename_bad");
    assert_eq!(svc.track_rename(&sid, "Kick_In", "Snare_Top").unwrap_err().code, 409);
    assert_eq!(svc.track_rename(&sid, "Kick_In", "a/b").unwrap_err().code, 400);
    assert_eq!(svc.track_rename(&sid, "Kick_In", " ").unwrap_err().code, 400);
    assert_eq!(svc.track_rename(&sid, "Nope", "X").unwrap_err().code, 400);
    assert_eq!(svc.track_rename("nope", "Kick_In", "X").unwrap_err().code, 404);
    assert_eq!(svc.track_rename(&sid, "Kick_In", "Kick_In").unwrap()["name"], "Kick_In");
    assert!(!is_dirty(&svc, &sid));
}

/// P4-4 (B3): a name that differs only in case from another track's is taken (one file name for
/// the exports and the `Analysis/` folder on a disk that ignores case). (B4): a rename that changes
/// only the case of the track's own name keeps its analysis folder; before, that rename removed
/// the folder (the "other" folder was the same one) and then failed with a 500.
#[test]
fn track_rename_refuses_a_name_that_differs_only_in_case_from_another_track() {
    let (svc, sid, _dir, _root) = opened("track_rename_case_taken");
    let e = svc.track_rename(&sid, "Kick_In", "snare_top").unwrap_err();
    assert_eq!(
        (e.code, e.message.as_str()),
        (409, "a track named \"Snare_Top\" exists")
    );
    assert_eq!(svc.tracks_get(&sid).unwrap()["order"], json!(["Kick_In", "Snare_Top"]));
}

/// P4-4 fix round: APFS also folds `ß`/`SS`, the final sigma and NFC/NFD; such a name is taken too.
#[test]
fn track_rename_refuses_a_name_that_apfs_folds_onto_another_track() {
    let (svc, sid, _dir, _root) = opened("track_rename_fold");
    for (other, taken) in [("Straße", "STRASSE"), ("ΟΔΟΣ", "οδοσ"), ("\u{dc}ber", "u\u{308}ber")] {
        svc.track_rename(&sid, "Snare_Top", other).unwrap();
        assert_eq!(
            svc.track_rename(&sid, "Kick_In", taken).unwrap_err().code,
            409,
            "{taken}"
        );
        svc.track_rename(&sid, other, "Snare_Top").unwrap();
    }
    assert_eq!(svc.tracks_get(&sid).unwrap()["order"], json!(["Kick_In", "Snare_Top"]));
}

#[test]
fn track_rename_that_changes_only_the_case_keeps_the_analysis_folder() {
    let (svc, sid, dir, _root) = opened("track_rename_case");
    let adir = dir.join("Analysis");
    std::fs::create_dir_all(adir.join("Kick_In")).unwrap();
    std::fs::write(adir.join("Kick_In/peaks.bin"), b"peaks").unwrap();
    assert_eq!(
        svc.track_rename(&sid, "Kick_In", "KICK_IN").unwrap(),
        json!({"name": "KICK_IN"})
    );
    assert_eq!(std::fs::read(adir.join("KICK_IN/peaks.bin")).unwrap(), b"peaks");
    let names: Vec<String> = std::fs::read_dir(&adir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["KICK_IN"], "the folder took the new case");
    assert_eq!(svc.tracks_get(&sid).unwrap()["order"], json!(["KICK_IN", "Snare_Top"]));
}

#[test]
fn app_settings_get_reads_the_home_settings_file_or_an_empty_object() {
    let (svc, _sid, _dir, root) = opened("settings_get");
    assert_eq!(svc.app_settings_get(), json!({}));
    std::fs::write(
        root.join("home/settings.json"),
        r#"{"master_db": -3.0, "inspector": false}"#,
    )
    .unwrap();
    assert_eq!(svc.app_settings_get(), json!({"master_db": -3.0, "inspector": false}));
}

#[test]
fn app_settings_set_writes_one_key_and_keeps_the_others() {
    let (svc, _sid, _dir, root) = opened("settings_set");
    std::fs::write(root.join("home/settings.json"), r#"{"audio_output": "AirPods"}"#).unwrap();
    assert_eq!(
        svc.app_settings_set("inspector", &json!(true)).unwrap(),
        json!({"key": "inspector", "value": true})
    );
    assert_eq!(
        svc.app_settings_get(),
        json!({"audio_output": "AirPods", "inspector": true})
    );
    assert_eq!(svc.app_settings_set("master_db", &json!(0)).unwrap_err().code, 400);
    assert_eq!(svc.app_settings_set("Bad Key", &json!(0)).unwrap_err().code, 400);
    assert_eq!(svc.app_settings_get()["audio_output"], "AirPods");
}

// ---------- project_recent ----------

#[test]
fn project_recent_keeps_ten_most_recent_first_without_duplicates() {
    let root = tmp("recent_ten");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    assert_eq!(svc.project_recent(), json!([]));
    let dir = root.to_str().unwrap();
    for k in 0..12 {
        svc.project_new(Some(&format!("P{k:02}")), Some(dir), None).unwrap();
    }
    // opening an old one moves it to the top
    svc.project_open(root.join("P05").to_str().unwrap()).unwrap();
    let r = svc.project_recent();
    let names: Vec<&str> = r
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 10);
    assert_eq!(names[..3], ["P05", "P11", "P10"]);
    assert_eq!(names.iter().filter(|n| **n == "P05").count(), 1);
    assert!(!names.contains(&"P00") && !names.contains(&"P01"));
    assert_eq!(keys(&r[0]), ["folder", "name", "opened"]);
    assert_eq!(r[0]["folder"], root.join("P05").display().to_string());
}

#[test]
fn project_recent_drops_folders_that_are_gone_and_survives_a_damaged_file() {
    let root = tmp("recent_gone");
    let svc = service_dsp();
    let home = root.join("home");
    svc.set_home(&home);
    let dir = root.to_str().unwrap();
    svc.project_new(Some("Keep"), Some(dir), None).unwrap();
    svc.project_new(Some("Gone"), Some(dir), None).unwrap();
    std::fs::remove_dir_all(root.join("Gone")).unwrap();
    let names = |v: Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names(svc.project_recent()), ["Keep"]);
    std::fs::write(home.join("recent.json"), "{broken").unwrap();
    assert_eq!(svc.project_recent(), json!([]));
    svc.project_open(root.join("Keep").to_str().unwrap()).unwrap();
    assert_eq!(names(svc.project_recent()), ["Keep"], "the next open repairs the file");
    // save and save as touch the list too
    let sid = svc.project_list(drumspike_app::ListShape::Ids)[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    svc.project_save_as(&sid, Some("Moved"), Some(dir), false, None)
        .unwrap();
    assert_eq!(names(svc.project_recent())[0], "Moved");
}

#[test]
fn project_recent_is_empty_and_writes_nothing_without_a_home_folder() {
    let root = tmp("recent_nohome");
    let svc = service_dsp();
    // a service built here has DRUMSPIKE_HOME unset in the test environment
    if svc.home().is_some() {
        return;
    }
    svc.project_new(Some("X"), Some(root.to_str().unwrap()), None).unwrap();
    assert_eq!(svc.project_recent(), json!([]));
}

// #441 Clear Recent: the meta-test `every_lifecycle_command_has_a_test_in_lifecycle_rs` asks
// for a test named after the command here, next to the unit test in `src/lifecycle/tests.rs`.
#[test]
fn project_recent_clear_empties_the_stored_list_and_a_new_open_starts_it_again() {
    let root = tmp("recent_clear");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = root.to_str().unwrap();
    svc.project_new(Some("One"), Some(dir), None).unwrap();
    svc.project_new(Some("Two"), Some(dir), None).unwrap();
    assert_eq!(svc.project_recent().as_array().unwrap().len(), 2);
    // the answer counts the entries the file held
    assert_eq!(svc.project_recent_clear(), json!({"cleared": 2}));
    // the list and the stored file are empty
    assert_eq!(svc.project_recent(), json!([]));
    let stored: Value = serde_json::from_str(&std::fs::read_to_string(root.join("home/recent.json")).unwrap()).unwrap();
    assert_eq!(stored, json!([]));
    // a project opened afterwards starts a new list
    svc.project_open(root.join("One").to_str().unwrap()).unwrap();
    let r = svc.project_recent();
    let names: Vec<&str> = r
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["One"]);
    assert_eq!(svc.project_recent_clear(), json!({"cleared": 1}));
}

// ---------- the dirty event ----------

#[test]
fn project_dirty_events_follow_every_change_of_the_flag() {
    let (svc, sid, _dir, _root) = opened("dirty_events");
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = Arc::clone(&seen);
    svc.set_emitter(Arc::new(move |name, v| {
        assert_eq!(name, "project-dirty");
        sink.lock().unwrap().push(v);
    }));
    svc.track_set_note(&sid, "Kick_In", 37).unwrap();
    svc.events_set(&sid, "Kick_In", &json!([])).unwrap(); // already dirty: no event
    svc.project_save(&sid).unwrap();
    svc.project_rename(&sid, "R").unwrap();
    svc.project_close(&sid, true).unwrap(); // closing sends none
    let want = [true, false, true].map(|d| json!({"sid": sid, "dirty": d}));
    assert_eq!(*seen.lock().unwrap(), want);
}

#[test]
fn project_dirty_event_reaches_the_bridge_channel() {
    let (svc, sid, _dir, _root) = opened("dirty_bridge");
    let bridge = Bridge::new(svc, repo_root().join("static"));
    let args = json!({"sid": sid, "name": "Kick_In", "note": 41});
    assert!(matches!(
        bridge.invoke("track_set_note", &args).unwrap(),
        Reply::Json(_)
    ));
    let r = bridge.events().poll(0, Duration::from_millis(200));
    assert_eq!(r["seq"], 1);
    assert_eq!(
        r["events"],
        json!([{"event": "project-dirty", "payload": {"sid": sid, "dirty": true}}])
    );
    // a read does not take the entry away; a poll after it sees nothing and times out
    assert_eq!(
        bridge.events().poll(0, Duration::from_millis(10))["events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        bridge.events().poll(1, Duration::from_millis(30)),
        json!({"seq": 1, "events": []})
    );
    // over HTTP: after=now starts at the end of the log
    let now = bridge.handle("GET", "/events?after=now&timeout_ms=10", b"");
    assert_eq!(now.status, 200);
    let old = bridge.handle("GET", "/events?after=0&timeout_ms=10", b"");
    assert!(String::from_utf8_lossy(&old.body).contains("project-dirty"));
}

// ---------- the bridge ----------

#[test]
fn project_commands_work_through_the_bridge_with_the_wrapper_argument_names() {
    let root = tmp("bridge_all");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let bridge = Bridge::new(svc, repo_root().join("static"));
    let json_of = |cmd: &str, args: Value| match bridge.invoke(cmd, &args).unwrap_or_else(|e| panic!("{cmd}: {e}")) {
        Reply::Json(v) => v,
        Reply::Bytes(_) => panic!("bytes"),
    };
    let v = json_of(
        "project_new",
        json!({"name": "B1", "dir": root.to_str().unwrap(), "audio_mode": "copied"}),
    );
    let sid = v["sid"].as_str().unwrap().to_string();
    assert_eq!(
        json_of("project_rename", json!({"sid": sid, "name": "B2"})),
        json!({"name": "B2"})
    );
    assert_eq!(json_of("project_save", json!({"sid": sid}))["dirty"], false);
    let moved = json_of(
        "project_save_as",
        json!({"sid": sid, "name": "B3", "dir": root.to_str().unwrap(), "copy_audio": false}),
    );
    assert_eq!(moved["name"], "B3");
    assert_eq!(json_of("project_recent", json!({})).as_array().unwrap().len(), 2);
    assert_eq!(
        json_of("project_close", json!({"sid": sid, "force": false}))["closed"],
        sid.as_str()
    );
    let e = bridge.invoke("project_close", &json!({"sid": sid})).unwrap_err();
    assert_eq!(e.code, 404);
    let e = bridge
        .invoke("track_set_note", &json!({"sid": "x", "name": "k"}))
        .unwrap_err();
    assert_eq!(e.code, 400, "a missing note is a bad request");
    let e = bridge.invoke("project_rename", &json!({"sid": 5})).unwrap_err();
    assert_eq!(e.code, 400);
    assert_eq!(bridge.invoke("no_such_command", &json!({})).unwrap_err().code, 404);
}

// ---------- the autosave ----------

#[test]
fn autosave_tick_writes_a_backup_of_a_dirty_project_and_leaves_the_document_alone() {
    let (svc, sid, dir, _root) = opened("autosave_tick");
    let doc = dir.join("Song.drumspike");
    let sha = file_sha256(&doc).unwrap();
    svc.set_autosave_interval(Duration::from_secs(60));
    // clean: nothing, however late
    assert!(svc.autosave_tick(Instant::now() + Duration::from_secs(3600)).is_empty());
    svc.events_set(&sid, "Kick_In", &json!([{"t": 1.0, "v": 70, "keep_me": [1, 2]}]))
        .unwrap();
    // dirty, interval not over
    assert!(svc.autosave_tick(Instant::now() + Duration::from_secs(10)).is_empty());
    let written = svc.autosave_tick(Instant::now() + Duration::from_secs(61));
    assert_eq!(written.len(), 1);
    assert!(written[0].starts_with(dir.join(BACKUPS_DIR)));
    assert_eq!(file_sha256(&doc).unwrap(), sha, "the main document is untouched");
    assert!(is_dirty(&svc, &sid), "an autosave does not clean the project");
    // the backup is a loadable document with the unsaved edit
    let text = std::fs::read_to_string(&written[0]).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["tracks"]["Kick_In"]["events"][0]["keep_me"], json!([1, 2]));
    assert_eq!(v["name"], "Song");
    assert_eq!(v["format"], 1);
}

/// #121: the autosave hits the stale-write check of `project_save`. It never writes the main document, so the outside
/// content stays and the backup holds the edit; the page gets one `project-stale` event per outside change.
#[test]
fn autosave_tick_hits_the_stale_write_check_and_tells_the_page_once() {
    let (svc, sid, dir, _root) = opened("autosave_stale");
    let doc = dir.join("Song.drumspike");
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = Arc::clone(&seen);
    svc.set_emitter(Arc::new(move |name, v| {
        if name == "project-stale" {
            sink.lock().unwrap().push(v);
        }
    }));
    svc.set_autosave_interval(Duration::from_secs(60));
    svc.track_set_note(&sid, "Kick_In", 46).unwrap();
    let base = Instant::now();
    assert_eq!(svc.autosave_tick(base + Duration::from_secs(61)).len(), 1);
    assert!(seen.lock().unwrap().is_empty(), "no outside change yet");
    let outside = write_outside(&doc);
    let written = svc.autosave_tick(base + Duration::from_secs(122));
    assert_eq!(written.len(), 1, "the backup is written: it keeps the edit");
    let v: Value = serde_json::from_slice(&std::fs::read(&written[0]).unwrap()).unwrap();
    assert_eq!(v["tracks"]["Kick_In"]["note"], 46);
    assert_eq!(std::fs::read(&doc).unwrap(), outside, "the outside content stays");
    {
        let ev = seen.lock().unwrap();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0]["sid"], json!(sid));
        assert!(
            ev[0]["message"]
                .as_str()
                .unwrap()
                .starts_with(drumspike_app::registry::CHANGED_ON_DISK)
        );
    }
    assert_eq!(svc.autosave_tick(base + Duration::from_secs(183)).len(), 1);
    assert_eq!(seen.lock().unwrap().len(), 1, "one event per outside change");
    assert_eq!(
        svc.project_save(&sid).unwrap_err().code,
        409,
        "the save refuses the same file"
    );
}

/// #228: a failed autosave stays on the project (`autosave_error` in the summary) and goes to the page once per failure
/// streak; a good autosave clears it.
#[test]
fn a_failed_autosave_is_kept_and_a_good_one_clears_it() {
    let (svc, sid, dir, _root) = opened("autosave_failed");
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = Arc::clone(&seen);
    svc.set_emitter(Arc::new(move |name, v| {
        if name == "autosave-failed" {
            sink.lock().unwrap().push(v);
        }
    }));
    svc.set_autosave_interval(Duration::from_secs(60));
    let backups = dir.join(BACKUPS_DIR);
    let _ = std::fs::remove_dir_all(&backups);
    std::fs::write(&backups, b"not a folder").unwrap();
    svc.track_set_note(&sid, "Kick_In", 46).unwrap();
    assert!(svc.project_get(&sid).unwrap().get("autosave_error").is_none());
    let base = Instant::now();
    assert!(
        svc.autosave_tick(base + Duration::from_secs(61)).is_empty(),
        "the write fails"
    );
    let err = svc.project_get(&sid).unwrap()["autosave_error"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(err.contains(BACKUPS_DIR), "{err}");
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(seen.lock().unwrap()[0]["sid"], json!(sid));
    assert_eq!(seen.lock().unwrap()[0]["message"], json!(err));
    assert!(
        svc.autosave_tick(base + Duration::from_secs(122)).is_empty(),
        "it retries per interval"
    );
    assert_eq!(seen.lock().unwrap().len(), 1, "one event per failure streak");
    assert!(svc.project_get(&sid).unwrap().get("autosave_error").is_some());
    std::fs::remove_file(&backups).unwrap();
    assert_eq!(svc.autosave_tick(base + Duration::from_secs(183)).len(), 1);
    assert!(
        svc.project_get(&sid).unwrap().get("autosave_error").is_none(),
        "a good autosave clears it"
    );
    // a save clears it too
    std::fs::remove_dir_all(&backups).unwrap();
    std::fs::write(&backups, b"not a folder").unwrap();
    svc.track_set_note(&sid, "Kick_In", 47).unwrap();
    assert!(svc.autosave_tick(base + Duration::from_secs(300)).is_empty());
    assert!(svc.project_get(&sid).unwrap().get("autosave_error").is_some());
    std::fs::remove_file(&backups).unwrap();
    svc.project_save(&sid).unwrap();
    assert!(
        svc.project_get(&sid).unwrap().get("autosave_error").is_none(),
        "a save clears it"
    );
}

#[test]
fn autosave_tick_restarts_the_clock_after_each_backup() {
    let (svc, sid, _dir, _root) = opened("autosave_clock");
    svc.set_autosave_interval(Duration::from_secs(60));
    svc.track_set_note(&sid, "Kick_In", 39).unwrap();
    let base = Instant::now();
    assert_eq!(svc.autosave_tick(base + Duration::from_secs(61)).len(), 1);
    assert!(
        svc.autosave_tick(base + Duration::from_secs(90)).is_empty(),
        "29 s after the last backup"
    );
    assert_eq!(svc.autosave_tick(base + Duration::from_secs(125)).len(), 1);
}

#[test]
fn autosave_keeps_ten_backups() {
    let (svc, sid, dir, _root) = opened("autosave_rotate");
    svc.set_autosave_interval(Duration::from_secs(1));
    svc.track_set_note(&sid, "Kick_In", 39).unwrap();
    let base = Instant::now();
    for k in 1..=15u64 {
        assert_eq!(
            svc.autosave_tick(base + Duration::from_secs(2 * k)).len(),
            1,
            "round {k}"
        );
    }
    assert_eq!(backups(&dir).len(), 10);
    // pre-save copies count against the same ten
    svc.project_save(&sid).unwrap();
    assert_eq!(backups(&dir).len(), 10);
}

#[test]
fn autosave_thread_writes_a_backup_with_a_one_second_interval() {
    let (svc, sid, dir, _root) = opened("autosave_thread");
    let doc = dir.join("Song.drumspike");
    let sha = file_sha256(&doc).unwrap();
    svc.set_autosave_interval(Duration::from_secs(1));
    let n_backups = backups(&dir).len();
    svc.track_set_note(&sid, "Kick_In", 44).unwrap();
    let end = Instant::now() + Duration::from_secs(8);
    while backups(&dir).len() == n_backups {
        assert!(Instant::now() < end, "no autosave within 8 s");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(file_sha256(&doc).unwrap(), sha);
    assert!(is_dirty(&svc, &sid));
}

// ---------- peaks_build_start, peaks_build_status ----------

fn collect() -> (Progress, Arc<Mutex<Vec<Value>>>) {
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let p: Progress = Arc::new(move |v: Value| sink.lock().unwrap().push(v));
    (p, seen)
}

fn wait_lanes(svc: &Service, sid: &str) -> Value {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let st = svc.peaks_build_status(sid);
        if st["state"] == "done" {
            return st;
        }
        assert!(Instant::now() < end, "the build did not finish: {st}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn peaks_build_start_reports_every_track_and_builds_the_caches() {
    let (svc, sid, dir, _root) = opened("lanes_build");
    assert_eq!(svc.peaks_build_status(&sid), json!({"state": "idle", "tracks": {}}));
    let (p, seen) = collect();
    let r = svc.peaks_build_start(&sid, None, Some(p)).unwrap();
    assert_eq!(r, json!({"started": ["Kick_In", "Snare_Top"]}));
    assert_eq!(
        svc.peaks_build_status(&sid)["total"],
        2,
        "queued before the call returns"
    );
    let st = wait_lanes(&svc, &sid);
    assert_eq!(st["tracks"]["Kick_In"], json!({"state": "done"}));
    assert_eq!(st["tracks"]["Snare_Top"], json!({"state": "done"}));
    for t in ["Kick_In", "Snare_Top"] {
        assert!(dir.join("Analysis").join(t).join("peaks.bin").is_file());
        assert!(dir.join("Analysis").join(t).join("key.json").is_file());
    }
    // the messages: {sid, job}; per track running (building) before done, then done.
    // P3-15d: the tracks build in parallel (up to 6 at once), so the order between the tracks
    // is free; the messages follow the changes one by one
    final_state(&seen, "job");
    let msgs = seen.lock().unwrap().clone();
    assert!(msgs.iter().all(|m| m["sid"] == sid.as_str()));
    assert_eq!(msgs.first().unwrap()["job"]["state"], "running");
    assert_eq!(msgs.last().unwrap()["job"]["state"], "done");
    for t in ["Kick_In", "Snare_Top"] {
        let run = msgs
            .iter()
            .position(|m| m["job"]["tracks"][t] == json!({"state": "running", "stage": "building"}))
            .unwrap_or_else(|| panic!("{t} never running: {msgs:?}"));
        let done = msgs
            .iter()
            .position(|m| m["job"]["tracks"][t]["state"] == "done")
            .unwrap();
        assert!(run < done, "{t}: running at {run}, done at {done}");
        assert!(msgs[done..].iter().all(|m| m["job"]["tracks"][t]["state"] == "done"));
    }
    let finished: Vec<usize> = msgs
        .iter()
        .map(|m| {
            let t = m["job"]["tracks"].as_object().unwrap();
            t.values().filter(|v| v["state"] == "done").count()
        })
        .collect();
    assert!(finished.windows(2).all(|w| w[0] <= w[1]), "{finished:?}");
    // job running, 2 x (track running, track done), job done: one message per change
    assert_eq!(msgs.len(), 6, "{msgs:?}");
    // the second start has nothing to build: done at once, one message, started []
    let (p2, seen2) = collect();
    let r2 = svc.peaks_build_start(&sid, None, Some(p2)).unwrap();
    assert_eq!(r2, json!({"started": []}));
    assert_eq!(svc.peaks_build_status(&sid)["state"], "done");
    let m2 = seen2.lock().unwrap().clone();
    assert_eq!(m2.len(), 1);
    assert_eq!(m2[0]["job"], json!({"state": "done", "total": 0, "tracks": {}}));
}

#[test]
fn peaks_build_start_takes_a_track_list_and_skips_offline_tracks() {
    let (svc, sid, dir, _root) = opened("lanes_subset");
    let r = svc
        .peaks_build_start(&sid, Some(vec!["Snare_Top".into()]), None)
        .unwrap();
    assert_eq!(r, json!({"started": ["Snare_Top"]}));
    wait_lanes(&svc, &sid);
    assert!(!dir.join("Analysis/Kick_In/peaks.bin").exists());
    // an unknown track is a 400, an unknown project a 404
    assert_eq!(
        svc.peaks_build_start(&sid, Some(vec!["Nope".into()]), None)
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(svc.peaks_build_start("nope", None, None).unwrap_err().code, 404);
    // an offline track without a cache is not in the job; the rest builds
    std::fs::remove_file(dir.parent().unwrap().join("daw/Kick_In.wav")).unwrap();
    svc.project_get(&sid).unwrap(); // resolves the sources again
    let r = svc.peaks_build_start(&sid, None, None).unwrap();
    assert_eq!(r, json!({"started": []}), "Kick_In is offline, Snare_Top is built");
    assert_eq!(
        svc.peaks_info(&sid, "Kick_In").unwrap_err().code,
        409,
        "peaks_info still tells why"
    );
}

#[test]
fn peaks_build_status_shows_an_error_of_one_track_and_goes_on() {
    // a source that is not a WAV file: its record matches the file, so it is online, but it cannot be decoded
    let root = tmp("lanes_error");
    let dir = project(&root, "Song", &[kick_spec("Snare_Top", "snare", 38)]);
    let mut p = load_project(&dir).unwrap();
    let junk = root.join("daw/Junk.wav");
    std::fs::write(&junk, b"this is not a wav file").unwrap();
    let rec = build_source(&junk, &p.dir, "referenced").unwrap();
    add_track(&mut p, "Junk", Some(rec), "other", 36, Some("average")).unwrap();
    p.doc.order = Some(vec!["Junk".into(), "Snare_Top".into()]);
    save_project(&mut p).unwrap();
    let svc = service_dsp();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let (prog, seen) = collect();
    let r = svc.peaks_build_start(&sid, None, Some(prog)).unwrap();
    assert_eq!(r, json!({"started": ["Junk", "Snare_Top"]}));
    let st = wait_lanes(&svc, &sid);
    assert_eq!(st["tracks"]["Junk"]["state"], "error");
    assert!(
        st["tracks"]["Junk"]["error"].as_str().unwrap().contains("Junk.wav"),
        "{st}"
    );
    assert_eq!(
        st["tracks"]["Snare_Top"],
        json!({"state": "done"}),
        "the next track still builds"
    );
    assert!(!dir.join("Analysis/Junk/peaks.bin").exists());
    assert_eq!(final_state(&seen, "job"), "done");
}

/// Two tests of one binary that pass the same tag to `opened` share one temp folder
/// (`ds_app_it_<tag>_<pid>`), and the cleanup of one pulls the folder from under the other
/// (CI on x86, 2026-10-12: `recent.json: No such file or directory` in the rename tests).
#[test]
fn every_opened_tag_in_this_file_is_unique() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/lifecycle.rs")).unwrap();
    let mut seen = std::collections::HashMap::new();
    for (n, line) in src.lines().enumerate() {
        if let Some(i) = line.find("opened(\"") {
            let rest = &line[i + 8..];
            let tag = &rest[..rest.find('"').unwrap()];
            if let Some(first) = seen.insert(tag.to_string(), n + 1) {
                panic!(
                    "tag {tag:?} is used on lines {first} and {}: each test needs its own",
                    n + 1
                );
            }
        }
    }
    assert!(seen.len() >= 4, "the scan found the opened() calls");
}

/// P4-40a fix: the page's undo of a Delete Hit puts the hit back with `events_edit` add (`added: false`, `orig`, `keep`:
/// the fields the detection stored); the document is then the saved one again and `project_clean_check` clears the flag.
/// Without `keep` the hit lost its `rise` fields and the project stayed dirty.
#[test]
fn a_delete_and_its_undo_make_the_project_clean_again() {
    let (svc, sid, _dir, _root) = opened("clean_undo_delete");
    let hits = json!([
        {"t": 0.5, "v": 90, "rise": 3.25, "rise_db": 7.5},
        {"t": 1.0, "v": 80, "rise": 2.5, "rise_db": 6.0, "orig": 0.99},
        {"t": 1.5, "v": 70, "rise": 1.5, "rise_db": 3.5}
    ]);
    svc.events_set(&sid, "Kick_In", &hits).unwrap();
    svc.project_save(&sid).unwrap();
    let saved = svc.events_get(&sid, "Kick_In").unwrap()["events"].clone();
    let gone = saved[1].clone();
    svc.events_edit(&sid, "Kick_In", &[json!({"op": "delete", "index": 1})])
        .unwrap();
    assert!(is_dirty(&svc, &sid));
    let mut keep = gone.as_object().unwrap().clone();
    for k in ["t", "v", "orig"] {
        keep.remove(k);
    }
    let add = json!({"op": "add", "t": gone["t"], "v": gone["v"], "added": false, "orig": gone["orig"], "keep": keep});
    svc.events_edit(&sid, "Kick_In", &[add]).unwrap();
    assert_eq!(
        svc.events_get(&sid, "Kick_In").unwrap()["events"],
        saved,
        "the hit is back as it was"
    );
    assert_eq!(svc.project_clean_check(&sid).unwrap(), json!({"dirty": false}));
}
