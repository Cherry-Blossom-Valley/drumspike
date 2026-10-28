//! Parity helper of `tools/parity/check_service.py` (P3-3a): runs the service commands
//! on project folders that the Python check wrote, and prints JSON.
//!
//! ```text
//! service_tool analyze <project_dir>...   analyse every track of each project (the params in
//!                                         the document), print one JSON object per project
//! service_tool export <jobs.json>         one export_midi call per job
//! service_tool spikes <project_dir>...    export_spikes for every track (the events of the
//!                                         document), one JSON object per project (P3-3c)
//! service_tool velocities <jobs.json>     one velocities_recompute call per job (P3-3c)
//! service_tool export-all <project_dir>   export_all, prints the answer (P3-3c)
//! service_tool core-analyze <jobs.json>   the core's analysis of a file, one JSON line per job (P3-12,
//!                                         #294: DSP-only)
//! service_tool run <script.json>          P4-4: a list of app commands in one service, as the page
//!                                         sends them (`tools/check_exports.py` scenarios)
//! service_tool cli <command> ...          P4-5: the command-line tool of the app (`drumspike`)
//! service_tool mcp                        P4-5: the MCP server of the app (`drumspike mcp`)
//! service_tool learn-eval <spec.json>     P4-6b (#293, DSP-only): the Learn evaluation (`tools/learn_eval.py`; `learn_eval.rs`)
//! ```

use std::path::Path;
use std::process::ExitCode;

#[path = "service_tool/learn_eval.rs"]
mod learn_eval;

use drumspike_app::{jobs, media, Service, ServiceConfig};
use drumspike_core::analyze::{analyze_detected, detect_events, AnalyzeParams, Detector};
use serde_json::{json, Value};

fn service() -> Service {
    let models = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
    Service::new(ServiceConfig::from_env(&models))
}

fn analyze(dirs: &[String]) -> Result<(), String> {
    for v in drumspike_app::headless::analyze_projects(&service(), dirs)? {
        println!("{v}");
    }
    Ok(())
}

