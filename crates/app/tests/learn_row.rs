//! #587: a Learn rule acts on the whole row. Two audio files of one DAW track hold the marks of each other through
//! `track_learn_adopt` (`learn.row`). All signals are synthetic (a kick, a sharp click, a snare-like burst); no session of
//! the owner is read. The standalone proofs: an empty `learn.row` takes the old path, and a track that adopts nothing keeps
//! every byte of its events.

mod common;

use common::*;
use drumspike_app::Service;
use serde_json::{Value, json};

/// The "bad" hit of the rule tests: a thump of a higher pitch sweep (180 -> 110 Hz) and a shorter decay, as the bleed of a
/// tom into a kick microphone. It is a hit for the kick role, but its shape is not the kick's.
fn click_signal(onsets: &[f64], dur: f64, sr: u32) -> Vec<f32> {
    let srf = f64::from(sr);
    let mut x = vec![0.0f64; (dur * srf) as usize];
    for &t0 in onsets {
        let i0 = (t0 * srf).round() as usize;
        let mut cum = 0.0;
        for k in 0..((0.25 * srf) as usize) {
            if i0 + k >= x.len() {
                break;
            }
            let t = k as f64 / srf;
            cum += 110.0 + 70.0 * (-t / 0.02).exp();
            let env = (t / 0.0005).min(1.0) * (-t / 0.05).exp();
            x[i0 + k] += (2.0 * std::f64::consts::PI * cum / srf).sin() * env * 0.5;
        }
    }
    x.iter().map(|&v| v as f32).collect()
}

fn mix(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b).map(|(x, y)| x + y).collect()
}

/// `n` kicks, a click between every kick: the kick onsets and the click onsets (seconds).
fn grid(n: usize, step: f64) -> (Vec<f64>, Vec<f64>) {
    let kicks: Vec<f64> = (0..n).map(|i| 0.25 + step * i as f64).collect();
    let clicks: Vec<f64> = (0..n).map(|i| 0.25 + step * (i as f64 + 0.5)).collect();
    (kicks, clicks)
}

struct Row {
    dir: std::path::PathBuf,
    svc: Service,
    sid: String,
    kicks: Vec<f64>,
    clicks: Vec<f64>,
}

/// Two files of one DAW track, "a" at `sr_a` and "b" at `sr_b`: each holds `n` kicks and `n` clicks (the files are two
/// microphones of the same drum: the same pattern, a different noise seed is not needed, the click is the bleed).
fn row(tag: &str, n: usize, step: f64, sr_a: u32, sr_b: u32, b_only: Option<bool>) -> Row {
    let root = tmp(tag);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let (kicks, clicks) = grid(n, step);
    let dur = kicks.last().unwrap() + 1.5;
    // `Some(true)`: only thumps, `Some(false)`: only kicks, `None`: both
    let file = |sr: u32, only: Option<bool>| {
        let c = click_signal(&clicks, dur, sr);
        let k = kick_signal(&kicks, dur, sr);
        match only {
            Some(true) => c,
            Some(false) => k,
            None => mix(&k, &c),
        }
    };
    let specs = [
        Spec {
            name: "a",
            role: "kick",
            note: 36,
            x: file(sr_a, None),
            sr: sr_a,
        },
        Spec {
            name: "b",
            role: "kick",
            note: 36,
            x: file(sr_b, b_only),
            sr: sr_b,
        },
    ];
    let dir = project(&root, "Row", &specs);
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    Row {
        dir,
        svc,
        sid,
        kicks,
        clicks,
    }
}

fn events(r: &Row, name: &str) -> Vec<Value> {
    r.svc.events_get(&r.sid, name).unwrap()["events"]
        .as_array()
        .unwrap()
        .clone()
}

fn is_out(e: &Value) -> bool {
    e["filtered"] == true
}

fn near(e: &Value, ts: &[f64]) -> bool {
    let t = e["t"].as_f64().unwrap();
    ts.iter().any(|x| (x - t).abs() < 0.02)
}

