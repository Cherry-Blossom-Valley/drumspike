//! Ticket #120 step 2: the owner's example job headless through `drumspike run` on a demo-style
//! session folder (a Pro Tools session folder with an `Audio Files/` subfolder and an ignored
//! `.ptx`), track selection by name pattern, per-role params, and the export per role. The test
//! checks the stored params per track and the exported MIDI and spike files per role and fails
//! on any missing file or value.

mod common;

use common::*;
use drumspike_app::cli::{Io, run};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for Buf {
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
    (
        Io {
            input: Box::new(std::io::Cursor::new(input.as_bytes().to_vec())),
            out: Box::new(out.clone()),
            err: std::sync::Arc::new(std::sync::Mutex::new(Box::new(err.clone()))),
        },
        out,
        err,
    )
}

/// The demo session folder: a Pro Tools session (the `.ptx` at the top, the audio in
/// `Audio Files/`), 13 mics in the owner's name scheme, kick 36, snare 38, toms 41, 45, 47.
fn session(root: &std::path::Path) -> std::path::PathBuf {
    let audio = root.join("demo-session").join("Audio Files");
    let mics: &[(&str, &str, i64)] = &[
        ("k in", "kick", 36),
        ("k out", "kick", 36),
        ("s top", "snare", 38),
        ("s bot", "snare", 38),
        ("t1", "tom", 41),
        ("t2", "tom", 45),
        ("t3", "tom", 47),
        ("hh", "hat", 42),
        ("ride", "cymbal", 51),
        ("oh l", "cymbal", 49),
        ("oh r", "cymbal", 49),
        ("room", "other", 44),
        ("new take", "kick", 36),
    ];
    for (name, role, note) in mics {
        let s = kick_spec_box(name, role, *note);
        write_wav(&audio.join(format!("{name}.wav")), s.sr, &s.x);
    }
    std::fs::create_dir_all(root.join("demo-session")).unwrap();
    std::fs::write(root.join("demo-session").join("song.ptx"), b"not parsed").unwrap();
    root.join("demo-session")
}

fn kick_spec_box(name: &str, role: &str, note: i64) -> Spec {
    // 3.0 s so the 0.5 s onset grid has hits for every mic
    let name: &'static str = Box::leak(name.to_string().into_boxed_str());
    let role: &'static str = Box::leak(role.to_string().into_boxed_str());
    kick_spec_dur(name, role, note, 3.0)
}

fn lines(out: &Buf) -> Vec<Value> {
    out.text().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn project_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join("projects")
}

