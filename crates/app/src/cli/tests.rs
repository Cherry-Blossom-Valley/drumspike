use super::*;

/// A shared buffer as a writer.
#[derive(Clone, Default)]
pub struct Buf(pub Arc<Mutex<Vec<u8>>>);
impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Buf {
    pub fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn io(input: &str) -> (Io, Buf, Buf) {
    let (out, err) = (Buf::default(), Buf::default());
    (
        Io {
            input: Box::new(std::io::Cursor::new(input.as_bytes().to_vec())),
            out: Box::new(out.clone()),
            err: Arc::new(Mutex::new(Box::new(err.clone()))),
        },
        out,
        err,
    )
}

fn svc() -> Service {
    Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    })
}

fn args(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

#[test]
fn exit_codes_follow_the_error_codes() {
    assert_eq!(exit_code(&AppError::bad("x")), 2);
    assert_eq!(exit_code(&AppError::bad("unknown track")), 3);
    assert_eq!(exit_code(&AppError::bad("unknown track zz")), 3);
    assert_eq!(exit_code(&AppError::not_found("x")), 3);
    assert_eq!(exit_code(&AppError::conflict("x")), 4);
    assert_eq!(exit_code(&AppError::internal("x")), 1);
}

#[test]
fn help_lists_every_agent_row_and_one_row_in_full() {
    let all = help_text(None).unwrap();
    for c in api::agent_commands() {
        assert!(all.contains(c.name), "{}", c.name);
    }
    let one = help_text(Some("events_edit")).unwrap();
    assert!(one.contains("--ops") && one.contains("(required)") && one.contains("result:"));
    assert_eq!(help_text(Some("transport_play")).unwrap_err().code, 404);
    let (mut i, out, _) = io("");
    assert_eq!(run(&svc(), &args("help project_open"), &mut i), 0);
    assert!(out.text().contains("--read-only"));
}

#[test]
fn commands_json_has_a_row_per_agent_command() {
    let v = commands_json();
    assert_eq!(v["commands"].as_array().unwrap().len(), api::agent_commands().count());
    assert_eq!(v["commands"][0]["inputSchema"]["type"], "object");
}

#[test]
fn flags_are_typed_by_the_row() {
    let row = api::find("track_velocity_set").unwrap();
    let (mut i, _, _) = io("");
    let p = parse_flags(
        row,
        &args("--sid s --name k --lo_db -40 --hi-db -6.5 --cut-db -50"),
        &mut i,
    )
    .unwrap();
    assert_eq!(p.args["lo_db"], -40.0);
    assert_eq!(p.args["hi_db"], -6.5);
    assert_eq!(p.args["sid"], "s");
    let row = api::find("project_close").unwrap();
    let p = parse_flags(row, &args("--sid s --force --pretty"), &mut i).unwrap();
    assert_eq!(p.args["force"], true);
    assert!(p.pretty && p.save);
    let row = api::find("analyze_start").unwrap();
    let p = parse_flags(
        row,
        &args("--sid s --tracks a --tracks b --keep-edited false --args {\"params\":{}}"),
        &mut i,
    )
    .unwrap();
    assert_eq!(p.args["tracks"], json!(["a", "b"]));
    assert_eq!(p.args["keep_edited"], false);
    assert_eq!(p.args["params"], json!({}));
    for bad in ["--nope 1", "--sid", "sid s", "--lo_db x"] {
        let row = api::find("track_velocity_set").unwrap();
        assert_eq!(
            parse_flags(row, &args(bad), &mut i).err().map(|e| e.code),
            Some(400),
            "{bad}"
        );
    }
}

#[test]
fn a_one_shot_command_prints_json_and_the_exit_code_for_an_error() {
    let s = svc();
    let (mut i, out, err) = io("");
    assert_eq!(run(&s, &args("project_list"), &mut i), 0);
    assert_eq!(out.text().trim(), "[]");
    let (mut i, out, err2) = io("");
    assert_eq!(run(&s, &args("project_get --sid zz"), &mut i), 3);
    assert!(out.text().is_empty());
    assert!(err2.text().contains("\"code\":404"), "{}", err2.text());
    let (mut i, _, err3) = io("");
    assert_eq!(
        run(&s, &args("nope"), &mut i),
        3,
        "an unknown command is not found, like `help nope`"
    );
    assert!(err3.text().contains("unknown command nope"));
    let (mut i, _, _) = io("");
    assert_eq!(run(&s, &args("help nope"), &mut i), 3);
    let (mut i, _, _) = io("");
    assert_eq!(run(&s, &args("project_get"), &mut i), 2);
    assert!(err.text().is_empty());
    let (mut i, _, _) = io("");
    assert_eq!(run(&s, &[], &mut i), 2);
}

#[test]
fn batch_answers_a_line_per_step_and_substitutes_the_sid() {
    let s = svc();
    let dir = std::env::temp_dir().join(format!("ds_cli_batch_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    s.set_home(dir.join("home"));
    let script = format!(
        "{}\n{}\nnot json\n{}\n",
        json!({"command": "project_new", "args": {"name": "B", "dir": dir.display().to_string()}}),
        json!({"command": "project_rename", "args": {"sid": "$sid", "name": "B2"}}),
        json!({"command": "project_get", "args": {"sid": "nope"}}),
    );
    let (mut i, out, _) = io(&script);
    assert_eq!(run(&s, &args("batch"), &mut i), 0);
    let lines: Vec<Value> = out.text().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0]["ok"], true);
    assert_eq!(lines[1]["result"]["name"], "B2");
    assert_eq!(lines[2]["error"]["code"], 400);
    assert_eq!(lines[3]["error"]["code"], 404);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_stops_at_the_first_error_with_its_exit_code() {
    let s = svc();
    let path = std::env::temp_dir().join(format!("ds_cli_run_{}.json", std::process::id()));
    std::fs::write(
        &path,
        json!([{"cmd": "project_list"}, {"cmd": "project_get", "args": {"sid": "x"}}, {"cmd": "project_list"}])
            .to_string(),
    )
    .unwrap();
    let (mut i, out, _) = io("");
    assert_eq!(run(&s, &["run".into(), path.display().to_string()], &mut i), 3);
    assert_eq!(out.text().lines().count(), 2);
    let (mut i, _, _) = io("");
    assert_eq!(run(&s, &args("run /nonexistent.json"), &mut i), 3);
    let _ = std::fs::remove_file(path);
}