fn export(jobs_file: &str) -> Result<(), String> {
    let svc = service();
    let text = std::fs::read_to_string(jobs_file).map_err(|e| format!("{jobs_file}: {e}"))?;
    let jobs: Vec<Value> = serde_json::from_str(&text).map_err(|e| format!("{jobs_file}: {e}"))?;
    let mut out = Vec::new();
    for j in &jobs {
        let dir = j["project"].as_str().ok_or("job without project")?;
        let sid = svc.project_open(dir).map_err(|e| e.to_string())?["sid"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let events = j["events"].as_array().cloned().unwrap_or_default();
        let r = svc.export_midi(
            &sid,
            j["track"].as_str().ok_or("job without track")?,
            j["note"].as_i64().unwrap_or(36),
            &events,
            j["ppq"].as_i64().unwrap_or(24000),
            j["tempo"].as_f64().unwrap_or(120.0),
        );
        out.push(match r {
            Ok(v) => v,
            Err(e) => json!({"error": e.to_string()}),
        });
    }
    println!("{}", Value::Array(out));
    Ok(())
}

fn open(svc: &Service, dir: &str) -> Result<String, String> {
    let opened = svc.project_open(dir).map_err(|e| e.to_string())?;
    Ok(opened["sid"].as_str().unwrap_or_default().to_string())
}

/// P3-3c: `export_spikes` for every track of each project.
fn spikes(dirs: &[String]) -> Result<(), String> {
    let svc = service();
    for dir in dirs {
        let sid = open(&svc, dir)?;
        let tracks = svc.tracks_get(&sid).map_err(|e| e.to_string())?;
        let mut out = serde_json::Map::new();
        for name in tracks["order"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            out.insert(
                name.to_string(),
                svc.export_spikes(&sid, name, None)
                    .unwrap_or_else(|e| json!({"error": e.to_string()})),
            );
        }
        println!("{}", json!({"project": dir, "tracks": out}));
    }
    Ok(())
}

/// P3-3c: `velocities_recompute` per job `{project, track, events?}`.
fn velocities(jobs_file: &str) -> Result<(), String> {
    let svc = service();
    let text = std::fs::read_to_string(jobs_file).map_err(|e| format!("{jobs_file}: {e}"))?;
    let jobs: Vec<Value> = serde_json::from_str(&text).map_err(|e| format!("{jobs_file}: {e}"))?;
    let mut out = Vec::new();
    for j in &jobs {
        let sid = open(&svc, j["project"].as_str().ok_or("job without project")?)?;
        let events = j["events"].as_array().cloned();
        let r = svc.velocities_recompute(&sid, j["track"].as_str().ok_or("job without track")?, events.as_deref());
        out.push(r.unwrap_or_else(|e| json!({"error": e.to_string()})));
    }
    println!("{}", Value::Array(out));
    Ok(())
}

/// P3-3c: `export_all` on one project; the time is the command's own.
fn export_all(dir: &str) -> Result<(), String> {
    let svc = service();
    let sid = open(&svc, dir)?;
    let start = std::time::Instant::now();
    let r = svc.export_all(&sid, None).map_err(|e| e.to_string())?;
    println!(
        "{}",
        json!({"result": r, "wall_ms": start.elapsed().as_millis() as u64})
    );
    Ok(())
}

/// P3-12, #294: the analysis of the core on a file, for the frozen references of the Python
/// server (`tools/parity/check_analyze.py`, `tests/test_rust_analyze.py`). One job per entry of
/// `<jobs.json>`: `{wav, mix?, role, params?, detected?}` — DSP-only: the DSP detector runs when
/// no `detected` is given (a non-null `model` key is an error
/// here); `detected` ([[t, note], ...]) skips the detector and runs only the steps after it
/// (`analyze_detected`). The params are read as the app reads a track's `params`
/// (`jobs::parse_params`). P4-12: `rule` (`1`, `2`; default 1, the frozen engine's) is the
/// refinement rule. Prints one JSON line per job: `{events, branches, detected, detector, sr}`
/// or `{error}`.
fn core_analyze(jobs_file: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(jobs_file).map_err(|e| format!("{jobs_file}: {e}"))?;
    let jobs: Vec<Value> = serde_json::from_str(&text).map_err(|e| format!("{jobs_file}: {e}"))?;
    for j in &jobs {
        let r = core_analyze_one(j);
        println!("{}", r.unwrap_or_else(|e| json!({ "error": e })));
    }
    Ok(())
}

fn core_analyze_one(j: &Value) -> Result<Value, String> {
    if j.get("model").is_some_and(|m| !m.is_null()) {
        return Err("non-null `model` job key: main (#294) runs the DSP detector only".into());
    }
    let wav = j["wav"].as_str().ok_or("job without wav")?;
    let mix = media::mix_mode(j["mix"].as_str().unwrap_or("ffmpeg"));
    let dec = media::decode(Path::new(wav), mix).map_err(|e| e.message)?;
    let role = j["role"].as_str().unwrap_or("other");
    let parsed = jobs::parse_params(j["params"].as_object())?;
    let mut params = parsed.params.clone();
    if let Some(r) = j.get("rule").filter(|r| !r.is_null()) {
        params.rule = r
            .as_u64()
            .and_then(drumspike_core::refine::RefineRule::from_version)
            .ok_or_else(|| format!("unknown rule {r}"))?;
    }
    let detected: Vec<(f64, u8)> = match j.get("detected").and_then(Value::as_array) {
        Some(list) => list
            .iter()
            .map(|p| (p[0].as_f64().unwrap_or(f64::NAN), p[1].as_u64().unwrap_or(0) as u8))
            .collect(),
        None => {
            if let Some(e) = &parsed.thresholds_error {
                return Err(e.clone());
            }
            params.thresholds = AnalyzeParams::default().thresholds;
            detect_events(&dec.samples, dec.sr, role, &params.thresholds, Detector::Dsp).map_err(|e| e.to_string())?
        }
    };
    let a = analyze_detected(&dec.samples, dec.sr, role, &detected, &params).map_err(|e| e.to_string())?;
    let events: Vec<Value> = a
        .events
        .iter()
        .map(|h| {
            let mut e = json!({"t": h.t, "v": h.v});
            if let Some(r) = h.rise {
                e["rise"] = json!(r);
            }
            e
        })
        .collect();
    let branches: serde_json::Map<String, Value> = a
        .branches
        .pairs()
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect();
    Ok(json!({
        "events": events,
        "branches": branches,
        "detected": detected.iter().map(|(t, n)| json!([t, n])).collect::<Vec<_>>(),
        "detector": jobs::DETECTOR_DSP,
        "sr": dec.sr,
    }))
}

/// P4-4: run the steps of `<script.json>` (`[{cmd, args}, ...]`) through the test bridge's
/// `invoke` in one service, so a scenario edits, saves and exports as the page does (no HTTP, no
/// page). The string `"$sid"` anywhere in `args` is the sid of the last answer that carried one
/// (`project_open`, `project_new`). Two steps are the tool's own: `wait_analysis {sid}` polls
/// `analyze_status` until the job is neither queued nor running, and `move_file {from, to}`
/// renames a file (a track goes offline or comes back). Prints one JSON line per step:
/// `{cmd, answer}` or `{cmd, error: {code, message}}`; a failed step does not stop the script.
fn run(script: &str) -> Result<(), String> {
    use drumspike_app::bridge::{Bridge, Reply};
    let text = std::fs::read_to_string(script).map_err(|e| format!("{script}: {e}"))?;
    let steps: Vec<Value> = serde_json::from_str(&text).map_err(|e| format!("{script}: {e}"))?;
    let statics = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../static");
    let bridge = Bridge::new(service(), statics);
    let mut sid = String::new();
    for step in &steps {
        let cmd = step["cmd"].as_str().ok_or("a step without cmd")?;
        let args = with_sid(step.get("args").cloned().unwrap_or(json!({})), &sid);
        let r: Result<Value, drumspike_app::AppError> = match cmd {
            "wait_analysis" => loop {
                let st = bridge.invoke("analyze_status", &args).map(|r| match r {
                    Reply::Json(v) => v,
                    Reply::Bytes(_) => Value::Null,
                });
                match st {
                    Ok(v) if v["state"] == "queued" || v["state"] == "running" => {
                        std::thread::sleep(std::time::Duration::from_millis(20))
                    }
                    other => break other,
                }
            },
            "move_file" => {
                let (from, to) = (args["from"].as_str().unwrap_or(""), args["to"].as_str().unwrap_or(""));
                std::fs::rename(from, to)
                    .map(|()| json!({"moved": to}))
                    .map_err(|e| drumspike_app::AppError::bad(format!("{from}: {e}")))
            }
            _ => bridge.invoke(cmd, &args).map(|r| match r {
                Reply::Json(v) => v,
                Reply::Bytes(b) => json!({"bytes": b.len()}),
            }),
        };
        let line = match r {
            Ok(v) => {
                if let Some(s) = v.get("sid").and_then(Value::as_str) {
                    sid = s.to_string();
                }
                json!({"cmd": cmd, "answer": v})
            }
            Err(e) => json!({"cmd": cmd, "error": {"code": e.code, "message": e.message}}),
        };
        println!("{line}");
    }
    Ok(())
}

/// `v` with every string `"$sid"` replaced by `sid`.
fn with_sid(v: Value, sid: &str) -> Value {
    match v {
        Value::String(s) if s == "$sid" => Value::String(sid.to_string()),
        Value::Array(a) => Value::Array(a.into_iter().map(|x| with_sid(x, sid)).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, x)| (k, with_sid(x, sid))).collect()),
        other => other,
    }
}

