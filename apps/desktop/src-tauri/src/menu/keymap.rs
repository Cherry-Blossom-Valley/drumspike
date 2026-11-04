//! The key command table (`static/keymap.js`, compiled in): its parser and checks, the accelerator of an item per
//! preset and typing state, and the preset and typing state of the live menu.

use std::sync::{LazyLock, Mutex};

use serde_json::Value;

use super::items::Kind;
use super::lock;

/// `static/keymap.js`, compiled in (P3-13b): the key command table of the app. The page loads the same file.
pub const KEYMAP_JS: &str = include_str!("../../../../../static/keymap.js");
const KEYMAP_BEGIN: &str = "/*KEYMAP-BEGIN*/";
const KEYMAP_END: &str = "/*KEYMAP-END*/";
/// The modifiers of the table's key notation, in the macOS order.
pub const KEY_MODS: [&str; 4] = ["Control", "Option", "Shift", "Cmd"];
const KEY_NAMES: [&str; 26] = [
    "Plus",
    "Minus",
    "Space",
    "Tab",
    "Return",
    "Delete",
    "Backspace",
    "Escape",
    "Left",
    "Right",
    "Up",
    "Down",
    "NumpadMinus",
    "NumpadPlus",
    "NumpadPeriod",
    "NumpadEnter",
    "Numpad0",
    "Numpad1",
    "Numpad2",
    "Numpad3",
    "Numpad4",
    "Numpad5",
    "Numpad6",
    "Numpad7",
    "Numpad8",
    "Numpad9",
];
const KEY_CHARS: &str = "[],.=_/;'`\\";

/// One row of the key command table (`static/keymap.js`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCommand {
    /// The command id (for a menu item: the item id).
    pub id: String,
    /// The menu item id, or `None` for a key that only the page handles.
    pub menu: Option<String>,
    /// The menu of the item, one title per level (`"View > Zoom Presets"`: `["View", "Zoom Presets"]`); empty
    /// without a menu item.
    pub path: Vec<String>,
    /// The item needs an open project (menu rows; `None` when the row does not say).
    pub needs_project: Option<bool>,
    /// Plain or check item (menu rows; `None` when the row does not say).
    pub kind: Option<Kind>,
    /// The menu text.
    pub title: String,
    /// The menu accelerator in the table's notation (`Shift+Cmd+S`), or `None`.
    pub menu_key: Option<String>,
    /// The keys that the page handles.
    pub page_keys: Vec<String>,
    /// The keys of the "Cubase" preset (P3-13c), or `None` when the preset keeps the default keys.
    pub cubase: Option<Vec<String>>,
}

impl KeyCommand {
    /// The menu accelerator of the command in a preset (P3-13c): in `cubase` the first key of the row's list with
    /// Cmd, Control or Option (the other keys are page keys), else the row's `menu_key`; `None` without a menu item.
    #[must_use]
    pub fn menu_key_in(&self, preset: &str) -> Option<String> {
        self.menu.as_ref()?;
        match (&self.cubase, preset) {
            (Some(c), "cubase") => c.iter().find(|k| has_menu_modifier(k)).cloned(),
            _ => self.menu_key.clone(),
        }
    }

    /// The key that the menu shows for the command in a preset (#300): the [`menu_key_in`](Self::menu_key_in), else the
    /// first key of the preset that the menu can show (a Numpad key it cannot: the row's next key). `None` without a
    /// menu item or a showable key. A key without Cmd, Control or Option is on the menu only while no text field
    /// has the focus ([`set_typing`]).
    #[must_use]
    pub fn shown_key_in(&self, preset: &str) -> Option<String> {
        self.menu.as_ref()?;
        self.menu_key_in(preset).or_else(|| {
            self.keys(preset)
                .into_iter()
                .find(|k| parse_key(k).is_some_and(|(_, key)| !key.starts_with("Numpad")))
        })
    }

    /// The keys of the command in a preset: `default` (the menu key, then the page keys) or `cubase`.
    #[must_use]
    pub fn keys(&self, preset: &str) -> Vec<String> {
        match (&self.cubase, preset) {
            (Some(c), "cubase") => c.clone(),
            _ => self.menu_key.iter().chain(&self.page_keys).cloned().collect(),
        }
    }
}

