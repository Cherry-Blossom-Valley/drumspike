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
    assert_eq!(seen.load(std::sync::atomic::Ordering::Relaxed), 0, "the page saw no call");
    let ok = handle(&s, "secret", &request(Some("secret"), "POST", "/call", call));
    assert_eq!(ok.status, 200, "{}", String::from_utf8_lossy(&ok.body));
    assert_eq!(seen.load(std::sync::atomic::Ordering::Relaxed), 1);
    stop(p);
}

#[test]
fn hits_through_the_endpoint_equal_the_pages_own_answer() {
    let s = svc();
    s.window_relay().enable();
    let hits = json!([{"index": 0, "t": 0.5, "sample": 24000, "v": 100}, {"index": 1, "t": 1.5, "sample": 72000, "v": 90}]);
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
    assert!(body_of(&r)["message"].as_str().unwrap().contains("Training Mode is off"));
    stop(p);
}

#[test]
fn only_the_named_helpers_pass_and_a_missing_page_or_a_silent_one_is_an_error() {
    let s = svc();
    let h = |body: &str| handle(&s, "t", &request(Some("t"), "POST", "/call", body));
    // no endpoint open, no page: nothing to call
    assert_eq!(h(r#"{"fn":"hits","args":[]}"#).status, 409);
    s.window_relay().enable();
    assert_eq!(h(r#"{"fn":"hits","args":[]}"#).status, 409, "enabled, but no page has polled");
    assert_eq!(h(r#"{"fn":"eval","args":["1"]}"#).status, 400, "a name outside the helper list");
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
    let json = raw.split_once("\r\n\r\n").map_or(Value::Null, |(_, b)| {
        serde_json::from_str(b).unwrap_or(Value::Null)
    });
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
        let mode = std::fs::metadata(home.join(ENDPOINT_FILE)).unwrap().permissions().mode();
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
    assert_eq!(http(ep.port, Some(&ep.token), "GET", "/status", "").1["listening"], true);
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
