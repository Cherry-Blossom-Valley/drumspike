//! P4-5 step 1: the command registry (`src/api.rs`) against the Tauri wrappers
//! (`apps/desktop/src-tauri/src/*.rs`, read as text) and against real answers.
//!
//! * every command in a `generate_handler!` list has a row, and every row is in a list (the debug
//!   list holds the `Selftest` rows, the release list every other row);
//! * every `#[tauri::command]` function is in a list;
//! * the arguments of a row are the parameters of its wrapper (without `State`, `AppHandle` and the
//!   `progress` channel), and a wrapper with a multi-word parameter has `rename_all = "snake_case"`
//!   (the wire names are the registry's);
//! * a `progress` parameter is a command that is `Run::Progress` or starts a job with a channel;
//! * the `required` keys of a result schema are in a real answer.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use common::*;
use drumspike_app::api::{REGISTRY, Run, Surface, find};
use drumspike_app::cli;
use serde_json::{Value, json};

fn tauri_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/src-tauri/src")
}

/// The names in the `generate_handler![...]` blocks: (release, debug).
fn handler_lists(lib: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut lists = Vec::new();
    let mut rest = lib;
    while let Some(i) = rest.find("generate_handler![") {
        rest = &rest[i + "generate_handler![".len()..];
        let end = rest.find(']').unwrap();
        let names: BTreeSet<String> = rest[..end]
            .split(',')
            .map(|s| s.trim().rsplit("::").next().unwrap().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        lists.push(names);
        rest = &rest[end..];
    }
    assert_eq!(lists.len(), 2, "a release and a debug handler list");
    let (a, b) = (lists.remove(0), lists.remove(0));
    if b.len() >= a.len() { (a, b) } else { (b, a) }
}

/// A wrapper: its parameters (name, type text) and whether the attribute renames to `snake_case`.
struct Wrapper {
    params: Vec<(String, String)>,
    snake: bool,
}

/// Every `#[tauri::command...]` function of the desktop crate.
fn wrappers() -> BTreeMap<String, Wrapper> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(tauri_src()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        let pieces: Vec<&str> = src.split("#[tauri::command").collect();
        for (i, chunk) in pieces.iter().enumerate().skip(1) {
            // an attribute starts its line; the same text inside a string literal is not one
            if !pieces[i - 1].rsplit('\n').next().unwrap().trim().is_empty() {
                continue;
            }
            let attr_end = chunk.find(']').unwrap();
            let snake = chunk[..attr_end].contains("rename_all = \"snake_case\"");
            let after = &chunk[chunk.find("fn ").unwrap() + 3..];
            let name = after[..after.find(['(', '<']).unwrap()].trim().to_string();
            let open = after.find('(').unwrap();
            // the parameter text up to the matching close parenthesis
            let (mut depth, mut end) = (0i32, 0usize);
            for (k, ch) in after[open..].char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + k;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let text = &after[open + 1..end];
            let (mut parts, mut depth, mut cur) = (Vec::new(), 0i32, String::new());
            for ch in text.chars() {
                match ch {
                    '<' | '(' | '[' => depth += 1,
                    '>' | ')' | ']' => depth -= 1,
                    ',' if depth == 0 => {
                        parts.push(std::mem::take(&mut cur));
                        continue;
                    }
                    _ => {}
                }
                cur.push(ch);
            }
            parts.push(cur);
            let params = parts
                .iter()
                .filter_map(|p| p.split_once(':'))
                // a raw identifier (`r#loop`) is the argument `loop`: Tauri takes the name without the `r#`
                .map(|(n, t)| (n.trim().trim_start_matches("r#").to_string(), t.trim().to_string()))
                .collect();
            out.insert(name, Wrapper { params, snake });
        }
    }
    out
}

fn registry_names(selftest: bool) -> BTreeSet<String> {
    REGISTRY
        .iter()
        .filter(|c| (c.surface == Surface::Selftest) == selftest)
        .map(|c| c.name.to_string())
        .collect()
}

