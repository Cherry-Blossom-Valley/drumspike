//! Tests of the Learn filter's pure parts with planted faults (concept 3.3, 3.6).

use super::{
    EDITED_REACH, REACH, dropped_times, forced_this, forget_attack_gate, mark_out, note_dropped, refilter_merge,
    role_hits, threshold_of, undrop_at,
};
use serde_json::json;

fn frames(times: &[f64]) -> Vec<u32> {
    times.iter().map(|&t| (t * 100.0).round() as u32).collect()
}

fn det(times: &[f64]) -> Vec<(f64, u8)> {
    times.iter().map(|&t| (t, 40)).collect()
}

#[test]
fn a_forget_mark_takes_only_the_hits_with_its_attack() {
    // #293 (B8): the Forget mark attacks in 0.5 ms. Three hits score under the threshold (cos 0.975 to the mark): a
    // real stroke at 0.6 ms stays, bleed smeared over 12 ms goes, a hit at 0.2 ms is in the 2 ms reach and stays.
    let mut out = vec![true, true, true];
    forget_attack_gate(&mut out, &[0.6, 12.0, 0.2], &[0.5], &[false, false, false]);
    assert_eq!(out, vec![true, false, true]);
    // the reach grows with the mark: 50 % of 10 ms is 5 ms, so 14.9 ms stays and 15.1 ms goes
    let mut out = vec![true, true];
    forget_attack_gate(&mut out, &[14.9, 15.1], &[10.0], &[false, false]);
    assert_eq!(out, vec![true, false]);
    // a mark of the "not" examples itself always goes, whatever its attack
    let mut out = vec![true];
    forget_attack_gate(&mut out, &[60.0], &[0.5], &[true]);
    assert_eq!(out, vec![true]);
    // two marks: the span of their attacks (1 ms and 8 ms) widened at each end (2 ms, 4 ms): -1 ms to 12 ms
    let mut out = vec![true, true, true];
    forget_attack_gate(&mut out, &[-1.5, 11.9, 12.5], &[1.0, 8.0], &[false, false, false]);
    assert_eq!(out, vec![false, true, false]);
    // a hit the score did not mark stays unmarked
    let mut out = vec![false];
    forget_attack_gate(&mut out, &[0.5], &[0.5], &[false]);
    assert_eq!(out, vec![false]);
}

#[test]
fn the_kept_flag_is_the_gate_decision_on_a_forget_only_track() {
    // #293 (B8): pulses of 30 ms at 0.5 s (a stroke rising in 0.5 ms), 1.0 s (the Forget mark, 12 ms) and 1.5 s (bleed,
    // 12 ms). The mark's own hit always goes (kept: false); the bleed matches the mark's attack (kept: false); the
    // sharp stroke is off the mark's attack, so the gate keeps it out of the grey (kept: true).
    let sr: u32 = 48_000;
    let mut x = vec![0.0f32; 2 * sr as usize];
    for (t, rise_ms) in [(0.5, 0.5), (1.0, 12.0), (1.5, 12.0)] {
        let a = (t * f64::from(sr)) as usize;
        let n = rise_ms / 1000.0 * f64::from(sr);
        for (k, v) in x[a..a + (0.03 * f64::from(sr)) as usize].iter_mut().enumerate() {
            *v = (0.5 * (k as f64).min(n) / n) as f32;
        }
    }
    let times = [0.5, 1.0, 1.5];
    assert_eq!(
        super::attack_gate_kept(&x, sr, &times, &[], &[1.0]),
        vec![true, false, false]
    );
    // a "this" example turns the gate off: no candidate is kept by it
    assert_eq!(
        super::attack_gate_kept(&x, sr, &times, &[0.5], &[1.0]),
        vec![false, false, false]
    );
    // no "not" example: the gate does not run
    assert_eq!(
        super::attack_gate_kept(&x, sr, &times, &[], &[]),
        vec![false, false, false]
    );
}

