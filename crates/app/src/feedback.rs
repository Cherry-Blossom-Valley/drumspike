//! #164 (part 2 of #153): the feedback gateway command. The page builds the whole payload
//! (kind, title, description, optional steps, meta) and shows it as the preview; this command
//! checks the private-word guard and POSTs exactly the previewed JSON to the CBV feedback
//! gateway (part 1, Cherry-Blossom-Valley/Cbvsite, route `POST .../feedback`). The transport is
//! `curl` (std only: no new crate in Cargo.lock, the same rule as the About `open <address>`).
//! The test bridge answers the command in the page (static/backend.js), so e2e never needs the
//! network; the unit test here points the command at a local HTTP server.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::AppError;

/// The CBV feedback gateway (the owner's design decision on #153: option b, the CBV site
/// backend). The default of the app setting `feedback_url` (the page passes it); the route is
/// part 1, `POST .../feedback` of the make-server-91a5b4f8 edge function.
pub const FEEDBACK_URL: &str = "https://pubtzkqxycnqqhmrfnhn.supabase.co/functions/v1/make-server-91a5b4f8/feedback";

/// The public anon key of the CBV site (Cherry-Blossom-Valley/Cbvsite
/// utils/supabase/info.tsx `publicAnonKey`): public by design, it ships in the website's JS.
/// Supabase edge functions want it as `Authorization: Bearer <anon key>`, exactly like the
/// site's contact form sends it. It is not a secret and must never be one.
pub const FEEDBACK_ANON_KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6InB1YnR6a3F4eWNucXFobXJmbmhuIiwicm9sZSI6ImFub24iLCJpYXQiOjE3Njc2MjcxMzMsImV4cCI6MjA4MzIwMzEzM30.yye8PrO22dIkvCLmGV-mMwaW-jYFB-3Ns3IXk32Sxl0";

/// The static part of the private-word guard (`tools/media/check_paths.py` `BANNED_BASE`): a
/// report with a private path, a host alias or a user name must not leave the app.
/// `DRUMSPIKE_PRIVATE_WORDS` is a host variable of the repo tools, never an app setting.
pub const PRIVATE_WORDS: [&str; 8] = [
    "/Users",       // private-list-definition
    "/var/folders", // private-list-definition
    "/home",        // private-list-definition
    "/private/tmp", // private-list-definition
    "x86-mac",      // private-list-definition
    "admin@",       // private-list-definition
    "arm64-mac",    // private-list-definition
    "linux-arm64",  // private-list-definition
];

/// The free-text fields of the payload the guard runs on.
const GUARDED_FIELDS: [&str; 3] = ["title", "description", "steps"];

/// The kinds the gateway takes (the form's select).
pub const KINDS: [&str; 3] = ["bug", "improvement", "other"];

/// The private words and path-like tokens of one free text: `[]` when the text may leave the app.
#[must_use]
pub fn private_hits(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let low = text.to_lowercase();
    for w in PRIVATE_WORDS {
        let w = w.to_lowercase();
        if low.contains(&w) && !out.iter().any(|h| h == &w) {
            out.push(w);
        }
    }
    for tok in text.split_whitespace() {
        let t = tok.trim_end_matches(['.', ',', ';', '!', '?', ')']);
        let is_path = (t.starts_with('/') && t.contains('/'))
            || t.starts_with("~/")
            || (t.len() > 3 && t.as_bytes()[1] == b':' && (t.as_bytes()[2] == b'\\' || t.as_bytes()[2] == b'/'));
        if is_path && !out.iter().any(|h| h == t) {
            out.push(t.to_string());
        }
    }
    out
}

/// The guard for one field: a clear error names the field and the hit, the report is not sent.
///
/// # Errors
///
/// `AppError::bad` when the text holds a private word or a path-like token.
pub fn guard_field(field: &str, text: &str) -> Result<(), AppError> {
    let hits = private_hits(text);
    if hits.is_empty() {
        return Ok(());
    }
    Err(AppError::bad(format!(
        "the {field} holds a private name or path ({}) and was not sent; remove it and send again",
        hits.join(", ")
    )))
}

