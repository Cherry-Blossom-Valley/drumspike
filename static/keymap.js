/* Key command table (P3-13b, docs/migration/ux-daw-reference.md 3.3). Loaded before shell.js and app.js.
   Every command of the app is here once: the native menu (apps/desktop/src-tauri/src/menu.rs) takes its
   accelerators from this table, the page (app.js keydown) takes its keys from it, and the help panel
   (Help > Keyboard Shortcuts) is made from it.

   The table between the markers KEYMAP-BEGIN and KEYMAP-END is strict JSON (double quotes, no comments, no
   trailing comma): menu.rs reads this file at compile time (include_str!) and parses the text between the
   markers. One row per command:
     id         the command id; for a menu item it is the item id of menu.rs SPEC and of shell.js handlers
     menu       the menu item id, or null for a key that has no menu item (handled by the page only)
     path       the menu of the item ("View > Zoom Presets" for a submenu), or null
     title      the menu text (menu.rs checks that its label is the same)
     menu_key   the native menu accelerator: one key with Cmd, Control or Option, or null
     page_keys  the keys that the page handles (app.js); a text field keeps them
     cubase     the keys of the "Cubase" preset (P3-13c), or null when the preset keeps the default keys. A list
                REPLACES the menu key and the page keys of the row (nothing is added to the default keys).
                An empty list [] means no key in the preset (P3-19: the keys that only the Pro Tools preset has).
                The menu accelerator of the row in this preset is the FIRST key of the list that has Cmd,
                Control or Option; the other keys are page keys (menuKeyOf here, accel_of in menu.rs).
     note       a short line for the help panel, or null
   Key notation: modifiers in the macOS order Control, Option, Shift, Cmd, then one key, joined with "+":
   "Shift+Cmd+S", "Control+Option+Tab". Keys: A to Z, 0 to 9, the characters [ ] , . = _ / ; ' ` \, and the
   names Plus, Minus, Space, Tab, Return, Delete, Backspace, Escape, Left, Right, Up, Down, NumpadMinus,
   NumpadPlus, NumpadPeriod, NumpadEnter, Numpad0 to Numpad9. A character key has no Shift (the character
   already says it: "+" is Plus, with or without Shift). */
