use super::*;
use std::collections::HashSet;

#[test]
fn ids_are_unique_and_do_not_use_the_entry_prefixes() {
    let ids: HashSet<&str> = SPEC.iter().map(|i| i.id).collect();
    assert_eq!(ids.len(), SPEC.len());
    assert!(SPEC.iter().all(|i| !i.id.starts_with(RECENT_PREFIX)
        && !i.id.starts_with(PROJECT_PREFIX)
        && !i.id.starts_with(FOLLOW_PREFIX)
        && ![RECENT_ID, WINDOW_ID, PROJECT_ID, FOLLOW_ID, FOLLOW_TRACK_ID].contains(&i.id)));
    assert_eq!(
        SPEC.len(),
        109,
        "#169 adds Show Overview, Add Current and Organize; #157 adds Other Mics Under Hit; #158 adds Set Marker at Selection, Next Marker, Previous Marker, Zoom Toggle, Audition Hit, Select Previous Track and Select Next Track; #150 adds Next and Previous Doubtful Hit; P4-wf10 adds Suspend Follow when Editing; #436 adds Stop Returns to Start; P4-31 adds Export Again, P4-40f adds Velocity Lane, P4-40g adds Markers Ruler, P4-40a adds Undo History, P4-39a adds Remove Track, P4-39d adds Set Bar 1 at Playhead, P3-20a adds three Waveform Zoom and five Nudge Value items, P3-20b Half-Speed Play, P4-8 Import Tempo Map, Detect Tempo, three rulers and the grid, P4-6a Learn This, Not This, Learn Filter and Forget Examples, P4-36 Forget This; P4-30 drops Import and Export Project Zip, Export All, Export MIDI and Export Spikes for Export and Export Selected Tracks; #468 adds Set Bar 1 at Audio Start; #149 adds Bars|Beats and Timecode (30 fps); #175 adds Active Hits Only and All Found Hits (Tab Moves To)"
    );
}

/// The accelerators are written as muda parses them (modifiers, then one key), and each has Cmd, Control
/// or Option: a key without one is a page key, so a text field keeps it. The live parse is the self-test's
/// (the menu bar does not build with a wrong accelerator).
#[test]
fn shortcuts_are_unique_have_a_modifier_and_are_written_as_tauri_wants() {
    for preset in PRESETS {
        let mut seen = HashSet::new();
        for a in SPEC.iter().filter_map(|i| accel_of(i.id, preset)) {
            let parts: Vec<&str> = a.split('+').collect();
            let (key, mods) = parts.split_last().unwrap();
            assert!(
                mods.iter().all(|m| ["CmdOrCtrl", "Ctrl", "Alt", "Shift"].contains(m)),
                "{preset} {a}: modifier"
            );
            assert!(
                mods.iter().any(|m| ["CmdOrCtrl", "Ctrl", "Alt"].contains(m)),
                "{preset} {a}: no Cmd, Control or Option"
            );
            let one = key.len() == 1
                && key
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || "[],".contains(c));
            assert!(
                one || ["Tab", "Enter", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].contains(key),
                "{preset} {a}: key"
            );
            assert!(seen.insert(a.clone()), "{preset} {a} twice");
        }
        // P3-14c: Cmd+Down and Cmd+Up in both presets; P3-19: Open Most Recent and Keyboard Shortcuts have a key in the default preset only
        // P3-20a: Waveform Zoom In, Out and Reset have a key in the default preset only; P4-31: Export Again has a key in the default preset only
        // #158: Next Marker and Previous Marker have a key in both presets, Audition Hit has one in the Cubase preset only
        // #169: Show Overview (Option+O) has a key in both presets, like Show Inspector (Option+I)
        // #207: Export Selected Tracks and Undo History have a key in both presets
        assert_eq!(seen.len(), if preset == "default" { 39 } else { 34 }, "{preset}");
    }
}

