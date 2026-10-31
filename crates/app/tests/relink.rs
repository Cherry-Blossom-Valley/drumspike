//! P3-4d: copy jobs (Save As with copied audio, Consolidate), `job_status`, `job_cancel`, the cancel
//! of a copy-mode `tracks_add`, `project_relink` and `relink_search`, through `drumspike_app::Service`
//! without Tauri. Test names start with the command name; the last tests check the route map rows 8, 9, 11.
//!
//! The mid-copy actions (an edit, a cancel, a second job) run inside the progress callback. It runs on
//! the worker thread between two files, when the project lock is free, so these tests are
//! deterministic: no sleep decides what happens during the copy.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::file_sha256;
use common::*;
use drumspike_app::{COPYJOB_COMMANDS, Progress, RELINK_COMMANDS, Service};
use drumspike_core::project::{AUDIO_DIR, consolidate, load_project};
use serde_json::{Value, json};

fn service(root: &Path) -> Service {
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    svc.set_projects_dir(root.join("projects"));
    svc
}

/// `n` mono 48 kHz files `t01.wav` ... in `<root>/<audio>/`, each with its own content.
fn make_audio(root: &Path, audio: &str, n: usize) -> Vec<PathBuf> {
    (1..=n)
        .map(|i| {
            let p = root.join(audio).join(format!("t{i:02}.wav"));
            write_wav(&p, 48_000, &kick_signal(&[0.1 + 0.013 * i as f64], 1.2, 48_000));
            p
        })
        .collect()
}

fn open_new(svc: &Service, name: &str) -> (String, PathBuf) {
    let r = svc.project_new(Some(name), None, None).unwrap();
    (
        r["sid"].as_str().unwrap().to_string(),
        PathBuf::from(r["folder"].as_str().unwrap()),
    )
}

/// A saved project with `n` referenced tracks, each with one hit of its own. Returns (sid, folder, files).
fn session(svc: &Service, root: &Path, n: usize) -> (String, PathBuf, Vec<PathBuf>) {
    let files = make_audio(root, "audio", n);
    let (sid, dir) = open_new(svc, "Song");
    let paths: Vec<String> = files.iter().map(|p| p.display().to_string()).collect();
    let r = svc.tracks_add(&sid, &paths, None, &[], &[], false, None).unwrap();
    assert_eq!(r["added"].as_array().unwrap().len(), n);
    for (i, name) in r["added"].as_array().unwrap().iter().enumerate() {
        let ev = json!([{"t": 0.1 + 0.013 * (i + 1) as f64, "v": 100}]);
        svc.events_set(&sid, name.as_str().unwrap(), &ev).unwrap();
    }
    svc.project_save(&sid).unwrap();
    (sid, dir, files)
}

fn collect() -> (Progress, Arc<Mutex<Vec<Value>>>) {
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let p: Progress = Arc::new(move |v: Value| sink.lock().unwrap().push(v));
    (p, seen)
}

fn wait_job(svc: &Service, sid: &str) -> Value {
    let end = Instant::now() + Duration::from_secs(120);
    loop {
        let st = svc.job_status(sid);
        if st["state"] != "running" {
            return st;
        }
        assert!(Instant::now() < end, "the job did not finish: {st}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == ext) {
                out.push(p);
            }
        }
    }
    out
}

fn statuses(svc: &Service, sid: &str) -> (usize, usize, usize) {
    let g = svc.project_get(sid).unwrap();
    let t = g["tracks"].as_object().unwrap();
    let online = t.values().filter(|v| v["status"] == "ok").count();
    let copied = t.values().filter(|v| v["source"]["mode"] == "copied").count();
    (online, copied, t.len())
}

fn all_events(svc: &Service, sid: &str) -> Vec<Value> {
    let order = svc.project_get(sid).unwrap()["order"].clone();
    order
        .as_array()
        .unwrap()
        .iter()
        .map(|n| svc.events_get(sid, n.as_str().unwrap()).unwrap()["events"].clone())
        .collect()
}

// ---------- job_status and job_cancel ----------

#[test]
fn job_status_is_idle_without_a_job_and_keeps_the_last_state() {
    let root = tmp("rl_status");
    let svc = service(&root);
    let (sid, _dir, _files) = session(&svc, &root, 2);
    assert_eq!(svc.job_status(&sid)["state"], "idle");
    svc.project_consolidate(&sid, None).unwrap();
    let st = wait_job(&svc, &sid);
    assert_eq!(st["state"], "done");
    assert_eq!(st["kind"], "consolidate");
    assert_eq!(st["result"]["consolidated"].as_array().unwrap().len(), 2);
    assert_eq!((st["done"].clone(), st["total"].clone()), (json!(2), json!(2)));
    assert_eq!(st["bytes_done"], st["bytes_total"]);
    assert!(st["bytes_total"].as_u64().unwrap() > 0);
    assert_eq!(svc.job_status("nope")["state"], "idle");
}

#[test]
fn job_cancel_says_whether_a_job_was_running() {
    let root = tmp("rl_cancel_says");
    let svc = service(&root);
    let (sid, _dir, _files) = session(&svc, &root, 2);
    assert_eq!(svc.job_cancel(&sid), json!({"sid": sid, "cancelling": false}));
    svc.project_consolidate(&sid, None).unwrap();
    wait_job(&svc, &sid);
    assert_eq!(
        svc.job_cancel(&sid)["cancelling"],
        false,
        "a finished job cannot be cancelled"
    );
    assert_eq!(svc.job_cancel("nope")["cancelling"], false);
}

