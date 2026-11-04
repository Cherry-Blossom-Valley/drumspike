//! R3 stage A: characterization of the dispatch. Every registry row answers on the page surface (the test bridge,
//! `Bridge::invoke`) and on the agent surface (`call::call`) as recorded in `tests/dispatch_rows.txt`, for four
//! argument shapes: `{}`, the required arguments only, `null`, and every argument of a wrong type. A change of
//! the dispatch must leave every line as it is. Set `DRUMSPIKE_BLESS_DISPATCH=1` to write the file again (only
//! for a change that means to change an answer).
//!
//! The surface defaults are pinned with a real project: `keep_edited` is false on the page and true for an agent;
//! a wave window without `start`/`end` is the window `0`/`i64::MAX`.

mod common;

use std::path::Path;

use common::*;
use drumspike_app::api::{self, Arg, CommandSpec, Ty};
use drumspike_app::bridge::Bridge;
use drumspike_app::call::call;
use drumspike_app::reply::Reply;
use drumspike_app::{AppError, Service};
use drumspike_core::transport::device::Output;
use serde_json::{Value, json};

const SNAPSHOT: &str = "tests/dispatch_rows.txt";

/// A string value for an argument: a path or folder inside the test root, a URL nobody listens on, else `zz`.
fn str_value(a: &Arg, root: &Path) -> Value {
    let n = a.name;
    if n == "url" {
        return json!("http://127.0.0.1:9/");
    }
    if ["path", "dir", "folder", "dest", "session_dir", "file"]
        .iter()
        .any(|k| n.contains(k))
    {
        return json!(root.join("missing").display().to_string());
    }
    json!("zz")
}

fn right_value(a: &Arg, root: &Path) -> Value {
    match a.ty {
        Ty::Str => str_value(a, root),
        Ty::Int => json!(0),
        Ty::Num => json!(0.0),
        Ty::Bool => json!(false),
        Ty::Strs | Ty::Objs => json!([]),
        Ty::Obj | Ty::Any => json!({}),
        Ty::TempoMap => Value::Null,
    }
}

fn wrong_value(a: &Arg) -> Value {
    match a.ty {
        Ty::Str => json!(1),
        Ty::Int | Ty::Num | Ty::Bool | Ty::Strs | Ty::Obj | Ty::Objs | Ty::TempoMap => json!("x"),
        Ty::Any => json!([1]),
    }
}

