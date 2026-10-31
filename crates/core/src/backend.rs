//! Detector plug-ins 1 (#363, `docs/design/detector-plugins.md` sections 2, 4 and 5): the
//! [`DetectorBackend`] seam with the built-in [`DspBackend`] and the [`ExternalDetector`]
//! process adapter, the protocol 1 request/response types and the response validator.
//!
//! One hit type everywhere: [`DetectedHit`], in the return value and in the candidate list.
//! The protocol's string class is validated and mapped to the internal class index before a
//! hit enters the pipeline; unknown classes are dropped. Everything downstream of step 1 of
//! the analysis (the role filter, the dedups, the refinement, the velocities) is unchanged,
//! so a plugin proposes coarse hit times and Drum Spike keeps them sample-exact.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::detect::CLASS_NOTES;
use crate::detect::onset::{OnsetError, detect_onsets};
use crate::io::{WavFormat, WavSpec};

/// The protocol classes, in the internal class order (`N_CLASSES`), plus `other`.
pub const CLASSES: [&str; 6] = ["kick", "snare", "tom", "hat", "cymbal", "other"];

/// The index of `class` in [`CLASSES`], or `None` for an unknown class (dropped).
#[must_use]
pub fn class_index(class: &str) -> Option<u8> {
    CLASSES.iter().position(|&c| c == class).map(|i| i as u8)
}

/// The MIDI note of an internal class index at the track's `role` (the `other` class and an
/// unknown role take the role's note, as [`crate::analyze::role_note`] does).
#[must_use]
pub fn class_note(class: u8, role: &str) -> u8 {
    if (class as usize) < CLASS_NOTES.len() {
        CLASS_NOTES[class as usize]
    } else {
        crate::analyze::role_note(role)
    }
}

/// The limits and timeouts of the design section 5.
pub const HITS_PER_S: f64 = 50.0;
/// The candidates-per-second limit.
pub const CANDIDATES_PER_S: f64 = 400.0;
/// The hits-per-track limit.
pub const MAX_HITS: usize = 100_000;
/// The candidates-per-track limit.
pub const MAX_CANDIDATES: usize = 1_000_000;
/// The response line length limit: 64 MiB.
pub const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;
/// The default per-track timeout in seconds (a plugin param can lower it, not raise it).
pub const DEFAULT_TIMEOUT_S: u64 = 120;

/// A hit of any detector: seconds from the start of the file, the internal class index and a
/// confidence in 0 to 1.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedHit {
    /// Seconds from the start of the file.
    pub time: f64,
    /// The internal class index (into [`CLASSES`], `5` = `other`).
    pub class: u8,
    /// The confidence, 0.0 to 1.0.
    pub confidence: f32,
}

/// The candidate list of a detection (the Sensitivity slider, the histograms and Learn).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CandidateList {
    /// The candidates.
    pub hits: Vec<DetectedHit>,
}

/// Errors of the backend seam (the readable track errors of the design section 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// The DSP fallback failed.
    Onset(OnsetError),
    /// The plugin's answer carried `{"error": ...}`.
    Plugin(String),
    /// The process failed to start, crashed or misbehaved.
    Process(String),
    /// A limit of section 5 was exceeded.
    Limit(String),
    /// The plugin speaks another protocol major.
    Protocol(String),
    /// Invalid JSON (request or response).
    Json(String),
    /// I/O (the temp WAV, the pipes).
    Io(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendError::Onset(e) => write!(f, "detect: {e}"),
            BackendError::Plugin(e) => write!(f, "detector plugin: {e}"),
            BackendError::Process(e) => write!(f, "detector plugin: {e}"),
            BackendError::Limit(e) => write!(f, "detector plugin: limit exceeded: {e}"),
            BackendError::Protocol(e) => write!(f, "detector plugin: {e}"),
            BackendError::Json(e) => write!(f, "detector plugin: invalid JSON: {e}"),
            BackendError::Io(e) => write!(f, "detector plugin: {e}"),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<OnsetError> for BackendError {
    fn from(e: OnsetError) -> Self {
        BackendError::Onset(e)
    }
}

/// A detector backend: step 1 of the analysis for one track.
pub trait DetectorBackend {
    /// The hits of one track and, optionally, its candidate list.
    fn detect(&self, x: &[f32], sr: u32, role: &str)
    -> Result<(Vec<DetectedHit>, Option<CandidateList>), BackendError>;
}

/// The built-in DSP detector (the default): [`detect_onsets`] with the role's parameters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DspBackend;

impl DetectorBackend for DspBackend {
    fn detect(
        &self,
        x: &[f32],
        sr: u32,
        role: &str,
    ) -> Result<(Vec<DetectedHit>, Option<CandidateList>), BackendError> {
        let hits = detect_onsets(
            x,
            sr,
            crate::detect::onset::Role::parse(role).unwrap_or(crate::detect::onset::Role::Other),
        )?;
        let class = class_index(role).unwrap_or(5);
        Ok((
            hits.into_iter()
                .map(|h| DetectedHit {
                    time: h.time,
                    class,
                    confidence: 1.0,
                })
                .collect(),
            None,
        ))
    }
}

/// A protocol 1 hit as the plugin writes it.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ProtocolHit {
    /// Seconds from the start of the file.
    pub t: f64,
    /// One of the protocol classes.
    pub class: String,
    /// Optional; a missing confidence is 1.0.
    #[serde(default)]
    pub confidence: Option<f32>,
}

