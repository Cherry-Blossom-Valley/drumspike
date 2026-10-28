//! P4-40g: the Tauri wrappers of the review markers (`crates/app/src/markers.rs`). Each only
//! converts its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

#[tauri::command]
pub fn markers_get(svc: State<'_, Service>, sid: String) -> Result<Value, AppError> {
    svc.markers_get(&sid)
}

#[tauri::command]
pub fn markers_set(svc: State<'_, Service>, sid: String, markers: Vec<Value>) -> Result<Value, AppError> {
    svc.markers_set(&sid, &markers)
}