fn shapes(c: &CommandSpec, root: &Path) -> [(&'static str, Value); 4] {
    let minimal: serde_json::Map<String, Value> = c
        .args
        .iter()
        .filter(|a| a.required)
        .map(|a| (a.name.to_string(), right_value(a, root)))
        .collect();
    let wrong: serde_json::Map<String, Value> = c.args.iter().map(|a| (a.name.to_string(), wrong_value(a))).collect();
    [
        ("{}", json!({})),
        ("min", Value::Object(minimal)),
        ("null", Value::Null),
        ("wrong", Value::Object(wrong)),
    ]
}

/// One answer as a short line: the kind of reply (top-level keys of an object, length of a list or of bytes) or
/// the error code and message, with the test root replaced by `<root>`.
fn outcome(r: Result<Reply, AppError>, root: &Path) -> String {
    match r {
        Ok(Reply::Bytes(b)) => format!("ok bytes {}", b.len()),
        Ok(Reply::Json(Value::Object(m))) => {
            format!("ok {{{}}}", m.keys().cloned().collect::<Vec<_>>().join(","))
        }
        Ok(Reply::Json(Value::Array(a))) => format!("ok [{}]", a.len()),
        Ok(Reply::Json(v)) => format!("ok {v}"),
        Err(e) => {
            let msg = e.message.replace(&root.display().to_string(), "<root>");
            format!("err {} {}", e.code, msg.replace('\n', " "))
        }
    }
}

fn table() -> String {
    let root = tmp("dispatch_rows");
    let svc = Service::new(drumspike_app::ServiceConfig::default());
    svc.set_home(root.join("home"));
    svc.set_projects_dir(root.join("projects"));
    svc.set_output_select(Output::Null { sample_rate: None });
    let bridge = Bridge::new(svc.clone(), &root);
    let mut out = String::new();
    for c in api::REGISTRY {
        for (tag, args) in shapes(c, &root) {
            let page = outcome(bridge.invoke(c.name, &args), &root);
            let agent = outcome(call(&svc, c.name, &args, None), &root);
            out.push_str(&format!("{} {tag}\n  page: {page}\n  agent: {agent}\n", c.name));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    out
}

/// Every registry row answers each argument shape on both surfaces as recorded: the reply kind or the error code
/// and message. Pins the dispatch (which handler runs a name, its argument checks and defaults) across R3.
#[test]
fn every_registry_row_answers_each_argument_shape_as_recorded() {
    let got = table();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(SNAPSHOT);
    if std::env::var_os("DRUMSPIKE_BLESS_DISPATCH").is_some() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap();
    let diff: Vec<String> = want
        .lines()
        .zip(got.lines())
        .filter(|(w, g)| w != g)
        .map(|(w, g)| format!("- {w}\n+ {g}"))
        .take(20)
        .collect();
    assert!(
        diff.is_empty() && want.lines().count() == got.lines().count(),
        "{} lines differ (first 20):\n{}",
        diff.len(),
        diff.join("\n")
    );
}

fn opened(tag: &str) -> (Service, String, std::path::PathBuf) {
    let root = tmp(tag);
    let dir = project(&root, "Dispatch", &[kick_spec("k in", "kick", 36)]);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    (svc, sid, root)
}

fn added(svc: &Service, sid: &str) -> usize {
    svc.events_get(sid, "k in").unwrap()["events"]
        .as_array()
        .map_or(0, |a| a.iter().filter(|e| e["added"] == true).count())
}

/// `analyze_start` without `keep_edited` replaces a hand edit on the page surface and keeps it for an agent.
#[test]
fn analyze_start_without_keep_edited_replaces_edits_on_the_page_and_keeps_them_for_an_agent() {
    let (svc, sid, root) = opened("dispatch_keep");
    let bridge = Bridge::new(svc.clone(), &root);
    let edit = || {
        svc.events_edit(&sid, "k in", &[json!({"op": "add", "t": 0.1})])
            .unwrap();
        assert_eq!(added(&svc, &sid), 1);
    };
    let args = json!({"sid": sid, "tracks": ["k in"]});
    edit();
    bridge.invoke("analyze_start", &args).unwrap();
    wait_done(&svc, &sid);
    assert_eq!(added(&svc, &sid), 0, "the page default replaces the edit");
    edit();
    call(&svc, "analyze_start", &args, None).unwrap();
    wait_done(&svc, &sid);
    assert_eq!(added(&svc, &sid), 1, "the agent default keeps the edit");
    let _ = std::fs::remove_dir_all(&root);
}

/// A wave window without `start` and `end` is the window from 0 to `i64::MAX`; a wrong type falls back the same.
#[test]
fn a_wave_window_without_start_and_end_is_the_whole_file() {
    let (svc, sid, root) = opened("dispatch_wave");
    let shot = root.join("shot.wav");
    write_wav(&shot, 48_000, &kick_signal(&[0.01], 0.2, 48_000));
    svc.track_layer_set(&sid, "k in", &json!({"path": shot.display().to_string()}))
        .unwrap();
    let bridge = Bridge::new(svc.clone(), &root);
    let bytes = |a: Value| match bridge.invoke("track_layer_wave", &a).unwrap() {
        Reply::Bytes(b) => b,
        Reply::Json(v) => panic!("bytes expected, got {v}"),
    };
    let whole = bytes(json!({"sid": sid, "name": "k in"}));
    assert_eq!(
        whole,
        bytes(json!({"sid": sid, "name": "k in", "start": 0, "end": i64::MAX}))
    );
    assert_eq!(
        whole,
        bytes(json!({"sid": sid, "name": "k in", "start": "x", "end": "y"}))
    );
    assert_ne!(
        whole,
        bytes(json!({"sid": sid, "name": "k in", "start": 10, "end": 20}))
    );
    let _ = std::fs::remove_dir_all(&root);
}
