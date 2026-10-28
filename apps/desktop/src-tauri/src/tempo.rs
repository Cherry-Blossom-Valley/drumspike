//! P4-8: the Tauri wrappers of the tempo and meter map (`crates/app/src/tempo.rs`). Each only
//! converts its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

use crate::blocking;

#[tauri::command]
pub fn tempo_map_get(svc: State<'_, Service>, sid: String) -> Result<Value, AppError> {
    svc.tempo_map_get(&sid)
}

#[tauri::command]
pub fn tempo_map_set(svc: State<'_, Service>, sid: String, map: Value) -> Result<Value, AppError> {
    svc.tempo_map_set(&sid, &map)
}

/// Reads a file: on the blocking pool.
#[tauri::command]
pub async fn tempo_map_import_midi(svc: State<'_, Service>, sid: String, path: String) -> Result<Value, AppError> {
    blocking(&svc, "tempo_map_import_midi", move |s| {
        s.tempo_map_import_midi(&sid, &path)
    })
    .await
}

/// P4-8 stage B: a suggested map from the kick and snare hits (read only).
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
#[tauri::command]
pub fn tempo_map_refit_bar1(svc: State<'_, Service>, sid: String, bpm: f64) -> Result<Value, AppError> {
    svc.tempo_map_refit_bar1(&sid, bpm)
}