/// The protocol 1 request, one JSON line on the plugin's stdin.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Request<'a> {
    /// The protocol major version: 1.
    pub protocol: u32,
    /// Absolute path of the decoded mono WAV file for this track.
    pub wav: String,
    /// 0-based source channel; always 0 in protocol 1.
    pub channel: u32,
    /// One of the protocol roles.
    pub role: &'a str,
    /// Hz, the sample rate of the WAV file.
    pub sr: u32,
    /// Free-form detector parameters (empty when the track stores none).
    pub params: serde_json::Value,
}

/// The protocol 1 response as it arrives, before validation: a hit set or an error.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct PluginResponse {
    /// The response's own protocol major, when the plugin names one (absent: 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<u32>,
    /// The hit set.
    #[serde(default)]
    pub hits: Vec<ProtocolHit>,
    /// The optional candidate list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<ProtocolHit>>,
    /// The plugin's readable error, instead of a hit set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The validated response of one track, ready for the pipeline.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Validated {
    /// The hits, sorted by time (the validator sorts when the plugin did not).
    pub hits: Vec<DetectedHit>,
    /// The candidate list, when the plugin gave one.
    pub candidates: Option<CandidateList>,
    /// Hits dropped: `t` outside the file (NaN or infinite counts).
    pub dropped_out_of_range: usize,
    /// Hits dropped: an unknown class.
    pub dropped_unknown_class: usize,
    /// Hits dropped: `t` outside the file, candidates.
    pub candidates_dropped: usize,
    /// Whether the plugin's hits were unsorted and the validator sorted them.
    pub sorted: bool,
}

