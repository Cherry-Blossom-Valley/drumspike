//! P4-5: the Tauri wrappers of the hit and timing edits (`crates/app/src/edit.rs`). Each only
//! converts its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

use crate::blocking;

#[tauri::command]
pub async fn events_edit(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    ops: Vec<Value>,
) -> Result<Value, AppError> {
    blocking(&svc, "events_edit", move |s| s.events_edit(&sid, &name, &ops)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn track_shift_set(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    shift_samples: i64,
) -> Result<Value, AppError> {
    blocking(&svc, "track_shift_set", move |s| {
        s.track_shift_set(&sid, &name, shift_samples)
    })
    .await
}