/// The body the gateway receives: the payload the page previewed, byte for byte.
///
/// # Errors
///
/// `AppError::bad` when the payload is not a JSON object, the kind is unknown, the
/// title is empty or a guarded field fails the private-word guard.
pub fn check_payload(payload: &Value) -> Result<(), AppError> {
    let obj = payload
        .as_object()
        .ok_or_else(|| AppError::bad("payload must be a JSON object"))?;
    let kind = obj.get("kind").and_then(Value::as_str).unwrap_or("");
    if !KINDS.contains(&kind) {
        return Err(AppError::bad("kind must be bug, improvement or other"));
    }
    let title = obj.get("title").and_then(Value::as_str).unwrap_or("");
    if title.trim().is_empty() {
        return Err(AppError::bad("title must not be empty"));
    }
    for field in GUARDED_FIELDS {
        if let Some(s) = obj.get(field).and_then(Value::as_str) {
            guard_field(field, s)?;
        }
    }
    Ok(())
}

/// POST the payload with curl; answers the gateway's JSON reply (`{ok, issue_url}`).
fn post(url: &str, body: &[u8]) -> Result<Value, AppError> {
    let mut child = Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "20",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-H",
            &format!("Authorization: Bearer {FEEDBACK_ANON_KEY}"),
            "--data-binary",
            "@-",
        ])
        .arg("-w")
        .arg("\n%{http_code}")
        .arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::internal(format!("the feedback gateway needs curl: {e}")))?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(body)
        .map_err(|e| AppError::internal(format!("the feedback gateway needs curl: {e}")))?;
    let mut out = String::new();
    child
        .stdout
        .take()
        .expect("piped stdout")
        .read_to_string(&mut out)
        .map_err(|e| AppError::internal(format!("the feedback gateway needs curl: {e}")))?;
    let status = child.wait().map_err(|e| AppError::internal(format!("curl: {e}")))?;
    if !status.success() {
        let mut err = String::new();
        if let Some(mut se) = child.stderr.take() {
            let _ = se.read_to_string(&mut err);
        }
        return Err(AppError::internal(format!(
            "curl failed ({}): {}; the report was not sent",
            status,
            err.trim()
        )));
    }
    let (text, code) = out.rsplit_once('\n').unwrap_or(("", "0"));
    let code: u32 = code.trim().parse().unwrap_or(0);
    let value = serde_json::from_str(text).unwrap_or(Value::Null);
    if !(200..300).contains(&code) {
        // the gateway's own message (rate limit, spam rule) reaches the form
        let msg = value.get("error").and_then(Value::as_str).map_or_else(
            || format!("the feedback gateway answered {code}"),
            std::string::ToString::to_string,
        );
        return Err(AppError {
            code: code.clamp(400, 599) as u16,
            message: msg,
        });
    }
    Ok(value)
}

