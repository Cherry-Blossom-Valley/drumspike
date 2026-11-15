//! Drum Spike webview spike (S2, #505): the editor page of Drum Spike (`static/`) in the
//! window of a CLAP plugin through a `wry` web view, wrapped as VST3 by `clap-wrapper`.
//! Throwaway code.
//!
//! One chain: host -> CLAP (GUI extension, cocoa, child of the host's NSView) -> `wry`
//! -> WKWebView. The page loads from the plugin bundle over the `dspage://` custom URL
//! scheme (files of `<bundle>/Contents/Resources/page`, no network, no local server). A
//! stub transport injected at document start answers the page's backend calls from a
//! recorded fixture project (see src/page_stub.js); `static/` itself is read, not changed.
//!
//! The test switch is the environment (the test host sets it before it loads the plugin):
//!
//! - `DS_WEBVIEW_SPIKE_MODE`: `boot` | `measure` | `keys` | `focus` | `resize` | `no-ready`.
//! - `DS_WEBVIEW_SPIKE_OUT`: the folder for the report files. The page talks over wry's
//!   IPC (`window.ipc.postMessage`); the plugin writes one JSON file per message kind:
//!   `ready-<cycle>.json` (one per editor open), `measure.json`, `keys.json`, `focus.json`,
//!   `resize.json` and the console log `log-<cycle>.txt`.
//!
//! The editor is created per `set_parent` (the host calls create -> set_parent -> show)
//! and torn down per `destroy`. The open counter names the per-cycle ready files.
#![allow(non_upper_case_globals, dead_code)]

use clap_sys::entry::clap_plugin_entry;
use clap_sys::ext::gui::{clap_plugin_gui, clap_window, CLAP_EXT_GUI, CLAP_WINDOW_API_COCOA};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::version::CLAP_VERSION;
use objc2::msg_send;
use objc2::runtime::AnyObject;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use wry::WebViewExtMacOS;

macro_rules! cs {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

/// The boundary guard: a panic that reached the C ABI would abort the host. Caught here,
/// the entry point answers its default and the process lives.
fn guard<T: Default>(where_: &'static str, body: impl FnOnce() -> T + std::panic::UnwindSafe) -> T {
    match std::panic::catch_unwind(body) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("webview-spike: caught panic in {where_}");
            T::default()
        }
    }
}

static PAGE_DIR: OnceLock<PathBuf> = OnceLock::new();
static OUT_DIR: OnceLock<PathBuf> = OnceLock::new();
static MODE: OnceLock<String> = OnceLock::new();
/// The editor open counter: names the per-cycle ready and log files.
static OPEN_COUNT: AtomicUsize = AtomicUsize::new(0);

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

/// The entry init: the bundle path and the test environment. `plugin_path` is the bundle
/// the host passed to `clap_entry.init` (the page files live under Contents/Resources).
/// clap-sys 0.5 names one argument; hosts pass the factory id as a second one (ignored).
unsafe extern "C" fn e_init(plugin_path: *const c_char) -> bool {
    guard("entry init", move || {
        if let Some(mode) = env_var("DS_WEBVIEW_SPIKE_MODE") {
            let _ = MODE.set(mode);
        }
        if let Some(out) = env_var("DS_WEBVIEW_SPIKE_OUT") {
            let _ = OUT_DIR.set(PathBuf::from(out));
        }
        if let Some(p) = env_path_of(plugin_path) {
            let _ = PAGE_DIR.set(p.join("Contents").join("Resources").join("page"));
        }
        true
    })
}

fn env_path_of(p: *const c_char) -> Option<PathBuf> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().to_string();
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

unsafe extern "C" fn e_deinit() {}

// ---------------------------------------------------------------- the page server

/// The `dspage://` custom protocol: files from the bundle's page folder, nothing else.
/// On macOS wry registers a WKURLSchemeHandler; the request URI is `dspage://localhost/<path>`.
fn serve_page(
    _id: wry::WebViewId,
    request: wry::http::Request<Vec<u8>>,
) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    let path = request.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let body = PAGE_DIR.get().and_then(|dir| std::fs::read(dir.join(path)).ok());
    match body {
        Some(body) => respond(200, mime_of(path), body),
        None => respond(404, "text/plain", format!("not in the bundle: {path}").into_bytes()),
    }
}

