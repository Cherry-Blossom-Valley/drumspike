//! The `editor` mode of the test host (M1b2): the host analyses the files in an ARA document,
//! binds a plugin instance to it, opens the plugin's editor in a real AppKit window (the
//! CLAP GUI extension, cocoa, child of the content view) and runs one phase of the check. A
//! phase ends when the page's report arrives in the plugin's report file (DS_ARA_OUT). The
//! main thread pumps the run loop in slices: the page's timers, the compositor, the main
//! dispatch queue (the plugin answers the web view there) and the event delivery need it.
//!
//! ```text
//! ara-host <bundle> editor --files <wav>... --editor-mode ready|keys|focus|resize|measure|edit|shell|analyse|net|transport|hit|menu|geometry
//!                          |edit-round|open-first|refuse|bigpost
//!                          [--cycles N] [--archive <file>] [--arg <text>] [--no-ara]
//!                          [--late-files <wav>...] [--wait-sync]
//! `--store-at-edit <file>`: the archive is stored the moment the page's edit report arrives
//! (modes `edit`, `edit-destroy`, `edit-region`); `edit-destroy` then closes the editor and the
//! document at once. `edit-region`: after the edit the host trims the region of the edited
//! source (`--region a,d,p`). `analyse-race --destroy-late <i> --archive-window <file>`: the
//! host destroys source `i` while the page's own analysis runs and stores an archive in that time.
//! Mode `leftovers` (M1d1): after the page's `leftovers` report the host replaces the audio of source k (`--update-content-after <k>:<wav>`);
//! the page reports again once the plugin has analysed it and told the page.
//! Mode `names` (M1b10): after the page's `names` report the host renames region sequences
//! (`--rename-seq <j>=<name>`) and moves regions (`--move-region <i>=<j>`); the page reports again.
//! `--late-files` are added to the document (and analysed) while the editor is open: in
//! `open-first` when the page has booted (the files then are `--late-files` only), in
//! `edit-round` and `refuse` after the page's report, in `analyse-race` when the page's own
//! analysis has started. `--wait-sync` (mode `edit`): after the edit the host waits for the row
//! `page_sync` of the plugin's worker (without the scenario `sync-delay`).
//! ```
//! Keys and clicks are real `NSEvent`s through `[NSApp sendEvent:]` (the S2 finding: the
//! window server path needs the accessibility trust).
use super::*;
use clap_sys::ext::gui::{clap_plugin_gui, clap_window, clap_window_handle};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AnyThread, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEvent, NSEventMask, NSEventModifierFlags,
    NSEventType, NSMenu, NSMenuItem, NSResponder, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize};

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source_handled: bool) -> i32;
}

type Key = u16;
const K_SPACE: Key = 49;
const K_TAB: Key = 48;
const K_ESCAPE: Key = 53;
const K_RIGHT: Key = 124;
const K_Z: Key = 6;
const FLAG_COMMAND: u64 = 1 << 20;

#[derive(Clone, Copy)]
enum Action {
    Key(Key, u64),
    /// M1b12: a key with a label, its characters, its characters without modifiers, its flags.
    Hk(&'static str, Key, &'static str, &'static str, u64),
    /// A key down and up with `isARepeat` set: a key that is held.
    HkRepeat(&'static str, Key, &'static str, &'static str, u64),
    /// A key, then the host's main thread is blocked for 400 ms before it runs the loop again (a DAW that stalls).
    HkSlow(&'static str, Key, &'static str, &'static str, u64),
    /// A change of the modifier keys alone.
    Flags(&'static str, u64),
    /// A real mouse click (down and up through `[NSApp sendEvent:]`) at a point of the window.
    Mouse(&'static str, f64, f64),
    /// The action of a menu item with no target (Save): what a click on the menu sends.
    Menu(&'static str),
    ClickOut,
    ClickIn,
}

// ---- M1b12 (#561): the record of the keys that reach the host

const FLAG_OPTION: u64 = 1 << 19;
const K_F12: Key = 111;
const K_F13: Key = 105;
const K_F14: Key = 107;

/// One key event that reached the host: through its view (`view`: the next responder of the plugin's view
/// goes on to it) or its main menu (`menu`: a key command of the host, as a DAW has).
struct SeenKey {
    step: String,
    via: &'static str,
    kind: String,
    code: u16,
    chars: String,
    flags: u64,
    title: String,
}

static SEEN_KEYS: Mutex<Vec<SeenKey>> = Mutex::new(Vec::new());
static KEY_STEP: Mutex<String> = Mutex::new(String::new());
/// What the host posted, in order: (label, key code, flags).
/// The class of the window's first responder after a step that may move it: (step, class name).
static FIRST_RESPONDER: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static POSTED_KEYS: Mutex<Vec<(String, u16, u64)>> = Mutex::new(Vec::new());

fn set_step(label: &str) {
    *KEY_STEP.lock().unwrap() = label.to_string();
}

fn see_key(via: &'static str, kind: &str, event: Option<&NSEvent>, title: &str) {
    let (code, chars, flags) = match event {
        Some(e) => {
            let key_like = matches!(e.r#type(), NSEventType::KeyDown | NSEventType::KeyUp);
            (
                if key_like { e.keyCode() } else { 0 },
                if key_like {
                    e.charactersIgnoringModifiers()
                        .map(|s| s.to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                },
                e.modifierFlags().0 as u64 & 0xffff_0000,
            )
        }
        None => (0, String::new(), 0),
    };
    SEEN_KEYS.lock().unwrap().push(SeenKey {
        step: KEY_STEP.lock().unwrap().clone(),
        via,
        kind: kind.to_string(),
        code,
        chars,
        flags,
        title: title.to_string(),
    });
}

define_class!(
    /// The host's content view, an ANCESTOR of the plugin's view: what a DAW's own view does. It refuses the first
    /// responder (many views of a DAW do), and it takes a key that has no Command as one of its own key commands (a
    /// view above the plugin sees a key equivalent before the plugin's views do).
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "AraHostKeyView"]
    struct HostView;

    impl HostView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            false
        }
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> objc2::runtime::Bool {
            let key_like = matches!(event.r#type(), NSEventType::KeyDown);
            if key_like && event.modifierFlags().0 as u64 & FLAG_COMMAND == 0 {
                see_key("view", "equivalent", Some(event), "");
                return objc2::runtime::Bool::YES;
            }
            objc2::runtime::Bool::NO
        }
        /// The editing commands that a host view above the plugin may implement, and that delete or move data: a command that
        /// WebKit derives from a key the page did not use must not reach them while the plugin has the focus.
        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            see_key("view", "flagsChanged", Some(event), "");
        }
        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            see_key("view", "keyUp", Some(event), "");
        }
        #[unsafe(method(deleteBackward:))]
        fn delete_backward(&self, _sender: Option<&AnyObject>) {
            see_key("action", "command", None, "deleteBackward:");
        }
        #[unsafe(method(deleteWordBackward:))]
        fn delete_word_backward(&self, _sender: Option<&AnyObject>) {
            see_key("action", "command", None, "deleteWordBackward:");
        }
        #[unsafe(method(moveWordLeft:))]
        fn move_word_left(&self, _sender: Option<&AnyObject>) {
            see_key("action", "command", None, "moveWordLeft:");
        }
        // what WebKit sends to the responder chain for a key it did not use (measured: `noop:`, and `complete:` for Escape)
        #[unsafe(method(noop:))]
        fn noop(&self, _sender: Option<&AnyObject>) {
            see_key("action", "command", None, "noop:");
        }
        #[unsafe(method(complete:))]
        fn complete(&self, _sender: Option<&AnyObject>) {
            see_key("action", "command", None, "complete:");
        }
        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: Option<&AnyObject>) {
            see_key("action", "command", None, "cancelOperation:");
        }
        /// A menu action with no target that reaches this view through the responder chain.
        #[unsafe(method(hostSave:))]
        fn host_save(&self, _sender: Option<&AnyObject>) {
            see_key("menu", "command", None, "Host Save");
        }
    }
);

define_class!(
    /// A view of the host that DOES take the first responder (a text field, a button): the next key view of the plugin's
    /// view, where a Tab would go if the plugin let it.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "AraHostFocusView"]
    struct HostFocusView;

    impl HostFocusView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            see_key("focus-view", "keyDown", Some(event), "");
        }
    }
);

define_class!(
    /// The host's window: a key that nobody took ends here when the window itself is the first responder.
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[name = "AraHostKeyWindow"]
    struct HostWindow;

    impl HostWindow {
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            see_key("window", "keyDown", Some(event), "");
        }
        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            see_key("window", "keyUp", Some(event), "");
        }
        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            see_key("window", "flagsChanged", Some(event), "");
        }
    }
);

