//! The test bridge (P3-7a): the commands of [`Service`] over HTTP, for the app tests.
//!
//! `ds-bridge` (`crates/devtools/src/bin/ds-bridge.rs`) is the only user. It serves `static/` and
//! `POST /invoke/<command>` on 127.0.0.1, so the page runs in Chromium (Playwright) on both
//! hosts without a `WebView`. A command here calls the same `Service` method as the Tauri
//! wrapper of the same name in `apps/desktop/src-tauri/src/lib.rs`; only the arguments
//! come from JSON instead of from Tauri. Errors keep the server's codes as the HTTP status
//! and as the body `{code, message}`.
//!
//! Channels: Tauri sends progress on a `Channel`. The page cannot get a push from this
//! server, so an argument `{"__channel": id}` makes the bridge keep the messages in a
//! queue, and the page fetches them with `GET /channel/<id>?timeout_ms=N` (long poll).
//!
//! Debug builds only: the binary refuses to start in a release build and the Tauri bundle
//! does not contain it. Nothing here is a security boundary; the server listens on 127.0.0.1.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::reply::{Channels, QUEUE_CAP, Reply};
use crate::{AppError, Service};

/// X-3 (P3-9b): how long a connection may stay idle before the bridge closes it.
pub const IDLE_READ_TIMEOUT: Duration = Duration::from_secs(2);

/// The largest request body the bridge reads (the page posts JSON of a few MB at most).
pub const MAX_BODY: usize = 256 << 20;

/// The listening socket of `ds-bridge` on 127.0.0.1 (`port` 0: a free port).
///
/// # Errors
///
/// The [`std::io::Error`] of binding 127.0.0.1:`port`, for example when the port is in use or not permitted.
pub fn listen(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
}

/// One path as the bridge names it (`index.html`, `backend.js`; no leading `/`) and its bytes:
/// the desktop crate's resolver over the assets embedded in the binary (the Tauri asset
/// context, `frontendDist`).
pub type EmbeddedPage = Box<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// Where the static files of a bridge come from: a folder (`ds-bridge`, and the `--static`
/// override of the page mode) or the assets embedded in the binary (the page mode of the
/// shipped app, `drumspike bridge`; the bundle gains no files).
pub enum StaticSource {
    /// The files of one folder.
    Dir(PathBuf),
    /// One path as the bridge names it (`index.html`, `backend.js`; no leading `/`).
    Embedded(EmbeddedPage),
}

impl From<PathBuf> for StaticSource {
    fn from(dir: PathBuf) -> StaticSource {
        StaticSource::Dir(dir)
    }
}

/// The advisory flock of the home folder (`<home>/.drumspike.lock`, the same
/// [`crate::lock::ProjectLock`] the app uses for projects). The window takes it for client
/// `app` at startup, the page mode for client `bridge`; the page mode refuses to start while
/// the window runs on the same home. `Ok(None)`: the folder cannot hold a lock (a read-only
/// volume, a missing folder) and the caller starts unlocked. `Err(holder)`: another process
/// holds the lock; the text names its client and pid.
///
/// # Errors
///
/// A `String` naming the holder (`<client> (pid <n>)`) when another process holds the lock.
pub fn take_home_lock(dir: &Path, client: &str) -> Result<Option<crate::lock::ProjectLock>, String> {
    let _ = std::fs::create_dir_all(dir);
    match crate::lock::ProjectLock::acquire(dir, client) {
        Ok(crate::lock::Lock::Held(l)) => Ok(Some(l)),
        Ok(crate::lock::Lock::Unavailable(_)) => Ok(None),
        Err(_) => Err(match crate::lock::ProjectLock::holder_of(dir) {
            Some((pid, c)) => format!("{c} (pid {pid})"),
            None => "another process".into(),
        }),
    }
}