#[test]
fn the_attack_profile_gives_the_times_and_the_window_the_gate_reads() {
    // #293 (B8) diagnosis: the same pulses as the kept-flag test. The profile's window is the mark's attack widened by
    // its reach, and a candidate is kept by the gate exactly when its attack lies outside that window.
    let sr: u32 = 48_000;
    let mut x = vec![0.0f32; 2 * sr as usize];
    for (t, rise_ms) in [(0.5, 0.5), (1.0, 12.0), (1.5, 12.0)] {
        let a = (t * f64::from(sr)) as usize;
        let n = rise_ms / 1000.0 * f64::from(sr);
        for (k, v) in x[a..a + (0.03 * f64::from(sr)) as usize].iter_mut().enumerate() {
            *v = (0.5 * (k as f64).min(n) / n) as f32;
        }
    }
    let times = [0.5, 1.0, 1.5];
    let p = super::attack_profile(&x, sr, &times, &[1.0]);
    assert_eq!((p.attacks.len(), p.marks_ms.len()), (3, 1));
    let m = p.marks_ms[0];
    let reach = 2.0f64.max(0.5 * m);
    assert!(
        (p.window.0 - (m - reach)).abs() < 1e-9 && (p.window.1 - (m + reach)).abs() < 1e-9,
        "{p:?}"
    );
    let kept = super::attack_gate_kept(&x, sr, &times, &[], &[1.0]);
    for (a, k) in p.attacks.iter().zip(&kept) {
        assert_eq!(
            *k,
            *a < p.window.0 || *a > p.window.1,
            "attack {a} ms, window {:?}",
            p.window
        );
    }
}

#[test]
fn a_hit_under_the_threshold_goes_out_and_one_over_it_stays() {
    let times = [0.5, 1.0];
    let out = mark_out(&det(&times), &frames(&times), &[0.9, 0.2], 0.5, &[], &[], &[]);
    assert_eq!(out, vec![false, true]);
    // planted fault: a threshold under both scores keeps both
    assert_eq!(
        mark_out(&det(&times), &frames(&times), &[0.9, 0.2], 0.1, &[], &[], &[]),
        vec![false, false]
    );
}

#[test]
fn a_hit_without_a_candidate_stays_in() {
    // a roll fill hit at 2.0 has no candidate frame: no score, never out
    let out = mark_out(&det(&[0.5, 2.0]), &frames(&[0.5]), &[0.2], 0.5, &[], &[], &[]);
    assert_eq!(out, vec![true, false]);
}

#[test]
fn a_this_example_and_an_edited_hit_are_never_out() {
    let times = [0.5, 1.0];
    let f = frames(&times);
    assert_eq!(
        mark_out(&det(&times), &f, &[0.1, 0.1], 0.5, &[0.51], &[], &[1.01]),
        vec![false, false]
    );
    // the edited reach is the merge window: an edited hit 25 ms away does not protect the hit
    const { assert!(EDITED_REACH < 0.025 && REACH > 0.025) };
    assert_eq!(
        mark_out(&det(&times), &f, &[0.1, 0.1], 0.5, &[], &[], &[1.025]),
        vec![true, true]
    );
}

#[test]
fn a_not_example_takes_its_nearest_hit_even_over_the_threshold() {
    let times = [0.5, 1.0, 1.02];
    let out = mark_out(&det(&times), &frames(&times), &[0.9, 0.9, 0.9], 0.5, &[], &[1.004], &[]);
    assert_eq!(out, vec![false, true, false]);
    // a not with no hit within the reach takes nothing
    assert_eq!(
        mark_out(&det(&times), &frames(&times), &[0.9, 0.9, 0.9], 0.5, &[], &[5.0], &[]),
        vec![false; 3]
    );
}

#[test]
fn forced_this_adds_the_candidate_of_an_example_the_pick_has_no_hit_for() {
    let cands = vec![
        json!([0.5, 2, 0.9, 30]),
        json!([1.0, 2, 0.1, 140]),
        json!([3.0, 2, 0.1, 140]),
    ];
    let d = det(&[0.5]);
    // 1.01 and 1.0 take the candidate at 1.0; 2.0 has none: the example's own time (it always yields a hit)
    assert_eq!(forced_this(&d, &cands, &[0.505, 1.01, 1.0, 2.0]), vec![1.0, 2.0]);
    assert!(forced_this(&d, &cands, &[0.5]).is_empty(), "the pick has the hit");
}

