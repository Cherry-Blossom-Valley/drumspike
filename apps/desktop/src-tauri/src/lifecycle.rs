//! The Tauri wrappers of the lifecycle commands (P3-4a). Each only converts its arguments and
//! calls the method of `drumspike_app::Service` with the same name (`crates/app/src/lifecycle.rs`,
//! `lanes.rs`). The argument names are the ones of the test bridge (`lifecycle::dispatch`).
//! P3-15c: the commands that read or write files are async and run on the blocking pool
//! (`crate::blocking`); the rest stay on the main thread (memory work only).

use std::sync::Arc;

use drumspike_app::{AppError, Progress, Service};
use serde_json::Value;
use tauri::State;
use tauri::ipc::Channel;

use crate::blocking;

/// Create a project folder and open it.
///
/// # Errors
///
/// 400 when `audio_mode` is neither `referenced` nor `copied`, or `name` is not a valid folder
/// name. 409 when the project folder already exists, or its lock is held by another process. 500
/// when the folder or the document cannot be written.
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
///
/// # Errors
///
/// 404 when the project is unknown. 409 when the project is open read-only, or (without `force`)
/// the document on disk changed since it was read or written. 500 when the document or its
/// backup cannot be written.
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
///
/// # Errors
///
/// 400 when `name` is missing or not valid. 404 when the project is unknown. 409 when the target
/// folder already holds a project, a copy to that name is running, or the project lock is held by
/// another process. 500 when the new folder or document cannot be written. A failure of the audio
/// copy is reported on `progress`, not here.
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
///
/// # Errors
///
/// 404 when the project is unknown. 409 when the project has unsaved changes and `force` is not
/// set.
#[tauri::command]
pub async fn project_close(svc: State<'_, Service>, sid: String, force: Option<bool>) -> Result<Value, AppError> {
    blocking(&svc, "project_close", move |s| {
        s.project_close(&sid, force.unwrap_or(false))
    })
    .await
}

/// P4-40a: clears the dirty flag when the document equals the saved one (an undo back to the saved state).
///
/// # Errors
///
/// 404 when the project is unknown.
#[tauri::command]
pub fn project_clean_check(svc: State<'_, Service>, sid: String) -> Result<Value, AppError> {
    svc.project_clean_check(&sid)
}

/// Set the display name of the project (the folder keeps its name).
///
/// # Errors
///
/// 400 when `name` is empty or not a valid file name. 404 when the project is unknown. 409 when
/// the project is open read-only.
#[tauri::command]
pub fn project_rename(svc: State<'_, Service>, sid: String, name: String) -> Result<Value, AppError> {
    svc.project_rename(&sid, &name)
}

/// Set the display order of the tracks.
///
/// # Errors
///
/// 400 when `order` is not a list of strings that is a permutation of the track names. 404 when
/// the project is unknown. 409 when the project is open read-only.
#[tauri::command]
pub fn tracks_reorder(svc: State<'_, Service>, sid: String, order: Value) -> Result<Value, AppError> {
    svc.tracks_reorder(&sid, &order)
}

/// Set the MIDI note of the track `name`.
///
/// # Errors
///
/// 400 when the track is unknown or `note` is outside 0 to 127. 404 when the project is unknown.
/// 409 when the project is open read-only.
#[tauri::command]
pub fn track_set_note(svc: State<'_, Service>, sid: String, name: String, note: i64) -> Result<Value, AppError> {
    svc.track_set_note(&sid, &name, note)
}

/// 0.9.1: sets the role of a track; a change re-analyzes it, so on the blocking pool.
///
/// # Errors
///
/// 400 when `role` is not a known role, `source` is not a known source, `note` is outside 0 to
/// 127, or `events` is not a list. 404 when the project or the track is unknown. 409 when the
/// project is open read-only, or an analysis or another write of the project is running. An
/// error of the re-pick also applies; the role fields are put back then.
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
///
/// # Errors
///
/// 404 when the project is unknown. 409 when the project is open read-only, or the follower is
/// offline. 400 when the track would follow itself, the source is unknown or follows a track, or
/// the track has followers. The error of the first derivation of the hits also applies; the
/// previous `follows` is restored then.
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

/// Rename the track `name` and its `Analysis/` folder.
///
/// # Errors
///
/// 400 when the track is unknown or `new_name` is too long or not valid. 404 when the project is
/// unknown. 409 when the project is open read-only, the new name is taken, or an analysis, a lane
/// build or a copy job of the project is running.
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
///
/// # Errors
///
/// The settings read itself cannot fail. The call fails only (500) when the blocking task cannot
/// finish, for example because it panicked.
#[tauri::command]
pub async fn app_settings_get(svc: State<'_, Service>) -> Result<Value, AppError> {
    blocking(&svc, "app_settings_get", |s| Ok(s.app_settings_get())).await
}

/// P3-14a: one key of the app settings: a file write, on the blocking pool.
///
/// # Errors
///
/// 400 when `key` is not 1 to 32 characters of `[a-z_]`, or is an audio key (`audio_output`,
/// `master_db`). 500 when `settings.json` cannot be written.
#[tauri::command]
pub async fn app_settings_set(svc: State<'_, Service>, key: String, value: Value) -> Result<Value, AppError> {
    blocking(&svc, "app_settings_set", move |s| s.app_settings_set(&key, &value)).await
}

/// Reads `recent.json` and looks at every folder of it: on the blocking pool.
///
/// # Errors
///
/// The listing itself cannot fail (a missing or damaged file is an empty list). The call fails
/// only (500) when the blocking task cannot finish, for example because it panicked.
#[tauri::command]
pub async fn project_recent(svc: State<'_, Service>) -> Result<Value, AppError> {
    blocking(&svc, "project_recent", |s| Ok(s.project_recent())).await
}

/// #441 Clear Recent: empties the stored recent list (`recent.json`): on the blocking pool.
///
/// # Errors
///
/// The clearing itself cannot fail. The call fails only (500) when the blocking task cannot
/// finish, for example because it panicked.
#[tauri::command]
pub async fn project_recent_clear(svc: State<'_, Service>) -> Result<Value, AppError> {
    blocking(&svc, "project_recent_clear", |s| Ok(s.project_recent_clear())).await
}

/// Starts the lane build and returns at once; the channel `progress` gets `{sid, job}` after every
/// change of the job (like `analyze_start`).
///
/// # Errors
///
/// 404 when the project is unknown or a name in `tracks` is not a track. 409 when the project is
/// open read-only. Nothing is queued then.
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