/// The arguments of the page mode: `drumspike bridge [--port N] [--home DIR] [--projects DIR] [--static DIR]`.
/// `port` 0 is a free port. `--static` serves a folder instead of the embedded assets (development).
pub struct PageArgs {
    pub port: u16,
    pub home: Option<PathBuf>,
    pub projects: Option<PathBuf>,
    pub static_dir: Option<PathBuf>,
}

/// Parse the arguments after `bridge`.
///
/// # Errors
///
/// A `String` naming the bad argument when an option is unknown, needs a value or has a bad value.
pub fn parse_page_args(argv: &[String]) -> Result<PageArgs, String> {
    let mut a = PageArgs {
        port: 0,
        home: None,
        projects: None,
        static_dir: None,
    };
    let mut it = argv.iter();
    while let Some(k) = it.next() {
        let v = it.next().ok_or_else(|| format!("{k} needs a value"))?;
        let val = |name: &str| v.parse().map_err(|_| format!("bad {name} {v}"));
        match k.as_str() {
            "--port" => a.port = val("port")?,
            "--home" => a.home = Some(PathBuf::from(v)),
            "--projects" => a.projects = Some(PathBuf::from(v)),
            "--static" => a.static_dir = Some(PathBuf::from(v)),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(a)
}

/// The one line the page mode prints on stdout when it listens: the URL the agent opens in a browser.
///
/// # Errors
///
/// The [`std::io::Error`] of writing to `w` (a closed stdout, for example).
pub fn print_url_line(w: &mut (impl Write + ?Sized), port: u16) -> std::io::Result<()> {
    writeln!(w, "drumspike bridge: http://127.0.0.1:{port}/")
}

/// The page mode of the shipped app (#370): a [`Service`] with the app's own home and projects
/// folders (`--home`/`--projects` override them), the invoke API of the command registry and the
/// page from `source`, on 127.0.0.1 only, no token (like the development bridge). Refuses to
/// start when another process holds the home's lock (the window, or a second bridge on the same
/// home). Prints the one URL line to `out` and serves until the process is stopped (SIGINT) or
/// the listener fails.
///
/// # Errors
///
/// A `String` when the home is in use or the port cannot be bound.
pub fn run_page_mode(args: &PageArgs, source: StaticSource, out: &mut dyn Write) -> Result<(), String> {
    let svc = crate::agent::service(Path::new(""), "bridge");
    if let Some(h) = &args.home {
        svc.set_home(h.clone());
    }
    if let Some(p) = &args.projects {
        svc.set_projects_dir(p.clone());
    }
    if let Some(home) = svc.home() {
        take_home_lock(&home, "bridge").map_err(|holder| {
            format!(
                "drumspike bridge: the home is in use by {holder}: close it there, or pass another home with --home"
            )
        })?;
    }
    let listener = listen(args.port).map_err(|e| format!("drumspike bridge: cannot listen: {e}"))?;
    let port = listener.local_addr().map_or(args.port, |a| a.port());
    let bridge = Arc::new(Bridge::with_source(svc, source));
    let _ = print_url_line(out, port);
    let _ = out.flush();
    serve(listener, bridge);
    Ok(())
}

/// X-3 (P3-9b): the HTTP/1.1 server of the bridge, one thread per connection.
///
/// Until P3-9b the bridge used `tiny_http` 0.12. Its pool reads each connection on one of its threads
/// with no read timeout and starts with 4 threads. Chromium opens up to 6 connections for the first
/// page load and leaves some idle (preconnects); when the pool queued a connection while its threads
/// waited on idle ones, the page load stalled until Chromium gave up (the `Page.goto` timeouts of G4,
/// seen as a 180 s stall with 4 threads in `recvfrom` and 6 connections open). Here every connection
/// has its own thread and a read timeout of [`IDLE_READ_TIMEOUT`], so an idle connection blocks no
/// other one and is closed after that time. Runs until the listener fails.
pub fn serve(listener: TcpListener, bridge: Arc<Bridge>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let bridge = bridge.clone();
        std::thread::spawn(move || connection(stream, &bridge));
    }
}

fn connection(stream: TcpStream, bridge: &Bridge) {
    let _ = stream.set_read_timeout(Some(IDLE_READ_TIMEOUT));
    let _ = stream.set_nodelay(true);
    let Ok(read_half) = stream.try_clone() else { return };
    let mut reader = BufReader::new(read_half);
    let mut writer = stream;
    loop {
        match read_request(&mut reader) {
            Ok(Some(req)) => {
                let reply = bridge.handle(&req.method, &req.url, &req.body);
                if write_reply(&mut writer, &reply, req.close).is_err() || req.close {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = write_reply(&mut writer, &HttpReply::text(400, &format!("bad request: {e}")), true);
                break;
            }
        }
    }
    let _ = writer.shutdown(std::net::Shutdown::Both);
}

/// One HTTP request as the bridge reads it: the method, the request target (path and query, as sent),
/// the body, and `close` when the client asked to close the connection after the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub body: Vec<u8>,
    pub close: bool,
}

fn idle(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

fn bad(msg: &str) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidData, msg.to_string())
}

