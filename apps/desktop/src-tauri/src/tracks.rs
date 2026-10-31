//! The Tauri wrappers of the track import and the kit presets (P3-4c). Each only converts its
//! arguments and calls the method of `drumspike_app::Service` with the same name
//! (`crates/app/src/tracks.rs`, `roles.rs`). The argument names are the ones of the test bridge
//! (`tracks::dispatch`). `tracks_add` runs on the blocking pool (P3-15c, `crate::blocking`; a copy of GB of audio takes seconds);
//! its channel gets `{sid, add: {state, done, total, file}}`.

use std::sync::Arc;

use drumspike_app::{AppError, Progress, Service};
use serde_json::Value;
use tauri::State;
use tauri::ipc::Channel;

use crate::blocking;

/// List the audio files that `paths` stand for, each with a guessed role and note.
///
/// # Errors
///
/// Only when `preset` names a kit preset that cannot be loaded: 404 for an unknown preset, 400
/// for a preset file that is not valid, 409 when the app has no data folder. A file that cannot
/// be read is listed as rejected, not an error.
#[tauri::command]
pub async fn tracks_scan(
    svc: State<'_, Service>,
    paths: Vec<String>,
    preset: Option<String>,
) -> Result<Value, AppError> {
    blocking(&svc, "tracks_scan", move |s| s.tracks_scan(&paths, preset.as_deref())).await
}

/// Add the audio files `paths` to the project as tracks; the progress goes to `progress`.
///
/// # Errors
///
/// 404 when the project is unknown. 409 when the project is open read-only, or the copy job
/// cannot start. An error also comes from adding an accepted file as a track (for example a bad
/// track name). A file that cannot be probed or copied is reported as rejected, not as an error.
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
///
/// # Errors
///
/// 400 when `names` is empty. 404 when the project is unknown or a name is not a track (nothing
/// is removed then). 409 when the project is open read-only, or an analysis, a lane build or a
/// copy job of the project is running. 500 when the analysis folder of a track cannot be deleted
/// or a track record cannot be serialized.
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
///
/// # Errors
///
/// 400 when `tracks` is not a list, a row has no valid `name`, or a track record cannot be
/// parsed. 404 when the project is unknown. 409 when the project is open read-only, or a track
/// of that name exists.
#[tauri::command]
pub async fn tracks_restore(svc: State<'_, Service>, sid: String, tracks: Value) -> Result<Value, AppError> {
    blocking(&svc, "tracks_restore", move |s| s.tracks_restore(&sid, &tracks)).await
}

/// The kit presets stored in the app data folder, sorted by name.
///
/// # Errors
///
/// 409 when the app has no data folder. A file that is not a preset is skipped.
#[tauri::command]
pub async fn kit_presets_list(svc: State<'_, Service>) -> Result<Value, AppError> {
    blocking(&svc, "kit_presets_list", |s| s.kit_presets_list()).await
}

/// Write the kit preset `name` with `rules` (a preset of the same name is replaced).
///
/// # Errors
///
/// 400 when `name` is not usable as a file name, `rules` is not a list, or a rule is not valid.
/// 409 when the app has no data folder. 500 when the folder or the file cannot be written.
#[tauri::command]
pub async fn kit_preset_save(svc: State<'_, Service>, name: String, rules: Value) -> Result<Value, AppError> {
    blocking(&svc, "kit_preset_save", move |s| s.kit_preset_save(&name, &rules)).await
}

/// The rules of the kit preset `name`.
///
/// # Errors
///
/// 404 when there is no preset of that name. 400 when `name` is not usable as a file name or the
/// preset file is not valid. 409 when the app has no data folder.
#[tauri::command]
pub async fn kit_preset_load(svc: State<'_, Service>, name: String) -> Result<Value, AppError> {
    blocking(&svc, "kit_preset_load", move |s| s.kit_preset_load(&name)).await
}
