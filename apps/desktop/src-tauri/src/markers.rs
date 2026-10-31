//! P4-40g: the Tauri wrappers of the review markers (`crates/app/src/markers.rs`). Each only
//! converts its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

/// The review markers of the project.
///
/// # Errors
///
/// 404 when the project is unknown.
#[tauri::command]
pub fn markers_get(svc: State<'_, Service>, sid: String) -> Result<Value, AppError> {
    svc.markers_get(&sid)
}

/// Replace the whole list of review markers (an empty list clears them).
///
/// # Errors
///
/// 400 when an entry is not an object with a numeric `t` and an optional string `note`. 404 when
/// the project is unknown. 409 when the project is open read-only.
#[tauri::command]
pub fn markers_set(svc: State<'_, Service>, sid: String, markers: Vec<Value>) -> Result<Value, AppError> {
    svc.markers_set(&sid, &markers)
}
