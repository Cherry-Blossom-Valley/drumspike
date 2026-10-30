//! Drum Spike desktop shell (P3-1b, P3-3a). A thin Tauri 2 layer: the window, the bundle
//! resources and the commands. Every command only converts its arguments and calls a method
//! of `drumspike_app::Service` (`crates/app`), which holds the logic and the tests.

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use drumspike_app::{AppError, ListShape, Progress, Service, ServiceConfig};
use serde_json::{Map, Value};
use tauri::ipc::{Channel, Response};
use tauri::{Emitter, Manager, State};

pub mod about;
pub mod edit;
pub mod histogram;
pub mod import;
pub mod jobs;
pub mod layer;
pub mod learn;
pub mod lifecycle;
pub mod markers;
pub mod menu;
pub mod open;
pub mod rowheights;
#[cfg(debug_assertions)]
pub mod selftest;
pub mod shell;
pub mod splash;
pub mod tempo;
pub mod tracks;
pub mod window;

pub use drumspike_app::{CHAIN, DETECTOR, app_info_value};

/// Start of the process, for the optional cold-start log.
static START: OnceLock<Instant> = OnceLock::new();

/// P3-15c: the workers of the async runtime. Every command that reads or writes files runs its
/// body on the blocking pool ([`blocking`]), so the workers only start and finish commands.
pub const ASYNC_WORKERS: usize = 4;

/// The runtime that `run` gives Tauri (`tauri::async_runtime::set`); it lives as long as the process.
static RUNTIME: OnceLock<tauri::async_runtime::TokioRuntime> = OnceLock::new();

