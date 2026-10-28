use super::*;
use crate::ServiceConfig;

fn bridge() -> Bridge {
    // one folder per call: the tests run in parallel and write the files again
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("ds_bridge_static_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<html><head><title>x</title></head></html>").unwrap();
    std::fs::write(dir.join("a.js"), "1").unwrap();
    Bridge::new(
        Service::new(ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        }),
        dir,
    )
}

/// The bridge server on a free port, with the test static folder.
fn serve_bridge() -> u16 {
    let l = listen(0).unwrap();
    let port = l.local_addr().unwrap().port();
    let b = Arc::new(bridge());
    std::thread::spawn(move || serve(l, b));
    port
}

#[test]
fn an_idle_connection_is_closed_after_the_read_timeout() {
    let port = serve_bridge();
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(IDLE_READ_TIMEOUT * 3)).unwrap();
    let t0 = Instant::now();
    let mut buf = [0u8; 16];
    let n = c.read(&mut buf);
    let waited = t0.elapsed();
    assert!(
        matches!(n, Ok(0)),
        "the server did not close the idle connection: {n:?} after {waited:?}"
    );
    assert!(
        waited >= IDLE_READ_TIMEOUT / 2 && waited < IDLE_READ_TIMEOUT * 2,
        "{waited:?}"
    );
}

#[test]
fn a_request_is_answered_while_many_connections_stay_idle() {
    let port = serve_bridge();
    // more idle connections than the 4 threads tiny_http starts with, opened in a burst like Chromium's preconnects
    let idle: Vec<_> = (0..12)
        .map(|_| std::net::TcpStream::connect(("127.0.0.1", port)).unwrap())
        .collect();
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(IDLE_READ_TIMEOUT * 3)).unwrap();
    let t0 = Instant::now();
    c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut out = String::new();
    let r = c.read_to_string(&mut out);
    assert!(
        r.is_ok() && out.starts_with("HTTP/1.1 200 OK\r\n") && out.contains("<title>x</title>"),
        "{r:?} {out:?}"
    );
    assert!(t0.elapsed() < IDLE_READ_TIMEOUT, "{:?}", t0.elapsed());
    drop(idle);
}

#[test]
fn one_connection_carries_several_requests_and_a_bad_one_gets_400() {
    let port = serve_bridge();
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(IDLE_READ_TIMEOUT * 3)).unwrap();
    c.write_all(b"GET /a.js HTTP/1.1\r\nHost: x\r\n\r\nPOST /invoke/project_list HTTP/1.1\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
        .unwrap();
    let mut out = String::new();
    c.read_to_string(&mut out).unwrap();
    assert_eq!(out.matches("HTTP/1.1 200 OK").count(), 2, "{out}");
    assert!(
        out.contains("Connection: keep-alive") && out.contains("Connection: close"),
        "{out}"
    );
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.write_all(b"NONSENSE\r\n\r\n").unwrap();
    let mut out = String::new();
    c.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 400 Bad Request"), "{out}");
}

#[test]
fn read_request_parses_the_head_and_the_body_and_refuses_the_rest() {
    let mut r = std::io::Cursor::new(
        b"POST /invoke/x?a=1 HTTP/1.1\r\nContent-Length: 3\r\nconnection: Keep-Alive\r\n\r\nabcGET / HTTP/1.0\r\n\r\n"
            .to_vec(),
    );
    let a = read_request(&mut r).unwrap().unwrap();
    assert_eq!(
        (a.method.as_str(), a.url.as_str(), a.body.as_slice(), a.close),
        ("POST", "/invoke/x?a=1", &b"abc"[..], false)
    );
    let b = read_request(&mut r).unwrap().unwrap();
    assert_eq!((b.method.as_str(), b.body.len(), b.close), ("GET", 0, true));
    assert_eq!(read_request(&mut r).unwrap(), None);
    for bad_head in [
        &b"GET /\r\n\r\n"[..],
        b"GET / SPDY/3\r\n\r\n",
        b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
        b"POST / HTTP/1.1\r\nContent-Length: x\r\n\r\n",
        b"POST / HTTP/1.1\r\nContent-Length: 999999999999\r\n\r\n",
        b"GET / HTTP/1.1\r\nno colon\r\n\r\n",
        b"GET / HTTP/1.1\r\nHost: x\r\n",
        b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\nab",
        b"GET / HTTP/1.1\r\nHost: \xff\r\n\r\n",
    ] {
        assert!(
            read_request(&mut std::io::Cursor::new(bad_head.to_vec())).is_err(),
            "{:?}",
            String::from_utf8_lossy(bad_head)
        );
    }
    let long = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(20_000));
    assert!(read_request(&mut std::io::Cursor::new(long.into_bytes())).is_err());
    let many = format!("GET / HTTP/1.1\r\n{}\r\n", "X: y\r\n".repeat(101));
    assert!(read_request(&mut std::io::Cursor::new(many.into_bytes())).is_err());
}

