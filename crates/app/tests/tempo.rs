//! P4-8 stage A: the tempo and meter map commands and the MIDI export through the map
//! (`crates/app/src/tempo.rs`), through `drumspike_app::Service`, no Tauri.

mod common;

use common::*;
use drumspike_app::Service;
use drumspike_core::midi::{MidiEvent, midi_bytes};
use serde_json::{Value, json};

const HITS: [f64; 6] = [0.25, 0.75, 1.25, 1.75, 2.25, 2.75];

fn opened(tag: &str) -> (Service, String, std::path::PathBuf) {
    open_with(tag, &[kick_spec("k in", "kick", 36), kick_spec("s top", "snare", 38)])
}

/// `opened` with tracks of 150 s at 8 kHz: the detection leaves out hits after the tracks' end
/// (P4-8 fix round 2), so a train of a minute or two needs tracks that long.
fn opened_long(tag: &str) -> (Service, String, std::path::PathBuf) {
    let long = |name, role, note| Spec {
        name,
        role,
        note,
        x: kick_signal(&ONSETS, 150.0, 8000),
        sr: 8000,
    };
    open_with(tag, &[long("k in", "kick", 36), long("s top", "snare", 38)])
}

fn open_with(tag: &str, specs: &[Spec]) -> (Service, String, std::path::PathBuf) {
    let root = tmp(tag);
    let dir = project(&root, "Tempo", specs);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let ev: Vec<Value> = HITS.iter().map(|&t| json!({"t": t, "v": 90})).collect();
    for n in ["k in", "s top"] {
        svc.events_set(&sid, n, &Value::Array(ev.clone())).unwrap();
    }
    (svc, sid, dir)
}

fn map3() -> Value {
    // 120 BPM, then 100 BPM at 1 s, 150 BPM at 2 s; 3/4 from bar 3; bar 1 at 0.5 s. All on whole ticks.
    json!({"bpm": 120.0, "steps": [{"t": 1.0, "bpm": 100.0}, {"t": 2.0, "bpm": 150.0}],
           "meter": [4, 4], "changes": [{"bar": 3, "meter": [3, 4]}], "bar1_s": 0.5})
}

fn note_ticks(path: &str) -> Vec<u64> {
    let bytes = std::fs::read(path).unwrap();
    let smf = midly::Smf::parse(&bytes).unwrap();
    let mut at = 0u64;
    let mut out = vec![];
    for e in &smf.tracks[1] {
        at += u64::from(e.delta.as_int());
        if let midly::TrackEventKind::Midi {
            message: midly::MidiMessage::NoteOn { vel, .. },
            ..
        } = e.kind
            && vel.as_int() > 0
        {
            out.push(at);
        }
    }
    out
}

#[test]
fn a_new_project_has_the_default_map_and_the_key_is_not_stored() {
    let (svc, sid, dir) = opened("tm1");
    let g = svc.tempo_map_get(&sid).unwrap();
    assert_eq!(g["default"], true);
    assert_eq!(g["map"]["bpm"], 120.0);
    assert_eq!(g["map"]["meter"], json!([4, 4]));
    assert_eq!(g["map"]["bar1_s"], 0.0);
    // setting the default map is no change and leaves the document without the key
    svc.project_save(&sid).unwrap();
    let r = svc.tempo_map_set(&sid, &json!({})).unwrap();
    assert_eq!(r["changed"], false);
    assert!(!svc.project_get(&sid).unwrap()["dirty"].as_bool().unwrap());
    assert!(
        !std::fs::read_to_string(dir.join("Tempo.drumspike"))
            .unwrap()
            .contains("tempo_map")
    );
}

#[test]
fn set_replaces_the_whole_map_in_one_step_and_previous_undoes_it() {
    let (svc, sid, dir) = opened("tm2");
    svc.project_save(&sid).unwrap();
    let r = svc.tempo_map_set(&sid, &map3()).unwrap();
    assert_eq!(
        (r["changed"].clone(), r["previous"]["bpm"].clone()),
        (json!(true), json!(120.0))
    );
    assert_eq!(r["map"], svc.tempo_map_get(&sid).unwrap()["map"]);
    assert!(svc.project_get(&sid).unwrap()["dirty"].as_bool().unwrap());
    // the second call replaces the first as a whole (no merge with the steps of the first)
    let r2 = svc.tempo_map_set(&sid, &json!({"bpm": 90.0})).unwrap();
    assert_eq!(r2["map"]["steps"], json!([]));
    assert_eq!(r2["previous"], r["map"]);
    // undo: one call with the previous map
    let back = svc.tempo_map_set(&sid, &r2["previous"]).unwrap();
    assert_eq!(back["map"], r["map"]);
    // save and reopen: the map is in the document, the format stays 1
    svc.project_save(&sid).unwrap();
    let text = std::fs::read_to_string(dir.join("Tempo.drumspike")).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["format"], 1);
    assert_eq!(doc["tempo_map"]["steps"][1]["bpm"], 150.0);
    svc.project_close(&sid, true).unwrap();
    let sid2 = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(svc.tempo_map_get(&sid2).unwrap()["map"], r["map"]);
    // a null map is the default map and removes the key again
    svc.tempo_map_set(&sid2, &Value::Null).unwrap();
    svc.project_save(&sid2).unwrap();
    assert!(
        !std::fs::read_to_string(dir.join("Tempo.drumspike"))
            .unwrap()
            .contains("tempo_map")
    );
}

