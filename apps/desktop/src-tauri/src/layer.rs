//! P4-7a and P4-7b: the Tauri wrappers of the one-shot layer (`crates/app/src/layer.rs`). Each only
//! converts its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::{Value, json};
use tauri::State;

use crate::blocking;

/// Reads the one-shot file: on the blocking pool.
///
/// # Errors
///
/// 404 when the project or the track is unknown. 409 when the project is open read-only, or a
/// setting (not a `path`) is changed on a track without a layer. 400 for a `path` that is not a
/// text or null, a one-shot file the app cannot read, or a value of another setting that is
/// wrong.
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn track_layer_set(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    path: Option<Value>,
    level_db: Option<f64>,
    polarity: Option<String>,
    on: Option<bool>,
    curve: Option<Value>,
    align: Option<bool>,
    onset: Option<Value>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_layer_set", move |s| {
        let mut args = json!({"level_db": level_db, "polarity": polarity, "on": on, "curve": curve, "align": align});
        // a missing `path` keeps the file; `null` removes the layer
        if let Some(p) = path {
            args["path"] = p;
        }
        // P4-40e: a missing `onset` changes nothing; `null` resets to the measured onset
        if let Some(o) = onset {
            args["onset"] = o;
        }
        s.track_layer_set(&sid, &name, &args)
    })
    .await
}

/// The layer of the track `name` (`layer` is null for a track without one).
///
/// # Errors
///
/// 404 when the project or the track is unknown.
#[tauri::command]
pub fn track_layer_get(svc: State<'_, Service>, sid: String, name: String) -> Result<Value, AppError> {
    svc.track_layer_get(&sid, &name)
}

/// Reads the close mic: on the blocking pool.
///
/// # Errors
///
/// 404 when the project or the track is unknown. 409 when the track has no layer, its layer file
/// is missing, or the track is offline. An error also comes from a source or one-shot that cannot
/// be read.
#[tauri::command]
pub async fn track_layer_align(svc: State<'_, Service>, sid: String, name: String) -> Result<Value, AppError> {
    blocking(&svc, "track_layer_align", move |s| s.track_layer_align(&sid, &name)).await
}

/// Writes a file: on the blocking pool. `path` (P4-7d): the file the save dialog chose; default Exports/.
///
/// # Errors
///
/// 404 when the project or the track is unknown. 409 when the project is open read-only, the
/// track has no layer, its layer file is missing, or the track is offline. An error also comes
/// from a `path` that is not a valid save target (for example a file of the project that must not
/// be overwritten), and from reading the audio or writing the rendered file.
#[tauri::command]
pub async fn export_layer(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    path: Option<String>,
) -> Result<Value, AppError> {
    blocking(&svc, "export_layer", move |s| {
        s.export_layer_to(&sid, &name, path.as_deref())
    })
    .await
}

/// P4-7c: writes the calibration MIDI file: on the blocking pool. `path` (P4-7d): the file the save dialog chose.
///
/// # Errors
///
/// 404 when the project or the track is unknown. 409 when the project is open read-only. 400 when
/// the stored tempo map is not valid or has no usable bar 2. An error also comes from a `path` or
/// `dest` that is not a valid target, and 500 when the file cannot be written.
#[tauri::command]
pub async fn sampler_calibration_midi(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    dest: Option<String>,
    path: Option<String>,
) -> Result<Value, AppError> {
    blocking(&svc, "sampler_calibration_midi", move |s| {
        s.sampler_calibration_midi_to(&sid, &name, dest.as_deref(), path.as_deref())
    })
    .await
}

/// The sampler calibration of the track `name` (all null for a track without one).
///
/// # Errors
///
/// 404 when the project or the track is unknown.
#[tauri::command]
pub fn track_sampler_get(svc: State<'_, Service>, sid: String, name: String) -> Result<Value, AppError> {
    svc.track_sampler_get(&sid, &name)
}

/// P4-7c: reads the bounce and the close mic: on the blocking pool. An empty or missing `path` removes the calibration.
///
/// # Errors
///
/// 404 when the project or the track is unknown. 409 when the project is open read-only. 400 when
/// the stored tempo map is not valid, or the bounce cannot be read, is silent or cannot be
/// measured. With `one_shot`, the errors of [`track_layer_set`] also apply.
#[tauri::command(rename_all = "snake_case")]
pub async fn track_sampler_set(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    path: Option<String>,
    one_shot: Option<bool>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_sampler_set", move |s| {
        s.track_sampler_set(&sid, &name, &json!({"path": path, "one_shot": one_shot}))
    })
    .await
}
