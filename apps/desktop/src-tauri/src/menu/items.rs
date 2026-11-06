//! The custom items of the menu bar: the ids of the dynamic entries, `Kind`, `Item`, one constant per item,
//! `SPEC` (every custom item) and the radio modes of the page.

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
/// #343: View > Training Mode (the correction log of the owner's edits; a page setting, off by default).
pub const TRAINING_MODE: Item = check("training_mode", "Training Mode");
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
/// #354: the selected hit logged as reviewed, right where it is (Training Mode; the document is unchanged).
/// Greyed while Training Mode is off (`sync_checks` reads `trainingMode`) and without a hit selection.
pub const CONFIRM_HIT: Item = item("confirm_hit", "Confirm Hit", true);
pub const SNAP: Item = check("snap", "Snap Playhead to Hits");
pub const ANALYZE_TRACK: Item = item("analyze_track", "Analyze Selected Track", true);
/// #265: Re-analyze Selection, the hits of the range only (greyed without a range: `sync_checks`
/// reads `selRange`).
pub const REANALYZE_SELECTION: Item = item("reanalyze_selection", "Re-analyze Selection", true);
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
pub const SPEC: [Item; 115] = [
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
    TRAINING_MODE,
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
    CONFIRM_HIT,
    SNAP,
    ANALYZE_TRACK,
    REANALYZE_SELECTION,
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
