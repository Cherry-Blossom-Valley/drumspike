//! P4-5 fix round H2: only another holder refuses a project. A folder or file system that cannot hold a lock opens the
//! project unlocked, and the open answer says why. This file has its own process: the file system failure is simulated
//! process wide (`lock::fake_lock_error`).

mod common;

use std::os::unix::fs::PermissionsExt;

use common::*;
use drumspike_app::lock::{fake_lock_error, ProjectLock};
use serde_json::Value;

fn app(root: &std::path::Path) -> drumspike_app::Service {
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    svc.set_lock_client("app");
    svc
}

#[test]
fn a_read_only_folder_and_a_file_system_without_locks_open_the_project_unlocked_and_say_why() {
    let root = tmp("lock_unavail");
    let dir = project(&root, "Ro", &[kick_spec("k in", "kick", 36)]);
    let other = project(&root, "NoLocks", &[kick_spec("k in", "kick", 36)]);
    let svc = app(&root);

    // a folder the process cannot write into (mode 0555): the lock file cannot be made
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let r = svc.project_open(&dir.display().to_string());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let summary = r.expect("the project opens although the folder is read-only");
    let note = summary["lock"].as_str().unwrap();
    assert!(note.starts_with("unavailable: "), "{note}");
    assert!(summary.get("read_only").is_none(), "unlocked is not read-only");
    assert_eq!(
        svc.tracks_get(summary["sid"].as_str().unwrap()).unwrap()["order"][0],
        "k in"
    );

    // a volume whose lock call fails with ENOTSUP-like "unsupported"
    fake_lock_error(Some(std::io::ErrorKind::Unsupported));
    let r = svc.project_open(&other.display().to_string());
    fake_lock_error(None);
    let summary: Value = r.expect("the project opens although flock is unsupported");
    assert_eq!(summary["lock"], "unavailable: operation not supported");

    // a normal project says `held`, and another holder still refuses (the one case that does)
    let third = project(&root, "Held", &[kick_spec("k in", "kick", 36)]);
    let held = svc.project_open(&third.display().to_string()).unwrap();
    assert_eq!(held["lock"], "held");
    let held_by_other = project(&root, "Other", &[kick_spec("k in", "kick", 36)]);
    let _theirs = ProjectLock::acquire(&held_by_other, "cli")
        .unwrap()
        .into_held()
        .unwrap();
    let e = svc.project_open(&held_by_other.display().to_string()).unwrap_err();
    assert_eq!(e.code, 409);
}
