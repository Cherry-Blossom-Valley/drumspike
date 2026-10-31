//! The Tauri commands of the native shell (P3-4b). Each only converts its arguments and calls the
//! service, `menu.rs`, `open.rs` or `window.rs`. The self-test commands exist in debug builds only.

use drumspike_app::{AppError, ListShape, Service};
use serde_json::Value;
#[cfg(debug_assertions)]
use serde_json::json;
use tauri::{AppHandle, State};

use crate::{menu, open, window};

/// The page tells the menu whether a project is open (greys Save, Close and so on) and refreshes Open
/// Recent from the service's list. P3-13a: `state` (`{current, snap, follow, click}`, optional) sets the
/// check items and the Window menu (the service's open projects, the current one checked); `state.keyPreset` (P3-13c)
/// sets the accelerators of the key preset; `projectName` and `dirty` (P4-26) set the window title.
/// Answer: the number of recent entries.
#[tauri::command(rename_all = "snake_case")]
pub fn shell_sync(
    app: AppHandle,
    svc: State<'_, Service>,
    has_project: bool,
    state: Option<Value>,
) -> Result<usize, AppError> {
    let recent: Vec<String> = svc
        .project_recent()
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.get("folder").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    window::mark_page_ready(&app); // the page listens for the quit request from now on
    let err = |e: tauri::Error| AppError::internal(format!("menu: {e}"));
    menu::sync(&app, has_project, &recent).map_err(err)?;
    let state = state.unwrap_or(Value::Null);
    let current = state.get("current").and_then(Value::as_str);
    menu::sync_window(&app, &menu::window_entries(&svc.project_list(ListShape::Ids), current)).map_err(err)?;
    menu::sync_checks(&app, &state).map_err(err)?;
    menu::sync_follow(&app, &menu::follow_entries(&state)).map_err(err)?; // P3-18: Track > Follow Hits Of
    // #169: Zoom Presets > the own presets
    menu::sync_zoom_own(&app, &menu::zoom_own_names(&state)).map_err(err)?;
    // P3-13c: the key preset (app setting `key_preset`) sets the accelerators; a missing or unknown name keeps the menu as it is
    if let Some(p) = state.get("keyPreset").and_then(Value::as_str) {
        menu::set_preset(&app, p).map_err(err)?;
    }
    // P4-26: the window title from the page state (`projectName`, `dirty`); no project: the app name
    let project = if has_project {
        state.get("projectName").and_then(Value::as_str)
    } else {
        None
    };
    let edited = state.get("dirty").and_then(Value::as_bool).unwrap_or(false);
    window::set_title(&app, &window::window_title(project, edited));
    window::set_edited(&app, project.is_some() && edited);
    Ok(recent.len().min(menu::RECENT_MAX))
}

/// Take the paths that came from the Finder, the Dock or a drop: `{projects, others}`.
#[tauri::command]
pub fn open_paths_take(app: AppHandle) -> Value {
    open::take(&app)
}

/// The page has answered every prompt of the quit flow: end the app.
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    window::quit(&app);
}

/// The files that Export all would replace (P4-30: for the options of the Export dialog): `{count, files, dest, ...}`.
#[tauri::command(rename_all = "snake_case")]
pub fn export_existing(
    svc: State<'_, Service>,
    sid: String,
    dest: Option<String>,
    midi_mode: Option<String>,
    tracks: Option<Vec<String>>,
    formats: Option<Vec<String>>,
    compensate: Option<bool>,
) -> Result<Value, AppError> {
    let opts = export_opts(dest, midi_mode, tracks, formats, compensate)?;
    svc.export_existing_opts(&sid, &opts)
}

/// P4-30: keep the export folder of the project (`null` back to `Exports/`).
#[tauri::command]
pub fn export_dest_set(svc: State<'_, Service>, sid: String, dest: Option<String>) -> Result<Value, AppError> {
    svc.export_dest_set(&sid, dest.as_deref())
}

// P4-40g: keep the "Markers in MIDI" checkbox of the Export dialog.
#[tauri::command]
pub fn export_markers_set(svc: State<'_, Service>, sid: String, on: bool) -> Result<Value, AppError> {
    svc.export_markers_set(&sid, on)
}

/// P4-31: the export shape of the project (what the dialog shows again and Export again repeats); `null`
/// before the first export.
#[tauri::command]
pub fn export_last_get(svc: State<'_, Service>, sid: String) -> Result<Value, AppError> {
    svc.export_last_get(&sid)
}

/// P4-31: keep the export shape of the project (the dialog's choices at once, and what an export ran with).
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub fn export_last_set(
    svc: State<'_, Service>,
    sid: String,
    formats: Vec<String>,
    midi_mode: Option<String>,
    compensate: Option<bool>,
    scope: Option<String>,
    track: Option<String>,
    dest: Option<String>,
) -> Result<Value, AppError> {
    let args = serde_json::json!({
        "formats": formats, "midi_mode": midi_mode, "compensate": compensate,
        "scope": scope, "track": track, "dest": dest,
    });
    svc.export_last_set(&sid, &args)
}

