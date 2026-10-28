//! P4-5 step 3: the command-line tool. One subcommand per `Agent` row of the registry (`api`), JSON in and out.
//!
//! ```text
//! drumspike <command> [--<arg> <value>]... [--args '<json>' | --args -] [--project <path>] [--no-save] [--pretty]
//! drumspike help [<command>]        the commands, or one row (arguments, result schema, effect)
//! drumspike commands                the whole registry as JSON (what `drumspike mcp` lists as tools)
//! drumspike batch                   JSON lines on stdin: {"command", "args", "wait"?}; an answer line each
//! drumspike run <script.json>       the same from a file: a list of steps, stops at the first error
//! ```
//!
//! Output: the result as one JSON value on stdout (`--pretty` indents it), progress as JSON lines on stderr
//! (`{"progress": ...}`), an error as `{"error": {"code", "message"}}` on stderr. Exit codes: 0 ok, 1 another error
//! (500) or a failed analysis track, 2 bad usage or argument (400), 3 not found (404), 4 conflict or locked (409).
//!
//! One call is one process. `--project <path>` replaces `--sid`: the process opens the project (for writing when the
//! command changes it, else read-only), runs the command, saves when the command changed the project (unless
//! `--no-save`) and closes. A command that starts a job (`Poll` row) waits for it. `batch` and `run` keep the
//! service, so a `sid` stays valid across steps (`"$sid"` in an argument is the sid of the last answer that had
//! one), nothing is saved unless a step is `project_save`, and a step's `"wait": true` waits for its job.

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::api::{self, CommandSpec, Run, Ty};
use crate::call::call;
use crate::reply::Reply;
use crate::{AppError, Progress, Service};

/// The streams of one run (tests pass buffers).
pub struct Io {
    pub input: Box<dyn BufRead + Send>,
    pub out: Box<dyn Write + Send>,
    pub err: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl Io {
    /// The process's stdin, stdout and stderr.
    pub fn stdio() -> Io {
        Io {
            input: Box::new(std::io::BufReader::new(std::io::stdin())),
            out: Box::new(std::io::stdout()),
            err: Arc::new(Mutex::new(Box::new(std::io::stderr()))),
        }
    }

    fn say_err(&self, line: &str) {
        let mut e = self.err.lock().unwrap_or_else(|p| p.into_inner());
        let _ = writeln!(e, "{line}");
    }
}

/// The exit code of an error: 2 for 400, 3 for 404, 4 for 409, else 1. An unknown track is a 400 of the service (the
/// page reads that code), but it is "not found" for a caller of this tool: 3, like an unknown command or project.
pub fn exit_code(e: &AppError) -> i32 {
    match e.code {
        400 if e.message.starts_with("unknown track") => 3,
        400 => 2,
        404 => 3,
        409 => 4,
        _ => 1,
    }
}

fn error_json(e: &AppError) -> Value {
    json!({"error": {"code": e.code, "message": e.message}})
}

/// The flags of the global options and of the command, parsed against its row.
struct Parsed {
    args: Map<String, Value>,
    project: Option<String>,
    save: bool,
    pretty: bool,
}

fn usage(msg: impl Into<String>) -> AppError {
    AppError::bad(msg)
}

fn flag_value(row: &CommandSpec, name: &str, raw: &str) -> Result<Value, AppError> {
    let arg = row.args.iter().find(|a| a.name == name).ok_or_else(|| {
        usage(format!(
            "unknown flag --{name} for {}: try `drumspike help {}`",
            row.name, row.name
        ))
    })?;
    let bad = |what: &str| usage(format!("--{name}: expected {what}, got `{raw}`"));
    Ok(match arg.ty {
        Ty::Str => Value::String(raw.to_string()),
        Ty::Int => Value::from(raw.parse::<i64>().map_err(|_| bad("an integer"))?),
        Ty::Num => Value::from(raw.parse::<f64>().map_err(|_| bad("a number"))?),
        Ty::Bool => Value::Bool(match raw {
            "true" | "1" | "yes" => true,
            "false" | "0" | "no" => false,
            _ => return Err(bad("true or false")),
        }),
        Ty::Strs if !raw.trim_start().starts_with('[') => Value::Array(vec![Value::String(raw.to_string())]),
        Ty::Any | Ty::TempoMap => serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string())),
        Ty::Strs | Ty::Obj | Ty::Objs => serde_json::from_str(raw).map_err(|e| bad(&format!("JSON ({e})")))?,
    })
}

