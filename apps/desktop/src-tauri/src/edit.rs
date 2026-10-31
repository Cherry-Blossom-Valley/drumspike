//! P4-5: the Tauri wrappers of the hit and timing edits (`crates/app/src/edit.rs`). Each only
//! converts its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

use crate::blocking;

/// Apply the hit edits `ops` to the track `name` of the project `sid`.
///
/// # Errors
///
/// 400 for an empty `ops` or an op that does not validate (nothing is changed then). 404 when the
/// project or the track is unknown. 409 when the project is open read-only, when the track follows
/// another track, or when another write of the project is in progress (retry).
/// 500 when a snap cannot decode the source audio.
#[tauri::command]
pub async fn events_edit(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    ops: Vec<Value>,
) -> Result<Value, AppError> {
    blocking(&svc, "events_edit", move |s| s.events_edit(&sid, &name, &ops)).await
}

/// Set the Hitpoint shift of the track `name` and move its hits by the difference.
///
/// # Errors
///
/// 400 when `shift_samples` is outside the shift limit or the track has no sample rate. 404 when
/// the project or the track is unknown. 409 when the project is open read-only, the track follows
/// another track, or an analysis or a re-pick store is running.
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
