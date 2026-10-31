//! P4-6a: the Tauri wrappers of the Learn marks (`crates/app/src/learn.rs`; P4-6b: the scores,
//! `crates/app/src/learn_score.rs`). Each only converts
//! its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

use crate::blocking;

/// Store one Learn mark (`kind`) for the range `t0`..`t1` or the single `hit` of the track `name`.
///
/// # Errors
///
/// 400 when only one of `t0` and `t1` is given, when neither a range nor a `hit` (or both) is
/// given, when a time is not a finite number, when `kind` is not valid, when the hit is not
/// there, or when the kind would hold more than the maximum number of examples. 404 when the
/// project or the track is unknown. 409 when the project is open read-only, when a follower's
/// source track is gone, or when an offline track has audio the rule needs.
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

/// Remove the mark `id` of the track `name`, or with `at` only its example at that time.
///
/// # Errors
///
/// 400 when there is no mark `id`, or the mark has no example at `at`. 404 when the project or
/// the track is unknown. 409 when the project is open read-only, or a follower's source track is
/// gone.
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

/// Remove the examples at one `hit` or inside the range `t0`..`t1` (Forget This).
///
/// # Errors
///
/// 400 when only one of `t0` and `t1` is given, when neither a range nor a `hit` (or both) is
/// given, or when a time is not a finite number. 404 when the project or the track is unknown.
/// 409 when the project is open read-only, or a follower's source track is gone.
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

/// Change the mark `id` from "this" to "not" or back.
///
/// # Errors
///
/// 400 when there is no mark `id`, or the other kind would hold more than the maximum number of
/// examples. 404 when the project or the track is unknown. 409 when the project is open
/// read-only, or a follower's source track is gone.
#[tauri::command]
pub async fn track_learn_flip(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    id: String,
) -> Result<Value, AppError> {
    blocking(&svc, "track_learn_flip", move |s| s.track_learn_flip(&sid, &name, &id)).await
}

/// Remove every Learn mark of the track `name` (none is fine) and the learned hits they supported.
///
/// # Errors
///
/// 404 when the project or the track is unknown. 409 when the project is open read-only, or a
/// follower's source track is gone. The sweep of the unsupported learned hits can fail like an
/// hit edit.
#[tauri::command]
pub async fn track_learn_forget(svc: State<'_, Service>, sid: String, name: String) -> Result<Value, AppError> {
    blocking(&svc, "track_learn_forget", move |s| s.track_learn_forget(&sid, &name)).await
}

/// Store the whole `learn` object (null removes it) and, with `events`, that hit list: the undo
/// and redo of a Learn step.
///
/// # Errors
///
/// 400 when `events` is not a list, `learn` is neither an object nor null, or the object does not
/// pass the Learn check. 404 when the project or the track is unknown. 409 when the project is
/// open read-only, or a follower's source track is gone.
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

/// The similarity score of each candidate and each example of the track `name`. A track without
/// kept candidates answers `{state: "missing"}`, not an error.
///
/// # Errors
///
/// 404 when the project or the track is unknown. 409 when the track is offline, its source is
/// gone, or a follower's source track is gone.
#[tauri::command]
pub async fn track_learn_scores(svc: State<'_, Service>, sid: String, name: String) -> Result<Value, AppError> {
    blocking(&svc, "track_learn_scores", move |s| s.track_learn_scores(&sid, &name)).await
}