/// The keys of reference section 3.2 and 3.3 (decision 1: Pro Tools; decision 2: Return = project start), as
/// the table gives them to the menu and to the page.
#[test]
fn the_reference_keys_are_the_accelerators_or_page_keys() {
    let acc = |i: &Item| accel_of(i.id, "default").unwrap();
    assert_eq!(acc(&SETTINGS), "CmdOrCtrl+,");
    assert_eq!(acc(&NEW), "CmdOrCtrl+N");
    assert_eq!(acc(&OPEN), "CmdOrCtrl+O");
    assert_eq!(acc(&CLOSE), "CmdOrCtrl+Shift+W"); // P3-19: Cmd+W is a page alias
    assert_eq!(acc(&SAVE), "CmdOrCtrl+S");
    assert_eq!(acc(&SAVE_AS), "CmdOrCtrl+Shift+S");
    assert_eq!(acc(&ADD_TRACKS), "CmdOrCtrl+Shift+I");
    assert_eq!(acc(&EXPORT), "CmdOrCtrl+E");
    assert_eq!(acc(&UNDO), "CmdOrCtrl+Z");
    assert_eq!(acc(&REDO), "CmdOrCtrl+Shift+Z");
    assert_eq!(acc(&ZOOM_IN), "CmdOrCtrl+]");
    assert_eq!(acc(&ZOOM_OUT), "CmdOrCtrl+[");
    assert_eq!(acc(&ZOOM_PROJECT), "Ctrl+CmdOrCtrl+[");
    assert_eq!(
        [ZOOM_FULL, ZOOM_30S, ZOOM_5S, ZOOM_500MS, ZOOM_50MS, ZOOM_5MS].map(|i| acc(&i)),
        ["Ctrl+1", "Ctrl+2", "Ctrl+3", "Ctrl+4", "Ctrl+5", "Ctrl+6"]
    );
    assert_eq!(acc(&INSPECTOR), "Alt+I");
    assert_eq!(acc(&HEIGHT_TALLER), "CmdOrCtrl+ArrowDown"); // P3-14c, reference 3.2 (Cubase Zoom In Tracks)
    assert_eq!(acc(&HEIGHT_SHORTER), "CmdOrCtrl+ArrowUp");
    assert!(
        [HEIGHT_SMALL, HEIGHT_MEDIUM, HEIGHT_LARGE]
            .iter()
            .all(|i| accel_of(i.id, "default").is_none() && accel_of(i.id, "cubase").is_none())
    );
    assert_eq!(acc(&GO_END), "Alt+Enter");
    assert_eq!(acc(&PREV_HIT), "Alt+Tab");
    assert_eq!(acc(&NEXT_HIT_ALL), "Ctrl+Tab");
    assert_eq!(acc(&PREV_HIT_ALL), "Ctrl+Alt+Tab");
    assert_eq!(acc(&QUIT), "CmdOrCtrl+Q");
    let page = |id: &str| keymap().iter().find(|c| c.id == id).unwrap().page_keys.join(" ");
    assert_eq!(page("delete_hit"), "Delete Backspace");
    assert_eq!(page("nudge_earlier"), "Left NumpadMinus ,");
    assert_eq!(page("nudge_later"), "Right NumpadPlus .");
    assert_eq!(page("play_stop"), "Space Numpad0 Option+Space"); // #207: Option+Space is play-and-return
    assert_eq!(page("half_speed"), "Shift+Space Option+Shift+Space"); // P3-20b, A1 p. 21; #207: play-and-return at half speed
    assert_eq!(accel_of("half_speed", "default"), None);
    assert_eq!(page("go_start"), "Return Control+Return");
    assert_eq!(page("go_play_start"), "Shift+Return");
    assert_eq!(page("follow_toggle"), "F");
    assert_eq!(page("click"), "C Numpad7");
    assert_eq!(page("next_hit"), "Tab '");
    assert_eq!(page("prev_hit"), "Shift+Tab L");
    assert_eq!(page("next_hit_all"), "N Control+Cmd+'");
    assert_eq!(page("prev_hit_all"), "Shift+N Control+Cmd+L");
    assert_eq!(page("snap"), "J");
    assert_eq!(page("mute_track"), "Shift+M");
    assert_eq!(page("solo_track"), "Shift+S");
    assert_eq!(page("zoom_in"), "T Plus =");
    assert_eq!(page("zoom_out"), "R Minus _");
    assert_eq!(page("redo"), "Shift+Z Cmd+Y");
    // P3-19: the Pro Tools keys of card P3-19 (A1 = Avid Pro Tools Shortcuts Guide 2025.10)
    assert_eq!(page("undo"), "Z");
    assert_eq!(page("zoom_project"), "Option+A");
    assert_eq!(page("close"), "Cmd+W");
    assert_eq!(page("height_taller"), "Option+Down");
    assert_eq!(page("height_shorter"), "Option+Up");
    assert_eq!(page("prev_track"), "P");
    assert_eq!(page("next_track"), ";");
    assert_eq!(page("zoom_toggle"), "E");
    assert_eq!(page("zoom_keep"), "Option+Shift+E");
    // P3-20a (A1 p. 33, 34, 28): Waveform Zoom keys are menu keys, the Nudge Value keys are page keys
    assert_eq!(acc(&AMP_IN), "CmdOrCtrl+Alt+]");
    assert_eq!(acc(&AMP_OUT), "CmdOrCtrl+Alt+[");
    assert_eq!(acc(&AMP_RESET), "Ctrl+CmdOrCtrl+Alt+[");
    assert_eq!(page("nudge_value_up"), "Option+Cmd+NumpadPlus");
    assert_eq!(page("nudge_value_down"), "Option+Cmd+NumpadMinus");
    assert!(NUDGE_MODES.iter().all(|(_, id)| accel_of(id, "default").is_none()));
    assert_eq!(page("counter_exchange"), "");
    // #207: the convenience keys (no DAW convention; decided per row, the skipped ones stay without a key)
    assert_eq!(acc(&EXPORT_SELECTED), "CmdOrCtrl+Shift+E"); // the E family: Export (Cmd+E), Export Again (Option+Cmd+E)
    assert_eq!(acc(&UNDO_HISTORY), "CmdOrCtrl+Alt+Z"); // the Z family: Undo (Cmd+Z), Redo (Shift+Cmd+Z)
    assert_eq!(page("add_hit"), "H"); // H for Hit; the Cubase preset zooms in with H
    assert_eq!(page("velocity_lane"), "V"); // V for Velocity
    assert_eq!(page("grid"), "G"); // G for Grid; the Cubase preset zooms out with G
    assert_eq!(accel_of("add_hit", "cubase"), None);
    assert_eq!(accel_of("grid", "cubase"), None);
    assert_eq!(acc(&OPEN_RECENT), "CmdOrCtrl+Shift+O");
    assert_eq!(acc(&SHORTCUTS), "Ctrl+Shift+K");
    // the Cubase preset (P3-13c) of reference 3.3
    let cubase = |id: &str| keymap().iter().find(|c| c.id == id).unwrap().keys("cubase").join(" ");
    // #207: the Cubase list REPLACES the default keys, so the preset has no play-and-return; add_hit and grid
    // have no key there (the Cubase preset zooms with H and G), velocity_lane keeps its key
    assert_eq!(cubase("add_hit"), "");
    assert_eq!(cubase("grid"), "");
    assert_eq!(cubase("velocity_lane"), "V");
    assert_eq!(cubase("play_stop"), "Space");
    assert_eq!(cubase("half_speed"), "");
    assert_eq!(
        [
            "next_hit",
            "prev_hit",
            "next_hit_all",
            "prev_hit_all",
            "zoom_in",
            "zoom_out",
            "zoom_project"
        ]
        .map(cubase),
        ["Option+N", "Option+B", "N", "B", "H Cmd+]", "G Cmd+[", "Shift+F"]
    );
    assert_eq!(
        [
            "mute_track",
            "solo_track",
            "nudge_earlier",
            "nudge_later",
            "go_start",
            "stop_return"
        ]
        .map(cubase),
        ["M", "S", "Cmd+Left", "Cmd+Right", "NumpadPeriod Return", "Numpad0"]
    );
    assert_eq!(cubase("snap"), "J");
    // owner decision 2026-10-11: the preset REPLACES the nudge keys, the plain arrows and the numpad keys are gone
    assert_eq!(
        [cubase("nudge_earlier"), cubase("nudge_later")],
        ["Cmd+Left", "Cmd+Right"]
    );
    // the gaps of P3-13b: Zoom In and Out keep Cmd+] and Cmd+[, Go to Project Start has a laptop key
    assert_eq!([cubase("zoom_in"), cubase("zoom_out")], ["H Cmd+]", "G Cmd+["]);
}

