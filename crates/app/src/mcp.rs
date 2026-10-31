//! P4-5 step 4: the MCP server over stdio (`drumspike mcp`). Hand-written JSON-RPC 2.0, one JSON message per line on
//! stdin and stdout (the stdio transport of the Model Context Protocol); the log goes to stderr, nothing else is
//! printed on stdout. No network listener, no MCP crate.
//!
//! Methods: `initialize` (the client's protocol version when it is one of [`VERSIONS`], else the newest; capability
//! `tools`), `notifications/*` (no answer), `ping`, `tools/list`, `tools/call`. Every `Agent` row of the registry is
//! one tool: its name, its one-line text and the argument schema; the result schema is the `outputSchema` when the
//! result is an object. A result is the JSON as text and as `structuredContent`; an error has `isError: true` and
//! `{"error": {code, message}}` as text. Other methods get the JSON-RPC error -32601, an unknown tool -32602.
//!
//! A call runs on its own thread, so a long call does not block `ping` or a second call. Long jobs: a `Poll` row
//! (`analyze_start`, `peaks_build_start`) answers at once and is polled with its status tool. A `Progress` row
//! (`export_all`, `tracks_add`, `project_save_as`, `project_consolidate`) would run longer than a client waits, so
//! its tool starts the work and answers `{"job": "j1"}`; `job_get {job}` answers `{state: running | done | error,
//! progress, result?, error?}`. Nothing is saved unless the agent calls `project_save`; each tool that changes the
//! project in memory says so in its description.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::api::{self, CommandSpec, Run};
use crate::call::call;
use crate::reply::Reply;
use crate::{AppError, Progress, Service};

/// The protocol versions the server answers with, newest first.
pub const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// The extra tool for the jobs of `Progress` rows.
const JOB_GET: &str = "job_get";

const INSTRUCTIONS: &str = "Drum Spike: close-mic drum hit detection with sample-exact MIDI and spike files. Typical order: \
project_open (or project_new, tracks_add), track_params_set, analyze_start then poll analyze_status until state is done, \
track_velocity_set (it recomputes every velocity and stores them with the cut, so do it before single-hit edits), \
events_get and events_edit, project_save, export_all then poll job_get. A project the Drum Spike app has open for \
writing is locked: open it with read_only to read it. Changes stay in memory until project_save.";

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

/// The state of one background job of a `Progress` tool.
#[derive(Clone, Default)]
struct Job {
    state: &'static str,
    progress: Value,
    result: Option<Value>,
    error: Option<Value>,
}

/// The background jobs of the `Progress` tools; one value lives as long as the server (tests keep it across calls).
#[derive(Clone, Default)]
pub struct Jobs(
    Arc<Mutex<(u64, HashMap<String, Job>)>>,
    Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
);

impl Jobs {
    fn start(&self) -> String {
        let mut g = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.0 += 1;
        let id = format!("j{}", g.0);
        g.1.insert(
            id.clone(),
            Job {
                state: "running",
                ..Job::default()
            },
        );
        id
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Job)) {
        if let Some(j) = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .1
            .get_mut(id)
        {
            f(j);
        }
    }

    fn get(&self, id: &str) -> Option<Value> {
        let g = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let j = g.1.get(id)?;
        let mut v = json!({"job": id, "state": j.state, "progress": j.progress});
        if let Some(r) = &j.result {
            v["result"] = r.clone();
        }
        if let Some(e) = &j.error {
            v["error"] = e.clone();
        }
        Some(v)
    }
}

/// True when the tool changes the project in memory only (the agent must call `project_save`).
fn needs_save(c: &CommandSpec) -> bool {
    c.effect == api::Effect::Doc || matches!(c.name, "analyze_start" | "tracks_add")
}