/// Read one line of at most 16 KB, without the line end.
fn read_line(r: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = r.take(16 * 1024 + 2).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() != Some(&b'\n') {
        return Err(bad("line too long or cut off"));
    }
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| bad("the head is not UTF-8"))
}

/// Read the next request of a connection. `Ok(None)`: the client closed the connection, or it stayed
/// idle for the read timeout before the first byte of a request. A body needs `Content-Length`
/// (the page's `fetch` always sends it); a chunked body is refused.
///
/// # Errors
///
/// An [`std::io::Error`] of kind `InvalidData` for a request line that is not `METHOD URL HTTP/1.x`, a line over
/// 16 KB or not UTF-8, a header without `:`, a bad or over-[`MAX_BODY`] `Content-Length`, a chunked body, more
/// than 100 headers, or a head that ends early; any other read error of `r`, including an end of input inside
/// the body. A read timeout before the first byte is `Ok(None)`, not an error.
pub fn read_request(r: &mut impl BufRead) -> std::io::Result<Option<HttpRequest>> {
    let line = match read_line(r) {
        Ok(Some(l)) => l,
        Ok(None) => return Ok(None),
        Err(e) if idle(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut parts = line.split_whitespace();
    let (Some(method), Some(url), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad("bad request line"));
    };
    if !version.starts_with("HTTP/1.") {
        return Err(bad("only HTTP/1.x"));
    }
    let mut close = version == "HTTP/1.0";
    let mut length = 0usize;
    for _ in 0..100 {
        let h = read_line(r)?.ok_or_else(|| bad("the head ends early"))?;
        if h.is_empty() {
            let mut body = vec![0u8; length];
            r.read_exact(&mut body)?;
            return Ok(Some(HttpRequest {
                method: method.to_string(),
                url: url.to_string(),
                body,
                close,
            }));
        }
        let (k, v) = h.split_once(':').ok_or_else(|| bad("bad header"))?;
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_ascii_lowercase());
        match k.as_str() {
            "content-length" => {
                length = v.parse().map_err(|_| bad("bad Content-Length"))?;
                if length > MAX_BODY {
                    return Err(bad("body too large"));
                }
            }
            "transfer-encoding" if v != "identity" => return Err(bad("chunked bodies are not supported")),
            "connection" if v.contains("close") => close = true,
            "connection" if v.contains("keep-alive") => close = false,
            _ => {}
        }
    }
    Err(bad("too many headers"))
}

/// Write one answer with its length (and `Connection: close` when the connection ends after it).
///
/// # Errors
///
/// The [`std::io::Error`] of writing or flushing to `w`, for example a connection the client closed.
pub fn write_reply(w: &mut impl Write, reply: &HttpReply, close: bool) -> std::io::Result<()> {
    let reason = match reply.status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "Status",
    };
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: {}\r\n\r\n",
        reply.status,
        reply.content_type,
        reply.body.len(),
        if close { "close" } else { "keep-alive" }
    );
    w.write_all(head.as_bytes())?;
    w.write_all(&reply.body)?;
    w.flush()
}

