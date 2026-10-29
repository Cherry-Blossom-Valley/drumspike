use serde_json::{json, Map, Value};

use super::*;
use crate::api;
use crate::call::call;

fn svc() -> Service {
    Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    })
}

fn m(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn check_args_names_the_key_and_the_type() {
    let row = api::find("events_edit").unwrap();
    assert!(check_args(
        row,
        &m(json!({"sid": "a", "name": "n", "ops": [{"op": "add", "t": 1}]}))
    )
    .is_ok());
    for (bad, text) in [
        (json!({"name": "n", "ops": []}), "missing argument `sid`"),
        (
            json!({"sid": 1, "name": "n", "ops": []}),
            "wrong argument `sid`: expected string",
        ),
        (json!({"sid": "a", "name": "n", "ops": [1]}), "expected list of objects"),
        (
            json!({"sid": "a", "name": "n", "ops": [], "x": 1}),
            "unknown argument `x`",
        ),
    ] {
        let e = check_args(row, &m(bad)).unwrap_err();
        assert_eq!(e.code, 400);
        assert!(e.message.contains(text), "{}", e.message);
    }
    assert!(check_args(api::find("app_info").unwrap(), &m(json!({"x": 1})))
        .unwrap_err()
        .message
        .contains("none"));
}

#[test]
fn a_read_only_project_allows_what_does_not_change_it() {
    let f = |n: &str, a: Value| changes_project(api::find(n).unwrap(), &m(a));
    assert!(f("events_edit", json!({})) && f("project_save", json!({})) && f("analyze_start", json!({})));
    assert!(f("track_repick", json!({"store": true})));
    assert!(!f("track_repick", json!({"store": false})) && !f("track_repick", json!({})));
    assert!(!f("project_save_as", json!({})) && !f("project_get", json!({})));
}

/// The pinned defaults: each surface's default is one constant here, and the agent surface fills the table.
#[test]
fn the_defaults_are_pinned_once() {
    assert_eq!(
        AGENT_DEFAULTS,
        [
            ("analyze_start", "keep_edited", true),
            ("track_repick", "keep_edited", true),
            ("track_velocity_set", "apply", true),
        ]
    );
    let mut a = m(json!({"sid": "a", "keep_edited": false}));
    fill_agent_defaults("analyze_start", &mut a);
    assert_eq!(a.get("keep_edited"), Some(&json!(false)), "a given argument wins");
    let mut b = Map::new();
    fill_agent_defaults("analyze_start", &mut b);
    assert_eq!(b.get("keep_edited"), Some(&json!(true)));
    let mut c = Map::new();
    fill_agent_defaults("events_edit", &mut c);
    assert!(c.is_empty(), "a command without a table row gets nothing");
}

/// Equal defaults: the bridge's export parser and the policy give the same `compensate`, and the bridge's
/// transport window is the one `PlayWindow::from_args` gives with nothing set.
#[test]
fn equal_defaults_on_the_bridge_and_the_policy() {
    let opts = crate::export::ExportOpts::from_args(&json!({})).unwrap();
    assert_eq!(opts.compensate, COMPENSATE_DEFAULT);
    assert!(transport_args::play_window(None, None, None, None, None)
        .unwrap()
        .is_none());
    assert_eq!(transport_args::opt_f64(&json!({}), "start").unwrap(), None);
    assert_eq!(transport_args::opt_bool(&json!({}), "close"), None);
}

/// Equal validation on the agent surface: an unknown key and a wrong type answer 400 with the key named.
#[test]
fn validation_names_the_key_on_the_agent_surface() {
    let s = svc();
    let e = call(
        &s,
        "events_edit",
        &json!({"sid": "a", "name": "n", "ops": [], "x": 1}),
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, 400);
    assert!(e.message.contains("unknown argument `x`"), "{}", e.message);
    let e = call(&s, "events_edit", &json!({"sid": 1, "name": "n", "ops": []}), None).unwrap_err();
    assert_eq!(e.code, 400);
    assert!(e.message.contains("wrong argument `sid`"), "{}", e.message);
}

/// Equal mutation policy: a command that changes the project is not refused for a project that is not open
/// (the guard only looks at an open read-only project); a read-only refusal is 409 (see `tests/read_only.rs`).
#[test]
fn the_guard_passes_a_call_on_an_unknown_project_to_the_command() {
    let s = svc();
    let row = api::find("events_edit").unwrap();
    assert!(read_only_refusal(&s, row, &m(json!({"sid": "zz"}))).is_none());
    assert!(read_only_refusal(&s, row, &m(json!({}))).is_none());
    let save_as = api::find("project_save_as").unwrap();
    assert!(read_only_refusal(&s, save_as, &m(json!({"sid": "zz"}))).is_none());
}
