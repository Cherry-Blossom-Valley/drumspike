//! P4-3: the Tauri wrappers of the Inspector histograms (`crates/app/src/histogram.rs`). Each
//! only converts its arguments and calls the method of `drumspike_app::Service` with the same
//! name, on the blocking pool (each reads the source audio). The argument names are the ones of
//! the test bridge (`histogram::dispatch`).

use drumspike_app::{AppError, Service};
use serde_json::{Map, Value};
use tauri::State;

use crate::blocking;

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
