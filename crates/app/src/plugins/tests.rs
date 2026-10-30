//! The tests of `crates/app/src/plugins.rs` (the manifest reader, the folder scan, the
//! acceptance record). Each test builds its own Detectors folder and points
//! `DRUMSPIKE_DETECTORS_DIR` at it (the env is read per call).

use super::*;
use std::sync::Mutex;

const LICENCE: &str = "MIT License\n\nPermission is hereby granted, free of charge.\n";

/// One valid plugin folder with the given manifest additions; returns the Detectors dir.
fn make_plugin(dir: &Path, id: &str, extra: Value) -> PathBuf {
    let folder = dir.join(id);
    std::fs::create_dir_all(&folder).unwrap();
    let mut m = json!({
        "id": id,
        "name": "JSON Hits",
        "version": "1.0.0",
        "protocol": 1,
        "executable": "run.py",
        "roles": ["kick", "snare"],
        "sample_rates": [44100, 48000],
        "licence": "MIT",
        "licence_file": "LICENCE",
    });
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            if v.is_null() {
                m.as_object_mut().unwrap().remove(k);
            } else {
                m[k] = v.clone();
            }
        }
    }
    std::fs::write(folder.join("manifest.json"), serde_json::to_string(&m).unwrap()).unwrap();
    std::fs::write(folder.join("run.py"), "#!/usr/bin/env python3\n").unwrap();
    std::fs::write(folder.join("LICENCE"), LICENCE).unwrap();
    dir.to_path_buf()
}

/// Tests run in parallel and share the process env: the lock keeps one test's
/// DETECTORS_DIR from being removed while another test scans its folder.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_dir(dir: &Path, f: impl FnOnce()) {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe {
        std::env::set_var(DETECTORS_DIR_ENV, dir);
    }
    f();
    unsafe {
        std::env::remove_var(DETECTORS_DIR_ENV);
    }
}

#[test]
fn a_valid_manifest_loads() {
    let dir = std::env::temp_dir().join(format!("ds_plugins_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    make_plugin(&dir, "json-hits", json!({}));
    with_dir(&dir, || {
        let i = find("json-hits").expect("installed");
        assert_eq!(i.manifest.name, "JSON Hits");
        assert_eq!(i.licence_text, LICENCE);
        assert_eq!(i.licence_sha256().len(), 64);
        assert!(i.supports_role("kick") && !i.supports_role("tom"));
        assert_eq!(i.pass_rate(48_000), Some(48_000));
        assert_eq!(i.pass_rate(96_000), Some(44_100)); // 96 -> 44.1 resample
        assert_eq!(i.pass_rate(88_200), None); // not listed, not resampleable
        let empty = serde_json::Map::new();
        assert_eq!(Installed::timeout_of(&empty), 120);
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_invalid_manifest_is_invisible() {
    let dir = std::env::temp_dir().join(format!("ds_plugins_bad_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // protocol 2, a missing executable, a wrong folder name and a bad version
    make_plugin(&dir, "proto2", json!({"protocol": 2}));
    make_plugin(&dir, "noexe", json!({"executable": "gone.sh"}));
    make_plugin(&dir, "ver", json!({"version": "1.0"}));
    std::fs::create_dir_all(dir.join("other-name")).unwrap();
    std::fs::write(
        dir.join("other-name/manifest.json"),
        r#"{"id":"other-name","name":"X","version":"1.0.0","protocol":1,"executable":"run.py","roles":["kick"],"sample_rates":[44100],"licence":"MIT","licence_file":"LICENCE"}"#,
    )
    .unwrap();
    with_dir(&dir, || {
        assert!(find("proto2").is_none());
        assert!(find("noexe").is_none());
        assert!(find("ver").is_none());
        assert!(find("other-name").is_none());
        assert!(list().is_empty());
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_manifest_fields_are_ignored_and_request_params_filters_reserved_keys() {
    let dir = std::env::temp_dir().join(format!("ds_plugins_extra_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    make_plugin(
        &dir,
        "json-hits",
        json!({"homepage": "https://example.net", "sponsors": 3}),
    );
    with_dir(&dir, || {
        let i = find("json-hits").expect("installed");
        assert_eq!(i.manifest.homepage.as_deref(), Some("https://example.net"));
        let params = serde_json::json!({"detector": "json-hits", "min_gap_ms": 20.0, "sensitivity": 0.5})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(Installed::request_params(&params), serde_json::json!({}));
        let params = serde_json::json!({"detector": "x", "timeout_s": 9, "model": "tiny", "note": 5})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            Installed::request_params(&params),
            serde_json::json!({"model": "tiny", "note": 5})
        );
        assert_eq!(Installed::timeout_of(&params), 9);
    });
    let _ = std::fs::remove_dir_all(&dir);
}
