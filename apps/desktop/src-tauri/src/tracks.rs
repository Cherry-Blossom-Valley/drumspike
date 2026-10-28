//! The Tauri wrappers of the track import and the kit presets (P3-4c). Each only converts its
//! arguments and calls the method of `drumspike_app::Service` with the same name
//! (`crates/app/src/tracks.rs`, `roles.rs`). The argument names are the ones of the test bridge
//! (`tracks::dispatch`). `tracks_add` runs on the blocking pool (P3-15c, `crate::blocking`; a copy of GB of audio takes seconds);
//! its channel gets `{sid, add: {state, done, total, file}}`.

use std::sync::Arc;

use drumspike_app::{AppError, Progress, Service};
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::State;

use crate::blocking;

#[tauri::command]
pub async fn tracks_scan(
    svc: State<'_, Service>,
    paths: Vec<String>,
    preset: Option<String>,
) -> Result<Value, AppError> {
    blocking(&svc, "tracks_scan", move |s| s.tracks_scan(&paths, preset.as_deref())).await
}

#[tauri::command]
pub async fn tracks_add(
    svc: State<'_, Service>,
    sid: String,
    paths: Vec<String>,
    roles: Option<Vec<Value>>,
    notes: Option<Vec<Value>>,
    copy: Option<bool>,
    progress: Channel<Value>,
) -> Result<Value, AppError> {
    let progress: Progress = Arc::new(move |v: Value| {
        let _ = progress.send(v);
    });
    blocking(&svc, "tracks_add", move |s| {
        s.tracks_add(
            &sid,
            &paths,
            &roles.unwrap_or_default(),
            &notes.unwrap_or_default(),
            copy.unwrap_or(false),
            Some(progress),
        )
    })
    .await
}

/// P4-39a: Track > Remove Track. `snapshot` answers the removed tracks as JSON for the page's undo.
#[tauri::command]
pub async fn tracks_remove(
    svc: State<'_, Service>,
    sid: String,
    names: Vec<String>,
    snapshot: Option<bool>,
) -> Result<Value, AppError> {
    blocking(&svc, "tracks_remove", move |s| {
        s.tracks_remove(&sid, &names, snapshot.unwrap_or(false))
    })
    .await
}

/// P4-39a: the undo of `tracks_remove` (builds the peak caches again, so it runs on the blocking pool).
#[tauri::command]
pub async fn tracks_restore(svc: State<'_, Service>, sid: String, tracks: Value) -> Result<Value, AppError> {
    blocking(&svc, "tracks_restore", move |s| s.tracks_restore(&sid, &tracks)).await
}

#[tauri::command]
pub async fn kit_presets_list(svc: State<'_, Service>) -> Result<Value, AppError> {
    blocking(&svc, "kit_presets_list", |s| s.kit_presets_list()).await
}

#[tauri::command]
pub async fn kit_preset_save(svc: State<'_, Service>, name: String, rules: Value) -> Result<Value, AppError> {
    blocking(&svc, "kit_preset_save", move |s| s.kit_preset_save(&name, &rules)).await
}

#[tauri::command]
pub async fn kit_preset_load(svc: State<'_, Service>, name: String) -> Result<Value, AppError> {
    blocking(&svc, "kit_preset_load", move |s| s.kit_preset_load(&name)).await
}