fn take(rest: &[String], i: &mut usize, flag: &str) -> Result<String, AppError> {
    let v = rest
        .get(*i)
        .cloned()
        .ok_or_else(|| usage(format!("--{flag} needs a value")))?;
    *i += 1;
    Ok(v)
}

fn parse_flags(row: &CommandSpec, rest: &[String], io: &mut Io) -> Result<Parsed, AppError> {
    let mut p = Parsed {
        args: Map::new(),
        project: None,
        save: true,
        pretty: false,
    };
    let mut i = 0;
    while i < rest.len() {
        let tok = &rest[i];
        let Some(flag) = tok.strip_prefix("--") else {
            return Err(usage(format!("unexpected `{tok}`: arguments are flags, --name value")));
        };
        i += 1;
        let name = flag.replace('-', "_");
        match name.as_str() {
            "pretty" => p.pretty = true,
            "no_save" => p.save = false,
            "project" => p.project = Some(take(rest, &mut i, flag)?),
            "args" => {
                let text = take(rest, &mut i, flag)?;
                let text = if text == "-" {
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut io.input, &mut s).map_err(|e| usage(format!("stdin: {e}")))?;
                    s
                } else {
                    text
                };
                let v: Value = serde_json::from_str(&text).map_err(|e| usage(format!("--args: {e}")))?;
                let Value::Object(m) = v else {
                    return Err(usage("--args must be a JSON object"));
                };
                p.args.extend(m);
            }
            other => {
                let is_bool = row.args.iter().any(|a| a.name == other && a.ty == Ty::Bool);
                let raw = if is_bool && !rest.get(i).is_some_and(|v| matches!(v.as_str(), "true" | "false")) {
                    "true".to_string()
                } else {
                    take(rest, &mut i, flag)?
                };
                let v = flag_value(row, other, &raw)?;
                if row.args.iter().any(|a| a.name == other && a.ty == Ty::Strs) {
                    if let (Some(Value::Array(old)), Value::Array(new)) = (p.args.get_mut(other), &v) {
                        old.extend(new.clone()); // a repeated flag adds to the list
                        continue;
                    }
                }
                p.args.insert(other.to_string(), v);
            }
        }
    }
    Ok(p)
}