#[test]
fn a_bad_map_is_refused_with_400_and_changes_nothing() {
    let (svc, sid, _) = opened("tm3");
    svc.tempo_map_set(&sid, &map3()).unwrap();
    for bad in [
        json!({"bpm": 0}),
        json!({"bpm": 100, "tempo": 1}),
        json!({"steps": [{"t": 2.0, "bpm": 100}, {"t": 1.0, "bpm": 90}]}),
        json!({"meter": [4, 5]}),
        json!({"changes": [{"bar": 1, "meter": [3, 4]}]}),
        json!({"bar1_s": -2.0}),
        json!("fast"),
    ] {
        let e = svc.tempo_map_set(&sid, &bad).unwrap_err();
        assert_eq!(e.code, 400, "{bad}: {}", e.message);
    }
    assert_eq!(
        svc.tempo_map_get(&sid).unwrap()["map"],
        svc.tempo_map_set(&sid, &map3()).unwrap()["map"]
    );
}

#[test]
fn the_default_map_exports_the_bytes_of_before() {
    let (svc, sid, _) = opened("tm4");
    let ev: Vec<MidiEvent> = HITS.iter().map(|&t| MidiEvent { t, v: 90 }).collect();
    let want = midi_bytes(&ev, 36, 24_000, 120.0).unwrap();
    let r = svc
        .export_midi(&sid, "k in", 36, &events_json(), 24_000, 120.0)
        .unwrap();
    assert_eq!(std::fs::read(r["path"].as_str().unwrap()).unwrap(), want);
    let all = svc.export_all(&sid, None).unwrap();
    assert_eq!(all["errors"], json!([]));
    let p = r["path"].as_str().unwrap().replace("k in", "s top");
    assert_eq!(std::fs::read(p).unwrap(), midi_bytes(&ev, 38, 24_000, 120.0).unwrap());
    // a map that is set and set back to the default writes the old bytes too
    svc.tempo_map_set(&sid, &map3()).unwrap();
    svc.tempo_map_set(&sid, &json!({})).unwrap();
    let r = svc
        .export_midi(&sid, "k in", 36, &events_json(), 24_000, 120.0)
        .unwrap();
    assert_eq!(std::fs::read(r["path"].as_str().unwrap()).unwrap(), want);
}

fn events_json() -> Vec<Value> {
    HITS.iter().map(|&t| json!({"t": t, "v": 90})).collect()
}

#[test]
fn a_map_changes_the_ticks_of_the_single_and_the_all_export_and_imports_back() {
    let (svc, sid, _) = opened("tm5");
    svc.tempo_map_set(&sid, &map3()).unwrap();
    // 120 BPM to 1 s (48,000 ticks per second), 100 BPM to 2 s (40,000), then 150 BPM (60,000)
    let want: Vec<u64> = vec![
        12_000,
        36_000,
        48_000 + 10_000,
        48_000 + 30_000,
        88_000 + 15_000,
        88_000 + 45_000,
    ];
    let r = svc
        .export_midi(&sid, "k in", 36, &events_json(), 24_000, 120.0)
        .unwrap();
    let path = r["path"].as_str().unwrap().to_string();
    assert_eq!(note_ticks(&path), want, "single export; the tempo argument is not used");
    let all = svc.export_all(&sid, None).unwrap();
    assert_eq!(all["errors"], json!([]));
    assert_eq!(note_ticks(&path.replace("k in", "s top")), want, "Export All");
    // the file's map comes back through the import command, in a project with another map
    let (svc2, sid2, _) = opened("tm5b");
    svc2.tempo_map_set(&sid2, &json!({"bpm": 77.0})).unwrap();
    let imp = svc2.tempo_map_import_midi(&sid2, &path).unwrap();
    assert_eq!(imp["map"], svc.tempo_map_get(&sid).unwrap()["map"]);
    assert_eq!(imp["previous"]["bpm"], 77.0);
    assert_eq!(
        (imp["ppq"].clone(), imp["tempo_events"].clone(), imp["snapped"].clone()),
        (json!(24_000), json!(3), json!(0))
    );
    assert!(svc2.project_get(&sid2).unwrap()["dirty"].as_bool().unwrap());
}