/// Build the runtime with [`ASYNC_WORKERS`] workers (threads `ds-async`) and hand it to Tauri.
/// False when Tauri's runtime was set already (a second call).
fn async_runtime_setup() -> bool {
    if RUNTIME.get().is_some() {
        return false;
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(ASYNC_WORKERS)
        .thread_name("ds-async")
        .enable_all()
        .build()
        .expect("the async runtime");
    let handle = rt.handle().clone();
    if RUNTIME.set(rt).is_err() {
        return false;
    }
    tauri::async_runtime::set(handle);
    true
}

/// P3-15c: run a command body on the blocking pool of the async runtime, so a command that
/// reads or writes files keeps the main thread (and the async workers) free. The span `cmd`
/// (feature `trace`) is on the pool thread.
pub(crate) async fn blocking<T, F>(svc: &Service, name: &'static str, f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(Service) -> Result<T, AppError> + Send + 'static,
{
    let svc = svc.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _span = drumspike_app::trace::command(name);
        f(svc)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

/// P3-15c: every IPC call is a span `ipc` (feature `trace`) on the thread that dispatches it, the
/// main thread: for a non-async command it covers the whole command, for an async one the
/// dispatch only (its body is a span `cmd` on the blocking pool, see [`blocking`]).
fn traced(
    h: impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    move |invoke| {
        let _span = drumspike_app::trace::ipc(invoke.message.command());
        if invoke.message.webview().label() == "main" {
            splash::on_command(invoke.message.webview().app_handle(), invoke.message.command());
        }
        h(invoke)
    }
}

/// #281 D16: main is model-free. The `models_dir` plumbing of the model path lived here and moved
/// to the model branch; `ServiceConfig::from_env` ignores the folder now.
/// Modes without a window (P3-11), for a Mac without a login session (the Intel test host):
///
/// - `--app-info`: print the `app_info` answer (the detector and chain) as one JSON line.
/// - `cli ...` and `mcp` (P4-5): the agent API, see `drumspike_app::agent`
/// - `--analyze <project_dir>...`: the output of `service_tool analyze` (one JSON line per
///   folder), for `tools/parity/check_service.py --bin`; exit 0, or 1 on an error.
///
/// Any other first argument returns `None` and the app starts as usual.
pub fn headless(args: &[String]) -> Option<i32> {
    headless_as("", args)
}

/// [`headless`] with the program name: P4-5 adds `cli` and `mcp` (the command-line tool and the MCP server of
/// `drumspike_app::agent`), and a binary called `drumspike` (the symlink the user makes) is the command-line tool.
pub fn headless_as(program: &str, args: &[String]) -> Option<i32> {
    let by_name = Path::new(program)
        .file_name()
        .is_some_and(|n| n == drumspike_app::agent::TOOL_NAME);
    if by_name || matches!(args.first().map(String::as_str), Some("cli" | "mcp")) {
        return drumspike_app::agent::entry(program, args, Path::new(""));
    }
    let mode = args.first()?.as_str();
    if mode != "--app-info" && mode != "--analyze" {
        return None;
    }
    if mode == "--app-info" {
        let v = app_info_value(Path::new(""), true);
        println!("{v}");
        return Some(0);
    }
    let svc = Service::new(ServiceConfig::from_env(Path::new("")));
    match drumspike_app::headless::analyze_projects(&svc, &args[1..]) {
        Ok(rows) => {
            for v in rows {
                println!("{v}");
            }
            Some(0)
        }
        Err(e) => {
            eprintln!("drumspike-desktop --analyze: {e}");
            Some(1)
        }
    }
}

#[tauri::command]
async fn app_info(svc: State<'_, Service>) -> Result<Value, AppError> {
    blocking(&svc, "app_info", |s| Ok(s.app_info())).await
}

#[tauri::command(rename_all = "snake_case")]
async fn project_open(svc: State<'_, Service>, path: String, read_only: Option<bool>) -> Result<Value, AppError> {
    blocking(&svc, "project_open", move |s| {
        s.project_open_with(&path, read_only.unwrap_or(false))
    })
    .await
}

#[tauri::command]
async fn project_get(svc: State<'_, Service>, sid: String) -> Result<Value, AppError> {
    blocking(&svc, "project_get", move |s| s.project_get(&sid)).await
}

#[tauri::command]
fn project_list(svc: State<'_, Service>, shape: Option<String>) -> Value {
    svc.project_list(ListShape::parse(shape.as_deref()))
}

#[tauri::command]
async fn tracks_get(svc: State<'_, Service>, sid: String) -> Result<Value, AppError> {
    blocking(&svc, "tracks_get", move |s| s.tracks_get(&sid)).await
}

#[tauri::command]
fn events_get(svc: State<'_, Service>, sid: String, name: String) -> Result<Value, AppError> {
    svc.events_get(&sid, &name)
}

#[tauri::command]
fn events_set(svc: State<'_, Service>, sid: String, name: String, events: Value) -> Result<Value, AppError> {
    svc.events_set(&sid, &name, &events)
}

#[tauri::command]
async fn peaks_info(svc: State<'_, Service>, sid: String, name: String) -> Result<Value, AppError> {
    blocking(&svc, "peaks_info", move |s| s.peaks_info(&sid, &name)).await
}

/// Bytes: `u32 level, u32 start, u32 n`, then `min[n]`, `max[n]`, `rms[n]` as f32 (little endian).
#[tauri::command]
async fn peaks_tile(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    level: usize,
    from: i64,
    to: i64,
) -> Result<Response, AppError> {
    blocking(&svc, "peaks_tile", move |s| s.peaks_tile(&sid, &name, level, from, to))
        .await
        .map(Response::new)
}

/// Bytes: `u32 start, u32 sample rate`, then the samples as f32 (little endian).
#[tauri::command]
async fn samples_window(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    start: i64,
    end: i64,
) -> Result<Response, AppError> {
    blocking(&svc, "samples_window", move |s| {
        s.samples_window(&sid, &name, start, end)
    })
    .await
    .map(Response::new)
}

/// P4-40e: the Layer view's waveform of the one-shot. Same bytes as `samples_window`.
#[tauri::command(rename_all = "snake_case")]
async fn track_layer_wave(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    start: Option<i64>,
    end: Option<i64>,
) -> Result<Response, AppError> {
    blocking(&svc, "track_layer_wave", move |s| {
        s.track_layer_wave(&sid, &name, start.unwrap_or(0), end.unwrap_or(i64::MAX))
    })
    .await
    .map(Response::new)
}

/// P4-40e: the Layer view's waveform of the sampler calibration bounce. Same bytes as
/// `samples_window`.
#[tauri::command(rename_all = "snake_case")]
async fn track_sampler_wave(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    start: Option<i64>,
    end: Option<i64>,
) -> Result<Response, AppError> {
    blocking(&svc, "track_sampler_wave", move |s| {
        s.track_sampler_wave(&sid, &name, start.unwrap_or(0), end.unwrap_or(i64::MAX))
    })
    .await
    .map(Response::new)
}

/// Starts the job and returns at once; the channel `progress` gets `{sid, job}` after every
/// change of the job (the page passes a channel; it may ignore the messages and poll instead).
#[tauri::command(rename_all = "snake_case")]
fn analyze_start(
    svc: State<'_, Service>,
    sid: String,
    tracks: Option<Vec<String>>,
    params: Option<Map<String, Value>>,
    detector: Option<String>,
    keep_edited: Option<bool>,
    progress: Channel<Value>,
) -> Result<Value, AppError> {
    let progress: Progress = Arc::new(move |v: Value| {
        let _ = progress.send(v);
    });
    svc.analyze_start_keep(
        &sid,
        tracks,
        params.as_ref(),
        detector.as_deref(),
        Some(progress),
        keep_edited.unwrap_or(drumspike_app::command_policy::PAGE_KEEP_EDITED),
    )
}

#[tauri::command]
fn analyze_status(svc: State<'_, Service>, sid: String) -> Value {
    svc.analyze_status(&sid)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // the wrapper takes the arguments of the command (api.rs row)
async fn export_midi(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    note: Option<i64>,
    events: Option<Vec<Value>>,
    ppq: Option<i64>,
    tempo: Option<f64>,
    dest: Option<String>,
    compensate: Option<bool>,
) -> Result<Value, AppError> {
    blocking(&svc, "export_midi", move |s| {
        // P4-5: no `note` or `events` in the call: the document's (the page always sends both)
        // P4-7c: `compensate` (default true) moves the notes of a track with a sampler calibration
        s.export_midi_stored_with(
            &sid,
            &name,
            note,
            events.as_deref(),
            ppq,
            tempo,
            dest.as_deref(),
            compensate.unwrap_or(drumspike_app::command_policy::COMPENSATE_DEFAULT),
        )
    })
    .await
}

/// P3-3c: `Exports/<name>_spike.wav` for the hits of the call (or of the document).
#[tauri::command]
async fn export_spikes(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    events: Option<Vec<Value>>,
    dest: Option<String>,
) -> Result<Value, AppError> {
    blocking(&svc, "export_spikes", move |s| {
        s.export_spikes_to(&sid, &name, events.as_deref(), dest.as_deref())
    })
    .await
}

/// P3-3c: MIDI and spike files of every track. Runs on the async pool; the channel gets
/// `{sid, export: {state, done, total, track}}` after each track.
#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)] // the wrapper takes the arguments of the command (api.rs row)
async fn export_all(
    svc: State<'_, Service>,
    sid: String,
    dest: Option<String>,
    midi_mode: Option<String>,
    tracks: Option<Vec<String>>,
    formats: Option<Vec<String>>,
    compensate: Option<bool>,
    progress: Channel<Value>,
) -> Result<Value, AppError> {
    let progress: Progress = Arc::new(move |v: Value| {
        let _ = progress.send(v);
    });
    // P4-30: the options of the Export dialog; the same parser as the bridge and the command line
    let opts = drumspike_app::export::ExportOpts::from_args(&serde_json::json!({
        "dest": dest, "midi_mode": midi_mode, "tracks": tracks, "formats": formats, "compensate": compensate }))?;
    blocking(&svc, "export_all", move |s| {
        s.export_all_opts(&sid, &opts, Some(progress))
    })
    .await
}

/// P3-3c: route 34, the times stay and `v` is computed again.
#[tauri::command]
async fn velocities_recompute(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    events: Option<Vec<Value>>,
) -> Result<Value, AppError> {
    blocking(&svc, "velocities_recompute", move |s| {
        s.velocities_recompute(&sid, &name, events.as_deref())
    })
    .await
}

/// Starts the transport (opens it when needed), plays from `position` seconds and returns when
/// the first block plays. The channel `progress` gets the playhead messages (on a change, else 4 Hz).
/// P3-20b: `speed` 0.5 plays at half speed (one octave down); 1 or none at normal speed.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // the wrapper takes the arguments of the command (api.rs row)
async fn transport_play(
    svc: State<'_, Service>,
    sid: String,
    position: Option<f64>,
    events: Option<Value>,
    speed: Option<f64>,
    start: Option<f64>,
    end: Option<f64>,
    r#loop: Option<bool>,
    tracks: Option<Value>,
    shot: Option<String>,
    progress: Channel<Value>,
) -> Result<Value, AppError> {
    let progress: Progress = Arc::new(move |v: Value| {
        let _ = progress.send(v);
    });
    blocking(&svc, "transport_play", move |s| {
        let shot_json = shot.map(Value::String);
        let win = drumspike_app::command_policy::transport_args::play_window(
            start,
            end,
            r#loop,
            tracks.as_ref(),
            shot_json.as_ref(),
        )?;
        // P4-40d: an audition (start, end, loop, tracks) sends no playhead messages (the service drops the channel)
        s.transport_play_window(&sid, position, events.as_ref(), Some(progress), speed, win)
    })
    .await
}

#[tauri::command]
async fn transport_stop(svc: State<'_, Service>, close: Option<bool>) -> Result<Value, AppError> {
    blocking(&svc, "transport_stop", move |s| {
        Ok(s.transport_stop(close.unwrap_or(drumspike_app::command_policy::CLOSE_DEFAULT)))
    })
    .await
}

/// P3-15c: the seek waits (up to 100 ms) for the callback to take it, without the host lock and
/// on the blocking pool.
#[tauri::command]
async fn transport_seek(svc: State<'_, Service>, position: f64) -> Result<Value, AppError> {
    blocking(&svc, "transport_seek", move |s| s.transport_seek(position)).await
}

#[tauri::command]
async fn transport_mix(svc: State<'_, Service>, mute: Option<Value>, solo: Option<Value>) -> Result<Value, AppError> {
    blocking(&svc, "transport_mix", move |s| {
        s.transport_mix(&mute.unwrap_or(Value::Null), &solo.unwrap_or(Value::Null))
    })
    .await
}

#[tauri::command]
async fn transport_clicks(
    svc: State<'_, Service>,
    enabled: Option<bool>,
    volume: Option<f64>,
    pitches: Option<Value>,
) -> Result<Value, AppError> {
    blocking(&svc, "transport_clicks", move |s| {
        s.transport_clicks(enabled, volume, pitches.as_ref())
    })
    .await
}

/// On the blocking pool: the host lock can be held while a transport opens its files.
#[tauri::command]
async fn transport_status(svc: State<'_, Service>) -> Result<Value, AppError> {
    blocking(&svc, "transport_status", |s| Ok(s.transport_status())).await
}

/// P3-16: a track's playback volume (dB) and pan; kept in the project.
#[tauri::command(rename_all = "snake_case")]
async fn track_set_mix(
    svc: State<'_, Service>,
    sid: String,
    name: String,
    volume_db: Option<f64>,
    pan: Option<f64>,
) -> Result<Value, AppError> {
    blocking(&svc, "track_set_mix", move |s| {
        s.track_set_mix(&sid, &name, volume_db, pan)
    })
    .await
}

/// P3-16: the master volume (dB); kept in the app settings.
#[tauri::command(rename_all = "snake_case")]
async fn transport_master(svc: State<'_, Service>, volume_db: f64) -> Result<Value, AppError> {
    blocking(&svc, "transport_master", move |s| s.transport_master(volume_db)).await
}

/// P3-16: the output devices, the choice and the device in use (a CoreAudio query: blocking pool).
#[tauri::command]
async fn outputs_list(svc: State<'_, Service>) -> Result<Value, AppError> {
    blocking(&svc, "outputs_list", |s| Ok(s.outputs_list())).await
}

/// P3-16: choose the output device by name (`null`: the system default); kept in the app settings.
#[tauri::command]
async fn output_select(svc: State<'_, Service>, device: Option<String>) -> Result<Value, AppError> {
    blocking(&svc, "output_select", move |s| s.output_select(device.as_deref())).await
}

/// Cold-start probe. Only the page script that `run` injects when `DRUMSPIKE_STARTUP_LOG` is
/// set calls it. Without `info`: print the ms from process start to the first animation
/// frame; with `DRUMSPIKE_STARTUP_LOG=1` the process exits then. With `info`: print the
/// `app_info` answer that the page got through the global `window.__TAURI__`, then exit.
#[tauri::command]
fn startup_mark(info: Option<String>) {
    let Some(mode) = std::env::var_os("DRUMSPIKE_STARTUP_LOG") else {
        return;
    };
    match info {
        None => {
            let ms = START.get().map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
            println!("first_js_frame_ms={ms:.1}");
            if mode != "info" {
                std::process::exit(0);
            }
        }
        Some(json) => {
            println!("app_info={json}");
            std::process::exit(0);
        }
    }
}

/// Self-test (P3-7a, debug builds only): ms since the start of the process.
#[cfg(debug_assertions)]
#[tauri::command]
fn selftest_clock() -> f64 {
    START.get().map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0)
}

/// Self-test (P3-6b, debug builds only): footprint of the WebKit helper processes of this app
/// (WebContent, GPU), as `selftest::memory` describes.
#[cfg(debug_assertions)]
#[tauri::command]
async fn selftest_memory() -> Value {
    tauri::async_runtime::spawn_blocking(selftest::memory)
        .await
        .unwrap_or(Value::Null)
}

/// Self-test (P3-7a, debug builds only): the page sends the finished report; the process writes
/// it to the `--out` file and exits with 0 (pass) or 1. With `via_quit` (P3-4b) the app ends through
/// the real quit path (`window::quit_with`, the gate and `RunEvent::ExitRequested` with a code)
/// instead of `process::exit`.
#[cfg(debug_assertions)]
#[tauri::command(rename_all = "snake_case")]
fn selftest_report(app: tauri::AppHandle, report: Value, via_quit: Option<bool>) {
    let ms = selftest_clock();
    let code = match std::env::args().collect::<Vec<_>>().get(1..).map(selftest::parse_args) {
        Some(Ok(Some(req))) => match selftest::write_report(&req.out, report, ms) {
            Ok(true) => 0,
            Ok(false) => 1,
            Err(e) => {
                eprintln!("self-test: {e}");
                2
            }
        },
        _ => 2,
    };
    if via_quit == Some(true) {
        window::quit_with(&app, code);
    } else {
        drumspike_app::trace::finish();
        std::process::exit(code);
    }
}

/// Runs in the page before its own scripts. `info` mode asks `app_info` through the
/// injected global `window.__TAURI__`.
const STARTUP_SCRIPT: &str = "requestAnimationFrame(() => { \
    if (/splash\\.html$/.test(location.pathname)) return; \
    const i = window.__TAURI_INTERNALS__.invoke; \
    i('startup_mark', {info: null}).then(() => window.__TAURI__.core.invoke('app_info')) \
     .then(r => i('startup_mark', {info: JSON.stringify(r)})); });";

/// The command handler of a release build: no self-test command.
#[cfg(not(debug_assertions))]
fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        app_info,
        about::about_info,
        about::about_third_party,
        about::open_link,
        startup_mark,
        transport_play,
        transport_stop,
        transport_seek,
        transport_mix,
        transport_clicks,
        transport_status,
        track_set_mix,
        transport_master,
        outputs_list,
        output_select,
        project_open,
        project_get,
        project_list,
        tracks_get,
        events_get,
        events_set,
        peaks_info,
        peaks_tile,
        samples_window,
        analyze_start,
        analyze_status,
        export_midi,
        export_spikes,
        export_all,
        velocities_recompute,
        lifecycle::project_new,
        lifecycle::project_save,
        lifecycle::project_clean_check,
        lifecycle::project_save_as,
        lifecycle::project_close,
        lifecycle::project_rename,
        lifecycle::tracks_reorder,
        lifecycle::track_set_note,
        lifecycle::track_set_role,
        lifecycle::track_rename,
        lifecycle::track_set_follow,
        lifecycle::app_settings_get,
        lifecycle::app_settings_set,
        lifecycle::project_recent,
        lifecycle::project_recent_clear,
        lifecycle::peaks_build_start,
        lifecycle::peaks_build_status,
        shell::shell_sync,
        shell::open_paths_take,
        shell::quit_app,
        splash::app_ready,
        splash::splash_painted,
        splash::splash_dismiss,
        shell::export_existing,
        shell::export_dest_set,
        shell::export_markers_set,
        shell::export_last_get,
        shell::export_last_set,
        shell::export_drag_prepare,
        shell::export_drag_take,
        shell::reveal_path,
        import::legacy_list,
        import::project_import,
        tracks::tracks_scan,
        tracks::tracks_add,
        tracks::tracks_remove,
        tracks::tracks_restore,
        tracks::kit_presets_list,
        tracks::kit_preset_save,
        tracks::kit_preset_load,
        jobs::project_consolidate,
        jobs::job_status,
        jobs::job_cancel,
        jobs::project_relink,
        jobs::relink_search,
        histogram::track_histograms,
        histogram::track_repick,
        histogram::track_velocity_set,
        histogram::track_params_set,
        edit::events_edit,
        edit::track_shift_set,
        learn::track_learn_mark,
        learn::track_learn_remove,
        learn::track_learn_forget_at,
        learn::track_learn_flip,
        learn::track_learn_forget,
        learn::track_learn_set,
        learn::track_learn_scores,
        tempo::tempo_map_get,
        tempo::tempo_map_set,
        tempo::tempo_map_import_midi,
        tempo::tempo_map_detect,
        tempo::tempo_map_refit_bar1,
        markers::markers_get,
        markers::markers_set,
        rowheights::row_heights_get,
        rowheights::row_heights_set,
        histogram::track_confidence,
        layer::track_layer_set,
        layer::track_layer_get,
        layer::track_layer_align,
        track_layer_wave,
        track_sampler_wave,
        layer::export_layer,
        layer::sampler_calibration_midi,
        layer::track_sampler_get,
        layer::track_sampler_set
    ]
}

/// The command handler of a debug build: the same commands and the two self-test commands.
#[cfg(debug_assertions)]
fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        app_info,
        about::about_info,
        about::about_third_party,
        about::open_link,
        startup_mark,
        transport_play,
        transport_stop,
        transport_seek,
        transport_mix,
        transport_clicks,
        transport_status,
        track_set_mix,
        transport_master,
        outputs_list,
        output_select,
        selftest_clock,
        selftest_memory,
        selftest_report,
        project_open,
        project_get,
        project_list,
        tracks_get,
        events_get,
        events_set,
        peaks_info,
        peaks_tile,
        samples_window,
        analyze_start,
        analyze_status,
        export_midi,
        export_spikes,
        export_all,
        velocities_recompute,
        lifecycle::project_new,
        lifecycle::project_save,
        lifecycle::project_clean_check,
        lifecycle::project_save_as,
        lifecycle::project_close,
        lifecycle::project_rename,
        lifecycle::tracks_reorder,
        lifecycle::track_set_note,
        lifecycle::track_set_role,
        lifecycle::track_rename,
        lifecycle::track_set_follow,
        lifecycle::app_settings_get,
        lifecycle::app_settings_set,
        lifecycle::project_recent,
        lifecycle::project_recent_clear,
        lifecycle::peaks_build_start,
        lifecycle::peaks_build_status,
        shell::shell_sync,
        shell::open_paths_take,
        shell::quit_app,
        splash::app_ready,
        splash::splash_painted,
        splash::splash_dismiss,
        shell::export_existing,
        shell::export_dest_set,
        shell::export_markers_set,
        shell::export_last_get,
        shell::export_last_set,
        shell::export_drag_prepare,
        shell::export_drag_take,
        shell::reveal_path,
        splash::splash_report,
        shell::selftest_menu,
        shell::selftest_menu_report,
        shell::selftest_open_paths,
        shell::selftest_close_window,
        shell::selftest_ns_menu,
        shell::selftest_menu_key,
        shell::selftest_menu_log,
        shell::selftest_menu_probe,
        import::legacy_list,
        import::project_import,
        tracks::tracks_scan,
        tracks::tracks_add,
        tracks::tracks_remove,
        tracks::tracks_restore,
        tracks::kit_presets_list,
        tracks::kit_preset_save,
        tracks::kit_preset_load,
        jobs::project_consolidate,
        jobs::job_status,
        jobs::job_cancel,
        jobs::project_relink,
        jobs::relink_search,
        histogram::track_histograms,
        histogram::track_repick,
        histogram::track_velocity_set,
        histogram::track_params_set,
        edit::events_edit,
        edit::track_shift_set,
        learn::track_learn_mark,
        learn::track_learn_remove,
        learn::track_learn_forget_at,
        learn::track_learn_flip,
        learn::track_learn_forget,
        learn::track_learn_set,
        learn::track_learn_scores,
        tempo::tempo_map_get,
        tempo::tempo_map_set,
        tempo::tempo_map_import_midi,
        tempo::tempo_map_detect,
        tempo::tempo_map_refit_bar1,
        markers::markers_get,
        markers::markers_set,
        rowheights::row_heights_get,
        rowheights::row_heights_set,
        histogram::track_confidence,
        layer::track_layer_set,
        layer::track_layer_get,
        layer::track_layer_align,
        track_layer_wave,
        track_sampler_wave,
        layer::export_layer,
        layer::sampler_calibration_midi,
        layer::track_sampler_get,
        layer::track_sampler_set
    ]
}

