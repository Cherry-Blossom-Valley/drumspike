//! The editor window of the plugin (M1b2, #538): the Drum Spike page (`static/`) in a `wry`
//! web view (WKWebView) as a child of the host's NSView, on the service of the ARA document.
//!
//! The design is in `docs/design/daw-plugin/m1b2-editor.md`. The short form:
//!
//! - `clap.gui` (cocoa, embedded). `set_parent` builds the web view in the host's view,
//!   `destroy` drops it. The web view is made, used and dropped on the host's main thread.
//! - The page comes from the bundle (`Contents/Resources/page`), found from the path the host
//!   gives to `clap_entry.init`, or from the address of this code. A scheme handler for
//!   `dspage://localhost/` serves the files; a `Content-Security-Policy` keeps every request
//!   on that scheme (no network).
//! - The shell script (`editor/shell.js`) gives `window.__TAURI__`. `invoke` is a `fetch` POST
//!   to `/__ds/invoke/<command>`. The handler runs on the main thread, hands the request to a
//!   thread of its own, runs `drumspike_app::call::run` there on the service of the document,
//!   and hops back to the main thread to answer the web view.
//! - Channel messages and events come by long poll on the same scheme.
//!
//! Every entry point that the host can call runs inside the panic guard.
#![allow(clippy::missing_safety_doc)]

use crate::service::PageHandle;
use crate::{guard, session, Plugin};
use clap_sys::ext::gui::{clap_plugin_gui, clap_window};
use clap_sys::plugin::clap_plugin;
use drumspike_app::reply::{Channels, Reply};
use drumspike_app::AppError;
use objc2::msg_send;
use objc2::runtime::AnyObject;
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use wry::http::{Request, Response};
use wry::WebViewExtMacOS;

const EDITOR_W: u32 = 1000;
const EDITOR_H: u32 = 640;
const SCHEME: &str = "dspage";
const PAGE_URL: &str = "dspage://localhost/index.html";
const API: &str = "/__ds/";
/// The most requests that run at once (each runs on a thread of its own).
const MAX_INFLIGHT: usize = 96;

/// The CSP of the page: every load stays on the page's own scheme (plus `blob:` and `data:`
/// for the page's workers and canvases). A request to another host is refused by the web view.
const CSP: &str = "default-src dspage: blob: data:; script-src dspage: blob: 'unsafe-inline' 'unsafe-eval'; \
style-src dspage: 'unsafe-inline'; img-src dspage: blob: data:; font-src dspage: data:; \
worker-src dspage: blob:; connect-src dspage: blob: data:; media-src dspage: blob: data:; \
object-src 'none'; base-uri dspage:; form-action 'none'";

// ---------------------------------------------------------------- counters of the run

pub static READY: AtomicUsize = AtomicUsize::new(0);
pub static OPENED: AtomicUsize = AtomicUsize::new(0);
pub static CLOSED: AtomicUsize = AtomicUsize::new(0);
/// A web view call that ran off the host's main thread (the check fails on more than 0).
pub static OFF_MAIN: AtomicUsize = AtomicUsize::new(0);
pub static MAIN_CHECKS: AtomicUsize = AtomicUsize::new(0);
pub static COMMANDS: AtomicUsize = AtomicUsize::new(0);
pub static COMMAND_ERRORS: AtomicUsize = AtomicUsize::new(0);
pub static PANICS: AtomicUsize = AtomicUsize::new(0);
pub static PEAK_INFLIGHT: AtomicUsize = AtomicUsize::new(0);
pub static QUITS: AtomicUsize = AtomicUsize::new(0);

#[link(name = "System")]
extern "C" {
    fn pthread_main_np() -> i32;
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dladdr(addr: *const c_void, info: *mut DlInfo) -> i32;
}

#[repr(C)]
struct DlInfo {
    dli_fname: *const c_char,
    dli_fbase: *mut c_void,
    dli_sname: *const c_char,
    dli_saddr: *mut c_void,
}

fn on_main_thread() -> bool {
    unsafe { pthread_main_np() == 1 }
}

/// Count one use of the web view and note when it is not on the main thread.
fn main_check(what: &'static str) {
    MAIN_CHECKS.fetch_add(1, Ordering::SeqCst);
    if !on_main_thread() {
        OFF_MAIN.fetch_add(1, Ordering::SeqCst);
        eprintln!("drumspike-ara: web view call off the main thread: {what}");
    }
}