/// Validate one response (the design section 5): drop out-of-range and unknown-class hits,
/// clamp the confidences, sort by time and enforce the rate and total limits.
pub fn validate_response(
    response: &PluginResponse,
    file_len_s: f64,
    duration_s: f64,
) -> Result<Validated, BackendError> {
    if let Some(p) = response.protocol
        && p != 1
    {
        return Err(BackendError::Protocol(format!(
            "plugin speaks protocol {p}, this app speaks protocol 1"
        )));
    }
    if let Some(e) = &response.error {
        return Err(BackendError::Plugin(e.clone()));
    }
    let dur = duration_s.max(file_len_s);
    let check_rate = |n: usize, per_s: f64, what: &str| -> Result<(), BackendError> {
        let rate = if dur > 0.0 { n as f64 / dur } else { 0.0 };
        if rate > per_s + f64::EPSILON {
            return Err(BackendError::Limit(format!(
                "{what} per second: {rate:.1} over the limit of {per_s:.0}"
            )));
        }
        Ok(())
    };
    let convert = |list: &[ProtocolHit], what: &str| -> Result<(Vec<DetectedHit>, usize, usize, bool), BackendError> {
        if list.len() > if what == "hits" { MAX_HITS } else { MAX_CANDIDATES } {
            return Err(BackendError::Limit(format!(
                "{what}: {} over the limit of {}",
                list.len(),
                if what == "hits" { MAX_HITS } else { MAX_CANDIDATES }
            )));
        }
        let mut dropped_range = 0usize;
        let mut dropped_class = 0usize;
        let mut out: Vec<DetectedHit> = vec![];
        for h in list {
            let ok = h.t.is_finite() && h.t >= 0.0 && h.t <= file_len_s;
            if !ok {
                dropped_range += 1;
                continue;
            }
            let Some(class) = class_index(&h.class) else {
                dropped_class += 1;
                continue;
            };
            out.push(DetectedHit {
                time: h.t,
                class,
                confidence: h.confidence.unwrap_or(1.0).clamp(0.0, 1.0),
            });
        }
        let unsorted = out.windows(2).any(|w| w[0].time > w[1].time);
        if unsorted {
            out.sort_by(|a, b| a.time.total_cmp(&b.time));
        }
        Ok((out, dropped_range, dropped_class, unsorted))
    };
    let (hits, dropped_out_of_range, dropped_unknown_class, sorted) = convert(&response.hits, "hits")?;
    check_rate(hits.len(), HITS_PER_S, "hits")?;
    let mut candidates_dropped = 0usize;
    let candidates = match &response.candidates {
        Some(c) => {
            let (list, dropped, _, _) = convert(c, "candidates")?;
            candidates_dropped = dropped;
            check_rate(list.len(), CANDIDATES_PER_S, "candidates")?;
            Some(CandidateList { hits: list })
        }
        None => None,
    };
    Ok(Validated {
        hits,
        candidates,
        dropped_out_of_range,
        dropped_unknown_class,
        candidates_dropped,
        sorted,
    })
}

/// One installed plugin: the executable path and the timeout (a plugin param `timeout_s` can
/// lower the 120 s default, not raise it).
#[derive(Debug, Clone)]
pub struct ExternalDetector {
    /// The plugin id (the manifest `id`, the folder name).
    pub id: String,
    /// The executable path (the manifest `executable`, resolved against the folder).
    pub executable: PathBuf,
    /// The per-track timeout in seconds.
    pub timeout_s: u64,
    /// Free-form params for the request.
    pub params: serde_json::Value,
}

impl Default for ExternalDetector {
    fn default() -> Self {
        ExternalDetector {
            id: String::new(),
            executable: PathBuf::new(),
            timeout_s: DEFAULT_TIMEOUT_S,
            params: serde_json::Value::Object(serde_json::Map::new()),
        }
    }
}

/// The stderr tail of a failed run: at most 20 lines.
fn stderr_tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(20);
    lines[start..].join("\n")
}