fn respond(status: u16, mime: &str, body: Vec<u8>) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    wry::http::Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Cache-Control", "no-store")
        .body(std::borrow::Cow::Owned(body))
        .unwrap_or_else(|_| wry::http::Response::new(std::borrow::Cow::Owned(Vec::new())))
}

fn mime_of(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "woff" | "woff2" => "font/woff2",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------- the report channel

/// One IPC message of the page: one file in the out folder. The kind decides the name;
/// the body (valid JSON) is the evidence the checker reads.
fn handle_ipc(request: wry::http::Request<String>) {
    let body = request.into_body();
    let Some(out) = OUT_DIR.get() else { return };
    let kind = kind_of(&body);
    let name = match kind {
        Some("ready") => format!("ready-{}.json", OPEN_COUNT.load(Ordering::SeqCst).max(1)),
        Some("measure") => "measure.json".to_string(),
        Some("keys") => "keys.json".to_string(),
        Some("focus") => "focus.json".to_string(),
        Some("resize") => "resize.json".to_string(),
        _ => format!("log-{}.txt", OPEN_COUNT.load(Ordering::SeqCst).max(1)),
    };
    let content = match kind {
        Some("log") | None => body + "\n",
        _ => body,
    };
    let _ = std::fs::write(out.join(name), content);
}

fn kind_of(body: &str) -> Option<&str> {
    for k in ["ready", "measure", "keys", "focus", "resize", "log-no-ready", "log"] {
        if body.contains(&format!("\"kind\":\"{k}\"")) {
            return Some(k);
        }
    }
    None
}

// ---------------------------------------------------------------- the editor (the web view)

struct ParentView(*mut c_void);
impl wry::raw_window_handle::HasWindowHandle for ParentView {
    fn window_handle(&self) -> Result<wry::raw_window_handle::WindowHandle<'_>, wry::raw_window_handle::HandleError> {
        let ptr = std::ptr::NonNull::new(self.0).ok_or(wry::raw_window_handle::HandleError::Unavailable)?;
        let handle = wry::raw_window_handle::AppKitWindowHandle::new(ptr);
        Ok(unsafe { wry::raw_window_handle::WindowHandle::borrow_raw(handle.into()) })
    }
}

/// The editor of the plugin: one `wry` web view as a child of the host's NSView. Built at
/// `set_parent` (the CLAP cocoa parent is an NSView), dropped at `destroy`.
struct Editor {
    webview: wry::WebView,
}

/// The size the editor asks for (the host may resize it freely; can_resize answers true).
const EDITOR_W: u32 = 1000;
const EDITOR_H: u32 = 640;

impl Editor {
    /// Build the web view in `nsview` (the host's parent view, CLAP cocoa).
    fn build(nsview: *mut c_void) -> Result<Editor, String> {
        let mode = MODE.get().cloned().unwrap_or_else(|| "boot".to_string());
        // the init script: the mode first, then the stub (it reads the mode)
        let init_js = format!("window.__DS_SPIKE_MODE__=\"{mode}\";\n{}", include_str!("page_stub.js"));
        let parent = ParentView(nsview);
        if parent.0.is_null() {
            return Err("the host passed no cocoa parent view".to_string());
        }
        // the parent's current size: the web view starts filling it, and the autoresizing
        // mask keeps it filling it on every host resize. The parent bounds are logical
        // (points): wry maps them to the physical pixels of the display itself.
        let (w, h) = unsafe { view_size(parent.0) };
        let bounds = wry::Rect {
            position: wry::dpi::Position::Logical(wry::dpi::LogicalPosition::new(0.0, 0.0)),
            size: wry::dpi::Size::Logical(wry::dpi::LogicalSize::new(w, h)),
        };
        let webview = wry::WebViewBuilder::new()
            .with_bounds(bounds)
            .with_url("dspage://localhost/index.html")
            .with_custom_protocol("dspage".into(), serve_page)
            .with_initialization_script(init_js)
            .with_ipc_handler(handle_ipc)
            .with_focused(true)
            .build_as_child(&parent)
            .map_err(|e| format!("the web view did not build: {e}"))?;
        // fill the parent on every resize (wry sets only ViewMinYMargin in child mode)
        unsafe {
            let v = webview.webview();
            let mask: objc2_app_kit::NSAutoresizingMaskOptions =
                objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                    | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable;
            let _: () = msg_send![&*v, setAutoresizingMask: mask];
            // the keyboard: wry's `with_focused` is a no-op on macOS, so the web view never
            // becomes first responder by itself. A host that shows an editor expects the
            // editor to take the keys: make the WKWebView the first responder of its window.
            let win: *mut AnyObject = msg_send![&*v, window];
            if !win.is_null() {
                let _: () = msg_send![win, makeFirstResponder: &*v];
            }
        }
        OPEN_COUNT.fetch_add(1, Ordering::SeqCst);
        Ok(Editor { webview })
    }