/// Run a command; a `Poll` row waits for its job when `wait` (the answer then has `status`).
pub fn exec(
    svc: &Service,
    row: &CommandSpec,
    args: &Value,
    wait: bool,
    progress: Option<Progress>,
) -> Result<Value, AppError> {
    let r = match call(svc, row.name, args, progress.clone())? {
        Reply::Json(v) => v,
        Reply::Bytes(_) => return Err(AppError::bad("this command answers bytes")),
    };
    let Run::Poll(status) = row.run else {
        return Ok(r);
    };
    if !wait {
        return Ok(r);
    }
    let sid = args["sid"].clone();
    let st = api::find(status).expect("a Poll row names a status row");
    let mut last = Value::Null;
    loop {
        let s = match call(svc, st.name, &json!({"sid": sid}), None)? {
            Reply::Json(v) => v,
            Reply::Bytes(_) => Value::Null,
        };
        if s != last {
            if let Some(p) = &progress {
                p(json!({"sid": sid, "status": s}));
            }
            last = s.clone();
        }
        if !matches!(s["state"].as_str(), Some("queued" | "running")) {
            let mut out = r.clone();
            out["status"] = s;
            return Ok(out);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// True when a job's status has a track with an error.
fn job_failed(v: &Value) -> bool {
    v["status"]["tracks"]
        .as_object()
        .is_some_and(|t| t.values().any(|x| x["state"] == "error"))
}

fn progress_to(io: &Io) -> Progress {
    let err = Arc::clone(&io.err);
    Arc::new(move |v: Value| {
        let mut e = err.lock().unwrap_or_else(|p| p.into_inner());
        let _ = writeln!(e, "{}", json!({"progress": v}));
    })
}

fn print_json(io: &mut Io, v: &Value, pretty: bool) {
    let text = if pretty {
        serde_json::to_string_pretty(v)
    } else {
        serde_json::to_string(v)
    }
    .unwrap_or_default();
    let _ = writeln!(io.out, "{text}");
    let _ = io.out.flush();
}

/// `drumspike help` text.
pub fn help_text(command: Option<&str>) -> Result<String, AppError> {
    let Some(name) = command else {
        let mut s = String::from("drumspike <command> [--name value]... [--project <path>] [--no-save] [--pretty]\n\n");
        let w = api::agent_commands().map(|c| c.name.len()).max().unwrap_or(0);
        for c in api::agent_commands() {
            s.push_str(&format!("  {:w$}  {}\n", c.name, c.about));
        }
        s.push_str("\nAlso: help <command>, commands, batch, run <script.json>, mcp\n");
        return Ok(s);
    };
    let c = api::agent_commands()
        .find(|c| c.name == name)
        .ok_or_else(|| AppError::not_found(format!("unknown command {name}")))?;
    let mut s = format!("{}: {}\n", c.name, c.about);
    s.push_str(&format!(
        "changes the project: {}   runs: {}\n\narguments:\n",
        if c.writes_project() { "yes" } else { "no" },
        match c.run {
            Run::Quick => "at once".to_string(),
            Run::Poll(st) => format!("starts a job, waits for it (status: {st})"),
            Run::Progress => "with progress lines until done".to_string(),
        }
    ));
    if c.args.is_empty() {
        s.push_str("  none\n");
    }
    for a in c.args {
        s.push_str(&format!(
            "  --{} <{:?}>{}  {}\n",
            a.name.replace('_', "-"),
            a.ty,
            if a.required { " (required)" } else { "" },
            a.about
        ));
    }
    s.push_str(&format!("\nresult: {}\n", c.result));
    Ok(s)
}

/// The registry of the agent surface as JSON: what `drumspike commands` prints.
pub fn commands_json() -> Value {
    let rows: Vec<Value> = api::agent_commands()
        .map(|c| {
            json!({
                "name": c.name,
                "description": c.about,
                "inputSchema": c.args_schema(),
                "resultSchema": c.result_schema(),
                "writes_project": c.writes_project(),
                "run": match c.run { Run::Quick => "quick", Run::Poll(_) => "poll", Run::Progress => "progress" },
            })
        })
        .collect();
    json!({"commands": rows})
}

fn with_sid(v: &Value, sid: &str) -> Value {
    match v {
        Value::String(s) if s == "$sid" => Value::String(sid.to_string()),
        Value::Array(a) => Value::Array(a.iter().map(|x| with_sid(x, sid)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), with_sid(x, sid))).collect()),
        other => other.clone(),
    }
}

/// One step of `batch` or `run`: `{command | cmd, args?, wait?}`; the answer line is
/// `{command, ok: true, result}` or `{command, ok: false, error}`.
fn step(svc: &Service, v: &Value, sid: &mut String, progress: &Progress) -> (Value, Option<AppError>) {
    let name = v
        .get("command")
        .or_else(|| v.get("cmd"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let line = |r: Result<Value, AppError>| match r {
        Ok(res) => (json!({"command": name, "ok": true, "result": res}), None),
        Err(e) => (
            json!({"command": name, "ok": false, "error": error_json(&e)["error"]}),
            Some(e),
        ),
    };
    let Some(row) = api::agent_commands().find(|c| c.name == name) else {
        return line(Err(AppError::not_found(format!("unknown command {name}"))));
    };
    let args = with_sid(v.get("args").unwrap_or(&Value::Null), sid);
    let wait = v.get("wait").and_then(Value::as_bool).unwrap_or(false);
    let r = exec(svc, row, &args, wait, Some(progress.clone()));
    if let Ok(res) = &r {
        if let Some(s) = res.get("sid").and_then(Value::as_str) {
            *sid = s.to_string();
        }
    }
    line(r)
}

fn batch(svc: &Service, io: &mut Io) -> i32 {
    let progress = progress_to(io);
    let mut sid = String::new();
    let mut l = String::new();
    // one line at a time: an answer is out before the next line is read (a caller may wait for it)
    while io.input.read_line(&mut l).map(|n| n > 0).unwrap_or(false) {
        let text = std::mem::take(&mut l);
        if text.trim().is_empty() {
            continue;
        }
        let out = match serde_json::from_str::<Value>(&text) {
            Ok(v) => step(svc, &v, &mut sid, &progress).0,
            Err(e) => json!({"command": "", "ok": false, "error": {"code": 400, "message": format!("not JSON: {e}")}}),
        };
        print_json(io, &out, false);
    }
    svc.wait_idle(&|v| progress(json!({"exit": v}))); // a job that is still running (a copy Save As) finishes first
    0
}

fn run_script(svc: &Service, path: &str, io: &mut Io) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            io.say_err(&error_json(&AppError::not_found(format!("{path}: {e}"))).to_string());
            return 3;
        }
    };
    let steps: Vec<Value> = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Array(a)) => a,
        Ok(Value::Object(m)) if m.get("steps").is_some_and(Value::is_array) => {
            m["steps"].as_array().cloned().unwrap_or_default()
        }
        Ok(_) | Err(_) => {
            io.say_err(&error_json(&AppError::bad(format!("{path}: expected a list of steps"))).to_string());
            return 2;
        }
    };
    let progress = progress_to(io);
    let mut sid = String::new();
    for s in &steps {
        let (out, err) = step(svc, s, &mut sid, &progress);
        print_json(io, &out, false);
        if let Some(e) = err {
            svc.wait_idle(&|v| progress(json!({"exit": v})));
            return exit_code(&e);
        }
    }
    svc.wait_idle(&|v| progress(json!({"exit": v})));
    0
}