/// The first kick and the first thump of a file as hit times, taken from the stored hits.
fn first_of(r: &Row, name: &str, ts: &[f64]) -> f64 {
    events(r, name)
        .iter()
        .find(|e| near(e, ts))
        .map(|e| e["t"].as_f64().unwrap())
        .unwrap()
}

/// Learn This on a kick and Forget This on a thump of the file `name`: the rule of the DAW track.
fn make_rule(r: &Row, name: &str) -> Value {
    r.svc
        .track_learn_mark(&r.sid, name, "this", None, Some(first_of(r, name, &r.kicks)))
        .unwrap();
    r.svc
        .track_learn_mark(&r.sid, name, "not", None, Some(first_of(r, name, &r.clicks)))
        .unwrap()
}

fn out_of(r: &Row, name: &str, ts: &[f64]) -> usize {
    events(r, name).iter().filter(|e| is_out(e) && near(e, ts)).count()
}

fn learn_of(r: &Row, name: &str) -> Value {
    r.svc.tracks_get(&r.sid).unwrap()["tracks"][name]["learn"].clone()
}

/// #587 / R7, same instrument: a Forget made on file a takes the same kind of hit out of file b. Hit counts per file
/// before and after; no kick of either file is lost.
#[test]
fn a_forget_on_one_file_takes_the_same_kind_of_hit_out_of_the_other_file_of_the_row() {
    let r = row("lr_same", 300, 0.2, 48_000, 48_000, None);
    let (a0, b0) = (events(&r, "a"), events(&r, "b"));
    let n = (a0.len(), b0.len());
    assert!(n.0 >= 600 && n.1 >= 600, "every kick and every thump is a hit: {n:?}");
    make_rule(&r, "a");
    assert_eq!(out_of(&r, "a", &r.clicks), 300, "file a: every thump is out");
    assert_eq!(out_of(&r, "b", &r.clicks), 0, "file b has no rule yet");
    let ad = r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    assert_eq!(ad["adopted"], 2, "{ad}");
    let (a1, b1) = (events(&r, "a"), events(&r, "b"));
    eprintln!(
        "#587 same instrument: file a {} hits, thumps out 0 -> {}; file b {} hits, thumps out 0 -> {}; kicks out {} / {}",
        a1.len(),
        out_of(&r, "a", &r.clicks),
        b1.len(),
        out_of(&r, "b", &r.clicks),
        out_of(&r, "a", &r.kicks),
        out_of(&r, "b", &r.kicks)
    );
    assert_eq!(out_of(&r, "b", &r.clicks), 300, "file b: every thump is out too");
    assert_eq!(
        (out_of(&r, "a", &r.kicks), out_of(&r, "b", &r.kicks)),
        (0, 0),
        "no kick is lost"
    );
    assert_eq!(
        (a1.len(), b1.len()),
        n,
        "a hit is marked, never dropped from the document"
    );
    // the times of every hit are the times before
    let t = |ev: &[Value]| ev.iter().map(|e| e["t"].as_f64().unwrap()).collect::<Vec<_>>();
    assert_eq!(t(&b0), t(&b1));
    // the answer carries the guard counts of the file
    assert_eq!(
        (ad["learn_out_before"].clone(), ad["learn_out_after"].clone()),
        (json!(0), json!(300))
    );
    assert_eq!(ad["hits"], n.1);
}

/// #587 / R7, two sample rates for the same instrument.
#[test]
fn the_rule_acts_on_a_file_of_another_sample_rate() {
    let r = row("lr_rates", 120, 0.2, 44_100, 48_000, None);
    let n = (events(&r, "a").len(), events(&r, "b").len());
    assert!(n.0 >= 240 && n.1 >= 240, "{n:?}");
    make_rule(&r, "a");
    r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    eprintln!(
        "#587 44.1 kHz rule on 48 kHz file: thumps out {} of 120, kicks out {}",
        out_of(&r, "b", &r.clicks),
        out_of(&r, "b", &r.kicks)
    );
    assert_eq!(out_of(&r, "b", &r.clicks), 120);
    assert_eq!(out_of(&r, "b", &r.kicks), 0);
    // and the other way round
    let r = row("lr_rates2", 120, 0.2, 48_000, 44_100, None);
    make_rule(&r, "b");
    r.svc.track_learn_adopt(&r.sid, "a", "b", None).unwrap();
    assert_eq!((out_of(&r, "a", &r.clicks), out_of(&r, "a", &r.kicks)), (120, 0));
}

