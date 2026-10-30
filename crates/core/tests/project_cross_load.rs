//! Cross-load (P1-6, I2): documents written by the Python layer of the retired server (`project.py`, through
//! the helpers of its `tests/test_project.py`) load in Rust with bit-exact event times, velocities and
//! flags, and the Rust loader's view equals Python's `load_project` result key for key.
//!
//! P3-12: the fixtures are frozen in `tests/golden/project_fixtures/` (written by
//! `tools/parity/write_project_fixtures.py --freeze` at the commit named in
//! `docs/archive/phase3-report.md`, section "P3-12"). `expected.json` holds per fixture the folder,
//! the document as `project.load_project` returned it and the IEEE bits of every event time. No audio is
//! in git: the legacy session's `source.wav` is a `tone()` file that this test writes again (same bytes,
//! same mtime, checked against the frozen SHA-256), because the legacy import builds a source record from
//! it. The other direction (Rust writes, Python loads) was `check_project.py`; it went with project.py,
//! the app is the only reader left.

use std::path::{Path, PathBuf};

use drumspike_core::project::{Document, legacy_session_document, load_project, migrate_forward};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap()
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to);
        } else if e.file_name() != ".keep" {
            std::fs::copy(e.path(), &to).unwrap();
        }
    }
}

/// `write_project_fixtures.tone`: a mono 16-bit 48 kHz WAV of `int(12000 * sin(2 pi hz i / sr))`, written
/// as Python's `wave` module writes it (a 44-byte header).
fn tone_wav(seconds: f64, hz: f64, sr: u32) -> Vec<u8> {
    let n = (seconds * f64::from(sr)) as usize;
    let mut data = Vec::with_capacity(2 * n);
    for i in 0..n {
        let v = (12000.0 * (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(sr)).sin()) as i16;
        data.extend_from_slice(&v.to_le_bytes());
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(2 * sr).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// The frozen fixtures in a fresh temporary folder; returns (folder, expected.json).
fn fixtures(dir: &Path) -> Value {
    let frozen = repo_root().join("tests/golden/project_fixtures");
    copy_dir(&frozen, dir);
    let v: Value = serde_json::from_str(&std::fs::read_to_string(frozen.join("expected.json")).unwrap()).unwrap();
    assert!(
        v["fixtures"].as_object().is_some_and(|f| f.len() == 4),
        "expected.json shape"
    );
    for (rel, a) in v["audio"].as_object().unwrap() {
        if a["tone"] != true {
            continue; // the projects' audio is never read by load_project
        }
        let bytes = tone_wav(1.5, 60.0, 48_000);
        assert_eq!(
            sha256_hex(&bytes),
            a["sha256"].as_str().unwrap(),
            "{rel}: the tone file is the frozen one"
        );
        let path = dir.join(rel);
        std::fs::write(&path, &bytes).unwrap();
        let mtime = std::time::UNIX_EPOCH + std::time::Duration::from_secs(a["mtime"].as_u64().unwrap());
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
    }
    v
}

/// Python `==` on JSON values: `1 == 1.0`, key order ignored, everything else exact.
/// (`serde_json::Value` treats the integer 1 and the float 1.0 as different numbers; the UI
/// writes `"t": 1` for a hit at exactly 1.0 s and the Rust `Event` holds `t` as a double.)
fn py_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y || x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| py_eq(p, q)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, p)| y.get(k).is_some_and(|q| py_eq(p, q)))
        }
        _ => a == b,
    }
}

/// The IEEE bit pattern as the Python side writes it (`struct.pack("<d", x).hex()`).
fn bits(x: f64) -> String {
    x.to_le_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn check_document(name: &str, doc: &Document, fx: &Value) {
    // every key equal to Python's load_project view (serde_json Value equality is exact on f64)
    let got = serde_json::to_value(doc).unwrap();
    assert!(
        py_eq(&got, &fx["doc"]),
        "{name}: the Rust loader's document differs from Python's:\n{got}\n{}",
        fx["doc"]
    );
    // and the event times and velocities bit for bit, track by track
    let mut n = 0;
    for (track, expected) in fx["bits"].as_object().unwrap() {
        let events = doc
            .track(track)
            .unwrap_or_else(|| panic!("{name}: track {track}"))
            .events();
        let expected = expected.as_array().unwrap();
        assert_eq!(events.len(), expected.len(), "{name}/{track}: event count");
        for (e, want) in events.iter().zip(expected) {
            assert_eq!(bits(e.t), want[0].as_str().unwrap(), "{name}/{track}: t");
            assert_eq!(
                e.v.map(Value::from).unwrap_or(Value::Null),
                want[1],
                "{name}/{track}: v"
            );
            match (&e.orig, &want[2]) {
                (Some(o), Value::String(s)) => assert_eq!(&bits(*o), s, "{name}/{track}: orig"),
                (None, Value::Null) => {}
                other => panic!("{name}/{track}: orig {other:?}"),
            }
            n += 1;
        }
    }
    assert!(n > 0, "{name}: no events compared");
    eprintln!("{name}: {n} events bit-identical, document equal");
}

#[test]
fn documents_written_by_python_load_bit_exact() {
    let dir = std::env::temp_dir().join(format!("drumspike-core-cross-load-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let frozen = fixtures(&dir);
    for (name, fx) in frozen["fixtures"].as_object().unwrap() {
        let path = dir.join(fx["path"].as_str().unwrap());
        let mut fx = fx.clone();
        let doc = if fx["kind"] == "session" {
            // the source record of the legacy import points at the file this test wrote
            let wav = path.join("tracks/Kick_In/source.wav");
            fx["doc"]["tracks"]["Kick_In"]["source"]["path"] = Value::from(wav.to_str().unwrap());
            migrate_forward(legacy_session_document(&path).unwrap()).unwrap()
        } else {
            let p = load_project(&path).unwrap();
            assert_eq!(p.dir, path.to_path_buf(), "{name}: project folder");
            p.doc
        };
        check_document(name, &doc, &fx);
        // the odd-named fixture carries unknown keys at every level: they are all there
        if name == "odd_named" {
            assert_eq!(doc.extra["future_key"], serde_json::json!([1, 2, {"x": null}]));
            assert_eq!(doc.settings.as_ref().unwrap().extra["theme"], "dark");
            let k = doc.track("K").unwrap();
            assert_eq!(k.extra["lane_height"], 120);
            assert_eq!(k.events()[0].extra["tag"], "x");
            assert_eq!(k.source, Some(None));
            assert_eq!(doc.track("NoSource").unwrap().source, None);
            assert_eq!(doc.created.as_ref().unwrap().as_i64(), Some(1759950000));
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A planted change in one frozen event fails the comparison (P3-12: the golden check can fail).
#[test]
fn a_planted_event_change_is_seen() {
    let dir = std::env::temp_dir().join(format!("drumspike-core-cross-load-plant-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let frozen = fixtures(&dir);
    let mut fx = frozen["fixtures"]["two_track"].clone();
    let doc = load_project(&dir.join(fx["path"].as_str().unwrap())).unwrap().doc;
    fx["doc"]["tracks"]["Kick In"]["events"][1]["t"] = Value::from(1.0 / 3.0 + 1e-12);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check_document("planted", &doc, &fx)));
    assert!(r.is_err(), "a changed event time must fail");
    std::fs::remove_dir_all(&dir).unwrap();
}
