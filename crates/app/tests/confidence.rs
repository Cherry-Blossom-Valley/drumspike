//! R13 #237: the confidence reads the activations and the loudness in detector time. The Hitpoint
//! shift (`track_shift_set`) moves the stored hits, not the audio, so the confidence of a detected
//! hit is the same with and without a shift — also with a neighbouring stroke next to it — and the
//! review queue order (D, the doubtful hits lowest confidence first) stays the same.

mod common;

use common::*;
use serde_json::{json, Value};

/// The review queue order (D) of a `track_confidence` answer: the doubtful hits, lowest
/// confidence first, ties by hit index.
fn queue_order(conf: &Value) -> Vec<usize> {
    let mut hits: Vec<(usize, f64)> = conf["confidence"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, e)| e["doubt"].as_bool() == Some(true))
        .map(|(i, e)| (i, e["c"].as_f64().unwrap()))
        .collect();
    hits.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    hits.into_iter().map(|(i, _)| i).collect()
}

/// A short sine burst per `(time, amplitude)`: no decay tail, so a shifted read lands on silence
/// or on the neighbour, never on the hit's own tail.
fn bursts(onsets: &[(f64, f64)], dur: f64, sr: u32) -> Vec<f32> {
    let srf = f64::from(sr);
    let mut x = vec![0.0f32; (dur * srf) as usize];
    for &(t0, a) in onsets {
        let i0 = ((t0 * srf).round() as usize).min(x.len());
        for k in 0..((0.01 * srf) as usize) {
            if i0 + k < x.len() {
                x[i0 + k] = (a * (2.0 * std::f64::consts::PI * 220.0 * (k as f64 / srf)).sin()) as f32;
            }
        }
    }
    x
}

fn spec(name: &'static str, sr: u32, x: Vec<f32>) -> Spec {
    Spec {
        name,
        role: "kick",
        note: 36,
        x,
        sr,
    }
}

/// The level path (no model): a loud stroke, a quiet and a mid neighbour, 150 ms apart. A shifted
/// read lands on the neighbour or on silence, so the confidences and the review order must not
/// move with the shift.
#[test]
fn level_confidence_and_review_order_do_not_follow_the_shift() {
    let root = tmp("conf_r13_level");
    let x = bursts(&[(0.5, 0.8), (0.65, 0.01), (0.8, 0.2)], 1.2, 48_000);
    let dir = project(&root, "Song", &[spec("k", 48_000, x)]);
    let svc = service_dsp();
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    // plain stored hits (no marks): the level path rates them
    svc.events_set(&sid, "k", &json!([{"t": 0.5}, {"t": 0.65}, {"t": 0.8}]))
        .unwrap();
    let base = svc.track_confidence(&sid, "k", None).unwrap();
    assert_eq!(base["source"], "level");
    assert_eq!(
        queue_order(&base),
        vec![1],
        "the quiet middle stroke is the doubtful one"
    );
    // 100 ms at 48 kHz, a whole activation frame
    for shift in [4_800i64, -4_800] {
        svc.track_shift_set(&sid, "k", shift).unwrap();
        let shifted = svc.track_confidence(&sid, "k", None).unwrap();
        assert_eq!(shifted["confidence"], base["confidence"], "shift {shift}");
        assert_eq!(queue_order(&shifted), queue_order(&base), "shift {shift}");
    }
}

/// Detector #8 package 4: the review queue says which gate kept a hit. A generated session —
/// a snare track with six loud strokes and two ghost notes 20 dB under them — analyzed by the
/// DSP detector keeps the ghosts through the soft gate (A1: below the level gate, shape within
/// `soft_sim` of the template, no kick or tom hit within 30 ms). The `why` of a soft-rescued
/// hit names the soft gate and its `c` is under [`DOUBT_BELOW`]; a loud hit (the level gate)
/// does not name it and is not doubtful.
#[test]
fn soft_rescued_hit_names_the_soft_gate_and_the_loud_hit_does_not() {
    let root = tmp("conf_p4_soft");
    let mut x = kick_signal(&ONSETS, 3.5, 48_000);
    for &g in &[0.5f64, 2.0] {
        let ghost = kick_signal(&[g], 3.5, 48_000);
        for (a, b) in x.iter_mut().zip(ghost) {
            *a += b * 10f32.powf(-20.0 / 20.0);
        }
    }
    let dir = project(
        &root,
        "Song",
        &[Spec {
            name: "s top",
            role: "snare",
            note: 38,
            x,
            sr: 48_000,
        }],
    );
    let svc = service_dsp();
    let sid = svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.analyze_start(&sid, Some(vec!["s top".into()]), None, None).unwrap();
    let st = wait_done(&svc, &sid);
    assert_eq!(st["tracks"]["s top"]["state"], "done", "{st}");
    let events = svc.events_get(&sid, "s top").unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        events.len(),
        8,
        "six loud strokes and two soft-rescued ghost notes: {events:?}"
    );
    let conf = svc.track_confidence(&sid, "s top", None).unwrap();
    assert_eq!(conf["source"], "level");
    let rows = conf["confidence"].as_array().unwrap();
    assert_eq!(rows.len(), events.len());
    for (e, r) in events.iter().zip(rows) {
        let t = e["t"].as_f64().unwrap();
        let ghost = (t - 0.5).abs() < 0.05 || (t - 2.0).abs() < 0.05;
        let why = r["why"].as_array().unwrap();
        if ghost {
            assert_eq!(r["doubt"], json!(true), "{t}: {r}");
            assert!(
                r["c"].as_f64().unwrap() < conf["doubt_below"].as_f64().unwrap(),
                "{t}: {r}"
            );
            assert!(
                why.iter().any(|w| w.as_str().unwrap().contains("soft gate")),
                "{t}: {r}"
            );
        } else {
            assert_eq!(r["doubt"], json!(false), "{t}: {r}");
            assert!(
                r["c"].as_f64().unwrap() >= conf["doubt_below"].as_f64().unwrap(),
                "{t}: {r}"
            );
            assert!(
                !why.iter().any(|w| w.as_str().unwrap().contains("soft gate")),
                "{t}: {r}"
            );
        }
    }
}