/// The description of a tool: the row's line plus what the agent must know.
fn describe(c: &CommandSpec) -> String {
    let mut d = c.about.to_string();
    if !d.ends_with('.') {
        d.push('.');
    }
    if needs_save(c) {
        d.push_str(" Changes the open project in memory: call project_save to keep it.");
    }
    match c.run {
        Run::Poll(st) => d.push_str(&format!(
            " Starts a job and answers at once: poll {st} until state is done."
        )),
        Run::Progress => d.push_str(" Starts the work and answers {job} at once: poll job_get until state is done."),
        Run::Quick => {}
    }
    if c.name == "analyze_start" || c.name == "track_repick" {
        d.push_str(" Hits you added or moved are kept unless keep_edited is false.");
    }
    d
}

/// The `tools/list` entry of a row.
#[must_use]
pub fn tool(c: &CommandSpec) -> Value {
    let mut t = json!({
        "name": c.name,
        "description": describe(c),
        "inputSchema": c.args_schema(),
        "annotations": {
            "readOnlyHint": !c.writes_project() && c.effect == api::Effect::Read,
            "destructiveHint": false,
        },
    });
    if c.run == Run::Progress {
        // the tool answers the job id at once; the registry's result schema is what `job_get` returns at the end
        t["outputSchema"] = json!({"type": "object", "required": ["job"], "properties": {"job": {"type": "string"}},
                                   "description": "The job id; poll job_get for the result"});
        return t;
    }
    let out = c.result_schema();
    if out["type"] == "object" {
        t["outputSchema"] = out;
    }
    t
}

/// Every tool: the `Agent` rows and `job_get`.
pub fn tools() -> Vec<Value> {
    let mut v: Vec<Value> = api::agent_commands().map(tool).collect();
    v.push(json!({
        "name": JOB_GET,
        "description": "State of a job that a tool started (export_all, tracks_add, project_save_as, project_consolidate): \
                        {job, state: running | done | error, progress, result?, error?}.",
        "inputSchema": {"type": "object", "properties": {"job": {"type": "string", "description": "The job id the tool answered"}},
                        "required": ["job"], "additionalProperties": false},
        "outputSchema": {"type": "object", "required": ["job", "state", "progress"], "properties": {
            "job": {"type": "string"}, "state": {"type": "string"}, "progress": {}}},
        "annotations": {"readOnlyHint": true, "destructiveHint": false},
    }));
    v
}

fn send(out: &Out, v: &Value) {
    let mut o = out.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_result(r: Result<Value, AppError>) -> Value {
    match r {
        Ok(v) => {
            let mut res = json!({"content": [{"type": "text", "text": v.to_string()}], "isError": false});
            if v.is_object() {
                res["structuredContent"] = v;
            }
            res
        }
        Err(e) => json!({
            "content": [{"type": "text", "text": json!({"error": {"code": e.code, "message": e.message}}).to_string()}],
            "isError": true,
        }),
    }
}

/// Run one tool call; a `Progress` row starts a background job and answers its id.
fn run_tool(svc: &Service, jobs: &Jobs, name: &str, args: &Value) -> Result<Value, AppError> {
    if name == JOB_GET {
        let id = args
            .get("job")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::bad("missing argument `job` (string)"))?;
        return jobs
            .get(id)
            .ok_or_else(|| AppError::not_found(format!("no such job {id}")));
    }
    let row = api::agent_commands()
        .find(|c| c.name == name)
        .ok_or_else(|| AppError::not_found(format!("unknown tool {name}")))?;
    if row.run != Run::Progress {
        return match call(svc, name, args, None)? {
            Reply::Json(v) => Ok(v),
            Reply::Bytes(_) => Err(AppError::bad("this tool answers bytes")),
        };
    }
    // check the arguments before the job starts, so a bad call is an error and not a failed job
    crate::command_policy::check_args(row, args.as_object().unwrap_or(&serde_json::Map::new()))?;
    let id = jobs.start();
    let (svc, jobs2, name2, args2) = (svc.clone(), jobs.clone(), name.to_string(), args.clone());
    let jid = id.clone();
    let handle = std::thread::spawn(move || {
        let (j, id) = (jobs2.clone(), jid.clone());
        let progress: Progress = Arc::new(move |v: Value| j.update(&id, |job| job.progress = v));
        let r = call(&svc, &name2, &args2, Some(progress));
        jobs2.update(&jid, |job| match r {
            Ok(Reply::Json(v)) => {
                job.state = "done";
                job.result = Some(v);
            }
            Ok(Reply::Bytes(_)) => job.state = "done",
            Err(e) => {
                job.state = "error";
                job.error = Some(json!({"code": e.code, "message": e.message}));
            }
        });
    });
    jobs.1
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(handle);
    Ok(json!({"job": id}))
}