impl ExternalDetector {
    /// The hits of one track: write the mono WAV, run the exchange and validate.
    fn detect_wav(&self, wav: &Path, role: &str, sr: u32, file_len_s: f64) -> Result<Validated, BackendError> {
        let request = Request {
            protocol: 1,
            wav: wav.display().to_string(),
            channel: 0,
            role,
            sr,
            params: self.params.clone(),
        };
        let line = serde_json::to_string(&request).map_err(|e| BackendError::Json(e.to_string()))?;
        let mut child = Command::new(&self.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| BackendError::Process(format!("{} does not run: {e}", self.id)))?;
        let request = format!("{line}\n");
        if let Err(e) = child
            .stdin
            .take()
            .ok_or_else(|| BackendError::Process("no stdin".into()))?
            .write_all(request.as_bytes())
        {
            // A plugin that exits without reading the request still has its answer on
            // stdout; only a real write error fails here.
            if e.kind() != std::io::ErrorKind::BrokenPipe {
                return Err(BackendError::Io(e.to_string()));
            }
        }
        // drop the stdin: the plugin answers after one request
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BackendError::Process("no stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BackendError::Process("no stderr".into()))?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut lines = vec![];
            let mut buf = String::new();
            loop {
                buf.clear();
                match reader.read_line(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        lines.push(buf.trim_end_matches('\n').to_string());
                        if lines.len() >= 2 {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(BackendError::Io(e.to_string())));
                        return;
                    }
                }
            }
            let _ = tx.send(Ok(lines));
            // keep the reader alive until EOF so the plugin does not block on a full pipe
            let mut rest = String::new();
            let _ = reader.read_to_string(&mut rest);
        });
        let stderr_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = BufReader::new(stderr).read_to_end(&mut buf);
            buf
        });
        let lines = match rx.recv_timeout(Duration::from_secs(self.timeout_s)) {
            Ok(Ok(lines)) => lines,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                let _ = child.kill();
                return Err(BackendError::Process(format!(
                    "{} timed out after {} s",
                    self.id, self.timeout_s
                )));
            }
        };
        let stderr = stderr_handle.join().unwrap_or_default();
        let status = child.wait().ok().map(|s| s.code().unwrap_or(-1));
        if lines.is_empty() {
            let tail = stderr_tail(&stderr);
            return Err(BackendError::Process(format!(
                "{} gave no response line (exit {status:?}){}",
                self.id,
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(": {tail}")
                }
            )));
        }
        if lines.len() > 1 {
            return Err(BackendError::Process(format!(
                "{} gave {} response lines for one request",
                self.id,
                lines.len()
            )));
        }
        if lines[0].len() > MAX_LINE_BYTES {
            return Err(BackendError::Limit(format!(
                "response line of {} bytes over the limit of {MAX_LINE_BYTES}",
                lines[0].len()
            )));
        }
        let response: PluginResponse =
            serde_json::from_str(&lines[0]).map_err(|e| BackendError::Json(format!("{}: {e}", self.id)))?;
        if let Some(status) = status
            && status != 0
        {
            let tail = stderr_tail(&stderr);
            return Err(BackendError::Process(format!(
                "{id} exited with {status}{}",
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(": {tail}")
                },
                id = self.id
            )));
        }
        validate_response(&response, file_len_s, file_len_s)
    }
}