/// One rule of file a (Learn This on `this_ts`, Forget This on `not_ts`) copied to a file b that holds only `b_only`
/// (`Some(true)` thumps, `Some(false)` kicks). Returns the guard numbers of b and checks that one undo (the stored learn
/// and hits of each file, as the page's group step restores them) brings every hit of both files back, byte-equal.
fn other_instrument(tag: &str, b_only: bool, kick_is_this: bool) -> (u64, u64) {
    let r = row(tag, 100, 0.3, 48_000, 48_000, Some(b_only));
    let (a0, b0) = (events(&r, "a"), events(&r, "b"));
    let (la0, lb0) = (learn_of(&r, "a"), learn_of(&r, "b"));
    let (good, bad) = if kick_is_this {
        (&r.kicks, &r.clicks)
    } else {
        (&r.clicks, &r.kicks)
    };
    r.svc
        .track_learn_mark(&r.sid, "a", "this", None, Some(first_of(&r, "a", good)))
        .unwrap();
    let m = r
        .svc
        .track_learn_mark(&r.sid, "a", "not", None, Some(first_of(&r, "a", bad)))
        .unwrap();
    let ad = r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    eprintln!(
        "#587 other instrument (file b only {}): file a out {} -> {} of {}; file b out {} -> {} of {}",
        if b_only { "thumps" } else { "kicks" },
        m["learn_out_before"],
        m["learn_out_after"],
        m["hits"],
        ad["learn_out_before"],
        ad["learn_out_after"],
        ad["hits"]
    );
    // undo: each file back to the learn and the hits it had before
    r.svc
        .track_learn_set_events(&r.sid, "b", &lb0, Some(&Value::Array(b0.clone())))
        .unwrap();
    r.svc
        .track_learn_set_events(&r.sid, "a", &la0, Some(&Value::Array(a0.clone())))
        .unwrap();
    assert_eq!(events(&r, "a"), a0, "file a: byte-equal hits");
    assert_eq!(events(&r, "b"), b0, "file b: byte-equal hits");
    assert_eq!(learn_of(&r, "b"), lb0);
    (ad["learn_out_after"].as_u64().unwrap(), ad["hits"].as_u64().unwrap())
}

/// #587 / R7, different instruments on one row: a file of kicks only and a file of thumps only get the rule of a file
/// that holds both. The guard numbers of the answer tell what the rule does to each file (printed in the report), and one
/// undo brings every hit of both files back.
#[test]
fn a_rule_of_another_instrument_is_visible_in_the_guard_counts_and_one_undo_brings_the_hits_back() {
    // thumps only, rule: kicks good, thumps out
    let (out, hits) = other_instrument("lr_other_t", true, true);
    // kicks only, rule: thumps good, kicks out
    let (out2, hits2) = other_instrument("lr_other_k", false, false);
    assert!(hits >= 100 && hits2 >= 100);
    // the rule of a file that holds both takes nothing out of a file whose hits are all alike (its own mean is the pattern)
    assert_eq!(out, 0, "thumps only: nothing is taken out");
    // but it can take EVERY hit out of a file of the other instrument; the guard of the page sees it (more than max(10, 25 %))
    assert!(out2 as f64 >= 0.9 * hits2 as f64, "kicks only: {out2} of {hits2} out");
    assert!(out2 as f64 > f64::max(10.0, 0.25 * hits2 as f64));
}