#[test]
fn the_example_job_runs_headless_and_exports_per_role() {
    let root = tmp("agent_job");
    let session = session(&root);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project_dir(&root).display().to_string();
    let script = json!([
        {"command": "project_new", "args": {"name": "Demo", "dir": dir}},
        // the session folder stands for its Audio Files; `tracks` filters the scanned files
        {"command": "tracks_add", "args": {"sid": "$sid", "paths": [session.display().to_string()],
            "tracks": ["k in", "s top", "t1-t3"], "roles": ["kick", "snare", "tom", "tom", "tom"],
            "notes": [36, 38, 41, 45, 47]}},
        {"command": "tracks_get", "args": {"sid": "$sid"}},
        // per-role params on the pattern-selected tracks, then the analysis (wait for it)
        {"command": "analyze_start", "wait": true, "args": {"sid": "$sid",
            "tracks": ["k in", "s top", "t1-t3"],
            "params": {"k_in": {"sensitivity": 55}, "s_top": {"sensitivity": 60, "vel_hi_db": -8},
                       "t1": {"sensitivity": 50}}}},
        {"command": "tracks_get", "args": {"sid": "$sid"}},
        // the export per role: kick, snare, then the toms by range
        {"command": "export_all", "args": {"sid": "$sid", "roles": ["kick"], "formats": ["midi", "spikes"]}},
        {"command": "export_all", "args": {"sid": "$sid", "roles": ["snare"], "formats": ["midi", "spikes"]}},
        {"command": "export_all", "args": {"sid": "$sid", "tracks": ["t1-t3"], "formats": ["midi", "spikes"]}},
        {"command": "project_save", "args": {"sid": "$sid"}},
    ]);
    let script_path = root.join("job.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let (mut i, out, _) = io("");
    let code = run(&svc, &["run".into(), script_path.display().to_string()], &mut i);
    assert_eq!(code, 0, "the job stops at an error: {}", out.text());

    let ls = lines(&out);
    assert_eq!(ls.len(), script.as_array().unwrap().len(), "one answer line per step");

    // step 1: only the pattern's files were added, the `.ptx` and the other mics stayed out
    let added = ls[1]["result"]["added"].as_array().unwrap();
    assert_eq!(
        added.iter().map(Value::as_str).collect::<Vec<_>>(),
        vec![Some("k_in"), Some("s_top"), Some("t1"), Some("t2"), Some("t3")]
    );

    // steps 2 and 4: the roles and notes of the filtered files, and the stored params per track
    for (name, role, note) in [
        ("k_in", "kick", 36),
        ("s_top", "snare", 38),
        ("t1", "tom", 41),
        ("t2", "tom", 45),
        ("t3", "tom", 47),
    ] {
        let row = ls[2]["result"]["tracks"][name].clone();
        assert_eq!(row["role"], role, "{name} role");
        assert_eq!(row["note"], note, "{name} note");
    }
    let params =
        |ls: &[Value], step: usize, name: &str| -> Value { ls[step]["result"]["tracks"][name]["params"].clone() };
    assert_eq!(params(&ls, 4, "k_in")["sensitivity"], 55, "k in keeps its stored param");
    assert_eq!(params(&ls, 4, "s_top")["vel_hi_db"], -8);
    assert_eq!(params(&ls, 4, "t1")["sensitivity"], 50);
    assert_eq!(params(&ls, 4, "t2"), Value::Null, "t2 got no params");
    // the analysis ran on the pattern's tracks and found hits
    let sid: String = ls[0]["result"]["sid"].as_str().unwrap().to_string();
    let ev = match svc.events_get(&sid, "k_in") {
        Ok(v) => v,
        Err(e) => panic!("events_get failed: {e} (step 3: {})", ls[3]),
    };
    assert!(!ev["events"].as_array().unwrap().is_empty(), "no hits on k in");

    // the exports per role: MIDI and spike WAV per track of the group, nothing missing
    let exports = project_dir(&root).join("Demo").join("Exports");
    let (mut midi_files, mut spike_files) = (0u32, 0u32);
    for step in [5usize, 6, 7] {
        let r = &ls[step]["result"];
        assert_eq!(ls[step]["ok"], true, "export step {step}: {}", ls[step]);
        assert_eq!(
            r["errors"].as_array().unwrap().len(),
            0,
            "export step {step}: {}",
            r["errors"]
        );
        for f in r["files"].as_array().unwrap() {
            let p = std::path::PathBuf::from(f.as_str().unwrap());
            assert!(p.exists(), "missing file {p:?}");
        }
        midi_files += r["midi"].as_i64().unwrap_or(0) as u32;
        spike_files += r["spikes"].as_i64().unwrap_or(0) as u32;
    }
    assert_eq!(midi_files, 5, "one MIDI file per exported track");
    assert_eq!(spike_files, 5, "one spike file per exported track");
    for name in ["k_in", "s_top", "t1", "t2", "t3"] {
        assert!(exports.join(format!("{name}_spike.mid")).exists(), "{name}: no MIDI");
        assert!(exports.join(format!("{name}_spike.wav")).exists(), "{name}: no spike");
        assert!(
            !svc.events_get(&sid, name).unwrap()["events"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{name}: no events"
        );
    }
    // the other mics were never added, so nothing of them is exported either
    assert!(!exports.join("hh_spike.mid").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_pattern_or_role_that_matches_nothing_stops_the_job_with_its_error() {
    let root = tmp("agent_job_nomatch");
    let session = session(&root);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project_dir(&root).display().to_string();
    let build = |steps: Value, name: &str| {
        let p = root.join(name);
        std::fs::write(&p, steps.to_string()).unwrap();
        p.display().to_string()
    };
    // a `tracks` pattern that matches no scanned file
    let script = build(
        json!([
            {"command": "project_new", "args": {"name": "NoMatch", "dir": dir}},
            {"command": "tracks_add", "args": {"sid": "$sid", "paths": [session.display().to_string()], "tracks": ["x*"]}},
        ]),
        "nomatch.json",
    );
    let (mut i, out, _) = io("");
    assert_eq!(run(&svc, &["run".into(), script], &mut i), 2, "{}", out.text());
    assert!(out.text().contains("x*"), "the error names the pattern: {}", out.text());

    // an export role that no track has
    let script = build(
        json!([
            {"command": "project_new", "args": {"name": "NoRole", "dir": dir}},
            {"command": "tracks_add", "args": {"sid": "$sid", "paths": [session.display().to_string()], "tracks": ["t1-t3"]}},
            {"command": "export_all", "args": {"sid": "$sid", "roles": ["bongo"]}},
        ]),
        "norole.json",
    );
    let (mut i, out, _) = io("");
    assert_eq!(run(&svc, &["run".into(), script], &mut i), 2, "{}", out.text());
    assert!(out.text().contains("bongo"), "the error names the role: {}", out.text());
    let _ = std::fs::remove_dir_all(&root);
}