extern "C" fn run_boxed(context: *mut c_void) {
    let work = unsafe { Box::from_raw(context as *mut Box<dyn FnOnce() + Send>) };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_err() {
        eprintln!("drumspike-ara: caught panic in a main-thread job");
    }
}

/// Run `work` on the host's main thread (the main dispatch queue, which every host with a
/// window serves).
fn on_main(work: Box<dyn FnOnce() + Send>) {
    let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(work);
    unsafe {
        dispatch_async_f(
            std::ptr::addr_of!(_dispatch_main_q),
            Box::into_raw(boxed) as *mut c_void,
            run_boxed,
        );
    }
}

// ---------------------------------------------------------------- the bundle

static BUNDLE_HINT: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The entry `init` gives the path of the bundle.
pub unsafe fn set_bundle_hint(path: *const c_char) {
    if path.is_null() {
        return;
    }
    let p = CStr::from_ptr(path).to_string_lossy().into_owned();
    if !p.is_empty() {
        *BUNDLE_HINT.lock().unwrap_or_else(|e| e.into_inner()) = Some(PathBuf::from(p));
    }
}

fn page_of(bundle: &Path) -> PathBuf {
    bundle.join("Contents").join("Resources").join("page")
}

fn has_page(dir: &Path) -> bool {
    dir.join("index.html").is_file()
}

/// The folder of the page and how it was found (`init` or `dladdr`).
pub fn page_dir() -> Option<(PathBuf, &'static str)> {
    if std::env::var("DS_ARA_EDITOR_FAULT").as_deref() == Ok("no-bundle") {
        return None;
    }
    if let Some(hint) = BUNDLE_HINT.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        let dir = page_of(&hint);
        if has_page(&dir) {
            return Some((dir, "init"));
        }
    }
    let mut info = DlInfo {
        dli_fname: std::ptr::null(),
        dli_fbase: std::ptr::null_mut(),
        dli_sname: std::ptr::null(),
        dli_saddr: std::ptr::null_mut(),
    };
    let here = page_dir as *const c_void;
    if unsafe { dladdr(here, &mut info) } != 0 && !info.dli_fname.is_null() {
        let binary = PathBuf::from(unsafe { CStr::from_ptr(info.dli_fname) }.to_string_lossy().into_owned());
        if let Some(bundle) = binary.ancestors().nth(3) {
            let dir = page_of(bundle);
            if has_page(&dir) {
                return Some((dir, "dladdr"));
            }
        }
    }
    None
}

// ---------------------------------------------------------------- what an editor needs

/// What the editor needs from the document it belongs to.
pub struct Backend {
    pub page: Arc<PageHandle>,
    /// Called after a command changed the project: the plugin reads the events again.
    pub changed: Arc<dyn Fn() + Send + Sync>,
    pub doc_id: String,
}

struct Shared {
    backend: Backend,
    channels: Arc<Channels>,
    closed: AtomicBool,
    inflight: AtomicUsize,
    page_dir: Option<PathBuf>,
    n: usize,
}

fn test_mode() -> Option<String> {
    std::env::var("DS_ARA_EDITOR_MODE").ok().filter(|s| !s.is_empty())
}

fn test_arg() -> String {
    std::env::var("DS_ARA_EDITOR_ARG").unwrap_or_default()
}

fn fault() -> String {
    std::env::var("DS_ARA_EDITOR_FAULT").unwrap_or_default()
}

// ---------------------------------------------------------------- the scheme handler

type Responder = wry::RequestAsyncResponder;
type Reply2 = Response<std::borrow::Cow<'static, [u8]>>;

fn respond(status: u16, mime: &str, body: Vec<u8>, csp: bool) -> Reply2 {
    let mut b = Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Cache-Control", "no-store");
    if csp {
        b = b.header("Content-Security-Policy", CSP);
    }
    b.body(std::borrow::Cow::Owned(body))
        .unwrap_or_else(|_| Response::new(std::borrow::Cow::Owned(Vec::new())))
}

fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The file of the page for a request path: plain components below the page folder only.
fn file_of(dir: &Path, request_path: &str) -> Option<PathBuf> {
    let rel = request_path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let mut out = dir.to_path_buf();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(part) => out.push(part),
            _ => return None,
        }
    }
    Some(out)
}

