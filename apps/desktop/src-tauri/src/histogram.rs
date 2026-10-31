//! P4-3: the Tauri wrappers of the Inspector histograms (`crates/app/src/histogram.rs`). Each
//! only converts its arguments and calls the method of `drumspike_app::Service` with the same
//! name, on the blocking pool (each reads the source audio). The argument names are the ones of
//! the test bridge (`histogram::dispatch`).

use drumspike_app::{AppError, Service};
use serde_json::{Map, Value};
use tauri::State;

use crate::blocking;

/// The loudness levels of the hits and the detection candidates of the track `name`.
///
/// # Errors
///
/// 404 when the project or the track is unknown. 400 when a hit in `events` (or in the stored
/// list) has no valid time. An error also comes from a track without a usable source (offline)
/// and from a source that cannot be decoded (500).
#[tauri::command]
pub async fn track_histograms(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    events: Option<Vec<Value>>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_histograms", move |s| {
        s.track_histograms(&sid, &name, events.as_deref())
    })
    .await
}

/// Pick the hits of the track `name` again from the kept candidates with the slider's `params`;
/// with `store` the result replaces the stored hits, otherwise it is a preview.
///
/// # Errors
///
/// 404 when the project or the track is unknown. 409 when `store` is set and the project is open
/// read-only, or when another write of the project is in progress. An error also comes from a
/// track that has no kept candidates to re-pick from, from `params` that do not parse (400) and
/// from a source that cannot be decoded (500).
#[tauri::command(rename_all = "snake_case")]
pub async fn track_repick(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    params: Option<Map<String, Value>>,
    store: Option<bool>,
    keep_edited: Option<bool>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_repick", move |s| {
        s.track_repick_keep(
            &sid,
            &name,
            params.as_ref(),
            store.unwrap_or(false),
            keep_edited.unwrap_or(false),
        )
    })
    .await
}

/// Fix round: the undo and redo of a re-pick store the slider's params again.
///
/// # Errors
///
/// 404 when the project or the track is unknown. 409 when the project is open read-only or
/// another write of the project is in progress.
#[tauri::command]
pub async fn track_params_set(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    params: Map<String, Value>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_params_set", move |s| {
        s.track_params_set(&sid, &name, &params)
    })
    .await
}

/// Store the velocity range (`lo_db`, `hi_db`, optional `cut_db`) of the track `name`; with
/// `apply` the velocities of the hits are recomputed.
///
/// # Errors
///
/// 400 when `lo_db`, `hi_db` or `cut_db` is not a finite number, or a hit in `events` is invalid.
/// 404 when the project or the track is unknown. 409 when the project is open read-only or
/// another write of the project is in progress. A rejected command changes nothing. An error
/// also comes from a source that cannot be decoded (500).
#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub async fn track_velocity_set(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    lo_db: f64,
    hi_db: f64,
    cut_db: Option<f64>,
    events: Option<Vec<Value>>,
    apply: Option<bool>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_velocity_set", move |s| {
        s.track_velocity_set_apply(
            &sid,
            &name,
            lo_db,
            hi_db,
            cut_db,
            events.as_deref(),
            apply.unwrap_or(false),
        )
    })
    .await
}

/// #150: the confidence of each hit for the review queue (`crates/app/src/confidence.rs`).
///
/// # Errors
///
/// 404 when the project or the track is unknown. 400 when a hit in `events` (or in the stored
/// list) has no valid time. An error also comes from a track without a usable source (offline)
/// and from a source that cannot be decoded (500). A track that follows another track answers
/// without reading audio.
#[tauri::command]
pub async fn track_confidence(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    events: Option<Vec<Value>>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_confidence", move |s| {
        s.track_confidence(&sid, &name, events.as_deref())
    })
    .await
}
