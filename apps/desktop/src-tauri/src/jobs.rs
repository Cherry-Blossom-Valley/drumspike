//! The Tauri wrappers of the copy jobs and the relink flow (P3-4d). Each only converts its
//! arguments and calls the method of `drumspike_app::Service` with the same name
//! (`crates/app/src/copyjob.rs`, `relink.rs`). The argument names are the ones of the test bridge.
//! `project_consolidate` starts its job and returns at once; its channel gets `{sid, copy: ...}`
//! (the same messages as the Save As copy of `lifecycle::project_save_as`). `project_relink` and
//! `relink_search` hash file heads, so they run on the blocking pool (P3-15c, `crate::blocking`).

use std::sync::Arc;

use drumspike_app::{AppError, Progress, Service};
use serde_json::Value;
use tauri::State;
use tauri::ipc::Channel;

use crate::blocking;

/// Start the job that copies the referenced audio of the project into its folder. It returns at
/// once; the progress goes to `progress`.
///
/// # Errors
///
/// 404 when the project is unknown. 409 when the project is open read-only, or a copy job of the
/// project is already running. The copy itself runs on a thread, and its failure is reported as
/// a message on `progress`, not here.
#[tauri::command]
pub async fn project_consolidate(
    svc: State<'_, Service>,
    sid: String,
    progress: Channel<Value>,
) -> Result<Value, AppError> {
    let progress: Progress = Arc::new(move |v: Value| {
        let _ = progress.send(v);
    });
    blocking(&svc, "project_consolidate", move |s| {
        s.project_consolidate(&sid, Some(progress))
    })
    .await
}

#[tauri::command]
pub fn job_status(svc: State<'_, Service>, sid: String) -> Value {
    svc.job_status(&sid)
}

#[tauri::command]
pub fn job_cancel(svc: State<'_, Service>, sid: String) -> Value {
    svc.job_cancel(&sid)
}

/// Point the track `track` at the audio file `path`.
///
/// # Errors
///
/// 404 when the project or the track is unknown, or `path` is not a file. 409 when the project is
/// open read-only, or the file does not match the track's source (size or head hash) and `force`
/// is not set. 400 when a forced file cannot be read as audio. Other errors come from recording
/// the new source.
#[tauri::command]
pub async fn project_relink(
    svc: State<'_, Service>,
    sid: String,
    track: String,
    path: String,
    force: Option<bool>,
) -> Result<Value, AppError> {
    blocking(&svc, "project_relink", move |s| {
        s.project_relink(&sid, &track, &path, force.unwrap_or(false))
    })
    .await
}

/// Search `folder` for the files of the offline tracks (and offline layer files) of the project.
///
/// # Errors
///
/// 404 when the project is unknown or `folder` is not a directory. 409 when the project is open
/// read-only.
#[tauri::command]
pub async fn relink_search(svc: State<'_, Service>, sid: String, folder: String) -> Result<Value, AppError> {
    blocking(&svc, "relink_search", move |s| s.relink_search(&sid, &folder)).await
}