    /// Run one JS snippet in the page (the native side may ask for a report at once).
    fn evaluate(&self, js: &str) {
        let _ = self.webview.evaluate_script(js);
    }

    /// Resize the editor (CLAP set_size): the web view fills the parent view; the plugin
    /// moves its own view to the requested rect (the autoresizing mask keeps it filling
    /// the parent on later host resizes).
    fn set_size(&self, w: u32, h: u32) {
        let _ = self.webview.set_bounds(wry::Rect {
            position: wry::dpi::Position::Physical(wry::dpi::PhysicalPosition::new(0, 0)),
            size: wry::dpi::Size::Physical(wry::dpi::PhysicalSize::new(w.max(320), h.max(240))),
        });
    }
}

/// The bounds of the host's parent view (the web view must fill it from the first frame).
unsafe fn view_size(nsview: *mut c_void) -> (f64, f64) {
    let rect: objc2_foundation::NSRect = msg_send![nsview as *const AnyObject, bounds];
    (rect.size.width.max(320.0), rect.size.height.max(240.0))
}

// ---------------------------------------------------------------- the CLAP plugin

const PLUGIN_ID: *const c_char = cs!("com.cherryblossomvalley.drumspike.webviewspike");

static FEATURES: SyncPtr<[*const c_char; 2]> = SyncPtr([cs!("instrument"), std::ptr::null()]);

struct SyncPtr<T>(pub T);
unsafe impl<T> Sync for SyncPtr<T> {}
unsafe impl<T> Send for SyncPtr<T> {}

static DESCRIPTOR: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: PLUGIN_ID,
    name: cs!("Drum Spike Webview Spike"),
    vendor: cs!("Cherry Blossom Valley"),
    url: cs!("https://github.com/Cherry-Blossom-Valley/drumspike"),
    manual_url: cs!(""),
    support_url: cs!(""),
    version: cs!("0.0.1"),
    description: cs!("Spike: the editor page in a plugin window (wry)"),
    features: &FEATURES.0 as *const [*const c_char; 2] as *const *const c_char,
};

struct Plugin {
    clap: clap_plugin,
    editor: Option<Editor>,
}

// the editor is built and dropped on the main thread only (the CLAP GUI contract)
unsafe impl Send for Plugin {}