fn main() -> ExitCode {
    // P3-15b: built with `--features trace` and started with `DRUMSPIKE_TRACE=<file.json>`, the
    // tool records the job stages (`analyze:decode`, `analyze:prepare`, `analyze:detect`).
    drumspike_app::trace::init_from_env(0.0);
    let args: Vec<String> = std::env::args().skip(1).collect();
    // P4-5: `service_tool cli ...` and `service_tool mcp` are the command-line tool and the MCP server of the app
    // (the same engine; the app binary carries them as `drumspike-desktop cli` and `drumspike-desktop mcp`)
    let models = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
    if let Some(code) = drumspike_app::agent::entry("service_tool", &args, &models) {
        return ExitCode::from(code as u8);
    }
    let r = match args.first().map(String::as_str) {
        Some("analyze") if args.len() > 1 => analyze(&args[1..]),
        Some("export") if args.len() == 2 => export(&args[1]),
        Some("spikes") if args.len() > 1 => spikes(&args[1..]),
        Some("velocities") if args.len() == 2 => velocities(&args[1]),
        Some("export-all") if args.len() == 2 => export_all(&args[1]),
        Some("core-analyze") if args.len() == 2 => core_analyze(&args[1]),
        Some("run") if args.len() == 2 => run(&args[1]),
        Some("learn-eval") if args.len() == 2 => learn_eval::run(&args[1]),
        _ => Err("usage: service_tool analyze <project_dir>... | export <jobs.json> | spikes <project_dir>... | velocities <jobs.json> | export-all <project_dir> | core-analyze <jobs.json> | run <script.json> | learn-eval <spec.json>".into()),
    };
    drumspike_app::trace::finish();
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("service_tool: {e}");
            ExitCode::FAILURE
        }
    }
}
