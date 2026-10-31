//! P4-13 stage B: the flam rule in the analysis (`crates/app/src/flam.rs`, core `detect::flam`): the main stroke of a
//! flam on a snare mic comes in as a `"flam": true` hit, the version and the outcome are recorded, the bleed gate reads
//! the project's other mics (no snare and no sample track), the track's switch turns it off, an old analysis keeps its
//! hits on a re-pick, and the pin turns the rule off.
mod common;

use common::*;
use drumspike_app::Service;
use drumspike_app::flam::FLAM_RULE;
use serde_json::{Map, Value, json};

const SR: u32 = 48_000;

/// A deterministic standard normal source (LCG and Box-Muller).
struct Normal(u64);

impl Normal {
    fn uniform(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    fn next(&mut self) -> f64 {
        let (a, b) = (self.uniform(), self.uniform());
        (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
    }
}

/// The DRT kit's snare: a 190 Hz body (40 ms decay) and noise (`decay` s), 250 ms long.
fn snare(rng: &mut Normal, decay: f64) -> Vec<f32> {
    (0..SR as usize / 4)
        .map(|i| {
            let t = i as f64 / f64::from(SR);
            let body = (2.0 * std::f64::consts::PI * 190.0 * t).sin() * (-t / 0.04).exp();
            let noise = rng.next() * (-t / decay).exp() * 0.6;
            ((body + noise) / 1.6) as f32
        })
        .collect()
}

fn place(buf: &mut [f32], s: &[f32], at_s: f64, db: f64) {
    let at = (at_s * f64::from(SR)).round() as usize;
    let g = 10f64.powf(db / 20.0) as f32;
    for (i, v) in s.iter().enumerate() {
        if at + i < buf.len() {
            buf[at + i] += g * v;
        }
    }
}

/// A snare track: a backbeat every second at -6 dB from 1 s; at 3, 6 and 9 s a close pair instead: a short stroke at -8 dB and
/// the main stroke at -6 dB 30 ms later (the model keeps the first). Returns the signal and the main strokes' times.
fn flam_track(seed: u64) -> (Vec<f32>, Vec<f64>) {
    let mut rng = Normal(seed);
    let mut x = vec![0.0f32; 12 * SR as usize];
    let mut mains = Vec::new();
    for i in 0..10 {
        let t = 1.0 + f64::from(i);
        if i % 3 == 2 {
            let g = snare(&mut rng, 0.008);
            place(&mut x, &g, t, -8.0);
            let m = snare(&mut rng, 0.06);
            place(&mut x, &m, t + 0.03, -6.0);
            mains.push(t + 0.03);
        } else {
            let s = snare(&mut rng, 0.06);
            place(&mut x, &s, t, -6.0);
        }
    }
    for v in &mut x {
        *v += 1e-4 * rng.next() as f32;
    }
    (x, mains)
}

/// A kick or hat track: a hit every 0.5 s from 1.25 s (never at a flam).
fn other_track(seed: u64, hz: f64) -> Vec<f32> {
    let mut rng = Normal(seed);
    let mut x = vec![0.0f32; 12 * SR as usize];
    for i in 0..20 {
        let at = ((1.25 + 0.5 * f64::from(i)) * f64::from(SR)) as usize;
        for k in 0..(SR as usize / 10) {
            let t = k as f64 / f64::from(SR);
            let v = if hz > 0.0 {
                (2.0 * std::f64::consts::PI * hz * t).sin() * (-t / 0.05).exp()
            } else {
                rng.next() * (-t / 0.015).exp() * 0.3
            };
            x[at + k] += (0.5 * v) as f32;
        }
    }
    x
}

/// A snare bottom mic: a short weak first stroke, and the wires' noise that rises 14 ms after the hit and peaks over the
/// first 12 ms: a late-peaking mic.
/// A tom track with 3 hits: too few strong hits for a transfer gain.
fn few_track(seed: u64) -> Vec<f32> {
    let mut rng = Normal(seed);
    let mut x = vec![0.0f32; 12 * SR as usize];
    for i in 0..3 {
        let at = ((1.25 + 3.0 * f64::from(i)) * f64::from(SR)) as usize;
        for k in 0..(SR as usize / 10) {
            let t = k as f64 / f64::from(SR);
            x[at + k] += (0.4 * rng.next() * (-t / 0.02).exp()) as f32;
        }
    }
    x
}

fn spec(name: &'static str, role: &'static str, x: Vec<f32>) -> Spec {
    let note = match role {
        "kick" => 36,
        "hat" => 42,
        _ => 38,
    };
    Spec {
        name,
        role,
        note,
        x,
        sr: SR,
    }
}

fn events(svc: &Service, sid: &str, name: &str) -> Vec<Value> {
    svc.events_get(sid, name).unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn flams(svc: &Service, sid: &str, name: &str) -> Vec<f64> {
    events(svc, sid, name)
        .iter()
        .filter(|e| e["flam"] == true)
        .map(|e| e["t"].as_f64().unwrap())
        .collect()
}

fn detector(svc: &Service, sid: &str, name: &str) -> Value {
    svc.tracks_get(sid).unwrap()["tracks"][name]["detector"].clone()
}

fn open(svc: &Service, dir: &std::path::Path) -> String {
    svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string()
}

fn params(svc: &Service, sid: &str, name: &str) -> Map<String, Value> {
    svc.tracks_get(sid).unwrap()["tracks"][name]["params"]
        .as_object()
        .cloned()
        .unwrap_or_default()
}

fn at_mains(found: &[f64], mains: &[f64]) {
    for t in found {
        assert!(
            mains.iter().any(|m| (m - t).abs() < 0.003),
            "flam hit {t} at a main stroke {mains:?}"
        );
    }
}

#[test]
fn a_flam_gets_its_main_stroke_and_the_switch_turns_it_off() {
    let root = tmp("flam_rule");
    let svc = service_dsp();
    let (x, mains) = flam_track(3);
    let dir = project(
        &root,
        "Flams",
        &[
            spec("s top", "snare", x.clone()),
            spec("s samp", "snare", x),
            spec("k in", "kick", other_track(5, 60.0)),
            spec("hh", "hat", other_track(7, 0.0)),
            spec("t1", "tom", few_track(9)),
        ],
    );
    let sid = open(&svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    let st = wait_done(&svc, &sid);
    let found = flams(&svc, &sid, "s top");
    let ts: Vec<f64> = events(&svc, &sid, "s top")
        .iter()
        .map(|e| e["t"].as_f64().unwrap())
        .collect();
    assert_eq!(found.len(), mains.len(), "flam hits {found:?}: {ts:?} {st}");
    at_mains(&found, &mains);
    let det = detector(&svc, &sid, "s top");
    assert_eq!(det["flam_rule"], FLAM_RULE);
    assert_eq!(det["flam"]["on"], true);
    assert_eq!(det["flam"]["added"], mains.len());
    assert_eq!(
        det["flam"]["sources"],
        json!(["k in", "hh"]),
        "no snare and no sample track, and a source with a gain (the tom has 3 hits, fewer than 5): {det}"
    );
    assert!(
        det["flam"]["late_share"].as_f64().unwrap() < 0.4,
        "a slow-decay top mic (60 ms, A over -3 dB) passes the late-peak gate: {det}"
    );
    assert_eq!(st["flam"]["tracks"]["s top"], mains.len(), "{st}");
    // a sample layer is no target
    assert!(flams(&svc, &sid, "s samp").is_empty());
    assert!(detector(&svc, &sid, "s samp").get("flam_rule").is_none());
    // a flam hit has a velocity and nothing else of an edit
    let e = events(&svc, &sid, "s top")
        .into_iter()
        .find(|e| e["flam"] == true)
        .unwrap();
    assert!(e["v"].as_i64().unwrap() > 20 && e.get("added").is_none(), "{e}");
    // a stored re-pick brings the same flam hits again
    svc.track_repick(&sid, "s top", Some(&params(&svc, &sid, "s top")), true)
        .unwrap();
    assert_eq!(flams(&svc, &sid, "s top"), found);
    // the switch: "Split flams" off applies on the next re-analyze
    let mut p = params(&svc, &sid, "s top");
    p.insert("flam_split".into(), Value::Bool(false));
    let mut all = Map::new();
    all.insert("s top".into(), Value::Object(p));
    svc.analyze_start(&sid, Some(vec!["s top".into()]), Some(&all), None)
        .unwrap();
    wait_done(&svc, &sid);
    assert!(flams(&svc, &sid, "s top").is_empty());
    let det = detector(&svc, &sid, "s top");
    assert_eq!(
        (det["flam_rule"].clone(), det["flam"]["on"].clone()),
        (json!(FLAM_RULE), json!(false)),
        "{det}"
    );
}
