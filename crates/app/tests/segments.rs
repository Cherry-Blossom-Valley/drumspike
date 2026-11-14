//! #508 (DAW plugin P1): a track as segments on a timeline. The gate: a single full-file
//! segment analyses byte-identically to the plain track, two segments with a gap give the
//! hits of each segment at `timeline start + offset in the segment` exact to the sample (a
//! hit in the gap or a trimmed part is absent), the session rules run on timeline positions
//! (the flam rule on a shifted session marks the same hits), and an old project file (no
//! `segments`) loads and analyses byte-identically.
mod common;

use common::*;
use drumspike_app::Service;
use drumspike_app::tom_bleed::run_after;
use drumspike_core::project::{Segment, add_track, build_source, load_project, new_project, save_project};
use serde_json::Value;

const SR: u32 = 48_000;

/// A track of [`project_segments`]: name, role, its signal and its timeline segments
/// (`(start, offset, length)`; empty: a plain track with the file as its source).
struct SegSpec {
    name: &'static str,
    role: &'static str,
    x: Vec<f32>,
    segs: Vec<(f64, f64, f64)>,
}

/// A project folder with the given tracks. A track with segments carries them (with the
/// shared record) and no top-level source, like a DAW clip list; the WAV files go to
/// `<root>/daw/<name>.wav`.
fn project_segments(root: &std::path::Path, project_name: &str, tracks: &[SegSpec]) -> std::path::PathBuf {
    let mut p = new_project(root, project_name).unwrap();
    for s in tracks {
        let wav = root.join("daw").join(format!("{}.wav", s.name));
        write_wav(&wav, SR, &s.x);
        let rec = build_source(&wav, &p.dir, "referenced").unwrap();
        let tr = add_track(&mut p, s.name, Some(rec.clone()), s.role, 36, Some("average")).unwrap();
        if !s.segs.is_empty() {
            tr.segments = Some(
                s.segs
                    .iter()
                    .map(|&(start, offset, length)| Segment {
                        start,
                        offset,
                        length,
                        source: Some(Some(rec.clone())),
                        extra: Default::default(),
                    })
                    .collect(),
            );
            tr.source = None;
        }
    }
    save_project(&mut p).unwrap();
    p.dir
}

