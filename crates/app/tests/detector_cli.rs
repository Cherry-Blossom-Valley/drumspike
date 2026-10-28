//! #388: the CLI runs an analysis with the example plugin
//! (tests/fixtures/plugins/json_hits.py) against a fixture WAV, and the hits match the
//! plugin's times.json. The plugin is installed into a temp Detectors folder that
//! `DRUMSPIKE_DETECTORS_DIR` names (see crates/app/src/plugins.rs).

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use common::*;
use drumspike_app::cli::{self, Io};
use drumspike_core::project::load_project;
use serde_json::{json, Value};

const TIMES: [f64; 6] = [0.25, 0.75, 1.25, 1.75, 2.25, 2.75];

/// A shared buffer as a writer (the CLI's `Io` needs owned writers; the clones share).
#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);
impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Buf {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn io(input: &str) -> (Io, Buf, Buf) {
    let (out, err) = (Buf::default(), Buf::default());
    let err_arc: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(Box::new(err.clone())));
    (
        Io {
            input: Box::new(std::io::Cursor::new(input.as_bytes().to_vec())),
            out: Box::new(out.clone()),
            err: err_arc,
        },
        out,
        err,
    )
}

/// The example plugin (tests/fixtures/plugins/json_hits.py) as a real install: the fixture
/// script, times.json and the manifest.
fn install_plugin(dir: &std::path::Path) {
    let folder = dir.join("json-hits");
    std::fs::create_dir_all(&folder).unwrap();
    let exe = folder.join("json_hits.py");
    std::fs::copy(repo_root().join("tests/fixtures/plugins/json_hits.py"), &exe).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        folder.join("times.json"),
        serde_json::to_string(&json!({"kick": TIMES})).unwrap(),
    )
    .unwrap();
    std::fs::write(folder.join("LICENCE"), "MIT License\n\nPermission is hereby granted.\n").unwrap();
    std::fs::write(
        folder.join("manifest.json"),
        serde_json::to_string(&json!({
            "id": "json-hits", "name": "JSON Hits", "version": "1.0.0", "protocol": 1,
            "executable": "json_hits.py",
            "roles": ["kick", "snare", "tom", "hat", "cymbal", "other"],
            "sample_rates": [44100, 48000, 88200, 96000, 192000],
            "licence": "MIT", "licence_file": "LICENCE",
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn the_cli_runs_an_analysis_with_the_example_plugin() {
    let root = tmp("detector_cli");
    install_plugin(&root.join("detectors"));
    std::env::set_var("DRUMSPIKE_DETECTORS_DIR", root.join("detectors"));

    let svc = service_dsp();
    let dir = project(&root, "Song", &[kick_spec("Kick", "kick", 36)]);
    let path = dir.display().to_string();

    // the analysis with the flag: one shot over --project (waits for the job)
    let (mut i, out, err) = io("");
    let code = cli::run(
        &svc,
        &[
            "analyze_start".into(),
            "--project".into(),
            path.clone(),
            "--detector".into(),
            "json-hits".into(),
            "--tracks".into(),
            "Kick".into(),
        ],
        &mut i,
    );
    assert_eq!(code, 0, "out {} err {}", out.text(), err.text());
    let result: Value = serde_json::from_str(out.text().trim()).expect("one JSON line");
    assert_eq!(result["started"], json!(["Kick"]), "{result}");

    // the hits of the stored project are the plugin's times (times.json)
    let p = load_project(&dir).unwrap();
    let events = p.doc.track("Kick").unwrap().events.as_ref().expect("events");
    let times: Vec<f64> = events.iter().map(|e| e.t).collect();
    assert_eq!(times.len(), TIMES.len(), "{times:?}");
    for (t, want) in times.iter().zip(TIMES) {
        assert!((t - want).abs() < 0.01, "{times:?} vs {want}");
    }

    // the record of the document names the plugin (the section 6 format)
    let rec = &p.doc.track("Kick").unwrap().extra["detector"];
    assert_eq!(rec["detector"], "json-hits");
    assert_eq!(rec["plugin"]["id"], "json-hits");
    assert_eq!(rec["plugin"]["version"], "1.0.0");
    assert_eq!(rec["plugin"]["licence_sha256"].as_str().unwrap().len(), 64);

    // and `--detector builtin` (the default) runs the DSP detector
    let (mut i, _out, _err) = io("");
    let code = cli::run(
        &svc,
        &[
            "analyze_start".into(),
            "--project".into(),
            path,
            "--detector".into(),
            "builtin".into(),
            "--tracks".into(),
            "Kick".into(),
        ],
        &mut i,
    );
    assert_eq!(code, 0);
    let p = load_project(&dir).unwrap();
    let rec = &p.doc.track("Kick").unwrap().extra["detector"];
    assert_eq!(rec["detector"], "rust-dsp");
    assert!(rec.get("plugin").is_none(), "{rec}");
    let _ = std::fs::remove_dir_all(&root);
    std::env::remove_var("DRUMSPIKE_DETECTORS_DIR");
}
