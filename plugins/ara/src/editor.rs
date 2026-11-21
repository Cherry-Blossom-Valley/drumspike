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
/// The commands of the page whose name starts with `transport_` (the app's own audio transport).
/// The host plays in a plugin: the check asserts 0 while the host plays (M1b5).
pub static TRANSPORT_COMMANDS: AtomicUsize = AtomicUsize::new(0);
pub static COMMAND_ERRORS: AtomicUsize = AtomicUsize::new(0);
/// M1b7: the calls of the page to the host's playback controller, by the thread that made them.
pub static PLAYBACK_ON_MAIN: AtomicUsize = AtomicUsize::new(0);
pub static PLAYBACK_OFF_MAIN: AtomicUsize = AtomicUsize::new(0);
pub static PANICS: AtomicUsize = AtomicUsize::new(0);
pub static PEAK_INFLIGHT: AtomicUsize = AtomicUsize::new(0);
pub static QUITS: AtomicUsize = AtomicUsize::new(0);

/// The threads that ran the progress of a job of the page, the request threads of its
/// `analyze_start`, and the main thread: the check asserts the first two never meet the third.
static PROGRESS_THREADS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static START_THREADS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static MAIN_THREAD: Mutex<Option<String>> = Mutex::new(None);

fn note_thread(list: &Mutex<Vec<String>>) {
    let me = format!("{:?}", std::thread::current().id());
    let mut l = list.lock().unwrap_or_else(|e| e.into_inner());
    if !l.contains(&me) && l.len() < 64 {
        l.push(me);
    }
}

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
    if crate::test_switch("DS_ARA_EDITOR_FAULT").as_deref() == Ok("no-bundle") {
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

/// Reads the events of the document's tracks after a command of the page (see `Backend::record`).
pub type RecordFn = Arc<dyn Fn(&drumspike_app::Service, &str) -> Result<bool, String> + Send + Sync>;

/// What the editor needs from the document it belongs to.
pub struct Backend {
    pub page: Arc<PageHandle>,
    /// Called after a command changed the project: the plugin reads the events again.
    pub changed: Arc<dyn Fn() + Send + Sync>,
    /// Called on the request thread, before the page gets the answer of a command that changed the
    /// project: the events and notes of the document follow at once (F2).
    pub record: RecordFn,
    /// The regions of the document's sources, `(source index, region)`: the page maps the
    /// position of the host with them (M1b5).
    pub regions: RegionsFn,
    /// A change counter of those regions: the page maps again when it grows.
    pub regions_gen: Arc<dyn Fn() -> u64 + Send + Sync>,
    /// What the host gives of the ARA playback controller (M1b7).
    pub playback_offer: Arc<dyn Fn() -> crate::playback::Offer + Send + Sync>,
    /// Makes calls to the playback controller of the host, and answers the number made. It must run
    /// on the host's main thread (the thread of the document controller).
    pub playback_run: PlaybackRunFn,
    /// The sources of a track (by name; empty name: all) and the longest of their lengths in seconds.
    pub track_info: TrackInfoFn,
    /// The name the host gave (`clap_host.name`).
    pub host_name: String,
    pub doc_id: String,
}

/// The source indexes of a track and their longest length in seconds.
pub type TrackInfoFn = Arc<dyn Fn(&str) -> (Vec<usize>, f64) + Send + Sync>;

/// Makes calls to the playback controller of the host; answers the number of calls made.
pub type PlaybackRunFn = Arc<dyn Fn(&[crate::playback::Call]) -> usize + Send + Sync>;

/// The playback regions of every source of the document, as the notes use them.
pub type RegionsFn = Arc<dyn Fn() -> Vec<(usize, crate::service::RegionRec)> + Send + Sync>;

struct Shared {
    backend: Backend,
    /// The queue the long polls read.
    channels: Arc<Channels>,
    /// The channels the commands write to: they note the thread of every message, then push
    /// into `channels`.
    run_channels: Arc<Channels>,
    closed: AtomicBool,
    inflight: AtomicUsize,
    page_dir: Option<PathBuf>,
    n: usize,
    /// The head of the document's event log when this editor opened: the page's first poll of
    /// events starts here, so an event announced before that poll is not lost.
    opened_head: u64,
    /// The pacing of the transport messages of this page (M1b5).
    poller: Mutex<crate::transport::Poller>,
    /// The row of the first transport message of this editor is written.
    transport_row: AtomicBool,
    /// M1b7: the requests of the page to the host wait here for the main thread.
    host_queue: crate::playback::Queue,
    /// The track the page has selected (the shell tells it). The follow and the locate use its regions.
    selected: Mutex<String>,
    /// Grows when the selected track changes: the poll maps the position again.
    sel_gen: std::sync::atomic::AtomicU64,
    /// The order of arrival of the requests of the page.
    req_seq: std::sync::atomic::AtomicU64,
    /// M1b7: the row of the first request of each kind (play, stop, locate) is written.
    request_row: [AtomicBool; 3],
    /// M1b7: the row of the first refused request of each kind is written.
    refused_row: [AtomicBool; 3],
}

fn test_mode() -> Option<String> {
    crate::test_switch("DS_ARA_EDITOR_MODE").ok().filter(|s| !s.is_empty())
}

fn test_arg() -> String {
    crate::test_switch("DS_ARA_EDITOR_ARG").unwrap_or_default()
}

fn fault() -> String {
    crate::test_switch("DS_ARA_EDITOR_FAULT").unwrap_or_default()
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

/// A request of the page to the host (M1b7) waits for the main thread, up to 2 s. It does not use
/// the budget of the commands of the app: a blocked main thread (menu tracking, a modal dialog)
/// must not make every other command of the page answer 503. The queue of host requests has its
/// own small limit ([`crate::playback::MAX_PENDING`]).
fn counts_inflight(rest: &str) -> bool {
    !rest.starts_with("invoke/plugin_host_")
}

/// True when a request may start, given the number in flight after counting it.
fn admit(counted: bool, now: usize) -> bool {
    !counted || now <= MAX_INFLIGHT
}

thread_local! {
    /// The order of arrival of the request this thread runs (set where the request thread starts).
    static REQ_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn spawn_request(shared: &Arc<Shared>, rest: String, query: String, body: Vec<u8>, responder: Responder) {
    let counted = counts_inflight(&rest);
    let seq = shared.req_seq.fetch_add(1, Ordering::SeqCst);
    let now = if counted {
        shared.inflight.fetch_add(1, Ordering::SeqCst) + 1
    } else {
        shared.inflight.load(Ordering::SeqCst)
    };
    PEAK_INFLIGHT.fetch_max(now, Ordering::SeqCst);
    if !admit(counted, now) || shared.closed.load(Ordering::SeqCst) {
        if counted {
            shared.inflight.fetch_sub(1, Ordering::SeqCst);
        }
        responder.respond(respond(
            503,
            "application/json",
            br#"{"code":503,"message":"the editor is busy or closing"}"#.to_vec(),
            false,
        ));
        return;
    }
    if fault() == "inline" {
        // the fault run: the command runs on the main thread, as a plugin that had no thread
        // for it would do
        REQ_SEQ.with(|c| c.set(seq));
        let answer = route(shared, &rest, &query, &body);
        if counted {
            shared.inflight.fetch_sub(1, Ordering::SeqCst);
        }
        responder.respond(respond(answer.status, answer.mime, answer.body, false));
        return;
    }
    let sh = Arc::clone(shared);
    let slot: Arc<Mutex<Option<Responder>>> = Arc::new(Mutex::new(Some(responder)));
    let slot2 = Arc::clone(&slot);
    let spawned = std::thread::Builder::new()
        .name("drumspike-ara-page".into())
        .spawn(move || {
            REQ_SEQ.with(|c| c.set(seq));
            let run = || route(&sh, &rest, &query, &body);
            let answer = if crate::test_switch("DS_ARA_GUARD_FAULT").as_deref() == Ok("off") {
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
            if counted {
                sh.inflight.fetch_sub(1, Ordering::SeqCst);
            }
            let responder = slot2.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(responder) = responder {
                on_main(Box::new(move || {
                    main_check("answer to the web view");
                    responder.respond(respond(answer.status, answer.mime, answer.body, false));
                }));
            }
        });
    if spawned.is_err() {
        if counted {
            shared.inflight.fetch_sub(1, Ordering::SeqCst);
        }
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

/// The command name of a request path (the page sends `encodeURIComponent`).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
}

fn transport_fault() -> String {
    crate::test_switch("DS_ARA_TRANSPORT_FAULT").unwrap_or_default()
}

/// The long poll of the host transport (M1b5): answers `{"messages": [..]}` with at most one
/// message, as soon as the pacer has one, or with none when the time is over. The thread sleeps
/// 8 ms between looks; it reads atomics and the region list, never the audio thread's lock.
fn poll_transport(sh: &Arc<Shared>, total: std::time::Duration) -> Vec<Value> {
    use crate::transport as tr;
    let end = std::time::Instant::now() + total;
    let identity = transport_fault() == "identity";
    let invent = transport_fault() == "invent";
    let keep_playing = transport_fault() == "keep-playing";
    loop {
        if sh.closed.load(Ordering::SeqCst) {
            return Vec::new();
        }
        let mut snap = tr::read();
        if snap.is_none() && invent {
            // the fault run: a plugin that assumes the host plays when it has seen no transport
            snap = Some(tr::Snap {
                generation: 0,
                pos_s: 0.0,
                pos_samples: 0,
                seen_us: tr::now_us(),
                playing: true,
                has_position: true,
                flags: 0,
                frames: 0,
                rate: 48000.0,
            });
        }
        let now = tr::now_us();
        let (msg, wait, stale_row, nopos_row, regions) = {
            let mut poller = sh.poller.lock().unwrap_or_else(|e| e.into_inner());
            poller.no_stop_fault = keep_playing;
            // a change of the regions or of the selected track maps the position again
            let gen = (sh.backend.regions_gen)().wrapping_add(sh.sel_gen.load(Ordering::SeqCst) << 40);
            let (msg, wait) = poller.look(snap, now, gen, &mut || {
                if identity {
                    Vec::new()
                } else if crate::test_switch("DS_ARA_PLAYBACK_FAULT").as_deref() == Ok("first-track") {
                    (sh.backend.regions)() // the fault run: the follow ignores the selected track
                } else {
                    sh.mapping_regions()
                }
            });
            (
                msg,
                wait,
                poller.take_stale_row(),
                poller.take_no_position_row(),
                poller.region_count(),
            )
        };
        if stale_row {
            // the evidence for the next run of a real host: it stopped its process calls while playing
            session().log(
                json!({"event": "transport_stale", "editor": sh.n, "last_position_seconds": snap.map(|s| s.pos_s)})
                    .to_string(),
            );
            session().write_out();
        }
        if nopos_row {
            // a process call with a transport but no seconds timeline: the page cannot follow
            session().log(
                json!({"event": "transport_no_position", "editor": sh.n, "flags": snap.map(|s| s.flags),
                    "playing": snap.map(|s| s.playing)})
                .to_string(),
            );
            session().write_out();
        }
        if let Some(m) = msg {
            if !sh.transport_row.swap(true, Ordering::SeqCst) {
                // the first transport of this page: where it came from and what the page got
                session().log(
                    json!({"event": "transport_page_first", "editor": sh.n, "source": "clap_process",
                        "host_position_seconds": m.song_s,
                        "host_position_samples": snap.map(|s| s.pos_samples),
                        "sample_rate": snap.map(|s| s.rate),
                        "playing": m.playing, "page_time": m.pos,
                        "mapped_by": format!("{:?}", m.mapped), "regions": regions})
                    .to_string(),
                );
                session().write_out();
            }
            return vec![
                json!({"epoch": m.epoch, "t_ns": m.t_ns, "pos": m.pos, "playing": m.playing,
                "why": format!("{:?}", m.why)}),
            ];
        }
        let left = end.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Vec::new();
        }
        std::thread::sleep(std::time::Duration::from_millis(wait).min(left));
        if sh.closed.load(Ordering::SeqCst) {
            return Vec::new(); // the editor closed during the wait
        }
    }
}

fn route(sh: &Arc<Shared>, rest: &str, query: &str, body: &[u8]) -> Answer {
    if let Some(cmd) = rest.strip_prefix("invoke/") {
        return invoke(sh, &percent_decode(cmd), body);
    }
    let ms = query_value(query, "timeout_ms")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(2000)
        .min(5000);
    if let Some(id) = rest.strip_prefix("channel/") {
        let msgs = poll_channel(sh, id, std::time::Duration::from_millis(ms));
        return Answer::json(200, &Value::Array(msgs));
    }
    if rest == "transport" {
        let msgs = poll_transport(sh, std::time::Duration::from_millis(ms));
        return Answer::json(200, &json!({"messages": msgs}));
    }
    if rest == "events" && fault() == "no-events" {
        // the fault run: the events of the service never reach the page
        std::thread::sleep(std::time::Duration::from_millis(ms.min(500)));
        return Answer::json(200, &json!({"seq": sh.backend.page.log.head(), "events": []}));
    }
    if rest == "events" {
        let log = &sh.backend.page.log;
        let after = match query_value(query, "after").as_deref() {
            // the first poll of a page: from the moment this editor opened
            Some("open") => sh.opened_head,
            Some(v) => v.parse::<u64>().unwrap_or_else(|_| log.head()),
            None => log.head(),
        };
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
const SHELL_COMMANDS: [&str; 34] = [
    "plugin_report",
    "plugin_host_track",
    "plugin_host_offer",
    "plugin_host_play",
    "plugin_host_stop",
    "plugin_host_locate",
    "window_call_next",
    "window_call_result",
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

/// F3: the host owns the track set of the document. The page cannot add, remove, rename or
/// reorder tracks: the plugin's list of tracks follows the ARA sources.
const REFUSED_TRACK_SET: [&str; 6] = [
    "tracks_add",
    "tracks_remove",
    "tracks_restore",
    "tracks_reorder",
    "track_rename",
    "tracks_scan",
];

/// F5: the host owns the project files and the document: no open, new, close, import, save
/// as, rename, consolidate or relink.
const REFUSED_PROJECT_FILES: [&str; 10] = [
    "project_open",
    "project_new",
    "project_close",
    "project_import",
    "project_save_as",
    "project_rename",
    "project_consolidate",
    "project_relink",
    "relink_search",
    "legacy_list",
];

/// F5: not in a plugin: the agent skill of the desktop app, the network post of a feedback
/// report, and the app's own audio transport and output devices (the host plays the audio).
const REFUSED_NOT_HERE: [&str; 11] = [
    "agent_skill_install",
    "agent_skill_status",
    "feedback_send",
    "transport_play",
    "transport_stop",
    "transport_seek",
    "transport_mix",
    "transport_clicks",
    "transport_master",
    "transport_status",
    "outputs_list",
];

/// More commands that are not in a plugin (kept apart for the array length).
const REFUSED_NOT_HERE_2: [&str; 1] = ["output_select"];

/// F5: the commands the page may run on the service of the document. Every one of them writes
/// only below the cache folder of the plugin (the document and the project folder of the
/// document, its `Exports/` and `Analysis/` folders, the home of the document's service), or
/// reads. The commands that name a folder or a file to write (`dest`, `path`) are refused
/// when they carry one: see [`writes_outside`].
const ALLOWED: [&str; 77] = [
    "app_info",
    "track_set_role",
    "track_set_mix",
    "project_get",
    "project_list",
    "tracks_get",
    "events_get",
    "events_set",
    "peaks_info",
    "peaks_tile",
    "samples_window",
    "analyze_start",
    "analyze_status",
    "reanalyze_selection",
    "export_midi",
    "export_spikes",
    "export_all",
    "velocities_recompute",
    "project_save",
    "project_clean_check",
    "track_set_note",
    "track_set_follow",
    "app_settings_get",
    "app_settings_set",
    "project_recent",
    "project_recent_clear",
    "peaks_build_start",
    "peaks_build_status",
    "export_existing",
    "export_dest_set",
    "export_markers_set",
    "export_compensate_set",
    "export_last_get",
    "export_last_set",
    "export_drag_prepare",
    "export_drag_take",
    "kit_presets_list",
    "kit_preset_save",
    "kit_preset_load",
    "job_status",
    "job_cancel",
    "track_histograms",
    "track_repick",
    "track_velocity_set",
    "track_params_set",
    "events_edit",
    "track_shift_set",
    "track_learn_mark",
    "track_learn_remove",
    "track_learn_forget_at",
    "track_learn_flip",
    "track_learn_forget",
    "track_learn_set",
    "track_learn_scores",
    "tempo_map_get",
    "tempo_map_set",
    "tempo_map_detect",
    "tempo_map_refit_bar1",
    "tempo_map_import_midi",
    "markers_get",
    "markers_set",
    "row_heights_get",
    "row_heights_set",
    "track_confidence",
    "training_status",
    "training_hand_placed",
    "track_layer_set",
    "track_layer_get",
    "track_layer_align",
    "track_layer_wave",
    "export_layer",
    "sampler_calibration_midi",
    "track_sampler_get",
    "track_sampler_set",
    "track_sampler_wave",
    // routes of the app without a registry row (the detector plug-ins of #387)
    "plugins_list",
    "plugin_accept",
];

/// The class of a command name for the editor.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Class {
    Allowed,
    RefusedTrackSet,
    RefusedFiles,
    RefusedNotHere,
    Shell,
    Unclassed,
}

fn class_of(cmd: &str) -> Class {
    let mut found = Vec::new();
    if ALLOWED.contains(&cmd) {
        found.push(Class::Allowed);
    }
    if REFUSED_TRACK_SET.contains(&cmd) {
        found.push(Class::RefusedTrackSet);
    }
    if REFUSED_PROJECT_FILES.contains(&cmd) {
        found.push(Class::RefusedFiles);
    }
    if REFUSED_NOT_HERE.contains(&cmd) || REFUSED_NOT_HERE_2.contains(&cmd) {
        found.push(Class::RefusedNotHere);
    }
    if SHELL_COMMANDS.contains(&cmd) {
        found.push(Class::Shell);
    }
    match found.as_slice() {
        [one] => *one,
        _ => Class::Unclassed,
    }
}

/// True when a command that writes files carries a folder or a file of the caller: the plugin
/// writes below its cache folder only, and the page has no file dialog yet (it answers cancel).
fn writes_outside(cmd: &str, args: &Value) -> bool {
    matches!(
        cmd,
        "export_all"
            | "export_layer"
            | "export_existing"
            | "export_dest_set"
            | "export_drag_prepare"
            | "export_midi"
            | "export_spikes"
            | "sampler_calibration_midi"
    ) && ["dest", "path"]
        .iter()
        .any(|k| args.get(k).and_then(Value::as_str).is_some_and(|s| !s.is_empty()))
}

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
    // wait for the child: no zombie stays in the host process
    c.arg(arg).status().map(|_| ()).map_err(|e| format!("open: {e}"))
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
        // the agent relay of the desktop window: the plugin has none, the page's loop ends
        "window_call_next" => Answer::json(200, &json!({"enabled": false, "call": null})),
        "window_call_result" => Answer::err(403, "the plugin has no agent relay"),
        // the shell script of the page reports a function of the page that is missing (S4)
        // M1b7: the page asks the host to play, stop or locate (the shell script's HT object)
        "plugin_host_offer" => host_offer(sh),
        "plugin_host_track" => {
            sh.select_track(args.get("track").and_then(Value::as_str).unwrap_or(""));
            Answer::json(200, &json!({"ok": true}))
        }
        "plugin_host_play" => host_request(sh, crate::playback::Kind::Play, args),
        "plugin_host_stop" => host_request(sh, crate::playback::Kind::Stop, args),
        "plugin_host_locate" => host_request(sh, crate::playback::Kind::Locate, args),
        "plugin_report" => {
            session().log(json!({"event": "page_shell_report", "what": args}).to_string());
            session().write_out();
            Answer::json(200, &Value::Null)
        }
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

/// How long the shell waits for a position of the host before it shows the last host position
/// again (a host that did not do what the page asked). `DS_ARA_PLAYBACK_FAULT=no-fallback`: never.
const CONFIRM_MS: u64 = 1000;
/// How long the request thread waits for the main thread to make the host call.
const MAIN_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// What the host offers, for the page: the page shows play, stop and locate as not available, with
/// the text, when the host gives no control (M1b7 gate line 5).
fn host_offer(sh: &Arc<Shared>) -> Answer {
    use crate::playback::{unavailable_text, Fault, Kind};
    let offer = (sh.backend.playback_offer)();
    let fault = Fault::read();
    let text = |k: Kind| {
        if offer.has(k) {
            Value::Null
        } else {
            json!(unavailable_text(&sh.backend.host_name, offer, k, fault))
        }
    };
    Answer::json(
        200,
        &json!({"host": sh.backend.host_name, "play": offer.start, "stop": offer.stop, "locate": offer.position,
            "play_text": text(Kind::Play), "stop_text": text(Kind::Stop), "locate_text": text(Kind::Locate),
            "fault": crate::playback::shell_fault()}),
    )
}

/// A request of the page to the host (M1b7): decide with `playback::plan`, then make the calls on
/// the host's main thread. The answer is always HTTP 200 (a refusal is not an error of the page):
/// `{ok, reason, song_s, confirm_ms}`.
fn host_request(sh: &Arc<Shared>, kind: crate::playback::Kind, args: &Value) -> Answer {
    use crate::playback::{plan, Fault, HostAt, Plan};
    let fault = Fault::read();
    let offer = (sh.backend.playback_offer)();
    if let Some(t) = args.get("track").and_then(Value::as_str) {
        sh.select_track(t); // the shell sends the track with the request: no race with its own track message
    }
    let track = sh.selected.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let track = track.as_str();
    // the regions of the selected track decide (stems of one length lie at the same source time);
    // no track, or a track with no region: the rule over all regions
    let (_seqs, len_s) = (sh.backend.track_info)(track);
    let page_raw = args.get("t").and_then(Value::as_f64);
    let page_s = page_raw.and_then(|t| crate::playback::bound_time(t, Some(len_s)));
    if page_raw.is_some() && page_s.is_none() && kind == crate::playback::Kind::Locate {
        return Answer::json(
            200,
            &json!({"ok": false, "reason": "that time is out of range", "confirm_ms": 0}),
        );
    }
    let snap = crate::transport::read().filter(|s| s.has_position);
    let host_at = snap.map(|s| HostAt {
        song_s: s.pos_s,
        playing: s.playing,
    });
    let regions = sh.mapping_regions();
    let confirm = if crate::test_switch("DS_ARA_PLAYBACK_FAULT").as_deref() == Ok("no-fallback") {
        0
    } else {
        CONFIRM_MS
    };
    let k = kind.index();
    match plan(kind, page_s, offer, &sh.backend.host_name, &regions, host_at, fault) {
        Plan::Refuse(text) => {
            if !sh.refused_row[k].swap(true, Ordering::SeqCst) {
                // the evidence for the next run of a real host: what it gave, and what the page was told
                session().log(
                    json!({"event": "playback_unavailable", "editor": sh.n, "kind": kind.name(), "host": sh.backend.host_name,
                        "offer": {"start": offer.start, "stop": offer.stop, "set_position": offer.position},
                        "regions": regions.len(), "page_time": page_s, "text": text})
                    .to_string(),
                );
                session().write_out();
            }
            if crate::test_switch("DS_ARA_PLAYBACK_FAULT").as_deref() == Ok("error-row") {
                // the fault run: a refusal is an error of the page (it writes an error row)
                return Answer::err(503, text);
            }
            Answer::json(200, &json!({"ok": false, "reason": text, "confirm_ms": 0}))
        }
        Plan::Send { calls, song_s, rule } => {
            let calls_made = calls.clone();
            use crate::playback::Outcome;
            let outcome = if crate::test_switch("DS_ARA_PLAYBACK_FAULT").as_deref() == Ok("direct") {
                // the fault run: the request thread calls the host itself (the thread the ARA rule keeps out)
                PLAYBACK_OFF_MAIN.fetch_add(1, Ordering::SeqCst);
                HostRun::Outcome(Outcome::Made((sh.backend.playback_run)(&calls)))
            } else {
                run_on_main(sh, kind, calls)
            };
            let made = match outcome {
                HostRun::Outcome(Outcome::Made(n)) => n,
                // a newer request passed this one: no error, the page does not react
                HostRun::Outcome(Outcome::Superseded) => {
                    return Answer::json(
                        200,
                        &json!({"ok": false, "superseded": true, "reason": "superseded", "confirm_ms": 0}),
                    )
                }
                HostRun::Outcome(Outcome::Closed) => {
                    return Answer::json(
                        200,
                        &json!({"ok": false, "reason": "the editor is closed", "confirm_ms": 0}),
                    )
                }
                HostRun::Busy => {
                    return Answer::json(
                        200,
                        &json!({"ok": false, "reason": "too many requests wait for the host", "confirm_ms": 0}),
                    )
                }
                HostRun::Timeout => {
                    return Answer::json(
                        200,
                        &json!({"ok": false, "reason": "the host did not take the request in time", "confirm_ms": 0}),
                    )
                }
            };
            if !sh.request_row[k].swap(true, Ordering::SeqCst) {
                let rate = snap.map(|s| s.rate);
                session().log(
                    json!({"event": "playback_request", "editor": sh.n, "kind": kind.name(), "host": sh.backend.host_name,
                        "offer": {"start": offer.start, "stop": offer.stop, "set_position": offer.position},
                        "page_time": page_s, "song_seconds": song_s, "song_samples": song_s.zip(rate).map(|(s, r)| (s * r).round() as i64),
                        "mapped_by": rule.map(|r| format!("{r:?}")), "regions": regions.len(),
                        "host_position_seconds": host_at.map(|h| h.song_s), "host_playing": host_at.map(|h| h.playing),
                        "calls": calls_made.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>(), "made": made,
                        "thread": "main"})
                    .to_string(),
                );
                session().write_out();
            }
            if made == 0 && !fault.skip {
                return Answer::json(
                    200,
                    &json!({"ok": false, "reason": "the host took no request: its document is closed", "confirm_ms": 0}),
                );
            }
            Answer::json(200, &json!({"ok": true, "song_s": song_s, "confirm_ms": confirm}))
        }
    }
}

impl Shared {
    /// The page selected a track (empty: none). A change makes the poll map again.
    fn select_track(&self, name: &str) {
        let mut cur = self.selected.lock().unwrap_or_else(|e| e.into_inner());
        if *cur != name {
            *cur = name.to_string();
            self.sel_gen.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The regions that map the page time, in both directions: those of the selected track, else all.
    fn mapping_regions(&self) -> Vec<(usize, crate::service::RegionRec)> {
        let all = (self.backend.regions)();
        let track = self.selected.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if track.is_empty() {
            return all;
        }
        crate::playback::regions_for(&all, &(self.backend.track_info)(&track).0)
    }
}

/// What became of a request to the host's main thread, for the request thread.
enum HostRun {
    Outcome(crate::playback::Outcome),
    /// Too many requests wait: answered at once.
    Busy,
    /// The main thread did not take it in time.
    Timeout,
}

/// Hand the calls to the host's main thread through the queue of the editor and wait for the answer,
/// at most [`MAIN_WAIT`]. The queue keeps the order of arrival, drops a locate that a newer locate
/// passes, and makes no call after the editor closed. A job that the main thread meets after the
/// wait does nothing (the flag): the page was told "not taken".
fn run_on_main(sh: &Arc<Shared>, kind: crate::playback::Kind, calls: Vec<crate::playback::Call>) -> HostRun {
    use crate::playback::Job;
    let (tx, rx) = std::sync::mpsc::channel();
    let late = Arc::new(AtomicBool::new(false));
    let job = Job {
        seq: REQ_SEQ.with(|c| c.get()),
        kind,
        calls,
        reply: tx,
        late: Arc::clone(&late),
    };
    match sh.host_queue.push(job) {
        Err(_) => return HostRun::Busy,
        Ok(true) => {
            let sh2 = Arc::clone(sh);
            on_main(Box::new(move || {
                main_check("playback request");
                let run = Arc::clone(&sh2.backend.playback_run);
                let sh3 = Arc::clone(&sh2);
                sh2.host_queue
                    .drain(&move || sh3.closed.load(Ordering::SeqCst), &mut |calls| {
                        if on_main_thread() {
                            PLAYBACK_ON_MAIN.fetch_add(1, Ordering::SeqCst);
                        } else {
                            PLAYBACK_OFF_MAIN.fetch_add(1, Ordering::SeqCst);
                        }
                        run(calls)
                    });
            }));
        }
        Ok(false) => {}
    }
    match rx.recv_timeout(MAIN_WAIT) {
        Ok(o) => HostRun::Outcome(o),
        Err(_) => {
            late.store(true, Ordering::SeqCst);
            HostRun::Timeout
        }
    }
}

fn scenario() -> String {
    crate::test_switch("DS_ARA_EDITOR_SCENARIO").unwrap_or_default()
}

/// The scenario `DS_ARA_EDITOR_SCENARIO=panic:<command>`: the command panics where `call::run`
/// would run, so the guard of the request thread is what stops it.
fn scenario_panics(cmd: &str) -> bool {
    crate::test_switch("DS_ARA_EDITOR_SCENARIO")
        .ok()
        .and_then(|v| v.strip_prefix("panic:").map(str::to_string))
        .is_some_and(|c| c == cmd)
}

/// One command of the page, on the thread of the request, never on the main thread.
fn invoke(sh: &Arc<Shared>, cmd: &str, body: &[u8]) -> Answer {
    COMMANDS.fetch_add(1, Ordering::SeqCst);
    if cmd.starts_with("transport_") {
        TRANSPORT_COMMANDS.fetch_add(1, Ordering::SeqCst);
    }
    let args: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => return Answer::err(400, format!("the arguments are no JSON: {e}")),
        }
    };
    if fault() != "no-shell" {
        match class_of(cmd) {
            Class::Shell => {
                if let Some(a) = shell_command(sh, cmd, &args) {
                    return a;
                }
            }
            Class::RefusedTrackSet if fault() != "allow-track-set" => {
                return Answer::err(
                    403,
                    format!("the host owns the track set of this document: {cmd} is not available in the plugin"),
                )
            }
            Class::RefusedFiles => {
                return Answer::err(
                    403,
                    format!("the host owns the project files: {cmd} is not available in the plugin"),
                )
            }
            Class::RefusedNotHere => return Answer::err(403, format!("{cmd} is not in the plugin")),
            // a name in no list at all (a route of the app without a registry row, a new command)
            // is refused, except the two named routes that `ALLOWED` lists
            Class::Unclassed => return Answer::err(403, format!("{cmd} is not classed for the plugin")),
            _ => {}
        }
    }
    if writes_outside(cmd, &args) {
        return Answer::err(
            403,
            format!("{cmd} writes only below the cache folder of the plugin: it takes no folder or file of its own"),
        );
    }
    if on_main_thread() {
        OFF_MAIN.fetch_add(1, Ordering::SeqCst);
    }
    if cmd == "analyze_start" {
        note_thread(&START_THREADS);
    }
    let Some(svc) = sh.backend.page.service() else {
        return Answer::err(503, "the document of this editor is closed");
    };
    if scenario_panics(cmd) {
        panic!("test scenario: the command {cmd} panics");
    }
    match drumspike_app::call::run(&svc, &sh.run_channels, cmd, &args) {
        None => Answer::err(404, format!("unknown command {cmd}")),
        Some(Err(e)) => Answer::error(&e),
        Some(Ok(reply)) => {
            changed_after(sh, &svc, cmd, &args, &reply);
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

/// After a command that changes the project, or a job that ended: the events and notes of the
/// document follow at once, on this thread, before the page gets its answer (F2); the worker
/// saves the project later.
fn changed_after(sh: &Arc<Shared>, svc: &drumspike_app::Service, cmd: &str, args: &Value, reply: &Reply) {
    let writes = drumspike_app::api::find(cmd).is_some_and(|c| c.writes_project());
    let ended = matches!(cmd, "analyze_status" | "job_status")
        && matches!(reply, Reply::Json(v) if v.get("state").and_then(Value::as_str) == Some("done"));
    if !(writes || ended) || fault() == "no-sync" {
        return;
    }
    if fault() != "no-record" {
        if let Some(sid) = args.get("sid").and_then(Value::as_str) {
            if let Err(e) = (sh.backend.record)(svc, sid) {
                session().log(json!({"event": "page_record_failed", "command": cmd, "error": e}).to_string());
            }
        }
    }
    if scenario() == "sync-delay" {
        // the scenario: the worker hears about the change late (it is not tied to the record
        // above): a store of the host in that time must still carry the edit
        let changed = Arc::clone(&sh.backend.changed);
        let _ = std::thread::Builder::new()
            .name("drumspike-ara-sync-delay".into())
            .spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(12));
                changed();
            });
        return;
    }
    (sh.backend.changed)();
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
///
/// wry 0.57 leaks a web view on purpose when it is dropped (its `Drop` retains the WKWebView
/// and its content manager once more), about 1.2 MB of the host's memory each time. A DAW
/// opens and closes the editor many times, so the plugin makes ONE web view for the life of
/// the plugin instance: `destroy` detaches it from the host's view and sends it to a blank
/// page, `set_parent` puts it into the new view and loads the page again. The one leak that
/// is left is the one of the plugin instance's end.
pub struct Editor {
    webview: wry::WebView,
    /// What the handlers of the web view serve now: `None` while the editor is closed.
    current: Arc<Mutex<Option<Arc<Shared>>>>,
}

static NEXT_EDITOR: AtomicUsize = AtomicUsize::new(1);

impl Editor {
    fn build(nsview: *mut c_void, backend: Backend) -> Result<Editor, String> {
        main_check("build");
        if nsview.is_null() {
            return Err("the host passed no cocoa parent view".to_string());
        }
        let current: Arc<Mutex<Option<Arc<Shared>>>> = Arc::new(Mutex::new(None));
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
        let for_handler = Arc::clone(&current);
        let for_ipc = Arc::clone(&current);
        let id = format!("drumspike-ara-editor-{}", NEXT_EDITOR.load(Ordering::SeqCst));
        let mut builder = wry::WebViewBuilder::new()
            .with_id(&id)
            .with_bounds(bounds)
            .with_url("about:blank")
            // wry 0.57 cannot place a persistent data store in the plugin's cache folder (a
            // store with an identifier lives below ~/Library/WebKit), so the store is
            // non-persistent: nothing of the page is written outside the cache folder.
            .with_incognito(true)
            .with_asynchronous_custom_protocol(SCHEME.into(), move |_id, request, responder| {
                guard("scheme handler", (), || {
                    let shared = for_handler.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    match shared {
                        Some(shared) => handle(&shared, request, responder),
                        None => responder.respond(respond(503, "text/plain", b"the editor is closed".to_vec(), false)),
                    }
                })
            })
            .with_initialization_script(init)
            .with_navigation_handler(move |url: String| {
                guard("navigation", false, || {
                    main_check("navigation");
                    let ok = url.starts_with("dspage://") || url == "about:blank";
                    if !ok {
                        session().log(json!({"event": "page_navigation_refused", "url": url}).to_string());
                    }
                    ok
                })
            })
            .with_new_window_req_handler(|url: String, _features| {
                guard("new window", false, || {
                    session().log(json!({"event": "page_new_window_refused", "url": url}).to_string());
                    false
                });
                wry::NewWindowResponse::Deny
            });
        if mode.is_some() {
            builder = builder.with_ipc_handler(move |request: Request<String>| {
                guard("ipc", (), || {
                    main_check("ipc");
                    let shared = for_ipc.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    if let Some(shared) = shared {
                        report(&shared, request.into_body());
                    }
                })
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
            if fault() != "no-resize" {
                let _: () = msg_send![&*v, setAutoresizingMask: mask];
            }
        }
        let ed = Editor { webview, current };
        ed.open(backend, false)?;
        Ok(ed)
    }

    /// Open the editor in the view it is in: a new `Shared` for the document, the page loads.
    fn open(&self, backend: Backend, reload: bool) -> Result<(), String> {
        let page = page_dir();
        let n = NEXT_EDITOR.fetch_add(1, Ordering::SeqCst);
        session().log(
            json!({"event": "editor_open", "editor": n, "doc": backend.doc_id, "view_reused": reload,
                "page_dir": page.as_ref().map(|(d, _)| d.display().to_string()),
                "page_found_by": page.as_ref().map(|(_, how)| *how),
                "main_thread": on_main_thread()})
            .to_string(),
        );
        if on_main_thread() {
            *MAIN_THREAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("{:?}", std::thread::current().id()));
        }
        let queue = Arc::new(Channels::default());
        let sink_to = Arc::clone(&queue);
        let run_channels = Arc::new(Channels::with_sink(move |id: &str, m: Value| {
            note_thread(&PROGRESS_THREADS);
            sink_to.push(id, m);
        }));
        let opened_head = backend.page.log.head();
        let shared = Arc::new(Shared {
            backend,
            channels: queue,
            run_channels,
            closed: AtomicBool::new(false),
            inflight: AtomicUsize::new(0),
            host_queue: crate::playback::Queue::default(),
            selected: Mutex::new(String::new()),
            sel_gen: std::sync::atomic::AtomicU64::new(0),
            req_seq: std::sync::atomic::AtomicU64::new(0),
            page_dir: page.map(|(d, _)| d),
            n,
            opened_head,
            poller: Mutex::new(crate::transport::Poller::new(if transport_fault() == "flood" {
                0
            } else {
                crate::transport::TICK_US
            })),
            transport_row: AtomicBool::new(false),
            request_row: [AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)],
            refused_row: [AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)],
        });
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = Some(shared);
        self.webview
            .load_url(PAGE_URL)
            .map_err(|e| format!("the page did not load: {e}"))?;
        if fault() == "leak" {
            // the fault run: every editor leaves 4 MB behind
            let leaked = vec![1u8; 4 << 20];
            std::hint::black_box(&leaked);
            Box::leak(leaked.into_boxed_slice());
        }
        OPENED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// The host gives a new parent view: the web view moves into it and the page loads again.
    fn attach(&self, nsview: *mut c_void, backend: Backend) -> Result<(), String> {
        main_check("attach");
        if nsview.is_null() {
            return Err("the host passed no cocoa parent view".to_string());
        }
        unsafe {
            let v = self.webview.webview();
            let (w, h) = view_size(nsview);
            let _: () = msg_send![nsview as *const AnyObject, addSubview: &*v];
            let frame = objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(0.0, 0.0),
                objc2_foundation::NSSize::new(w, h),
            );
            let _: () = msg_send![&*v, setFrame: frame];
        }
        self.open(backend, true)
    }

    /// The editor closes: the page stops, the web view leaves the host's view and stays here.
    fn detach(&self) {
        main_check("detach");
        let shared = self.current.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(shared) = shared else { return };
        shared.closed.store(true, Ordering::SeqCst);
        let _ = self.webview.load_url("about:blank");
        unsafe {
            let v = self.webview.webview();
            let _: () = msg_send![&*v, removeFromSuperview];
        }
        CLOSED.fetch_add(1, Ordering::SeqCst);
        session().log(
            json!({"event": "editor_close", "editor": shared.n, "main_thread": on_main_thread(),
                "commands": COMMANDS.load(Ordering::SeqCst),
                "transport_commands": TRANSPORT_COMMANDS.load(Ordering::SeqCst), "off_main": OFF_MAIN.load(Ordering::SeqCst),
                "command_errors": COMMAND_ERRORS.load(Ordering::SeqCst),
                "playback_calls_main": PLAYBACK_ON_MAIN.load(Ordering::SeqCst),
                "playback_calls_request_thread": PLAYBACK_OFF_MAIN.load(Ordering::SeqCst),
                "main_checks": MAIN_CHECKS.load(Ordering::SeqCst), "panics": PANICS.load(Ordering::SeqCst),
                "peak_inflight": PEAK_INFLIGHT.load(Ordering::SeqCst),
                "main_thread_id": MAIN_THREAD.lock().unwrap_or_else(|e| e.into_inner()).clone(),
                "progress_threads": PROGRESS_THREADS.lock().unwrap_or_else(|e| e.into_inner()).clone(),
                "start_threads": START_THREADS.lock().unwrap_or_else(|e| e.into_inner()).clone()})
            .to_string(),
        );
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
        // the end of the plugin instance (the host's main thread)
        self.detach();
    }
}

unsafe fn take_keys(view: &objc2::rc::Retained<wry::WryWebView>) {
    if fault() == "no-focus" {
        return; // the fault run: nothing makes the web view the first responder
    }
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
        Some(doc) => {
            let mut b = crate::backend_of_doc(&doc);
            b.host_name = plugin.host_name.clone();
            b
        }
        None => Backend {
            page: crate::service::unbound_page(&session().cache),
            changed: Arc::new(|| {}),
            record: Arc::new(|_, _| Ok(false)),
            regions: Arc::new(Vec::new),
            regions_gen: Arc::new(|| 0),
            playback_offer: Arc::new(crate::playback::Offer::default),
            playback_run: Arc::new(|_| 0),
            track_info: Arc::new(|_| (Vec::new(), 0.0)),
            host_name: plugin.host_name.clone(),
            doc_id: "unbound".to_string(),
        },
    }
}

/// F7: a gui entry that touches the web view runs on the host's main thread only; anywhere else
/// it does nothing and answers false (the counter notes it, the check fails on it).
fn main_only(what: &'static str) -> bool {
    if on_main_thread() {
        return true;
    }
    OFF_MAIN.fetch_add(1, Ordering::SeqCst);
    eprintln!("drumspike-ara: {what} was called off the main thread and refused");
    false
}

unsafe extern "C" fn g_set_parent(p: *const clap_plugin, window: *const clap_window) -> bool {
    guard("gui.set_parent", false, move || {
        if !main_only("gui.set_parent") {
            return false;
        }
        let Some(plugin) = plugin_of(p) else { return false };
        if window.is_null() {
            return false;
        }
        let w = &*window;
        if !is_cocoa(w.api) {
            return false;
        }
        let nsview = w.specific.cocoa;
        // the cell is not borrowed while the web view is made or moved
        let existing = plugin.editor.borrow_mut().take();
        let made = match existing {
            Some(ed) => {
                ed.detach();
                let r = ed.attach(nsview, backend_of(plugin));
                *plugin.editor.borrow_mut() = Some(ed);
                r
            }
            None => Editor::build(nsview, backend_of(plugin)).map(|ed| {
                *plugin.editor.borrow_mut() = Some(ed);
            }),
        };
        match made {
            Ok(()) => true,
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
        if !main_only("gui.destroy") {
            // the web view stays attached until the host calls again on the main thread: say so
            session()
                .log(json!({"event": "gui_destroy_off_main_refused", "main_thread": on_main_thread()}).to_string());
            session().write_out();
            return;
        }
        if let Some(plugin) = plugin_of(p) {
            // the web view stays with the plugin instance (see `Editor`); it leaves the host's view
            if let Some(ed) = plugin.editor.borrow().as_ref() {
                ed.detach();
            }
            session().write_out();
        }
    })
}

unsafe extern "C" fn g_show(p: *const clap_plugin) -> bool {
    guard("gui.show", false, move || {
        if !main_only("gui.show") {
            return false;
        }
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
        let (cw, ch) = plugin_of(p).map_or((EDITOR_W, EDITOR_H), |pl| {
            let v = pl.gui_size.load(Ordering::SeqCst);
            ((v >> 32) as u32, (v & 0xffff_ffff) as u32)
        });
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
        if !main_only("gui.set_size") {
            return false;
        }
        let Some(plugin) = plugin_of(p) else { return false };
        plugin
            .gui_size
            .store((u64::from(w.max(320)) << 32) | u64::from(h.max(240)), Ordering::SeqCst);
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
            "plugin_host_offer",
            "plugin_host_track",
            "plugin_host_play",
            "plugin_host_stop",
            "plugin_host_locate",
            "selftest_clock",
            "selftest_menu",
        ] {
            assert!(SHELL_COMMANDS.contains(&c), "{c}");
        }
        assert!(link_address("website").is_some());
        assert!(link_address("nope").is_none());
    }

    /// F5: every command of the app (its registry, taken from the app crate) is in exactly one
    /// class of the editor. A new command of the app fails this test until it is classed.
    #[test]
    fn every_command_of_the_app_is_in_exactly_one_class() {
        let mut unclassed = Vec::new();
        for c in drumspike_app::api::REGISTRY {
            if class_of(c.name) == Class::Unclassed {
                unclassed.push(c.name);
            }
        }
        assert!(unclassed.is_empty(), "not classed for the editor: {unclassed:?}");
        // no name of the lists is a name the app does not have (a list that kept a removed command)
        for name in ALLOWED
            .iter()
            .chain(&REFUSED_TRACK_SET)
            .chain(&REFUSED_PROJECT_FILES)
            .chain(&REFUSED_NOT_HERE)
            .chain(&REFUSED_NOT_HERE_2)
            .chain(&SHELL_COMMANDS)
        {
            let known = drumspike_app::api::find(name).is_some()
                || [
                    "plugins_list",
                    "plugin_accept",
                    "plugin:drag|start_drag",
                    "plugin_report",
                    "plugin_host_offer",
                    "plugin_host_track",
                    "plugin_host_play",
                    "plugin_host_stop",
                    "plugin_host_locate",
                ]
                .contains(name);
            assert!(known, "{name} is not a command of the app");
        }
        assert_eq!(class_of("tracks_remove"), Class::RefusedTrackSet);
        assert_eq!(class_of("events_edit"), Class::Allowed);
        assert_eq!(class_of("app_ready"), Class::Shell);
        assert!(writes_outside("export_all", &json!({"dest": "/tmp/x"})));
        assert!(!writes_outside("export_all", &json!({"dest": null})));
    }

    #[test]
    fn requests_to_the_host_do_not_use_the_budget_of_the_app_commands() {
        assert!(!counts_inflight("invoke/plugin_host_locate"));
        assert!(!counts_inflight("invoke/plugin_host_offer"));
        assert!(counts_inflight("invoke/events_get"));
        assert!(counts_inflight("transport"));
        // a blocked main thread: 8 host requests wait and the queue refuses the rest at once, while
        // the app commands are admitted as long as their own budget holds
        let mut counted = 0usize;
        for _ in 0..200 {
            if counts_inflight("invoke/plugin_host_locate") {
                counted += 1;
            }
        }
        assert_eq!(counted, 0, "200 host requests use none of the 96 slots");
        assert!(admit(true, MAX_INFLIGHT));
        assert!(!admit(true, MAX_INFLIGHT + 1));
        assert!(
            admit(false, MAX_INFLIGHT + 50),
            "a host request is never refused by the app budget"
        );
    }

    #[test]
    fn a_command_name_of_the_path_is_decoded() {
        assert_eq!(percent_decode("plugin%3Adrag%7Cstart_drag"), "plugin:drag|start_drag");
        assert_eq!(percent_decode("events_get"), "events_get");
        assert_eq!(percent_decode("a%"), "a%");
    }

    #[test]
    fn the_csp_keeps_every_load_on_the_page_scheme() {
        assert!(CSP.contains("default-src dspage:"));
        assert!(!CSP.contains("http"));
    }
}
