//! Window and quit handling (P3-4b). The close button, Cmd-Q and the Dock menu's Quit do not end
//! the app by themselves: the page asks Save, Discard or Cancel for each project with unsaved
//! changes (`static/shell.js`) and then calls the command `quit_app`. A drop on the window goes to
//! `open.rs`.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, Runtime, Window, WindowEvent};

use crate::open;

/// `gate`: `true` once the page has answered every prompt and called `quit_app`. `page_ready`: `true`
/// once the page has called `shell_sync`, that is once it listens for the quit request. A page that
/// never loaded cannot ask, so the close button must not be held back then.
#[derive(Default)]
pub struct QuitGate {
    pub gate: AtomicBool,
    pub page_ready: AtomicBool,
}

/// Hold a close or quit request back for the page's prompts?
pub fn should_hold(page_ready: bool, gate_open: bool) -> bool {
    page_ready && !gate_open
}

/// The page listens: from now on a close request goes to its prompts.
pub fn mark_page_ready<R: Runtime>(app: &AppHandle<R>) {
    if let Some(g) = app.try_state::<QuitGate>() {
        g.page_ready.store(true, Ordering::SeqCst);
    }
}

/// Ask the page to run the quit flow (the event `menu` with the id `quit`).
pub fn request_quit<R: Runtime>(app: &AppHandle<R>, source: &str) {
    let _ = app.emit("menu", json!({"id": "quit", "source": source}));
}

fn holds<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<QuitGate>()
        .is_some_and(|g| should_hold(g.page_ready.load(Ordering::SeqCst), g.gate.load(Ordering::SeqCst)))
}

/// `Builder::on_window_event`: hold the close button back and take drops.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    let app = window.app_handle();
    match event {
        WindowEvent::CloseRequested { api, .. } => {
            // P3-17: only the main window asks the page; the splash window is closed by the shell
            if window.label() == "main" && holds(app) {
                api.prevent_close();
                request_quit(app, "window");
            }
        }
        WindowEvent::DragDrop(DragDropEvent::Drop { paths, position }) => {
            // P4-7a: the position in logical pixels (the page's coordinates)
            let s = window.scale_factor().unwrap_or(1.0).max(0.1);
            open::push_drop(
                app,
                paths.iter().map(|p| p.display().to_string()).collect(),
                (position.x / s, position.y / s),
            );
        }
        _ => {}
    }
}

/// `RunEvent::ExitRequested` without an exit code is the user asking to quit (Dock menu, last window
/// closed): hold it back until the page has asked. `code: Some(_)` comes from `quit_app`.
pub fn on_exit_requested<R: Runtime>(app: &AppHandle<R>, code: Option<i32>) -> bool {
    if code.is_some() || !holds(app) {
        return false;
    }
    request_quit(app, "exit");
    true
}

/// The page has answered every prompt: let the app end.
pub fn quit<R: Runtime>(app: &AppHandle<R>) {
    quit_with(app, 0);
}

/// The same with an exit code (the self-test ends the app this way, with 0 for pass and 1 for fail).
pub fn quit_with<R: Runtime>(app: &AppHandle<R>, code: i32) {
    if let Some(g) = app.try_state::<QuitGate>() {
        g.gate.store(true, Ordering::SeqCst);
    }
    app.exit(code);
}

/// P4-26: the native window title. The project name when a project is open, else the app name; unsaved changes add
/// an asterisk after the name ("Song*"). A blank name counts as no project.
pub fn window_title(project: Option<&str>, edited: bool) -> String {
    match project.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) if edited => format!("{n}*"),
        Some(n) => n.to_string(),
        None => "Drum Spike".to_string(),
    }
}

/// Set the title of the main window when it differs (a sync runs often; the OS call is skipped when nothing changed).
pub fn set_title<R: Runtime>(app: &AppHandle<R>, title: &str) {
    if let Some(w) = app.get_webview_window("main") {
        if w.title().is_ok_and(|t| t == title) {
            return;
        }
        let _ = w.set_title(title);
    }
}

/// P4-26: the dot in the red close button while the project has unsaved changes (macOS only; elsewhere nothing).
#[cfg(target_os = "macos")]
pub fn set_edited<R: Runtime>(app: &AppHandle<R>, edited: bool) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let Ok(ptr) = w.ns_window() else { return };
    let addr = ptr as usize; // the window outlives the call: the main thread runs it before the next event
    let _ = app.run_on_main_thread(move || {
        let win = addr as *mut objc2::runtime::AnyObject;
        // SAFETY: `win` is the NSWindow of the main window; setDocumentEdited: takes a BOOL and answers nothing.
        unsafe {
            let _: () = objc2::msg_send![win, setDocumentEdited: objc2::runtime::Bool::new(edited)];
        }
    });
}
#[cfg(not(target_os = "macos"))]
pub fn set_edited<R: Runtime>(_app: &AppHandle<R>, _edited: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_is_the_project_name_with_an_asterisk_after_unsaved_changes() {
        assert_eq!(window_title(None, false), "Drum Spike");
        assert_eq!(
            window_title(None, true),
            "Drum Spike",
            "no project, nothing to be edited"
        );
        assert_eq!(window_title(Some("Song"), false), "Song");
        assert_eq!(window_title(Some("Song"), true), "Song*");
        assert_eq!(window_title(Some("  "), true), "Drum Spike");
    }

    #[test]
    fn the_gate_starts_closed_and_the_page_not_ready() {
        let g = QuitGate::default();
        assert!(!g.gate.load(Ordering::SeqCst) && !g.page_ready.load(Ordering::SeqCst));
    }

    #[test]
    fn a_request_is_held_only_while_the_page_listens_and_has_not_answered() {
        assert!(!should_hold(false, false), "a page that never loaded cannot ask");
        assert!(should_hold(true, false));
        assert!(!should_hold(true, true), "after quit_app the app may end");
        assert!(!should_hold(false, true));
    }
}
