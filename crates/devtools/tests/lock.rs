//! P4-5 step 5: the cross-process project lock (`src/lock.rs`) and the read-only open. A child process
//! (`service_tool batch`, the engine of the command-line tool) holds a project; this process is the app.

#[path = "../../app/tests/common/mod.rs"]
mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use common::*;
use drumspike_app::call::call;
use drumspike_app::lock::{LOCK_FILE, ProjectLock};
use drumspike_app::reply::Reply;
use serde_json::{Value, json};

struct Holder {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
}

impl Holder {
    fn start(home: &std::path::Path) -> Holder {
        let mut child = Command::new(env!("CARGO_BIN_EXE_service_tool"))
            .args(["cli", "batch"])
            .env("DRUMSPIKE_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let (stdin, out) = (
            child.stdin.take().unwrap(),
            BufReader::new(child.stdout.take().unwrap()),
        );
        Holder { child, stdin, out }
    }

    fn step(&mut self, v: Value) -> Value {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: `{line}`"))
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn app_service(root: &std::path::Path) -> drumspike_app::Service {
    let svc = service_dsp();
    svc.set_home(root.join("app-home"));
    svc.set_lock_client("app");
    svc
}

#[test]
fn a_project_held_by_another_process_is_locked_read_only_works_and_a_kill_frees_it() {
    let root = tmp("lock1");
    let dir = project(&root, "Held", &[kick_spec("k in", "kick", 36)]);
    let path = dir.display().to_string();
    let mut h = Holder::start(&root.join("cli-home"));
    let r = h.step(json!({"command": "project_open", "args": {"path": path}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(ProjectLock::holder_of(&dir), Some((h.child.id(), "cli".to_string())));

    let app = app_service(&root);
    let e = app.project_open(&path).unwrap_err();
    assert_eq!(e.code, 409);
    assert!(
        e.message.contains(&format!("cli (pid {})", h.child.id())),
        "{}",
        e.message
    );
    assert!(e.message.contains("read-only"), "{}", e.message);

    // read-only: works while the other process writes; reads work, writes are refused
    let ro = app.project_open_with(&path, true).unwrap();
    assert_eq!(ro["read_only"], true);
    let sid = ro["sid"].as_str().unwrap().to_string();
    assert!(matches!(
        call(&app, "project_get", &json!({"sid": sid}), None),
        Ok(Reply::Json(_))
    ));
    assert!(matches!(
        call(&app, "events_get", &json!({"sid": sid, "name": "k in"}), None),
        Ok(Reply::Json(_))
    ));
    for (cmd, args) in [
        ("project_rename", json!({"sid": sid, "name": "X"})),
        ("analyze_start", json!({"sid": sid})),
        (
            "events_edit",
            json!({"sid": sid, "name": "k in", "ops": [{"op": "add", "t": 0.1}]}),
        ),
        ("project_save", json!({"sid": sid})),
        ("export_all", json!({"sid": sid})),
    ] {
        let e = call(&app, cmd, &args, None).unwrap_err();
        assert_eq!(e.code, 409, "{cmd}");
        assert!(e.message.contains("read-only"), "{cmd}: {}", e.message);
    }
    assert!(!dir.join("Exports").join("k in_spike.mid").exists());
    assert!(app.project_get(&sid).unwrap()["read_only"] == true);

    // the holder's own commands still work, and a kill (no clean close) frees the lock
    assert_eq!(h.step(json!({"command": "project_list"}))["ok"], true);
    app.project_close(&sid, true).unwrap();
    assert_eq!(app.project_open(&path).unwrap_err().code, 409);
    h.child.kill().unwrap();
    h.child.wait().unwrap();
    let opened = app.project_open(&path).unwrap();
    assert!(opened.get("read_only").is_none());
    assert_eq!(
        ProjectLock::holder_of(&dir),
        Some((std::process::id(), "app".to_string()))
    );
    // the same folder opened again in this process is the same project
    assert_eq!(app.project_open(&path).unwrap()["sid"], opened["sid"]);
}

#[test]
fn closing_a_project_releases_its_lock_and_save_as_moves_it_to_the_new_folder() {
    let root = tmp("lock2");
    let dir = project(&root, "Moves", &[kick_spec("k in", "kick", 36)]);
    let path = dir.display().to_string();
    let a = app_service(&root);
    let b = app_service(&root);
    let sid = a.project_open(&path).unwrap()["sid"].as_str().unwrap().to_string();
    assert_eq!(
        b.project_open(&path).unwrap_err().code,
        409,
        "two services of one process are two clients"
    );
    let out = root.join("out");
    let saved = a
        .project_save_as(&sid, Some("Moved"), Some(&out.display().to_string()), false, None)
        .unwrap();
    let new_dir = std::path::PathBuf::from(saved["folder"].as_str().unwrap());
    assert!(b.project_open(&path).is_ok(), "the old folder is free after Save As");
    assert_eq!(
        b.project_open(&new_dir.display().to_string()).unwrap_err().code,
        409,
        "the new folder is held"
    );
    a.project_close(&sid, true).unwrap();
    assert!(b.project_open(&new_dir.display().to_string()).is_ok());
    assert!(dir.join(LOCK_FILE).exists());
}

#[test]
fn save_as_takes_the_new_folders_lock_before_it_writes_and_keeps_the_old_project_on_a_refusal() {
    let root = tmp("lock4");
    let dir = project(&root, "Src", &[kick_spec("k in", "kick", 36)]);
    let app = app_service(&root);
    let sid = app.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    app.track_set_note(&sid, "k in", 41).unwrap(); // dirty
    let out = root.join("out");
    let dest = out.join("Moved");
    std::fs::create_dir_all(&dest).unwrap();
    // somebody else holds the lock of the destination folder
    let _theirs = ProjectLock::acquire(&dest, "cli").unwrap().into_held().unwrap();
    for copy in [false, true] {
        let e = app
            .project_save_as(&sid, Some("Moved"), Some(&out.display().to_string()), copy, None)
            .unwrap_err();
        assert_eq!(e.code, 409, "copy_audio {copy}: {}", e.message);
        assert!(e.message.contains("cli (pid"), "{}", e.message);
        assert!(
            !dest.join("Moved.drumspike").exists(),
            "nothing was written (copy_audio {copy})"
        );
        assert!(!dest.join("Audio Files").exists());
        // the old project, its folder and its lock are untouched
        let g = app.project_get(&sid).unwrap();
        assert!(g["folder"].as_str().unwrap().ends_with("/Src"), "{g}");
        assert_eq!(g["dirty"], true);
        assert_eq!(
            app_service(&root)
                .project_open(&dir.display().to_string())
                .unwrap_err()
                .code,
            409,
            "old lock kept"
        );
        assert_eq!(app.job_status(&sid)["state"], "idle", "no job was started");
    }
    // and with the destination free it works, and the old folder is released
    drop(_theirs);
    app.project_save_as(&sid, Some("Moved"), Some(&out.display().to_string()), false, None)
        .unwrap();
    assert!(
        app_service(&root).project_open(&dir.display().to_string()).is_ok(),
        "old lock released"
    );
}

#[test]
fn a_read_only_project_allows_repick_preview_and_save_as_to_a_writable_copy() {
    let root = tmp("lock5");
    let dir = project(&root, "Ro2", &[kick_spec("k in", "kick", 36)]);
    let app = app_service(&root);
    let sid = app.project_open_with(&dir.display().to_string(), true).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    // a re-pick without `store` is a read (here it fails for the missing model, not for the read-only mode) ...
    let r = call(&app, "track_repick", &json!({"sid": sid, "name": "k in"}), None);
    assert!(
        r.as_ref().err().is_none_or(|e| !e.message.contains("read-only")),
        "{r:?}"
    );
    // ... with `store` it changes the project
    let e = call(
        &app,
        "track_repick",
        &json!({"sid": sid, "name": "k in", "store": true}),
        None,
    )
    .unwrap_err();
    assert!(e.message.contains("read-only"), "{}", e.message);
    // Save As is the way to a writable copy: the project then is the copy, with its own lock
    let out = root.join("out");
    let Ok(Reply::Json(copy)) = call(
        &app,
        "project_save_as",
        &json!({"sid": sid, "name": "Copy", "dir": out.display().to_string()}),
        None,
    ) else {
        panic!("save as")
    };
    assert!(copy.get("read_only").is_none(), "{copy}");
    assert_eq!(copy["lock"], "held");
    assert!(call(&app, "project_rename", &json!({"sid": sid, "name": "Renamed"}), None).is_ok());
    assert_eq!(
        app_service(&root)
            .project_open(copy["folder"].as_str().unwrap())
            .unwrap_err()
            .code,
        409
    );
    assert!(
        !dir.join(LOCK_FILE).exists(),
        "a read-only open never made the lock file of the original"
    );
}

#[test]
fn a_refused_save_as_removes_only_what_the_call_made() {
    let root = tmp("lock6");
    let dir = project(&root, "Src", &[kick_spec("k in", "kick", 36)]);
    let out = root.join("out");
    let app = app_service(&root);
    let sid = app.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let out_s = out.display().to_string();

    // L2: Save As onto an existing project nobody holds: 409 before any lock is made; its lock file stays
    let taken = project(&out, "Taken", &[kick_spec("k in", "kick", 36)]);
    let theirs = app_service(&root);
    let their_sid = theirs.project_open(&taken.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    theirs.project_close(&their_sid, true).unwrap();
    assert!(taken.join(LOCK_FILE).exists(), "the earlier open left its lock file");
    let held_before = std::fs::read(taken.join(LOCK_FILE)).unwrap();
    for copy in [false, true] {
        let e = app
            .project_save_as(&sid, Some("Taken"), Some(&out_s), copy, None)
            .unwrap_err();
        assert_eq!(e.code, 409, "{}", e.message);
        assert!(e.message.contains("already exists"), "{}", e.message);
        assert!(
            taken.join(LOCK_FILE).exists(),
            "the lock file of the existing project stays (copy_audio {copy})"
        );
        assert_eq!(std::fs::read(taken.join(LOCK_FILE)).unwrap(), held_before);
    }

    // L1: a copy job is refused because another job of the project runs: the folder this call made goes again
    app.set_copy_delay(std::time::Duration::from_millis(400));
    app.project_save_as(&sid, Some("First"), Some(&out_s), true, None)
        .unwrap();
    let e = app
        .project_save_as(&sid, Some("Second"), Some(&out_s), true, None)
        .unwrap_err();
    assert_eq!(e.code, 409, "{}", e.message);
    assert!(
        !out.join("Second").exists(),
        "a folder only the refused call made is removed"
    );
    // a folder the user made stays, without the lock file
    std::fs::create_dir_all(out.join("Third")).unwrap();
    assert!(
        app.project_save_as(&sid, Some("Third"), Some(&out_s), true, None)
            .is_err()
    );
    assert!(out.join("Third").is_dir() && !out.join("Third").join(LOCK_FILE).exists());
    let _ = app.job_cancel(&sid);
}

#[test]
fn a_service_without_a_lock_client_takes_no_lock() {
    let root = tmp("lock3");
    let dir = project(&root, "Free", &[kick_spec("k in", "kick", 36)]);
    let svc = service_dsp();
    svc.project_open(&dir.display().to_string()).unwrap();
    assert!(!dir.join(LOCK_FILE).exists());
}

/// P4-5 round 4 (L1): a copy Save As into an existing folder that has no document but holds files is refused up front
/// (409), before any copy; an empty folder is fine, and so is one with only a lock file.
#[test]
fn a_copy_save_as_into_a_non_empty_folder_without_a_document_is_refused_before_any_copy() {
    let root = tmp("lock_l1");
    let dir = project(&root, "Src", &[kick_spec("k in", "kick", 36)]);
    let app = app_service(&root);
    let sid = app.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let out = root.join("out");
    let dest = out.join("Moved");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("notes.txt"), b"mine").unwrap();
    let e = app
        .project_save_as(&sid, Some("Moved"), Some(&out.display().to_string()), true, None)
        .unwrap_err();
    assert_eq!(e.code, 409, "{}", e.message);
    assert!(e.message.contains("not empty"), "{}", e.message);
    assert!(!out.join("Moved.partial").exists(), "nothing was copied");
    assert_eq!(app.job_status(&sid)["state"], "idle", "no job was started");
    assert!(dest.join("notes.txt").exists());
    // an empty folder is replaced by the finished copy
    std::fs::remove_file(dest.join("notes.txt")).unwrap();
    app.project_save_as(&sid, Some("Moved"), Some(&out.display().to_string()), true, None)
        .unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.job_status(&sid)["state"] == "running" {
        assert!(std::time::Instant::now() < end, "the copy did not end");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(app.job_status(&sid)["state"], "done");
    assert!(dest.join("Moved.drumspike").is_file());
}

/// P4-5 round 4 (L2): a plain Save As to a name whose copy Save As holds `<name>.partial` is refused at once (409).
#[test]
fn a_plain_save_as_is_refused_while_another_process_builds_the_partial_folder_of_that_name() {
    let root = tmp("lock_l2");
    let dir = project(&root, "Src", &[kick_spec("k in", "kick", 36)]);
    let app = app_service(&root);
    let sid = app.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let out = root.join("out");
    let partial = out.join("Moved.partial");
    std::fs::create_dir_all(&partial).unwrap();
    let theirs = ProjectLock::acquire(&partial, "cli").unwrap().into_held().unwrap();
    let e = app
        .project_save_as(&sid, Some("Moved"), Some(&out.display().to_string()), false, None)
        .unwrap_err();
    assert_eq!(e.code, 409, "{}", e.message);
    assert!(e.message.contains("cli (pid"), "{}", e.message);
    assert!(!out.join("Moved").exists(), "nothing was written");
    // the old project is untouched, and once the other copy is gone the Save As works
    assert!(
        app.project_get(&sid).unwrap()["folder"]
            .as_str()
            .unwrap()
            .ends_with("/Src")
    );
    drop(theirs);
    app.project_save_as(&sid, Some("Moved"), Some(&out.display().to_string()), false, None)
        .unwrap();
    assert!(out.join("Moved/Moved.drumspike").is_file());
}

/// P4-5 round 5 (L1): a destination folder that holds only a stale lock file is replaced by the finished copy.
#[test]
fn a_copy_save_as_into_a_folder_with_only_a_stale_lock_file_succeeds() {
    let root = tmp("lock_l1b");
    let dir = project(&root, "Src", &[kick_spec("k in", "kick", 36)]);
    let app = app_service(&root);
    let sid = app.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let out = root.join("out");
    let dest = out.join("Moved");
    std::fs::create_dir_all(&dest).unwrap();
    drop(ProjectLock::acquire(&dest, "cli").unwrap()); // leaves the lock file, nobody holds it
    assert!(dest.join(LOCK_FILE).exists());
    app.project_save_as(&sid, Some("Moved"), Some(&out.display().to_string()), true, None)
        .unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.job_status(&sid)["state"] == "running" {
        assert!(std::time::Instant::now() < end, "the copy did not end");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(app.job_status(&sid)["state"], "done", "{}", app.job_status(&sid));
    assert!(dest.join("Moved.drumspike").is_file());
    assert!(!out.join("Moved.partial").exists());
}
