//! The key rule of the editor (M1b12, #561).
//!
//! Rule: while the plugin's view has the keyboard focus, EVERY key reaches the plugin only. Each key
//! down, key up, modifier change and key equivalent is consumed here, whether the page used it or not.
//! Nothing travels on to the host's responder chain, its key commands or its menu, except the short
//! list `SYSTEM_KEYS` below. The space bar is no exception: the page takes it and asks the host to
//! play or stop through `B.hostTransport` (#554); it never reaches the host as a key.
//!
//! Three parts hold the rule (the design note `docs/design/daw-plugin/m1b12-keys.md` has the full path
//! of a key and the evidence):
//!
//! 1. The key monitor (`watch`): a local event monitor runs inside `[NSApp sendEvent:]`, before the
//!    host's menu, its views (a host view above the plugin sees a key equivalent before the plugin's
//!    own views) and its responder chain. While the plugin's view has the focus it takes EVERY key
//!    down, key up and modifier change of its window except the system list, gives it to the web view
//!    itself, and drops the event. Measured with the test host: the main menu gets a key with
//!    Command before every view, and a host view above the plugin gets a plain key before the shield.
//! 2. The key shield: the web view lives inside one plain view of the plugin, its superview and so the
//!    next responder above it. What WebKit hands on as unused stops here (a second line, and the
//!    place that the tab key cannot leave through: the web view is its own next key view).
//! 3. The mouse monitor: the keys follow the first responder, and a click on a host view that does
//!    not accept the first responder would leave it with the web view. A mouse down in the window
//!    outside the shield takes the first responder away from the web view (the host has the keys
//!    again); a mouse down in the shield gives it to the web view.
//!
//! The page shows the text of the key's command group when the key is one of an absent command
//! (`editor/keys.js`); that is the page side and needs no call from here.
//!
//! The Objective-C class of the shield is made at run time with a name that is unique in the process
//! (two copies of the plugin, or a reload, must not clash).
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{msg_send, sel, ClassType, MainThreadMarker};
use objc2_app_kit::{NSEvent, NSEventMask, NSEventType, NSView};
use objc2_foundation::NSRect;
use serde_json::json;
use std::ffi::CStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// The modifier bits of `NSEvent.modifierFlags` that count (the device-independent ones).
pub const SHIFT: u64 = 1 << 17;
pub const CONTROL: u64 = 1 << 18;
pub const OPTION: u64 = 1 << 19;
pub const COMMAND: u64 = 1 << 20;
const MODS: u64 = SHIFT | CONTROL | OPTION | COMMAND;

/// One key that the plugin passes on to the host on purpose: a command of the system, which the
/// user expects to work in whatever app has the focus.
pub struct SystemKey {
    /// The key as the characters of the event say it, lower case (`charactersIgnoringModifiers`); empty: the code decides.
    pub chars: &'static str,
    /// The virtual key code, for a key that has no stable character (the key above Tab); 0: the character decides.
    pub code: u16,
    /// The modifiers that must be down, exactly these (of Shift, Control, Option, Command).
    pub mods: u64,
    /// Why the key goes to the host.
    pub why: &'static str,
}

/// THE exception list of the rule (also in the design note). Nothing else is passed on.
/// The space bar is not here: it plays and stops the host through the page, not as a key.
pub const SYSTEM_KEYS: [SystemKey; 6] = [
    SystemKey {
        chars: "q",
        code: 0,
        mods: COMMAND,
        why: "quit the application",
    },
    SystemKey {
        chars: "h",
        code: 0,
        mods: COMMAND,
        why: "hide the application",
    },
    SystemKey {
        chars: "h",
        code: 0,
        mods: COMMAND | OPTION,
        why: "hide the other applications",
    },
    SystemKey {
        chars: "m",
        code: 0,
        mods: COMMAND,
        why: "minimise the window",
    },
    SystemKey {
        chars: "",
        code: 50,
        mods: COMMAND,
        why: "next window of the application",
    },
    SystemKey {
        chars: "",
        code: 50,
        mods: COMMAND | SHIFT,
        why: "previous window of the application",
    },
];

/// Is this key a command of the system (the list above)? `chars`: `charactersIgnoringModifiers`.
pub fn is_system_key(chars: &str, code: u16, flags: u64) -> bool {
    system_key(chars, code, flags).is_some()
}

/// The entry of the list that this key is, if any.
pub fn system_key(chars: &str, code: u16, flags: u64) -> Option<&'static SystemKey> {
    let mods = flags & MODS;
    let chars = chars.to_lowercase();
    SYSTEM_KEYS.iter().find(|k| {
        k.mods == mods
            && if k.code != 0 {
                k.code == code
            } else {
                !k.chars.is_empty() && k.chars == chars
            }
    })
}

/// The number of key events the plugin took, for the report (the check compares it with the host's record).
pub static STOPPED: AtomicUsize = AtomicUsize::new(0);
/// The number of system keys that it passed on.
pub static PASSED: AtomicUsize = AtomicUsize::new(0);
/// Rows of the report are written for the first keys only.
const ROW_LIMIT: usize = 400;

