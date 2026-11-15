//! The test host of the webview spike (S2, #505): a small AppKit app that loads the CLAP
//! bundle, opens the editor in its own window (the CLAP GUI extension, cocoa child of the
//! content view) and runs one phase of `check_s2.sh`. One report file per phase, one JSON
//! line on stdout, exit 0 when the phase answered, 2 on a timeout or a refusal. A crash of
//! the editor is a crash of this process: the caller sees a non-zero rc.
//!
//! usage: webview-spike-host <path-to-.clap-bundle-or-its-binary>
//!          [--mode boot|measure|keys|focus|resize] [--cycles N] [--out DIR]
//!
//! phases (the plugin and the injected page stub read the same environment):
//!   boot     open the editor once, wait for the page's ready message (20 s, gate item 2)
//!   measure  boot, then the page reports its frame times while it scrolls and zooms
//!   keys     boot, then the host sends Space, Right, Cmd+Z, Tab, Esc (and Space again)
//!            as real events into the window; the page reports what reached it
//!   focus    boot, then the host clicks out (titlebar) and back in (the editor), then a
//!            key; the page reports hasFocus over time
//!   resize   boot, then the host resizes the window twice; the page reports innerWidth
//!   --cycles N: open and close the editor N times (one plugin per cycle), each with a
//!            ready wait; the proof of gate item 2 (no crash over 50 open/close cycles)
//!
//! The main thread pumps the run loop in slices (CFRunLoopRunInMode + one AppKit event
//! pass): the page's timers, the compositor and the event delivery all need it.
#![allow(clippy::too_many_arguments)]

use libloading::os::unix::{Library, Symbol};
use objc2::rc::Retained;
use objc2::MainThreadOnly;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEvent, NSEventMask, NSEventModifierFlags,
    NSEventType, NSResponder, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{ns_string, NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const PLUGIN_ID: &[u8] = b"com.cherryblossomvalley.drumspike.webviewspike\0";
/// The ready deadline of the gate (item 2): the page must be through its boot in 20 s.
const READY_LIMIT: Duration = Duration::from_secs(20);
/// The later phases boot first and then drive for a while; their limit is looser.
const PHASE_LIMIT: Duration = Duration::from_secs(40);

type CGKeyCode = u16;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source_handled: bool) -> i32;
}

// the key codes of the ANSI layout (the keys the gate names: Space, arrows, Cmd+Z, Tab, Esc)
const K_SPACE: CGKeyCode = 49;
const K_TAB: CGKeyCode = 48;
const K_ESCAPE: CGKeyCode = 53;
const K_RIGHT: CGKeyCode = 124;
const K_Z: CGKeyCode = 6;
/// NSEventModifierFlagCommand == the CGEvent flag of the same name (1 << 20).
const FLAG_COMMAND: u64 = 1 << 20;

/// The binary inside the bundle: a directory path becomes `<bundle>/Contents/MacOS/<name>`.
fn binary_of(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_dir() {
        let macos = p.join("Contents").join("MacOS");
        let bin = std::fs::read_dir(&macos)
            .ok()
            .and_then(|d| d.flatten().next())
            .map(|e| e.path());
        return bin.unwrap_or_else(|| p.clone());
    }
    p
}

/// The bundle path (the `init` argument): two levels above the binary.
fn bundle_of(path: &str) -> String {
    let p = PathBuf::from(path);
    if p.is_dir() {
        return p.display().to_string();
    }
    p.ancestors()
        .nth(2)
        .map(|a| a.display().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == name {
            return Some(args[i + 1].clone());
        }
        i += 1;
    }
    None
}

#[derive(Clone, Copy)]
enum Action {
    /// A real key event through the app's responder chain (the gate's keys: Space, arrows,
    /// Cmd+Z, Tab, Esc). A host receives the keyDown from the window server and calls
    /// [NSApp sendEvent:]; from a shell without the accessibility trust this host cannot
    /// tap the HID level (CGEventPostToPid is dropped), so it sends the same NSEvent itself.
    Key(CGKeyCode, u64),
    /// The host's own view takes the first responder: what a DAW does when the user
    /// clicks into its UI and the page loses the focus.
    ClickOut,
    /// A real mouse-down/up into the editor: the web view takes the first responder back.
    ClickIn,
}

