//! P4-3: the histogram data and the re-pick without the model (`crates/app/src/histogram.rs`,
//! the kept activations of `crates/app/src/activation.rs`). Through `drumspike_app::Service`,
//! no Tauri.

mod common;

use common::*;
use drumspike_app::{HISTOGRAM_COMMANDS, Service};
use serde_json::{Map, Value, json};

const SR: u32 = 48_000;

/// Loud kicks at [`ONSETS`] and soft ones (-30 dB) between them: two clusters of candidates.
fn loud_and_soft() -> Vec<f32> {
    let soft_on: Vec<f64> = ONSETS.iter().map(|t| t + 0.25).collect();
    let loud = kick_signal(&ONSETS, 3.5, SR);
    let soft = kick_signal(&soft_on, 3.5, SR);
    loud.iter()
        .zip(&soft)
        .map(|(a, b)| a + b * 10f32.powf(-30.0 / 20.0))
        .collect()
}

/// A project with a kick (loud and soft hits), a snare and a soft kick sub (-40 dB).
fn session(svc: &Service, root: &std::path::Path) -> String {
    let sub: Vec<f32> = kick_signal(&ONSETS, 3.5, SR)
        .iter()
        .map(|v| v * 10f32.powf(-40.0 / 20.0))
        .collect();
    let dir = project(
        root,
        "Hist",
        &[
            Spec {
                name: "k in",
                role: "kick",
                note: 36,
                x: loud_and_soft(),
                sr: SR,
            },
            kick_spec("s top", "snare", 38),
            Spec {
                name: "k sub",
                role: "kick",
                note: 36,
                x: sub,
                sr: SR,
            },
        ],
    );
    svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string()
}

