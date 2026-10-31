//! The native menu bar (P3-4b, P3-13a, P3-13b). One table (`SPEC`) holds every custom item of the app: id, label,
//! kind (plain or check item) and whether the item needs an open project. `build` makes the menu bar of
//! `docs/migration/ux-daw-reference.md` section 3.2 from the table, `dispatch` turns a click into the event `menu`
//! for the page (`static/shell.js` does the work), and `sync`, `sync_checks` and `sync_window` grey the items that
//! need an open project, refresh Open Recent and the Window menu, and set the check items to the page's state.
//!
//! P3-13b: the keys are not in this file. `static/keymap.js` is the key command table of the app (one row per
//! command: menu id, menu accelerator, page keys, Cubase keys); its rows between the markers `KEYMAP-BEGIN` and
//! `KEYMAP-END` are strict JSON. This file includes the table at compile time (`include_str!`), parses it once
//! ([`keymap`]) and gives each item the accelerator of its row ([`accel_of`], converted to the Tauri notation).
//! The page reads the same file (`DSKeys`), so the menu, the page keys and the help panel cannot differ; the tests
//! below fail when a key is in two commands or an accelerator of the menu differs from the table.
//! A shortcut (accelerator) is set only for keys with Cmd, Control or Option: the keys without one (Space, Tab,
//! Return, Delete, letters) stay page keys, so a text field keeps them (`static/app.js` keydown).
//! An item whose command does not exist yet would stay in the menu, greyed out (`ready: false`); none is.

use std::sync::{LazyLock, Mutex};

use serde_json::{Value, json};
use tauri::menu::{
    CheckMenuItem, CheckMenuItemBuilder, Menu, MenuBuilder, MenuItem, MenuItemBuilder, MenuItemKind,
    PredefinedMenuItem, Submenu, SubmenuBuilder,
};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// The id of the Open Recent submenu.
pub const RECENT_ID: &str = "recent";
/// The id prefix of the Open Recent entries: `recent:0` to `recent:9`.
pub const RECENT_PREFIX: &str = "recent:";
/// The id of the Clear Recent item at the end of the Open Recent submenu (#441).
pub const RECENT_CLEAR_ID: &str = "recent:clear";
/// The payload id of the Clear Recent item: the page gets `{id: "recent_clear"}`.
pub const RECENT_CLEAR_PAYLOAD: &str = "recent_clear";
/// How many recent projects the menu shows.
pub const RECENT_MAX: usize = 10;
/// The id of the Window menu.
pub const WINDOW_ID: &str = "window";
/// The id prefix of the Window menu's project entries: `project:0`, `project:1`, ...
pub const PROJECT_PREFIX: &str = "project:";
/// The payload id of a Window menu project entry: the page gets `{id: "project", sid}`.
pub const PROJECT_ID: &str = "project";
/// P3-18: the id of the submenu Track > Follow Hits Of.
pub const FOLLOW_ID: &str = "follow_of";
/// P3-18: the id prefix of the Follow Hits Of entries: `follow:0`, `follow:1`, ... (the other tracks of the project).
pub const FOLLOW_PREFIX: &str = "follow:";
/// P3-18: the payload id of a Follow Hits Of entry: the page gets `{id: "follow", name}`.
pub const FOLLOW_TRACK_ID: &str = "follow";
/// #169: the id of the submenu View > Zoom Presets (its own presets are appended to it, see `sync_zoom_own`).
pub const ZOOM_PRESETS_ID: &str = "zoom_presets";
/// #169: the id prefix of the own zoom presets: `zoom_own:0`, `zoom_own:1`, ... (the engineer's spans, app setting `zoom_presets`).
/// The first five carry Option+1 to Option+5.
pub const ZOOM_OWN_PREFIX: &str = "zoom_own:";
/// #169: the payload id of an own zoom preset: the page gets `{id: "zoom_own", n}` (n: the index in its list).
pub const ZOOM_OWN_ID: &str = "zoom_own";

/// A plain item or a check item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A command.
    Plain,
    /// A command with a check mark that shows a state of the page (snap, follow mode, click, Inspector).
    Check,
}

/// One custom menu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    /// The id that the page gets in the event `menu`.
    pub id: &'static str,
    /// The menu text (the `title` of its row in `static/keymap.js`; a test checks that both agree).
    pub label: &'static str,
    /// The command behind the item exists. `false`: the item is greyed out.
    pub ready: bool,
    /// The item needs an open project. `sync` greys it while none is open.
    pub needs_project: bool,
    /// The card that makes the item work, for the greyed-out ones.
    pub card: Option<&'static str>,
    /// Plain or check item.
    pub kind: Kind,
}