/// The handler of the scheme, on the main thread. A file of the page is answered here; a
/// command is handed to a thread.
fn handle(shared: &Arc<Shared>, request: Request<Vec<u8>>, responder: Responder) {
    main_check("scheme handler");
    let path = request.uri().path().to_string();
    if let Some(rest) = path.strip_prefix(API) {
        let query = request.uri().query().unwrap_or("").to_string();
        let rest = rest.to_string();
        let body = request.into_body();
        spawn_request(shared, rest, query, body, responder);
        return;
    }
    let reply = match shared.page_dir.as_ref().and_then(|d| file_of(d, &path)) {
        Some(f) => match std::fs::read(&f) {
            Ok(body) => {
                let html = f.extension().is_some_and(|e| e == "html");
                respond(200, mime_of(&path), body, html)
            }
            Err(_) => respond(
                404,
                "text/plain",
                format!("not in the bundle: {path}").into_bytes(),
                false,
            ),
        },
        None => respond(
            404,
            "text/plain",
            format!("not in the bundle: {path}").into_bytes(),
            false,
        ),
    };
    responder.respond(reply);
}

struct Answer {
    status: u16,
    mime: &'static str,
    body: Vec<u8>,
}

impl Answer {
    fn json(status: u16, v: &Value) -> Answer {
        Answer {
            status,
            mime: "application/json",
            body: serde_json::to_vec(v).unwrap_or_else(|_| b"null".to_vec()),
        }
    }

    fn error(e: &AppError) -> Answer {
        COMMAND_ERRORS.fetch_add(1, Ordering::SeqCst);
        Answer::json(e.code.clamp(400, 599), &json!({"code": e.code, "message": e.message}))
    }

    fn err(code: u16, message: impl Into<String>) -> Answer {
        Answer::error(&AppError {
            code,
            message: message.into(),
        })
    }
}

fn spawn_request(shared: &Arc<Shared>, rest: String, query: String, body: Vec<u8>, responder: Responder) {
    let now = shared.inflight.fetch_add(1, Ordering::SeqCst) + 1;
    PEAK_INFLIGHT.fetch_max(now, Ordering::SeqCst);
    if now > MAX_INFLIGHT || shared.closed.load(Ordering::SeqCst) {
        shared.inflight.fetch_sub(1, Ordering::SeqCst);
        responder.respond(respond(
            503,
            "application/json",
            br#"{"code":503,"message":"the editor is busy or closing"}"#.to_vec(),
            false,
        ));
        return;
    }
    let sh = Arc::clone(shared);
    let slot: Arc<Mutex<Option<Responder>>> = Arc::new(Mutex::new(Some(responder)));
    let slot2 = Arc::clone(&slot);
    let spawned = std::thread::Builder::new()
        .name("drumspike-ara-page".into())
        .spawn(move || {
            let run = || route(&sh, &rest, &query, &body);
            let answer = if std::env::var("DS_ARA_GUARD_FAULT").as_deref() == Ok("off") {
                // the fault run: no guard. A panic ends the thread, the responder is dropped and
                // the page never gets an answer.
                run()
            } else {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
                    Ok(a) => a,
                    Err(_) => {
                        PANICS.fetch_add(1, Ordering::SeqCst);
                        eprintln!("drumspike-ara: caught panic in a command of the page");
                        session().log(json!({"event": "page_panic_caught", "command": rest}).to_string());
                        Answer::err(
                            500,
                            "the command panicked inside the plugin; the plugin is still running",
                        )
                    }
                }
            };
            sh.inflight.fetch_sub(1, Ordering::SeqCst);
            let responder = slot2.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(responder) = responder {
                on_main(Box::new(move || {
                    main_check("answer to the web view");
                    responder.respond(respond(answer.status, answer.mime, answer.body, false));
                }));
            }
        });
    if spawned.is_err() {
        shared.inflight.fetch_sub(1, Ordering::SeqCst);
        if let Some(responder) = slot.lock().unwrap_or_else(|e| e.into_inner()).take() {
            responder.respond(respond(
                503,
                "application/json",
                br#"{"code":503,"message":"the plugin could not start a thread"}"#.to_vec(),
                false,
            ));
        }
    }
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
}

