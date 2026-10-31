use super::*;

fn ev(t: f64, v: i64) -> Value {
    json!({"t": t, "v": v})
}

#[test]
fn apply_ops_adds_moves_deletes_and_sets_velocities() {
    let list = vec![ev(0.5, 80), ev(1.0, 90), ev(1.5, 100)];
    let ops = [
        json!({"op": "add", "t": 0.25}),
        json!({"op": "move", "index": 0, "t": 0.6}),
        json!({"op": "delete", "index": 2}),
        json!({"op": "velocity", "index": 1, "v": 64}),
    ];
    let (out, c) = apply_ops(list, &ops, Some(100), None).unwrap();
    assert_eq!(c, [1, 1, 1, 1, 0, 0]);
    assert_eq!(
        out,
        vec![
            json!({"t": 0.25, "v": 100, "added": true}),
            json!({"t": 0.6, "v": 80, "orig": 0.5}),
            json!({"t": 1.0, "v": 64}),
        ]
    );
}

#[test]
fn apply_ops_keeps_the_first_orig_and_drops_cut_from_edited_hits() {
    let list = vec![
        json!({"t": 0.5, "v": 80, "orig": 0.4, "cut": true}),
        json!({"t": 1.0, "v": 80, "cut": true}),
    ];
    let ops = [
        json!({"op": "move", "index": 0, "t": 0.7}),
        json!({"op": "velocity", "index": 1, "v": 70}),
    ];
    let (out, _) = apply_ops(list, &ops, None, None).unwrap();
    assert_eq!(out[0], json!({"t": 0.7, "v": 80, "orig": 0.4}));
    assert_eq!(out[1]["cut"], true, "an unmoved detected hit keeps its cut mark");
}

#[test]
fn restore_and_an_edit_clear_the_tom_bleed_mark() {
    let list = vec![
        json!({"t": 0.5, "v": 80, "bleed": true}),
        json!({"t": 1.0, "v": 80, "bleed": true}),
        json!({"t": 1.5, "v": 80, "bleed": true}),
        json!({"t": 2.0, "v": 80}),
    ];
    let ops = [
        json!({"op": "restore", "index": 0}),
        json!({"op": "move", "index": 1, "t": 1.1}),
        json!({"op": "restore", "index": 3}),
    ];
    let (out, c) = apply_ops(list, &ops, None, None).unwrap();
    assert_eq!(c, [0, 0, 1, 0, 1, 0], "a restore of an unmarked hit counts nothing");
    assert_eq!(out[0], json!({"t": 0.5, "v": 80, "edited": true}), "restored and kept");
    assert_eq!(
        out[1],
        json!({"t": 1.1, "v": 80, "orig": 1.0}),
        "a moved hit loses the mark"
    );
    assert_eq!(out[2]["bleed"], true, "an untouched hit keeps it");
    assert_eq!(out[3], json!({"t": 2.0, "v": 80, "edited": true}));
    // a kept hit stays through a merge, and a mark on it goes at the next edit
    let merged = merge_edited(vec![json!({"t": 0.5, "v": 60}), json!({"t": 3.0, "v": 60})], &out);
    assert!(
        merged.contains(&json!({"t": 0.5, "v": 80, "edited": true})),
        "{merged:?}"
    );
    assert!(
        !merged.contains(&json!({"t": 0.5, "v": 60})),
        "the pick at a kept hit goes"
    );
    let (again, _) = apply_ops(
        vec![json!({"t": 0.5, "edited": true, "bleed": true})],
        &[json!({"op": "velocity", "index": 0, "v": 9})],
        None,
        None,
    )
    .unwrap();
    assert!(again[0].get("bleed").is_none());
    assert!(is_kept(json!({"t": 1, "edited": true}).as_object().unwrap()));
    assert!(!is_kept(json!({"t": 1, "edited": false}).as_object().unwrap()));
    let e = apply_ops(
        vec![json!({"t": 0.5})],
        &[json!({"op": "restore", "index": 1})],
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, 400);
}