const fn item(id: &'static str, label: &'static str, needs_project: bool) -> Item {
    Item {
        id,
        label,
        ready: true,
        needs_project,
        card: None,
        kind: Kind::Plain,
    }
}

const fn check(id: &'static str, label: &'static str) -> Item {
    let mut i = item(id, label, false);
    i.kind = Kind::Check;
    i
}

const fn check_project(id: &'static str, label: &'static str) -> Item {
    let mut i = check(id, label);
    i.needs_project = true;
    i
}

// Drum Spike
pub const ABOUT: Item = item("about", "About Drum Spike", false);
pub const SETTINGS: Item = item("settings", "Settings\u{2026}", false);
pub const QUIT: Item = item("quit", "Quit Drum Spike", false);
// File
pub const NEW: Item = item("new", "New Project\u{2026}", false);
pub const OPEN: Item = item("open", "Open\u{2026}", false);
pub const OPEN_RECENT: Item = item("open_recent", "Open Most Recent", false); // P3-19: greyed while Open Recent is empty
pub const CLOSE: Item = item("close", "Close Project", true);
pub const SAVE: Item = item("save", "Save", true);
pub const SAVE_AS: Item = item("save_as", "Save As\u{2026}", true);
pub const RENAME: Item = item("rename", "Rename Project\u{2026}", true);
pub const ADD_TRACKS: Item = item("add_tracks", "Add Tracks\u{2026}", true);
pub const IMPORT_OLD: Item = item("import_old", "Import Old Session\u{2026}", false);
// P4-8: File > Import Tempo Map, Track > Detect Tempo, View > the three map rulers and the grid
pub const IMPORT_TEMPO_MAP: Item = item("import_tempo_map", "Import Tempo Map\u{2026}", true);
pub const CONSOLIDATE: Item = item("consolidate", "Consolidate Audio", true);
/// P4-30: the one Export dialog (all tracks); Track > Export Selected Tracks opens it with the selected track.
pub const EXPORT: Item = item("export", "Export\u{2026}", true);
/// P4-31: the last export again (greyed before the project keeps an export shape: `sync_checks` reads `exportAgain`).
pub const EXPORT_AGAIN: Item = item("export_again", "Export Again", true);
// Edit
pub const UNDO: Item = item("undo", "Undo", true);
pub const REDO: Item = item("redo", "Redo", true);
/// P4-40a: the page lists the steps of the undo stack (page only; the service has no part in it).
pub const UNDO_HISTORY: Item = item("undo_history", "Undo History\u{2026}", true);
pub const DELETE_HIT: Item = item("delete_hit", "Delete Hit", true);
pub const ADD_HIT: Item = item("add_hit", "Add Hit at Playhead", true);
/// P4-40g: a review marker at the selected hit or the playhead (the M key stays the page's: no modifier).
pub const MARKERS_SET: Item = item("markers_set", "Set Marker at Selection", true);
pub const NUDGE_EARLIER: Item = item("nudge_earlier", "Nudge Hit Earlier", true);
pub const NUDGE_LATER: Item = item("nudge_later", "Nudge Hit Later", true);
// P3-20a: Edit > Nudge Value (radio items; the value is a view setting, so no project is needed)
pub const NUDGE_VAL_1SMP: Item = check("nudge_val_1smp", "1 Sample");
pub const NUDGE_VAL_10SMP: Item = check("nudge_val_10smp", "10 Samples");
pub const NUDGE_VAL_100SMP: Item = check("nudge_val_100smp", "100 Samples");
pub const NUDGE_VAL_1MS: Item = check("nudge_val_1ms", "1 ms");
pub const NUDGE_VAL_10MS: Item = check("nudge_val_10ms", "10 ms");
pub const RECOMPUTE: Item = item("recompute_velocities", "Recompute Velocities", true);
// View
pub const ZOOM_IN: Item = item("zoom_in", "Zoom In", true);
pub const ZOOM_OUT: Item = item("zoom_out", "Zoom Out", true);
pub const ZOOM_PROJECT: Item = item("zoom_project", "Zoom to Project", true);
/// #158 (rank 8): the page's zoom toggle (E is a page key: no accelerator without Cmd, Control or Option).
pub const ZOOM_TOGGLE: Item = item("zoom_toggle", "Zoom Toggle", true);
// P3-20a: View > Waveform Zoom (a display gain of the wave lanes; a view setting, so no project is needed)
pub const AMP_IN: Item = item("amp_in", "In", false);
pub const AMP_OUT: Item = item("amp_out", "Out", false);
pub const AMP_RESET: Item = item("amp_reset", "Reset", false);
pub const ZOOM_FULL: Item = item("zoom_full", "Full", true);
pub const ZOOM_30S: Item = item("zoom_30s", "30 s", true);
pub const ZOOM_5S: Item = item("zoom_5s", "5 s", true);
pub const ZOOM_500MS: Item = item("zoom_500ms", "500 ms", true);
pub const ZOOM_50MS: Item = item("zoom_50ms", "50 ms", true);
pub const ZOOM_5MS: Item = item("zoom_5ms", "5 ms", true);
/// #169: the span of the view under a name, the new preset in the zoom dropdown and in this submenu; Organize renames and removes.
pub const ZOOM_ADD: Item = item("zoom_add", "Add Current\u{2026}", true);
pub const ZOOM_ORGANIZE: Item = item("zoom_organize", "Organize\u{2026}", true);
// P3-14c: View > Track Height (the heights apply to every track; an app setting, so no project is needed)
pub const HEIGHT_TALLER: Item = item("height_taller", "Taller", false);
pub const HEIGHT_SHORTER: Item = item("height_shorter", "Shorter", false);
pub const HEIGHT_SMALL: Item = check("height_small", "Small");
pub const HEIGHT_MEDIUM: Item = check("height_medium", "Medium");
pub const HEIGHT_LARGE: Item = check("height_large", "Large");
pub const INSPECTOR: Item = check("inspector", "Show Inspector");
pub const INFO_LINE: Item = check("info_line", "Show Info Line");
/// #169: View > Show Overview (the overview strip over the lanes; hidden, the lanes take its height).
pub const OVERVIEW: Item = check("overview", "Show Overview");
pub const COUNTER_SECONDS: Item = check("counter_seconds", "Seconds");
pub const COUNTER_SAMPLES: Item = check("counter_samples", "Samples");
/// #149: Bars|Beats (bar|beat|ticks, 960 ticks per beat) and Timecode (a fixed 30 fps, not the session rate).
pub const COUNTER_BARSBEATS: Item = check("counter_barsbeats", "Bars|Beats");
pub const COUNTER_TIMECODE: Item = check("counter_timecode", "Timecode (30 fps)");
pub const COUNTER_EXCHANGE: Item = item("counter_exchange", "Exchange Counters", false);
/// #175: View > Tab Moves To (radio items; which hits the Tab keys step to, a user setting of the page).
pub const TAB_ACTIVE: Item = check("tab_active", "Active Hits Only");
pub const TAB_ALL: Item = check("tab_all", "All Found Hits");
pub const RULER_TEMPO: Item = check("ruler_tempo", "Tempo Ruler");
pub const RULER_METER: Item = check("ruler_meter", "Meter Ruler");
pub const RULER_BEATS: Item = check("ruler_beats", "Bars|Beats Ruler");
/// P4-40g: the review markers ruler.
pub const RULER_MARKERS: Item = check("ruler_markers", "Markers Ruler");
pub const GRID: Item = check("grid", "Show Bar and Beat Grid");
/// P4-40f: View > Velocity Lane (the thin lane with a stalk per hit under the spike lane).
pub const VELOCITY_LANE: Item = check("velocity_lane", "Velocity Lane");
/// #157: View > Other Mics Under Hit (the panel of the other tracks in the window of the selected hit).
pub const OTHER_MICS: Item = check("other_mics", "Other Mics Under Hit");
// Transport
pub const PLAY_STOP: Item = item("play_stop", "Play/Stop", true);
/// P3-20b: Shift+Space (a page key: no accelerator without Cmd, Control or Option).
pub const HALF_SPEED: Item = item("half_speed", "Half-Speed Play", true);
pub const STOP_RETURN: Item = item("stop_return", "Stop and Return", true);
pub const GO_START: Item = item("go_start", "Go to Project Start", true);
pub const GO_END: Item = item("go_end", "Go to Project End", true);
pub const GO_PLAY_START: Item = item("go_play_start", "Go to Playback Start", true);
/// #158 (rank 8): the review markers of P4-40g (Option+M and Option+Shift+M are the accelerators now that the items exist).
pub const MARKER_NEXT: Item = item("marker_next", "Next Marker", true);
pub const MARKER_PREV: Item = item("marker_prev", "Previous Marker", true);
pub const FOLLOW_SMOOTH: Item = check("follow_smooth", "Smooth");
pub const FOLLOW_PAGE: Item = check("follow_page", "Page");
pub const FOLLOW_OFF: Item = check("follow_off", "Off");
pub const FOLLOW_TOGGLE: Item = item("follow_toggle", "Follow Playhead On/Off", false);
/// P4-wf10: an edit during playback suspends the follow for 2 s of playback (a user setting: the page reports it).
pub const SUSPEND_FOLLOW: Item = check("suspend_follow", "Suspend Follow when Editing");
/// #436: while checked, a Stop puts the playhead back where the play started (a user setting: the page reports it).
pub const STOP_RETURN_MODE: Item = check("stop_return_mode", "Stop Returns to Start");
/// #240: a Tab step to a hit plays it (a user setting: the page reports it).
pub const AUDITION_TAB: Item = check("audition_tab", "Audition on Tab");
pub const CLICK: Item = check("click", "Metronome Click");
// Hits
pub const NEXT_HIT: Item = item("next_hit", "Next Hit", true);
pub const PREV_HIT: Item = item("prev_hit", "Previous Hit", true);
pub const NEXT_HIT_ALL: Item = item("next_hit_all", "Next Hit on All Tracks", true);
pub const PREV_HIT_ALL: Item = item("prev_hit_all", "Previous Hit on All Tracks", true);
// #150: the review queue
pub const REVIEW_NEXT: Item = item("review_next", "Next Doubtful Hit", true);
pub const REVIEW_PREV: Item = item("review_prev", "Previous Doubtful Hit", true);
/// #158 (rank 8): P4-40d's audition (A and Shift+A are page keys; the Cubase preset owns Option+A).
pub const AUDITION: Item = item("audition", "Audition Hit", true);
/// #242: the selected hit between active and deactivated (X is the page key in both presets).
pub const TOGGLE_HIT_ACTIVE: Item = item("toggle_hit_active", "Toggle Hit Active", true);
pub const SNAP: Item = check("snap", "Snap Playhead to Hits");
pub const ANALYZE_TRACK: Item = item("analyze_track", "Analyze Selected Track", true);
pub const ANALYZE_ALL: Item = item("analyze_all", "Analyze All", true);
// P4-6a: Learn marks (P4-6c: the filter is a check item; #293: the Learn items work on the DSP candidates)
pub const LEARN_THIS: Item = item("learn_this", "Learn This", true);
pub const LEARN_NOT: Item = item("learn_not", "Forget This", true);
pub const LEARN_FORGET_THIS: Item = item("learn_forget_this", "Remove Example", true); // P4-36
pub const LEARN_FILTER: Item = check_project("learn_filter", "Learn Filter"); // P4-6c: checked = the selected track's filter is on
pub const LEARN_FORGET: Item = item("learn_forget", "Forget Examples of Track", true);
// Track (Add Tracks is the File item, shown here as well)
/// #158 (rank 8): the P and ; of the page step through the tracks (no accelerator: keys without a modifier).
pub const PREV_TRACK: Item = item("prev_track", "Select Previous Track", true);
pub const NEXT_TRACK: Item = item("next_track", "Select Next Track", true);
pub const MUTE_TRACK: Item = item("mute_track", "Mute Selected Track", true);
pub const SOLO_TRACK: Item = item("solo_track", "Solo Selected Track", true);
pub const RESET_MIX: Item = item("reset_mix", "Reset Volume and Pan", true);
/// P3-18: the selected track detects its hits itself (Follow Hits Of is the dynamic submenu before it).
pub const FOLLOW_OWN: Item = item("follow_own", "Own Detection", true);
pub const DETECT_TEMPO: Item = item("detect_tempo", "Detect Tempo\u{2026}", true);
/// P4-39d: bar 1 of the tempo map at the playhead (or the selected hit); the tempo stays.
pub const SET_BAR1: Item = item("set_bar1", "Set Bar 1 at Playhead", true);
/// #468: bar 1 at 0 s, the DAW convention (the session start is bar 1); no keyboard shortcut.
pub const SET_BAR1_AUDIO: Item = item("set_bar1_audio", "Set Bar 1 at Audio Start", true);
pub const EXPORT_SELECTED: Item = item("export_selected", "Export Selected Tracks\u{2026}", true);
/// P4-39a: removes the selected track (greyed without a selected track: `sync_checks` reads `selected`).
pub const TRACKS_REMOVE: Item = item("tracks_remove", "Remove Track", true);
// Help
/// #255: the manual (static/help, prebuilt from docs/guide) with its Shortcuts section.
pub const DRUMSPIKE_HELP: Item = item("drumspike_help", "Drum Spike Help", false);
pub const SHORTCUTS: Item = item("shortcuts", "Keyboard Shortcuts", false);

