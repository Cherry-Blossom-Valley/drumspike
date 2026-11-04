//! The menu bar from `SPEC` (`build`) and a click on an item (`dispatch`).

use std::sync::{LazyLock, Mutex};

use serde_json::{Value, json};
use tauri::menu::{
    CheckMenuItem, CheckMenuItemBuilder, Menu, MenuBuilder, MenuItem, MenuItemBuilder, PredefinedMenuItem, Submenu,
    SubmenuBuilder,
};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::items::{
    ABOUT, ADD_HIT, ADD_TRACKS, AMP_IN, AMP_OUT, AMP_RESET, ANALYZE_ALL, ANALYZE_TRACK, AUDITION, AUDITION_TAB, CLICK,
    CLOSE, CONSOLIDATE, COUNTER_BARSBEATS, COUNTER_EXCHANGE, COUNTER_SAMPLES, COUNTER_SECONDS, COUNTER_TIMECODE,
    DELETE_HIT, DETECT_TEMPO, DRUMSPIKE_HELP, EXPORT, EXPORT_AGAIN, EXPORT_SELECTED, FOLLOW_ID, FOLLOW_OFF, FOLLOW_OWN,
    FOLLOW_PAGE, FOLLOW_PREFIX, FOLLOW_SMOOTH, FOLLOW_TOGGLE, FOLLOW_TRACK_ID, GO_END, GO_PLAY_START, GO_START, GRID,
    HALF_SPEED, HEIGHT_LARGE, HEIGHT_MEDIUM, HEIGHT_SHORTER, HEIGHT_SMALL, HEIGHT_TALLER, IMPORT_OLD, IMPORT_TEMPO_MAP,
    INFO_LINE, INSPECTOR, Item, LEARN_FILTER, LEARN_FORGET, LEARN_FORGET_THIS, LEARN_NOT, LEARN_THIS, MARKER_NEXT,
    MARKER_PREV, MARKERS_SET, MUTE_TRACK, NEW, NEXT_HIT, NEXT_HIT_ALL, NEXT_TRACK, NUDGE_EARLIER, NUDGE_LATER,
    NUDGE_VAL_1MS, NUDGE_VAL_1SMP, NUDGE_VAL_10MS, NUDGE_VAL_10SMP, NUDGE_VAL_100SMP, OPEN, OPEN_RECENT, OTHER_MICS,
    OVERVIEW, PLAY_STOP, PREV_HIT, PREV_HIT_ALL, PREV_TRACK, PROJECT_ID, PROJECT_PREFIX, QUIT, REANALYZE_SELECTION,
    RECENT_CLEAR_ID, RECENT_CLEAR_PAYLOAD, RECENT_ID, RECENT_PREFIX, RECOMPUTE, REDO, RENAME, RESET_MIX, REVIEW_NEXT,
    REVIEW_PREV, RULER_BEATS, RULER_MARKERS, RULER_METER, RULER_TEMPO, SAVE, SAVE_AS, SET_BAR1, SET_BAR1_AUDIO,
    SETTINGS, SHORTCUTS, SNAP, SOLO_TRACK, SPEC, STOP_RETURN, STOP_RETURN_MODE, SUSPEND_FOLLOW, TAB_ACTIVE, TAB_ALL,
    TOGGLE_HIT_ACTIVE, TRACKS_REMOVE, UNDO, UNDO_HISTORY, VELOCITY_LANE, WINDOW_ID, ZOOM_5MS, ZOOM_5S, ZOOM_30S,
    ZOOM_50MS, ZOOM_500MS, ZOOM_ADD, ZOOM_FULL, ZOOM_IN, ZOOM_ORGANIZE, ZOOM_OUT, ZOOM_OWN_ID, ZOOM_OWN_PREFIX,
    ZOOM_PRESETS_ID, ZOOM_PROJECT, ZOOM_TOGGLE,
};
use super::keymap::{current_preset, is_typing, live_accel};
use super::lock;
use super::sync::MenuState;

/// #206 diagnosis: every menu id that reached `dispatch`, in order.
pub static MENU_LOG: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| Mutex::new(Vec::new()));

pub(super) fn no_tracks<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<MenuItem<R>> {
    MenuItemBuilder::with_id(format!("{FOLLOW_PREFIX}none"), "No Other Track")
        .enabled(false)
        .build(app)
}

