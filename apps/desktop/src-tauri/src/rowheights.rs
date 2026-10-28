//! #167: the Tauri wrappers of the row heights (`crates/app/src/rowheights.rs`). Each only
//! converts its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

#[tauri::command]
pub fn row_heights_get(svc: State<'_, Service>, sid: String) -> Result<Value, AppError> {
    svc.row_heights_get(&sid)
}

#[tauri::command]
pub fn row_heights_set(svc: State<'_, Service>, sid: String, heights: Value) -> Result<Value, AppError> {
    svc.row_heights_set(&sid, &heights)
}