fn route(sh: &Arc<Shared>, rest: &str, query: &str, body: &[u8]) -> Answer {
    if let Some(cmd) = rest.strip_prefix("invoke/") {
        return invoke(sh, cmd, body);
    }
    let ms = query_value(query, "timeout_ms")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(2000)
        .min(5000);
    if let Some(id) = rest.strip_prefix("channel/") {
        let msgs = poll_channel(sh, id, std::time::Duration::from_millis(ms));
        return Answer::json(200, &Value::Array(msgs));
    }
    if rest == "events" {
        let log = &sh.backend.page.log;
        let after = query_value(query, "after")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or_else(|| log.head());
        return Answer::json(200, &log.poll(after, std::time::Duration::from_millis(ms)));
    }
    Answer::err(404, format!("no such endpoint: {rest}"))
}

/// A long poll of one channel in slices, so a closed editor ends it.
fn poll_channel(sh: &Arc<Shared>, id: &str, total: std::time::Duration) -> Vec<Value> {
    let end = std::time::Instant::now() + total;
    loop {
        if sh.closed.load(Ordering::SeqCst) {
            return Vec::new();
        }
        let left = end.saturating_duration_since(std::time::Instant::now());
        let msgs = sh.channels.poll(id, left.min(std::time::Duration::from_millis(250)));
        if !msgs.is_empty() || left.is_zero() {
            return msgs;
        }
    }
}

// ---------------------------------------------------------------- the commands

/// The commands that `call::run` leaves to the desktop shell (`SHELL_ONLY` of
/// `crates/app/src/call.rs`), plus the one command of the drag plugin. Each has an answer.
const SHELL_COMMANDS: [&str; 26] = [
    "about_info",
    "about_third_party",
    "open_link",
    "open_url",
    "startup_mark",
    "shell_sync",
    "shell_typing",
    "open_paths_take",
    "quit_app",
    "app_ready",
    "splash_painted",
    "splash_dismiss",
    "reveal_path",
    "selftest_clock",
    "selftest_memory",
    "selftest_report",
    "splash_report",
    "selftest_menu",
    "selftest_menu_report",
    "selftest_ns_menu",
    "selftest_menu_key",
    "selftest_menu_log",
    "selftest_menu_probe",
    "selftest_open_paths",
    "selftest_close_window",
    "plugin:drag|start_drag",
];

/// In a test run the calls that would reach the system (a browser, the Finder) are recorded.
fn open_system(arg: &str, reveal: bool) -> Result<(), String> {
    if test_mode().is_some() {
        session().log(json!({"event": "page_open", "reveal": reveal, "arg": arg}).to_string());
        return Ok(());
    }
    let mut c = std::process::Command::new("/usr/bin/open");
    if reveal {
        c.arg("-R");
    }
    c.arg(arg).spawn().map(|_| ()).map_err(|e| format!("open: {e}"))
}

