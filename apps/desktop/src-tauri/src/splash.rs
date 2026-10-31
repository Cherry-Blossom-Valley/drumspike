//! Splash screen and clean start (P3-17). The main window is created hidden (`visible: false` in
//! `tauri.conf.json`). `open` shows a small borderless splash window at once; the page tells the shell
//! with the command `app_ready` that its first layout is finished; then the main window is shown and the
//! splash is closed together. The splash stays at least [`MIN_MS`] from its first painted frame (#441:
//! the page reports that with `splash_painted`) and at most [`MAX_MS`] (then the main window is shown
//! whatever the page did). A click or any key on the splash dismisses it early (`splash_dismiss`).
//! The splash page is `static/splash.html`; the shell sends it the status line with `eval` (the splash
//! holds no other IPC).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// The splash stays at least this long from its first painted frame, so that it never flashes.
pub const MIN_MS: u64 = 5_000;
/// The main window is shown at the latest after this time, ready or not.
pub const MAX_MS: u64 = 15_000;
const TICK_MS: u64 = 40;
/// Size of the splash window in points.
pub const SIZE: (f64, f64) = (640.0, 400.0);

/// What the ticker does now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    Wait,
    /// The page is ready and the minimum time has passed.
    Ready,
    /// The maximum time has passed.
    Timeout,
}

/// Decide from the state of the page and the two clocks: `painted` is true once the splash page
/// reported its first painted frame and `paint_elapsed_ms` counts from that frame; `elapsed_ms`
/// counts from the opening of the splash. Without a paint report (a page that never painted) the
/// minimum falls back to the clock of the opening, as before #441.
#[must_use]
pub fn due(page_ready: bool, painted: bool, paint_elapsed_ms: u64, elapsed_ms: u64) -> Due {
    if elapsed_ms >= MAX_MS {
        Due::Timeout
    } else if page_ready && (if painted { paint_elapsed_ms } else { elapsed_ms }) >= MIN_MS {
        Due::Ready
    } else {
        Due::Wait
    }
}

/// The status line for a command of the page, with its stage (a status never goes back to an earlier stage).
#[must_use]
pub fn status_for_command(cmd: &str) -> Option<(u8, &'static str)> {
    match cmd {
        "project_list" | "project_open" => Some((3, "Opening the project\u{2026}")),
        "tracks_get" => Some((4, "Drawing the tracks\u{2026}")),
        _ => None,
    }
}

#[derive(Default)]
struct Inner {
    opened: Option<Instant>,
    stage: u8,
    status: String,
    steps: Vec<(u64, String)>,
    page_ready: bool,
    ready_ms: Option<u64>,
    /// The first painted frame of the splash page (#441): the instant and the ms since it opened.
    painted_at: Option<Instant>,
    painted_ms: Option<u64>,
    main_visible_at_ready: Option<bool>,
    main_shown_ms: Option<u64>,
    closed_ms: Option<u64>,
    /// Ms from the first painted frame to the close, when the page painted.
    closed_after_paint_ms: Option<u64>,
    reason: &'static str,
    finished: bool,
    plant: Option<String>,
}

impl Inner {
    fn ms(&self) -> u64 {
        self.opened.map_or(0, |t| t.elapsed().as_millis() as u64)
    }
    fn set_status(&mut self, stage: u8, text: &str) {
        if self.finished || stage < self.stage {
            return;
        }
        self.stage = stage;
        if self.status != text {
            self.status = text.to_string();
            let t = self.ms();
            self.steps.push((t, text.to_string()));
        }
    }
}

/// The state of the splash, managed by the app.
#[derive(Default)]
pub struct SplashState(Mutex<Inner>);

/// Debug builds only: `DRUMSPIKE_SPLASH_PLANT=early_show` shows the main window at once (the self-test
/// must fail), `keep_splash` never finishes (the self-test must fail). Release builds ignore it.
fn plant() -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var("DRUMSPIKE_SPLASH_PLANT").ok().filter(|v| !v.is_empty())
    } else {
        None
    }
}

fn js_status(text: &str) -> String {
    format!(
        "window.__splashSet&&window.__splashSet({})",
        Value::String(text.to_string())
    )
}

/// Set the status line (stage: 1 model, 2 editor, 3 project, 4 tracks).
pub fn status<R: Runtime>(app: &AppHandle<R>, stage: u8, text: &str) {
    if let Some(s) = app.try_state::<SplashState>()
        && let Ok(mut i) = s.0.lock()
    {
        i.set_status(stage, text);
    }
}