// ---------- project_save_as with copied audio ----------

#[test]
fn project_save_as_copies_the_audio_as_a_job_with_progress_and_the_project_stays_usable() {
    let root = tmp("rl_saveas");
    let svc = service(&root);
    let (sid, dir, files) = session(&svc, &root, 30);
    let before = all_events(&svc, &sid);
    let sums: Vec<String> = files.iter().map(|p| file_sha256(p).unwrap()).collect();
    let out = root.join("out");
    // an edit while the copy runs: from the progress callback, after the third file
    let edit_ok = Arc::new(AtomicBool::new(false));
    let edited = json!([{"t": 0.5, "v": 90}, {"t": 0.75, "v": 91}]);
    let (sink, seen) = collect();
    let (s2, sid2, ok2, ed2) = (svc.clone(), sid.clone(), Arc::clone(&edit_ok), edited.clone());
    let done_once = Arc::new(AtomicBool::new(false));
    let p: Progress = Arc::new(move |v: Value| {
        sink(v.clone());
        if v["copy"]["done"] == 3 && !done_once.swap(true, Ordering::SeqCst) {
            ok2.store(s2.events_set(&sid2, "t01", &ed2).is_ok(), Ordering::SeqCst);
        }
    });
    let r = svc
        .project_save_as(&sid, Some("Song Copy"), Some(out.to_str().unwrap()), true, Some(p))
        .unwrap();
    assert_eq!(r["job"]["state"], "running", "the call returns at once: {r}");
    assert_eq!(r["job"]["kind"], "save_as");
    let st = wait_job(&svc, &sid);
    assert_eq!(st["state"], "done", "{st}");
    final_state(&seen, "copy");
    let msgs = seen.lock().unwrap();
    assert!(msgs.len() >= 10, "at least 10 progress updates, got {}", msgs.len());
    assert!(msgs.iter().all(|m| m["sid"] == json!(sid)));
    assert_eq!(msgs.last().unwrap()["copy"]["state"], "done");
    assert!(edit_ok.load(Ordering::SeqCst), "events_set during the copy succeeded");
    // the result is the summary of the new project, under the same id, clean
    let summary = &st["result"];
    let new_dir = out.join("Song Copy");
    assert_eq!(summary["sid"], json!(sid));
    assert_eq!(summary["folder"], json!(new_dir.display().to_string()));
    assert_eq!(summary["dirty"], false);
    assert_eq!(statuses(&svc, &sid), (30, 30, 30));
    // the edit made during the copy is in the new document; the other tracks are unchanged
    let after = all_events(&svc, &sid);
    assert_eq!(after[0], edited);
    assert_eq!(after[1..], before[1..]);
    let reopened = load_project(&new_dir).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.doc.track("t01").unwrap().events.clone()).unwrap(),
        edited
    );
    // the copy opens 30 / 30 online after the original folder and its audio are renamed
    svc.project_close(&sid, true).unwrap();
    std::fs::rename(&dir, root.join("projects/Song renamed")).unwrap();
    std::fs::rename(root.join("audio"), root.join("audio renamed")).unwrap();
    let again = svc.project_open(new_dir.to_str().unwrap()).unwrap();
    let sid3 = again["sid"].as_str().unwrap();
    assert_eq!(statuses(&svc, sid3), (30, 30, 30));
    assert_eq!(files_with_ext(&new_dir, "tmp").len(), 0);
    assert_eq!(files_with_ext(&new_dir.join(AUDIO_DIR), "wav").len(), 30);
    // the sources are untouched
    let sums_after: Vec<String> = files
        .iter()
        .map(|p| file_sha256(root.join("audio renamed").join(p.file_name().unwrap())).unwrap())
        .collect();
    assert_eq!(sums, sums_after);
}

#[test]
fn project_save_as_copy_cancel_removes_every_file_and_leaves_the_project_as_it_was() {
    let root = tmp("rl_saveas_cancel");
    let svc = service(&root);
    let (sid, dir, _files) = session(&svc, &root, 8);
    let doc_before = std::fs::read(drumspike_core::project::document_path(&dir)).unwrap();
    let events_before = all_events(&svc, &sid);
    let out = root.join("out");
    let (sink, seen) = collect();
    let (s2, sid2) = (svc.clone(), sid.clone());
    let p: Progress = Arc::new(move |v: Value| {
        sink(v.clone());
        if v["copy"]["done"] == 3 {
            let _ = s2.job_cancel(&sid2);
        }
    });
    svc.project_save_as(&sid, Some("Song Copy"), Some(out.to_str().unwrap()), true, Some(p))
        .unwrap();
    let st = wait_job(&svc, &sid);
    assert_eq!(st["state"], "cancelled", "{st}");
    assert_eq!(final_state(&seen, "copy"), "cancelled");
    assert_eq!(files_with_ext(&out, "tmp").len(), 0, "0 .tmp files");
    assert!(!out.join("Song Copy").exists(), "the folder the job made is removed");
    // the project is the one it was: same folder, clean, same events, same document on disk
    let g = svc.project_get(&sid).unwrap();
    assert_eq!(g["folder"], json!(dir.display().to_string()));
    assert_eq!(g["dirty"], false);
    assert_eq!(statuses(&svc, &sid), (8, 0, 8));
    assert_eq!(all_events(&svc, &sid), events_before);
    assert_eq!(
        std::fs::read(drumspike_core::project::document_path(&dir)).unwrap(),
        doc_before
    );
    // and a new Save As to the same place works
    svc.project_save_as(&sid, Some("Song Copy"), Some(out.to_str().unwrap()), true, None)
        .unwrap();
    assert_eq!(wait_job(&svc, &sid)["state"], "done");
    assert_eq!(files_with_ext(&out, "tmp").len(), 0);
}

