use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::ServiceConfig;

fn svc() -> Service {
    Service::new(ServiceConfig::default())
}

/// A stand-in for the page: polls for calls, answers each with `answer(fn, args)`, until `stop` is set.
fn page(
    svc: &Service,
    answer: impl Fn(&str, &Value) -> Result<Value, String> + Send + 'static,
) -> (Arc<std::sync::atomic::AtomicBool>, std::thread::JoinHandle<()>) {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (s, st) = (svc.clone(), stop.clone());
    let h = std::thread::spawn(move || {
        while !st.load(std::sync::atomic::Ordering::Relaxed) {
            let r = window_call_next(&s, &json!({"timeout_ms": 100})).unwrap();
            let c = &r["call"];
            if c.is_null() {
                continue;
            }
            let res = answer(c["fn"].as_str().unwrap(), &c["args"]);
            let body = match res {
                Ok(v) => json!({"id": c["id"], "ok": true, "value": v}),
                Err(e) => json!({"id": c["id"], "ok": false, "error": e}),
            };
            window_call_result(&s, &body).unwrap();
        }
    });
    // a call before the first poll is rightly refused ("no page is listening"), so hand back only a listening page
    let t0 = Instant::now();
    while !svc.window_relay().listening() {
        assert!(t0.elapsed() < Duration::from_secs(5), "the stand-in page never polled");
        std::thread::sleep(Duration::from_millis(5));
    }
    (stop, h)
}

fn stop(p: (Arc<std::sync::atomic::AtomicBool>, std::thread::JoinHandle<()>)) {
    p.0.store(true, std::sync::atomic::Ordering::Relaxed);
    p.1.join().unwrap();
}

fn request(token: Option<&str>, method: &str, url: &str, body: &str) -> HttpRequest {
    HttpRequest {
        method: method.into(),
        url: url.into(),
        body: body.as_bytes().to_vec(),
        close: true,
        authorization: token.map(|t| format!("Bearer {t}")),
    }
}

fn body_of(r: &HttpReply) -> Value {
    serde_json::from_slice(&r.body).unwrap()
}

#[test]
fn a_call_without_the_token_or_with_a_wrong_one_is_refused_before_it_reaches_the_page() {
    let s = svc();
    s.window_relay().enable();
    let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let n = seen.clone();
    let p = page(&s, move |_, _| {
        n.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(json!([]))
    });
    let call = r#"{"fn":"hits","args":["Kick In"],"timeout_ms":500}"#;
    for token in [None, Some("wrong"), Some("")] {
        let r = handle(&s, "secret", &request(token, "POST", "/call", call));
        assert_eq!(r.status, 401, "{token:?}");
        assert_eq!(body_of(&r)["code"], 401);
    }
    let mut bare = request(None, "POST", "/call", call);
    bare.authorization = Some("secret".into()); // the token without the scheme is no bearer token
    assert_eq!(handle(&s, "secret", &bare).status, 401);
    assert_eq!(handle(&s, "secret", &request(None, "GET", "/status", "")).status, 401);
    assert_eq!(
        seen.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "the page saw no call"
    );
    let ok = handle(&s, "secret", &request(Some("secret"), "POST", "/call", call));
    assert_eq!(ok.status, 200, "{}", String::from_utf8_lossy(&ok.body));
    assert_eq!(seen.load(std::sync::atomic::Ordering::Relaxed), 1);
    stop(p);
}