/// A key was taken (or passed). A gate build (feature `test-switches`) writes a row for the first keys. A delivery
/// build writes at most one row, for the first stopped key; the counts go to the row `editor_close`. The row is
/// kept in memory, never written to the file from here (this runs on the host's main thread).
fn note(kind: &str, event: &NSEvent, why: Option<&str>) {
    let pass = kind == "pass";
    let n = if pass {
        PASSED.fetch_add(1, Ordering::SeqCst)
    } else {
        STOPPED.fetch_add(1, Ordering::SeqCst)
    };
    if !cfg!(feature = "test-switches") {
        // a delivery build writes one row, for the first key DOWN that the plugin stopped
        static WRITTEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if pass || event.r#type() != NSEventType::KeyDown || WRITTEN.swap(true, Ordering::SeqCst) {
            return;
        }
    } else if n >= ROW_LIMIT {
        return;
    }
    // a delivery build logs the key code and the modifier flags only: the character is what the user typed
    let chars = if cfg!(feature = "test-switches") {
        event_chars(event)
    } else {
        String::new()
    };
    let row = json!({"event": if pass { "key_pass" } else { "key_stop" }, "where": kind,
        "code": event_code(event), "chars": chars, "flags": event_flags(event) & MODS, "why": why});
    crate::session().log_deferred(row.to_string());
}

fn event_chars(event: &NSEvent) -> String {
    // a modifier-only event (flagsChanged) has no characters: the call would raise
    if !is_key(event) {
        return String::new();
    }
    event
        .charactersIgnoringModifiers()
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn event_code(event: &NSEvent) -> u16 {
    if is_key(event) {
        event.keyCode()
    } else {
        0
    }
}

fn event_flags(event: &NSEvent) -> u64 {
    event.modifierFlags().0 as u64
}

fn is_key(event: &NSEvent) -> bool {
    matches!(event.r#type(), NSEventType::KeyDown | NSEventType::KeyUp)
}

/// The monitors of the editor (see `watch`).
pub type Monitors = Vec<Retained<AnyObject>>;

/// The fault runs: the whole rule is off (`keys-pass`).
fn rule_off() -> bool {
    crate::editor::fault() == "keys-pass"
}

/// The fault runs: the shield lets the keys it should take go on to the host (`no-shield`), or the whole rule is off.
fn shield_open() -> bool {
    rule_off() || crate::editor::fault() == "no-shield"
}

// ---------------------------------------------------------------- the key shield (a class made at run time)

const IV_WEB: &CStr = c"ds_web";
const IV_LAST_AT: &CStr = c"ds_last_at";
const IV_LAST_CODE: &CStr = c"ds_last_code";
const IV_LAST_STAMP: &CStr = c"ds_last_stamp";
const IV_LAST_MS: &CStr = c"ds_last_ms";
const IV_MARK_MS: &CStr = c"ds_mark_ms";

/// How long after the plugin gave a key to the web view the answers of WebKit (editing commands, a menu action it derives)
/// still count as belonging to that key. WebKit finishes a key in another process, so the answer may come late.
const MARK_MS: u64 = 2000;

thread_local! {
    /// The key codes whose key down the plugin took (main thread only): their key up is the plugin's too; the key up of a
    /// key whose down went to the host goes to the host.
    static TAKEN: std::cell::RefCell<std::collections::HashSet<u16>> = std::cell::RefCell::new(std::collections::HashSet::new());
}

/// A pointer to one of the shield's instance variables (all `usize` or `u64`).
fn iv<T: objc2::Encode>(obj: &AnyObject, name: &CStr) -> *mut T {
    let ivar = obj
        .class()
        .instance_variable(name)
        .expect("the shield has its variables");
    // SAFETY: the variable was added with the same type by `shield_class`.
    unsafe { ivar.load_ptr::<T>(obj) }
}

fn view_of(obj: &AnyObject) -> &NSView {
    // SAFETY: the shield is an NSView subclass.
    unsafe { &*(obj as *const AnyObject as *const NSView) }
}

fn now_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// The class of the shield. Its name is unique in the process: a fixed name makes a second copy of the plugin
/// (the CLAP and the VST3 bundle in one process, two versions, a reload) panic or get the class of the first copy.
fn shield_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut n = 0usize;
        loop {
            // the address of this static differs for every loaded copy of this code; the counter covers a reload at the same address
            let name = std::ffi::CString::new(format!("DrumSpikeAraKeyShield_{:x}_{n}", &CLASS as *const _ as usize))
                .expect("no nul in the class name");
            // the fault run: one fixed name, as the first version had (a second copy then cannot make its class)
            let builder = if crate::editor::fault() == "fixed-class-name" {
                Some(
                    ClassBuilder::new(c"DrumSpikeAraKeyShield", NSView::class())
                        .expect("the class name is taken by another copy"),
                )
            } else {
                ClassBuilder::new(&name, NSView::class())
            };
            if let Some(mut b) = builder {
                b.add_ivar::<usize>(IV_WEB);
                b.add_ivar::<usize>(IV_LAST_AT);
                b.add_ivar::<usize>(IV_LAST_CODE);
                b.add_ivar::<u64>(IV_LAST_STAMP);
                b.add_ivar::<u64>(IV_LAST_MS);
                b.add_ivar::<u64>(IV_MARK_MS);
                // SAFETY: the signatures are those of the overridden methods.
                unsafe {
                    b.add_method(sel!(keyDown:), key_down as extern "C-unwind" fn(_, _, _));
                    b.add_method(sel!(keyUp:), key_up as extern "C-unwind" fn(_, _, _));
                    b.add_method(sel!(flagsChanged:), flags_changed as extern "C-unwind" fn(_, _, _));
                    b.add_method(
                        sel!(performKeyEquivalent:),
                        perform_key_equivalent as extern "C-unwind" fn(_, _, _) -> Bool,
                    );
                    b.add_method(sel!(doCommandBySelector:), do_command as extern "C-unwind" fn(_, _, _));
                    b.add_method(
                        sel!(tryToPerform:with:),
                        try_to_perform as extern "C-unwind" fn(_, _, _, _) -> Bool,
                    );
                }
                return b.register();
            }
            n += 1;
        }
    })
}