#[test]
fn the_refilter_keeps_deletions_velocities_and_edits_and_marks_the_rest() {
    let base = vec![
        json!({"t": 0.5, "v": 90}),
        json!({"t": 1.0, "v": 90, "filtered": true}),
        json!({"t": 1.5, "v": 90}),
        json!({"t": 2.0, "v": 90, "filtered": true}),
    ];
    // the engineer deleted 1.5, set 0.5 to 127, moved 2.0 to 2.004; 3.0 was a forced hit of an earlier step
    let current = vec![
        json!({"t": 0.5, "v": 127, "cut": true}),
        json!({"t": 1.0, "v": 90}),
        json!({"t": 2.004, "v": 90, "orig": 2.0, "filtered": true}),
        json!({"t": 3.0, "v": 80}),
    ];
    let forced = vec![json!({"t": 4.0, "v": 70})];
    let got = refilter_merge(&current, &base, &forced, &[]);
    assert_eq!(
        got,
        vec![
            json!({"t": 0.5, "v": 127, "cut": true}),
            json!({"t": 1.0, "v": 90, "filtered": true}),
            json!({"t": 2.004, "v": 90, "orig": 2.0}),
            json!({"t": 4.0, "v": 70}),
        ]
    );
    // planted fault: a forced hit next to an existing one does not come in twice
    let twice = refilter_merge(&got, &base, &[json!({"t": 1.01, "v": 70})], &[]);
    assert_eq!(twice.len(), 3, "{twice:?}");
}

#[test]
fn the_refilter_takes_the_add_rule_hits_of_the_base_in() {
    // P4-38d: a base hit of the Add rule (learned) with no current hit near comes in; a filtered one and one next to
    // a current hit do not; a plain base hit with no current hit stays out (a deletion stays)
    let base = vec![
        json!({"t": 0.5, "v": 90}),
        json!({"t": 1.0, "v": 60, "learned": true}),
        json!({"t": 1.5, "v": 90}),
        json!({"t": 2.0, "v": 60, "learned": true, "filtered": true}),
        json!({"t": 2.51, "v": 60, "learned": true}),
    ];
    let current = vec![json!({"t": 0.5, "v": 90}), json!({"t": 2.5, "v": 80, "added": true})];
    let got = refilter_merge(&current, &base, &[], &[]);
    assert_eq!(
        got,
        vec![
            json!({"t": 0.5, "v": 90}),
            json!({"t": 1.0, "v": 60, "learned": true}), // planted fault: the Add-rule hits stay out
            json!({"t": 2.5, "v": 80, "added": true}),
        ]
    );
}

#[test]
fn a_dropped_learn_hit_stays_out_of_the_refilter() {
    // P4-38e: an Add-rule or forced hit near a dropped time does not come back; the others do
    let base = vec![
        json!({"t": 1.0, "v": 60, "learned": true}),
        json!({"t": 2.0, "v": 60, "learned": true}),
    ];
    let forced = vec![json!({"t": 3.0, "v": 70})];
    let got = refilter_merge(&[], &base, &forced, &[1.005, 3.0]);
    assert_eq!(got, vec![json!({"t": 2.0, "v": 60, "learned": true})]);
}

