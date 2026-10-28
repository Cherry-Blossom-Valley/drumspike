//! P4-5 step 3: the one dispatcher of the commands (the match that `bridge.rs` held). The test bridge, the
//! command-line tool and the MCP server all call it; the Tauri wrappers call the same `Service` methods.
//!
//! * [`run`] runs one command by name with JSON arguments (the names and argument names are those of the Tauri
//!   wrappers); `None` for a name it does not know (the bridge then tries the transport).
//! * [`call`] is the agent entry: it checks the name against the registry (`api`), the arguments against the row
//!   (required, types, no unknown key), fills the agent defaults (`keep_edited` is true), refuses a command that
//!   changes the project when the project is open read-only, and gives a progress callback to the commands that
//!   take a `progress` channel.

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::api::{self, CommandSpec, Surface, Ty};
use crate::reply::{Channels, Reply};
use crate::{AppError, ListShape, Progress, Service};

pub(crate) fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (string)")))
}

pub(crate) fn arg_i64(args: &Value, key: &str) -> Result<i64, AppError> {
    args.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (integer)")))
}

pub(crate) fn opt_i64(args: &Value, key: &str) -> Result<Option<i64>, AppError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| AppError::bad(format!("wrong argument `{key}` (integer)"))),
    }
}

/// Run one command with JSON arguments; `None` when `cmd` is not one this function runs.
pub fn run(svc: &Service, channels: &Arc<Channels>, cmd: &str, args: &Value) -> Option<Result<Reply, AppError>> {
    let s = svc;
    let json = |r: Result<Value, AppError>| r.map(Reply::Json);
    if matches!(
        cmd,
        "track_set_mix"
            | "app_info"
            | "project_open"
            | "project_get"
            | "project_list"
            | "tracks_get"
            | "events_get"
            | "events_set"
            | "peaks_info"
            | "peaks_tile"
            | "samples_window"
            | "track_layer_wave"
            | "track_sampler_wave"
            | "analyze_start"
            | "analyze_status"
            | "export_midi"
            | "export_spikes"
            | "export_all"
            | "velocities_recompute"
            | "export_existing"
            | "export_dest_set"
            | "export_markers_set"
            | "export_drag_prepare"
            | "export_drag_take"
            | "export_last_get"
            | "export_last_set"
    ) {
        return Some(run_direct(svc, channels, cmd, args));
    }
    let r = crate::lifecycle::dispatch(s, channels, cmd, args)
        .or_else(|| crate::import::dispatch(s, cmd, args))
        .or_else(|| crate::plugins::dispatch(s, cmd, args))
        .or_else(|| crate::tracks::dispatch(s, channels, cmd, args))
        .or_else(|| crate::copyjob::dispatch(s, channels, cmd, args))
        .or_else(|| crate::relink::dispatch(s, channels, cmd, args))
        .or_else(|| crate::histogram::dispatch(s, cmd, args))
        .or_else(|| crate::edit::dispatch(s, cmd, args))
        .or_else(|| crate::learn::dispatch(s, cmd, args))
        .or_else(|| crate::tempo::dispatch(s, cmd, args))
        .or_else(|| crate::markers::dispatch(s, cmd, args))
        .or_else(|| crate::rowheights::dispatch(s, cmd, args))
        .or_else(|| crate::confidence::dispatch(s, cmd, args))
        .or_else(|| crate::layer::dispatch(s, cmd, args))
        .or_else(|| crate::sampler::dispatch(s, cmd, args))?;
    Some(json(r))
}