/// The scheduled actions of the phase, in milliseconds from the editor being shown.
fn schedule_of(mode: &str) -> Vec<(u64, Action)> {
    match mode {
        "keys" => vec![
            (2500, Action::Key(K_SPACE, 0)),
            (3500, Action::Key(K_RIGHT, 0)),
            (4500, Action::Key(K_Z, FLAG_COMMAND)),
            (5500, Action::Key(K_TAB, 0)),
            (6500, Action::Key(K_ESCAPE, 0)),
            (7500, Action::Key(K_SPACE, 0)),
        ],
        "focus" => vec![
            (2500, Action::ClickOut),        // the host takes the focus from the page
            (3500, Action::Key(K_SPACE, 0)), // ... this key must NOT reach the page
            (4500, Action::ClickIn),         // a real click into the editor
            (5500, Action::Key(K_SPACE, 0)), // ... this key must reach the page again
        ],
        _ => vec![],
    }
}

/// The characters WebKit sees for a key code (the DOM `code` comes from the key code
/// itself; the characters feed `ev.key` and the text insertion).
fn chars_of(code: CGKeyCode) -> &'static str {
    match code {
        K_SPACE => " ",
        K_TAB => "\t",
        K_ESCAPE => "\u{1b}",
        K_RIGHT => "\u{f703}", // NSRightArrowFunctionKey
        _ => "z",              // K_Z
    }
}

fn post_key(code: CGKeyCode, flags: u64, app: &NSApplication, window: &NSWindow) {
    let chars = objc2_foundation::NSString::from_str(chars_of(code));
    for ty in [NSEventType::KeyDown, NSEventType::KeyUp] {
        let ev = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            ty,
            NSPoint::new(0.0, 0.0),
            NSEventModifierFlags(flags as usize),
            0.0,
            window.windowNumber(),
            None,
            &chars,
            &chars,
            false,
            code,
        );
        if let Some(ev) = ev {
            app.sendEvent(&ev);
        }
    }
}

fn post_click(app: &NSApplication, window: &NSWindow) {
    // the centre of the editor area (below the title bar), in window coordinates
    let f = window.frame();
    let at = NSPoint::new(f.size.width / 2.0, (f.size.height - 28.0) / 2.0);
    for (ty, pressure) in [(NSEventType::LeftMouseDown, 1.0), (NSEventType::LeftMouseUp, 0.0)] {
        let ev = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            ty,
            at,
            NSEventModifierFlags(0),
            0.0,
            window.windowNumber(),
            None,
            1,
            1,
            pressure,
        );
        if let Some(ev) = ev {
            app.sendEvent(&ev);
        }
    }
}

