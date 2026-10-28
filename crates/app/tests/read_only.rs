//! R06 (#237): the service itself refuses every mutating command on a read-only session, so the
//! Tauri wrappers, the command-line tool, the MCP server and the test bridge share the policy.
//!
//! The setup of the gate: another process holds the write lock, the project is opened read-only
//! beside it, and every mutating command of the registry (the rows with `writes_project`, never a
//! hand-written list) runs through the service directly, the bridge and the agent entry of the
//! command-line tool. Each one is refused (409, read-only), the document file keeps its SHA-256
//! and the in-memory document does not change. The documented exceptions keep their behaviour:
//! the read commands answer, `track_repick` without `store` answers, and `project_save_as` turns
//! the session into a writable copy.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::*;
use drumspike_app::api::{self, Surface};
use drumspike_app::bridge::{Bridge, Channels, Reply};
use drumspike_app::call;
use drumspike_app::lock::ProjectLock;
use drumspike_app::{AppError, Service, ServiceConfig};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const KICK: &str = "Kick_In";
const SNARE: &str = "Snare_Top";

/// One open read-only session beside a write lock another process holds, with its document file.
struct Session {
    svc: Service,
    sid: String,
    doc: PathBuf,
    wav: PathBuf,
    daw: PathBuf,
    root: PathBuf,
    sha: String,
    state: Value,
    job: Value,
}

impl Session {
    /// The project with two tracks and a few hits, the write lock held (`otherapp`), opened
    /// read-only. The hits come from a throwaway service that closes before the lock is taken.
    fn open(tag: &str) -> Session {
        let root = tmp(tag);
        let dir = project(
            &root,
            "Song",
            &[kick_spec(KICK, "kick", 36), kick_spec(SNARE, "snare", 38)],
        );
        let hits = json!((0..8)
            .map(|i| json!({"t": 0.5 + i as f64 * 0.5, "v": 0.8}))
            .collect::<Vec<_>>());
        // the DSP detector always: the re-pick preview answers then
        let cfg = || ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        };
        let prep = Service::new(cfg());
        let sid = prep.project_open(dir.to_str().unwrap()).unwrap()["sid"]
            .as_str()
            .unwrap()
            .to_string();
        prep.events_set(&sid, KICK, &hits).unwrap();
        prep.events_set(&sid, SNARE, &hits).unwrap();
        // a stored analysis, so the re-pick preview (a read) answers on the read-only session
        prep.analyze_start_keep(&sid, None, None, None, None, true).unwrap();
        // one analyze_status call per iteration: a job that moves queued -> running between
        // two calls in one condition exits with the state still `running` (combined gate
        // run-70 on main 597dd418 asserted `done` and got `running`)
        let mut s = prep.analyze_status(&sid)["state"].clone();
        while s == "running" || s == "queued" {
            std::thread::sleep(std::time::Duration::from_millis(10));
            s = prep.analyze_status(&sid)["state"].clone();
        }
        // R270: the job may end `done` with a track in `error` (a cancelled or crashed worker
        // marks the job `done`). The setup must refuse that here, not three steps later as a
        // misleading `no kept detection` on the re-pick preview.
        let job = prep.analyze_status(&sid);
        assert_eq!(job["state"], "done", "R270 setup: the analysis did not finish: {job}");
        for (name, t) in job["tracks"].as_object().expect("job tracks") {
            assert_eq!(
                t["state"], "done",
                "R270 setup: the analysis of {name} did not finish: {t}"
            );
        }
        // R270 (review): a `done` track can still have lost its kept activation: a failed
        // The kept detection is found right after the analysis, with the same lookup the
        // re-pick uses, so a recurrence fails here with its cause instead of three steps
        // later as a 409.
        for name in [KICK, SNARE] {
            if let Err(e) = prep.track_repick(&sid, name, None, false) {
                panic!(
                    "R270 setup: the kept detection of {name} is not found after the analysis (the re-pick lookup): {} ({}); job: {job}",
                    e.message,
                    e.code,
                );
            }
        }
        prep.project_save(&sid).unwrap();
        prep.project_close(&sid, true).unwrap();
        drop(prep);
        // another process holds the write lock; the read-only open takes no lock and works beside it
        let _held = ProjectLock::acquire(&dir, "otherapp")
            .unwrap()
            .into_held()
            .expect("the other process holds the lock");
        let svc = Service::new(cfg());
        let open = svc.project_open_with(dir.to_str().unwrap(), true).unwrap();
        let sid = open["sid"].as_str().unwrap().to_string();
        let doc = root.join("Song").join("Song.drumspike");
        let state = Self::state(&svc, &sid);
        let sha = {
            let bytes = std::fs::read(&doc).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex(hasher.finalize())
        };
        Session {
            sha,
            svc,
            sid,
            doc,
            wav: root.join("daw").join(format!("{KICK}.wav")),
            daw: root.join("daw"),
            root,
            state,
            job,
        }
    }

    /// The in-memory document as the read commands show it: the summary, the tracks, the hits of
    /// both tracks, the tempo map, the markers and the lane heights.
    fn state(svc: &Service, sid: &str) -> Value {
        json!({
            "project": svc.project_get(sid).unwrap(),
            "tracks": svc.tracks_get(sid).unwrap(),
            "kick": svc.events_get(sid, KICK).unwrap(),
            "snare": svc.events_get(sid, SNARE).unwrap(),
            "tempo": svc.tempo_map_get(sid).unwrap(),
            "markers": svc.markers_get(sid).unwrap(),
            "heights": svc.row_heights_get(sid).unwrap(),
        })
    }

    fn unchanged(&self) {
        let current_sha = {
            let bytes = std::fs::read(&self.doc).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex(hasher.finalize())
        };
        assert_eq!(current_sha, self.sha, "the document file changed");
        assert_eq!(
            Self::state(&self.svc, &self.sid),
            self.state,
            "the in-memory document changed"
        );
    }
}

