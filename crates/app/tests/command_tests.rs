//! Every command of the slice (P3-3a), of the project lifecycle (P3-4a) and of the transport (P3-5b) has a
//! test named after it. P3-12: this was part of `route_map.rs` and `lifecycle_route_map.rs`, which checked the
//! rows of `apps/desktop/route-map.json` (the server's routes); the route map went with the server.

use std::path::Path;

use drumspike_app::{COMMANDS, LAYER_COMMANDS, LIFECYCLE_COMMANDS, SAMPLER_COMMANDS, TRANSPORT_COMMANDS};

fn has_tests(file: &str, commands: &[&str]) -> Vec<String> {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(file)).unwrap();
    commands
        .iter()
        .filter(|c| !src.contains(&format!("fn {c}_")))
        .map(|c| c.to_string())
        .collect()
}

#[test]
fn every_command_has_a_test_in_commands_rs() {
    assert_eq!(has_tests("commands.rs", &COMMANDS), Vec::<String>::new());
}

#[test]
fn every_lifecycle_command_has_a_test_in_lifecycle_rs() {
    assert_eq!(has_tests("lifecycle.rs", &LIFECYCLE_COMMANDS), Vec::<String>::new());
}

#[test]
fn every_transport_command_has_a_test_in_transport_rs() {
    assert_eq!(has_tests("transport.rs", &TRANSPORT_COMMANDS), Vec::<String>::new());
}

/// P4-7a: the layer commands, in `layer.rs`.
#[test]
fn every_layer_command_has_a_test_in_layer_rs() {
    assert_eq!(has_tests("layer.rs", &LAYER_COMMANDS), Vec::<String>::new());
}

/// P4-7c: the sampler commands, in `sampler.rs`.
#[test]
fn every_sampler_command_has_a_test_in_sampler_rs() {
    assert_eq!(has_tests("sampler.rs", &SAMPLER_COMMANDS), Vec::<String>::new());
}

#[test]
fn the_check_can_fail() {
    assert_eq!(has_tests("commands.rs", &["no_such_command"]), ["no_such_command"]);
}

/// P3-6b: the page holds no PCM: the backend never invokes a PCM or audio command and reads waveforms as tiles
/// and sample windows.
#[test]
fn the_backend_never_asks_for_pcm() {
    let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../static/backend.js")).unwrap();
    for bad in [
        "invoke(\"pcm",
        "invoke(\"audio",
        "invoke(\"samples_all",
        "/api/pcm",
        "/api/audio",
        " pcm:",
        " audio:",
    ] {
        assert!(!js.contains(bad), "backend.js has {bad}");
    }
    assert!(js.contains("peaks_tile") && js.contains("samples_window"));
}

/// #70: the page sends `keep_edited` itself on both invokes that need it (see `PAGE_KEEP_EDITED`).
#[test]
fn the_page_sends_keep_edited_itself() {
    let js = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../static/backend.js")).unwrap();
    for command in ["analyze_start", "track_repick"] {
        let call = format!("invoke(\"{command}\"");
        let line = js
            .lines()
            .find(|l| l.contains(&call))
            .unwrap_or_else(|| panic!("backend.js has no {call}"));
        assert!(line.contains("keep_edited"), "{call} does not send keep_edited: {line}");
    }
}