#[test]
fn hits_through_the_endpoint_equal_the_pages_own_answer() {
    let s = svc();
    s.window_relay().enable();
    let hits =
        json!([{"index": 0, "t": 0.5, "sample": 24000, "v": 100}, {"index": 1, "t": 1.5, "sample": 72000, "v": 90}]);
    let own = hits.clone();
    let p = page(&s, move |f, a| {
        assert_eq!((f, a), ("hits", &json!(["Kick In"])));
        Ok(own.clone())
    });
    let r = handle(
        &s,
        "t",
        &request(Some("t"), "POST", "/call", r#"{"fn":"hits","args":["Kick In"]}"#),
    );
    assert_eq!(r.status, 200);
    assert_eq!(body_of(&r), json!({"ok": true, "value": hits}));
    stop(p);
}

#[test]
fn the_page_error_comes_back_as_a_409_with_its_message() {
    let s = svc();
    s.window_relay().enable();
    let p = page(&s, |_, _| {
        Err("Training Mode is off: turn it on first with setTraining(true)".into())
    });
    let r = handle(
        &s,
        "t",
        &request(Some("t"), "POST", "/call", r#"{"fn":"move","args":["Kick In",1,2]}"#),
    );
    assert_eq!(r.status, 409);
    assert!(
        body_of(&r)["message"]
            .as_str()
            .unwrap()
            .contains("Training Mode is off")
    );
    stop(p);
}

#[test]
fn only_the_named_helpers_pass_and_a_missing_page_or_a_silent_one_is_an_error() {
    let s = svc();
    let h = |body: &str| handle(&s, "t", &request(Some("t"), "POST", "/call", body));
    // no endpoint open, no page: nothing to call
    assert_eq!(h(r#"{"fn":"hits","args":[]}"#).status, 409);
    s.window_relay().enable();
    assert_eq!(
        h(r#"{"fn":"hits","args":[]}"#).status,
        409,
        "enabled, but no page has polled"
    );
    assert_eq!(
        h(r#"{"fn":"eval","args":["1"]}"#).status,
        400,
        "a name outside the helper list"
    );
    assert_eq!(h(r#"{"fn":"hits","args":"x"}"#).status, 400);
    assert_eq!(h(r#"{"args":[]}"#).status, 400);
    assert_eq!(h("not json").status, 400);
    assert_eq!(h(r#"{"fn":"hits","timeout_ms":"x"}"#).status, 400);
    assert_eq!(handle(&s, "t", &request(Some("t"), "GET", "/nope", "")).status, 404);
    // a page that polls but never answers: the call ends with 504 after its wait
    let silent = std::thread::spawn({
        let s = s.clone();
        move || window_call_next(&s, &json!({"timeout_ms": 1000})).unwrap()
    });
    std::thread::sleep(Duration::from_millis(50));
    let r = h(r#"{"fn":"look","args":["Kick In",0],"timeout_ms":100}"#);
    assert_eq!(r.status, 504, "{}", String::from_utf8_lossy(&r.body));
    assert!(!silent.join().unwrap()["call"].is_null());
    // its late answer finds no call that waits
    assert_eq!(
        window_call_result(&s, &json!({"id": 1, "ok": true, "value": 1}))
            .unwrap_err()
            .code,
        404
    );
}

#[test]
fn the_pages_commands_check_their_arguments_and_a_closed_endpoint_asks_the_page_to_stop() {
    let s = svc();
    assert_eq!(
        window_call_next(&s, &json!({"timeout_ms": 5})).unwrap(),
        json!({"enabled": false, "call": null})
    );
    s.window_relay().enable();
    assert_eq!(
        window_call_next(&s, &json!({"timeout_ms": 5})).unwrap(),
        json!({"enabled": true, "call": null})
    );
    assert_eq!(window_call_next(&s, &json!({"timeout_ms": "x"})).unwrap_err().code, 400);
    assert_eq!(window_call_result(&s, &json!({"ok": true})).unwrap_err().code, 400);
    assert_eq!(window_call_result(&s, &json!({"id": 1})).unwrap_err().code, 400);
    // the page's long poll is a command of the registry, on the window surface only
    for name in ["window_call_next", "window_call_result"] {
        let row = crate::api::find(name).unwrap();
        assert_eq!(row.surface, crate::api::Surface::Window, "{name}");
    }
}

#[test]
fn a_newer_page_ends_the_hanging_poll_of_a_closed_one_so_it_swallows_no_call() {
    let s = svc();
    s.window_relay().enable();
    let old = {
        let s = s.clone();
        std::thread::spawn(move || s.window_relay().next("old", Duration::from_secs(10)))
    };
    let t0 = Instant::now();
    while !s.window_relay().listening() {
        assert!(t0.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    let new = {
        let s = s.clone();
        std::thread::spawn(move || s.window_relay().next("new", Duration::from_secs(10)))
    };
    assert!(
        old.join().unwrap().is_none(),
        "the old poll ended when the new page polled"
    );
    let s2 = s.clone();
    let caller = std::thread::spawn(move || s2.window_relay().call("status", &json!([]), Duration::from_secs(5)));
    let got = new.join().unwrap().expect("the new page got the call");
    window_call_result(&s, &json!({"id": got["id"], "ok": true, "value": 7})).unwrap();
    assert_eq!(caller.join().unwrap().unwrap(), json!(7));
}

fn http(port: u16, token: Option<&str>, method: &str, path: &str, body: &str) -> (u16, Value) {
    let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    write!(
        c,
        "{method} {path} HTTP/1.1\r\nHost: x\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut raw = String::new();
    c.read_to_string(&mut raw).unwrap();
    let status = raw.split_whitespace().nth(1).unwrap().parse().unwrap();
    let json = raw
        .split_once("\r\n\r\n")
        .map_or(Value::Null, |(_, b)| serde_json::from_str(b).unwrap_or(Value::Null));
    (status, json)
}

#[test]
fn the_endpoint_listens_on_loopback_names_itself_in_a_private_file_and_needs_its_token() {
    let s = svc();
    let home = std::env::temp_dir().join(format!("ds_window_ep_{}", std::process::id()));
    let ep = start(&s, &home).unwrap();
    let file: Value = serde_json::from_slice(&std::fs::read(home.join(ENDPOINT_FILE)).unwrap()).unwrap();
    assert_eq!(file["port"], ep.port);
    assert_eq!(file["token"], ep.token.as_str());
    assert_eq!(file["pid"], std::process::id());
    assert_eq!(ep.token.len(), 64);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(home.join(ENDPOINT_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "only the owner reads the token");
    }
    let call = r#"{"fn":"status","args":[],"timeout_ms":2000}"#;
    assert_eq!(http(ep.port, None, "POST", "/call", call).0, 401);
    assert_eq!(http(ep.port, Some("nope"), "POST", "/call", call).0, 401);
    let (code, st) = http(ep.port, Some(&ep.token), "GET", "/status", "");
    assert_eq!((code, st["listening"].as_bool()), (200, Some(false)));
    let p = page(&s, |f, _| Ok(json!({"training": f == "status", "records": 0})));
    std::thread::sleep(Duration::from_millis(150));
    let (code, out) = http(ep.port, Some(&ep.token), "POST", "/call", call);
    assert_eq!(code, 200, "{out}");
    assert_eq!(out, json!({"ok": true, "value": {"training": true, "records": 0}}));
    assert_eq!(
        http(ep.port, Some(&ep.token), "GET", "/status", "").1["listening"],
        true
    );
    stop(p);
    ep.remove_file();
    assert!(!home.join(ENDPOINT_FILE).exists());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn two_tokens_differ_and_compare_exactly() {
    assert_ne!(new_token().unwrap(), new_token().unwrap());
    assert!(same_token("abc", "abc"));
    assert!(!same_token("abc", "abd"));
    assert!(!same_token("abc", "abcd"));
}

type Page = (Arc<std::sync::atomic::AtomicBool>, std::thread::JoinHandle<()>);

/// A service with a window endpoint in its own home folder and a stand-in page that echoes `[fn, args]`.
fn open_window(tag: &str) -> (Service, PathBuf, Endpoint, Page) {
    let s = svc();
    let home = std::env::temp_dir().join(format!("ds_agent_call_{tag}_{}", std::process::id()));
    s.set_home(&home);
    let ep = start(&s, &home).unwrap();
    let p = page(&s, |f, a| {
        if f == "undo" {
            Err("Training Mode is off".into())
        } else {
            Ok(json!([f, a]))
        }
    });
    (s, home, ep, p)
}

#[derive(Clone, Default)]
struct Sink(Arc<std::sync::Mutex<Vec<u8>>>);
impl Write for Sink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Sink {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

#[test]
fn agent_call_reads_the_endpoint_file_and_answers_the_helpers_value_or_a_plain_error() {
    let (s, home, ep, p) = open_window("direct");
    let v = agent_call(s.home().as_deref(), "hits", &json!(["Kick", 1]), Some(5000)).unwrap();
    assert_eq!(v, json!(["hits", ["Kick", 1]]));
    let e = agent_call(s.home().as_deref(), "undo", &json!([]), Some(5000)).unwrap_err();
    assert_eq!((e.code, e.message.as_str()), (409, "Training Mode is off"));
    assert_eq!(
        agent_call(s.home().as_deref(), "eval", &json!([]), None)
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        agent_call_args(s.home().as_deref(), &json!({"args": []}))
            .unwrap_err()
            .code,
        400,
        "fn is required"
    );
    stop(p);
    // the app is gone (file removed) or died (file left, port closed): the same plain error
    ep.remove_file();
    let e = agent_call(Some(&home), "hits", &json!([]), None).unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.contains("no Drum Spike window is open"), "{}", e.message);
    assert_eq!(agent_call(None, "hits", &json!([]), None).unwrap_err().code, 409);
    std::fs::write(home.join(ENDPOINT_FILE), r#"{"port":1,"token":"x"}"#).unwrap();
    assert_eq!(agent_call(Some(&home), "hits", &json!([]), None).unwrap_err().code, 409);
    let _ = std::fs::remove_dir_all(&home);
}

fn mcp_text(s: &Service, lines: &[Value]) -> Vec<Value> {
    let input = lines.iter().map(|l| format!("{l}\n")).collect::<String>();
    let out = Sink::default();
    crate::mcp::serve(s, std::io::Cursor::new(input.into_bytes()), Box::new(out.clone()));
    out.text().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn the_mcp_tool_agent_call_reaches_the_window_and_says_so_when_none_is_open() {
    let (s, home, ep, p) = open_window("mcp");
    let call = |id: i64, args: Value| json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": "agent_call", "arguments": args}});
    let msgs = mcp_text(
        &s,
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            call(2, json!({"fn": "move", "args": ["Snare", 3, 0.5]})),
            call(3, json!({"fn": "undo"})),
        ],
    );
    let by = |id: i64| msgs.iter().find(|m| m["id"] == id).unwrap();
    assert!(
        by(1)["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "agent_call")
    );
    assert_eq!(by(2)["result"]["isError"], false);
    assert_eq!(
        by(2)["result"]["content"][0]["text"],
        json!(["move", ["Snare", 3, 0.5]]).to_string()
    );
    assert_eq!(by(3)["result"]["isError"], true);
    assert!(
        by(3)["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Training Mode is off")
    );
    stop(p);
    ep.remove_file();
    let msgs = mcp_text(&s, &[call(4, json!({"fn": "hits"}))]);
    assert_eq!(msgs[0]["result"]["isError"], true);
    assert!(
        msgs[0]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no Drum Spike window is open")
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn cli_agent_call_prints_the_helpers_value_and_exits_4_without_a_window() {
    let (s, home, ep, p) = open_window("cli");
    let run = |a: &[&str]| {
        let (out, err) = (Sink::default(), Sink::default());
        let mut io = crate::cli::Io {
            input: Box::new(std::io::Cursor::new(Vec::new())),
            out: Box::new(out.clone()),
            err: Arc::new(std::sync::Mutex::new(Box::new(err.clone()))),
        };
        let args: Vec<String> = a.iter().map(ToString::to_string).collect();
        let code = crate::cli::run(&s, &args, &mut io);
        (code, out.text(), err.text())
    };
    let (code, out, _) = run(&["agent_call", "hits", "Kick", "2", r#"{"t":1.5}"#]);
    assert_eq!(code, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&out).unwrap(),
        json!(["hits", ["Kick", 2, {"t": 1.5}]])
    );
    assert_eq!(run(&["agent_call"]).0, 2);
    stop(p);
    ep.remove_file();
    let (code, _, err) = run(&["agent_call", "hits"]);
    assert_eq!(code, 4);
    assert!(err.contains("no Drum Spike window is open"), "{err}");
    let _ = std::fs::remove_dir_all(&home);
}
