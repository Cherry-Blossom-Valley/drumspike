//! P4-5 step 2: `events_edit`, `track_shift_set`, `export_midi_stored` and the sensitivity in the
//! params (`crates/app/src/edit.rs`), through `drumspike_app::Service`, no Tauri.

mod common;

use common::*;
use drumspike_app::Service;
use serde_json::{json, Value};
use std::sync::{Arc, Barrier};

fn opened(tag: &str) -> (Service, String, std::path::PathBuf) {
    let root = tmp(tag);
    let dir = project(
        &root,
        "Edit",
        &[kick_spec("k in", "kick", 36), kick_spec("k sub", "kick", 36)],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    (svc, sid, dir)
}

fn hits(svc: &Service, sid: &str, name: &str) -> Vec<Value> {
    svc.events_get(sid, name).unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn seed(svc: &Service, sid: &str, name: &str) {
    let ev = json!([{"t": 0.5, "v": 80}, {"t": 1.0, "v": 90}, {"t": 1.5, "v": 100}]);
    svc.events_set(sid, name, &ev).unwrap();
}

#[test]
fn events_edit_changes_single_hits_and_marks_them_as_the_page_does() {
    let (svc, sid, _) = opened("ee1");
    seed(&svc, &sid, "k in");
    let ops = [
        json!({"op": "add", "t": 0.25}),
        json!({"op": "move", "index": 0, "t": 0.6}),
        json!({"op": "delete", "index": 2}),
        json!({"op": "velocity", "index": 1, "v": 64}),
    ];
    let r = svc.events_edit(&sid, "k in", &ops).unwrap();
    assert_eq!(r["saved"], 3);
    assert_eq!(
        [
            r["added"].clone(),
            r["deleted"].clone(),
            r["moved"].clone(),
            r["velocity"].clone()
        ],
        [1, 1, 1, 1]
    );
    let ev = hits(&svc, &sid, "k in");
    assert_eq!(ev[0], json!({"t": 0.25, "v": 100, "added": true}));
    assert_eq!(ev[1], json!({"t": 0.6, "v": 80, "orig": 0.5}));
    assert_eq!(ev[2], json!({"t": 1.0, "v": 64}));
    assert!(svc.project_get(&sid).unwrap()["dirty"].as_bool().unwrap());
}

#[test]
fn events_edit_add_with_added_false_puts_a_detected_or_moved_hit_back() {
    let (svc, sid, _) = opened("ee1b");
    seed(&svc, &sid, "k in");
    let ops = [
        json!({"op": "add", "t": 0.7, "v": 55, "added": false}),
        json!({"op": "add", "t": 0.9, "v": 66, "added": false, "orig": 0.8}),
    ];
    svc.events_edit(&sid, "k in", &ops).unwrap();
    let ev = hits(&svc, &sid, "k in");
    assert_eq!(ev[1], json!({"t": 0.7, "v": 55}));
    assert_eq!(ev[2], json!({"t": 0.9, "v": 66, "orig": 0.8}));
}

#[test]
// #151: the range edits of the page send one `events_edit` call with one op per hit: a batch of
// deletes removes exactly its hits (the other hits and the other track stay), the undo re-adds them
// in one call so the document is the stored one again, and a batch of moves moves every hit once.
fn events_edit_batch_delete_and_move_act_on_their_hits_as_one_call() {
    let (svc, sid, _) = opened("ee-range");
    let seven: Vec<Value> = [0.3, 0.9, 1.2, 1.5, 1.8, 2.1, 3.5]
        .iter()
        .map(|t| json!({"t": t, "v": 90}))
        .collect();
    svc.events_set(&sid, "k in", &Value::Array(seven.clone())).unwrap();
    svc.events_set(&sid, "k sub", &json!([{"t": 0.7, "v": 90}])).unwrap();

    // one call, 5 delete ops: exactly those 5 are gone
    let del: Vec<Value> = [1, 2, 3, 4, 5]
        .iter()
        .map(|&i| json!({"op": "delete", "index": i}))
        .collect();
    let r = svc.events_edit(&sid, "k in", &del).unwrap();
    assert_eq!(r["deleted"], 5);
    assert_eq!(
        Value::Array(hits(&svc, &sid, "k in")),
        json!([{"t": 0.3, "v": 90}, {"t": 3.5, "v": 90}])
    );
    assert_eq!(hits(&svc, &sid, "k sub").len(), 1, "the other track does not change");

    // the page's undo puts them back in one call (added false, as stored): the document is the saved one again
    let back_t = [0.9, 1.2, 1.5, 1.8, 2.1];
    let back: Vec<Value> = back_t
        .iter()
        .map(|t| json!({"op": "add", "t": t, "v": 90, "added": false}))
        .collect();
    svc.events_edit(&sid, "k in", &back).unwrap();
    assert_eq!(Value::Array(hits(&svc, &sid, "k in")), Value::Array(seven.clone()));

    // a batch of moves: one call moves every hit of the range (a moved detected hit keeps its orig)
    let to = [0.901, 1.201, 1.501, 1.801, 2.101];
    let mv: Vec<Value> = to
        .iter()
        .zip([1usize, 2, 3, 4, 5])
        .map(|(&t, i)| json!({"op": "move", "index": i, "t": t}))
        .collect();
    let r = svc.events_edit(&sid, "k in", &mv).unwrap();
    assert_eq!(r["moved"], 5);
    let ev = hits(&svc, &sid, "k in");
    assert_eq!(ev[0], json!({"t": 0.3, "v": 90}), "the hit outside the range stays");
    // `t` sits on the sample grid (half a sample of slack at 48 kHz); `orig` is the time before the move
    for (k, ev) in ev[1..6].iter().enumerate() {
        let (t, orig) = (ev["t"].as_f64().unwrap(), ev["orig"].as_f64().unwrap());
        let (want_t, want_orig) = (to[k], back_t[k]);
        assert!(
            (t - want_t).abs() < 1e-4,
            "hit {k}: t {t} is not on the grid ({want_t})"
        );
        assert!(
            (orig - want_orig).abs() < 1e-9,
            "hit {k}: orig {orig} is not the time before the move ({want_orig})"
        );
    }
    assert_eq!(ev[6], json!({"t": 3.5, "v": 90}), "the hit outside the range stays");

    // one bad op in the batch refuses the whole call: nothing changes
    let bad = vec![
        json!({"op": "delete", "index": 1}),
        json!({"op": "delete", "index": 99}),
    ];
    assert_eq!(svc.events_edit(&sid, "k in", &bad).unwrap_err().code, 400);
    assert_eq!(hits(&svc, &sid, "k in").len(), 7, "nothing changed");
}

#[test]
fn events_edit_refuses_a_bad_op_without_a_change_and_a_follower() {
    let (svc, sid, _) = opened("ee2");
    seed(&svc, &sid, "k in");
    let bad = [
        json!({"op": "velocity", "index": 0, "v": 70}),
        json!({"op": "delete", "index": 9}),
    ];
    assert_eq!(svc.events_edit(&sid, "k in", &bad).unwrap_err().code, 400);
    assert_eq!(hits(&svc, &sid, "k in")[0]["v"], 80, "nothing changed");
    assert_eq!(svc.events_edit(&sid, "k in", &[]).unwrap_err().code, 400);
    assert_eq!(svc.events_edit(&sid, "nope", &bad[..1]).unwrap_err().code, 400);
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    let e = svc.events_edit(&sid, "k sub", &bad[..1]).unwrap_err();
    assert_eq!(e.code, 409, "{}", e.message);
}

#[test]
fn track_shift_set_moves_the_hits_by_the_difference_and_stores_the_value() {
    let (svc, sid, _) = opened("ts1");
    seed(&svc, &sid, "k in");
    svc.events_edit(&sid, "k in", &[json!({"op": "move", "index": 0, "t": 0.6})])
        .unwrap();
    let r = svc.track_shift_set(&sid, "k in", 48).unwrap(); // +1 ms at 48 kHz
    assert_eq!((r["delta"].clone(), r["moved"].clone()), (json!(48), json!(3)));
    let ev = hits(&svc, &sid, "k in");
    assert_eq!(ev[0]["t"], 0.601);
    assert_eq!(ev[0]["orig"], 0.501, "orig moves too");
    assert_eq!(ev[1]["t"], 1.001);
    let again = svc.track_shift_set(&sid, "k in", 48).unwrap();
    assert_eq!((again["delta"].clone(), again["moved"].clone()), (json!(0), json!(0)));
    let back = svc.track_shift_set(&sid, "k in", 0).unwrap();
    assert_eq!(back["delta"], -48);
    assert_eq!(hits(&svc, &sid, "k in")[1]["t"], 1.0);
    let tracks = svc.tracks_get(&sid).unwrap();
    assert_eq!(tracks["tracks"]["k in"]["params"]["shift_samples"], 0);
}

#[test]
fn track_shift_set_checks_range_follower_and_track() {
    let (svc, sid, _) = opened("ts2");
    assert_eq!(svc.track_shift_set(&sid, "k in", 100_001).unwrap_err().code, 400);
    assert_eq!(svc.track_shift_set(&sid, "nope", 1).unwrap_err().code, 400);
    svc.track_set_follow(&sid, "k sub", Some("k in")).unwrap();
    assert_eq!(svc.track_shift_set(&sid, "k sub", 1).unwrap_err().code, 409);
}

/// R14: `i64::MIN` must not overflow in debug or evade the bound in release.
#[test]
fn track_shift_set_rejects_i64_min() {
    let (svc, sid, _) = opened("ts3");
    assert_eq!(svc.track_shift_set(&sid, "k in", i64::MIN).unwrap_err().code, 400);
}

#[test]
fn export_midi_stored_takes_the_note_and_the_hits_from_the_document() {
    let (svc, sid, dir) = opened("em1");
    seed(&svc, &sid, "k in");
    svc.track_set_note(&sid, "k in", 40).unwrap();
    let r = svc.export_midi_stored(&sid, "k in", None, None, None, None).unwrap();
    assert_eq!(r["n_events"], 3);
    let bytes = std::fs::read(dir.join("Exports/k in_spike.mid")).unwrap();
    assert!(bytes.windows(2).any(|w| w == [0x99, 40]), "note 40 in the file");
    let explicit = svc
        .export_midi_stored(&sid, "k in", Some(38), Some(&[json!({"t": 1.0, "v": 90})]), None, None)
        .unwrap();
    assert_eq!(explicit["n_events"], 1);
}

#[test]
fn track_params_set_maps_a_sensitivity_to_thresholds() {
    let (svc, sid, _) = opened("ps1");
    let params = json!({"sensitivity": 100, "min_gap_ms": 20});
    let r = svc.track_params_set(&sid, "k in", params.as_object().unwrap()).unwrap();
    let th = r["params"]["thresholds"].as_array().unwrap();
    assert_eq!(th.len(), 5);
    assert!((th[0].as_f64().unwrap() - 0.088).abs() < 1e-9, "{th:?}");
}

#[test]
fn analyze_start_keep_edited_keeps_the_hand_edits_and_the_default_replaces_them() {
    let (svc, sid, _) = opened("ka1");
    svc.analyze_start(&sid, Some(vec!["k in".into()]), None, None).unwrap();
    wait_done(&svc, &sid);
    let auto = hits(&svc, &sid, "k in");
    assert!(auto.len() >= 4, "{auto:?}");
    let t0 = auto[0]["t"].as_f64().unwrap();
    let ops = [
        json!({"op": "add", "t": 0.1}),
        json!({"op": "move", "index": 1, "t": auto[1]["t"].as_f64().unwrap() + 0.05}),
    ];
    svc.events_edit(&sid, "k in", &ops).unwrap();
    // the default (the page): the analysis replaces everything
    svc.analyze_start(&sid, Some(vec!["k in".into()]), None, None).unwrap();
    wait_done(&svc, &sid);
    assert!(hits(&svc, &sid, "k in")
        .iter()
        .all(|e| e.get("added").is_none() && e.get("orig").is_none()));
    svc.events_edit(&sid, "k in", &ops).unwrap();
    svc.analyze_start_keep(&sid, Some(vec!["k in".into()]), None, None, None, true)
        .unwrap();
    wait_done(&svc, &sid);
    let kept = hits(&svc, &sid, "k in");
    assert!(kept.iter().any(|e| e["t"] == 0.1 && e["added"] == true), "{kept:?}");
    let moved: Vec<_> = kept.iter().filter(|e| e.get("orig").is_some()).collect();
    assert_eq!(moved.len(), 1, "{kept:?}");
    assert!(
        kept.iter().any(|e| (e["t"].as_f64().unwrap() - t0).abs() < 1e-9),
        "a detected hit is back"
    );
    let ts: Vec<f64> = kept.iter().map(|e| e["t"].as_f64().unwrap()).collect();
    assert!(ts.windows(2).all(|w| w[0] <= w[1]), "sorted");
}

/// #144: an add with `snap: true` lands on the analysis onset of the attack near its time, with the velocity that
/// `velocities_recompute` gives there; with no attack within 10 ms it keeps its time and velocity 100. The Hitpoint shift
/// moves the snapped hit with the analysed ones. An add without `snap` stays at its time.
#[test]
fn events_edit_add_with_snap_lands_on_the_analysis_onset_with_the_audio_velocity() {
    let (svc, sid, _) = opened("snap1");
    svc.analyze_start(&sid, Some(vec!["k in".into()]), None, None).unwrap();
    wait_done(&svc, &sid);
    for shift in [0i64, 48] {
        svc.track_shift_set(&sid, "k in", shift).unwrap();
        let auto = hits(&svc, &sid, "k in");
        let i = auto
            .iter()
            .position(|e| (e["t"].as_f64().unwrap() - 0.75).abs() < 0.01)
            .expect("the analysis found the kick at 0.75 s");
        let onset = auto[i]["t"].as_f64().unwrap();
        svc.events_edit(&sid, "k in", &[json!({"op": "delete", "index": i})])
            .unwrap();
        let r = svc
            .events_edit(
                &sid,
                "k in",
                &[
                    json!({"op": "add", "t": onset - 0.003, "snap": true}),
                    json!({"op": "add", "t": 1.0, "snap": true}),
                    json!({"op": "add", "t": 1.2}),
                ],
            )
            .unwrap();
        let s = r["snaps"].as_array().unwrap();
        assert_eq!(s.len(), 2, "{r}");
        let t = s[0]["t"].as_f64().unwrap();
        assert_eq!(s[0]["snapped"], true, "{r}");
        assert!(
            ((t - onset) * 48_000.0).abs() <= 2.0,
            "shift {shift}: snapped {t}, onset {onset}"
        );
        let want = svc
            .velocities_recompute(&sid, "k in", Some(&[json!({"t": t})]))
            .unwrap()["events"][0]["v"]
            .clone();
        assert_eq!(s[0]["v"], want);
        assert_ne!(want, 100, "planted fault: the fixed velocity would pass");
        assert_eq!(
            s[1],
            json!({"click": 1.0, "t": 1.0, "v": 100, "snapped": false}),
            "a tail does not snap"
        );
        let ev = hits(&svc, &sid, "k in");
        assert!(
            ev.iter()
                .any(|e| e["t"] == json!(t) && e["v"] == want && e["added"] == true),
            "{ev:?}"
        );
        assert!(ev.iter().any(|e| e["t"] == 1.0 && e["v"] == 100));
        assert!(ev.iter().any(|e| e["t"] == 1.2 && e["v"] == 100));
        assert!(ev.iter().all(|e| e.get("snap").is_none()));
        // back to the analysed hits for the next round
        let gone: Vec<Value> = ev
            .iter()
            .enumerate()
            .filter(|(_, e)| e["added"] == true)
            .map(|(k, _)| json!({"op": "delete", "index": k}))
            .collect();
        svc.events_edit(&sid, "k in", &gone).unwrap();
        svc.events_edit(&sid, "k in", &[json!({"op": "add", "t": onset, "added": false})])
            .unwrap();
    }
}

/// P4-5 fix round M2: through `call::call` (the agent surface) `track_velocity_set` also writes the new velocities and the
/// cut marks to the stored hits; the Service method and an explicit `apply: false` leave them as they were.
#[test]
fn track_velocity_set_through_call_stores_the_velocities_and_the_cut() {
    use drumspike_app::bridge::Reply;
    use drumspike_app::call::call;
    let (svc, sid, _) = opened("vs1");
    svc.analyze_start(&sid, Some(vec!["k in".into()]), None, None).unwrap();
    wait_done(&svc, &sid);
    let before = hits(&svc, &sid, "k in");
    assert!(before.iter().all(|e| e["v"].as_i64().unwrap() > 100), "{before:?}");
    let map = json!({"sid": sid, "name": "k in", "lo_db": -3.0, "hi_db": 0.0});
    // the Service method (the page) and `apply: false`: the answer has the new velocities, nothing is stored
    let r = svc.track_velocity_set(&sid, "k in", -3.0, 0.0, None, None).unwrap();
    assert!(r["events"].as_array().unwrap().iter().all(|e| e["v"] == 20));
    assert_eq!(hits(&svc, &sid, "k in"), before);
    let mut no = map.clone();
    no["apply"] = json!(false);
    assert!(matches!(
        call(&svc, "track_velocity_set", &no, None),
        Ok(Reply::Json(_))
    ));
    assert_eq!(hits(&svc, &sid, "k in"), before);
    // the agent default: stored, with the answer's events
    let Ok(Reply::Json(r)) = call(&svc, "track_velocity_set", &map, None) else {
        panic!()
    };
    assert_eq!(r["applied"], true);
    let stored = hits(&svc, &sid, "k in");
    assert_eq!(&stored, r["events"].as_array().unwrap());
    assert!(stored.iter().all(|e| e["v"] == 20), "{stored:?}");
    // the cut marks are stored too
    let mut cut = map.clone();
    cut["cut_db"] = json!(-1.0);
    call(&svc, "track_velocity_set", &cut, None).unwrap();
    assert!(hits(&svc, &sid, "k in").iter().all(|e| e["cut"] == true));
    // and the export has them: a cut hit leaves the MIDI file
    let Ok(Reply::Json(m)) = call(&svc, "export_midi", &json!({"sid": sid, "name": "k in"}), None) else {
        panic!()
    };
    assert_eq!(m["n_events"], 0, "{m}");
}

// P4-40f: one `events_edit` call with many `setv` ops is one step (the page sends it as one call,
// one undo step in its stack): every velocity applies, each hand velocity marks the hit `edited`,
// and the hand velocities survive Recompute Velocities (the recompute keeps their `v`) and a
// re-pick (`merge_edited` keeps the edited hits and drops the picks at their times).
#[test]
fn events_edit_batch_setv_is_one_step_that_survives_recompute_and_repick() {
    use drumspike_core::io::{encode_wav, WavFormat, WavSpec};
    let root = tmp("ee-setv");
    let mut p = drumspike_core::project::new_project(&root, "Song").unwrap();
    let x = kick_signal(&ONSETS, 3.5, 48_000);
    let spec = WavSpec {
        sample_rate: 48_000,
        channels: 1,
        format: WavFormat::Pcm16,
        extensible: false,
        channel_mask: 0,
    };
    let wav = root.join("daw/Kick.wav");
    std::fs::create_dir_all(wav.parent().unwrap()).unwrap();
    std::fs::write(&wav, encode_wav(&spec, &x, None)).unwrap();
    let rec = drumspike_core::project::build_source(&wav, &p.dir, "referenced").unwrap();
    drumspike_core::project::add_track(&mut p, "Kick", Some(rec), "kick", 36, Some("average")).unwrap();
    drumspike_core::project::save_project(&mut p).unwrap();
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = svc.project_open(p.dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let seed: Vec<Value> = ONSETS.iter().map(|t| json!({"t": t, "v": 90})).collect();
    svc.events_set(&sid, "Kick", &Value::Array(seed.clone())).unwrap();
    let before: Vec<i64> = svc.velocities_recompute(&sid, "Kick", None).unwrap()["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["v"].as_i64().unwrap())
        .collect();
    assert!(before.iter().any(|&v| v != 90), "{before:?}");

    // one call, many setv ops: the batch counts as one and every op applies
    let hand = [1, 3, 4];
    let ops: Vec<Value> = hand
        .iter()
        .map(|&i| json!({"op": "velocity", "index": i, "v": 30 + i as i64, "edited": true}))
        .collect();
    let r = svc.events_edit(&sid, "Kick", &ops).unwrap();
    assert_eq!(r["velocity"], 3);
    let ev = hits(&svc, &sid, "Kick");
    for &i in &hand {
        assert_eq!(ev[i]["v"], 30 + i as i64);
        assert_eq!(ev[i]["edited"], true, "hand velocity marks the hit edited");
    }
    assert!(ev[0].get("edited").is_none(), "the untouched hit stays as it was");
    let hand_v: Vec<i64> = hand.iter().map(|&i| ev[i]["v"].as_i64().unwrap()).collect();

    // Recompute Velocities: the hand velocities keep their value, the rest is recomputed
    let rec = svc.velocities_recompute(&sid, "Kick", None).unwrap()["events"]
        .as_array()
        .unwrap()
        .clone();
    for &i in &hand {
        assert_eq!(rec[i]["v"], 30 + i as i64, "hand velocity survives");
    }
    assert_ne!(rec[0]["v"].as_i64().unwrap(), 90, "the untouched hit follows the audio");
    assert_ne!(
        &hand_v[..],
        &before[1..4],
        "sanity: the hand values differ from the audio's"
    );

    // a re-pick: merge_edited keeps the edited hits and drops the picks at their times
    let picked: Vec<Value> = ONSETS
        .iter()
        .map(|t| json!({"t": t, "v": 100}))
        .chain(std::iter::once(json!({"t": ONSETS[5] + 0.5, "v": 100})))
        .collect();
    let merged = drumspike_app::edit::merge_edited(picked, &rec);
    for &i in &hand {
        let m = merged.iter().find(|e| e["t"] == rec[i]["t"]).unwrap();
        assert_eq!(m["v"], 30 + i as i64, "hand velocity survives the re-pick");
    }
    assert_eq!(
        merged.len(),
        7,
        "the picks at the three edited hits went, the other picks and the edited hits stay"
    );
    assert!(merged.iter().any(|e| e["t"] == json!(ONSETS[5] + 0.5)));
}

// ---------- R07 (#253): event edits are one atomic write ----------

/// R07: an edit that overlaps another write of the project (a second edit, a re-pick store, a
/// shift) is refused with a retryable 409 — the store guard of the write that is in flight is
/// the barrier — and the retry lands: both edits survive, none is silently lost.
#[test]
fn events_edit_refuses_with_a_retryable_conflict_while_another_write_runs() {
    let (svc, sid, _) = opened("ee_r07_refuse");
    seed(&svc, &sid, "k in");
    let g = svc.jobs().begin_store(&sid).expect("the write guard is free");
    let s2 = svc.clone();
    let sid2 = sid.clone();
    let overlapping = std::thread::spawn(move || s2.events_edit(&sid2, "k in", &[json!({"op": "add", "t": 0.25})]));
    let e = overlapping.join().unwrap().unwrap_err();
    assert_eq!(e.code, 409, "{e:?}");
    assert!(e.message.contains("try again"), "the conflict is retryable: {e:?}");
    drop(g); // the write in flight is done
             // the refused edit retries and lands; the edit whose critical section the guard stood for lands too
    svc.events_edit(&sid, "k in", &[json!({"op": "add", "t": 0.25})])
        .unwrap();
    svc.events_edit(&sid, "k in", &[json!({"op": "add", "t": 1.75})])
        .unwrap();
    let ts: Vec<f64> = hits(&svc, &sid, "k in")
        .iter()
        .filter_map(|e| e["t"].as_f64())
        .collect();
    assert!(ts.contains(&0.25) && ts.contains(&1.75), "both edits survive: {ts:?}");
    assert_eq!(ts.len(), 5, "the three seeded hits and the two edits: {ts:?}");
}

/// R07: two overlapping edits start at a barrier (no sleeps). Both edits survive, or the loser
/// of the store guard gets a retryable 409 and its retry lands. The snap add decodes the audio
/// between the read of the list and the commit — the widest window of the old race; every edit
/// that answered ok must be in the final list, in every schedule.
#[test]
fn two_overlapping_edits_never_lose_one() {
    let (svc, sid, _) = opened("ee_r07_overlap");
    for round in 0..30 {
        seed(&svc, &sid, "k in");
        let barrier = Arc::new(Barrier::new(2));
        let mut threads = Vec::new();
        // 0.6 has no attack within SNAP_S, so the snap add keeps its click whether it snaps or not
        for (t, snap) in [(0.25_f64, false), (0.6, true)] {
            let (svc, sid, barrier) = (svc.clone(), sid.clone(), Arc::clone(&barrier));
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                let mut op = json!({"op": "add", "t": t});
                if snap {
                    op["snap"] = json!(true);
                }
                (t, svc.events_edit(&sid, "k in", &[op]))
            }));
        }
        let mut want: Vec<f64> = Vec::new();
        let mut refused: Vec<f64> = Vec::new();
        for h in threads {
            let (t, r) = h.join().unwrap();
            match r {
                Ok(resp) => want.push(resp.get("snaps").and_then(|s| s[0]["t"].as_f64()).unwrap_or(t)),
                Err(e) if e.code == 409 => refused.push(t),
                Err(e) => panic!("round {round}: {e:?}"),
            }
        }
        for t in refused {
            // the page retries: the op recomputes on the list as it is now
            let resp = svc.events_edit(&sid, "k in", &[json!({"op": "add", "t": t})]).unwrap();
            want.push(resp.get("snaps").and_then(|s| s[0]["t"].as_f64()).unwrap_or(t));
        }
        let ev = hits(&svc, &sid, "k in");
        assert_eq!(
            ev.len(),
            3 + want.len(),
            "round {round}: no edit is silently lost: {ev:?}"
        );
        for t in &want {
            assert!(
                ev.iter().any(|e| e["t"].as_f64().is_some_and(|x| (x - t).abs() < 1e-9)),
                "round {round}: the edit at {t} survives: {ev:?}"
            );
        }
    }
}

/// R07: an edit and a Hitpoint shift start at a barrier (no sleeps). The shift's write and the
/// edit's commit are one writer at a time: the add is in the final list in every schedule (at
/// its own time, or shifted when the shift stored after it), and the shift is applied or was
/// retried after its retryable 409.
#[test]
fn an_edit_and_a_shift_set_never_lose_the_edit() {
    let (svc, sid, _) = opened("ee_r07_shift");
    let sr = 48_000.0;
    for round in 0..30 {
        seed(&svc, &sid, "k in");
        let barrier = Arc::new(Barrier::new(2));
        let (s1, s2, sid1, sid2, b1, b2) = (
            svc.clone(),
            svc.clone(),
            sid.clone(),
            sid.clone(),
            Arc::clone(&barrier),
            Arc::clone(&barrier),
        );
        let shift = std::thread::spawn(move || {
            b1.wait();
            s1.track_shift_set(&sid1, "k in", 48)
        });
        let edit = std::thread::spawn(move || {
            b2.wait();
            s2.events_edit(&sid2, "k in", &[json!({"op": "add", "t": 0.25})])
                .map(|_| 0.25)
        });
        let shift = shift.join().unwrap();
        let edit = edit.join().unwrap();
        // the page retries what the store guard refused
        let delta = match &shift {
            Ok(r) => r["delta"].as_i64().unwrap(),
            Err(e) if e.code == 409 => svc.track_shift_set(&sid, "k in", 48).unwrap()["delta"]
                .as_i64()
                .unwrap(),
            Err(e) => panic!("round {round}: {e:?}"),
        };
        if let Err(e) = &edit {
            assert_eq!(e.code, 409, "round {round}: {e:?}");
            svc.events_edit(&sid, "k in", &[json!({"op": "add", "t": 0.25})])
                .unwrap();
        }
        let d = delta as f64 / sr;
        let on_grid = |t: f64| (t * sr).round() / sr;
        let ev = hits(&svc, &sid, "k in");
        assert_eq!(ev.len(), 4, "round {round}: the edit is not lost: {ev:?}");
        for t in [0.5_f64, 1.0, 1.5] {
            let want = on_grid(t + d);
            assert!(
                ev.iter()
                    .any(|e| e["t"].as_f64().is_some_and(|x| (x - want).abs() < 1e-6)),
                "round {round}: the seeded hit at {t} sits at {want}: {ev:?}"
            );
        }
        assert!(
            ev.iter().any(|e| e["t"]
                .as_f64()
                .is_some_and(|x| (x - on_grid(0.25 + d)).abs() < 1e-6 || (x - 0.25).abs() < 1e-6)),
            "round {round}: the add survives: {ev:?}"
        );
    }
}
