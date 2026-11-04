//! The `AppKit` parts: the key equivalents that muda leaves on the `NSMenuItem`s, the Shift convention, and the
//! `NSMenu` probes of the self-test (debug builds; stubs on other systems).

#[cfg(debug_assertions)]
use serde_json::{Value, json};
#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuItemKind};
#[cfg(any(target_os = "macos", debug_assertions))]
use tauri::{AppHandle, Runtime};

#[cfg(all(target_os = "macos", debug_assertions))]
use super::keymap::parse_key;

/// The position (menu bar index, then the index in each submenu) of the plain and check items `ids` in the menu: the
/// path to the same item in the `NSMenu`, which muda builds in the same order.
#[cfg(target_os = "macos")]
pub(super) fn item_paths<R: Runtime>(menu: &Menu<R>, ids: &std::collections::HashSet<&str>) -> Vec<Vec<usize>> {
    fn walk<R: Runtime>(
        items: &[MenuItemKind<R>],
        at: &mut Vec<usize>,
        ids: &std::collections::HashSet<&str>,
        out: &mut Vec<Vec<usize>>,
    ) {
        for (i, kind) in items.iter().enumerate() {
            at.push(i);
            match kind {
                MenuItemKind::Submenu(s) => {
                    if let Ok(sub) = s.items() {
                        walk(&sub, at, ids, out);
                    }
                }
                MenuItemKind::MenuItem(_) | MenuItemKind::Check(_) if ids.contains(kind.id().as_ref()) => {
                    out.push(at.clone());
                }
                _ => {}
            }
            at.pop();
        }
    }
    let mut out = Vec::new();
    if let Ok(items) = menu.items() {
        walk(&items, &mut Vec::new(), ids, &mut out);
    }
    out
}

/// Take the key equivalent off the `NSMenuItem`s at `paths` ([`item_paths`]). Runs on the main thread, after the
/// accelerators that muda set.
#[cfg(target_os = "macos")]
pub(super) fn clear_ns_equivalents<R: Runtime>(app: &AppHandle<R>, paths: Vec<Vec<usize>>) {
    let _ = app.run_on_main_thread(move || {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSEventModifierFlags};
        use objc2_foundation::NSString;
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(bar) = NSApplication::sharedApplication(mtm).mainMenu() else {
            return;
        };
        for path in &paths {
            let mut menu = bar.clone();
            for (depth, &i) in path.iter().enumerate() {
                let Some(it) = menu.itemArray().iter().nth(i) else {
                    break;
                };
                if depth + 1 == path.len() {
                    it.setKeyEquivalent(&NSString::from_str(""));
                    it.setKeyEquivalentModifierMask(NSEventModifierFlags(0));
                } else if let Some(sub) = it.submenu() {
                    menu = sub;
                } else {
                    break;
                }
            }
        }
    });
}