#[test]
fn import_refuses_a_missing_file_a_non_midi_file_and_a_file_without_events() {
    let (svc, sid, dir) = opened("tm6");
    svc.tempo_map_set(&sid, &map3()).unwrap();
    let before = svc.tempo_map_get(&sid).unwrap();
    assert_eq!(
        svc.tempo_map_import_midi(&sid, "/nonexistent/x.mid").unwrap_err().code,
        404
    );
    let junk = dir.join("junk.mid");
    std::fs::write(&junk, b"not midi at all").unwrap();
    assert_eq!(
        svc.tempo_map_import_midi(&sid, junk.to_str().unwrap())
            .unwrap_err()
            .code,
        400
    );
    // a valid file with a note track only: no tempo and no meter event
    let bare = [
        b"MThd".as_slice(),
        &[0, 0, 0, 6, 0, 0, 0, 1, 0x01, 0xE0],
        b"MTrk",
        &[0, 0, 0, 4, 0, 0xFF, 0x2F, 0],
    ]
    .concat();
    let p = dir.join("bare.mid");
    std::fs::write(&p, bare).unwrap();
    let e = svc.tempo_map_import_midi(&sid, p.to_str().unwrap()).unwrap_err();
    assert_eq!(e.code, 400, "{}", e.message);
    assert_eq!(svc.tempo_map_get(&sid).unwrap(), before, "nothing changed");
}

#[test]
fn a_read_only_project_refuses_the_set_and_the_import_but_answers_the_get() {
    let (svc, sid, dir) = opened("tm7");
    svc.project_save(&sid).unwrap();
    svc.project_close(&sid, true).unwrap();
    let opened = svc.project_open_with(&dir.display().to_string(), true).unwrap();
    let sid = opened["sid"].as_str().unwrap().to_string();
    let call = |cmd: &str, args: Value| drumspike_app::call::call(&svc, cmd, &args, None);
    assert!(call("tempo_map_get", json!({"sid": sid})).is_ok());
    let e = call("tempo_map_set", json!({"sid": sid, "map": {"bpm": 90.0}}))
        .err()
        .unwrap();
    assert_eq!(e.code, 409, "{}", e.message);
    let e = call("tempo_map_import_midi", json!({"sid": sid, "path": "/x.mid"}))
        .err()
        .unwrap();
    assert_eq!(e.code, 409);
    // stage B: the detection only reads, so a read-only project answers it
    assert!(call("tempo_map_detect", json!({"sid": sid})).is_ok());
}

// ---------------------------------------------------------------- P4-8 stage B: detection

/// Bar 1 of the synthetic train, the tempo of its two halves and the bar of the step.
const B1: f64 = 0.6;
const BPM_A: f64 = 100.0;
const BPM_B: f64 = 125.0;
const STEP_BAR: usize = 17;

/// Kick on beats 1 and 3, snare on 2 and 4: 16 bars at 100 BPM from 0.6 s, then 16 bars at 125.
/// Returns the kick and snare events and the time of the step.
fn train() -> (Vec<Value>, Vec<Value>, f64) {
    let (mut k, mut s) = (vec![], vec![]);
    let mut t = B1;
    let mut step = 0.0;
    for bar in 1..=32 {
        let beat = 60.0 / if bar < STEP_BAR { BPM_A } else { BPM_B };
        if bar == STEP_BAR {
            step = t;
        }
        for b in 0..4 {
            let e = json!({"t": t + f64::from(b) * beat, "v": 100});
            if b % 2 == 0 { k.push(e) } else { s.push(e) }
        }
        t += 4.0 * beat;
    }
    (k, s, step)
}

fn with_train(tag: &str) -> (Service, String, std::path::PathBuf, f64) {
    let (svc, sid, dir) = opened_long(tag);
    let (k, s, step) = train();
    svc.events_set(&sid, "k in", &Value::Array(k)).unwrap();
    svc.events_set(&sid, "s top", &Value::Array(s)).unwrap();
    svc.project_save(&sid).unwrap();
    (svc, sid, dir, step)
}