define_class!(
    /// The target of the host's key commands (its main menu): every command only records that it ran.
    #[unsafe(super(NSObject))]
    #[name = "AraHostKeyCommands"]
    struct HostCommands;

    impl HostCommands {
        #[unsafe(method(hostKey:))]
        fn host_key(&self, sender: &NSMenuItem) {
            let title = sender.title().to_string();
            // a menu command records its title: the event that made it is not read (the current event of the app may be an older one)
            see_key("menu", "command", None, &title);
        }
    }
);

/// The key commands of the host: the keys of a DAW's own commands (a clip trim, an undo, play) that the
/// plugin's keys must not trigger. Cmd+H and Cmd+Q stand for the system's keys.
const HOST_COMMANDS: [(&str, &str, u64); 11] = [
    ("Host Play", " ", 0),
    ("Host Grid", "g", 0),
    ("Host Audition", "u", 0),
    ("Host Export", "e", FLAG_COMMAND),
    ("Host Y", "y", 0),
    ("Host Cmd J", "j", FLAG_COMMAND),
    ("Host Option X", "x", FLAG_OPTION),
    ("Host Undo", "z", FLAG_COMMAND),
    ("Host Q", "q", 0),
    ("Host Hide", "h", FLAG_COMMAND),
    ("Host Quit", "q", FLAG_COMMAND),
];

fn install_host_commands(app: &NSApplication, mtm: objc2::MainThreadMarker) -> Retained<HostCommands> {
    let target: Retained<HostCommands> = unsafe { msg_send![super(HostCommands::alloc().set_ivars(())), init] };
    let bar = NSMenu::new(mtm);
    let top = NSMenuItem::new(mtm);
    let menu = NSMenu::new(mtm);
    for (title, key, mask) in HOST_COMMANDS {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &objc2_foundation::NSString::from_str(title),
                Some(sel!(hostKey:)),
                &objc2_foundation::NSString::from_str(key),
            )
        };
        item.setKeyEquivalentModifierMask(NSEventModifierFlags(mask as usize));
        unsafe { item.setTarget(Some(&target)) };
        menu.addItem(&item);
    }
    top.setSubmenu(Some(&menu));
    bar.addItem(&top);
    app.setMainMenu(Some(&bar));
    target
}

fn key_record_row() -> String {
    let posted: Vec<serde_json::Value> = POSTED_KEYS
        .lock()
        .unwrap()
        .iter()
        .map(|(l, c, f)| serde_json::json!({"step": l, "code": c, "flags": f}))
        .collect();
    let seen: Vec<serde_json::Value> = SEEN_KEYS
        .lock()
        .unwrap()
        .iter()
        .map(|k| {
            serde_json::json!({"step": k.step, "via": k.via, "kind": k.kind, "code": k.code, "chars": k.chars,
                "flags": k.flags, "title": k.title})
        })
        .collect();
    let responders: Vec<serde_json::Value> = FIRST_RESPONDER
        .lock()
        .unwrap()
        .iter()
        .map(|(l, c)| serde_json::json!({"step": l, "class": c}))
        .collect();
    serde_json::json!({"event": "key_record", "posted": posted, "seen": seen, "first_responder": responders})
        .to_string()
}

/// The steps of the `hostkeys` mode (M1b12), one every `HK_GAP_MS` after the page is ready. Each kind of key of the
/// ticket: a page key, an absent command's key, a key with no command, a key with a modifier, the space bar, the
/// same with a text field of the page in focus, the system keys, and a click out of the plugin and back in.
const HK_GAP_MS: u64 = 900;

fn hostkeys_steps() -> Vec<Action> {
    use Action::*;
    const SHIFT: u64 = 1 << 17;
    const CONTROL: u64 = 1 << 18;
    vec![
        Hk("space", K_SPACE, " ", " ", 0),
        HkRepeat("space-repeat", K_SPACE, " ", " ", 0),
        Hk("page-key", 5, "g", "g", 0),
        Hk("page-add-hit", 4, "h", "h", 0),
        Hk("absent-key", 32, "u", "u", 0),
        // Export (Cmd+E) is a command of the DAW page since #560; Export Again (Option+Cmd+E) is still absent in host mode
        Hk("absent-key-cmd", 14, "\u{b4}", "e", FLAG_COMMAND | FLAG_OPTION),
        Hk("no-command", 16, "y", "y", 0),
        Hk("no-command-shift", 16, "Y", "Y", SHIFT),
        Hk("no-command-cmd", 38, "j", "j", FLAG_COMMAND),
        Hk("no-command-option", 7, "\u{2248}", "x", FLAG_OPTION),
        Hk("page-key-cmd", K_Z, "z", "z", FLAG_COMMAND),
        Hk("escape", K_ESCAPE, "\u{1b}", "\u{1b}", 0),
        Hk("return", 36, "\r", "\r", 0),
        Hk("arrow-left", 123, "\u{f702}", "\u{f702}", 0),
        Hk("function-key", 96, "\u{f708}", "\u{f708}", 0),
        Hk("control-key", 37, "l", "l", CONTROL),
        Hk("cmd-s", 1, "s", "s", FLAG_COMMAND),
        Hk("cmd-w", 13, "w", "w", FLAG_COMMAND),
        HkSlow("slow-delete", 51, "\u{7f}", "\u{7f}", FLAG_OPTION),
        HkSlow("slow-move", 123, "\u{f702}", "\u{f702}", FLAG_OPTION),
        Hk("marker-field-on", K_F13, "\u{f710}", "\u{f710}", 0),
        Hk("field-letter", 32, "u", "u", 0),
        Hk("field-space", K_SPACE, " ", " ", 0),
        Hk("field-cmd-absent", 14, "\u{b4}", "e", FLAG_COMMAND | FLAG_OPTION),
        Hk("field-cmd-none", 38, "j", "j", FLAG_COMMAND),
        Hk("field-option-at", 37, "@", "l", FLAG_OPTION),
        Hk("field-option-lbracket", 23, "[", "5", FLAG_OPTION),
        Hk("field-option-rbracket", 22, "]", "6", FLAG_OPTION),
        Hk("field-delete", 51, "\u{7f}", "\u{7f}", 0),
        Hk("field-undo", K_Z, "z", "z", FLAG_COMMAND),
        Hk("field-tab", K_TAB, "\t", "\t", 0),
        Hk("field-tab-2", K_TAB, "\t", "\t", 0),
        Hk("field-tab-3", K_TAB, "\t", "\t", 0),
        Hk("field-tab-4", K_TAB, "\t", "\t", 0),
        Hk("after-tab", 16, "y", "y", 0),
        Hk("marker-field-off", K_F14, "\u{f711}", "\u{f711}", 0),
        Menu("menu-action"),
        Hk("system-hide", 4, "h", "h", FLAG_COMMAND),
        Hk("system-quit", 12, "q", "q", FLAG_COMMAND),
        Mouse("click-out", 50.0, 300.0),
        Hk("given-back", 12, "q", "q", 0),
        Flags("mod-down", FLAG_COMMAND),
        Mouse("click-in", 560.0, 300.0),
        Flags("mod-up", 0),
        Hk("taken", 12, "q", "q", 0),
        Mouse("click-overlay", 940.0, 540.0),
        Hk("overlay-key", 12, "q", "q", 0),
        Mouse("click-in-again", 560.0, 300.0),
        Hk("marker-end", K_F12, "\u{f70f}", "\u{f70f}", 0),
    ]
}