/// An Objective-C class cannot be unregistered, and the key shield (and a pending callback of the page) hold code of this image.
/// If the host unloaded the bundle while one is alive, a later call would go to freed code. So the image is pinned: it is
/// opened again from its own path with RTLD_NODELETE, and the host's unload then leaves it mapped. (The cost: the bundle cannot
/// be replaced in a running process; the host has to be restarted for a new version, as it has to for any plugin with a class.)
fn pin_image() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(pin_image_once);
}

fn pin_image_once() {
    #[repr(C)]
    struct DlInfo {
        dli_fname: *const std::ffi::c_char,
        dli_fbase: *mut std::ffi::c_void,
        dli_sname: *const std::ffi::c_char,
        dli_saddr: *mut std::ffi::c_void,
    }
    extern "C" {
        fn dladdr(addr: *const std::ffi::c_void, info: *mut DlInfo) -> i32;
        fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut std::ffi::c_void;
    }
    const RTLD_NOW: i32 = 2;
    const RTLD_NODELETE: i32 = 0x80;
    let mut info = DlInfo {
        dli_fname: std::ptr::null(),
        dli_fbase: std::ptr::null_mut(),
        dli_sname: std::ptr::null(),
        dli_saddr: std::ptr::null_mut(),
    };
    // SAFETY: dladdr fills the struct for an address of this image; dlopen of the same path only raises its count.
    unsafe {
        if dladdr(pin_image as *const std::ffi::c_void, &mut info) != 0 && !info.dli_fname.is_null() {
            let handle = dlopen(info.dli_fname, RTLD_NOW | RTLD_NODELETE);
            crate::session().log_deferred(json!({"event": "key_image_pinned", "ok": !handle.is_null()}).to_string());
        }
    }
}

/// Make the shield: a view of the plugin's class, with the frame given.
pub fn new_shield(_mtm: MainThreadMarker, frame: NSRect) -> Retained<NSView> {
    let cls = shield_class();
    pin_image();
    crate::session().log_deferred(json!({"event": "key_shield", "class": cls.name().to_string_lossy()}).to_string());
    // SAFETY: alloc and init of an NSView subclass on the main thread.
    unsafe {
        let raw: *mut AnyObject = msg_send![cls, alloc];
        let raw: *mut AnyObject = msg_send![raw, initWithFrame: frame];
        Retained::from_raw(raw as *mut NSView).expect("the shield was made")
    }
}

/// The web view that sits in the shield. It is its own next key view: a Tab at the last element of the page (WebKit asks
/// the window for the next key view) must not move the focus out of the plugin.
pub fn set_web_view(shield: &NSView, web: &AnyObject) {
    let obj: &AnyObject = shield.as_ref();
    // SAFETY: the variable is `usize`.
    unsafe { *iv::<usize>(obj, IV_WEB) = web as *const AnyObject as usize };
    let _: () = unsafe { msg_send![web, setNextKeyView: web] };
}

extern "C-unwind" fn key_down(this: &AnyObject, _sel: Sel, event: *mut AnyObject) {
    // SAFETY: AppKit gives a valid event.
    let event: &NSEvent = unsafe { &*(event as *const NSEvent) };
    crate::guard("key shield", (), || {
        if shield_open() {
            let _: () = unsafe { msg_send![super(this, NSView::class()), keyDown: event] };
            return;
        }
        TAKEN.with(|t| {
            t.borrow_mut().insert(event.keyCode());
        });
        note("keyDown", event, None);
    })
}