impl DetectorBackend for ExternalDetector {
    fn detect(
        &self,
        x: &[f32],
        sr: u32,
        role: &str,
    ) -> Result<(Vec<DetectedHit>, Option<CandidateList>), BackendError> {
        // the temp WAV of this track, next to nothing else (the app passes its own file; core
        // writes one scratch file per call)
        let mut path = std::env::temp_dir();
        path.push(format!("drumspike-plugin-{}-{}.wav", self.id, std::process::id()));
        let spec = WavSpec {
            sample_rate: sr,
            channels: 1,
            format: WavFormat::Float32,
            extensible: false,
            channel_mask: 0,
        };
        crate::io::write_wav(&path, &spec, x, None).map_err(|e| BackendError::Io(e.to_string()))?;
        let file_len_s = x.len() as f64 / f64::from(sr);
        let result = self.detect_wav(&path, role, sr, file_len_s);
        let _ = std::fs::remove_file(&path);
        let v = result?;
        // the class stays the internal index; the dispatch maps it to the note
        Ok((v.hits, v.candidates))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(t: f64, class: &str, confidence: Option<f32>) -> ProtocolHit {
        ProtocolHit {
            t,
            class: class.into(),
            confidence,
        }
    }

    fn response(hits: Vec<ProtocolHit>) -> PluginResponse {
        PluginResponse {
            hits,
            ..Default::default()
        }
    }

    /// Section 5, row 1: every `t` finite and inside the file; out-of-range hits are dropped
    /// and counted (NaN counts as out of range).
    #[test]
    fn out_of_range_hits_are_dropped_and_counted() {
        let v = validate_response(
            &response(vec![
                hit(0.5, "kick", None),
                hit(-1.0, "kick", None),
                hit(f64::NAN, "kick", None),
            ]),
            10.0,
            10.0,
        )
        .unwrap();
        assert_eq!(
            v.hits,
            vec![DetectedHit {
                time: 0.5,
                class: 0,
                confidence: 1.0
            }]
        );
        assert_eq!(v.dropped_out_of_range, 2);
    }

    /// Section 5, row 2: unsorted hits are sorted by time (the min-gap dedup needs order).
    #[test]
    fn unsorted_hits_are_sorted() {
        let v = validate_response(
            &response(vec![hit(2.0, "kick", None), hit(1.0, "kick", None)]),
            10.0,
            10.0,
        )
        .unwrap();
        let times: Vec<f64> = v.hits.iter().map(|h| h.time).collect();
        assert_eq!(times, vec![1.0, 2.0]);
        assert!(v.sorted);
    }

    /// Section 5, row 3: unknown classes are dropped and counted; the known classes map to
    /// the internal index.
    #[test]
    fn unknown_classes_are_dropped_and_counted() {
        let v = validate_response(
            &response(vec![
                hit(1.0, "kick", None),
                hit(2.0, "cowbell", None),
                hit(3.0, "other", None),
            ]),
            10.0,
            10.0,
        )
        .unwrap();
        let classes: Vec<u8> = v.hits.iter().map(|h| h.class).collect();
        assert_eq!(classes, vec![0, 5]);
        assert_eq!(v.dropped_unknown_class, 1);
    }

    /// Section 5, row 4: the confidence is clamped to 0 to 1; a missing one is 1.0.
    #[test]
    fn confidence_is_clamped_and_defaults_to_one() {
        let v = validate_response(
            &response(vec![
                hit(1.0, "kick", Some(2.0)),
                hit(2.0, "kick", Some(-0.5)),
                hit(3.0, "kick", None),
            ]),
            10.0,
            10.0,
        )
        .unwrap();
        let confidences: Vec<f32> = v.hits.iter().map(|h| h.confidence).collect();
        assert_eq!(confidences, vec![1.0, 0.0, 1.0]);
    }

    /// Section 5, limits: more than 50 hits per second of audio fails the track.
    #[test]
    fn the_hits_per_second_limit_fails_the_track() {
        let hits: Vec<ProtocolHit> = (0..51).map(|i| hit(f64::from(i) * 0.01, "kick", None)).collect();
        let e = validate_response(&response(hits), 1.0, 1.0).unwrap_err();
        assert!(matches!(e, BackendError::Limit(_)), "{e}");
    }

    /// Section 5, limits: more than 400 candidates per second fails the track.
    #[test]
    fn the_candidates_per_second_limit_fails_the_track() {
        let cands: Vec<ProtocolHit> = (0..401).map(|i| hit(f64::from(i) * 0.001, "kick", None)).collect();
        let mut r = response(vec![]);
        r.candidates = Some(cands);
        let e = validate_response(&r, 1.0, 1.0).unwrap_err();
        assert!(matches!(e, BackendError::Limit(_)), "{e}");
    }

    /// Section 5, limits: more than 100000 hits on one track fails the track.
    #[test]
    fn the_hits_per_track_limit_fails_the_track() {
        let hits: Vec<ProtocolHit> = (0..MAX_HITS + 1).map(|i| hit(i as f64 * 0.001, "kick", None)).collect();
        let e = validate_response(&response(hits), 1e9, 1e9).unwrap_err();
        assert!(matches!(e, BackendError::Limit(_)), "{e}");
    }

    /// Section 5, limits: more than 1000000 candidates on one track fails the track.
    #[test]
    fn the_candidates_per_track_limit_fails_the_track() {
        let cands: Vec<ProtocolHit> = (0..MAX_CANDIDATES + 1)
            .map(|i| hit(i as f64 * 0.001, "kick", None))
            .collect();
        let mut r = response(vec![]);
        r.candidates = Some(cands);
        let e = validate_response(&r, 1e9, 1e9).unwrap_err();
        assert!(matches!(e, BackendError::Limit(_)), "{e}");
    }

    /// Section 5: a response naming another protocol major is refused with the message of
    /// section 4; a response without the field is protocol 1.
    #[test]
    fn a_protocol_major_mismatch_is_refused() {
        let mut r = response(vec![]);
        r.protocol = Some(2);
        let e = validate_response(&r, 10.0, 10.0).unwrap_err();
        assert_eq!(
            e,
            BackendError::Protocol("plugin speaks protocol 2, this app speaks protocol 1".into())
        );
        assert!(validate_response(&response(vec![]), 10.0, 10.0).is_ok());
    }

    /// Section 5: `{"error": ...}` comes back as the track's error sentence.
    #[test]
    fn an_error_response_becomes_the_track_error() {
        let mut r = response(vec![]);
        r.error = Some("the model file is missing".into());
        let e = validate_response(&r, 10.0, 10.0).unwrap_err();
        assert_eq!(e, BackendError::Plugin("the model file is missing".into()));
    }
}