/// #587 / R4: a copy of a "not" example never takes out a hit the user placed or moved in this file.
#[test]
fn an_adopted_not_example_never_takes_out_an_edited_or_added_hit() {
    let r = row("lr_edited", 60, 0.3, 48_000, 48_000, None);
    let b = events(&r, "b");
    let thump = b.iter().position(|e| near(e, &r.clicks)).unwrap();
    let other = b.iter().rposition(|e| near(e, &r.clicks)).unwrap();
    let to = b[thump]["t"].as_f64().unwrap() + 0.001;
    r.svc
        .events_edit(&r.sid, "b", &[json!({"op": "move", "index": thump, "t": to})])
        .unwrap();
    // a hit the user adds in a quiet place of file b
    let added_t = r.kicks.last().unwrap() + 1.0;
    r.svc
        .events_edit(&r.sid, "b", &[json!({"op": "add", "t": added_t})])
        .unwrap();
    make_rule(&r, "a");
    r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    let ev = events(&r, "b");
    let moved = ev
        .iter()
        .find(|e| (e["t"].as_f64().unwrap() - to).abs() < 1e-6)
        .expect("the moved hit stays");
    assert!(!is_out(moved), "an edited hit is never taken out: {moved}");
    let added = ev
        .iter()
        .find(|e| (e["t"].as_f64().unwrap() - added_t).abs() < 1e-3)
        .expect("the added hit stays");
    assert!(!is_out(added), "{added}");
    // the unedited thumps of b did go
    let t_other = b[other]["t"].as_f64().unwrap();
    assert!(is_out(
        ev.iter()
            .find(|e| (e["t"].as_f64().unwrap() - t_other).abs() < 1e-6)
            .unwrap()
    ));
}

/// #587 / R11: the command validates its inputs; a refused call changes nothing.
#[test]
fn a_refused_adopt_changes_nothing() {
    let r = row("lr_refuse", 20, 0.3, 48_000, 48_000, None);
    make_rule(&r, "a");
    let (la, lb) = (learn_of(&r, "a"), learn_of(&r, "b"));
    let dirty = |r: &Row| r.svc.project_get(&r.sid).unwrap()["dirty"].clone();
    let before = (events(&r, "b"), lb.clone(), dirty(&r));
    for (name, from, ids, code) in [
        ("b", "b", None, 400),
        ("b", "nope", None, 400),
        ("nope", "a", None, 400),
        ("b", "a", Some(vec!["m99".to_string()]), 400),
    ] {
        let e = r.svc.track_learn_adopt(&r.sid, name, from, ids.as_deref()).unwrap_err();
        assert_eq!(e.code, code, "{name} from {from}: {e:?}");
    }
    assert!(r.svc.track_learn_adopt("zz", "b", "a", None).is_err());
    assert_eq!((events(&r, "b"), learn_of(&r, "b"), dirty(&r)), before);
    assert_eq!(learn_of(&r, "a"), la);
    assert_eq!(lb, Value::Null);
}

/// #587: a copy that exists is skipped; the copies are a valid learn object (the page's undo stores it); Forget Examples takes them.
#[test]
fn adopt_is_repeatable_the_copies_are_a_valid_learn_object_and_go_with_forget() {
    let r = row("lr_repeat", 30, 0.3, 48_000, 48_000, None);
    make_rule(&r, "a");
    let first = r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    assert_eq!(first["adopted"], 2);
    let again = r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    assert_eq!(again["adopted"], 0, "a copy that exists is skipped");
    let row_entries = learn_of(&r, "b")["row"].as_array().unwrap().clone();
    assert_eq!(row_entries.len(), 2);
    assert_eq!(row_entries[0]["from"], "a");
    assert_eq!(
        learn_of(&r, "b")["on"],
        true,
        "the first Forget example turns the filter on, as on one file"
    );
    // the filter runs again after a re-analyze (the stored pick reads learn.row)
    r.svc.analyze_start(&r.sid, None, None, None).unwrap();
    wait_done(&r.svc, &r.sid);
    assert_eq!(out_of(&r, "b", &r.clicks), 30, "Re-analyze keeps the rule of the row");
    // check_learn refuses a wrong row
    let mut l = learn_of(&r, "b");
    l["row"][0]["fps"] = json!("x");
    assert!(r.svc.track_learn_set(&r.sid, "b", &l).is_err());
    // Forget Examples of Track takes the copies
    let f = r.svc.track_learn_forget(&r.sid, "b").unwrap();
    assert_eq!(f["examples"], 2);
    assert!(learn_of(&r, "b").get("row").is_none() || learn_of(&r, "b")["row"].is_null());
    assert_eq!(out_of(&r, "b", &r.clicks), 0);
}

