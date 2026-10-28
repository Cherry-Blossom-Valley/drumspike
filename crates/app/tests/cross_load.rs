//! Cross-load (P3-4a, I2): a project the app writes loads in the retired server's layer (project.py) with
//! the same name, order, notes, source modes and bit-exact event times, and a project the server's layer
//! wrote loads in the app. Four cases: app writes / server loads and server writes / app loads, each for a
//! referenced and a copied project.
//!
//! P3-12: the server's side is frozen in `tests/golden/frozen/app_cross_load.json` (recorded by
//! `tools/parity/cross_load.py` at the commit named there): `write/<case>` holds the two documents
//! project.py wrote and the views it read back; `check/<case>` holds the document the app had saved when
//! project.py read it and that view. Paths below the test folder are `{ROOT}`; `created`, `modified` and the
//! source `mtime` are left out. "App writes, server loads" now means: the app writes the same document as at
//! the freeze, which the server read as `want`. "Server writes, app loads": the documents are written again
//! from the frozen ones (the audio is the stdlib tone of cross_load.py, checked against the frozen SHA-256).

mod common;

use sha2::Digest;
use std::path::Path;

use common::*;
use drumspike_core::project::{add_track, build_source, copy_into_project, new_project, save_project};
use serde_json::{json, Value};

/// The frozen value of `key` (one host's value, else the shared one).
fn frozen(key: &str) -> Value {
    let path = repo_root().join("tests/golden/frozen/app_cross_load.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let e = &v["entries"][key];
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    let one = if e.get("all").is_some() {
        &e["all"]
    } else {
        &e["by_host"][arch]
    };
    assert!(one.is_object(), "no frozen value for {key} on {arch}");
    one["value"].clone()
}

/// The document without what changes from run to run: the test folder (`.../ds_app_it_<tag>_<pid>`) becomes
/// `{ROOT}`; `created`, `modified` and `mtime` are left out (cross_load.py `normalize`).
fn normalize(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "created" | "modified" | "mtime"))
                .map(|(k, x)| (k.clone(), normalize(x)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(normalize).collect()),
        Value::String(s) => match s.find("ds_app_it_") {
            Some(i) => {
                let rest = &s[i..];
                let end = rest.find('/').unwrap_or(rest.len());
                Value::from(format!("{{ROOT}}{}", &rest[end..]))
            }
            None => v.clone(),
        },
        _ => v.clone(),
    }
}

fn saved_document(folder: &Path) -> Value {
    let doc = std::fs::read_dir(folder)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x == "drumspike"))
        .expect("a document");
    serde_json::from_str(&std::fs::read_to_string(doc).unwrap()).unwrap()
}

/// cross_load.py `tone`: a mono 16-bit 48 kHz WAV of `int(12000 * sin(2 pi hz i / 48000))`, as Python's
/// `wave` module writes it.
fn tone(path: &Path, seconds: f64, hz: f64) {
    let sr = 48_000u32;
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
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, out).unwrap();
}