fn make_item<R: Runtime>(app: &AppHandle<R>, it: &Item) -> tauri::Result<MenuItem<R>> {
    let mut b = MenuItemBuilder::with_id(it.id, it.label).enabled(it.ready && !it.needs_project);
    if let Some(a) = live_accel(it.id, current_preset(), is_typing()) {
        b = b.accelerator(a);
    }
    b.build(app)
}

fn make_check<R: Runtime>(app: &AppHandle<R>, it: &Item, on: bool) -> tauri::Result<CheckMenuItem<R>> {
    let mut b = CheckMenuItemBuilder::with_id(it.id, it.label)
        .checked(on)
        .enabled(it.ready && !it.needs_project);
    if let Some(a) = live_accel(it.id, current_preset(), is_typing()) {
        b = b.accelerator(a);
    }
    b.build(app)
}

pub(super) fn no_projects<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<MenuItem<R>> {
    MenuItemBuilder::with_id(format!("{PROJECT_PREFIX}none"), "No Open Projects")
        .enabled(false)
        .build(app)
}

/// The Open Recent submenu with its "No Recent Projects" placeholder; `sync_recent` fills it.
fn recent_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    SubmenuBuilder::with_id(app, RECENT_ID, "Open Recent")
        .item(
            &MenuItemBuilder::with_id("recent:none", "No Recent Projects")
                .enabled(false)
                .build(app)?,
        )
        .build()
}

/// The Drum Spike menu: About, Settings, the macOS services and window items, Quit.
fn app_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    SubmenuBuilder::new(app, "Drum Spike")
        .item(&make_item(app, &ABOUT)?)
        .separator()
        .item(&make_item(app, &SETTINGS)?)
        .separator()
        .item(&PredefinedMenuItem::services(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::hide(app, None)?)
        .item(&PredefinedMenuItem::hide_others(app, None)?)
        .item(&PredefinedMenuItem::show_all(app, None)?)
        .separator()
        .item(&make_item(app, &QUIT)?)
        .build()
}

/// The File menu: project open/save, Add Tracks, the imports, Export and Export Again. `add_tracks`
/// is the same item as in the Track menu; `open_recent` arrives disabled (P3-19: `sync` enables it
/// when Open Recent has an entry).
fn file_menu<R: Runtime>(
    app: &AppHandle<R>,
    recent: &Submenu<R>,
    open_recent: &MenuItem<R>,
    add_tracks: &MenuItem<R>,
) -> tauri::Result<Submenu<R>> {
    SubmenuBuilder::new(app, "File")
        .item(&make_item(app, &NEW)?)
        .item(&make_item(app, &OPEN)?)
        .item(recent)
        .item(open_recent)
        .separator()
        .item(&make_item(app, &CLOSE)?)
        .item(&make_item(app, &SAVE)?)
        .item(&make_item(app, &SAVE_AS)?)
        .item(&make_item(app, &RENAME)?)
        .separator()
        .item(add_tracks)
        .item(&make_item(app, &IMPORT_OLD)?)
        .item(&make_item(app, &IMPORT_TEMPO_MAP)?)
        .item(&make_item(app, &CONSOLIDATE)?)
        .separator()
        .item(&make_item(app, &EXPORT)?)
        .item(&make_item(app, &EXPORT_AGAIN)?)
        .build()
}

/// The Edit menu: undo/redo and history, the macOS clipboard items, the hit edits, the Nudge Value
/// submenu and Recompute Velocities.
fn edit_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let nudge_value = SubmenuBuilder::with_id(app, "nudge_value", "Nudge Value")
        .item(&make_check(app, &NUDGE_VAL_1SMP, true)?)
        .item(&make_check(app, &NUDGE_VAL_10SMP, false)?)
        .item(&make_check(app, &NUDGE_VAL_100SMP, false)?)
        .item(&make_check(app, &NUDGE_VAL_1MS, false)?)
        .item(&make_check(app, &NUDGE_VAL_10MS, false)?)
        .build()?;
    SubmenuBuilder::new(app, "Edit")
        .item(&make_item(app, &UNDO)?)
        .item(&make_item(app, &REDO)?)
        .item(&make_item(app, &UNDO_HISTORY)?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .separator()
        .item(&make_item(app, &DELETE_HIT)?)
        .item(&make_item(app, &ADD_HIT)?)
        .item(&make_item(app, &MARKERS_SET)?) // #158 (rank 8)
        .item(&make_item(app, &NUDGE_EARLIER)?)
        .item(&make_item(app, &NUDGE_LATER)?)
        .item(&nudge_value)
        .separator()
        .item(&make_item(app, &RECOMPUTE)?)
        .build()
}