/// #587 / R10: a project nobody adopted into takes the old path: a learn object with an empty `row` gives the scores and
/// the hits of one without the key, and adopting into the other file changes no byte of the first file's events or MIDI.
#[test]
fn a_file_without_copies_is_unchanged_and_adopting_leaves_the_origin_alone() {
    let r = row("lr_standalone", 40, 0.3, 48_000, 48_000, None);
    make_rule(&r, "a");
    let ev_a = events(&r, "a");
    let scores_a = r.svc.track_learn_scores(&r.sid, "a").unwrap();
    let midi_a = r.svc.export_midi_stored(&r.sid, "a", None, None, None, None).unwrap();
    // the same rule with an explicit empty row
    let mut l = learn_of(&r, "a");
    l["row"] = json!([]);
    r.svc.track_learn_set(&r.sid, "a", &l).unwrap();
    assert_eq!(events(&r, "a"), ev_a, "an empty row is the old path");
    let again = r.svc.track_learn_scores(&r.sid, "a").unwrap();
    assert_eq!(again["candidates"], scores_a["candidates"]);
    assert_eq!(again["threshold"], scores_a["threshold"]);
    // adopting into b writes only b
    let mut l = learn_of(&r, "a");
    l.as_object_mut().unwrap().remove("row");
    r.svc.track_learn_set(&r.sid, "a", &l).unwrap();
    r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    assert_eq!(events(&r, "a"), ev_a, "the origin file has the same hits");
    let midi_after = r.svc.export_midi_stored(&r.sid, "a", None, None, None, None).unwrap();
    assert_eq!(midi_after["n_events"], midi_a["n_events"]);
    assert_eq!(learn_of(&r, "a")["marks"], scores_marks(&l));
}

fn scores_marks(l: &Value) -> Value {
    l["marks"].clone()
}

/// #587 / R8: the time of one Learn step on a row of two dense files of 10 minutes (run by hand in a release build:
/// `cargo test -p drumspike-app --release --test learn_row -- --ignored --nocapture`).
#[test]
#[ignore = "a measurement: two files of 10 minutes"]
fn measure_one_learn_step_on_two_dense_files_of_ten_minutes() {
    use std::time::Instant;
    let r = row("lr_ten_minutes", 1500, 0.2, 48_000, 48_000, None);
    eprintln!(
        "#587 10 min: {} + {} hits",
        events(&r, "a").len(),
        events(&r, "b").len()
    );
    r.svc
        .track_learn_mark(&r.sid, "a", "this", None, Some(first_of(&r, "a", &r.kicks)))
        .unwrap();
    let t = Instant::now();
    let m = r
        .svc
        .track_learn_mark(&r.sid, "a", "not", None, Some(first_of(&r, "a", &r.clicks)))
        .unwrap();
    let mark_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let ad = r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    let adopt_ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "#587 one Learn step on file a (mark): {mark_ms:.0} ms; adopt into file b: {adopt_ms:.0} ms (out {} -> {})",
        m["learn_out_after"], ad["learn_out_after"]
    );
}

/// #587: the last mark wins on a copy too: Forget This and then Learn This on the same hit of file a leave file b one example
/// of it (the "this" one), not both.
#[test]
fn a_hit_marked_again_on_the_other_file_replaces_its_copy() {
    let r = row("lr_flip", 20, 0.3, 48_000, 48_000, None);
    let t = first_of(&r, "a", &r.clicks);
    r.svc.track_learn_mark(&r.sid, "a", "not", None, Some(t)).unwrap();
    r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    let m = r.svc.track_learn_mark(&r.sid, "a", "this", None, Some(t)).unwrap();
    assert_eq!(m["flipped"], 1, "{m}");
    let id = m["mark"]["id"].as_str().unwrap().to_string();
    r.svc.track_learn_adopt(&r.sid, "b", "a", Some(&[id])).unwrap();
    let row_b = learn_of(&r, "b")["row"].as_array().unwrap().clone();
    assert_eq!(row_b.len(), 1, "{row_b:?}");
    assert_eq!(row_b[0]["kind"], "this");
}