fn bits(x: f64) -> String {
    x.to_le_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// The events both sides use: awkward doubles and every flag.
fn events() -> Vec<Value> {
    let t = [
        0.1 + 0.2,
        1.0 / 3.0,
        2.675,
        1e-7,
        123_456.123_456_789,
        0.25,
        48001.0 / 48000.0,
    ];
    vec![
        json!({"t": t[0], "v": 100}),
        json!({"t": t[1], "v": 20, "added": true}),
        json!({"t": t[2], "v": 127, "orig": t[3], "edited": true}),
        json!({"t": t[4], "v": 64, "rise": 12.3}),
        json!({"t": t[5], "v": 1, "rise_db": 7.25, "id": "l5"}),
        json!({"t": t[6], "v": 99, "orig": t[6]}),
    ]
}

/// `[t bits, v, orig bits or null]` of an event, the shape of the script's view.
fn event_view(e: &Value) -> Value {
    json!([
        bits(e["t"].as_f64().unwrap()),
        e["v"],
        e.get("orig")
            .and_then(Value::as_f64)
            .map_or(Value::Null, |o| Value::from(bits(o)))
    ])
}

fn views(list: &Value) -> Value {
    Value::Array(list.as_array().unwrap().iter().map(event_view).collect())
}

/// Build `<root>/projects/<name>` with the core and save it (the app's documents are written by
/// the same functions): a kick and a snare WAV file, referenced or copied.
fn build(root: &Path, name: &str, copied: bool) -> std::path::PathBuf {
    let mut p = new_project(&root.join("projects"), name).unwrap();
    for (tname, role, note) in [("Kick In", "kick", 36), ("Snare Top", "snare", 38)] {
        let wav = root.join("daw").join(format!("{tname}.wav"));
        write_wav(&wav, 48_000, &kick_signal(&ONSETS, 1.5, 48_000));
        let mut rec = build_source(&wav, &p.dir, "referenced").unwrap();
        if copied {
            rec = copy_into_project(&rec, &p.dir, None).unwrap();
        }
        let tr = add_track(&mut p, tname, Some(rec), role, note, Some("average")).unwrap();
        tr.sr = Some(48_000);
        tr.n_samples = Some(72_000);
        tr.duration = Some(1.5);
    }
    save_project(&mut p).unwrap();
    p.dir
}

/// The app edits the project and saves it; the server's layer loads the folder.
fn app_writes_server_loads(tag: &str, copied: bool) {
    let root = tmp(tag);
    let dir = build(&root, "AppProject", copied);
    let svc = service_dsp();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.events_set(&sid, "Kick In", &Value::Array(events())).unwrap();
    svc.events_set(&sid, "Snare Top", &json!([{"t": 0.5, "v": 77}]))
        .unwrap();
    svc.track_set_note(&sid, "Kick In", 35).unwrap();
    svc.tracks_reorder(&sid, &json!(["Snare Top", "Kick In"])).unwrap();
    svc.project_rename(&sid, "App Renamed").unwrap();
    svc.project_save(&sid).unwrap();
    let frozen = frozen(&format!("check/{tag}"));
    assert_eq!(
        normalize(&saved_document(&dir)),
        normalize(&frozen["document"]),
        "the app writes the document the server's layer read at the freeze"
    );
    let got = frozen["view"].clone();
    let mode = if copied { "copied" } else { "referenced" };
    let want = json!({
        "name": "App Renamed",
        "order": ["Snare Top", "Kick In"],
        "format": 1,
        "tracks": {
            "Kick In": {"role": "kick", "note": 35, "mode": mode, "status": "ok", "events": views(&Value::Array(events()))},
            "Snare Top": {"role": "snare", "note": 38, "mode": mode, "status": "ok", "events": views(&json!([{"t": 0.5, "v": 77}]))},
        },
    });
    assert_eq!(got, want);
}

/// The server's layer wrote both projects (frozen); the app loads one of them.
fn server_writes_app_loads(tag: &str, name: &str, mode: &str) {
    let root = tmp(tag);
    let written = frozen(&format!("write/{tag}"));
    for (rel, sha) in written["audio"].as_object().unwrap() {
        let hz = if rel.contains("Kick In") { 60.0 } else { 200.0 };
        let path = root.join(rel);
        tone(&path, 1.0, hz);
        let bytes = std::fs::read(&path).unwrap();
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let got = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(got, sha.as_str().unwrap(), "{rel}: the frozen tone file");
    }
    for (pname, doc) in written["documents"].as_object().unwrap() {
        let folder = root.join("projects").join(pname);
        for sub in ["Audio Files", "Analysis", "Exports", "Backups"] {
            std::fs::create_dir_all(folder.join(sub)).unwrap();
        }
        let text = serde_json::to_string_pretty(doc)
            .unwrap()
            .replace("{ROOT}", root.to_str().unwrap());
        std::fs::write(folder.join(format!("{pname}.drumspike")), text).unwrap();
    }
    let want = &written["views"][name];
    assert_eq!(
        want["tracks"]["Kick In"]["mode"], mode,
        "the frozen run wrote what the test expects"
    );
    let svc = service_dsp();
    let v = svc
        .project_open(root.join("projects").join(name).to_str().unwrap())
        .unwrap();
    let sid = v["sid"].as_str().unwrap();
    assert_eq!(v["name"], want["name"]);
    assert_eq!(v["order"], want["order"]);
    assert_eq!(v["format"], 1);
    let tracks = svc.tracks_get(sid).unwrap();
    assert_eq!(tracks["order"], want["order"]);
    for (tname, w) in want["tracks"].as_object().unwrap() {
        let t = &tracks["tracks"][tname];
        assert_eq!(t["role"], w["role"], "{tname}");
        assert_eq!(t["note"], w["note"], "{tname}");
        assert_eq!(t["source_mode"], w["mode"], "{tname}");
        assert_eq!(t["status"], w["status"], "{tname}");
        let ev = svc.events_get(sid, tname).unwrap();
        assert_eq!(views(&ev["events"]), w["events"], "{tname}: event times, bit for bit");
    }
    // the app saves it back: the document is the one the server's layer read again at the freeze
    svc.track_set_note(sid, "Kick In", 36).unwrap();
    svc.project_save(sid).unwrap();
    let again = frozen(&format!("check/{tag}"));
    assert_eq!(
        normalize(&saved_document(&root.join("projects").join(name))),
        normalize(&again["document"])
    );
    assert_eq!(
        again["view"]["tracks"]["Kick In"]["events"],
        want["tracks"]["Kick In"]["events"]
    );
    assert_eq!(again["view"]["tracks"]["Snare Top"]["status"], "ok");
}

/// A planted change in one frozen event fails the comparison of the documents (P3-12).
#[test]
fn cross_load_a_planted_event_change_is_seen() {
    let mut doc = frozen("check/xl_app_ref")["document"].clone();
    let before = normalize(&doc);
    doc["tracks"]["Kick In"]["events"][0]["t"] = json!(0.30000000000000004 + 1e-15);
    assert_ne!(normalize(&doc), before);
}

#[test]
fn cross_load_1_app_writes_referenced_server_loads() {
    app_writes_server_loads("xl_app_ref", false);
}

#[test]
fn cross_load_2_app_writes_copied_server_loads() {
    app_writes_server_loads("xl_app_copy", true);
}

#[test]
fn cross_load_3_server_writes_referenced_app_loads() {
    server_writes_app_loads("xl_srv_ref", "ServerRef", "referenced");
}

#[test]
fn cross_load_4_server_writes_copied_app_loads() {
    server_writes_app_loads("xl_srv_copy", "ServerCopy", "copied");
}

#[test]
fn cross_load_check_can_fail() {
    assert_ne!(bits(0.1 + 0.2), bits(0.3));
    assert_eq!(
        event_view(&json!({"t": 0.5, "v": 77})),
        json!(["000000000000e03f", 77, null])
    );
}
