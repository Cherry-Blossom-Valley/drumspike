//! The live menu follows the page and the service: enabled and checked states, Open Recent, the Window menu,
//! Follow Hits Of, the own zoom presets, the accelerators of a preset, and the report of the self-test.

use std::sync::Mutex;

use serde_json::{Value, json};
use tauri::menu::{CheckMenuItemBuilder, Menu, MenuItemBuilder, MenuItemKind, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Runtime};

use super::build::{no_projects, no_tracks};
use super::items::{
    AUDITION, AUDITION_TAB, CLICK, CONFIRM_HIT, COUNTER_MODES, DELETE_HIT, EXPORT_AGAIN, FOLLOW_ID, FOLLOW_MODES,
    FOLLOW_PREFIX, GRID, HEIGHT_MODES, INFO_LINE, INSPECTOR, Kind, LEARN_FILTER, NUDGE_EARLIER, NUDGE_LATER,
    NUDGE_MODES, OPEN_RECENT, OTHER_MICS, OVERVIEW, PROJECT_PREFIX, REANALYZE_SELECTION, RECENT_CLEAR_ID, RECENT_ID,
    RECENT_MAX, RECENT_PREFIX, REDO, RULER_BEATS, RULER_MARKERS, RULER_METER, RULER_TEMPO, SNAP, SPEC,
    STOP_RETURN_MODE, SUSPEND_FOLLOW, TAB_MODES, TOGGLE_HIT_ACTIVE, TRACKS_REMOVE, TRAINING_MODE, UNDO, VELOCITY_LANE,
    WINDOW_ID, ZOOM_OWN_PREFIX, ZOOM_PRESETS_ID,
};
use super::keymap::{PRESET, TYPING, current_preset, is_typing, key_row, live_accel, preset_or_default, tauri_accel};
use super::lock;
#[cfg(target_os = "macos")]
use super::macos::{clear_ns_equivalents, fix_shift_equivalents, item_paths};

/// One project entry of the Window menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowEntry {
    /// The project id of the service.
    pub sid: String,
    /// The project name (the menu text).
    pub name: String,
    /// The page shows this project (the check mark).
    pub current: bool,
}

/// P3-18: one entry of Track > Follow Hits Of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowEntry {
    /// The track name (the menu text and the payload).
    pub name: String,
    /// The selected track follows this one (the check mark).
    pub current: bool,
}

/// What the dynamic parts of the menu show now: Open Recent (folders), the Window menu (open projects) and
/// Track > Follow Hits Of (the other tracks of the project, P3-18), so that a sync rebuilds a submenu only when it
/// changed, and `dispatch` finds the folder, project or track of an entry.
#[derive(Default)]
pub struct MenuState {
    /// The Open Recent folders, newest first.
    pub recent: Mutex<Vec<String>>,
    /// The Window menu's project entries, in the order of opening.
    pub projects: Mutex<Vec<WindowEntry>>,
    /// P3-18: the Follow Hits Of entries, in the project's track order.
    pub follow: Mutex<Vec<FollowEntry>>,
    /// #169: the names of the own zoom presets, in the order of the page's list.
    pub zoom_own: Mutex<Vec<String>>,
}

/// The name under which `lib.rs` manages the state (P3-4b).
pub type RecentState = MenuState;