#[test]
fn a_hand_delete_of_a_learn_hit_is_noted_and_its_undo_takes_it_out() {
    // P4-38e: the Add-rule hit at 1.0 and the forced hit at 3.0 (a "this" example at 2.99) are deleted, the plain
    // hit at 2.0 too (not a Learn hit: not noted); the times are stored before the Hitpoint shift (0.01 s)
    let mut learn = json!({"marks": [{"id": "m1", "kind": "this", "t0": 2.99, "t1": 2.99, "at": [2.99]}]})
        .as_object()
        .unwrap()
        .clone();
    let old = vec![
        json!({"t": 1.01, "v": 60, "learned": true}),
        json!({"t": 2.01, "v": 90}),
        json!({"t": 3.0, "v": 70}),
        json!({"t": 4.01, "v": 60, "learned": true}),
    ];
    let new = vec![old[3].clone()];
    assert!(note_dropped(&mut learn, &old, &new, 0.01));
    let d = dropped_times(&learn);
    assert_eq!(d.len(), 2, "{d:?}");
    assert!((d[0] - 1.0).abs() < 1e-9 && (d[1] - 2.99).abs() < 1e-9, "{d:?}");
    assert!(!note_dropped(&mut learn, &new, &new, 0.01), "no change, no write");
    // the undo puts the hit at 1.01 back: its time goes out; the other stays
    let back = vec![old[0].clone(), old[3].clone()];
    assert!(note_dropped(&mut learn, &new, &back, 0.01));
    assert_eq!(dropped_times(&learn).len(), 1);
    let all = vec![old[0].clone(), old[2].clone(), old[3].clone()];
    assert!(note_dropped(&mut learn, &back, &all, 0.01));
    assert!(learn.get("dropped").is_none(), "an empty list removes the key");
}

#[test]
fn k_on_a_dropped_time_takes_it_out() {
    // P4-38e: a K example within REACH of a dropped time takes that time out; a time farther away stays
    let mut learn = json!({"dropped": [1.0, 2.0]}).as_object().unwrap().clone();
    assert!(!undrop_at(&mut learn, &[1.5]), "no dropped time near: no change");
    assert!(undrop_at(&mut learn, &[1.0 + REACH * 0.5]));
    assert_eq!(dropped_times(&learn), vec![2.0]);
    assert!(undrop_at(&mut learn, &[2.0]));
    assert!(learn.get("dropped").is_none(), "an empty list removes the key");
}

#[test]
fn a_learn_step_keeps_the_flam_hits_and_they_follow_their_first_stroke() {
    // P4-13 stage B: the pick has no second stroke, so a flam hit has no base hit; the refilter keeps it (velocity and
    // all) and filters it exactly when the hit before it is filtered
    let base = vec![
        json!({"t": 1.0, "v": 90, "filtered": true}),
        json!({"t": 2.0, "v": 90}),
        json!({"t": 3.0, "v": 90}),
    ];
    let current = vec![
        json!({"t": 1.0, "v": 90}),
        json!({"t": 1.03, "v": 61, "flam": true}),
        json!({"t": 2.0, "v": 90}),
        json!({"t": 2.03, "v": 55, "flam": true, "filtered": true}),
        json!({"t": 3.0, "v": 90}),
        json!({"t": 3.2, "v": 50, "flam": true, "filtered": true}),
        json!({"t": 3.5, "v": 77, "flam": true, "added": true, "filtered": true}),
    ];
    let got = refilter_merge(&current, &base, &[], &[]);
    assert_eq!(
        got,
        vec![
            json!({"t": 1.0, "v": 90, "filtered": true}),
            json!({"t": 1.03, "v": 61, "flam": true, "filtered": true}),
            json!({"t": 2.0, "v": 90}),
            json!({"t": 2.03, "v": 55, "flam": true}),
            json!({"t": 3.0, "v": 90}),
            json!({"t": 3.2, "v": 50, "flam": true}),
            json!({"t": 3.5, "v": 77, "flam": true, "added": true}),
        ],
        "a flam hit is filtered with its first stroke and only then; an edited one is never filtered"
    );
    // planted fault: a flam hit whose first stroke is out of reach is not filtered by a filtered hit before it
    let far = refilter_merge(
        &[json!({"t": 1.0, "v": 90}), json!({"t": 1.2, "v": 60, "flam": true})],
        &[json!({"t": 1.0, "v": 90, "filtered": true})],
        &[],
        &[],
    );
    assert_eq!(far[1], json!({"t": 1.2, "v": 60, "flam": true}));
}