fn schedule_of(mode: &str) -> Vec<(u64, Action)> {
    match mode {
        "keys" => vec![
            (2500, Action::Key(K_SPACE, 0)),
            (3000, Action::Key(K_RIGHT, 0)),
            (3500, Action::Key(K_Z, FLAG_COMMAND)),
            (4000, Action::Key(K_TAB, 0)),
            (4500, Action::Key(K_ESCAPE, 0)),
        ],
        "focus" => vec![
            (2500, Action::ClickOut),
            (3500, Action::Key(K_SPACE, 0)),
            (4500, Action::ClickIn),
            (5500, Action::Key(K_SPACE, 0)),
        ],
        _ => vec![],
    }
}

fn chars_of(code: Key) -> &'static str {
    match code {
        K_SPACE => " ",
        K_TAB => "\t",
        K_ESCAPE => "\u{1b}",
        K_RIGHT => "\u{f703}",
        _ => "z",
    }
}

fn post_key(code: Key, flags: u64, app: &NSApplication, window: &NSWindow) {
    post_key_chars(code, chars_of(code), chars_of(code), flags, false, app, window);
}

/// A real key down and key up through `[NSApp sendEvent:]` to the window of this host (never a global event).
fn post_key_chars(
    code: Key,
    chars: &str,
    ignoring: &str,
    flags: u64,
    repeat: bool,
    app: &NSApplication,
    window: &NSWindow,
) {
    let chars = objc2_foundation::NSString::from_str(chars);
    let ignoring = objc2_foundation::NSString::from_str(ignoring);
    let none = objc2_foundation::NSString::from_str("");
    let stamp = || objc2_foundation::NSProcessInfo::processInfo().systemUptime();
    // a real keyboard sends a change of the modifier keys before and after a key with a modifier
    let modifier_code: Option<Key> = [(FLAG_COMMAND, 55), (FLAG_OPTION, 58), (1 << 18, 59), (1 << 17, 56)]
        .iter()
        .find(|(f, _)| flags & f != 0)
        .map(|(_, c)| *c);
    let flags_changed = |now: u64| {
        if let Some(c) = modifier_code {
            if let Some(ev) = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::FlagsChanged, NSPoint::new(0.0, 0.0), NSEventModifierFlags(now as usize), stamp(), window.windowNumber(), None, &none, &none, false, c,
            ) {
                app.sendEvent(&ev);
            }
        }
    };
    flags_changed(flags);
    for ty in [NSEventType::KeyDown, NSEventType::KeyUp] {
        let ev = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            ty, NSPoint::new(0.0, 0.0), NSEventModifierFlags(flags as usize), objc2_foundation::NSProcessInfo::processInfo().systemUptime(),
            window.windowNumber(), None, &chars, &ignoring, repeat, code,
        );
        if let Some(ev) = ev {
            app.sendEvent(&ev);
        }
    }
    flags_changed(0);
}

fn post_click(app: &NSApplication, window: &NSWindow) {
    let f = window.frame();
    post_click_window(app, window, f.size.width / 2.0, (f.size.height - 28.0) / 2.0);
}

/// A real mouse click at a point of the window (window coordinates), to the window of this host.
fn post_click_window(app: &NSApplication, window: &NSWindow, x: f64, y: f64) {
    let at = NSPoint::new(x, y);
    for (ty, pressure) in [(NSEventType::LeftMouseDown, 1.0), (NSEventType::LeftMouseUp, 0.0)] {
        let ev = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            ty, at, NSEventModifierFlags(0), objc2_foundation::NSProcessInfo::processInfo().systemUptime(), window.windowNumber(), None, 1, 1, pressure,
        );
        if let Some(ev) = ev {
            app.sendEvent(&ev);
        }
    }
}

/// A click of the pointer at a place of the page (CSS pixels from the top left of the web view, which fills the
/// content view): `count` presses and releases, the click count of each one rising, as a user's double click does
/// (M1b11: the finding of a hit made with the pointer in a real web view).
fn post_click_at(app: &NSApplication, window: &NSWindow, x: f64, y: f64, count: isize) {
    // the event number rises with every event: a web view drops an event whose number it has seen
    static EVENT_NO: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(5000);
    let Some(view) = window.contentView() else { return };
    let at = view.convertPoint_toView(NSPoint::new(x, view.bounds().size.height - y), None);
    // the pointer arrives first (a user moves it before the press): a web view takes the first press at a new place as
    // the move that makes it the target
    let moved = |app: &NSApplication| {
        let mv = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            NSEventType::MouseMoved, at, NSEventModifierFlags(0), objc2_foundation::NSProcessInfo::processInfo().systemUptime(), window.windowNumber(), None,
            EVENT_NO.fetch_add(1, Ordering::SeqCst), 0, 0.0,
        );
        if let Some(ev) = mv {
            app.postEvent_atStart(&ev, false);
        }
        pump(app);
    };
    for _ in 0..12 {
        moved(app);
    }
    for n in 1..=count {
        for (ty, pressure) in [(NSEventType::LeftMouseDown, 1.0), (NSEventType::LeftMouseUp, 0.0)] {
            let ev = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                ty, at, NSEventModifierFlags(0), objc2_foundation::NSProcessInfo::processInfo().systemUptime(), window.windowNumber(), None,
                EVENT_NO.fetch_add(1, Ordering::SeqCst), n, pressure,
            );
            if let Some(ev) = ev {
                app.postEvent_atStart(&ev, false);
            }
            pump(app);
        }
    }
    // and the pointer moves on: a web view hands the release to the page with the next event
    for _ in 0..6 {
        moved(app);
    }
}

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

/// The rows of the plugin's report file (it is rewritten atomically).
fn rows() -> Vec<serde_json::Value> {
    let Some(path) = std::env::var("DS_ARA_OUT").ok().filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default()
}

fn count_rows(event: &str, kind: Option<&str>) -> usize {
    rows()
        .iter()
        .filter(|r| r["event"] == event && kind.is_none_or(|k| r["kind"] == k))
        .count()
}

fn rss_mb() -> f64 {
    let out = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    out.trim().parse::<f64>().unwrap_or(0.0) / 1024.0
}

fn arg_flag(rest: &[String], name: &str) -> bool {
    rest.iter().any(|a| a == name)
}

