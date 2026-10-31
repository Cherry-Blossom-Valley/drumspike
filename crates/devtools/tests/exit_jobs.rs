//! P4-5 fix round 2 (L3): a process that ends while a job runs waits for it. `cli batch` at the end of its input and the
//! MCP server when stdin closes must not leave a half-made Save As: the destination is a complete project.
//! The child processes are `service_tool cli batch` and `service_tool mcp`; `DRUMSPIKE_TEST_COPY_DELAY_MS` makes every
//! copy job pause after each chunk, so the job is still running when the input ends.

#[path = "../../app/tests/common/mod.rs"]
mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use common::*;
use serde_json::{Value, json};

fn three_track_project(root: &Path, tag: &str) -> std::path::PathBuf {
    project(
        root,
        tag,
        &[
            kick_spec("k in", "kick", 36),
            kick_spec("k out", "kick", 36),
            kick_spec("s top", "snare", 38),
        ],
    )
}

fn child(args: &[&str], root: &Path) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_service_tool"))
        .args(args)
        .env("DRUMSPIKE_HOME", root.join("home"))
        .env("DRUMSPIKE_TEST_COPY_DELAY_MS", "300")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// The destination holds a complete project: the document, and every audio file as large as its source.
fn assert_complete(dest: &Path, src_dir: &Path) {
    let doc = dest.join(format!("{}.drumspike", dest.file_name().unwrap().to_str().unwrap()));
    assert!(
        doc.is_file(),
        "no document in {}: {:?}",
        dest.display(),
        std::fs::read_dir(dest).map(std::iter::Iterator::count)
    );
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(doc).unwrap()).unwrap();
    for name in ["k in", "k out", "s top"] {
        let rec = &doc["tracks"][name]["source"];
        assert_eq!(rec["mode"], "copied", "{name}: {rec}");
        let copy = dest.join(rec["relative"].as_str().unwrap());
        let orig = src_dir.parent().unwrap().join("daw").join(format!("{name}.wav"));
        assert_eq!(
            std::fs::metadata(&copy).unwrap().len(),
            std::fs::metadata(&orig).unwrap().len(),
            "{name}"
        );
    }
}

fn wait(mut c: std::process::Child) -> (i32, String) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if let Some(st) = c.try_wait().unwrap() {
            let mut err = String::new();
            std::io::Read::read_to_string(c.stderr.as_mut().unwrap(), &mut err).unwrap();
            return (st.code().unwrap_or(-1), err);
        }
        assert!(std::time::Instant::now() < end, "the process did not end");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn batch_waits_for_a_copy_save_as_that_is_still_running_at_the_end_of_its_input() {
    let root = tmp("exit1");
    let dir = three_track_project(&root, "Src");
    let out = root.join("out");
    let mut c = child(&["cli", "batch"], &root);
    let mut stdin = c.stdin.take().unwrap();
    for step in [
        json!({"command": "project_open", "args": {"path": dir.display().to_string()}}),
        json!({"command": "project_save_as", "args": {"sid": "$sid", "name": "Moved", "dir": out.display().to_string(), "copy_audio": true}}),
    ] {
        writeln!(stdin, "{step}").unwrap();
    }
    drop(stdin); // the end of the input: the copy job is still running (300 ms per file)
    let (code, err) = wait(c);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("waiting"), "progress on stderr says it waits: {err}");
    assert_complete(&out.join("Moved"), &dir);
}

#[test]
fn the_mcp_server_waits_for_a_copy_save_as_when_stdin_closes() {
    let root = tmp("exit2");
    let dir = three_track_project(&root, "Src");
    let out = root.join("out");
    let mut c = child(&["mcp"], &root);
    let (mut stdin, mut stdout) = (c.stdin.take().unwrap(), BufReader::new(c.stdout.take().unwrap()));
    let mut call = |id: u64, name: &str, args: Value| -> Value {
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
        )
        .unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let sid = call(1, "project_open", json!({"path": dir.display().to_string()}))["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let r = call(
        2,
        "project_save_as",
        json!({"sid": sid, "name": "Moved", "dir": out.display().to_string(), "copy_audio": true}),
    );
    assert!(r["job"].is_string(), "{r}");
    drop(stdin); // the client goes away while the job runs
    let (code, err) = wait(c);
    assert_eq!(code, 0, "{err}");
    assert_complete(&out.join("Moved"), &dir);
}

#[test]
fn a_one_shot_copy_save_as_prints_the_finished_job_and_its_result() {
    let root = tmp("exit3");
    let dir = three_track_project(&root, "Src");
    let out = root.join("out");
    let c = child(
        &[
            "cli",
            "project_save_as",
            "--project",
            &dir.display().to_string(),
            "--name",
            "Moved",
            "--dir",
            &out.display().to_string(),
            "--copy_audio",
            "true",
        ],
        &root,
    );
    let o = c.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value =
        serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)));
    assert_eq!(v["job"]["state"], "done", "the final state, not the start answer: {v}");
    assert!(v["result"]["folder"].as_str().unwrap().ends_with("/Moved"), "{v}");
    assert_eq!(v["result"]["tracks"].as_object().unwrap().len(), 3, "{v}");
    assert_complete(&out.join("Moved"), &dir);
}

/// A killed process (SIGKILL; the MCP client's close and Ctrl+C come to the same) leaves no destination folder without a
/// document: the copy is built in `<name>.partial`. The next Save As to the name removes the stale partial and succeeds.
#[test]
fn a_killed_copy_save_as_leaves_no_half_made_destination_and_the_next_attempt_cleans_the_partial() {
    let root = tmp("exit4");
    let dir = three_track_project(&root, "Src");
    let out = root.join("out");
    let (dest, partial) = (out.join("Moved"), out.join("Moved.partial"));
    let mut c = child(&["cli", "batch"], &root);
    let mut stdin = c.stdin.take().unwrap();
    for step in [
        json!({"command": "project_open", "args": {"path": dir.display().to_string()}}),
        json!({"command": "project_save_as", "args": {"sid": "$sid", "name": "Moved", "dir": out.display().to_string(), "copy_audio": true}}),
    ] {
        writeln!(stdin, "{step}").unwrap();
    }
    stdin.flush().unwrap();
    // wait until the copy has put audio in the partial folder, then kill the process without a chance to clean up
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !std::fs::read_dir(partial.join("Audio Files")).is_ok_and(|mut d| d.next().is_some()) {
        assert!(std::time::Instant::now() < end, "the copy never started");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    c.kill().unwrap();
    c.wait().unwrap();
    assert!(
        !dest.exists(),
        "no destination folder (with audio and no document) after a kill: {:?}",
        std::fs::read_dir(&dest).map(std::iter::Iterator::count)
    );
    assert!(partial.is_dir(), "the stale partial folder is what the kill left");

    // the next attempt (the app side: locks, no delay) removes it and ends with a complete project
    let app = service_dsp();
    app.set_home(root.join("app-home"));
    app.set_lock_client("app");
    let sid = app.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    app.project_save_as(&sid, Some("Moved"), Some(&out.display().to_string()), true, None)
        .unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.job_status(&sid)["state"] == "running" {
        assert!(std::time::Instant::now() < end, "the copy did not end");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(app.job_status(&sid)["state"], "done");
    assert!(!partial.exists(), "the partial folder is gone");
    assert_complete(&dest, &dir);
}
