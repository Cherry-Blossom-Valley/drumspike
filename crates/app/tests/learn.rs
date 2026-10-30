//! P4-6a: Learn marks (`crates/app/src/learn.rs`) through `drumspike_app::Service`, no Tauri. The
//! candidate test needs `DRUMSPIKE_MODEL` (CI sets it).

mod common;

use common::*;
use drumspike_app::{LEARN_COMMANDS, Service};
use drumspike_core::project::load_project;
use serde_json::{Value, json};
use std::sync::{Arc, Barrier};

/// A project with a kick in, a kick sub and a snare; the kick in holds the hits `hits`.
fn session(tag: &str, hits: &[f64]) -> (Service, String, std::path::PathBuf) {
    let root = tmp(tag);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(
        &root,
        "Learn",
        &[
            kick_spec("k in", "kick", 36),
            kick_spec("k sub", "kick", 36),
            kick_spec("s top", "snare", 38),
        ],
    );
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let ev: Vec<Value> = hits.iter().map(|t| json!({"t": t, "v": 90})).collect();
    svc.events_set(&sid, "k in", &Value::Array(ev)).unwrap();
    (svc, sid, dir)
}

fn learn_of(svc: &Service, sid: &str, name: &str) -> Value {
    svc.tracks_get(sid).unwrap()["tracks"][name]["learn"].clone()
}

#[test]
fn a_range_takes_its_active_hits_and_a_tight_range_takes_one() {
    let (svc, sid, _) = session("learn_hits", &[0.5, 1.0, 1.5, 3.0]);
    let wide = svc
        .track_learn_mark(&sid, "k in", "this", Some((0.4, 1.6)), None)
        .unwrap();
    assert_eq!(wide["marked"], true);
    assert_eq!(wide["from"], "hits");
    assert_eq!(wide["mark"]["at"], json!([0.5, 1.0, 1.5]));
    assert_eq!(
        (wide["mark"]["t0"].clone(), wide["mark"]["t1"].clone()),
        (json!(0.4), json!(1.6))
    );
    assert_eq!(wide["mark"]["id"], "m1");
    assert_eq!(wide["counts"], json!({"this": 3, "not": 0}));
    let tight = svc
        .track_learn_mark(&sid, "k in", "not", Some((2.95, 3.05)), None)
        .unwrap();
    assert_eq!(tight["mark"]["at"], json!([3.0]));
    assert_eq!(tight["counts"], json!({"this": 3, "not": 1}));
    let l = learn_of(&svc, &sid, "k in");
    assert_eq!(l["on"], true);
    assert_eq!(l["method"], "patch-2");
    assert_eq!(l["marks"].as_array().unwrap().len(), 2);
    // a reversed range is the same range
    let rev = svc
        .track_learn_mark(&sid, "k in", "this", Some((3.05, 2.95)), None)
        .unwrap();
    assert_eq!(rev["flipped"], 1, "the hit at 3.0 changes kind");
    assert_eq!(rev["mark"]["t0"], 2.95);
}

#[test]
fn the_hitpoint_shift_comes_off_the_example_times() {
    let (svc, sid, _) = session("learn_shift", &[0.5]);
    svc.track_params_set(&sid, "k in", json!({"shift_samples": 480}).as_object().unwrap())
        .unwrap();
    let r = svc
        .track_learn_mark(&sid, "k in", "this", Some((0.4, 0.6)), None)
        .unwrap();
    let at = r["mark"]["at"][0].as_f64().unwrap();
    assert!((at - 0.49).abs() < 1e-9, "10 ms of shift off: {at}");
}

#[test]
fn a_range_with_no_hit_takes_the_strongest_rise_else_nothing() {
    let (svc, sid, _) = session("learn_rise", &[]);
    // the kick at 0.75 s is in the audio, not in the hits (and the DSP fallback keeps no candidates)
    let r = svc
        .track_learn_mark(&sid, "k in", "this", Some((0.7, 0.85)), None)
        .unwrap();
    assert_eq!(r["marked"], true, "{r}");
    assert_eq!(r["from"], "rise");
    let at = r["mark"]["at"][0].as_f64().unwrap();
    assert!((0.745..0.8).contains(&at), "{at}");
    // P4-38b: a range of silence (the audio ends at 3.5 s) is no longer refused: K adds a hit at
    // the range centre (#81), flagged learned (the hand-add path), and the example is stored
    let n = svc
        .track_learn_mark(&sid, "k in", "this", Some((3.0, 3.4)), None)
        .unwrap();
    assert_eq!(n["marked"], true, "{n}");
    assert_eq!(n["from"], "hand");
    assert_eq!(n["added_hit"], json!(3.2));
    assert_eq!(n["k_rise"]["at"], "centre", "{n}");
    assert_eq!(learn_of(&svc, &sid, "k in")["marks"].as_array().unwrap().len(), 2);
    let evs = svc.events_get(&sid, "k in").unwrap()["events"]
        .as_array()
        .expect("events")
        .clone();
    let added = evs
        .iter()
        .find(|e| (e["t"].as_f64().unwrap() - 3.2).abs() < 1e-9)
        .expect("the added hit is in the document");
    assert_eq!(added["learned"], json!(true));
    assert_eq!(added["added"], json!(true));
    // Shift+K on a silent range keeps today's answer (a different range: 3.0-3.4 holds the added hit)
    let n = svc
        .track_learn_mark(&sid, "k in", "not", Some((3.45, 3.49)), None)
        .unwrap();
    assert_eq!(n["marked"], false);
    assert_eq!(n["reason"], "no hit in the selection");
}

#[test]
fn a_selected_hit_is_one_example_and_a_missing_hit_is_refused() {
    let (svc, sid, _) = session("learn_hit", &[0.5, 1.0]);
    let r = svc.track_learn_mark(&sid, "k in", "not", None, Some(1.002)).unwrap();
    assert_eq!(
        (r["from"].clone(), r["mark"]["at"].clone()),
        (json!("hit"), json!([1.0]))
    );
    assert_eq!(r["mark"]["t0"], r["mark"]["t1"]);
    // P4-6c: the first not example turns the filter on (threshold following the examples); the DSP
    // fallback keeps no activations, so the re-pick that applies it is refused and says why
    assert_eq!(
        (
            r["learn"]["on"].clone(),
            r["learn"]["auto"].clone(),
            r["filter_on"].clone()
        ),
        (json!(true), json!(true), json!(true))
    );
    assert!(r["filter_error"].is_string() && r.get("filter").is_none(), "{r}");
    // P4-38b: K at a time with neither a hit nor a candidate adds the hit at the time (the
    // hand-add path); Shift+K keeps today's refusal
    let k = svc.track_learn_mark(&sid, "k in", "this", None, Some(2.0)).unwrap();
    assert_eq!(k["marked"], json!(true), "{k}");
    assert_eq!(k["from"], "hand");
    assert_eq!(k["added_hit"], json!(2.0));
    assert_eq!(
        svc.track_learn_mark(&sid, "k in", "not", None, Some(2.5))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        svc.track_learn_mark(&sid, "k in", "not", None, None).unwrap_err().code,
        400
    );
    assert_eq!(
        svc.track_learn_mark(&sid, "k in", "this", Some((0.0, 1.0)), Some(1.0))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        svc.track_learn_mark(&sid, "k in", "maybe", None, Some(1.0))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        svc.track_learn_mark(&sid, "nope", "this", None, Some(1.0))
            .unwrap_err()
            .code,
        400
    );
}