#[test]
fn detect_suggests_the_map_of_the_hits_and_changes_nothing() {
    let (svc, sid, _, step) = with_train("tm_det1");
    let r = svc.tempo_map_detect(&sid, None, None, None).unwrap();
    let m = &r["suggestion"];
    assert!((m["bpm"].as_f64().unwrap() - BPM_A).abs() < 0.01, "{r}");
    assert_eq!(m["steps"].as_array().unwrap().len(), 1, "{r}");
    assert!((m["steps"][0]["bpm"].as_f64().unwrap() - BPM_B).abs() < 0.01, "{r}");
    assert!((m["steps"][0]["t"].as_f64().unwrap() - step).abs() < 0.001, "{r}");
    assert!((m["bar1_s"].as_f64().unwrap() - B1).abs() < 0.001, "{r}");
    assert_eq!(m["meter"], json!([4, 4]));
    assert_eq!(r["steps"][0]["bar"], STEP_BAR as i64);
    assert_eq!(r["steps"][0]["bars"], 16);
    assert_eq!(r["changed"], false);
    // kick on 1 and 3, snare on 2 and 4 and nothing between: 2/4 fits as well (fix round 2)
    assert_eq!(r["meter"]["feel"], "two-beat", "{r}");
    assert_eq!(r["warnings"].as_array().unwrap().len(), 1, "{r}");
    assert!(r["warnings"][0].as_str().unwrap().contains("compound feel"), "{r}");
    // fix round 2: the error bar, the bar-1 measures and alternative, the alternatives' comb
    assert!(r["start"]["error_bar"].as_f64().unwrap() >= 0.0, "exact train: 0 {r}");
    assert_eq!(r["start"]["range_edge"], false);
    let alt = r["bar1"]["alternative"].as_f64().unwrap();
    assert!((alt - (B1 + 2.0 * 60.0 / BPM_A)).abs() < 0.001, "{r}");
    let c = r["bar1"]["confidence"].as_f64().unwrap();
    let (bb, db) = (
        r["bar1"]["backbeat"].as_f64().unwrap(),
        r["bar1"]["downbeat"].as_f64().unwrap(),
    );
    assert!((c - bb * db).abs() < 0.002, "{r}");
    for a in r["start"]["alternatives"].as_array().unwrap() {
        assert!(a["comb"].is_number() && a["gated"].is_boolean(), "{a}");
    }
    assert_eq!(
        (r["fit"]["late"].clone(), r["fit"]["isolated"].clone()),
        (json!(0), json!(0))
    );
    assert_eq!(r["params"], json!({"min_bars": 4, "min_step_pct": 2.0}));
    assert_eq!(
        r["tracks"],
        json!([{"name": "k in", "role": "kick", "hits": 64}, {"name": "s top", "role": "snare", "hits": 64}])
    );
    // read only: the map and the dirty flag stay
    assert_eq!(svc.tempo_map_get(&sid).unwrap()["default"], true);
    assert!(!svc.project_get(&sid).unwrap()["dirty"].as_bool().unwrap());
    // accepted with tempo_map_set: every kick of the export is on beat 1 or 3 of its bar
    svc.tempo_map_set(&sid, m).unwrap();
    let path = svc.export_midi_stored(&sid, "k in", None, None, None, None).unwrap()["path"]
        .as_str()
        .unwrap()
        .to_string();
    let ticks = note_ticks(&path);
    assert_eq!(ticks.len(), 64);
    let bar1 = 24_000u64; // 0.6 s at 100 BPM is one quarter
    for (i, t) in ticks.iter().enumerate() {
        let rel = t - bar1;
        assert!(rel % 48_000 <= 1 || rel % 48_000 >= 47_999, "kick {i} at tick {t}");
    }
    assert_eq!(ticks[32] - bar1, 16 * 96_000, "bar 17 beat 1");
}

