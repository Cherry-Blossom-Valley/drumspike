//! P3-15c: the feature `trace` writes a Chrome trace with the spans `ipc`, `cmd`, `job` and
//! `lock_wait` (thread name included). Runs only with `--features trace`; one test in its own
//! binary, because the recording is global to the process.
#![cfg(feature = "trace")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use drumspike_app::registry::lock;
use drumspike_app::trace;
use serde_json::Value;

#[test]
fn trace_writes_the_spans_with_their_thread_and_the_lock_waits() {
    let path = std::env::temp_dir().join(format!("ds_trace_{}.json", std::process::id()));
    assert!(trace::init(&path, 12.5));
    assert!(!trace::init(&path, 0.0), "one recording per process");
    {
        let _i = trace::ipc("project_open");
        let _c = trace::command("project_open");
        let _j = trace::stage("resolve");
    }
    let m = Arc::new(Mutex::new(0));
    let held = lock(&m);
    let m2 = Arc::clone(&m);
    let waiter = std::thread::Builder::new()
        .name("waiter".into())
        .spawn(move || {
            *lock(&m2) += 1;
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    drop(held);
    waiter.join().unwrap();
    trace::finish();
    let _late = trace::command("after_finish"); // not written, and no panic
    let text = std::fs::read_to_string(&path).unwrap();
    let ev: Vec<Value> = serde_json::from_str(&text).expect("a complete JSON array");
    let begins: Vec<&Value> = ev.iter().filter(|e| e["ph"] == "B").collect();
    let names: Vec<(String, String)> = begins
        .iter()
        .map(|e| {
            (
                e["name"].as_str().unwrap().to_string(),
                e["args"]["name"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    assert!(names.contains(&("ipc".into(), "project_open".into())), "{names:?}");
    assert!(names.contains(&("cmd".into(), "project_open".into())));
    assert!(names.contains(&("job".into(), "resolve".into())));
    let wait = begins.iter().find(|e| e["name"] == "lock_wait").expect("a lock wait");
    assert_eq!(wait["args"]["thread"], "waiter");
    assert!(
        wait["args"]["at"].as_str().unwrap().contains("tests/trace.rs"),
        "{wait}"
    );
    let start = ev.iter().find(|e| e["name"] == "trace_start").unwrap();
    // tracing-chrome writes a float field as its text
    let ms = &start["args"]["process_ms"];
    assert_eq!(
        ms.as_f64().or_else(|| ms.as_str().and_then(|t| t.parse().ok())),
        Some(12.5)
    );
    assert!(
        ev.iter().any(|e| e["ph"] == "M" && e["args"]["name"] == "waiter"),
        "thread names are written"
    );
    assert!(!names.iter().any(|(_, n)| n == "after_finish"));
    let ends = ev.iter().filter(|e| e["ph"] == "E").count();
    assert_eq!(ends, begins.len(), "every span ended");
    std::fs::remove_file(&path).unwrap();
}