/// An HTTP answer, independent of the HTTP crate (so the unit tests need no socket).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl HttpReply {
    fn json(status: u16, v: &Value) -> HttpReply {
        HttpReply {
            status,
            content_type: "application/json",
            body: v.to_string().into_bytes(),
        }
    }

    fn text(status: u16, msg: &str) -> HttpReply {
        HttpReply {
            status,
            content_type: "text/plain; charset=utf-8",
            body: msg.as_bytes().to_vec(),
        }
    }
}

/// The events of the service (`project-dirty`) as a numbered log. Unlike a channel, a read does
/// not take the messages away: every page asks for the entries after the number it has seen, so a
/// poll that an earlier page left open cannot steal an event from the next page.
#[derive(Default)]
pub struct EventLog {
    log: Mutex<(u64, VecDeque<(u64, Value)>)>,
    cv: Condvar,
}

impl EventLog {
    /// Add `{event, payload}` and wake the waiting polls.
    pub fn push(&self, name: &str, payload: Value) {
        let mut g = self.log.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.0 += 1;
        let n = g.0;
        if g.1.len() >= QUEUE_CAP {
            g.1.pop_front();
        }
        g.1.push_back((n, json!({"event": name, "payload": payload})));
        self.cv.notify_all();
    }

    /// The number of the newest entry (0 for an empty log).
    pub fn head(&self) -> u64 {
        self.log.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0
    }