impl super::Service {
    /// `feedback_send`: the guard, then the exact previewed JSON to the gateway.
    /// The URL comes from the page (one constant there); `None` or empty: the default.
    ///
    /// # Errors
    ///
    /// `AppError` when the payload fails the guard, is not valid JSON, or the
    /// gateway call fails (non-2xx or curl error).
    pub fn feedback_send(&self, payload: &Value, url: Option<&str>) -> Result<Value, AppError> {
        check_payload(payload)?;
        let url = match url {
            Some(u) if !u.is_empty() => u,
            _ => FEEDBACK_URL,
        };
        let body = serde_json::to_vec(payload)
            .map_err(|e| AppError::internal(format!("the payload is not valid JSON: {e}")))?;
        post(url, &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One local HTTP server: reads one request, answers `{ok, issue_url}`; returns the body it got.
    fn mock_gateway() -> (std::net::SocketAddr, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let got: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // read the headers, then content-length more bytes
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let body = loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break Vec::new();
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&raw[..i + 4]).to_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        let need = i + 4 + len;
                        while raw.len() < need {
                            let n = stream.read(&mut buf).unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            raw.extend_from_slice(&buf[..n]);
                        }
                        break raw[i + 4..raw.len().min(need)].to_vec();
                    }
                };
                *g.lock().unwrap() = body;
                let reply_body = "{\"ok\":true,\"issue_url\":\"https://example.com/x/1\"}";
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply_body.len(),
                    reply_body
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (addr, got)
    }

    #[test]
    fn the_guard_names_the_private_words_and_paths() {
        // #164: the samples are built from the constant, so this diff carries no
        // literal private token outside the marked definition lines.
        let (users, home, admin_at, arm64_mac) =
            (PRIVATE_WORDS[0], PRIVATE_WORDS[2], PRIVATE_WORDS[5], PRIVATE_WORDS[6]);
        assert!(private_hits("the snare is 30 ms early").is_empty());
        let path = format!("{home}/me/stuff");
        assert_eq!(
            private_hits(&format!("my path is {path}")),
            vec![home.to_string(), path.clone()]
        );
        assert_eq!(private_hits(&arm64_mac.to_uppercase()), vec![arm64_mac.to_string()]);
        let win = format!("C:\\{}\\me\\a.wav", users.trim_start_matches('/'));
        assert_eq!(private_hits(&win), vec![win.clone()]);
        assert_eq!(private_hits("see ~/Music/take1.wav"), vec!["~/Music/take1.wav"]);
        assert_eq!(private_hits(&format!("{admin_at} saw it")), vec![admin_at.to_string()]);
    }

    #[test]
    fn guard_field_refuses_with_the_field_name() {
        let desc = format!("it broke in {}/me/track.wav", PRIVATE_WORDS[0]);
        let e = guard_field("description", &desc).unwrap_err();
        assert_eq!(e.code, 400);
        assert!(e.message.contains("description"), "{}", e.message);
        assert!(guard_field("title", "Kick misses on 2 and 4").is_ok());
    }

    #[test]
    fn check_payload_takes_only_complete_reports() {
        assert!(check_payload(&json!({"kind": "bug", "title": "t", "description": "d"})).is_ok());
        assert!(check_payload(&json!({"kind": "other", "title": "t"})).is_ok());
        assert_eq!(
            check_payload(&json!({"kind": "spam", "title": "t"})).unwrap_err().code,
            400
        );
        assert_eq!(
            check_payload(&json!({"kind": "bug", "title": " "})).unwrap_err().code,
            400
        );
        assert_eq!(
            check_payload(&json!({"kind": "bug", "title": "/etc/passwd"}))
                .unwrap_err()
                .code,
            400
        );
    }

    #[test]
    fn feedback_send_posts_the_payload_and_answers_the_gateway() {
        let (addr, got) = mock_gateway();
        let s = super::super::Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        let payload = json!({
            "kind": "bug",
            "title": "Kick misses on 2 and 4",
            "description": "The review queue lists them every time.",
            "steps": "Open the demo project, press D twice.",
            "meta": {"app_version": "0.9.8", "os": "linux", "arch": "arm64"},
        });
        let url = format!("http://{addr}/feedback");
        let r = s.feedback_send(&payload, Some(&url)).unwrap();
        assert_eq!(r["ok"], true);
        assert_eq!(r["issue_url"], "https://example.com/x/1");
        let want = serde_json::to_vec(&payload).unwrap();
        assert_eq!(
            &*got.lock().unwrap(),
            &want[..],
            "the gateway receives exactly the previewed JSON"
        );
    }

    #[test]
    fn feedback_send_refuses_a_private_payload_before_the_network() {
        let s = super::super::Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        let e = s
            .feedback_send(
                &json!({
                    "kind": "bug",
                    "title": "t",
                    "description": format!("at {}/me/x.wav", PRIVATE_WORDS[2])
                }),
                Some("http://127.0.0.1:1/feedback"),
            )
            .unwrap_err();
        assert_eq!(e.code, 400);
        assert!(e.message.contains(PRIVATE_WORDS[2]));
    }
}