#[test]
fn project_save_as_copy_into_an_existing_non_empty_folder_is_refused_and_leaves_its_files() {
    let root = tmp("rl_saveas_cancel_existing");
    let svc = service(&root);
    let (sid, _dir, _files) = session(&svc, &root, 4);
    let out = root.join("out");
    let target = out.join("Song Copy");
    std::fs::create_dir_all(target.join("Audio Files")).unwrap();
    std::fs::write(target.join("keep.txt"), b"mine").unwrap();
    std::fs::write(target.join("Audio Files/keep.wav"), b"mine").unwrap();
    // P4-5 round 4 (L1): a folder with files and no document is refused up front, before any copy; the files stay
    let e = svc
        .project_save_as(&sid, Some("Song Copy"), Some(out.to_str().unwrap()), true, None)
        .unwrap_err();
    assert_eq!(e.code, 409, "{}", e.message);
    assert_eq!(svc.job_status(&sid)["state"], "idle");
    assert_eq!(std::fs::read(target.join("keep.txt")).unwrap(), b"mine");
    assert_eq!(std::fs::read(target.join("Audio Files/keep.wav")).unwrap(), b"mine");
    let left = files_with_ext(&target, "wav");
    assert_eq!(left, [target.join("Audio Files/keep.wav")], "{left:?}");
    assert!(!out.join("Song Copy.partial").exists());
}

#[test]
fn project_save_as_copy_refuses_an_offline_source_a_taken_folder_and_a_second_job() {
    let root = tmp("rl_saveas_refuse");
    let svc = service(&root);
    let (sid, _dir, files) = session(&svc, &root, 3);
    let out = root.join("out");
    let d = out.to_str().unwrap();
    // a taken folder
    svc.project_save_as(&sid, Some("Taken"), Some(d), false, None).unwrap();
    let e = svc
        .project_save_as(&sid, Some("Taken"), Some(d), true, None)
        .unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.contains("already exists"), "{e}");
    // an offline source: 409 before anything is written
    std::fs::remove_file(&files[1]).unwrap();
    let e = svc
        .project_save_as(&sid, Some("Never"), Some(d), true, None)
        .unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.contains("relink"), "{e}");
    assert!(!out.join("Never").exists());
    assert_eq!(svc.job_status(&sid)["state"], "idle", "no job started");
}

#[test]
fn project_save_as_copy_second_job_is_refused_while_one_runs() {
    let root = tmp("rl_saveas_second");
    let svc = service(&root);
    let (sid, _dir, _files) = session(&svc, &root, 4);
    let refused = Arc::new(AtomicUsize::new(0));
    let (s2, sid2, r2) = (svc.clone(), sid.clone(), Arc::clone(&refused));
    let p: Progress = Arc::new(move |v: Value| {
        if v["copy"]["done"] == 1 {
            let a = s2.project_consolidate(&sid2, None).unwrap_err();
            let b = s2.relink_search(&sid2, "/tmp").unwrap_err();
            let c = s2
                .tracks_add(&sid2, &["/nonexistent.wav".to_string()], None, &[], &[], true, None)
                .unwrap_err();
            r2.store(
                usize::from(a.code == 409) + usize::from(b.code == 409) + usize::from(c.code == 409),
                Ordering::SeqCst,
            );
        }
    });
    svc.project_save_as(
        &sid,
        Some("Copy"),
        Some(root.join("out").to_str().unwrap()),
        true,
        Some(p),
    )
    .unwrap();
    assert_eq!(wait_job(&svc, &sid)["state"], "done");
    assert_eq!(
        refused.load(Ordering::SeqCst),
        3,
        "Consolidate, a search and a copying add are refused with 409"
    );
}