fn events(svc: &Service, sid: &str, name: &str) -> Vec<Value> {
    svc.events_get(sid, name).unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn times(ev: &[Value]) -> Vec<f64> {
    ev.iter().map(|e| e["t"].as_f64().unwrap()).collect()
}

fn at_samples(ts: &[f64]) -> Vec<i64> {
    ts.iter().map(|t| (t * f64::from(SR)).round() as i64).collect()
}

fn open(svc: &Service, dir: &std::path::Path) -> String {
    svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string()
}

fn analyze_all(svc: &Service, dir: &std::path::Path) -> String {
    let sid = open(svc, dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(svc, &sid);
    sid
}

/// Gate 1: a track made of one segment that covers the full file gives an event list that is
/// byte-identical to the same file as a normal track.
#[test]
fn a_single_full_file_segment_is_byte_identical_to_the_plain_track() {
    let (root, root_seg) = (tmp("seg_full_plain"), tmp("seg_full_seg"));
    let x = kick_signal(&ONSETS, 3.5, SR);
    let dur = x.len() as f64 / f64::from(SR);
    let dir = project(&root, "Plain", &[kick_spec("k in", "kick", 36)]);
    let dir_seg = project_segments(
        &root_seg,
        "Seg",
        &[SegSpec {
            name: "k in",
            role: "kick",
            x: x.clone(),
            segs: vec![(0.0, 0.0, dur)],
        }],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let svc_seg = service_dsp();
    svc_seg.set_home(root_seg.join("home"));
    let sid = analyze_all(&svc, &dir);
    let sid_seg = analyze_all(&svc_seg, &dir_seg);
    let a = serde_json::to_string(&events(&svc, &sid, "k in")).unwrap();
    let b = serde_json::to_string(&events(&svc_seg, &sid_seg, "k in")).unwrap();
    assert!(!a.is_empty() && a != "[]", "no hits at all: {a}");
    assert_eq!(a, b, "the segmented track's events differ from the plain track's");
}

/// Gate 2: two segments of one file with a gap: the hits of each segment sit at
/// `timeline start + offset in the segment`, exact to the sample; a hit in the gap is absent.
#[test]
fn two_segments_with_a_gap_place_their_hits_on_the_timeline() {
    let (root, root_seg) = (tmp("seg_gap_full"), tmp("seg_gap_seg"));
    // onsets 1 s apart: every cut lands in written silence (a kick's tail ends 0.4 s after
    // its onset, the next onset is 1.0 s later)
    let onsets = [0.25, 1.25, 2.25, 3.25];
    let x = kick_signal(&onsets, 4.0, SR);
    // segment A: file 0..1 s at timeline 0; segment B: file 2..4 s at timeline 3
    // (timeline gap 1..3: the hit at 1.25 is absent)
    let dir_seg = project_segments(
        &root_seg,
        "Seg",
        &[SegSpec {
            name: "k in",
            role: "kick",
            x: x.clone(),
            segs: vec![(0.0, 0.0, 1.0), (3.0, 2.0, 2.0)],
        }],
    );
    let dir = project(
        &root,
        "Full",
        &[Spec {
            name: "k in",
            role: "kick",
            note: 36,
            x: x.clone(),
            sr: SR,
        }],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let svc_seg = service_dsp();
    svc_seg.set_home(root_seg.join("home"));
    let sid_full = analyze_all(&svc, &dir);
    let sid_seg = analyze_all(&svc_seg, &dir_seg);
    let full = at_samples(&times(&events(&svc, &sid_full, "k in")));
    let seg = at_samples(&times(&events(&svc_seg, &sid_seg, "k in")));
    // expected: every full-file hit inside a segment, moved by `start - offset`, exact to
    // the sample; the others (in the gap or a trimmed part) absent
    let mut expected = Vec::new();
    for s in full {
        let f = s as f64 / f64::from(SR);
        if f < 1.0 {
            expected.push(s);
        } else if (2.0..4.0).contains(&f) {
            expected.push(((f - 2.0 + 3.0) * f64::from(SR)).round() as i64);
        }
    }
    expected.sort_unstable();
    let mut got = seg.clone();
    got.sort_unstable();
    assert!(!expected.is_empty(), "the full-file track found no hits");
    assert_eq!(got, expected, "segmented hits are not the shifted segment hits");
    // and no hit in the gap
    for t in &seg {
        let f = *t as f64 / f64::from(SR);
        assert!(!((1.0..3.0).contains(&f)), "a hit in the gap at {f}s");
    }
}

/// Gate 3: the session rules run on timeline positions. The flam rule on a session where the
/// snare and its sources sit at timeline start 10 s (one segment each; the tom stays plain
/// at 0) marks the same hits as the unshifted session, shifted by exactly 10 s.
#[test]
fn the_flam_rule_marks_the_same_hits_on_shifted_tracks() {
    let (root, root_seg) = (tmp("seg_flam"), tmp("seg_flam_shift"));
    let (x_snare, _mains) = flam_track(3);
    let (x_kick, x_hat, x_tom) = (other_track(5, 60.0), other_track(7, 0.0), few_track(9));
    let dir = project(
        &root,
        "Flams",
        &[
            spec("s top", "snare", x_snare.clone()),
            spec("s samp", "snare", x_snare.clone()),
            spec("k in", "kick", x_kick.clone()),
            spec("hh", "hat", x_hat.clone()),
            spec("t1", "tom", x_tom.clone()),
        ],
    );
    let shift = 10.0;
    let span = |x: &[f32]| x.len() as f64 / f64::from(SR);
    let dir_seg = project_segments(
        &root_seg,
        "Flams",
        &[
            SegSpec {
                name: "s top",
                role: "snare",
                x: x_snare.clone(),
                segs: vec![(shift, 0.0, span(&x_snare))],
            },
            SegSpec {
                name: "s samp",
                role: "snare",
                x: x_snare.clone(),
                segs: vec![(shift, 0.0, span(&x_snare))],
            },
            SegSpec {
                name: "k in",
                role: "kick",
                x: x_kick.clone(),
                segs: vec![(shift, 0.0, span(&x_kick))],
            },
            SegSpec {
                name: "hh",
                role: "hat",
                x: x_hat.clone(),
                segs: vec![(shift, 0.0, span(&x_hat))],
            },
            SegSpec {
                name: "t1",
                role: "tom",
                x: x_tom,
                segs: Vec::new(),
            },
        ],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let svc_seg = service_dsp();
    svc_seg.set_home(root_seg.join("home"));
    let sid = analyze_all(&svc, &dir);
    let sid_seg = analyze_all(&svc_seg, &dir_seg);
    let unshifted = at_samples(&flams(&svc, &sid, "s top"));
    let shifted = at_samples(&flams(&svc_seg, &sid_seg, "s top"));
    assert_eq!(
        unshifted.len(),
        shifted.len(),
        "the shifted session marks a different number of flams"
    );
    let mut want: Vec<i64> = unshifted
        .iter()
        .map(|t| t + (shift * f64::from(SR)).round() as i64)
        .collect();
    want.sort_unstable();
    let mut got = shifted;
    got.sort_unstable();
    assert_eq!(got, want, "the shifted flam hits are not the unshifted ones + 10 s");
}

fn flams(svc: &Service, sid: &str, name: &str) -> Vec<f64> {
    events(svc, sid, name)
        .iter()
        .filter(|e| e["flam"] == true)
        .map(|e| e["t"].as_f64().unwrap())
        .collect()
}

/// Gate 4: a project file written before this change (no `segments` key) loads and analyses
/// to byte-identical events, and the document round-trips byte-identically.
#[test]
fn an_old_project_file_loads_and_analyses_byte_identically() {
    let (root, root_plain) = (tmp("seg_old"), tmp("seg_old_plain"));
    let dir = project(&root, "Old", &[kick_spec("k in", "kick", 36)]);
    let doc = dir.join("Old.drumspike");
    let text = String::from_utf8(std::fs::read(&doc).unwrap()).unwrap();
    let d = drumspike_core::Document::from_json(&text).unwrap();
    for (_, tr) in d.tracks() {
        assert!(tr.segments.is_none(), "an old track got segments");
        assert!(tr.segments().is_empty());
    }
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = analyze_all(&svc, &dir);
    let ev = serde_json::to_string(&events(&svc, &sid, "k in")).unwrap();
    assert!(!ev.is_empty() && ev != "[]");
    // the same audio as a plain track in a fresh project: identical events
    let dir2 = project(&root_plain, "Old2", &[kick_spec("k in", "kick", 36)]);
    let svc2 = service_dsp();
    svc2.set_home(root_plain.join("home"));
    let sid2 = analyze_all(&svc2, &dir2);
    assert_eq!(
        ev,
        serde_json::to_string(&events(&svc2, &sid2, "k in")).unwrap(),
        "the old project's events differ from a plain track of the same audio"
    );
    // and the document round-trips: load + save rewrites the same document (the save's
    // `modified` aside, no `segments` key appears and no key moves)
    let mut p = load_project(&dir).unwrap();
    save_project(&mut p).unwrap();
    let after = String::from_utf8(std::fs::read(&doc).unwrap()).unwrap();
    let strip = |s: &str| -> Value {
        let mut v: Value = serde_json::from_str(s).unwrap();
        v.as_object_mut().unwrap().remove("modified");
        v
    };
    assert_eq!(strip(&text), strip(&after), "the document changed through load + save");
    assert!(
        !after.contains("\"segments\""),
        "a saved old document gained a segments key"
    );
}

// ---------- the flam fixtures of tests/flam.rs (copied; a test binary cannot import another) ----------

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

/// A snare track: a backbeat every second at -6 dB from 1 s; at 3, 6 and 9 s a close pair
/// instead (the flam). Returns the signal.
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

// ---------- the coordinator's round on 785deeca: findings 1 to 3 ----------

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

/// The own hits of one tom: 12 hits 0.9 s apart from 0.5 s, every third one soft.
fn own_hits(d: usize) -> Vec<(usize, f32)> {
    (0..12)
        .map(|i| {
            let t = 0.5 + 0.3 * d as f64 + 0.9 * f64::from(i);
            ((t * f64::from(SR)) as usize, if i % 3 == 2 { 0.1 } else { 0.5 })
        })
        .collect()
}

/// Three tom close mics: each hears the other drums 20 dB down and 1 ms later; a noise floor
/// keeps every mic out of digital silence. Returns the signals and, per mic, the own and the
/// bleed hit samples.
fn toms3() -> (Vec<Vec<f32>>, Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let n = (12.0 * f64::from(SR)) as usize;
    let freqs = [140.0, 110.0, 85.0];
    let mut x = vec![vec![0.0f32; n]; 3];
    let mut owns = vec![Vec::new(); 3];
    let mut bleeds = vec![Vec::new(); 3];
    for (d, &freq) in freqs.iter().enumerate() {
        for (at, a) in own_hits(d) {
            for m in 0..3 {
                if m == d {
                    burst(&mut x[m], at, a, freq);
                    owns[m].push(at);
                } else {
                    burst(&mut x[m], at + 48, a * 0.1, freq);
                    bleeds[m].push(at + 48);
                }
            }
        }
    }
    let mut s = 7u64;
    for v in x.iter_mut().flatten() {
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *v += 1e-4 * (((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0) as f32;
    }
    for v in owns.iter_mut().chain(bleeds.iter_mut()) {
        v.sort_unstable();
    }
    (x, owns, bleeds)
}

/// The `bleed == true` events of one track, as samples.
fn marked(svc: &Service, sid: &str, name: &str) -> Vec<usize> {
    events(svc, sid, name)
        .iter()
        .filter(|e| e["bleed"] == true)
        .map(|e| (e["t"].as_f64().unwrap() * f64::from(SR)).round() as usize)
        .collect()
}

/// The hits `samples` as an events list at timeline offset `shift_s` seconds.
fn hits_at(samples: &[usize], shift_s: f64) -> Value {
    let mut v: Vec<usize> = samples.to_vec();
    v.sort_unstable();
    Value::Array(
        v.iter()
            .map(|&s| serde_json::json!({"t": (s as f64 + shift_s * f64::from(SR)) / f64::from(SR), "v": 100}))
            .collect(),
    )
}

/// A kick-like hit (125 -> 55 Hz sweep, 0.5 ms attack, 0.12 s decay) at `t0` s, `db` dBFS
/// (the fixture of `crates/core/src/detect/cross_bleed/tests.rs`).
fn kick_hit(x: &mut [f32], t0: f64, db: f64) {
    let i0 = (t0 * f64::from(SR)).round() as usize;
    let amp = 10f64.powf(db / 20.0);
    let mut ph = 0.0f64;
    for k in 0..((0.3 * f64::from(SR)) as usize).min(x.len().saturating_sub(i0)) {
        let t = k as f64 / f64::from(SR);
        ph += 2.0 * std::f64::consts::PI * (55.0 + 70.0 * (-t / 0.03).exp()) / f64::from(SR);
        let env = (t / 0.0005).min(1.0) * (-t / 0.12).exp();
        x[i0 + k] += (ph.sin() * env * amp) as f32;
    }
}

/// A tom-like hit (200 -> 90 Hz sweep, 0.5 ms attack, 0.25 s decay) at `t0` s, `db` dBFS.
fn tom_hit(x: &mut [f32], t0: f64, db: f64) {
    let i0 = (t0 * f64::from(SR)).round() as usize;
    let amp = 10f64.powf(db / 20.0);
    let mut ph = 0.0f64;
    for k in 0..((0.4 * f64::from(SR)) as usize).min(x.len().saturating_sub(i0)) {
        let t = k as f64 / f64::from(SR);
        ph += 2.0 * std::f64::consts::PI * (90.0 + 110.0 * (-t / 0.05).exp()) / f64::from(SR);
        let env = (t / 0.0005).min(1.0) * (-t / 0.25).exp();
        x[i0 + k] += (ph.sin() * env * amp) as f32;
    }
}

/// Finding 1, cross bleed: a tom close mic that hears a kick 18 dB down marks the bleed hits;
/// the same session with both tracks as segments at timeline start 10 s marks the same hits
/// shifted by exactly 10 s. (The unshifted session must mark at least one hit.)
#[test]
fn the_cross_bleed_rule_marks_the_same_hits_on_shifted_tracks() {
    let (root, root_seg) = (tmp("seg_cross"), tmp("seg_cross_shift"));
    let n = (10.0 * f64::from(SR)) as usize;
    let mut tom = vec![0.0f32; n];
    let mut kick = vec![0.0f32; n];
    // the tom's own hits (loud sweeps) and, between them, the kick's strokes, which the tom
    // hears 1 ms later and 18 dB under (the bleed); the kick hears the tom 18 dB down too,
    // so the tom's own hits stay unmarked
    for i in 0..8 {
        tom_hit(&mut tom, 1.0 + 0.9 * f64::from(i), 0.0);
        kick_hit(&mut tom, 1.45 + 0.9 * f64::from(i) + 0.001, -12.0);
        kick_hit(&mut kick, 1.45 + 0.9 * f64::from(i), -3.0);
        tom_hit(&mut kick, 1.0 + 0.9 * f64::from(i) + 0.001, -12.0);
    }
    let shift = 10.0;
    let span = |x: &[f32]| x.len() as f64 / f64::from(SR);
    let dir = project(
        &root,
        "Cross",
        &[spec("t1", "tom", tom.clone()), spec("k in", "kick", kick.clone())],
    );
    let dir_seg = project_segments(
        &root_seg,
        "Cross",
        &[
            SegSpec {
                name: "t1",
                role: "tom",
                x: tom.clone(),
                segs: vec![(shift, 0.0, span(&tom))],
            },
            SegSpec {
                name: "k in",
                role: "kick",
                x: kick.clone(),
                segs: vec![(shift, 0.0, span(&kick))],
            },
        ],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let svc_seg = service_dsp();
    svc_seg.set_home(root_seg.join("home"));
    let sid = analyze_all(&svc, &dir);
    let sid_seg = analyze_all(&svc_seg, &dir_seg);
    // the cross-bleed marks: the hits pass 1 judged bleed (the detector record's
    // `pass1_removed`; the re-detection replaces the step-1 flags on the events)
    let unshifted = cross_marked(&svc, &sid, "t1");
    assert!(
        !unshifted.is_empty(),
        "the unshifted session marks no cross bleed at all"
    );
    let mut want: Vec<i64> = unshifted
        .iter()
        .map(|s| s + (shift * f64::from(SR)).round() as i64)
        .collect();
    want.sort_unstable();
    let mut got = cross_marked(&svc_seg, &sid_seg, "t1");
    got.sort_unstable();
    assert_eq!(
        got, want,
        "the shifted cross-bleed marks are not the unshifted ones + 10 s"
    );
}

/// The times (samples) of the hits the cross-bleed rule judged bleed on one track.
fn cross_marked(svc: &Service, sid: &str, name: &str) -> Vec<i64> {
    svc.tracks_get(sid).unwrap()["tracks"][name]["detector"]["cross_bleed"]["pass1_removed"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .map(|r| (r["t"].as_f64().unwrap() * f64::from(SR)).round() as i64)
        .collect()
}

/// Finding 1, tom bleed: the three toms hear each other 20 dB down; the rule on the stored
/// hits marks exactly the bleed hits, and the same session with all three toms as segments at
/// timeline start 10 s marks the same hits shifted by exactly 10 s.
#[test]
fn the_tom_bleed_rule_marks_the_same_hits_on_shifted_tracks() {
    let (root, root_seg) = (tmp("seg_tbleed"), tmp("seg_tbleed_shift"));
    let (x, owns, bleeds) = toms3();
    let shift = 10.0;
    let span = |x: &[f32]| x.len() as f64 / f64::from(SR);
    let dir = project(
        &root,
        "Toms",
        &[
            spec("t1", "tom", x[0].clone()),
            spec("t2", "tom", x[1].clone()),
            spec("t3", "tom", x[2].clone()),
        ],
    );
    let dir_seg = project_segments(
        &root_seg,
        "Toms",
        &[
            SegSpec {
                name: "t1",
                role: "tom",
                x: x[0].clone(),
                segs: vec![(shift, 0.0, span(&x[0]))],
            },
            SegSpec {
                name: "t2",
                role: "tom",
                x: x[1].clone(),
                segs: vec![(shift, 0.0, span(&x[1]))],
            },
            SegSpec {
                name: "t3",
                role: "tom",
                x: x[2].clone(),
                segs: vec![(shift, 0.0, span(&x[2]))],
            },
        ],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let svc_seg = service_dsp();
    svc_seg.set_home(root_seg.join("home"));
    let sid = analyze_all(&svc, &dir);
    let sid_seg = analyze_all(&svc_seg, &dir_seg);
    for (m, name) in ["t1", "t2", "t3"].iter().enumerate() {
        let all: Vec<usize> = owns[m].iter().chain(&bleeds[m]).copied().collect();
        // the hits a detector would list, at the timeline positions
        svc.events_set(&sid, name, &hits_at(&all, 0.0)).unwrap();
        svc_seg.events_set(&sid_seg, name, &hits_at(&all, shift)).unwrap();
    }
    let targets: Vec<(String, u32)> = ["t1", "t2", "t3"].iter().map(|n| (n.to_string(), 1)).collect();
    let r = run_after(&svc, &sid, &targets).unwrap().unwrap();
    assert_eq!(r["on"], true, "{r}");
    let r_seg = run_after(&svc_seg, &sid_seg, &targets).unwrap().unwrap();
    assert_eq!(r_seg["on"], true, "{r_seg}");
    for (m, name) in ["t1", "t2", "t3"].iter().enumerate() {
        let unshifted = marked(&svc, &sid, name);
        assert_eq!(
            unshifted, bleeds[m],
            "{name}: the unshifted marks are not the bleed hits"
        );
        let shifted = marked(&svc_seg, &sid_seg, name);
        let want: Vec<usize> = bleeds[m].iter().map(|s| s + (shift * f64::from(SR)) as usize).collect();
        assert_eq!(shifted, want, "{name}: the shifted marks are not the bleed hits + 10 s");
    }
}

// ---------- finding 2: a segment with bad numbers is a 400, not a panic ----------

/// Rewrites the document of `dir` through `f` (the raw JSON, so a value that does not read as
/// a segment list can be planted).
fn patch_doc(dir: &std::path::Path, project_name: &str, f: impl FnOnce(&mut Value)) {
    let doc = dir.join(format!("{project_name}.drumspike"));
    let text = String::from_utf8(std::fs::read(&doc).unwrap()).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    f(&mut v);
    std::fs::write(&doc, serde_json::to_string(&v).unwrap()).unwrap();
}

/// A project whose one segment carries `value` under the key `field`, analysed: the state of
/// the `k in` track.
fn analyze_with_first_segment(tag: &str, field: &str, value: Value) -> Value {
    let root = tmp(tag);
    let x = kick_signal(&ONSETS, 3.0, SR);
    let dir = project_segments(
        &root,
        "Seg",
        &[SegSpec {
            name: "k in",
            role: "kick",
            x,
            segs: vec![(0.0, 0.0, 1.0)],
        }],
    );
    patch_doc(&dir, "Seg", |v| {
        v["tracks"]["k in"]["segments"][0][field] = value;
    });
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = open(&svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    let st = wait_done(&svc, &sid);
    let t = st["tracks"]["k in"].clone();
    std::fs::remove_dir_all(&root).unwrap();
    t
}

/// Finding 2, negative `length`: the analysis answers the 400 that names the track and the
/// segment index instead of panicking the timeline slice (`i1 < i0`). Red on 785deeca: the
/// decode panics and the track never answers this error.
#[test]
fn a_negative_segment_length_answers_a_400() {
    let t = analyze_with_first_segment("seg_badlen", "length", serde_json::json!(-1.0));
    assert_eq!(t["state"], "error", "{t}");
    let msg = t["error"].as_str().unwrap();
    assert!(
        msg.contains("k in") && msg.contains("segment 0") && msg.contains("length"),
        "{msg}"
    );
}

/// Finding 2, negative `offset`: the same 400; on 785deeca the analysis silently reads from
/// the file start instead, so the track analyses (the test fails).
#[test]
fn a_negative_segment_offset_answers_a_400() {
    let t = analyze_with_first_segment("seg_badoff", "offset", serde_json::json!(-0.5));
    assert_eq!(t["state"], "error", "{t}");
    let msg = t["error"].as_str().unwrap();
    assert!(
        msg.contains("k in") && msg.contains("segment 0") && msg.contains(">= 0"),
        "{msg}"
    );
}

/// Finding 2, a NaN: a NaN cannot survive a document round-trip (JSON has no NaN, and
/// `serde_json` writes one as `null`), so this exercises the in-memory path the library and
/// the bindings take: the segment list is patched on the loaded document. Red on 785deeca:
/// `track_audio` answers the timeline and the decode corrupts the assembly.
#[test]
fn a_nan_segment_answers_a_400() {
    let root = tmp("seg_nan");
    let x = kick_signal(&ONSETS, 3.0, SR);
    let dir = project_segments(
        &root,
        "Seg",
        &[SegSpec {
            name: "k in",
            role: "kick",
            x,
            segs: vec![(0.0, 0.0, 1.0)],
        }],
    );
    let mut p = load_project(&dir).unwrap();
    let tr = p.doc.track_mut("k in").unwrap();
    tr.segments.as_mut().unwrap()[0].length = f64::NAN;
    let op = drumspike_app::registry::OpenProject::new("nanprobe0".into(), p);
    let err = op.track_audio("k in").unwrap_err();
    assert!(err.message.contains("finite"), "{err:?}");
    std::fs::remove_dir_all(&root).unwrap();
}

// ---------- finding 3: a malformed `segments` value is an error, not silent ----------

/// Finding 3: one segment without `length` (so the list does not read as segments and stays
/// in `extra`) with a top-level source present: the analysis answers the error and does not
/// analyse the full file. The document still loads and saves unchanged.
#[test]
fn a_malformed_segments_value_answers_a_400_and_is_not_analysed_as_a_plain_track() {
    let root = tmp("seg_malformed");
    let dir = project(&root, "Mal", &[kick_spec("k in", "kick", 36)]);
    patch_doc(&dir, "Mal", |v| {
        v["tracks"]["k in"]["segments"] = serde_json::json!([{"start": 0.0, "offset": 0.0}]);
    });
    // the document still loads and saves unchanged: the value stays as it is in `extra`
    let mut p = load_project(&dir).unwrap();
    assert!(p.doc.track("k in").unwrap().extra.contains_key("segments"));
    let doc = dir.join("Mal.drumspike");
    let before = String::from_utf8(std::fs::read(&doc).unwrap()).unwrap();
    save_project(&mut p).unwrap();
    let after = String::from_utf8(std::fs::read(&doc).unwrap()).unwrap();
    let strip = |s: &str| -> Value {
        let mut v: Value = serde_json::from_str(s).unwrap();
        v.as_object_mut().unwrap().remove("modified");
        v
    };
    assert_eq!(
        strip(&before),
        strip(&after),
        "the document changed through load + save"
    );
    // the analysis answers the error and does not analyse the full file
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = open(&svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    let st = wait_done(&svc, &sid);
    let t = &st["tracks"]["k in"];
    assert_eq!(t["state"], "error", "{st}");
    assert!(
        t["error"].as_str().unwrap().contains("k in: segments are malformed"),
        "{t}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