#[test]
fn apply_ops_refuses_a_hit_after_the_end_of_the_track() {
    let list = vec![ev(0.5, 80)];
    for op in [
        json!({"op": "add", "t": 3.6}),
        json!({"op": "add", "t": 1e7}),
        json!({"op": "move", "index": 0, "t": 3.6}),
    ] {
        let e = apply_ops(list.clone(), std::slice::from_ref(&op), None, Some(3.5)).unwrap_err();
        assert_eq!(e.code, 400, "{op}");
        assert!(e.message.contains("after the end of the track"), "{}", e.message);
    }
    let (out, c) = apply_ops(list, &[json!({"op": "add", "t": 3.5})], None, Some(3.5)).unwrap();
    assert_eq!((out.len(), c[0]), (2, 1), "the last sample time is inside");
}

#[test]
fn track_end_reads_the_duration_or_the_samples() {
    let mut tr = drumspike_core::project::Track::default();
    assert_eq!(track_end(&tr), None);
    tr.n_samples = Some(96_000);
    tr.sr = Some(48_000);
    assert_eq!(track_end(&tr), Some(2.0));
    tr.duration = Some(3.5);
    assert_eq!(track_end(&tr), Some(3.5));
}

#[test]
fn apply_ops_refuses_bad_ops_and_changes_nothing() {
    let list = vec![ev(0.5, 80)];
    for bad in [
        json!({"op": "move", "index": 3, "t": 1.0}),
        json!({"op": "velocity", "index": 0, "v": 0}),
        json!({"op": "velocity", "index": 0}),
        json!({"op": "add", "t": -1.0}),
        json!({"op": "nudge", "index": 0}),
        json!("add"),
    ] {
        assert_eq!(apply_ops(list.clone(), &[bad], None, None).unwrap_err().code, 400);
    }
    let twice = [
        json!({"op": "delete", "index": 0}),
        json!({"op": "move", "index": 0, "t": 1.0}),
    ];
    assert_eq!(apply_ops(list, &twice, None, None).unwrap_err().code, 400);
}