(function () {
  "use strict";
  const COMMANDS = /*KEYMAP-BEGIN*/[
    {"id": "about", "menu": "about", "path": "Drum Spike", "title": "About Drum Spike", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "settings", "menu": "settings", "path": "Drum Spike", "title": "Settings…", "menu_key": "Cmd+,", "page_keys": [], "cubase": null, "note": "metronome click, volume, click pitches"},
    {"id": "quit", "menu": "quit", "path": "Drum Spike", "title": "Quit Drum Spike", "menu_key": "Cmd+Q", "page_keys": [], "cubase": null, "note": null},
    {"id": "new", "menu": "new", "path": "File", "title": "New Project…", "menu_key": "Cmd+N", "page_keys": [], "cubase": null, "note": null},
    {"id": "open", "menu": "open", "path": "File", "title": "Open…", "menu_key": "Cmd+O", "page_keys": [], "cubase": null, "note": null},
    {"id": "open_recent", "menu": "open_recent", "path": "File", "title": "Open Most Recent", "menu_key": "Shift+Cmd+O", "page_keys": [], "cubase": [], "note": "the first entry of Open Recent; greyed when the list is empty"},
    {"id": "close", "menu": "close", "path": "File", "title": "Close Project", "menu_key": "Shift+Cmd+W", "page_keys": ["Cmd+W"], "cubase": ["Cmd+W"], "note": "Cmd+W is a page key in the default preset"},
    {"id": "save", "menu": "save", "path": "File", "title": "Save", "menu_key": "Cmd+S", "page_keys": [], "cubase": null, "note": "edits are autosaved to Backups/ while unsaved; ● = unsaved"},
    {"id": "save_as", "menu": "save_as", "path": "File", "title": "Save As…", "menu_key": "Shift+Cmd+S", "page_keys": [], "cubase": null, "note": null},
    {"id": "rename", "menu": "rename", "path": "File", "title": "Rename Project…", "menu_key": null, "page_keys": [], "cubase": null, "note": "or double-click the project name"},
    {"id": "add_tracks", "menu": "add_tracks", "path": "File", "title": "Add Tracks…", "menu_key": "Shift+Cmd+I", "page_keys": [], "cubase": null, "note": "also in the Track menu"},
    {"id": "import_old", "menu": "import_old", "path": "File", "title": "Import Old Session…", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "import_tempo_map", "menu": "import_tempo_map", "path": "File", "title": "Import Tempo Map…", "menu_key": null, "page_keys": [], "cubase": null, "note": "replaces the project's tempo and meter map with the tempo and time-signature events of a MIDI file from the DAW; one undo step"},
    {"id": "consolidate", "menu": "consolidate", "path": "File", "title": "Consolidate Audio", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "export", "menu": "export", "path": "File", "title": "Export…", "menu_key": "Cmd+E", "page_keys": [], "cubase": null, "note": "the Export dialog: MIDI and spike WAV, all tracks or the selected track, the destination folder"},
    {"id": "export_again", "menu": "export_again", "path": "File", "title": "Export Again", "menu_key": "Option+Cmd+E", "page_keys": [], "cubase": [], "note": "P4-31: writes the files of the last export again: the same formats, tracks and folder, no dialog; greyed before the first export (a key in the default preset only)"},
    {"id": "undo", "menu": "undo", "path": "Edit", "title": "Undo", "menu_key": "Cmd+Z", "page_keys": ["Z"], "cubase": ["Cmd+Z"], "note": null},
    {"id": "redo", "menu": "redo", "path": "Edit", "title": "Redo", "menu_key": "Shift+Cmd+Z", "page_keys": ["Shift+Z", "Cmd+Y"], "cubase": ["Shift+Cmd+Z", "Cmd+Y"], "note": null},
    {"id": "undo_history", "menu": "undo_history", "path": "Edit", "title": "Undo History…", "menu_key": "Option+Cmd+Z", "page_keys": [], "cubase": null, "note": "lists the last steps, newest first (the redo steps above, grey); a click undoes or redoes back to that step"},
    {"id": "delete_hit", "menu": "delete_hit", "path": "Edit", "title": "Delete Hit", "menu_key": null, "page_keys": ["Delete", "Backspace"], "cubase": null, "note": "or right-click the hit"},
    {"id": "add_hit", "menu": "add_hit", "path": "Edit", "title": "Add Hit at Playhead", "menu_key": null, "page_keys": ["H"], "cubase": [], "note": "on the selected track; double-click or Option-click a lane adds a hit there (H is a Pro Tools preset key only: the Cubase preset zooms in with H)"},
    {"id": "markers_set", "menu": "markers_set", "path": "Edit", "title": "Set Marker at Selection", "menu_key": null, "page_keys": ["M"], "cubase": [], "note": "P4-40g: sets a marker at the selected hit or the playhead; a double-click on the marker in the Markers ruler takes its optional note (M is the Cubase mute key, so the preset has none)"},
    {"id": "nudge_earlier", "menu": "nudge_earlier", "path": "Edit", "title": "Nudge Hit Earlier", "menu_key": null, "page_keys": ["Left", "NumpadMinus", ","], "cubase": ["Cmd+Left"], "note": "by the nudge value (Edit > Nudge Value); no hit selected: the key pans the view"},
    {"id": "nudge_later", "menu": "nudge_later", "path": "Edit", "title": "Nudge Hit Later", "menu_key": null, "page_keys": ["Right", "NumpadPlus", "."], "cubase": ["Cmd+Right"], "note": "by the nudge value (Edit > Nudge Value); no hit selected: the key pans the view"},
    {"id": "nudge_val_1smp", "menu": "nudge_val_1smp", "path": "Edit > Nudge Value", "title": "1 Sample", "menu_key": null, "page_keys": [], "cubase": null, "note": "a click on the Nudge value next to the main counter opens the same list; Option+Cmd+numpad + and - step through it"},
    {"id": "nudge_val_10smp", "menu": "nudge_val_10smp", "path": "Edit > Nudge Value", "title": "10 Samples", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "nudge_val_100smp", "menu": "nudge_val_100smp", "path": "Edit > Nudge Value", "title": "100 Samples", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "nudge_val_1ms", "menu": "nudge_val_1ms", "path": "Edit > Nudge Value", "title": "1 ms", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "nudge_val_10ms", "menu": "nudge_val_10ms", "path": "Edit > Nudge Value", "title": "10 ms", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "recompute_velocities", "menu": "recompute_velocities", "path": "Edit", "title": "Recompute Velocities", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "zoom_in", "menu": "zoom_in", "path": "View", "title": "Zoom In", "menu_key": "Cmd+]", "page_keys": ["T", "Plus", "="], "cubase": ["H", "Cmd+]"], "note": "around the playhead"},
    {"id": "zoom_out", "menu": "zoom_out", "path": "View", "title": "Zoom Out", "menu_key": "Cmd+[", "page_keys": ["R", "Minus", "_"], "cubase": ["G", "Cmd+["], "note": null},
    {"id": "zoom_project", "menu": "zoom_project", "path": "View", "title": "Zoom to Project", "menu_key": "Control+Cmd+[", "page_keys": ["Option+A"], "cubase": ["Shift+F"], "note": null},
    {"id": "zoom_toggle", "menu": "zoom_toggle", "path": "View", "title": "Zoom Toggle", "menu_key": null, "page_keys": ["E"], "cubase": [], "note": "the selected range fills the lane width, else 5 ms around the selected hit (or the playhead); again: back to the zoom, position and track heights before"},
    {"id": "amp_in", "menu": "amp_in", "path": "View > Waveform Zoom", "title": "In", "menu_key": "Option+Cmd+]", "page_keys": [], "cubase": [], "note": "doubles the displayed amplitude of all wave lanes, up to 64x; display only, the hits and the exports do not change"},
    {"id": "amp_out", "menu": "amp_out", "path": "View > Waveform Zoom", "title": "Out", "menu_key": "Option+Cmd+[", "page_keys": [], "cubase": [], "note": "halves it, down to 1x"},
    {"id": "amp_reset", "menu": "amp_reset", "path": "View > Waveform Zoom", "title": "Reset", "menu_key": "Control+Option+Cmd+[", "page_keys": [], "cubase": [], "note": "back to 1x (Zoom to Project does the same)"},
    {"id": "zoom_full", "menu": "zoom_full", "path": "View > Zoom Presets", "title": "Full", "menu_key": "Control+1", "page_keys": ["1"], "cubase": null, "note": null},
    {"id": "zoom_30s", "menu": "zoom_30s", "path": "View > Zoom Presets", "title": "30 s", "menu_key": "Control+2", "page_keys": ["2"], "cubase": null, "note": null},
    {"id": "zoom_5s", "menu": "zoom_5s", "path": "View > Zoom Presets", "title": "5 s", "menu_key": "Control+3", "page_keys": ["3"], "cubase": null, "note": null},
    {"id": "zoom_500ms", "menu": "zoom_500ms", "path": "View > Zoom Presets", "title": "500 ms", "menu_key": "Control+4", "page_keys": ["4"], "cubase": null, "note": null},
    {"id": "zoom_50ms", "menu": "zoom_50ms", "path": "View > Zoom Presets", "title": "50 ms", "menu_key": "Control+5", "page_keys": ["5"], "cubase": null, "note": null},
    {"id": "zoom_5ms", "menu": "zoom_5ms", "path": "View > Zoom Presets", "title": "5 ms", "menu_key": "Control+6", "page_keys": ["6"], "cubase": null, "note": null},
    {"id": "zoom_add", "menu": "zoom_add", "path": "View > Zoom Presets", "title": "Add Current…", "menu_key": null, "page_keys": [], "cubase": null, "note": "#169: saves the span of the view under a name; it shows in the zoom dropdown and at the end of this menu, where the first five have Option+1 to Option+5 (no entry in this table: the own presets come from the app settings)"},
    {"id": "zoom_organize", "menu": "zoom_organize", "path": "View > Zoom Presets", "title": "Organize…", "menu_key": null, "page_keys": [], "cubase": null, "note": "#169: renames and removes the own presets"},
    {"id": "height_taller", "menu": "height_taller", "path": "View > Track Height", "title": "Taller", "menu_key": "Cmd+Down", "page_keys": ["Option+Down"], "cubase": ["Cmd+Down"], "note": "the next larger height of every track (small, medium, large)"},
    {"id": "height_shorter", "menu": "height_shorter", "path": "View > Track Height", "title": "Shorter", "menu_key": "Cmd+Up", "page_keys": ["Option+Up"], "cubase": ["Cmd+Up"], "note": "the next smaller height"},
    {"id": "height_small", "menu": "height_small", "path": "View > Track Height", "title": "Small", "menu_key": null, "page_keys": ["Option+Cmd+1"], "cubase": null, "note": "wave lane 56 px; #439: the key follows the Option+1..5 zoom presets"},
    {"id": "height_medium", "menu": "height_medium", "path": "View > Track Height", "title": "Medium", "menu_key": null, "page_keys": ["Option+Cmd+2"], "cubase": null, "note": "wave lane 120 px (the default); #439"},
    {"id": "height_large", "menu": "height_large", "path": "View > Track Height", "title": "Large", "menu_key": null, "page_keys": ["Option+Cmd+3"], "cubase": null, "note": "wave lane 240 px; #439"},
    {"id": "inspector", "menu": "inspector", "path": "View", "title": "Show Inspector", "menu_key": "Option+I", "page_keys": [], "cubase": null, "note": "the left zone: name, source, hit counts, detection parameters, volume and pan of the selected track"},
    {"id": "info_line", "menu": "info_line", "path": "View", "title": "Show Info Line", "menu_key": "Cmd+I", "page_keys": [], "cubase": null, "note": "the selected hit; sample and velocity can be typed (Return applies, Escape cancels)"},
    {"id": "overview", "menu": "overview", "path": "View", "title": "Show Overview", "menu_key": "Option+O", "page_keys": [], "cubase": null, "note": "#169: the overview strip over the lanes; hidden, the lanes take its height; the choice is kept"},
    {"id": "counter_seconds", "menu": "counter_seconds", "path": "View > Main Counter", "title": "Seconds", "menu_key": null, "page_keys": [], "cubase": null, "note": "6 decimals"},
    {"id": "counter_samples", "menu": "counter_samples", "path": "View > Main Counter", "title": "Samples", "menu_key": null, "page_keys": [], "cubase": null, "note": "at the rate of the selected track"},
    {"id": "counter_barsbeats", "menu": "counter_barsbeats", "path": "View > Main Counter", "title": "Bars|Beats", "menu_key": null, "page_keys": [], "cubase": null, "note": "bar|beat|ticks of the tempo map, 960 ticks per beat; without a map the default 120 BPM 4/4 (#149)"},
    {"id": "counter_timecode", "menu": "counter_timecode", "path": "View > Main Counter", "title": "Timecode (30 fps)", "menu_key": null, "page_keys": [], "cubase": null, "note": "HH:MM:SS:FF at 30 fps from the sample position; the rate is fixed, not the session's (#149)"},
    {"id": "counter_exchange", "menu": "counter_exchange", "path": "View > Main Counter", "title": "Exchange Counters", "menu_key": null, "page_keys": [], "cubase": ["."], "note": "click the main counter; the key . nudges a hit in the default preset"},
    {"id": "tab_active", "menu": "tab_active", "path": "View > Tab Moves To", "title": "Active Hits Only", "menu_key": null, "page_keys": [], "cubase": null, "note": "#175: the default; Tab and Shift+Tab skip the deactivated hits"},
    {"id": "tab_all", "menu": "tab_all", "path": "View > Tab Moves To", "title": "All Found Hits", "menu_key": null, "page_keys": [], "cubase": null, "note": "#175: Tab and Shift+Tab visit the deactivated hits too"},
    {"id": "ruler_tempo", "menu": "ruler_tempo", "path": "View", "title": "Tempo Ruler", "menu_key": null, "page_keys": [], "cubase": null, "note": "tempo steps above the lanes: double-click adds one, double-click a value edits it, Delete removes the selected one; shown once a map is set, until a toggle stores your choice"},
    {"id": "ruler_meter", "menu": "ruler_meter", "path": "View", "title": "Meter Ruler", "menu_key": null, "page_keys": [], "cubase": null, "note": "meter changes above the lanes: double-click adds one at a bar, double-click a value edits it, Delete removes the selected one"},
    {"id": "ruler_beats", "menu": "ruler_beats", "path": "View", "title": "Bars|Beats Ruler", "menu_key": null, "page_keys": ["Shift+B"], "cubase": null, "note": "bar numbers and beat ticks from the tempo and meter map; a click puts the playhead; #439: B is a Cubase key (next hit on all tracks), so the preset key is Shift+B"},
    {"id": "ruler_markers", "menu": "ruler_markers", "path": "View", "title": "Markers Ruler", "menu_key": null, "page_keys": ["Shift+R"], "cubase": null, "note": "review markers above the lanes (P4-40g): M sets one at the selected hit or the playhead; a click puts the playhead, a click on a marker selects it (Delete removes it), a double-click edits its note; Option+M and Option+Shift+M jump; shown when the project has markers, until a toggle stores your choice; #439: the preset key is Shift+R (R for review markers)"},
    {"id": "grid", "menu": "grid", "path": "View", "title": "Show Bar and Beat Grid", "menu_key": null, "page_keys": ["G"], "cubase": [], "note": "light bar and beat lines in the lanes, under the hit lines; no snap; shown once a map is set, until a toggle stores your choice (G is a Pro Tools preset key only: the Cubase preset zooms out with G)"},
    {"id": "velocity_lane", "menu": "velocity_lane", "path": "View", "title": "Velocity Lane", "menu_key": null, "page_keys": ["V"], "cubase": null, "note": "Drag a stalk to set velocity. Option-drag: draw a line. Cmd-drag: draw freehand. One stalk of a selected range scales the range by the same ratio. A stalk's height is its velocity (127 top, 0 bottom); grey stalks leave the output. Drag the top border to resize the lane. One gesture is one undo step"},
    {"id": "other_mics", "menu": "other_mics", "path": "View", "title": "Other Mics Under Hit", "menu_key": null, "page_keys": ["O"], "cubase": null, "note": "#157: a panel under the lanes with the other tracks in the window of the selected hit (+-50 ms), the hit time as a line; shown only with a selected hit"},
    {"id": "play_stop", "menu": "play_stop", "path": "Transport", "title": "Play/Stop", "menu_key": null, "page_keys": ["Space", "Numpad0", "Option+Space"], "cubase": ["Space"], "note": "from the playhead; Option+Space is play-and-return: the next Space or Option+Space stops and puts the playhead back where this play started"},
    {"id": "half_speed", "menu": "half_speed", "path": "Transport", "title": "Half-Speed Play", "menu_key": null, "page_keys": ["Shift+Space", "Option+Shift+Space"], "cubase": [], "note": "from the playhead at half speed, one octave down (the clicks too); Space or Shift+Space stops; Option+Shift+Space is play-and-return: the next Space or Option+Shift+Space stops and puts the playhead back where this play started"},
    {"id": "stop_return", "menu": "stop_return", "path": "Transport", "title": "Stop and Return", "menu_key": null, "page_keys": [], "cubase": ["Numpad0"], "note": "the ■ button"},
    {"id": "go_start", "menu": "go_start", "path": "Transport", "title": "Go to Project Start", "menu_key": null, "page_keys": ["Return", "Control+Return"], "cubase": ["NumpadPeriod", "Return"], "note": "Cubase preset: numpad . or Return"},
    {"id": "go_end", "menu": "go_end", "path": "Transport", "title": "Go to Project End", "menu_key": "Option+Return", "page_keys": [], "cubase": null, "note": null},
    {"id": "go_play_start", "menu": "go_play_start", "path": "Transport", "title": "Go to Playback Start", "menu_key": null, "page_keys": ["Shift+Return"], "cubase": null, "note": null},
    {"id": "marker_next", "menu": "marker_next", "path": "Transport", "title": "Next Marker", "menu_key": "Option+M", "page_keys": [], "cubase": null, "note": "P4-40g: moves the playhead to the next marker"},
    {"id": "marker_prev", "menu": "marker_prev", "path": "Transport", "title": "Previous Marker", "menu_key": "Option+Shift+M", "page_keys": [], "cubase": null, "note": "P4-40g: moves the playhead to the previous marker"},
    {"id": "follow_smooth", "menu": "follow_smooth", "path": "Transport > Follow Playhead", "title": "Smooth", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "follow_page", "menu": "follow_page", "path": "Transport > Follow Playhead", "title": "Page", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "follow_off", "menu": "follow_off", "path": "Transport > Follow Playhead", "title": "Off", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "follow_toggle", "menu": "follow_toggle", "path": "Transport", "title": "Follow Playhead On/Off", "menu_key": null, "page_keys": ["F"], "cubase": null, "note": null},
    {"id": "suspend_follow", "menu": "suspend_follow", "path": "Transport", "title": "Suspend Follow when Editing", "menu_key": null, "page_keys": [], "cubase": null, "note": "P4-wf10: an add, move or delete during playback holds the view for 2 s of playback, then the follow resumes at the next page; a user setting"},
    {"id": "stop_return_mode", "menu": "stop_return_mode", "path": "Transport", "title": "Stop Returns to Start", "menu_key": null, "page_keys": ["I"], "cubase": null, "note": "when on, a Stop (Space while playing, or the ■ button) puts the playhead back where the play started, like Option+Space; a user setting that survives a restart (I is free in both presets)"},
    {"id": "audition_tab", "menu": "audition_tab", "path": "Transport", "title": "Audition on Tab", "menu_key": null, "page_keys": ["U"], "cubase": null, "note": "#240: Tab plays the hit it steps to (Settings > Audition on Tab); a user setting that survives a restart (U is free in both presets)"},
    {"id": "click", "menu": "click", "path": "Transport", "title": "Metronome Click", "menu_key": null, "page_keys": ["C", "Numpad7"], "cubase": ["C"], "note": null},
    {"id": "next_hit", "menu": "next_hit", "path": "Hits", "title": "Next Hit", "menu_key": null, "page_keys": ["Tab", "'"], "cubase": ["Option+N"], "note": "on the selected track; all tracks when no track is selected"},
    {"id": "prev_hit", "menu": "prev_hit", "path": "Hits", "title": "Previous Hit", "menu_key": "Option+Tab", "page_keys": ["Shift+Tab", "L"], "cubase": ["Option+B"], "note": null},
    {"id": "next_hit_all", "menu": "next_hit_all", "path": "Hits", "title": "Next Hit on All Tracks", "menu_key": "Control+Tab", "page_keys": ["N", "Control+Cmd+'"], "cubase": ["N"], "note": null},
    {"id": "prev_hit_all", "menu": "prev_hit_all", "path": "Hits", "title": "Previous Hit on All Tracks", "menu_key": "Control+Option+Tab", "page_keys": ["Shift+N", "Control+Cmd+L"], "cubase": ["B"], "note": null},
    {"id": "review_next", "menu": "review_next", "path": "Hits", "title": "Next Doubtful Hit", "menu_key": null, "page_keys": ["D"], "cubase": null, "note": "the review queue: the doubtful hits of all tracks, lowest confidence first; the info line says why a hit is doubtful"},
    {"id": "review_prev", "menu": "review_prev", "path": "Hits", "title": "Previous Doubtful Hit", "menu_key": null, "page_keys": ["Shift+D"], "cubase": null, "note": null},
    {"id": "audition", "menu": "audition", "path": "Hits", "title": "Audition Hit", "menu_key": null, "page_keys": ["A", "Shift+A"], "cubase": ["Option+A", "Option+Shift+A"], "note": "the selected hit on its track alone, 30 ms before to 250 ms after; Shift loops it until Escape or Space; a range selection loops the range; Half-Speed applies; Settings > Audition on Tab plays the hit that Tab steps to"},
    {"id": "snap", "menu": "snap", "path": "Hits", "title": "Snap Playhead to Hits", "menu_key": null, "page_keys": ["J"], "cubase": null, "note": null},
    {"id": "analyze_track", "menu": "analyze_track", "path": "Hits", "title": "Analyze Selected Track", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "analyze_all", "menu": "analyze_all", "path": "Hits", "title": "Analyze All", "menu_key": null, "page_keys": [], "cubase": null, "note": null},
    {"id": "learn_this", "menu": "learn_this", "path": "Hits", "title": "Learn This", "menu_key": null, "page_keys": ["K"], "cubase": null, "note": "marks the selected range (drag in a wave lane) or the selected hit as a good example; the hits of the range are taken, or the strongest candidate, or the strongest rise of 6 dB or more"},
    {"id": "learn_not", "menu": "learn_not", "path": "Hits", "title": "Forget This", "menu_key": null, "page_keys": ["Shift+K"], "cubase": null, "note": "marks the selected range or hit as a bad example (bleed, a ghost note to drop); a hit marked the other way changes kind"},
    {"id": "learn_forget_this", "menu": "learn_forget_this", "path": "Hits", "title": "Remove Example", "menu_key": null, "page_keys": ["Option+K"], "cubase": null, "note": "removes the Learn example at the selected hit or range (or the selected mark bar); one undo step; nothing there changes nothing"},
    {"id": "learn_filter", "menu": "learn_filter", "path": "Hits", "title": "Learn Filter", "menu_key": null, "page_keys": [], "cubase": null, "note": "check item: the Learn filter of the selected track (P4-6c)"},
    {"id": "learn_forget", "menu": "learn_forget", "path": "Hits", "title": "Forget Examples of Track", "menu_key": null, "page_keys": [], "cubase": null, "note": "removes every Learn mark of the selected track (Undo brings them back)"},
    {"id": "prev_track", "menu": "prev_track", "path": "Track", "title": "Select Previous Track", "menu_key": null, "page_keys": ["P"], "cubase": [], "note": "the Inspector and the header follow; nothing happens at the first track; no track selected: the last"},
    {"id": "next_track", "menu": "next_track", "path": "Track", "title": "Select Next Track", "menu_key": null, "page_keys": [";"], "cubase": [], "note": "nothing happens at the last track; no track selected: the first"},
    {"id": "mute_track", "menu": "mute_track", "path": "Track", "title": "Mute Selected Track", "menu_key": null, "page_keys": ["Shift+M"], "cubase": ["M"], "note": null},
    {"id": "solo_track", "menu": "solo_track", "path": "Track", "title": "Solo Selected Track", "menu_key": null, "page_keys": ["Shift+S"], "cubase": ["S"], "note": null},
    {"id": "reset_mix", "menu": "reset_mix", "path": "Track", "title": "Reset Volume and Pan", "menu_key": null, "page_keys": [], "cubase": null, "note": "of the selected track: 0 dB, centre"},
    {"id": "follow_own", "menu": "follow_own", "path": "Track", "title": "Own Detection", "menu_key": null, "page_keys": [], "cubase": null, "note": "the selected track detects its hits itself; Track > Follow Hits Of takes them from another track (a kick sub mic follows the kick in)"},
    {"id": "detect_tempo", "menu": "detect_tempo", "path": "Track", "title": "Detect Tempo…", "menu_key": null, "page_keys": [], "cubase": null, "note": "suggests the tempo, bar 1, the meter and tempo steps from the kick and snare hits (analyze them first); Accept is one undo step"},
    {"id": "set_bar1", "menu": "set_bar1", "path": "Track", "title": "Set Bar 1 at Playhead", "menu_key": null, "page_keys": [], "cubase": null, "note": "moves bar 1 of the tempo map to the playhead (or to the selected hit); the tempo, steps and meter stay; one undo step. Also: drag the bar-1 marker of the Bars|Beats ruler, or double-click it to type the time"},
    {"id": "set_bar1_audio", "menu": "set_bar1_audio", "path": "Track", "title": "Set Bar 1 at Audio Start", "menu_key": null, "page_keys": [], "cubase": null, "note": "sets bar 1 of the tempo map to 0 s, the DAW convention (the session start is bar 1); one undo step"},
    {"id": "export_selected", "menu": "export_selected", "path": "Track", "title": "Export Selected Tracks…", "menu_key": "Shift+Cmd+E", "page_keys": [], "cubase": null, "note": "opens the Export dialog with the selected track"},
    {"id": "tracks_remove", "menu": "tracks_remove", "path": "Track", "title": "Remove Track", "menu_key": null, "page_keys": [], "cubase": null, "note": "removes the selected track with its hits after a confirm; Delete or Backspace does it when no hit and no marker is selected; Undo brings the track back"},
    {"id": "shortcuts", "menu": "shortcuts", "path": "Help", "title": "Keyboard Shortcuts", "menu_key": "Control+Shift+K", "page_keys": [], "cubase": [], "note": "this panel"},
    {"id": "nudge_earlier_10", "menu": null, "path": null, "title": "Nudge Hit Earlier by 10 x the Value", "menu_key": null, "page_keys": ["Shift+Left"], "cubase": null, "note": "10 x the nudge value; no hit selected: the key pans the view"},
    {"id": "nudge_later_10", "menu": null, "path": null, "title": "Nudge Hit Later by 10 x the Value", "menu_key": null, "page_keys": ["Shift+Right"], "cubase": null, "note": "10 x the nudge value; no hit selected: the key pans the view"},
    {"id": "nudge_value_up", "menu": null, "path": null, "title": "Nudge Value Up", "menu_key": null, "page_keys": ["Option+Cmd+NumpadPlus"], "cubase": [], "note": "the next larger value: 1, 10, 100 samples, 1 ms, 10 ms"},
    {"id": "nudge_value_down", "menu": null, "path": null, "title": "Nudge Value Down", "menu_key": null, "page_keys": ["Option+Cmd+NumpadMinus"], "cubase": [], "note": "the next smaller value"},
    {"id": "zoom_keep", "menu": null, "path": null, "title": "Keep the Zoom", "menu_key": null, "page_keys": ["Option+Shift+E"], "cubase": [], "note": "forget the zoom that E stored; the view stays"},
    {"id": "deselect", "menu": null, "path": null, "title": "Deselect the Hit", "menu_key": null, "page_keys": ["Escape"], "cubase": null, "note": null}
  ]/*KEYMAP-END*/;

  /* The mouse gestures of the help panel (no commands, no keys). */
  const GESTURES = [
    ["click the main counter", "Exchange the main and the sub counter (seconds and samples)"],
    ["click lane or ruler", "Put the playhead (orange line); it snaps to a near hit when Snap is on"],
    ["drag wave lane", "Select a range on that track (K: Learn This, Shift+K: Forget This; Escape clears it)"],
    ["drag empty spike lane", "Pan the view"],
    ["wheel, Shift+wheel", "Scroll the track list; pan the timeline (also two-finger sideways)"],
    ["click track header", "Select the track: the Inspector shows it; Tab, Analyze Selected Track, Mute, Solo and Export Selected Tracks use it"],
    ["drag track header", "Reorder the tracks (the order is saved)"],
    ["double-click or Option-click a lane", "Add a hit there (also Edit > Add Hit at Playhead)"],
    ["drag spike", "Move the hit; the status bar shows the change in samples"],
    ["right-click spike", "Delete the hit"],
    ["right-click a selection or a Learn mark bar", "Learn This, Forget This, Clear Selection; Remove Example, Change to Forget This (or This)"],
    ["offline", "Track header: the file is missing or changed; click the badge to relink (the Inspector shows the source and its state)"],
  ];

  const MODS = ["Control", "Option", "Shift", "Cmd"];
  const NAMES = ["Plus", "Minus", "Space", "Tab", "Return", "Delete", "Backspace", "Escape", "Left", "Right", "Up", "Down",
    "NumpadMinus", "NumpadPlus", "NumpadPeriod", "NumpadEnter", "Numpad0", "Numpad1", "Numpad2", "Numpad3", "Numpad4",
    "Numpad5", "Numpad6", "Numpad7", "Numpad8", "Numpad9"];
  const CHARS = "[],.=_/;'`\\";
  const isChar = k => k.length === 1 && CHARS.includes(k);

  /* "Shift+Cmd+S" -> {mods: ["Shift", "Cmd"], key: "S"}; null when the key is not written in the table's notation */
  function parse(s) {
    if (typeof s !== "string" || !s) return null;
    const parts = s.split("+"), key = parts.pop(), mods = parts;
    const order = mods.map(m => MODS.indexOf(m));
    if (order.some((o, i) => o < 0 || (i > 0 && o <= order[i - 1]))) return null;
    if (!(/^[A-Z0-9]$/.test(key) || NAMES.includes(key) || isChar(key))) return null;
    if (isChar(key) && mods.includes("Shift")) return null;
    return { mods, key };
  }
  const compose = (mods, key) => MODS.filter(m => mods.includes(m)).concat([key]).join("+");
  const menuMod = k => { const p = parse(k); return !!p && p.mods.some(m => m !== "Shift"); };

  /* The key of a keydown event in the table's notation, or null */
  const NAMED = { " ": "Space", Enter: "Return", Tab: "Tab", Delete: "Delete", Backspace: "Backspace", Escape: "Escape",
    ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down", "+": "Plus", "-": "Minus" };
  const NUMPAD = { NumpadSubtract: "NumpadMinus", NumpadAdd: "NumpadPlus", NumpadDecimal: "NumpadPeriod", NumpadEnter: "NumpadEnter" };
  const CODE_CHARS = { BracketLeft: "[", BracketRight: "]" };
  function keyOf(ev) {
    const code = ev.code || "", k = String(ev.key || "");
    let key = NUMPAD[code] || (/^Numpad[0-9]$/.test(code) ? code : null);
    let char = false;
    if (!key && NAMED[k]) { key = NAMED[k]; char = k === "+" || k === "-"; }
    else if (!key && /^[a-z0-9]$/i.test(k)) key = k.toUpperCase();
    else if (!key && isChar(k)) { key = k; char = true; }
    else if (!key && /^Key[A-Z]$/.test(code)) key = code.slice(3);     // Option changes ev.key on macOS (Option+N is a dead key)
    else if (!key && /^Digit[0-9]$/.test(code)) key = code.slice(5);
    else if (!key && CODE_CHARS[code]) { key = CODE_CHARS[code]; char = true; }   // P3-20a: Option+] types a dead character on macOS
    else if (!key && code === "Space") key = "Space";   // #207: Option+Space is a no-break space on macOS (ev.key changes, the code does not)
    if (!key) return null;
    const mods = [];
    if (ev.ctrlKey) mods.push("Control");
    if (ev.altKey) mods.push("Option");
    if (ev.shiftKey && !char) mods.push("Shift");
    if (ev.metaKey) mods.push("Cmd");
    return compose(mods, key);
  }

  /* The text of a key for people: "Shift+Cmd+S", "numpad -", "+" */
  const SHOW = { Plus: "+", Minus: "-", NumpadMinus: "numpad -", NumpadPlus: "numpad +", NumpadPeriod: "numpad .", NumpadEnter: "numpad Enter" };
  function display(s) {
    const p = parse(s);
    if (!p) return String(s);
    const key = SHOW[p.key] || (/^Numpad[0-9]$/.test(p.key) ? "numpad " + p.key.slice(6) : p.key);
    return p.mods.concat([key]).join("+");
  }

  /* The menu accelerator in the notation of menu.rs (Tauri): the self-test compares it with the live menu */
  const TAURI_KEY = { Return: "Enter", NumpadMinus: "NumpadSubtract", NumpadPlus: "NumpadAdd", NumpadPeriod: "NumpadDecimal", Left: "ArrowLeft",
    Right: "ArrowRight", Up: "ArrowUp", Down: "ArrowDown" };
  function tauriOf(s) {
    const p = parse(s);
    if (!p) return null;
    const m = [["Control", "Ctrl"], ["Cmd", "CmdOrCtrl"], ["Option", "Alt"], ["Shift", "Shift"]].filter(([a]) => p.mods.includes(a)).map(([, b]) => b);
    return m.concat([TAURI_KEY[p.key] || p.key]).join("+");
  }

  /* The preset names: "default" is shown as "Pro Tools (default)". The choice is the app setting "key_preset". */
  const PRESETS = { default: "Pro Tools (default)", cubase: "Cubase" };
  const isPreset = p => Object.prototype.hasOwnProperty.call(PRESETS, p);

  /* The keys of a command in a preset: "default" (Pro Tools) or "cubase" */
  function keysOf(c, preset) {
    if (preset === "cubase" && Array.isArray(c.cubase)) return c.cubase.slice();
    return (c.menu_key ? [c.menu_key] : []).concat(c.page_keys || []);
  }

  /* The native menu accelerator of a command in a preset (P3-13c), or null: the menu_key of the default preset;
     in "cubase" the first key of the row's list with Cmd, Control or Option (a row without a list keeps menu_key). */
  function menuKeyOf(c, preset) {
    if (!c.menu) return null;
    if (preset === "cubase" && Array.isArray(c.cubase)) return c.cubase.find(menuMod) || null;
    return c.menu_key || null;
  }

  /* Everything wrong with a table: a key in two commands (per preset), a key not in the notation, a menu key
     without Cmd, Control or Option, an id twice. [] when the table is good. menu.rs runs the same checks. */
  function problems(cmds) {
    const out = [], ids = new Set();
    for (const c of cmds) {
      if (ids.has(c.id)) out.push(`${c.id}: the id is twice in the table`);
      ids.add(c.id);
      if (c.menu_key && !menuMod(c.menu_key)) out.push(`${c.id}: menu key ${c.menu_key} has no Cmd, Control or Option`);
      if (c.menu_key && !c.menu) out.push(`${c.id}: menu key ${c.menu_key} without a menu item`);
    }
    for (const preset of ["default", "cubase"]) {
      const seen = new Map();
      for (const c of cmds) {
        for (const k of keysOf(c, preset)) {
          if (!parse(k)) { out.push(`${c.id}: key "${k}" is not in the table's notation`); continue; }
          if (seen.has(k) && seen.get(k) !== c.id) out.push(`${preset}: key ${k} is in two commands: ${seen.get(k)} and ${c.id}`);
          else if (seen.has(k)) out.push(`${preset}: key ${k} is twice in ${c.id}`);
          seen.set(k, c.id);
        }
      }
    }
    return out;
  }

  /* key -> {cmd, menu}: menu = the key is the command's menu accelerator (the native menu runs it in the app) */
  const tables = {};
  function table(preset) {
    const p = preset || DSKeys.preset;
    if (tables[p]) return tables[p];
    const t = new Map();
    for (const c of COMMANDS) for (const k of keysOf(c, p)) if (!t.has(k)) t.set(k, { cmd: c, menu: k === menuKeyOf(c, p) });
    return (tables[p] = t);
  }

  /* The command of a keydown event: the key itself, then a numpad key as its main key (numpad 1 = 1), then
     Control as Cmd (Control+S saves as on other systems). {cmd, key} or null. */
  const MAIN = { NumpadMinus: "Minus", NumpadPlus: "Plus", NumpadPeriod: ".", NumpadEnter: "Return" };
  function find(ev) {
    const k = keyOf(ev);
    if (!k) return null;
    const t = table(), p = parse(k), tries = [k];
    const main = MAIN[p.key] || (/^Numpad[0-9]$/.test(p.key) ? p.key.slice(6) : null);
    if (main) tries.push(compose(isChar(main) || main === "Minus" || main === "Plus" ? p.mods.filter(m => m !== "Shift") : p.mods, main));
    if (p.mods.includes("Control") && !p.mods.includes("Cmd")) tries.push(compose(p.mods.filter(m => m !== "Control").concat(["Cmd"]), p.key));
    for (const x of tries) if (t.has(x)) return { cmd: t.get(x).cmd, key: x };
    return null;
  }

  /* The native menu runs this key (app only): the page must not run it again */
  function menuOwns(ev) {
    if (!window.__DS_NATIVE_MENU__) return false;
    const k = keyOf(ev), hit = k && table().get(k);
    return !!(hit && hit.menu);
  }
  const menuKeys = preset => COMMANDS.map(c => menuKeyOf(c, preset || DSKeys.preset)).filter(Boolean);
  const presetName = preset => PRESETS[preset || DSKeys.preset];
  /* Switch the preset of the page (the menu follows through DSShell.sync). false when the name is not a preset. */
  function setPreset(p) {
    if (!isPreset(p)) return false;
    DSKeys.preset = p;
    return true;
  }

  /* The help panel: one row per command, grouped by menu in the order of the table, then the gestures */
  function renderHelp(el) {
    if (!el) return 0;
    el.textContent = "";
    const row = (cls, a, b, id) => {
      const tr = document.createElement("tr");
      if (cls) tr.className = cls;
      if (id) tr.dataset.cmd = id;
      const ka = document.createElement("td"), kb = document.createElement("td");
      if (a !== null) { const bold = document.createElement("b"); bold.textContent = a; ka.appendChild(bold); }
      kb.textContent = b;
      tr.append(ka, kb); el.appendChild(tr);
    };
    let group = undefined, n = 0;
    for (const c of COMMANDS) {
      const g = c.path ? c.path.split(" > ")[0] : "Keys without a menu item";
      if (g !== group) { group = g; row("kmGroup", null, g); }
      const keys = keysOf(c, DSKeys.preset).map(display).join(", ");
      const where = c.path ? `${c.path} > ${c.title}` : c.title;
      const live = DSKeys.live[c.id] ? DSKeys.live[c.id]() : "";   // P3-20a: a current value (the nudge value)
      row(keys ? "" : "kmNone", keys || "none", where + (c.note || live ? ` (${[c.note, live].filter(Boolean).join("; ")})` : ""), c.id);
      n++;
    }
    row("kmGroup", null, "Mouse");
    for (const [a, b] of GESTURES) row("", a, b);
    return n;
  }

  const DSKeys = { live: {}, COMMANDS, GESTURES, MODS, preset: "default", PRESETS, parse, compose, keyOf, display, tauriOf, keysOf, problems,
    table, find, menuOwns, menuKeys, menuKeyOf, presetName, setPreset, renderHelp };
  window.DSKeys = DSKeys;
})();
