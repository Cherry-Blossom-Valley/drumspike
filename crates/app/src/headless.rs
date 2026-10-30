//! Analysis without a window (P3-11): the `analyze` step of `crates/devtools/src/bin/service_tool.rs`, shared
//! with the release app's `--analyze` mode, so `tools/parity/check_service.py` can run
//! the analysis inside a built `.app` (its DSP detector) with `--bin`.

use serde_json::{Map, Value, json};

use crate::Service;

/// Open each project folder, run `analyze_start` on every track (the params in the
/// document), wait for the job and return one object per folder:
/// `{"project", "job", "events": {track: events}}` (the events of `events_get`).
/// The first folder that does not open or start is an error.
pub fn analyze_projects(svc: &Service, dirs: &[String]) -> Result<Vec<Value>, String> {
    let mut out = Vec::with_capacity(dirs.len());
    for dir in dirs {
        let opened = svc.project_open(dir).map_err(|e| format!("{dir}: {e}"))?;
        let sid = opened["sid"].as_str().unwrap_or_default().to_string();
        svc.analyze_start(&sid, None, None, None)
            .map_err(|e| format!("{dir}: {e}"))?;
        let job = loop {
            let st = svc.analyze_status(&sid);
            if st["state"] == "done" {
                break st;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let mut events = Map::new();
        for name in job["tracks"].as_object().into_iter().flatten().map(|(n, _)| n) {
            let ev = svc.events_get(&sid, name).map_err(|e| e.to_string())?;
            events.insert(name.clone(), ev["events"].clone());
        }
        out.push(json!({"project": dir, "job": job, "events": events}));
    }
    Ok(out)
}
