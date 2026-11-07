//! P4-5 step 3: the one dispatcher of the commands (the match that `bridge.rs` held). The test bridge, the
//! command-line tool and the MCP server all call it; the Tauri wrappers call the same `Service` methods.
//!
//! * [`run`] runs one command by name with JSON arguments (the names and argument names are those of the Tauri
//!   wrappers); `None` for a name it does not know. R3: one name-to-handler map built from the modules'
//!   `HANDLERS` slices (the transport's too); a registry row has one handler here or, for the rows only the
//!   desktop shell answers, none (`every_registry_row_has_exactly_one_handler`).
//! * [`call`] is the agent entry: it checks the name against the registry (`api`), the arguments against the row
//!   (required, types, no unknown key), fills the agent defaults (`keep_edited` is true), refuses a command that
//!   changes the project when the project is open read-only, and gives a progress callback to the commands that
//!   take a `progress` channel.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use serde_json::{Map, Value};

use crate::api::{self, Surface};
use crate::command_policy as policy;
use crate::reply::{Channels, Reply};
use crate::{AppError, Progress, Service};

/// The handler of one command: the service, the progress channels, the command name and the JSON arguments.
pub(crate) type Handler = fn(&Service, &Arc<Channels>, &str, &Value) -> Result<Reply, AppError>;

/// R3: one row of a module's `HANDLERS`: a command name and the function that runs it.
#[derive(Clone, Copy)]
pub(crate) struct Route {
    pub(crate) name: &'static str,
    pub(crate) run: Handler,
}

/// The routes of `names`, all to `run` (a module whose dispatcher matches on the name).
#[must_use]
pub(crate) const fn routes<const N: usize>(names: [&'static str; N], run: Handler) -> [Route; N] {
    let mut out = [Route { name: "", run }; N];
    let mut i = 0;
    while i < N {
        out[i].name = names[i];
        i += 1;
    }
    out
}

/// The reply of a module dispatcher that answers JSON and `None` for a name it does not run (which a name of
/// its `HANDLERS` never is).
///
/// # Errors
///
/// The error of the command; a 404 when the dispatcher does not run `cmd`.
pub(crate) fn json_reply(cmd: &str, r: Option<Result<Value, AppError>>) -> Result<Reply, AppError> {
    r.unwrap_or_else(|| Err(unknown(cmd))).map(Reply::Json)
}

fn unknown(cmd: &str) -> AppError {
    AppError::not_found(format!("unknown command {cmd}"))
}

/// The handlers of every module; a name is in exactly one of them.
const HANDLERS: [&[Route]; 22] = [
    &crate::commands::HANDLERS,
    &crate::transport::HANDLERS,
    &crate::shell::HANDLERS,
    &crate::reanalyze::HANDLERS,
    &crate::feedback::HANDLERS,
    &crate::lifecycle::HANDLERS,
    &crate::import::HANDLERS,
    &crate::plugins::HANDLERS,
    &crate::tracks::HANDLERS,
    &crate::copyjob::HANDLERS,
    &crate::relink::HANDLERS,
    &crate::histogram::HANDLERS,
    &crate::edit::HANDLERS,
    &crate::learn::HANDLERS,
    &crate::tempo::HANDLERS,
    &crate::markers::HANDLERS,
    &crate::rowheights::HANDLERS,
    &crate::confidence::HANDLERS,
    &crate::layer::HANDLERS,
    &crate::sampler::HANDLERS,
    &crate::training::HANDLERS,
    &crate::agent_skill::HANDLERS,
];

/// Every route of [`run`], in the order of the module list.
pub(crate) fn all_routes() -> impl Iterator<Item = &'static Route> {
    HANDLERS.iter().flat_map(|h| h.iter())
}

/// The name-to-handler map of [`run`]. A name has one route (`every_registry_row_has_exactly_one_handler`); the
/// first one would win.
fn table() -> &'static HashMap<&'static str, Handler> {
    static TABLE: OnceLock<HashMap<&'static str, Handler>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut m = HashMap::new();
        for r in all_routes() {
            m.entry(r.name).or_insert(r.run);
        }
        m
    })
}

/// Run one command with JSON arguments; `None` when `cmd` is not one this function runs.
pub fn run(svc: &Service, channels: &Arc<Channels>, cmd: &str, args: &Value) -> Option<Result<Reply, AppError>> {
    let handler = table().get(cmd)?;
    Some(handler(svc, channels, cmd, args))
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
        Service::new(crate::ServiceConfig::default())
    }

    #[test]
    fn call_refuses_what_is_not_on_the_agent_surface_and_bad_arguments() {
        let s = svc();
        assert_eq!(call(&s, "transport_play", &json!({}), None).unwrap_err().code, 404);
        // a window command that `run` answers stays off the agent surface
        assert!(run(&s, &Arc::new(Channels::default()), "transport_status", &json!({})).is_some());
        assert_eq!(call(&s, "transport_status", &json!({}), None).unwrap_err().code, 404);
        assert_eq!(call(&s, "peaks_tile", &json!({}), None).unwrap_err().code, 404);
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

    /// The registry rows that `run` does not answer: the desktop shell runs them (window and self-test commands;
    /// the test bridge answers `selftest_menu` itself). `tests/api.rs` pins their Tauri commands.
    const SHELL_ONLY: [&str; 25] = [
        "about_info",
        "about_third_party",
        "open_link",
        "open_url",
        "startup_mark",
        "shell_sync",
        "shell_typing",
        "open_paths_take",
        "quit_app",
        "app_ready",
        "splash_painted",
        "splash_dismiss",
        "reveal_path",
        "selftest_clock",
        "selftest_memory",
        "selftest_report",
        "splash_report",
        "selftest_menu",
        "selftest_menu_report",
        "selftest_ns_menu",
        "selftest_menu_key",
        "selftest_menu_log",
        "selftest_menu_probe",
        "selftest_open_paths",
        "selftest_close_window",
    ];

    /// The routes without a registry row: the #387 plug-in commands. Only the test bridge runs them (no Tauri
    /// command, not on the agent surface); a registry row for them is a behaviour change, not part of R3.
    const UNREGISTERED: [&str; 2] = ["plugins_list", "plugin_accept"];

    /// R3: every registry row has exactly one handler: one route of `run` (no name twice, no route without a
    /// row), or the desktop shell for the rows of [`SHELL_ONLY`]. Every agent row has a route.
    #[test]
    fn every_registry_row_has_exactly_one_handler() {
        let mut routes: HashMap<&str, usize> = HashMap::new();
        for r in all_routes() {
            *routes.entry(r.name).or_default() += 1;
        }
        let twice: Vec<&str> = routes.iter().filter(|(_, n)| **n > 1).map(|(k, _)| *k).collect();
        assert!(twice.is_empty(), "two handlers: {twice:?}");
        for name in routes.keys() {
            let row = api::find(name).is_some();
            assert!(row != UNREGISTERED.contains(name), "{name}: registry row {row}");
        }
        for c in api::REGISTRY {
            let (routed, shell) = (routes.contains_key(c.name), SHELL_ONLY.contains(&c.name));
            assert!(routed != shell, "{}: routed {routed}, shell only {shell}", c.name);
            assert!(
                routed || c.surface != Surface::Agent,
                "{}: an agent row without a route",
                c.name
            );
        }
        assert_eq!(
            routes.len() + SHELL_ONLY.len(),
            api::REGISTRY.len() + UNREGISTERED.len()
        );
        assert_eq!(table().len(), routes.len());
    }
}