extern "C-unwind" fn key_up(this: &AnyObject, _sel: Sel, event: *mut AnyObject) {
    // SAFETY: AppKit gives a valid event.
    let event: &NSEvent = unsafe { &*(event as *const NSEvent) };
    crate::guard("key shield", (), || {
        // the key up of a key whose down the plugin did not take (the down went to the host) goes up the chain to the host
        let ours = TAKEN.with(|t| t.borrow_mut().remove(&event.keyCode()));
        if shield_open() || !ours {
            let _: () = unsafe { msg_send![super(this, NSView::class()), keyUp: event] };
            return;
        }
        note("keyUp", event, None);
    })
}

extern "C-unwind" fn flags_changed(this: &AnyObject, _sel: Sel, event: *mut AnyObject) {
    // SAFETY: AppKit gives a valid event.
    let event: &NSEvent = unsafe { &*(event as *const NSEvent) };
    crate::guard("key shield", (), || {
        // a modifier alone runs no command: it goes up the chain to the host (it may track the modifiers from events and must
        // see the release of one that it saw go down)
        if shield_open() || crate::editor::fault() != "no-forward-modifier" {
            let _: () = unsafe { msg_send![super(this, NSView::class()), flagsChanged: event] };
            return;
        }
        note("flagsChanged", event, None);
    })
}

/// A key equivalent that reaches the views of the window (the host has no menu item for it, or no monitor is in
/// place). AppKit offers EVERY key down to the views of the window, whoever has the focus, so the shield takes a key
/// only while the plugin's view has the focus. A command of the system goes on to the host. Any other key is taken.
extern "C-unwind" fn perform_key_equivalent(this: &AnyObject, _sel: Sel, event: *mut AnyObject) -> Bool {
    // SAFETY: AppKit gives a valid event.
    let event: &NSEvent = unsafe { &*(event as *const NSEvent) };
    crate::guard("key shield", Bool::NO, || {
        if rule_off() {
            return unsafe { msg_send![super(this, NSView::class()), performKeyEquivalent: event] };
        }
        let shield = view_of(this);
        if !plugin_has_focus(shield, event) {
            return Bool::NO;
        }
        if is_key(event) && crate::editor::fault() != "no-system-keys" {
            if let Some(k) = system_key(&event_chars(event), event.keyCode(), event_flags(event)) {
                note("pass", event, Some(k.why));
                return Bool::NO;
            }
        }
        take(this, event, "performKeyEquivalent");
        Bool::YES
    })
}

/// An editing command that nobody below took (WebKit asks the responder chain; `deleteBackward:`, `cancelOperation:`, a move
/// command). It is reachable only from below the shield, so while the plugin has the focus it always stops here: a host view
/// above may delete data on such a command. (The first version of round 2 accepted it only for a young event: a slow main
/// thread of the host made it pass.)
extern "C-unwind" fn do_command(this: &AnyObject, _sel: Sel, selector: Sel) {
    crate::guard("key shield", (), || {
        if cfg!(feature = "test-switches") {
            crate::session().log_deferred(
                json!({"event": "key_command", "selector": selector.name().to_string_lossy()}).to_string(),
            );
        }
        let old = crate::editor::fault() == "old-freshness";
        let ours = if old {
            current_key_event_is_ours_old(this)
        } else {
            has_focus_now(view_of(this))
        };
        if shield_open() || !ours {
            let _: () = unsafe { msg_send![super(this, NSView::class()), doCommandBySelector: selector] };
            return;
        }
        STOPPED.fetch_add(1, Ordering::SeqCst);
    })
}

/// The decision of `try_to_perform`, as a function of what is known: the shield swallows an action only while the plugin
/// has the focus, a key of the plugin was given to the web view a moment ago (`marked`), the event being handled is no mouse
/// or menu event (a click on a menu item of the host has one, and must still work), and it is no system key.
pub fn action_is_ours(
    open: bool,
    focus: bool,
    active: bool,
    marked: bool,
    mouse_event: bool,
    system_key_event: bool,
) -> bool {
    !open && focus && active && marked && !mouse_event && !system_key_event
}

/// A menu action of the host with no target (Save, Undo, Close) asks the responder chain too. The shield answers YES only
/// for an action that WebKit derives from a key of the plugin (see `action_is_ours`); else the chain goes on.
extern "C-unwind" fn try_to_perform(this: &AnyObject, _sel: Sel, selector: Sel, object: *mut AnyObject) -> Bool {
    crate::guard("key shield", Bool::NO, || {
        let fault = crate::editor::fault();
        let ours = if fault == "swallow-actions" {
            true
        } else if fault == "old-freshness" {
            current_key_event_is_ours_old(this)
        } else {
            let shield = view_of(this);
            let (mouse, system) = current_event_class();
            action_is_ours(
                shield_open(),
                has_focus_now(shield),
                window_is_active(shield),
                marked(this),
                mouse,
                system,
            )
        };
        if ours {
            STOPPED.fetch_add(1, Ordering::SeqCst);
            return Bool::YES;
        }
        unsafe { msg_send![super(this, NSView::class()), tryToPerform: selector, with: object] }
    })
}