/// The View menu: the zoom items with the Waveform Zoom, Zoom Presets and Track Height submenus,
/// the page check items, the Main Counter and Tab Moves To submenus, the rulers, the grid, the
/// velocity lane and the other mics, and the macOS fullscreen item.
fn view_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    // #169: the own presets follow Organize (sync_zoom_own appends them)
    let presets = SubmenuBuilder::with_id(app, ZOOM_PRESETS_ID, "Zoom Presets")
        .item(&make_item(app, &ZOOM_FULL)?)
        .item(&make_item(app, &ZOOM_30S)?)
        .item(&make_item(app, &ZOOM_5S)?)
        .item(&make_item(app, &ZOOM_500MS)?)
        .item(&make_item(app, &ZOOM_50MS)?)
        .item(&make_item(app, &ZOOM_5MS)?)
        .separator()
        .item(&make_item(app, &ZOOM_ADD)?)
        .item(&make_item(app, &ZOOM_ORGANIZE)?)
        .build()?;
    let amp = SubmenuBuilder::with_id(app, "waveform_zoom", "Waveform Zoom")
        .item(&make_item(app, &AMP_IN)?)
        .item(&make_item(app, &AMP_OUT)?)
        .item(&make_item(app, &AMP_RESET)?)
        .build()?;
    let height = SubmenuBuilder::with_id(app, "track_height", "Track Height")
        .item(&make_item(app, &HEIGHT_TALLER)?)
        .item(&make_item(app, &HEIGHT_SHORTER)?)
        .separator()
        .item(&make_check(app, &HEIGHT_SMALL, false)?)
        .item(&make_check(app, &HEIGHT_MEDIUM, true)?)
        .item(&make_check(app, &HEIGHT_LARGE, false)?)
        .build()?;
    let counter = SubmenuBuilder::with_id(app, "main_counter", "Main Counter")
        .item(&make_check(app, &COUNTER_SECONDS, true)?)
        .item(&make_check(app, &COUNTER_SAMPLES, false)?)
        .item(&make_check(app, &COUNTER_BARSBEATS, false)?) // #149
        .item(&make_check(app, &COUNTER_TIMECODE, false)?) // #149: a fixed 30 fps, not the session rate
        .separator()
        .item(&make_item(app, &COUNTER_EXCHANGE)?)
        .build()?;
    let tab_mode = SubmenuBuilder::with_id(app, "tab_mode", "Tab Moves To") // #175
        .item(&make_check(app, &TAB_ACTIVE, true)?) // the default: active hits only
        .item(&make_check(app, &TAB_ALL, false)?)
        .build()?;
    SubmenuBuilder::new(app, "View")
        .item(&make_item(app, &ZOOM_IN)?)
        .item(&make_item(app, &ZOOM_OUT)?)
        .item(&make_item(app, &ZOOM_PROJECT)?)
        .item(&make_item(app, &ZOOM_TOGGLE)?) // #158 (rank 8)
        .item(&amp)
        .item(&presets)
        .item(&height)
        .separator()
        .item(&make_check(app, &INSPECTOR, true)?)
        .item(&make_check(app, &INFO_LINE, true)?)
        .item(&make_check(app, &OVERVIEW, true)?)
        .item(&counter)
        .item(&tab_mode) // #175: View > Tab Moves To
        .separator()
        .item(&make_check(app, &RULER_TEMPO, true)?)
        .item(&make_check(app, &RULER_METER, true)?)
        .item(&make_check(app, &RULER_BEATS, true)?)
        .item(&make_check(app, &RULER_MARKERS, true)?)
        .item(&make_check(app, &GRID, true)?)
        .item(&make_check(app, &VELOCITY_LANE, false)?)
        .item(&make_check(app, &OTHER_MICS, false)?)
        .separator()
        .item(&PredefinedMenuItem::fullscreen(app, None)?)
        .build()
}

