//! The Tauri wrappers of the lifecycle commands (P3-4a). Each only converts its arguments and
//! calls the method of `drumspike_app::Service` with the same name (`crates/app/src/lifecycle.rs`,
//! `lanes.rs`). The argument names are the ones of the test bridge (`lifecycle::dispatch`).
//! P3-15c: the commands that read or write files are async and run on the blocking pool
//! (`crate::blocking`); the rest stay on the main thread (memory work only).

use std::sync::Arc;

use drumspike_app::{AppError, Progress, Service};
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::State;

use crate::blocking;

#[tauri::command(rename_all = "snake_case")]
pub async fn project_new(
    svc: State<'_, Service>,
    name: Option<String>,
    dir: Option<String>,
    audio_mode: Option<String>,
) -> Result<Value, AppError> {
    blocking(&svc, "project_new", move |s| {
        s.project_new(name.as_deref(), dir.as_deref(), audio_mode.as_deref())
    })
    .await
}

/// `force` (#121, Save anyway): write also when the document changed on disk since it was read.
#[tauri::command]
pub async fn project_save(svc: State<'_, Service>, sid: String, force: Option<bool>) -> Result<Value, AppError> {
    blocking(&svc, "project_save", move |s| {
        s.project_save_with(&sid, force.unwrap_or(false))
    })
    .await
}

/// With `copy_audio` the copy is a job (P3-4d): the call answers `{job}` at once and the channel
/// `progress` gets `{sid, copy: ...}` until the state is `done` (with the summary as `result`),
/// `cancelled` or `error`. The channel is required by Tauri; without `copy_audio` it stays unused.
#[tauri::command(rename_all = "snake_case")]
pub async fn project_save_as(
    svc: State<'_, Service>,
    sid: String,
    name: Option<String>,
    dir: Option<String>,
    copy_audio: Option<bool>,
    progress: Channel<Value>,
) -> Result<Value, AppError> {
    let progress: Progress = Arc::new(move |v: Value| {
        let _ = progress.send(v);
    });
    blocking(&svc, "project_save_as", move |s| {
        s.project_save_as(
            &sid,
            name.as_deref(),
            dir.as_deref(),
            copy_audio.unwrap_or(false),
            Some(progress),
        )
    })
    .await
}

/// On the blocking pool: the close releases a transport that plays the project (its stream and files).
#[tauri::command]
pub async fn project_close(svc: State<'_, Service>, sid: String, force: Option<bool>) -> Result<Value, AppError> {
    blocking(&svc, "project_close", move |s| {
        s.project_close(&sid, force.unwrap_or(false))
    })
    .await
}

/// P4-40a: clears the dirty flag when the document equals the saved one (an undo back to the saved state).
#[tauri::command]
pub fn project_clean_check(svc: State<'_, Service>, sid: String) -> Result<Value, AppError> {
    svc.project_clean_check(&sid)
}

#[tauri::command]
pub fn project_rename(svc: State<'_, Service>, sid: String, name: String) -> Result<Value, AppError> {
    svc.project_rename(&sid, &name)
}

#[tauri::command]
pub fn tracks_reorder(svc: State<'_, Service>, sid: String, order: Value) -> Result<Value, AppError> {
    svc.tracks_reorder(&sid, &order)
}

#[tauri::command]
pub fn track_set_note(svc: State<'_, Service>, sid: String, name: String, note: i64) -> Result<Value, AppError> {
    svc.track_set_note(&sid, &name, note)
}

/// 0.9.1: sets the role of a track; a change re-analyzes it, so on the blocking pool.
// The 8 args mirror the track_set_role API command (sid, name, role, note, source,
// role_match, events); a params struct would change the JS invoke contract of #227.
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn track_set_role(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    role: String,
    note: Option<i64>,
    source: Option<String>,
    role_match: Option<String>,
    events: Option<Value>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_set_role", move |s| {
        s.track_set_role(
            &sid,
            &name,
            &role,
            note,
            source.as_deref(),
            role_match.as_deref(),
            events.as_ref(),
        )
    })
    .await
}

/// P3-14a: renames the track and its `Analysis/` folder (file work): on the blocking pool.
/// P3-18: the track takes the hits of `source` (another track), or its own detection with `null`.
#[tauri::command]
pub async fn track_set_follow(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    source: Option<String>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_set_follow", move |s| {
        s.track_set_follow(&sid, &name, source.as_deref())
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn track_rename(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    new_name: String,
) -> Result<Value, AppError> {
    blocking(&svc, "track_rename", move |s| s.track_rename(&sid, &name, &new_name)).await
}

/// P3-14a: the app settings (`<home>/settings.json`): a file read, on the blocking pool.
#[tauri::command]
pub async fn app_settings_get(svc: State<'_, Service>) -> Result<Value, AppError> {
    blocking(&svc, "app_settings_get", |s| Ok(s.app_settings_get())).await
}

/// P3-14a: one key of the app settings: a file write, on the blocking pool.
#[tauri::command]
pub async fn app_settings_set(svc: State<'_, Service>, key: String, value: Value) -> Result<Value, AppError> {
    blocking(&svc, "app_settings_set", move |s| s.app_settings_set(&key, &value)).await
}

/// Reads `recent.json` and looks at every folder of it: on the blocking pool.
#[tauri::command]
pub async fn project_recent(svc: State<'_, Service>) -> Result<Value, AppError> {
    blocking(&svc, "project_recent", |s| Ok(s.project_recent())).await
}

/// #441 Clear Recent: empties the stored recent list (`recent.json`): on the blocking pool.
#[tauri::command]
pub async fn project_recent_clear(svc: State<'_, Service>) -> Result<Value, AppError> {
    blocking(&svc, "project_recent_clear", |s| Ok(s.project_recent_clear())).await
}

/// Starts the lane build and returns at once; the channel `progress` gets `{sid, job}` after every
/// change of the job (like `analyze_start`).
#[tauri::command]
pub async fn peaks_build_start(
    svc: State<'_, Service>,
    sid: String,
    tracks: Option<Vec<String>>,
    progress: Channel<Value>,
) -> Result<Value, AppError> {
    let progress: Progress = Arc::new(move |v: Value| {
        let _ = progress.send(v);
    });
    blocking(&svc, "peaks_build_start", move |s| {
        s.peaks_build_start(&sid, tracks, Some(progress))
    })
    .await
}

#[tauri::command]
pub fn peaks_build_status(svc: State<'_, Service>, sid: String) -> Value {
    svc.peaks_build_status(&sid)
}