#[test]
fn every_tauri_handler_has_a_row_and_every_row_is_in_a_handler_list() {
    let lib = std::fs::read_to_string(tauri_src().join("lib.rs")).unwrap();
    let (release, debug) = handler_lists(&lib);
    assert_eq!(release, registry_names(false), "release handler list against the rows");
    let mut all = registry_names(false);
    all.extend(registry_names(true));
    assert_eq!(debug, all, "debug handler list against the rows");
}

#[test]
fn every_tauri_command_function_is_in_a_handler_list() {
    let lib = std::fs::read_to_string(tauri_src().join("lib.rs")).unwrap();
    let (_, debug) = handler_lists(&lib);
    let fns: BTreeSet<String> = wrappers().keys().cloned().collect();
    assert_eq!(fns, debug, "#[tauri::command] functions against the debug handler list");
}

#[test]
fn the_arguments_of_a_row_are_the_parameters_of_its_wrapper() {
    let w = wrappers();
    for c in REGISTRY {
        let wrapper = w.get(c.name).unwrap_or_else(|| panic!("{}: no wrapper", c.name));
        let mut want: Vec<&str> = c.args.iter().map(|a| a.name).collect();
        let mut got = Vec::new();
        let mut channel = false;
        for (n, t) in &wrapper.params {
            if t.contains("State<") || t.contains("AppHandle") {
                continue;
            }
            if t.contains("Channel<") {
                assert_eq!(n, "progress", "{}", c.name);
                channel = true;
                continue;
            }
            got.push(n.as_str());
        }
        want.sort_unstable();
        got.sort_unstable();
        assert_eq!(got, want, "{}: wrapper parameters against the row", c.name);
        assert_eq!(channel, c.has_progress(), "{}: the progress channel", c.name);
        // optional in the row means Option in the wrapper, and the reverse
        for a in c.args {
            let (_, t) = wrapper.params.iter().find(|(n, _)| n == a.name).unwrap();
            assert_eq!(
                t.starts_with("Option<"),
                !a.required,
                "{}.{}: required against Option",
                c.name,
                a.name
            );
        }
        let multi = c.args.iter().any(|a| a.name.contains('_'));
        if multi {
            assert!(
                wrapper.snake,
                "{}: a multi-word argument needs rename_all = \"snake_case\"",
                c.name
            );
        }
    }
}

#[test]
fn the_check_can_fail() {
    // a handler list without a row and a row without a handler would both show up as a set difference
    let (r, _) = handler_lists("generate_handler![a, b::c] generate_handler![a, b::c, d]");
    assert_eq!(r.len(), 2);
    assert_ne!(r, registry_names(false));
    assert!(find("events_edit").is_some() && find("nope").is_none());
    assert!(REGISTRY.iter().any(|c| matches!(c.run, Run::Poll(_))));
}

