//! P4-6a: the Tauri wrappers of the Learn marks (`crates/app/src/learn.rs`; P4-6b: the scores,
//! `crates/app/src/learn_score.rs`). Each only converts
//! its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

use crate::blocking;

#[tauri::command(rename_all = "snake_case")]
pub async fn track_learn_mark(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    kind: String,
    t0: Option<f64>,
    t1: Option<f64>,
    hit: Option<f64>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_learn_mark", move |s| {
        let range = match (t0, t1) {
            (Some(a), Some(b)) => Some((a, b)),
            (None, None) => None,
            _ => return Err(AppError::bad("give both `t0` and `t1`")),
        };
        s.track_learn_mark(&sid, &name, &kind, range, hit)
    })
    .await
}

#[tauri::command]
pub async fn track_learn_remove(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    id: String,
    at: Option<f64>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_learn_remove", move |s| {
        s.track_learn_remove(&sid, &name, &id, at)
    })
    .await
}

#[tauri::command]
pub async fn track_learn_forget_at(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    t0: Option<f64>,
    t1: Option<f64>,
    hit: Option<f64>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_learn_forget_at", move |s| {
        let range = match (t0, t1) {
            (Some(a), Some(b)) => Some((a, b)),
            (None, None) => None,
            _ => return Err(AppError::bad("give both `t0` and `t1`")),
        };
        s.track_learn_forget_at(&sid, &name, range, hit)
    })
    .await
}

#[tauri::command]
pub async fn track_learn_flip(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    id: String,
) -> Result<Value, AppError> {
    blocking(&svc, "track_learn_flip", move |s| s.track_learn_flip(&sid, &name, &id)).await
}

#[tauri::command]
pub async fn track_learn_forget(svc: State<'_, Service>, sid: String, name: String) -> Result<Value, AppError> {
    blocking(&svc, "track_learn_forget", move |s| s.track_learn_forget(&sid, &name)).await
}

#[tauri::command]
pub async fn track_learn_set(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    learn: Value,
    events: Option<Value>,
) -> Result<Value, AppError> {
    // P4-6c: `events` is the snapshot the page's undo and redo restore
    blocking(&svc, "track_learn_set", move |s| {
        s.track_learn_set_events(&sid, &name, &learn, events.as_ref().filter(|v| !v.is_null()))
    })
    .await
}

#[tauri::command]
pub async fn track_learn_scores(svc: State<'_, Service>, sid: String, name: String) -> Result<Value, AppError> {
    blocking(&svc, "track_learn_scores", move |s| s.track_learn_scores(&sid, &name)).await
}