/// P3-19: the Pro Tools keys go into the default preset only. Every row that got a key keeps its old keys in the
/// Cubase preset (an explicit list), and the new rows have no key there.
#[test]
fn the_cubase_preset_keeps_the_keys_it_had_before_p3_19() {
    let cubase = |id: &str| keymap().iter().find(|c| c.id == id).unwrap().keys("cubase").join(" ");
    for (id, keys) in [
        ("close", "Cmd+W"),
        ("undo", "Cmd+Z"),
        ("redo", "Shift+Cmd+Z Cmd+Y"),
        ("height_taller", "Cmd+Down"),
        ("height_shorter", "Cmd+Up"),
        ("counter_exchange", "."),
        ("play_stop", "Space"),
        ("click", "C"),
        ("stop_return", "Numpad0"),
        ("go_start", "NumpadPeriod Return"),
        ("zoom_project", "Shift+F"),
        ("shortcuts", ""),
        ("open_recent", ""),
        ("prev_track", ""),
        ("next_track", ""),
        ("zoom_toggle", ""),
        ("zoom_keep", ""),
        ("amp_in", ""),
        ("amp_out", ""),
        ("amp_reset", ""),
        ("nudge_value_up", ""),
        ("nudge_value_down", ""),
        ("half_speed", ""), // P3-20b: Pro Tools preset only
    ] {
        assert_eq!(cubase(id), keys, "{id}");
    }
    assert_eq!(accel_of("close", "cubase").as_deref(), Some("CmdOrCtrl+W"));
    assert_eq!(accel_of("open_recent", "cubase"), None);
    assert_eq!(accel_of("shortcuts", "cubase"), None);
}

/// P3-13c: the accelerators of the Cubase preset. A key with Cmd, Control or Option becomes the accelerator
/// (the first one of the row's list), the keys without one stay page keys. The default preset is unchanged.
#[test]
fn the_cubase_preset_has_its_own_accelerators() {
    let acc = |id: &str| accel_of(id, "cubase");
    assert_eq!(acc("next_hit").as_deref(), Some("Alt+N"));
    assert_eq!(acc("prev_hit").as_deref(), Some("Alt+B"));
    assert_eq!(acc("nudge_earlier").as_deref(), Some("CmdOrCtrl+ArrowLeft"));
    assert_eq!(acc("nudge_later").as_deref(), Some("CmdOrCtrl+ArrowRight"));
    assert_eq!(acc("zoom_in").as_deref(), Some("CmdOrCtrl+]"));
    assert_eq!(acc("zoom_out").as_deref(), Some("CmdOrCtrl+["));
    assert_eq!(acc("audition").as_deref(), Some("Alt+A")); // #158: the first key of the row's Cubase list with a modifier
    assert_eq!(acc("marker_next").as_deref(), Some("Alt+M")); // #158: the rows without a Cubase list keep their default key
    assert_eq!(acc("marker_prev").as_deref(), Some("Alt+Shift+M"));
    // N, B, H, G, M, S, Shift+F, numpad 0, numpad . and Return have no modifier: page keys
    for id in [
        "next_hit_all",
        "prev_hit_all",
        "mute_track",
        "solo_track",
        "zoom_project",
        "stop_return",
        "go_start",
    ] {
        assert_eq!(acc(id), None, "{id}");
    }
    // the rows without a Cubase list keep their default key
    for id in [
        "save",
        "undo",
        "go_end",
        "inspector",
        "zoom_30s",
        "marker_next",
        "marker_prev",
    ] {
        assert_eq!(acc(id), accel_of(id, "default"), "{id}");
    }
    // the default preset has none of the Cubase keys
    assert_eq!(accel_of("next_hit", "default"), None);
    assert_eq!(accel_of("nudge_earlier", "default"), None);
    assert_eq!(accel_of("audition", "default"), None); // #158: A and Shift+A are page keys in the default preset
    assert_eq!(accel_of("next_hit_all", "default").as_deref(), Some("Ctrl+Tab"));
    assert_eq!(accel_of("zoom_project", "default").as_deref(), Some("Ctrl+CmdOrCtrl+["));
    // an unknown name is the default preset; a row without a menu item has no accelerator
    assert_eq!(preset_or_default("cubase"), "cubase");
    assert_eq!(preset_or_default("Cubase "), "default");
    assert_eq!(preset_or_default(""), "default");
    assert_eq!(accel_of("deselect", "cubase"), None);
    assert_eq!(accel_of("nudge_earlier_10", "cubase"), None);
    assert_eq!(accel_of("no_such_item", "cubase"), None);
    assert_eq!(current_preset(), "default");
}