#[test]
fn project_save_as_copy_of_a_closed_project_cancels_the_job() {
    let root = tmp("rl_saveas_close");
    let svc = service(&root);
    let (sid, _dir, _files) = session(&svc, &root, 6);
    let out = root.join("out");
    let (sink, seen) = collect();
    let (s2, sid2) = (svc.clone(), sid.clone());
    let p: Progress = Arc::new(move |v: Value| {
        sink(v.clone());
        if v["copy"]["done"] == 2 && v["copy"]["state"] == "running" {
            s2.project_close(&sid2, true).unwrap();
        }
    });
    svc.project_save_as(&sid, Some("Copy"), Some(out.to_str().unwrap()), true, Some(p))
        .unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    while seen
        .lock()
        .unwrap()
        .last()
        .is_none_or(|m| m["copy"]["state"] == "running")
    {
        assert!(Instant::now() < end, "the job did not end");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(final_state(&seen, "copy"), "cancelled");
    assert!(!out.join("Copy").exists(), "the folder of the cancelled job is removed");
    assert_eq!(files_with_ext(&out, "tmp").len(), 0);
    assert_eq!(svc.project_get(&sid).unwrap_err().code, 404);
}

// ---------- project_consolidate ----------

#[test]
fn project_consolidate_gives_the_records_of_the_core_consolidate() {
    let root = tmp("rl_consolidate");
    let svc = service(&root);
    // a mixed project: two tracks are copied already, three are referenced
    let files = make_audio(&root, "audio2", 5);
    let (sid, dir) = open_new(&svc, "Mixed");
    let paths: Vec<String> = files.iter().map(|p| p.display().to_string()).collect();
    svc.tracks_add(&sid, &paths[..2], None, &[], &[], true, None).unwrap();
    svc.tracks_add(&sid, &paths[2..], None, &[], &[], false, None).unwrap();
    svc.project_save(&sid).unwrap();
    // the reference: the core function the Python server calls (`project.consolidate`), on a copy of the folder
    let ref_dir = root.join("ref/Mixed");
    copy_tree(&dir, &ref_dir);
    let mut reference = load_project(&ref_dir).unwrap();
    let changed = consolidate(&mut reference).unwrap();
    let r = svc.project_consolidate(&sid, None).unwrap();
    assert_eq!(r["job"]["total"], 3, "three referenced tracks");
    let st = wait_job(&svc, &sid);
    assert_eq!(st["state"], "done");
    let names: Vec<String> = st["result"]["consolidated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, changed, "the same tracks, in document order");
    assert_eq!(st["result"]["dirty"], true);
    svc.project_save(&sid).unwrap();
    let mine = load_project(&dir).unwrap();
    let rec = |v: &drumspike_core::Source| {
        let mut m = serde_json::to_value(v).unwrap().as_object().unwrap().clone();
        m.remove("path"); // absolute paths differ: the two projects are in two folders
        Value::Object(m)
    };
    for (name, tr) in reference.doc.tracks() {
        let have = mine.doc.track(name).unwrap().source_ref().unwrap();
        assert_eq!(rec(tr.source_ref().unwrap()), rec(have), "{name}");
        assert_eq!(have.mode.as_deref(), Some("copied"));
    }
    assert_eq!(statuses(&svc, &sid), (5, 5, 5));
    // a second run has nothing to do and is done at once
    let r = svc.project_consolidate(&sid, None).unwrap();
    assert_eq!(r["job"]["state"], "done");
    assert_eq!(r["job"]["result"]["consolidated"], json!([]));
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_tree(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

#[test]
fn project_consolidate_cancel_removes_the_copied_files_and_changes_no_record() {
    let root = tmp("rl_consolidate_cancel");
    let svc = service(&root);
    let (sid, dir, _files) = session(&svc, &root, 8);
    let (s2, sid2) = (svc.clone(), sid.clone());
    let (sink, seen) = collect();
    let p: Progress = Arc::new(move |v: Value| {
        sink(v.clone());
        if v["copy"]["done"] == 4 {
            let _ = s2.job_cancel(&sid2);
        }
    });
    svc.project_consolidate(&sid, Some(p)).unwrap();
    let st = wait_job(&svc, &sid);
    assert_eq!(st["state"], "cancelled", "{st}");
    assert_eq!(final_state(&seen, "copy"), "cancelled");
    assert_eq!(files_with_ext(&dir, "tmp").len(), 0, "0 .tmp files");
    assert_eq!(
        files_with_ext(&dir.join(AUDIO_DIR), "wav").len(),
        0,
        "the copies are removed"
    );
    assert_eq!(statuses(&svc, &sid), (8, 0, 8), "every record is still referenced");
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], false);
    // run again: it finishes and copies all 8
    svc.project_consolidate(&sid, None).unwrap();
    assert_eq!(wait_job(&svc, &sid)["state"], "done");
    assert_eq!(statuses(&svc, &sid), (8, 8, 8));
    assert_eq!(files_with_ext(&dir.join(AUDIO_DIR), "wav").len(), 8);
}

#[test]
fn project_consolidate_refuses_an_offline_source_and_an_unknown_project() {
    let root = tmp("rl_consolidate_bad");
    let svc = service(&root);
    let (sid, _dir, files) = session(&svc, &root, 3);
    std::fs::remove_file(&files[2]).unwrap();
    let e = svc.project_consolidate(&sid, None).unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.starts_with("cannot copy source (missing)"), "{e}");
    assert_eq!(svc.job_status(&sid)["state"], "idle");
    assert_eq!(svc.project_consolidate("nope", None).unwrap_err().code, 404);
}

#[test]
fn project_consolidate_progress_counts_files_and_bytes() {
    let root = tmp("rl_consolidate_progress");
    let svc = service(&root);
    let (sid, _dir, _files) = session(&svc, &root, 6);
    let (p, seen) = collect();
    svc.project_consolidate(&sid, Some(p)).unwrap();
    wait_job(&svc, &sid);
    final_state(&seen, "copy");
    let msgs = seen.lock().unwrap();
    assert!(msgs.len() >= 12, "two messages per file at least: {}", msgs.len());
    let done: Vec<i64> = msgs.iter().map(|m| m["copy"]["done"].as_i64().unwrap()).collect();
    assert!(done.windows(2).all(|w| w[0] <= w[1]), "done never goes back: {done:?}");
    let last = &msgs.last().unwrap()["copy"];
    assert_eq!((last["done"].clone(), last["total"].clone()), (json!(6), json!(6)));
    assert_eq!(last["bytes_done"], last["bytes_total"]);
    assert_eq!(last["result"]["consolidated"].as_array().unwrap().len(), 6);
}

// ---------- tracks_add with copy: the same cancel ----------

#[test]
fn tracks_add_copy_cancel_adds_no_track_and_leaves_no_file() {
    let root = tmp("rl_add_cancel");
    let svc = service(&root);
    let files = make_audio(&root, "drop", 10);
    let paths: Vec<String> = files.iter().map(|p| p.display().to_string()).collect();
    let (sid, dir) = open_new(&svc, "Song");
    svc.project_save(&sid).unwrap();
    let (s2, sid2) = (svc.clone(), sid.clone());
    let (sink, seen) = collect();
    let p: Progress = Arc::new(move |v: Value| {
        sink(v.clone());
        if v["add"]["state"] == "running" && v["add"]["done"] == 4 {
            let _ = s2.job_cancel(&sid2);
        }
    });
    let r = svc.tracks_add(&sid, &paths, None, &[], &[], true, Some(p)).unwrap();
    assert_eq!(r["cancelled"], true, "{r}");
    assert_eq!(r["added"], json!([]));
    assert_eq!(final_state(&seen, "add"), "cancelled");
    let g = svc.project_get(&sid).unwrap();
    assert_eq!(g["tracks"].as_object().unwrap().len(), 0, "no track was added");
    assert_eq!(g["dirty"], false, "the project is as it was");
    assert_eq!(files_with_ext(&dir, "tmp").len(), 0, "0 .tmp files");
    assert_eq!(
        files_with_ext(&dir.join(AUDIO_DIR), "wav").len(),
        0,
        "the copied files are removed"
    );
    assert_eq!(svc.job_status(&sid)["state"], "cancelled");
    // the same call without the cancel adds all ten
    let r = svc.tracks_add(&sid, &paths, None, &[], &[], true, None).unwrap();
    assert_eq!(r["added"].as_array().unwrap().len(), 10);
    assert_eq!(files_with_ext(&dir.join(AUDIO_DIR), "wav").len(), 10);
    // a referenced add does not take a job slot
    svc.tracks_add(&sid, &paths[..1], None, &[], &[], false, None).unwrap();
}

// ---------- project_relink ----------

#[test]
fn project_relink_accepts_the_moved_file_refuses_another_and_forces_a_different_one() {
    let root = tmp("rl_relink");
    let svc = service(&root);
    let (sid, dir, files) = session(&svc, &root, 2);
    let before = all_events(&svc, &sid);
    // t01 moves; t02 stays
    let moved = root.join("moved/t01.wav");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&files[0], &moved).unwrap();
    assert_eq!(svc.project_get(&sid).unwrap()["tracks"]["t01"]["status"], "missing");
    // a different file: 409 without force
    let other = root.join("other/t01.wav");
    write_wav(&other, 44_100, &kick_signal(&[0.3], 2.0, 44_100));
    let e = svc
        .project_relink(&sid, "t01", other.to_str().unwrap(), false)
        .unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.contains("force=true"), "{e}");
    assert_eq!(svc.project_get(&sid).unwrap()["tracks"]["t01"]["status"], "missing");
    // unknown track, a path that is no file
    assert_eq!(
        svc.project_relink(&sid, "ghost", moved.to_str().unwrap(), false)
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        svc.project_relink(&sid, "t01", root.join("nope.wav").to_str().unwrap(), false)
            .unwrap_err()
            .code,
        404
    );
    assert_eq!(svc.project_relink("nope", "t01", "/x", false).unwrap_err().code, 404);
    // the moved original: accepted, matched
    let r = svc.project_relink(&sid, "t01", moved.to_str().unwrap(), false).unwrap();
    assert_eq!(r["matched"], true);
    assert_eq!(r["status"], "ok");
    assert_eq!(r["source"]["mode"], "referenced");
    assert_eq!(r["source"]["path"], json!(moved.display().to_string()));
    assert_eq!(svc.project_get(&sid).unwrap()["tracks"]["t01"]["status"], "ok");
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], true);
    assert_eq!(all_events(&svc, &sid), before, "the events stay");
    // the different file with force: the identity and the header facts are refreshed, the events stay
    let r = svc.project_relink(&sid, "t01", other.to_str().unwrap(), true).unwrap();
    assert_eq!(r["matched"], false);
    let t = svc.tracks_get(&sid).unwrap();
    assert_eq!(t["tracks"]["t01"]["sr"], 44_100);
    assert_eq!(t["tracks"]["t01"]["n_samples"], 88_200);
    assert_eq!(all_events(&svc, &sid), before);
    // a file inside Audio Files/ makes the source copied
    let inside = dir.join(AUDIO_DIR).join("t02.wav");
    std::fs::copy(&files[1], &inside).unwrap();
    let r = svc
        .project_relink(&sid, "t02", inside.to_str().unwrap(), false)
        .unwrap();
    assert_eq!(r["source"]["mode"], "copied");
    // a file that is no audio file cannot be forced onto a track
    let junk = root.join("junk.wav");
    std::fs::write(&junk, b"not a wav").unwrap();
    let e = svc
        .project_relink(&sid, "t02", junk.to_str().unwrap(), true)
        .unwrap_err();
    assert_eq!(e.code, 400, "{e}");
}

