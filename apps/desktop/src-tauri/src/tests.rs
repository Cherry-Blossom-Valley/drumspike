use super::*;

#[test]
fn app_info_reports_version_detector_and_chain() {
    let v = app_info_value(Path::new(""), false);
    assert_eq!(v["core_version"], drumspike_core::VERSION);
    assert_eq!(v["detector"], DETECTOR);
    assert_eq!(v["chain"], "mix");
    assert_eq!(v["model"], Value::Null);
}

#[test]
fn headless_ignores_other_arguments_and_reports_app_info() {
    assert_eq!(headless(&[]), None);
    assert_eq!(headless(&["--self-test".into(), "slice".into()]), None);
    assert_eq!(headless(&["--app-info".into()]), Some(0));
    assert_eq!(headless(&["--analyze".into(), "/nonexistent/project".into()]), Some(1));
    assert_eq!(headless(&["--analyze".into()]), Some(0));
}

#[cfg(debug_assertions)]
#[test]
fn selftest_clock_counts_from_the_start_of_the_process() {
    START.get_or_init(Instant::now);
    let a = selftest_clock();
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert!(selftest_clock() > a && a >= 0.0);
}

#[test]
fn capability_allows_the_open_and_save_dialogs_and_the_file_drag_only() {
    let cap = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json")).unwrap();
    let v: Value = serde_json::from_str(&cap).unwrap();
    let perms: Vec<&str> = v["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        perms,
        [
            "core:default",
            "dialog:allow-open",
            "dialog:allow-save",
            "drag:allow-start-drag"
        ]
    );
}

#[test]
fn the_bundle_declares_the_document_type_and_project_folders() {
    let conf = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json")).unwrap();
    let v: Value = serde_json::from_str(&conf).unwrap();
    let assoc = v["bundle"]["fileAssociations"].as_array().unwrap();
    assert_eq!(assoc.len(), 2);
    assert_eq!(assoc[0]["ext"], serde_json::json!(["drumspike"]));
    assert_eq!(assoc[0]["role"], "Editor");
    assert_eq!(assoc[0]["exportedType"]["identifier"], "com.cbv.drumspike.project");
    assert_eq!(assoc[1]["contentTypes"], serde_json::json!(["public.folder"]));
    assert_eq!(open::DOC_EXT, "drumspike");
}

/// Every `#[tauri::command]` of `shell.rs` is in the handler of the debug build, and (the self-test ones
/// excepted) in the handler of the release build: a command that is not listed fails only in the app.
#[test]
fn every_shell_command_is_in_the_handler_lists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let shell = std::fs::read_to_string(root.join("shell.rs")).unwrap();
    let lib = std::fs::read_to_string(root.join("lib.rs")).unwrap();
    let release = &lib[lib.find("#[cfg(not(debug_assertions))]\nfn handler").unwrap()
        ..lib.find("/// The command handler of a debug build").unwrap()];
    let debug =
        &lib[lib.find("/// The command handler of a debug build").unwrap()..lib.find("/// With `--self-test").unwrap()];
    let mut n = 0;
    for chunk in shell.split("#[tauri::command").skip(1) {
        let name = chunk.split("fn ").nth(1).unwrap().split('(').next().unwrap(); // sync and async commands
        assert!(
            debug.contains(&format!("shell::{name}")),
            "{name} is not in the debug handler"
        );
        if !name.starts_with("selftest_") {
            assert!(
                release.contains(&format!("shell::{name}")),
                "{name} is not in the release handler"
            );
        } else {
            assert!(!release.contains(name), "{name} is in the release handler");
        }
        n += 1;
    }
    assert_eq!(n, 19); // #433: export_drag_take; P4-31: export_last_get and export_last_set; P4-30: export_dest_set, export_drag_prepare and reveal_path; P4-40g: export_markers_set; #206: selftest_ns_menu, selftest_menu_key, selftest_menu_log and selftest_menu_probe
}

/// P3-15c: the runtime has 4 workers named `ds-async`, and `blocking` runs the body on another
/// thread than the caller (the blocking pool) and returns its result.
#[test]
fn the_async_runtime_has_four_workers_and_blocking_runs_on_the_pool() {
    assert!(async_runtime_setup(), "the first call sets the runtime");
    assert!(!async_runtime_setup(), "a second call does nothing");
    let ids = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let tasks: Vec<_> = (0..12)
        .map(|_| {
            let ids = Arc::clone(&ids);
            tauri::async_runtime::spawn(async move {
                let t = std::thread::current();
                assert_eq!(t.name(), Some("ds-async"));
                ids.lock().unwrap().insert(t.id());
                std::thread::sleep(std::time::Duration::from_millis(30)); // keeps the worker busy
            })
        })
        .collect();
    tauri::async_runtime::block_on(async {
        for t in tasks {
            t.await.unwrap();
        }
    });
    let n = ids.lock().unwrap().len();
    assert!((1..=ASYNC_WORKERS).contains(&n), "{n} workers ran the tasks");
    let svc = Service::new(ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    let caller = std::thread::current().id();
    let (v, on) = tauri::async_runtime::block_on(blocking(&svc, "project_list", move |s| {
        Ok((s.project_list(ListShape::Ids), std::thread::current().id()))
    }))
    .unwrap();
    assert_eq!(v, serde_json::json!([]));
    assert_ne!(on, caller);
    let e = tauri::async_runtime::block_on(blocking(&svc, "project_get", |s| s.project_get("nope"))).unwrap_err();
    assert_eq!(e.code, 404);
}

#[test]
fn startup_probe_script_calls_the_command() {
    assert!(STARTUP_SCRIPT.contains("startup_mark"));
}
