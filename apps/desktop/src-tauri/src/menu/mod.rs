//! The native menu bar (P3-4b, P3-13a, P3-13b). One table (`SPEC`) holds every custom item of the app: id, label,
//! kind (plain or check item) and whether the item needs an open project. `build` makes the menu bar of
//! section 3.2 of the UX reference from the table, `dispatch` turns a click into the event `menu`
//! for the page (`static/shell.js` does the work), and `sync`, `sync_checks` and `sync_window` grey the items that
//! need an open project, refresh Open Recent and the Window menu, and set the check items to the page's state.
//!
//! P3-13b: the keys are not in this file. `static/keymap.js` is the key command table of the app (one row per
//! command: menu id, menu accelerator, page keys, Cubase keys); its rows between the markers `KEYMAP-BEGIN` and
//! `KEYMAP-END` are strict JSON. This file includes the table at compile time (`include_str!`), parses it once
//! ([`keymap`]) and gives each item the accelerator of its row ([`accel_of`], converted to the Tauri notation).
//! The page reads the same file (`DSKeys`), so the menu, the page keys and the help panel cannot differ; the tests
//! below fail when a key is in two commands or an accelerator of the menu differs from the table.
//! #300: every item with a key in the table shows it ([`shown_accel_of`]), also the keys without Cmd, Control or
//! Option (Space, Tab, Return, Delete, letters). Those plain accelerators are on the menu only while the page has
//! no text field or dialog in focus: the page tells the shell on every focus change ([`set_typing`]), the shell takes
//! them off the items and puts them back, so a text field keeps its keys and a key runs once.
//! An item whose command does not exist yet would stay in the menu, greyed out (`ready: false`); none is.

mod build;
mod items;
mod keymap;
mod macos;
mod sync;

pub use build::*;
pub use items::*;
pub use keymap::*;
pub use macos::*;
pub use sync::*;

use std::sync::Mutex;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