/// With `--self-test <scenario> --out <file>`: inject the driver and start the watchdog. A
/// scenario that cannot load writes a failure report and ends the process with 2.
#[cfg(debug_assertions)]
fn self_test_setup<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let req = match selftest::parse_args(&args) {
        Ok(Some(r)) => {
            selftest::snapshot_helpers(); // P3-6b: the WebKit helpers of other apps are not ours
            r
        }
        Ok(None) => return builder,
        Err(e) => {
            eprintln!("self-test: {e}");
            std::process::exit(2);
        }
    };
    match selftest::load_scenario(&req) {
        Ok(sc) => {
            let name = sc["name"].as_str().unwrap_or("?").to_string();
            selftest::FRONT_WINDOW.store(true, std::sync::atomic::Ordering::Relaxed);
            selftest::spawn_watchdog(&sc, req.out.clone(), name);
            builder.append_invoke_initialization_script(selftest::init_script(&sc))
        }
        Err(e) => {
            eprintln!("self-test: {e}");
            let _ = selftest::write_report(&req.out, selftest::failure_report(&req.scenario, &e), 0.0);
            std::process::exit(2);
        }
    }
}

/// P3-4a: the home folder (`recent.json`) is the app data folder, new projects go to the documents
/// folder, and `project-dirty` events reach the page through `AppHandle::emit`. `DRUMSPIKE_HOME` and
/// `DRUMSPIKE_PROJECTS` (set by `Service::new`) win over these defaults. P4-5: also names the app in the project lock.
fn lifecycle_setup(svc: &Service, app: &tauri::AppHandle) {
    svc.set_lock_client("app"); // P4-5: a project the app opens for writing is locked against the agent tools
    if svc.home().is_none()
        && let Ok(dir) = app.path().app_data_dir()
    {
        svc.set_home(dir);
    }
    if std::env::var_os("DRUMSPIKE_PROJECTS").is_none_or(|v| v.is_empty())
        && let Ok(dir) = app.path().document_dir()
    {
        svc.set_projects_dir(dir.join("Drumspike Projects"));
    }
    let handle = app.clone();
    svc.set_emitter(Arc::new(move |name: &str, payload: Value| {
        let _ = handle.emit(name, payload);
    }));
}