/// The Transport menu: play and speed, the go-to items and the markers, the follow items and the
/// click.
fn transport_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let follow = SubmenuBuilder::with_id(app, "follow", "Follow Playhead")
        .item(&make_check(app, &FOLLOW_SMOOTH, true)?)
        .item(&make_check(app, &FOLLOW_PAGE, false)?)
        .item(&make_check(app, &FOLLOW_OFF, false)?)
        .build()?;
    SubmenuBuilder::new(app, "Transport")
        .item(&make_item(app, &PLAY_STOP)?)
        .item(&make_item(app, &HALF_SPEED)?)
        .item(&make_item(app, &STOP_RETURN)?)
        .separator()
        .item(&make_item(app, &GO_START)?)
        .item(&make_item(app, &GO_END)?)
        .item(&make_item(app, &GO_PLAY_START)?)
        .item(&make_item(app, &MARKER_NEXT)?) // #158 (rank 8)
        .item(&make_item(app, &MARKER_PREV)?)
        .separator()
        .item(&follow)
        .item(&make_item(app, &FOLLOW_TOGGLE)?)
        .item(&make_check(app, &SUSPEND_FOLLOW, false)?)
        .item(&make_check(app, &STOP_RETURN_MODE, false)?)
        .item(&make_check(app, &AUDITION_TAB, false)?)
        .separator()
        .item(&make_check(app, &CLICK, true)?)
        .build()
}

/// The Hits menu: the hit navigation, the review and audition, Snap, the analysis and the learn
/// items.
fn hits_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    SubmenuBuilder::new(app, "Hits")
        .item(&make_item(app, &NEXT_HIT)?)
        .item(&make_item(app, &PREV_HIT)?)
        .item(&make_item(app, &NEXT_HIT_ALL)?)
        .item(&make_item(app, &PREV_HIT_ALL)?)
        .item(&make_item(app, &REVIEW_NEXT)?)
        .item(&make_item(app, &REVIEW_PREV)?)
        .item(&make_item(app, &AUDITION)?) // #158 (rank 8)
        .item(&make_item(app, &TOGGLE_HIT_ACTIVE)?) // #242
        .separator()
        .item(&make_check(app, &SNAP, true)?)
        .separator()
        .item(&make_item(app, &ANALYZE_TRACK)?)
        .item(&make_item(app, &REANALYZE_SELECTION)?) // #265: the hits of the range only
        .item(&make_item(app, &ANALYZE_ALL)?)
        .separator()
        .item(&make_item(app, &LEARN_THIS)?)
        .item(&make_item(app, &LEARN_NOT)?)
        .item(&make_item(app, &LEARN_FORGET_THIS)?)
        .item(&make_check(app, &LEARN_FILTER, false)?)
        .item(&make_item(app, &LEARN_FORGET)?)
        .build()
}

/// The Track menu: Add Tracks (the same item as in the File menu), the track navigation, the mix
/// items, Follow Hits Of and Own Detection, the tempo items, Export Selected Tracks and Remove
/// Tracks.
fn track_menu<R: Runtime>(app: &AppHandle<R>, add_tracks: &MenuItem<R>) -> tauri::Result<Submenu<R>> {
    // P3-18: Follow Hits Of lists the other tracks of the project (sync_follow), Own Detection is the plain item
    let follow_of = SubmenuBuilder::with_id(app, FOLLOW_ID, "Follow Hits Of")
        .item(&no_tracks(app)?)
        .build()?;
    SubmenuBuilder::new(app, "Track")
        .item(add_tracks)
        .separator()
        .item(&make_item(app, &PREV_TRACK)?) // #158 (rank 8)
        .item(&make_item(app, &NEXT_TRACK)?)
        .separator()
        .item(&make_item(app, &MUTE_TRACK)?)
        .item(&make_item(app, &SOLO_TRACK)?)
        .item(&make_item(app, &RESET_MIX)?)
        .separator()
        .item(&follow_of)
        .item(&make_item(app, &FOLLOW_OWN)?)
        .separator()
        .item(&make_item(app, &DETECT_TEMPO)?)
        .item(&make_item(app, &SET_BAR1)?)
        .item(&make_item(app, &SET_BAR1_AUDIO)?) // #468
        .separator()
        .item(&make_item(app, &EXPORT_SELECTED)?)
        .separator()
        .item(&make_item(app, &TRACKS_REMOVE)?)
        .build()
}

/// The Window menu: the project list placeholder (`sync_window`) and the macOS window items.
fn window_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    SubmenuBuilder::with_id(app, WINDOW_ID, "Window")
        .item(&no_projects(app)?)
        .separator()
        .item(&PredefinedMenuItem::minimize(app, None)?)
        .item(&PredefinedMenuItem::maximize(app, Some("Zoom"))?)
        .build()
}