/// `KeyCommand::menu_key_in`: the first key of a Cubase list with Cmd, Control or Option; a row without a list or
/// the default preset gives the `menu_key`; a row without a menu item gives none (Shift alone is no modifier).
#[test]
fn the_menu_key_of_a_row_in_a_preset() {
    let row = |menu: Option<&str>, menu_key: Option<&str>, cubase: Option<&[&str]>| KeyCommand {
        id: "x".into(),
        menu: menu.map(Into::into),
        title: "X".into(),
        menu_key: menu_key.map(Into::into),
        page_keys: vec!["Q".into()],
        cubase: cubase.map(|l| l.iter().map(std::string::ToString::to_string).collect()),
    };
    let r = row(Some("x"), Some("Cmd+K"), Some(&["Shift+F", "Option+N", "Cmd+L"]));
    assert_eq!(r.menu_key_in("cubase").as_deref(), Some("Option+N"));
    assert_eq!(r.menu_key_in("default").as_deref(), Some("Cmd+K"));
    assert_eq!(
        row(Some("x"), Some("Cmd+K"), None).menu_key_in("cubase").as_deref(),
        Some("Cmd+K")
    );
    assert_eq!(
        row(Some("x"), Some("Cmd+K"), Some(&["H", "Shift+F"])).menu_key_in("cubase"),
        None
    );
    assert_eq!(row(None, None, Some(&["Cmd+L"])).menu_key_in("cubase"), None);
    assert_eq!(row(None, None, None).menu_key_in("default"), None);
}

/// P3-13b done criterion, part 1: no key is in two commands (default and Cubase preset), every key is in the
/// table's notation, every menu key has Cmd, Control or Option.
#[test]
fn the_key_table_parses_and_no_key_is_in_two_commands() {
    let rows = parse_keymap(KEYMAP_JS).expect("static/keymap.js");
    assert_eq!(rows, keymap());
    let problems = keymap_problems(&rows);
    assert!(problems.is_empty(), "static/keymap.js: {problems:#?}");
    assert_eq!(rows.len(), 115); // 107 menu items (#169: Show Overview, Add Current, Organize) (#157: Other Mics Under Hit; #158: Set Marker at Selection, Next Marker, Previous Marker, Zoom Toggle, Audition Hit, Select Previous Track, Select Next Track; #150: Next and Previous Doubtful Hit; P4-wf10: Suspend Follow when Editing; P4-31: Export Again; P4-40f: Velocity Lane; P4-40g: Markers Ruler; P4-40a: Undo History) (P4-39a: Remove Track; P4-39d: Set Bar 1 at Playhead; #468: Set Bar 1 at Audio Start) (P4-36: Forget This) (P4-30: three fewer) (P4-8: six more; P4-6a: four more) (#436: Stop Returns to Start) (#149: Bars|Beats, Timecode) (#175: two more, Tab Moves To) and 15 keys without a menu item (P4-40g: Set Marker at Selection, Next Marker, Previous Marker; P4-40d: Audition Hit) (P3-20a: two more, Nudge Value Up and Down; P3-20b: Half-Speed Play) (#175: two more, Active Hits Only and All Found Hits)
}

/// #207, owner rule of 2026-10-21: no key of the table is a macOS standard shortcut, in either preset, not
/// even as an extra key. The list holds the gate's shortcuts: Spotlight (Cmd+Space), the input source
/// (Control+Space), Mission Control (Control+arrows), app switching (Cmd+Tab, Cmd+Backtick), the screenshots
/// (Shift+Cmd+3/4/5), hide and minimize (Cmd+H, Cmd+M) and Force Quit (Cmd+Option+Escape). Cmd+Q and Cmd+,
/// are the macOS standard Quit and Settings commands of the app itself and stay on them.
#[test]
fn no_key_is_a_macos_standard_shortcut() {
    const RESERVED: [&str; 14] = [
        "Cmd+Space",
        "Control+Space",
        "Control+Up",
        "Control+Down",
        "Control+Left",
        "Control+Right",
        "Cmd+Tab",
        "Cmd+`",
        "Shift+Cmd+3",
        "Shift+Cmd+4",
        "Shift+Cmd+5",
        "Cmd+H",
        "Cmd+M",
        "Cmd+Option+Escape",
    ];
    for preset in ["default", "cubase"] {
        for c in keymap() {
            for k in c.keys(preset) {
                if RESERVED.contains(&k.as_str()) {
                    // Cmd+Q on Quit and Cmd+, on Settings stay: the macOS standard keys for exactly those commands.
                    let ok = matches!((k.as_str(), c.id.as_str()), ("Cmd+Q", "quit") | ("Cmd+,", "settings"));
                    assert!(ok, "{preset}: {k} on {} is a macOS standard shortcut", c.id);
                }
            }
        }
    }
}

