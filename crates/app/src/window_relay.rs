//! #388: the agent drives the running window. The page helpers of `static/agent.js` (`window.DSAgent`) run in the page of
//! the window, so a call from outside has to reach that page. This module is the way there, in two halves:
//!
//! * [`WindowRelay`] (held by the [`Service`]) is a queue between an endpoint and the page. The page long-polls
//!   `window_call_next` (a command like any other, so the Tauri window and the test bridge share it), runs the helper it is
//!   given and answers with `window_call_result`. Nothing is evaluated in the page that the helper list does not name.
//! * [`start`] opens the local endpoint: `POST /call {"fn", "args"}` on 127.0.0.1 with a bearer token, answered when the
//!   page has answered. The port and the token are written to `<home>/window-endpoint.json` (mode 0600), so the agent that
//!   runs as the same user finds them and nothing else on the machine can call the page without reading that file.
//!
//! No browser, no second process on the project, no quit: the project the agent edits is the one open in the window.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufReader, Read};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::bridge::{HttpReply, HttpRequest, IDLE_READ_TIMEOUT, read_request, write_reply};
use crate::{AppError, Service};

/// The helpers of `window.DSAgent` that the endpoint passes on; any other name is refused before it reaches the page.
pub const FUNCTIONS: [&str; 14] = [
    "project",
    "hits",
    "look",
    "wide",
    "move",
    "confirm",
    "undo",
    "plan",
    "audit",
    "apply",
    "save",
    "status",
    "statusLine",
    "setTraining",
];

/// The file in the home folder that names the endpoint: `{port, token, pid}`.
pub const ENDPOINT_FILE: &str = "window-endpoint.json";

/// How long a call waits for the page when the caller names no `timeout_ms`.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(60);

/// The longest wait a caller may ask for.
pub const MAX_WAIT: Duration = Duration::from_secs(600);

/// The longest long-poll of the page (`window_call_next`).
pub const MAX_POLL: Duration = Duration::from_secs(30);

/// A page that polled this recently counts as listening, in the gap between two polls.
const LISTEN_GRACE: Duration = Duration::from_secs(5);

#[derive(Default)]
struct State {
    next_id: u64,
    /// The endpoint is open: the page may poll (a bridge without it answers `enabled: false` and the page stops).
    enabled: bool,
    queue: VecDeque<(u64, Value)>,
    /// Calls the page took and has not answered yet.
    inflight: HashSet<u64>,
    done: HashMap<u64, Result<Value, String>>,
    pollers: usize,
    /// The page that polled last; the polls of any other page end at once.
    current_poller: String,
    last_poll: Option<Instant>,
}

