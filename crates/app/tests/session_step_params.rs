//! #438: the session steps keep the track's parameters. The ghost gate and cross-bleed pass 2
//! build their `AnalyzeParams` from the track's stored parameter object (the same parse as the
//! analysis job) instead of the struct defaults, and a flam second stroke is stored with the
//! track's Hitpoint shift. Each test runs the same audio twice (shift 0 and shift 240, or
//! default and changed parameters) and compares the stored hits; every test prints the count
//! of compared hits and fails on 0.
mod common;

use common::*;
use drumspike_app::Service;
use serde_json::{Map, Value, json};

const SR: u32 = 48_000;
const SECONDS: f64 = 12.0;
const SHIFT: f64 = 240.0;

/// A deterministic standard normal source (LCG and Box-Muller), as in `tests/flam.rs`.
struct Normal(u64);

impl Normal {
    fn next(&mut self) -> f64 {
        let a = (self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407)
            >> 11) as f64
            / (1u64 << 53) as f64;
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let b = (self.0 >> 11) as f64 / (1u64 << 53) as f64;
        (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
    }
}

/// The DRT kit's snare: a 190 Hz body (40 ms decay) and noise, 250 ms long.
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

fn spec(name: &'static str, role: &'static str, x: Vec<f32>) -> Spec {
    let note = match role {
        "kick" => 36,
        "tom" => 47,
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

fn events(svc: &Service, sid: &str, name: &str) -> Vec<f64> {
    svc.events_get(sid, name).unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
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

/// The ghost-gate session: a snare close mic with loud hits (1.0, 1.1, 2.0, 3.0, 4.0, 5.0,
/// 6.0) and quiet copies of the same stroke 24 dB down at 1.5, 3.5 and 5.5, and a kick whose
/// hits sit on the quiet copies (the busy clearance denies them). The soft rescue keeps the
/// quiet hits in the initial detection, so the ghost gate runs and re-stores the track.
fn ghost_session(svc: &Service, root: &std::path::Path) -> String {
    let mut rng = Normal(11);
    let mut s = vec![0.0f32; SECONDS as usize * SR as usize];
    for &t in &[1.0, 1.1, 2.0, 3.0, 4.0, 5.0, 6.0] {
        place(&mut s, &snare(&mut rng, 0.06), t, -6.0);
    }
    let quiet = snare(&mut rng, 0.06);
    for &t in &[1.5, 3.5, 5.5] {
        place(&mut s, &quiet, t, -30.0);
    }
    for v in &mut s {
        *v += 1e-4 * rng.next() as f32;
    }
    let k = kick_signal(&[1.5, 2.5, 3.5, 4.5, 5.5], SECONDS, SR);
    let dir = project(root, "Ghost", &[spec("s top", "snare", s), spec("k in", "kick", k)]);
    open(svc, &dir)
}

/// A decaying tone burst at sample `at` (as in `tests/tom_bleed.rs`).
fn burst(x: &mut [f32], at: usize, amp: f32, freq: f64) {
    let mut phase = 0.0f64;
    let mut s = 99u64;
    for i in 0..(SR as usize / 5) {
        let k = at + i;
        if k >= x.len() {
            break;
        }
        let t = i as f64 / f64::from(SR);
        phase += 2.0 * std::f64::consts::PI * freq * (1.0 + (-t / 0.02).exp()) / f64::from(SR);
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let noise = ((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0;
        let env = (t / 0.0005).min(1.0) * (-t / 0.08).exp();
        x[k] += amp * (phase.sin() * env + 0.5 * noise * (-t / 0.002).exp()) as f32;
    }
}

/// A tom session: two played tom close mics (12 hits each 0.9 s apart, every third soft) that
/// hear each other 20 dB down, and a kick. Cross-bleed pass 2 re-detects every tom with a
/// stored candidate list and stores the analysis again.
fn tom_session(svc: &Service, root: &std::path::Path) -> String {
    let n = SECONDS as usize * SR as usize;
    let mk = |freq: f64, seed: u64| {
        let mut x = vec![0.0f32; n];
        for i in 0..12 {
            let at = ((0.5 + 0.9 * f64::from(i)) * f64::from(SR)) as usize;
            let a = if i % 3 == 2 { 0.1 } else { 0.5 };
            burst(&mut x, at, a, freq);
        }
        let mut s = seed;
        for v in &mut x {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            *v += 1e-4 * (((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0) as f32;
        }
        x
    };
    let (t1, t2) = (mk(140.0, 3), mk(110.0, 5));
    let k = kick_signal(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], SECONDS, SR);
    let dir = project(
        root,
        "Toms",
        &[spec("t1", "tom", t1), spec("t2", "tom", t2), spec("k in", "kick", k)],
    );
    open(svc, &dir)
}

/// The flam session of `tests/flam.rs`: a snare with close pairs (a short stroke and the main
/// stroke 30 ms later) that the flam rule splits.
fn flam_session(svc: &Service, root: &std::path::Path) -> String {
    let mut rng = Normal(3);
    let mut x = vec![0.0f32; 12 * SR as usize];
    for i in 0..10 {
        let t = 1.0 + f64::from(i);
        if i % 3 == 2 {
            place(&mut x, &snare(&mut rng, 0.008), t, -8.0);
            place(&mut x, &snare(&mut rng, 0.06), t + 0.03, -6.0);
        } else {
            place(&mut x, &snare(&mut rng, 0.06), t, -6.0);
        }
    }
    for v in &mut x {
        *v += 1e-4 * rng.next() as f32;
    }
    let dir = project(root, "Flams", &[spec("s top", "snare", x)]);
    open(svc, &dir)
}

fn analyze(svc: &Service, sid: &str, targets: &[&str], params: Option<&Map<String, Value>>) -> Value {
    let targets: Vec<String> = targets.iter().copied().map(str::to_string).collect();
    svc.analyze_start(sid, Some(targets), params, None).unwrap();
    wait_done(svc, sid)
}

fn shift_params(v: f64) -> Map<String, Value> {
    let mut p = Map::new();
    p.insert("shift_samples".into(), json!(v));
    p
}

/// Every stored hit of the shift-240 run is exactly 240 samples after the hit of the shift-0
/// run; prints the compared count and fails on 0.
fn assert_shift(run0: &[f64], run240: &[f64], what: &str) {
    assert_eq!(run0.len(), run240.len(), "{what}: the runs found different hits");
    let n = run0.len();
    println!("{what}: compared {n} hits");
    assert!(n > 0, "{what}: no hits to compare");
    let dt = SHIFT / f64::from(SR);
    for (a, b) in run0.iter().zip(run240) {
        assert!((b - a - dt).abs() < 1e-6, "{what}: {b} - {a} is not {dt} s");
    }
}

#[test]
fn the_ghost_gate_keeps_the_tracks_params_and_hitpoint_shift() {
    let root = tmp("ghost_shift");
    let svc = service_dsp();
    let sid = ghost_session(&svc, &root);
    analyze(&svc, &sid, &[], None);
    let zero = events(&svc, &sid, "s top");
    let det0 = detector(&svc, &sid, "s top");
    assert_eq!(det0["ghost_gate"]["on"], true, "the ghost gate must run: {det0}");
    let p = shift_params(SHIFT);
    let mut all = Map::new();
    all.insert("s top".into(), Value::Object(p));
    analyze(&svc, &sid, &[], Some(&all));
    let shifted = events(&svc, &sid, "s top");
    assert!(
        detector(&svc, &sid, "s top")["ghost_gate"]["on"] == true,
        "the ghost gate must run again: {}",
        detector(&svc, &sid, "s top")
    );
    assert_shift(&zero, &shifted, "ghost gate");
}

#[test]
fn cross_bleed_pass2_keeps_the_tracks_params_and_hitpoint_shift() {
    let root = tmp("pass2_shift");
    let svc = service_dsp();
    let sid = tom_session(&svc, &root);
    analyze(&svc, &sid, &["t1", "t2"], None);
    let zero = events(&svc, &sid, "t1");
    assert!(
        svc.tracks_get(&sid).unwrap()["tracks"]["t1"]["detector"]
            .get("cross_bleed")
            .is_some(),
        "cross-bleed pass 2 must run on t1"
    );
    let p = shift_params(SHIFT);
    let mut all = Map::new();
    all.insert("t1".into(), Value::Object(p));
    analyze(&svc, &sid, &["t1", "t2"], Some(&all));
    let shifted = events(&svc, &sid, "t1");
    assert_shift(&zero, &shifted, "cross-bleed pass 2");
}

#[test]
fn a_flam_second_stroke_gets_the_tracks_hitpoint_shift() {
    let root = tmp("flam_shift");
    let svc = service_dsp();
    let sid = flam_session(&svc, &root);
    analyze(&svc, &sid, &["s top"], None);
    let flam_of = |svc: &Service, sid: &str| -> Vec<f64> {
        svc.events_get(sid, "s top").unwrap()["events"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|e| e["flam"] == true)
            .map(|e| e["t"].as_f64().unwrap())
            .collect()
    };
    let zero = flam_of(&svc, &sid);
    let p = shift_params(SHIFT);
    let mut all = Map::new();
    all.insert("s top".into(), Value::Object(p));
    analyze(&svc, &sid, &["s top"], Some(&all));
    let shifted = flam_of(&svc, &sid);
    assert_shift(&zero, &shifted, "flam strokes");
}

#[test]
fn the_ghost_gate_second_pass_gets_the_refine_window_and_the_min_gap() {
    let root = tmp("ghost_params");
    let svc = service_dsp();
    let sid = ghost_session(&svc, &root);
    analyze(&svc, &sid, &[], None);
    let default_run = events(&svc, &sid, "s top");
    // a 500 ms minimum gap and a zero refine window: the second pass must collapse the pair
    // at 1.0/1.1 and keep the hits on the detection grid
    let mut p = Map::new();
    p.insert("min_gap_ms".into(), json!(500.0));
    p.insert("refine_lo_ms".into(), json!(0.0));
    p.insert("refine_hi_ms".into(), json!(0.0));
    let mut all = Map::new();
    all.insert("s top".into(), Value::Object(p));
    let st = analyze(&svc, &sid, &[], Some(&all));
    assert!(
        detector(&svc, &sid, "s top")["ghost_gate"]["on"] == true,
        "the ghost gate must run: {st}"
    );
    let changed = events(&svc, &sid, "s top");
    println!(
        "ghost gate second pass: {} hits with the track's params, {} with the defaults",
        changed.len(),
        default_run.len()
    );
    assert_eq!(
        changed.len(),
        default_run.len() - 1,
        "the second pass must collapse the 1.0/1.1 pair: {changed:?} vs {default_run:?}"
    );
    assert!(
        !changed.iter().any(|t| (t - 1.1).abs() < 0.05),
        "the 500 ms minimum gap did not reach the second pass: {changed:?}"
    );
    // the zero refine window keeps the hit on the detection grid: the default run's refine
    // walked it past the grid time (the shipped rule places later than the flux frame here)
    fn near(v: &[f64], t: f64) -> f64 {
        *v.iter()
            .min_by(|a, b| (**a - t).abs().total_cmp(&(**b - t).abs()))
            .unwrap()
    }
    let d = near(&default_run, 2.0);
    let c = near(&changed, 2.0);
    println!("hit near 2.0: refined {d}, grid {c}");
    assert!(
        d - c > 0.0005,
        "the refine window did not reach the second pass: grid {c} vs refined {d}"
    );
}