/// Every `required` key of a result schema is in a real answer (the commands that run without a
/// model on a two-track project).
#[test]
fn result_schemas_name_keys_that_real_answers_have() {
    let root = tmp("api1");
    let dir = project(
        &root,
        "Api",
        &[kick_spec("k in", "kick", 36), kick_spec("s top", "snare", 38)],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let opened = svc.project_open(&dir.display().to_string()).unwrap();
    let sid = opened["sid"].as_str().unwrap().to_string();
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    let ev = svc.events_get(&sid, "k in").unwrap();
    let mid = svc.export_midi_stored(&sid, "k in", None, None, None, None).unwrap()["path"]
        .as_str()
        .unwrap()
        .to_string();
    let answers: Vec<(&str, Value)> = vec![
        ("app_info", svc.app_info()),
        ("project_open", opened),
        ("project_get", svc.project_get(&sid).unwrap()),
        ("tracks_get", svc.tracks_get(&sid).unwrap()),
        ("events_get", ev.clone()),
        ("events_set", svc.events_set(&sid, "k in", &ev["events"]).unwrap()),
        ("peaks_info", svc.peaks_info(&sid, "k in").unwrap()),
        ("analyze_status", svc.analyze_status(&sid)),
        (
            "export_midi",
            svc.export_midi_stored(&sid, "k in", None, None, None, None).unwrap(),
        ),
        ("export_spikes", svc.export_spikes(&sid, "k in", None).unwrap()),
        (
            "velocities_recompute",
            svc.velocities_recompute(&sid, "k in", None).unwrap(),
        ),
        (
            "track_velocity_set",
            svc.track_velocity_set(&sid, "k in", -40.0, -6.0, None, None).unwrap(),
        ),
        (
            "track_params_set",
            svc.track_params_set(&sid, "k in", json!({"min_gap_ms": 20}).as_object().unwrap())
                .unwrap(),
        ),
        (
            "events_edit",
            svc.events_edit(&sid, "k in", &[json!({"op": "add", "t": 0.1})])
                .unwrap(),
        ),
        ("track_shift_set", svc.track_shift_set(&sid, "k in", 4).unwrap()),
        ("tempo_map_get", svc.tempo_map_get(&sid).unwrap()),
        (
            "tempo_map_set",
            svc.tempo_map_set(&sid, &json!({"bpm": 100.0, "bar1_s": 1.0})).unwrap(),
        ),
        ("tempo_map_import_midi", svc.tempo_map_import_midi(&sid, &mid).unwrap()),
        (
            "tempo_map_detect",
            svc.tempo_map_detect(&sid, None, None, None).unwrap(),
        ),
        // #150: the confidence of each hit
        ("track_confidence", svc.track_confidence(&sid, "k in", None).unwrap()),
        // P4-40g: the review markers (set twice: the second answers the first as `previous`)
        ("markers_get", svc.markers_get(&sid).unwrap()),
        (
            "markers_set",
            svc.markers_set(&sid, &[json!({"t": 1.5, "note": "check"}), json!({"t": 0.5})])
                .unwrap(),
        ),
        ("markers_set", svc.markers_set(&sid, &[json!({"t": 2.0})]).unwrap()),
        ("markers_get", svc.markers_get(&sid).unwrap()),
        (
            "tracks_reorder",
            svc.tracks_reorder(&sid, &json!(["s top", "k in"])).unwrap(),
        ),
        (
            "track_set_mix",
            svc.track_set_mix(&sid, "k in", Some(-3.0), None).unwrap(),
        ),
        // P4-7a and P4-7b: the snare file is the kick's one-shot
        (
            "track_layer_set",
            svc.track_layer_set(
                &sid,
                "k in",
                &json!({"path": root.join("daw").join("s top.wav").display().to_string(), "align": true}),
            )
            .unwrap(),
        ),
        ("track_layer_get", svc.track_layer_get(&sid, "k in").unwrap()),
        ("track_layer_align", svc.track_layer_align(&sid, "k in").unwrap()),
        ("export_layer", svc.export_layer(&sid, "k in").unwrap()),
        // P4-7c: the sampler calibration (the snare file is the bounce)
        (
            "sampler_calibration_midi",
            svc.sampler_calibration_midi(&sid, "k in", None).unwrap(),
        ),
        (
            "track_sampler_set",
            svc.track_sampler_set(
                &sid,
                "k in",
                &json!({"path": root.join("daw").join("s top.wav").display().to_string()}),
            )
            .unwrap(),
        ),
        ("track_sampler_get", svc.track_sampler_get(&sid, "k in").unwrap()),
        ("app_settings_set", svc.app_settings_set("zz_key", &json!(3)).unwrap()),
        ("project_rename", svc.project_rename(&sid, "Renamed").unwrap()),
        ("track_rename", svc.track_rename(&sid, "s top", "s top2").unwrap()),
        ("track_set_note", svc.track_set_note(&sid, "k in", 40).unwrap()),
        (
            "track_set_follow",
            svc.track_set_follow(&sid, "s top2", Some("k in")).unwrap(),
        ),
        ("peaks_build_status", svc.peaks_build_status(&sid)),
        ("export_existing", svc.export_existing(&sid).unwrap()),
        (
            "tracks_scan",
            svc.tracks_scan(&[root.join("daw").display().to_string()], None)
                .unwrap(),
        ),
        ("kit_preset_save", svc.kit_preset_save("zz", &json!([])).unwrap()),
        ("kit_preset_load", svc.kit_preset_load("zz").unwrap()),
        ("project_save", svc.project_save(&sid).unwrap()),
        ("project_clean_check", svc.project_clean_check(&sid).unwrap()),
        ("job_status", svc.job_status(&sid)),
        ("job_cancel", svc.job_cancel(&sid)),
        (
            "project_relink",
            svc.project_relink(
                &sid,
                "k in",
                &root.join("daw").join("k in.wav").display().to_string(),
                false,
            )
            .unwrap(),
        ),
        (
            "relink_search",
            svc.relink_search(&sid, &root.join("daw").display().to_string())
                .unwrap(),
        ),
        (
            "tracks_remove",
            svc.tracks_remove(&sid, &["s top2".to_string()], false).unwrap(),
        ),
        ("export_all", svc.export_all(&sid, None).unwrap()),
        ("export_dest_set", svc.export_dest_set(&sid, None).unwrap()),
        ("export_markers_set", svc.export_markers_set(&sid, false).unwrap()),
        ("export_compensate_set", svc.export_compensate_set(&sid, false).unwrap()),
        ("export_last_get", svc.export_last_get(&sid).unwrap()),
        (
            "export_last_set",
            svc.export_last_set(&sid, &json!({"formats": ["midi"]})).unwrap(),
        ),
        (
            "project_save_as",
            svc.project_save_as(
                &sid,
                Some("Copy"),
                Some(&root.join("out").display().to_string()),
                false,
                None,
            )
            .unwrap(),
        ),
    ];
    let _ = svc.project_close(&sid, true);
    assert!(answers.len() > 30);
    for (name, answer) in answers {
        let schema = find(name).unwrap().result_schema();
        assert_eq!(schema["type"], "object", "{name}");
        for key in schema["required"].as_array().into_iter().flatten() {
            assert!(
                answer.get(key.as_str().unwrap()).is_some(),
                "{name}: the answer has no `{key}`: {answer}"
            );
        }
    }
}

/// P4-5 done criterion: every command of the agent surface has a command-line subcommand and an MCP tool, and
/// nothing else is on either list (the extra MCP tool is `job_get`).
#[test]
fn every_agent_row_has_a_cli_subcommand_and_an_mcp_tool() {
    use drumspike_app::{cli, mcp};
    let agent: BTreeSet<&str> = REGISTRY
        .iter()
        .filter(|c| c.surface == Surface::Agent)
        .map(|c| c.name)
        .collect();
    let tools: BTreeSet<String> = mcp::tools()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    let listed: BTreeSet<String> = cli::commands_json()["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    let want: BTreeSet<String> = agent.iter().map(std::string::ToString::to_string).collect();
    assert_eq!(listed, want, "`drumspike commands`");
    let mut with_job = want.clone();
    with_job.insert("job_get".into());
    assert_eq!(tools, with_job, "MCP tools");
    let svc = service_dsp();
    for name in &agent {
        assert!(cli::help_text(Some(name)).is_ok(), "{name}: drumspike help");
        // a flag that no row has: the subcommand exists (a usage error about the flag, not about the command)
        let (out, err) = (
            Vec::<u8>::new(),
            std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new())),
        );
        let _ = out;
        let sink = SharedBuf(err.clone());
        let mut io = cli::Io {
            input: Box::new(std::io::Cursor::new(Vec::new())),
            out: Box::new(std::io::sink()),
            err: std::sync::Arc::new(std::sync::Mutex::new(Box::new(sink))),
        };
        let code = cli::run(&svc, &[name.to_string(), "--no-such-flag".into(), "1".into()], &mut io);
        let text = String::from_utf8(err.lock().unwrap().clone()).unwrap();
        assert_eq!(code, 2, "{name}: {text}");
        assert!(text.contains("unknown flag"), "{name}: {text}");
    }
    // the check can fail: a command that is not on the surface is unknown to the command line
    let err = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let mut io = cli::Io {
        input: Box::new(std::io::Cursor::new(Vec::new())),
        out: Box::new(std::io::sink()),
        err: std::sync::Arc::new(std::sync::Mutex::new(Box::new(SharedBuf(err.clone())))),
    };
    let code = cli::run(&svc, &["transport_play".into()], &mut io);
    assert_eq!(code, 3, "an unknown command exits 3, as `help nope` does");
    assert!(
        String::from_utf8(err.lock().unwrap().clone())
            .unwrap()
            .contains("unknown command")
    );
}

struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for SharedBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- the MCP answers against their output schemas

/// The JSON Schema subset the registry and the MCP server use: `type`, `required`, `properties`, `items`.
fn validate(schema: &Value, v: &Value, at: &str) -> Result<(), String> {
    if let Some(t) = schema.get("type").and_then(Value::as_str) {
        let ok = match t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "number" => v.is_number(),
            "integer" => v.is_i64() || v.is_u64(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            _ => true,
        };
        if !ok {
            return Err(format!("{at}: expected {t}, got {v}"));
        }
    }
    if let (Some(req), Some(o)) = (schema.get("required").and_then(Value::as_array), v.as_object()) {
        for k in req {
            if !o.contains_key(k.as_str().unwrap()) {
                return Err(format!("{at}: missing `{k}` in {v}"));
            }
        }
    }
    if let (Some(props), Some(o)) = (schema.get("properties").and_then(Value::as_object), v.as_object()) {
        for (k, p) in props {
            if let Some(x) = o.get(k) {
                validate(p, x, &format!("{at}.{k}"))?;
            }
        }
    }
    if let (Some(items), Some(a)) = (schema.get("items"), v.as_array()) {
        for (i, x) in a.iter().enumerate() {
            validate(items, x, &format!("{at}[{i}]"))?;
        }
    }
    Ok(())
}