/// Every custom item (not the Open Recent and Window entries, not the system items Cut, Copy and so on).
pub const SPEC: [Item; 112] = [
    ABOUT,
    SETTINGS,
    QUIT,
    NEW,
    OPEN,
    OPEN_RECENT,
    CLOSE,
    SAVE,
    SAVE_AS,
    RENAME,
    ADD_TRACKS,
    IMPORT_OLD,
    IMPORT_TEMPO_MAP,
    CONSOLIDATE,
    EXPORT,
    EXPORT_AGAIN,
    UNDO,
    REDO,
    UNDO_HISTORY,
    DELETE_HIT,
    ADD_HIT,
    MARKERS_SET,
    NUDGE_EARLIER,
    NUDGE_LATER,
    NUDGE_VAL_1SMP,
    NUDGE_VAL_10SMP,
    NUDGE_VAL_100SMP,
    NUDGE_VAL_1MS,
    NUDGE_VAL_10MS,
    RECOMPUTE,
    ZOOM_IN,
    ZOOM_OUT,
    ZOOM_PROJECT,
    ZOOM_TOGGLE,
    AMP_IN,
    AMP_OUT,
    AMP_RESET,
    ZOOM_FULL,
    ZOOM_30S,
    ZOOM_5S,
    ZOOM_500MS,
    ZOOM_50MS,
    ZOOM_5MS,
    ZOOM_ADD,
    ZOOM_ORGANIZE,
    HEIGHT_TALLER,
    HEIGHT_SHORTER,
    HEIGHT_SMALL,
    HEIGHT_MEDIUM,
    HEIGHT_LARGE,
    INSPECTOR,
    INFO_LINE,
    OVERVIEW,
    COUNTER_SECONDS,
    COUNTER_SAMPLES,
    COUNTER_BARSBEATS,
    COUNTER_TIMECODE,
    COUNTER_EXCHANGE,
    TAB_ACTIVE,
    TAB_ALL,
    RULER_TEMPO,
    RULER_METER,
    RULER_BEATS,
    RULER_MARKERS,
    GRID,
    VELOCITY_LANE,
    OTHER_MICS,
    PLAY_STOP,
    HALF_SPEED,
    STOP_RETURN,
    STOP_RETURN_MODE,
    AUDITION_TAB,
    GO_START,
    GO_END,
    GO_PLAY_START,
    MARKER_NEXT,
    MARKER_PREV,
    FOLLOW_SMOOTH,
    FOLLOW_PAGE,
    FOLLOW_OFF,
    FOLLOW_TOGGLE,
    SUSPEND_FOLLOW,
    CLICK,
    NEXT_HIT,
    PREV_HIT,
    NEXT_HIT_ALL,
    PREV_HIT_ALL,
    REVIEW_NEXT,
    REVIEW_PREV,
    AUDITION,
    TOGGLE_HIT_ACTIVE,
    SNAP,
    ANALYZE_TRACK,
    ANALYZE_ALL,
    LEARN_THIS,
    LEARN_NOT,
    LEARN_FORGET_THIS,
    LEARN_FILTER,
    LEARN_FORGET,
    PREV_TRACK,
    NEXT_TRACK,
    MUTE_TRACK,
    SOLO_TRACK,
    RESET_MIX,
    FOLLOW_OWN,
    DETECT_TEMPO,
    SET_BAR1,
    SET_BAR1_AUDIO,
    EXPORT_SELECTED,
    TRACKS_REMOVE,
    DRUMSPIKE_HELP,
    SHORTCUTS,
];

