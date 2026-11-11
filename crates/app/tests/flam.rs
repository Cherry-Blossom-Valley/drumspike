//! P4-13 stage B: the flam rule in the analysis (`crates/app/src/flam.rs`, core `detect::flam`): the main stroke of a
//! flam on a snare mic comes in as a `"flam": true` hit, the version and the outcome are recorded, the bleed gate reads
//! the project's other mics (no snare and no sample track), the track's switch turns it off, an old analysis keeps its
//! hits on a re-pick, and the pin turns the rule off.
mod common;

use common::*;
use drumspike_app::flam::FLAM_RULE;
use drumspike_app::{Service, ServiceConfig};
use drumspike_core::detect::flam::{FlamParams, flam_split};
use drumspike_core::refine::RefineRule;
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

/// A deterministic noise source in [-1, 1) (the same source as the core flam tests).
struct W444(u64);

impl W444 {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

/// A noise burst of `len_s` seconds with the decay `tau` (s) and the amplitude `amp`, drawn from `n`.
fn burst444(n: &mut W444, len_s: f64, tau: f64, amp: f64) -> Vec<f64> {
    let len = (len_s * f64::from(SR)) as usize;
    (0..len)
        .map(|k| amp * n.next() * (-(k as f64) / (tau * f64::from(SR))).exp())
        .collect()
}

/// `s` added at `at` seconds; returns the stroke's first sample.
fn add_at48(x: &mut [f64], s: &[f64], at: f64) -> usize {
    let i0 = (at * f64::from(SR)).round() as usize;
    for (k, &v) in s.iter().enumerate() {
        if i0 + k < x.len() {
            x[i0 + k] += v;
        }
    }
    i0
}

fn f32s48(x: &[f64]) -> Vec<f32> {
    x.iter().map(|&v| v as f32).collect()
}

/// The #444-shape stroke body (the core test `the_second_stroke_follows_the_rule_of_the_track`'s
/// `stroke_at`): a line with a slow wobble, a pre-swing, a fast edge and a ring. The shape where
/// refine versions 9 and 10 place differently at 48 kHz (version 10's true line median).
fn stroke_body(t: f64, dip: f64, gap: f64, width: f64, rise: f64, hz: f64) -> f64 {
    use std::f64::consts::PI;
    let line = 0.05 + 0.003 * (2.0 * PI * 180.0 * t + 0.4).sin() + 0.001_5 * (2.0 * PI * 2_300.0 * t + 1.1).sin();
    let swing = if t > -gap - width && t < -gap {
        let u = (t + gap + width) / width;
        -dip * (0.5 - 0.5 * (2.0 * PI * u).cos())
    } else {
        0.0
    };
    let body = if t <= 0.0 {
        0.0
    } else if t < rise {
        0.5 - 0.5 * (PI * t / rise).cos()
    } else {
        let u = t - rise;
        (-u / 0.03).exp() * (2.0 * PI * hz * u).cos()
    };
    -(line + swing + body)
}

/// Six #444-shape flams: a grace burst 30 ms before each body, the flams 80 ms apart. The model
/// keeps the grace (the first stroke of the pair); the body is the second stroke the rule adds.
/// The 30 ms gap keeps the body out of the late-peak window (the 30 ms end is excluded) and
/// inside the re-rise search (12 to 50 ms, 10 ms before the next hit).
fn flam444_fixture() -> Vec<f32> {
    let srf = f64::from(SR);
    let mut x = vec![0.0f64; (3.0 * srf) as usize];
    let mut nz = W444(0x4455_6677_8899_0011);
    for k in 0..6 {
        let te = 1.0 + 0.080 * f64::from(k);
        let grace = burst444(&mut nz, 0.02, 0.003, 0.5);
        add_at48(&mut x, &grace, te - 0.030);
        let i0 = (te * srf).round() as usize;
        for j in 0..(0.06 * srf) as usize {
            let v = stroke_body(
                j as f64 / srf,
                [0.03, 0.045, 0.06, 0.08, 0.045, 0.06][j % 6],
                [0.0, 40e-6, 80e-6, 110e-6, 150e-6, 180e-6][j % 6],
                [150e-6, 212.5e-6, 275e-6, 337.5e-6, 400e-6, 250e-6][j % 6],
                [60e-6, 80e-6, 100e-6, 60e-6, 80e-6, 100e-6][j % 6],
                [180.0, 215.0, 250.0, 285.0, 320.0, 355.0][j % 6],
            );
            if i0 + j < x.len() {
                x[i0 + j] += v;
            }
        }
    }
    for v in &mut x {
        *v += 1e-5 * nz.next();
    }
    f32s48(&x)
}

fn at_samples(ts: &[f64]) -> Vec<i64> {
    ts.iter().map(|t| (t * f64::from(SR)).round() as i64).collect()
}

/// #445 (2): the re-pick of a stored project places the flam second stroke by the refine rule the
/// analysis recorded, not by the current rule. The analysis runs pinned to version 9; the strokes
/// are #444-shape, where versions 9 and 10 place differently at 48 kHz. The re-pick under a
/// version-10 service keeps the version-9 placement, and a fresh analysis under it moves the
/// strokes: the test would catch a re-pick that ran the current rule.
#[test]
fn a_repick_places_the_flam_second_stroke_by_the_recorded_refine_rule() {
    let root = tmp("flam_rule_repick9");
    let x = flam444_fixture();
    let dir = project(&root, "Repick9", &[spec("s top", "snare", x.clone())]);
    // the analysis pinned to version 9: the record carries 9, and the six bodies are added
    let svc9 = Service::new(ServiceConfig {
        refine_rule: Some(RefineRule::Departure),
        ..ServiceConfig::default()
    });
    let sid = open(&svc9, &dir);
    svc9.analyze_start(&sid, None, None, None).unwrap();
    let st = wait_done(&svc9, &sid);
    let f9 = flams(&svc9, &sid, "s top");
    assert_eq!(f9.len(), 6, "the six bodies are found and added: {st}");
    assert_eq!(detector(&svc9, &sid, "s top")["refine"], json!(9));
    let f9s = at_samples(&f9);
    svc9.project_close(&sid, false).unwrap();
    // the re-pick under a version-10 service: the recorded rule 9 places the second strokes
    let svc10 = service_dsp();
    let sid10 = open(&svc10, &dir);
    svc10
        .track_repick(&sid10, "s top", Some(&params(&svc10, &sid10, "s top")), true)
        .unwrap();
    let frs = at_samples(&flams(&svc10, &sid10, "s top"));
    assert_eq!(
        frs, f9s,
        "the re-pick moved the second strokes off the recorded rule 9: {frs} against {f9s}"
    );
    // the placement is the recorded rule's own answer on the stored hits, and the fixture
    // discriminates: version 10 places the same bodies differently
    let mains: Vec<f64> = events(&svc10, &sid10, "s top")
        .iter()
        .filter(|e| e["flam"] != true)
        .map(|e| e["t"].as_f64().unwrap())
        .collect();
    let p = FlamParams::default();
    let (d9, _) = flam_split(&x, SR, &mains, &[], &p, 2, RefineRule::Departure);
    let (d10, _) = flam_split(&x, SR, &mains, &[], &p, 2, RefineRule::Symmetric);
    let t9 = at_samples(&d9.iter().filter(|s| !s.bleed).map(|s| s.t).collect::<Vec<_>>());
    let t10 = at_samples(&d10.iter().filter(|s| !s.bleed).map(|s| s.t).collect::<Vec<_>>());
    assert_eq!(
        t9, frs,
        "the re-pick's placement is not the recorded rule's answer: {t9} against {frs}"
    );
    assert_ne!(t9, t10, "the fixture does not discriminate version 9 from 10: {t9}");
    // a fresh analysis under the version-10 service moves the second strokes
    svc10.analyze_start(&sid10, None, None, None).unwrap();
    wait_done(&svc10, &sid10);
    let f10s = at_samples(&flams(&svc10, &sid10, "s top"));
    assert_ne!(
        f10s, f9s,
        "a fresh analysis under version 10 places as 9: {f10s} against {f9s}"
    );
}