fn run_direct(svc: &Service, channels: &Arc<Channels>, cmd: &str, args: &Value) -> Result<Reply, AppError> {
    let s = svc;
    let json = |r: Result<Value, AppError>| r.map(Reply::Json);
    match cmd {
        "app_info" => Ok(Reply::Json(s.app_info())),
        "project_open" => json(s.project_open_with(
            arg_str(args, "path")?,
            args.get("read_only").and_then(Value::as_bool).unwrap_or(false),
        )),
        "project_get" => json(s.project_get(arg_str(args, "sid")?)),
        "project_list" => Ok(Reply::Json(
            s.project_list(ListShape::parse(args.get("shape").and_then(Value::as_str))),
        )),
        "tracks_get" => json(s.tracks_get(arg_str(args, "sid")?)),
        "events_get" => json(s.events_get(arg_str(args, "sid")?, arg_str(args, "name")?)),
        "events_set" => {
            let events = args.get("events").cloned().unwrap_or(Value::Null);
            json(s.events_set(arg_str(args, "sid")?, arg_str(args, "name")?, &events))
        }
        "peaks_info" => json(s.peaks_info(arg_str(args, "sid")?, arg_str(args, "name")?)),
        "peaks_tile" => {
            let level = usize::try_from(arg_i64(args, "level")?).map_err(|_| AppError::bad("level out of range"))?;
            s.peaks_tile(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                level,
                arg_i64(args, "from")?,
                arg_i64(args, "to")?,
            )
            .map(Reply::Bytes)
        }
        "samples_window" => s
            .samples_window(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                arg_i64(args, "start")?,
                arg_i64(args, "end")?,
            )
            .map(Reply::Bytes),
        // P4-40e: the Layer view's waveforms (the one-shot, the sampler bounce)
        "track_layer_wave" => s
            .track_layer_wave(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                args.get("start").and_then(Value::as_i64).unwrap_or(0),
                args.get("end").and_then(Value::as_i64).unwrap_or(i64::MAX),
            )
            .map(Reply::Bytes),
        "track_sampler_wave" => s
            .track_sampler_wave(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                args.get("start").and_then(Value::as_i64).unwrap_or(0),
                args.get("end").and_then(Value::as_i64).unwrap_or(i64::MAX),
            )
            .map(Reply::Bytes),
        "analyze_start" => {
            let sid = arg_str(args, "sid")?;
            let tracks = match args.get("tracks") {
                None | Some(Value::Null) => None,
                Some(Value::Array(a)) => Some(
                    a.iter()
                        .map(|v| {
                            v.as_str()
                                .map(str::to_string)
                                .ok_or_else(|| AppError::bad("`tracks` must hold strings"))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                Some(_) => return Err(AppError::bad("`tracks` must be a list")),
            };
            let params: Option<&Map<String, Value>> = args.get("params").and_then(Value::as_object);
            // #388: `--detector <id>`: one detector for every track of the analysis; absent
            // keeps each track's stored `params.detector` (#387).
            let detector = args.get("detector").and_then(Value::as_str).map(str::to_string);
            let progress = args
                .get("progress")
                .and_then(|c| c.get("__channel"))
                .and_then(Value::as_str)
                .map(|id| {
                    let (ch, id) = (channels.clone(), id.to_string());
                    let p: Progress = Arc::new(move |v: Value| ch.push(&id, v));
                    p
                });
            json(s.analyze_start_keep(
                sid,
                tracks,
                params,
                detector.as_deref(),
                progress,
                args.get("keep_edited").and_then(Value::as_bool).unwrap_or(false),
            ))
        }
        "analyze_status" => Ok(Reply::Json(s.analyze_status(arg_str(args, "sid")?))),
        "export_midi" => {
            // P4-5: a call without `note` or `events` takes them from the document (`export_midi_stored`)
            let events: Option<Vec<Value>> = args.get("events").and_then(Value::as_array).cloned();
            let o = crate::export::ExportOpts::from_args(args)?;
            json(s.export_midi_stored_with(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                opt_i64(args, "note")?,
                events.as_deref(),
                opt_i64(args, "ppq")?,
                args.get("tempo").and_then(Value::as_f64),
                o.dest.as_deref(),
                o.compensate,
            ))
        }
        "export_spikes" | "export_all" | "velocities_recompute" => {
            crate::export::bridge_call(s, channels, cmd, args).map(Reply::Json)
        }
        "track_set_mix" => {
            let f = |k: &str| match args.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v
                    .as_f64()
                    .map(Some)
                    .ok_or_else(|| AppError::bad(format!("wrong argument `{k}` (number)"))),
            };
            json(s.track_set_mix(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                f("volume_db")?,
                f("pan")?,
            ))
        }
        "export_existing"
        | "export_dest_set"
        | "export_markers_set"
        | "export_drag_prepare"
        | "export_drag_take"
        | "export_last_get"
        | "export_last_set" => {
            json(crate::shell::dispatch(s, cmd, args).unwrap_or_else(|| Err(AppError::bad("export_existing"))))
        }
        other => Err(AppError::not_found(format!("unknown command {other}"))),
    }
}

/// The arguments an agent gets when it gives none: `(command, key, value)`. The page keeps the old behaviour
/// (the page sends `keep_edited` itself since P4-9 and no `apply`: it applies the velocities as one undo step).
const AGENT_DEFAULTS: [(&str, &str, bool); 3] = [
    ("analyze_start", "keep_edited", true),
    ("track_repick", "keep_edited", true),
    ("track_velocity_set", "apply", true),
];

fn type_ok(ty: Ty, v: &Value) -> bool {
    match ty {
        Ty::Str => v.is_string(),
        Ty::Int => v.is_i64() || v.is_u64(),
        Ty::Num => v.is_number(),
        Ty::Bool => v.is_boolean(),
        Ty::Strs => v.as_array().is_some_and(|a| a.iter().all(Value::is_string)),
        Ty::Obj => v.is_object(),
        Ty::Objs => v.as_array().is_some_and(|a| a.iter().all(Value::is_object)),
        Ty::Any | Ty::TempoMap => true,
    }
}

fn type_name(ty: Ty) -> &'static str {
    match ty {
        Ty::Str => "string",
        Ty::Int => "integer",
        Ty::Num => "number",
        Ty::Bool => "boolean",
        Ty::Strs => "list of strings",
        Ty::Obj => "object",
        Ty::Objs => "list of objects",
        Ty::Any => "any value",
        Ty::TempoMap => "tempo map (object) or null",
    }
}

/// Check `args` against the row: no unknown key, every required key, every value of its type (a `null` for an
/// optional argument counts as not given). 400 with the key named.
pub fn check_args(row: &CommandSpec, args: &Map<String, Value>) -> Result<(), AppError> {
    for k in args.keys() {
        if !row.args.iter().any(|a| a.name == k) {
            let known: Vec<&str> = row.args.iter().map(|a| a.name).collect();
            return Err(AppError::bad(format!(
                "unknown argument `{k}` for {}: its arguments are {}",
                row.name,
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                }
            )));
        }
    }
    for a in row.args {
        match args.get(a.name) {
            None | Some(Value::Null) if a.required => {
                return Err(AppError::bad(format!(
                    "missing argument `{}` ({})",
                    a.name,
                    type_name(a.ty)
                )))
            }
            Some(v) if !v.is_null() && !type_ok(a.ty, v) => {
                return Err(AppError::bad(format!(
                    "wrong argument `{}`: expected {}",
                    a.name,
                    type_name(a.ty)
                )))
            }
            _ => {}
        }
    }
    Ok(())
}