/// Build and run the app.
pub fn run() {
    START.set(Instant::now()).ok();
    // P3-15c: the trace (feature `trace`, `DRUMSPIKE_TRACE=<file>`) starts first, so its clock
    // and the self-test clock (`selftest_clock`) differ by the offset of its first event
    drumspike_app::trace::init_from_env(START.get().map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0));
    async_runtime_setup();
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_drag::init()) // P4-30: the native file drag of the Export dialog
        // managed before the event loop starts: `RunEvent::Opened` of a cold `open -a <app> <document>` can
        // arrive before `setup` has run, and its path must reach the queue
        .manage(menu::RecentState::default())
        .manage(open::OpenQueue::default())
        .manage(window::QuitGate::default())
        .manage(splash::SplashState::default())
        .menu(menu::build)
        .on_menu_event(|app, ev| {
            menu::dispatch(app, ev.id().as_ref());
        })
        .on_window_event(window::on_window_event)
        .append_invoke_initialization_script("window.__DS_NATIVE_MENU__ = true;")
        .setup(|app| {
            splash::open(app.handle());
            // #463: a self-test window that ends up occluded gets its page suspended by WebKit
            // (rAF and timers stop, the in-page guards cannot fire), so keep it front.
            // The self-test exists in debug builds only.
            #[cfg(debug_assertions)]
            if selftest::FRONT_WINDOW.load(std::sync::atomic::Ordering::Relaxed) {
                let h = app.handle().clone();
                let front = move || {
                    if let Some(w) = h.get_webview_window("main") {
                        let _ = w.set_focus();
                    }
                };
                front();
                std::thread::spawn(move || {
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        front();
                    }
                });
            }
            // #206: the menu is in place; apply the AppKit Shift convention (uppercase, no Shift flag)
            #[cfg(target_os = "macos")]
            menu::fix_shift_equivalents(app.handle());
            let svc = Service::new(ServiceConfig::from_env(Path::new("")));
            lifecycle_setup(&svc, app.handle());
            app.manage(svc);
            splash::status(app.handle(), 1, "Loading the editor\u{2026}");
            Ok(())
        })
        .invoke_handler(traced(handler()));
    if std::env::var_os("DRUMSPIKE_STARTUP_LOG").is_some() {
        builder = builder.append_invoke_initialization_script(STARTUP_SCRIPT);
    }
    #[cfg(debug_assertions)]
    {
        builder = self_test_setup(builder);
    }
    builder
        .build(tauri::generate_context!())
        .expect("error while building the Drum Spike app")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                if window::on_exit_requested(app, code) {
                    api.prevent_exit();
                }
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Opened { urls } => open::push(app, open::paths_of_urls(&urls)),
            tauri::RunEvent::Exit => drumspike_app::trace::finish(),
            _ => {}
        });
}

#[cfg(test)]
mod tests;
