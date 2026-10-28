//! The About window data (P3-8a). Help > About (`menu.rs`, item `about`) makes the page open its
//! About dialog (`static/shell.js`, function `about`). The page holds the plain-language lines (what
//! the app does, the non-commercial notice). This module gives it the facts of the
//! build: the version and, on request, the third-party list
//! (`THIRD_PARTY.md`, made by `apps/desktop/scripts/gen_third_party.py`). The file is a bundle
//! resource. #281 D16: main is model-free; the model notices live on the model branch.

use std::path::{Path, PathBuf};

use drumspike_app::AppError;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

/// The file name of the third-party list in the bundle resources.
pub const THIRD_PARTY_FILE: &str = "THIRD_PARTY.md";

/// The addresses behind the fixed link ids of the About dialog (#18). The page never passes an
/// address, only one of these ids, so it cannot open any other address.
pub const LINK_WEBSITE: &str = "https://drumspike.cherry-blossom-valley.com/";
pub const LINK_SOURCE: &str = "https://github.com/Cherry-Blossom-Valley/drumspike";
pub const LINK_CBV: &str = "https://cherry-blossom-valley.com/";

/// The one address of a fixed id; anything else is refused.
pub fn link_address(id: &str) -> Option<&'static str> {
    match id {
        "website" => Some(LINK_WEBSITE),
        "source" => Some(LINK_SOURCE),
        "cbv" => Some(LINK_CBV),
        _ => None,
    }
}

/// #18: open one of the fixed addresses of the About dialog by id (`open <address>` on macOS).
#[tauri::command]
pub fn open_link(id: String) -> Result<Value, AppError> {
    let addr = link_address(&id).ok_or_else(|| AppError::not_found(format!("unknown link id: {id}")))?;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(addr)
            .status()
            .map_err(|e| AppError::internal(format!("open: {e}")))?;
        Ok(Value::Null)
    }
    #[cfg(not(target_os = "macos"))]
    Err(AppError::internal("open_link is a macOS feature"))
}

/// The answer of `about_info`: `name`, `version` and `core_version` (main is model-free, #281 D16).
pub fn about_value(version: &str) -> Value {
    json!({
        "name": "Drum Spike",
        "version": version,
        "core_version": drumspike_core::VERSION,
    })
}

/// The first existing third-party list: in the bundle resources, else (a plain `cargo run`) next to
/// `src-tauri` in the repository.
pub fn third_party_path(resource_dir: Option<&Path>) -> Option<PathBuf> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(THIRD_PARTY_FILE);
    resource_dir
        .map(|d| d.join(THIRD_PARTY_FILE))
        .into_iter()
        .chain(std::iter::once(repo))
        .find(|p| p.is_file())
}

/// The text of the third-party list, or a line that says why it is missing.
pub fn third_party_text(resource_dir: Option<&Path>) -> String {
    match third_party_path(resource_dir).map(std::fs::read_to_string) {
        Some(Ok(t)) => t,
        Some(Err(e)) => format!("The third-party list cannot be read: {e}"),
        None => format!("The third-party list ({THIRD_PARTY_FILE}) is not in this build."),
    }
}

/// P3-15c: the file reads run on the blocking pool, not on the main thread.
#[tauri::command]
pub async fn about_info(app: AppHandle) -> Value {
    tauri::async_runtime::spawn_blocking(move || {
        let _span = drumspike_app::trace::command("about_info");
        about_value(&app.package_info().version.to_string())
    })
    .await
    .unwrap_or(Value::Null)
}

#[tauri::command]
pub async fn about_third_party(app: AppHandle) -> String {
    tauri::async_runtime::spawn_blocking(move || {
        let _span = drumspike_app::trace::command("about_third_party");
        third_party_text(app.path().resource_dir().ok().as_deref())
    })
    .await
    .unwrap_or_else(|e| format!("The third-party list cannot be read: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_value_has_version_and_core_version() {
        let v = about_value("9.8.7");
        assert_eq!(v["name"], "Drum Spike");
        assert_eq!(v["version"], "9.8.7");
        assert_eq!(v["core_version"], drumspike_core::VERSION);
        assert!(
            v.get("notice").is_none(),
            "the model notice is gone with the model (#281 D16)"
        );
    }

    #[test]
    fn third_party_path_prefers_the_resources_and_falls_back_to_the_repository() {
        let dir = std::env::temp_dir().join(format!("ds_about_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // no file in the resources: the repository copy
        let repo = third_party_path(Some(&dir)).unwrap();
        assert!(repo.ends_with("../THIRD_PARTY.md"));
        std::fs::write(dir.join(THIRD_PARTY_FILE), "list").unwrap();
        assert_eq!(third_party_path(Some(&dir)).unwrap(), dir.join(THIRD_PARTY_FILE));
        assert_eq!(third_party_text(Some(&dir)), "list");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn third_party_list_of_the_repository_names_the_crates() {
        let t = third_party_text(None);
        assert!(t.contains("# Third-party software in Drum Spike"));
        assert!(t.contains("| tauri |"));
        assert!(t.contains("MPL-2.0"));
    }

    #[test]
    fn the_three_link_ids_map_to_exactly_the_three_addresses() {
        assert_eq!(link_address("website"), Some("https://drumspike.cherry-blossom-valley.com/"));
        assert_eq!(link_address("source"), Some("https://github.com/Cherry-Blossom-Valley/drumspike"));
        assert_eq!(link_address("cbv"), Some("https://cherry-blossom-valley.com/"));
    }

    #[test]
    fn an_unknown_link_id_is_refused() {
        assert!(link_address("").is_none());
        assert!(link_address("Website").is_none());
        assert!(link_address("cbv ").is_none());
        assert!(link_address("https://example.com").is_none());
        assert!(link_address("../../etc/passwd").is_none());
    }

    #[test]
    fn both_commands_are_in_the_two_handler_lists() {
        let lib = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs")).unwrap();
        for name in ["about::about_info", "about::about_third_party", "about::open_link"] {
            assert_eq!(
                lib.matches(name).count(),
                2,
                "{name} must be in the release and the debug handler"
            );
        }
    }

    #[test]
    fn the_bundle_carries_the_notices_and_the_third_party_list() {
        let conf = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json")).unwrap();
        let v: Value = serde_json::from_str(&conf).unwrap();
        let res = v["bundle"]["resources"].as_object().unwrap();
        for name in ["THIRD_PARTY.md", "LICENSE", "NOTICE"] {
            assert!(res.values().any(|t| t == name), "resource {name} missing");
        }
        assert_eq!(res.len(), 3);
        assert!(!serde_json::to_string(&res).unwrap().contains(concat!("onn", "x"))); // spelled out: the source scan must stay clean
        assert_eq!(v["bundle"]["macOS"]["hardenedRuntime"], true);
        // P3-8b: the entitlements file exists, has no key (no sandbox, no exception).
        assert_eq!(v["bundle"]["macOS"]["entitlements"], "entitlements.plist");
        let plist = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("entitlements.plist")).unwrap();
        assert!(
            plist.contains("<dict/>"),
            "entitlements.plist must hold an empty dictionary"
        );
        assert_eq!(v["identifier"], "com.cbv.drumspike");
        assert!(v["bundle"]["createUpdaterArtifacts"].is_null());
    }
}