/// True when this call changes the project (its document or folder): the registry's `writes_project`, except what is
/// allowed on a read-only project: `track_repick` without `store` and
/// `project_save_as` (the way from a read-only project to a writable copy: the project then is the copy).
pub fn changes_project(row: &CommandSpec, args: &Map<String, Value>) -> bool {
    row.writes_project()
        && match row.name {
            "project_save_as" => false,
            "track_repick" => args.get("store").and_then(Value::as_bool) == Some(true),
            _ => true,
        }
}

/// Run a command of the agent surface (see the module docs). `progress` gets the messages of a command that
/// takes a `progress` channel; the call answers when the command does (a `Poll` row answers at once).
pub fn call(svc: &Service, name: &str, args: &Value, progress: Option<Progress>) -> Result<Reply, AppError> {
    let row = api::find(name)
        .filter(|c| c.surface == Surface::Agent)
        .ok_or_else(|| AppError::not_found(format!("unknown command {name}")))?;
    let mut a = match args {
        Value::Null => Map::new(),
        Value::Object(m) => m.clone(),
        _ => return Err(AppError::bad("the arguments must be an object")),
    };
    a.retain(|_, v| !v.is_null());
    check_args(row, &a)?;
    for (cmd, key, v) in AGENT_DEFAULTS {
        if cmd == name {
            a.entry(key).or_insert(Value::Bool(v));
        }
    }
    if changes_project(row, &a) {
        if let Some(sid) = a.get("sid").and_then(Value::as_str) {
            if svc
                .inner
                .registry
                .get(sid)
                .is_ok_and(|op| crate::registry::lock(&op).read_only)
            {
                return Err(AppError::conflict(format!(
                    "project {sid} is open read-only: {name} changes it; open it for writing (project_open without read_only)"
                )));
            }
        }
    }
    let channels = match progress {
        Some(p) if row.has_progress() => {
            a.insert("progress".into(), serde_json::json!({"__channel": "p"}));
            Arc::new(Channels::with_sink(move |_id, v| p(v)))
        }
        _ => Arc::new(Channels::default()),
    };
    run(svc, &channels, name, &Value::Object(a))
        .unwrap_or_else(|| Err(AppError::not_found(format!("unknown command {name}"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn svc() -> Service {
        Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        })
    }

    #[test]
    fn check_args_names_the_key_and_the_type() {
        let row = api::find("events_edit").unwrap();
        let m = |v: Value| v.as_object().unwrap().clone();
        assert!(check_args(
            row,
            &m(json!({"sid": "a", "name": "n", "ops": [{"op": "add", "t": 1}]}))
        )
        .is_ok());
        for (bad, text) in [
            (json!({"name": "n", "ops": []}), "missing argument `sid`"),
            (
                json!({"sid": 1, "name": "n", "ops": []}),
                "wrong argument `sid`: expected string",
            ),
            (json!({"sid": "a", "name": "n", "ops": [1]}), "expected list of objects"),
            (
                json!({"sid": "a", "name": "n", "ops": [], "x": 1}),
                "unknown argument `x`",
            ),
        ] {
            let e = check_args(row, &m(bad)).unwrap_err();
            assert_eq!(e.code, 400);
            assert!(e.message.contains(text), "{}", e.message);
        }
        assert!(check_args(api::find("app_info").unwrap(), &m(json!({"x": 1})))
            .unwrap_err()
            .message
            .contains("none"));
    }

    #[test]
    fn a_read_only_project_allows_what_does_not_change_it() {
        let m = |v: Value| v.as_object().unwrap().clone();
        let f = |n: &str, a: Value| changes_project(api::find(n).unwrap(), &m(a));
        assert!(f("events_edit", json!({})) && f("project_save", json!({})) && f("analyze_start", json!({})));
        assert!(f("track_repick", json!({"store": true})));
        assert!(!f("track_repick", json!({"store": false})) && !f("track_repick", json!({})));
        assert!(!f("project_save_as", json!({})) && !f("project_get", json!({})));
    }

    #[test]
    fn call_refuses_what_is_not_on_the_agent_surface_and_bad_arguments() {
        let s = svc();
        assert_eq!(call(&s, "transport_play", &json!({}), None).unwrap_err().code, 404);
        assert_eq!(call(&s, "selftest_clock", &json!({}), None).unwrap_err().code, 404);
        assert_eq!(call(&s, "nope", &json!({}), None).unwrap_err().code, 404);
        assert_eq!(call(&s, "project_get", &json!({}), None).unwrap_err().code, 400);
        assert_eq!(call(&s, "project_get", &json!("x"), None).unwrap_err().code, 400);
        assert_eq!(
            call(&s, "project_get", &json!({"sid": "zz"}), None).unwrap_err().code,
            404
        );
        let r = call(&s, "project_list", &Value::Null, None).unwrap();
        assert_eq!(r, Reply::Json(json!([])));
    }

    #[test]
    fn every_agent_row_is_run_by_the_dispatcher() {
        let s = svc();
        let ch = Arc::new(Channels::default());
        let root = std::env::temp_dir().join(format!("ds_call_{}", std::process::id()));
        s.set_home(root.join("home"));
        s.set_projects_dir(root.join("projects"));
        for c in api::agent_commands() {
            if let Some(Err(e)) = run(&s, &ch, c.name, &json!({})) {
                assert!(!e.message.starts_with("unknown command"), "{}: {}", c.name, e.message);
            } else if run(&s, &ch, c.name, &json!({})).is_none() {
                panic!("{}: the dispatcher does not know it", c.name);
            }
        }
        assert!(run(&s, &ch, "nope", &json!({})).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