/// P4-30: render the files of the dialog's drag tiles into `Exports/.drag/` (file work: the blocking pool).
#[tauri::command(rename_all = "snake_case")]
pub async fn export_drag_prepare(
    svc: State<'_, Service>,
    sid: String,
    midi_mode: Option<String>,
    tracks: Option<Vec<String>>,
    formats: Option<Vec<String>>,
    compensate: Option<bool>,
    clear: Option<bool>,
) -> Result<Value, AppError> {
    if clear == Some(true) {
        return crate::blocking(&svc, "export_drag_clear", move |s| s.export_drag_clear(&sid)).await;
    }
    let opts = export_opts(None, midi_mode, tracks, formats, compensate)?;
    crate::blocking(&svc, "export_drag_prepare", move |s| s.export_drag_prepare(&sid, &opts)).await
}

/// #433: a drag starts: the pre-rendered files move from `Exports/.drag/` into `Exports/` (file work: the blocking pool).
#[tauri::command(rename_all = "snake_case")]
pub async fn export_drag_take(svc: State<'_, Service>, sid: String, paths: Vec<String>) -> Result<Value, AppError> {
    crate::blocking(&svc, "export_drag_take", move |s| s.export_drag_take(&sid, &paths)).await
}

/// P4-30: show a file or folder in the Finder (the Export dialog's "Show in Finder"): `open -R <path>`.
#[tauri::command]
pub fn reveal_path(path: String) -> Result<Value, AppError> {
    let p = std::path::Path::new(&path);
    if !p.is_absolute() || !p.exists() {
        return Err(AppError::not_found(format!("no such file or folder: {path}")));
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(p)
            .status()
            .map_err(|e| AppError::internal(format!("open: {e}")))?;
        Ok(Value::Null)
    }
    #[cfg(not(target_os = "macos"))]
    Err(AppError::internal("Show in Finder is a macOS feature"))
}

fn export_opts(
    dest: Option<String>,
    midi_mode: Option<String>,
    tracks: Option<Vec<String>>,
    formats: Option<Vec<String>>,
    compensate: Option<bool>,
) -> Result<drumspike_app::export::ExportOpts, AppError> {
    drumspike_app::export::ExportOpts::from_args(&serde_json::json!({
        "dest": dest, "midi_mode": midi_mode, "tracks": tracks, "formats": formats, "compensate": compensate }))
}

/// Self-test: a click on the menu item `id`, through the code that a real click runs.
#[cfg(debug_assertions)]
#[tauri::command]
pub fn selftest_menu(app: AppHandle, id: String) -> Value {
    menu::dispatch(&app, &id)
}

/// Self-test: the items of the live menu with their enabled state.
#[cfg(debug_assertions)]
#[tauri::command]
pub fn selftest_menu_report(app: AppHandle) -> Value {
    menu::report(&app)
}

/// Self-test: paths arrive as if the Finder had sent them (`RunEvent::Opened`).
#[cfg(debug_assertions)]
#[tauri::command]
pub fn selftest_open_paths(app: AppHandle, paths: Vec<String>) {
    open::push(&app, paths);
}

/// Self-test: the close button. `WebviewWindow::close` sends `CloseRequested` like the button.
#[cfg(debug_assertions)]
#[tauri::command]
pub fn selftest_close_window(app: AppHandle) -> Result<(), AppError> {
    use tauri::Manager;
    app.get_webview_window("main")
        .ok_or_else(|| AppError::internal("no main window"))?
        .close()
        .map_err(|e| AppError::internal(e.to_string()))
}

/// Self-test (#206): the main menu as macOS has it (NSMenu key equivalents and modifier masks).
#[cfg(debug_assertions)]
#[tauri::command]
pub fn selftest_ns_menu(app: AppHandle) -> Value {
    menu::ns_menu_rows(&app)
}

/// Self-test (#206): a `menu_key` through the real native menu path (`NSMenu performKeyEquivalent:`).
#[cfg(debug_assertions)]
#[tauri::command]
pub fn selftest_menu_key(app: AppHandle, key: String) -> Value {
    menu::perform_key_equivalent(&app, &key)
}

/// Self-test (#206) diagnosis: the menu ids that reached `dispatch` so far.
#[cfg(debug_assertions)]
#[tauri::command]
pub fn selftest_menu_log() -> Value {
    serde_json::to_value(
        menu::MENU_LOG
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone(),
    )
    .unwrap_or(json!([]))
}

/// Self-test (#206) diagnosis: a synthetic keyDown with explicit characters/flags, for probing which
/// variant of an event the NSMenu matches (`selftest_menu_key` builds the physical-looking one).
#[cfg(debug_assertions)]
#[tauri::command]
pub fn selftest_menu_probe(app: AppHandle, chars: String, flags: u64) -> Value {
    json!({"matched": menu::perform_key_equivalent_raw(&app, &chars, &chars, flags)})
}