/// The rows of a key table file: the strict JSON between the markers `KEYMAP-BEGIN` and `KEYMAP-END`.
///
/// # Errors
///
/// When a marker is missing, the text between them is not strict JSON or not a list, or a row has
/// no `id`, no `title` or no `cubase` key, a field of the wrong type, or a `kind` other than `plain` and
/// `check`. The message names the row.
pub fn parse_keymap(src: &str) -> Result<Vec<KeyCommand>, String> {
    let a = src.find(KEYMAP_BEGIN).ok_or("no KEYMAP-BEGIN marker")? + KEYMAP_BEGIN.len();
    let b = a + src[a..].find(KEYMAP_END).ok_or("no KEYMAP-END marker")?;
    let rows: Value = serde_json::from_str(&src[a..b]).map_err(|e| format!("the table is not strict JSON: {e}"))?;
    let rows = rows.as_array().ok_or("the table is not a list")?;
    let text = |r: &Value, n: usize, k: &str| -> Result<Option<String>, String> {
        match r.get(k) {
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(Value::Null) | None => Ok(None),
            _ => Err(format!("row {n}: \"{k}\" is not a string or null")),
        }
    };
    let list = |v: &Value, n: usize, k: &str| -> Result<Vec<String>, String> {
        v.as_array()
            .and_then(|a| {
                a.iter()
                    .map(|x| x.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| format!("row {n}: \"{k}\" is not a list of strings"))
    };
    rows.iter()
        .enumerate()
        .map(|(n, r)| {
            Ok(KeyCommand {
                id: text(r, n, "id")?.ok_or_else(|| format!("row {n}: no id"))?,
                menu: text(r, n, "menu")?,
                path: text(r, n, "path")?
                    .map(|p| p.split(" > ").map(str::to_string).collect())
                    .unwrap_or_default(),
                needs_project: match r.get("needs_project") {
                    Some(Value::Bool(b)) => Some(*b),
                    Some(Value::Null) | None => None,
                    _ => return Err(format!("row {n}: \"needs_project\" is not true, false or null")),
                },
                kind: match text(r, n, "kind")?.as_deref() {
                    Some("plain") => Some(Kind::Plain),
                    Some("check") => Some(Kind::Check),
                    None => None,
                    Some(k) => return Err(format!("row {n}: \"kind\" is \"{k}\", not \"plain\" or \"check\"")),
                },
                title: text(r, n, "title")?.ok_or_else(|| format!("row {n}: no title"))?,
                menu_key: text(r, n, "menu_key")?,
                page_keys: list(r.get("page_keys").unwrap_or(&Value::Null), n, "page_keys")?,
                cubase: match r.get("cubase") {
                    Some(Value::Null) => None,
                    Some(v) => Some(list(v, n, "cubase")?),
                    None => return Err(format!("row {n}: no \"cubase\"")),
                },
            })
        })
        .collect()
}

/// A key of the table split into its modifiers and its key (`Shift+Cmd+S`: `["Shift", "Cmd"]`, `S`), or `None`
/// when it is not in the table's notation (unknown key, modifiers out of the order Control, Option, Shift, Cmd,
/// Shift on a character key).
#[must_use]
pub fn parse_key(s: &str) -> Option<(Vec<&str>, &str)> {
    let mut parts: Vec<&str> = s.split('+').collect();
    let key = parts.pop()?;
    let order: Vec<usize> = parts
        .iter()
        .map(|m| KEY_MODS.iter().position(|x| x == m))
        .collect::<Option<_>>()?;
    if order.windows(2).any(|w| w[0] >= w[1]) {
        return None;
    }
    let one = key.len() == 1 && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    let char_key = key.len() == 1 && KEY_CHARS.contains(key);
    if !(one || char_key || KEY_NAMES.contains(&key)) || (char_key && parts.contains(&"Shift")) {
        return None;
    }
    Some((parts, key))
}

/// The key has Cmd, Control or Option: it can be a menu accelerator (a text field does not need it).
#[must_use]
pub fn has_menu_modifier(key: &str) -> bool {
    parse_key(key).is_some_and(|(mods, _)| mods.iter().any(|m| *m != "Shift"))
}

/// A key of the table in the Tauri accelerator notation: `Shift+Cmd+S` is `CmdOrCtrl+Shift+S`, `Option+Return` is
/// `Alt+Enter` (modifier order Ctrl, `CmdOrCtrl`, Alt, Shift, as the menu had them in P3-13a). `None` when the key is
/// not in the table's notation.
#[must_use]
pub fn tauri_accel(key: &str) -> Option<String> {
    let (mods, k) = parse_key(key)?;
    let mut out: Vec<&str> = [
        ("Control", "Ctrl"),
        ("Cmd", "CmdOrCtrl"),
        ("Option", "Alt"),
        ("Shift", "Shift"),
    ]
    .iter()
    .filter(|(m, _)| mods.contains(m))
    .map(|(_, t)| *t)
    .collect();
    let k = match k {
        "Return" => "Enter",
        "NumpadMinus" => "NumpadSubtract",
        "NumpadPlus" => "NumpadAdd",
        "NumpadPeriod" => "NumpadDecimal",
        "Left" => "ArrowLeft",
        "Right" => "ArrowRight",
        "Up" => "ArrowUp",
        "Down" => "ArrowDown",
        other => other,
    };
    out.push(k);
    Some(out.join("+"))
}

/// Everything wrong with a key table: an id twice, a menu key without Cmd, Control or Option or without a menu
/// item, a key not in the notation, a key in two commands (in the default and in the Cubase preset). Empty when the
/// table is good. `DSKeys.problems` in `static/keymap.js` makes the same checks in the page.
#[must_use]
pub fn keymap_problems(rows: &[KeyCommand]) -> Vec<String> {
    let mut out = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for c in rows {
        if !ids.insert(c.id.as_str()) {
            out.push(format!("{}: the id is twice in the table", c.id));
        }
        if let Some(k) = &c.menu_key {
            if !has_menu_modifier(k) {
                out.push(format!("{}: menu key {k} has no Cmd, Control or Option", c.id));
            }
            if c.menu.is_none() {
                out.push(format!("{}: menu key {k} without a menu item", c.id));
            }
        }
    }
    for preset in ["default", "cubase"] {
        let mut seen: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
        for c in rows {
            for k in c.keys(preset) {
                if parse_key(&k).is_none() {
                    out.push(format!("{}: key \"{k}\" is not in the table's notation", c.id));
                    continue;
                }
                match seen.get(&k) {
                    Some(other) if *other != c.id => {
                        out.push(format!("{preset}: key {k} is in two commands: {other} and {}", c.id));
                    }
                    Some(_) => out.push(format!("{preset}: key {k} is twice in {}", c.id)),
                    None => {}
                }
                seen.insert(k, c.id.as_str());
            }
        }
    }
    out
}

static KEYMAP: LazyLock<Vec<KeyCommand>> = LazyLock::new(|| {
    parse_keymap(KEYMAP_JS).unwrap_or_else(|e| {
        eprintln!("static/keymap.js: {e}; the menu has no shortcuts");
        Vec::new()
    })
});

/// The key table of this build (`static/keymap.js`). Empty when the file does not parse (a test makes sure it does).
#[must_use]
pub fn keymap() -> &'static [KeyCommand] {
    &KEYMAP
}

/// The row of the menu item `id`.
#[must_use]
pub fn key_row(id: &str) -> Option<&'static KeyCommand> {
    keymap().iter().find(|c| c.menu.as_deref() == Some(id))
}

