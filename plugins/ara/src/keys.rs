//! The key rule of the editor (M1b12, #561).
//!
//! Rule: while the plugin's view has the keyboard focus, EVERY key reaches the plugin only. Each key
//! down, key up, modifier change and key equivalent is consumed here, whether the page used it or not.
//! Nothing travels on to the host's responder chain, its key commands or its menu, except the short
//! list `SYSTEM_KEYS` below. The space bar is no exception: the page takes it and asks the host to
//! play or stop through `B.hostTransport` (#554); it never reaches the host as a key.
//!
//! Two parts hold the rule (the design note `docs/design/daw-plugin/m1b12-keys.md` has the full path
//! of a key and the evidence):
//!
//! 1. The key shield (`KeyShield`): the web view lives inside one plain view of the plugin, which is
//!    the web view's superview and so the next responder above it. WebKit hands a key that the page
//!    did not use to the next responder; the shield takes it (key down, key up, modifier change, an
//!    editing command that nobody took). Without the shield such a key goes on up the host's chain.
//! 2. The equivalent monitor (`watch_equivalents`): AppKit offers a key with Command, Control or
//!    Option to the MAIN MENU of the host before it offers it to the views of the window (measured
//!    with the test host: a menu command ran and the page saw no key). A view cannot stop that. A
//!    local event monitor runs inside `[NSApp sendEvent:]`, first. While the plugin's view has the
//!    focus it takes such a key, gives it to the web view as the key down that AppKit would send if
//!    no menu had taken it, and drops the event.
//!
//! The page shows the text of the key's command group when the key is one of an absent command
//! (`editor/keys.js`); that is the page side and needs no call from here.
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSEvent, NSEventMask, NSView};
use objc2_foundation::NSRect;
use serde_json::json;
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// The number of keys the shield took, for the report (the check compares it with the host's record).
pub static STOPPED: AtomicUsize = AtomicUsize::new(0);
/// The number of system keys that it passed on.
pub static PASSED: AtomicUsize = AtomicUsize::new(0);
/// Rows of the report are written for the first keys only.
const ROW_LIMIT: usize = 400;

fn note(kind: &str, event: &NSEvent, why: Option<&str>) {
    let n = if kind == "pass" {
        PASSED.fetch_add(1, Ordering::SeqCst)
    } else {
        STOPPED.fetch_add(1, Ordering::SeqCst)
    };
    if n >= ROW_LIMIT {
        return;
    }
    let chars = event_chars(event);
    let row = json!({"event": if kind == "pass" { "key_pass" } else { "key_stop" }, "where": kind_name(kind),
        "code": event_code(event), "chars": chars, "flags": event_flags(event) & MODS, "why": why});
    crate::session().log(row.to_string());
    crate::session().write_out();
}

fn kind_name(kind: &str) -> &str {
    match kind {
        "pass" => "performKeyEquivalent",
        other => other,
    }
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
    use objc2_app_kit::NSEventType;
    matches!(event.r#type(), NSEventType::KeyDown | NSEventType::KeyUp)
}

/// The monitor of key equivalents (see `watch_equivalents`).
pub type Monitor = Retained<AnyObject>;

pub struct ShieldIvars {
    /// The web view inside the shield (not retained: the shield's subviews keep it).
    web: Cell<*mut AnyObject>,
    /// The key equivalent that was last given to the web view: the address of the event, the time, the key code. WebKit may
    /// send an unused key equivalent again through `[NSApp sendEvent:]`; that second pass must not be given to the web view
    /// again (a loop). The address alone is no proof (a later event may get the same address), so the key code, the time stamp of
    /// the event and the time since the last pass count too (a second pass comes within milliseconds; the bound is generous).
    last: Cell<(usize, std::time::Instant, u16, u64)>,
}

define_class!(
    /// The superview of the web view: the next responder above it, and a view that sees a key equivalent.
    /// The class name is global to the process: the Objective-C runtime keeps the first definition if two copies
    /// of the plugin are in one host (the code of every copy is the same version).
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DrumSpikeAraKeyShield"]
    #[ivars = ShieldIvars]
    pub struct KeyShield;

    impl KeyShield {
        /// A key down that the page did not use (WebKit gives it to the next responder).
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if shield_open() {
                let _: () = unsafe { msg_send![super(self), keyDown: event] };
                return;
            }
            note("keyDown", event, None);
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            if shield_open() {
                let _: () = unsafe { msg_send![super(self), keyUp: event] };
                return;
            }
            note("keyUp", event, None);
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            if shield_open() {
                let _: () = unsafe { msg_send![super(self), flagsChanged: event] };
                return;
            }
            note("flagsChanged", event, None);
        }

        /// A key equivalent that reaches the views of the window (the host has no menu item for it, or the monitor
        /// is not in place). A command of the system goes on to the host. Any other key is taken.
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> Bool {
            if rule_off() {
                let r: Bool = unsafe { msg_send![super(self), performKeyEquivalent: event] };
                return r;
            }
            // AppKit offers EVERY key down to the views of the window, whoever has the focus: the shield takes
            // a key only while the plugin's view has it (else the keys of the host's own views would be taken)
            if !plugin_has_focus(self, event) {
                return Bool::NO;
            }
            if is_key(event) && crate::editor::fault() != "no-system-keys" {
                if let Some(k) = system_key(&event_chars(event), event.keyCode(), event_flags(event)) {
                    note("pass", event, Some(k.why));
                    return Bool::NO;
                }
            }
            self.take_equivalent(event, "performKeyEquivalent");
            Bool::YES
        }

        /// An editing command that nobody below took (WebKit asks the responder chain): the host must not get it.
        #[unsafe(method(doCommandBySelector:))]
        fn do_command(&self, selector: Sel) {
            if shield_open() {
                let _: () = unsafe { msg_send![super(self), doCommandBySelector: selector] };
                return;
            }
            STOPPED.fetch_add(1, Ordering::SeqCst);
        }

        #[unsafe(method(tryToPerform:with:))]
        fn try_to_perform(&self, selector: Sel, object: Option<&AnyObject>) -> Bool {
            if shield_open() {
                let r: Bool = unsafe { msg_send![super(self), tryToPerform: selector, with: object] };
                return r;
            }
            STOPPED.fetch_add(1, Ordering::SeqCst);
            Bool::YES
        }
    }
);

