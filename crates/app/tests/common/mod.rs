//! Shared helpers of the integration tests: temporary folders, a synthetic kick track as a
//! WAV file, a project folder with tracks, the key-set `shape` of a JSON value and a wait for
//! the analysis job.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use drumspike_app::{Service, ServiceConfig};
use drumspike_core::io::{WavFormat, WavSpec, encode_wav};
use drumspike_core::project::{add_track, build_source, new_project, save_project};
use serde_json::Value;

/// Remove the folders of earlier runs, once per test binary. A `ds_app_it_*` folder that is
/// not touched for six hours belongs to no running test; each run leaves about 100 of them
/// (2026-10-14: 23,317 folders, 41 GB in the temp folder of the dev machine).
fn sweep_old_runs() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
            return;
        };
        for e in entries.flatten() {
            if !e.file_name().to_string_lossy().starts_with("ds_app_it_") {
                continue;
            }
            let age = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok());
            if age.is_some_and(|a| a > Duration::from_secs(6 * 3600)) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    });
}

/// A fresh temporary folder per test.
pub fn tmp(tag: &str) -> PathBuf {
    sweep_old_runs();
    let d = std::env::temp_dir().join(format!("ds_app_it_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// tests/helpers.kick_hit: pitch sweep 125 -> 55 Hz, 0.5 ms attack ramp, 0.12 s decay.
pub fn kick_signal(onsets: &[f64], dur: f64, sr: u32) -> Vec<f32> {
    let srf = f64::from(sr);
    let mut x = vec![0.0f64; (dur * srf) as usize];
    for &t0 in onsets {
        let i0 = (t0 * srf).round() as usize;
        let g = 10f64.powf(-3.0 / 20.0);
        let mut cum = 0.0;
        for k in 0..((0.4 * srf) as usize) {
            if i0 + k >= x.len() {
                break;
            }
            let t = k as f64 / srf;
            cum += 55.0 + 70.0 * (-t / 0.03).exp();
            let env = (t / 0.0005).min(1.0) * (-t / 0.12).exp();
            x[i0 + k] += (2.0 * std::f64::consts::PI * cum / srf).sin() * env * g;
        }
    }
    x.iter().map(|&v| v as f32).collect()
}

pub const ONSETS: [f64; 6] = [0.25, 0.75, 1.25, 1.75, 2.25, 2.75];

/// Steady pink-ish noise (no hits, no pitch): the core scores it None, so no audio suggestion.
pub fn noise_signal(dur: f64, sr: u32) -> Vec<f32> {
    let srf = f64::from(sr);
    let n = (dur * srf) as usize;
    let g = 10f64.powf(-12.0 / 20.0);
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut state = 0.0f64;
    (0..n)
        .map(|_| {
            // xorshift for the white part, a one-pole lowpass for the pink tilt
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let white = (seed as f64 / u64::MAX as f64) * 2.0 - 1.0;
            state += 0.04 * (white - state);
            (state * 6.0 * g) as f32
        })
        .collect()
}

/// Write a mono float32 WAV file.
pub fn write_wav(path: &Path, sr: u32, x: &[f32]) {
    let spec = WavSpec {
        sample_rate: sr,
        channels: 1,
        format: WavFormat::Float32,
        extensible: false,
        channel_mask: 0,
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, encode_wav(&spec, x, None)).unwrap();
}

/// A track of [`project`]: name, role, MIDI note and the signal.
pub struct Spec {
    pub name: &'static str,
    pub role: &'static str,
    pub note: i64,
    pub x: Vec<f32>,
    pub sr: u32,
}

pub fn kick_spec(name: &'static str, role: &'static str, note: i64) -> Spec {
    kick_spec_dur(name, role, note, 3.5)
}

/// [`kick_spec`] with a clip of `dur` seconds: the 0.5 s onset grid of [`ONSETS`] continues
/// past its last entry (3.25, 3.75, ...) while it fits, so the clip has no silent tail.
/// `dur` 3.5 is exactly [`kick_spec`].
pub fn kick_spec_dur(name: &'static str, role: &'static str, note: i64, dur: f64) -> Spec {
    let mut onsets: Vec<f64> = ONSETS.to_vec();
    let mut t = 3.25;
    while t + 0.5 < dur {
        onsets.push(t);
        t += 0.5;
    }
    Spec {
        name,
        role,
        note,
        x: kick_signal(&onsets, dur, 48_000),
        sr: 48_000,
    }
}

/// A project folder `<root>/<project name>/` with the given tracks (referenced WAV files in
/// `<root>/daw/`), saved. The track records carry the keys the server's upload writes.
pub fn project(root: &Path, project_name: &str, specs: &[Spec]) -> PathBuf {
    let mut p = new_project(root, project_name).unwrap();
    for s in specs {
        let wav = root.join("daw").join(format!("{}.wav", s.name));
        write_wav(&wav, s.sr, &s.x);
        let rec = build_source(&wav, &p.dir, "referenced").unwrap();
        let tr = add_track(&mut p, s.name, Some(rec), s.role, s.note, Some("average")).unwrap();
        tr.sr = Some(i64::from(s.sr));
        tr.n_samples = Some(s.x.len() as i64);
        tr.duration = Some(s.x.len() as f64 / f64::from(s.sr));
        tr.color = Some("#e05555".into());
    }
    save_project(&mut p).unwrap();
    p.dir
}

/// A service without a model file: the DSP fallback runs.
pub fn service_dsp() -> Service {
    Service::new(ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    })
}

/// The SHA-256 of a file as lowercase hex (was `detect::model::file_sha256`; the tests hash
/// audio files and documents to prove that an operation left them unchanged).
pub fn file_sha256(path: impl AsRef<Path>) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(hasher.finalize()))
}

/// Wait until the job of `sid` is `done` (60 s at most).
pub fn wait_done(svc: &Service, sid: &str) -> Value {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let st = svc.analyze_status(sid);
        if st["state"] == "done" {
            return st;
        }
        assert!(Instant::now() < end, "the job did not finish: {st}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The key-set shape of a value (the same function as `shape` in `tests/test_app_shapes.py`):
/// an object is its sorted keys with the shapes of its values; a list inside an object is
/// "list"; a list at the top is a one-element list with the shape of the first row; a scalar
/// is null.
pub fn shape(v: &Value) -> Value {
    shape_at(v, true)
}

fn shape_at(v: &Value, top: bool) -> Value {
    match v {
        Value::Array(a) if top && !a.is_empty() => Value::Array(vec![shape_at(&a[0], false)]),
        Value::Array(_) => Value::from("list"),
        Value::Object(m) => {
            let sorted: BTreeMap<&String, Value> = m.iter().map(|(k, x)| (k, shape_at(x, false))).collect();
            serde_json::to_value(sorted).unwrap()
        }
        _ => Value::Null,
    }
}

/// The last progress message of kind `key` once its state is no longer `running`. A job sets its final
/// state before it sends the final message, so a status poll can see the end a moment before the message
/// arrives; waiting here (up to 5 s) removes that race. Returns the last state seen.
pub fn final_state(seen: &Arc<Mutex<Vec<Value>>>, key: &str) -> Value {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let last = seen
            .lock()
            .unwrap()
            .last()
            .map(|v| v[key]["state"].clone())
            .unwrap_or(Value::Null);
        if (last != "running" && !last.is_null()) || Instant::now() > end {
            return last;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Lowercase hex of bytes (sha2 0.11 no longer implements LowerHex for the digest).
pub fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