/// P3-13b done criterion, part 2: every menu item has its row (same id, same label), every row with a menu id
/// is an item of the menu, and the accelerator that `build` gives each item is the row's `menu_key`. The check
/// decodes both sides by itself (modifier set and key), so a menu key that comes from anywhere but the table,
/// or a wrong conversion, fails here.
#[test]
fn every_menu_accelerator_is_the_tables() {
    fn decode_tauri(a: &str) -> (Vec<&'static str>, String) {
        let mut parts: Vec<&str> = a.split('+').collect();
        let key = parts.pop().unwrap();
        let mut mods: Vec<&'static str> = parts
            .iter()
            .map(|m| match *m {
                "CmdOrCtrl" => "Cmd",
                "Ctrl" => "Control",
                "Alt" => "Option",
                "Shift" => "Shift",
                other => panic!("{a}: modifier {other}"),
            })
            .collect();
        mods.sort_unstable();
        let key = match key {
            "Enter" => "Return".to_string(),
            "ArrowLeft" => "Left".to_string(),
            "ArrowRight" => "Right".to_string(),
            "ArrowUp" => "Up".to_string(),
            "ArrowDown" => "Down".to_string(),
            k => k.to_string(),
        };
        (mods, key)
    }
    fn decode_table(k: &str) -> (Vec<&'static str>, String) {
        let (mods, key) = parse_key(k).unwrap_or_else(|| panic!("{k}: not in the notation"));
        let mut mods: Vec<&'static str> = mods
            .iter()
            .map(|m| *KEY_MODS.iter().find(|x| *x == m).unwrap())
            .collect();
        mods.sort_unstable();
        (mods, key.to_string())
    }
    let mut wrong = Vec::new();
    for it in SPEC {
        let Some(row) = key_row(it.id) else {
            wrong.push(format!("{}: in the menu, not in static/keymap.js", it.id));
            continue;
        };
        if row.title != it.label {
            wrong.push(format!("{}: label \"{}\", table \"{}\"", it.id, it.label, row.title));
        }
        for preset in PRESETS {
            let menu = accel_of(it.id, preset).map(|a| decode_tauri(&a));
            // decoded here, not by `menu_key_in`: the first key of the Cubase list with a modifier other than Shift
            let key = match (&row.cubase, preset) {
                (Some(list), "cubase") => list
                    .iter()
                    .find(|k| decode_table(k).0.iter().any(|m| *m != "Shift"))
                    .map(String::as_str),
                _ => row.menu_key.as_deref(),
            };
            let table = key.map(decode_table);
            if menu != table {
                wrong.push(format!(
                    "{preset} {}: the menu has {menu:?}, the table {table:?}",
                    it.id
                ));
            }
        }
    }
    for row in keymap().iter().filter(|r| r.menu.is_some()) {
        let id = row.menu.as_deref().unwrap();
        if !SPEC.iter().any(|i| i.id == id) {
            wrong.push(format!("{id}: in static/keymap.js, not in the menu"));
        }
    }
    assert!(wrong.is_empty(), "the menu and static/keymap.js differ: {wrong:#?}");
}

#[test]
fn a_planted_duplicate_and_a_bad_menu_key_are_found() {
    let mut rows = parse_keymap(KEYMAP_JS).unwrap();
    let follow = rows.iter().position(|r| r.id == "follow_toggle").unwrap();
    rows[follow].page_keys.push("J".into()); // J is Snap Playhead to Hits
    let p = keymap_problems(&rows);
    assert_eq!(
        p,
        [
            "default: key J is in two commands: follow_toggle and snap",
            "cubase: key J is in two commands: follow_toggle and snap"
        ]
    );
    let mut rows = parse_keymap(KEYMAP_JS).unwrap();
    rows[follow].menu_key = Some("F".into());
    rows[0].menu_key = Some("Cmd+Q".into());
    rows[1].page_keys = vec!["Cmd+Shift+X".into(), "Shift+=".into()];
    rows[1].id = "about".into();
    let p = keymap_problems(&rows);
    for want in [
        "about: the id is twice in the table",
        "follow_toggle: menu key F has no Cmd, Control or Option",
        "default: key Cmd+Q is in two commands: about and quit",
        "about: key \"Cmd+Shift+X\" is not in the table's notation",
        "about: key \"Shift+=\" is not in the table's notation",
    ] {
        assert!(p.iter().any(|x| x == want), "{want} not in {p:#?}");
    }
    let mut rows = parse_keymap(KEYMAP_JS).unwrap();
    rows[0].menu = None;
    rows[0].menu_key = Some("Cmd+K".into());
    assert_eq!(keymap_problems(&rows), ["about: menu key Cmd+K without a menu item"]);
    rows[0].menu_key = Some("Cmd+K".into());
    rows[0].page_keys = vec!["Cmd+K".into()];
    assert!(keymap_problems(&rows).contains(&"default: key Cmd+K is twice in about".to_string()));
}