/// The fault runs: the whole rule is off (`keys-pass`).
fn rule_off() -> bool {
    crate::editor::fault() == "keys-pass"
}

/// The fault runs: the shield lets the keys it should take go on to the host (`no-shield`), or the whole rule is off.
fn shield_open() -> bool {
    rule_off() || crate::editor::fault() == "no-shield"
}

impl KeyShield {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<KeyShield> {
        let this = mtm.alloc::<KeyShield>().set_ivars(ShieldIvars {
            web: Cell::new(std::ptr::null_mut()),
            last: Cell::new((0, std::time::Instant::now(), 0, 0)),
        });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// The web view that sits in the shield.
    pub fn set_web_view(&self, web: *mut AnyObject) {
        self.ivars().web.set(web);
    }

    /// Take a key equivalent: give it to the web view as the key down that AppKit sends to the first responder when
    /// no menu item and no view takes a key equivalent (that is how the page gets Cmd+Z), and nowhere else. The page
    /// then uses it or not. The clipboard keys and Select All also run the web view's own action: they are menu items
    /// in an app, and a text field of the page would lose them when the host's menu cannot take them any more.
    fn take_equivalent(&self, event: &NSEvent, where_: &str) {
        let me = event as *const NSEvent as usize;
        let stamp = event.timestamp().to_bits();
        let (last_at, last_time, last_code, last_stamp) = self.ivars().last.get();
        if last_at == me
            && last_code == event.keyCode()
            && last_stamp == stamp
            && last_time.elapsed() < std::time::Duration::from_secs(2)
        {
            // the second pass of the same event (WebKit sends an unused key equivalent again): it stays here
            note("second-pass", event, None);
            return;
        }
        self.ivars()
            .last
            .set((me, std::time::Instant::now(), event.keyCode(), stamp));
        note(where_, event, None);
        let web = self.ivars().web.get();
        if web.is_null() {
            return;
        }
        let _: () = unsafe { msg_send![web, keyDown: event] };
        if event_flags(event) & MODS == COMMAND {
            let action = match event_chars(event).as_str() {
                "c" => Some(objc2::sel!(copy:)),
                "x" => Some(objc2::sel!(cut:)),
                "v" => Some(objc2::sel!(paste:)),
                "a" => Some(objc2::sel!(selectAll:)),
                _ => None,
            };
            if let Some(sel) = action {
                let nothing: *const AnyObject = std::ptr::null();
                let _: *mut AnyObject = unsafe { msg_send![web, performSelector: sel, withObject: nothing] };
            }
        }
    }
}

/// Does the plugin's view have the keyboard focus for this event? The event is for the window of the shield and the
/// first responder of that window is the web view or a view below the shield.
fn plugin_has_focus(shield: &KeyShield, event: &NSEvent) -> bool {
    let Some(shield_window) = shield.window() else {
        return false;
    };
    let Some(event_window) = event.window(shield.mtm()) else {
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

/// A key equivalent (a key with Command, Control or Option) goes to the MAIN MENU of the host BEFORE the views of the
/// key window (see the head of this file). A local event monitor sees the event first, so this is the one place that
/// can stop it: while the plugin's view has the focus, the monitor takes the key equivalent (`take_equivalent`) and
/// drops the event. A system key (the list) and every key while the focus is elsewhere go on unchanged. Returns the
/// monitor; the caller removes it with `unwatch` when the editor leaves the host's view.
pub fn watch_equivalents(shield: &Retained<KeyShield>) -> Option<Monitor> {
    use objc2::rc::Weak;
    use std::ptr::NonNull;
    let weak: Weak<KeyShield> = Weak::from_retained(shield);
    let handler = block2::RcBlock::new(move |ev: NonNull<NSEvent>| -> *mut NSEvent {
        let pass = ev.as_ptr();
        crate::guard("key monitor", pass, || {
            if rule_off() || crate::editor::fault() == "no-monitor" {
                return pass;
            }
            // SAFETY: AppKit gives a valid event for the call.
            let event: &NSEvent = unsafe { ev.as_ref() };
            let Some(shield) = weak.load() else { return pass };
            let flags = event_flags(event);
            if !is_key(event) || flags & (COMMAND | CONTROL | OPTION) == 0 || !plugin_has_focus(&shield, event) {
                return pass;
            }
            if crate::editor::fault() != "no-system-keys" {
                if let Some(k) = system_key(&event_chars(event), event.keyCode(), flags) {
                    note("pass", event, Some(k.why));
                    return pass;
                }
            }
            shield.take_equivalent(event, "monitor");
            std::ptr::null_mut()
        })
    });
    // SAFETY: the block returns the event it got or null, as the method asks.
    unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler) }
}

/// Remove a monitor made by `watch_equivalents`.
pub fn unwatch(monitor: &Monitor) {
    // SAFETY: the object came from `addLocalMonitorForEventsMatchingMask:handler:`.
    unsafe { NSEvent::removeMonitor(monitor) };
}

#[cfg(test)]
mod tests {
    use super::*;

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