#[test]
fn detect_reads_named_tracks_skips_followers_and_cut_hits_and_checks_its_arguments() {
    let (svc, sid, _, _) = with_train("tm_det2");
    // the 4-bar rule as an argument: with min_bars 20 the 16 bars at 125 are no step
    let r = svc.tempo_map_detect(&sid, None, Some(20), None).unwrap();
    assert_eq!(r["steps"], json!([]), "{r}");
    // a step of 25 % is under a min_step_pct of 30
    let r = svc.tempo_map_detect(&sid, None, None, Some(30.0)).unwrap();
    assert_eq!(r["steps"], json!([]), "{r}");
    // named tracks: the kick alone gives a warning and a lower confidence
    let both = svc.tempo_map_detect(&sid, None, None, None).unwrap();
    let r = svc
        .tempo_map_detect(&sid, Some(&["k in".to_string()]), None, None)
        .unwrap();
    assert_eq!(r["tracks"].as_array().unwrap().len(), 1);
    assert!(r["warnings"][0].as_str().unwrap().contains("snare"), "{r}");
    assert!(r["start"]["confidence"].as_f64().unwrap() < both["start"]["confidence"].as_f64().unwrap());
    assert_eq!(
        svc.tempo_map_detect(&sid, Some(&["nope".to_string()]), None, None)
            .unwrap_err()
            .code,
        404
    );
    for (bars, pct) in [
        (Some(0), None),
        (Some(65), None),
        (Some(-1), None),
        (None, Some(0.0)),
        (None, Some(f64::NAN)),
    ] {
        assert_eq!(
            svc.tempo_map_detect(&sid, None, bars, pct).unwrap_err().code,
            400,
            "{bars:?} {pct:?}"
        );
    }
    // cut hits are left out: every snare hit cut leaves the kick alone
    let (_, s, _) = train();
    let cut: Vec<Value> = s.iter().map(|e| json!({"t": e["t"], "v": 100, "cut": true})).collect();
    svc.events_set(&sid, "s top", &Value::Array(cut)).unwrap();
    let r = svc.tempo_map_detect(&sid, None, None, None).unwrap();
    assert_eq!(r["tracks"][1]["hits"], 0, "{r}");
    assert!(!r["warnings"].as_array().unwrap().is_empty());
    // P4-14: so are hits of the tom bleed rule
    let bleed: Vec<Value> = s
        .iter()
        .map(|e| json!({"t": e["t"], "v": 100, "bleed": true}))
        .collect();
    svc.events_set(&sid, "s top", &Value::Array(bleed)).unwrap();
    let r = svc.tempo_map_detect(&sid, None, None, None).unwrap();
    assert_eq!(r["tracks"][1]["hits"], 0, "{r}");
    // a follower is not read by default (its hits are another track's)
    svc.events_set(&sid, "s top", &Value::Array(s)).unwrap();
    svc.track_set_follow(&sid, "s top", Some("k in")).unwrap();
    let r = svc.tempo_map_detect(&sid, None, None, None).unwrap();
    assert_eq!(r["tracks"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["tracks"][0]["name"], "k in");
    // through the dispatcher, as the CLI and MCP call it
    let a = drumspike_app::call::call(
        &svc,
        "tempo_map_detect",
        &json!({"sid": sid, "tracks": ["k in"], "min_bars": 4}),
        None,
    )
    .unwrap();
    let drumspike_app::reply::Reply::Json(a) = a else {
        panic!("not JSON")
    };
    assert_eq!(a["tracks"][0]["name"], "k in");
    // too few hits: 400 with the reason
    let three: Vec<Value> = [0.5, 1.0, 1.5].iter().map(|&t| json!({"t": t, "v": 90})).collect();
    svc.events_set(&sid, "k in", &Value::Array(three)).unwrap();
    let e = svc.tempo_map_detect(&sid, None, None, None).unwrap_err();
    assert_eq!(e.code, 400);
    assert!(e.message.contains("at least 8"), "{}", e.message);
}

// ---------------------------------------------------------------- P4-8 fix round 1

/// Rock pattern (kick on 1, 3 and the eighth after 3, snare on 2 and 4), bars at a tempo each.
fn sections_train(secs: &[(usize, f64)]) -> (Vec<Value>, Vec<Value>, Vec<f64>) {
    let (mut k, mut s, mut starts) = (vec![], vec![], vec![]);
    let mut t = 1.0;
    for &(bars, bpm) in secs {
        starts.push(t);
        let beat = 60.0 / bpm;
        for _ in 0..bars {
            for (b, kick) in [(0.0, true), (1.0, false), (2.0, true), (2.5, true), (3.0, false)] {
                let e = json!({"t": t + b * beat, "v": 100});
                if kick { k.push(e) } else { s.push(e) }
            }
            t += 4.0 * beat;
        }
    }
    (k, s, starts)
}

/// Save the project, put `value` under the key `tempo_map` of its document file and open it again.
fn with_stored_map(tag: &str, value: Value) -> (Service, String, std::path::PathBuf) {
    let (svc, sid, dir) = opened(tag);
    svc.project_save(&sid).unwrap();
    svc.project_close(&sid, true).unwrap();
    let file = dir.join("Tempo.drumspike");
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    doc["tempo_map"] = value;
    std::fs::write(&file, serde_json::to_string(&doc).unwrap()).unwrap();
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    (svc, sid, dir)
}

#[test]
fn a_stored_map_that_is_not_valid_is_replaced_by_set_with_previous_null() {
    let (svc, sid, _) = with_stored_map("tm_bad", json!({"bpm": 0}));
    assert_eq!(svc.tempo_map_get(&sid).unwrap_err().code, 400);
    assert_eq!(
        svc.export_midi(&sid, "k in", 36, &events_json(), 24_000, 120.0)
            .unwrap_err()
            .code,
        400,
        "the stored map blocks the export"
    );
    // `{}` is the default map, but the stored value is broken: the call must write
    let r = svc.tempo_map_set(&sid, &json!({})).unwrap();
    assert_eq!(r["changed"], true, "{r}");
    assert_eq!(r["previous"], Value::Null, "{r}");
    assert!(svc.project_get(&sid).unwrap()["dirty"].as_bool().unwrap());
    assert_eq!(svc.tempo_map_get(&sid).unwrap()["default"], true);
    svc.export_midi(&sid, "k in", 36, &events_json(), 24_000, 120.0)
        .unwrap();
    // set again: now the stored map is valid, so the same call changes nothing
    let r = svc.tempo_map_set(&sid, &json!({})).unwrap();
    assert_eq!(
        (r["changed"].clone(), r["previous"]["bpm"].clone()),
        (json!(false), json!(120.0))
    );
}

#[test]
fn a_stored_map_with_an_unknown_key_exports_but_a_caller_map_with_one_is_refused() {
    let (svc, sid, _) = with_stored_map("tm_fwd", json!({"bpm": 90.0, "swing": 0.6}));
    assert_eq!(svc.tempo_map_get(&sid).unwrap()["map"]["bpm"], 90.0);
    svc.export_midi(&sid, "k in", 36, &events_json(), 24_000, 120.0)
        .unwrap();
    assert_eq!(svc.export_all(&sid, None).unwrap()["errors"], json!([]));
    let e = svc
        .tempo_map_set(&sid, &json!({"bpm": 90.0, "swing": 0.6}))
        .unwrap_err();
    assert_eq!(e.code, 400, "{}", e.message);
}

#[test]
fn the_map_schemas_describe_the_map() {
    use drumspike_app::api::find;
    let set = find("tempo_map_set").unwrap();
    let m = &set.args_schema()["properties"]["map"];
    assert_eq!(m["properties"]["steps"]["items"]["required"], json!(["t", "bpm"]));
    assert_eq!(m["properties"]["bpm"]["minimum"], 4);
    assert_eq!(m["properties"]["meter"]["maxItems"], 2);
    assert_eq!(m["properties"]["changes"]["items"]["properties"]["bar"]["minimum"], 2);
    assert!(m["properties"]["bar1_s"]["type"] == "number");
    let r = set.result_schema();
    assert_eq!(r["properties"]["map"]["properties"]["bpm"]["maximum"], 1000);
    for (name, path) in [
        ("tempo_map_get", "map"),
        ("tempo_map_detect", "suggestion"),
        ("tempo_map_import_midi", "map"),
    ] {
        let r = find(name).unwrap().result_schema();
        assert!(r["properties"][path]["properties"]["steps"].is_object(), "{name}");
    }
    // a real answer has the keys the schema names
    let (svc, sid, _) = opened("tm_schema");
    let got = svc.tempo_map_get(&sid).unwrap()["map"].clone();
    let props = find("tempo_map_get").unwrap().result_schema()["properties"]["map"]["properties"].clone();
    for k in got.as_object().unwrap().keys() {
        assert!(props[k].is_object(), "key {k} is not in the schema");
    }
}

#[test]
fn detect_warns_when_a_short_excursion_pulls_the_tempo() {
    let (svc, sid, _) = opened_long("tm_exc");
    let (k, s, starts) = sections_train(&[(24, 120.0), (3, 140.0), (24, 120.0)]);
    svc.events_set(&sid, "k in", &Value::Array(k)).unwrap();
    svc.events_set(&sid, "s top", &Value::Array(s)).unwrap();
    let r = svc.tempo_map_detect(&sid, None, None, None).unwrap();
    let w = r["warnings"].as_array().unwrap();
    assert_eq!(w.len(), 1, "{r}");
    assert!(w[0].as_str().unwrap().contains("shorter than min_bars"), "{r}");
    assert_eq!(r["excursions"].as_array().unwrap().len(), 1, "{r}");
    assert!(
        (r["excursions"][0]["t"].as_f64().unwrap() - starts[1]).abs() < 0.01,
        "{r}"
    );
    assert_eq!(r["excursions"][0]["bars"], 3);
    assert!(
        r["start"]["confidence"].as_f64().unwrap() > 0.5,
        "the confidence is not quartered"
    );
    // with min_bars 2 the excursion is a step: no warning
    let r = svc.tempo_map_detect(&sid, None, Some(2), None).unwrap();
    assert_eq!(
        (r["warnings"].clone(), r["excursions"].clone()),
        (json!([]), json!([])),
        "{r}"
    );
    // a steady song has none
    let (k, s, _) = sections_train(&[(48, 120.0)]);
    svc.events_set(&sid, "k in", &Value::Array(k)).unwrap();
    svc.events_set(&sid, "s top", &Value::Array(s)).unwrap();
    let r = svc.tempo_map_detect(&sid, None, None, None).unwrap();
    assert_eq!(
        (r["warnings"].clone(), r["excursions"].clone()),
        (json!([]), json!([])),
        "{r}"
    );
}

// ---------------------------------------------------------------- P4-8 fix round 2

/// xorshift64* with a Box-Muller normal, as in the core's synthetic trains.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn normal(&mut self) -> f64 {
        let u = self.next().max(1e-12);
        let v = self.next();
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

/// Funk at 96 BPM from 1 s, 48 bars: kick on 1, the "a" of 1, the "and" of 3 and the "e" of 4;
/// snare backbeat (velocity 100) and ghost notes on the sixteenths (25 to 45), 8 ms jitter.
fn funk_train(seed: u64) -> (Vec<Value>, Vec<Value>) {
    let (kick, snare) = ("x..x......x..x..", ".g.gx.g..g.gx..g");
    let beat = 60.0 / 96.0;
    let mut rng = Rng(seed);
    let mut vel = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
    let (mut k, mut s) = (vec![], vec![]);
    for bar in 0..48 {
        let t0 = 1.0 + f64::from(bar) * 4.0 * beat;
        for (out, pat) in [(&mut k, kick), (&mut s, snare)] {
            for (i, c) in pat.chars().enumerate() {
                if c == '.' {
                    continue;
                }
                let t = t0 + i as f64 * beat / 4.0 + rng.normal() * 0.008;
                let v = if c == 'g' {
                    (25.0 + 20.0 * vel.next()).round()
                } else {
                    100.0
                };
                out.push(json!({"t": t, "v": v as i64}));
            }
        }
    }
    let by_t = |a: &Value, b: &Value| a["t"].as_f64().unwrap().total_cmp(&b["t"].as_f64().unwrap());
    k.sort_by(by_t);
    s.sort_by(by_t);
    (k, s)
}

#[test]
fn detect_weighs_ghost_notes_by_their_velocity() {
    // up to round 1 the app passed no velocity: this train gave 93.25 BPM and bar 1 off
    let (svc, sid, _) = opened_long("tm_funk");
    let (k, s) = funk_train(4);
    svc.events_set(&sid, "k in", &Value::Array(k)).unwrap();
    svc.events_set(&sid, "s top", &Value::Array(s)).unwrap();
    let r = svc.tempo_map_detect(&sid, None, None, None).unwrap();
    assert!((r["start"]["bpm"].as_f64().unwrap() - 96.0).abs() < 0.1, "{r}");
    assert_eq!(r["steps"], json!([]), "{r}");
    let bar = 4.0 * 60.0 / 96.0;
    let off = (r["bar1"]["t"].as_f64().unwrap() - 1.0).rem_euclid(bar);
    assert!(off.min(bar - off) < 0.02, "{r}");
}

#[test]
fn detect_leaves_out_hits_after_the_tracks_end_and_events_edit_refuses_them() {
    let (svc, sid, _) = opened_long("tm_late");
    let (k, s, _) = sections_train(&[(48, 120.0)]);
    svc.events_set(&sid, "k in", &Value::Array(k.clone())).unwrap();
    svc.events_set(&sid, "s top", &Value::Array(s)).unwrap();
    let base = svc.tempo_map_detect(&sid, None, None, None).unwrap();
    assert_eq!(base["warnings"], json!([]), "{base}");
    // a stray kick far after the end, as `events_set` (the page) may still write it
    let mut far = k.clone();
    far.push(json!({"t": 1e7, "v": 100}));
    svc.events_set(&sid, "k in", &Value::Array(far)).unwrap();
    let t = std::time::Instant::now();
    let r = svc.tempo_map_detect(&sid, None, None, None).unwrap();
    assert!(t.elapsed().as_secs_f64() < 5.0, "{:?}", t.elapsed());
    assert_eq!(r["fit"]["late"], 1, "{r}");
    assert_eq!(r["suggestion"], base["suggestion"]);
    let w = r["warnings"].as_array().unwrap();
    assert!(
        w.len() == 1
            && w[0]
                .as_str()
                .unwrap()
                .contains("after the end of the tracks (150.00 s)"),
        "{r}"
    );
    // events_edit refuses a hit after the end: add and move
    for op in [
        json!({"op": "add", "t": 1e7}),
        json!({"op": "move", "index": 0, "t": 150.5}),
    ] {
        let e = svc.events_edit(&sid, "k in", std::slice::from_ref(&op)).unwrap_err();
        assert_eq!(e.code, 400, "{op}");
        assert!(e.message.contains("after the end of the track"), "{}", e.message);
    }
    svc.events_edit(&sid, "k in", &[json!({"op": "add", "t": 149.5})])
        .unwrap();
}

/// #468: a typed start tempo re-fits bar 1 to the hits. The command answers the supported
/// phase of the new tempo as the new bar 1 (read only; the page commits it with the tempo as
/// one `tempo_map_set` step, so one undo restores both).
#[test]
fn refit_bar1_answers_the_beat_phase_the_hits_support() {
    let (svc, sid, dir) = opened_long("tm_refit1");
    let beat = 60.0 / 92.0;
    let train: Vec<Value> = (0..24)
        .map(|k| json!({"t": (f64::from(k) * beat * 1e6).round() / 1e6, "v": 100}))
        .collect();
    svc.events_set(&sid, "k in", &Value::Array(train)).unwrap();
    svc.events_set(&sid, "s top", &Value::Array(vec![])).unwrap();
    svc.tempo_map_set(&sid, &json!({"bpm": 46.0, "bar1_s": 2.555})).unwrap();
    let dirty0 = svc.project_get(&sid).unwrap()["dirty"].as_bool().unwrap();
    let r = svc.tempo_map_refit_bar1(&sid, 92.0).unwrap();
    assert_eq!(r["changed"], false, "{r}");
    assert_eq!(r["bpm"], 92.0);
    assert_eq!(r["previous_bar1_s"], 2.555);
    assert_eq!(r["hits"], 24, "{r}");
    let bar1 = r["bar1_s"].as_f64().unwrap();
    let phase = bar1.rem_euclid(beat);
    let off = phase.min(beat - phase);
    assert!(
        off <= 0.005,
        "bar 1 at {bar1} s is {:.1} ms off a beat of the 92 BPM grid from 0 s",
        off * 1000.0
    );
    // The bar keeps its number: it moves by at most half a beat, here onto the next beat.
    assert!((bar1 - 2.555).abs() <= 0.5 * beat, "{bar1}");
    assert!(r["support"].as_f64().unwrap() > 0.0, "{r}");
    // Read only: the stored map is untouched and the dirty flag does not move until the page commits.
    assert_eq!(svc.tempo_map_get(&sid).unwrap()["map"]["bar1_s"], 2.555);
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"].as_bool().unwrap(), dirty0);
    // The page's commit: tempo and bar 1 in one step, `previous` undoes both.
    let next = json!({"bpm": 92.0, "bar1_s": bar1});
    let c = svc.tempo_map_set(&sid, &next).unwrap();
    assert_eq!(c["previous"]["bar1_s"], 2.555);
    assert_eq!(c["previous"]["bpm"], 46.0);
    let _ = dir;
}

#[test]
fn refit_bar1_refuses_a_bad_tempo_and_too_few_hits() {
    let (svc, sid, dir) = opened("tm_refit2");
    let e = svc.tempo_map_refit_bar1(&sid, 0.0).unwrap_err();
    assert_eq!(e.code, 400, "{e}");
    let e = svc.tempo_map_refit_bar1(&sid, -92.0).unwrap_err();
    assert_eq!(e.code, 400, "{e}");
    let e = svc.tempo_map_refit_bar1(&sid, 1500.0).unwrap_err();
    assert_eq!(e.code, 400, "{e}");
    // 12 kick and snare hits here: the refit answers. With fewer than 8 it refuses.
    let few: Vec<Value> = (0..4).map(|k| json!({"t": f64::from(k) * 0.5, "v": 100})).collect();
    svc.events_set(&sid, "k in", &Value::Array(few)).unwrap();
    svc.events_set(&sid, "s top", &Value::Array(vec![])).unwrap();
    let e = svc.tempo_map_refit_bar1(&sid, 92.0).unwrap_err();
    assert_eq!(e.code, 400, "{e}");
    assert!(e.message.contains("at least 8"), "{e}");
    let _ = dir;
}
