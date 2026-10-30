use super::*;

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);
impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn talk(lines: &[Value]) -> Vec<Value> {
    let svc = Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    let input = lines.iter().map(|l| format!("{l}\n")).collect::<String>();
    let buf = Buf::default();
    serve(&svc, std::io::Cursor::new(input.into_bytes()), Box::new(buf.clone()));
    let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    text.lines()
        .map(|l| serde_json::from_str(l).expect("only JSON on stdout"))
        .collect()
}

#[test]
fn initialize_list_and_call_follow_the_protocol() {
    let out = talk(&[
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "project_list", "arguments": {}}}),
        json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "project_get", "arguments": {"sid": "zz"}}}),
        json!({"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {"name": "nope"}}),
        json!({"jsonrpc": "2.0", "id": 7, "method": "resources/list"}),
        json!({"jsonrpc": "2.0", "id": 8, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}),
    ]);
    let by = |id: i64| out.iter().find(|m| m["id"] == id).unwrap();
    assert_eq!(out.len(), 8, "the notification has no answer");
    assert_eq!(by(1)["result"]["protocolVersion"], "2025-03-26");
    assert!(by(1)["result"]["capabilities"]["tools"].is_object());
    assert_eq!(by(8)["result"]["protocolVersion"], VERSIONS[0]);
    assert_eq!(by(2)["result"], json!({}));
    let names: Vec<&str> = by(3)["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for c in api::agent_commands() {
        assert!(names.contains(&c.name), "{}", c.name);
    }
    assert!(names.contains(&"job_get"));
    assert_eq!(by(4)["result"]["isError"], false);
    assert_eq!(by(4)["result"]["content"][0]["text"], "[]");
    assert_eq!(by(5)["result"]["isError"], true);
    assert!(by(5)["result"]["content"][0]["text"].as_str().unwrap().contains("404"));
    assert_eq!(by(6)["error"]["code"], -32602);
    assert_eq!(by(7)["error"]["code"], -32601);
}

#[test]
fn bad_input_is_a_json_rpc_error_not_silence() {
    let svc = Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    let buf = Buf::default();
    serve(
        &svc,
        std::io::Cursor::new(b"not json\n[1]\n\n".to_vec()),
        Box::new(buf.clone()),
    );
    let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    let codes: Vec<i64> = text
        .lines()
        .map(|l| {
            serde_json::from_str::<Value>(l).unwrap()["error"]["code"]
                .as_i64()
                .unwrap()
        })
        .collect();
    assert_eq!(codes, [-32700, -32600]);
}

#[test]
fn tool_descriptions_say_what_the_agent_must_know() {
    let t = |n: &str| tool(api::find(n).unwrap());
    assert!(
        t("events_edit")["description"]
            .as_str()
            .unwrap()
            .contains("call project_save")
    );
    assert!(
        t("analyze_start")["description"]
            .as_str()
            .unwrap()
            .contains("poll analyze_status")
    );
    assert!(t("export_all")["description"].as_str().unwrap().contains("job_get"));
    assert!(
        !t("project_get")["description"]
            .as_str()
            .unwrap()
            .contains("project_save")
    );
    assert_eq!(t("project_get")["annotations"]["readOnlyHint"], true);
    assert_eq!(t("events_edit")["annotations"]["readOnlyHint"], false);
    assert!(
        t("project_list").get("outputSchema").is_none(),
        "an array result has no outputSchema"
    );
    assert!(t("project_get")["outputSchema"]["required"].is_array());
    for c in api::agent_commands() {
        if needs_save(c) {
            assert!(describe(c).contains("project_save"), "{}", c.name);
        }
    }
}

#[test]
fn a_progress_tool_starts_a_job_and_job_get_reports_it() {
    let dir = std::env::temp_dir().join(format!("ds_mcp_job_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let svc = Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    svc.set_home(dir.join("home"));
    let sid = svc
        .project_new(Some("J"), Some(&dir.display().to_string()), None)
        .unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let jobs = Jobs::default();
    let r = run_tool(&svc, &jobs, "export_all", &json!({"sid": sid})).unwrap();
    let id = r["job"].as_str().unwrap().to_string();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let st = loop {
        let st = run_tool(&svc, &jobs, JOB_GET, &json!({"job": id})).unwrap();
        if st["state"] != "running" || std::time::Instant::now() > end {
            break st;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(st["state"], "done", "{st}");
    assert_eq!(st["result"]["midi"], 0);
    assert_eq!(
        run_tool(&svc, &jobs, JOB_GET, &json!({"job": "j99"})).unwrap_err().code,
        404
    );
    assert_eq!(run_tool(&svc, &jobs, "export_all", &json!({})).unwrap_err().code, 400);
    let _ = std::fs::remove_dir_all(&dir);
}

/// #388: the MCP tool schema of `analyze_start` names the `detector` flag.
#[test]
fn the_analyze_start_schema_names_the_detector_flag() {
    let tools = tools();
    let t = tools.iter().find(|t| t["name"] == "analyze_start").expect("the row");
    let props = t["inputSchema"]["properties"].as_object().expect("properties");
    assert!(props.contains_key("detector"), "{props:?}");
    assert!(props["detector"]["description"].as_str().unwrap().contains("builtin"));
}
