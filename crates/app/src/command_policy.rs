//! R15 step 3: the one policy layer of the commands. Every default value, every argument check
//! and every refusal that depends on the open project is decided here, so the agent surface
//! ([`crate::call`]), the test bridge ([`crate::bridge`]) and the Tauri wrappers answer the same
//! request the same way. The adapters only read the arguments and call these functions.
//!
//! * [`arg_str`], [`arg_i64`], [`opt_i64`]: the typed readers of the JSON arguments.
//! * [`AGENT_DEFAULTS`] and [`fill_agent_defaults`]: what an agent gets when it gives none.
//! * [`COMPENSATE_DEFAULT`], [`CLOSE_DEFAULT`], [`PAGE_KEEP_EDITED`]: the defaults of the page
//!   surface (the bridge and the Tauri wrappers). The agent surface differs on `keep_edited` on
//!   purpose: see [`AGENT_DEFAULTS`].
//! * [`check_args`]: the registry row's arguments (no unknown key, required, types).
//! * [`changes_project`] and [`read_only_refusal`]: the read-only guard.
//! * [`transport_args`]: the transport parsing that the bridge and `transport_play` share.

use serde_json::{Map, Value};

use crate::api::{CommandSpec, Ty};
use crate::{AppError, Service};

/// `export_midi` without `compensate`: the latency is compensated (both surfaces).
pub const COMPENSATE_DEFAULT: bool = true;
/// `transport_stop` without `close`: the output stays open (both surfaces).
pub const CLOSE_DEFAULT: bool = false;
/// `analyze_start` on the page surface without `keep_edited`: the page applies the answer itself
/// (the page sends the flag since P4-9; the default is only reached by a script that omits it).
/// `the_page_sends_keep_edited_itself` (`tests/command_tests.rs`) pins that the page sends it.
pub const PAGE_KEEP_EDITED: bool = false;

/// The string argument `key` of `args`.
///
/// # Errors
///
/// A 400 [`AppError`] when `key` is missing or its value is not a JSON string.
pub fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (string)")))
}

/// The integer argument `key` of `args`.
///
/// # Errors
///
/// A 400 [`AppError`] when `key` is missing or its value is not an integer that fits an `i64`.
pub fn arg_i64(args: &Value, key: &str) -> Result<i64, AppError> {
    args.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (integer)")))
}

/// The integer argument `key` of `args`, or `None` when it is missing or `null`.
///
/// # Errors
///
/// A 400 [`AppError`] when `key` is given but its value is not an integer that fits an `i64`.
pub fn opt_i64(args: &Value, key: &str) -> Result<Option<i64>, AppError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| AppError::bad(format!("wrong argument `{key}` (integer)"))),
    }
}

/// The arguments an agent gets when it gives none: `(command, key, value)`. The page keeps the old behaviour
/// (the page sends `keep_edited` itself since P4-9 and no `apply`: it applies the velocities as one undo step).
pub const AGENT_DEFAULTS: [(&str, &str, bool); 3] = [
    ("analyze_start", "keep_edited", true),
    ("track_repick", "keep_edited", true),
    ("track_velocity_set", "apply", true),
];

/// Fill the agent defaults of `name` into `args`; an argument the caller gave wins.
pub fn fill_agent_defaults(name: &str, args: &mut Map<String, Value>) {
    for (cmd, key, v) in AGENT_DEFAULTS {
        if cmd == name {
            args.entry(key).or_insert(Value::Bool(v));
        }
    }
}

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
///
/// # Errors
///
/// A 400 [`AppError`] for an argument key the row does not list, a missing or `null` required argument,
/// or a value of the wrong type.
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
                )));
            }
            Some(v) if !v.is_null() && !type_ok(a.ty, v) => {
                return Err(AppError::bad(format!(
                    "wrong argument `{}`: expected {}",
                    a.name,
                    type_name(a.ty)
                )));
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

/// The read-only guard: the 409 for a command that changes the project `sid` when that project is open read-only.
/// `None` when the call may run.
pub fn read_only_refusal(svc: &Service, row: &CommandSpec, args: &Map<String, Value>) -> Option<AppError> {
    if !changes_project(row, args) {
        return None;
    }
    let sid = args.get("sid").and_then(Value::as_str)?;
    svc.inner
        .registry
        .get(sid)
        .is_ok_and(|op| crate::registry::lock(&op).read_only)
        .then(|| {
            AppError::conflict(format!(
                "project {sid} is open read-only: {} changes it; open it for writing (project_open without read_only)",
                row.name
            ))
        })
}

/// The transport parsing that the bridge (`invoke_transport`) and the Tauri wrapper of `transport_play` share.
pub mod transport_args {
    use serde_json::Value;

    use crate::AppError;
    use crate::transport::PlayWindow;

    /// A number or nothing; any other value is a 400 naming the key.
    ///
    /// # Errors
    ///
    /// A 400 [`AppError`] when `key` is given (not `null`) but its value is not a number.
    pub fn opt_f64(args: &Value, key: &str) -> Result<Option<f64>, AppError> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_f64()
                .map(Some)
                .ok_or_else(|| AppError::bad(format!("wrong argument `{key}` (number)"))),
        }
    }

    /// A boolean or nothing. Lenient on purpose: a value of another type reads as nothing (the bridge's
    /// behaviour since P3-5b; the typed wrappers never see the difference).
    pub fn opt_bool(args: &Value, key: &str) -> Option<bool> {
        args.get(key).and_then(Value::as_bool)
    }

    /// The audition window of `transport_play` (P4-40d: start, end, loop, tracks; P4-40e: shot). Both surfaces
    /// build it with [`PlayWindow::from_args`], so the default (no window) and the checks are the same.
    ///
    /// # Errors
    ///
    /// The 400 [`AppError`] of [`PlayWindow::from_args`] for an invalid window (see there).
    pub fn play_window(
        start: Option<f64>,
        end: Option<f64>,
        looped: Option<bool>,
        tracks: Option<&Value>,
        shot: Option<&Value>,
    ) -> Result<Option<PlayWindow>, AppError> {
        PlayWindow::from_args(start, end, looped, tracks, shot)
    }
}

#[cfg(test)]
mod tests;
