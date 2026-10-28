use super::*;
use crate::ServiceConfig;

#[test]
fn the_import_command_list_has_no_duplicates() {
    let mut v = IMPORT_COMMANDS.to_vec();
    v.sort_unstable();
    v.dedup();
    assert_eq!(v.len(), IMPORT_COMMANDS.len());
}

#[test]
fn folder_name_follows_the_server() {
    assert_eq!(folder_name("Song: Take 2"), "Song- Take 2");
    assert_eq!(folder_name("a/\\:b"), "a-b");
    assert_eq!(folder_name("  ..x.. "), "x");
    assert_eq!(folder_name(""), "Imported project");
    assert_eq!(folder_name("..."), "Imported project");
    assert_eq!(folder_name(&"x".repeat(100)).len(), 80);
}

#[test]
fn dispatch_ignores_other_commands_and_checks_arguments() {
    let svc = Service::new(ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    assert!(dispatch(&svc, "project_open", &json!({})).is_none());
    assert_eq!(
        dispatch(&svc, "legacy_list", &json!({})).unwrap().unwrap_err().code,
        400
    );
    assert_eq!(
        dispatch(&svc, "project_import", &json!({})).unwrap().unwrap_err().code,
        400
    );
    assert!(dispatch(&svc, "project_export_zip", &json!({})).is_none());
    assert_eq!(
        dispatch(&svc, "legacy_list", &json!({"folder": 3}))
            .unwrap()
            .unwrap_err()
            .code,
        400
    );
}

#[test]
fn project_import_checks_its_folder() {
    let svc = Service::new(ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    assert_eq!(svc.project_import("  ").unwrap_err().code, 400);
    assert_eq!(svc.project_import("/nonexistent/session").unwrap_err().code, 404);
}