/// The follow modes of the page and their radio items (check items, exactly one of them on).
pub const FOLLOW_MODES: [(&str, &str); 3] = [
    ("smooth", "follow_smooth"),
    ("page", "follow_page"),
    ("off", "follow_off"),
];

/// The main counter modes of the page and their radio items (check items, exactly one of them on).
pub const COUNTER_MODES: [(&str, &str); 4] = [
    ("seconds", "counter_seconds"),
    ("samples", "counter_samples"),
    ("barsbeats", "counter_barsbeats"),
    ("timecode", "counter_timecode"),
];

/// The "Tab moves to" modes of the page (#175) and their radio items (check items, exactly one of them on).
pub const TAB_MODES: [(&str, &str); 2] = [("active", "tab_active"), ("all", "tab_all")];

/// The nudge values of the page (P3-20a) and their radio items (check items, exactly one of them on).
pub const NUDGE_MODES: [(&str, &str); 5] = [
    ("1smp", "nudge_val_1smp"),
    ("10smp", "nudge_val_10smp"),
    ("100smp", "nudge_val_100smp"),
    ("1ms", "nudge_val_1ms"),
    ("10ms", "nudge_val_10ms"),
];

/// The track heights of the page (P3-14c) and their radio items (check items, exactly one of them on).
pub const HEIGHT_MODES: [(&str, &str); 3] = [
    ("small", "height_small"),
    ("medium", "height_medium"),
    ("large", "height_large"),
];

