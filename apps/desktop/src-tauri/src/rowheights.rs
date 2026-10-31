//! #167: the Tauri wrappers of the row heights (`crates/app/src/rowheights.rs`). Each only
//! converts its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

/// The per-track row height factors of the project.
///
/// # Errors
///
/// 404 when the project is unknown.
#[tauri::command]
pub fn row_heights_get(svc: State<'_, Service>, sid: String) -> Result<Value, AppError> {
    svc.row_heights_get(&sid)
}

/// Replace the whole map of row height factors (an empty map clears the overrides).
///
/// # Errors
///
/// 400 when `heights` is not an object, or an entry is not a positive finite number. 404 when the
/// project is unknown. 409 when the project is open read-only.
#[tauri::command]
pub fn row_heights_set(svc: State<'_, Service>, sid: String, heights: Value) -> Result<Value, AppError> {
    svc.row_heights_set(&sid, &heights)
}