/// One slice of the run loop (timers, Core Animation, the web content IPC), then one
/// AppKit event pass (the CGEvents of the schedule and the key handling of the web view).
fn pump(app: &NSApplication) {
    unsafe {
        let mode = objc2_foundation::NSString::from_str("kCFRunLoopDefaultMode");
        CFRunLoopRunInMode(
            (&*mode) as *const objc2_foundation::NSString as *const c_void,
            0.02,
            false,
        );
        let ev = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::KeyDown
                | NSEventMask::KeyUp
                | NSEventMask::FlagsChanged
                | NSEventMask::LeftMouseDown
                | NSEventMask::LeftMouseUp
                | NSEventMask::MouseMoved,
            Some(&NSDate::now()),
            NSDefaultRunLoopMode,
            true,
        );
        if let Some(ev) = ev {
            app.sendEvent(&ev);
        }
        app.updateWindows();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(plugin_path) = args.first() else {
        eprintln!(
            "usage: webview-spike-host <path-to-.clap-bundle-or-its-binary> \
             [--mode boot|measure|keys|focus|resize] [--cycles N] [--out DIR]"
        );
        std::process::exit(64);
    };
    let mode = arg_value(&args, "--mode").unwrap_or_else(|| "boot".to_string());
    let cycles: usize = arg_value(&args, "--cycles").and_then(|v| v.parse().ok()).unwrap_or(0);
    let out_dir = PathBuf::from(arg_value(&args, "--out").unwrap_or_else(|| "out".to_string()));
    let _ = std::fs::create_dir_all(&out_dir);

    // the plugin and the injected stub read this before the first page boot
    std::env::set_var("DS_WEBVIEW_SPIKE_MODE", &mode);
    std::env::set_var("DS_WEBVIEW_SPIKE_OUT", out_dir.display().to_string());

    let mtm = objc2::MainThreadMarker::new().expect("the host runs on the main thread");
    unsafe {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(120.0, 120.0), NSSize::new(1000.0, 640.0)),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setTitle(ns_string!("Drum Spike webview spike host"));
        window.makeKeyAndOrderFront(None);
        app.finishLaunching();
        // the app runs from a shell: without the activation the window never becomes key
        // and no key or click reaches the page (the page reports hasFocus=false forever).
        // activateIgnoringOtherApps works on every macOS; activate() needs 14.
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);

        let bin = binary_of(plugin_path);
        let bundle = bundle_of(plugin_path);
        let lib = match Library::new(&bin) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("webview-spike-host: dlopen {bin:?} failed: {e}");
                std::process::exit(3);
            }
        };
        let entry: Symbol<*mut c_void> = match lib.get(b"clap_entry") {
            Ok(s) => s,
            Err(e) => {
                eprintln!("webview-spike-host: no clap_entry in {bin:?}: {e}");
                std::process::exit(3);
            }
        };
        let entry = entry.into_raw() as *const clap_plugin_entry;
        let plugin_path_c = CString::new(bundle).unwrap_or_default();
        let factory_id = CStr::from_bytes_with_nul(CLAP_PLUGIN_FACTORY_ID_BYTES).expect("factory id");
        let init: EntryInit = ((*entry).init).expect("clap_entry.init");
        if !init(plugin_path_c.as_ptr()) {
            eprintln!("webview-spike-host: clap_entry.init refused");
            std::process::exit(3);
        }
        let factory =
            ((*entry).get_factory).expect("clap_entry.get_factory")(factory_id.as_ptr()) as *const clap_plugin_factory;
        if factory.is_null() {
            eprintln!("webview-spike-host: no plugin factory");
            std::process::exit(3);
        }
        let id = CString::new(&PLUGIN_ID[..PLUGIN_ID.len() - 1]).unwrap_or_default();
        let gui_id = CStr::from_bytes_with_nul(CLAP_EXT_GUI_BYTES).expect("gui id");
        let cocoa_c = CLAP_WINDOW_API_COCOA_BYTES.as_ptr() as *const c_char;

        let host: *const clap_host = &HOST;
        let n_rounds = if cycles > 0 { cycles } else { 1 };
        let mut opened = 0usize;
        let mut failed: Option<String> = None;
        for round in 1..=n_rounds {
            let plugin = ((*factory).create_plugin).expect("create_plugin")(factory, host, id.as_ptr());
            if plugin.is_null() {
                failed = Some("create_plugin returned null".to_string());
                break;
            }
            if !((*plugin).init.expect("init"))(plugin) {
                ((*plugin).destroy.expect("destroy"))(plugin);
                failed = Some("plugin init refused".to_string());
                break;
            }
            let gui =
                ((*plugin).get_extension.expect("get_extension"))(plugin, gui_id.as_ptr()) as *const clap_plugin_gui;
            if gui.is_null() {
                ((*plugin).destroy.expect("destroy"))(plugin);
                failed = Some("no clap.gui extension".to_string());
                break;
            }
            if !((*gui).create.expect("create"))(plugin, cocoa_c, false) {
                ((*plugin).destroy.expect("destroy"))(plugin);
                failed = Some("gui create refused".to_string());
                break;
            }
            let parent_view = window
                .contentView()
                .map(|v| (&*v) as *const NSView as *mut c_void)
                .unwrap_or(std::ptr::null_mut());
            let parent = clap_window {
                api: cocoa_c,
                specific: clap_window_handle { cocoa: parent_view },
            };
            if parent_view.is_null() || !((*gui).set_parent.expect("set_parent"))(plugin, &parent) {
                ((*gui).destroy.expect("destroy"))(plugin);
                ((*plugin).destroy.expect("destroy"))(plugin);
                failed = Some("gui set_parent refused (the web view did not build)".to_string());
                break;
            }
            if !((*gui).show.expect("show"))(plugin) {
                ((*gui).destroy.expect("destroy"))(plugin);
                ((*plugin).destroy.expect("destroy"))(plugin);
                failed = Some("gui show refused".to_string());
                break;
            }

            // the phase: wait for the report file while the run loop stays alive
            let want = match mode.as_str() {
                "measure" => out_dir.join("measure.json"),
                "keys" => out_dir.join("keys.json"),
                "focus" => out_dir.join("focus.json"),
                "resize" => out_dir.join("resize.json"),
                _ => out_dir.join(format!("ready-{round}.json")),
            };
            let _ = std::fs::remove_file(&want); // a stale file of an earlier run must not pass
            let limit = if mode == "boot" || mode == "no-ready" || cycles > 0 {
                READY_LIMIT
            } else {
                PHASE_LIMIT
            };
            let deadline = Instant::now() + limit;
            let t0 = Instant::now();
            let mut schedule = schedule_of(&mode);
            let mut resized = 0usize;
            // the editor view that had the first responder before the host took it
            // (ClickIn hands it back; see the comment there)
            let mut saved_resp: Option<Retained<NSResponder>> = None;
            let mut got = false;
            while Instant::now() < deadline {
                let elapsed = t0.elapsed().as_millis() as u64;
                let now_due: Vec<Action> = schedule
                    .iter()
                    .filter(|(at, _)| *at <= elapsed)
                    .map(|(_, a)| *a)
                    .collect();
                for a in now_due {
                    match a {
                        Action::Key(code, flags) => post_key(code, flags, &app, &window),
                        Action::ClickOut => {
                            // the host's own view takes the first responder: the click in
                            // the host that takes the focus from the page. The view that
                            // had it (the editor's web view) comes back with ClickIn.
                            saved_resp = window.firstResponder();
                            let cv = window.contentView();
                            if let Some(cv) = cv {
                                window.makeFirstResponder(Some(&*cv));
                            }
                        }
                        Action::ClickIn => {
                            // the real mouse-down into the editor; on the headless host the
                            // window-server part of the click cannot complete (no key window,
                            // no activation from a shell), so the host hands the first
                            // responder to the view the click landed on — what the web view
                            // does by itself in a GUI session.
                            post_click(&app, &window);
                            if let Some(r) = &saved_resp {
                                window.makeFirstResponder(Some(&**r));
                            }
                        }
                    }
                    schedule.retain(|(at, _)| *at > elapsed);
                }
                if mode == "resize" {
                    if resized == 0 && elapsed >= 2000 {
                        window.setFrame_display(
                            NSRect::new(NSPoint::new(120.0, 120.0), NSSize::new(1400.0, 850.0)),
                            true,
                        );
                        resized = 1;
                    } else if resized == 1 && elapsed >= 4000 {
                        window
                            .setFrame_display(NSRect::new(NSPoint::new(120.0, 120.0), NSSize::new(760.0, 480.0)), true);
                        resized = 2;
                    }
                }
                pump(&app);
                if want.exists() {
                    got = true;
                    break;
                }
            }

            // teardown of the round, whether or not the phase answered
            ((*gui).destroy.expect("gui.destroy"))(plugin);
            ((*plugin).deactivate.expect("deactivate"))(plugin);
            ((*plugin).destroy.expect("destroy"))(plugin);
            if !got {
                failed = Some(format!(
                    "no report after {} s ({}), phase {mode}",
                    limit.as_secs(),
                    want.display()
                ));
                break;
            }
            opened += 1;
            std::thread::sleep(Duration::from_millis(150)); // settle between the cycles
        }

        ((*entry).deinit).expect("deinit")();
        drop(lib);

        let report = match &failed {
            None => format!("{{\"mode\":\"{mode}\",\"cycles\":{opened},\"ok\":true}}\n"),
            Some(why) => {
                format!("{{\"mode\":\"{mode}\",\"cycles\":{opened},\"ok\":false,\"why\":{why:?}}}\n")
            }
        };
        print!("{report}");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        std::process::exit(if failed.is_none() { 0 } else { 2 });
    }
}