/// The shield's window is the key window and the application is active: else a menu action that travels through the plugin's
/// chain belongs to a panel or a sheet, or to the host in the background.
fn window_is_active(shield: &NSView) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    shield.window().is_some_and(|w| w.isKeyWindow()) && objc2_app_kit::NSApplication::sharedApplication(mtm).isActive()
}

/// Is the plugin's view the first responder now (the web view or a view below the shield)? No event needed.
fn has_focus_now(shield: &NSView) -> bool {
    let Some(window) = shield.window() else { return false };
    window
        .firstResponder()
        .and_then(|r| r.downcast::<NSView>().ok())
        .is_some_and(|v| v.isDescendantOf(shield))
}

/// Did `take` give a key to the web view in the last `MARK_MS`? (Counted from the handling, not from the creation of the event.)
fn marked(this: &AnyObject) -> bool {
    // SAFETY: the variable is `u64`.
    let at = unsafe { *iv::<u64>(this, IV_MARK_MS) };
    at != 0 && now_ms().saturating_sub(at) < MARK_MS
}

/// (is the event being handled a mouse event, is it a key of the system list)
fn current_event_class() -> (bool, bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return (false, false);
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    match app.currentEvent() {
        Some(ev) => {
            let mouse = matches!(
                ev.r#type(),
                NSEventType::LeftMouseDown
                    | NSEventType::LeftMouseUp
                    | NSEventType::RightMouseDown
                    | NSEventType::RightMouseUp
                    | NSEventType::OtherMouseDown
                    | NSEventType::OtherMouseUp
            );
            let system = is_key(&ev) && system_key(&event_chars(&ev), ev.keyCode(), event_flags(&ev)).is_some();
            (mouse, system)
        }
        None => (false, false),
    }
}

/// The rule of round 2 (the fault run `old-freshness`): the event must be a young key event, counted from its creation.
fn current_key_event_is_ours_old(this: &AnyObject) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    match app.currentEvent() {
        Some(ev) => {
            let age = objc2_foundation::NSProcessInfo::processInfo().systemUptime() - ev.timestamp();
            is_key(&ev) && (0.0..0.25).contains(&age) && plugin_has_focus(view_of(this), &ev)
        }
        None => false,
    }
}

/// The decision of the second-pass guard: the same event as the last one that was given to the web view? The address alone is no
/// proof (a later event may get it); the key code, the time stamp and the time since then count too.
pub fn same_event(last: (usize, usize, u64, u64), now: (usize, usize, u64, u64)) -> bool {
    last.0 == now.0 && last.1 == now.1 && last.2 == now.2 && now.3.saturating_sub(last.3) < 2000
}

/// Is this the second pass of the last event (WebKit sends an unused key again through `[NSApp sendEvent:]`)?
fn is_second_pass(this: &AnyObject, event: &NSEvent) -> bool {
    // SAFETY: the variables are `usize` and `u64`.
    let last = unsafe {
        (
            *iv::<usize>(this, IV_LAST_AT),
            *iv::<usize>(this, IV_LAST_CODE),
            *iv::<u64>(this, IV_LAST_STAMP),
            *iv::<u64>(this, IV_LAST_MS),
        )
    };
    same_event(
        last,
        (
            event as *const NSEvent as usize,
            event_code(event) as usize,
            event.timestamp().to_bits(),
            now_ms(),
        ),
    )
}

/// Take a key event: give it to the web view as AppKit gives it to the first responder (key down, key up, modifier
/// change), and nowhere else. The page then uses it or not. After a key down with Command that edits text, the page is
/// asked (`__DS_KEYS__.edit()`) whether a text field has the key and no handler used it; the web view then runs the
/// action of the menu item that an app would have (copy, cut, paste). Select All and undo are done by the page script.
fn take(this: &AnyObject, event: &NSEvent, where_: &str) {
    let me = event as *const NSEvent as usize;
    let stamp = event.timestamp().to_bits();
    let ms = now_ms();
    // SAFETY: the variables are `usize` and `u64`.
    let code_now = event_code(event) as usize;
    if is_second_pass(this, event) {
        note("second-pass", event, None);
        return;
    }
    // SAFETY: as above.
    unsafe {
        *iv::<usize>(this, IV_LAST_AT) = me;
        *iv::<usize>(this, IV_LAST_CODE) = code_now;
        *iv::<u64>(this, IV_LAST_STAMP) = stamp;
        *iv::<u64>(this, IV_LAST_MS) = ms;
    }
    note(where_, event, None);
    // SAFETY: as above. The mark says: from now on the answers of WebKit belong to a key of the plugin.
    if event.r#type() == NSEventType::KeyDown {
        unsafe { *iv::<u64>(this, IV_MARK_MS) = ms.max(1) };
    }
    match event.r#type() {
        NSEventType::KeyDown => TAKEN.with(|t| {
            t.borrow_mut().insert(event.keyCode());
        }),
        NSEventType::KeyUp => TAKEN.with(|t| {
            t.borrow_mut().remove(&event.keyCode());
        }),
        _ => {}
    }
    // SAFETY: as above.
    let web = unsafe { *iv::<usize>(this, IV_WEB) } as *mut AnyObject;
    if web.is_null() {
        return;
    }
    match event.r#type() {
        NSEventType::KeyUp => {
            let _: () = unsafe { msg_send![web, keyUp: event] };
        }
        NSEventType::FlagsChanged => {
            let _: () = unsafe { msg_send![web, flagsChanged: event] };
        }
        _ => {
            let _: () = unsafe { msg_send![web, keyDown: event] };
            let mods = event_flags(event) & MODS;
            if mods & COMMAND != 0
                && mods & (CONTROL | OPTION) == 0
                && ["c", "x", "v"].contains(&event_chars(event).as_str())
            {
                ask_page_for_edit(web);
            }
        }
    }
}