/// The queue between the endpoint and the page (see the module docs).
#[derive(Default)]
pub struct WindowRelay {
    state: Mutex<State>,
    cv: Condvar,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl WindowRelay {
    /// The endpoint is open: `window_call_next` now waits for calls instead of answering `enabled: false`.
    pub fn enable(&self) {
        lock(&self.state).enabled = true;
    }

    /// Whether the endpoint is open.
    #[must_use]
    pub fn enabled(&self) -> bool {
        lock(&self.state).enabled
    }

    /// Whether a page takes calls: it waits in a poll now, runs a call, or polled within [`LISTEN_GRACE`].
    #[must_use]
    pub fn listening(&self) -> bool {
        let g = lock(&self.state);
        g.pollers > 0 || !g.inflight.is_empty() || g.last_poll.is_some_and(|t| t.elapsed() < LISTEN_GRACE)
    }

    /// Pass the helper `fun` with `args` (a JSON list) to the page and wait for its answer.
    ///
    /// # Errors
    ///
    /// 400 for a helper name outside [`FUNCTIONS`] or `args` that is not a list; 409 when no page listens or the helper
    /// refused (its message is the page's: Training Mode off, no such hit); 504 when the page did not answer within `wait`.
    pub fn call(&self, fun: &str, args: &Value, wait: Duration) -> Result<Value, AppError> {
        if !FUNCTIONS.contains(&fun) {
            return Err(AppError::bad(format!(
                "unknown helper {fun}: the helpers are {}",
                FUNCTIONS.join(", ")
            )));
        }
        if !args.is_array() {
            return Err(AppError::bad("`args` must be a list"));
        }
        let end = Instant::now() + wait;
        let mut g = lock(&self.state);
        let listening =
            g.pollers > 0 || !g.inflight.is_empty() || g.last_poll.is_some_and(|t| t.elapsed() < LISTEN_GRACE);
        if !g.enabled || !listening {
            return Err(AppError::conflict(
                "no page is listening: the window has no open page that polls for calls",
            ));
        }
        g.next_id += 1;
        let id = g.next_id;
        g.queue.push_back((id, json!({"id": id, "fn": fun, "args": args})));
        self.cv.notify_all();
        loop {
            if let Some(r) = g.done.remove(&id) {
                return r.map_err(AppError::conflict);
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                g.queue.retain(|(i, _)| *i != id);
                g.inflight.remove(&id);
                return Err(AppError {
                    code: 504,
                    message: format!("the page did not answer {fun} in {} s", wait.as_secs()),
                });
            }
            g = self
                .cv
                .wait_timeout(g, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }

    /// The page `poller` asks for its next call; waits up to `wait` (at most [`MAX_POLL`]). `None`: nothing came, or a
    /// newer page took over (a poll of a closed page can hang on after its window is gone and must not swallow a call).
    pub fn next(&self, poller: &str, wait: Duration) -> Option<Value> {
        let end = Instant::now() + wait.min(MAX_POLL);
        let mut g = lock(&self.state);
        if g.current_poller != poller {
            g.current_poller = poller.to_string();
            self.cv.notify_all();
        }
        g.pollers += 1;
        let out = loop {
            if g.current_poller != poller {
                break None;
            }
            if let Some((id, call)) = g.queue.pop_front() {
                g.inflight.insert(id);
                break Some(call);
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break None;
            }
            g = self
                .cv
                .wait_timeout(g, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        };
        g.pollers -= 1;
        g.last_poll = Some(Instant::now());
        out
    }

    /// The page answers call `id`: `Ok(value)` or `Err(message)` of the helper.
    ///
    /// # Errors
    ///
    /// 404 when no call `id` waits for an answer (it timed out, or was answered).
    pub fn finish(&self, id: u64, result: Result<Value, String>) -> Result<(), AppError> {
        let mut g = lock(&self.state);
        if !g.inflight.remove(&id) {
            return Err(AppError::not_found(format!("no call {id} waits for an answer")));
        }
        g.done.insert(id, result);
        self.cv.notify_all();
        Ok(())
    }
}

/// `window_call_next {timeout_ms?}`: the page's long poll. Answers `{enabled, call}`; `call` is `{id, fn, args}` or null.
///
/// # Errors
///
/// 400 when `timeout_ms` is not a number.
pub fn window_call_next(svc: &Service, args: &Value) -> Result<Value, AppError> {
    let relay = svc.window_relay();
    let ms = match args.get("timeout_ms") {
        None | Some(Value::Null) => 25_000,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| AppError::bad("`timeout_ms` must be a whole number"))?,
    };
    if !relay.enabled() {
        return Ok(json!({"enabled": false, "call": null}));
    }
    let poller = args.get("poller").and_then(Value::as_str).unwrap_or("");
    Ok(json!({"enabled": true, "call": relay.next(poller, Duration::from_millis(ms))}))
}

/// `window_call_result {id, ok, value?, error?}`: the page's answer to call `id`.
///
/// # Errors
///
/// 400 for a missing `id` or `ok`; 404 when no call `id` waits.
pub fn window_call_result(svc: &Service, args: &Value) -> Result<Value, AppError> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| AppError::bad("missing argument `id` (whole number)"))?;
    let ok = args
        .get("ok")
        .and_then(Value::as_bool)
        .ok_or_else(|| AppError::bad("missing argument `ok` (boolean)"))?;
    let result = if ok {
        Ok(args.get("value").cloned().unwrap_or(Value::Null))
    } else {
        Err(args
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("the helper failed")
            .to_string())
    };
    svc.window_relay().finish(id, result)?;
    Ok(json!({}))
}

/// R3: the handlers of the page's two commands for [`crate::call::run`].
pub(crate) const HANDLERS: [crate::call::Route; 2] =
    crate::call::routes(["window_call_next", "window_call_result"], |s, _, cmd, args| {
        let r = match cmd {
            "window_call_next" => Some(window_call_next(s, args)),
            "window_call_result" => Some(window_call_result(s, args)),
            _ => None,
        };
        crate::call::json_reply(cmd, r)
    });

/// A running endpoint: its port and token, and the file that names them.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
    pub file: PathBuf,
}