/// The JSON arguments of one mutating row: every required argument of the row, shape-valid and
/// pointed at the fixture project. A row the table does not know fails the loop with 400, so a
/// new command adds its row here instead of slipping past the gate.
fn row_args(row: &api::CommandSpec, s: &Session) -> Value {
    let mut m = serde_json::Map::new();
    for a in row.args {
        let v = match (a.name, a.ty) {
            ("sid", _) => json!(s.sid),
            ("name", drumspike_app::api::Ty::Str) if row.name == "project_rename" => json!("Song Two"),
            ("name", _) => json!(KICK),
            ("new_name", _) => json!("Kick_Up"),
            ("role", _) => json!("kick"),
            ("kind", _) => json!("this"),
            ("id", _) => json!("m1"),
            ("track", _) => json!(KICK),
            ("folder", _) => json!(s.daw.display().to_string()),
            ("path", _) if row.name == "project_relink" => json!(s.wav.display().to_string()),
            ("path", _) if row.name == "tempo_map_import_midi" => json!("x.mid"),
            ("paths", _) => json!([s.wav.display().to_string()]),
            ("names", _) => json!([SNARE]),
            ("order", _) => json!([KICK, SNARE]),
            ("formats", _) => json!(["midi", "spikes"]),
            ("note", _) => json!(40),
            ("shift_samples", _) => json!(100),
            ("lo_db", _) => json!(-30.0),
            ("hi_db", _) => json!(0.0),
            ("t0", _) => json!(0.2),
            ("t1", _) => json!(0.3),
            ("on", _) => json!(true),
            ("events", drumspike_app::api::Ty::Objs) => json!([{"t": 0.25, "v": 0.8}]),
            ("ops", _) => json!([{"op": "add", "t": 0.3, "v": 0.7}]),
            ("markers", _) => json!([{"t": 1.0}]),
            ("params", _) => json!({"sensitivity": 70}),
            ("heights", _) => json!({KICK: 1.5}),
            ("map", _) => json!({}),
            ("learn", _) => json!({"on": false, "marks": []}),
            ("tracks", _) => json!([]),
            _ => {
                assert!(!a.required, "{}: no value for the required `{}`", row.name, a.name);
                continue;
            }
        };
        m.insert(a.name.into(), v);
    }
    // the documented conditional: a re-pick that stores is a change, the preview is not
    if row.name == "track_repick" {
        m.insert("store".into(), json!(true));
    }
    Value::Object(m)
}

/// The answer must be the read-only refusal (409).
fn refused(route: &str, name: &str, r: Result<Reply, AppError>) {
    let e = r.expect_err(&format!("{route} {name} changed the read-only project"));
    assert_eq!(e.code, 409, "{route} {name}: {}", e.message);
    assert!(e.message.contains("read-only"), "{route} {name}: {}", e.message);
}