/// The end of the copy job of `sid` as an answer: `{job: <final state>, result?}`; an error or a cancel is an error.
fn finished_job(svc: &Service, sid: &str) -> Result<Value, AppError> {
    let st = svc.job_status(sid);
    match st["state"].as_str() {
        Some("done") => {
            let mut out = json!({"job": st.clone()});
            if let Some(r) = st.get("result") {
                out["result"] = r.clone();
            }
            Ok(out)
        }
        Some("cancelled") => Err(AppError::conflict("the copy was cancelled")),
        Some("error") => Err(AppError::internal(
            st["error"].as_str().unwrap_or("the copy failed").to_string(),
        )),
        _ => Err(AppError::internal(format!("the copy job did not end: {st}"))),
    }
}

/// The one-shot command with its flags.
fn one_shot(svc: &Service, row: &CommandSpec, rest: &[String], io: &mut Io) -> i32 {
    let p = match parse_flags(row, rest, io) {
        Ok(p) => p,
        Err(e) => {
            io.say_err(&error_json(&e).to_string());
            return exit_code(&e);
        }
    };
    let mut args = p.args.clone();
    let mut opened: Option<String> = None;
    if let Some(path) = &p.project {
        if !row.args.iter().any(|a| a.name == "sid") || args.contains_key("sid") {
            let e = usage(format!(
                "--project is for commands that take a sid; {} does not, or --sid is given",
                row.name
            ));
            io.say_err(&error_json(&e).to_string());
            return exit_code(&e);
        }
        let open = api::find("project_open").expect("the registry has project_open");
        let a = json!({"path": path, "read_only": !crate::call::changes_project(row, &p.args)});
        match exec(svc, open, &a, false, None) {
            Ok(summary) => {
                let sid = summary["sid"].as_str().unwrap_or_default().to_string();
                args.insert("sid".into(), Value::String(sid.clone()));
                opened = Some(sid);
            }
            Err(e) => {
                io.say_err(&error_json(&e).to_string());
                return exit_code(&e);
            }
        }
    }
    let progress = progress_to(io);
    let sid_arg = args.get("sid").and_then(Value::as_str).map(str::to_string);
    let result = exec(svc, row, &Value::Object(args), true, Some(progress.clone()));
    svc.wait_idle(&|v| progress(json!({"exit": v}))); // a copy job answers at once and runs on: wait before saving and closing
                                                      // a copy job answered `{job}` when it started; the one-shot prints how it ended (`job` and, when done, `result`)
    let result = match (result, sid_arg) {
        (Ok(v), Some(sid))
            if v.get("job").is_some() && matches!(row.name, "project_save_as" | "project_consolidate") =>
        {
            finished_job(svc, &sid)
        }
        (r, _) => r,
    };
    let mut code = 0;
    let mut saved_err = None;
    if let (Some(sid), Ok(_)) = (&opened, &result) {
        let dirty = svc.project_get(sid).map(|s| s["dirty"] == true).unwrap_or(false);
        if crate::call::changes_project(row, &p.args) && p.save && dirty && row.name != "project_save" {
            if let Err(e) = svc.project_save(sid) {
                saved_err = Some(e);
            }
        }
    }
    if let Some(sid) = &opened {
        let _ = svc.project_close(sid, true);
    }
    match (result, saved_err) {
        (Ok(v), None) => {
            if job_failed(&v) {
                code = 1;
            }
            print_json(io, &v, p.pretty);
        }
        (Ok(v), Some(e)) => {
            print_json(io, &v, p.pretty);
            io.say_err(&error_json(&e).to_string());
            code = exit_code(&e);
        }
        (Err(e), _) => {
            io.say_err(&error_json(&e).to_string());
            code = exit_code(&e);
        }
    }
    code
}

