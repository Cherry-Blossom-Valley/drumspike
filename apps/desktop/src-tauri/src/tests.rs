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

/// F-9: one capability file per concern, each with explicit permissions. No file grants `core:default`.
#[test]
fn capabilities_grant_each_concern_its_permissions_and_nothing_more() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let mut granted: Vec<(String, Vec<String>)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            let perms = v["permissions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect();
            (v["identifier"].as_str().unwrap().to_string(), perms)
        })
        .collect();
    granted.sort();
    let expected: Vec<(String, Vec<String>)> = [
        ("default", vec!["core:event:allow-listen"]),
        ("dialogs", vec!["dialog:allow-open", "dialog:allow-save"]),
        ("drag", vec!["drag:allow-start-drag"]),
    ]
    .into_iter()
    .map(|(id, perms)| (id.to_string(), perms.into_iter().map(String::from).collect()))
    .collect();
    assert_eq!(granted, expected);
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

/// #235: the merged `Info.plist` gives the document type and the exported type the document icon, the file
/// is in the bundle resources, and the plist keeps the names and the type identifier of `tauri.conf.json`.
#[test]
fn the_document_type_has_its_own_icon_in_the_bundle() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let plist = std::fs::read_to_string(dir.join("Info.plist")).unwrap();
    for key in ["CFBundleTypeIconFile", "UTTypeIconFile"] {
        assert!(
            plist.contains(&format!("<key>{key}</key>\n\t\t\t<string>document.icns</string>")),
            "{key}"
        );
    }
    for s in ["Drum Spike project", "com.cbv.drumspike.project", "public.folder"] {
        assert!(plist.contains(s), "{s}");
    }
    let conf: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("tauri.conf.json")).unwrap()).unwrap();
    assert_eq!(
        conf["bundle"]["macOS"]["files"]["Resources/document.icns"],
        "icons/document.icns"
    );
    let icns = std::fs::read(dir.join("icons/document.icns")).unwrap();
    assert_eq!(&icns[..4], b"icns");
}

/// The command list of each build as written in `lib.rs` (paths such as `shell::shell_sync`):
/// (release, debug). The one `generate_handler![...]` list inside the `command_handler!` macro is
/// common; each `fn handler()` adds the names of its `command_handler![...]` call, and the `cfg`
/// line above it names the build.
pub(crate) fn handler_lists(lib: &str) -> (Vec<String>, Vec<String>) {
    fn names(list: &str) -> Vec<String> {
        list.split(',')
            .map(|s| s.trim().to_string())
            // `$($extra)*` is the macro argument, not a command
            .filter(|s| !s.is_empty() && !s.starts_with('$'))
            .collect()
    }
    fn list_after<'a>(text: &'a str, open: &str) -> &'a str {
        let start = text.find(open).unwrap_or_else(|| panic!("no {open}")) + open.len();
        &text[start..start + text[start..].find(']').unwrap()]
    }
    assert_eq!(lib.matches("generate_handler![").count(), 1, "one handler list");
    let common = names(list_after(lib, "generate_handler!["));
    let (mut release, mut debug) = (None, None);
    let mut rest = lib;
    while let Some(i) = rest.find("fn handler()") {
        let cfg = rest[..i].trim_end().rsplit('\n').next().unwrap().trim();
        rest = &rest[i + "fn handler()".len()..];
        let mut all = common.clone();
        all.extend(names(list_after(rest, "command_handler![")));
        let slot = match cfg {
            "#[cfg(not(debug_assertions))]" => &mut release,
            "#[cfg(debug_assertions)]" => &mut debug,
            other => panic!("fn handler() under {other:?}"),
        };
        assert!(slot.replace(all).is_none(), "two handlers for one build");
    }
    (release.expect("a release handler"), debug.expect("a debug handler"))
}

/// The parser reads the common list and the extra arguments of each build.
#[test]
fn handler_lists_reads_the_common_list_and_the_extras() {
    let lib = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs")).unwrap();
    let (release, debug) = handler_lists(&lib);
    assert!(release.iter().any(|n| n == "app_info"));
    assert!(release.iter().any(|n| n == "shell::shell_sync"));
    assert!(!release.iter().any(|n| n == "selftest_clock"));
    assert!(debug.iter().any(|n| n == "selftest_clock"));
    assert_eq!(
        debug.len(),
        release.len() + 12,
        "the debug build adds the 12 self-test commands"
    );
    assert!(release.iter().all(|n| debug.contains(n)));
}

/// Every `#[tauri::command]` of `shell.rs` is in the handler of the debug build, and (the self-test ones
/// excepted) in the handler of the release build: a command that is not listed fails only in the app.
#[test]
fn every_shell_command_is_in_the_handler_lists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let shell = std::fs::read_to_string(root.join("shell.rs")).unwrap();
    let lib = std::fs::read_to_string(root.join("lib.rs")).unwrap();
    let (release, debug) = handler_lists(&lib);
    let mut n = 0;
    for chunk in shell.split("#[tauri::command").skip(1) {
        let name = chunk.split("fn ").nth(1).unwrap().split('(').next().unwrap(); // sync and async commands
        let path = format!("shell::{name}");
        assert!(debug.contains(&path), "{name} is not in the debug handler");
        if !name.starts_with("selftest_") {
            assert!(release.contains(&path), "{name} is not in the release handler");
        } else {
            assert!(!release.contains(&path), "{name} is in the release handler");
        }
        n += 1;
    }
    assert_eq!(n, 21); // #300: shell_typing; #232: export_compensate_set; #433: export_drag_take; P4-31: export_last_get and export_last_set; P4-30: export_dest_set, export_drag_prepare and reveal_path; P4-40g: export_markers_set; #206: selftest_ns_menu, selftest_menu_key, selftest_menu_log and selftest_menu_probe
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
    let svc = Service::new(ServiceConfig::default());
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