// ---------------------------------------------------------------- the CLAP ABI of the host

#[repr(C)]
#[derive(Copy, Clone)]
union clap_window_handle {
    cocoa: *mut c_void,
}

#[repr(C)]
struct clap_window {
    api: *const c_char,
    specific: clap_window_handle,
}

unsafe impl Send for clap_window {}
unsafe impl Sync for clap_window {}

unsafe impl Send for clap_host {}
unsafe impl Sync for clap_host {}

#[repr(C)]
#[derive(Copy, Clone)]
struct clap_gui_resize_hints {
    can_resize_horizontally: bool,
    can_resize_vertically: bool,
    preserve_aspect_ratio: bool,
    aspect_ratio_width: u32,
    aspect_ratio_height: u32,
}

#[repr(C)]
struct clap_plugin_gui {
    is_api_supported: Option<unsafe extern "C" fn(*const clap_plugin, *const c_char, bool) -> bool>,
    get_preferred_api: Option<unsafe extern "C" fn(*const clap_plugin, *mut *const c_char, *mut bool) -> bool>,
    create: Option<unsafe extern "C" fn(*const clap_plugin, *const c_char, bool) -> bool>,
    destroy: Option<unsafe extern "C" fn(*const clap_plugin)>,
    set_scale: Option<unsafe extern "C" fn(*const clap_plugin, f64) -> bool>,
    get_size: Option<unsafe extern "C" fn(*const clap_plugin, *mut u32, *mut u32) -> bool>,
    can_resize: Option<unsafe extern "C" fn(*const clap_plugin) -> bool>,
    get_resize_hints: Option<unsafe extern "C" fn(*const clap_plugin, *mut clap_gui_resize_hints) -> bool>,
    adjust_size: Option<unsafe extern "C" fn(*const clap_plugin, *mut u32, *mut u32) -> bool>,
    set_size: Option<unsafe extern "C" fn(*const clap_plugin, u32, u32) -> bool>,
    set_parent: Option<unsafe extern "C" fn(*const clap_plugin, *const clap_window) -> bool>,
    set_transient: Option<unsafe extern "C" fn(*const clap_plugin, *const clap_window) -> bool>,
    suggest_title: Option<unsafe extern "C" fn(*const clap_plugin, *const c_char)>,
    show: Option<unsafe extern "C" fn(*const clap_plugin) -> bool>,
    hide: Option<unsafe extern "C" fn(*const clap_plugin) -> bool>,
}