#[test]
fn the_last_kind_wins_and_the_same_kind_again_changes_nothing() {
    let (svc, sid, _) = session("learn_last", &[0.5, 1.0]);
    svc.track_learn_mark(&sid, "k in", "this", Some((0.4, 1.1)), None)
        .unwrap();
    let r = svc.track_learn_mark(&sid, "k in", "not", None, Some(1.0)).unwrap();
    assert_eq!(
        (r["flipped"].clone(), r["counts"].clone()),
        (json!(1), json!({"this": 1, "not": 1}))
    );
    let again = svc.track_learn_mark(&sid, "k in", "not", None, Some(1.0)).unwrap();
    assert_eq!(
        (again["marked"].clone(), again["reason"].clone()),
        (json!(false), json!("already marked"))
    );
    assert_eq!(again["skipped"], 1);
}

#[test]
fn remove_flip_forget_and_set_change_the_marks_and_the_document() {
    let (svc, sid, _) = session("learn_ops", &[0.5, 1.0, 1.5]);
    svc.track_learn_mark(&sid, "k in", "this", Some((0.4, 1.1)), None)
        .unwrap();
    svc.track_learn_mark(&sid, "k in", "not", None, Some(1.5)).unwrap();
    let r = svc.track_learn_remove(&sid, "k in", "m1", Some(0.5)).unwrap();
    assert_eq!(r["removed"], 1);
    assert_eq!(r["learn"]["marks"][0]["at"], json!([1.0]));
    let r = svc.track_learn_flip(&sid, "k in", "m2").unwrap();
    assert_eq!(r["kind"], "this");
    let saved = learn_of(&svc, &sid, "k in");
    assert_eq!(svc.track_learn_remove(&sid, "k in", "m9", None).unwrap_err().code, 400);
    let r = svc.track_learn_forget(&sid, "k in").unwrap();
    assert_eq!((r["forgotten"].clone(), r["examples"].clone()), (json!(2), json!(2)));
    // Forget removes the examples only: the settings stay for the scorer of P4-6b
    let kept = learn_of(&svc, &sid, "k in");
    assert_eq!(kept["marks"], json!([]));
    assert_eq!(
        (
            kept["on"].clone(),
            kept["threshold"].clone(),
            kept["auto"].clone(),
            kept["method"].clone()
        ),
        (
            saved["on"].clone(),
            saved["threshold"].clone(),
            saved["auto"].clone(),
            saved["method"].clone()
        )
    );
    assert_eq!(svc.track_learn_forget(&sid, "k in").unwrap()["forgotten"], 0);
    assert_eq!(svc.track_learn_forget(&sid, "s top").unwrap()["forgotten"], 0);
    assert_eq!(
        learn_of(&svc, &sid, "s top"),
        Value::Null,
        "a track without learn gets none from Forget"
    );
    // undo: the whole object goes back
    svc.track_learn_set(&sid, "k in", &saved).unwrap();
    assert_eq!(learn_of(&svc, &sid, "k in"), saved);
    assert_eq!(
        svc.track_learn_set(&sid, "k in", &json!({"marks": 5}))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(svc.track_learn_set(&sid, "k in", &json!(3)).unwrap_err().code, 400);
    svc.track_learn_set(&sid, "k in", &Value::Null).unwrap();
    assert_eq!(learn_of(&svc, &sid, "k in"), Value::Null);
}

#[test]
fn a_follower_marks_its_source() {
    let (svc, sid, _) = session("learn_follow", &[0.5, 1.0]);
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    let r = svc
        .track_learn_mark(&sid, "k sub", "this", Some((0.4, 1.1)), None)
        .unwrap();
    assert_eq!((r["routed"].clone(), r["track"].clone()), (json!(true), json!("k in")));
    assert_eq!(r["mark"]["at"], json!([0.5, 1.0]));
    assert_eq!(learn_of(&svc, &sid, "k sub"), Value::Null);
    assert_eq!(learn_of(&svc, &sid, "k in")["marks"][0]["id"], "m1");
    let f = svc.track_learn_flip(&sid, "k sub", "m1").unwrap();
    assert_eq!((f["routed"].clone(), f["kind"].clone()), (json!(true), json!("not")));
}

#[test]
fn the_marks_survive_save_reopen_and_a_new_analysis() {
    let (svc, sid, dir) = session("learn_keep", &[0.5, 1.0]);
    svc.track_learn_mark(&sid, "k in", "this", Some((0.4, 1.1)), None)
        .unwrap();
    let want = learn_of(&svc, &sid, "k in");
    svc.analyze_start(&sid, Some(vec!["k in".into()]), None, None).unwrap();
    wait_done(&svc, &sid);
    assert_eq!(learn_of(&svc, &sid, "k in"), want, "Re-analyze keeps learn");
    svc.project_save(&sid).unwrap();
    // the file: the key is there, format 1
    let p = load_project(&dir).unwrap();
    assert_eq!(p.doc.format, 1);
    assert_eq!(p.doc.track("k in").unwrap().extra["learn"], want);
    svc.project_close(&sid, true).unwrap();
    let sid2 = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(learn_of(&svc, &sid2, "k in"), want);
    assert_eq!(learn_of(&svc, &sid2, "s top"), Value::Null);
}

#[test]
fn a_kind_holds_64_examples() {
    let hits: Vec<f64> = (0..70).map(|i| 0.05 + f64::from(i) * 0.04).collect();
    let (svc, sid, _) = session("learn_limit", &hits);
    svc.track_learn_mark(&sid, "k in", "this", Some((0.0, 2.7)), None)
        .unwrap_err();
    let before = learn_of(&svc, &sid, "k in");
    assert_eq!(before, Value::Null, "a refused mark changes nothing");
    let r = svc
        .track_learn_mark(&sid, "k in", "this", Some((0.0, 2.4)), None)
        .unwrap();
    assert_eq!(r["counts"]["this"], 59);
}

#[test]
fn a_candidate_below_the_threshold_is_the_example_of_an_empty_range() {
    let root = tmp("learn_cand");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(&root, "Cand", &[kick_spec("k in", "kick", 36)]);
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    svc.events_set(&sid, "k in", &json!([])).unwrap();
    let r = svc
        .track_learn_mark(&sid, "k in", "this", Some((0.2, 0.3)), None)
        .unwrap();
    assert_eq!(
        (r["marked"].clone(), r["from"].clone()),
        (json!(true), json!("candidate")),
        "{r}"
    );
    let at = r["mark"]["at"][0].as_f64().unwrap();
    assert!((0.2..=0.3).contains(&at), "{at}");
    let h = svc.track_learn_mark(&sid, "k in", "not", None, Some(0.75)).unwrap();
    assert_eq!(h["from"], "candidate", "no stored hit there, a candidate within 30 ms");
}

#[test]
fn forget_at_takes_the_examples_of_one_hit_or_a_range_and_nothing_else() {
    let (svc, sid, _) = session("learn_forget_at", &[0.5, 1.0, 1.5, 2.0]);
    svc.track_learn_mark(&sid, "k in", "this", Some((0.4, 1.1)), None)
        .unwrap();
    svc.track_learn_mark(&sid, "k in", "not", None, Some(1.5)).unwrap();
    let before = learn_of(&svc, &sid, "k in");
    // a hit with no example: nothing changes, the answer says 0
    let r = svc.track_learn_forget_at(&sid, "k in", None, Some(2.0)).unwrap();
    assert_eq!(r["removed"], 0);
    assert_eq!(learn_of(&svc, &sid, "k in"), before);
    // 3 ms away is the same hit, 20 ms away is not
    let r = svc.track_learn_forget_at(&sid, "k in", None, Some(0.52)).unwrap();
    assert_eq!(r["removed"], 0, "20 ms is outside the hit window");
    let r = svc.track_learn_forget_at(&sid, "k in", None, Some(0.503)).unwrap();
    assert_eq!(r["removed"], 1);
    assert_eq!(r["learn"]["marks"][0]["at"], json!([1.0]));
    // a range takes both kinds; an emptied mark goes
    let r = svc.track_learn_forget_at(&sid, "k in", Some((0.9, 1.6)), None).unwrap();
    assert_eq!(r["removed"], 2);
    assert_eq!(r["learn"]["marks"], json!([]));
    assert_eq!(
        svc.track_learn_forget_at(&sid, "k in", None, None).unwrap_err().code,
        400
    );
    assert_eq!(
        svc.track_learn_forget_at(&sid, "k in", Some((0.0, 1.0)), Some(0.5))
            .unwrap_err()
            .code,
        400
    );
}

#[test]
fn the_commands_list_matches_the_dispatcher() {
    let (svc, sid, _) = session("learn_list", &[0.5]);
    for c in LEARN_COMMANDS {
        let r = drumspike_app::call::run(
            &svc,
            &std::sync::Arc::new(Default::default()),
            c,
            &json!({"sid": sid, "name": "k in"}),
        );
        assert!(r.is_some(), "{c}");
    }
}

#[test]
fn a_single_hit_mark_takes_the_hitpoint_shift_off_and_a_mark_makes_the_project_dirty() {
    let (svc, sid, _) = session("learn_hit_shift", &[0.5]);
    svc.track_params_set(&sid, "k in", json!({"shift_samples": 480}).as_object().unwrap())
        .unwrap();
    svc.project_save(&sid).unwrap();
    assert_eq!(svc.tracks_get(&sid).unwrap()["dirty"], false);
    let r = svc.track_learn_mark(&sid, "k in", "this", None, Some(0.5)).unwrap();
    let at = r["mark"]["at"][0].as_f64().unwrap();
    assert!((at - 0.49).abs() < 1e-9, "10 ms of shift off a hit: {at}");
    assert_eq!(
        svc.tracks_get(&sid).unwrap()["dirty"],
        true,
        "a mark makes the project dirty"
    );
    svc.project_save(&sid).unwrap();
    svc.track_learn_flip(&sid, "k in", "m1").unwrap();
    assert_eq!(svc.tracks_get(&sid).unwrap()["dirty"], true, "so does a flip");
    svc.project_save(&sid).unwrap();
    svc.track_learn_forget(&sid, "k in").unwrap();
    assert_eq!(svc.tracks_get(&sid).unwrap()["dirty"], true, "and Forget");
}

/// P4-6b stage A: `track_learn_scores` without kept activations, on a follower, and (with
/// `DRUMSPIKE_MODEL`) on an analysed track: one row per candidate, the examples' scores, the
/// automatic and the stored threshold, and the cache `Analysis/<track>/learn.bin` built once,
/// read after, rebuilt for another method or a changed key.
#[test]
fn track_learn_scores_scores_the_candidates_and_keeps_a_cache() {
    let (svc, sid, _) = session("learn_scores_dsp", &[0.5]);
    let r = svc.track_learn_scores(&sid, "k in").unwrap();
    assert_eq!(
        (r["state"].clone(), r["routed"].clone()),
        (json!("missing"), json!(false))
    );
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    let r = svc.track_learn_scores(&sid, "k sub").unwrap();
    assert_eq!(
        (r["routed"].clone(), r["track"].clone()),
        (json!(true), json!("k in")),
        "a follower scores its source"
    );

    let root = tmp("learn_scores");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(&root, "Scores", &[kick_spec("k in", "kick", 36)]);
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let r = svc.track_learn_scores(&sid, "k in").unwrap();
    assert_eq!(r["state"], "missing", "no analysis yet");
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    // no example yet: every score is 0.5 and the threshold the default
    let r = svc.track_learn_scores(&sid, "k in").unwrap();
    assert_eq!(r["state"], "ok", "{r}");
    let n = svc.track_histograms(&sid, "k in", None).unwrap()["detection"]["candidates"]
        .as_array()
        .unwrap()
        .len();
    assert!(n >= ONSETS.len(), "{n}");
    assert_eq!(r["candidates"].as_array().unwrap().len(), n, "one row per candidate");
    assert_eq!(r["shape"].as_array().unwrap().len(), n, "one shape pair per candidate");
    assert!(r["candidates"].as_array().unwrap().iter().all(|c| c[2] == 0.5));
    assert_eq!((r["threshold"].clone(), r["auto"].clone()), (json!(0.5), json!(0.5)));
    let analysis = dir.join("Analysis").join("k in");
    assert_eq!(r["cache"], "built");
    assert!(analysis.join("learn.bin").is_file() && analysis.join("learn.json").is_file());
    assert_eq!(svc.track_learn_scores(&sid, "k in").unwrap()["cache"], "hit");
    // two examples on hits: their candidates score at the top
    for t in [ONSETS[1], ONSETS[3]] {
        svc.track_learn_mark(&sid, "k in", "this", None, Some(t)).unwrap();
    }
    let r = svc.track_learn_scores(&sid, "k in").unwrap();
    assert_eq!(r["cache"], "hit", "a mark does not change the features");
    let this = r["examples"]["this"].as_array().unwrap();
    assert_eq!(this.len(), 2, "{r}");
    let top = r["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c[2].as_f64().unwrap())
        .fold(0.0, f64::max);
    let at_hit = r["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| (c[0].as_f64().unwrap() - ONSETS[1]).abs() <= 0.03)
        .map(|c| c[2].as_f64().unwrap())
        .fold(0.0, f64::max);
    assert!(at_hit > 0.5 && (at_hit - top).abs() < 0.05, "{at_hit} {top}");
    // the stored threshold, or the automatic one with `auto`
    let mut learn = learn_of(&svc, &sid, "k in");
    assert_eq!(learn["auto"], true, "P4-6c: a new learn object follows the examples");
    learn["threshold"] = json!(0.7);
    learn["auto"] = json!(false);
    svc.track_learn_set(&sid, "k in", &learn).unwrap();
    assert_eq!(svc.track_learn_scores(&sid, "k in").unwrap()["threshold"], 0.7);
    learn["auto"] = json!(true);
    svc.track_learn_set(&sid, "k in", &learn).unwrap();
    let r = svc.track_learn_scores(&sid, "k in").unwrap();
    assert_eq!(r["threshold"], r["auto"]);
    // another method: another key, the cache is built again; a changed key file too
    // (P4-38d: the Learn pick of the step itself builds it, the filter off: the Add rule runs)
    let key_before = std::fs::read_to_string(analysis.join("learn.json")).unwrap();
    learn["method"] = json!("mfcc-2");
    svc.track_learn_set(&sid, "k in", &learn).unwrap();
    let r = svc.track_learn_scores(&sid, "k in").unwrap();
    assert_eq!(
        (r["method"].clone(), r["cache"].clone()),
        (json!("mfcc-2"), json!("hit"))
    );
    assert_ne!(
        std::fs::read_to_string(analysis.join("learn.json")).unwrap(),
        key_before,
        "another key"
    );
    std::fs::write(analysis.join("learn.json"), "{}").unwrap();
    assert_eq!(svc.track_learn_scores(&sid, "k in").unwrap()["cache"], "built");
    // an unknown method scores with the default recipe
    learn["method"] = json!("nope-9");
    svc.track_learn_set(&sid, "k in", &learn).unwrap();
    assert_eq!(svc.track_learn_scores(&sid, "k in").unwrap()["method"], "patch-2");
}

/// The DSP service of the filter tests: a kick track analysed by the DSP detector (#293: no model).
fn model_session(_test: &str, tag: &str) -> Option<(Service, String, std::path::PathBuf)> {
    let root = tmp(tag);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let dir = project(&root, "Filter", &[kick_spec("k in", "kick", 36)]);
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    Some((svc, sid, dir))
}

fn events(svc: &Service, sid: &str) -> Vec<Value> {
    svc.events_get(sid, "k in").unwrap()["events"]
        .as_array()
        .unwrap()
        .clone()
}

fn times(ev: &[Value]) -> Vec<f64> {
    ev.iter().map(|e| e["t"].as_f64().unwrap()).collect()
}

fn filtered(ev: &[Value]) -> Vec<f64> {
    ev.iter()
        .filter(|e| e["filtered"] == true)
        .map(|e| e["t"].as_f64().unwrap())
        .collect()
}

/// P4-6c: the Learn filter through the commands. The first Shift+K turns the filter on and the hits
/// follow at once (a stored re-pick): the marked hit is a grey hit (`filtered`), the hit times stay
/// those of the filter off; a K changes nothing while the filter is off; off, Forget and undo
/// (`track_learn_set`) give the unfiltered hits back; Re-analyze with Learn stores what the re-pick
/// with Learn stored; an edited hit is never filtered; a filtered hit leaves the MIDI.
#[test]
fn the_learn_filter_marks_hits_out_and_follows_every_learn_change() {
    let Some((svc, sid, dir)) = model_session(
        "the_learn_filter_marks_hits_out_and_follows_every_learn_change",
        "learn_filter",
    ) else {
        return;
    };
    let plain = events(&svc, &sid);
    assert!(
        plain.len() >= ONSETS.len() - 1 && filtered(&plain).is_empty(),
        "{plain:?}"
    );
    // K: the filter stays off and takes no hit out (P4-38d: the Add rule runs, every hit stays)
    let r = svc
        .track_learn_mark(&sid, "k in", "this", None, Some(plain[0]["t"].as_f64().unwrap()))
        .unwrap();
    assert_eq!(
        (r["filter_on"].clone(), r["learn"]["on"].clone()),
        (json!(false), json!(false))
    );
    let after_k = events(&svc, &sid);
    assert!(filtered(&after_k).is_empty(), "{after_k:?}");
    assert!(times(&plain).iter().all(|t| times(&after_k).contains(t)), "{after_k:?}");
    let plain = after_k;
    // the first Shift+K on the third hit: on, the hit goes grey, the times stay
    let t3 = plain[2]["t"].as_f64().unwrap();
    let r = svc.track_learn_mark(&sid, "k in", "not", None, Some(t3)).unwrap();
    assert_eq!(r["filter_on"], true, "{r}");
    let after = events(&svc, &sid);
    assert_eq!(
        r["filter"]["events"].as_array().unwrap(),
        &after,
        "the answer holds the stored hits"
    );
    assert_eq!(
        times(&after),
        times(&plain),
        "the filter marks hits, it never moves or drops one"
    );
    assert!(filtered(&after).iter().any(|t| (t - t3).abs() < 1e-9), "{after:?}");
    assert!(
        !filtered(&after)
            .iter()
            .any(|t| (t - plain[0]["t"].as_f64().unwrap()).abs() < 1e-9),
        "the this example stays"
    );
    let scores = svc.track_learn_scores(&sid, "k in").unwrap();
    assert_eq!(
        scores["threshold"], scores["suggest"],
        "auto: the threshold follows the examples"
    );
    // the MIDI has no grey hit
    let midi = svc.export_midi_stored(&sid, "k in", None, None, None, None).unwrap();
    assert_eq!(
        midi["n_events"].as_u64().map(|n| n as usize),
        Some(after.len() - filtered(&after).len()),
        "{midi}"
    );
    // Re-analyze with Learn = the re-pick with Learn
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    assert_eq!(events(&svc, &sid), after, "Re-analyze stores what the re-pick stored");
    // a threshold over every score (auto off): every hit goes grey but the this example
    let mut learn = learn_of(&svc, &sid, "k in");
    learn["auto"] = json!(false);
    learn["threshold"] = json!(1.0);
    let r = svc.track_learn_set(&sid, "k in", &learn).unwrap();
    let all = r["filter"]["events"].as_array().unwrap().clone();
    assert_eq!(filtered(&all).len(), all.len() - 1, "{all:?}");
    // an edited hit is never filtered: move the last hit by 1 ms
    let last = all.len() - 1;
    let to = all[last]["t"].as_f64().unwrap() + 0.001;
    svc.events_edit(&sid, "k in", &[json!({"op": "move", "index": last, "t": to})])
        .unwrap();
    let r = svc.track_learn_set(&sid, "k in", &learn).unwrap();
    let ev = r["filter"]["events"].as_array().unwrap().clone();
    let moved = ev
        .iter()
        .find(|e| e.get("orig").is_some())
        .expect("the moved hit stays");
    assert!(moved.get("filtered").is_none(), "{moved}");
    // off: the hits of the filter off (the moved one stays moved)
    learn["on"] = json!(false);
    let r = svc.track_learn_set(&sid, "k in", &learn).unwrap();
    assert!(filtered(r["filter"]["events"].as_array().unwrap()).is_empty(), "{r}");
    // on again, then Forget: no example, the filter runs no more
    learn["on"] = json!(true);
    svc.track_learn_set(&sid, "k in", &learn).unwrap();
    assert!(!filtered(&events(&svc, &sid)).is_empty());
    let r = svc.track_learn_forget(&sid, "k in").unwrap();
    assert!(filtered(r["filter"]["events"].as_array().unwrap()).is_empty(), "{r}");
    assert_eq!(learn_of(&svc, &sid, "k in")["on"], true, "Forget keeps the setting");
    let _ = dir;
}

/// #293 (B8): Re-analyze with the filter on keeps the marks the filter gives (the same out-set as the stored re-pick).
#[test]
fn a_re_analyze_with_the_filter_on_keeps_the_marks_of_the_re_pick() {
    let Some((svc, sid, _)) = model_session(
        "a_re_analyze_with_the_filter_on_keeps_the_marks_of_the_re_pick",
        "learn_reanalyze_on",
    ) else {
        return;
    };
    let plain = events(&svc, &sid);
    let t3 = plain[2]["t"].as_f64().unwrap();
    let r = svc.track_learn_mark(&sid, "k in", "not", None, Some(t3)).unwrap();
    assert_eq!(r["filter_on"], true, "{r}");
    let picked = events(&svc, &sid);
    let out = filtered(&picked);
    assert!(!out.is_empty(), "the filter marks a hit: {picked:?}");
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    let again = events(&svc, &sid);
    assert_eq!(filtered(&again), out, "Re-analyze keeps the filter's out-set");
    assert_eq!(times(&again), times(&picked), "the filter never moves or drops a hit");
}

/// #293 (B8): Re-analyze with the filter off changes no hit: the hits and their marks stay as they were.
#[test]
fn a_re_analyze_with_the_filter_off_changes_no_hit() {
    let Some((svc, sid, _)) = model_session("a_re_analyze_with_the_filter_off_changes_no_hit", "learn_reanalyze_off")
    else {
        return;
    };
    let plain = events(&svc, &sid);
    // a K keeps the filter off (the Add rule runs, every hit stays)
    let r = svc
        .track_learn_mark(&sid, "k in", "this", None, Some(plain[0]["t"].as_f64().unwrap()))
        .unwrap();
    assert_eq!(r["filter_on"], false, "{r}");
    let before = events(&svc, &sid);
    assert!(filtered(&before).is_empty(), "{before:?}");
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    let after = events(&svc, &sid);
    assert!(filtered(&after).is_empty(), "{after:?}");
    assert_eq!(times(&after), times(&before), "Re-analyze changes no hit");
}

/// P4-6c fix round: a Learn change keeps the hand edits (a deleted hit stays deleted, a velocity edit stays), and
/// `track_learn_set` with `events` restores a step's snapshot exactly (the page's undo and redo).
#[test]
fn a_learn_change_keeps_hand_deletions_and_velocities_and_undo_restores_the_snapshot() {
    let Some((svc, sid, _)) = model_session(
        "a_learn_change_keeps_hand_deletions_and_velocities_and_undo_restores_the_snapshot",
        "learn_edits",
    ) else {
        return;
    };
    let plain = events(&svc, &sid);
    let t = |i: usize| plain[i]["t"].as_f64().unwrap();
    svc.track_learn_mark(&sid, "k in", "not", None, Some(t(2))).unwrap();
    svc.events_edit(
        &sid,
        "k in",
        &[
            json!({"op": "delete", "index": 0}),
            json!({"op": "velocity", "index": 1, "v": 127}),
        ],
    )
    .unwrap();
    let edited = events(&svc, &sid);
    let learn_before = learn_of(&svc, &sid, "k in");
    let r = svc.track_learn_mark(&sid, "k in", "this", None, Some(t(3))).unwrap();
    let after = events(&svc, &sid);
    assert_eq!(r["filter"]["events"].as_array().unwrap(), &after);
    assert!(
        !times(&after).iter().any(|x| (x - t(0)).abs() < 0.02),
        "the deleted hit stays deleted: {after:?}"
    );
    let one = after
        .iter()
        .find(|e| (e["t"].as_f64().unwrap() - t(1)).abs() < 1e-9)
        .unwrap();
    assert_eq!(one["v"], 127, "the velocity edit stays");
    assert_eq!(times(&after), times(&edited), "the same hits as before the mark");
    // undo: the learn and the hits of the snapshot, exactly
    let r = svc
        .track_learn_set_events(&sid, "k in", &learn_before, Some(&Value::Array(edited.clone())))
        .unwrap();
    assert_eq!(events(&svc, &sid), edited);
    assert_eq!(learn_of(&svc, &sid, "k in"), learn_before);
    assert_eq!(r["filter"]["events"].as_array().unwrap(), &edited);
    // a snapshot that is not a list is refused
    assert!(
        svc.track_learn_set_events(&sid, "k in", &learn_before, Some(&json!({"t": 1})))
            .is_err()
    );
}

/// P4-35: every Learn change answers the hits the filter takes out before and after (`learn_out_before`,
/// `learn_out_after`, `hits`, `hits_before`); the counts equal the stored hits' `filtered` marks, the next
/// step's "before" is the last step's "after", and `track_learn_set` with `events` (undo) reports them too.
#[test]
fn a_learn_change_answers_the_hits_the_filter_takes_out_before_and_after() {
    let Some((svc, sid, _)) = model_session(
        "a_learn_change_answers_the_hits_the_filter_takes_out_before_and_after",
        "learn_guard",
    ) else {
        return;
    };
    let plain = events(&svc, &sid);
    let t = |i: usize| plain[i]["t"].as_f64().unwrap();
    let n = |r: &Value, k: &str| r[k].as_u64().unwrap_or_else(|| panic!("no {k} in {r}")) as usize;
    let check = |r: &Value, before: usize| -> usize {
        let ev = events(&svc, &sid);
        let out = filtered(&ev).len();
        assert_eq!(n(r, "learn_out_before"), before, "{r}");
        assert_eq!(
            n(r, "learn_out_after"),
            out,
            "the answer counts the stored grey hits: {r}"
        );
        assert_eq!(n(r, "hits"), ev.len(), "{r}");
        out
    };
    // K with the filter off: nothing changes, before = after = 0
    let r = svc.track_learn_mark(&sid, "k in", "this", None, Some(t(0))).unwrap();
    let mut out = check(&r, 0);
    assert_eq!(out, 0);
    // Shift+K turns the filter on: at least the marked hit goes
    let r = svc.track_learn_mark(&sid, "k in", "not", None, Some(t(2))).unwrap();
    out = check(&r, out);
    assert!(out >= 1, "{r}");
    // flip, remove and Forget each report from the last state
    let id = r["mark"]["id"].as_str().unwrap().to_string();
    let r = svc.track_learn_flip(&sid, "k in", &id).unwrap();
    out = check(&r, out);
    let r = svc.track_learn_flip(&sid, "k in", &id).unwrap();
    out = check(&r, out);
    assert!(out >= 1);
    let r = svc.track_learn_forget(&sid, "k in").unwrap();
    let after_forget = check(&r, out);
    assert_eq!(after_forget, 0, "Forget gives the hits back: {r}");
    assert!(n(&r, "learn_out_before") >= 1, "{r}");
    // undo: the stored snapshot, counted too
    let learn_forgot = learn_of(&svc, &sid, "k in");
    let r = svc.track_learn_mark(&sid, "k in", "not", None, Some(t(2))).unwrap();
    let marked = events(&svc, &sid);
    let learn_marked = learn_of(&svc, &sid, "k in");
    assert!(n(&r, "learn_out_after") >= 1);
    let r = svc
        .track_learn_set_events(&sid, "k in", &learn_forgot, Some(&Value::Array(plain.clone())))
        .unwrap();
    assert_eq!(
        (n(&r, "learn_out_before"), n(&r, "learn_out_after")),
        (filtered(&marked).len(), 0)
    );
    let r = svc
        .track_learn_set_events(&sid, "k in", &learn_marked, Some(&Value::Array(marked.clone())))
        .unwrap();
    assert_eq!(
        (n(&r, "learn_out_before"), n(&r, "learn_out_after")),
        (0, filtered(&marked).len())
    );
}

/// P4-6c fix round: a 0.4.0 project holds `on: true` after a first K; it opens with the filter off. One with a "not"
/// example keeps it on.
#[test]
fn a_0_4_0_project_with_only_this_examples_opens_with_the_filter_off() {
    let (svc, sid, dir) = session("learn_migrate", &[0.5, 1.0]);
    svc.track_learn_mark(&sid, "k in", "this", None, Some(0.5)).unwrap();
    let mut learn = learn_of(&svc, &sid, "k in");
    learn["on"] = json!(true);
    learn["auto"] = json!(false);
    svc.track_learn_set(&sid, "k in", &learn).unwrap();
    svc.project_save(&sid).unwrap();
    svc.project_close(&sid, true).unwrap();
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(learn_of(&svc, &sid, "k in")["on"], false, "migrated on load");
    // planted fault: with a not example the filter stays on
    svc.track_learn_mark(&sid, "k in", "not", None, Some(1.0)).unwrap();
    assert_eq!(learn_of(&svc, &sid, "k in")["on"], true);
    svc.project_save(&sid).unwrap();
    svc.project_close(&sid, true).unwrap();
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(learn_of(&svc, &sid, "k in")["on"], true);
}

/// P4-38b fix round 2: K on an existing hit (a `hit` mark) never adds a hit.
#[test]
fn k_on_a_detected_hit_adds_no_hit_and_does_not_turn_the_filter_on() {
    let (svc, sid, _) = session("learn_k_on_hit", &[0.5, 1.0]);
    let before = svc.events_get(&sid, "k in").unwrap()["events"]
        .as_array()
        .unwrap()
        .len();
    let r = svc.track_learn_mark(&sid, "k in", "this", None, Some(1.0)).unwrap();
    assert_eq!(r["marked"], true, "{r}");
    assert_eq!(r["from"], "hit");
    assert_eq!(r["added_hit"], Value::Null, "{r}"); // planted fault: the fallback always adds
    assert_eq!(r["filter_on"], false);
    assert_eq!(learn_of(&svc, &sid, "k in")["on"], false);
    let evs = svc.events_get(&sid, "k in").unwrap()["events"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(evs.len(), before, "{evs:?}");
    assert!(evs.iter().all(|e| e.get("learned").is_none()));
}

/// P4-38d (fix of P4-38b fix round 2): a K that adds a hit keeps the filter off (the Add rule runs
/// with it off), and the answer carries the track's events (the page adopts them).
#[test]
fn a_k_add_keeps_the_filter_off_and_the_answer_carries_the_events() {
    let (svc, sid, _) = session("learn_k_add_off", &[0.5, 1.0]);
    let r = svc
        .track_learn_mark(&sid, "k in", "this", Some((3.0, 3.4)), None)
        .unwrap();
    assert_eq!(r["added_hit"], json!(3.2), "{r}");
    assert_eq!(r["filter_on"], false, "{r}"); // planted fault: the old switch turns the filter on
    assert_eq!(learn_of(&svc, &sid, "k in")["on"], false);
    let times: Vec<f64> = r["filter"]["events"] // planted fault: no events in the answer
        .as_array()
        .unwrap_or_else(|| panic!("{r}"))
        .iter()
        .map(|e| e["t"].as_f64().unwrap())
        .collect();
    assert_eq!(times, vec![0.5, 1.0, 3.2], "{r}");
    let r2 = svc
        .track_learn_mark(&sid, "k in", "this", Some((3.45, 3.49)), None)
        .unwrap();
    assert_eq!(r2["added_hit"], json!(3.47), "{r2}");
    assert_eq!(r2["filter_on"], false);
    assert_eq!(learn_of(&svc, &sid, "k in")["on"], false);
    assert_eq!(r2["filter"]["events"].as_array().unwrap().len(), 4, "{r2}");
}

/// P4-38d: a K on a detected hit first, then a K that ADDS a hit: the filter stays off.
#[test]
fn a_this_mark_on_a_detected_hit_then_a_k_add_keep_the_filter_off() {
    let (svc, sid, _) = session("learn_k_hit_then_add", &[0.5, 1.0]);
    let a = svc.track_learn_mark(&sid, "k in", "this", None, Some(1.0)).unwrap();
    assert_eq!(
        (a["filter_on"].clone(), a["added_hit"].clone()),
        (json!(false), Value::Null)
    );
    let b = svc
        .track_learn_mark(&sid, "k in", "this", Some((3.0, 3.4)), None)
        .unwrap();
    assert_eq!(b["added_hit"], json!(3.2), "{b}");
    assert_eq!(b["filter_on"], false, "{b}"); // planted fault: the old switch turns the filter on
    assert_eq!(b["mark"]["added"], json!(true));
    assert_eq!(learn_of(&svc, &sid, "k in")["on"], false);
}

/// P4-38d Gate: on a busy kick track (at least 300 detected hits) one K-add on an empty range keeps
/// every detected hit: the count after is the count before + 1 (the added hit) + the Add-rule hits,
/// no hit is marked out, and `learn.on` stays false. P4-38c found the old switch (the first K-add
/// turned the filter on) cut such a track from 357 to 108 output hits.
#[test]
fn a_k_add_on_a_busy_track_keeps_every_detected_hit_with_the_filter_off() {
    let root = tmp("learn_busy");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    // 330 kicks every 0.2 s, then 2 s of silence for the K-add
    let onsets: Vec<f64> = (0..330).map(|i| 0.25 + 0.2 * f64::from(i)).collect();
    let dur = onsets.last().unwrap() + 2.5;
    let spec = Spec {
        name: "k in",
        role: "kick",
        note: 36,
        x: kick_signal(&onsets, dur, 48_000),
        sr: 48_000,
    };
    let dir = project(&root, "Busy", &[spec]);
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    let before = events(&svc, &sid);
    assert!(before.len() >= 300, "a busy track: {} hits", before.len());
    let t_add = onsets.last().unwrap() + 1.0;
    let r = svc
        .track_learn_mark(&sid, "k in", "this", Some((t_add, t_add + 0.4)), None)
        .unwrap();
    let added = r["added_hit"].as_f64().unwrap_or_else(|| panic!("K adds a hit: {r}"));
    assert!((t_add - 1e-6..=t_add + 0.4).contains(&added), "{r}");
    let after = events(&svc, &sid);
    let add_rule = after
        .iter()
        .filter(|e| e["learned"] == true && e.get("added").is_none())
        .count();
    eprintln!(
        "P4-38d busy track: before {} hits, after {} hits (1 K-add + {add_rule} Add-rule), filtered {}",
        before.len(),
        after.len(),
        filtered(&after).len()
    );
    assert_eq!(learn_of(&svc, &sid, "k in")["on"], false, "{r}"); // planted fault: the old switch
    assert!(r.get("filter_error").is_none(), "the Learn pick ran: {r}");
    assert_eq!(filtered(&after), Vec::<f64>::new(), "no hit is marked out");
    assert_eq!(after.len(), before.len() + 1 + add_rule, "{r}");
    let now = times(&after);
    for t in times(&before) {
        assert!(
            now.iter().any(|x| (x - t).abs() < 1e-9),
            "the detected hit at {t} stays"
        );
    }
    assert_eq!(
        r["filter"]["events"].as_array().unwrap(),
        &after,
        "the answer holds the stored hits"
    );
}

/// P4-38e: the session of the deleted-Learn-hit tests: loud kicks, then quiet kicks (detected), then silence. A K on the
/// silence after the last kick adds the Learn hit (the hand-add path, no Add-rule look-alikes). Returns the service, the
/// sid, the plain hits (before the K), the K-added hit and the hits after the K.
struct Dropped {
    svc: Service,
    sid: String,
    plain: Vec<Value>,
    k_hit: f64,
    after: Vec<Value>,
}

fn dropped_session(_test: &str, tag: &str) -> Option<Dropped> {
    let root = tmp(tag);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    // loud kicks, then quiet kicks of the same shape (under the Sensitivity: the look-alikes)
    let sr = 48_000;
    let loud: Vec<f64> = (0..8).map(|i| 0.25 + 0.5 * f64::from(i)).collect();
    let quiet: Vec<f64> = (0..8).map(|i| 4.5 + 0.5 * f64::from(i)).collect();
    let dur = 9.0;
    let q = kick_signal(&quiet, dur, sr);
    let x: Vec<f32> = kick_signal(&loud, dur, sr)
        .iter()
        .zip(&q)
        .map(|(a, b)| a + 0.06 * b)
        .collect();
    let spec = Spec {
        name: "k in",
        role: "kick",
        note: 36,
        x,
        sr,
    };
    let dir = project(&root, "Dropped", &[spec]);
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    let plain = events(&svc, &sid);
    // K on the silence after the last kick (the range of no hit): it adds the Learn hit at the range centre
    let r = svc
        .track_learn_mark(&sid, "k in", "this", Some((8.6, 8.8)), None)
        .unwrap();
    let k_hit = r["added_hit"]
        .as_f64()
        .unwrap_or_else(|| panic!("K adds a hit: {r} (plain {plain:?})"));
    assert!(!near(&times(&plain), k_hit), "the K hit is no detected hit: {plain:?}");
    let after = events(&svc, &sid);
    assert!(
        after
            .iter()
            .any(|e| (e["t"].as_f64().unwrap() - k_hit).abs() < 0.001 && e["learned"] == true),
        "the K hit is a learned hit: {after:?}"
    );
    Some(Dropped {
        svc,
        sid,
        plain,
        k_hit,
        after,
    })
}

/// The index of the hit within 1 ms of `t` (`added_hit` is rounded).
fn index_near(ev: &[Value], t: f64) -> usize {
    ev.iter()
        .position(|e| (e["t"].as_f64().unwrap() - t).abs() < 0.001)
        .unwrap_or_else(|| panic!("a hit at {t}: {ev:?}"))
}

fn near(now: &[f64], t: f64) -> bool {
    now.iter().any(|x| (x - t).abs() < 0.02)
}

/// P4-38e Gate: a K on a silent spot adds a Learn hit; the engineer deletes it; a second Learn change keeps it deleted
/// and the other hits; the undo of the delete (the page writes its snapshot back) brings it back, and it stays through
/// the next Learn change.
#[test]
fn a_deleted_learn_hit_stays_deleted_on_the_next_learn_change() {
    let Some(Dropped {
        svc,
        sid,
        plain,
        k_hit,
        after,
    }) = dropped_session(
        "a_deleted_learn_hit_stays_deleted_on_the_next_learn_change",
        "learn_dropped",
    )
    else {
        return;
    };
    // delete the K-added hit
    svc.events_edit(
        &sid,
        "k in",
        &[json!({"op": "delete", "index": index_near(&after, k_hit)})],
    )
    .unwrap();
    let edited = events(&svc, &sid);
    assert_eq!(
        learn_of(&svc, &sid, "k in")["dropped"].as_array().map(Vec::len),
        Some(1),
        "the deletion is noted"
    );
    // a second Learn change: K on a loud detected hit (no add); the Add rule and the forced hit run again
    let r = svc
        .track_learn_mark(&sid, "k in", "this", None, Some(plain[1]["t"].as_f64().unwrap()))
        .unwrap();
    assert!(r.get("filter_error").is_none(), "the Learn pick ran: {r}");
    let now = times(&events(&svc, &sid));
    // planted fault: the refilter ignores the delete
    assert!(!near(&now, k_hit), "the deleted K hit stays deleted: {now:?}");
    assert_eq!(now, times(&edited), "no other hit changes");
    // undo of the delete: the page writes its snapshot back; the next Learn change keeps the hit
    svc.events_set_by_hand(&sid, "k in", &Value::Array(after.clone()))
        .unwrap();
    assert!(
        learn_of(&svc, &sid, "k in").get("dropped").is_none(),
        "the undo takes the times out"
    );
    svc.track_learn_mark(&sid, "k in", "this", None, Some(plain[2]["t"].as_f64().unwrap()))
        .unwrap();
    let now = times(&events(&svc, &sid));
    assert!(near(&now, k_hit), "the undone hit is back: {now:?}");
}

/// P4-38e: the engineer deletes the Learn hit, then presses K exactly on its spot because they want it after all: the
/// hit is there, `learn.dropped` no longer holds the time, and the next Learn change keeps the hit.
#[test]
fn k_on_the_spot_of_a_deleted_learn_hit_brings_it_back() {
    let Some(Dropped {
        svc,
        sid,
        plain,
        k_hit,
        after,
    }) = dropped_session("k_on_the_spot_of_a_deleted_learn_hit_brings_it_back", "learn_undrop")
    else {
        return;
    };
    let gone = k_hit;
    svc.events_edit(
        &sid,
        "k in",
        &[json!({"op": "delete", "index": index_near(&after, gone)})],
    )
    .unwrap();
    let dropped = |svc: &Service| -> Vec<f64> {
        learn_of(svc, &sid, "k in")["dropped"]
            .as_array()
            .map(|d| d.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default()
    };
    assert!(near(&dropped(&svc), gone), "the deletion is noted: {:?}", dropped(&svc));
    // K on the spot of the deleted hit
    let r = svc.track_learn_mark(&sid, "k in", "this", None, Some(gone)).unwrap();
    assert!(r.get("filter_error").is_none(), "the Learn pick ran: {r}");
    let now = times(&events(&svc, &sid));
    assert!(near(&now, gone), "the K brings the hit back: {r} {now:?}");
    assert_eq!(
        now.iter().filter(|&&x| (x - gone).abs() < 0.02).count(),
        1,
        "one hit there: {now:?}"
    );
    // planted fault: the K-mark path keeps the dropped time
    assert!(
        !near(&dropped(&svc), gone),
        "the K takes the time out: {:?}",
        dropped(&svc)
    );
    // the next Learn change keeps it
    svc.track_learn_mark(&sid, "k in", "this", None, Some(plain[1]["t"].as_f64().unwrap()))
        .unwrap();
    assert!(
        near(&times(&events(&svc, &sid)), gone),
        "the hit stays on the next Learn change"
    );
}

/// P4-38e: the sweep of Forget This takes the K-added hit away, but that is no hand delete: `learn.dropped` stays
/// empty. A hand delete of a K-added hit is noted.
#[test]
fn the_learn_sweep_is_no_hand_delete() {
    let (svc, sid, _) = session("learn_sweep_dropped", &[0.5, 1.0]);
    let k = svc.track_learn_mark(&sid, "k in", "this", None, Some(2.0)).unwrap();
    assert_eq!(k["added_hit"], json!(2.0), "{k}");
    let r = svc.track_learn_forget_at(&sid, "k in", None, Some(2.0)).unwrap();
    assert_eq!(r["swept"], json!(1), "{r}");
    assert!(!near(&times(&events(&svc, &sid)), 2.0), "the sweep takes the hit away");
    // planted fault: the sweep goes through the hand path
    assert!(learn_of(&svc, &sid, "k in").get("dropped").is_none(), "no dropped time");
    // the same hit deleted by hand is noted
    svc.track_learn_mark(&sid, "k in", "this", None, Some(3.0)).unwrap();
    let i = index_near(&events(&svc, &sid), 3.0);
    svc.events_edit(&sid, "k in", &[json!({"op": "delete", "index": i})])
        .unwrap();
    let d = learn_of(&svc, &sid, "k in")["dropped"].clone();
    assert!(
        d.as_array()
            .is_some_and(|a| a.len() == 1 && (a[0].as_f64().unwrap() - 3.0).abs() < 1e-3),
        "the hand delete is noted: {d}"
    );
}

/// P4-38b Add rule on the DSP path: a ghost kick at -18 dB sits under the level gate, so the detector drops it. A Learn
/// This on a loud kick (the filter stays off) makes the Add rule recover the ghost: a learned hit, not a K-added one.
#[test]
fn the_add_rule_recovers_a_soft_hit_the_detector_drops() {
    let root = tmp("learn_add_soft");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sr = 48_000;
    let dur = 3.5;
    let ghost_t = 1.5;
    let loud = kick_signal(&ONSETS, dur, sr);
    let ghost = kick_signal(&[ghost_t], dur, sr);
    let g = 10f32.powf(-18.0 / 20.0);
    let x: Vec<f32> = loud.iter().zip(&ghost).map(|(a, b)| a + g * b).collect();
    let spec = Spec {
        name: "k in",
        role: "kick",
        note: 36,
        x,
        sr,
    };
    let dir = project(&root, "AddSoft", &[spec]);
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    let plain = events(&svc, &sid);
    assert!(
        !near(&times(&plain), ghost_t),
        "premise: the detector drops the ghost: {plain:?}"
    );
    let r = svc
        .track_learn_mark(&sid, "k in", "this", None, Some(ONSETS[1]))
        .unwrap();
    assert!(r.get("filter_error").is_none(), "the Learn pick ran: {r}");
    let after = events(&svc, &sid);
    assert!(
        after
            .iter()
            .any(|e| (e["t"].as_f64().unwrap() - ghost_t).abs() < 0.02 && e["learned"] == true),
        "the Add rule recovers the ghost as a learned hit: {r} {after:?}"
    );
    assert!(
        after.iter().all(|e| e.get("added").is_none()),
        "a Learn This does not K-add: {after:?}"
    );
}

// ---------- R07 (#253): the hand write is one atomic write with the Learn bookkeeping ----------

/// The seeded hits as Learn hits, with a learn object on the track.
fn learned_session(tag: &str) -> (Service, String, Vec<Value>) {
    let (svc, sid, _) = session(tag, &[0.5, 1.0, 1.5]);
    let hits: Vec<Value> = [0.5, 1.0, 1.5]
        .iter()
        .map(|t| json!({"t": t, "v": 90, "learned": true}))
        .collect();
    svc.events_set(&sid, "k in", &Value::Array(hits.clone())).unwrap();
    svc.track_learn_set(&sid, "k in", &json!({"on": false, "marks": []}))
        .unwrap();
    (svc, sid, hits)
}

/// R07: a hand write that overlaps another write of the project is refused with a retryable 409
/// (the held store guard is the barrier); the retry stores the list and its Learn bookkeeping as
/// one transaction: the deleted Learn hit is out of the events and in `learn.dropped`.
#[test]
fn events_set_by_hand_refuses_while_another_write_runs_and_keeps_learn_matching() {
    let (svc, sid, hits) = learned_session("learn_r07_refuse");
    let g = svc.jobs().begin_store(&sid).expect("the write guard is free");
    let s2 = svc.clone();
    let sid2 = sid.clone();
    let overlapping = std::thread::spawn(move || s2.events_set_by_hand(&sid2, "k in", &Value::Array(hits)));
    let e = overlapping.join().unwrap().unwrap_err();
    assert_eq!(e.code, 409, "{e:?}");
    assert!(e.message.contains("try again"), "the conflict is retryable: {e:?}");
    drop(g); // the write in flight is done
    // the page's edited list (the hit at 1.0 deleted) retries and lands with its bookkeeping
    let edited: Vec<Value> = [0.5_f64, 1.5]
        .iter()
        .map(|t| json!({"t": t, "v": 90, "learned": true}))
        .collect();
    svc.events_set_by_hand(&sid, "k in", &Value::Array(edited)).unwrap();
    let f = times(&events(&svc, &sid));
    assert!(!near(&f, 1.0), "the delete is stored: {f:?}");
    let d = learn_of(&svc, &sid, "k in")["dropped"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        d.len() == 1 && (d[0].as_f64().unwrap() - 1.0).abs() < 1e-6,
        "the deleted Learn hit is noted: {d:?}"
    );
}

/// R07: a hand delete of a Learn hit and the page's stale full-list write start at a barrier
/// (no sleeps). Whatever the schedule, the stored `learn.dropped` never names a hit that the
/// events still hold, and a hit is never both: that pair is the silently lost edit of the old
/// read/commit race.
#[test]
fn overlapping_hand_edits_keep_learn_matching_the_events() {
    let (svc, sid, stale) = learned_session("learn_r07_overlap");
    for round in 0..30 {
        svc.events_set(&sid, "k in", &Value::Array(stale.clone())).unwrap();
        // a fresh learn object per round: the last round's drop note must not leak into this one
        svc.track_learn_set(&sid, "k in", &json!({"on": false, "marks": []}))
            .unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let (s1, s2, sid1, sid2, b1, b2) = (
            svc.clone(),
            svc.clone(),
            sid.clone(),
            sid.clone(),
            Arc::clone(&barrier),
            Arc::clone(&barrier),
        );
        let del = std::thread::spawn(move || {
            b1.wait();
            s1.events_edit(&sid1, "k in", &[json!({"op": "delete", "index": 1})])
                .map(|_| ())
        });
        let stale_round = stale.clone();
        let stale_write = std::thread::spawn(move || {
            b2.wait();
            s2.events_set_by_hand(&sid2, "k in", &Value::Array(stale_round))
        });
        let del = del.join().unwrap();
        let stale_write = stale_write.join().unwrap();
        // the page retries its refused op-based edit (a stale full list is never retried blind:
        // the page re-reads); a refused hand write stays refused
        if let Err(e) = &del {
            assert_eq!(e.code, 409, "round {round}: {e:?}");
            svc.events_edit(&sid, "k in", &[json!({"op": "delete", "index": 1})])
                .unwrap();
        }
        if let Err(e) = &stale_write {
            assert_eq!(e.code, 409, "round {round}: {e:?}");
        }
        let f = events(&svc, &sid);
        let d = learn_of(&svc, &sid, "k in")["dropped"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let gone = d.iter().any(|x| (x.as_f64().unwrap() - 1.0).abs() < 1e-6);
        let back = near(&times(&f), 1.0);
        assert!(
            gone != back,
            "round {round}: the hit at 1.0 is in the events xor in learn.dropped, never both or neither: events {f:?} dropped {d:?}"
        );
    }
}