/// The fixed addresses of the About dialog: the page passes an id, never an address.
fn link_address(id: &str) -> Option<&'static str> {
    match id {
        "website" => Some("https://drumspike.cherry-blossom-valley.com/"),
        "source" => Some("https://github.com/Cherry-Blossom-Valley/drumspike"),
        "cbv" => Some("https://cherry-blossom-valley.com/"),
        "warranty" => Some("https://github.com/Cherry-Blossom-Valley/drumspike#no-warranty"),
        _ => None,
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

fn shell_command(sh: &Arc<Shared>, cmd: &str, args: &Value) -> Option<Answer> {
    let text = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Some(match cmd {
        "shell_sync" => Answer::json(200, &json!(0)),
        "shell_typing" | "startup_mark" | "splash_painted" | "splash_dismiss" | "splash_report" => {
            Answer::json(200, &Value::Null)
        }
        "app_ready" => {
            // the page says that its first layout is done (the fault run never counts it)
            if fault() != "no-ready" {
                let n = READY.fetch_add(1, Ordering::SeqCst) + 1;
                session().log(
                    json!({"event": "page_ready", "n": n, "editor": sh.n, "doc": sh.backend.doc_id,
                        "ready_at_unix_ms": now_ms().to_string()})
                    .to_string(),
                );
                session().write_out();
            }
            Answer::json(200, &json!({"ok": true}))
        }
        "open_paths_take" => Answer::json(200, &json!({"projects": [], "others": []})),
        // the host owns the window: the plugin never ends the host process
        "quit_app" => {
            QUITS.fetch_add(1, Ordering::SeqCst);
            Answer::json(200, &json!({}))
        }
        "reveal_path" => {
            let p = text("path");
            if p.is_empty() || !Path::new(&p).is_absolute() {
                Answer::err(400, "reveal_path needs an absolute path")
            } else {
                match open_system(&p, true) {
                    Ok(()) => Answer::json(200, &json!({})),
                    Err(e) => Answer::err(500, e),
                }
            }
        }
        "about_info" => Answer::json(
            200,
            &json!({"name": "Drum Spike", "version": env!("CARGO_PKG_VERSION"),
                "build_sha": option_env!("DS_BUILD_SHA").unwrap_or("dev"),
                "core_version": drumspike_core::VERSION}),
        ),
        "about_third_party" => Answer::json(
            200,
            &json!(
                "The third-party list of the plugin bundle comes with the delivery of the plugin (M1c). \
Until then NOTICE of the repository and the licence table in plugins/ara/README.md name the parts."
            ),
        ),
        "open_link" => {
            let id = text("id");
            match link_address(&id) {
                Some(addr) => match open_system(addr, false) {
                    Ok(()) => Answer::json(200, &Value::Null),
                    Err(e) => Answer::err(500, e),
                },
                None => Answer::err(404, format!("link id {id} is not in the plugin")),
            }
        }
        "open_url" => {
            let url = text("url");
            if url.starts_with("https://") || url.starts_with("http://") {
                match open_system(&url, false) {
                    Ok(()) => Answer::json(200, &Value::Null),
                    Err(e) => Answer::err(500, e),
                }
            } else {
                Answer::err(404, format!("not an http(s) address: {url}"))
            }
        }
        "plugin:drag|start_drag" => Answer::err(501, "the file drag out of the plugin comes with part M1c"),
        c if c.starts_with("selftest_") => Answer::err(
            404,
            format!("{c} is a test command of the desktop app; the plugin has none"),
        ),
        _ => return None,
    })
}

/// The scenario `DS_ARA_EDITOR_SCENARIO=panic:<command>`: the command panics where `call::run`
/// would run, so the guard of the request thread is what stops it.
fn scenario_panics(cmd: &str) -> bool {
    std::env::var("DS_ARA_EDITOR_SCENARIO")
        .ok()
        .and_then(|v| v.strip_prefix("panic:").map(str::to_string))
        .is_some_and(|c| c == cmd)
}

/// One command of the page, on the thread of the request, never on the main thread.
fn invoke(sh: &Arc<Shared>, cmd: &str, body: &[u8]) -> Answer {
    COMMANDS.fetch_add(1, Ordering::SeqCst);
    let args: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => return Answer::err(400, format!("the arguments are no JSON: {e}")),
        }
    };
    if SHELL_COMMANDS.contains(&cmd) {
        if let Some(a) = shell_command(sh, cmd, &args) {
            return a;
        }
    }
    if on_main_thread() {
        OFF_MAIN.fetch_add(1, Ordering::SeqCst);
    }
    let Some(svc) = sh.backend.page.service() else {
        return Answer::err(503, "the document of this editor is closed");
    };
    if scenario_panics(cmd) {
        panic!("test scenario: the command {cmd} panics");
    }
    match drumspike_app::call::run(&svc, &sh.channels, cmd, &args) {
        None => Answer::err(404, format!("unknown command {cmd}")),
        Some(Err(e)) => Answer::error(&e),
        Some(Ok(reply)) => {
            changed_after(sh, cmd, &reply);
            match reply {
                Reply::Json(v) => Answer::json(200, &v),
                Reply::Bytes(b) => Answer {
                    status: 200,
                    mime: "application/octet-stream",
                    body: b,
                },
            }
        }
    }
}

/// After a command that changes the project, or a job that ended, the plugin reads the events
/// again (on its worker thread): the archive and the notes follow the page's edit.
fn changed_after(sh: &Arc<Shared>, cmd: &str, reply: &Reply) {
    let writes = drumspike_app::api::find(cmd).is_some_and(|c| c.writes_project());
    let ended = matches!(cmd, "analyze_status" | "job_status")
        && matches!(reply, Reply::Json(v) if v.get("state").and_then(Value::as_str) == Some("done"));
    if writes || ended {
        (sh.backend.changed)();
    }
}

// ---------------------------------------------------------------- the editor