#[repr(C)]
struct clap_host {
    clap_version: clap_version_t,
    host_data: *mut c_void,
    name: *const c_char,
    vendor: *const c_char,
    url: *const c_char,
    version: *const c_char,
    get_extension: Option<unsafe extern "C" fn(*const clap_host, *const c_char) -> *const c_void>,
    request_restart: Option<unsafe extern "C" fn(*const clap_host)>,
    request_process: Option<unsafe extern "C" fn(*const clap_host)>,
    request_callback: Option<unsafe extern "C" fn(*const clap_host)>,
}

#[repr(C)]
struct clap_plugin_descriptor {
    clap_version: u32,
    id: *const c_char,
    name: *const c_char,
    vendor: *const c_char,
    url: *const c_char,
    manual_url: *const c_char,
    support_url: *const c_char,
    version: *const c_char,
    description: *const c_char,
    features: *const *const c_char,
}

#[repr(C)]
struct clap_plugin {
    desc: *const clap_plugin_descriptor,
    plugin_data: *mut c_void,
    init: Option<unsafe extern "C" fn(*const clap_plugin) -> bool>,
    destroy: Option<unsafe extern "C" fn(*const clap_plugin)>,
    activate: Option<unsafe extern "C" fn(*const clap_plugin, f64, u32, u32) -> bool>,
    deactivate: Option<unsafe extern "C" fn(*const clap_plugin)>,
    start_processing: Option<unsafe extern "C" fn(*const clap_plugin) -> bool>,
    stop_processing: Option<unsafe extern "C" fn(*const clap_plugin)>,
    reset: Option<unsafe extern "C" fn(*const clap_plugin)>,
    process: Option<unsafe extern "C" fn(*const clap_plugin, *const c_void) -> u32>,
    get_extension: Option<unsafe extern "C" fn(*const clap_plugin, *const c_char) -> *const c_void>,
    on_main_thread: Option<unsafe extern "C" fn(*const clap_plugin)>,
}

#[repr(C)]
struct clap_plugin_factory {
    get_plugin_count: Option<unsafe extern "C" fn(*const clap_plugin_factory) -> u32>,
    get_plugin_descriptor:
        Option<unsafe extern "C" fn(*const clap_plugin_factory, u32) -> *const clap_plugin_descriptor>,
    create_plugin:
        Option<unsafe extern "C" fn(*const clap_plugin_factory, *const clap_host, *const c_char) -> *const clap_plugin>,
}

#[repr(C)]
struct clap_plugin_entry {
    clap_version: clap_version_t,
    init: Option<unsafe extern "C" fn(*const c_char) -> bool>,
    deinit: Option<unsafe extern "C" fn()>,
    get_factory: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
}

type EntryInit = unsafe extern "C" fn(*const c_char) -> bool;

#[repr(C)]
#[derive(Copy, Clone)]
struct clap_version_t {
    major: u32,
    minor: u32,
    revision: u32,
}

const CLAP_VERSION: clap_version_t = clap_version_t {
    major: 1,
    minor: 2,
    revision: 2,
};
const CLAP_PLUGIN_FACTORY_ID_BYTES: &[u8] = b"clap.plugin-factory\0";
const CLAP_EXT_GUI_BYTES: &[u8] = b"clap.gui\0";
const CLAP_WINDOW_API_COCOA_BYTES: &[u8] = b"cocoa\0";

macro_rules! cs {
    ($s:literal) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

static HOST: clap_host = clap_host {
    clap_version: CLAP_VERSION,
    host_data: std::ptr::null_mut(),
    name: cs!("webview-spike-host"),
    vendor: cs!("Cherry Blossom Valley"),
    url: cs!("https://github.com/Cherry-Blossom-Valley/drumspike"),
    version: cs!("0.0.1"),
    get_extension: None,
    request_restart: None,
    request_process: None,
    request_callback: None,
};
