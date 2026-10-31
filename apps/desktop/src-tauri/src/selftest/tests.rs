use super::*;

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn parse_args_reads_scenario_and_out() {
    assert_eq!(parse_args(&s(&["app"])).unwrap(), None);
    let r = parse_args(&s(&["app", "--self-test", "slice", "--out", "/x/r.json"]))
        .unwrap()
        .unwrap();
    assert_eq!((r.scenario.as_str(), r.out), ("slice", PathBuf::from("/x/r.json")));
    assert!(parse_args(&s(&["--self-test"])).is_err());
    assert!(parse_args(&s(&["--self-test", "slice"])).is_err());
    assert!(parse_args(&s(&["--self-test", "--out", "x"])).is_err());
    assert!(parse_args(&s(&["--self-test", "slice", "--out"])).is_err());
}

#[test]
fn scenario_path_maps_names_and_keeps_paths() {
    assert!(scenario_path("slice").ends_with("selftest/slice.json"));
    assert_eq!(scenario_path("/a/b.json"), PathBuf::from("/a/b.json"));
    assert_eq!(scenario_path("b.json"), PathBuf::from("b.json"));
}

#[test]
fn resolve_env_replaces_dollar_strings_everywhere() {
    unsafe {
        std::env::set_var("DS_SELFTEST_UNIT", "/p");
    }
    let mut v = json!({"a": "$DS_SELFTEST_UNIT", "b": ["x", "$DS_SELFTEST_UNIT"], "c": {"d": "plain", "e": 3}});
    resolve_env(&mut v).unwrap();
    assert_eq!(v, json!({"a": "/p", "b": ["x", "/p"], "c": {"d": "plain", "e": 3}}));
    let mut bad = json!({"a": "$DS_SELFTEST_SURELY_UNSET"});
    assert!(resolve_env(&mut bad).unwrap_err().contains("DS_SELFTEST_SURELY_UNSET"));
}

#[test]
fn the_slice_scenario_loads_and_has_the_fields_the_driver_reads() {
    unsafe {
        std::env::set_var("DRUMSPIKE_SELFTEST_PROJECT", "/tmp/p");
    }
    let v = load_scenario(&Request {
        scenario: "slice".into(),
        out: PathBuf::from("x"),
    })
    .unwrap();
    assert_eq!(v["name"], "slice");
    assert_eq!(v["project"], "/tmp/p");
    assert_eq!(v["expect"]["tracks"], 10);
    assert!(v["expect"]["detector"].is_string() && v["timeout_s"].is_number());
    let missing = load_scenario(&Request {
        scenario: "/nonexistent/x.json".into(),
        out: PathBuf::from("x"),
    });
    assert!(missing.unwrap_err().contains("cannot read scenario"));
}

#[test]
fn init_script_sets_the_scenario_before_the_driver() {
    let js = init_script(&json!({"name": "t"}));
    assert!(js.contains("window.__SELFTEST__ = {\"name\":\"t\"}"));
    assert!(js.contains("selftest_report") && js.find("__SELFTEST__").unwrap() < js.find("selftest_report").unwrap());
    assert!(js.starts_with("window.addEventListener('load'"));
    // P3-5b: the play step is defined before the driver calls it
    let (play, driver) = (
        js.find("window.__selftestPlay = ").unwrap(),
        js.find("window.__selftestPlay({ report").unwrap(),
    );
    assert!(play < driver);
    // P3-6b: so is the memory step
    let (mem, call) = (
        js.find("window.__selftestMemory = ").unwrap(),
        js.find("window.__selftestMemory({ report").unwrap(),
    );
    assert!(mem < call);
}