/// Ask the page what a key with Command meant for a text field: "copy", "cut", "paste" or nothing. The answer comes
/// back on the main thread; the web view runs its own action then.
fn ask_page_for_edit(web: *mut AnyObject) {
    // SAFETY: the web view lives as long as the plugin instance (wry keeps it).
    let Some(web) = (unsafe { Retained::retain(web) }) else {
        return;
    };
    let js = objc2_foundation::NSString::from_str("window.__DS_KEYS__ ? window.__DS_KEYS__.edit() : ''");
    let web2 = web.clone();
    let done = block2::RcBlock::new(move |result: *mut AnyObject, _error: *mut AnyObject| {
        crate::guard("key edit", (), || {
            if result.is_null() {
                return;
            }
            let is_text: Bool = unsafe { msg_send![result, isKindOfClass: objc2_foundation::NSString::class()] };
            if !is_text.as_bool() {
                return;
            }
            // SAFETY: checked to be an NSString.
            let text = unsafe { &*(result as *const objc2_foundation::NSString) }.to_string();
            let action = match text.as_str() {
                "copy" => Some(sel!(copy:)),
                "cut" => Some(sel!(cut:)),
                "paste" => Some(sel!(paste:)),
                _ => None,
            };
            if let Some(sel) = action {
                // an Objective-C exception through Rust frames is undefined: ask first
                let responds: Bool = unsafe { msg_send![&*web2, respondsToSelector: sel] };
                if !responds.as_bool() {
                    return;
                }
                let nothing: *const AnyObject = std::ptr::null();
                let _: *mut AnyObject = unsafe { msg_send![&*web2, performSelector: sel, withObject: nothing] };
            }
        })
    });
    let _: () = unsafe { msg_send![&*web, evaluateJavaScript: &*js, completionHandler: &*done] };
}

/// Does the plugin's view have the keyboard focus for this event? The event is for the window of the shield and the
/// first responder of that window is the web view or a view below the shield.
fn plugin_has_focus(shield: &NSView, event: &NSEvent) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(shield_window) = shield.window() else {
        return false;
    };
    let Some(event_window) = event.window(mtm) else {
        return false;
    };
    if !std::ptr::eq(&*shield_window, &*event_window) {
        return false;
    }
    let Some(responder) = shield_window.firstResponder() else {
        return false;
    };
    match responder.downcast::<NSView>() {
        Ok(view) => view.isDescendantOf(shield),
        Err(_) => false,
    }
}

/// Give a key up or a modifier change to the responder above the shield: the host's views (and its window) get it as they would if
/// the web view had passed it on.
fn up_the_chain(shield: &NSView, event: &NSEvent) {
    if crate::editor::fault() == "no-forward-modifier" {
        return;
    }
    // SAFETY: the next responder is a live responder of the window.
    let next = unsafe { shield.nextResponder() };
    let Some(next) = next else { return };
    match event.r#type() {
        NSEventType::KeyUp => {
            let _: () = unsafe { msg_send![&*next, keyUp: event] };
        }
        NSEventType::FlagsChanged => {
            let _: () = unsafe { msg_send![&*next, flagsChanged: event] };
        }
        _ => {}
    }
}

/// The focus leaves the plugin: give the web view a change of the modifier keys with no modifier down, so that the page
/// does not keep a Command that it saw go down.
fn release_modifiers(shield: &AnyObject, like: &NSEvent) {
    // SAFETY: the variable is `usize`.
    let web = unsafe { *iv::<usize>(shield, IV_WEB) } as *mut AnyObject;
    if web.is_null() {
        return;
    }
    let none = objc2_foundation::NSString::from_str("");
    let window_number = like.windowNumber();
    if let Some(ev) = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::FlagsChanged,
        objc2_foundation::NSPoint::new(0.0, 0.0),
        objc2_app_kit::NSEventModifierFlags(0),
        like.timestamp(),
        window_number,
        None,
        &none,
        &none,
        false,
        55,
    ) {
        let _: () = unsafe { msg_send![web, flagsChanged: &*ev] };
    }
}