/// The gate: another process holds the write lock, the session is open read-only, and every
/// mutating command of the registry is refused on all three routes; the document file keeps its
/// SHA-256 and the in-memory document does not change.
#[test]
fn the_service_refuses_every_mutating_command_on_a_read_only_session() {
    let s = Session::open("read_only_r06");
    let bridge = Bridge::new(s.svc.clone(), repo_root().join("static"));
    let ch = Arc::new(Channels::default());
    let mut refused_count = 0;
    for row in api::REGISTRY.iter() {
        // the list is the registry's, not a hand-written one; the rows without a session
        // (project_new, project_import) and the documented exception (project_save_as) are out
        if !row.writes_project() || row.name == "project_save_as" || !row.args.iter().any(|a| a.name == "sid") {
            continue;
        }
        let args = row_args(row, &s);
        // the service directly (the boundary the Tauri wrappers share)
        let r = call::run(&s.svc, &ch, row.name, &args)
            .unwrap_or_else(|| panic!("the dispatcher does not run {}", row.name));
        refused("service", row.name, r);
        // the test bridge (what `service_tool run` and the page drive)
        refused("bridge", row.name, bridge.invoke(row.name, &args));
        // the command-line tool and the MCP server (the agent surface)
        if row.surface == Surface::Agent {
            refused("cli", row.name, call::call(&s.svc, row.name, &args, None));
        }
        refused_count += 1;
    }
    assert!(refused_count >= 37, "the mutating list is too short: {refused_count}"); // #281 D16: the model-era commands are gone from main
    s.unchanged();
}

/// The documented behaviour on a read-only session: the read commands answer, the re-pick
/// preview answers (no `store`), and Save As turns the session into a writable copy without
/// touching the original document file.
#[test]
fn a_read_only_session_answers_reads_and_saves_as_to_a_writable_copy() {
    let s = Session::open("read_only_r06_reads");
    // reads answer
    assert!(s.svc.project_get(&s.sid).is_ok());
    assert!(s.svc.tracks_get(&s.sid).is_ok());
    assert!(s.svc.events_get(&s.sid, KICK).is_ok());
    assert!(s.svc.tempo_map_get(&s.sid).is_ok());
    assert!(s.svc.tempo_map_detect(&s.sid, None, None, None).is_ok());
    assert!(s.svc.velocities_recompute(&s.sid, KICK, None).is_ok());
    // the re-pick preview is a read: no `store`, no refusal. It answers on the DSP detector;
    // it never gives the read-only refusal.
    match s.svc.track_repick(&s.sid, KICK, None, false) {
        Ok(_) => {}
        Err(e) => assert!(
            !(e.code == 409 && e.message.contains("read-only")),
            "the re-pick preview is refused: {} ({}); R270 setup-end job: {}",
            e.message,
            e.code,
            s.job,
        ),
    }
    // the agent route refuses the stored re-pick and answers the preview (the documented split)
    let e = call::call(
        &s.svc,
        "track_repick",
        &json!({"sid": s.sid, "name": KICK, "store": true}),
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, 409);
    // Save As: the way from a read-only project to a writable copy
    let out = s
        .svc
        .project_save_as(
            &s.sid,
            Some("Copy"),
            Some(&s.root.join("out").display().to_string()),
            false,
            None,
        )
        .unwrap();
    let new_sid = out["sid"].as_str().unwrap().to_string();
    // the session is the copy now: a change goes through, the original file stays
    assert!(s.svc.track_set_note(&new_sid, KICK, 40).is_ok());
    let original_sha = {
        let bytes = std::fs::read(&s.doc).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        hex(hasher.finalize())
    };
    assert_eq!(original_sha, s.sha, "Save As changed the original file");
}

/// The reason the session is read-only: the write lock is held by another process, so the folder
/// refuses a writable open (409) while the read-only open works beside it.
#[test]
fn the_write_lock_stays_with_the_other_process() {
    let root = tmp("read_only_r06_lock");
    let dir = project(&root, "Song", &[kick_spec(KICK, "kick", 36)]);
    let _held = ProjectLock::acquire(&dir, "otherapp")
        .unwrap()
        .into_held()
        .expect("held");
    let svc = service_dsp();
    svc.set_lock_client("test");
    let e = svc.project_open_with(dir.to_str().unwrap(), false).unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.contains("otherapp"), "{}", e.message);
    let open = svc.project_open_with(dir.to_str().unwrap(), true).unwrap();
    assert!(open["sid"].as_str().is_some());
}