#[test]
fn key_notation_and_tauri_conversion() {
    assert_eq!(parse_key("Shift+Cmd+S"), Some((vec!["Shift", "Cmd"], "S")));
    assert_eq!(
        parse_key("Control+Option+Tab"),
        Some((vec!["Control", "Option"], "Tab"))
    );
    assert_eq!(parse_key("Plus"), Some((vec![], "Plus")));
    assert_eq!(parse_key("["), Some((vec![], "[")));
    for bad in [
        "Cmd+Shift+S",
        "Shift+Shift+S",
        "Hyper+S",
        "s",
        "Cmd+",
        "",
        "Shift+[",
        "F13",
        "Cmd+Cmd+S",
    ] {
        assert_eq!(parse_key(bad), None, "{bad}");
    }
    assert!(has_menu_modifier("Option+Return") && has_menu_modifier("Control+1") && has_menu_modifier("Cmd+Y"));
    assert!(!has_menu_modifier("Shift+M") && !has_menu_modifier("J") && !has_menu_modifier("bad key"));
    assert_eq!(tauri_accel("Control+Cmd+[").as_deref(), Some("Ctrl+CmdOrCtrl+["));
    assert_eq!(tauri_accel("Shift+Cmd+Z").as_deref(), Some("CmdOrCtrl+Shift+Z"));
    assert_eq!(tauri_accel("Control+Option+Tab").as_deref(), Some("Ctrl+Alt+Tab"));
    assert_eq!(tauri_accel("Option+Return").as_deref(), Some("Alt+Enter"));
    assert_eq!(tauri_accel("Cmd+Left").as_deref(), Some("CmdOrCtrl+ArrowLeft"));
    assert_eq!(tauri_accel("NumpadMinus").as_deref(), Some("NumpadSubtract"));
    assert_eq!(tauri_accel("NumpadPlus").as_deref(), Some("NumpadAdd"));
    assert_eq!(tauri_accel("NumpadPeriod").as_deref(), Some("NumpadDecimal"));
    assert_eq!(tauri_accel("Right").as_deref(), Some("ArrowRight"));
    assert_eq!(
        [tauri_accel("Up"), tauri_accel("Down")],
        [Some("ArrowUp".into()), Some("ArrowDown".into())]
    );
    assert_eq!(tauri_accel("Cmd+Shift+S"), None);
}