pub fn run(bundle: &str, files: &[String], rest: &[String]) -> i32 {
    let mode = args_value(rest, "--editor-mode").unwrap_or_else(|| "ready".into());
    let cycles: usize = args_value(rest, "--cycles").and_then(|v| v.parse().ok()).unwrap_or(1);
    let archive = args_value(rest, "--archive");
    let no_ara = arg_flag(rest, "--no-ara");
    let late_files = args_list(rest, "--late-files");
    let wait_sync = arg_flag(rest, "--wait-sync");
    let store_at_edit = args_value(rest, "--store-at-edit");
    // M1b10 (mode `names`): after the page's first report the host renames region sequences
    // (`--rename-seq <j>=<name>`) and moves regions to other sequences (`--move-region <i>=<j>`)
    let renames: Vec<(usize, String)> = args_list(rest, "--rename-seq")
        .iter()
        .filter_map(|v| {
            v.split_once('=')
                .and_then(|(j, n)| Some((j.parse().ok()?, n.to_string())))
        })
        .collect();
    let moves: Vec<(usize, usize)> = args_list(rest, "--move-region")
        .iter()
        .filter_map(|v| {
            v.split_once('=')
                .and_then(|(i, j)| Some((i.parse().ok()?, j.parse().ok()?)))
        })
        .collect();
    // M1c1: `--shift-region <i>=<song seconds>` moves region i in playback time only (no new analysis round), in mode
    // `delivery` after the page's report `delivery-ready`
    let shifts: Vec<(usize, f64)> = args_list(rest, "--shift-region")
        .iter()
        .filter_map(|v| {
            v.split_once('=')
                .and_then(|(i, p)| Some((i.parse().ok()?, p.parse().ok()?)))
        })
        .collect();
    // `--trim-region <i>=<start>,<length>`: region i keeps the source at the same song time but shows only that slice
    let trims: Vec<(usize, f64, f64)> = args_list(rest, "--trim-region")
        .iter()
        .filter_map(|v| {
            let (i, r) = v.split_once('=')?;
            let (a, d) = r.split_once(',')?;
            Some((i.parse().ok()?, a.parse().ok()?, d.parse().ok()?))
        })
        .collect();
    let region_spec = args_value(rest, "--region").and_then(|v| parse_region(&v).ok());
    // M1d1 (mode `leftovers`): after the page's `leftovers` report the host replaces the audio of source k (`--update-content-after <k>:<wav>`)
    let update_after: Option<(usize, String)> = args_value(rest, "--update-content-after").and_then(|v| {
        v.split_once(':')
            .and_then(|(k, p)| Some((k.parse().ok()?, p.to_string())))
    });
    // M1b13 (mode `selection`): the steps of the host's view, `[<pre action of the page>@]<host step>` separated by `;`. The page does the
    // pre action (a pointer or key held down, the switch in the menu, a zoom) and then reports `sel-armed`; the host sends its step, the
    // page reports `sel-snap` with what it shows. The host step is described at `view_step`.
    let sel_steps: Vec<String> = args_value(rest, "--sel-steps")
        .map(|v| v.split(';').filter(|x| !x.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    // `--sel-pre <step>;<step>`: sent before the editor opens (the page must take it from the plugin when it opens)
    let sel_pre: Vec<String> = args_value(rest, "--sel-pre")
        .map(|v| v.split(';').filter(|x| !x.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    let mut sel_sent: usize;
    let mut view_handle: Option<ViewHandle> = None;
    let destroy_late: Option<usize> = args_value(rest, "--destroy-late").and_then(|v| v.parse().ok());
    let archive_window = args_value(rest, "--archive-window");
    // M1b7: the playback controller of the host (`--playback full|ignore|partial`, default none) and the
    // grid that a position request snaps to (`--snap <s>`). The mode must be set before the document
    // is made: the controller is part of the host instance.
    let playback = args_value(rest, "--playback").unwrap_or_else(|| "none".into());
    let snap: f64 = args_value(rest, "--snap").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    set_playback(
        match playback.as_str() {
            "full" => PbMode::Full,
            "ignore" => PbMode::Ignore,
            "partial" => PbMode::Partial,
            _ => PbMode::None,
        },
        snap,
    );
    set_playback_delay(
        args_value(rest, "--playback-delay")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    );
    std::env::set_var("DS_ARA_EDITOR_MODE", &mode);
    if let Some(a) = args_value(rest, "--arg") {
        std::env::set_var("DS_ARA_EDITOR_ARG", a);
    }
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        eprintln!("editor: the host must run on the main thread");
        return 2;
    };
    unsafe {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let rect = NSRect::new(NSPoint::new(120.0, 120.0), NSSize::new(1000.0, 640.0));
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
        let window: Retained<NSWindow> = if mode == "hostkeys" {
            // M1b12: the key stages use a window that records the keys that end at it
            let w: Retained<HostWindow> = msg_send![HostWindow::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false];
            Retained::into_super(w)
        } else {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setReleasedWhenClosed(false);
        // M1b12: the key stages give the host a content view and a main menu that record every key that reaches them
        let mut slot_view: Option<Retained<NSView>> = None;
        let _host_commands = if mode == "hostkeys" {
            let hv: Retained<HostView> = msg_send![HostView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1000.0, 640.0))];
            window.setContentView(Some(&hv));
            // the plugin gets the right part of the window; the left strip (120 pt) is the host's own area
            let slot = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(120.0, 0.0), NSSize::new(880.0, 640.0)),
            );
            hv.addSubview(&slot);
            // a host view that takes the focus, at the top of the left strip (the click outside lands lower)
            let focus: Retained<HostFocusView> = msg_send![HostFocusView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::new(10.0, 500.0), NSSize::new(100.0, 100.0))];
            hv.addSubview(&focus);
            // a host view OVER a part of the plugin (an overlay, a resize grip): a plain view, it refuses the first responder
            let overlay = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(900.0, 500.0), NSSize::new(80.0, 80.0)),
            );
            hv.addSubview(&overlay);
            slot_view = Some(slot);
            Some(install_host_commands(&app, mtm))
        } else {
            None
        };
        window.makeKeyAndOrderFront(None);
        window.orderFrontRegardless();
        window.setLevel(3);
        app.finishLaunching();
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);

        let mut shown = false;
        for _ in 0..25 {
            pump(&app);
        }
        // the page is "visible" to WebKit only while its window is not occluded
        eprintln!(
            "editor: window visible={} occlusion={:?} key={} app_active={}",
            window.isVisible(),
            window.occlusionState(),
            window.isKeyWindow(),
            app.isActive()
        );
        shown |= window.isVisible();
        let _ = shown;
        let rss_closed = rss_mb();
        let lib = match load(bundle) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let mut doc = match HostDoc::new(&lib) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        if !files.is_empty() && mode != "open-first" {
            if let Err(e) = doc.add_and_analyse(files, "editor") {
                eprintln!("{e}");
                return 2;
            }
        }
        let plugin = match clap_plugin_of(&lib) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        if !no_ara {
            let ext_id = CString::new(&ARA_PLUGIN_EXT_ID[..ARA_PLUGIN_EXT_ID.len() - 1]).unwrap();
            let ext = ((*plugin).get_extension.expect("get_extension"))(plugin, ext_id.as_ptr());
            if ext.is_null() {
                eprintln!("no ARA plugin extension");
                return 2;
            }
            let aext = ext as *const drumspike_ara::clap_ara_plugin_extension;
            let roles: ARAPlugInInstanceRoleFlags =
                (kARAPlaybackRendererRole | kARAEditorRendererRole | kARAEditorViewRole) as ARAPlugInInstanceRoleFlags;
            let pext = ((*aext).bind_to_document_controller.expect("bind"))(plugin, dc_ref_of(doc.inst), roles, roles);
            if pext.is_null() {
                eprintln!("bind_to_document_controller returned null");
                return 2;
            }
            view_handle = Some(ViewHandle {
                view: std::ptr::addr_of!((*pext).editorViewRef).read_unaligned(),
                iface: std::ptr::addr_of!((*pext).editorViewInterface).read_unaligned(),
            });
        }
        if let Some(h) = view_handle {
            for spec in &sel_pre {
                view_step(&doc, h, spec);
            }
        }
        let gui_id = CString::new("clap.gui").unwrap();
        let gui = ((*plugin).get_extension.expect("get_extension"))(plugin, gui_id.as_ptr()) as *const clap_plugin_gui;
        if gui.is_null() {
            eprintln!("the plugin has no clap.gui extension");
            return 2;
        }
        let cocoa = CString::new("cocoa").unwrap();

        // the audio thread of the `analyse` mode: blocks while the page's analysis runs
        let audio_stop = std::sync::Arc::new(AtomicBool::new(false));
        let audio_stats = std::sync::Arc::new(Mutex::new((0usize, 0usize, 0u128, 0u128)));
        let mut audio_thread = None;
        if mode == "analyse" || mode == "analyse-race" {
            ((*plugin).activate.expect("activate"))(plugin, SAMPLE_RATE, 64, 4096);
            ((*plugin).start_processing.expect("start_processing"))(plugin);
            let p = plugin as usize;
            let (stop, stats) = (audio_stop.clone(), audio_stats.clone());
            audio_thread = Some(std::thread::spawn(move || {
                let plugin = p as *const clap_plugin;
                let mut input = vec![0.0f32; FRAMES];
                let mut output = vec![0.0f32; FRAMES];
                let in_buf = clap_sys::audio_buffer::clap_audio_buffer {
                    data32: &mut input.as_mut_ptr(),
                    data64: std::ptr::null_mut(),
                    channel_count: 1,
                    latency: 0,
                    constant_mask: 0,
                };
                let mut out_buf = clap_sys::audio_buffer::clap_audio_buffer {
                    data32: &mut output.as_mut_ptr(),
                    data64: std::ptr::null_mut(),
                    channel_count: 1,
                    latency: 0,
                    constant_mask: 0,
                };
                let in_events = clap_sys::events::clap_input_events {
                    ctx: std::ptr::null_mut(),
                    size: Some(in_events_size),
                    get: Some(in_events_get),
                };
                let out_events = clap_sys::events::clap_output_events {
                    ctx: std::ptr::null_mut(),
                    try_push: Some(out_events_try_push),
                };
                let budget = Duration::from_secs_f64(FRAMES as f64 / SAMPLE_RATE);
                let mut n = 0i64;
                while !stop.load(Ordering::SeqCst) {
                    let process = clap_process {
                        steady_time: n,
                        frames_count: FRAMES as u32,
                        transport: std::ptr::null(),
                        audio_inputs: &in_buf,
                        audio_outputs: &mut out_buf,
                        audio_inputs_count: 1,
                        audio_outputs_count: 1,
                        in_events: &in_events,
                        out_events: &out_events,
                    };
                    let t0 = Instant::now();
                    let _ = ((*plugin).process.expect("process"))(plugin, &process);
                    let took = t0.elapsed();
                    if let Ok(mut s) = stats.lock() {
                        s.0 += 1;
                        if took > budget {
                            s.1 += 1;
                        }
                        s.2 = s.2.max(took.as_micros());
                        s.3 = budget.as_micros();
                    }
                    n += 1;
                    // a real host paces its blocks; this one runs at a tenth of real time
                    std::thread::sleep(budget / 10);
                }
            }));
        }

        // the transport thread of the `--transport` option (M1b5): starts when the page has booted
        let transport_kind = args_value(rest, "--transport").unwrap_or_else(|| "none".into());
        let transport_stop = std::sync::Arc::new(AtomicBool::new(false));
        let mut transport_thread: Option<std::thread::JoinHandle<Vec<Segment>>> = None;

        let mut opened = 0usize;
        let mut failed: Option<String> = None;
        let mut rss_open = 0.0;
        let mut window_visible = false;
        let ready_limit = Duration::from_secs(25);
        // mode `deliver` (M1c3) waits for the first use of the wav kinds, an edit and the files after it: minutes, not one
        let phase_limit = Duration::from_secs(if mode == "deliver" { 300 } else { 60 });
        for round in 1..=cycles {
            if !((*gui).create.expect("create"))(plugin, cocoa.as_ptr(), false) {
                failed = Some("gui create refused".into());
                break;
            }
            let parent_view = slot_view
                .as_ref()
                .map(|v| (&**v) as *const NSView as *mut c_void)
                .or_else(|| window.contentView().map(|v| (&*v) as *const NSView as *mut c_void))
                .unwrap_or(std::ptr::null_mut());
            let parent = clap_window {
                api: cocoa.as_ptr(),
                specific: clap_window_handle { cocoa: parent_view },
            };
            if parent_view.is_null() || !((*gui).set_parent.expect("set_parent"))(plugin, &parent) {
                failed = Some("gui set_parent refused (the web view did not build)".into());
                break;
            }
            if !((*gui).show.expect("show"))(plugin) {
                failed = Some("gui show refused".into());
                break;
            }
            if mode == "hostkeys" && !((*gui).show.expect("show"))(plugin) {
                // a host may call show again while the page has the focus
                failed = Some("the second gui show refused".into());
                break;
            }
            let ready_before = count_rows("page_ready", None);
            let want_kind: Option<&str> = match mode.as_str() {
                "keys" | "focus" | "resize" | "measure" | "shell" | "analyse" | "bigpost" | "refuse" | "transport"
                | "playback" | "hit" | "menu" | "geometry" | "hostkeys" | "delivery" | "selection" | "exportdialog"
                | "deliver" => Some(mode.as_str()),
                // the edit mode ends with the event report that follows the edit report
                "edit" => Some("edit-events"),
                "analyse-race" => Some("analyse"),
                "edit-round" | "open-first" | "edit-region" => Some("follow"),
                // the page reports the names it got, the host renames and moves, the page reports again
                "names" => Some("names-after"),
                "leftovers" => Some("leftovers-after"),
                // M1b8: the page reports the rows it got, the host renames, moves, shifts and trims, the page reports again
                "song" => Some("song-after"),
                "edit-destroy" => Some("edit"),
                "net" => Some("now"),
                _ => None,
            };
            let reports_before = want_kind.map_or(0, |k| count_rows("page_report", Some(k)));
            let boots_before = count_rows("page_report", Some("boot"));
            let names_before = count_rows("page_report", Some("names"));
            let leftovers_before = count_rows("page_report", Some("leftovers"));
            let song_before = count_rows("page_report", Some("song"));
            let mut song_handled = false;
            let mut update_handled = false;
            let mut names_handled = false;
            let mut shift_handled = false;
            let sel_armed_before = count_rows("page_report", Some("sel-armed"));
            let sel_snap_before = count_rows("page_report", Some("sel-snap"));
            sel_sent = 0;
            let delivery_ready_before = count_rows("page_report", Some("delivery-ready"));
            let edits_before = count_rows("page_report", Some("edit"));
            let refuse_before = count_rows("page_report", Some("refuse"));
            let started_before = count_rows("page_report", Some("analyse-started"));
            let mut late_done = false;
            let mut edit_handled = false;
            let mut destroyed_jobs: Option<usize> = None;
            if round == 1 {
                // WebKit draws a page only while its window is not occluded: a locked screen
                // hides every window, and the page is then suspended
                for _ in 0..20 {
                    pump(&app);
                }
                window_visible = window.occlusionState().0 & 2 != 0;
            }
            let t0 = Instant::now();
            let limit = if mode == "measure" && !window_visible {
                Duration::from_secs(20)
            } else if want_kind.is_some() {
                phase_limit
            } else {
                ready_limit
            };
            let mut schedule = schedule_of(&mode);
            // M1b12: the key steps start when the page says it is ready (its transport is wired)
            let hk_ready_before = count_rows("page_report", Some("hostkeys-ready"));
            let mut hk_started = false;
            let mut resized = 0usize;
            let mut clicks_done = 0usize;
            let mut saved_resp: Option<Retained<NSResponder>> = None;
            let mut got = false;
            while t0.elapsed() < limit {
                let elapsed = t0.elapsed().as_millis() as u64;
                let due: Vec<Action> = schedule
                    .iter()
                    .filter(|(at, _)| *at <= elapsed)
                    .map(|(_, a)| *a)
                    .collect();
                schedule.retain(|(at, _)| *at > elapsed);
                if mode == "hostkeys"
                    && !hk_started
                    && count_rows("page_report", Some("hostkeys-ready")) > hk_ready_before
                {
                    hk_started = true;
                    schedule = hostkeys_steps()
                        .into_iter()
                        .enumerate()
                        .map(|(i, a)| (elapsed + 600 + i as u64 * HK_GAP_MS, a))
                        .collect();
                }
                if !due.is_empty() && mode == "hostkeys" {
                    // the first responder after the step before (a Tab may have moved it)
                    let prev = KEY_STEP.lock().unwrap().clone();
                    if !prev.is_empty() {
                        let first = window
                            .firstResponder()
                            .map(|r| r.class().name().to_string_lossy().into_owned())
                            .unwrap_or_default();
                        FIRST_RESPONDER.lock().unwrap().push((prev, first));
                    }
                }
                for a in due {
                    match a {
                        Action::Key(code, flags) => post_key(code, flags, &app, &window),
                        Action::Hk(label, code, chars, ignoring, flags) => {
                            set_step(label);
                            POSTED_KEYS.lock().unwrap().push((label.to_string(), code, flags));
                            post_key_chars(code, chars, ignoring, flags, false, &app, &window);
                        }
                        Action::HkSlow(label, code, chars, ignoring, flags) => {
                            set_step(label);
                            POSTED_KEYS.lock().unwrap().push((label.to_string(), code, flags));
                            post_key_chars(code, chars, ignoring, flags, false, &app, &window);
                            std::thread::sleep(Duration::from_millis(400));
                        }
                        Action::Flags(label, flags) => {
                            set_step(label);
                            POSTED_KEYS.lock().unwrap().push((label.to_string(), 0, flags));
                            if let Some(ev) = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                                NSEventType::FlagsChanged,
                                NSPoint::new(0.0, 0.0),
                                NSEventModifierFlags(flags as usize),
                                objc2_foundation::NSProcessInfo::processInfo().systemUptime(),
                                window.windowNumber(),
                                None,
                                &objc2_foundation::NSString::from_str(""),
                                &objc2_foundation::NSString::from_str(""),
                                false,
                                55,
                            ) {
                                app.sendEvent(&ev);
                            }
                        }
                        Action::HkRepeat(label, code, chars, ignoring, flags) => {
                            set_step(label);
                            POSTED_KEYS.lock().unwrap().push((label.to_string(), code, flags));
                            post_key_chars(code, chars, ignoring, flags, true, &app, &window);
                        }
                        Action::Mouse(label, x, y) => {
                            set_step(label);
                            POSTED_KEYS.lock().unwrap().push((label.to_string(), 0, 0));
                            post_click_window(&app, &window, x, y);
                        }
                        Action::Menu(label) => {
                            set_step(label);
                            POSTED_KEYS.lock().unwrap().push((label.to_string(), 0, 0));
                            // what a click on a menu item with no target sends
                            // the action comes later than 2 s after the last key of the plugin (a menu click long after a key)
                            std::thread::sleep(Duration::from_millis(2200));
                            // the action goes down the responder chain from the first responder, as the application does for a menu item with no target
                            let done: bool = match window.firstResponder() {
                                Some(r) => {
                                    msg_send![&*r, tryToPerform: sel!(hostSave:), with: std::ptr::null::<AnyObject>()]
                                }
                                None => false,
                            };
                            FIRST_RESPONDER
                                .lock()
                                .unwrap()
                                .push(("menu-action-sent".to_string(), format!("{done}")));
                        }
                        Action::ClickOut => {
                            set_step("click-out");
                            POSTED_KEYS.lock().unwrap().push(("click-out".into(), 0, 0));
                            saved_resp = window.firstResponder();
                            if let Some(cv) = window.contentView() {
                                window.makeFirstResponder(Some(&*cv));
                            }
                        }
                        Action::ClickIn => {
                            set_step("click-in");
                            POSTED_KEYS.lock().unwrap().push(("click-in".into(), 0, 0));
                            post_click(&app, &window);
                            if let Some(r) = &saved_resp {
                                window.makeFirstResponder(Some(&**r));
                            }
                        }
                    }
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
                if matches!(mode.as_str(), "hit" | "menu" | "geometry") {
                    // M1b11: the driver of the page asks the host for what only a host can do: a window of another
                    // size (below and above the 520 px of the low window) and a real click of the pointer
                    if resized == 0 && count_rows("page_report", Some("host-low")) > 0 {
                        window.setFrame_display(
                            NSRect::new(NSPoint::new(120.0, 120.0), NSSize::new(1100.0, 420.0)),
                            true,
                        );
                        resized = 1;
                    } else if resized == 1 && count_rows("page_report", Some("host-high")) > 0 {
                        window.setFrame_display(
                            NSRect::new(NSPoint::new(120.0, 120.0), NSSize::new(1100.0, 860.0)),
                            true,
                        );
                        resized = 2;
                    }
                    let asked = count_rows("page_report", Some("host-click"));
                    if asked > clicks_done {
                        clicks_done = asked;
                        if let Some(r) = rows()
                            .iter()
                            .rev()
                            .find(|r| r["event"] == "page_report" && r["kind"] == "host-click")
                        {
                            let (x, y) = (
                                r["report"]["x"].as_f64().unwrap_or(0.0),
                                r["report"]["y"].as_f64().unwrap_or(0.0),
                            );
                            let n = r["report"]["count"].as_i64().unwrap_or(1) as isize;
                            post_click_at(&app, &window, x, y, n);
                        }
                    }
                }
                if !edit_handled && count_rows("page_report", Some("edit")) > edits_before && mode.starts_with("edit") {
                    edit_handled = true;
                    if let Some(path) = &store_at_edit {
                        // the archive is stored the moment the page's edit is answered
                        match doc.store() {
                            Ok(blob) => {
                                let _ = std::fs::write(path, &blob);
                                println!("{{\"event\":\"stored_at_edit\",\"bytes\":{}}}", blob.len());
                            }
                            Err(e) => {
                                failed = Some(e);
                                break;
                            }
                        }
                    }
                    if mode == "edit-destroy" {
                        got = true;
                        break; // the editor and the document close at once
                    }
                    if mode == "edit-region" {
                        let name = rows()
                            .iter()
                            .rev()
                            .find(|r| r["event"] == "page_report" && r["kind"] == "edit")
                            .and_then(|r| r["report"]["name"].as_str().map(str::to_string))
                            .unwrap_or_default();
                        let at = files.iter().position(|f| stem_safe(f) == name);
                        if let (Some(i), Some((a, d, p))) = (at, region_spec) {
                            let jobs_before = count_rows("job", None);
                            update_region(&doc, i, a, d, p);
                            let t = Instant::now();
                            while t.elapsed() < Duration::from_secs(30) && count_rows("job", None) <= jobs_before {
                                pump(&app);
                                doc_pump(&doc);
                            }
                        } else {
                            failed = Some(format!("no source for the edited track {name:?} or no --region"));
                            break;
                        }
                    }
                }
                if mode == "delivery"
                    && !shift_handled
                    && count_rows("page_report", Some("delivery-ready")) > delivery_ready_before
                {
                    shift_handled = true;
                    for (i, p) in &shifts {
                        shift_region(doc.inst, doc.state, *i, *p);
                    }
                    for (i, a, d) in &trims {
                        trim_region(doc.inst, doc.state, *i, *a, *d);
                    }
                }
                if mode == "selection" {
                    // M1b13: the host sends step k when the page is armed for it (its pre action is done), and the next one when
                    // the page has reported what it showed for step k
                    let armed = count_rows("page_report", Some("sel-armed")) - sel_armed_before;
                    let snapped = count_rows("page_report", Some("sel-snap")) - sel_snap_before;
                    if sel_sent < sel_steps.len() && armed > sel_sent && snapped >= sel_sent {
                        let step = &sel_steps[sel_sent];
                        let spec = step.split_once('@').map_or(step.as_str(), |(_, h)| h);
                        match view_handle {
                            Some(h) => view_step(&doc, h, spec),
                            None => {
                                failed = Some("the selection mode needs the ARA view of the plugin".into());
                                break;
                            }
                        }
                        sel_sent += 1;
                    }
                }
                if mode == "leftovers"
                    && !update_handled
                    && count_rows("page_report", Some("leftovers")) > leftovers_before
                {
                    update_handled = true;
                    if let Some((k, path)) = &update_after {
                        if let Err(e) = update_source_content(&doc, *k, path) {
                            failed = Some(e);
                            break;
                        }
                    }
                }
                if mode == "names" && !names_handled && count_rows("page_report", Some("names")) > names_before {
                    names_handled = true;
                    for (j, name) in &renames {
                        rename_sequence(doc.inst, doc.state, *j, name);
                    }
                    for (i, j) in &moves {
                        move_region(doc.inst, doc.state, *i, *j);
                    }
                }
                if mode == "song" && !song_handled && count_rows("page_report", Some("song")) > song_before {
                    song_handled = true;
                    for (j, name) in &renames {
                        rename_sequence(doc.inst, doc.state, *j, name);
                    }
                    for (i, j) in &moves {
                        move_region(doc.inst, doc.state, *i, *j);
                    }
                    for (i, p) in &shifts {
                        shift_region(doc.inst, doc.state, *i, *p);
                    }
                    for (i, a, d) in &trims {
                        trim_region(doc.inst, doc.state, *i, *a, *d);
                    }
                }
                if mode == "analyse-race" && destroyed_jobs.is_none() {
                    if let Some(i) = destroy_late {
                        if count_rows("page_report", Some("analyse-started")) > started_before {
                            destroyed_jobs = Some(count_rows("job", None));
                            if let Err(e) = doc.destroy_source(i) {
                                failed = Some(e);
                                break;
                            }
                            if let Some(path) = &archive_window {
                                // a store while the page's job still runs and the track is not gone yet
                                match doc.store() {
                                    Ok(blob) => {
                                        let _ = std::fs::write(path, &blob);
                                        println!("{{\"event\":\"stored_in_window\",\"bytes\":{}}}", blob.len());
                                    }
                                    Err(e) => {
                                        failed = Some(e);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                if !late_done && !late_files.is_empty() {
                    let trigger = match mode.as_str() {
                        "open-first" => count_rows("page_report", Some("boot")) > boots_before,
                        "edit-round" => count_rows("page_report", Some("edit")) > edits_before,
                        "refuse" => count_rows("page_report", Some("refuse")) > refuse_before,
                        "analyse-race" => count_rows("page_report", Some("analyse-started")) > started_before,
                        _ => false,
                    };
                    if trigger {
                        late_done = true;
                        // the editor stays alive while the host works (its page waits)
                        let jobs_before = count_rows("job", None);
                        if let Err(e) = doc.add_and_analyse(&late_files, "late") {
                            failed = Some(e);
                            break;
                        }
                        // the source is complete a moment before the plugin writes the row of its
                        // round: wait for the row, so the report is whole when the run ends
                        let t = Instant::now();
                        while t.elapsed() < Duration::from_secs(20) && count_rows("job", None) <= jobs_before {
                            pump(&app);
                        }
                        if count_rows("job", None) <= jobs_before {
                            failed = Some("the plugin wrote no row for the round of the late files in 20 s".into());
                            break;
                        }
                    }
                }
                if transport_thread.is_none()
                    && transport_kind != "none"
                    && count_rows("page_report", Some("boot")) > boots_before
                {
                    ((*plugin).activate.expect("activate"))(plugin, SAMPLE_RATE, 64, 4096);
                    ((*plugin).start_processing.expect("start_processing"))(plugin);
                    transport_thread = Some(spawn_transport(
                        plugin as usize,
                        &transport_kind,
                        transport_stop.clone(),
                    ));
                }
                pump(&app);
                let ready_now = count_rows("page_ready", None) > ready_before;
                let done = match want_kind {
                    Some("analyse") if destroy_late.is_some() => {
                        count_rows("page_report", Some("analyse")) > reports_before
                            && destroyed_jobs.is_some_and(|j| count_rows("job", None) > j)
                    }
                    Some(k) => count_rows("page_report", Some(k)) > reports_before,
                    // the ready mode waits for the page's boot report too (the driver sends it
                    // right after the ready call)
                    None => ready_now && count_rows("page_report", Some("boot")) > boots_before,
                };
                if done {
                    got = true;
                    break;
                }
            }
            // the report of the edit may arrive inside the same pump that ends the wait (the
            // edit-destroy mode): the archive of that moment is stored here then
            if got && !edit_handled && mode == "edit-destroy" {
                if let Some(path) = &store_at_edit {
                    match doc.store() {
                        Ok(blob) => {
                            let _ = std::fs::write(path, &blob);
                            println!("{{\"event\":\"stored_at_edit\",\"bytes\":{}}}", blob.len());
                        }
                        Err(e) => failed = Some(e),
                    }
                }
            }
            if got && mode == "names" {
                // the notes after the rename and the move: the same notes as before them
                doc.print_all_notes("after-names", None);
            }
            if round == 1 && got {
                // let the page settle, then the first memory sample with the editor open
                let t = Instant::now();
                while t.elapsed() < Duration::from_millis(1500) {
                    pump(&app);
                }
                rss_open = rss_mb();
            }
            if got && (mode == "edit" || mode == "song") && wait_sync {
                // the plugin reads the events again on its worker; wait for its row
                let t = Instant::now();
                while t.elapsed() < Duration::from_secs(15) && count_rows("page_sync", None) == 0 {
                    pump(&app);
                    doc_pump(&doc);
                }
                // the host asks for the plugin's model updates a few times (the notification of
                // a source whose notes changed comes in `notifyModelUpdates`)
                let t = Instant::now();
                while t.elapsed() < Duration::from_millis(500) {
                    pump(&app);
                    doc_pump(&doc);
                }
            }
            if got && mode == "song" {
                // M1b8: the notes for the host after the page's edit (the first notes of the run are made before any edit)
                doc.print_all_notes("after-song", None);
            }
            ((*gui).destroy.expect("gui.destroy"))(plugin);
            if !got {
                failed = Some(format!(
                    "no page report after {} s (phase {mode}, round {round})",
                    limit.as_secs()
                ));
                break;
            }
            opened += 1;
            let t = Instant::now();
            while t.elapsed() < Duration::from_millis(100) {
                pump(&app);
            }
        }
        // the editor is closed: let queued main-thread jobs run, then the last sample
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(1500) {
            pump(&app);
        }
        let rss_after = rss_mb();
        if let Some(h) = args_value(rest, "--hold").and_then(|v| v.parse::<u64>().ok()) {
            // a debugging aid: stay alive so `leaks <pid>` or `vmmap <pid>` can look
            eprintln!("editor: holding pid {}", std::process::id());
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(h) {
                pump(&app);
            }
        }
        transport_stop.store(true, Ordering::SeqCst);
        if let Some(h) = transport_thread {
            let segments = h.join().unwrap_or_default();
            ((*plugin).stop_processing.expect("stop_processing"))(plugin);
            ((*plugin).deactivate.expect("deactivate"))(plugin);
            let list: Vec<String> = segments
                .iter()
                .map(|g| {
                    format!(
                        "{{\"wall_ms\":{:.3},\"playing\":{},\"song\":{:.6}}}",
                        g.wall_ms, g.playing, g.song
                    )
                })
                .collect();
            println!(
                "{{\"event\":\"transport_script\",\"kind\":{transport_kind:?},\"segments\":[{}]}}",
                list.join(",")
            );
        }
        if playback != "none" {
            println!("{}", playback_row(&playback, snap));
        }
        if mode == "hostkeys" {
            println!("{}", key_record_row());
        }
        audio_stop.store(true, Ordering::SeqCst);
        if let Some(h) = audio_thread {
            let _ = h.join();
            ((*plugin).stop_processing.expect("stop_processing"))(plugin);
            ((*plugin).deactivate.expect("deactivate"))(plugin);
        }
        if failed.is_none() {
            if let Some(path) = &archive {
                doc_pump(&doc);
                match doc.store() {
                    Ok(blob) => {
                        let _ = std::fs::write(path, &blob);
                        println!("{{\"event\":\"stored\",\"bytes\":{}}}", blob.len());
                    }
                    Err(e) => failed = Some(e),
                }
            }
        }
        ((*plugin).destroy.expect("destroy"))(plugin);
        doc.close();
        ((*lib.entry).deinit.expect("deinit"))();
        let audio = audio_stats.lock().map(|s| *s).unwrap_or_default();
        println!(
            "{{\"event\":\"editor_run\",\"mode\":{mode:?},\"cycles\":{cycles},\"opened\":{opened},\"ok\":{},\"why\":{:?},\"window_visible\":{window_visible},\"rss_closed_mb\":{rss_closed:.1},\"rss_open_mb\":{rss_open:.1},\"rss_after_mb\":{rss_after:.1},\"audio_blocks\":{},\"audio_overruns\":{},\"audio_worst_us\":{},\"audio_budget_us\":{},\"content\":{}}}",
            failed.is_none(),
            failed.clone().unwrap_or_default(),
            audio.0,
            audio.1,
            audio.2,
            audio.3,
            content_counters()
        );
        if failed.is_none() {
            0
        } else {
            2
        }
    }
}

fn doc_pump(doc: &HostDoc) {
    unsafe { super::pump(doc.inst) }
}

fn stem_safe(file: &str) -> String {
    // the track name of a file: the app's `safe_name` of its stem
    let stem = std::path::Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = String::new();
    let mut gap = false;
    for c in stem.chars() {
        if c.is_ascii() && (c.is_ascii_alphanumeric() || c == '-') {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

fn update_region(doc: &HostDoc, i: usize, a: f64, d: f64, p: f64) {
    unsafe {
        let dc = iface_of(doc.inst);
        let state = &*doc.state;
        if let Some(r) = state.regions.get(i) {
            let props = playback_props_for(state, i, a, d, p);
            (dc.updatePlaybackRegionProperties
                .expect("updatePlaybackRegionProperties"))(dc_ref_of(doc.inst), *r, &props);
        }
    }
}

/// One stretch of the host transport: the wall time (epoch ms) of its first block, whether it
/// plays, and the song position (seconds) at that block. In a playing stretch the position at
/// wall time `w` is `song + (w - wall_ms) / 1000`: the check computes the host's truth from it.
pub struct Segment {
    wall_ms: f64,
    playing: bool,
    song: f64,
}

/// The script of the transport: (start ms, playing, song position at the start). The `loop` kind
/// is made in code.
const SCRIPT: [(u64, bool, f64); 9] = [
    (0, false, 2.5),
    (1500, true, 2.5),
    (3300, false, 4.3),
    (4500, false, 3.2),
    (5500, true, 3.2),
    (6300, true, 2.2),
    (7500, false, 3.4),
    (8500, false, 1.0),
    (9500, false, 4.5),
];
const SCRIPT_END_MS: u64 = 11_500;
const LOOP_MS: u64 = 2_500;
const DEACTIVATE_MS: u64 = 2_000;

fn wall_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// The process calls of a host with a transport, paced like a real host (one block of `FRAMES`
/// every `FRAMES / SAMPLE_RATE` s, to absolute deadlines). `kind`: `script`, `loop`, or `null`
/// (calls that carry no transport).
fn spawn_transport(
    plugin_addr: usize,
    kind: &str,
    stop: std::sync::Arc<AtomicBool>,
) -> std::thread::JoinHandle<Vec<Segment>> {
    let kind = kind.to_string();
    std::thread::spawn(move || unsafe {
        let plugin = plugin_addr as *const clap_plugin;
        let mut input = vec![0.0f32; FRAMES];
        let mut output = vec![0.0f32; FRAMES];
        let in_buf = clap_sys::audio_buffer::clap_audio_buffer {
            data32: &mut input.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count: 1,
            latency: 0,
            constant_mask: 0,
        };
        let mut out_buf = clap_sys::audio_buffer::clap_audio_buffer {
            data32: &mut output.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count: 1,
            latency: 0,
            constant_mask: 0,
        };
        let in_events = clap_sys::events::clap_input_events {
            ctx: std::ptr::null_mut(),
            size: Some(in_events_size),
            get: Some(in_events_get),
        };
        let out_events = clap_sys::events::clap_output_events {
            ctx: std::ptr::null_mut(),
            try_push: Some(out_events_try_push),
        };
        let block = Duration::from_secs_f64(FRAMES as f64 / SAMPLE_RATE);
        let go = Instant::now();
        let mut transport: clap_sys::events::clap_event_transport = std::mem::zeroed();
        let mut segments: Vec<Segment> = Vec::new();
        let (mut seg_index, mut seg_block0) = (usize::MAX, 0u64);
        let mut k = 0u64;
        while !stop.load(Ordering::SeqCst) {
            let due = go + block * k as u32;
            let now = Instant::now();
            if due > now {
                std::thread::sleep(due - now);
            }
            let t_ms = (block.as_secs_f64() * 1000.0 * k as f64) as u64;
            let (playing, start_song, index) = match kind.as_str() {
                // M1b7: the position and the play state are the playback controller's (`PB_STATE`)
                "follow" => {
                    let mut st = PB_STATE.lock().unwrap();
                    let now = (st.playing, st.song);
                    if st.playing {
                        st.song += FRAMES as f64 / SAMPLE_RATE;
                    }
                    (now.0, now.1, 0)
                }
                "loop" => (true, 0.0, (t_ms / LOOP_MS) as usize),
                // plays from 2.5 s, then the host deactivates the plugin with no stopped process call
                "deactivate" => {
                    if t_ms >= DEACTIVATE_MS {
                        segments.push(Segment {
                            wall_ms: wall_ms(),
                            playing: false,
                            song: 2.5 + DEACTIVATE_MS as f64 / 1000.0,
                        });
                        ((*plugin).deactivate.expect("deactivate"))(plugin);
                        break;
                    }
                    (true, 2.5, 0)
                }
                _ => {
                    if t_ms >= SCRIPT_END_MS && kind == "script" {
                        break;
                    }
                    let i = SCRIPT.iter().rposition(|(at, _, _)| *at <= t_ms).unwrap_or(0);
                    (SCRIPT[i].1, SCRIPT[i].2, i)
                }
            };
            if index != seg_index {
                seg_index = index;
                seg_block0 = k;
                segments.push(Segment {
                    wall_ms: wall_ms(),
                    playing,
                    song: start_song,
                });
            }
            let song = start_song
                + if playing && kind != "follow" {
                    (k - seg_block0) as f64 * FRAMES as f64 / SAMPLE_RATE
                } else {
                    0.0
                };
            transport.flags = clap_sys::events::CLAP_TRANSPORT_HAS_SECONDS_TIMELINE
                | if playing {
                    clap_sys::events::CLAP_TRANSPORT_IS_PLAYING
                } else {
                    0
                };
            transport.song_pos_seconds = (song * 2_147_483_648.0) as i64;
            let process = clap_process {
                steady_time: (k * FRAMES as u64) as i64,
                frames_count: FRAMES as u32,
                transport: if kind == "null" { std::ptr::null() } else { &transport },
                audio_inputs: &in_buf,
                audio_outputs: &mut out_buf,
                audio_inputs_count: 1,
                audio_outputs_count: 1,
                in_events: &in_events,
                out_events: &out_events,
            };
            let _ = ((*plugin).process.expect("process"))(plugin, &process);
            k += 1;
        }
        segments
    })
}
