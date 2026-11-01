//! One or more tests per command of the slice (P3-3a), through `drumspike_app::Service`
//! without Tauri. The test names start with the command name; `command_tests.rs` checks that.

mod common;

use std::sync::{Arc, Mutex};

use common::file_sha256;
use common::*;
use drumspike_app::{ListShape, Progress, Service, ServiceConfig};
use drumspike_core::io::MixMode;
use drumspike_core::midi::{MidiEvent, midi_bytes};
use drumspike_core::{Track, read_wav_mono};
use serde_json::{Value, json};

/// Open a two-track project; returns (service, sid, folder, root).
fn opened(tag: &str) -> (Service, String, std::path::PathBuf, std::path::PathBuf) {
    let root = tmp(tag);
    let dir = project(
        &root,
        "Song",
        &[kick_spec("Kick_In", "kick", 36), kick_spec("Snare_Top", "snare", 38)],
    );
    let svc = service_dsp();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    (svc, sid, dir, root)
}

fn f32s(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}

// ---------- app_info ----------

#[test]
fn app_info_reports_version_detector_chain() {
    let svc = Service::new(ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    let v = svc.app_info();
    assert_eq!(v["core_version"], drumspike_core::VERSION);
    assert_eq!(v["detector"], drumspike_app::DETECTOR);
    assert_eq!(v["chain"], drumspike_app::CHAIN);
    assert_eq!(v["model"], Value::Null);
}

// ---------- project_open ----------

#[test]
fn project_open_returns_the_summary_and_keeps_the_id_of_an_open_folder() {
    let (svc, sid, dir, root) = opened("open");
    let again = svc.project_open(dir.to_str().unwrap()).unwrap();
    assert_eq!(again["sid"], sid.as_str());
    assert_eq!(again["name"], "Song");
    assert_eq!(again["folder"], dir.display().to_string());
    assert_eq!(again["order"], json!(["Kick_In", "Snare_Top"]));
    assert_eq!(again["tracks"]["Kick_In"]["status"], "ok");
    assert_eq!(again["tracks"]["Kick_In"]["source"]["mode"], "referenced");
    // the document file opens too
    let by_doc = svc.project_open(dir.join("Song.drumspike").to_str().unwrap()).unwrap();
    assert_eq!(by_doc["sid"], sid.as_str());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn project_open_errors_carry_the_server_codes() {
    let svc = service_dsp();
    assert_eq!(svc.project_open("").unwrap_err().code, 400);
    let root = tmp("open_err");
    let e = svc.project_open(root.join("nothing").to_str().unwrap()).unwrap_err();
    assert_eq!(e.code, 404, "{e}");
    std::fs::write(root.join("x.drumspike"), "[1]").unwrap();
    assert_eq!(svc.project_open(root.to_str().unwrap()).unwrap_err().code, 400);
    // a document of a newer format is 409
    std::fs::write(root.join("x.drumspike"), r#"{"format": 99, "tracks": {}}"#).unwrap();
    assert_eq!(svc.project_open(root.to_str().unwrap()).unwrap_err().code, 409);
    std::fs::remove_dir_all(&root).unwrap();
}

/// P3-15e: the Finder sends a document, its folder and repeated double-clicks; since P3-15c the opens
/// run in parallel on the blocking pool. Every open of the same project ends in one session.
#[test]
fn project_open_in_parallel_of_one_project_gives_one_session() {
    let root = tmp("open_parallel");
    let dir = project(&root, "Song", &[kick_spec("Kick_In", "kick", 36)]);
    let doc = dir.join("Song.drumspike");
    let ways = [
        dir.display().to_string(),
        doc.display().to_string(),
        format!("{}/", dir.display()),
        format!("{}/./Song.drumspike", dir.display()),
    ];
    for round in 0..20 {
        let svc = service_dsp();
        let gate = Arc::new(std::sync::Barrier::new(8));
        let sids: Vec<String> = (0..8)
            .map(|i| {
                let (svc, gate, path) = (svc.clone(), Arc::clone(&gate), ways[i % ways.len()].clone());
                std::thread::spawn(move || {
                    gate.wait();
                    svc.project_open(&path).unwrap()["sid"].as_str().unwrap().to_string()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        let list = svc.project_list(ListShape::Ids);
        assert_eq!(list.as_array().unwrap().len(), 1, "round {round}: {list}");
        assert!(sids.iter().all(|s| *s == sids[0]), "round {round}: {sids:?}");
        assert_eq!(svc.project_get(&sids[0]).unwrap()["folder"], dir.display().to_string());
    }
    std::fs::remove_dir_all(&root).unwrap();
}

// ---------- project_get ----------

#[test]
fn project_get_resolves_the_sources_again() {
    let (svc, sid, _dir, root) = opened("get");
    assert_eq!(svc.project_get(&sid).unwrap()["tracks"]["Kick_In"]["status"], "ok");
    std::fs::rename(root.join("daw/Kick_In.wav"), root.join("daw/moved.wav")).unwrap();
    let v = svc.project_get(&sid).unwrap();
    assert_eq!(v["tracks"]["Kick_In"]["status"], "missing");
    assert_eq!(v["tracks"]["Kick_In"]["offline"], true);
    assert_eq!(v["tracks"]["Kick_In"]["path"], Value::Null);
    assert_eq!(v["tracks"]["Snare_Top"]["status"], "ok");
    assert_eq!(svc.project_get("nope").unwrap_err().code, 404);
    std::fs::remove_dir_all(&root).unwrap();
}

// ---------- project_list ----------

#[test]
fn project_list_gives_both_row_shapes_newest_first() {
    let root = tmp("list");
    let a = project(&root.join("a"), "Old", &[kick_spec("K", "kick", 36)]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let b = project(&root.join("b"), "New", &[kick_spec("K", "kick", 36)]);
    let svc = service_dsp();
    assert_eq!(svc.project_list(ListShape::Sessions), json!([]));
    let sid_a = svc.project_open(a.to_str().unwrap()).unwrap()["sid"].clone();
    let sid_b = svc.project_open(b.to_str().unwrap()).unwrap()["sid"].clone();
    let rows = svc.project_list(ListShape::Sessions);
    let names: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["New", "Old"], "newest first");
    assert_eq!(rows[0]["sid"], sid_b);
    assert_eq!(rows[0]["tracks"], json!(["K"]));
    assert_eq!(rows[0]["dirty"], false);
    assert!(rows[0]["mtime"].as_f64().unwrap() >= rows[1]["mtime"].as_f64().unwrap());
    let ids = svc.project_list(ListShape::Ids);
    assert_eq!(ids[0]["id"], sid_a, "opening order");
    assert_eq!(ids[1]["id"], sid_b);
    assert!(ids[0].get("sid").is_none() && ids[0].get("mtime").is_none());
    std::fs::remove_dir_all(&root).unwrap();
}

// ---------- tracks_get ----------

#[test]
fn tracks_get_lists_tracks_without_events_and_marks_offline_ones() {
    let (svc, sid, _dir, root) = opened("tracks");
    std::fs::remove_file(root.join("daw/Snare_Top.wav")).unwrap();
    let v = svc.tracks_get(&sid).unwrap();
    assert_eq!(v["sid"], sid.as_str());
    assert_eq!(v["order"], json!(["Kick_In", "Snare_Top"]));
    let k = &v["tracks"]["Kick_In"];
    assert_eq!(k["role"], "kick");
    assert_eq!(k["status"], "ok");
    assert_eq!(k["offline"], false);
    assert_eq!(k["source_mode"], "referenced");
    assert_eq!(k["n_events"], 0);
    assert!(k.get("events").is_none() && k.get("source").is_none());
    assert_eq!(v["tracks"]["Snare_Top"]["status"], "missing");
    assert_eq!(v["tracks"]["Snare_Top"]["offline"], true);
    assert_eq!(svc.tracks_get("nope").unwrap_err().code, 404);
    std::fs::remove_dir_all(&root).unwrap();
}

// ---------- events_get / events_set ----------

#[test]
fn events_get_has_the_defaults_of_a_track_without_analysis() {
    let (svc, sid, _dir, root) = opened("events_get");
    let v = svc.events_get(&sid, "Kick_In").unwrap();
    assert_eq!(v["events"], json!([]));
    assert_eq!(v["sr"], 48000);
    assert_eq!(v["duration"].as_f64().unwrap(), 3.5);
    assert_eq!(v["n_samples"], 168_000);
    assert_eq!(
        svc.events_get(&sid, "nope").unwrap_err(),
        drumspike_app::AppError::bad("unknown track")
    );
    assert_eq!(svc.events_get("nope", "Kick_In").unwrap_err().code, 404);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn events_set_keeps_every_key_of_every_event() {
    let (svc, sid, _dir, root) = opened("events_set");
    let events = json!([
        {"t": 0.5, "v": 100},
        {"t": 0.1 + 0.2, "v": 20, "added": true, "id": "l4"},
        {"t": 2.675, "v": 127, "orig": 1e-7, "edited": true, "rise": 12.3, "rise_db": 7.25},
        {"t": 1.0 / 3.0, "v": 90.0, "planted_unknown": {"deep": [1, "two", null]}, "id": 7},
        {"t": 4.5, "v": null, "added": "yes", "zz_last": false}
    ]);
    assert_eq!(svc.events_set(&sid, "Kick_In", &events).unwrap(), json!({"saved": 5}));
    let got = svc.events_get(&sid, "Kick_In").unwrap()["events"].clone();
    assert_eq!(got, events, "values equal");
    for (g, w) in got.as_array().unwrap().iter().zip(events.as_array().unwrap()) {
        let mut gk: Vec<&String> = g.as_object().unwrap().keys().collect();
        let mut wk: Vec<&String> = w.as_object().unwrap().keys().collect();
        gk.sort();
        wk.sort();
        assert_eq!(gk, wk, "key set of {w}");
    }
    assert_eq!(got[3]["planted_unknown"]["deep"], json!([1, "two", null]));
    assert_eq!(got[3]["v"], json!(90.0), "a float velocity stays a float");
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], true);
    assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"]["Kick_In"]["n_events"], 5);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn events_set_keeps_a_list_the_typed_events_cannot_hold() {
    let (svc, sid, _dir, root) = opened("events_untyped");
    let events = json!([{"t": 0.5, "v": 100}, {"v": 5, "note": "no time"}, {"t": "0.7"}]);
    assert_eq!(svc.events_set(&sid, "Kick_In", &events).unwrap(), json!({"saved": 3}));
    assert_eq!(svc.events_get(&sid, "Kick_In").unwrap()["events"], events);
    assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"]["Kick_In"]["n_events"], 3);
    // a typed list replaces it again
    svc.events_set(&sid, "Kick_In", &json!([{"t": 1.5, "v": 64}])).unwrap();
    assert_eq!(
        svc.events_get(&sid, "Kick_In").unwrap()["events"],
        json!([{"t": 1.5, "v": 64}])
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn events_set_rejects_a_non_list_and_an_unknown_track() {
    let (svc, sid, _dir, root) = opened("events_set_err");
    let e = svc.events_set(&sid, "Kick_In", &json!({"t": 1})).unwrap_err();
    assert_eq!((e.code, e.message.as_str()), (400, "events must be a list"));
    assert_eq!(svc.events_set(&sid, "nope", &json!([])).unwrap_err().code, 400);
    assert_eq!(svc.events_set("nope", "Kick_In", &json!([])).unwrap_err().code, 404);
    assert_eq!(
        svc.project_get(&sid).unwrap()["dirty"],
        false,
        "a failed call does not mark dirty"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

// ---------- peaks_info / peaks_tile ----------

#[test]
fn peaks_info_lists_the_levels_and_builds_the_cache() {
    let (svc, sid, dir, root) = opened("peaks_info");
    assert!(!dir.join("Analysis/Kick_In/peaks.bin").exists());
    let v = svc.peaks_info(&sid, "Kick_In").unwrap();
    assert_eq!(v["sr"], 48000);
    assert_eq!(v["n_samples"], 168_000);
    assert_eq!(v["levels"]["256"]["n"], 168_000 / 256);
    assert_eq!(v["levels"]["4096"]["n"], 168_000 / 4096);
    assert_eq!(v["levels"]["65536"]["n"], 2);
    assert!(dir.join("Analysis/Kick_In/peaks.bin").is_file());
    assert!(dir.join("Analysis/Kick_In/key.json").is_file());
    assert_eq!(svc.peaks_info(&sid, "nope").unwrap_err().code, 404);
    assert_eq!(svc.peaks_info("nope", "Kick_In").unwrap_err().code, 404);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn peaks_tile_equals_track_peaks_for_the_same_range() {
    let (svc, sid, _dir, root) = opened("peaks_tile");
    let (x, info) = read_wav_mono(&root.join("daw/Kick_In.wav"), MixMode::Average).unwrap();
    let track = Track::new(info.sample_rate, x);
    for level in [256usize, 4096, 65536] {
        for (from, to) in [
            (0i64, 100i64),
            (10, 40),
            (-5, 20),
            (3, 100_000),
            (50, 20),
            (100_000, 200_000),
        ] {
            let b = svc.peaks_tile(&sid, "Kick_In", level, from, to).unwrap();
            let want = track.peaks(level, from, to).unwrap();
            let n = want.min.len();
            assert_eq!(b.len(), 12 + 12 * n, "level {level} {from}..{to}");
            assert_eq!(u32_at(&b, 0) as usize, level);
            assert_eq!(u32_at(&b, 4) as usize, want.start);
            assert_eq!(u32_at(&b, 8) as usize, n);
            let v = f32s(&b[12..]);
            for (col, want_col) in [want.min, want.max, want.rms].iter().enumerate() {
                for i in 0..n {
                    assert_eq!(
                        f64::from(v[col * n + i]).to_bits(),
                        want_col[i].to_bits(),
                        "level {level} col {col} bin {i}"
                    );
                }
            }
        }
    }
    assert_eq!(svc.peaks_tile(&sid, "Kick_In", 512, 0, 10).unwrap_err().code, 400);
    assert_eq!(svc.peaks_tile(&sid, "nope", 256, 0, 10).unwrap_err().code, 404);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn peaks_tile_serves_a_current_cache_of_an_offline_track_and_409s_without_one() {
    let (svc, sid, _dir, root) = opened("peaks_offline");
    svc.peaks_info(&sid, "Kick_In").unwrap();
    std::fs::remove_file(root.join("daw/Kick_In.wav")).unwrap();
    std::fs::remove_file(root.join("daw/Snare_Top.wav")).unwrap();
    svc.project_get(&sid).unwrap();
    assert_eq!(
        svc.peaks_tile(&sid, "Kick_In", 256, 0, 4).unwrap().len(),
        12 + 12 * 4,
        "cache is current"
    );
    let e = svc.peaks_tile(&sid, "Snare_Top", 256, 0, 4).unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.contains("offline (missing)"), "{e}");
    std::fs::remove_dir_all(&root).unwrap();
}

// ---------- samples_window ----------

#[test]
fn samples_window_returns_the_f32_samples_with_start_and_rate() {
    let (svc, sid, _dir, root) = opened("samples");
    let (x, _) = read_wav_mono(&root.join("daw/Kick_In.wav"), MixMode::Average).unwrap();
    let b = svc.samples_window(&sid, "Kick_In", 12_000, 12_100).unwrap();
    assert_eq!((u32_at(&b, 0), u32_at(&b, 4)), (12_000, 48_000));
    let v = f32s(&b[8..]);
    assert_eq!(v.len(), 100);
    assert_eq!(v, &x[12_000..12_100], "not rounded");
    // clamps: negative start, past the end, reversed, empty
    let b = svc.samples_window(&sid, "Kick_In", -50, 10).unwrap();
    assert_eq!((u32_at(&b, 0), f32s(&b[8..]).len()), (0, 10));
    let b = svc.samples_window(&sid, "Kick_In", 167_990, 999_999).unwrap();
    assert_eq!((u32_at(&b, 0), f32s(&b[8..]).len()), (167_990, 10));
    assert_eq!(svc.samples_window(&sid, "Kick_In", 500, 100).unwrap().len(), 8);
    assert_eq!(svc.samples_window(&sid, "Kick_In", 900_000, 999_999).unwrap().len(), 8);
    assert_eq!(svc.samples_window(&sid, "nope", 0, 10).unwrap_err().code, 400);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn samples_window_caps_at_262144_samples_and_409s_for_an_offline_track() {
    let root = tmp("samples_cap");
    let mut long = kick_spec("Long", "kick", 36);
    long.x = kick_signal(&ONSETS, 7.0, 48_000); // 336,000 samples
    let dir = project(&root, "Cap", &[long]);
    let svc = service_dsp();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let b = svc.samples_window(&sid, "Long", 0, 1_000_000).unwrap();
    assert_eq!(f32s(&b[8..]).len(), 262_144);
    let b = svc.samples_window(&sid, "Long", 100_000, 1_000_000).unwrap();
    assert_eq!(
        (u32_at(&b, 0), f32s(&b[8..]).len()),
        (100_000, 236_000),
        "the cap, then the end of the track"
    );
    let b = svc.samples_window(&sid, "Long", 100_000, 100_000 + 262_144).unwrap();
    assert_eq!(f32s(&b[8..]).len(), 236_000);
    std::fs::remove_file(root.join("daw/Long.wav")).unwrap();
    svc.project_get(&sid).unwrap();
    assert_eq!(svc.samples_window(&sid, "Long", 0, 10).unwrap_err().code, 409);
    std::fs::remove_dir_all(&root).unwrap();
}

// ---------- analyze_start / analyze_status ----------

fn collect_progress() -> (Progress, Arc<Mutex<Vec<Value>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = Arc::clone(&seen);
    (Arc::new(move |v| s2.lock().unwrap().push(v)), seen)
}

#[test]
fn analyze_start_runs_the_job_and_stores_events_sr_and_detector() {
    let (svc, sid, dir, root) = opened("analyze");
    assert_eq!(svc.analyze_status(&sid), json!({"state": "idle", "tracks": {}}));
    let (progress, seen) = collect_progress();
    let params = json!({"Kick_In": {"min_gap_ms": 30, "refine": true}});
    let r = svc
        .analyze_start(&sid, Some(vec!["Kick_In".into()]), params.as_object(), Some(progress))
        .unwrap();
    assert_eq!(r, json!({"started": ["Kick_In"]}));
    let st = wait_done(&svc, &sid);
    let e = &st["tracks"]["Kick_In"];
    assert_eq!(e["state"], "done");
    assert_eq!(e["detector"], "rust-dsp");
    assert_eq!(e["chain"], Value::Null);
    assert!(e["n_events"].as_u64().unwrap() >= 5, "{e}");
    assert!(st["tracks"].get("Snare_Top").is_none(), "only the asked track");
    let ev = svc.events_get(&sid, "Kick_In").unwrap();
    assert_eq!(
        ev["events"].as_array().unwrap().len() as u64,
        e["n_events"].as_u64().unwrap()
    );
    assert!(ev["events"][0]["v"].as_i64().unwrap() >= 20);
    assert!(ev["events"][0]["t"].as_f64().unwrap() > 0.2);
    let t = &svc.tracks_get(&sid).unwrap()["tracks"]["Kick_In"];
    assert_eq!(t["params"], json!({"min_gap_ms": 30, "refine": true}));
    // P4-12: the record carries the version of the refinement rule the analysis ran (5, the capped foot, #127)
    assert_eq!(
        t["detector"],
        json!({"detector": "rust-dsp", "chain": null, "model": null, "refine": 5})
    );
    assert_eq!(t["n_samples"], 168_000);
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], true);
    // the progress callback saw queued-to-done states of the job and a stage
    final_state(&seen, "job");
    let seen = seen.lock().unwrap();
    assert!(seen.iter().all(|m| m["sid"] == sid.as_str()));
    assert_eq!(seen.last().unwrap()["job"]["state"], "done");
    assert!(
        seen.iter()
            .any(|m| m["job"]["tracks"]["Kick_In"]["stage"] == "decoding")
    );
    assert!(
        seen.iter()
            .any(|m| m["job"]["tracks"]["Kick_In"]["stage"] == "analyzing")
    );
    // the second run keeps the first events in Backups/<track>.events.prev.json
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.6, "v": 77, "added": true}]))
        .unwrap();
    svc.analyze_start(&sid, Some(vec!["Kick_In".into()]), None, None)
        .unwrap();
    wait_done(&svc, &sid);
    let prev: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("Backups/Kick_In.events.prev.json")).unwrap()).unwrap();
    assert_eq!(prev["events"], json!([{"t": 0.6, "v": 77, "added": true}]));
    assert_eq!(prev["sr"], 48000);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn analyze_start_without_a_track_list_runs_every_track() {
    let (svc, sid, _dir, root) = opened("analyze_all");
    assert_eq!(
        svc.analyze_start(&sid, None, None, None).unwrap(),
        json!({"started": ["Kick_In", "Snare_Top"]})
    );
    let st = wait_done(&svc, &sid);
    assert_eq!(st["tracks"]["Kick_In"]["state"], "done");
    assert_eq!(st["tracks"]["Snare_Top"]["state"], "done");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn analyze_start_rejects_an_unknown_track_and_an_unknown_project() {
    let (svc, sid, _dir, root) = opened("analyze_err");
    let e = svc
        .analyze_start(&sid, Some(vec!["nope".into()]), None, None)
        .unwrap_err();
    assert_eq!(
        (e.code, e.message.as_str()),
        (
            400,
            "the pattern \"nope\" matches no track (the tracks: Kick_In, Snare_Top)"
        )
    );
    assert_eq!(svc.analyze_status(&sid)["state"], "idle", "nothing started");
    assert_eq!(svc.analyze_start("nope", None, None, None).unwrap_err().code, 404);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn analyze_start_marks_an_offline_track_as_an_error_and_goes_on() {
    let (svc, sid, _dir, root) = opened("analyze_offline");
    std::fs::remove_file(root.join("daw/Kick_In.wav")).unwrap();
    svc.project_get(&sid).unwrap();
    svc.analyze_start(&sid, None, None, None).unwrap();
    let st = wait_done(&svc, &sid);
    assert_eq!(st["tracks"]["Kick_In"]["state"], "error");
    assert_eq!(
        st["tracks"]["Kick_In"]["error"],
        "track is offline (missing): relink it first"
    );
    assert_eq!(st["tracks"]["Snare_Top"]["state"], "done");
    std::fs::remove_dir_all(&root).unwrap();
}

/// P3-15b: `job_cancel` reaches a running analysis. The cancel is asked from the progress
/// callback at the moment the first track enters its model step (`analyzing`), so it is set
/// before that track ends: the job ends after it, the two other tracks get the error
/// `analysis cancelled`, and their events are untouched.
#[test]
fn job_cancel_ends_a_running_analysis_after_the_track_in_its_model_step() {
    let root = tmp("analyze_cancel");
    let dir = project(
        &root,
        "Cancel",
        &[
            kick_spec("A", "kick", 36),
            kick_spec("B", "kick", 36),
            kick_spec("C", "kick", 36),
        ],
    );
    let svc = service_dsp();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        svc.job_cancel(&sid),
        json!({"sid": sid, "cancelling": false}),
        "idle: nothing to cancel"
    );
    let answers = Arc::new(Mutex::new(Vec::new()));
    let (svc2, sid2, answers2) = (svc.clone(), sid.clone(), Arc::clone(&answers));
    let progress: Progress = Arc::new(move |v: Value| {
        if v["job"]["tracks"]["A"]["stage"] == "analyzing" && answers2.lock().unwrap().is_empty() {
            answers2.lock().unwrap().push(svc2.job_cancel(&sid2));
        }
    });
    svc.analyze_start(&sid, None, None, Some(progress)).unwrap();
    let st = wait_done(&svc, &sid);
    assert_eq!(
        *answers.lock().unwrap(),
        vec![json!({"sid": sid, "cancelling": true, "kind": "analyze"})]
    );
    assert_eq!(st["state"], "done");
    assert_eq!(st["tracks"]["A"]["state"], "done", "{st}");
    assert!(st["tracks"]["A"]["n_events"].as_u64().unwrap() >= 5);
    for name in ["B", "C"] {
        assert_eq!(
            st["tracks"][name],
            json!({"state": "error", "error": "analysis cancelled"}),
            "{name}: {st}"
        );
        assert_eq!(svc.events_get(&sid, name).unwrap()["events"], json!([]));
    }
    assert_eq!(svc.job_cancel(&sid)["cancelling"], false, "a done job is not cancelled");
    // the next analysis of the same project runs every track
    svc.analyze_start(&sid, None, None, None).unwrap();
    let st = wait_done(&svc, &sid);
    for name in ["A", "B", "C"] {
        assert_eq!(st["tracks"][name]["state"], "done", "{name}: {st}");
    }
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn analyze_status_reports_a_bad_rate_and_bad_params_as_a_track_error() {
    let root = tmp("analyze_bad");
    let mut odd = kick_spec("Odd", "kick", 36);
    odd.sr = 22_050;
    odd.x = kick_signal(&ONSETS, 3.5, 22_050);
    let dir = project(&root, "Bad", &[odd, kick_spec("Kick_In", "kick", 36)]);
    let svc = service_dsp();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let params = json!({"Kick_In": {"min_gap_ms": "fast"}});
    svc.analyze_start(&sid, None, params.as_object(), None).unwrap();
    let st = wait_done(&svc, &sid);
    assert_eq!(st["tracks"]["Odd"]["state"], "error");
    assert!(
        st["tracks"]["Odd"]["error"]
            .as_str()
            .unwrap()
            .contains("unsupported sample rate 22050")
    );
    assert_eq!(st["tracks"]["Kick_In"]["state"], "error");
    assert!(
        st["tracks"]["Kick_In"]["error"]
            .as_str()
            .unwrap()
            .contains("bad params: min_gap_ms")
    );
    std::fs::remove_dir_all(&root).unwrap();
}

// ---------- export_midi ----------

#[test]
fn export_midi_writes_the_bytes_of_midi_bytes() {
    let (svc, sid, dir, root) = opened("export");
    let events = vec![
        json!({"t": 0.25, "v": 100}),
        json!({"t": 0.75, "v": 20}),
        json!({"t": 1.25}),
    ];
    let r = svc.export_midi(&sid, "Kick_In", 36, &events, 24000, 120.0).unwrap();
    let path = dir.join("Exports/Kick_In_spike.mid");
    assert_eq!(r["path"], path.display().to_string());
    assert_eq!(r["n_events"], 3);
    let want = midi_bytes(
        &[
            MidiEvent { t: 0.25, v: 100 },
            MidiEvent { t: 0.75, v: 20 },
            MidiEvent { t: 1.25, v: 100 },
        ],
        36,
        24000,
        120.0,
    )
    .unwrap();
    let got = std::fs::read(&path).unwrap();
    assert_eq!(got, want, "a missing v is 100");
    assert_eq!(r["size"], got.len());
    // another note, ppq and tempo change the bytes
    svc.export_midi(&sid, "Kick_In", 38, &events, 480, 90.0).unwrap();
    assert_ne!(std::fs::read(&path).unwrap(), want);
    // the empty list is a valid file
    svc.export_midi(&sid, "Kick_In", 36, &[], 24000, 120.0).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        midi_bytes(&[], 36, 24000, 120.0).unwrap()
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn export_midi_rejects_bad_arguments() {
    let (svc, sid, _dir, root) = opened("export_err");
    let ok = vec![json!({"t": 0.25, "v": 100})];
    assert_eq!(
        svc.export_midi(&sid, "nope", 36, &ok, 24000, 120.0).unwrap_err().code,
        400
    );
    assert_eq!(
        svc.export_midi("nope", "Kick_In", 36, &ok, 24000, 120.0)
            .unwrap_err()
            .code,
        404
    );
    assert_eq!(
        svc.export_midi(&sid, "Kick_In", 200, &ok, 24000, 120.0)
            .unwrap_err()
            .code,
        400,
        "note"
    );
    assert_eq!(
        svc.export_midi(&sid, "Kick_In", 36, &ok, 70_000, 120.0)
            .unwrap_err()
            .code,
        400,
        "ppq"
    );
    assert_eq!(
        svc.export_midi(&sid, "Kick_In", 36, &ok, 24000, -1.0).unwrap_err().code,
        400,
        "tempo"
    );
    assert_eq!(
        svc.export_midi(&sid, "Kick_In", 36, &[json!({"v": 1})], 24000, 120.0)
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        svc.export_midi(&sid, "Kick_In", 36, &[json!({"t": 1, "v": "x"})], 24000, 120.0)
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        svc.export_midi(&sid, "Kick_In", 36, &[json!({"t": 1, "v": 300})], 24000, 120.0)
            .unwrap_err()
            .code,
        400
    );
    std::fs::remove_dir_all(&root).unwrap();
}

// ---------- P3-3c: export_spikes, export_all, velocities_recompute ----------

/// A project of one 24-bit stereo track with a `bext` chunk (time reference above 2^32) and one
/// 16-bit mono track; returns (service, sid, folder, root).
fn spike_project(tag: &str) -> (Service, String, std::path::PathBuf, std::path::PathBuf) {
    use drumspike_core::io::Bext;
    let root = tmp(tag);
    let mut p = drumspike_core::project::new_project(&root, "Song").unwrap();
    let bext = Bext {
        description: "take 3".into(),
        originator: "Pro Tools".into(),
        origination_date: "2026-09-14".into(),
        origination_time: "10:31:02".into(),
        time_reference: 5_000_000_123,
        version: 2,
        ..Bext::default()
    };
    let x = kick_signal(&ONSETS, 3.5, 48_000);
    let stereo: Vec<f32> = x.iter().flat_map(|&v| [v, v * 0.5]).collect();
    let specs = [
        ("Wide", WavFormat::Pcm24, 2u16, Some(&bext), stereo),
        ("Tight", WavFormat::Pcm16, 1u16, None, x.clone()),
    ];
    for (name, format, channels, bx, data) in specs {
        let spec = WavSpec {
            sample_rate: 48_000,
            channels,
            format,
            extensible: false,
            channel_mask: 0,
        };
        let wav = root.join("daw").join(format!("{name}.wav"));
        std::fs::create_dir_all(wav.parent().unwrap()).unwrap();
        std::fs::write(&wav, encode_wav(&spec, &data, bx)).unwrap();
        let rec = drumspike_core::project::build_source(&wav, &p.dir, "referenced").unwrap();
        drumspike_core::project::add_track(&mut p, name, Some(rec), "kick", 36, Some("average")).unwrap();
    }
    drumspike_core::project::save_project(&mut p).unwrap();
    let svc = service_dsp();
    let sid = svc.project_open(p.dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    (svc, sid, p.dir, root)
}

use drumspike_core::io::{WavFormat, WavSpec, encode_wav};

fn hits() -> Vec<Value> {
    vec![
        json!({"t": 0.25, "v": 90}),
        json!({"t": 1.5, "v": 100, "added": true}),
        json!({"t": 3.499}),
        json!({"t": 9.0}),
    ]
}

#[test]
fn export_spikes_writes_the_file_of_the_core_in_the_source_format_with_its_time_reference() {
    let (svc, sid, dir, root) = spike_project("spikes");
    let ev = hits();
    let r = svc.export_spikes(&sid, "Wide", Some(&ev)).unwrap();
    let path = dir.join("Exports/Wide_spike.wav");
    assert_eq!(r["path"], path.display().to_string());
    assert_eq!((r["sr"].as_u64(), r["format"].as_str()), (Some(48_000), Some("pcm24")));
    assert_eq!(r["n_events"], 4);
    assert_eq!(r["time_reference"], 5_000_000_123u64);
    // the same bytes as the core function with the source's header
    let src = dir.parent().unwrap().join("daw/Wide.wav");
    let (_, info) = drumspike_core::read_wav_bytes(&src).unwrap();
    let want = dir.join("want.wav");
    let times = [0.25, 1.5, 3.499, 9.0];
    drumspike_core::export_spikes(
        &want,
        info.sample_rate,
        info.n_frames,
        &times,
        info.bext.as_ref(),
        WavFormat::Pcm24,
    )
    .unwrap();
    let got = std::fs::read(&path).unwrap();
    assert_eq!(got, std::fs::read(&want).unwrap());
    assert_eq!(r["size"], got.len());
    assert_eq!(r["n_samples"], info.n_frames);
    let (_, back) = drumspike_core::read_wav_bytes(&path).unwrap();
    assert_eq!(back.bext.unwrap().time_reference, 5_000_000_123);
    assert_eq!(
        (back.channels, back.bits_per_sample, back.n_frames),
        (1, 24, info.n_frames)
    );
    // a 16-bit mono source without a bext chunk gives a 16-bit file without one
    let r = svc.export_spikes(&sid, "Tight", Some(&ev)).unwrap();
    assert_eq!(
        (r["format"].as_str(), r["time_reference"].is_null()),
        (Some("pcm16"), true)
    );
    let (_, back) = drumspike_core::read_wav_bytes(&dir.join("Exports/Tight_spike.wav")).unwrap();
    assert!(back.bext.is_none());
    assert_eq!(back.bits_per_sample, 16);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn export_spikes_takes_the_events_of_the_document_without_a_list_and_rejects_bad_calls() {
    let (svc, sid, dir, root) = spike_project("spikes_err");
    svc.events_set(&sid, "Wide", &json!(hits())).unwrap();
    let r = svc.export_spikes(&sid, "Wide", None).unwrap();
    assert_eq!(r["n_events"], 4);
    let from_doc = std::fs::read(dir.join("Exports/Wide_spike.wav")).unwrap();
    svc.export_spikes(&sid, "Wide", Some(&hits())).unwrap();
    assert_eq!(std::fs::read(dir.join("Exports/Wide_spike.wav")).unwrap(), from_doc);
    // no events at all is a silent file with the same header
    svc.export_spikes(&sid, "Tight", None).unwrap();
    assert_eq!(svc.export_spikes(&sid, "nope", None).unwrap_err().code, 400);
    assert_eq!(svc.export_spikes("nope", "Wide", None).unwrap_err().code, 404);
    assert_eq!(
        svc.export_spikes(&sid, "Wide", Some(&[json!({"v": 1})]))
            .unwrap_err()
            .code,
        400
    );
    // a file that changed since the open is offline ("changed"): 409, nothing written
    std::fs::remove_file(dir.join("Exports/Tight_spike.wav")).unwrap();
    std::fs::write(root.join("daw/Tight.wav"), b"ID3 this is not a wav file").unwrap();
    let e = svc.export_spikes(&sid, "Tight", None).unwrap_err();
    assert_eq!(e.code, 409, "{e}");
    assert!(e.message.contains("changed"), "{e}");
    assert!(!dir.join("Exports/Tight_spike.wav").exists());
    std::fs::remove_file(root.join("daw/Wide.wav")).unwrap();
    svc.project_get(&sid).unwrap();
    assert_eq!(svc.export_spikes(&sid, "Wide", None).unwrap_err().code, 409);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn export_all_writes_the_midi_and_the_spike_file_of_every_track_and_reports_progress() {
    let (svc, sid, dir, root) = spike_project("export_all");
    svc.events_set(&sid, "Wide", &json!(hits())).unwrap();
    svc.events_set(&sid, "Tight", &json!([{"t": 0.5, "v": 64}])).unwrap();
    let (progress, got) = collect_progress();
    let r = svc.export_all(&sid, Some(progress)).unwrap();
    assert_eq!((r["midi"].as_u64(), r["spikes"].as_u64()), (Some(2), Some(2)));
    assert_eq!(r["errors"], json!([]));
    assert_eq!(r["files"].as_array().unwrap().len(), 4);
    for f in r["files"].as_array().unwrap() {
        assert!(std::path::Path::new(f.as_str().unwrap()).is_file(), "{f}");
    }
    let want = midi_bytes(&[MidiEvent { t: 0.5, v: 64 }], 36, 24000, 120.0).unwrap();
    assert_eq!(std::fs::read(dir.join("Exports/Tight_spike.mid")).unwrap(), want);
    let single = svc.export_spikes(&sid, "Wide", None).unwrap();
    let bytes = std::fs::read(single["path"].as_str().unwrap()).unwrap();
    svc.export_all(&sid, None).unwrap();
    assert_eq!(std::fs::read(dir.join("Exports/Wide_spike.wav")).unwrap(), bytes);
    let msgs = got.lock().unwrap().clone();
    let running: Vec<_> = msgs.iter().filter(|m| m["export"]["state"] == "running").collect();
    assert_eq!(running.len(), 2);
    assert!(msgs.iter().all(|m| m["sid"] == sid.as_str()));
    let last = msgs.last().unwrap();
    assert_eq!(last["export"]["state"], "done");
    assert_eq!(
        (last["export"]["done"].as_u64(), last["export"]["total"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(last["export"]["result"]["spikes"], 2);
    std::fs::remove_dir_all(&root).unwrap();
}

/// #209: the `audio` format writes `<name>_audio_spike.wav`, cut from the track's own audio
/// (1.5 periods per hit): every snippet starts at the hit sample, the same index the blip
/// spike of the same hit starts at; the source is untouched.
#[test]
fn export_all_writes_the_audio_spikes_when_the_format_is_chosen() {
    let (svc, sid, dir, root) = spike_project("export_all_audio");
    svc.events_set(&sid, "Wide", &json!(hits())).unwrap();
    let src = root.join("daw/Wide.wav");
    let before = file_sha256(&src).unwrap();
    let opts = drumspike_app::export::ExportOpts::from_args(&json!({ "formats": ["audio"] })).unwrap();
    let r = svc.export_all_opts(&sid, &opts, None).unwrap();
    assert_eq!(
        (r["midi"].as_u64(), r["spikes"].as_u64(), r["audio"].as_u64()),
        (Some(0), Some(0), Some(2))
    );
    assert_eq!(r["errors"], json!([]));
    let path = dir.join("Exports/Wide_audio_spike.wav");
    assert!(path.is_file(), "{}", r["files"]);
    assert_eq!(file_sha256(&src).unwrap(), before, "the source's bytes are unchanged");
    // every in-file hit: exact silence up to the hit sample, the track's audio from it (the
    // 3.499 s hit sits 48 samples before the file end: its clamped snippet may be silence)
    let (x, info) = drumspike_core::read_wav_mono(&path, MixMode::Ffmpeg).unwrap();
    assert_eq!((info.sample_rate, info.n_frames), (48_000, 168_000));
    let mut checked = 0;
    let mut with_audio = 0;
    for t in [0.25, 1.5, 3.499] {
        let start = (t * f64::from(info.sample_rate)).round() as usize;
        let from = start.saturating_sub(64);
        assert!(
            x[from..start].iter().all(|&v| v == 0.0),
            "exact silence before the hit at {t} s"
        );
        if x[start..].iter().any(|&v| v != 0.0) {
            let stop = (start + 64).min(x.len());
            assert!(
                x[start..stop].iter().any(|&v| v != 0.0),
                "the snippet of the hit at {t} s holds the track's audio"
            );
            with_audio += 1;
        }
        checked += 1;
    }
    assert_eq!(checked, 3);
    assert!(with_audio >= 2, "the hits inside the file hold the track's audio");
    std::fs::remove_dir_all(&root).unwrap();
}

/// #274: an audio-only export leaves no synthetic spike WAV with the old hits next to it.
#[test]
fn an_audio_spike_export_rewrites_the_older_synthetic_spike_file() {
    let (svc, sid, dir, root) = spike_project("export_also");
    svc.events_set(&sid, "Wide", &json!(hits())).unwrap();
    let synth = drumspike_app::export::ExportOpts::from_args(&json!({ "formats": ["spikes"] })).unwrap();
    svc.export_all_opts(&sid, &synth, None).unwrap();
    let path = dir.join("Exports/Wide_spike.wav");
    let count = |p: &std::path::Path| {
        let (x, _) = drumspike_core::read_wav_mono(p, MixMode::Ffmpeg).unwrap();
        x.iter().filter(|&&v| v != 0.0).count()
    };
    let before = count(&path);
    // delete the 1.5 s hit, then export with the spike type Audio only
    svc.events_set(&sid, "Wide", &json!([{"t": 0.25, "v": 90}, {"t": 3.499}]))
        .unwrap();
    let audio = drumspike_app::export::ExportOpts::from_args(&json!({ "formats": ["audio"] })).unwrap();
    let r = svc.export_all_opts(&sid, &audio, None).unwrap();
    let after = count(&path);
    assert!(
        after < before,
        "the synthetic spike file holds the edited hits: {after} < {before}"
    );
    assert_eq!(r["also_updated"].as_array().map(Vec::len), Some(2), "{r}");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn export_all_goes_on_after_a_track_that_fails_and_lists_its_errors() {
    let (svc, sid, dir, root) = spike_project("export_all_err");
    std::fs::remove_file(root.join("daw/Tight.wav")).unwrap();
    let r = svc.export_all(&sid, None).unwrap();
    // the MIDI file needs no audio; the spike file of the offline track fails with a 409
    assert_eq!((r["midi"].as_u64(), r["spikes"].as_u64()), (Some(2), Some(1)));
    let errs = r["errors"].as_array().unwrap();
    assert_eq!(errs.len(), 1);
    assert_eq!(
        (errs[0]["name"].as_str(), errs[0]["code"].as_u64()),
        (Some("Tight"), Some(409))
    );
    assert!(dir.join("Exports/Wide_spike.wav").is_file() && dir.join("Exports/Tight_spike.mid").is_file());
    assert!(!dir.join("Exports/Tight_spike.wav").exists());
    assert_eq!(svc.export_all("nope", None).unwrap_err().code, 404);
    std::fs::remove_dir_all(&root).unwrap();
}

/// P4-4 (B1): a hit with no sample in the file (before the start, after the end) is not in the
/// spike file and is a note of the MIDI file; every answer counts them as `outside` (the spike
/// export skipped them with no word). The bytes stay as they were.
#[test]
fn the_export_answers_count_the_hits_outside_the_audio_file() {
    let (svc, sid, dir, root) = spike_project("outside");
    // the 3.5 s file: -0.001 s rounds to sample -48, 3.5 s is one past the last sample, 9 s is after the end
    let ev = vec![
        json!({"t": -0.001, "v": 90}),
        json!({"t": 0.25, "v": 90}),
        json!({"t": 3.5 - 1.0 / 48_000.0}),
        json!({"t": 3.5}),
        json!({"t": 9.0, "cut": true}),
        json!({"t": 9.0}),
    ];
    let r = svc.export_spikes(&sid, "Wide", Some(&ev)).unwrap();
    assert_eq!(
        (r["n_events"].as_u64(), r["outside"].as_u64()),
        (Some(5), Some(3)),
        "{r}"
    );
    let want = dir.join("want.wav");
    let src = root.join("daw/Wide.wav");
    let (_, info) = drumspike_core::read_wav_bytes(&src).unwrap();
    drumspike_core::export_spikes(
        &want,
        48_000,
        info.n_frames,
        &[0.25, 3.5 - 1.0 / 48_000.0],
        info.bext.as_ref(),
        WavFormat::Pcm24,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(dir.join("Exports/Wide_spike.wav")).unwrap(),
        std::fs::read(&want).unwrap()
    );
    let m = svc.export_midi(&sid, "Wide", 36, &ev, 24000, 120.0).unwrap();
    assert_eq!(
        (m["n_events"].as_u64(), m["outside"].as_u64()),
        (Some(5), Some(3)),
        "{m}"
    );
    assert_eq!(m["before"], 1, "-0.001 s: before the start, at 0 s in the MIDI file");
    svc.events_set(&sid, "Wide", &json!(ev)).unwrap();
    svc.events_set(&sid, "Tight", &json!([{"t": 0.5, "v": 64}])).unwrap();
    let all = svc.export_all(&sid, None).unwrap();
    assert_eq!(all["outside"], json!({"Wide": 3}), "a track with none is left out");
    assert_eq!(svc.export_spikes(&sid, "Tight", None).unwrap()["outside"], 0);
    std::fs::remove_dir_all(&root).unwrap();
}

/// P4-4 (B3): `Kick` and `kick` are two tracks but one file name on a disk that ignores case
/// (APFS by default). Export All wrote both tracks' files into the same two files, in a race; now
/// the later track (in the document's order) is a 409 and the earlier one keeps its files. The
/// single exports refuse it too.
#[test]
fn two_track_names_that_differ_only_in_case_never_share_their_export_files() {
    let root = tmp("case_names");
    let mut p = drumspike_core::project::new_project(&root, "Case").unwrap();
    for (name, folder) in [("Kick", "a"), ("kick", "b"), ("Snare", "a")] {
        let wav = root.join(folder).join(format!("{name}.wav"));
        write_wav(&wav, 48_000, &kick_signal(&ONSETS, 3.5, 48_000));
        let rec = drumspike_core::project::build_source(&wav, &p.dir, "referenced").unwrap();
        drumspike_core::project::add_track(&mut p, name, Some(rec), "kick", 36, Some("average")).unwrap();
    }
    drumspike_core::project::save_project(&mut p).unwrap();
    let svc = service_dsp();
    let sid = svc.project_open(p.dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.events_set(&sid, "Kick", &json!([{"t": 0.5, "v": 100}])).unwrap();
    svc.events_set(&sid, "kick", &json!([{"t": 1.0, "v": 50}, {"t": 2.0, "v": 60}]))
        .unwrap();
    let all = svc.export_all(&sid, None).unwrap();
    let errs = all["errors"].as_array().unwrap();
    assert_eq!(errs.len(), 1, "{all}");
    assert_eq!(
        (errs[0]["name"].as_str(), errs[0]["code"].as_u64()),
        (Some("kick"), Some(409))
    );
    assert!(errs[0]["message"].as_str().unwrap().contains("track Kick"), "{all}");
    assert_eq!((all["midi"].as_u64(), all["spikes"].as_u64()), (Some(2), Some(2)));
    let want = midi_bytes(&[MidiEvent { t: 0.5, v: 100 }], 36, 24000, 120.0).unwrap();
    assert_eq!(
        std::fs::read(p.dir.join("Exports/Kick_spike.mid")).unwrap(),
        want,
        "Kick's own file"
    );
    assert_eq!(svc.export_spikes(&sid, "kick", None).unwrap_err().code, 409);
    assert_eq!(
        svc.export_midi(&sid, "kick", 36, &[], 24000, 120.0).unwrap_err().code,
        409
    );
    assert!(svc.export_spikes(&sid, "Kick", None).is_ok());
    assert!(svc.export_spikes(&sid, "Snare", None).is_ok());
    // the overwrite prompt lists the files Export All replaces: Kick's and Snare's, once
    let ex = svc.export_existing(&sid).unwrap();
    assert_eq!(ex["count"], 4, "{ex}");
    std::fs::remove_dir_all(&root).unwrap();
}

/// P4-4 fix round 2: a stale follower whose name differs only in case from an earlier track gets the
/// owner message, not "earlier export": the files in `Exports/` are the other track's.
#[test]
fn a_stale_follower_that_shares_its_file_names_gets_the_owner_error_first() {
    let root = tmp("case_stale");
    let mut p = drumspike_core::project::new_project(&root, "CaseStale").unwrap();
    for (name, folder) in [("Kick", "a"), ("kick", "b"), ("Snare", "a")] {
        let wav = root.join(folder).join(format!("{name}.wav"));
        write_wav(&wav, 48_000, &kick_signal(&ONSETS, 3.5, 48_000));
        let rec = drumspike_core::project::build_source(&wav, &p.dir, "referenced").unwrap();
        drumspike_core::project::add_track(&mut p, name, Some(rec), "kick", 36, Some("average")).unwrap();
    }
    drumspike_core::project::save_project(&mut p).unwrap();
    let svc = service_dsp();
    let sid = svc.project_open(p.dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.events_set(&sid, "Snare", &json!([{"t": 0.5, "v": 100}])).unwrap();
    svc.events_set(&sid, "Kick", &json!([{"t": 0.5, "v": 100}])).unwrap();
    svc.export_all(&sid, None).unwrap(); // Exports/Kick_spike.* exist now
    svc.track_set_follow(&sid, "kick", Some("Snare")).unwrap();
    let wav = root.join("b/kick.wav");
    std::fs::rename(&wav, root.join("b/kick.away")).unwrap();
    svc.project_get(&sid).unwrap();
    svc.events_set(&sid, "Snare", &json!([{"t": 0.5, "v": 100}, {"t": 1.5, "v": 100}]))
        .unwrap();
    let all = svc.export_all(&sid, None).unwrap();
    let errs = all["errors"].as_array().unwrap();
    let e = errs.iter().find(|e| e["name"] == "kick").expect("kick is an error");
    let m = e["message"].as_str().unwrap();
    assert!(m.contains("track Kick"), "{m}");
    assert!(!m.contains("earlier export"), "{m}");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn velocities_recompute_keeps_times_and_every_other_key_and_follows_velocity_map() {
    let (svc, sid, _dir, root) = spike_project("vel");
    let ev = vec![
        json!({"t": 0.25, "v": 1, "added": true, "orig": 0.24, "id": "l4", "planted": {"a": [1]}}),
        json!({"t": 1.25, "v": 127}),
        json!({"t": 2.25}),
        json!({"t": 99.0, "v": 55}),
    ];
    let r = svc.velocities_recompute(&sid, "Tight", Some(&ev)).unwrap();
    let out = r["events"].as_array().unwrap();
    assert_eq!(out.len(), 4);
    let src = root.join("daw/Tight.wav");
    let (x, info) = read_wav_mono(&src, MixMode::Average).unwrap();
    let times = [0.25, 1.25, 2.25, 99.0];
    let want = drumspike_core::velocity_map(&x, info.sample_rate, &times, &drumspike_core::VelocityMap::default());
    for (i, e) in out.iter().enumerate() {
        assert_eq!(e["t"].as_f64().unwrap(), times[i]);
        assert_eq!(e["v"].as_i64().unwrap(), want[i], "event {i}");
    }
    assert!(
        want[0] > 20 && want[3] == 20,
        "{want:?}: a hit in the audio is louder than one past the end"
    );
    assert_eq!(out[0]["planted"], json!({"a": [1]}));
    assert_eq!(
        (
            out[0]["id"].as_str(),
            out[0]["added"].as_bool(),
            out[0]["orig"].as_f64()
        ),
        (Some("l4"), Some(true), Some(0.24))
    );
    assert_eq!(
        out[0].as_object().unwrap().keys().collect::<Vec<_>>(),
        ["t", "v", "added", "orig", "id", "planted"],
        "the key order stays"
    );
    // the track's vel_lo_db and vel_hi_db change the map; without events the document's are used
    svc.analyze_start(
        &sid,
        Some(vec!["Tight".into()]),
        Some(&serde_json::from_value(json!({"Tight": {"vel_lo_db": -60.0, "vel_hi_db": -20.0}})).unwrap()),
        None,
    )
    .unwrap();
    wait_done(&svc, &sid);
    let n = svc.events_get(&sid, "Tight").unwrap()["events"]
        .as_array()
        .unwrap()
        .len();
    let r = svc.velocities_recompute(&sid, "Tight", None).unwrap();
    assert_eq!(r["events"].as_array().unwrap().len(), n);
    let map = drumspike_core::VelocityMap {
        lo_db: -60.0,
        hi_db: -20.0,
        ..Default::default()
    };
    let doc_times: Vec<f64> = r["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["t"].as_f64().unwrap())
        .collect();
    let want = drumspike_core::velocity_map(&x, info.sample_rate, &doc_times, &map);
    let got: Vec<i64> = r["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["v"].as_i64().unwrap())
        .collect();
    assert_eq!(got, want);
    // errors
    assert_eq!(svc.velocities_recompute(&sid, "nope", None).unwrap_err().code, 400);
    assert_eq!(svc.velocities_recompute("nope", "Tight", None).unwrap_err().code, 404);
    assert_eq!(
        svc.velocities_recompute(&sid, "Tight", Some(&[json!({"v": 3})]))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        svc.velocities_recompute(&sid, "Tight", Some(&[json!(3)]))
            .unwrap_err()
            .code,
        400
    );
    std::fs::remove_file(&src).unwrap();
    svc.project_get(&sid).unwrap();
    assert_eq!(svc.velocities_recompute(&sid, "Tight", None).unwrap_err().code, 409);
    std::fs::remove_dir_all(&root).unwrap();
}
