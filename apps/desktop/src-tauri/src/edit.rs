//! P4-5: the Tauri wrappers of the hit and timing edits (`crates/app/src/edit.rs`). Each only
//! converts its arguments and calls the method of `drumspike_app::Service` with the same name.

use drumspike_app::{AppError, Service};
use serde_json::Value;
use tauri::State;

use crate::blocking;

/// Apply the hit edits `ops` to the track `name` of the project `sid`.
///
/// # Errors
///
/// 400 for an empty `ops` or an op that does not validate (nothing is changed then). 404 when the
/// project or the track is unknown. 409 when the project is open read-only, when the track follows
/// another track, or when another write of the project is in progress (retry).
/// 500 when a snap cannot decode the source audio.
#[tauri::command]
pub async fn events_edit(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    ops: Vec<Value>,
    training: Option<bool>,
    view: Option<Value>,
    by: Option<Value>,
) -> Result<Value, AppError> {
    // #389: who made the edit (the page sends "person" or "agent" with the training flag); checked as the bridge does
    let by = drumspike_app::edit::edit_author(by.as_ref())?.map(str::to_string);
    blocking(&svc, "events_edit", move |s| {
        s.events_edit_training_by(&sid, &name, &ops, training.unwrap_or(false), view, by.as_deref())
    })
    .await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use drumspike_app::edit::edit_author;

    #[test]
    fn the_author_of_an_edit_is_person_or_agent_and_nothing_else() {
        assert_eq!(edit_author(None).unwrap(), None);
        assert_eq!(edit_author(Some(&json!(null))).unwrap(), None);
        assert_eq!(edit_author(Some(&json!("agent"))).unwrap(), Some("agent"));
        assert_eq!(edit_author(Some(&json!("person"))).unwrap(), Some("person"));
        for bad in [json!("robot"), json!(1), json!(true), json!(["agent"])] {
            assert_eq!(edit_author(Some(&bad)).unwrap_err().code, 400, "{bad}");
        }
    }
}

/// The Training Mode state of the project `sid` (`{on, records, path}`); `on` sets the state first.
///
/// # Errors
///
/// 404 when the project is unknown. 500 when the app has no home folder.
#[tauri::command]
pub async fn training_status(svc: State<'_, Service>, sid: String, on: Option<bool>) -> Result<Value, AppError> {
    blocking(&svc, "training_status", move |s| s.training_status(&sid, on)).await
}

/// Set the Hitpoint shift of the track `name` and move its hits by the difference.
///
/// # Errors
///
/// 400 when `shift_samples` is outside the shift limit or the track has no sample rate. 404 when
/// the project or the track is unknown. 409 when the project is open read-only, the track follows
/// another track, or an analysis or a re-pick store is running.
#[tauri::command(rename_all = "snake_case")]
pub async fn track_shift_set(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    shift_samples: i64,
) -> Result<Value, AppError> {
    blocking(&svc, "track_shift_set", move |s| {
        s.track_shift_set(&sid, &name, shift_samples)
    })
    .await
}