/// The monitors of the editor. Both run inside `[NSApp sendEvent:]`, before the host's menu and views.
///
/// Key monitor: while the plugin's view has the focus, EVERY key down, key up and modifier change of its window is
/// taken (`take`) and the event dropped; a system key (the list) and every key while the focus is elsewhere go on
/// unchanged. Mouse monitor: a mouse down in the window outside the shield takes the first responder from the web
/// view (the host's views may refuse it, and the web view would keep every key); a mouse down in the shield gives
/// it to the web view. Both are removed with `unwatch` when the editor leaves the host's view.
pub fn watch(shield: &Retained<NSView>) -> Monitors {
    use objc2::rc::Weak;
    use std::ptr::NonNull;
    let mut out = Vec::new();
    let weak: Weak<NSView> = Weak::from_retained(shield);
    let keys = block2::RcBlock::new(move |ev: NonNull<NSEvent>| -> *mut NSEvent {
        let pass = ev.as_ptr();
        crate::guard("key monitor", pass, || {
            if rule_off() || crate::editor::fault() == "no-monitor" {
                return pass;
            }
            // SAFETY: AppKit gives a valid event for the call.
            let event: &NSEvent = unsafe { ev.as_ref() };
            let Some(shield) = weak.load() else { return pass };
            if !plugin_has_focus(&shield, event) {
                return pass;
            }
            if is_key(event) && crate::editor::fault() != "no-system-keys" {
                if let Some(k) = system_key(&event_chars(event), event.keyCode(), event_flags(event)) {
                    if event.r#type() == NSEventType::KeyDown {
                        note("pass", event, Some(k.why));
                    }
                    return pass;
                }
            }
            match event.r#type() {
                // a modifier alone runs no command: the page gets it (the web view), and the host's views get it from the view
                // above the shield, as the responder chain would give it if the web view passed it on. (Normal dispatch does not:
                // the web view keeps a modifier change for itself, measured with the test host.)
                NSEventType::FlagsChanged => {
                    let obj: &AnyObject = shield.as_ref();
                    // SAFETY: the variable is `usize`.
                    let web = unsafe { *iv::<usize>(obj, IV_WEB) } as *mut AnyObject;
                    if !web.is_null() {
                        let _: () = unsafe { msg_send![web, flagsChanged: event] };
                    }
                    up_the_chain(&shield, event);
                    std::ptr::null_mut()
                }
                // the key up of a key whose down the plugin did not take belongs to the host
                // (a second pass of a key up that the plugin took is the plugin's: `take` removed its code from the set)
                NSEventType::KeyUp
                    if !TAKEN.with(|t| t.borrow().contains(&event.keyCode()))
                        && !is_second_pass(shield.as_ref(), event) =>
                {
                    up_the_chain(&shield, event);
                    std::ptr::null_mut()
                }
                _ => {
                    take(shield.as_ref(), event, "monitor");
                    std::ptr::null_mut()
                }
            }
        })
    });
    let mask = NSEventMask::KeyDown | NSEventMask::KeyUp | NSEventMask::FlagsChanged;
    // SAFETY: the block returns the event it got or null, as the method asks.
    if let Some(m) = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &keys) } {
        out.push(m);
    }
    let weak2: Weak<NSView> = Weak::from_retained(shield);
    let mouse = block2::RcBlock::new(move |ev: NonNull<NSEvent>| -> *mut NSEvent {
        let pass = ev.as_ptr();
        crate::guard("mouse monitor", pass, || {
            if rule_off() || crate::editor::fault() == "no-click-out" {
                return pass;
            }
            // SAFETY: AppKit gives a valid event for the call.
            let event: &NSEvent = unsafe { ev.as_ref() };
            let Some(shield) = weak2.load() else { return pass };
            let Some(mtm) = MainThreadMarker::new() else {
                return pass;
            };
            let (Some(window), Some(event_window)) = (shield.window(), event.window(mtm)) else {
                return pass;
            };
            if !std::ptr::eq(&*window, &*event_window) {
                return pass;
            }
            // inside = the view that the click lands on is the web view or a view below the shield (a host view over a
            // part of the plugin, an overlay or a resize grip, is outside)
            let inside = if crate::editor::fault() == "rect-inside" {
                let p = shield.convertPoint_fromView(event.locationInWindow(), None);
                let b = shield.bounds();
                p.x >= b.origin.x
                    && p.x < b.origin.x + b.size.width
                    && p.y >= b.origin.y
                    && p.y < b.origin.y + b.size.height
            } else {
                match window.contentView() {
                    Some(content) => {
                        let at = match unsafe { content.superview() } {
                            Some(sup) => sup.convertPoint_fromView(event.locationInWindow(), None),
                            None => event.locationInWindow(),
                        };
                        content.hitTest(at).is_some_and(|v| v.isDescendantOf(&shield))
                    }
                    None => false,
                }
            };
            let has = has_focus_now(&shield);
            let obj: &AnyObject = shield.as_ref();
            // SAFETY: the variable is `usize`.
            let web = unsafe { *iv::<usize>(obj, IV_WEB) } as *mut AnyObject;
            if inside && !has && !web.is_null() {
                let _: Bool = unsafe { msg_send![&*window, makeFirstResponder: web] };
            } else if !inside && has {
                // the click is outside the plugin: the host has the keys again, whether the view it clicks takes the first
                // responder or not. The first responder becomes the window; the click goes on, and the host sets its own
                // responder from it if it wants one. (No responder is stored: it could be the web view itself.)
                let _: Bool = unsafe { msg_send![&*window, makeFirstResponder: std::ptr::null::<AnyObject>()] };
                TAKEN.with(|t| t.borrow_mut().clear());
                // the page must not keep a modifier that it saw go down and will not see go up
                release_modifiers(obj, event);
            }
            pass
        })
    });
    let mask = NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown;
    // SAFETY: as above.
    if let Some(m) = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &mouse) } {
        out.push(m);
    }
    out
}

