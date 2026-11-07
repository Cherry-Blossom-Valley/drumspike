//! #371: the Tauri wrappers of the agent fine-tune skill (`crates/app/src/agent_skill.rs`). Each only converts its
//! arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

/// Install the bundled skill folder for an agent; an older copy is replaced.
///
/// # Errors
///
/// 404 when the app carries no skill folder. 409 when `target` exists and is not a copy of the skill. 500 when a
/// file cannot be written or there is no home folder and no `target`.
#[tauri::command]
pub fn agent_skill_install(svc: State<'_, Service>, target: Option<String>) -> Result<Value, AppError> {
    svc.agent_skill_install(target.as_deref())
}

/// Whether the skill is installed, with its version and the one line that adds the MCP server.
///
/// # Errors
///
/// 500 when there is no home folder and no `target`.
#[tauri::command]
pub fn agent_skill_status(svc: State<'_, Service>, target: Option<String>) -> Result<Value, AppError> {
    svc.agent_skill_status(target.as_deref())
}