struct ParentView(*mut c_void);
impl wry::raw_window_handle::HasWindowHandle for ParentView {
    fn window_handle(&self) -> Result<wry::raw_window_handle::WindowHandle<'_>, wry::raw_window_handle::HandleError> {
        let ptr = std::ptr::NonNull::new(self.0).ok_or(wry::raw_window_handle::HandleError::Unavailable)?;
        let handle = wry::raw_window_handle::AppKitWindowHandle::new(ptr);
        Ok(unsafe { wry::raw_window_handle::WindowHandle::borrow_raw(handle.into()) })
    }
}

/// The editor of one plugin instance: one web view as a child of the host's NSView.
pub struct Editor {
    webview: wry::WebView,
    shared: Arc<Shared>,
}

static NEXT_EDITOR: AtomicUsize = AtomicUsize::new(1);

impl Editor {
    fn build(nsview: *mut c_void, backend: Backend) -> Result<Editor, String> {
        main_check("build");
        if nsview.is_null() {
            return Err("the host passed no cocoa parent view".to_string());
        }
        let page = page_dir();
        let n = NEXT_EDITOR.fetch_add(1, Ordering::SeqCst);
        session().log(
            json!({"event": "editor_open", "editor": n, "doc": backend.doc_id,
                "page_dir": page.as_ref().map(|(d, _)| d.display().to_string()),
                "page_found_by": page.as_ref().map(|(_, how)| *how),
                "main_thread": on_main_thread()})
            .to_string(),
        );
        let shared = Arc::new(Shared {
            backend,
            channels: Arc::new(Channels::default()),
            closed: AtomicBool::new(false),
            inflight: AtomicUsize::new(0),
            page_dir: page.map(|(d, _)| d),
            n,
        });
        let mut init = String::new();
        let mode = test_mode();
        if let Some(m) = &mode {
            init.push_str(&format!(
                "window.__DS_TEST__={{mode:{m:?},arg:{:?},fault:{:?}}};\n",
                test_arg(),
                fault()
            ));
        }
        init.push_str(include_str!("editor/shell.js"));
        if mode.is_some() {
            init.push_str(include_str!("editor/driver.js"));
        }
        let parent = ParentView(nsview);
        let (w, h) = unsafe { view_size(nsview) };
        let bounds = wry::Rect {
            position: wry::dpi::Position::Logical(wry::dpi::LogicalPosition::new(0.0, 0.0)),
            size: wry::dpi::Size::Logical(wry::dpi::LogicalSize::new(w, h)),
        };
        let for_handler = Arc::clone(&shared);
        let for_ipc = Arc::clone(&shared);
        let id = format!("drumspike-ara-editor-{n}");
        let mut builder = wry::WebViewBuilder::new()
            .with_id(&id)
            .with_bounds(bounds)
            .with_url(PAGE_URL)
            // wry 0.57 cannot place a persistent data store in the plugin's cache folder (a
            // store with an identifier lives below ~/Library/WebKit), so the store is
            // non-persistent: nothing of the page is written outside the cache folder.
            .with_incognito(true)
            .with_asynchronous_custom_protocol(SCHEME.into(), move |_id, request, responder| {
                handle(&for_handler, request, responder);
            })
            .with_initialization_script(init)
            .with_navigation_handler(move |url: String| {
                main_check("navigation");
                let ok = url.starts_with("dspage://") || url == "about:blank";
                if !ok {
                    session().log(json!({"event": "page_navigation_refused", "url": url}).to_string());
                }
                ok
            })
            .with_new_window_req_handler(|url: String, _features| {
                session().log(json!({"event": "page_new_window_refused", "url": url}).to_string());
                wry::NewWindowResponse::Deny
            });
        if mode.is_some() {
            builder = builder.with_ipc_handler(move |request: Request<String>| {
                main_check("ipc");
                report(&for_ipc, request.into_body());
            });
        }
        let webview = builder
            .build_as_child(&parent)
            .map_err(|e| format!("the web view did not build: {e}"))?;
        unsafe {
            let v = webview.webview();
            // fill the parent on every resize of the host (wry sets only ViewMinYMargin)
            let mask: objc2_app_kit::NSAutoresizingMaskOptions =
                objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                    | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable;
            let _: () = msg_send![&*v, setAutoresizingMask: mask];
            take_keys(&v);
        }
        OPENED.fetch_add(1, Ordering::SeqCst);
        Ok(Editor { webview, shared })
    }

    /// Make the web view the first responder of its window. wry's `with_focused` does nothing
    /// on macOS, so without this call no key reaches the page.
    fn focus(&self) {
        main_check("focus");
        unsafe {
            let v = self.webview.webview();
            take_keys(&v);
        }
    }

