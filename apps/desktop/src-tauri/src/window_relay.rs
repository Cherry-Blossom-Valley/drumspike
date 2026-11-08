//! #388: the Tauri wrappers of the window endpoint's two page commands (`crates/app/src/window_relay.rs`). Each only
//! builds the argument object and calls the function of `drumspike_app::window_relay`. The poll waits up to 30 s, so it
//! runs on the blocking pool and holds neither the main thread nor an async worker.

use drumspike_app::{AppError, Service, window_relay};
use serde_json::{Value, json};
use tauri::State;

use crate::blocking;

/// The page waits for the next helper call of the window endpoint (long poll): `{enabled, call}`.
///
/// # Errors
///
/// 400 when `timeout_ms` is not a number.
#[tauri::command(rename_all = "snake_case")]
pub async fn window_call_next(
    svc: State<'_, Service>,
    timeout_ms: Option<u64>,
    poller: Option<String>,
) -> Result<Value, AppError> {
    blocking(&svc, "window_call_next", move |s| {
        window_relay::window_call_next(&s, &json!({"timeout_ms": timeout_ms, "poller": poller}))
    })
    .await
}

/// The page answers the helper call `id` with a value (`ok`) or an error message.
///
/// # Errors
///
/// 404 when no call `id` waits for an answer.
#[tauri::command]
pub fn window_call_result(
    svc: State<'_, Service>,
    id: u64,
    ok: bool,
    value: Option<Value>,
    error: Option<String>,
) -> Result<Value, AppError> {
    window_relay::window_call_result(&svc, &json!({"id": id, "ok": ok, "value": value, "error": error}))
}