/// `clap/gui`: the cocoa child-window API, the web view of the page in the host's window.
static PLUGIN_GUI: clap_plugin_gui = clap_plugin_gui {
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

/// The CLAP window api strings cross the dylib boundary: compare by content, not by pointer.
fn is_api(api: *const c_char, want: &[u8]) -> bool {
    !api.is_null() && unsafe { CStr::from_ptr(api) }.to_bytes() == want
}

unsafe extern "C" fn g_is_api_supported(_p: *const clap_plugin, api: *const c_char, floating: bool) -> bool {
    guard("gui.is_api_supported", move || !floating && is_api(api, b"cocoa"))
}

unsafe extern "C" fn g_preferred_api(_p: *const clap_plugin, api: *mut *const c_char, floating: *mut bool) -> bool {
    guard("gui.get_preferred_api", move || {
        *api = CLAP_WINDOW_API_COCOA.as_ptr();
        *floating = false;
        true
    })
}

/// create(): the editor exists as a flag; the web view comes with set_parent (child mode).
unsafe extern "C" fn g_create(_p: *const clap_plugin, api: *const c_char, floating: bool) -> bool {
    guard("gui.create", move || !(floating || !is_api(api, b"cocoa")))
}

fn plugin_mut<'a>(p: *const clap_plugin) -> Option<&'a mut Plugin> {
    if p.is_null() {
        return None;
    }
    unsafe { ((*p).plugin_data as *mut Plugin).as_mut() }
}

/// set_parent(): build the web view in the host's NSView.
unsafe extern "C" fn g_set_parent(p: *const clap_plugin, window: *const clap_window) -> bool {
    guard("gui.set_parent", move || {
        let Some(plugin) = plugin_mut(p) else { return false };
        if window.is_null() {
            return false;
        }
        let w = &*window;
        if !is_api(w.api, b"cocoa") {
            return false;
        }
        let nsview = w.specific.cocoa;
        match Editor::build(nsview) {
            Ok(ed) => {
                plugin.editor = Some(ed);
                true
            }
            Err(e) => {
                eprintln!("webview-spike: {e}");
                false
            }
        }
    })
}

unsafe extern "C" fn g_destroy(p: *const clap_plugin) {
    guard("gui.destroy", move || {
        if let Some(plugin) = plugin_mut(p) {
            plugin.editor = None; // the drop detaches the WKWebView from the host's view
        }
    })
}

unsafe extern "C" fn g_show(p: *const clap_plugin) -> bool {
    // child mode: the host's view owns the visibility; the web view is already in it
    guard("gui.show", move || {
        plugin_mut(p).map(|pl| pl.editor.is_some()).unwrap_or(false)
    })
}

unsafe extern "C" fn g_hide(_p: *const clap_plugin) -> bool {
    guard("gui.hide", move || true)
}

unsafe extern "C" fn g_set_scale(_p: *const clap_plugin, _scale: f64) -> bool {
    guard("gui.set_scale", move || true)
}

unsafe extern "C" fn g_get_size(_p: *const clap_plugin, w: *mut u32, h: *mut u32) -> bool {
    guard("gui.get_size", move || {
        if w.is_null() || h.is_null() {
            return false;
        }
        *w = EDITOR_W;
        *h = EDITOR_H;
        true
    })
}

unsafe extern "C" fn g_can_resize(_p: *const clap_plugin) -> bool {
    guard("gui.can_resize", move || true)
}

unsafe extern "C" fn g_adjust_size(_p: *const clap_plugin, w: *mut u32, h: *mut u32) -> bool {
    guard("gui.adjust_size", move || {
        if w.is_null() || h.is_null() {
            return false;
        }
        *w = (*w).max(320);
        *h = (*h).max(240);
        true
    })
}

unsafe extern "C" fn g_set_size(p: *const clap_plugin, w: u32, h: u32) -> bool {
    guard("gui.set_size", move || {
        let Some(plugin) = plugin_mut(p) else { return false };
        match &plugin.editor {
            Some(ed) => {
                ed.set_size(w.max(320), h.max(240));
                true
            }
            None => false,
        }
    })
}

unsafe extern "C" fn g_suggest_title(_p: *const clap_plugin, _title: *const c_char) {}

// the plugin: no audio, no parameters - the spike is the editor window only

unsafe extern "C" fn p_init(_p: *const clap_plugin) -> bool {
    guard("init", move || true)
}

unsafe extern "C" fn p_destroy(p: *const clap_plugin) {
    if !p.is_null() {
        unsafe { drop(Box::from_raw((*p).plugin_data as *mut Plugin)) };
    }
}

unsafe extern "C" fn p_activate(_p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    guard("activate", move || true)
}
unsafe extern "C" fn p_deactivate(_p: *const clap_plugin) {}
unsafe extern "C" fn p_start(_p: *const clap_plugin) -> bool {
    guard("start_processing", move || true)
}
unsafe extern "C" fn p_stop(_p: *const clap_plugin) {}
unsafe extern "C" fn p_reset(_p: *const clap_plugin) {}

unsafe extern "C" fn p_process(
    _p: *const clap_plugin,
    process: *const clap_sys::process::clap_process,
) -> clap_sys::process::clap_process_status {
    guard("process", move || {
        // silence out; the spike has no audio of its own
        let pr = &*process;
        if pr.audio_outputs_count > 0 {
            let out = &*pr.audio_outputs;
            let frames = pr.frames_count as usize;
            for c in 0..out.channel_count as usize {
                let dst = *out.data32.add(c);
                if !dst.is_null() {
                    std::ptr::write_bytes(dst, 0, frames);
                }
            }
        }
        clap_sys::process::CLAP_PROCESS_CONTINUE
    })
}

unsafe extern "C" fn p_get_extension(_p: *const clap_plugin, id: *const c_char) -> *const c_void {
    guard("get_extension", move || {
        let id = CStr::from_ptr(id).to_bytes();
        if id == CLAP_EXT_GUI.to_bytes() {
            return &PLUGIN_GUI as *const _ as *const c_void;
        }
        std::ptr::null()
    })
}

unsafe extern "C" fn p_on_main_thread(_p: *const clap_plugin) {}

unsafe extern "C" fn f_count(_f: *const clap_plugin_factory) -> u32 {
    1
}
unsafe extern "C" fn f_desc(_f: *const clap_plugin_factory, i: u32) -> *const clap_plugin_descriptor {
    if i == 0 {
        &DESCRIPTOR
    } else {
        std::ptr::null()
    }
}
unsafe extern "C" fn f_create(
    _f: *const clap_plugin_factory,
    _host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
    if CStr::from_ptr(id) != CStr::from_ptr(PLUGIN_ID) {
        return std::ptr::null();
    }
    let p = Box::into_raw(Box::new(Plugin {
        clap: clap_plugin {
            desc: &DESCRIPTOR,
            plugin_data: std::ptr::null_mut(),
            init: Some(p_init),
            destroy: Some(p_destroy),
            activate: Some(p_activate),
            deactivate: Some(p_deactivate),
            start_processing: Some(p_start),
            stop_processing: Some(p_stop),
            reset: Some(p_reset),
            process: Some(p_process),
            get_extension: Some(p_get_extension),
            on_main_thread: Some(p_on_main_thread),
        },
        editor: None,
    }));
    unsafe {
        (*p).clap.plugin_data = p as *mut c_void;
        &(*p).clap
    }
}

static PLUGIN_FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(f_count),
    get_plugin_descriptor: Some(f_desc),
    create_plugin: Some(f_create),
};

/// The CLAP entry the loader looks for (the bundle carries it in its binary).
#[no_mangle]
#[used]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(e_init),
    deinit: Some(e_deinit),
    get_factory: Some(e_get_factory),
};

unsafe extern "C" fn e_get_factory(id: *const c_char) -> *const c_void {
    let id = CStr::from_ptr(id).to_bytes();
    if id == CLAP_PLUGIN_FACTORY_ID.to_bytes() {
        return &PLUGIN_FACTORY as *const _ as *const c_void;
    }
    std::ptr::null()
}

/// A host or a test may ask the page for its report at once (the evaluate path of `Editor`).
/// Kept public for the crate tests: the same JS the plugin would run.
pub fn report_now_js() -> String {
    "window.__DSSPIKE_REPORT__ && window.__DSSPIKE_REPORT__();".to_string()
}

/// The identifier of the plugin (the host and the wrapped VST3 address it).
pub fn plugin_id() -> CString {
    CString::new(&b"com.cherryblossomvalley.drumspike.webviewspike"[..]).unwrap_or_default()
}