/// One MCP message through `mcp::serve` on `svc`; the one answer.
fn mcp_once(
    svc: &drumspike_app::Service,
    jobs: &drumspike_app::mcp::Jobs,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string() + "\n";
    let buf = SharedBuf(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
    drumspike_app::mcp::serve_with(
        svc,
        jobs,
        std::io::Cursor::new(line.into_bytes()),
        Box::new(buf.clone()),
    );
    let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    serde_json::from_str(text.lines().next().expect("an answer")).unwrap()
}

impl Clone for SharedBuf {
    fn clone(&self) -> SharedBuf {
        SharedBuf(self.0.clone())
    }
}

/// P4-5 fix round H1: every `tools/call` answer that is not an error conforms to the `outputSchema` its tool lists (a
/// strict client, the official TypeScript SDK, rejects an answer that does not). The `Progress` tools answer `{job}`;
/// `job_get` holds the result. Also: every listed tool dispatches at `tools/call` (never a JSON-RPC error), and every
/// `Agent` row runs in `drumspike batch` (never "unknown command").
#[test]
fn every_mcp_answer_validates_against_its_output_schema_and_every_tool_and_batch_step_dispatches() {
    let root = tmp("api_mcp");
    let dir = project(
        &root,
        "Mcp",
        &[kick_spec("k in", "kick", 36), kick_spec("s top", "snare", 38)],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    svc.set_projects_dir(root.join("projects"));
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let jobs = drumspike_app::mcp::Jobs::default();
    let listed = mcp_once(&svc, &jobs, 1, "tools/list", json!({}));
    let schemas: std::collections::BTreeMap<String, Value> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                t.get("outputSchema").cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    let (daw, k, s) = (root.join("daw"), "k in", "s top");
    write_wav(&daw.join("extra.wav"), 48_000, &kick_signal(&ONSETS, 3.5, 48_000));
    let q = |p: &std::path::Path| p.display().to_string();
    let sd = |a: Value| {
        let mut a = a;
        a["sid"] = json!(sid);
        a
    };
    let mut calls: Vec<(&str, Value)> = vec![
        ("app_info", json!({})),
        ("project_get", sd(json!({}))),
        ("project_list", json!({})),
        ("tracks_get", sd(json!({}))),
        ("events_get", sd(json!({"name": k}))),
        (
            "events_set",
            sd(json!({"name": k, "events": [{"t": 0.5, "v": 90}, {"t": 1.0, "v": 90}]})),
        ),
        ("events_edit", sd(json!({"name": k, "ops": [{"op": "add", "t": 0.2}]}))),
        (
            "track_params_set",
            sd(json!({"name": k, "params": {"sensitivity": 60}})),
        ),
        ("analyze_start", sd(json!({}))),
        ("analyze_status", sd(json!({}))),
        // #265: the hits of a range only (after analyze_start, so the events are there)
        ("reanalyze_selection", sd(json!({"name": k, "t0": 0.2, "t1": 0.6}))),
        ("track_histograms", sd(json!({"name": k}))),
        ("track_confidence", sd(json!({"name": k}))),
        ("track_repick", sd(json!({"name": k}))),
        ("track_velocity_set", sd(json!({"name": k, "lo_db": -40, "hi_db": -6}))),
        ("track_shift_set", sd(json!({"name": k, "shift_samples": 4}))),
        // P4-6a / #293: Learn marks on the DSP candidates (a range of one kick hit, then a Forget range)
        (
            "track_learn_mark",
            sd(json!({"name": k, "kind": "this", "t0": 0.2, "t1": 0.3})),
        ),
        (
            "track_learn_mark",
            sd(json!({"name": k, "kind": "not", "t0": 0.7, "t1": 0.8})),
        ),
        ("track_learn_flip", sd(json!({"name": k, "id": "m1"}))),
        ("track_learn_remove", sd(json!({"name": k, "id": "m1"}))),
        ("track_learn_forget_at", sd(json!({"name": k, "t0": 0.7, "t1": 0.8}))),
        ("track_learn_scores", sd(json!({"name": k}))),
        (
            "track_learn_set",
            sd(json!({"name": k, "learn": {"on": false, "marks": []}})),
        ),
        ("track_learn_forget", sd(json!({"name": k}))),
        ("tempo_map_get", sd(json!({}))),
        ("tempo_map_set", sd(json!({"map": {"bpm": 90.0, "meter": [3, 4]}}))),
        ("export_midi", sd(json!({"name": k}))),
        (
            "tempo_map_import_midi",
            sd(json!({"path": q(&dir.join("Exports").join("k in_spike.mid"))})),
        ),
        ("tempo_map_detect", sd(json!({"min_bars": 4}))),
        ("tempo_map_refit_bar1", sd(json!({"bpm": 92.0}))),
        ("export_spikes", sd(json!({"name": k}))),
        ("velocities_recompute", sd(json!({"name": k}))),
        ("export_all", sd(json!({}))),
        ("export_existing", sd(json!({}))),
        ("track_set_mix", sd(json!({"name": k, "volume_db": -3}))),
        (
            "track_layer_set",
            sd(json!({"name": k, "path": q(&daw.join("extra.wav")), "polarity": "auto"})),
        ),
        ("track_layer_get", sd(json!({"name": k}))),
        ("track_layer_align", sd(json!({"name": k}))),
        ("export_layer", sd(json!({"name": k}))),
        ("sampler_calibration_midi", sd(json!({"name": k}))),
        (
            "track_sampler_set",
            sd(json!({"name": k, "path": q(&daw.join("extra.wav"))})),
        ),
        ("track_sampler_get", sd(json!({"name": k}))),
        ("track_set_note", sd(json!({"name": k, "note": 40}))),
        ("track_set_role", sd(json!({"name": k, "role": "kick"}))),
        ("track_set_follow", sd(json!({"name": s, "source": k}))),
        ("track_set_follow", sd(json!({"name": s}))),
        ("tracks_reorder", sd(json!({"order": [s, k]}))),
        ("track_rename", sd(json!({"name": s, "new_name": "s top2"}))),
        ("project_rename", sd(json!({"name": "Renamed"}))),
        ("app_settings_get", json!({})),
        ("app_settings_set", json!({"key": "zz_key", "value": 3})),
        ("project_recent", json!({})),
        ("project_recent_clear", json!({})),
        ("kit_presets_list", json!({})),
        ("kit_preset_save", json!({"name": "zz", "rules": []})),
        ("kit_preset_load", json!({"name": "zz"})),
        ("tracks_scan", json!({"paths": [q(&daw)]})),
        ("tracks_add", sd(json!({"paths": [q(&daw.join("extra.wav"))]}))),
        ("tracks_remove", sd(json!({"names": ["extra"]}))),
        ("peaks_build_start", sd(json!({}))),
        ("peaks_build_status", sd(json!({}))),
        ("job_status", sd(json!({}))),
        ("job_cancel", sd(json!({}))),
        (
            "project_relink",
            sd(json!({"track": k, "path": q(&daw.join("k in.wav"))})),
        ),
        ("relink_search", sd(json!({"folder": q(&daw)}))),
        ("legacy_list", json!({"folder": q(&root)})),
        ("project_save", sd(json!({}))),
        ("project_clean_check", sd(json!({}))),
        ("export_dest_set", sd(json!({}))),
        ("export_markers_set", sd(json!({"on": true}))),
        ("export_compensate_set", sd(json!({"compensate": false}))),
        ("export_last_get", sd(json!({}))),
        ("export_last_set", sd(json!({"formats": ["midi"]}))),
        ("project_consolidate", sd(json!({}))),
        (
            "project_save_as",
            sd(json!({"name": "Copy", "dir": q(&root.join("out"))})),
        ),
        ("project_import", json!({"session_dir": q(&root.join("no session"))})),
        ("project_new", json!({"name": "N1", "dir": q(&root.join("np"))})),
        ("project_open", json!({"path": q(&dir)})),
        ("markers_get", sd(json!({}))),
        (
            "markers_set",
            sd(json!({"markers": [{"t": 1.5, "note": "check"}, {"t": 0.5}]})),
        ),
        ("markers_set", sd(json!({"markers": []}))),
        ("markers_get", sd(json!({}))),
        // #167: per-track lane heights (set, get back, clear, get back)
        ("row_heights_set", sd(json!({"heights": {"kick": 1.5}}))),
        ("row_heights_get", sd(json!({}))),
        ("row_heights_set", sd(json!({"heights": {}}))),
        ("row_heights_get", sd(json!({}))),
        ("project_close", sd(json!({"force": true}))),
    ];
    // every Agent row is in the table (so a new row needs a call here), and `job_get` is in the loop below
    let have: BTreeSet<&str> = calls.iter().map(|c| c.0).collect();
    let agent: BTreeSet<&str> = REGISTRY
        .iter()
        .filter(|c| c.surface == Surface::Agent)
        .map(|c| c.name)
        .collect();
    assert_eq!(
        agent.difference(&have).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "a row without a call"
    );
    let may_fail = [
        "track_repick",
        "tracks_add",
        "project_relink",
        "project_consolidate",
        "project_open",
        "project_close",
        "project_import",
    ];
    let (mut id, mut ok_by_tool): (u64, BTreeSet<String>) = (10, BTreeSet::new());
    let mut check = |name: &str, res: &Value, id: u64| {
        let r = &res["result"];
        assert!(res.get("error").is_none(), "{name}: a JSON-RPC error: {res}");
        if r["isError"] == true {
            assert!(may_fail.contains(&name), "{name}: {r}");
            return;
        }
        let body: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        if let Some(sc) = r.get("structuredContent") {
            assert_eq!(sc, &body, "{name}");
        }
        let schema = schemas.get(name).unwrap_or_else(|| panic!("{name}: not listed"));
        if !schema.is_null() {
            validate(schema, &body, name).unwrap_or_else(|e| panic!("{name} (call {id}): {e}"));
        }
        ok_by_tool.insert(name.to_string());
    };
    for (name, args) in std::mem::take(&mut calls) {
        id += 1;
        let res = mcp_once(&svc, &jobs, id, "tools/call", json!({"name": name, "arguments": args}));
        check(name, &res, id);
        let body = res["result"]["content"][0]["text"]
            .as_str()
            .and_then(|t| serde_json::from_str::<Value>(t).ok());
        if let Some(job) = body
            .as_ref()
            .and_then(|b| b["job"].as_str())
            .filter(|_| name != "job_cancel")
        {
            // a Progress tool: poll job_get (its answers validate too) until the job ends
            loop {
                id += 1;
                let g = mcp_once(
                    &svc,
                    &jobs,
                    id,
                    "tools/call",
                    json!({"name": "job_get", "arguments": {"job": job}}),
                );
                check("job_get", &g, id);
                let st = serde_json::from_str::<Value>(g["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
                if st["state"] != "running" {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        if name == "analyze_start" {
            // one analyze_status call per iteration: a job that moves queued -> running
            // between two calls in one condition exits with the state still `running`
            let mut s = svc.analyze_status(&sid)["state"].clone();
            while s == "running" || s == "queued" {
                std::thread::sleep(std::time::Duration::from_millis(10));
                s = svc.analyze_status(&sid)["state"].clone();
            }
        }
    }
    for must in [
        "export_all",
        "job_get",
        "events_edit",
        "track_velocity_set",
        "project_save_as",
        "tracks_get",
    ] {
        assert!(ok_by_tool.contains(must), "{must} never answered without an error");
    }
    // the Progress tools advertise the job answer, not the final result
    for n in ["export_all", "tracks_add", "project_save_as", "project_consolidate"] {
        assert_eq!(schemas[n]["required"], json!(["job"]), "{n}");
    }
    // every listed tool dispatches (an error result is fine, a JSON-RPC error is not)
    for name in schemas.keys() {
        id += 1;
        let res = mcp_once(&svc, &jobs, id, "tools/call", json!({"name": name, "arguments": {}}));
        assert!(res.get("error").is_none() && res["result"].is_object(), "{name}: {res}");
    }
    // every Agent row runs in `drumspike batch`
    let steps: String = agent
        .iter()
        .map(|n| json!({"command": n, "args": {}}).to_string() + "\n")
        .collect();
    let out = SharedBuf(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
    let mut io = cli::Io {
        input: Box::new(std::io::Cursor::new(steps.into_bytes())),
        out: Box::new(out.clone()),
        err: std::sync::Arc::new(std::sync::Mutex::new(Box::new(std::io::sink()))),
    };
    assert_eq!(cli::run(&svc, &["batch".into()], &mut io), 0);
    let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), agent.len());
    for l in &lines {
        let msg = l["error"]["message"].as_str().unwrap_or("");
        assert!(!msg.starts_with("unknown command"), "{l}");
    }
    let _ = (dir, svc);
}