    /// The entries after number `after`; waits up to `timeout` for the first one. Returns
    /// `{"seq": newest number, "events": [...]}`.
    pub fn poll(&self, after: u64, timeout: Duration) -> Value {
        let end = Instant::now() + timeout;
        let mut g = self.log.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if g.0 > after {
                let events: Vec<Value> = g.1.iter().filter(|(n, _)| *n > after).map(|(_, v)| v.clone()).collect();
                return json!({"seq": g.0, "events": events});
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return json!({"seq": g.0, "events": []});
            }
            g = self
                .cv
                .wait_timeout(g, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
}

/// The bridge: a [`Service`], the channels and the source of the static files.
pub struct Bridge {
    svc: Service,
    channels: Arc<Channels>,
    events: Arc<EventLog>,
    static_source: StaticSource,
}

#[cfg(debug_assertions)]
use crate::command_policy::opt_i64;
use crate::command_policy::arg_str;

/// The MIME type of a static file, by extension.
#[must_use]
pub fn mime_of(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// The flag the page reads to choose the bridge transport (`static/backend.js`).
pub const BRIDGE_FLAG: &str = "<script>window.__DS_BRIDGE__ = true;</script>";

/// `index.html` with [`BRIDGE_FLAG`] right after `<head>`.
#[must_use]
pub fn inject_flag(html: &str) -> String {
    match html.find("<head>") {
        Some(i) => format!("{}{}{}", &html[..i + 6], BRIDGE_FLAG, &html[i + 6..]),
        None => format!("{BRIDGE_FLAG}{html}"),
    }
}

/// Split `/path?query` and decode the value of `key` in the query, if any.
fn query_value(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
}

impl Bridge {
    /// A bridge over `svc` that serves the files of `static_dir`.
    pub fn new(svc: Service, static_dir: impl Into<PathBuf>) -> Bridge {
        Bridge::with_source(svc, StaticSource::Dir(static_dir.into()))
    }

    /// A bridge over `svc` that serves its static files from `source` (a folder, or the assets
    /// embedded in the binary — the page mode of the shipped app).
    #[must_use]
    pub fn with_source(svc: Service, source: StaticSource) -> Bridge {
        let channels = Arc::new(Channels::default());
        // Events (`project-dirty`) go to the log; the page long-polls `GET /events?after=N`.
        let events = Arc::new(EventLog::default());
        let sink = Arc::clone(&events);
        svc.set_emitter(Arc::new(move |name: &str, payload: Value| sink.push(name, payload)));
        Bridge {
            svc,
            channels,
            events,
            static_source: source,
        }
    }

    /// The event log (for tests and for the long poll of `GET /events`).
    #[must_use]
    pub fn events(&self) -> &Arc<EventLog> {
        &self.events
    }

    /// Put the event `menu` `{id, path?, sid?}` into the event log, as a click on a native menu item does in
    /// the app (P3-13a). Returns the payload.
    ///
    /// # Errors
    ///
    /// A 400 [`AppError`] when `args` has no string `id`.
    pub fn menu_event(&self, args: &Value) -> Result<Value, AppError> {
        let mut payload = json!({"id": arg_str(args, "id")?});
        for key in ["path", "sid", "name"] {
            if let Some(v) = args.get(key).filter(|v| v.is_string()) {
                payload[key] = v.clone();
            }
        }
        // #169: the index of an own zoom preset (the menu item `zoom_own` with n, as menu::dispatch sends it)
        if let Some(n) = args.get("n").filter(|v| v.is_u64()) {
            payload["n"] = n.clone();
        }
        self.events.push("menu", payload.clone());
        Ok(payload)
    }

    /// The channels (for tests and for the long poll).
    #[must_use]
    pub fn channels(&self) -> &Arc<Channels> {
        &self.channels
    }

    /// Run one command with JSON arguments. The names and argument names are those of the
    /// Tauri wrappers (`apps/desktop/src-tauri/src/lib.rs`). The commands of the registry run
    /// in [`crate::call::run`] (the transport too, R3); the bridge adds the test commands.
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] for an unknown command; a 400 for a missing or wrongly typed argument (for example
    /// `ms`, `position`, `volume_db`, `device`, `devices`); otherwise the error of the command itself.
    pub fn invoke(&self, cmd: &str, args: &Value) -> Result<Reply, AppError> {
        let s = &self.svc;
        match cmd {
            // tests only (the development bridge is a debug binary; the page mode of the shipped
            // app does not carry them): a copy job pauses `ms` after each chunk
            #[cfg(debug_assertions)]
            "test_copy_delay" => {
                s.set_copy_delay(Duration::from_millis(opt_i64(args, "ms")?.unwrap_or(0).max(0) as u64));
                Ok(Reply::Json(json!({"ms": opt_i64(args, "ms")?.unwrap_or(0)})))
            }
            // P3-13a: the event `menu` as the native menu sends it (the app's debug command of the same name),
            // so the browser tests reach the menu items through the page's own listener
            #[cfg(debug_assertions)]
            "selftest_menu" => Ok(Reply::Json(self.menu_event(args)?)),
            other => match crate::call::run(s, &self.channels, other, args) {
                Some(r) => r,
                None => self.invoke_test_outputs(other, args),
            },
        }
    }

    /// The bridge commands that are not in the registry: the test outputs of P3-16 in debug
    /// builds; in a release binary (the page mode) every name outside the registry is unknown.
    fn invoke_test_outputs(&self, cmd: &str, args: &Value) -> Result<Reply, AppError> {
        #[cfg(not(debug_assertions))]
        let _ = args;
        #[cfg(not(debug_assertions))]
        return Err(AppError::not_found(format!("unknown command {cmd}")));
        #[cfg(debug_assertions)]
        {
            let s = &self.svc;
            match cmd {
                "outputs_test_list" => {
                    let list = match args.get("devices") {
                        None | Some(Value::Null) => None,
                        Some(Value::Array(a)) => Some(
                            a.iter()
                                .map(|d| {
                                    Ok(crate::DeviceKey {
                                        name: d
                                            .get("name")
                                            .and_then(Value::as_str)
                                            .ok_or_else(|| AppError::bad("a device needs a `name`"))?
                                            .to_string(),
                                        rate: d.get("rate").and_then(Value::as_u64).unwrap_or(48_000) as u32,
                                        channels: d.get("channels").and_then(Value::as_u64).unwrap_or(2) as u16,
                                    })
                                })
                                .collect::<Result<Vec<_>, AppError>>()?,
                        ),
                        Some(_) => return Err(AppError::bad("`devices` must be a list")),
                    };
                    s.set_device_list(list);
                    Ok(Reply::Json(s.outputs_list()))
                }
                other => Err(AppError::not_found(format!("unknown command {other}"))),
            }
        }
    }

    /// One HTTP request: `POST /invoke/<cmd>`, `GET /channel/<id>`, or a static file.
    pub fn handle(&self, method: &str, url: &str, body: &[u8]) -> HttpReply {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        if let Some(cmd) = path.strip_prefix("/invoke/") {
            if method != "POST" {
                return HttpReply::text(405, "POST only");
            }
            let args: Value = if body.iter().all(u8::is_ascii_whitespace) {
                json!({})
            } else {
                match serde_json::from_slice(body) {
                    Ok(v) => v,
                    Err(e) => return error_reply(&AppError::bad(format!("arguments are not JSON: {e}"))),
                }
            };
            return match self.invoke(cmd, &args) {
                Ok(Reply::Json(v)) => HttpReply::json(200, &v),
                Ok(Reply::Bytes(b)) => HttpReply {
                    status: 200,
                    content_type: "application/octet-stream",
                    body: b,
                },
                Err(e) => error_reply(&e),
            };
        }
        if path == "/events" {
            // `after=now` (or nothing) starts at the end of the log: a new page sees only new events
            let after = query_value(query, "after")
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| self.events.head());
            let ms: u64 = query_value(query, "timeout_ms")
                .and_then(|v| v.parse().ok())
                .unwrap_or(2000)
                .min(30_000);
            return HttpReply::json(200, &self.events.poll(after, Duration::from_millis(ms)));
        }
        if let Some(id) = path.strip_prefix("/channel/") {
            let ms: u64 = query_value(query, "timeout_ms")
                .and_then(|v| v.parse().ok())
                .unwrap_or(2000)
                .min(30_000);
            let msgs = self.channels.poll(id, Duration::from_millis(ms));
            return HttpReply::json(200, &Value::Array(msgs));
        }
        if method != "GET" && method != "HEAD" {
            return HttpReply::text(405, "GET only");
        }
        self.static_file(path)
    }

    /// A file of the static source; `/` is `index.html` with the bridge flag. Paths with `..`
    /// or an absolute part are refused.
    fn static_file(&self, path: &str) -> HttpReply {
        let rel = path.trim_start_matches('/');
        let rel = if rel.is_empty() { "index.html" } else { rel };
        let rel_path = Path::new(rel);
        if rel_path.components().any(|c| !matches!(c, Component::Normal(_))) || rel.contains('\\') {
            return HttpReply::text(400, "bad path");
        }
        let bytes = match &self.static_source {
            StaticSource::Dir(dir) => std::fs::read(dir.join(rel_path)).ok(),
            StaticSource::Embedded(read) => read(rel),
        };
        let mime = mime_of(rel_path);
        match bytes {
            Some(bytes) if rel == "index.html" => HttpReply {
                status: 200,
                content_type: mime,
                body: inject_flag(&String::from_utf8_lossy(&bytes)).into_bytes(),
            },
            Some(bytes) => HttpReply {
                status: 200,
                content_type: mime,
                body: bytes,
            },
            None => HttpReply::text(404, "not found"),
        }
    }
}

fn error_reply(e: &AppError) -> HttpReply {
    HttpReply::json(e.code, &json!({"code": e.code, "message": e.message}))
}

#[cfg(test)]
mod tests;