/// `static/keymap.js`, compiled in (P3-13b): the key command table of the app. The page loads the same file.
pub const KEYMAP_JS: &str = include_str!("../../../../static/keymap.js");
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
/// no `id`, no `title` or no `cubase` key, or a field of the wrong type. The message names the row.
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

/// The key presets of the app (P3-13c): the setting `key_preset` of `<home>/settings.json` holds one of them.
pub const PRESETS: [&str; 2] = ["default", "cubase"];

/// `name` when it is a preset, else the default one (a damaged setting never leaves the menu without keys).
#[must_use]
pub fn preset_or_default(name: &str) -> &'static str {
    PRESETS.iter().copied().find(|p| *p == name).unwrap_or("default")
}

/// The preset that the menu shows now (the last [`set_preset`], the default at start).
static PRESET: Mutex<&'static str> = Mutex::new("default");

/// The preset of the accelerators that the live menu has now.
#[must_use]
pub fn current_preset() -> &'static str {
    *lock(&PRESET)
}

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

/// #206 diagnosis: every menu id that reached `dispatch`, in order.
pub static MENU_LOG: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

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
        ("suspendFollow", &SUSPEND_FOLLOW), // P4-wf10: Transport > Suspend Follow when Editing
        ("stopReturnMode", &STOP_RETURN_MODE), // #436: Transport > Stop Returns to Start
        ("auditionTab", &AUDITION_TAB),     // #240: Transport > Audition on Tab
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