/// Remove the monitors made by `watch`.
pub fn unwatch(monitors: Monitors) {
    TAKEN.with(|t| t.borrow_mut().clear());
    for m in monitors {
        // SAFETY: the object came from `addLocalMonitorForEventsMatchingMask:handler:`.
        unsafe { NSEvent::removeMonitor(&m) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_action_is_swallowed_only_for_a_key_of_the_plugin() {
        // the plugin has the focus, a key was given to the web view a moment ago, the event is no mouse event: swallowed
        assert!(action_is_ours(false, true, true, true, false, false));
        // a click on a menu item of the host (a mouse event) goes on
        assert!(!action_is_ours(false, true, true, true, true, false));
        // the focus is elsewhere, or no key of the plugin is recent
        assert!(!action_is_ours(false, false, true, true, false, false));
        assert!(!action_is_ours(false, true, true, false, false, false));
        // a system key (the action of Command+H, Q, M with no target) goes on to the host
        assert!(!action_is_ours(false, true, true, true, false, true));
        // the rule is off
        assert!(!action_is_ours(true, true, true, true, false, false));
    }

    #[test]
    fn an_action_is_not_swallowed_while_a_panel_is_key_or_the_host_is_in_the_background() {
        assert!(!action_is_ours(false, true, false, true, false, false));
    }

    #[test]
    fn the_second_pass_of_an_event_is_the_same_event() {
        let last = (0x1000, 12, 77, 5000);
        assert!(
            same_event(last, (0x1000, 12, 77, 5100)),
            "the same event a moment later"
        );
        assert!(
            !same_event(last, (0x1000, 12, 78, 5100)),
            "another time stamp: another event at the same address"
        );
        assert!(!same_event(last, (0x1000, 13, 77, 5100)), "another key");
        assert!(!same_event(last, (0x2000, 12, 77, 5100)), "another address");
        assert!(!same_event(last, (0x1000, 12, 77, 9000)), "much later");
    }

    #[test]
    fn the_system_list_is_short_and_named() {
        assert!(SYSTEM_KEYS.len() <= 8, "the exception list must stay short");
        assert!(SYSTEM_KEYS.iter().all(|k| !k.why.is_empty()));
    }

    #[test]
    fn a_command_of_the_system_goes_to_the_host() {
        assert!(is_system_key("q", 12, COMMAND));
        assert!(
            is_system_key("Q", 12, COMMAND),
            "the case of the character does not matter"
        );
        assert!(is_system_key("h", 4, COMMAND));
        assert!(is_system_key("h", 4, COMMAND | OPTION));
        assert!(is_system_key("m", 46, COMMAND));
        assert!(is_system_key("`", 50, COMMAND));
        assert!(is_system_key("~", 50, COMMAND | SHIFT));
    }

    #[test]
    fn no_other_key_goes_to_the_host() {
        // the space bar plays through the page, never as a key
        assert!(!is_system_key(" ", 49, 0));
        assert!(!is_system_key(" ", 49, COMMAND));
        // a plain key, a key of a clip command, a key with a modifier that is no command of the system
        assert!(!is_system_key("q", 12, 0), "Q alone is no quit");
        assert!(
            !is_system_key("q", 12, COMMAND | SHIFT),
            "Shift+Cmd+Q (log out) is not passed on"
        );
        assert!(!is_system_key("q", 12, COMMAND | CONTROL));
        assert!(!is_system_key("e", 14, COMMAND));
        assert!(!is_system_key("z", 6, COMMAND));
        assert!(
            !is_system_key("w", 13, COMMAND),
            "close window is the host's own command: it stays in the plugin"
        );
        assert!(!is_system_key("h", 4, 0));
        assert!(!is_system_key("h", 4, COMMAND | OPTION | SHIFT));
        assert!(!is_system_key("", 0, 0));
        assert!(!is_system_key("`", 50, COMMAND | OPTION));
    }
}