/// A command of the main page arrived: move the status line on when the command is a startup step.
pub fn on_command<R: Runtime>(app: &AppHandle<R>, cmd: &str) {
    if let Some((stage, text)) = status_for_command(cmd) {
        status(app, stage, text);
    }
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Open the splash window and start the ticker. When the window cannot be built, the main window is shown
/// at once (an app without a splash is better than an app without a window).
pub fn open<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<SplashState>() else {
        return;
    };
    let version = app.package_info().version.to_string();
    {
        let Ok(mut i) = state.0.lock() else { return };
        i.opened = Some(Instant::now());
        i.plant = plant();
        i.set_status(0, "Starting Drum Spike\u{2026}");
    }
    let init = format!("window.__SPLASH__={};", json!({"version": version}));
    let built = WebviewWindowBuilder::new(app, "splash", WebviewUrl::App("splash.html".into()))
        .title("Drum Spike")
        .inner_size(SIZE.0, SIZE.1)
        .decorations(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .shadow(true)
        .skip_taskbar(true)
        .center()
        .initialization_script(init)
        .build();
    if built.is_err() {
        if let Ok(mut i) = state.0.lock() {
            i.finished = true;
            i.reason = "no_splash";
        }
        show_main(app);
        return;
    }
    if state.0.lock().is_ok_and(|i| i.plant.as_deref() == Some("early_show")) {
        show_main(app);
    }
    let app = app.clone();
    std::thread::Builder::new()
        .name("ds-splash".into())
        .spawn(move || ticker(&app))
        .ok();
}

fn ticker<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<SplashState>() else {
        return;
    };
    loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        let (action, text, keep) = {
            let Ok(i) = state.0.lock() else { return };
            let paint_elapsed = i.painted_at.map_or(0, |t| t.elapsed().as_millis() as u64);
            (
                due(i.page_ready, i.painted_at.is_some(), paint_elapsed, i.ms()),
                i.status.clone(),
                i.plant.as_deref() == Some("keep_splash"),
            )
        };
        // the page of the splash may load after the first status: send the line on every tick
        if let Some(w) = app.get_webview_window("splash") {
            let _ = w.eval(js_status(&text));
        }
        if keep || action == Due::Wait {
            continue;
        }
        finish(app, if action == Due::Ready { "ready" } else { "timeout" });
        return;
    }
}

/// Close the splash: show the main window, destroy the splash window, record the times.
/// `reason` is `ready`, `timeout` or `dismissed` (#441).
fn finish<R: Runtime>(app: &AppHandle<R>, reason: &'static str) {
    let Some(state) = app.try_state::<SplashState>() else {
        return;
    };
    if let Ok(mut i) = state.0.lock() {
        if i.finished {
            return;
        }
        i.finished = true;
        i.reason = reason;
        i.status = "Ready".into();
    }
    show_main(app);
    let t = state.0.lock().map_or(0, |i| i.ms());
    if let Some(w) = app.get_webview_window("splash") {
        let _ = w.destroy();
    }
    if let Ok(mut i) = state.0.lock() {
        i.main_shown_ms = Some(t);
        i.closed_ms = Some(i.ms());
        i.closed_after_paint_ms = i.painted_at.map(|t| t.elapsed().as_millis() as u64);
    };
}

/// The page says that its first layout is finished and drawn: the main window may be shown. The splash
/// closes with it, at the earliest [`MIN_MS`] after its first painted frame.
#[tauri::command]
#[must_use]
pub fn app_ready(app: AppHandle) -> Value {
    let visible = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    let Some(state) = app.try_state::<SplashState>() else {
        return json!({"ok": true});
    };
    let Ok(mut i) = state.0.lock() else {
        return json!({"ok": false});
    };
    if i.page_ready {
        return json!({"ok": true, "again": true});
    }
    i.page_ready = true;
    let t = i.ms();
    i.ready_ms = Some(t);
    i.main_visible_at_ready = Some(visible);
    i.set_status(5, "Ready");
    json!({"ok": true})
}

/// #441: the splash page reports its first painted frame. The 5 s minimum counts from here.
/// The first call wins; later ones change nothing.
#[tauri::command]
#[must_use]
pub fn splash_painted(app: AppHandle) -> Value {
    let Some(state) = app.try_state::<SplashState>() else {
        return json!({"ok": false});
    };
    let Ok(mut i) = state.0.lock() else {
        return json!({"ok": false});
    };
    if i.painted_at.is_none() {
        i.painted_at = Some(Instant::now());
        i.painted_ms = Some(i.ms());
    }
    json!({"ok": true})
}

/// #441: a click or any key on the splash dismisses it early: the main window is shown and the
/// splash closes at once. The report says `reason: "dismissed"`.
#[tauri::command]
#[must_use]
pub fn splash_dismiss(app: AppHandle) -> Value {
    let Some(state) = app.try_state::<SplashState>() else {
        return json!({"ok": false});
    };
    let finished = state.0.lock().map_or(true, |i| i.finished);
    if !finished {
        finish(&app, "dismissed");
    }
    json!({"ok": true})
}