/// The check states that the page reports (`{snap, follow, click, infoLine, counter, inspector, trackHeight, nudgeValue, suspendFollow}`): one `(item id, checked)` per check item.
/// A missing or unknown value leaves its items out.
pub fn check_states(state: &Value) -> Vec<(&'static str, bool)> {
    let mut out = Vec::new();
    if let Some(b) = state.get("snap").and_then(Value::as_bool) {
        out.push((SNAP.id, b));
    }
    if let Some(b) = state.get("click").and_then(Value::as_bool) {
        out.push((CLICK.id, b));
    }
    // P4-8: the three map rulers and the grid
    for (key, item) in [
        ("tempoRuler", &RULER_TEMPO),
        ("meterRuler", &RULER_METER),
        ("beatsRuler", &RULER_BEATS),
        ("markersRuler", &RULER_MARKERS),
        ("grid", &GRID),
        ("velocityLane", &VELOCITY_LANE),
        ("otherMics", &OTHER_MICS),
        ("trainingMode", &TRAINING_MODE),      // #343: View > Training Mode
        ("suspendFollow", &SUSPEND_FOLLOW),    // P4-wf10: Transport > Suspend Follow when Editing
        ("stopReturnMode", &STOP_RETURN_MODE), // #436: Transport > Stop Returns to Start
        ("auditionTab", &AUDITION_TAB),        // #240: Transport > Audition on Tab
    ] {
        if let Some(b) = state.get(key).and_then(Value::as_bool) {
            out.push((item.id, b));
        }
    }
    if let Some(b) = state.get("inspector").and_then(Value::as_bool) {
        out.push((INSPECTOR.id, b));
    }
    // P4-6c: Hits > Learn Filter shows the Learn filter of the selected track (its source for a follower)
    if let Some(b) = state.get("learnFilter").and_then(Value::as_bool) {
        out.push((LEARN_FILTER.id, b));
    }
    if let Some(b) = state.get("infoLine").and_then(Value::as_bool) {
        out.push((INFO_LINE.id, b));
    }
    if let Some(b) = state.get("overview").and_then(Value::as_bool) {
        out.push((OVERVIEW.id, b));
    }
    if let Some(mode) = state.get("counter").and_then(Value::as_str)
        && COUNTER_MODES.iter().any(|(m, _)| *m == mode)
    {
        for (m, id) in COUNTER_MODES {
            out.push((id, m == mode));
        }
    }
    if let Some(mode) = state.get("tabMode").and_then(Value::as_str)
        && TAB_MODES.iter().any(|(m, _)| *m == mode)
    {
        for (m, id) in TAB_MODES {
            out.push((id, m == mode));
        }
    }
    if let Some(mode) = state.get("trackHeight").and_then(Value::as_str)
        && HEIGHT_MODES.iter().any(|(m, _)| *m == mode)
    {
        for (m, id) in HEIGHT_MODES {
            out.push((id, m == mode));
        }
    }
    if let Some(mode) = state.get("nudgeValue").and_then(Value::as_str)
        && NUDGE_MODES.iter().any(|(m, _)| *m == mode)
    {
        for (m, id) in NUDGE_MODES {
            out.push((id, m == mode));
        }
    }
    if let Some(mode) = state.get("follow").and_then(Value::as_str)
        && FOLLOW_MODES.iter().any(|(m, _)| *m == mode)
    {
        for (m, id) in FOLLOW_MODES {
            out.push((id, m == mode));
        }
    }
    out
}