#[test]
fn write_reply_sends_the_length_and_the_connection_header() {
    let mut out = Vec::new();
    write_reply(&mut out, &HttpReply::text(404, "no"), true).unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(
        s.starts_with("HTTP/1.1 404 Not Found\r\n") && s.contains("Content-Length: 2\r\n"),
        "{s}"
    );
    assert!(
        s.contains("Connection: close\r\n") && s.contains("Cache-Control: no-store") && s.ends_with("\r\n\r\nno"),
        "{s}"
    );
    for (code, reason) in [
        (200, "OK"),
        (400, "Bad Request"),
        (409, "Conflict"),
        (500, "Internal Server Error"),
        (418, "Status"),
    ] {
        let mut out = Vec::new();
        write_reply(&mut out, &HttpReply::text(code, ""), false).unwrap();
        assert!(String::from_utf8(out)
            .unwrap()
            .starts_with(&format!("HTTP/1.1 {code} {reason}\r\n")));
    }
}

#[test]
fn the_event_log_keeps_entries_for_every_reader() {
    let log = EventLog::default();
    assert_eq!(log.head(), 0);
    assert_eq!(log.poll(0, Duration::from_millis(10)), json!({"seq": 0, "events": []}));
    log.push("a", json!(1));
    log.push("b", json!(2));
    assert_eq!(log.head(), 2);
    let first = log.poll(0, Duration::from_millis(10));
    assert_eq!(
        first,
        log.poll(0, Duration::from_millis(10)),
        "a read takes nothing away"
    );
    assert_eq!(first["events"][1], json!({"event": "b", "payload": 2}));
    assert_eq!(
        log.poll(1, Duration::from_millis(10))["events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(log.poll(2, Duration::from_millis(10))["events"], json!([]));
    for k in 0..QUEUE_CAP + 5 {
        log.push("c", json!(k));
    }
    assert_eq!(
        log.poll(0, Duration::from_millis(10))["events"]
            .as_array()
            .unwrap()
            .len(),
        QUEUE_CAP
    );
}

#[test]
fn channels_poll_waits_for_a_push_and_times_out_empty() {
    let c = Arc::new(Channels::default());
    assert!(c.poll("a", Duration::from_millis(20)).is_empty());
    let c2 = c.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        c2.push("a", json!(1));
        c2.push("a", json!(2));
    });
    let got = c.poll("a", Duration::from_secs(5));
    t.join().unwrap();
    assert_eq!(got[0], json!(1));
    assert!(
        c.poll("b", Duration::from_millis(5)).is_empty(),
        "another id stays empty"
    );
}

#[test]
fn channels_push_drops_the_oldest_over_the_cap() {
    let c = Channels::default();
    for i in 0..(QUEUE_CAP + 5) {
        c.push("a", json!(i));
    }
    let got = c.poll("a", Duration::from_millis(1));
    assert_eq!(got.len(), QUEUE_CAP);
    assert_eq!(got[0], json!(5));
}

#[test]
fn menu_event_goes_to_the_event_log() {
    let b = bridge();
    let head = b.events().head();
    let r = b
        .invoke("selftest_menu", &json!({"id": "recent", "path": "/x/Song", "sid": 3}))
        .unwrap();
    assert_eq!(r, Reply::Json(json!({"id": "recent", "path": "/x/Song"})));
    let got = b.events().poll(head, Duration::from_millis(10));
    assert_eq!(
        got["events"][0],
        json!({"event": "menu", "payload": {"id": "recent", "path": "/x/Song"}})
    );
    assert_eq!(
        b.menu_event(&json!({"id": "project", "sid": "s1"})).unwrap(),
        json!({"id": "project", "sid": "s1"})
    );
    // #169: an own zoom preset keeps its index
    assert_eq!(
        b.menu_event(&json!({"id": "zoom_own", "n": 2})).unwrap(),
        json!({"id": "zoom_own", "n": 2})
    );
    assert_eq!(b.invoke("selftest_menu", &json!({})).unwrap_err().code, 400);
}