/// The record of the start, for the self-test (debug builds): times in ms from the opening of the splash.
pub fn report(i: &SplashState) -> Value {
    let Ok(i) = i.0.lock() else { return Value::Null };
    json!({
        "page_ready": i.page_ready,
        "ready_ms": i.ready_ms,
        "painted_ms": i.painted_ms,
        "main_visible_at_ready": i.main_visible_at_ready,
        "main_shown_ms": i.main_shown_ms,
        "splash_closed_ms": i.closed_ms,
        "closed_after_paint_ms": i.closed_after_paint_ms,
        "reason": i.reason,
        "finished": i.finished,
        "steps": i.steps.iter().map(|(t, s)| json!({"ms": t, "text": s})).collect::<Vec<_>>(),
        "min_ms": MIN_MS,
        "max_ms": MAX_MS,
    })
}

#[cfg(debug_assertions)]
#[tauri::command]
#[must_use]
pub fn splash_report(app: AppHandle) -> Value {
    let main_visible = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    let splash_open = app.get_webview_window("splash").is_some();
    let mut v = app.try_state::<SplashState>().map_or(Value::Null, |s| report(&s));
    if let Some(o) = v.as_object_mut() {
        o.insert("main_visible_now".into(), json!(main_visible));
        o.insert("splash_window_open".into(), json!(splash_open));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_splash_waits_for_the_page_the_paint_and_the_minimum_time() {
        assert_eq!(due(false, false, 0, 0), Due::Wait);
        assert_eq!(due(true, false, 0, MIN_MS - 1), Due::Wait);
        // no paint report: the minimum falls back to the clock of the opening (the P3-17 rule)
        assert_eq!(due(true, false, 0, MIN_MS), Due::Ready);
        // a painted splash waits for the minimum time from the first painted frame
        assert_eq!(due(true, true, MIN_MS - 1, MIN_MS - 1), Due::Wait);
        assert_eq!(due(true, true, MIN_MS, MIN_MS), Due::Ready);
        assert_eq!(
            due(false, true, MAX_MS - 1, MAX_MS - 1),
            Due::Wait,
            "a page that is not ready keeps the splash"
        );
    }

    #[test]
    fn the_main_window_is_shown_after_the_maximum_time_whatever_the_page_did() {
        assert_eq!(due(false, true, 0, MAX_MS), Due::Timeout);
        assert_eq!(due(true, true, MIN_MS, MAX_MS + 1), Due::Timeout);
        const { assert!(MIN_MS == 5_000 && MAX_MS == 15_000) };
    }

    #[test]
    fn the_status_line_goes_forward_only() {
        let mut i = Inner::default();
        i.set_status(0, "Starting");
        i.set_status(3, "Opening the project");
        i.set_status(1, "Checking the model");
        assert_eq!(
            i.status, "Opening the project",
            "a later step from an earlier stage is ignored"
        );
        i.set_status(3, "Opening the project");
        assert_eq!(i.steps.len(), 2, "the same text twice is one step");
        i.finished = true;
        i.set_status(5, "Ready");
        assert_eq!(i.status, "Opening the project", "after the end nothing changes");
    }

    #[test]
    fn commands_of_the_startup_move_the_status_line() {
        assert_eq!(status_for_command("project_list").unwrap().0, 3);
        assert!(status_for_command("tracks_get").unwrap().0 > status_for_command("project_list").unwrap().0);
        assert!(status_for_command("transport_play").is_none());
    }

    #[test]
    fn the_status_reaches_the_page_as_a_quoted_string() {
        let js = js_status("a \"b\"\n");
        assert_eq!(js, "window.__splashSet&&window.__splashSet(\"a \\\"b\\\"\\n\")");
    }

    #[test]
    fn the_report_lists_the_steps_and_the_limits() {
        let s = SplashState::default();
        s.0.lock()
            .unwrap()
            .set_status(1, "Checking the detection model\u{2026}");
        let v = report(&s);
        assert_eq!(v["steps"][0]["text"], "Checking the detection model\u{2026}");
        assert_eq!(v["min_ms"], 5_000);
        assert_eq!(v["page_ready"], false);
        assert_eq!(v["painted_ms"], Value::Null, "no paint report yet");
        assert_eq!(v["closed_after_paint_ms"], Value::Null);
    }

    #[test]
    fn the_plant_is_off_without_the_variable() {
        if std::env::var_os("DRUMSPIKE_SPLASH_PLANT").is_none() {
            assert_eq!(plant(), None);
        }
    }

    #[test]
    fn the_config_hides_the_main_window_and_the_splash_page_exists() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let conf: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("tauri.conf.json")).unwrap()).unwrap();
        let main = &conf["app"]["windows"][0];
        assert_eq!(main["label"], "main");
        assert_eq!(main["visible"], false, "the main window starts hidden");
        let html = std::fs::read_to_string(root.join("../../../static/splash.html")).unwrap();
        assert!(html.contains("brought to you by") && html.contains("cbv-logo-white.png"));
        assert!(root.join("../../../static/brand/cbv-logo-white.png").is_file());
    }
}