/// The Window menu's entries from the service's project list (`project_list` shape `ids`: `{id, name}`, in the
/// order of opening) and the page's current project.
#[must_use]
pub fn window_entries(list: &Value, current: Option<&str>) -> Vec<WindowEntry> {
    list.as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let sid = r.get("id").or_else(|| r.get("sid")).and_then(Value::as_str)?;
                    let name = r.get("name").and_then(Value::as_str).unwrap_or(sid);
                    Some(WindowEntry {
                        sid: sid.to_string(),
                        name: name.to_string(),
                        current: Some(sid) == current,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// P3-18: the entries of Track > Follow Hits Of from the page's state (`tracks`: the track names in order,
/// `selected`: the selected track, `follows`: the track it follows): every track but the selected one, the
/// followed one checked. No entries without a selected track (the item needs one).
pub fn follow_entries(state: &Value) -> Vec<FollowEntry> {
    let Some(selected) = state.get("selected").and_then(Value::as_str) else {
        return Vec::new();
    };
    let follows = state.get("follows").and_then(Value::as_str);
    state
        .get("tracks")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .filter(|n| *n != selected)
                .map(|n| FollowEntry {
                    name: n.to_string(),
                    current: Some(n) == follows,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// #169: the names of the own zoom presets from the page's state (`zoomOwn`, the list in its order).
pub fn zoom_own_names(state: &Value) -> Vec<String> {
    state
        .get("zoomOwn")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Every submenu of `items`, at any depth.
fn submenus<R: Runtime>(items: Vec<MenuItemKind<R>>, out: &mut Vec<Submenu<R>>) {
    for it in items {
        if let MenuItemKind::Submenu(s) = it {
            if let Ok(inner) = s.items() {
                submenus(inner, out);
            }
            out.push(s);
        }
    }
}

fn all_submenus<R: Runtime>(menu: &Menu<R>) -> Vec<Submenu<R>> {
    let mut out = Vec::new();
    if let Ok(items) = menu.items() {
        submenus(items, &mut out);
    }
    out
}

/// The submenu with the id `id`, wherever it is in the bar.
fn find_submenu<R: Runtime>(menu: &Menu<R>, id: &str) -> Option<Submenu<R>> {
    all_submenus(menu).into_iter().find(|s| s.id().as_ref() == id)
}

/// The item `id` of the menu bar, at any depth (the first one: an item in two menus is one item).
fn find_item<R: Runtime>(menu: &Menu<R>, id: &str) -> Option<MenuItemKind<R>> {
    menu.get(id)
        .or_else(|| all_submenus(menu).into_iter().find_map(|s| s.get(id)))
}

fn set_enabled<R: Runtime>(menu: &Menu<R>, id: &str, on: bool) -> tauri::Result<()> {
    match find_item(menu, id) {
        Some(MenuItemKind::MenuItem(m)) => m.set_enabled(on),
        Some(MenuItemKind::Check(c)) => c.set_enabled(on),
        _ => Ok(()),
    }
}

/// P4-40a: the text of Edit > Undo or Redo: the page's text (one line, at most 90 characters), the plain label for an empty one.
#[must_use]
pub fn step_title(label: &str, page: &str) -> String {
    let one: String = page.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.is_empty() {
        return label.to_string();
    }
    if one.chars().count() > 90 {
        let cut: String = one.chars().take(89).collect();
        return format!("{cut}\u{2026}");
    }
    one
}

/// Grey the items that need a project while none is open, and refresh the Open Recent list.
/// `recent` holds the project folders, newest first.
///
/// # Errors
///
/// When the windowing library cannot enable or disable an item, or change the Open Recent
/// submenu. Nothing happens (and nothing fails) when the app has no menu yet.
pub fn sync<R: Runtime>(app: &AppHandle<R>, has_project: bool, recent: &[String]) -> tauri::Result<()> {
    let Some(menu) = app.menu() else { return Ok(()) };
    for it in SPEC.iter().filter(|i| i.needs_project) {
        set_enabled(&menu, it.id, it.ready && has_project)?;
    }
    let state = app.state::<MenuState>();
    let recent: Vec<String> = recent.iter().take(RECENT_MAX).cloned().collect();
    if *lock(&state.recent) == recent {
        return Ok(());
    }
    if let Some(sub) = find_submenu(&menu, RECENT_ID) {
        for old in sub.items()? {
            sub.remove(&old)?;
        }
        if recent.is_empty() {
            sub.append(
                &MenuItemBuilder::with_id("recent:none", "No Recent Projects")
                    .enabled(false)
                    .build(app)?,
            )?;
        }
        for (n, folder) in recent.iter().enumerate() {
            let name = std::path::Path::new(folder)
                .file_name()
                .map_or_else(|| folder.clone(), |s| s.to_string_lossy().into_owned());
            sub.append(&MenuItemBuilder::with_id(format!("{RECENT_PREFIX}{n}"), name).build(app)?)?;
        }
        if !recent.is_empty() {
            // #441: Clear Recent at the end of the submenu, only while the list has entries
            sub.append(&PredefinedMenuItem::separator(app)?)?;
            sub.append(&MenuItemBuilder::with_id(RECENT_CLEAR_ID, "Clear Recent").build(app)?)?;
        }
    }
    set_enabled(&menu, OPEN_RECENT.id, !recent.is_empty())?; // P3-19
    *lock(&state.recent) = recent;
    Ok(())
}

/// Set the check items to the page's state (`{snap, follow, click, infoLine, counter, inspector, trackHeight, nudgeValue}`, see [`check_states`]). A native click on a
/// check item toggles its mark before the page has run the item; this call puts the page's state back.
///
/// # Errors
///
/// When the windowing library cannot set the checked state of an item. Nothing happens (and
/// nothing fails) when the app has no menu yet.
pub fn sync_checks<R: Runtime>(app: &AppHandle<R>, state: &Value) -> tauri::Result<()> {
    let Some(menu) = app.menu() else { return Ok(()) };
    for (id, on) in check_states(state) {
        if let Some(MenuItemKind::Check(c)) = find_item(&menu, id) {
            c.set_checked(on)?;
        }
    }
    // P4-40a: Edit > Undo and Redo name the step they take ("Undo Delete Hit (s top, 12.345 s)"); the page sends the texts
    for (key, it) in [("undoTitle", &UNDO), ("redoTitle", &REDO)] {
        if let (Some(t), Some(MenuItemKind::MenuItem(m))) =
            (state.get(key).and_then(Value::as_str), find_item(&menu, it.id))
        {
            m.set_text(step_title(it.label, t))?;
        }
    }
    // P4-39a: Track > Remove Track needs a selected track (and a project, which `sync` checks: no project, no selection)
    if let Some(sel) = state.get("selected") {
        set_enabled(&menu, TRACKS_REMOVE.id, sel.is_string())?;
    }
    // #158 (rank 9): Edit > Undo and Redo are greyed while their stack is empty (the page sends the flags with every sync)
    if let Some(on) = state.get("undo").and_then(Value::as_bool) {
        set_enabled(&menu, UNDO.id, on)?;
    }
    if let Some(on) = state.get("redo").and_then(Value::as_bool) {
        set_enabled(&menu, REDO.id, on)?;
    }
    // #158 (rank 9): the items that need a hit selection (a hit or a range) are greyed without one. `sync` has
    // already enabled them with the project; a state without the key (an old page) keeps that state.
    if let Some(on) = state.get("selHit").and_then(Value::as_bool) {
        for it in [DELETE_HIT, NUDGE_EARLIER, NUDGE_LATER, AUDITION, TOGGLE_HIT_ACTIVE] {
            set_enabled(&menu, it.id, on)?;
        }
        // #354: Hits > Confirm Hit needs a hit selection AND Training Mode on (greyed while the mode is off)
        if let Some(tm) = state.get("trainingMode").and_then(Value::as_bool) {
            set_enabled(&menu, CONFIRM_HIT.id, on && tm)?;
        }
    }
    // #265: Hits > Re-analyze Selection is greyed without a range selection (LN.sel)
    if let Some(on) = state.get("selRange").and_then(Value::as_bool) {
        set_enabled(&menu, REANALYZE_SELECTION.id, on)?;
    }
    // P4-31: File > Export Again follows the export shape of the project (greyed before the first export; the
    // page sends the key whenever it syncs, and `S.exportAgain` is false without a project)
    if let Some(on) = state.get("exportAgain").and_then(Value::as_bool) {
        set_enabled(&menu, EXPORT_AGAIN.id, on)?;
    }
    Ok(())
}

/// Give every item the accelerators of a key preset (P3-13c). The items stay (their marks, enabled state, and the
/// Open Recent and Window entries are kept), only the key equivalents change: the same result as a new menu bar,
/// without the flicker. Does nothing when the preset is the one the menu has. An unknown name is the default.
///
/// # Errors
///
/// When the windowing library cannot set the accelerator of an item.
pub fn set_preset<R: Runtime>(app: &AppHandle<R>, name: &str) -> tauri::Result<()> {
    let preset = preset_or_default(name);
    if current_preset() == preset {
        return Ok(());
    }
    apply_accels(app, preset, is_typing())?;
    *lock(&PRESET) = preset;
    Ok(())
}

/// #300: the page says whether an editable element or a dialog has the focus. The menu shows the plain keys (V,
/// Space, Tab) as accelerators only while it is not so: a text field must get them as text, and a dialog its Return.
/// Does nothing when the state is the one the menu has.
///
/// # Errors
///
/// When the windowing library cannot set the accelerator of an item.
pub fn set_typing<R: Runtime>(app: &AppHandle<R>, on: bool) -> tauri::Result<()> {
    if is_typing() == on {
        return Ok(());
    }
    apply_accels(app, current_preset(), on)?;
    *lock(&TYPING) = on;
    Ok(())
}

/// Give every item the accelerator of `preset` for the typing state (the plain keys only when not `typing`).
fn apply_accels<R: Runtime>(app: &AppHandle<R>, preset: &str, typing: bool) -> tauri::Result<()> {
    let Some(menu) = app.menu() else { return Ok(()) };
    let mut without = std::collections::HashSet::new();
    for it in SPEC {
        let accel = live_accel(it.id, preset, typing);
        if accel.is_none() {
            without.insert(it.id);
        }
        match find_item(&menu, it.id) {
            Some(MenuItemKind::MenuItem(m)) => m.set_accelerator(accel)?,
            Some(MenuItemKind::Check(c)) => c.set_accelerator(accel)?,
            _ => {}
        }
    }
    // muda on macOS ignores `None`: the key equivalent of the `NSMenuItem` stays (V would still fire the menu while
    // a text field has the focus, and a key of the other preset would stay), so the items without a key are cleared here
    #[cfg(target_os = "macos")]
    {
        clear_ns_equivalents(app, item_paths(&menu, &without));
        fix_shift_equivalents(app);
    }
    Ok(())
}

/// The Window menu: one check item per open project (the current one checked) before Minimize and Zoom.
/// The entries are made again only when the projects or their names changed; the marks are set every time.
///
/// # Errors
///
/// When the windowing library cannot remove, add or mark a Window menu item. Nothing happens (and
/// nothing fails) when the app has no menu or the Window submenu is missing.
pub fn sync_window<R: Runtime>(app: &AppHandle<R>, entries: &[WindowEntry]) -> tauri::Result<()> {
    let Some(menu) = app.menu() else { return Ok(()) };
    let Some(sub) = find_submenu(&menu, WINDOW_ID) else {
        return Ok(());
    };
    let state = app.state::<MenuState>();
    let same = {
        let old = lock(&state.projects);
        old.len() == entries.len() && old.iter().zip(entries).all(|(a, b)| a.sid == b.sid && a.name == b.name)
    };
    let shown = sub.items()?.iter().any(|k| k.id().as_ref().starts_with(PROJECT_PREFIX));
    if !same || !shown {
        for old in sub.items()? {
            if old.id().as_ref().starts_with(PROJECT_PREFIX) {
                sub.remove(&old)?;
            }
        }
        if entries.is_empty() {
            sub.insert(&no_projects(app)?, 0)?;
        }
        for (n, e) in entries.iter().enumerate() {
            let it = CheckMenuItemBuilder::with_id(format!("{PROJECT_PREFIX}{n}"), &e.name)
                .checked(e.current)
                .build(app)?;
            sub.insert(&it, n)?;
        }
    }
    for (n, e) in entries.iter().enumerate() {
        if let Some(MenuItemKind::Check(c)) = sub.get(&format!("{PROJECT_PREFIX}{n}")) {
            c.set_checked(e.current)?;
        }
    }
    *lock(&state.projects) = entries.to_vec();
    Ok(())
}

/// P3-18: Track > Follow Hits Of: one check item per other track of the project (the followed one checked).
/// The entries are made again only when the tracks changed; the marks are set every time.
///
/// # Errors
///
/// When the windowing library cannot remove, add or mark a Follow Hits Of item. Nothing happens
/// (and nothing fails) when the app has no menu or the submenu is missing.
pub fn sync_follow<R: Runtime>(app: &AppHandle<R>, entries: &[FollowEntry]) -> tauri::Result<()> {
    let Some(menu) = app.menu() else { return Ok(()) };
    let Some(sub) = find_submenu(&menu, FOLLOW_ID) else {
        return Ok(());
    };
    let state = app.state::<MenuState>();
    let same = {
        let old = lock(&state.follow);
        old.len() == entries.len() && old.iter().zip(entries).all(|(a, b)| a.name == b.name)
    };
    let shown = sub.items()?.iter().any(|k| k.id().as_ref().starts_with(FOLLOW_PREFIX));
    if !same || !shown {
        for old in sub.items()? {
            sub.remove(&old)?;
        }
        if entries.is_empty() {
            sub.append(&no_tracks(app)?)?;
        }
        for (n, e) in entries.iter().enumerate() {
            let it = CheckMenuItemBuilder::with_id(format!("{FOLLOW_PREFIX}{n}"), &e.name)
                .checked(e.current)
                .build(app)?;
            sub.append(&it)?;
        }
    }
    for (n, e) in entries.iter().enumerate() {
        if let Some(MenuItemKind::Check(c)) = sub.get(&format!("{FOLLOW_PREFIX}{n}")) {
            c.set_checked(e.current)?;
        }
    }
    *lock(&state.follow) = entries.to_vec();
    Ok(())
}

/// #169: View > Zoom Presets: the own presets after Organize, one item per name. The first five carry Option+1 to
/// Option+5. The entries are made again only when the names changed (or the submenu lost them).
///
/// # Errors
///
/// When the windowing library cannot remove or add a Zoom Presets item. Nothing happens (and
/// nothing fails) when the app has no menu or the submenu is missing.
pub fn sync_zoom_own<R: Runtime>(app: &AppHandle<R>, names: &[String]) -> tauri::Result<()> {
    let Some(menu) = app.menu() else { return Ok(()) };
    let Some(sub) = find_submenu(&menu, ZOOM_PRESETS_ID) else {
        return Ok(());
    };
    let state = app.state::<MenuState>();
    let same = *lock(&state.zoom_own) == names;
    let shown = sub
        .items()?
        .iter()
        .any(|k| k.id().as_ref().starts_with(ZOOM_OWN_PREFIX));
    if !same || shown != !names.is_empty() {
        for old in sub.items()? {
            if old.id().as_ref().starts_with(ZOOM_OWN_PREFIX) {
                sub.remove(&old)?;
            }
        }
        for (n, name) in names.iter().enumerate() {
            // the own presets have no row in the key table (they come from the settings): Option+1 to Option+5 is theirs
            let item = MenuItemBuilder::with_id(format!("{ZOOM_OWN_PREFIX}{n}"), name).build(app)?;
            if n < 5 {
                item.set_accelerator(tauri_accel(&format!("Option+{}", n + 1)))?;
            }
            sub.append(&item)?;
        }
    }
    *lock(&state.zoom_own) = names.to_vec();
    Ok(())
}

/// What the live menu holds, for the self-test (debug builds): one row per custom item with its label,
/// shortcut, kind, enabled and checked state as the `NSMenu` has them, the menu titles, the entries of Open
/// Recent, and the entries of the Window menu.
pub fn report<R: Runtime>(app: &AppHandle<R>) -> Value {
    let Some(menu) = app.menu() else {
        return json!({"error": "the app has no menu"});
    };
    let mut items = Vec::new();
    for it in SPEC {
        let live = find_item(&menu, it.id);
        let (enabled, checked, live_kind) = match &live {
            Some(MenuItemKind::MenuItem(m)) => (m.is_enabled().ok(), None, Some("plain")),
            Some(MenuItemKind::Check(c)) => (c.is_enabled().ok(), c.is_checked().ok(), Some("check")),
            _ => (None, None, None),
        };
        let kind = match it.kind {
            Kind::Plain => "plain",
            Kind::Check => "check",
        };
        let row = key_row(it.id);
        items.push(json!({
            "id": it.id, "label": it.label, "accel": live_accel(it.id, current_preset(), is_typing()), "ready": it.ready, "card": it.card,
            "needs_project": it.needs_project, "kind": kind,
            "menu_key": row.and_then(|r| r.menu_key.clone()), "page_keys": row.map(|r| r.page_keys.clone()).unwrap_or_default(),
            "in_menu": live_kind == Some(kind), "enabled": enabled, "checked": checked,
        }));
    }
    let recent = find_submenu(&menu, RECENT_ID)
        .and_then(|s| s.items().ok())
        .map(|v| v.iter().map(|k| k.id().as_ref().to_string()).collect::<Vec<_>>())
        .unwrap_or_default();
    let window = find_submenu(&menu, WINDOW_ID)
        .and_then(|s| s.items().ok())
        .map(|v| {
            v.iter()
                .filter_map(|k| match k {
                    MenuItemKind::Check(c) => Some(json!({
                        "id": c.id().as_ref(), "label": c.text().unwrap_or_default(),
                        "checked": c.is_checked().unwrap_or(false),
                    })),
                    MenuItemKind::MenuItem(m) if m.id().as_ref().starts_with(PROJECT_PREFIX) => Some(json!({
                        "id": m.id().as_ref(), "label": m.text().unwrap_or_default(), "checked": false,
                    })),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let menus = menu
        .items()
        .map(|v| {
            v.iter()
                .filter_map(|k| match k {
                    MenuItemKind::Submenu(s) => s.text().ok(),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let follow = find_submenu(&menu, FOLLOW_ID)
        .and_then(|s| s.items().ok())
        .map(|v| {
            v.iter()
                .map(|k| match k {
                    MenuItemKind::Check(c) => json!({"id": c.id().as_ref(), "label": c.text().unwrap_or_default(),
                        "checked": c.is_checked().unwrap_or(false)}),
                    other => json!({"id": other.id().as_ref(), "label": "", "checked": false}),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let zoom_own = find_submenu(&menu, ZOOM_PRESETS_ID)
        .and_then(|s| s.items().ok())
        .map(|v| v.iter().map(|k| k.id().as_ref().to_string()).collect::<Vec<_>>())
        .unwrap_or_default();
    json!({"items": items, "recent": recent, "window": window, "follow": follow, "menus": menus, "preset": current_preset(), "zoom_own": zoom_own})
}