#[test]
fn the_threshold_defaults_to_the_similarity_default() {
    let learn = serde_json::Map::new();
    assert!((threshold_of(&learn) - 0.5).abs() < 1e-9);
    let mut learn = serde_json::Map::new();
    learn.insert("threshold".into(), json!(0.62));
    assert!((threshold_of(&learn) - 0.62).abs() < 1e-9);
}

#[test]
fn an_other_class_detection_near_an_example_neither_stands_in_nor_takes_the_not() {
    // a tom track (notes 45/47/50 in its classes); a snare detection (38) 2 ms from the tom hit at 1.0 and one at 2.0
    let raw = vec![(0.5, 47), (0.998, 38), (1.0, 47), (2.0, 38)];
    let (d, roll) = role_hits(&raw, &[], "tom");
    assert_eq!(d, vec![(0.5, 47), (1.0, 47)]);
    assert!(roll.is_empty());
    // a This example at 2.0 is forced (planted fault: the raw list's snare hit stands in and nothing is forced)
    let cands = vec![json!([2.0, 3, 0.3, 120])];
    assert_eq!(forced_this(&d, &cands, &[2.0]), vec![2.0]);
    assert!(
        forced_this(&raw, &cands, &[2.0]).is_empty(),
        "the raw list hides the example"
    );
    // a Not example at 0.997 takes the tom hit at 1.0 (planted fault: the raw list takes the snare at 0.998)
    let times: Vec<f64> = d.iter().map(|h| h.0).collect();
    assert_eq!(
        mark_out(&d, &frames(&times), &[0.9, 0.9], 0.5, &[], &[0.997], &[]),
        vec![false, true]
    );
    let rt: Vec<f64> = raw.iter().map(|h| h.0).collect();
    assert!(
        !mark_out(&raw, &frames(&rt), &[0.9; 4], 0.5, &[], &[0.997], &[])[2],
        "the raw list leaves the tom hit in"
    );
    // the roll flags follow the kept hits
    let (_, r) = role_hits(&raw, &[true, false, false, true], "tom");
    assert_eq!(r, vec![true, false]);
}

#[test]
fn add_threshold_defaults_to_0_85_and_follows_the_learn_object() {
    assert_eq!(super::add_threshold(json!({}).as_object().unwrap()), 0.85);
    assert_eq!(super::add_threshold(json!({"add": 0.6}).as_object().unwrap()), 0.6);
}

#[test]
fn add_times_takes_below_sens_candidates_over_th_and_skips_not_and_known() {
    // candidates at 0.5 (v 40: solid at sens 100? no, 40 <= 100 -> solid, never added),
    // 1.0 (v 120: below the Sensitivity, score 0.9 >= 0.85 -> added), 2.0 (v 130, score 0.5 -> not)
    let cands = vec![
        json!([0.5, 2, 30, 40.0]),
        json!([1.0, 2, 30, 120.0]),
        json!([2.0, 2, 30, 130.0]),
    ];
    let fr = frames(&[0.5, 1.0, 2.0]);
    let sc = [0.2, 0.9, 0.5];
    let got = super::add_times(&cands, &fr, &sc, 100.0, 0.85, &[], &[]);
    assert_eq!(got, vec![1.0]); // planted fault: the solid one or the low score fail this
    // a Forget example within 50 ms excludes the hit
    let got = super::add_times(&cands, &fr, &sc, 100.0, 0.85, &[], &[1.02]);
    assert!(got.is_empty());
    // a detection hit within REACH excludes it
    let got = super::add_times(&cands, &fr, &sc, 100.0, 0.85, &det(&[1.0]), &[]);
    assert!(got.is_empty());
}

#[test]
fn add_times_is_empty_without_below_sens_candidates() {
    let cands = vec![json!([1.0, 2, 30, 90.0])];
    let got = super::add_times(&cands, &frames(&[1.0]), &[0.99], 100.0, 0.85, &[], &[]);
    assert!(got.is_empty()); // v 90 <= sens 100: the detection has it already
}