#[test]
fn invoke_runs_the_commands_and_keeps_the_error_codes() {
    let b = bridge();
    assert!(matches!(b.invoke("app_info", &json!({})).unwrap(), Reply::Json(_)));
    assert_eq!(b.invoke("project_list", &json!({})).unwrap(), Reply::Json(json!([])));
    assert_eq!(b.invoke("project_get", &json!({"sid": "x"})).unwrap_err().code, 404);
    assert_eq!(b.invoke("project_get", &json!({})).unwrap_err().code, 400);
    assert_eq!(b.invoke("nope", &json!({})).unwrap_err().code, 404);
    assert_eq!(
        b.invoke("analyze_status", &json!({"sid": "x"})).unwrap(),
        Reply::Json(b.svc.analyze_status("x"))
    );
    assert_eq!(
        b.invoke("project_open", &json!({"path": "/nonexistent"}))
            .unwrap_err()
            .code,
        404
    );
    assert_eq!(
        b.invoke(
            "peaks_tile",
            &json!({"sid": "x", "name": "n", "level": -1, "from": 0, "to": 1})
        )
        .unwrap_err()
        .code,
        400
    );
    assert_eq!(
        b.invoke("analyze_start", &json!({"sid": "x", "tracks": 5}))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        b.invoke("export_midi", &json!({"sid": "x", "name": "n"}))
            .unwrap_err()
            .code,
        404
    );
}

#[test]
fn handle_maps_replies_and_errors_to_http() {
    let b = bridge();
    let r = b.handle("POST", "/invoke/project_list", b"");
    assert_eq!(
        (r.status, r.content_type, r.body.as_slice()),
        (200, "application/json", &b"[]"[..])
    );
    let r = b.handle("POST", "/invoke/project_get", br#"{"sid":"x"}"#);
    assert_eq!(r.status, 404);
    let v: Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(v["code"], 404);
    assert!(!v["message"].as_str().unwrap().is_empty());
    assert_eq!(b.handle("POST", "/invoke/project_list", b"{").status, 400);
    assert_eq!(b.handle("GET", "/invoke/project_list", b"").status, 405);
    assert_eq!(b.handle("POST", "/a.js", b"").status, 405);
}

#[test]
fn handle_serves_static_files_and_refuses_bad_paths() {
    let b = bridge();
    let r = b.handle("GET", "/", b"");
    assert_eq!(r.status, 200);
    let html = String::from_utf8(r.body).unwrap();
    assert!(html.contains(BRIDGE_FLAG) && html.starts_with("<html><head><script>"));
    assert_eq!(
        b.handle("GET", "/index.html?v=1", b"").body,
        b.handle("GET", "/", b"").body
    );
    let r = b.handle("GET", "/a.js?v=44", b"");
    assert_eq!(
        (r.status, r.content_type, r.body.as_slice()),
        (200, "text/javascript; charset=utf-8", &b"1"[..])
    );
    assert_eq!(b.handle("GET", "/missing.css", b"").status, 404);
    assert_eq!(b.handle("GET", "/../Cargo.toml", b"").status, 400);
    assert_eq!(b.handle("GET", "/a/../a.js", b"").status, 400);
}

#[test]
fn handle_channel_returns_the_queued_messages() {
    let b = bridge();
    b.channels().push("c1", json!({"k": 1}));
    let r = b.handle("GET", "/channel/c1?timeout_ms=10", b"");
    assert_eq!(serde_json::from_slice::<Value>(&r.body).unwrap(), json!([{"k": 1}]));
    let r = b.handle("GET", "/channel/c1?timeout_ms=10", b"");
    assert_eq!(r.body, b"[]");
}

#[test]
fn inject_flag_goes_after_head_or_in_front() {
    assert_eq!(inject_flag("<head>a"), format!("<head>{BRIDGE_FLAG}a"));
    assert_eq!(inject_flag("zz"), format!("{BRIDGE_FLAG}zz"));
}

#[test]
fn mime_of_knows_the_page_files() {
    assert_eq!(mime_of(Path::new("a.html")), "text/html; charset=utf-8");
    assert_eq!(mime_of(Path::new("a.css")), "text/css; charset=utf-8");
    assert_eq!(mime_of(Path::new("a.json")), "application/json");
    assert_eq!(mime_of(Path::new("a.bin")), "application/octet-stream");
    assert_eq!(mime_of(Path::new("a")), "application/octet-stream");
}