// ---------- relink_search ----------

/// The 30-track case of the card on a short generated session: the audio folder moves, one search
/// relinks every track, the events stay.
#[test]
fn relink_search_relinks_the_moved_audio_folder_in_one_action() {
    let root = tmp("rl_search30");
    let svc = service(&root);
    let (sid, dir, _files) = session(&svc, &root, 30);
    let before = all_events(&svc, &sid);
    svc.project_close(&sid, true).unwrap();
    std::fs::create_dir_all(root.join("elsewhere/deep")).unwrap();
    std::fs::rename(root.join("audio"), root.join("elsewhere/deep/audio moved")).unwrap();
    let opened = svc.project_open(dir.to_str().unwrap()).unwrap();
    let sid = opened["sid"].as_str().unwrap().to_string();
    assert_eq!(statuses(&svc, &sid).0, 0, "all 30 are offline after the move");
    let t0 = Instant::now();
    let r = svc
        .relink_search(&sid, root.join("elsewhere").to_str().unwrap())
        .unwrap();
    let took = t0.elapsed();
    assert_eq!(
        (r["offline"].clone(), r["relinked"].clone()),
        (json!(30), json!(30)),
        "{r}"
    );
    assert_eq!(r["found"].as_array().unwrap().len(), 30);
    assert!(r["found"].as_array().unwrap().iter().all(|f| f["by"] == "name"));
    assert_eq!((r["ambiguous"].clone(), r["missing"].clone()), (json!([]), json!([])));
    assert_eq!(r["files_seen"], 30);
    assert!(took < Duration::from_secs(5), "30 / 30 in under 5 s: {took:?}");
    assert_eq!(statuses(&svc, &sid), (30, 0, 30), "30 / 30 online");
    assert_eq!(all_events(&svc, &sid), before, "the events stay unchanged");
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], true);
    // the Analysis cache key did not change: nothing is rebuilt
    let started = svc.peaks_build_start(&sid, None, None).unwrap();
    assert_eq!(started["started"], json!([]), "every peak cache is still current");
    // a save makes it permanent: the next open finds the audio
    svc.project_save(&sid).unwrap();
    svc.project_close(&sid, true).unwrap();
    let again = svc.project_open(dir.to_str().unwrap()).unwrap();
    assert_eq!(statuses(&svc, again["sid"].as_str().unwrap()).0, 30);
}