#[test]
fn dispatch_knows_its_commands_and_checks_the_arguments() {
    let svc = Service::new(crate::ServiceConfig {
        refine_rule: None,
        bleed_rule: None,
        flam_rule: None,
        cross_bleed: None,
        ghost_gate: None,
    });
    assert!(dispatch(&svc, "project_list", &json!({})).is_none());
    assert_eq!(
        dispatch(&svc, "events_edit", &json!({"sid": "x", "name": "n"}))
            .unwrap()
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        dispatch(
            &svc,
            "events_edit",
            &json!({"sid": "x", "name": "n", "ops": [{"op": "add", "t": 1.0}]})
        )
        .unwrap()
        .unwrap_err()
        .code,
        404
    );
    assert_eq!(
        dispatch(&svc, "track_shift_set", &json!({"sid": "x", "name": "n"}))
            .unwrap()
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(EDIT_COMMANDS, ["events_edit", "track_shift_set"]);
}

#[test]
fn merge_edited_follows_the_fixture_cases() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/agent-parity/merge_edited.json");
    let fx: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let cases = fx["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 4);
    for c in cases {
        let list = |k: &str| c[k].as_array().unwrap().clone();
        let got = merge_edited(list("picked"), &list("current"));
        assert_eq!(got, list("expected"), "{}", c["name"]);
    }
}

#[test]
fn is_kept_follows_the_shared_predicate_table() {
    // R04 (#237) ticket #250: one table drives the Rust predicate and the page's isKept
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/agent-parity/is_kept.json");
    let fx: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for c in fx["cases"].as_array().unwrap() {
        let hit = c["hit"].as_object().unwrap();
        assert_eq!(is_kept(hit), c["kept"].as_bool().unwrap(), "{}", c["name"]);
    }
}

#[test]
fn on_grid_rounds_to_the_sample_and_is_manual_reads_the_markers() {
    assert_eq!(on_grid(0.123_456, Some(1000)), 0.123);
    assert_eq!(on_grid(0.123_456, None), 0.123_456);
    let m = |v: Value| is_manual(v.as_object().unwrap());
    assert!(m(json!({"t": 1.0, "added": true})));
    assert!(m(json!({"t": 1.0, "orig": 0.9})));
    assert!(!m(json!({"t": 1.0, "orig": 1.0})));
    assert!(!m(json!({"t": 1.0})));
}

#[test]
fn restore_and_an_edit_clear_the_learn_filter_mark() {
    let list = vec![
        json!({"t": 0.5, "v": 80, "filtered": true}),
        json!({"t": 1.0, "v": 80, "filtered": true}),
        json!({"t": 1.5, "v": 80, "filtered": true}),
    ];
    let ops = [
        json!({"op": "restore", "index": 0}),
        json!({"op": "move", "index": 1, "t": 1.1}),
    ];
    let (out, c) = apply_ops(list, &ops, None, None).unwrap();
    assert_eq!(c, [0, 0, 1, 0, 1, 0]);
    assert_eq!(out[0], json!({"t": 0.5, "v": 80, "edited": true}), "restored and kept");
    assert_eq!(
        out[1],
        json!({"t": 1.1, "v": 80, "orig": 1.0}),
        "a moved hit is never filtered"
    );
    assert_eq!(
        out[2]["filtered"], true,
        "planted fault: an untouched hit keeps the mark"
    );
}

/// R05: the undo of an edit restores the hit byte-identical (`undo(edit(doc)) == doc`): the page
/// sends the field presence the hit had before the edit as `was` on the inverse op.
#[test]
fn undo_restores_the_event_exactly() {
    // a hand velocity on a plain detected hit: the forward edit adds `edited: true`, the undo takes it off again
    let doc = vec![json!({"t": 1.0, "v": 100})];
    let (edited, _) = apply_ops(
        doc.clone(),
        &[json!({"op": "velocity", "index": 0, "v": 90, "edited": true})],
        None,
        None,
    )
    .unwrap();
    assert_eq!(edited, vec![json!({"t": 1.0, "v": 90, "edited": true})]);
    let (undone, _) = apply_ops(
        edited,
        &[json!({"op": "velocity", "index": 0, "v": 100, "was": {"edited": false}})],
        None,
        None,
    )
    .unwrap();
    assert_eq!(undone, doc, "undo(edit(doc)) == doc");

    // a move of a detected hit: the undo takes `orig` off; a move of an already moved hit restores its `orig` value
    let (moved, _) = apply_ops(doc.clone(), &[json!({"op": "move", "index": 0, "t": 1.1})], None, None).unwrap();
    assert_eq!(moved, vec![json!({"t": 1.1, "v": 100, "orig": 1.0})]);
    let (undone, _) = apply_ops(
        moved,
        &[json!({"op": "move", "index": 0, "t": 1.0, "was": {"orig": null}})],
        None,
        None,
    )
    .unwrap();
    assert_eq!(undone, doc);
    let doc2 = vec![json!({"t": 1.0, "v": 100, "orig": 0.9})];
    let (moved2, _) = apply_ops(doc2.clone(), &[json!({"op": "move", "index": 0, "t": 1.2})], None, None).unwrap();
    assert_eq!(moved2, vec![json!({"t": 1.2, "v": 100, "orig": 0.9})]);
    let (undone2, _) = apply_ops(
        moved2,
        &[json!({"op": "move", "index": 0, "t": 1.0, "was": {"orig": 0.9}})],
        None,
        None,
    )
    .unwrap();
    assert_eq!(undone2, doc2);

    // a velocity on a cut and bleed hit: the forward edit strips the bleed mark (the hit is kept) and
    // keeps the cut mark (the hit is not manual), the undo brings the bleed mark back
    let doc3 = vec![json!({"t": 0.5, "v": 100, "cut": true, "bleed": true})];
    let (edited3, _) = apply_ops(
        doc3.clone(),
        &[json!({"op": "velocity", "index": 0, "v": 64, "edited": true})],
        None,
        None,
    )
    .unwrap();
    assert_eq!(edited3, vec![json!({"t": 0.5, "v": 64, "cut": true, "edited": true})]);
    let (undone3, _) = apply_ops(
        edited3,
        &[json!({"op": "velocity", "index": 0, "v": 100, "was": {"edited": false, "cut": true, "bleed": true}})],
        None,
        None,
    )
    .unwrap();
    assert_eq!(undone3, doc3);

    // the undo of a restore: the tom bleed mark and the Learn filter mark come back, `edited` goes
    let doc4 = vec![json!({"t": 2.0, "v": 80, "bleed": true, "filtered": true, "learned": true})];
    let (restored, c) = apply_ops(doc4.clone(), &[json!({"op": "restore", "index": 0})], None, None).unwrap();
    assert_eq!(c[4], 1);
    assert_eq!(
        restored,
        vec![json!({"t": 2.0, "v": 80, "learned": true, "edited": true})]
    );
    let (undone4, _) = apply_ops(
        restored,
        &[json!({"op": "restore", "index": 0, "was": {"edited": false, "bleed": true, "filtered": true}})],
        None,
        None,
    )
    .unwrap();
    assert_eq!(undone4, doc4);

    // tags that ride along (`learned`, `roll`, `rise`) come back with `was` too
    let doc5 = vec![json!({"t": 1.0, "v": 100, "learned": true, "roll": true, "rise": true})];
    let (edited5, _) = apply_ops(
        doc5.clone(),
        &[json!({"op": "velocity", "index": 0, "v": 50, "edited": true})],
        None,
        None,
    )
    .unwrap();
    let (undone5, _) = apply_ops(
        edited5,
        &[json!({"op": "velocity", "index": 0, "v": 100, "was": {"edited": false, "learned": true, "roll": true, "rise": true}})],
        None,
        None,
    )
    .unwrap();
    assert_eq!(undone5, doc5);
}

/// R05: after the undo the hit is as before the edit, so a re-pick no longer treats it as kept.
#[test]
fn after_the_undo_a_repick_treats_the_hit_as_before_the_edit() {
    let doc = vec![json!({"t": 1.0, "v": 100})];
    let (edited, _) = apply_ops(
        doc.clone(),
        &[json!({"op": "velocity", "index": 0, "v": 90, "edited": true})],
        None,
        None,
    )
    .unwrap();
    assert!(is_kept(edited[0].as_object().unwrap()), "the hand velocity is kept");
    let (undone, _) = apply_ops(
        edited,
        &[json!({"op": "velocity", "index": 0, "v": 100, "was": {"edited": false}})],
        None,
        None,
    )
    .unwrap();
    assert!(!is_kept(undone[0].as_object().unwrap()), "the undo takes the edit off");
    let pick = json!({"t": 1.0, "v": 70});
    let merged = merge_edited(vec![pick.clone()], &undone);
    assert_eq!(merged, vec![pick], "the pick wins over the undone hit");
}

/// #242: the toggle op sets and clears the user's per-hit flags exactly, the undo sends the
/// state before as `was`, and the strip removes the automatic marks from a hit the user keeps.
#[test]
fn the_toggle_op_sets_the_user_flags_and_the_undo_restores_the_state() {
    let list = vec![json!({"t": 1.0, "v": 90, "cut": true}), json!({"t": 2.0, "v": 90})];
    // a deactivated hit becomes active: keep goes on, a deactivated mark comes off
    let (out, c) = apply_ops(
        list.clone(),
        &[json!({"op": "toggle", "index": 0, "keep": true, "deactivated": false})],
        None,
        None,
    )
    .unwrap();
    assert_eq!(c, [0, 0, 0, 0, 0, 1]);
    assert_eq!(
        out[0],
        json!({"t": 1.0, "v": 90, "keep": true}),
        "kept in the output; the cut mark leaves with the strip"
    );
    // an active hit becomes deactivated: the user's own mark goes on
    let (out, c) = apply_ops(
        list.clone(),
        &[json!({"op": "toggle", "index": 1, "keep": false, "deactivated": true})],
        None,
        None,
    )
    .unwrap();
    assert_eq!(c, [0, 0, 0, 0, 0, 1]);
    assert_eq!(out[1], json!({"t": 2.0, "v": 90, "deactivated": true}));
    // the undo sends the state before: the flags go back to absent, the cut mark comes back
    let (undone, c) = apply_ops(
        out,
        &[json!({"op": "toggle", "index": 0, "keep": false, "deactivated": false, "was": {"cut": true, "keep": false, "deactivated": false}})],
        None,
        None,
    )
    .unwrap();
    assert_eq!(c, [0, 0, 0, 0, 0, 1]);
    assert_eq!(undone[0], list[0], "byte-identical with the hit before the toggle");
    // a missing or non-boolean flag is a 400 and nothing changes
    let e = apply_ops(list.clone(), &[json!({"op": "toggle", "index": 0})], None, None).unwrap_err();
    assert_eq!(e.code, 400);
    let e = apply_ops(
        list,
        &[json!({"op": "toggle", "index": 0, "keep": 1, "deactivated": false})],
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, 400);
}

/// #242: a kept hit survives a re-pick or Re-analyze merge (`merge_edited` reads `is_kept`),
/// with its flags; a pick at another time comes in beside it.
#[test]
fn merge_edited_keeps_a_toggled_hit() {
    let current = vec![json!({"t": 2.0, "v": 90, "deactivated": true, "cut": true})];
    let merged = merge_edited(
        vec![
            json!({"t": 0.5, "v": 80}),
            json!({"t": 1.5, "v": 80}),
            json!({"t": 2.5, "v": 80}),
        ],
        &current,
    );
    assert_eq!(
        merged,
        vec![
            json!({"t": 0.5, "v": 80}),
            json!({"t": 1.5, "v": 80}),
            json!({"t": 2.0, "v": 90, "deactivated": true, "cut": true}),
            json!({"t": 2.5, "v": 80}),
        ],
        "the toggled hit stays as it is between the picks"
    );
    let kept = vec![json!({"t": 2.0, "v": 90, "keep": true})];
    let merged = merge_edited(vec![json!({"t": 2.0, "v": 70})], &kept);
    assert_eq!(
        merged, kept,
        "a pick within the merge window loses against the kept hit"
    );
}

/// A synthetic kick (the app tests' `kick_signal`) at each of `onsets` in `dur` seconds at 48 kHz.
fn kicks(onsets: &[f64], dur: f64) -> Vec<f32> {
    let srf = 48_000.0;
    let mut x = vec![0.0f64; (dur * srf) as usize];
    for &t0 in onsets {
        let i0 = (t0 * srf).round() as usize;
        let mut cum = 0.0;
        for k in 0..((0.4 * srf) as usize) {
            if i0 + k >= x.len() {
                break;
            }
            let t = k as f64 / srf;
            cum += 55.0 + 70.0 * (-t / 0.03).exp();
            let env = (t / 0.0005).min(1.0) * (-t / 0.12).exp();
            x[i0 + k] += (2.0 * std::f64::consts::PI * cum / srf).sin() * env * 0.7;
        }
    }
    x.iter().map(|&v| v as f32).collect()
}

#[test]
fn snap_onset_finds_the_analysis_onset_of_the_attack_and_nothing_in_a_tail() {
    let x = kicks(&[0.5, 1.5], 2.0);
    for rule in [RefineRule::Reference, RefineRule::Reach, RefineRule::FirstMotion] {
        for peak in [drumspike_core::refine::FM_PEAK_DB, 29.0] {
            // the analysis: the detector's coarse time about 6 ms before the attack, the default window
            let onset = refine_rule_peak(&x, 0.494, 48_000, &[1.494], RefineWindow::DEFAULT, rule, peak).t;
            assert!((onset - 0.5).abs() < 0.002, "{rule:?}: analysis onset {onset}");
            for click in [onset - 0.003, onset - 0.008, onset + 0.002] {
                let t = snap_onset(&x, 48_000, click, &[1.5], rule, peak).expect("an attack near the click");
                assert!(
                    ((t - onset) * 48_000.0).abs() <= 2.0,
                    "{rule:?} {peak}: click {click} snapped to {t}, analysis {onset}"
                );
            }
            // 30 ms into the decay, in the silence before the first kick, and 15 ms before the attack: no snap
            for click in [onset + 0.030, 0.2, onset - 0.015] {
                assert_eq!(
                    snap_onset(&x, 48_000, click, &[1.5], rule, peak),
                    None,
                    "{rule:?}: click {click}"
                );
            }
        }
    }
    // the neighbours clip the search: a hit 2 ms after the click hides the attack behind it
    assert_eq!(
        snap_onset(&x, 48_000, 0.497, &[0.499], RefineRule::FirstMotion, 33.0).filter(|t| *t > 0.498),
        None
    );
}