/// The Help menu: Drum Spike Help (the manual) and Keyboard Shortcuts.
fn help_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    SubmenuBuilder::new(app, "Help")
        .item(&make_item(app, &DRUMSPIKE_HELP)?) // #255: before Keyboard Shortcuts
        .item(&make_item(app, &SHORTCUTS)?)
        .build()
}

/// The whole menu bar (reference section 3.2). The check items start as the page starts (snap on, follow
/// smooth, click on); the page's first sync sets them to its real state.
///
/// # Errors
///
/// When the windowing library cannot create a menu, a submenu or an item.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    // one item in two menus (File and Track; Add Tracks): muda keeps both native items in step (enabled state)
    let add_tracks = make_item(app, &ADD_TRACKS)?;
    let open_recent = make_item(app, &OPEN_RECENT)?;
    open_recent.set_enabled(false)?; // P3-19: `sync` enables it when Open Recent has an entry
    let recent = recent_menu(app)?;
    let app_menu = app_menu(app)?;
    let file = file_menu(app, &recent, &open_recent, &add_tracks)?;
    let edit = edit_menu(app)?;
    let view = view_menu(app)?;
    let transport = transport_menu(app)?;
    let hits = hits_menu(app)?;
    let track = track_menu(app, &add_tracks)?;
    let window = window_menu(app)?;
    let help = help_menu(app)?;
    MenuBuilder::new(app)
        .items(&[
            &app_menu, &file, &edit, &view, &transport, &hits, &track, &window, &help,
        ])
        .build()
}

/// The number of a numbered entry: `recent:3` with the prefix `recent:` is 3.
pub(super) fn entry_no(id: &str, prefix: &str) -> Option<usize> {
    id.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok())
}

/// An id of this menu: a custom item, an Open Recent entry, the Clear Recent item or a Window menu
/// project entry.
#[must_use]
pub fn is_known(id: &str) -> bool {
    SPEC.iter().any(|i| i.id == id)
        || id == RECENT_CLEAR_ID
        || entry_no(id, RECENT_PREFIX).is_some()
        || entry_no(id, PROJECT_PREFIX).is_some()
        || entry_no(id, FOLLOW_PREFIX).is_some()
        || entry_no(id, ZOOM_OWN_PREFIX).is_some()
}

/// A click on the item `id`: the page gets the event `menu` with `{id, path?, sid?}`. An Open Recent entry
/// (`recent:<n>`) carries the folder of the project, a Window entry (`project:<n>`) the project id. Returns the
/// payload that was sent.
pub fn dispatch<R: Runtime>(app: &AppHandle<R>, id: &str) -> Value {
    // #206 diagnosis: which menu ids reach the Rust side (selftest_menu_log). The file copy
    // (DRUMSPIKE_MENU_LOG_FILE) lets the real-event probe on an Apple-silicon Mac read the ids that a
    // physical press via osascript dispatched (the probe cannot call the tauri command).
    // a poisoned lock is unwrapped: the log entries are still pushed (the poison of another
    // thread's panic must not take the menu dispatch down, #206 diagnosis)
    MENU_LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(id.to_string());
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("DRUMSPIKE_MENU_LOG_FILE") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "{id}");
        }
    }
    if !is_known(id) {
        return Value::Null;
    }
    let state = app.try_state::<MenuState>();
    let payload = if id == RECENT_CLEAR_ID {
        json!({"id": RECENT_CLEAR_PAYLOAD})
    } else if let Some(n) = entry_no(id, RECENT_PREFIX) {
        let path = state.as_ref().and_then(|s| lock(&s.recent).get(n).cloned());
        json!({"id": RECENT_ID, "path": path})
    } else if let Some(n) = entry_no(id, PROJECT_PREFIX) {
        let sid = state
            .as_ref()
            .and_then(|s| lock(&s.projects).get(n).map(|e| e.sid.clone()));
        json!({"id": PROJECT_ID, "sid": sid})
    } else if let Some(n) = entry_no(id, FOLLOW_PREFIX) {
        // P3-18: a Follow Hits Of entry carries the track name
        let name = state
            .as_ref()
            .and_then(|s| lock(&s.follow).get(n).map(|e| e.name.clone()));
        json!({"id": FOLLOW_TRACK_ID, "name": name})
    } else if let Some(n) = entry_no(id, ZOOM_OWN_PREFIX) {
        // #169: an own zoom preset carries its index in the page's list
        json!({"id": ZOOM_OWN_ID, "n": n})
    } else {
        json!({"id": id})
    };
    let _ = app.emit("menu", payload.clone());
    payload
}