/// #587 review F1: the copies survive a save and a reopen. `project_open_with` runs `migrate_learn`, which switched the
/// filter off for a file whose own "not" examples are none: a file that only holds copies lost its rule silently.
#[test]
fn the_rule_of_a_file_that_only_holds_copies_survives_save_and_reopen() {
    let r = row("lr_reopen", 40, 0.3, 48_000, 48_000, None);
    make_rule(&r, "a");
    r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    let (learn, ev) = (learn_of(&r, "b"), events(&r, "b"));
    assert_eq!(learn["on"], true);
    assert_eq!(out_of(&r, "b", &r.clicks), 40);
    r.svc.project_save(&r.sid).unwrap();
    r.svc.project_close(&r.sid, true).unwrap();
    let sid = r.svc.project_open_with(&r.dir.display().to_string(), false).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let after = r.svc.tracks_get(&sid).unwrap()["tracks"]["b"]["learn"].clone();
    assert_eq!(after, learn, "the learn object of file b is as it was saved");
    assert_eq!(
        r.svc.events_get(&sid, "b").unwrap()["events"]
            .as_array()
            .unwrap()
            .clone(),
        ev,
        "and so are its hits"
    );
}

/// #587 review F2: the cap counts own examples plus copies on every path (mark, flip, adopt, check).
#[test]
fn an_own_mark_counts_the_copies_against_the_cap() {
    let r = row("lr_cap", 20, 0.3, 48_000, 48_000, None);
    let dim = drumspike_core::similarity::Recipe::Patch.len();
    let at: Vec<f64> = (0..40).map(|i| 100.0 + f64::from(i)).collect();
    let learn = json!({"on": false, "threshold": 0.5, "auto": true, "method": "patch-2",
        "marks": [{"id": "m1", "kind": "this", "t0": 100.0, "t1": 140.0, "at": at}],
        "row": [{"id": "m1", "from": "a", "kind": "this", "fps": vec![vec![0.1; dim]; 24], "atk": vec![0.0; 24]}]});
    r.svc.track_learn_set(&r.sid, "b", &learn).unwrap();
    assert_eq!(
        learn_of(&r, "b")["row"].as_array().unwrap().len(),
        1,
        "40 own + 24 copied = 64 is stored"
    );
    let t = first_of(&r, "b", &r.kicks);
    let e = r.svc.track_learn_mark(&r.sid, "b", "this", None, Some(t)).unwrap_err();
    assert!(e.message.contains("at most 64 this examples"), "{e:?}");
    assert_eq!(learn_of(&r, "b"), learn, "a refused mark changes nothing");
    // the other kind has room
    assert!(r.svc.track_learn_mark(&r.sid, "b", "not", None, Some(t)).is_ok());
}

/// #587 review F4: a copied "not" example counts as an earlier "not": the filter that the user switched off stays off at the
/// first own Forget This.
#[test]
fn an_own_forget_does_not_switch_a_filter_on_that_the_user_switched_off_when_copies_exist() {
    let r = row("lr_off", 20, 0.3, 48_000, 48_000, None);
    make_rule(&r, "a");
    r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    let mut l = learn_of(&r, "b");
    l["on"] = json!(false);
    r.svc.track_learn_set(&r.sid, "b", &l).unwrap();
    let t = first_of(&r, "b", &r.clicks);
    let m = r.svc.track_learn_mark(&r.sid, "b", "not", None, Some(t)).unwrap();
    assert_eq!(m["filter_on"], false, "{m}");
    assert_eq!(learn_of(&r, "b")["on"], false);
}

/// #587 review F5: a copy of a mark that the origin file removed goes at the next adopt (the page's Apply), also when there is
/// nothing new to copy.
#[test]
fn adopt_takes_back_the_copy_of_a_mark_that_the_origin_removed() {
    let r = row("lr_stale", 20, 0.3, 48_000, 48_000, None);
    make_rule(&r, "a");
    r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    assert_eq!(learn_of(&r, "b")["row"].as_array().unwrap().len(), 2);
    r.svc.track_learn_remove(&r.sid, "a", "m2", None).unwrap();
    assert_eq!(
        learn_of(&r, "b")["row"].as_array().unwrap().len(),
        2,
        "removing the origin's mark touches b not yet"
    );
    let ad = r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    assert_eq!(ad["adopted"], 0);
    let row_b = learn_of(&r, "b")["row"].as_array().unwrap().clone();
    assert_eq!(row_b.len(), 1, "{row_b:?}");
    assert_eq!(row_b[0]["id"], "m1");
}