fn events(svc: &Service, sid: &str, name: &str) -> Vec<Value> {
    svc.events_get(sid, name).unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The params of slider value `v` with `extra` keys, as the page sends them.
fn params_at(v: f64, extra: &Value) -> Map<String, Value> {
    let mut p = json!({"thresholds": drumspike_app::sensitivity::thresholds_of(v), "sensitivity": v,
                       "min_gap_ms": 20, "refine": true, "shift_samples": 0, "vel_lo_db": -40, "vel_hi_db": -6});
    for (k, x) in extra.as_object().unwrap() {
        p[k] = x.clone();
    }
    p.as_object().unwrap().clone()
}

/// A full analysis of one track with `params` (the model runs); returns the stored events and
/// the job entry.
fn reanalyze(svc: &Service, sid: &str, name: &str, params: &Map<String, Value>) -> (Vec<Value>, Value) {
    let mut all = Map::new();
    all.insert(name.into(), Value::Object(params.clone()));
    svc.analyze_start(sid, Some(vec![name.into()]), Some(&all), None)
        .unwrap();
    let st = wait_done(svc, sid);
    assert_eq!(st["tracks"][name]["state"], "done", "{st}");
    (events(svc, sid, name), st["tracks"][name].clone())
}

#[test]
fn track_histograms_give_the_levels_and_the_candidates_after_an_analysis() {
    let root = tmp("hist_data");
    let svc = service_dsp();
    let sid = session(&svc, &root);
    // before an analysis: no hits, no kept detection
    let h = svc.track_histograms(&sid, "k in", None).unwrap();
    assert_eq!(h["detection"], json!({"state": "missing"}));
    assert_eq!(h["loudness"]["levels"], json!([]));
    reanalyze(&svc, &sid, "k in", &params_at(50.0, &json!({})));
    let ev = events(&svc, &sid, "k in");
    assert_eq!(ev.len(), ONSETS.len(), "the loud hits at the default: {ev:?}");
    let h = svc.track_histograms(&sid, "k in", None).unwrap();
    let levels = h["loudness"]["levels"].as_array().unwrap();
    assert_eq!(levels.len(), ev.len());
    for l in levels {
        let l = l.as_f64().unwrap();
        assert!((-6.0..=0.0).contains(&l), "a loud kick peaks near -3 dBFS: {l}");
    }
    let d = &h["detection"];
    assert_eq!(d["state"], "ok", "{d}");
    assert_eq!(d["classes"], json!([0]));
    let cands = d["candidates"].as_array().unwrap();
    // every hit at 50 is a candidate that is on at 50 or below
    let on: Vec<&Value> = cands.iter().filter(|c| c[3].as_u64().unwrap() <= 50).collect();
    assert_eq!(on.len(), ev.len(), "{cands:?}");
    assert!(
        cands.len() > ev.len(),
        "the soft hits are candidates past 50: {cands:?}"
    );
    // the page's own events: the levels of those
    let mine = vec![json!({"t": ONSETS[0]}), json!({"t": 3.4})];
    let h2 = svc.track_histograms(&sid, "k in", Some(&mine)).unwrap();
    let l2 = h2["loudness"]["levels"].as_array().unwrap();
    assert_eq!(l2[0], levels[0].clone().as_f64().map(|_| l2[0].clone()).unwrap());
    assert!(l2[1].as_f64().unwrap() < -60.0, "{h2}");
    // a stale key (another source state): missing again (the DSP path keeps candidates.json,
    // the retired model branch kept its own activation.json)
    let act = root.join("Hist").join("Analysis").join("k in").join("candidates.json");
    let text = std::fs::read_to_string(&act).unwrap();
    std::fs::write(&act, text.replace("\"mix\":\"average\"", "\"mix\":\"ffmpeg\"")).unwrap();
    assert_eq!(
        svc.track_histograms(&sid, "k in", None).unwrap()["detection"]["state"],
        "missing"
    );
    assert_eq!(svc.track_histograms(&sid, "nope", None).unwrap_err().code, 400);
}

#[test]
fn track_repick_picks_from_the_kept_candidates_without_the_model() {
    let root = tmp("hist_dsp_repick");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = session(&svc, &root);
    // before an analysis the re-pick refuses as before
    assert_eq!(svc.track_repick(&sid, "k in", None, false).unwrap_err().code, 409);
    reanalyze(&svc, &sid, "k in", &params_at(50.0, &json!({})));
    let at_default = events(&svc, &sid, "k in");
    assert!(!at_default.is_empty(), "the detection finds the hits: {at_default:?}");
    // the detection histogram reads the kept candidates (A4)
    let d = svc.track_histograms(&sid, "k in", None).unwrap()["detection"].clone();
    assert_eq!(d["state"], "ok", "{d}");
    let cands = d["candidates"].as_array().unwrap();
    assert!(
        cands.len() >= at_default.len(),
        "the candidates cover the hits: {cands:?}"
    );
    // every hit of the analysis is a candidate on at the default or looser
    for e in &at_default {
        let t = e["t"].as_f64().unwrap();
        assert!(
            cands.iter().any(|c| (c[0].as_f64().unwrap() - t).abs() < 0.021),
            "{e} in {cands:?}"
        );
    }
    // the slider over three settings: no 409, the hit count is monotonic and the default
    // equals the analysis result
    let counts: Vec<usize> = [0.0, 50.0, 150.0]
        .iter()
        .map(|&v| {
            let r = svc
                .track_repick(&sid, "k in", Some(&params_at(v, &json!({}))), false)
                .unwrap_or_else(|e| panic!("repick at {v}: {e}"));
            r["n_events"].as_u64().unwrap() as usize
        })
        .collect();
    assert!(
        counts[0] <= counts[1] && counts[1] <= counts[2],
        "monotonic: {counts:?}"
    );
    assert_eq!(
        counts[1],
        at_default.len(),
        "the default slider reproduces the analysis"
    );
    // a stored re-pick keeps working and names the DSP detector
    let stored = svc
        .track_repick(&sid, "k in", Some(&params_at(150.0, &json!({}))), true)
        .unwrap();
    assert!(stored.get("entry").is_some(), "{stored}");
    let det = events(&svc, &sid, "k in");
    assert_eq!(det.len(), counts[2], "{det:?}");
    // the candidates survive a re-open (the store is current)
    let sid2 = svc.project_open(&root.join("Hist").display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let again = svc
        .track_repick(&sid2, "k in", Some(&params_at(150.0, &json!({}))), false)
        .unwrap();
    assert_eq!(again["n_events"], json!(counts[2] as u64), "{again}");
}

#[test]
fn track_histograms_of_a_follower_show_its_own_levels_only() {
    let root = tmp("hist_follow");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = session(&svc, &root);
    svc.analyze_start(&sid, Some(vec!["k in".into()]), None, None).unwrap();
    wait_done(&svc, &sid);
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    let h = svc.track_histograms(&sid, "k sub", None).unwrap();
    assert_eq!(h["detection"], json!({"state": "follows", "source": "k in"}));
    let levels = h["loudness"]["levels"].as_array().unwrap();
    assert_eq!(levels.len(), events(&svc, &sid, "k in").len());
    // -40 dB below the kick in: its own signal
    let own = svc.track_histograms(&sid, "k in", None).unwrap()["loudness"]["levels"][0]
        .as_f64()
        .unwrap();
    let sub = levels[0].as_f64().unwrap();
    assert!((own - sub - 40.0).abs() < 1.5, "{own} {sub}");
    assert_eq!(svc.track_repick(&sid, "k sub", None, false).unwrap_err().code, 409);
}

#[test]
fn track_velocity_set_stores_the_map_and_maps_every_hit_from_its_peak_level() {
    let root = tmp("hist_vel");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = session(&svc, &root);
    svc.analyze_start(&sid, Some(vec!["k in".into()]), None, None).unwrap();
    wait_done(&svc, &sid);
    let ev = events(&svc, &sid, "k in");
    let levels = svc.track_histograms(&sid, "k in", None).unwrap()["loudness"]["levels"].clone();
    // a ceiling below every peak: 127 for every hit; the events are not stored
    let r = svc.track_velocity_set(&sid, "k in", -60.0, -30.0, None, None).unwrap();
    let vs: Vec<i64> = r["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["v"].as_i64().unwrap())
        .collect();
    assert!(vs.iter().all(|&v| v == 127), "{vs:?} {levels}");
    assert_eq!(events(&svc, &sid, "k in"), ev, "not stored");
    let p = &svc.tracks_get(&sid).unwrap();
    let rec = p["tracks"].as_array().map_or_else(
        || p["tracks"]["k in"].clone(),
        |a| a.iter().find(|t| t["name"] == "k in").cloned().unwrap(),
    );
    assert_eq!(rec["params"]["vel_lo_db"], -60.0, "{rec}");
    assert_eq!(rec["params"]["vel_hi_db"], -30.0);
    // the page's list: its order and its other keys stay
    let mine = vec![
        json!({"t": ev[1]["t"], "id": "x1", "v": 5}),
        json!({"t": 3.45, "id": "x2"}),
    ];
    let r = svc
        .track_velocity_set(&sid, "k in", -40.0, -6.0, None, Some(&mine))
        .unwrap();
    assert_eq!(r["events"][0]["id"], "x1");
    assert_eq!(
        r["events"][0]["v"], ev[1]["v"],
        "the default map gives the analysis velocity"
    );
    assert_eq!(r["events"][1]["v"], 20, "silence");
    // a follower derives again with its new map and stores it
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    let before = events(&svc, &sid, "k sub");
    let r = svc.track_velocity_set(&sid, "k sub", -90.0, -60.0, None, None).unwrap();
    assert_eq!(r["follows"], "k in");
    let after = events(&svc, &sid, "k sub");
    assert_ne!(before, after);
    assert!(after.iter().all(|e| e["v"] == 127), "{after:?}");
    assert_eq!(
        svc.track_velocity_set(&sid, "k in", f64::NAN, -6.0, None, None)
            .unwrap_err()
            .code,
        400
    );
}

#[test]
fn every_histogram_command_has_a_test_here() {
    let src =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/histogram.rs")).unwrap();
    for c in HISTOGRAM_COMMANDS {
        assert!(
            src.contains(&format!("fn {c}_")),
            "no test named {c}_... in tests/histogram.rs"
        );
    }
}

/// Decision 4 of the controller (DRT's Loudness slider filters): the loudness cut marks the quiet detected hits;
/// they stay in the document but leave MIDI, spikes, Export All, the click and a follower; an analysis keeps the cut.
#[test]
fn track_velocity_set_with_a_cut_marks_the_quiet_hits_and_they_leave_the_output() {
    let root = tmp("hist_cut");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = session(&svc, &root);
    // the loud hits (about -3 dBFS) and the soft ones (-33 dBFS) of the kick, set as detected hits
    let set: Vec<Value> = ONSETS
        .iter()
        .flat_map(|&t| [json!({"t": t, "v": 100}), json!({"t": t + 0.25, "v": 30})])
        .collect();
    svc.events_set(&sid, "k in", &Value::Array(set)).unwrap();
    let ev = events(&svc, &sid, "k in");
    let cut = -20.0;
    let quiet = ONSETS.len();
    // the page's list: an added quiet hit is never cut
    let mut mine = ev.clone();
    mine.push(json!({"t": 3.45, "v": 20, "added": true}));
    let r = svc
        .track_velocity_set(&sid, "k in", -40.0, -6.0, Some(cut), Some(&mine))
        .unwrap();
    let marked: Vec<Value> = r["events"].as_array().unwrap().clone();
    assert_eq!(marked.iter().filter(|e| e["cut"] == true).count(), quiet);
    assert!(marked.last().unwrap().get("cut").is_none(), "an added hit is not cut");
    assert_eq!(
        svc.tracks_get(&sid).unwrap()["tracks"]["k in"]["params"]["vel_cut_db"],
        cut
    );
    svc.events_set(&sid, "k in", &Value::Array(marked.clone())).unwrap();
    // the output: MIDI, spikes, Export All
    let n_out = marked.len() - quiet;
    let midi = svc.export_midi(&sid, "k in", 36, &marked, 24000, 120.0).unwrap();
    assert_eq!(midi["n_events"], n_out);
    assert_eq!(svc.export_spikes(&sid, "k in", None).unwrap()["n_events"], n_out);
    let all = svc.export_all(&sid, None).unwrap();
    assert!(all["errors"].as_array().is_none_or(|e| e.is_empty()), "{all}");
    // a follower does not take the cut hits
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    assert_eq!(events(&svc, &sid, "k sub").len(), n_out);
    // a new analysis keeps the cut (here above the loud hits: the DSP fallback finds only those, all cut)
    let mut p = svc.tracks_get(&sid).unwrap()["tracks"]["k in"]["params"]
        .as_object()
        .unwrap()
        .clone();
    p.insert("vel_cut_db".into(), json!(-1.0));
    let mut all_p = Map::new();
    all_p.insert("k in".into(), Value::Object(p));
    svc.analyze_start(&sid, Some(vec!["k in".into()]), Some(&all_p), None)
        .unwrap();
    wait_done(&svc, &sid);
    let again = events(&svc, &sid, "k in");
    assert!(!again.is_empty() && again.iter().all(|e| e["cut"] == true), "{again:?}");
    // the cut off: every mark goes
    let r = svc.track_velocity_set(&sid, "k in", -40.0, -6.0, None, None).unwrap();
    assert!(r["events"].as_array().unwrap().iter().all(|e| e.get("cut").is_none()));
    assert!(
        svc.tracks_get(&sid).unwrap()["tracks"]["k in"]["params"]
            .get("vel_cut_db")
            .is_none()
    );
    assert_eq!(
        svc.track_velocity_set(&sid, "k in", -40.0, -6.0, Some(f64::INFINITY), None)
            .unwrap_err()
            .code,
        400
    );
}

/// Fix round: the undo and redo of a re-pick send the slider's params back.
#[test]
fn track_params_set_stores_the_page_params_and_waits_for_a_store() {
    let root = tmp("hist_params");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = session(&svc, &root);
    let p = params_at(70.0, &json!({"min_gap_ms": 33}));
    let r = svc.track_params_set(&sid, "k in", &p).unwrap();
    assert_eq!(r["params"]["sensitivity"], 70.0);
    let rec = &svc.tracks_get(&sid).unwrap()["tracks"]["k in"]["params"];
    assert_eq!(rec["sensitivity"], 70.0);
    assert_eq!(rec["min_gap_ms"], 33);
    assert_eq!(svc.track_params_set(&sid, "nope", &p).unwrap_err().code, 400);
    let g = svc.jobs().begin_store(&sid).unwrap();
    assert_eq!(svc.track_params_set(&sid, "k in", &p).unwrap_err().code, 409);
    drop(g);
}

/// Fix round: one writer at a time. A re-pick store, the velocity map and an analysis of a
/// project do not overlap (409), and the store checks the job state again.
#[test]
fn track_repick_store_velocity_set_and_analyze_start_refuse_while_another_writes() {
    let root = tmp("hist_busy");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = session(&svc, &root);
    let g = svc.jobs().begin_store(&sid).expect("free");
    let e = svc
        .track_velocity_set(&sid, "k in", -40.0, -6.0, None, None)
        .unwrap_err();
    assert_eq!(e.code, 409, "{e:?}");
    let e = svc
        .analyze_start(&sid, Some(vec!["k in".into()]), None, None)
        .unwrap_err();
    assert_eq!(e.code, 409, "{e:?}");
    assert_eq!(svc.analyze_status(&sid)["state"], "idle", "nothing queued");
    drop(g);
    svc.jobs().queue(&sid, &["k in".into()]);
    let e = svc
        .track_velocity_set(&sid, "k in", -40.0, -6.0, None, None)
        .unwrap_err();
    assert!(e.code == 409 && e.message.contains("analysis"), "{e:?}");
    assert_eq!(svc.track_repick(&sid, "k in", None, true).unwrap_err().code, 409);
    svc.jobs().clear(&sid);
    assert!(svc.track_velocity_set(&sid, "k in", -40.0, -6.0, None, None).is_ok());
}

/// R09 (#237): a rejected `track_velocity_set` leaves the document and the dirty flag alone:
/// invalid events and a source that cannot be decoded fail before anything is stored.
#[test]
fn track_velocity_set_rejects_and_changes_nothing() {
    let root = tmp("hist_r09_velocity");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = session(&svc, &root);
    let before = svc.tracks_get(&sid).unwrap()["tracks"]["k in"].clone();

    // invalid events: a hit without a numeric t (also with `apply`, which stores the hits)
    let e = svc
        .track_velocity_set(&sid, "k in", -40.0, -6.0, None, Some(&[json!({"nope": 1})]))
        .unwrap_err();
    assert_eq!(e.code, 400, "{e:?}");
    let e = svc
        .track_velocity_set_apply(&sid, "k in", -40.0, -6.0, None, Some(&[json!(1)]), true)
        .unwrap_err();
    assert_eq!(e.code, 400, "{e:?}");

    // a failing decode: the source file is gone, so the track is offline
    std::fs::remove_file(root.join("daw").join("k in.wav")).unwrap();
    let e = svc
        .track_velocity_set(&sid, "k in", -40.0, -6.0, None, None)
        .unwrap_err();
    assert_eq!(e.code, 409, "{e:?}");

    // the resolve status of the removed file is derived state; the document is what counts
    let doc_view = |v: &Value| {
        let mut m = v.as_object().unwrap().clone();
        m.shift_remove("status");
        m.shift_remove("offline");
        Value::Object(m)
    };
    let after = svc.tracks_get(&sid).unwrap()["tracks"]["k in"].clone();
    assert_eq!(doc_view(&after), doc_view(&before), "the track record is unchanged");
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], json!(false), "not dirty");
}

/// R09: a follower whose own audio is gone refuses and keeps its params (the params go in
/// before the derive reads the new map, and go back when the derive fails).
#[test]
fn track_velocity_set_restores_a_follower_when_the_derive_fails() {
    let root = tmp("hist_r09_follow");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = session(&svc, &root);
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    svc.project_save(&sid).unwrap();
    std::fs::remove_file(root.join("daw").join("k sub.wav")).unwrap();
    let before = svc.tracks_get(&sid).unwrap()["tracks"]["k sub"].clone();

    let e = svc
        .track_velocity_set(&sid, "k sub", -30.0, -3.0, None, None)
        .unwrap_err();
    assert_eq!(e.code, 409, "{e:?}");
    let after = svc.tracks_get(&sid).unwrap()["tracks"]["k sub"].clone();
    assert_eq!(after, before, "the track record is unchanged");
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], json!(false), "not dirty");
}