#[test]
fn the_memory_scenario_loads_with_the_limits_of_the_card() {
    unsafe {
        std::env::set_var("DRUMSPIKE_SELFTEST_PROJECT", "/tmp/p");
    }
    let v = load_scenario(&Request {
        scenario: "memory".into(),
        out: PathBuf::from("x"),
    })
    .unwrap();
    assert_eq!(v["steps"], json!(["memory"]));
    assert_eq!(v["expect"]["tracks"], 30);
    assert_eq!(v["memory"]["target_web_content_mb"], 400);
    assert!(v["memory"]["max_web_content_mb"].as_f64().unwrap() > 400.0);
    assert!(v["memory"]["max_open_to_drawn_ms"].as_f64().unwrap() <= 1000.0);
    assert_eq!(v["memory"]["play_seconds"], 60);
}

#[test]
fn the_play_step_refuses_an_audible_run() {
    // the guard that keeps a device run silent is in the script the page runs
    assert!(PLAY.contains("st.master === 0") && PLAY.contains("DRUMSPIKE_MASTER_GAIN=0"));
    assert!(PLAY.contains("allow_audible"));
}

#[test]
fn write_report_adds_the_host_and_returns_the_verdict() {
    let dir = std::env::temp_dir().join(format!("ds_selftest_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("r.json");
    assert!(write_report(&out, json!({"pass": true, "steps": []}), 1234.567).unwrap());
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(v["host"]["profile"], "debug");
    assert_eq!(v["process_ms"], 1234.57);
    assert!(
        !write_report(&out, json!({"steps": []}), 0.0).unwrap(),
        "no pass field fails"
    );
    assert!(write_report(&dir.join("no/such/dir.json"), json!({}), 0.0).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn failure_report_fails_and_names_the_reason() {
    let v = failure_report("slice", "boom");
    assert_eq!(
        (v["pass"].as_bool(), v["errors"][0].as_str()),
        (Some(false), Some("boom"))
    );
    assert!(host_info()["core_version"].is_string());
}

#[test]
fn driver_calls_the_two_commands_and_every_step_of_the_slice() {
    for needle in [
        "selftest_clock",
        "selftest_report",
        "step(\"open\"",
        "step(\"lanes\"",
        "step(\"frames\"",
        "step(\"analyze\"",
        "step(\"edit\"",
        "step(\"export\"",
    ] {
        assert!(DRIVER.contains(needle), "driver.js lacks {needle}");
    }
}

#[test]
fn the_shell_scenario_loads_with_its_own_driver_and_the_fields_the_driver_reads() {
    for (k, v) in [
        ("DRUMSPIKE_SELFTEST_PROJECT", "/tmp/p"),
        ("DRUMSPIKE_SELFTEST_DIR", "/tmp/w"),
        ("DRUMSPIKE_SELFTEST_OPEN_DOC", ""),
    ] {
        unsafe {
            std::env::set_var(k, v);
        }
    }
    let v = load_scenario(&Request {
        scenario: "shell".into(),
        out: PathBuf::from("x"),
    })
    .unwrap();
    assert_eq!(
        (v["name"].as_str(), v["driver"].as_str()),
        (Some("shell"), Some("shell"))
    );
    assert_eq!(
        (v["project"].as_str(), v["work"].as_str(), v["open_doc"].as_str()),
        (Some("/tmp/p"), Some("/tmp/w"), Some(""))
    );
    // The count is menu::SPEC, not a literal: a new custom item changes the menu and fails
    // here until shell.json follows (the driver needs menu_items == items.length == key rows).
    assert!(
        v["expect"]["tracks"].is_number()
            && v["expect"]["menu_items"] == crate::menu::SPEC.len() as u64
            && v["timeout_s"].is_number()
    );
    assert!(driver_for(&v) == SHELL_DRIVER && SHELL_DRIVER != DRIVER);
    assert!(driver_for(&json!({"name": "slice"})) == DRIVER);
    assert!(init_script(&v).contains("step(\"every_item_triggered\""));
}

#[test]
fn shell_driver_runs_every_step_of_the_card_and_reports_through_the_quit_path() {
    for needle in [
        "step(\"cold_start\"",
        "step(\"menu_table\"",
        "step(\"menu_key_preset\"",
        "step(\"finder_open\"",
        "step(\"menu_open\"",
        "step(\"menu_check_items\"",
        "step(\"menu_new_and_recent\"",
        "step(\"recent_clear\"", // #441: the Clear Recent item empties the list and the menu
        "step(\"menu_window\"",
        "step(\"menu_new_items\"",
        "step(\"menu_edit_undo_redo_recompute\"",
        "step(\"menu_save\"",
        "step(\"menu_export\"",
        "step(\"menu_save_as\"",
        "step(\"menu_consolidate\"",
        "step(\"menu_save_as_copy\"",
        "step(\"menu_add_tracks\"",
        "step(\"menu_follow\"",
        "step(\"menu_about\"",
        "step(\"menu_close\"",
        "step(\"finder_paths_event\"",
        "step(\"quit_prompts\"",
        "selftest_close_window",
        "selftest_open_paths",
        "selftest_menu_report",
        "via_quit: true",
    ] {
        assert!(SHELL_DRIVER.contains(needle), "shell_driver.js lacks {needle}");
    }
}

#[test]
fn parse_helpers_keeps_the_webkit_helpers_only() {
    let ps = "  101  2048 /usr/bin/foo\n  202 340112 /System/Library/Frameworks/WebKit.framework/Versions/A/XPCServices/com.apple.WebKit.WebContent.xpc/Contents/MacOS/com.apple.WebKit.WebContent\n  303 9000 /x/com.apple.WebKit.GPU\nbad line\n";
    let h = parse_helpers(ps);
    assert_eq!(h.len(), 2);
    assert_eq!((h[0].pid, h[0].rss_kb, h[0].kind.as_str()), (202, 340112, "WebContent"));
    assert_eq!(h[1].kind, "GPU");
}

#[test]
fn memory_names_the_new_helpers_and_footprints_skip_nothing_when_empty() {
    snapshot_helpers();
    let m = memory();
    assert!(m["processes"].is_array() && m["web_content_mb"].is_number());
    assert!(footprints(&[]).is_empty());
    let me = std::process::id();
    // `footprint --json` is a macOS tool; where it is absent there is no footprint to read.
    if !std::path::Path::new("/usr/bin/footprint").exists() {
        return;
    }
    assert!(footprints(&[me]).get(&me).is_some_and(|b| *b > 0));
}

#[test]
fn the_tracks_scenario_loads_with_its_own_driver_and_counts_four_actions() {
    // #194: the variables come from the argument, not the process env — this test ran green
    // only when the shell test had left DRUMSPIKE_SELFTEST_DIR set first.
    let v = load_scenario_from(
        &Request {
            scenario: "tracks".into(),
            out: PathBuf::from("x"),
        },
        &[("DRUMSPIKE_SELFTEST_DIR", "/tmp/w"), ("DRUMSPIKE_SELFTEST_DROP", "/tmp/drop")],
    )
    .unwrap();
    assert_eq!(
        (v["name"].as_str(), v["driver"].as_str()),
        (Some("tracks"), Some("tracks"))
    );
    assert_eq!(v["drop"], "/tmp/drop");
    assert_eq!(v["work"], "/tmp/w");
    assert!(driver_for(&v) == TRACKS_DRIVER && TRACKS_DRIVER != DRIVER && TRACKS_DRIVER != SHELL_DRIVER);
    assert_eq!(
        v["expect"]["roles"].as_object().unwrap().len() as u64,
        v["expect"]["tracks"].as_u64().unwrap()
    );
    for needle in [
        "step(\"drop_to_list\"",
        "step(\"accept\"",
        "step(\"analyze\"",
        "step(\"export\"",
        "step(\"actions\"",
        "selftest_open_paths",
        "user_actions",
        "via_quit: true",
    ] {
        assert!(TRACKS_DRIVER.contains(needle), "tracks_driver.js lacks {needle}");
    }
}

#[test]
fn the_watchdog_thread_starts_without_waiting() {
    spawn_watchdog(
        &json!({"timeout_s": 3600}),
        PathBuf::from("/nonexistent/never"),
        "t".into(),
    );
}
