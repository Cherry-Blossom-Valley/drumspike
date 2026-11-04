//! R3: the dispatcher of the transport commands ([`TRANSPORT_COMMANDS`]), registered in [`crate::call::run`] like
//! the other modules. Until R3 the test bridge ran them itself (`invoke_transport`) and `call.rs` ran
//! `track_set_mix`. `transport_play` takes a `progress` channel like `analyze_start`; the page fetches the
//! playhead messages with the long poll. All but `track_set_mix` are on the `Window` surface, so
//! [`crate::call::call`] refuses them for an agent.

use std::sync::Arc;

use serde_json::Value;

use super::TRANSPORT_COMMANDS;
use crate::call::{Route, json_reply, routes};
use crate::command_policy::{CLOSE_DEFAULT, arg_str, transport_args};
use crate::reply::Channels;
use crate::{AppError, Progress, Service};

/// R3: the handlers of [`TRANSPORT_COMMANDS`] for [`crate::call::run`].
pub(crate) const HANDLERS: [Route; 10] = routes(TRANSPORT_COMMANDS, |s, ch, cmd, args| {
    json_reply(cmd, dispatch(s, ch, cmd, args))
});

/// Run a transport command with JSON arguments (the names and argument names of the Tauri wrappers); `None` when
/// `cmd` is not one of [`TRANSPORT_COMMANDS`].
#[must_use]
pub fn dispatch(svc: &Service, channels: &Arc<Channels>, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if !TRANSPORT_COMMANDS.contains(&cmd) {
        return None;
    }
    let s = svc;
    let opt_f64 = |key: &str| transport_args::opt_f64(args, key);
    let opt_bool = |key: &str| transport_args::opt_bool(args, key);
    let run = || -> Result<Value, AppError> {
        match cmd {
            "transport_play" => {
                let progress = args
                    .get("progress")
                    .and_then(|c| c.get("__channel"))
                    .and_then(Value::as_str)
                    .map(|id| {
                        let (ch, id) = (channels.clone(), id.to_string());
                        let p: Progress = Arc::new(move |v: Value| ch.push(&id, v));
                        p
                    });
                // P4-40d: an audition window (start, end, loop, tracks); P4-40e: shot (the one-shot preview)
                let win = transport_args::play_window(
                    opt_f64("start")?,
                    opt_f64("end")?,
                    opt_bool("loop"),
                    args.get("tracks"),
                    args.get("shot"),
                )?;
                s.transport_play_window(
                    arg_str(args, "sid")?,
                    opt_f64("position")?,
                    args.get("events"),
                    progress,
                    opt_f64("speed")?,
                    win,
                )
            }
            "transport_stop" => Ok(s.transport_stop(opt_bool("close").unwrap_or(CLOSE_DEFAULT))),
            "transport_seek" => {
                let p = opt_f64("position")?.ok_or_else(|| AppError::bad("missing argument `position` (number)"))?;
                s.transport_seek(p)
            }
            "transport_mix" => s.transport_mix(
                args.get("mute").unwrap_or(&Value::Null),
                args.get("solo").unwrap_or(&Value::Null),
            ),
            "transport_clicks" => s.transport_clicks(opt_bool("enabled"), opt_f64("volume")?, args.get("pitches")),
            "transport_status" => Ok(s.transport_status()),
            "track_set_mix" => {
                let f = |k: &str| match args.get(k) {
                    None | Some(Value::Null) => Ok(None),
                    Some(v) => v
                        .as_f64()
                        .map(Some)
                        .ok_or_else(|| AppError::bad(format!("wrong argument `{k}` (number)"))),
                };
                s.track_set_mix(
                    arg_str(args, "sid")?,
                    arg_str(args, "name")?,
                    f("volume_db")?,
                    f("pan")?,
                )
            }
            "transport_master" => {
                let db = opt_f64("volume_db")?.ok_or_else(|| AppError::bad("missing argument `volume_db` (number)"))?;
                s.transport_master(db)
            }
            "outputs_list" => Ok(s.outputs_list()),
            "output_select" => {
                let device = match args.get("device") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(d)) => Some(d.as_str()),
                    Some(_) => return Err(AppError::bad("wrong argument `device` (string or null)")),
                };
                s.output_select(device)
            }
            other => Err(AppError::not_found(format!("unknown command {other}"))),
        }
    };
    Some(run())
}