/// Handle one message; the answer (if any) is sent on `out`. A `tools/call` runs on a thread that is pushed to `workers`.
fn handle(svc: &Service, jobs: &Jobs, out: &Out, msg: &Value, workers: &mut Vec<std::thread::JoinHandle<()>>) {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let Some(id) = id else {
        return; // a notification: no answer
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let want = params["protocolVersion"].as_str().unwrap_or("");
            let version = VERSIONS.iter().find(|v| **v == want).unwrap_or(&VERSIONS[0]);
            send(
                out,
                &json!({"jsonrpc": "2.0", "id": id, "result": {
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "drumspike", "version": drumspike_core::VERSION},
                    "instructions": INSTRUCTIONS,
                }}),
            );
        }
        "ping" => send(out, &json!({"jsonrpc": "2.0", "id": id, "result": {}})),
        "tools/list" => send(out, &json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools()}})),
        "tools/call" => {
            let Some(name) = params["name"].as_str().map(str::to_string) else {
                send(out, &rpc_error(&id, -32602, "tools/call needs a tool name"));
                return;
            };
            if name != JOB_GET && !api::agent_commands().any(|c| c.name == name) {
                send(out, &rpc_error(&id, -32602, &format!("Unknown tool: {name}")));
                return;
            }
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            let (svc, jobs, out) = (svc.clone(), jobs.clone(), Arc::clone(out));
            workers.push(std::thread::spawn(move || {
                let r = run_tool(&svc, &jobs, &name, &args);
                send(&out, &json!({"jsonrpc": "2.0", "id": id, "result": tool_result(r)}));
            }));
        }
        _ => send(out, &rpc_error(&id, -32601, &format!("Method not found: {method}"))),
    }
}

/// Serve until `input` ends; then wait for the calls that still run. Returns when everything is answered.
pub fn serve(svc: &Service, input: impl BufRead, out: Box<dyn Write + Send>) {
    serve_with(svc, &Jobs::default(), input, out);
}

/// [`serve`] with the job table of an earlier run (so `job_get` finds the jobs that run started).
pub fn serve_with(svc: &Service, jobs: &Jobs, input: impl BufRead, out: Box<dyn Write + Send>) {
    let out: Out = Arc::new(Mutex::new(out));
    let jobs = jobs.clone();
    let mut workers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(_)) => send(&out, &rpc_error(&Value::Null, -32600, "batches are not supported")),
            Ok(msg) => handle(svc, &jobs, &out, &msg, &mut workers),
            Err(e) => send(&out, &rpc_error(&Value::Null, -32700, &format!("Parse error: {e}"))),
        }
        workers.retain(|w| !w.is_finished());
    }
    for w in workers {
        let _ = w.join();
    }
    // P4-5 (L3): stdin is closed, but a job started by a `Progress` tool or a copy or analysis job may still run: the
    // process ends only when they are done, so no destination stays half made (progress on stderr)
    let handles: Vec<_> = std::mem::take(&mut *jobs.1.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    for h in handles {
        let _ = h.join();
    }
    svc.wait_idle(&|v| eprintln!("{}", json!({"progress": {"exit": v}})));
}

#[cfg(test)]
mod tests;