#[test]
fn relink_search_lists_ambiguous_and_missing_tracks_and_chooses_none() {
    let root = tmp("rl_search_mixed");
    let svc = service(&root);
    let (sid, dir, files) = session(&svc, &root, 4);
    svc.project_close(&sid, true).unwrap();
    // t01 and t02 exist twice (two places), t03 is gone, t04 comes back under another name
    let lib = root.join("lib");
    for (i, f) in files.iter().enumerate() {
        let name = f.file_name().unwrap();
        match i {
            0 | 1 => {
                std::fs::create_dir_all(lib.join("a")).unwrap();
                std::fs::create_dir_all(lib.join("b")).unwrap();
                std::fs::copy(f, lib.join("a").join(name)).unwrap();
                std::fs::copy(f, lib.join("b").join(name)).unwrap();
            }
            3 => {
                std::fs::create_dir_all(&lib).unwrap();
                std::fs::copy(f, lib.join("renamed take.wav")).unwrap();
            }
            _ => {}
        }
    }
    // a decoy for t03: same name, other content
    write_wav(&lib.join("t03.wav"), 48_000, &kick_signal(&[0.9], 1.2, 48_000));
    std::fs::remove_dir_all(root.join("audio")).unwrap();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let r = svc.relink_search(&sid, lib.to_str().unwrap()).unwrap();
    assert_eq!(
        (r["offline"].clone(), r["relinked"].clone()),
        (json!(4), json!(1)),
        "{r}"
    );
    assert_eq!(r["found"][0]["track"], "t04");
    assert_eq!(r["found"][0]["by"], "content");
    let amb: Vec<&str> = r["ambiguous"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["track"].as_str().unwrap())
        .collect();
    assert_eq!(amb, ["t01", "t02"]);
    assert_eq!(r["ambiguous"][0]["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(r["missing"][0]["track"], "t03");
    assert_eq!(
        r["missing"][0]["differs"],
        json!([lib.join("t03.wav").display().to_string()])
    );
    // the ambiguous and the missing tracks stay offline
    let g = svc.project_get(&sid).unwrap();
    for (n, want) in [
        ("t01", "missing"),
        ("t02", "missing"),
        ("t03", "missing"),
        ("t04", "ok"),
    ] {
        assert_eq!(g["tracks"][n]["status"], want, "{n}");
    }
    // the user picks one of the candidates with project_relink
    let pick = r["ambiguous"][0]["candidates"][0].as_str().unwrap();
    svc.project_relink(&sid, "t01", pick, false).unwrap();
    assert_eq!(svc.project_get(&sid).unwrap()["tracks"]["t01"]["status"], "ok");
    // the forced decoy is a different file (not matching): refused without force
    let e = svc
        .project_relink(&sid, "t03", lib.join("t03.wav").to_str().unwrap(), false)
        .unwrap_err();
    assert_eq!(e.code, 409);
}

#[test]
fn relink_search_checks_its_arguments_and_leaves_online_tracks_alone() {
    let root = tmp("rl_search_args");
    let svc = service(&root);
    let (sid, _dir, _files) = session(&svc, &root, 2);
    assert_eq!(svc.relink_search("nope", "/tmp").unwrap_err().code, 404);
    assert_eq!(svc.relink_search(&sid, "/no/such/folder").unwrap_err().code, 404);
    let r = svc.relink_search(&sid, root.to_str().unwrap()).unwrap();
    assert_eq!((r["offline"].clone(), r["relinked"].clone()), (json!(0), json!(0)));
    assert_eq!(
        svc.project_get(&sid).unwrap()["dirty"],
        false,
        "nothing found, nothing changed"
    );
    assert_eq!(svc.job_status(&sid)["state"], "done");
    assert_eq!(svc.job_status(&sid)["kind"], "relink");
}

#[test]
fn project_consolidate_pauses_after_each_chunk_when_the_test_delay_is_set() {
    // `set_copy_delay` is the hook of the e2e tests (bridge command `test_copy_delay`): a job lasts long enough to act on
    let root = tmp("rl_delay");
    let svc = service(&root);
    let (sid, _dir, _files) = session(&svc, &root, 3);
    svc.set_copy_delay(Duration::from_millis(120));
    let t0 = Instant::now();
    svc.project_consolidate(&sid, None).unwrap();
    assert_eq!(wait_job(&svc, &sid)["state"], "done");
    assert!(t0.elapsed() >= Duration::from_millis(3 * 120), "{:?}", t0.elapsed());
    svc.set_copy_delay(Duration::ZERO);
}

// ---------- every command of P3-4d has a test ----------

#[test]
fn every_copyjob_and_relink_command_has_a_test_here() {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/relink.rs")).unwrap();
    for c in COPYJOB_COMMANDS.iter().chain(RELINK_COMMANDS.iter()) {
        assert!(
            src.contains(&format!("fn {c}_")),
            "no test named {c}_... in tests/relink.rs"
        );
    }
}

// ---------- the numbers of the card on the generated 30-track session ----------

/// `DRUMSPIKE_TRACKS_BENCH=<make_session.py folder>` (30 mono 24-bit files, 96 kHz, 300 s, 86 MB each):
/// import referenced, move the audio folder, one relink search; Save As with copied audio (progress count,
/// an edit during the copy, the copy opens 30 / 30 online after the original is renamed); a cancel (0 `.tmp`
/// files); Consolidate against the core function of the server. The sources are copied into the system
/// temp folder first, so nothing moves in the original.
/// Run with `cargo test --release -p drumspike-app --test relink -- --ignored --nocapture`.
#[test]
#[ignore = "needs a generated session: DRUMSPIKE_TRACKS_BENCH=<make_session.py folder>"]
fn relink_bench_of_the_generated_session() {
    let Some(folder) = std::env::var_os("DRUMSPIKE_TRACKS_BENCH").map(PathBuf::from) else {
        panic!("DRUMSPIKE_TRACKS_BENCH is not set");
    };
    let root = tmp("rl_bench");
    let svc = service(&root);
    let audio = root.join("audio");
    std::fs::create_dir_all(&audio).unwrap();
    let mut wavs: Vec<PathBuf> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    wavs.sort();
    let sums: Vec<String> = wavs.iter().map(|p| file_sha256(p).unwrap()).collect();
    for w in &wavs {
        std::fs::copy(w, audio.join(w.file_name().unwrap())).unwrap();
    }
    let (sid, dir) = open_new(&svc, "Bench");
    let paths = [audio.display().to_string()];
    let r = svc.tracks_add(&sid, &paths, None, &[], &[], false, None).unwrap();
    let names: Vec<String> = r["added"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    for (i, n) in names.iter().enumerate() {
        svc.events_set(
            &sid,
            n,
            &json!([{"t": 1.0 + i as f64, "v": 100}, {"t": 2.5 + i as f64, "v": 64}]),
        )
        .unwrap();
    }
    svc.project_save(&sid).unwrap();
    let events = all_events(&svc, &sid);
    // 1. move the audio folder, search once
    svc.project_close(&sid, true).unwrap();
    let moved = root.join("elsewhere/audio moved");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&audio, &moved).unwrap();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let offline = 30 - statuses(&svc, &sid).0;
    let t0 = Instant::now();
    let r = svc
        .relink_search(&sid, root.join("elsewhere").to_str().unwrap())
        .unwrap();
    let t_search = t0.elapsed();
    let ok = statuses(&svc, &sid).0;
    let same = all_events(&svc, &sid) == events;
    eprintln!(
        "BENCH relink: offline {offline}, relinked {} of {}, online after {ok}, search {:.0} ms (files {}), events unchanged {same}",
        r["relinked"],
        r["offline"],
        t_search.as_secs_f64() * 1000.0,
        r["files_seen"]
    );
    assert_eq!(ok, 30);
    assert!(same);
    svc.project_save(&sid).unwrap();
    // 2. Save As with copied audio; an edit during the copy; progress count; the copy opens after a rename
    let out = root.join("out");
    let (sink, seen) = collect();
    let (s2, sid2, edit) = (svc.clone(), sid.clone(), Arc::new(AtomicBool::new(false)));
    let (e2, name0) = (Arc::clone(&edit), names[0].clone());
    let once = Arc::new(AtomicBool::new(false));
    let p: Progress = Arc::new(move |v: Value| {
        sink(v.clone());
        if v["copy"]["done"] == 2 && !once.swap(true, Ordering::SeqCst) {
            e2.store(
                s2.events_set(&sid2, &name0, &json!([{"t": 9.0, "v": 77}])).is_ok(),
                Ordering::SeqCst,
            );
        }
    });
    let t0 = Instant::now();
    svc.project_save_as(&sid, Some("Bench Copy"), Some(out.to_str().unwrap()), true, Some(p))
        .unwrap();
    let st = wait_job(&svc, &sid);
    let t_copy = t0.elapsed();
    let n_msgs = seen.lock().unwrap().len();
    let new_dir = out.join("Bench Copy");
    svc.project_close(&sid, true).unwrap();
    std::fs::rename(&dir, root.join("Bench renamed")).unwrap();
    std::fs::rename(&moved, root.join("audio renamed")).unwrap();
    let again = svc.project_open(new_dir.to_str().unwrap()).unwrap();
    let (online, copied, total) = statuses(&svc, again["sid"].as_str().unwrap());
    eprintln!(
        "BENCH save as: state {}, {n_msgs} progress updates, edit during the copy ok {}, copy {:.2} s, opens {online} / {total} online ({copied} copied), tmp files {}",
        st["state"],
        edit.load(Ordering::SeqCst),
        t_copy.as_secs_f64(),
        files_with_ext(&new_dir, "tmp").len()
    );
    assert_eq!((st["state"].as_str(), online, total), (Some("done"), 30, 30));
    assert!(n_msgs >= 10 && edit.load(Ordering::SeqCst));
    // 3. cancel in the middle: 0 .tmp files, no folder
    let sid = again["sid"].as_str().unwrap().to_string();
    svc.project_close(&sid, true).unwrap();
    std::fs::rename(root.join("audio renamed"), &moved).unwrap();
    std::fs::rename(root.join("Bench renamed"), &dir).unwrap();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let out2 = root.join("out2");
    let (s2, sid2) = (svc.clone(), sid.clone());
    let p: Progress = Arc::new(move |v: Value| {
        if v["copy"]["done"] == 5 {
            let _ = s2.job_cancel(&sid2);
        }
    });
    svc.project_save_as(&sid, Some("Cancelled"), Some(out2.to_str().unwrap()), true, Some(p))
        .unwrap();
    let st = wait_job(&svc, &sid);
    eprintln!(
        "BENCH cancel: state {}, tmp files {}, folder left {}, project folder {}",
        st["state"],
        files_with_ext(&out2, "tmp").len(),
        out2.join("Cancelled").exists(),
        svc.project_get(&sid).unwrap()["folder"]
    );
    assert_eq!(st["state"], "cancelled");
    assert_eq!(files_with_ext(&out2, "tmp").len(), 0);
    // 4. consolidate against the core function
    let ref_dir = root.join("ref/Bench");
    copy_tree(&dir, &ref_dir);
    let mut p = load_project(&ref_dir).unwrap();
    let t0 = Instant::now();
    let changed = consolidate(&mut p).unwrap();
    let t_core = t0.elapsed();
    let t0 = Instant::now();
    svc.project_consolidate(&sid, None).unwrap();
    let st = wait_job(&svc, &sid);
    let t_job = t0.elapsed();
    svc.project_save(&sid).unwrap();
    let mine = load_project(&dir).unwrap();
    let mut equal = 0;
    for (n, tr) in p.doc.tracks() {
        let (a, b) = (
            tr.source_ref().unwrap(),
            mine.doc.track(n).unwrap().source_ref().unwrap(),
        );
        if (&a.mode, &a.relative, a.size, a.mtime, &a.head_sha1)
            == (&b.mode, &b.relative, b.size, b.mtime, &b.head_sha1)
        {
            equal += 1;
        }
    }
    eprintln!(
        "BENCH consolidate: job {} in {:.2} s (core {:.2} s), {} tracks changed (core {}), records equal paths aside {equal} / 30",
        st["state"],
        t_job.as_secs_f64(),
        t_core.as_secs_f64(),
        st["result"]["consolidated"].as_array().unwrap().len(),
        changed.len()
    );
    assert_eq!(equal, 30);
    let after: Vec<String> = wavs.iter().map(|p| file_sha256(p).unwrap()).collect();
    assert_eq!(sums, after, "the sources keep their SHA-256");
}