/// #587 review M1: the bytes of 64 copied examples (run by hand: `cargo test -p drumspike-app --test learn_row -- --ignored --nocapture size`).
#[test]
#[ignore = "a measurement"]
fn size_of_the_copies() {
    let r = row("lr_size", 80, 0.3, 48_000, 48_000, None);
    let last = r.kicks[31] + 0.1;
    let m = r
        .svc
        .track_learn_mark(&r.sid, "a", "this", Some((0.0, last)), None)
        .unwrap();
    eprintln!(
        "#587 size: marked {} examples on a",
        m["mark"]["at"].as_array().unwrap().len()
    );
    let ad = r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    let with = learn_of(&r, "b");
    let n: usize = with["row"][0]["fps"].as_array().unwrap().len();
    let mut without = with.clone();
    without.as_object_mut().unwrap().remove("row");
    let ans_with = ad.to_string().len();
    let set = r.svc.track_learn_set(&r.sid, "b", &without).unwrap();
    let ans_without = set.to_string().len();
    r.svc.track_learn_set(&r.sid, "b", &with).unwrap();
    r.svc.project_save(&r.sid).unwrap();
    let file = |d: &std::path::Path| {
        std::fs::read_dir(d)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.path().extension().is_some_and(|x| x == "drumspike"))
            .map(|e| std::fs::metadata(e.path()).unwrap().len())
            .unwrap()
    };
    let f_with = file(&r.dir);
    r.svc.track_learn_set(&r.sid, "b", &without).unwrap();
    r.svc.project_save(&r.sid).unwrap();
    let f_without = file(&r.dir);
    eprintln!(
        "#587 size: {n} copied examples; learn JSON {} bytes with, {} without; one Learn answer {ans_with} with, {ans_without} without; project file {f_with} with, {f_without} without",
        with.to_string().len(),
        without.to_string().len()
    );
}

/// #587 review F7: a partial Remove Example on the origin leaves the copy valid for the rest; a later adopt with a subset of ids
/// copies the changed mark again, so file b follows the current mark and loses nothing.
#[test]
fn a_partial_remove_example_is_copied_again_with_the_next_adopt() {
    let r = row("lr_partial", 20, 0.3, 48_000, 48_000, None);
    let (t0, t1) = (r.kicks[0] - 0.05, r.kicks[2] + 0.05);
    let m = r
        .svc
        .track_learn_mark(&r.sid, "a", "this", Some((t0, t1)), None)
        .unwrap();
    let id = m["mark"]["id"].as_str().unwrap().to_string();
    let ats: Vec<f64> = m["mark"]["at"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert!(ats.len() >= 3, "{ats:?}");
    r.svc.track_learn_adopt(&r.sid, "b", "a", None).unwrap();
    let n_before = learn_of(&r, "b")["row"][0]["fps"].as_array().unwrap().len();
    r.svc.track_learn_remove(&r.sid, "a", &id, Some(ats[1])).unwrap();
    let m2 = r
        .svc
        .track_learn_mark(&r.sid, "a", "not", None, Some(first_of(&r, "a", &r.clicks)))
        .unwrap();
    let id2 = m2["mark"]["id"].as_str().unwrap().to_string();
    r.svc
        .track_learn_adopt(&r.sid, "b", "a", Some(std::slice::from_ref(&id2)))
        .unwrap();
    let row_b = learn_of(&r, "b")["row"].as_array().unwrap().clone();
    let copy = |i: &str| row_b.iter().find(|e| e["id"] == i).cloned();
    let old = copy(&id).expect("the copy of the changed mark is still there");
    assert_eq!(
        old["fps"].as_array().unwrap().len(),
        n_before - 1,
        "and it follows the origin: one example less"
    );
    assert!(copy(&id2).is_some(), "the new mark is copied");
}