#[test]
fn a_broken_table_is_refused_with_the_reason() {
    let wrap = |rows: &str| format!("x = /*KEYMAP-BEGIN*/{rows}/*KEYMAP-END*/;");
    assert!(parse_keymap("no markers").unwrap_err().contains("KEYMAP-BEGIN"));
    assert!(parse_keymap("/*KEYMAP-BEGIN*/[]").unwrap_err().contains("KEYMAP-END"));
    assert!(parse_keymap(&wrap("[1,]")).unwrap_err().contains("strict JSON"));
    assert!(parse_keymap(&wrap("{}")).unwrap_err().contains("not a list"));
    let row = |extra: &str| {
        wrap(&format!(
            r#"[{{"id": "a", "menu": null, "title": "A", "menu_key": null, "page_keys": [], "cubase": null{extra}}}]"#
        ))
    };
    assert_eq!(parse_keymap(&row("")).unwrap()[0].id, "a");
    assert!(
        parse_keymap(&wrap(r#"[{"title": "A", "page_keys": [], "cubase": null}]"#))
            .unwrap_err()
            .contains("no id")
    );
    assert!(
        parse_keymap(&wrap(r#"[{"id": "a", "page_keys": [], "cubase": null}]"#))
            .unwrap_err()
            .contains("no title")
    );
    assert!(
        parse_keymap(&wrap(r#"[{"id": 1}]"#))
            .unwrap_err()
            .contains("not a string")
    );
    assert!(
        parse_keymap(&wrap(
            r#"[{"id": "a", "title": "A", "page_keys": "J", "cubase": null}]"#
        ))
        .unwrap_err()
        .contains("page_keys")
    );
    assert!(
        parse_keymap(&wrap(r#"[{"id": "a", "title": "A", "page_keys": []}]"#))
            .unwrap_err()
            .contains("no \"cubase\"")
    );
    let c = &parse_keymap(&wrap(
        r#"[{"id": "a", "title": "A", "menu_key": "Cmd+A", "page_keys": ["A"], "cubase": ["B"]}]"#,
    ))
    .unwrap()[0];
    assert_eq!(
        (c.keys("default"), c.keys("cubase")),
        (vec!["Cmd+A".to_string(), "A".into()], vec!["B".to_string()])
    );
}

/// P3-18: the Follow Hits Of entries from the page's state; the ids of the entries are known to `dispatch`.
/// #169: View > Show Overview follows the page's `overview` state.
#[test]
fn show_overview_follows_the_page_state() {
    assert_eq!(check_states(&json!({"overview": false})), [("overview", false)]);
    assert_eq!(check_states(&json!({"overview": true})), [("overview", true)]);
    assert!(check_states(&json!({"overview": 1})).is_empty());
}

/// #169: the own zoom presets are menu ids that carry their index; the first five have Option+1 to Option+5.
#[test]
fn own_zoom_presets_are_menu_ids_and_the_first_five_have_option_keys() {
    assert!(is_known("zoom_own:0") && is_known("zoom_own:7"));
    assert!(!is_known("zoom_own:x"));
    assert_eq!(tauri_accel("Option+1").as_deref(), Some("Alt+1"));
    assert_eq!(tauri_accel("Option+5").as_deref(), Some("Alt+5"));
    assert_eq!(
        zoom_own_names(&json!({"zoomOwn": ["Snare hit", "Kick"]})),
        ["Snare hit", "Kick"]
    );
    assert!(zoom_own_names(&json!({"zoomOwn": "Kick"})).is_empty());
    assert!(zoom_own_names(&Value::Null).is_empty());
}

#[test]
fn follow_entries_list_the_other_tracks_with_the_followed_one_checked() {
    let st = json!({"tracks": ["k in", "k sub", "s top"], "selected": "k sub", "follows": "k in"});
    let e = follow_entries(&st);
    assert_eq!(
        e,
        vec![
            FollowEntry {
                name: "k in".into(),
                current: true
            },
            FollowEntry {
                name: "s top".into(),
                current: false
            },
        ]
    );
    let none = follow_entries(&json!({"tracks": ["k in", "k sub"], "selected": "k in", "follows": null}));
    assert_eq!(none.len(), 1);
    assert!(!none[0].current);
    assert!(
        follow_entries(&json!({"tracks": ["k in"], "selected": null})).is_empty(),
        "no selected track: no entries"
    );
    assert!(
        follow_entries(&json!({"tracks": ["k in"], "selected": "k in"})).is_empty(),
        "the selected track is not its own source"
    );
    assert!(follow_entries(&Value::Null).is_empty());
    assert!(is_known("follow:3") && is_known("follow_own") && !is_known("follow"));
    assert_eq!(entry_no("follow:2", FOLLOW_PREFIX), Some(2));
}

#[test]
fn every_item_has_a_command() {
    // #293: every item works, the Learn items included (the DSP candidates carry them again)
    let later: Vec<&str> = SPEC.iter().filter(|i| !i.ready).map(|i| i.id).collect();
    assert!(later.is_empty(), "{later:?}");
    assert!(
        SPEC.iter()
            .any(|i| i.id == "learn_filter" && i.needs_project && i.kind == Kind::Check)
    );
    assert!(
        SPEC.iter().any(|i| i.id == "consolidate" && i.needs_project),
        "Consolidate needs a project"
    );
}

#[test]
fn items_that_need_no_project() {
    let free: Vec<&str> = SPEC.iter().filter(|i| !i.needs_project).map(|i| i.id).collect();
    assert_eq!(
        free,
        [
            "about",
            "settings",
            "quit",
            "new",
            "open",
            "open_recent",
            "import_old",
            "nudge_val_1smp",
            "nudge_val_10smp",
            "nudge_val_100smp",
            "nudge_val_1ms",
            "nudge_val_10ms",
            "amp_in",
            "amp_out",
            "amp_reset",
            "height_taller",
            "height_shorter",
            "height_small",
            "height_medium",
            "height_large",
            "inspector",
            "info_line",
            "overview",
            "counter_seconds",
            "counter_samples",
            "counter_barsbeats",
            "counter_timecode",
            "counter_exchange",
            "tab_active",
            "tab_all",
            "ruler_tempo",
            "ruler_meter",
            "ruler_beats",
            "ruler_markers",
            "grid",
            "velocity_lane",
            "other_mics",
            "stop_return_mode",
            "audition_tab",
            "follow_smooth",
            "follow_page",
            "follow_off",
            "follow_toggle",
            "suspend_follow",
            "click",
            "snap",
            "shortcuts"
        ]
    );
}

#[test]
fn check_items_are_snap_click_info_line_and_the_radio_modes() {
    let checks: Vec<&str> = SPEC.iter().filter(|i| i.kind == Kind::Check).map(|i| i.id).collect();
    assert_eq!(
        checks,
        [
            "nudge_val_1smp",
            "nudge_val_10smp",
            "nudge_val_100smp",
            "nudge_val_1ms",
            "nudge_val_10ms",
            "height_small",
            "height_medium",
            "height_large",
            "inspector",
            "info_line",
            "overview",
            "counter_seconds",
            "counter_samples",
            "counter_barsbeats",
            "counter_timecode",
            "tab_active",
            "tab_all",
            "ruler_tempo",
            "ruler_meter",
            "ruler_beats",
            "ruler_markers",
            "grid",
            "velocity_lane",
            "other_mics",
            "stop_return_mode",
            "audition_tab",
            "follow_smooth",
            "follow_page",
            "follow_off",
            "suspend_follow",
            "click",
            "snap",
            "learn_filter"
        ]
    );
    assert!(
        COUNTER_MODES
            .iter()
            .all(|(_, id)| SPEC.iter().any(|i| i.id == *id && i.kind == Kind::Check))
    );
    assert!(
        FOLLOW_MODES
            .iter()
            .all(|(_, id)| SPEC.iter().any(|i| i.id == *id && i.kind == Kind::Check))
    );
    assert!(
        HEIGHT_MODES
            .iter()
            .all(|(_, id)| SPEC.iter().any(|i| i.id == *id && i.kind == Kind::Check))
    );
    assert!(
        NUDGE_MODES
            .iter()
            .all(|(_, id)| SPEC.iter().any(|i| i.id == *id && i.kind == Kind::Check))
    );
}

#[test]
fn known_ids_are_the_items_and_the_numbered_entries() {
    assert!(is_known("save") && is_known("recent:0") && is_known("recent:9") && is_known("project:0"));
    assert!(is_known("snap") && is_known("shortcuts") && is_known("follow_off"));
    // #441: the Clear Recent item of the Open Recent submenu
    assert!(is_known(RECENT_CLEAR_ID) && RECENT_CLEAR_ID == "recent:clear");
    assert!(
        entry_no(RECENT_CLEAR_ID, RECENT_PREFIX).is_none(),
        "not a numbered entry"
    );
    assert!(!is_known("recent:none") && !is_known("recent:") && !is_known("project:none"));
    assert!(!is_known("copy") && !is_known("") && !is_known("project") && !is_known("window"));
}

#[test]
fn entry_numbers() {
    assert_eq!(entry_no("recent:3", RECENT_PREFIX), Some(3));
    assert_eq!(entry_no("project:12", PROJECT_PREFIX), Some(12));
    assert_eq!(entry_no("project:x", PROJECT_PREFIX), None);
    assert_eq!(entry_no("recent:1", PROJECT_PREFIX), None);
}

#[test]
fn check_states_follow_the_page_state() {
    let s = check_states(&json!({"snap": false, "follow": "page", "click": true, "inspector": false, "current": "a"}));
    assert_eq!(
        s,
        [
            ("snap", false),
            ("click", true),
            ("inspector", false),
            ("follow_smooth", false),
            ("follow_page", true),
            ("follow_off", false)
        ]
    );
    // P4-8: the three map rulers and the grid
    assert_eq!(
        check_states(
            &json!({"tempoRuler": true, "meterRuler": false, "beatsRuler": true, "grid": false, "velocityLane": true, "otherMics": true})
        ),
        [
            ("ruler_tempo", true),
            ("ruler_meter", false),
            ("ruler_beats", true),
            ("grid", false),
            ("velocity_lane", true),
            ("other_mics", true)
        ]
    );
    assert!(check_states(&json!({"grid": "yes"})).is_empty());
    // P4-6c: the Learn filter of the selected track
    assert_eq!(
        check_states(&json!({"learnFilter": true})),
        vec![("learn_filter", true)]
    );
    assert!(check_states(&json!({"learnFilter": "on"})).is_empty());
    assert!(check_states(&json!({"follow": "sideways"})).is_empty());
    assert!(check_states(&json!({"counter": "frames"})).is_empty());
    assert!(check_states(&json!({"trackHeight": "huge"})).is_empty());
    assert_eq!(
        check_states(&json!({"trackHeight": "large"})),
        [
            ("height_small", false),
            ("height_medium", false),
            ("height_large", true)
        ]
    );
    let c = check_states(&json!({"infoLine": false, "counter": "samples"}));
    assert_eq!(
        c,
        [
            ("info_line", false),
            ("counter_seconds", false),
            ("counter_samples", true),
            ("counter_barsbeats", false),
            ("counter_timecode", false)
        ]
    );
    // P3-20a: exactly one Nudge Value item is checked; an unknown value leaves the items alone
    assert!(check_states(&json!({"nudgeValue": "1 beat"})).is_empty());
    assert_eq!(
        check_states(&json!({"nudgeValue": "1ms"})),
        [
            ("nudge_val_1smp", false),
            ("nudge_val_10smp", false),
            ("nudge_val_100smp", false),
            ("nudge_val_1ms", true),
            ("nudge_val_10ms", false)
        ]
    );
    assert!(check_states(&Value::Null).is_empty());
    assert_eq!(
        check_states(&json!({"follow": "off"}))
            .iter()
            .filter(|(_, on)| *on)
            .count(),
        1
    );
}

#[test]
fn window_entries_mark_the_current_project() {
    let list = json!([{"id": "a", "name": "Song A", "folder": "/x", "dirty": false}, {"id": "b", "name": "Song B"}, {"name": "no id"}]);
    let e = window_entries(&list, Some("b"));
    assert_eq!(e.len(), 2);
    assert_eq!(
        (e[0].sid.as_str(), e[0].name.as_str(), e[0].current),
        ("a", "Song A", false)
    );
    assert_eq!(
        (e[1].sid.as_str(), e[1].name.as_str(), e[1].current),
        ("b", "Song B", true)
    );
    assert!(window_entries(&list, None).iter().all(|w| !w.current));
    assert!(window_entries(&json!({"not": "a list"}), Some("a")).is_empty());
    assert_eq!(window_entries(&json!([{"sid": "s"}]), None)[0].name, "s");
}

#[test]
fn the_menu_state_starts_empty() {
    let s: RecentState = MenuState::default();
    assert!(lock(&s.recent).is_empty() && lock(&s.projects).is_empty());
}

#[test]
fn labels_use_plain_text_and_no_em_dash() {
    for i in SPEC {
        assert!(!i.label.contains('\u{2014}') && !i.label.is_empty());
    }
}

#[test]
fn the_undo_and_redo_titles_take_the_pages_text() {
    // P4-40a
    assert_eq!(
        step_title("Undo", "Undo Delete Hit (s top, 12.345 s)"),
        "Undo Delete Hit (s top, 12.345 s)"
    );
    assert_eq!(step_title("Undo", ""), "Undo");
    assert_eq!(step_title("Redo", "  \n "), "Redo");
    assert_eq!(step_title("Redo", "Redo  Nudge\nHit"), "Redo Nudge Hit");
    let long = format!("Undo {}", "x".repeat(200));
    let cut = step_title("Undo", &long);
    assert_eq!(cut.chars().count(), 90);
    assert!(cut.ends_with('\u{2026}') && cut.starts_with("Undo xxx"));
    // the label of an Edit item with a page text is still the keymap's label
    assert!(
        SPEC.iter()
            .any(|i| i.id == "undo_history" && i.label == "Undo History\u{2026}" && i.needs_project)
    );
}