/// The `NSEvent` characters and modifier flags of a key of the table (`"Cmd+Down"`): the key
/// equivalent that macOS matches a physical key press against (muda's mapping in
/// `muda/src/accelerator/macos.rs`). Returns `(characters, charactersIgnoringModifiers, flags)`.
/// A physical Shift+Cmd press has the uppercase character in `characters` and the lowercase one
/// in `charactersIgnoringModifiers` — `NSMenu` keys its match off the latter, so the synthetic
/// event must carry both (the probe of #206: `characters` alone matched nothing). None when the
/// key is not in the table's notation.
#[cfg(all(target_os = "macos", debug_assertions))]
fn ns_event_key(key: &str) -> Option<(String, String, u64)> {
    let (mods, k) = parse_key(key)?;
    let named = [
        ("Up", "\u{F700}"),
        ("Down", "\u{F701}"),
        ("Left", "\u{F702}"),
        ("Right", "\u{F703}"),
        ("Return", "\u{0d}"),
        ("Tab", "\u{09}"),
        ("Escape", "\u{1b}"),
        ("Backspace", "\u{08}"),
        ("Delete", "\u{7f}"),
        ("Space", " "),
        ("Plus", "+"),
        ("Minus", "-"),
        ("NumpadMinus", "-"),
        ("NumpadPlus", "+"),
        ("NumpadPeriod", "."),
        ("NumpadEnter", "\u{0d}"),
    ];
    let chars = if k.len() == 1 {
        // muda registers the key equivalent of a letter in lowercase (the keyDown of a physical
        // press carries the lowercase character when only Cmd is held) and the uppercase one when
        // Shift is held (Shift+Cmd+O has 'O' in characters)
        if mods.contains(&"Shift") {
            k.to_ascii_uppercase()
        } else {
            k.to_ascii_lowercase()
        }
    } else {
        named
            .iter()
            .find(|(n, _)| *n == k)
            .map(|(_, c)| c.to_string())
            .or_else(|| {
                k.strip_prefix("Numpad")
                    .and_then(|d| d.parse::<u8>().ok())
                    .map(|d| d.to_string())
            })?
    };
    // a physical press with Shift carries the shifted character in `characters` AND in
    // `charactersIgnoringModifiers` (Shift+Cmd+S has "S" in both); the NSMenu of macOS matches
    // the latter (probe of #206), so the synthetic event carries the uppercase letter in both
    let cim = if k.len() == 1 {
        if mods.contains(&"Shift") {
            k.to_ascii_uppercase()
        } else {
            k.to_ascii_lowercase()
        }
    } else {
        chars.clone()
    };
    // NSEventModifierFlag (Cocoa): CapsLock 1<<16, Shift 1<<17, Control 1<<18, Option 1<<19, Command 1<<20
    // (the NSMenu masks of the #206 snapshot decode 1179648 = Cmd+Shift, 1572864 = Cmd+Option)
    let mut flags: u64 = 0;
    for m in mods {
        flags |= match m {
            "Shift" => 1 << 17,
            "Control" => 1 << 18,
            "Option" => 1 << 19,
            "Cmd" => 1 << 20,
            _ => 0,
        };
    }
    Some((chars, cim, flags))
}

/// #206: the `AppKit` convention for a Shift+Cmd accelerator: the key equivalent carries the
/// SHIFTED (uppercase) character and the modifier mask does NOT contain Shift. muda registers
/// the lowercase character with Shift in the mask; `NSMenu` then matches a physical Shift+Cmd
/// press case-insensitively against `charactersIgnoringModifiers` and an earlier Cmd-only item
/// consumes the event (Save ate Shift+Cmd+S). With the convention the match is exact. Runs on
/// the main thread; walks the whole menu bar and rewrites only letter key equivalents of items
/// with Shift in the mask.
#[cfg(target_os = "macos")]
pub fn fix_shift_equivalents<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.run_on_main_thread(|| {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSEventModifierFlags};
        use objc2_foundation::NSString;
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu() else {
            return;
        };
        fn walk(menu: &objc2_app_kit::NSMenu) {
            for it in &menu.itemArray() {
                if it.hasSubmenu() {
                    if let Some(sub) = it.submenu() {
                        walk(&sub);
                    }
                    continue;
                }
                // NSEventModifierFlag (Cocoa): Shift 1<<17
                let mask = it.keyEquivalentModifierMask().0;
                if mask & (1 << 17) == 0 {
                    continue;
                }
                let ke = it.keyEquivalent().to_string();
                let mut ch = ke.chars();
                if let (Some(c), None) = (ch.next(), ch.next())
                    && c.is_ascii_alphabetic()
                {
                    it.setKeyEquivalent(&NSString::from_str(&c.to_ascii_uppercase().to_string()));
                    it.setKeyEquivalentModifierMask(NSEventModifierFlags(mask & !(1 << 17)));
                }
            }
        }
        for sub in &menu.itemArray() {
            if let Some(s) = sub.submenu() {
                walk(&s);
            }
        }
    });
}

/// The main menu as macOS has it (debug builds, the self-test of #206): one row per menu item
/// with the menu title, the label, the key equivalent and the modifier mask of the `NSMenu`. This
/// is what `performKeyEquivalent` matches a physical key against — not the accelerator strings
/// that [`report`] shows. Separators (no label) are left out.
#[cfg(all(target_os = "macos", debug_assertions))]
pub fn ns_menu_rows<R: Runtime>(app: &AppHandle<R>) -> Value {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel();
    let _ = app.run_on_main_thread(move || {
        let rows = (|| {
            use objc2::MainThreadMarker;
            use objc2_app_kit::NSApplication;
            let Some(mtm) = MainThreadMarker::new() else {
                return json!([]);
            };
            let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu() else {
                return json!([]);
            };
            let mut rows = Vec::new();
            // nested submenus too (View > Waveform Zoom > In): walk every level
            fn walk(menu: &objc2_app_kit::NSMenu, title: &str, rows: &mut Vec<Value>) {
                for it in &menu.itemArray() {
                    if it.hasSubmenu() {
                        if let Some(sub) = it.submenu() {
                            walk(&sub, &it.title().to_string(), rows);
                        }
                        continue;
                    }
                    if it.title().is_empty() {
                        continue;
                    }
                    rows.push(json!({
                        "menu": title, "label": it.title().to_string(),
                        "key": it.keyEquivalent().to_string(), "mask": it.keyEquivalentModifierMask().0,
                        "enabled": it.isEnabled(),
                    }));
                }
            }
            for sub in &menu.itemArray() {
                let title = sub.title().to_string();
                let Some(sub) = sub.submenu() else { continue };
                walk(&sub, &title, &mut rows);
            }
            json!(rows)
        })();
        let _ = tx.send(rows);
    });
    rx.recv_timeout(std::time::Duration::from_secs(3))
        .unwrap_or(json!({"error": "the main thread did not answer"}))
}