    fn set_size(&self, w: u32, h: u32) {
        main_check("set_size");
        let _ = self.webview.set_bounds(wry::Rect {
            position: wry::dpi::Position::Physical(wry::dpi::PhysicalPosition::new(0, 0)),
            size: wry::dpi::Size::Physical(wry::dpi::PhysicalSize::new(w.max(320), h.max(240))),
        });
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        main_check("destroy");
        self.shared.closed.store(true, Ordering::SeqCst);
        CLOSED.fetch_add(1, Ordering::SeqCst);
        session().log(
            json!({"event": "editor_close", "editor": self.shared.n, "main_thread": on_main_thread(),
                "commands": COMMANDS.load(Ordering::SeqCst), "off_main": OFF_MAIN.load(Ordering::SeqCst),
                "main_checks": MAIN_CHECKS.load(Ordering::SeqCst), "panics": PANICS.load(Ordering::SeqCst),
                "peak_inflight": PEAK_INFLIGHT.load(Ordering::SeqCst)})
            .to_string(),
        );
    }
}

unsafe fn take_keys(view: &objc2::rc::Retained<wry::WryWebView>) {
    let win: *mut AnyObject = msg_send![&**view, window];
    if !win.is_null() {
        let _: bool = msg_send![win, makeFirstResponder: &**view];
    }
}

unsafe fn view_size(nsview: *mut c_void) -> (f64, f64) {
    let rect: objc2_foundation::NSRect = msg_send![nsview as *const AnyObject, bounds];
    (rect.size.width.max(320.0), rect.size.height.max(240.0))
}

/// One report of the test driver of the page: a row of the plugin's report.
fn report(sh: &Arc<Shared>, body: String) {
    let v: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({"kind": "text", "text": body}));
    let kind = v.get("kind").and_then(Value::as_str).unwrap_or("report").to_string();
    session().log(json!({"event": "page_report", "kind": kind, "editor": sh.n, "report": v}).to_string());
    session().write_out();
}

// ---------------------------------------------------------------- clap.gui

fn is_cocoa(api: *const c_char) -> bool {
    !api.is_null() && unsafe { CStr::from_ptr(api) }.to_bytes() == b"cocoa"
}

pub static PLUGIN_GUI: clap_plugin_gui = clap_plugin_gui {
    is_api_supported: Some(g_is_api_supported),
    get_preferred_api: Some(g_preferred_api),
    create: Some(g_create),
    destroy: Some(g_destroy),
    set_scale: Some(g_set_scale),
    get_size: Some(g_get_size),
    can_resize: Some(g_can_resize),
    get_resize_hints: None,
    adjust_size: Some(g_adjust_size),
    set_size: Some(g_set_size),
    set_parent: Some(g_set_parent),
    set_transient: None,
    suggest_title: Some(g_suggest_title),
    show: Some(g_show),
    hide: Some(g_hide),
};

unsafe extern "C" fn g_is_api_supported(_p: *const clap_plugin, api: *const c_char, floating: bool) -> bool {
    guard("gui.is_api_supported", false, move || !floating && is_cocoa(api))
}

unsafe extern "C" fn g_preferred_api(_p: *const clap_plugin, api: *mut *const c_char, floating: *mut bool) -> bool {
    guard("gui.get_preferred_api", false, move || {
        if api.is_null() || floating.is_null() {
            return false;
        }
        *api = clap_sys::ext::gui::CLAP_WINDOW_API_COCOA.as_ptr();
        *floating = false;
        true
    })
}

unsafe extern "C" fn g_create(_p: *const clap_plugin, api: *const c_char, floating: bool) -> bool {
    guard("gui.create", false, move || !floating && is_cocoa(api))
}

fn plugin_of<'a>(p: *const clap_plugin) -> Option<&'a Plugin> {
    if p.is_null() {
        return None;
    }
    unsafe { ((*p).plugin_data as *const Plugin).as_ref() }
}

/// The backend of a plugin instance: the document it is bound to (the ARA bind call), or a
/// service of its own when the host does not speak ARA.
fn backend_of(plugin: &Plugin) -> Backend {
    let bound = plugin.doc.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match bound {
        Some(doc) => crate::backend_of_doc(&doc),
        None => Backend {
            page: crate::service::unbound_page(&session().cache),
            changed: Arc::new(|| {}),
            doc_id: "unbound".to_string(),
        },
    }
}

