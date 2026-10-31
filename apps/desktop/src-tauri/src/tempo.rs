//! P4-8: the Tauri wrappers of the tempo and meter map (`crates/app/src/tempo.rs`). Each only
//! converts its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

use crate::blocking;

/// The tempo and meter map of the project, and whether it is the default.
///
/// # Errors
///
/// 404 when the project is unknown. 400 when the stored tempo map is not valid.
#[tauri::command]
pub fn tempo_map_get(svc: State<'_, Service>, sid: String) -> Result<Value, AppError> {
    svc.tempo_map_get(&sid)
}

/// Replace the whole tempo and meter map (null resets it to the default).
///
/// # Errors
///
/// 400 when `map` is not a valid tempo map. 404 when the project is unknown. 409 when the project
/// is open read-only.
#[tauri::command]
pub fn tempo_map_set(svc: State<'_, Service>, sid: String, map: Value) -> Result<Value, AppError> {
    svc.tempo_map_set(&sid, &map)
}

/// Reads a file: on the blocking pool.
///
/// # Errors
///
/// 404 when the project is unknown or `path` cannot be read from the file system. 409 when the
/// project is open read-only. 400 when `path` is not a MIDI file within the size limit, has no
/// tempo or time-signature event, or holds a tempo outside the valid range. 500 when the file
/// cannot be read.
#[tauri::command]
pub async fn tempo_map_import_midi(svc: State<'_, Service>, sid: String, path: String) -> Result<Value, AppError> {
    blocking(&svc, "tempo_map_import_midi", move |s| {
        s.tempo_map_import_midi(&sid, &path)
    })
    .await
}

/// P4-8 stage B: a suggested map from the kick and snare hits (read only).
///
/// # Errors
///
/// 400 when `min_bars` or `min_step_pct` is outside the valid range, when the stored tempo map is
/// not valid, or when there are too few hits to detect a map. 404 when the project is unknown or
/// a name in `tracks` is not a track.
#[tauri::command(rename_all = "snake_case")]
pub fn tempo_map_detect(
    svc: State<'_, Service>,
    sid: String,
    tracks: Option<Vec<String>>,
    min_bars: Option<i64>,
    min_step_pct: Option<f64>,
) -> Result<Value, AppError> {
    svc.tempo_map_detect(&sid, tracks.as_deref(), min_bars, min_step_pct)
}

/// #468: the beat phase of `bpm` that the kick and snare hits support, as the new bar 1 (read only).
///
/// # Errors
///
/// 400 when `bpm` is not a positive number of at most 1000, when the stored tempo map is not
/// valid, or when there are too few kick and snare hits. 404 when the project is unknown.
#[tauri::command]
pub fn tempo_map_refit_bar1(svc: State<'_, Service>, sid: String, bpm: f64) -> Result<Value, AppError> {
    svc.tempo_map_refit_bar1(&sid, bpm)
}