/// Run the command line `args` (after the program name and the `cli` word). Returns the exit code.
pub fn run(svc: &Service, args: &[String], io: &mut Io) -> i32 {
    let Some(first) = args.first().map(String::as_str) else {
        let _ = write!(io.out, "{}", help_text(None).unwrap_or_default());
        return 2;
    };
    match first {
        "help" | "--help" | "-h" => match help_text(args.get(1).map(String::as_str)) {
            Ok(t) => {
                let _ = write!(io.out, "{t}");
                0
            }
            Err(e) => {
                io.say_err(&error_json(&e).to_string());
                exit_code(&e)
            }
        },
        "commands" => {
            print_json(io, &commands_json(), args.iter().any(|a| a == "--pretty"));
            0
        }
        "batch" => batch(svc, io),
        "run" => match args.get(1) {
            Some(path) => run_script(svc, path, io),
            None => {
                io.say_err(&error_json(&usage("run needs a script file")).to_string());
                2
            }
        },
        name => match api::agent_commands().find(|c| c.name == name) {
            Some(row) => one_shot(svc, row, &args[1..], io),
            None => {
                let e = AppError::not_found(format!("unknown command {name}: try `drumspike help`"));
                io.say_err(&error_json(&e).to_string());
                exit_code(&e)
            }
        },
    }
}

#[cfg(test)]
mod tests;