impl Endpoint {
    /// Remove the endpoint file (at exit), so a client does not read a dead port.
    pub fn remove_file(&self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

/// 32 random bytes from the system, as 64 hex characters.
fn new_token() -> std::io::Result<String> {
    let mut b = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Write `{port, token, pid}` to `<home>/window-endpoint.json`, readable by the owner only.
fn write_endpoint_file(home: &Path, port: u16, token: &str) -> std::io::Result<PathBuf> {
    use std::io::Write as _;
    std::fs::create_dir_all(home)?;
    let file = home.join(ENDPOINT_FILE);
    let tmp = home.join(format!("{ENDPOINT_FILE}.{}", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(
        json!({"port": port, "token": token, "pid": std::process::id()})
            .to_string()
            .as_bytes(),
    )?;
    f.flush()?;
    drop(f);
    std::fs::rename(&tmp, &file)?;
    Ok(file)
}

/// Open the window endpoint for `svc`: a listener on 127.0.0.1 (a free port), a fresh token and the endpoint file in
/// `home`; the relay starts taking calls. The listener runs on its own threads until the process ends.
///
/// # Errors
///
/// The [`std::io::Error`] of reading the system's random bytes, binding the listener or writing the file.
pub fn start(svc: &Service, home: &Path) -> std::io::Result<Endpoint> {
    let token = new_token()?;
    let listener = crate::bridge::listen(0)?;
    let port = listener.local_addr()?.port();
    let file = write_endpoint_file(home, port, &token)?;
    svc.window_relay().enable();
    let (svc2, token2) = (svc.clone(), token.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let (svc, token) = (svc2.clone(), token2.clone());
            std::thread::spawn(move || connection(stream, &svc, &token));
        }
    });
    Ok(Endpoint { port, token, file })
}

fn connection(stream: TcpStream, svc: &Service, token: &str) {
    let _ = stream.set_read_timeout(Some(IDLE_READ_TIMEOUT));
    let Ok(read_half) = stream.try_clone() else { return };
    let mut reader = BufReader::new(read_half);
    let mut writer = stream;
    loop {
        match read_request(&mut reader) {
            Ok(Some(req)) => {
                let reply = handle(svc, token, &req);
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

/// Compare two tokens without stopping at the first difference.
fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn error_reply(e: &AppError) -> HttpReply {
    HttpReply::json(e.code, &json!({"code": e.code, "message": e.message}))
}

/// One request of the endpoint: `POST /call` and `GET /status`, both with `Authorization: Bearer <token>`.
#[must_use]
pub fn handle(svc: &Service, token: &str, req: &HttpRequest) -> HttpReply {
    let given = req
        .authorization
        .as_deref()
        .and_then(|a| a.strip_prefix("Bearer ").or_else(|| a.strip_prefix("bearer ")));
    if !given.is_some_and(|g| same_token(g.trim(), token)) {
        return error_reply(&AppError {
            code: 401,
            message: format!("a call needs the token of {ENDPOINT_FILE} in the header `Authorization: Bearer <token>`"),
        });
    }
    let path = req.url.split('?').next().unwrap_or("");
    match (req.method.as_str(), path) {
        ("GET", "/status") => HttpReply::json(
            200,
            &json!({"listening": svc.window_relay().listening(), "helpers": FUNCTIONS}),
        ),
        ("POST", "/call") => {
            let body: Value = match serde_json::from_slice(&req.body) {
                Ok(v) => v,
                Err(e) => return error_reply(&AppError::bad(format!("the body is not JSON: {e}"))),
            };
            let Some(fun) = body.get("fn").and_then(Value::as_str) else {
                return error_reply(&AppError::bad("missing `fn` (string): the helper to call"));
            };
            let args = body.get("args").cloned().unwrap_or_else(|| json!([]));
            let wait = match body.get("timeout_ms") {
                None | Some(Value::Null) => DEFAULT_WAIT,
                Some(v) => match v.as_u64() {
                    Some(ms) => Duration::from_millis(ms).min(MAX_WAIT),
                    None => return error_reply(&AppError::bad("`timeout_ms` must be a whole number")),
                },
            };
            match svc.window_relay().call(fun, &args, wait) {
                Ok(value) => HttpReply::json(200, &json!({"ok": true, "value": value})),
                Err(e) => error_reply(&e),
            }
        }
        _ => HttpReply::text(404, "POST /call or GET /status"),
    }
}

/// The one call of an agent (`agent_call` as MCP tool, `cli agent_call fn args...`): read `<home>/window-endpoint.json`,
/// `POST /call` and answer the helper's value. The agent does not read the token or speak HTTP itself.
///
/// # Errors
///
/// 409 with a plain message when no window is open (no endpoint file, or its port refuses: a dead app left the file) or
/// the helper refused; 400 for a helper outside [`FUNCTIONS`]; 504 when the page did not answer; 500 for a broken reply.
pub fn agent_call(home: Option<&Path>, fun: &str, args: &Value, timeout_ms: Option<u64>) -> Result<Value, AppError> {
    use std::io::Write as _;
    let no_window = || {
        AppError::conflict(
            "no Drum Spike window is open (or it has no open page): open the app, or start the bridge with a browser tool",
        )
    };
    let home = home.ok_or_else(no_window)?;
    let file: Value = std::fs::read(home.join(ENDPOINT_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(no_window)?;
    let (Some(port), Some(token)) = (
        file["port"].as_u64().and_then(|p| u16::try_from(p).ok()),
        file["token"].as_str(),
    ) else {
        return Err(no_window());
    };
    let wait = timeout_ms.map_or(DEFAULT_WAIT, |ms| Duration::from_millis(ms).min(MAX_WAIT));
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|_| no_window())?;
    // the endpoint answers when the page has, or after `wait`: allow a little more for the 504
    let _ = stream.set_read_timeout(Some(wait + Duration::from_secs(10)));
    let body = json!({"fn": fun, "args": args, "timeout_ms": wait.as_millis() as u64}).to_string();
    write!(
        stream,
        "POST /call HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(|e| AppError::internal(format!("the window endpoint: {e}")))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| AppError::internal(format!("the window endpoint did not answer: {e}")))?;
    let raw = String::from_utf8_lossy(&raw);
    let code = raw
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| AppError::internal("the window endpoint sent no HTTP status"))?;
    let reply: Value = raw
        .split_once("\r\n\r\n")
        .and_then(|(_, b)| serde_json::from_str(b).ok())
        .unwrap_or(Value::Null);
    if code == 200 {
        return Ok(reply["value"].clone());
    }
    Err(AppError {
        code,
        message: reply["message"]
            .as_str()
            .unwrap_or("the window endpoint refused the call")
            .to_string(),
    })
}

/// The `agent_call` arguments of a tool call: `{fn, args?, timeout_ms?}`.
///
/// # Errors
///
/// 400 for a missing `fn`, or `args` or `timeout_ms` of the wrong type; otherwise those of [`agent_call`].
pub fn agent_call_args(home: Option<&Path>, a: &Value) -> Result<Value, AppError> {
    let fun = a
        .get("fn")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad("missing argument `fn` (string): the helper to call"))?;
    let args = match a.get("args") {
        None | Some(Value::Null) => json!([]),
        Some(v) if v.is_array() => v.clone(),
        Some(_) => return Err(AppError::bad("`args` must be a list")),
    };
    let ms = match a.get("timeout_ms") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .ok_or_else(|| AppError::bad("`timeout_ms` must be a whole number"))?,
        ),
    };
    agent_call(home, fun, &args, ms)
}

#[cfg(test)]
mod tests;
