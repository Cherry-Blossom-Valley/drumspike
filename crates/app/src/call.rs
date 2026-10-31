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

use crate::api::{self, Surface};
use crate::command_policy::{self as policy, arg_i64, arg_str, opt_i64};
use crate::reply::{Channels, Reply};
use crate::{AppError, ListShape, Progress, Service};

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
            // #164 (part 2 of #153): the feedback gateway (the page previews, this posts)
            | "feedback_send"
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
            json(
                s.analyze_start_keep(
                    sid,
                    tracks,
                    params,
                    detector.as_deref(),
                    progress,
                    args.get("keep_edited")
                        .and_then(Value::as_bool)
                        .unwrap_or(policy::PAGE_KEEP_EDITED),
                ),
            )
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
        // #164 (part 2 of #153): the page previews the payload, this posts exactly that JSON
        "feedback_send" => json(s.feedback_send(
            args.get("payload").unwrap_or(&Value::Null),
            args.get("url").and_then(Value::as_str),
        )),
        other => Err(AppError::not_found(format!("unknown command {other}"))),
    }
}

/// Run a command of the agent surface (see the module docs). `progress` gets the messages of a command that
/// takes a `progress` channel; the call answers when the command does (a `Poll` row answers at once).
///
/// # Errors
///
/// A 404 [`AppError`] when `name` is not a command of the agent surface; a 400 when `args` is neither an object
/// nor `null` or fails the row's argument check; a 409 when the command changes a project that is open
/// read-only; otherwise the error of the command itself.
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
    policy::check_args(row, &a)?;
    policy::fill_agent_defaults(name, &mut a);
    if let Some(refusal) = policy::read_only_refusal(svc, row, &a) {
        return Err(refusal);
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