/// The accelerator of the menu item `id` in the Tauri notation: the `menu_key` of its row in the key table.
/// `build` sets exactly this; there is no other source of menu keys.
#[must_use]
pub fn accel_of(id: &str, preset: &str) -> Option<String> {
    key_row(id)
        .and_then(|c| c.menu_key_in(preset))
        .and_then(|k| tauri_accel(&k))
}

/// The accelerator that the menu item `id` shows (#300): [`accel_of`], else the first showable key of its row, also
/// a key without Cmd, Control or Option (Space, Tab, V). Tauri notation.
#[must_use]
pub fn shown_accel_of(id: &str, preset: &str) -> Option<String> {
    key_row(id)
        .and_then(|c| c.shown_key_in(preset))
        .and_then(|k| tauri_accel(&k))
}

/// The accelerator that the live menu gives the item `id`: while `typing` (a text field or a dialog of the page
/// has the focus) only the keys with Cmd, Control or Option, else every shown key.
#[must_use]
pub fn live_accel(id: &str, preset: &str, typing: bool) -> Option<String> {
    if typing {
        accel_of(id, preset)
    } else {
        shown_accel_of(id, preset)
    }
}

/// The key presets of the app (P3-13c): the setting `key_preset` of `<home>/settings.json` holds one of them.
pub const PRESETS: [&str; 2] = ["default", "cubase"];

/// `name` when it is a preset, else the default one (a damaged setting never leaves the menu without keys).
#[must_use]
pub fn preset_or_default(name: &str) -> &'static str {
    PRESETS.iter().copied().find(|p| *p == name).unwrap_or("default")
}

/// The preset that the menu shows now (the last [`set_preset`], the default at start).
pub(super) static PRESET: Mutex<&'static str> = Mutex::new("default");

/// The preset of the accelerators that the live menu has now.
#[must_use]
pub fn current_preset() -> &'static str {
    *lock(&PRESET)
}

/// Whether the page has an editable element or a dialog in focus now (the last [`set_typing`]).
pub(super) static TYPING: Mutex<bool> = Mutex::new(false);

/// The page is typing now: the live menu has no plain-key accelerators.
#[must_use]
pub fn is_typing() -> bool {
    *lock(&TYPING)
}