fn no_tracks<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<MenuItem<R>> {
    MenuItemBuilder::with_id(format!("{FOLLOW_PREFIX}none"), "No Other Track")
        .enabled(false)
        .build(app)
}

fn make_item<R: Runtime>(app: &AppHandle<R>, it: &Item) -> tauri::Result<MenuItem<R>> {
    let mut b = MenuItemBuilder::with_id(it.id, it.label).enabled(it.ready && !it.needs_project);
    if let Some(a) = accel_of(it.id, current_preset()) {
        b = b.accelerator(a);
    }
    b.build(app)
}

fn make_check<R: Runtime>(app: &AppHandle<R>, it: &Item, on: bool) -> tauri::Result<CheckMenuItem<R>> {
    let mut b = CheckMenuItemBuilder::with_id(it.id, it.label)
        .checked(on)
        .enabled(it.ready && !it.needs_project);
    if let Some(a) = accel_of(it.id, current_preset()) {
        b = b.accelerator(a);
    }
    b.build(app)
}

fn no_projects<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<MenuItem<R>> {
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
fn entry_no(id: &str, prefix: &str) -> Option<usize> {
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
    // (DRUMSPIKE_MENU_LOG_FILE) lets the real-event probe on arm64-mac read the ids that a
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
    let Some(menu) = app.menu() else { return Ok(()) };
    for it in SPEC {
        let accel = accel_of(it.id, preset);
        match find_item(&menu, it.id) {
            Some(MenuItemKind::MenuItem(m)) => m.set_accelerator(accel)?,
            Some(MenuItemKind::Check(c)) => c.set_accelerator(accel)?,
            _ => {}
        }
    }
    *lock(&PRESET) = preset;
    #[cfg(target_os = "macos")]
    fix_shift_equivalents(app);
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
            "id": it.id, "label": it.label, "accel": accel_of(it.id, current_preset()), "ready": it.ready, "card": it.card,
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

#[cfg(test)]
mod tests;
