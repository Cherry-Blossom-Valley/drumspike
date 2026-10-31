//! The Tauri wrappers of the importer of old sessions (P3-4e; the zip import and export went in P4-30). Each only converts its
//! arguments and calls the method of `drumspike_app::Service` with the same name
//! (`crates/app/src/import.rs`). The argument names are the ones of the test bridge
//! (`import::dispatch`). They run on the blocking pool (P3-15c, `crate::blocking`): copying the audio takes seconds.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

use crate::blocking;

/// List the sessions of the old server in `folder` (the folder itself, or its sub-folders).
///
/// # Errors
///
/// 404 when `folder` is empty or not a directory.
#[tauri::command]
pub async fn legacy_list(svc: State<'_, Service>, folder: String) -> Result<Value, AppError> {
    blocking(&svc, "legacy_list", move |s| s.legacy_list(&folder)).await
}

/// Open the session folder `session_dir` of the old server as a new project with copied audio.
///
/// # Errors
///
/// 400 for an empty `session_dir`. 404 when it is not a directory. Other errors come from reading
/// the old session, copying its audio or creating the new project folder; a failed import
/// removes the new folder.
#[tauri::command(rename_all = "snake_case")]
pub async fn project_import(svc: State<'_, Service>, session_dir: String) -> Result<Value, AppError> {
    blocking(&svc, "project_import", move |s| s.project_import(&session_dir)).await
}