unsafe extern "C" fn g_set_parent(p: *const clap_plugin, window: *const clap_window) -> bool {
    guard("gui.set_parent", false, move || {
        let Some(plugin) = plugin_of(p) else { return false };
        if window.is_null() {
            return false;
        }
        let w = &*window;
        if !is_cocoa(w.api) {
            return false;
        }
        plugin.editor.borrow_mut().take();
        match Editor::build(w.specific.cocoa, backend_of(plugin)) {
            Ok(ed) => {
                *plugin.editor.borrow_mut() = Some(ed);
                true
            }
            Err(e) => {
                eprintln!("drumspike-ara: {e}");
                session().log(json!({"event": "editor_failed", "error": e}).to_string());
                session().write_out();
                false
            }
        }
    })
}

unsafe extern "C" fn g_destroy(p: *const clap_plugin) {
    guard("gui.destroy", (), move || {
        if let Some(plugin) = plugin_of(p) {
            let old = plugin.editor.borrow_mut().take();
            drop(old);
            session().write_out();
        }
    })
}

unsafe extern "C" fn g_show(p: *const clap_plugin) -> bool {
    guard("gui.show", false, move || {
        let Some(plugin) = plugin_of(p) else { return false };
        let ed = plugin.editor.borrow();
        match ed.as_ref() {
            Some(e) => {
                e.focus();
                true
            }
            None => false,
        }
    })
}

unsafe extern "C" fn g_hide(_p: *const clap_plugin) -> bool {
    guard("gui.hide", false, move || true)
}

unsafe extern "C" fn g_set_scale(_p: *const clap_plugin, _scale: f64) -> bool {
    guard("gui.set_scale", false, move || true)
}

unsafe extern "C" fn g_get_size(p: *const clap_plugin, w: *mut u32, h: *mut u32) -> bool {
    guard("gui.get_size", false, move || {
        if w.is_null() || h.is_null() {
            return false;
        }
        let (cw, ch) = plugin_of(p).map_or((EDITOR_W, EDITOR_H), |pl| pl.gui_size.get());
        *w = cw;
        *h = ch;
        true
    })
}

unsafe extern "C" fn g_can_resize(_p: *const clap_plugin) -> bool {
    guard("gui.can_resize", false, move || true)
}

unsafe extern "C" fn g_adjust_size(_p: *const clap_plugin, w: *mut u32, h: *mut u32) -> bool {
    guard("gui.adjust_size", false, move || {
        if w.is_null() || h.is_null() {
            return false;
        }
        *w = (*w).max(320);
        *h = (*h).max(240);
        true
    })
}

unsafe extern "C" fn g_set_size(p: *const clap_plugin, w: u32, h: u32) -> bool {
    guard("gui.set_size", false, move || {
        let Some(plugin) = plugin_of(p) else { return false };
        plugin.gui_size.set((w.max(320), h.max(240)));
        if let Some(ed) = plugin.editor.borrow().as_ref() {
            ed.set_size(w, h);
        }
        true
    })
}

unsafe extern "C" fn g_suggest_title(_p: *const clap_plugin, _title: *const c_char) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_path_stays_below_the_page_folder() {
        let dir = Path::new("/bundle/page");
        assert_eq!(file_of(dir, "/app.js"), Some(PathBuf::from("/bundle/page/app.js")));
        assert_eq!(file_of(dir, "/"), Some(PathBuf::from("/bundle/page/index.html")));
        assert_eq!(file_of(dir, "/../secret"), None);
        assert_eq!(file_of(dir, "/a/../../secret"), None);
    }

    #[test]
    fn every_shell_command_of_the_desktop_has_an_answer() {
        for c in [
            "about_info",
            "about_third_party",
            "open_link",
            "open_url",
            "startup_mark",
            "shell_sync",
            "shell_typing",
            "open_paths_take",
            "quit_app",
            "app_ready",
            "splash_painted",
            "splash_dismiss",
            "reveal_path",
            "selftest_clock",
            "selftest_menu",
        ] {
            assert!(SHELL_COMMANDS.contains(&c), "{c}");
        }
        assert!(link_address("website").is_some());
        assert!(link_address("nope").is_none());
    }

    #[test]
    fn the_csp_keeps_every_load_on_the_page_scheme() {
        assert!(CSP.contains("default-src dspage:"));
        assert!(!CSP.contains("http"));
    }
}