/// The real native menu path of a `menu_key` (debug builds, the self-test of #206): a synthetic
/// keyDown `NSEvent` for `key` (in the table's notation) goes to the app's main menu through
/// `NSMenu performKeyEquivalent:` — the same route a physical key press takes. `matched` says
/// whether the menu performed the key equivalent (the item fired: its action sent the
/// `MenuEvent`, `on_menu_event` dispatches it and the page handler runs). `rows` is
/// [`ns_menu_rows`] evidence about the registration.
#[cfg(all(target_os = "macos", debug_assertions))]
pub fn perform_key_equivalent<R: Runtime>(app: &AppHandle<R>, key: &str) -> Value {
    let Some((chars, cim, flags)) = ns_event_key(key) else {
        return json!({"error": format!("{key} is not in the table's notation")});
    };
    let matched = perform_key_equivalent_raw(app, &chars, &cim, flags);
    json!({"key": key, "characters": chars, "characters_ignoring_modifiers": cim, "flags": flags, "matched": matched})
}

/// The raw event probe behind [`perform_key_equivalent`] (the self-test diagnoses which
/// characters/flags variant the `NSMenu` matches): the probe passes the `characters` variant and
/// uses it for `charactersIgnoringModifiers` too. Returns 1 performed, 0 not, -1 no main thread.
#[cfg(all(target_os = "macos", debug_assertions))]
pub fn perform_key_equivalent_raw<R: Runtime>(app: &AppHandle<R>, chars: &str, cim: &str, flags: u64) -> i8 {
    use std::sync::mpsc;
    let chars_ev = chars.to_string();
    let cim_ev = cim.to_string();
    let (tx, rx) = mpsc::channel();
    let _ = app.run_on_main_thread(move || {
        let matched = (|| {
            use objc2::MainThreadMarker;
            use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSApplication};
            use objc2_foundation::NSString;
            let Some(mtm) = MainThreadMarker::new() else { return -1 };
            let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu() else { return -1 };
            let cs = NSString::from_str(&chars_ev);
            let ci = NSString::from_str(&cim_ev);
            let Some(ev) = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::KeyDown, objc2_foundation::NSPoint::ZERO,
                NSEventModifierFlags(flags as usize), 0.0, 0, None, &cs, &ci, false, 0,
            ) else { return -1 };
            if menu.performKeyEquivalent(&ev) { 1 } else { 0 }
        })();
        let _ = tx.send(matched);
    });
    rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap_or(-1)
}

/// Linux (and other non-macOS) stubs for the #206 self-test probes: the native-menu introspection
/// and the key-equivalent probe are AppKit-only; here they report that they are not available, so
/// the desktop crate compiles and `cargo test -p drumspike-desktop` runs its unit tests.
#[cfg(all(debug_assertions, not(target_os = "macos")))]
pub fn ns_menu_rows<R: Runtime>(_app: &AppHandle<R>) -> Value {
    json!({ "error": "the NSMenu self-test is macOS-only" })
}

#[cfg(all(debug_assertions, not(target_os = "macos")))]
pub fn perform_key_equivalent<R: Runtime>(_app: &AppHandle<R>, _key: &str) -> Value {
    json!({ "error": "the NSMenu self-test is macOS-only" })
}

#[cfg(all(debug_assertions, not(target_os = "macos")))]
pub fn perform_key_equivalent_raw<R: Runtime>(_app: &AppHandle<R>, _chars: &str, _cim: &str, _flags: u64) -> i8 {
    -1
}
